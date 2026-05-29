mod capture;
mod focus;
mod plugin;
mod process;
mod transport;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

pub use capture::{capture_viewport, UnityViewportCapture};
pub use plugin::{
    check_plugin_status, emit_plugin_status, find_plugin_source_dir, install_or_update_plugin,
    plugin_install_root, plugin_skills_root, PluginStatus,
};
pub use process::{
    query_current_project_editor_process, UnityEditorProcessInfo, UnityEditorProcessState,
};
pub use transport::{
    send_message, send_message_with_timeout, send_message_without_timeout, set_event_app_handle,
};

pub type UnityMonitorHandle = Arc<tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>;

pub const UNITY_EDITOR_STATUS_DISCONNECTED: &str = "disconnected";
pub const UNITY_EDITOR_STATUS_EDITING: &str = "editing";
pub const UNITY_EDITOR_STATUS_PLAYING: &str = "playing";
pub const UNITY_EDITOR_STATUS_PLAYING_PAUSED: &str = "playing_paused";
pub const UNITY_EDITOR_STATUS_SCHEMA: &str = "disconnected | editing | playing | playing_paused";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipeResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

pub const UNITY_EXECUTE_PROGRESS_TAG: &str = "locus-unity-progress";
pub const UNITY_EXECUTE_CANCELLED: &str = "__locus_unity_execute_cancelled__";
const UNITY_EXECUTE_PROGRESS_POLL_MS: u64 = 250;
const UNITY_EXECUTE_START_TIMEOUT_SECS: u64 = 15;
const UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityLaunchResult {
    pub editor_path: String,
    pub project_path: String,
    pub project_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConnectionStatus {
    pub connected: bool,
    pub editor_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_path: Option<String>,
    pub editor_process_state: UnityEditorProcessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_checked_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_last_error: Option<String>,
    pub pipe_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub reconnect_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub checked_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnityExecuteProgressSnapshot {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectAssetRequest<'a> {
    asset_path: &'a str,
    focus_project_window: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneObjectRequest<'a> {
    scene_path: &'a str,
    object_path: &'a str,
}

type ProjectUnityOpLock = Arc<Mutex<()>>;

fn unity_operation_locks() -> &'static Mutex<HashMap<String, ProjectUnityOpLock>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, ProjectUnityOpLock>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn project_unity_op_lock(project_path: &str) -> ProjectUnityOpLock {
    let key = strip_extended_path_prefix(project_path).to_string();
    let mut locks = unity_operation_locks().lock().await;
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn strip_extended_path_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

fn get_pipe_name(project_path: &str) -> String {
    let path = strip_extended_path_prefix(project_path);
    let sanitized = path
        .replace('\\', "_")
        .replace('/', "_")
        .replace(':', "_")
        .replace(' ', "_");
    format!(r"\\.\pipe\locus_unity_{}", sanitized)
}

pub fn is_unity_project(path: &str) -> bool {
    let p = Path::new(strip_extended_path_prefix(path));
    p.join("Assets").is_dir() && p.join("ProjectSettings").is_dir()
}

pub fn read_project_unity_version(project_path: &str) -> Result<Option<String>, String> {
    let version_path = Path::new(strip_extended_path_prefix(project_path))
        .join("ProjectSettings")
        .join("ProjectVersion.txt");
    if !version_path.is_file() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&version_path).map_err(|error| {
        format!(
            "Failed to read Unity project version file '{}': {}",
            version_path.display(),
            error
        )
    })?;

    Ok(content.lines().find_map(|line| {
        line.strip_prefix("m_EditorVersion:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }))
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

fn push_editor_install_root_candidates(paths: &mut Vec<PathBuf>, root: PathBuf) {
    #[cfg(target_os = "windows")]
    {
        push_unique_path(paths, root.join("Editor").join("Unity.exe"));
        push_unique_path(paths, root.join("Unity.exe"));
    }

    #[cfg(target_os = "macos")]
    {
        push_unique_path(
            paths,
            root.join("Unity.app")
                .join("Contents")
                .join("MacOS")
                .join("Unity"),
        );
        push_unique_path(paths, root.join("Contents").join("MacOS").join("Unity"));
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        push_unique_path(paths, root.join("Editor").join("Unity"));
        push_unique_path(paths, root.join("Unity"));
    }
}

fn push_env_editor_candidates(paths: &mut Vec<PathBuf>) {
    let Some(raw_path) = std::env::var_os("LOCUS_UNITY_EDITOR_PATH") else {
        return;
    };
    let path = PathBuf::from(raw_path);
    if path.is_file() {
        push_unique_path(paths, path);
    } else {
        push_editor_install_root_candidates(paths, path);
    }
}

#[cfg(target_os = "windows")]
fn push_windows_registry_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let subkeys = [
        format!(r"SOFTWARE\Unity Technologies\Installer\Unity {version}"),
        format!(r"SOFTWARE\WOW6432Node\Unity Technologies\Installer\Unity {version}"),
    ];
    let hives = [
        RegKey::predef(HKEY_CURRENT_USER),
        RegKey::predef(HKEY_LOCAL_MACHINE),
    ];

    for hive in hives {
        for subkey in &subkeys {
            let Ok(key) = hive.open_subkey(subkey) else {
                continue;
            };
            for value_name in ["Location x64", "Location"] {
                let Ok(location) = key.get_value::<String, _>(value_name) else {
                    continue;
                };
                let location = location.trim();
                if !location.is_empty() {
                    push_editor_install_root_candidates(paths, PathBuf::from(location));
                }
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn push_windows_registry_editor_candidates(_paths: &mut Vec<PathBuf>, _version: &str) {}

fn push_default_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            push_editor_install_root_candidates(
                paths,
                PathBuf::from(program_files)
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
        if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
            push_editor_install_root_candidates(
                paths,
                PathBuf::from(program_files_x86)
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        push_editor_install_root_candidates(
            paths,
            PathBuf::from("/Applications")
                .join("Unity")
                .join("Hub")
                .join("Editor")
                .join(version),
        );
        push_editor_install_root_candidates(paths, PathBuf::from("/Applications").join("Unity"));
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(home_dir) = dirs::home_dir() {
            push_editor_install_root_candidates(
                paths,
                home_dir
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
        push_editor_install_root_candidates(
            paths,
            PathBuf::from("/opt")
                .join("Unity")
                .join("Hub")
                .join("Editor")
                .join(version),
        );
    }
}

pub fn resolve_unity_editor_executable(version: &str) -> Result<PathBuf, String> {
    let version = version.trim();
    if version.is_empty() {
        return Err("Unity project version is empty".to_string());
    }

    let mut candidates = Vec::new();
    push_env_editor_candidates(&mut candidates);
    push_windows_registry_editor_candidates(&mut candidates, version);
    push_default_editor_candidates(&mut candidates, version);

    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }

    let checked = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "Unity Editor {} was not found. Checked: {}",
        version, checked
    ))
}

fn normalized_project_path_for_launch(project_path: &str) -> PathBuf {
    let trimmed = strip_extended_path_prefix(project_path).trim();
    dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf())
}

pub fn launch_project(project_path: &str) -> Result<UnityLaunchResult, String> {
    if !is_unity_project(project_path) {
        return Err("Current working directory is not a Unity project".to_string());
    }

    let project_version = read_project_unity_version(project_path)?
        .ok_or_else(|| "Current Unity project is missing ProjectVersion.txt".to_string())?;
    let editor_path = resolve_unity_editor_executable(&project_version)?;
    let project_path = normalized_project_path_for_launch(project_path);

    let mut command = std::process::Command::new(&editor_path);
    command
        .arg("-projectPath")
        .arg(&project_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().map_err(|error| {
        format!(
            "Failed to launch Unity Editor '{}': {}",
            editor_path.display(),
            error
        )
    })?;

    eprintln!(
        "[Locus] launched Unity Editor: editor='{}', project='{}'",
        editor_path.display(),
        project_path.display()
    );

    Ok(UnityLaunchResult {
        editor_path: editor_path.display().to_string(),
        project_path: project_path.display().to_string(),
        project_version,
    })
}

// ── Public API (cross-platform, routes through transport) ────────────

pub fn normalize_editor_status(status: &str) -> &'static str {
    match status {
        UNITY_EDITOR_STATUS_DISCONNECTED => UNITY_EDITOR_STATUS_DISCONNECTED,
        UNITY_EDITOR_STATUS_PLAYING => UNITY_EDITOR_STATUS_PLAYING,
        UNITY_EDITOR_STATUS_PLAYING_PAUSED => UNITY_EDITOR_STATUS_PLAYING_PAUSED,
        _ => UNITY_EDITOR_STATUS_EDITING,
    }
}

pub fn is_known_editor_status(status: &str) -> bool {
    matches!(
        status,
        UNITY_EDITOR_STATUS_DISCONNECTED
            | UNITY_EDITOR_STATUS_EDITING
            | UNITY_EDITOR_STATUS_PLAYING
            | UNITY_EDITOR_STATUS_PLAYING_PAUSED
    )
}

pub fn is_play_mode_status(status: &str) -> bool {
    matches!(
        normalize_editor_status(status),
        UNITY_EDITOR_STATUS_PLAYING | UNITY_EDITOR_STATUS_PLAYING_PAUSED
    )
}

fn requested_run_states_editor_status(request: &serde_json::Value) -> Result<&str, String> {
    let requested_status = request
        .get("request_editor_status")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing required parameter: request_editor_status".to_string())?;

    if requested_status == UNITY_EDITOR_STATUS_DISCONNECTED
        || !is_known_editor_status(requested_status)
    {
        return Err(format!(
            "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
            requested_status
        ));
    }

    Ok(requested_status)
}

pub fn format_editor_status_for_prompt(status: &str) -> &'static str {
    match normalize_editor_status(status) {
        UNITY_EDITOR_STATUS_DISCONNECTED => {
            "`disconnected` (Unity Editor is not reachable; use file-level operations)"
        }
        UNITY_EDITOR_STATUS_PLAYING => {
            "`playing` (Play Mode running; avoid persistent asset or scene modifications via `unity_execute`)"
        }
        UNITY_EDITOR_STATUS_PLAYING_PAUSED => {
            "`playing_paused` (Play Mode paused; apply the same write-safety rules as `playing`)"
        }
        _ => "`editing` (Edit Mode; Editor API operations and persistent asset or scene changes are available)",
    }
}

pub fn format_editor_status_for_event(status: &str) -> &'static str {
    match normalize_editor_status(status) {
        UNITY_EDITOR_STATUS_DISCONNECTED => "`disconnected`",
        UNITY_EDITOR_STATUS_PLAYING => "`playing`",
        UNITY_EDITOR_STATUS_PLAYING_PAUSED => "`playing_paused`",
        _ => "`editing`",
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn parse_unity_status_message(message: &str) -> (&'static str, Option<String>) {
    let (status_part, scene_part) = match message.split_once('|') {
        Some((status, scene)) => (status, Some(scene.trim().to_string())),
        None => (message, None),
    };
    (
        normalize_editor_status(status_part),
        scene_part.filter(|scene| !scene.is_empty()),
    )
}

fn apply_unity_process_info(
    status: &mut UnityConnectionStatus,
    process_info: UnityEditorProcessInfo,
) {
    status.editor_process_state = process_info.state;
    status.editor_process_id = process_info.process_id;
    status.editor_process_path = process_info.executable_path;
    status.editor_project_path = process_info.project_path;
    status.process_checked_at_ms = Some(process_info.checked_at_ms);
    status.process_last_error = process_info.last_error;
}

pub async fn query_unity_connection_status(project_path: &str) -> UnityConnectionStatus {
    let pipe_name = get_pipe_name(project_path);
    let checked_at_ms = unix_now_ms();
    let started_at = std::time::Instant::now();

    match send_message(project_path, "status", "").await {
        Ok(resp) if resp.ok => {
            let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let message = resp.message.unwrap_or_default();
            let (editor_status, scene_path) = parse_unity_status_message(&message);
            UnityConnectionStatus {
                connected: true,
                editor_status: editor_status.to_string(),
                scene_path,
                editor_process_state: UnityEditorProcessState::Running,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: None,
                checked_at_ms,
            }
        }
        Ok(resp) => {
            let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let mut status = UnityConnectionStatus {
                connected: false,
                editor_status: UNITY_EDITOR_STATUS_DISCONNECTED.to_string(),
                scene_path: None,
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: Some(
                    resp.error
                        .unwrap_or_else(|| "Unity status returned ok=false".to_string()),
                ),
                checked_at_ms,
            };
            let process_info = query_current_project_editor_process(project_path).await;
            apply_unity_process_info(&mut status, process_info);
            status
        }
        Err(error) => {
            let mut status = UnityConnectionStatus {
                connected: false,
                editor_status: UNITY_EDITOR_STATUS_DISCONNECTED.to_string(),
                scene_path: None,
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: None,
                reconnect_attempts: 0,
                last_error: Some(error),
                checked_at_ms,
            };
            let process_info = query_current_project_editor_process(project_path).await;
            apply_unity_process_info(&mut status, process_info);
            status
        }
    }
}

pub async fn is_unity_connected(project_path: &str) -> bool {
    query_unity_status(project_path).await.0
}

pub async fn select_asset(
    project_path: &str,
    asset_path: &str,
    focus_project_window: bool,
) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let _prev_foreground = if focus_project_window {
        focus::bring_unity_to_foreground()
    } else {
        None
    };
    let payload = serde_json::to_string(&SelectAssetRequest {
        asset_path,
        focus_project_window,
    })
    .map_err(|e| e.to_string())?;
    let resp = send_message(project_path, "select_asset", &payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "select_asset failed".to_string()))
    }
}

pub async fn open_asset_inspector(project_path: &str, asset_path: &str) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&SelectAssetRequest {
        asset_path,
        focus_project_window: false,
    })
    .map_err(|e| e.to_string())?;
    let resp = send_message(project_path, "open_asset_inspector", &payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "open_asset_inspector failed".to_string()))
    }
}

