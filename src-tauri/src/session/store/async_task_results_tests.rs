use super::*;

#[test]
fn async_delivery_failure_retries_without_losing_or_duplicating_results() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path()).unwrap());
    let session = store
        .create_session("Retry", None, None, "chat", None)
        .unwrap();
    let manager = crate::async_tasks::AsyncTaskManager::new(store.clone()).unwrap();
    let task = manager.create_task(&session, "bash", true);
    manager.prepare_task(&task.task_id, None).unwrap();
    manager.finish(
        &task.task_id,
        &crate::tool::ToolResult {
            output: "retriable result".into(),
            is_error: false,
        },
    );
    store.conn.lock().unwrap().execute_batch("CREATE TRIGGER reject_async_delivery BEFORE INSERT ON messages
        WHEN NEW.id LIKE 'async-result:%' BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").unwrap();
    assert!(manager.deliver_notifications(&session, &store).is_err());
    assert_eq!(
        store.pending_async_notifications(&session).unwrap().len(),
        1
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_async_delivery;")
        .unwrap();
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
}

#[test]
fn async_background_result_survives_foreground_round_finalization() {
    let call: ToolCallInfo = serde_json::from_value(serde_json::json!({
        "id":"tool-1", "name":"bash", "arguments":"{\"async\":\"notify\"}",
        "outcome":"done", "recordedOutput":"actual result"
    }))
    .unwrap();
    let mut incoming = call.clone();
    incoming.recorded_output = None;
    incoming.outcome = Some(crate::commands::ToolCallOutcome::Done);
    SessionStore::preserve_background_results(std::slice::from_mut(&mut incoming), &[call]);
    assert_eq!(incoming.recorded_output.as_deref(), Some("actual result"));
}

#[test]
fn v41_migration_preserves_old_session_export_and_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("locus.db")).unwrap();
    SessionStore::create_latest_schema(&conn).unwrap();
    conn.execute_batch("DROP TABLE agent_messages; DROP TABLE async_task_notifications; DROP TABLE async_task_results;
        INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('legacy','Legacy','chat',1,1);
        INSERT INTO messages(id,session_id,role,content,created_at) VALUES ('old','legacy','user','old context',1);
        PRAGMA user_version = 41;").unwrap();
    drop(conn);
    let store = SessionStore::new(dir.path()).unwrap();
    assert_eq!(
        store.export_async_tasks("legacy").unwrap(),
        serde_json::json!("empty")
    );
    let conn = store.conn.lock().unwrap();
    SessionStore::create_async_task_schema(&conn).unwrap();
    assert_eq!(
        conn.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        SessionStore::schema_version()
    );
    drop(conn);
    let output = dir.path().join("context.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, "legacy", "", None, None, &output,
    )
    .unwrap();
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(yaml["sessions"][0]["async_tasks"].as_str(), Some("empty"));
    assert!(store
        .get_messages_for_prompt("legacy")
        .unwrap()
        .iter()
        .any(|m| m.content == "old context"));
}

#[test]
fn async_v42_migration_keeps_delivery_flags_assigns_local_names_and_exports_empty_fields() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("locus.db")).unwrap();
    SessionStore::create_latest_schema(&conn).unwrap();
    conn.execute_batch("DROP TABLE agent_messages; DROP TABLE async_task_notifications;
        INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('legacy','Legacy','chat',1,1);
        PRAGMA user_version = 42;").unwrap();
    for (id, delivered) in [("old-a", false), ("old-b", true)] {
        let snapshot = serde_json::json!({"taskId":id,"sessionId":"legacy","toolName":"subagent", "status":"failed",
            "createdAt":1000,"updatedAt":2000,"finishedAt":2000,"notify":true,"output":"old failure","isError":true});
        conn.execute("INSERT INTO async_task_results(task_id,session_id,snapshot_json,reminder,delivered) VALUES(?1,'legacy',?2,?3,?4)", params![id,snapshot.to_string(),format!("old result {id}"),delivered]).unwrap();
    }
    drop(conn);
    let store = SessionStore::new(dir.path()).unwrap();
    let tasks = store.list_async_tasks("legacy").unwrap();
    assert_eq!(
        tasks.iter().map(|t| t.public_id()).collect::<Vec<_>>(),
        ["t1", "t2"]
    );
    assert_eq!(tasks[0].attempt, 1);
    assert!(tasks[0].resume.is_none());
    assert_eq!(
        store.pending_async_notifications("legacy").unwrap().len(),
        1
    );
    assert_eq!(
        store.deliver_async_notifications("legacy").unwrap(),
        ["old result old-a"]
    );
    SessionStore::migrate_async_task_attempts(&store.conn.lock().unwrap()).unwrap();
    assert!(store
        .deliver_async_notifications("legacy")
        .unwrap()
        .is_empty());
    let output = dir.path().join("legacy-context.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, "legacy", "", None, None, &output,
    )
    .unwrap();
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(
        yaml["sessions"][0]["async_tasks"][0]["resume"].as_str(),
        Some("empty")
    );
    assert_eq!(
        yaml["sessions"][0]["async_tasks"][0]["assistantMessageId"].as_str(),
        Some("empty")
    );
    assert_eq!(
        yaml["sessions"][0]["agent_messages"].as_str(),
        Some("empty")
    );
}

#[test]
fn async_agent_message_transaction_retries_and_exports_queued_messages() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let source = store
        .create_session("Source", None, None, "chat", None)
        .unwrap();
    let target = store
        .create_session("Target", Some(&source), None, "chat", None)
        .unwrap();
    let id = store
        .queue_agent_message(&source, &target, "parent", "message body", None)
        .unwrap();
    assert_eq!(
        store.export_agent_messages(&target).unwrap()[0]["body"],
        "message body"
    );
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_agent_mail BEFORE INSERT ON messages
        WHEN NEW.id LIKE 'agent-message:%' BEGIN SELECT RAISE(ABORT, 'injected error'); END;",
        )
        .unwrap();
    assert!(store.deliver_async_notifications(&target).is_err());
    assert!(store.agent_message_pending(&id).unwrap());
    store
        .conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_agent_mail")
        .unwrap();
    assert_eq!(store.deliver_async_notifications(&target).unwrap().len(), 1);
    assert!(!store.agent_message_pending(&id).unwrap());
    assert!(store
        .deliver_async_notifications(&target)
        .unwrap()
        .is_empty());
}
