use super::*;

fn contract(description: &str) -> SessionToolContract {
    SessionToolContract {
        description: description.to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {"prompt": {"type": "string"}}}),
    }
}

#[test]
fn first_pinned_tool_definition_wins_survives_reload_and_both_fork_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Pinned", None, None, "chat", None)
        .unwrap();
    assert!(store
        .session_tool_contract(&session, "subagent")
        .unwrap()
        .is_none());

    let first = store
        .pin_session_tool_contract(&session, "subagent", &contract("first"))
        .unwrap();
    assert_eq!(first, contract("first"));
    let second = store
        .pin_session_tool_contract(&session, "subagent", &contract("changed settings"))
        .unwrap();
    assert_eq!(
        second,
        contract("first"),
        "a later pin must not replace the session's definition"
    );

    let message = store
        .add_message(&session, MessageRole::User, "Fork this context")
        .unwrap();
    let fork = store
        .fork_session_from_message(&session, &message, None)
        .unwrap();
    let snapshot = store.create_export_snapshot().unwrap();
    let snapshot_fork = store
        .fork_session_from_export_snapshot(&snapshot, &session, None)
        .unwrap();
    drop(snapshot);
    drop(store);

    let store = SessionStore::new(dir.path()).unwrap();
    for id in [&session, &fork, &snapshot_fork] {
        assert_eq!(
            store.session_tool_contract(id, "subagent").unwrap(),
            Some(contract("first"))
        );
    }
    let other = store
        .create_session("Other", None, None, "chat", None)
        .unwrap();
    assert!(store
        .session_tool_contract(&other, "subagent")
        .unwrap()
        .is_none());
    store.delete_session(&session).unwrap();
    assert!(store
        .session_tool_contract(&session, "subagent")
        .unwrap()
        .is_none());
}

#[test]
fn v55_database_gains_the_tool_contract_table() {
    let dir = tempfile::tempdir().unwrap();
    drop(SessionStore::new(dir.path()).unwrap());
    {
        let conn = Connection::open(dir.path().join("locus.db")).unwrap();
        conn.execute_batch("DROP TABLE session_tool_contracts; PRAGMA user_version = 55;")
            .unwrap();
    }
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Migrated", None, None, "chat", None)
        .unwrap();
    store
        .pin_session_tool_contract(&session, "subagent", &contract("migrated"))
        .unwrap();
    assert_eq!(
        store.session_tool_contract(&session, "subagent").unwrap(),
        Some(contract("migrated"))
    );
}