pub async fn select_scene_object(
    project_path: &str,
    scene_path: &str,
    object_path: &str,
) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&SceneObjectRequest {
        scene_path,
        object_path,
    })
    .map_err(|e| e.to_string())?;
    let resp = send_message(project_path, "select_scene_object", &payload).await?;
    if resp.ok {
        let _ = focus::bring_unity_to_foreground();
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "select_scene_object failed".to_string()))
    }
}

pub async fn open_scene_object_inspector(
    project_path: &str,
    scene_path: &str,
    object_path: &str,
) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&SceneObjectRequest {
        scene_path,
        object_path,
    })
    .map_err(|e| e.to_string())?;
    let resp = send_message(project_path, "open_scene_object_inspector", &payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "open_scene_object_inspector failed".to_string()))
    }
}

pub async fn start_asset_drag(
    project_path: &str,
    payload: &str,
    trace_id: &str,
) -> Result<(), String> {
    let started_at = Instant::now();
    log_unity_asset_drag_trace(
        trace_id,
        "bridge_start_asset_drag_lock_wait",
        started_at,
        "",
    );
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    log_unity_asset_drag_trace(
        trace_id,
        "bridge_start_asset_drag_lock_acquired",
        started_at,
        "",
    );
    log_unity_asset_drag_trace(
        trace_id,
        "bridge_start_asset_drag_pipe_send",
        started_at,
        "",
    );
    let resp = send_message(project_path, "start_asset_drag", payload).await?;
    if resp.ok {
        log_unity_asset_drag_trace(trace_id, "bridge_start_asset_drag_pipe_ok", started_at, "");
        Ok(())
    } else {
        log_unity_asset_drag_trace(
            trace_id,
            "bridge_start_asset_drag_pipe_error",
            started_at,
            "",
        );
        Err(resp
            .error
            .unwrap_or_else(|| "start_asset_drag failed".to_string()))
    }
}

