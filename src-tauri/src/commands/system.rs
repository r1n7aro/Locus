use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use futures::future::join_all;
use tauri::State;
use tauri::{AppHandle, Manager};

use crate::error::AppError;
use crate::workspace_service::event::WorkspaceEventScope;
use crate::workspace_service::{
    ProjectRegistry, ResolvedWorkspaceScope, ServiceKind, ServiceStatus, WorkspaceRef,
};

#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
const WINDOWS_NOTIFICATION_DISPLAY_NAME: &str = "Locus";

/// Serializes process-level Unity settings whose durable config, per-checkout
/// markers, and in-process switches form one logical transaction.
pub(crate) fn unity_process_settings_mutation_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn resolve_workspace_scope(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<ResolvedWorkspaceScope, AppError> {
    super::session::resolve_workspace_scope(workspace_registry, workspace_ref, operation)
}

fn scope_root(scope: &ResolvedWorkspaceScope) -> String {
    scope.runtime().root().to_string_lossy().to_string()
}

fn live_unity_runtime_scopes(workspace_registry: &ProjectRegistry) -> Vec<(WorkspaceRef, String)> {
    let mut scopes = workspace_registry
        .runtimes()
        .into_iter()
        .filter_map(|runtime| {
            let root = runtime.root().to_string_lossy().to_string();
            crate::unity_bridge::is_unity_project(&root)
                .then(|| (WorkspaceRef::for_runtime(&runtime), root))
        })
        .collect::<Vec<_>>();
    scopes.sort_by(|left, right| left.0.checkout_id.cmp(&right.0.checkout_id));
    scopes
}

fn sync_unity_markers_transactionally<F>(
    roots: &[String],
    value: bool,
    previous: bool,
    sync: F,
) -> Result<(), String>
where
    F: Fn(&str, bool) -> Result<(), String>,
{
    let mut attempted = Vec::new();
    for root in roots {
        attempted.push(root.as_str());
        if let Err(error) = sync(root, value) {
            let rollback_errors = attempted
                .iter()
                .filter_map(|attempted_root| {
                    sync(attempted_root, previous)
                        .err()
                        .map(|rollback| format!("{attempted_root}: {rollback}"))
                })
                .collect::<Vec<_>>();
            let rollback_detail = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback failed: {}", rollback_errors.join("; "))
            };
            return Err(format!(
                "failed to update Unity marker for {root}: {error}{rollback_detail}"
            ));
        }
    }
    Ok(())
}

async fn unity_observable_event_scope(scope: &ResolvedWorkspaceScope) -> WorkspaceEventScope {
    let runtime = scope.runtime();
    let service_identity = runtime
        .services()
        .state_snapshot(ServiceKind::Unity)
        .await
        .and_then(|snapshot| {
            matches!(
                snapshot.status,
                ServiceStatus::Starting | ServiceStatus::Running
            )
            .then_some(snapshot.identity)
            .flatten()
        });
    let event_scope = WorkspaceEventScope {
        project_id: runtime.project_id().clone(),
        checkout_id: runtime.checkout_id().clone(),
        workspace_generation: runtime.generation(),
        service_instance_id: service_identity
            .as_ref()
            .map(|identity| identity.service_instance_id.clone()),
        service_generation: service_identity
            .as_ref()
            .map(|identity| identity.runtime_generation),
    };
    crate::unity_bridge::bind_workspace_observable_status(
        &runtime.root().to_string_lossy(),
        &event_scope,
    );
    event_scope
}

fn publish_background_hook_status(
    app: &AppHandle,
    workspace_registry: &ProjectRegistry,
    scope: &WorkspaceEventScope,
    status: crate::unity_bridge::UnityBackgroundHookStatus,
) -> crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityBackgroundHookStatus> {
    workspace_registry.event_router().publish_for_scope(
        app,
        scope,
        "unity-background-hook-status",
        status.clone(),
    );
    crate::unity_bridge::UnityWorkspaceStatus::from_scope(scope, status)
}

