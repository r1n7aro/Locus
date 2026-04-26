use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::Emitter;
use tauri::Manager;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::core::BOOL;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::ClientToScreen,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            EnumChildWindows, GetClassNameW, GetClientRect, GetWindowRect, SetWindowPos,
            SET_WINDOW_POS_FLAGS, SIZE_MINIMIZED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, WINDOWPOS, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_MOVE, WM_MOVING,
            WM_NCDESTROY, WM_SIZE, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
        },
    },
};

const MAIN_WINDOW_LABEL: &str = "main";
const RESIZE_BORDERS_CLASS_NAME: &str = "TAURI_DRAG_RESIZE_BORDERS";
const NATIVE_CLIENT_SIZE_EVENT: &str = "locus-native-window-client-size";
const SUBCLASS_ID: usize = 0x4c6f63757352537a;
const CHILD_SUBCLASS_ID: usize = 0x4c6f63757352537b;
const MIN_SYNC_WIDTH_PX: i32 = 320;
const MIN_SYNC_HEIGHT_PX: i32 = 120;

static INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Serialize)]
struct NativeWindowClientSize {
    width: i32,
    height: i32,
}

struct ResizeSyncState {
    app_handle: tauri::AppHandle,
    controller: ICoreWebView2Controller,
    parent_hwnd: HWND,
    webview_hwnd: HWND,
    resize_borders_hwnd: HWND,
    last_x: i32,
    last_y: i32,
    last_width: i32,
    last_height: i32,
    resize_target_active: bool,
    resize_target_left: i32,
    resize_target_top: i32,
    last_native_width: i32,
    last_native_height: i32,
    live_resize: bool,
}

struct WindowClientMetrics {
    window_width: i32,
    window_height: i32,
    client_width: i32,
    client_height: i32,
    frame_width: i32,
    frame_height: i32,
}

pub fn install_for_main_window(app: &tauri::App) -> Result<(), String> {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }

    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        INSTALLED.store(false, Ordering::Release);
        return Err(format!(
            "main webview window '{MAIN_WINDOW_LABEL}' was not found"
        ));
    };

    let parent_hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to read main window handle: {error}"))?;
    let parent_hwnd_value = parent_hwnd.0 as isize;
    let app_handle = app.handle().clone();

    window
        .with_webview(move |webview| {
            let controller = webview.controller();
            let parent_hwnd = HWND(parent_hwnd_value as *mut std::ffi::c_void);
            unsafe {
                install_subclass(parent_hwnd, controller, app_handle);
            }
        })
        .map_err(|error| {
            INSTALLED.store(false, Ordering::Release);
            format!("failed to access main WebView2 controller: {error}")
        })
}

unsafe fn install_subclass(
    parent_hwnd: HWND,
    controller: ICoreWebView2Controller,
    app_handle: tauri::AppHandle,
) {
    let mut webview_hwnd = HWND::default();
    let _ = controller.ParentWindow(&mut webview_hwnd);
    let state = Box::new(ResizeSyncState {
        app_handle,
        controller,
        parent_hwnd,
        webview_hwnd,
        resize_borders_hwnd: unsafe { find_resize_borders_hwnd(parent_hwnd) },
        last_x: 0,
        last_y: 0,
        last_width: 0,
        last_height: 0,
        resize_target_active: false,
        resize_target_left: 0,
        resize_target_top: 0,
        last_native_width: 0,
        last_native_height: 0,
        live_resize: false,
    });
    let state_ptr = Box::into_raw(state);

    if !unsafe {
        SetWindowSubclass(
            parent_hwnd,
            Some(resize_sync_subclass_proc),
            SUBCLASS_ID,
            state_ptr as usize,
        )
    }
    .as_bool()
    {
        unsafe {
            drop(Box::from_raw(state_ptr));
        }
        INSTALLED.store(false, Ordering::Release);
        eprintln!("[Locus] failed to install WebView2 resize sync subclass");
        return;
    }

    unsafe {
        ensure_child_subclasses(&mut *state_ptr);
        sync_from_client_rect(parent_hwnd, &mut *state_ptr, true);
    }
    eprintln!("[Locus] WebView2 resize sync installed");
}

