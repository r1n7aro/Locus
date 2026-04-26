mod focus;
mod plugin;
mod transport;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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
pub use transport::{send_message, send_message_with_timeout, send_message_without_timeout};

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
    let _prev_foreground = focus::bring_unity_to_foreground();
    let payload = serde_json::to_string(&SceneObjectRequest {
        scene_path,
        object_path,
    })
    .map_err(|e| e.to_string())?;
    let resp = send_message(project_path, "select_scene_object", &payload).await?;
    if resp.ok {
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
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize unity_run_states request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "run_states", &payload).await?;
    let output = if resp.ok {
        resp.message.unwrap_or_default()
    } else {
        resp.error
            .unwrap_or_else(|| "unity_run_states failed".to_string())
    };

    let rewritten = rewrite_run_states_output_for_size(project_path, output)?;
    if resp.ok {
        Ok(rewritten)
    } else {
        Err(rewritten)
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
        "{\"active\":false,\"title\":\"\",\"info\":\"\",\"progress\":0,\"revision\":0}".to_string()
    });
    format!(
        "<{tag}>{payload}</{tag}>\n",
        tag = UNITY_EXECUTE_PROGRESS_TAG,
        payload = payload
    )
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

pub async fn unity_execute_code_with_progress<F>(
    project_path: &str,
    code: &str,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;

    let execute = send_message_without_timeout(project_path, "execute_code", code);
    tokio::pin!(execute);

    let mut progress_tick = tokio::time::interval(Duration::from_millis(250));
    progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_progress_revision = 0u64;

    let resp = loop {
        tokio::select! {
            result = &mut execute => break result?,
            _ = progress_tick.tick() => {
                if let Some(snapshot) = query_unity_execute_progress(project_path).await {
                    if snapshot.revision != last_progress_revision {
                        last_progress_revision = snapshot.revision;
                        on_progress(snapshot);
                    }
                }
            }
        }
    };

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
}

pub async fn unity_execute_code(project_path: &str, code: &str) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message_without_timeout(project_path, "execute_code", code).await?;
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

#[cfg(test)]
mod tests {
    use super::rewrite_run_states_output_for_size;

    fn result_file(summary: &str) -> String {
        summary
            .lines()
            .find_map(|line| line.strip_prefix("result_file: "))
            .expect("result_file field")
            .to_string()
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
