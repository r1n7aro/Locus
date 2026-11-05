use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

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
            crate::workspace_window::is_application_window(label)
                || label.starts_with(SHARED_WORKBENCH_WINDOW_LABEL_PREFIX)
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

/// Floating windows are independent top-level windows: no native owner, so
/// they keep their own taskbar button, minimize and z-order, like VS Code's
/// auxiliary windows. `FloatingWindows` ties their lifetime to the opener.
fn base_window_builder<'a>(
    app_handle: &'a AppHandle,
    label: &str,
) -> Result<WebviewWindowBuilder<'a, Wry, AppHandle<Wry>>, String> {
    let builder =
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
            .visible(false)
            // A focused webview takes the foreground at creation even while its
            // window is hidden. Revealing a claimed window focuses it, and
            // `webview_drop_target` registers the drop targets wry misses.
            .focused(false);
    Ok(builder)
}

fn prepared_windows() -> &'static Mutex<std::collections::HashMap<String, String>> {
    static WINDOWS: OnceLock<Mutex<std::collections::HashMap<String, String>>> = OnceLock::new();
    WINDOWS.get_or_init(Default::default)
}

#[tauri::command]
pub async fn prepare_shared_workbench_window(
    app: AppHandle,
    window: tauri::WebviewWindow,
    label: String,
) -> Result<(), String> {
    let url: tauri::Url = format!("about:blank#{SHARED_WORKBENCH_WINDOW_FRAGMENT_PREFIX}{label}")
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;
    if label_from_url(&url).as_deref() != Some(&label)
        || !crate::workspace_window::is_application_window(window.label())
    {
        return Err("Invalid shared Workbench window request".into());
    }
    if app.get_webview_window(&label).is_some() {
        return Err("Workbench window already exists".into());
    }
    // Building inside NewWindowRequested can reenter another WebView creation
    // while Wry holds its WebContext mutex. Prepare from an async command and
    // let the synchronous callback only hand over this already-created window.
    let prepared = base_window_builder(&app, &label)?
        .build()
        .map_err(|e| e.to_string())?;
    with_floating_windows(|windows| windows.register(&label, window.label()));
    prepared_windows()
        .lock()
        .unwrap()
        .insert(label, window.label().into());
    let _ = prepared;
    Ok(())
}

/// Floating windows render the Vue tree of the application window that opened
/// them, so they cannot outlive that window's page. Without a native owner,
/// this table records which application window drives each one.
#[derive(Default)]
struct FloatingWindows {
    /// Floating window label -> label of the application window that opened it.
    owners: HashMap<String, String>,
    /// Floating windows closed because their opener's page reloaded. The new
    /// page reopens them, so their saved state must survive the close.
    reloading: HashSet<String>,
    /// Application window label -> floating windows hidden with it in the tray.
    tray_hidden: HashMap<String, Vec<String>>,
}

impl FloatingWindows {
    fn register(&mut self, label: &str, owner: &str) {
        self.owners.insert(label.to_string(), owner.to_string());
    }

    fn owner_of(&self, label: &str) -> Option<&str> {
        self.owners.get(label).map(String::as_str)
    }

    fn owned_by(&self, owner: &str) -> Vec<String> {
        let mut labels: Vec<String> = self
            .owners
            .iter()
            .filter(|(_, candidate)| candidate.as_str() == owner)
            .map(|(label, _)| label.clone())
            .collect();
        labels.sort();
        labels
    }

    fn mark_reloading(&mut self, owner: &str) -> Vec<String> {
        let labels = self.owned_by(owner);
        self.reloading.extend(labels.iter().cloned());
        labels
    }

    /// Drops every trace of a destroyed window, floating or application.
    fn forget(&mut self, label: &str) {
        self.owners.remove(label);
        self.reloading.remove(label);
        self.tray_hidden.remove(label);
        for hidden in self.tray_hidden.values_mut() {
            hidden.retain(|candidate| candidate != label);
        }
    }
}

fn floating_windows() -> &'static Mutex<FloatingWindows> {
    static WINDOWS: OnceLock<Mutex<FloatingWindows>> = OnceLock::new();
    WINDOWS.get_or_init(Default::default)
}

/// Window operations below never run under this lock: showing or hiding on the
/// main thread dispatches window events synchronously, which take it again.
fn with_floating_windows<R>(update: impl FnOnce(&mut FloatingWindows) -> R) -> Option<R> {
    floating_windows()
        .lock()
        .ok()
        .map(|mut windows| update(&mut windows))
}

/// Label of the application window whose page renders floating window `label`.
pub fn owner_of(label: &str) -> Option<String> {
    with_floating_windows(|windows| windows.owner_of(label).map(str::to_string)).flatten()
}

/// Whether a floating window being destroyed comes back with its opener's
/// reloaded page, so its saved work state and pane contexts must be kept.
pub fn keeps_state_on_destroy(label: &str) -> bool {
    with_floating_windows(|windows| windows.reloading.contains(label)).unwrap_or(false)
}

/// Closes the floating windows of an application window that was destroyed.
pub fn destroy_owned_windows(app_handle: &AppHandle, owner: &str) {
    let labels = with_floating_windows(|windows| windows.owned_by(owner)).unwrap_or_default();
    destroy_windows(app_handle, &labels);
}

