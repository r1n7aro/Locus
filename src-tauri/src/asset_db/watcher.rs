//

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;

use super::db;
use super::meta_parser;
use super::object_index;
use super::scanner::{self, P1_EXTENSIONS};
use super::script_parser;
use super::types::*;
use super::AssetDb;
use crate::unity_yaml;

const MTIME_SCAN_INTERVAL_SECS: u64 = 60;
const NEW_META_DISCOVERY_INTERVAL_SECS: u64 = 10 * 60;
const MAX_LINKED_ASSET_WATCH_ROOTS: usize = 64;
type SharedLinkedAssetRoots = Arc<RwLock<Vec<LinkedAssetRoot>>>;
type SharedOsWatcher = Arc<Mutex<RecommendedWatcher>>;
type SharedWatchedPaths = Arc<Mutex<HashSet<PathBuf>>>;

#[derive(Clone)]
struct LinkedAssetWatchState {
    os_watcher: SharedOsWatcher,
    watched_paths: SharedWatchedPaths,
    linked_watched_paths: SharedWatchedPaths,
    linked_roots: SharedLinkedAssetRoots,
}

/// Default per-item debounce in milliseconds. Tunable at runtime via
/// [`WatcherTuning::debounce_ms`]. Stepless on the frontend slider, range
/// `[0, 1000]`.
pub const DEFAULT_WORKER_DEBOUNCE_MS: u64 = 100;

/// Maximum number of physical worker threads spawned per watcher. The active
/// subset is gated at runtime by [`WatcherTuning::worker_count`]; threads
/// whose index ≥ the active count idle until the user raises the count.
pub const MAX_WORKER_THREADS: usize = 8;

/// Rolling window used by watcher diagnostics and queue-summary logging.
pub const RECENT_ENQUEUE_WINDOW_MS: u64 = 8_000;
const RECENT_ENQUEUE_RETENTION_MS: u64 = 5 * 60_000;
const RECENT_ENQUEUE_SAMPLE_LIMIT: usize = 8;
const RECENT_ENQUEUE_BUFFER_LIMIT: usize = 512;
const QUEUE_SUMMARY_LOG_INTERVAL_SECS: u64 = 3;
const RECONCILE_PROGRESS_TARGET_EVENTS: u64 = 96;
const RECONCILE_PROGRESS_MIN_STEP: u64 = 32;

#[derive(Debug, Clone, Copy)]
struct MtimeScanOptions {
    discover_new_meta: bool,
    verify_hashes: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupReconcileStats {
    pub queued: u64,
    pub processed: u64,
    pub failed: u64,
}

#[derive(Debug, Clone)]
pub struct StartupReconcileProgress {
    pub verify_hashes: bool,
    pub stage: &'static str,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub queued: Option<u64>,
    pub failed: Option<u64>,
}

impl StartupReconcileProgress {
    pub fn to_scan_phase(&self) -> ScanPhase {
        ScanPhase::Reconcile {
            verify_hashes: self.verify_hashes,
            stage: Some(self.stage.to_string()),
            total: self.total,
            completed: self.completed,
            queued: self.queued,
            failed: self.failed,
        }
    }
}

type ReconcileProgressCallback<'a> = Option<&'a dyn Fn(&StartupReconcileProgress)>;

fn reconcile_progress_emit_step(total: u64) -> u64 {
    if total == 0 {
        1
    } else {
        (total / RECONCILE_PROGRESS_TARGET_EVENTS).max(RECONCILE_PROGRESS_MIN_STEP)
    }
}

fn should_emit_reconcile_progress(completed: u64, total: u64) -> bool {
    if total == 0 {
        return completed == 0;
    }
    let step = reconcile_progress_emit_step(total);
    completed == 0 || completed == 1 || completed == total || completed % step == 0
}

fn emit_reconcile_progress(
    on_progress: ReconcileProgressCallback<'_>,
    progress: StartupReconcileProgress,
) {
    if let Some(on_progress) = on_progress {
        on_progress(&progress);
    }
}

/// Compute the default number of active worker threads as ¼ of the host's
/// available parallelism, clamped to `[1, MAX_WORKER_THREADS]`. Falls back to
/// `1` when the OS does not report parallelism (e.g. some sandboxed runners).
pub fn default_worker_count() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 4).clamp(1, MAX_WORKER_THREADS)
}

/// Live-tunable knobs shared between every running watcher and the
/// `set_watcher_tuning` Tauri command. Both fields are atomics so the worker
/// loops can read them on every iteration without taking a lock.
pub struct WatcherTuning {
    pub debounce_ms: AtomicU64,
    pub worker_count: AtomicUsize,
}

impl WatcherTuning {
    pub fn new() -> Self {
        Self {
            debounce_ms: AtomicU64::new(DEFAULT_WORKER_DEBOUNCE_MS),
            worker_count: AtomicUsize::new(default_worker_count()),
        }
    }

    pub fn snapshot(&self) -> (u64, usize) {
        (
            self.debounce_ms.load(Ordering::Relaxed),
            self.worker_count.load(Ordering::Relaxed),
        )
    }

    pub fn set(&self, debounce_ms: u64, worker_count: usize) {
        let clamped_workers = worker_count.clamp(1, MAX_WORKER_THREADS);
        self.debounce_ms.store(debounce_ms, Ordering::Relaxed);
        self.worker_count.store(clamped_workers, Ordering::Relaxed);
    }
}

impl Default for WatcherTuning {
    fn default() -> Self {
        Self::new()
    }
}

/// Tauri-managed wrapper so the state container exposes a stable type.
pub struct WatcherTuningState(pub Arc<WatcherTuning>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QueueEnqueueReason {
    MetaChanged,
    ContentChanged,
    MtimeResync,
    NewMetaDiscovered,
    ScriptCascade,
}

impl QueueEnqueueReason {
    fn sort_rank(self) -> u8 {
        match self {
            Self::MetaChanged => 0,
            Self::ContentChanged => 1,
            Self::MtimeResync => 2,
            Self::NewMetaDiscovered => 3,
            Self::ScriptCascade => 4,
        }
    }

    fn log_label(self) -> &'static str {
        match self {
            Self::MetaChanged => "meta-changed",
            Self::ContentChanged => "content-changed",
            Self::MtimeResync => "mtime-resync",
            Self::NewMetaDiscovered => "new-meta",
            Self::ScriptCascade => "script-cascade",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueReasonCount {
    pub reason: QueueEnqueueReason,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentQueueFile {
    pub path: String,
    pub reason: QueueEnqueueReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentQueueActivity {
    pub window_ms: u64,
    pub total_added: u64,
    pub reasons: Vec<QueueReasonCount>,
    pub files: Vec<RecentQueueFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<u64>,
}

#[derive(Debug, Clone)]
struct QueueActivityEntry {
    at_unix_ms: u64,
    path: String,
    reason: QueueEnqueueReason,
    source_path: Option<String>,
}

pub struct RecentQueueActivityLog {
    inner: Mutex<VecDeque<QueueActivityEntry>>,
}

impl RecentQueueActivityLog {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self, path: String, reason: QueueEnqueueReason, source_path: Option<String>) {
        let now = unix_time_ms();
        let mut inner = self.inner.lock().unwrap();
        trim_recent_activity_locked(&mut inner, now);
        inner.push_back(QueueActivityEntry {
            at_unix_ms: now,
            path,
            reason,
            source_path,
        });
        while inner.len() > RECENT_ENQUEUE_BUFFER_LIMIT {
            inner.pop_front();
        }
    }

    pub fn snapshot(&self, window_ms: u64, max_files: usize) -> RecentQueueActivity {
        let now = unix_time_ms();
        let cutoff = now.saturating_sub(window_ms);
        let mut inner = self.inner.lock().unwrap();
        trim_recent_activity_locked(&mut inner, now);

        let mut total_added = 0u64;
        let mut last_event_at = None;
        let mut counts: HashMap<QueueEnqueueReason, u64> = HashMap::new();
        let mut files = Vec::new();

        for entry in inner.iter().rev() {
            if entry.at_unix_ms < cutoff {
                break;
            }
            total_added += 1;
            *counts.entry(entry.reason).or_insert(0) += 1;
            if files.len() < max_files {
                files.push(RecentQueueFile {
                    path: entry.path.clone(),
                    reason: entry.reason,
                    source_path: entry.source_path.clone(),
                    at_unix_ms: entry.at_unix_ms,
                });
            }
            last_event_at = Some(last_event_at.map_or(entry.at_unix_ms, |current: u64| {
                current.max(entry.at_unix_ms)
            }));
        }

        let mut reasons: Vec<_> = counts
            .into_iter()
            .map(|(reason, count)| QueueReasonCount { reason, count })
            .collect();
        reasons.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.reason.sort_rank().cmp(&b.reason.sort_rank()))
        });

        RecentQueueActivity {
            window_ms,
            total_added,
            reasons,
            files,
            last_event_at,
        }
    }
}

impl Default for RecentQueueActivityLog {
    fn default() -> Self {
        Self::new()
    }
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn trim_recent_activity_locked(entries: &mut VecDeque<QueueActivityEntry>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(RECENT_ENQUEUE_RETENTION_MS);
    while entries
        .front()
        .map(|entry| entry.at_unix_ms < cutoff)
        .unwrap_or(false)
    {
        entries.pop_front();
    }
}

#[derive(Debug, Clone)]
struct QueueEnqueueRequest {
    rel_path: String,
    reason: QueueEnqueueReason,
    source_path: Option<String>,
}

fn enqueue_with_activity(
    queue: &DirtyQueue,
    activity: &RecentQueueActivityLog,
    rel_path: String,
    reason: QueueEnqueueReason,
    source_path: Option<String>,
) -> bool {
    let added = queue.enqueue(rel_path.clone());
    if added {
        activity.record(rel_path, reason, source_path);
    }
    added
}

pub struct DirtyQueue {
    inner: Mutex<DirtyQueueInner>,
    condvar: Condvar,
}

struct DirtyQueueInner {
    set: HashSet<String>,
    queue: VecDeque<String>,
}

impl DirtyQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(DirtyQueueInner {
                set: HashSet::new(),
                queue: VecDeque::new(),
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn enqueue(&self, rel_path: String) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.set.contains(&rel_path) {
            return false;
        }
        inner.set.insert(rel_path.clone());
        inner.queue.push_back(rel_path);
        self.condvar.notify_one();
        true
    }

    pub fn dequeue(&self, stop: &AtomicBool) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        loop {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(path) = inner.queue.pop_front() {
                inner.set.remove(&path);
                return Some(path);
            }
            let (guard, _) = self
                .condvar
                .wait_timeout(inner, Duration::from_secs(1))
                .unwrap();
            inner = guard;
        }
    }

    fn try_dequeue(&self) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        let path = inner.queue.pop_front()?;
        inner.set.remove(&path);
        Some(path)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }
}

fn is_yaml_asset_ext(ext: &str) -> bool {
    P1_EXTENSIONS.contains(&ext)
}