fn log_unity_asset_drag_trace(trace_id: &str, phase: &str, started_at: Instant, detail: &str) {
    let elapsed_ms = started_at.elapsed().as_millis();
    if detail.is_empty() {
        eprintln!("[Locus][UnityAssetDrag] trace={trace_id} phase={phase} elapsedMs={elapsed_ms}");
    } else {
        eprintln!(
            "[Locus][UnityAssetDrag] trace={trace_id} phase={phase} elapsedMs={elapsed_ms} {detail}"
        );
    }
}

pub async fn cancel_asset_drag(project_path: &str) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message(project_path, "cancel_asset_drag", "").await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "cancel_asset_drag failed".to_string()))
    }
}

/// Canonical status values: "disconnected" | "editing" | "playing" | "playing_paused"
pub async fn query_unity_status(project_path: &str) -> (bool, &'static str, Option<String>) {
    match send_message(project_path, "status", "").await {
        Ok(resp) if resp.ok => {
            let msg = resp.message.unwrap_or_default();
            let (status, scene_part) = parse_unity_status_message(&msg);
            (true, status, scene_part)
        }
        _ => (false, UNITY_EDITOR_STATUS_DISCONNECTED, None),
    }
}

pub async fn exit_play_mode(project_path: &str) -> Result<(), String> {
    let resp = send_message(project_path, "exit_play_mode", "").await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "exit_play_mode failed".to_string()));
    }
    let msg = resp.message.unwrap_or_default();
    if msg == "already_editing" {
        return Ok(());
    }

    let max_wait = Duration::from_secs(30);
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > max_wait {
            return Err("Timed out waiting to exit play mode (30s)".to_string());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (_, status, _) = query_unity_status(project_path).await;
        if status == UNITY_EDITOR_STATUS_EDITING {
            return Ok(());
        }
    }
}

pub async fn set_editor_status(project_path: &str, desired_status: &str) -> Result<(), String> {
    if !is_known_editor_status(desired_status) || desired_status == UNITY_EDITOR_STATUS_DISCONNECTED
    {
        return Err(format!(
            "Invalid requested Unity Editor status: {}",
            desired_status
        ));
    }

    let resp = send_message(project_path, "set_editor_status", desired_status).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "set_editor_status failed".to_string()));
    }

    let max_wait = Duration::from_secs(30);
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > max_wait {
            return Err(format!(
                "Timed out waiting for Unity Editor status '{}' (30s)",
                desired_status
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let (_connected, status, _) = query_unity_status(project_path).await;
        if status == desired_status {
            return Ok(());
        }
    }
}

const RUN_STATES_INLINE_PRINT_LIMIT_TOKENS: u64 = 100_000;
const RUN_STATES_HARD_PRINT_LIMIT_TOKENS: u64 = 1_000_000;
const RUN_STATES_TOKEN_BYTE_RATIO: u64 = 4;

#[derive(Debug, Clone, Copy)]
struct RunStatesPrintStats {
    lines: u64,
    tokens: u64,
}

fn estimate_run_states_tokens(byte_count: u64) -> u64 {
    if byte_count == 0 {
        0
    } else {
        (byte_count + RUN_STATES_TOKEN_BYTE_RATIO - 1) / RUN_STATES_TOKEN_BYTE_RATIO
    }
}

fn parse_run_states_u64_field(output: &str, key: &str) -> Option<u64> {
    let prefix = format!("{key}:");
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .and_then(|value| value.trim().parse::<u64>().ok())
    })
}

fn compute_run_states_print_stats(output: &str) -> RunStatesPrintStats {
    let mut found_prints = false;
    let mut lines = 0u64;
    let mut bytes = 0u64;

    for line in output.lines() {
        if found_prints {
            lines += 1;
            bytes = bytes.saturating_add(line.as_bytes().len() as u64 + 1);
            continue;
        }

        if line.trim().eq_ignore_ascii_case("prints:") {
            found_prints = true;
        }
    }

    RunStatesPrintStats {
        lines: parse_run_states_u64_field(output, "print_lines").unwrap_or(lines),
        tokens: parse_run_states_u64_field(output, "print_tokens_estimate")
            .unwrap_or_else(|| estimate_run_states_tokens(bytes)),
    }
}

fn run_states_output_header(output: &str) -> String {
    let mut lines = Vec::new();
    for line in output.lines() {
        if line.trim().eq_ignore_ascii_case("prints:") {
            break;
        }
        lines.push(line.trim_end_matches('\r'));
    }
    lines.join("\n")
}

fn run_states_has_field(output: &str, key: &str) -> bool {
    let prefix = format!("{key}:");
    output
        .lines()
        .any(|line| line.trim_start().starts_with(&prefix))
}

fn push_run_states_field_if_missing(summary: &mut String, header: &str, key: &str, value: &str) {
    if !run_states_has_field(header, key) {
        summary.push_str(key);
        summary.push_str(": ");
        summary.push_str(value);
        summary.push('\n');
    }
}