unsafe extern "system" fn resize_sync_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    ref_data: usize,
) -> LRESULT {
    let state_ptr = ref_data as *mut ResizeSyncState;
    if state_ptr.is_null() {
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }

    let state = unsafe { &mut *state_ptr };
    if msg == WM_NCDESTROY {
        unsafe {
            remove_child_subclasses(state);
            let _ = RemoveWindowSubclass(hwnd, Some(resize_sync_subclass_proc), SUBCLASS_ID);
            drop(Box::from_raw(state_ptr));
            INSTALLED.store(false, Ordering::Release);
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
    }

    match msg {
        WM_ENTERSIZEMOVE => {
            state.live_resize = false;
            state.resize_target_active = false;
            unsafe {
                sync_from_client_rect(hwnd, state, true);
            }
        }
        WM_WINDOWPOSCHANGING => unsafe {
            sync_from_changing_window_pos(hwnd, state, lparam);
        },
        WM_SIZE => {
            if wparam.0 != SIZE_MINIMIZED as usize {
                let (width, height) = size_from_lparam(lparam);
                unsafe {
                    sync_webview_bounds(state, width, height, false);
                }
            }
        }
        WM_MOVE | WM_MOVING => unsafe {
            notify_parent_position_changed(state);
        },
        _ => {}
    }

    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };

    match msg {
        WM_WINDOWPOSCHANGED => unsafe {
            sync_from_window_pos(state, lparam, false);
        },
        WM_SIZE => {
            if wparam.0 != SIZE_MINIMIZED as usize {
                let (width, height) = size_from_lparam(lparam);
                unsafe {
                    sync_webview_bounds(state, width, height, false);
                }
            }
        }
        WM_EXITSIZEMOVE => {
            state.live_resize = false;
            state.resize_target_active = false;
            unsafe {
                sync_from_client_rect(hwnd, state, true);
                notify_parent_position_changed(state);
            }
        }
        WM_MOVE | WM_MOVING => unsafe {
            notify_parent_position_changed(state);
        },
        _ => {}
    }

    result
}

unsafe extern "system" fn child_sync_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    ref_data: usize,
) -> LRESULT {
    let state_ptr = ref_data as *mut ResizeSyncState;
    if state_ptr.is_null() {
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }

    let state = unsafe { &mut *state_ptr };
    if msg == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(child_sync_subclass_proc), CHILD_SUBCLASS_ID);
            if state.webview_hwnd.0 == hwnd.0 {
                state.webview_hwnd = HWND::default();
            }
            if state.resize_borders_hwnd.0 == hwnd.0 {
                state.resize_borders_hwnd = HWND::default();
            }
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
    }

    if state.live_resize && msg == WM_WINDOWPOSCHANGING {
        unsafe {
            clamp_changing_child_window_pos(state, lparam);
        }
    }

    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };

    if state.live_resize && (msg == WM_WINDOWPOSCHANGED || msg == WM_SIZE) {
        unsafe {
            clamp_child_window_to_parent_client(state, hwnd);
        }
    }

    result
}

fn size_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let packed = lparam.0 as u32;
    ((packed & 0xffff) as i32, ((packed >> 16) & 0xffff) as i32)
}

unsafe fn sync_from_client_rect(hwnd: HWND, state: &mut ResizeSyncState, force: bool) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    unsafe {
        sync_webview_bounds(state, rect.right - rect.left, rect.bottom - rect.top, force);
    }
}

