use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
const PLUGIN_SKILLS_DIR: &str = "Editor/Skills";
const PLUGIN_ASMDEF_NAME: &str = "Locus.Editor.asmdef";
const PLUGIN_HASH_FILE: &str = ".locus_plugin_hash";
const PLUGIN_LEGACY_ASSETS_INSTALL_DIRS: &[&str] = &["Assets/Locus", "Assets/Plugins/Locus"];
const PLUGIN_REQUIRED_SOURCE_FILES: &[&str] = &[
    "package.json",
    "Editor/Locus.Editor.asmdef",
    "Editor/Json/Locus.Json.dll",
    "Editor/Json/Locus.Json.dll.meta",
    "Editor/Roslyn/Locus.Roslyn.dll",
    "Editor/Roslyn/Locus.Roslyn.dll.meta",
    "Editor/Detour/Locus.Detour.dll",
    "Editor/Detour/Locus.Detour.dll.meta",
    "Editor/HotReload/Locus.HotReload.Runtime.dll",
    "Editor/HotReload/Locus.HotReload.Runtime.dll.meta",
];

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

pub fn plugin_install_root(project_path: &Path) -> PathBuf {
    expected_install_dir(project_path)
}

pub fn plugin_skills_root(project_path: &Path) -> PathBuf {
    expected_install_dir(project_path).join(PLUGIN_SKILLS_DIR)
}

fn plugin_meta_path(path: &Path) -> PathBuf {
    let mut meta = path.as_os_str().to_os_string();
    meta.push(".meta");
    PathBuf::from(meta)
}

fn plugin_dir_or_meta_exists(path: &Path) -> bool {
    path.exists() || plugin_meta_path(path).exists()
}

fn push_installed_plugin_dir(
    results: &mut Vec<InstalledPluginDir>,
    seen: &mut BTreeSet<String>,
    root: PathBuf,
    location: PluginInstallLocation,
) {
    let key = normalize_path_key(&root);
    if seen.insert(key) {
        results.push(InstalledPluginDir { root, location });
    }
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

    let expected_dir = expected_install_dir(project_path);
    if plugin_dir_or_meta_exists(&expected_dir) {
        push_installed_plugin_dir(
            &mut results,
            &mut seen,
            expected_dir,
            PluginInstallLocation::Packages,
        );
    }

    for legacy_dir in PLUGIN_LEGACY_ASSETS_INSTALL_DIRS {
        let root = project_path.join(legacy_dir);
        if plugin_dir_or_meta_exists(&root) {
            push_installed_plugin_dir(&mut results, &mut seen, root, PluginInstallLocation::Assets);
        }
    }

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

            push_installed_plugin_dir(&mut results, &mut seen, plugin_root.to_path_buf(), location);
        }
    }

    results
}

fn remove_plugin_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove old plugin directory: {}", e))?;
    }

    let meta_path = plugin_meta_path(path);
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

fn plugin_source_rel_key(source_dir: &Path, path: &Path) -> Result<String, String> {
    let rel_path = path
        .strip_prefix(source_dir)
        .map_err(|e| format!("strip_prefix failed: {}", e))?;
    Ok(rel_path.to_string_lossy().replace('\\', "/"))
}

fn should_skip_plugin_source_entry(source_dir: &Path, path: &Path) -> bool {
    let Ok(rel) = plugin_source_rel_key(source_dir, path) else {
        return false;
    };

    if rel.is_empty() {
        return false;
    }

    if rel == PLUGIN_HASH_FILE {
        return true;
    }

    if rel == format!("{}.meta", PLUGIN_SKILLS_DIR)
        || rel.starts_with(&format!("{}/", PLUGIN_SKILLS_DIR))
    {
        return true;
    }

    if rel.starts_with("Editor/Roslyn/ILRepack-") {
        return true;
    }

    if rel.starts_with("Editor/Roslyn/Locus.Roslyn.dll.")
        && rel != "Editor/Roslyn/Locus.Roslyn.dll.meta"
    {
        return true;
    }

    if rel.starts_with("Editor/Json/ILRepack-") {
        return true;
    }

    if rel.starts_with("Editor/Detour/ILRepack-") {
        return true;
    }

    if rel.starts_with("Editor/Detour/Locus.Detour.dll.")
        && rel != "Editor/Detour/Locus.Detour.dll.meta"
    {
        return true;
    }

    if rel.starts_with("Editor/HotReload/Locus.HotReload.Runtime.dll.")
        && rel != "Editor/HotReload/Locus.HotReload.Runtime.dll.meta"
    {
        return true;
    }

    rel.starts_with("Editor/Json/Locus.Json.dll.") && rel != "Editor/Json/Locus.Json.dll.meta"
}

