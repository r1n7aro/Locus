use super::*;
use super::super::native_journal::Journal;
use serde_json::json;

pub(super) fn snapshot() -> NativeSnapshot {
    serde_json::from_value(json!({
        "version": 1, "nativeId": uuid::Uuid::new_v4(), "digest": "a".repeat(64),
        "messages": [
            {"key":"user", "role":"user", "uuids":[uuid::Uuid::new_v4()], "content":"inspect the asset"},
            {"key":"assistant", "role":"assistant", "uuids":[uuid::Uuid::new_v4()],
             "content":[{"type":"text", "text":"Checking the asset."}]}
        ],
        "todos":[], "incompleteTail":false, "pendingToolIds":["unflushed-read"]
    })).unwrap()
}

fn record(journal: &Journal, id: &str, tool: &str) {
    journal.begin(&json!({"callId":id, "tool":tool,
        "input":{"path":"Assets/Example.asset", "readonly":true}})).unwrap();
}

#[test]
fn recovery_validates_hidden_user_identities_and_accepts_legacy_snapshots() {
    let mut native=snapshot();
    assert!(native.hidden_user_uuids.is_empty());
    assert!(native.validate(None).is_ok());
    native.hidden_user_uuids=vec![uuid::Uuid::new_v4().to_string()];
    assert!(native.validate(None).is_ok());
    native.hidden_user_uuids.push(native.hidden_user_uuids[0].clone());
    assert!(native.validate(None).is_err());
    native.hidden_user_uuids=vec![native.messages[0].uuids[0].clone()];
    assert!(native.validate(None).is_err());
    native.hidden_user_uuids=vec!["invalid".into()];
    assert!(native.validate(None).is_err());
}

#[test]
fn recovery_removes_old_command_bubbles_including_archived_rows_but_keeps_pending_input() {
    let root=tempfile::tempdir().unwrap();
    let store=SessionStore::new(root.path()).unwrap();
    let id=store.create_session("command display recovery",None,None,"chat",None).unwrap();
    let remote:ssh::SshConfig=serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let mut legacy=snapshot();
    legacy.pending_tool_ids.clear();
    let output="<local-command-stdout>Compacted</local-command-stdout>";
    let mut echo=legacy.messages[0].clone();
    echo.key="old-command".into();echo.uuids=vec![uuid::Uuid::new_v4().to_string()];echo.content=json!(output);
    let mut summary=echo.clone();
    summary.key="summary".into();summary.uuids=vec![uuid::Uuid::new_v4().to_string()];
    summary.content=json!("native summary");summary.compact_summary=true;
    let mut recent=echo.clone();
    recent.key="recent-command".into();recent.uuids=vec![uuid::Uuid::new_v4().to_string()];
    legacy.messages.extend([echo.clone(),summary,recent.clone()]);
    store.reconcile_claude_native(&id,&remote,&legacy,&[],None).unwrap();
    let before=store.get_messages(&id).unwrap();
    let echo_id=before[2].id.clone();
    assert!(!store.get_messages_for_prompt(&id).unwrap().iter().any(|m|m.id==echo_id));
    // Identical text entered by a person is not a native echo and must survive.
    let pending=store.add_message(&id,MessageRole::User,output).unwrap();
    let mut clean=legacy.clone();
    clean.messages.retain(|m|m.key!=echo.key && m.key!=recent.key);
    clean.hidden_user_uuids=vec![echo.uuids[0].clone(),recent.uuids[0].clone()];
    clean.digest="b".repeat(64);
    assert!(store.reconcile_claude_native(&id,&remote,&clean,&store.get_messages(&id).unwrap(),Some(&pending)).unwrap());
    let after=store.get_messages(&id).unwrap();
    assert_eq!(after.len(),4);
    assert_eq!(after[0].id,before[0].id);
    assert_eq!(after[1].id,before[1].id);
    assert_eq!(after[2].id,before[3].id);
    // Full-history reads redact the summary into a display marker; inspect
    // the prompt projection to verify that its actual content was preserved.
    assert!(store.get_messages_for_prompt(&id).unwrap().iter().any(|m|
        m.id==after[2].id && m.content.contains("native summary")));
    assert_eq!(after[3].id,pending);
    assert_eq!(after[3].content,output);
    assert!(!after.iter().any(|m|m.id==echo_id || m.id==before[4].id));
    assert!(!store.reconcile_claude_native(&id,&remote,&clean,&after,Some(&pending)).unwrap());
    let backups=root.path().join("claude-recovery").join(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,id.as_bytes()).to_string());
    assert!(std::fs::read_dir(backups).unwrap().any(|entry|std::fs::read_to_string(entry.unwrap().path()).unwrap().contains(&echo_id)));
}

