use super::*;

#[test]
fn claude_history_retention_survives_rebuild_retry_missed_compaction_and_reopen() {
    for mode in ["synchronize", "rebuild", "retry"] {
        for compact_received in [false, true] {
            for has_old_usage in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let store = SessionStore::new(root.path()).unwrap();
                let id = store.create_session("retained transcript", None, None, "chat", None).unwrap();
                let remote: NativeConnection = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
                let native_id = Uuid::new_v4().to_string();
                store.bind_claude_native_session(&id, &remote, &native_id).unwrap();
                let old_user = store.add_message(&id, MessageRole::User, "original request").unwrap();
                let old_answer = store.add_message(&id, MessageRole::Assistant, "original answer").unwrap();
                for message_id in [&old_user, &old_answer] {
                    let mut metadata = json!({"backend":"claude_code_cli","claude_connection":remote,
                        "claude_checkpoint":{"version":1,"sessionId":native_id,"uuid":Uuid::new_v4()}});
                    if message_id == &old_answer && has_old_usage {
                        metadata["claude_usage"] = json!({"input_tokens":10,"output_tokens":5});
                    }
                    store.set_message_response_request_metadata(&id, message_id, &metadata).unwrap();
                }
                let handoff = compact_received.then(|| {
                    let handoff = crate::compact::claude_native::build_handoff_message("native summary", 1, false);
                    let metadata = store.get_response_request_metadata(&id).unwrap();
                    store.compact_messages_retaining(&id, &handoff, &old_answer, 0, &[old_answer.clone()], Some(&json!({
                        "backend":"claude_code_cli","claude_connection":remote,
                        "claude_checkpoint":metadata[&old_answer]["claude_checkpoint"],
                        "claude_compaction":{"type":"compact_done","summary":"native summary","keptMessageIds":[old_answer]}
                    }))).unwrap();
                    handoff.id
                });
                let pending = store.add_message(&id, MessageRole::User, "continue").unwrap();
                store.set_message_response_request_metadata(&id, &pending, &json!({"backend":"claude_code_cli",
                    "claude_pending_input":{"version":1,"token":Uuid::new_v4()}})).unwrap();
                let snapshot: NativeSnapshot = serde_json::from_value(json!({
                    "version":1,"nativeId":native_id,"digest":"a".repeat(64),"todos":[],
                    "incompleteTail":false,"pendingToolIds":[],"messages":[
                        {"key":"summary","role":"user","uuids":[Uuid::new_v4()],"content":"native summary","compactSummary":true},
                        {"key":"new-answer","role":"assistant","uuids":[Uuid::new_v4()],"content":"new answer",
                            "model":"opus","usage":{"input_tokens":8,"output_tokens":7}}
                    ]
                })).unwrap();
                for _ in 0..2 {
                    let before = store.get_messages(&id).unwrap();
                    match mode {
                        "synchronize" => store.reconcile_claude_native(&id, &remote, &snapshot, &before, Some(&pending)),
                        "rebuild" => store.rebuild_claude_native(&id, &remote, &snapshot, &before, Some(&pending)),
                        _ => store.rebuild_claude_native_for_retry(&id, &remote, &snapshot, &before, Some(&pending)),
                    }.unwrap();
                    let transcript = store.get_messages(&id).unwrap();
                    assert_eq!(transcript.len(), 5, "{mode}, compact={compact_received}, usage={has_old_usage}");
                    assert_eq!((&transcript[0].id, transcript[0].content.as_str()), (&old_user, "original request"));
                    assert_eq!((&transcript[1].id, transcript[1].content.as_str()), (&old_answer, "original answer"));
                    if let Some(handoff) = &handoff { assert_eq!(&transcript[2].id, handoff); }
                    let prompt = store.get_messages_for_prompt(&id).unwrap();
                    assert_eq!(prompt.len(), 3);
                    assert!(!prompt.iter().any(|m| m.id == old_user || m.id == old_answer));
                    assert!(prompt.iter().any(|m| m.id == pending));
                    assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens, if has_old_usage { 12 } else { 7 });
                }
                drop(store);
                let store = SessionStore::new(root.path()).unwrap();
                let page = store.load_session_tail_page(&id, 3).unwrap();
                assert!(page.has_more_history);
                assert!(page.messages.iter().any(|m| m.content == CONTEXT_COMPACTED_DISPLAY_MARKER));
                let older = store.load_session_message_page(&id, page.oldest_message_row_id.unwrap(), 3).unwrap();
                assert_eq!(older.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [old_user.as_str(), old_answer.as_str()]);
                let output = root.path().join("context.yaml");
                crate::session::context_export::export_session_context_yaml(&store, &id, "F:/test", None, None, &output).unwrap();
                let yaml: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
                let exported = yaml["sessions"][0]["messages"].as_sequence().unwrap();
                assert_eq!(exported.len(), 5);
                assert_eq!(exported[0]["content"].as_str(), Some("original request"));
                assert_eq!(exported[1]["content"].as_str(), Some("original answer"));
                assert_eq!(exported[1]["claudeNative"].as_str(), Some("empty"));
            }
        }
    }
}