unsafe fn sync_from_changing_window_pos(hwnd: HWND, state: &mut ResizeSyncState, lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    let window_pos = unsafe { &*(lparam.0 as *const WINDOWPOS) };
    if contains_set_window_pos_flag(window_pos.flags, SWP_NOSIZE) {
        state.live_resize = false;
        state.resize_target_active = false;
        unsafe {
            sync_from_client_rect(hwnd, state, true);
        }
        return;
    }

    let Some(metrics) = (unsafe { window_client_metrics(hwnd) }) else {
        return;
    };
    let current_width = metrics.window_width;
    let current_height = metrics.window_height;
    let current_client_width = metrics.client_width;
    let current_client_height = metrics.client_height;
    let proposed_client_width = proposed_client_dimension(window_pos.cx, metrics.frame_width);
    let proposed_client_height = proposed_client_dimension(window_pos.cy, metrics.frame_height);
    if proposed_client_width < MIN_SYNC_WIDTH_PX || proposed_client_height < MIN_SYNC_HEIGHT_PX {
        return;
    }

    let width_changed = window_pos.cx != current_width;
    let height_changed = window_pos.cy != current_height;
    if !width_changed && !height_changed {
        state.live_resize = false;
        state.resize_target_active = false;
        unsafe {
            publish_native_client_size(state, current_client_width, current_client_height);
            sync_webview_bounds_at(
                state,
                0,
                0,
                current_client_width,
                current_client_height,
                true,
            );
        }
        return;
    }

    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err() {
        return;
    }
    let proposed_left = if contains_set_window_pos_flag(window_pos.flags, SWP_NOMOVE) {
        window_rect.left
    } else {
        window_pos.x
    };
    let proposed_top = if contains_set_window_pos_flag(window_pos.flags, SWP_NOMOVE) {
        window_rect.top
    } else {
        window_pos.y
    };
    let proposed_right = proposed_left + window_pos.cx;
    let proposed_bottom = proposed_top + window_pos.cy;
    let right_stable = (proposed_right - window_rect.right).abs() <= 2;
    let bottom_stable = (proposed_bottom - window_rect.bottom).abs() <= 2;
    let left_stable = proposed_left == window_rect.left;
    let top_stable = proposed_top == window_rect.top;
    let horizontal_resize = width_changed && (right_stable || left_stable);
    let vertical_resize = height_changed && (bottom_stable || top_stable);
    if !horizontal_resize && !vertical_resize {
        state.live_resize = false;
        state.resize_target_active = false;
        unsafe {
            publish_native_client_size(state, current_client_width, current_client_height);
            sync_webview_bounds_at(
                state,
                0,
                0,
                current_client_width,
                current_client_height,
                true,
            );
        }
        return;
    }

    state.live_resize = true;
    state.resize_target_active = true;
    state.resize_target_left = proposed_left;
    state.resize_target_top = proposed_top;
    let child_x = 0;
    let child_y = 0;
    let child_width = if width_changed && right_stable && !left_stable {
        current_client_width.max(proposed_client_width)
    } else {
        proposed_client_width
    };
    let child_height = if height_changed && bottom_stable && !top_stable {
        current_client_height.max(proposed_client_height)
    } else {
        proposed_client_height
    };
    unsafe {
        publish_native_client_size(state, proposed_client_width, proposed_client_height);
        sync_webview_bounds_at(state, child_x, child_y, child_width, child_height, false);
    }
}

unsafe fn sync_from_window_pos(state: &mut ResizeSyncState, lparam: LPARAM, force: bool) {
    if lparam.0 == 0 {
        return;
    }
    let window_pos = unsafe { &*(lparam.0 as *const WINDOWPOS) };
    if contains_set_window_pos_flag(window_pos.flags, SWP_NOSIZE) {
        state.live_resize = false;
        state.resize_target_active = false;
        unsafe {
            sync_from_client_rect(state.parent_hwnd, state, true);
        }
        return;
    }
    unsafe {
        sync_from_client_rect(state.parent_hwnd, state, force);
    }
}

fn contains_set_window_pos_flag(flags: SET_WINDOW_POS_FLAGS, flag: SET_WINDOW_POS_FLAGS) -> bool {
    flags & flag == flag
}

