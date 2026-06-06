//! Asset page commands.
//!
//! Tauri command surface:
//!   - `asset_db_overview` — DB stats + 4-state status (Indexed / Scanning /
//!     None / Error), derived from `AssetDbState` + the live `ScanPhaseState`.
//!   - `search_workspace_assets` — filename + path-fragment search across
//!     `Assets/`, `Packages/`, `ProjectSettings/`, with a WalkDir fallback
//!     when the ref_graph DB is not loaded. Capped at 200 results.
//!   - `preview_workspace_asset` — single-file workspace preview. Dispatches
//!     to one of four payload shapes:
//!       * `text` — code/text extensions, streamed via `read_text_snippet`.
//!       * `binaryPreview` — image/psd/model under the per-kind size budget,
//!         served through the existing `BinaryCache` + `locus-binary` URI.
//!       * `binaryInfo` — metadata-only fallback (unknown kind, oversized,
//!         or read failure).
//!       * `structured` — Scene/Prefab and YAML Unity assets. Returns target
//!         metadata only; per-target inspector panels are fetched lazily.
//!   - `preview_workspace_asset_target` — companion command for `structured`
//!     payloads. Looks up a `WorkspacePreviewSession` by `previewKey` and
//!     builds a `SemanticTargetInspector` for the requested target id. Cache
//!     misses return a retryable `asset.preview.cache_miss` error.
//!   - `preview_workspace_asset_thumbnail` — asks Unity for a real asset
//!     preview image when the local parser cannot produce a visual thumbnail.
//!   - `get_watcher_tuning` / `set_watcher_tuning` — live tunables for the
//!     incremental ref_graph watcher (debounce + worker count).
//!
//! v1 simplifications worth knowing:
//!   - `WorkspacePreviewCache` is an 8-slot hand-rolled FIFO+LRU; sessions
//!     are evicted on workspace switch (`set_working_dir`) so a stale session
//!     can never be paired with a new project's `AssetDb`.
//!   - PrefabInstance targets in scene previews render the GameObject header
//!     only — proper source-prefab cross-resolution is out of scope for v1.
//!   - `ScanPhaseState` is process-only; the most recent successful scan
//!     snapshot is persisted per-workspace under `Library/Locus/`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::binary_cache::BinaryCache;
use crate::diff::context::{GuidResolver, SideContext, SideFileSource, SourceMode};
use crate::diff::semantic::inspector::build_doc_panel_pair_readonly;
use crate::diff::semantic::resolve_script_class_name_readonly;
use crate::diff::semantic::scene::{
    build_scene_side_data_readonly, build_workspace_scene_target_inspector, node_id_from_entry,
    SceneSemanticSideData,
};
use crate::diff::semantic::script::ScriptInfoCache;
// Aliased to dodge a name clash with the local `AssetPreviewPayload::BinaryPreview`
// variant — the variant name has to match its serde discriminator
// (`"binaryPreview"`), so we keep the diff layer's struct under a different
// identifier inside this module.
use crate::asset_db::types::{
    AssetRiskEntry, AssetRiskKind, DuplicateGuidOverview, ScanPhase, ScanStats,
};
use crate::asset_db::AssetDbState;
use crate::diff::types::{
    BinaryAssetRef, BinaryPreview as DiffBinaryPreview, InspectorPanelKind, SemanticLayout,
    SemanticTargetInspector, SemanticTreeNode, UnityAssetKind,
};
use crate::error::AppError;
use crate::unity_yaml::{UnityYamlDocs, UnityYamlFile, YamlDoc};
use crate::workspace::Workspace;
use crate::AssetDbWatcherHandle;

// ── LastScanInfo: latest successful scan snapshot for the active workspace ──

/// Information about the most recent successful `ref_graph_scan` invocation.
/// Stored in `LastScanInfoState` and mirrored to disk per workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastScanInfo {
    /// Wall-clock time the scan finished, as unix milliseconds.
    pub finished_at_unix_ms: u64,
    /// Total scan duration in milliseconds.
    pub duration_ms: u64,
    /// Stats reported by the scan.
    pub stats: ScanStats,
}

const LAST_SCAN_INFO_FILE: &str = "asset-scan-info.json";

fn last_scan_info_path(project_root: &Path) -> PathBuf {
    project_root
        .join("Library")
        .join("Locus")
        .join(LAST_SCAN_INFO_FILE)
}

pub fn read_persisted_last_scan_info(project_root: &Path) -> Result<Option<LastScanInfo>, String> {
    if project_root.as_os_str().is_empty() {
        return Ok(None);
    }

    let path = last_scan_info_path(project_root);
    if !path.is_file() {
        return Ok(None);
    }

    let raw = std::fs::read(&path)
        .map_err(|e| format!("Failed to read persisted asset scan info: {}", e))?;
    let info = serde_json::from_slice::<LastScanInfo>(&raw)
        .map_err(|e| format!("Failed to parse persisted asset scan info: {}", e))?;
    Ok(Some(info))
}

pub fn write_persisted_last_scan_info(
    project_root: &Path,
    info: &LastScanInfo,
) -> Result<(), String> {
    if project_root.as_os_str().is_empty() {
        return Ok(());
    }

    let path = last_scan_info_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create asset scan info directory: {}", e))?;
    }

    let raw = serde_json::to_vec_pretty(info)
        .map_err(|e| format!("Failed to serialize persisted asset scan info: {}", e))?;
    std::fs::write(&path, raw)
        .map_err(|e| format!("Failed to write persisted asset scan info: {}", e))
}

pub fn delete_persisted_last_scan_info(project_root: &Path) -> Result<(), String> {
    if project_root.as_os_str().is_empty() {
        return Ok(());
    }

    let path = last_scan_info_path(project_root);
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "Failed to delete persisted asset scan info at {}: {}",
            path.display(),
            err
        )),
    }
}

/// Tauri-managed state holding the most recent `LastScanInfo`, if any.
#[derive(Clone, Default)]
pub struct LastScanInfoState(pub Arc<Mutex<Option<LastScanInfo>>>);

impl LastScanInfoState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    /// Replace the stored value. Silently no-ops on lock poisoning.
    pub fn set(&self, info: LastScanInfo) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some(info);
        }
    }

    /// Snapshot the current value.
    pub fn snapshot(&self) -> Option<LastScanInfo> {
        self.0.lock().ok().and_then(|g| g.clone())
    }

    /// Reset to `None`. Called when the workspace switches so the new project
    /// does not inherit the previous project's scan timestamp.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = None;
        }
    }
}

// ── ScanPhaseState: live snapshot of the most recent scan phase ──
//
// Lets `asset_db_overview` reconstruct a 4-state UI even when the page opens
// mid-scan (or after the user missed the first `ref-graph-scan` event). The
// snapshot is `None` when no scan is running and no error is sticky; on error
// it holds `Some(ScanPhase::Error{..})` until the next scan starts; while
// scanning or verifying it holds the latest active phase. Completion events are
// intentionally never stored — completion goes back to None.
#[derive(Clone, Default)]
pub struct ScanPhaseState(pub Arc<Mutex<Option<ScanPhase>>>);

impl ScanPhaseState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    pub fn is_running_phase(phase: &ScanPhase) -> bool {
        matches!(
            phase,
            ScanPhase::DirScan
                | ScanPhase::MetaParse { .. }
                | ScanPhase::YamlParse { .. }
                | ScanPhase::DbWrite
                | ScanPhase::Reconcile { .. }
        )
    }

    pub fn try_begin_scan(&self) -> Result<bool, AppError> {
        let mut guard = self.0.lock().map_err(|e| {
            AppError::new("ref_graph.phase_lock_failed", format!("Lock error: {}", e))
        })?;
        if guard.as_ref().map(Self::is_running_phase).unwrap_or(false) {
            return Ok(false);
        }
        *guard = Some(ScanPhase::DirScan);
        Ok(true)
    }

    pub fn set(&self, phase: Option<ScanPhase>) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = phase;
        }
    }

    pub fn snapshot(&self) -> Option<ScanPhase> {
        self.0.lock().ok().and_then(|g| g.clone())
    }

    pub fn clear(&self) {
        self.set(None);
    }
}

#[derive(Clone)]
struct AssetDbReconcileTask {
    cwd: String,
    workspace_generation: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct AssetDbReconcileTaskState(Arc<Mutex<Option<AssetDbReconcileTask>>>);

impl AssetDbReconcileTaskState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, cwd: String, workspace_generation: u64) -> AssetDbReconcileRegistration {
        let task = AssetDbReconcileTask {
            cwd,
            workspace_generation,
            cancel: Arc::new(AtomicBool::new(false)),
        };

        if let Ok(mut guard) = self.0.lock() {
            if let Some(previous) = guard.replace(task.clone()) {
                previous.cancel.store(true, Ordering::Relaxed);
                eprintln!(
                    "[AssetDb] replaced active background reconcile for {} generation {}; cancellation requested",
                    previous.cwd, previous.workspace_generation
                );
            }
        }

        AssetDbReconcileRegistration {
            state: self.clone(),
            task,
        }
    }

    pub fn cancel_current(&self, reason: &str) {
        let task = match self.0.lock() {
            Ok(mut guard) => guard.take(),
            Err(error) => {
                eprintln!(
                    "[AssetDb] failed to lock background reconcile state for cancellation: {error}"
                );
                None
            }
        };

        if let Some(task) = task {
            task.cancel.store(true, Ordering::Relaxed);
            eprintln!(
                "[AssetDb] cancelling background reconcile for {} generation {} ({})",
                task.cwd, task.workspace_generation, reason
            );
        }
    }
}

pub struct AssetDbReconcileRegistration {
    state: AssetDbReconcileTaskState,
    task: AssetDbReconcileTask,
}

impl AssetDbReconcileRegistration {
    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        self.task.cancel.clone()
    }
}

impl Drop for AssetDbReconcileRegistration {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.0.lock() {
            if guard
                .as_ref()
                .map(|current| Arc::ptr_eq(&current.cancel, &self.task.cancel))
                .unwrap_or(false)
            {
                *guard = None;
            }
        }
    }
}

