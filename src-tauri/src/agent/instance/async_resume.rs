use super::*;

impl AgentInstance {
    pub(super) async fn execute_subagent(
        &self,
        app: &AppHandle,
        store: &SessionStore,
        args: &serde_json::Value,
        tool_call_id: &str,
        run_id: &str,
    ) -> ExecutedToolResult {
        if !self.multi_agent_enabled {
            return self.multi_agent_disabled_result();
        }
        if self.background_task_id.is_some() {
            return self
                .execute_subagent_inner(app, store, args, tool_call_id, run_id)
                .await;
        }
        let manager = app
            .state::<Arc<crate::async_tasks::AsyncTaskManager>>()
            .inner()
            .clone();
        let started = manager.create_task_in_workspace(
            &self.session_id,
            "subagent",
            false,
            Some(&self.working_dir),
        );
        if let Err(output) = manager.prepare_named_task(
            &started.task_id,
            args.get("description").and_then(serde_json::Value::as_str),
            args.get("name").and_then(serde_json::Value::as_str),
        ) {
            manager.discard_task(&started.task_id);
            return ExecutedToolResult::from_tool_result(ToolResult {
                output,
                is_error: true,
            });
        }
        let mut guard = manager.run_guard(&started.task_id);
        let mut parent_cancel = self.cancel_waiter();
        let mut task_cancel = started.cancel_rx.clone();
        let mut executor = self.clone_for_background_task(started.cancel_rx.clone());
        executor.background_task_id = Some(started.task_id.clone());
        manager.mark_running(&started.task_id, "Running subagent");
        let mut result = tokio::select! {
            result = executor.execute_subagent_inner(app, store, args, tool_call_id, run_id) => result,
            _ = parent_cancel.changed() => Self::interrupted_tool_result(),
            _ = task_cancel.changed() => Self::interrupted_tool_result(),
        };
        if result.outcome == ToolRunOutcome::Interrupted {
            if let Some(snapshot) = manager.mark_cancelled_without_notification(&started.task_id) {
                manager.enqueue_completion_notification(&snapshot);
            }
        } else {
            manager.finish(
                &started.task_id,
                &ToolResult {
                    output: result.output.clone(),
                    is_error: result.is_error,
                },
            );
        }
        let task = manager
            .get_task(&started.task_id)
            .expect("tracked subagent task");
        result.output = format!("Task id: {}\n{}", task.public_id(), result.output);
        guard.complete();
        result
    }

    /// The active parent supplies current credentials, workspace services, and
    /// permissions. Saved child metadata survives restarts; backend objects do not.
    pub(super) fn register_async_resumer(
        &self,
        app: &AppHandle,
        store: &SessionStore,
        run_id: &str,
    ) {
        if !self.async_tasks_enabled {
            return;
        }
        let manager = app.state::<Arc<crate::async_tasks::AsyncTaskManager>>();
        manager.attach_runtime(app);
        let parent = self.clone_for_background_task(self.cancel_waiter());
        let app = app.clone();
        let store = store.clone();
        let run_id = run_id.to_string();
        let handler: crate::async_tasks::SubagentResumeHandler = Arc::new(
            move |task, message, cancel_rx| {
                let mut executor = parent.clone_for_background_task(cancel_rx);
                let app = app.clone();
                let store = store.clone();
                let run_id = run_id.clone();
                Box::pin(async move {
                    let Some(info) = task.resume.clone() else {
                        return (
                            ToolResult {
                                output: "No saved subagent continuation context.".into(),
                                is_error: true,
                            },
                            false,
                        );
                    };
                    if normalize_workspace_path_key(&executor.working_dir, ".")
                        != normalize_workspace_path_key(&info.working_dir, ".")
                    {
                        return (ToolResult { output: "The task belongs to a different checkout. Resume it from its original workspace.".into(), is_error: true }, false);
                    }
                    let previous_done = {
                        let active = app.state::<crate::ActiveTasks>();
                        let tasks = active.lock().await;
                        tasks
                            .get(&info.child_session_id)
                            .map(|task| task.done_rx.clone())
                    };
                    if let Some(mut done) = previous_done {
                        while !*done.borrow() && done.changed().await.is_ok() {}
                    }
                    let prompt = if message.trim().is_empty() {
                        "Continue the unfinished task from this conversation's existing context. The previous attempt failed or was interrupted. Inspect the current state, preserve completed work, and finish the remaining work. Do not repeat already completed changes.".to_string()
                    } else {
                        message
                    };
                    let args = serde_json::json!({
                        "subagent_type": info.agent_id,
                        "description": task.description.as_deref().unwrap_or("Continue subagent"),
                        "prompt": prompt,
                    });
                    executor.background_task_id = Some(task.task_id.clone());
                    executor.resumed_subagent = Some(info);
                    let result = executor
                        .execute_subagent(
                            &app,
                            &store,
                            &args,
                            task.tool_call_id.as_deref().unwrap_or(&task.task_id),
                            &run_id,
                        )
                        .await;
                    let interrupted = result.outcome == ToolRunOutcome::Interrupted;
                    (result.into_tool_result(), interrupted)
                })
            },
        );
        manager.register_resume_handler(&self.session_id, handler);
    }
}

pub(super) fn subtract_previous_usage(
    usage: &mut crate::commands::TokenUsage,
    previous: Option<&crate::commands::TokenUsage>,
) {
    let Some(previous) = previous else {
        return;
    };
    usage.total_input_tokens = usage
        .total_input_tokens
        .saturating_sub(previous.total_input_tokens);
    usage.total_output_tokens = usage
        .total_output_tokens
        .saturating_sub(previous.total_output_tokens);
    usage.total_cache_read_tokens = usage
        .total_cache_read_tokens
        .saturating_sub(previous.total_cache_read_tokens);
    usage.total_cache_write_tokens = usage
        .total_cache_write_tokens
        .saturating_sub(previous.total_cache_write_tokens);
    usage.timed_output_tokens = usage
        .timed_output_tokens
        .saturating_sub(previous.timed_output_tokens);
    usage.model_active_duration_ms = usage
        .model_active_duration_ms
        .saturating_sub(previous.model_active_duration_ms);
    usage.total_cost_usd = (usage.total_cost_usd - previous.total_cost_usd).max(0.0);
    usage.priced_rounds = usage.priced_rounds.saturating_sub(previous.priced_rounds);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_continuation_usage_merges_only_new_tokens_and_cost() {
        let before = crate::commands::TokenUsage {
            total_input_tokens: 100,
            total_output_tokens: 50,
            total_cache_read_tokens: 20,
            total_cache_write_tokens: 10,
            timed_output_tokens: 50,
            model_active_duration_ms: 1000,
            total_cost_usd: 0.25,
            priced_rounds: 1,
            context_tokens: 0,
            context_limit: 0,
        };
        let mut after = before.clone();
        after.total_input_tokens += 5;
        after.total_output_tokens += 7;
        after.total_cost_usd += 0.125;
        subtract_previous_usage(&mut after, Some(&before));
        assert_eq!(after.total_input_tokens, 5);
        assert_eq!(after.total_output_tokens, 7);
        assert_eq!(after.total_cache_read_tokens, 0);
        assert_eq!(after.total_cache_write_tokens, 0);
        assert_eq!(after.model_active_duration_ms, 0);
        assert_eq!(after.total_cost_usd, 0.125);
        assert_eq!(after.priced_rounds, 0);
    }
}