fn run_states_result_dir(project_path: &str) -> PathBuf {
    Path::new(project_path)
        .join("Library")
        .join("Locus")
        .join("RunStates")
}

fn persist_run_states_result(project_path: &str, output: &str) -> Result<PathBuf, String> {
    let dir = run_states_result_dir(project_path);
    std::fs::create_dir_all(&dir).map_err(|error| {
        format!(
            "Failed to create unity_run_states result dir '{}': {}",
            dir.display(),
            error
        )
    })?;

    let path = dir.join(format!("run-states-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, output).map_err(|error| {
        format!(
            "Failed to save unity_run_states result to '{}': {}",
            path.display(),
            error
        )
    })?;
    Ok(path)
}

fn build_run_states_large_summary(
    output: &str,
    stats: RunStatesPrintStats,
    result_file: Option<&Path>,
) -> String {
    let header = run_states_output_header(output);
    let mut summary = header.trim_end().to_string();
    if !summary.is_empty() {
        summary.push('\n');
    }

    push_run_states_field_if_missing(
        &mut summary,
        &header,
        "print_lines",
        &stats.lines.to_string(),
    );
    push_run_states_field_if_missing(
        &mut summary,
        &header,
        "print_tokens_estimate",
        &stats.tokens.to_string(),
    );
    push_run_states_field_if_missing(&mut summary, &header, "print_output", "too large");

    if let Some(path) = result_file {
        push_run_states_field_if_missing(
            &mut summary,
            &header,
            "result_file",
            &path.display().to_string(),
        );
        push_run_states_field_if_missing(
            &mut summary,
            &header,
            "print_output_message",
            &format!(
                "print output exceeded {} estimated tokens; full result saved to result_file.",
                RUN_STATES_INLINE_PRINT_LIMIT_TOKENS
            ),
        );
    } else {
        push_run_states_field_if_missing(
            &mut summary,
            &header,
            "print_output_message",
            &format!(
                "print output exceeded hard limit of {} estimated tokens; result was not saved.",
                RUN_STATES_HARD_PRINT_LIMIT_TOKENS
            ),
        );
    }

    summary.trim_end().to_string()
}

fn rewrite_run_states_output_for_size(
    project_path: &str,
    output: String,
) -> Result<String, String> {
    let stats = compute_run_states_print_stats(&output);
    if stats.tokens <= RUN_STATES_INLINE_PRINT_LIMIT_TOKENS {
        return Ok(output);
    }

    if stats.tokens > RUN_STATES_HARD_PRINT_LIMIT_TOKENS {
        return Err(build_run_states_large_summary(&output, stats, None));
    }

    let path = persist_run_states_result(project_path, &output).map_err(|error| {
        format!(
            "print_output: too large\nprint_lines: {}\nprint_tokens_estimate: {}\nprint_output_message: {}\n{}",
            stats.lines,
            stats.tokens,
            "print output exceeded inline limit and could not be saved.",
            error
        )
    })?;
    Ok(build_run_states_large_summary(&output, stats, Some(&path)))
}

pub async fn unity_run_states(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    requested_run_states_editor_status(request)?;

    let prepared = prepare_unity_run_states_request_for_send(project_path, request).await;
    let payload = serde_json::to_string(&prepared.request)
        .map_err(|error| format!("Failed to serialize unity_run_states request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "run_states", &payload).await?;
    let output = if resp.ok {
        resp.message.unwrap_or_default()
    } else {
        resp.error
            .unwrap_or_else(|| "unity_run_states failed".to_string())
    };

    let rewritten = match rewrite_run_states_output_for_size(project_path, output) {
        Ok(output) => output,
        Err(error) if resp.ok => return Err(error),
        Err(error) => {
            return Err(crate::unity_type_index::append_auto_using_notes(
                error,
                &prepared.prepared_code,
            ));
        }
    };
    if resp.ok {
        Ok(rewritten)
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            rewritten,
            &prepared.prepared_code,
        ))
    }
}

pub async fn compile_run_states(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    requested_run_states_editor_status(request)?;

    let prepared = prepare_unity_run_states_request_for_send(project_path, request).await;
    let payload = serde_json::to_string(&prepared.request).map_err(|error| {
        format!(
            "Failed to serialize unity_run_states compilation request: {}",
            error
        )
    })?;
    let resp = send_message_without_timeout(project_path, "compile_run_states", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error
                .unwrap_or_else(|| "unity_run_states compilation failed".to_string()),
            &prepared.prepared_code,
        ))
    }
}

pub async fn compile_named(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize compile_named request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "compile_named", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "compile_named failed".to_string()))
    }
}

pub async fn compile_skill_package(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request).map_err(|error| {
        format!(
            "Failed to serialize compile_skill_package request: {}",
            error
        )
    })?;
    let resp =
        send_message_without_timeout(project_path, "compile_skill_package", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "compile_skill_package failed".to_string()))
    }
}

pub async fn invoke_skill_package(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request).map_err(|error| {
        format!(
            "Failed to serialize invoke_skill_package request: {}",
            error
        )
    })?;
    let resp = send_message_without_timeout(project_path, "invoke_skill_package", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_skill_package failed".to_string()))
    }
}

pub async fn invoke_named(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize invoke_named request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "invoke_named", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_named failed".to_string()))
    }
}

pub async fn invoke_named_cached(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize invoke_named_cached request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "invoke_named_cached", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_named_cached failed".to_string()))
    }
}

pub async fn view_binding_read(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_read", request).await
}

pub async fn view_binding_discover(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_discover", request).await
}

pub async fn view_binding_write(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_write", request).await
}

pub async fn view_binding_apply(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_apply", request).await
}

async fn send_view_binding_message(
    project_path: &str,
    message_type: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize {} request: {}", message_type, error))?;
    let resp = send_message_without_timeout(project_path, message_type, &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| format!("{} failed", message_type)))
    }
}

pub async fn unity_log(project_path: &str, message: &str) -> Result<(), String> {
    let resp = send_message(project_path, "log", message).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
}

pub async fn unity_warn(project_path: &str, message: &str) -> Result<(), String> {
    let resp = send_message(project_path, "warn", message).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
}

pub async fn unity_error(project_path: &str, message: &str) -> Result<(), String> {
    let resp = send_message(project_path, "error", message).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
}

/// Begin a Unity edit session and suppress Auto Refresh until the session ends.
pub async fn begin_edit_session(project_path: &str, owner: &str) -> Result<String, String> {
    let resp = send_message(project_path, "begin_edit_session", owner).await?;
    if resp.ok {
        Ok(resp
            .message
            .unwrap_or_else(|| "active_edit_sessions:0".to_string()))
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "begin_edit_session failed".to_string()))
    }
}

/// End a Unity edit session for the given owner.
/// Pass an empty owner to release every active session before recompiling.
pub async fn end_edit_session(project_path: &str, owner: &str) -> Result<String, String> {
    let resp = send_message(project_path, "end_edit_session", owner).await?;
    if resp.ok {
        Ok(resp
            .message
            .unwrap_or_else(|| "active_edit_sessions:0".to_string()))
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "end_edit_session failed".to_string()))
    }
}

