//! Delegation entry points of a Claude Code round host.
//!
//! Native Claude subagents run in the conversation's `NativeAgentHub`, outside
//! the parent's assistant rounds. The round host only ends its batch's writes
//! before an `Agent` call starts (the workspace lock is not reentrant), and
//! runs the calls the Mod routes to Locus: a Locus-routed `Agent` as a Locus
//! `subagent` with the same call id, and `SendMessage` to a Locus subagent.
use super::*;
use super::super::claude_native_agents::{locus_agent_result, locus_background_agent_result};

/// Progress title of an Agent card run as a Locus subagent (`agentToolCall.ts`).
const LOCUS_AGENT_PROGRESS_TITLE: &str = "locus-agent";

fn error_result(output: impl Into<String>) -> ExecutedToolResult {
    ExecutedToolResult::from_tool_result(crate::tool::ToolResult {
        output: output.into(),
        is_error: true,
    })
}

pub(super) fn nested_record(id: &str, name: &str, arguments: String, order: Option<u32>) -> ToolCallInfo {
    ToolCallInfo {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
        order,
        server_tool: None,
        server_tool_output: None,
        server_tool_result: None,
        outcome: None,
        recorded_output: None,
        nested_tool_calls: None,
    }
}
impl PendingAssistantRound {
    /// The completed writes of the batch holding `agent_call_id`, to end
    /// before that `Agent` runs. The workspace lock is not reentrant: a
    /// subagent writing under its parent's batch lock would wait on the
    /// parent, which waits on the subagent. Writes after the `Agent` take a
    /// new checkpoint and lock.
    fn take_writes_before_agent(
        pending: &mut Option<Self>,
        waiting: &mut [Self],
        agent_call_id: &str,
    ) -> Option<CliRoundCompletion> {
        let round = pending
            .iter_mut()
            .chain(waiting.iter_mut())
            .find(|round| round.tool_calls.iter().any(|call| call.id == agent_call_id))?;
        let workspace_guard = round.workspace_guard.take()?;
        round.undo_checked = false;
        Some(CliRoundCompletion {
            message_id: round.message_id.clone(),
            undo_guard: round.undo_guard.take(),
            has_unity_execute: std::mem::take(&mut round.has_unity_execute),
            queued_unity_asset_paths: std::mem::take(&mut round.queued_unity_asset_paths),
            workspace_guard: Some(workspace_guard),
        })
    }
}

impl<'a> ClaudeCodeRoundHost<'a> {
    /// Records undo for, and unlocks, the parent batch's writes completed
    /// before its `Agent` call. Claude never runs a write concurrently with
    /// another call, so the batch's earlier writes have returned.
    pub(super) async fn finish_parent_writes_before_agent(&mut self, agent_call_id: &str) {
        let completion = PendingAssistantRound::take_writes_before_agent(
            &mut self.pending_round,
            &mut self.waiting_rounds,
            agent_call_id,
        );
        if let Some(completion) = completion {
            self.finish_cli_round_external_side_effects(completion).await;
        }
    }

    /// The assistant message that holds a call of this run.
    pub(super) fn message_id_for_tool_call(&self, tool_call_id: &str) -> Option<String> {
        self.pending_round
            .iter()
            .chain(self.waiting_rounds.iter())
            .find(|round| round.tool_calls.iter().any(|call| call.id == tool_call_id))
            .map(|round| round.message_id.clone())
            .or_else(|| self.last_persisted_assistant_message_id.clone())
    }