fn publish_state_probe_status(
    app: &AppHandle,
    workspace_registry: &ProjectRegistry,
    scope: &WorkspaceEventScope,
    status: crate::unity_bridge::UnityStateProbeStatus,
) -> crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityStateProbeStatus> {
    workspace_registry.event_router().publish_for_scope(
        app,
        scope,
        "unity-state-probe-status",
        status.clone(),
    );
    crate::unity_bridge::UnityWorkspaceStatus::from_scope(scope, status)
}

fn bind_integration_request_to_checkout(
    mut request: crate::cli_driver::UnityIntegrationTestRunRequest,
    checkout_root: String,
) -> crate::cli_driver::UnityIntegrationTestRunRequest {
    request.project_path = Some(checkout_root);
    request
}

#[tauri::command]
pub fn get_system_locale() -> Option<String> {
    sys_locale::get_locale()
}

#[tauri::command]
pub fn get_proxy_status() -> crate::network::ProxyStatus {
    crate::network::get_proxy_status()
}

#[tauri::command]
pub fn save_proxy_config(
    config: crate::network::ProxyConfig,
) -> Result<crate::network::ProxyStatus, crate::error::AppError> {
    crate::network::save_proxy_config(config).map_err(crate::error::AppError::from)
}

#[tauri::command]
pub async fn get_python_runtime_state(
    app_handle: AppHandle,
    refresh: Option<bool>,
    discover: Option<bool>,
) -> Result<crate::python_runtime::PythonRuntimeState, crate::error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::python_runtime::python_runtime_state_with_options(
            Some(&app_handle),
            refresh.unwrap_or(false),
            discover.unwrap_or(true),
        )
    })
    .await
    .map_err(|e| {
        crate::error::AppError::new(
            "python_runtime.join_failed",
            format!("Failed to load Python runtime state: {}", e),
        )
    })?
    .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub async fn save_python_runtime_selection(
    selected_id: String,
    app_handle: AppHandle,
) -> Result<crate::python_runtime::PythonRuntimeState, crate::error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::python_runtime::save_python_runtime_selection(&selected_id, Some(&app_handle))
    })
    .await
    .map_err(|e| {
        crate::error::AppError::new(
            "python_runtime.join_failed",
            format!("Failed to save Python runtime selection: {}", e),
        )
    })?
    .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn send_system_notification(
    app_handle: AppHandle,
    title: String,
    body: Option<String>,
) -> Result<(), String> {
    send_system_notification_impl(&app_handle, &title, body.as_deref())
}

#[tauri::command]
pub fn play_custom_notification_sound(path: String, volume: Option<f32>) -> Result<(), String> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("Audio file path is empty".into());
    }
    if !path.is_file() {
        return Err(format!("Audio file does not exist: {}", path.display()));
    }

    let file = std::fs::File::open(&path)
        .map_err(|error| format!("Failed to open audio file: {error}"))?;
    let reader = std::io::BufReader::new(file);
    let sink_handle = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|error| format!("Failed to open default audio output: {error}"))?;
    let player = rodio::play(sink_handle.mixer(), reader)
        .map_err(|error| format!("Failed to play audio file: {error}"))?;
    let volume = volume
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 2.0);
    player.set_volume(volume);

    std::thread::Builder::new()
        .name("locus-custom-notification-sound".into())
        .spawn(move || {
            player.sleep_until_end();
            drop(sink_handle);
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to start audio playback thread: {error}"))
}

#[tauri::command]
pub async fn request_app_exit(app_handle: AppHandle) {
    exit_app_inner(app_handle).await;
}

pub(crate) fn exit_app(app_handle: &AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        exit_app_inner(app_handle).await;
    });
}

static APP_EXIT_STARTED: AtomicBool = AtomicBool::new(false);

/// Window teardown during process exit must preserve the last durable
/// window/pane recovery projection. Interactive window closes still detach
/// their contexts before this flag is raised.
pub(crate) fn app_exit_started() -> bool {
    APP_EXIT_STARTED.load(Ordering::SeqCst)
}

