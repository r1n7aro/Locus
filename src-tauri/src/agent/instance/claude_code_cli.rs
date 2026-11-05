use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::StreamExt;
use tauri::{AppHandle, Emitter};

use super::{
    emit_parent_stream, emit_stream, finalize_tool_call_record, normalize_tool_args, AgentInstance,
    AssistantStreamState, ExecutedToolResult, StreamRenderOrderTracker,
};
use crate::agent::workspace_execution_lock::{
    process_workspace_execution_lock, WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError,
    WorkspaceExecutionLockOwner, WorkspaceExecutionLockRequest,
};
use crate::commands::{StreamEvent, ToolCallOutcome};
use crate::llm::claude_code_cli::{
    self, ClaudeCodeAssistantMessage, ClaudeCodeCliOptions, ClaudeCodeHost, ClaudeCodeHostFuture,
    ClaudeCodeToolDefinition, ClaudeCodeToolResult,
};
use crate::session::models::{ChatMessage, ImageData, MessageRole, ToolCallInfo};
use crate::session::store::SessionStore;

const CLAUDE_CODE_CLI_PROVIDER: &str = "claude_code";

#[cfg(test)]
#[path = "claude_fork_sync_tests.rs"]
mod fork_sync_tests;
#[path = "claude_native_agent_host.rs"]
mod native_agent_host;
#[path = "claude_native_agent_hub.rs"]
mod native_agent_hub;

pub(super) fn pending_input_metadata() -> serde_json::Value {
    serde_json::json!({
        "backend": "claude_code_cli",
        "claude_pending_input": {"version": 1, "token": uuid::Uuid::new_v4().to_string(), "carrier": null},
    })
}

pub(super) fn pending_claude_resume_message(store: &SessionStore, session_id: &str) -> Result<Option<ChatMessage>, String> {
    let messages = store.get_messages(session_id)?;
    let metadata = store.get_response_request_metadata(session_id)?;
    let pending = store.claude_recovery_pending_message(session_id, &messages, &metadata)?;
    Ok(messages.into_iter().find(|message| Some(message.id.as_str()) == pending.as_deref()))
}

pub(super) fn persist_turn_user_message(
    store: &SessionStore, session_id: &str, resume_claude: bool,
    create: impl FnOnce() -> Result<String, String>,
) -> Result<(String, bool), String> {
    if resume_claude {
        if let Some(message) = pending_claude_resume_message(store, session_id)? {
            return Ok((message.id, true));
        }
    }
    create().map(|id| (id, false))
}

fn duplicates_system_reminder(prefix: &str, system_prompt: &str) -> bool {
    prefix.trim().strip_prefix("<system-reminder>")
        .and_then(|text| text.strip_suffix("</system-reminder>"))
        .map(str::trim).is_some_and(|text| !text.is_empty() && system_prompt.contains(text))
}

pub(super) fn claude_user_prompt(message: &ChatMessage, continuation: Option<&str>, system_prompt: &str) -> String {
    let prefix = message.prompt_prefix.as_deref().filter(|text| !duplicates_system_reminder(text, system_prompt));
    let mut prompt = crate::session::history::render_prompt_content(
        &message.content, prefix, message.prompt_suffix.as_deref(),
    );
    if let Some(continuation) = continuation.filter(|text| !text.is_empty()
        && message.prompt_suffix.as_deref().is_none_or(|suffix| suffix.trim() != text.trim())) {
        if !prompt.is_empty() { prompt.push_str("\n\n"); }
        prompt.push_str(continuation);
    }
    prompt
}

fn ensure_claude_history_origin(store: &SessionStore, session_id: &str, pending: Option<&str>) -> Result<(), String> {
    let messages = store.get_messages(session_id)?;
    let metadata = store.get_response_request_metadata(session_id)?;
    let pending = store.claude_recovery_pending_messages(session_id,&messages,&metadata,pending)?;
    store.verify_claude_history_origin(session_id, &messages, &metadata, &pending)
}

#[cfg(test)]
fn prepare_pending_claude_input(
    store: &SessionStore, session_id: &str, current: Option<&str>, prompt: &str,
    remote: &Option<claude_code_cli::ssh::SshConfig>, system_prompt: &str,
) -> Result<serde_json::Value, String> {
    prepare_pending_claude_input_with_model(store, session_id, current, prompt, &remote.clone().map(Into::into), system_prompt, None)
}

fn prepare_pending_claude_input_with_model(
    store: &SessionStore, session_id: &str, current: Option<&str>, prompt: &str,
    remote: &Option<claude_code_cli::connection::NativeConnection>, system_prompt: &str, model: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut messages = store.get_messages(session_id)?;
    let mut metadata = store.get_response_request_metadata(session_id)?;
    let mut pending = store.claude_recovery_pending_messages(session_id,&messages,&metadata,current)?;
    if pending.is_empty() {
        // Absence from the pending tail is not an ACK: a concurrent repair may
        // have removed the row. Only a verified native checkpoint permits a
        // continuation in place of the user's explicitly submitted input.
        let previous = messages.iter().find(|m|Some(m.id.as_str())==current && m.role==MessageRole::User)
            .ok_or("The current Claude input is missing; it was not replaced with a continuation")?;
        let checkpoint = metadata.get(&previous.id).and_then(|value|value.get("claude_checkpoint"))
            .ok_or("The current Claude input has no native acknowledgement; it was not replaced with a continuation")?;
        claude_code_cli::recovery::validate_input_checkpoint(checkpoint)?;
        if claude_code_cli::recovery::native_session_id(store, session_id)?.as_deref() != checkpoint["sessionId"].as_str() {
            return Err("The current Claude input belongs to another native conversation; it was not replaced with a continuation".into());
        }
        let rendered = claude_user_prompt(previous,None,system_prompt);
        let continuation = prompt.strip_prefix(&rendered).unwrap_or("").trim();
        let continuation = if continuation.is_empty() { "Continue from the existing conversation context and finish the current request. Do not repeat completed work or tool operations. Use the recorded results and inspect current state before any new action." } else { continuation };
        let continuation = claude_code_cli::recovery::continuation_prompt(continuation);
        let receipt = pending_input_metadata();
        let id = store.add_message_with_images_asset_refs_signature_and_response(
            session_id, MessageRole::User, "", None, None, None, None, Some(&continuation),
            Some(&receipt),
        )?;
        metadata.insert(id.clone(), receipt);
        pending.push(id);
    }
    let mut tokens = Vec::new();
    for id in &pending {
        let receipt = metadata.get(id).and_then(|v|v.get("claude_pending_input"));
        let token = match receipt {
            Some(value) if value["version"]==1 => value["token"].as_str().ok_or("Invalid Claude input receipt")?.to_string(),
            Some(_) => return Err("Unsupported Claude input receipt version".into()),
            None => uuid::Uuid::new_v4().to_string(),
        };
        uuid::Uuid::parse_str(&token).map_err(|_|"Invalid Claude input receipt token")?;
        tokens.push(token);
    }
    let last = pending.len()-1;
    let notifications = store.bind_claude_notifications(session_id, &pending[last])?;
    messages = store.get_messages(session_id)?;
    let mut earlier = Vec::new();
    let mut images = Vec::new();
    let mut final_prompt = String::new();
    for (index,id) in pending.iter().enumerate() {
        let message = messages.iter().find(|m|&m.id==id).ok_or("Pending Claude input disappeared")?;
        let mut value = metadata.remove(id).unwrap_or_else(||serde_json::json!({}));
        value["backend"] = serde_json::json!("claude_code_cli");
        value["claude_connection"] = serde_json::json!(remote);
        if let Some(model) = model { value["model"] = serde_json::json!(model); }
        value["claude_pending_input"] = serde_json::json!({"version":1,"token":tokens[index],
            "carrier":if index<last {Some(&tokens[last])} else {None}});
        store.set_message_response_request_metadata(session_id,id,&value)?;
        let mut text = if index==last && Some(id.as_str())==current {prompt.to_string()} else {claude_user_prompt(message,None,system_prompt)};
        if index == last {
            for reminder in &notifications {
                if !text.contains(reminder) { text.push_str("\n\n"); text.push_str(reminder); }
            }
        }
        let text = claude_code_cli::recovery::pending_input_text(&tokens[index],&text);
        if index==last { final_prompt=text; }
        else { earlier.push(serde_json::json!({"id":id,"token":tokens[index],"text":text})); }
        images.extend(message.images.iter().flatten().cloned());
    }
    let mut user = build_cli_user_message(&final_prompt,Some(&images));
    user["locusMessageId"] = serde_json::json!(pending[last]);
    user["locusPendingUsers"] = serde_json::json!(earlier);
    Ok(user)
}

fn claude_response_duration_ms(timing: &serde_json::Value) -> u64 {
    if timing["version"] != 1 || timing["source"] != "claude-turn-step" { return 0; }
    let Some(start) = timing["requestStartedAtMs"].as_u64() else { return 0; };
    let Some(stop) = timing["responseStoppedAtMs"].as_u64() else { return 0; };
    let Some(ready) = timing["resultReadyAtMs"].as_u64() else { return 0; };
    if ready < stop { return 0; }
    stop.checked_sub(start).unwrap_or(0)
}

struct ClaudeQueuedInputs<'a> {
    steering: super::codex_steering::RunSteering<'a>,
    agent: &'a AgentInstance,
    app: &'a AppHandle,
    initial_skills: &'a HashSet<String>,
}
impl claude_code_cli::ClaudeCodeInputSource for ClaudeQueuedInputs<'_> {
    fn claim(&self)->Result<Option<claude_code_cli::ClaudeCodeInput>,String>{
        let Some(input)=self.steering.claim_for_claude()? else{return Ok(None);};
        let mut content=Vec::new();
        for block in input.input[0]["content"].as_array().ok_or("Invalid pending user input")? {
            match block["type"].as_str() {
                Some("input_text")=>content.push(serde_json::json!({"type":"text","text":claude_code_cli::recovery::pending_input_text(&input.id,block["text"].as_str().ok_or("Invalid pending text")?)})),
                Some("input_image")=>{
                    let url=block["image_url"].as_str().ok_or("Invalid pending image")?;
                    let (mime,data)=url.strip_prefix("data:").and_then(|v|v.split_once(";base64,")).ok_or("Pending image is not embedded data")?;
                    content.push(serde_json::json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}));
                }
                _=>return Err("Unsupported pending Claude attachment".into()),
            }
        }
        Ok(Some(claude_code_cli::ClaudeCodeInput{id:input.id,message:serde_json::json!({"role":"user","content":content})}))
    }
    fn accepted(&self,id:&str,checkpoint:&serde_json::Value)->Result<(),String>{
        self.steering.committed_claude(id,checkpoint)
    }
    fn refresh_tools(&self)->std::pin::Pin<Box<dyn std::future::Future<Output=Result<Option<Vec<claude_code_cli::ClaudeCodeToolDefinition>>,String>>+Send+'_>> {
        Box::pin(async {
            self.agent.prepare_active_skill_package_unity_runtimes_if_connected().await;
            let active=self.agent.active_skill_tool_names(self.initial_skills);
            let tools=self.agent.build_cli_api_tools(self.agent.dynamic_tool_loading_mode(self.app),&active).await;
            Ok(Some(convert_api_tools_to_claude_code(&tools)))
        })
    }
}

#[derive(Default)]
struct PendingAssistantRound {
    message_id: String,
    text: String,
    tool_calls: Vec<ToolCallInfo>,
    remaining: HashSet<String>,
    content_order: Option<u32>,
    thinking_order: Option<u32>,
    thinking_text: String,
    thinking_signature: String,
    undo_checked: bool,
    undo_guard: Option<crate::vcs::undo::UndoRoundGuard>,
    has_unity_execute: bool,
    unity_edit_session_started: bool,
    queued_unity_asset_paths: Vec<String>,
    workspace_guard: Option<WorkspaceExecutionGuard>,
    confirmations_prepared: bool,
    confirmation_preapproved: HashSet<String>,
    confirmation_rejections: HashMap<String, ExecutedToolResult>,
    subagent_phase_prepared: bool,
    precompleted_subagent_results: HashMap<String, ExecutedToolResult>,
}

impl PendingAssistantRound {
    fn activate(pending: &mut Option<Self>, waiting: &mut Vec<Self>, id: &str) {
        if pending.as_ref().is_some_and(|round| round.tool_calls.iter().any(|call| call.id == id)) { return; }
        if let Some(index) = waiting.iter().position(|round| round.tool_calls.iter().any(|call| call.id == id)) {
            let selected = waiting.remove(index);
            if let Some(previous) = pending.replace(selected) { waiting.push(previous); }
        }
    }
}

struct CliRoundCompletion {
    message_id: String,
    undo_guard: Option<crate::vcs::undo::UndoRoundGuard>,
    has_unity_execute: bool,
    queued_unity_asset_paths: Vec<String>,
    workspace_guard: Option<WorkspaceExecutionGuard>,
}

struct ClaudeCodeRoundHost<'a> {
    agent: &'a AgentInstance,
    app_handle: &'a AppHandle,
    store: &'a SessionStore,
    run_id: &'a str,
    input_message_id: Option<String>,
    cache_message_id: Option<String>,
    cache_request: Option<crate::session::store::PromptCacheRequest>,
    mode: &'a str,
    active_skill_tool_names: &'a HashSet<String>,
    streamed_text: String,
    partial_assistant: Arc<AssistantStreamState>,
    started_tool_calls: VecDeque<(String, String)>,
    known_tool_calls: VecDeque<ToolCallInfo>,
    completed_tool_ids: HashSet<String>,
    pending_round: Option<PendingAssistantRound>,
    waiting_rounds: Vec<PendingAssistantRound>,
    last_assistant: Option<ClaudeCodeAssistantMessage>,
    last_persisted_assistant_message_id: Option<String>,
    cli_session_id: Option<String>,
    response_metadata: serde_json::Value,
    native_checkpoints: Vec<serde_json::Value>,
    step_message_ids: Vec<String>,
    render_order: StreamRenderOrderTracker,
    render_scope: String,
    retrying: bool,
}

// Validate the same output the native adapter will receive, before persisting
// or emitting a successful tool completion. A later Mod refusal cannot replace
// an already completed tool result (and must never cause the operation to rerun).
fn validate_native_tool_result(
    tool_call_id: &str,
    result: &mut ExecutedToolResult,
    stored_output: &mut String,
) {
    if result.outcome != super::ToolRunOutcome::Done {
        return;
    }
    let Some(scope) = claude_code_cli::native_scope::current()
        .filter(|scope| scope.call.id == tool_call_id)
    else {
        return;
    };
    let reply = claude_code_cli::native_tools::finish_result(
        &scope.call,
        &ClaudeCodeToolResult {
            output: stored_output.clone(),
            is_error: result.is_error,
            images: result.images.clone(),
            context: Vec::new(),
        },
        scope.result(),
    );
    if let Some(error) = reply["deny"].as_str() {
        *stored_output = error.to_string();
        result.output = error.to_string();
        result.is_error = true;
        result.outcome = super::ToolRunOutcome::Error;
        result.images = None;
    }
}

fn bound_native_tool_result(tool_call_id: &str, stored_output: &str) {
    let Some(scope) = claude_code_cli::native_scope::current()
        .filter(|scope| scope.call.id == tool_call_id) else { return; };
    if let Some(mut value) = scope.result() {
        claude_code_cli::native_output::apply_persisted_output(&scope.call.tool, &mut value, stored_output);
        scope.set_result(value);
    }
}

fn save_claude_code_tool_result(
    store: &SessionStore,
    session_id: &str,
    run_id: &str,
    tool_call_id: &str,
    stored_output: &str,
    images: Option<&[ImageData]>,
) -> Result<Option<String>, String> {
    store.add_tool_result_with_images_for_run(
        session_id,
        run_id,
        tool_call_id,
        stored_output,
        images,
    )
}

/// Retain producer provenance for error/cancellation snapshots, including when
/// the native host future is dropped. A stream is not a committed native row:
/// never copy a prior checkpoint, pending-input receipt or usage attribution.
fn track_claude_stream_origin(
    stream: &AssistantStreamState,
    metadata: &serde_json::Value,
    run_id: &str,
) {
    let mut origin = serde_json::json!({"backend":"claude_code_cli", "claude_run_id":run_id});
    for key in ["model", "upstream_model", "claude_connection", "claude_native_tool_search"] {
        if let Some(value) = metadata.get(key) { origin[key] = value.clone(); }
    }
    stream.set_response_metadata(origin);
}

