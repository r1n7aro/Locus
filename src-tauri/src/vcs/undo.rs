//! Round-level undo backed by git snapshots.
//!
//! Snapshot reclamation policy:
//! - While an entry is live, its pre/post snapshots are pinned via
//!   `refs/locus/undo/<entry-id>/{pre,post}` so `git gc` keeps them alive.
//! - Pins are released when the entry is undone, its session is deleted, it
//!   falls past `MAX_UNDO_ENTRIES_PER_SESSION`, or it exceeds
//!   `UNDO_ENTRY_MAX_AGE_MS` (enforced when loading persisted stacks).
//! - Refs orphaned by crashes are swept once per workspace per app run, on the
//!   first recorded round in that workspace.
//! - Unpinned snapshot objects are reclaimed by the user's normal `git gc`;
//!   Locus never runs gc itself.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock};

use super::{Checkpoint, GitProvider, VcsProvider};

/// Live entries per session; older entries are pruned (and their refs
/// released) as new rounds are recorded.
const MAX_UNDO_ENTRIES_PER_SESSION: usize = 50;
/// Entries older than this are dropped when persisted stacks are loaded.
const UNDO_ENTRY_MAX_AGE_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const PERSIST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub status: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    pub id: String,
    pub session_id: String,
    pub assistant_message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// VCS checkpoint
    pub checkpoint: Checkpoint,
    /// Workspace the snapshots live in; undo refuses to restore elsewhere.
    #[serde(default)]
    pub working_dir: String,
    /// Snapshot of the worktree right after the round; the baseline for
    /// detecting external modifications made since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_state_id: Option<String>,
    pub changed_files: Vec<ChangedFile>,
    pub has_unity_execute: bool,
    pub consumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoConflict {
    pub session_id: String,
    pub assistant_message_id: String,
    pub checkpoint: Checkpoint,
    pub changed_files: Vec<ChangedFile>,
}

pub struct UndoRoundGuard {
    pub checkpoint: Checkpoint,
    _workspace_guard: OwnedRwLockReadGuard<()>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UndoPerformOptions {
    /// Skip the cross-session conflict check (and the dirty check).
    pub force: bool,
    /// Proceed even when files were modified after the round; those extra
    /// modifications are rolled back together with the round's changes.
    pub accept_dirty: bool,
}

#[derive(Debug)]
pub enum UndoPerformError {
    Conflict(Vec<UndoConflict>),
    /// Files in the restore set were modified after the agent round ended
    /// (by the user or another process); confirming rolls them back too.
    Dirty(Vec<ChangedFile>),
    Other(String),
}

#[derive(Debug, Clone)]
pub struct UndoPerformResult {
    pub entry: UndoEntry,
    pub restored_files: Vec<ChangedFile>,
}

pub struct UndoManager {
    provider: GitProvider,
    stacks: Mutex<HashMap<String, Vec<UndoEntry>>>,
    /// Per-workspace round/undo coordination: rounds hold the lock shared so
    /// they can run concurrently; undo takes it exclusively so it never
    /// interleaves with an in-flight round.
    workspace_guards: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    next_checkpoint_created_at: AtomicI64,
    persist_path: Option<PathBuf>,
    swept_workspaces: Mutex<HashSet<String>>,
}

fn normalize_path_key(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn collect_changed_file_keys(file: &ChangedFile, keys: &mut HashSet<String>) {
    keys.insert(normalize_path_key(&file.path));
    if let Some(old_path) = &file.old_path {
        keys.insert(normalize_path_key(old_path));
    }
}

fn changed_file_overlaps(target_keys: &HashSet<String>, file: &ChangedFile) -> bool {
    target_keys.contains(&normalize_path_key(&file.path))
        || file
            .old_path
            .as_ref()
            .map(|p| target_keys.contains(&normalize_path_key(p)))
            .unwrap_or(false)
}

fn is_internal_generated_changed_file(file: &ChangedFile) -> bool {
    crate::view::is_view_frontend_log_workspace_path(&file.path)
        || file
            .old_path
            .as_deref()
            .map(crate::view::is_view_frontend_log_workspace_path)
            .unwrap_or(false)
}

fn push_changed_file_if_new_target(
    seen: &mut HashSet<String>,
    files: &mut Vec<ChangedFile>,
    file: &ChangedFile,
) {
    let mut has_new_target = seen.insert(normalize_path_key(&file.path));
    if let Some(old_path) = &file.old_path {
        has_new_target |= seen.insert(normalize_path_key(old_path));
    }
    if has_new_target {
        files.push(file.clone());
    }
}

/// Parse one `--name-status` diff line ("M\tpath" / "R100\told\tnew").
fn parse_changed_file_line(line: &str) -> Option<ChangedFile> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 2 {
        return None;
    }
    let status = parts[0].chars().next()?.to_string();
    if parts.len() >= 3 {
        Some(ChangedFile {
            status,
            path: parts[2].trim().to_string(),
            old_path: Some(parts[1].trim().to_string()),
        })
    } else {
        Some(ChangedFile {
            status,
            path: parts[1].trim().to_string(),
            old_path: None,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedStacks {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sessions: HashMap<String, Vec<UndoEntry>>,
}

fn load_stacks(
    path: &Path,
    valid_session_ids: Option<&HashSet<String>>,
) -> HashMap<String, Vec<UndoEntry>> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(_) => return HashMap::new(),
    };
    let parsed: PersistedStacks = match serde_json::from_slice(&raw) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!(
                "[UndoManager] failed to parse persisted undo stacks '{}': {}",
                path.display(),
                e
            );
            return HashMap::new();
        }
    };
    if parsed.version > PERSIST_VERSION {
        eprintln!(
            "[UndoManager] persisted undo stacks use newer version {} (supported {}); loading best-effort",
            parsed.version, PERSIST_VERSION
        );
    }

    let now = now_ms();
    let mut sessions = parsed.sessions;
    sessions.retain(|session_id, stack| {
        if let Some(valid) = valid_session_ids {
            if !valid.contains(session_id) {
                return false;
            }
        }
        stack.retain(|entry| {
            !entry.consumed
                && now.saturating_sub(entry.checkpoint.created_at) <= UNDO_ENTRY_MAX_AGE_MS
        });
        !stack.is_empty()
    });
    sessions
}

impl UndoManager {
    pub fn new(provider: GitProvider) -> Self {
        Self::build(provider, None, None)
    }

