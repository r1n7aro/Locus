//! The native Claude subagents of one conversation.
//!
//! A subagent's bridged calls run with the parent's authority (tool
//! permissions, confirmations, Plan mode, workspace lock) but outside the
//! parent's assistant rounds and outside any Locus run: the session's agent
//! pump calls the hub concurrently, and a background agent keeps working
//! while the conversation is idle. Each write takes its own workspace lock
//! and undo checkpoint, attributed to the assistant message holding the
//! agent's `Agent` call (its card).
//!
//! Records nest under the card. A foreground card takes them with the
//! `Agent` result; a background card (and every resumed run) persists each
//! record as it happens. A background run is a Locus async task named by
//! the agent id: the panel shows and stops it, and Locus delivers its result
//! to the conversation (waking it when idle) instead of Claude's own notice.
use super::*;
use super::native_agent_host::nested_record;
use super::native_agent_record::{AgentRecords, RecordScope};
use super::super::claude_native_agents::native_agent_tool_refusal;
use crate::llm::claude_code_cli::{
    ClaudeCodeAgentNotice, ClaudeCodeNativeAgent, ClaudeCodeUnitFuture, ClaudeNativeAgentHost,
};
use std::sync::{Mutex, RwLock, Weak};
use tauri::Manager;

/// Live and stored output of a nested call. The complete result stays in
/// the subagent's own context.
const NESTED_OUTPUT_PREVIEW_CHARS: usize = 500;
/// Async task tool name of a background native agent.
pub(super) const NATIVE_AGENT_TASK_TOOL: &str = "Agent";
/// What a native subagent's Agent card shows beyond its call: the model it
/// runs on and its record (`src/services/claudeAgentCards.ts`).
const CLAUDE_AGENT_CARD_EVENT: &str = "claude-agent-card";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeAgentCardEvent {
    session_id: String,
    /// The card: the `Agent` call that started the subagent.
    tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_session_id: Option<String>,
}

fn output_preview(output: &str) -> String {
    let count = output.chars().count();
    if count <= NESTED_OUTPUT_PREVIEW_CHARS {
        return output.to_string();
    }
    let prefix: String = output.chars().take(NESTED_OUTPUT_PREVIEW_CHARS).collect();
    format!("{prefix}... ({count} chars)")
}

/// Card text of a continued agent: each run's result in order. Display only;
/// the model received each result when its run ended.
fn card_with_continuation(earlier: &str, output: &str) -> String {
    if output.trim().is_empty() {
        return earlier.to_string();
    }
    format!("{}\n\n---\n\n{}", earlier.trim_end(), output.trim_start())
}

fn error_result(output: impl Into<String>) -> ExecutedToolResult {
    ExecutedToolResult::from_tool_result(crate::tool::ToolResult {
        output: output.into(),
        is_error: true,
    })
}

/// The latest run's authority: a background copy of its agent, so a call
/// never borrows a run that may have ended.
struct Authority {
    agent: AgentInstance,
    run_id: String,
    mode: String,
    active_skill_tool_names: HashSet<String>,
    _cancel: tokio::sync::watch::Sender<bool>,
}

/// The assistant message holding a card, and the run that created it.
#[derive(Clone)]
struct Card {
    message_id: String,
    run_id: String,
}

#[derive(Default)]
struct HubState {
    cards: HashMap<String, Card>,
    /// Records of foreground cards, until the Agent result takes them.
    records: HashMap<String, Vec<ToolCallInfo>>,
    finished: HashSet<String>,
    /// Background agent id → its async task.
    tasks: HashMap<String, String>,
    /// Task id of a resumed run → what its card showed before the run: the
    /// earlier runs' results, which the new result follows instead of replacing.
    earlier_output: HashMap<String, String>,
    usage: HashSet<String>,
    order: u32,
}

pub(super) struct NativeAgentHub {
    session_id: String,
    app: AppHandle,
    store: Arc<SessionStore>,
    authority: RwLock<Option<Arc<Authority>>>,
    state: Mutex<HubState>,
    /// Each agent's read-only Locus record (a child session).
    records: AgentRecords,
}

fn hubs() -> &'static Mutex<HashMap<String, Weak<NativeAgentHub>>> {
    static HUBS: std::sync::OnceLock<Mutex<HashMap<String, Weak<NativeAgentHub>>>> = std::sync::OnceLock::new();
    HUBS.get_or_init(|| Mutex::new(HashMap::new()))
}

