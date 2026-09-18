use super::*;

impl SessionStore {
    /// v46: absent reasoning-update history is explicit, never reconstructed
    /// from the session's current effort. Re-key shared payloads atomically.
    pub(super) fn migrate_codex_reasoning_updates(conn: &Connection) -> rusqlite::Result<()> {
        let rows = {
            let mut statement =
                conn.prepare("SELECT id, payload_json FROM response_request_payloads")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (old_id, payload) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let Some(object) = value.as_object_mut() else {
                continue;
            };
            if object.contains_key("codex_reasoning") {
                continue;
            }
            object.insert("codex_reasoning".to_string(), serde_json::Value::Null);
            let (new_id, payload) = response_request_payload(&value).map_err(|error| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
            })?;
            conn.execute(
                "INSERT OR IGNORE INTO response_request_payloads(id,payload_json) VALUES (?1,?2)",
                params![new_id, payload],
            )?;
            conn.execute(
                "UPDATE messages SET response_request_id=?1 WHERE response_request_id=?2",
                params![new_id, old_id],
            )?;
            conn.execute(
                "DELETE FROM response_request_payloads WHERE id=?1",
                params![old_id],
            )?;
        }
        // Preserve the synthesized prefix on upgrade as well as on subsequent
        // effort switches. This column stores the serialized cache identity.
        let caches = {
            let mut statement =
                conn.prepare("SELECT session_id, provider_key FROM session_prompt_prefix_cache")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (session, key) in caches {
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&key) else {
                continue;
            };
            if value["provider"]
                .as_str()
                .is_some_and(|p| p.starts_with("openai_codex:"))
                && crate::llm::codex::supports_reasoning_updates(
                    value["model"].as_str().unwrap_or_default(),
                )
            {
                value["effort"] = serde_json::Value::Null;
                conn.execute(
                    "UPDATE session_prompt_prefix_cache SET provider_key=?1 WHERE session_id=?2",
                    params![value.to_string(), session],
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "codex_reasoning_tests.rs"]
mod tests;