#[test]
fn authoritative_recovery_discards_local_drift_and_keeps_only_current_input() {
    let root=tempfile::tempdir().unwrap(); let store=SessionStore::new(root.path()).unwrap();
    let id=store.create_session("remote recovery",None,None,"chat",None).unwrap();
    let remote:ssh::SshConfig=serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let mut native=snapshot();
    native.messages[1].content.as_array_mut().unwrap().push(json!({"type":"tool_use","id":"unflushed-read","name":"mcp__locus__unity_test_run","input":{"mode":"edit"}}));
    store.bind_claude_native_session(&id,&remote,&native.native_id).unwrap();
    store.add_message(&id,MessageRole::Assistant,"unverified local partial answer").unwrap();
    store.add_message(&id,MessageRole::User,"old unsent input must not replay").unwrap();
    let current=store.add_message(&id,MessageRole::User,"continue with fewer tests").unwrap();
    let before=store.get_messages(&id).unwrap();
    assert!(store.rebuild_claude_native(&id,&remote,&native,&before,Some(&current)).unwrap());
    let after=store.get_messages(&id).unwrap();
    assert_eq!(after.len(),3);
    assert_eq!(after[0].content,"inspect the asset");
    assert_eq!(after[1].tool_calls.as_ref().unwrap()[0].outcome,Some(crate::commands::ToolCallOutcome::Error));
    assert_eq!(after[2].id,current);
    assert!(!serde_json::to_string(&after).unwrap().contains("old unsent input"));
    assert!(store.rebuild_claude_native(&id,&remote,&native,&before,Some(&current)).is_err(),"stale concurrent replacement must fail");
    assert_eq!(serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(),serde_json::to_value(&after).unwrap());
    store.rebuild_claude_native(&id,&remote,&native,&after,Some(&current)).unwrap();
    assert_eq!(serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(),serde_json::to_value(&after).unwrap(),"repeated recovery is idempotent");
    let mut wrong=native.clone();wrong.native_id=uuid::Uuid::new_v4().to_string();
    assert!(store.rebuild_claude_native(&id,&remote,&wrong,&after,None).is_err());
    let backups=root.path().join("claude-recovery").join(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,id.as_bytes()).to_string());
    assert!(std::fs::read_dir(backups).unwrap().any(|e|std::fs::read_to_string(e.unwrap().path()).unwrap().contains("old unsent input")));
    drop(store);
    let store=SessionStore::new(root.path()).unwrap();
    let output=root.path().join("recovered.yaml");
    crate::session::context_export::export_session_context_yaml(&store,&id,"F:/test",None,None,&output).unwrap();
    let text=std::fs::read_to_string(output).unwrap();
    assert!(text.contains(&native.native_id) && text.contains("continue with fewer tests"));
}

#[test]
fn recovery_marks_unknown_calls_failed_without_retrying_any_tool() {
    for tool in ["Read","Write","Edit","Bash","TaskStop","mcp__locus__unity_test_run","mcp__locus__unity_execute","custom"] {
        let root=tempfile::tempdir().unwrap(); let journal=Journal::new(root.path().into());
        let native=snapshot(); record(&journal,"unflushed-read",tool);
        let context=native.resume_context(&journal).unwrap().unwrap();
        assert!(context.contains("\"status\":\"failed\"") && context.contains("Do not retry"));
        let call=json!({"callId":"unflushed-read","tool":tool,"input":{"path":"Assets/Example.asset","readonly":true}});
        assert_eq!(journal.begin(&call).unwrap().unwrap()["deny"],failed_tool_message());
        assert_eq!(native.resume_context(&journal).unwrap().unwrap(),context);
        assert!(native.can_compact().is_ok());
        assert!(native.can_continue().is_err(), "archive transfer still requires native confirmation");
    }
}

#[test]
fn recovery_handles_missing_and_torn_journals_and_many_calls() {
    let root=tempfile::tempdir().unwrap(); let journal=Journal::new(root.path().join("calls"));
    let mut native=snapshot();
    native.pending_tool_ids=(0..24).map(|i|format!("call-{i}")).collect();
    for id in native.pending_tool_ids.iter().take(2) { record(&journal,id,"Write"); }
    for entry in std::fs::read_dir(root.path().join("calls")).unwrap() {
        std::fs::write(entry.unwrap().path(),"{").unwrap();
    }
    let before=serde_json::to_value(&native).unwrap();
    let context=native.resume_context(&journal).unwrap().unwrap();
    for id in &native.pending_tool_ids {
        assert!(context.contains(id));
        assert_eq!(journal.begin(&json!({"callId":id,"tool":"Write","input":{}})).unwrap().unwrap()["deny"],failed_tool_message());
    }
    assert_eq!(serde_json::to_value(&native).unwrap(),before,"never fabricate native history");
}

