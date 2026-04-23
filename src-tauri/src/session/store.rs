use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::models::{
    ChatMessage, KnowledgeProposal, KnowledgeProposalStatus, MessageRole, SessionDetail,
    SessionSummary, TodoItem, TodoSnapshot, ToolCallInfo,
};
use crate::commands::TokenUsage;

pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
    data_dir: PathBuf,
}

const TOOL_RESULTS_DIR: &str = "tool-results";
const TOOL_RESULT_PREVIEW_CHARS: usize = 2_000;
const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;
const LARGE_RESULT_TAG_OPEN: &str = "<persisted-output>";
const LARGE_RESULT_TAG_CLOSE: &str = "</persisted-output>";

#[derive(Debug, Clone)]
pub struct PersistedToolResult {
    pub filepath: PathBuf,
    pub original_size: usize,
    pub preview: String,
    pub has_more: bool,
}

fn is_large_result_reference(content: &str) -> bool {
    content.trim_start().starts_with(LARGE_RESULT_TAG_OPEN)
}

fn estimate_preview(content: &str, max_chars: usize) -> (String, bool) {
    let mut preview = String::new();
    let mut count = 0usize;
    let mut has_more = false;

    for ch in content.chars() {
        if count >= max_chars {
            has_more = true;
            break;
        }
        preview.push(ch);
        count += 1;
    }

    (preview, has_more)
}

fn tool_result_threshold(tool_name: &str) -> Option<usize> {
    match tool_name {
        // Read already self-bounds and persisting it introduces a circular
        // "read output -> file -> read again" pattern.
        "read" | "knowledge_read" => None,
        "bash" | "list" | "knowledge_list" | "knowledge_query" => Some(30_000),
        "grep" => Some(20_000),
        "webfetch" => Some(100_000),
        _ => Some(DEFAULT_MAX_RESULT_SIZE_CHARS),
    }
}

fn pick_result_extension(content: &str) -> &'static str {
    let trimmed = content.trim();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        "json"
    } else {
        "txt"
    }
}

pub fn build_large_tool_result_message(result: &PersistedToolResult) -> String {
    let mut message = String::new();
    message.push_str(LARGE_RESULT_TAG_OPEN);
    message.push('\n');
    message.push_str(&format!(
        "Output too large ({} chars). Full output saved to: {}\n",
        result.original_size,
        result.filepath.display()
    ));
    message.push_str("Use the Read tool with this exact path if you need the full output.\n\n");
    message.push_str(&format!(
        "Preview (first {} chars):\n",
        result.preview.chars().count()
    ));
    message.push_str(&result.preview);
    if result.has_more {
        message.push_str("\n...");
    }
    message.push('\n');
    message.push_str(LARGE_RESULT_TAG_CLOSE);
    message
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MessageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_proposal: Option<KnowledgeProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_request: Option<serde_json::Value>,
}

fn message_metadata_json(
    knowledge_proposal: Option<&KnowledgeProposal>,
    response_id: Option<&str>,
    response_request: Option<&serde_json::Value>,
) -> Result<Option<String>, String> {
    let metadata = MessageMetadata {
        knowledge_proposal: knowledge_proposal.cloned(),
        response_id: response_id.map(|value| value.to_string()),
        response_request: response_request.cloned(),
    };
    if metadata.knowledge_proposal.is_none()
        && metadata.response_id.is_none()
        && metadata.response_request.is_none()
    {
        return Ok(None);
    }
    serde_json::to_string(&metadata)
        .map(Some)
        .map_err(|e| format!("Failed to serialize message metadata: {}", e))
}

fn merge_prompt_prefixes(carried: &str, existing: Option<&str>) -> String {
    let carried_trimmed = carried.trim();
    if carried_trimmed.is_empty() {
        return existing.unwrap_or_default().to_string();
    }

    let existing_value = existing.unwrap_or_default();
    let existing_trimmed = existing_value.trim();
    if existing_trimmed.is_empty() {
        return carried_trimmed.to_string();
    }
    if existing_trimmed == carried_trimmed || existing_trimmed.starts_with(carried_trimmed) {
        return existing_value.to_string();
    }

    format!("{}\n\n{}", carried_trimmed, existing_trimmed)
}

fn strip_top_level_recorded_output(tool_calls: &[ToolCallInfo]) -> Vec<ToolCallInfo> {
    tool_calls
        .iter()
        .map(|tool_call| {
            let mut tool_call = tool_call.clone();
            tool_call.recorded_output = None;
            tool_call
        })
        .collect()
}

impl SessionStore {
    /// v7 is the oldest session schema we still support upgrading in place.
    /// Schemas below this baseline are treated as pre-release/unsupported and
    /// are reset on startup instead of migrated.
    const MIN_MIGRATABLE_SCHEMA_VERSION: i32 = 7;

    /// Current schema version for persisted session data.
    ///
    /// If you change any persisted conversation/message/todo/token schema at
    /// v7+, you must:
    /// 1. bump `SCHEMA_VERSION`
    /// 2. add an explicit migration block in `run_migrations`
    /// 3. keep existing sessions readable after upgrade
    ///
    /// Do not rely on ad-hoc `ALTER TABLE ... .ok()` fallbacks or silent
    /// schema drift. Session data must migrate deterministically.
    const SCHEMA_VERSION: i32 = 13;

    pub fn new(data_dir: &Path) -> Result<Self, String> {
        let db_path = data_dir.join("locus.db");

        // Schemas below the supported migration baseline are not upgraded
        // anymore. Drop them before opening so the app never mixes pre-v7
        // session data with the v7+ schema contract.
        if db_path.is_file() {
            if let Ok(probe) = Connection::open(&db_path) {
                let ver: i32 = probe
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .unwrap_or(0);
                drop(probe);
                if ver < Self::MIN_MIGRATABLE_SCHEMA_VERSION {
                    eprintln!(
                        "[Locus] session db version {} < minimum migratable {}, deleting for fresh start",
                        ver,
                        Self::MIN_MIGRATABLE_SCHEMA_VERSION
                    );
                    let _ = std::fs::remove_file(&db_path);
                    // Also remove WAL/SHM leftovers
                    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
                    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
                }
            }
        }

        let conn =
            Connection::open(&db_path).map_err(|e| format!("Failed to open database: {}", e))?;

        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| format!("Failed to enable foreign keys: {}", e))?;

        Self::run_migrations(&conn)?;