fn is_unity_asset_path(rel_path: &str) -> bool {
    if rel_path.ends_with(".meta") {
        return true;
    }
    let ext = Path::new(rel_path)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    matches!(
        ext.as_str(),
        "unity"
            | "prefab"
            | "asset"
            | "mat"
            | "anim"
            | "controller"
            | "cs"
            | "png"
            | "jpg"
            | "jpeg"
            | "tga"
            | "psd"
            | "tif"
            | "tiff"
            | "bmp"
            | "gif"
            | "exr"
            | "hdr"
            | "wav"
            | "mp3"
            | "ogg"
            | "aif"
            | "aiff"
            | "shader"
            | "cginc"
            | "hlsl"
            | "glsl"
            | "compute"
            | "fbx"
            | "obj"
            | "blend"
            | "dae"
            | "3ds"
            | "max"
    )
}

fn asset_rel_path_and_reason(rel: String) -> Option<(String, QueueEnqueueReason)> {
    if !rel.starts_with("Assets/") && !rel.starts_with("Packages/") {
        return None;
    }

    for component in rel.split('/') {
        if scanner::IGNORED_DIRS
            .iter()
            .any(|d| d.eq_ignore_ascii_case(component))
        {
            return None;
        }
    }

    if !is_unity_asset_path(&rel) {
        return None;
    }

    let reason = if rel.ends_with(".meta") {
        QueueEnqueueReason::MetaChanged
    } else {
        QueueEnqueueReason::ContentChanged
    };
    let asset_path = rel.strip_suffix(".meta").unwrap_or(&rel).to_string();
    Some((asset_path, reason))
}

fn join_linked_asset_rel_path(link_rel_path: &str, target_relative: &Path) -> String {
    let suffix = target_relative.to_string_lossy().replace('\\', "/");
    let suffix = suffix.trim_matches('/');
    if suffix.is_empty() {
        link_rel_path.to_string()
    } else {
        format!("{}/{}", link_rel_path.trim_end_matches('/'), suffix)
    }
}

fn linked_asset_rel_paths_for_abs(
    abs_path: &Path,
    linked_roots: &[LinkedAssetRoot],
) -> Vec<String> {
    let canonical_abs = dunce::canonicalize(abs_path).ok();
    let mut rel_paths = Vec::new();
    let mut seen = HashSet::new();

    for root in linked_roots {
        let mut mapped = None;
        if let Some(canonical_abs) = canonical_abs.as_ref() {
            if let Ok(relative) = canonical_abs.strip_prefix(&root.target_path) {
                mapped = Some(join_linked_asset_rel_path(&root.link_rel_path, relative));
            }
        }
        if mapped.is_none() {
            if let Ok(relative) = abs_path.strip_prefix(&root.target_path) {
                mapped = Some(join_linked_asset_rel_path(&root.link_rel_path, relative));
            }
        }
        if let Some(rel_path) = mapped {
            if seen.insert(rel_path.clone()) {
                rel_paths.push(rel_path);
            }
        }
    }

    rel_paths
}

fn push_asset_rel_path_and_reason(
    rel: String,
    out: &mut Vec<(String, QueueEnqueueReason)>,
    seen: &mut HashSet<String>,
) {
    if let Some((asset_path, reason)) = asset_rel_path_and_reason(rel) {
        if seen.insert(asset_path.clone()) {
            out.push((asset_path, reason));
        }
    }
}

fn to_asset_rel_paths_and_reasons(
    project_root: &Path,
    abs_path: &Path,
    linked_roots: &[LinkedAssetRoot],
) -> Vec<(String, QueueEnqueueReason)> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(rel) = abs_path.strip_prefix(project_root) {
        let rel = rel.to_string_lossy().replace('\\', "/");
        push_asset_rel_path_and_reason(rel, &mut results, &mut seen);
    }

    for rel in linked_asset_rel_paths_for_abs(abs_path, linked_roots) {
        push_asset_rel_path_and_reason(rel, &mut results, &mut seen);
    }

    results
}

fn file_mtime_ns(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .as_ref()
        .map(scanner::get_mtime_ns)
        .unwrap_or(0)
}

fn sort_linked_asset_roots(mut roots: Vec<LinkedAssetRoot>) -> Vec<LinkedAssetRoot> {
    roots.sort_by(|left, right| {
        right
            .target_path
            .components()
            .count()
            .cmp(&left.target_path.components().count())
            .then_with(|| left.link_rel_path.cmp(&right.link_rel_path))
    });
    roots
}

fn record_linked_asset_root(
    project_root: &Path,
    entry: &walkdir::DirEntry,
    linked_asset_roots: &mut Vec<LinkedAssetRoot>,
    linked_asset_rel_paths: &mut HashSet<String>,
) {
    if !entry.path_is_symlink() || !entry.file_type().is_dir() {
        return;
    }

    let Ok(rel) = entry.path().strip_prefix(project_root) else {
        return;
    };
    let link_rel_path = rel.to_string_lossy().replace('\\', "/");
    if !linked_asset_rel_paths.insert(link_rel_path.clone()) {
        return;
    }
    let target_path =
        dunce::canonicalize(entry.path()).unwrap_or_else(|_| entry.path().to_path_buf());
    linked_asset_roots.push(LinkedAssetRoot {
        link_rel_path,
        target_path,
    });
}

fn cached_linked_asset_roots(graph_state: &Arc<Mutex<Option<AssetDb>>>) -> Vec<LinkedAssetRoot> {
    let guard = match graph_state.lock() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!(
                "[AssetDb Watcher] warning: failed to lock ref_graph for linked roots: {}",
                error
            );
            return Vec::new();
        }
    };

    let Some(graph) = guard.as_ref() else {
        return Vec::new();
    };

    match graph.linked_asset_roots() {
        Ok(roots) => sort_linked_asset_roots(roots),
        Err(error) => {
            eprintln!(
                "[AssetDb Watcher] warning: failed to load cached linked asset roots: {}",
                error
            );
            Vec::new()
        }
    }
}

fn watch_key(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn linked_watch_keys_for_roots(roots: &[LinkedAssetRoot]) -> HashSet<PathBuf> {
    roots
        .iter()
        .take(MAX_LINKED_ASSET_WATCH_ROOTS)
        .map(|root| watch_key(&root.target_path))
        .collect()
}

fn watch_path_once(
    os_watcher: &SharedOsWatcher,
    watched_paths: &SharedWatchedPaths,
    watch_path: &Path,
    label: &str,
) -> Result<bool, String> {
    let key = watch_key(watch_path);
    {
        let mut watched = watched_paths
            .lock()
            .map_err(|e| format!("Failed to lock watched paths: {}", e))?;
        if !watched.insert(key.clone()) {
            return Ok(false);
        }
    }

    let watch_result = os_watcher
        .lock()
        .map_err(|e| format!("Failed to lock OS watcher: {}", e))?
        .watch(watch_path, RecursiveMode::Recursive);

    match watch_result {
        Ok(()) => Ok(true),
        Err(error) => {
            if let Ok(mut watched) = watched_paths.lock() {
                watched.remove(&key);
            }
            Err(format!(
                "Failed to watch {} {}: {}",
                label,
                watch_path.display(),
                error
            ))
        }
    }
}

fn install_linked_asset_root_watches(
    watch_state: &LinkedAssetWatchState,
    roots: &[LinkedAssetRoot],
) -> usize {
    let mut linked_watch_count = 0usize;
    for linked_root in roots.iter().take(MAX_LINKED_ASSET_WATCH_ROOTS) {
        match watch_path_once(
            &watch_state.os_watcher,
            &watch_state.watched_paths,
            &linked_root.target_path,
            "linked asset root",
        ) {
            Ok(true) => {
                if let Ok(mut linked_watched) = watch_state.linked_watched_paths.lock() {
                    linked_watched.insert(watch_key(&linked_root.target_path));
                }
                linked_watch_count += 1;
                eprintln!(
                    "[AssetDb Watcher] watching linked asset root: {} -> {}",
                    linked_root.link_rel_path,
                    linked_root.target_path.display()
                );
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "[AssetDb Watcher] warning: failed to watch linked asset root {} -> {}: {}",
                    linked_root.link_rel_path,
                    linked_root.target_path.display(),
                    error
                );
            }
        }
    }
    if roots.len() > MAX_LINKED_ASSET_WATCH_ROOTS {
        eprintln!(
            "[AssetDb Watcher] warning: linked asset roots watch limit reached: watching {} of {}",
            linked_watch_count,
            roots.len()
        );
    }
    linked_watch_count
}

fn prune_stale_linked_watch_keys(
    linked_watched_paths: &SharedWatchedPaths,
    desired_keys: &HashSet<PathBuf>,
) -> Vec<PathBuf> {
    let mut linked_watched = match linked_watched_paths.lock() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!(
                "[AssetDb Watcher] warning: failed to lock linked watched paths: {}",
                error
            );
            return Vec::new();
        }
    };

    let stale_keys: Vec<PathBuf> = linked_watched
        .iter()
        .filter(|key| !desired_keys.contains(*key))
        .cloned()
        .collect();
    for key in &stale_keys {
        linked_watched.remove(key);
    }
    stale_keys
}

fn uninstall_stale_linked_asset_root_watches(
    watch_state: &LinkedAssetWatchState,
    desired_keys: &HashSet<PathBuf>,
) {
    let stale_keys = prune_stale_linked_watch_keys(&watch_state.linked_watched_paths, desired_keys);

    for key in stale_keys {
        let unwatch_result = watch_state
            .os_watcher
            .lock()
            .map_err(|e| format!("Failed to lock OS watcher: {}", e))
            .and_then(|mut watcher| {
                watcher.unwatch(&key).map_err(|e| {
                    format!(
                        "Failed to unwatch linked asset root {}: {}",
                        key.display(),
                        e
                    )
                })
            });
        if let Err(error) = unwatch_result {
            eprintln!("[AssetDb Watcher] warning: {}", error);
        } else {
            eprintln!(
                "[AssetDb Watcher] unwatched linked asset root: {}",
                key.display()
            );
        }

        if let Ok(mut watched) = watch_state.watched_paths.lock() {
            watched.remove(&key);
        }
    }
}

fn sync_linked_asset_root_watches(watch_state: &LinkedAssetWatchState, roots: &[LinkedAssetRoot]) {
    let desired_keys = linked_watch_keys_for_roots(roots);
    uninstall_stale_linked_asset_root_watches(watch_state, &desired_keys);
    install_linked_asset_root_watches(watch_state, roots);
}

fn persist_linked_asset_roots_if_changed(
    state: &Arc<Mutex<Option<AssetDb>>>,
    roots: &[LinkedAssetRoot],
) -> Result<bool, String> {
    let mut guard = state.lock().map_err(|e| format!("Lock error: {}", e))?;
    let Some(graph) = guard.as_mut() else {
        return Ok(false);
    };

    let current = sort_linked_asset_roots(graph.linked_asset_roots()?);
    if current == roots {
        return Ok(false);
    }

    let tx = graph
        .conn
        .transaction()
        .map_err(|e| format!("Failed to begin linked asset root update: {}", e))?;
    db::replace_linked_asset_roots(&tx, roots)?;
    tx.commit()
        .map_err(|e| format!("Failed to commit linked asset root update: {}", e))?;
    Ok(true)
}

