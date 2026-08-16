use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::watch;

use crate::tool::output::{append_field, append_text_field};
use crate::tool::ToolResult;

pub const ASYNC_MODE_PARAMETER: &str = "async";
pub const GET_TASK_STATUS_TOOL_NAME: &str = "get_task_status";
pub const CANCEL_TASK_TOOL_NAME: &str = "cancel_task";
pub const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
pub const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";
pub const ASYNC_TASK_UPDATED_EVENT: &str = "async-task-updated";

const MAX_RETAINED_TASKS: usize = 256;
const MAX_NOTIFICATION_PREVIEW_CHARS: usize = 2_000;
const MAX_LIVE_OUTPUT_CHARS: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncMode {
    Sync,
    Async,
    Notify,
}

impl AsyncMode {
    pub fn parse(args: &serde_json::Value, enabled: bool) -> Result<Self, String> {
        let raw = args
            .get(ASYNC_MODE_PARAMETER)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("sync")
            .trim();
        let mode = match raw {
            "" | "sync" => Self::Sync,
            "async" => Self::Async,
            "notify" | "async_notify" => Self::Notify,
            value => {
                return Err(format!(
                    "Invalid async mode '{value}'. Use 'sync', 'async', or 'notify'."
                ));
            }
        };
        if !enabled && mode != Self::Sync {
            return Err(
                "Async tasks are disabled. Enable them in Settings > Experimental first."
                    .to_string(),
            );
        }
        Ok(mode)
    }

    pub fn is_background(self) -> bool {
        matches!(self, Self::Async | Self::Notify)
    }

    pub fn should_notify(self) -> bool {
        self == Self::Notify
    }
}

pub fn remove_async_mode(args: &serde_json::Value) -> serde_json::Value {
    let mut args = args.clone();
    if let Some(object) = args.as_object_mut() {
        object.remove(ASYNC_MODE_PARAMETER);
    }
    args
}

pub fn supports_async_mode(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "bash" | "unity_execute" | "unity_test_run" | "subagent"
    )
}