#[test]
fn recovery_preserves_known_results_and_deduplicates_notices() {
    let root=tempfile::tempdir().unwrap(); let journal=Journal::new(root.path().into());
    let mut native=snapshot(); native.pending_tool_ids=vec!["complete".into(),"interrupted".into(),"unknown".into()];
    record(&journal,"complete","Edit");
    let result=json!({"callId":"complete","result":"saved"});
    journal.complete("complete",&result).unwrap();
    record(&journal,"interrupted","mcp__locus__unity_execute");
    journal.complete("interrupted",&json!({"callId":"interrupted","deny":super::super::native_journal::INTERRUPTED_DISPATCH_MESSAGE})).unwrap();
    let context=native.resume_context(&journal).unwrap().unwrap();
    for status in ["completed","interrupted","failed"] { assert!(context.contains(&format!("\"status\":\"{status}\""))); }
    let mut user=native.messages[0].clone(); user.key="recovered-input".into();user.content=json!(context);
    native.messages.push(user);
    assert!(native.resume_context(&journal).unwrap().is_none());
    native.pending_tool_ids.push("another".into());
    let notice=native.resume_context(&journal).unwrap().unwrap();
    assert!(notice.contains("another") && !notice.contains("\"callId\":\"complete\""));
    let call=json!({"callId":"complete","tool":"Edit","input":{"path":"Assets/Example.asset","readonly":true}});
    assert_eq!(journal.begin(&call).unwrap(),Some(result));
}

#[test]
fn recovery_allows_native_pending_calls_to_be_closed_as_errors_by_cli() {
    let root=tempfile::tempdir().unwrap(); let journal=Journal::new(root.path().into());
    let mut native=snapshot();
    native.messages[1].content.as_array_mut().unwrap().push(json!({"type":"tool_use","id":"unflushed-read","name":"Write","input":{}}));
    assert!(native.can_resume(&journal).is_ok());
    native.incomplete_tail=true;
    assert!(native.can_resume(&journal).unwrap_err().contains("incomplete final record"));
}

#[test]
#[ignore = "requires a captured native archive and copied local tool journal"]
fn dispatch_recovery_from_captured_archive_without_compaction() {
    let root=PathBuf::from(std::env::var("LOCUS_DISPATCH_RECOVERY_FIXTURE").unwrap());
    let info:super::super::archive::ArchiveInfo=serde_json::from_slice(&std::fs::read(root.join("archive-info.json")).unwrap()).unwrap();
    let files:std::collections::BTreeMap<_,_>=info.manifest.iter().map(|(name,cursor)|{
        assert!(super::super::archive::valid_name(name));
        let bytes=std::fs::read(root.join("archive").join(name)).unwrap();
        assert_eq!(super::super::archive::digest(&bytes),cursor.sha256);
        (name.clone(),bytes)
    }).collect();
    let native=super::super::archive::project(&info,&files).unwrap();
    let isolated=tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(root.join("profile/calls")).unwrap() {
        let entry=entry.unwrap(); if entry.file_type().unwrap().is_file() {std::fs::copy(entry.path(),isolated.path().join(entry.file_name())).unwrap();}
    }
    let journal=Journal::new(isolated.path().into());
    let before=serde_json::to_value(&native).unwrap();
    let context=native.resume_context(&journal).unwrap().unwrap();
    for id in &native.pending_tool_ids { assert!(context.contains(id)); }
    assert!(context.contains("\"status\":\"failed\"") || context.contains("\"status\":\"interrupted\""));
    assert_eq!(serde_json::to_value(&native).unwrap(),before);
}

