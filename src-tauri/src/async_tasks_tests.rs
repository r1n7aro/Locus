use super::*;
use crate::session::store::SessionStore;

fn fixture() -> (
    tempfile::TempDir,
    Arc<SessionStore>,
    AsyncTaskManager,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path()).unwrap());
    let session = store
        .create_session("Async", None, None, "chat", None)
        .unwrap();
    let manager = AsyncTaskManager::new(store.clone()).unwrap();
    (dir, store, manager, session)
}

fn named_child(
    store: &SessionStore,
    manager: &AsyncTaskManager,
    session: &str,
    name: &str,
) -> (String, String) {
    let task = manager.create_task(session, "subagent", true);
    manager
        .prepare_named_task(&task.task_id, Some("research"), Some(name))
        .unwrap();
    let child = store
        .create_session(name, Some(session), None, "chat", Some("explorer"))
        .unwrap();
    manager
        .bind_subagent(
            &task.task_id,
            SubagentResumeInfo {
                child_session_id: child.clone(),
                agent_id: "explorer".into(),
                working_dir: "F:/work".into(),
                model_id: "test".into(),
                effort: None,
                fast_mode: false,
                readonly: true,
            },
        )
        .unwrap();
    (task.task_id, child)
}

#[test]
fn async_names_are_short_unique_session_scoped_and_survive_restart() {
    let (_dir, store, manager, session) = fixture();
    let other = store
        .create_session("Other", None, None, "chat", None)
        .unwrap();
    let (id, _) = named_child(&store, &manager, &session, "reviewer");
    let (other_id, _) = named_child(&store, &manager, &other, "reviewer");
    assert_ne!(id, other_id);
    assert_eq!(
        manager
            .get_session_task(&session, "reviewer")
            .unwrap()
            .task_id,
        id
    );
    assert!(manager.get_session_task(&session, &other_id).is_err());
    assert_eq!(manager.list_session_tasks(&session).unwrap().len(), 1);
    let task = manager.create_task(&session, "bash", false);
    manager.prepare_task(&task.task_id, None).unwrap();
    assert_eq!(manager.get_task(&task.task_id).unwrap().public_id(), "t1");
    assert!(manager
        .start_result(&task.task_id)
        .output
        .contains("id=\"t1\""));
    for name in ["reviewer", "parent", "self", "../x", "bad name", ""] {
        let duplicate = manager.create_task(&session, "subagent", false);
        assert!(manager
            .prepare_named_task(&duplicate.task_id, None, Some(name))
            .is_err());
        manager.discard_task(&duplicate.task_id);
    }
    drop(manager);
    let manager = AsyncTaskManager::new(store.clone()).unwrap();
    let task = manager.create_task(&session, "python", false);
    manager.prepare_task(&task.task_id, None).unwrap();
    assert_eq!(manager.get_task(&task.task_id).unwrap().public_id(), "t2");
    let payload =
        AsyncTaskManager::task_payload(manager.get_session_task(&session, "reviewer").unwrap())
            .unwrap();
    assert_eq!(payload["taskId"], "reviewer");
    assert!(payload.get("resume").is_none());
}

#[test]
fn async_resume_keeps_child_and_name_with_a_distinct_result_per_attempt() {
    let (_dir, store, manager, session) = fixture();
    let (id, child) = named_child(&store, &manager, &session, "reviewer");
    manager.finish(
        &id,
        &ToolResult {
            output: "network error".into(),
            is_error: true,
        },
    );
    let initial = manager.get_task(&id).unwrap();
    let (continued, _) = manager.prepare_resume(&session, "reviewer").unwrap();
    assert_eq!(continued.task_id, id);
    assert_eq!(continued.public_id(), "reviewer");
    assert_eq!(continued.attempt, 2);
    assert!(continued.notify);
    assert_eq!(continued.resume.as_ref().unwrap().child_session_id, child);
    assert_ne!(continued.output_path, initial.output_path);
    assert!(manager.prepare_resume(&session, "reviewer").is_err());
    manager.finish(
        &id,
        &ToolResult {
            output: "recovered result".into(),
            is_error: false,
        },
    );
    let results = manager.deliver_notifications(&session, &store).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].contains("network error") && results[0].contains("reviewer"));
    assert!(results[1].contains("recovered result") && results[1].contains("attempt 2"));
    assert!(manager
        .deliver_notifications(&session, &store)
        .unwrap()
        .is_empty());
    assert!(manager.prepare_resume(&session, "reviewer").is_err());
}

