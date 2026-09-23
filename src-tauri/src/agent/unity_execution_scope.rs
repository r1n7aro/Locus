//! Serialize stateful Unity tools without occupying the workspace file gate.
//! Nested dispatch through ToolRegistry borrows the current task's scope; RPC
//! calls and independently spawned tasks must acquire their own scope.
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex as AsyncMutex};

static LOCKS: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct Scope {
    project: String,
    test_deadline: Option<Instant>,
}
tokio::task_local! { static CURRENT_SCOPE: Scope; }

pub(crate) fn remaining_test_timeout(fallback: Option<Duration>) -> Option<Duration> {
    CURRENT_SCOPE
        .try_with(|scope| {
            scope
                .test_deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        })
        .ok()
        .flatten()
        .or(fallback)
}

pub(crate) async fn run<T>(
    project: &str,
    name: &str,
    args: &serde_json::Value,
    cancel: Option<watch::Receiver<bool>>,
    background: bool,
    work: impl Future<Output = T>,
) -> Result<T, &'static str> {
    if !super::tool_execution_policy::needs_unity_execution_barrier(name, args) {
        return Ok(work.await);
    }
    let key = super::workspace_execution_lock::normalize_workspace_path_key(project, ".");
    if CURRENT_SCOPE
        .try_with(|current| current.project == key)
        .unwrap_or(false)
    {
        return Ok(work.await);
    }
    let lock = {
        let mut locks = LOCKS.lock().unwrap_or_else(|error| error.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        let entry = locks.entry(key.clone()).or_default();
        if let Some(lock) = entry.upgrade() {
            lock
        } else {
            let lock = Arc::new(AsyncMutex::new(()));
            *entry = Arc::downgrade(&lock);
            lock
        }
    };
    let test_deadline = (name == "unity_test_run" && !background).then(|| {
        Instant::now()
            + Duration::from_millis(
                args.get("timeout_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(600_000)
                    .clamp(1_000, 3_600_000),
            )
    });
    let mut cancel = cancel;
    let _guard = tokio::select! {
        biased;
        _ = async {
            match cancel.as_mut() {
                Some(cancel) => { let _ = cancel.wait_for(|cancelled| *cancelled).await; }
                None => std::future::pending::<()>().await,
            }
        } => return Err("Unity execution cancelled while waiting for the Editor"),
        _ = async {
            match test_deadline {
                Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
                None => std::future::pending::<()>().await,
            }
        } => return Err("Unity Test run timed out while waiting for the Editor"),
        guard = lock.lock() => guard,
    };
    Ok(CURRENT_SCOPE
        .scope(
            Scope {
                project: key,
                test_deadline,
            },
            work,
        )
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_execution_policy::{background_workspace_request, workspace_request};
    use crate::agent::workspace_execution_lock::{
        process_workspace_execution_lock, WorkspaceExecutionLockOwner,
    };
    use serde_json::json;
    use tokio::sync::oneshot;

    const UNITY_TOOLS: &[&str] = &[
        "unity_execute",
        "unity_run_states",
        "unity_test_list",
        "unity_test_run",
        "unity_recompile",
        "unity_hot_reload",
        "unity_set_play_mode",
    ];

    #[tokio::test]
    async fn every_unity_tool_allows_real_source_edits_without_undo() {
        for tool in UNITY_TOOLS {
            let root = tempfile::tempdir().unwrap();
            let project = root.path().to_string_lossy().to_string();
            let source = root.path().join("Player.cs");
            std::fs::write(&source, "class Player { int speed = 1; }\n").unwrap();
            let args = json!({"readonly":false});
            assert!(workspace_request(tool, &args, &project, true, false).is_none());
            assert!(
                background_workspace_request(tool, &args, &project, true, false, "batch").is_none()
            );
            assert!(workspace_request(tool, &args, &project, true, true).is_some());
            let (entered, ready) = oneshot::channel();
            let (finish, hold) = oneshot::channel();
            let unity_project = project.clone();
            let unity = tokio::spawn(async move {
                run(&unity_project, tool, &args, None, false, async {
                    entered.send(()).unwrap();
                    hold.await.unwrap();
                })
                .await
                .unwrap();
            });
            ready.await.unwrap();
            let edit = json!({"filePath":source,"edits":[{"oldString":"speed = 1","newString":"speed = 2"}]});
            let (_cancel, rx) = watch::channel(false);
            let file_guard = tokio::time::timeout(
                Duration::from_secs(2),
                process_workspace_execution_lock(&project).acquire(
                    workspace_request("edit", &edit, &project, true, false).unwrap(),
                    WorkspaceExecutionLockOwner {
                        session_id: "editor".into(),
                        run_id: "edit".into(),
                        iteration: 1,
                        workspace: project.clone(),
                        tools: vec!["edit".into()],
                    },
                    rx,
                ),
            )
            .await
            .expect("Unity must not block source editing")
            .unwrap();
            let result = crate::tool::ToolRegistry::with_builtins()
                .execute_with_context(
                    "edit",
                    &edit,
                    crate::tool::ToolExecutionContext {
                        working_dir: Some(project.clone()),
                        ..Default::default()
                    },
                )
                .await;
            assert!(!result.is_error, "{tool}: {}", result.output);
            assert!(std::fs::read_to_string(source)
                .unwrap()
                .contains("speed = 2"));
            assert!(
                !unity.is_finished(),
                "edit must finish while Unity is still active"
            );
            drop(file_guard);
            finish.send(()).unwrap();
            unity.await.unwrap();
        }
    }

    #[tokio::test]
    async fn unity_tools_serialize_reenter_and_cancel_independently_of_files() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().to_string_lossy().to_string();
        let (entered, ready) = oneshot::channel();
        let (finish, hold) = oneshot::channel();
        let holder_project = project.clone();
        let holder = tokio::spawn(async move {
            run(
                &holder_project,
                "unity_test_run",
                &json!({}),
                None,
                true,
                async {
                    // ToolRegistry is nested under Agent/MCP dispatch in the same task.
                    run(
                        &holder_project,
                        "unity_test_run",
                        &json!({}),
                        None,
                        true,
                        async {},
                    )
                    .await
                    .unwrap();
                    entered.send(()).unwrap();
                    hold.await.unwrap();
                },
            )
            .await
            .unwrap();
        });
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        let (cancel, rx) = watch::channel(false);
        let args = json!({});
        let mut waiting = Box::pin(run(
            &project,
            "unity_recompile",
            &args,
            Some(rx),
            false,
            async { panic!("cancelled waiter must not execute") },
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut waiting)
                .await
                .is_err()
        );
        cancel.send(true).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .is_err());
        finish.send(()).unwrap();
        holder.await.unwrap();
        run(
            &project,
            "unity_recompile",
            &json!({}),
            None,
            false,
            async {},
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn unity_queue_respects_test_timeout_and_other_projects_do_not_wait() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().to_string_lossy().to_string();
        run(
            &project,
            "unity_recompile",
            &json!({}),
            None,
            false,
            async {
                let blocked_project = project.clone();
                let blocked = tokio::spawn(async move {
                    run(
                        &blocked_project,
                        "unity_test_run",
                        &json!({"timeout_ms":1000}),
                        None,
                        false,
                        async { panic!("timed out test must not start") },
                    )
                    .await
                });
                let other = tempfile::tempdir().unwrap();
                run(
                    &other.path().to_string_lossy(),
                    "unity_recompile",
                    &json!({}),
                    None,
                    false,
                    async {},
                )
                .await
                .unwrap();
                let result = tokio::time::timeout(Duration::from_secs(3), blocked)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(result.unwrap_err().contains("timed out"));
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn imports_and_edit_cleanup_wait_for_running_unity_tools() {
        for operation in [
            "unity_import_assets",
            "unity_edit_session_cleanup",
            "unity_capture_viewport",
        ] {
            let root = tempfile::tempdir().unwrap();
            let project = root.path().to_string_lossy().to_string();
            let (started, ready) = oneshot::channel();
            let (finish, hold) = oneshot::channel();
            let current_project = project.clone();
            let current = tokio::spawn(async move {
                run(
                    &current_project,
                    "unity_test_run",
                    &json!({}),
                    None,
                    true,
                    async {
                        started.send(()).unwrap();
                        hold.await.unwrap();
                    },
                )
                .await
                .unwrap();
            });
            ready.await.unwrap();
            let args = json!({});
            let mut queued = Box::pin(run(&project, operation, &args, None, true, async {}));
            assert!(tokio::time::timeout(Duration::from_millis(20), &mut queued)
                .await
                .is_err());
            finish.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(1), queued)
                .await
                .unwrap()
                .unwrap();
            current.await.unwrap();
        }
    }
}
