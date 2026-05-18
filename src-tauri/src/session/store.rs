use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::models::{
    AssistantRenderPart, ChatMessage, KnowledgeProposal, KnowledgeProposalStatus, MessageRole,
    SessionDetail, SessionEventRecord, SessionRunSummary, SessionSummary, TodoItem, TodoSnapshot,
    ToolCallInfo,
};
use crate::commands::TokenUsage;
use crate::compact;

#[derive(Clone)]
pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
    data_dir: PathBuf,
    event_writer: Arc<SessionEventWriter>,
}

#[derive(Debug, Clone)]
pub struct SessionEventAppend {
    pub session_id: String,
    pub run_id: String,
    pub event_type: String,
    pub payload_json: String,
}

#[derive(Debug, Clone)]
pub struct SessionEventMerge {
    pub key: String,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct SessionRunStatusUpdate {
    pub run_id: String,
    pub status: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
struct QueuedSessionEvent {
    event: SessionEventAppend,
    merge: Option<SessionEventMerge>,
    status_updates: Vec<SessionRunStatusUpdate>,
}

struct SessionEventWriter {
    sender: mpsc::Sender<QueuedSessionEvent>,
}

const TOOL_RESULTS_DIR: &str = "tool-results";
const TOOL_RESULT_PREVIEW_CHARS: usize = 2_000;
const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 50_000;
const LARGE_RESULT_TAG_OPEN: &str = "<persisted-output>";
const LARGE_RESULT_TAG_CLOSE: &str = "</persisted-output>";
pub const CHILD_SESSION_FORK_ERROR: &str = "Child sessions cannot be forked";
const RUN_STATUS_QUEUED: &str = "queued";
const RUN_STATUS_STARTING: &str = "starting";
const RUN_STATUS_RUNNING: &str = "running";
const RUN_STATUS_WAITING_INPUT: &str = "waiting_input";
const RUN_STATUS_FINISHING: &str = "finishing";
const RUN_STATUS_CANCELLING: &str = "cancelling";
const RUN_STATUS_DONE: &str = "done";
const RUN_STATUS_CANCELLED: &str = "cancelled";
const RUN_STATUS_ERROR: &str = "error";
const CONTEXT_HANDOFF_MARKER: &str = "## Context Handoff";
const CONTEXT_COMPACTED_DISPLAY_MARKER: &str = "## Context Handoff\n\nContext compacted.";

impl SessionEventWriter {
    const FLUSH_INTERVAL: Duration = Duration::from_millis(25);
    const MAX_BATCH_SIZE: usize = 128;

    fn new(conn: Arc<Mutex<Connection>>) -> Self {
        let (sender, receiver) = mpsc::channel::<QueuedSessionEvent>();
        thread::Builder::new()
            .name("locus-session-event-writer".to_string())
            .spawn(move || Self::run(conn, receiver))
            .expect("spawn session event writer");
        Self { sender }
    }

    fn enqueue(&self, event: QueuedSessionEvent) -> Result<(), String> {
        self.sender
            .send(event)
            .map_err(|e| format!("Failed to queue session event: {}", e))
    }

    fn run(conn: Arc<Mutex<Connection>>, receiver: mpsc::Receiver<QueuedSessionEvent>) {
        let mut batch = Vec::with_capacity(Self::MAX_BATCH_SIZE);
        while let Ok(first) = receiver.recv() {
            batch.clear();
            batch.push(first);

            while batch.len() < Self::MAX_BATCH_SIZE {
                match receiver.recv_timeout(Self::FLUSH_INTERVAL) {
                    Ok(event) => batch.push(event),
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }

            let coalesced = Self::coalesce_batch(&batch);
            let events = coalesced
                .iter()
                .map(|item| item.event.clone())
                .collect::<Vec<_>>();

            if let Err(error) = SessionStore::append_session_events_batch_on_conn(&conn, &events) {
                eprintln!("[Locus] failed to flush session event batch: {}", error);
            }

            for status in coalesced.iter().flat_map(|item| item.status_updates.iter()) {
                if let Err(error) = SessionStore::update_run_status_on_conn(
                    &conn,
                    &status.run_id,
                    &status.status,
                    status.error_message.as_deref(),
                ) {
                    eprintln!(
                        "[Locus] failed to flush session run status {} for run {}: {}",
                        status.status, status.run_id, error
                    );
                }
            }
        }
    }

    fn coalesce_batch(batch: &[QueuedSessionEvent]) -> Vec<QueuedSessionEvent> {
        let mut out: Vec<QueuedSessionEvent> = Vec::with_capacity(batch.len());

        for item in batch {
            if let Some(merge) = item.merge.as_ref() {
                if let Some(last) = out.last_mut() {
                    let same_key = last
                        .merge
                        .as_ref()
                        .map(|last_merge| last_merge.key.as_str())
                        == Some(merge.key.as_str());

                    if same_key
                        && Self::append_payload_field(
                            &mut last.event.payload_json,
                            &merge.field,
                            &merge.value,
                        )
                    {
                        last.status_updates.extend(item.status_updates.clone());
                        continue;
                    }
                }
            }

            out.push(item.clone());
        }

        out
    }

    fn append_payload_field(payload_json: &mut String, field: &str, value: &str) -> bool {
        let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(payload_json) else {
            return false;
        };
        let Some(existing) = payload.get(field).and_then(|value| value.as_str()) else {
            return false;
        };
        let merged = format!("{}{}", existing, value);
        payload[field] = serde_json::Value::String(merged);
        match serde_json::to_string(&payload) {
            Ok(next) => {
                *payload_json = next;
                true
            }
            Err(_) => false,
        }
    }
}

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
        "web_fetch" => Some(100_000),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    content_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    render_parts: Option<Vec<AssistantRenderPart>>,
}

fn message_metadata_json(
    knowledge_proposal: Option<&KnowledgeProposal>,
    response_id: Option<&str>,
    response_request: Option<&serde_json::Value>,
    content_order: Option<u32>,
    thinking_order: Option<u32>,
    render_parts: Option<&[AssistantRenderPart]>,
) -> Result<Option<String>, String> {
    let metadata = MessageMetadata {
        knowledge_proposal: knowledge_proposal.cloned(),
        response_id: response_id.map(|value| value.to_string()),
        response_request: response_request.cloned(),
        content_order,
        thinking_order,
        render_parts: render_parts.map(|value| value.to_vec()),
    };
    if metadata.knowledge_proposal.is_none()
        && metadata.response_id.is_none()
        && metadata.response_request.is_none()
        && metadata.content_order.is_none()
        && metadata.thinking_order.is_none()
        && metadata.render_parts.is_none()
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

fn is_context_handoff_message(message: &ChatMessage) -> bool {
    message.role == MessageRole::Assistant && message.content.starts_with(CONTEXT_HANDOFF_MARKER)
}

fn redact_context_handoff_for_display(message: &mut ChatMessage) {
    if !is_context_handoff_message(message) {
        return;
    }

    message.content = CONTEXT_COMPACTED_DISPLAY_MARKER.to_string();
    message.prompt_prefix = None;
    message.prompt_suffix = None;
    message.response_id = None;
    message.content_order = None;
    message.thinking_order = None;
    message.tool_calls = None;
    message.tool_call_id = None;
    message.images = None;
    message.asset_refs = None;
    message.thinking_content = None;
    message.thinking_duration = None;
    message.thinking_signature = None;
    message.knowledge_proposal = None;
    message.render_parts = None;
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

fn copy_dir_recursively(source: &Path, target: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Ok(());
    }

    std::fs::create_dir_all(target).map_err(|e| {
        format!(
            "Failed to create copied tool result dir '{}': {}",
            target.display(),
            e
        )
    })?;

    for entry in std::fs::read_dir(source).map_err(|e| {
        format!(
            "Failed to read tool result dir '{}': {}",
            source.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("Failed to read tool result entry: {}", e))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to inspect tool result entry: {}", e))?;
        if file_type.is_dir() {
            copy_dir_recursively(&source_path, &target_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create copied tool result parent '{}': {}",
                        parent.display(),
                        e
                    )
                })?;
            }
            std::fs::copy(&source_path, &target_path).map_err(|e| {
                format!(
                    "Failed to copy tool result '{}' to '{}': {}",
                    source_path.display(),
                    target_path.display(),
                    e
                )
            })?;
        }
    }

    Ok(())
}

fn path_reference_pairs(source: &Path, target: &Path) -> Vec<(String, String)> {
    let raw_pairs = [
        (source.display().to_string(), target.display().to_string()),
        (
            source.to_string_lossy().into_owned(),
            target.to_string_lossy().into_owned(),
        ),
    ];
    let mut pairs = Vec::new();
    for (source_value, target_value) in raw_pairs {
        for (source_variant, target_variant) in [
            (source_value.clone(), target_value.clone()),
            (
                source_value.replace('\\', "/"),
                target_value.replace('\\', "/"),
            ),
            (
                source_value.replace('/', "\\"),
                target_value.replace('/', "\\"),
            ),
        ] {
            if source_variant.is_empty()
                || pairs
                    .iter()
                    .any(|(existing, _): &(String, String)| existing == &source_variant)
            {
                continue;
            }
            pairs.push((source_variant, target_variant));
        }
    }
    pairs
}

fn rewrite_tool_result_references(content: &str, source_dir: &Path, target_dir: &Path) -> String {
    let mut rewritten = content.to_string();
    for (source, target) in path_reference_pairs(source_dir, target_dir) {
        rewritten = rewritten.replace(&source, &target);
    }
    rewritten
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
    const SCHEMA_VERSION: i32 = 19;

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

        Self::run_migrations(&conn, data_dir)?;
        Self::mark_nonterminal_runs_cancelled(&conn)?;

        let conn = Arc::new(Mutex::new(conn));
        let event_writer = Arc::new(SessionEventWriter::new(conn.clone()));

        Ok(SessionStore {
            conn,
            data_dir: data_dir.to_path_buf(),
            event_writer,
        })
    }

