use crate::workspace_service::{ProjectRegistry, WorkspaceRef};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn merge_job_prepare(
    app: AppHandle,
    workspace_ref: WorkspaceRef,
    request: crate::merge_jobs::PrepareRequest,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Value, String> {
    let scope = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(|e| e.to_string())?;
    if request
        .project_id
        .as_ref()
        .map(|id| id != &scope.runtime().project_id().to_string())
        .unwrap_or(false)
    {
        return Err("Merge request project does not own the destination checkout".into());
    }
    let root = scope.runtime().root().to_path_buf();
    let _guard =
        crate::merge_jobs::coordination::acquire(&app, scope.runtime(), "prepare", None).await?;
    let job = crate::merge_jobs::coordination::run_blocking(_guard, move || {
        let _scope = scope;
        crate::merge_jobs::prepare(&root, &request)
    })
    .await
    ??;
    Ok(crate::merge_jobs::summary(&job))
}

#[tauri::command]
pub async fn merge_job_execute(
    app: AppHandle,
    workspace_ref: WorkspaceRef,
    job_id: String,
    action: String,
    params: Value,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Value, String> {
    let scope = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(|e| e.to_string())?;
    let root = scope.runtime().root().to_path_buf();
    let _guard = if crate::merge_jobs::coordination::needs_guard(&action, &params) {
        Some(crate::merge_jobs::coordination::acquire(&app, scope.runtime(), &action, None).await?)
    } else {
        None
    };
    if action == "apply" {
        crate::merge_jobs::preflight_apply(&root, &job_id).await?;
    }
    if action == "validate" && params.get("level").and_then(Value::as_str) == Some("unity") {
        if params.get("paths").and_then(Value::as_array).is_some() {
            return crate::merge_jobs::validate_commit_unity(&app, &root, &job_id, params).await;
        }
        return crate::merge_jobs::validate_unity(&root, &job_id).await;
    }
    crate::merge_jobs::coordination::run_blocking(_guard, move || {
        let _scope = scope;
        crate::merge_jobs::execute(&root, &job_id, &action, params)
    })
    .await
    ?
}
