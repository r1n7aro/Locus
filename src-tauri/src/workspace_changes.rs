use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use serde::Serialize;

pub const WORKSPACE_FILE_CHANGED_EVENT: &str = "workspace-file-changed";

pub const MAX_PENDING_PATHS_PER_WORKSPACE: usize = 16_384;
const MAX_DUPLICATE_FINGERPRINT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChangeKind {
    Upsert,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChangeSource {
    OsWatcher,
    LocusWrite,
    PluginInstall,
    Reconcile,
}

/// Normalized event envelope shared by immediate consumers. Durable state is
/// maintained by each projection: RefGraph keeps its own dirty queue/DB,
/// while the Unity projection coalesces compile inputs in this hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeEvent {
    pub seq: u64,
    pub generation: u64,
    pub path: String,
    pub kind: WorkspaceChangeKind,
    pub source: WorkspaceChangeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeJournalHealth {
    Unverified,
    Healthy,
    Suspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityAssetSyncMode {
    None,
    Targeted,
    Full,
}

impl UnityAssetSyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Targeted => "targeted",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone)]
struct PendingWorkspaceChange {
    seq: u64,
    kind: WorkspaceChangeKind,
    _source: WorkspaceChangeSource,
}

#[derive(Debug)]
struct WorkspaceChangeState {
    next_seq: u64,
    generation: u64,
    compile_ack: u64,
    health: ChangeJournalHealth,
    health_reason: String,
    watcher_count: usize,
    watcher_started_once: bool,
    pending: HashMap<String, PendingWorkspaceChange>,
    rescan_count: u64,
    watch_error_count: u64,
    overflow_count: u64,
    last_sync_mode: Option<UnityAssetSyncMode>,
    last_sync_reason: Option<String>,
}

impl Default for WorkspaceChangeState {
    fn default() -> Self {
        Self {
            next_seq: 0,
            generation: 1,
            compile_ack: 0,
            health: ChangeJournalHealth::Unverified,
            health_reason: "startup_unverified".to_string(),
            watcher_count: 0,
            watcher_started_once: false,
            pending: HashMap::new(),
            rescan_count: 0,
            watch_error_count: 0,
            overflow_count: 0,
            last_sync_mode: None,
            last_sync_reason: None,
        }
    }
}

