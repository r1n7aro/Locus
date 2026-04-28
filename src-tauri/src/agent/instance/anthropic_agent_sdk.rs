use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use tauri::AppHandle;

use super::{
    emit_stream, finalize_tool_call_record, normalize_tool_args, AgentInstance,
    AssistantStreamState, ExecutedToolResult,
};
use crate::commands::{StreamEvent, ToolCallOutcome};
use crate::llm::anthropic_agent_sdk::{
    self, ClaudeCodeSdkOptions, ClaudeSdkAssistantMessage, ClaudeSdkHost, ClaudeSdkHostFuture,
    ClaudeSdkToolDefinition,
};
use crate::session::models::{ImageData, MessageRole, ToolCallInfo};
use crate::session::store::SessionStore;

struct PendingAssistantRound {
    message_id: String,
    text: String,
    tool_calls: Vec<ToolCallInfo>,
    remaining: HashSet<String>,
}

struct ClaudeSdkRoundHost<'a> {
    agent: &'a AgentInstance,
    app_handle: &'a AppHandle,
    store: &'a SessionStore,
    run_id: &'a str,
    mode: &'a str,
    streamed_text: String,
    partial_assistant: Arc<AssistantStreamState>,
    started_tool_calls: VecDeque<(String, String)>,
    known_tool_calls: VecDeque<ToolCallInfo>,
    completed_tool_ids: HashSet<String>,
    pending_round: Option<PendingAssistantRound>,
    last_assistant: Option<ClaudeSdkAssistantMessage>,
}

impl<'a> ClaudeSdkRoundHost<'a> {
    fn emit_tool_call_start(&self, tool_call_id: &str, tool_name: &str, arguments: &str) {
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::ToolCallStart {
                session_id: self.agent.session_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
            },
        );
        if let Some((ref parent_sid, ref parent_tc_id)) = self.agent.parent_tool_call {
            emit_stream(
                self.app_handle,
                self.run_id,
                StreamEvent::SubagentToolCallStart {
                    session_id: parent_sid.clone(),
                    parent_tool_call_id: parent_tc_id.clone(),
                    tool_call_id: tool_call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    arguments: arguments.to_string(),
                },
            );
        }
    }

    fn emit_tool_call_done(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        output: &str,
        outcome: ToolCallOutcome,
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
            },
        );
        if let Some((ref parent_sid, ref parent_tc_id)) = self.agent.parent_tool_call {
            let truncated_output = if output.chars().count() > 500 {
                let prefix: String = output.chars().take(500).collect();
                format!("{}... ({} chars)", prefix, output.chars().count())
            } else {
                output.to_string()
            };
            emit_stream(
                self.app_handle,
                self.run_id,
                StreamEvent::SubagentToolCallDone {
                    session_id: parent_sid.clone(),
                    parent_tool_call_id: parent_tc_id.clone(),
                    tool_call_id: tool_call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: truncated_output,
                    outcome,
                },
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
            if let Err(err) = self
                .store
                .update_message_tool_calls(&round.message_id, &round.tool_calls)
            {
                eprintln!(
                    "[Agent {}] failed to update Claude SDK tool_calls for message {}: {}",
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
                    id: format!("sdk_{}", sanitize_request_id(request_id)),
                    name: tool_name.to_string(),
                    arguments: raw_arguments_str.clone(),
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
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
                    id: format!("sdk_{}", sanitize_request_id(request_id)),
                    name: tool_name.to_string(),
                    arguments: raw_arguments_str.clone(),
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
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
                };
            }
        }

        ToolCallInfo {
            id: format!("sdk_{}", sanitize_request_id(request_id)),
            name: tool_name.to_string(),
            arguments: raw_arguments_str,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }
    }
}