unsafe fn window_client_metrics(hwnd: HWND) -> Option<WindowClientMetrics> {
    let mut window_rect = RECT::default();
    let mut client_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err()
        || unsafe { GetClientRect(hwnd, &mut client_rect) }.is_err()
    {
        return None;
    }

    let window_width = window_rect.right - window_rect.left;
    let window_height = window_rect.bottom - window_rect.top;
    let client_width = client_rect.right - client_rect.left;
    let client_height = client_rect.bottom - client_rect.top;
    Some(WindowClientMetrics {
        window_width,
        window_height,
        client_width,
        client_height,
        frame_width: (window_width - client_width).max(0),
        frame_height: (window_height - client_height).max(0),
    })
}

fn proposed_client_dimension(proposed_window_dimension: i32, frame_dimension: i32) -> i32 {
    (proposed_window_dimension - frame_dimension).max(1)
}

unsafe fn sync_webview_bounds(state: &mut ResizeSyncState, width: i32, height: i32, force: bool) {
    unsafe {
        publish_native_client_size(state, width, height);
        sync_webview_bounds_at(state, 0, 0, width, height, force);
    }
}

unsafe fn publish_native_client_size(state: &mut ResizeSyncState, width: i32, height: i32) {
    if width < MIN_SYNC_WIDTH_PX || height < MIN_SYNC_HEIGHT_PX {
        return;
    }
    if state.last_native_width == width && state.last_native_height == height {
        return;
    }
    state.last_native_width = width;
    state.last_native_height = height;
    let _ = state.app_handle.emit(
        NATIVE_CLIENT_SIZE_EVENT,
        NativeWindowClientSize { width, height },
    );
}

unsafe fn sync_webview_bounds_at(
    state: &mut ResizeSyncState,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    force: bool,
) {
    if width < MIN_SYNC_WIDTH_PX || height < MIN_SYNC_HEIGHT_PX {
        return;
    }
    if !force
        && !state.webview_hwnd.0.is_null()
        && !state.resize_borders_hwnd.0.is_null()
        && state.last_x == x
        && state.last_y == y
        && state.last_width == width
        && state.last_height == height
    {
        return;
    }

    state.last_x = x;
    state.last_y = y;
    state.last_width = width;
    state.last_height = height;

    unsafe {
        ensure_child_subclasses(state);
    }

    let bounds = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    let _ = unsafe { state.controller.SetBounds(bounds) };

    if state.webview_hwnd.0.is_null() {
        let _ = unsafe { state.controller.ParentWindow(&mut state.webview_hwnd) };
    }
    unsafe {
        sync_child_window(state.webview_hwnd, x, y, width, height);
    }

    if state.resize_borders_hwnd.0.is_null() {
        state.resize_borders_hwnd = unsafe { find_resize_borders_hwnd(state.parent_hwnd) };
    }
    unsafe {
        sync_child_window(state.resize_borders_hwnd, x, y, width, height);
    }

    if state.live_resize {
        unsafe {
            notify_parent_position_changed(state);
        }
    }
}

unsafe fn ensure_child_subclasses(state: &mut ResizeSyncState) {
    if state.webview_hwnd.0.is_null() {
        let _ = unsafe { state.controller.ParentWindow(&mut state.webview_hwnd) };
    }
    unsafe {
        install_child_subclass(state.webview_hwnd, state);
    }

    if state.resize_borders_hwnd.0.is_null() {
        state.resize_borders_hwnd = unsafe { find_resize_borders_hwnd(state.parent_hwnd) };
    }
    unsafe {
        install_child_subclass(state.resize_borders_hwnd, state);
    }
}

unsafe fn install_child_subclass(hwnd: HWND, state: &mut ResizeSyncState) {
    if hwnd.0.is_null() {
        return;
    }

    let _ = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(child_sync_subclass_proc),
            CHILD_SUBCLASS_ID,
            state as *mut ResizeSyncState as usize,
        )
    };
}

