use std::{path::Path, sync::Arc};

use crate::session::store::SessionStore;
use crate::workspace_service::pool::{self, AcquirePoolRequest, PoolAcquisition};
use crate::workspace_service::worktrees::{
    self, CreateWorktreeRequest, ManagedWorktree, WorktreeOperation,
};
use crate::workspace_service::{CheckoutId, ProjectRegistry};
use tauri::State;
use serde::Deserialize;
use crate::workspace_service::WorkspaceRef;
use worktrees::estimate::{Progress, WorktreePlanProgress};
use tauri::ipc::{Channel, JavaScriptChannelId};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectWorktreeRequest {
    workspace_ref: WorkspaceRef,
    branch: String,
    create_branch: bool,
    include_dirty: bool,
    #[serde(default)]
    allow_new_project: bool,
    expected_start_oid: Option<String>,
    start_ref: Option<String>,
}

#[tauri::command]
pub async fn plan_worktree_selection(
    request: SelectWorktreeRequest,
    registry: State<'_, Arc<ProjectRegistry>>,
    on_progress: Option<JavaScriptChannelId>,
    webview: tauri::Webview,
) -> Result<worktrees::usage::WorktreeCreationPlan, String> {
    let on_progress: Option<Channel<WorktreePlanProgress>> = on_progress.map(|id| id.channel_on(webview));
    let scope = registry.resolve_workspace_ref(&request.workspace_ref).map_err(|e| e.to_string())?;
    let max_slots = registry.resource_policy().snapshot().limits.max_unity_editors;
    tauri::async_runtime::spawn_blocking(move || worktrees::usage::selection_plan_with_progress(
        scope.runtime().root(), &request.branch, request.create_branch, request.include_dirty, max_slots, request.start_ref.as_deref(),
        &mut Progress::new(&|event| { if let Some(channel) = &on_progress { let _ = channel.send(event); } }),
    )).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_worktree_pool_usage(source_root: String, registry: State<'_, Arc<ProjectRegistry>>) -> Result<worktrees::usage::WorktreePoolUsage, String> {
    registered_source(&registry, &source_root)?;
    tauri::async_runtime::spawn_blocking(move || worktrees::usage::pool_usage(Path::new(&source_root))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_all_worktree_pool_usage(store: State<'_, Arc<SessionStore>>) -> Result<Vec<worktrees::usage::WorktreeProjectUsage>, String> {
    let checkouts = store.list_visible_workspace_checkouts()?.into_iter()
        .map(|checkout| (checkout.project_id, checkout.root_path)).collect();
    tauri::async_runtime::spawn_blocking(move || worktrees::usage::project_pool_usages(checkouts)).await.map_err(|error| error.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWorktreeCreationRequest {
    source_root: String,
    start_ref: String,
    include_dirty: bool,
    pool_mode: bool,
    directory: String,
    max_slots: usize,
}

#[tauri::command]
pub async fn plan_worktree_creation(request: PlanWorktreeCreationRequest, registry: State<'_, Arc<ProjectRegistry>>, on_progress: Option<JavaScriptChannelId>, webview: tauri::Webview) -> Result<worktrees::usage::WorktreeCreationPlan, String> {
    let on_progress: Option<Channel<WorktreePlanProgress>> = on_progress.map(|id| id.channel_on(webview));
    registered_source(&registry, &request.source_root)?;
    tauri::async_runtime::spawn_blocking(move || worktrees::usage::creation_plan_with_progress(
        Path::new(&request.source_root), &request.start_ref, request.include_dirty, request.pool_mode,
        Some(Path::new(&request.directory)), request.max_slots,
        &mut Progress::new(&|event| { if let Some(channel) = &on_progress { let _ = channel.send(event); } }),
    )).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_worktree_branches(
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<worktrees::selector::WorktreeBranchOption>, String> {
    let scope = registry.resolve_workspace_ref(&workspace_ref).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || worktrees::selector::branches(scope.runtime().root()))
        .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn select_worktree_branch(
    request: SelectWorktreeRequest,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    on_progress: Option<JavaScriptChannelId>,
    webview: tauri::Webview,
) -> Result<ManagedWorktree, String> {
    let on_progress: Option<Channel<WorktreePlanProgress>> = on_progress.map(|id| id.channel_on(webview));
    let scope = registry.resolve_workspace_ref(&request.workspace_ref).map_err(|e| e.to_string())?;
    let max_slots = registry.resource_policy().snapshot().limits.max_unity_editors;
    let record = tauri::async_runtime::spawn_blocking(move || {
        let report = |event| { if let Some(channel) = &on_progress { let _ = channel.send(event); } };
        let plan = worktrees::usage::selection_plan_with_progress(scope.runtime().root(), &request.branch, request.create_branch, request.include_dirty, max_slots, request.start_ref.as_deref(), &mut Progress::new(&report))?;
        if request.expected_start_oid.as_ref().is_some_and(|oid| *oid != plan.start_oid) { return Err("Source branch changed; review the worktree creation again".into()); }
        if plan.at_capacity { return Err("Project pool is at capacity; manually recycle a project in settings".into()); }
        if request.allow_new_project && plan.budget.as_ref().is_some_and(|budget| budget.free_bytes.is_some_and(|free| free < budget.estimated_bytes)) {
            return Err("Insufficient disk space for the estimated project size".into());
        }
        report(WorktreePlanProgress { phase: "creating", files: 0, total_files: None, bytes: 0 });
        worktrees::selector::select_from_ref(
            scope.runtime().root(), &request.branch, request.create_branch, request.include_dirty, max_slots, request.allow_new_project,
            request.start_ref.as_deref(),
        )
    }).await.map_err(|e| e.to_string())??;
    register_record(&registry, &store, &record)?;
    Ok(record)
}

fn registered_source(registry: &ProjectRegistry, source: &str) -> Result<(), String> {
    registry
        .runtime_for_root(Path::new(source))
        .ok_or("Source workspace must be open in Locus")?;
    Ok(())
}

pub(crate) fn register_record(
    registry: &ProjectRegistry,
    store: &SessionStore,
    record: &ManagedWorktree,
) -> Result<(), String> {
    let runtime = registry.register(&record.root)?;
    if runtime.project_id().as_str() != record.project_id {
        return Err("Registered worktree project identity changed".into());
    }
    store.upsert_workspace_checkout(&crate::session::models::WorkspaceCheckoutRecord {
        checkout_id: runtime.checkout_id().to_string(),
        project_id: runtime.project_id().to_string(),
        root_path: record.root.clone(),
        normalized_root: runtime.normalized_root().into(),
        last_opened_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    })?;
    Ok(())
}

#[tauri::command]
pub async fn create_worktree(
    request: CreateWorktreeRequest,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<ManagedWorktree, String> {
    registered_source(&registry, &request.source_root)?;
    let record = tauri::async_runtime::spawn_blocking(move || worktrees::create(&request))
        .await
        .map_err(|e| e.to_string())??;
    register_record(&registry, &store, &record)?;
    Ok(record)
}

#[tauri::command]
pub async fn list_managed_worktrees(
    source_root: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<ManagedWorktree>, String> {
    registered_source(&registry, &source_root)?;
    tauri::async_runtime::spawn_blocking(move || worktrees::list(Path::new(&source_root)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn discover_worktrees(
    source_root: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<String>, String> {
    registered_source(&registry, &source_root)?;
    tauri::async_runtime::spawn_blocking(move || worktrees::discover(Path::new(&source_root)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn import_worktree(
    source_root: String,
    target_root: String,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<ManagedWorktree, String> {
    registered_source(&registry, &source_root)?;
    let record = tauri::async_runtime::spawn_blocking(move || {
        worktrees::import(Path::new(&source_root), Path::new(&target_root))
    })
    .await
    .map_err(|e| e.to_string())??;
    register_record(&registry, &store, &record)?;
    Ok(record)
}

#[tauri::command]
pub async fn remove_managed_worktree(
    source_root: String,
    checkout_id: String,
    expected_epoch: u64,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), String> {
    registered_source(&registry, &source_root)?;
    let record = worktrees::list(Path::new(&source_root))?
        .into_iter()
        .find(|record| record.checkout_id == checkout_id)
        .ok_or("Checkout is outside the source repository")?;
    if !record.managed
        || record.materialization_epoch != expected_epoch
        || record.assignment_id.is_some()
    {
        return Err(
            "Only an unassigned managed checkout with the current epoch may be removed".into(),
        );
    }
    worktrees::ensure_recyclable(&record)?;
    registry
        .retire_managed_checkout(&CheckoutId::new(&checkout_id).map_err(|e| e.to_string())?)
        .await?;
    tauri::async_runtime::spawn_blocking(move || {
        worktrees::remove(Path::new(&source_root), &checkout_id, expected_epoch)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_worktree_operations(
    source_root: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<WorktreeOperation>, String> {
    registered_source(&registry, &source_root)?;
    tauri::async_runtime::spawn_blocking(move || worktrees::operations(Path::new(&source_root)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn bind_session_worktree(
    session_id: String,
    checkout_id: String,
    expected_epoch: u64,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), String> {
    let id = CheckoutId::new(checkout_id).map_err(|e| e.to_string())?;
    let runtime = registry.activate_persisted_checkout(&id)?;
    if runtime.materialization_epoch() != expected_epoch {
        return Err("Stale materialization epoch".into());
    }
    store.bind_session_checkout(&session_id, id.as_str(), expected_epoch)
}

#[tauri::command]
pub async fn acquire_unity_project_slot(
    request: AcquirePoolRequest,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    allow_new_project: Option<bool>,
) -> Result<PoolAcquisition, String> {
    registered_source(&registry, &request.source_root)?;
    let acquired = tauri::async_runtime::spawn_blocking(move || pool::acquire_with_creation_policy(&request, allow_new_project.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())??;
    register_record(&registry, &store, &acquired.worktree)?;
    Ok(acquired)
}

#[tauri::command]
pub async fn release_unity_project_slot(
    source_root: String,
    checkout_id: String,
    assignment_id: String,
    expected_epoch: u64,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ManagedWorktree, String> {
    registered_source(&registry, &source_root)?;
    let record = worktrees::list(Path::new(&source_root))?
        .into_iter()
        .find(|record| record.checkout_id == checkout_id)
        .ok_or("Pool slot is outside the source repository")?;
    if !record.pool_slot
        || record.assignment_id.as_deref() != Some(&assignment_id)
        || record.materialization_epoch != expected_epoch
    {
        return Err("Pool assignment or materialization epoch is stale".into());
    }
    worktrees::ensure_recyclable(&record)?;
    registry
        .retire_managed_checkout(&CheckoutId::new(&checkout_id).map_err(|e| e.to_string())?)
        .await?;
    tauri::async_runtime::spawn_blocking(move || {
        pool::release(
            Path::new(&source_root),
            &checkout_id,
            &assignment_id,
            expected_epoch,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