    /// An `Agent` call the Mod routed to Locus: a Locus `subagent` with the
    /// same call id, answered with the native Agent result shape. A
    /// background call returns at once with a completed-shaped launch result
    /// (Claude never tracks it as one of its own tasks); Locus delivers the
    /// subagent's result like any other notifying task.
    pub(super) async fn execute_locus_agent(
        &mut self,
        tool_call: &ToolCallInfo,
        input: &serde_json::Value,
        message_id: &str,
    ) -> ExecutedToolResult {
        let Some(directory) = self.agent.claude_delegation() else {
            return error_result("Agent is not available in this conversation; no subagent was started.");
        };
        let (entry, mut args) = match self.agent.claude_locus_agent_call(&directory, input) {
            Ok(call) => call,
            Err(error) => return error_result(error),
        };
        let background = input["run_in_background"] == true;
        if background {
            args["async"] = serde_json::json!("notify");
        }
        // Tells the Agent card its backend while it runs; the stored result
        // keeps the Locus task id for history.
        super::super::emit_tool_progress(
            self.app_handle,
            self.run_id,
            &self.agent.session_id,
            &tool_call.id,
            LOCUS_AGENT_PROGRESS_TITLE,
            entry.model.clone(),
            None,
            "running",
        );
        let call = nested_record(
            &tool_call.id,
            "subagent",
            serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
            None,
        );
        let scope = claude_code_cli::native_scope::current().filter(|scope| scope.call.id == tool_call.id);
        let started = std::time::Instant::now();
        let active_skill_tool_names = self.agent.active_skill_tool_names(self.active_skill_tool_names);
        // The subagent's own tools must not observe this call's native scope.
        let result = claude_code_cli::native_scope::NativeInvocation::without(Box::pin(
            self.agent.execute_single_tool(
                self.app_handle,
                self.store,
                &call,
                &args,
                self.run_id,
                message_id,
                self.mode,
                &active_skill_tool_names,
                false,
            ),
        ))
        .await;
        if result.outcome == super::super::ToolRunOutcome::Done && !result.is_error {
            if let Some(scope) = scope {
                let value = if background {
                    match background_task_id(&result.output) {
                        Some(task_id) => locus_background_agent_result(&entry, input, &task_id),
                        None => return error_result(format!("The Locus subagent did not start in the background: {}", result.output)),
                    }
                } else {
                    let tool_uses = result.nested_tool_calls.as_ref().map_or(0, Vec::len);
                    locus_agent_result(
                        &entry,
                        input,
                        &result.output,
                        started.elapsed().as_millis() as u64,
                        tool_uses,
                        &tool_call.id,
                    )
                };
                scope.set_result(value);
            }
        }
        result
    }

    /// `SendMessage` the Mod routed to Locus: a message to a Locus subagent
    /// (`locus:<task id>`), queued in its mailbox. A finished subagent
    /// continues with it and notifies again.
    pub(super) fn execute_locus_send_message(
        &self,
        tool_call: &ToolCallInfo,
        input: &serde_json::Value,
    ) -> ExecutedToolResult {
        use tauri::Manager;
        let to = input["to"].as_str().unwrap_or_default();
        let Some(address) = to.strip_prefix("locus:").filter(|id| !id.is_empty()) else {
            return error_result(format!("SendMessage to {to:?} does not address a Locus subagent; no message was sent."));
        };
        let Some(message) = input["message"].as_str().filter(|text| !text.trim().is_empty()) else {
            return error_result("SendMessage requires a non-empty message; no message was sent.");
        };
        let manager = self
            .app_handle
            .state::<Arc<crate::async_tasks::AsyncTaskManager>>()
            .inner()
            .clone();
        match manager.send_agent_message(self.app_handle, &self.agent.session_id, address, message) {
            Ok(receipt) => {
                if let Some(scope) = claude_code_cli::native_scope::current().filter(|scope| scope.call.id == tool_call.id) {
                    scope.set_result(serde_json::json!({
                        "success": true,
                        "message": format!("Message queued for {to}. A running agent reads it before its next step; a finished agent continues with it and notifies you again, so wait for that notification before resending."),
                    }));
                }
                ExecutedToolResult::from_tool_result(crate::tool::ToolResult { output: receipt.to_string(), is_error: false })
            }
            Err(error) => {
                let reachable = manager
                    .reachable_agent_ids(&self.agent.session_id)
                    .into_iter()
                    .filter(|id| !id.contains('/') && id != "parent")
                    .map(|id| format!("`locus:{id}`"))
                    .collect::<Vec<_>>();
                error_result(format!(
                    "{error} No message was sent.{}",
                    if reachable.is_empty() { String::new() } else { format!(" Reachable Locus agents: {}.", reachable.join(", ")) }
                ))
            }
        }
    }
}

