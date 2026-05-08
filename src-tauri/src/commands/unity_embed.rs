use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

use crate::error::AppError;
use crate::workspace::Workspace;

const WINDOW_LABEL: &str = "unity-embed";
const CONTROL_PIPE_NAME_PREFIX: &str = r"\\.\pipe\locus_tauri_unity_embed_";
const EMBED_URL: &str = "/unity-embed?host=tauri-overlay";
const CLOSE_REASON_DOMAIN_RELOAD: &str = "domainReload";
const TRANSIENT_CLOSE_DESTROY_DELAY: Duration = Duration::from_secs(30);
const ASSET_DRAG_CACHE_TTL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnityEmbedControlMessage {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    parent_hwnd: i64,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    asset_refs: Option<Vec<UnityEmbedAssetRef>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnityEmbedAssetRef {
    path: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    type_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnityEmbedAssetDropPayload {
    refs: Vec<UnityEmbedAssetRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnityEmbedAssetDragStatePayload {
    has_refs: bool,
    refs: Vec<UnityEmbedAssetRef>,
}

#[derive(Default)]
struct UnityEmbedAssetDragCache {
    refs: Vec<UnityEmbedAssetRef>,
    updated_at: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityEmbedStatus {
    pub ok: bool,
    pub runtime: String,
    pub message: String,
    pub pipe_name: String,
    pub window_label: String,
    pub control: UnityEmbedControlSnapshot,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityEmbedControlSnapshot {
    pub update_count: u64,
    pub last_type: String,
    pub last_rect: String,
    pub last_parent_hwnd: i64,
    pub last_child_hwnd: i64,
    pub last_visible: bool,
    pub last_mounted: bool,
    pub last_error: String,
    pub last_update_ms_ago: Option<u128>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityEmbedFocusDebugSnapshot {
    pub ok: bool,
    pub reason: String,
    pub foreground_hwnd: i64,
    pub foreground_title: String,
    pub overlay_hwnd: i64,
    pub overlay_title: String,
    pub overlay_visible: bool,
    pub overlay_foreground: bool,
    pub overlay_child_window: bool,
    pub overlay_parent_hwnd: i64,
    pub overlay_no_activate: bool,
    pub activation_guard_enabled: bool,
    pub mouse_activate_hook_installed: bool,
    pub mouse_activate_hooked_hwnd_count: usize,
    pub mouse_activate_block_count: u64,
    pub mouse_activation_suppressed: bool,
    pub parent_hwnd: i64,
    pub parent_title: String,
    pub parent_visible: bool,
    pub parent_foreground: bool,
}

#[derive(Debug, Default)]
struct UnityEmbedControlState {
    update_count: u64,
    last_type: String,
    last_rect: String,
    last_parent_hwnd: i64,
    last_child_hwnd: i64,
    last_visible: bool,
    last_mounted: bool,
    last_error: String,
    last_update_at: Option<Instant>,
}

#[derive(Debug, Default)]
struct UnityEmbedAppliedState {
    has_window: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent_hwnd: i64,
    visible: bool,
}

#[derive(Debug, Default)]
struct UnityEmbedTransientCloseState {
    generation: u64,
}

fn default_visible() -> bool {
    true
}

fn control_state() -> &'static Mutex<UnityEmbedControlState> {
    static STATE: OnceLock<Mutex<UnityEmbedControlState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(UnityEmbedControlState::default()))
}

fn applied_state() -> &'static Mutex<UnityEmbedAppliedState> {
    static STATE: OnceLock<Mutex<UnityEmbedAppliedState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(UnityEmbedAppliedState::default()))
}

fn transient_close_state() -> &'static Mutex<UnityEmbedTransientCloseState> {
    static STATE: OnceLock<Mutex<UnityEmbedTransientCloseState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(UnityEmbedTransientCloseState::default()))
}

fn record_control_message(msg: &UnityEmbedControlMessage) {
    if let Ok(mut state) = control_state().lock() {
        state.update_count = state.update_count.saturating_add(1);
        state.last_type = msg.kind.clone();
        state.last_rect = format!("{} {} {} {}", msg.x, msg.y, msg.width, msg.height);
        state.last_parent_hwnd = msg.parent_hwnd;
        state.last_visible = msg.visible;
        state.last_update_at = Some(Instant::now());
    }
}

fn record_child_hwnd(hwnd: i64) {
    if let Ok(mut state) = control_state().lock() {
        state.last_child_hwnd = hwnd;
    }
}

fn record_mount_result(mounted: bool, error: Option<String>) {
    if let Ok(mut state) = control_state().lock() {
        state.last_mounted = mounted;
        state.last_error = error.unwrap_or_default();
    }
}

fn next_transient_close_generation() -> u64 {
    transient_close_state()
        .lock()
        .map(|mut state| {
            state.generation = state.generation.saturating_add(1);
            state.generation
        })
        .unwrap_or(0)
}

fn cancel_transient_close_destroy() {
    let _ = next_transient_close_generation();
}

fn is_transient_close_generation_current(generation: u64) -> bool {
    transient_close_state()
        .lock()
        .map(|state| state.generation == generation)
        .unwrap_or(false)
}

fn is_transient_close_reason(reason: &str) -> bool {
    reason == CLOSE_REASON_DOMAIN_RELOAD
}

fn schedule_transient_close_destroy(app_handle: &AppHandle) {
    let generation = next_transient_close_generation();
    let app_for_timer = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(TRANSIENT_CLOSE_DESTROY_DELAY).await;
        if !is_transient_close_generation_current(generation) {
            return;
        }

        let app_for_main = app_for_timer.clone();
        if let Err(error) = app_for_timer.run_on_main_thread(move || {
            if is_transient_close_generation_current(generation) {
                destroy_unity_embed_control_window_on_main(&app_for_main);
            }
        }) {
            eprintln!("[Locus] failed to dispatch Unity embed transient close cleanup: {error}");
        }
    });
}

fn needs_geometry_apply(msg: &UnityEmbedControlMessage) -> bool {
    if let Ok(state) = applied_state().lock() {
        return !state.has_window
            || state.x != msg.x
            || state.y != msg.y
            || state.width != msg.width
            || state.height != msg.height
            || state.parent_hwnd != msg.parent_hwnd;
    }

    true
}

fn needs_visibility_apply(visible: bool) -> bool {
    if let Ok(state) = applied_state().lock() {
        return !state.has_window || state.visible != visible;
    }

    true
}

fn record_applied_geometry(msg: &UnityEmbedControlMessage) {
    if let Ok(mut state) = applied_state().lock() {
        state.has_window = true;
        state.x = msg.x;
        state.y = msg.y;
        state.width = msg.width;
        state.height = msg.height;
        state.parent_hwnd = msg.parent_hwnd;
    }
}

fn record_applied_visibility(visible: bool) {
    if let Ok(mut state) = applied_state().lock() {
        state.has_window = true;
        state.visible = visible;
    }
}

fn record_window_destroyed() {
    if let Ok(mut state) = applied_state().lock() {
        *state = UnityEmbedAppliedState::default();
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::disable_popup_sync();
        windows_impl::remove_mouse_activate_hook();
        windows_impl::reset_mouse_activation_suppressed();
    }
}

fn should_show_window_now(window: &tauri::WebviewWindow, msg: &UnityEmbedControlMessage) -> bool {
    #[cfg(target_os = "windows")]
    if msg.parent_hwnd > 0 {
        return msg.visible && windows_impl::is_overlay_parent_visible(window, msg);
    }

    msg.visible
}

