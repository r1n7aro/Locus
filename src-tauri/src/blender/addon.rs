//! Host add-on install state per Blender user scripts directory.
//!
//! The add-on is the `locus_host` package installed into
//! `<user scripts>/startup/locus_host/`, which Blender imports at startup
//! whatever the add-on preferences say. The expected version is a blake3 hash
//! over the bundled package (including the DLL); installs write it to
//! `.locus_host_hash`, mirroring Unity's `.locus_plugin_hash`.

use std::path::{Path, PathBuf};

use serde::Serialize;

pub const HASH_FILE: &str = ".locus_host_hash";
pub const PACKAGE_DIR: &str = "locus_host";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddonStatus {
    Missing,
    Outdated,
    UpToDate,
    /// The bundled sources are missing, so no comparison is possible.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonState {
    pub status: AddonStatus,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
}

fn package_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "__pycache__" || name == HASH_FILE || name.starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => files.push(path),
                _ => {}
            }
        }
    }
    files.sort();
    files
}

/// blake3 over relative paths and contents of the bundled `locus_host`.
pub fn hash_package(root: &Path) -> Option<String> {
    let files = package_files(root);
    if files.is_empty() {
        return None;
    }
    let mut hasher = blake3::Hasher::new();
    for file in files {
        let relative = file.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
        hasher.update(relative.as_bytes());
        hasher.update(&[0]);
        hasher.update(&std::fs::read(&file).ok()?);
        hasher.update(&[0]);
    }
    Some(hasher.finalize().to_hex().to_string())
}

/// Hash of the bundled package, recomputed only when a file's size or
/// modification time changes (listing installations calls this often).
pub fn expected_hash() -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
    let dir = super::resources::host_package_dir()?;
    let mut stamp = blake3::Hasher::new();
    for file in package_files(&dir) {
        let metadata = std::fs::metadata(&file).ok()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        stamp.update(file.to_string_lossy().as_bytes());
        stamp.update(&metadata.len().to_le_bytes());
        stamp.update(&modified.to_le_bytes());
    }
    let stamp = stamp.finalize().to_hex().to_string();
    if let Ok(cache) = CACHE.lock() {
        if let Some((cached_stamp, hash)) = cache.as_ref() {
            if *cached_stamp == stamp {
                return Some(hash.clone());
            }
        }
    }
    let hash = hash_package(&dir)?;
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((stamp, hash.clone()));
    }
    Some(hash)
}

pub fn installed_dir(user_scripts: &Path) -> PathBuf {
    user_scripts.join("startup").join(PACKAGE_DIR)
}