// ── AssetDbOverview payload (returned to frontend) ──

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetDbStatus {
    /// `AssetDb` is loaded and no scan is in progress.
    Indexed,
    /// A scan is currently running. Derived from `ScanPhaseState` carrying a
    /// `DirScan | MetaParse | YamlParse | DbWrite` variant.
    Scanning,
    /// No `AssetDb` is loaded and no scan is in progress.
    None,
    /// The most recent scan attempt failed and no successful scan has run
    /// since. Derived from `ScanPhaseState` carrying a sticky `Error` variant.
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetKindCount {
    /// camelCase string identifier (e.g. "scene", "prefab", "genericAsset").
    pub kind: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDbOverview {
    pub status: AssetDbStatus,
    pub nodes: u64,
    pub edges: u64,
    /// Total on-disk footprint of the asset DB files (`locus.db` + WAL/SHM).
    pub db_bytes: u64,
    /// Total size of indexed asset files currently present on disk.
    pub asset_bytes: u64,
    /// Unix milliseconds; absent when no successful scan history is available
    /// for the current workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_duration_ms: Option<u64>,
    /// Detailed stats from the most recent successful scan for this
    /// workspace, restored from disk on startup and workspace switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_stats: Option<ScanStats>,
    pub watcher_running: bool,
    /// Number of pending dirty assets waiting to be re-indexed by the
    /// incremental watcher worker. `0` when the queue is empty or when the
    /// watcher is not running.
    pub watcher_queue_len: u64,
    /// Workspace-relative path of the asset the watcher worker is currently
    /// processing, or `None` when the worker is idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_current_file: Option<String>,
    pub by_kind: Vec<AssetKindCount>,
    pub asset_risks: Vec<AssetRiskEntry>,
    pub duplicate_guids: DuplicateGuidOverview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_guid_report_path: Option<String>,
    /// Latest snapshot from the live scan-phase tracker. Present whenever a
    /// scan is running or the last attempt errored. Lets the frontend rebuild
    /// the "scanning… 6.2k/12k" or "scan failed" status when the page is
    /// opened mid-scan or after missing the first event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_scan_phase: Option<ScanPhase>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDbLightStatus {
    pub status: AssetDbStatus,
    pub nodes: u64,
    pub edges: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_stats: Option<ScanStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_scan_phase: Option<ScanPhase>,
}

// ── Command: asset_db_overview ──

/// Classify a `ScanPhase` snapshot for status-derivation purposes.
fn classify_scan_phase(phase: &ScanPhase) -> AssetDbStatus {
    match phase {
        ScanPhase::DirScan
        | ScanPhase::MetaParse { .. }
        | ScanPhase::YamlParse { .. }
        | ScanPhase::DbWrite
        | ScanPhase::Reconcile { .. } => AssetDbStatus::Scanning,
        ScanPhase::Error { .. } => AssetDbStatus::Error,
        // Completion is never stored in `ScanPhaseState` (we set to None on
        // success), but the match needs to be exhaustive. Treat it as idle.
        ScanPhase::Done { .. } | ScanPhase::ReconcileDone => AssetDbStatus::Indexed,
    }
}

fn optional_file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn optional_file_modified_unix_ms(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn db_footprint_bytes(project_root: &Path) -> u64 {
    if project_root.as_os_str().is_empty() {
        return 0;
    }
    let db_path = project_root.join("Library").join("Locus").join("locus.db");
    optional_file_size(&db_path)
        + optional_file_size(&db_path.with_extension("db-wal"))
        + optional_file_size(&db_path.with_extension("db-shm"))
}

fn db_last_updated_unix_ms(project_root: &Path) -> Option<u64> {
    if project_root.as_os_str().is_empty() {
        return None;
    }

    let db_path = project_root.join("Library").join("Locus").join("locus.db");
    [
        optional_file_modified_unix_ms(&db_path),
        optional_file_modified_unix_ms(&db_path.with_extension("db-wal")),
        optional_file_modified_unix_ms(&db_path.with_extension("db-shm")),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn duplicate_guid_report_rel_path(project_root: &Path) -> Option<String> {
    let report_path = project_root
        .join("Temp")
        .join("Locus")
        .join("duplicate-guid-report.txt");
    if !report_path.is_file() {
        return None;
    }

    report_path
        .strip_prefix(project_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn parse_failure_report_rel_path(project_root: &Path) -> Option<String> {
    let report_path = project_root
        .join("Temp")
        .join("Locus")
        .join("parse-failures-report.txt");
    if !report_path.is_file() {
        return None;
    }

    report_path
        .strip_prefix(project_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn report_path_to_rel(project_root: &Path, report_path: &Path) -> Result<String, AppError> {
    report_path
        .strip_prefix(project_root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .map_err(|_| {
            AppError::new(
                "asset.risk.report_outside_workspace",
                format!(
                    "Risk report resolves outside workspace: {}",
                    report_path.display()
                ),
            )
        })
}

#[tauri::command]
pub async fn asset_db_overview(
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase: State<'_, ScanPhaseState>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<AssetDbOverview, AppError> {
    let (watcher_running, watcher_queue_len, watcher_current_file) = watcher_handle
        .lock()
        .map(|guard| match &*guard {
            Some(w) => (true, w.queue_len() as u64, w.current_file()),
            None => (false, 0u64, None),
        })
        .unwrap_or((false, 0, None));
    let working_dir = workspace.path.read().await.clone();

    let project_root = Path::new(&working_dir);
    let last = last_scan_info.snapshot();
    let last_scan_at = last
        .as_ref()
        .map(|i| i.finished_at_unix_ms)
        .or_else(|| db_last_updated_unix_ms(project_root));
    let last_scan_duration_ms = last.as_ref().map(|i| i.duration_ms);
    let last_scan_stats = last.as_ref().map(|i| i.stats.clone());

    let phase_snapshot = scan_phase.snapshot();
    let phase_derived_status = phase_snapshot.as_ref().map(classify_scan_phase);

    let guard = ref_graph_state
        .0
        .lock()
        .map_err(|e| AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e)))?;

    // Status precedence:
    //   1. Live scan phase: `Scanning` or sticky `Error` always wins.
    //   2. Otherwise: `Indexed` if AssetDb is loaded, else `None`.
    // This ordering matters when a scan is running but AssetDb is still
    // pointing at the old graph instance — we want the UI to say "scanning",
    // not the stale "indexed".
    let derived_status = match phase_derived_status {
        Some(AssetDbStatus::Scanning) => AssetDbStatus::Scanning,
        Some(AssetDbStatus::Error) => AssetDbStatus::Error,
        _ => match &*guard {
            Some(_) => AssetDbStatus::Indexed,
            None => AssetDbStatus::None,
        },
    };

    match &*guard {
        None => Ok(AssetDbOverview {
            status: derived_status,
            nodes: 0,
            edges: 0,
            db_bytes: db_footprint_bytes(project_root),
            asset_bytes: 0,
            last_scan_at,
            last_scan_duration_ms,
            last_scan_stats,
            watcher_running,
            watcher_queue_len,
            watcher_current_file: watcher_current_file.clone(),
            by_kind: Vec::new(),
            asset_risks: Vec::new(),
            duplicate_guids: DuplicateGuidOverview::default(),
            duplicate_guid_report_path: None,
            current_scan_phase: phase_snapshot,
        }),
        Some(graph) => {
            let (nodes, edges) = graph
                .get_stats()
                .map_err(|e| AppError::new("ref_graph.stats_failed", e))?;
            let asset_bytes = graph
                .get_asset_size_bytes()
                .map_err(|e| AppError::new("ref_graph.asset_size_failed", e))?;
            let kind_rows = graph
                .get_kind_counts()
                .map_err(|e| AppError::new("ref_graph.kind_counts_failed", e))?;
            let by_kind = kind_rows
                .into_iter()
                .map(|(kind, count)| AssetKindCount {
                    kind: kind.camel_str().to_string(),
                    count,
                })
                .collect();
            let asset_risks = graph
                .get_asset_risks()
                .map_err(|e| AppError::new("ref_graph.asset_risks_failed", e))?;
            let duplicate_guids = graph
                .get_duplicate_guid_overview()
                .map_err(|e| AppError::new("ref_graph.duplicate_guid_stats_failed", e))?;
            let duplicate_guid_report_path = duplicate_guid_report_rel_path(graph.project_root());
            Ok(AssetDbOverview {
                status: derived_status,
                nodes,
                edges,
                db_bytes: db_footprint_bytes(graph.project_root()),
                asset_bytes,
                last_scan_at,
                last_scan_duration_ms,
                last_scan_stats,
                watcher_running,
                watcher_queue_len,
                watcher_current_file,
                by_kind,
                asset_risks,
                duplicate_guids,
                duplicate_guid_report_path,
                current_scan_phase: phase_snapshot,
            })
        }
    }
}

#[tauri::command]
pub async fn asset_db_light_status(
    ref_graph_state: State<'_, AssetDbState>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase: State<'_, ScanPhaseState>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<AssetDbLightStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let project_root = Path::new(&working_dir);
    let last = last_scan_info.snapshot();
    let last_scan_at = last
        .as_ref()
        .map(|i| i.finished_at_unix_ms)
        .or_else(|| db_last_updated_unix_ms(project_root));
    let last_scan_duration_ms = last.as_ref().map(|i| i.duration_ms);
    let last_scan_stats = last.as_ref().map(|i| i.stats.clone());

    let phase_snapshot = scan_phase.snapshot();
    let phase_derived_status = phase_snapshot.as_ref().map(classify_scan_phase);

    let guard = ref_graph_state
        .0
        .lock()
        .map_err(|e| AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e)))?;

    let (db_loaded, nodes, edges) = match &*guard {
        Some(graph) => {
            let (nodes, edges) = graph
                .get_stats()
                .map_err(|e| AppError::new("ref_graph.stats_failed", e))?;
            (true, nodes, edges)
        }
        None => (false, 0, 0),
    };

    let status = match phase_derived_status {
        Some(AssetDbStatus::Scanning) => AssetDbStatus::Scanning,
        Some(AssetDbStatus::Error) => AssetDbStatus::Error,
        _ if db_loaded => AssetDbStatus::Indexed,
        _ => AssetDbStatus::None,
    };

    Ok(AssetDbLightStatus {
        status,
        nodes,
        edges,
        last_scan_at,
        last_scan_duration_ms,
        last_scan_stats,
        current_scan_phase: phase_snapshot,
    })
}

#[tauri::command]
pub async fn asset_risk_report(
    kind: AssetRiskKind,
    ref_graph_state: State<'_, AssetDbState>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Err(AppError::new(
            "asset.risk.no_workspace",
            "Open a Unity workspace before viewing asset risk details.",
        ));
    }
    let project_root = Path::new(&working_dir);

    match kind {
        AssetRiskKind::DuplicateGuids => {
            duplicate_guid_report_rel_path(project_root).ok_or_else(|| {
                AppError::new(
                    "asset.risk.report_missing",
                    "Duplicate GUID details are unavailable. Run a rescan to rebuild the report.",
                )
            })
        }
        AssetRiskKind::ParseFailures => {
            parse_failure_report_rel_path(project_root).ok_or_else(|| {
                AppError::new(
                    "asset.risk.report_missing",
                    "Parse failure details are unavailable. Run a rescan to rebuild the report.",
                )
            })
        }
        AssetRiskKind::BrokenReferences | AssetRiskKind::MissingScripts => {
            let guard = ref_graph_state.0.lock().map_err(|e| {
                AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e))
            })?;
            let graph = guard.as_ref().ok_or_else(|| {
                AppError::new(
                    "ref_graph.not_loaded",
                    "Asset database is unavailable. Run a rescan to rebuild it.",
                )
            })?;
            let report_path = graph
                .build_missing_reference_report(kind == AssetRiskKind::MissingScripts)
                .map_err(|e| AppError::new("asset.risk.report_failed", e))?
                .ok_or_else(|| {
                    AppError::new(
                        "asset.risk.none",
                        "No detail report is available for this asset risk.",
                    )
                })?;
            report_path_to_rel(project_root, &report_path)
        }
    }
}

// ── Watcher tuning commands ──
//
// Live-tunable knobs for the incremental watcher: per-item debounce (lower =
// more aggressive but more CPU) and active worker count (capped to
// `MAX_WORKER_THREADS`). Both reads and writes touch the shared atomics in
// `WatcherTuning`, so updates take effect on the next worker iteration without
// restarting the watcher.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherTuningPayload {
    pub debounce_ms: u64,
    pub worker_count: u32,
    pub max_worker_count: u32,
}

