use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::strip_extended_path_prefix;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PluginStatus {
    Missing,
    Outdated,
    UpToDate,
}

const PLUGIN_DEFAULT_INSTALL_DIR: &str = "Packages/com.farlocus.locus";
const PLUGIN_ASMDEF_NAME: &str = "Locus.Editor.asmdef";
const PLUGIN_HASH_FILE: &str = ".locus_plugin_hash";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginInstallLocation {
    Assets,
    Packages,
}

#[derive(Debug, Clone)]
struct InstalledPluginDir {
    root: PathBuf,
    location: PluginInstallLocation,
}

pub fn find_plugin_source_dir() -> Option<std::path::PathBuf> {
    let mut candidates = vec![
        std::path::PathBuf::from("../locus_unity"), // dev: src-tauri/../locus_unity
        std::path::PathBuf::from("locus_unity"),    // cwd/locus_unity
    ];

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("../locus_unity")); // dev: target/debug/../../../locus_unity
            candidates.push(exe_dir.join("locus_unity")); // production: alongside exe
        }
    }

    let result = candidates
        .iter()
        .find(|p| p.join("Editor").is_dir())
        .cloned();
    if let Some(ref dir) = result {
        eprintln!(
            "[Locus] plugin source dir found: {:?}",
            dunce::canonicalize(dir).unwrap_or(dir.clone())
        );
    } else {
        eprintln!(
            "[Locus] plugin source dir NOT found! cwd={:?}, candidates checked: {:?}",
            std::env::current_dir().ok(),
            candidates
                .iter()
                .map(|c| format!("{} (exists={})", c.display(), c.join("Editor").is_dir()))
                .collect::<Vec<_>>()
        );
    }
    result
}

fn normalize_path_key(path: &Path) -> String {
    let normalized = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalized
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn expected_install_dir(project_path: &Path) -> PathBuf {
    project_path.join(PLUGIN_DEFAULT_INSTALL_DIR)
}

fn find_installed_plugin_dirs(project_path: &Path) -> Vec<InstalledPluginDir> {
    let search_roots = [
        (
            project_path.join("Packages"),
            PluginInstallLocation::Packages,
        ),
        (project_path.join("Assets"), PluginInstallLocation::Assets),
    ];

    let mut results = Vec::new();
    let mut seen = BTreeSet::new();

    for (search_root, location) in search_roots {
        if !search_root.is_dir() {
            continue;
        }

        for entry in walkdir::WalkDir::new(&search_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            if entry.file_name() != PLUGIN_ASMDEF_NAME {
                continue;
            }

            let Some(editor_dir) = entry.path().parent() else {
                continue;
            };
            let Some(plugin_root) = editor_dir.parent() else {
                continue;
            };

            if location == PluginInstallLocation::Packages
                && !plugin_root.join("package.json").is_file()
            {
                continue;
            }

            let key = normalize_path_key(plugin_root);
            if seen.insert(key) {
                results.push(InstalledPluginDir {
                    root: plugin_root.to_path_buf(),
                    location,
                });
            }
        }
    }

    results
}

fn remove_plugin_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove old plugin directory: {}", e))?;
    }

    let meta_path = PathBuf::from(format!("{}.meta", path.display()));
    if meta_path.exists() {
        std::fs::remove_file(&meta_path).map_err(|e| {
            format!(
                "Failed to remove plugin meta file {}: {}",
                meta_path.display(),
                e
            )
        })?;
    }

    Ok(())
}

fn copy_plugin_dir(source_dir: &Path, install_dir: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(source_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(source_dir)
            .map_err(|e| format!("strip_prefix: {}", e))?;
        let dest = install_dir.join(rel);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("Failed to create directory {}: {}", dest.display(), e))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let data = std::fs::read(entry.path())
                .map_err(|e| format!("Failed to read {}: {}", rel.display(), e))?;
            std::fs::write(&dest, &data)
                .map_err(|e| format!("Failed to write {}: {}", dest.display(), e))?;
        }
    }

    Ok(())
}

fn check_plugin_status_with_source_dir(
    source_dir: &Path,
    project_path: &Path,
) -> Result<PluginStatus, String> {
    let installed_dirs = find_installed_plugin_dirs(project_path);

    if installed_dirs.is_empty() {
        eprintln!(
            "[Locus] no installed plugin found in project: {}",
            project_path.display()
        );
        return Ok(PluginStatus::Missing);
    }

    if installed_dirs.len() > 1 {
        eprintln!(
            "[Locus] multiple plugin installs detected: {:?}",
            installed_dirs
                .iter()
                .map(|dir| dir.root.display().to_string())
                .collect::<Vec<_>>()
        );
        return Ok(PluginStatus::Outdated);
    }

    let install_dir = &installed_dirs[0];
    let expected_dir = expected_install_dir(project_path);
    if install_dir.location != PluginInstallLocation::Packages
        || normalize_path_key(&install_dir.root) != normalize_path_key(&expected_dir)
    {
        eprintln!(
            "[Locus] plugin install requires migration: current={}, expected={}",
            install_dir.root.display(),
            expected_dir.display()
        );
        return Ok(PluginStatus::Outdated);
    }

    let source_hash = compute_dir_hash(source_dir)?;
    let hash_file = install_dir.root.join(PLUGIN_HASH_FILE);
    let installed_hash = std::fs::read_to_string(&hash_file).unwrap_or_default();

    eprintln!(
        "[Locus] plugin hash check: source={}, installed={}",
        &source_hash[..16],
        if installed_hash.trim().len() >= 16 {
            &installed_hash.trim()[..16]
        } else {
            installed_hash.trim()
        }
    );

    if installed_hash.trim() == source_hash {
        Ok(PluginStatus::UpToDate)
    } else {
        Ok(PluginStatus::Outdated)
    }
}

