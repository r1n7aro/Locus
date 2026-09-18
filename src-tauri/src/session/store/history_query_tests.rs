use super::*;

pub(super) fn fixture() -> (tempfile::TempDir, SessionStore, String, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    for checkout in ["checkout-a", "checkout-b"] {
        let root = dir.path().join(checkout).to_string_lossy().into_owned();
        store
            .upsert_workspace_checkout(&WorkspaceCheckoutRecord {
                checkout_id: checkout.into(),
                project_id: "project".into(),
                root_path: root.clone(),
                normalized_root: root,
                last_opened_at: 1,
            })
            .unwrap();
    }
    let active = store
        .create_session_scoped(
            "Shader 调试",
            None,
            Some("project"),
            Some("checkout-a"),
            "chat",
            None,
        )
        .unwrap();
    let archived = store
        .create_session_scoped(
            "Old Shader",
            None,
            Some("project"),
            Some("checkout-a"),
            "chat",
            None,
        )
        .unwrap();
    let other = store
        .create_session_scoped(
            "Other Shader",
            None,
            Some("project"),
            Some("checkout-b"),
            "chat",
            None,
        )
        .unwrap();
    for id in [&active, &archived, &other] {
        store
            .add_message(id, MessageRole::User, "检查 Shader 编译 100%_literal")
            .unwrap();
    }
    store.archive_session(&archived).unwrap();
    (dir, store, active, archived, other)
}

