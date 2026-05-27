use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{Checkpoint, GitProvider, VcsProvider};

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
    _workspace_guard: OwnedMutexGuard<()>,
}

#[derive(Debug)]
pub enum UndoPerformError {
    Conflict(Vec<UndoConflict>),
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
    workspace_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    next_checkpoint_created_at: AtomicI64,
}

fn normalize_path_key(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn collect_changed_file_keys(file: &ChangedFile, keys: &mut std::collections::HashSet<String>) {
    keys.insert(normalize_path_key(&file.path));
    if let Some(old_path) = &file.old_path {
        keys.insert(normalize_path_key(old_path));
    }
}

fn changed_file_overlaps(
    target_keys: &std::collections::HashSet<String>,
    file: &ChangedFile,
) -> bool {
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
    seen: &mut std::collections::HashSet<String>,
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

impl UndoManager {
    pub fn new(provider: GitProvider) -> Self {
        UndoManager {
            provider,
            stacks: Mutex::new(HashMap::new()),
            workspace_guards: Mutex::new(HashMap::new()),
            next_checkpoint_created_at: AtomicI64::new(0),
        }
    }

    async fn acquire_workspace_guard(&self, working_dir: &str) -> OwnedMutexGuard<()> {
        let key = normalize_path_key(working_dir);
        let workspace_mutex = {
            let mut guards = self.workspace_guards.lock().await;
            guards
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        workspace_mutex.lock_owned().await
    }

    fn next_checkpoint_created_at(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

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
        let mut target_keys = std::collections::HashSet::new();
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
        let workspace_guard = self.acquire_workspace_guard(working_dir).await;
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
        let changed_files: Vec<ChangedFile> =
            super::GitProvider::diff_files(working_dir, &checkpoint.id)
                .await
                .map_err(|e| {
                    eprintln!(
                        "[UndoManager] failed to capture undo diff for session {} message {} checkpoint {}: {}",
                        session_id, assistant_message_id, checkpoint.id, e
                    );
                    format!("failed to capture undo diff: {}", e)
                })?
                .iter()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() < 2 {
                        return None;
                    }
                    let status = parts[0].chars().next()?.to_string();
                    let file = if parts.len() >= 3 {
                        // Rename: "R100\told_path\tnew_path"
                        ChangedFile {
                            status,
                            path: parts[2].trim().to_string(),
                            old_path: Some(parts[1].trim().to_string()),
                        }
                    } else {
                        ChangedFile {
                            status,
                            path: parts[1].trim().to_string(),
                            old_path: None,
                        }
                    };
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
            changed_files,
            has_unity_execute,
            consumed: false,
        };
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
        let mut stacks = self.stacks.lock().await;
        stacks
            .entry(session_id.to_string())
            .or_default()
            .push(entry);
        Ok(true)
    }

    pub async fn perform_undo_checked(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        working_dir: &str,
        force: bool,
    ) -> Result<UndoPerformResult, UndoPerformError> {
        let _workspace_guard = self.acquire_workspace_guard(working_dir).await;

        let (checkpoint_id, all_changed, need_reload, result, consumed_ids) = {
            let mut stacks = self.stacks.lock().await;

            if !force {
                let conflicts =
                    Self::collect_conflicts_from_stacks(&stacks, session_id, assistant_message_id)
                        .map_err(UndoPerformError::Other)?;
                if !conflicts.is_empty() {
                    return Err(UndoPerformError::Conflict(conflicts));
                }
            }

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

            let mut seen = std::collections::HashSet::new();
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

        if let Err(e) = super::GitProvider::restore_files(
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

    pub async fn perform_undo(
        &self,
        session_id: &str,
        assistant_message_id: &str,
        working_dir: &str,
    ) -> Result<UndoEntry, String> {
        self.perform_undo_checked(session_id, assistant_message_id, working_dir, true)
            .await
            .map(|result| result.entry)
            .map_err(|e| match e {
                UndoPerformError::Conflict(_) => {
                    "undo conflict check unexpectedly failed under force mode".to_string()
                }
                UndoPerformError::Other(msg) => msg,
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

        let mut seen = std::collections::HashSet::new();
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

    pub async fn on_session_delete(&self, session_id: &str, working_dir: &str) {
        let mut stacks = self.stacks.lock().await;
        if let Some(stack) = stacks.remove(session_id) {
            for entry in &stack {
                let _ = self
                    .provider
                    .discard(working_dir, &entry.checkpoint.id)
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UndoManager, UndoPerformError};
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
            .perform_undo_checked("session-a", "msg-a", &repo_str, true)
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
                .perform_undo_checked("session-a", "msg-a", &repo_for_undo, false)
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
            Err(UndoPerformError::Other(msg)) => {
                panic!("expected conflict, got error: {}", msg);
            }
            Ok(_) => panic!("expected conflict, got successful undo"),
        }

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }
}
