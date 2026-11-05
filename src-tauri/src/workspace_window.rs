use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::work_state::WorkWindow;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef, WorkspaceRuntime};

pub const WORKSPACE_WINDOW_LABEL_PREFIX: &str = "workspace-app-";
/// Label of the window Locus opens first and the template every application
/// window is built from. It is an ordinary application window: it can be
/// closed while others stay open, and nothing outlives the last window.
pub const DEFAULT_WINDOW_LABEL: &str = "main";
/// Broadcast with the new primary label (or `null`) when it changes.
pub const PRIMARY_WINDOW_CHANGED_EVENT: &str = "locus-primary-window-changed";

pub fn is_workspace_window(label: &str) -> bool {
    label.starts_with(WORKSPACE_WINDOW_LABEL_PREFIX)
}

/// Top-level windows a user works in. Floating workbench windows, page
/// windows and dialogs are owned by one of these and close with their owner.
pub fn is_application_window(label: &str) -> bool {
    label == DEFAULT_WINDOW_LABEL || is_workspace_window(label)
}

#[derive(Default)]
struct ApplicationWindows {
    /// Creation order. The first live entry is the primary window.
    order: Vec<String>,
    /// Windows parked by "minimize to tray". They were the last visible
    /// window when parked, so they never count as survivors for the others.
    tray_hidden: Vec<String>,
    primary: Option<String>,
}

impl ApplicationWindows {
    fn register(&mut self, label: &str) {
        if !self.order.iter().any(|known| known == label) {
            self.order.push(label.to_string());
        }
    }

    fn unregister(&mut self, label: &str) {
        self.order.retain(|known| known != label);
        self.tray_hidden.retain(|known| known != label);
    }

    /// First registered window that is still alive, then any live application
    /// window that was never registered.
    fn primary_among(&self, live: &[String]) -> Option<String> {
        self.order
            .iter()
            .find(|label| live.contains(label))
            .or_else(|| live.iter().find(|label| is_application_window(label)))
            .cloned()
    }

    fn surviving_windows(&self, live: &[String], closing: &str) -> usize {
        live.iter()
            .filter(|label| {
                is_application_window(label)
                    && label.as_str() != closing
                    && !self.tray_hidden.contains(label)
            })
            .count()
    }
}

fn application_windows() -> &'static Mutex<ApplicationWindows> {
    static STATE: OnceLock<Mutex<ApplicationWindows>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

fn live_application_labels(app: &AppHandle) -> Vec<String> {
    let mut labels: Vec<String> = app
        .webview_windows()
        .into_keys()
        .filter(|label| is_application_window(label))
        .collect();
    labels.sort();
    labels
}

/// Recomputes the primary window and, when it moved, tells every renderer and
/// hands the Unity embed host to the new owner.
fn refresh_primary(app: &AppHandle) {
    let live = live_application_labels(app);
    let (previous, next) = {
        let Ok(mut windows) = application_windows().lock() else {
            return;
        };
        let next = windows.primary_among(&live);
        if windows.primary == next {
            return;
        }
        (std::mem::replace(&mut windows.primary, next.clone()), next)
    };
    eprintln!("[Locus] primary window changed: {previous:?} -> {next:?}");
    let _ = app.emit(PRIMARY_WINDOW_CHANGED_EVENT, next.clone());
    crate::unity_embed_webview::set_owner_window(app, next.as_deref());
}

pub fn register_application_window(app: &AppHandle, label: &str) {
    if let Ok(mut windows) = application_windows().lock() {
        windows.register(label);
    }
    refresh_primary(app);
}

pub fn unregister_application_window(app: &AppHandle, label: &str) {
    if let Ok(mut windows) = application_windows().lock() {
        windows.unregister(label);
    }
    refresh_primary(app);
}

/// Label of the window that currently carries process-wide renderer duties
/// (Unity embed hosting, external script opens, update checks).
pub fn primary_application_window_label(app: &AppHandle) -> Option<String> {
    let live = live_application_labels(app);
    application_windows()
        .lock()
        .ok()
        .and_then(|windows| windows.primary_among(&live))
}

pub fn primary_application_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    primary_application_window_label(app).and_then(|label| app.get_webview_window(&label))
}

#[tauri::command]
pub fn get_primary_application_window(app_handle: AppHandle) -> Option<String> {
    primary_application_window_label(&app_handle)
}

/// Whether another application window would keep running if `current_label`
/// closed. A window parked in the tray is the last one, never a survivor.
pub fn has_other_application_windows(app: &AppHandle, current_label: &str) -> bool {
    let live = live_application_labels(app);
    application_windows()
        .lock()
        .map(|windows| windows.surviving_windows(&live, current_label) > 0)
        .unwrap_or(false)
}