#[tauri::command]
pub async fn get_watcher_tuning(
    tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<WatcherTuningPayload, AppError> {
    let (debounce_ms, worker_count) = tuning.0.snapshot();
    Ok(WatcherTuningPayload {
        debounce_ms,
        worker_count: worker_count as u32,
        max_worker_count: crate::asset_db::watcher::MAX_WORKER_THREADS as u32,
    })
}

#[tauri::command]
pub async fn set_watcher_tuning(
    debounce_ms: u64,
    worker_count: u32,
    tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
) -> Result<WatcherTuningPayload, AppError> {
    // Clamp inputs defensively. The frontend slider already restricts to
    // [0, 2000] / [1, MAX_WORKER_THREADS], but we don't trust that.
    let clamped_debounce = debounce_ms.min(5_000);
    let clamped_workers =
        (worker_count as usize).clamp(1, crate::asset_db::watcher::MAX_WORKER_THREADS);
    tuning.0.set(clamped_debounce, clamped_workers);
    Ok(WatcherTuningPayload {
        debounce_ms: clamped_debounce,
        worker_count: clamped_workers as u32,
        max_worker_count: crate::asset_db::watcher::MAX_WORKER_THREADS as u32,
    })
}

// ── search_workspace_assets ──

/// Maximum results returned to the frontend. Anything beyond this is
/// truncated; the frontend renders a "(truncated)" hint when this is hit.
const SEARCH_RESULT_LIMIT: usize = 200;
const SEARCH_RESULT_MAX_LIMIT: usize = 5000;

/// Maximum recursion depth for the WalkDir fallback. Deep enough for typical
/// Unity projects, shallow enough that pathological junk dirs don't blow up.
const FALLBACK_WALK_MAX_DEPTH: usize = 12;

/// Directories the WalkDir fallback always skips, regardless of which root we
/// are walking. These are either VCS noise, build outputs, or directories that
/// would never contain editable assets.
const SKIPPED_DIR_NAMES: &[&str] = &[
    ".git",
    "Library",
    "Temp",
    "obj",
    "bin",
    "node_modules",
    ".vs",
    ".idea",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AssetSearchRoot {
    Assets,
    Packages,
    ProjectSettings,
}

impl AssetSearchRoot {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "Assets" => Some(Self::Assets),
            "Packages" => Some(Self::Packages),
            "ProjectSettings" => Some(Self::ProjectSettings),
            _ => None,
        }
    }

    fn dir_name(&self) -> &'static str {
        match self {
            Self::Assets => "Assets",
            Self::Packages => "Packages",
            Self::ProjectSettings => "ProjectSettings",
        }
    }

    /// Translate to the low-level `ref_graph::types::AssetRoot` used by the
    /// DB layer. The two enums are kept in lockstep on purpose — the command
    /// layer owns the IPC-facing camelCase serialization; the db layer owns
    /// the storage shape; neither imports the other.
    fn to_db_root(self) -> crate::asset_db::types::AssetRoot {
        match self {
            Self::Assets => crate::asset_db::types::AssetRoot::Assets,
            Self::Packages => crate::asset_db::types::AssetRoot::Packages,
            Self::ProjectSettings => crate::asset_db::types::AssetRoot::ProjectSettings,
        }
    }

    fn from_db_root(r: crate::asset_db::types::AssetRoot) -> Option<Self> {
        match r {
            crate::asset_db::types::AssetRoot::Assets => Some(Self::Assets),
            crate::asset_db::types::AssetRoot::Packages => Some(Self::Packages),
            crate::asset_db::types::AssetRoot::ProjectSettings => Some(Self::ProjectSettings),
            crate::asset_db::types::AssetRoot::Other => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetSearchSource {
    /// Result came from the ref_graph SQLite index.
    AssetDb,
    /// Result came from a filesystem walk fallback. The frontend uses this to
    /// surface the "index not built — results limited" hint.
    Filesystem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSearchResult {
    /// Workspace-relative path with forward slashes.
    pub path: String,
    /// Filename including extension.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_key: Option<String>,
    pub root: AssetSearchRoot,
    /// camelCase asset kind string. For ref_graph results this is
    /// `AssetKind::camel_str()`; for filesystem-fallback results it falls back
    /// to extension-derived AssetKind so the frontend can still pick an icon.
    pub kind: String,
    /// Optional user-facing type label for ScriptableObject assets. When
    /// present, the frontend prefers this over the coarse `kind` badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_search: Option<String>,
    pub is_sub_asset: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Higher = better match. Used for sorting; not exposed for ranking math.
    pub match_score: i32,
    pub source: AssetSearchSource,
}

/// Match score tiers. Higher wins. Used only by the WalkDir fallback path
/// (`ref_graph` unavailable, or always for `ProjectSettings/`). The
/// ref-graph-backed branch ranks rows in SQL.
const SCORE_FILENAME_EXACT: i32 = 4;
const SCORE_FILENAME_PREFIX: i32 = 3;
const SCORE_FILENAME_CONTAINS: i32 = 2;
const SCORE_PATH_CONTAINS: i32 = 1;

/// Compute a single-term match score against a workspace-relative path.
fn walker_score_term(rel_path: &str, term_lower: &str) -> Option<i32> {
    if term_lower.is_empty() {
        return None;
    }
    let path_lower = rel_path.to_ascii_lowercase();
    let filename_lower = path_lower.rsplit('/').next().unwrap_or(path_lower.as_str());
    let filename_stem = filename_lower
        .rsplit_once('.')
        .map(|(stem, _ext)| stem)
        .unwrap_or(filename_lower);

    if filename_lower == term_lower || filename_stem == term_lower {
        Some(SCORE_FILENAME_EXACT)
    } else if filename_lower.starts_with(term_lower) {
        Some(SCORE_FILENAME_PREFIX)
    } else if filename_lower.contains(term_lower) {
        Some(SCORE_FILENAME_CONTAINS)
    } else if path_lower.contains(term_lower) {
        Some(SCORE_PATH_CONTAINS)
    } else {
        None
    }
}

/// AND-style match across multiple bare terms.
fn walker_score_match(rel_path: &str, query_terms: &[String]) -> Option<i32> {
    if query_terms.is_empty() {
        return None;
    }
    let mut total = 0;
    for term in query_terms {
        total += walker_score_term(rel_path, term)?;
    }
    Some(total)
}

/// Return the [`AssetSearchRoot`] that owns `rel_path`, or `None` if the path
/// does not begin with one of the three known roots.
fn root_for_rel_path(rel_path: &str) -> Option<AssetSearchRoot> {
    if let Some(rest) = rel_path.strip_prefix("Assets") {
        if rest.is_empty() || rest.starts_with('/') {
            return Some(AssetSearchRoot::Assets);
        }
    }
    if let Some(rest) = rel_path.strip_prefix("Packages") {
        if rest.is_empty() || rest.starts_with('/') {
            return Some(AssetSearchRoot::Packages);
        }
    }
    if let Some(rest) = rel_path.strip_prefix("ProjectSettings") {
        if rest.is_empty() || rest.starts_with('/') {
            return Some(AssetSearchRoot::ProjectSettings);
        }
    }
    None
}

/// Walk one root directory under `cwd` and emit search results matching
/// `query_lower`. Used as the fallback when ref_graph is unavailable, and
/// always used for `ProjectSettings/`.
fn walk_root_for_search(
    cwd: &Path,
    root: AssetSearchRoot,
    query_terms: &[String],
    out: &mut Vec<AssetSearchResult>,
) {
    let root_dir = cwd.join(root.dir_name());
    if !root_dir.is_dir() {
        return;
    }
    let walker = WalkDir::new(&root_dir)
        .max_depth(FALLBACK_WALK_MAX_DEPTH)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| {
            // Skip well-known noise dirs at any depth.
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().is_dir() && SKIPPED_DIR_NAMES.iter().any(|d| *d == name) {
                return false;
            }
            true
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        // Skip Unity .meta sidecars in the fallback — they would double every
        // result and the user almost never wants to open them directly.
        if abs.extension().and_then(|s| s.to_str()) == Some("meta") {
            continue;
        }
        let Ok(rel) = abs.strip_prefix(cwd) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let Some(score) = walker_score_match(&rel_str, query_terms) else {
            continue;
        };
        let name = abs
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = abs
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let kind = crate::asset_db::types::AssetKind::from_ext(&ext)
            .camel_str()
            .to_string();
        out.push(AssetSearchResult {
            path: rel_str,
            name,
            guid: None,
            file_id: None,
            object_key: None,
            root,
            kind,
            type_label: None,
            type_search: None,
            is_sub_asset: false,
            target_id: None,
            match_score: score,
            source: AssetSearchSource::Filesystem,
        });
    }
}

fn is_script_ref_filter_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.starts_with("component:")
        || lower.starts_with("script:")
        || lower.starts_with("uses:")
        || lower.starts_with("inherits:")
}

fn extract_script_ref_terms(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in query.split_whitespace() {
        if !is_script_ref_filter_token(token) {
            continue;
        }
        let Some((_, value)) = token.split_once(':') else {
            continue;
        };
        let term = value.trim().trim_matches('"').to_ascii_lowercase();
        if term.is_empty() || !seen.insert(term.clone()) {
            continue;
        }
        out.push(term);
    }
    out
}

fn strip_script_ref_filters(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|token| !is_script_ref_filter_token(token))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strip the known DSL prefixes from a query and return the remaining bare
/// terms. Spaces behave as AND on the asset page, so bare terms must stay
/// split instead of being collapsed into one phrase.
fn extract_bare_terms(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in query.split_whitespace() {
        if token.starts_with("t:")
            || token.starts_with("n:")
            || token.starts_with("n=")
            || token.starts_with("n^")
            || token.starts_with("n$")
            || token.starts_with("under:")
            || token.starts_with("guid:")
            || token.to_ascii_lowercase().starts_with("fileid:")
            || is_script_ref_filter_token(token)
        {
            continue;
        }
        let lowered = token.to_ascii_lowercase();
        if lowered.is_empty() || !seen.insert(lowered.clone()) {
            continue;
        }
        out.push(lowered);
    }
    out
}

fn ui_score_match(row: &crate::asset_db::AssetSearchRowDb, bare_terms: &[String]) -> i32 {
    if bare_terms.is_empty() {
        return SCORE_FILENAME_EXACT;
    }

    let path_lower = row.path.to_ascii_lowercase();
    let mut score = 0;
    for term in bare_terms {
        if row.script_class_lower == *term || row.type_lower == *term || row.name_lower == *term {
            score += SCORE_FILENAME_EXACT;
        } else if row.script_class_lower.starts_with(term) || row.name_lower.starts_with(term) {
            score += SCORE_FILENAME_PREFIX;
        } else if row.type_search.contains(term) || row.file_name_lower.contains(term) {
            score += SCORE_FILENAME_CONTAINS;
        } else if path_lower.contains(term) {
            score += SCORE_PATH_CONTAINS;
        }
    }
    score
}

#[tauri::command]
pub async fn search_workspace_assets(
    query: String,
    roots: Vec<String>,
    limit: Option<usize>,
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
) -> Result<Vec<AssetSearchResult>, AppError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let script_ref_terms = extract_script_ref_terms(trimmed);
    let search_query = strip_script_ref_filters(trimmed);
    // For walker-fallback substring matching and the UI score tier we want
    // ONLY the bare-text portion of the query — matching against literal
    // `t:prefab` would never hit anything.
    let bare_terms = extract_bare_terms(&search_query);

    // Parse the requested root set; an unknown name is silently dropped.
    let requested_roots: Vec<AssetSearchRoot> = roots
        .iter()
        .filter_map(|s| AssetSearchRoot::from_str(s.as_str()))
        .collect();
    if requested_roots.is_empty() {
        return Ok(Vec::new());
    }
    let result_limit = limit
        .unwrap_or(SEARCH_RESULT_LIMIT)
        .clamp(1, SEARCH_RESULT_MAX_LIMIT);

    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(Vec::new());
    }
    let cwd_path = PathBuf::from(&cwd);
    if !cwd_path.is_dir() {
        return Err(AppError::new(
            "asset.search.invalid_workspace",
            format!("Workspace directory not found: {}", cwd),
        ));
    }

    // ── Phase 1: ref_graph-backed roots (Assets, Packages) ─────────────
    // Push the search down into SQLite. The DB query path uses derived
    // columns + composite indexes for structured predicates, FTS5 trigram
    // for free-text substring search, and a stem-prefix range for short
    // (<3 char) inputs that the trigram tokenizer can't help with.
    let want_index_root = requested_roots
        .iter()
        .any(|r| matches!(r, AssetSearchRoot::Assets | AssetSearchRoot::Packages));

    let index_results: Option<Vec<crate::asset_db::AssetSearchRowDb>> = if want_index_root {
        let guard = ref_graph_state
            .0
            .lock()
            .map_err(|e| AppError::new("ref_graph.lock_failed", format!("Lock error: {}", e)))?;
        match &*guard {
            Some(graph) => {
                let db_roots: Vec<crate::asset_db::types::AssetRoot> = requested_roots
                    .iter()
                    .filter(|r| !matches!(r, AssetSearchRoot::ProjectSettings))
                    .map(|r| r.to_db_root())
                    .collect();
                let search_limit = if script_ref_terms.is_empty() {
                    result_limit
                } else {
                    SEARCH_RESULT_MAX_LIMIT
                };
                let mut rows = graph
                    .search_assets_for_command(&search_query, &db_roots, search_limit as u32)
                    .map_err(|e| AppError::new("ref_graph.search_for_command_failed", e))?;
                if !script_ref_terms.is_empty() {
                    let ref_paths: HashSet<String> = graph
                        .find_asset_paths_referencing_script_terms(&script_ref_terms)
                        .map_err(|e| AppError::new("ref_graph.script_ref_search_failed", e))?
                        .into_iter()
                        .collect();
                    rows.retain(|row| ref_paths.contains(&row.path));
                }
                Some(rows)
            }
            None if script_ref_terms.is_empty() => None,
            None => Some(Vec::new()),
        }
    } else {
        None
    };

    let mut results: Vec<AssetSearchResult> = Vec::new();

    if let Some(rows) = index_results {
        // SQL has already filtered + sorted by relevance against the bare
        // query. Translate row shape and emit IN ORDER — do NOT re-sort
        // below, that would clobber the SQL ranking.
        for row in rows {
            let Some(ui_root) = AssetSearchRoot::from_db_root(row.root) else {
                continue;
            };
            let name = row.name.clone();
            // Coarse UI score tier computed against the BARE token only —
            // matching the whole raw query (incl. `t:prefab`) would collapse
            // every row to the lowest tier. Purely cosmetic; ordering is
            // already locked in by the SQL ORDER BY.
            let score = ui_score_match(&row, &bare_terms);
            results.push(AssetSearchResult {
                path: row.path,
                name,
                guid: Some(crate::asset_db::types::guid_to_hex(&row.guid)),
                file_id: row.file_id,
                object_key: Some(row.object_key),
                root: ui_root,
                kind: row.kind.camel_str().to_string(),
                type_label: if row.is_sub_asset {
                    Some(row.type_name)
                } else if row.kind == crate::asset_db::types::AssetKind::GenericAsset {
                    row.script_class_name
                } else {
                    None
                },
                type_search: (!row.type_search.trim().is_empty()).then_some(row.type_search),
                is_sub_asset: row.is_sub_asset,
                target_id: row.target_id,
                match_score: score,
                source: AssetSearchSource::AssetDb,
            });
        }
    } else if want_index_root {
        // ref_graph not built yet — fall back to filesystem walk for the
        // index-backed roots. Walker uses the bare query so DSL prefixes
        // (which the walker can't honor) don't poison substring matching.
        if requested_roots.contains(&AssetSearchRoot::Assets) {
            walk_root_for_search(
                &cwd_path,
                AssetSearchRoot::Assets,
                &bare_terms,
                &mut results,
            );
        }
        if requested_roots.contains(&AssetSearchRoot::Packages) {
            walk_root_for_search(
                &cwd_path,
                AssetSearchRoot::Packages,
                &bare_terms,
                &mut results,
            );
        }
    }

    // ── Phase 2: ProjectSettings always uses the walker ──
    if script_ref_terms.is_empty() && requested_roots.contains(&AssetSearchRoot::ProjectSettings) {
        walk_root_for_search(
            &cwd_path,
            AssetSearchRoot::ProjectSettings,
            &bare_terms,
            &mut results,
        );
    }

    // ── Phase 3: dedupe by path, preserving first-seen order ──
    // Index rows come first (already in SQL relevance order); walker rows
    // are appended after. We keep the FIRST occurrence of each path so
    // index ordering wins over walker ordering.
    let mut seen: HashSet<String> = HashSet::new();
    results.retain(|r| {
        let key = r
            .object_key
            .as_ref()
            .cloned()
            .unwrap_or_else(|| r.path.clone());
        seen.insert(key)
    });

    if results.len() > result_limit {
        results.truncate(result_limit);
    }
    Ok(results)
}

