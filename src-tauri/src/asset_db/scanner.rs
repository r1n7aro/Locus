use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(crate) const IGNORED_DIRS: &[&str] = &[
    "Library",
    "Temp",
    "Logs",
    "Obj",
    "Build",
    "Builds",
    "UserSettings",
    ".git",
    ".svn",
    ".vs",
    "node_modules",
];

pub(crate) const P1_EXTENSIONS: &[&str] =
    &["unity", "prefab", "asset", "mat", "anim", "controller"];

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
}

fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

pub(crate) fn get_mtime_ns(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub fn scan_directory(project_root: &Path) -> DirSnapshot {
    let scan_roots = ["Assets", "Packages"];
    let mut meta_files = Vec::new();
    let mut yaml_asset_files = Vec::new();
    let mut dirs_scanned = 0u64;

    for root_name in &scan_roots {
        let root_path = project_root.join(root_name);
        if !root_path.is_dir() {
            continue;
        }

        let walker = WalkDir::new(&root_path).into_iter().filter_entry(|entry| {
            if entry.file_type().is_dir() {
                let name = entry.file_name().to_string_lossy();
                !is_ignored_dir(&name)
            } else {
                true
            }
        });

        for entry in walker.filter_map(|e| e.ok()) {
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
    }
}