    /// Manager with a persistent stack file: loads surviving entries
    /// (dropping unknown sessions, consumed entries, and entries past the age
    /// cap) and saves after every stack mutation.
    pub fn with_persistence(
        provider: GitProvider,
        persist_path: PathBuf,
        valid_session_ids: Option<HashSet<String>>,
    ) -> Self {
        Self::build(provider, Some(persist_path), valid_session_ids)
    }

    fn build(
        provider: GitProvider,
        persist_path: Option<PathBuf>,
        valid_session_ids: Option<HashSet<String>>,
    ) -> Self {
        let stacks = persist_path
            .as_deref()
            .map(|path| load_stacks(path, valid_session_ids.as_ref()))
            .unwrap_or_default();
        // Seed the monotonic clock past every persisted checkpoint so new
        // entries always order after restored ones.
        let max_created_at = stacks
            .values()
            .flatten()
            .map(|entry| entry.checkpoint.created_at)
            .max()
            .unwrap_or(0);
        UndoManager {
            provider,
            stacks: Mutex::new(stacks),
            workspace_guards: Mutex::new(HashMap::new()),
            next_checkpoint_created_at: AtomicI64::new(max_created_at),
            persist_path,
            swept_workspaces: Mutex::new(HashSet::new()),
        }
    }

    fn save_locked(&self, stacks: &HashMap<String, Vec<UndoEntry>>) {
        let Some(path) = &self.persist_path else {
            return;
        };
        let payload = serde_json::json!({
            "version": PERSIST_VERSION,
            "sessions": stacks,
        });
        let bytes = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("[UndoManager] failed to serialize undo stacks: {}", e);
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        let result = std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            eprintln!(
                "[UndoManager] failed to persist undo stacks to '{}': {}",
                path.display(),
                e
            );
        }
    }