pub fn installed_hash(user_scripts: &Path) -> Option<String> {
    std::fs::read_to_string(installed_dir(user_scripts).join(HASH_FILE))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

pub fn addon_state(user_scripts: &Path) -> AddonState {
    let dir = installed_dir(user_scripts);
    let installed = installed_hash(user_scripts);
    let status = match (&installed, expected_hash()) {
        (None, _) if !dir.join("__init__.py").is_file() => AddonStatus::Missing,
        (_, None) => AddonStatus::Unknown,
        (Some(installed), Some(expected)) if *installed == expected => AddonStatus::UpToDate,
        _ => AddonStatus::Outdated,
    };
    AddonState {
        status,
        path: dir.to_string_lossy().to_string(),
        installed_hash: installed,
    }
}

/// Install or update into `<user scripts>/startup/locus_host`. Files are
/// staged first (and validated), then copied over the installed package;
/// the hash file is written last, so an interrupted update reads as
/// outdated. A DLL loaded by a running Blender is never overwritten: each
/// build has a content-hash file name, and the old one is removed later.
pub fn install(user_scripts: &Path) -> Result<String, String> {
    let source = super::resources::host_package_dir()
        .ok_or_else(|| "Blender host sources (locus_blender/locus_host) are missing".to_string())?;
    let hash = hash_package(&source).ok_or_else(|| "Blender host sources are empty".to_string())?;
    if !source.join("bin").join("native.json").is_file() {
        return Err("the Blender native layer is not built (run `bun run blender:bundle-native`)".to_string());
    }
    let startup = user_scripts.join("startup");
    std::fs::create_dir_all(&startup).map_err(|e| format!("{}: {e}", startup.display()))?;
    let staging = startup.join(format!(".locus_host_staging_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    copy_tree(&source, &staging)?;
    if hash_package(&staging).as_deref() != Some(hash.as_str()) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("staged Blender host does not match the bundled sources".to_string());
    }
    let target = installed_dir(user_scripts);
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let _ = std::fs::remove_file(target.join(HASH_FILE));
    let result = copy_tree(&staging, &target);
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    remove_stale_files(&source, &target);
    std::fs::write(target.join(HASH_FILE), &hash).map_err(|e| format!("failed to write {HASH_FILE}: {e}"))?;
    Ok(hash)
}

pub fn uninstall(user_scripts: &Path) -> Result<(), String> {
    let target = installed_dir(user_scripts);
    if !target.exists() {
        return Ok(());
    }
    let _ = std::fs::remove_file(target.join(HASH_FILE));
    let _ = std::fs::remove_file(target.join("__init__.py"));
    match std::fs::remove_dir_all(&target) {
        Ok(()) => Ok(()),
        // A running Blender keeps its DLL locked; without `__init__.py`
        // the package no longer loads, and the leftovers go on next install.
        Err(error) => {
            eprintln!("[Locus] Blender host partly removed ({}): {error}", target.display());
            Ok(())
        }
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    for file in package_files(source) {
        let relative = file.strip_prefix(source).map_err(|e| e.to_string())?;
        let destination = target.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if is_versioned_dll(&destination) && destination.is_file() {
            continue; // same content-hash name: identical file, maybe loaded
        }
        std::fs::copy(&file, &destination)
            .map_err(|e| format!("failed to copy {} -> {}: {e}", file.display(), destination.display()))?;
    }
    Ok(())
}

fn is_versioned_dll(path: &Path) -> bool {
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            name.starts_with("locus_blender_native_") && name.ends_with(".dll")
        })
        .unwrap_or(false)
}

/// Remove installed files the bundle no longer has. Old DLLs still loaded by
/// a running Blender fail to delete and are retried on the next install.
fn remove_stale_files(source: &Path, target: &Path) {
    let wanted = package_files(source)
        .into_iter()
        .filter_map(|file| file.strip_prefix(source).ok().map(|p| p.to_path_buf()))
        .collect::<std::collections::BTreeSet<_>>();
    for file in package_files(target) {
        let Ok(relative) = file.strip_prefix(target) else { continue };
        if !wanted.contains(relative) {
            let _ = std::fs::remove_file(&file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_ignores_caches_and_tracks_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("locus_host");
        std::fs::create_dir_all(root.join("__pycache__")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("__init__.py"), "x = 1").unwrap();
        std::fs::write(root.join("bin/native.json"), "{}").unwrap();
        let first = hash_package(&root).unwrap();
        std::fs::write(root.join("__pycache__/a.pyc"), "cache").unwrap();
        std::fs::write(root.join(HASH_FILE), "old").unwrap();
        assert_eq!(hash_package(&root).unwrap(), first);
        std::fs::write(root.join("__init__.py"), "x = 2").unwrap();
        assert_ne!(hash_package(&root).unwrap(), first);
    }

    #[test]
    fn copy_skips_existing_versioned_dlls_and_removes_stale_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        let target = temp.path().join("dst");
        std::fs::create_dir_all(source.join("bin")).unwrap();
        std::fs::create_dir_all(target.join("bin")).unwrap();
        std::fs::write(source.join("host.py"), "new").unwrap();
        std::fs::write(source.join("bin/locus_blender_native_aaaa.dll"), "dll-new").unwrap();
        std::fs::write(target.join("bin/locus_blender_native_aaaa.dll"), "dll-loaded").unwrap();
        std::fs::write(target.join("bin/locus_blender_native_old.dll"), "old").unwrap();
        std::fs::write(target.join("gone.py"), "stale").unwrap();
        copy_tree(&source, &target).unwrap();
        remove_stale_files(&source, &target);
        assert_eq!(std::fs::read_to_string(target.join("host.py")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(target.join("bin/locus_blender_native_aaaa.dll")).unwrap(), "dll-loaded");
        assert!(!target.join("bin/locus_blender_native_old.dll").exists());
        assert!(!target.join("gone.py").exists());
    }
}
