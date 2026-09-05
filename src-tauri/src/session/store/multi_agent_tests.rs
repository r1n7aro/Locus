use super::*;

#[test]
fn multi_agent_selection_survives_reload_and_both_fork_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Multi agent", None, None, "chat", None)
        .unwrap();
    assert!(!store
        .get_session_multi_agent_enabled(&session)
        .unwrap()
        .unwrap_or(false));
    store
        .set_session_execution_state(
            &session,
            "openai/gpt-6-astra",
            Some("high"),
            false,
            Some(true),
        )
        .unwrap();
    let message = store.add_message(&session, MessageRole::User, "Fork this context").unwrap();
    let fork = store.fork_session_from_message(&session, &message, None).unwrap();
    let snapshot = store.create_export_snapshot().unwrap();
    let snapshot_fork = store
        .fork_session_from_export_snapshot(&snapshot, &session, None)
        .unwrap();
    drop(snapshot);
    drop(store);
    let store = SessionStore::new(dir.path()).unwrap();
    for id in [&session, &fork, &snapshot_fork] {
        assert_eq!(
            store.load_session(id).unwrap().last_multi_agent_enabled,
            Some(true)
        );
        assert_eq!(
            store
                .load_session_view(id, 20)
                .unwrap()
                .session
                .last_multi_agent_enabled,
            Some(true)
        );
    }
    store
        .set_session_execution_state(&session, "openai/gpt-6-astra", Some("max"), true, None)
        .unwrap();
    assert_eq!(
        store.get_session_multi_agent_enabled(&session).unwrap(),
        Some(true)
    );
    store
        .set_session_execution_state(
            &session,
            "openai/gpt-6-astra",
            Some("max"),
            true,
            Some(false),
        )
        .unwrap();
    assert_eq!(
        store.get_session_multi_agent_enabled(&session).unwrap(),
        Some(false)
    );
    assert_eq!(
        store.get_session_multi_agent_enabled(&fork).unwrap(),
        Some(true)
    );
}

#[test]
fn v43_multi_agent_migration_is_repeatable_and_exports_old_context() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("locus.db")).unwrap();
    SessionStore::create_latest_schema(&conn).unwrap();
    conn.execute_batch("ALTER TABLE sessions DROP COLUMN last_multi_agent_enabled;
        INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('legacy','Legacy','chat',1,1);
        INSERT INTO messages(id,session_id,role,content,created_at) VALUES ('old','legacy','user','keep this context',1);
        PRAGMA user_version = 43;").unwrap();
    drop(conn);
    let store = SessionStore::new(dir.path()).unwrap();
    assert_eq!(
        store
            .load_session("legacy")
            .unwrap()
            .last_multi_agent_enabled,
        None
    );
    {
        let conn = store.conn.lock().unwrap();
        SessionStore::migrate_multi_agent_selection(&conn).unwrap();
        SessionStore::migrate_multi_agent_selection(&conn).unwrap();
        assert_eq!(
            conn.pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            SessionStore::schema_version()
        );
    }
    let output = dir.path().join("legacy.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, "legacy", "", None, None, &output,
    )
    .unwrap();
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(
        yaml["sessions"][0]["metadata"]["lastMultiAgentEnabled"].as_str(),
        Some("empty")
    );
    assert!(store
        .get_messages_for_prompt("legacy")
        .unwrap()
        .iter()
        .any(|message| message.content == "keep this context"));
}