impl NativeAgentHub {
    /// The conversation's hub, now acting with this run's authority. The
    /// Claude session keeps it alive (see `tui::agent_pump`).
    pub(super) fn install(host: &ClaudeCodeRoundHost<'_>) -> Arc<Self> {
        let session_id = host.agent.session_id.clone();
        let hub = {
            let mut hubs = hubs().lock().unwrap_or_else(|e| e.into_inner());
            hubs.retain(|_, hub| hub.strong_count() > 0);
            match hubs.get(&session_id).and_then(Weak::upgrade) {
                Some(hub) => hub,
                None => {
                    let store = host.app_handle.state::<Arc<SessionStore>>().inner().clone();
                    let app = host.app_handle.clone();
                    let announcer: super::native_agent_record::Announcer = Arc::new(move |events, working_dir, record| {
                        crate::commands::emit_session_content_changed(&app, Some(events), working_dir, record,
                            super::native_agent_record::ANNOUNCE_SOURCE);
                    });
                    let hub = Arc::new(Self {
                        session_id: session_id.clone(),
                        app: host.app_handle.clone(),
                        records: AgentRecords::new(session_id.clone(), store.clone(), announcer),
                        store,
                        authority: RwLock::new(None),
                        state: Mutex::new(HubState::default()),
                    });
                    hubs.insert(session_id, Arc::downgrade(&hub));
                    hub
                }
            }
        };
        let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
        *hub.authority.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Authority {
            agent: host.agent.clone_for_background_task(cancel_rx),
            run_id: host.run_id.to_string(),
            mode: host.mode.to_string(),
            active_skill_tool_names: host.active_skill_tool_names.clone(),
            _cancel: cancel,
        }));
        hub
    }

    pub(super) fn existing(session_id: &str) -> Option<Arc<Self>> {
        hubs().lock().unwrap_or_else(|e| e.into_inner()).get(session_id).and_then(Weak::upgrade)
    }

    fn authority(&self) -> Option<Arc<Authority>> {
        self.authority.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The conversation's workspace, which its agents' records join.
    fn record_scope(&self) -> RecordScope {
        let authority = self.authority();
        let instance = authority.as_ref().map(|authority| &authority.agent);
        let execution = instance.and_then(|instance| instance.execution_context.as_ref());
        RecordScope {
            project_id: instance.and_then(|instance| instance.workspace_id.clone()),
            checkout_id: execution.map(|execution| execution.checkout_id.as_str().to_string()),
            working_dir: instance.map(|instance| instance.working_dir.clone()).unwrap_or_default(),
            events: execution.map(|execution| crate::workspace_service::event::WorkspaceEventScope::for_runtime(execution.workspace.as_ref())),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claude is about to start this `Agent` call of `run_id`.
    pub(super) fn note_agent_call(&self, card_id: &str, message_id: Option<String>, run_id: &str) {
        if let Some(message_id) = message_id {
            self.state().cards.insert(card_id.to_string(), Card { message_id, run_id: run_id.to_string() });
        }
    }

    /// The card's message; a resumed agent of an earlier process finds its
    /// persisted call.
    fn card(&self, card_id: &str) -> Option<Card> {
        if let Some(card) = self.state().cards.get(card_id).cloned() {
            return Some(card);
        }
        let messages = self.store.get_messages(&self.session_id).ok()?;
        let message = messages.iter().rev().find(|message| {
            message.tool_calls.as_ref().is_some_and(|calls| calls.iter().any(|call| call.id == card_id))
        })?;
        let card = Card {
            message_id: message.id.clone(),
            run_id: self.authority().map(|authority| authority.run_id.clone()).unwrap_or_default(),
        };
        self.state().cards.insert(card_id.to_string(), card.clone());
        Some(card)
    }

    /// Tells the open conversation what the agent's card links to.
    fn announce_card(&self, agent: &ClaudeCodeNativeAgent, model: Option<String>, record_session_id: Option<String>) {
        let event = ClaudeAgentCardEvent {
            session_id: self.session_id.clone(),
            tool_call_id: agent.parent_tool_use_id.clone(),
            model,
            record_session_id,
        };
        if let Err(error) = self.app.emit(CLAUDE_AGENT_CARD_EVENT, event) {
            eprintln!("[Claude subagent] failed to update the card of {}: {error}", agent.agent_id);
        }
    }

    fn parent(&self, card_id: &str, run_id: &str) -> super::super::ParentToolCall {
        super::super::ParentToolCall::new(self.session_id.clone(), run_id.to_string(), card_id.to_string())
    }

    /// Nested records of a finished foreground `Agent` call. Calls without
    /// a result ended with their subagent.
    pub(super) fn take_records(&self, card_id: &str) -> Vec<ToolCallInfo> {
        let (mut calls, run_id) = {
            let mut state = self.state();
            let run_id = state.cards.get(card_id).map(|card| card.run_id.clone()).unwrap_or_default();
            (state.records.remove(card_id).unwrap_or_default(), run_id)
        };
        let parent = self.parent(card_id, &run_id);
        let scope = self.record_scope();
        for call in calls.iter_mut().filter(|call| call.outcome.is_none()) {
            call.outcome = Some(ToolCallOutcome::Interrupted);
            self.state().finished.insert(call.id.clone());
            emit_parent_stream(
                &self.app,
                parent.subagent_tool_call_done(call.id.clone(), call.name.clone(), String::new(), ToolCallOutcome::Interrupted, None),
            );
            self.records.call_finished(&call.id, crate::session::history::INTERRUPTED_TOOL_RESULT, ToolCallOutcome::Interrupted, None, &scope);
        }
        calls
    }

    /// A background card is already persisted: its records are written as
    /// they change and pushed to the open conversation.
    fn persist_record(&self, card_id: &str, record: &ToolCallInfo) {
        let Some(card) = self.card(card_id) else { return; };
        if let Err(error) = self.store.upsert_nested_tool_call(&card.message_id, card_id, record) {
            eprintln!("[Claude subagent] failed to persist nested call {} of {card_id}: {error}", record.id);
            return;
        }
        let event = crate::async_tasks::NestedToolCallUpdatedEvent {
            session_id: self.session_id.clone(),
            assistant_message_id: card.message_id,
            tool_call_id: card_id.to_string(),
            nested_tool_call: record.clone(),
        };
        if let Err(error) = self.app.emit(crate::async_tasks::NESTED_TOOL_CALL_UPDATED_EVENT, event) {
            eprintln!("[Claude subagent] failed to emit nested call update: {error}");
        }
    }

    fn record_started(&self, agent: &ClaudeCodeNativeAgent, id: &str, name: &str, arguments: String) {
        let card_id = agent.parent_tool_use_id.clone();
        let (record, run_id) = {
            let mut state = self.state();
            if state.finished.contains(id) || state.records.get(&card_id).is_some_and(|calls| calls.iter().any(|call| call.id == id)) {
                return;
            }
            state.order += 1;
            let record = nested_record(id, name, arguments, Some(state.order));
            if !agent.background {
                state.records.entry(card_id.clone()).or_default().push(record.clone());
            } else {
                // Deduplicates a re-announced background call.
                state.records.entry(format!("background:{card_id}")).or_default().push(record.clone());
            }
            let run_id = state.cards.get(&card_id).map(|card| card.run_id.clone())
                .or_else(|| self.authority().map(|authority| authority.run_id.clone())).unwrap_or_default();
            (record, run_id)
        };
        emit_parent_stream(
            &self.app,
            self.parent(&card_id, &run_id).subagent_tool_call_start(
                id.to_string(), name.to_string(), record.arguments.clone(), record.order, Some(id.to_string()), record.order,
            ),
        );
        if agent.background {
            self.persist_record(&card_id, &record);
        }
        self.records.call_started(agent, &record, &self.record_scope());
    }

    /// Records a nested call's outcome once and updates the card.
    fn record_finished(&self, agent: &ClaudeCodeNativeAgent, id: &str, result: &ExecutedToolResult, images: Option<&[ImageData]>) {
        let card_id = agent.parent_tool_use_id.clone();
        let outcome = result.outcome.as_stream_outcome();
        let preview = output_preview(&result.output);
        let (record, run_id) = {
            let mut state = self.state();
            if !state.finished.insert(id.to_string()) {
                return;
            }
            let key = if agent.background { format!("background:{card_id}") } else { card_id.clone() };
            let Some(call) = state.records.get_mut(&key).and_then(|calls| calls.iter_mut().find(|call| call.id == id)) else {
                return;
            };
            call.outcome = Some(outcome);
            call.recorded_output = Some(preview.clone());
            let record = call.clone();
            let run_id = state.cards.get(&card_id).map(|card| card.run_id.clone()).unwrap_or_default();
            (record, run_id)
        };
        emit_parent_stream(
            &self.app,
            self.parent(&card_id, &run_id).subagent_tool_call_done(
                id.to_string(), record.name.clone(), preview, outcome, images.map(<[ImageData]>::to_vec),
            ),
        );
        if agent.background {
            self.persist_record(&card_id, &record);
        }
        // The record keeps the complete result, as a Locus subagent's session does.
        self.records.call_finished(id, &result.output, outcome, images, &self.record_scope());
    }

    fn refusal(authority: &Authority, agent: &ClaudeCodeNativeAgent, name: &str, args: &serde_json::Value) -> Option<String> {
        if name == "Agent" {
            return Some("Claude Code subagents cannot start other agents; no agent was started.".into());
        }
        if let Some(error) = native_agent_tool_refusal(name) {
            return Some(error);
        }
        if agent.background && name == "ask_user_question" {
            return Some("A background agent cannot ask the user. Finish with your question in the final report instead.".into());
        }
        // Like a Locus subagent of a planning session: strictly read-only,
        // including the plan file.
        if authority.agent.plan_runtime_snapshot().is_some()
            && !super::super::plan_policy::allows_observation(name, args)
        {
            return Some(format!(
                "Tool '{name}' is not allowed: the parent session is in plan mode, so this subagent is strictly read-only."
            ));
        }
        None
    }

    async fn execute_call(&self, agent: &ClaudeCodeNativeAgent, id: &str, name: &str, arguments: serde_json::Value) -> ClaudeCodeToolResult {
        let Some(authority) = self.authority() else {
            return ClaudeCodeToolResult::from(crate::tool::ToolResult { output: "Locus is not ready to run this subagent's tool.".into(), is_error: true });
        };
        let call = nested_record(id, name, serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string()), None);
        let mut args = arguments;
        normalize_tool_args(&mut args);
        authority.agent.inject_working_dir(&call.name, &mut args);
        let message_id = self.card(&agent.parent_tool_use_id).map(|card| card.message_id).unwrap_or_default();
        let label = if agent.description.trim().is_empty() { agent.name.clone() } else { agent.description.clone() };
        let mut result = match Self::refusal(&authority, agent, &call.name, &args) {
            Some(error) => error_result(error),
            None => super::super::CONFIRM_AGENT_LABEL
                .scope(label, self.run_locked(&authority, agent, &call, &args, &message_id))
                .await,
        };
        let mut stored_output = match self.store.rewrite_tool_result_for_storage(&self.session_id, id, &call.name, &result.output) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("[Claude subagent] failed to bound tool result for '{}' (id={id}): {error}", call.name);
                result.output.clone()
            }
        };
        if !result.is_error && stored_output != result.output {
            bound_native_tool_result(id, &stored_output);
        }
        validate_native_tool_result(id, &mut result, &mut stored_output);
        let images = result.images.clone();
        let mut recorded = result.clone();
        recorded.output = stored_output.clone();
        self.record_finished(agent, id, &recorded, images.as_deref());
        ClaudeCodeToolResult { output: stored_output, is_error: result.is_error, images, context: Vec::new() }
    }

    async fn run_locked(
        &self,
        authority: &Authority,
        agent: &ClaudeCodeNativeAgent,
        call: &ToolCallInfo,
        args: &serde_json::Value,
        message_id: &str,
    ) -> ExecutedToolResult {
        let instance = &authority.agent;
        let run_id = authority.run_id.as_str();
        // Without snapshots, execute_single_tool holds this call's path guard.
        let lock_request = if instance.session_undo_enabled {
            instance.workspace_execution_request_for_tool(&call.name, args)
        } else {
            None
        };
        // Confirm before taking the workspace lock, as the parent's rounds do.
        // Other tools confirm inside execute_single_tool.
        let mut preapproved = false;
        if lock_request.is_some() || call.name == "bash" {
            let decision = instance
                .request_tool_confirm(&self.app, &call.id, &call.name, &call.arguments, args, run_id)
                .await;
            if let Some(result) = instance.confirmation_rejection_result(&call.name, decision) {
                return result;
            }
            preapproved = true;
        }
        let mut workspace_guard = None;
        let mut undo_guard = None;
        if let Some(request) = lock_request {
            let Some(execution) = instance.execution_context.as_ref() else {
                return error_result("File mutation requires a checkout-scoped AgentExecutionContext.");
            };
            let owner = WorkspaceExecutionLockOwner {
                session_id: self.session_id.clone(),
                run_id: format!("claude-agent:{}", agent.agent_id),
                iteration: 0,
                workspace: instance.working_dir.clone(),
                tools: vec![call.name.clone()],
            };
            match process_workspace_execution_lock(&instance.working_dir)
                .acquire_with_diagnostics(
                    request,
                    owner,
                    instance.cancel_waiter(),
                    crate::workspace_service::event::WorkspaceEventScope::for_runtime(execution.workspace.as_ref()),
                    &self.app,
                )
                .await
            {
                Ok(guard) => workspace_guard = Some(guard),
                Err(_) => return AgentInstance::interrupted_tool_result(),
            }
            if instance.should_track_session_undo(&call.name, args) {
                if let Some(undo_mgr) = instance.undo_manager.as_ref() {
                    match undo_mgr.before_round(&instance.working_dir, "claude subagent tool").await {
                        Ok(checkpoint) => undo_guard = checkpoint,
                        Err(error) => {
                            eprintln!("[Claude subagent] undo checkpoint failed: {error}");
                            emit_undo_checkpoint_warning(&self.app, &error);
                        }
                    }
                }
            }
        }
        let active_skill_tool_names = instance.active_skill_tool_names(&authority.active_skill_tool_names);
        let result = instance
            .execute_single_tool(
                &self.app,
                &self.store,
                call,
                args,
                run_id,
                message_id,
                &authority.mode,
                &active_skill_tool_names,
                preapproved,
            )
            .await;
        let queued_unity_asset_paths = instance.unity_asset_relative_paths(call, args, &result);
        // One batch per write: undo on the card's message, then unlock. A
        // background write belongs to no Locus run.
        record_cli_side_effects(
            instance,
            &self.app,
            run_id,
            (!agent.background).then_some(run_id),
            CliRoundCompletion {
                message_id: message_id.to_string(),
                undo_guard,
                has_unity_execute: AgentInstance::is_unity_execute_undo_call(&call.name, args),
                queued_unity_asset_paths,
                workspace_guard,
            },
        )
        .await;
        result
    }

    /// `SendMessage` from a subagent to the main agent: a message in the
    /// conversation's mailbox, delivered like a task result (waking it when
    /// idle). It does not end the subagent.
    fn message_main(&self, agent: &ClaudeCodeNativeAgent, input: &serde_json::Value) -> ClaudeCodeToolResult {
        let failure = |output: String| ClaudeCodeToolResult::from(crate::tool::ToolResult { output, is_error: true });
        if input["to"] != "main" {
            return failure("A Claude Code subagent can send messages only to the main agent (to: \"main\").".into());
        }
        let Some(message) = input["message"].as_str().filter(|text| !text.trim().is_empty()) else {
            return failure("SendMessage requires a non-empty message; no message was sent.".into());
        };
        let manager = self.app.state::<Arc<crate::async_tasks::AsyncTaskManager>>().inner().clone();
        let queued = self.store.queue_agent_message(&self.session_id, &self.session_id, &agent.agent_id, message, None);
        match queued {
            Ok(message_id) => {
                manager.ensure_parent_message_delivery(self.app.clone(), self.session_id.clone(), message_id);
                ClaudeCodeToolResult::from(crate::tool::ToolResult {
                    output: serde_json::json!({"success": true, "message": "Message sent to the main agent. Continue your task; your final answer is delivered separately."}).to_string(),
                    is_error: false,
                })
            }
            Err(error) => failure(format!("The message could not be queued: {error}")),
        }
    }

    fn task_manager(&self) -> Arc<crate::async_tasks::AsyncTaskManager> {
        self.app.state::<Arc<crate::async_tasks::AsyncTaskManager>>().inner().clone()
    }

    /// A background run: the async task named by the agent id, a new attempt
    /// for a resumed agent. Stopping the task stops the agent.
    fn start_task(&self, agent: &ClaudeCodeNativeAgent) {
        let manager = self.task_manager();
        let working_dir = self.authority().map(|authority| authority.agent.working_dir.clone());
        let card = self.card(&agent.parent_tool_use_id);
        let existing = manager.get_session_task(&self.session_id, &agent.agent_id).ok()
            .filter(|task| task.tool_name == NATIVE_AGENT_TASK_TOOL);
        let started = match existing {
            Some(task) if !task.status.is_terminal() => {
                self.state().tasks.insert(agent.agent_id.clone(), task.task_id.clone());
                return;
            }
            Some(task) => manager.restart_task_attempt(&self.session_id, &task.task_id, working_dir.as_deref())
                .map(|(snapshot, cancel)| (snapshot.task_id, cancel)),
            None => {
                let start = manager.create_task_in_workspace(&self.session_id, NATIVE_AGENT_TASK_TOOL, true, working_dir.as_deref());
                let description = if agent.description.trim().is_empty() { agent.name.as_str() } else { agent.description.as_str() };
                manager.prepare_named_task(&start.task_id, Some(description), Some(&agent.agent_id))
                    .and_then(|_| match &card {
                        Some(card) => manager.bind_origin(&start.task_id, &card.message_id, &agent.parent_tool_use_id),
                        None => Ok(()),
                    })
                    .map(|_| (start.task_id, start.cancel_rx))
            }
        };
        let (task_id, mut cancel) = match started {
            Ok(started) => started,
            Err(error) => {
                eprintln!("[Claude subagent] background agent {} has no task entry: {error}", agent.agent_id);
                return;
            }
        };
        manager.mark_running(&task_id, "Running");
        // A resumed run reports on the card of the Agent call that first
        // started it; the earlier results stay above the new one.
        let earlier = (agent.invocation_tool_use_id != agent.parent_tool_use_id)
            .then(|| card.as_ref().and_then(|card| self.card_display(card, &agent.parent_tool_use_id)))
            .flatten();
        {
            let mut state = self.state();
            match earlier {
                Some(earlier) => state.earlier_output.insert(task_id.clone(), earlier),
                None => state.earlier_output.remove(&task_id),
            };
            state.tasks.insert(agent.agent_id.clone(), task_id);
        }
        // The panel's stop (and "stop all") cancels the task: stop the agent.
        let session_id = self.session_id.clone();
        let agent_id = agent.agent_id.clone();
        tauri::async_runtime::spawn(async move {
            if cancel.wait_for(|cancelled| *cancelled).await.is_ok() {
                if let Err(error) = crate::llm::claude_code_cli::tui::stop_native_agent(&session_id, &agent_id) {
                    eprintln!("[Claude subagent] could not stop {agent_id}: {error}");
                }
            }
        });
    }

    fn task_for(&self, agent_id: &str) -> Option<crate::async_tasks::AsyncTaskSnapshot> {
        let manager = self.task_manager();
        let known = self.state().tasks.get(agent_id).cloned();
        known.and_then(|task_id| manager.snapshot(&task_id))
            .or_else(|| manager.get_session_task(&self.session_id, agent_id).ok())
            .filter(|task| task.tool_name == NATIVE_AGENT_TASK_TOOL)
    }

    /// The card's result so far: its recorded display (an earlier background
    /// run's outcome), else the Agent call's own result row.
    fn card_display(&self, card: &Card, card_id: &str) -> Option<String> {
        let messages = self.store.get_messages(&self.session_id).ok()?;
        let recorded = messages
            .iter()
            .find(|message| message.id == card.message_id)
            .and_then(|message| message.tool_calls.as_ref()?.iter().find(|call| call.id == card_id)?.recorded_output.clone());
        recorded
            .or_else(|| {
                messages
                    .iter()
                    .find(|message| message.role == MessageRole::Tool && message.tool_call_id.as_deref() == Some(card_id))
                    .map(|message| message.content.clone())
            })
            .filter(|output| !output.trim().is_empty())
    }

    /// Updates the card with the run's outcome, as for other async tools. A
    /// resumed run's result is appended to the card's earlier results.
    fn show_outcome(&self, task_id: &str) {
        let manager = self.task_manager();
        let Some(mut snapshot) = manager.snapshot(task_id) else { return; };
        let (Some(message_id), Some(tool_call_id)) = (snapshot.assistant_message_id.clone(), snapshot.tool_call_id.clone()) else { return; };
        let earlier = self.state().earlier_output.get(task_id).cloned();
        if let Some(earlier) = earlier {
            let output = snapshot.output.as_deref().unwrap_or_default();
            snapshot.output = Some(card_with_continuation(&earlier, output));
        }
        crate::async_tasks::emit_task_updated(&self.app, &message_id, &tool_call_id, &snapshot);
        if snapshot.status.is_terminal() {
            self.state().earlier_output.remove(task_id);
            let outcome = match snapshot.status {
                crate::async_tasks::AsyncTaskStatus::Cancelled => ToolCallOutcome::Interrupted,
                crate::async_tasks::AsyncTaskStatus::Failed => ToolCallOutcome::Error,
                _ => ToolCallOutcome::Done,
            };
            if let Err(error) = self.store.update_background_tool_display(&message_id, &tool_call_id, snapshot.output.as_deref().unwrap_or_default(), outcome) {
                eprintln!("[Claude subagent] failed to update the Agent card: {error}");
            }
        }
    }

    /// Claude continued the agent before Locus saw its earlier run's result:
    /// that result reaches the main agent like the agent's messages, and the
    /// task stays with the newer run.
    fn earlier_run_finished(&self, notice: &ClaudeCodeAgentNotice) {
        let Some(result) = notice.result.as_deref().filter(|text| !text.trim().is_empty()) else { return; };
        if notice.status != "completed" { return; }
        match self.store.queue_agent_message(&self.session_id, &self.session_id, &notice.agent_id, result, None) {
            Ok(message_id) => self.task_manager().ensure_parent_message_delivery(self.app.clone(), self.session_id.clone(), message_id),
            Err(error) => eprintln!("[Claude subagent] the earlier result of {} was not delivered: {error}", notice.agent_id),
        }
    }

    fn finish_run(&self, notice: &ClaudeCodeAgentNotice) {
        let Some(task) = self.task_for(&notice.agent_id) else { return; };
        if task.status.is_terminal() {
            return;
        }
        let manager = self.task_manager();
        match notice.status.as_str() {
            "completed" | "failed" => {
                let output = notice.result.clone().filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| notice.summary.clone());
                manager.finish(&task.task_id, &crate::tool::ToolResult { output, is_error: notice.status == "failed" });
            }
            _ => {
                // The user's stop (the panel cancelled the task) is reported
                // on the next turn without waking the conversation. The
                // model's own TaskStop needs no report.
                let by_user = task.status == crate::async_tasks::AsyncTaskStatus::Cancelling;
                if !by_user {
                    manager.suppress_notification(&task.task_id);
                }
                if let Some(snapshot) = manager.mark_cancelled_without_notification(&task.task_id) {
                    manager.enqueue_completion_notification(&snapshot);
                }
            }
        }
        self.show_outcome(&task.task_id);
    }
}

