use super::*;
use crate::llm::claude_code_cli::archive::{self, ArchiveInfo, Cursor};
use std::collections::BTreeMap;

#[test]
#[ignore = "requires LOCUS_DISPATCH_RECOVERY_FIXTURE with a copied evidence.db"]
fn authoritative_recovery_of_captured_session_rebuilds_and_exports() {
    let source=std::path::PathBuf::from(std::env::var("LOCUS_DISPATCH_RECOVERY_FIXTURE").unwrap());
    let root=tempfile::tempdir().unwrap();let store=SessionStore::new(root.path()).unwrap();
    {
        let conn=store.conn.lock().unwrap();
        conn.execute("ATTACH DATABASE ?1 AS diagnostic",[format!("file:{}?mode=ro",source.join("evidence.db").display().to_string().replace('\\',"/"))]).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        for table in ["response_request_payloads","sessions","messages","token_usage","todos","session_runs","session_events","claude_native_archives","claude_native_archive_files"] {
            let mut stmt=conn.prepare(&format!("PRAGMA diagnostic.table_info({table})")).unwrap();
            let columns=stmt.query_map([],|r|r.get::<_,String>(1)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            if columns.is_empty() {continue;}
            let columns=columns.iter().map(|c|format!("\"{c}\"")).collect::<Vec<_>>().join(",");
            conn.execute_batch(&format!("INSERT INTO main.{table} (rowid,{columns}) SELECT rowid,{columns} FROM diagnostic.{table};")).unwrap();
        }
        conn.execute_batch("DETACH DATABASE diagnostic; PRAGMA foreign_keys=ON;").unwrap();
    }
    let id:String=store.conn.lock().unwrap().query_row("SELECT id FROM sessions",[],|r|r.get(0)).unwrap();
    let remote=recovery::recorded_remote(&store,&id).unwrap().unwrap();
    let info:ArchiveInfo=serde_json::from_slice(&std::fs::read(source.join("archive-info.json")).unwrap()).unwrap();
    let files=info.manifest.keys().map(|name|(name.clone(),std::fs::read(source.join("archive").join(name)).unwrap())).collect();
    let native=archive::project(&info,&files).unwrap();
    store.bind_claude_native_session(&id,&remote,&native.native_id).unwrap();
    let current=store.add_message(&id,MessageRole::User,"继续当前工作，不重复中断工具").unwrap();
    let before=store.get_messages(&id).unwrap();
    store.rebuild_claude_native(&id,&remote,&native,&before,Some(&current)).unwrap();
    let after=store.get_messages(&id).unwrap();
    assert_eq!(after.len(),native.messages.len()+1);
    assert_eq!(after.last().unwrap().id,current);
    assert_eq!(recovery::native_session_id(&store,&id).unwrap(),Some(native.native_id.clone()));
    store.rebuild_claude_native(&id,&remote,&native,&after,Some(&current)).unwrap();
    assert_eq!(serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(),serde_json::to_value(&after).unwrap());
    let output=root.path().join("recovered.yaml");
    crate::session::context_export::export_session_context_yaml(&store,&id,"F:/test",None,None,&output).unwrap();
    assert!(std::fs::read_to_string(output).unwrap().contains(&native.native_id));
    eprintln!("Authoritative recovery: {} local rows -> {} remote rows + current input; export verified",before.len(),native.messages.len());
}

fn fixture() -> (NativeConnection, NativeSnapshot) {
    let remote = serde_json::from_value(json!({"target":"test@host","port":22})).unwrap();
    let uuid = |n| Uuid::from_u128(n).to_string();
    let row = |n, parent: Option<u128>, role: &str, content: Value| json!({"uuid":uuid(n),"parentUuid":parent.map(uuid),
        "type":role,"timestamp":"2026-10-16T00:00:00Z","message":{"content":content}});
    let mut thinking = row(2, Some(1), "assistant", json!([{"type":"thinking","thinking":"reason","signature":"signed"}]));
    let mut first = row(3, Some(2), "assistant", json!([{"type":"text","text":"reading"},
        {"type":"tool_use","id":"read-a","name":"read","input":{"filePath":"a"}}]));
    let mut second = row(5, Some(3), "assistant", json!([{"type":"tool_use","id":"read-b","name":"read","input":{"filePath":"b"}}]));
    for part in [&mut thinking, &mut first, &mut second] {
        part["message"]["id"] = json!("api-response");
        part["message"]["model"] = json!("opus");
        part["message"]["usage"] = json!({"input_tokens":11,"output_tokens":7});
    }
    let rows = [row(1, None, "user", json!("question")), thinking, first,
        row(4, Some(3), "user", json!([{"type":"tool_result","tool_use_id":"read-a","content":"one"}])), second,
        row(6, Some(5), "user", json!([{"type":"tool_result","tool_use_id":"read-b","content":"two"}]))];
    let raw = rows.iter().map(|row| format!("{row}\n")).collect::<String>().into_bytes();
    let info = ArchiveInfo { version:1,native_id:uuid(100),source_root:"/test".into(),updated_at:0,
        transferred_bytes:raw.len() as u64,incomplete_tail:false,
        manifest:BTreeMap::from([("transcript.jsonl".into(),Cursor { size:raw.len() as u64,sha256:archive::digest(&raw) })]) };
    let snapshot = archive::project(&info,&BTreeMap::from([("transcript.jsonl".into(),raw)])).unwrap();
    (remote, snapshot)
}

fn seed(store: &SessionStore, id: &str, remote: &NativeConnection, snapshot: &NativeSnapshot) -> Vec<String> {
    let user = store.add_message(id, MessageRole::User, "question").unwrap();
    let assistant = &snapshot.messages[1];
    let mut ids = vec![user.clone()];
    for (index, uuid) in [Uuid::from_u128(3),Uuid::from_u128(5)].iter().enumerate() {
        let part = assistant.parts.iter().find(|part| part.uuid == uuid.to_string()).unwrap();
        let parsed = recovery::assistant_content(&Value::Array(part.content.clone()));
        let message = store.add_assistant_with_tool_calls_and_render_parts(id,&parsed.text,&parsed.tool_calls,None,None,None,None,None,None,None,&[]).unwrap();
        store.set_message_response_request_metadata(id,&message,&json!({"backend":"claude_code_cli","claude_connection":remote,
            "claude_checkpoint":{"version":1,"sessionId":snapshot.native_id,"uuid":uuid},
            "claude_usage":{"input_tokens":11,"output_tokens":7},"model":"claude_code/opus"})).unwrap();
        ids.push(message);
        let native = &snapshot.messages[index + 2];
        let tool = store.add_message(id, MessageRole::Tool, &recovery::text_content(&native.content)).unwrap();
        store.conn.lock().unwrap().execute("UPDATE messages SET tool_call_id=?1 WHERE id=?2",params![native.tool_call_id,tool]).unwrap();
        store.set_message_response_request_metadata(id,&tool,&json!({"backend":"claude_code_cli","claude_connection":remote,
            "claude_checkpoint":{"version":1,"sessionId":snapshot.native_id,"uuid":native.uuids[0]}})).unwrap();
        ids.push(tool);
    }
    store.set_message_response_request_metadata(id,&user,&json!({"backend":"claude_code_cli","claude_connection":remote,
        "claude_checkpoint":{"version":1,"sessionId":snapshot.native_id,"uuid":snapshot.messages[0].uuids[0]}})).unwrap();
    ids
}

#[test]
fn claude_recovery_preserves_split_round_ids_results_usage_and_exports_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("split",None,None,"chat",None).unwrap();
    let (remote, snapshot) = fixture();
    let ids = seed(&store,&id,&remote,&snapshot);
    let pending = store.add_message(&id,MessageRole::User,"follow-up").unwrap();
    let before = store.get_messages(&id).unwrap();
    assert!(store.reconcile_claude_native(&id,&remote,&snapshot,&before,Some(&pending)).unwrap());
    let after = store.get_messages(&id).unwrap();
    assert_eq!(after.len(),before.len());
    for old in &before { assert!(after.iter().any(|m|m.id==old.id)); }
    assert_eq!(after.last().unwrap().id,pending);
    let first = after.iter().find(|m|m.id==ids[1]).unwrap();
    let second = after.iter().find(|m|m.id==ids[3]).unwrap();
    assert_eq!(first.thinking_content.as_deref(),Some("reason"));
    assert_eq!(first.thinking_signature.as_deref(),Some("signed"));
    assert_eq!(first.tool_calls.as_ref().unwrap()[0].recorded_output.as_deref(),Some("one"));
    assert_eq!(second.tool_calls.as_ref().unwrap()[0].recorded_output.as_deref(),Some("two"));
    assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens,7);
    assert!(!store.reconcile_claude_native(&id,&remote,&snapshot,&after,Some(&pending)).unwrap());
    // A different archive digest must reapply the migration without changing IDs.
    let mut changed = snapshot.clone(); changed.digest = "b".repeat(64);
    assert!(store.reconcile_claude_native(&id,&remote,&changed,&after,Some(&pending)).unwrap());
    drop(store);
    let store = SessionStore::new(root.path()).unwrap();
    assert!(!store.reconcile_claude_native(&id,&remote,&changed,&store.get_messages(&id).unwrap(),Some(&pending)).unwrap());
    let path = root.path().join("export.yaml");
    crate::session::context_export::export_session_context_yaml(&store,&id,"F:/test",None,None,&path).unwrap();
    let yaml:serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(yaml["sessions"][0]["messages"].as_sequence().unwrap().len(),before.len());
    assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens,7);
}