impl<'a> ClaudeCodeRoundHost<'a> {
    fn select_tool_round(&mut self, id: &str) {
        PendingAssistantRound::activate(&mut self.pending_round, &mut self.waiting_rounds, id);
    }

    /// Summarizer instructions, a local transcript copy and the carry-forward
    /// block, captured while the conversation being compacted is still intact.
    async fn compaction_preparation(&self) -> Result<serde_json::Value, String> {
        let session_id = self.agent.session_id.as_str();
        let (transcript_path, copy_error) = match self.save_compaction_transcript().await {
            Ok(path) => (path, None),
            Err(error) => {
                eprintln!("[Agent {}] Claude transcript copy failed for session {}: {}", self.agent.id, session_id, error);
                (None, Some(error))
            }
        };
        // A local Claude names a path Locus can read; a remote one cannot.
        let remote = claude_code_cli::recovery::recorded_remote(self.store, session_id)?.is_some();
        let transcript_error = (transcript_path.is_none() && remote).then(|| copy_error
            .unwrap_or_else(|| "The native transcript was not available in the local archive.".to_string()));
        let display = self.store.get_messages_for_display(session_id)?;
        let context = self.store.get_messages_for_prompt(session_id)?;
        let todos = self.store.get_todos(session_id)?.items;
        let model_window = self.store.get_token_usage(session_id)?.context_limit;
        let configured = claude_code_cli::ssh::load_settings().ok()
            .and_then(|settings| settings.runtime.auto_compact_window);
        let compaction_window = match (configured, model_window) {
            (Some(configured), 0) => configured,
            (Some(configured), model) => configured.min(model),
            (None, model) => model,
        };
        let carry_forward = crate::compact::claude_native::build_carry_forward(
            &crate::compact::claude_native::CarryForwardInput {
                display_messages: &display,
                context_messages: &context,
                todos: &todos,
                working_dir: &self.agent.working_dir,
                transcript_path: transcript_path.as_deref(),
                transcript_unreachable: transcript_error.is_some(),
                compaction_window,
            },
        );
        Ok(serde_json::json!({
            "ok": true,
            "instructions": crate::compact::claude_native::SUMMARY_INSTRUCTIONS,
            "transcriptPath": transcript_path,
            "transcriptError": transcript_error,
            "carryForward": carry_forward,
        }))
    }

