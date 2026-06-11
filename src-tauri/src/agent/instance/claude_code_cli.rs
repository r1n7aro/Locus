use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tauri::AppHandle;

use super::{
    emit_parent_stream, emit_stream, finalize_tool_call_record, normalize_tool_args, AgentInstance,
    AssistantStreamState, ExecutedToolResult, StreamRenderOrderTracker,
};
use crate::commands::{StreamEvent, ToolCallOutcome};
use crate::llm::claude_code_cli::{
    self, ClaudeCodeAssistantMessage, ClaudeCodeCliOptions, ClaudeCodeHost, ClaudeCodeHostFuture,
    ClaudeCodeToolDefinition, ClaudeCodeToolResult,
};
use crate::session::models::{ImageData, MessageRole, ToolCallInfo};
use crate::session::store::SessionStore;

const CLAUDE_CODE_CLI_PROVIDER: &str = "claude_code";

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
}

struct CliRoundCompletion {
    message_id: String,
    undo_guard: Option<crate::vcs::undo::UndoRoundGuard>,
    has_unity_execute: bool,
    queued_unity_asset_paths: Vec<String>,
}

struct ClaudeCodeRoundHost<'a> {
    agent: &'a AgentInstance,
    app_handle: &'a AppHandle,
    store: &'a SessionStore,
    run_id: &'a str,
    mode: &'a str,
    active_skill_tool_names: &'a HashSet<String>,
    streamed_text: String,
    partial_assistant: Arc<AssistantStreamState>,
    started_tool_calls: VecDeque<(String, String)>,
    known_tool_calls: VecDeque<ToolCallInfo>,
    completed_tool_ids: HashSet<String>,
    pending_round: Option<PendingAssistantRound>,
    last_assistant: Option<ClaudeCodeAssistantMessage>,
    last_persisted_assistant_message_id: Option<String>,
    render_order: StreamRenderOrderTracker,
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

