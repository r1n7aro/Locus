use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

use crate::error::AppError;

const WINDOW_LABEL: &str = "unity-embed";
const CONTROL_PIPE_NAME: &str = r"\\.\pipe\locus_tauri_unity_embed";
const EMBED_URL: &str = "/unity-embed?host=tauri-overlay";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnityEmbedControlMessage {
    #[serde(rename = "type")]
    kind: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    parent_hwnd: i64,
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

fn needs_visibility_apply(msg: &UnityEmbedControlMessage) -> bool {
    if let Ok(state) = applied_state().lock() {
        return !state.has_window || state.visible != msg.visible;
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
    windows_impl::disable_popup_sync();
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

#[tauri::command]
pub async fn unity_embed_status() -> Result<UnityEmbedStatus, AppError> {
    Ok(UnityEmbedStatus {
        ok: true,
        runtime: "tauri".to_string(),
        message: "pong".to_string(),
        pipe_name: CONTROL_PIPE_NAME.to_string(),
        window_label: WINDOW_LABEL.to_string(),
        control: control_snapshot(),
    })
}

pub(crate) fn start_unity_embed_control_server(app_handle: AppHandle) {
    #[cfg(target_os = "windows")]
    windows_impl::start(app_handle);

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
    }
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

    #[cfg(target_os = "windows")]
    let builder = if msg.parent_hwnd > 0 {
        use windows::Win32::Foundation::HWND;
        builder.owner_raw(HWND(msg.parent_hwnd as isize as *mut std::ffi::c_void))
    } else {
        builder
    };

    builder
        .build()
        .inspect(|window| {
            #[cfg(target_os = "windows")]
            if let Ok(hwnd) = window.hwnd() {
                record_child_hwnd(hwnd.0 as isize as i64);
            }
        })
        .map(|window| (window, true))
        .map_err(|error| format!("Failed to create Unity embed window: {error}"))
}

fn apply_control_message_on_main(
    app_handle: &AppHandle,
    msg: UnityEmbedControlMessage,
) -> Result<(), String> {
    record_control_message(&msg);
    match msg.kind.as_str() {
        "open" | "update" => {
            let (window, created) = ensure_embed_window(app_handle, &msg)?;

            if created || needs_geometry_apply(&msg) {
                apply_window_geometry(&window, &msg)?;
                record_applied_geometry(&msg);
            }

            if created || needs_visibility_apply(&msg) {
                if should_show_window_now(&window, &msg) {
                    window
                        .show()
                        .map_err(|error| format!("Failed to show Unity embed window: {error}"))?;
                } else {
                    window
                        .hide()
                        .map_err(|error| format!("Failed to hide Unity embed window: {error}"))?;
                }
                record_applied_visibility(msg.visible);
                #[cfg(target_os = "windows")]
                windows_impl::set_popup_sync_visible(msg.visible);
            }
            Ok(())
        }
        "close" => {
            if let Some(window) = app_handle.get_webview_window(WINDOW_LABEL) {
                window
                    .destroy()
                    .or_else(|_| window.close())
                    .map_err(|error| format!("Failed to close Unity embed window: {error}"))?;
            }
            record_window_destroyed();
            Ok(())
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
    windows_impl::disable_popup_sync();
    apply_overlay_geometry(window, msg)
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
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{
            GetTopWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindow,
            IsWindowVisible, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            GWLP_HWNDPARENT, GWL_STYLE, GW_HWNDNEXT, HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOOWNERZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WS_CAPTION, WS_CHILD, WS_MAXIMIZEBOX,
            WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
        },
    };

    const POPUP_SYNC_ACTIVE_INTERVAL_MS: u64 = 16;
    const POPUP_SYNC_IDLE_INTERVAL_MS: u64 = 120;

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

    fn popup_sync_state() -> &'static Mutex<PopupSyncState> {
        static STATE: OnceLock<Mutex<PopupSyncState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(PopupSyncState::default()))
    }

    pub(super) fn start(app_handle: AppHandle) {
        tauri::async_runtime::spawn(async move {
            popup_sync_loop().await;
        });

        tauri::async_runtime::spawn(async move {
            if let Err(error) = server_loop(app_handle).await {
                eprintln!("[Locus] Unity embed control pipe stopped: {error}");
            }
        });
    }

    fn create_server() -> io::Result<NamedPipeServer> {
        ServerOptions::new()
            .max_instances(16)
            .create(CONTROL_PIPE_NAME)
    }

    async fn server_loop(app_handle: AppHandle) -> io::Result<()> {
        let mut server = create_server()?;
        eprintln!("[Locus] Unity embed control pipe listening: {CONTROL_PIPE_NAME}");

        loop {
            server.connect().await?;
            let next_server = create_server()?;
            let connected = std::mem::replace(&mut server, next_server);
            let app_for_client = app_handle.clone();

            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle_client(app_for_client, connected).await {
                    eprintln!("[Locus] Unity embed control client error: {error}");
                }
            });
        }
    }

    async fn handle_client(app_handle: AppHandle, server: NamedPipeServer) -> io::Result<()> {
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
            let insert_after = find_intersecting_window_above_parent(parent, child, target_rect)
                .unwrap_or(HWND_TOP);

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

    fn find_intersecting_window_above_parent(
        parent: HWND,
        child: HWND,
        target_rect: RECT,
    ) -> Option<HWND> {
        let mut blocker = None;
        let mut hwnd = unsafe { GetTopWindow(None).ok()? };
        for _ in 0..512 {
            if hwnd == parent {
                return blocker;
            }

            if hwnd != child && unsafe { is_visible_intersecting_window(hwnd, target_rect) } {
                blocker = Some(hwnd);
            }

            hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT).ok()? };
        }

        blocker
    }

    pub(super) fn position_owned_overlay(
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

            let insert_after = find_intersecting_window_above_parent(parent, child, target_rect)
                .unwrap_or(HWND_TOP);
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
