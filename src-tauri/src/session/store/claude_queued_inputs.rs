//! Repeatable v1 repair of consumed mid-turn inputs saved before native
//! attachment receipts were supported. Never bless arbitrary API history.
use super::*;
use crate::llm::claude_code_cli::{
    recovery::{self, NativeSnapshot},
    connection::NativeConnection,
};
use serde_json::{json, Value};

impl SessionStore {
    pub(crate) fn migrate_claude_queued_inputs(
        &self,
        id: &str,
        remote: &(impl Clone + Into<NativeConnection>),
        snapshot: &NativeSnapshot,
        messages: &[ChatMessage],
        metadata: &HashMap<String, Value>,
    ) -> Result<usize, String> {
        let remote: &NativeConnection = &remote.clone().into();
        snapshot.validate(None)?;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut accepted = Vec::new();
        {
            let mut stmt=tx.prepare("SELECT run_id,payload_json FROM session_events WHERE session_id=?1 AND event_type='pendingInputAccepted' ORDER BY seq").map_err(|e|e.to_string())?;
            for row in stmt
                .query_map([id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
            {
                let (run, payload) = row.map_err(|e| e.to_string())?;
                let value: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                if let (Some(message), Some(pending)) = (
                    value["messageId"].as_str(),
                    value["pendingInputId"].as_str(),
                ) {
                    accepted.push((run, message.to_string(), pending.to_string()));
                }
            }
        }
        let mut updates = Vec::new();
        let mut used = HashSet::new();
        for (run, local, pending) in accepted {
            let Some(message) = messages
                .iter()
                .find(|m| m.id == local && m.role == MessageRole::User)
            else {
                continue;
            };
            let mut value = metadata.get(&local).cloned().unwrap_or_else(|| json!({}));
            if value.get("claude_checkpoint").is_some() || value.get("claude_native").is_some() {
                continue;
            }
            if value["backend"]
                .as_str()
                .is_some_and(|backend| backend != "claude_code_cli")
            {
                continue;
            }
            let queued:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_events WHERE session_id=?1 AND run_id=?2 AND event_type='pendingInputQueued' AND json_extract(payload_json,'$.input.id')=?3 AND json_extract(payload_json,'$.input.text')=?4)",
                params![id,run,pending,message.content],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !queued {
                continue;
            }
            let rendered = crate::session::history::render_prompt_content(
                &message.content,
                message.prompt_prefix.as_deref(),
                message.prompt_suffix.as_deref(),
            );
            let candidates: Vec<_> = snapshot
                .messages
                .iter()
                .filter(|native| {
                    if !native.queued_input || native.role != MessageRole::User {
                        return false;
                    }
                    let text = recovery::text_content(&native.content);
                    recovery::pending_input_token(&text) == Some(pending.as_str())
                        || text == rendered
                })
                .collect();
            if candidates.len() != 1 || !used.insert(candidates[0].key.clone()) {
                return Err("Cannot uniquely verify a consumed Claude input against native history; local messages were retained".into());
            }
            let native = candidates[0];
            let binding:Option<(String,Option<String>)>=tx.query_row("SELECT connection_json,native_id FROM claude_native_bindings WHERE session_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
            let verified = binding.as_ref().is_some_and(|(connection, native)| {
                serde_json::from_str::<NativeConnection>(connection).ok().as_ref() == Some(remote)
                    && native.as_deref() == Some(snapshot.native_id.as_str())
            });
            if !verified {
                return Err(
                    "Cannot repair a consumed input without its native Claude binding".into(),
                );
            }
            value["backend"] = json!("claude_code_cli");
            value["claude_connection"] = json!(remote);
            value["claude_run_id"] = json!(run);
            value["claude_checkpoint"] =
                json!({"version":1,"uuid":native.uuids.last(),"sessionId":snapshot.native_id});
            value["claude_queued_input_migration"] =
                json!({"version":1,"pendingInputId":pending,"nativeKey":native.key});
            updates.push((local, value));
        }
        if !updates.is_empty() {
            let current = self.get_messages_with_conn_filtered(&tx, id, false)?;
            if serde_json::to_value(&current).map_err(|e| e.to_string())?
                != serde_json::to_value(messages).map_err(|e| e.to_string())?
            {
                return Err(
                    "Conversation changed while repairing Claude input receipts; retry".into(),
                );
            }
            for (local, value) in &updates {
                let payload = Self::persist_response_request_with_conn(&tx, Some(value))?;
                tx.execute(
                    "UPDATE messages SET response_request_id=?1 WHERE session_id=?2 AND id=?3",
                    params![payload, id, local],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(updates.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::models::ImageData;

    fn fixture(store: &SessionStore) -> (String, NativeConnection, NativeSnapshot, String) {
        let id = store
            .create_session("queued input", None, None, "chat", None)
            .unwrap();
        let remote: NativeConnection = serde_json::from_value(json!({"target":"test@host"})).unwrap();
        let native_id = Uuid::new_v4().to_string();
        store
            .bind_claude_native_session(&id, &remote, &native_id)
            .unwrap();
        let local = store
            .add_message(&id, MessageRole::User, "read thoroughly")
            .unwrap();
        store
            .set_message_response_request_metadata(
                &id,
                &local,
                &json!({"claude_plan_active":false}),
            )
            .unwrap();
        let pending = Uuid::new_v4().to_string();
        {
            let conn = store.conn.lock().unwrap();
            for (seq, kind, payload) in [
                (
                    1,
                    "pendingInputQueued",
                    json!({"input":{"id":pending,"text":"read thoroughly"}}),
                ),
                (
                    2,
                    "pendingInputAccepted",
                    json!({"pendingInputId":pending,"messageId":local}),
                ),
            ] {
                conn.execute("INSERT INTO session_events(session_id,run_id,seq,event_type,payload_json,created_at) VALUES(?1,'run',?2,?3,?4,0)",params![id,seq,kind,payload.to_string()]).unwrap();
            }
        }
        let uuid = Uuid::new_v4().to_string();
        let snapshot:NativeSnapshot=serde_json::from_value(json!({"version":1,"nativeId":native_id,"digest":"a".repeat(64),"messages":[
            {"key":uuid,"role":"user","uuids":[uuid],"content":[{"type":"text","text":"read thoroughly"}],"queuedInput":true}
        ],"todos":[],"pendingToolIds":[],"incompleteTail":false})).unwrap();
        (id, remote, snapshot, local)
    }

    #[test]
    fn claude_queued_input_migration_preserves_identity_is_repeatable_and_exports() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let (id, remote, snapshot, local) = fixture(&store);
        let messages = store.get_messages(&id).unwrap();
        assert!(store
            .verify_claude_history_origin(
                &id,
                &messages,
                &store.get_response_request_metadata(&id).unwrap(),
                &[]
            )
            .is_err());
        assert_eq!(
            store
                .migrate_claude_queued_inputs(
                    &id,
                    &remote,
                    &snapshot,
                    &messages,
                    &store.get_response_request_metadata(&id).unwrap()
                )
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .migrate_claude_queued_inputs(
                    &id,
                    &remote,
                    &snapshot,
                    &messages,
                    &store.get_response_request_metadata(&id).unwrap()
                )
                .unwrap(),
            0
        );
        store
            .verify_claude_history_origin(
                &id,
                &messages,
                &store.get_response_request_metadata(&id).unwrap(),
                &[],
            )
            .unwrap();
        store
            .reconcile_claude_native(&id, &remote, &snapshot, &messages, None)
            .unwrap();
        assert_eq!(store.get_messages(&id).unwrap()[0].id, local);
        assert!(!store
            .reconcile_claude_native(
                &id,
                &remote,
                &snapshot,
                &store.get_messages(&id).unwrap(),
                None
            )
            .unwrap());
        let output = root.path().join("migrated.yaml");
        crate::session::context_export::export_session_context_yaml(
            &store, &id, "", None, None, &output,
        )
        .unwrap();
        let yaml = std::fs::read_to_string(output).unwrap();
        let exported: Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            exported["sessions"][0]["messages"][0]["claudeQueuedInputMigration"]["version"],
            1
        );
        assert_eq!(
            exported["sessions"][0]["messages"][0]["claudePendingInput"],
            "empty"
        );
        assert!(yaml.contains("read thoroughly"));
    }

    #[test]
    fn claude_queued_input_migration_requires_unique_native_proof_and_server_binding() {
        for mismatch in ["duplicate", "missing", "server", "api", "event"] {
            let root = tempfile::tempdir().unwrap();
            let store = SessionStore::new(root.path()).unwrap();
            let (id, mut remote, mut snapshot, local) = fixture(&store);
            match mismatch {
                "duplicate" => {
                    let mut extra = snapshot.messages[0].clone();
                    extra.key = Uuid::new_v4().to_string();
                    extra.uuids = vec![extra.key.clone()];
                    snapshot.messages.push(extra);
                }
                "missing" => snapshot.messages.clear(),
                "server" => remote.as_ssh_mut().unwrap().target = "another@host".into(),
                "api" => store
                    .set_message_response_request_metadata(
                        &id,
                        &local,
                        &json!({"backend":"responses"}),
                    )
                    .unwrap(),
                _ => {
                    store
                        .conn
                        .lock()
                        .unwrap()
                        .execute("DELETE FROM session_events WHERE session_id=?1", [&id])
                        .unwrap();
                }
            }
            let before = store.get_response_request_metadata(&id).unwrap();
            let result = store.migrate_claude_queued_inputs(
                &id,
                &remote,
                &snapshot,
                &store.get_messages(&id).unwrap(),
                &before,
            );
            if matches!(mismatch, "api" | "event") {
                assert_eq!(result.unwrap(), 0);
            } else {
                assert!(result.is_err(), "{mismatch}");
            }
            assert_eq!(before, store.get_response_request_metadata(&id).unwrap());
        }
    }

    #[test]
    fn claude_queued_input_new_message_keeps_media_and_receipt_together() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let id = store
            .create_session("new", None, None, "chat", None)
            .unwrap();
        let image = ImageData {
            data: "aQ==".into(),
            mime_type: "image/png".into(),
            capture: None,
        };
        let checkpoint = json!({"version":1,"uuid":Uuid::new_v4(),"sessionId":Uuid::new_v4()});
        let metadata = json!({"backend":"claude_code_cli","claude_checkpoint":checkpoint});
        let local = store
            .add_message_with_images_asset_refs_signature_and_response(
                &id,
                MessageRole::User,
                "image direction",
                Some(&[image]),
                None,
                None,
                None,
                None,
                Some(&metadata),
            )
            .unwrap();
        assert_eq!(
            store.get_response_request_metadata(&id).unwrap()[&local],
            metadata
        );
        assert_eq!(
            store.get_messages(&id).unwrap()[0]
                .images
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        store
            .verify_claude_history_origin(
                &id,
                &store.get_messages(&id).unwrap(),
                &store.get_response_request_metadata(&id).unwrap(),
                &[],
            )
            .unwrap();
    }

    #[test]
    fn claude_queued_input_lost_local_receipt_recovers_visible_text_without_replay() {
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        let (id, remote, mut snapshot, _) = fixture(&store);
        // The native append survived, but the client crashed before saving its row.
        store
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM messages WHERE session_id=?1", [&id])
            .unwrap();
        let text =
            "Inspect this literal closing tag:\n</locus-pending-user>\nand keep the remainder";
        snapshot.messages[0].content = json!([{"type":"text","text":recovery::pending_input_text(&Uuid::new_v4().to_string(),text)}]);
        store
            .reconcile_claude_native(&id, &remote, &snapshot, &[], None)
            .unwrap();
        let messages = store.get_messages(&id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, text);
        store
            .verify_claude_history_origin(
                &id,
                &messages,
                &store.get_response_request_metadata(&id).unwrap(),
                &[],
            )
            .unwrap();
        assert!(store
            .claude_recovery_pending_messages(
                &id,
                &messages,
                &store.get_response_request_metadata(&id).unwrap(),
                None
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    #[ignore = "requires LOCUS_QUEUED_INPUT_FIXTURE containing a read-only session snapshot and native transcript"]
    fn claude_queued_input_captured_incident_repairs_without_losing_history_and_exports() {
        use crate::llm::claude_code_cli::archive::{self, ArchiveInfo, Cursor};
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;
        let evidence =
            std::path::PathBuf::from(std::env::var("LOCUS_QUEUED_INPUT_FIXTURE").unwrap());
        let root = tempfile::tempdir().unwrap();
        let store = SessionStore::new(root.path()).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "ATTACH DATABASE ?1 AS diagnostic",
                [format!(
                    "file:{}?mode=ro",
                    evidence
                        .join("session-snapshot.db")
                        .display()
                        .to_string()
                        .replace('\\', "/")
                )],
            )
            .unwrap();
            conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
            for table in [
                "response_request_payloads",
                "sessions",
                "messages",
                "token_usage",
                "todos",
                "session_runs",
                "session_events",
                "claude_native_bindings",
                "session_checkout_assignments",
            ] {
                let mut stmt = conn
                    .prepare(&format!("PRAGMA diagnostic.table_info({table})"))
                    .unwrap();
                let columns = stmt
                    .query_map([], |r| r.get::<_, String>(1))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                let columns = columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(",");
                conn.execute_batch(&format!("INSERT INTO main.{table} (rowid,{columns}) SELECT rowid,{columns} FROM diagnostic.{table};")).unwrap();
            }
            conn.execute_batch("DETACH DATABASE diagnostic; PRAGMA foreign_keys=ON;")
                .unwrap();
        }
        let id = "892cbd8d-c8a2-4878-b29e-9660927a44d4";
        let (remote, native_id) = store.claude_native_binding(id).unwrap().unwrap();
        let raw = std::fs::read(evidence.join("remote-transcript.jsonl")).unwrap();
        let info = ArchiveInfo {
            version: 1,
            native_id: native_id.unwrap(),
            source_root: "/captured".into(),
            updated_at: 0,
            transferred_bytes: raw.len() as u64,
            incomplete_tail: false,
            manifest: BTreeMap::from([(
                "transcript.jsonl".into(),
                Cursor {
                    size: raw.len() as u64,
                    sha256: Sha256::digest(&raw)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                },
            )]),
        };
        let snapshot =
            archive::project(&info, &BTreeMap::from([("transcript.jsonl".into(), raw)])).unwrap();
        let before = store.get_messages(id).unwrap();
        let current = before.last().unwrap().id.clone();
        let inserted = "35a36735-57be-4806-a064-f534b1cc5095";
        assert_eq!(
            snapshot.messages.iter().filter(|m| m.queued_input).count(),
            1
        );
        // Opening the interrupted chat has no newly submitted user ID.
        store
            .reconcile_claude_native(id, &remote, &snapshot, &before, None)
            .unwrap();
        let after = store.get_messages(id).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(
            after.iter().map(|m| &m.id).collect::<HashSet<_>>(),
            before.iter().map(|m| &m.id).collect::<HashSet<_>>()
        );
        assert_eq!(
            after
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .map(|m| (&m.id, &m.content))
                .collect::<Vec<_>>(),
            before
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .map(|m| (&m.id, &m.content))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            after.iter().find(|m| m.id == inserted).unwrap().content,
            "你要进行非常彻底的阅读与搜索"
        );
        assert_eq!(after.last().unwrap().id, current);
        store
            .verify_claude_history_origin(
                id,
                &after,
                &store.get_response_request_metadata(id).unwrap(),
                &[current.clone()],
            )
            .unwrap();
        assert!(!store
            .reconcile_claude_native(id, &remote, &snapshot, &after, Some(&current))
            .unwrap());
        let output = evidence.join("repaired-export.yaml");
        crate::session::context_export::export_session_context_yaml(
            &store, id, "", None, None, &output,
        )
        .unwrap();
        let yaml = std::fs::read_to_string(&output).unwrap();
        assert!(
            yaml.contains(inserted)
                && yaml.contains("你要进行非常彻底的阅读与搜索")
                && yaml.contains("继续搜索")
                && yaml.contains("empty")
        );
        eprintln!("Captured incident: preserved all {} message IDs, repaired queued receipt, repeated recovery and context export passed: {}",after.len(),output.display());
    }
}