    /// Copy the native transcript to this machine. In SSH and server mode the
    /// path Claude names is on the Claude host, unreachable for Locus tools.
    async fn save_compaction_transcript(&self) -> Result<Option<String>, String> {
        let id = self.agent.session_id.as_str();
        let Some(native_id) = claude_code_cli::recovery::native_session_id(self.store, id)? else {
            return Ok(None);
        };
        let Some(target) = crate::compact::claude_native::transcript_copy_path(id, &native_id) else {
            return Ok(None);
        };
        let bytes = if claude_code_cli::recovery::recorded_remote(self.store, id)?.is_some() {
            // The archive follows every checkpoint; a slightly older copy is
            // still the right fallback when the refresh is slow or fails.
            match tokio::time::timeout(
                std::time::Duration::from_secs(20),
                claude_code_cli::archive::refresh_for_compaction(self.store, id),
            ).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("[Agent {}] Claude archive refresh before compaction failed: {}", self.agent.id, error),
                Err(_) => eprintln!("[Agent {}] Claude archive refresh before compaction timed out", self.agent.id),
            }
            self.store.claude_archive_files(id)?.remove("transcript.jsonl")
        } else {
            match claude_code_cli::tui::local_native_transcript(id, &native_id) {
                Some(path) => Some(tokio::fs::read(&path).await.map_err(|e| format!("Failed to read native transcript: {e}"))?),
                None => None,
            }
        };
        let Some(bytes) = bytes.filter(|bytes| !bytes.is_empty()) else {
            return Ok(None);
        };
        let parent = target.parent().ok_or("Invalid transcript copy path")?;
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("Failed to create transcript directory: {e}"))?;
        let temporary = target.with_extension("jsonl.partial");
        tokio::fs::write(&temporary, &bytes).await.map_err(|e| format!("Failed to write transcript copy: {e}"))?;
        tokio::fs::rename(&temporary, &target).await.map_err(|e| format!("Failed to install transcript copy: {e}"))?;
        Ok(Some(target.to_string_lossy().replace('\\', "/")))
    }

    fn save_assistant_message(&mut self, message: ClaudeCodeAssistantMessage, force_persist: bool) -> Result<(), String> {
        if message.tool_calls.is_empty() && !force_persist {
            self.last_assistant = Some(message);
            return Ok(());
        }

        if let Some(existing) = self.pending_round.take() {
            // Native searches can finish out of order. Keep each envelope's
            // persistence/undo/confirmation state until its own result arrives.
            if !existing.remaining.is_empty() { self.waiting_rounds.push(existing); }
        }
        let text_part = (!message.text.is_empty()).then(|| {
            self.render_order
                .mark_text(&self.render_scope, "claude-code-round-text")
        });
        let thinking_part = (!message.thinking_text.is_empty()).then(|| {
            self.render_order
                .mark_thinking(&self.render_scope, "claude-code-round-thinking")
        });
        let content_order = text_part.as_ref().map(|part| part.seq);
        let thinking_order = thinking_part.as_ref().map(|part| part.seq);
        let ordered_tool_calls = self
            .render_order
            .assign_tool_orders_for_run(self.run_id, &message.tool_calls);
        let render_parts = super::assistant_render_parts_for_response(
            self.run_id,
            text_part,
            &message.text,
            thinking_part,
            &message.thinking_text,
            None,
            (!message.thinking_signature.is_empty()).then_some(message.thinking_signature.as_str()),
            &[],
            &ordered_tool_calls,
        );

        for tool_call in &ordered_tool_calls {
            self.emit_tool_call_start(&tool_call.id, &tool_call.name, &tool_call.arguments);
            self.started_tool_calls
                .retain(|(id, _)| id != &tool_call.id);
            if !self.completed_tool_ids.contains(&tool_call.id)
                && !self.known_tool_calls.iter().any(|tc| tc.id == tool_call.id)
            {
                self.known_tool_calls.push_back(tool_call.clone());
            }
        }

        let thinking_text = if message.thinking_text.is_empty() {
            None
        } else {
            Some(message.thinking_text.as_str())
        };
        let thinking_signature = if message.thinking_signature.is_empty() {
            None
        } else {
            Some(message.thinking_signature.as_str())
        };
        let message_id = self.store.add_assistant_with_tool_calls_and_render_parts(
            &self.agent.session_id,
            &message.text,
            &ordered_tool_calls,
            thinking_text,
            None,
            thinking_signature,
            None,
            Some(&self.response_metadata),
            content_order,
            thinking_order,
            &render_parts,
        )?;
        self.step_message_ids.push(message_id.clone());
        if let Some(session_id)=self.cli_session_id.as_deref() {
            self.store.set_message_cli_session_id(&self.agent.session_id,&message_id,CLAUDE_CODE_CLI_PROVIDER,session_id)?;
        }
        self.partial_assistant.mark_persisted(
            message_id.clone(),
            message.text.clone(),
            thinking_text.map(str::to_string),
            None,
        );

        let remaining: HashSet<String> = ordered_tool_calls
            .iter()
            .filter(|tc| !self.completed_tool_ids.contains(&tc.id))
            .map(|tc| tc.id.clone())
            .collect();

        self.pending_round = Some(PendingAssistantRound {
            message_id: message_id.clone(),
            text: message.text,
            tool_calls: ordered_tool_calls,
            remaining,
            content_order,
            thinking_order,
            thinking_text: message.thinking_text,
            thinking_signature: message.thinking_signature,
            undo_checked: false,
            undo_guard: None,
            has_unity_execute: false,
            unity_edit_session_started: false,
            queued_unity_asset_paths: Vec::new(),
            workspace_guard: None,
            confirmations_prepared: false,
            confirmation_preapproved: HashSet::new(),
            confirmation_rejections: HashMap::new(),
            subagent_phase_prepared: false,
            precompleted_subagent_results: HashMap::new(),
        });
        self.last_persisted_assistant_message_id = Some(message_id);
        self.streamed_text.clear();
        self.maybe_finish_pending_round();
        self.flush_native_checkpoints()?;
        Ok(())
    }

    fn save_response_metadata(&self, id: &str) -> Result<(), String> {
        let mut value=self.response_metadata.clone();
        value["claude_run_id"]=serde_json::json!(self.run_id);
        value["claude_plan_active"]=serde_json::json!(self.agent.plan_mode_main_active());
        if let Some(saved)=self.store.get_response_request_metadata(&self.agent.session_id)?.get(id) {
            // Session startup updates model/run metadata before input is
            // accepted; it must not discard the stable delivery receipt.
            for key in ["claude_checkpoint","claude_pending_input"] {
                if let Some(saved)=saved.get(key) {value[key]=saved.clone();}
            }
        }
        self.store.set_message_response_request_metadata(&self.agent.session_id,id,&value)
    }
    fn flush_native_checkpoints(&mut self) -> Result<(),String> {
        let messages=self.store.get_messages(&self.agent.session_id)?;
        let metadata=self.store.get_response_request_metadata(&self.agent.session_id)?;
        let notifications=self.store.claude_notification_ids(&self.agent.session_id,&messages,&metadata)?;
        let current_user=messages.iter().rposition(|message|message.role==MessageRole::User && !notifications.contains(&message.id)).unwrap_or(0);
        let mut waiting=Vec::new();
        for event in std::mem::take(&mut self.native_checkpoints) {
            let tools=event["tools"].as_array().cloned().unwrap_or_default();
            let results=event["results"].as_array().cloned().unwrap_or_default();
            let found=messages.iter().enumerate().rev().find(|(index,message)| {
                if !results.is_empty() {return message.tool_call_id.as_ref().is_some_and(|id|results.iter().any(|v|v.as_str()==Some(id)));}
                if !tools.is_empty() {return message.tool_calls.as_ref().is_some_and(|calls|calls.iter().any(|call|tools.iter().any(|v|v.as_str()==Some(call.id.as_str()))));}
                if event["user"]==true {
                    if message.role!=MessageRole::User || notifications.contains(&message.id) { return false; }
                    if let Some(submission)=event["submissionId"].as_str() {
                        return metadata.get(&message.id).is_some_and(|value|value["claude_pending_input"]["token"]==submission);
                    }
                    return event["messageId"].as_str().map_or(*index==current_user,|id|message.id==id);
                }
                event["role"]=="assistant" && *index>current_user && message.role==MessageRole::Assistant && !message.content.is_empty() && message.content==event["text"].as_str().unwrap_or("")
            });
            if let Some((_,message))=found {
                if event["user"] == true { self.cache_message_id = Some(message.id.clone()); }
                let mut value=metadata.get(&message.id).cloned().unwrap_or_else(||self.response_metadata.clone());
                if value.get("claude_plan_active").is_none() {value["claude_plan_active"]=serde_json::json!(self.agent.plan_mode_main_active());}
                if value.get("claude_run_id").is_none() {value["claude_run_id"]=serde_json::json!(self.run_id);}
                value["claude_checkpoint"]=serde_json::json!({"version":1,"uuid":event["uuid"],"sessionId":event["sessionId"]});
                self.store.set_message_response_request_metadata(&self.agent.session_id,&message.id,&value)?;
            } else if event["user"]==true || event["role"]=="assistant" || !results.is_empty() {waiting.push(event);}
        }
        self.native_checkpoints=waiting;
        Ok(())
    }
    fn emit_tool_call_start(&mut self, tool_call_id: &str, tool_name: &str, arguments: &str) {
        let mark = self.render_order.mark_tool(self.run_id, tool_call_id);
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::ToolCallStart {
                session_id: self.agent.session_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
                order: Some(mark.seq),
                part_id: Some(tool_call_id.to_string()),
                render_seq: Some(mark.seq),
            },
        );
        if let Some(ref parent) = self.agent.parent_tool_call {
            emit_parent_stream(
                self.app_handle,
                parent.subagent_tool_call_start(
                    tool_call_id.to_string(),
                    tool_name.to_string(),
                    arguments.to_string(),
                    Some(mark.seq),
                    Some(mark.id),
                    Some(mark.seq),
                ),
            );
        }
    }

    fn emit_tool_call_done(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        output: &str,
        outcome: ToolCallOutcome,
        images: Option<&[ImageData]>,
    ) {
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::ToolCallDone {
                session_id: self.agent.session_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: output.to_string(),
                outcome,
                images: images.map(|items| items.to_vec()),
            },
        );
        if let Some(ref parent) = self.agent.parent_tool_call {
            let truncated_output = if output.chars().count() > 500 {
                let prefix: String = output.chars().take(500).collect();
                format!("{}... ({} chars)", prefix, output.chars().count())
            } else {
                output.to_string()
            };
            emit_parent_stream(
                self.app_handle,
                parent.subagent_tool_call_done(
                    tool_call_id.to_string(),
                    tool_name.to_string(),
                    truncated_output,
                    outcome,
                    images.map(|items| items.to_vec()),
                ),
            );
        }
    }

    fn maybe_finish_pending_round(&mut self) {
        let should_finish = self
            .pending_round
            .as_ref()
            .map(|round| round.remaining.is_empty())
            .unwrap_or(false);
        if !should_finish {
            return;
        }

        if let Some(round) = self.pending_round.take() {
            let render_parts = super::assistant_render_parts_for_response(
                self.run_id,
                round.content_order.map(|seq| super::RenderPartMark {
                    id: format!("{}:text:claude-code-round", self.render_scope),
                    seq,
                }),
                &round.text,
                round.thinking_order.map(|seq| super::RenderPartMark {
                    id: format!("{}:thinking:claude-code-round", self.render_scope),
                    seq,
                }),
                &round.thinking_text,
                None,
                (!round.thinking_signature.is_empty()).then_some(round.thinking_signature.as_str()),
                &[],
                &round.tool_calls,
            );
            if let Err(err) = self.store.update_message_tool_calls_and_render_parts(
                &round.message_id,
                &round.tool_calls,
                &render_parts,
            ) {
                eprintln!(
                    "[Agent {}] failed to update Claude Code CLI tool_calls/render_parts for message {}: {}",
                    self.agent.id, round.message_id, err
                );
            }
            emit_stream(
                self.app_handle,
                self.run_id,
                StreamEvent::ToolCallRoundDone {
                    session_id: self.agent.session_id.clone(),
                    message_id: round.message_id,
                    full_text: round.text,
                    tool_calls: round.tool_calls,
                    content_order: round.content_order,
                    thinking_order: round.thinking_order,
                    render_parts: Some(render_parts),
                },
            );
            self.partial_assistant.reset();
            track_claude_stream_origin(&self.partial_assistant, &self.response_metadata, self.run_id);
        }
    }

    fn take_matching_tool_call(
        &mut self,
        request_id: &str,
        tool_name: &str,
        raw_arguments: &serde_json::Value,
    ) -> ToolCallInfo {
        let raw_arguments_str =
            serde_json::to_string(raw_arguments).unwrap_or_else(|_| "{}".to_string());

        // Mods retain Claude's actual tool_use_id; prefer it over name/argument
        // matching, especially when two parallel calls have identical inputs.
        if let Some(index) = self
            .known_tool_calls
            .iter()
            .position(|tc| tc.id == request_id)
        {
            return self
                .known_tool_calls
                .remove(index)
                .expect("matched native tool call");
        }

        if let Some(index) = self.known_tool_calls.iter().position(|tc| {
            tc.name == tool_name && equivalent_json_args(&tc.arguments, &raw_arguments_str)
        }) {
            return self
                .known_tool_calls
                .remove(index)
                .unwrap_or_else(|| ToolCallInfo {
                    id: format!("claude_code_{}", sanitize_request_id(request_id)),
                    name: tool_name.to_string(),
                    arguments: raw_arguments_str.clone(),
                    server_tool: None,
                    server_tool_output: None,
                    server_tool_result: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                    order: None,
                });
        }

        if let Some(index) = self
            .known_tool_calls
            .iter()
            .position(|tc| tc.name == tool_name)
        {
            return self
                .known_tool_calls
                .remove(index)
                .unwrap_or_else(|| ToolCallInfo {
                    id: format!("claude_code_{}", sanitize_request_id(request_id)),
                    name: tool_name.to_string(),
                    arguments: raw_arguments_str.clone(),
                    server_tool: None,
                    server_tool_output: None,
                    server_tool_result: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                    order: None,
                });
        }

        if let Some(index) = self
            .started_tool_calls
            .iter()
            .position(|(_, started_name)| started_name == tool_name)
        {
            if let Some((tool_call_id, started_name)) = self.started_tool_calls.remove(index) {
                return ToolCallInfo {
                    id: tool_call_id,
                    name: started_name,
                    arguments: raw_arguments_str,
                    server_tool: None,
                    server_tool_output: None,
                    server_tool_result: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                    order: None,
                };
            }
        }

        ToolCallInfo {
            id: format!("claude_code_{}", sanitize_request_id(request_id)),
            name: tool_name.to_string(),
            arguments: raw_arguments_str,
            server_tool: None,
            server_tool_output: None,
            server_tool_result: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
            order: None,
        }
    }

    fn emit_undo_checkpoint_warning(&self, error: &str) {
        emit_undo_checkpoint_warning(self.app_handle, error);
    }

    fn cli_round_workspace_policy(
        &self,
        current_tool_name: &str,
        current_args: &serde_json::Value,
    ) -> Result<Option<(WorkspaceExecutionLockRequest, Vec<String>)>, String> {
        let mut tools = Vec::new();
        let mut request = None;
        if let Some(round) = self.pending_round.as_ref() {
            for tool_call in &round.tool_calls {
                let mut args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                    .unwrap_or_else(|_| serde_json::json!({}));
                normalize_tool_args(&mut args);
                let effective_name = self
                    .agent
                    .effective_tool_name_for_round(&tool_call.name, &args);
                request = AgentInstance::merge_workspace_execution_request(
                    request,
                    self.agent
                        .workspace_execution_request_for_tool(&tool_call.name, &args),
                );
                tools.push(effective_name);
            }
        }
        if tools.is_empty() {
            tools.push(current_tool_name.to_string());
            request = self
                .agent
                .workspace_execution_request_for_tool(current_tool_name, current_args);
        }

        let has_ask = tools.iter().any(|name| name == "ask_user_question");
        let has_external_mcp = tools
            .iter()
            .any(|name| name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX));
        if has_ask {
            if current_tool_name == "ask_user_question" {
                return Ok(None);
            }
            if AgentInstance::is_deterministic_pre_ask_tool(current_tool_name) {
                return Ok(None);
            }
            return Err(format!(
                "Tool '{}' was skipped by the Claude Code tool-round scheduler: user-input rounds only allow deterministic pre-ask tools. Retry it in the next tool round.",
                current_tool_name
            ));
        }

        let allowed_kind = if has_external_mcp
            && tools
                .iter()
                .any(|name| !name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX))
        {
            Some("external MCP")
        } else {
            None
        };
        if let Some(allowed_kind) = allowed_kind {
            let current_allowed = match allowed_kind {
                _ => current_tool_name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX),
            };
            if !current_allowed {
                return Err(format!(
                    "Tool '{}' was skipped by the Claude Code tool-round scheduler: {} calls must run without local sibling tools. Retry it in the next tool round.",
                    current_tool_name, allowed_kind
                ));
            }
            return Ok(None);
        }
        // execute_single_tool owns per-call path guards when snapshots are off.
        // Do not acquire a second guard or retain it until the CLI round ends.
        if !self.agent.session_undo_enabled || current_tool_name == "subagent" || has_external_mcp {
            return Ok(None);
        }

        if self
            .agent
            .workspace_execution_request_for_tool(current_tool_name, current_args)
            .is_none()
        {
            return Ok(None);
        }

        tools.retain(|name| name != "subagent");

        Ok(request.map(|request| (request, tools)))
    }

    async fn precomplete_cli_tool_calls(
        &mut self,
        prepared: Vec<(ToolCallInfo, serde_json::Value)>,
        assistant_message_id: &str,
        phase: &str,
    ) {
        if prepared.is_empty() {
            return;
        }
        eprintln!(
            "[Agent {}] Claude Code executing {} session={} run={} calls={}",
            self.agent.id,
            phase,
            self.agent.session_id,
            self.run_id,
            prepared.len()
        );
        let mut pending = futures::stream::FuturesUnordered::new();
        for (tool_call, args) in prepared {
            let agent = self.agent;
            let app_handle = self.app_handle;
            let store = self.store;
            let run_id = self.run_id;
            let mode = self.mode;
            let active_skill_tool_names = self.agent.active_skill_tool_names(self.active_skill_tool_names);
            pending.push(async move {
                let result = agent
                    .execute_single_tool(
                        app_handle,
                        store,
                        &tool_call,
                        &args,
                        run_id,
                        assistant_message_id,
                        mode,
                        &active_skill_tool_names,
                        false,
                    )
                    .await;
                (tool_call.id, result)
            });
        }

        let mut completed = HashMap::new();
        while let Some((tool_call_id, result)) = pending.next().await {
            completed.insert(tool_call_id, result);
        }
        if let Some(round) = self.pending_round.as_mut() {
            round.precompleted_subagent_results.extend(completed);
        }
    }

    async fn ensure_cli_foreground_subagent_phase(&mut self) {
        let (tool_calls, assistant_message_id) = match self.pending_round.as_ref() {
            Some(round) if !round.subagent_phase_prepared => {
                (round.tool_calls.clone(), round.message_id.clone())
            }
            _ => return,
        };

        let mut writable_subagents = Vec::new();
        let mut readonly_subagents = Vec::new();
        let mut readonly_local_siblings = Vec::new();
        let mut has_local_sibling = false;
        let mut has_ask = false;
        let mut has_external_mcp = false;
        for tool_call in &tool_calls {
            let mut args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                .unwrap_or_else(|_| serde_json::json!({}));
            normalize_tool_args(&mut args);
            self.agent.inject_working_dir(&tool_call.name, &mut args);
            let effective_name = self
                .agent
                .effective_tool_name_for_round(&tool_call.name, &args);
            has_ask |= effective_name == "ask_user_question";
            has_external_mcp |= effective_name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX);
            has_local_sibling |= effective_name != "subagent"
                && effective_name != "ask_user_question"
                && !effective_name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX);
            if effective_name == "subagent"
                && !self
                    .agent
                    .tool_call_runs_in_background(&effective_name, &args)
            {
                if self
                    .agent
                    .subagent_call_is_workspace_readonly(&tool_call.name, &args)
                {
                    readonly_subagents.push((tool_call.clone(), args));
                } else {
                    writable_subagents.push((tool_call.clone(), args));
                }
            } else if AgentInstance::is_deterministic_pre_ask_tool(&effective_name) {
                readonly_local_siblings.push((tool_call.clone(), args));
            }
        }

        if let Some(round) = self.pending_round.as_mut() {
            round.subagent_phase_prepared = true;
        }
        if has_ask || has_external_mcp {
            return;
        }

        if has_local_sibling {
            self.precomplete_cli_tool_calls(
                writable_subagents,
                &assistant_message_id,
                "writable foreground subagent phase before local siblings",
            )
            .await;
        }

        if !readonly_subagents.is_empty() && (has_local_sibling || readonly_subagents.len() > 1) {
            readonly_subagents.extend(readonly_local_siblings);
            self.precomplete_cli_tool_calls(
                readonly_subagents,
                &assistant_message_id,
                "read-only parallel phase",
            )
            .await;
        }
    }

    async fn ensure_cli_round_confirmations_prepared(&mut self) {
        let Some(round) = self.pending_round.as_mut() else {
            return;
        };
        if round.confirmations_prepared {
            return;
        }

        // Mark first because this method spans user-driven awaits. The host is
        // mutably borrowed while awaiting, so the round cannot execute a tool
        // concurrently, and the flag prevents accidental repeated prompts.
        round.confirmations_prepared = true;
        let tool_calls = round.tool_calls.clone();
        let round_has_ask = tool_calls.iter().any(|tool_call| {
            let mut args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                .unwrap_or_else(|_| serde_json::json!({}));
            normalize_tool_args(&mut args);
            self.agent
                .effective_tool_name_for_round(&tool_call.name, &args)
                == "ask_user_question"
        });
        let mut approved = HashSet::new();
        let mut rejected = HashMap::new();

        for tool_call in tool_calls {
            let mut args = match serde_json::from_str::<serde_json::Value>(&tool_call.arguments) {
                Ok(args) => args,
                Err(_) => continue,
            };
            normalize_tool_args(&mut args);
            let effective_name = self
                .agent
                .effective_tool_name_for_round(&tool_call.name, &args);
            if effective_name == "ask_user_question"
                || (round_has_ask && !AgentInstance::is_deterministic_pre_ask_tool(&effective_name))
            {
                continue;
            }
            if matches!(
                effective_name.as_str(),
                "tool_load" | super::CODEX_TOOL_SEARCH_TOOL_NAME | "exit_plan_mode"
            ) {
                continue;
            }
            let Some((confirm_name, confirm_arguments, confirm_args)) =
                self.agent.tool_round_confirmation_target(&tool_call, &args)
            else {
                continue;
            };
            let decision = self
                .agent
                .request_tool_confirm(
                    self.app_handle,
                    &tool_call.id,
                    &confirm_name,
                    &confirm_arguments,
                    &confirm_args,
                    self.run_id,
                )
                .await;
            if let Some(result) = self
                .agent
                .confirmation_rejection_result(&confirm_name, decision)
            {
                rejected.insert(tool_call.id, result);
            } else {
                approved.insert(tool_call.id);
            }
        }

        if let Some(round) = self.pending_round.as_mut() {
            eprintln!(
                "[Agent {}] Claude Code tool round confirmations prepared session={} run={} approved={} rejected={}",
                self.agent.id,
                self.agent.session_id,
                self.run_id,
                approved.len(),
                rejected.len()
            );
            round.confirmation_preapproved = approved;
            round.confirmation_rejections = rejected;
        }
    }

    async fn ensure_cli_round_undo_checkpoint(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        if !self.agent.should_track_session_undo(tool_name, args) {
            return;
        }

        let should_check = self
            .pending_round
            .as_ref()
            .map(|round| !round.undo_checked)
            .unwrap_or(false);
        if !should_check {
            return;
        }
        if let Some(round) = self.pending_round.as_mut() {
            round.undo_checked = true;
        }

        let Some(undo_mgr) = self.agent.undo_manager.as_ref() else {
            return;
        };
        let checkpoint = match undo_mgr
            .before_round(&self.agent.working_dir, "cli tool round")
            .await
        {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                eprintln!(
                    "[Agent {}] Claude Code CLI undo checkpoint failed: {}",
                    self.agent.id, error
                );
                self.emit_undo_checkpoint_warning(&error);
                None
            }
        };

        if let Some(round) = self.pending_round.as_mut() {
            round.undo_guard = checkpoint;
        }
    }

    async fn prepare_cli_unity_tool(&mut self, tool_call: &ToolCallInfo, args: &serde_json::Value) {
        if let Some(round) = self.pending_round.as_mut() {
            if AgentInstance::is_unity_execute_undo_call(&tool_call.name, args) {
                round.has_unity_execute = true;
            }
        }

        let queued_before_recompile = if tool_call.name == "unity_recompile" {
            self.pending_round
                .as_mut()
                .map(|round| std::mem::take(&mut round.queued_unity_asset_paths))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if !queued_before_recompile.is_empty() {
            match crate::unity_bridge::import_assets(
                &self.agent.working_dir,
                &queued_before_recompile,
            )
            .await
            {
                Ok(message) => eprintln!(
                    "[Agent {}] Claude Code CLI imported changed Unity assets before recompile: {}",
                    self.agent.id, message
                ),
                Err(error) => eprintln!(
                    "[Agent {}] Claude Code CLI failed to import changed Unity assets before recompile: {}",
                    self.agent.id, error
                ),
            }
        }

        if !crate::unity_bridge::unity_integration_enabled(&self.agent.working_dir)
            || !self.agent.is_unity_asset_write_call(tool_call, args)
        {
            return;
        }

        let should_begin = self
            .pending_round
            .as_ref()
            .map(|round| !round.unity_edit_session_started)
            .unwrap_or(false);
        if !should_begin {
            return;
        }
        if let Some(round) = self.pending_round.as_mut() {
            round.unity_edit_session_started = true;
        }

        if !self.agent.session_undo_enabled {
            crate::unity_bridge::begin_edit_session_in_background(&self.agent.working_dir, &self.agent.session_id);
            return;
        }

        match crate::unity_bridge::begin_edit_session(
            &self.agent.working_dir,
            &self.agent.session_id,
        )
        .await
        {
            Ok(message) => eprintln!(
                "[Agent {}] Claude Code CLI Unity edit session active for {}: {}",
                self.agent.id, self.agent.session_id, message
            ),
            Err(error) => eprintln!(
                "[Agent {}] Claude Code CLI failed to begin Unity edit session for {}: {}",
                self.agent.id, self.agent.session_id, error
            ),
        }
    }

    fn note_cli_unity_tool_result(
        &mut self,
        tool_call: &ToolCallInfo,
        args: &serde_json::Value,
        result: &ExecutedToolResult,
    ) {
        let asset_paths = self
            .agent
            .unity_asset_relative_paths(tool_call, args, result);
        if let Some(round) = self.pending_round.as_mut() {
            round.queued_unity_asset_paths.extend(asset_paths);
        }
    }

    fn take_cli_mutation_completion_if_ready(&mut self) -> Option<CliRoundCompletion> {
        let should_finish = self.pending_round.as_ref().is_some_and(|round| {
            round.workspace_guard.is_some()
                && !round.remaining.iter().any(|tool_call_id| {
                    let Some(tool_call) = round
                        .tool_calls
                        .iter()
                        .find(|tool_call| &tool_call.id == tool_call_id)
                    else {
                        return false;
                    };
                    let mut args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    normalize_tool_args(&mut args);
                    self.agent
                        .workspace_execution_request_for_tool(&tool_call.name, &args)
                        .is_some()
                })
        });
        if !should_finish {
            return None;
        }
        let round = self.pending_round.as_mut()?;
        Some(CliRoundCompletion {
            message_id: round.message_id.clone(),
            undo_guard: round.undo_guard.take(),
            has_unity_execute: round.has_unity_execute,
            queued_unity_asset_paths: std::mem::take(&mut round.queued_unity_asset_paths),
            workspace_guard: round.workspace_guard.take(),
        })
    }

    async fn finish_cli_round_external_side_effects(&self, completion: CliRoundCompletion) {
        record_cli_side_effects(self.agent, self.app_handle, self.run_id, Some(self.run_id), completion).await;
    }
}

fn emit_undo_checkpoint_warning(app_handle: &AppHandle, error: &str) {
    let lower = error.to_ascii_lowercase();
    let message = if lower.contains("unable to index file 'nul'")
        || lower.contains("short read while indexing nul")
    {
        "Undo is unavailable for this round because Git could not snapshot the workspace. Remove or rename reserved Windows file names such as NUL in the repository."
    } else {
        "Undo may be unavailable for this round because the workspace snapshot failed."
    };
    crate::error::AppError::emit_background(
        app_handle,
        &crate::error::AppError::new("undo.checkpoint_failed", message)
            .detail(error.to_string())
            .operation("undo")
            .severity(crate::error::ErrorSeverity::Warning),
    );
}