// ── preview_workspace_asset (Slice 3a: text dispatch only, others stubbed) ──

/// Maximum lines a text snippet may contain. Matches the existing
/// `preview_workspace_file` budget so the asset page and chat hover popover
/// behave the same way for the same file.
const TEXT_SNIPPET_MAX_LINES: usize = 200;
/// Hard byte cap for the snippet to defend against very long lines.
const TEXT_SNIPPET_MAX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetTextPreview {
    /// Snippet content. May be truncated; see `truncated`.
    pub snippet: String,
    /// `true` when the file had more lines than `TEXT_SNIPPET_MAX_LINES`, OR
    /// when the byte budget was exhausted before reaching the line cap.
    pub truncated: bool,
    /// Total lines in the file (informational; lets the frontend display
    /// "showing 200 / 1245 lines").
    pub total_lines: u32,
    /// Lowercase language identifier (e.g. "csharp", "json", "yaml").
    /// `None` when the extension does not map to a known language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTexturePreviewMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_is_transparency: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBinaryMeta {
    /// Workspace-relative path with forward slashes.
    pub path: String,
    /// Filename including extension.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Lowercase extension without leading dot, or empty string if none.
    pub ext: String,
    /// Hex-encoded GUID resolved via `AssetDb::resolve_guid_by_path`, or
    /// `None` when ref_graph is not loaded or the asset has no `.meta`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    /// Best-effort Unity importer hints from the sidecar `.meta` file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unity_texture: Option<UnityTexturePreviewMeta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetThumbnailPreview {
    pub asset_path: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPreviewFrame {
    pub asset_path: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pan_x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pan_y: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pan_z: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetPreviewFrameCacheMeta {
    asset_path: String,
    source_mtime_ms: u64,
    width: u32,
    height: u32,
    mime_type: String,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pan_x: f32,
    pan_y: f32,
    pan_z: f32,
}

/// Lightweight target metadata returned in the first `structured` payload.
/// Carries enough info for the frontend to render a hierarchy / target list
/// without any inspector data — the actual panels arrive via the companion
/// `preview_workspace_asset_target` command keyed by `(previewKey, id)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetTargetMeta {
    /// Stable target id, of the form `"doc:<fileId>"`. Frontend passes this
    /// back to `preview_workspace_asset_target`.
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// Discriminated union returned by `preview_workspace_asset`.
///
/// Variants:
/// - `text` — file is a text/code asset; payload carries a snippet.
/// - `binaryPreview` — file is a renderable binary (image/psd/model) under
///   the per-kind size budget. Wraps the existing `crate::diff::types::
///   BinaryPreview` struct so the frontend `BinaryPreviewHost` component can
///   consume it directly with `mode="neutral"`. The `before` slot carries
///   the asset bytes; `after` is `None` because there is no diff partner.
/// - `binaryInfo` — fallback metadata-only card. Used when (a) the file is
///   binary but the kind is unknown or oversized, or (b) the file is a
///   structured Unity asset awaiting Slice 3d (scene/prefab). Frontend renders
///   this as a small info card with no preview surface.
/// - `structured` — file is a YAML Unity asset (Slice 3c: material /
///   scriptableObject / animationClip / animatorController / genericYaml).
///   First-packet semantics: NO panels included. The frontend uses
///   `targets[]` to render a hierarchy and fetches each target's
///   `SemanticTargetInspector` on click via
///   `preview_workspace_asset_target(previewKey, targetId)`.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssetPreviewPayload {
    Text(AssetTextPreview),
    BinaryPreview {
        preview: DiffBinaryPreview,
        meta: AssetBinaryMeta,
    },
    BinaryInfo {
        meta: AssetBinaryMeta,
    },
    Structured {
        /// Cache key for the live `WorkspacePreviewSession`. Frontend passes
        /// this back to `preview_workspace_asset_target`.
        preview_key: String,
        layout: SemanticLayout,
        /// Hierarchy nodes for the left-side tree pane. For YAML asset
        /// preview (Slice 3c) this is currently a flat list — one node per
        /// top-level YamlDoc. Slice 3d adds the proper scene/prefab tree.
        tree: Vec<SemanticTreeNode>,
        /// Lightweight target metadata. Frontend uses these to populate the
        /// inspector selector and to call `preview_workspace_asset_target`.
        targets: Vec<AssetTargetMeta>,
    },
}

// ── WorkspacePreviewSession + WorkspacePreviewCache ──
//
// A single parsed workspace asset, kept fully owned so it can live in the
// process-wide preview cache without lifetime entanglement. Each session is
// produced by `preview_workspace_asset` (Slice 3c+) and consumed by
// `preview_workspace_asset_target` to build per-target inspector panels on
// demand.
//
// Construction is the expensive step (read file → parse YAML → resolve script
// classes). Per-target inspector building reuses the parsed docs, so a Scene
// with 200 GameObjects only pays the parse cost once.
/// Parsed YAML payload backing a [`WorkspacePreviewSession`]. Two
/// variants pin the cost of parsing to what the consumer actually needs:
///
/// - `Flat`: only docs + lines. Used by flat asset previews
///   (`.asset` / `.controller` / `.mat` / `.anim`) where downstream code
///   never reads hierarchy or component indices. Avoids paying for
///   `build_go_tree` + index passes that would be discarded.
/// - `Scene`: docs + lines + hierarchy + per-GO component index. Used
///   by scene/prefab previews where the inspector lookups need O(1)
///   component navigation.
pub enum WorkspacePreviewYaml {
    Flat(UnityYamlDocs),
    Scene(UnityYamlFile),
}

impl WorkspacePreviewYaml {
    /// Parsed YAML documents — available in both variants.
    pub fn docs(&self) -> &[YamlDoc] {
        match self {
            Self::Flat(d) => &d.docs,
            Self::Scene(f) => &f.docs,
        }
    }

    /// Line-split file content — available in both variants.
    pub fn lines(&self) -> &[String] {
        match self {
            Self::Flat(d) => &d.lines,
            Self::Scene(f) => &f.lines,
        }
    }

    /// Borrow the indexed scene view, if this is a scene/prefab session.
    /// Returns `None` for flat YAML sessions.
    pub fn as_scene(&self) -> Option<&UnityYamlFile> {
        match self {
            Self::Scene(f) => Some(f),
            Self::Flat(_) => None,
        }
    }
}

pub struct WorkspacePreviewSession {
    /// Workspace-relative path; informational only.
    pub rel_path: String,
    /// Parsed YAML payload — `Flat` for asset-style previews, `Scene`
    /// for scene/prefab previews. The variant also acts as the parse-cost
    /// switch: flat sessions skip the hierarchy + component-index passes
    /// they would never read.
    pub yaml: WorkspacePreviewYaml,
    /// Empty placeholder map for `build_doc_panel_pair_readonly` `old_labels`/
    /// `new_labels` parameters. Asset YAML doesn't need scene-style label
    /// resolution; scene-style labelling could be filled in later but the
    /// inspector code tolerates an empty map.
    pub doc_labels: HashMap<i64, String>,
    /// Per-session script cache. Built fresh; no warmup pass for now.
    pub script_cache: ScriptInfoCache,
    /// Scene/Prefab-only: hierarchy entries keyed by GameObject file_id.
    /// `None` for non-scene asset kinds. Used by
    /// `preview_workspace_asset_target` to look up a target's path for
    /// header subtitles. Always `Some` when `yaml` is `Scene` and `None`
    /// when `yaml` is `Flat` — the two travel together but we keep the
    /// flattened entries beside the indexed file because the entry
    /// labels carry UI metadata that doesn't belong on `UnityYamlFile`.
    pub scene_side: Option<SceneSemanticSideData>,
}

/// Bounded process-wide cache of parsed previews, keyed by random uuid. The
/// frontend never sees the cache directly — it just round-trips a `previewKey`
/// from `preview_workspace_asset` back to `preview_workspace_asset_target`.
///
/// Eviction policy: simple LRU touch on `get`, FIFO replacement on `insert`
/// once `CAP` entries are reached. For cap = 8 the data structure cost is
/// trivial; we deliberately don't pull in the `lru` crate.
pub struct WorkspacePreviewCache {
    inner: Mutex<VecDeque<(String, Arc<WorkspacePreviewSession>)>>,
}

impl WorkspacePreviewCache {
    const CAP: usize = 8;

    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(Self::CAP + 1)),
        }
    }

    /// Insert a fresh session and return its uuid key. Evicts the oldest
    /// entry if at capacity.
    pub fn insert(&self, session: Arc<WorkspacePreviewSession>) -> String {
        let key = Uuid::new_v4().to_string();
        if let Ok(mut q) = self.inner.lock() {
            while q.len() >= Self::CAP {
                q.pop_front();
            }
            q.push_back((key.clone(), session));
        }
        key
    }

    /// LRU touch + fetch. Moves the matched entry to the back of the queue
    /// so frequently-used previews stay resident.
    pub fn get(&self, key: &str) -> Option<Arc<WorkspacePreviewSession>> {
        let mut q = self.inner.lock().ok()?;
        let pos = q.iter().position(|(k, _)| k == key)?;
        let entry = q.remove(pos)?;
        let session = entry.1.clone();
        q.push_back(entry);
        Some(session)
    }

    /// Drop every cached session. Called on real workspace switches so a
    /// stale session from project A can never be paired with project B's
    /// `AssetDb` / GuidResolver in `preview_workspace_asset_target`.
    pub fn clear(&self) {
        if let Ok(mut q) = self.inner.lock() {
            q.clear();
        }
    }
}