async fn exit_app_inner(app_handle: AppHandle) {
    if APP_EXIT_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    if let Some(tasks) = app_handle.try_state::<crate::ActiveTasks>() {
        let tasks = tasks.lock().await;
        for task in tasks.values() {
            let _ = task.cancel_tx.send(true);
        }
    }
    if let Some(tasks) = app_handle.try_state::<Arc<crate::async_tasks::AsyncTaskManager>>() {
        tasks.cancel_all();
    }
    let terminated = crate::process_util::begin_managed_process_shutdown();
    if terminated > 0 {
        eprintln!(
            "[Locus] terminating {} managed process tree(s) before exit",
            terminated
        );
    }
    let drained =
        crate::process_util::wait_for_managed_processes(std::time::Duration::from_millis(1500))
            .await;
    if !drained {
        eprintln!("[Locus] managed process shutdown grace period elapsed; forcing remaining trees");
        crate::process_util::terminate_all_managed_processes();
    }
    if let Some(tasks) = app_handle.try_state::<crate::ActiveTasks>() {
        let mut tasks = tasks.lock().await;
        for (_, task) in tasks.drain() {
            task.join_handle.abort();
        }
    }

    if let Some(registry) = app_handle.try_state::<Arc<crate::workspace_service::ProjectRegistry>>()
    {
        registry.shutdown_all().await;
    }

    if let Err(error) = crate::unity_bridge::restore_background_hook_runtime() {
        eprintln!("[Locus] failed to restore Unity background hook before exit: {error}");
    }
    crate::csharp_lsp::kill_active_server_for_exit();
    crate::csharp_compile::kill_active_server_for_exit();
    crate::mcp::manager::kill_all_for_exit();
    crate::commands::destroy_unity_embed_control_window_on_main(&app_handle);
    app_handle.exit(0);
}

#[tauri::command]
pub async fn get_running_task_count(
    active_tasks: State<'_, crate::ActiveTasks>,
    async_tasks: State<'_, Arc<crate::async_tasks::AsyncTaskManager>>,
) -> Result<usize, crate::error::AppError> {
    Ok(active_tasks.lock().await.len() + async_tasks.active_count())
}

