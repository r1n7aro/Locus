use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::asset_db::{AssetDb, AssetDbState, LoadExistingAssetDb};
use crate::commands::asset::{
    delete_persisted_last_scan_info, read_persisted_last_scan_info, AssetDbReconcileTaskState,
    LastScanInfoState, ScanPhaseState, WorkspacePreviewCache,
};
use crate::error::AppError;
use crate::keychain;
use crate::unity_bridge::UnityMonitorHandle;
use crate::workspace::Workspace;
use crate::AssetDbWatcherHandle;
use crate::KnowledgeFsWatcherHandle;

const ENDPOINT_TEST_HTML_RESPONSE_CODE: &str = "endpoint_test.html_response";

/// Returns a stable app config directory inside the OS config root.
/// On Windows this resolves under `%APPDATA%\\locus`, which keeps model config
/// under the app-data tree while staying outside Tauri's bundle-specific
/// `app_data_dir` that may be cleared during reinstall.
pub(crate) fn persistent_config_dir() -> Result<std::path::PathBuf, String> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Failed to get config directory".to_string())?;
    let dir = config_dir.join("locus");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create persistent config dir: {}", e))?;
    Ok(dir)
}

fn app_temp_dir_override() -> &'static Mutex<Option<std::path::PathBuf>> {
    static OVERRIDE: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn set_app_temp_dir_override(
    dir: std::path::PathBuf,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
    let dir = dunce::canonicalize(&dir).unwrap_or(dir);
    let mut guard = app_temp_dir_override()
        .lock()
        .map_err(|e| format!("Failed to lock app temp directory override: {}", e))?;
    *guard = Some(dir.clone());
    Ok(dir)
}

pub(crate) fn app_temp_dir() -> Result<std::path::PathBuf, String> {
    if let Some(dir) = app_temp_dir_override()
        .lock()
        .map_err(|e| format!("Failed to lock app temp directory override: {}", e))?
        .clone()
    {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
        return Ok(dir);
    }

    let dir = persistent_config_dir()?.join("temp");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
    Ok(dir)
}

fn read_nonempty_string(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(crate) fn custom_endpoints_path(_app_handle: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("custom_endpoints.json"))
}

#[tauri::command]
pub async fn get_working_dir(workspace: State<'_, Arc<Workspace>>) -> Result<String, AppError> {
    let dir = workspace.path.read().await.clone();
    Ok(dir)
}

struct WorkspaceSwitchTimer {
    target: String,
    started_at: Instant,
    lap_started_at: Instant,
}

impl WorkspaceSwitchTimer {
    fn new(target: &str, started_at: Instant) -> Self {
        Self {
            target: target.to_string(),
            started_at,
            lap_started_at: started_at,
        }
    }

    fn mark(&mut self, phase: &str) {
        self.mark_detail(phase, "");
    }

    fn mark_detail(&mut self, phase: &str, detail: impl AsRef<str>) {
        let now = Instant::now();
        let detail = detail.as_ref();
        tracing::info!(
            log_module = "WorkspaceSwitch",
            "phase={} total_ms={} delta_ms={} target={}{}",
            phase,
            now.duration_since(self.started_at).as_millis(),
            now.duration_since(self.lap_started_at).as_millis(),
            self.target,
            detail
        );
        self.lap_started_at = now;
    }
}

fn emit_asset_phase_if_current(
    workspace: &Arc<Workspace>,
    workspace_generation: u64,
    app_handle: &AppHandle,
    scan_phase_state: &ScanPhaseState,
    phase: crate::asset_db::types::ScanPhase,
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

fn emit_asset_reconcile_done_if_current(
    workspace: &Arc<Workspace>,
    workspace_generation: u64,
    app_handle: &AppHandle,
    scan_phase_state: &ScanPhaseState,
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

    let phase = crate::asset_db::types::ScanPhase::ReconcileDone;
    let _ = app_handle.emit("ref-graph-scan", &phase);
    if scan_phase_state
        .snapshot()
        .as_ref()
        .map(|phase| matches!(phase, crate::asset_db::types::ScanPhase::Reconcile { .. }))
        .unwrap_or(false)
    {
        scan_phase_state.clear();
    }
    true
}

fn emit_asset_reconcile_error_if_current(
    workspace: &Arc<Workspace>,
    workspace_generation: u64,
    app_handle: &AppHandle,
    scan_phase_state: &ScanPhaseState,
    error: AppError,
) -> bool {
    emit_asset_phase_if_current(
        workspace,
        workspace_generation,
        app_handle,
        scan_phase_state,
        crate::asset_db::types::ScanPhase::Error { error },
    )
}

fn spawn_background_asset_hash_reconcile(
    app_handle: AppHandle,
    workspace: Arc<Workspace>,
    workspace_generation: u64,
    project_root: std::path::PathBuf,
    graph_state: Arc<Mutex<Option<AssetDb>>>,
    scan_phase_state: ScanPhaseState,
    reconcile_task_state: AssetDbReconcileTaskState,
) {
    let cwd = project_root.display().to_string();
    let registration = reconcile_task_state.register(cwd.clone(), workspace_generation);
    let cancel_token = registration.cancel_token();
    let phase = crate::asset_db::types::ScanPhase::reconcile_started(true);

    if !emit_asset_phase_if_current(
        &workspace,
        workspace_generation,
        &app_handle,
        &scan_phase_state,
        phase,
    ) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let _registration = registration;
        let started_at = Instant::now();
        let root_for_task = project_root.clone();
        let graph_for_task = graph_state.clone();
        let cancel_for_task = cancel_token.clone();
        let app_handle_for_progress = app_handle.clone();
        let workspace_for_progress = workspace.clone();
        let scan_phase_state_for_progress = scan_phase_state.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            crate::asset_db::watcher::reconcile_graph_state_with_cancel_and_progress(
                &root_for_task,
                graph_for_task,
                &cancel_for_task,
                true,
                |progress| {
                    let _ = emit_asset_phase_if_current(
                        &workspace_for_progress,
                        workspace_generation,
                        &app_handle_for_progress,
                        &scan_phase_state_for_progress,
                        progress.to_scan_phase(),
                    );
                },
            )
        })
        .await;

        if cancel_token.load(Ordering::Relaxed) || workspace.generation() != workspace_generation {
            eprintln!(
                "[AssetDb] background hash reconcile discarded for {} generation {}",
                cwd, workspace_generation
            );
            return;
        }

        match result {
            Ok(Ok(stats)) => {
                tracing::info!(
                    log_module = "AssetDb",
                    "background hash reconcile complete: workspace={} queued={} processed={} failed={} elapsed_ms={}",
                    cwd,
                    stats.queued,
                    stats.processed,
                    stats.failed,
                    started_at.elapsed().as_millis()
                );
                emit_asset_reconcile_done_if_current(
                    &workspace,
                    workspace_generation,
                    &app_handle,
                    &scan_phase_state,
                );
            }
            Ok(Err(err)) => {
                eprintln!(
                    "[AssetDb] background hash reconcile failed: workspace={} elapsed_ms={} error={}",
                    cwd,
                    started_at.elapsed().as_millis(),
                    err
                );
                let error = AppError::new(
                    "ref_graph.rescan_required.reconcile_failed",
                    "Persisted asset database could not be verified. Run a rescan to rebuild it.",
                )
                .detail(err)
                .retryable(true);
                emit_asset_reconcile_error_if_current(
                    &workspace,
                    workspace_generation,
                    &app_handle,
                    &scan_phase_state,
                    error,
                );
            }
            Err(err) => {
                eprintln!(
                    "[AssetDb] background hash reconcile task join failed: workspace={} elapsed_ms={} error={}",
                    cwd,
                    started_at.elapsed().as_millis(),
                    err
                );
                let error = AppError::new(
                    "ref_graph.reconcile_task_join_failed",
                    format!("Task join error: {}", err),
                )
                .retryable(true);
                emit_asset_reconcile_error_if_current(
                    &workspace,
                    workspace_generation,
                    &app_handle,
                    &scan_phase_state,
                    error,
                );
            }
        }
    });
}