impl<'a> ClaudeSdkHost for ClaudeSdkRoundHost<'a> {
    fn on_text_delta(&mut self, delta: String) {
        self.streamed_text.push_str(&delta);
        self.partial_assistant.append_text(&delta);
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::TextDelta {
                session_id: self.agent.session_id.clone(),
                text: delta.clone(),
            },
        );
        if let Some((ref parent_sid, ref tc_id)) = self.agent.parent_tool_call {
            emit_stream(
                self.app_handle,
                self.run_id,
                StreamEvent::ToolCallDelta {
                    session_id: parent_sid.clone(),
                    tool_call_id: tc_id.clone(),
                    delta,
                },
            );
        }
    }

    fn on_thinking_delta(&mut self, delta: String) {
        self.partial_assistant.append_thinking(&delta);
        emit_stream(
            self.app_handle,
            self.run_id,
            StreamEvent::ThinkingDelta {
                session_id: self.agent.session_id.clone(),
                text: delta,
            },
        );
    }

    fn on_tool_call_start(&mut self, tool_call_id: String, tool_name: String) {
        self.emit_tool_call_start(&tool_call_id, &tool_name, "");
        self.started_tool_calls.push_back((tool_call_id, tool_name));
    }

    fn on_assistant_message(&mut self, message: ClaudeSdkAssistantMessage) -> Result<(), String> {
        if message.tool_calls.is_empty() {
            self.last_assistant = Some(message);
            return Ok(());
        }

        if let Some(existing) = self.pending_round.take() {
            if !existing.remaining.is_empty() {
                eprintln!(
                    "[Agent {}] Claude SDK emitted a new tool round before the previous one finished",
                    self.agent.id
                );
            }
        }

        for tool_call in &message.tool_calls {
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
        let message_id = self.store.add_assistant_with_tool_calls_and_thinking(
            &self.agent.session_id,
            &message.text,
            &message.tool_calls,
            thinking_text,
            None,
            thinking_signature,
            None,
            None,
        )?;
        self.partial_assistant.mark_persisted(
            message_id.clone(),
            message.text.clone(),
            thinking_text.map(str::to_string),
            None,
        );

        let remaining: HashSet<String> = message
            .tool_calls
            .iter()
            .filter(|tc| !self.completed_tool_ids.contains(&tc.id))
            .map(|tc| tc.id.clone())
            .collect();

        self.pending_round = Some(PendingAssistantRound {
            message_id,
            text: message.text,
            tool_calls: message.tool_calls,
            remaining,
        });
        self.streamed_text.clear();
        self.maybe_finish_pending_round();
        Ok(())
    }

    fn execute_tool<'b>(
        &'b mut self,
        request_id: &'b str,
        tool_name: &'b str,
        arguments: serde_json::Value,
    ) -> ClaudeSdkHostFuture<'b> {
        Box::pin(async move {
            let tool_call = self.take_matching_tool_call(request_id, tool_name, &arguments);
            let mut args_for_exec = arguments.clone();
            normalize_tool_args(&mut args_for_exec);
            self.agent
                .inject_working_dir(&tool_call.name, &mut args_for_exec);

            let result = self
                .agent
                .execute_single_tool(
                    self.app_handle,
                    self.store,
                    &tool_call,
                    &args_for_exec,
                    self.run_id,
                    self.mode,
                )
                .await;

            let stored_output = match self.store.rewrite_tool_result_for_storage(
                &self.agent.session_id,
                &tool_call.id,
                &tool_call.name,
                &result.output,
            ) {
                Ok(output) => output,
                Err(err) => {
                    eprintln!(
                        "[Agent {}] failed to persist Claude SDK tool result for '{}' (id={}): {}",
                        self.agent.id, tool_call.name, tool_call.id, err
                    );
                    result.output.clone()
                }
            };

            self.emit_tool_call_done(
                &tool_call.id,
                &tool_call.name,
                &stored_output,
                result.outcome.as_stream_outcome(),
            );

            if let Err(err) =
                self.store
                    .add_tool_result(&self.agent.session_id, &tool_call.id, &result.output)
            {
                eprintln!(
                    "[Agent {}] failed to save Claude SDK tool result for '{}' (id={}): {}",
                    self.agent.id, tool_call.name, tool_call.id, err
                );
            }

            self.completed_tool_ids.insert(tool_call.id.clone());
            if let Some(round) = self.pending_round.as_mut() {
                if let Some(existing) = round
                    .tool_calls
                    .iter_mut()
                    .find(|pending_tool_call| pending_tool_call.id == tool_call.id)
                {
                    *existing = finalize_tool_call_record(existing, Some(&result));
                }
                round.remaining.remove(&tool_call.id);
            }
            self.maybe_finish_pending_round();

            ExecutedToolResult::into_tool_result(result)
        })
    }
}