/// Queue changed Unity asset paths so the editor can import them before recompiling.
pub async fn import_assets(project_path: &str, asset_paths: &[String]) -> Result<String, String> {
    if asset_paths.is_empty() {
        return Ok("0 assets queued".to_string());
    }

    let resp = send_message(project_path, "import_assets", &asset_paths.join("\n")).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_else(|| "assets queued".to_string()))
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "import_assets failed".to_string()))
    }
}

/// Queue changed Unity asset paths without blocking the caller.
pub fn import_assets_fire_and_forget(project_path: &str, asset_paths: Vec<String>) {
    if asset_paths.is_empty() {
        return;
    }
    let path = project_path.to_string();
    tokio::spawn(async move {
        match import_assets(&path, &asset_paths).await {
            Ok(msg) => eprintln!("[Locus] queued changed Unity assets: {}", msg),
            Err(e) => eprintln!("[Locus] import_assets skipped: {}", e),
        }
    });
}

pub fn format_unity_execute_progress_delta(snapshot: &UnityExecuteProgressSnapshot) -> String {
    let payload = serde_json::to_string(snapshot).unwrap_or_else(|_| {
        "{\"active\":false,\"title\":\"\",\"info\":\"\",\"progress\":0,\"revision\":0,\"source\":\"\"}".to_string()
    });
    format!(
        "<{tag}>{payload}</{tag}>\n",
        tag = UNITY_EXECUTE_PROGRESS_TAG,
        payload = payload
    )
}

fn rust_unity_execute_progress(
    title: impl Into<String>,
    info: impl Into<String>,
    revision: u64,
) -> UnityExecuteProgressSnapshot {
    UnityExecuteProgressSnapshot {
        active: true,
        title: title.into(),
        info: info.into(),
        progress: 0.0,
        revision,
        source: "rust".to_string(),
    }
}

async fn query_unity_execute_progress(project_path: &str) -> Option<UnityExecuteProgressSnapshot> {
    let resp = send_message_with_timeout(
        project_path,
        "execute_code_progress",
        "",
        Duration::from_secs(2),
    )
    .await
    .ok()?;

    if !resp.ok {
        return None;
    }

    let message = resp.message?;
    serde_json::from_str(&message).ok()
}

async fn wait_for_unity_bridge_ready(
    project_path: &str,
    max_wait: Duration,
    context: &str,
) -> Result<(), String> {
    let start = std::time::Instant::now();

    loop {
        let detail =
            match send_message_with_timeout(project_path, "status", "", Duration::from_secs(5))
                .await
            {
                Ok(resp) if resp.ok => return Ok(()),
                Ok(resp) => resp
                    .error
                    .unwrap_or_else(|| "Unity status returned ok=false".to_string()),
                Err(error) => error,
            };

        if start.elapsed() > max_wait {
            return Err(format!(
                "Timed out waiting for Unity bridge to become ready {} ({}s): {}",
                context,
                max_wait.as_secs(),
                detail
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn reconnect_unity_pipe_for_execute(project_path: &str, reason: &str) -> Result<(), String> {
    transport::disconnect_with_reason(project_path, reason).await;
    wait_for_unity_bridge_ready(
        project_path,
        Duration::from_secs(20),
        "after execute pipe reset",
    )
    .await
}

async fn reconnect_unity_pipe_for_execute_cancellable(
    project_path: &str,
    reason: &str,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let reconnect = reconnect_unity_pipe_for_execute(project_path, reason);
    tokio::pin!(reconnect);
    loop {
        tokio::select! {
            result = &mut reconnect => return result,
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
            }
        }
    }
}

fn append_execute_reconnect_result(reason: &str, reconnect: Result<(), String>) -> String {
    match reconnect {
        Ok(()) => format!("{}; Unity pipe reconnected.", reason),
        Err(error) => format!("{}; Unity pipe reconnect failed: {}", reason, error),
    }
}

pub async fn cancel_unity_execute_code(project_path: &str) -> Result<String, String> {
    let resp = send_message_with_timeout(
        project_path,
        "cancel_execute_code",
        "",
        Duration::from_secs(5),
    )
    .await?;

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "cancel_execute_code failed".to_string()))
    }
}

pub async fn refresh_unity_type_index(
    project_path: &str,
) -> Result<Arc<crate::unity_type_index::UnityTypeIndex>, String> {
    let resp = send_message_with_timeout(
        project_path,
        "export_type_index",
        "",
        Duration::from_secs(30),
    )
    .await?;

    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "export_type_index failed".to_string()));
    }

    let message = resp.message.unwrap_or_default();
    crate::unity_type_index::persist_exported_type_index(project_path, &message).await
}

pub struct UnityTypeIndexUpdateResult {
    pub mode: String,
}

async fn current_unity_type_index_fingerprint(project_path: &str) -> Result<String, String> {
    let resp = send_message_with_timeout(
        project_path,
        "export_type_index_fingerprint",
        "",
        Duration::from_secs(10),
    )
    .await?;

    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "export_type_index_fingerprint failed".to_string()));
    }

    let message = resp.message.unwrap_or_default();
    crate::unity_type_index::parse_exported_type_index_fingerprint(&message)
}

async fn cached_unity_type_index_is_current(
    project_path: &str,
    index: &crate::unity_type_index::UnityTypeIndex,
) -> Result<bool, String> {
    let current_fingerprint = current_unity_type_index_fingerprint(project_path).await?;
    Ok(!index.fingerprint.is_empty() && index.fingerprint == current_fingerprint)
}

pub async fn ensure_unity_type_index_current(
    project_path: &str,
) -> Result<UnityTypeIndexUpdateResult, String> {
    match crate::unity_type_index::load_cached_type_index(project_path).await {
        Ok(Some(index)) if cached_unity_type_index_is_current(project_path, &index).await? => {
            Ok(UnityTypeIndexUpdateResult {
                mode: "current".to_string(),
            })
        }
        Ok(Some(_)) | Ok(None) => {
            refresh_unity_type_index(project_path).await?;
            Ok(UnityTypeIndexUpdateResult {
                mode: "full".to_string(),
            })
        }
        Err(error) => Err(error),
    }
}

