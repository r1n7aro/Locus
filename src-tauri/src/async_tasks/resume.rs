use super::*;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubagentResumeInfo {
    pub child_session_id: String,
    pub agent_id: String,
    pub working_dir: String,
    pub model_id: String,
    pub effort: Option<String>,
    pub fast_mode: bool,
    pub readonly: bool,
}

pub(crate) type SubagentResumeHandler = Arc<
    dyn Fn(
            AsyncTaskSnapshot,
            String,
            watch::Receiver<bool>,
        ) -> Pin<Box<dyn Future<Output = (ToolResult, bool)> + Send>>
        + Send
        + Sync,
>;

impl AsyncTaskManager {
    pub(crate) fn register_resume_handler(&self, session_id: &str, handler: SubagentResumeHandler) {
        self.resume_handlers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string(), handler);
    }

    pub(crate) fn bind_origin(
        &self,
        task_id: &str,
        message_id: &str,
        tool_call_id: &str,
    ) -> Result<(), String> {
        self.update(task_id, |task| {
            task.assistant_message_id = Some(message_id.to_string());
            task.tool_call_id = Some(tool_call_id.to_string());
        });
        if let Some(store) = &self.store {
            store.save_async_task(&self.get_task(task_id)?, None)?;
        }
        Ok(())
    }

    pub(crate) fn bind_subagent(
        &self,
        task_id: &str,
        info: SubagentResumeInfo,
    ) -> Result<(), String> {
        self.update(task_id, |task| task.resume = Some(info));
        if let Some(store) = &self.store {
            store.save_async_task(&self.get_task(task_id)?, None)?;
        }
        Ok(())
    }

    pub(crate) fn get_session_task(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<AsyncTaskSnapshot, String> {
        self.list_session_tasks(session_id)?
            .into_iter()
            .find(|task| task.public_id() == task_id || task.task_id == task_id)
            .ok_or_else(|| format!("Async task '{task_id}' was not found in the current session."))
    }

    pub(crate) fn list_session_tasks(
        &self,
        session_id: &str,
    ) -> Result<Vec<AsyncTaskSnapshot>, String> {
        let mut by_id: HashMap<_, _> = match &self.store {
            Some(store) => store
                .list_async_tasks(session_id)?
                .into_iter()
                .map(|task| (task.task_id.clone(), task))
                .collect(),
            None => HashMap::new(),
        };
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        for entry in tasks
            .values()
            .filter(|entry| entry.snapshot.session_id == session_id)
        {
            by_id.insert(entry.snapshot.task_id.clone(), entry.snapshot.clone());
        }
        let mut result: Vec<_> = by_id.into_values().collect();
        result.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.task_id.cmp(&b.task_id))
        });
        Ok(result)
    }

    pub(crate) fn task_payload(task: AsyncTaskSnapshot) -> Result<serde_json::Value, String> {
        let child_session_id = task
            .resume
            .as_ref()
            .map(|info| info.child_session_id.clone());
        let can_resume = task.resume.is_some()
            && matches!(
                task.status,
                AsyncTaskStatus::Failed | AsyncTaskStatus::Cancelled
            );
        let public_id = task.public_id().to_string();
        let mut value = serde_json::to_value(task).map_err(|e| e.to_string())?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("resume");
            obj.remove("localId");
            obj.insert("taskId".into(), serde_json::json!(public_id));
            obj.remove("assistantMessageId");
            obj.remove("toolCallId");
            obj.insert("childSessionId".into(), serde_json::json!(child_session_id));
            obj.insert("canResume".into(), serde_json::json!(can_resume));
        }
        Ok(value)
    }

    pub(super) fn prepare_resume(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<(AsyncTaskSnapshot, watch::Receiver<bool>), String> {
        self.prepare_resume_attempt(session_id, task_id, false, None)
    }

    fn prepare_resume_attempt(
        &self,
        session_id: &str,
        task_id: &str,
        allow_completed: bool,
        pending: Option<&communication::PendingDelivery>,
    ) -> Result<(AsyncTaskSnapshot, watch::Receiver<bool>), String> {
        let previous = self.get_session_task(session_id, task_id)?;
        let task_id = previous.task_id.as_str();
        if previous.tool_name != "subagent" {
            return Err("Only subagent tasks can be resumed. Bash/Python processes cannot resume from an exited execution position.".into());
        }
        let info = previous
            .resume
            .as_ref()
            .ok_or("This task has no saved subagent continuation context.")?;
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(store), Some(pending)) = (&self.store, pending) {
            if !pending.is_pending(store)? {
                return Err("The message was already delivered.".into());
            }
        }
        if let Some(current) = tasks.get(task_id) {
            if current.snapshot.attempt != previous.attempt || !current.completion_ready {
                return Err("This async task is already running or still finalizing.".into());
            }
        }
        if !matches!(
            previous.status,
            AsyncTaskStatus::Failed | AsyncTaskStatus::Cancelled
        ) && !(allow_completed && previous.status == AsyncTaskStatus::Completed)
        {
            return Err("Only failed or cancelled subagent tasks can be resumed.".into());
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let mut next = previous.clone();
        next.attempt = next
            .attempt
            .checked_add(1)
            .ok_or("Task attempt limit reached")?;
        next.status = AsyncTaskStatus::Queued;
        next.notify = true;
        next.started_at = Some(now_millis());
        next.updated_at = now_millis();
        next.finished_at = None;
        next.output = None;
        next.is_error = None;
        next.progress = Some("Queued for continuation".into());
        if let Some(store) = &self.store {
            next.output_path = Some(
                store
                    .async_task_output_path(
                        session_id,
                        &format!("{task_id}_attempt_{}", next.attempt),
                    )?
                    .to_string_lossy()
                    .into_owned(),
            );
            store.save_async_task(&next, None)?;
        }
        tasks.insert(
            task_id.to_string(),
            AsyncTaskEntry {
                snapshot: next.clone(),
                cancel_tx,
                working_dir: Some(info.working_dir.clone()),
                completion_ready: false,
                completion_persisted: false,
            },
        );
        self.changes.notify_waiters();
        Ok((next, cancel_rx))
    }

    pub(crate) fn resume_task(
        self: &Arc<Self>,
        session_id: &str,
        task_id: &str,
        message: String,
        app: tauri::AppHandle,
    ) -> Result<AsyncTaskSnapshot, String> {
        self.start_continuation(session_id, task_id, message, Some(app), false, None)
    }

    pub(crate) fn resume_task_for_message(
        self: &Arc<Self>,
        session_id: &str,
        task_id: &str,
        app: tauri::AppHandle,
        pending: communication::PendingDelivery,
    ) -> Result<AsyncTaskSnapshot, String> {
        self.start_continuation(session_id, task_id, "Process the newly delivered agent message or task result in the existing conversation and complete the requested follow-up.".into(), Some(app), true, Some(pending))
    }

    pub(super) fn start_continuation(
        self: &Arc<Self>,
        session_id: &str,
        task_id: &str,
        message: String,
        app: Option<tauri::AppHandle>,
        allow_completed: bool,
        pending: Option<communication::PendingDelivery>,
    ) -> Result<AsyncTaskSnapshot, String> {
        // Validate ownership/type before looking up the live parent execution context.
        let task = self.get_session_task(session_id, task_id)?;
        if task.tool_name != "subagent" {
            return Err(
                "Only subagent tasks support continuation; bash/python tasks cannot be resumed."
                    .into(),
            );
        }
        let handler = self
            .resume_handlers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned()
            .ok_or("Continue the owning session before resuming its subagent task.")?;
        let (snapshot, mut cancel_rx) =
            self.prepare_resume_attempt(session_id, task_id, allow_completed, pending.as_ref())?;
        let task = snapshot.clone();
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut guard = manager.run_guard(&task.task_id);
            manager.mark_running(&task.task_id, "Continuing subagent");
            if let Some(app) = &app {
                manager.emit_resumed_update(app, &task.task_id);
            }
            let (result, interrupted) = if *cancel_rx.borrow() {
                (
                    ToolResult {
                        output: "Task cancelled.".into(),
                        is_error: true,
                    },
                    true,
                )
            } else {
                tokio::select! {
                    result = handler(task.clone(), message, cancel_rx.clone()) => result,
                    _ = cancel_rx.changed() => (ToolResult { output: "Task cancelled.".into(), is_error: true }, true),
                }
            };
            if interrupted || *cancel_rx.borrow() {
                if let Some(snapshot) = manager.mark_cancelled_without_notification(&task.task_id) {
                    manager.enqueue_completion_notification(&snapshot);
                }
            } else {
                manager.finish(&task.task_id, &result);
            }
            if let Some(app) = &app {
                manager.emit_resumed_update(app, &task.task_id);
            }
            guard.complete();
        });
        Ok(snapshot)
    }

    fn emit_resumed_update(&self, app: &tauri::AppHandle, task_id: &str) {
        let Some(snapshot) = self.snapshot(task_id) else {
            return;
        };
        if let (Some(message), Some(tool)) =
            (&snapshot.assistant_message_id, &snapshot.tool_call_id)
        {
            emit_task_updated(app, message, tool, &snapshot);
            if snapshot.status.is_terminal() {
                if let Some(store) = &self.store {
                    let outcome = match snapshot.status {
                        AsyncTaskStatus::Cancelled => crate::commands::ToolCallOutcome::Interrupted,
                        AsyncTaskStatus::Failed => crate::commands::ToolCallOutcome::Error,
                        _ => crate::commands::ToolCallOutcome::Done,
                    };
                    if let Err(error) = store.update_background_tool_display(
                        message,
                        tool,
                        snapshot.output.as_deref().unwrap_or_default(),
                        outcome,
                    ) {
                        eprintln!("[Agent async] failed to update resumed task display: {error}");
                    }
                }
            }
        }
    }
}
