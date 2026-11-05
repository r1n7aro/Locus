use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::config::{WorkspaceServiceResourceLimits, WorkspaceServiceResourceLimitsUpdateError};
use crate::error::AppError;
use crate::resource_policy::{ResourcePolicySnapshot, ResourcePolicyStore};
use crate::session::store::SessionStore;
use crate::workspace_service::service::ServiceKind;
use crate::workspace_service::{
    CheckoutId, ProjectRegistry, WindowContextError, WindowContextRegistry,
    WindowIntentEpochSnapshot, WindowPaneWorkspaceContext, WorkspaceRef, WorkspaceResolveError,
    WorkspaceRuntime,
};

const WINDOW_CONTEXT_RECOVERY_FILE: &str = "window_workspace_contexts.json";

#[derive(Default)]
pub struct WindowContextPersistence {
    pub(crate) mutation: std::sync::Mutex<()>,
}

pub(crate) fn load_window_context_recovery(
    data_dir: &std::path::Path,
) -> Vec<WindowPaneWorkspaceContext> {
    let path = data_dir.join(WINDOW_CONTEXT_RECOVERY_FILE);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(crate) struct RestoredWindowContexts {
    pub main_runtime: Option<Arc<WorkspaceRuntime>>,
    pub restored_panes: usize,
    pub warnings: Vec<String>,
}

/// Rebuild every durable window/pane projection. Restored panes initially hold
/// BackgroundOpen leases; main/main is upgraded to VisiblePane after the full
/// hierarchy exists. An explicit startup root replaces only main/main.
pub(crate) fn restore_persisted_window_contexts(
    recovered: Vec<WindowPaneWorkspaceContext>,
    requested_main_root: Option<&str>,
    legacy_active_session_candidates: &[String],
    registry: &ProjectRegistry,
    contexts: &WindowContextRegistry,
    store: &SessionStore,
) -> Result<RestoredWindowContexts, String> {
    let mut restored_panes = 0;
    let mut warnings = Vec::new();

    for mut persisted in recovered {
        let checkout_id = persisted.focused_checkout_id.clone();
        let checkout = match store.get_workspace_checkout(checkout_id.as_str()) {
            Ok(Some(checkout)) => checkout,
            Ok(None) => {
                warnings.push(format!(
                    "checkout {checkout_id} has no durable checkout record"
                ));
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "checkout {checkout_id} could not be loaded: {error}"
                ));
                continue;
            }
        };
        if !checkout_root_available(&checkout) {
            warnings.push(format!(
                "checkout {checkout_id} root is missing or its project identity changed: {}",
                checkout.root_path
            ));
            continue;
        }
        let runtime = match registry.register(&checkout.root_path) {
            Ok(runtime)
                if runtime.checkout_id() == &checkout_id
                    && runtime.project_id().as_str() == checkout.project_id =>
            {
                runtime
            }
            Ok(runtime) => {
                warnings.push(format!(
                    "checkout recovery identity changed: expected {checkout_id}, resolved {}",
                    runtime.checkout_id()
                ));
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "checkout {checkout_id} could not be registered: {error}"
                ));
                continue;
            }
        };
        if let Some(session_id) = persisted.active_session_id.clone() {
            let valid = registry
                .project(runtime.project_id())
                .is_some_and(|project| {
                    project
                        .sessions()
                        .resolve_for_checkout(runtime.checkout_id(), &session_id)
                        .is_ok()
                });
            if !valid {
                persisted.active_session_id = None;
                warnings.push(format!(
                    "active session {session_id} is outside recovered checkout {checkout_id}"
                ));
            }
        }
        match contexts.restore_background(persisted, runtime) {
            Ok(_) => restored_panes += 1,
            Err(error) => warnings.push(format!(
                "checkout {checkout_id} pane restore failed: {error}"
            )),
        }
    }

    if let Some(root) = requested_main_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        let runtime = registry.register(root)?;
        let intent_epoch = contexts
            .next_pane_intent_epoch("main", "main")
            .map_err(|error| error.to_string())?;
        contexts
            .focus("main", "main", runtime, intent_epoch)
            .map_err(|error| error.to_string())?;
    } else if let Some(main) = contexts
        .pane("main", "main")
        .map_err(|error| error.to_string())?
    {
        if let Some(runtime) = registry.runtime(&main.focused_checkout_id) {
            let intent_epoch = contexts
                .next_pane_intent_epoch("main", "main")
                .map_err(|error| error.to_string())?;
            contexts
                .focus("main", "main", runtime, intent_epoch)
                .map_err(|error| error.to_string())?;
        }
    }

    let mut main_runtime = None;
    if let Some(main) = contexts
        .pane("main", "main")
        .map_err(|error| error.to_string())?
    {
        main_runtime = registry.runtime(&main.focused_checkout_id);
        if main.active_session_id.is_none() {
            if let Some(runtime) = main_runtime.as_ref() {
                let candidate = legacy_active_session_candidates.iter().find(|session_id| {
                    registry
                        .project(runtime.project_id())
                        .is_some_and(|project| {
                            project
                                .sessions()
                                .resolve_for_checkout(runtime.checkout_id(), session_id)
                                .is_ok()
                        })
                });
                if let Some(session_id) = candidate {
                    let intent_epoch = contexts
                        .next_pane_intent_epoch("main", "main")
                        .map_err(|error| error.to_string())?;
                    contexts
                        .set_active_session("main", "main", Some(session_id.clone()), intent_epoch)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
    }

    Ok(RestoredWindowContexts {
        main_runtime,
        restored_panes,
        warnings,
    })
}

pub(crate) fn persist_window_context_recovery(
    app_handle: &tauri::AppHandle,
    contexts: &WindowContextRegistry,
) -> Result<(), String> {
    let data_dir = crate::commands::resolve_runtime_storage_dir(app_handle)?;
    let snapshots = contexts.snapshots().map_err(|error| error.to_string())?;
    let json = serde_json::to_vec_pretty(&snapshots)
        .map_err(|error| format!("failed to serialize window workspace contexts: {error}"))?;
    crate::config::atomic_write_config(&data_dir.join(WINDOW_CONTEXT_RECOVERY_FILE), &json)
}

fn persistence_lock_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "workspace.focus_persistence_unavailable",
        "The workspace focus recovery state is unavailable.",
    )
    .detail(error.to_string())
}

