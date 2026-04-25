//

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;

use super::db;
use super::meta_parser;
use super::scanner::{self, P1_EXTENSIONS};
use super::script_parser;
use super::types::*;
use super::AssetDb;
use crate::unity_yaml;

const MTIME_SCAN_INTERVAL_SECS: u64 = 60;
const NEW_META_DISCOVERY_INTERVAL_SECS: u64 = 10 * 60;

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

fn to_asset_rel_path_and_reason(
    project_root: &Path,
    abs_path: &Path,
) -> Option<(String, QueueEnqueueReason)> {
    let rel = abs_path
        .strip_prefix(project_root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");

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

fn file_mtime_ns(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .as_ref()
        .map(scanner::get_mtime_ns)
        .unwrap_or(0)
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

    if asset_exists && is_yaml_asset_ext(&ext) {
        let content = std::fs::read(&asset_abs)
            .map_err(|e| format!("Failed to read {}: {}", asset_abs.display(), e))?;
        let refs = unity_yaml::extract_refs(&content);
        content_hash = hash128(&content);
        let metadata = std::fs::metadata(&asset_abs).ok();
        asset_mtime = metadata.as_ref().map(scanner::get_mtime_ns).unwrap_or(0);
        asset_size = metadata.map(|m| m.len()).unwrap_or(0);
        kind = AssetKind::from_ext(&ext);

        edges = refs
            .iter()
            .map(|r| RefEdge {
                src_guid: guid,
                dst_guid: r.dst_guid,
                dst_file_id: r.dst_file_id,
                class_id_hint: r.class_id_hint,
                field_hint: r.field_hint.clone(),
                ref_path: r.ref_path.clone(),
            })
            .collect();

        if kind == AssetKind::GenericAsset {
            let script_guid = unity_yaml::parse_yaml_docs(&content)
                .into_iter()
                .find(|doc| doc.doc_index == 0 && doc.class_id == 114)
                .and_then(|doc| doc.m_script_guid);

            if let Some(script_guid) = script_guid {
                let guard = graph_state
                    .lock()
                    .map_err(|e| format!("Lock error: {}", e))?;
                if let Some(graph) = guard.as_ref() {
                    if let Some(meta) = db::get_stored_script_metadata(&graph.conn, &script_guid)? {
                        if meta.inherits_scriptable_object() {
                            stored_script_meta = Some(meta);
                        }
                    }
                }
            }
        }
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
        db::atomic_update_asset(&mut graph.conn, &node, &edges, &file_records)?;
        if ext == "cs" {
            let source_path = Some(asset_rel_path.to_string());
            for path in db::find_scriptable_asset_paths_for_script(&graph.conn, &guid)? {
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

fn event_receiver_loop(
    rx: std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>,
    queue: Arc<DirtyQueue>,
    stop: Arc<AtomicBool>,
    project_root: PathBuf,
    activity: Arc<RecentQueueActivityLog>,
) {
    eprintln!("[AssetDb Watcher] event receiver thread started");
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if let Some((rel, reason)) = to_asset_rel_path_and_reason(&project_root, path) {
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
) {
    mtime_scan_once(&queue, &stop, &state, &project_root, &activity, true);

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
            mtime_scan_once(
                &queue,
                &stop,
                &state,
                &project_root,
                &activity,
                discover_new_meta,
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
    if stop.load(Ordering::Relaxed) {
        return;
    }

    let (asset_records, indexed_meta_mtimes): (Vec<db::AssetMtimeRecord>, HashMap<String, u64>) = {
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
                let meta_assets = match db::get_all_meta_asset_mtimes(&graph.conn) {
                    Ok(v) => v.into_iter().collect(),
                    Err(e) => {
                        eprintln!("[AssetDb Watcher] meta mtime query error: {}", e);
                        return;
                    }
                };
                (mtimes, meta_assets)
            }
            None => return,
        }
    };

    if asset_records.is_empty() && indexed_meta_mtimes.is_empty() {
        return;
    }

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

        if !meta_exists || content_missing || disk_mtime > record.mtime_ns {
            enqueue_with_activity(
                queue,
                activity,
                record.path.clone(),
                QueueEnqueueReason::MtimeResync,
                None,
            );
        }
    }

    for (asset_path, stored_meta_mtime) in &indexed_meta_mtimes {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let meta_path = project_root.join(format!("{}.meta", asset_path));
        let meta_exists = meta_path.is_file();
        let meta_mtime = if meta_exists {
            file_mtime_ns(&meta_path)
        } else {
            0
        };

        if !meta_exists || meta_mtime > *stored_meta_mtime {
            enqueue_with_activity(
                queue,
                activity,
                asset_path.clone(),
                QueueEnqueueReason::MtimeResync,
                None,
            );
        }
    }

    if !discover_new_meta {
        return;
    }

    let scan_roots = ["Assets", "Packages"];
    for root_name in &scan_roots {
        let root_path = project_root.join(root_name);
        if !root_path.is_dir() {
            continue;
        }

        let walker = walkdir::WalkDir::new(&root_path)
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

            if !indexed_meta_mtimes.contains_key(&asset_path) {
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

        if pending == 0 && current_file.is_none() && snapshot.total_added == 0 {
            continue;
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
    }
    eprintln!("[AssetDb Watcher] queue summary logger stopped");
}

/// Holds the relative path of the asset the worker thread is currently
/// processing, or `None` when the worker is idle. Cleared after each item.
pub type CurrentFileSlot = Arc<Mutex<Option<String>>>;

pub struct AssetDbWatcher {
    stop: Arc<AtomicBool>,
    dirty_queue: Arc<DirtyQueue>,
    current_file: CurrentFileSlot,
    recent_activity: Arc<RecentQueueActivityLog>,
    tuning: Arc<WatcherTuning>,
    os_watcher: Option<RecommendedWatcher>,
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
        let mut os_watcher = RecommendedWatcher::new(tx, Config::default())
            .map_err(|e| format!("Failed to create file watcher: {}", e))?;

        for dir_name in &["Assets", "Packages"] {
            let watch_path = project_root.join(dir_name);
            if watch_path.is_dir() {
                os_watcher
                    .watch(&watch_path, RecursiveMode::Recursive)
                    .map_err(|e| format!("Failed to watch {}: {}", dir_name, e))?;
                eprintln!("[AssetDb Watcher] watching: {}", watch_path.display());
            }
        }

        let queue_ev = dirty_queue.clone();
        let stop_ev = stop.clone();
        let root_ev = project_root.clone();
        let activity_ev = recent_activity.clone();
        let event_thread = std::thread::Builder::new()
            .name("refgraph-events".into())
            .spawn(move || {
                event_receiver_loop(rx, queue_ev, stop_ev, root_ev, activity_ev);
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
        let mtime_thread = std::thread::Builder::new()
            .name("refgraph-mtime".into())
            .spawn(move || {
                mtime_scanner_loop(queue_mt, stop_mt, state_mt, root_mt, activity_mt);
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