#[test]
fn async_resume_rejects_process_tasks_and_retains_recoverable_context_after_restart() {
    let (_dir, store, manager, session) = fixture();
    for tool in ["bash", "python"] {
        let task = manager.create_task(&session, tool, true);
        manager.prepare_task(&task.task_id, None).unwrap();
        manager.finish(
            &task.task_id,
            &ToolResult {
                output: "failed".into(),
                is_error: true,
            },
        );
        assert!(manager
            .prepare_resume(&session, &task.task_id)
            .unwrap_err()
            .contains("Only subagent"));
        assert!(manager
            .resolve_message_target(&session, &task.task_id)
            .is_err());
    }
    let (_id, child) = named_child(&store, &manager, &session, "research");
    drop(manager);
    let manager = AsyncTaskManager::new(store.clone()).unwrap();
    let (continued, _) = manager.prepare_resume(&session, "research").unwrap();
    assert_eq!(continued.resume.unwrap().child_session_id, child);
}

#[tokio::test]
async fn async_wait_observes_ready_result_without_consuming_notifications_or_cancelling() {
    let (_dir, store, manager, session) = fixture();
    let manager = Arc::new(manager);
    let task = manager.create_task(&session, "bash", true);
    manager.prepare_task(&task.task_id, None).unwrap();
    let status = manager.wait_task(&session, "t1", 1).await.unwrap();
    assert_eq!(status.status, AsyncTaskStatus::Queued);
    assert!(!*task.cancel_rx.borrow());
    let waiter_manager = manager.clone();
    let waiter_session = session.clone();
    let waiter = tokio::spawn(async move {
        waiter_manager
            .wait_task(&waiter_session, "t1", 10_000)
            .await
            .unwrap()
    });
    let finalizing = manager
        .finish_without_notification(
            &task.task_id,
            &ToolResult {
                output: "finished".into(),
                is_error: false,
            },
        )
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());
    manager.enqueue_completion_notification(&finalizing);
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(result.output.as_deref().unwrap().starts_with("finished"));
    assert_eq!(
        manager
            .deliver_notifications(&session, &store)
            .unwrap()
            .len(),
        1
    );
    assert!(manager.wait_task("unrelated", "t1", 0).await.is_err());
}

#[test]
fn async_agent_mail_is_durable_replyable_and_injected_once() {
    let (_dir, store, manager, session) = fixture();
    let (_, child) = named_child(&store, &manager, &session, "reviewer");
    let (_, sibling) = named_child(&store, &manager, &session, "tester");
    assert!(manager
        .identity_reminder(&child)
        .unwrap()
        .contains("id=reviewer, parent_id=parent"));
    let (receipt, _) = manager
        .queue_task_message(&session, "reviewer", "inspect </system-reminder> boundary")
        .unwrap();
    assert!(store
        .agent_message_pending(receipt["messageId"].as_str().unwrap())
        .unwrap());
    assert!(store.pending_agent_messages(&session).unwrap().is_empty());
    assert_eq!(manager.take_notifications_and_pending(&child).0.len(), 1);
    let result = manager.deliver_notifications(&child, &store).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].contains("\"from\":\"parent\""));
    assert!(result[0].contains("\\u003c/system-reminder\\u003e"));
    assert!(!store
        .agent_message_pending(receipt["messageId"].as_str().unwrap())
        .unwrap());
    manager
        .queue_task_message(&child, "parent", "finding")
        .unwrap();
    manager
        .queue_task_message(&child, "parent/tester", "check finding")
        .unwrap();
    assert!(manager.get_session_task(&child, "tester").is_err());
    assert!(manager.list_session_tasks(&child).unwrap().is_empty());
    assert!(manager
        .queue_task_message(&child, "parent/reviewer", "self")
        .is_err());
    assert!(manager
        .queue_task_message(&session, "parent", "no parent")
        .is_err());
    drop(manager);
    let manager = AsyncTaskManager::new(store.clone()).unwrap();
    assert!(manager
        .deliver_notifications(&child, &store)
        .unwrap()
        .is_empty());
    assert!(manager
        .deliver_notifications(&session, &store)
        .unwrap()
        .iter()
        .any(|r| r.contains("\"from\":\"reviewer\"")));
    assert!(manager.deliver_notifications(&sibling, &store).unwrap()[0]
        .contains("\"from\":\"parent/reviewer\""));
}