/// The page that rendered these windows is gone. The reloaded page reopens
/// them from saved work state, so only the native windows are closed.
pub fn close_owned_windows_for_reload(app_handle: &AppHandle, owner: &str) {
    let labels = with_floating_windows(|windows| windows.mark_reloading(owner)).unwrap_or_default();
    destroy_windows(app_handle, &labels);
}

fn destroy_windows(app_handle: &AppHandle, labels: &[String]) {
    for label in labels {
        let Some(window) = app_handle.get_webview_window(label) else {
            with_floating_windows(|windows| windows.forget(label));
            continue;
        };
        // `destroy` is queued to the event loop, so it is safe inside window
        // event callbacks; `Destroyed` then clears the bookkeeping.
        if let Err(error) = window.destroy() {
            eprintln!("[Locus] failed to close floating window {label}: {error}");
            with_floating_windows(|windows| windows.reloading.remove(label));
        }
    }
}

/// Hides the visible floating windows of an application window parked in the
/// tray, remembering them for `show_owned_windows_from_tray`.
pub fn hide_owned_windows_to_tray(app_handle: &AppHandle, owner: &str) {
    let labels = with_floating_windows(|windows| windows.owned_by(owner)).unwrap_or_default();
    let mut hidden = Vec::new();
    for label in labels {
        let Some(window) = app_handle.get_webview_window(&label) else {
            continue;
        };
        if !window.is_visible().unwrap_or(false) {
            continue;
        }
        match window.hide() {
            Ok(()) => hidden.push(label),
            Err(error) => eprintln!("[Locus] failed to hide floating window {label}: {error}"),
        }
    }
    if !hidden.is_empty() {
        with_floating_windows(|windows| windows.tray_hidden.insert(owner.to_string(), hidden));
    }
}

pub fn show_owned_windows_from_tray(app_handle: &AppHandle, owner: &str) {
    let labels = with_floating_windows(|windows| windows.tray_hidden.remove(owner))
        .flatten()
        .unwrap_or_default();
    for label in labels {
        let Some(window) = app_handle.get_webview_window(&label) else {
            continue;
        };
        if let Err(error) = window.show() {
            eprintln!("[Locus] failed to show floating window {label}: {error}");
        }
    }
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if !matches!(event, tauri::WindowEvent::Destroyed) {
        return;
    }
    with_floating_windows(|windows| windows.forget(window.label()));
    if let Ok(mut prepared) = prepared_windows().lock() {
        prepared.remove(window.label());
    }
}

pub fn handle_new_window(
    app_handle: &AppHandle,
    owner_label: &str,
    url: tauri::Url,
    _features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    let Some(label) = label_from_url(&url) else {
        return NewWindowResponse::Deny;
    };

    let reserved_owner = prepared_windows().lock().unwrap().remove(&label);
    if reserved_owner.as_deref() != Some(owner_label) {
        return NewWindowResponse::Deny;
    }
    match app_handle.get_webview_window(&label) {
        Some(window) => NewWindowResponse::Create { window },
        None => NewWindowResponse::Deny,
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

    fn floating(entries: &[(&str, &str)]) -> FloatingWindows {
        let mut windows = FloatingWindows::default();
        for (label, owner) in entries {
            windows.register(label, owner);
        }
        windows
    }

    #[test]
    fn floating_windows_are_listed_by_the_window_that_opened_them() {
        let windows = floating(&[
            ("workbench-shared-b", "main"),
            ("workbench-shared-a", "main"),
            ("workbench-shared-c", "workspace-app-x"),
        ]);
        assert_eq!(
            windows.owner_of("workbench-shared-c"),
            Some("workspace-app-x")
        );
        assert_eq!(windows.owner_of("workbench-shared-z"), None);
        assert_eq!(
            windows.owned_by("main"),
            ["workbench-shared-a", "workbench-shared-b"]
        );
        assert!(windows.owned_by("workspace-app-y").is_empty());
    }

    #[test]
    fn a_reload_keeps_state_only_until_the_old_window_is_destroyed() {
        let mut windows = floating(&[
            ("workbench-shared-a", "main"),
            ("workbench-shared-b", "workspace-app-x"),
        ]);
        assert_eq!(windows.mark_reloading("main"), ["workbench-shared-a"]);
        assert!(windows.reloading.contains("workbench-shared-a"));
        assert!(!windows.reloading.contains("workbench-shared-b"));

        windows.forget("workbench-shared-a");
        assert!(windows.reloading.is_empty());
        assert_eq!(windows.owner_of("workbench-shared-a"), None);

        // The reloaded page reopens the window under the same label; closing
        // it later is an ordinary close again.
        windows.register("workbench-shared-a", "main");
        assert!(!windows.reloading.contains("workbench-shared-a"));
    }

    #[test]
    fn destroyed_windows_leave_no_tray_bookkeeping() {
        let mut windows = floating(&[
            ("workbench-shared-a", "main"),
            ("workbench-shared-b", "main"),
        ]);
        windows.tray_hidden.insert(
            "main".into(),
            vec!["workbench-shared-a".into(), "workbench-shared-b".into()],
        );
        windows.forget("workbench-shared-a");
        assert_eq!(windows.tray_hidden["main"], ["workbench-shared-b"]);
        windows.forget("main");
        assert!(windows.tray_hidden.is_empty());
        assert_eq!(windows.owner_of("workbench-shared-b"), Some("main"));
    }
}