    async fn workspace_lock(&self, working_dir: &str) -> Arc<RwLock<()>> {
        let key = normalize_path_key(working_dir);
        let mut guards = self.workspace_guards.lock().await;
        guards
            .entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    fn next_checkpoint_created_at(&self) -> i64 {
        let now = now_ms();
        loop {
            let last = self.next_checkpoint_created_at.load(Ordering::Relaxed);
            let next = now.max(last.saturating_add(1));
            match self.next_checkpoint_created_at.compare_exchange(
                last,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return next,
                Err(_) => continue,
            }
        }
    }

    fn collect_conflicts_from_stacks(
        stacks: &HashMap<String, Vec<UndoEntry>>,
        session_id: &str,
        assistant_message_id: &str,
    ) -> Result<Vec<UndoConflict>, String> {
        let stack = stacks
            .get(session_id)
            .ok_or_else(|| "no undo history for this session".to_string())?;

        let target_idx = stack
            .iter()
            .position(|e| !e.consumed && e.assistant_message_id == assistant_message_id)
            .ok_or_else(|| "undo entry not found for this message".to_string())?;

        let target_checkpoint_at = stack[target_idx].checkpoint.created_at;
        let target_dir_key = normalize_path_key(&stack[target_idx].working_dir);
        let mut target_keys = HashSet::new();
        for entry in &stack[target_idx..] {
            if entry.consumed {
                continue;
            }
            for file in &entry.changed_files {
                collect_changed_file_keys(file, &mut target_keys);
            }
        }

        if target_keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut conflicts = Vec::new();
        for (other_session_id, other_stack) in stacks.iter() {
            if other_session_id == session_id {
                continue;
            }
            for entry in other_stack {
                if entry.consumed || entry.checkpoint.created_at <= target_checkpoint_at {
                    continue;
                }
                // Path keys are workspace-relative; entries from a different
                // workspace can collide spuriously (e.g. Assets/... in two
                // Unity projects), so only compare within the same one.
                if !entry.working_dir.is_empty()
                    && !target_dir_key.is_empty()
                    && normalize_path_key(&entry.working_dir) != target_dir_key
                {
                    continue;
                }
                let overlapping: Vec<ChangedFile> = entry
                    .changed_files
                    .iter()
                    .filter(|file| changed_file_overlaps(&target_keys, file))
                    .cloned()
                    .collect();
                if overlapping.is_empty() {
                    continue;
                }
                conflicts.push(UndoConflict {
                    session_id: other_session_id.clone(),
                    assistant_message_id: entry.assistant_message_id.clone(),
                    checkpoint: entry.checkpoint.clone(),
                    changed_files: overlapping,
                });
            }
        }

        conflicts.sort_by(|a, b| {
            a.checkpoint
                .created_at
                .cmp(&b.checkpoint.created_at)
                .then_with(|| a.session_id.cmp(&b.session_id))
                .then_with(|| a.assistant_message_id.cmp(&b.assistant_message_id))
        });
        Ok(conflicts)
    }

    pub async fn before_round(
        &self,
        working_dir: &str,
        label: &str,
    ) -> Result<Option<UndoRoundGuard>, String> {
        if !self.provider.is_available(working_dir).await {
            return Ok(None);
        }
        // Shared guard: rounds in the same workspace run concurrently; only
        // undo (exclusive) waits for them and blocks new ones.
        let workspace_guard = self.workspace_lock(working_dir).await.read_owned().await;
        let checkpoint = self.provider.checkpoint(working_dir, label).await?;
        Ok(checkpoint.map(|mut checkpoint| {
            checkpoint.created_at = self.next_checkpoint_created_at();
            UndoRoundGuard {
                checkpoint,
                _workspace_guard: workspace_guard,
            }
        }))
    }

    pub async fn after_round(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        run_id: Option<&str>,
        round: UndoRoundGuard,
        has_unity_execute: bool,
        working_dir: &str,
    ) -> Result<bool, String> {
        let checkpoint = round.checkpoint;
        let _workspace_guard = round._workspace_guard;
        let round_diff = GitProvider::diff_files(working_dir, &checkpoint.id)
            .await
            .map_err(|e| {
                eprintln!(
                    "[UndoManager] failed to capture undo diff for session {} message {} checkpoint {}: {}",
                    session_id, assistant_message_id, checkpoint.id, e
                );
                format!("failed to capture undo diff: {}", e)
            })?;
        let changed_files: Vec<ChangedFile> = round_diff
            .lines
            .iter()
            .filter_map(|line| {
                let file = parse_changed_file_line(line)?;
                if is_internal_generated_changed_file(&file) {
                    None
                } else {
                    Some(file)
                }
            })
            .collect();

        if changed_files.is_empty() {
            eprintln!(
                "[UndoManager] skipping undo entry for session {} message {} because no file changes were detected",
                session_id, assistant_message_id
            );
            return Ok(false);
        }

        let entry = UndoEntry {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            assistant_message_id: assistant_message_id.to_string(),
            run_id: run_id.map(str::to_string),
            checkpoint,
            working_dir: working_dir.to_string(),
            after_state_id: Some(round_diff.after_state_id.clone()),
            changed_files,
            has_unity_execute,
            consumed: false,
        };
        let entry_id = entry.id.clone();
        let pre_state_id = entry.checkpoint.id.clone();
        let changed_summary = entry
            .changed_files
            .iter()
            .map(|file| match &file.old_path {
                Some(old_path) => format!("{}:{}->{}", file.status, old_path, file.path),
                None => format!("{}:{}", file.status, file.path),
            })
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "[UndoManager] recorded undo entry for session {} message {} run {:?}: {} file(s) [{}]",
            session_id,
            assistant_message_id,
            entry.run_id,
            entry.changed_files.len(),
            changed_summary
        );

        let pruned = {
            let mut stacks = self.stacks.lock().await;
            let pruned = {
                let stack = stacks.entry(session_id.to_string()).or_default();
                stack.push(entry);
                let excess = stack.len().saturating_sub(MAX_UNDO_ENTRIES_PER_SESSION);
                stack.drain(..excess).collect::<Vec<_>>()
            };
            self.save_locked(&stacks);
            pruned
        };

        // Pin only after the entry is in the stack: the orphan sweep lists
        // refs before reading the stack, so this order never sweeps a fresh
        // pin.
        if let Err(e) = GitProvider::pin_undo_refs(
            working_dir,
            &entry_id,
            &pre_state_id,
            Some(&round_diff.after_state_id),
        )
        .await
        {
            eprintln!(
                "[UndoManager] failed to pin undo refs for entry {}: {}",
                entry_id, e
            );
        }
        for stale in &pruned {
            let dir = if stale.working_dir.is_empty() {
                working_dir
            } else {
                stale.working_dir.as_str()
            };
            GitProvider::release_undo_refs(dir, &stale.id).await;
        }
        self.sweep_orphan_refs_once(working_dir).await;
        Ok(true)
    }