#[tauri::command]
pub async fn set_working_dir(
    path: String,
    workspace: State<'_, Arc<Workspace>>,
    unity_monitor: State<'_, UnityMonitorHandle>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    knowledge_watcher_handle: State<'_, KnowledgeFsWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    scan_task_state: State<'_, super::RefGraphScanTaskState>,
    reconcile_task_state: State<'_, AssetDbReconcileTaskState>,
    preview_cache: State<'_, WorkspacePreviewCache>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
    knowledge_index_state: State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    registry: State<'_, crate::AgentDefRegistryState>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let switch_started_at = Instant::now();
    let path = path.trim().to_string();
    let mut switch_timer = WorkspaceSwitchTimer::new(&path, switch_started_at);
    switch_timer.mark("request_received");
    if path.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let p = std::path::Path::new(&path);
    if !p.is_dir() {
        return Err(format!("Directory not found: {}", path).into());
    }

    if !p.join("Assets").is_dir() {
        return Err(
            "Selected directory is not a Unity project (Assets/ folder not found)"
                .to_string()
                .into(),
        );
    }
    switch_timer.mark("target_validated");

    let canonical = dunce::canonicalize(p)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.clone());
    switch_timer.mark_detail("canonicalized", format!(" canonical={}", canonical));

    let ws_id = crate::workspace::load_or_create_workspace(&canonical)?;
    switch_timer.mark_detail("workspace_id_ready", format!(" workspace_id={}", ws_id));

    // Decide whether the workspace is actually changing. We compare the
    // canonical form against the currently-stored cwd. If unchanged, we keep
    // the previous `LastScanInfo` so the asset page status row stays accurate;
    // a re-`set_working_dir` of the same project should not erase its history.
    let prev_cwd = workspace.path.read().await.clone();
    let is_real_switch = prev_cwd != canonical;
    if is_real_switch {
        reconcile_task_state.cancel_current("workspace switch");
        let cancelled = scan_task_state.cancel_current_and_wait("workspace switch");
        switch_timer.mark_detail(
            "active_scan_cancel_checked",
            format!(" cancelled={}", cancelled),
        );
        if !cancelled {
            eprintln!(
                "[Locus] warning: asset DB scan cancellation did not finish before workspace switch"
            );
        }
    } else {
        switch_timer.mark("same_workspace_checked");
    }

    let old_ref_graph_watcher = {
        let mut dir = workspace.path.write().await;
        let old_ref_graph_watcher = if is_real_switch {
            let generation_guard = workspace
                .lock_generation()
                .map_err(|e| AppError::new("workspace.generation_lock_failed", e))?;
            generation_guard.bump_generation();
            last_scan_info.clear();
            scan_phase_state.clear();
            // Drop any preview sessions parsed against the previous workspace —
            // they hold owned YAML docs that would otherwise be paired with the
            // new project's AssetDb in `preview_workspace_asset_target`.
            preview_cache.clear();
            dir_entries_cache.clear();
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = None;
            watcher_handle
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?
                .take()
        } else {
            None
        };
        *dir = canonical.clone();
        old_ref_graph_watcher
    };
    switch_timer.mark_detail(
        "workspace_state_committed",
        format!(" is_real_switch={}", is_real_switch),
    );
    if let Some(old) = old_ref_graph_watcher {
        old.stop_and_join();
        switch_timer.mark("old_ref_graph_watcher_stopped");
        eprintln!("[Locus] stopped ref_graph watcher (working dir changed)");
    }
    {
        let mut wid = workspace.workspace_id.write().await;
        *wid = Some(ws_id.clone());
    }
    switch_timer.mark("workspace_id_state_committed");
    super::plugin::reload_agent_registry(&registry, &app_agent_dir, &canonical).await;
    switch_timer.mark("agent_registry_reloaded");

    if is_real_switch {
        super::reset_unity_embed_control_window(&app_handle);
        super::refresh_unity_embed_control_server(app_handle.clone());
        switch_timer.mark("unity_embed_control_refreshed");
    }

    if let Ok(data_dir) = super::resolve_runtime_storage_dir(&app_handle) {
        let file: std::path::PathBuf = data_dir.join("working_dir.txt");
        let _ = std::fs::write(&file, &canonical);
        save_recent_dir(&data_dir, &canonical);
    }
    switch_timer.mark("working_dir_persisted");

    if is_real_switch {
        let library_dir = crate::knowledge_index::library_dir_for_working_dir(&canonical);
        let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;
        switch_timer.mark("knowledge_index_rebuild_start");
        knowledge_index_state
            .rebuild(&library_dir, &model_storage_dir)
            .await?;
        switch_timer.mark("knowledge_index_rebuild_done");
        let knowledge_state = knowledge_index_state.inner().clone();
        let working_dir_for_index = canonical.clone();
        let app_handle_for_index = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            let app_knowledge_dir: tauri::State<'_, crate::commands::AppKnowledgeDir> =
                app_handle_for_index.state();
            if let Err(e) = crate::knowledge_index::maybe_auto_activate_embedding_runtime(
                knowledge_state.clone(),
                &working_dir_for_index,
                app_knowledge_dir.0.as_ref().as_ref(),
            )
            .await
            {
                eprintln!("[Locus] knowledge embedding auto-activate error: {}", e);
            }
            if let Err(e) = crate::knowledge_index::reconcile_workspace(
                &working_dir_for_index,
                app_knowledge_dir.0.as_ref().as_ref(),
                knowledge_state,
            )
            .await
            {
                eprintln!("[Locus] knowledge reconcile error: {}", e);
            }
        });
        switch_timer.mark("knowledge_reconcile_task_spawned");
    }

    if is_real_switch {
        if let Err(error) = crate::knowledge_store::ensure_knowledge_roots(&canonical) {
            eprintln!(
                "[Locus] warning: failed to prepare knowledge roots for new working dir: {}",
                error
            );
        }
        switch_timer.mark("knowledge_roots_ready");
        let mut knowledge_watcher = knowledge_watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = knowledge_watcher.take() {
            old.stop();
            switch_timer.mark("old_knowledge_watcher_stopped");
            eprintln!("[Locus] stopped knowledge watcher (working dir changed)");
        }
        switch_timer.mark("knowledge_watcher_start_begin");
        match crate::knowledge_watcher::KnowledgeFsWatcher::start(
            app_handle.clone(),
            canonical.clone(),
            app_knowledge_dir.0.as_ref().as_ref().cloned(),
            knowledge_index_state.inner().clone(),
        ) {
            Ok(watcher) => {
                *knowledge_watcher = Some(watcher);
                switch_timer.mark("knowledge_watcher_start_done");
                eprintln!("[Locus] knowledge watcher started for new working dir");
            }
            Err(error) => {
                switch_timer.mark_detail(
                    "knowledge_watcher_start_failed",
                    format!(" error={}", error),
                );
                eprintln!(
                    "[Locus] warning: failed to start knowledge watcher: {}",
                    error
                );
            }
        }
    }

    switch_timer.mark("asset_db_load_existing_start");
    match AssetDb::load_existing(std::path::Path::new(&canonical)) {
        LoadExistingAssetDb::Ready(graph) => {
            switch_timer.mark("asset_db_load_existing_ready");
            switch_timer.mark_detail("asset_db_reconcile_start", " verify_hashes=false");
            match crate::asset_db::watcher::reconcile_loaded_db_light(
                std::path::Path::new(&canonical),
                graph,
            ) {
                Ok((graph, stats)) => {
                    switch_timer.mark_detail(
                        "asset_db_reconcile_done",
                        format!(
                            " verify_hashes=false queued={} processed={} failed={}",
                            stats.queued, stats.processed, stats.failed
                        ),
                    );
                    tracing::info!(
                        log_module = "Locus",
                        "ref_graph DB light-reconciled for new working dir: queued={}, processed={}, failed={}",
                        stats.queued,
                        stats.processed,
                        stats.failed
                    );
                    let db_path = std::path::Path::new(&canonical)
                        .join("Library")
                        .join("Locus")
                        .join("locus.db");
                    eprintln!(
                        "[Locus] ref_graph DB loaded for new working dir: {}",
                        db_path.display()
                    );
                    *ref_graph_state
                        .0
                        .lock()
                        .map_err(|e| format!("Lock error: {}", e))? = Some(graph);
                    match read_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                        Ok(Some(info)) => last_scan_info.set(info),
                        Ok(None) => {
                            if is_real_switch {
                                last_scan_info.clear();
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "[Locus] warning: failed to load persisted asset scan info: {}",
                                err
                            );
                            if is_real_switch {
                                last_scan_info.clear();
                            }
                        }
                    }
                    switch_timer.mark("asset_db_state_ready");

                    let graph_arc = ref_graph_state.0.clone();
                    let watcher_root = std::path::PathBuf::from(&canonical);
                    switch_timer.mark("asset_db_watcher_start_begin");
                    match crate::asset_db::watcher::AssetDbWatcher::start(
                        watcher_root,
                        graph_arc,
                        watcher_tuning.0.clone(),
                    ) {
                        Ok(w) => {
                            *watcher_handle
                                .lock()
                                .map_err(|e| format!("Lock error: {}", e))? = Some(w);
                            switch_timer.mark("asset_db_watcher_start_done");
                            eprintln!("[Locus] ref_graph watcher started for new working dir");
                        }
                        Err(e) => {
                            switch_timer.mark_detail(
                                "asset_db_watcher_start_failed",
                                format!(" error={}", e),
                            );
                            eprintln!("[Locus] warning: failed to start ref_graph watcher: {}", e);
                        }
                    }
                    if is_real_switch {
                        let workspace_generation = workspace.generation();
                        spawn_background_asset_hash_reconcile(
                            app_handle.clone(),
                            workspace.inner().clone(),
                            workspace_generation,
                            std::path::PathBuf::from(&canonical),
                            ref_graph_state.0.clone(),
                            scan_phase_state.inner().clone(),
                            reconcile_task_state.inner().clone(),
                        );
                        switch_timer.mark("asset_db_background_hash_reconcile_spawned");
                    }
                }
                Err(err) => {
                    switch_timer
                        .mark_detail("asset_db_reconcile_failed", format!(" error={}", err));
                    eprintln!(
                        "[Locus] ref_graph DB reconcile failed for new working dir, rescan required: {}",
                        err
                    );
                    last_scan_info.clear();
                    if let Err(clear_err) =
                        delete_persisted_last_scan_info(std::path::Path::new(&canonical))
                    {
                        eprintln!(
                            "[Locus] warning: failed to clear stale asset scan info: {}",
                            clear_err
                        );
                    }
                    *ref_graph_state
                        .0
                        .lock()
                        .map_err(|e| format!("Lock error: {}", e))? = None;
                    scan_phase_state.set(Some(crate::asset_db::types::ScanPhase::Error {
                        error: crate::error::AppError::new(
                            "ref_graph.rescan_required.reconcile_failed",
                            "Persisted asset database could not be reconciled. Run a rescan to rebuild it.",
                        )
                        .detail(err)
                        .retryable(true),
                    }));
                }
            }
        }
        LoadExistingAssetDb::NeedsRescan(issue) => {
            switch_timer.mark_detail(
                "asset_db_load_existing_needs_rescan",
                format!(" reason={}", issue.message),
            );
            eprintln!(
                "[Locus] ref_graph DB invalidated for new working dir, rescan required: {}",
                issue.message
            );
            last_scan_info.clear();
            if let Err(err) = delete_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                eprintln!(
                    "[Locus] warning: failed to clear stale asset scan info: {}",
                    err
                );
            }
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = None;
            scan_phase_state.set(Some(crate::asset_db::types::ScanPhase::Error {
                error: issue.to_app_error(),
            }));
        }
        LoadExistingAssetDb::Missing => {
            switch_timer.mark("asset_db_load_existing_missing");
            eprintln!("[Locus] no ref_graph DB in new working dir, clearing state");
            last_scan_info.clear();
            if let Err(err) = delete_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                eprintln!(
                    "[Locus] warning: failed to clear stale asset scan info: {}",
                    err
                );
            }
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = None;
        }
    }

    if crate::unity_bridge::is_unity_project(&canonical) {
        switch_timer.mark("unity_monitor_start_begin");
        crate::unity_bridge::start_unity_monitor(
            app_handle.clone(),
            canonical.clone(),
            &unity_monitor,
        )
        .await;
        switch_timer.mark("unity_monitor_start_done");
        switch_timer.mark("plugin_status_emit_begin");
        crate::unity_bridge::emit_plugin_status(&app_handle, &canonical);
        switch_timer.mark("plugin_status_emit_done");
    } else {
        crate::unity_bridge::stop_unity_monitor(&unity_monitor).await;
        let _ = app_handle.emit("unity-connection-status", false);
        switch_timer.mark("unity_monitor_stopped");
    }

    switch_timer.mark_detail(
        "finished",
        format!(
            " canonical={} workspace_id={} is_real_switch={}",
            canonical, ws_id, is_real_switch
        ),
    );
    eprintln!(
        "[Locus] working_dir changed to: {}, workspace_id: {}",
        canonical, ws_id
    );
    Ok(canonical)
}

