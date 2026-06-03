use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::asset_db::types::{guid_to_hex, parse_guid_hex, ScanPhase, ScanStats};
use crate::asset_db::{AssetDb, AssetDbState};
use crate::commands::asset::{
    write_persisted_last_scan_info, LastScanInfo, LastScanInfoState, ScanPhaseState,
};
use crate::error::AppError;
use crate::workspace::Workspace;
use crate::AssetDbWatcherHandle;

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

    fn register(&self, cwd: String, workspace_generation: u64) -> RefGraphScanRegistration {
        let task = RefGraphScanTask {
            cwd,
            workspace_generation,
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new((Mutex::new(false), Condvar::new())),
        };

        if let Ok(mut guard) = self.inner.lock() {
            if let Some(previous) = guard.replace(task.clone()) {
                previous.cancel.store(true, Ordering::Relaxed);
                eprintln!(
                    "[AssetDb] replaced active scan for {} generation {}; cancellation requested",
                    previous.cwd, previous.workspace_generation
                );
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

struct RefGraphScanContext {
    app_handle: AppHandle,
    workspace: Arc<Workspace>,
    cwd: String,
    workspace_generation: u64,
    ref_graph_state: Arc<Mutex<Option<AssetDb>>>,
    watcher_handle: AssetDbWatcherHandle,
    last_scan_info: LastScanInfoState,
    scan_phase_state: ScanPhaseState,
    watcher_tuning: Arc<crate::asset_db::watcher::WatcherTuning>,
    cancel_token: Arc<AtomicBool>,
}

struct RefGraphScanBegin {
    cwd: String,
    workspace_generation: u64,
    project_root: PathBuf,
}

enum RefGraphScanBeginResult {
    Started(RefGraphScanBegin),
    AlreadyRunning,
    Stale,
}

enum RefGraphScanJobOutcome {
    Completed(ScanStats),
    Stale,
}

impl RefGraphScanContext {
    fn is_current(&self) -> bool {
        self.workspace.generation() == self.workspace_generation
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_token.load(Ordering::Relaxed)
    }

    fn clear_scan_phase_if_current(&self) {
        if self.is_current() {
            self.scan_phase_state.clear();
        }
    }

    fn generation_lock_failed(error: String) -> AppError {
        AppError::new("workspace.generation_lock_failed", error)
    }
}

fn scan_already_running_error() -> AppError {
    AppError::new(
        "ref_graph.scan_already_running",
        "Asset database scan is already running.",
    )
    .retryable(true)
}

fn validate_scan_workspace(cwd: &str) -> Result<PathBuf, AppError> {
    let project_root = Path::new(cwd);
    if !project_root.join("Assets").is_dir() {
        return Err(AppError::new(
            "ref_graph.not_unity_project",
            "Not a Unity project (Assets/ not found)",
        ));
    }
    Ok(project_root.to_path_buf())
}

fn emit_scan_phase_if_current(
    workspace: &Arc<Workspace>,
    workspace_generation: u64,
    app_handle: &AppHandle,
    scan_phase_state: &ScanPhaseState,
    phase: ScanPhase,
) -> bool {
    let generation_guard = match workspace.lock_generation() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("[AssetDb] warning: failed to lock workspace generation: {error}");
            return false;
        }
    };

    if !generation_guard.is_current(workspace_generation) {
        return false;
    }

    let _ = app_handle.emit("ref-graph-scan", &phase);
    scan_phase_state.set(Some(phase));
    true
}

fn emit_scan_error(context: &RefGraphScanContext, error: &AppError) {
    let phase = ScanPhase::Error {
        error: error.clone(),
    };
    let _ = emit_scan_phase_if_current(
        &context.workspace,
        context.workspace_generation,
        &context.app_handle,
        &context.scan_phase_state,
        phase,
    );
}

fn emit_scan_done(context: &RefGraphScanContext, stats: ScanStats) {
    let phase = ScanPhase::Done { stats };
    let _ = context.app_handle.emit("ref-graph-scan", &phase);
}

async fn scan_workspace_snapshot(workspace: &Arc<Workspace>) -> (String, u64) {
    let path = workspace.path.read().await;
    (path.clone(), workspace.generation())
}

fn begin_ref_graph_scan_from_snapshot(
    workspace: &Arc<Workspace>,
    scan_phase_state: &ScanPhaseState,
    cwd: String,
    workspace_generation: u64,
) -> Result<RefGraphScanBeginResult, AppError> {
    let generation_guard = workspace
        .lock_generation()
        .map_err(RefGraphScanContext::generation_lock_failed)?;
    if !generation_guard.is_current(workspace_generation) {
        return Ok(RefGraphScanBeginResult::Stale);
    }

    let project_root = validate_scan_workspace(&cwd)?;
    if !scan_phase_state.try_begin_scan()? {
        return Ok(RefGraphScanBeginResult::AlreadyRunning);
    }

    Ok(RefGraphScanBeginResult::Started(RefGraphScanBegin {
        cwd,
        workspace_generation,
        project_root,
    }))
}

async fn begin_ref_graph_scan(
    workspace: &Arc<Workspace>,
    scan_phase_state: &ScanPhaseState,
) -> Result<RefGraphScanBeginResult, AppError> {
    let (cwd, workspace_generation) = scan_workspace_snapshot(workspace).await;
    begin_ref_graph_scan_from_snapshot(workspace, scan_phase_state, cwd, workspace_generation)
}

async fn run_ref_graph_scan_job(
    context: RefGraphScanContext,
    project_root: PathBuf,
) -> Result<RefGraphScanJobOutcome, AppError> {
    let scan_started = std::time::Instant::now();

    if context.is_cancelled() {
        context.clear_scan_phase_if_current();
        return Ok(RefGraphScanJobOutcome::Stale);
    }

    if !context.is_current() {
        return Ok(RefGraphScanJobOutcome::Stale);
    }

    let old_watcher = {
        let generation_guard = context
            .workspace
            .lock_generation()
            .map_err(RefGraphScanContext::generation_lock_failed)?;
        if !generation_guard.is_current(context.workspace_generation) {
            return Ok(RefGraphScanJobOutcome::Stale);
        }

        let mut wh = match context.watcher_handle.lock() {
            Ok(guard) => guard,
            Err(e) => {
                let error = AppError::new(
                    "ref_graph.watcher_lock_failed",
                    format!("Lock error: {}", e),
                );
                drop(generation_guard);
                emit_scan_error(&context, &error);
                return Err(error);
            }
        };
        let old_watcher = wh.take();

        // Drop the old AssetDb (and thus its SQLite Connection) BEFORE we let
        // the rebuild path inside `db::open_db` try to remove the on-disk DB.
        // SQLite holds an exclusive file lock on Windows for as long as a
        // Connection is alive; without this drop, schema-version-mismatch
        // rebuilds would fail with a sharing violation.
        let mut g = match context.ref_graph_state.lock() {
            Ok(guard) => guard,
            Err(e) => {
                let error = AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e));
                drop(generation_guard);
                emit_scan_error(&context, &error);
                return Err(error);
            }
        };
        *g = None;
        old_watcher
    };

    if let Some(old) = old_watcher {
        old.stop_and_join();
        eprintln!("[AssetDb] stopped previous watcher before rescan");
    }

    let root = project_root.clone();
    let handle = context.app_handle.clone();
    let scan_phase_state = context.scan_phase_state.clone();
    let workspace = context.workspace.clone();
    let workspace_generation = context.workspace_generation;
    let cancel_token = context.cancel_token.clone();
    let result = match tokio::task::spawn_blocking(move || {
        if cancel_token.load(Ordering::Relaxed) {
            return Err(REF_GRAPH_SCAN_CANCELLED_DETAIL.to_string());
        }
        let mut graph = AssetDb::open(&root)?;
        let cancel_for_progress = cancel_token.clone();
        let stats = graph.full_scan_with_cancel(
            |phase| {
                if cancel_for_progress.load(Ordering::Relaxed) {
                    return;
                }
                let _ = emit_scan_phase_if_current(
                    &workspace,
                    workspace_generation,
                    &handle,
                    &scan_phase_state,
                    phase.clone(),
                );
            },
            &cancel_token,
        )?;
        if cancel_token.load(Ordering::Relaxed) {
            return Err(REF_GRAPH_SCAN_CANCELLED_DETAIL.to_string());
        }
        let handle_for_reconcile = handle.clone();
        let scan_phase_state_for_reconcile = scan_phase_state.clone();
        let workspace_for_reconcile = workspace.clone();
        let cancel_for_reconcile_progress = cancel_token.clone();
        let (graph, reconcile_stats) =
            crate::asset_db::watcher::reconcile_loaded_db_with_cancel_and_progress(
                &root,
                graph,
                &cancel_token,
                |progress| {
                    if cancel_for_reconcile_progress.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = emit_scan_phase_if_current(
                        &workspace_for_reconcile,
                        workspace_generation,
                        &handle_for_reconcile,
                        &scan_phase_state_for_reconcile,
                        progress.to_scan_phase(),
                    );
                },
            )?;
        eprintln!(
            "[AssetDb] post-scan reconcile complete: queued={}, processed={}, failed={}",
            reconcile_stats.queued, reconcile_stats.processed, reconcile_stats.failed
        );
        Ok::<(AssetDb, ScanStats), String>((graph, stats))
    })
    .await
    {
        Ok(result) => result,
        Err(e) => {
            let error = AppError::new(
                "ref_graph.scan_task_join_failed",
                format!("Task join error: {}", e),
            )
            .retryable(true);
            emit_scan_error(&context, &error);
            return Err(error);
        }
    };

    if context.is_cancelled() {
        context.clear_scan_phase_if_current();
        return Ok(RefGraphScanJobOutcome::Stale);
    }

    if !context.is_current() {
        return Ok(RefGraphScanJobOutcome::Stale);
    }

    match result {
        Ok((graph, scan_stats)) => {
            let generation_guard = context
                .workspace
                .lock_generation()
                .map_err(RefGraphScanContext::generation_lock_failed)?;
            if !generation_guard.is_current(context.workspace_generation) {
                return Ok(RefGraphScanJobOutcome::Stale);
            }

            let finished_at_unix_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let scan_info = LastScanInfo {
                finished_at_unix_ms,
                duration_ms: scan_stats
                    .elapsed_ms
                    .max(scan_started.elapsed().as_millis() as u64),
                stats: scan_stats.clone(),
            };

            {
                let mut guard = match context.ref_graph_state.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        let error =
                            AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e));
                        drop(generation_guard);
                        emit_scan_error(&context, &error);
                        return Err(error);
                    }
                };
                *guard = Some(graph);
            }

            context.last_scan_info.set(scan_info.clone());
            if let Err(err) = write_persisted_last_scan_info(Path::new(&context.cwd), &scan_info) {
                eprintln!(
                    "[AssetDb] warning: failed to persist last successful scan info: {}",
                    err
                );
            }
            context.scan_phase_state.clear();

            let graph_arc = context.ref_graph_state.clone();
            let watcher_root = PathBuf::from(&context.cwd);
            match crate::asset_db::watcher::AssetDbWatcher::start(
                watcher_root,
                graph_arc,
                context.watcher_tuning.clone(),
            ) {
                Ok(w) => {
                    let mut wh = match context.watcher_handle.lock() {
                        Ok(guard) => guard,
                        Err(e) => {
                            let error = AppError::new(
                                "ref_graph.watcher_lock_failed",
                                format!("Lock error: {}", e),
                            );
                            drop(generation_guard);
                            emit_scan_error(&context, &error);
                            return Err(error);
                        }
                    };
                    *wh = Some(w);
                    eprintln!("[AssetDb] incremental watcher started");
                }
                Err(e) => {
                    eprintln!("[AssetDb] warning: failed to start watcher: {}", e);
                }
            }

            emit_scan_done(&context, scan_stats.clone());
            Ok(RefGraphScanJobOutcome::Completed(scan_stats))
        }
        Err(e) => {
            if context.is_cancelled() {
                context.clear_scan_phase_if_current();
                return Ok(RefGraphScanJobOutcome::Stale);
            }
            if !context.is_current() {
                return Ok(RefGraphScanJobOutcome::Stale);
            }
            eprintln!("[AssetDb] scan failed: {}", e);
            let scan_error = AppError::new("ref_graph.scan_failed", &e).retryable(true);
            emit_scan_error(&context, &scan_error);
            Err(scan_error)
        }
    }
}