/// Records undo for a finished batch of writes, attributed to its assistant
/// message, then releases the batch's workspace lock. A background subagent's
/// batch has no Locus run (`undo_run_id` is None); `emit_run_id` scopes the
/// live notice.
async fn record_cli_side_effects(
    agent: &AgentInstance,
    app_handle: &AppHandle,
    emit_run_id: &str,
    undo_run_id: Option<&str>,
    completion: CliRoundCompletion,
) {
    let workspace_guard = completion.workspace_guard;
    if !completion.queued_unity_asset_paths.is_empty() {
        crate::unity_bridge::import_assets_fire_and_forget(
            &agent.working_dir,
            completion.queued_unity_asset_paths,
        );
    }

    let Some(undo_guard) = completion.undo_guard else {
        drop(workspace_guard);
        return;
    };
    let Some(undo_mgr) = agent.undo_manager.as_ref() else {
        drop(workspace_guard);
        return;
    };
    let recorded = undo_mgr
        .after_round_for_paths(
            &agent.session_id,
            &completion.message_id,
            undo_run_id,
            undo_guard,
            completion.has_unity_execute,
            &agent.working_dir,
            workspace_guard
                .as_ref()
                .and_then(WorkspaceExecutionGuard::path_keys),
        )
        .await;
    match recorded {
        Ok(true) => {
            if let Some(entry) = undo_mgr
                .find_entry(&agent.session_id, &completion.message_id)
                .await
            {
                if AgentInstance::changed_files_touch_view_tree(&entry.changed_files) {
                    if let Some(execution) = agent.execution_context.as_ref() {
                        crate::view::emit_view_tree_changed_for_scope(
                            app_handle,
                            &crate::workspace_service::event::WorkspaceEventScope::for_runtime(
                                execution.workspace.as_ref(),
                            ),
                        );
                    }
                }
            }
            eprintln!(
                "[Agent {}] emitting Claude Code CLI UndoAvailable for session {} run {} message {}",
                agent.id, agent.session_id, emit_run_id, completion.message_id
            );
            emit_stream(
                app_handle,
                emit_run_id,
                StreamEvent::UndoAvailable {
                    session_id: agent.session_id.clone(),
                    assistant_message_id: completion.message_id,
                },
            );
        }
        Ok(false) => {}
        Err(error) => eprintln!(
            "[Agent {}] failed to record Claude Code CLI undo state for session {} message {}: {}",
            agent.id, agent.session_id, completion.message_id, error
        ),
    }
    drop(workspace_guard);
}