fn validate_plugin_source_dir(source_dir: &Path) -> Result<(), String> {
    let missing = PLUGIN_REQUIRED_SOURCE_FILES
        .iter()
        .filter(|rel| !source_dir.join(rel).is_file())
        .map(|rel| source_dir.join(rel).display().to_string())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Locus Unity plugin source is incomplete. Missing required file(s): {}",
        missing.join(", ")
    ))
}

fn copy_plugin_dir(source_dir: &Path, install_dir: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(source_dir)
        .into_iter()
        .filter_entry(|e| !should_skip_plugin_source_entry(source_dir, e.path()))
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

fn copy_dir_contents(source_dir: &Path, target_dir: &Path) -> Result<(), String> {
    if !source_dir.is_dir() {
        return Ok(());
    }

    for entry in walkdir::WalkDir::new(source_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(source_dir)
            .map_err(|e| format!("strip_prefix: {}", e))?;
        let dest = target_dir.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("Failed to create directory {}: {}", dest.display(), e))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create directory {}: {}", parent.display(), e)
                })?;
            }
            std::fs::copy(entry.path(), &dest).map_err(|e| {
                format!(
                    "Failed to preserve plugin file {} -> {}: {}",
                    entry.path().display(),
                    dest.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

fn preserve_installed_skill_files(install_dir: &Path, staging_dir: &Path) -> Result<(), String> {
    let source_skills = install_dir.join(PLUGIN_SKILLS_DIR);
    if source_skills.is_dir() {
        copy_dir_contents(&source_skills, &staging_dir.join(PLUGIN_SKILLS_DIR))?;
    }

    let source_skills_meta = plugin_meta_path(&source_skills);
    if source_skills_meta.is_file() {
        let target_skills_meta = plugin_meta_path(&staging_dir.join(PLUGIN_SKILLS_DIR));
        if let Some(parent) = target_skills_meta.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
        }
        std::fs::copy(&source_skills_meta, &target_skills_meta).map_err(|e| {
            format!(
                "Failed to preserve plugin skills meta {} -> {}: {}",
                source_skills_meta.display(),
                target_skills_meta.display(),
                e
            )
        })?;
    }

    Ok(())
}

fn unique_staging_dir(project_path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    project_path
        .join("Temp")
        .join("LocusPluginInstall")
        .join(format!(
            "com.farlocus.locus-{}-{}",
            std::process::id(),
            timestamp
        ))
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

    validate_plugin_source_dir(source_dir)?;

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
    let installed_hash_trimmed = installed_hash.trim();
    let installed_hash_display = if installed_hash_trimmed.len() >= 16 {
        installed_hash_trimmed.chars().take(16).collect::<String>()
    } else {
        installed_hash_trimmed.to_string()
    };

    eprintln!(
        "[Locus] plugin hash check: source={}, installed={}",
        &source_hash[..16],
        installed_hash_display
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
    validate_plugin_source_dir(source_dir)?;

    let install_dir = expected_install_dir(project_path);
    let installed_dirs = find_installed_plugin_dirs(project_path);
    let staging_dir = unique_staging_dir(project_path);
    if let Some(parent) = staging_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create plugin staging directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir).map_err(|e| {
            format!(
                "Failed to remove stale plugin staging directory {}: {}",
                staging_dir.display(),
                e
            )
        })?;
    }

    copy_plugin_dir(source_dir, &staging_dir)?;
    validate_plugin_source_dir(&staging_dir)?;

    let hash = compute_dir_hash(source_dir)?;
    std::fs::write(staging_dir.join(PLUGIN_HASH_FILE), &hash)
        .map_err(|e| format!("Failed to write staged hash file: {}", e))?;

    if install_dir.is_dir() {
        preserve_installed_skill_files(&install_dir, &staging_dir)?;
    }

    for dir in installed_dirs {
        if normalize_path_key(&dir.root) != normalize_path_key(&install_dir) {
            preserve_installed_skill_files(&dir.root, &staging_dir)?;
        }
        remove_plugin_dir(&dir.root)?;
    }

    if install_dir.exists() {
        remove_plugin_dir(&install_dir)?;
    }

    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
    }
    std::fs::rename(&staging_dir, &install_dir).map_err(|e| {
        format!(
            "Failed to move staged plugin into {}: {}",
            install_dir.display(),
            e
        )
    })?;

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
        .filter_entry(|e| !should_skip_plugin_source_entry(dir, e.path()))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel = plugin_source_rel_key(dir, entry.path())?;
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
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn create_minimal_plugin_source(source_root: &Path) {
        write_file(&source_root.join("package.json"), b"{}");
        write_file(&source_root.join("Editor/Locus.Editor.asmdef"), b"{}");
        write_file(&source_root.join("Editor/Json/Locus.Json.dll"), b"dll");
        write_file(
            &source_root.join("Editor/Json/Locus.Json.dll.meta"),
            b"meta",
        );
        write_file(&source_root.join("Editor/Roslyn/Locus.Roslyn.dll"), b"dll");
        write_file(
            &source_root.join("Editor/Roslyn/Locus.Roslyn.dll.meta"),
            b"meta",
        );
        write_file(&source_root.join("Editor/Detour/Locus.Detour.dll"), b"dll");
        write_file(
            &source_root.join("Editor/Detour/Locus.Detour.dll.meta"),
            b"meta",
        );
        write_file(
            &source_root.join("Editor/HotReload/Locus.HotReload.Runtime.dll"),
            b"dll",
        );
        write_file(
            &source_root.join("Editor/HotReload/Locus.HotReload.Runtime.dll.meta"),
            b"meta",
        );
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

    #[test]
    fn stale_assets_plugins_locus_skeleton_is_removed_on_install() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        write_file(
            &temp.path().join("Assets/Plugins/Locus.meta"),
            b"legacy-meta",
        );
        write_file(
            &temp.path().join("Assets/Plugins/Locus/Editor.meta"),
            b"legacy-editor-meta",
        );
        write_file(
            &temp.path().join("Assets/Plugins/Locus/Editor/Roslyn.meta"),
            b"legacy-roslyn-meta",
        );

        install_or_update_plugin_with_source_dir(&source_dir, temp.path()).unwrap();

        assert!(!temp.path().join("Assets/Plugins/Locus").exists());
        assert!(!temp.path().join("Assets/Plugins/Locus.meta").exists());
        assert!(temp
            .path()
            .join("Packages/com.farlocus.locus/package.json")
            .is_file());
    }

    #[test]
    fn install_rejects_incomplete_source_without_merged_roslyn() {
        let temp = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        create_unity_project(temp.path());

        write_file(&source.path().join("package.json"), b"{}");
        write_file(&source.path().join("Editor/Locus.Editor.asmdef"), b"{}");
        write_file(&source.path().join("Editor/Json/Locus.Json.dll"), b"dll");
        write_file(
            &source.path().join("Editor/Json/Locus.Json.dll.meta"),
            b"meta",
        );
        write_file(
            &source.path().join("Editor/Roslyn/Locus.Roslyn.dll.meta"),
            b"meta",
        );

        let error =
            install_or_update_plugin_with_source_dir(source.path(), temp.path()).unwrap_err();
        assert!(error.contains("Locus.Roslyn.dll"));
        assert!(!temp.path().join("Packages/com.farlocus.locus").exists());
    }

    #[test]
    fn copy_plugin_dir_skips_ilrepack_temp_artifacts() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        create_minimal_plugin_source(source.path());
        write_file(
            &source.path().join("Editor/Roslyn/ILRepack-123456/temp.dll"),
            b"temp",
        );
        write_file(
            &source.path().join("Editor/Roslyn/Locus.Roslyn.dll.tmp"),
            b"temp",
        );
        write_file(
            &source.path().join("Editor/Json/ILRepack-123456/temp.dll"),
            b"temp",
        );
        write_file(
            &source.path().join("Editor/Json/Locus.Json.dll.tmp"),
            b"temp",
        );

        copy_plugin_dir(source.path(), target.path()).unwrap();

        assert!(target
            .path()
            .join("Editor/Roslyn/Locus.Roslyn.dll")
            .is_file());
        assert!(target.path().join("Editor/Json/Locus.Json.dll").is_file());
        assert!(!target.path().join("Editor/Roslyn/ILRepack-123456").exists());
        assert!(!target
            .path()
            .join("Editor/Roslyn/Locus.Roslyn.dll.tmp")
            .exists());
        assert!(!target.path().join("Editor/Json/ILRepack-123456").exists());
        assert!(!target
            .path()
            .join("Editor/Json/Locus.Json.dll.tmp")
            .exists());
    }

    #[test]
    fn install_copies_merged_editor_bundles_without_original_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        install_or_update_plugin_with_source_dir(&source_dir, temp.path()).unwrap();

        let installed_root = temp.path().join(PLUGIN_DEFAULT_INSTALL_DIR);
        assert!(!installed_root
            .join("Editor/Roslyn/System.Runtime.CompilerServices.Unsafe.dll")
            .exists());
        assert!(!installed_root
            .join("Editor/Roslyn/System.Runtime.CompilerServices.Unsafe.dll.meta")
            .exists());
        assert!(installed_root
            .join("Editor/Roslyn/Locus.Roslyn.dll")
            .is_file());
        assert!(installed_root.join("Editor/Json/Locus.Json.dll").is_file());
        assert!(!installed_root
            .join("Editor/Roslyn/Microsoft.CodeAnalysis.dll")
            .exists());
        assert!(!installed_root
            .join("Editor/Json/Newtonsoft.Json.dll")
            .exists());

        let status = check_plugin_status_with_source_dir(&source_dir, temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::UpToDate));
    }

    #[test]
    fn plugin_hash_ignores_installed_skill_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        create_unity_project(temp.path());
        create_minimal_plugin_source(source.path());

        install_or_update_plugin_with_source_dir(source.path(), temp.path()).unwrap();
        let skill_file = temp
            .path()
            .join(PLUGIN_DEFAULT_INSTALL_DIR)
            .join(PLUGIN_SKILLS_DIR)
            .join("com.example.skill/Bridge.cs");
        write_file(&skill_file, b"public static class Bridge {}");

        let status = check_plugin_status_with_source_dir(source.path(), temp.path()).unwrap();
        assert!(matches!(status, PluginStatus::UpToDate));
    }

    #[test]
    fn install_preserves_installed_skill_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        create_unity_project(temp.path());
        create_minimal_plugin_source(source.path());

        install_or_update_plugin_with_source_dir(source.path(), temp.path()).unwrap();
        let install_dir = temp.path().join(PLUGIN_DEFAULT_INSTALL_DIR);
        let skill_file = install_dir
            .join(PLUGIN_SKILLS_DIR)
            .join("com.example.skill/Bridge.cs");
        let skills_meta = plugin_meta_path(&install_dir.join(PLUGIN_SKILLS_DIR));
        write_file(&skill_file, b"public static class Bridge {}");
        write_file(&skills_meta, b"fileFormatVersion: 2");
        write_file(&source.path().join("Editor/BridgeHost.cs"), b"host update");

        install_or_update_plugin_with_source_dir(source.path(), temp.path()).unwrap();

        assert_eq!(
            std::fs::read(&skill_file).unwrap(),
            b"public static class Bridge {}"
        );
        assert_eq!(
            std::fs::read(&skills_meta).unwrap(),
            b"fileFormatVersion: 2"
        );
    }

    #[test]
    fn update_from_old_plugin_keeps_merged_roslyn() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = fixture_source_dir();
        create_unity_project(temp.path());

        copy_plugin_dir(&source_dir, &temp.path().join("Assets/Locus")).unwrap();
        install_or_update_plugin_with_source_dir(&source_dir, temp.path()).unwrap();

        assert!(temp
            .path()
            .join("Packages/com.farlocus.locus/Editor/Roslyn/Locus.Roslyn.dll",)
            .is_file());
        assert!(temp
            .path()
            .join("Packages/com.farlocus.locus/Editor/Json/Locus.Json.dll",)
            .is_file());
    }
}
