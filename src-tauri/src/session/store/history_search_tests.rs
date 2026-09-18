use super::super::tests::fixture;
use super::*;

#[test]
fn global_search_fields_separate_titles_content_and_project_scope() {
    let (_dir, store, active, archived, other) = fixture();
    insert_messages(&store, &active, 3, "internal-marker");
    let titles = store
        .search_session_history_fields(
            "project", "shader", false, None, 20, None, true, false, false, true,
        )
        .unwrap();
    assert_eq!(titles.scanned_messages, 2);
    assert!(titles
        .matches
        .iter()
        .all(|hit| hit.updated_at > 1_700_000_000));
    assert_eq!(titles.matches.len(), 2);
    assert!(titles
        .matches
        .iter()
        .all(|hit| hit.field == "title" && hit.message_id.is_none()));
    assert!(titles.matches.iter().any(|hit| hit.session_id == other));
    assert!(!titles.matches.iter().any(|hit| hit.session_id == archived));
    let content = store
        .search_session_history_fields(
            "project",
            "internal-marker",
            false,
            None,
            20,
            None,
            false,
            true,
            false,
            true,
        )
        .unwrap();
    assert_eq!(content.matches.len(), 1);
    assert!(content.matches.iter().all(|hit| hit.field == "content"));
    let outside = store
        .search_session_history_fields(
            "other-project",
            "shader",
            false,
            None,
            20,
            None,
            true,
            true,
            false,
            true,
        )
        .unwrap();
    assert!(outside.matches.is_empty());
    let archived = store
        .search_session_history_fields(
            "project", "shader", true, None, 20, None, true, false, false, true,
        )
        .unwrap();
    assert_eq!(archived.matches.len(), 1);
}

#[test]
fn global_search_title_previews_use_latest_visible_text_and_keep_sdk_title_excerpts() {
    let (_dir, store, active, archived, _) = fixture();
    let text = format!("Visible preview {}", "中文🙂".repeat(200));
    store
        .add_message(&active, MessageRole::Assistant, &text)
        .unwrap();
    store
        .add_message(&active, MessageRole::Tool, "hidden tool payload")
        .unwrap();
    store
        .add_message(&active, MessageRole::Assistant, " \n\t ")
        .unwrap();
    let empty = store
        .create_session_scoped(
            "Shader empty",
            None,
            Some("project"),
            Some("checkout-a"),
            "chat",
            None,
        )
        .unwrap();
    let page = store
        .search_session_history_fields(
            "project", "shader", false, None, 20, None, true, false, false, true,
        )
        .unwrap();
    let hit = page
        .matches
        .iter()
        .find(|hit| hit.session_id == active)
        .unwrap();
    assert!(hit.excerpt.starts_with("Visible preview 中文🙂"));
    assert!(hit.excerpt.ends_with('…'));
    assert_eq!(hit.excerpt.chars().count(), 321);
    assert_eq!(hit.field, "title");
    assert!(hit.message_id.is_none());
    assert!(page
        .matches
        .iter()
        .find(|hit| hit.session_id == empty)
        .unwrap()
        .excerpt
        .is_empty());
    assert!(!page.matches.iter().any(|hit| hit.session_id == archived));
    let archived_page = store
        .search_session_history_fields(
            "project", "shader", true, None, 20, None, true, false, false, true,
        )
        .unwrap();
    assert_eq!(
        archived_page.matches[0].excerpt,
        "检查 Shader 编译 100%_literal"
    );
    let sdk = store
        .search_session_history("checkout-a", "shader", false, Some(&active), 20, None)
        .unwrap();
    assert_eq!(
        sdk.matches
            .iter()
            .find(|hit| hit.field == "title")
            .unwrap()
            .excerpt,
        "Shader 调试"
    );
}

