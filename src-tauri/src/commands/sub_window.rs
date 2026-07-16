//! Generic pooled sub-window management for the lightweight `window.html`
//! entry. Mirrors the View host pool in `view.rs`: one hidden pre-warmed
//! window waits off-screen, gets claimed (configured + assigned a window
//! kind) on open, and is replenished right after. Windows reveal themselves
//! from the frontend once their shell has painted, so the user never sees
//! the white WebView2 init phase or the bundle load.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use crate::error::AppError;

const MAIN_WINDOW_LABEL: &str = "main";
const SUB_WINDOW_POOL_LABEL_PREFIX: &str = "sub-pool-";
const SUB_WINDOW_POOL_ROUTE: &str = "/window.html?subWindowPool=1";
const SUB_WINDOW_ROUTE_PREFIX: &str = "/window.html?";
pub const SUB_WINDOW_ASSIGN_EVENT: &str = "sub-window:assign";
const DEFAULT_BACKGROUND_COLOR: &str = "#1d1d21";

#[derive(Default)]
struct SubWindowPoolState {
    next_index: u64,
    /// Pool window built and waiting for its frontend to signal readiness.
    pending_label: Option<String>,
    /// Pool window whose frontend registered the assign listener.
    available_label: Option<String>,
    /// kind -> live window label (pool-claimed or directly created).
    claimed: HashMap<String, String>,
    /// kind -> query of the latest open request. Components pull this after
    /// registering their payload listener: a payload event emitted for an
    /// `existing` window can fire before the (async-loaded) component
    /// listens, and would otherwise be lost.
    claimed_queries: HashMap<String, String>,
    /// Last theme background color reported by the frontend; used for
    /// pool windows created before any open request carries a color.
    background_color: Option<String>,
}

fn sub_window_pool_state() -> &'static Mutex<SubWindowPoolState> {
    static STATE: OnceLock<Mutex<SubWindowPoolState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(SubWindowPoolState::default()))
}

fn is_sub_window_pool_label(label: &str) -> bool {
    label.starts_with(SUB_WINDOW_POOL_LABEL_PREFIX)
}

