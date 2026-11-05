//! A native Claude subagent's Locus record: a read-only child session of its
//! conversation that shows what the subagent did, like a Locus subagent's
//! session. Claude Code owns the agent and its context; the record is display
//! only and never runs (`SessionStore::ensure_not_native_agent_record`).
//!
//! One record per agent, also across its runs (a `SendMessage` resume) and
//! Claude processes: its id derives from the conversation and the agent id.
//! Each run opens with its prompt; the subagent's thinking and text, its calls
//! and their results follow in order. An open record reloads after changes,
//! at most once per `ANNOUNCE_DELAY`.
use crate::commands::ToolCallOutcome;
use crate::llm::claude_code_cli::ClaudeCodeNativeAgent;
use crate::session::models::{ImageData, MessageRole, ToolCallInfo};
use crate::session::store::{SessionStore, NATIVE_AGENT_SESSION_TYPE};
use crate::workspace_service::event::WorkspaceEventScope;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Each announcement reloads the session list of every window, so a busy
/// record announces at most once a second.
const ANNOUNCE_DELAY: Duration = Duration::from_millis(1000);
pub(super) const ANNOUNCE_SOURCE: &str = "claude_agent_record";

/// Tells the record's workspace that it changed (`session-content-changed`).
pub(super) type Announcer = Arc<dyn Fn(&WorkspaceEventScope, &str, &str) + Send + Sync>;

/// The record's session id for an agent of a conversation.
pub(crate) fn record_session_id(parent_session_id: &str, agent_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("locus-claude-agent-record:{parent_session_id}:{agent_id}").as_bytes(),
    )
    .to_string()
}

/// Where the conversation runs: the record joins its workspace, and an open
/// view of it learns of changes through that workspace's events.
#[derive(Clone)]
pub(super) struct RecordScope {
    pub project_id: Option<String>,
    pub checkout_id: Option<String>,
    pub working_dir: String,
    pub events: Option<WorkspaceEventScope>,
}

struct RecordedCall {
    session_id: String,
    message_id: String,
    call: ToolCallInfo,
}

#[derive(Default)]
struct RecordState {
    /// Agent id → its record, once it exists.
    sessions: HashMap<String, String>,
    /// Calls waiting for their result.
    calls: HashMap<String, RecordedCall>,
    /// Records with a pending change announcement.
    announcing: HashSet<String>,
}

pub(super) struct AgentRecords {
    parent_session_id: String,
    store: Arc<SessionStore>,
    announcer: Announcer,
    state: Arc<Mutex<RecordState>>,
}