impl Default for WorkspacePreviewCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension classifier. Returns the language identifier for known text
/// extensions; `None` means "treat as binary or structured".
///
/// Kept narrow on purpose — anything we don't recognize as text gets the
/// binary/structured path. This avoids accidentally trying to render a 200MB
/// FBX as utf-8.
fn text_language_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "cs" => Some("csharp"),
        "json" => Some("json"),
        "txt" => Some("text"),
        "md" | "markdown" => Some("markdown"),
        "shader" | "cginc" | "hlsl" | "glsl" | "compute" => Some("hlsl"),
        "xml" => Some("xml"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "ini" | "cfg" | "conf" => Some("ini"),
        "js" | "ts" => Some("typescript"),
        "py" => Some("python"),
        "rs" => Some("rust"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        // CSV / TSV / log are technically text but rarely useful as previews;
        // include them to avoid surprising the user.
        "csv" | "tsv" | "log" => Some("text"),
        _ => None,
    }
}

/// Validates that `requested` is a workspace-relative file path. Symlinked
/// directories are accepted when the path is reached through the workspace
/// tree, matching Unity's treatment of linked asset folders.
fn resolve_workspace_path(
    workspace_root: &Path,
    requested: &str,
) -> Result<(PathBuf, String), AppError> {
    let rel_path = super::workspace::normalize_workspace_sub_path(requested).map_err(|_| {
        AppError::new(
            "asset.preview.invalid_path",
            format!("Path is not within the workspace: {}", requested),
        )
    })?;
    if rel_path.is_empty() {
        return Err(AppError::new(
            "asset.preview.invalid_path",
            "Asset preview path cannot be empty.",
        ));
    }
    if root_for_rel_path(&rel_path).is_none() {
        return Err(AppError::new(
            "asset.preview.invalid_root",
            format!(
                "Asset preview only supports Assets, Packages, and ProjectSettings paths: {}",
                requested
            ),
        ));
    }
    let candidate = workspace_root.join(&rel_path);
    if !candidate.exists() {
        return Err(AppError::new(
            "asset.preview.not_found",
            format!("File not found: {}", requested),
        ));
    }
    if !candidate.is_file() {
        return Err(AppError::new(
            "asset.preview.not_file",
            format!("Path is not a regular file: {}", requested),
        ));
    }
    Ok((candidate, rel_path))
}

/// Read up to `TEXT_SNIPPET_MAX_LINES` lines / `TEXT_SNIPPET_MAX_BYTES` bytes
/// from `path`, returning a snippet plus truncation info.
///
/// **Streaming**: this function never loads more than ~`TEXT_SNIPPET_MAX_BYTES`
/// + one trailing line into memory. Pointing the asset preview at a 2 GiB log
/// file used to OOM because the previous implementation called
/// `std::fs::read` first and only enforced the budgets afterwards.
///
/// `total_lines` is the number of lines actually emitted in `snippet`. When
/// the file is fully read (`truncated == false`) it is the file's true line
/// count; when truncated it is the lower-bound shown count. We deliberately do
/// not scan the rest of the file just to compute a precise total — that would
/// reintroduce the DoS we just fixed.
fn read_text_snippet(path: &Path) -> Result<AssetTextPreview, AppError> {
    use std::io::{BufRead, BufReader, Read};

    let file = std::fs::File::open(path).map_err(|e| {
        AppError::new(
            "asset.preview.read_failed",
            format!("Failed to open file: {}", e),
        )
    })?;
    let mut reader = BufReader::new(file);

    let mut snippet = String::with_capacity(TEXT_SNIPPET_MAX_BYTES.min(8 * 1024));
    let mut emitted_lines: u32 = 0;
    let mut truncated = false;
    let mut buf: Vec<u8> = Vec::with_capacity(1024);

    loop {
        if (emitted_lines as usize) >= TEXT_SNIPPET_MAX_LINES {
            // Probe for any remaining byte to set the truncated flag.
            let mut probe = [0u8; 1];
            if reader.read(&mut probe).map(|n| n > 0).unwrap_or(false) {
                truncated = true;
            }
            break;
        }
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).map_err(|e| {
            AppError::new(
                "asset.preview.read_failed",
                format!("Failed to read file: {}", e),
            )
        })?;
        if n == 0 {
            break; // EOF
        }
        // Lossy decode of just this line — replacement chars beat hard-failing
        // on a stray non-utf8 byte in an otherwise text file. Strip the line's
        // own terminator so we re-emit a uniform '\n' at the end.
        let mut line: String = String::from_utf8_lossy(&buf).into_owned();
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }

        // Byte budget enforcement, *before* push.
        if snippet.len().saturating_add(line.len()).saturating_add(1) > TEXT_SNIPPET_MAX_BYTES {
            // First-line-too-long edge case: still emit a UTF-8-safe prefix so
            // the user gets *something* instead of an empty preview card.
            if emitted_lines == 0 {
                let remaining = TEXT_SNIPPET_MAX_BYTES.saturating_sub(snippet.len() + 1);
                let mut take = remaining.min(line.len());
                while take > 0 && !line.is_char_boundary(take) {
                    take -= 1;
                }
                snippet.push_str(&line[..take]);
                snippet.push('\n');
                emitted_lines = 1;
            }
            truncated = true;
            break;
        }

        snippet.push_str(&line);
        snippet.push('\n');
        emitted_lines += 1;
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let language = text_language_for_ext(&ext).map(|s| s.to_string());

    Ok(AssetTextPreview {
        snippet,
        truncated,
        total_lines: emitted_lines,
        language,
    })
}

/// Build the binary-info stub payload for a file. Used for any non-text asset
/// in Slice 3a until proper handlers replace it.
fn build_binary_info_payload(
    rel_path: &str,
    canonical: &Path,
    ref_graph_state: &AssetDbState,
) -> AssetPreviewPayload {
    let metadata = std::fs::metadata(canonical).ok();
    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
    let name = canonical
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel_path.to_string());
    let ext = canonical
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let unity_texture = read_unity_texture_preview_meta(canonical, &ext);

    // Best-effort GUID lookup. Failure is non-fatal — `guid: None` simply
    // means the asset has no `.meta` or ref_graph isn't loaded.
    let guid = {
        let guard = ref_graph_state.0.lock().ok();
        guard
            .and_then(|g| {
                g.as_ref()
                    .and_then(|graph| graph.resolve_guid_by_path(rel_path).ok())
            })
            .flatten()
            .map(|g| crate::asset_db::types::guid_to_hex(&g))
    };

    AssetPreviewPayload::BinaryInfo {
        meta: AssetBinaryMeta {
            path: rel_path.to_string(),
            name,
            size,
            ext,
            guid,
            unity_texture,
        },
    }
}

fn read_unity_texture_preview_meta(canonical: &Path, ext: &str) -> Option<UnityTexturePreviewMeta> {
    if !matches!(
        ext,
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tga" | "psd"
    ) {
        return None;
    }

    let mut meta_path = canonical.as_os_str().to_os_string();
    meta_path.push(".meta");
    let content = std::fs::read(PathBuf::from(meta_path)).ok()?;
    let importer = crate::asset_db::meta_parser::extract_importer_name(&content);
    let alpha_is_transparency =
        crate::asset_db::meta_parser::extract_alpha_is_transparency(&content);

    if importer.is_none() && alpha_is_transparency.is_none() {
        return None;
    }

    Some(UnityTexturePreviewMeta {
        importer,
        alpha_is_transparency,
    })
}

/// Try to build a renderable `binaryPreview` payload for `canonical`. Returns
/// `None` whenever the file is not a kind we know how to render, exceeds the
/// per-kind size budget, or fails to read — the caller falls back to
/// `binaryInfo` in those cases.
///
/// Single-side semantics: the asset bytes go into the `before` slot;
/// `after` is `None`. The frontend `BinaryPreviewHost` consumes this with
/// `mode="neutral"`, which hides the diff toggle and renders the single side.
fn try_build_binary_rendered_payload(
    rel_path: &str,
    canonical: &Path,
    binary_cache: &BinaryCache,
    ref_graph_state: &AssetDbState,
) -> Option<AssetPreviewPayload> {
    let total_start = Instant::now();
    // Cheap kind detection: read just enough header bytes for magic-number
    // sniffing, fall back to extension. We avoid loading the whole file
    // before deciding it's even renderable.
    let sniff_start = Instant::now();
    let mut header = [0u8; 16];
    let header_len = match std::fs::File::open(canonical) {
        Ok(mut f) => {
            use std::io::Read;
            f.read(&mut header).unwrap_or(0)
        }
        Err(_) => 0,
    };
    let sniff_ms = sniff_start.elapsed().as_millis() as u64;
    let detect_start = Instant::now();
    let kind = crate::diff::content::detect_binary_kind(rel_path, &header[..header_len])?;
    let detect_ms = detect_start.elapsed().as_millis() as u64;
    let trace_fbx = matches!(kind, crate::diff::types::BinaryPreviewKind::Model)
        && rel_path.to_ascii_lowercase().ends_with(".fbx");

    let stat_start = Instant::now();
    let size = std::fs::metadata(canonical).ok()?.len();
    let stat_ms = stat_start.elapsed().as_millis() as u64;
    if !crate::diff::content::within_binary_threshold(kind, size) {
        // Too large — caller falls back to binaryInfo so the user at least
        // sees the metadata card instead of an empty preview.
        if trace_fbx {
            eprintln!(
                "[perf:asset-preview:fbx:build] path={} total={}ms sniff={}ms detect={}ms stat={}ms skipped=threshold size={}B",
                rel_path,
                total_start.elapsed().as_millis(),
                sniff_ms,
                detect_ms,
                stat_ms,
                size
            );
        }
        return None;
    }

    let read_start = Instant::now();
    let bytes = std::fs::read(canonical).ok()?;
    let read_ms = read_start.elapsed().as_millis() as u64;
    let mime = crate::diff::content::mime_for_ext(rel_path);
    let cache_insert_start = Instant::now();
    let blob_id = binary_cache.insert(bytes, mime.clone());
    let cache_insert_ms = cache_insert_start.elapsed().as_millis() as u64;

    let asset_ref = BinaryAssetRef {
        url: format!("http://locus-binary.localhost/blob/{}", blob_id),
        mime_type: Some(mime),
        byte_size: size,
    };
    let preview = DiffBinaryPreview {
        kind,
        before: Some(asset_ref),
        after: None,
    };

    // Reuse the binary-info builder for `meta` so the GUID lookup logic stays
    // in one place. We then unwrap and re-wrap into the BinaryPreview variant.
    let meta_start = Instant::now();
    let info = build_binary_info_payload(rel_path, canonical, ref_graph_state);
    let meta = match info {
        AssetPreviewPayload::BinaryInfo { meta } => meta,
        // build_binary_info_payload always returns BinaryInfo; this arm is
        // unreachable but the compiler can't prove it without an exhaustive
        // match.
        _ => return None,
    };
    let meta_ms = meta_start.elapsed().as_millis() as u64;

    if trace_fbx {
        eprintln!(
            "[perf:asset-preview:fbx:build] path={} total={}ms sniff={}ms detect={}ms stat={}ms read={}ms cacheInsert={}ms meta={}ms size={}B",
            rel_path,
            total_start.elapsed().as_millis(),
            sniff_ms,
            detect_ms,
            stat_ms,
            read_ms,
            cache_insert_ms,
            meta_ms,
            size
        );
    }

    Some(AssetPreviewPayload::BinaryPreview { preview, meta })
}

/// Convenience: render-or-fallback. Tries the renderable binary path first
/// and falls back to the metadata-only card if anything bails.
fn build_binary_payload(
    rel_path: &str,
    canonical: &Path,
    binary_cache: &BinaryCache,
    ref_graph_state: &AssetDbState,
) -> AssetPreviewPayload {
    if let Some(rendered) =
        try_build_binary_rendered_payload(rel_path, canonical, binary_cache, ref_graph_state)
    {
        return rendered;
    }
    build_binary_info_payload(rel_path, canonical, ref_graph_state)
}