#[test]
fn claude_recovery_migrates_legacy_parts_with_unique_tool_identities() {
    let root = tempfile::tempdir().unwrap(); let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("legacy parts",None,None,"chat",None).unwrap();
    let (remote,snapshot) = fixture(); let ids = seed(&store,&id,&remote,&snapshot);
    // The other part already has a checkpoint; a strong match must not hide
    // this legacy sibling and cause it to be silently deleted.
    for id_to_update in [&ids[3]] {
        store.set_message_response_request_metadata(&id,id_to_update,&json!({"backend":"claude_code_cli","claude_connection":remote})).unwrap();
    }
    let before = store.get_messages(&id).unwrap();
    store.reconcile_claude_native(&id,&remote,&snapshot,&before,None).unwrap();
    let after = store.get_messages(&id).unwrap();
    assert_eq!(after.len(),before.len());
    assert!(before.iter().all(|old| after.iter().any(|new|new.id==old.id)));
    assert!(!store.reconcile_claude_native(&id,&remote,&snapshot,&after,None).unwrap());
}

#[test]
fn claude_recovery_rejects_duplicate_foreign_and_inconsistent_part_anchors_atomically() {
    for invalid in ["duplicate","foreign","wrong-tool","missing-parts"] {
        let root = tempfile::tempdir().unwrap(); let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session(invalid,None,None,"chat",None).unwrap();
        let (remote,mut snapshot) = fixture(); let ids = seed(&store,&id,&remote,&snapshot);
        let mut metadata = store.get_response_request_metadata(&id).unwrap()[&ids[3]].clone();
        match invalid {
            "duplicate" => metadata["claude_checkpoint"]["uuid"] = json!(Uuid::from_u128(3)),
            "foreign" => metadata["claude_checkpoint"]["sessionId"] = json!(Uuid::new_v4()),
            "wrong-tool" => { store.conn.lock().unwrap().execute("UPDATE messages SET tool_calls=?1 WHERE id=?2",
                params![r#"[{"id":"wrong","name":"read","arguments":"{}"}]"#,ids[3]]).unwrap(); },
            _ => snapshot.messages[1].parts.clear(),
        }
        store.set_message_response_request_metadata(&id,&ids[3],&metadata).unwrap();
        let before = store.get_messages(&id).unwrap();
        assert!(store.reconcile_claude_native(&id,&remote,&snapshot,&before,None).is_err(),"{invalid}");
        assert_eq!(serde_json::to_value(store.get_messages(&id).unwrap()).unwrap(),serde_json::to_value(before).unwrap());
    }
}

#[test]
fn claude_recovery_preserves_only_a_durably_recorded_interrupted_user_turn() {
    let root = tempfile::tempdir().unwrap(); let store = SessionStore::new(root.path()).unwrap();
    let id = store.create_session("interrupted input",None,None,"chat",None).unwrap();
    let user = store.add_message(&id,MessageRole::User,"keep this input").unwrap();
    let messages = store.get_messages(&id).unwrap();
    assert!(store.claude_recovery_pending_message(&id,&messages,&HashMap::new()).unwrap().is_none());
    {
        let conn = store.conn.lock().unwrap();
        conn.execute("INSERT INTO session_runs(run_id,session_id,status,started_at,updated_at) VALUES('failed',?1,'error',1,1)",[&id]).unwrap();
        conn.execute("INSERT INTO session_events(session_id,run_id,seq,event_type,payload_json,created_at) VALUES(?1,'failed',1,'userMessage',?2,1)",
            params![id,json!({"message":{"id":user}}).to_string()]).unwrap();
    }
    assert_eq!(store.claude_recovery_pending_message(&id,&messages,&HashMap::new()).unwrap(),Some(user.clone()));
    store.try_start_run(&id,"retry").unwrap();
    assert_eq!(store.claude_recovery_pending_message(&id,&messages,&HashMap::new()).unwrap(),Some(user.clone()));
    let metadata = HashMap::from([(user,json!({"claude_checkpoint":{"uuid":Uuid::new_v4()}}))]);
    assert!(store.claude_recovery_pending_message(&id,&messages,&metadata).unwrap().is_none());
    store.conn.lock().unwrap().execute("UPDATE session_runs SET status='done' WHERE session_id=?1",[&id]).unwrap();
    assert!(store.claude_recovery_pending_message(&id,&messages,&HashMap::new()).unwrap().is_none());
}

#[test]
fn claude_recovery_keeps_previous_grouped_records_and_counts_archived_parts_once() {
    let root = tempfile::tempdir().unwrap(); let store = SessionStore::new(root.path()).unwrap();
    let (remote,snapshot) = fixture();
    let grouped = store.create_session("grouped",None,None,"chat",None).unwrap();
    store.reconcile_claude_native(&grouped,&remote,&snapshot,&[],None).unwrap();
    let before = store.get_messages(&grouped).unwrap();
    let mut changed = snapshot.clone(); changed.digest = "c".repeat(64);
    assert!(store.reconcile_claude_native(&grouped,&remote,&changed,&before,None).unwrap());
    assert_eq!(store.get_messages(&grouped).unwrap().len(),snapshot.messages.len());
    assert_eq!(store.get_messages(&grouped).unwrap()[1].id,before[1].id);

    let id = store.create_session("archived parts",None,None,"chat",None).unwrap();
    seed(&store,&id,&remote,&snapshot);
    store.reconcile_claude_native(&id,&remote,&snapshot,&store.get_messages(&id).unwrap(),None).unwrap();
    let mut summary = snapshot.messages[0].clone();
    summary.key = "summary".into(); summary.uuids = vec![Uuid::new_v4().to_string()];
    summary.content = json!("compacted"); summary.compact_summary = true;
    let mut compacted = snapshot.clone(); compacted.messages.push(summary.clone()); compacted.digest = "d".repeat(64);
    store.reconcile_claude_native(&id,&remote,&compacted,&store.get_messages(&id).unwrap(),None).unwrap();
    compacted.messages = vec![summary]; compacted.digest = "e".repeat(64);
    store.reconcile_claude_native(&id,&remote,&compacted,&store.get_messages(&id).unwrap(),None).unwrap();
    assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens,7);
    assert_eq!(store.get_messages_for_prompt(&id).unwrap().len(),1);
}

#[test]
fn claude_recovery_preserves_multiple_failed_inputs_and_matches_a_lost_native_receipt() {
    let root=tempfile::tempdir().unwrap(); let store=SessionStore::new(root.path()).unwrap();
    let id=store.create_session("multiple pending",None,None,"chat",None).unwrap();
    let (remote,mut snapshot)=fixture(); seed(&store,&id,&remote,&snapshot);
    store.reconcile_claude_native(&id,&remote,&snapshot,&store.get_messages(&id).unwrap(),None).unwrap();
    let mut ids=Vec::new(); let tokens=[Uuid::new_v4().to_string(),Uuid::new_v4().to_string()];
    for i in 0..2 {
        let run=format!("failed-{i}"); store.try_start_run(&id,&run).unwrap();
        // Equal text is intentionally ambiguous without delivery tokens.
        let user=store.add_message(&id,MessageRole::User,"same follow-up").unwrap();
        store.append_session_event(&id,&run,"userMessage",&json!({"message":{"id":user}}).to_string()).unwrap();
        store.update_run_status(&run,"error",Some("not submitted")).unwrap(); ids.push(user);
    }
    let before=store.get_messages(&id).unwrap();
    let original_ids=before.iter().map(|m|m.id.clone()).collect::<Vec<_>>();
    assert_eq!(store.claude_recovery_pending_messages(&id,&before,&store.get_response_request_metadata(&id).unwrap(),None).unwrap(),ids);
    store.reconcile_claude_native(&id,&remote,&snapshot,&before,None).unwrap();
    assert_eq!(store.get_messages(&id).unwrap().iter().map(|m|m.id.clone()).collect::<Vec<_>>(),original_ids);
    for (i,user) in ids.iter().enumerate() {
        store.set_message_response_request_metadata(&id,user,&json!({"backend":"claude_code_cli","claude_connection":remote,
            "claude_pending_input":{"version":1,"token":tokens[i],"carrier":if i==0 {Some(&tokens[1])} else {None}}})).unwrap();
    }
    let mut native=snapshot.messages[0].clone();
    native.uuids=vec![Uuid::new_v4().to_string()]; native.key=native.uuids[0].clone();
    native.content=json!([{"type":"text","text":recovery::pending_input_text(&tokens[0],"same follow-up")}]);
    snapshot.messages.push(native.clone()); snapshot.digest="f".repeat(64);
    store.reconcile_claude_native(&id,&remote,&snapshot,&store.get_messages(&id).unwrap(),None).unwrap();
    let after=store.get_messages(&id).unwrap();
    assert_eq!(after.iter().map(|m|m.id.clone()).collect::<Vec<_>>(),original_ids);
    let metadata=store.get_response_request_metadata(&id).unwrap();
    assert_eq!(metadata[&ids[0]]["claude_checkpoint"]["uuid"],native.uuids[0]);
    assert!(metadata[&ids[1]]["claude_checkpoint"].is_null());
    assert_eq!(store.claude_recovery_pending_messages(&id,&after,&metadata,None).unwrap(),ids);
    assert!(!store.reconcile_claude_native(&id,&remote,&snapshot,&after,None).unwrap());
    // Reopen/export keeps both local user rows and their native proof.
    drop(store); let store=SessionStore::new(root.path()).unwrap();
    let export=root.path().join("pending.yaml");
    crate::session::context_export::export_session_context_yaml(&store,&id,"F:/test",None,None,&export).unwrap();
    assert_eq!(store.get_messages(&id).unwrap().len(),original_ids.len());
    assert!(std::fs::read_to_string(export).unwrap().contains("same follow-up"));
}

#[test]
#[ignore = "Uses an explicitly supplied, read-only diagnostic session snapshot"]
fn claude_recovery_diagnostic_snapshot_preserves_every_message_and_exports() {
    let source = std::env::var("LOCUS_RECOVERY_TEST_SNAPSHOT").expect("diagnostic snapshot path");
    let root = tempfile::tempdir().unwrap(); let store = SessionStore::new(root.path()).unwrap();
    {
        let conn = store.conn.lock().unwrap();
        conn.execute("ATTACH DATABASE ?1 AS diagnostic",[format!("file:{}?mode=ro",source.replace('\\',"/"))]).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        for table in ["response_request_payloads","sessions","messages","token_usage","todos","session_runs","session_events","claude_native_archives","claude_native_archive_files"] {
            let mut stmt = conn.prepare(&format!("PRAGMA diagnostic.table_info({table})")).unwrap();
            let columns = stmt.query_map([],|r|r.get::<_,String>(1)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            let columns = columns.iter().map(|c|format!("\"{c}\"")).collect::<Vec<_>>().join(",");
            conn.execute_batch(&format!("INSERT INTO main.{table} (rowid,{columns}) SELECT rowid,{columns} FROM diagnostic.{table};")).unwrap();
        }
        conn.execute_batch("DETACH DATABASE diagnostic; PRAGMA foreign_keys=ON;").unwrap();
    }
    let id:String = store.conn.lock().unwrap().query_row("SELECT id FROM sessions",[],|r|r.get(0)).unwrap();
    let remote = recovery::recorded_remote(&store,&id).unwrap().unwrap();
    let snapshot = if let Ok(path)=std::env::var("LOCUS_RECOVERY_NATIVE_SNAPSHOT") {
        serde_json::from_slice::<NativeSnapshot>(&std::fs::read(path).unwrap()).unwrap()
    } else {
        archive::project(&store.claude_archive_info(&id).unwrap().unwrap(),&store.claude_archive_files(&id).unwrap()).unwrap()
    };
    snapshot.can_continue().unwrap();
    let before = store.get_messages(&id).unwrap();
    let pending = before.last().filter(|m|m.role==MessageRole::User).map(|m|m.id.clone());
    assert_eq!(store.claude_recovery_pending_message(&id,&before,&store.get_response_request_metadata(&id).unwrap()).unwrap(),pending);
    let todos_before = store.get_todos(&id).unwrap();
    store.reconcile_claude_native(&id,&remote,&snapshot,&before,pending.as_deref()).unwrap();
    let after = store.get_messages(&id).unwrap();
    assert_eq!(after.len(),before.len());
    assert_eq!(serde_json::to_value(&store.get_todos(&id).unwrap().items).unwrap(),serde_json::to_value(&todos_before.items).unwrap());
    for old in &before {
        let new = after.iter().find(|m|m.id==old.id).expect("original message ID retained");
        if old.role==MessageRole::User { assert_eq!(new.content,old.content); assert_eq!(serde_json::to_value(&new.images).unwrap(),serde_json::to_value(&old.images).unwrap()); }
        for tool in old.tool_calls.iter().flatten() { assert!(new.tool_calls.iter().flatten().any(|t|t.id==tool.id)); }
        assert_eq!(new.tool_call_id,old.tool_call_id);
    }
    assert!(!store.reconcile_claude_native(&id,&remote,&snapshot,&after,pending.as_deref()).unwrap());
    let expected_output:u64 = snapshot.messages.iter().filter_map(|m|m.usage.as_ref()).map(|v|v["output_tokens"].as_u64().unwrap_or(0)).sum();
    assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens,expected_output);
    let export = root.path().join("export.yaml");
    crate::session::context_export::export_session_context_yaml(&store,&id,"F:/test",None,None,&export).unwrap();
    let yaml:serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(export).unwrap()).unwrap();
    assert_eq!(yaml["sessions"][0]["messages"].as_sequence().unwrap().len(),before.len());
    eprintln!("Recovered {} messages, preserved every local ID, output tokens {}",after.len(),expected_output);
}
