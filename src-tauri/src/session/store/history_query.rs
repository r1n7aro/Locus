//! Read-only, checkout-scoped history queries for the Python SDK.
use super::*;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchMatch {
    pub session_id: String,
    pub session_title: String,
    pub message_id: Option<String>,
    pub message_row_id: Option<i64>,
    pub role: Option<String>,
    pub field: String,
    pub excerpt: String,
    #[serde(skip)]
    pub default_checkout_id: Option<String>,
    #[serde(skip)]
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchPage {
    pub matches: Vec<SessionSearchMatch>,
    pub next_cursor: Option<String>,
    pub scanned_messages: u32,
    pub scanned_bytes: u64,
}

#[path = "history_search.rs"]
mod search;

impl SessionStore {
    fn require_history_session(
        conn: &Connection,
        checkout_id: &str,
        session_id: &str,
    ) -> Result<(), String> {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1 AND default_checkout_id = ?2)",
                params![session_id, checkout_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err(format!(
                "Session '{session_id}' not found in the selected checkout"
            ));
        }
        Ok(())
    }

    /// Uses the transcript's tool-round boundaries and chronological page order.
    /// Does not load the full session, turn index, events or request envelopes.
    pub fn read_session_history(
        &self,
        checkout_id: &str,
        session_id: &str,
        before_row_id: Option<i64>,
        limit: u32,
    ) -> Result<SessionMessagePage, String> {
        if !(1..=1_000).contains(&limit) || before_row_id.is_some_and(|id| id <= 0) {
            return Err("limit must be 1..1000 and beforeRowId must be positive".into());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        Self::require_history_session(&conn, checkout_id, session_id)?;
        let mut page = Self::get_message_page_with_conn(&conn, session_id, before_row_id, limit)?;
        drop(conn);
        for message in &mut page.messages {
            redact_context_handoff_for_display(message);
        }
        page.messages = normalize_messages_for_display(&page.messages);
        Ok(page)
    }
}

#[cfg(test)]
#[path = "history_query_tests.rs"]
mod tests;