#[test]
fn global_search_content_excludes_tool_payloads_and_resumes_without_duplicates() {
    let (_dir, store, active, _, _) = fixture();
    insert_messages(&store, &active, 1100, "content-hit");
    store
        .add_message(&active, MessageRole::Tool, "hidden-tool-hit")
        .unwrap();
    let mut cursor = None;
    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
        let page = store
            .search_session_history_fields(
                "checkout-a",
                "content-hit",
                false,
                None,
                20,
                cursor.as_deref(),
                false,
                true,
                false,
                false,
            )
            .unwrap();
        assert!(page.scanned_messages <= MAX_SCAN_MESSAGES);
        for hit in page.matches {
            assert!(ids.insert(hit.message_id.unwrap()));
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none());
    assert_eq!(ids.len(), 1100);
    let mut cursor = None;
    loop {
        let page = store
            .search_session_history_fields(
                "checkout-a",
                "hidden-tool-hit",
                false,
                None,
                20,
                cursor.as_deref(),
                false,
                true,
                false,
                false,
            )
            .unwrap();
        assert!(page.matches.is_empty());
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
}

fn insert_messages(store: &SessionStore, session: &str, count: usize, text: &str) {
    let mut conn = store.conn.lock().unwrap();
    let transaction = conn.transaction().unwrap();
    {
        let mut statement = transaction.prepare(
            "INSERT INTO messages(id,session_id,role,content,thinking_content,tool_calls,created_at)
             VALUES (?1,?2,'assistant',?3,?3,?3,1)"
        ).unwrap();
        for n in 0..count {
            statement
                .execute(params![format!("bulk-{session}-{n}"), session, text])
                .unwrap();
        }
    }
    transaction.commit().unwrap();
}

#[test]
fn search_budget_returns_empty_continuations_without_rescanning_large_history() {
    let (_dir, store, active, _, _) = fixture();
    insert_messages(&store, &active, 1500, &"unmatched ".repeat(100));
    let mut page = store
        .search_session_history("checkout-a", "shader", false, Some(&active), 20, None)
        .unwrap();
    let mut matches = Vec::new();
    let mut total_rows = 0;
    let mut total_bytes = 0;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 30);
        assert!(page.scanned_messages <= MAX_SCAN_MESSAGES);
        total_rows += page.scanned_messages;
        total_bytes += page.scanned_bytes;
        matches.extend(page.matches);
        let Some(cursor) = page.next_cursor else {
            break;
        };
        page = store
            .search_session_history(
                "checkout-a",
                "shader",
                false,
                Some(&active),
                20,
                Some(&cursor),
            )
            .unwrap();
    }
    assert!(pages >= 3);
    assert_eq!(matches.len(), 2);
    assert!(total_rows >= 1501 && total_rows <= 1501 + pages);
    assert!(total_bytes >= 4_500_000 && total_bytes < 4_501_000);
}

#[test]
fn byte_budget_and_field_cursor_preserve_hits_in_a_single_large_message() {
    let (_dir, store, active, _, _) = fixture();
    let text = format!("{}NEEDLE", "x".repeat(5 * 1024 * 1024));
    insert_messages(&store, &active, 1, &text);
    let mut cursor = None;
    let mut fields = Vec::new();
    let mut bytes = 0;
    for _ in 0..10 {
        let page = store
            .search_session_history(
                "checkout-a",
                "needle",
                false,
                Some(&active),
                20,
                cursor.as_deref(),
            )
            .unwrap();
        assert!(page.scanned_bytes < MAX_SCAN_BYTES + text.len() as u64 + 100);
        fields.extend(page.matches.into_iter().map(|hit| hit.field));
        bytes += page.scanned_bytes;
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none());
    assert_eq!(fields, ["content", "thinking", "tool_calls"]);
    assert!(bytes >= text.len() as u64 * 3 && bytes < text.len() as u64 * 3 + 1000);
}

#[test]
fn search_resumes_each_field_and_excludes_new_messages_after_the_first_page() {
    let (_dir, store, active, _, _) = fixture();
    insert_messages(&store, &active, 1, "needle");
    let mut page = store
        .search_session_history("checkout-a", "needle", false, Some(&active), 1, None)
        .unwrap();
    store
        .add_message(&active, MessageRole::User, "new needle")
        .unwrap();
    let mut fields = Vec::new();
    loop {
        fields.extend(page.matches.into_iter().map(|hit| hit.field));
        let Some(cursor) = page.next_cursor else {
            break;
        };
        page = store
            .search_session_history(
                "checkout-a",
                "needle",
                false,
                Some(&active),
                1,
                Some(&cursor),
            )
            .unwrap();
    }
    assert_eq!(fields, ["content", "thinking", "tool_calls"]);
}

