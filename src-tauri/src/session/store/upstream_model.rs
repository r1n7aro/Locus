use super::*;
use crate::llm::codex::server_model::from_saved_response;

impl SessionStore {
    /// v47 captured headers only, often persisting null despite a model in
    /// terminal event metadata. Re-key each repaired payload and its references
    /// in the migration transaction; repeating this pass is a no-op.
    pub(super) fn migrate_upstream_model_declarations(conn: &Connection) -> rusqlite::Result<()> {
        let rows = {
            let mut stmt =
                conn.prepare("SELECT id, payload_json FROM response_request_payloads")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (old_id, payload) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            if !value.is_object()
                || value
                    .get(crate::llm::upstream_model::METADATA_KEY)
                    .is_some()
            {
                continue;
            }
            let model = from_saved_response(&value["codex_response"]);
            if let Some(response) = value.get_mut("codex_response").filter(|v| v.is_object()) {
                response["server_model"] = serde_json::json!(model);
            }
            value[crate::llm::upstream_model::METADATA_KEY] = serde_json::json!(model);
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
        Ok(())
    }

    pub(super) fn migrate_upstream_model(conn: &Connection) -> rusqlite::Result<()> {
        let rows = {
            let mut stmt =
                conn.prepare("SELECT id, payload_json FROM response_request_payloads")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (old_id, payload) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let Some(response) = value.get_mut("codex_response").filter(|v| v.is_object()) else {
                continue;
            };
            if response.get("server_model").is_some() {
                continue;
            }
            response["server_model"] = serde_json::json!(from_saved_response(response));
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
        Ok(())
    }

    /// Inspect the latest assistant only: a missing model must not resurrect
    /// another response's model, including after switching providers.
    pub fn get_latest_upstream_model(&self, session_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let payload: Option<String> = conn
            .query_row(
                "SELECT r.payload_json FROM messages m
             LEFT JOIN response_request_payloads r ON r.id = m.response_request_id
             WHERE m.session_id = ?1 AND m.role = 'assistant'
             ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Failed to load upstream model: {error}"))?
            .flatten();
        let Some(payload) = payload else {
            return Ok(None);
        };
        let request: serde_json::Value = serde_json::from_str(&payload)
            .map_err(|error| format!("Failed to parse upstream model metadata: {error}"))?;
        Ok(crate::llm::upstream_model::from_request(&request))
    }
}

#[cfg(test)]
#[path = "upstream_model_tests.rs"]
mod tests;
