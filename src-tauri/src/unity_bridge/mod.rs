mod focus;
mod plugin;
mod transport;

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

pub use plugin::{
    check_plugin_status, emit_plugin_status, find_plugin_source_dir, install_or_update_plugin,
    PluginStatus,
};
pub use transport::send_message;

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

pub async fn is_unity_connected(project_path: &str) -> bool {
    match send_message(project_path, "ping", "").await {
        Ok(resp) => resp.ok,
        Err(_) => false,
    }
}

/// Canonical status values: "disconnected" | "editing" | "playing" | "playing_paused"
pub async fn query_unity_status(project_path: &str) -> (bool, &'static str, Option<String>) {
    match send_message(project_path, "status", "").await {
        Ok(resp) if resp.ok => {
            let msg = resp.message.unwrap_or_default();
            let (status_part, scene_part) = match msg.split_once('|') {
                Some((s, scene)) => (s, Some(scene.to_string())),
                None => (msg.as_str(), None),
            };
            let status = normalize_editor_status(status_part);
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

pub async fn unity_execute_code(project_path: &str, code: &str) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message(project_path, "execute_code", code).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
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

    let resp = send_message(project_path, "request_recompile", "").await?;
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

    let pipe_name = get_pipe_name(&project_path);
    eprintln!(
        "[Locus] Unity project detected, starting connection monitor (pipe: {})",
        pipe_name
    );

    let handle = tauri::async_runtime::spawn(async move {
        let mut last_status: Option<bool> = None;

        loop {
            let result = send_message(&project_path, "ping", "").await;
            let connected = matches!(&result, Ok(resp) if resp.ok);

            match &result {
                Ok(_) if connected => {
                    if last_status != Some(true) {
                        eprintln!("[Locus] Unity Editor connected! (pipe: {})", pipe_name);
                    }
                }
                Ok(resp) => {
                    eprintln!(
                        "[Locus] Unity ping ok=false, error: {:?} (pipe: {})",
                        resp.error, pipe_name
                    );
                }
                Err(e) => {
                    if last_status != Some(false) {
                        eprintln!(
                            "[Locus] Unity Editor not connected (pipe: {}): {}",
                            pipe_name, e
                        );
                    }
                }
            }

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