fn control_snapshot() -> UnityEmbedControlSnapshot {
    let now = Instant::now();
    if let Ok(state) = control_state().lock() {
        return UnityEmbedControlSnapshot {
            update_count: state.update_count,
            last_type: state.last_type.clone(),
            last_rect: state.last_rect.clone(),
            last_parent_hwnd: state.last_parent_hwnd,
            last_child_hwnd: state.last_child_hwnd,
            last_visible: state.last_visible,
            last_mounted: state.last_mounted,
            last_error: state.last_error.clone(),
            last_update_ms_ago: state
                .last_update_at
                .map(|updated_at| now.duration_since(updated_at).as_millis()),
        };
    }

    UnityEmbedControlSnapshot::default()
}

fn strip_extended_path_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

fn normalize_pipe_project_path(path: &str) -> String {
    let trimmed = strip_extended_path_prefix(path).trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized_path = dunce::canonicalize(trimmed)
        .unwrap_or_else(|_| Path::new(trimmed).to_path_buf())
        .to_string_lossy()
        .to_string();
    let normalized_path = strip_extended_path_prefix(&normalized_path);

    normalized_path
        .trim_end_matches(|ch| ch == '/' || ch == '\\')
        .replace('\\', "_")
        .replace('/', "_")
        .replace(':', "_")
        .replace(' ', "_")
}

fn control_pipe_name_for_project_path(project_path: &str) -> String {
    let sanitized = normalize_pipe_project_path(project_path);
    let suffix = if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    };
    format!("{CONTROL_PIPE_NAME_PREFIX}{suffix}")
}

async fn current_workspace_path(app_handle: &AppHandle) -> String {
    match app_handle.try_state::<Arc<Workspace>>() {
        Some(workspace) => workspace.path.read().await.clone(),
        None => String::new(),
    }
}

pub(crate) fn handle_unity_embed_webview_event(
    webview: &tauri::Webview,
    event: &tauri::WebviewEvent,
) {
    if webview.label() != WINDOW_LABEL {
        return;
    }

    let paths = match event {
        tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => paths.clone(),
        _ => return,
    };
    if paths.is_empty() {
        return;
    }

    let app_handle = webview.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let workspace_path = current_workspace_path(&app_handle).await;
        let refs = unity_file_drop_asset_refs(&workspace_path, &paths);
        if refs.is_empty() {
            return;
        }
        if let Err(error) = emit_unity_embed_asset_drop(&app_handle, refs) {
            eprintln!("[Locus] failed to emit Unity embed file drop: {error}");
        }
    });
}

fn emit_unity_embed_asset_drop(
    app_handle: &AppHandle,
    refs: Vec<UnityEmbedAssetRef>,
) -> Result<(), String> {
    app_handle
        .emit_to(
            WINDOW_LABEL,
            "unity-embed-asset-drop",
            UnityEmbedAssetDropPayload { refs },
        )
        .map_err(|error| format!("Failed to emit Unity embed asset drop: {error}"))
}

fn emit_unity_embed_asset_drag_state(
    app_handle: &AppHandle,
    refs: Vec<UnityEmbedAssetRef>,
) -> Result<(), String> {
    app_handle
        .emit_to(
            WINDOW_LABEL,
            "unity-embed-asset-drag-state",
            UnityEmbedAssetDragStatePayload {
                has_refs: !refs.is_empty(),
                refs,
            },
        )
        .map_err(|error| format!("Failed to emit Unity embed asset drag state: {error}"))
}

fn asset_drag_cache() -> &'static Mutex<UnityEmbedAssetDragCache> {
    static CACHE: OnceLock<Mutex<UnityEmbedAssetDragCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(UnityEmbedAssetDragCache::default()))
}

fn cache_unity_embed_asset_drag_refs(refs: Vec<UnityEmbedAssetRef>) {
    let Ok(mut cache) = asset_drag_cache().lock() else {
        return;
    };

    if refs.is_empty() {
        cache.refs.clear();
        cache.updated_at = None;
        return;
    }

    cache.refs = refs;
    cache.updated_at = Some(Instant::now());
}

fn current_unity_embed_asset_drag_refs() -> Vec<UnityEmbedAssetRef> {
    let Ok(cache) = asset_drag_cache().lock() else {
        return Vec::new();
    };

    let Some(updated_at) = cache.updated_at else {
        return Vec::new();
    };
    if updated_at.elapsed() > ASSET_DRAG_CACHE_TTL {
        return Vec::new();
    }

    cache.refs.clone()
}

#[tauri::command]
pub async fn unity_embed_commit_asset_drop(app_handle: AppHandle) -> Result<(), AppError> {
    let refs = current_unity_embed_asset_drag_refs();
    if refs.is_empty() {
        return Ok(());
    }

    emit_unity_embed_asset_drop(&app_handle, refs).map_err(AppError::from)?;
    cache_unity_embed_asset_drag_refs(Vec::new());
    emit_unity_embed_asset_drag_state(&app_handle, Vec::new()).map_err(AppError::from)
}

fn unity_file_drop_asset_refs(workspace_path: &str, paths: &[PathBuf]) -> Vec<UnityEmbedAssetRef> {
    let mut seen = HashSet::new();
    let mut refs = Vec::new();

    for path in paths {
        let Some(asset_ref) = unity_file_drop_asset_ref(workspace_path, path) else {
            continue;
        };
        let key = format!("{}\n{}", asset_ref.kind, asset_ref.path);
        if seen.insert(key) {
            refs.push(asset_ref);
        }
    }

    refs
}

fn unity_file_drop_asset_ref(workspace_path: &str, path: &Path) -> Option<UnityEmbedAssetRef> {
    let relative_path = unity_relative_drop_path(workspace_path, path)?;
    let name = unity_drop_name(path, &relative_path);
    let type_label = unity_drop_type_label(path);
    Some(UnityEmbedAssetRef {
        path: relative_path,
        kind: "asset".to_string(),
        name,
        type_label,
        source: Some("unity".to_string()),
    })
}

fn unity_relative_drop_path(workspace_path: &str, path: &Path) -> Option<String> {
    let raw_path = normalize_unity_path_text(&path.to_string_lossy());
    if is_supported_unity_ref_path(&raw_path) {
        return Some(raw_path);
    }
    if workspace_path.trim().is_empty() {
        return None;
    }

    let workspace_path = normalize_existing_path_text(Path::new(workspace_path));
    if workspace_path.is_empty() {
        return None;
    }

    let dropped_path = normalize_existing_path_text(path);
    let workspace_len = workspace_path.len();
    let is_direct_child = dropped_path.len() > workspace_len
        && dropped_path[..workspace_len].eq_ignore_ascii_case(&workspace_path)
        && dropped_path.as_bytes().get(workspace_len) == Some(&b'/');
    if !is_direct_child {
        return None;
    }

    let relative_path = normalize_unity_path_text(&dropped_path[workspace_len + 1..]);
    if is_supported_unity_ref_path(&relative_path) {
        Some(relative_path)
    } else {
        None
    }
}

fn normalize_existing_path_text(path: &Path) -> String {
    let normalized = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_unity_path_text(&normalized.to_string_lossy())
}

