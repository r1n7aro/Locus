use super::*;
use serde_json::{json, Value};

fn insert_request(conn: &Connection, request: &Value) {
    let (id, payload) = response_request_payload(request).unwrap();
    conn.execute(
        "INSERT INTO response_request_payloads(id,payload_json) VALUES (?1,?2)",
        params![id, payload],
    )
    .unwrap();
    conn.execute("INSERT INTO messages(id,session_id,role,content,created_at,response_request_id) VALUES ('answer','legacy','assistant','old answer',1,?1)", params![id]).unwrap();
}

#[test]
fn v45_reasoning_migration_is_repeatable_preserves_prefix_and_exports_empty() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("locus.db")).unwrap();
    SessionStore::create_latest_schema(&conn).unwrap();
    conn.execute_batch("INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('legacy','Legacy','chat',1,1); PRAGMA user_version=45;").unwrap();
    let original = json!({"model":"gpt-6-astra","reasoning":{"effort":"high"},"codex_response":{"version":1,"output":[]}});
    insert_request(&conn, &original);
    let mut key = json!({"provider":"openai_codex:default","model":"openai/gpt-6-astra","effort":"high","fastMode":false});
    conn.execute("INSERT INTO session_prompt_prefix_cache(session_id,provider_key,base_prompt,rules_prompt,knowledge_prompt,env_prompt,synthesized_at,last_remote_response_at) VALUES ('legacy',?1,'base','rules','knowledge','env',100,200)", params![key.to_string()]).unwrap();
    drop(conn);
    let store = SessionStore::new(dir.path()).unwrap();
    let requests = store.get_response_request_metadata("legacy").unwrap();
    let mut expected = original;
    expected["codex_reasoning"] = Value::Null;
    expected["codex_response"]["server_model"] = Value::Null;
    expected["upstream_model"] = Value::Null;
    assert_eq!(requests["answer"], expected);
    key["effort"] = Value::Null;
    let cached = store
        .fresh_prompt_prefix_cache("legacy", &key.to_string(), 300, 201)
        .unwrap()
        .unwrap();
    assert_eq!(cached.base_prompt, "base");
    assert_eq!(cached.synthesized_at, 100);
    assert_eq!(cached.last_remote_response_at, Some(200));
    {
        let conn = store.conn.lock().unwrap();
        SessionStore::migrate_codex_reasoning_updates(&conn).unwrap();
        SessionStore::migrate_codex_reasoning_updates(&conn).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM response_request_payloads", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            conn.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            SessionStore::schema_version()
        );
    }
    assert_eq!(
        requests,
        store.get_response_request_metadata("legacy").unwrap()
    );
    let export = dir.path().join("legacy.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, "legacy", "", None, None, &export,
    )
    .unwrap();
    let yaml: Value = serde_yaml::from_str(&std::fs::read_to_string(export).unwrap()).unwrap();
    assert_eq!(
        yaml["sessions"][0]["messages"][0]["codexReasoning"],
        "empty"
    );
    assert_eq!(yaml["sessions"][0]["messages"][0]["content"], "old answer");
    assert_eq!(yaml["sessions"][0]["messages"][0]["upstreamModel"], "empty");
}

#[test]
fn new_database_keeps_reasoning_updates_through_reload_fork_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let state = json!({"version":1,"effort":"max","update_trailing_items":1});
    {
        let conn = store.conn.lock().unwrap();
        conn.execute_batch("INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('legacy','New','chat',1,1);").unwrap();
        insert_request(
            &conn,
            &json!({"model":"gpt-6-astra","reasoning":{"effort":"low"},"codex_reasoning":state}),
        );
    }
    let fork = store
        .fork_session_from_message("legacy", "answer", None)
        .unwrap();
    drop(store);
    let store = SessionStore::new(dir.path()).unwrap();
    for session in ["legacy", fork.as_str()] {
        let requests = store.get_response_request_metadata(session).unwrap();
        assert_eq!(requests.values().next().unwrap()["codex_reasoning"], state);
        let path = dir.path().join(format!("{session}.yaml"));
        crate::session::context_export::export_session_context_yaml(
            &store, session, "", None, None, &path,
        )
        .unwrap();
        let yaml: Value = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(yaml["sessions"][0]["messages"][0]["codexReasoning"], state);
    }
}