/// Task id of a background Locus tool start (`AsyncTaskManager::start_result`).
fn background_task_id(output: &str) -> Option<String> {
    let id = output.strip_prefix("Async task: id=")?.split_whitespace().next()?;
    Some(serde_json::from_str::<String>(id).unwrap_or_else(|_| id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::workspace_execution_lock::{
        process_workspace_execution_lock, WorkspaceExecutionLockOwner,
        WorkspaceExecutionLockRequest,
    };
    use std::time::Duration;

    fn owner(run_id: &str) -> WorkspaceExecutionLockOwner {
        WorkspaceExecutionLockOwner {
            session_id: "session".into(),
            run_id: run_id.into(),
            iteration: 0,
            workspace: "workspace".into(),
            tools: vec!["write".into()],
        }
    }

    fn round(message_id: &str, calls: &[(&str, &str)]) -> PendingAssistantRound {
        PendingAssistantRound {
            message_id: message_id.into(),
            tool_calls: calls
                .iter()
                .map(|(id, name)| nested_record(id, name, "{}".into(), None))
                .collect(),
            remaining: calls.iter().map(|(id, _)| id.to_string()).collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn parent_batch_writes_end_before_its_agent_so_subagent_writes_proceed() {
        let root = tempfile::tempdir().unwrap();
        let lock = process_workspace_execution_lock(&root.path().to_string_lossy());
        let (_cancel, cancel_rx) = tokio::sync::watch::channel(false);
        let subagent_write = || {
            lock.acquire(
                WorkspaceExecutionLockRequest::PathWrite(vec!["b.txt".into()]),
                owner("subagent"),
                cancel_rx.clone(),
            )
        };
        // [Bash, Agent, Edit] after Bash: the batch lock spans the batch.
        let mut parent = round("parent-message", &[("bash", "bash"), ("agent", "Agent"), ("edit", "edit")]);
        parent.remaining.remove("bash");
        parent.undo_checked = true;
        parent.has_unity_execute = true;
        parent.queued_unity_asset_paths = vec!["Assets/a.prefab".into()];
        parent.workspace_guard = Some(
            lock.acquire(WorkspaceExecutionLockRequest::Exclusive, owner("parent"), cancel_rx.clone())
                .await
                .unwrap(),
        );
        let mut pending = Some(round("later-message", &[("read", "read")]));
        let mut waiting = vec![parent];

        // Held across the Agent, the subagent's write would wait on its parent.
        assert!(tokio::time::timeout(Duration::from_millis(100), subagent_write()).await.is_err());
        assert!(PendingAssistantRound::take_writes_before_agent(&mut pending, &mut waiting, "missing").is_none());

        let completion = PendingAssistantRound::take_writes_before_agent(&mut pending, &mut waiting, "agent")
            .expect("the parent's completed writes");
        assert_eq!(completion.message_id, "parent-message");
        assert!(completion.has_unity_execute);
        assert_eq!(completion.queued_unity_asset_paths, ["Assets/a.prefab"]);
        assert!(completion.workspace_guard.is_some());
        // finish_cli_round_external_side_effects records undo, then unlocks.
        drop(completion);
        tokio::time::timeout(Duration::from_secs(1), subagent_write())
            .await
            .expect("the subagent's write proceeds")
            .unwrap();

        // The Edit after the Agent takes a new checkpoint and lock.
        let parent = &waiting[0];
        assert!(!parent.undo_checked && !parent.has_unity_execute && parent.workspace_guard.is_none());
        assert_eq!(parent.remaining, HashSet::from(["agent".to_string(), "edit".to_string()]));
        // Later subagent calls find nothing left to end.
        assert!(PendingAssistantRound::take_writes_before_agent(&mut pending, &mut waiting, "agent").is_none());
    }
}