    /// Fresh databases are created directly at the latest schema version.
    /// Supported upgrades start at v7, and every schema change after that must
    /// be expressed as an explicit migration keyed by `user_version`.
    fn run_migrations(conn: &Connection, data_dir: &Path) -> Result<(), String> {
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

        if current < 14 {
            Self::migrate(conn, 14, "add session run and event log tables", |conn| {
                Self::create_session_sync_schema(conn)
            })?;
        }

        if current < 15 {
            Self::migrate(conn, 15, "persist oversized tool results", |conn| {
                Self::migrate_oversized_tool_results(conn, data_dir)
            })?;
        }

        if current < 16 {
            Self::migrate(conn, 16, "add message render order metadata", |conn| {
                Self::migrate_message_render_orders(conn)
            })?;
        }

        if current < 17 {
            Self::migrate(conn, 17, "add message asset references", |conn| {
                if !Self::table_has_column(conn, "messages", "asset_refs")? {
                    conn.execute_batch("ALTER TABLE messages ADD COLUMN asset_refs TEXT;")?;
                }
                Ok(())
            })?;
        }

        if current < 18 {
            Self::migrate(conn, 18, "persist latest context usage", |conn| {
                if !Self::table_has_column(conn, "token_usage", "last_context_tokens")? {
                    conn.execute_batch(
                        "ALTER TABLE token_usage ADD COLUMN last_context_tokens INTEGER NOT NULL DEFAULT 0;",
                    )?;
                }
                if !Self::table_has_column(conn, "token_usage", "last_context_limit")? {
                    conn.execute_batch(
                        "ALTER TABLE token_usage ADD COLUMN last_context_limit INTEGER NOT NULL DEFAULT 0;",
                    )?;
                }
                Ok(())
            })?;
        }

        if current < 19 {
            Self::migrate(conn, 19, "reserve in-memory pending input queue", |_conn| {
                Ok(())
            })?;
        }

        debug_assert_eq!(Self::SCHEMA_VERSION, 19, "add a new migration block above");
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
                asset_refs TEXT,
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
                priced_rounds INTEGER NOT NULL DEFAULT 0,
                last_context_tokens INTEGER NOT NULL DEFAULT 0,
                last_context_limit INTEGER NOT NULL DEFAULT 0
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
        .and_then(|_| Self::create_session_sync_schema(conn))
    }