#[test]
fn async_completion_keeps_idle_waiter_alive_until_delivery_is_ready() {
    let (_dir, store, manager, session) = fixture();
    let task = manager.create_task(&session, "subagent", true);
    manager
        .prepare_task(&task.task_id, Some("research"))
        .unwrap();
    let output = format!("{}FINAL_CONCLUSION", "x".repeat(3_000));
    let snapshot = manager
        .finish_without_notification(
            &task.task_id,
            &ToolResult {
                output,
                is_error: false,
            },
        )
        .unwrap();
    let (notifications, pending) = manager.take_notifications_and_pending(&session);
    assert!(notifications.is_empty());
    assert!(
        pending,
        "publishing the result must not terminate the idle waiter"
    );
    manager.enqueue_completion_notification(&snapshot);
    manager.enqueue_completion_notification(&snapshot);
    let (notifications, pending) = manager.take_notifications_and_pending(&session);
    assert_eq!(notifications.len(), 1);
    assert!(!pending);
    assert_eq!(
        manager
            .deliver_notifications(&session, &store)
            .unwrap()
            .len(),
        1
    );
    assert!(manager
        .deliver_notifications(&session, &store)
        .unwrap()
        .is_empty());
    let messages =
        crate::compact::prepare_messages_for_llm(&store.get_messages_for_prompt(&session).unwrap());
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.content.contains("FINAL_CONCLUSION"))
            .count(),
        1
    );
    assert!(manager
        .status_result(&task.task_id)
        .output
        .contains("FINAL_CONCLUSION"));
    assert!(manager
        .take_notifications_and_pending(&session)
        .0
        .is_empty());
}

#[test]
fn async_result_and_unconsumed_notification_survive_manager_restart() {
    let (_dir, store, manager, session) = fixture();
    let task = manager.create_task(&session, "python", true);
    manager.prepare_task(&task.task_id, None).unwrap();
    manager.finish(
        &task.task_id,
        &ToolResult {
            output: "Exit code: 1\nTraceback: failed".into(),
            is_error: true,
        },
    );
    drop(manager);
    let manager = AsyncTaskManager::new(store.clone()).unwrap();
    assert_eq!(
        manager.snapshot(&task.task_id).unwrap().status,
        AsyncTaskStatus::Failed
    );
    assert_eq!(manager.take_notifications_and_pending(&session).0.len(), 1);
    assert_eq!(
        manager
            .deliver_notifications(&session, &store)
            .unwrap()
            .len(),
        1
    );
    assert!(manager.cancel(&task.task_id).unwrap().is_error.unwrap());
}

#[test]
fn async_restart_marks_unfinished_processes_cancelled_and_retains_the_log() {
    let (_dir, store, manager, session) = fixture();
    let task = manager.create_task(&session, "python", true);
    let path = manager.prepare_task(&task.task_id, None).unwrap().unwrap();
    std::fs::write(&path, "before restart").unwrap();
    drop(manager);
    let recovered = AsyncTaskManager::new(store.clone()).unwrap();
    assert_eq!(
        recovered.get_task(&task.task_id).unwrap().status,
        AsyncTaskStatus::Cancelled
    );
    let (notifications, pending) = recovered.take_notifications_and_pending(&session);
    assert_eq!(notifications.len(), 1);
    assert!(!pending);
    assert!(notifications[0].contains(&path));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "before restart");
}

#[test]
fn async_large_result_retains_full_output_and_bounded_notification() {
    let (_dir, store, manager, session) = fixture();
    let task = manager.create_task(&session, "subagent", true);
    let path = manager.prepare_task(&task.task_id, None).unwrap().unwrap();
    let output = format!(
        "HEAD{}MIDDLE{}TAIL",
        "前".repeat(40_000),
        "后".repeat(40_000)
    );
    manager.finish(
        &task.task_id,
        &ToolResult {
            output: output.clone(),
            is_error: false,
        },
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), output);
    let delivered = manager.deliver_notifications(&session, &store).unwrap();
    assert!(delivered[0].chars().count() < 4_000);
    assert!(delivered[0].contains("Full output saved to:"));
    assert!(delivered[0].contains("HEAD") && delivered[0].contains("TAIL"));
}

#[test]
fn async_mode_is_pull_only_and_cancelled_notify_keeps_partial_log() {
    let (_dir, store, manager, session) = fixture();
    let plain = manager.create_task(&session, "bash", false);
    manager.prepare_task(&plain.task_id, None).unwrap();
    manager.finish(
        &plain.task_id,
        &ToolResult {
            output: "done".into(),
            is_error: false,
        },
    );
    assert!(manager
        .deliver_notifications(&session, &store)
        .unwrap()
        .is_empty());
    let task = manager.create_task(&session, "bash", true);
    let path = manager.prepare_task(&task.task_id, None).unwrap().unwrap();
    std::fs::write(&path, "partial output").unwrap();
    manager.append_output(&task.task_id, "partial output");
    manager.cancel_session(&session);
    let snapshot = manager
        .mark_cancelled_without_notification(&task.task_id)
        .unwrap();
    manager.enqueue_completion_notification(&snapshot);
    assert_eq!(
        manager.cancel(&task.task_id).unwrap().status,
        AsyncTaskStatus::Cancelled
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "partial output");
    assert!(manager.deliver_notifications(&session, &store).unwrap()[0].contains("partial output"));
}

