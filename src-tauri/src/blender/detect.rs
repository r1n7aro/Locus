//! Bounded workspace scan for `.blend` files. It only decides the initial
//! workspace mode for Blender (found: lazy, none: off); it never gates use.

use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 20_000;
const SKIPPED_DIRS: &[&str] = &[
    "library",
    "temp",
    "logs",
    "obj",
    ".git",
    "node_modules",
    ".locus",
];

/// True when a `.blend` file exists within `MAX_DEPTH` directory levels of
/// `root`. Stops at the first match and after `MAX_ENTRIES` entries, so the
/// cost is bounded on large Unity projects.
pub fn workspace_has_blend_files(root: &Path) -> bool {
    !blend_files(root, 1).is_empty()
}

/// Up to `limit` `.blend` files within the bounded scan of `root`.
pub fn blend_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES || found.len() >= limit {
                return found;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if depth + 1 < MAX_DEPTH
                    && !SKIPPED_DIRS.contains(&name.to_ascii_lowercase().as_str())
                {
                    stack.push((entry.path(), depth + 1));
                }
            } else if file_type.is_file() && is_blend_file_name(&name) {
                found.push(entry.path());
            }
        }
    }
    found
}

/// `.blend` only; numbered backups (`.blend1`, `.blend2`) are ignored.
pub fn is_blend_file_name(name: &str) -> bool {
    name.len() > ".blend".len() && name.to_ascii_lowercase().ends_with(".blend")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_blend_files_within_depth_and_skips_backups_and_library() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("Library/Models")).unwrap();
        std::fs::write(root.join("Library/Models/cached.blend"), b"x").unwrap();
        std::fs::write(root.join("scene.blend1"), b"x").unwrap();
        assert!(!workspace_has_blend_files(root));

        std::fs::create_dir_all(root.join("Assets/Art/Characters")).unwrap();
        std::fs::write(root.join("Assets/Art/Characters/hero.BLEND"), b"x").unwrap();
        assert!(workspace_has_blend_files(root));
    }

    #[test]
    fn ignores_files_below_the_depth_limit() {
        let temp = tempfile::tempdir().unwrap();
        let deep = temp.path().join("a/b/c/d/e");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.blend"), b"x").unwrap();
        assert!(!workspace_has_blend_files(temp.path()));
        assert!(is_blend_file_name("x.blend"));
        assert!(!is_blend_file_name(".blend"));
        assert!(!is_blend_file_name("x.blend1"));
    }
}