fn normalize_unity_path_text(path: &str) -> String {
    strip_extended_path_prefix(path)
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn is_supported_unity_ref_path(path: &str) -> bool {
    let normalized = normalize_unity_path_text(path);
    let lower = normalized.to_ascii_lowercase();
    matches!(lower.as_str(), "assets" | "packages" | "projectsettings")
        || lower.starts_with("assets/")
        || lower.starts_with("packages/")
        || lower.starts_with("projectsettings/")
}

fn unity_drop_name(path: &Path, relative_path: &str) -> Option<String> {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            relative_path
                .rsplit('/')
                .next()
                .map(|name| name.to_string())
                .filter(|name| !name.is_empty())
        })
}

fn unity_drop_type_label(path: &Path) -> Option<String> {
    if path.is_dir() {
        return Some("Folder".to_string());
    }

    path.extension()
        .map(|extension| extension.to_string_lossy().to_string())
        .filter(|extension| !extension.is_empty())
}

async fn current_control_pipe_name(app_handle: &AppHandle) -> Option<String> {
    let current_project_path = current_workspace_path(app_handle).await;
    if current_project_path.trim().is_empty() {
        None
    } else {
        Some(control_pipe_name_for_project_path(&current_project_path))
    }
}

async fn is_current_control_pipe(app_handle: &AppHandle, pipe_name: &str) -> bool {
    current_control_pipe_name(app_handle)
        .await
        .as_deref()
        .map(|current| current == pipe_name)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn unity_embed_status(app_handle: AppHandle) -> Result<UnityEmbedStatus, AppError> {
    let pipe_name = current_control_pipe_name(&app_handle)
        .await
        .unwrap_or_default();
    Ok(UnityEmbedStatus {
        ok: true,
        runtime: "tauri".to_string(),
        message: "pong".to_string(),
        pipe_name,
        window_label: WINDOW_LABEL.to_string(),
        control: control_snapshot(),
    })
}

#[tauri::command]
pub async fn unity_embed_set_mouse_activation_suppressed(
    app_handle: AppHandle,
    suppressed: bool,
) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let app_for_main = app_handle.clone();
        app_handle
            .run_on_main_thread(move || {
                let result = windows_impl::set_mouse_activation_suppressed(
                    app_for_main.get_webview_window(WINDOW_LABEL).as_ref(),
                    suppressed,
                );
                let _ = tx.send(result);
            })
            .map_err(|error| {
                format!("Failed to dispatch Unity embed activation update: {error}")
            })?;

        rx.await
            .map_err(|_| "Unity embed activation update was cancelled".to_string())??;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
        let _ = suppressed;
    }

    Ok(())
}

#[tauri::command]
pub async fn unity_embed_activate_for_input(app_handle: AppHandle) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let app_for_main = app_handle.clone();
        app_handle
            .run_on_main_thread(move || {
                let result = windows_impl::activate_for_input(
                    app_for_main.get_webview_window(WINDOW_LABEL).as_ref(),
                );
                let _ = tx.send(result);
            })
            .map_err(|error| format!("Failed to dispatch Unity embed input activation: {error}"))?;

        rx.await
            .map_err(|_| "Unity embed input activation was cancelled".to_string())??;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
    }

    Ok(())
}

#[tauri::command]
pub async fn unity_embed_focus_debug_snapshot(
    app_handle: AppHandle,
) -> Result<UnityEmbedFocusDebugSnapshot, AppError> {
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let app_for_main = app_handle.clone();
        app_handle
            .run_on_main_thread(move || {
                let result = windows_impl::focus_debug_snapshot(
                    app_for_main.get_webview_window(WINDOW_LABEL).as_ref(),
                );
                let _ = tx.send(result);
            })
            .map_err(|error| {
                format!("Failed to dispatch Unity embed focus debug snapshot: {error}")
            })?;

        let snapshot = rx
            .await
            .map_err(|_| "Unity embed focus debug snapshot was cancelled".to_string())?;
        return Ok(snapshot);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
        Ok(UnityEmbedFocusDebugSnapshot {
            ok: false,
            reason: "focus debug is only available on Windows".to_string(),
            ..UnityEmbedFocusDebugSnapshot::default()
        })
    }
}

pub(crate) fn start_unity_embed_control_server(app_handle: AppHandle) {
    #[cfg(target_os = "windows")]
    windows_impl::start(app_handle);

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
    }
}

pub(crate) fn refresh_unity_embed_control_server(app_handle: AppHandle) {
    #[cfg(target_os = "windows")]
    windows_impl::refresh(app_handle);

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
    }
}

pub(crate) fn reset_unity_embed_control_window(app_handle: &AppHandle) {
    let app_for_main = app_handle.clone();
    if let Err(error) = app_handle.run_on_main_thread(move || {
        destroy_unity_embed_control_window_on_main(&app_for_main);
    }) {
        eprintln!("[Locus] failed to dispatch Unity embed reset: {error}");
    }
}

pub(crate) fn destroy_unity_embed_control_window_on_main(app_handle: &AppHandle) {
    cancel_transient_close_destroy();
    if let Some(window) = app_handle.get_webview_window(WINDOW_LABEL) {
        if let Err(close_error) = window.destroy().or_else(|_| window.close()) {
            eprintln!("[Locus] failed to destroy Unity embed window: {close_error}");
        }
    }
    record_window_destroyed();
}

fn normalized_rect(msg: &UnityEmbedControlMessage) -> (i32, i32, u32, u32) {
    (
        msg.x,
        msg.y,
        msg.width.max(1) as u32,
        msg.height.max(1) as u32,
    )
}

fn ensure_embed_window(
    app_handle: &AppHandle,
    msg: &UnityEmbedControlMessage,
) -> Result<(tauri::WebviewWindow, bool), String> {
    if let Some(window) = app_handle.get_webview_window(WINDOW_LABEL) {
        #[cfg(target_os = "windows")]
        if let Ok(hwnd) = window.hwnd() {
            record_child_hwnd(hwnd.0 as isize as i64);
        }
        return Ok((window, false));
    }

    let (x, y, width, height) = normalized_rect(msg);
    let builder = tauri::WebviewWindowBuilder::new(
        app_handle,
        WINDOW_LABEL,
        WebviewUrl::App(EMBED_URL.into()),
    )
    .title("Locus")
    .position(x as f64, y as f64)
    .inner_size(width as f64, height as f64)
    .decorations(false)
    .resizable(false)
    .shadow(false)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .disable_drag_drop_handler();

    let window = builder
        .build()
        .map_err(|error| format!("Failed to create Unity embed window: {error}"))?;

    #[cfg(target_os = "windows")]
    {
        if let Ok(hwnd) = window.hwnd() {
            record_child_hwnd(hwnd.0 as isize as i64);
        }
    }

    Ok((window, true))
}