#[derive(Debug)]
pub struct WorkspaceChangeHub {
    project_key: String,
    state: Mutex<WorkspaceChangeState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeStatus {
    pub project_key: String,
    pub watcher_active: bool,
    pub health: ChangeJournalHealth,
    pub health_reason: String,
    pub generation: u64,
    pub next_seq: u64,
    pub compile_ack: u64,
    pub pending_count: usize,
    pub unity_pending_count: usize,
    pub rescan_count: u64,
    pub watch_error_count: u64,
    pub overflow_count: u64,
    pub last_sync_mode: Option<UnityAssetSyncMode>,
    pub last_sync_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnitySyncSnapshot {
    pub through_seq: u64,
    pub generation: u64,
    pub health: ChangeJournalHealth,
    pub mode: UnityAssetSyncMode,
    pub paths: Vec<String>,
    pub reason: String,
    path_stamps: HashMap<String, FileStamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified_ns: u128,
    content_hash: Option<[u8; 32]>,
}

impl WorkspaceChangeHub {
    fn new(project_key: String) -> Self {
        Self {
            project_key,
            state: Mutex::new(WorkspaceChangeState::default()),
        }
    }

    pub fn project_key(&self) -> &str {
        &self.project_key
    }

    pub fn watcher_started(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.watcher_count == 0 && state.watcher_started_once {
            mark_suspect_locked(&mut state, "watcher_restarted");
        }
        state.watcher_started_once = true;
        state.watcher_count = state.watcher_count.saturating_add(1);
    }

    pub fn watcher_stopped(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.watcher_count == 0 {
            return;
        }
        state.watcher_count -= 1;
        if state.watcher_count == 0 {
            mark_suspect_locked(&mut state, "watcher_stopped");
        }
    }

    pub fn mark_rescan_required(&self, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.rescan_count = state.rescan_count.saturating_add(1);
        mark_suspect_locked(&mut state, reason);
    }

    pub fn mark_watch_error(&self, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.watch_error_count = state.watch_error_count.saturating_add(1);
        mark_suspect_locked(&mut state, reason);
    }

    pub fn mark_structural_gap(&self, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        mark_suspect_locked(&mut state, reason);
    }

    pub fn record(
        &self,
        path: &str,
        kind: WorkspaceChangeKind,
        source: WorkspaceChangeSource,
    ) -> Option<u64> {
        self.observe(path, kind, source).map(|event| event.seq)
    }

    pub fn observe(
        &self,
        path: &str,
        kind: WorkspaceChangeKind,
        source: WorkspaceChangeSource,
    ) -> Option<WorkspaceChangeEvent> {
        let path = normalize_workspace_relative_path(path)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.next_seq = state.next_seq.saturating_add(1);
        let seq = state.next_seq;

        if is_package_control_path(&path) {
            mark_suspect_locked(&mut state, "package_structure_changed");
        }

        if is_unity_compile_input(&path) {
            if state.pending.len() >= MAX_PENDING_PATHS_PER_WORKSPACE
                && !state.pending.contains_key(&path)
            {
                state.overflow_count = state.overflow_count.saturating_add(1);
                mark_suspect_locked(&mut state, "journal_capacity_exceeded");
            } else {
                state.pending.insert(
                    path.clone(),
                    PendingWorkspaceChange {
                        seq,
                        kind,
                        _source: source,
                    },
                );
            }
        }

        Some(WorkspaceChangeEvent {
            seq,
            generation: state.generation,
            path,
            kind,
            source,
        })
    }

    pub fn unity_snapshot(
        &self,
        project_root: &Path,
        explicit_paths: &[String],
    ) -> UnitySyncSnapshot {
        // Clone only the small coalesced journal view while holding the
        // per-project lock. Filesystem probes and optional small-file hashes
        // happen after releasing it, so a large targeted batch cannot stall
        // the watcher's event receiver for this or any neighboring project.
        let (through_seq, mut generation, mut health, watcher_active, health_reason, pending) = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            (
                state.next_seq,
                state.generation,
                state.health,
                state.watcher_count > 0,
                state.health_reason.clone(),
                state
                    .pending
                    .iter()
                    .filter(|(_, change)| change.seq <= state.next_seq)
                    .map(|(path, change)| (path.clone(), change.clone()))
                    .collect::<Vec<_>>(),
            )
        };

        let mut paths = Vec::new();
        let mut requires_full = false;
        let mut full_reason = None;

        if !watcher_active {
            requires_full = true;
            full_reason = Some("watcher_inactive".to_string());
        } else if health != ChangeJournalHealth::Healthy {
            requires_full = true;
            full_reason = Some(health_reason);
        }

        for (path, change) in pending {
            if !is_unity_compile_input(&path) {
                continue;
            }
            if is_package_control_path(&path) {
                requires_full = true;
                full_reason = Some("package_structure_changed".to_string());
                continue;
            }
            if change.kind == WorkspaceChangeKind::Delete {
                requires_full = true;
                full_reason = Some("compile_input_deleted".to_string());
                continue;
            }
            paths.push(path);
        }

        for path in explicit_paths {
            let Some(path) = normalize_workspace_relative_path(path) else {
                requires_full = true;
                full_reason = Some("explicit_path_invalid".to_string());
                continue;
            };
            if !is_unity_compile_input(&path) {
                continue;
            }
            if is_package_control_path(&path) {
                requires_full = true;
                full_reason = Some("package_structure_changed".to_string());
                continue;
            }
            let absolute = project_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !absolute.is_file() {
                requires_full = true;
                full_reason = Some("explicit_compile_input_missing".to_string());
                continue;
            }
            paths.push(path);
        }

        paths.sort();
        paths.dedup();

        if !requires_full {
            for path in &paths {
                if file_stamp_for_relative_path(project_root, path).is_none() {
                    requires_full = true;
                    full_reason = Some("compile_input_missing_at_snapshot".to_string());
                    break;
                }
            }
        }

        let (mode, reason) = if requires_full {
            (
                UnityAssetSyncMode::Full,
                full_reason.unwrap_or_else(|| "journal_not_trusted".to_string()),
            )
        } else if paths.is_empty() {
            (
                UnityAssetSyncMode::None,
                "healthy_no_compile_changes".to_string(),
            )
        } else {
            (
                UnityAssetSyncMode::Targeted,
                "healthy_known_compile_changes".to_string(),
            )
        };

        let path_stamps = if mode == UnityAssetSyncMode::Targeted {
            paths
                .iter()
                .filter_map(|path| {
                    file_stamp_for_relative_path(project_root, path)
                        .map(|stamp| (path.clone(), stamp))
                })
                .collect()
        } else {
            HashMap::new()
        };

        let (mode, reason, through_seq) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            // A rescan/error/restart raced with the filesystem probes. A full
            // refresh based on the newest generation is the safe decision;
            // its acknowledgement still checks that no later gap occurred.
            let (mode, reason, through_seq) =
                if state.generation != generation || (state.watcher_count > 0) != watcher_active {
                    generation = state.generation;
                    health = state.health;
                    (
                        UnityAssetSyncMode::Full,
                        "journal_changed_during_snapshot".to_string(),
                        state.next_seq,
                    )
                } else {
                    (mode, reason, through_seq)
                };
            state.last_sync_mode = Some(mode);
            state.last_sync_reason = Some(reason.clone());
            (mode, reason, through_seq)
        };

        UnitySyncSnapshot {
            through_seq,
            generation,
            health,
            mode,
            paths,
            reason,
            path_stamps,
        }
    }

