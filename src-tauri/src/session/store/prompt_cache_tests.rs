use super::*;
use tempfile::tempdir;

fn fixture() -> (tempfile::TempDir, SessionStore, String, String) {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store.create_session("cache", None, None, "chat", None).unwrap();
    let user = store.add_message(&session, MessageRole::User, "long task").unwrap();
    (dir, store, session, user)
}

fn record(session: &str, user: &str, step: &str, input: u64, read: u64, write: u64) -> ModelUsageRecord {
    ModelUsageRecord {
        session_id: session.into(), model_id: "claude_code/test".into(), provider: "Claude Code CLI".into(),
        request_kind: "completion".into(), input_tokens: input, output_tokens: 10,
        cache_read_tokens: read, cache_write_tokens: write, model_active_duration_ms: 100,
        cost_usd: 0.0, priced_rounds: 0, context_tokens: Some((input + read + write) as u32), context_limit: Some(200_000),
        cache_request: Some(PromptCacheRequest::new(step.into(), Some(user.into()))),
    }
}

#[test]
fn cache_writes_detect_misses_in_later_steps_of_the_same_user_message() {
    let (_dir, store, session, user) = fixture();
    let (_, cold) = store.record_model_usage_request(record(&session, &user, "1", 0, 0, 100_000)).unwrap();
    assert_eq!(cold.unwrap().reason, "no_baseline");
    let (_, healthy) = store.record_model_usage_request(record(&session, &user, "2", 5, 100_000, 1000)).unwrap();
    assert!(!healthy.unwrap().invalidated);
    let (_, missed) = store.record_model_usage_request(record(&session, &user, "3", 5, 0, 102_000)).unwrap();
    let missed = missed.unwrap();
    assert!(missed.invalidated);
    assert_eq!(missed.excess_input_tokens, 101_005);
    let rows = store.list_cache_invalidations(&session).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id, "3");
    assert_eq!(rows[0].request_sequence, 3);
    assert_eq!(rows[0].message_id, user);
    assert_eq!(rows[0].cache_write_tokens, Some(102_000));
    assert_eq!(store.get_token_usage(&session).unwrap().total_input_tokens, 10);
    assert_eq!(store.cache_detection_summary(&session).unwrap().checked_requests, 2);
}

#[test]
fn cache_formula_excludes_normal_growth_and_keeps_strict_threshold() {
    for (input, read, write, expected, excess) in [
        (5, 100_000, 150_000, false, 0),
        (5, 80_000, 21_000, false, 20_000),
        (0, 20_000, 80_000, false, 80_000),
        (0, 19_999, 80_001, true, 80_001),
        (101_005, 0, 0, true, 100_000),
        (5, 0, 101_000, true, 100_000),
    ] {
        let (_dir, store, session, user) = fixture();
        store.record_model_usage_request(record(&session, &user, "1", 0, 0, 100_000)).unwrap();
        let (_, check) = store.record_model_usage_request(record(&session, &user, "2", input, read, write)).unwrap();
        let check = check.unwrap();
        assert_eq!(check.invalidated, expected, "{input}/{read}/{write}");
        assert_eq!(check.excess_input_tokens, excess);
    }
}

#[test]
fn cache_receipt_replay_does_not_duplicate_billing_or_checks_and_keeps_captured_user() {
    let (_dir, store, session, user) = fixture();
    store.record_model_usage_request(record(&session, &user, "1", 0, 0, 100_000)).unwrap();
    let pending_response = record(&session, &user, "2", 5, 0, 101_000);
    store.add_message(&session, MessageRole::User, "queued while requesting").unwrap();
    let (before, _) = store.record_model_usage_request(pending_response.clone()).unwrap();
    let (after, duplicate) = store.record_model_usage_best_effort(pending_response).unwrap();
    assert!(duplicate.is_none());
    assert_eq!(before.total_input_tokens, after.total_input_tokens);
    assert_eq!(before.total_cache_write_tokens, after.total_cache_write_tokens);
    assert_eq!(before.total_output_tokens, after.total_output_tokens);
    assert_eq!(before.model_active_duration_ms, after.model_active_duration_ms);
    let events = store.list_cache_invalidations(&session).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message_id, user);
    let conn = store.conn.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM model_usage_events WHERE session_id=?1", [&session], |r| r.get::<_, i64>(0)).unwrap(), 2);
}

