//! Regression coverage for native stream snapshots saved after failure/cancel.
//! All history is synthetic and stored in a temporary database; no CLI/network.
use super::*;
use crate::llm::claude_code_cli::{connection::NativeConnection, recovery};
use serde_json::json;
use uuid::Uuid;

fn fixture() -> (NativeConnection, recovery::NativeSnapshot) {
    let remote = NativeConnection::server(Uuid::new_v4().to_string()).unwrap();
    let snapshot = serde_json::from_value(json!({
        "version": 1,
        "nativeId": Uuid::new_v4(),
        "digest": "a".repeat(64),
        "messages": [
            {"key":"user", "role":"user", "uuids":[Uuid::new_v4()], "content":"request"},
            {"key":"answer", "role":"assistant", "uuids":[Uuid::new_v4()],
             "content":[{"type":"text", "text":"The complete native answer."}], "stopReason":"end_turn"}
        ],
        "todos": [], "taskSnapshot": {}, "incompleteTail": false, "pendingToolIds": []
    })).unwrap();
    (remote, snapshot)
}

fn native_user(store: &SessionStore, id: &str, remote: &NativeConnection, snapshot: &recovery::NativeSnapshot) {
    store.bind_claude_native_session(id, remote, &snapshot.native_id).unwrap();
    store.add_message_with_thinking(id, MessageRole::User, "request", None, None, None, None,
        Some(&json!({"backend":"claude_code_cli", "claude_connection":remote,
            "claude_checkpoint":{"version":1, "sessionId":snapshot.native_id, "uuid":snapshot.messages[0].uuids[0]}}))).unwrap();
}

#[tokio::test]
async fn claude_stream_error_and_cancel_snapshots_synchronize_without_api_history_warning() {
    // Complete answer + late guard failure was the reported incident. A
    // partially received answer must also retain provenance, without claiming
    // that its text or the previous response's checkpoint was committed.
    for status in ["error", "cancelled"] {
        for text in ["The complete native answer.", "The complete"] {
            let root = tempfile::tempdir().unwrap();
            let store = SessionStore::new(root.path()).unwrap();
            let id = store.create_session("native interruption", None, None, "chat", None).unwrap();
            let run = format!("{id}-run");
            let (remote, snapshot) = fixture();
            native_user(&store, &id, &remote, &snapshot);
            store.try_start_run(&id, &run).unwrap();
            let stream = AssistantStreamState::default();
            let metadata = json!({"backend":"claude_code_cli", "model":"claude_code/test", "upstream_model":"test",
                "claude_connection":remote, "claude_native_tool_search":true,
                "claude_checkpoint":{"uuid":snapshot.messages[0].uuids[0], "sessionId":snapshot.native_id, "version":1},
                "claude_pending_input":{"version":1,"token":Uuid::new_v4()},
                "claude_native":{"key":"previous-row"}, "claude_usage":{"output_tokens":100},
                "claude_cache_request":{"key":"previous-request"}, "claude_timing":{"outputTokens":100}});
            track_claude_stream_origin(&stream, &metadata, &run);
            stream.append_text(text);
            let interrupted = AgentInstance::persist_interrupted_assistant_snapshot(&store, &id, &stream.snapshot()).unwrap();
            store.update_run_status(&run, status, None).unwrap();
            let saved = store.get_response_request_metadata(&id).unwrap()[&interrupted.message_id].clone();
            assert_eq!(saved["backend"], "claude_code_cli");
            assert_eq!(saved["claude_connection"], json!(remote));
            assert_eq!(saved["claude_run_id"], run);
            assert_eq!(saved["upstream_model"], "test");
            for key in ["claude_checkpoint", "claude_pending_input", "claude_native", "claude_usage", "claude_cache_request", "claude_timing"] {
                assert!(saved.get(key).is_none(), "must not invent/copy {key}");
            }
            let report = recovery::synchronize_with_fetch(&store, &id, None, false, false, |bound, native| {
                assert_eq!(bound, remote);
                assert_eq!(native.as_deref(), Some(snapshot.native_id.as_str()));
                let snapshot = snapshot.clone();
                async move { Ok(snapshot) }
            }).await.unwrap();
            assert!(report.changed);
            assert!(report.warning.is_none());
            let messages = store.get_messages(&id).unwrap();
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].id, interrupted.message_id);
            assert_eq!(messages[1].content, "The complete native answer.");
            let saved = store.get_response_request_metadata(&id).unwrap();
            assert_eq!(saved[&interrupted.message_id]["claude_checkpoint"]["uuid"], snapshot.messages[1].uuids[0]);
            assert!(messages.iter().all(|m| m.role != MessageRole::Tool));
            let report = recovery::synchronize_with_fetch(&store, &id, None, false, false,
                |_, _| async { Ok(snapshot.clone()) }).await.unwrap();
            assert!(!report.changed, "reopening must be idempotent");
            claude_code_cli::clear_session_id(&id).await;
        }
    }
}