#[test]
fn claude_history_retention_requires_native_confirmation_and_excludes_it_from_context_without_summary() {
    for retry in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("confirmed history", None, None, "chat", None).unwrap();
        let remote: NativeConnection = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
        let native_id = Uuid::new_v4().to_string();
        let hidden_uuid = Uuid::new_v4();
        let mut confirmed = String::new();
        for kind in ["confirmed", "wrong-session", "provider-only", "hidden-command"] {
            let role = if kind == "hidden-command" { MessageRole::User } else { MessageRole::Assistant };
            let message = store.add_message(&id, role, kind).unwrap();
            let mut metadata = json!({"backend":"claude_code_cli","claude_connection":remote});
            if kind != "provider-only" {
                metadata["claude_checkpoint"] = json!({"version":1,
                    "uuid":if kind == "hidden-command" { hidden_uuid } else { Uuid::new_v4() },
                    "sessionId":if kind == "wrong-session" { Uuid::new_v4().to_string() } else { native_id.clone() }});
            }
            store.set_message_response_request_metadata(&id, &message, &metadata).unwrap();
            if kind == "confirmed" { confirmed = message; }
        }
        let snapshot: NativeSnapshot = serde_json::from_value(json!({
            "version":1,"nativeId":native_id,"digest":"b".repeat(64),"todos":[],
            "incompleteTail":false,"pendingToolIds":[],"hiddenUserUuids":[hidden_uuid],"messages":[
                {"key":"active","role":"user","uuids":[Uuid::new_v4()],"content":"active request"}
            ]
        })).unwrap();
        let before = store.get_messages(&id).unwrap();
        if retry {
            store.rebuild_claude_native_for_retry(&id, &remote, &snapshot, &before, None).unwrap();
        } else {
            store.rebuild_claude_native(&id, &remote, &snapshot, &before, None).unwrap();
        }
        let transcript = store.get_messages(&id).unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].id, confirmed);
        let prompt = store.get_messages_for_prompt(&id).unwrap();
        assert_eq!(prompt.len(), 1);
        assert_eq!(prompt[0].content, "active request");
    }
}

#[test]
fn claude_history_retention_keeps_tool_results_and_usage_once_during_rebuild_and_retry() {
    for retry in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store.create_session("tool transcript", None, None, "chat", None).unwrap();
        let remote: NativeConnection = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
        let mut snapshot: NativeSnapshot = serde_json::from_value(json!({
            "version":1,"nativeId":Uuid::new_v4(),"digest":"a".repeat(64),"todos":[],
            "incompleteTail":false,"pendingToolIds":[],"messages":[
                {"key":"user","role":"user","uuids":[Uuid::new_v4()],"content":"read file"},
                {"key":"answer","role":"assistant","uuids":[Uuid::new_v4()],
                    "content":[{"type":"tool_use","id":"read-once","name":"read","input":{"path":"file"}}],
                    "model":"opus","usage":{"input_tokens":11,"output_tokens":7}},
                {"key":"result","role":"tool","uuids":[Uuid::new_v4()],"toolCallId":"read-once","content":"original result"}
            ]
        })).unwrap();
        store.reconcile_claude_native(&id, &remote, &snapshot, &[], None).unwrap();
        let original = store.get_messages(&id).unwrap();
        let mut summary = snapshot.messages[0].clone();
        summary.key = "summary".into();
        summary.uuids = vec![Uuid::new_v4().to_string()];
        summary.content = json!("native summary");
        summary.compact_summary = true;
        snapshot.messages = vec![summary];
        snapshot.digest = "b".repeat(64);
        for _ in 0..2 {
            let before = store.get_messages(&id).unwrap();
            if retry {
                store.rebuild_claude_native_for_retry(&id, &remote, &snapshot, &before, None).unwrap();
            } else {
                store.rebuild_claude_native(&id, &remote, &snapshot, &before, None).unwrap();
            }
            let transcript = store.get_messages(&id).unwrap();
            assert_eq!(transcript.len(), 4);
            assert_eq!(serde_json::to_value(&transcript[..3]).unwrap(), serde_json::to_value(&original).unwrap());
            assert_eq!(store.get_messages_for_prompt(&id).unwrap().len(), 1);
            assert_eq!(store.get_token_usage(&id).unwrap().total_output_tokens, 7);
            let display = store.load_session_view(&id, 20).unwrap();
            assert_eq!(display.session.messages[1].tool_calls.as_ref().unwrap()[0].recorded_output.as_deref(), Some("original result"));
        }
    }
}