impl<'a> ClaudeCodeRoundHost<'a> {
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
                    id: format!("{}:text:claude-code-round", self.run_id),
                    seq,
                }),
                &round.text,
                round.thinking_order.map(|seq| super::RenderPartMark {
                    id: format!("{}:thinking:claude-code-round", self.run_id),
                    seq,
                }),
                &round.thinking_text,
                None,
                (!round.thinking_signature.is_empty()).then_some(round.thinking_signature.as_str()),
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
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
            order: None,
        }
    }

    fn emit_undo_checkpoint_warning(&self, error: &str) {
        let lower = error.to_ascii_lowercase();
        let message = if lower.contains("unable to index file 'nul'")
            || lower.contains("short read while indexing nul")
        {
            "Undo is unavailable for this round because Git could not snapshot the workspace. Remove or rename reserved Windows file names such as NUL in the repository."
        } else {
            "Undo may be unavailable for this round because the workspace snapshot failed."
        };
        crate::error::AppError::emit_background(
            self.app_handle,
            &crate::error::AppError::new("undo.checkpoint_failed", message)
                .detail(error.to_string())
                .operation("undo")
                .severity(crate::error::ErrorSeverity::Warning),
        );
    }

    async fn ensure_cli_round_undo_checkpoint(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        if !self.agent.tool_call_needs_undo_tracking(tool_name, args) {
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
            if tool_call.name == "unity_execute" || tool_call.name == "unity_run_states" {
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

        if !crate::unity_bridge::is_unity_project(&self.agent.working_dir)
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
        let Some(asset_path) = self
            .agent
            .unity_asset_relative_path(tool_call, args, result)
        else {
            return;
        };
        if let Some(round) = self.pending_round.as_mut() {
            round.queued_unity_asset_paths.push(asset_path);
        }
    }

    fn take_cli_round_completion_if_ready(&mut self) -> Option<CliRoundCompletion> {
        let round = self.pending_round.as_mut()?;
        if !round.remaining.is_empty() {
            return None;
        }
        Some(CliRoundCompletion {
            message_id: round.message_id.clone(),
            undo_guard: round.undo_guard.take(),
            has_unity_execute: round.has_unity_execute,
            queued_unity_asset_paths: std::mem::take(&mut round.queued_unity_asset_paths),
        })
    }

    async fn finish_cli_round_external_side_effects(&self, completion: CliRoundCompletion) {
        if !completion.queued_unity_asset_paths.is_empty() {
            crate::unity_bridge::import_assets_fire_and_forget(
                &self.agent.working_dir,
                completion.queued_unity_asset_paths,
            );
        }

        let Some(undo_guard) = completion.undo_guard else {
            return;
        };
        let Some(undo_mgr) = self.agent.undo_manager.as_ref() else {
            return;
        };
        let recorded = undo_mgr
            .after_round(
                &self.agent.session_id,
                &completion.message_id,
                Some(self.run_id),
                undo_guard,
                completion.has_unity_execute,
                &self.agent.working_dir,
            )
            .await;
        match recorded {
            Ok(true) => {
                if let Some(entry) = undo_mgr
                    .find_entry(&self.agent.session_id, &completion.message_id)
                    .await
                {
                    if AgentInstance::changed_files_touch_view_tree(&entry.changed_files) {
                        crate::view::emit_view_tree_changed(self.app_handle);
                    }
                }
                eprintln!(
                    "[Agent {}] emitting Claude Code CLI UndoAvailable for session {} run {} message {}",
                    self.agent.id, self.agent.session_id, self.run_id, completion.message_id
                );
                emit_stream(
                    self.app_handle,
                    self.run_id,
                    StreamEvent::UndoAvailable {
                        session_id: self.agent.session_id.clone(),
                        assistant_message_id: completion.message_id,
                    },
                );
            }
            Ok(false) => {}
            Err(error) => eprintln!(
                "[Agent {}] failed to record Claude Code CLI undo state for session {} message {}: {}",
                self.agent.id, self.agent.session_id, completion.message_id, error
            ),
        }
    }
}

impl<'a> ClaudeCodeHost for ClaudeCodeRoundHost<'a> {
    fn on_text_delta(&mut self, delta: String) {
        let mark = self
            .render_order
            .mark_text(self.run_id, "claude-code-stream-text");
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
            .mark_thinking(self.run_id, "claude-code-stream-thinking");
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

    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> {
        if message.tool_calls.is_empty() {
            self.last_assistant = Some(message);
            return Ok(());
        }

        if let Some(existing) = self.pending_round.take() {
            if !existing.remaining.is_empty() {
                eprintln!(
                    "[Agent {}] Claude Code CLI emitted a new tool round before the previous one finished",
                    self.agent.id
                );
            }
        }

        let text_part = (!message.text.is_empty()).then(|| {
            self.render_order
                .mark_text(self.run_id, "claude-code-round-text")
        });
        let thinking_part = (!message.thinking_text.is_empty()).then(|| {
            self.render_order
                .mark_thinking(self.run_id, "claude-code-round-thinking")
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
            None,
            content_order,
            thinking_order,
            &render_parts,
        )?;
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
        });
        self.last_persisted_assistant_message_id = Some(message_id);
        self.streamed_text.clear();
        self.maybe_finish_pending_round();
        Ok(())
    }

    fn execute_tool<'b>(
        &'b mut self,
        request_id: &'b str,
        tool_name: &'b str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'b> {
        Box::pin(async move {
            let mut tool_call = self.take_matching_tool_call(request_id, tool_name, &arguments);
            let mut args_for_exec = arguments.clone();
            if tool_call.name == "tool_call" {
                match super::parse_meta_tool_call_arguments(&tool_call.arguments) {
                    Ok((target_name, mut target_args)) => {
                        let allowed = self
                            .agent
                            .allowed_tool_set_for_active_skills(self.active_skill_tool_names)
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

            self.ensure_cli_round_undo_checkpoint(&tool_call.name, &args_for_exec)
                .await;
            self.prepare_cli_unity_tool(&tool_call, &args_for_exec)
                .await;

            let result = self
                .agent
                .execute_single_tool(
                    self.app_handle,
                    self.store,
                    &tool_call,
                    &args_for_exec,
                    self.run_id,
                    self.mode,
                    self.active_skill_tool_names,
                )
                .await;

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

            let stored_output = match self.store.rewrite_tool_result_for_storage(
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

            match save_claude_code_tool_result(
                self.store,
                &self.agent.session_id,
                self.run_id,
                &tool_call.id,
                &stored_output,
                result.images.as_deref(),
            ) {
                Ok(Some(_)) => {}
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
            let completed_round = self.take_cli_round_completion_if_ready();
            self.maybe_finish_pending_round();
            if let Some(completed_round) = completed_round {
                self.finish_cli_round_external_side_effects(completed_round)
                    .await;
            }

            ClaudeCodeToolResult {
                output: result.output,
                is_error: result.is_error,
                images: result.images,
            }
        })
    }
}

impl AgentInstance {
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
    ) -> Result<String, String> {
        let dynamic_tool_loading_mode = self.dynamic_tool_loading_mode(app_handle);
        if dynamic_tool_loading_mode == crate::config::DynamicToolLoadingMode::Direct {
            let messages = store.get_messages_for_prompt(&self.session_id)?;
            self.seed_loaded_tools_from_history(&messages).await;
        }
        let request_tools = self
            .build_cli_request_tool_names(dynamic_tool_loading_mode, active_skill_tool_names)
            .await;
        let api_tools = self.build_api_tools(&request_tools).await;

        // The persisted per-message session id is authoritative: it follows
        // Locus history surgery (rollback, regenerate, message deletion),
        // while the in-process cache may still point at a Claude session that
        // contains rolled-back turns. Only fall back to the cache when the
        // store cannot be read at all, and keep the cache mirroring the
        // authoritative value so the degraded path stays close to correct.
        let resume_session_id =
            match store.latest_cli_session_id(&self.session_id, CLAUDE_CODE_CLI_PROVIDER) {
                Ok(persisted) => {
                    match persisted.as_deref() {
                        Some(session_id) => {
                            claude_code_cli::store_session_id(&self.session_id, session_id).await;
                        }
                        None => {
                            claude_code_cli::clear_session_id(&self.session_id).await;
                        }
                    }
                    persisted
                }
                Err(error) => {
                    eprintln!(
                        "[Agent {}] failed to load persisted Claude Code CLI session id for {}: {}",
                        self.id, self.session_id, error
                    );
                    claude_code_cli::cached_session_id(&self.session_id).await
                }
            };
        if resume_session_id.is_none() && store.get_messages_for_prompt(&self.session_id)?.len() > 1
        {
            eprintln!(
                "[Agent {}] Claude Code CLI session '{}' has no cached Claude session id; existing Locus history will not be replayed",
                self.id, self.session_id
            );
        }

        let user_message = build_cli_user_message(prompt_text, images);
        let tools = convert_api_tools_to_claude_code(&api_tools);
        let mut host = ClaudeCodeRoundHost {
            agent: self,
            app_handle,
            store,
            run_id,
            mode: initial_mode,
            active_skill_tool_names,
            streamed_text: String::new(),
            partial_assistant: self.partial_assistant_state(),
            started_tool_calls: VecDeque::new(),
            known_tool_calls: VecDeque::new(),
            completed_tool_ids: HashSet::new(),
            pending_round: None,
            last_assistant: None,
            last_persisted_assistant_message_id: None,
            render_order: StreamRenderOrderTracker::default(),
        };

        let options = ClaudeCodeCliOptions {
            locus_session_id: self.session_id.clone(),
            cwd: if self.has_selected_working_dir() {
                self.working_dir.clone()
            } else {
                std::env::current_dir()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            },
            system_prompt: system_prompt.to_string(),
            model: self.effective_model.clone(),
            effort: self.effort.clone(),
            resume_session_id,
            server_name: "locus".to_string(),
            tools,
            debug: self.debug,
        };
        let mut cancel_rx = self.cancel_waiter();
        let turn = tokio::select! {
            result = claude_code_cli::run_turn(options, user_message, &mut host) => result?,
            _ = cancel_rx.changed() => {
                eprintln!(
                    "[Agent {}] Claude Code CLI turn cancelled before completion: session={} run={}",
                    self.id, self.session_id, run_id
                );
                self.clear_pending_knowledge_proposal(app_handle).await;
                self.emit_cancelled(app_handle, store, run_id, None);
                return Ok(String::new());
            }
        };

        if let Some(claude_session_id) = turn.claude_session_id.as_deref() {
            claude_code_cli::store_session_id(&self.session_id, claude_session_id).await;
        }

        if turn.input_tokens > 0
            || turn.output_tokens > 0
            || turn.cache_read_tokens > 0
            || turn.cache_write_tokens > 0
        {
            let context_tokens = turn.input_tokens
                + turn.output_tokens
                + turn.cache_read_tokens
                + turn.cache_write_tokens;
            let context_limit = super::model_context_limit(&self.effective_model);
            match store.record_token_usage(
                &self.session_id,
                turn.input_tokens as u64,
                turn.output_tokens as u64,
                turn.cache_read_tokens as u64,
                turn.cache_write_tokens as u64,
                turn.cost_usd,
                0,
                Some(context_tokens),
                Some(context_limit),
            ) {
                Ok(totals) => {
                    emit_stream(
                        app_handle,
                        run_id,
                        StreamEvent::UsageUpdate {
                            session_id: self.session_id.clone(),
                            input_tokens: turn.input_tokens,
                            output_tokens: turn.output_tokens,
                            cache_read_tokens: turn.cache_read_tokens,
                            cache_write_tokens: turn.cache_write_tokens,
                            total_input_tokens: totals.total_input_tokens,
                            total_output_tokens: totals.total_output_tokens,
                            total_cache_read_tokens: totals.total_cache_read_tokens,
                            total_cache_write_tokens: totals.total_cache_write_tokens,
                            total_cost_usd: totals.total_cost_usd,
                            priced_rounds: totals.priced_rounds,
                            context_tokens,
                            context_limit,
                        },
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[Agent {}] failed to record Claude Code CLI token usage: {}",
                        self.id, err
                    );
                }
            }
        }

        {
            let round = super::RawRound {
                round: 1,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                request: serde_json::json!({
                    "backend": "claude_code_cli",
                    "session_id": self.session_id,
                    "claude_session_id": turn.claude_session_id,
                    "raw_stdin": turn.raw_request,
                }),
                response: turn.raw_response.clone(),
            };
            self.raw_store
                .lock()
                .await
                .entry(self.session_id.clone())
                .or_insert_with(Vec::new)
                .push(round);
        }

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
                .mark_text(run_id, "claude-code-final-text");
            let thinking_part = thinking_text.map(|_| {
                host.render_order
                    .mark_thinking(run_id, "claude-code-final-thinking")
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
            );
            done_message_id = store.add_message_with_thinking_and_render_parts(
                &self.session_id,
                MessageRole::Assistant,
                &final_text,
                thinking_text,
                None,
                thinking_signature,
                None,
                None,
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
mod tests {
    use super::*;
    use tempfile::tempdir;

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
