use super::*;
use crate::session::models::SessionEventRecord;
use tempfile::tempdir;

fn record_turn(store: &SessionStore, session: &str, run: &str) -> Vec<String> {
    store.try_start_run(session, run).unwrap();
    let user = store
        .add_message(session, MessageRole::User, &format!("{run} input"))
        .unwrap();
    store
        .append_session_event(
            session,
            run,
            "userMessage",
            &json!({
                "message": { "id": user }
            })
            .to_string(),
        )
        .unwrap();
    let call: ToolCallInfo = serde_json::from_value(json!({
        "id": format!("{run}-call"), "name": "read", "arguments": "{}"
    }))
    .unwrap();
    let assistant = store
        .add_assistant_with_tool_calls(session, &format!("{run} tool"), &[call.clone()])
        .unwrap();
    store
        .append_session_event(
            session,
            run,
            "toolCallRoundDone",
            &json!({
                "messageId": assistant
            })
            .to_string(),
        )
        .unwrap();
    let tool = store
        .add_tool_result(session, &call.id, &format!("{run} output"))
        .unwrap();
    let answer = store
        .add_message(session, MessageRole::Assistant, &format!("{run} answer"))
        .unwrap();
    store
        .append_session_event(
            session,
            run,
            "done",
            &json!({ "messageId": answer }).to_string(),
        )
        .unwrap();
    for (iteration, attempt, status) in [(1, 1, "failed"), (1, 2, "completed"), (2, 1, "completed")]
    {
        store
            .record_context_attempt(
                session,
                run,
                iteration,
                attempt,
                "normal",
                status,
                "openai_codex",
                "test-model",
                Some("high"),
                &json!({
                    "instructions": "system context",
                    "input": [{"role": "user", "content": "historical-input-in-request"},
                        {"role": "user", "content": format!("{run} input")}],
                    "tools": [{"type": "function", "name": "read"}]
                }),
                &format!("{run} response"),
                None,
            )
            .unwrap();
    }
    store.update_run_status(run, "done", None).unwrap();
    vec![user, assistant, tool, answer]
}