impl<'a> ClaudeCodeHost for ClaudeCodeRoundHost<'a> {
    fn refresh_tools(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Vec<ClaudeCodeToolDefinition>>, String>> + Send + '_>> {
        Box::pin(async {
            let active = self.agent.active_skill_tool_names(self.active_skill_tool_names);
            let tools = self.agent.build_cli_api_tools(self.agent.dynamic_tool_loading_mode(self.app_handle), &active).await;
            Ok(Some(convert_api_tools_to_claude_code(&tools)))
        })
    }
    fn on_checkpoint(&mut self,event:&serde_json::Value)->Result<(),String>{
        self.native_checkpoints.push(event.clone());
        self.flush_native_checkpoints()?;
        claude_code_cli::archive::schedule(self.store.clone(),self.agent.session_id.clone(),self.app_handle.clone());
        Ok(())
    }
    fn on_compaction(&mut self, event: &serde_json::Value) -> Result<(), String> {
        let usage = self.store.get_token_usage(&self.agent.session_id)?;
        if event["type"] == "compact_start" {
            emit_stream(self.app_handle,self.run_id,StreamEvent::CompactStart {
                session_id:self.agent.session_id.clone(),context_tokens:usage.context_tokens,context_limit:usage.context_limit,
                trigger:Some(if event["trigger"]=="auto" {crate::commands::CompactTrigger::Auto} else {crate::commands::CompactTrigger::Manual}),
            });
        } else {
            // Claude's own post-compaction count excludes the system prompt and
            // tools. The Mod's local estimate covers the whole context.
            let tokens=event["estimate"].as_u64().filter(|value|*value>0)
                .or_else(||event["tokens"].as_u64()).or_else(||event["context"]["tokens"].as_u64())
                .map(|v|v.min(u32::MAX as u64) as u32);
            let limit=event["context"]["window"].as_u64().map(|v|v.min(u32::MAX as u64) as u32);
            let usage=self.store.record_token_usage(&self.agent.session_id,0,0,0,0,0.0,0,tokens,limit)?;
            if let Some(point)=event.get("checkpoint").filter(|value|value.is_object()) {
                self.response_metadata["claude_checkpoint"]=point.clone();
            }
            let messages=self.store.get_messages_for_prompt(&self.agent.session_id)?;
            if let Some(boundary)=messages.last() {
                let summary=event["summary"].as_str().filter(|text|!text.is_empty()).ok_or("Claude compaction returned no summary")?;
                let kept: Vec<crate::compact::claude_native::KeptMessage> =
                    serde_json::from_value(event["kept"].clone()).unwrap_or_default();
                let retained = crate::compact::claude_native::resolve_kept_message_ids(&messages, &kept);
                let earliest = retained.first()
                    .and_then(|id| messages.iter().find(|message| &message.id == id))
                    .map(|message| message.created_at)
                    .unwrap_or(boundary.created_at);
                // The receipt belongs to the handoff only; later rows of this
                // run must not each persist a copy of the summary.
                let mut receipt = event.clone();
                if let Some(object) = receipt.as_object_mut() { object.remove("kept"); }
                receipt["keptMessageIds"] = serde_json::json!(retained);
                let mut request = self.response_metadata.clone();
                request["claude_compaction"] = receipt;
                // Claude reads the summary, its kept messages and then the
                // Locus carry-forward (SessionStart context); the mirror keeps
                // both texts on the handoff.
                let body=match event["carryForward"].as_str().filter(|text|!text.trim().is_empty()) {
                    Some(carry)=>format!("{}\n\n{}",summary.trim_end(),carry),
                    None=>summary.to_string(),
                };
                let handoff=crate::compact::claude_native::build_handoff_message(&body,earliest,!retained.is_empty());
                self.store.compact_messages_retaining(&self.agent.session_id,&handoff,&boundary.id,0,&retained,Some(&request))?;
                if let Some(session_id)=self.cli_session_id.as_deref() {
                    self.store.set_message_cli_session_id(&self.agent.session_id,&handoff.id,CLAUDE_CODE_CLI_PROVIDER,session_id)?;
                }
            }
            let page = crate::commands::StreamHistoryPage::load(self.store, &self.agent.session_id)?;
            emit_stream(self.app_handle,self.run_id,StreamEvent::CompactDone {
                session_id:self.agent.session_id.clone(),messages_before:event["before"].as_u64().unwrap_or(0) as u32,
                messages_after:event["after"].as_u64().unwrap_or(0) as u32,context_tokens:usage.context_tokens,context_limit:usage.context_limit,
                messages:page.messages,oldest_message_row_id:page.oldest_message_row_id,has_more_history:page.has_more_history,
            });
        }
        Ok(())
    }
    fn prepare_compaction<'b>(&'b mut self, _event: &'b serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = serde_json::Value> + Send + 'b>> {
        Box::pin(async move {
            match self.compaction_preparation().await {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("[Agent {}] Claude compaction preparation failed for session {}: {}", self.agent.id, self.agent.session_id, error);
                    // Claude still compacts; it only loses the Locus additions.
                    serde_json::json!({"ok":false,"error":error,"instructions":crate::compact::claude_native::SUMMARY_INSTRUCTIONS})
                }
            }
        })
    }
    fn on_phase(&mut self, phase: crate::session::models::ClaudeCodePhase) {
        use crate::session::models::ClaudeCodePhase;
        let phase = if self.retrying && phase == ClaudeCodePhase::Starting { ClaudeCodePhase::Recovering } else { phase };
        emit_stream(self.app_handle, self.run_id, StreamEvent::ClaudeCodePhase {
            session_id: self.agent.session_id.clone(), phase,
        });
    }

    fn on_diagnostic(&mut self, notice: crate::error::AppError) {
        emit_stream(self.app_handle, self.run_id, StreamEvent::ClaudeCodeNotice {
            session_id: self.agent.session_id.clone(), notice,
        });
    }

    fn on_text_delta(&mut self, delta: String) {
        let mark = self
            .render_order
            .mark_text(&self.render_scope, "claude-code-stream-text");
        self.streamed_text.push_str(&delta);
        self.partial_assistant.append_text(&delta);
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::TextDelta {
                session_id: self.agent.session_id.clone(),
                text: delta.clone(),
                order: Some(mark.seq),
                part_id: Some(mark.id),
                render_seq: Some(mark.seq),
            },
        );
        if let Some(ref parent) = self.agent.parent_tool_call {
            emit_parent_stream(self.app_handle, parent.tool_call_delta(delta));
        }
    }

    fn on_thinking_delta(&mut self, delta: String) {
        let mark = self
            .render_order
            .mark_thinking(&self.render_scope, "claude-code-stream-thinking");
        self.partial_assistant.append_thinking(&delta);
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::ThinkingDelta {
                session_id: self.agent.session_id.clone(),
                text: delta,
                order: Some(mark.seq),
                part_id: Some(mark.id),
                render_seq: Some(mark.seq),
            },
        );
    }

    fn on_tool_call_start(&mut self, tool_call_id: String, tool_name: String) {
        self.emit_tool_call_start(&tool_call_id, &tool_name, "");
        self.started_tool_calls.push_back((tool_call_id, tool_name));
    }

    fn on_tool_call_abandoned(&mut self, tool_call_id: &str) {
        if let Some((_, name)) = self.started_tool_calls.iter().find(|(id, _)| id == tool_call_id).cloned() {
            // Duplicate preview receipts still describe one abandoned input.
            self.started_tool_calls.retain(|(id, _)| id != tool_call_id);
            // End the preview in the GUI only. There is no committed call,
            // local execution, journal entry or model-visible tool result.
            self.emit_tool_call_done(tool_call_id, &name,
                "Claude abandoned incomplete tool input before execution.",
                ToolCallOutcome::Interrupted, None);
        }
    }

    fn on_session_started(&mut self, session_id: &str) -> Result<(), String> {
        self.cli_session_id=Some(session_id.to_string());
        if let Some(remote)=self.response_metadata.get("claude_connection").filter(|v|!v.is_null()) {
            let remote: claude_code_cli::connection::NativeConnection=serde_json::from_value(remote.clone()).map_err(|e|format!("Invalid Claude connection: {e}"))?;
            self.store.bind_claude_native_session(&self.agent.session_id,&remote,session_id)?;
        }
        // The user row already exists before launch. Persist the provider resume
        // point before inference, including a first turn that never completes.
        if let Some(message_id)=self.input_message_id.as_deref() {
            self.store.set_message_cli_session_id(&self.agent.session_id,message_id,CLAUDE_CODE_CLI_PROVIDER,session_id)?;
            self.save_response_metadata(message_id)?;
        }
        Ok(())
    }

    fn on_model_resolved(&mut self, model: &str) -> Result<(), String> {
        // Keep the requested model ID stable. The CLI's resolved model is
        // request metadata, never a new composer selection or catalog entry.
        self.response_metadata["upstream_model"] = serde_json::json!(model);
        self.response_metadata["claude_timing"] = serde_json::Value::Null;
        track_claude_stream_origin(&self.partial_assistant, &self.response_metadata, self.run_id);
        self.step_message_ids.clear();
        Ok(())
    }

    fn on_step_started(&mut self, step_id: u64) -> Result<(), String> {
        // Capture the consumed user before inference; a queued input can arrive
        // before this response's usage receipt, but must not steal its attribution.
        let request = crate::session::store::PromptCacheRequest::new(
            format!("live:{}:{step_id}", self.render_scope),
            Some(self.cache_message_id.clone().or_else(|| self.input_message_id.clone()).unwrap_or_default()),
        );
        self.response_metadata["claude_cache_request"] = serde_json::to_value(&request).map_err(|e| e.to_string())?;
        self.cache_request = Some(request);
        Ok(())
    }

    fn on_step_usage(&mut self, usage: &serde_json::Value, context: &serde_json::Value, timing: &serde_json::Value, step_id: u64) -> Result<(), String> {
        if usage["model"].as_str().is_some_and(|value| !value.trim().is_empty()) {
            self.response_metadata["upstream_model"] = usage["model"].clone();
        }
        self.response_metadata["claude_usage"] = usage.clone();
        self.response_metadata["claude_timing"] = timing.clone();
        if timing.is_object() {
            self.response_metadata["claude_timing"]["outputTokens"] = usage["output_tokens"].clone();
        }
        for message_id in std::mem::take(&mut self.step_message_ids) {
            self.save_response_metadata(&message_id)?;
        }
        let count = |name: &str| usage[name].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
        let input_tokens = count("input_tokens");
        let output_tokens = count("output_tokens");
        let cache_read_tokens = count("cache_read_input_tokens");
        let cache_write_tokens = count("cache_creation_input_tokens");
        // A turn may contain many responses. Context is only the latest
        // response's input, while billing counters accumulate every response.
        let context_tokens = input_tokens.saturating_add(cache_read_tokens).saturating_add(cache_write_tokens);
        let context_limit = context["window"].as_u64().filter(|value| *value > 0)
            .unwrap_or(super::model_context_limit(&self.agent.effective_model) as u64).min(u32::MAX as u64) as u32;
        let model = usage["model"].as_str().filter(|value| !value.trim().is_empty())
            .or_else(|| self.response_metadata["upstream_model"].as_str())
            .or_else(|| self.response_metadata["model"].as_str()).unwrap_or_default();
        let mut cache_request = self.cache_request.clone().filter(|request| request.key == format!("live:{}:{step_id}", self.render_scope))
            .unwrap_or_else(|| crate::session::store::PromptCacheRequest::new(format!("live:{}:{step_id}", self.render_scope), Some(String::new())));
        cache_request.usage_complete = crate::session::store::claude_prompt_usage_complete(usage) && !model.is_empty();
        let context_available = cache_request.usage_complete;
        let Some((totals, check)) = self.store.record_model_usage_best_effort(crate::session::store::ModelUsageRecord {
            session_id: self.agent.session_id.clone(), model_id: format!("claude_code/{model}"),
            provider: "Claude Code CLI".into(), request_kind: "completion".into(),
            input_tokens: input_tokens as u64, output_tokens: output_tokens as u64,
            model_active_duration_ms: claude_response_duration_ms(timing),
            cache_read_tokens: cache_read_tokens as u64, cache_write_tokens: cache_write_tokens as u64,
            cost_usd: 0.0, priced_rounds: 0, context_tokens: context_available.then_some(context_tokens), context_limit: Some(context_limit),
            cache_request: Some(cache_request),
        }) else {
            // Raw usage is already retained in response_metadata. Statistics
            // use bounded retries and must not abort the final answer.
            return Ok(());
        };
        emit_stream(self.app_handle, self.run_id, StreamEvent::UsageUpdate {
            session_id: self.agent.session_id.clone(), input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
            cache_invalidated: check.as_ref().is_some_and(|check| check.invalidated),
            cache_baseline_tokens: check.as_ref().map(|check| check.baseline_tokens).unwrap_or(0),
            cache_excess_input_tokens: check.as_ref().map(|check| check.excess_input_tokens).unwrap_or(0),
            cache_invalidation_reason: check.filter(|check| check.invalidated).map(|check| check.reason),
            total_input_tokens: totals.total_input_tokens, total_output_tokens: totals.total_output_tokens,
            total_cache_read_tokens: totals.total_cache_read_tokens, total_cache_write_tokens: totals.total_cache_write_tokens,
            timed_output_tokens: totals.timed_output_tokens, model_active_duration_ms: totals.model_active_duration_ms,
            total_cost_usd: totals.total_cost_usd, priced_rounds: totals.priced_rounds, context_tokens: totals.context_tokens, context_limit,
        });
        Ok(())
    }

    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> {
        self.save_assistant_message(message, false)
    }

    fn on_interrupted_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> {
        self.last_assistant = None;
        if message.text.is_empty() && message.thinking_text.is_empty() { return Ok(()); }
        self.save_assistant_message(message, true)
    }
    fn native_agent_host(&self) -> Option<Arc<dyn claude_code_cli::ClaudeNativeAgentHost>> {
        self.agent.claude_delegation()?;
        Some(native_agent_hub::NativeAgentHub::install(self))
    }
    fn before_native_agent<'b>(
        &'b mut self,
        tool_call_id: &'b str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'b>> {
        Box::pin(async move {
            if let Some(hub) = native_agent_hub::NativeAgentHub::existing(&self.agent.session_id) {
                let message_id = self.message_id_for_tool_call(tool_call_id);
                hub.note_agent_call(tool_call_id, message_id, self.run_id);
            }
            self.finish_parent_writes_before_agent(tool_call_id).await;
        })
    }
    fn on_external_tool_result<'b>(
        &'b mut self,
        id: &'b str,
        result: ClaudeCodeToolResult,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'b>> {
        Box::pin(async move {
            // Bridged/MCP executions already persisted their result. This path
            // records CLI session tools and Mod refusals without executing them.
            if self.completed_tool_ids.contains(id) {
                return Ok(());
            }
            if self.agent.is_cancel_requested() {
                return Ok(());
            }
            self.select_tool_round(id);
            let call = self.known_tool_calls.iter().position(|call| call.id == id)
                .and_then(|index|self.known_tool_calls.remove(index))
                .or_else(||self.pending_round.as_ref().and_then(|round|round.tool_calls.iter().find(|call|call.id==id).cloned()));
            let Some(call)=call else {return Ok(());};
            let is_error = result.is_error;
            let images = result.images.clone();
            let snapshot = serde_json::from_str::<serde_json::Value>(&result.output).ok()
                .and_then(|value| value.get("locusTodoSnapshot").cloned());
            let output = if is_error { result.text_with_context() } else {
                let mut projected = result.clone();
                projected.output = claude_code_cli::native_output::external(&call.name, &result.output);
                projected.text_with_context()
            };
            // A native Agent's subagent calls persist with its result.
            let nested = native_agent_hub::NativeAgentHub::existing(&self.agent.session_id)
                .map(|hub| hub.take_records(id))
                .unwrap_or_default();
            let result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output, is_error })
                .with_nested_tool_calls(nested);
            let output = self.store.rewrite_tool_result_for_storage(
                &self.agent.session_id,
                id,
                &call.name,
                &result.output,
            )?;
            let message_id=save_claude_code_tool_result(
                self.store,
                &self.agent.session_id,
                self.run_id,
                id,
                &output,
                images.as_deref(),
            )?;
            let Some(message_id)=message_id else {return Ok(());};
            if !is_error {
                if let Some(snapshot)=snapshot {
                    self.store.record_claude_todos(&self.agent.session_id,&message_id,self.run_id,&snapshot)?;
                }
            }
            self.emit_tool_call_done(
                id,
                &call.name,
                &output,
                result.outcome.as_stream_outcome(),
                images.as_deref(),
            );
            self.completed_tool_ids.insert(id.to_string());
            if let Some(round) = self.pending_round.as_mut() {
                if let Some(record) = round.tool_calls.iter_mut().find(|call| call.id == id) {
                    *record = finalize_tool_call_record(record, Some(&result));
                }
                round.remaining.remove(id);
            }
            let completion = self.take_cli_mutation_completion_if_ready();
            self.maybe_finish_pending_round();
            if let Some(completion) = completion {
                self.finish_cli_round_external_side_effects(completion)
                    .await;
            }
            Ok(())
        })
    }

    fn execute_tool<'b>(
        &'b mut self,
        request_id: &'b str,
        tool_name: &'b str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'b> {
        Box::pin(async move {
            self.select_tool_round(request_id);
            self.ensure_cli_foreground_subagent_phase().await;
            let mut tool_call = self.take_matching_tool_call(request_id, tool_name, &arguments);
            let mut args_for_exec = arguments.clone();
            if tool_call.name == "tool_call" {
                match super::parse_meta_tool_call_arguments(&tool_call.arguments) {
                    Ok((target_name, mut target_args)) => {
                        let allowed = self
                            .agent
                            .allowed_tool_set_for_active_skills(&self.agent.active_skill_tool_names(self.active_skill_tool_names))
                            .await;
                        if let Some(canonical) =
                            self.agent.canonical_tool_name(&target_name).filter(|name| {
                                !AgentInstance::is_meta_tool(name) && allowed.contains(name)
                            })
                        {
                            normalize_tool_args(&mut target_args);
                            let target_arguments = serde_json::to_string(&target_args)
                                .unwrap_or_else(|_| "{}".to_string());
                            eprintln!(
                                "[Agent {}] meta-call dispatch: tool_call -> '{}' args_len={}",
                                self.agent.id,
                                canonical,
                                target_arguments.len()
                            );
                            tool_call.name = canonical;
                            tool_call.arguments = target_arguments.clone();
                            args_for_exec = target_args;
                            self.emit_tool_call_start(
                                &tool_call.id,
                                &tool_call.name,
                                &tool_call.arguments,
                            );
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "[Agent {}] invalid Claude Code CLI meta-call arguments for tool_call id={}: {}",
                            self.agent.id, tool_call.id, error
                        );
                    }
                }
            } else {
                normalize_tool_args(&mut args_for_exec);
            }
            self.agent
                .inject_working_dir(&tool_call.name, &mut args_for_exec);
            // The native route delegates only through Agent, including meta calls.
            let delegation_refusal = (self.agent.claude_delegation().is_some()
                && super::multi_agent::requires_multi_agent(&tool_call.name))
            .then(|| ExecutedToolResult::from_tool_result(crate::tool::ToolResult {
                output: format!("Delegate with the Agent tool in this conversation; '{}' was not run.", tool_call.name),
                is_error: true,
            }));

            let workspace_policy = self.cli_round_workspace_policy(&tool_call.name, &args_for_exec);
            let is_deterministic_pre_ask_call =
                AgentInstance::is_deterministic_pre_ask_tool(&tool_call.name)
                    && self.pending_round.as_ref().is_some_and(|round| {
                        round.tool_calls.iter().any(|pending_tool_call| {
                            let mut pending_args = serde_json::from_str::<serde_json::Value>(
                                &pending_tool_call.arguments,
                            )
                            .unwrap_or_else(|_| serde_json::json!({}));
                            normalize_tool_args(&mut pending_args);
                            self.agent.effective_tool_name_for_round(
                                &pending_tool_call.name,
                                &pending_args,
                            ) == "ask_user_question"
                        })
                    });
            let mut confirmation_preapproved = false;
            let precompleted_subagent_result = self
                .pending_round
                .as_mut()
                .and_then(|round| round.precompleted_subagent_results.remove(&tool_call.id));
            let mut result_override = precompleted_subagent_result.or(delegation_refusal).or_else(|| match workspace_policy.as_ref() {
                Err(error) => {
                    eprintln!(
                        "[Agent {}] Claude Code tool round policy blocked tool='{}' id={} session={} run={} reason={}",
                        self.agent.id,
                        tool_call.name,
                        tool_call.id,
                        self.agent.session_id,
                        self.run_id,
                        error
                    );
                    Some(ExecutedToolResult::from_tool_result(
                        crate::tool::ToolResult {
                            output: error.clone(),
                            is_error: true,
                        },
                    ))
                }
                Ok(_) => None,
            });

            // Bash permission semantics are call-specific. An external-workdir
            // write skips the primary-workspace lock while remaining a write
            // call; a declared read-only command may still need confirmation
            // when dangerous-command or knowledge governance detects risk.
            let needs_confirmation_preflight =
                workspace_policy.as_ref().is_ok_and(Option::is_some) || tool_call.name == "bash";
            if result_override.is_none() && needs_confirmation_preflight {
                if self.pending_round.is_some() {
                    self.ensure_cli_round_confirmations_prepared().await;
                    if let Some(round) = self.pending_round.as_ref() {
                        if let Some(result) = round.confirmation_rejections.get(&tool_call.id) {
                            result_override = Some(result.clone());
                        } else {
                            confirmation_preapproved =
                                round.confirmation_preapproved.contains(&tool_call.id);
                        }
                    }
                } else if !matches!(
                    tool_call.name.as_str(),
                    "tool_load" | super::CODEX_TOOL_SEARCH_TOOL_NAME | "exit_plan_mode"
                ) {
                    let decision = self
                        .agent
                        .request_tool_confirm(
                            self.app_handle,
                            &tool_call.id,
                            &tool_call.name,
                            &tool_call.arguments,
                            &args_for_exec,
                            self.run_id,
                        )
                        .await;
                    if let Some(result) = self
                        .agent
                        .confirmation_rejection_result(&tool_call.name, decision)
                    {
                        result_override = Some(result);
                    } else {
                        confirmation_preapproved = true;
                    }
                }
            }

            let mut _single_tool_workspace_guard: Option<WorkspaceExecutionGuard> = None;
            if result_override.is_none() {
                if let Ok(Some((lock_request, tools))) = workspace_policy.as_ref() {
                    let already_locked = !is_deterministic_pre_ask_call
                        && self
                            .pending_round
                            .as_ref()
                            .is_some_and(|round| round.workspace_guard.is_some());
                    if !already_locked {
                        eprintln!(
                            "[Agent {}] Claude Code tool round acquiring workspace mutation coordination mode={} session={} run={} tools=[{}]",
                            self.agent.id,
                            lock_request.label(),
                            self.agent.session_id,
                            self.run_id,
                            tools.join(",")
                        );
                        let owner = WorkspaceExecutionLockOwner {
                            session_id: self.agent.session_id.clone(),
                            run_id: self.run_id.to_string(),
                            iteration: 0,
                            workspace: self.agent.working_dir.clone(),
                            tools: tools.clone(),
                        };
                        let acquisition = match self.agent.execution_context.as_ref() {
                            Some(execution) => process_workspace_execution_lock(
                                &self.agent.working_dir,
                            )
                            .acquire_with_diagnostics(
                                lock_request.clone(),
                                owner,
                                self.agent.cancel_waiter(),
                                crate::workspace_service::event::WorkspaceEventScope::for_runtime(
                                    execution.workspace.as_ref(),
                                ),
                                self.app_handle,
                            )
                            .await,
                            None => Err(WorkspaceExecutionLockAcquireError::MissingWorkspaceScope),
                        };
                        match acquisition {
                            Ok(guard) => {
                                if is_deterministic_pre_ask_call {
                                    _single_tool_workspace_guard = Some(guard);
                                } else if let Some(round) = self.pending_round.as_mut() {
                                    round.workspace_guard = Some(guard);
                                } else {
                                    _single_tool_workspace_guard = Some(guard);
                                }
                            }
                            Err(_) => {
                                result_override = Some(AgentInstance::interrupted_tool_result());
                            }
                        }
                    }
                }
            }

            let assistant_message_id = self
                .pending_round
                .as_ref()
                .map(|round| round.message_id.clone())
                .or_else(|| self.last_persisted_assistant_message_id.clone())
                .unwrap_or_default();
            let mut result = if let Some(result) = result_override {
                result
            } else if tool_call.name == "Agent" {
                // Only Locus-routed types reach the host; native types run in Claude.
                self.finish_parent_writes_before_agent(&tool_call.id).await;
                self.execute_locus_agent(&tool_call, &arguments, &assistant_message_id)
                    .await
            } else if tool_call.name == "SendMessage" {
                // Only messages to Locus subagents reach the host.
                self.execute_locus_send_message(&tool_call, &arguments)
            } else {
                self.ensure_cli_round_undo_checkpoint(&tool_call.name, &args_for_exec)
                    .await;
                self.prepare_cli_unity_tool(&tool_call, &args_for_exec)
                    .await;
                self.agent
                    .execute_single_tool(
                        self.app_handle,
                        self.store,
                        &tool_call,
                        &args_for_exec,
                        self.run_id,
                        &assistant_message_id,
                        self.mode,
                        &self.agent.active_skill_tool_names(self.active_skill_tool_names),
                        confirmation_preapproved,
                    )
                    .await
            };

            if !self.agent.run_is_current_for_session(
                self.store,
                self.run_id,
                "claude_code_tool_result",
                Some(&tool_call.id),
            ) || self.agent.is_cancel_requested()
            {
                return ClaudeCodeToolResult::from(crate::tool::ToolResult {
                    output: crate::session::history::INTERRUPTED_TOOL_RESULT.to_string(),
                    is_error: false,
                });
            }

            let mut stored_output = match self.store.rewrite_tool_result_for_storage(
                &self.agent.session_id,
                &tool_call.id,
                &tool_call.name,
                &result.output,
            ) {
                Ok(output) => output,
                Err(err) => {
                    eprintln!(
                        "[Agent {}] failed to persist Claude Code CLI tool result for '{}' (id={}): {}",
                        self.agent.id, tool_call.name, tool_call.id, err
                    );
                    result.output.clone()
                }
            };

            if !result.is_error && stored_output != result.output {
                bound_native_tool_result(&tool_call.id, &stored_output);
            }
            validate_native_tool_result(&tool_call.id, &mut result, &mut stored_output);
            match save_claude_code_tool_result(
                self.store,
                &self.agent.session_id,
                self.run_id,
                &tool_call.id,
                &stored_output,
                result.images.as_deref(),
            ) {
                Ok(Some(message_id)) => {
                    if tool_call.name=="todowrite" && !result.is_error {
                        let snapshot=self.store.get_todos(&self.agent.session_id).map(|todos|serde_json::json!({"version":1,"kind":"todos","items":todos.items}));
                        if let Err(error)=snapshot.and_then(|snapshot|self.store.record_claude_todos(&self.agent.session_id,&message_id,self.run_id,&snapshot)) {
                            return ClaudeCodeToolResult{output:format!("Todo update completed but its history snapshot could not be saved: {error}"),is_error:true,images:None,context:Vec::new()};
                        }
                    }
                }
                Ok(None) => {
                    eprintln!(
                        "[Agent {}] discarding stale Claude Code CLI tool result before save: session={} run={} tool_call_id={}",
                        self.agent.id, self.agent.session_id, self.run_id, tool_call.id
                    );
                    return ClaudeCodeToolResult::from(crate::tool::ToolResult {
                        output: crate::session::history::INTERRUPTED_TOOL_RESULT.to_string(),
                        is_error: false,
                    });
                }
                Err(err) => {
                    eprintln!(
                        "[Agent {}] failed to save Claude Code CLI tool result for '{}' (id={}): {}",
                        self.agent.id, tool_call.name, tool_call.id, err
                    );
                }
            }

            self.emit_tool_call_done(
                &tool_call.id,
                &tool_call.name,
                &stored_output,
                result.outcome.as_stream_outcome(),
                result.images.as_deref(),
            );

            self.completed_tool_ids.insert(tool_call.id.clone());
            self.note_cli_unity_tool_result(&tool_call, &args_for_exec, &result);
            if let Some(round) = self.pending_round.as_mut() {
                if let Some(existing) = round
                    .tool_calls
                    .iter_mut()
                    .find(|pending_tool_call| pending_tool_call.id == tool_call.id)
                {
                    existing.name = tool_call.name.clone();
                    existing.arguments = tool_call.arguments.clone();
                    *existing = finalize_tool_call_record(existing, Some(&result));
                }
                round.remaining.remove(&tool_call.id);
            }
            let completed_round = self.take_cli_mutation_completion_if_ready();
            self.maybe_finish_pending_round();
            if let Some(completed_round) = completed_round {
                self.finish_cli_round_external_side_effects(completed_round)
                    .await;
            }

            // Claude owns its model loop. Deliver queued completions alongside
            // the last tool response, before it makes its next model request.
            let output = stored_output;
            let mut context = Vec::new();
            if self.pending_round.is_none() {
                use tauri::Manager;
                let manager = self
                    .app_handle
                    .state::<Arc<crate::async_tasks::AsyncTaskManager>>();
                let delivery = manager.deliver_notifications(&self.agent.session_id, self.store).and_then(|_| {
                    let messages = self.store.get_messages(&self.agent.session_id)?;
                    let carrier = messages.iter().find(|message| message.tool_call_id.as_deref() == Some(&tool_call.id))
                        .ok_or("Missing Claude tool result for notifications")?;
                    self.store.bind_claude_notifications(&self.agent.session_id, &carrier.id)
                });
                match delivery {
                    Ok(reminders) => {
                        for reminder in reminders {
                            if !context.contains(&reminder) { context.push(reminder); }
                        }
                    }
                    Err(error) => {
                        eprintln!("[Agent async] CLI result delivery will retry: {error}")
                    }
                }
            }
            ClaudeCodeToolResult {
                output,
                context,
                is_error: result.is_error,
                images: result.images,
            }
        })
    }
}

