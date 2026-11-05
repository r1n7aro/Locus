use super::*;
use serde_json::json;

#[test]
fn retry_recognizes_completed_native_answer_but_not_old_or_unfinished_output() {
    let mut native = tests::snapshot();
    native.pending_tool_ids.clear();
    native.messages[1].stop_reason = Some("end_turn".into());
    let receipt = json!({"claude_checkpoint":{"uuid":native.messages[0].uuids[0]}});
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_some());
    assert!(completed_native_message(&native, None, &[]).is_none());
    assert!(completed_native_message(
        &native,
        Some(&json!({"claude_checkpoint":{"uuid":uuid::Uuid::new_v4()}})),
        &[]
    )
    .is_none());
    assert!(
        completed_native_message(&native, Some(&receipt), &["unsent steering".into()]).is_none()
    );
    native.incomplete_tail = true;
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_none());
    native.incomplete_tail = false;
    native.pending_tool_ids.push("unknown write".into());
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_none());
    native.pending_tool_ids.clear();
    native.messages[1].stop_reason = None;
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_none());
    native.messages[1].stop_reason = Some("max_tokens".into());
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_none());
}

#[test]
fn retry_completion_can_use_input_token_when_native_ack_was_lost() {
    let mut native = tests::snapshot();
    native.pending_tool_ids.clear();
    native.messages[1].stop_reason = Some("end_turn".into());
    let token = uuid::Uuid::new_v4().to_string();
    native.messages[0].content = json!(pending_input_text(&token, "write once"));
    let receipt = json!({"claude_pending_input":{"version":1,"token":token}});
    assert!(completed_native_message(&native, Some(&receipt), &[]).is_some());
}

#[test]
fn retry_rebuild_keeps_every_proven_unsent_input_and_does_not_change_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store
        .create_session("automatic recovery", None, None, "chat", None)
        .unwrap();
    let remote: ssh::SshConfig =
        serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let native = tests::snapshot();
    store
        .bind_claude_native_session(&id, &remote, &native.native_id)
        .unwrap();
    store
        .reconcile_claude_native(&id, &remote, &native, &[], None)
        .unwrap();
    let first = store
        .add_message(&id, MessageRole::User, "queued instruction one")
        .unwrap();
    let second = store
        .add_message(&id, MessageRole::User, "queued instruction two")
        .unwrap();
    for message in [&first, &second] {
        store
            .set_message_response_request_metadata(
                &id,
                message,
                &json!({
                    "backend":"claude_code_cli", "claude_connection":remote,
                    "claude_pending_input":{"version":1,"token":uuid::Uuid::new_v4()},
                }),
            )
            .unwrap();
    }
    let before = store.get_messages(&id).unwrap();
    store
        .rebuild_claude_native_for_retry(&id, &remote, &native, &before, Some(&first))
        .unwrap();
    let after = store.get_messages(&id).unwrap();
    assert_eq!(after[after.len() - 2].id, first);
    assert_eq!(after.last().unwrap().id, second);
    let metadata = store.get_response_request_metadata(&id).unwrap();
    assert_eq!(
        store
            .claude_recovery_pending_messages(&id, &after, &metadata, Some(&first))
            .unwrap(),
        [first, second]
    );
    let mut wrong = native.clone();
    wrong.native_id = uuid::Uuid::new_v4().to_string();
    assert!(store
        .rebuild_claude_native_for_retry(&id, &remote, &wrong, &after, None)
        .is_err());
    assert_eq!(
        native_session_id(&store, &id).unwrap().as_deref(),
        Some(native.native_id.as_str())
    );
}

