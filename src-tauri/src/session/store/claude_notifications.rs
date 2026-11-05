//! Local delivery records are not independent Claude transcript messages.
//! Verify them against the durable outbox, and bind their text to a real input
//! or tool result before sending. Never manufacture native checkpoints for them.
use super::*;
use crate::llm::claude_code_cli::recovery::{self, NativeSnapshot};
use serde_json::{json, Value};

const RECEIPT: &str = "locus_claude_notification";

impl SessionStore {
    pub(crate) fn claude_notification_ids(
        &self,
        id: &str,
        messages: &[ChatMessage],
        metadata: &HashMap<String, Value>,
    ) -> Result<HashSet<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut expected = HashMap::new();
        let mut stmt = conn.prepare("SELECT n.task_id,n.attempt,n.reminder FROM async_task_notifications n JOIN async_task_results t ON t.task_id=n.task_id AND t.session_id=n.session_id WHERE n.session_id=?1 AND n.delivered=1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (task, attempt, reminder) = row.map_err(|e| e.to_string())?;
            let key = if attempt == 1 {
                format!("async-result:{task}")
            } else {
                format!("async-result:{task}:{attempt}")
            };
            expected.insert(key, vec![reminder]);
        }
        let mut stmt = conn.prepare("SELECT id,sender,body FROM agent_messages WHERE delivered=1 AND ((target_task_id IS NULL AND target_session_id=?1) OR target_task_id IN (SELECT task_id FROM async_task_results WHERE json_extract(snapshot_json,'$.resume.childSessionId')=?1))")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (key, sender, body) = row.map_err(|e| e.to_string())?;
            expected.insert(
                format!("agent-message:{key}"),
                Self::agent_message_reminder_variants(&sender, &body),
            );
        }
        let mut verified = HashSet::new();
        for message in messages {
            let Some(reminders) = expected.get(&message.id) else {
                continue;
            };
            // An ID prefix or a saved marker alone is never provenance. Match
            // the original outbox body, destination and complete message shape.
            if message.role != MessageRole::User
                || !message.content.is_empty()
                || !message
                    .prompt_suffix
                    .as_ref()
                    .is_some_and(|suffix| reminders.contains(suffix))
                || message
                    .prompt_prefix
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
                || message.tool_call_id.is_some()
                || message.tool_calls.as_ref().is_some_and(|v| !v.is_empty())
                || message.images.as_ref().is_some_and(|v| !v.is_empty())
                || message.asset_refs.as_ref().is_some_and(|v| !v.is_empty())
                || message
                    .thinking_content
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
            {
                continue;
            }
            if let Some(value) = metadata.get(&message.id) {
                if value["backend"]
                    .as_str()
                    .is_some_and(|backend| backend != "claude_code_cli")
                {
                    continue;
                }
                if value
                    .get(RECEIPT)
                    .is_some_and(|receipt| receipt["version"] != 1)
                {
                    return Err("Unsupported Claude notification receipt version".into());
                }
            }
            verified.insert(message.id.clone());
        }
        Ok(verified)
    }

    pub(crate) fn verify_claude_history_origin(
        &self,
        id: &str,
        messages: &[ChatMessage],
        metadata: &HashMap<String, Value>,
        pending: &[String],
    ) -> Result<(), String> {
        let notifications = self.claude_notification_ids(id, messages, metadata)?;
        if notifications.is_empty() {
            return recovery::verify_origin_with_pending(messages, metadata, pending);
        }
        let native: Vec<_> = messages
            .iter()
            .filter(|m| !notifications.contains(&m.id))
            .cloned()
            .collect();
        recovery::verify_origin_with_pending(&native, metadata, pending)
    }

    /// Explicit, repeatable v1 migration for older delivery rows. Only native
    /// transcript evidence acknowledges delivery; the outbox's `delivered`
    /// bit merely means that Locus saved the local notification.
    pub(super) fn claude_notification_migration(
        messages: &[ChatMessage],
        metadata: &HashMap<String, Value>,
        notifications: &HashSet<String>,
        snapshot: &NativeSnapshot,
    ) -> Vec<(String, Value)> {
        messages
            .iter()
            .filter(|m| notifications.contains(&m.id))
            .filter_map(|message| {
                let mut payload = metadata
                    .get(&message.id)
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let before = payload.clone();
                let mut receipt = payload
                    .get(RECEIPT)
                    .cloned()
                    .unwrap_or_else(|| json!({"version":1}));
                let reminder = message.prompt_suffix.as_deref().unwrap_or_default();
                if let Some(native) = snapshot.messages.iter().find(|m| {
                    matches!(m.role, MessageRole::User | MessageRole::Tool)
                        && recovery::text_content(&m.content).contains(reminder)
                }) {
                    receipt["nativeId"] = json!(snapshot.native_id);
                    receipt["uuid"] = json!(native.uuids.last());
                }
                payload[RECEIPT] = receipt;
                (payload != before).then(|| (message.id.clone(), payload))
            })
            .collect()
    }

    /// Persist both the real carrier's content and the local receipt together.
    /// Repeated preparation returns the same reminders without appending twice.
    pub(crate) fn bind_claude_notifications(
        &self,
        id: &str,
        carrier_id: &str,
    ) -> Result<Vec<String>, String> {
        let messages = self.get_messages(id)?;
        let metadata = self.get_response_request_metadata(id)?;
        let notifications = self.claude_notification_ids(id, &messages, &metadata)?;
        let carrier = messages
            .iter()
            .find(|m| {
                m.id == carrier_id
                    && !notifications.contains(&m.id)
                    && matches!(m.role, MessageRole::User | MessageRole::Tool)
            })
            .ok_or("Missing Claude notification carrier")?;
        let mut updates = Vec::new();
        let mut reminders = Vec::new();
        for message in messages.iter().filter(|m| notifications.contains(&m.id)) {
            let mut payload = metadata
                .get(&message.id)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let receipt = &payload[RECEIPT];
            if receipt["nativeId"].is_string() {
                continue;
            }
            if let Some(previous) = receipt["carrierMessageId"].as_str() {
                if previous != carrier_id && messages.iter().any(|m| m.id == previous) {
                    continue;
                }
            }
            let reminder = message.prompt_suffix.clone().unwrap_or_default();
            payload[RECEIPT] = json!({"version":1,"carrierMessageId":carrier_id});
            updates.push((message.id.clone(), payload));
            reminders.push(reminder);
        }
        if reminders.is_empty() {
            return Ok(reminders);
        }
        let mut content = if carrier.role == MessageRole::User {
            carrier.prompt_suffix.clone().unwrap_or_default()
        } else {
            carrier.content.clone()
        };
        for reminder in &reminders {
            if !content.contains(reminder) {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(reminder);
            }
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let changed = if carrier.role == MessageRole::User {
            tx.execute("UPDATE messages SET prompt_suffix=?1 WHERE session_id=?2 AND id=?3 AND prompt_suffix IS ?4", params![content,id,carrier_id,carrier.prompt_suffix])
        } else {
            tx.execute("UPDATE messages SET content=?1 WHERE session_id=?2 AND id=?3 AND content=?4", params![content,id,carrier_id,carrier.content])
        }.map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("Claude notification carrier changed; retry".into());
        }
        for (message, payload) in updates {
            let payload_id = Self::persist_response_request_with_conn(&tx, Some(&payload))?;
            tx.execute(
                "UPDATE messages SET response_request_id=?1 WHERE session_id=?2 AND id=?3",
                params![payload_id, id, message],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(reminders)
    }
}

#[cfg(test)]
#[path = "claude_notifications_tests.rs"]
mod tests;