fn asset_preview_payload_kind_label(payload: &AssetPreviewPayload) -> &'static str {
    match payload {
        AssetPreviewPayload::Text(_) => "text",
        AssetPreviewPayload::BinaryPreview { .. } => "binaryPreview",
        AssetPreviewPayload::BinaryInfo { .. } => "binaryInfo",
        AssetPreviewPayload::Structured { .. } => "structured",
    }
}

// ── YAML structured dispatch (Slice 3c) ──

/// Returns the workspace asset extensions Slice 3c handles as YAML structured
/// previews. **Excludes `unity` / `prefab`** — those are routed to the Slice
/// 3d scene/prefab builder, which runs *before* this branch in
/// `preview_workspace_asset`. The remaining list mirrors
/// `crate::diff::content::is_unity_yaml` so any GenericYaml asset that the
/// diff path recognizes also gets a structured workspace preview rather than
/// silently falling through to the binaryInfo card.
fn is_slice3c_yaml_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mat"
            | "asset"
            | "anim"
            | "controller"
            | "overridecontroller"
            | "mixer"
            | "physicmaterial"
            | "physicsmaterial2d"
            | "flare"
            | "mask"
            | "fontsettings"
            | "preset"
            | "lighting"
            | "terrainlayer"
            | "rendertexture"
            | "signal"
            | "playable"
            | "cubemap"
            | "guiskin"
            | "brush"
    )
}

/// Build a one-line label for a `YamlDoc` target. Mirrors the labelling logic
/// in `diff::semantic::asset::build_asset_target` but uses the readonly script
/// cache so it works without `&mut SemanticBuildEnv`.
fn label_for_doc(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> String {
    let title = crate::diff::semantic::doc_type_label_readonly(doc, lines, side_ctx, script_cache);
    let script_class = if doc.class_id == 114 {
        resolve_script_class_name_readonly(doc, lines, side_ctx, script_cache)
    } else {
        None
    };
    match &doc.m_name {
        Some(name) if !name.is_empty() => match script_class.as_ref() {
            Some(cls) => format!("{} ({})", name, cls),
            None => format!("{} {}", title, name),
        },
        _ => format!("{} (fileID:{})", title, doc.file_id),
    }
}

fn yaml_structured_preview_doc_should_include(rel_path: &str, doc: &YamlDoc) -> bool {
    let ext = rel_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "playable" && doc.doc_index > 0 {
        return false;
    }
    true
}

/// Construct a workspace-mode `SideContext` borrowed from `ref_graph_state`.
/// Both sides of the readonly inspector pair use this same context — there is
/// no diff partner so the "two sides" are conceptually identical.
fn neutral_side_context(ref_graph_state: &AssetDbState) -> SideContext<'_> {
    SideContext {
        guid_resolver: GuidResolver::Workspace(ref_graph_state),
        script_guid_resolver: GuidResolver::Workspace(ref_graph_state),
        source_mode: SourceMode::Workspace,
        file_source: SideFileSource::Workspace,
    }
}

/// Top-level handler for YAML asset structured previews (Slice 3c).
///
/// Parses the file once, builds an owned `WorkspacePreviewSession`, stores it
/// in the cache, and returns a `structured` payload with target metadata only
/// (no panels). The frontend then fetches each target's panels on demand via
/// `preview_workspace_asset_target`.
fn try_build_yaml_structured_payload(
    rel_path: &str,
    canonical: &Path,
    asset_kind: UnityAssetKind,
    preview_cache: &WorkspacePreviewCache,
    ref_graph_state: &AssetDbState,
) -> Option<AssetPreviewPayload> {
    let bytes = std::fs::read(canonical).ok()?;
    // Flat asset preview: docs + lines only. No hierarchy/component
    // indices — the inspector path for `doc:<fileId>` targets reads
    // neither.
    let mut flat = UnityYamlDocs::parse(&bytes);
    if flat.docs.is_empty() {
        return None;
    }
    flat.docs = flat
        .docs
        .into_iter()
        .filter(|doc| yaml_structured_preview_doc_should_include(rel_path, doc))
        .collect();
    if flat.docs.is_empty() {
        return None;
    }

    // Build target metadata + flat tree nodes (one node per top-level doc).
    // The script_cache for labelling is built locally and then moved into the
    // session so the inspector lookups can reuse it.
    let script_cache = ScriptInfoCache::default();
    let side_ctx = neutral_side_context(ref_graph_state);

    let mut targets: Vec<AssetTargetMeta> = Vec::with_capacity(flat.docs.len());
    let mut tree: Vec<SemanticTreeNode> = Vec::with_capacity(flat.docs.len());
    for doc in &flat.docs {
        let target_id = format!("doc:{}", doc.file_id);
        let title = label_for_doc(doc, &flat.lines, &side_ctx, &script_cache);
        let subtitle = Some(format!("fileID:{}", doc.file_id));

        targets.push(AssetTargetMeta {
            id: target_id.clone(),
            title: title.clone(),
            subtitle: subtitle.clone(),
        });
        tree.push(SemanticTreeNode {
            id: target_id.clone(),
            parent_id: None,
            label: title,
            // For non-scene assets we don't have a Unity object kind in the
            // diff sense; the frontend treats `assetRoot` as a flat doc.
            object_kind: "assetRoot".to_string(),
            change_kind: "unchanged".to_string(),
            path: target_id,
            child_ids: Vec::new(),
            badge_counts: Default::default(),
            has_inspector: true,
        });
    }

    let session = WorkspacePreviewSession {
        rel_path: rel_path.to_string(),
        yaml: WorkspacePreviewYaml::Flat(flat),
        doc_labels: HashMap::new(),
        script_cache,
        scene_side: None,
    };
    let _ = asset_kind; // Reserved for future dispatch; AssetInspector is the only YAML layout today.
    let preview_key = preview_cache.insert(Arc::new(session));

    Some(AssetPreviewPayload::Structured {
        preview_key,
        layout: SemanticLayout::AssetInspector,
        tree,
        targets,
    })
}

// ── Scene/Prefab structured dispatch (Slice 3d) ──

/// Returns true for Scene/Prefab extensions handled by Slice 3d.
fn is_scene_or_prefab_ext(ext: &str) -> bool {
    matches!(ext, "unity" | "prefab")
}

/// Top-level handler for Scene/Prefab structured previews.
///
/// Parses the YAML once, builds the GameObject hierarchy via the existing
/// `build_go_tree` + `collect_hierarchy_entries` helpers, converts each
/// `HierarchyEntry` to a `SemanticTreeNode` (with proper parent_id/child_ids
/// linking), and stores everything in a `WorkspacePreviewSession`. Per-target
/// inspector building happens lazily in `preview_workspace_asset_target`.
fn try_build_scene_structured_payload(
    rel_path: &str,
    canonical: &Path,
    _asset_kind: UnityAssetKind,
    preview_cache: &WorkspacePreviewCache,
    ref_graph_state: &AssetDbState,
) -> Option<AssetPreviewPayload> {
    let bytes = std::fs::read(canonical).ok()?;
    // Scene/prefab preview: full indexed view. The component_index and
    // hierarchy_roots are read by the inspector path for `go:` / `pi:`
    // targets, so we pay for them once at parse time.
    let file = UnityYamlFile::parse(&bytes);
    if file.docs.is_empty() {
        return None;
    }

    let script_cache = ScriptInfoCache::default();
    let side_ctx = neutral_side_context(ref_graph_state);
    let scene_side =
        build_scene_side_data_readonly(&file.docs, &file.lines, &side_ctx, &script_cache);
    if scene_side.entries.is_empty() {
        // Empty hierarchy = nothing useful to show; bail to binaryInfo.
        return None;
    }

    // Build target metadata (one per hierarchy entry) and SemanticTreeNodes
    // (with proper parent/child linking via node_id_from_entry).
    let mut entries_by_order: Vec<_> = scene_side.entries.values().collect();
    entries_by_order.sort_by_key(|e| e.order);

    // Pre-compute node ids and a parent → children map so child_ids can be
    // populated in one pass.
    let mut id_by_file_id: HashMap<i64, String> = HashMap::with_capacity(scene_side.entries.len());
    for entry in &entries_by_order {
        id_by_file_id.insert(entry.file_id, node_id_from_entry(entry));
    }
    let mut child_ids_by_file_id: HashMap<i64, Vec<String>> =
        HashMap::with_capacity(scene_side.entries.len());
    for entry in &entries_by_order {
        if let Some(pid) = entry.parent_id {
            if let Some(child_id) = id_by_file_id.get(&entry.file_id) {
                child_ids_by_file_id
                    .entry(pid)
                    .or_default()
                    .push(child_id.clone());
            }
        }
    }

    let mut targets: Vec<AssetTargetMeta> = Vec::with_capacity(scene_side.entries.len());
    let mut tree: Vec<SemanticTreeNode> = Vec::with_capacity(scene_side.entries.len());
    for entry in &entries_by_order {
        let node_id = id_by_file_id
            .get(&entry.file_id)
            .cloned()
            .unwrap_or_else(|| node_id_from_entry(entry));
        let parent_id_str = entry
            .parent_id
            .and_then(|pid| id_by_file_id.get(&pid).cloned());
        let child_ids = child_ids_by_file_id
            .remove(&entry.file_id)
            .unwrap_or_default();

        targets.push(AssetTargetMeta {
            id: node_id.clone(),
            title: entry.label.clone(),
            subtitle: Some(entry.path.clone()),
        });
        tree.push(SemanticTreeNode {
            id: node_id,
            parent_id: parent_id_str,
            label: entry.label.clone(),
            object_kind: entry.object_kind.clone(),
            change_kind: "unchanged".to_string(),
            path: entry.path.clone(),
            child_ids,
            badge_counts: Default::default(),
            has_inspector: true,
        });
    }

    let session = WorkspacePreviewSession {
        rel_path: rel_path.to_string(),
        yaml: WorkspacePreviewYaml::Scene(file),
        doc_labels: HashMap::new(),
        script_cache,
        scene_side: Some(scene_side),
    };
    let preview_key = preview_cache.insert(Arc::new(session));

    Some(AssetPreviewPayload::Structured {
        preview_key,
        layout: SemanticLayout::SceneHierarchyInspector,
        tree,
        targets,
    })
}

/// Build the multi-panel inspector for a single GameObject in a scene/prefab
/// session: the GameObject header + one panel per child component.
///
/// PrefabInstance simplification (per plan v1): for `pi:<fileId>` targets we
/// only emit the GameObject header without component panels, because proper
/// PrefabInstance rendering requires source-prefab cross-resolution which is
/// out of scope for v1.
fn build_scene_target_inspector(
    session: &WorkspacePreviewSession,
    target_id: &str,
    file_id: i64,
    side_ctx: &SideContext,
    cwd: &str,
) -> Result<SemanticTargetInspector, AppError> {
    let scene_side = session.scene_side.as_ref().ok_or_else(|| {
        AppError::new(
            "asset.preview.session_kind_mismatch",
            "Session does not have shared scene semantic data",
        )
    })?;
    // Scene targets need the indexed `Scene` variant of the YAML payload.
    // A `Flat` session reaching this function would mean `parse_target_id`
    // accepted a `go:` / `pi:` target on a non-scene session — currently
    // impossible because that path also checks `hierarchy_entries`, but
    // we re-validate here so the error has a meaningful message instead
    // of an unwrap panic if the invariants ever drift.
    let scene = session.yaml.as_scene().ok_or_else(|| {
        AppError::new(
            "asset.preview.session_kind_mismatch",
            "Session does not have scene-indexed YAML data",
        )
    })?;
    build_workspace_scene_target_inspector(scene, scene_side, target_id, file_id, side_ctx, cwd)
}

// ── preview_workspace_asset_target (Slice 3c) ──