#[tokio::test]
async fn claude_stream_origin_does_not_exempt_api_or_unverified_history() {
    for metadata in [None, Some(json!({"backend":"openai"}))] {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("mixed history", None, None, "chat", None).unwrap();
        let (remote, snapshot) = fixture();
        native_user(&store, &id, &remote, &snapshot);
        let stream = AssistantStreamState::default();
        if let Some(metadata) = metadata { stream.set_response_metadata(metadata); }
        // Even identical text plus an existing native session binding is not
        // producer provenance; no arbitrary API row may be relabeled native.
        stream.append_text("The complete native answer.");
        let saved = AgentInstance::persist_interrupted_assistant_snapshot(&store, &id, &stream.snapshot()).unwrap();
        let before = serde_json::to_value(store.get_messages(&id).unwrap()).unwrap();
        let error = recovery::synchronize_with_fetch(&store, &id, None, false, false,
            |_, _| async { Ok(snapshot) }).await.unwrap_err();
        assert!(error.contains("API or unverified history"));
        assert_eq!(serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(), before);
        assert!(store.get_response_request_metadata(&id).unwrap().get(&saved.message_id)
            .is_none_or(|value| value["backend"] != "claude_code_cli"));
    }
}

#[test]
fn claude_stream_origin_is_reestablished_after_tool_round_and_cleared_between_runs() {
    let stream = AssistantStreamState::default();
    let metadata = json!({"backend":"claude_code_cli", "claude_connection":null});
    track_claude_stream_origin(&stream, &metadata, "native-run");
    stream.append_text("tool round");
    stream.mark_persisted("round".into(), "tool round".into(), None, None);
    // The native host resets partial output after each completed tool round.
    stream.reset();
    track_claude_stream_origin(&stream, &metadata, "native-run");
    stream.append_thinking("interrupted thinking");
    let snapshot = stream.snapshot();
    assert_eq!(snapshot.response_metadata.as_ref().unwrap()["backend"], "claude_code_cli");
    assert!(snapshot.persisted_message_id.is_none());
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("thinking", None, None, "chat", None).unwrap();
    let saved = AgentInstance::persist_interrupted_assistant_snapshot(&store, &id, &snapshot).unwrap();
    assert_eq!(store.get_response_request_metadata(&id).unwrap()[&saved.message_id]["claude_run_id"], "native-run");
    stream.mark_persisted(saved.message_id.clone(), snapshot.text, Some(snapshot.thinking_content), None);
    let again = AgentInstance::persist_interrupted_assistant_snapshot(&store, &id, &stream.snapshot()).unwrap();
    assert_eq!(again.message_id, saved.message_id);
    assert_eq!(store.get_messages(&id).unwrap().len(), 1);
    // General error/cancel handling can now persist a subsequent API run. A
    // prior native origin must not survive reset and bless that other backend.
    stream.reset();
    stream.append_text("API response");
    assert!(stream.snapshot().response_metadata.is_none());
}