impl AgentRecords {
    pub(super) fn new(parent_session_id: String, store: Arc<SessionStore>, announcer: Announcer) -> Self {
        Self { parent_session_id, store, announcer, state: Arc::default() }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RecordState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The agent's record, created on its first run. `created` tells the
    /// caller to announce the new session.
    fn ensure(&self, agent: &ClaudeCodeNativeAgent, scope: &RecordScope) -> Option<(String, bool)> {
        if let Some(id) = self.state().sessions.get(&agent.agent_id).cloned() {
            return Some((id, false));
        }
        let id = record_session_id(&self.parent_session_id, &agent.agent_id);
        let created = match self.store.session_type(&id) {
            Ok(Some(kind)) if kind == NATIVE_AGENT_SESSION_TYPE => false,
            Ok(Some(_)) => {
                eprintln!("[Claude subagent] record id of {} is taken by another session", agent.agent_id);
                return None;
            }
            Ok(None) => {
                let label = if agent.description.trim().is_empty() { agent.name.trim() } else { agent.description.trim() };
                let title = format!("sub:{label}");
                let create = |checkout: Option<&str>| self.store.create_session_scoped_with_id(
                    &id, &title, Some(&self.parent_session_id), scope.project_id.as_deref(), checkout,
                    NATIVE_AGENT_SESSION_TYPE, None,
                );
                // A checkout released since the conversation started still
                // keeps the record in its project.
                if let Err(error) = create(scope.checkout_id.as_deref()).or_else(|_| create(None)) {
                    eprintln!("[Claude subagent] no record for {}: {error}", agent.agent_id);
                    return None;
                }
                true
            }
            Err(error) => {
                eprintln!("[Claude subagent] no record for {}: {error}", agent.agent_id);
                return None;
            }
        };
        self.state().sessions.insert(agent.agent_id.clone(), id.clone());
        Some((id, created))
    }

    /// Content reaches the record even if it overtakes the run's start (they
    /// arrive on separate callbacks); a registered agent always has one.
    fn record(&self, agent: &ClaudeCodeNativeAgent, scope: &RecordScope) -> Option<String> {
        self.ensure(agent, scope).map(|(id, _)| id)
    }

    /// A run started: the record (created for the first run) opens it with
    /// the run's prompt. Returns the record and whether it is new.
    pub(super) fn run_started(&self, agent: &ClaudeCodeNativeAgent, scope: &RecordScope) -> Option<(String, bool)> {
        let (id, created) = self.ensure(agent, scope)?;
        if !agent.prompt.trim().is_empty() {
            if let Err(error) = self.store.add_message(&id, MessageRole::User, &agent.prompt) {
                eprintln!("[Claude subagent] prompt of {} was not recorded: {error}", agent.agent_id);
            }
        }
        self.announce(&id, scope);
        Some((id, created))
    }

    /// A model message of the subagent.
    pub(super) fn message(&self, agent: &ClaudeCodeNativeAgent, thinking: &str, text: &str, scope: &RecordScope) {
        let Some(id) = self.record(agent, scope) else { return; };
        let thinking = Some(thinking).filter(|value| !value.trim().is_empty());
        if let Err(error) = self.store.add_message_with_thinking(&id, MessageRole::Assistant, text, thinking, None, None, None, None) {
            eprintln!("[Claude subagent] message of {} was not recorded: {error}", agent.agent_id);
            return;
        }
        self.announce(&id, scope);
    }

    /// A call the subagent made (Locus name and arguments, as on its card).
    pub(super) fn call_started(&self, agent: &ClaudeCodeNativeAgent, call: &ToolCallInfo, scope: &RecordScope) {
        let Some(id) = self.record(agent, scope) else { return; };
        let mut call = call.clone();
        call.order = None;
        call.outcome = None;
        call.recorded_output = None;
        match self.store.add_assistant_with_tool_calls_and_thinking(&id, "", std::slice::from_ref(&call), None, None, None, None, None) {
            Ok(message_id) => {
                self.state().calls.insert(call.id.clone(), RecordedCall { session_id: id.clone(), message_id, call });
                self.announce(&id, scope);
            }
            Err(error) => eprintln!("[Claude subagent] call {} of {} was not recorded: {error}", call.id, agent.agent_id),
        }
    }

    /// A call's result (the complete stored output, not the card's preview).
    pub(super) fn call_finished(&self, call_id: &str, output: &str, outcome: ToolCallOutcome, images: Option<&[ImageData]>, scope: &RecordScope) {
        let Some(mut recorded) = self.state().calls.remove(call_id) else { return; };
        recorded.call.outcome = Some(outcome);
        if let Err(error) = self.store.update_message_tool_calls(&recorded.message_id, std::slice::from_ref(&recorded.call)) {
            eprintln!("[Claude subagent] outcome of {call_id} was not recorded: {error}");
        }
        if let Err(error) = self.store.add_tool_result_with_images(&recorded.session_id, call_id, output, images) {
            eprintln!("[Claude subagent] result of {call_id} was not recorded: {error}");
        }
        self.announce(&recorded.session_id, scope);
    }

    /// Coalesces change announcements: an open record reloads at most once
    /// per delay, and nothing is scheduled while the record is unchanged.
    fn announce(&self, session_id: &str, scope: &RecordScope) {
        let Some(events) = scope.events.clone() else { return; };
        if !self.state().announcing.insert(session_id.to_string()) {
            return;
        }
        let (state, announcer, session_id, working_dir) = (self.state.clone(), self.announcer.clone(), session_id.to_string(), scope.working_dir.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(ANNOUNCE_DELAY).await;
            state.lock().unwrap_or_else(|e| e.into_inner()).announcing.remove(&session_id);
            announcer(&events, &working_dir, &session_id);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::history::INTERRUPTED_TOOL_RESULT;

    fn agent(prompt: &str) -> ClaudeCodeNativeAgent {
        ClaudeCodeNativeAgent {
            agent_id: "a1".into(), parent_tool_use_id: "toolu_agent".into(), invocation_tool_use_id: "toolu_agent".into(),
            name: "general-purpose".into(), description: "Scan handouts".into(), prompt: prompt.into(),
            run_id: "run".into(), background: false,
        }
    }

    fn scope() -> RecordScope {
        RecordScope { project_id: None, checkout_id: None, working_dir: String::new(), events: None }
    }

    #[test]
    fn each_agent_of_a_conversation_has_one_stable_record() {
        let first = record_session_id("conversation", "agent-1");
        assert_eq!(first, record_session_id("conversation", "agent-1"));
        assert_ne!(first, record_session_id("conversation", "agent-2"));
        assert_ne!(first, record_session_id("other", "agent-1"));
        assert!(uuid::Uuid::parse_str(&first).is_ok());
    }

    #[test]
    fn a_record_follows_the_subagent_across_its_runs_and_processes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path()).unwrap());
        let parent = store.create_session("Parent", None, None, "chat", None).unwrap();
        let records = AgentRecords::new(parent.clone(), store.clone(), Arc::new(|_, _, _| {}));
        let (record, created) = records.run_started(&agent("Scan lectures 02-06"), &scope()).unwrap();
        assert!(created);
        let detail = store.load_session(&record).unwrap();
        assert_eq!((detail.title.as_str(), detail.session_type.as_str(), detail.parent_session_id.as_deref()),
            ("sub:Scan handouts", NATIVE_AGENT_SESSION_TYPE, Some(parent.as_str())));
        // Thinking and text, then two calls finishing out of order.
        records.message(&agent(""), "Plan the scan", "Reading the outline.", &scope());
        let read = super::super::native_agent_host::nested_record("toolu_read", "read", r#"{"filePath":"a.tex"}"#.into(), Some(3));
        let bash = super::super::native_agent_host::nested_record("toolu_bash", "bash", r#"{"command":"make"}"#.into(), Some(4));
        records.call_started(&agent(""), &read, &scope());
        records.call_started(&agent(""), &bash, &scope());
        records.call_finished("toolu_bash", "make: *** failed", ToolCallOutcome::Error, None, &scope());
        records.call_finished("toolu_read", "\\section{Plot}", ToolCallOutcome::Done, None, &scope());
        // A result is recorded once; an unknown call records nothing.
        records.call_finished("toolu_read", "again", ToolCallOutcome::Done, None, &scope());
        records.call_finished("toolu_other", "x", ToolCallOutcome::Done, None, &scope());
        // A later process resumes the same agent (SendMessage): same record.
        let restarted = AgentRecords::new(parent.clone(), store.clone(), Arc::new(|_, _, _| {}));
        let interrupted = super::super::native_agent_host::nested_record("toolu_late", "read", "{}".into(), None);
        assert_eq!(restarted.run_started(&agent("Also lecture 07"), &scope()), Some((record.clone(), false)));
        restarted.call_started(&agent(""), &interrupted, &scope());
        restarted.call_finished("toolu_late", INTERRUPTED_TOOL_RESULT, ToolCallOutcome::Interrupted, None, &scope());

        let messages = store.get_messages(&record).unwrap();
        let shape: Vec<_> = messages.iter().map(|message| (
            message.role.clone(),
            message.content.clone(),
            message.thinking_content.clone(),
            message.tool_calls.as_ref().map(|calls| calls.iter().map(|call| (call.id.clone(), call.outcome, call.order)).collect::<Vec<_>>()),
            message.tool_call_id.clone(),
        )).collect();
        assert_eq!(shape, vec![
            (MessageRole::User, "Scan lectures 02-06".into(), None, None, None),
            (MessageRole::Assistant, "Reading the outline.".into(), Some("Plan the scan".into()), None, None),
            (MessageRole::Assistant, String::new(), None, Some(vec![("toolu_read".into(), Some(ToolCallOutcome::Done), None)]), None),
            (MessageRole::Assistant, String::new(), None, Some(vec![("toolu_bash".into(), Some(ToolCallOutcome::Error), None)]), None),
            (MessageRole::Tool, "make: *** failed".into(), None, None, Some("toolu_bash".into())),
            (MessageRole::Tool, "\\section{Plot}".into(), None, None, Some("toolu_read".into())),
            (MessageRole::User, "Also lecture 07".into(), None, None, None),
            (MessageRole::Assistant, String::new(), None, Some(vec![("toolu_late".into(), Some(ToolCallOutcome::Interrupted), None)]), None),
            (MessageRole::Tool, INTERRUPTED_TOOL_RESULT.into(), None, None, Some("toolu_late".into())),
        ]);
        // Nothing can run there.
        assert!(store.try_start_run(&record, "run").is_err());
    }

    #[test]
    fn content_that_overtakes_the_run_start_still_reaches_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path()).unwrap());
        let parent = store.create_session("Parent", None, None, "chat", None).unwrap();
        let records = AgentRecords::new(parent.clone(), store.clone(), Arc::new(|_, _, _| {}));
        records.message(&agent(""), "", "Early text", &scope());
        let id = record_session_id(&parent, "a1");
        assert_eq!(store.session_type(&id).unwrap().as_deref(), Some(NATIVE_AGENT_SESSION_TYPE));
        // The run's start finds the record and adds its prompt once.
        assert_eq!(records.run_started(&agent("Scan"), &scope()), Some((id.clone(), false)));
        let contents: Vec<_> = store.get_messages(&id).unwrap().into_iter().map(|message| message.content).collect();
        assert_eq!(contents, vec!["Early text".to_string(), "Scan".to_string()]);
        // A session of another type is never written as a record.
        let taken = record_session_id(&parent, "a2");
        store.create_session_scoped_with_id(&taken, "Other", None, None, None, "chat", None).unwrap();
        let mut other = agent("x");
        other.agent_id = "a2".into();
        assert_eq!(records.run_started(&other, &scope()), None);
        assert!(store.get_messages(&taken).unwrap().is_empty());
    }
}