/// Build the inspector for a YAML asset target (Slice 3c path: `doc:<id>`).
fn build_yaml_target_inspector(
    session: &WorkspacePreviewSession,
    target_id: &str,
    file_id: i64,
    side_ctx: &SideContext,
) -> Result<SemanticTargetInspector, AppError> {
    let docs = session.yaml.docs();
    let lines = session.yaml.lines();
    let doc = docs.iter().find(|d| d.file_id == file_id).ok_or_else(|| {
        AppError::new(
            "asset.preview.target_not_found",
            format!("Target not found in session: {}", target_id),
        )
    })?;

    let title = label_for_doc(doc, lines, side_ctx, &session.script_cache);
    let script_class = if doc.class_id == 114 {
        resolve_script_class_name_readonly(doc, lines, side_ctx, &session.script_cache)
    } else {
        None
    };

    // Two-same-doc trick: feeding identical `Some(doc)` on both sides through
    // `build_doc_panel_pair_readonly` produces `change_kind = "unchanged"`,
    // which makes the `changed` panel `None` and the `full` panel `Some` with
    // every field included. We take the `full` panel.
    let (_changed, full) = build_doc_panel_pair_readonly(
        InspectorPanelKind::AssetRoot,
        title.clone(),
        script_class,
        Some(doc),
        Some(doc),
        lines,
        lines,
        &session.doc_labels,
        &session.doc_labels,
        side_ctx,
        side_ctx,
        &session.script_cache,
        Some(doc.class_id),
    );

    let panels: Vec<_> = full.into_iter().collect();

    Ok(SemanticTargetInspector {
        target_id: target_id.to_string(),
        title,
        subtitle: Some(format!("fileID:{}", doc.file_id)),
        path: session.rel_path.clone(),
        panels,
    })
}

/// Parse a target_id into one of the three supported forms:
///   - `doc:<fileId>` — YAML asset doc target (Slice 3c)
///   - `go:<fileId>`  — Scene/Prefab GameObject target (Slice 3d)
///   - `pi:<fileId>`  — Scene/Prefab PrefabInstance target (Slice 3d, header-only)
///
/// All three forms are keyed by Unity `fileID`, the only stable identifier in
/// a YAML doc. We deliberately do **not** key on the GameObject path: Unity
/// allows duplicate sibling names, so two `go:Parent/Cube` ids would collide
/// and `parse_target_id` would arbitrarily pick whichever the HashMap iterator
/// happened to surface first.
fn parse_target_id(
    target_id: &str,
    session: &WorkspacePreviewSession,
) -> Result<(TargetKind, i64), AppError> {
    if let Some(rest) = target_id.strip_prefix("doc:") {
        let file_id: i64 = rest.parse().map_err(|_| {
            AppError::new(
                "asset.preview.bad_target_id",
                format!("Malformed doc target id: {}", target_id),
            )
        })?;
        return Ok((TargetKind::YamlDoc, file_id));
    }
    if let Some(rest) = target_id.strip_prefix("pi:") {
        let file_id: i64 = rest.parse().map_err(|_| {
            AppError::new(
                "asset.preview.bad_target_id",
                format!("Malformed prefab-instance target id: {}", target_id),
            )
        })?;
        return Ok((TargetKind::PrefabInstance, file_id));
    }
    if let Some(rest) = target_id.strip_prefix("go:") {
        let scene_side = session.scene_side.as_ref().ok_or_else(|| {
            AppError::new(
                "asset.preview.session_kind_mismatch",
                "go: target requires a scene/prefab session",
            )
        })?;
        let file_id: i64 = rest.parse().map_err(|_| {
            AppError::new(
                "asset.preview.bad_target_id",
                format!("Malformed GameObject target id: {}", target_id),
            )
        })?;
        if !scene_side.entries.contains_key(&file_id) {
            return Err(AppError::new(
                "asset.preview.target_not_found",
                format!("GameObject not found in session: {}", target_id),
            ));
        }
        return Ok((TargetKind::GameObject, file_id));
    }
    Err(AppError::new(
        "asset.preview.bad_target_id",
        format!("Unrecognized target id format: {}", target_id),
    ))
}

#[derive(Debug, Clone, Copy)]
enum TargetKind {
    YamlDoc,
    GameObject,
    PrefabInstance,
}

#[tauri::command]
pub async fn preview_workspace_asset_target(
    preview_key: String,
    target_id: String,
    preview_cache: State<'_, WorkspacePreviewCache>,
    ref_graph_state: State<'_, AssetDbState>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<SemanticTargetInspector, AppError> {
    let session = preview_cache.get(&preview_key).ok_or_else(|| {
        AppError::new(
            "asset.preview.cache_miss",
            format!(
                "Preview session not found (likely evicted): {}. Re-open the asset.",
                preview_key
            ),
        )
        .retryable(true)
    })?;

    let cwd = workspace.path.read().await.clone();
    let side_ctx = neutral_side_context(&ref_graph_state);
    let (kind, file_id) = parse_target_id(&target_id, &session)?;

    match kind {
        TargetKind::YamlDoc => {
            build_yaml_target_inspector(&session, &target_id, file_id, &side_ctx)
        }
        TargetKind::GameObject => {
            build_scene_target_inspector(&session, &target_id, file_id, &side_ctx, &cwd)
        }
        TargetKind::PrefabInstance => {
            build_scene_target_inspector(&session, &target_id, file_id, &side_ctx, &cwd)
        }
    }
}

#[tauri::command]
pub async fn preview_workspace_asset_thumbnail(
    file_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<AssetThumbnailPreview, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "asset.thumbnail.no_workspace",
            "No workspace is currently open",
        ));
    }

    let workspace_root = PathBuf::from(&cwd);
    let (_canonical, asset_rel_path) = resolve_workspace_path(&workspace_root, &file_path)?;
    if !asset_rel_path.to_ascii_lowercase().ends_with(".prefab") {
        return Err(AppError::new(
            "asset.thumbnail.unsupported",
            format!(
                "Asset thumbnail only supports prefab paths: {}",
                asset_rel_path
            ),
        ));
    }

    let thumbnail = crate::unity_bridge::asset_thumbnail(&cwd, &asset_rel_path, 192)
        .await
        .map_err(|error| {
            AppError::new("asset.thumbnail.unavailable", error)
                .operation("previewWorkspaceAssetThumbnail")
        })?;
    let mime_type = if thumbnail.mime_type.trim().is_empty() {
        "image/png".to_string()
    } else {
        thumbnail.mime_type
    };
    let url = format!("data:{};base64,{}", mime_type, thumbnail.png_base64);

    Ok(AssetThumbnailPreview {
        asset_path: thumbnail.asset_path,
        url,
        width: thumbnail.width,
        height: thumbnail.height,
        mime_type,
    })
}

const ASSET_PREVIEW_FRAME_CACHE_DIR: &str = "asset-preview-cache";

fn asset_preview_frame_cache_key(asset_rel_path: &str) -> String {
    blake3::hash(asset_rel_path.to_ascii_lowercase().as_bytes())
        .to_hex()
        .to_string()
}

fn asset_preview_frame_cache_paths(
    workspace_root: &Path,
    asset_rel_path: &str,
) -> (PathBuf, PathBuf) {
    let key = asset_preview_frame_cache_key(asset_rel_path);
    let dir = workspace_root
        .join("Library")
        .join("Locus")
        .join(ASSET_PREVIEW_FRAME_CACHE_DIR);
    (
        dir.join(format!("{}.png", key)),
        dir.join(format!("{}.json", key)),
    )
}

fn source_mtime_ms(path: &Path) -> Result<u64, AppError> {
    let modified = std::fs::metadata(path)
        .map_err(|error| AppError::new("asset.preview_frame_cache.metadata", error.to_string()))?
        .modified()
        .map_err(|error| AppError::new("asset.preview_frame_cache.mtime", error.to_string()))?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64)
}

fn decode_preview_frame_data_url(url: &str) -> Result<(String, Vec<u8>), AppError> {
    let (header, encoded) = url.split_once(',').ok_or_else(|| {
        AppError::new(
            "asset.preview_frame_cache.invalid_data_url",
            "Preview frame cache data URL is missing payload",
        )
    })?;
    let mime_type = header
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .ok_or_else(|| {
            AppError::new(
                "asset.preview_frame_cache.invalid_data_url",
                "Preview frame cache only accepts base64 data URLs",
            )
        })?;
    if mime_type != "image/png" {
        return Err(AppError::new(
            "asset.preview_frame_cache.unsupported_mime",
            format!(
                "Preview frame cache only accepts image/png, got {}",
                mime_type
            ),
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| AppError::new("asset.preview_frame_cache.decode", error.to_string()))?;
    Ok((mime_type.to_string(), bytes))
}

#[tauri::command]
pub async fn read_workspace_asset_preview_frame_cache(
    file_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Option<AssetPreviewFrame>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "asset.preview_frame_cache.no_workspace",
            "No workspace is currently open",
        ));
    }

    let workspace_root = PathBuf::from(&cwd);
    let (canonical, asset_rel_path) = resolve_workspace_path(&workspace_root, &file_path)?;
    if !asset_rel_path.to_ascii_lowercase().ends_with(".prefab") {
        return Ok(None);
    }

    let (image_path, meta_path) = asset_preview_frame_cache_paths(&workspace_root, &asset_rel_path);
    if !image_path.is_file() || !meta_path.is_file() {
        return Ok(None);
    }

    let meta_raw = match std::fs::read_to_string(&meta_path) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let meta = match serde_json::from_str::<AssetPreviewFrameCacheMeta>(&meta_raw) {
        Ok(meta) => meta,
        Err(_) => return Ok(None),
    };
    if meta.asset_path != asset_rel_path || meta.source_mtime_ms != source_mtime_ms(&canonical)? {
        return Ok(None);
    }

    let bytes = match std::fs::read(&image_path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mime_type = if meta.mime_type.trim().is_empty() {
        "image/png".to_string()
    } else {
        meta.mime_type
    };

    Ok(Some(AssetPreviewFrame {
        asset_path: meta.asset_path,
        url: format!("data:{};base64,{}", mime_type, encoded),
        width: meta.width,
        height: meta.height,
        mime_type,
        yaw: Some(meta.yaw),
        pitch: Some(meta.pitch),
        distance: Some(meta.distance),
        pan_x: Some(meta.pan_x),
        pan_y: Some(meta.pan_y),
        pan_z: Some(meta.pan_z),
    }))
}

#[tauri::command]
pub async fn cache_workspace_asset_preview_frame(
    file_path: String,
    url: String,
    width: u32,
    height: u32,
    mime_type: String,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pan_x: f32,
    pan_y: f32,
    pan_z: f32,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "asset.preview_frame_cache.no_workspace",
            "No workspace is currently open",
        ));
    }

    let workspace_root = PathBuf::from(&cwd);
    let (canonical, asset_rel_path) = resolve_workspace_path(&workspace_root, &file_path)?;
    if !asset_rel_path.to_ascii_lowercase().ends_with(".prefab") {
        return Ok(());
    }

    let (_mime_from_url, bytes) = decode_preview_frame_data_url(&url)?;
    let (image_path, meta_path) = asset_preview_frame_cache_paths(&workspace_root, &asset_rel_path);
    if let Some(parent) = image_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "asset.preview_frame_cache.create_dir",
                format!("Failed to create preview frame cache directory: {}", error),
            )
        })?;
    }
    std::fs::write(&image_path, bytes).map_err(|error| {
        AppError::new(
            "asset.preview_frame_cache.write_image",
            format!("Failed to write preview frame cache image: {}", error),
        )
    })?;
    let meta = AssetPreviewFrameCacheMeta {
        asset_path: asset_rel_path,
        source_mtime_ms: source_mtime_ms(&canonical)?,
        width,
        height,
        mime_type: if mime_type.trim().is_empty() {
            "image/png".to_string()
        } else {
            mime_type
        },
        yaw,
        pitch,
        distance,
        pan_x,
        pan_y,
        pan_z,
    };
    let meta_json = serde_json::to_vec_pretty(&meta).map_err(|error| {
        AppError::new(
            "asset.preview_frame_cache.serialize",
            format!(
                "Failed to serialize preview frame cache metadata: {}",
                error
            ),
        )
    })?;
    std::fs::write(&meta_path, meta_json).map_err(|error| {
        AppError::new(
            "asset.preview_frame_cache.write_meta",
            format!("Failed to write preview frame cache metadata: {}", error),
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn render_workspace_asset_preview_frame(
    file_path: String,
    width: u32,
    height: u32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pan_x: f32,
    pan_y: f32,
    pan_z: f32,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<AssetPreviewFrame, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "asset.preview_frame.no_workspace",
            "No workspace is currently open",
        ));
    }

    let workspace_root = PathBuf::from(&cwd);
    let (_canonical, asset_rel_path) = resolve_workspace_path(&workspace_root, &file_path)?;
    if !asset_rel_path.to_ascii_lowercase().ends_with(".prefab") {
        return Err(AppError::new(
            "asset.preview_frame.unsupported",
            format!(
                "Interactive asset preview only supports prefab paths: {}",
                asset_rel_path
            ),
        ));
    }

    let frame = crate::unity_bridge::asset_preview_render(
        &cwd,
        &asset_rel_path,
        width.clamp(96, 640),
        height.clamp(96, 640),
        yaw,
        pitch,
        distance,
        pan_x,
        pan_y,
        pan_z,
    )
    .await
    .map_err(|error| {
        AppError::new("asset.preview_frame.unavailable", error)
            .operation("renderWorkspaceAssetPreviewFrame")
    })?;
    let mime_type = if frame.mime_type.trim().is_empty() {
        "image/png".to_string()
    } else {
        frame.mime_type
    };
    let url = format!("data:{};base64,{}", mime_type, frame.data_base64);

    Ok(AssetPreviewFrame {
        asset_path: frame.asset_path,
        url,
        width: frame.width,
        height: frame.height,
        mime_type,
        yaw: Some(yaw),
        pitch: Some(pitch),
        distance: Some(distance),
        pan_x: Some(pan_x),
        pan_y: Some(pan_y),
        pan_z: Some(pan_z),
    })
}

