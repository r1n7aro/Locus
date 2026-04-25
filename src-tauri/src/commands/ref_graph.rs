use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::asset_db::types::{guid_to_hex, parse_guid_hex, ScanPhase, ScanStats};
use crate::asset_db::{AssetDb, AssetDbState};
use crate::commands::asset::{
    write_persisted_last_scan_info, LastScanInfo, LastScanInfoState, ScanPhaseState,
};
use crate::error::AppError;
use crate::workspace::Workspace;
use crate::AssetDbWatcherHandle;

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
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<ScanStats, AppError> {
    let cwd = workspace.path.read().await.clone();
    let project_root = std::path::Path::new(&cwd);

    if !project_root.join("Assets").is_dir() {
        return Err("Not a Unity project (Assets/ not found)".to_string().into());
    }

    {
        let mut wh = watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = wh.take() {
            old.stop_and_join();
            eprintln!("[AssetDb] stopped previous watcher before rescan");
        }
    }

    // Drop the old AssetDb (and thus its SQLite Connection) BEFORE we let
    // the rebuild path inside `db::open_db` try to remove the on-disk DB.
    // SQLite holds an exclusive file lock on Windows for as long as a
    // Connection is alive; without this drop, schema-version-mismatch
    // rebuilds would fail with a sharing violation.
    {
        let mut g = ref_graph_state
            .0
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        *g = None;
    }

    // Mark scan as starting so any concurrent `asset_db_overview` call sees
    // `Scanning` immediately, even before the first phase callback fires.
    scan_phase_state.set(Some(ScanPhase::DirScan));

    let root = project_root.to_path_buf();
    let handle = app_handle.clone();
    let scan_started = std::time::Instant::now();
    // Clone the inner Arc so the closure can update the live phase from
    // inside `spawn_blocking` (the `State<...>` itself can't be moved across
    // the boundary).
    let phase_arc = scan_phase_state.0.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut graph = AssetDb::open(&root)?;
        let stats = graph.full_scan(|phase| {
            let _ = handle.emit("ref-graph-scan", phase);
            // Update the live tracker on every progress event. We don't store
            // `Done` here — completion is signalled by writing `None` from the
            // success branch below.
            if let Ok(mut g) = phase_arc.lock() {
                *g = Some(phase.clone());
            }
        })?;
        Ok::<(AssetDb, ScanStats), String>((graph, stats))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;

    match result {
        Ok((graph, scan_stats)) => {
            // Publish the new graph FIRST, then record the scan info. The reverse order
            // would create a window where `asset_db_overview` could observe
            // `lastScanAt = Some(now)` while `AssetDbState` is still `None`.
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = Some(graph);

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
            last_scan_info.set(scan_info.clone());
            if let Err(err) = write_persisted_last_scan_info(std::path::Path::new(&cwd), &scan_info)
            {
                eprintln!(
                    "[AssetDb] warning: failed to persist last successful scan info: {}",
                    err
                );
            }
            // Clear the live phase: success returns the UI to the steady
            // `Indexed` state.
            scan_phase_state.clear();

            let graph_arc = ref_graph_state.0.clone();
            let watcher_root = std::path::PathBuf::from(&cwd);
            match crate::asset_db::watcher::AssetDbWatcher::start(
                watcher_root,
                graph_arc,
                watcher_tuning.0.clone(),
            ) {
                Ok(w) => {
                    let mut wh = watcher_handle
                        .lock()
                        .map_err(|e| format!("Lock error: {}", e))?;
                    *wh = Some(w);
                    eprintln!("[AssetDb] incremental watcher started");
                }
                Err(e) => {
                    eprintln!("[AssetDb] warning: failed to start watcher: {}", e);
                }
            }

            Ok(scan_stats)
        }
        Err(e) => {
            eprintln!("[AssetDb] scan failed: {}", e);
            let scan_error = AppError::new("ref_graph.scan_failed", &e).retryable(true);
            let _ = app_handle.emit(
                "ref-graph-scan",
                &ScanPhase::Error {
                    error: scan_error.clone(),
                },
            );
            // Stick the error in `ScanPhaseState` so any later
            // `asset_db_overview` call can still report `Error` status even
            // after the failure event has been consumed.
            scan_phase_state.set(Some(ScanPhase::Error { error: scan_error }));
            Err(e.into())
        }
    }
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