#[tauri::command]
pub fn get_close_behavior(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::config::AppCloseBehavior, crate::error::AppError> {
    Ok(config.close_behavior())
}

#[tauri::command]
pub fn set_close_behavior(
    value: crate::config::AppCloseBehavior,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_close_behavior(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_dynamic_tool_loading_mode(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::config::DynamicToolLoadingMode, crate::error::AppError> {
    Ok(config.dynamic_tool_loading_mode())
}

#[tauri::command]
pub fn set_dynamic_tool_loading_mode(
    value: crate::config::DynamicToolLoadingMode,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_dynamic_tool_loading_mode(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_anthropic_native_lazy_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.anthropic_native_lazy_enabled())
}

#[tauri::command]
pub fn set_anthropic_native_lazy_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_anthropic_native_lazy_enabled(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_async_tasks_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.async_tasks_enabled())
}

#[tauri::command]
pub fn set_async_tasks_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_async_tasks_enabled(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_unity_multi_agent_editor_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_multi_agent_editor_enabled())
}

#[tauri::command]
pub fn set_unity_multi_agent_editor_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_unity_multi_agent_editor_enabled(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_unity_background_hook_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_background_hook_enabled())
}

#[tauri::command]
pub async fn set_unity_background_hook_enabled(
    value: bool,
    app: AppHandle,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<
    crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityBackgroundHookStatus>,
    crate::error::AppError,
> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "set_unity_background_hook_enabled",
    )?;
    let _settings_mutation = unity_process_settings_mutation_gate().lock().await;
    let cwd = scope_root(&scope);
    let event_scope = unity_observable_event_scope(&scope).await;
    let runtime_scopes = live_unity_runtime_scopes(workspace_registry.inner());
    let cwd_is_unity = crate::unity_bridge::is_unity_project(&cwd);
    let roots = runtime_scopes
        .iter()
        .map(|(_, root)| root.clone())
        .collect::<Vec<_>>();
    let previous = config.unity_background_hook_enabled();

    if let Err(error) = config.set_unity_background_hook_enabled(value) {
        let _ = config.set_unity_background_hook_enabled(previous);
        return Err(crate::error::AppError::from(error));
    }

    if let Err(error) = sync_unity_markers_transactionally(
        &roots,
        value,
        previous,
        crate::unity_bridge::sync_background_hook_marker,
    ) {
        let _ = config.set_unity_background_hook_enabled(previous);
        return Err(
            crate::error::AppError::new("unity.background_hook.marker_failed", error)
                .operation("setUnityBackgroundHookEnabled"),
        );
    }

    let status = match crate::unity_bridge::set_background_hook_enabled(value) {
        Ok(status) => status,
        Err(error) => {
            let _ = config.set_unity_background_hook_enabled(previous);
            let _ = sync_unity_markers_transactionally(
                &roots,
                previous,
                value,
                crate::unity_bridge::sync_background_hook_marker,
            );
            let _ = crate::unity_bridge::set_background_hook_enabled(previous);
            return Err(
                crate::error::AppError::new("unity.background_hook.restore_failed", error)
                    .operation("setUnityBackgroundHookEnabled"),
            );
        }
    };

    if !value {
        return Ok(publish_background_hook_status(
            &app,
            workspace_registry.inner(),
            &event_scope,
            status,
        ));
    }

    for (_, root) in &runtime_scopes {
        if root == &cwd {
            continue;
        }
        match crate::unity_bridge::ensure_background_hook_for_project(root).await {
            Ok(other_status)
                if other_status.enabled
                    && other_status.state
                        == crate::unity_bridge::UnityBackgroundHookState::Failed =>
            {
                eprintln!(
                    "[Locus] warning: Unity background hook failed for checkout root '{}': {}",
                    root,
                    other_status.error.as_deref().unwrap_or("unknown failure")
                );
            }
            Ok(_) => {}
            Err(error) => {
                let process_info =
                    crate::unity_bridge::query_current_project_editor_process(root).await;
                if !matches!(
                    process_info.state,
                    crate::unity_bridge::UnityEditorProcessState::NotRunning
                ) {
                    eprintln!(
                        "[Locus] warning: failed to apply Unity background hook for checkout root '{}': {}",
                        root, error
                    );
                }
            }
        }
    }

    if !cwd_is_unity {
        return Ok(publish_background_hook_status(
            &app,
            workspace_registry.inner(),
            &event_scope,
            status,
        ));
    }

    match crate::unity_bridge::ensure_background_hook_for_project(&cwd).await {
        Ok(status) => {
            if status.enabled
                && status.state == crate::unity_bridge::UnityBackgroundHookState::Failed
            {
                let message = status
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unity background hook failed".to_string());
                return Err(
                    crate::error::AppError::new("unity.background_hook.failed", message)
                        .operation("setUnityBackgroundHookEnabled"),
                );
            }
            Ok(publish_background_hook_status(
                &app,
                workspace_registry.inner(),
                &event_scope,
                status,
            ))
        }
        Err(error) => {
            let process_info =
                crate::unity_bridge::query_current_project_editor_process(&cwd).await;
            if matches!(
                process_info.state,
                crate::unity_bridge::UnityEditorProcessState::NotRunning
            ) {
                return Ok(publish_background_hook_status(
                    &app,
                    workspace_registry.inner(),
                    &event_scope,
                    status,
                ));
            }
            Err(
                crate::error::AppError::new("unity.background_hook.failed", error)
                    .operation("setUnityBackgroundHookEnabled"),
            )
        }
    }
}

#[tauri::command]
pub async fn get_unity_background_hook_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<
    crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityBackgroundHookStatus>,
    crate::error::AppError,
> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_background_hook_status",
    )?;
    let event_scope = unity_observable_event_scope(&scope).await;
    Ok(crate::unity_bridge::background_hook_status_for_scope(
        &event_scope,
    ))
}

#[tauri::command]
pub fn get_unity_external_editor_default_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_external_editor_default_enabled())
}

