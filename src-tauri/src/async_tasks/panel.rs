use super::*;

/// Lightweight live state for the composer panel. Output and continuation
/// context stay in the manager rather than crossing IPC on every refresh.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackgroundTaskInfo {
    pub task_id: String,
    pub session_id: String,
    pub local_id: String,
    pub tool_name: String,
    pub description: Option<String>,
    pub status: AsyncTaskStatus,
    pub progress: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
}

impl From<&AsyncTaskSnapshot> for BackgroundTaskInfo {
    fn from(task: &AsyncTaskSnapshot) -> Self {
        Self {
            task_id: task.task_id.clone(),
            session_id: task.session_id.clone(),
            local_id: task.public_id().to_string(),
            tool_name: task.tool_name.clone(),
            description: task.description.clone(),
            status: task.status.clone(),
            progress: task.progress.clone(),
            created_at: task.created_at,
            started_at: task.started_at,
        }
    }
}

/// App-wide event: `{ sessionId, tasks }` with the session's live panel rows,
/// sent after they change so panels and triggers need not poll.
pub(crate) const BACKGROUND_TASKS_CHANGED_EVENT: &str = "background-tasks-changed";
/// Progress lines can arrive in bursts; one event per session covers each window.
const PANEL_CHANGE_COALESCE: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Default)]
struct PanelChanges {
    sessions: std::collections::HashSet<String>,
    scheduled: bool,
}

fn panel_app() -> &'static std::sync::OnceLock<tauri::AppHandle> {
    static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
    &APP
}

fn panel_changes() -> std::sync::MutexGuard<'static, PanelChanges> {
    static CHANGES: std::sync::OnceLock<Mutex<PanelChanges>> = std::sync::OnceLock::new();
    CHANGES
        .get_or_init(|| Mutex::new(PanelChanges::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Enables [`BACKGROUND_TASKS_CHANGED_EVENT`]; the manager is read from app state.
pub(crate) fn start_panel_events(app: tauri::AppHandle) {
    let _ = panel_app().set(app);
}

/// Record that a session's panel rows changed. Takes only a leaf lock, so it
/// is safe while the task table is locked.
pub(super) fn note_panel_change(session_id: &str) {
    let Some(app) = panel_app().get() else {
        return;
    };
    let mut changes = panel_changes();
    changes.sessions.insert(session_id.to_string());
    if changes.scheduled {
        return;
    }
    changes.scheduled = true;
    drop(changes);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PANEL_CHANGE_COALESCE).await;
        let sessions = {
            let mut changes = panel_changes();
            changes.scheduled = false;
            std::mem::take(&mut changes.sessions)
        };
        use tauri::Manager;
        let Some(manager) = app.try_state::<Arc<AsyncTaskManager>>() else {
            return;
        };
        for session_id in sessions {
            let tasks = manager.list_background_tasks(&session_id);
            let _ = app.emit(
                BACKGROUND_TASKS_CHANGED_EVENT,
                serde_json::json!({ "sessionId": session_id, "tasks": tasks }),
            );
        }
    });
}

impl AsyncTaskManager {
    pub(crate) fn list_background_tasks(&self, session_id: &str) -> Vec<BackgroundTaskInfo> {
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let mut result: Vec<_> = tasks
            .values()
            .filter(|entry| {
                entry.snapshot.session_id == session_id && !entry.snapshot.status.is_terminal()
            })
            .map(|entry| BackgroundTaskInfo::from(&entry.snapshot))
            .collect();
        result.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.task_id.cmp(&b.task_id))
        });
        result
    }

    pub(crate) fn cancel_background_task(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<AsyncTaskSnapshot, String> {
        // Use the immutable internal id, not the session-local display name.
        let task = self.get_task(task_id)?;
        if task.session_id != session_id {
            return Err("Async task was not found in the current session.".into());
        }
        self.cancel(&task.task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_lists_only_live_tasks_owned_by_the_session() {
        let manager = AsyncTaskManager::default();
        let queued = manager.create_task("current", "bash", false);
        let running = manager.create_task("current", "subagent", true);
        manager.mark_running(&running.task_id, "Inspecting assets");
        let cancelling = manager.create_task("current", "python", false);
        manager.cancel(&cancelling.task_id).unwrap();
        let completed = manager.create_task("current", "bash", false);
        manager.finish(
            &completed.task_id,
            &ToolResult {
                output: "done".into(),
                is_error: false,
            },
        );
        let failed = manager.create_task("current", "bash", false);
        manager.finish(
            &failed.task_id,
            &ToolResult {
                output: "failed".into(),
                is_error: true,
            },
        );
        let cancelled = manager.create_task("current", "bash", false);
        manager.mark_cancelled_without_notification(&cancelled.task_id);
        manager.create_task("other", "bash", false);

        let tasks = manager.list_background_tasks("current");
        assert_eq!(tasks.len(), 3);
        for task in [&queued, &running, &cancelling] {
            assert!(tasks.iter().any(|info| info.task_id == task.task_id));
        }
        assert_eq!(
            tasks
                .iter()
                .find(|info| info.task_id == running.task_id)
                .unwrap()
                .progress
                .as_deref(),
            Some("Inspecting assets")
        );
        assert!(manager.list_background_tasks("").is_empty());
    }

    #[test]
    fn panel_cancellation_is_scoped_and_signals_only_the_selected_task() {
        let manager = AsyncTaskManager::default();
        let first = manager.create_task("current", "bash", false);
        let second = manager.create_task("current", "python", false);
        assert!(manager
            .cancel_background_task("other", &first.task_id)
            .is_err());
        assert!(!*first.cancel_rx.borrow());
        let result = manager
            .cancel_background_task("current", &first.task_id)
            .unwrap();
        assert_eq!(result.status, AsyncTaskStatus::Cancelling);
        assert!(*first.cancel_rx.borrow());
        assert!(!*second.cancel_rx.borrow());

        manager.mark_cancelled_without_notification(&first.task_id);
        assert_eq!(
            manager
                .cancel_background_task("current", &first.task_id)
                .unwrap()
                .status,
            AsyncTaskStatus::Cancelled
        );
    }
}
