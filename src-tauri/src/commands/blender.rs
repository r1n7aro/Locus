//! Blender IPC commands: installation table and add-on (Settings > Blender),
//! the status popover (instances, targets, background instances, launch)
//! and the workspace version. Status queries never start anything: without
//! a running Blender service they report `running: false`.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use crate::blender::{installations, manage, target};
use crate::error::AppError;
use crate::workspace_service::service::ServiceKind;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

fn blender_error(code: &'static str, message: &'static str, detail: impl Into<String>) -> AppError {
    AppError::new(code, message).detail(detail.into())
}

fn require_enabled() -> Result<(), AppError> {
    if crate::blender::blender_integration_enabled() {
        Ok(())
    } else {
        Err(AppError::new(
            "blender.disabled",
            "Blender integration is turned off in Settings > Services.",
        ))
    }
}

fn emit_installations_changed() {
    if let Some(app) = crate::blender::hub::app_handle() {
        use tauri::Emitter;
        let _ = app.emit("blender-installations-changed", ());
    }
}

// ── installations ──────────────────────────────────────────────────────────

/// Current table; unprobed executables are probed in the background and a
/// `blender-installations-changed` event follows each result.
#[tauri::command]
pub async fn blender_list_installations() -> Result<Vec<installations::Installation>, AppError> {
    Ok(installations::list(Some(Arc::new(emit_installations_changed))))
}

#[tauri::command]
pub async fn blender_add_installation(path: String) -> Result<installations::Installation, AppError> {
    let installation = installations::add_manual(&path)
        .await
        .map_err(|error| blender_error(error.code(), "Could not add this Blender.", error.detail()))?;
    emit_installations_changed();
    Ok(installation)
}

#[tauri::command]
pub async fn blender_remove_installation(id: String) -> Result<(), AppError> {
    installations::remove_or_hide(&id)
        .map_err(|error| blender_error("blender.remove_installation_failed", "Could not remove this Blender.", error))?;
    emit_installations_changed();
    Ok(())
}

#[tauri::command]
pub async fn blender_unhide_installation(id: String) -> Result<(), AppError> {
    installations::unhide(&id)
        .map_err(|error| blender_error("blender.unhide_installation_failed", "Could not show this Blender.", error))?;
    emit_installations_changed();
    Ok(())
}

#[tauri::command]
pub async fn blender_set_default_installation(id: Option<String>) -> Result<(), AppError> {
    installations::set_default(id.as_deref())
        .map_err(|error| blender_error("blender.default_installation_failed", "Could not set the default Blender.", error))?;
    emit_installations_changed();
    Ok(())
}

/// Forget cached probe results and probe every executable again.
#[tauri::command]
pub async fn blender_refresh_installations() -> Result<Vec<installations::Installation>, AppError> {
    installations::invalidate_probes()
        .map_err(|error| blender_error("blender.refresh_failed", "Could not refresh the Blender list.", error))?;
    Ok(installations::list(Some(Arc::new(emit_installations_changed))))
}

// ── add-on ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn blender_install_addon(
    installation: Option<String>,
    all: Option<bool>,
) -> Result<Vec<manage::InstallReport>, AppError> {
    require_enabled()?;
    manage::install_selected(installation.as_deref(), all.unwrap_or(false))
        .await
        .map_err(|error| blender_error("blender.install_failed", "Could not install the Locus add-on.", error))
}