#[test]
fn message_export_scopes_messages_attempts_runs_and_timeline_after_reopen() {
    let dir = tempdir().unwrap();
    let session;
    let selected;
    {
        let store = SessionStore::new(dir.path()).unwrap();
        session = store
            .create_session("Review turns", None, None, "chat", None)
            .unwrap();
        record_turn(&store, &session, "earlier");
        selected = record_turn(&store, &session, "selected");
        record_turn(&store, &session, "later");
        let child = store
            .create_session("Unrelated child", Some(&session), None, "chat", None)
            .unwrap();
        store
            .add_message(&child, MessageRole::User, "unrelated-child-content")
            .unwrap();
    }
    let store = SessionStore::new(dir.path()).unwrap();
    let snapshot = store.create_export_snapshot().unwrap();
    // Later writes and another run's streaming state must not enter the file.
    store
        .add_message(&session, MessageRole::User, "after-snapshot")
        .unwrap();
    let runtime: SessionRuntimeSnapshot = serde_json::from_value(json!({
        "activeRun": {"runId": "later", "sessionId": session, "status": "running", "startedAt": 1, "updatedAt": 1},
        "streamingText": "unrelated-live-output"
    })).unwrap();
    let live = ContextExportLiveSnapshot {
        captured_at: 1,
        sessions: HashMap::from([(
            session.clone(),
            ContextExportLiveSession {
                pending_inputs: vec![],
                runtime: Some(runtime),
            },
        )]),
    };

    for selected_id in &selected {
        let output = dir.path().join("message.yaml");
        let result =
            export_message_context_yaml(&snapshot, &session, selected_id, Some(&live), &output)
                .unwrap();
        assert_eq!(result.session_count, 1);
        assert_eq!(result.attempt_count, 3);
        let raw = std::fs::read_to_string(output).unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
        let exported = &yaml["sessions"][0];
        let ids: Vec<_> = exported["messages"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|message| message["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, selected);
        assert_eq!(
            yaml["source"]["selection"]["kind"].as_str(),
            Some("message_turn")
        );
        assert_eq!(
            yaml["source"]["selection"]["selected_message_id"].as_str(),
            Some(selected_id.as_str())
        );
        assert_eq!(exported["runs"].as_sequence().unwrap().len(), 1);
        assert_eq!(exported["runs"][0]["runId"].as_str(), Some("selected"));
        assert!(exported["timeline"]
            .as_sequence()
            .unwrap()
            .iter()
            .all(|event| event["runId"].as_str() == Some("selected")));
        assert!(exported["context_attempts"]
            .as_sequence()
            .unwrap()
            .iter()
            .all(|attempt| attempt["run_id"].as_str() == Some("selected")));
        for excluded in [
            "earlier input",
            "later input",
            "unrelated-child-content",
            "after-snapshot",
            "unrelated-live-output",
        ] {
            assert!(!raw.contains(excluded), "unexpected content: {excluded}");
        }
        assert!(raw.contains("historical-input-in-request"));
        assert!(raw.contains("selected output"));
        assert_eq!(exported["runtime"].as_str(), Some(EMPTY));
        assert_eq!(exported["token_usage"].as_str(), Some(EMPTY));
        let hash = yaml["integrity"]["content_hash"].as_str().unwrap();
        let unhashed = raw.replace(&format!("content_hash: {hash}"), "content_hash: empty");
        let expected: String = Sha256::digest(unhashed.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hash, expected);
    }

    let full = dir.path().join("full.yaml");
    let result = export_session_context_yaml(&snapshot, &session, "", None, None, &full).unwrap();
    assert_eq!(result.session_count, 2);
    assert_eq!(result.attempt_count, 9);
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(full).unwrap()).unwrap();
    assert!(yaml["source"]["selection"].is_null());
    assert_eq!(
        yaml["sessions"][0]["messages"].as_sequence().unwrap().len(),
        12
    );
}

#[test]
fn message_export_uses_explicit_empty_for_unlinked_historical_attempts() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Legacy", None, None, "chat", None)
        .unwrap();
    store
        .add_message(&session, MessageRole::User, "older input")
        .unwrap();
    store
        .add_message(&session, MessageRole::Assistant, "older answer")
        .unwrap();
    let user = store
        .add_message(&session, MessageRole::User, "selected input")
        .unwrap();
    let answer = store
        .add_message(&session, MessageRole::Assistant, "selected answer")
        .unwrap();
    store
        .add_message(&session, MessageRole::User, "newer input")
        .unwrap();
    store
        .record_context_attempt(
            &session,
            "unlinked",
            1,
            1,
            "normal",
            "completed",
            "legacy",
            "test",
            None,
            &json!({"input": "must-not-export-unlinked-request"}),
            "unlinked response",
            None,
        )
        .unwrap();
    let output = dir.path().join("legacy.yaml");
    let result = export_message_context_yaml(&store, &session, &answer, None, &output).unwrap();
    assert_eq!(result.capture_quality, "reconstructed");
    assert_eq!(result.attempt_count, 0);
    let raw = std::fs::read_to_string(output).unwrap();
    let yaml: serde_yaml::Value = serde_yaml::from_str(&raw).unwrap();
    assert_eq!(
        yaml["sessions"][0]["messages"].as_sequence().unwrap().len(),
        2
    );
    assert_eq!(
        yaml["sessions"][0]["messages"][0]["id"].as_str(),
        Some(user.as_str())
    );
    assert_eq!(
        yaml["sessions"][0]["context_attempts"].as_str(),
        Some(EMPTY)
    );
    assert!(!raw.contains("must-not-export-unlinked-request"));
    assert!(raw.contains("no persisted provider attempts can be linked"));
}

#[test]
fn message_export_rejects_unknown_message_without_writing_a_session_export() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Source", None, None, "chat", None)
        .unwrap();
    let output = dir.path().join("missing.yaml");
    assert!(export_message_context_yaml(&store, &session, "wrong-message", None, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn message_turn_selection_includes_steering_inputs_and_render_part_run_ids() {
    fn message(id: &str, role: &str, run: Option<&str>) -> ChatMessage {
        let mut value = json!({"id": id, "role": role, "content": id, "createdAt": 100});
        if let Some(run) = run {
            value["renderParts"] = json!([{"kind": "text", "id": id, "order": {"runId": run, "seq": 1}, "content": id}]);
        }
        serde_json::from_value(value).unwrap()
    }
    let messages = vec![
        message("before", "user", None),
        message("before-answer", "assistant", Some("before")),
        message("user", "user", None),
        message("intermediate", "assistant", Some("selected")),
        message("steering", "user", None),
        message("answer", "assistant", Some("selected")),
        message("after", "user", None),
        message("after-answer", "assistant", Some("after")),
    ];
    let events = ["user", "steering"].map(|id| SessionEventRecord {
        session_id: "session".into(),
        run_id: "selected".into(),
        seq: 1,
        event_type: "userMessage".into(),
        payload: json!({"message": {"id": id}}),
        created_at: 100,
    });
    for id in ["user", "intermediate", "steering", "answer"] {
        let selection = MessageTurnSelection::resolve(&messages, &events, id).unwrap();
        assert_eq!(
            selection.message_ids,
            ["user", "intermediate", "steering", "answer"]
                .map(str::to_string)
                .into()
        );
        assert_eq!(selection.run_ids, ["selected".to_string()].into());
    }
}
