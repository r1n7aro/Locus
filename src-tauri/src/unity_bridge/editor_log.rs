use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const LOG_SESSION_MTIME_SLACK_MS: u64 = 5_000;
const SAFE_MODE_LOG_MARKER: &str = "Safe Mode: Only loading a subset of assemblies";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedEditorLogState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub source: String,
    pub safe_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_mode_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at_ms: Option<u64>,
}

impl Default for ObservedEditorLogState {
    fn default() -> Self {
        Self {
            path: None,
            source: "unavailable".to_string(),
            safe_mode: false,
            safe_mode_source: None,
            modified_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorLogEntry {
    pub level: String,
    pub message: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct EditorLogRead {
    pub path: String,
    pub entries: Vec<EditorLogEntry>,
    pub matched_count: usize,
    pub unique_count: usize,
    pub truncated: bool,
}

pub fn observe(
    project_path: &str,
    process_id: Option<u32>,
    process_created_at_ms: Option<u64>,
    allow_log_safe_mode_fallback: bool,
) -> ObservedEditorLogState {
    let (path, source) = resolve_editor_log_path(project_path, process_id)
        .map(|(path, source)| (Some(path), source))
        .unwrap_or((None, "unavailable"));
    let modified_at_ms = path.as_deref().and_then(file_modified_at_ms);

    if let Some(process_id) = process_id {
        if let Some(title) = super::dialog::main_window_title(process_id) {
            let safe_mode = window_title_indicates_safe_mode(&title);
            return ObservedEditorLogState {
                path: path.map(|value| value.display().to_string()),
                source: source.to_string(),
                safe_mode,
                safe_mode_source: safe_mode.then(|| "window_title".to_string()),
                modified_at_ms,
            };
        }
    }

    let safe_mode = allow_log_safe_mode_fallback
        && path.as_deref().is_some_and(|path| {
            log_belongs_to_process_session(path, process_created_at_ms, modified_at_ms)
                && read_tail(path, MAX_TAIL_BYTES)
                    .map(|tail| tail.contains(SAFE_MODE_LOG_MARKER))
                    .unwrap_or(false)
        });

    ObservedEditorLogState {
        path: path.map(|value| value.display().to_string()),
        source: source.to_string(),
        safe_mode,
        safe_mode_source: safe_mode.then(|| "editor_log".to_string()),
        modified_at_ms,
    }
}

pub fn read_console_entries(
    project_path: &str,
    process_id: Option<u32>,
    requested_levels: &[String],
    limit: usize,
) -> Result<EditorLogRead, String> {
    let (path, _) = resolve_editor_log_path(project_path, process_id).ok_or_else(|| {
        format!("Could not locate the Unity Editor log for project {project_path}")
    })?;
    let raw = read_tail(&path, MAX_TAIL_BYTES)?;
    let normalized_levels = requested_levels
        .iter()
        .map(|level| level.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let matches_level = |level: &str| {
        normalized_levels.is_empty() || normalized_levels.iter().any(|candidate| candidate == level)
    };

    let mut groups = Vec::<EditorLogEntry>::new();
    let mut indexes = HashMap::<(String, String), usize>::new();
    let mut matched_count = 0usize;
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(level) = classify_log_line(line) else {
            continue;
        };
        if !matches_level(level) {
            continue;
        }
        matched_count = matched_count.saturating_add(1);
        let key = (level.to_string(), line.to_string());
        if let Some(index) = indexes.get(&key).copied() {
            groups[index].count = groups[index].count.saturating_add(1);
            continue;
        }
        indexes.insert(key, groups.len());
        groups.push(EditorLogEntry {
            level: level.to_string(),
            message: line.to_string(),
            count: 1,
        });
    }

    let unique_count = groups.len();
    let normalized_limit = limit.clamp(1, 200);
    let start = unique_count.saturating_sub(normalized_limit);
    let entries = groups.into_iter().skip(start).collect();
    Ok(EditorLogRead {
        path: path.display().to_string(),
        entries,
        matched_count,
        unique_count,
        truncated: unique_count > normalized_limit,
    })
}

pub fn recent_error_lines(
    project_path: &str,
    process_id: Option<u32>,
    limit: usize,
) -> Vec<String> {
    read_console_entries(project_path, process_id, &["error".to_string()], limit)
        .map(|read| {
            read.entries
                .into_iter()
                .map(|entry| entry.message)
                .collect()
        })
        .unwrap_or_default()
}

pub fn resolve_editor_log_path(
    project_path: &str,
    process_id: Option<u32>,
) -> Option<(PathBuf, &'static str)> {
    if let Some(explicit) = process_id
        .and_then(super::process::explicit_editor_log_path)
        .map(|path| resolve_log_path(project_path, path))
    {
        if explicit.is_file() {
            return Some((explicit, "command_line"));
        }
    }

    let project_root = Path::new(super::strip_extended_path_prefix(project_path));
    for candidate in [
        project_root.join("Logs").join("Editor.log"),
        project_root.join("Editor.log"),
    ] {
        if candidate.is_file() {
            return Some((candidate, "project"));
        }
    }

    platform_default_editor_log_path()
        .filter(|candidate| candidate.is_file())
        .map(|candidate| (candidate, "platform_default"))
}

fn resolve_log_path(project_path: &str, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        Path::new(super::strip_extended_path_prefix(project_path)).join(path)
    }
}

fn platform_default_editor_log_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Unity").join("Editor").join("Editor.log"));
    }
    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| root.join("Library/Logs/Unity/Editor.log"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| root.join(".config/unity3d/Editor.log"));
    }
    #[allow(unreachable_code)]
    None
}