fn parse_background_color(value: &str) -> Option<tauri::webview::Color> {
    let hex = value.trim().strip_prefix('#')?;
    let parse = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    match hex.len() {
        6 => Some(tauri::webview::Color(
            parse(0..2)?,
            parse(2..4)?,
            parse(4..6)?,
            0xff,
        )),
        8 => Some(tauri::webview::Color(
            parse(0..2)?,
            parse(2..4)?,
            parse(4..6)?,
            parse(6..8)?,
        )),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubWindowOpenRequest {
    /// Stable window kind, doubles as the label for directly created
    /// windows (e.g. "plan-view").
    pub kind: String,
    /// Query string for `window.html` including the legacy location flag
    /// (e.g. "planView=1&planFilePath=..."), without a leading `?`.
    pub query: String,
    pub title: String,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub min_width: Option<f64>,
    #[serde(default)]
    pub min_height: Option<f64>,
    #[serde(default = "default_true")]
    pub resizable: bool,
    #[serde(default = "default_true")]
    pub maximizable: bool,
    #[serde(default)]
    pub minimizable: bool,
    #[serde(default = "default_true")]
    pub closable: bool,
    /// Whether hitting an already-open window of this kind grabs focus;
    /// quiet progress windows opt out to avoid stealing the foreground.
    #[serde(default = "default_true")]
    pub focus_existing: bool,
    #[serde(default)]
    pub background_color: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubWindowOpenResult {
    pub label: String,
    /// True when a live window of this kind already existed; the caller
    /// re-delivers its payload event instead of a fresh assignment.
    pub existing: bool,
    pub pooled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubWindowAssignPayload {
    kind: String,
    query: String,
}

fn normalize_sub_window_kind(kind: &str) -> Result<String, String> {
    let normalized = kind.trim();
    if normalized.is_empty() {
        return Err("Sub-window kind must not be empty".to_string());
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("Invalid sub-window kind: {normalized}"));
    }
    Ok(normalized.to_string())
}

fn remember_background_color(color: &Option<String>) {
    let Some(color) = color.as_deref().map(str::trim) else {
        return;
    };
    if parse_background_color(color).is_none() {
        return;
    }
    if let Ok(mut state) = sub_window_pool_state().lock() {
        state.background_color = Some(color.to_string());
    }
}

fn pool_background_color() -> String {
    sub_window_pool_state()
        .lock()
        .ok()
        .and_then(|state| state.background_color.clone())
        .unwrap_or_else(|| DEFAULT_BACKGROUND_COLOR.to_string())
}

fn build_sub_window(
    app_handle: &AppHandle,
    label: &str,
    url: &str,
    request: Option<&SubWindowOpenRequest>,
) -> Result<(), String> {
    let color = request
        .and_then(|request| request.background_color.clone())
        .unwrap_or_else(pool_background_color);
    let mut builder = tauri::WebviewWindowBuilder::new(
        app_handle,
        label,
        tauri::WebviewUrl::App(url.to_string().into()),
    )
    .title(
        request
            .map(|request| request.title.clone())
            .unwrap_or_else(|| "Locus".to_string()),
    )
    .decorations(false)
    .shadow(true)
    .resizable(request.map(|request| request.resizable).unwrap_or(true))
    .maximizable(request.map(|request| request.maximizable).unwrap_or(true))
    .minimizable(request.map(|request| request.minimizable).unwrap_or(false))
    .closable(request.map(|request| request.closable).unwrap_or(true))
    .visible(false);
    if let Some(color) = parse_background_color(&color) {
        builder = builder.background_color(color);
    }
    builder = match request {
        Some(request) => {
            let mut sized = builder.inner_size(request.width, request.height).center();
            if let (Some(min_width), Some(min_height)) = (request.min_width, request.min_height) {
                sized = sized.min_inner_size(min_width, min_height);
            }
            sized
        }
        // Pool windows park off-screen until a claim positions them.
        None => builder.inner_size(920.0, 720.0).position(-32000.0, -32000.0),
    };
    if let Some(main_window) = app_handle.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder
            .parent(&main_window)
            .map_err(|error| format!("Failed to attach sub-window to main window: {error}"))?;
    }
    builder
        .build()
        .map(|_| ())
        .map_err(|error| format!("Failed to open sub-window: {error}"))
}

pub fn ensure_sub_window_pool_window(app_handle: &AppHandle) -> Result<(), String> {
    {
        let mut state = sub_window_pool_state()
            .lock()
            .map_err(|_| "Sub-window pool state is unavailable".to_string())?;
        if let Some(label) = state.available_label.clone() {
            if app_handle.get_webview_window(&label).is_some() {
                return Ok(());
            }
            state.available_label = None;
        }
        if let Some(label) = state.pending_label.clone() {
            if app_handle.get_webview_window(&label).is_some() {
                return Ok(());
            }
            state.pending_label = None;
        }
    }

    let label = {
        let mut state = sub_window_pool_state()
            .lock()
            .map_err(|_| "Sub-window pool state is unavailable".to_string())?;
        state.next_index = state.next_index.saturating_add(1);
        let label = format!("{}{}", SUB_WINDOW_POOL_LABEL_PREFIX, state.next_index);
        state.pending_label = Some(label.clone());
        label
    };

    let result = build_sub_window(app_handle, &label, SUB_WINDOW_POOL_ROUTE, None);
    if let Err(error) = result {
        if let Ok(mut state) = sub_window_pool_state().lock() {
            if state.pending_label.as_deref() == Some(&label) {
                state.pending_label = None;
            }
        }
        return Err(error);
    }
    Ok(())
}

pub fn mark_sub_window_pool_ready(app_handle: &AppHandle, label: &str) -> Result<(), String> {
    if !is_sub_window_pool_label(label) {
        return Err(format!("Window is not a sub-window pool window: {label}"));
    }
    if app_handle.get_webview_window(label).is_none() {
        return Err(format!("Sub-window pool window is not open: {label}"));
    }
    let mut state = sub_window_pool_state()
        .lock()
        .map_err(|_| "Sub-window pool state is unavailable".to_string())?;
    if state.available_label.as_deref() == Some(label) {
        return Ok(());
    }
    if state.pending_label.as_deref() != Some(label) {
        return Ok(());
    }
    state.pending_label = None;
    state.available_label = Some(label.to_string());
    Ok(())
}

fn take_sub_window_pool_window(app_handle: &AppHandle) -> Option<String> {
    let label = sub_window_pool_state()
        .lock()
        .ok()
        .and_then(|mut state| state.available_label.take())?;
    if app_handle.get_webview_window(&label).is_some() {
        return Some(label);
    }
    None
}

fn configure_claimed_sub_window(
    window: &tauri::WebviewWindow,
    request: &SubWindowOpenRequest,
) -> Result<(), String> {
    let apply = |step: &str, result: tauri::Result<()>| {
        result.map_err(|error| format!("Failed to {step} claimed sub-window: {error}"))
    };
    apply("retitle", window.set_title(&request.title))?;
    apply(
        "resize",
        window.set_size(tauri::LogicalSize::new(request.width, request.height)),
    )?;
    if let (Some(min_width), Some(min_height)) = (request.min_width, request.min_height) {
        apply(
            "constrain",
            window.set_min_size(Some(tauri::LogicalSize::new(min_width, min_height))),
        )?;
    }
    apply("center", window.center())?;
    // Best-effort flag sync; claimed windows share the pool default otherwise.
    let _ = window.set_resizable(request.resizable);
    let _ = window.set_maximizable(request.maximizable);
    let _ = window.set_minimizable(request.minimizable);
    let _ = window.set_closable(request.closable);
    Ok(())
}

fn open_sub_window_inner(
    app_handle: &AppHandle,
    request: SubWindowOpenRequest,
) -> Result<SubWindowOpenResult, String> {
    let kind = normalize_sub_window_kind(&request.kind)?;
    remember_background_color(&request.background_color);

    {
        let mut state = sub_window_pool_state()
            .lock()
            .map_err(|_| "Sub-window pool state is unavailable".to_string())?;
        if let Some(label) = state.claimed.get(&kind).cloned() {
            if let Some(window) = app_handle.get_webview_window(&label) {
                if request.focus_existing {
                    let _ = window.set_focus();
                }
                state
                    .claimed_queries
                    .insert(kind.clone(), request.query.clone());
                return Ok(SubWindowOpenResult {
                    label,
                    existing: true,
                    pooled: false,
                });
            }
            state.claimed.remove(&kind);
            state.claimed_queries.remove(&kind);
        }
    }

    if let Some(label) = take_sub_window_pool_window(app_handle) {
        let window = app_handle
            .get_webview_window(&label)
            .ok_or_else(|| format!("Claimed sub-window pool window vanished: {label}"))?;
        configure_claimed_sub_window(&window, &request)?;
        if let Ok(mut state) = sub_window_pool_state().lock() {
            state.claimed.insert(kind.clone(), label.clone());
            state
                .claimed_queries
                .insert(kind.clone(), request.query.clone());
        }
        app_handle
            .emit_to(
                &label,
                SUB_WINDOW_ASSIGN_EVENT,
                SubWindowAssignPayload {
                    kind: kind.clone(),
                    query: request.query.clone(),
                },
            )
            .map_err(|error| format!("Failed to assign sub-window pool window: {error}"))?;
        if let Err(error) = ensure_sub_window_pool_window(app_handle) {
            eprintln!("[Locus SubWindowPool] replenish failed: {error}");
        }
        return Ok(SubWindowOpenResult {
            label,
            existing: false,
            pooled: true,
        });
    }

    let label = kind.clone();
    let url = format!("{}{}", SUB_WINDOW_ROUTE_PREFIX, request.query);
    build_sub_window(app_handle, &label, &url, Some(&request))?;
    if let Ok(mut state) = sub_window_pool_state().lock() {
        state.claimed_queries.insert(kind.clone(), request.query.clone());
        state.claimed.insert(kind, label.clone());
    }
    if let Err(error) = ensure_sub_window_pool_window(app_handle) {
        eprintln!("[Locus SubWindowPool] prepare after direct open failed: {error}");
    }
    Ok(SubWindowOpenResult {
        label,
        existing: false,
        pooled: false,
    })
}

/// Resolve a live window by sub-window kind: pool-claimed windows carry
/// pool labels, so fixed-label lookups must go through the claimed map.
/// Falls back to treating the kind as a literal label for windows opened
/// via legacy code paths.
pub fn find_sub_window(app_handle: &AppHandle, kind: &str) -> Option<tauri::WebviewWindow> {
    let claimed_label = sub_window_pool_state()
        .lock()
        .ok()
        .and_then(|state| state.claimed.get(kind).cloned());
    if let Some(label) = claimed_label {
        if let Some(window) = app_handle.get_webview_window(&label) {
            return Some(window);
        }
    }
    app_handle.get_webview_window(kind)
}

pub fn handle_sub_window_event(window: &tauri::Window, event: &WindowEvent) {
    if !matches!(event, WindowEvent::Destroyed) {
        return;
    }
    let label = window.label().to_string();
    let Ok(mut state) = sub_window_pool_state().lock() else {
        return;
    };
    if state.available_label.as_deref() == Some(&label) {
        state.available_label = None;
    }
    if state.pending_label.as_deref() == Some(&label) {
        state.pending_label = None;
    }
    let mut closed_kinds: Vec<String> = Vec::new();
    state.claimed.retain(|kind, claimed_label| {
        if claimed_label == &label {
            closed_kinds.push(kind.clone());
            false
        } else {
            true
        }
    });
    for kind in closed_kinds {
        state.claimed_queries.remove(&kind);
    }
}

#[tauri::command]
pub async fn sub_window_open(
    request: SubWindowOpenRequest,
    app_handle: AppHandle,
) -> Result<SubWindowOpenResult, AppError> {
    open_sub_window_inner(&app_handle, request).map_err(Into::into)
}

#[tauri::command]
pub async fn sub_window_pool_prepare(
    background_color: Option<String>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    remember_background_color(&background_color);
    ensure_sub_window_pool_window(&app_handle).map_err(Into::into)
}

#[tauri::command]
pub async fn sub_window_pool_ready(
    label: String,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    mark_sub_window_pool_ready(&app_handle, &label).map_err(Into::into)
}

/// Query of the latest open request for a live window kind. Lets a window
/// component pull the payload it may have missed while its event listener
/// was still registering (open -> immediate re-open of the same kind).
#[tauri::command]
pub async fn sub_window_claimed_query(kind: String) -> Result<Option<String>, AppError> {
    let kind = normalize_sub_window_kind(&kind).map_err(AppError::from)?;
    Ok(sub_window_pool_state()
        .lock()
        .map(|state| state.claimed_queries.get(&kind).cloned())
        .unwrap_or(None))
}
