use std::collections::HashMap;
use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::extra_workdirs::{self, ExtraWorkdirEntry, ExtraWorkdirStatus};
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

fn workspace_dir(
    registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<String, AppError> {
    registry
        .resolve_workspace_ref(workspace_ref)
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .map_err(|error| {
            AppError::new("workspace.scope_invalid", error.to_string()).operation(operation)
        })
}

#[tauri::command]
pub async fn extra_workdirs_get(
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<ExtraWorkdirStatus>, AppError> {
    let dir = workspace_dir(registry.inner(), &workspace_ref, "extra_workdirs_get")?;
    Ok(extra_workdirs::load_statuses(&dir))
}

#[tauri::command]
pub async fn extra_workdirs_set(
    workspace_ref: WorkspaceRef,
    entries: Vec<ExtraWorkdirEntry>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<ExtraWorkdirStatus>, AppError> {
    let dir = workspace_dir(registry.inner(), &workspace_ref, "extra_workdirs_set")?;
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
    workspace_refs: Vec<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<HashMap<String, Vec<ExtraWorkdirStatus>>, AppError> {
    let mut map = HashMap::new();
    for workspace_ref in workspace_refs {
        let path = workspace_dir(registry.inner(), &workspace_ref, "extra_workdirs_map")?;
        let statuses = extra_workdirs::load_statuses(&path);
        if !statuses.is_empty() {
            map.insert(path, statuses);
        }
    }
    Ok(map)
}