impl AgentInstance {
    async fn build_cli_api_tools(
        &self,
        dynamic_mode: crate::config::DynamicToolLoadingMode,
        active_skill_tool_names: &HashSet<String>,
    ) -> Vec<serde_json::Value> {
        // The native route delegates only through Agent/SendMessage. Locus
        // subagent/send_message stay authorized for that route's forwarding.
        let unified = self.claude_delegation().is_some();
        let delegation_hidden = |tool: &serde_json::Value| unified
            && tool["function"]["name"].as_str().is_some_and(super::multi_agent::requires_multi_agent);
        if self.cached_lazy_tool_renderer() == super::LazyToolRenderer::ClaudeCliNative {
            let plan = self.build_native_request_tool_plan(super::LazyToolRenderer::ClaudeCliNative, active_skill_tool_names).await;
            let mut tools = self.build_api_tools_for_plan(super::LazyToolRenderer::ClaudeCliNative, &plan).await;
            // Only the model-facing catalog is deduplicated. The local names
            // must remain authorized: native Edit/Bash/etc. dispatch to them.
            tools.retain(|tool| tool["function"]["name"].as_str().is_none_or(|name|
                claude_code_cli::native_tools::native_equivalent_for_locus_tool(name).is_none()) && !delegation_hidden(tool));
            return tools;
        }
        let names = self.build_cli_request_tool_names(dynamic_mode, active_skill_tool_names).await;
        let mut tools = self.build_api_tools(&names).await;
        tools.retain(|tool| !delegation_hidden(tool));
        tools
    }

    async fn build_cli_request_tool_names(
        &self,
        dynamic_tool_loading_mode: crate::config::DynamicToolLoadingMode,
        active_skill_tool_names: &HashSet<String>,
    ) -> Vec<String> {
        if dynamic_tool_loading_mode != crate::config::DynamicToolLoadingMode::Direct {
            return self
                .build_request_tool_names_for_mode_and_skills(
                    dynamic_tool_loading_mode,
                    active_skill_tool_names,
                )
                .await;
        }

        let mut names = vec!["tool_load".to_string()];
        let mut allowed: Vec<String> = self
            .allowed_tool_set_for_active_skills(active_skill_tool_names)
            .await
            .into_iter()
            .filter(|name| !Self::is_meta_tool(name))
            .collect();
        allowed.sort();
        for name in allowed {
            if !names.iter().any(|existing| existing == &name) {
                names.push(name);
            }
        }
        names
    }

    pub(super) async fn run_claude_code_cli(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        prompt_text: &str,
        system_prompt: &str,
        images: Option<&[ImageData]>,
        initial_mode: &str,
        run_id: &str,
        active_skill_tool_names: &HashSet<String>,
        current_user_message_id: Option<&str>,
    ) -> Result<String, String> {
        use claude_code_cli::turn_retry;
        use std::sync::atomic::{AtomicU32, Ordering};
        let current_attempt = AtomicU32::new(0);
        // Keep steering ownership across transport attempts. Dropping it after
        // each disconnect would restore already-consumed input into the queue.
        let input_prefix = store.get_messages_for_prompt(&self.session_id)?.into_iter().find_map(|message| message.prompt_prefix)
            .filter(|text| !duplicates_system_reminder(text, system_prompt));
        let queued_inputs = ClaudeQueuedInputs {
            steering: super::codex_steering::RunSteering::new(self, app_handle, store, run_id, input_prefix.as_deref()),
            agent: self, app: app_handle, initial_skills: active_skill_tool_names,
        };
        let inputs = &queued_inputs;
        let fallback = || serde_json::json!({
            "backend":"claude_code_cli", "session_id":self.session_id,
            "model":self.effective_model, "effort":self.effort, "system":system_prompt,
            "messages":[{"role":"user","content":prompt_text}],
        });
        let result = turn_retry::run(
            |attempt| {
                current_attempt.store(attempt, Ordering::Relaxed);
                let mut capture = fallback();
                async move {
                    let result = self.run_claude_code_cli_attempt(app_handle, store, prompt_text,
                        system_prompt, images, initial_mode, run_id, active_skill_tool_names,
                        current_user_message_id, attempt, &mut capture, inputs).await;
                    if let Err(error) = &result {
                        self.record_captured_attempt(store, run_id, "normal", 1, attempt,
                            "failed", capture, "", Some(error)).await;
                    }
                    result
                }
            },
            |attempt, error| async move {
                // Local CLI lacks a source-history reconciliation transport, and
                // replaying /compact could compact a successfully completed command
                // twice. Fail closed rather than treating either as a normal turn.
                if initial_mode == "compact" || claude_code_cli::recovery::recorded_remote(store, &self.session_id)?.is_none() {
                    return Err(format!("Automatic continuation requires a verified remote Claude conversation; no input was replayed. {error}"));
                }
                if !self.run_is_current_for_session(store, run_id, "claude_code_retry", None) {
                    return Err("Claude recovery stopped because this run is no longer current".into());
                }
                Self::persist_interrupted_assistant_snapshot(store, &self.session_id, &self.partial_assistant.snapshot());
                self.partial_assistant.reset();
                claude_code_cli::tui::close_session(&self.session_id);
                self.cleanup_unity_edit_session().await;
                eprintln!("[Agent {}] Claude transport recovery {}/{}: session={} run={} error={}",
                    self.id, attempt, turn_retry::RETRY_DELAYS.len(), self.session_id, run_id, error);
                emit_stream(app_handle, run_id, StreamEvent::ClaudeCodePhase {
                    session_id: self.session_id.clone(), phase: crate::session::models::ClaudeCodePhase::Recovering,
                });
                Ok(())
            },
            self.cancel_waiter(), &turn_retry::RETRY_DELAYS,
        ).await?;
        drop(queued_inputs);
        match result {
            Some(text) => Ok(text),
            None => {
                // Claude's own Esc already cancelled the turn. Running
                // background agents keep the process, as in Claude's TUI.
                claude_code_cli::tui::close_cancelled_session(&self.session_id);
                self.record_captured_attempt(store, run_id, "normal", 1,
                    current_attempt.load(Ordering::Relaxed).max(1), "cancelled", fallback(), "",
                    Some("cancelled during Claude execution or recovery")).await;
                self.clear_pending_knowledge_proposal(app_handle).await;
                self.emit_cancelled(app_handle, store, run_id, None);
                Ok(String::new())
            }
        }
    }

    async fn run_claude_code_cli_attempt(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        prompt_text: &str,
        system_prompt: &str,
        images: Option<&[ImageData]>,
        initial_mode: &str,
        run_id: &str,
        active_skill_tool_names: &HashSet<String>,
        current_user_message_id: Option<&str>,
        attempt: u32,
        capture: &mut serde_json::Value,
        inputs: &ClaudeQueuedInputs<'_>,
    ) -> Result<String, String> {
        if attempt > 1 && !self.run_is_current_for_session(store, run_id, "claude_code_retry_start", None) {
            return Err("Claude recovery stopped because this run is no longer current".into());
        }
        let dynamic_tool_loading_mode = self.dynamic_tool_loading_mode(app_handle);
        if dynamic_tool_loading_mode == crate::config::DynamicToolLoadingMode::Direct {
            let messages = store.get_messages_for_prompt(&self.session_id)?;
            self.seed_loaded_tools_from_history(&messages).await;
        }
        let api_tools = self
            .build_cli_api_tools(dynamic_tool_loading_mode, active_skill_tool_names)
            .await;

        #[cfg(feature = "server-client")]
        let server_binding = crate::server_client::binding_for_model(&self.effective_model)?.map(|(binding, _)| binding.id);
        #[cfg(not(feature = "server-client"))]
        let server_binding: Option<String> = None;
        let recorded_connection = claude_code_cli::recovery::recorded_connection(store,&self.session_id)?;
        if let Some(binding_id)=server_binding.as_deref() {
            let expected=claude_code_cli::connection::NativeConnection::server(binding_id)?;
            if recorded_connection.as_ref().is_some_and(|recorded|recorded.as_ref()!=Some(&expected)) {
                return Err("Use a new conversation for Locus Server; this history belongs to another connection.".into());
            }
        } else {
            if recorded_connection.as_ref().and_then(|value|value.as_ref()).is_some_and(|connection|connection.server_binding_id().is_some()) {
                return Err("This conversation requires its bound internal Locus Server connection".into());
            }
            claude_code_cli::tui::prewarm::wait_for(&self.session_id).await?;
        }
        let pending = if initial_mode == "compact" { None } else { current_user_message_id.map(str::to_owned) };
        if server_binding.is_none() && claude_code_cli::archive::transfer_check(store,&self.session_id)?.is_some() {
            return Err("The SSH server has changed. Confirm restoring this conversation in the input area before sending.".into());
        }
        let compact = initial_mode == "compact";
        let recovered = if attempt > 1 {
            claude_code_cli::recovery::synchronize_for_retry(store, &self.session_id, pending.as_deref()).await?
        } else if compact {
            claude_code_cli::recovery::synchronize_for_compaction(store, &self.session_id).await?
        } else {
            claude_code_cli::recovery::synchronize(store, &self.session_id, pending.as_deref()).await?
        };
        let recovery_required = recovered.warning.is_some() || recovered.resume_context.is_some();
        if recovered.changed || attempt > 1 {
            let page = crate::commands::StreamHistoryPage::load(store, &self.session_id)?;
            emit_stream(app_handle, run_id, StreamEvent::HistorySynchronized {
                session_id: self.session_id.clone(),
                messages: page.messages,
                oldest_message_row_id: page.oldest_message_row_id,
                has_more_history: page.has_more_history,
            });
        }
        if !compact {
            if let Some(warning) = recovered.warning { return Err(warning); }
        }
        ensure_claude_history_origin(store, &self.session_id, pending.as_deref())?;
        if attempt > 1 { inputs.steering.reconcile_claude_history()?; }
        if let Some(message) = recovered.completed_message.filter(|_| !inputs.steering.has_immediate_claude_input()) {
            // The source already contains the final answer; acknowledge it
            // without submitting another prompt or repeating completed work.
            self.record_captured_attempt(store, run_id, "normal", 1, attempt, "completed",
                serde_json::json!({"backend":"claude_code_cli","recovered_completion":true}),
                &message.content, None).await;
            store.close_run_pending_input_queue(run_id)?;
            store.set_latest_completed_run_id(&self.session_id, Some(run_id))?;
            emit_stream(app_handle, run_id, StreamEvent::Done {
                session_id: self.session_id.clone(), message_id: message.id,
                full_text: message.content.clone(), content_order: None, thinking_order: None, render_parts: None,
            });
            self.partial_assistant.reset();
            if let Err(error) = self.flush_pending_knowledge_proposal(app_handle, store, run_id).await {
                eprintln!("[Agent {}] failed to flush recovered knowledge proposal: {error}", self.id);
            }
            return Ok(message.content);
        }
        // The durable native binding survives replacement of the display
        // history and is updated atomically on rewind. Message IDs are only
        // a legacy fallback; the in-process cache is never the authority.
        // A failed read must never silently start a replacement conversation.
        let resume_session_id =
            match claude_code_cli::recovery::native_session_id(store, &self.session_id) {
                Ok(persisted) => {
                    match persisted.as_deref() {
                        Some(session_id) => {
                            claude_code_cli::store_session_id(&self.session_id, session_id).await;
                        }
                        None => {
                            claude_code_cli::clear_stale_session_id_for_turn(&self.session_id).await;
                        }
                    }
                    persisted
                }
                Err(error) => return Err(format!("Cannot verify the saved Claude conversation identity: {error}")),
            };
        if resume_session_id.is_none() && store.get_messages_for_prompt(&self.session_id)?.iter().any(|m| m.role == MessageRole::Assistant)
        {
            return Err("This conversation has no verifiable Claude Code session. Create a new Claude Code conversation; existing history was not sent.".into());
        }

        let remote = if let Some(binding_id) = &server_binding {
            let selected = claude_code_cli::connection::NativeConnection::server(binding_id.clone())?;
            if recorded_connection.as_ref().is_some_and(|recorded|recorded.as_ref()!=Some(&selected)) {
                return Err("The saved Claude conversation belongs to another connection".into());
            }
            Some(selected)
        } else {
            claude_code_cli::ssh::native_session_connection(&self.session_id, resume_session_id.is_some(), recorded_connection)?
        };
        if !compact {
            use tauri::Manager;
            // Synchronize old receipts before deciding which notifications
            // still need a carrier. Local persistence is not a native ACK.
            app_handle.state::<Arc<crate::async_tasks::AsyncTaskManager>>()
                .deliver_notifications(&self.session_id, store)?;
        }
        let mut user_message = if initial_mode=="compact" {build_cli_user_message(prompt_text,images)} else {
            prepare_pending_claude_input_with_model(store,&self.session_id,pending.as_deref(),prompt_text,&remote,system_prompt,server_binding.as_ref().map(|_|self.effective_model.as_str()))?
        };
        if let Some(context)=recovered.resume_context {
            if let Some(text)=user_message["content"].as_str() {
                user_message["content"]=serde_json::json!(format!("{text}\n\n{context}"));
            } else {
                user_message["content"].as_array_mut().ok_or("Invalid Claude input content")?.push(serde_json::json!({"type":"text","text":context}));
            }
        }
        if initial_mode == "compact" {
            user_message["locusControl"] = serde_json::json!("compact");
        }
        let tools = convert_api_tools_to_claude_code(&api_tools);
        let mut host = ClaudeCodeRoundHost {
            agent: self,
            app_handle,
            store,
            run_id,
            input_message_id: user_message["locusMessageId"].as_str().map(str::to_owned),
            cache_message_id: None,
            cache_request: None,
            mode: initial_mode,
            active_skill_tool_names,
            streamed_text: String::new(),
            partial_assistant: self.partial_assistant_state(),
            started_tool_calls: VecDeque::new(),
            known_tool_calls: VecDeque::new(),
            completed_tool_ids: HashSet::new(),
            pending_round: None,
            waiting_rounds: Vec::new(),
            last_assistant: None,
            last_persisted_assistant_message_id: None,
            cli_session_id: None,
            response_metadata: serde_json::json!({"backend":"claude_code_cli", "model": self.effective_model, "upstream_model": null, "claude_connection": remote, "claude_native_tool_search": self.cached_lazy_tool_renderer() == super::LazyToolRenderer::ClaudeCliNative}),
            native_checkpoints: Vec::new(),
            step_message_ids: Vec::new(),
            render_order: StreamRenderOrderTracker::default(),
            render_scope: if attempt == 1 { run_id.to_string() } else { format!("{run_id}:attempt-{attempt}") },
            retrying: attempt > 1,
        };
        track_claude_stream_origin(&self.partial_assistant, &host.response_metadata, run_id);

        let delegation = self.claude_delegation();
        let cwd = if self.has_selected_working_dir() {
            self.working_dir.clone()
        } else {
            std::env::current_dir()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|_| ".".to_string())
        };
        let agents = delegation
            .as_ref()
            .map(|directory| directory.mod_payload(&super::claude_native_agents::claude_subagent_environment(&cwd)))
            .unwrap_or(serde_json::Value::Null);
        let options = ClaudeCodeCliOptions {
            remote: remote.as_ref().and_then(|remote| remote.as_ssh()).cloned(),
            locus_session_id: self.session_id.clone(),
            cwd,
            system_prompt: system_prompt.to_string(),
            model: self.effective_model.clone(),
            effort: self.effort.clone(),
            resume_session_id,
            tools,
            native_tool_search: self.cached_lazy_tool_renderer() == super::LazyToolRenderer::ClaudeCliNative,
            native_agents: delegation.is_some(),
            agents,
        };
        *capture = serde_json::json!({
            "backend": "claude_code_cli",
            "session_id": self.session_id,
            "model": self.effective_model,
            "effort": self.effort,
            "system": system_prompt,
            "messages": [{
                "role": "user",
                "content": prompt_text,
            }],
            "tools": api_tools,
            "raw_stdin": user_message,
        });
        let turn_result = if compact {
            // Queued input cannot turn summary-only compaction into inference.
            claude_code_cli::run_turn(options, user_message, &mut host).await
        } else {
            claude_code_cli::run_turn_with_inputs(options, user_message, &mut host, inputs).await
        };
        if compact && recovery_required {
            // A healthy TUI is normally reused without synchronization. Do not
            // let successful compaction bypass the interrupted-tool guard on
            // the next ordinary turn. Leave the process alive to flush its rows.
            claude_code_cli::tui::require_recovery(&self.session_id);
        }
        let turn = turn_result?;