fn apply_control_message_on_main(
    app_handle: &AppHandle,
    msg: UnityEmbedControlMessage,
) -> Result<(), String> {
    if msg.kind != "assetDrop" && msg.kind != "assetDrag" {
        record_control_message(&msg);
    }
    match msg.kind.as_str() {
        "open" | "update" => {
            cancel_transient_close_destroy();
            let (window, created) = ensure_embed_window(app_handle, &msg)?;

            if created || needs_geometry_apply(&msg) {
                apply_window_geometry(&window, &msg)?;
                record_applied_geometry(&msg);
            }

            let desired_visible = should_show_window_now(&window, &msg);
            if created || needs_visibility_apply(desired_visible) {
                apply_embed_window_visibility(&window, desired_visible)?;
                record_applied_visibility(desired_visible);
            }
            #[cfg(target_os = "windows")]
            windows_impl::set_popup_sync_visible(msg.visible);
            Ok(())
        }
        "close" => {
            if is_transient_close_reason(&msg.reason) {
                schedule_transient_close_destroy(app_handle);
                return Ok(());
            }

            cancel_transient_close_destroy();
            if let Some(window) = app_handle.get_webview_window(WINDOW_LABEL) {
                window
                    .destroy()
                    .or_else(|_| window.close())
                    .map_err(|error| format!("Failed to close Unity embed window: {error}"))?;
            }
            record_window_destroyed();
            Ok(())
        }
        "assetDrop" => {
            let refs = msg.asset_refs.unwrap_or_default();
            if refs.is_empty() {
                return Ok(());
            }
            emit_unity_embed_asset_drop(app_handle, refs)?;
            cache_unity_embed_asset_drag_refs(Vec::new());
            emit_unity_embed_asset_drag_state(app_handle, Vec::new())
        }
        "assetDrag" => {
            let refs = msg.asset_refs.unwrap_or_default();
            cache_unity_embed_asset_drag_refs(refs.clone());
            emit_unity_embed_asset_drag_state(app_handle, refs)
        }
        other => Err(format!("Unknown Unity embed control message: {other}")),
    }
}

async fn apply_control_message(
    app_handle: AppHandle,
    msg: UnityEmbedControlMessage,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app_for_main = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            let result = apply_control_message_on_main(&app_for_main, msg);
            let _ = tx.send(result);
        })
        .map_err(|error| format!("Failed to dispatch Unity embed control: {error}"))?;

    rx.await
        .map_err(|_| "Unity embed control dispatch was cancelled".to_string())?
}

fn apply_window_geometry(
    window: &tauri::WebviewWindow,
    msg: &UnityEmbedControlMessage,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if msg.parent_hwnd > 0 {
        if let Err(error) = windows_impl::position_owned_overlay(window, msg) {
            eprintln!("[Locus] Unity embed Win32 overlay failed, using Tauri fallback: {error}");
            record_mount_result(false, Some(error.clone()));
            windows_impl::disable_popup_sync();
            return apply_overlay_geometry(window, msg);
        }
        record_mount_result(true, None);
        return Ok(());
    }

    record_mount_result(false, Some("Unity parent HWND is missing".to_string()));
    #[cfg(target_os = "windows")]
    {
        windows_impl::disable_popup_sync();
        windows_impl::set_activation_guard_enabled(Some(window), true)?;
    }
    apply_overlay_geometry(window, msg)
}

fn apply_embed_window_visibility(
    window: &tauri::WebviewWindow,
    visible: bool,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return windows_impl::set_window_visible_no_activate(window, visible);
    }

    #[cfg(not(target_os = "windows"))]
    {
        if visible {
            window
                .show()
                .map_err(|error| format!("Failed to show Unity embed window: {error}"))
        } else {
            window
                .hide()
                .map_err(|error| format!("Failed to hide Unity embed window: {error}"))
        }
    }
}