#[test]
fn claude_history_retention_keeps_unconfirmed_rounds_in_transcript_order() {
    // A cancelled run saves its streamed round with the CLI checkpoint, but the
    // CLI never commits it to its transcript. Synchronization renumbers the
    // native rows; the retained round must keep its place among them.
    for committed in [&[true, true, false][..], &[true, true, false, true, true][..]] {
        for mode in ["synchronize", "passive", "rebuild", "retry"] {
            let root = tempfile::tempdir().unwrap();
            let store = SessionStore::new(root.path()).unwrap();
            let id = store.create_session("cancelled round", None, None, "chat", None).unwrap();
            let remote: NativeConnection = serde_json::from_value(json!({"target":"unused.invalid"})).unwrap();
            let native_id = Uuid::new_v4().to_string();
            store.bind_claude_native_session(&id, &remote, &native_id).unwrap();
            let mut local = Vec::new();
            let mut native = Vec::new();
            for (index, committed) in committed.iter().enumerate() {
                let role = if index % 3 == 0 { MessageRole::User } else { MessageRole::Assistant };
                let text = format!("message {index}");
                let message = store.add_message(&id, role.clone(), &text).unwrap();
                let uuid = Uuid::new_v4();
                store.set_message_response_request_metadata(&id, &message, &json!({"backend":"claude_code_cli",
                    "claude_connection":remote,"claude_checkpoint":{"version":1,"sessionId":native_id,"uuid":uuid}})).unwrap();
                if *committed {
                    let content = if role == MessageRole::User { json!(text) } else { json!([{"type":"text","text":text}]) };
                    native.push(json!({"key":uuid,"role":role.as_str(),"uuids":[uuid],"content":content}));
                }
                local.push(message);
            }
            let uncommitted = local[2].clone();
            let snapshot: NativeSnapshot = serde_json::from_value(json!({
                "version":1,"nativeId":native_id,"digest":"a".repeat(64),"todos":[],
                "incompleteTail":false,"pendingToolIds":[],"messages":native
            })).unwrap();
            for _ in 0..2 {
                let before = store.get_messages(&id).unwrap();
                match mode {
                    "synchronize" => store.reconcile_claude_native(&id, &remote, &snapshot, &before, None),
                    "passive" => store.reconcile_claude_native_if_idle(&id, &remote, &snapshot, &before),
                    "rebuild" => store.rebuild_claude_native(&id, &remote, &snapshot, &before, None),
                    _ => store.rebuild_claude_native_for_retry(&id, &remote, &snapshot, &before, None),
                }.unwrap();
                let transcript: Vec<_> = store.get_messages(&id).unwrap().into_iter().map(|m| m.id).collect();
                assert_eq!(transcript, local, "{mode}, rows={}", committed.len());
                let prompt = store.get_messages_for_prompt(&id).unwrap();
                assert_eq!(prompt.len(), committed.len() - 1, "{mode}");
                assert!(!prompt.iter().any(|m| m.id == uncommitted), "{mode}");
            }
        }
    }
}