#[tauri::command]
pub async fn blender_uninstall_addon(installation: String) -> Result<(), AppError> {
    let list = installations::list_probed().await;
    let item = list
        .iter()
        .find(|item| item.id == installation)
        .ok_or_else(|| AppError::new("blender.unknown_installation", "This Blender is not in the list."))?;
    manage::uninstall(item)
        .await
        .map_err(|error| blender_error("blender.uninstall_failed", "Could not remove the Locus add-on.", error))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadHostsResult {
    reloaded: Vec<u32>,
    restart_needed: Vec<u32>,
}

#[tauri::command]
pub async fn blender_reload_hosts() -> Result<ReloadHostsResult, AppError> {
    require_enabled()?;
    let (reloaded, restart_needed) = manage::reload_hosts()
        .await
        .map_err(|error| blender_error("blender.reload_failed", "Could not reload the Blender hosts.", error))?;
    Ok(ReloadHostsResult { reloaded, restart_needed })
}

// ── workspace status (popover) ─────────────────────────────────────────────

fn scope_for(
    registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    session_id: Option<&str>,
) -> Result<target::Scope, AppError> {
    let resolved = registry.resolve_workspace_ref(workspace_ref).map_err(|error| {
        AppError::new("workspace.resolve_failed", "The workspace could not be resolved.").detail(error.to_string())
    })?;
    let runtime = resolved.runtime();
    Ok(target::Scope::new(runtime.checkout_id().as_str(), runtime.root(), session_id))
}

/// Everything the status item and popover show. Cheap when Blender is idle:
/// no hub means no instances and nothing is started.
#[tauri::command]
pub async fn blender_workspace_status(
    workspace_ref: WorkspaceRef,
    session_id: Option<String>,
    detail: Option<bool>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Value, AppError> {
    if !crate::blender::blender_integration_enabled() {
        return Ok(json!({ "enabled": false, "running": false }));
    }
    let scope = scope_for(registry.inner(), &workspace_ref, session_id.as_deref())?;
    let Some(hub) = crate::blender::hub::current() else {
        return Ok(json!({ "enabled": true, "running": false }));
    };
    let instances = target::scoped_instances(&hub, &scope);
    let mut status = json!({
        "enabled": true,
        "running": true,
        "instances": instances,
        "target": target::session_target(&scope).map(|spec| spec.describe()),
        "background": hub.background.snapshots(),
        "limits": {
            "maxBackground": crate::blender::background::limits().max_instances,
            "idleTimeoutSecs": crate::blender::background::limits().idle_timeout.as_secs(),
        },
    });
    // Popover-only details (header scan, installation table) are computed
    // only while it is open.
    if detail.unwrap_or(false) {
        let version = tokio::task::spawn_blocking({
            let scope = scope.clone();
            move || target::workspace_version(&scope)
        })
        .await
        .map_err(|error| AppError::new("blender.status_failed", "Blender status failed.").detail(error.to_string()))?;
        status["workspaceVersion"] = serde_json::to_value(version).unwrap_or(Value::Null);
        status["withoutHost"] = serde_json::to_value(
            crate::blender::hub::unregistered_processes(&hub)
                .into_iter()
                .map(|process| json!({ "pid": process.pid, "path": process.image_path }))
                .collect::<Vec<_>>(),
        )
        .unwrap_or(Value::Null);
        status["addon"] = serde_json::to_value(manage::default_addon_status(&scope).await).unwrap_or(Value::Null);
    }
    Ok(status)
}

#[tauri::command]
pub async fn blender_set_session_target(
    workspace_ref: WorkspaceRef,
    session_id: String,
    target_spec: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Option<String>, AppError> {
    require_enabled()?;
    let scope = scope_for(registry.inner(), &workspace_ref, Some(&session_id))?;
    match target_spec.as_deref().map(str::trim).filter(|spec| !spec.is_empty()) {
        Some(spec) => {
            let spec = target::TargetSpec::parse(spec)
                .map_err(|error| blender_error("blender.invalid_target", "Invalid Blender target.", error))?;
            let description = spec.describe();
            target::set_session_target(&scope, spec)
                .map_err(|error| blender_error("blender.invalid_target", "Invalid Blender target.", error))?;
            Ok(Some(description))
        }
        None => {
            target::clear_session_target(&scope);
            Ok(None)
        }
    }
}

/// "Start Blender": binds (and starts) the workspace's Blender service, then
/// launches an interactive Blender with the host loaded.
#[tauri::command]
pub async fn blender_launch(
    workspace_ref: WorkspaceRef,
    session_id: Option<String>,
    file: Option<String>,
    installation: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::blender::hub::InstanceSnapshot, AppError> {
    require_enabled()?;
    let registry = registry.inner().clone();
    let resolved = registry.resolve_workspace_ref(&workspace_ref).map_err(|error| {
        AppError::new("workspace.resolve_failed", "The workspace could not be resolved.").detail(error.to_string())
    })?;
    let execution = registry
        .execution_context(resolved.runtime().checkout_id(), &[ServiceKind::Blender])
        .await
        .map_err(|error| blender_error("blender.service_failed", "Could not start the Blender service.", error))?;
    let _binding = execution
        .resolve_service(ServiceKind::Blender)
        .map_err(|error| blender_error("blender.service_failed", "Could not start the Blender service.", error.to_string()))?;
    let scope = target::Scope::new(execution.checkout_id.as_str(), execution.root(), session_id.as_deref());
    let hub = crate::blender::hub::current()
        .ok_or_else(|| AppError::new("blender.service_failed", "The Blender service is not running."))?;
    let file = file.map(|file| {
        let path = std::path::PathBuf::from(&file);
        if path.is_absolute() { path } else { scope.root.join(path) }
    });
    let instance = target::launch_interactive(&hub, &scope, file.as_deref(), installation.as_deref(), None, false)
        .await
        .map_err(|error| blender_error("blender.launch_failed", "Blender could not be started.", error))?;
    Ok(instance.snapshot())
}

#[tauri::command]
pub async fn blender_close_background(pid: u32) -> Result<(), AppError> {
    let hub = crate::blender::hub::current()
        .ok_or_else(|| AppError::new("blender.not_running", "The Blender service is not running."))?;
    if !hub.background.owns(pid) {
        return Err(AppError::new(
            "blender.not_background",
            "Only background instances that Locus launched can be closed.",
        ));
    }
    hub.background.close(&hub, pid).await;
    Ok(())
}

#[tauri::command]
pub async fn blender_set_workspace_version(
    workspace_ref: WorkspaceRef,
    version: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), AppError> {
    let scope = scope_for(registry.inner(), &workspace_ref, None)?;
    target::set_pinned_version(&scope.checkout_id, version.as_deref())
        .map_err(|error| blender_error("blender.version_failed", "Could not save the workspace Blender version.", error))
}

/// Settings > Blender > running background instances (every workspace).
#[tauri::command]
pub async fn blender_background_instances() -> Result<Vec<crate::blender::background::BackgroundSnapshot>, AppError> {
    Ok(crate::blender::hub::current()
        .map(|hub| hub.background.snapshots())
        .unwrap_or_default())
}