unsafe fn remove_child_subclasses(state: &ResizeSyncState) {
    unsafe {
        remove_child_subclass(state.webview_hwnd);
        remove_child_subclass(state.resize_borders_hwnd);
    }
}

unsafe fn remove_child_subclass(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let _ =
        unsafe { RemoveWindowSubclass(hwnd, Some(child_sync_subclass_proc), CHILD_SUBCLASS_ID) };
}

unsafe fn parent_client_size(state: &ResizeSyncState) -> Option<(i32, i32)> {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(state.parent_hwnd, &mut rect) }.is_err() {
        return None;
    }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width < MIN_SYNC_WIDTH_PX || height < MIN_SYNC_HEIGHT_PX {
        return None;
    }

    Some((width, height))
}

fn target_child_frame(state: &ResizeSyncState) -> Option<(i32, i32, i32, i32)> {
    if state.live_resize
        && state.last_width >= MIN_SYNC_WIDTH_PX
        && state.last_height >= MIN_SYNC_HEIGHT_PX
    {
        let mut x = state.last_x;
        let mut y = state.last_y;
        if state.resize_target_active {
            let mut parent_rect = RECT::default();
            if unsafe { GetWindowRect(state.parent_hwnd, &mut parent_rect) }.is_ok() {
                if (parent_rect.left - state.resize_target_left).abs() <= 1 {
                    x = 0;
                }
                if (parent_rect.top - state.resize_target_top).abs() <= 1 {
                    y = 0;
                }
            }
        }
        return Some((x, y, state.last_width, state.last_height));
    }

    unsafe { parent_client_size(state).map(|(width, height)| (0, 0, width, height)) }
}

unsafe fn clamp_changing_child_window_pos(state: &ResizeSyncState, lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    let Some((x, y, width, height)) = target_child_frame(state) else {
        return;
    };

    let window_pos = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
    window_pos.x = x;
    window_pos.y = y;
    window_pos.cx = width;
    window_pos.cy = height;
    window_pos.flags = SET_WINDOW_POS_FLAGS(window_pos.flags.0 & !SWP_NOMOVE.0 & !SWP_NOSIZE.0);
}

unsafe fn clamp_child_window_to_parent_client(state: &ResizeSyncState, hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let Some((x, y, width, height)) = target_child_frame(state) else {
        return;
    };

    let Some(client_origin) = (unsafe { parent_client_origin(state.parent_hwnd) }) else {
        return;
    };
    let mut child_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut child_rect) }.is_err() {
        return;
    }

    if child_rect.left == client_origin.x + x
        && child_rect.top == client_origin.y + y
        && child_rect.right == client_origin.x + x + width
        && child_rect.bottom == client_origin.y + y + height
    {
        return;
    }

    unsafe {
        sync_child_window(hwnd, x, y, width, height);
    }
}

unsafe fn parent_client_origin(parent_hwnd: HWND) -> Option<POINT> {
    let mut point = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(parent_hwnd, &mut point) }.as_bool() {
        Some(point)
    } else {
        None
    }
}

unsafe fn notify_parent_position_changed(state: &ResizeSyncState) {
    let _ = unsafe { state.controller.NotifyParentWindowPositionChanged() };
}

unsafe fn sync_child_window(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    if hwnd.0.is_null() {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
}

unsafe fn find_resize_borders_hwnd(parent_hwnd: HWND) -> HWND {
    let mut found = HWND::default();
    let found_ptr = &mut found as *mut HWND;
    let _ = unsafe {
        EnumChildWindows(
            Some(parent_hwnd),
            Some(find_resize_borders_proc),
            LPARAM(found_ptr as isize),
        )
    };
    found
}

unsafe extern "system" fn find_resize_borders_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut class_name = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if len > 0 {
        let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
        if class_name == RESIZE_BORDERS_CLASS_NAME {
            let found = lparam.0 as *mut HWND;
            if !found.is_null() {
                unsafe {
                    *found = hwnd;
                }
            }
            return BOOL(0);
        }
    }
    BOOL(1)
}