    fn create_session_sync_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_runs (
                run_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                finished_at INTEGER,
                error_message TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_session_runs_session ON session_runs(session_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_session_runs_status ON session_runs(status);

            CREATE TABLE IF NOT EXISTS session_events (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                run_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            CREATE INDEX IF NOT EXISTS idx_session_events_run ON session_events(run_id, seq);
            CREATE INDEX IF NOT EXISTS idx_session_events_session_seq ON session_events(session_id, seq);",
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

    fn mark_nonterminal_runs_cancelled(conn: &Connection) -> Result<(), String> {
        let now = Self::now_ts();
        conn.execute(
            "UPDATE session_runs
             SET status = ?1,
                 updated_at = ?2,
                 finished_at = COALESCE(finished_at, ?2),
                 error_message = COALESCE(error_message, 'Interrupted by application restart')
             WHERE status IN (?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                RUN_STATUS_CANCELLED,
                now,
                RUN_STATUS_QUEUED,
                RUN_STATUS_STARTING,
                RUN_STATUS_RUNNING,
                RUN_STATUS_WAITING_INPUT,
                RUN_STATUS_FINISHING,
                RUN_STATUS_CANCELLING,
            ],
        )
        .map_err(|e| format!("Failed to normalize interrupted session runs: {}", e))?;
        Ok(())
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

    fn migrate_oversized_tool_results(conn: &Connection, data_dir: &Path) -> rusqlite::Result<()> {
        fn to_sql_error(error: String) -> rusqlite::Error {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                error,
            )))
        }

        let mut stmt = conn.prepare("SELECT id FROM sessions ORDER BY created_at ASC, id ASC")?;
        let session_ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        for session_id in session_ids {
            let raw_messages =
                Self::get_messages_with_conn_filtered_static(conn, &session_id, false)
                    .map_err(to_sql_error)?;
            let normalized = crate::session::history::normalize_tool_round_history(&raw_messages);
            let mut tool_names: HashMap<String, String> = HashMap::new();
            for message in &normalized {
                if message.role != MessageRole::Assistant {
                    continue;
                }
                if let Some(tool_calls) = message.tool_calls.as_ref() {
                    for tool_call in tool_calls {
                        if !tool_call.id.trim().is_empty() {
                            tool_names
                                .entry(tool_call.id.clone())
                                .or_insert_with(|| tool_call.name.clone());
                        }
                    }
                }
            }

            for message in raw_messages {
                if message.role != MessageRole::Tool {
                    continue;
                }
                let Some(tool_call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                let tool_name = tool_names
                    .get(tool_call_id)
                    .map(String::as_str)
                    .unwrap_or("unknown");
                let rewritten = Self::rewrite_tool_result_for_storage_at(
                    data_dir,
                    &session_id,
                    tool_call_id,
                    tool_name,
                    &message.content,
                )
                .map_err(to_sql_error)?;
                if rewritten != message.content {
                    conn.execute(
                        "UPDATE messages SET content = ?1 WHERE id = ?2 AND session_id = ?3",
                        params![rewritten, message.id, session_id],
                    )?;
                }
            }
        }

        Ok(())
    }

    fn migrate_message_render_orders(conn: &Connection) -> rusqlite::Result<()> {
        fn to_sql_error(
            error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
        ) -> rusqlite::Error {
            rusqlite::Error::ToSqlConversionFailure(error.into())
        }

        fn bump_next_order(next_order: &mut u32, order: Option<u32>) {
            if let Some(order) = order.filter(|value| *value > 0) {
                *next_order = (*next_order).max(order.saturating_add(1));
            }
        }

        fn assign_tool_call_orders(tool_calls: &mut [ToolCallInfo], next_order: &mut u32) -> bool {
            let mut changed = false;
            for tool_call in tool_calls {
                if tool_call.order.is_none() {
                    tool_call.order = Some(*next_order);
                    *next_order = next_order.saturating_add(1);
                    changed = true;
                } else {
                    bump_next_order(next_order, tool_call.order);
                }

                if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_mut() {
                    changed |= assign_tool_call_orders(nested_tool_calls, next_order);
                }
            }
            changed
        }

        let mut stmt = conn.prepare(
            "SELECT id, role, content, tool_calls, thinking_content, metadata_json
             FROM messages
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        for (message_id, role, content, tool_calls_json, thinking_content, metadata_json) in rows {
            if role != "assistant" {
                continue;
            }

            let mut metadata: MessageMetadata = metadata_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(to_sql_error)?
                .unwrap_or_default();
            let mut metadata_changed = false;
            let mut next_order = 1u32;

            bump_next_order(&mut next_order, metadata.thinking_order);
            bump_next_order(&mut next_order, metadata.content_order);

            let has_thinking = thinking_content
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            if has_thinking && metadata.thinking_order.is_none() {
                metadata.thinking_order = Some(next_order);
                next_order = next_order.saturating_add(1);
                metadata_changed = true;
            }

            if !content.trim().is_empty() && metadata.content_order.is_none() {
                metadata.content_order = Some(next_order);
                next_order = next_order.saturating_add(1);
                metadata_changed = true;
            }

            let mut next_tool_calls_json = None;
            if let Some(tool_calls_json) = tool_calls_json.as_deref() {
                let mut tool_calls: Vec<ToolCallInfo> =
                    serde_json::from_str(tool_calls_json).map_err(to_sql_error)?;
                if assign_tool_call_orders(&mut tool_calls, &mut next_order) {
                    next_tool_calls_json =
                        Some(serde_json::to_string(&tool_calls).map_err(to_sql_error)?);
                }
            }

            let next_metadata_json = if metadata_changed {
                if metadata.knowledge_proposal.is_none()
                    && metadata.response_id.is_none()
                    && metadata.response_request.is_none()
                    && metadata.content_order.is_none()
                    && metadata.thinking_order.is_none()
                    && metadata.render_parts.is_none()
                {
                    Some(None)
                } else {
                    Some(Some(
                        serde_json::to_string(&metadata).map_err(to_sql_error)?,
                    ))
                }
            } else {
                None
            };

            if next_metadata_json.is_none() && next_tool_calls_json.is_none() {
                continue;
            }

            conn.execute(
                "UPDATE messages
                 SET metadata_json = COALESCE(?1, metadata_json),
                     tool_calls = COALESCE(?2, tool_calls)
                 WHERE id = ?3",
                params![
                    next_metadata_json.flatten(),
                    next_tool_calls_json,
                    message_id,
                ],
            )?;
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

    pub fn fork_session(&self, source_id: &str, title: Option<&str>) -> Result<String, String> {
        #[derive(Debug)]
        struct PersistedMessageRow {
            role: String,
            content: String,
            created_at: i64,
            prompt_prefix: Option<String>,
            prompt_suffix: Option<String>,
            tool_calls: Option<String>,
            tool_call_id: Option<String>,
            images: Option<String>,
            asset_refs: Option<String>,
            thinking_content: Option<String>,
            thinking_duration: Option<i64>,
            thinking_signature: Option<String>,
            metadata_json: Option<String>,
            include_in_prompt: i64,
        }

        let new_id = Uuid::new_v4().to_string();
        let now = Self::now_ts();
        let source_tool_dir = self.session_tool_results_dir(source_id);
        let target_tool_dir = self.session_tool_results_dir(&new_id);
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute("BEGIN IMMEDIATE", [])
            .map_err(|e| format!("Failed to begin session fork transaction: {}", e))?;

        let result = (|| -> Result<String, String> {
            let (
                source_title,
                parent_session_id,
                workspace_id,
                session_type,
                agent_id,
                latest_completed_run_id,
                latest_todo_run_id,
            ) = conn
                .query_row(
                    "SELECT title, parent_session_id, workspace_id, session_type, agent_id, latest_completed_run_id, latest_todo_run_id
                     FROM sessions WHERE id = ?1",
                    params![source_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .map_err(|e| format!("Session not found: {}", e))?;

            if parent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            {
                return Err(CHILD_SESSION_FORK_ERROR.to_string());
            }

            if source_tool_dir.is_dir() {
                copy_dir_recursively(&source_tool_dir, &target_tool_dir)?;
            }

            let resolved_title = title
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{} (fork)", source_title));

            conn.execute(
                "INSERT INTO sessions (
                    id,
                    title,
                    parent_session_id,
                    workspace_id,
                    session_type,
                    agent_id,
                    archived_at,
                    latest_completed_run_id,
                    latest_todo_run_id,
                    created_at,
                    updated_at
                 )
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?8)",
                params![
                    new_id,
                    resolved_title,
                    workspace_id,
                    session_type,
                    agent_id,
                    latest_completed_run_id,
                    latest_todo_run_id,
                    now,
                ],
            )
            .map_err(|e| format!("Failed to create forked session: {}", e))?;

            let message_rows = {
                let mut stmt = conn
                    .prepare(
                        "SELECT role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, asset_refs, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt
                         FROM messages
                         WHERE session_id = ?1
                         ORDER BY rowid ASC",
                    )
                    .map_err(|e| format!("Failed to prepare fork message query: {}", e))?;
                let rows = stmt
                    .query_map(params![source_id], |row| {
                        Ok(PersistedMessageRow {
                            role: row.get(0)?,
                            content: row.get(1)?,
                            created_at: row.get(2)?,
                            prompt_prefix: row.get(3)?,
                            prompt_suffix: row.get(4)?,
                            tool_calls: row.get(5)?,
                            tool_call_id: row.get(6)?,
                            images: row.get(7)?,
                            asset_refs: row.get(8)?,
                            thinking_content: row.get(9)?,
                            thinking_duration: row.get(10)?,
                            thinking_signature: row.get(11)?,
                            metadata_json: row.get(12)?,
                            include_in_prompt: row.get(13)?,
                        })
                    })
                    .map_err(|e| format!("Failed to query messages for fork: {}", e))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Failed to read fork message row: {}", e))?
            };

            let rewrite_tool_paths = source_tool_dir.is_dir();
            for row in message_rows {
                let message_id = Uuid::new_v4().to_string();
                let content = if rewrite_tool_paths {
                    rewrite_tool_result_references(&row.content, &source_tool_dir, &target_tool_dir)
                } else {
                    row.content
                };
                conn.execute(
                    "INSERT INTO messages (
                        id,
                        session_id,
                        role,
                        content,
                        created_at,
                        prompt_prefix,
                        prompt_suffix,
                        tool_calls,
                        tool_call_id,
                        images,
                        asset_refs,
                        thinking_content,
                        thinking_duration,
                        thinking_signature,
                        metadata_json,
                        include_in_prompt
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        message_id,
                        new_id,
                        row.role,
                        content,
                        row.created_at,
                        row.prompt_prefix,
                        row.prompt_suffix,
                        row.tool_calls,
                        row.tool_call_id,
                        row.images,
                        row.asset_refs,
                        row.thinking_content,
                        row.thinking_duration,
                        row.thinking_signature,
                        row.metadata_json,
                        row.include_in_prompt,
                    ],
                )
                .map_err(|e| format!("Failed to copy message into fork: {}", e))?;
            }

            conn.execute(
                "INSERT INTO token_usage (
                    session_id,
                    total_input_tokens,
                    total_output_tokens,
                    total_cache_read_tokens,
                    total_cache_write_tokens,
                    total_cost_usd,
                    priced_rounds,
                    last_context_tokens,
                    last_context_limit
                 )
                 SELECT ?1,
                    total_input_tokens,
                    total_output_tokens,
                    total_cache_read_tokens,
                    total_cache_write_tokens,
                    total_cost_usd,
                    priced_rounds,
                    last_context_tokens,
                    last_context_limit
                 FROM token_usage
                 WHERE session_id = ?2",
                params![new_id, source_id],
            )
            .map_err(|e| format!("Failed to copy token usage into fork: {}", e))?;

            conn.execute(
                "INSERT INTO todos (session_id, position, content, status, priority)
                 SELECT ?1, position, content, status, priority
                 FROM todos
                 WHERE session_id = ?2
                 ORDER BY position ASC",
                params![new_id, source_id],
            )
            .map_err(|e| format!("Failed to copy todos into fork: {}", e))?;

            Ok(new_id.clone())
        })();

        match result {
            Ok(id) => {
                if let Err(e) = conn.execute("COMMIT", []) {
                    if target_tool_dir.is_dir() {
                        let _ = std::fs::remove_dir_all(&target_tool_dir);
                    }
                    return Err(format!("Failed to commit session fork: {}", e));
                }
                Ok(id)
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", []);
                if target_tool_dir.is_dir() {
                    let _ = std::fs::remove_dir_all(&target_tool_dir);
                }
                Err(error)
            }
        }
    }

    pub fn try_start_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute("BEGIN IMMEDIATE", [])
            .map_err(|e| format!("Failed to begin run transaction: {}", e))?;

        let active_run = conn
            .query_row(
                "SELECT run_id FROM session_runs
                 WHERE session_id = ?1 AND status IN (?2, ?3, ?4, ?5, ?6, ?7)
                 ORDER BY updated_at DESC
                 LIMIT 1",
                params![
                    session_id,
                    RUN_STATUS_QUEUED,
                    RUN_STATUS_STARTING,
                    RUN_STATUS_RUNNING,
                    RUN_STATUS_WAITING_INPUT,
                    RUN_STATUS_FINISHING,
                    RUN_STATUS_CANCELLING,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                format!("Failed to query active run: {}", e)
            })?;

        if let Some(active_run) = active_run {
            let _ = conn.execute("ROLLBACK", []);
            return Err(format!("Session already has an active run: {}", active_run));
        }

        conn.execute(
            "INSERT INTO session_runs (run_id, session_id, status, started_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![run_id, session_id, RUN_STATUS_STARTING, now],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to start session run: {}", e)
        })?;

        conn.execute("COMMIT", [])
            .map_err(|e| format!("Failed to commit run transaction: {}", e))?;
        Ok(())
    }

    pub fn update_run_status(
        &self,
        run_id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<(), String> {
        Self::update_run_status_on_conn(&self.conn, run_id, status, error_message)
    }

    fn update_run_status_on_conn(
        conn: &Arc<Mutex<Connection>>,
        run_id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<(), String> {
        let now = Self::now_ts();
        let is_terminal = matches!(
            status,
            RUN_STATUS_DONE | RUN_STATUS_CANCELLED | RUN_STATUS_ERROR
        );
        let is_terminal_flag = if is_terminal { 1i64 } else { 0i64 };
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE session_runs
             SET status = ?1,
                 updated_at = ?2,
                 finished_at = CASE WHEN ?3 = 1 THEN COALESCE(finished_at, ?2) ELSE finished_at END,
                 error_message = COALESCE(?4, error_message)
             WHERE run_id = ?5
               AND status NOT IN (?6, ?7, ?8)
               AND NOT (status = ?9 AND ?3 = 0)",
            params![
                status,
                now,
                is_terminal_flag,
                error_message,
                run_id,
                RUN_STATUS_DONE,
                RUN_STATUS_CANCELLED,
                RUN_STATUS_ERROR,
                RUN_STATUS_CANCELLING,
            ],
        )
        .map_err(|e| format!("Failed to update session run status: {}", e))?;
        Ok(())
    }

    pub fn close_run_pending_input_queue(&self, run_id: &str) -> Result<(), String> {
        self.update_run_status(run_id, RUN_STATUS_FINISHING, None)
    }

    pub fn active_run_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionRunSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT run_id, session_id, status, started_at, updated_at, finished_at, error_message
             FROM session_runs
             WHERE session_id = ?1 AND status IN (?2, ?3, ?4, ?5, ?6, ?7)
             ORDER BY updated_at DESC
             LIMIT 1",
            params![
                session_id,
                RUN_STATUS_QUEUED,
                RUN_STATUS_STARTING,
                RUN_STATUS_RUNNING,
                RUN_STATUS_WAITING_INPUT,
                RUN_STATUS_FINISHING,
                RUN_STATUS_CANCELLING,
            ],
            |row| {
                Ok(SessionRunSummary {
                    run_id: row.get(0)?,
                    session_id: row.get(1)?,
                    status: row.get(2)?,
                    started_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    error_message: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("Failed to query active session run: {}", e))
    }

    pub fn active_descendant_runs(
        &self,
        root_session_id: &str,
    ) -> Result<Vec<SessionRunSummary>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE descendants(id) AS (
                    SELECT id FROM sessions WHERE parent_session_id = ?1
                    UNION ALL
                    SELECT sessions.id
                    FROM sessions
                    JOIN descendants ON sessions.parent_session_id = descendants.id
                 )
                 SELECT session_runs.run_id,
                        session_runs.session_id,
                        session_runs.status,
                        session_runs.started_at,
                        session_runs.updated_at,
                        session_runs.finished_at,
                        session_runs.error_message
                 FROM session_runs
                 JOIN descendants ON descendants.id = session_runs.session_id
                  WHERE session_runs.status IN (?2, ?3, ?4, ?5, ?6, ?7)
                 ORDER BY session_runs.updated_at DESC",
            )
            .map_err(|e| format!("Failed to prepare active descendant run query: {}", e))?;

        let rows = stmt
            .query_map(
                params![
                    root_session_id,
                    RUN_STATUS_QUEUED,
                    RUN_STATUS_STARTING,
                    RUN_STATUS_RUNNING,
                    RUN_STATUS_WAITING_INPUT,
                    RUN_STATUS_FINISHING,
                    RUN_STATUS_CANCELLING,
                ],
                |row| {
                    Ok(SessionRunSummary {
                        run_id: row.get(0)?,
                        session_id: row.get(1)?,
                        status: row.get(2)?,
                        started_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        finished_at: row.get(5)?,
                        error_message: row.get(6)?,
                    })
                },
            )
            .map_err(|e| format!("Failed to query active descendant runs: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read active descendant run: {}", e))
    }

