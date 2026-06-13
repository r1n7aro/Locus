use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;

use super::types::LinkedAssetRoot;

/// Whether a path component should be skipped while scanning for assets.
///
/// The scanner only ever walks `Assets/` and `Packages/`, so Unity's
/// project-root folders (`Library`, `Temp`, `Build`, ...) are never visited
/// and must NOT be filtered by name — `Assets/Build` or `Assets/Temp` are
/// perfectly legal asset folders that older versions of this list silently
/// dropped. What we skip mirrors what the Unity importer itself ignores:
/// hidden entries (leading `.` — also covers `.git`/`.svn`/`.vs`), entries
/// ending in `~` (`Samples~`, `Documentation~`, editor backup files), and
/// `cvs`. `node_modules` is kept as a pragmatic extra: Unity would import
/// it, but in practice it only appears as huge vendored toolchain trees.
pub(crate) fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || name.eq_ignore_ascii_case("cvs")
        || name.eq_ignore_ascii_case("node_modules")
}

pub(crate) const P1_EXTENSIONS: &[&str] = &[
    "unity",
    "prefab",
    "asset",
    "mat",
    "anim",
    "controller",
    "overridecontroller",
    "mixer",
    "physicmaterial",
    "physicsmaterial2d",
    "flare",
    "mask",
    "preset",
    "fontsettings",
    "lighting",
    "terrainlayer",
    "rendertexture",
    "signal",
    "playable",
    "cubemap",
    "guiskin",
    "brush",
];

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub ext: String,
    pub mtime_ns: u64,
    pub size: u64,
}

/// On-disk facts for one entry the walk visited. Captured for *every* file
/// and directory (not just `.meta`/P1 files) so the meta-parse phase can
/// resolve a meta's sibling content file with a hash lookup instead of one
/// `stat` syscall per asset. Directories report `is_file = false` with zero
/// mtime/size, matching what the old per-meta disk probe returned for
/// folder assets.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EntryProbe {
    pub is_file: bool,
    pub mtime_ns: u64,
    pub size: u64,
}

pub struct DirSnapshot {
    pub meta_files: Vec<FileEntry>,
    pub yaml_asset_files: Vec<FileEntry>,
    pub dirs_scanned: u64,
    pub linked_asset_roots: Vec<LinkedAssetRoot>,
    /// rel_path → on-disk facts for every entry the walk visited. Empty when
    /// the caller opted out of probe collection (`collect_probes = false`).
    pub(crate) entry_probes: HashMap<String, EntryProbe>,
}

pub(crate) fn get_mtime_ns(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn scan_directory(project_root: &Path) -> DirSnapshot {
    let cancel = AtomicBool::new(false);
    scan_directory_with_cancel(project_root, &cancel)
}

pub fn scan_directory_with_cancel(project_root: &Path, cancel: &AtomicBool) -> DirSnapshot {
    scan_directory_with_options(project_root, cancel, true)
}

/// Partial scan result produced by one directory subtree. Merged pairwise by
/// rayon's order-preserving `reduce`, so the final entry order is
/// deterministic for a given on-disk layout (readdir order within a
/// directory, files before subdirectory contents).
#[derive(Default)]
struct ScanPartial {
    meta_files: Vec<FileEntry>,
    yaml_asset_files: Vec<FileEntry>,
    dirs_scanned: u64,
    linked_asset_roots: Vec<LinkedAssetRoot>,
    entry_probes: Vec<(String, EntryProbe)>,
}

impl ScanPartial {
    fn merge(mut self, mut other: ScanPartial) -> ScanPartial {
        self.meta_files.append(&mut other.meta_files);
        self.yaml_asset_files.append(&mut other.yaml_asset_files);
        self.dirs_scanned += other.dirs_scanned;
        self.linked_asset_roots
            .append(&mut other.linked_asset_roots);
        self.entry_probes.append(&mut other.entry_probes);
        self
    }
}

struct ScanContext<'a> {
    project_root: &'a Path,
    cancel: &'a AtomicBool,
    collect_probes: bool,
    /// Canonical targets of symlinked directories that have already been
    /// walked (pre-seeded with the scan roots). Guards against symlink
    /// cycles and double-walking a tree reachable through two links — the
    /// old `walkdir` + `follow_links` walker only caught ancestor cycles.
    visited_link_targets: Mutex<HashSet<PathBuf>>,
}