#[tauri::command]
pub async fn set_unity_external_editor_default_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<bool, crate::error::AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "set_unity_external_editor_default_enabled",
    )?;
    let _settings_mutation = unity_process_settings_mutation_gate().lock().await;
    let request_checkout = scope.runtime().checkout_id().clone();
    let runtime_scopes = live_unity_runtime_scopes(workspace_registry.inner());
    let registry = workspace_registry.inner();
    let mut ready_roots = Vec::new();
    let connection_states = join_all(runtime_scopes.iter().map(|(runtime_ref, root)| async move {
        (
            runtime_ref.clone(),
            root.clone(),
            crate::unity_bridge::query_unity_status(root).await.0,
        )
    }))
    .await;
    let ready_states = join_all(
        connection_states
            .into_iter()
            .filter(|(_, _, connected)| *connected)
            .map(|(runtime_ref, _, _)| async move {
                let result = super::workspace::resolve_unity_ready_ipc_scope(
                    registry,
                    &runtime_ref,
                    "set_unity_external_editor_default_enabled",
                )
                .await;
                (runtime_ref, result)
            }),
    )
    .await;
    for (runtime_ref, result) in ready_states {
        match result {
            Ok(ready) => ready_roots.push(ready.root_text()),
            Err(error) if runtime_ref.checkout_id == request_checkout => return Err(error),
            Err(error) => eprintln!(
                "[Locus] warning: Unity external editor setting is waiting for checkout {} Ready: {}",
                runtime_ref.checkout_id, error
            ),
        }
    }

    config
        .set_unity_external_editor_default_enabled(value)
        .map_err(crate::error::AppError::from)?;
    crate::unity_bridge::set_external_editor_default(value);

    let apply_results = join_all(ready_roots.into_iter().map(|root| async move {
        let result = crate::unity_bridge::configure_locus_external_editor(&root, value).await;
        (root, result)
    }))
    .await;
    for (root, result) in apply_results {
        if let Err(error) = result {
            eprintln!(
                "[Locus] warning: failed to apply Unity external editor setting for checkout root '{}': {}",
                root, error
            );
        }
    }
    Ok(value)
}

#[tauri::command]
pub fn take_external_script_open_request(
    pending: State<'_, crate::unity_bridge::PendingExternalScriptOpenRequest>,
) -> Result<Option<crate::unity_bridge::ExternalScriptOpenRequest>, crate::error::AppError> {
    Ok(pending.take())
}

#[tauri::command]
pub fn get_unity_state_probe_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_state_probe_enabled())
}

#[tauri::command]
pub async fn set_unity_state_probe_enabled(
    value: bool,
    app: AppHandle,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<
    crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityStateProbeStatus>,
    crate::error::AppError,
> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "set_unity_state_probe_enabled",
    )?;
    let _settings_mutation = unity_process_settings_mutation_gate().lock().await;
    let event_scope = unity_observable_event_scope(&scope).await;
    config
        .set_unity_state_probe_enabled(value)
        .map_err(crate::error::AppError::from)?;
    let status = crate::unity_bridge::set_state_probe_enabled(value);
    Ok(publish_state_probe_status(
        &app,
        workspace_registry.inner(),
        &event_scope,
        status,
    ))
}

#[tauri::command]
pub async fn get_unity_state_probe_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<
    crate::unity_bridge::UnityWorkspaceStatus<crate::unity_bridge::UnityStateProbeStatus>,
    crate::error::AppError,
> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_state_probe_status",
    )?;
    let event_scope = unity_observable_event_scope(&scope).await;
    Ok(crate::unity_bridge::state_probe_status_for_scope(
        &event_scope,
    ))
}

#[tauri::command]
pub fn get_unity_native_bridge_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_native_bridge_enabled())
}

/// Toggle the native broker transport. Persists the config flag, flips the
/// in-process transport switch, and writes/removes the per-project marker the
/// Unity plugin checks before loading the native DLL. The change takes full
/// effect after the editor's next domain reload (when the plugin re-reads the
/// marker); disabling it leaves the native-only Unity command transport
/// unavailable until the marker is restored.
#[tauri::command]
pub async fn set_unity_native_bridge_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<bool, crate::error::AppError> {
    let _scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "set_unity_native_bridge_enabled",
    )?;
    let _settings_mutation = unity_process_settings_mutation_gate().lock().await;
    let roots = live_unity_runtime_scopes(workspace_registry.inner())
        .into_iter()
        .map(|(_, root)| root)
        .collect::<Vec<_>>();
    let previous = config.unity_native_bridge_enabled();

    if let Err(error) = config.set_unity_native_bridge_enabled(value) {
        let _ = config.set_unity_native_bridge_enabled(previous);
        return Err(crate::error::AppError::from(error));
    }

    if let Err(error) = sync_unity_markers_transactionally(
        &roots,
        value,
        previous,
        crate::unity_bridge::sync_native_bridge_marker,
    ) {
        let _ = config.set_unity_native_bridge_enabled(previous);
        return Err(
            crate::error::AppError::new("unity.native_bridge.marker_failed", error)
                .operation("setUnityNativeBridgeEnabled"),
        );
    }
    crate::unity_bridge::set_native_bridge_enabled(value);
    Ok(value)
}

