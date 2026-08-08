//! Persistent corpus of hot-reload failures.
//!
//! This log is deliberately separate from `locus.log`: every entry carries the
//! exact baseline/current source pair needed to turn a real-world cold verdict
//! into a regression test later. The JSON document is capped by entry count and
//! rewritten oldest-to-newest, so it remains a bounded FIFO corpus.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub const FAILURE_LOG_FILE_NAME: &str = "hot-reload-failures.json";
pub const MAX_FAILURE_ENTRIES: usize = 100;
const FAILURE_LOG_SCHEMA_VERSION: u32 = 1;

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn write_lock() -> &'static Mutex<()> {
    WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone)]
pub struct FailureFileInput {
    pub path: String,
    pub baseline_text: String,
    pub edited_text: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FailureInput {
    pub project_path: String,
    pub domain_generation: Option<String>,
    pub stage: String,
    pub reason: String,
    pub files: Vec<FailureFileInput>,
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
    project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain_generation: Option<String>,
    stage: String,
    reason: String,
    files: Vec<FailureFileEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureFileEntry {
    path: String,
    baseline_text: String,
    edited_text: String,
    reasons: Vec<String>,
}

impl From<FailureFileInput> for FailureFileEntry {
    fn from(value: FailureFileInput) -> Self {
        Self {
            path: value.path,
            baseline_text: value.baseline_text,
            edited_text: value.edited_text,
            reasons: value.reasons,
        }
    }
}

fn default_log_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?
        .join("logs")
        .join(FAILURE_LOG_FILE_NAME))
}

/// Best-effort failure recording. Diagnostic persistence must never replace
/// the original hot-reload result, so errors are reported to the unified log
/// and swallowed here.
pub async fn record(input: FailureInput) {
    let _guard = write_lock().lock().await;
    let path = match default_log_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[HotReload] failed to resolve failure-log path: {error}");
            return;
        }
    };
    if let Err(error) = append_to_path(&path, input, MAX_FAILURE_ENTRIES) {
        eprintln!(
            "[HotReload] failed to persist failure corpus at {}: {error}",
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
        project_path: input.project_path,
        domain_generation: input.domain_generation,
        stage: input.stage,
        reason: input.reason,
        files: input.files.into_iter().map(Into::into).collect(),
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
                "[HotReload] preserved malformed failure log as {}: {error}",
                archive.display()
            );
            Ok(FailureLog::default())
        }
    }
}

fn corrupt_archive_path(path: &Path) -> PathBuf {
    let file_name = format!(
        "hot-reload-failures.corrupt-{}-{}.json",
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
    if let Err(error) = std::fs::rename(&temp, path) {
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
            project_path: "C:/Project".to_string(),
            domain_generation: Some("domain-1".to_string()),
            stage: "classification".to_string(),
            reason: format!("failure-{index}"),
            files: vec![FailureFileInput {
                path: format!("C:/Project/Assets/Test{index}.cs"),
                baseline_text: format!("class Test{index} {{ int Value => 1; }}"),
                edited_text: format!("class Test{index} {{ int Value => 2; }}"),
                reasons: vec![format!("reason-{index}")],
            }],
        }
    }

    #[test]
    fn failure_log_preserves_exact_edit_payload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(FAILURE_LOG_FILE_NAME);
        append_to_path(&path, input(7), 10).expect("append");

        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
        assert_eq!(value["schemaVersion"], FAILURE_LOG_SCHEMA_VERSION);
        assert_eq!(value["maxEntries"], 10);
        assert_eq!(value["entries"][0]["stage"], "classification");
        assert_eq!(value["entries"][0]["reason"], "failure-7");
        assert_eq!(
            value["entries"][0]["files"][0]["baselineText"],
            "class Test7 { int Value => 1; }"
        );
        assert_eq!(
            value["entries"][0]["files"][0]["editedText"],
            "class Test7 { int Value => 2; }"
        );
        assert_eq!(value["entries"][0]["files"][0]["reasons"][0], "reason-7");
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
        let reasons: Vec<&str> = log
            .entries
            .iter()
            .map(|entry| entry.reason.as_str())
            .collect();
        assert_eq!(reasons, vec!["failure-2", "failure-3", "failure-4"]);
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
                    .is_some_and(|name| name.starts_with("hot-reload-failures.corrupt-"))
            })
            .collect();
        assert_eq!(archives.len(), 1);
        assert_eq!(
            std::fs::read(&archives[0]).expect("read archive"),
            b"{broken"
        );
    }
}
