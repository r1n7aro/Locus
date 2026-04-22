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

const PLUGIN_DEFAULT_INSTALL_DIR: &str = "Assets/Locus";
const PLUGIN_ASMDEF_NAME: &str = "Locus.Editor.asmdef";
const PLUGIN_HASH_FILE: &str = ".locus_plugin_hash";

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

fn find_installed_plugin_dir(project_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let assets_dir = project_path.join("Assets");
    for entry in walkdir::WalkDir::new(&assets_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if entry.file_name() == PLUGIN_ASMDEF_NAME {
            if let Some(editor_dir) = entry.path().parent() {
                if let Some(plugin_root) = editor_dir.parent() {
                    return Some(plugin_root.to_path_buf());
                }
            }
        }
    }
    None
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

    let project = std::path::Path::new(strip_extended_path_prefix(project_path));

    let install_dir = match find_installed_plugin_dir(project) {
        Some(dir) => {
            eprintln!("[Locus] installed plugin found at: {}", dir.display());
            dir
        }
        None => {
            eprintln!(
                "[Locus] no installed plugin found in project: {}",
                project.display()
            );
            return Ok(PluginStatus::Missing);
        }
    };

    let source_hash = compute_dir_hash(&source_dir)?;

    let hash_file = install_dir.join(PLUGIN_HASH_FILE);
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

pub fn install_or_update_plugin(project_path: &str) -> Result<String, String> {
    let source_dir = find_plugin_source_dir()
        .ok_or_else(|| "locus_unity source directory not found".to_string())?;

    let project = std::path::Path::new(strip_extended_path_prefix(project_path));

    let install_dir = find_installed_plugin_dir(project)
        .unwrap_or_else(|| project.join(PLUGIN_DEFAULT_INSTALL_DIR));

    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)
            .map_err(|e| format!("Failed to remove old plugin directory: {}", e))?;
    }

    for entry in walkdir::WalkDir::new(&source_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(&source_dir)
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

    let hash = compute_dir_hash(&source_dir)?;
    std::fs::write(install_dir.join(PLUGIN_HASH_FILE), &hash)
        .map_err(|e| format!("Failed to write hash file: {}", e))?;

    eprintln!(
        "[Locus] locus_unity plugin installed/updated at: {}",
        install_dir.display()
    );
    Ok(hash)
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