    pub fn acknowledge_unity_sync(&self, project_root: &Path, snapshot: &UnitySyncSnapshot) {
        let current_path_stamps = if snapshot.mode == UnityAssetSyncMode::Targeted {
            snapshot
                .path_stamps
                .keys()
                .filter_map(|path| {
                    file_stamp_for_relative_path(project_root, path)
                        .map(|stamp| (path.clone(), stamp))
                })
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.compile_ack = state.compile_ack.max(snapshot.through_seq);
        state.pending.retain(|path, change| {
            if change.seq <= snapshot.through_seq {
                return false;
            }
            // OS notifications for a Locus write can arrive after the compile
            // snapshot. Drop that delayed duplicate only when the file stamp
            // still matches the exact bytes imported by the targeted pass;
            // a concurrent edit changes the stamp and remains pending.
            if snapshot.mode == UnityAssetSyncMode::Targeted
                && change.kind == WorkspaceChangeKind::Upsert
                && snapshot.path_stamps.get(path).is_some_and(|before| {
                    current_path_stamps
                        .get(path)
                        .is_some_and(|after| stamps_prove_same_content(before, after))
                })
            {
                return false;
            }
            true
        });

        if snapshot.mode == UnityAssetSyncMode::Full
            && state.watcher_count > 0
            && state.generation == snapshot.generation
        {
            state.health = ChangeJournalHealth::Healthy;
            state.health_reason = "full_refresh_baseline".to_string();
        }
    }

    pub fn status(&self) -> WorkspaceChangeStatus {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        WorkspaceChangeStatus {
            project_key: self.project_key.clone(),
            watcher_active: state.watcher_count > 0,
            health: state.health,
            health_reason: state.health_reason.clone(),
            generation: state.generation,
            next_seq: state.next_seq,
            compile_ack: state.compile_ack,
            pending_count: state.pending.len(),
            unity_pending_count: state
                .pending
                .keys()
                .filter(|path| is_unity_compile_input(path))
                .count(),
            rescan_count: state.rescan_count,
            watch_error_count: state.watch_error_count,
            overflow_count: state.overflow_count,
            last_sync_mode: state.last_sync_mode,
            last_sync_reason: state.last_sync_reason.clone(),
        }
    }
}

fn mark_suspect_locked(state: &mut WorkspaceChangeState, reason: &str) {
    state.generation = state.generation.saturating_add(1);
    state.health = ChangeJournalHealth::Suspect;
    state.health_reason = reason.to_string();
}

fn registry() -> &'static RwLock<HashMap<String, Weak<WorkspaceChangeHub>>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, Weak<WorkspaceChangeHub>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn hub_for_workspace(project_root: &Path) -> Arc<WorkspaceChangeHub> {
    let key = workspace_key(project_root);
    if let Ok(registry) = registry().read() {
        if let Some(hub) = registry.get(&key).and_then(Weak::upgrade) {
            return hub;
        }
    }

