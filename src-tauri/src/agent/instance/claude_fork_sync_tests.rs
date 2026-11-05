use super::*;
use claude_code_cli::recovery::{self, NativeSnapshot};
use serde_json::json;

const INPUT: &str = "我希望在正式场合使用克制写实的军政剧对白，请给出已有作品的段落参考";

fn fork_fixture() -> (
    tempfile::TempDir,
    SessionStore,
    String,
    claude_code_cli::ssh::SshConfig,
    NativeSnapshot,
) {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let source = store
        .create_session("source", None, None, "chat", None)
        .unwrap();
    let remote: claude_code_cli::ssh::SshConfig = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let mut native: NativeSnapshot = serde_json::from_value(json!({
        "version":1, "nativeId":uuid::Uuid::new_v4(), "digest":"a".repeat(64),
        "messages":[
            {"key":"user", "role":"user", "uuids":[uuid::Uuid::new_v4()], "content":"分析文风"},
            {"key":"assistant", "role":"assistant", "uuids":[uuid::Uuid::new_v4()],
             "content":[{"type":"text", "text":"这里是五种语言风格方案"}], "stopReason":"end_turn"}
        ],
        "todos":[], "pendingToolIds":[], "incompleteTail":false
    }))
    .unwrap();
    store
        .reconcile_claude_native(&source, &remote, &native, &[], None)
        .unwrap();
    let boundary = store
        .get_messages(&source)
        .unwrap()
        .last()
        .unwrap()
        .id
        .clone();
    store
        .add_message(&source, MessageRole::User, "later source request")
        .unwrap();
    store
        .add_message(&source, MessageRole::Assistant, "later source answer")
        .unwrap();
    let fork = store
        .fork_session_from_message(&source, &boundary, Some("fork"))
        .unwrap();
    // The SSH fixture represents the frozen native transcript prepared by fork::prepare.
    native.native_id = uuid::Uuid::new_v4().to_string();
    store
        .bind_claude_native_session(&fork, &remote, &native.native_id)
        .unwrap();
    for (id, mut metadata) in store.get_response_request_metadata(&fork).unwrap() {
        metadata["claude_checkpoint"]["sessionId"] = json!(native.native_id);
        store
            .set_message_response_request_metadata(&fork, &id, &metadata)
            .unwrap();
        store
            .set_message_cli_session_id(&fork, &id, "claude_code", &native.native_id)
            .unwrap();
    }
    assert_eq!(store.get_messages(&fork).unwrap().len(), 2);
    (root, store, fork, remote, native)
}

fn save_input(store: &SessionStore, id: &str, with_receipt: bool) -> String {
    store
        .add_message_with_images_asset_refs_signature_and_response(
            id,
            MessageRole::User,
            INPUT,
            None,
            None,
            None,
            None,
            None,
            with_receipt.then(pending_input_metadata).as_ref(),
        )
        .unwrap()
}

#[tokio::test]
async fn claude_fork_immediate_send_survives_inflight_background_history_sync() {
    // Also protect older/in-flight writers before they have saved a receipt.
    for with_receipt in [false, true] {
        let (root, store, id, remote, native) = fork_fixture();
        let (fetch_started, fetch_started_rx) = tokio::sync::oneshot::channel();
        let (finish_fetch, finish_fetch_rx) = tokio::sync::oneshot::channel();
        let frozen = native.clone();
        let background =
            recovery::synchronize_with_fetch(&store, &id, None, false, false, |_, _| async move {
                fetch_started.send(()).unwrap();
                finish_fetch_rx.await.unwrap();
                Ok(frozen)
            });
        let send = async {
            fetch_started_rx.await.unwrap();
            store.try_start_run(&id, "new-run").unwrap();
            let input = save_input(&store, &id, with_receipt);
            finish_fetch.send(()).unwrap();
            input
        };
        let (report, input) = tokio::join!(background, send);
        assert!(
            !report.unwrap().changed,
            "background synchronization must yield to the new run"
        );
        let before_send = store.get_messages(&id).unwrap();
        assert_eq!(before_send.len(), 3);
        assert_eq!(before_send.last().unwrap().id, input);
        assert_eq!(before_send.last().unwrap().content, INPUT);

        // The sending run performs its own synchronization with the exact input identity.
        recovery::synchronize_with_fetch(&store, &id, Some(&input), false, false, |_, _| async {
            Ok(native.clone())
        })
        .await
        .unwrap();
        let wire =
            prepare_pending_claude_input(&store, &id, Some(&input), INPUT, &Some(remote), "")
                .unwrap();
        assert_eq!(wire["locusMessageId"], input);
        assert!(wire["content"].as_str().unwrap().contains(INPUT));
        assert!(!wire["content"]
            .as_str()
            .unwrap()
            .contains("Continue from the existing conversation"));
        assert_eq!(store.get_messages(&id).unwrap().len(), 3);
        store
            .update_run_status("new-run", "error", Some("interrupted before submission"))
            .unwrap();
        drop(store);
        let reopened = SessionStore::new(root.path()).unwrap();
        let display = reopened.get_messages_for_display(&id).unwrap();
        assert_eq!(display.len(), 3);
        assert_eq!(display.last().unwrap().id, input);
        assert_eq!(display.last().unwrap().content, INPUT);
        let output = root.path().join("fork.yaml");
        crate::session::context_export::export_session_context_yaml(
            &reopened, &id, "F:/test", None, None, &output,
        )
        .unwrap();
        let exported = std::fs::read_to_string(output).unwrap();
        assert!(exported.contains(INPUT));
        assert!(!exported.contains("Continue from the existing conversation"));
    }
}