    pub fn session_id_for_run(&self, run_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT session_id FROM session_runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("Failed to query session run owner: {}", e))
    }

    pub fn append_session_event(
        &self,
        session_id: &str,
        run_id: &str,
        event_type: &str,
        payload_json: &str,
    ) -> Result<i64, String> {
        let seqs = self.append_session_events_batch(&[SessionEventAppend {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload_json: payload_json.to_string(),
        }])?;
        seqs.first()
            .copied()
            .ok_or_else(|| "Session event batch unexpectedly produced no sequence".to_string())
    }

    pub fn enqueue_session_event(
        &self,
        event: SessionEventAppend,
        merge: Option<SessionEventMerge>,
        status: Option<SessionRunStatusUpdate>,
    ) -> Result<(), String> {
        self.event_writer.enqueue(QueuedSessionEvent {
            event,
            merge,
            status_updates: status.into_iter().collect(),
        })
    }

    pub fn append_session_events_batch(
        &self,
        records: &[SessionEventAppend],
    ) -> Result<Vec<i64>, String> {
        Self::append_session_events_batch_on_conn(&self.conn, records)
    }

    fn append_session_events_batch_on_conn(
        conn: &Arc<Mutex<Connection>>,
        records: &[SessionEventAppend],
    ) -> Result<Vec<i64>, String> {
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let now = Self::now_ts();
        let conn = conn.lock().map_err(|e| e.to_string())?;

        conn.execute("BEGIN IMMEDIATE", [])
            .map_err(|e| format!("Failed to begin session event transaction: {}", e))?;

        let result = (|| -> Result<Vec<i64>, String> {
            let mut next_seq_by_session: HashMap<String, i64> = HashMap::new();
            for record in records {
                if next_seq_by_session.contains_key(&record.session_id) {
                    continue;
                }
                let next_seq = conn
                    .query_row(
                        "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE session_id = ?1",
                        params![record.session_id.as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|e| format!("Failed to allocate session event sequence: {}", e))?;
                next_seq_by_session.insert(record.session_id.clone(), next_seq);
            }

            let mut seqs = Vec::with_capacity(records.len());
            {
                let mut insert = conn
                    .prepare(
                        "INSERT INTO session_events (session_id, run_id, seq, event_type, payload_json, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .map_err(|e| format!("Failed to prepare session event insert: {}", e))?;

                for record in records {
                    let seq_ref =
                        next_seq_by_session
                            .get_mut(&record.session_id)
                            .ok_or_else(|| {
                                "Session event sequence allocation was missing".to_string()
                            })?;
                    let seq = *seq_ref;
                    *seq_ref += 1;

                    insert
                        .execute(params![
                            record.session_id.as_str(),
                            record.run_id.as_str(),
                            seq,
                            record.event_type.as_str(),
                            record.payload_json.as_str(),
                            now
                        ])
                        .map_err(|e| format!("Failed to append session event: {}", e))?;
                    seqs.push(seq);
                }
            }

            Ok(seqs)
        })();

        let seqs = match result {
            Ok(seqs) => seqs,
            Err(error) => {
                let _ = conn.execute("ROLLBACK", []);
                return Err(error);
            }
        };

        conn.execute("COMMIT", [])
            .map_err(|e| format!("Failed to commit session event transaction: {}", e))?;

        Ok(seqs)
    }

    pub fn list_session_events(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let after_seq = after_seq.unwrap_or(0);
        let limit = i64::from(limit.unwrap_or(500).clamp(1, 2_000));
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT session_id, run_id, seq, event_type, payload_json, created_at
                 FROM session_events
                 WHERE session_id = ?1 AND seq > ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )
            .map_err(|e| format!("Failed to prepare session event query: {}", e))?;
        let rows = stmt
            .query_map(params![session_id, after_seq, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| format!("Failed to query session events: {}", e))?;

        let mut events = Vec::new();
        for row in rows {
            let (session_id, run_id, seq, event_type, payload_json, created_at) =
                row.map_err(|e| format!("Failed to read session event row: {}", e))?;
            let payload =
                serde_json::from_str::<serde_json::Value>(&payload_json).map_err(|e| {
                    format!(
                        "Failed to parse session event payload for session {} seq {}: {}",
                        session_id, seq, e
                    )
                })?;
            events.push(SessionEventRecord {
                session_id,
                run_id,
                seq,
                event_type,
                payload,
                created_at,
            });
        }

        Ok(events)
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
            pending_inputs: Vec::new(),
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

    pub fn truncate_latest_conversation_turn(&self, session_id: &str) -> Result<u64, String> {
        let now = Self::now_ts();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("Failed to begin latest turn truncation: {}", e))?;

        let result = (|| -> Result<u64, String> {
            let truncate_from_rowid: Option<i64> = conn
                .query_row(
                    "SELECT rowid FROM messages
                     WHERE session_id = ?1 AND role = 'user' AND tool_call_id IS NULL
                     ORDER BY rowid DESC
                     LIMIT 1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("Failed to find latest conversation turn: {}", e))?;

            let Some(truncate_from_rowid) = truncate_from_rowid else {
                return Ok(0);
            };

            let deleted = conn
                .execute(
                    "DELETE FROM messages WHERE session_id = ?1 AND rowid >= ?2",
                    params![session_id, truncate_from_rowid],
                )
                .map_err(|e| format!("Failed to truncate latest conversation turn: {}", e))?;

            if deleted > 0 {
                conn.execute(
                    "UPDATE sessions SET latest_completed_run_id = NULL, updated_at = ?1 WHERE id = ?2",
                    params![now, session_id],
                )
                .map_err(|e| format!("Failed to update session after latest turn truncation: {}", e))?;
            }

            Ok(deleted as u64)
        })();

        match result {
            Ok(deleted) => {
                conn.execute_batch("COMMIT")
                    .map_err(|e| format!("Failed to commit latest turn truncation: {}", e))?;
                Ok(deleted)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn add_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
    ) -> Result<String, String> {
        self.add_message_full(
            session_id, role, content, None, None, None, None, None, None,
        )
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
            None,
        )
    }

    pub fn add_message_with_images_asset_refs_and_signature(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        images: Option<&[super::models::ImageData]>,
        asset_refs: Option<&[super::models::AssetRefData]>,
        thinking_signature: Option<&str>,
        prompt_prefix: Option<&str>,
        prompt_suffix: Option<&str>,
    ) -> Result<String, String> {
        let images_json = images
            .filter(|imgs| !imgs.is_empty())
            .map(|imgs| serde_json::to_string(imgs))
            .transpose()
            .map_err(|e| format!("Failed to serialize images: {}", e))?;
        let asset_refs_json = asset_refs
            .filter(|refs| !refs.is_empty())
            .map(|refs| serde_json::to_string(refs))
            .transpose()
            .map_err(|e| format!("Failed to serialize asset refs: {}", e))?;
        self.add_message_full_with_thinking(
            session_id,
            role,
            content,
            None,
            None,
            images_json.as_deref(),
            asset_refs_json.as_deref(),
            None,
            None,
            thinking_signature,
            None,
            prompt_prefix,
            prompt_suffix,
            None,
            None,
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
        self.add_message_with_thinking_and_order(
            session_id,
            role,
            content,
            thinking_content,
            thinking_duration,
            thinking_signature,
            response_id,
            response_request,
            None,
            None,
        )
    }

    pub fn add_message_with_thinking_and_order(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
    ) -> Result<String, String> {
        self.add_message_full_with_thinking(
            session_id,
            role,
            content,
            None,
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
            content_order,
            thinking_order,
        )
    }

    pub fn add_message_with_thinking_and_render_parts(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
        render_parts: &[AssistantRenderPart],
    ) -> Result<String, String> {
        self.add_message_full_with_thinking_and_render_parts(
            session_id,
            role,
            content,
            None,
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
            content_order,
            thinking_order,
            Some(render_parts),
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
        self.add_assistant_with_tool_calls_and_thinking_and_order(
            session_id,
            content,
            tool_calls,
            thinking_content,
            thinking_duration,
            thinking_signature,
            response_id,
            response_request,
            None,
            None,
        )
    }

    pub fn add_assistant_with_tool_calls_and_thinking_and_order(
        &self,
        session_id: &str,
        content: &str,
        tool_calls: &[ToolCallInfo],
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
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
            None,
            thinking_content,
            thinking_duration,
            thinking_signature,
            None,
            None,
            None,
            response_id,
            response_request,
            content_order,
            thinking_order,
        )
    }

    pub fn add_assistant_with_tool_calls_and_render_parts(
        &self,
        session_id: &str,
        content: &str,
        tool_calls: &[ToolCallInfo],
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
        render_parts: &[AssistantRenderPart],
    ) -> Result<String, String> {
        let tool_calls_json = serde_json::to_string(tool_calls)
            .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
        self.add_message_full_with_thinking_and_render_parts(
            session_id,
            MessageRole::Assistant,
            content,
            Some(&tool_calls_json),
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
            content_order,
            thinking_order,
            Some(render_parts),
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

    pub fn update_message_tool_calls_and_render_parts(
        &self,
        message_id: &str,
        tool_calls: &[ToolCallInfo],
        render_parts: &[AssistantRenderPart],
    ) -> Result<(), String> {
        let tool_calls_json = serde_json::to_string(tool_calls)
            .map_err(|e| format!("Failed to serialize tool_calls: {}", e))?;
        let render_parts = render_parts.to_vec();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let metadata_json: Option<String> = conn
            .query_row(
                "SELECT metadata_json FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("Failed to load message metadata: {}", e))?
            .flatten();
        let mut metadata: MessageMetadata = metadata_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| format!("Failed to parse message metadata: {}", e))?
            .unwrap_or_default();
        metadata.render_parts = Some(render_parts);
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|e| format!("Failed to serialize message metadata: {}", e))?;
        conn.execute(
            "UPDATE messages SET tool_calls = ?1, metadata_json = ?2 WHERE id = ?3",
            params![tool_calls_json, metadata_json, message_id],
        )
        .map_err(|e| {
            format!(
                "Failed to update tool_calls/render_parts for message '{}': {}",
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
            None,
        )
    }

    fn session_tool_results_dir(&self, session_id: &str) -> PathBuf {
        Self::session_tool_results_dir_for(&self.data_dir, session_id)
    }

    fn session_tool_results_dir_for(data_dir: &Path, session_id: &str) -> PathBuf {
        data_dir.join(TOOL_RESULTS_DIR).join(session_id)
    }

    pub fn rewrite_tool_result_for_storage(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> Result<String, String> {
        Self::rewrite_tool_result_for_storage_at(
            &self.data_dir,
            session_id,
            tool_call_id,
            tool_name,
            content,
        )
    }

    fn rewrite_tool_result_for_storage_at(
        data_dir: &Path,
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

        let dir = Self::session_tool_results_dir_for(data_dir, session_id);
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
        asset_refs_json: Option<&str>,
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
            asset_refs_json,
            None,
            None,
            None,
            knowledge_proposal,
            prompt_prefix,
            None,
            None,
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
        asset_refs_json: Option<&str>,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        knowledge_proposal: Option<&KnowledgeProposal>,
        prompt_prefix: Option<&str>,
        prompt_suffix: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
    ) -> Result<String, String> {
        self.add_message_full_with_thinking_and_render_parts(
            session_id,
            role,
            content,
            tool_calls_json,
            tool_call_id,
            images_json,
            asset_refs_json,
            thinking_content,
            thinking_duration,
            thinking_signature,
            knowledge_proposal,
            prompt_prefix,
            prompt_suffix,
            response_id,
            response_request,
            content_order,
            thinking_order,
            None,
        )
    }

    fn add_message_full_with_thinking_and_render_parts(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        tool_calls_json: Option<&str>,
        tool_call_id: Option<&str>,
        images_json: Option<&str>,
        asset_refs_json: Option<&str>,
        thinking_content: Option<&str>,
        thinking_duration: Option<u32>,
        thinking_signature: Option<&str>,
        knowledge_proposal: Option<&KnowledgeProposal>,
        prompt_prefix: Option<&str>,
        prompt_suffix: Option<&str>,
        response_id: Option<&str>,
        response_request: Option<&serde_json::Value>,
        content_order: Option<u32>,
        thinking_order: Option<u32>,
        render_parts: Option<&[AssistantRenderPart]>,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let now = Self::now_ts();
        let metadata_json = message_metadata_json(
            knowledge_proposal,
            response_id,
            response_request,
            content_order,
            thinking_order,
            render_parts,
        )?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, asset_refs, thinking_content, thinking_duration, thinking_signature, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![id, session_id, role.as_str(), content, now, prompt_prefix, prompt_suffix, tool_calls_json, tool_call_id, images_json, asset_refs_json, thinking_content, thinking_duration.map(|d| d as i64), thinking_signature, metadata_json],
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
        context_tokens: Option<u32>,
        context_limit: Option<u32>,
    ) -> Result<TokenUsage, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO token_usage (
                session_id,
                total_input_tokens,
                total_output_tokens,
                total_cache_read_tokens,
                total_cache_write_tokens,
                total_cost_usd,
                priced_rounds,
                last_context_tokens,
                last_context_limit
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, 0), COALESCE(?9, 0))
             ON CONFLICT(session_id) DO UPDATE SET
                total_input_tokens = total_input_tokens + ?2,
                total_output_tokens = total_output_tokens + ?3,
                total_cache_read_tokens = total_cache_read_tokens + ?4,
                total_cache_write_tokens = total_cache_write_tokens + ?5,
                total_cost_usd = total_cost_usd + ?6,
                priced_rounds = priced_rounds + ?7,
                last_context_tokens = CASE WHEN ?8 IS NULL THEN last_context_tokens ELSE ?8 END,
                last_context_limit = CASE WHEN ?9 IS NULL THEN last_context_limit ELSE ?9 END",
            params![
                session_id,
                input_tokens as i64,
                output_tokens as i64,
                cache_read_tokens as i64,
                cache_write_tokens as i64,
                cost_usd,
                priced_rounds as i64,
                context_tokens.map(|value| value as i64),
                context_limit.map(|value| value as i64),
            ],
        )
        .map_err(|e| format!("Failed to record token usage: {}", e))?;

        let (
            total_in,
            total_out,
            total_cr,
            total_cw,
            total_cost_usd,
            priced_rounds,
            last_context_tokens,
            last_context_limit,
        ) = conn
            .query_row(
                "SELECT
                    total_input_tokens,
                    total_output_tokens,
                    total_cache_read_tokens,
                    total_cache_write_tokens,
                    total_cost_usd,
                    priced_rounds,
                    last_context_tokens,
                    last_context_limit
                 FROM token_usage WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
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
            context_tokens: last_context_tokens as u32,
            context_limit: last_context_limit as u32,
        })
    }

    pub fn get_token_usage(&self, session_id: &str) -> Result<TokenUsage, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let result = conn.query_row(
            "SELECT
                total_input_tokens,
                total_output_tokens,
                total_cache_read_tokens,
                total_cache_write_tokens,
                total_cost_usd,
                priced_rounds,
                last_context_tokens,
                last_context_limit
             FROM token_usage WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        );

        match result {
            Ok((
                total_in,
                total_out,
                total_cr,
                total_cw,
                total_cost_usd,
                priced_rounds,
                last_context_tokens,
                last_context_limit,
            )) => Ok(TokenUsage {
                total_input_tokens: total_in as u64,
                total_output_tokens: total_out as u64,
                total_cache_read_tokens: total_cr as u64,
                total_cache_write_tokens: total_cw as u64,
                total_cost_usd,
                priced_rounds: priced_rounds as u64,
                context_tokens: last_context_tokens as u32,
                context_limit: last_context_limit as u32,
            }),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(TokenUsage {
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cache_read_tokens: 0,
                total_cache_write_tokens: 0,
                total_cost_usd: 0.0,
                priced_rounds: 0,
                context_tokens: 0,
                context_limit: 0,
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

        let prompt_messages = Self::get_messages_with_conn_filtered_static(&conn, session_id, true)
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                e
            })?;
        let _boundary_idx = prompt_messages
            .iter()
            .position(|message| message.id == keep_from_message_id)
            .ok_or_else(|| {
                let _ = conn.execute("ROLLBACK", []);
                format!(
                    "Compact boundary message is not included in prompt: {}",
                    keep_from_message_id
                )
            })?;
        let carried_prompt_prefix = prompt_messages.iter().find_map(|message| {
            if (message.role == MessageRole::User && message.tool_call_id.is_none())
                || is_context_handoff_message(message)
            {
                message
                    .prompt_prefix
                    .as_deref()
                    .filter(|prefix| !prefix.trim().is_empty())
                    .map(|prefix| prefix.to_string())
            } else {
                None
            }
        });
        let retained_user_ids = compact::select_recent_user_message_ids_for_compact_prompt(
            &prompt_messages,
            prompt_messages.len(),
            compact::compact_user_message_token_budget(),
        );

        conn.execute(
            "UPDATE messages
             SET include_in_prompt = 0
             WHERE session_id = ?1
               AND include_in_prompt = 1",
            params![session_id],
        )
        .map_err(|e| {
            let _ = conn.execute("ROLLBACK", []);
            format!("Failed to mark compacted messages: {}", e)
        })?;

        for message_id in &retained_user_ids {
            conn.execute(
                "UPDATE messages
                 SET include_in_prompt = 1
                 WHERE session_id = ?1
                   AND id = ?2
                   AND role = 'user'
                   AND tool_call_id IS NULL",
                params![session_id, message_id],
            )
            .map_err(|e| {
                let _ = conn.execute("ROLLBACK", []);
                format!(
                    "Failed to restore retained user message after compact: {}",
                    e
                )
            })?;
        }

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, asset_refs, thinking_content, thinking_duration, thinking_signature, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
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
        let asset_refs_select = if Self::table_has_column(conn, "messages", "asset_refs")
            .map_err(|e| format!("Failed to inspect messages.asset_refs: {}", e))?
        {
            "asset_refs"
        } else {
            "NULL AS asset_refs"
        };
        let query = if prompt_only {
            format!(
                "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, {asset_refs_select}, thinking_content, thinking_duration, thinking_signature, metadata_json
             FROM messages
             WHERE session_id = ?1 AND include_in_prompt = 1
             ORDER BY created_at ASC, rowid ASC"
            )
        } else {
            format!(
                "SELECT id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, {asset_refs_select}, thinking_content, thinking_duration, thinking_signature, metadata_json
             FROM messages
             WHERE session_id = ?1
             ORDER BY rowid ASC"
            )
        };

        let mut stmt = conn
            .prepare(&query)
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
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
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
                asset_refs_json,
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

            let asset_refs: Option<Vec<super::models::AssetRefData>> = asset_refs_json
                .as_deref()
                .map(|json| serde_json::from_str(json))
                .transpose()
                .map_err(|e| format!("Failed to parse asset refs: {}", e))?;

            let metadata: Option<MessageMetadata> = metadata_json
                .as_deref()
                .map(|json| serde_json::from_str(json))
                .transpose()
                .map_err(|e| format!("Failed to parse message metadata: {}", e))?;
            let (knowledge_proposal, response_id, content_order, thinking_order, render_parts) =
                metadata
                    .map(|value| {
                        (
                            value.knowledge_proposal,
                            value.response_id,
                            value.content_order,
                            value.thinking_order,
                            value.render_parts,
                        )
                    })
                    .unwrap_or((None, None, None, None, None));

            messages.push(ChatMessage {
                id,
                role,
                content,
                created_at,
                prompt_prefix,
                prompt_suffix,
                response_id,
                content_order,
                thinking_order,
                tool_calls,
                tool_call_id,
                images,
                asset_refs,
                thinking_content,
                thinking_duration: thinking_duration_raw.map(|d| d as u32),
                thinking_signature,
                knowledge_proposal,
                render_parts,
            });
        }
        if !prompt_only {
            for message in &mut messages {
                redact_context_handoff_for_display(message);
            }
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
    use super::{
        build_large_tool_result_message, PersistedToolResult, SessionStore,
        CHILD_SESSION_FORK_ERROR, CONTEXT_COMPACTED_DISPLAY_MARKER,
    };
    use crate::session::models::{
        ChatMessage, KnowledgeProposalStatus, MessageRole, TodoItem, ToolCallInfo,
    };
    use rusqlite::{params, Connection, OptionalExtension};
    use std::fs;
    use tempfile::tempdir;

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(()),
        )
        .optional()
        .expect("query sqlite master")
        .is_some()
    }

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
        assert!(SessionStore::table_has_column(&conn, "messages", "asset_refs").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
        assert!(
            SessionStore::table_has_column(&conn, "token_usage", "last_context_tokens").unwrap()
        );
        assert!(
            SessionStore::table_has_column(&conn, "token_usage", "last_context_limit").unwrap()
        );
        assert!(table_exists(&conn, "session_runs"));
        assert!(table_exists(&conn, "session_events"));
    }

    #[test]
    fn v18_database_migrates_forward_to_v19_without_pending_input_table() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        {
            let _store = SessionStore::new(dir.path()).expect("initialize latest store");
        }

        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch("PRAGMA user_version = 18;")
            .expect("simulate v18 schema");
        drop(conn);

        let _store = SessionStore::new(dir.path()).expect("migrate store");
        let conn = Connection::open(&db_path).expect("open migrated db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(table_exists(&conn, "session_runs"));
        assert!(table_exists(&conn, "session_events"));
    }

    #[test]
    fn token_usage_persists_latest_context_window() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Usage", None, None, "chat", None)
            .expect("create session");

        let usage = store
            .record_token_usage(&session_id, 100, 20, 10, 5, 0.0, 0, Some(135), Some(1000))
            .expect("record usage");
        assert_eq!(usage.context_tokens, 135);
        assert_eq!(usage.context_limit, 1000);

        let usage = store
            .record_token_usage(&session_id, 7, 3, 0, 0, 0.0, 0, None, None)
            .expect("record usage without context");
        assert_eq!(usage.total_input_tokens, 107);
        assert_eq!(usage.total_output_tokens, 23);
        assert_eq!(usage.context_tokens, 135);
        assert_eq!(usage.context_limit, 1000);

        let reloaded = store.get_token_usage(&session_id).expect("read usage");
        assert_eq!(reloaded.context_tokens, 135);
        assert_eq!(reloaded.context_limit, 1000);
    }

    #[test]
    fn fork_session_copies_root_session_data_and_tool_results() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Source", None, Some("workspace-1"), "chat", Some("dev"))
            .expect("create session");

        store
            .set_latest_completed_run_id(&session_id, Some("run-1"))
            .expect("set latest completed run");
        store
            .record_token_usage(&session_id, 100, 20, 10, 5, 1.25, 2, Some(512), Some(4096))
            .expect("record usage");
        store
            .update_todos(
                &session_id,
                Some("run-1"),
                &[TodoItem {
                    content: "Review copied session".to_string(),
                    status: "pending".to_string(),
                    priority: "medium".to_string(),
                }],
            )
            .expect("update todos");

        let source_tool_dir = store.session_tool_results_dir(&session_id);
        fs::create_dir_all(&source_tool_dir).expect("create tool dir");
        let source_tool_file = source_tool_dir.join("tool-a.txt");
        fs::write(&source_tool_file, "full tool output").expect("write tool output");
        let persisted_message = build_large_tool_result_message(&PersistedToolResult {
            filepath: source_tool_file.clone(),
            original_size: 16,
            preview: "full tool output".to_string(),
            has_more: false,
        });

        {
            let conn = store.conn.lock().expect("lock db");
            conn.execute(
                "INSERT INTO messages (
                    id, session_id, role, content, created_at, prompt_prefix, prompt_suffix,
                    tool_calls, tool_call_id, images, asset_refs, thinking_content,
                    thinking_duration, thinking_signature, metadata_json, include_in_prompt
                 )
                 VALUES (?1, ?2, 'user', 'hello', 10, 'prefix', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1)",
                params!["user-old", session_id],
            )
            .expect("insert user message");
            conn.execute(
                "INSERT INTO messages (
                    id, session_id, role, content, created_at, prompt_prefix, prompt_suffix,
                    tool_calls, tool_call_id, images, asset_refs, thinking_content,
                    thinking_duration, thinking_signature, metadata_json, include_in_prompt
                 )
                 VALUES (?1, ?2, 'tool', ?3, 11, NULL, NULL, NULL, 'tool-a', NULL, NULL, NULL, NULL, NULL, NULL, 0)",
                params!["tool-old", session_id, persisted_message],
            )
            .expect("insert tool message");
        }

        let fork_id = store
            .fork_session(&session_id, Some("Forked"))
            .expect("fork session");

        assert_ne!(fork_id, session_id);
        let detail = store.load_session(&fork_id).expect("load forked session");
        assert_eq!(detail.title, "Forked");
        assert_eq!(detail.agent_id.as_deref(), Some("dev"));
        assert_eq!(detail.session_type, "chat");
        assert_eq!(detail.parent_session_id, None);
        assert_eq!(detail.latest_completed_run_id.as_deref(), Some("run-1"));
        assert_eq!(detail.messages.len(), 2);
        assert_ne!(detail.messages[0].id, "user-old");
        assert_ne!(detail.messages[1].id, "tool-old");
        assert_eq!(detail.messages[0].content, "hello");

        let target_tool_file = store.session_tool_results_dir(&fork_id).join("tool-a.txt");
        assert_eq!(
            fs::read_to_string(&target_tool_file).expect("read copied tool output"),
            "full tool output"
        );
        assert!(detail.messages[1]
            .content
            .contains(&target_tool_file.display().to_string()));
        assert!(!detail.messages[1]
            .content
            .contains(&source_tool_file.display().to_string()));

        let prompt_messages = store
            .get_messages_for_prompt(&fork_id)
            .expect("load fork prompt messages");
        assert_eq!(prompt_messages.len(), 1);
        assert_eq!(prompt_messages[0].content, "hello");
        assert_eq!(prompt_messages[0].prompt_prefix.as_deref(), Some("prefix"));

        let usage = store.get_token_usage(&fork_id).expect("load copied usage");
        assert_eq!(usage.total_input_tokens, 100);
        assert_eq!(usage.total_output_tokens, 20);
        assert_eq!(usage.total_cost_usd, 1.25);
        assert_eq!(usage.priced_rounds, 2);
        assert_eq!(usage.context_tokens, 512);
        assert_eq!(usage.context_limit, 4096);

        let todos = store.get_todos(&fork_id).expect("load copied todos");
        assert_eq!(todos.latest_run_id.as_deref(), Some("run-1"));
        assert_eq!(todos.items.len(), 1);
        assert_eq!(todos.items[0].content, "Review copied session");
    }

    #[test]
    fn fork_session_rejects_child_sessions() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let parent_id = store
            .create_session("Parent", None, None, "chat", None)
            .expect("create parent");
        let child_id = store
            .create_session("Child", Some(&parent_id), None, "chat", Some("explorer"))
            .expect("create child");

        let error = store
            .fork_session(&child_id, Some("Child copy"))
            .expect_err("child fork should fail");
        assert_eq!(error, CHILD_SESSION_FORK_ERROR);
    }

    #[test]
    fn v15_database_migrates_message_render_orders() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");
        SessionStore::create_latest_schema(&conn).expect("create schema");

        conn.execute(
            "INSERT INTO sessions (id, title, session_type, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["s1", "Render Order", "chat", 10, 10],
        )
        .expect("insert session");
        let tool_calls_json = serde_json::to_string(&vec![ToolCallInfo {
            id: "tc-1".to_string(),
            name: "ask_user_question".to_string(),
            arguments: "{}".to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }])
        .expect("serialize tool calls");
        conn.execute(
            "INSERT INTO messages (
                id, session_id, role, content, created_at, tool_calls,
                thinking_content, include_in_prompt
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            params![
                "m1",
                "s1",
                "assistant",
                "https://x.com/",
                11,
                tool_calls_json,
                "thinking"
            ],
        )
        .expect("insert message");
        conn.pragma_update(None, "user_version", 15)
            .expect("set legacy version");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("s1").expect("load session");
        let message = detail.messages.first().expect("migrated message");

        assert_eq!(message.thinking_order, Some(1));
        assert_eq!(message.content_order, Some(2));
        assert_eq!(
            message
                .tool_calls
                .as_ref()
                .and_then(|tool_calls| tool_calls.first())
                .and_then(|tool_call| tool_call.order),
            Some(3)
        );

        let version: i32 = Connection::open(&db_path)
            .expect("open migrated db")
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
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
        assert!(SessionStore::table_has_column(&conn, "messages", "asset_refs").unwrap());
        assert!(SessionStore::table_has_column(&conn, "messages", "include_in_prompt").unwrap());
        assert!(
            SessionStore::table_has_column(&conn, "token_usage", "last_context_tokens").unwrap()
        );
        assert!(
            SessionStore::table_has_column(&conn, "token_usage", "last_context_limit").unwrap()
        );
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
            order: None,
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
    fn v13_database_is_migrated_forward_with_session_sync_tables() {
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
                latest_todo_run_id TEXT,
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
             PRAGMA user_version = 13;",
        )
        .expect("create v13 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, latest_completed_run_id, latest_todo_run_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        assert_eq!(detail.title, "Migrated Session");

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
        assert!(table_exists(&conn, "session_runs"));
        assert!(SessionStore::table_has_column(&conn, "session_runs", "status").unwrap());
        assert!(table_exists(&conn, "session_events"));
        assert!(SessionStore::table_has_column(&conn, "session_events", "payload_json").unwrap());
    }

    #[test]
    fn v14_migration_persists_oversized_tool_results() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");
        SessionStore::create_latest_schema(&conn).expect("create schema");
        conn.pragma_update(None, "user_version", 14)
            .expect("set v14");
        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, latest_completed_run_id, latest_todo_run_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");

        let tool_calls_json = serde_json::to_string(&vec![ToolCallInfo {
            id: "tc-large".to_string(),
            name: "bash".to_string(),
            arguments: "{}".to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }])
        .expect("serialize tool calls");
        let large_output = "A".repeat(31_000);
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt)
             VALUES (?1, ?2, 'assistant', '', 100, NULL, NULL, ?3, NULL, NULL, NULL, NULL, NULL, NULL, 1)",
            params!["assistant-1", "session-1", tool_calls_json],
        )
        .expect("insert assistant");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json, include_in_prompt)
             VALUES (?1, ?2, 'tool', ?3, 101, NULL, NULL, NULL, ?4, NULL, NULL, NULL, NULL, NULL, 1)",
            params!["tool-1", "session-1", large_output, "tc-large"],
        )
        .expect("insert tool");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let prompt_messages = store
            .get_messages_for_prompt("session-1")
            .expect("load prompt messages");
        let tool_message = prompt_messages
            .iter()
            .find(|message| message.role == MessageRole::Tool)
            .expect("tool message");
        assert!(tool_message.content.starts_with("<persisted-output>"));
        assert!(tool_message.content.contains("Full output saved to:"));

        let conn = Connection::open(&db_path).expect("reopen db");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read schema version");
        assert_eq!(version, SessionStore::SCHEMA_VERSION);
    }

    #[test]
    fn large_tool_result_saved_as_persisted_reference_in_prompt_history() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Tool Result Storage", None, None, "chat", None)
            .expect("create session");
        let large_output = "B".repeat(31_000);
        let stored_output = store
            .rewrite_tool_result_for_storage(&session_id, "tc-large", "bash", &large_output)
            .expect("rewrite large output");
        store
            .add_tool_result(&session_id, "tc-large", &stored_output)
            .expect("add tool result");

        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        assert_eq!(prompt_messages.len(), 1);
        assert!(prompt_messages[0].content.starts_with("<persisted-output>"));
        assert!(!prompt_messages[0].content.contains(&large_output));
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
    fn try_start_run_blocks_active_run_and_allows_after_terminal_status() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Run Lock", None, None, "chat", None)
            .expect("create session");

        store
            .try_start_run(&session_id, "run-1")
            .expect("start first run");
        let active = store
            .active_run_for_session(&session_id)
            .expect("load active run")
            .expect("active run");
        assert_eq!(active.run_id, "run-1");
        assert_eq!(active.status, "starting");

        let locked = store.try_start_run(&session_id, "run-2");
        assert!(locked.is_err());

        store
            .update_run_status("run-1", "done", None)
            .expect("finish first run");
        assert!(store
            .active_run_for_session(&session_id)
            .expect("load active run")
            .is_none());
        store
            .try_start_run(&session_id, "run-2")
            .expect("start second run");
    }

    #[test]
    fn session_id_for_run_returns_run_owner() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Run Owner", None, None, "chat", None)
            .expect("create session");

        assert_eq!(
            store
                .session_id_for_run("missing-run")
                .expect("query missing run"),
            None
        );

        store
            .try_start_run(&session_id, "run-1")
            .expect("start run");

        assert_eq!(
            store.session_id_for_run("run-1").expect("query run owner"),
            Some(session_id)
        );
    }

    #[test]
    fn active_descendant_runs_returns_active_child_tree_runs() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let parent_id = store
            .create_session("Parent", None, None, "chat", None)
            .expect("create parent");
        let child_id = store
            .create_session("Child", Some(&parent_id), None, "chat", None)
            .expect("create child");
        let grandchild_id = store
            .create_session("Grandchild", Some(&child_id), None, "chat", None)
            .expect("create grandchild");
        let sibling_id = store
            .create_session("Sibling", Some(&parent_id), None, "chat", None)
            .expect("create sibling");
        let unrelated_id = store
            .create_session("Unrelated", None, None, "chat", None)
            .expect("create unrelated");

        store
            .try_start_run(&parent_id, "run-parent")
            .expect("start parent run");
        store
            .try_start_run(&child_id, "run-child")
            .expect("start child run");
        store
            .try_start_run(&grandchild_id, "run-grandchild")
            .expect("start grandchild run");
        store
            .try_start_run(&sibling_id, "run-sibling")
            .expect("start sibling run");
        store
            .update_run_status("run-sibling", "done", None)
            .expect("finish sibling run");
        store
            .try_start_run(&unrelated_id, "run-unrelated")
            .expect("start unrelated run");

        let mut runs = store
            .active_descendant_runs(&parent_id)
            .expect("query active descendants")
            .into_iter()
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        runs.sort();

        assert_eq!(runs, vec!["run-child", "run-grandchild"]);
    }

    #[test]
    fn terminal_run_status_is_not_overwritten_by_late_nonterminal_update() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Run Status", None, None, "chat", None)
            .expect("create session");

        store
            .try_start_run(&session_id, "run-1")
            .expect("start run");
        store
            .update_run_status("run-1", "done", None)
            .expect("mark done");
        store
            .update_run_status("run-1", "cancelling", None)
            .expect("ignore late cancelling");

        let conn = store.conn.lock().expect("lock store connection");
        let status: String = conn
            .query_row(
                "SELECT status FROM session_runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .expect("read run status");
        assert_eq!(status, "done");
    }

    #[test]
    fn cancelling_run_status_is_not_overwritten_by_late_running_update() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Run Cancelling", None, None, "chat", None)
            .expect("create session");

        store
            .try_start_run(&session_id, "run-1")
            .expect("start run");
        store
            .update_run_status("run-1", "cancelling", None)
            .expect("mark cancelling");
        store
            .update_run_status("run-1", "running", None)
            .expect("ignore late running");

        let conn = store.conn.lock().expect("lock store connection");
        let status: String = conn
            .query_row(
                "SELECT status FROM session_runs WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .expect("read run status");
        assert_eq!(status, "cancelling");
    }

    #[test]
    fn session_events_allocate_monotonic_sequence() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Event Log", None, None, "chat", None)
            .expect("create session");

        store
            .try_start_run(&session_id, "run-1")
            .expect("start run");
        let first_seq = store
            .append_session_event(
                &session_id,
                "run-1",
                "runStart",
                r#"{"type":"runStart","sessionId":"session"}"#,
            )
            .expect("append first event");
        let second_seq = store
            .append_session_event(
                &session_id,
                "run-1",
                "textDelta",
                r#"{"type":"textDelta","sessionId":"session","text":"hello"}"#,
            )
            .expect("append second event");

        assert_eq!(first_seq, 1);
        assert_eq!(second_seq, 2);

        let events = store
            .list_session_events(&session_id, Some(0), Some(10))
            .expect("list events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].event_type, "runStart");
        assert_eq!(events[1].seq, 2);
        assert_eq!(events[1].payload["text"].as_str(), Some("hello"));

        let tail = store
            .list_session_events(&session_id, Some(1), Some(10))
            .expect("list tail");
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, 2);
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
    fn truncate_latest_conversation_turn_removes_latest_user_round_only() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Latest Turn", None, None, "chat", None)
            .expect("create session");

        store
            .add_message(&session_id, MessageRole::User, "old user")
            .expect("insert old user");
        store
            .add_message(&session_id, MessageRole::Assistant, "old assistant")
            .expect("insert old assistant");
        store
            .add_message(&session_id, MessageRole::User, "latest user")
            .expect("insert latest user");
        store
            .add_message(&session_id, MessageRole::Assistant, "latest assistant")
            .expect("insert latest assistant");
        store
            .set_latest_completed_run_id(&session_id, Some("run-latest"))
            .expect("set latest run");

        let deleted = store
            .truncate_latest_conversation_turn(&session_id)
            .expect("truncate latest turn");
        assert_eq!(deleted, 2);

        let detail = store.load_session(&session_id).expect("load session");
        assert_eq!(detail.latest_completed_run_id, None);
        let contents = detail
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>();
        assert_eq!(contents, vec!["old user", "old assistant"]);
    }

    #[test]
    fn truncate_latest_conversation_turn_returns_zero_without_user_message() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Empty", None, None, "chat", None)
            .expect("create session");

        let deleted = store
            .truncate_latest_conversation_turn(&session_id)
            .expect("truncate latest turn");
        assert_eq!(deleted, 0);
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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
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
                latest_user_id,
                latest_assistant_id,
                "handoff-1"
            ]
        );
        assert_eq!(all_messages[4].content, CONTEXT_COMPACTED_DISPLAY_MARKER);
        assert_eq!(prompt_messages.len(), 3);
        assert_eq!(prompt_messages[0].id, old_user_id);
        assert_eq!(prompt_messages[1].id, "handoff-1");
        assert_eq!(prompt_messages[2].content, "最新需求");
        assert_eq!(
            prompt_messages[0].prompt_prefix.as_deref(),
            Some("<system-reminder>\nEnv\n</system-reminder>")
        );
        assert_eq!(prompt_messages[2].prompt_prefix, None);
        assert_eq!(
            store
                .first_user_message_id(&session_id)
                .expect("first prompt user"),
            Some(old_user_id.to_string())
        );
    }

    #[test]
    fn compact_messages_caps_old_user_prompt_history_and_carries_prefix() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact User Budget Test", None, None, "chat", None)
            .expect("create session");

        let old_user_id = "old-user";
        let latest_user_id = "latest-user";
        {
            let conn = store.conn.lock().expect("lock store connection");
            let oversized_user_content = "历史需求".repeat(30_000);
            for (id, role, content, created_at, prompt_prefix) in [
                (
                    old_user_id,
                    "user",
                    oversized_user_content.as_str(),
                    100i64,
                    Some("<system-reminder>\nEnv\n</system-reminder>"),
                ),
                ("old-assistant", "assistant", "旧回答", 101i64, None),
                (latest_user_id, "user", "最新需求", 102i64, None),
                ("latest-assistant", "assistant", "最新回答", 103i64, None),
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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };

        let (count_before, count_after) = store
            .compact_messages(&session_id, &summary_msg, latest_user_id)
            .expect("compact messages");
        assert_eq!(count_before, 4);
        assert_eq!(count_after, 2);

        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        let prompt_ids = prompt_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(prompt_ids, vec!["handoff-1", latest_user_id]);
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

    #[test]
    fn compact_marker_displays_after_assistant_tail_when_compacting_after_turn() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact Marker Order Test", None, None, "chat", None)
            .expect("create session");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-1", "user", "测试 unity_execute", 100i64),
                ("assistant-tools", "assistant", "已调用工具", 101i64),
                ("assistant-final", "assistant", "测试完成", 102i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };

        store
            .compact_messages(&session_id, &summary_msg, "assistant-final")
            .expect("compact messages");

        let all_messages = store.get_messages(&session_id).expect("load all messages");
        let all_ids = all_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            all_ids,
            vec!["user-1", "assistant-tools", "assistant-final", "handoff-1"]
        );
        assert_eq!(
            all_messages.last().map(|message| message.content.as_str()),
            Some(CONTEXT_COMPACTED_DISPLAY_MARKER)
        );
    }

    #[test]
    fn compact_markers_follow_message_insert_order_across_multiple_compacts() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact Marker Insert Order Test", None, None, "chat", None)
            .expect("create session");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-1", "user", "第一轮需求", 100i64),
                ("assistant-1", "assistant", "第一轮回答", 101i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
                )
                .expect("insert first turn message");
            }
        }

        let first_handoff = ChatMessage {
            id: "handoff-1".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第一次交接".to_string(),
            created_at: 101,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &first_handoff, "assistant-1")
            .expect("first compact");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-2", "user", "你好", 102i64),
                ("assistant-2", "assistant", "你好，我在。", 103i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
                )
                .expect("insert second turn message");
            }
        }

        let second_handoff = ChatMessage {
            id: "handoff-2".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第二次交接".to_string(),
            created_at: 103,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &second_handoff, "assistant-2")
            .expect("second compact");

        let all_messages = store.get_messages(&session_id).expect("load all messages");
        let all_ids = all_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            all_ids,
            vec![
                "user-1",
                "assistant-1",
                "handoff-1",
                "user-2",
                "assistant-2",
                "handoff-2"
            ]
        );
        assert_eq!(all_messages[2].content, CONTEXT_COMPACTED_DISPLAY_MARKER);
        assert_eq!(all_messages[5].content, CONTEXT_COMPACTED_DISPLAY_MARKER);
    }

    #[test]
    fn compact_messages_excludes_previous_handoff_on_later_compact() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact Twice Test", None, None, "chat", None)
            .expect("create session");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-1", "user", "第一轮需求", 100i64),
                ("assistant-1", "assistant", "第一轮回答", 101i64),
                ("user-2", "user", "第二轮需求", 102i64),
                ("assistant-2", "assistant", "第二轮回答", 103i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
                )
                .expect("insert message");
            }
        }

        let first_handoff = ChatMessage {
            id: "handoff-1".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第一次交接".to_string(),
            created_at: 101,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &first_handoff, "user-2")
            .expect("first compact");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-3", "user", "第三轮需求", 104i64),
                ("assistant-3", "assistant", "第三轮回答", 105i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
                )
                .expect("insert later message");
            }
        }

        let second_handoff = ChatMessage {
            id: "handoff-2".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第二次交接".to_string(),
            created_at: 103,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &second_handoff, "user-3")
            .expect("second compact");

        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        let prompt_ids = prompt_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();

        assert!(!prompt_ids.contains(&"handoff-1"));
        assert!(prompt_ids.contains(&"handoff-2"));
        assert!(prompt_ids.contains(&"user-1"));
        assert!(prompt_ids.contains(&"user-2"));
        assert!(prompt_ids.contains(&"user-3"));
        assert!(!prompt_ids.contains(&"assistant-2"));
    }

    #[test]
    fn compact_messages_replaces_previous_handoff_when_boundary_is_handoff() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let session_id = store
            .create_session("Compact Handoff Boundary Test", None, None, "chat", None)
            .expect("create session");

        {
            let conn = store.conn.lock().expect("lock store connection");
            for (id, role, content, created_at) in [
                ("user-1", "user", "第一轮需求", 100i64),
                ("assistant-1", "assistant", "第一轮回答", 101i64),
                ("user-2", "user", "第二轮需求", 102i64),
                ("assistant-2", "assistant", "第二轮回答", 103i64),
            ] {
                conn.execute(
                    "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, tool_calls, tool_call_id, images, thinking_content, thinking_duration, thinking_signature, metadata_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
                    params![id, session_id, role, content, created_at],
                )
                .expect("insert message");
            }
        }

        let first_handoff = ChatMessage {
            id: "handoff-1".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第一次交接".to_string(),
            created_at: 101,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &first_handoff, "user-2")
            .expect("first compact");

        let second_handoff = ChatMessage {
            id: "handoff-2".to_string(),
            role: MessageRole::Assistant,
            content: "## Context Handoff\n\n第二次交接".to_string(),
            created_at: 102,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        };
        store
            .compact_messages(&session_id, &second_handoff, "handoff-1")
            .expect("second compact");

        let all_messages = store.get_messages(&session_id).expect("load all messages");
        let prompt_messages = store
            .get_messages_for_prompt(&session_id)
            .expect("load prompt messages");
        let all_ids = all_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        let prompt_ids = prompt_messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();

        assert!(all_ids.contains(&"handoff-1"));
        assert!(all_ids.contains(&"handoff-2"));
        assert!(!prompt_ids.contains(&"handoff-1"));
        assert!(prompt_ids.contains(&"handoff-2"));
        assert_eq!(
            prompt_ids
                .iter()
                .filter(|id| id.starts_with("handoff-"))
                .count(),
            1
        );
    }
}