#[test]
fn search_cursor_rejects_changed_query_scope_and_survives_deleted_rows() {
    let (_dir, store, active, _, _) = fixture();
    let first = store
        .search_session_history("checkout-a", "Shader", false, Some(&active), 1, None)
        .unwrap();
    let cursor = first.next_cursor.as_deref();
    for (checkout, query, archived, selected) in [
        ("checkout-b", "Shader", false, Some(active.as_str())),
        ("checkout-a", "other", false, Some(active.as_str())),
        ("checkout-a", "Shader", true, Some(active.as_str())),
        ("checkout-a", "Shader", false, None),
    ] {
        assert!(store
            .search_session_history(checkout, query, archived, selected, 1, cursor)
            .is_err());
    }
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![active],
        )
        .unwrap();
    let end = store
        .search_session_history("checkout-a", "Shader", false, Some(&active), 1, cursor)
        .unwrap();
    assert!(end.matches.is_empty());
    assert!(end.next_cursor.is_none());
}

#[test]
fn search_uses_session_rowid_index_and_does_not_take_the_writer_mutex() {
    let (_dir, store, active, _, _) = fixture();
    let guard = store.conn.lock().unwrap();
    let reader_store = store.clone();
    let reader_session = active.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        sender
            .send(reader_store.search_session_history(
                "checkout-a",
                "shader",
                false,
                Some(&reader_session),
                20,
                None,
            ))
            .unwrap()
    });
    let page = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("search must not wait on the writer mutex")
        .unwrap();
    reader.join().unwrap();
    assert_eq!(page.matches.len(), 2);
    let plan: Vec<String> = guard
        .prepare(&format!("EXPLAIN QUERY PLAN {MESSAGE_QUERY}"))
        .unwrap()
        .query_map(
            params![active, i64::MAX, MESSAGE_BATCH, "checkout-a", false],
            |r| r.get(3),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|line| line.contains("idx_messages_session") && line.contains("rowid<?")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|line| line.contains("SCAN ") || line.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

#[test]
fn literal_search_keeps_unicode_case_semantics_and_metacharacters() {
    for query in ["中文", "[a]*?%_\\", "É"] {
        assert!(LiteralMatcher::new(query)
            .find(&format!("prefix {query} suffix"))
            .is_some());
    }
    assert!(LiteralMatcher::new("É").find("é").is_none());
    assert!(LiteralMatcher::new("SHADER").find("shader").is_some());
}

#[test]
#[ignore = "large synthetic history benchmark; run explicitly with --ignored --nocapture"]
fn history_search_large_session_benchmark() {
    let (_dir, store, active, _, _) = fixture();
    let text = format!("common needle {}", "x".repeat(2034));
    insert_messages(&store, &active, 100_000, &text);
    let started = Instant::now();
    let first = store
        .search_session_history("checkout-a", "common", false, Some(&active), 20, None)
        .unwrap();
    let first_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let next = store
        .search_session_history(
            "checkout-a",
            "common",
            false,
            Some(&active),
            20,
            first.next_cursor.as_deref(),
        )
        .unwrap();
    let next_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let mut page = store
        .search_session_history("checkout-a", "not-found", false, Some(&active), 20, None)
        .unwrap();
    let miss_first_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut pages = 1;
    let mut bytes = page.scanned_bytes;
    let mut max_page_ms = miss_first_ms;
    while let Some(cursor) = page.next_cursor {
        let page_start = Instant::now();
        page = store
            .search_session_history(
                "checkout-a",
                "not-found",
                false,
                Some(&active),
                20,
                Some(&cursor),
            )
            .unwrap();
        max_page_ms = max_page_ms.max(page_start.elapsed().as_secs_f64() * 1000.0);
        bytes += page.scanned_bytes;
        pages += 1;
        assert!(pages < 1000);
    }
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(first.matches.len(), 20);
    assert_eq!(next.matches.len(), 20);
    std::println!(
        "HISTORY_SEARCH_BENCH {}",
        serde_json::json!({
            "messages":100_000,"scannedBytes":bytes,"firstHitMs":first_ms,"nextHitMs":next_ms,
            "firstMissMs":miss_first_ms,"maxMissPageMs":max_page_ms,"fullMissMs":total_ms,"missPages":pages,
        })
    );
    let baseline_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/session-search-performance/baseline.sql");
    if let Ok(sql) = std::fs::read_to_string(baseline_path) {
        let conn =
            Connection::open_with_flags(&store.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        for query in ["common", "not-found"] {
            let started = Instant::now();
            let mut stmt = conn.prepare(&sql).unwrap();
            let count = stmt
                .query_map(params!["checkout-a", false, active, query, 21, 0], |_| {
                    Ok(())
                })
                .unwrap()
                .count();
            std::println!(
                "HISTORY_SEARCH_BASELINE {}",
                serde_json::json!({"query":query,"rows":count,"ms":started.elapsed().as_secs_f64()*1000.0})
            );
        }
    }
}