        if let Some(claude_session_id) = turn.claude_session_id.as_deref() {
            claude_code_cli::store_session_id(&self.session_id, claude_session_id).await;
        }

        self.record_captured_attempt(
            store,
            run_id,
            "normal",
            1,
            attempt,
            "completed",
            serde_json::json!({
                "backend": "claude_code_cli",
                "session_id": self.session_id,
                "claude_session_id": turn.claude_session_id,
                "model": self.effective_model,
                "effort": self.effort,
                "system": system_prompt,
                "messages": [{
                    "role": "user",
                    "content": prompt_text,
                }],
                "tools": api_tools,
                "raw_stdin": turn.raw_request,
            }),
            &turn.raw_response,
            None,
        )
        .await;

        if !self.run_is_current_for_session(store, run_id, "claude_code_turn_done", None) {
            return Ok(String::new());
        }

        if self.is_cancel_requested() {
            self.clear_pending_knowledge_proposal(app_handle).await;
            self.emit_cancelled(app_handle, store, run_id, None);
            return Ok(String::new());
        }

        let final_snapshot = host.last_assistant.clone();
        let final_text = if let Some(snapshot) = final_snapshot.as_ref() {
            if !snapshot.text.is_empty() {
                snapshot.text.clone()
            } else if !turn.final_text.is_empty() {
                turn.final_text.clone()
            } else if !host.streamed_text.is_empty() {
                host.streamed_text.clone()
            } else {
                String::new()
            }
        } else if !turn.final_text.is_empty() {
            turn.final_text.clone()
        } else if !host.streamed_text.is_empty() {
            host.streamed_text.clone()
        } else {
            String::new()
        };

        let mut done_message_id = String::new();
        let mut done_content_order = None;
        let mut done_thinking_order = None;
        let mut done_render_parts = Vec::new();
        if !final_text.is_empty() {
            let thinking_text = final_snapshot.as_ref().and_then(|snapshot| {
                (!snapshot.thinking_text.is_empty()).then_some(snapshot.thinking_text.as_str())
            });
            let thinking_signature = final_snapshot.as_ref().and_then(|snapshot| {
                (!snapshot.thinking_signature.is_empty())
                    .then_some(snapshot.thinking_signature.as_str())
            });
            let text_part = host
                .render_order
                .mark_text(&host.render_scope, "claude-code-final-text");
            let thinking_part = thinking_text.map(|_| {
                host.render_order
                    .mark_thinking(&host.render_scope, "claude-code-final-thinking")
            });
            done_content_order = Some(text_part.seq);
            done_thinking_order = thinking_part.as_ref().map(|part| part.seq);
            done_render_parts = super::assistant_render_parts_for_response(
                run_id,
                Some(text_part),
                &final_text,
                thinking_part,
                thinking_text.unwrap_or_default(),
                None,
                thinking_signature,
                &[],
                &[],
            );
            done_message_id = store.add_message_with_thinking_and_render_parts(
                &self.session_id,
                MessageRole::Assistant,
                &final_text,
                thinking_text,
                None,
                thinking_signature,
                None,
                Some(&host.response_metadata),
                done_content_order,
                done_thinking_order,
                &done_render_parts,
            )?;
            self.partial_assistant.mark_persisted(
                done_message_id.clone(),
                final_text.clone(),
                thinking_text.map(str::to_string),
                None,
            );
            host.last_persisted_assistant_message_id = Some(done_message_id.clone());
        }

        if let (Some(claude_session_id), Some(message_id)) = (
            turn.claude_session_id.as_deref(),
            host.last_persisted_assistant_message_id.as_deref(),
        ) {
            if let Err(error) = store.set_message_cli_session_id(
                &self.session_id,
                message_id,
                CLAUDE_CODE_CLI_PROVIDER,
                claude_session_id,
            ) {
                eprintln!(
                    "[Agent {}] failed to persist Claude Code CLI session id for message {}: {}",
                    self.id, message_id, error
                );
            }
        }

        host.flush_native_checkpoints()?;
        // The response is durable locally. Bulk SSH backup has its own status
        // and must not keep the conversation running after Claude has finished.
        claude_code_cli::archive::schedule(store.clone(),self.session_id.clone(),app_handle.clone());
        store.close_run_pending_input_queue(run_id)?;

        if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(run_id)) {
            eprintln!(
                "[Agent {}] failed to persist latest completed run id for session {} run {}: {}",
                self.id, self.session_id, run_id, error
            );
            crate::error::AppError::emit_background(
                app_handle,
                &crate::error::AppError::new(
                    "session.latest_run_persist_failed",
                    "Latest run boundary may be unavailable for this session.",
                )
                .detail(error)
                .operation("session")
                .severity(crate::error::ErrorSeverity::Warning),
            );
        }

        emit_stream(
            app_handle,
            run_id,
            StreamEvent::Done {
                session_id: self.session_id.clone(),
                message_id: done_message_id,
                full_text: final_text.clone(),
                content_order: done_content_order,
                thinking_order: done_thinking_order,
                render_parts: (!done_render_parts.is_empty()).then_some(done_render_parts),
            },
        );
        self.partial_assistant.reset();

        if let Err(error) = self
            .flush_pending_knowledge_proposal(app_handle, store, run_id)
            .await
        {
            eprintln!(
                "[Agent {}] failed to flush knowledge proposal for session {}: {}",
                self.id, self.session_id, error
            );
        }

        Ok(final_text)
    }
}

fn convert_api_tools_to_claude_code(
    api_tools: &[serde_json::Value],
) -> Vec<ClaudeCodeToolDefinition> {
    api_tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name")?.as_str()?.to_string();
            // Claude's native Read keeps its schema and is executed by Locus.
            if name == "read" {
                return None;
            }
            let description = function
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let input_schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            Some(ClaudeCodeToolDefinition {
                name,
                description,
                input_schema,
                defer_loading: tool.get("defer_loading").and_then(serde_json::Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

fn build_cli_user_message(text: &str, images: Option<&[ImageData]>) -> serde_json::Value {
    if let Some(images) = images {
        if !images.is_empty() {
            let mut blocks: Vec<serde_json::Value> = images
                .iter()
                .map(|image| {
                    serde_json::json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": image.mime_type,
                            "data": image.data,
                        }
                    })
                })
                .collect();
            if !text.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": text,
                }));
            }
            return serde_json::json!({
                "role": "user",
                "content": blocks,
            });
        }
    }

    serde_json::json!({
        "role": "user",
        "content": text,
    })
}

fn equivalent_json_args(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }

    match (
        serde_json::from_str::<serde_json::Value>(left),
        serde_json::from_str::<serde_json::Value>(right),
    ) {
        (Ok(l), Ok(r)) => l == r,
        _ => false,
    }
}

fn sanitize_request_id(request_id: &str) -> String {
    request_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(test)]