impl AgentInstance {
    pub(super) async fn run_anthropic_agent_sdk(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        prompt_text: &str,
        system_prompt: &str,
        images: Option<&[ImageData]>,
        initial_mode: &str,
        run_id: &str,
    ) -> Result<String, String> {
        let effective_tools = self.resolve_effective_tool_names().await;
        let api_tools = self.build_api_tools(&effective_tools).await;

        let resume_session_id = anthropic_agent_sdk::cached_session_id(&self.session_id).await;
        if resume_session_id.is_none() && store.get_messages_for_prompt(&self.session_id)?.len() > 1
        {
            eprintln!(
                "[Agent {}] Claude SDK session '{}' has no cached Claude session id; existing Locus history will not be replayed",
                self.id, self.session_id
            );
        }

        let user_message = build_sdk_user_message(prompt_text, images);
        let tools = convert_api_tools_to_sdk(&api_tools);
        let mut host = ClaudeSdkRoundHost {
            agent: self,
            app_handle,
            store,
            run_id,
            mode: initial_mode,
            streamed_text: String::new(),
            partial_assistant: self.partial_assistant_state(),
            started_tool_calls: VecDeque::new(),
            known_tool_calls: VecDeque::new(),
            completed_tool_ids: HashSet::new(),
            pending_round: None,
            last_assistant: None,
        };

        let turn = anthropic_agent_sdk::run_turn(
            ClaudeCodeSdkOptions {
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
                resume_session_id,
                server_name: "locus".to_string(),
                tools,
                debug: self.debug,
            },
            user_message,
            &mut host,
        )
        .await?;

        if let Some(claude_session_id) = turn.claude_session_id.as_deref() {
            anthropic_agent_sdk::store_session_id(&self.session_id, claude_session_id).await;
        }

        if turn.input_tokens > 0
            || turn.output_tokens > 0
            || turn.cache_read_tokens > 0
            || turn.cache_write_tokens > 0
        {
            match store.record_token_usage(
                &self.session_id,
                turn.input_tokens as u64,
                turn.output_tokens as u64,
                turn.cache_read_tokens as u64,
                turn.cache_write_tokens as u64,
                turn.cost_usd,
                0,
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
                            context_tokens: turn.input_tokens
                                + turn.output_tokens
                                + turn.cache_read_tokens
                                + turn.cache_write_tokens,
                            context_limit: super::model_context_limit(&self.effective_model),
                        },
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[Agent {}] failed to record Claude SDK token usage: {}",
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
                    "backend": "anthropic_agent_sdk",
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

        if self.is_cancel_requested() {
            self.clear_pending_knowledge_proposal(app_handle).await;
            self.emit_cancelled(app_handle, store, run_id);
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
        if !final_text.is_empty() {
            let thinking_text = final_snapshot.as_ref().and_then(|snapshot| {
                (!snapshot.thinking_text.is_empty()).then_some(snapshot.thinking_text.as_str())
            });
            let thinking_signature = final_snapshot.as_ref().and_then(|snapshot| {
                (!snapshot.thinking_signature.is_empty())
                    .then_some(snapshot.thinking_signature.as_str())
            });
            done_message_id = store.add_message_with_thinking(
                &self.session_id,
                MessageRole::Assistant,
                &final_text,
                thinking_text,
                None,
                thinking_signature,
                None,
                None,
            )?;
            self.partial_assistant.mark_persisted(
                done_message_id.clone(),
                final_text.clone(),
                thinking_text.map(str::to_string),
                None,
            );
        }

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

fn convert_api_tools_to_sdk(api_tools: &[serde_json::Value]) -> Vec<ClaudeSdkToolDefinition> {
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
            Some(ClaudeSdkToolDefinition {
                name,
                description,
                input_schema,
            })
        })
        .collect()
}

fn build_sdk_user_message(text: &str, images: Option<&[ImageData]>) -> serde_json::Value {
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