pub fn mark_hidden_to_tray(label: &str) {
    if let Ok(mut windows) = application_windows().lock() {
        if !windows.tray_hidden.iter().any(|known| known == label) {
            windows.tray_hidden.push(label.to_string());
        }
    }
}

/// Windows to bring back from the tray, oldest first.
pub fn take_hidden_to_tray() -> Vec<String> {
    application_windows()
        .lock()
        .map(|mut windows| std::mem::take(&mut windows.tray_hidden))
        .unwrap_or_default()
}

pub fn clear_hidden_to_tray(label: &str) {
    if let Ok(mut windows) = application_windows().lock() {
        windows.tray_hidden.retain(|known| known != label);
    }
}

/// Whether startup must create the default window. A previous run that closed
/// it leaves only workspace windows; recreating it would add a window nobody
/// asked for. A hidden default window comes from builds that kept it as an
/// invisible shell, which no longer exists. An explicit launch request (a
/// workspace argument or an external script open) always targets it.
pub fn should_build_default_window(
    windows: &BTreeMap<String, WorkWindow>,
    explicit_launch_request: bool,
) -> bool {
    if explicit_launch_request || !windows.keys().any(|label| is_workspace_window(label)) {
        return true;
    }
    windows
        .get(DEFAULT_WINDOW_LABEL)
        .is_some_and(|record| record.visible)
}

#[tauri::command]
pub async fn open_workspace_window(
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
    workspace_ref: WorkspaceRef,
    background_color: Option<tauri::webview::Color>,
) -> Result<String, String> {
    let scope = registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(|error| error.to_string())?;
    let runtime = scope.runtime();
    let label = format!("{WORKSPACE_WINDOW_LABEL_PREFIX}{}", uuid::Uuid::new_v4());
    app_handle
        .state::<Arc<crate::work_state::WorkStateStore>>()
        .register(&label, &label)?;
    if let Err(error) = build_workspace_window(&app_handle, &label, Some(runtime.as_ref())) {
        let _ = app_handle
            .state::<Arc<crate::work_state::WorkStateStore>>()
            .forget(&label);
        return Err(error);
    }
    if let (Some(color), Some(window)) = (background_color, app_handle.get_webview_window(&label)) {
        let _ = window.set_background_color(Some(color));
    }
    Ok(label)
}

pub fn build_workspace_window(
    app_handle: &AppHandle,
    label: &str,
    runtime: Option<&WorkspaceRuntime>,
) -> Result<(), String> {
    let mut url = tauri::Url::parse("http://localhost/").map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("workspaceWindow", "1")
        .append_pair("restore", "1");
    if let Some(runtime) = runtime {
        url.query_pairs_mut()
            .append_pair("checkoutId", runtime.checkout_id().as_str())
            .append_pair("workspaceGeneration", &runtime.generation().to_string())
            .append_pair(
                "materializationEpoch",
                &runtime.materialization_epoch().to_string(),
            );
    }
    // Reuse the default window's dimensions and native frame, without
    // parenting this window to another application window.
    let mut config = app_handle
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == DEFAULT_WINDOW_LABEL)
        .cloned()
        .ok_or("Default window configuration is unavailable")?;
    config.label = label.to_string();
    config.url = WebviewUrl::App(format!("/?{}", url.query().unwrap_or_default()).into());
    config.visible = false;
    config.focus = false;
    config.title = runtime
        .map(|runtime| {
            format!(
                "Locus - {}",
                runtime
                    .root()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })
        .unwrap_or_else(|| "Locus".into());
    let popup_app = app_handle.clone();
    let popup_owner_label = label.to_string();
    let window = WebviewWindowBuilder::from_config(app_handle, &config)
        .map_err(|error| error.to_string())?
        .on_new_window(move |url, features| {
            crate::shared_workbench_window::handle_new_window(
                &popup_app,
                &popup_owner_label,
                url,
                features,
            )
        })
        .build()
        .map_err(|error| error.to_string())?;
    register_application_window(app_handle, label);
    #[cfg(target_os = "windows")]
    if let Err(error) = crate::windows_window_frame::apply_window_frame(&window) {
        eprintln!("[Locus] warning: failed to restore window frame for {label}: {error}");
    }
    if let Some(record) = app_handle
        .state::<Arc<crate::work_state::WorkStateStore>>()
        .snapshot()
        .windows
        .get(label)
    {
        crate::work_state::apply_hidden_geometry(&window, record)?;
    }
    Ok(())
}