/// Best-effort native broker status for the current workspace. `None` when the
/// bridge is disabled or no live broker is serving this project.
#[tauri::command]
pub async fn get_unity_native_broker_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Option<crate::unity_bridge::NativeBrokerStatus>, crate::error::AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_native_broker_status",
    )?;
    let cwd = scope_root(&scope);
    if cwd.trim().is_empty() || !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(None);
    }
    Ok(crate::unity_bridge::query_native_broker_status(&cwd).await)
}

/// Fused semantic editor state (pipe + process + native signals) for the
/// current workspace. Returns `unknown` when no workspace is selected.
#[tauri::command]
pub async fn get_unity_semantic_state(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::unity_bridge::SemanticState, crate::error::AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_semantic_state",
    )?;
    let cwd = scope_root(&scope);
    if cwd.trim().is_empty() || !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(crate::unity_bridge::unity_semantic_state("").await);
    }
    Ok(crate::unity_bridge::unity_semantic_state(&cwd).await)
}

#[tauri::command]
pub async fn unity_state_probe_selftest_run(
    app: AppHandle,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), crate::error::AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_state_probe_selftest_run",
    )
    .await?;
    let event_scope = ready.checkout_event_scope();
    crate::unity_bridge::run_state_probe_selftest_scoped(app, ready.root_text(), event_scope)
        .await
        .map_err(|error| {
            crate::error::AppError::new("unity.state_probe.selftest_failed", error)
                .operation("unityStateProbeSelftestRun")
        })
}

#[tauri::command]
pub async fn unity_native_bridge_selftest_run(
    app: AppHandle,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), crate::error::AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_native_bridge_selftest_run",
    )
    .await?;
    let event_scope = ready.checkout_event_scope();
    crate::unity_bridge::run_native_bridge_selftest_scoped(app, ready.root_text(), event_scope)
        .await
        .map_err(|error| {
            crate::error::AppError::new("unity.native_bridge.selftest_failed", error)
                .operation("unityNativeBridgeSelftestRun")
        })
}

#[tauri::command]
pub async fn unity_integration_test_run(
    app: AppHandle,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    request: crate::cli_driver::UnityIntegrationTestRunRequest,
) -> Result<crate::cli_driver::UnityIntegrationTestRunStarted, crate::error::AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_integration_test_run",
    )
    .await?;
    let cwd = ready.root_text();
    let request = bind_integration_request_to_checkout(request, cwd.clone());
    crate::cli_driver::spawn_ui(app, request).map_err(|error| {
        crate::error::AppError::new("unity.integration_test.start_failed", error)
            .operation("unityIntegrationTestRun")
    })
}

#[tauri::command]
pub fn unity_integration_test_cancel() -> Result<(), crate::error::AppError> {
    crate::cli_driver::cancel_ui();
    Ok(())
}

#[cfg(windows)]
pub(crate) fn ensure_windows_notification_identity(app_handle: &AppHandle) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let app_id = app_handle.config().identifier.as_str();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(format!(r"SOFTWARE\Classes\AppUserModelId\{app_id}"))
        .map_err(|error| format!("Failed to create notification AppUserModelId key: {error}"))?;

    key.set_value("DisplayName", &WINDOWS_NOTIFICATION_DISPLAY_NAME)
        .map_err(|error| format!("Failed to write notification display name: {error}"))?;

    if let Ok(exe_path) = std::env::current_exe() {
        let icon_uri = exe_path.display().to_string();
        let _ = key.set_value("IconUri", &icon_uri);
        let _ = key.set_value("IconBackgroundColor", &"0");
    }

    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn ensure_windows_notification_identity(_app_handle: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn send_system_notification_impl(
    app_handle: &AppHandle,
    title: &str,
    body: Option<&str>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    use tauri_winrt_notification::Toast;

    ensure_windows_notification_identity(app_handle)?;

    let app_id = app_handle.config().identifier.as_str();
    let (line1, line2) = split_notification_body(body);

    let mut toast = Toast::new(app_id).title(title);
    if !line1.is_empty() {
        toast = toast.text1(&line1);
    }
    if !line2.is_empty() {
        toast = toast.text2(&line2);
    }

    match toast.show() {
        Ok(()) => Ok(()),
        Err(error) => {
            let mut fallback = app_handle.notification().builder().title(title);
            if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
                fallback = fallback.body(body);
            }
            fallback
                .show()
                .map_err(|fallback_error| {
                    format!(
                        "Failed to send Windows notification ({error}); fallback notification also failed: {fallback_error}"
                    )
                })
        }
    }
}

