use std::collections::HashMap;

use crate::error::AppError;
use crate::extra_workdirs::{self, ExtraWorkdirEntry, ExtraWorkdirStatus};

fn validated_workspace_dir(workspace_path: &str) -> Result<String, AppError> {
    let trimmed = workspace_path.trim();
    if trimmed.is_empty() {
        return Err("Workspace path cannot be empty".to_string().into());
    }
    if !std::path::Path::new(trimmed).is_dir() {
        return Err(format!("Directory not found: {}", trimmed).into());
    }
    Ok(trimmed.to_string())
}

#[tauri::command]
pub async fn extra_workdirs_get(
    workspace_path: String,
) -> Result<Vec<ExtraWorkdirStatus>, AppError> {
    let dir = validated_workspace_dir(&workspace_path)?;
    Ok(extra_workdirs::load_statuses(&dir))
}

#[tauri::command]
pub async fn extra_workdirs_set(
    workspace_path: String,
    entries: Vec<ExtraWorkdirEntry>,
) -> Result<Vec<ExtraWorkdirStatus>, AppError> {
    let dir = validated_workspace_dir(&workspace_path)?;
    let normalized = extra_workdirs::normalize_entries(&dir, entries);
    extra_workdirs::save_entries(&dir, &normalized)
        .map_err(|e| AppError::new("workspace.extra_workdirs_write_failed", e))?;
    Ok(extra_workdirs::entry_statuses(&normalized))
}

/// Batch lookup for the workspace selector: returns attachment statuses for
/// each requested workspace path, keyed by the path exactly as passed in.
/// Workspaces without attachments are omitted.
#[tauri::command]
pub async fn extra_workdirs_map(
    paths: Vec<String>,
) -> Result<HashMap<String, Vec<ExtraWorkdirStatus>>, AppError> {
    let mut map = HashMap::new();
    for path in paths {
        if path.trim().is_empty() {
            continue;
        }
        let statuses = extra_workdirs::load_statuses(path.trim());
        if !statuses.is_empty() {
            map.insert(path, statuses);
        }
    }
    Ok(map)
}