pub fn augment_tool_schema(tool_name: &str, tool: &mut serde_json::Value) {
    if !supports_async_mode(tool_name) {
        return;
    }
    let Some(parameters) = tool
        .get_mut("function")
        .and_then(|function| function.get_mut("parameters"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let properties = parameters
        .entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    let Some(properties) = properties.as_object_mut() else {
        return;
    };
    properties.insert(
        ASYNC_MODE_PARAMETER.to_string(),
        serde_json::json!({
            "type": "string",
            "enum": ["sync", "async", "notify"],
            "description": "Execution mode. 'sync' waits and returns the result; 'async' runs without an execution deadline and returns a task id immediately; 'notify' automatically resumes or reminds this session with the final result when the task finishes, so do not poll get_task_status. Failures detected during startup are returned directly and do not require get_task_status. Default 'sync'.",
            "default": "sync"
        }),
    );
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsyncTaskStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl AsyncTaskStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncTaskSnapshot {
    pub task_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub status: AsyncTaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    pub notify: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncTaskUpdatedEvent {
    pub session_id: String,
    pub assistant_message_id: String,
    pub tool_call_id: String,
    pub task_id: String,
    pub tool_name: String,
    pub status: AsyncTaskStatus,
    pub output: String,
}

pub fn emit_task_updated(
    app_handle: &tauri::AppHandle,
    assistant_message_id: &str,
    tool_call_id: &str,
    snapshot: &AsyncTaskSnapshot,
) {
    let event = AsyncTaskUpdatedEvent {
        session_id: snapshot.session_id.clone(),
        assistant_message_id: assistant_message_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        task_id: snapshot.task_id.clone(),
        tool_name: snapshot.tool_name.clone(),
        status: snapshot.status.clone(),
        output: snapshot.output.clone().unwrap_or_default(),
    };
    if let Err(error) = app_handle.emit(ASYNC_TASK_UPDATED_EVENT, event) {
        eprintln!(
            "[Agent async] failed to emit task update for {}: {}",
            snapshot.task_id, error
        );
    }
}

impl AsyncTaskSnapshot {
    pub fn elapsed_ms(&self) -> i64 {
        self.finished_at
            .unwrap_or_else(now_millis)
            .saturating_sub(self.created_at)
    }
}

fn format_task_snapshot(snapshot: &AsyncTaskSnapshot, include_output: bool) -> String {
    let mut output = "Async task:".to_string();
    append_text_field(&mut output, "id", &snapshot.task_id);
    append_text_field(&mut output, "tool", &snapshot.tool_name);
    append_field(&mut output, "status", snapshot.status.as_str());
    append_field(&mut output, "elapsed_ms", snapshot.elapsed_ms());
    append_field(&mut output, "notify", snapshot.notify);

    let mut timing = "timing:".to_string();
    append_field(&mut timing, "created_at_ms", snapshot.created_at);
    append_field(&mut timing, "updated_at_ms", snapshot.updated_at);
    if let Some(finished_at) = snapshot.finished_at {
        append_field(&mut timing, "finished_at_ms", finished_at);
    }
    output.push('\n');
    output.push_str(&timing);

    if let Some(progress) = snapshot
        .progress
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        output.push_str("\nprogress: ");
        output.push_str(progress);
    }
    if include_output {
        if let Some(result) = snapshot.output.as_deref() {
            output.push_str("\nresult:");
            if result.is_empty() {
                output.push_str(" (no output)");
            } else {
                output.push('\n');
                output.push_str(result);
            }
        }
    }
    output
}

struct AsyncTaskEntry {
    snapshot: AsyncTaskSnapshot,
    cancel_tx: watch::Sender<bool>,
    working_dir: Option<String>,
}

#[derive(Clone)]
pub struct AsyncTaskStart {
    pub task_id: String,
    pub cancel_rx: watch::Receiver<bool>,
}

pub struct AsyncTaskRunGuard {
    manager: Arc<AsyncTaskManager>,
    task_id: String,
    armed: bool,
}

impl AsyncTaskRunGuard {
    pub fn complete(&mut self) {
        self.armed = false;
    }
}

impl Drop for AsyncTaskRunGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let result = ToolResult {
            output: "Async task terminated unexpectedly.".to_string(),
            is_error: true,
        };
        self.manager.finish(&self.task_id, &result);
    }
}

#[derive(Default)]
pub struct AsyncTaskManager {
    tasks: Mutex<HashMap<String, AsyncTaskEntry>>,
    notifications: Mutex<HashMap<String, VecDeque<String>>>,
}

impl AsyncTaskManager {
    pub fn run_guard(self: &Arc<Self>, task_id: &str) -> AsyncTaskRunGuard {
        AsyncTaskRunGuard {
            manager: self.clone(),
            task_id: task_id.to_string(),
            armed: true,
        }
    }

    pub fn create_task(&self, session_id: &str, tool_name: &str, notify: bool) -> AsyncTaskStart {
        self.create_task_in_workspace(session_id, tool_name, notify, None)
    }

    pub fn create_task_in_workspace(
        &self,
        session_id: &str,
        tool_name: &str,
        notify: bool,
        working_dir: Option<&str>,
    ) -> AsyncTaskStart {
        let task_id = format!("task_{}", uuid::Uuid::new_v4().simple());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let now = now_millis();
        let entry = AsyncTaskEntry {
            snapshot: AsyncTaskSnapshot {
                task_id: task_id.clone(),
                session_id: session_id.to_string(),
                tool_name: tool_name.to_string(),
                status: AsyncTaskStatus::Queued,
                created_at: now,
                updated_at: now,
                finished_at: None,
                progress: Some("Queued".to_string()),
                output: None,
                is_error: None,
                notify,
            },
            cancel_tx,
            working_dir: working_dir.map(str::to_string),
        };
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        prune_terminal_tasks(&mut tasks);
        tasks.insert(task_id.clone(), entry);
        AsyncTaskStart { task_id, cancel_rx }
    }

    pub fn mark_running(&self, task_id: &str, progress: impl Into<String>) {
        self.update(task_id, |snapshot| {
            snapshot.status = AsyncTaskStatus::Running;
            snapshot.progress = Some(progress.into());
        });
    }

    pub fn report_progress(&self, task_id: &str, progress: impl Into<String>) {
        self.update(task_id, |snapshot| {
            if !snapshot.status.is_terminal() {
                snapshot.progress = Some(progress.into());
            }
        });
    }

    pub fn append_output(&self, task_id: &str, chunk: &str) -> Option<AsyncTaskSnapshot> {
        if chunk.is_empty() {
            return self.snapshot(task_id);
        }
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = tasks.get_mut(task_id)?;
        if entry.snapshot.status.is_terminal() {
            return Some(entry.snapshot.clone());
        }
        let output = entry.snapshot.output.get_or_insert_with(String::new);
        output.push_str(chunk);
        truncate_live_output(output);
        entry.snapshot.updated_at = now_millis();
        Some(entry.snapshot.clone())
    }

    pub fn finish(&self, task_id: &str, result: &ToolResult) -> Option<AsyncTaskSnapshot> {
        let snapshot = self.finish_without_notification(task_id, result)?;
        self.enqueue_completion_notification(&snapshot);
        Some(snapshot)
    }

    pub(crate) fn finish_without_notification(
        &self,
        task_id: &str,
        result: &ToolResult,
    ) -> Option<AsyncTaskSnapshot> {
        {
            let mut tasks = self
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = tasks.get_mut(task_id)?;
            entry.snapshot.status = if result.is_error {
                AsyncTaskStatus::Failed
            } else {
                AsyncTaskStatus::Completed
            };
            entry.snapshot.progress = Some(if result.is_error {
                "Failed".to_string()
            } else {
                "Completed".to_string()
            });
            entry.snapshot.output = Some(result.output.clone());
            if let Some(output) = entry.snapshot.output.as_mut() {
                truncate_live_output(output);
            }
            entry.snapshot.is_error = Some(result.is_error);
            entry.snapshot.finished_at = Some(now_millis());
            entry.snapshot.updated_at = now_millis();
            Some(entry.snapshot.clone())
        }
    }

    pub(crate) fn mark_cancelled_without_notification(
        &self,
        task_id: &str,
    ) -> Option<AsyncTaskSnapshot> {
        {
            let mut tasks = self
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = tasks.get_mut(task_id)?;
            entry.snapshot.status = AsyncTaskStatus::Cancelled;
            entry.snapshot.progress = Some("Cancelled".to_string());
            let output = entry.snapshot.output.get_or_insert_with(String::new);
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("Task cancelled.");
            truncate_live_output(output);
            entry.snapshot.is_error = Some(true);
            entry.snapshot.finished_at = Some(now_millis());
            entry.snapshot.updated_at = now_millis();
            Some(entry.snapshot.clone())
        }
    }

    pub fn snapshot(&self, task_id: &str) -> Option<AsyncTaskSnapshot> {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(task_id)
            .map(|entry| entry.snapshot.clone())
    }

    pub fn cancel(&self, task_id: &str) -> Result<AsyncTaskSnapshot, String> {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Async task '{task_id}' was not found."))?;
        if entry.snapshot.status.is_terminal() {
            return Ok(entry.snapshot.clone());
        }
        entry.snapshot.status = AsyncTaskStatus::Cancelling;
        entry.snapshot.progress = Some("Cancellation requested".to_string());
        entry.snapshot.updated_at = now_millis();
        entry.cancel_tx.send_replace(true);
        Ok(entry.snapshot.clone())
    }

    fn cancel_matching(
        &self,
        predicate: impl Fn(&AsyncTaskEntry) -> bool,
    ) -> Vec<AsyncTaskSnapshot> {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut cancelled = Vec::new();
        for entry in tasks.values_mut() {
            if entry.snapshot.status.is_terminal() || !predicate(entry) {
                continue;
            }
            entry.snapshot.status = AsyncTaskStatus::Cancelling;
            entry.snapshot.progress = Some("Cancellation requested".to_string());
            entry.snapshot.updated_at = now_millis();
            entry.cancel_tx.send_replace(true);
            cancelled.push(entry.snapshot.clone());
        }
        cancelled
    }

    pub fn cancel_session(&self, session_id: &str) -> Vec<AsyncTaskSnapshot> {
        self.cancel_matching(|entry| entry.snapshot.session_id == session_id)
    }

    pub fn cancel_workspace(&self, working_dir: &str) -> Vec<AsyncTaskSnapshot> {
        let target = working_dir_key(working_dir);
        self.cancel_matching(|entry| {
            entry
                .working_dir
                .as_deref()
                .is_some_and(|value| working_dir_key(value) == target)
        })
    }

    pub fn cancel_all(&self) -> Vec<AsyncTaskSnapshot> {
        self.cancel_matching(|_| true)
    }

    pub fn active_count(&self) -> usize {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|entry| !entry.snapshot.status.is_terminal())
            .count()
    }

    pub fn start_result(&self, task_id: &str) -> ToolResult {
        let notify = self
            .snapshot(task_id)
            .is_some_and(|snapshot| snapshot.notify);
        let guidance = if notify {
            "Completion and the final result will be delivered automatically in a system reminder. Do not call get_task_status for this task; use cancel_task to stop it."
        } else {
            "Use get_task_status with this id for progress and the final result; use cancel_task to stop it."
        };
        ToolResult {
            output: format!(
                "Async task: id={} status=queued notify={}\n{}",
                crate::tool::output::flat_text(task_id),
                notify,
                guidance
            ),
            is_error: false,
        }
    }

    pub fn status_result(&self, task_id: &str) -> ToolResult {
        match self.snapshot(task_id) {
            Some(snapshot) => ToolResult {
                output: format_task_snapshot(&snapshot, true),
                is_error: false,
            },
            None => ToolResult {
                output: format!("Async task '{task_id}' was not found."),
                is_error: true,
            },
        }
    }

    pub fn cancel_result(&self, task_id: &str) -> ToolResult {
        match self.cancel(task_id) {
            Ok(snapshot) => ToolResult {
                output: format_task_snapshot(&snapshot, snapshot.status.is_terminal()),
                is_error: false,
            },
            Err(output) => ToolResult {
                output,
                is_error: true,
            },
        }
    }

    pub fn enqueue_notification(&self, session_id: &str, reminder: String) {
        self.notifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(session_id.to_string())
            .or_default()
            .push_back(reminder);
    }

    pub(crate) fn enqueue_completion_notification(&self, snapshot: &AsyncTaskSnapshot) {
        if snapshot.notify {
            self.enqueue_notification(&snapshot.session_id, Self::completion_reminder(snapshot));
        }
    }

    pub fn take_notifications(&self, session_id: &str) -> Vec<String> {
        self.notifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    pub fn take_notifications_and_pending(&self, session_id: &str) -> (Vec<String>, bool) {
        let tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = tasks.values().any(|entry| {
            entry.snapshot.session_id == session_id
                && entry.snapshot.notify
                && !entry.snapshot.status.is_terminal()
        });
        let notifications = self
            .notifications
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id)
            .map(VecDeque::into_iter)
            .map(Iterator::collect)
            .unwrap_or_default();
        (notifications, pending)
    }

    pub fn completion_reminder(snapshot: &AsyncTaskSnapshot) -> String {
        let output = snapshot.output.as_deref().unwrap_or_default();
        let preview = truncate_chars(output, MAX_NOTIFICATION_PREVIEW_CHARS);
        format!(
            "{SYSTEM_REMINDER_OPEN}\nAsync task {} ({}) finished with status {:?}. Its original tool call now contains the final result. Do not call get_task_status for this task. Continue the current work using the result.\n\nResult preview:\n{}\n{SYSTEM_REMINDER_CLOSE}",
            snapshot.task_id, snapshot.tool_name, snapshot.status, preview
        )
    }

    fn update(&self, task_id: &str, update: impl FnOnce(&mut AsyncTaskSnapshot)) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = tasks.get_mut(task_id) else {
            return;
        };
        update(&mut entry.snapshot);
        entry.snapshot.updated_at = now_millis();
    }
}

