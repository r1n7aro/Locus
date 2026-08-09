//! Persistent, bounded corpus of failed agent tool calls.
//!
//! Each entry keeps both the failure payload and an exact pointer back to the
//! session message/tool call that produced it. The JSON document is capped by
//! entry count and rewritten oldest-to-newest, so it remains a bounded FIFO
//! corpus suitable for later tool-quality analysis.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const FAILURE_LOG_FILE_NAME: &str = "tool-call-failures.json";
pub const MAX_FAILURE_ENTRIES: usize = 100;
const FAILURE_LOG_SCHEMA_VERSION: u32 = 1;

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn write_lock() -> &'static Mutex<()> {
    WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone)]
pub struct FailureLocationInput {
    pub session_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub tool_call_id: String,
    pub tool_call_order: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ParentLocationInput {
    pub session_id: String,
    pub run_id: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone)]
pub struct FailureInput {
    pub agent_id: String,
    pub working_directory: String,
    pub execution_mode: String,
    pub declared_tool_name: String,
    pub tool_name: String,
    pub raw_arguments: String,
    pub arguments: serde_json::Value,
    pub error_output: String,
    pub location: FailureLocationInput,
    pub parent_location: Option<ParentLocationInput>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureLog {
    schema_version: u32,
    max_entries: usize,
    entries: Vec<FailureEntry>,
}

impl Default for FailureLog {
    fn default() -> Self {
        Self {
            schema_version: FAILURE_LOG_SCHEMA_VERSION,
            max_entries: MAX_FAILURE_ENTRIES,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureEntry {
    id: String,
    recorded_at: String,
    recorded_at_ms: i64,
    agent_id: String,
    working_directory: String,
    execution_mode: String,
    declared_tool_name: String,
    tool_name: String,
    raw_arguments: String,
    arguments: serde_json::Value,
    error_output: String,
    location: FailureLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_location: Option<ParentLocation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureLocation {
    session_id: String,
    run_id: String,
    assistant_message_id: String,
    tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_order: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentLocation {
    session_id: String,
    run_id: String,
    tool_call_id: String,
}

impl From<FailureLocationInput> for FailureLocation {
    fn from(value: FailureLocationInput) -> Self {
        Self {
            session_id: value.session_id,
            run_id: value.run_id,
            assistant_message_id: value.assistant_message_id,
            tool_call_id: value.tool_call_id,
            tool_call_order: value.tool_call_order,
        }
    }
}

impl From<ParentLocationInput> for ParentLocation {
    fn from(value: ParentLocationInput) -> Self {
        Self {
            session_id: value.session_id,
            run_id: value.run_id,
            tool_call_id: value.tool_call_id,
        }
    }
}

fn default_log_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?
        .join("logs")
        .join(FAILURE_LOG_FILE_NAME))
}

/// Best-effort failure recording. Diagnostic persistence must never replace
/// the original tool result with a new error.
pub async fn record(input: FailureInput) {
    let _guard = write_lock().lock().await;
    let path = match default_log_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[ToolFailureLog] failed to resolve log path: {error}");
            return;
        }
    };
    if let Err(error) = append_to_path(&path, input, MAX_FAILURE_ENTRIES) {
        eprintln!(
            "[ToolFailureLog] failed to persist failure corpus at {}: {error}",
            path.display()
        );
    }
}

fn append_to_path(path: &Path, input: FailureInput, limit: usize) -> Result<(), String> {
    let mut log = load_or_recover(path)?;
    let now = chrono::Utc::now();
    log.entries.push(FailureEntry {
        id: uuid::Uuid::new_v4().to_string(),
        recorded_at: now.to_rfc3339(),
        recorded_at_ms: now.timestamp_millis(),
        agent_id: input.agent_id,
        working_directory: input.working_directory,
        execution_mode: input.execution_mode,
        declared_tool_name: input.declared_tool_name,
        tool_name: input.tool_name,
        raw_arguments: input.raw_arguments,
        arguments: input.arguments,
        error_output: input.error_output,
        location: input.location.into(),
        parent_location: input.parent_location.map(Into::into),
    });

    if limit == 0 {
        log.entries.clear();
    } else if log.entries.len() > limit {
        let remove = log.entries.len() - limit;
        log.entries.drain(..remove);
    }
    log.schema_version = FAILURE_LOG_SCHEMA_VERSION;
    log.max_entries = limit;
    write_atomic(path, &log)
}

fn load_or_recover(path: &Path) -> Result<FailureLog, String> {
    if !path.is_file() {
        return Ok(FailureLog::default());
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    match serde_json::from_slice::<FailureLog>(&bytes) {
        Ok(log) if log.schema_version == FAILURE_LOG_SCHEMA_VERSION => Ok(log),
        Ok(log) => Err(format!(
            "unsupported schemaVersion {} in {} (expected {})",
            log.schema_version,
            path.display(),
            FAILURE_LOG_SCHEMA_VERSION
        )),
        Err(error) => {
            let archive = corrupt_archive_path(path);
            std::fs::rename(path, &archive).map_err(|rename_error| {
                format!(
                    "invalid JSON ({error}); failed to preserve it as {}: {rename_error}",
                    archive.display()
                )
            })?;
            eprintln!(
                "[ToolFailureLog] preserved malformed failure log as {}: {error}",
                archive.display()
            );
            Ok(FailureLog::default())
        }
    }
}

fn corrupt_archive_path(path: &Path) -> PathBuf {
    let file_name = format!(
        "tool-call-failures.corrupt-{}-{}.json",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        uuid::Uuid::new_v4()
    );
    path.with_file_name(file_name)
}

fn write_atomic(path: &Path, log: &FailureLog) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("failure-log path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(log)
        .map_err(|error| format!("failed to serialize failure log: {error}"))?;
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        FAILURE_LOG_FILE_NAME,
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temp, bytes)
        .map_err(|error| format!("failed to write {}: {error}", temp.display()))?;

    #[cfg(target_os = "windows")]
    let replace_result = {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        use windows_core::PCWSTR;

        let temp_wide = temp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let path_wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(temp_wide.as_ptr()),
                PCWSTR(path_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| error.to_string())
    };

    #[cfg(not(target_os = "windows"))]
    let replace_result = std::fs::rename(&temp, path).map_err(|error| error.to_string());

    if let Err(error) = replace_result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("failed to replace {}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(index: usize) -> FailureInput {
        FailureInput {
            agent_id: "agent-main".to_string(),
            working_directory: "C:/Project".to_string(),
            execution_mode: "foreground".to_string(),
            declared_tool_name: "tool_call".to_string(),
            tool_name: "unity_execute".to_string(),
            raw_arguments: format!(r#"{{"code":"Run({index})"}}"#),
            arguments: serde_json::json!({ "code": format!("Run({index})") }),
            error_output: format!("failure-{index}"),
            location: FailureLocationInput {
                session_id: "session-1".to_string(),
                run_id: "run-2".to_string(),
                assistant_message_id: "message-3".to_string(),
                tool_call_id: format!("call-{index}"),
                tool_call_order: Some(index as u32),
            },
            parent_location: Some(ParentLocationInput {
                session_id: "parent-session".to_string(),
                run_id: "parent-run".to_string(),
                tool_call_id: "parent-call".to_string(),
            }),
        }
    }

    #[test]
    fn failure_log_preserves_payload_and_session_position() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(FAILURE_LOG_FILE_NAME);
        append_to_path(&path, input(7), 10).expect("append");

        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(value["schemaVersion"], FAILURE_LOG_SCHEMA_VERSION);
        assert_eq!(value["maxEntries"], 10);
        assert_eq!(value["entries"][0]["declaredToolName"], "tool_call");
        assert_eq!(value["entries"][0]["toolName"], "unity_execute");
        assert_eq!(value["entries"][0]["arguments"]["code"], "Run(7)");
        assert_eq!(value["entries"][0]["errorOutput"], "failure-7");
        assert_eq!(value["entries"][0]["location"]["sessionId"], "session-1");
        assert_eq!(
            value["entries"][0]["location"]["assistantMessageId"],
            "message-3"
        );
        assert_eq!(value["entries"][0]["location"]["toolCallId"], "call-7");
        assert_eq!(
            value["entries"][0]["parentLocation"]["sessionId"],
            "parent-session"
        );
    }

    #[test]
    fn failure_log_evicts_oldest_entries_fifo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(FAILURE_LOG_FILE_NAME);
        for index in 0..5 {
            append_to_path(&path, input(index), 3).expect("append");
        }

        let log: FailureLog =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        let outputs: Vec<&str> = log
            .entries
            .iter()
            .map(|entry| entry.error_output.as_str())
            .collect();
        assert_eq!(outputs, vec!["failure-2", "failure-3", "failure-4"]);
        assert_eq!(log.max_entries, 3);
    }

    #[test]
    fn malformed_log_is_preserved_before_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(FAILURE_LOG_FILE_NAME);
        std::fs::write(&path, b"{broken").expect("seed malformed log");

        append_to_path(&path, input(1), 3).expect("recover and append");

        let recovered: FailureLog =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(recovered.entries.len(), 1);
        let archives: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("tool-call-failures.corrupt-"))
            })
            .collect();
        assert_eq!(archives.len(), 1);
        assert_eq!(
            std::fs::read(&archives[0]).expect("read archive"),
            b"{broken"
        );
    }
}