#[test]
fn history_queries_filter_checkout_archive_and_selected_session() {
    let (_dir, store, active, archived, other) = fixture();
    assert_eq!(
        store.list_sessions_for_checkout("checkout-a").unwrap()[0].id,
        active
    );
    assert_eq!(
        store
            .list_archived_sessions_for_checkout("checkout-a")
            .unwrap()[0]
            .id,
        archived
    );
    for (archived_flag, expected) in [(false, &active), (true, &archived)] {
        let page = store
            .search_session_history("checkout-a", "sHaDeR", archived_flag, None, 20, None)
            .unwrap();
        assert_eq!(page.matches.len(), 2);
        assert!(page.matches.iter().all(|hit| &hit.session_id == expected));
        assert!(page
            .matches
            .iter()
            .any(|hit| hit.field == "title" && hit.message_id.is_none()));
        assert!(page
            .matches
            .iter()
            .any(|hit| hit.field == "content" && hit.message_row_id.is_some()));
        assert_eq!(page.next_cursor, None);
    }
    assert!(store
        .search_session_history("checkout-a", "Shader", true, Some(&active), 20, None)
        .unwrap()
        .matches
        .is_empty());
    assert!(store
        .search_session_history("checkout-a", "Shader", false, Some(&other), 20, None)
        .is_err());
    assert!(store
        .read_session_history("checkout-a", &other, None, 5)
        .is_err());
    assert!(store
        .read_session_history("checkout-a", "missing", None, 5)
        .is_err());
    assert_eq!(
        store
            .read_session_history("checkout-a", &archived, None, 5)
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn history_search_is_literal_has_bounded_unicode_excerpts_and_pages() {
    let (_dir, store, active, _, _) = fixture();
    let long = format!("{}命中标记{}", "前".repeat(10_000), "后".repeat(10_000));
    store
        .add_message(&active, MessageRole::Assistant, &long)
        .unwrap();
    let page = store
        .search_session_history("checkout-a", "命中标记", false, Some(&active), 1, None)
        .unwrap();
    assert_eq!(page.matches.len(), 1);
    assert!(page.matches[0].excerpt.contains("命中标记"));
    assert!(page.matches[0].excerpt.chars().count() <= 322);
    let literal = store
        .search_session_history("checkout-a", "%_", false, None, 20, None)
        .unwrap();
    assert_eq!(literal.matches.len(), 1);
    assert!(store
        .search_session_history("checkout-a", "%' OR 1=1 --", false, None, 20, None)
        .unwrap()
        .matches
        .is_empty());
    let first = store
        .search_session_history("checkout-a", "shader", false, None, 1, None)
        .unwrap();
    let second = store
        .search_session_history(
            "checkout-a",
            "shader",
            false,
            None,
            1,
            first.next_cursor.as_deref(),
        )
        .unwrap();
    assert_ne!(first.matches[0].field, second.matches[0].field);
    let end = store
        .search_session_history(
            "checkout-a",
            "shader",
            false,
            None,
            1,
            second.next_cursor.as_deref(),
        )
        .unwrap();
    assert!(end.matches.is_empty());
    assert!(end.next_cursor.is_none());
    for query in ["", " \n ", "\0"] {
        assert!(store
            .search_session_history("checkout-a", query, false, None, 20, None)
            .is_err());
    }
    assert!(store
        .search_session_history("checkout-a", "x", false, None, 0, None)
        .is_err());
    assert!(store
        .search_session_history("checkout-a", "x", false, None, 101, None)
        .is_err());
    assert!(store
        .search_session_history("checkout-a", "x", false, None, 20, Some("invalid"))
        .is_err());
}

#[test]
fn history_pages_use_insert_order_and_do_not_repeat_when_new_messages_arrive() {
    let (_dir, store, active, _, _) = fixture();
    let mut ids = vec![store.load_session(&active).unwrap().messages[0].id.clone()];
    for n in 0..4 {
        ids.push(
            store
                .add_message(&active, MessageRole::Assistant, &format!("message {n}"))
                .unwrap(),
        );
    }
    // Equal timestamps must never become the pagination boundary.
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE messages SET created_at = 1 WHERE session_id = ?1",
            params![active],
        )
        .unwrap();
    let latest = store
        .read_session_history("checkout-a", &active, None, 2)
        .unwrap();
    assert_eq!(
        latest.messages.iter().map(|m| &m.id).collect::<Vec<_>>(),
        ids[3..].iter().collect::<Vec<_>>()
    );
    assert!(latest.has_more_history);
    store
        .add_message(&active, MessageRole::Assistant, "new arrival")
        .unwrap();
    let middle = store
        .read_session_history("checkout-a", &active, latest.oldest_message_row_id, 2)
        .unwrap();
    assert_eq!(
        middle.messages.iter().map(|m| &m.id).collect::<Vec<_>>(),
        ids[1..3].iter().collect::<Vec<_>>()
    );
    assert!(middle.has_more_history);
    let oldest = store
        .read_session_history("checkout-a", &active, middle.oldest_message_row_id, 2)
        .unwrap();
    assert_eq!(oldest.messages[0].id, ids[0]);
    assert!(!oldest.has_more_history);
    let empty = store
        .read_session_history("checkout-a", &active, oldest.oldest_message_row_id, 2)
        .unwrap();
    assert!(empty.messages.is_empty());
    assert_eq!(empty.oldest_message_row_id, None);
    assert!(!empty.has_more_history);
    for (cursor, limit) in [(Some(0), 2), (Some(-1), 2), (None, 0), (None, 1001)] {
        assert!(store
            .read_session_history("checkout-a", &active, cursor, limit)
            .is_err());
    }
}

#[test]
fn history_pages_preserve_tool_rounds_and_search_thinking_and_tool_arguments() {
    let (_dir, store, active, _, _) = fixture();
    let assistant = store
        .add_message_with_thinking(
            &active,
            MessageRole::Assistant,
            "Inspecting",
            Some("reasoning needle"),
            None,
            None,
            None,
            Some(&serde_json::json!({"model": "test", "large": "x".repeat(50_000)})),
        )
        .unwrap();
    let tool = store
        .add_message(&active, MessageRole::Tool, "tool result needle")
        .unwrap();
    {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE messages SET tool_calls = ?2 WHERE id = ?1",
            params![
                assistant,
                r#"[{"id":"call-1","name":"read","arguments":"{\"path\":\"argument needle\"}"}]"#
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE messages SET tool_call_id = 'call-1' WHERE id = ?1",
            params![tool],
        )
        .unwrap();
    }
    let page = store
        .read_session_history("checkout-a", &active, None, 1)
        .unwrap();
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].id, assistant);
    assert_eq!(page.messages[1].id, tool);
    assert_eq!(
        page.messages[0].tool_calls.as_ref().unwrap()[0]
            .recorded_output
            .as_deref(),
        Some("tool result needle")
    );
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("responseRequest"));
    assert!(json.len() < 10_000);
    let hits = store
        .search_session_history("checkout-a", "needle", false, Some(&active), 20, None)
        .unwrap();
    assert_eq!(hits.matches.len(), 3);
    assert!(hits.matches.iter().any(|hit| hit.field == "thinking"));
    assert!(hits.matches.iter().any(|hit| hit.field == "tool_calls"));
}
