use super::SessionStore;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

impl SessionStore {
    /// Logical stored bytes, excluding shared SQLite page/index/free-space overhead.
    /// Use a separate WAL reader: a large archive must never hold `self.conn`.
    pub fn archived_storage_bytes(&self, project_id: &str) -> Result<u64, String> {
        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare("SELECT id FROM sessions WHERE workspace_id = ?1 AND archived_at IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let session_ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if session_ids.is_empty() {
            return Ok(0);
        }

        // Read stored values, including compressed blobs, without loading/decompressing
        // conversations. Discover session-owned tables so newer persistence modules
        // (events, context attempts, async tasks, etc.) are counted as well.
        let mut tables = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .map_err(|e| e.to_string())?;
        let tables = tables
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut total = 0u64;
        for table in tables {
            let quoted_table = quote_identifier(&table);
            let mut columns = conn
                .prepare(&format!("PRAGMA table_info({quoted_table})"))
                .map_err(|e| e.to_string())?;
            let columns = columns
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let predicate = if table == "sessions" {
                "r.workspace_id = ?1 AND r.archived_at IS NOT NULL".to_string()
            } else if columns.iter().any(|c| c == "session_id") {
                "r.session_id IN (SELECT id FROM sessions WHERE workspace_id = ?1 AND archived_at IS NOT NULL)".to_string()
            } else if table == "response_request_payloads" {
                // DISTINCT membership counts deduplicated request payloads once.
                "r.id IN (SELECT m.response_request_id FROM messages m JOIN sessions s ON s.id = m.session_id WHERE s.workspace_id = ?1 AND s.archived_at IS NOT NULL)".to_string()
            } else {
                continue;
            };
            let sizes = columns.iter().map(|column| {
                let column = format!("r.{}", quote_identifier(column));
                format!("CASE typeof({column}) WHEN 'null' THEN 0 WHEN 'integer' THEN 8 WHEN 'real' THEN 8 ELSE length(CAST({column} AS BLOB)) END")
            }).collect::<Vec<_>>().join(" + ");
            let bytes: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COALESCE(SUM({sizes}), 0) FROM {quoted_table} r WHERE {predicate}"
                    ),
                    [project_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            total = total.saturating_add(bytes.max(0) as u64);
        }
        conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        for session_id in session_ids {
            total = total.saturating_add(directory_bytes(
                &self.session_tool_results_dir(&session_id),
            )?);
        }
        Ok(total)
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn directory_bytes(path: &Path) -> Result<u64, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        total = total.saturating_add(directory_bytes(&entry.map_err(|e| e.to_string())?.path())?);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::models::MessageRole;

    #[test]
    fn archive_storage_counts_utf8_and_shared_payloads_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let first = store
            .create_session("First", None, Some("project-a"), "chat", None)
            .unwrap();
        let second = store
            .create_session("Second", None, Some("project-a"), "chat", None)
            .unwrap();
        for id in [&first, &second] {
            store.add_message(id, MessageRole::User, "a").unwrap();
            store.archive_session(id).unwrap();
        }
        let before = store.archived_storage_bytes("project-a").unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE messages SET content = '中文' WHERE session_id = ?1",
                [&first],
            )
            .unwrap();
        }
        assert_eq!(
            store.archived_storage_bytes("project-a").unwrap(),
            before + 5
        );
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO response_request_payloads(id, payload_json) VALUES('shared', ?1)",
                ["x".repeat(1000)],
            )
            .unwrap();
            conn.execute("UPDATE messages SET response_request_id = 'shared'", [])
                .unwrap();
        }
        let before = store.archived_storage_bytes("project-a").unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE response_request_payloads SET payload_json = ?1 WHERE id = 'shared'",
                ["x".repeat(2000)],
            )
            .unwrap();
        }
        assert_eq!(
            store.archived_storage_bytes("project-a").unwrap(),
            before + 1000
        );
    }

    #[test]
    fn archive_storage_is_scoped_and_does_not_lock_session_loading() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let archived = store
            .create_session("Archived", None, Some("project-a"), "chat", None)
            .unwrap();
        let active = store
            .create_session("Active", None, Some("project-a"), "chat", None)
            .unwrap();
        store
            .add_message(&archived, MessageRole::User, "归档消息")
            .unwrap();
        store.archive_session(&archived).unwrap();
        let before = store.archived_storage_bytes("project-a").unwrap();
        assert!(before > 0);
        store
            .add_message(&active, MessageRole::User, &"active".repeat(1000))
            .unwrap();
        assert_eq!(store.archived_storage_bytes("project-a").unwrap(), before);
        assert_eq!(store.archived_storage_bytes("project-b").unwrap(), 0);
        let files = store.session_tool_results_dir(&archived);
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("result.txt"), b"1234567890").unwrap();
        // This would deadlock if statistics reused the foreground connection.
        let guard = store.conn.lock().unwrap();
        assert_eq!(
            store.archived_storage_bytes("project-a").unwrap(),
            before + 10
        );
        drop(guard);
        store.unarchive_session(&archived).unwrap();
        assert_eq!(store.archived_storage_bytes("project-a").unwrap(), 0);
    }
}
