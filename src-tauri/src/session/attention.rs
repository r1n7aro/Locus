//! Unread results ("attention") of sessions: a finished run, an error, a
//! question or confirmation waiting for the user, a knowledge proposal.
//!
//! The desktop keeps this state for every client, so reading a result in any
//! Locus window or on a paired phone clears it everywhere, and results that
//! arrive while a phone is offline are still unread when it reconnects. It is
//! UI state only: session messages and the session database are unchanged.
//!
//! Every stream event passes through `session::gateway::emit_stream`, which
//! records the relevant ones here. Changes are published as
//! [`SESSION_ATTENTION_CHANGED_EVENT`]; each carries the entry's revision so
//! clients can merge them with a snapshot in any order.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::StreamEvent;
use crate::session::models::SessionRunScopeSnapshot;
use crate::session::store::SessionStore;
use crate::workspace_service::event::{WorkspaceEventPublishOutcome, WorkspaceEventScope};
use crate::workspace_service::{CheckoutId, ProjectId};

pub const SESSION_ATTENTION_CHANGED_EVENT: &str = "session-attention-changed";

const FILE_NAME: &str = "session-attention.json";
const FORMAT_VERSION: u32 = 1;
/// Sessions with a recorded result. Read entries go first when exceeded.
const MAX_ENTRIES: usize = 4096;
/// Recorded result keys per session, for re-announced questions.
const MAX_SEEN_PER_SESSION: usize = 256;
/// Changes arrive in bursts (a run ends, a window reads it); one write each.
const SAVE_DELAY: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum AttentionKind {
    Done,
    Error,
    AskUser,
    ToolConfirm,
    KnowledgeProposal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttentionEntry {
    /// Global order of the latest result.
    pub sequence: u64,
    /// Results up to this sequence were read; unread while below `sequence`.
    pub read_sequence: u64,
    pub kind: AttentionKind,
    pub run_id: String,
    /// Reply message, question or proposal message of the latest result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Sequence of the latest completed run (desktop tree ordering).
    pub completion_sequence: u64,
    /// Store revision of the last change to this entry.
    pub revision: u64,
    /// Unix milliseconds of the latest result.
    pub updated_at: i64,
}

impl SessionAttentionEntry {
    pub fn unread(&self) -> bool {
        self.sequence > self.read_sequence
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttentionSnapshot {
    pub revision: u64,
    pub sequence: u64,
    pub sessions: HashMap<String, SessionAttentionEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttentionChange {
    pub session_id: String,
    pub entry: SessionAttentionEntry,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    version: u32,
    #[serde(flatten)]
    snapshot: SessionAttentionSnapshot,
}

/// The state machine, without persistence or delivery.
#[derive(Default)]
struct AttentionState {
    snapshot: SessionAttentionSnapshot,
    /// Latest run per session: results of older runs arriving late are stale.
    runs: HashMap<String, String>,
    /// Results already recorded for the session's current run.
    seen: HashMap<String, HashSet<(AttentionKind, String)>>,
}

/// Whether `event` can change attention (cheap pre-check for the stream path).
pub fn is_attention_event(event: &StreamEvent) -> bool {
    matches!(
        event,
        StreamEvent::RunStart { .. }
            | StreamEvent::Done { .. }
            | StreamEvent::Error { .. }
            | StreamEvent::AskUser { .. }
            | StreamEvent::ToolConfirm { .. }
            | StreamEvent::KnowledgeProposal { .. }
    )
}

impl AttentionState {
    fn record(&mut self, run_id: &str, event: &StreamEvent, now_ms: i64) -> Option<SessionAttentionChange> {
        let (session_id, kind, target_id) = match event {
            StreamEvent::RunStart { session_id } => {
                if self.runs.get(session_id).map(String::as_str) != Some(run_id) {
                    self.runs.insert(session_id.clone(), run_id.to_string());
                    self.seen.remove(session_id);
                }
                return None;
            }
            StreamEvent::Done { session_id, message_id, .. } => {
                (session_id, AttentionKind::Done, Some(message_id.clone()))
            }
            StreamEvent::Error { session_id, .. } => (session_id, AttentionKind::Error, None),
            StreamEvent::AskUser { session_id, question_id, .. } => {
                (session_id, AttentionKind::AskUser, Some(question_id.clone()))
            }
            StreamEvent::ToolConfirm { session_id, question_id, .. } => {
                (session_id, AttentionKind::ToolConfirm, Some(question_id.clone()))
            }
            StreamEvent::KnowledgeProposal { session_id, message } => {
                (session_id, AttentionKind::KnowledgeProposal, Some(message.id.clone()))
            }
            _ => return None,
        };
        // Knowledge proposals are standalone messages with their own run ids.
        if kind != AttentionKind::KnowledgeProposal {
            if let Some(current) = self.runs.get(session_id) {
                if current != run_id {
                    return None;
                }
            }
        }
        // One completion or failure per run; questions and proposals by id.
        let seen_target = match kind {
            AttentionKind::Done | AttentionKind::Error => run_id.to_string(),
            _ => target_id.clone().unwrap_or_default(),
        };
        let seen = self.seen.entry(session_id.clone()).or_default();
        if seen.len() >= MAX_SEEN_PER_SESSION {
            seen.clear();
        }
        if !seen.insert((kind, seen_target)) {
            return None;
        }

        let snapshot = &mut self.snapshot;
        snapshot.sequence += 1;
        snapshot.revision += 1;
        let sequence = snapshot.sequence;
        let previous = snapshot.sessions.get(session_id);
        let entry = SessionAttentionEntry {
            sequence,
            read_sequence: previous.map_or(0, |entry| entry.read_sequence),
            kind,
            run_id: run_id.to_string(),
            target_id,
            completion_sequence: if kind == AttentionKind::Done {
                sequence
            } else {
                previous.map_or(0, |entry| entry.completion_sequence)
            },
            revision: snapshot.revision,
            updated_at: now_ms,
        };
        snapshot.sessions.insert(session_id.clone(), entry.clone());
        self.evict();
        Some(SessionAttentionChange { session_id: session_id.clone(), entry })
    }

    /// Marks results up to `sequence` read. An older sequence (a client that
    /// had not seen the newest result yet) never hides a newer one.
    fn mark_read(&mut self, session_id: &str, sequence: u64) -> Option<SessionAttentionChange> {
        let snapshot = &mut self.snapshot;
        let entry = snapshot.sessions.get_mut(session_id)?;
        let read = sequence.min(entry.sequence);
        if read <= entry.read_sequence {
            return None;
        }
        snapshot.revision += 1;
        entry.read_sequence = read;
        entry.revision = snapshot.revision;
        Some(SessionAttentionChange { session_id: session_id.to_string(), entry: entry.clone() })
    }

    fn evict(&mut self) {
        let sessions = &mut self.snapshot.sessions;
        if sessions.len() <= MAX_ENTRIES {
            return;
        }
        let mut order: Vec<(bool, u64, String)> = sessions
            .iter()
            .map(|(id, entry)| (entry.unread(), entry.sequence, id.clone()))
            .collect();
        // Read before unread, then oldest first.
        order.sort();
        let excess = sessions.len() - MAX_ENTRIES;
        for (_, _, id) in order.into_iter().take(excess) {
            sessions.remove(&id);
            self.runs.remove(&id);
            self.seen.remove(&id);
        }
    }
}

pub struct SessionAttentionStore {
    path: PathBuf,
    state: Mutex<AttentionState>,
    save_scheduled: AtomicBool,
}

impl SessionAttentionStore {
    /// Loads `session-attention.json` from the data directory. A missing or
    /// unreadable file starts empty; the next change replaces it.
    pub fn load(data_dir: &Path) -> Arc<Self> {
        let path = data_dir.join(FILE_NAME);
        let snapshot = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedState>(&bytes) {
                Ok(state) if state.version == FORMAT_VERSION => state.snapshot,
                Ok(state) => {
                    tracing::warn!(
                        log_module = "SessionAttention",
                        "ignoring {} with unsupported version {}",
                        path.display(),
                        state.version
                    );
                    SessionAttentionSnapshot::default()
                }
                Err(error) => {
                    tracing::warn!(
                        log_module = "SessionAttention",
                        "ignoring unreadable {}: {}",
                        path.display(),
                        error
                    );
                    SessionAttentionSnapshot::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                SessionAttentionSnapshot::default()
            }
            Err(error) => {
                tracing::warn!(
                    log_module = "SessionAttention",
                    "could not read {}: {}",
                    path.display(),
                    error
                );
                SessionAttentionSnapshot::default()
            }
        };
        Arc::new(Self {
            path,
            state: Mutex::new(AttentionState { snapshot, ..Default::default() }),
            save_scheduled: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AttentionState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn snapshot(&self) -> SessionAttentionSnapshot {
        self.lock().snapshot.clone()
    }

    pub fn record(self: &Arc<Self>, run_id: &str, event: &StreamEvent) -> Option<SessionAttentionChange> {
        let change = self.lock().record(run_id, event, now_ms());
        if change.is_some() {
            self.schedule_save();
        }
        change
    }

    pub fn mark_read(self: &Arc<Self>, session_id: &str, sequence: u64) -> Option<SessionAttentionChange> {
        let change = self.lock().mark_read(session_id, sequence);
        if change.is_some() {
            self.schedule_save();
        }
        change
    }

    fn schedule_save(self: &Arc<Self>) {
        if self.save_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let store = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = store.save_now() {
                    tracing::warn!(log_module = "SessionAttention", "save failed: {}", error);
                }
            })
            .await;
        });
    }

    fn save_now(&self) -> Result<(), String> {
        // Cleared before the copy: a change made after it schedules another save.
        self.save_scheduled.store(false, Ordering::Release);
        let state = PersistedState { version: FORMAT_VERSION, snapshot: self.snapshot() };
        let bytes = serde_json::to_vec(&state).map_err(|error| error.to_string())?;
        crate::config::atomic_write_config(&self.path, &bytes)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Workspace scope of a run, for routing its attention change to the clients
/// watching that checkout (paired phones only receive scoped events).
pub fn run_event_scope(scope: &SessionRunScopeSnapshot) -> Option<WorkspaceEventScope> {
    Some(WorkspaceEventScope {
        project_id: ProjectId::new(scope.project_id.clone()).ok()?,
        checkout_id: CheckoutId::new(scope.checkout_id.clone()).ok()?,
        workspace_generation: scope.workspace_generation,
        materialization_epoch: scope.materialization_epoch,
        service_instance_id: None,
        service_generation: None,
    })
}

/// Delivers a change to desktop windows and, through the workspace event
/// router, to paired phones interested in that checkout. Without a current
/// scope only desktop windows receive it; phones catch up from the snapshot.
pub fn publish_change(
    app_handle: &AppHandle,
    scope: Option<&WorkspaceEventScope>,
    change: &SessionAttentionChange,
) {
    if let Some(scope) = scope {
        let outcome = crate::workspace_service::event::emit_for_workspace_scope(
            app_handle,
            scope,
            SESSION_ATTENTION_CHANGED_EVENT,
            change.clone(),
        );
        if outcome == WorkspaceEventPublishOutcome::PublishedScoped {
            return;
        }
    }
    if let Err(error) = app_handle.emit(SESSION_ATTENTION_CHANGED_EVENT, change) {
        tracing::warn!(log_module = "SessionAttention", "emit failed: {}", error);
    }
}

#[tauri::command]
pub fn get_session_attention(
    attention: tauri::State<'_, Arc<SessionAttentionStore>>,
) -> SessionAttentionSnapshot {
    attention.snapshot()
}

/// Marks a session's results up to `sequence` read for every client.
#[tauri::command]
pub fn mark_session_attention_read(
    session_id: String,
    sequence: u64,
    app_handle: AppHandle,
    attention: tauri::State<'_, Arc<SessionAttentionStore>>,
    store: tauri::State<'_, Arc<SessionStore>>,
) -> Option<SessionAttentionEntry> {
    let change = attention.mark_read(&session_id, sequence)?;
    let scope = crate::session::gateway::live_session_event_scope(&app_handle, &store, &session_id);
    publish_change(&app_handle, scope.as_ref(), &change);
    Some(change.entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    fn done(session: &str, message: &str) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "done", "sessionId": session, "messageId": message, "fullText": "Finished"
        }))
        .expect("done event")
    }

    fn run_start(session: &str) -> StreamEvent {
        StreamEvent::RunStart { session_id: session.into() }
    }

    fn ask(session: &str, question: &str) -> StreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "askUser", "sessionId": session, "questionId": question, "toolCallId": "tool",
            "question": "Choose", "options": []
        }))
        .expect("ask event")
    }

    fn error(session: &str) -> StreamEvent {
        StreamEvent::Error { session_id: session.into(), error: AppError::new("x", "failed") }
    }

    fn unread(state: &AttentionState, session: &str) -> bool {
        state.snapshot.sessions.get(session).is_some_and(SessionAttentionEntry::unread)
    }

    #[test]
    fn results_are_unread_until_read_and_completions_are_ordered() {
        let mut state = AttentionState::default();
        assert!(state.record("r1", &run_start("a"), 1).is_none());
        let first = state.record("r1", &done("a", "m1"), 1).expect("done recorded");
        assert_eq!(first.entry.kind, AttentionKind::Done);
        assert_eq!(first.entry.completion_sequence, first.entry.sequence);
        assert!(unread(&state, "a"));

        let read = state.mark_read("a", first.entry.sequence).expect("read recorded");
        assert_eq!(read.entry.read_sequence, first.entry.sequence);
        assert!(read.entry.revision > first.entry.revision);
        assert!(!unread(&state, "a"));
        // Reading again changes nothing.
        assert!(state.mark_read("a", first.entry.sequence).is_none());

        state.record("r2", &run_start("b"), 2);
        let question = state.record("r2", &ask("b", "q1"), 2).expect("question recorded");
        assert_eq!(question.entry.completion_sequence, 0, "questions are not completions");
        assert!(unread(&state, "b"));
    }

    #[test]
    fn repeated_and_stale_results_are_ignored() {
        let mut state = AttentionState::default();
        state.record("r1", &run_start("a"), 1);
        let first = state.record("r1", &done("a", "m1"), 1).expect("done");
        // A second terminal event of the same run.
        assert!(state.record("r1", &done("a", "other"), 1).is_none());
        state.record("r2", &run_start("a"), 2);
        let second = state.record("r2", &done("a", "m2"), 2).expect("second run");
        // A late result of the first run does not replace the second.
        assert!(state.record("r1", &error("a"), 3).is_none());
        // Reading the first result keeps the newer one unread.
        assert!(state.mark_read("a", first.entry.sequence).is_some());
        assert!(unread(&state, "a"));
        assert_eq!(state.snapshot.sessions["a"].run_id, "r2");
        assert_eq!(state.snapshot.sessions["a"].sequence, second.entry.sequence);
    }

    #[test]
    fn re_announced_questions_count_once_and_proposals_ignore_run_ids() {
        let mut state = AttentionState::default();
        state.record("r1", &run_start("a"), 1);
        assert!(state.record("r1", &ask("a", "q1"), 1).is_some());
        assert!(state.record("r1", &ask("a", "q2"), 1).is_some());
        assert!(state.record("r1", &ask("a", "q1"), 1).is_none());

        let proposal: StreamEvent = serde_json::from_value(serde_json::json!({
            "type": "knowledgeProposal", "sessionId": "a",
            "message": {"id": "p1", "role": "assistant", "content": "", "createdAt": 1}
        }))
        .expect("proposal event");
        let change = state.record("knowledge_5", &proposal, 2).expect("proposal recorded");
        assert_eq!(change.entry.kind, AttentionKind::KnowledgeProposal);
        assert_eq!(change.entry.target_id.as_deref(), Some("p1"));
        assert!(state.record("knowledge_6", &proposal, 2).is_none());
    }

    #[test]
    fn cancellation_and_deltas_change_nothing_and_unread_survives_a_new_run() {
        let mut state = AttentionState::default();
        state.record("r1", &run_start("a"), 1);
        state.record("r1", &done("a", "m1"), 1);
        let revision = state.snapshot.revision;
        state.record("r2", &run_start("a"), 2);
        let cancelled: StreamEvent =
            serde_json::from_value(serde_json::json!({"type": "cancelled", "sessionId": "a"}))
                .expect("cancelled event");
        assert!(!is_attention_event(&cancelled));
        assert!(state.record("r2", &cancelled, 2).is_none());
        assert_eq!(state.snapshot.revision, revision);
        assert!(unread(&state, "a"));
    }

    #[test]
    fn eviction_drops_read_entries_first() {
        let mut state = AttentionState::default();
        for index in 0..=MAX_ENTRIES {
            let session = format!("s{index}");
            let change = state.record("r", &done(&session, "m"), 1).expect("recorded");
            if index != 0 {
                state.mark_read(&session, change.entry.sequence);
            }
        }
        assert_eq!(state.snapshot.sessions.len(), MAX_ENTRIES);
        assert!(state.snapshot.sessions.contains_key("s0"), "the only unread entry stays");
        assert!(!state.snapshot.sessions.contains_key("s1"), "the oldest read entry goes");
    }

    #[test]
    fn state_survives_a_reload_and_ignores_corrupt_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = SessionAttentionStore::load(dir.path());
        store.lock().record("r1", &done("a", "m1"), 1);
        store.save_now().expect("save");
        let reloaded = SessionAttentionStore::load(dir.path());
        assert_eq!(reloaded.snapshot(), store.snapshot());
        assert!(reloaded.snapshot().sessions["a"].unread());

        std::fs::write(dir.path().join(FILE_NAME), b"{not json").expect("write");
        assert_eq!(SessionAttentionStore::load(dir.path()).snapshot(), SessionAttentionSnapshot::default());
    }
}
