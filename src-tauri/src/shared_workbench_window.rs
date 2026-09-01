use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

const MAIN_WINDOW_LABEL: &str = "main";
const SHARED_WORKBENCH_WINDOW_FRAGMENT_PREFIX: &str = "locus-shared-workbench-";
const SHARED_WORKBENCH_WINDOW_LABEL_PREFIX: &str = "workbench-";
pub const SHARED_WORKBENCH_DRAG_POINT_EVENT: &str = "shared-workbench:drag-point";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedWorkbenchDragPoint {
    x: i32,
    y: i32,
    left_button_pressed: bool,
    target_window_label: Option<String>,
}

fn drag_tracker_state() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    static STATE: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
pub fn start_shared_workbench_drag_tracking(app_handle: AppHandle) -> Result<(), String> {
    stop_shared_workbench_drag_tracking();
    let stop = Arc::new(AtomicBool::new(false));
    *drag_tracker_state()
        .lock()
        .map_err(|_| "Shared Workbench drag tracker state is unavailable".to_string())? =
        Some(stop.clone());
    std::thread::spawn(move || track_native_drag_pointer(app_handle, stop));
    Ok(())
}

#[tauri::command]
pub fn stop_shared_workbench_drag_tracking() {
    if let Ok(mut state) = drag_tracker_state().lock() {
        if let Some(stop) = state.take() {
            stop.store(true, Ordering::Release);
        }
    }
}

#[cfg(target_os = "windows")]
fn track_native_drag_pointer(app_handle: AppHandle, stop: Arc<AtomicBool>) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let workbench_windows = app_handle
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| {
            label == MAIN_WINDOW_LABEL || label.starts_with(SHARED_WORKBENCH_WINDOW_LABEL_PREFIX)
        })
        .filter_map(|(label, window)| window.hwnd().ok().map(|hwnd| (label, hwnd)))
        .collect::<Vec<_>>();

    let mut last = POINT {
        x: i32::MIN,
        y: i32::MIN,
    };
    let mut last_pressed = false;
    while !stop.load(Ordering::Acquire) {
        let mut point = POINT::default();
        let left_button_pressed = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
        if unsafe { GetCursorPos(&mut point) }.is_ok()
            && (point.x != last.x || point.y != last.y || left_button_pressed != last_pressed)
        {
            last = point;
            last_pressed = left_button_pressed;
            let target_window_label =
                unsafe { workbench_window_label_at(point, &workbench_windows) };
            let _ = app_handle.emit(
                SHARED_WORKBENCH_DRAG_POINT_EVENT,
                SharedWorkbenchDragPoint {
                    x: point.x,
                    y: point.y,
                    left_button_pressed,
                    target_window_label,
                },
            );
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

#[cfg(target_os = "windows")]
unsafe fn workbench_window_label_at(
    point: windows::Win32::Foundation::POINT,
    workbench_windows: &[(String, windows::Win32::Foundation::HWND)],
) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetTopWindow, GetWindow, GetWindowRect, IsWindowVisible, WindowFromPoint,
        GA_ROOT, GW_HWNDNEXT,
    };

    let direct_root = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    if let Some(label) = workbench_windows
        .iter()
        .find_map(|(label, hwnd)| (*hwnd == direct_root).then(|| label.clone()))
    {
        return Some(label);
    }

    let mut current = unsafe { GetTopWindow(None).ok()? };
    loop {
        if let Some(label) = workbench_windows.iter().find_map(|(label, hwnd)| {
            if *hwnd != current || !unsafe { IsWindowVisible(current).as_bool() } {
                return None;
            }
            let mut bounds = windows::Win32::Foundation::RECT::default();
            if unsafe { GetWindowRect(current, &mut bounds) }.is_err()
                || point.x < bounds.left
                || point.x >= bounds.right
                || point.y < bounds.top
                || point.y >= bounds.bottom
            {
                return None;
            }
            Some(label.clone())
        }) {
            return Some(label);
        }
        current = match unsafe { GetWindow(current, GW_HWNDNEXT) } {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn track_native_drag_pointer(_app_handle: AppHandle, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn about_blank() -> Result<tauri::Url, String> {
    "about:blank"
        .parse()
        .map_err(|error| format!("Invalid about:blank URL: {error}"))
}

pub fn label_from_url(url: &tauri::Url) -> Option<String> {
    if url.scheme() != "about" || url.path() != "blank" {
        return None;
    }
    let label = url
        .fragment()?
        .strip_prefix(SHARED_WORKBENCH_WINDOW_FRAGMENT_PREFIX)?;
    if !label.starts_with(SHARED_WORKBENCH_WINDOW_LABEL_PREFIX)
        || label.len() > 96
        || !label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    Some(label.to_string())
}

fn base_window_builder<'a>(
    app_handle: &'a AppHandle,
    label: &str,
) -> Result<WebviewWindowBuilder<'a, Wry, AppHandle<Wry>>, String> {
    let mut builder =
        WebviewWindowBuilder::new(app_handle, label, WebviewUrl::External(about_blank()?))
            .title("Locus")
            .decorations(false)
            .shadow(true)
            .resizable(true)
            .maximizable(true)
            .minimizable(true)
            .closable(true)
            .inner_size(1120.0, 760.0)
            .min_inner_size(620.0, 420.0)
            .visible(false);
    if let Some(main_window) = app_handle.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder
            .parent(&main_window)
            .map_err(|error| format!("Failed to parent shared Workbench window: {error}"))?;
    }
    Ok(builder)
}

pub fn handle_new_window(
    app_handle: &AppHandle,
    url: tauri::Url,
    _features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    let Some(label) = label_from_url(&url) else {
        return NewWindowResponse::Deny;
    };

    if app_handle.get_webview_window(&label).is_some() {
        return NewWindowResponse::Deny;
    }

    let builder = match base_window_builder(app_handle, &label) {
        Ok(builder) => builder,
        Err(error) => {
            eprintln!("[WorkbenchWindow] failed to prepare shared child {label}: {error}");
            return NewWindowResponse::Deny;
        }
    };
    match builder.build() {
        Ok(window) => {
            eprintln!("[WorkbenchWindow] shared child created directly: {label}");
            NewWindowResponse::Create { window }
        }
        Err(error) => {
            eprintln!("[WorkbenchWindow] failed to create shared child {label}: {error}");
            NewWindowResponse::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_scoped_about_blank_workbench_urls() {
        let valid: tauri::Url = "about:blank#locus-shared-workbench-workbench-window-1"
            .parse()
            .unwrap();
        assert_eq!(
            label_from_url(&valid).as_deref(),
            Some("workbench-window-1")
        );

        for invalid in [
            "https://example.com/#locus-shared-workbench-workbench-window-1",
            "about:blank#workbench-window-1",
            "about:blank#locus-shared-workbench-other-window-1",
            "about:blank#locus-shared-workbench-workbench-window_1",
        ] {
            assert_eq!(label_from_url(&invalid.parse().unwrap()), None, "{invalid}");
        }
    }
}