#[tauri::command]
pub async fn preview_workspace_asset(
    file_path: String,
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    binary_cache: State<'_, Arc<BinaryCache>>,
    preview_cache: State<'_, WorkspacePreviewCache>,
) -> Result<AssetPreviewPayload, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "asset.preview.no_workspace",
            "No workspace is currently open",
        ));
    }
    let workspace_root = PathBuf::from(&cwd);
    let (canonical, asset_rel_path) = resolve_workspace_path(&workspace_root, &file_path)?;

    let ext = canonical
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let trace_fbx = ext == "fbx";
    let preview_start = trace_fbx.then(Instant::now);

    if text_language_for_ext(&ext).is_some() {
        // Text path: read snippet inline. (Slice 3a does not yet share state
        // with `commands::knowledge::preview_workspace_file`; the snippet
        // budgets are matched manually so users see consistent output.)
        let preview = read_text_snippet(&canonical)?;
        return Ok(AssetPreviewPayload::Text(preview));
    }

    // Slice 3d Scene/Prefab structured dispatch. Try this BEFORE the YAML
    // path so .unity/.prefab don't get caught by the catch-all YAML branch.
    if is_scene_or_prefab_ext(&ext) {
        let asset_kind = crate::diff::content::unity_asset_kind(&asset_rel_path);
        if let Some(payload) = try_build_scene_structured_payload(
            &asset_rel_path,
            &canonical,
            asset_kind,
            &preview_cache,
            &ref_graph_state,
        ) {
            return Ok(payload);
        }
        // Parse failure or empty hierarchy → fall through to the binary path.
    }

    // Slice 3c YAML structured dispatch (Material/ScriptableObject/AnimClip/
    // AnimatorController/GenericYaml).
    if is_slice3c_yaml_ext(&ext) {
        let asset_kind = crate::diff::content::unity_asset_kind(&asset_rel_path);
        if let Some(payload) = try_build_yaml_structured_payload(
            &asset_rel_path,
            &canonical,
            asset_kind,
            &preview_cache,
            &ref_graph_state,
        ) {
            return Ok(payload);
        }
        // Parse failure → fall through to binary/info path so the user still
        // sees something rather than an error.
    }

    // Binary / structured fallthrough.
    //
    // Slice 3b: try to render image/psd/model via the existing BinaryCache
    //   + locus-binary URI protocol. On success → `binaryPreview` payload.
    //   On unknown kind / oversized / read error → `binaryInfo` fallback.
    let payload = build_binary_payload(
        &asset_rel_path,
        &canonical,
        binary_cache.inner(),
        &ref_graph_state,
    );
    if let Some(start) = preview_start {
        eprintln!(
            "[perf:preview-workspace-asset:fbx] path={} total={}ms payload={}",
            file_path,
            start.elapsed().as_millis(),
            asset_preview_payload_kind_label(&payload)
        );
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path as StdPath;

    #[cfg(unix)]
    fn create_dir_symlink(source: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(source: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(source, link)
    }

    fn create_dir_symlink_or_skip(source: &StdPath, link: &StdPath) -> bool {
        match create_dir_symlink(source, link) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipping symlink test; failed to create directory symlink: {error}");
                false
            }
        }
    }

    fn test_yaml_doc(doc_index: usize) -> YamlDoc {
        YamlDoc {
            file_id: doc_index as i64 + 1,
            class_id: 114,
            type_name: "MonoBehaviour".to_string(),
            line_start: 0,
            line_end: 0,
            m_name: None,
            m_game_object_id: None,
            m_father_id: None,
            is_stripped: false,
            source_prefab_guid: None,
            transform_parent_id: None,
            prefab_instance_id: None,
            m_layer: None,
            m_tag_string: None,
            m_static_editor_flags: None,
            m_is_active: None,
            m_enabled: None,
            transform_root_order: None,
            transform_children: Vec::new(),
            m_script_guid: None,
            doc_index,
        }
    }

    #[test]
    fn root_for_rel_path_classifies_known_roots() {
        assert_eq!(
            root_for_rel_path("Assets/Scenes/Main.unity"),
            Some(AssetSearchRoot::Assets)
        );
        assert_eq!(
            root_for_rel_path("Packages/com.x/foo.cs"),
            Some(AssetSearchRoot::Packages)
        );
        assert_eq!(
            root_for_rel_path("ProjectSettings/PlayerSettings.asset"),
            Some(AssetSearchRoot::ProjectSettings)
        );
        assert_eq!(root_for_rel_path("Library/foo.dll"), None);
        // Prefix-but-not-segment match must NOT classify.
        assert_eq!(root_for_rel_path("AssetsBackup/foo.txt"), None);
    }

    #[test]
    fn walker_score_match_tiers() {
        // Exact filename incl extension
        assert_eq!(
            walker_score_match("Assets/A/Player.cs", &[String::from("player.cs")]),
            Some(SCORE_FILENAME_EXACT)
        );
        // Exact filename stem (no extension typed)
        assert_eq!(
            walker_score_match("Assets/A/Player.cs", &[String::from("player")]),
            Some(SCORE_FILENAME_EXACT)
        );
        // Prefix
        assert_eq!(
            walker_score_match("Assets/A/PlayerCtrl.cs", &[String::from("play")]),
            Some(SCORE_FILENAME_PREFIX)
        );
        // Substring of filename
        assert_eq!(
            walker_score_match("Assets/A/MyPlayerCtrl.cs", &[String::from("playerc")]),
            Some(SCORE_FILENAME_CONTAINS)
        );
        // Path-only fragment match (filename does not contain it)
        assert_eq!(
            walker_score_match("Assets/UI/MainMenu.prefab", &[String::from("ui/main")]),
            Some(SCORE_PATH_CONTAINS)
        );
        assert_eq!(
            walker_score_match(
                "Assets/UI/HeroEnemy.prefab",
                &[String::from("hero"), String::from("ui")]
            ),
            Some(SCORE_FILENAME_PREFIX + SCORE_PATH_CONTAINS)
        );
        // No match
        assert_eq!(
            walker_score_match("Assets/A/Foo.cs", &[String::from("bar")]),
            None
        );
        // Empty query — never matches
        assert_eq!(walker_score_match("Assets/A/Foo.cs", &[]), None);
    }

    #[test]
    fn extract_bare_terms_keeps_space_split_tokens() {
        assert_eq!(
            extract_bare_terms("t:prefab hero enemy under:Assets/UI hero"),
            vec!["hero".to_string(), "enemy".to_string()]
        );
    }

    #[test]
    fn script_ref_filters_are_removed_from_bare_search_terms() {
        assert_eq!(
            extract_script_ref_terms("t:prefab component:Entity inherits:IData hero"),
            vec!["entity".to_string(), "idata".to_string()]
        );
        assert_eq!(
            strip_script_ref_filters("t:prefab component:Entity hero"),
            "t:prefab hero"
        );
        assert_eq!(
            extract_bare_terms("t:prefab component:Entity hero"),
            vec!["hero".to_string()]
        );
    }

    #[test]
    fn playable_structured_preview_keeps_only_main_yaml_doc() {
        assert!(yaml_structured_preview_doc_should_include(
            "Assets/Timeline/Farming.playable",
            &test_yaml_doc(0)
        ));
        assert!(!yaml_structured_preview_doc_should_include(
            "Assets/Timeline/Farming.playable",
            &test_yaml_doc(1)
        ));
        assert!(yaml_structured_preview_doc_should_include(
            "Assets/Audio/DefaultAudioMixer.mixer",
            &test_yaml_doc(1)
        ));
    }

    #[test]
    fn asset_preview_resolution_allows_files_inside_symlinked_asset_folders() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-assets");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Hero.cs"), b"class Hero {}").expect("write linked file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Assets/Linked")) {
            return;
        }

        let (resolved, rel_path) = resolve_workspace_path(&workspace, "Assets/Linked/Hero.cs")
            .expect("resolve linked asset file");
        assert_eq!(rel_path, "Assets/Linked/Hero.cs");
        assert_eq!(
            std::fs::read_to_string(resolved).expect("read resolved file"),
            "class Hero {}"
        );
    }

    #[test]
    fn asset_preview_resolution_rejects_symlinks_outside_asset_roots() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-docs");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Secret.txt"), b"secret").expect("write linked file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Docs")) {
            return;
        }

        let err = resolve_workspace_path(&workspace, "Docs/Secret.txt")
            .expect_err("reject non-asset linked file");
        assert_eq!(err.code, "asset.preview.invalid_root");
    }

    #[test]
    fn asset_search_fallback_follows_symlinked_asset_folders() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-assets");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Hero.prefab"), b"prefab").expect("write linked file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Assets/Linked")) {
            return;
        }

        let mut results = Vec::new();
        walk_root_for_search(
            &workspace,
            AssetSearchRoot::Assets,
            &[String::from("hero")],
            &mut results,
        );

        assert!(results
            .iter()
            .any(|result| result.path == "Assets/Linked/Hero.prefab"));
    }

    #[test]
    fn persisted_last_scan_info_round_trips() {
        let temp = tempfile::tempdir().expect("create temp workspace");
        let project_root = temp.path();
        let info = LastScanInfo {
            finished_at_unix_ms: 1_234_567,
            duration_ms: 4_321,
            stats: ScanStats {
                dirs_scanned: 11,
                meta_files_found: 22,
                yaml_assets_found: 33,
                nodes_added: 44,
                edges_added: 55,
                nodes_updated: 0,
                nodes_deleted: 0,
                parse_failures: 2,
                elapsed_ms: 4_321,
                duplicate_guids: DuplicateGuidOverview {
                    group_count: 3,
                    path_count: 6,
                    assets_only_groups: 1,
                    packages_only_groups: 1,
                    cross_root_groups: 1,
                },
            },
        };

        write_persisted_last_scan_info(project_root, &info).expect("persist last scan info");
        let restored = read_persisted_last_scan_info(project_root)
            .expect("read last scan info")
            .expect("scan info should exist");

        assert_eq!(restored.finished_at_unix_ms, info.finished_at_unix_ms);
        assert_eq!(restored.duration_ms, info.duration_ms);
        assert_eq!(restored.stats.nodes_added, info.stats.nodes_added);
        assert_eq!(restored.stats.edges_added, info.stats.edges_added);
        assert_eq!(
            restored.stats.duplicate_guids.group_count,
            info.stats.duplicate_guids.group_count
        );

        delete_persisted_last_scan_info(project_root).expect("delete persisted last scan info");
        assert!(read_persisted_last_scan_info(project_root)
            .expect("read after delete")
            .is_none());
    }
}
