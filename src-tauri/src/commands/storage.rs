use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::AppError;

const STORAGE_OVERRIDE_FILE: &str = "storage_dir_override.json";
const STORAGE_MIGRATION_PLAN_FILE: &str = "storage_dir_migration.json";
const STORAGE_LAST_DEFAULT_FILE: &str = "storage_last_default.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageOverrideConfig {
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageMigrationPlan {
    source_path: String,
    target_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageLastDefaultConfig {
    path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStorageInfo {
    pub active_path: String,
    pub default_path: String,
    pub active_size_bytes: u64,
    pub uses_custom_path: bool,
    pub pending_target_path: Option<String>,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTempInfo {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DefaultStorageSelection {
    Default(PathBuf),
    Fallback(PathBuf),
}

fn ensure_storage_dir(path: &Path, label: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(path)
        .map_err(|e| format!("Failed to create {} '{}': {}", label, path.display(), e))?;
    Ok(dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn legacy_app_storage_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    ensure_storage_dir(&dir, "app data dir")
}

pub(crate) fn packaged_runtime_storage_dir() -> Result<Option<PathBuf>, String> {
    let exe_path =
        std::env::current_exe().map_err(|e| format!("Failed to resolve current exe: {}", e))?;
    let Some(exe_dir) = exe_path.parent() else {
        return Ok(None);
    };
    if !looks_like_packaged_runtime_dir(exe_dir) {
        return Ok(None);
    }
    let data_dir = exe_dir.join("data");
    ensure_storage_dir(&data_dir, "packaged storage dir").map(Some)
}

pub(crate) fn default_app_storage_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    match packaged_runtime_storage_dir() {
        Ok(Some(dir)) => Ok(dir),
        Ok(None) => legacy_app_storage_dir(app_handle),
        Err(error) => {
            eprintln!(
                "[Locus] packaged storage dir unavailable, falling back to app data dir: {}",
                error
            );
            legacy_app_storage_dir(app_handle)
        }
    }
}

fn storage_override_path() -> Result<PathBuf, String> {
    Ok(super::persistent_config_dir()?.join(STORAGE_OVERRIDE_FILE))
}

fn storage_migration_plan_path() -> Result<PathBuf, String> {
    Ok(super::persistent_config_dir()?.join(STORAGE_MIGRATION_PLAN_FILE))
}

fn storage_last_default_path() -> Result<PathBuf, String> {
    Ok(super::persistent_config_dir()?.join(STORAGE_LAST_DEFAULT_FILE))
}

fn canonical_dir(path: &Path) -> Result<PathBuf, String> {
    let raw = path.to_path_buf();
    if !raw.exists() {
        return Err(format!("Directory not found: {}", raw.display()));
    }
    if !raw.is_dir() {
        return Err(format!("Path is not a directory: {}", raw.display()));
    }
    dunce::canonicalize(&raw)
        .map_err(|e| format!("Failed to resolve directory '{}': {}", raw.display(), e))
}

fn read_storage_override() -> Result<Option<PathBuf>, String> {
    let path = storage_override_path()?;
    let Some(raw) = std::fs::read_to_string(&path).ok() else {
        return Ok(None);
    };
    let cfg: StorageOverrideConfig =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid storage override: {}", e))?;
    let trimmed = cfg.path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let dir = PathBuf::from(trimmed);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create storage dir '{}': {}", dir.display(), e))?;
    canonical_dir(&dir).map(Some)
}

fn write_storage_override(path: Option<&Path>) -> Result<(), String> {
    let file = storage_override_path()?;
    match path {
        Some(dir) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create config dir '{}': {}", parent.display(), e)
                })?;
            }
            let cfg = StorageOverrideConfig {
                path: dir.display().to_string(),
            };
            let json = serde_json::to_string_pretty(&cfg)
                .map_err(|e| format!("Failed to serialize storage override: {}", e))?;
            std::fs::write(&file, json).map_err(|e| {
                format!(
                    "Failed to write storage override '{}': {}",
                    file.display(),
                    e
                )
            })?;
        }
        None => {
            let _ = std::fs::remove_file(&file);
        }
    }
    Ok(())
}