#[tokio::test]
async fn claude_fork_background_sync_preserves_durable_input_before_any_run_event() {
    let (_root, store, id, _remote, mut native) = fork_fixture();
    let input = save_input(&store, &id, true);
    let receipt =
        store.get_response_request_metadata(&id).unwrap()[&input]["claude_pending_input"].clone();
    // Exercise a changed source and a required recovery notice, which formerly
    // selected authoritative replacement even on passive view hydration.
    native.digest = "b".repeat(64);
    native
        .pending_tool_ids
        .push("unknown-interrupted-call".into());
    let report = recovery::synchronize_with_fetch(&store, &id, None, false, false, |_, _| async {
        Ok(native)
    })
    .await
    .unwrap();
    assert!(report.changed);
    assert!(report.resume_context.is_some());
    let messages = store.get_messages(&id).unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages.last().unwrap().id, input);
    assert_eq!(messages.last().unwrap().content, INPUT);
    let metadata = store.get_response_request_metadata(&id).unwrap();
    assert_eq!(metadata[&input]["claude_pending_input"], receipt);
    assert_eq!(
        store
            .claude_recovery_pending_messages(&id, &messages, &metadata, None)
            .unwrap(),
        [input]
    );
}

#[tokio::test]
async fn claude_fork_background_sync_does_not_rebuild_unverified_local_input() {
    let (_root, store, id, _remote, native) = fork_fixture();
    let input = save_input(&store, &id, false);
    let error = recovery::synchronize_with_fetch(&store, &id, None, false, false, |_, _| async {
        Ok(native)
    })
    .await
    .unwrap_err();
    assert!(error.contains("unverified history"));
    let messages = store.get_messages(&id).unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages.last().unwrap().id, input);
    assert_eq!(messages.last().unwrap().content, INPUT);
}

#[test]
fn claude_fork_missing_or_unacknowledged_input_never_becomes_continuation() {
    for state in [
        "missing",
        "unacknowledged",
        "wrong-session",
        "invalid-checkpoint",
    ] {
        let (_root, store, id, remote, native) = fork_fixture();
        let input = if state == "missing" {
            uuid::Uuid::new_v4().to_string()
        } else {
            save_input(&store, &id, true)
        };
        if state == "unacknowledged" {
            // A non-tail input without a receipt used to be mistaken for an ACK.
            store
                .add_message(&id, MessageRole::Assistant, "unrelated cached output")
                .unwrap();
        } else if state != "missing" {
            let session = if state == "wrong-session" {
                uuid::Uuid::new_v4().to_string()
            } else {
                native.native_id
            };
            let uuid = if state == "invalid-checkpoint" {
                "invalid".to_string()
            } else {
                uuid::Uuid::new_v4().to_string()
            };
            store.set_message_response_request_metadata(&id, &input, &json!({
                "backend":"claude_code_cli", "claude_checkpoint":{"version":1,"uuid":uuid,"sessionId":session},
            })).unwrap();
        }
        let before = store.get_messages(&id).unwrap();
        assert!(
            prepare_pending_claude_input(&store, &id, Some(&input), INPUT, &Some(remote), "")
                .is_err(),
            "{state}"
        );
        assert_eq!(
            serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
}