    let mut registry = registry()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(hub) = registry.get(&key).and_then(Weak::upgrade) {
        return hub;
    }
    registry.retain(|_, hub| hub.strong_count() > 0);
    let hub = Arc::new(WorkspaceChangeHub::new(key.clone()));
    registry.insert(key, Arc::downgrade(&hub));
    hub
}

pub fn lookup_workspace_hub(project_root: &Path) -> Option<Arc<WorkspaceChangeHub>> {
    let key = workspace_key(project_root);
    registry()
        .read()
        .ok()
        .and_then(|registry| registry.get(&key).and_then(Weak::upgrade))
}

pub fn record_known_paths(project_root: &Path, paths: &[String], source: WorkspaceChangeSource) {
    if paths.is_empty() {
        return;
    }
    let hub = hub_for_workspace(project_root);
    for path in paths {
        let Some(normalized) = normalize_workspace_relative_path(path) else {
            hub.mark_structural_gap("known_path_invalid");
            continue;
        };
        if !is_unity_compile_input(&normalized) {
            continue;
        }
        let absolute = project_root.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR));
        let kind = if absolute.exists() {
            WorkspaceChangeKind::Upsert
        } else {
            WorkspaceChangeKind::Delete
        };
        hub.record(&normalized, kind, source);
    }
}

pub fn snapshot_unity_sync(project_root: &Path, explicit_paths: &[String]) -> UnitySyncSnapshot {
    hub_for_workspace(project_root).unity_snapshot(project_root, explicit_paths)
}

pub fn acknowledge_unity_sync(project_root: &Path, snapshot: &UnitySyncSnapshot) {
    if let Some(hub) = lookup_workspace_hub(project_root) {
        hub.acknowledge_unity_sync(project_root, snapshot);
    }
}

pub fn normalize_workspace_relative_path(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized.trim_start_matches("./").trim_matches('/');
    if normalized.is_empty()
        || normalized.contains("/../")
        || normalized.starts_with("../")
        || normalized.ends_with("/..")
        || (!normalized.starts_with("Assets/") && !normalized.starts_with("Packages/"))
    {
        return None;
    }
    Some(normalized.to_string())
}

pub fn is_unity_compile_input(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if is_package_control_path(&normalized) || normalized.ends_with(".meta") {
        return is_package_control_path(&normalized);
    }
    matches!(
        Path::new(&normalized)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("cs" | "asmdef" | "asmref" | "rsp" | "dll")
    )
}

fn is_package_control_path(path: &str) -> bool {
    path.eq_ignore_ascii_case("Packages/manifest.json")
        || path.eq_ignore_ascii_case("Packages/packages-lock.json")
}

