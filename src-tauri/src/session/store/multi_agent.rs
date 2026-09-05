use super::*;

#[cfg(test)]
#[path = "multi_agent_tests.rs"]
mod tests;

impl SessionStore {
    pub(super) fn migrate_multi_agent_selection(conn: &Connection) -> rusqlite::Result<()> {
        if !Self::table_has_column(conn, "sessions", "last_multi_agent_enabled")? {
            conn.execute_batch(
                "ALTER TABLE sessions ADD COLUMN last_multi_agent_enabled INTEGER
                 CHECK(last_multi_agent_enabled IN (0, 1));",
            )?;
        }
        Ok(())
    }

    pub(super) fn read_multi_agent_selection(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<bool>, String> {
        conn.query_row(
            "SELECT last_multi_agent_enabled FROM sessions WHERE id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read session multi agent selection: {}", e))
    }

    pub fn get_session_multi_agent_enabled(
        &self,
        session_id: &str,
    ) -> Result<Option<bool>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        Self::read_multi_agent_selection(&conn, session_id)
    }
}