pub fn restore_workspace_windows(app: &AppHandle) {
    let state = app
        .state::<Arc<crate::work_state::WorkStateStore>>()
        .snapshot();
    let registry = app.state::<Arc<ProjectRegistry>>();
    let contexts = app.state::<Arc<crate::workspace_service::WindowContextRegistry>>();
    let contexts = contexts.snapshots().unwrap_or_default();
    for record in state
        .windows
        .values()
        .filter(|record| is_workspace_window(&record.label))
    {
        if app.get_webview_window(&record.label).is_some() {
            continue;
        }
        let checkout = record
            .frontend
            .get("activeWorkspaceId")
            .and_then(|value| value.as_str());
        let context = contexts
            .iter()
            .filter(|context| context.window_id == record.label)
            .filter(|context| checkout.is_none_or(|id| context.focused_checkout_id.as_str() == id))
            .max_by_key(|context| context.intent_epoch);
        let runtime = context.and_then(|context| registry.runtime(&context.focused_checkout_id));
        if let Err(error) = build_workspace_window(app, &record.label, runtime.as_deref()) {
            eprintln!("[WorkState] failed to rebuild {}: {error}", record.label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn windows(values: &[&str]) -> ApplicationWindows {
        let mut windows = ApplicationWindows::default();
        for value in values {
            windows.register(value);
        }
        windows
    }

    #[test]
    fn primary_is_the_oldest_window_still_open() {
        let windows = windows(&["main", "workspace-app-b", "workspace-app-a"]);
        let live = labels(&["main", "workspace-app-a", "workspace-app-b"]);
        assert_eq!(windows.primary_among(&live).as_deref(), Some("main"));
    }

    #[test]
    fn primary_moves_to_the_next_window_when_the_first_closes() {
        let mut windows = windows(&["main", "workspace-app-b", "workspace-app-a"]);
        windows.unregister("main");
        let live = labels(&["workspace-app-a", "workspace-app-b"]);
        assert_eq!(
            windows.primary_among(&live).as_deref(),
            Some("workspace-app-b")
        );
    }

    #[test]
    fn primary_skips_registered_windows_that_are_gone() {
        let windows = windows(&["main", "workspace-app-a"]);
        let live = labels(&["workspace-app-a"]);
        assert_eq!(
            windows.primary_among(&live).as_deref(),
            Some("workspace-app-a")
        );
    }

    #[test]
    fn an_unregistered_application_window_can_still_be_primary() {
        let windows = ApplicationWindows::default();
        let live = labels(&["sub-pool-1", "workspace-app-a"]);
        assert_eq!(
            windows.primary_among(&live).as_deref(),
            Some("workspace-app-a")
        );
        assert_eq!(windows.primary_among(&labels(&["sub-pool-1"])), None);
    }

    #[test]
    fn only_application_windows_keep_the_session_alive() {
        let windows = windows(&["main"]);
        let live = labels(&[
            "main",
            "workbench-floating",
            "workspace-page-settings",
            "plan-view",
        ]);
        assert_eq!(windows.surviving_windows(&live, "main"), 0);
        let live = labels(&["main", "workspace-app-a", "plan-view"]);
        assert_eq!(windows.surviving_windows(&live, "main"), 1);
        assert_eq!(windows.surviving_windows(&live, "workspace-app-a"), 1);
    }

    #[test]
    fn a_window_parked_in_the_tray_is_not_a_survivor() {
        let mut windows = windows(&["main", "workspace-app-a"]);
        windows.tray_hidden.push("main".into());
        let live = labels(&["main", "workspace-app-a"]);
        assert_eq!(windows.surviving_windows(&live, "workspace-app-a"), 0);
        windows.unregister("main");
        assert!(windows.tray_hidden.is_empty());
    }

    fn work_state(records: &[(&str, bool)]) -> BTreeMap<String, WorkWindow> {
        records
            .iter()
            .map(|(label, visible)| {
                let mut record = WorkWindow::new(label, label);
                record.visible = *visible;
                (label.to_string(), record)
            })
            .collect()
    }

    #[test]
    fn the_default_window_is_built_when_no_workspace_window_was_saved() {
        assert!(should_build_default_window(&work_state(&[]), false));
        assert!(should_build_default_window(
            &work_state(&[("main", true)]),
            false
        ));
    }

    #[test]
    fn a_closed_default_window_is_not_rebuilt_beside_saved_workspace_windows() {
        let saved = work_state(&[("workspace-app-a", true)]);
        assert!(!should_build_default_window(&saved, false));
    }

    #[test]
    fn a_hidden_shell_window_from_an_older_build_is_dropped() {
        let saved = work_state(&[("main", false), ("workspace-app-a", true)]);
        assert!(!should_build_default_window(&saved, false));
        let saved = work_state(&[("main", true), ("workspace-app-a", true)]);
        assert!(should_build_default_window(&saved, false));
    }

    #[test]
    fn an_explicit_launch_request_always_builds_the_default_window() {
        let saved = work_state(&[("workspace-app-a", true)]);
        assert!(should_build_default_window(&saved, true));
    }
}