fn stale_scan_error() -> AppError {
    AppError::new(
        "ref_graph.scan_stale",
        "Asset database scan was superseded by a workspace change.",
    )
    .retryable(true)
}

#[tauri::command]
pub async fn ref_graph_status(
    ref_graph_state: State<'_, AssetDbState>,
    last_scan_info: State<'_, LastScanInfoState>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Option<ScanStats>, AppError> {
    if workspace.path.read().await.trim().is_empty() {
        return Ok(None);
    }
    if let Some(info) = last_scan_info.snapshot() {
        return Ok(Some(info.stats));
    }
    let guard = ref_graph_state
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
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    scan_task_state: State<'_, RefGraphScanTaskState>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<ScanStats, AppError> {
    let workspace = workspace.inner().clone();
    let scan_phase_state = scan_phase_state.inner().clone();

    let begin = match begin_ref_graph_scan(&workspace, &scan_phase_state).await? {
        RefGraphScanBeginResult::Started(begin) => begin,
        RefGraphScanBeginResult::AlreadyRunning => return Err(scan_already_running_error()),
        RefGraphScanBeginResult::Stale => return Err(stale_scan_error()),
    };

    let scan_registration = scan_task_state.register(begin.cwd.clone(), begin.workspace_generation);
    let cancel_token = scan_registration.cancel_token();

    run_ref_graph_scan_job(
        RefGraphScanContext {
            app_handle,
            workspace,
            cwd: begin.cwd,
            workspace_generation: begin.workspace_generation,
            ref_graph_state: ref_graph_state.0.clone(),
            watcher_handle: watcher_handle.inner().clone(),
            last_scan_info: last_scan_info.inner().clone(),
            scan_phase_state,
            watcher_tuning: watcher_tuning.0.clone(),
            cancel_token,
        },
        begin.project_root,
    )
    .await
    .and_then(|outcome| match outcome {
        RefGraphScanJobOutcome::Completed(stats) => Ok(stats),
        RefGraphScanJobOutcome::Stale => Err(stale_scan_error()),
    })
}