fn install_or_update_plugin_with_source_dir(
    source_dir: &Path,
    project_path: &Path,
) -> Result<String, String> {
    let install_dir = expected_install_dir(project_path);

    for dir in find_installed_plugin_dirs(project_path) {
        remove_plugin_dir(&dir.root)?;
    }

    if install_dir.exists() {
        remove_plugin_dir(&install_dir)?;
    }

    copy_plugin_dir(source_dir, &install_dir)?;

    let hash = compute_dir_hash(source_dir)?;
    std::fs::write(install_dir.join(PLUGIN_HASH_FILE), &hash)
        .map_err(|e| format!("Failed to write hash file: {}", e))?;

    eprintln!(
        "[Locus] locus_unity plugin installed/updated at: {}",
        install_dir.display()
    );
    Ok(hash)
}

fn compute_dir_hash(dir: &std::path::Path) -> Result<String, String> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if entry.file_name() == PLUGIN_HASH_FILE {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(dir)
            .map_err(|e| format!("strip_prefix failed: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        let content = std::fs::read(entry.path()).map_err(|e| format!("read {}: {}", rel, e))?;
        entries.push((rel, content));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (rel, content) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(&(content.len() as u64).to_le_bytes());
        hasher.update(content);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn check_plugin_status(project_path: &str) -> Result<PluginStatus, String> {
    let source_dir = find_plugin_source_dir()
        .ok_or_else(|| "locus_unity source directory not found".to_string())?;

    let project = Path::new(strip_extended_path_prefix(project_path));
    check_plugin_status_with_source_dir(&source_dir, project)
}

pub fn install_or_update_plugin(project_path: &str) -> Result<String, String> {
    let source_dir = find_plugin_source_dir()
        .ok_or_else(|| "locus_unity source directory not found".to_string())?;

    let project = Path::new(strip_extended_path_prefix(project_path));
    install_or_update_plugin_with_source_dir(&source_dir, project)
}

pub fn emit_plugin_status(app_handle: &AppHandle, project_path: &str) {
    let status = check_plugin_status(project_path);
    eprintln!(
        "[Locus] plugin check result for '{}': {:?}",
        project_path, status
    );
    match status {
        Ok(status) => {
            let _ = app_handle.emit("unity-plugin-status", status);
        }
        Err(e) => {
            eprintln!("[Locus] plugin check error: {}", e);
            let _ = app_handle.emit("unity-plugin-status", PluginStatus::Missing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_source_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../locus_unity")
    }

    fn create_unity_project(project_root: &Path) {
        std::fs::create_dir_all(project_root.join("Assets")).unwrap();
        std::fs::create_dir_all(project_root.join("ProjectSettings")).unwrap();
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn missing_when_plugin_is_not_installed() {
        let temp = tempfile::tempdir().unwrap();
        create_unity_project(temp.path());

        let status =
            check_plugin_status_with_source_dir(&fixture_source_dir(), temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::Missing));
    }

    #[test]
    fn legacy_assets_install_is_outdated_even_when_hash_matches() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        let legacy_dir = temp.path().join("Assets/Locus");
        copy_plugin_dir(&source_dir, &legacy_dir).unwrap();
        let hash = compute_dir_hash(&source_dir).unwrap();
        write_file(&legacy_dir.join(PLUGIN_HASH_FILE), hash.as_bytes());

        let status = check_plugin_status_with_source_dir(&source_dir, temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::Outdated));
    }

    #[test]
    fn install_migrates_assets_plugin_into_embedded_package() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        write_file(
            &temp.path().join("Assets/Locus/Editor/Locus.Editor.asmdef"),
            b"legacy",
        );
        write_file(&temp.path().join("Assets/Locus.meta"), b"legacy-meta");

        install_or_update_plugin_with_source_dir(&source_dir, temp.path()).unwrap();

        assert!(!temp.path().join("Assets/Locus").exists());
        assert!(!temp.path().join("Assets/Locus.meta").exists());
        assert!(temp
            .path()
            .join("Packages/com.farlocus.locus/package.json")
            .is_file());

        let status = check_plugin_status_with_source_dir(&source_dir, temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::UpToDate));
    }

    #[test]
    fn duplicate_installs_report_outdated() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        install_or_update_plugin_with_source_dir(&source_dir, temp.path()).unwrap();
        write_file(
            &temp.path().join("Assets/Locus/Editor/Locus.Editor.asmdef"),
            b"legacy",
        );

        let status = check_plugin_status_with_source_dir(&source_dir, temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::Outdated));
    }
}