        Ok(SessionStore {
            conn: Arc::new(Mutex::new(conn)),
            data_dir: data_dir.to_path_buf(),
        })
    }

    /// Fresh databases are created directly at the latest schema version.
    /// Supported upgrades start at v7, and every schema change after that must
    /// be expressed as an explicit migration keyed by `user_version`.
    fn run_migrations(conn: &Connection) -> Result<(), String> {
        let current: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| format!("Failed to read schema version: {}", e))?;

        if current > Self::SCHEMA_VERSION {
            return Err(format!(
                "Database schema version {} is newer than supported version {}. \
                 Please upgrade the application.",
                current,
                Self::SCHEMA_VERSION
            ));
        }

        if current == 0 {
            Self::migrate(conn, Self::SCHEMA_VERSION, "create latest schema", |conn| {
                Self::create_latest_schema(conn)
            })?;
            return Ok(());
        }

        if current < Self::MIN_MIGRATABLE_SCHEMA_VERSION {
            return Err(format!(
                "Database schema version {} is below minimum migratable version {}. \
                 Delete the session database and restart.",
                current,
                Self::MIN_MIGRATABLE_SCHEMA_VERSION
            ));
        }

        if current < 8 {
            Self::migrate(conn, 8, "add archived_at to sessions", |conn| {
                if !Self::table_has_column(conn, "sessions", "archived_at")? {
                    conn.execute_batch("ALTER TABLE sessions ADD COLUMN archived_at INTEGER;")?;
                }
                Ok(())
            })?;
        }

        if current < 9 {
            Self::migrate(
                conn,
                9,
                "add prompt_prefix and prompt_suffix to messages",
                |conn| {
                    if !Self::table_has_column(conn, "messages", "prompt_prefix")? {
                        conn.execute_batch("ALTER TABLE messages ADD COLUMN prompt_prefix TEXT;")?;
                    }
                    if !Self::table_has_column(conn, "messages", "prompt_suffix")? {
                        conn.execute_batch("ALTER TABLE messages ADD COLUMN prompt_suffix TEXT;")?;
                    }
                    Ok(())
                },
            )?;
        }

        if current < 10 {
            Self::migrate(conn, 10, "add include_in_prompt to messages", |conn| {
                if !Self::table_has_column(conn, "messages", "include_in_prompt")? {
                    conn.execute_batch(
                            "ALTER TABLE messages ADD COLUMN include_in_prompt INTEGER NOT NULL DEFAULT 1;
                             UPDATE messages SET include_in_prompt = 1 WHERE include_in_prompt IS NULL;",
                        )?;
                }
                Ok(())
            })?;
        }

        if current < 11 {
            Self::migrate(
                conn,
                11,
                "add latest_completed_run_id to sessions",
                |conn| {
                    if !Self::table_has_column(conn, "sessions", "latest_completed_run_id")? {
                        conn.execute_batch(
                            "ALTER TABLE sessions ADD COLUMN latest_completed_run_id TEXT;",
                        )?;
                    }
                    Ok(())
                },
            )?;
        }

        if current < 12 {
            Self::migrate(
                conn,
                12,
                "canonicalize persisted tool call payloads",
                |conn| Self::migrate_tool_call_payloads(conn),
            )?;
        }

        if current < 13 {
            Self::migrate(conn, 13, "add latest_todo_run_id to sessions", |conn| {
                if !Self::table_has_column(conn, "sessions", "latest_todo_run_id")? {
                    conn.execute_batch("ALTER TABLE sessions ADD COLUMN latest_todo_run_id TEXT;")?;
                }
                Ok(())
            })?;
        }

        debug_assert_eq!(Self::SCHEMA_VERSION, 13, "add a new migration block above");
        Ok(())
    }

    fn create_latest_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                latest_completed_run_id TEXT,
                latest_todo_run_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON sessions(workspace_id);

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT,
                include_in_prompt INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);

            CREATE TABLE IF NOT EXISTS token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
            );
            CREATE INDEX IF NOT EXISTS idx_todos_session ON todos(session_id);",
        )
    }

    fn table_has_column(conn: &Connection, table: &str, col: &str) -> rusqlite::Result<bool> {
        let sql = format!("PRAGMA table_info({})", table);
        let mut stmt = conn.prepare(&sql)?;
        let found = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .any(|r| r.map(|name| name == col).unwrap_or(false));
        Ok(found)
    }

    /// Run a single migration step inside a transaction, setting user_version on success.
    fn migrate<F>(conn: &Connection, to_version: i32, label: &str, f: F) -> Result<(), String>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<()>,
    {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| {
            format!(
                "migration v{} ({}): failed to begin transaction: {}",
                to_version, label, e
            )
        })?;

        if let Err(e) = f(conn) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(format!(
                "migration v{} ({}) failed: {}",
                to_version, label, e
            ));
        }

        conn.pragma_update(None, "user_version", to_version)
            .map_err(|e| {
                let _ = conn.execute_batch("ROLLBACK");
                format!(
                    "migration v{} ({}): failed to update schema version: {}",
                    to_version, label, e
                )
            })?;

        conn.execute_batch("COMMIT").map_err(|e| {
            format!(
                "migration v{} ({}): failed to commit: {}",
                to_version, label, e
            )
        })?;

        Ok(())
    }

    fn migrate_tool_call_payloads(conn: &Connection) -> rusqlite::Result<()> {
        let mut stmt = conn.prepare("SELECT id FROM sessions ORDER BY created_at ASC, id ASC")?;
        let session_ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        for session_id in session_ids {
            let raw_messages =
                Self::get_messages_with_conn_filtered_static(conn, &session_id, false).map_err(
                    |error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            error,
                        )))
                    },
                )?;
            let normalized = crate::session::history::normalize_tool_round_history(&raw_messages);

            for message in normalized {
                let Some(tool_calls) = message.tool_calls.as_ref() else {
                    continue;
                };
                if message.role != MessageRole::Assistant || tool_calls.is_empty() {
                    continue;
                }

                let serialized = serde_json::to_string(&strip_top_level_recorded_output(
                    tool_calls,
                ))
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                conn.execute(
                    "UPDATE messages SET tool_calls = ?1 WHERE id = ?2",
                    params![serialized, message.id],
                )?;
            }
        }

        Ok(())
    }

    fn now_ts() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    pub fn create_session(
        &self,
        title: &str,
        parent_id: Option<&str>,
        workspace_id: Option<&str>,
        session_type: &str,
        agent_id: Option<&str>,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, title, parent_id, workspace_id, session_type, agent_id, now, now],
        )
        .map_err(|e| format!("Failed to create session: {}", e))?;
        Ok(id)
    }

    pub fn list_sessions(&self, workspace_id: Option<&str>) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_by_archive_state(workspace_id, false)
    }

    pub fn list_archived_sessions(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_by_archive_state(workspace_id, true)
    }

    fn list_sessions_by_archive_state(
        &self,
        workspace_id: Option<&str>,
        archived: bool,
    ) -> Result<Vec<SessionSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let row_mapper = |row: &rusqlite::Row| -> rusqlite::Result<SessionSummary> {
            Ok(SessionSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                agent_id: row.get(2)?,
                session_type: row.get(3)?,
                parent_session_id: row.get(4)?,
                updated_at: row.get(5)?,
                runtime_status: None,
            })
        };

        let mut sessions = Vec::new();
        match workspace_id {
            Some(wid) => {
                let mut stmt = conn
                    .prepare(
                        if archived {
                            "SELECT id, title, agent_id, session_type, parent_session_id, updated_at FROM sessions WHERE workspace_id = ?1 AND archived_at IS NOT NULL ORDER BY archived_at DESC, updated_at DESC"
                        } else {
                            "SELECT id, title, agent_id, session_type, parent_session_id, updated_at FROM sessions WHERE workspace_id = ?1 AND archived_at IS NULL ORDER BY updated_at DESC"
                        },
                    )
                    .map_err(|e| format!("Failed to prepare query: {}", e))?;
                let rows = stmt
                    .query_map(params![wid], row_mapper)
                    .map_err(|e| format!("Failed to query sessions: {}", e))?;
                for row in rows {
                    sessions.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        if archived {
                            "SELECT id, title, agent_id, session_type, parent_session_id, updated_at FROM sessions WHERE workspace_id IS NULL AND archived_at IS NOT NULL ORDER BY archived_at DESC, updated_at DESC"
                        } else {
                            "SELECT id, title, agent_id, session_type, parent_session_id, updated_at FROM sessions WHERE workspace_id IS NULL AND archived_at IS NULL ORDER BY updated_at DESC"
                        },
                    )
                    .map_err(|e| format!("Failed to prepare query: {}", e))?;
                let rows = stmt
                    .query_map([], row_mapper)
                    .map_err(|e| format!("Failed to query sessions: {}", e))?;
                for row in rows {
                    sessions.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
                }
            }
        }
        Ok(sessions)
    }

    pub fn load_session(&self, id: &str) -> Result<SessionDetail, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let (
            title,
            agent_id,
            session_type,
            parent_session_id,
            latest_completed_run_id,
            created_at,
            updated_at,
        ) = conn
            .query_row(
                "SELECT title, agent_id, session_type, parent_session_id, latest_completed_run_id, created_at, updated_at FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .map_err(|e| format!("Session not found: {}", e))?;

        let raw_messages = self.get_messages_with_conn(&conn, id)?;
        let messages = crate::session::history::normalize_tool_round_history(&raw_messages);

        Ok(SessionDetail {
            id: id.to_string(),
            title,
            agent_id,
            session_type,
            parent_session_id,
            latest_completed_run_id,
            created_at,
            updated_at,
            messages,
        })
    }

    pub fn set_latest_completed_run_id(
        &self,
        session_id: &str,
        run_id: Option<&str>,
    ) -> Result<(), String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET latest_completed_run_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![run_id, now, session_id],
        )
        .map_err(|e| format!("Failed to update latest completed run id: {}", e))?;
        Ok(())
    }

    pub fn get_session_agent_id(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT agent_id FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| format!("Session not found: {}", e))
    }

    pub fn get_session_title(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        match conn.query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(title) => Ok(Some(title)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Failed to load session title: {}", e)),
        }
    }

    pub fn get_session_workspace_id(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        match conn.query_row(
            "SELECT workspace_id FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(workspace_id) => Ok(workspace_id),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Failed to load session workspace id: {}", e)),
        }
    }

    pub fn rename_session(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, id],
        )
        .map_err(|e| format!("Failed to rename session: {}", e))?;
        Ok(())
    }

    pub fn archive_session(&self, id: &str) -> Result<(), String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET archived_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(|e| format!("Failed to archive session: {}", e))?;
        Ok(())
    }

    pub fn unarchive_session(&self, id: &str) -> Result<(), String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET archived_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(|e| format!("Failed to unarchive session: {}", e))?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete session: {}", e))?;
        let tool_dir = self.session_tool_results_dir(id);
        if tool_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&tool_dir);
        }
        Ok(())
    }

    pub fn truncate_from_message(&self, session_id: &str, message_id: &str) -> Result<u64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let message_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM messages WHERE id = ?1 AND session_id = ?2",
                params![message_id, session_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Message not found: {}", e))?;

        // Use rowid boundaries so same-second messages are not collapsed into a
        // single truncation point.
        let truncate_from_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM messages WHERE session_id = ?1 AND role = 'user' AND rowid < ?2 ORDER BY rowid DESC LIMIT 1",
                params![session_id, message_rowid],
                |row| row.get(0),
            )
            .unwrap_or(message_rowid);
        let deleted = conn
            .execute(
                "DELETE FROM messages WHERE session_id = ?1 AND rowid >= ?2",
                params![session_id, truncate_from_rowid],
            )
            .map_err(|e| format!("Failed to truncate messages: {}", e))?;

        Ok(deleted as u64)
    }

    pub fn add_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
    ) -> Result<String, String> {
        self.add_message_full(session_id, role, content, None, None, None, None, None)
    }

    pub fn add_message_with_images(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        images: Option<&[super::models::ImageData]>,
    ) -> Result<String, String> {
        let images_json = images
            .filter(|imgs| !imgs.is_empty())
            .map(|imgs| serde_json::to_string(imgs))
            .transpose()
            .map_err(|e| format!("Failed to serialize images: {}", e))?;
        self.add_message_full(
            session_id,
            role,
            content,
            None,
            None,
            images_json.as_deref(),
            None,
            None,
        )
    }

    pub fn add_message_with_images_and_signature(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        images: Option<&[super::models::ImageData]>,
        thinking_signature: Option<&str>,
        prompt_prefix: Option<&str>,
        prompt_suffix: Option<&str>,
    ) -> Result<String, String> {
        let images_json = images
            .filter(|imgs| !imgs.is_empty())
            .map(|imgs| serde_json::to_string(imgs))
            .transpose()
            .map_err(|e| format!("Failed to serialize images: {}", e))?;
        self.add_message_full_with_thinking(
            session_id,
            role,
            content,
            None,
            None,
            images_json.as_deref(),
            None,
            None,
            thinking_signature,
            None,
            prompt_prefix,
            prompt_suffix,
            None,
            None,
        )
    }

    pub fn add_message_with_thinking(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
    ) -> Result<String, String> {
        self.add_message_full_with_thinking(
            session_id,
            role,
            content,
            None,
            None,
            None,
            thinking_content,
            thinking_duration,
            thinking_signature,
            None,
            None,
            None,
            response_id,
            response_request,
        )
    }

    #[allow(dead_code)]
    pub fn add_assistant_with_tool_calls(
        &self,
        session_id: &str,
        content: &str,
        tool_calls: &[ToolCallInfo],
    ) -> Result<String, String> {
        self.add_assistant_with_tool_calls_and_thinking(
            session_id, content, tool_calls, None, None, None, None, None,
        )
    }

    pub fn add_assistant_with_tool_calls_and_thinking(
        &self,
        session_id: &str,
        content: &str,
        tool_calls: &[ToolCallInfo],
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
    ) -> Result<String, String> {
        let tool_calls_json = serde_json::to_string(tool_calls)
            .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
        self.add_message_full_with_thinking(
            session_id,
            MessageRole::Assistant,
            content,
            Some(&tool_calls_json),
            None,
            None,
            thinking_content,
            thinking_duration,
            thinking_signature,
            None,
            None,
            None,
            response_id,
            response_request,
        )
    }

    pub fn update_message_tool_calls(
        &self,
        message_id: &str,
        tool_calls: &[ToolCallInfo],
    ) -> Result<(), String> {
        let tool_calls_json = serde_json::to_string(tool_calls)
            .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET tool_calls = ?1 WHERE id = ?2",
            params![tool_calls_json, message_id],
        )
        .map_err(|e| {
            format!(
                "Failed to update tool_calls for message '{}': {}",
                message_id, e
            )
        })?;
        Ok(())
    }

    pub fn add_tool_result(
        &self,
        session_id: &str,
        tool_call_id: &str,
        content: &str,
    ) -> Result<String, String> {
        self.add_message_full(
            session_id,
            MessageRole::Tool,
            content,
            None,
            Some(tool_call_id),
            None,
            None,
            None,
        )
    }

    fn session_tool_results_dir(&self, session_id: &str) -> PathBuf {
        self.data_dir.join(TOOL_RESULTS_DIR).join(session_id)
    }

    pub fn rewrite_tool_result_for_storage(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> Result<String, String> {
        if content.is_empty()
            || tool_call_id.is_empty()
            || is_large_result_reference(content)
            || content == crate::compact::CLEARED_TOOL_RESULT
        {
            return Ok(content.to_string());
        }

        let Some(threshold) = tool_result_threshold(tool_name) else {
            return Ok(content.to_string());
        };

        let char_count = content.chars().count();
        if char_count <= threshold {
            return Ok(content.to_string());
        }

        let dir = self.session_tool_results_dir(session_id);
        std::fs::create_dir_all(&dir).map_err(|e| {
            format!(
                "Failed to create tool result dir '{}': {}",
                dir.display(),
                e
            )
        })?;

        let path = dir.join(format!(
            "{}.{}",
            tool_call_id,
            pick_result_extension(content)
        ));
        std::fs::write(&path, content).map_err(|e| {
            format!(
                "Failed to persist tool result to '{}': {}",
                path.display(),
                e
            )
        })?;

        let (preview, has_more) = estimate_preview(content, TOOL_RESULT_PREVIEW_CHARS);
        Ok(build_large_tool_result_message(&PersistedToolResult {
            filepath: path,
            original_size: char_count,
            preview,
            has_more,
        }))
    }

    fn add_message_full(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        tool_calls_json: Option<&str>,
        tool_call_id: Option<&str>,
        images_json: Option<&str>,
        prompt_prefix: Option<&str>,
        knowledge_proposal: Option<&KnowledgeProposal>,
    ) -> Result<String, String> {
        self.add_message_full_with_thinking(
            session_id,
            role,
            content,
            tool_calls_json,
            tool_call_id,
            images_json,
            None,
            None,
            None,
            knowledge_proposal,
            prompt_prefix,
            None,
            None,
            None,
        )
    }

    fn add_message_full_with_thinking(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        tool_calls_json: Option<&str>,
        tool_call_id: Option<&str>,
        images_json: Option<&str>,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        knowledge_proposal: Option<&KnowledgeProposal>,
        prompt_prefix: Option<&str>,
        prompt_suffix: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now_ts();
        let metadata_json =
            message_metadata_json(knowledge_proposal, response_id, response_request)?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![id, session_id, role.as_str(), content, now, prompt_prefix, prompt_suffix, tool_calls_json, tool_call_id, images_json, thinking_content, thinking_duration.map(|d| d as i64), thinking_signature, metadata_json],
        )
        .map_err(|e| format!("Failed to add message: {}", e))?;

        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| format!("Failed to update session: {}", e))?;

        Ok(id)
    }

    pub fn add_knowledge_proposal_message(
        &self,
        session_id: &str,
        proposal: &KnowledgeProposal,
    ) -> Result<String, String> {
        self.add_message_full(
            session_id,
            MessageRole::Assistant,
            "",
            None,
            None,
            None,
            None,
            Some(proposal),
        )
    }

    pub fn get_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_messages_with_conn_filtered(&conn, session_id, false)
    }

    pub fn get_messages_for_prompt(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.get_messages_with_conn_filtered(&conn, session_id, true)
    }

    pub fn get_response_request_metadata(
        &self,
        session_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, metadata_json FROM messages
                 WHERE session_id = ?1 AND metadata_json IS NOT NULL
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|e| format!("Failed to prepare response request query: {}", e))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| format!("Failed to query response request metadata: {}", e))?;

        let mut metadata_map = HashMap::new();
        for row in rows {
            let (message_id, metadata_json) =
                row.map_err(|e| format!("Failed to read response request row: {}", e))?;
            let Some(metadata_json) = metadata_json else {
                continue;
            };
            let metadata: MessageMetadata = serde_json::from_str(&metadata_json)
                .map_err(|e| format!("Failed to parse response request metadata: {}", e))?;
            if let Some(response_request) = metadata.response_request {
                metadata_map.insert(message_id, response_request);
            }
        }

        Ok(metadata_map)
    }

    pub fn first_user_message_id(&self, session_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id FROM messages
             WHERE session_id = ?1 AND include_in_prompt = 1 AND role = 'user' AND tool_call_id IS NULL
             ORDER BY created_at ASC, rowid ASC
             LIMIT 1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("Failed to query first user message: {}", e))
    }

    pub fn update_message_prompt_prefix(
        &self,
        session_id: &str,
        message_id: &str,
        prompt_prefix: Option<&str>,
    ) -> Result<(), String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE messages SET prompt_prefix = ?1 WHERE id = ?2 AND session_id = ?3",
            params![prompt_prefix, message_id, session_id],
        )
        .map_err(|e| format!("Failed to update message prompt prefix: {}", e))?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| format!("Failed to update session timestamp: {}", e))?;
        Ok(())
    }

    pub fn record_token_usage(
        &self,
        session_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        cost_usd: f64,
        priced_rounds: u64,
    ) -> Result<TokenUsage, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO token_usage (session_id, total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_write_tokens, total_cost_usd, priced_rounds)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(session_id) DO UPDATE SET
                total_input_tokens = total_input_tokens + ?2,
                total_output_tokens = total_output_tokens + ?3,
                total_cache_read_tokens = total_cache_read_tokens + ?4,
                total_cache_write_tokens = total_cache_write_tokens + ?5,
                total_cost_usd = total_cost_usd + ?6,
                priced_rounds = priced_rounds + ?7",
            params![
                session_id,
                input_tokens as i64,
                output_tokens as i64,
                cache_read_tokens as i64,
                cache_write_tokens as i64,
                cost_usd,
                priced_rounds as i64
            ],
        )
        .map_err(|e| format!("Failed to record token usage: {}", e))?;

        let (total_in, total_out, total_cr, total_cw, total_cost_usd, priced_rounds) = conn
            .query_row(
                "SELECT total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_write_tokens, total_cost_usd, priced_rounds FROM token_usage WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .map_err(|e| format!("Failed to read token usage: {}", e))?;

        Ok(TokenUsage {
            total_input_tokens: total_in as u64,
            total_output_tokens: total_out as u64,
            total_cache_read_tokens: total_cr as u64,
            total_cache_write_tokens: total_cw as u64,
            total_cost_usd,
            priced_rounds: priced_rounds as u64,
        })
    }

    pub fn get_token_usage(&self, session_id: &str) -> Result<TokenUsage, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let result = conn.query_row(
            "SELECT total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_write_tokens, total_cost_usd, priced_rounds FROM token_usage WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        );

        match result {
            Ok((total_in, total_out, total_cr, total_cw, total_cost_usd, priced_rounds)) => {
                Ok(TokenUsage {
                    total_input_tokens: total_in as u64,
                    total_output_tokens: total_out as u64,
                    total_cache_read_tokens: total_cr as u64,
                    total_cache_write_tokens: total_cw as u64,
                    total_cost_usd,
                    priced_rounds: priced_rounds as u64,
                })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(TokenUsage {
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_write_tokens: 0,
                total_cost_usd: 0.0,
                priced_rounds: 0,
            }),
            Err(e) => Err(format!("Failed to get token usage: {}", e)),
        }
    }

    pub fn update_todos(
        &self,
        session_id: &str,
        latest_run_id: Option<&str>,
        todos: &[TodoItem],
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute("BEGIN", [])
            .map_err(|e| format!("Failed to begin transaction: {}", e))?;

        conn.execute(
            "DELETE FROM todos WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to delete old todos: {}", e)
        })?;

        for (position, todo) in todos.iter().enumerate() {
            conn.execute(
                "INSERT INTO todos (session_id, position, content, status, priority) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, position as i64, todo.content, todo.status, todo.priority],
            )
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                format!("Failed to insert todo: {}", e)
            })?;
        }

        conn.execute(
            "UPDATE sessions SET latest_todo_run_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![latest_run_id, Self::now_ts(), session_id],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to update todo run boundary: {}", e)
        })?;

        conn.execute("COMMIT", [])
            .map_err(|e| format!("Failed to commit transaction: {}", e))?;

        Ok(())
    }

    pub fn get_todos(&self, session_id: &str) -> Result<TodoSnapshot, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT content, status, priority FROM todos WHERE session_id = ?1 ORDER BY position ASC")
            .map_err(|e| format!("Failed to prepare todos query: {}", e))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(TodoItem {
                    content: row.get(0)?,
                    status: row.get(1)?,
                    priority: row.get(2)?,
                })
            })
            .map_err(|e| format!("Failed to query todos: {}", e))?;

        let mut todos = Vec::new();
        for row in rows {
            todos.push(row.map_err(|e| format!("Failed to read todo row: {}", e))?);
        }
        let latest_run_id = conn
            .query_row(
                "SELECT latest_todo_run_id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| format!("Failed to query todo run boundary: {}", e))?
            .flatten();
        Ok(TodoSnapshot {
            items: todos,
            latest_run_id,
        })
    }

    pub fn compact_messages(
        &self,
        session_id: &str,
        summary_msg: &ChatMessage,
        keep_from_message_id: &str,
    ) -> Result<(u32, u32), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let count_before: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND include_in_prompt = 1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count messages: {}", e))?;

        conn.execute("BEGIN", [])
            .map_err(|e| format!("Failed to begin transaction: {}", e))?;

        let keep_from_rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM messages WHERE session_id = ?1 AND id = ?2",
                params![session_id, keep_from_message_id],
                |row| row.get(0),
            )
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                format!("Failed to resolve compact boundary: {}", e)
            })?;

        let carried_prompt_prefix = conn
            .query_row(
                "SELECT rowid, prompt_prefix FROM messages
                 WHERE session_id = ?1
                   AND include_in_prompt = 1
                   AND role = 'user'
                   AND tool_call_id IS NULL
                   AND prompt_prefix IS NOT NULL
                   AND trim(prompt_prefix) != ''
                 ORDER BY created_at ASC, rowid ASC
                 LIMIT 1",
                params![session_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                format!("Failed to load carried prompt prefix: {}", e)
            })?
            .filter(|(rowid, _)| *rowid < keep_from_rowid)
            .map(|(_, prefix)| prefix);

        conn.execute(
            "UPDATE messages
             SET include_in_prompt = 0
             WHERE session_id = ?1 AND include_in_prompt = 1 AND rowid < ?2",
            params![session_id, keep_from_rowid],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to mark compacted messages: {}", e)
        })?;

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            params![
                summary_msg.id,
                session_id,
                summary_msg.role.as_str(),
                summary_msg.content,
                summary_msg.created_at
            ],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to insert summary message: {}", e)
        })?;

        if let Some(carried_prefix) = carried_prompt_prefix.as_deref() {
            let target_message = conn
                .query_row(
                    "SELECT id, prompt_prefix FROM messages
                     WHERE session_id = ?1
                       AND include_in_prompt = 1
                       AND role = 'user'
                       AND tool_call_id IS NULL
                     ORDER BY created_at ASC, rowid ASC
                     LIMIT 1",
                    params![session_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .map_err(|e| {
                    let _ = conn.execute("ROLLBACK", []);
                    format!("Failed to resolve prompt-prefix target: {}", e)
                })?;

            if let Some((target_id, existing_prefix)) = target_message {
                let merged_prefix =
                    merge_prompt_prefixes(carried_prefix, existing_prefix.as_deref());
                conn.execute(
                    "UPDATE messages SET prompt_prefix = ?1 WHERE id = ?2 AND session_id = ?3",
                    params![merged_prefix, target_id, session_id],
                )
                .map_err(|e| {
                    let _ = conn.execute("ROLLBACK", []);
                    format!("Failed to carry prompt prefix across compact: {}", e)
                })?;
            } else {
                let merged_prefix = merge_prompt_prefixes(carried_prefix, None);
                conn.execute(
                    "UPDATE messages SET prompt_prefix = ?1 WHERE id = ?2 AND session_id = ?3",
                    params![merged_prefix, summary_msg.id, session_id],
                )
                .map_err(|e| {
                    let _ = conn.execute("ROLLBACK", []);
                    format!("Failed to attach carried prompt prefix to handoff: {}", e)
                })?;
            }
        }

        conn.execute("COMMIT", [])
            .map_err(|e| format!("Failed to commit compact transaction: {}", e))?;

        let count_after: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND include_in_prompt = 1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to count messages after compact: {}", e))?;

        Ok((count_before, count_after))
    }

    fn get_messages_with_conn(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<ChatMessage>, String> {
        Self::get_messages_with_conn_filtered_static(conn, session_id, false)
    }

    fn get_messages_with_conn_filtered(
        &self,
        conn: &Connection,
        session_id: &str,
        prompt_only: bool,
    ) -> Result<Vec<ChatMessage>, String> {
        Self::get_messages_with_conn_filtered_static(conn, session_id, prompt_only)
    }

    fn get_messages_with_conn_filtered_static(
        conn: &Connection,
        session_id: &str,
        prompt_only: bool,
    ) -> Result<Vec<ChatMessage>, String> {
        let query = if prompt_only {
            "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json
             FROM messages
             WHERE session_id = ?1 AND include_in_prompt = 1
             ORDER BY created_at ASC, rowid ASC"
        } else {
            "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json
             FROM messages
             WHERE session_id = ?1
             ORDER BY created_at ASC, rowid ASC"
        };

        let mut stmt = conn
            .prepare(query)
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            })
            .map_err(|e| format!("Failed to query messages: {}", e))?;

        let mut messages = Vec::new();
        for row in rows {
            let (
                id,
                role_str,
                content,
                created_at,
                prompt_prefix,
                prompt_suffix,
                tool_calls_json,
                tool_call_id,
                images_json,
                thinking_content,
                thinking_duration_raw,
                thinking_signature,
                metadata_json,
            ) = row.map_err(|e| format!("Failed to read row: {}", e))?;
            let role = MessageRole::from_str(&role_str)?;

            let tool_calls: Option<Vec<ToolCallInfo>> = tool_calls_json
                .as_deref()
                .map(|json| serde_json::from_str(json))
                .transpose()
                .map_err(|e| format!("Failed to parse tool_calls: {}", e))?;

            let images: Option<Vec<super::models::ImageData>> = images_json
                .as_deref()
                .map(|json| serde_json::from_str(json))
                .transpose()
                .map_err(|e| format!("Failed to parse images: {}", e))?;

            let metadata: Option<MessageMetadata> = metadata_json
                .as_deref()
                .map(|json| serde_json::from_str(json))
                .transpose()
                .map_err(|e| format!("Failed to parse message metadata: {}", e))?;
            let (knowledge_proposal, response_id) = metadata
                .map(|value| (value.knowledge_proposal, value.response_id))
                .unwrap_or((None, None));

            messages.push(ChatMessage {
                id,
                role,
                content,
                created_at,
                prompt_prefix,
                prompt_suffix,
                response_id,
                tool_calls,
                tool_call_id,
                images,
                thinking_content,
                thinking_duration: thinking_duration_raw.map(|d| d as u32),
                thinking_signature,
                knowledge_proposal,
            });
        }
        Ok(messages)
    }

    pub fn get_knowledge_proposal_message(
        &self,
        session_id: &str,
        proposal_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        let messages = self.get_messages(session_id)?;
        Ok(messages.into_iter().find(|message| {
            message
                .knowledge_proposal
                .as_ref()
                .map(|proposal| proposal.proposal_id == proposal_id)
                .unwrap_or(false)
        }))
    }

    pub fn stale_pending_knowledge_proposals(
        &self,
        session_id: &str,
    ) -> Result<Vec<ChatMessage>, String> {
        self.update_pending_knowledge_proposals(session_id, KnowledgeProposalStatus::Stale)
    }

    pub fn update_knowledge_proposal_status(
        &self,
        session_id: &str,
        proposal_id: &str,
        status: KnowledgeProposalStatus,
    ) -> Result<Option<ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, metadata_json FROM messages
                 WHERE session_id = ?1 AND metadata_json IS NOT NULL
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|e| format!("Failed to prepare knowledge proposal query: {}", e))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("Failed to query knowledge proposals: {}", e))?;

        let mut target_message_id: Option<String> = None;
        let mut target_metadata_json: Option<String> = None;
        let now = Self::now_ts();

        for row in rows {
            let (message_id, metadata_json) =
                row.map_err(|e| format!("Failed to read knowledge proposal row: {}", e))?;
            let mut metadata: MessageMetadata = serde_json::from_str(&metadata_json)
                .map_err(|e| format!("Failed to parse knowledge proposal metadata: {}", e))?;
            let Some(proposal) = metadata.knowledge_proposal.as_mut() else {
                continue;
            };
            if proposal.proposal_id != proposal_id {
                continue;
            }
            if !Self::is_valid_knowledge_proposal_status_transition(&proposal.status, &status) {
                return Err(format!(
                    "Invalid knowledge proposal transition: {:?} -> {:?}",
                    proposal.status, status
                ));
            }
            proposal.status = status.clone();
            proposal.updated_at = now;
            target_message_id = Some(message_id);
            target_metadata_json =
                Some(serde_json::to_string(&metadata).map_err(|e| {
                    format!("Failed to serialize knowledge proposal metadata: {}", e)
                })?);
            break;
        }
        drop(stmt);

        let Some(message_id) = target_message_id else {
            return Ok(None);
        };
        let Some(metadata_json) = target_metadata_json else {
            return Ok(None);
        };

        conn.execute(
            "UPDATE messages SET metadata_json = ?1 WHERE id = ?2 AND session_id = ?3",
            params![metadata_json, message_id, session_id],
        )
        .map_err(|e| format!("Failed to update knowledge proposal status: {}", e))?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| format!("Failed to update session timestamp: {}", e))?;
        drop(conn);

        self.get_knowledge_proposal_message(session_id, proposal_id)
    }

    fn is_valid_knowledge_proposal_status_transition(
        current: &KnowledgeProposalStatus,
        next: &KnowledgeProposalStatus,
    ) -> bool {
        if current == next {
            return true;
        }
        matches!(
            (current, next),
            (
                KnowledgeProposalStatus::Pending,
                KnowledgeProposalStatus::Applying
            ) | (
                KnowledgeProposalStatus::Pending,
                KnowledgeProposalStatus::Invalidated
            ) | (
                KnowledgeProposalStatus::Pending,
                KnowledgeProposalStatus::Stale
            ) | (
                KnowledgeProposalStatus::Applying,
                KnowledgeProposalStatus::Applied
            ) | (
                KnowledgeProposalStatus::Applying,
                KnowledgeProposalStatus::Pending
            ) | (
                KnowledgeProposalStatus::Applying,
                KnowledgeProposalStatus::Invalidated
            )
        )
    }

    fn update_pending_knowledge_proposals(
        &self,
        session_id: &str,
        status: KnowledgeProposalStatus,
    ) -> Result<Vec<ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, metadata_json FROM messages
                 WHERE session_id = ?1 AND metadata_json IS NOT NULL
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|e| format!("Failed to prepare pending knowledge proposal query: {}", e))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("Failed to query pending knowledge proposals: {}", e))?;

        let now = Self::now_ts();
        let mut updates: Vec<(String, String)> = Vec::new();
        let mut changed_proposal_ids: Vec<String> = Vec::new();

        for row in rows {
            let (message_id, metadata_json) =
                row.map_err(|e| format!("Failed to read pending knowledge proposal row: {}", e))?;
            let mut metadata: MessageMetadata =
                serde_json::from_str(&metadata_json).map_err(|e| {
                    format!("Failed to parse pending knowledge proposal metadata: {}", e)
                })?;
            let Some(proposal) = metadata.knowledge_proposal.as_mut() else {
                continue;
            };
            if proposal.status != KnowledgeProposalStatus::Pending {
                continue;
            }
            proposal.status = status.clone();
            proposal.updated_at = now;
            let proposal_id = proposal.proposal_id.clone();
            let serialized = serde_json::to_string(&metadata).map_err(|e| {
                format!(
                    "Failed to serialize pending knowledge proposal metadata: {}",
                    e
                )
            })?;
            updates.push((message_id, serialized));
            changed_proposal_ids.push(proposal_id);
        }
        drop(stmt);

        if updates.is_empty() {
            return Ok(Vec::new());
        }

        for (message_id, metadata_json) in &updates {
            conn.execute(
                "UPDATE messages SET metadata_json = ?1 WHERE id = ?2 AND session_id = ?3",
                params![metadata_json, message_id, session_id],
            )
            .map_err(|e| format!("Failed to update pending knowledge proposal: {}", e))?;
        }
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| format!("Failed to update session timestamp: {}", e))?;
        drop(conn);

        let all_messages = self.get_messages(session_id)?;
        Ok(all_messages
            .into_iter()
            .filter(|message| {
                message
                    .knowledge_proposal
                    .as_ref()
                    .map(|proposal| changed_proposal_ids.contains(&proposal.proposal_id))
                    .unwrap_or(false)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionStore;
    use crate::session::models::{
        ChatMessage, KnowledgeProposalStatus, MessageRole, TodoItem, ToolCallInfo,
    };
    use rusqlite::{params, Connection};
    use tempfile::tempdir;

    #[test]
    fn knowledge_proposal_status_transition_is_closed() {
        assert!(SessionStore::is_valid_knowledge_proposal_status_transition(
            &KnowledgeProposalStatus::Pending,
            &KnowledgeProposalStatus::Applying,
        ));
        assert!(SessionStore::is_valid_knowledge_proposal_status_transition(
            &KnowledgeProposalStatus::Applying,
            &KnowledgeProposalStatus::Applied,
        ));
        assert!(
            !SessionStore::is_valid_knowledge_proposal_status_transition(
                &KnowledgeProposalStatus::Applied,
                &KnowledgeProposalStatus::Invalidated,
            )
        );
        assert!(
            !SessionStore::is_valid_knowledge_proposal_status_transition(
                &KnowledgeProposalStatus::Stale,
                &KnowledgeProposalStatus::Invalidated,
            )
        );
    }

    #[test]
    fn fresh_database_is_created_at_latest_schema_version() {
        let dir = tempdir().expect("create temp dir");

        let _store = SessionStore::new(dir.path()).expect("initialize store");
        let conn = Connection::open(dir.path().join("locus.db")).expect("open db");

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(SessionStore::table_has_column(&conn, "sessions", "archived_at").unwrap());
        assert!(SessionStore::table_has_column(&conn, "sessions", "workspace_id").unwrap());
        assert!(
            SessionStore::table_has_column(&conn, "sessions", "latest_completed_run_id").unwrap()
        );
        assert!(SessionStore::table_has_column(&conn, "messages", "metadata_json").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_prefix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_suffix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
    }

    #[test]
    fn v7_database_is_migrated_forward_without_losing_sessions() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 7;",
        )
        .expect("create v7 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, metadata_json)
             VALUES (?1, ?2, 'assistant', 'hello', 100, NULL)",
            params!["message-1", "session-1"],
        )
        .expect("insert message");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let sessions = store.list_sessions(None).expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Migrated Session");

        let detail = store.load_session("session-1").expect("load session");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.messages[0].content, "hello");
        assert_eq!(detail.messages[0].prompt_prefix, None);
        assert_eq!(detail.messages[0].prompt_suffix, None);
        assert_eq!(detail.latest_completed_run_id, None);

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(SessionStore::table_has_column(&conn, "sessions", "archived_at").unwrap());
        assert!(
            SessionStore::table_has_column(&conn, "sessions", "latest_completed_run_id").unwrap()
        );
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_prefix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_suffix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
    }

    #[test]
    fn v8_database_is_migrated_forward_with_prompt_columns() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 8;",
        )
        .expect("create v8 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, metadata_json)
             VALUES (?1, ?2, 'user', 'hello', 100, NULL)",
            params!["message-1", "session-1"],
        )
        .expect("insert message");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.messages[0].content, "hello");
        assert_eq!(detail.messages[0].prompt_prefix, None);
        assert_eq!(detail.messages[0].prompt_suffix, None);
        assert_eq!(detail.latest_completed_run_id, None);

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(
            SessionStore::table_has_column(&conn, "sessions", "latest_completed_run_id").unwrap()
        );
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_prefix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "prompt_suffix").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
    }

    #[test]
    fn v9_database_is_migrated_forward_with_prompt_window_flag() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 9;",
        )
        .expect("create v9 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, metadata_json)
             VALUES (?1, ?2, 'user', 'hello', 100, 'prefix', NULL, NULL)",
            params!["message-1", "session-1"],
        )
        .expect("insert message");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.messages[0].content, "hello");
        assert_eq!(detail.messages[0].prompt_prefix.as_deref(), Some("prefix"));
        assert_eq!(detail.latest_completed_run_id, None);

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(
            SessionStore::table_has_column(&conn, "sessions", "latest_completed_run_id").unwrap()
        );
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
        let include_in_prompt: i64 = conn
            .query_row(
                "SELECT include_in_prompt FROM messages WHERE id = 'message-1'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated flag");
        assert_eq!(include_in_prompt, 1);
    }

    #[test]
    fn v10_database_is_migrated_forward_with_latest_completed_run_id() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT,
                include_in_prompt INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 10;",
        )
        .expect("create v10 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        assert_eq!(detail.latest_completed_run_id, None);

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(
            SessionStore::table_has_column(&conn, "sessions", "latest_completed_run_id").unwrap()
        );
    }

    #[test]
    fn v11_database_migrates_tool_call_payloads_forward() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                latest_completed_run_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT,
                include_in_prompt INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 11;",
        )
        .expect("create v11 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, latest_completed_run_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");

        let tool_calls_json = serde_json::to_string(&vec![ToolCallInfo {
            id: "tc-1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }])
        .expect("serialize tool calls");

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt)
             VALUES (?1, ?2, 'assistant', '', 100, NULL, NULL, ?3, NULL, NULL, NULL, NULL, NULL, NULL, 1)",
            params!["message-1", "session-1", tool_calls_json],
        )
        .expect("insert assistant message");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt)
             VALUES (?1, ?2, 'tool', ?3, 100, NULL, NULL, NULL, ?4, NULL, NULL, NULL, NULL, NULL, 1)",
            params!["tool-1", "session-1", crate::session::history::INTERRUPTED_TOOL_RESULT, "tc-1"],
        )
        .expect("insert tool message");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt)
             VALUES (?1, ?2, 'user', 'continue', 101, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1)",
            params!["user-1", "session-1"],
        )
        .expect("insert user message");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        let tool_calls = detail.messages[0]
            .tool_calls
            .as_ref()
            .expect("assistant tool calls");
        assert_eq!(
            tool_calls[0].outcome,
            Some(crate::commands::ToolCallOutcome::Interrupted)
        );

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
    }

    #[test]
    fn set_latest_completed_run_id_persists_to_session_detail() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Run Boundary", None, None, "chat", None)
            .expect("create session");

        store
            .set_latest_completed_run_id(&session_id, Some("run-final"))
            .expect("set run id");

        let detail = store.load_session(&session_id).expect("load session");
        assert_eq!(detail.latest_completed_run_id.as_deref(), Some("run-final"));

        store
            .set_latest_completed_run_id(&session_id, None)
            .expect("clear run id");

        let detail = store.load_session(&session_id).expect("reload session");
        assert_eq!(detail.latest_completed_run_id, None);
    }

    #[test]
    fn v12_database_is_migrated_forward_with_latest_todo_run_id() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                latest_completed_run_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT,
                include_in_prompt INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 12;",
        )
        .expect("create v12 schema");

        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let sessions = store
            .list_sessions(None)
            .expect("list sessions after migration");
        assert_eq!(sessions.len(), 0);

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(SessionStore::table_has_column(&conn, "sessions", "latest_todo_run_id").unwrap());
    }

    #[test]
    fn update_todos_persists_latest_todo_run_id() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Todo Boundary", None, None, "chat", None)
            .expect("create session");

        store
            .update_todos(
                &session_id,
                Some("run-todo"),
                &[TodoItem {
                    content: "Track current run".to_string(),
                    status: "completed".to_string(),
                    priority: "medium".to_string(),
                }],
            )
            .expect("persist todos");

        let snapshot = store.get_todos(&session_id).expect("load todo snapshot");
        assert_eq!(snapshot.latest_run_id.as_deref(), Some("run-todo"));
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].content, "Track current run");
    }

    #[test]
    fn pre_v7_database_is_reset_instead_of_migrated() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             PRAGMA user_version = 6;",
        )
        .expect("create legacy schema");
        conn.execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, 1, 1)",
            params!["legacy-session", "Legacy Session"],
        )
        .expect("insert legacy session");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("recreate store");
        let sessions = store.list_sessions(None).expect("list sessions");
        assert!(sessions.is_empty());

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
    }

    #[test]
    fn truncate_from_message_uses_rowid_boundary_for_same_second_messages() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Rowid Boundary", None, None, "chat", None)
            .expect("create session");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content) in [
                ("user-old", "user", "older user"),
                ("assistant-old", "assistant", "older assistant"),
                ("user-target", "user", "target user"),
                ("assistant-target", "assistant", "target assistant"),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, 100i64],
                )
                .expect("insert message");
            }
        }

        let deleted = store
            .truncate_from_message(&session_id, "assistant-target")
            .expect("truncate messages");
        assert_eq!(deleted, 2);

        let messages = store.get_messages(&session_id).expect("load messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, "user-old");
        assert_eq!(messages[1].id, "assistant-old");
    }

    #[test]
    fn compact_messages_preserve_visible_history_and_limit_future_prompt_context() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact Test", None, None, "chat", None)
            .expect("create session");

        let old_user_id = "old-user";
        let old_assistant_id = "old-assistant";
        let latest_user_id = "latest-user";
        let latest_assistant_id = "latest-assistant";
        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at, prompt_prefix) in [
                (
                    old_user_id,
                    "user",
                    "旧需求",
                    100i64,
                    Some("<system-reminder>\nEnv\n</system-reminder>"),
                ),
                (old_assistant_id, "assistant", "旧回答", 101i64, None),
                (latest_user_id, "user", "最新需求", 102i64, None),
                (latest_assistant_id, "assistant", "最新回答", 103i64, None),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at, prompt_prefix],
                )
                .expect("insert message");
            }
        }

        let summary_msg = ChatMessage {
            id: "handoff-1".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n交接摘要".to_string(),
            created_at: 101,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        };

        let (count_before, count_after) = store
            .compact_messages(&session_id, &summary_msg, latest_user_id)
            .expect("compact messages");
        assert_eq!(count_before, 4);
        assert_eq!(count_after, 3);

        let all_messages = store.get_messages(&session_id).expect("load all messages");
        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        let detail = store
            .load_session(&session_id)
            .expect("load session detail");

        assert_eq!(all_messages.len(), 5);
        assert_eq!(detail.messages.len(), 5);
        assert_eq!(
            all_messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                old_user_id,
                old_assistant_id,
                "handoff-1",
                latest_user_id,
                latest_assistant_id,
            ]
        );
        assert_eq!(prompt_messages.len(), 3);
        assert_eq!(prompt_messages[0].id, "handoff-1");
        assert_eq!(prompt_messages[1].content, "最新需求");
        assert_eq!(prompt_messages[2].content, "最新回答");
        assert_eq!(
            prompt_messages[1].prompt_prefix.as_deref(),
            Some("<system-reminder>\nEnv\n</system-reminder>")
        );
        assert_eq!(
            store
                .first_user_message_id(&session_id)
                .expect("first prompt user"),
            Some(latest_user_id.to_string())
        );
    }
}