pub async fn update_unity_type_index_after_skill_package_compile(
    project_path: &str,
    compile_response: &serde_json::Value,
) -> Result<UnityTypeIndexUpdateResult, String> {
    let package_id = compile_response
        .get("packageId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let source_hash = compile_response
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let assembly_id = compile_response
        .get("assemblyId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let previous_assembly_id = compile_response
        .get("previousAssemblyId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let previous_fingerprint = compile_response
        .get("previousTypeIndexFingerprint")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let current_fingerprint = compile_response
        .get("typeIndexFingerprint")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let types = serde_json::from_value::<Vec<crate::unity_type_index::UnityTypeIndexEntry>>(
        compile_response
            .get("types")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|error| format!("Failed to parse Skill package type index delta: {}", error))?;

    if package_id.is_empty() || source_hash.is_empty() || assembly_id.is_empty() {
        refresh_unity_type_index(project_path).await?;
        return Ok(UnityTypeIndexUpdateResult {
            mode: "full".to_string(),
        });
    }

    let cached = crate::unity_type_index::load_cached_type_index(project_path).await?;
    if let Some(index) = cached.as_ref() {
        if !current_fingerprint.is_empty() && index.fingerprint == current_fingerprint {
            return Ok(UnityTypeIndexUpdateResult {
                mode: "current".to_string(),
            });
        }
    }

    if !previous_fingerprint.is_empty() && !current_fingerprint.is_empty() {
        if let Some(index) = cached.as_ref() {
            if index.fingerprint == previous_fingerprint {
                if crate::unity_type_index::persist_skill_package_type_index_delta(
                    project_path,
                    previous_fingerprint,
                    current_fingerprint,
                    package_id,
                    source_hash,
                    assembly_id,
                    previous_assembly_id,
                    types,
                )
                .await?
                .is_some()
                {
                    return Ok(UnityTypeIndexUpdateResult {
                        mode: "incremental".to_string(),
                    });
                }
            }
        }
    }

    refresh_unity_type_index(project_path).await?;
    Ok(UnityTypeIndexUpdateResult {
        mode: "full".to_string(),
    })
}

async fn unity_type_index_for_execute(
    project_path: &str,
) -> Option<Arc<crate::unity_type_index::UnityTypeIndex>> {
    match crate::unity_type_index::load_cached_type_index(project_path).await {
        Ok(Some(index)) => match cached_unity_type_index_is_current(project_path, &index).await {
            Ok(true) => return Some(index),
            Ok(false) => {
                eprintln!("[Locus] Unity type index cache is stale; refreshing.");
                crate::unity_type_index::invalidate_cached_type_index(project_path).await;
            }
            Err(error) => {
                eprintln!(
                    "[Locus] Unity type index cache validation failed; refreshing: {}",
                    error
                );
                crate::unity_type_index::invalidate_cached_type_index(project_path).await;
            }
        },
        Ok(None) => {}
        Err(error) => eprintln!("[Locus] Unity type index cache ignored: {}", error),
    }

    match refresh_unity_type_index(project_path).await {
        Ok(index) => Some(index),
        Err(error) => {
            eprintln!("[Locus] Unity type index export skipped: {}", error);
            None
        }
    }
}

async fn prepare_unity_execute_code_for_send(
    project_path: &str,
    code: &str,
) -> crate::unity_type_index::PreparedUnityCode {
    let index = unity_type_index_for_execute(project_path).await;
    crate::unity_type_index::prepare_unity_execute_code(code, index.as_deref())
}

async fn prepare_unity_run_states_request_for_send(
    project_path: &str,
    request: &serde_json::Value,
) -> crate::unity_type_index::PreparedUnityRunStatesRequest {
    let index = unity_type_index_for_execute(project_path).await;
    crate::unity_type_index::prepare_unity_run_states_request(request, index.as_deref())
}

pub async fn unity_execute_code_with_progress<F>(
    project_path: &str,
    code: &str,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    let mut rust_progress_revision = 1u64;
    on_progress(rust_unity_execute_progress(
        "Waiting for Locus Unity operation lock",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;

    on_progress(rust_unity_execute_progress(
        "Preparing Unity type index",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let prepared = prepare_unity_execute_code_for_send(project_path, code).await;

    let mut send_attempt = 1u32;
    let resp = loop {
        on_progress(rust_unity_execute_progress(
            if send_attempt == 1 {
                "Sending execute_code to Unity"
            } else {
                "Retrying execute_code after Unity pipe reconnect"
            },
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;

        let execute = send_message_without_timeout(project_path, "execute_code", &prepared.code);
        tokio::pin!(execute);

        let mut progress_tick =
            tokio::time::interval(Duration::from_millis(UNITY_EXECUTE_PROGRESS_POLL_MS));
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_progress_revision = 0u64;
        let mut saw_unity_progress = false;
        let execute_started_at = std::time::Instant::now();
        let mut progress_unavailable_since: Option<std::time::Instant> = None;

        let attempt_result: Result<PipeResponse, String> = loop {
            tokio::select! {
                result = &mut execute => break result,
                _ = progress_tick.tick() => {
                    if let Some(snapshot) = query_unity_execute_progress(project_path).await {
                        progress_unavailable_since = None;
                        if snapshot.active {
                            saw_unity_progress = true;
                        }
                        if snapshot.revision != last_progress_revision {
                            last_progress_revision = snapshot.revision;
                            on_progress(snapshot);
                        }
                    } else if saw_unity_progress {
                        let unavailable_since = progress_unavailable_since
                            .get_or_insert_with(std::time::Instant::now);
                        if unavailable_since.elapsed()
                            > Duration::from_secs(UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS)
                        {
                            let reason = format!(
                                "Unity execute progress was unavailable for {}s; reconnecting Unity pipe",
                                UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS
                            );
                            return Err(append_execute_reconnect_result(
                                &reason,
                                reconnect_unity_pipe_for_execute(project_path, &reason).await,
                            ));
                        }
                    }

                    if !saw_unity_progress
                        && execute_started_at.elapsed()
                            > Duration::from_secs(UNITY_EXECUTE_START_TIMEOUT_SECS)
                    {
                        break Err(format!(
                            "Unity execute did not leave the sending stage within {}s",
                            UNITY_EXECUTE_START_TIMEOUT_SECS
                        ));
                    }
                }
            }
        };

        match attempt_result {
            Ok(resp) => break resp,
            Err(error) if !saw_unity_progress && send_attempt == 1 => {
                on_progress(rust_unity_execute_progress(
                    "Reconnecting Unity pipe",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                if let Err(reconnect_error) =
                    reconnect_unity_pipe_for_execute(project_path, &error).await
                {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Err(error) => {
                return Err(append_execute_reconnect_result(
                    &error,
                    reconnect_unity_pipe_for_execute(project_path, &error).await,
                ));
            }
        }
    };

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error.unwrap_or_else(|| "unknown error".to_string()),
            &prepared,
        ))
    }
}

pub async fn unity_execute_code_with_progress_cancellable<F>(
    project_path: &str,
    code: &str,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    if *cancel_rx.borrow() {
        return Err(UNITY_EXECUTE_CANCELLED.to_string());
    }

    let mut rust_progress_revision = 1u64;
    on_progress(rust_unity_execute_progress(
        "Waiting for Locus Unity operation lock",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = tokio::select! {
        guard = op_lock.lock() => guard,
        _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
    };

    on_progress(rust_unity_execute_progress(
        "Preparing Unity type index",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let prepared = tokio::select! {
        prepared = prepare_unity_execute_code_for_send(project_path, code) => prepared,
        _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
    };

    let mut send_attempt = 1u32;
    let resp = loop {
        on_progress(rust_unity_execute_progress(
            if send_attempt == 1 {
                "Sending execute_code to Unity"
            } else {
                "Retrying execute_code after Unity pipe reconnect"
            },
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;

        let execute = send_message_without_timeout(project_path, "execute_code", &prepared.code);
        tokio::pin!(execute);

        let mut progress_tick =
            tokio::time::interval(Duration::from_millis(UNITY_EXECUTE_PROGRESS_POLL_MS));
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_progress_revision = 0u64;
        let mut saw_unity_progress = false;
        let execute_started_at = std::time::Instant::now();
        let mut progress_unavailable_since: Option<std::time::Instant> = None;

        let attempt_result: Result<PipeResponse, String> = loop {
            tokio::select! {
                result = &mut execute => break result,
                changed = cancel_rx.changed() => {
                    let cancelled = changed.is_err() || *cancel_rx.borrow();
                    if !cancelled {
                        continue;
                    }

                    if let Err(error) = cancel_unity_execute_code(project_path).await {
                        eprintln!("[Locus] cancel_execute_code skipped: {}", error);
                    }

                    let drain = tokio::time::sleep(Duration::from_secs(5));
                    tokio::pin!(drain);
                    loop {
                        tokio::select! {
                            result = &mut execute => {
                                if let Err(error) = result {
                                    eprintln!("[Locus] execute_code after cancel ended with transport error: {}", error);
                                }
                                break;
                            },
                            _ = &mut drain => {
                                eprintln!("[Locus] execute_code cancel drain timed out");
                                transport::disconnect_with_reason(
                                    project_path,
                                    "execute_code cancel drain timed out",
                                ).await;
                                break;
                            },
                            _ = progress_tick.tick() => {
                                if let Some(snapshot) = query_unity_execute_progress(project_path).await {
                                    if snapshot.revision != last_progress_revision {
                                        last_progress_revision = snapshot.revision;
                                        on_progress(snapshot);
                                    }
                                }
                            }
                        }
                    }

                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                },
                _ = progress_tick.tick() => {
                    if let Some(snapshot) = query_unity_execute_progress(project_path).await {
                        progress_unavailable_since = None;
                        if snapshot.active {
                            saw_unity_progress = true;
                        }
                        if snapshot.revision != last_progress_revision {
                            last_progress_revision = snapshot.revision;
                            on_progress(snapshot);
                        }
                    } else if saw_unity_progress {
                        let unavailable_since = progress_unavailable_since
                            .get_or_insert_with(std::time::Instant::now);
                        if unavailable_since.elapsed()
                            > Duration::from_secs(UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS)
                        {
                            let reason = format!(
                                "Unity execute progress was unavailable for {}s; reconnecting Unity pipe",
                                UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS
                            );
                            let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                                project_path,
                                &reason,
                                &mut cancel_rx,
                            )
                            .await;
                            if reconnect
                                .as_ref()
                                .err()
                                .map(|error| error == UNITY_EXECUTE_CANCELLED)
                                .unwrap_or(false)
                            {
                                return Err(UNITY_EXECUTE_CANCELLED.to_string());
                            }
                            return Err(append_execute_reconnect_result(&reason, reconnect));
                        }
                    }

                    if !saw_unity_progress
                        && execute_started_at.elapsed()
                            > Duration::from_secs(UNITY_EXECUTE_START_TIMEOUT_SECS)
                    {
                        break Err(format!(
                            "Unity execute did not leave the sending stage within {}s",
                            UNITY_EXECUTE_START_TIMEOUT_SECS
                        ));
                    }
                }
            }
        };

        match attempt_result {
            Ok(resp) => break resp,
            Err(error) if !saw_unity_progress && send_attempt == 1 => {
                on_progress(rust_unity_execute_progress(
                    "Reconnecting Unity pipe",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                if let Err(reconnect_error) = reconnect {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Err(error) => {
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                return Err(append_execute_reconnect_result(&error, reconnect));
            }
        }
    };

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error.unwrap_or_else(|| "unknown error".to_string()),
            &prepared,
        ))
    }
}

pub async fn unity_execute_code(project_path: &str, code: &str) -> Result<String, String> {
    unity_execute_code_with_progress(project_path, code, |_| {}).await
}

async fn wait_for_unity_bridge_ready_after_recompile(project_path: &str) -> Result<(), String> {
    wait_for_unity_bridge_ready(project_path, Duration::from_secs(30), "after recompile").await
}

async fn refresh_unity_type_index_after_recompile(project_path: &str) -> Result<(), String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_error = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match refresh_unity_type_index(project_path).await {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = error;
                eprintln!(
                    "[Locus] Unity type index refresh after recompile attempt {}/{} failed: {}",
                    attempt, MAX_ATTEMPTS, last_error
                );
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    Err(last_error)
}

/// Trigger a Unity recompile and wait until the new domain is ready.
///
/// Flow:
/// 1. Release every edit session so Unity can see the full batch of file writes.
/// 2. Send `request_recompile`.
/// 3. Poll `get_compile_result`.
///    - `pending`: compilation or reload is still in progress.
///    - `ok`: compilation succeeded and the reloaded AppDomain reported completion.
///    - `error:*`: compilation failed; surface the compiler errors immediately.
/// 4. If the pipe drops during reload, wait for Unity to reconnect as a fallback signal.
pub async fn recompile_and_wait(project_path: &str) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let prev_foreground = focus::bring_unity_to_foreground();

    let finish = |result: Result<String, String>| -> Result<String, String> {
        if let Some(hwnd) = prev_foreground {
            focus::restore_foreground(hwnd);
        }
        result
    };

    if let Err(e) = end_edit_session(project_path, "").await {
        eprintln!(
            "[Locus] failed to end edit sessions before recompile (continuing): {}",
            e
        );
    }

    let resp = match send_message(project_path, "request_recompile", "").await {
        Ok(resp) => resp,
        Err(error) => return finish(Err(error)),
    };
    if !resp.ok {
        return finish(Err(resp
            .error
            .unwrap_or_else(|| "request_recompile failed".to_string())));
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    let max_wait = Duration::from_secs(120);
    let start = std::time::Instant::now();
    let mut disconnected = false;

    loop {
        if start.elapsed() > max_wait {
            return finish(Err("Compilation timed out (120s)".to_string()));
        }

        if disconnected {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match send_message(project_path, "ping", "").await {
                Ok(resp) if resp.ok => {
                    eprintln!("[Locus] Unity reconnected after domain reload");
                    crate::unity_type_index::invalidate_cached_type_index(project_path).await;
                    if let Err(error) =
                        wait_for_unity_bridge_ready_after_recompile(project_path).await
                    {
                        return finish(Err(error));
                    }
                    if let Err(error) = refresh_unity_type_index_after_recompile(project_path).await
                    {
                        eprintln!(
                            "[Locus] Unity type index refresh after recompile skipped: {}",
                            error
                        );
                    }
                    return finish(Ok(
                        "Compilation succeeded, domain reload complete".to_string()
                    ));
                }
                _ => continue,
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
        match send_message(project_path, "get_compile_result", "").await {
            Ok(resp) => {
                if resp.ok {
                    let msg = resp.message.unwrap_or_default();
                    match msg.as_str() {
                        "pending" => continue,
                        "ok" => {
                            crate::unity_type_index::invalidate_cached_type_index(project_path)
                                .await;
                            if let Err(error) =
                                wait_for_unity_bridge_ready_after_recompile(project_path).await
                            {
                                return finish(Err(error));
                            }
                            if let Err(error) =
                                refresh_unity_type_index_after_recompile(project_path).await
                            {
                                eprintln!(
                                    "[Locus] Unity type index refresh after recompile skipped: {}",
                                    error
                                );
                            }
                            return finish(Ok(
                                "Compilation succeeded, domain reload complete".to_string()
                            ));
                        }
                        other => {
                            eprintln!("[Locus] unexpected compile result: {}", other);
                            continue;
                        }
                    }
                } else {
                    return finish(Err(resp
                        .error
                        .unwrap_or_else(|| "Compilation failed (unknown error)".to_string())));
                }
            }
            Err(_) => {
                disconnected = true;
                transport::disconnect(project_path).await;
                eprintln!("[Locus] Unity disconnected during recompile, waiting for reconnect...");
            }
        }
    }
}

pub async fn start_unity_monitor(
    app_handle: AppHandle,
    project_path: String,
    monitor: &UnityMonitorHandle,
) {
    stop_unity_monitor(monitor).await;
    set_event_app_handle(app_handle.clone());

    let pipe_name = get_pipe_name(&project_path);
    eprintln!(
        "[Locus] Unity project detected, starting connection monitor (pipe: {})",
        pipe_name
    );

    let handle = tauri::async_runtime::spawn(async move {
        let mut last_status: Option<bool> = None;
        let mut disconnected_attempts: u32 = 0;

        loop {
            let mut status = query_unity_connection_status(&project_path).await;
            let connected = status.connected;

            if connected {
                if last_status != Some(true) {
                    eprintln!("[Locus] Unity Editor connected! (pipe: {})", pipe_name);
                }
                disconnected_attempts = 0;
            } else {
                disconnected_attempts = disconnected_attempts.saturating_add(1);
                status.reconnect_attempts = disconnected_attempts;

                match status.last_error.as_deref() {
                    Some(error) if last_status != Some(false) => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity Editor not connected (pipe: {}): {}",
                            pipe_name,
                            error
                        );
                    }
                    Some(error) if disconnected_attempts % 10 == 0 => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity reconnect still failing after {} attempt(s) (pipe: {}): {}",
                            disconnected_attempts,
                            pipe_name,
                            error
                        );
                    }
                    None if last_status != Some(false) => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity Editor not connected (pipe: {}): status returned disconnected",
                            pipe_name
                        );
                    }
                    None => {}
                    _ => {}
                }
            }

            if connected {
                status.reconnect_attempts = 0;
            }

            let _ = app_handle.emit("unity-connection-status-detail", status.clone());

            if last_status != Some(connected) {
                last_status = Some(connected);
                let _ = app_handle.emit("unity-connection-status", connected);
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    monitor.lock().await.replace(handle);
}

pub async fn stop_unity_monitor(monitor: &UnityMonitorHandle) {
    if let Some(handle) = monitor.lock().await.take() {
        handle.abort();
        eprintln!("[Locus] Unity connection monitor stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        read_project_unity_version, requested_run_states_editor_status,
        rewrite_run_states_output_for_size,
    };
    use serde_json::json;

    fn result_file(summary: &str) -> String {
        summary
            .lines()
            .find_map(|line| line.strip_prefix("result_file: "))
            .expect("result_file field")
            .to_string()
    }

    #[test]
    fn read_project_unity_version_extracts_editor_version() {
        let project = tempfile::tempdir().expect("temp project");
        let settings_dir = project.path().join("ProjectSettings");
        std::fs::create_dir_all(&settings_dir).expect("create settings dir");
        std::fs::write(
            settings_dir.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\nm_EditorVersionWithRevision: 2022.3.47f1 (88c277b85d21)\n",
        )
        .expect("write version");

        let version =
            read_project_unity_version(&project.path().to_string_lossy()).expect("read version");
        assert_eq!(version.as_deref(), Some("2022.3.47f1"));
    }

    #[test]
    fn run_states_requested_editor_status_accepts_supported_statuses() {
        let request = json!({ "request_editor_status": " playing_paused " });

        assert_eq!(
            requested_run_states_editor_status(&request).unwrap(),
            "playing_paused"
        );
    }

    #[test]
    fn run_states_requested_editor_status_rejects_missing_or_invalid_status() {
        assert!(requested_run_states_editor_status(&json!({}))
            .unwrap_err()
            .contains("Missing required parameter"));

        assert!(requested_run_states_editor_status(&json!({
            "request_editor_status": "disconnected"
        }))
        .unwrap_err()
        .contains("Invalid request_editor_status"));

        assert!(requested_run_states_editor_status(&json!({
            "request_editor_status": "compiling"
        }))
        .unwrap_err()
        .contains("Invalid request_editor_status"));
    }

    #[test]
    fn run_states_small_print_output_stays_inline() {
        let output = [
            "status: ok",
            "final_state: done",
            "print_lines: 2",
            "print_tokens_estimate: 2",
            "prints:",
            "a",
            "b",
        ]
        .join("\n");

        let rewritten = rewrite_run_states_output_for_size("C:/Project", output.clone()).unwrap();
        assert_eq!(rewritten, output);
    }

    #[test]
    fn run_states_large_print_output_is_saved_under_project_library() {
        let project = tempfile::tempdir().expect("temp project");
        let output = [
            "status: ok",
            "final_state: done",
            "print_lines: 12000",
            "print_tokens_estimate: 100001",
            "prints:",
            "large output",
        ]
        .join("\n");

        let rewritten =
            rewrite_run_states_output_for_size(&project.path().to_string_lossy(), output.clone())
                .unwrap();
        assert!(rewritten.contains("print_output: too large"));
        assert!(rewritten.contains("print_lines: 12000"));
        assert!(rewritten.contains("print_tokens_estimate: 100001"));

        let path = result_file(&rewritten);
        assert!(path
            .replace('\\', "/")
            .contains("/Library/Locus/RunStates/"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), output);
    }

    #[test]
    fn run_states_hard_limit_returns_too_large_without_saving() {
        let project = tempfile::tempdir().expect("temp project");
        let output = [
            "status: error",
            "final_state: done",
            "print_lines: 90000",
            "print_tokens_estimate: 1000001",
            "print_output: too large",
        ]
        .join("\n");

        let error = rewrite_run_states_output_for_size(&project.path().to_string_lossy(), output)
            .unwrap_err();
        assert!(error.contains("print_output: too large"));
        assert!(error.contains("print_lines: 90000"));
        assert!(error.contains("result was not saved"));
        assert!(!project.path().join("Library").join("Locus").exists());
    }
}