#[tokio::test]
async fn claude_history_retention_survives_automatic_reconciliation_failure_fallback() {
    // App println/eprintln macros emit tracing events rather than stderr.
    // Capture the actual subscriber output to verify the recovery audit trail.
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for LogCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt().without_time().with_ansi(false)
        .with_writer(move || LogCapture(writer.clone())).finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("retry fallback", None, None, "chat", None).unwrap();
    let remote: ssh::SshConfig = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let mut original = tests::snapshot();
    original.pending_tool_ids.clear();
    store.bind_claude_native_session(&id, &remote, &original.native_id).unwrap();
    store.reconcile_claude_native(&id, &remote, &original, &[], None).unwrap();
    let old_ids: Vec<_> = store.get_messages(&id).unwrap().into_iter().map(|m| m.id).collect();
    // A stale unverified row makes ordinary reconciliation fail. Automatic
    // fallback must not take confirmed history with it when rebuilding.
    store.add_message(&id, MessageRole::Assistant, "unverified drift").unwrap();
    let pending = store.add_message(&id, MessageRole::User, "continue").unwrap();
    store.set_message_response_request_metadata(&id, &pending, &json!({"backend":"claude_code_cli",
        "claude_pending_input":{"version":1,"token":uuid::Uuid::new_v4()}})).unwrap();
    let mut summary = original.messages[0].clone();
    summary.key = "summary".into();
    summary.uuids = vec![uuid::Uuid::new_v4().to_string()];
    summary.content = json!("native summary");
    summary.compact_summary = true;
    original.messages = vec![summary];
    original.digest = "b".repeat(64);
    let recovered = synchronize_with_fetch(&store, &id, Some(&pending), false, true,
        |_, _| async move { Ok(original) }).await.unwrap();
    assert!(recovered.changed);
    let transcript = store.get_messages(&id).unwrap();
    assert_eq!(transcript.len(), 4);
    assert_eq!(transcript[..2].iter().map(|m| &m.id).collect::<Vec<_>>(), old_ids.iter().collect::<Vec<_>>());
    assert!(!transcript.iter().any(|m| m.content == "unverified drift"));
    let prompt = store.get_messages_for_prompt(&id).unwrap();
    assert_eq!(prompt.len(), 2);
    assert!(!prompt.iter().any(|m| old_ids.contains(&m.id)));
    assert!(prompt.iter().any(|m| m.id == pending));
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("reason=reconciliation_failed:"));
    assert!(logs.contains("This conversation contains API or unverified history"));
    assert!(logs.contains("retained_history=2") && logs.contains("backup="));
}

#[tokio::test]
async fn history_handshake_timeout_retries_sync_before_any_input_or_tool_replay() {
    use super::super::{native_journal::Journal, turn_retry};
    use std::sync::atomic::{AtomicU32, Ordering};
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("history handshake recovery", None, None, "chat", None).unwrap();
    let remote: ssh::SshConfig = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let mut native = tests::snapshot();
    native.pending_tool_ids.clear();
    store.bind_claude_native_session(&id, &remote, &native.native_id).unwrap();
    store.reconcile_claude_native(&id, &remote, &native, &[], None).unwrap();
    let input = store.get_messages(&id).unwrap()[0].id.clone();
    native.messages[1].stop_reason = Some("end_turn".into());
    native.messages[1].content = json!([{"type":"text","text":"Updated once."}]);
    native.digest = "b".repeat(64);
    let journal = Journal::new(root.path().join("calls"));
    let effect = root.path().join("effect");
    let executions = AtomicU32::new(0);
    let synchronizations = AtomicU32::new(0);
    let submissions = AtomicU32::new(0);
    let (_send, receive) = tokio::sync::watch::channel(false);
    let (store, id, native, journal, effect, input) = (&store, &id, &native, &journal, &effect, &input);
    let (executions, synchronizations, submissions) = (&executions, &synchronizations, &submissions);
    let result = turn_retry::run(
        |attempt| async move {
            if attempt == 1 {
                let call = json!({"callId":"write-once","tool":"Write","input":{"file_path":"asset","content":"updated"}});
                assert!(journal.begin(&call)?.is_none());
                executions.fetch_add(1, Ordering::SeqCst);
                std::fs::write(effect, "updated once").unwrap();
                journal.complete("write-once", &json!({"callId":"write-once","result":{"type":"create"}}))?;
                return Err(turn_retry::disconnected("lost tool result ACK"));
            }
            let recovered = synchronize_with_fetch(store, id, Some(input), false, true, |_, _| async move {
                synchronizations.fetch_add(1, Ordering::SeqCst);
                if attempt == 2 {
                    let mut handshake = ssh::startup::Handshake::new(&json!({"mode":"history"}));
                    handshake.progress(&json!({"type":"progress","protocol":1,"stage":"history"}));
                    Err(handshake.failed(true, Duration::from_secs(45), None, b""))
                } else { Ok(native.clone()) }
            }).await?;
            // Mirrors the production early completion branch, before run_turn.
            if let Some(message) = recovered.completed_message { return Ok(message.content); }
            submissions.fetch_add(1, Ordering::SeqCst);
            Err("History did not prove completion; test must not replay input".into())
        },
        |_, _| async { Ok(()) },
        receive,
        &[Duration::ZERO; 3],
    ).await.unwrap();
    assert_eq!(result.as_deref(), Some("Updated once."));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(synchronizations.load(Ordering::SeqCst), 2);
    assert_eq!(submissions.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(effect).unwrap(), "updated once");
}

