//! Full Locus workbenches hosted by WebView2 directly in Unity's container.
//! The associated documents share the desktop Vue owner and its IPC channel.
//! No Tauri window is created, restyled, or reparented for these surfaces.
use crate::workspace_service::WorkspaceRef;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT: &str = "unity-native-embed:changed";
pub const DOCUMENT_CLOSED: &str = "unity-native-embed:document-closed";
pub const FRAGMENT: &str = "about:blank#locus-unity-workbench-";
#[path = "unity_embed_recovery.rs"]
mod recovery;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub label: String,
    pub window_id: String,
    pub target_kind: String,
    pub target_id: String,
    pub title: String,
    pub workspace_ref: WorkspaceRef,
    pub parent_hwnd: i64,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub visible: bool,
    pub generation: String,
    pub document_id: String,
    pub ready: bool,
    pub error: Option<String>,
}

/// Window whose renderer hosts the embedded workbench documents. It follows
/// the primary application window, so closing the first window hands the
/// documents to the next one instead of orphaning them.
fn owner_state() -> &'static std::sync::Mutex<Option<String>> {
    static OWNER: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
        std::sync::OnceLock::new();
    OWNER.get_or_init(Default::default)
}
pub fn owner_label() -> String {
    owner_state()
        .lock()
        .ok()
        .and_then(|owner| owner.clone())
        .unwrap_or_else(|| crate::workspace_window::DEFAULT_WINDOW_LABEL.to_string())
}
/// Moves hosting to `label` and installs the WebView2 hooks on it. The new
/// owner's renderer then calls `unity_native_embed_attach`, which regenerates
/// every surface exactly as it does after a renderer reload.
pub fn set_owner_window(app: &AppHandle, label: Option<&str>) {
    let changed = owner_state()
        .lock()
        .map(|mut owner| {
            let changed = owner.as_deref() != label;
            *owner = label.map(str::to_string);
            changed
        })
        .unwrap_or(false);
    let Some(label) = label.filter(|_| changed) else {
        return;
    };
    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        let label = label.to_string();
        let scheduled = app.clone().run_on_main_thread(move || {
            if let Err(error) = windows_host::install(&app, &label) {
                eprintln!("[UnityWebView] failed to install on window '{label}': {error}");
            }
        });
        if let Err(error) = scheduled {
            eprintln!("[UnityWebView] failed to schedule owner install: {error}");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, label);
    }
}
fn publish(app: &AppHandle) {
    refresh_snapshot();
    let _ = app.emit_to(owner_label(), EVENT, snapshot());
}
fn snapshots() -> &'static std::sync::Mutex<Vec<Surface>> {
    static SNAPSHOTS: std::sync::OnceLock<std::sync::Mutex<Vec<Surface>>> =
        std::sync::OnceLock::new();
    SNAPSHOTS.get_or_init(Default::default)
}
fn refresh_snapshot() {
    #[cfg(target_os = "windows")]
    {
        *snapshots().lock().unwrap() = windows_host::snapshot();
    }
}

