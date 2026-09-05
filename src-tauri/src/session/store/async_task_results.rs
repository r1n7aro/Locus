use super::*;
use crate::async_tasks::AsyncTaskSnapshot;

#[cfg(test)]
#[path = "async_task_results_tests.rs"]
mod tests;

impl SessionStore {
    pub(super) fn preserve_background_results(
        incoming: &mut [ToolCallInfo],
        existing: &[ToolCallInfo],
    ) {
        for call in incoming {
            let background = serde_json::from_str::<serde_json::Value>(&call.arguments)
                .ok()
                .and_then(|args| {
                    args.get("async")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|mode| matches!(mode.as_str(), "async" | "notify" | "async_notify"));
            if !background {
                continue;
            }
            if let Some(completed) = existing.iter().find(|old| {
                old.id == call.id && old.outcome.is_some() && old.recorded_output.is_some()
            }) {
                call.recorded_output = completed.recorded_output.clone();
                call.outcome = completed.outcome;
            }
        }
    }

    pub(super) fn create_async_task_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS async_task_results (
                task_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                snapshot_json TEXT NOT NULL,
                reminder TEXT,
                delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0, 1))
            );
            CREATE INDEX IF NOT EXISTS idx_async_task_delivery
                ON async_task_results(session_id, delivered);",
        )?;
        Self::create_async_notification_schema(conn)?;
        Self::create_agent_message_schema(conn)
    }

    fn create_async_notification_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS async_task_notifications (
            task_id TEXT NOT NULL REFERENCES async_task_results(task_id) ON DELETE CASCADE,
            attempt INTEGER NOT NULL,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            reminder TEXT NOT NULL,
            delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0, 1)),
            PRIMARY KEY(task_id, attempt)
        );
        CREATE INDEX IF NOT EXISTS idx_async_notification_delivery ON async_task_notifications(session_id, delivered);")
    }

    pub(super) fn migrate_async_task_attempts(conn: &Connection) -> rusqlite::Result<()> {
        Self::create_async_notification_schema(conn)?;
        Self::create_agent_message_schema(conn)?;
        conn.execute_batch("INSERT OR IGNORE INTO async_task_notifications(task_id, attempt, session_id, reminder, delivered)
            SELECT task_id, 1, session_id, reminder, delivered FROM async_task_results WHERE reminder IS NOT NULL;")?;
        let rows = {
            let mut stmt = conn
                .prepare("SELECT task_id, snapshot_json FROM async_task_results ORDER BY rowid")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut names: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
        for (_, json) in &rows {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            if let (Some(session), Some(name)) = (
                value["sessionId"].as_str(),
                value["localId"].as_str().filter(|name| !name.is_empty()),
            ) {
                names
                    .entry(session.to_string())
                    .or_default()
                    .insert(name.to_string());
            }
        }
        for (id, json) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            if let Some(obj) = value.as_object_mut() {
                let session = obj
                    .get("sessionId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if obj
                    .get("localId")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    let occupied = names.entry(session).or_default();
                    let name = (1_u64..)
                        .map(|n| format!("t{n}"))
                        .find(|name| !occupied.contains(name))
                        .unwrap();
                    occupied.insert(name.clone());
                    obj.insert("localId".into(), serde_json::json!(name));
                }
                let created_at = obj.get("createdAt").cloned().unwrap_or_default();
                obj.entry("attempt").or_insert(serde_json::json!(1));
                obj.entry("startedAt").or_insert(created_at);
                for key in ["resume", "assistantMessageId", "toolCallId"] {
                    obj.entry(key).or_insert(serde_json::Value::Null);
                }
            }
            conn.execute(
                "UPDATE async_task_results SET snapshot_json = ?1 WHERE task_id = ?2",
                params![value.to_string(), id],
            )?;
        }
        Ok(())
    }

    pub(crate) fn async_task_output_path(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<PathBuf, String> {
        let dir = self.session_tool_results_dir(session_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create task log directory: {e}"))?;
        Ok(dir.join(format!("{task_id}.log")))
    }

    pub(crate) fn save_async_task(
        &self,
        snapshot: &AsyncTaskSnapshot,
        reminder: Option<&str>,
    ) -> Result<(), String> {
        let json = serde_json::to_string(snapshot).map_err(|e| e.to_string())?;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO async_task_results(task_id, session_id, snapshot_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(task_id) DO UPDATE SET snapshot_json = excluded.snapshot_json",
            params![snapshot.task_id, snapshot.session_id, json],
        )
        .map_err(|e| format!("Failed to save async task: {e}"))?;
        if let Some(reminder) = reminder {
            tx.execute("INSERT OR IGNORE INTO async_task_notifications(task_id, attempt, session_id, reminder)
                VALUES(?1, ?2, ?3, ?4)", params![snapshot.task_id, snapshot.attempt, snapshot.session_id, reminder]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn load_async_task(
        &self,
        task_id: &str,
    ) -> Result<Option<AsyncTaskSnapshot>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let json: Option<String> = conn
            .query_row(
                "SELECT snapshot_json FROM async_task_results WHERE task_id = ?1",
                [task_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
            .transpose()
    }

    pub(crate) fn export_async_tasks(&self, session_id: &str) -> Result<serde_json::Value, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT snapshot_json FROM async_task_results WHERE session_id = ?1 ORDER BY rowid",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return Ok(serde_json::json!("empty"));
        }
        let mut values = rows
            .iter()
            .map(|json| serde_json::from_str::<serde_json::Value>(json).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        for value in &mut values {
            if let Some(obj) = value.as_object_mut() {
                for key in [
                    "resume",
                    "assistantMessageId",
                    "toolCallId",
                    "description",
                    "outputPath",
                    "finishedAt",
                    "progress",
                    "output",
                    "isError",
                ] {
                    if obj.get(key).is_none_or(serde_json::Value::is_null) {
                        obj.insert(key.into(), serde_json::json!("empty"));
                    }
                }
            }
        }
        Ok(serde_json::Value::Array(values))
    }

    pub(crate) fn list_async_tasks(
        &self,
        session_id: &str,
    ) -> Result<Vec<AsyncTaskSnapshot>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT snapshot_json FROM async_task_results WHERE session_id = ?1 ORDER BY rowid",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).map_err(|e| e.to_string()))
            .collect()
    }

    pub(crate) fn pending_async_notifications(
        &self,
        session_id: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT task_id, reminder FROM async_task_notifications
             WHERE session_id = ?1 AND delivered = 0 AND reminder IS NOT NULL ORDER BY rowid",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub(crate) fn async_notification_pending(
        &self,
        task_id: &str,
        attempt: u32,
    ) -> Result<bool, String> {
        self.conn.lock().map_err(|e| e.to_string())?.query_row(
            "SELECT EXISTS(SELECT 1 FROM async_task_notifications WHERE task_id = ?1 AND attempt = ?2 AND delivered = 0)",
            params![task_id, attempt], |r| r.get(0)).map_err(|e| e.to_string())
    }

    /// Append each completion once and acknowledge it in the same transaction.
    /// The old tool response remains an immutable record of the original launch.
    pub(crate) fn deliver_async_notifications(
        &self,
        session_id: &str,
    ) -> Result<Vec<String>, String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let pending = {
            let mut stmt = tx
                .prepare(
                    "SELECT task_id, attempt, reminder FROM async_task_notifications
                 WHERE session_id = ?1 AND delivered = 0 AND reminder IS NOT NULL ORDER BY rowid",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([session_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        let mut delivered = Vec::new();
        for (task_id, attempt, reminder) in pending {
            let message_id = if attempt == 1 {
                format!("async-result:{task_id}")
            } else {
                format!("async-result:{task_id}:{attempt}")
            };
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO messages(id, session_id, role, content, created_at, prompt_suffix)
                 VALUES (?1, ?2, 'user', '', ?3, ?4)",
                params![message_id, session_id, Self::now_ts(), reminder],
            ).map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE async_task_notifications SET delivered = 1 WHERE task_id = ?1 AND attempt = ?2",
                params![task_id, attempt],
            )
            .map_err(|e| e.to_string())?;
            if inserted != 0 {
                delivered.push(reminder);
            }
        }
        delivered.extend(Self::deliver_agent_messages(&tx, session_id)?);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(delivered)
    }

    /// Processes cannot survive a Locus restart. Keep their output reference and
    /// expose the interrupted result on the next user-driven run, without waking it.
    pub(crate) fn recover_async_tasks(&self) -> Result<(), String> {
        let snapshots = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn
                .prepare("SELECT snapshot_json FROM async_task_results")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        for json in snapshots {
            let mut snapshot: AsyncTaskSnapshot =
                serde_json::from_str(&json).map_err(|e| e.to_string())?;
            if snapshot.status.is_terminal() {
                continue;
            }
            snapshot.status = crate::async_tasks::AsyncTaskStatus::Cancelled;
            snapshot.is_error = Some(true);
            snapshot.finished_at = Some(Self::now_ts() * 1_000);
            snapshot.updated_at = snapshot.finished_at.unwrap();
            snapshot.output = Some(crate::async_tasks::prepare_final_output(
                &snapshot,
                "Task interrupted by Locus shutdown. The process was not resumed.",
            ));
            let reminder = snapshot
                .notify
                .then(|| crate::async_tasks::AsyncTaskManager::completion_reminder(&snapshot));
            self.save_async_task(&snapshot, reminder.as_deref())?;
        }
        Ok(())
    }
}
