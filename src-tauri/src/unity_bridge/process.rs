use std::{collections::HashMap, path::Path, process::Stdio, sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{strip_extended_path_prefix, unix_now_ms};

const UNITY_PROCESS_PROBE_CACHE_TTL_MS: u64 = 15_000;
const UNITY_PROCESS_PROBE_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityEditorProcessState {
    Running,
    NotRunning,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct UnityEditorProcessInfo {
    pub state: UnityEditorProcessState,
    pub process_id: Option<u32>,
    pub executable_path: Option<String>,
    pub project_path: Option<String>,
    pub checked_at_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Win32UnityProcess {
    process_id: u32,
    executable_path: Option<String>,
    command_line: Option<String>,
}

fn unity_process_probe_cache() -> &'static Mutex<HashMap<String, UnityEditorProcessInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<String, UnityEditorProcessInfo>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl UnityEditorProcessInfo {
    pub fn inferred_running(checked_at_ms: u64) -> Self {
        Self {
            state: UnityEditorProcessState::Running,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: None,
        }
    }

    fn not_running(checked_at_ms: u64) -> Self {
        Self {
            state: UnityEditorProcessState::NotRunning,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: None,
        }
    }

    fn unknown(checked_at_ms: u64, error: impl Into<String>) -> Self {
        Self {
            state: UnityEditorProcessState::Unknown,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: Some(error.into()),
        }
    }

    fn running(checked_at_ms: u64, process: Win32UnityProcess, project_path: String) -> Self {
        Self {
            state: UnityEditorProcessState::Running,
            process_id: Some(process.process_id),
            executable_path: process.executable_path,
            project_path: Some(project_path),
            checked_at_ms,
            last_error: None,
        }
    }
}

fn normalize_project_identity(path: &str) -> Option<String> {
    let trimmed = strip_extended_path_prefix(path)
        .trim()
        .trim_matches('"')
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized =
        dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf());
    let mut value = normalized.to_string_lossy().replace('/', "\\");
    while value.len() > 3 && value.ends_with('\\') {
        value.pop();
    }

    if value.is_empty() {
        return None;
    }

    #[cfg(windows)]
    {
        value = value.to_ascii_lowercase();
    }

    Some(value)
}

fn process_cache_key(project_path: &str) -> String {
    normalize_project_identity(project_path).unwrap_or_else(|| {
        strip_extended_path_prefix(project_path)
            .trim()
            .to_ascii_lowercase()
    })
}

pub async fn query_current_project_editor_process(project_path: &str) -> UnityEditorProcessInfo {
    let key = process_cache_key(project_path);
    let now = unix_now_ms();

    {
        let cache = unity_process_probe_cache().lock().await;
        if let Some(cached) = cache.get(&key) {
            if now.saturating_sub(cached.checked_at_ms) <= UNITY_PROCESS_PROBE_CACHE_TTL_MS {
                return cached.clone();
            }
        }
    }

    let project_path = project_path.to_string();
    let probe = match tokio::time::timeout(
        Duration::from_secs(UNITY_PROCESS_PROBE_TIMEOUT_SECS),
        query_current_project_editor_process_uncached(project_path),
    )
    .await
    {
        Ok(info) => info,
        Err(_) => UnityEditorProcessInfo::unknown(
            unix_now_ms(),
            format!(
                "Unity process probe timed out after {}s",
                UNITY_PROCESS_PROBE_TIMEOUT_SECS
            ),
        ),
    };

    let mut cache = unity_process_probe_cache().lock().await;
    cache.insert(key, probe.clone());
    probe
}

#[cfg(windows)]
async fn query_current_project_editor_process_uncached(
    project_path: String,
) -> UnityEditorProcessInfo {
    let checked_at_ms = unix_now_ms();
    let target = match normalize_project_identity(&project_path) {
        Some(value) => value,
        None => {
            return UnityEditorProcessInfo::unknown(
                checked_at_ms,
                "Current workspace path is empty",
            )
        }
    };

    let records = match query_unity_processes().await {
        Ok(records) => records,
        Err(error) => return UnityEditorProcessInfo::unknown(checked_at_ms, error),
    };

    if records.is_empty() {
        return UnityEditorProcessInfo::not_running(checked_at_ms);
    }

    let mut missing_command_lines = 0usize;
    let mut readable_command_lines = 0usize;

    for process in records {
        let command_line = match process.command_line.as_deref() {
            Some(value) if !value.trim().is_empty() => value,
            _ => {
                missing_command_lines += 1;
                continue;
            }
        };

        readable_command_lines += 1;
        let args = split_windows_command_line(command_line);
        let Some(project_arg) = project_path_from_args(&args) else {
            continue;
        };
        let Some(candidate) = normalize_project_identity(project_arg) else {
            continue;
        };

        if candidate == target {
            return UnityEditorProcessInfo::running(
                checked_at_ms,
                process,
                project_arg.to_string(),
            );
        }
    }

    if readable_command_lines == 0 && missing_command_lines > 0 {
        return UnityEditorProcessInfo::unknown(
            checked_at_ms,
            "Unity process command line is unavailable",
        );
    }

    UnityEditorProcessInfo::not_running(checked_at_ms)
}

#[cfg(not(windows))]
async fn query_current_project_editor_process_uncached(
    _project_path: String,
) -> UnityEditorProcessInfo {
    UnityEditorProcessInfo::unknown(
        unix_now_ms(),
        "Unity editor process detection is only supported on Windows",
    )
}

#[cfg(windows)]
async fn query_unity_processes() -> Result<Vec<Win32UnityProcess>, String> {
    let script = r#"$ErrorActionPreference = 'Stop'; $items = @(Get-CimInstance Win32_Process -Filter "Name = 'Unity.exe'" | ForEach-Object { [pscustomobject]@{ ProcessId = $_.ProcessId; ExecutablePath = $_.ExecutablePath; CommandLine = $_.CommandLine } }); ConvertTo-Json -Compress -Depth 3 -InputObject $items"#;

    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command
        .output()
        .await
        .map_err(|error| format!("Failed to query Unity processes: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Unity process query failed with status {}", output.status)
        } else {
            format!("Unity process query failed: {}", stderr)
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str::<Vec<Win32UnityProcess>>(&stdout)
        .map_err(|error| format!("Failed to parse Unity process query output: {}", error))
}

fn project_path_from_args(args: &[String]) -> Option<&str> {
    const PROJECT_PATH_ARG: &str = "-projectpath";

    for (index, arg) in args.iter().enumerate() {
        let lower = arg.to_ascii_lowercase();
        if lower == PROJECT_PATH_ARG {
            return args.get(index + 1).map(String::as_str);
        }

        if lower.starts_with("-projectpath=") {
            return arg.get(PROJECT_PATH_ARG.len() + 1..);
        }
    }

    None
}

fn split_windows_command_line(command_line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut backslashes = 0usize;
    let mut has_current = false;

    for ch in command_line.chars() {
        if ch == '\\' {
            backslashes += 1;
            has_current = true;
            continue;
        }

        if ch == '"' {
            current.extend(std::iter::repeat('\\').take(backslashes / 2));
            if backslashes % 2 == 0 {
                in_quotes = !in_quotes;
            } else {
                current.push('"');
            }
            backslashes = 0;
            has_current = true;
            continue;
        }

        if backslashes > 0 {
            current.extend(std::iter::repeat('\\').take(backslashes));
            backslashes = 0;
        }

        if ch.is_whitespace() && !in_quotes {
            if has_current {
                args.push(std::mem::take(&mut current));
                has_current = false;
            }
            continue;
        }

        current.push(ch);
        has_current = true;
    }

    if backslashes > 0 {
        current.extend(std::iter::repeat('\\').take(backslashes));
    }

    if has_current {
        args.push(current);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::{normalize_project_identity, project_path_from_args, split_windows_command_line};

    #[test]
    fn parses_unity_hub_project_path_argument_case_insensitively() {
        let args = split_windows_command_line(
            r#"E:\2022.3.58f1\Editor\Unity.exe -projectpath "J:\My project (2)" -useHub -hubIPC"#,
        );

        assert_eq!(project_path_from_args(&args), Some(r#"J:\My project (2)"#));
    }

    #[test]
    fn parses_locus_project_path_argument() {
        let args = split_windows_command_line(
            r#""C:\Program Files\Unity\Editor\Unity.exe" -projectPath C:\Projects\Game -batchmode"#,
        );

        assert_eq!(project_path_from_args(&args), Some(r#"C:\Projects\Game"#));
    }

    #[test]
    fn parses_project_path_equals_form() {
        let args =
            split_windows_command_line(r#"Unity.exe -projectPath="C:\Projects\Game With Spaces""#);

        assert_eq!(
            project_path_from_args(&args),
            Some(r#"C:\Projects\Game With Spaces"#)
        );
    }

    #[test]
    fn normalizes_extended_path_prefixes() {
        let normalized = normalize_project_identity(r#"\\?\C:\Projects\Game\"#).unwrap();

        #[cfg(windows)]
        assert_eq!(normalized, r#"c:\projects\game"#);

        #[cfg(not(windows))]
        assert_eq!(normalized, r#"C:\Projects\Game"#);
    }
}
