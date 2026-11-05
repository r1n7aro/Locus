//! Tool definitions pinned per session. A contextual tool description (such as
//! the subagent list with its models) is recorded when the session first offers
//! the tool and reused afterwards, so later Agent setting changes do not alter
//! an existing conversation's tool prefix. Calls are validated against the
//! current settings instead.
use super::*;

#[cfg(test)]
#[path = "tool_contracts_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionToolContract {
    pub description: String,
    pub parameters: serde_json::Value,
}

impl SessionStore {
    pub(super) fn create_tool_contract_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_tool_contracts (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                tool_name TEXT NOT NULL,
                description TEXT NOT NULL,
                parameters_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, tool_name)
            );",
        )
    }

    pub(crate) fn session_tool_contract(
        &self,
        session_id: &str,
        tool_name: &str,
    ) -> Result<Option<SessionToolContract>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        Self::read_tool_contract(&conn, session_id, tool_name)
    }

    /// Keeps the first recorded contract, so concurrent runs of one session
    /// (a background clone, a resumed child) agree on the same definition.
    pub(crate) fn pin_session_tool_contract(
        &self,
        session_id: &str,
        tool_name: &str,
        contract: &SessionToolContract,
    ) -> Result<SessionToolContract, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let parameters = serde_json::to_string(&contract.parameters).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO session_tool_contracts
                (session_id, tool_name, description, parameters_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                tool_name,
                contract.description,
                parameters,
                Self::now_ts()
            ],
        )
        .map_err(|e| format!("Failed to pin session tool definition: {e}"))?;
        Self::read_tool_contract(&conn, session_id, tool_name)?
            .ok_or_else(|| "Pinned session tool definition is missing".to_string())
    }

    fn read_tool_contract(
        conn: &Connection,
        session_id: &str,
        tool_name: &str,
    ) -> Result<Option<SessionToolContract>, String> {
        let row = conn
            .query_row(
                "SELECT description, parameters_json FROM session_tool_contracts
                 WHERE session_id = ?1 AND tool_name = ?2",
                params![session_id, tool_name],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("Failed to load session tool definition: {e}"))?;
        row.map(|(description, parameters)| {
            Ok(SessionToolContract {
                description,
                parameters: serde_json::from_str(&parameters)
                    .map_err(|e| format!("Invalid pinned tool parameters: {e}"))?,
            })
        })
        .transpose()
    }

    /// Rows for a fork. Read through the source connection or snapshot.
    pub(super) fn tool_contract_rows(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<(String, String, String, i64)>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT tool_name, description, parameters_json, created_at
                 FROM session_tool_contracts WHERE session_id = ?1 ORDER BY tool_name",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }

    /// A fork continues the source conversation, including its tool prefix.
    pub(super) fn insert_tool_contract_rows(
        conn: &Connection,
        session_id: &str,
        rows: &[(String, String, String, i64)],
    ) -> Result<(), String> {
        for (tool_name, description, parameters, created_at) in rows {
            conn.execute(
                "INSERT OR IGNORE INTO session_tool_contracts
                    (session_id, tool_name, description, parameters_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, tool_name, description, parameters, created_at],
            )
            .map_err(|e| format!("Failed to copy session tool definitions: {e}"))?;
        }
        Ok(())
    }
}