fn refresh_linked_asset_roots(
    state: &Arc<Mutex<Option<AssetDb>>>,
    watch_state: Option<&LinkedAssetWatchState>,
    roots: Vec<LinkedAssetRoot>,
) {
    let roots = sort_linked_asset_roots(roots);
    let changed = match persist_linked_asset_roots_if_changed(state, &roots) {
        Ok(changed) => changed,
        Err(error) => {
            eprintln!(
                "[AssetDb Watcher] warning: failed to persist linked asset roots: {}",
                error
            );
            true
        }
    };

    let Some(watch_state) = watch_state else {
        return;
    };

    let roots_changed_for_runtime = watch_state
        .linked_roots
        .read()
        .map(|current| *current != roots)
        .unwrap_or(true);
    if !changed && !roots_changed_for_runtime {
        return;
    }

    sync_linked_asset_root_watches(watch_state, &roots);
    if let Ok(mut current) = watch_state.linked_roots.write() {
        *current = roots;
    }
}

fn sleep_interruptible(duration: Duration, stop: &AtomicBool) -> bool {
    let tick = Duration::from_millis(50);
    let mut slept = Duration::ZERO;
    while slept < duration {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        let remaining = duration.saturating_sub(slept);
        let slice = remaining.min(tick);
        std::thread::sleep(slice);
        slept += slice;
    }
    stop.load(Ordering::Relaxed)
}

fn resolve_guid_paths_for_content(
    content: &[u8],
    graph_state: &Arc<Mutex<Option<AssetDb>>>,
) -> Result<HashMap<Guid, String>, String> {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text.lines().collect();
    let guids = unity_yaml::collect_guids_from_lines(&lines, 0, lines.len());
    if guids.is_empty() {
        return Ok(HashMap::new());
    }

    let guard = graph_state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    match guard.as_ref() {
        Some(graph) => db::batch_resolve_paths(&graph.conn, &guids),
        None => Ok(HashMap::new()),
    }
}

fn process_dirty_asset(
    asset_rel_path: &str,
    project_root: &Path,
    graph_state: &Arc<Mutex<Option<AssetDb>>>,
    stop: &AtomicBool,
) -> Result<Vec<QueueEnqueueRequest>, String> {
    if stop.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }

    let meta_abs = project_root.join(format!("{}.meta", asset_rel_path));
    let asset_abs = project_root.join(asset_rel_path);

    let meta_exists = meta_abs.is_file();
    let asset_exists = asset_abs.is_file();

    if !meta_exists {
        if stop.load(Ordering::Relaxed) {
            return Ok(Vec::new());
        }
        let mut guard = graph_state
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(ref mut graph) = *guard {
            if db::delete_missing_asset_path(&mut graph.conn, asset_rel_path)? {
                eprintln!(
                    "[AssetDb Watcher] removed deleted asset: {}",
                    asset_rel_path
                );
            } else {
                eprintln!(
                    "[AssetDb Watcher] removed deleted asset bookkeeping: {}",
                    asset_rel_path
                );
            }
        }
        return Ok(Vec::new());
    }

    if stop.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }
    let meta_content = std::fs::read(&meta_abs)
        .map_err(|e| format!("Failed to read {}: {}", meta_abs.display(), e))?;
    let guid = meta_parser::extract_guid(&meta_content)
        .ok_or_else(|| format!("No GUID in {}", meta_abs.display()))?;
    let meta_hash = hash128(&meta_content);
    let meta_mtime = file_mtime_ns(&meta_abs);
    let meta_size = std::fs::metadata(&meta_abs).map(|m| m.len()).unwrap_or(0);

    let ext = Path::new(asset_rel_path)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    let old_script_meta = if ext == "cs" {
        let guard = graph_state
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(graph) = guard.as_ref() {
            db::get_stored_script_metadata(&graph.conn, &guid)?
        } else {
            None
        }
    } else {
        None
    };

    let kind;
    let mut content_hash = [0u8; 16];
    let mut edges: Vec<RefEdge> = Vec::new();
    let mut asset_mtime = meta_mtime;
    let mut asset_size = 0u64;
    let mut stored_script_meta: Option<db::StoredScriptMetadata> = None;
    let mut yaml_docs: Option<Vec<unity_yaml::YamlDoc>> = None;
    let mut script_type_by_guid: HashMap<Guid, db::StoredScriptMetadata> = HashMap::new();

    if asset_exists && is_yaml_asset_ext(&ext) {
        let content = std::fs::read(&asset_abs)
            .map_err(|e| format!("Failed to read {}: {}", asset_abs.display(), e))?;
        let guid_to_path = resolve_guid_paths_for_content(&content, graph_state)?;
        let refs = unity_yaml::extract_refs_with_resolver(&content, Some(&guid_to_path));
        let docs = unity_yaml::parse_yaml_docs(&content);
        content_hash = hash128(&content);
        let metadata = std::fs::metadata(&asset_abs).ok();
        asset_mtime = metadata.as_ref().map(scanner::get_mtime_ns).unwrap_or(0);
        asset_size = metadata.map(|m| m.len()).unwrap_or(0);
        kind = AssetKind::from_ext(&ext);

        let script_guids: HashSet<Guid> = docs.iter().filter_map(|doc| doc.m_script_guid).collect();
        if !script_guids.is_empty() {
            let guard = graph_state
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            if let Some(graph) = guard.as_ref() {
                for script_guid in script_guids {
                    if let Some(meta) = db::get_stored_script_metadata(&graph.conn, &script_guid)? {
                        script_type_by_guid.insert(script_guid, meta);
                    }
                }
            }
        }

        edges = refs
            .iter()
            .map(|r| RefEdge {
                src_guid: guid,
                src_file_id: r.src_file_id,
                dst_guid: r.dst_guid,
                dst_file_id: r.dst_file_id,
                class_id_hint: r.class_id_hint,
                field_hint: r.field_hint.clone(),
                ref_path: r.ref_path.clone(),
            })
            .collect();

        if kind == AssetKind::GenericAsset {
            let script_guid = docs
                .iter()
                .find(|doc| doc.doc_index == 0 && doc.class_id == 114)
                .and_then(|doc| doc.m_script_guid);

            if let Some(script_guid) = script_guid {
                if let Some(meta) = script_type_by_guid.get(&script_guid) {
                    if meta.inherits_scriptable_object() {
                        stored_script_meta = Some(meta.clone());
                    }
                }
            }
        }
        yaml_docs = Some(docs);
    } else if asset_exists && ext == "cs" {
        let snapshot = script_parser::read_script_file_snapshot(&asset_abs)
            .ok_or_else(|| format!("Failed to read script file: {}", asset_abs.display()))?;
        kind = AssetKind::Script;
        content_hash = snapshot.content_hash;
        asset_mtime = snapshot.mtime_ns;
        asset_size = snapshot.size;

        // A script file is still a valid asset even when we can't extract a
        // top-level type from it (commented out, behind `#if false`, only
        // contains delegates / extension stubs, etc.). In that case we
        // index its hash + mtime + size but skip the class-name search
        // metadata. We only log a warning when the file *should* have
        // parsed (real class hidden behind a tree-sitter recovery error);
        // empty namespaces and enum-only files are silently indexed since
        // Unity doesn't bind those to a `.cs` filename anyway.
        if snapshot.metadata.is_none() {
            if let Some(script_parser::ScriptNoMetadataReason::Unparseable) =
                snapshot.no_metadata_reason
            {
                eprintln!(
                    "[AssetDb Watcher] no parseable C# type in {} (indexed without class metadata)",
                    asset_rel_path
                );
            }
        }
        if snapshot.metadata.is_some() {
            let preferred_namespace_lower = script_parser::normalize_namespace_lower(
                snapshot
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.namespace.as_deref()),
            );
            let inherited_base_search = if let Some(base_type) = snapshot
                .metadata
                .as_ref()
                .and_then(|m| m.base_type.as_deref())
            {
                let guard = graph_state
                    .lock()
                    .map_err(|e| format!("Lock error: {}", e))?;
                if let Some(graph) = guard.as_ref() {
                    db::get_stored_script_metadata_for_base_type(
                        &graph.conn,
                        base_type,
                        (!preferred_namespace_lower.is_empty())
                            .then_some(preferred_namespace_lower.as_str()),
                    )?
                    .map(|meta| meta.type_search_lower)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(indexed) = script_parser::build_indexed_script_metadata(
                &snapshot,
                inherited_base_search.as_deref(),
            ) {
                stored_script_meta = Some(db::StoredScriptMetadata {
                    class_name: indexed.class_name,
                    class_name_lower: indexed.class_name_lower,
                    namespace_lower: indexed.namespace_lower,
                    full_name_lower: indexed.full_name_lower,
                    type_search_lower: indexed.type_search_lower,
                    inheritance_search_lower: indexed.inheritance_search_lower,
                });
            }
        }
    } else {
        kind = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "tga" | "psd" | "tif" | "tiff" | "bmp" | "gif" | "exr"
            | "hdr" => AssetKind::Texture,
            "wav" | "mp3" | "ogg" | "aif" | "aiff" => AssetKind::Audio,
            "shader" | "cginc" | "hlsl" | "glsl" | "compute" => AssetKind::Shader,
            "fbx" | "obj" | "blend" | "dae" | "3ds" | "max" => AssetKind::Model,
            _ => {
                if asset_exists {
                    AssetKind::OtherYaml
                } else {
                    AssetKind::MetaOnly
                }
            }
        };
        if asset_exists {
            let metadata = std::fs::metadata(&asset_abs).ok();
            asset_mtime = metadata.as_ref().map(scanner::get_mtime_ns).unwrap_or(0);
            asset_size = metadata.map(|m| m.len()).unwrap_or(0);
        }
    }

    let node = AssetNode {
        guid,
        path: asset_rel_path.to_string(),
        ext: ext.clone(),
        kind,
        exists_on_disk: asset_exists,
        mtime_ns: asset_mtime.max(meta_mtime),
        size: asset_size,
        content_hash,
        meta_hash,
        parser_version: 1,
        script_class_name: stored_script_meta
            .as_ref()
            .map(|meta| meta.class_name.clone()),
        script_class_lower: stored_script_meta
            .as_ref()
            .map(|meta| meta.class_name_lower.clone())
            .unwrap_or_default(),
        script_namespace_lower: stored_script_meta
            .as_ref()
            .map(|meta| meta.namespace_lower.clone())
            .unwrap_or_default(),
        script_full_name_lower: stored_script_meta
            .as_ref()
            .map(|meta| meta.full_name_lower.clone())
            .unwrap_or_default(),
        script_type_search: stored_script_meta
            .as_ref()
            .map(|meta| meta.type_search_lower.clone())
            .unwrap_or_default(),
        script_inheritance_search: stored_script_meta
            .as_ref()
            .map(|meta| meta.inheritance_search_lower.clone())
            .unwrap_or_default(),
    };

    let mut asset_objects: Vec<AssetObject> = Vec::new();
    if let Some(docs) = yaml_docs.as_ref() {
        asset_objects.extend(object_index::build_yaml_asset_objects(
            &node,
            docs,
            |script_guid| {
                script_type_by_guid
                    .get(script_guid)
                    .map(|meta| object_index::ScriptTypeInfo {
                        class_name: meta.class_name.clone(),
                        class_name_lower: meta.class_name_lower.clone(),
                        full_name_lower: meta.full_name_lower.clone(),
                        type_search_lower: meta.type_search_lower.clone(),
                    })
            },
        ));
    }
    let importer_subassets = object_index::parse_importer_subassets(&meta_content);
    if !importer_subassets.is_empty() {
        asset_objects.extend(object_index::build_importer_sub_asset_objects(
            &node,
            &importer_subassets,
        ));
    }

    let meta_rel = format!("{}.meta", asset_rel_path);
    let mut file_records = vec![(meta_rel, FileRole::Meta, meta_mtime, meta_size, meta_hash)];
    if asset_exists && is_yaml_asset_ext(&ext) {
        file_records.push((
            asset_rel_path.to_string(),
            FileRole::YamlAsset,
            asset_mtime,
            asset_size,
            content_hash,
        ));
    }

    if stop.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }

    let mut guard = graph_state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let mut cascade_paths = Vec::new();
    if let Some(ref mut graph) = *guard {
        db::atomic_update_asset(
            &mut graph.conn,
            &node,
            &asset_objects,
            &edges,
            &file_records,
        )?;
        if ext == "cs" {
            let source_path = Some(asset_rel_path.to_string());
            for path in db::find_asset_paths_referencing_guid(&graph.conn, &guid)? {
                cascade_paths.push(QueueEnqueueRequest {
                    rel_path: path,
                    reason: QueueEnqueueReason::ScriptCascade,
                    source_path: source_path.clone(),
                });
            }

            let mut class_names = Vec::new();
            if let Some(old_meta) = old_script_meta.as_ref() {
                class_names.push(old_meta.cascade_lookup_term().to_string());
            }
            if let Some(new_meta) = stored_script_meta.as_ref() {
                if !class_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(new_meta.cascade_lookup_term()))
                {
                    class_names.push(new_meta.cascade_lookup_term().to_string());
                }
            }
            for path in db::find_script_descendant_paths(&graph.conn, &class_names, &guid)? {
                cascade_paths.push(QueueEnqueueRequest {
                    rel_path: path,
                    reason: QueueEnqueueReason::ScriptCascade,
                    source_path: source_path.clone(),
                });
            }
        }
    }

    Ok(cascade_paths)
}