const MAX_RECENT_DIRS: usize = 8;

pub fn save_recent_dir_pub(data_dir: &std::path::Path, dir: &str) {
    save_recent_dir(data_dir, dir);
}

fn recent_dirs_file(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("recent_dirs.json")
}

fn read_recent_dirs(data_dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(recent_dirs_file(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_recent_dirs(data_dir: &std::path::Path, dirs: &[String]) -> Result<(), AppError> {
    let file = recent_dirs_file(data_dir);
    let text = serde_json::to_string(dirs)
        .map_err(|e| AppError::new("workspace.recent_dirs_serialize_failed", e.to_string()))?;
    std::fs::write(&file, text).map_err(|e| {
        AppError::new(
            "workspace.recent_dirs_write_failed",
            format!("Failed to save recent directories: {}", e),
        )
    })
}

fn existing_recent_dirs(dirs: Vec<String>) -> Vec<String> {
    dirs.into_iter()
        .filter(|d| std::path::Path::new(d).is_dir())
        .collect()
}

fn save_recent_dir(data_dir: &std::path::Path, dir: &str) {
    let mut dirs = read_recent_dirs(data_dir);

    dirs.retain(|d| d != dir);
    dirs.insert(0, dir.to_string());
    dirs.truncate(MAX_RECENT_DIRS);

    let _ = write_recent_dirs(data_dir, &dirs);
}

#[tauri::command]
pub async fn list_recent_dirs(app_handle: AppHandle) -> Result<Vec<String>, AppError> {
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(existing_recent_dirs(read_recent_dirs(&data_dir)))
}

#[tauri::command]
pub async fn remove_recent_dir(
    path: String,
    app_handle: AppHandle,
) -> Result<Vec<String>, AppError> {
    let target = path.trim();
    if target.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let mut dirs = read_recent_dirs(&data_dir);
    dirs.retain(|d| d != target);
    write_recent_dirs(&data_dir, &dirs)?;
    Ok(existing_recent_dirs(dirs))
}

#[tauri::command]
pub async fn open_dir_in_file_explorer(path: String) -> Result<(), AppError> {
    let target = path.trim();
    if target.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let path = std::path::Path::new(target);
    if !path.is_dir() {
        return Err(format!("Directory not found: {}", target).into());
    }

    let canonical =
        dunce::canonicalize(path).map_err(|e| format!("Failed to resolve path: {}", e))?;
    crate::commands::knowledge::reveal_path_native(&canonical).map_err(Into::into)
}

#[tauri::command]
pub async fn get_last_model(_app_handle: AppHandle) -> Result<String, AppError> {
    let primary_path = persistent_config_dir()?.join("last_model.txt");
    if let Some(val) = read_nonempty_string(&primary_path) {
        return Ok(val);
    }
    Ok(String::new())
}

#[tauri::command]
pub async fn save_last_model(model_id: String, _app_handle: AppHandle) -> Result<(), AppError> {
    let trimmed = model_id.trim();
    // Save to persistent location (~/.locus/) — survives reinstalls
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("last_model.txt"), trimmed)
        .map_err(|e| format!("Failed to save last model: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_last_effort(_app_handle: AppHandle) -> Result<String, AppError> {
    let primary_path = persistent_config_dir()?.join("last_effort.txt");
    if let Some(val) = read_nonempty_string(&primary_path) {
        return Ok(val);
    }
    Ok(String::new())
}

#[tauri::command]
pub async fn save_last_effort(effort: String, _app_handle: AppHandle) -> Result<(), AppError> {
    let trimmed = effort.trim();
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("last_effort.txt"), trimmed)
        .map_err(|e| format!("Failed to save last effort: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefaults {
    #[serde(default)]
    pub main_model: String,
    #[serde(default)]
    pub plan_model: String,
    #[serde(default)]
    pub subagent_models: std::collections::HashMap<String, String>,
}

impl Default for ModelDefaults {
    fn default() -> Self {
        ModelDefaults {
            main_model: String::new(),
            plan_model: String::new(),
            subagent_models: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexTransportMode {
    Http,
    Websocket,
}

impl Default for CodexTransportMode {
    fn default() -> Self {
        Self::Websocket
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelConfig {
    #[serde(default)]
    pub transport: CodexTransportMode,
}

fn codex_model_config_path() -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("codex_model_config.json"))
}

pub(crate) fn load_codex_model_config() -> Result<CodexModelConfig, String> {
    let path = codex_model_config_path()?;
    Ok(std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<CodexModelConfig>(&s).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn get_model_defaults(_app_handle: AppHandle) -> Result<ModelDefaults, AppError> {
    let primary_path = persistent_config_dir()?.join("model_defaults.json");
    if let Some(defaults) = std::fs::read_to_string(&primary_path)
        .ok()
        .and_then(|s| serde_json::from_str::<ModelDefaults>(&s).ok())
    {
        return Ok(defaults);
    }
    Ok(ModelDefaults::default())
}

#[tauri::command]
pub async fn save_model_defaults(
    defaults: ModelDefaults,
    _app_handle: AppHandle,
) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(&defaults)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    // Save to persistent location
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("model_defaults.json"), &json)
        .map_err(|e| format!("Failed to save model defaults: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_codex_model_config() -> Result<CodexModelConfig, AppError> {
    load_codex_model_config().map_err(AppError::from)
}

#[tauri::command]
pub async fn get_codex_available_models(
    codex: State<'_, crate::commands::auth::CodexAuthStateHandle>,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<Vec<crate::llm::codex_models::CodexAvailableModel>, AppError> {
    let cache_dir = persistent_config_dir().map_err(AppError::from)?;
    let (access_token, account_id) = {
        let mut codex_guard = codex.lock().await;
        let access_token = codex_guard.access_token().await.map_err(AppError::from)?;
        let account_id = codex_guard.account_id();
        (access_token, account_id)
    };

    crate::llm::codex_models::list_codex_available_models(
        &access_token,
        account_id.as_deref(),
        config.base_url.as_deref(),
        &cache_dir,
    )
    .await
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn save_codex_model_config(config: CodexModelConfig) -> Result<(), AppError> {
    let path = codex_model_config_path().map_err(AppError::from)?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize codex model config: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save codex model config: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomReasoningParamFormat {
    None,
    OpenaiChatReasoningEffort,
    OpenaiResponsesReasoningEffort,
    AnthropicThinking,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpointServerTools {
    #[serde(default)]
    pub web_search: bool,
}

fn default_supports_tool_lazy_loading() -> bool {
    false
}

fn default_supports_vision() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub id: String,
    pub name: String,
    pub api_model: String,
    pub endpoint: String,
    pub api_format: ApiFormat,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_context_length")]
    pub context_length: u32,
    #[serde(default)]
    pub beta_flags: Vec<String>,
    #[serde(default = "default_supported_reasoning_efforts")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning_param_format: Option<CustomReasoningParamFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_reasoning_content: Option<bool>,
    #[serde(default)]
    pub server_tools: CustomEndpointServerTools,
    #[serde(default = "default_supports_tool_lazy_loading")]
    pub supports_tool_lazy_loading: bool,
    #[serde(default = "default_supports_vision")]
    pub supports_vision: bool,
}

const DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH: u32 = 256_000;

fn default_context_length() -> u32 {
    DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH
}

fn default_supported_reasoning_efforts() -> Vec<String> {
    ["low", "medium", "high", "max"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn default_reasoning_param_format(api_format: &ApiFormat) -> CustomReasoningParamFormat {
    match api_format {
        ApiFormat::OpenaiResponses => CustomReasoningParamFormat::OpenaiResponsesReasoningEffort,
        ApiFormat::AnthropicMessages => CustomReasoningParamFormat::AnthropicThinking,
        ApiFormat::OpenaiChat => CustomReasoningParamFormat::OpenaiChatReasoningEffort,
    }
}

fn default_replay_reasoning_content(endpoint: &CustomEndpoint) -> bool {
    endpoint.api_format == ApiFormat::OpenaiChat
}

fn normalize_reasoning_effort(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Some(trimmed),
        _ => None,
    }
}

fn custom_endpoint_ids_from_file(path: &std::path::Path) -> HashSet<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<CustomEndpoint>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|endpoint| endpoint.id)
        .collect()
}

fn is_stale_custom_model_ref(model_id: &str, valid_endpoint_ids: &HashSet<String>) -> bool {
    if let Some(endpoint_id) = model_id.trim().strip_prefix("custom/") {
        return !endpoint_id.is_empty() && !valid_endpoint_ids.contains(endpoint_id);
    }
    false
}

fn prune_stale_custom_model_refs(valid_endpoint_ids: &HashSet<String>) -> Result<(), String> {
    let dir = persistent_config_dir()?;
    let last_model_path = dir.join("last_model.txt");
    if let Some(last_model) = read_nonempty_string(&last_model_path) {
        if is_stale_custom_model_ref(&last_model, valid_endpoint_ids) {
            let _ = std::fs::remove_file(&last_model_path);
        }
    }

    let defaults_path = dir.join("model_defaults.json");
    let Some(mut defaults) = std::fs::read_to_string(&defaults_path)
        .ok()
        .and_then(|s| serde_json::from_str::<ModelDefaults>(&s).ok())
    else {
        return Ok(());
    };

    let mut changed = false;
    if is_stale_custom_model_ref(&defaults.main_model, valid_endpoint_ids) {
        defaults.main_model.clear();
        changed = true;
    }
    if is_stale_custom_model_ref(&defaults.plan_model, valid_endpoint_ids) {
        defaults.plan_model.clear();
        changed = true;
    }
    defaults.subagent_models.retain(|_, model_id| {
        let keep = !is_stale_custom_model_ref(model_id, valid_endpoint_ids);
        if !keep {
            changed = true;
        }
        keep
    });

    if changed {
        let json = serde_json::to_string_pretty(&defaults)
            .map_err(|e| format!("Failed to serialize model defaults: {}", e))?;
        std::fs::write(&defaults_path, json)
            .map_err(|e| format!("Failed to save model defaults: {}", e))?;
    }
    Ok(())
}

pub(crate) fn normalize_custom_endpoint_config(endpoint: &mut CustomEndpoint) {
    endpoint.supported_reasoning_efforts = endpoint
        .supported_reasoning_efforts
        .iter()
        .filter_map(|value| normalize_reasoning_effort(value))
        .collect();
    if endpoint.supported_reasoning_efforts.is_empty() {
        endpoint.supported_reasoning_efforts = default_supported_reasoning_efforts();
    }
    if endpoint.context_length == 0 {
        endpoint.context_length = default_context_length();
    }
    if endpoint.reasoning_param_format.is_none() {
        endpoint.reasoning_param_format =
            Some(default_reasoning_param_format(&endpoint.api_format));
    }
    if endpoint.replay_reasoning_content.is_none() {
        endpoint.replay_reasoning_content = Some(default_replay_reasoning_content(endpoint));
    }
    endpoint.supports_tool_lazy_loading = false;
}

#[tauri::command]
pub async fn get_custom_endpoints(app_handle: AppHandle) -> Result<Vec<CustomEndpoint>, AppError> {
    let path = custom_endpoints_path(&app_handle)?;
    let mut endpoints: Vec<CustomEndpoint> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    for ep in &mut endpoints {
        normalize_custom_endpoint_config(ep);
        if let Ok(Some(key)) = keychain::get_secret(&keychain::endpoint_key_name(&ep.id)) {
            ep.api_key = key;
        }
    }

    Ok(endpoints)
}

#[tauri::command]
pub async fn save_custom_endpoints(
    endpoints: Vec<CustomEndpoint>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let path = custom_endpoints_path(&app_handle)?;
    let previous_endpoint_ids = custom_endpoint_ids_from_file(&path);
    let next_endpoint_ids: HashSet<String> = endpoints
        .iter()
        .map(|endpoint| endpoint.id.clone())
        .collect();

    // Save api_key to keychain, strip from JSON file
    for ep in &endpoints {
        if !ep.api_key.is_empty() {
            keychain::set_secret(&keychain::endpoint_key_name(&ep.id), &ep.api_key)?;
        } else {
            let _ = keychain::delete_secret(&keychain::endpoint_key_name(&ep.id));
        }
    }

    let mut stripped = endpoints;
    for ep in &mut stripped {
        normalize_custom_endpoint_config(ep);
        ep.api_key = String::new();
    }

    let json = serde_json::to_string_pretty(&stripped)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save custom endpoints: {}", e))?;
    for endpoint_id in previous_endpoint_ids.difference(&next_endpoint_ids) {
        let _ = keychain::delete_secret(&keychain::endpoint_key_name(endpoint_id));
    }
    prune_stale_custom_model_refs(&next_endpoint_ids)?;
    Ok(())
}

#[tauri::command]
pub async fn test_custom_endpoint(endpoint: CustomEndpoint) -> Result<String, AppError> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(30))
            .gzip(true)
            .deflate(true),
    )
    .map_err(|e| format!("HTTP client error: {}", e))?;

    match endpoint.api_format {
        ApiFormat::OpenaiChat => {
            let url = format!(
                "{}/chat/completions",
                endpoint.endpoint.trim_end_matches('/')
            );
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "messages": [{"role": "user", "content": "Hi"}],
                "max_tokens": 16,
                "stream": false,
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !endpoint.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                    return Ok(content.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
        ApiFormat::OpenaiResponses => {
            let url = format!("{}/responses", endpoint.endpoint.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "input": "Hi",
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !endpoint.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                // Responses API: output[].content[].text
                if let Some(output) = json["output"].as_array() {
                    for item in output {
                        if let Some(content) = item["content"].as_array() {
                            for block in content {
                                if let Some(t) = block["text"].as_str() {
                                    return Ok(t.to_string());
                                }
                            }
                        }
                        if let Some(t) = item["text"].as_str() {
                            return Ok(t.to_string());
                        }
                    }
                }
                if let Some(t) = json["output_text"].as_str() {
                    return Ok(t.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
        ApiFormat::AnthropicMessages => {
            let url = format!("{}/messages", endpoint.endpoint.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "messages": [{"role": "user", "content": "Hi"}],
                "max_tokens": 16,
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("anthropic-version", "2023-06-01");
            if !endpoint.beta_flags.is_empty() {
                req = req.header("anthropic-beta", endpoint.beta_flags.join(","));
            }
            if !endpoint.api_key.is_empty() {
                req = req
                    .header("x-api-key", &endpoint.api_key)
                    .header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(content) = json["content"][0]["text"].as_str() {
                    return Ok(content.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

/// If the response body looks like HTML (e.g. a CDN challenge page),
/// save it to a temp file and return a message with `[OPEN_HTML:filepath]` marker.
fn maybe_html_fallback(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let head = trimmed
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    if head.starts_with("<!") || head.starts_with("<html") {
        let tmp =
            std::env::temp_dir().join(format!("locus_endpoint_test_{}.html", std::process::id()));
        if std::fs::write(&tmp, text).is_ok() {
            Some(format!(
                "Server returned an HTML page instead of JSON (possible verification/challenge page). [OPEN_HTML:{}]",
                tmp.display()
            ))
        } else {
            Some("Server returned an HTML page instead of JSON.".to_string())
        }
    } else {
        None
    }
}

fn endpoint_html_response_error(message: String, status: Option<reqwest::StatusCode>) -> AppError {
    let message = match status {
        Some(status) => format!("HTTP {} — {}", status.as_u16(), message),
        None => message,
    };
    AppError::new(ENDPOINT_TEST_HTML_RESPONSE_CODE, message)
}

#[tauri::command]
pub async fn get_debug_mode(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.debug_enabled())
}

#[tauri::command]
pub async fn set_debug_mode(
    value: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config.set_debug_enabled(value).map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn get_file_tool_workspace_boundary(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.file_tool_workspace_boundary_enabled())
}

#[tauri::command]
pub async fn set_file_tool_workspace_boundary(
    value: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config
        .set_file_tool_workspace_boundary_enabled(value)
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn get_tool_permission_mode(
    mode: State<'_, crate::ToolPermissionMode>,
) -> Result<String, AppError> {
    Ok(mode.0.read().await.clone())
}

fn normalize_tool_permission_mode_request(value: Option<&str>, mode: Option<&str>) -> &'static str {
    let requested = value.or(mode).unwrap_or_default().trim();
    if requested.eq_ignore_ascii_case("ask") {
        "ask"
    } else {
        "auto"
    }
}

#[tauri::command]
pub async fn save_tool_permission_mode(
    value: Option<String>,
    mode: Option<String>,
    mode_state: State<'_, crate::ToolPermissionMode>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    // Accept both `value` and the legacy `mode` argument to keep older frontends working.
    let normalized =
        normalize_tool_permission_mode_request(value.as_deref(), mode.as_deref()).to_string();
    *mode_state.0.write().await = normalized.clone();
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let path = data_dir.join("tool_permission_mode.txt");
    std::fs::write(&path, &normalized)
        .map_err(|e| format!("Failed to save tool permission mode: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_tool_permissions(
    perms: State<'_, crate::ToolPermissions>,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    Ok(perms.0.read().await.clone())
}

#[tauri::command]
pub async fn save_tool_permissions(
    value: std::collections::HashMap<String, String>,
    perms: State<'_, crate::ToolPermissions>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let normalized: std::collections::HashMap<String, String> = value
        .into_iter()
        .map(|(k, v)| {
            let mode = normalize_tool_permission_mode_request(Some(v.as_str()), None).to_string();
            (k, mode)
        })
        .collect();
    *perms.0.write().await = normalized.clone();
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let path = data_dir.join("tool_permissions.json");
    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("Failed to serialize tool permissions: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save tool permissions: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub rel_path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchEntry {
    pub rel_path: String,
    pub name: String,
    pub parent_path: String,
    pub is_dir: bool,
    pub match_score: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntryStat {
    pub path: String,
    pub exists: bool,
    pub entry_kind: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntriesPage {
    pub entries: Vec<DirEntry>,
    pub total_count: usize,
    pub next_offset: usize,
    pub has_more: bool,
}

#[derive(Default)]
struct DirEntriesPageCacheInner {
    order: VecDeque<String>,
    listings: HashMap<String, Arc<[DirEntry]>>,
}

#[derive(Clone, Default)]
pub struct DirEntriesPageCache(Arc<Mutex<DirEntriesPageCacheInner>>);

impl DirEntriesPageCache {
    const MAX_ENTRIES: usize = 24;

    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(DirEntriesPageCacheInner::default())))
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.0.lock() {
            guard.order.clear();
            guard.listings.clear();
        }
    }

    fn get(&self, key: &str) -> Option<Arc<[DirEntry]>> {
        let mut guard = self.0.lock().ok()?;
        let listing = guard.listings.get(key).cloned()?;
        if let Some(index) = guard.order.iter().position(|existing| existing == key) {
            guard.order.remove(index);
        }
        guard.order.push_back(key.to_string());
        Some(listing)
    }

    fn insert(&self, key: String, entries: Vec<DirEntry>) -> Arc<[DirEntry]> {
        let listing: Arc<[DirEntry]> = Arc::from(entries.into_boxed_slice());
        if let Ok(mut guard) = self.0.lock() {
            if let Some(index) = guard.order.iter().position(|existing| existing == &key) {
                guard.order.remove(index);
            }
            guard.order.push_back(key.clone());
            guard.listings.insert(key, listing.clone());

            while guard.order.len() > Self::MAX_ENTRIES {
                if let Some(stale_key) = guard.order.pop_front() {
                    guard.listings.remove(&stale_key);
                }
            }
        }
        listing
    }
}

const WORKSPACE_HIDDEN_DIRS: &[&str] = &[
    ".git",
    ".vs",
    ".vscode",
    ".idea",
    "node_modules",
    "__pycache__",
    ".next",
    "dist",
    "build",
    "Library",
    "Temp",
    "Logs",
    "obj",
];

const ASSET_ROOT_DIRS: &[&str] = &["Assets", "Packages", "ProjectSettings"];
const LINKED_ASSET_ROOT_DIRS: &[&str] = &["Assets", "Packages"];
const WORKSPACE_SEARCH_MAX_DEPTH: usize = 64;

pub(crate) fn normalize_workspace_sub_path(sub_path: &str) -> Result<String, AppError> {
    let unified = sub_path.replace('\\', "/");
    if unified.contains('\0')
        || unified.starts_with('/')
        || unified
            .split('/')
            .next()
            .map(|head| {
                head.len() >= 2
                    && head.as_bytes()[1] == b':'
                    && head.as_bytes()[0].is_ascii_alphabetic()
            })
            .unwrap_or(false)
    {
        return Err("Path is not within the working directory"
            .to_string()
            .into());
    }

    let mut parts = Vec::new();

    for component in std::path::Path::new(&unified).components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err("Path is not within the working directory"
                    .to_string()
                    .into());
            }
        }
    }

    Ok(parts.join("/"))
}

fn resolve_workspace_dir_target(
    cwd: &str,
    sub_path: &str,
) -> Result<(std::path::PathBuf, String), AppError> {
    let base = std::path::Path::new(cwd);
    let normalized_sub_path = normalize_workspace_sub_path(sub_path)?;
    let target = if normalized_sub_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(&normalized_sub_path)
    };

    if !target.is_dir() {
        return Ok((target, normalized_sub_path));
    }

    let canonical_base = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let canonical_target = dunce::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if canonical_target.starts_with(&canonical_base)
        || path_reaches_allowed_linked_asset_dir(base, &normalized_sub_path)
    {
        return Ok((target, normalized_sub_path));
    }

    Err("Path is not within the working directory"
        .to_string()
        .into())
}

fn should_skip_workspace_entry(file_name: &str, is_dir: bool, exclude_meta: bool) -> bool {
    if file_name.starts_with('.') {
        return true;
    }

    if exclude_meta && file_name.ends_with(".meta") {
        return true;
    }

    is_dir && WORKSPACE_HIDDEN_DIRS.contains(&file_name)
}

fn join_workspace_rel_path(sub_path: &str, file_name: &str) -> String {
    if sub_path.is_empty() {
        file_name.to_string()
    } else {
        format!("{}/{}", sub_path.trim_end_matches('/'), file_name)
    }
}

fn is_allowed_linked_asset_rel_path(rel_path: &str) -> bool {
    LINKED_ASSET_ROOT_DIRS.iter().any(|root| {
        rel_path == *root
            || rel_path
                .strip_prefix(root)
                .map(|rest| rest.starts_with('/'))
                .unwrap_or(false)
    })
}

fn path_reaches_allowed_linked_asset_dir(base: &std::path::Path, rel_path: &str) -> bool {
    let mut current = base.to_path_buf();
    let mut rel_parts = Vec::new();
    let mut saw_allowed_linked_dir = false;

    for part in rel_path.split('/').filter(|part| !part.is_empty()) {
        current.push(part);
        rel_parts.push(part);
        if path_is_symlink_dir(&current) {
            let current_rel_path = rel_parts.join("/");
            if !is_allowed_linked_asset_rel_path(&current_rel_path) {
                return false;
            }
            saw_allowed_linked_dir = true;
        }
    }

    saw_allowed_linked_dir
}

fn entry_is_dir(entry: &std::fs::DirEntry, rel_path: &str) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => true,
        Ok(file_type) if file_type.is_symlink() => {
            is_allowed_linked_asset_rel_path(rel_path) && entry.path().is_dir()
        }
        Ok(_) => false,
        Err(_) => {
            let path = entry.path();
            if path_is_symlink_dir(&path) {
                is_allowed_linked_asset_rel_path(rel_path)
            } else {
                path.is_dir()
            }
        }
    }
}

fn entry_is_file(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_file() => true,
        Ok(file_type) if file_type.is_symlink() => entry.path().is_file(),
        Ok(_) => false,
        Err(_) => entry.path().is_file(),
    }
}

fn entry_is_symlink_dir(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_symlink() => entry.path().is_dir(),
        _ => false,
    }
}

fn entry_is_disallowed_symlink_dir(entry: &std::fs::DirEntry, rel_path: &str) -> bool {
    entry_is_symlink_dir(entry) && !is_allowed_linked_asset_rel_path(rel_path)
}

fn path_is_symlink_dir(path: &std::path::Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => path.is_dir(),
        _ => false,
    }
}

fn collect_dir_entries(
    target: &std::path::Path,
    sub_path: &str,
    exclude_meta: bool,
) -> Result<Vec<DirEntry>, AppError> {
    let mut entries: Vec<DirEntry> = Vec::new();
    let read_dir =
        std::fs::read_dir(target).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let rel_path = join_workspace_rel_path(sub_path, &file_name);

        if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
            continue;
        }

        let is_dir = entry_is_dir(&entry, &rel_path);

        if should_skip_workspace_entry(&file_name, is_dir, exclude_meta) {
            continue;
        }

        entries.push(DirEntry {
            rel_path,
            name: file_name,
            is_dir,
        });
    }

    entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));

    Ok(entries)
}

fn normalize_workspace_entry_stat_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') && !normalized.ends_with(":/") {
        normalized.pop();
    }
    normalized
}

fn missing_workspace_entry_stat(path: &str) -> WorkspaceEntryStat {
    WorkspaceEntryStat {
        path: normalize_workspace_entry_stat_path(path),
        exists: false,
        entry_kind: "missing".to_string(),
    }
}

fn workspace_entry_stat_from_target(path: String, target: &std::path::Path) -> WorkspaceEntryStat {
    let is_dir = target.is_dir();
    let is_file = target.is_file();
    let entry_kind = if is_dir {
        "folder"
    } else if is_file {
        "file"
    } else {
        "other"
    };

    WorkspaceEntryStat {
        path,
        exists: target.exists(),
        entry_kind: entry_kind.to_string(),
    }
}

fn is_workspace_entry_stat_absolute_path(path: &str) -> bool {
    let normalized = path.trim();
    std::path::Path::new(normalized).is_absolute()
        || normalized.starts_with("\\\\")
        || normalized.starts_with("//")
}

fn workspace_entry_target_allowed(
    workspace_root: &std::path::Path,
    rel_path: &str,
    target: &std::path::Path,
) -> bool {
    let canonical_base =
        dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let canonical_target = match dunce::canonicalize(target) {
        Ok(path) => path,
        Err(_) => return false,
    };
    canonical_target.starts_with(&canonical_base)
        || path_reaches_allowed_linked_asset_dir(workspace_root, rel_path)
}

fn workspace_entry_stat_for_path(cwd: &str, raw_path: &str) -> WorkspaceEntryStat {
    let display_path = normalize_workspace_entry_stat_path(raw_path);
    if display_path.is_empty() {
        return missing_workspace_entry_stat(raw_path);
    }

    if is_workspace_entry_stat_absolute_path(&display_path) {
        let target =
            std::path::PathBuf::from(display_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !target.exists() {
            return missing_workspace_entry_stat(&display_path);
        }
        return workspace_entry_stat_from_target(display_path, &target);
    }

    if cwd.trim().is_empty() {
        return missing_workspace_entry_stat(&display_path);
    }

    let rel_path = match normalize_workspace_sub_path(&display_path) {
        Ok(path) if !path.is_empty() => path,
        _ => return missing_workspace_entry_stat(&display_path),
    };
    let workspace_root = std::path::Path::new(cwd);
    let target = workspace_root.join(&rel_path);
    if !target.exists() {
        return missing_workspace_entry_stat(&rel_path);
    }
    if !workspace_entry_target_allowed(workspace_root, &rel_path, &target) {
        return missing_workspace_entry_stat(&rel_path);
    }

    workspace_entry_stat_from_target(rel_path, &target)
}

fn workspace_search_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn compact_workspace_search(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn workspace_search_score(query: &str, name: &str, rel_path: &str, is_dir: bool) -> Option<i32> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let query_lower = trimmed.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    let rel_lower = rel_path.to_ascii_lowercase();
    let query_tokens = workspace_search_tokens(&query_lower);
    if !query_tokens.is_empty()
        && query_tokens
            .iter()
            .any(|token| !name_lower.contains(token) && !rel_lower.contains(token))
    {
        return None;
    }

    let compact_query = compact_workspace_search(&query_lower);
    let compact_name = compact_workspace_search(&name_lower);
    let compact_rel = compact_workspace_search(&rel_lower);

    let mut score = if name_lower == query_lower {
        1240
    } else if rel_lower == query_lower {
        1200
    } else if name_lower.starts_with(&query_lower) {
        1140 - name_lower.len().min(48) as i32
    } else if rel_lower.starts_with(&query_lower) {
        1080 - rel_lower.len().min(72) as i32
    } else if let Some(index) = name_lower.find(&query_lower) {
        1020 - index as i32 * 8
    } else if let Some(index) = rel_lower.find(&query_lower) {
        960 - index as i32 * 5
    } else if !compact_query.is_empty() && compact_name.starts_with(&compact_query) {
        920 - compact_name.len().min(48) as i32
    } else if !compact_query.is_empty() && compact_rel.contains(&compact_query) {
        let index = compact_rel.find(&compact_query).unwrap_or(0) as i32;
        860 - index * 4
    } else {
        return None;
    };

    score -= rel_path.matches('/').count() as i32 * 3;
    if is_dir {
        score += 12;
    }
    Some(score)
}

fn build_workspace_search_entry(
    rel_path: String,
    name: String,
    is_dir: bool,
    match_score: i32,
) -> WorkspaceSearchEntry {
    let parent_path = rel_path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default();
    WorkspaceSearchEntry {
        rel_path,
        name,
        parent_path,
        is_dir,
        match_score,
    }
}

fn collect_workspace_search_entries(
    root_dir: &std::path::Path,
    root_rel_path: &str,
    include_files: bool,
    query: &str,
    results: &mut Vec<WorkspaceSearchEntry>,
) -> Result<(), AppError> {
    let initial_linked_visit_keys = path_is_symlink_dir(root_dir).then(|| Arc::new(HashSet::new()));
    let mut stack = vec![(
        root_dir.to_path_buf(),
        root_rel_path.to_string(),
        0usize,
        initial_linked_visit_keys,
    )];

    while let Some((dir_path, dir_rel_path, depth, linked_visit_keys)) = stack.pop() {
        let current_linked_visit_keys = if let Some(keys) = linked_visit_keys {
            let visit_key = dunce::canonicalize(&dir_path).unwrap_or_else(|_| dir_path.clone());
            if keys.contains(&visit_key) {
                continue;
            }
            let mut updated = (*keys).clone();
            updated.insert(visit_key);
            Some(Arc::new(updated))
        } else {
            None
        };

        let read_dir =
            std::fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?;
        let mut child_dirs: Vec<(
            std::path::PathBuf,
            String,
            Option<Arc<HashSet<std::path::PathBuf>>>,
        )> = Vec::new();

        for entry in read_dir.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let rel_path = join_workspace_rel_path(&dir_rel_path, &file_name);
            if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
                continue;
            }
            let is_dir = entry_is_dir(&entry, &rel_path);
            if should_skip_workspace_entry(&file_name, is_dir, false) {
                continue;
            }

            let is_file = entry_is_file(&entry);
            if !is_dir && (!include_files || !is_file) {
                continue;
            }
            if let Some(match_score) = workspace_search_score(query, &file_name, &rel_path, is_dir)
            {
                results.push(build_workspace_search_entry(
                    rel_path.clone(),
                    file_name.clone(),
                    is_dir,
                    match_score,
                ));
            }

            if is_dir && depth < WORKSPACE_SEARCH_MAX_DEPTH {
                let child_linked_visit_keys = current_linked_visit_keys
                    .clone()
                    .or_else(|| entry_is_symlink_dir(&entry).then(|| Arc::new(HashSet::new())));
                child_dirs.push((entry.path(), rel_path, child_linked_visit_keys));
            }
        }

        child_dirs.sort_by(|left, right| right.1.cmp(&left.1));
        stack.extend(
            child_dirs
                .into_iter()
                .map(|(path, rel_path, linked_visit_keys)| {
                    (path, rel_path, depth + 1, linked_visit_keys)
                }),
        );
    }

    Ok(())
}

fn search_workspace_entries_in_dir(
    workspace_root: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    if !workspace_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let read_dir = std::fs::read_dir(workspace_root)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let rel_path = file_name.clone();
        if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
            continue;
        }
        let is_dir = entry_is_dir(&entry, &rel_path);
        if should_skip_workspace_entry(&file_name, is_dir, false) {
            continue;
        }

        if let Some(match_score) = workspace_search_score(query, &file_name, &rel_path, is_dir) {
            results.push(build_workspace_search_entry(
                rel_path.clone(),
                file_name.clone(),
                is_dir,
                match_score,
            ));
        }

        if !is_dir {
            continue;
        }

        let include_files = !ASSET_ROOT_DIRS.contains(&rel_path.as_str());
        collect_workspace_search_entries(
            &entry.path(),
            &rel_path,
            include_files,
            query,
            &mut results,
        )?;
    }

    results.sort_by(|left, right| {
        right
            .match_score
            .cmp(&left.match_score)
            .then_with(|| right.is_dir.cmp(&left.is_dir))
            .then_with(|| left.rel_path.len().cmp(&right.rel_path.len()))
            .then_with(|| left.rel_path.cmp(&right.rel_path))
    });

    if results.len() > limit {
        results.truncate(limit);
    }

    Ok(results)
}

#[tauri::command]
pub async fn list_dir_entries(
    sub_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<DirEntry>, AppError> {
    let cwd = workspace.path.read().await.clone();
    let (target, normalized_sub_path) = resolve_workspace_dir_target(&cwd, &sub_path)?;
    if !target.is_dir() {
        return Ok(vec![]);
    }

    collect_dir_entries(&target, &normalized_sub_path, false)
}

#[tauri::command]
pub async fn list_dir_entries_page(
    sub_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    exclude_meta: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
) -> Result<DirEntriesPage, AppError> {
    let cwd = workspace.path.read().await.clone();
    let (target, normalized_sub_path) = resolve_workspace_dir_target(&cwd, &sub_path)?;
    if !target.is_dir() {
        return Ok(DirEntriesPage {
            entries: Vec::new(),
            total_count: 0,
            next_offset: 0,
            has_more: false,
        });
    }

    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(200).clamp(1, 2_000);
    let exclude_meta = exclude_meta.unwrap_or(false);
    let cache_key = format!(
        "{}::{}::{}",
        cwd,
        normalized_sub_path,
        u8::from(exclude_meta)
    );

    let listing = if offset == 0 {
        let entries = collect_dir_entries(&target, &normalized_sub_path, exclude_meta)?;
        dir_entries_cache.insert(cache_key.clone(), entries)
    } else if let Some(cached) = dir_entries_cache.get(&cache_key) {
        cached
    } else {
        let entries = collect_dir_entries(&target, &normalized_sub_path, exclude_meta)?;
        dir_entries_cache.insert(cache_key.clone(), entries)
    };

    let total_count = listing.len();
    let start = offset.min(total_count);
    let end = (start + limit).min(total_count);

    Ok(DirEntriesPage {
        entries: listing[start..end].to_vec(),
        total_count,
        next_offset: end,
        has_more: end < total_count,
    })
}

#[tauri::command]
pub async fn search_workspace_entries(
    query: String,
    limit: Option<usize>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.unwrap_or(200).clamp(1, 500);
    search_workspace_entries_in_dir(std::path::Path::new(&cwd), trimmed, limit)
}

#[tauri::command]
pub async fn stat_workspace_entries(
    paths: Vec<String>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<WorkspaceEntryStat>, AppError> {
    let cwd = workspace.path.read().await.clone();
    let mut stats = Vec::new();
    let mut seen = HashSet::new();

    for path in paths.into_iter().take(300) {
        let normalized = normalize_workspace_entry_stat_path(&path);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        stats.push(workspace_entry_stat_for_path(&cwd, &normalized));
    }

    Ok(stats)
}

#[tauri::command]
pub async fn check_unity_connection(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<bool, AppError> {
    let cwd = workspace.path.read().await.clone();
    Ok(crate::unity_bridge::is_unity_connected(&cwd).await)
}

#[tauri::command]
pub async fn check_unity_connection_status(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<crate::unity_bridge::UnityConnectionStatus, AppError> {
    let cwd = workspace.path.read().await.clone();
    Ok(crate::unity_bridge::query_unity_connection_status(&cwd).await)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConsoleTextEntry {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConsoleTextPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub entries: Vec<UnityConsoleTextEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[tauri::command]
pub async fn get_unity_console_text(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<UnityConsoleTextPayload, AppError> {
    let cwd = workspace.path.read().await.clone();
    let resp = crate::unity_bridge::send_message(&cwd, "get_console_text", "").await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "Failed to read Unity Console".to_string())
            .into());
    }

    let message = resp.message.unwrap_or_default();
    let mut payload: UnityConsoleTextPayload = serde_json::from_str(&message).map_err(|error| {
        AppError::from(format!("Failed to parse Unity Console response: {error}"))
    })?;
    payload
        .entries
        .retain(|entry| !entry.text.trim().is_empty());
    Ok(payload)
}

#[tauri::command]
pub async fn check_unity_plugin(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<crate::unity_bridge::PluginStatus, AppError> {
    let cwd = workspace.path.read().await.clone();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(crate::unity_bridge::PluginStatus::UpToDate);
    }
    crate::unity_bridge::check_plugin_status(&cwd).map_err(Into::into)
}

#[tauri::command]
pub async fn install_unity_plugin(
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Err("Current working directory is not a Unity project"
            .to_string()
            .into());
    }
    let hash = crate::unity_bridge::install_or_update_plugin(&cwd)?;
    crate::unity_bridge::emit_plugin_status(&app_handle, &cwd);
    Ok(hash)
}

#[tauri::command]
pub async fn launch_unity_project(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<crate::unity_bridge::UnityLaunchResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::launch_project(&cwd).map_err(Into::into)
}

#[tauri::command]
pub async fn send_unity_log(
    message: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    let resp = crate::unity_bridge::send_message(&cwd, "log", &message).await?;
    if resp.ok {
        Ok(format!("Unity log sent: {}", message))
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "unknown error".to_string())
            .into())
    }
}

#[tauri::command]
pub async fn select_unity_asset(
    asset_path: String,
    focus_project_window: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::select_asset(&cwd, &asset_path, focus_project_window.unwrap_or(true))
        .await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_asset_inspector(
    asset_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::open_asset_inspector(&cwd, &asset_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn select_unity_scene_object(
    scene_path: String,
    object_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::select_scene_object(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_scene_object_inspector(
    scene_path: String,
    object_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::open_scene_object_inspector(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn reset_all_config(
    workspace: State<'_, Arc<Workspace>>,
    unity_monitor: State<'_, UnityMonitorHandle>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    scan_task_state: State<'_, super::RefGraphScanTaskState>,
    reconcile_task_state: State<'_, AssetDbReconcileTaskState>,
    preview_cache: State<'_, WorkspacePreviewCache>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
    knowledge_index_state: State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>>,
    mode: State<'_, crate::ToolPermissionMode>,
    perms: State<'_, crate::ToolPermissions>,
    api_key_state: State<'_, crate::ApiKeyState>,
    provider_keys: State<'_, crate::ProviderKeysState>,
    auth: State<'_, Arc<tokio::sync::Mutex<crate::auth::AuthState>>>,
    codex: State<'_, crate::commands::auth::CodexAuthStateHandle>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;

    let cancelled = scan_task_state.cancel_current_and_wait("config reset");
    if !cancelled {
        eprintln!("[Locus] warning: asset DB scan cancellation did not finish before reset");
    }
    reconcile_task_state.cancel_current("config reset");

    // Clear keychain secrets: OpenRouter key
    let _ = keychain::delete_secret(keychain::KEY_OPENROUTER);

    // Clear keychain secrets: all provider keys
    {
        let keys = provider_keys.read().await;
        for id in keys.keys() {
            let _ = keychain::delete_secret(&keychain::provider_key_name(id));
        }
    }

    // Clear keychain secrets: custom endpoint API keys
    let ep_path = custom_endpoints_path(&app_handle)
        .unwrap_or_else(|_| data_dir.join("custom_endpoints.json"));
    if let Ok(content) = std::fs::read_to_string(&ep_path) {
        if let Ok(endpoints) = serde_json::from_str::<Vec<CustomEndpoint>>(&content) {
            for ep in &endpoints {
                let _ = keychain::delete_secret(&keychain::endpoint_key_name(&ep.id));
            }
        }
    }

    // OAuth/Codex tokens are cleared by .logout() which now uses keychain

    let config_files = [
        "provider_key_ids.json",
        "working_dir.txt",
        "recent_dirs.json",
        "active_session_selection.json",
        "tool_permission_mode.txt",
        "tool_permissions.json",
        "git_path_override.txt",
        "config.json",
    ];

    for file in &config_files {
        let path = data_dir.join(file);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    // Also clear the stable config dir.
    if let Ok(pdir) = persistent_config_dir() {
        for file in [
            "config.json",
            "last_model.txt",
            "last_effort.txt",
            "model_defaults.json",
            "custom_endpoints.json",
            "codex_model_config.json",
            crate::python_runtime::config_file_name(),
        ] {
            let path = pdir.join(file);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    if let Some(webview) = app_handle.webview_windows().values().next() {
        let _ = webview.clear_all_browsing_data();
    }

    {
        let mut wh = watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = wh.take() {
            old.stop_and_join();
            eprintln!("[Locus] stopped ref_graph watcher during reset");
        }
    }
    {
        *ref_graph_state
            .0
            .lock()
            .map_err(|e| format!("Lock error: {}", e))? = None;
    }
    last_scan_info.clear();
    scan_phase_state.clear();
    preview_cache.clear();
    dir_entries_cache.clear();

    crate::unity_bridge::stop_unity_monitor(&unity_monitor).await;
    let _ = app_handle.emit("unity-connection-status", false);

    *workspace.path.write().await = String::new();
    *workspace.workspace_id.write().await = None;
    super::reset_unity_embed_control_window(&app_handle);
    super::refresh_unity_embed_control_server(app_handle.clone());
    let no_workspace_library_dir = crate::knowledge_index::no_workspace_library_dir();
    knowledge_index_state
        .rebuild(&no_workspace_library_dir, &data_dir)
        .await?;
    *mode.0.write().await = "auto".to_string();
    *perms.0.write().await = std::collections::HashMap::new();
    *api_key_state.write().await = String::new();
    *provider_keys.write().await = std::collections::HashMap::new();
    auth.lock().await.logout();
    codex.lock().await.logout();

    eprintln!(
        "[Locus] All config reset (keychain + config files + runtime state + WebView browsing data)"
    );
    Ok(())
}

// ── Config registry ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_config_registry(
    category: Option<String>,
    app_handle: AppHandle,
) -> Result<Vec<crate::config_registry::ConfigEntry>, AppError> {
    match category.as_deref() {
        Some(cat) => crate::config_registry::collect_by_category(&app_handle, cat),
        None => crate::config_registry::collect_all(&app_handle),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_dir_entries, normalize_custom_endpoint_config,
        normalize_tool_permission_mode_request, normalize_workspace_sub_path,
        resolve_workspace_dir_target, search_workspace_entries_in_dir,
        workspace_entry_stat_for_path, workspace_search_score, CustomEndpoint,
    };
    use std::path::Path;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_dir_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(source, link)
    }

    fn create_dir_symlink_or_skip(source: &Path, link: &Path) -> bool {
        match create_dir_symlink(source, link) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipping symlink test; failed to create directory symlink: {error}");
                false
            }
        }
    }

    #[test]
    fn normalize_tool_permission_mode_accepts_primary_value_arg() {
        assert_eq!(
            normalize_tool_permission_mode_request(Some("ask"), Some("auto")),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(Some("auto"), Some("ask")),
            "auto"
        );
    }

    #[test]
    fn normalize_tool_permission_mode_accepts_legacy_mode_arg() {
        assert_eq!(
            normalize_tool_permission_mode_request(None, Some("ask")),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(None, Some("auto")),
            "auto"
        );
        assert_eq!(normalize_tool_permission_mode_request(None, None), "auto");
    }

    #[test]
    fn normalize_tool_permission_mode_trims_and_normalizes_case() {
        assert_eq!(
            normalize_tool_permission_mode_request(Some(" Ask "), None),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(Some(" AUTO "), None),
            "auto"
        );
    }

    #[test]
    fn custom_endpoint_defaults_to_256k_context_length() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "model",
            "endpoint": "https://example.com/v1",
            "apiFormat": "openai_chat"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].context_length, 256_000);
        assert_eq!(endpoints[0].replay_reasoning_content, Some(true));
        assert!(!endpoints[0].server_tools.web_search);
        assert!(!endpoints[0].supports_tool_lazy_loading);
        assert!(endpoints[0].supports_vision);
    }

    #[test]
    fn custom_endpoint_disables_tool_lazy_loading_for_all_formats() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "model",
            "endpoint": "https://example.com/v1",
            "apiFormat": "openai_responses",
            "supportsToolLazyLoading": true
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(!endpoints[0].supports_tool_lazy_loading);
    }

    #[test]
    fn custom_endpoint_preserves_server_tool_settings() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "claude-sonnet-4-20250514",
            "endpoint": "https://api.anthropic.com/v1",
            "apiFormat": "anthropic_messages",
            "serverTools": {
                "webSearch": true
            }
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(endpoints[0].server_tools.web_search);
    }

    #[test]
    fn custom_endpoint_preserves_disabled_vision_setting() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Text Only",
            "apiModel": "local-text",
            "endpoint": "http://localhost:8080/v1",
            "apiFormat": "openai_chat",
            "supportsVision": false
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(!endpoints[0].supports_vision);
    }

    #[test]
    fn custom_endpoint_disables_reasoning_content_replay_for_non_chat_formats() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Responses",
            "apiModel": "gpt-5.1",
            "endpoint": "https://api.openai.com/v1",
            "apiFormat": "openai_responses"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].replay_reasoning_content, Some(false));
    }

    #[test]
    fn custom_endpoint_defaults_anthropic_messages_reasoning_replay_to_disabled() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Anthropic",
            "apiModel": "claude-sonnet-4-20250514",
            "endpoint": "https://api.anthropic.com/v1",
            "apiFormat": "anthropic_messages"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].replay_reasoning_content, Some(false));
    }

    #[test]
    fn workspace_search_score_matches_compact_path_queries() {
        let score = workspace_search_score(
            "UIElementsSchema/UnityEditor.Overlays",
            "UnityEditor.Overlays.xsd",
            "UIElementsSchema/UnityEditor.Overlays.xsd",
            false,
        );

        assert!(score.is_some());
    }

    #[test]
    fn normalize_workspace_sub_path_rejects_workspace_escapes() {
        assert_eq!(
            normalize_workspace_sub_path("Assets\\Linked\\Hero.cs").unwrap(),
            "Assets/Linked/Hero.cs"
        );
        assert!(normalize_workspace_sub_path("../Assets").is_err());
        assert!(normalize_workspace_sub_path("Assets/../ProjectSettings").is_err());
        assert!(normalize_workspace_sub_path("C:/outside").is_err());
        assert!(normalize_workspace_sub_path("C:outside").is_err());
        assert!(normalize_workspace_sub_path("/tmp/outside").is_err());
        assert!(normalize_workspace_sub_path("//server/share").is_err());
    }

    #[test]
    fn search_workspace_entries_in_dir_returns_generic_files_and_directories() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("UIElementsSchema"))
            .expect("create workspace folder");
        std::fs::write(
            temp.path()
                .join("UIElementsSchema/UnityEditor.Overlays.xsd"),
            "schema",
        )
        .expect("write workspace file");
        std::fs::create_dir_all(temp.path().join("Assets/Scripts/UI")).expect("create assets dir");
        std::fs::write(temp.path().join("Assets/Scripts/UI/Hud.prefab"), "prefab")
            .expect("write asset file");

        let generic_results =
            search_workspace_entries_in_dir(temp.path(), "UnityEditor.Overlays", 100)
                .expect("search generic workspace");
        assert!(generic_results.iter().any(|entry| {
            entry.rel_path == "UIElementsSchema/UnityEditor.Overlays.xsd" && !entry.is_dir
        }));

        let folder_results = search_workspace_entries_in_dir(temp.path(), "Scripts", 100)
            .expect("search workspace folders");
        assert!(folder_results
            .iter()
            .any(|entry| { entry.rel_path == "Assets/Scripts" && entry.is_dir }));

        assert!(!folder_results
            .iter()
            .any(|entry| { entry.rel_path == "Assets/Scripts/UI/Hud.prefab" }));
    }

    #[test]
    fn workspace_entry_stat_classifies_existing_roots_files_and_missing_paths() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(temp.path().join("Packages")).expect("create packages dir");
        std::fs::write(temp.path().join("Assets/LICENSE"), "license")
            .expect("write extensionless file");

        let cwd = temp.path().to_string_lossy();
        let assets = workspace_entry_stat_for_path(&cwd, "Assets");
        let packages = workspace_entry_stat_for_path(&cwd, "Packages/");
        let license = workspace_entry_stat_for_path(&cwd, "Assets/LICENSE");
        let missing = workspace_entry_stat_for_path(&cwd, "Assets/Missing.prefab");

        assert!(assets.exists);
        assert_eq!(assets.entry_kind, "folder");
        assert_eq!(assets.path, "Assets");
        assert!(packages.exists);
        assert_eq!(packages.entry_kind, "folder");
        assert_eq!(packages.path, "Packages");
        assert!(license.exists);
        assert_eq!(license.entry_kind, "file");
        assert_eq!(license.path, "Assets/LICENSE");
        assert!(!missing.exists);
        assert_eq!(missing.entry_kind, "missing");
    }

    #[test]
    fn directory_listing_treats_symlinked_folders_as_directories() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(external.join("Nested")).expect("create linked target");
        std::fs::write(external.join("Nested/Hero.prefab"), b"prefab").expect("write asset");

        let link = workspace.join("Assets/Linked");
        if !create_dir_symlink_or_skip(&external, &link) {
            return;
        }

        let assets_entries =
            collect_dir_entries(&workspace.join("Assets"), "Assets", true).expect("list Assets");
        assert!(assets_entries
            .iter()
            .any(|entry| entry.rel_path == "Assets/Linked" && entry.is_dir));

        let workspace_str = workspace.to_string_lossy();
        let (target, normalized) = resolve_workspace_dir_target(&workspace_str, "Assets/Linked")
            .expect("resolve symlinked folder");
        assert_eq!(normalized, "Assets/Linked");

        let linked_entries =
            collect_dir_entries(&target, &normalized, true).expect("list symlinked folder");
        assert!(linked_entries
            .iter()
            .any(|entry| entry.rel_path == "Assets/Linked/Nested" && entry.is_dir));

        let (nested_target, nested_normalized) =
            resolve_workspace_dir_target(&workspace_str, "Assets/Linked/Nested")
                .expect("resolve nested symlinked folder path");
        assert_eq!(nested_normalized, "Assets/Linked/Nested");
        assert!(nested_target.is_dir());
    }

    #[test]
    fn directory_listing_rejects_non_asset_symlinked_folders() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-docs");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Secret.txt"), b"secret").expect("write external file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Docs")) {
            return;
        }

        let workspace_str = workspace.to_string_lossy();
        assert!(resolve_workspace_dir_target(&workspace_str, "Docs").is_err());

        let root_entries = collect_dir_entries(&workspace, "", true).expect("list workspace root");
        assert!(!root_entries.iter().any(|entry| entry.rel_path == "Docs"));
    }

    #[test]
    fn workspace_search_skips_non_asset_symlinked_folders() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-docs");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Secret.txt"), b"secret").expect("write external file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Docs")) {
            return;
        }

        let results =
            search_workspace_entries_in_dir(&workspace, "Secret", 100).expect("search workspace");
        assert!(!results
            .iter()
            .any(|entry| entry.rel_path == "Docs/Secret.txt"));
    }

    #[test]
    fn workspace_search_does_not_recurse_forever_through_symlink_cycle() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        std::fs::create_dir_all(workspace.join("Assets/Real")).expect("create assets dir");
        std::fs::write(workspace.join("Assets/Real/Hero.prefab"), "prefab")
            .expect("write asset file");

        let loop_link = workspace.join("Assets/Loop");
        if !create_dir_symlink_or_skip(&workspace, &loop_link) {
            return;
        }

        let results =
            search_workspace_entries_in_dir(&workspace, "Loop", 100).expect("search workspace");
        assert!(results
            .iter()
            .any(|entry| entry.rel_path == "Assets/Loop" && entry.is_dir));
        assert!(results.len() < 20);
    }
}
