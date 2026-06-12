use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

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

pub struct DirSnapshot {
    pub meta_files: Vec<FileEntry>,
    pub yaml_asset_files: Vec<FileEntry>,
    pub dirs_scanned: u64,
    pub linked_asset_roots: Vec<LinkedAssetRoot>,
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
    let scan_roots = ["Assets", "Packages"];
    let mut meta_files = Vec::new();
    let mut yaml_asset_files = Vec::new();
    let mut dirs_scanned = 0u64;
    let mut linked_asset_roots = Vec::new();
    let mut linked_asset_rel_paths = HashSet::new();

    for root_name in &scan_roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let root_path = project_root.join(root_name);
        if !root_path.is_dir() {
            continue;
        }

        let walker = WalkDir::new(&root_path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|entry| {
                // Files use the same rule as directories: Unity ignores
                // hidden / `~`-suffixed files too, so their orphan .meta
                // siblings never become indexable assets.
                let name = entry.file_name().to_string_lossy();
                !is_ignored_name(&name)
            });

        for entry in walker.filter_map(|e| e.ok()) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            if entry.path_is_symlink() && entry.file_type().is_dir() {
                if let Ok(rel) = entry.path().strip_prefix(project_root) {
                    let link_rel_path = rel.to_string_lossy().replace('\\', "/");
                    if linked_asset_rel_paths.insert(link_rel_path.clone()) {
                        let target_path = dunce::canonicalize(entry.path())
                            .unwrap_or_else(|_| entry.path().to_path_buf());
                        linked_asset_roots.push(LinkedAssetRoot {
                            link_rel_path,
                            target_path,
                        });
                    }
                }
            }

            if entry.file_type().is_dir() {
                dirs_scanned += 1;
                continue;
            }

            let abs_path = entry.path().to_path_buf();
            let rel_path = abs_path
                .strip_prefix(project_root)
                .unwrap_or(&abs_path)
                .to_string_lossy()
                .replace('\\', "/");

            let ext = abs_path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            let metadata = entry.metadata().ok();
            let mtime_ns = metadata.as_ref().map(get_mtime_ns).unwrap_or(0);
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

            let file_entry = FileEntry {
                rel_path,
                abs_path,
                ext: ext.clone(),
                mtime_ns,
                size,
            };

            if ext == "meta" {
                meta_files.push(file_entry);
            } else if P1_EXTENSIONS.contains(&ext.as_str()) {
                yaml_asset_files.push(file_entry);
            }
        }
    }

    DirSnapshot {
        meta_files,
        yaml_asset_files,
        dirs_scanned,
        linked_asset_roots,
    }
}