#[test]
fn crash_recovery_reconciles_committed_history_preserves_input_and_exports_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("crash", None, None, "chat", None).unwrap();
    let remote: ssh::SshConfig = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
    let native = snapshot();
    for row in &native.messages {
        let local = store.add_message(&id, row.role.clone(), &text_content(&row.content)).unwrap();
        store.set_message_response_request_metadata(&id, &local, &json!({
            "backend":"claude_code_cli", "claude_connection":remote,
            "claude_checkpoint":{"version":1,"uuid":row.uuids[0],"sessionId":native.native_id}
        })).unwrap();
    }
    let retained = store.add_message(&id, MessageRole::Assistant, "Confirmed text absent from active snapshot").unwrap();
    store.set_message_response_request_metadata(&id, &retained, &json!({
        "backend":"claude_code_cli", "claude_connection":remote,
        "claude_checkpoint":{"version":1,"uuid":uuid::Uuid::new_v4(),"sessionId":native.native_id}
    })).unwrap();
    let unconfirmed = store.add_message(&id, MessageRole::Assistant, "Unflushed streamed text").unwrap();
    store.set_message_response_request_metadata(&id, &unconfirmed, &json!({
        "backend":"claude_code_cli", "claude_connection":remote
    })).unwrap();
    let pending = store.add_message(&id, MessageRole::User, "continue with my new requirement").unwrap();
    let before = store.get_messages(&id).unwrap();
    assert!(store.reconcile_claude_native(&id, &remote, &native, &before, Some(&pending)).unwrap());
    let after = store.get_messages(&id).unwrap();
    assert_eq!(after.len(), 4);
    assert!(after.iter().any(|m| m.id == retained));
    assert!(!after.iter().any(|m| m.id == unconfirmed));
    let prompt = store.get_messages_for_prompt(&id).unwrap();
    assert_eq!(prompt.len(), 3);
    assert!(!prompt.iter().any(|m| m.id == retained));
    assert_eq!(after.last().unwrap().id, pending);
    assert_eq!(store.latest_cli_session_id(&id, "claude_code").unwrap().as_deref(), Some(native.native_id.as_str()));
    assert!(!store.reconcile_claude_native(&id, &remote, &native, &after, Some(&pending)).unwrap());
    let backups = root.path().join("claude-recovery").join(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, id.as_bytes()).to_string());
    let backup = std::fs::read_dir(backups).unwrap().next().unwrap().unwrap().path();
    assert!(std::fs::read_to_string(backup).unwrap().contains("Unflushed streamed text"));
    drop(store);
    let store = SessionStore::new(root.path()).unwrap();
    let output = root.path().join("recovered.yaml");
    crate::session::context_export::export_session_context_yaml(&store, &id, "F:/test", None, None, &output).unwrap();
    let text = std::fs::read_to_string(output).unwrap();
    assert!(text.contains("Checking the asset."));
    assert!(text.contains("continue with my new requirement"));
    assert!(text.contains("Confirmed text absent from active snapshot"));
}

#[tokio::test]
async fn crash_recovery_queues_behind_prewarm_and_concurrent_sync() {
    let id = uuid::Uuid::new_v4().to_string();
    let prewarm = tui::history_gate(&id).write_owned().await;
    let queued = synchronization_guard(&id);
    tokio::pin!(queued);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut queued).await.is_err());
    drop(prewarm);
    let first = queued.await.unwrap();
    let second = synchronization_guard(&id);
    tokio::pin!(second);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut second).await.is_err());
    drop(first);
    assert!(second.await.is_ok());
}

#[test]
#[ignore = "requires an explicitly selected COPY of a session database and read-only captured native history"]
fn crash_recovery_from_copied_database() {
    let root = std::env::var("LOCUS_RECOVERY_TEST_DATABASE_DIR").unwrap();
    let fixture = std::env::var("LOCUS_RECOVERY_TEST_SNAPSHOT").unwrap();
    let profile = std::env::var("LOCUS_RECOVERY_TEST_PROFILE").unwrap();
    let id = std::env::var("LOCUS_RECOVERY_TEST_SESSION").unwrap();
    let native: NativeSnapshot = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
    native.validate(None).unwrap();
    let journal = Journal::new(PathBuf::from(profile).join("calls"));
    native.can_resume(&journal).unwrap();
    let store = SessionStore::new(Path::new(&root)).unwrap();
    let remote = recorded_remote(&store, &id).unwrap().unwrap();
    let before = store.get_messages(&id).unwrap();
    let pending = before.last().filter(|m| m.role == MessageRole::User).map(|m| m.id.clone());
    store.reconcile_claude_native(&id, &remote, &native, &before, pending.as_deref()).unwrap();
    let after = store.get_messages(&id).unwrap();
    assert!(!store.reconcile_claude_native(&id, &remote, &native, &after, pending.as_deref()).unwrap());
    assert_eq!(store.latest_cli_session_id(&id, "claude_code").unwrap().as_deref(), Some(native.native_id.as_str()));
    let output = Path::new(&root).join("recovered-context.yaml");
    crate::session::context_export::export_session_context_yaml(&store, &id, "F:/test", None, None, &output).unwrap();
    eprintln!("Recovered {} local rows to {} rows; native identity {}; export {}", before.len(), after.len(), native.native_id, output.display());
}