    /// Release pin refs that no live entry references (crash leftovers).
    /// Runs at most once per workspace per app run.
    async fn sweep_orphan_refs_once(&self, working_dir: &str) {
        let dir_key = normalize_path_key(working_dir);
        {
            let mut swept = self.swept_workspaces.lock().await;
            if !swept.insert(dir_key.clone()) {
                return;
            }
        }
        // List refs before snapshotting live ids (see pin ordering above).
        let ref_ids = match GitProvider::list_undo_ref_entry_ids(working_dir).await {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!(
                    "[UndoManager] failed to list undo refs for sweep in '{}': {}",
                    working_dir, e
                );
                return;
            }
        };
        if ref_ids.is_empty() {
            return;
        }
        let live: HashSet<String> = {
            let stacks = self.stacks.lock().await;
            stacks
                .values()
                .flatten()
                .filter(|entry| normalize_path_key(&entry.working_dir) == dir_key)
                .map(|entry| entry.id.clone())
                .collect()
        };
        let mut removed = 0usize;
        for id in ref_ids.difference(&live) {
            GitProvider::release_undo_refs(working_dir, id).await;
            removed += 1;
        }
        if removed > 0 {
            eprintln!(
                "[UndoManager] released {} orphaned undo ref entr(y/ies) in '{}'",
                removed, working_dir
            );
        }
    }

    /// Files in the restore set whose current content differs from the
    /// after-state of the last round that touched them — i.e. modifications
    /// made outside the recorded rounds that an undo would roll back too.
    pub async fn check_dirty(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        working_dir: &str,
    ) -> Result<Vec<ChangedFile>, String> {
        let workspace_lock = self.workspace_lock(working_dir).await;
        let _guard = workspace_lock.read_owned().await;
        self.compute_dirty_files(session_id, assistant_message_id, working_dir)
            .await
    }

    async fn compute_dirty_files(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        working_dir: &str,
    ) -> Result<Vec<ChangedFile>, String> {
        // Last-writer-wins ownership: the expected on-disk content of every
        // restore-set path is the after-state of the LAST entry that touched
        // it; any difference is an external modification.
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        {
            let stacks = self.stacks.lock().await;
            let stack = stacks
                .get(session_id)
                .ok_or_else(|| "no undo history for this session".to_string())?;
            let target_idx = stack
                .iter()
                .position(|e| !e.consumed && e.assistant_message_id == assistant_message_id)
                .ok_or_else(|| "undo entry not found for this message".to_string())?;

            let mut owner: HashMap<String, (Option<String>, String)> = HashMap::new();
            for entry in &stack[target_idx..] {
                if entry.consumed {
                    continue;
                }
                for file in &entry.changed_files {
                    for path in std::iter::once(file.path.as_str()).chain(file.old_path.as_deref())
                    {
                        owner.insert(
                            normalize_path_key(path),
                            (entry.after_state_id.clone(), path.to_string()),
                        );
                    }
                }
            }
            for (after_state_id, path) in owner.into_values() {
                // Entries recorded before after-state capture existed cannot
                // be verified; skip them rather than alarm spuriously.
                let Some(after_state_id) = after_state_id else {
                    continue;
                };
                groups.entry(after_state_id).or_default().push(path);
            }
        }
        if groups.is_empty() {
            return Ok(Vec::new());
        }

        let current_ref = GitProvider::worktree_state_ref(working_dir, "locus dirty check").await?;

        let mut seen = HashSet::new();
        let mut dirty = Vec::new();
        for (after_state_id, paths) in groups {
            if after_state_id == current_ref {
                continue;
            }
            let lines =
                GitProvider::diff_paths_between(working_dir, &after_state_id, &current_ref, &paths)
                    .await?;
            for line in lines {
                let Some(file) = parse_changed_file_line(&line) else {
                    continue;
                };
                if is_internal_generated_changed_file(&file) {
                    continue;
                }
                push_changed_file_if_new_target(&mut seen, &mut dirty, &file);
            }
        }
        dirty.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(dirty)
    }

    pub async fn perform_undo_checked(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        working_dir: &str,
        options: UndoPerformOptions,
    ) -> Result<UndoPerformResult, UndoPerformError> {
        // Exclusive guard: waits for in-flight rounds and blocks new ones for
        // the duration of the restore.
        let workspace_lock = self.workspace_lock(working_dir).await;
        let _workspace_guard = workspace_lock.write_owned().await;

        {
            let stacks = self.stacks.lock().await;
            if !options.force {
                let conflicts =
                    Self::collect_conflicts_from_stacks(&stacks, session_id, assistant_message_id)
                        .map_err(UndoPerformError::Other)?;
                if !conflicts.is_empty() {
                    return Err(UndoPerformError::Conflict(conflicts));
                }
            }
            let stack = stacks.get(session_id).ok_or_else(|| {
                UndoPerformError::Other("no undo history for this session".to_string())
            })?;
            let target = stack
                .iter()
                .find(|e| !e.consumed && e.assistant_message_id == assistant_message_id)
                .ok_or_else(|| {
                    UndoPerformError::Other("undo entry not found for this message".to_string())
                })?;
            if !target.working_dir.is_empty()
                && normalize_path_key(&target.working_dir) != normalize_path_key(working_dir)
            {
                return Err(UndoPerformError::Other(format!(
                    "undo entry belongs to a different workspace: {}",
                    target.working_dir
                )));
            }
        }

        if !options.force && !options.accept_dirty {
            let dirty = self
                .compute_dirty_files(session_id, assistant_message_id, working_dir)
                .await
                .map_err(UndoPerformError::Other)?;
            if !dirty.is_empty() {
                return Err(UndoPerformError::Dirty(dirty));
            }
        }

        let (checkpoint_id, all_changed, need_reload, result, consumed_ids) = {
            let mut stacks = self.stacks.lock().await;

            let stack = stacks.get_mut(session_id).ok_or_else(|| {
                UndoPerformError::Other("no undo history for this session".to_string())
            })?;

            let target_idx = stack
                .iter()
                .position(|e| !e.consumed && e.assistant_message_id == assistant_message_id)
                .ok_or_else(|| {
                    UndoPerformError::Other("undo entry not found for this message".to_string())
                })?;

            let checkpoint_id = stack[target_idx].checkpoint.id.clone();

            let mut seen = HashSet::new();
            let mut all_changed = Vec::new();
            let mut need_reload = false;
            let mut consumed_ids = Vec::new();
            for entry in stack[target_idx..].iter_mut() {
                if !entry.consumed {
                    entry.consumed = true;
                    consumed_ids.push(entry.id.clone());
                    if entry.has_unity_execute {
                        need_reload = true;
                    }
                    for f in &entry.changed_files {
                        push_changed_file_if_new_target(&mut seen, &mut all_changed, f);
                    }
                }
            }

            let result = stack[target_idx].clone();
            (
                checkpoint_id,
                all_changed,
                need_reload,
                result,
                consumed_ids,
            )
        };

        if let Err(e) = GitProvider::restore_files(
            working_dir,
            &checkpoint_id,
            result.checkpoint.index_tree_id.as_deref(),
            &all_changed,
        )
        .await
        {
            let mut stacks = self.stacks.lock().await;
            if let Some(stack) = stacks.get_mut(session_id) {
                for entry in stack.iter_mut() {
                    if consumed_ids.iter().any(|id| id == &entry.id) {
                        entry.consumed = false;
                    }
                }
            }
            return Err(UndoPerformError::Other(e));
        }

        // Consumed entries can never be targeted again: drop them and release
        // their snapshot pins so git can eventually reclaim the objects.
        let released: Vec<UndoEntry> = {
            let mut stacks = self.stacks.lock().await;
            let mut released = Vec::new();
            if let Some(stack) = stacks.get_mut(session_id) {
                stack.retain_mut(|entry| {
                    if entry.consumed {
                        released.push(entry.clone());
                        false
                    } else {
                        true
                    }
                });
                if stack.is_empty() {
                    stacks.remove(session_id);
                }
            }
            if !released.is_empty() {
                self.save_locked(&stacks);
            }
            released
        };
        for entry in &released {
            let dir = if entry.working_dir.is_empty() {
                working_dir
            } else {
                entry.working_dir.as_str()
            };
            GitProvider::release_undo_refs(dir, &entry.id).await;
        }

        if need_reload {
            match crate::unity_bridge::send_message(working_dir, "reload_open_scenes", "").await {
                Ok(resp) if !resp.ok => {
                    eprintln!("[UndoManager] Unity scene reload error: {:?}", resp.error);
                }
                Err(e) => {
                    eprintln!("[UndoManager] failed to reload Unity scenes: {}", e);
                }
                _ => {
                    eprintln!("[UndoManager] Unity scenes reloaded successfully");
                }
            }
        }

        Ok(UndoPerformResult {
            entry: result,
            restored_files: all_changed,
        })
    }

    pub async fn check_conflicts(
        &self,
        session_id: &str,
        assistant_message_id: &str,
    ) -> Result<Vec<UndoConflict>, String> {
        let stacks = self.stacks.lock().await;
        Self::collect_conflicts_from_stacks(&stacks, session_id, assistant_message_id)
    }

    /// Find an undo entry by session + message ID (for diff checkpoint lookups).
    pub async fn find_entry(
        &self,
        session_id: &str,
        assistant_message_id: &str,
    ) -> Option<UndoEntry> {
        let stacks = self.stacks.lock().await;
        stacks.get(session_id).and_then(|stack| {
            stack
                .iter()
                .find(|e| e.assistant_message_id == assistant_message_id)
                .cloned()
        })
    }

    pub async fn list_entries(&self, session_id: &str) -> Vec<UndoEntry> {
        let stacks = self.stacks.lock().await;
        stacks
            .get(session_id)
            .map(|stack| stack.iter().filter(|e| !e.consumed).cloned().collect())
            .unwrap_or_default()
    }

    pub async fn preview(
        &self,
        session_id: &str,
        assistant_message_id: &str,
    ) -> Result<Vec<ChangedFile>, String> {
        let stacks = self.stacks.lock().await;
        let stack = stacks
            .get(session_id)
            .ok_or_else(|| "no undo history for this session".to_string())?;

        let target_idx = stack
            .iter()
            .position(|e| !e.consumed && e.assistant_message_id == assistant_message_id)
            .ok_or_else(|| "undo entry not found for this message".to_string())?;

        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for entry in &stack[target_idx..] {
            if entry.consumed {
                continue;
            }
            for f in &entry.changed_files {
                push_changed_file_if_new_target(&mut seen, &mut files, f);
            }
        }

        Ok(files)
    }

    pub async fn on_session_delete(&self, session_id: &str) {
        let removed = {
            let mut stacks = self.stacks.lock().await;
            let removed = stacks.remove(session_id);
            if removed.is_some() {
                self.save_locked(&stacks);
            }
            removed
        };
        if let Some(stack) = removed {
            for entry in &stack {
                if entry.working_dir.is_empty() {
                    continue;
                }
                GitProvider::release_undo_refs(&entry.working_dir, &entry.id).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{load_stacks, UndoManager, UndoPerformError, UndoPerformOptions};
    use crate::process_util::command;
    use crate::vcs::GitProvider;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    fn git_available() -> bool {
        crate::process_util::resolve_git().is_some()
    }

    fn temp_repo_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("locus-undo-{}-{}", name, uuid::Uuid::new_v4()))
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::write(path, content).expect("write file");
    }

    fn setup_repo(name: &str) -> PathBuf {
        let repo = temp_repo_dir(name);
        std::fs::create_dir_all(&repo).expect("create temp repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.name", "test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        write_file(&repo.join("tracked.txt"), "base\n");
        git(&repo, &["add", "tracked.txt"]);
        git(&repo, &["commit", "-m", "init"]);
        repo
    }

    fn force_options() -> UndoPerformOptions {
        UndoPerformOptions {
            force: true,
            accept_dirty: true,
        }
    }

    fn checked_options() -> UndoPerformOptions {
        UndoPerformOptions::default()
    }

    #[tokio::test]
    async fn checkpoint_created_at_is_strictly_monotonic() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("checkpoint-order");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round1 = manager
            .before_round(&repo_str, "round1")
            .await
            .expect("round1 before_round")
            .expect("round1 checkpoint");
        let created1 = round1.checkpoint.created_at;
        assert!(!manager
            .after_round("s1", "m1", Some("run-1"), round1, false, &repo_str)
            .await
            .expect("round1 after_round"));

        let round2 = manager
            .before_round(&repo_str, "round2")
            .await
            .expect("round2 before_round")
            .expect("round2 checkpoint");
        let created2 = round2.checkpoint.created_at;
        assert!(!manager
            .after_round("s2", "m2", Some("run-2"), round2, false, &repo_str)
            .await
            .expect("round2 after_round"));

        assert!(created2 > created1);

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn rounds_in_same_workspace_run_concurrently() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("concurrent-rounds");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round_a = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("round a before_round")
            .expect("round a checkpoint");

        // A second round must start while the first is still in flight.
        let round_b = tokio::time::timeout(
            Duration::from_secs(10),
            manager.before_round(&repo_str, "round-b"),
        )
        .await
        .expect("round b must not block on round a")
        .expect("round b before_round")
        .expect("round b checkpoint");

        write_file(&repo.join("tracked.txt"), "base\nagent-a\n");
        assert!(manager
            .after_round("s-a", "m-a", Some("run-a"), round_a, false, &repo_str)
            .await
            .expect("round a after_round"));
        assert!(manager
            .after_round("s-b", "m-b", Some("run-b"), round_b, false, &repo_str)
            .await
            .expect("round b after_round"));

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn after_round_records_delete_when_workspace_returns_to_clean() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-clean-delete");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        write_file(&repo.join("test1.cs"), "class T {}\n");

        let round = manager
            .before_round(&repo_str, "round-delete")
            .await
            .expect("before_round")
            .expect("checkpoint");
        std::fs::remove_file(repo.join("test1.cs")).expect("delete file");

        let recorded = manager
            .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
            .await
            .expect("after_round should succeed");
        assert!(recorded);

        let entries = manager.list_entries("session-a").await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].assistant_message_id, "msg-a");
        assert_eq!(entries[0].changed_files.len(), 1);
        assert_eq!(entries[0].changed_files[0].status, "D");
        assert_eq!(entries[0].changed_files[0].path, "test1.cs");
        assert_eq!(entries[0].changed_files[0].old_path, None);
        assert!(entries[0].after_state_id.is_some());
        assert_eq!(entries[0].working_dir, repo_str);

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn after_round_ignores_view_frontend_log_changes() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-ignore-view-log");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round = manager
            .before_round(&repo_str, "round-view-log")
            .await
            .expect("before_round")
            .expect("checkpoint");

        let log_path =
            repo.join("Locus/View/ProjectName/material-inspector/.locus/logs/frontend.log");
        std::fs::create_dir_all(log_path.parent().expect("log parent")).expect("create log parent");
        write_file(&log_path, "{\"level\":\"warn\"}\n");

        let recorded = manager
            .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
            .await
            .expect("after_round should succeed");
        assert!(!recorded);
        assert!(manager.list_entries("session-a").await.is_empty());

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn after_round_propagates_diff_capture_errors() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-diff-error");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round = manager
            .before_round(&repo_str, "round-error")
            .await
            .expect("before_round")
            .expect("checkpoint");

        std::fs::remove_dir_all(&repo).expect("remove repo");

        let err = manager
            .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
            .await
            .expect_err("after_round should report diff capture failure");
        assert!(err.contains("failed to capture undo diff"));
    }

    #[tokio::test]
    async fn undo_removes_later_deleted_file_absent_from_target_checkpoint() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-later-delete-absent-from-target");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round_a = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("round a before_round")
            .expect("round a checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent-a\n");
        assert!(manager
            .after_round(
                "session-a",
                "msg-a",
                Some("run-a"),
                round_a,
                false,
                &repo_str,
            )
            .await
            .expect("round a after_round"));

        write_file(&repo.join("later.txt"), "external file\n");
        let round_b = manager
            .before_round(&repo_str, "round-b")
            .await
            .expect("round b before_round")
            .expect("round b checkpoint");
        std::fs::remove_file(repo.join("later.txt")).expect("delete later file");
        assert!(manager
            .after_round(
                "session-a",
                "msg-b",
                Some("run-b"),
                round_b,
                false,
                &repo_str,
            )
            .await
            .expect("round b after_round"));

        manager
            .perform_undo_checked("session-a", "msg-a", &repo_str, force_options())
            .await
            .expect("undo should not fail on a later D path absent from target checkpoint");

        assert_eq!(
            std::fs::read_to_string(repo.join("tracked.txt"))
                .expect("tracked after undo")
                .replace("\r\n", "\n"),
            "base\n",
        );
        assert!(
            !repo.join("later.txt").exists(),
            "later file should stay absent because the target checkpoint did not contain it",
        );
        assert_eq!(git(&repo, &["status", "--short", "--", "later.txt"]), "");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn undo_waits_for_inflight_round_and_reports_conflict() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("atomic-undo");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = Arc::new(UndoManager::new(GitProvider));

        let round_a = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("round a before_round")
            .expect("round a checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent-a\n");
        assert!(manager
            .after_round(
                "session-a",
                "msg-a",
                Some("run-a"),
                round_a,
                false,
                &repo_str
            )
            .await
            .expect("round a after_round"));

        let round_b = manager
            .before_round(&repo_str, "round-b")
            .await
            .expect("round b before_round")
            .expect("round b checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent-a\nagent-b\n");

        let manager_for_undo = manager.clone();
        let repo_for_undo = repo_str.clone();
        let undo_task = tokio::spawn(async move {
            manager_for_undo
                .perform_undo_checked("session-a", "msg-a", &repo_for_undo, checked_options())
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !undo_task.is_finished(),
            "undo should wait for the in-flight round to finish"
        );

        assert!(manager
            .after_round(
                "session-b",
                "msg-b",
                Some("run-b"),
                round_b,
                false,
                &repo_str
            )
            .await
            .expect("round b after_round"));

        let result = undo_task.await.expect("undo task join");
        match result {
            Err(UndoPerformError::Conflict(conflicts)) => {
                assert_eq!(conflicts.len(), 1);
                assert_eq!(conflicts[0].session_id, "session-b");
                assert_eq!(conflicts[0].assistant_message_id, "msg-b");
            }
            Err(UndoPerformError::Dirty(_)) => panic!("expected conflict, got dirty"),
            Err(UndoPerformError::Other(msg)) => {
                panic!("expected conflict, got error: {}", msg);
            }
            Ok(_) => panic!("expected conflict, got successful undo"),
        }

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn undo_reports_dirty_when_file_changed_after_round() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-dirty-detect");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("before_round")
            .expect("checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent\n");
        assert!(manager
            .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
            .await
            .expect("after_round"));

        // External edit after the round ended.
        write_file(&repo.join("tracked.txt"), "base\nagent\nuser-extra\n");

        let dirty = manager
            .check_dirty("session-a", "msg-a", &repo_str)
            .await
            .expect("check_dirty");
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].path, "tracked.txt");

        let err = manager
            .perform_undo_checked("session-a", "msg-a", &repo_str, checked_options())
            .await
            .expect_err("undo should report dirty files");
        match err {
            UndoPerformError::Dirty(files) => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "tracked.txt");
            }
            other => panic!("expected dirty error, got {:?}", other),
        }

        manager
            .perform_undo_checked(
                "session-a",
                "msg-a",
                &repo_str,
                UndoPerformOptions {
                    force: false,
                    accept_dirty: true,
                },
            )
            .await
            .expect("undo with accept_dirty should proceed");
        assert_eq!(
            std::fs::read_to_string(repo.join("tracked.txt"))
                .expect("tracked after undo")
                .replace("\r\n", "\n"),
            "base\n",
        );

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn dirty_check_compares_against_last_round_that_touched_path() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-dirty-last-writer");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round_a = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("round a before_round")
            .expect("round a checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent-a\n");
        assert!(manager
            .after_round(
                "session-a",
                "msg-a",
                Some("run-a"),
                round_a,
                false,
                &repo_str
            )
            .await
            .expect("round a after_round"));

        // User edit between rounds; round B touches a different file, so its
        // after-state silently absorbs the user edit — ownership must keep
        // comparing tracked.txt against round A's after-state.
        write_file(&repo.join("tracked.txt"), "base\nagent-a\nuser-extra\n");

        let round_b = manager
            .before_round(&repo_str, "round-b")
            .await
            .expect("round b before_round")
            .expect("round b checkpoint");
        write_file(&repo.join("other.txt"), "agent-b\n");
        assert!(manager
            .after_round(
                "session-a",
                "msg-b",
                Some("run-b"),
                round_b,
                false,
                &repo_str
            )
            .await
            .expect("round b after_round"));

        let dirty = manager
            .check_dirty("session-a", "msg-a", &repo_str)
            .await
            .expect("check_dirty");
        assert_eq!(
            dirty.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["tracked.txt"],
        );

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn undo_refs_are_pinned_and_released() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-ref-lifecycle");
        let repo_str = repo.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round = manager
            .before_round(&repo_str, "round-a")
            .await
            .expect("before_round")
            .expect("checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nagent\n");
        assert!(manager
            .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
            .await
            .expect("after_round"));

        let refs = GitProvider::list_undo_ref_entry_ids(&repo_str)
            .await
            .expect("list refs");
        assert_eq!(refs.len(), 1, "expected one pinned entry, got {:?}", refs);

        manager
            .perform_undo_checked("session-a", "msg-a", &repo_str, checked_options())
            .await
            .expect("undo should succeed");

        let refs = GitProvider::list_undo_ref_entry_ids(&repo_str)
            .await
            .expect("list refs after undo");
        assert!(
            refs.is_empty(),
            "refs should be released after undo, got {:?}",
            refs
        );
        assert!(manager.list_entries("session-a").await.is_empty());

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn persisted_stacks_survive_manager_restart() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("undo-persistence");
        let repo_str = repo.to_string_lossy().to_string();
        let persist_path = repo.join("undo_stacks.json");

        {
            let manager = UndoManager::with_persistence(GitProvider, persist_path.clone(), None);
            let round = manager
                .before_round(&repo_str, "round-a")
                .await
                .expect("before_round")
                .expect("checkpoint");
            write_file(&repo.join("tracked.txt"), "base\nagent\n");
            assert!(manager
                .after_round("session-a", "msg-a", Some("run-a"), round, false, &repo_str)
                .await
                .expect("after_round"));
        }

        let manager = UndoManager::with_persistence(GitProvider, persist_path.clone(), None);
        let entries = manager.list_entries("session-a").await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].assistant_message_id, "msg-a");

        manager
            .perform_undo_checked("session-a", "msg-a", &repo_str, checked_options())
            .await
            .expect("undo should succeed after restart");
        assert_eq!(
            std::fs::read_to_string(repo.join("tracked.txt"))
                .expect("tracked after undo")
                .replace("\r\n", "\n"),
            "base\n",
        );

        let manager = UndoManager::with_persistence(GitProvider, persist_path, None);
        assert!(manager.list_entries("session-a").await.is_empty());

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[test]
    fn load_stacks_drops_stale_and_unknown_sessions() {
        let dir = std::env::temp_dir().join(format!("locus-undo-load-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("undo_stacks.json");

        let now = super::now_ms();
        let stale = now - super::UNDO_ENTRY_MAX_AGE_MS - 1;
        let entry = |created_at: i64, consumed: bool| {
            serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "sessionId": "ignored",
                "assistantMessageId": "msg",
                "checkpoint": {
                    "id": "deadbeef",
                    "label": "round",
                    "createdAt": created_at,
                },
                "workingDir": "C:/repo",
                "changedFiles": [{"status": "M", "path": "a.txt"}],
                "hasUnityExecute": false,
                "consumed": consumed,
            })
        };
        let payload = serde_json::json!({
            "version": 1,
            "sessions": {
                "live": [entry(now, false), entry(stale, false), entry(now, true)],
                "gone": [entry(now, false)],
            }
        });
        std::fs::write(&path, serde_json::to_vec(&payload).expect("serialize"))
            .expect("write persisted stacks");

        let valid: std::collections::HashSet<String> =
            std::iter::once("live".to_string()).collect();
        let stacks = load_stacks(&path, Some(&valid));
        assert_eq!(stacks.len(), 1);
        let live = stacks.get("live").expect("live session retained");
        assert_eq!(
            live.len(),
            1,
            "stale and consumed entries should be dropped"
        );
        assert_eq!(live[0].checkpoint.created_at, now);

        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn perform_rejects_entry_from_other_workspace() {
        if !git_available() {
            return;
        }

        let repo_a = setup_repo("undo-ws-a");
        let repo_b = setup_repo("undo-ws-b");
        let repo_a_str = repo_a.to_string_lossy().to_string();
        let repo_b_str = repo_b.to_string_lossy().to_string();
        let manager = UndoManager::new(GitProvider);

        let round = manager
            .before_round(&repo_a_str, "round-a")
            .await
            .expect("before_round")
            .expect("checkpoint");
        write_file(&repo_a.join("tracked.txt"), "base\nagent\n");
        assert!(manager
            .after_round(
                "session-a",
                "msg-a",
                Some("run-a"),
                round,
                false,
                &repo_a_str
            )
            .await
            .expect("after_round"));

        let err = manager
            .perform_undo_checked("session-a", "msg-a", &repo_b_str, force_options())
            .await
            .expect_err("undo against another workspace must fail");
        match err {
            UndoPerformError::Other(msg) => {
                assert!(msg.contains("different workspace"), "got: {}", msg);
            }
            other => panic!("expected workspace mismatch error, got {:?}", other),
        }

        std::fs::remove_dir_all(&repo_a).expect("cleanup repo a");
        std::fs::remove_dir_all(&repo_b).expect("cleanup repo b");
    }
}