fn read_migration_plan() -> Result<Option<StorageMigrationPlan>, String> {
    let path = storage_migration_plan_path()?;
    let Some(raw) = std::fs::read_to_string(&path).ok() else {
        return Ok(None);
    };
    let plan: StorageMigrationPlan =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid storage migration plan: {}", e))?;
    if plan.source_path.trim().is_empty() || plan.target_path.trim().is_empty() {
        return Err("Storage migration plan is incomplete".to_string());
    }
    Ok(Some(plan))
}

fn write_migration_plan(plan: &StorageMigrationPlan) -> Result<(), String> {
    let file = storage_migration_plan_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir '{}': {}", parent.display(), e))?;
    }
    let json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize storage migration plan: {}", e))?;
    std::fs::write(&file, json).map_err(|e| {
        format!(
            "Failed to write storage migration plan '{}': {}",
            file.display(),
            e
        )
    })
}

fn clear_migration_plan() -> Result<(), String> {
    let file = storage_migration_plan_path()?;
    let _ = std::fs::remove_file(&file);
    Ok(())
}

fn path_contains_files(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory '{}': {}", path.display(), e))?;
    Ok(entries.next().is_some())
}

fn compute_dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

fn clear_dir_contents(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| {
        format!(
            "Failed to create temporary directory '{}': {}",
            path.display(),
            e
        )
    })?;
    for entry in std::fs::read_dir(path).map_err(|e| {
        format!(
            "Failed to read temporary directory '{}': {}",
            path.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("Failed to read temporary entry: {}", e))?;
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            format!(
                "Failed to inspect temporary entry '{}': {}",
                entry_path.display(),
                e
            )
        })?;
        if file_type.is_dir() {
            std::fs::remove_dir_all(&entry_path).map_err(|e| {
                format!(
                    "Failed to remove temporary directory '{}': {}",
                    entry_path.display(),
                    e
                )
            })?;
        } else {
            std::fs::remove_file(&entry_path).map_err(|e| {
                format!(
                    "Failed to remove temporary file '{}': {}",
                    entry_path.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

fn move_file_or_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory '{}': {}", parent.display(), e))?;
    }

    match std::fs::rename(src, dst) {
        Ok(_) => Ok(()),
        Err(_) => {
            if src.is_dir() {
                copy_dir_recursive(src, dst)?;
                std::fs::remove_dir_all(src).map_err(|e| {
                    format!(
                        "Failed to remove source directory '{}' after copy: {}",
                        src.display(),
                        e
                    )
                })
            } else {
                std::fs::copy(src, dst).map_err(|e| {
                    format!(
                        "Failed to copy '{}' to '{}': {}",
                        src.display(),
                        dst.display(),
                        e
                    )
                })?;
                std::fs::remove_file(src).map_err(|e| {
                    format!(
                        "Failed to remove source file '{}' after copy: {}",
                        src.display(),
                        e
                    )
                })
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create directory '{}': {}", dst.display(), e))?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| format!("Failed to read directory '{}': {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|e| format!("Failed to read file type '{}': {}", src_path.display(), e))?
            .is_dir()
        {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "Failed to copy '{}' to '{}': {}",
                    src_path.display(),
                    dst_path.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

fn move_storage_contents(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| {
        format!(
            "Failed to create target directory '{}': {}",
            target.display(),
            e
        )
    })?;
    for entry in std::fs::read_dir(source).map_err(|e| {
        format!(
            "Failed to read source directory '{}': {}",
            source.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("Failed to read source entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = target.join(entry.file_name());
        if dst_path.exists() {
            return Err(format!(
                "Target directory is not empty: '{}' already exists",
                dst_path.display()
            ));
        }
        move_file_or_dir(&src_path, &dst_path)?;
    }

    if source.exists() {
        let _ = std::fs::remove_dir(source);
    }
    Ok(())
}

fn read_last_default_storage_dir() -> Result<Option<PathBuf>, String> {
    let path = storage_last_default_path()?;
    let Some(raw) = std::fs::read_to_string(&path).ok() else {
        return Ok(None);
    };
    let cfg: StorageLastDefaultConfig =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid last default storage: {}", e))?;
    let trimmed = cfg.path.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let dir = PathBuf::from(trimmed);
    if !dir.exists() || !dir.is_dir() {
        return Ok(None);
    }
    Ok(Some(
        dunce::canonicalize(&dir).unwrap_or_else(|_| dir.to_path_buf()),
    ))
}

fn write_last_default_storage_dir(path: &Path) -> Result<(), String> {
    let file = storage_last_default_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir '{}': {}", parent.display(), e))?;
    }
    let cfg = StorageLastDefaultConfig {
        path: path.display().to_string(),
    };
    let json = serde_json::to_string_pretty(&cfg)
        .map_err(|e| format!("Failed to serialize last default storage: {}", e))?;
    std::fs::write(&file, json).map_err(|e| {
        format!(
            "Failed to write last default storage '{}': {}",
            file.display(),
            e
        )
    })
}

fn looks_like_packaged_runtime_dir(exe_dir: &Path) -> bool {
    ["agent", "knowledge", "locus_unity"]
        .iter()
        .any(|name| exe_dir.join(name).is_dir())
}

fn push_unique_storage_candidate(
    candidates: &mut Vec<PathBuf>,
    path: PathBuf,
    current_default: &Path,
) {
    if path == current_default || candidates.iter().any(|candidate| candidate == &path) {
        return;
    }
    candidates.push(path);
}

fn default_storage_candidates(
    app_handle: &AppHandle,
    current_default: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut candidates = Vec::new();
    if let Some(previous_default) = read_last_default_storage_dir()? {
        push_unique_storage_candidate(&mut candidates, previous_default, current_default);
    }
    let legacy = legacy_app_storage_dir(app_handle)?;
    push_unique_storage_candidate(&mut candidates, legacy, current_default);
    Ok(candidates)
}

fn adopt_existing_storage_into_default(
    current_default: &Path,
    candidates: &[PathBuf],
) -> Result<DefaultStorageSelection, String> {
    if path_contains_files(current_default)? {
        return Ok(DefaultStorageSelection::Default(
            current_default.to_path_buf(),
        ));
    }

    for candidate in candidates {
        if !path_contains_files(candidate)? {
            continue;
        }
        match move_storage_contents(candidate, current_default) {
            Ok(()) => {
                return Ok(DefaultStorageSelection::Default(
                    current_default.to_path_buf(),
                ))
            }
            Err(error) => {
                eprintln!(
                    "[Locus] failed to move default storage from '{}' to '{}': {}",
                    candidate.display(),
                    current_default.display(),
                    error
                );
                return Ok(DefaultStorageSelection::Fallback(candidate.clone()));
            }
        }
    }

    Ok(DefaultStorageSelection::Default(
        current_default.to_path_buf(),
    ))
}

fn build_storage_info(app_handle: &AppHandle) -> Result<AppStorageInfo, String> {
    let active = resolve_runtime_storage_dir(app_handle)?;
    let default = default_app_storage_dir(app_handle)?;
    let pending = read_migration_plan()?;
    let pending_target_path = pending.as_ref().map(|plan| plan.target_path.clone());
    Ok(AppStorageInfo {
        active_path: active.display().to_string(),
        default_path: default.display().to_string(),
        active_size_bytes: compute_dir_size(&active),
        uses_custom_path: active != default,
        pending_target_path,
        restart_required: pending.is_some(),
    })
}

fn app_temp_dir_for_storage(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let temp = resolve_runtime_storage_dir(app_handle)?.join("temp");
    super::set_app_temp_dir_override(temp)
}

fn build_temp_info(app_handle: &AppHandle) -> Result<AppTempInfo, String> {
    let temp = app_temp_dir_for_storage(app_handle)?;
    Ok(AppTempInfo {
        path: temp.display().to_string(),
        size_bytes: compute_dir_size(&temp),
    })
}

pub(crate) fn resolve_runtime_storage_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let default = default_app_storage_dir(app_handle)?;
    if let Some(custom) = read_storage_override()? {
        return Ok(custom);
    }
    Ok(default)
}

pub(crate) fn prepare_runtime_storage_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let default = default_app_storage_dir(app_handle)?;
    if let Some(plan) = read_migration_plan()? {
        let source = PathBuf::from(plan.source_path.trim());
        let target = PathBuf::from(plan.target_path.trim());
        let migration_result = (|| -> Result<(), String> {
            if source == target {
                if target == default {
                    write_storage_override(None)?;
                } else {
                    write_storage_override(Some(&target))?;
                }
                clear_migration_plan()?;
                return Ok(());
            }

            move_storage_contents(&source, &target)?;
            if target == default {
                write_storage_override(None)?;
            } else {
                write_storage_override(Some(&target))?;
            }
            clear_migration_plan()?;
            Ok(())
        })();

        if let Err(err) = migration_result {
            eprintln!("[Locus] storage migration failed: {}", err);
            if source.is_dir() {
                std::fs::create_dir_all(&source).ok();
                return Ok(source);
            }
            return Ok(default);
        }
    }

    if let Some(custom) = read_storage_override()? {
        let _ = write_last_default_storage_dir(&default);
        return Ok(custom);
    }

    let candidates = default_storage_candidates(app_handle, &default)?;
    let active = match adopt_existing_storage_into_default(&default, &candidates)? {
        DefaultStorageSelection::Default(path) => path,
        DefaultStorageSelection::Fallback(path) => {
            if let Err(error) = write_storage_override(Some(&path)) {
                eprintln!(
                    "[Locus] failed to persist fallback storage override '{}': {}",
                    path.display(),
                    error
                );
            }
            path
        }
    };
    let _ = write_last_default_storage_dir(&default);
    Ok(active)
}

#[tauri::command]
pub async fn get_app_storage_info(app_handle: AppHandle) -> Result<AppStorageInfo, AppError> {
    build_storage_info(&app_handle).map_err(Into::into)
}

#[tauri::command]
pub async fn get_app_temp_info(app_handle: AppHandle) -> Result<AppTempInfo, AppError> {
    tauri::async_runtime::spawn_blocking(move || build_temp_info(&app_handle))
        .await
        .map_err(|e| {
            AppError::new(
                "temp.info_join_failed",
                format!("Failed to load temporary file info: {}", e),
            )
        })?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_app_temp_dir(app_handle: AppHandle) -> Result<AppTempInfo, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let temp = app_temp_dir_for_storage(&app_handle)?;
        clear_dir_contents(&temp)?;
        build_temp_info(&app_handle)
    })
    .await
    .map_err(|e| {
        AppError::new(
            "temp.clear_join_failed",
            format!("Failed to clear temporary files: {}", e),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn open_app_storage_dir(app_handle: AppHandle) -> Result<(), AppError> {
    let active = resolve_runtime_storage_dir(&app_handle)?;
    crate::commands::knowledge::open_file_native(&active).map_err(Into::into)
}

#[tauri::command]
pub async fn open_app_temp_dir(app_handle: AppHandle) -> Result<(), AppError> {
    let temp = app_temp_dir_for_storage(&app_handle)?;
    crate::commands::knowledge::open_file_native(&temp).map_err(Into::into)
}

#[tauri::command]
pub async fn schedule_app_storage_migration(
    target_path: String,
    app_handle: AppHandle,
) -> Result<AppStorageInfo, AppError> {
    let active = resolve_runtime_storage_dir(&app_handle)?;
    let default = default_app_storage_dir(&app_handle)?;

    let trimmed = target_path.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "storage.invalid_target",
            "Storage directory cannot be empty",
        ));
    }

    let target = canonical_dir(Path::new(trimmed))?;

    if target == active {
        return Err(AppError::new(
            "storage.same_directory",
            "The selected directory is already in use",
        ));
    }

    if target.starts_with(&active) || active.starts_with(&target) {
        return Err(AppError::new(
            "storage.invalid_target",
            "The target directory cannot contain the current storage directory or be its parent",
        ));
    }

    if path_contains_files(&target)? {
        return Err(AppError::new(
            "storage.target_not_empty",
            "The target directory must be empty before migration",
        ));
    }

    let plan = StorageMigrationPlan {
        source_path: active.display().to_string(),
        target_path: target.display().to_string(),
    };
    write_migration_plan(&plan)?;

    let mut info = build_storage_info(&app_handle)?;
    info.default_path = default.display().to_string();
    Ok(info)
}

#[tauri::command]
pub async fn clear_app_storage_migration(
    app_handle: AppHandle,
) -> Result<AppStorageInfo, AppError> {
    clear_migration_plan()?;
    build_storage_info(&app_handle).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{
        adopt_existing_storage_into_default, looks_like_packaged_runtime_dir,
        DefaultStorageSelection,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn packaged_runtime_dir_requires_bundled_resources() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!looks_like_packaged_runtime_dir(dir.path()));

        fs::create_dir_all(dir.path().join("agent")).expect("create agent");
        assert!(looks_like_packaged_runtime_dir(dir.path()));
    }

    #[test]
    fn adopt_existing_storage_moves_previous_default_into_new_default() {
        let root = tempfile::tempdir().expect("tempdir");
        let previous = root.path().join("old-default");
        let current = root.path().join("new-default");
        fs::create_dir_all(&previous).expect("create previous");
        fs::write(previous.join("session.json"), b"hello").expect("write previous data");

        let selection = adopt_existing_storage_into_default(&current, &[previous.clone()])
            .expect("adopt existing storage");

        assert_eq!(selection, DefaultStorageSelection::Default(current.clone()));
        assert!(current.join("session.json").is_file());
        assert!(!previous.exists());
    }

    #[test]
    fn adopt_existing_storage_keeps_current_default_when_already_populated() {
        let root = tempfile::tempdir().expect("tempdir");
        let previous = root.path().join("old-default");
        let current = root.path().join("new-default");
        fs::create_dir_all(&previous).expect("create previous");
        fs::create_dir_all(&current).expect("create current");
        fs::write(previous.join("old.txt"), b"old").expect("write previous data");
        fs::write(current.join("new.txt"), b"new").expect("write current data");

        let selection = adopt_existing_storage_into_default(&current, &[previous.clone()])
            .expect("adopt existing storage");

        assert_eq!(selection, DefaultStorageSelection::Default(current.clone()));
        assert!(current.join("new.txt").is_file());
        assert!(previous.join("old.txt").is_file());
    }

    #[test]
    fn adopt_existing_storage_treats_existing_target_content_as_active_default() {
        let root = tempfile::tempdir().expect("tempdir");
        let previous = root.path().join("old-default");
        let current = root.path().join("new-default");
        fs::create_dir_all(&previous).expect("create previous");
        fs::create_dir_all(current.join("nested")).expect("create nested target");
        fs::write(previous.join("nested"), b"old").expect("write conflicting previous file");

        let selection = adopt_existing_storage_into_default(&current, &[PathBuf::from(&previous)])
            .expect("adopt existing storage");

        assert_eq!(selection, DefaultStorageSelection::Default(current.clone()));
        assert!(previous.join("nested").is_file());
        assert!(current.join("nested").is_dir());
    }
}
