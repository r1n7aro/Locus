use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tauri::{AppHandle, State};

use crate::asset_db::types::{guid_to_hex, parse_guid_hex, ScanPhase, ScanStats};
use crate::asset_db::AssetDb;
use crate::commands::asset::{
    resolve_asset_workspace_scope, write_persisted_last_scan_info, LastScanInfo, LastScanInfoState,
    ScanPhaseState,
};
use crate::error::AppError;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

const REF_GRAPH_SCAN_CANCEL_WAIT_MS: u64 = 30_000;
const REF_GRAPH_SCAN_CANCELLED_DETAIL: &str = "Asset database scan cancelled.";

#[derive(Clone)]
struct RefGraphScanTask {
    cwd: String,
    workspace_generation: u64,
    cancel: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone, Default)]
pub struct RefGraphScanTaskState {
    inner: Arc<Mutex<Option<RefGraphScanTask>>>,
}

impl RefGraphScanTaskState {
    pub fn new() -> Self {
        Self::default()
    }

    fn register_scoped(
        &self,
        _scope_key: &str,
        cwd: String,
        workspace_generation: u64,
    ) -> RefGraphScanRegistration {
        let task = RefGraphScanTask {
            cwd,
            workspace_generation,
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new((Mutex::new(false), Condvar::new())),
        };

        let previous = self
            .inner
            .lock()
            .ok()
            .and_then(|mut guard| guard.replace(task.clone()));
        if let Some(previous) = previous {
            previous.cancel.store(true, Ordering::Relaxed);
            eprintln!(
                "[AssetDb] replacing active scan for {} generation {}; waiting for cancellation",
                previous.cwd, previous.workspace_generation
            );
            let (done_lock, done_cvar) = &*previous.done;
            let previous_finished = done_lock
                .lock()
                .ok()
                .and_then(|done| {
                    done_cvar
                        .wait_timeout_while(
                            done,
                            Duration::from_millis(REF_GRAPH_SCAN_CANCEL_WAIT_MS),
                            |finished| !*finished,
                        )
                        .ok()
                        .map(|(finished, _)| *finished)
                })
                .unwrap_or(false);
            if !previous_finished {
                eprintln!(
                    "[AssetDb] replacement scan cancelled because generation {} did not stop in time",
                    previous.workspace_generation
                );
                task.cancel.store(true, Ordering::Relaxed);
            }
        }

        RefGraphScanRegistration {
            state: self.clone(),
            task,
        }
    }

    pub fn cancel_current_and_wait(&self, reason: &str) -> bool {
        self.cancel_current_and_wait_for(
            reason,
            Duration::from_millis(REF_GRAPH_SCAN_CANCEL_WAIT_MS),
        )
    }

    fn cancel_current_and_wait_for(&self, reason: &str, timeout: Duration) -> bool {
        let task = match self.inner.lock() {
            Ok(guard) => guard.clone(),
            Err(error) => {
                eprintln!("[AssetDb] failed to lock scan task state for cancellation: {error}");
                return false;
            }
        };
        let Some(task) = task else {
            return true;
        };

        task.cancel.store(true, Ordering::Relaxed);
        eprintln!(
            "[AssetDb] cancelling active scan for {} generation {} ({})",
            task.cwd, task.workspace_generation, reason
        );

        let (done_lock, done_cvar) = &*task.done;
        let done_guard = match done_lock.lock() {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("[AssetDb] failed to lock scan completion state: {error}");
                return false;
            }
        };
        let wait_result = done_cvar.wait_timeout_while(done_guard, timeout, |done| !*done);
        match wait_result {
            Ok((guard, _timeout_result)) => {
                if *guard {
                    eprintln!("[AssetDb] active scan cancelled before workspace switch");
                    true
                } else {
                    eprintln!(
                        "[AssetDb] timed out waiting for active scan cancellation after {}ms",
                        timeout.as_millis()
                    );
                    false
                }
            }
            Err(error) => {
                eprintln!("[AssetDb] failed while waiting for scan cancellation: {error}");
                false
            }
        }
    }

    fn finish(&self, task: &RefGraphScanTask) {
        let (done_lock, done_cvar) = &*task.done;
        if let Ok(mut done) = done_lock.lock() {
            *done = true;
            done_cvar.notify_all();
        }

        if let Ok(mut guard) = self.inner.lock() {
            if guard
                .as_ref()
                .map(|current| Arc::ptr_eq(&current.cancel, &task.cancel))
                .unwrap_or(false)
            {
                *guard = None;
            }
        }
    }

    #[cfg(test)]
    fn has_active_task(&self) -> bool {
        self.inner
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }
}