fn file_stamp_for_relative_path(project_root: &Path, path: &str) -> Option<FileStamp> {
    let absolute = project_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let metadata = std::fs::metadata(absolute).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let content_hash = if metadata.len() <= MAX_DUPLICATE_FINGERPRINT_BYTES {
        std::fs::read(project_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .ok()
            .map(|content| *blake3::hash(&content).as_bytes())
    } else {
        None
    };
    Some(FileStamp {
        len: metadata.len(),
        modified_ns,
        content_hash,
    })
}

fn stamps_prove_same_content(before: &FileStamp, after: &FileStamp) -> bool {
    before.len == after.len
        && before.modified_ns == after.modified_ns
        && before
            .content_hash
            .zip(after.content_hash)
            .is_some_and(|(before, after)| before == after)
}

fn workspace_key(project_root: &Path) -> String {
    let path = dunce::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    let mut key = path.to_string_lossy().replace('\\', "/");
    while key.ends_with('/') && key.len() > 3 {
        key.pop();
    }
    #[cfg(target_os = "windows")]
    {
        key.make_ascii_lowercase();
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_hub(root: &Path) -> Arc<WorkspaceChangeHub> {
        let hub = hub_for_workspace(root);
        hub.watcher_started();
        let baseline = hub.unity_snapshot(root, &[]);
        assert_eq!(baseline.mode, UnityAssetSyncMode::Full);
        hub.acknowledge_unity_sync(root, &baseline);
        assert_eq!(hub.status().health, ChangeJournalHealth::Healthy);
        hub
    }

    #[test]
    fn coalesces_latest_change_per_path() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let hub = healthy_hub(temp.path());
        hub.record(
            "Assets/Test.cs",
            WorkspaceChangeKind::Delete,
            WorkspaceChangeSource::OsWatcher,
        );
        std::fs::create_dir_all(temp.path().join("Assets")).expect("assets dir");
        std::fs::write(temp.path().join("Assets/Test.cs"), "class Test {}").expect("script");
        hub.record(
            "Assets/Test.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::OsWatcher,
        );

        let snapshot = hub.unity_snapshot(temp.path(), &[]);
        assert_eq!(snapshot.mode, UnityAssetSyncMode::Targeted);
        assert_eq!(snapshot.paths, vec!["Assets/Test.cs"]);
        assert_eq!(hub.status().pending_count, 1);
    }

    #[test]
    fn shared_event_envelope_advances_sequence_without_polluting_unity_projection() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let hub = healthy_hub(temp.path());
        let event = hub
            .observe(
                "Assets/Scenes/Main.unity",
                WorkspaceChangeKind::Upsert,
                WorkspaceChangeSource::OsWatcher,
            )
            .expect("normalized event");

        assert_eq!(event.path, "Assets/Scenes/Main.unity");
        assert_eq!(event.seq, 1);
        assert_eq!(hub.status().pending_count, 0);
        assert_eq!(
            hub.unity_snapshot(temp.path(), &[]).mode,
            UnityAssetSyncMode::None
        );
    }

    #[test]
    fn ack_preserves_changes_observed_after_snapshot() {
        let temp = tempfile::tempdir().expect("temp workspace");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("assets dir");
        let hub = healthy_hub(temp.path());
        std::fs::write(temp.path().join("Assets/A.cs"), "class A {}").expect("script a");
        hub.record(
            "Assets/A.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::OsWatcher,
        );
        let snapshot = hub.unity_snapshot(temp.path(), &[]);
        std::fs::write(temp.path().join("Assets/B.cs"), "class B {}").expect("script b");
        hub.record(
            "Assets/B.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::OsWatcher,
        );
        hub.acknowledge_unity_sync(temp.path(), &snapshot);

        let next = hub.unity_snapshot(temp.path(), &[]);
        assert_eq!(next.mode, UnityAssetSyncMode::Targeted);
        assert_eq!(next.paths, vec!["Assets/B.cs"]);
    }

    #[test]
    fn ack_discards_delayed_duplicate_event_for_unchanged_imported_file() {
        let temp = tempfile::tempdir().expect("temp workspace");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("assets dir");
        let hub = healthy_hub(temp.path());
        std::fs::write(temp.path().join("Assets/A.cs"), "class A {}").expect("script a");
        hub.record(
            "Assets/A.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::LocusWrite,
        );
        let snapshot = hub.unity_snapshot(temp.path(), &[]);

        hub.record(
            "Assets/A.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::OsWatcher,
        );
        hub.acknowledge_unity_sync(temp.path(), &snapshot);

        assert_eq!(
            hub.unity_snapshot(temp.path(), &[]).mode,
            UnityAssetSyncMode::None
        );
    }

    #[test]
    fn ack_keeps_same_path_when_file_changes_during_compile() {
        let temp = tempfile::tempdir().expect("temp workspace");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("assets dir");
        let hub = healthy_hub(temp.path());
        let script = temp.path().join("Assets/A.cs");
        std::fs::write(&script, "class A {}").expect("script a");
        hub.record(
            "Assets/A.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::LocusWrite,
        );
        let snapshot = hub.unity_snapshot(temp.path(), &[]);

        std::fs::write(&script, "class A { int Changed; }").expect("change during compile");
        hub.record(
            "Assets/A.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::OsWatcher,
        );
        hub.acknowledge_unity_sync(temp.path(), &snapshot);

        let next = hub.unity_snapshot(temp.path(), &[]);
        assert_eq!(next.mode, UnityAssetSyncMode::Targeted);
        assert_eq!(next.paths, vec!["Assets/A.cs"]);
    }

    #[test]
    fn watcher_gap_requires_full_refresh_before_health_recovers() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let hub = healthy_hub(temp.path());
        hub.mark_rescan_required("notify_rescan");
        let snapshot = hub.unity_snapshot(temp.path(), &[]);
        assert_eq!(snapshot.mode, UnityAssetSyncMode::Full);
        assert_eq!(snapshot.reason, "notify_rescan");
        hub.acknowledge_unity_sync(temp.path(), &snapshot);
        assert_eq!(hub.status().health, ChangeJournalHealth::Healthy);
    }

    #[test]
    fn projects_keep_independent_health_and_sequences() {
        let left = tempfile::tempdir().expect("left workspace");
        let right = tempfile::tempdir().expect("right workspace");
        let left_hub = healthy_hub(left.path());
        let right_hub = healthy_hub(right.path());

        left_hub.mark_watch_error("left_only_error");
        right_hub.record(
            "Assets/Right.cs",
            WorkspaceChangeKind::Upsert,
            WorkspaceChangeSource::LocusWrite,
        );

        assert_eq!(left_hub.status().health, ChangeJournalHealth::Suspect);
        assert_eq!(right_hub.status().health, ChangeJournalHealth::Healthy);
        assert_ne!(left_hub.project_key(), right_hub.project_key());
        assert_eq!(right_hub.status().pending_count, 1);
    }

    #[test]
    fn deleted_compile_input_forces_full_refresh() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let hub = healthy_hub(temp.path());
        hub.record(
            "Assets/Removed.asmdef",
            WorkspaceChangeKind::Delete,
            WorkspaceChangeSource::OsWatcher,
        );
        let snapshot = hub.unity_snapshot(temp.path(), &[]);
        assert_eq!(snapshot.mode, UnityAssetSyncMode::Full);
        assert_eq!(snapshot.reason, "compile_input_deleted");
    }

    #[test]
    fn watcher_restart_preserves_hub_and_requires_new_baseline() {
        let temp = tempfile::tempdir().expect("temp workspace");
        let hub = healthy_hub(temp.path());
        let generation = hub.status().generation;
        hub.watcher_stopped();
        hub.watcher_started();

        let status = hub.status();
        assert!(status.generation > generation);
        assert_eq!(status.health, ChangeJournalHealth::Suspect);
        assert_eq!(
            hub.unity_snapshot(temp.path(), &[]).mode,
            UnityAssetSyncMode::Full
        );
    }

    #[test]
    fn journal_capacity_fails_closed_for_only_that_workspace() {
        let full = tempfile::tempdir().expect("full workspace");
        let neighbor = tempfile::tempdir().expect("neighbor workspace");
        let full_hub = healthy_hub(full.path());
        let neighbor_hub = healthy_hub(neighbor.path());

        for index in 0..=MAX_PENDING_PATHS_PER_WORKSPACE {
            full_hub.record(
                &format!("Assets/Generated{index}.cs"),
                WorkspaceChangeKind::Upsert,
                WorkspaceChangeSource::OsWatcher,
            );
        }

        let full_status = full_hub.status();
        assert_eq!(full_status.health, ChangeJournalHealth::Suspect);
        assert_eq!(full_status.overflow_count, 1);
        assert_eq!(full_status.pending_count, MAX_PENDING_PATHS_PER_WORKSPACE);
        assert_eq!(neighbor_hub.status().health, ChangeJournalHealth::Healthy);
    }
}