#[cfg(target_os = "windows")]
mod windows_host {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};
    use webview2_com::{
        CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
        Microsoft::Web::WebView2::Win32::*, NavigationStartingEventHandler,
        NewWindowRequestedEventHandler, ProcessFailedEventHandler, WebMessageReceivedEventHandler,
        WebResourceRequestedEventHandler,
    };
    use windows::{
        core::{w, Interface, PWSTR},
        Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::ScreenToClient,
            System::Threading::{AttachThreadInput, GetCurrentThreadId},
            UI::{
                Shell::SHCreateMemStream,
                WindowsAndMessaging::{
                    CreateWindowExW, GetClassNameW, IsWindow, IsWindowVisible, WINDOW_EX_STYLE,
                    WS_POPUP, GetWindowThreadProcessId, SetForegroundWindow,
                },
            },
        },
    };

    struct Entry {
        surface: Surface,
        controller: Option<ICoreWebView2Controller>,
        opening: bool,
        started: std::time::Instant,
        missing_parent: Option<std::time::Instant>,
        recovery: recovery::RecoveryBudget,
    }
    struct Idle {
        document_id: String,
        generation: String,
        controller: ICoreWebView2Controller,
    }
    #[derive(Default)]
    struct Host {
        entries: HashMap<String, Entry>,
        retired: std::collections::VecDeque<(Surface, ICoreWebView2Controller)>,
        idle: std::collections::VecDeque<Idle>,
        created: u64,
        reused: u64,
        closed: HashMap<String, u64>,
        installed: bool,
        parking: Option<HWND>,
        environment: Option<ICoreWebView2Environment>,
        recovery: recovery::RecoveryBudget,
        recovering: bool,
        recovery_started: Option<std::time::Instant>,
        file_drop: HashMap<&'static str, u64>,
    }
    thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }

    fn parent(surface: &Surface) -> HWND {
        HWND(surface.parent_hwnd as isize as *mut _)
    }
    fn valid_parent(hwnd: HWND) -> bool {
        unsafe {
            let mut class = [0u16; 128];
            IsWindow(Some(hwnd)).as_bool() && {
                let n = GetClassNameW(hwnd, &mut class) as usize;
                String::from_utf16_lossy(&class[..n]).starts_with("Unity")
            }
        }
    }
    fn apply(controller: &ICoreWebView2Controller, surface: &Surface) -> windows::core::Result<()> {
        unsafe {
            let hwnd = parent(surface);
            if !valid_parent(hwnd) {
                return controller.SetIsVisible(false);
            }
            let mut current = HWND::default();
            controller.ParentWindow(&mut current)?;
            if current != hwnd {
                controller.SetParentWindow(hwnd)?;
            }
            let mut point = POINT {
                x: surface.x,
                y: surface.y,
            };
            let _ = ScreenToClient(hwnd, &mut point);
            controller.SetBounds(RECT {
                left: point.x,
                top: point.y,
                right: point.x + surface.width.max(1),
                bottom: point.y + surface.height.max(1),
            })?;
            controller.NotifyParentWindowPositionChanged()?;
            controller
                .SetIsVisible(surface.ready && surface.visible && IsWindowVisible(hwnd).as_bool())
        }
    }
    pub(super) fn snapshot() -> Vec<Surface> {
        HOST.with(|h| {
            h.borrow()
                .entries
                .values()
                .map(|e| e.surface.clone())
                .collect()
        })
    }
    pub(super) fn controller(label: &str) -> Option<ICoreWebView2Controller> {
        HOST.with(|h| {
            let h = h.borrow();
            h.entries
                .get(label)
                .and_then(|e| e.controller.clone())
                .or_else(|| {
                    h.idle
                        .iter()
                        .find(|idle| idle.document_id == label)
                        .map(|idle| idle.controller.clone())
                })
        })
    }
    pub(super) fn diagnostics() -> serde_json::Value {
        let (controllers, retired, environment) = HOST.with(|h| {
            let h = h.borrow();
            (
                h.entries
                    .values()
                    .filter_map(|e| e.controller.clone().map(|c| (e.surface.clone(), c)))
                    .collect::<Vec<_>>(),
                h.retired.len(),
                h.environment.clone(),
            )
        });
        let mut children = Vec::new();
        let mut processes = Vec::new();
        unsafe {
            for (surface, controller) in controllers {
                let mut parent = HWND::default();
                let mut bounds = RECT::default();
                let mut visible = Default::default();
                let mut browser = 0;
                let _ = controller.ParentWindow(&mut parent);
                let _ = controller.Bounds(&mut bounds);
                let _ = controller.IsVisible(&mut visible);
                if let Ok(web) = controller.CoreWebView2() {
                    let _ = web.BrowserProcessId(&mut browser);
                }
                children.push(serde_json::json!({"label":surface.label,"generation":surface.generation,"parentHwnd":parent.0 as isize as i64,"visible":visible.as_bool(),"browserProcessId":browser,"bounds":{"x":bounds.left,"y":bounds.top,"width":bounds.right-bounds.left,"height":bounds.bottom-bounds.top}}));
            }
            if let Some(env) =
                environment.and_then(|env| env.cast::<ICoreWebView2Environment8>().ok())
            {
                if let Ok(infos) = env.GetProcessInfos() {
                    let mut count = 0;
                    let _ = infos.Count(&mut count);
                    for i in 0..count {
                        if let Ok(info) = infos.GetValueAtIndex(i) {
                            let mut pid = 0;
                            let mut kind = COREWEBVIEW2_PROCESS_KIND(0);
                            let _ = info.ProcessId(&mut pid);
                            let _ = info.Kind(&mut kind);
                            processes.push(serde_json::json!({"pid":pid,"kind":kind.0}));
                        }
                    }
                }
            }
        }
        let (idle, created, reused, closed) = HOST.with(|h| {
            let h = h.borrow();
            (
                h.idle
                    .iter()
                    .map(|i| i.document_id.clone())
                    .collect::<Vec<_>>(),
                h.created,
                h.reused,
                h.closed.clone(),
            )
        });
        let file_drop = HOST.with(|h| h.borrow().file_drop.clone());
        serde_json::json!({"controllers":children,"retiring":retired,"idle":idle,"created":created,"reused":reused,"closed":closed,"processes":processes,"fileDrop":file_drop})
    }
    pub(super) fn park(label: &str) -> Result<(), String> {
        let pair = HOST.with(|h| {
            let h = h.borrow();
            (
                h.entries.get(label).and_then(|e| e.controller.clone()),
                h.parking,
            )
        });
        if let (Some(controller), Some(parking)) = pair {
            unsafe {
                controller
                    .SetIsVisible(false)
                    .and_then(|_| controller.SetParentWindow(parking))
            }
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    unsafe fn wire_resources(
        app: &AppHandle,
        environment: &ICoreWebView2Environment,
        web: &ICoreWebView2,
    ) -> windows::core::Result<()> {
        let app = app.clone();
        let environment = environment.clone();
        for pattern in [
            "http://tauri.localhost/*",
            "https://tauri.localhost/*",
            "http://locus-binary.localhost/*",
            "https://locus-binary.localhost/*",
        ] {
            let pattern = CoTaskMemPWSTR::from(pattern);
            web.AddWebResourceRequestedFilter(
                *pattern.as_ref().as_pcwstr(),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            )?;
        }
        let mut token = 0;
        web.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let request = args.Request()?;
                let mut raw = PWSTR::null();
                request.Uri(&mut raw)?;
                let uri = CoTaskMemPWSTR::from(raw).to_string();
                let Ok(url) = tauri::Url::parse(&uri) else {
                    return Ok(());
                };
                let resource = match url.host_str() {
                    Some("tauri.localhost") => app
                        .asset_resolver()
                        .get(url.path().trim_start_matches('/').to_string())
                        .map(|asset| (asset.bytes, asset.mime_type)),
                    Some("locus-binary.localhost") => {
                        url.path().strip_prefix("/blob/").and_then(|id| {
                            app.state::<std::sync::Arc<crate::binary_cache::BinaryCache>>()
                                .get(id)
                        })
                    }
                    _ => return Ok(()),
                };
                let (status, bytes, mime) = match resource {
                    Some((bytes, mime)) => (200, bytes, mime),
                    None => (404, Vec::new(), "text/plain".into()),
                };
                let stream = SHCreateMemStream(Some(&bytes));
                let reason = CoTaskMemPWSTR::from(if status == 200 { "OK" } else { "Not Found" });
                let headers = CoTaskMemPWSTR::from(
                    format!("Content-Type: {mime}\r\nAccess-Control-Allow-Origin: *\r\n").as_str(),
                );
                let response = environment.CreateWebResourceResponse(
                    stream.as_ref(),
                    status,
                    *reason.as_ref().as_pcwstr(),
                    *headers.as_ref().as_pcwstr(),
                )?;
                args.SetResponse(&response)?;
                Ok(())
            })),
            &mut token,
        )
    }

    unsafe fn wire_file_drop(
        app: &AppHandle,
        web: &ICoreWebView2,
        document_id: &str,
    ) -> windows::core::Result<()> {
        let app = app.clone();
        let document_id = document_id.to_string();
        let mut token = 0;
        web.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let count = |key| HOST.with(|h| *h.borrow_mut().file_drop.entry(key).or_default() += 1);
                count("received");
                let Some(args) = args else {
                    return Ok(());
                };
                let Some(surface) = snapshot()
                    .into_iter()
                    .find(|s| s.document_id == document_id && s.ready && s.error.is_none())
                else {
                    count("inactive-document");
                    return Ok(());
                };
                let mut source = PWSTR::null();
                args.Source(&mut source)?;
                let source = CoTaskMemPWSTR::from(source).to_string();
                if source != "about:blank" && source != format!("{FRAGMENT}{document_id}") {
                    count("rejected-source");
                    return Ok(());
                }
                let mut raw = PWSTR::null();
                args.WebMessageAsJson(&mut raw)?;
                let raw = CoTaskMemPWSTR::from(raw).to_string();
                if raw.len() > 4096 {
                    return Ok(());
                }
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&raw) else {
                    return Ok(());
                };
                if message["generation"] != surface.generation {
                    count("stale-generation");
                    return Ok(());
                }
                let (Some(x), Some(y)) = (message["x"].as_f64(), message["y"].as_f64()) else {
                    return Ok(());
                };
                if message["type"] == "native-file-drag" {
                    count("drag");
                    let phase = message["phase"].as_str().unwrap_or("");
                    if matches!(phase, "enter" | "over" | "leave") {
                        let _ = crate::commands::native_webview_file_drag(
                            &app,
                            &surface.label,
                            phase,
                            message["count"].as_u64().unwrap_or(0).min(64) as usize,
                            x,
                            y,
                            Vec::new(),
                        );
                    }
                } else if message["type"] == "native-file-drop" {
                    count("drop");
                    let objects = args
                        .cast::<ICoreWebView2WebMessageReceivedEventArgs2>()?
                        .AdditionalObjects()?;
                    let mut length = 0;
                    objects.Count(&mut length)?;
                    let mut paths = Vec::new();
                    for i in 0..length.min(64) {
                        if let Ok(file) = objects.GetValueAtIndex(i)?.cast::<ICoreWebView2File>() {
                            let mut path = PWSTR::null();
                            file.Path(&mut path)?;
                            let path = CoTaskMemPWSTR::from(path).to_string();
                            if !path.is_empty() {
                                paths.push(std::path::PathBuf::from(path));
                            }
                        }
                    }
                    // Paths come exclusively from native File objects, never JSON supplied by a View.
                    count(if paths.is_empty() { "empty-files" } else { "native-files" });
                    let _ = crate::commands::native_webview_file_drag(
                        &app,
                        &surface.label,
                        "drop",
                        paths.len(),
                        x,
                        y,
                        paths,
                    );
                }
                Ok(())
            })),
            &mut token,
        )
    }
    pub(super) fn update(app: &AppHandle, mut surface: Surface) -> Result<(), String> {
        if surface.parent_hwnd == 0 {
            return Ok(());
        } // Unity has not completed its first layout yet.
        if !valid_parent(parent(&surface)) {
            return Err("Unity WebView container is unavailable".into());
        }
        if snapshot().iter().any(|old| {
            old.label == surface.label
                && (old.target_kind != surface.target_kind
                    || old.target_id != surface.target_id
                    || old.workspace_ref != surface.workspace_ref)
        }) {
            close(app, &surface.label);
        }
        let (controller, changed) = HOST.with(|h| {
            let mut h = h.borrow_mut();
            if !h.installed {
                return Err("Locus WebView host is not ready".to_string());
            }
            if let Some(entry) = h.entries.get_mut(&surface.label) {
                if entry.surface.target_kind != surface.target_kind
                    || entry.surface.target_id != surface.target_id
                    || entry.surface.workspace_ref != surface.workspace_ref
                {
                    return Err(
                        "An embedded workbench cannot change its workspace or target in place"
                            .into(),
                    );
                }
                surface.generation = entry.surface.generation.clone();
                surface.document_id = entry.surface.document_id.clone();
                surface.ready = entry.surface.ready;
                surface.error = entry.surface.error.clone();
                let changed = entry.surface.visible != surface.visible;
                entry.surface = surface;
                Ok((entry.controller.clone(), changed))
            } else {
                if h.entries.len() >= 4 {
                    // Expected capacity wait, exposed by the read-only status query.
                    return Ok((None, false));
                }
                surface.generation = uuid::Uuid::new_v4().simple().to_string();
                let idle = h.idle.pop_back();
                let controller = idle.map(|idle| {
                    surface.document_id = idle.document_id;
                    h.reused += 1;
                    idle.controller
                });
                if controller.is_none() {
                    surface.document_id = surface.generation.clone();
                }
                h.entries.insert(
                    surface.label.clone(),
                    Entry {
                        surface,
                        controller: controller.clone(),
                        opening: false,
                        started: std::time::Instant::now(),
                        missing_parent: None,
                        recovery: Default::default(),
                    },
                );
                Ok((controller, true))
            }
        })?;
        if let Some(controller) = controller {
            // Controllers are compared outside any RefCell borrow: COM calls may reenter.
            let current = HOST.with(|h| {
                h.borrow()
                    .entries
                    .values()
                    .find(|e| e.controller.as_ref() == Some(&controller))
                    .map(|e| e.surface.clone())
            });
            if let Some(current) = current {
                apply(&controller, &current).map_err(|e| e.to_string())?;
            }
        }
        refresh_snapshot();
        if changed {
            publish(app);
        }
        Ok(())
    }
    pub(super) fn close(app: &AppHandle, label: &str) {
        let removed = HOST.with(|h| h.borrow_mut().entries.remove(label));
        if let Some(entry) = removed {
            if let Some(controller) = entry.controller {
                let _ = unsafe { controller.SetIsVisible(false) };
                // Retarget through the SDK before releasing the Unity owner. This
                // also lets a parked controller survive a destroyed container HWND.
                if let Some(parking) = HOST.with(|h| h.borrow().parking) {
                    let _ = unsafe { controller.SetParentWindow(parking) };
                }
                let generation = entry.surface.generation.clone();
                let oldest = HOST.with(|h| {
                    let mut h = h.borrow_mut();
                    h.retired.push_back((entry.surface, controller));
                    if h.retired.len() > 4 {
                        h.retired.pop_front()
                    } else {
                        None
                    }
                });
                if let Some((surface, c)) = oldest {
                    dispose_document(app, &surface.document_id, c, "retired-capacity");
                }
                let app = std::sync::Arc::new(app.clone());
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let main_app = app.clone();
                    let _ = app.run_on_main_thread(move || release(&main_app, &generation, false));
                });
            } else {
                let _ = app.emit_to(super::owner_label(), DOCUMENT_CLOSED, &entry.surface.document_id);
            }
            // Vue first unmounts while its owner document is still alive, then
            // acknowledges release. The bounded timeout also handles a failed renderer.
            publish(app);
        }
    }
    fn dispose_document(
        app: &AppHandle,
        document_id: &str,
        controller: ICoreWebView2Controller,
        reason: &str,
    ) {
        HOST.with(|h| *h.borrow_mut().closed.entry(reason.into()).or_default() += 1);
        let _ = unsafe { controller.Close() };
        let _ = app.emit_to(super::owner_label(), DOCUMENT_CLOSED, document_id);
    }
    pub(super) fn release(app: &AppHandle, generation: &str, acknowledged: bool) {
        let retired = HOST.with(|h| {
            let mut h = h.borrow_mut();
            let i = h
                .retired
                .iter()
                .position(|(s, _)| s.generation == generation)?;
            h.retired.remove(i)
        });
        if let Some((surface, controller)) = retired {
            let parking = HOST.with(|h| h.borrow().parking);
            let parked = acknowledged
                && parking
                    .is_some_and(|parent| unsafe { controller.SetParentWindow(parent).is_ok() });
            if !parked {
                dispose_document(
                    app,
                    &surface.document_id,
                    controller,
                    if acknowledged {
                        "park-failed"
                    } else {
                        "unacknowledged"
                    },
                );
                return;
            }
            let document_id = surface.document_id;
            let generation = surface.generation;
            let oldest = HOST.with(|h| {
                let mut h = h.borrow_mut();
                h.idle.push_back(Idle {
                    document_id: document_id.clone(),
                    generation: generation.clone(),
                    controller,
                });
                if h.idle.len() > 2 {
                    h.idle.pop_front()
                } else {
                    None
                }
            });
            if let Some(idle) = oldest {
                dispose_document(app, &idle.document_id, idle.controller, "idle-capacity");
            }
            let app = std::sync::Arc::new(app.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let main_app = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let expired = HOST.with(|h| {
                        let mut h = h.borrow_mut();
                        let i = h.idle.iter().position(|i| {
                            i.document_id == document_id && i.generation == generation
                        })?;
                        h.idle.remove(i)
                    });
                    if let Some(idle) = expired {
                        dispose_document(
                            &main_app,
                            &idle.document_id,
                            idle.controller,
                            "idle-expired",
                        );
                    }
                });
            });
        }
    }
    pub(super) fn attach(app: &AppHandle) -> Vec<Surface> {
        let old = HOST.with(|h| {
            let mut h = h.borrow_mut();
            h.recovering = false;
            h.recovery_started = None;
            let mut controllers = Vec::new();
            for entry in h.entries.values_mut() {
                if let Some(c) = entry.controller.take() {
                    controllers.push((entry.surface.document_id.clone(), c));
                }
                entry.surface.generation = uuid::Uuid::new_v4().simple().to_string();
                entry.surface.document_id = entry.surface.generation.clone();
                entry.surface.ready = false;
                entry.surface.error = None;
                entry.opening = false;
                entry.started = std::time::Instant::now();
            }
            controllers.extend(h.retired.drain(..).map(|(s, c)| (s.document_id, c)));
            controllers.extend(h.idle.drain(..).map(|i| (i.document_id, i.controller)));
            controllers
        });
        for (document_id, controller) in old {
            dispose_document(app, &document_id, controller, "reattach");
        }
        publish(app);
        super::snapshot()
    }
    pub(super) fn ready(app: &AppHandle, label: &str, generation: &str) -> Result<(), String> {
        let pair = HOST.with(|h| {
            let mut h = h.borrow_mut();
            let entry = h
                .entries
                .get_mut(label)
                .ok_or("Embedded workbench was closed")?;
            if entry.surface.generation != generation {
                return Err("Embedded workbench generation changed");
            }
            if entry.surface.error.is_some() || entry.controller.is_none() {
                return Err("Embedded workbench is unavailable; retry required");
            }
            entry.surface.ready = true;
            Ok((entry.controller.clone(), entry.surface.clone()))
        })?;
        if let Some(c) = pair.0 {
            apply(&c, &pair.1).map_err(|e| e.to_string())?;
        }
        publish(app);
        Ok(())
    }
    pub(super) fn focus(label: &str) -> Result<(), String> {
        let controller = HOST.with(|h| {
            h.borrow()
                .entries
                .get(label)
                .and_then(|e| e.controller.clone())
        });
        if let Some(c) = controller {
            unsafe {
                let mut owner = HWND::default();
                c.ParentWindow(&mut owner).map_err(|e| e.to_string())?;
                if !valid_parent(owner) { return Err("Unity owner is unavailable".into()); }
                let current = GetCurrentThreadId();
                let target = GetWindowThreadProcessId(owner, None);
                // Only an explicit focus request activates Unity. Geometry and
                // background recovery must never steal keyboard focus.
                let attached = current != target && target != 0 && AttachThreadInput(current, target, true).as_bool();
                let _ = SetForegroundWindow(owner);
                let result = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                if attached { let _ = AttachThreadInput(current, target, false); }
                result.map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    pub(super) fn install(app: &AppHandle, label: &str) -> Result<(), String> {
        let window = app
            .get_webview_window(label)
            .ok_or_else(|| format!("Locus WebView '{label}' is unavailable"))?;
        let app = app.clone();
        window
            .with_webview(move |platform| unsafe {
                let environment = platform.environment();
                let result = (|| -> windows::core::Result<()> {
                    let web = platform.controller().CoreWebView2()?;
                    let recovery_app = app.clone();
                    let mut process_token = 0;
                    web.add_ProcessFailed(&ProcessFailedEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else { return Ok(()); };
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(0);
                        args.ProcessFailedKind(&mut kind)?;
                        if kind != COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                            && kind != COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED { return Ok(()); }
                        let recover = HOST.with(|h| {
                            let mut h = h.borrow_mut();
                            if h.recovering { return false; }
                            h.recovering = kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                                && h.recovery.take(std::time::Instant::now());
                            h.recovery_started = h.recovering.then(std::time::Instant::now);
                            h.recovering
                        });
                        for s in snapshot() {
                            fail(&recovery_app, &s.label, &s.generation, if recover { "host-recovering" } else { "host-restart-required" });
                        }
                        if recover {
                            // Let all ProcessFailed callbacks finish before reloading the Vue owner.
                            let app = std::sync::Arc::new(recovery_app.clone());
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                if let Some(main) = app.get_webview_window(&super::owner_label()) {
                                    let _ = main.with_webview(|platform| { let _ = platform.controller().CoreWebView2().and_then(|web| web.Reload()); });
                                }
                            });
                        }
                        Ok(())
                    })), &mut process_token)?;
                    // The parking window outlives any one owner; later owners
                    // reuse it and only swap the WebView2 environment.
                    let parking = match HOST.with(|h| h.borrow().parking) {
                        Some(parking) => parking,
                        None => CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            w!("STATIC"),
                            w!("Locus embedded WebView parking"),
                            WS_POPUP,
                            0,
                            0,
                            1,
                            1,
                            None,
                            None,
                            None,
                            None,
                        )?,
                    };
                    HOST.with(|h| {
                        let mut h = h.borrow_mut();
                        h.parking = Some(parking);
                        h.environment = Some(environment.clone());
                    });
                    let mut token = 0;
                    let watch_app = std::sync::Arc::new(app.clone());
                    web.add_NewWindowRequested(
                        &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut uri = PWSTR::null();
                            args.Uri(&mut uri)?;
                            let uri = CoTaskMemPWSTR::from(uri).to_string();
                            let Some(generation) = uri.strip_prefix(FRAGMENT) else {
                                return Ok(());
                            };
                            args.SetHandled(true)?;
                            let found = HOST.with(|h| {
                                let mut h = h.borrow_mut();
                                let entry = h.entries.values_mut().find(|e| {
                                    e.surface.document_id == generation
                                        && !e.opening
                                        && e.controller.is_none()
                                })?;
                                entry.opening = true;
                                Some(entry.surface.clone())
                            });
                            let Some(surface) = found else {
                                return Ok(());
                            };
                            let deferral = args.GetDeferral()?;
                            let finish = deferral.clone();
                            let app = app.clone();
                            let label = surface.label.clone();
                            let generation = surface.generation.clone();
                            let document_id = surface.document_id.clone();
                            let child_environment = environment.clone();
                            let failed_app = app.clone();
                            let handler = CreateCoreWebView2ControllerCompletedHandler::create(
                                Box::new(move |result, controller| {
                                    let setup = (|| -> windows::core::Result<()> {
                                        result?;
                                        let controller = controller.as_ref()
                                            .ok_or_else(windows::core::Error::from_win32)?;
                                        let live = HOST.with(|h| {
                                            h.borrow()
                                                .entries
                                                .get(&label)
                                                .is_some_and(|e| e.surface.generation == generation)
                                        });
                                        if !live {
                                            controller.Close()?;
                                            return Ok(());
                                        }
                                        controller.SetIsVisible(false)?;
                                        let web = controller.CoreWebView2()?;
                                        let failed_app = app.clone();
                                        let failed_document = document_id.clone();
                                        let mut process_token = 0;
                                        web.add_ProcessFailed(&ProcessFailedEventHandler::create(Box::new(move |_, args| {
                                            let Some(args) = args else { return Ok(()); };
                                            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(0);
                                            args.ProcessFailedKind(&mut kind)?;
                                            if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                                                if let Some(s) = snapshot().into_iter().find(|s| s.document_id == failed_document) {
                                                    fail(&failed_app, &s.label, &s.generation, if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED { "renderer-exited" } else { "host-restart-required" });
                                                }
                                            }
                                            Ok(())
                                        })), &mut process_token)?;
                                        let settings = web.Settings()?;
                                        settings.SetAreDefaultContextMenusEnabled(false)?;
                                        if let Ok(settings) = settings.cast::<ICoreWebView2Settings3>() {
                                            // Browser reload shortcuts would destroy the shared Vue document.
                                            settings.SetAreBrowserAcceleratorKeysEnabled(false)?;
                                        }
                                        wire_resources(&app, &child_environment, &web)?;
                                        wire_file_drop(&app, &web, &document_id)?;
                                        let mut nav_token = 0;
                                        let expected = format!("{FRAGMENT}{document_id}");
                                        web.add_NavigationStarting(
                                            &NavigationStartingEventHandler::create(Box::new(
                                                move |_, args| {
                                                    if let Some(args) = args {
                                                        let mut uri = PWSTR::null();
                                                        args.Uri(&mut uri)?;
                                                        let uri =
                                                            CoTaskMemPWSTR::from(uri).to_string();
                                                        if uri != expected && uri != "about:blank" {
                                                            args.SetCancel(true)?;
                                                        }
                                                    }
                                                    Ok(())
                                                },
                                            )),
                                            &mut nav_token,
                                        )?;
                                        let mut popup_token = 0;
                                        web.add_NewWindowRequested(
                                            &NewWindowRequestedEventHandler::create(Box::new(
                                                |_, args| {
                                                    if let Some(args) = args {
                                                        args.SetHandled(true)?;
                                                    }
                                                    Ok(())
                                                },
                                            )),
                                            &mut popup_token,
                                        )?;
                                        let current = HOST.with(|h| {
                                            let mut h = h.borrow_mut();
                                            h.created += 1;
                                            let entry = h
                                                .entries
                                                .get_mut(&label)
                                                .filter(|e| e.surface.generation == generation)?;
                                            entry.controller = Some(controller.clone());
                                            entry.opening = false;
                                            Some(entry.surface.clone())
                                        });
                                        let Some(current) = current else {
                                            controller.Close()?;
                                            return Ok(());
                                        };
                                        apply(&controller, &current)?;
                                        args.SetNewWindow(&web)?;
                                        Ok(())
                                    })();
                                    if let Err(error) = setup {
                                        if let Some(controller) = controller.as_ref() {
                                            let _ = controller.Close();
                                        }
                                        let _ = app.emit_to(super::owner_label(), DOCUMENT_CLOSED, &document_id);
                                        HOST.with(|h| {
                                            if let Some(entry) =
                                                h.borrow_mut().entries.get_mut(&label)
                                            {
                                                if entry.surface.generation == generation {
                                                    entry.controller = None;
                                                    entry.opening = false;
                                                    entry.surface.error = Some(error.to_string());
                                                }
                                            }
                                        });
                                        publish(&app);
                                    }
                                    let _ = finish.Complete();
                                    Ok(())
                                }),
                            );
                            if let Err(error) =
                                environment.CreateCoreWebView2Controller(parent(&surface), &handler)
                            {
                                let _ = deferral.Complete();
                                HOST.with(|h| {
                                    if let Some(entry) = h.borrow_mut().entries.get_mut(&surface.label) {
                                        if entry.surface.generation == surface.generation {
                                            entry.opening = false;
                                            entry.surface.error = Some(error.to_string());
                                        }
                                    }
                                });
                                publish(&failed_app);
                                return Err(error);
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    // One maintenance loop serves every owner.
                    let first_install =
                        HOST.with(|h| !std::mem::replace(&mut h.borrow_mut().installed, true));
                    if first_install {
                        tauri::async_runtime::spawn(async move {
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                let app = watch_app.clone();
                                if watch_app.run_on_main_thread(move || maintain(&app)).is_err() { break; }
                            }
                        });
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    eprintln!("[UnityWebView] installation failed: {error}");
                }
            })
            .map_err(|e| e.to_string())
    }

    pub(super) fn fail(app: &AppHandle, label: &str, generation: &str, error: &str) {
        let controller = HOST.with(|h| {
            let mut h = h.borrow_mut();
            let entry = h
                .entries
                .get_mut(label)
                .filter(|e| e.surface.generation == generation)?;
            if entry.surface.error.as_deref() == Some(error) {
                return None;
            }
            if error == "renderer-exited"
                && matches!(
                    entry.surface.error.as_deref(),
                    Some("host-recovering" | "host-restart-required")
                )
            {
                return None;
            }
            entry.surface.ready = false;
            entry.surface.error = Some(error.into());
            Some(entry.controller.clone())
        });
        let Some(controller) = controller else {
            return;
        };
        if let Some(c) = controller {
            let _ = unsafe { c.SetIsVisible(false) };
        }
        publish(app);
    }

    pub(super) fn retry(app: &AppHandle, label: &str, automatic: bool) -> Result<(), String> {
        let old = HOST.with(|h| {
            let mut h = h.borrow_mut();
            if h.recovering {
                return Ok(None);
            }
            let Some(entry) = h.entries.get_mut(label) else {
                return Ok(None);
            };
            if entry.surface.error.is_none() {
                return Ok(None);
            }
            if entry.surface.error.as_deref() == Some("host-restart-required") {
                return Err("Restart Locus to restore the browser".to_string());
            }
            if automatic && !entry.recovery.take(std::time::Instant::now()) {
                entry.surface.error = Some("recovery-exhausted".into());
                return Ok(None);
            }
            let old = (entry.surface.document_id.clone(), entry.controller.take());
            entry.surface.generation = uuid::Uuid::new_v4().simple().to_string();
            entry.surface.document_id = entry.surface.generation.clone();
            entry.surface.error = None;
            entry.surface.ready = false;
            entry.opening = false;
            entry.started = std::time::Instant::now();
            Ok(Some(old))
        })?;
        if let Some((id, Some(c))) = old {
            dispose_document(app, &id, c, "recovery");
        }
        publish(app);
        Ok(())
    }

    fn maintain(app: &AppHandle) {
        let now = std::time::Instant::now();
        let expired = HOST.with(|h| {
            let mut h = h.borrow_mut();
            if h.recovery_started
                .is_some_and(|at| now.duration_since(at).as_secs() >= 30)
            {
                h.recovering = false;
                h.recovery_started = None;
                true
            } else {
                false
            }
        });
        if expired {
            for s in snapshot() {
                fail(app, &s.label, &s.generation, "host-restart-required");
            }
        }
        for s in snapshot() {
            let missing = HOST.with(|h| {
                let mut h = h.borrow_mut();
                let Some(e) = h.entries.get_mut(&s.label) else {
                    return false;
                };
                if valid_parent(parent(&s)) {
                    e.missing_parent = None;
                    return false;
                }
                now.duration_since(*e.missing_parent.get_or_insert(now))
                    .as_secs()
                    >= 5
            });
            if missing {
                crate::commands::destroy_unity_embed_window_on_main(app, &s.label);
                continue;
            }
            if s.error.as_deref() == Some("renderer-exited") {
                let _ = retry(app, &s.label, true);
            } else if !s.ready
                && s.error.is_none()
                && HOST.with(|h| {
                    h.borrow()
                        .entries
                        .get(&s.label)
                        .is_some_and(|e| now.duration_since(e.started).as_secs() >= 30)
                })
            {
                fail(app, &s.label, &s.generation, "load-timeout");
            }
        }
    }
}

