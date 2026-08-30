use crate::error::AppError;
use crate::session::store::SessionStore;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};

fn sanitize_slug(path: &str) -> String {
    let last_segment = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    last_segment
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Deterministic per-session plan file location:
/// `{runtime_storage_dir}/plan/{project_slug}/{session_id_first8}.md`.
/// Deliberately outside the workspace (like Claude Code's ~/.claude/plans)
/// so planning never dirties the user's project.
pub fn plan_file_path_for_session(
    app_handle: &AppHandle,
    working_dir: &str,
    session_id: &str,
) -> Result<PathBuf, String> {
    let project_slug = sanitize_slug(working_dir);
    let data_dir = crate::commands::resolve_runtime_storage_dir(app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let sid_short: String = session_id.chars().take(8).collect();
    Ok(data_dir
        .join("plan")
        .join(&project_slug)
        .join(format!("{}.md", sid_short)))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlanStatePayload {
    pub active: bool,
    pub plan_file_path: String,
    pub plan_file_exists: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFileContentPayload {
    pub plan_file_path: String,
    pub content: String,
}

/// Read a plan file for the standalone plan review window. Only files under
/// the runtime storage `plan/` root are readable — this is a frontend-facing
/// command and must not become an arbitrary-file read primitive.
#[tauri::command]
pub async fn get_plan_file_content(
    session_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<PlanFileContentPayload, AppError> {
    let scope = super::session::resolve_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        None,
        "get_plan_file_content",
    )?;
    let requested = plan_file_path_for_session(
        &app_handle,
        &scope.runtime().root().to_string_lossy(),
        &session_id,
    )
    .map_err(|error| AppError::new("plan.path_failed", error).operation("plan"))?;
    let content = std::fs::read_to_string(&requested).map_err(|e| {
        AppError::new(
            "plan.read_failed",
            format!("Failed to read plan file: {}", e),
        )
        .operation("plan")
    })?;
    Ok(PlanFileContentPayload {
        plan_file_path: requested.to_string_lossy().to_string(),
        content,
    })
}

fn build_plan_state_payload(
    app_handle: &AppHandle,
    working_dir: &str,
    session_id: &str,
    active: bool,
) -> Result<SessionPlanStatePayload, AppError> {
    let plan_file = plan_file_path_for_session(app_handle, working_dir, session_id)
        .map_err(|e| AppError::new("plan.path_failed", e).operation("plan"))?;
    Ok(SessionPlanStatePayload {
        active,
        plan_file_exists: plan_file.is_file(),
        plan_file_path: plan_file.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn get_session_plan_state(
    session_id: String,
    workspace_ref: Option<WorkspaceRef>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<SessionPlanStatePayload, AppError> {
    let scope = super::session::resolve_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        workspace_ref.as_ref(),
        "get_session_plan_state",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    let state = store
        .get_plan_mode_state(&session_id)
        .map_err(|e| AppError::new("plan.state_failed", e).operation("plan"))?;
    build_plan_state_payload(&app_handle, &cwd, &session_id, state.active)
}

/// User-driven sticky plan-mode toggle (the /plan command with no message,
/// or the plan badge dismiss). exit_plan_mode approval flows through the
/// agent loop instead and does not use this command.
#[tauri::command]
pub async fn set_session_plan_mode(
    session_id: String,
    active: bool,
    workspace_ref: Option<WorkspaceRef>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<SessionPlanStatePayload, AppError> {
    let scope = super::session::resolve_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        workspace_ref.as_ref(),
        "set_session_plan_mode",
    )?;
    let runtime = scope.runtime();
    let cwd = runtime.root().to_string_lossy().to_string();
    let state = store
        .set_plan_mode_active(&session_id, active)
        .map_err(|e| AppError::new("plan.state_failed", e).operation("plan"))?;
    let payload = build_plan_state_payload(&app_handle, &cwd, &session_id, state.active)?;
    // Keep every window bound to this checkout in sync without creating a
    // phantom runtime snapshot for the synthetic toggle run.
    workspace_registry.event_router().publish(
        &app_handle,
        "stream-event",
        crate::workspace_service::event::WorkspaceEventEnvelope {
            project_id: runtime.project_id().clone(),
            checkout_id: runtime.checkout_id().clone(),
            workspace_generation: runtime.generation(),
            service_instance_id: None,
            service_generation: None,
            payload: crate::commands::StreamEventEnvelope {
                run_id: format!("{}_plan_toggle", session_id),
                project_id: Some(runtime.project_id().to_string()),
                checkout_id: Some(runtime.checkout_id().to_string()),
                workspace_generation: Some(runtime.generation()),
                event: crate::commands::StreamEvent::PlanModeChanged {
                    session_id,
                    active: state.active,
                    plan_file_path: Some(payload.plan_file_path.clone()),
                },
            },
        },
    );
    Ok(payload)
}