#[path = "claude_stream_tests.rs"]
mod stream_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn concurrent_native_results_restore_their_own_pending_round_without_losing_siblings() {
        use super::*;
        let make = |id: &str| PendingAssistantRound {
            message_id: format!("message-{id}"),
            tool_calls: vec![serde_json::from_value(serde_json::json!({"id":id,"name":"WebSearch","arguments":"{}"})).unwrap()],
            remaining: HashSet::from([id.to_string()]),
            confirmation_preapproved: HashSet::from([format!("approval-{id}")]),
            ..Default::default()
        };
        let mut pending = Some(make("d"));
        let mut waiting = vec![make("a"), make("b"), make("c")];
        for id in ["b", "d", "a", "c"] {
            PendingAssistantRound::activate(&mut pending, &mut waiting, id);
            let round = pending.take().unwrap();
            assert_eq!(round.message_id, format!("message-{id}"));
            assert!(round.remaining.contains(id));
            assert!(round.confirmation_preapproved.contains(&format!("approval-{id}")));
        }
        assert!(waiting.is_empty());
        PendingAssistantRound::activate(&mut pending, &mut waiting, "already-completed");
        assert!(pending.is_none());
    }

    #[test]
    fn claude_transport_retry_does_not_resubmit_an_input_with_a_lost_ack() {
        use super::*;
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("lost input ACK", None, None, "chat", None).unwrap();
        let user = store.add_message(&id, MessageRole::User, "Apply the change exactly once").unwrap();
        let remote: claude_code_cli::ssh::SshConfig = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
        let wire = prepare_pending_claude_input(&store, &id, Some(&user), "Apply the change exactly once", &Some(remote.clone()), "").unwrap();
        // Before source confirmation, retry preparation retains the exact token
        // and row, rather than duplicating the user request in local history.
        assert_eq!(wire, prepare_pending_claude_input(&store, &id, Some(&user), "Apply the change exactly once", &Some(remote.clone()), "").unwrap());
        let mut native: claude_code_cli::recovery::NativeSnapshot = serde_json::from_value(json!({
            "version":1,"nativeId":uuid::Uuid::new_v4(),"digest":"a".repeat(64),
            "messages":[{"key":"accepted-user","role":"user","uuids":[uuid::Uuid::new_v4()],"content":wire["content"]}],
            "todos":[],"pendingToolIds":[],"incompleteTail":false,
        })).unwrap();
        store.reconcile_claude_native(&id, &remote, &native, &store.get_messages(&id).unwrap(), Some(&user)).unwrap();
        let resumed = prepare_pending_claude_input(&store, &id, Some(&user), "Apply the change exactly once", &Some(remote.clone()), "").unwrap();
        assert_ne!(resumed["locusMessageId"], user);
        assert!(!resumed["content"].as_str().unwrap().contains("Apply the change exactly once"));
        assert!(resumed["content"].as_str().unwrap().contains("Continue from the existing conversation"));
        // A second disconnect before consuming the continuation reuses it.
        assert_eq!(resumed, prepare_pending_claude_input(&store, &id, Some(&user), "Apply the change exactly once", &Some(remote.clone()), "").unwrap());
        assert_eq!(store.get_messages(&id).unwrap().len(), 2);
        let mut resumed = resumed;
        for index in 0..2 {
            let messages = store.get_messages(&id).unwrap();
            let continuation = messages.iter().find(|m| Some(m.id.as_str()) == resumed["locusMessageId"].as_str()).unwrap();
            assert!(continuation.content.is_empty());
            assert!(claude_code_cli::recovery::is_continuation_prompt(continuation.prompt_suffix.as_deref().unwrap()));
            let view = store.load_session_view(&id, 100).unwrap();
            assert_eq!(view.user_message_ids, vec![user.clone()]);
            assert_eq!(view.session.messages.len(), 1);

            // The continuation itself was accepted, but the transport failed
            // again. Authoritative recovery must retain its hidden identity.
            let mut accepted = native.messages[0].clone();
            accepted.key = format!("accepted-continuation-{index}");
            accepted.uuids = vec![uuid::Uuid::new_v4().to_string()];
            accepted.content = resumed["content"].clone();
            native.messages.push(accepted);
            native.digest = format!("{:064x}", index + 1);
            store.rebuild_claude_native_for_retry(&id, &remote, &native, &messages, Some(&user)).unwrap();
            let recovered = store.get_messages(&id).unwrap();
            let continuation = recovered.iter().find(|m| Some(m.id.as_str()) == resumed["locusMessageId"].as_str()).unwrap();
            assert!(continuation.content.is_empty());
            assert!(claude_code_cli::recovery::is_continuation_prompt(continuation.prompt_suffix.as_deref().unwrap()));
            assert_eq!(store.load_session_view(&id, 100).unwrap().user_message_ids, vec![user.clone()]);
            let next = prepare_pending_claude_input(&store, &id, Some(&user), "Apply the change exactly once", &Some(remote.clone()), "").unwrap();
            assert_ne!(next["locusMessageId"], resumed["locusMessageId"]);
            assert!(!next["content"].as_str().unwrap().contains("Apply the change exactly once"));
            resumed = next;
        }
    }

    #[tokio::test]
    async fn native_cli_catalog_omits_aliases_without_revoking_local_execution() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let mut instance = super::super::tests::native_plan_test_instance(&root);
        instance.backend = crate::agent::instance::LlmBackend::ClaudeCodeCli;
        instance.refresh_lazy_tool_renderer(crate::config::DynamicToolLoadingMode::Native, true);
        let api = instance.build_cli_api_tools(crate::config::DynamicToolLoadingMode::Native, &HashSet::new()).await;
        assert!(api.iter().all(|tool| claude_code_cli::native_tools::native_equivalent_for_locus_tool(tool["function"]["name"].as_str().unwrap()).is_none()));
        assert!(!api.iter().any(|tool| tool["function"]["name"] == "web_fetch"));
        let allowed = instance.allowed_tool_set_for_active_skills(&HashSet::new()).await;
        assert!(allowed.contains("edit"), "the native Edit adapter still needs local edit authorization");
        assert!(allowed.contains("read"));
        // Existing compatibility renderers are not silently migrated.
        instance.refresh_lazy_tool_renderer(crate::config::DynamicToolLoadingMode::MetaTool, true);
        let legacy = instance.build_cli_api_tools(crate::config::DynamicToolLoadingMode::MetaTool, &HashSet::new()).await;
        assert!(legacy.iter().any(|tool| tool["function"]["name"] == "edit"));
    }

    #[tokio::test]
    async fn claude_cli_web_fetch_is_unavailable_in_all_loading_modes_and_meta_calls() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let mut instance = super::super::tests::native_plan_test_instance(&root);
        instance.backend = crate::agent::instance::LlmBackend::ClaudeCodeCli;
        let active = HashSet::from(["web_fetch".to_string()]);
        for mode in [crate::config::DynamicToolLoadingMode::Native, crate::config::DynamicToolLoadingMode::Direct, crate::config::DynamicToolLoadingMode::MetaTool] {
            instance.refresh_lazy_tool_renderer(mode, true);
            assert!(!instance.is_allowed_tool_for_active_skills("web_fetch", &active).await);
            assert!(instance.build_cli_api_tools(mode, &active).await.iter().all(|tool| tool["function"]["name"] != "web_fetch"));
            assert!(!instance.lazy_tool_manifest_prompt().await.unwrap_or_default().contains("`web_fetch`"));
            let loaded = instance.execute_tool_load_with_mode_and_skills(&serde_json::json!({"tools":["web_fetch"]}), mode, &active).await;
            let value: serde_json::Value = serde_json::from_str(&loaded.output).unwrap();
            assert_eq!(value["tools"][0]["status"], "not_allowed");
            assert!(value["tools"][0].get("parameters").is_none());
        }
        // The same Locus tool remains usable by API backends.
        instance.backend = crate::agent::instance::LlmBackend::OpenRouter { api_key: String::new(), base_url: None };
        assert!(instance.is_allowed_tool_for_active_skills("web_fetch", &active).await);
    }

    #[test]
    fn claude_notifications_keep_the_real_user_identity_and_retry_payload() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(root.path()).unwrap());
        let id = store.create_session("notify input", None, None, "chat", None).unwrap();
        let user = store.add_message(&id, MessageRole::User, "先不要继续跑测试，汇报修改").unwrap();
        let manager = crate::async_tasks::AsyncTaskManager::new(store.clone()).unwrap();
        let task = manager.create_task(&id, "unity_test_run", true);
        manager.prepare_task(&task.task_id, None).unwrap();
        manager.finish(&task.task_id, &crate::tool::ToolResult { output: "Task cancelled.".into(), is_error: true });
        let reminders = manager.deliver_notifications(&id, &store).unwrap();
        assert_eq!(reminders.len(), 1);
        assert_ne!(store.get_messages(&id).unwrap().last().unwrap().id, user);
        ensure_claude_history_origin(&store, &id, Some(&user)).unwrap();
        let wire = prepare_pending_claude_input(&store, &id, Some(&user), "先不要继续跑测试，汇报修改", &None, "").unwrap();
        assert_eq!(wire["locusMessageId"], user);
        assert!(wire["locusPendingUsers"].as_array().unwrap().is_empty());
        let text = wire["content"].as_str().unwrap();
        assert!(text.contains("先不要继续跑测试，汇报修改"));
        assert_eq!(text.matches("Task cancelled.").count(), 1);
        let retry = prepare_pending_claude_input(&store, &id, Some(&user), "先不要继续跑测试，汇报修改", &None, "").unwrap();
        assert_eq!(wire, retry);
        assert_eq!(store.get_messages(&id).unwrap().len(), 2);
    }

    #[test]
    fn claude_prompt_removes_only_repeated_environment_and_continuation_reminders() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("native prompt", None, None, "chat", None).unwrap();
        let environment = "<system-reminder>\nLocal tool host: Windows, F:/project\n</system-reminder>\n";
        let attachment = "<system-reminder>\nAttached file context\n</system-reminder>";
        let content = "Keep my literal <system-reminder>Local tool host: Windows, F:/project</system-reminder> text.";
        store.add_message_with_images_asset_refs_and_signature(&id, MessageRole::User, content,
            None, None, None, Some(environment), Some(attachment)).unwrap();
        let message = store.get_messages(&id).unwrap().remove(0);
        let system = "Locus Agent rules\nLocal tool host: Windows, F:/project";
        let prompt = claude_user_prompt(&message, Some(attachment), system);
        assert_eq!(prompt, format!("{content}{attachment}"));
        assert_eq!(store.get_messages(&id).unwrap()[0].prompt_prefix.as_deref(), Some(environment));
        let unrelated = claude_user_prompt(&message, Some("Continue"), "Other system context");
        assert!(unrelated.starts_with(environment.trim_end()));
        assert!(unrelated.ends_with("Continue"));
        assert_eq!(unrelated.matches("Attached file context").count(), 1);
    }

    #[test]
    fn claude_pending_batch_keeps_order_attachments_and_retries_without_duplicate_rows() {
        use super::*;
        use serde_json::json;
        let root=tempfile::tempdir().unwrap(); let store=SessionStore::new(root.path()).unwrap();
        let id=store.create_session("pending batch",None,None,"chat",None).unwrap();
        store.set_session_last_model_id(&id,"claude_code/opus").unwrap();
        let mut ids=Vec::new();
        for (i,text) in ["先不要直接合并，向我汇报方案","向我汇报方案"].iter().enumerate() {
            let run=format!("failed-{i}"); store.try_start_run(&id,&run).unwrap();
            let image=ImageData{data:format!("image-{i}"),mime_type:"image/png".into(),capture:None};
            let message=store.add_message_with_images_asset_refs_and_signature(&id,MessageRole::User,text,
                Some(&[image]),None,None,None,Some("saved attachment context")).unwrap();
            store.append_session_event(&id,&run,"userMessage",&json!({"message":{"id":message}}).to_string()).unwrap();
            store.update_run_status(&run,"error",Some("before send")).unwrap(); ids.push(message);
        }
        store.try_start_run(&id,"retry").unwrap();
        ensure_claude_history_origin(&store,&id,Some(&ids[1])).unwrap();
        let wire=prepare_pending_claude_input(&store,&id,Some(&ids[1]),"向我汇报方案",&None,"").unwrap();
        assert_eq!(wire["role"],"user"); assert_eq!(wire["locusMessageId"],ids[1]);
        assert_eq!(wire["locusPendingUsers"][0]["id"],ids[0]);
        assert!(wire["locusPendingUsers"][0]["text"].as_str().unwrap().contains("先不要直接合并"));
        assert!(wire["locusPendingUsers"][0]["text"].as_str().unwrap().contains("saved attachment context"));
        assert_eq!(wire["content"][0]["source"]["data"],"image-0");
        assert_eq!(wire["content"][1]["source"]["data"],"image-1");
        let mut metadata=store.get_response_request_metadata(&id).unwrap();
        // Earlier text was appended, but the final image-bearing prompt was
        // interrupted. Keep the whole pending batch until that carrier arrives.
        let native_id = uuid::Uuid::new_v4().to_string();
        metadata.get_mut(&ids[0]).unwrap()["claude_checkpoint"]=json!({"version":1,"uuid":uuid::Uuid::new_v4(),"sessionId":native_id});
        store.set_message_response_request_metadata(&id,&ids[0],&metadata[&ids[0]]).unwrap();
        store.set_message_cli_session_id(&id,&ids[0],"claude_code",&native_id).unwrap();
        assert_eq!(store.claude_recovery_pending_messages(&id,&store.get_messages(&id).unwrap(),&metadata,None).unwrap(),ids);
        let retry=prepare_pending_claude_input(&store,&id,Some(&ids[1]),"向我汇报方案",&None,"").unwrap();
        assert_eq!(wire,retry);
        assert_eq!(store.get_messages(&id).unwrap().len(),2);
        store.update_run_status("retry","error",Some("retry before submission")).unwrap();
        assert!(store.session_resume_available(&id).unwrap());
        metadata.get_mut(&ids[1]).unwrap()["claude_checkpoint"]=json!({"version":1,"uuid":uuid::Uuid::new_v4(),"sessionId":native_id});
        store.set_message_response_request_metadata(&id,&ids[1],&metadata[&ids[1]]).unwrap();
        store.set_message_cli_session_id(&id,&ids[1],"claude_code",&native_id).unwrap();
        assert!(store.claude_recovery_pending_messages(&id,&store.get_messages(&id).unwrap(),&metadata,None).unwrap().is_empty());
        assert!(!store.session_resume_available(&id).unwrap());
        let resumed=prepare_pending_claude_input(&store,&id,Some(&ids[1]),"向我汇报方案",&None,"").unwrap();
        assert!(!resumed.to_string().contains("向我汇报方案"));
        assert!(resumed["locusPendingUsers"].as_array().unwrap().is_empty());
        assert_ne!(resumed["locusMessageId"],ids[1]);
    }

    #[test]
    fn claude_resume_sends_saved_user_text_and_attachments_after_starting_retry() {
        use super::*;
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("resume",None,None,"chat",None).unwrap();
        let old = store.add_message(&id,MessageRole::Assistant,"already completed work").unwrap();
        store.set_message_response_request_metadata(&id,&old,&json!({"backend":"claude_code_cli"})).unwrap();
        store.try_start_run(&id,"failed").unwrap();
        let image = ImageData { data:"aQ==".into(),mime_type:"image/png".into(),capture:None };
        let pending = store.add_message_with_images_asset_refs_and_signature(&id,MessageRole::User,
            "先不要直接合并，向我汇报方案",Some(&[image.clone()]),None,Some("saved intent"),
            Some("workspace context"),Some("attached file context")).unwrap();
        store.append_session_event(&id,"failed","userMessage",&json!({"message":{"id":pending}}).to_string()).unwrap();
        store.update_run_status("failed","error",Some("failed before submission")).unwrap();
        store.try_start_run(&id,"retry").unwrap();

        for _ in 0..2 {
            let (message_id, reused) = persist_turn_user_message(&store,&id,true,|| {
                store.add_message(&id,MessageRole::User,"")
            }).unwrap();
            assert!(reused);
            assert_eq!(message_id,pending);
            let message = pending_claude_resume_message(&store,&id).unwrap().unwrap();
            assert_eq!(message.id,pending);
            let prompt = claude_user_prompt(&message,Some("<system-reminder>Continue</system-reminder>"),"");
            let wire = build_cli_user_message(&prompt,message.images.as_deref());
            assert_eq!(wire["role"],"user");
            assert_eq!(wire["content"][0]["source"]["data"],image.data);
            let text = wire["content"][1]["text"].as_str().unwrap();
            for expected in ["先不要直接合并，向我汇报方案","workspace context","attached file context","<system-reminder>Continue"] {
                assert!(text.contains(expected),"{expected}");
            }
            assert!(!text.contains("already completed work"));
            ensure_claude_history_origin(&store,&id,Some(&message.id)).unwrap();
            assert_eq!(store.get_messages(&id).unwrap().len(),2);
            assert_eq!(store.get_messages(&id).unwrap().last().unwrap().thinking_signature.as_deref(),Some("saved intent"));
        }
        // Once the native user checkpoint exists, continuing must not resend it.
        store.set_message_response_request_metadata(&id,&pending,&json!({"backend":"claude_code_cli",
            "claude_checkpoint":{"uuid":uuid::Uuid::new_v4(),"sessionId":uuid::Uuid::new_v4()}})).unwrap();
        assert!(pending_claude_resume_message(&store,&id).unwrap().is_none());
    }

    #[test]
    fn claude_cli_refuses_api_or_mixed_history() {
        let root = tempfile::tempdir().unwrap();
        let store = super::SessionStore::new(root.path()).unwrap();
        let id = store.create_session("origin", None, None, "chat", None).unwrap();
        let user = store.add_message(&id, super::MessageRole::User, "new input").unwrap();
        assert!(super::ensure_claude_history_origin(&store, &id, Some(&user)).is_ok());
        store.set_message_response_request_metadata(&id, &user, &serde_json::json!({"backend":"claude_code_cli"})).unwrap();
        let assistant = store.add_message(&id, super::MessageRole::Assistant, "API answer").unwrap();
        let next = store.add_message(&id, super::MessageRole::User, "continue").unwrap();
        assert!(super::ensure_claude_history_origin(&store, &id, Some(&next)).is_err());
        store.set_message_response_request_metadata(&id, &assistant, &serde_json::json!({"backend":"claude_code_cli"})).unwrap();
        assert!(super::ensure_claude_history_origin(&store, &id, Some(&next)).is_ok());
        store.add_message(&id, super::MessageRole::Assistant, "foreign answer").unwrap();
        assert!(super::ensure_claude_history_origin(&store, &id, Some(&next)).is_err());
    }

    #[test]
    fn response_timing_ends_at_model_stop_not_tool_completion() {
        let timing = serde_json::json!({"version":1,"source":"claude-turn-step","requestStartedAtMs":100,"firstOutputAtMs":300,"responseStoppedAtMs":1100,"resultReadyAtMs":21100});
        assert_eq!(super::claude_response_duration_ms(&timing), 1000);
        for invalid in [serde_json::Value::Null, serde_json::json!({"version":1}), {
            let mut value=timing.clone();value["responseStoppedAtMs"]=serde_json::json!(99);value
        }, {
            let mut value=timing.clone();value["responseStoppedAtMs"]=serde_json::Value::Null;value
        }] { assert_eq!(super::claude_response_duration_ms(&invalid), 0); }
    }
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn native_read_adapter_failure_is_persisted_as_error_before_completion() {
        use claude_code_cli::{native_scope::NativeInvocation, native_tools};
        use serde_json::json;
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let session_id = store.create_session("native error", None, None, "chat", None).unwrap();
        store.try_start_run(&session_id, "native-error-run").unwrap();
        let call = native_tools::adapt("native-error", "Read", &json!({"file_path":"large.txt"})).unwrap();
        let scope = NativeInvocation::new(call.clone(), session_id.clone());
        let mut result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult {
            output: "<persisted-output>Full output saved; use a smaller read range.</persisted-output>".into(),
            is_error: false,
        });
        let mut stored_output = result.output.clone();
        scope.run(async {
            validate_native_tool_result(&call.id, &mut result, &mut stored_output);
        }).await;
        assert!(result.is_error);
        assert_eq!(result.outcome.as_stream_outcome(), ToolCallOutcome::Error);
        assert_eq!(result.output, stored_output);
        let wire = native_tools::finish_result(&call, &ClaudeCodeToolResult {
            output: stored_output.clone(), is_error: result.is_error, images: result.images.clone(), context: Vec::new(),
        }, scope.result());
        assert_eq!(wire["deny"], stored_output);
        let tool_call: ToolCallInfo = serde_json::from_value(json!({
            "id":call.id,"name":"read","arguments":call.locus_input.to_string(),
        })).unwrap();
        assert_eq!(finalize_tool_call_record(&tool_call, Some(&result)).outcome, Some(ToolCallOutcome::Error));
        save_claude_code_tool_result(&store, &session_id, "native-error-run", &call.id, &stored_output, None)
            .unwrap().unwrap();
        let messages = store.get_messages_for_prompt(&session_id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, wire["deny"].as_str().unwrap());
    }

    #[tokio::test]
    async fn native_validation_preserves_skill_success_structured_results_and_interruption() {
        use claude_code_cli::{native_scope::NativeInvocation, native_tools};
        use serde_json::json;
        let call = native_tools::adapt("native-read", "Read", &json!({"file_path":"SKILL.md"})).unwrap();
        let scope = NativeInvocation::new(call.clone(), "test-session".into());
        let output = "<content>\n1\t# Workflow\n\n(End of file — 1 lines total)\n</content>\n\nLoaded Skill document tools for the next step: read";
        let mut result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output: output.into(), is_error: false });
        let mut stored = output.to_string();
        scope.run(async { validate_native_tool_result(&call.id, &mut result, &mut stored); }).await;
        assert!(!result.is_error);
        assert_eq!(result.outcome.as_stream_outcome(), ToolCallOutcome::Done);
        assert_eq!(stored, output);

        for (output, is_error) in [
            (crate::session::history::INTERRUPTED_TOOL_RESULT, false),
            ("Permission denied", true),
        ] {
            let mut result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output: output.into(), is_error });
            let outcome = result.outcome;
            let mut stored = output.to_string();
            scope.run(async { validate_native_tool_result(&call.id, &mut result, &mut stored); }).await;
            assert_eq!(result.outcome, outcome);
            assert_eq!(stored, output);
        }

        let call = native_tools::adapt("native-glob", "Glob", &json!({"pattern":"*.md"})).unwrap();
        let scope = NativeInvocation::new(call.clone(), "test-session".into());
        scope.set_result(json!({"filenames":["SKILL.md"],"numFiles":1}));
        let mut result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output: "structured search output".into(), is_error: false });
        let mut stored = result.output.clone();
        scope.run(async { validate_native_tool_result(&call.id, &mut result, &mut stored); }).await;
        assert!(!result.is_error);
        assert_eq!(stored, "structured search output");

        // Ordinary/MCP tools are not subject to Claude's native result schema.
        let mut result = ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output: "ordinary output".into(), is_error: false });
        let mut stored = result.output.clone();
        validate_native_tool_result("mcp-read", &mut result, &mut stored);
        assert!(!result.is_error);
        assert_eq!(stored, "ordinary output");
    }

    #[tokio::test]
    async fn persisted_native_search_bounds_the_actual_cli_payload() {
        use claude_code_cli::{native_scope::NativeInvocation, native_tools, native_output};
        use serde_json::json;
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let session = store.create_session("bounded native", None, None, "chat", None).unwrap();
        let call = native_tools::adapt("large-grep", "Grep", &json!({"pattern":"x","output_mode":"content"})).unwrap();
        let scope = NativeInvocation::new(call.clone(), session.clone());
        let raw = json!({"mode":"content","content":"x".repeat(40_000),"filenames":["a.txt"],"numFiles":1,"totalFiles":1,"numMatches":1,"numLines":1,"appliedOffset":0,"appliedLimit":200});
        let output = native_output::render("Grep", &raw);
        scope.set_result(raw);
        let stored = store.rewrite_tool_result_for_storage(&session, &call.id, "grep", &output).unwrap();
        assert!(stored.starts_with("<persisted-output>"));
        scope.run(async { bound_native_tool_result(&call.id, &stored); }).await;
        let result: ClaudeCodeToolResult = crate::tool::ToolResult { output:stored.clone(), is_error:false }.into();
        let reply = native_tools::finish_result(&call, &result, scope.result());
        assert!(reply.to_string().len() < 10_000);
        assert_eq!(reply["result"]["content"], stored);
        assert_eq!(reply["result"]["totalFiles"], 1);
        assert_eq!(reply["context"], json!([stored]));
    }

    #[test]
    fn claude_code_tool_result_uses_stored_output() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Claude Code CLI Tool Result", None, None, "chat", None)
            .expect("create session");
        store
            .try_start_run(&session_id, "run-claude-code")
            .expect("start run");
        let full_output = "C".repeat(31_000);
        let stored_output = store
            .rewrite_tool_result_for_storage(&session_id, "tc-claude-code", "bash", &full_output)
            .expect("rewrite tool output");

        save_claude_code_tool_result(
            &store,
            &session_id,
            "run-claude-code",
            "tc-claude-code",
            &stored_output,
            None,
        )
        .expect("save Claude Code tool result")
        .expect("current run");

        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        assert_eq!(prompt_messages.len(), 1);
        assert_eq!(prompt_messages[0].role, MessageRole::Tool);
        assert!(prompt_messages[0].content.starts_with("<persisted-output>"));
        assert!(!prompt_messages[0].content.contains(&full_output));
    }
}