pub type TaskProgressReporter = Arc<dyn Fn(String) + Send + Sync>;
pub type TaskOutputReporter = Arc<dyn Fn(String) + Send + Sync>;

fn truncate_live_output(output: &mut String) {
    let char_count = output.chars().count();
    if char_count <= MAX_LIVE_OUTPUT_CHARS {
        return;
    }
    const MARKER: &str = "[earlier output truncated]\n";
    let keep = MAX_LIVE_OUTPUT_CHARS.saturating_sub(MARKER.chars().count());
    let tail = output
        .chars()
        .skip(char_count.saturating_sub(keep))
        .collect::<String>();
    output.clear();
    output.push_str(MARKER);
    output.push_str(&tail);
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn prune_terminal_tasks(tasks: &mut HashMap<String, AsyncTaskEntry>) {
    if tasks.len() < MAX_RETAINED_TASKS {
        return;
    }
    let mut terminal = tasks
        .iter()
        .filter(|(_, entry)| entry.snapshot.status.is_terminal())
        .map(|(id, entry)| {
            (
                id.clone(),
                entry
                    .snapshot
                    .finished_at
                    .unwrap_or(entry.snapshot.updated_at),
            )
        })
        .collect::<Vec<_>>();
    terminal.sort_by_key(|(_, finished_at)| *finished_at);
    let remove_count = tasks
        .len()
        .saturating_sub(MAX_RETAINED_TASKS)
        .saturating_add(1);
    for (id, _) in terminal.into_iter().take(remove_count) {
        tasks.remove(&id);
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn working_dir_key(path: &str) -> String {
    let normalized = path
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if cfg!(target_os = "windows") {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_mode_defaults_to_sync_and_respects_gate() {
        assert_eq!(
            AsyncMode::parse(&serde_json::json!({}), false).unwrap(),
            AsyncMode::Sync
        );
        assert!(AsyncMode::parse(&serde_json::json!({ "async": "async" }), false).is_err());
        assert_eq!(
            AsyncMode::parse(&serde_json::json!({ "async": "notify" }), true).unwrap(),
            AsyncMode::Notify
        );
    }

    #[test]
    fn unity_execute_supports_background_task_status() {
        assert!(supports_async_mode("unity_execute"));
        let mut tool = serde_json::json!({
            "function": {
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        });
        augment_tool_schema("unity_execute", &mut tool);
        assert_eq!(
            tool.pointer("/function/parameters/properties/async/type")
                .and_then(serde_json::Value::as_str),
            Some("string")
        );
    }

    #[test]
    fn cancellation_is_idempotent_for_terminal_tasks() {
        let manager = AsyncTaskManager::default();
        let started = manager.create_task("session", "bash", false);
        manager.finish(
            &started.task_id,
            &ToolResult {
                output: "done".to_string(),
                is_error: false,
            },
        );
        let first = manager.cancel(&started.task_id).unwrap();
        let second = manager.cancel(&started.task_id).unwrap();
        assert_eq!(first.status, AsyncTaskStatus::Completed);
        assert_eq!(second.status, AsyncTaskStatus::Completed);
    }

    #[test]
    fn cancellation_signals_a_running_task_and_updates_progress() {
        let manager = AsyncTaskManager::default();
        let mut started = manager.create_task("session", "bash", true);
        manager.mark_running(&started.task_id, "running");

        let snapshot = manager.cancel(&started.task_id).unwrap();

        assert_eq!(snapshot.status, AsyncTaskStatus::Cancelling);
        assert!(*started.cancel_rx.borrow_and_update());
        let (_, pending) = manager.take_notifications_and_pending("session");
        assert!(pending);
    }

    #[test]
    fn session_cancellation_reaches_every_background_task_in_that_session() {
        let manager = AsyncTaskManager::default();
        let mut first =
            manager.create_task_in_workspace("session-a", "bash", false, Some("C:/workspace-a"));
        let mut second =
            manager.create_task_in_workspace("session-a", "bash", true, Some("C:/workspace-a"));
        let mut unrelated =
            manager.create_task_in_workspace("session-b", "bash", false, Some("C:/workspace-b"));

        let cancelled = manager.cancel_session("session-a");

        assert_eq!(cancelled.len(), 2);
        assert!(*first.cancel_rx.borrow_and_update());
        assert!(*second.cancel_rx.borrow_and_update());
        assert!(!*unrelated.cancel_rx.borrow_and_update());
        assert_eq!(manager.active_count(), 3);
    }

    #[test]
    fn workspace_cancellation_matches_normalized_paths() {
        let manager = AsyncTaskManager::default();
        let mut task = manager.create_task_in_workspace(
            "session-a",
            "bash",
            false,
            Some("C:\\Workspace\\Project\\"),
        );

        let cancelled = manager.cancel_workspace("c:/workspace/project");

        if cfg!(target_os = "windows") {
            assert_eq!(cancelled.len(), 1);
            assert!(*task.cancel_rx.borrow_and_update());
        } else {
            assert!(cancelled.is_empty());
            assert!(!*task.cancel_rx.borrow_and_update());
        }
    }

    #[test]
    fn dropped_run_guard_converts_a_panic_or_abort_into_a_failed_task() {
        let manager = Arc::new(AsyncTaskManager::default());
        let started = manager.create_task("session", "bash", true);
        {
            let _guard = manager.run_guard(&started.task_id);
        }

        let snapshot = manager.snapshot(&started.task_id).unwrap();
        assert_eq!(snapshot.status, AsyncTaskStatus::Failed);
        assert!(!manager.take_notifications("session").is_empty());
    }

    #[test]
    fn task_results_are_flat_and_terminal_status_includes_output() {
        let manager = AsyncTaskManager::default();
        let started = manager.create_task("session", "bash", false);
        assert_eq!(
            manager.start_result(&started.task_id).output,
            format!(
                "Async task: id=\"{}\" status=queued notify=false\nUse get_task_status with this id for progress and the final result; use cancel_task to stop it.",
                started.task_id
            )
        );

        manager.mark_running(&started.task_id, "Running command");
        let running = manager.status_result(&started.task_id).output;
        assert!(running.starts_with(&format!(
            "Async task: id=\"{}\" tool=\"bash\" status=running",
            started.task_id
        )));
        assert!(running.contains("\nprogress: Running command"));
        assert!(!running.contains("{\n"));

        manager.finish(
            &started.task_id,
            &ToolResult {
                output: "Exit code: 0\ndone".to_string(),
                is_error: false,
            },
        );
        let completed = manager.status_result(&started.task_id).output;
        assert!(completed.contains(" status=completed "));
        assert!(completed.ends_with("\nresult:\nExit code: 0\ndone"));
    }

    #[test]
    fn running_task_status_includes_incremental_output() {
        let manager = AsyncTaskManager::default();
        let started = manager.create_task("session", "bash", false);
        manager.mark_running(&started.task_id, "Running bash");
        manager.append_output(&started.task_id, "first\n");
        manager.append_output(&started.task_id, "second\n");

        let running = manager.status_result(&started.task_id).output;
        assert!(running.contains(" status=running "));
        assert!(running.ends_with("\nresult:\nfirst\nsecond\n"));
    }

    #[test]
    fn notify_tasks_discourage_polling_and_deliver_a_completion_reminder() {
        let manager = AsyncTaskManager::default();
        let started = manager.create_task("session", "bash", true);

        let queued = manager.start_result(&started.task_id).output;
        assert!(queued.contains("status=queued notify=true"));
        assert!(queued.contains("Do not call get_task_status for this task"));

        manager.finish(
            &started.task_id,
            &ToolResult {
                output: "Exit code: 0\ndone".to_string(),
                is_error: false,
            },
        );
        let reminders = manager.take_notifications("session");
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].contains("finished with status Completed"));
        assert!(reminders[0].contains("original tool call now contains the final result"));
        assert!(reminders[0].contains("Do not call get_task_status for this task"));
    }

    #[test]
    fn deferred_completion_notification_waits_for_tool_result_persistence() {
        let manager = AsyncTaskManager::default();
        let started = manager.create_task("session", "bash", true);
        let snapshot = manager
            .finish_without_notification(
                &started.task_id,
                &ToolResult {
                    output: "done".to_string(),
                    is_error: false,
                },
            )
            .expect("finished task");

        assert!(manager.take_notifications("session").is_empty());
        manager.enqueue_completion_notification(&snapshot);
        assert_eq!(manager.take_notifications("session").len(), 1);
    }
}