impl ClaudeNativeAgentHost for NativeAgentHub {
    fn registered<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move {
            // Once per run: the card links its record (new, or created by
            // content that overtook this start).
            if let Some((record, _)) = self.records.run_started(agent, &self.record_scope()) {
                self.announce_card(agent, None, Some(record));
            }
            if agent.background {
                self.start_task(agent);
            }
        })
    }

    fn message(&self, agent: &ClaudeCodeNativeAgent, thinking: &str, text: &str) {
        self.records.message(agent, thinking, text, &self.record_scope());
    }

    fn tool_started(&self, agent: &ClaudeCodeNativeAgent, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value) {
        self.record_started(agent, tool_call_id, tool_name, serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string()));
    }

    fn text(&self, agent: &ClaudeCodeNativeAgent, text: &str) {
        if agent.background {
            // The panel shows a background agent's latest progress.
            let line = text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default();
            if let Some(task_id) = self.state().tasks.get(&agent.agent_id).cloned() {
                self.task_manager().report_progress(&task_id, line.chars().take(200).collect::<String>());
            }
            return;
        }
        // Live progress on the Agent card; its result replaces it.
        let run_id = self.state().cards.get(&agent.parent_tool_use_id).map(|card| card.run_id.clone()).unwrap_or_default();
        emit_parent_stream(
            &self.app,
            self.parent(&agent.parent_tool_use_id, &run_id).tool_call_delta(format!("{}\n\n", text.trim_end())),
        );
    }

    fn execute<'a>(
        &'a self,
        agent: &'a ClaudeCodeNativeAgent,
        tool_call_id: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            if tool_name == "SendMessage" {
                let result = self.message_main(agent, &arguments);
                let recorded = ExecutedToolResult::from_tool_result(crate::tool::ToolResult {
                    output: result.output.clone(),
                    is_error: result.is_error,
                });
                self.record_finished(agent, tool_call_id, &recorded, None);
                return result;
            }
            self.execute_call(agent, tool_call_id, tool_name, arguments).await
        })
    }

    fn tool_finished<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent, tool_call_id: &'a str, result: ClaudeCodeToolResult) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move {
            let name = {
                let state = self.state();
                let key = if agent.background { format!("background:{}", agent.parent_tool_use_id) } else { agent.parent_tool_use_id.clone() };
                state.records.get(&key).and_then(|calls| calls.iter().find(|call| call.id == tool_call_id)).map(|call| call.name.clone())
            };
            let Some(name) = name else { return; };
            let output = if result.is_error {
                result.text_with_context()
            } else {
                claude_code_cli::native_output::external(&name, &result.output)
            };
            let executed = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output, is_error: result.is_error });
            self.record_finished(agent, tool_call_id, &executed, result.images.as_deref());
        })
    }

    fn notification<'a>(&'a self, notice: &'a ClaudeCodeAgentNotice) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move {
            if notice.superseded {
                self.earlier_run_finished(notice);
            } else {
                self.finish_run(notice);
            }
        })
    }

    fn usage(&self, agent: &ClaudeCodeNativeAgent, usage: &serde_json::Value) {
        let key = usage["stepKey"].as_str().map(str::to_owned);
        if let Some(key) = &key {
            if !self.state().usage.insert(format!("{}:{key}", agent.agent_id)) {
                return;
            }
        }
        let count = |name: &str| usage[name].as_u64().unwrap_or(0);
        let model = usage["model"].as_str().filter(|value| !value.trim().is_empty()).unwrap_or("subagent");
        // Billing counters only: a subagent's context is not the parent's.
        let recorded = self.store.record_model_usage_best_effort(crate::session::store::ModelUsageRecord {
            session_id: self.session_id.clone(),
            model_id: format!("claude_code/{model}"),
            provider: "Claude Code CLI".into(),
            request_kind: "subagent".into(),
            input_tokens: count("input_tokens"),
            output_tokens: count("output_tokens"),
            model_active_duration_ms: 0,
            cache_read_tokens: count("cache_read_input_tokens"),
            cache_write_tokens: count("cache_creation_input_tokens"),
            cost_usd: 0.0,
            priced_rounds: 0,
            context_tokens: None,
            context_limit: None,
            cache_request: None,
        });
        if recorded.is_none() {
            eprintln!("[Claude subagent] usage of {} was not recorded", agent.agent_id);
        }
    }

    fn model_resolved(&self, agent: &ClaudeCodeNativeAgent, model: &str) {
        // The registry already keeps it for history (`get_claude_agent_cards`).
        self.announce_card(agent, Some(model.to_string()), None);
    }

    fn process_exited<'a>(&'a self, agents: Vec<ClaudeCodeNativeAgent>) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move {
            let manager = self.task_manager();
            for agent in agents {
                let Some(task) = self.task_for(&agent.agent_id) else { continue; };
                if task.status.is_terminal() { continue; }
                // Reported on the next turn; never wakes the conversation.
                manager.append_output(&task.task_id, "Claude Code exited, so this background agent stopped. Continue it with SendMessage if its work is still needed.");
                if let Some(snapshot) = manager.mark_cancelled_without_notification(&task.task_id) {
                    manager.enqueue_completion_notification(&snapshot);
                }
                self.show_outcome(&task.task_id);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::card_with_continuation;

    #[test]
    fn a_continued_agent_card_keeps_every_run_result_in_order() {
        let first = card_with_continuation("First run.\n", "\nSecond run.");
        assert_eq!(first, "First run.\n\n---\n\nSecond run.");
        assert_eq!(card_with_continuation(&first, "Third run."), "First run.\n\n---\n\nSecond run.\n\n---\n\nThird run.");
        // A run without output (stopped before answering) leaves the card as it was.
        assert_eq!(card_with_continuation("First run.", "  "), "First run.");
    }
}