pub fn status(label: &str) -> serde_json::Value {
    let all = snapshot();
    match all.iter().find(|s| s.label == label) {
        Some(s) => {
            serde_json::json!({"state": if s.error.is_some() { "error" } else if s.ready { "ready" } else { "loading" }, "error":s.error})
        }
        None => {
            serde_json::json!({"state": if all.len() >= 4 { "capacity" } else { "loading" }, "error":null})
        }
    }
}

pub fn retry(app: &AppHandle, label: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_host::retry(app, label, false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, label);
        Err("Unsupported platform".into())
    }
}

#[tauri::command]
pub fn unity_native_embed_failed(
    app_handle: AppHandle,
    label: String,
    generation: String,
    error: String,
) {
    #[cfg(target_os = "windows")]
    windows_host::fail(
        &app_handle,
        &label,
        &generation,
        &error.chars().take(1024).collect::<String>(),
    );
}

pub fn snapshot() -> Vec<Surface> {
    snapshots().lock().unwrap().clone()
}
pub fn contains(label: &str) -> bool {
    snapshot().iter().any(|s| s.label == label)
}
pub fn matches_workspace(label: &str, workspace: &WorkspaceRef) -> bool {
    snapshot()
        .iter()
        .any(|s| s.label == label && &s.workspace_ref == workspace)
}
pub fn update(app: &AppHandle, surface: Surface) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_host::update(app, surface)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, surface);
        Err("Unity embedded WebViews require Windows".into())
    }
}
pub fn close(app: &AppHandle, label: &str) {
    #[cfg(target_os = "windows")]
    {
        windows_host::close(app, label);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, label);
    }
}
pub fn focus(label: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_host::focus(label)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = label;
        Ok(())
    }
}
pub fn park(label: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_host::park(label)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = label;
        Ok(())
    }
}
pub async fn diagnostics(app: &AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(windows_host::diagnostics());
        })
        .map_err(|e| e.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .map_err(|_| "WebView diagnostics timed out")?
            .map_err(|_| "WebView diagnostics cancelled".into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Ok(serde_json::json!({"controllers":[],"retiring":0,"processes":[]}))
    }
}
pub async fn devtools(
    app: &AppHandle,
    label: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use webview2_com::{CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR};
        let label = label.to_string();
        let method = method.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
            let callback = tx.clone();
            let result = (|| -> Result<(), String> {
                let controller =
                    windows_host::controller(&label).ok_or("Embedded WebView is unavailable")?;
                let core = unsafe { controller.CoreWebView2() }.map_err(|e| e.to_string())?;
                let method = CoTaskMemPWSTR::from(method.as_str());
                let params = CoTaskMemPWSTR::from(params.to_string().as_str());
                let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    move |result, json| {
                        if let Some(tx) = callback.lock().unwrap().take() {
                            let _ = tx.send(result.map_err(|e| e.to_string()).and_then(|_| {
                                serde_json::from_str(&json).map_err(|e| e.to_string())
                            }));
                        }
                        Ok(())
                    },
                ));
                unsafe {
                    core.CallDevToolsProtocolMethod(
                        *method.as_ref().as_pcwstr(),
                        *params.as_ref().as_pcwstr(),
                        &handler,
                    )
                }
                .map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(Err(error));
                }
            }
        })
        .map_err(|e| e.to_string())?;
        tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .map_err(|_| "Embedded WebView command timed out")?
            .map_err(|_| "Embedded WebView command cancelled")?
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, label, method, params);
        Err("Unsupported platform".into())
    }
}
#[tauri::command]
pub fn unity_native_embed_snapshot() -> Vec<Surface> {
    snapshot()
}
#[tauri::command]
pub fn unity_native_embed_focus(label: String, generation: String) -> Result<(), String> {
    if !snapshot()
        .iter()
        .any(|s| s.label == label && s.generation == generation)
    {
        return Err("Embedded workbench was closed or replaced".into());
    }
    focus(&label)
}
#[tauri::command]
pub fn unity_native_embed_attach(app_handle: AppHandle) -> Vec<Surface> {
    #[cfg(target_os = "windows")]
    {
        windows_host::attach(&app_handle)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
        Vec::new()
    }
}
#[tauri::command]
pub fn unity_native_embed_release(app_handle: AppHandle, generation: String) {
    #[cfg(target_os = "windows")]
    {
        windows_host::release(&app_handle, &generation, true);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app_handle, generation);
    }
}
#[tauri::command]
pub fn unity_native_embed_ready(
    app_handle: AppHandle,
    label: String,
    generation: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_host::ready(&app_handle, &label, &generation)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app_handle, label, generation);
        Err("Unsupported platform".into())
    }
}
