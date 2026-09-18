use super::SessionStore;
use crate::sqlite_maint::GarbageCollectionResult;

impl SessionStore {
    pub(crate) fn garbage_collect(&self) -> Result<GarbageCollectionResult, String> {
        let conn = self.conn.try_lock().map_err(|_| {
            "Session database is busy; retry when current operations finish.".to_string()
        })?;
        crate::sqlite_maint::garbage_collect(&conn, &self.db_path)
    }

    pub(crate) fn database_path(&self) -> &std::path::Path {
        &self.db_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::context_export::export_session_context_yaml;
    use crate::session::models::MessageRole;

    #[test]
    fn garbage_collection_preserves_session_content_order_and_export() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let id = store
            .create_session("Keep this session", None, None, "chat", None)
            .unwrap();
        store
            .add_message(&id, MessageRole::User, "Keep the question")
            .unwrap();
        store
            .add_message(&id, MessageRole::Assistant, "Keep the answer")
            .unwrap();
        {
            let conn = store.conn.lock().unwrap();
            // Non-contiguous rowids are also used as message-page cursors.
            conn.execute_batch(
                "UPDATE messages SET rowid=rowid*10;
                CREATE TABLE gc_fixture (data BLOB);
                INSERT INTO gc_fixture VALUES(zeroblob(2097152));
                DELETE FROM gc_fixture;",
            )
            .unwrap();
        }
        let rows = || {
            let conn = store.conn.lock().unwrap();
            let mut statement = conn
                .prepare("SELECT rowid, id, role, content FROM messages ORDER BY rowid")
                .unwrap();
            statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let export = |filename: &str| {
            let path = dir.path().join(filename);
            export_session_context_yaml(&store, &id, "", None, None, &path).unwrap();
            serde_yaml::from_str::<serde_yaml::Value>(&std::fs::read_to_string(path).unwrap())
                .unwrap()
        };
        let before_rows = rows();
        let before_export = export("before.yaml");
        assert!(store.garbage_collect().unwrap().reclaimed_bytes > 0);
        assert_eq!(rows(), before_rows);
        let after_export = export("after.yaml");
        assert_eq!(before_export["sessions"], after_export["sessions"]);
        assert_eq!(
            after_export["sessions"][0]["context_attempts"].as_str(),
            Some("empty")
        );
        assert_eq!(store.load_session(&id).unwrap().messages.len(), 2);
    }
}