#[test]
fn cache_missing_usage_breaks_comparison_until_a_new_valid_baseline() {
    let (_dir, store, session, user) = fixture();
    store.record_model_usage_request(record(&session, &user, "1", 0, 0, 100_000)).unwrap();
    let mut missing = record(&session, &user, "2", 0, 0, 0);
    missing.cache_request.as_mut().unwrap().usage_complete = false;
    let (_, check) = store.record_model_usage_request(missing).unwrap();
    assert_eq!(check.unwrap().reason, "usage_unavailable");
    let (_, next) = store.record_model_usage_request(record(&session, &user, "3", 5, 0, 101_000)).unwrap();
    assert_eq!(next.unwrap().reason, "baseline_unavailable");
    assert!(store.list_cache_invalidations(&session).unwrap().is_empty());
    assert_eq!(store.cache_detection_summary(&session).unwrap().unavailable_requests, 2);
    assert!(!claude_prompt_usage_complete(&serde_json::json!({"input_tokens":1})));
    assert!(!claude_prompt_usage_complete(&serde_json::Value::Null));
    assert!(claude_prompt_usage_complete(&serde_json::json!({
        "input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0})));
}

#[test]
fn cache_compaction_resets_comparison_and_forks_preserve_diagnostics() {
    let (_dir, store, session, user) = fixture();
    store.record_model_usage_request(record(&session, &user, "1", 0, 0, 100_000)).unwrap();
    store.record_model_usage_request(record(&session, &user, "2", 5, 0, 101_000)).unwrap();
    let summary = crate::compact::build_post_compact_message("short context", "", SessionStore::now_ts(), false, None);
    store.compact_messages(&session, &summary, &user, 0).unwrap();
    let fork = store.fork_session(&session, None).unwrap();
    let original = store.list_cache_invalidations(&session).unwrap();
    let copied = store.list_cache_invalidations(&fork).unwrap();
    assert_eq!(serde_json::to_value(&original).unwrap(), serde_json::to_value(&copied).unwrap());
    for target in [&session, &fork] {
        let (_, check) = store.record_model_usage_request(record(target, &user, "3", 0, 0, 90_000)).unwrap();
        assert_eq!(check.unwrap().reason, "context_reset");
        assert_eq!(store.list_cache_invalidations(target).unwrap().len(), 1);
    }
}

#[test]
fn cache_model_identity_is_canonical_and_model_and_provider_changes_are_distinct() {
    let (_dir, store, session, user) = fixture();
    let mut first = record(&session, &user, "1", 0, 0, 100_000);
    first.model_id = "test".into();
    store.record_model_usage_request(first).unwrap();
    let (_, same) = store.record_model_usage_request(record(&session, &user, "2", 0, 100_000, 0)).unwrap();
    assert_eq!(same.unwrap().reason, "below_threshold");
    let mut changed = record(&session, &user, "3", 0, 100_000, 0);
    changed.model_id = "claude_code/other".into();
    assert_eq!(store.record_model_usage_request(changed.clone()).unwrap().1.unwrap().reason, "model_changed");
    changed.cache_request.as_mut().unwrap().key = "4".into();
    changed.provider = "Another provider".into();
    assert_eq!(store.record_model_usage_request(changed).unwrap().1.unwrap().reason, "provider_changed");
}

#[test]
fn claude_cache_recovery_replays_responses_once_and_handles_boundaries_and_missing_usage() {
    use crate::llm::claude_code_cli::{recovery::NativeSnapshot, connection::NativeConnection};
    use serde_json::json;
    let (_dir, store, session, _user) = fixture();
    let remote: NativeConnection = serde_json::from_value(json!({"target":"test@127.0.0.1","port":22242})).unwrap();
    let native_id = Uuid::new_v4().to_string();
    let assistant = |key: &str, read: u64, write: u64| json!({
        "key":key,"role":"assistant","uuids":[Uuid::new_v4().to_string()],
        "content":[{"type":"text","text":key}],"model":"test",
        "usage":{"input_tokens":0,"output_tokens":10,"cache_read_input_tokens":read,"cache_creation_input_tokens":write},
        "timestamp":"2026-10-15T00:00:00Z"});
    let mut missing = assistant("missing", 0, 0);
    missing["usage"] = serde_json::Value::Null;
    let mut snapshot: NativeSnapshot = serde_json::from_value(json!({
        "version":1,"nativeId":native_id,"digest":"a".repeat(64),"todos":[],"incompleteTail":false,"pendingToolIds":[],
        "messages":[
            {"key":"user","role":"user","uuids":[Uuid::new_v4().to_string()],"content":"long task"},
            assistant("cold",0,100_000), assistant("hit",100_000,1000), assistant("miss",0,102_000),
            {"key":"summary","role":"user","uuids":[Uuid::new_v4().to_string()],"content":"short summary","compactSummary":true},
            assistant("after-compact",0,90_000), missing, assistant("new-baseline",0,95_000), assistant("miss-again",0,96_000)
        ]})).unwrap();
    store.rebuild_claude_native(&session, &remote, &snapshot, &store.get_messages(&session).unwrap(), None).unwrap();
    let events = store.list_cache_invalidations(&session).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_id, format!("native:{native_id}:miss-again"));
    assert_eq!(events[1].event_id, format!("native:{native_id}:miss"));
    assert_eq!(events[1].baseline_tokens, 101_000);
    let coverage = store.cache_detection_summary(&session).unwrap();
    assert_eq!(coverage.checked_requests, 3);
    assert_eq!(coverage.baseline_requests, 2);
    assert_eq!(coverage.unavailable_requests, 2);
    let usage = store.get_token_usage(&session).unwrap();
    assert!(!store.reconcile_claude_native(&session, &remote, &snapshot, &store.get_messages(&session).unwrap(), None).unwrap());
    snapshot.digest = "b".repeat(64);
    assert!(store.reconcile_claude_native(&session, &remote, &snapshot, &store.get_messages(&session).unwrap(), None).unwrap());
    assert_eq!(serde_json::to_value(&events).unwrap(), serde_json::to_value(store.list_cache_invalidations(&session).unwrap()).unwrap());
    assert_eq!(usage.total_cache_write_tokens, store.get_token_usage(&session).unwrap().total_cache_write_tokens);
    // Existing matching native metadata from an older client must still trigger
    // the one-time diagnostic replay, even when the native digest did not change.
    for message in store.get_messages(&session).unwrap() {
        let mut metadata = store.get_response_request_metadata(&session).unwrap()[&message.id].clone();
        metadata["claude_native"].as_object_mut().unwrap().remove("cacheAlgorithmVersion");
        store.set_message_response_request_metadata(&session, &message.id, &metadata).unwrap();
    }
    store.conn.lock().unwrap().execute("DELETE FROM session_prompt_cache_checks WHERE session_id=?1", [&session]).unwrap();
    assert!(store.reconcile_claude_native(&session, &remote, &snapshot, &store.get_messages(&session).unwrap(), None).unwrap());
    assert_eq!(store.list_cache_invalidations(&session).unwrap().len(), 2);
}

#[test]
fn v51_cache_migration_preserves_legacy_checks_without_inventing_cache_writes() {
    let (dir, store, session, user) = fixture();
    {
        let conn = store.conn.lock().unwrap();
        conn.execute_batch("DROP TABLE session_prompt_cache_checks;
            CREATE TABLE session_prompt_cache_checks (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, message_id TEXT NOT NULL,
                message TEXT NOT NULL, model_id TEXT NOT NULL, baseline_tokens INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL, excess_input_tokens INTEGER NOT NULL,
                invalidated INTEGER NOT NULL, reason TEXT NOT NULL, created_at INTEGER NOT NULL,
                UNIQUE(session_id,message_id)); PRAGMA user_version=51;").unwrap();
        conn.execute("INSERT INTO session_prompt_cache_checks VALUES(1,?1,?2,'legacy','model',100,90,0,90,1,'input_growth_exceeds_context_threshold',123)", params![session,user]).unwrap();
    }
    drop(store);
    let store = SessionStore::new(dir.path()).unwrap();
    let legacy = store.list_cache_invalidations(&session).unwrap();
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].algorithm_version, 1);
    assert_eq!(legacy[0].event_id, "legacy:1");
    assert_eq!(legacy[0].cache_write_tokens, None);
    assert_eq!(legacy[0].occurred_at, 123);
    assert_eq!(store.cache_detection_summary(&session).unwrap().legacy_checks, 1);
    let (_, fresh) = store.record_model_usage_request(record(&session, &user, "new1", 5, 0, 100_000)).unwrap();
    assert_eq!(fresh.unwrap().reason, "baseline_unavailable");
    store.record_model_usage_request(record(&session, &user, "new2", 5, 0, 101_000)).unwrap();
    assert_eq!(store.list_cache_invalidations(&session).unwrap().len(), 2);
}