fn log_belongs_to_process_session(
    path: &Path,
    process_created_at_ms: Option<u64>,
    modified_at_ms: Option<u64>,
) -> bool {
    match (process_created_at_ms, modified_at_ms) {
        (Some(created), Some(modified)) => {
            modified.saturating_add(LOG_SESSION_MTIME_SLACK_MS) >= created
        }
        _ => path.is_file(),
    }
}

fn window_title_indicates_safe_mode(title: &str) -> bool {
    title.to_ascii_uppercase().contains("SAFE MODE")
}

fn classify_log_line(line: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains(": error ")
        || lower.contains("error cs")
        || lower.contains("script compilation error")
        || lower.contains("tundra build failed")
        || lower.contains("compilation failed")
        || lower.contains("crash!!!")
        || lower.contains("fatal error")
        || lower.contains("exception")
    {
        Some("error")
    } else if lower.contains(": warning ")
        || lower.contains("warning cs")
        || lower.starts_with("warning:")
    {
        Some("warn")
    } else if lower.contains("safe mode")
        || lower.starts_with("[scriptcompilation]")
        || lower.starts_with("[locus]")
    {
        Some("info")
    } else {
        None
    }
}

fn read_tail(path: &Path, max_bytes: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| {
        format!(
            "Failed to open Unity Editor log '{}': {error}",
            path.display()
        )
    })?;
    let len = file
        .metadata()
        .map_err(|error| {
            format!(
                "Failed to inspect Unity Editor log '{}': {error}",
                path.display()
            )
        })?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).map_err(|error| {
        format!(
            "Failed to seek Unity Editor log '{}': {error}",
            path.display()
        )
    })?;
    let mut bytes = Vec::with_capacity((len - start).min(max_bytes) as usize);
    file.read_to_end(&mut bytes).map_err(|error| {
        format!(
            "Failed to read Unity Editor log '{}': {error}",
            path.display()
        )
    })?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(index) = text.find('\n') {
            text.drain(..=index);
        }
    }
    Ok(text)
}

fn file_modified_at_ms(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_safe_mode_window_titles() {
        assert!(window_title_indicates_safe_mode(
            "Game - SAFE MODE - 6000.5.8f1 <DX11>"
        ));
        assert!(!window_title_indicates_safe_mode(
            "Game - Main - Unity 6000.5.8f1 <DX11>"
        ));
    }

    #[test]
    fn classifies_compiler_and_crash_diagnostics() {
        assert_eq!(
            classify_log_line("Assets/Test.cs(1,2): error CS0103: Missing"),
            Some("error")
        );
        assert_eq!(
            classify_log_line("Assets/Test.cs(1,2): warning CS0168: Unused"),
            Some("warn")
        );
        assert_eq!(
            classify_log_line("Safe Mode: Only loading a subset of assemblies"),
            Some("info")
        );
        assert_eq!(classify_log_line("ordinary editor noise"), None);
    }

    #[test]
    fn reads_filtered_deduplicated_log_entries() {
        let root = tempfile::tempdir().expect("tempdir");
        let logs = root.path().join("Logs");
        std::fs::create_dir_all(&logs).expect("logs");
        std::fs::write(
            logs.join("Editor.log"),
            "Safe Mode: Only loading a subset of assemblies\nAssets/Test.cs(1,2): error CS0103: Missing\nAssets/Test.cs(1,2): error CS0103: Missing\nAssets/Test.cs(3,4): warning CS0168: Unused\n",
        )
        .expect("write log");

        let read = read_console_entries(
            &root.path().display().to_string(),
            None,
            &["error".to_string()],
            50,
        )
        .expect("read log");
        assert_eq!(read.entries.len(), 1);
        assert_eq!(read.entries[0].count, 2);
        assert!(read.entries[0].message.contains("CS0103"));
        assert_eq!(read.matched_count, 2);
    }
}
