use super::*;
use serde_json::{json, Value};

fn insert_response(conn: &Connection, message: &str, request: &Value) {
    let (id, payload) = response_request_payload(request).unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO response_request_payloads(id,payload_json) VALUES (?1,?2)",
        params![id, payload],
    )
    .unwrap();
    conn.execute("INSERT INTO messages(id,session_id,role,content,created_at,response_request_id) VALUES (?1,'session','assistant','answer',1,?2)", params![message,id]).unwrap();
}

fn insert_session(conn: &Connection) {
    conn.execute_batch("INSERT INTO sessions(id,title,session_type,created_at,updated_at) VALUES ('session','Model test','chat',1,1);").unwrap();
}

fn exported_model(store: &SessionStore, session: &str, path: &Path) -> Value {
    crate::session::context_export::export_session_context_yaml(
        store, session, "", None, None, path,
    )
    .unwrap();
    let yaml: Value = serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    yaml["sessions"][0]["messages"][0]["upstreamModel"].clone()
}

#[test]
fn upstream_model_v46_migration_recovers_headers_and_is_repeatable() {
    for (events, expected) in [
        (
            json!({"headers":{"openai-model":"terminal"},"response.metadata":{"headers":{"openai-model":"earlier"}}}),
            json!("terminal"),
        ),
        (
            json!({"response.metadata":{"headers":{"OpenAI-Model":"actual"}}}),
            json!("actual"),
        ),
        (
            json!({"response.created":{"response":{"model":"echoed"}}}),
            json!("echoed"),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("locus.db")).unwrap();
        SessionStore::create_latest_schema(&conn).unwrap();
        insert_session(&conn);
        let original =
            json!({"model":"requested","codex_response":{"version":1,"output":[],"events":events}});
        insert_response(&conn, "answer", &original);
        conn.pragma_update(None, "user_version", 46).unwrap();
        drop(conn);
        let store = SessionStore::new(dir.path()).unwrap();
        let requests = store.get_response_request_metadata("session").unwrap();
        let mut migrated = original;
        migrated["codex_response"]["server_model"] = expected.clone();
        migrated["upstream_model"] = expected.clone();
        assert_eq!(requests["answer"], migrated);
        {
            let conn = store.conn.lock().unwrap();
            SessionStore::migrate_upstream_model(&conn).unwrap();
            SessionStore::migrate_upstream_model(&conn).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM response_request_payloads",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0))
                    .unwrap(),
                SessionStore::schema_version()
            );
        }
        assert_eq!(
            requests,
            store.get_response_request_metadata("session").unwrap()
        );
        assert_eq!(
            store
                .get_latest_upstream_model("session")
                .unwrap()
                .as_deref(),
            expected.as_str()
        );
        assert_eq!(
            exported_model(&store, "session", &dir.path().join("export.yaml")),
            if expected.is_null() {
                json!("empty")
            } else {
                expected
            }
        );
    }
}

#[test]
fn upstream_model_new_database_survives_reload_fork_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    {
        let conn = store.conn.lock().unwrap();
        insert_session(&conn);
        insert_response(
            &conn,
            "answer",
            &json!({"model":"requested","codex_response":{"server_model":"actual"}}),
        );
    }
    let fork = store
        .fork_session_from_message("session", "answer", None)
        .unwrap();
    drop(store);
    let store = SessionStore::new(dir.path()).unwrap();
    for session in ["session", fork.as_str()] {
        assert_eq!(
            store.get_latest_upstream_model(session).unwrap().as_deref(),
            Some("actual")
        );
        assert_eq!(
            exported_model(&store, session, &dir.path().join(format!("{session}.yaml"))),
            "actual"
        );
    }
}

#[test]
fn upstream_model_latest_response_missing_headers_does_not_reuse_older_model() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    assert_eq!(store.get_latest_upstream_model("session").unwrap(), None);
    {
        let conn = store.conn.lock().unwrap();
        insert_session(&conn);
        insert_response(
            &conn,
            "first",
            &json!({"codex_response":{"server_model":"older"}}),
        );
        insert_response(&conn, "second", &json!({"model":"different-provider"}));
    }
    assert_eq!(store.get_latest_upstream_model("session").unwrap(), None);
}

#[test]
fn upstream_model_v47_recovers_null_body_models_and_preserves_export_and_references() {
    for (response, expected) in [
        (
            json!({"server_model":null,"events":{"model":"terminal-model"}}),
            json!("terminal-model"),
        ),
        (
            json!({"server_model":null,"events":{"response.created":{"response":{"model":"first-model"}},"model":"terminal-model"}}),
            json!("terminal-model"),
        ),
        (
            json!({"server_model":"header-model","events":{"model":"body-model"}}),
            json!("header-model"),
        ),
        (json!({"server_model":null,"events":{}}), Value::Null),
        (Value::Null, Value::Null),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("locus.db")).unwrap();
        SessionStore::create_latest_schema(&conn).unwrap();
        insert_session(&conn);
        let request = json!({"model":"requested", "codex_response":response});
        insert_response(&conn, "answer", &request);
        insert_response(&conn, "same-payload", &request);
        conn.pragma_update(None, "user_version", 47).unwrap();
        drop(conn);
        let store = SessionStore::new(dir.path()).unwrap();
        let metadata = store.get_response_request_metadata("session").unwrap();
        assert_eq!(metadata["answer"]["upstream_model"], expected);
        assert_eq!(metadata["answer"], metadata["same-payload"]);
        {
            let conn = store.conn.lock().unwrap();
            SessionStore::migrate_upstream_model_declarations(&conn).unwrap();
            SessionStore::migrate_upstream_model_declarations(&conn).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM response_request_payloads",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        assert_eq!(
            metadata,
            store.get_response_request_metadata("session").unwrap()
        );
        assert_eq!(
            store
                .get_latest_upstream_model("session")
                .unwrap()
                .as_deref(),
            expected.as_str()
        );
        assert_eq!(
            exported_model(&store, "session", &dir.path().join("export.yaml")),
            if expected.is_null() {
                json!("empty")
            } else {
                expected
            }
        );
    }
}

#[test]
fn upstream_model_generic_protocol_metadata_survives_reload_fork_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let mut request = None;
    crate::llm::upstream_model::record(
        &mut request,
        r#"{"model":"claude-requested"}"#,
        "event: message_start\ndata: {\"message\":{\"model\":\"claude-reported\"}}\n\n",
    );
    {
        let conn = store.conn.lock().unwrap();
        insert_session(&conn);
        insert_response(&conn, "answer", &request.unwrap());
    }
    let fork = store
        .fork_session_from_message("session", "answer", None)
        .unwrap();
    drop(store);
    let store = SessionStore::new(dir.path()).unwrap();
    for session in ["session", fork.as_str()] {
        assert_eq!(
            store.get_latest_upstream_model(session).unwrap().as_deref(),
            Some("claude-reported")
        );
        assert_eq!(
            exported_model(&store, session, &dir.path().join(format!("{session}.yaml"))),
            "claude-reported"
        );
    }
}