fn apply_overlay_geometry(
    window: &tauri::WebviewWindow,
    msg: &UnityEmbedControlMessage,
) -> Result<(), String> {
    let (x, y, width, height) = normalized_rect(msg);
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| format!("Failed to resize Unity embed window: {error}"))?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| format!("Failed to move Unity embed window: {error}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use std::io;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    };
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::Threading::{AttachThreadInput, GetCurrentThreadId},
        UI::{
            Input::KeyboardAndMouse::{SetActiveWindow, SetFocus as SetKeyboardFocus},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                BringWindowToTop, GetForegroundWindow, GetParent, GetTopWindow, GetWindow,
                GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow, SetParent,
                SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT, GWL_EXSTYLE,
                GWL_STYLE, GW_CHILD, GW_HWNDNEXT, HWND_TOP, MA_NOACTIVATE, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SW_HIDE,
                SW_SHOWNOACTIVATE, WM_MOUSEACTIVATE, WM_NCDESTROY, WS_CAPTION, WS_CHILD,
                WS_EX_NOACTIVATE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
                WS_THICKFRAME,
            },
        },
    };

    const POPUP_SYNC_ACTIVE_INTERVAL_MS: u64 = 16;
    const POPUP_SYNC_IDLE_INTERVAL_MS: u64 = 120;
    const MOUSE_HOOK_SYNC_INTERVAL_MS: u64 = 250;
    const Z_ORDER_SCAN_LIMIT: usize = 2048;
    const MOUSE_ACTIVATE_SUBCLASS_ID: usize = 0x4c6f637573;

    #[derive(Debug, Clone, Copy, Default)]
    struct PopupSyncSnapshot {
        parent_hwnd: i64,
        child_hwnd: i64,
        offset_x: i32,
        offset_y: i32,
        width: i32,
        height: i32,
    }

    #[derive(Debug, Default)]
    struct PopupSyncState {
        active: bool,
        visible: bool,
        snapshot: PopupSyncSnapshot,
    }

    #[derive(Debug)]
    struct MouseActivationState {
        suppressed: bool,
        guard_enabled: bool,
    }

    #[derive(Debug, Default)]
    struct MouseActivateHookState {
        hwnds: Vec<i64>,
        installed: bool,
        block_count: u64,
    }

    impl Default for MouseActivationState {
        fn default() -> Self {
            Self {
                suppressed: true,
                guard_enabled: true,
            }
        }
    }

    #[derive(Default)]
    struct ControlServerState {
        pipe_name: String,
        handle: Option<tauri::async_runtime::JoinHandle<()>>,
    }

    fn popup_sync_state() -> &'static Mutex<PopupSyncState> {
        static STATE: OnceLock<Mutex<PopupSyncState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(PopupSyncState::default()))
    }

    fn control_server_state() -> &'static Mutex<ControlServerState> {
        static STATE: OnceLock<Mutex<ControlServerState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(ControlServerState::default()))
    }

    fn mouse_activation_state() -> &'static Mutex<MouseActivationState> {
        static STATE: OnceLock<Mutex<MouseActivationState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(MouseActivationState::default()))
    }

    fn mouse_activate_hook_state() -> &'static Mutex<MouseActivateHookState> {
        static STATE: OnceLock<Mutex<MouseActivateHookState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(MouseActivateHookState::default()))
    }

    pub(super) fn start(app_handle: AppHandle) {
        static POPUP_SYNC_STARTED: OnceLock<()> = OnceLock::new();
        if POPUP_SYNC_STARTED.set(()).is_ok() {
            tauri::async_runtime::spawn(async move {
                popup_sync_loop().await;
            });
        }

        static MOUSE_HOOK_SYNC_STARTED: OnceLock<()> = OnceLock::new();
        if MOUSE_HOOK_SYNC_STARTED.set(()).is_ok() {
            let app_for_hook_sync = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                mouse_hook_sync_loop(app_for_hook_sync).await;
            });
        }

        refresh(app_handle);
    }

    pub(super) fn refresh(app_handle: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let next_pipe_name = current_control_pipe_name(&app_handle)
                .await
                .unwrap_or_default();

            let mut state = match control_server_state().lock() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("[Locus] Unity embed control state lock failed: {error}");
                    return;
                }
            };

            let running_same_pipe = state.pipe_name == next_pipe_name && state.handle.is_some();
            if running_same_pipe {
                return;
            }

            if let Some(handle) = state.handle.take() {
                handle.abort();
            }

            state.pipe_name = next_pipe_name.clone();
            if next_pipe_name.is_empty() {
                return;
            }

            let app_for_server = app_handle.clone();
            let pipe_for_server = next_pipe_name.clone();
            state.handle = Some(tauri::async_runtime::spawn(async move {
                if let Err(error) =
                    server_loop(app_for_server.clone(), pipe_for_server.clone()).await
                {
                    eprintln!(
                        "[Locus] Unity embed control pipe stopped ({}): {error}",
                        pipe_for_server
                    );
                }
                if let Ok(mut state) = control_server_state().lock() {
                    if state.pipe_name == pipe_for_server {
                        state.handle = None;
                    }
                }
            }));
        });
    }

    pub(super) fn set_mouse_activation_suppressed(
        window: Option<&tauri::WebviewWindow>,
        suppressed: bool,
    ) -> Result<(), String> {
        let guard_enabled = mouse_activation_state()
            .lock()
            .map(|mut state| {
                state.suppressed = suppressed;
                state.guard_enabled
            })
            .unwrap_or(true);

        if let Some(window) = window {
            apply_mouse_activation_style(window, guard_enabled && suppressed)?;
        }

        Ok(())
    }

    pub(super) fn set_activation_guard_enabled(
        window: Option<&tauri::WebviewWindow>,
        enabled: bool,
    ) -> Result<(), String> {
        if let Ok(mut state) = mouse_activation_state().lock() {
            state.guard_enabled = enabled;
        }

        if let Some(window) = window {
            if enabled {
                sync_mouse_activation_style(window)?;
                ensure_mouse_activate_hook(window)?;
            } else {
                apply_mouse_activation_style(window, false)?;
                remove_mouse_activate_hook();
            }
        }

        Ok(())
    }

    pub(super) fn reset_mouse_activation_suppressed() {
        if let Ok(mut state) = mouse_activation_state().lock() {
            state.suppressed = true;
            state.guard_enabled = true;
        }
    }

    pub(super) fn activate_for_input(window: Option<&tauri::WebviewWindow>) -> Result<(), String> {
        let Some(window) = window else {
            return Ok(());
        };

        let hwnd = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        if is_activation_guard_enabled() {
            set_mouse_activation_suppressed(Some(window), false)?;
            ensure_mouse_activate_hook(window)?;
        }

        unsafe {
            focus_embed_window_for_input(window, hwnd);
        }
        Ok(())
    }

    fn is_activation_guard_enabled() -> bool {
        mouse_activation_state()
            .lock()
            .map(|state| state.guard_enabled)
            .unwrap_or(true)
    }

    pub(super) fn set_window_visible_no_activate(
        window: &tauri::WebviewWindow,
        visible: bool,
    ) -> Result<(), String> {
        let hwnd = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        unsafe {
            let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        Ok(())
    }

    pub(super) fn ensure_mouse_activate_hook(window: &tauri::WebviewWindow) -> Result<(), String> {
        let hwnd = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        let mut hwnds = vec![hwnd];
        unsafe {
            collect_descendant_windows(hwnd, &mut hwnds);
        }

        let mut installed_hwnds = Vec::with_capacity(hwnds.len());
        for hwnd in hwnds {
            unsafe {
                let ok = SetWindowSubclass(
                    hwnd,
                    Some(unity_embed_mouse_activate_proc),
                    MOUSE_ACTIVATE_SUBCLASS_ID,
                    0,
                )
                .as_bool();
                if ok {
                    installed_hwnds.push(hwnd.0 as isize as i64);
                }
            }
        }

        if installed_hwnds.is_empty() {
            return Err("SetWindowSubclass failed for Unity embed mouse activation".to_string());
        }

        if let Ok(mut state) = mouse_activate_hook_state().lock() {
            for stale_hwnd in state.hwnds.iter().copied() {
                if installed_hwnds.contains(&stale_hwnd) {
                    continue;
                }
                let hwnd = HWND(stale_hwnd as isize as *mut std::ffi::c_void);
                unsafe {
                    if IsWindow(Some(hwnd)).as_bool() {
                        let _ = RemoveWindowSubclass(
                            hwnd,
                            Some(unity_embed_mouse_activate_proc),
                            MOUSE_ACTIVATE_SUBCLASS_ID,
                        );
                    }
                }
            }
            state.hwnds = installed_hwnds;
            state.installed = true;
        }

        Ok(())
    }

    unsafe fn collect_descendant_windows(parent: HWND, hwnds: &mut Vec<HWND>) {
        let mut child = match GetWindow(parent, GW_CHILD) {
            Ok(child) => child,
            Err(_) => return,
        };

        loop {
            hwnds.push(child);
            collect_descendant_windows(child, hwnds);
            match GetWindow(child, GW_HWNDNEXT) {
                Ok(next) => child = next,
                Err(_) => break,
            }
        }
    }

    pub(super) fn remove_mouse_activate_hook() {
        let hwnds = mouse_activate_hook_state()
            .lock()
            .ok()
            .map(|state| state.hwnds.clone())
            .unwrap_or_default();
        for hwnd in hwnds {
            let hwnd = HWND(hwnd as isize as *mut std::ffi::c_void);
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = RemoveWindowSubclass(
                        hwnd,
                        Some(unity_embed_mouse_activate_proc),
                        MOUSE_ACTIVATE_SUBCLASS_ID,
                    );
                }
            }
        }

        if let Ok(mut state) = mouse_activate_hook_state().lock() {
            state.hwnds.clear();
            state.installed = false;
        }
    }

    unsafe extern "system" fn unity_embed_mouse_activate_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid_subclass: usize,
        _ref_data: usize,
    ) -> LRESULT {
        if msg == WM_MOUSEACTIVATE {
            let should_suppress = mouse_activation_state()
                .lock()
                .map(|state| state.guard_enabled && state.suppressed)
                .unwrap_or(true);
            if should_suppress {
                if let Ok(mut state) = mouse_activate_hook_state().lock() {
                    state.block_count = state.block_count.saturating_add(1);
                }
                return LRESULT(MA_NOACTIVATE as isize);
            }
        }

        if msg == WM_NCDESTROY {
            if let Ok(mut state) = mouse_activate_hook_state().lock() {
                let hwnd_value = hwnd.0 as isize as i64;
                state.hwnds.retain(|hooked_hwnd| *hooked_hwnd != hwnd_value);
                state.installed = !state.hwnds.is_empty();
            }
        }

        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    unsafe fn focus_embed_window_for_input(window: &tauri::WebviewWindow, hwnd: HWND) {
        if !is_valid_window(hwnd) {
            return;
        }

        let foreground_parent = focus_parent_for_embed_window(hwnd);
        let foreground_target = if is_valid_window(foreground_parent) {
            foreground_parent
        } else {
            hwnd
        };
        let mut focus_targets = vec![foreground_target, hwnd, GetForegroundWindow()];
        unsafe {
            collect_descendant_windows(hwnd, &mut focus_targets);
        }

        let current_thread = GetCurrentThreadId();
        let attached_threads = attach_input_threads(current_thread, &focus_targets);

        if is_valid_window(foreground_target) {
            let _ = BringWindowToTop(foreground_target);
            let _ = SetForegroundWindow(foreground_target);
            let _ = SetActiveWindow(foreground_target);
        }

        focus_window_chain(hwnd);
        let _ = window.set_focus();
        focus_webview_controller(window);

        detach_input_threads(current_thread, attached_threads);
    }

    unsafe fn focus_parent_for_embed_window(hwnd: HWND) -> HWND {
        if has_child_style(hwnd) {
            if let Ok(parent) = GetParent(hwnd) {
                if is_valid_window(parent) {
                    return parent;
                }
            }
        }

        let parent_hwnd = control_state()
            .lock()
            .map(|state| state.last_parent_hwnd)
            .unwrap_or_default();
        if parent_hwnd > 0 {
            let parent = HWND(parent_hwnd as isize as *mut std::ffi::c_void);
            if is_valid_window(parent) {
                return parent;
            }
        }

        HWND(std::ptr::null_mut())
    }

    unsafe fn is_valid_window(hwnd: HWND) -> bool {
        !hwnd.0.is_null() && IsWindow(Some(hwnd)).as_bool()
    }

    unsafe fn attach_input_threads(current_thread: u32, hwnds: &[HWND]) -> Vec<u32> {
        let mut attached_threads = Vec::new();
        for hwnd in hwnds {
            if !is_valid_window(*hwnd) {
                continue;
            }
            let thread_id = GetWindowThreadProcessId(*hwnd, None);
            if thread_id == 0
                || thread_id == current_thread
                || attached_threads.contains(&thread_id)
            {
                continue;
            }

            if AttachThreadInput(current_thread, thread_id, true).as_bool() {
                attached_threads.push(thread_id);
            }
        }
        attached_threads
    }

    unsafe fn detach_input_threads(current_thread: u32, mut attached_threads: Vec<u32>) {
        attached_threads.reverse();
        for thread_id in attached_threads {
            let _ = AttachThreadInput(current_thread, thread_id, false);
        }
    }

    unsafe fn focus_window_chain(hwnd: HWND) {
        let mut focus_targets = vec![hwnd];
        collect_descendant_windows(hwnd, &mut focus_targets);

        for target in focus_targets {
            if is_valid_window(target) && IsWindowVisible(target).as_bool() {
                let _ = SetKeyboardFocus(Some(target));
            }
        }
    }

    fn focus_webview_controller(window: &tauri::WebviewWindow) {
        let _ = window.with_webview(|webview| {
            let controller = webview.controller();
            unsafe {
                let mut webview_hwnd = HWND::default();
                let _ = controller.ParentWindow(&mut webview_hwnd);
                if !webview_hwnd.0.is_null() {
                    let _ = SetKeyboardFocus(Some(webview_hwnd));
                }
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        });
    }

    pub(super) fn focus_debug_snapshot(
        window: Option<&tauri::WebviewWindow>,
    ) -> UnityEmbedFocusDebugSnapshot {
        let foreground = unsafe { GetForegroundWindow() };
        let overlay = window.and_then(|window| window.hwnd().ok());
        let parent_hwnd = control_state()
            .lock()
            .map(|state| state.last_parent_hwnd)
            .unwrap_or_default();
        let parent = if parent_hwnd > 0 {
            HWND(parent_hwnd as isize as *mut std::ffi::c_void)
        } else {
            HWND(std::ptr::null_mut())
        };

        let overlay_hwnd = overlay
            .map(|hwnd| hwnd.0 as isize as i64)
            .unwrap_or_default();
        let overlay_visible = overlay
            .map(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
            .unwrap_or(false);
        let overlay_no_activate = overlay
            .map(|hwnd| unsafe { has_no_activate_style(hwnd) })
            .unwrap_or(false);
        let overlay_child_window = overlay
            .map(|hwnd| unsafe { has_child_style(hwnd) })
            .unwrap_or(false);
        let overlay_parent_hwnd = overlay
            .and_then(|hwnd| unsafe { GetParent(hwnd).ok() })
            .map(|hwnd| hwnd.0 as isize as i64)
            .unwrap_or_default();
        let parent_visible = if parent_hwnd > 0 {
            unsafe { IsWindowVisible(parent).as_bool() }
        } else {
            false
        };
        let (mouse_activation_suppressed, activation_guard_enabled) = mouse_activation_state()
            .lock()
            .map(|state| (state.suppressed, state.guard_enabled))
            .unwrap_or((true, true));
        let (
            mouse_activate_hook_installed,
            mouse_activate_hooked_hwnd_count,
            mouse_activate_block_count,
        ) = mouse_activate_hook_state()
            .lock()
            .map(|state| (state.installed, state.hwnds.len(), state.block_count))
            .unwrap_or_default();
        let foreground_hwnd = foreground.0 as isize as i64;

        UnityEmbedFocusDebugSnapshot {
            ok: overlay.is_some(),
            reason: if overlay.is_some() {
                String::new()
            } else {
                "Unity embed overlay window is not available".to_string()
            },
            foreground_hwnd,
            foreground_title: unsafe { hwnd_title(foreground) },
            overlay_hwnd,
            overlay_title: overlay
                .map(|hwnd| unsafe { hwnd_title(hwnd) })
                .unwrap_or_default(),
            overlay_visible,
            overlay_foreground: overlay_hwnd != 0 && overlay_hwnd == foreground_hwnd,
            overlay_child_window,
            overlay_parent_hwnd,
            overlay_no_activate,
            activation_guard_enabled,
            mouse_activate_hook_installed,
            mouse_activate_hooked_hwnd_count,
            mouse_activate_block_count,
            mouse_activation_suppressed,
            parent_hwnd,
            parent_title: if parent_hwnd > 0 {
                unsafe { hwnd_title(parent) }
            } else {
                String::new()
            },
            parent_visible,
            parent_foreground: parent_hwnd > 0 && parent_hwnd == foreground_hwnd,
        }
    }

    pub(super) fn sync_mouse_activation_style(window: &tauri::WebviewWindow) -> Result<(), String> {
        let suppressed = mouse_activation_state()
            .lock()
            .map(|state| state.guard_enabled && state.suppressed)
            .unwrap_or(true);
        apply_mouse_activation_style(window, suppressed)
    }

    fn apply_mouse_activation_style(
        window: &tauri::WebviewWindow,
        suppressed: bool,
    ) -> Result<(), String> {
        let hwnd = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        unsafe { apply_mouse_activation_style_to_hwnd(hwnd, suppressed) }
    }

    unsafe fn apply_mouse_activation_style_to_hwnd(
        hwnd: HWND,
        suppressed: bool,
    ) -> Result<(), String> {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let current = ex_style as u32;
        let next = if suppressed {
            current | WS_EX_NOACTIVATE.0
        } else {
            current & !WS_EX_NOACTIVATE.0
        };

        if next == current {
            return Ok(());
        }

        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
        )
        .map_err(|error| format!("SetWindowPos failed for Unity embed activation style: {error}"))
    }

    unsafe fn has_no_activate_style(hwnd: HWND) -> bool {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        (ex_style & WS_EX_NOACTIVATE.0) != 0
    }

    unsafe fn has_child_style(hwnd: HWND) -> bool {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        (style & WS_CHILD.0) != 0
    }

    unsafe fn hwnd_title(hwnd: HWND) -> String {
        if hwnd.0.is_null() {
            return String::new();
        }

        let mut title = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title);
        if len <= 0 {
            return String::new();
        }

        String::from_utf16_lossy(&title[..len as usize])
    }

    fn create_server(pipe_name: &str) -> io::Result<NamedPipeServer> {
        ServerOptions::new().max_instances(16).create(pipe_name)
    }

    async fn server_loop(app_handle: AppHandle, pipe_name: String) -> io::Result<()> {
        let mut server = create_server(&pipe_name)?;
        eprintln!("[Locus] Unity embed control pipe listening: {pipe_name}");

        loop {
            server.connect().await?;
            let next_server = create_server(&pipe_name)?;
            let connected = std::mem::replace(&mut server, next_server);
            let app_for_client = app_handle.clone();
            let pipe_for_client = pipe_name.clone();

            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle_client(app_for_client, connected, pipe_for_client).await
                {
                    eprintln!("[Locus] Unity embed control client error: {error}");
                }
            });
        }
    }

    async fn handle_client(
        app_handle: AppHandle,
        server: NamedPipeServer,
        pipe_name: String,
    ) -> io::Result<()> {
        let mut reader = BufReader::new(server);
        let mut line = String::new();

        loop {
            line.clear();
            let read = reader.read_line(&mut line).await?;
            if read == 0 {
                break;
            }

            let trimmed = line.trim().trim_start_matches('\u{FEFF}');
            if trimmed.is_empty() {
                continue;
            }

            let msg: UnityEmbedControlMessage = match serde_json::from_str(trimmed) {
                Ok(msg) => msg,
                Err(error) => {
                    eprintln!(
                        "[Locus] failed to parse Unity embed control message: {error} | raw={trimmed}"
                    );
                    continue;
                }
            };

            if !is_current_control_pipe(&app_handle, &pipe_name).await {
                continue;
            }

            if let Err(error) = apply_control_message(app_handle.clone(), msg).await {
                eprintln!("[Locus] failed to apply Unity embed control message: {error}");
            }
        }

        Ok(())
    }

    async fn popup_sync_loop() {
        loop {
            let active = sync_popup_overlay_position();
            let delay = if active {
                POPUP_SYNC_ACTIVE_INTERVAL_MS
            } else {
                POPUP_SYNC_IDLE_INTERVAL_MS
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }

    async fn mouse_hook_sync_loop(app_handle: AppHandle) {
        loop {
            let app_for_main = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                if let Some(window) = app_for_main.get_webview_window(WINDOW_LABEL) {
                    if is_activation_guard_enabled() {
                        if let Err(error) = sync_mouse_activation_style(&window) {
                            eprintln!(
                                "[Locus] failed to sync Unity embed activation style: {error}"
                            );
                        }
                        if let Err(error) = ensure_mouse_activate_hook(&window) {
                            eprintln!("[Locus] failed to sync Unity embed mouse hook: {error}");
                        }
                    } else {
                        if let Err(error) = apply_mouse_activation_style(&window, false) {
                            eprintln!(
                                "[Locus] failed to clear Unity embed activation style: {error}"
                            );
                        }
                        remove_mouse_activate_hook();
                    }
                }
            });
            tokio::time::sleep(Duration::from_millis(MOUSE_HOOK_SYNC_INTERVAL_MS)).await;
        }
    }

    fn popup_sync_snapshot() -> Option<PopupSyncSnapshot> {
        let state = popup_sync_state().lock().ok()?;
        if !state.active || !state.visible {
            return None;
        }

        let snapshot = state.snapshot;
        if snapshot.parent_hwnd <= 0
            || snapshot.child_hwnd <= 0
            || snapshot.width <= 0
            || snapshot.height <= 0
        {
            return None;
        }

        Some(snapshot)
    }

    fn sync_popup_overlay_position() -> bool {
        let Some(snapshot) = popup_sync_snapshot() else {
            return false;
        };

        let parent = HWND(snapshot.parent_hwnd as isize as *mut std::ffi::c_void);
        let child = HWND(snapshot.child_hwnd as isize as *mut std::ffi::c_void);

        unsafe {
            if !IsWindow(Some(parent)).as_bool() || !IsWindow(Some(child)).as_bool() {
                disable_popup_sync();
                return false;
            }

            let mut parent_rect = RECT::default();
            if GetWindowRect(parent, &mut parent_rect).is_err() {
                return true;
            }

            let x = parent_rect.left + snapshot.offset_x;
            let y = parent_rect.top + snapshot.offset_y;
            let width = snapshot.width;
            let height = snapshot.height;

            let target_rect = RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            };
            let should_show = is_overlay_parent_visible_at(parent);
            sync_overlay_visibility(child, should_show);
            if !should_show {
                return false;
            }

            let mut child_rect = RECT::default();
            let child_matches = GetWindowRect(child, &mut child_rect)
                .map(|_| {
                    child_rect.left == x
                        && child_rect.top == y
                        && child_rect.right - child_rect.left == width
                        && child_rect.bottom - child_rect.top == height
                })
                .unwrap_or(false);
            let insert_after = overlay_insert_after(parent, child, target_rect);

            let _ = SetWindowPos(
                child,
                Some(insert_after),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );

            if !child_matches {
                return true;
            }
        }

        true
    }

    pub(super) fn disable_popup_sync() {
        if let Ok(mut state) = popup_sync_state().lock() {
            *state = PopupSyncState::default();
        }
    }

    pub(super) fn set_popup_sync_visible(visible: bool) {
        if let Ok(mut state) = popup_sync_state().lock() {
            state.visible = visible;
        }
    }

    pub(super) fn is_overlay_parent_visible(
        _window: &tauri::WebviewWindow,
        msg: &UnityEmbedControlMessage,
    ) -> bool {
        let parent = HWND(msg.parent_hwnd as isize as *mut std::ffi::c_void);
        unsafe { is_overlay_parent_visible_at(parent) }
    }

    fn update_popup_sync(
        parent_hwnd: i64,
        child_hwnd: i64,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        visible: bool,
    ) -> Result<(), String> {
        let parent = HWND(parent_hwnd as isize as *mut std::ffi::c_void);
        unsafe {
            let mut parent_rect = RECT::default();
            GetWindowRect(parent, &mut parent_rect)
                .map_err(|error| format!("GetWindowRect failed for Unity parent HWND: {error}"))?;

            if let Ok(mut state) = popup_sync_state().lock() {
                state.active = true;
                state.visible = visible;
                state.snapshot = PopupSyncSnapshot {
                    parent_hwnd,
                    child_hwnd,
                    offset_x: x - parent_rect.left,
                    offset_y: y - parent_rect.top,
                    width,
                    height,
                };
            }
        }

        Ok(())
    }

    unsafe fn is_overlay_parent_visible_at(parent: HWND) -> bool {
        IsWindow(Some(parent)).as_bool()
            && IsWindowVisible(parent).as_bool()
            && !IsIconic(parent).as_bool()
    }

    unsafe fn sync_overlay_visibility(child: HWND, visible: bool) {
        let is_visible = IsWindowVisible(child).as_bool();
        if visible == is_visible {
            return;
        }

        let _ = ShowWindow(child, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }

    fn overlay_insert_after(parent: HWND, child: HWND, target_rect: RECT) -> HWND {
        // Follow the current desktop z-order without moving Unity popup windows.
        // When several Unity floating windows overlap Locus, changing their
        // order here creates a 16ms reorder loop. Anchoring Locus below the
        // lowest current blocker keeps Unity's own order stable.
        find_intersecting_window_above_parent(parent, child, target_rect).unwrap_or(HWND_TOP)
    }

    fn find_intersecting_window_above_parent(
        parent: HWND,
        child: HWND,
        target_rect: RECT,
    ) -> Option<HWND> {
        let mut blocker = None;
        let mut hwnd = unsafe { GetTopWindow(None).ok()? };
        for _ in 0..Z_ORDER_SCAN_LIMIT {
            if hwnd == parent {
                return blocker;
            }

            if hwnd != child && unsafe { is_visible_intersecting_window(hwnd, target_rect) } {
                blocker = Some(hwnd);
            }

            match unsafe { GetWindow(hwnd, GW_HWNDNEXT) } {
                Ok(next) => hwnd = next,
                Err(_) => break,
            }
        }

        None
    }

    pub(super) fn position_owned_overlay(
        window: &tauri::WebviewWindow,
        msg: &UnityEmbedControlMessage,
    ) -> Result<(), String> {
        match position_child_overlay(window, msg) {
            Ok(()) => {
                disable_popup_sync();
                Ok(())
            }
            Err(child_error) => {
                eprintln!(
                    "[Locus] Unity embed child mount failed, using popup fallback: {child_error}"
                );
                set_activation_guard_enabled(Some(window), true)?;
                position_popup_overlay(window, msg).map_err(|popup_error| {
                    format!("{child_error}; popup fallback failed: {popup_error}")
                })
            }
        }
    }

    fn position_child_overlay(
        window: &tauri::WebviewWindow,
        msg: &UnityEmbedControlMessage,
    ) -> Result<(), String> {
        let child = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        let child_hwnd = child.0 as isize as i64;
        record_child_hwnd(child_hwnd);
        let parent = HWND(msg.parent_hwnd as isize as *mut std::ffi::c_void);
        let (x, y, width, height) = normalized_rect(msg);
        let width_i32 = width as i32;
        let height_i32 = height as i32;

        unsafe {
            if !is_overlay_parent_visible_at(parent) {
                return Err("Unity parent HWND is not visible".to_string());
            }

            let style = GetWindowLongPtrW(child, GWL_STYLE);
            let current_style = style as u32;
            let frame_style_mask = WS_POPUP.0
                | WS_CAPTION.0
                | WS_THICKFRAME.0
                | WS_MINIMIZEBOX.0
                | WS_MAXIMIZEBOX.0
                | WS_SYSMENU.0;
            let next_style = (current_style & !frame_style_mask) | WS_CHILD.0;
            let needs_style_update = next_style != current_style;
            let current_parent = GetParent(child).unwrap_or(HWND(std::ptr::null_mut()));
            let needs_parent_update = current_parent != parent;

            if needs_style_update {
                SetWindowLongPtrW(child, GWL_STYLE, next_style as isize);
            }

            if needs_parent_update {
                SetParent(child, Some(parent)).map_err(|error| {
                    format!("SetParent failed for Unity embed child window: {error}")
                })?;
            }

            let mut top_left = POINT { x, y };
            if !ScreenToClient(parent, &mut top_left).as_bool() {
                return Err("ScreenToClient failed for Unity embed child window".to_string());
            }

            let flags = if needs_style_update || needs_parent_update {
                SWP_NOACTIVATE | SWP_FRAMECHANGED
            } else {
                SWP_NOACTIVATE
            };

            SetWindowPos(
                child,
                Some(HWND_TOP),
                top_left.x,
                top_left.y,
                width_i32,
                height_i32,
                flags,
            )
            .map_err(|error| {
                format!("SetWindowPos failed for Unity embed child window: {error}")
            })?;
        }

        set_activation_guard_enabled(Some(window), false)?;
        Ok(())
    }

    fn position_popup_overlay(
        window: &tauri::WebviewWindow,
        msg: &UnityEmbedControlMessage,
    ) -> Result<(), String> {
        let child = window
            .hwnd()
            .map_err(|error| format!("Failed to read Tauri window handle: {error}"))?;
        let child_hwnd = child.0 as isize as i64;
        record_child_hwnd(child_hwnd);
        let parent_hwnd = msg.parent_hwnd;
        let parent = HWND(parent_hwnd as isize as *mut std::ffi::c_void);
        let (x, y, width, height) = normalized_rect(msg);
        let width_i32 = width as i32;
        let height_i32 = height as i32;

        unsafe {
            let style = GetWindowLongPtrW(child, GWL_STYLE);
            let current_style = style as u32;
            let frame_style_mask = WS_CHILD.0
                | WS_CAPTION.0
                | WS_THICKFRAME.0
                | WS_MINIMIZEBOX.0
                | WS_MAXIMIZEBOX.0
                | WS_SYSMENU.0;
            let needs_detach = (current_style & WS_CHILD.0) != 0;
            let needs_style_update =
                (current_style & frame_style_mask) != 0 || (current_style & WS_POPUP.0) == 0;
            let needs_owner_update = applied_state()
                .lock()
                .map(|state| !state.has_window || state.parent_hwnd != msg.parent_hwnd)
                .unwrap_or(true);

            if needs_detach {
                SetParent(child, None).map_err(|error| {
                    format!("SetParent detach failed for Unity embed window: {error}")
                })?;
            }

            if needs_style_update || needs_owner_update {
                let next_style = (current_style & !frame_style_mask) | WS_POPUP.0;
                SetWindowLongPtrW(child, GWL_STYLE, next_style as isize);
                SetWindowLongPtrW(child, GWLP_HWNDPARENT, parent.0 as isize);
            }

            let flags = if needs_style_update || needs_owner_update {
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_FRAMECHANGED
            } else {
                SWP_NOACTIVATE | SWP_NOOWNERZORDER
            };
            let target_rect = RECT {
                left: x,
                top: y,
                right: x + width_i32,
                bottom: y + height_i32,
            };
            if !is_overlay_parent_visible_at(parent) {
                sync_overlay_visibility(child, false);
            }

            let insert_after = overlay_insert_after(parent, child, target_rect);
            SetWindowPos(
                child,
                Some(insert_after),
                x,
                y,
                width_i32,
                height_i32,
                flags,
            )
            .map_err(|error| format!("SetWindowPos failed for Unity embed window: {error}"))?;
        }

        update_popup_sync(
            parent_hwnd,
            child_hwnd,
            x,
            y,
            width_i32,
            height_i32,
            msg.visible,
        )?;

        Ok(())
    }

    unsafe fn is_visible_intersecting_window(hwnd: HWND, target_rect: RECT) -> bool {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }

        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).is_ok()
            && rect.right > rect.left
            && rect.bottom > rect.top
            && rects_intersect(target_rect, rect)
    }

    fn rects_intersect(a: RECT, b: RECT) -> bool {
        a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        control_pipe_name_for_project_path, normalize_pipe_project_path,
        unity_file_drop_asset_refs, unity_relative_drop_path,
    };

    #[test]
    fn pipe_project_path_normalizes_windows_slashes_and_extended_prefix() {
        assert_eq!(
            normalize_pipe_project_path(r#"\\?\F:\Game\Project\"#),
            "F__Game_Project"
        );
    }

    #[test]
    fn control_pipe_name_includes_project_path_suffix() {
        assert_eq!(
            control_pipe_name_for_project_path(r"F:\Game\Project"),
            r"\\.\pipe\locus_tauri_unity_embed_F__Game_Project"
        );
    }

    #[test]
    fn unity_drop_path_maps_project_file_to_asset_ref() {
        let refs = unity_file_drop_asset_refs(
            "F:/Game/Project",
            &[PathBuf::from("f:/Game/Project/Assets/Prefabs/Enemy.prefab")],
        );

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "Assets/Prefabs/Enemy.prefab");
        assert_eq!(refs[0].kind, "asset");
        assert_eq!(refs[0].name.as_deref(), Some("Enemy"));
        assert_eq!(refs[0].type_label.as_deref(), Some("prefab"));
        assert_eq!(refs[0].source.as_deref(), Some("unity"));
    }

    #[test]
    fn unity_drop_path_rejects_files_outside_unity_ref_roots() {
        assert_eq!(
            unity_relative_drop_path("F:/Game/Project", Path::new("F:/Game/Project/README.md")),
            None
        );
    }
}