fn worker_loop(
    index: usize,
    queue: Arc<DirtyQueue>,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<Option<AssetDb>>>,
    project_root: PathBuf,
    current: CurrentFileSlot,
    tuning: Arc<WatcherTuning>,
    activity: Arc<RecentQueueActivityLog>,
) {
    eprintln!("[AssetDb Watcher] worker {} thread started", index);
    while !stop.load(Ordering::Relaxed) {
        // Idle gate: workers whose index is beyond the active count just
        // sleep. This lets `set_watcher_tuning` shrink/grow the active pool
        // without restarting the watcher.
        let active = tuning.worker_count.load(Ordering::Relaxed);
        if index >= active {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        let rel_path = match queue.dequeue(&stop) {
            Some(p) => p,
            None => continue,
        };

        // IMPORTANT: publish the current-file slot BEFORE the debounce sleep
        // so external observers (the asset page) see "processing X" for the
        // entire duration the worker holds onto the item — not just the SQL
        // write window. The previous order produced a long visible-idle gap
        // every iteration, even when the queue had thousands of items.
        if let Ok(mut slot) = current.lock() {
            *slot = Some(rel_path.clone());
        }

        let debounce_ms = tuning.debounce_ms.load(Ordering::Relaxed);
        if debounce_ms > 0 && sleep_interruptible(Duration::from_millis(debounce_ms), &stop) {
            if let Ok(mut slot) = current.lock() {
                *slot = None;
            }
            break;
        }

        match process_dirty_asset(&rel_path, &project_root, &state, &stop) {
            Ok(extra_paths) => {
                for extra_path in extra_paths {
                    enqueue_with_activity(
                        &queue,
                        &activity,
                        extra_path.rel_path,
                        extra_path.reason,
                        extra_path.source_path,
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "[AssetDb Watcher] worker {} error processing {}: {}",
                    index, rel_path, e
                );
            }
        }

        if let Ok(mut slot) = current.lock() {
            *slot = None;
        }
    }
    eprintln!("[AssetDb Watcher] worker {} thread stopped", index);
}

fn should_hash_asset_content(kind: AssetKind) -> bool {
    matches!(kind, AssetKind::Script)
}

fn file_hash128(path: &Path) -> Option<[u8; 16]> {
    std::fs::read(path).ok().map(|content| hash128(&content))
}

fn event_receiver_loop(
    rx: std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>,
    queue: Arc<DirtyQueue>,
    stop: Arc<AtomicBool>,
    project_root: PathBuf,
    linked_roots: SharedLinkedAssetRoots,
    activity: Arc<RecentQueueActivityLog>,
) {
    eprintln!("[AssetDb Watcher] event receiver thread started");
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(event)) => {
                let linked_roots_snapshot = linked_roots
                    .read()
                    .map(|roots| roots.clone())
                    .unwrap_or_default();
                for path in &event.paths {
                    for (rel, reason) in to_asset_rel_paths_and_reasons(
                        &project_root,
                        path,
                        linked_roots_snapshot.as_slice(),
                    ) {
                        if enqueue_with_activity(&queue, &activity, rel.clone(), reason, None) {
                            eprintln!("[AssetDb Watcher] dirty (OS/{:?}): {}", reason, rel);
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("[AssetDb Watcher] watch error: {}", e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    eprintln!("[AssetDb Watcher] event receiver thread stopped");
}

fn mtime_scanner_loop(
    queue: Arc<DirtyQueue>,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<Option<AssetDb>>>,
    project_root: PathBuf,
    activity: Arc<RecentQueueActivityLog>,
    linked_watch_state: LinkedAssetWatchState,
) {
    mtime_scan_once_with_linked_watch(
        &queue,
        &stop,
        &state,
        &project_root,
        &activity,
        true,
        Some(&linked_watch_state),
    );

    let mut mtime_elapsed = Duration::ZERO;
    let mut discovery_elapsed = Duration::ZERO;
    let mtime_interval = Duration::from_secs(MTIME_SCAN_INTERVAL_SECS);
    let discovery_interval = Duration::from_secs(NEW_META_DISCOVERY_INTERVAL_SECS);
    let tick = Duration::from_secs(1);

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(tick);
        mtime_elapsed += tick;
        discovery_elapsed += tick;
        if mtime_elapsed >= mtime_interval {
            mtime_elapsed = Duration::ZERO;
            let discover_new_meta = discovery_elapsed >= discovery_interval;
            if discover_new_meta {
                discovery_elapsed = Duration::ZERO;
            }
            mtime_scan_once_with_linked_watch(
                &queue,
                &stop,
                &state,
                &project_root,
                &activity,
                discover_new_meta,
                Some(&linked_watch_state),
            );
        }
    }
}

fn mtime_scan_once(
    queue: &DirtyQueue,
    stop: &AtomicBool,
    state: &Arc<Mutex<Option<AssetDb>>>,
    project_root: &Path,
    activity: &RecentQueueActivityLog,
    discover_new_meta: bool,
) {
    mtime_scan_once_with_linked_watch(
        queue,
        stop,
        state,
        project_root,
        activity,
        discover_new_meta,
        None,
    );
}

fn mtime_scan_once_with_linked_watch(
    queue: &DirtyQueue,
    stop: &AtomicBool,
    state: &Arc<Mutex<Option<AssetDb>>>,
    project_root: &Path,
    activity: &RecentQueueActivityLog,
    discover_new_meta: bool,
    linked_watch_state: Option<&LinkedAssetWatchState>,
) {
    mtime_scan_once_with_options(
        queue,
        stop,
        state,
        project_root,
        activity,
        MtimeScanOptions {
            discover_new_meta,
            verify_hashes: false,
        },
        linked_watch_state,
        None,
    );
}

fn mtime_scan_once_with_options(
    queue: &DirtyQueue,
    stop: &AtomicBool,
    state: &Arc<Mutex<Option<AssetDb>>>,
    project_root: &Path,
    activity: &RecentQueueActivityLog,
    options: MtimeScanOptions,
    linked_watch_state: Option<&LinkedAssetWatchState>,
    on_progress: ReconcileProgressCallback<'_>,
) {
    if stop.load(Ordering::Relaxed) {
        return;
    }

    let (asset_records, file_records): (Vec<db::AssetMtimeRecord>, Vec<db::FileMtimeRecord>) = {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match &*guard {
            Some(graph) => {
                let mtimes = match db::get_all_asset_mtime_records(&graph.conn) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[AssetDb Watcher] mtime query error: {}", e);
                        return;
                    }
                };
                let file_records = match db::get_all_file_mtime_records(&graph.conn) {
                    Ok(v) => v.into_iter().collect(),
                    Err(e) => {
                        eprintln!("[AssetDb Watcher] file mtime query error: {}", e);
                        return;
                    }
                };
                (mtimes, file_records)
            }
            None => return,
        }
    };

    if asset_records.is_empty() && file_records.is_empty() && !options.discover_new_meta {
        return;
    }

    let scan_total = (asset_records.len() + file_records.len()) as u64;
    let mut scan_completed = 0u64;
    emit_reconcile_progress(
        on_progress,
        StartupReconcileProgress {
            verify_hashes: options.verify_hashes,
            stage: "scanning",
            total: Some(scan_total),
            completed: Some(0),
            queued: Some(queue.len() as u64),
            failed: Some(0),
        },
    );

    for record in &asset_records {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let meta_path = project_root.join(format!("{}.meta", record.path));
        let content_path = project_root.join(&record.path);

        let meta_exists = meta_path.is_file();
        let content_exists = content_path.is_file();
        let meta_mtime = if meta_exists {
            file_mtime_ns(&meta_path)
        } else {
            0
        };
        let content_mtime = if content_exists {
            file_mtime_ns(&content_path)
        } else {
            0
        };
        let disk_mtime = meta_mtime.max(content_mtime);
        let content_should_exist = record.exists_on_disk && record.kind != AssetKind::MetaOnly;
        let content_missing = content_should_exist && !content_exists;
        let content_size_changed = content_should_exist
            && content_exists
            && std::fs::metadata(&content_path)
                .map(|metadata| metadata.len() != record.size)
                .unwrap_or(false);
        let content_hash_changed = options.verify_hashes
            && content_should_exist
            && content_exists
            && should_hash_asset_content(record.kind)
            && !content_size_changed
            && file_hash128(&content_path)
                .map(|hash| hash != record.content_hash)
                .unwrap_or(false);

        if !meta_exists
            || content_missing
            || disk_mtime > record.mtime_ns
            || content_size_changed
            || content_hash_changed
        {
            enqueue_with_activity(
                queue,
                activity,
                record.path.clone(),
                QueueEnqueueReason::MtimeResync,
                None,
            );
        }

        scan_completed += 1;
        if should_emit_reconcile_progress(scan_completed, scan_total) {
            emit_reconcile_progress(
                on_progress,
                StartupReconcileProgress {
                    verify_hashes: options.verify_hashes,
                    stage: "scanning",
                    total: Some(scan_total),
                    completed: Some(scan_completed),
                    queued: Some(queue.len() as u64),
                    failed: Some(0),
                },
            );
        }
    }

    let mut indexed_meta_paths = HashSet::new();
    for record in &file_records {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let asset_path = record
            .path
            .strip_suffix(".meta")
            .unwrap_or(&record.path)
            .to_string();
        if record.file_role == FileRole::Meta {
            indexed_meta_paths.insert(asset_path.clone());
        }

        let abs_path = project_root.join(&record.path);
        let file_exists = abs_path.is_file();
        let file_mtime = if file_exists {
            file_mtime_ns(&abs_path)
        } else {
            0
        };
        let file_size_changed = file_exists
            && std::fs::metadata(&abs_path)
                .map(|metadata| metadata.len() != record.size)
                .unwrap_or(false);
        let file_hash_changed = options.verify_hashes
            && file_exists
            && !file_size_changed
            && file_hash128(&abs_path)
                .map(|hash| hash != record.hash128)
                .unwrap_or(false);

        if !file_exists || file_mtime > record.mtime_ns || file_size_changed || file_hash_changed {
            enqueue_with_activity(
                queue,
                activity,
                asset_path.clone(),
                QueueEnqueueReason::MtimeResync,
                None,
            );
        }

        scan_completed += 1;
        if should_emit_reconcile_progress(scan_completed, scan_total) {
            emit_reconcile_progress(
                on_progress,
                StartupReconcileProgress {
                    verify_hashes: options.verify_hashes,
                    stage: "scanning",
                    total: Some(scan_total),
                    completed: Some(scan_completed),
                    queued: Some(queue.len() as u64),
                    failed: Some(0),
                },
            );
        }
    }

    if !options.discover_new_meta {
        return;
    }

    emit_reconcile_progress(
        on_progress,
        StartupReconcileProgress {
            verify_hashes: options.verify_hashes,
            stage: "discovering",
            total: None,
            completed: None,
            queued: Some(queue.len() as u64),
            failed: Some(0),
        },
    );

    let scan_roots = ["Assets", "Packages"];
    let mut linked_asset_roots = Vec::new();
    let mut linked_asset_rel_paths = HashSet::new();
    for root_name in &scan_roots {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let root_path = project_root.join(root_name);
        if !root_path.is_dir() {
            continue;
        }

        let walker = walkdir::WalkDir::new(&root_path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|entry| {
                if entry.file_type().is_dir() {
                    let name = entry.file_name().to_string_lossy();
                    !scanner::IGNORED_DIRS
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(&name))
                } else {
                    true
                }
            });

        for entry in walker.filter_map(|e| e.ok()) {
            if stop.load(Ordering::Relaxed) {
                break;
            }

            record_linked_asset_root(
                project_root,
                &entry,
                &mut linked_asset_roots,
                &mut linked_asset_rel_paths,
            );

            if !entry.file_type().is_file() {
                continue;
            }

            let abs_path = entry.path();
            let ext = abs_path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if ext != "meta" {
                continue;
            }

            let rel = abs_path
                .strip_prefix(project_root)
                .unwrap_or(abs_path)
                .to_string_lossy()
                .replace('\\', "/");
            let asset_path = rel.strip_suffix(".meta").unwrap_or(&rel).to_string();

            if !indexed_meta_paths.contains(&asset_path) {
                enqueue_with_activity(
                    queue,
                    activity,
                    asset_path,
                    QueueEnqueueReason::NewMetaDiscovered,
                    None,
                );
            }
        }
    }

    if stop.load(Ordering::Relaxed) {
        return;
    }
    refresh_linked_asset_roots(state, linked_watch_state, linked_asset_roots);
}

fn queue_summary_logger_loop(
    queue: Arc<DirtyQueue>,
    current: CurrentFileSlot,
    activity: Arc<RecentQueueActivityLog>,
    stop: Arc<AtomicBool>,
) {
    eprintln!("[AssetDb Watcher] queue summary logger started");
    let interval = Duration::from_secs(QUEUE_SUMMARY_LOG_INTERVAL_SECS);
    let tick = Duration::from_millis(250);
    let mut elapsed = Duration::ZERO;
    let mut last_logged_event_at: Option<u64> = None;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(tick);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        elapsed += tick;
        if elapsed < interval {
            continue;
        }
        elapsed = Duration::ZERO;

        let pending = queue.len();
        let current_file = current.lock().ok().and_then(|slot| slot.clone());
        let snapshot = activity.snapshot(RECENT_ENQUEUE_WINDOW_MS, RECENT_ENQUEUE_SAMPLE_LIMIT);

        if pending == 0 && current_file.is_none() {
            let already_logged = snapshot
                .last_event_at
                .zip(last_logged_event_at)
                .map(|(last_event_at, logged_at)| last_event_at <= logged_at)
                .unwrap_or(false);
            if snapshot.last_event_at.is_none() || already_logged {
                continue;
            }
        }

        let reasons = if snapshot.reasons.is_empty() {
            "none".to_string()
        } else {
            snapshot
                .reasons
                .iter()
                .map(|entry| format!("{}={}", entry.reason.log_label(), entry.count))
                .collect::<Vec<_>>()
                .join(", ")
        };

        eprintln!(
            "[AssetDb Watcher] queue summary: pending={}, current={}, recent={} in {}s, reasons=[{}]",
            pending,
            current_file.as_deref().unwrap_or("-"),
            snapshot.total_added,
            snapshot.window_ms / 1000,
            reasons
        );

        for file in snapshot.files.iter().take(6) {
            match file.source_path.as_deref() {
                Some(source) => eprintln!(
                    "  - {} [{}] <- {}",
                    file.path,
                    file.reason.log_label(),
                    source
                ),
                None => eprintln!("  - {} [{}]", file.path, file.reason.log_label()),
            }
        }

        if let Some(last_event_at) = snapshot.last_event_at {
            last_logged_event_at = Some(
                last_logged_event_at
                    .map(|logged_at| logged_at.max(last_event_at))
                    .unwrap_or(last_event_at),
            );
        }
    }
    eprintln!("[AssetDb Watcher] queue summary logger stopped");
}

/// Holds the relative path of the asset the worker thread is currently
/// processing, or `None` when the worker is idle. Cleared after each item.
pub type CurrentFileSlot = Arc<Mutex<Option<String>>>;

pub fn reconcile_loaded_db(
    project_root: &Path,
    graph: AssetDb,
) -> Result<(AssetDb, StartupReconcileStats), String> {
    let stop = AtomicBool::new(false);
    reconcile_loaded_db_with_options(
        project_root,
        graph,
        &stop,
        MtimeScanOptions {
            discover_new_meta: true,
            verify_hashes: true,
        },
        None,
    )
}

pub fn reconcile_loaded_db_light(
    project_root: &Path,
    graph: AssetDb,
) -> Result<(AssetDb, StartupReconcileStats), String> {
    let stop = AtomicBool::new(false);
    reconcile_loaded_db_with_options(
        project_root,
        graph,
        &stop,
        MtimeScanOptions {
            discover_new_meta: false,
            verify_hashes: false,
        },
        None,
    )
}

pub fn reconcile_loaded_db_with_cancel(
    project_root: &Path,
    graph: AssetDb,
    stop: &AtomicBool,
) -> Result<(AssetDb, StartupReconcileStats), String> {
    reconcile_loaded_db_with_options(
        project_root,
        graph,
        stop,
        MtimeScanOptions {
            discover_new_meta: true,
            verify_hashes: true,
        },
        None,
    )
}

pub fn reconcile_loaded_db_with_cancel_and_progress<F>(
    project_root: &Path,
    graph: AssetDb,
    stop: &AtomicBool,
    on_progress: F,
) -> Result<(AssetDb, StartupReconcileStats), String>
where
    F: Fn(&StartupReconcileProgress),
{
    reconcile_loaded_db_with_options(
        project_root,
        graph,
        stop,
        MtimeScanOptions {
            discover_new_meta: true,
            verify_hashes: true,
        },
        Some(&on_progress),
    )
}

fn reconcile_loaded_db_with_options(
    project_root: &Path,
    graph: AssetDb,
    stop: &AtomicBool,
    options: MtimeScanOptions,
    on_progress: ReconcileProgressCallback<'_>,
) -> Result<(AssetDb, StartupReconcileStats), String> {
    let state = Arc::new(Mutex::new(Some(graph)));
    let stats = reconcile_graph_state_with_options(
        project_root,
        state.clone(),
        stop,
        options,
        on_progress,
    )?;

    let mut guard = state.lock().map_err(|e| format!("Lock error: {}", e))?;
    let graph = guard
        .take()
        .ok_or_else(|| "AssetDb disappeared during startup reconcile".to_string())?;
    tracing::info!(
        log_module = "AssetDb Watcher",
        "startup reconcile complete: queued={}, processed={}, failed={}",
        stats.queued,
        stats.processed,
        stats.failed
    );
    Ok((graph, stats))
}

pub fn reconcile_graph_state(
    project_root: &Path,
    state: Arc<Mutex<Option<AssetDb>>>,
    verify_hashes: bool,
) -> Result<StartupReconcileStats, String> {
    let stop = AtomicBool::new(false);
    reconcile_graph_state_with_cancel(project_root, state, &stop, verify_hashes)
}

pub fn reconcile_graph_state_with_cancel(
    project_root: &Path,
    state: Arc<Mutex<Option<AssetDb>>>,
    stop: &AtomicBool,
    verify_hashes: bool,
) -> Result<StartupReconcileStats, String> {
    reconcile_graph_state_with_options(
        project_root,
        state,
        stop,
        MtimeScanOptions {
            discover_new_meta: true,
            verify_hashes,
        },
        None,
    )
}

pub fn reconcile_graph_state_with_cancel_and_progress<F>(
    project_root: &Path,
    state: Arc<Mutex<Option<AssetDb>>>,
    stop: &AtomicBool,
    verify_hashes: bool,
    on_progress: F,
) -> Result<StartupReconcileStats, String>
where
    F: Fn(&StartupReconcileProgress),
{
    reconcile_graph_state_with_options(
        project_root,
        state,
        stop,
        MtimeScanOptions {
            discover_new_meta: true,
            verify_hashes,
        },
        Some(&on_progress),
    )
}

fn reconcile_graph_state_with_options(
    project_root: &Path,
    state: Arc<Mutex<Option<AssetDb>>>,
    stop: &AtomicBool,
    options: MtimeScanOptions,
    on_progress: ReconcileProgressCallback<'_>,
) -> Result<StartupReconcileStats, String> {
    let queue = DirtyQueue::new();
    let activity = RecentQueueActivityLog::new();
    let mut stats = StartupReconcileStats::default();

    mtime_scan_once_with_options(
        &queue,
        stop,
        &state,
        project_root,
        &activity,
        options,
        None,
        on_progress,
    );
    stats.queued = queue.len() as u64;

    emit_reconcile_progress(
        on_progress,
        StartupReconcileProgress {
            verify_hashes: options.verify_hashes,
            stage: "processing",
            total: Some(stats.queued),
            completed: Some(0),
            queued: Some(queue.len() as u64),
            failed: Some(0),
        },
    );

    while let Some(rel_path) = queue.try_dequeue() {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        stats.processed += 1;
        match process_dirty_asset(&rel_path, project_root, &state, stop) {
            Ok(cascade_paths) => {
                for request in cascade_paths {
                    if enqueue_with_activity(
                        &queue,
                        &activity,
                        request.rel_path,
                        request.reason,
                        request.source_path,
                    ) {
                        stats.queued += 1;
                    }
                }
            }
            Err(error) => {
                stats.failed += 1;
                eprintln!(
                    "[AssetDb Watcher] startup reconcile failed for {}: {}",
                    rel_path, error
                );
            }
        }

        let total = stats.queued.max(stats.processed);
        if should_emit_reconcile_progress(stats.processed, total) || queue.len() == 0 {
            emit_reconcile_progress(
                on_progress,
                StartupReconcileProgress {
                    verify_hashes: options.verify_hashes,
                    stage: "processing",
                    total: Some(total),
                    completed: Some(stats.processed),
                    queued: Some(queue.len() as u64),
                    failed: Some(stats.failed),
                },
            );
        }
    }

    Ok(stats)
}

pub struct AssetDbWatcher {
    stop: Arc<AtomicBool>,
    dirty_queue: Arc<DirtyQueue>,
    current_file: CurrentFileSlot,
    recent_activity: Arc<RecentQueueActivityLog>,
    tuning: Arc<WatcherTuning>,
    os_watcher: Option<SharedOsWatcher>,
    threads: Vec<JoinHandle<()>>,
}

impl AssetDbWatcher {
    pub fn start(
        project_root: PathBuf,
        graph_state: Arc<Mutex<Option<AssetDb>>>,
        tuning: Arc<WatcherTuning>,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let dirty_queue = Arc::new(DirtyQueue::new());
        let current_file: CurrentFileSlot = Arc::new(Mutex::new(None));
        let recent_activity = Arc::new(RecentQueueActivityLog::new());
        let mut threads = Vec::new();

        let (tx, rx) = std::sync::mpsc::channel();
        let os_watcher = Arc::new(Mutex::new(
            RecommendedWatcher::new(tx, Config::default())
                .map_err(|e| format!("Failed to create file watcher: {}", e))?,
        ));
        let linked_roots = cached_linked_asset_roots(&graph_state);
        let watched_paths: SharedWatchedPaths = Arc::new(Mutex::new(HashSet::new()));
        let linked_watched_paths: SharedWatchedPaths = Arc::new(Mutex::new(HashSet::new()));

        for dir_name in &["Assets", "Packages"] {
            let watch_path = project_root.join(dir_name);
            if watch_path.is_dir() {
                match watch_path_once(&os_watcher, &watched_paths, &watch_path, dir_name) {
                    Ok(true) => {
                        eprintln!("[AssetDb Watcher] watching: {}", watch_path.display());
                    }
                    Ok(false) => {}
                    Err(error) => return Err(error),
                }
            }
        }

        let linked_roots = Arc::new(RwLock::new(linked_roots));
        let linked_watch_state = LinkedAssetWatchState {
            os_watcher: os_watcher.clone(),
            watched_paths: watched_paths.clone(),
            linked_watched_paths: linked_watched_paths.clone(),
            linked_roots: linked_roots.clone(),
        };
        let initial_linked_roots = linked_roots
            .read()
            .map(|roots| roots.clone())
            .unwrap_or_default();
        install_linked_asset_root_watches(&linked_watch_state, &initial_linked_roots);

        let queue_ev = dirty_queue.clone();
        let stop_ev = stop.clone();
        let root_ev = project_root.clone();
        let linked_roots_ev = linked_roots.clone();
        let activity_ev = recent_activity.clone();
        let event_thread = std::thread::Builder::new()
            .name("refgraph-events".into())
            .spawn(move || {
                event_receiver_loop(rx, queue_ev, stop_ev, root_ev, linked_roots_ev, activity_ev);
            })
            .map_err(|e| format!("Failed to spawn event thread: {}", e))?;
        threads.push(event_thread);

        let queue_log = dirty_queue.clone();
        let stop_log = stop.clone();
        let current_log = current_file.clone();
        let activity_log = recent_activity.clone();
        let log_thread = std::thread::Builder::new()
            .name("refgraph-queue-log".into())
            .spawn(move || {
                queue_summary_logger_loop(queue_log, current_log, activity_log, stop_log);
            })
            .map_err(|e| format!("Failed to spawn queue summary logger thread: {}", e))?;
        threads.push(log_thread);

        for index in 0..MAX_WORKER_THREADS {
            let queue_wk = dirty_queue.clone();
            let stop_wk = stop.clone();
            let state_wk = graph_state.clone();
            let root_wk = project_root.clone();
            let current_wk = current_file.clone();
            let tuning_wk = tuning.clone();
            let activity_wk = recent_activity.clone();
            let worker_thread = std::thread::Builder::new()
                .name(format!("refgraph-worker-{}", index))
                .spawn(move || {
                    worker_loop(
                        index,
                        queue_wk,
                        stop_wk,
                        state_wk,
                        root_wk,
                        current_wk,
                        tuning_wk,
                        activity_wk,
                    );
                })
                .map_err(|e| format!("Failed to spawn worker thread {}: {}", index, e))?;
            threads.push(worker_thread);
        }

        let queue_mt = dirty_queue.clone();
        let stop_mt = stop.clone();
        let state_mt = graph_state.clone();
        let root_mt = project_root.clone();
        let activity_mt = recent_activity.clone();
        let linked_watch_state_mt = linked_watch_state.clone();
        let mtime_thread = std::thread::Builder::new()
            .name("refgraph-mtime".into())
            .spawn(move || {
                mtime_scanner_loop(
                    queue_mt,
                    stop_mt,
                    state_mt,
                    root_mt,
                    activity_mt,
                    linked_watch_state_mt,
                );
            })
            .map_err(|e| format!("Failed to spawn mtime scanner thread: {}", e))?;
        threads.push(mtime_thread);

        eprintln!("[AssetDb Watcher] started for {}", project_root.display());

        Ok(Self {
            stop,
            dirty_queue,
            current_file,
            recent_activity,
            tuning,
            os_watcher: Some(os_watcher),
            threads,
        })
    }

    pub fn tuning(&self) -> &Arc<WatcherTuning> {
        &self.tuning
    }

    fn request_stop(&self) {
        let was_stopped = self.stop.swap(true, Ordering::Relaxed);
        self.dirty_queue.condvar.notify_all();
        if !was_stopped {
            eprintln!("[AssetDb Watcher] stop signal sent");
        }
    }

    fn join_threads(&mut self) {
        self.os_watcher.take();
        let current_id = std::thread::current().id();
        for handle in self.threads.drain(..) {
            if handle.thread().id() == current_id {
                continue;
            }
            if let Err(err) = handle.join() {
                eprintln!("[AssetDb Watcher] worker thread join failed: {:?}", err);
            }
        }
    }

    pub fn stop(&self) {
        self.request_stop();
    }

    pub fn stop_and_join(mut self) {
        self.request_stop();
        self.join_threads();
    }

    pub fn queue_len(&self) -> usize {
        self.dirty_queue.len()
    }

    /// Snapshot the relative path of the asset currently being processed by
    /// the worker thread, or `None` when idle.
    pub fn current_file(&self) -> Option<String> {
        self.current_file.lock().ok().and_then(|g| g.clone())
    }

    pub fn recent_activity(&self) -> RecentQueueActivity {
        self.recent_activity
            .snapshot(RECENT_ENQUEUE_WINDOW_MS, RECENT_ENQUEUE_SAMPLE_LIMIT)
    }
}

impl Drop for AssetDbWatcher {
    fn drop(&mut self) {
        self.request_stop();
        self.join_threads();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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

    fn write_asset(root: &Path, rel_path: &str, content: &[u8], guid_hex: &str) {
        let abs_path = root.join(rel_path);
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent).expect("create asset parent");
        }
        std::fs::write(&abs_path, content).expect("write asset");
        std::fs::write(
            root.join(format!("{}.meta", rel_path)),
            format!("fileFormatVersion: 2\nguid: {}\n", guid_hex),
        )
        .expect("write asset meta");
    }

    fn scan_test_graph(root: &Path) -> AssetDb {
        let mut graph = AssetDb::open(root).expect("open asset db");
        graph.full_scan(|_| {}).expect("scan asset db");
        graph
    }

    #[test]
    fn watcher_maps_external_events_from_symlinked_asset_root() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(root.join("Packages")).expect("create packages dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Hero.prefab.meta"), b"fileFormatVersion: 2\n")
            .expect("write linked meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/Linked")) {
            return;
        }

        let linked_roots =
            sort_linked_asset_roots(scanner::scan_directory(&root).linked_asset_roots);
        assert!(linked_roots
            .iter()
            .any(|entry| entry.link_rel_path == "Assets/Linked"));

        let mapped = to_asset_rel_paths_and_reasons(
            &root,
            &external.join("Hero.prefab.meta"),
            linked_roots.as_slice(),
        );
        assert_eq!(
            mapped,
            vec![(
                "Assets/Linked/Hero.prefab".to_string(),
                QueueEnqueueReason::MetaChanged
            )]
        );
    }

    #[test]
    fn watcher_maps_external_events_to_each_symlinked_asset_alias() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(root.join("Packages")).expect("create packages dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Hero.prefab.meta"), b"fileFormatVersion: 2\n")
            .expect("write linked meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/LinkedA")) {
            return;
        }
        if !create_dir_symlink_or_skip(&external, &root.join("Packages/LinkedB")) {
            return;
        }

        let linked_roots =
            sort_linked_asset_roots(scanner::scan_directory(&root).linked_asset_roots);
        let mapped = to_asset_rel_paths_and_reasons(
            &root,
            &external.join("Hero.prefab.meta"),
            linked_roots.as_slice(),
        );
        assert_eq!(
            mapped,
            vec![
                (
                    "Assets/LinkedA/Hero.prefab".to_string(),
                    QueueEnqueueReason::MetaChanged
                ),
                (
                    "Packages/LinkedB/Hero.prefab".to_string(),
                    QueueEnqueueReason::MetaChanged
                )
            ]
        );
    }

    #[test]
    fn watcher_loads_linked_roots_from_asset_db_cache() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(root.join("Packages")).expect("create packages dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(
            external.join("Hero.prefab"),
            b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Hero\n",
        )
        .expect("write linked prefab");
        std::fs::write(
            external.join("Hero.prefab.meta"),
            b"fileFormatVersion: 2\nguid: 22222222222222222222222222222222\n",
        )
        .expect("write linked prefab meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/Linked")) {
            return;
        }

        let mut graph = AssetDb::open(&root).expect("open asset db");
        graph.full_scan(|_| {}).expect("scan asset db");
        let state = Arc::new(Mutex::new(Some(graph)));

        let linked_roots = cached_linked_asset_roots(&state);
        assert_eq!(linked_roots.len(), 1);
        assert_eq!(linked_roots[0].link_rel_path, "Assets/Linked");
        assert_eq!(
            linked_roots[0].target_path,
            dunce::canonicalize(&external).expect("canonical external target")
        );
    }

    #[test]
    fn linked_watch_prune_removes_stale_keys_and_keeps_active_keys() {
        let current: SharedWatchedPaths = Arc::new(Mutex::new(HashSet::from([
            PathBuf::from("C:/shared-a"),
            PathBuf::from("C:/shared-b"),
            PathBuf::from("C:/shared-c"),
        ])));
        let desired = HashSet::from([PathBuf::from("C:/shared-b"), PathBuf::from("C:/shared-c")]);

        let stale = prune_stale_linked_watch_keys(&current, &desired);

        assert_eq!(stale, vec![PathBuf::from("C:/shared-a")]);
        let remaining = current.lock().expect("lock linked watched paths");
        assert!(!remaining.contains(&PathBuf::from("C:/shared-a")));
        assert!(remaining.contains(&PathBuf::from("C:/shared-b")));
        assert!(remaining.contains(&PathBuf::from("C:/shared-c")));
    }

    #[test]
    fn mtime_discovery_persists_new_symlinked_asset_root() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets/Base")).expect("create base assets dir");
        std::fs::create_dir_all(root.join("Packages")).expect("create packages dir");
        std::fs::create_dir_all(&external).expect("create external target");
        write_asset(
            &root,
            "Assets/Base/Existing.prefab",
            b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Existing\n",
            "33333333333333333333333333333333",
        );

        let graph = scan_test_graph(&root);
        let state = Arc::new(Mutex::new(Some(graph)));
        assert!(cached_linked_asset_roots(&state).is_empty());

        std::fs::write(
            external.join("Hero.prefab"),
            b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Hero\n",
        )
        .expect("write linked prefab");
        std::fs::write(
            external.join("Hero.prefab.meta"),
            b"fileFormatVersion: 2\nguid: 44444444444444444444444444444444\n",
        )
        .expect("write linked prefab meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/Linked")) {
            return;
        }

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);

        let linked_roots = cached_linked_asset_roots(&state);
        assert_eq!(linked_roots.len(), 1);
        assert_eq!(linked_roots[0].link_rel_path, "Assets/Linked");
        assert_eq!(
            linked_roots[0].target_path,
            dunce::canonicalize(&external).expect("canonical external target")
        );
        let mut queued_paths = Vec::new();
        while let Some(path) = queue.try_dequeue() {
            queued_paths.push(path);
        }
        assert!(queued_paths
            .iter()
            .any(|path| path == "Assets/Linked/Hero.prefab"));
    }

    #[test]
    fn mtime_discovery_finds_linked_asset_root_after_empty_scan() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(root.join("Packages")).expect("create packages dir");
        std::fs::create_dir_all(&external).expect("create external target");

        let graph = scan_test_graph(&root);
        let state = Arc::new(Mutex::new(Some(graph)));
        assert!(cached_linked_asset_roots(&state).is_empty());

        std::fs::write(
            external.join("Hero.prefab"),
            b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Hero\n",
        )
        .expect("write linked prefab");
        std::fs::write(
            external.join("Hero.prefab.meta"),
            b"fileFormatVersion: 2\nguid: 55555555555555555555555555555555\n",
        )
        .expect("write linked prefab meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/Linked")) {
            return;
        }

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);

        let linked_roots = cached_linked_asset_roots(&state);
        assert_eq!(linked_roots.len(), 1);
        assert_eq!(linked_roots[0].link_rel_path, "Assets/Linked");
        assert_eq!(
            linked_roots[0].target_path,
            dunce::canonicalize(&external).expect("canonical external target")
        );

        let mut queued_paths = Vec::new();
        while let Some(path) = queue.try_dequeue() {
            queued_paths.push(path);
        }
        assert!(queued_paths
            .iter()
            .any(|path| path == "Assets/Linked/Hero.prefab"));
    }

    #[test]
    fn recent_queue_activity_snapshot_groups_and_limits() {
        let log = RecentQueueActivityLog::new();
        log.record(
            "Assets/UI/Hud.prefab".to_string(),
            QueueEnqueueReason::ContentChanged,
            None,
        );
        log.record(
            "Assets/UI/Hud.meta".to_string(),
            QueueEnqueueReason::MetaChanged,
            None,
        );
        log.record(
            "Assets/Data/HudConfig.asset".to_string(),
            QueueEnqueueReason::ScriptCascade,
            Some("Assets/Scripts/HudConfig.cs".to_string()),
        );

        let snapshot = log.snapshot(RECENT_ENQUEUE_WINDOW_MS, 2);
        assert_eq!(snapshot.total_added, 3);
        assert_eq!(snapshot.files.len(), 2);
        assert_eq!(snapshot.files[0].path, "Assets/Data/HudConfig.asset");
        assert_eq!(
            snapshot.files[0].source_path.as_deref(),
            Some("Assets/Scripts/HudConfig.cs")
        );

        let mut counts = snapshot
            .reasons
            .into_iter()
            .map(|entry| (entry.reason, entry.count))
            .collect::<HashMap<_, _>>();
        assert_eq!(counts.remove(&QueueEnqueueReason::ContentChanged), Some(1));
        assert_eq!(counts.remove(&QueueEnqueueReason::MetaChanged), Some(1));
        assert_eq!(counts.remove(&QueueEnqueueReason::ScriptCascade), Some(1));
    }

    #[test]
    fn mtime_scan_once_resyncs_duplicate_guid_alias_meta_changes() {
        let root =
            std::env::temp_dir().join(format!("locus-watcher-meta-resync-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets/Game")).expect("create temp assets");

        let asset_path = "Assets/Game/Alias.prefab";
        let meta_path = root.join(format!("{}.meta", asset_path));
        std::fs::write(
            &meta_path,
            b"fileFormatVersion: 2\nguid: 1234567890abcdef1234567890abcdef\n",
        )
        .expect("write meta");
        let meta_mtime = file_mtime_ns(&meta_path);
        let meta_size = std::fs::metadata(&meta_path).expect("meta metadata").len();
        let meta_hash = hash128(&std::fs::read(&meta_path).expect("read meta for hash"));

        let graph = AssetDb::open(&root).expect("open asset db");
        db::upsert_file(
            &graph.conn,
            &format!("{}.meta", asset_path),
            FileRole::Meta,
            meta_mtime.saturating_sub(1),
            meta_size,
            &meta_hash,
            Some(&parse_guid_hex("1234567890abcdef1234567890abcdef").unwrap()),
        )
        .expect("seed meta bookkeeping");

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let state = Arc::new(Mutex::new(Some(graph)));
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);

        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.dequeue(&AtomicBool::new(false)),
            Some(asset_path.to_string())
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn light_reconcile_skips_new_meta_discovery() {
        let root =
            std::env::temp_dir().join(format!("locus-watcher-light-reconcile-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets/Game")).expect("create temp assets");

        let asset_path = "Assets/Game/New.prefab";
        std::fs::write(root.join(asset_path), b"%YAML 1.1\n").expect("write asset");
        std::fs::write(
            root.join(format!("{}.meta", asset_path)),
            b"fileFormatVersion: 2\nguid: 33333333333333333333333333333333\n",
        )
        .expect("write meta");

        let graph = AssetDb::open(&root).expect("open asset db");
        let (graph, stats) =
            reconcile_loaded_db_light(&root, graph).expect("light reconcile asset db");

        assert_eq!(stats.queued, 0);
        assert_eq!(stats.processed, 0);
        assert_eq!(
            db::resolve_guid_by_path(&graph.conn, asset_path).unwrap(),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mtime_scan_once_ignores_directory_mtime_for_meta_only_assets() {
        let root = std::env::temp_dir().join(format!("locus-watcher-dir-mtime-{}", Uuid::new_v4()));
        let folder_path = root.join("Assets/Folder");
        std::fs::create_dir_all(&folder_path).expect("create temp folder");

        let asset_path = "Assets/Folder";
        let meta_path = root.join("Assets/Folder.meta");
        let meta_bytes = b"fileFormatVersion: 2\nguid: fedcba0987654321fedcba0987654321\n";
        std::fs::write(&meta_path, meta_bytes).expect("write folder meta");
        let meta_mtime = file_mtime_ns(&meta_path);
        let meta_size = std::fs::metadata(&meta_path).expect("meta metadata").len();
        let meta_hash = hash128(meta_bytes);
        let guid = parse_guid_hex("fedcba0987654321fedcba0987654321").unwrap();

        let mut graph = AssetDb::open(&root).expect("open asset db");
        let node = AssetNode {
            guid,
            path: asset_path.to_string(),
            ext: String::new(),
            kind: AssetKind::MetaOnly,
            exists_on_disk: false,
            mtime_ns: meta_mtime,
            size: 0,
            content_hash: [0u8; 16],
            meta_hash,
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: String::new(),
            script_inheritance_search: String::new(),
        };
        db::atomic_update_asset(
            &mut graph.conn,
            &node,
            &[],
            &[],
            &[(
                format!("{}.meta", asset_path),
                FileRole::Meta,
                meta_mtime,
                meta_size,
                meta_hash,
            )],
        )
        .expect("seed meta-only asset");

        std::fs::write(folder_path.join("child.txt"), b"x").expect("write child file");

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let state = Arc::new(Mutex::new(Some(graph)));
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);

        assert_eq!(queue.len(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mtime_resync_converges_after_same_path_guid_replacement() {
        let root =
            std::env::temp_dir().join(format!("locus-watcher-stale-guid-{}", Uuid::new_v4()));
        let asset_path = "Packages/com.farlocus.locus/Editor";
        let folder_path = root.join(asset_path);
        std::fs::create_dir_all(&folder_path).expect("create package folder");

        let meta_path = root.join(format!("{}.meta", asset_path));
        let meta_bytes =
            b"fileFormatVersion: 2\nguid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\nfolderAsset: yes\n";
        std::fs::write(&meta_path, meta_bytes).expect("write folder meta");
        let meta_mtime = file_mtime_ns(&meta_path);
        let stale_mtime = meta_mtime.saturating_sub(1);
        let stale_guid = parse_guid_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();

        let mut graph = AssetDb::open(&root).expect("open asset db");
        let stale_node = AssetNode {
            guid: stale_guid,
            path: asset_path.to_string(),
            ext: String::new(),
            kind: AssetKind::MetaOnly,
            exists_on_disk: false,
            mtime_ns: stale_mtime,
            size: 0,
            content_hash: [0u8; 16],
            meta_hash: hash128(b"old-meta"),
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: String::new(),
            script_inheritance_search: String::new(),
        };
        db::atomic_update_asset(
            &mut graph.conn,
            &stale_node,
            &[],
            &[],
            &[(
                format!("{}.meta", asset_path),
                FileRole::Meta,
                stale_mtime,
                meta_bytes.len() as u64,
                hash128(b"old-meta"),
            )],
        )
        .expect("seed stale same-path asset");

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let state = Arc::new(Mutex::new(Some(graph)));
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.dequeue(&stop), Some(asset_path.to_string()));
        process_dirty_asset(asset_path, &root, &state, &stop).expect("process stale resync");

        mtime_scan_once(&queue, &stop, &state, &root, &activity, true);
        assert_eq!(queue.len(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_reconcile_updates_moved_asset_path_and_keeps_incoming_refs() {
        let root = std::env::temp_dir().join(format!("locus-watcher-move-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets")).expect("create temp assets");
        let target_guid = parse_guid_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let scene_guid = parse_guid_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let target_content = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Target\n";
        let scene_content = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Root\n--- !u!114 &2000\nMonoBehaviour:\n  m_GameObject: {fileID: 1000}\n  target: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}\n";
        write_asset(
            &root,
            "Assets/Prefabs/Target.prefab",
            target_content,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        write_asset(
            &root,
            "Assets/Scenes/Main.unity",
            scene_content,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );

        let graph = scan_test_graph(&root);
        std::fs::create_dir_all(root.join("Assets/Moved")).expect("create moved dir");
        std::fs::rename(
            root.join("Assets/Prefabs/Target.prefab"),
            root.join("Assets/Moved/Target.prefab"),
        )
        .expect("move asset");
        std::fs::rename(
            root.join("Assets/Prefabs/Target.prefab.meta"),
            root.join("Assets/Moved/Target.prefab.meta"),
        )
        .expect("move asset meta");

        let (graph, stats) = reconcile_loaded_db(&root, graph).expect("reconcile loaded db");
        assert!(stats.processed >= 2);
        assert_eq!(
            graph
                .resolve_guid_by_path("Assets/Prefabs/Target.prefab")
                .unwrap(),
            None
        );
        assert_eq!(
            graph
                .resolve_guid_by_path("Assets/Moved/Target.prefab")
                .unwrap(),
            Some(target_guid)
        );
        assert_eq!(
            graph.resolve_path_by_guid(&target_guid).unwrap().as_deref(),
            Some("Assets/Moved/Target.prefab")
        );

        let refs = graph.get_direct_refs(&target_guid).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].src_guid, scene_guid);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_reconcile_detects_same_mtime_same_size_yaml_hash_change() {
        let root = std::env::temp_dir().join(format!("locus-watcher-hash-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets")).expect("create temp assets");
        let source_guid = parse_guid_hex("cccccccccccccccccccccccccccccccc").unwrap();
        let old_target_guid = parse_guid_hex("dddddddddddddddddddddddddddddddd").unwrap();
        let new_target_guid = parse_guid_hex("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").unwrap();
        let target_content = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Target\n";
        let old_source = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Root\n--- !u!114 &2000\nMonoBehaviour:\n  m_GameObject: {fileID: 1000}\n  target: {fileID: 100100000, guid: dddddddddddddddddddddddddddddddd, type: 3}\n";
        let new_source = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Root\n--- !u!114 &2000\nMonoBehaviour:\n  m_GameObject: {fileID: 1000}\n  target: {fileID: 100100000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 3}\n";
        assert_eq!(old_source.len(), new_source.len());
        write_asset(
            &root,
            "Assets/Prefabs/OldTarget.prefab",
            target_content,
            "dddddddddddddddddddddddddddddddd",
        );
        write_asset(
            &root,
            "Assets/Prefabs/NewTarget.prefab",
            target_content,
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        );
        write_asset(
            &root,
            "Assets/Scenes/Main.unity",
            old_source,
            "cccccccccccccccccccccccccccccccc",
        );

        let graph = scan_test_graph(&root);
        let source_abs = root.join("Assets/Scenes/Main.unity");
        std::fs::write(&source_abs, new_source).expect("replace source content");
        let current_mtime = file_mtime_ns(&source_abs);
        let old_hash = hash128(old_source);
        graph
            .conn
            .execute(
                "UPDATE assets
                 SET mtime_ns = ?1, size = ?2, content_hash = ?3
                 WHERE path = ?4",
                rusqlite::params![
                    current_mtime as i64,
                    new_source.len() as i64,
                    old_hash.as_slice(),
                    "Assets/Scenes/Main.unity"
                ],
            )
            .expect("force stale asset mtime");
        graph
            .conn
            .execute(
                "UPDATE files
                 SET mtime_ns = ?1, size = ?2, hash128 = ?3
                 WHERE path = ?4",
                rusqlite::params![
                    current_mtime as i64,
                    new_source.len() as i64,
                    old_hash.as_slice(),
                    "Assets/Scenes/Main.unity"
                ],
            )
            .expect("force stale file mtime");

        let (graph, stats) = reconcile_loaded_db(&root, graph).expect("reconcile loaded db");
        assert!(stats.processed >= 1);

        let deps = graph.get_direct_deps(&source_guid).unwrap();
        assert!(deps.iter().any(|edge| edge.dst_guid == new_target_guid));
        assert!(!deps.iter().any(|edge| edge.dst_guid == old_target_guid));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn incremental_yaml_update_keeps_script_class_in_ref_path() {
        let root = std::env::temp_dir().join(format!("locus-watcher-refpath-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets")).expect("create temp assets");
        let script_guid = parse_guid_hex("1234567890abcdef1234567890abcdef").unwrap();
        let prefab_guid = parse_guid_hex("abcdef1234567890abcdef1234567890").unwrap();
        let script_content = b"public class FooBehaviour : UnityEngine.MonoBehaviour {}\n";
        let prefab_content = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Root\n--- !u!114 &2000\nMonoBehaviour:\n  m_GameObject: {fileID: 1000}\n  m_Script: {fileID: 11500000, guid: 1234567890abcdef1234567890abcdef, type: 3}\n";
        write_asset(
            &root,
            "Assets/Scripts/FooBehaviour.cs",
            script_content,
            "1234567890abcdef1234567890abcdef",
        );
        write_asset(
            &root,
            "Assets/Prefabs/Root.prefab",
            prefab_content,
            "abcdef1234567890abcdef1234567890",
        );

        let graph = scan_test_graph(&root);
        let state = Arc::new(Mutex::new(Some(graph)));
        let stop = AtomicBool::new(false);
        process_dirty_asset("Assets/Prefabs/Root.prefab", &root, &state, &stop)
            .expect("process prefab");

        let guard = state.lock().expect("lock graph");
        let graph = guard.as_ref().expect("graph exists");
        let deps = graph.get_direct_deps(&prefab_guid).unwrap();
        let script_edge = deps
            .iter()
            .find(|edge| edge.dst_guid == script_guid)
            .expect("script edge");
        assert_eq!(
            script_edge.ref_path.as_deref(),
            Some("Root/FooBehaviour/m_Script")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mtime_scan_once_resyncs_deleted_content_with_unchanged_meta() {
        let root =
            std::env::temp_dir().join(format!("locus-watcher-content-delete-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets/Game")).expect("create temp assets");

        let asset_path = "Assets/Game/Hud.prefab";
        let asset_abs = root.join(asset_path);
        let meta_abs = root.join(format!("{}.meta", asset_path));
        let guid = parse_guid_hex("11111111111111111111111111111111").unwrap();
        let meta_bytes = b"fileFormatVersion: 2\nguid: 11111111111111111111111111111111\n";
        let content_bytes = b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Hud\n";
        std::fs::write(&meta_abs, meta_bytes).expect("write meta");
        std::fs::write(&asset_abs, content_bytes).expect("write asset");

        let meta_mtime = file_mtime_ns(&meta_abs);
        let asset_mtime = file_mtime_ns(&asset_abs);
        let meta_size = std::fs::metadata(&meta_abs).expect("meta metadata").len();
        let asset_size = std::fs::metadata(&asset_abs).expect("asset metadata").len();
        let meta_hash = hash128(meta_bytes);
        let content_hash = hash128(content_bytes);

        let mut graph = AssetDb::open(&root).expect("open asset db");
        let node = AssetNode {
            guid,
            path: asset_path.to_string(),
            ext: "prefab".to_string(),
            kind: AssetKind::Prefab,
            exists_on_disk: true,
            mtime_ns: meta_mtime.max(asset_mtime),
            size: asset_size,
            content_hash,
            meta_hash,
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: String::new(),
            script_inheritance_search: String::new(),
        };
        db::atomic_update_asset(
            &mut graph.conn,
            &node,
            &[],
            &[],
            &[
                (
                    format!("{}.meta", asset_path),
                    FileRole::Meta,
                    meta_mtime,
                    meta_size,
                    meta_hash,
                ),
                (
                    asset_path.to_string(),
                    FileRole::YamlAsset,
                    asset_mtime,
                    asset_size,
                    content_hash,
                ),
            ],
        )
        .expect("seed asset");

        std::fs::remove_file(&asset_abs).expect("delete content asset");

        let queue = DirtyQueue::new();
        let stop = AtomicBool::new(false);
        let state = Arc::new(Mutex::new(Some(graph)));
        let activity = RecentQueueActivityLog::new();
        mtime_scan_once(&queue, &stop, &state, &root, &activity, false);

        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.dequeue(&AtomicBool::new(false)),
            Some(asset_path.to_string())
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn process_dirty_asset_respects_stop_before_db_write() {
        let root = std::env::temp_dir().join(format!("locus-watcher-stop-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets/Game")).expect("create temp assets");

        let asset_path = "Assets/Game/Stopped.prefab";
        std::fs::write(
            root.join(format!("{}.meta", asset_path)),
            b"fileFormatVersion: 2\nguid: 22222222222222222222222222222222\n",
        )
        .expect("write meta");
        std::fs::write(root.join(asset_path), b"%YAML 1.1\n").expect("write asset");

        let graph = AssetDb::open(&root).expect("open asset db");
        let state = Arc::new(Mutex::new(Some(graph)));
        let stop = AtomicBool::new(true);

        process_dirty_asset(asset_path, &root, &state, &stop).expect("stopped processing");

        let guard = state.lock().expect("lock state");
        let graph = guard.as_ref().expect("graph still present");
        assert_eq!(
            db::resolve_guid_by_path(&graph.conn, asset_path).unwrap(),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