#[tokio::test]
async fn hook_silence_transport_recovery_never_reexecutes_completed_or_unknown_tools() {
    use super::super::{bridge_health::{BridgeHealth, HEARTBEAT_LEASE}, native_journal::Journal, turn_retry};
    use std::sync::atomic::{AtomicU32, Ordering};
    for completed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let journal = Journal::new(root.path().join("calls"));
        let executions = AtomicU32::new(0);
        let synchronizations = AtomicU32::new(0);
        let (_send, receive) = tokio::sync::watch::channel(false);
        let call = json!({"callId":"write-once","tool":"Write","input":{"file_path":"effect","content":"updated"}});
        let result = turn_retry::run(
            |attempt| {
                if attempt > 1 { assert_eq!(synchronizations.load(Ordering::SeqCst), 1); }
                // The real dispatcher consults this journal before execution.
                if journal.begin(&call).unwrap().is_none() {
                    executions.fetch_add(1, Ordering::SeqCst);
                    std::fs::write(root.path().join("effect"), "updated once").unwrap();
                    if completed {
                        journal.complete("write-once", &json!({"callId":"write-once","result":{"type":"create"}})).unwrap();
                    }
                }
                async move {
                    if attempt == 1 {
                        let now = Instant::now();
                        let mut health = BridgeHealth::default();
                        health.begin_turn("run", now);
                        let failure = turn_retry::disconnected("SSH transport heartbeat received no response for 30s");
                        Err(health.error_with_transport("run", now + HEARTBEAT_LEASE, Some(&failure)).unwrap())
                    } else { Ok(()) }
                }
            },
            |_, _| {
                let mut native = tests::snapshot();
                native.pending_tool_ids = vec!["write-once".into()];
                native.validate(Some(&native.native_id)).unwrap();
                let notice = native.resume_context(&journal).unwrap().unwrap();
                assert!(notice.contains("Do not retry or repeat"));
                assert!(notice.contains(if completed { "\"status\":\"completed\"" } else { "\"status\":\"failed\"" }));
                synchronizations.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
            receive,
            &[Duration::ZERO; 3],
        ).await.unwrap();
        assert_eq!(result, Some(()));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(synchronizations.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_to_string(root.path().join("effect")).unwrap(), "updated once");
    }
}

#[tokio::test]
async fn transport_retry_reuses_completed_tool_receipt_instead_of_reexecuting_effect() {
    use super::super::{native_journal::Journal, turn_retry};
    use std::sync::atomic::{AtomicU32, Ordering};
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::new(root.path().into());
    let executions = AtomicU32::new(0);
    let synchronizations = AtomicU32::new(0);
    let (_send, receive) = tokio::sync::watch::channel(false);
    let call = json!({"callId":"write-once","tool":"Write","input":{"file_path":"asset","content":"updated"}});
    let result = turn_retry::run(
        |attempt| {
            if journal.begin(&call).unwrap().is_none() {
                executions.fetch_add(1, Ordering::SeqCst);
                std::fs::write(root.path().join("effect"), "updated once").unwrap();
                journal
                    .complete(
                        "write-once",
                        &json!({"callId":"write-once","result":{"type":"create"}}),
                    )
                    .unwrap();
            }
            async move {
                if attempt == 1 {
                    Err(turn_retry::disconnected("lost tool result ACK"))
                } else {
                    Ok(())
                }
            }
        },
        |_, _| {
            synchronizations.fetch_add(1, Ordering::SeqCst);
            let mut native = tests::snapshot();
            native.pending_tool_ids = vec!["write-once".into()];
            let notice = native.resume_context(&journal).unwrap().unwrap();
            assert!(notice.contains("completed") && notice.contains("Do not retry"));
            async { Ok(()) }
        },
        receive,
        &[Duration::ZERO],
    )
    .await
    .unwrap();
    assert_eq!(result, Some(()));
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(synchronizations.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("effect")).unwrap(),
        "updated once"
    );
}