#[tauri::command]
pub async fn ref_graph_scan_start(
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    scan_task_state: State<'_, RefGraphScanTaskState>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<RefGraphScanStartResult, AppError> {
    let workspace = workspace.inner().clone();
    let scan_phase_state = scan_phase_state.inner().clone();

    let begin = match begin_ref_graph_scan(&workspace, &scan_phase_state).await? {
        RefGraphScanBeginResult::Started(begin) => begin,
        RefGraphScanBeginResult::AlreadyRunning => {
            return Ok(RefGraphScanStartResult {
                started: false,
                already_running: true,
            });
        }
        RefGraphScanBeginResult::Stale => return Err(stale_scan_error()),
    };

    let scan_registration = scan_task_state.register(begin.cwd.clone(), begin.workspace_generation);
    let cancel_token = scan_registration.cancel_token();

    let context = RefGraphScanContext {
        app_handle,
        workspace,
        cwd: begin.cwd,
        workspace_generation: begin.workspace_generation,
        ref_graph_state: ref_graph_state.0.clone(),
        watcher_handle: watcher_handle.inner().clone(),
        last_scan_info: last_scan_info.inner().clone(),
        scan_phase_state,
        watcher_tuning: watcher_tuning.0.clone(),
        cancel_token,
    };

    tauri::async_runtime::spawn(async move {
        let _scan_registration = scan_registration;
        match run_ref_graph_scan_job(context, begin.project_root).await {
            Ok(RefGraphScanJobOutcome::Completed(_)) => {}
            Ok(RefGraphScanJobOutcome::Stale) => {
                eprintln!("[AssetDb] background scan discarded after workspace switch");
            }
            Err(err) => {
                eprintln!("[AssetDb] background scan failed: {}", err);
            }
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Option<String>, AppError> {
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Option<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<String>, AppError> {
    let guid = parse_guid_hex(&guid_hex).ok_or("Invalid GUID hex")?;
    let guard = ref_graph_state
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
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let guard = ref_graph_state
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

    fn temp_unity_workspace() -> (tempfile::TempDir, String) {
        let temp = tempfile::tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("create Assets dir");
        let cwd = temp.path().to_string_lossy().to_string();
        (temp, cwd)
    }

    #[test]
    fn begin_scan_rejects_stale_generation_without_setting_phase() {
        let (_temp, cwd) = temp_unity_workspace();
        let workspace = Arc::new(Workspace::new(cwd.clone(), Some("workspace-a".to_string())));
        let scan_phase_state = ScanPhaseState::new();
        let stale_generation = workspace.generation();

        workspace.bump_generation();

        let result = begin_ref_graph_scan_from_snapshot(
            &workspace,
            &scan_phase_state,
            cwd,
            stale_generation,
        )
        .expect("begin scan should not fail");

        assert!(matches!(result, RefGraphScanBeginResult::Stale));
        assert!(scan_phase_state.snapshot().is_none());
    }

    #[test]
    fn begin_scan_sets_dir_scan_for_current_generation() {
        let (_temp, cwd) = temp_unity_workspace();
        let workspace = Arc::new(Workspace::new(cwd.clone(), Some("workspace-a".to_string())));
        let scan_phase_state = ScanPhaseState::new();

        let result = begin_ref_graph_scan_from_snapshot(
            &workspace,
            &scan_phase_state,
            cwd.clone(),
            workspace.generation(),
        )
        .expect("begin scan should not fail");

        match result {
            RefGraphScanBeginResult::Started(begin) => {
                assert_eq!(begin.cwd, cwd);
                assert_eq!(begin.workspace_generation, workspace.generation());
            }
            _ => panic!("expected started scan"),
        }
        assert!(matches!(
            scan_phase_state.snapshot(),
            Some(ScanPhase::DirScan)
        ));
    }

    #[test]
    fn scan_task_state_cancel_waits_until_registration_finishes() {
        let state = RefGraphScanTaskState::new();
        let registration = state.register("F:/project-a".to_string(), 7);
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
