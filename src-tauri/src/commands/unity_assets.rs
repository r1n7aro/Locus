use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

#[tauri::command]
pub async fn unity_assets_execute(app: AppHandle, workspace_ref: WorkspaceRef, request: Value,
    registry: State<'_, Arc<ProjectRegistry>>) -> Result<Value, String> {
    let scope=registry.resolve_workspace_ref(&workspace_ref).map_err(|e|e.to_string())?;
    let action=request["action"].as_str().unwrap_or("");
    let _guard=if action.starts_with("apply") || action=="recover" {
        Some(crate::merge_jobs::coordination::acquire(&app,scope.runtime(),"assets.apply",None).await?)
    } else {None};
    crate::unity_assets::execute(scope.runtime().root(),request).await
}