struct RefGraphScanRegistration {
    state: RefGraphScanTaskState,
    task: RefGraphScanTask,
}

impl RefGraphScanRegistration {
    fn cancel_token(&self) -> Arc<AtomicBool> {
        self.task.cancel.clone()
    }
}

impl Drop for RefGraphScanRegistration {
    fn drop(&mut self) {
        self.state.finish(&self.task);
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefGraphScanStartResult {
    pub started: bool,
    pub already_running: bool,
}

fn scan_already_running_error() -> AppError {
    AppError::new(
        "ref_graph.scan_already_running",
        "Asset database scan is already running.",
    )
    .retryable(true)
}

fn validate_scan_workspace(cwd: &str) -> Result<std::path::PathBuf, AppError> {
    let project_root = std::path::Path::new(cwd);
    if !project_root.join("Assets").is_dir() {
        return Err(AppError::new(
            "ref_graph.not_unity_project",
            "Not a Unity project (Assets/ not found)",
        ));
    }
    Ok(project_root.to_path_buf())
}

fn stale_scan_error() -> AppError {
    AppError::new(
        "ref_graph.scan_stale",
        "Asset database scan was superseded by a workspace change.",
    )
    .retryable(true)
}

fn scoped_runtime_is_current(
    registry: &ProjectRegistry,
    runtime: &Arc<crate::workspace_service::WorkspaceRuntime>,
) -> bool {
    registry
        .runtime(runtime.checkout_id())
        .is_some_and(|current| {
            Arc::ptr_eq(&current, runtime) && current.generation() == runtime.generation()
        })
}

fn emit_scoped_scan_phase(
    app_handle: &AppHandle,
    registry: &ProjectRegistry,
    runtime: &crate::workspace_service::WorkspaceRuntime,
    scan_phase_state: &ScanPhaseState,
    phase: ScanPhase,
) {
    scan_phase_state.set(Some(phase.clone()));
    registry.event_router().publish(
        app_handle,
        "ref-graph-scan",
        crate::workspace_service::event::WorkspaceEventEnvelope {
            project_id: runtime.project_id().clone(),
            checkout_id: runtime.checkout_id().clone(),
            workspace_generation: runtime.generation(),
            service_instance_id: None,
            service_generation: None,
            payload: phase,
        },
    );
}

async fn run_scoped_ref_graph_scan_job(
    app_handle: AppHandle,
    registry: Arc<ProjectRegistry>,
    resolved_scope: crate::workspace_service::ResolvedWorkspaceScope,
    last_scan_info: LastScanInfoState,
    scan_phase_state: ScanPhaseState,
    watcher_tuning: Arc<crate::asset_db::watcher::WatcherTuning>,
    cancel_token: Arc<AtomicBool>,
) -> Result<ScanStats, AppError> {
    let (runtime, _lease) = resolved_scope.into_parts();
    let project_root = validate_scan_workspace(&runtime.root().to_string_lossy())?;
    let scan_started = std::time::Instant::now();

    runtime.core().stop_background_watchers();
    let graph_state = runtime.core().asset_db();
    {
        let mut graph = graph_state.lock().map_err(|error| {
            AppError::new("ref_graph.lock_failed", format!("Lock error: {error}"))
        })?;
        *graph = None;
    }

    let root = project_root.clone();
    let handle = app_handle.clone();
    let registry_for_scan = Arc::clone(&registry);
    let runtime_for_scan = Arc::clone(&runtime);
    let phase_for_scan = scan_phase_state.clone();
    let cancel_for_scan = Arc::clone(&cancel_token);
    let result = tauri::async_runtime::spawn_blocking(move || {
        if cancel_for_scan.load(Ordering::Relaxed) {
            return Err(REF_GRAPH_SCAN_CANCELLED_DETAIL.to_string());
        }
        let mut graph = AssetDb::open(&root)?;
        let cancel_for_progress = Arc::clone(&cancel_for_scan);
        let handle_for_progress = handle.clone();
        let registry_for_progress = Arc::clone(&registry_for_scan);
        let runtime_for_progress = Arc::clone(&runtime_for_scan);
        let phase_for_progress = phase_for_scan.clone();
        let stats = graph.full_scan_with_cancel(
            move |phase| {
                if cancel_for_progress.load(Ordering::Relaxed)
                    || !scoped_runtime_is_current(&registry_for_progress, &runtime_for_progress)
                {
                    return;
                }
                emit_scoped_scan_phase(
                    &handle_for_progress,
                    &registry_for_progress,
                    &runtime_for_progress,
                    &phase_for_progress,
                    phase.clone(),
                );
            },
            &cancel_for_scan,
        )?;
        if cancel_for_scan.load(Ordering::Relaxed) {
            return Err(REF_GRAPH_SCAN_CANCELLED_DETAIL.to_string());
        }
        let (graph, _reconcile_stats) =
            crate::asset_db::watcher::reconcile_loaded_db_with_cancel_and_progress(
                &root,
                graph,
                &cancel_for_scan,
                false,
                |_| {},
            )?;
        Ok::<(AssetDb, ScanStats), String>((graph, stats))
    })
    .await
    .map_err(|error| {
        AppError::new(
            "ref_graph.scan_task_join_failed",
            format!("Task join error: {error}"),
        )
        .retryable(true)
    })?;

    if cancel_token.load(Ordering::Relaxed) || !scoped_runtime_is_current(&registry, &runtime) {
        scan_phase_state.clear();
        return Err(stale_scan_error());
    }

    match result {
        Ok((graph, stats)) => {
            {
                let mut current = graph_state.lock().map_err(|error| {
                    AppError::new("ref_graph.lock_failed", format!("Lock error: {error}"))
                })?;
                *current = Some(graph);
            }
            let scan_info = LastScanInfo {
                finished_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
                duration_ms: stats
                    .elapsed_ms
                    .max(scan_started.elapsed().as_millis() as u64),
                stats: stats.clone(),
            };
            last_scan_info.set(scan_info.clone());
            if let Err(error) = write_persisted_last_scan_info(&project_root, &scan_info) {
                eprintln!("[AssetDb] warning: failed to persist scoped scan info: {error}");
            }
            scan_phase_state.clear();
            if let Err(error) =
                runtime
                    .core()
                    .start_background_watchers(&runtime, &app_handle, watcher_tuning)
            {
                eprintln!("[AssetDb] warning: failed to restart scoped watcher: {error}");
            }
            registry.event_router().publish(
                &app_handle,
                "ref-graph-scan",
                crate::workspace_service::event::WorkspaceEventEnvelope {
                    project_id: runtime.project_id().clone(),
                    checkout_id: runtime.checkout_id().clone(),
                    workspace_generation: runtime.generation(),
                    service_instance_id: None,
                    service_generation: None,
                    payload: ScanPhase::Done {
                        stats: stats.clone(),
                    },
                },
            );
            Ok(stats)
        }
        Err(detail) => {
            let error = AppError::new("ref_graph.scan_failed", detail).retryable(true);
            emit_scoped_scan_phase(
                &app_handle,
                &registry,
                &runtime,
                &scan_phase_state,
                ScanPhase::Error {
                    error: error.clone(),
                },
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn ref_graph_status(
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Option<ScanStats>, AppError> {
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    if scope.root().as_os_str().is_empty() {
        return Ok(None);
    }
    let last = scope
        .resolved()
        .runtime()
        .core()
        .asset_last_scan_info()
        .snapshot()
        .or_else(|| {
            crate::commands::asset::read_persisted_last_scan_info(scope.root())
                .ok()
                .flatten()
        });
    if let Some(info) = last {
        return Ok(Some(info.stats));
    }
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    match &*guard {
        Some(graph) => {
            let (nodes, edges) = graph.get_stats()?;
            let duplicate_guids = graph.get_duplicate_guid_overview()?;
            Ok(Some(ScanStats {
                nodes_added: nodes,
                edges_added: edges,
                duplicate_guids,
                ..Default::default()
            }))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn ref_graph_scan(
    app_handle: AppHandle,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<ScanStats, AppError> {
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    validate_scan_workspace(&scope.root_string())?;
    let resolved = scope.into_resolved();
    let last_scan_info = resolved.runtime().core().asset_last_scan_info().clone();
    let scan_phase_state = resolved.runtime().core().asset_scan_phase().clone();
    let scan_task_state = resolved.runtime().core().ref_graph_scan_tasks().clone();
    if !scan_phase_state.try_begin_scan()? {
        return Err(scan_already_running_error());
    }
    let runtime_generation = resolved.runtime().generation();
    let cwd = resolved.runtime().root().to_string_lossy().to_string();
    let scan_owner_key = resolved.runtime().checkout_id().to_string();
    let scan_registration =
        scan_task_state.register_scoped(&scan_owner_key, cwd, runtime_generation);
    let cancel_token = scan_registration.cancel_token();
    let result = run_scoped_ref_graph_scan_job(
        app_handle,
        project_registry.inner().clone(),
        resolved,
        last_scan_info,
        scan_phase_state,
        watcher_tuning.0.clone(),
        cancel_token,
    )
    .await;
    drop(scan_registration);
    result
}

#[tauri::command]
pub async fn ref_graph_scan_start(
    app_handle: AppHandle,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<RefGraphScanStartResult, AppError> {
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    validate_scan_workspace(&scope.root_string())?;
    let resolved = scope.into_resolved();
    let last_scan_info = resolved.runtime().core().asset_last_scan_info().clone();
    let scan_phase_state = resolved.runtime().core().asset_scan_phase().clone();
    let scan_task_state = resolved.runtime().core().ref_graph_scan_tasks().clone();
    if !scan_phase_state.try_begin_scan()? {
        return Ok(RefGraphScanStartResult {
            started: false,
            already_running: true,
        });
    }
    let runtime_generation = resolved.runtime().generation();
    let cwd = resolved.runtime().root().to_string_lossy().to_string();
    let scan_owner_key = resolved.runtime().checkout_id().to_string();
    let scan_registration =
        scan_task_state.register_scoped(&scan_owner_key, cwd, runtime_generation);
    let cancel_token = scan_registration.cancel_token();
    let app_handle_for_scan = app_handle.clone();
    let registry_for_scan = project_registry.inner().clone();
    let last_for_scan = last_scan_info;
    let phase_for_scan = scan_phase_state;
    let tuning_for_scan = watcher_tuning.0.clone();
    tauri::async_runtime::spawn(async move {
        let _scan_registration = scan_registration;
        if let Err(error) = run_scoped_ref_graph_scan_job(
            app_handle_for_scan,
            registry_for_scan,
            resolved,
            last_for_scan,
            phase_for_scan,
            tuning_for_scan,
            cancel_token,
        )
        .await
        {
            eprintln!("[AssetDb] scoped background scan failed: {error}");
        }
    });
    Ok(RefGraphScanStartResult {
        started: true,
        already_running: false,
    })
}

#[tauri::command]
pub async fn ref_graph_deps(
    guid_hex: String,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    let edges = graph.get_direct_deps(&guid)?;
    Ok(edges_to_json(&edges, graph))
}

#[tauri::command]
pub async fn ref_graph_refs(
    guid_hex: String,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    let edges = graph.get_direct_refs(&guid)?;
    Ok(edges_to_json(&edges, graph))
}

#[tauri::command]
pub async fn ref_graph_resolve_guid(
    path: String,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Option<String>, AppError> {
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    Ok(graph.resolve_guid_by_path(&path)?.map(|g| guid_to_hex(&g)))
}

#[tauri::command]
pub async fn ref_graph_resolve_path(
    guid_hex: String,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Option<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    graph.resolve_path_by_guid(&guid).map_err(Into::into)
}

#[tauri::command]
pub async fn ref_graph_walk_deps(
    guid_hex: String,
    max_depth: u32,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    let guids = graph.walk_deps(&guid, max_depth)?;
    Ok(guids.iter().map(guid_to_hex).collect())
}

#[tauri::command]
pub async fn ref_graph_walk_refs(
    guid_hex: String,
    max_depth: u32,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .as_ref()
        .ok_or("AssetDb not initialized. Run scan first.")?;
    let guids = graph.walk_refs(&guid, max_depth)?;
    Ok(guids.iter().map(guid_to_hex).collect())
}

fn split_search_terms(query: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(query.len());
    let mut prev_was_lower_or_digit = false;

    for ch in query.chars() {
        if ch == '@' || ch == '/' {
            continue;
        }

        if ch.is_ascii_uppercase() && prev_was_lower_or_digit && !normalized.ends_with(' ') {
            normalized.push(' ');
        }

        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            prev_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            if !normalized.ends_with(' ') {
                normalized.push(' ');
            }
            prev_was_lower_or_digit = false;
        }
    }

    let mut terms = Vec::new();
    for term in normalized.split_whitespace() {
        if terms.iter().any(|existing| existing == term) {
            continue;
        }
        terms.push(term.to_string());
    }
    terms
}

fn build_asset_name_query(query: &str) -> Option<String> {
    let terms = split_search_terms(query);
    if terms.is_empty() {
        return None;
    }

    Some(
        terms
            .into_iter()
            .map(|term| format!("n:{}", term))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[tauri::command]
pub async fn search_assets(
    query: String,
    workspace_ref: WorkspaceRef,
    project_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let scope = resolve_asset_workspace_scope(&workspace_ref, project_registry.inner()).await?;
    let asset_db = scope.asset_db();
    let guard = asset_db
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let graph = match guard.as_ref() {
        Some(g) => g,
        None => return Ok(vec![]),
    };

    let Some(q) = build_asset_name_query(query.trim()) else {
        return Ok(vec![]);
    };

    let fields = vec![
        "p".to_string(),
        "n".to_string(),
        "tp".to_string(),
        "guid".to_string(),
        "fileID".to_string(),
    ];
    let result = graph.search_assets(&q, &fields, 30, 0)?;

    Ok(result
        .rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "name": row.n.unwrap_or_default(),
                "path": row.p.unwrap_or_default(),
                "type": row.tp.unwrap_or_default(),
                "guid": row.guid.unwrap_or_default(),
                "fileID": row.file_id,
            })
        })
        .collect())
}

fn edges_to_json(
    edges: &[crate::asset_db::types::RefEdge],
    graph: &AssetDb,
) -> Vec<serde_json::Value> {
    edges
        .iter()
        .map(|e| {
            let src_path = graph
                .resolve_path_by_guid(&e.src_guid)
                .ok()
                .flatten()
                .unwrap_or_default();
            let dst_path = graph
                .resolve_path_by_guid(&e.dst_guid)
                .ok()
                .flatten()
                .unwrap_or_default();
            serde_json::json!({
                "src_guid": guid_to_hex(&e.src_guid),
                "src_file_id": e.src_file_id,
                "dst_guid": guid_to_hex(&e.dst_guid),
                "src_path": src_path,
                "dst_path": dst_path,
                "dst_file_id": e.dst_file_id,
                "class_id_hint": e.class_id_hint,
                "field_hint": e.field_hint,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_scan_waits_for_previous_runtime_task_to_finish() {
        let state = RefGraphScanTaskState::new();
        let registration_a = state.register_scoped("checkout-a", "F:/project-a".to_string(), 3);

        assert!(!registration_a.cancel_token().load(Ordering::Relaxed));

        let replacement_state = state.clone();
        let replacement_thread = std::thread::spawn(move || {
            replacement_state.register_scoped("checkout-a", "F:/project-a".to_string(), 4)
        });
        let started_at = std::time::Instant::now();
        while !registration_a.cancel_token().load(Ordering::Relaxed) {
            assert!(started_at.elapsed() < Duration::from_secs(1));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(registration_a.cancel_token().load(Ordering::Relaxed));
        drop(registration_a);
        let replacement_a = replacement_thread.join().expect("replacement registration");
        assert!(!replacement_a.cancel_token().load(Ordering::Relaxed));
    }

    #[test]
    fn scan_task_state_cancel_waits_until_registration_finishes() {
        let state = RefGraphScanTaskState::new();
        let registration = state.register_scoped("legacy", "F:/project-a".to_string(), 7);
        let cancel_token = registration.cancel_token();
        let waiter_state = state.clone();

        let waiter = std::thread::spawn(move || {
            waiter_state.cancel_current_and_wait_for("test", Duration::from_secs(2))
        });

        let started_at = std::time::Instant::now();
        while !cancel_token.load(Ordering::Relaxed) {
            assert!(started_at.elapsed() < Duration::from_secs(1));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.has_active_task());

        drop(registration);

        assert!(waiter.join().expect("waiter should not panic"));
        assert!(!state.has_active_task());
    }
}