#[cfg(not(windows))]
fn send_system_notification_impl(
    app_handle: &AppHandle,
    title: &str,
    body: Option<&str>,
) -> Result<(), String> {
    let mut notification = app_handle.notification().builder().title(title);
    if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
        notification = notification.body(body);
    }
    notification
        .show()
        .map_err(|error| format!("Failed to send notification: {error}"))
}

fn split_notification_body(body: Option<&str>) -> (String, String) {
    let normalized = body
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let first = lines.next().unwrap_or_default().to_string();
    let second = lines.collect::<Vec<_>>().join(" ");
    (first, second)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{
        bind_integration_request_to_checkout, split_notification_body,
        sync_unity_markers_transactionally,
    };

    #[test]
    fn split_notification_body_keeps_primary_and_secondary_lines() {
        assert_eq!(
            split_notification_body(Some("Session A\r\nCompleted response")),
            ("Session A".to_string(), "Completed response".to_string())
        );
        assert_eq!(
            split_notification_body(Some("Only one line")),
            ("Only one line".to_string(), String::new())
        );
        assert_eq!(
            split_notification_body(Some("First\n\nSecond\nThird")),
            ("First".to_string(), "Second Third".to_string())
        );
    }

    #[test]
    fn integration_test_request_is_bound_to_resolved_checkout() {
        let request = crate::cli_driver::UnityIntegrationTestRunRequest {
            project_path: Some("C:/stale-workspace".to_string()),
            workspace_paths: Vec::new(),
            suites: vec!["connect".to_string()],
            open_unity: None,
            install_plugin: None,
            force_edit_mode: None,
            type_index_sample_mode: None,
            yaml_parity_sample_count: None,
            yaml_parity_seed: None,
            connect_timeout_ms: None,
            suite_timeout_ms: None,
            poll_ms: None,
            no_progress_timeout_ms: None,
        };

        let bound =
            bind_integration_request_to_checkout(request, "C:/projects/checkout-b".to_string());

        assert_eq!(
            bound.project_path.as_deref(),
            Some("C:/projects/checkout-b")
        );
    }

    #[test]
    fn process_level_marker_fanout_rolls_back_every_attempted_checkout() {
        let calls = Mutex::new(Vec::<(String, bool)>::new());
        let roots = vec!["checkout-a".to_string(), "checkout-b".to_string()];
        let result = sync_unity_markers_transactionally(&roots, true, false, |root, value| {
            calls
                .lock()
                .expect("marker call log")
                .push((root.to_string(), value));
            if root == "checkout-b" && value {
                Err("simulated write failure".to_string())
            } else {
                Ok(())
            }
        });

        assert!(result.is_err());
        assert_eq!(
            calls.into_inner().expect("marker call log"),
            vec![
                ("checkout-a".to_string(), true),
                ("checkout-b".to_string(), true),
                ("checkout-a".to_string(), false),
                ("checkout-b".to_string(), false),
            ]
        );
    }

    #[tokio::test]
    async fn process_level_unity_setting_mutations_are_serialized() {
        let first = super::unity_process_settings_mutation_gate().lock().await;
        let (entered_tx, mut entered_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let _second = super::unity_process_settings_mutation_gate().lock().await;
            let _ = entered_tx.send(());
        });
        tokio::task::yield_now().await;
        assert!(matches!(
            entered_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut entered_rx)
            .await
            .expect("second mutation entered after first committed")
            .expect("second mutation signal");
        waiter.await.expect("setting mutation waiter");
    }
}