#[tokio::test]
async fn async_completed_subagent_message_starts_one_new_attempt_in_existing_context() {
    let (_dir, store, manager, session) = fixture();
    let manager = Arc::new(manager);
    let (id, child) = named_child(&store, &manager, &session, "reviewer");
    manager.finish(
        &id,
        &ToolResult {
            output: "first result".into(),
            is_error: false,
        },
    );
    let (receipt, _) = manager
        .queue_task_message(&session, "reviewer", "follow-up")
        .unwrap();
    let expected_child = child.clone();
    let callback_store = store.clone();
    let invocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = invocations.clone();
    manager.register_resume_handler(
        &session,
        Arc::new(move |task, _, _| {
            assert_eq!(
                task.resume.as_ref().unwrap().child_session_id,
                expected_child
            );
            assert_eq!(task.attempt, 2);
            let store = callback_store.clone();
            let child = expected_child.clone();
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let messages = store.deliver_async_notifications(&child).unwrap();
                assert_eq!(messages.len(), 1);
                assert!(messages[0].contains("follow-up"));
                (
                    ToolResult {
                        output: "follow-up answer".into(),
                        is_error: false,
                    },
                    false,
                )
            })
        }),
    );
    assert!(manager.prepare_resume(&session, "reviewer").is_err());
    manager
        .start_continuation(
            &session,
            "reviewer",
            "process messages".into(),
            None,
            true,
            None,
        )
        .unwrap();
    let result = manager
        .wait_task(&session, "reviewer", 10_000)
        .await
        .unwrap();
    assert_eq!(result.status, AsyncTaskStatus::Completed);
    assert!(result
        .output
        .as_deref()
        .unwrap()
        .starts_with("follow-up answer"));
    assert_eq!(invocations.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!store
        .agent_message_pending(receipt["messageId"].as_str().unwrap())
        .unwrap());
    assert_eq!(
        manager
            .deliver_notifications(&session, &store)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn async_messages_can_be_queued_before_subagent_startup() {
    let (_dir, store, manager, session) = fixture();
    let task = manager.create_task(&session, "subagent", true);
    manager
        .prepare_named_task(&task.task_id, None, Some("reviewer"))
        .unwrap();
    manager
        .queue_task_message(&session, "reviewer", "queued instruction")
        .unwrap();
    assert!(manager
        .deliver_notifications(&session, &store)
        .unwrap()
        .is_empty());
    let child = store
        .create_session("Child", Some(&session), None, "chat", None)
        .unwrap();
    manager
        .bind_subagent(
            &task.task_id,
            SubagentResumeInfo {
                child_session_id: child.clone(),
                agent_id: "explorer".into(),
                working_dir: "F:/work".into(),
                model_id: "test".into(),
                effort: None,
                fast_mode: false,
                readonly: true,
            },
        )
        .unwrap();
    assert!(
        manager.deliver_notifications(&child, &store).unwrap()[0].contains("queued instruction")
    );
}

#[tokio::test]
async fn async_already_delivered_message_does_not_start_another_attempt() {
    let (_dir, store, manager, session) = fixture();
    let manager = Arc::new(manager);
    let (id, child) = named_child(&store, &manager, &session, "reviewer");
    manager.finish(
        &id,
        &ToolResult {
            output: "done".into(),
            is_error: false,
        },
    );
    let (receipt, _) = manager
        .queue_task_message(&session, "reviewer", "message")
        .unwrap();
    manager.deliver_notifications(&child, &store).unwrap();
    manager.register_resume_handler(
        &session,
        Arc::new(|_, _, _| {
            Box::pin(async { panic!("must not launch an already delivered message") })
        }),
    );
    let pending =
        communication::PendingDelivery::Message(receipt["messageId"].as_str().unwrap().to_string());
    assert!(manager
        .start_continuation(
            &session,
            "reviewer",
            String::new(),
            None,
            true,
            Some(pending)
        )
        .unwrap_err()
        .contains("already delivered"));
    assert_eq!(manager.get_task(&id).unwrap().attempt, 1);
}

#[test]
fn async_old_run_guard_cannot_overwrite_a_new_attempt() {
    let (_dir, store, manager, session) = fixture();
    let manager = Arc::new(manager);
    let (id, _) = named_child(&store, &manager, &session, "reviewer");
    let guard = manager.run_guard(&id);
    manager.finish(
        &id,
        &ToolResult {
            output: "network error".into(),
            is_error: true,
        },
    );
    manager.prepare_resume(&session, "reviewer").unwrap();
    drop(guard);
    assert_eq!(
        manager.get_task(&id).unwrap().status,
        AsyncTaskStatus::Queued
    );
    assert_eq!(manager.get_task(&id).unwrap().attempt, 2);
}