fn persist_contexts_best_effort(app_handle: &tauri::AppHandle, contexts: &WindowContextRegistry) {
    if let Err(error) = persist_window_context_recovery(app_handle, contexts) {
        tracing::warn!(
            log_module = "WindowContextRegistry",
            "failed to persist workspace focus recovery: {}",
            error
        );
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRuntimeDescriptor {
    pub project_id: String,
    pub checkout_id: String,
    pub root: String,
    pub workspace_generation: u64,
    pub materialization_epoch: u64,
    pub lease_count: usize,
    /// Services this workspace uses (mode auto or lazy).
    pub detected_services: Vec<String>,
    /// `auto`: started on focus and bound when runs start. `lazy`: dormant
    /// until a tool or SDK call needs it.
    pub service_modes:
        std::collections::BTreeMap<String, crate::workspace_service::EffectiveServiceMode>,
    pub knowledge_enabled: bool,
    /// Locus-managed worktree of the project's source checkout.
    pub managed_worktree: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckoutDescriptor {
    pub checkout_id: String,
    pub project_id: String,
    pub root: String,
    pub normalized_root: String,
    pub last_opened_at: i64,
    pub available: bool,
    pub knowledge_enabled: bool,
    /// Locus-managed worktree of the project's source checkout.
    pub managed_worktree: bool,
    pub runtime: Option<WorkspaceRuntimeDescriptor>,
}

// Inspect before registering: registration opens checkout-owned data and runs
// persistence hooks, which must never run against a replacement project.
fn resolve_checkout_identity(
    checkout: &crate::session::models::WorkspaceCheckoutRecord,
) -> Option<crate::workspace_service::identity::ResolvedWorkspaceIdentity> {
    crate::workspace_service::identity::ProjectIdResolver::resolve(&checkout.root_path)
        .ok()
        .filter(|identity| {
            identity.checkout_id.as_str() == checkout.checkout_id
                && identity.project_id.as_str() == checkout.project_id
        })
}

fn checkout_root_available(checkout: &crate::session::models::WorkspaceCheckoutRecord) -> bool {
    resolve_checkout_identity(checkout).is_some()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContextDescriptor {
    pub project_id: String,
    pub detected_services: Vec<String>,
    pub checkouts: Vec<WorkspaceCheckoutDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServiceResourceMetrics {
    pub workspaces: crate::workspace_service::runtime::WorkspaceRegistryMetrics,
    pub lsp: crate::csharp_lsp::LspProcessPoolMetrics,
    pub compile: crate::csharp_compile::scheduler::CompileSchedulerMetrics,
}

fn policy_update_error(error: WorkspaceServiceResourceLimitsUpdateError) -> AppError {
    match error {
        WorkspaceServiceResourceLimitsUpdateError::Validation { fields } => AppError::new(
            "workspace.resource_policy_invalid",
            "Workspace resource policy validation failed.",
        )
        .detail(
            serde_json::to_string(&fields)
                .unwrap_or_else(|serialization_error| serialization_error.to_string()),
        ),
        WorkspaceServiceResourceLimitsUpdateError::Persistence { message } => AppError::new(
            "workspace.resource_policy_persist_failed",
            "Failed to save the workspace resource policy.",
        )
        .detail(message),
    }
}

pub(super) fn workspace_resolve_error(error: WorkspaceResolveError) -> AppError {
    match error {
        error @ WorkspaceResolveError::StaleMaterialization { .. } => AppError::new(
            "workspace.materialization_stale",
            "The checkout assignment changed. Reopen the checkout before continuing.",
        )
        .detail(error.to_string()),
        WorkspaceResolveError::RegistryUnavailable { detail } => AppError::new(
            "workspace.registry_unavailable",
            "The workspace registry is unavailable.",
        )
        .detail(detail),
        WorkspaceResolveError::CheckoutUnavailable { checkout_id } => AppError::new(
            "workspace.checkout_unavailable",
            "The requested checkout is unavailable.",
        )
        .detail(checkout_id.to_string()),
        WorkspaceResolveError::StaleGeneration {
            checkout_id,
            expected_generation,
            actual_generation,
        } => AppError::new(
            "workspace.generation_stale",
            "The workspace runtime changed before the request was handled.",
        )
        .detail(format!(
            "checkout={checkout_id}, expected={expected_generation}, actual={actual_generation}"
        )),
    }
}

fn window_context_error(error: WindowContextError) -> AppError {
    let code = match &error {
        WindowContextError::StaleMaterialization { .. } => "workspace.materialization_stale",
        WindowContextError::EmptyWindowId | WindowContextError::EmptyPaneId => {
            "workspace.focus_context_invalid"
        }
        WindowContextError::InvalidIntentEpoch { .. } => "workspace.intent_epoch_invalid",
        WindowContextError::StaleIntent { .. } => "workspace.intent_stale",
        WindowContextError::PaneUnavailable { .. } => "workspace.pane_context_unavailable",
        WindowContextError::RevisionExhausted { .. } => "workspace.focus_revision_exhausted",
        WindowContextError::LockPoisoned(_) => "workspace.focus_context_unavailable",
    };
    AppError::new(code, "Failed to update the workspace focus context.").detail(error.to_string())
}

fn runtime_descriptor(runtime: &WorkspaceRuntime) -> WorkspaceRuntimeDescriptor {
    // A service the workspace turned off is invisible to the frontend: no
    // status item, plugin notice or start on focus.
    let mut service_modes = std::collections::BTreeMap::new();
    for kind in runtime.services().detected_kinds() {
        let mode = runtime.services().effective_mode(kind);
        if mode != crate::workspace_service::EffectiveServiceMode::Off {
            service_modes.insert(kind.as_str().to_string(), mode);
        }
    }
    let detected_services = service_modes.keys().cloned().collect::<Vec<_>>();
    WorkspaceRuntimeDescriptor {
        service_modes,
        project_id: runtime.project_id().to_string(),
        checkout_id: runtime.checkout_id().to_string(),
        root: runtime.root().display().to_string(),
        workspace_generation: runtime.generation(),
        materialization_epoch: runtime.materialization_epoch(),
        lease_count: runtime.lease_count(),
        detected_services,
        knowledge_enabled: crate::workspace::knowledge_enabled(&runtime.root().to_string_lossy()),
        managed_worktree: runtime.is_managed_worktree(),
    }
}

fn project_detected_services(checkouts: &[WorkspaceCheckoutDescriptor]) -> Vec<String> {
    let mut detected_services = std::collections::BTreeSet::new();
    for checkout in checkouts {
        if let Some(runtime) = &checkout.runtime {
            detected_services.extend(runtime.detected_services.iter().cloned());
        }
        if crate::unity_bridge::unity_integration_enabled(&checkout.root) {
            detected_services.insert(ServiceKind::Unity.as_str().to_string());
        }
    }
    detected_services.into_iter().collect()
}

#[tauri::command]
pub fn get_workspace_service_resource_limits(
    resource_policy: State<'_, Arc<ResourcePolicyStore>>,
) -> ResourcePolicySnapshot {
    resource_policy.snapshot()
}

#[tauri::command]
pub async fn set_workspace_service_resource_limits(
    limits: WorkspaceServiceResourceLimits,
    resource_policy: State<'_, Arc<ResourcePolicyStore>>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ResourcePolicySnapshot, AppError> {
    let resource_policy = resource_policy.inner().clone();
    let registry = registry.inner().clone();
    let snapshot = resource_policy
        .update(limits)
        .map_err(policy_update_error)?;
    registry.converge_resource_policy().await;
    Ok(snapshot)
}

#[tauri::command]
pub async fn get_workspace_service_resource_metrics(
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<WorkspaceServiceResourceMetrics, AppError> {
    let registry = registry.inner().clone();
    let (workspaces, lsp) = tokio::join!(registry.metrics(), crate::csharp_lsp::pool_metrics());
    Ok(WorkspaceServiceResourceMetrics {
        workspaces,
        lsp,
        compile: crate::csharp_compile::scheduler::metrics(),
    })
}

#[tauri::command]
pub(crate) async fn get_unity_editor_resources() -> Result<Vec<crate::unity_bridge::managed_editor::EditorResource>, AppError> {
    crate::unity_bridge::managed_editor::resources().await.map_err(AppError::from)
}

#[tauri::command]
pub fn list_workspace_runtimes(
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Vec<WorkspaceRuntimeDescriptor> {
    let mut descriptors = registry
        .runtimes()
        .into_iter()
        .map(|runtime| runtime_descriptor(&runtime))
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.checkout_id.cmp(&right.checkout_id));
    descriptors
}

/// Read the host lifecycle and command-readiness projection for every service
/// detected in one checkout. The scope lease prevents runtime replacement
/// while the async snapshots are collected.
#[tauri::command]
pub async fn get_workspace_service_states(
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<crate::workspace_service::WorkspaceServiceStateSnapshot>, AppError> {
    let scope = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(workspace_resolve_error)?;
    Ok(scope.runtime().services().state_snapshots().await)
}

#[tauri::command]
pub fn list_project_contexts(
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<ProjectContextDescriptor>, AppError> {
    let persisted = store.list_visible_workspace_checkouts()?;
    let mut projects =
        std::collections::BTreeMap::<String, Vec<WorkspaceCheckoutDescriptor>>::new();
    for checkout in persisted {
        let identity = resolve_checkout_identity(&checkout);
        let available = identity.is_some();
        let managed_worktree = identity.is_some_and(|identity| identity.managed_worktree);
        let runtime = CheckoutId::new(checkout.checkout_id.clone())
            .ok()
            .filter(|_| available)
            .and_then(|checkout_id| registry.runtime(&checkout_id))
            .map(|runtime| runtime_descriptor(&runtime));
        projects
            .entry(checkout.project_id.clone())
            .or_default()
            .push(WorkspaceCheckoutDescriptor {
                knowledge_enabled: crate::workspace::knowledge_enabled(&checkout.root_path),
                checkout_id: checkout.checkout_id,
                project_id: checkout.project_id.clone(),
                root: checkout.root_path,
                normalized_root: checkout.normalized_root,
                last_opened_at: checkout.last_opened_at,
                available,
                managed_worktree,
                runtime,
            });
    }
    let mut descriptors = projects
        .into_iter()
        .map(|(project_id, mut checkouts)| {
            checkouts.sort_by(|left, right| {
                right
                    .last_opened_at
                    .cmp(&left.last_opened_at)
                    .then_with(|| left.checkout_id.cmp(&right.checkout_id))
            });
            let detected_services = project_detected_services(&checkouts);
            ProjectContextDescriptor {
                project_id,
                detected_services,
                checkouts,
            }
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| {
        let left_opened = left
            .checkouts
            .first()
            .map(|checkout| checkout.last_opened_at)
            .unwrap_or_default();
        let right_opened = right
            .checkouts
            .first()
            .map(|checkout| checkout.last_opened_at)
            .unwrap_or_default();
        right_opened
            .cmp(&left_opened)
            .then_with(|| left.project_id.cmp(&right.project_id))
    });
    Ok(descriptors)
}

#[tauri::command]
pub fn inspect_workspace_setup(
    path: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<crate::workspace::WorkspaceSetup, AppError> {
    let mut setup = crate::workspace::inspect_workspace_setup(&path)?;
    overlay_persisted_service_modes(&path, store.inner().as_ref(), &mut setup);
    Ok(setup)
}

/// Detection supplies the initial mode only; a saved choice wins.
fn overlay_persisted_service_modes(
    path: &str,
    store: &SessionStore,
    setup: &mut crate::workspace::WorkspaceSetup,
) {
    let Ok(identity) = crate::workspace_service::identity::ProjectIdResolver::resolve(path) else {
        return;
    };
    let Ok(records) = store.list_workspace_services(identity.checkout_id.as_str()) else {
        return;
    };
    for service in &mut setup.services {
        if let Some(record) = records
            .iter()
            .find(|record| record.service_kind == service.service)
        {
            service.mode = crate::workspace::WorkspaceServiceMode::from_record(
                record.enabled,
                &record.activation_policy,
            );
        }
    }
}

/// Persist the dialog's service choices for `identity`'s checkout and apply
/// them to a live runtime. Runs before registration for new workspaces, so
/// the registration restore finds these records instead of detecting again.
async fn persist_workspace_service_modes(
    identity: &crate::workspace_service::identity::ResolvedWorkspaceIdentity,
    services: &[crate::workspace::WorkspaceServiceSetup],
    store: &SessionStore,
    registry: &Arc<ProjectRegistry>,
) -> Result<bool, AppError> {
    let offered = crate::workspace::detected_service_setup(&identity.root.to_string_lossy());
    let mut changed = false;
    for choice in services {
        let Some(kind) = ServiceKind::parse(&choice.service) else {
            continue;
        };
        // Only services the dialog could offer are writable from here.
        if !offered.iter().any(|offer| offer.service == choice.service) {
            continue;
        }
        let policy = choice.mode.activation_policy();
        let existing = store
            .list_workspace_services(identity.checkout_id.as_str())
            .map_err(AppError::from)?
            .into_iter()
            .find(|record| record.service_kind == kind.as_str());
        if existing.as_ref().is_some_and(|record| {
            crate::workspace::WorkspaceServiceMode::from_record(
                record.enabled,
                &record.activation_policy,
            ) == choice.mode
        }) {
            continue;
        }
        store
            .upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
                checkout_id: identity.checkout_id.to_string(),
                project_id: identity.project_id.to_string(),
                root_path: identity.root.display().to_string(),
                normalized_root: identity.normalized_root.to_string(),
                last_opened_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            })
            .map_err(AppError::from)?;
        store
            .upsert_workspace_service(&crate::session::models::WorkspaceServiceRecord {
                checkout_id: identity.checkout_id.to_string(),
                service_kind: kind.as_str().to_string(),
                service_instance_id: crate::workspace_service::ServiceInstanceId::for_service(
                    &identity.checkout_id,
                    kind.as_str(),
                )
                .to_string(),
                enabled: choice.mode != crate::workspace::WorkspaceServiceMode::Off,
                activation_policy: policy.as_str().to_string(),
                local_config: existing
                    .map(|record| record.local_config)
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .map_err(AppError::from)?;
        if let Some(runtime) = registry.runtime(&identity.checkout_id) {
            registry
                .apply_service_policy(&runtime, kind, policy)
                .await
                .map_err(AppError::from)?;
        }
        changed = true;
    }
    Ok(changed)
}

#[tauri::command]
pub async fn save_workspace_setup(
    path: String,
    setup: crate::workspace::WorkspaceSetup,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    app_handle: tauri::AppHandle,
) -> Result<crate::workspace::WorkspaceSetup, AppError> {
    use tauri::{Emitter, Manager};
    let identity = crate::workspace_service::identity::ProjectIdResolver::resolve(&path)
        .map_err(|error| AppError::from(error.to_string()))?;
    crate::workspace::save_workspace_setup(&path, &setup)?;
    if persist_workspace_service_modes(
        &identity,
        &setup.services,
        store.inner().as_ref(),
        registry.inner(),
    )
    .await?
    {
        let switches = ServiceSwitches::from_config(
            app_handle.state::<Arc<crate::config::AppConfig>>().inner(),
        );
        if let Err(error) = app_handle.emit("workspace-services-changed", switches) {
            eprintln!("[Locus] failed to emit workspace-services-changed: {error}");
        }
    }
    if !setup.knowledge_enabled {
        if let Some(runtime) = registry.runtime(&identity.checkout_id) {
            runtime.core().stop_knowledge_watchers();
        }
    }
    if let Some(runtime) = registry.runtime(&identity.checkout_id) {
        crate::workspace_service::event::emit_for_workspace_scope(
            &app_handle,
            &crate::workspace_service::event::WorkspaceEventScope::for_runtime(&runtime),
            "workspace-setup-changed",
            serde_json::json!({ "knowledgeEnabled": setup.knowledge_enabled }),
        );
    }
    let mut saved = crate::workspace::inspect_workspace_setup(&path)?;
    overlay_persisted_service_modes(&path, store.inner().as_ref(), &mut saved);
    Ok(saved)
}

#[tauri::command]
pub async fn open_workspace(
    path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    app_handle: tauri::AppHandle,
) -> Result<WorkspaceRuntimeDescriptor, AppError> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err(AppError::new(
            "workspace.path_empty",
            "A workspace directory is required.",
        ));
    }
    let registry = registry.inner().clone();
    let runtime = tauri::async_runtime::spawn_blocking(move || registry.open_workspace(path))
        .await
        .map_err(|error| {
            AppError::new(
                "workspace.open_task_failed",
                "Failed to complete workspace registration.",
            )
            .detail(error.to_string())
        })?
        .map_err(|error| {
            AppError::new("workspace.open_failed", "Failed to open the workspace.").detail(error)
        })?;
    store.set_workspace_project_visible(runtime.project_id().as_str(), true)?;
    if let Ok(data_dir) = crate::commands::resolve_runtime_storage_dir(&app_handle) {
        crate::commands::save_recent_dir_pub(&data_dir, &runtime.root().to_string_lossy());
    }
    Ok(runtime_descriptor(&runtime))
}

#[tauri::command]
pub fn remove_workspace(
    project_id: String,
    store: State<'_, Arc<SessionStore>>,
    app_handle: tauri::AppHandle,
) -> Result<bool, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::new(
            "workspace.project_id_empty",
            "A workspace project id is required.",
        ));
    }
    let roots = store
        .list_workspace_checkouts(Some(project_id))?
        .into_iter()
        .map(|checkout| checkout.root_path)
        .collect::<Vec<_>>();
    let removed = store.set_workspace_project_visible(project_id, false)?;
    if removed {
        if let Ok(data_dir) = crate::commands::resolve_runtime_storage_dir(&app_handle) {
            let _ = crate::commands::remove_recent_dirs_pub(&data_dir, &roots);
        }
    }
    Ok(removed)
}

#[tauri::command]
pub fn focus_workspace(
    window_id: String,
    pane_id: String,
    workspace_ref: WorkspaceRef,
    intent_epoch: u64,
    registry: State<'_, Arc<ProjectRegistry>>,
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
    persistence: State<'_, Arc<WindowContextPersistence>>,
    app_handle: tauri::AppHandle,
) -> Result<WindowPaneWorkspaceContext, AppError> {
    let _mutation = persistence
        .mutation
        .lock()
        .map_err(persistence_lock_error)?;
    window_contexts
        .validate_pane_intent(&window_id, &pane_id, intent_epoch)
        .map_err(window_context_error)?;
    let resolved = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(workspace_resolve_error)?;
    registry
        .ensure_runtime_initialized(resolved.runtime())
        .map_err(|detail| {
            AppError::new(
                "workspace.initialization_failed",
                "The workspace runtime could not be activated.",
            )
            .detail(detail)
        })?;
    let context = window_contexts
        .focus_scope(&window_id, &pane_id, resolved, intent_epoch)
        .map_err(window_context_error)?;
    persist_contexts_best_effort(&app_handle, &window_contexts);
    Ok(context)
}

fn validate_or_bind_active_session_checkout(
    store: &SessionStore,
    session_id: &str,
    checkout_id: &CheckoutId,
) -> Result<(), AppError> {
    let session_scope = store.get_session_workspace_scope(session_id)?;
    let session_project_id = session_scope
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|project_id| !project_id.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "session.project_scope_missing",
                "The session does not belong to a project.",
            )
            .detail(session_id.to_string())
        })?;
    let checkout = store
        .get_workspace_checkout(checkout_id.as_str())?
        .ok_or_else(|| {
            AppError::new(
                "workspace.checkout_unavailable",
                "The pane checkout is unavailable.",
            )
            .detail(checkout_id.to_string())
        })?;
    if checkout.project_id != session_project_id {
        return Err(AppError::new(
            "session.workspace_scope_conflict",
            "The session belongs to a different project.",
        )
        .detail(format!(
            "session={session_id}, sessionProject={session_project_id}, paneCheckout={checkout_id}, paneProject={}",
            checkout.project_id
        )));
    }

    if session_scope.default_checkout_id.is_none() {
        store
            .bind_session_default_checkout_if_missing(session_id, checkout_id.as_str())
            .map_err(|error| {
                AppError::new(
                    "session.checkout_binding_failed",
                    "The historical session checkout binding could not be saved.",
                )
                .detail(error)
            })?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_active_session(
    window_id: String,
    pane_id: String,
    active_session_id: Option<String>,
    intent_epoch: u64,
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    persistence: State<'_, Arc<WindowContextPersistence>>,
    app_handle: tauri::AppHandle,
) -> Result<WindowPaneWorkspaceContext, AppError> {
    let _mutation = persistence
        .mutation
        .lock()
        .map_err(persistence_lock_error)?;
    window_contexts
        .validate_pane_intent(&window_id, &pane_id, intent_epoch)
        .map_err(window_context_error)?;
    if let Some(session_id) = active_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        let pane = window_contexts
            .pane(&window_id, &pane_id)
            .map_err(window_context_error)?
            .ok_or_else(|| {
                window_context_error(WindowContextError::PaneUnavailable {
                    window_id: window_id.trim().to_string(),
                    pane_id: pane_id.trim().to_string(),
                })
            })?;
        validate_or_bind_active_session_checkout(
            store.inner().as_ref(),
            session_id,
            &pane.focused_checkout_id,
        )?;
    }
    let context = window_contexts
        .set_active_session(&window_id, &pane_id, active_session_id, intent_epoch)
        .map_err(window_context_error)?;
    persist_contexts_best_effort(&app_handle, &window_contexts);
    Ok(context)
}

#[tauri::command]
pub fn detach_workspace_pane(
    window_id: String,
    pane_id: String,
    intent_epoch: u64,
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
    persistence: State<'_, Arc<WindowContextPersistence>>,
    app_handle: tauri::AppHandle,
) -> Result<bool, AppError> {
    if crate::commands::app_exit_started() {
        return Ok(false);
    }
    let _mutation = persistence
        .mutation
        .lock()
        .map_err(persistence_lock_error)?;
    let removed = window_contexts
        .remove_pane(&window_id, &pane_id, intent_epoch)
        .map_err(window_context_error)?;
    persist_contexts_best_effort(&app_handle, &window_contexts);
    Ok(removed)
}

#[tauri::command]
pub fn detach_workspace_window(
    window_id: String,
    intent_epoch: u64,
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
    persistence: State<'_, Arc<WindowContextPersistence>>,
    app_handle: tauri::AppHandle,
) -> Result<usize, AppError> {
    if crate::commands::app_exit_started() {
        return Ok(0);
    }
    let _mutation = persistence
        .mutation
        .lock()
        .map_err(persistence_lock_error)?;
    let removed = window_contexts
        .remove_window(&window_id, intent_epoch)
        .map_err(window_context_error)?;
    persist_contexts_best_effort(&app_handle, &window_contexts);
    Ok(removed)
}

#[tauri::command]
pub fn list_window_workspace_contexts(
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
) -> Result<Vec<WindowPaneWorkspaceContext>, AppError> {
    window_contexts.snapshots().map_err(window_context_error)
}

#[tauri::command]
pub fn list_window_workspace_intent_epochs(
    window_contexts: State<'_, Arc<WindowContextRegistry>>,
) -> Result<Vec<WindowIntentEpochSnapshot>, AppError> {
    window_contexts
        .intent_epoch_snapshots()
        .map_err(window_context_error)
}

/// App-level service switches (Settings > Services pages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSwitches {
    pub unity: bool,
    pub blender: bool,
}

impl ServiceSwitches {
    fn from_config(config: &crate::config::AppConfig) -> Self {
        Self {
            unity: config.unity_enabled(),
            blender: config.blender_enabled(),
        }
    }
}

/// Leases of running tools drain before a switched-off service is removed;
/// a run still holding one after this bound finishes its removal later.
const SERVICE_RETIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[tauri::command]
pub fn get_service_switches(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<ServiceSwitches, AppError> {
    Ok(ServiceSwitches::from_config(config.inner()))
}

#[tauri::command]
pub async fn set_service_enabled(
    service: String,
    enabled: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
    app_handle: tauri::AppHandle,
) -> Result<ServiceSwitches, AppError> {
    let key = match service.trim() {
        "unity" => crate::settings::UNITY_ENABLED,
        "blender" => crate::settings::BLENDER_ENABLED,
        other => {
            return Err(AppError::new(
                "workspace.unknown_service",
                "Unknown workspace service.",
            )
            .detail(other.to_string()))
        }
    };
    let context = crate::settings::apply::WriteContext::new(app_handle, None);
    crate::settings::apply::write_one(&context, key, enabled).await?;
    Ok(ServiceSwitches::from_config(config.inner()))
}

/// Effect of `unity.enabled` / `blender.enabled`: enabling redetects the
/// service in every checkout; disabling stops watchers and retires the
/// service once running tools release it.
pub(crate) async fn apply_service_switch(
    app_handle: &tauri::AppHandle,
    kind: ServiceKind,
    enabled: bool,
) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let registry = app_handle.state::<Arc<ProjectRegistry>>().inner().clone();
    if enabled {
        let store = app_handle.state::<Arc<SessionStore>>();
        registry.redetect_services(store.inner().as_ref());
        if kind == ServiceKind::Unity {
            let watcher_tuning =
                app_handle.state::<crate::asset_db::watcher::WatcherTuningState>();
            for runtime in registry.runtimes() {
                if let Err(error) = runtime.core().start_background_watchers(
                    &runtime,
                    app_handle,
                    Arc::clone(&watcher_tuning.0),
                ) {
                    eprintln!(
                        "[Locus] failed to restart watchers for checkout {}: {}",
                        runtime.checkout_id(),
                        error
                    );
                }
            }
        }
    } else {
        if kind == ServiceKind::Unity {
            for runtime in registry.runtimes() {
                runtime.core().stop_asset_watcher();
            }
        }
        let busy = registry
            .retire_service_kind(kind, SERVICE_RETIRE_TIMEOUT)
            .await;
        if !busy.is_empty() {
            // The policy is already Disabled, so no new work binds; finish
            // removing the slots once the remaining runs release them.
            let registry = Arc::clone(&registry);
            tauri::async_runtime::spawn(async move {
                let still_busy = registry
                    .retire_service_kind(kind, std::time::Duration::from_secs(3600))
                    .await;
                if !still_busy.is_empty() {
                    eprintln!(
                        "[Locus] {} service still leased in {} checkout(s) after retirement",
                        kind.as_str(),
                        still_busy.len()
                    );
                }
            });
        }
    }
    let config = app_handle.state::<Arc<crate::config::AppConfig>>();
    let switches = ServiceSwitches::from_config(config.inner());
    if let Err(error) = app_handle.emit("workspace-services-changed", switches) {
        eprintln!("[Locus] failed to emit workspace-services-changed: {error}");
    }
    Ok(())
}

/// Start a dormant (lazy-mode) service from its status item.
#[tauri::command]
pub async fn start_workspace_service(
    workspace_ref: WorkspaceRef,
    service: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::workspace_service::service::ServiceBindingSnapshot, AppError> {
    let kind = ServiceKind::parse(&service).ok_or_else(|| {
        AppError::new("workspace.unknown_service", "Unknown workspace service.").detail(service.clone())
    })?;
    let registry = registry.inner().clone();
    let resolved = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(workspace_resolve_error)?;
    let execution = registry
        .execution_context(resolved.runtime().checkout_id(), &[kind])
        .await
        .map_err(|error| {
            AppError::new(
                "workspace.service_start_failed",
                "Failed to start the workspace service.",
            )
            .detail(error)
        })?;
    execution
        .binding(kind)
        .map(|binding| binding.snapshot())
        .ok_or_else(|| {
            AppError::new(
                "workspace.service_binding_missing",
                "The workspace service started without a binding.",
            )
        })
}

#[tauri::command]
pub async fn start_workspace_unity_service(
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::workspace_service::service::ServiceBindingSnapshot, AppError> {
    let registry = registry.inner().clone();
    let resolved = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(workspace_resolve_error)?;
    let execution = registry
        .execution_context(resolved.runtime().checkout_id(), &[ServiceKind::Unity])
        .await
        .map_err(|error| {
            AppError::new(
                "workspace.unity_service_start_failed",
                "Failed to start the Unity workspace service.",
            )
            .detail(error)
        })?;
    execution
        .binding(ServiceKind::Unity)
        .map(|binding| binding.snapshot())
        .ok_or_else(|| {
            AppError::new(
                "workspace.unity_service_binding_missing",
                "The Unity service started without a binding.",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_registry(config_dir: &std::path::Path) -> Arc<ProjectRegistry> {
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.join("config.json"),
        ));
        let policy =
            Arc::new(ResourcePolicyStore::from_config(config).expect("workspace resource policy"));
        ProjectRegistry::new(policy, Vec::new())
    }

    #[test]
    fn window_context_recovery_round_trips_checkout_and_active_session() {
        let temp = tempfile::tempdir().expect("recovery dir");
        let expected = WindowPaneWorkspaceContext {
            window_id: "main".to_string(),
            pane_id: "main".to_string(),
            focused_checkout_id: CheckoutId::new("checkout-a").expect("checkout id"),
            workspace_generation: 9,
            materialization_epoch: None,
            active_session_id: Some("session-a".to_string()),
            intent_epoch: 12,
            revision: 4,
        };
        let raw = serde_json::to_vec_pretty(&vec![expected.clone()]).expect("serialize recovery");
        crate::config::atomic_write_config(&temp.path().join(WINDOW_CONTEXT_RECOVERY_FILE), &raw)
            .expect("persist recovery");

        assert_eq!(load_window_context_recovery(temp.path()), vec![expected]);
    }

    #[test]
    fn malformed_window_context_recovery_is_ignored() {
        let temp = tempfile::tempdir().expect("recovery dir");
        std::fs::write(temp.path().join(WINDOW_CONTEXT_RECOVERY_FILE), "{invalid")
            .expect("write malformed recovery");

        assert!(load_window_context_recovery(temp.path()).is_empty());
    }

    #[test]
    fn active_session_checkout_binding_repairs_history_and_allows_sibling_worktrees() {
        let store_dir = tempfile::tempdir().expect("store dir");
        let store = SessionStore::new(store_dir.path()).expect("session store");
        for checkout in [
            ("checkout-a", "project-shared", "f:/shared-a"),
            ("checkout-b", "project-shared", "f:/shared-b"),
            ("checkout-other", "project-other", "f:/other"),
        ] {
            store
                .upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
                    checkout_id: checkout.0.to_string(),
                    project_id: checkout.1.to_string(),
                    root_path: checkout.2.to_string(),
                    normalized_root: checkout.2.to_string(),
                    last_opened_at: 1,
                })
                .expect("persist checkout");
        }
        let session_id = store
            .create_session("Historical", None, Some("project-shared"), "chat", None)
            .expect("create historical session");

        validate_or_bind_active_session_checkout(
            &store,
            &session_id,
            &CheckoutId::new("checkout-a").expect("checkout A"),
        )
        .expect("bind historical session from explicit pane");
        validate_or_bind_active_session_checkout(
            &store,
            &session_id,
            &CheckoutId::new("checkout-b").expect("checkout B"),
        )
        .expect("open shared session in sibling checkout");
        assert_eq!(
            store
                .get_session_workspace_scope(&session_id)
                .expect("load repaired session scope")
                .default_checkout_id
                .as_deref(),
            Some("checkout-a")
        );
        assert!(validate_or_bind_active_session_checkout(
            &store,
            &session_id,
            &CheckoutId::new("checkout-other").expect("other checkout"),
        )
        .is_err());
    }

    #[test]
    fn project_services_detect_unity_for_an_inactive_checkout() {
        let temp = tempfile::tempdir().expect("workspace dir");
        let unity_root = temp.path().join("unity-project");
        std::fs::create_dir_all(unity_root.join("Assets")).expect("Assets directory");
        std::fs::create_dir_all(unity_root.join("ProjectSettings"))
            .expect("ProjectSettings directory");
        let checkouts = vec![WorkspaceCheckoutDescriptor {
            checkout_id: "checkout-unity".to_string(),
            knowledge_enabled: true,
            project_id: "project-unity".to_string(),
            root: unity_root.display().to_string(),
            normalized_root: unity_root.display().to_string(),
            last_opened_at: 1,
            available: true,
            managed_worktree: false,
            runtime: None,
        }];

        assert_eq!(
            project_detected_services(&checkouts),
            vec![ServiceKind::Unity.as_str().to_string()]
        );
    }

    #[test]
    fn startup_recovery_restores_every_window_and_pane_with_scoped_sessions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let roots = [
            temp.path().join("checkout-a"),
            temp.path().join("checkout-b"),
        ];
        for root in &roots {
            std::fs::create_dir_all(root).expect("workspace root");
        }
        let identities = roots
            .iter()
            .map(|root| {
                crate::workspace_service::identity::ProjectIdResolver::resolve(root)
                    .expect("workspace identity")
            })
            .collect::<Vec<_>>();
        let store_dir = tempfile::tempdir().expect("store dir");
        let store = Arc::new(SessionStore::new(store_dir.path()).expect("session store"));
        for (root, identity) in roots.iter().zip(&identities) {
            store
                .upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
                    checkout_id: identity.checkout_id.to_string(),
                    project_id: identity.project_id.to_string(),
                    root_path: root.display().to_string(),
                    normalized_root: identity.normalized_root.clone(),
                    last_opened_at: 1,
                })
                .expect("persist checkout");
        }
        let session_a = store
            .create_session_scoped(
                "A",
                None,
                Some(identities[0].project_id.as_str()),
                Some(identities[0].checkout_id.as_str()),
                "chat",
                None,
            )
            .expect("session A");
        let session_b = store
            .create_session_scoped(
                "B",
                None,
                Some(identities[1].project_id.as_str()),
                Some(identities[1].checkout_id.as_str()),
                "chat",
                None,
            )
            .expect("session B");

        let recovered = vec![
            WindowPaneWorkspaceContext {
                window_id: "main".to_string(),
                pane_id: "main".to_string(),
                focused_checkout_id: identities[0].checkout_id.clone(),
                workspace_generation: 99,
                materialization_epoch: None,
                active_session_id: Some(session_a.clone()),
                intent_epoch: 7,
                revision: 4,
            },
            WindowPaneWorkspaceContext {
                window_id: "main".to_string(),
                pane_id: "secondary".to_string(),
                focused_checkout_id: identities[1].checkout_id.clone(),
                workspace_generation: 98,
                materialization_epoch: None,
                active_session_id: Some(session_b),
                intent_epoch: 3,
                revision: 2,
            },
            WindowPaneWorkspaceContext {
                window_id: "knowledge-window".to_string(),
                pane_id: "main".to_string(),
                focused_checkout_id: identities[1].checkout_id.clone(),
                workspace_generation: 97,
                materialization_epoch: None,
                // A checkout-A session must not survive on checkout B.
                active_session_id: Some(session_a.clone()),
                intent_epoch: 5,
                revision: 3,
            },
        ];
        let registry = test_registry(temp.path());
        registry
            .attach_session_store(&store)
            .expect("attach session catalog");
        let contexts = WindowContextRegistry::new();

        let outcome = restore_persisted_window_contexts(
            recovered,
            None,
            &[],
            registry.as_ref(),
            &contexts,
            store.as_ref(),
        )
        .expect("restore contexts");

        assert_eq!(outcome.restored_panes, 3);
        assert_eq!(contexts.snapshots().expect("snapshots").len(), 3);
        let main = contexts
            .pane("main", "main")
            .expect("main read")
            .expect("main pane");
        assert_eq!(main.active_session_id.as_deref(), Some(session_a.as_str()));
        let invalid = contexts
            .pane("knowledge-window", "main")
            .expect("window read")
            .expect("window pane");
        assert!(invalid.active_session_id.is_none());
        let main_runtime = outcome.main_runtime.expect("main runtime");
        assert_eq!(
            main_runtime
                .activity_snapshot(std::time::Duration::MAX)
                .priority,
            crate::resource_policy::WorkspaceActivityPriority::VisiblePane
        );
        let secondary_runtime = registry
            .runtime(&identities[1].checkout_id)
            .expect("secondary runtime");
        assert_eq!(
            secondary_runtime
                .activity_snapshot(std::time::Duration::MAX)
                .background_open_leases,
            2
        );
        assert_eq!(
            registry
                .project(&identities[1].project_id)
                .expect("secondary project")
                .runtimes()
                .len(),
            1
        );
    }

    #[test]
    fn startup_recovery_skips_missing_or_replaced_roots_without_registering_them() {
        for scenario in ["renamed", "deleted", "file", "replaced"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("中文工程");
            std::fs::create_dir_all(root.join("Locus")).unwrap();
            std::fs::write(root.join("Locus/config.json"), r#"{"workspace_id":"original-project"}"#).unwrap();
            let identity = crate::workspace_service::identity::ProjectIdResolver::resolve(&root).unwrap();
            let store_dir = tempfile::tempdir().unwrap();
            let store = SessionStore::new(store_dir.path()).unwrap();
            let record = crate::session::models::WorkspaceCheckoutRecord {
                checkout_id: identity.checkout_id.to_string(),
                project_id: identity.project_id.to_string(),
                root_path: root.display().to_string(),
                normalized_root: identity.normalized_root,
                last_opened_at: 1,
            };
            store.upsert_workspace_checkout(&record).unwrap();
            let moved = temp.path().join("EnglishProject");
            std::fs::rename(&root, &moved).unwrap();
            match scenario {
                "deleted" => std::fs::remove_dir_all(&moved).unwrap(),
                "file" => std::fs::write(&root, "replaced by file").unwrap(),
                "replaced" => {
                    std::fs::create_dir_all(root.join("Locus")).unwrap();
                    std::fs::write(root.join("Locus/config.json"), r#"{"workspace_id":"different-project"}"#).unwrap();
                }
                _ => {}
            }
            let registry = test_registry(temp.path());
            registry.add_runtime_registration_hook(Arc::new(|_| {
                panic!("recovery must inspect disk identity before running registration hooks")
            })).unwrap();
            let contexts = WindowContextRegistry::new();
            let outcome = restore_persisted_window_contexts(
                vec![WindowPaneWorkspaceContext {
                    window_id: "main".into(), pane_id: "main".into(),
                    focused_checkout_id: identity.checkout_id.clone(),
                    workspace_generation: 1, materialization_epoch: None,
                    active_session_id: None, intent_epoch: 1, revision: 1,
                }],
                None, &[], registry.as_ref(), &contexts, &store,
            ).unwrap();
            assert!(outcome.main_runtime.is_none(), "{scenario}");
            assert_eq!(outcome.restored_panes, 0, "{scenario}");
            assert_eq!(outcome.warnings.len(), 1, "{scenario}");
            assert!(contexts.snapshots().unwrap().is_empty());
            assert_eq!(registry.checkout_count(), 0);
            assert_eq!(store.get_workspace_checkout(identity.checkout_id.as_str()).unwrap().unwrap().project_id,
                "original-project");
            if scenario == "renamed" {
                let reopened = test_registry(temp.path()).open_workspace(&moved).unwrap();
                assert_eq!(reopened.project_id(), &identity.project_id);
                assert_ne!(reopened.checkout_id(), &identity.checkout_id);
                assert!(reopened.root().ends_with("EnglishProject"));
            }
        }
    }

    #[test]
    fn legacy_workspace_root_is_materialized_as_main_recovery_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("legacy-workspace");
        std::fs::create_dir_all(&root).expect("legacy workspace root");
        let store_dir = tempfile::tempdir().expect("store dir");
        let store = Arc::new(SessionStore::new(store_dir.path()).expect("session store"));
        let registry = test_registry(temp.path());
        registry
            .attach_session_store(&store)
            .expect("attach session catalog");
        let contexts = WindowContextRegistry::new();

        let outcome = restore_persisted_window_contexts(
            Vec::new(),
            Some(&root.to_string_lossy()),
            &[],
            registry.as_ref(),
            &contexts,
            store.as_ref(),
        )
        .expect("materialize legacy root");

        let main = contexts
            .pane("main", "main")
            .expect("main read")
            .expect("main recovery context");
        let runtime = outcome.main_runtime.expect("main runtime");
        assert_eq!(main.focused_checkout_id, *runtime.checkout_id());
        assert_eq!(main.workspace_generation, runtime.generation());
        assert_eq!(main.intent_epoch, 1);
        assert_eq!(
            runtime.activity_snapshot(std::time::Duration::MAX).priority,
            crate::resource_policy::WorkspaceActivityPriority::VisiblePane
        );
    }
}