/// Walk `Assets/` and `Packages/` with a rayon-parallel recursive descent.
///
/// This replaces the serial `walkdir` walk with `follow_links(true)`, which
/// on Windows forced one full `fs::metadata` syscall per file (the followed
/// metadata can't come from the directory enumeration). `std::fs::read_dir`
/// entries expose file type and metadata from the enumeration itself (free
/// on Windows, one `lstat` on Unix), and each subdirectory becomes a rayon
/// task, so big NTFS trees scan with all cores instead of one.
///
/// Symlinked directories are followed manually: every one is recorded as a
/// [`LinkedAssetRoot`] (same as before) and its target walked exactly once.
pub(crate) fn scan_directory_with_options(
    project_root: &Path,
    cancel: &AtomicBool,
    collect_probes: bool,
) -> DirSnapshot {
    let scan_roots = ["Assets", "Packages"];

    let ctx = ScanContext {
        project_root,
        cancel,
        collect_probes,
        visited_link_targets: Mutex::new(HashSet::new()),
    };

    // Pre-seed the visited set with the scan roots so a symlink pointing
    // back at `Assets/` or `Packages/` never re-walks a whole tree.
    {
        let mut visited = ctx.visited_link_targets.lock().unwrap();
        for root_name in &scan_roots {
            let root_path = project_root.join(root_name);
            if root_path.is_dir() {
                let canonical =
                    dunce::canonicalize(&root_path).unwrap_or_else(|_| root_path.clone());
                visited.insert(canonical);
            }
        }
    }

    let mut combined = ScanPartial::default();
    for root_name in &scan_roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let root_path = project_root.join(root_name);
        if !root_path.is_dir() {
            continue;
        }

        // A scan root that is itself a symlink/junction is a linked asset
        // root too (the old walker recorded it via its depth-0 entry).
        let root_is_link = std::fs::symlink_metadata(&root_path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if root_is_link {
            let target_path = dunce::canonicalize(&root_path).unwrap_or_else(|_| root_path.clone());
            combined.linked_asset_roots.push(LinkedAssetRoot {
                link_rel_path: (*root_name).to_string(),
                target_path,
            });
        }

        combined = combined.merge(scan_tree(&ctx, root_path));
    }

    // Two different links can point at the same target; the rel-path dedupe
    // mirrors the old walker's HashSet guard.
    let mut seen_link_rel_paths = HashSet::new();
    combined
        .linked_asset_roots
        .retain(|root| seen_link_rel_paths.insert(root.link_rel_path.clone()));

    DirSnapshot {
        meta_files: combined.meta_files,
        yaml_asset_files: combined.yaml_asset_files,
        dirs_scanned: combined.dirs_scanned,
        linked_asset_roots: combined.linked_asset_roots,
        entry_probes: combined.entry_probes.into_iter().collect(),
    }
}

fn scan_tree(ctx: &ScanContext<'_>, dir_abs: PathBuf) -> ScanPartial {
    let mut local = ScanPartial {
        dirs_scanned: 1,
        ..ScanPartial::default()
    };

    if ctx.cancel.load(Ordering::Relaxed) {
        return local;
    }

    let read_dir = match std::fs::read_dir(&dir_abs) {
        Ok(read_dir) => read_dir,
        Err(_) => return local,
    };

    let mut child_dirs: Vec<PathBuf> = Vec::new();

    for entry in read_dir.flatten() {
        if ctx.cancel.load(Ordering::Relaxed) {
            return local;
        }

        let name = entry.file_name();
        if is_ignored_name(&name.to_string_lossy()) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let abs_path = entry.path();

        if file_type.is_symlink() {
            // One followed stat to learn what the link points at. Broken
            // links are skipped, matching the old walker's `.ok()` filter.
            let Ok(target_meta) = std::fs::metadata(&abs_path) else {
                continue;
            };
            if target_meta.is_dir() {
                let rel_path = rel_path_of(ctx.project_root, &abs_path);
                let target_path =
                    dunce::canonicalize(&abs_path).unwrap_or_else(|_| abs_path.clone());
                if ctx.collect_probes {
                    local
                        .entry_probes
                        .push((rel_path.clone(), EntryProbe::default()));
                }
                local.linked_asset_roots.push(LinkedAssetRoot {
                    link_rel_path: rel_path,
                    target_path: target_path.clone(),
                });
                let not_yet_walked = ctx.visited_link_targets.lock().unwrap().insert(target_path);
                if not_yet_walked {
                    child_dirs.push(abs_path);
                }
            } else if target_meta.is_file() {
                classify_file(ctx, &mut local, abs_path, &target_meta);
            }
            continue;
        }

        if file_type.is_dir() {
            if ctx.collect_probes {
                local.entry_probes.push((
                    rel_path_of(ctx.project_root, &abs_path),
                    EntryProbe::default(),
                ));
            }
            child_dirs.push(abs_path);
            continue;
        }

        // Regular file. `DirEntry::metadata` is served from the directory
        // enumeration on Windows (no extra syscall).
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        classify_file(ctx, &mut local, abs_path, &metadata);
    }

    let children = child_dirs
        .into_par_iter()
        .map(|dir| scan_tree(ctx, dir))
        .reduce(ScanPartial::default, ScanPartial::merge);

    local.merge(children)
}

fn rel_path_of(project_root: &Path, abs_path: &Path) -> String {
    abs_path
        .strip_prefix(project_root)
        .unwrap_or(abs_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn classify_file(
    ctx: &ScanContext<'_>,
    local: &mut ScanPartial,
    abs_path: PathBuf,
    metadata: &std::fs::Metadata,
) {
    let rel_path = rel_path_of(ctx.project_root, &abs_path);
    let ext = abs_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    let mtime_ns = get_mtime_ns(metadata);
    let size = metadata.len();

    if ctx.collect_probes {
        local.entry_probes.push((
            rel_path.clone(),
            EntryProbe {
                is_file: true,
                mtime_ns,
                size,
            },
        ));
    }

    let indexed = ext == "meta" || P1_EXTENSIONS.contains(&ext.as_str());
    if !indexed {
        return;
    }

    let file_entry = FileEntry {
        rel_path,
        abs_path,
        ext: ext.clone(),
        mtime_ns,
        size,
    };

    if ext == "meta" {
        local.meta_files.push(file_entry);
    } else {
        local.yaml_asset_files.push(file_entry);
    }
}
