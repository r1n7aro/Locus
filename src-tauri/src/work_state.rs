//! Durable desktop session. Native windows own geometry; renderers own editor state.
//! This file is deliberately separate from the conversation database schema.
use crate::workspace_service::WindowPaneWorkspaceContext;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State, WindowEvent};

const FILE_NAME: &str = "desktop_work_state.json";
/// Changes since the last snapshot. Its header names the snapshot generation it
/// extends, so a log left behind by an interrupted compaction is ignored.
/// Builds without this log still read the schema-1 snapshot.
const OPS_FILE_NAME: &str = "desktop_work_state.ops.jsonl";
const COMPACT_MIN_BYTES: u64 = 512 * 1024;
const SAVE_REQUEST: &str = "locus-work-state-save-requested";

/// Changed frontend fields. Map entries (`editors`, `layouts`, `navigation`)
/// are replaced one by one; null removes an entry. An entry may carry
/// `textDelta` instead of `text` to update a large unsaved buffer in place.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<serde_json::Value>,
    #[serde(default)]
    pub set: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub maps: serde_json::Map<String, serde_json::Value>,
}

impl FrontendPatch {
    fn is_empty(&self) -> bool {
        self.replace.is_none() && self.set.is_empty() && self.maps.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryTextDelta {
    /// `textRevision` of the buffer the renderer computed the delta from.
    base_revision: u64,
    revision: u64,
    from: usize,
    to: usize,
    insert: String,
}

fn apply_entry(
    previous: Option<&serde_json::Value>,
    mut value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    let Some(delta) = object.remove("textDelta") else {
        return Ok(value);
    };
    let delta: EntryTextDelta = serde_json::from_value(delta).map_err(|e| e.to_string())?;
    let base = previous
        .filter(|entry| {
            entry.get("textRevision").and_then(|revision| revision.as_u64())
                == Some(delta.base_revision)
        })
        .and_then(|entry| entry.get("text"))
        .and_then(|text| text.as_str())
        .ok_or("Work state text delta does not match the saved buffer")?;
    let text = crate::text_delta::apply(
        base,
        &crate::text_delta::TextDelta {
            from: delta.from,
            to: delta.to,
            insert: delta.insert,
        },
    )?;
    object.insert("text".into(), serde_json::Value::String(text));
    object.insert("textRevision".into(), serde_json::Value::from(delta.revision));
    Ok(value)
}

/// Applies a patch completely or not at all: every entry, including text
/// deltas, is resolved against the current state before anything changes.
pub fn apply_frontend_patch(
    frontend: &mut serde_json::Value,
    patch: &FrontendPatch,
) -> Result<(), String> {
    if patch.is_empty() {
        return Ok(());
    }
    let base = patch.replace.as_ref().unwrap_or(frontend);
    let mut entries: Vec<(&String, &String, Option<serde_json::Value>)> = Vec::new();
    for (field, changes) in &patch.maps {
        let Some(changes) = changes.as_object() else {
            continue;
        };
        let existing = base.get(field.as_str()).and_then(|map| map.as_object());
        for (key, value) in changes {
            let next = if value.is_null() {
                None
            } else {
                Some(apply_entry(existing.and_then(|map| map.get(key)), value.clone())?)
            };
            entries.push((field, key, next));
        }
    }
    if let Some(replace) = &patch.replace {
        *frontend = replace.clone();
    }
    if !frontend.is_object() {
        *frontend = serde_json::Value::Object(Default::default());
    }
    let object = frontend.as_object_mut().unwrap();
    for (field, value) in &patch.set {
        object.insert(field.clone(), value.clone());
    }
    for (field, key, next) in entries {
        let map = object
            .entry(field.clone())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if !map.is_object() {
            *map = serde_json::Value::Object(Default::default());
        }
        let map = map.as_object_mut().unwrap();
        match next {
            Some(value) => {
                map.insert(key.clone(), value);
            }
            None => {
                map.remove(key);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Op {
    #[serde(rename_all = "camelCase")]
    Header { generation: String },
    #[serde(rename_all = "camelCase")]
    Window {
        label: String,
        owner_label: String,
        bounds: Option<WindowBounds>,
        maximized: bool,
        always_on_top: bool,
        visible: bool,
    },
    #[serde(rename_all = "camelCase")]
    Frontend { label: String, patch: FrontendPatch },
    #[serde(rename_all = "camelCase")]
    Remove { label: String },
    #[serde(rename_all = "camelCase")]
    Meta {
        focused_window: Option<String>,
        legacy_aux_import_pending: bool,
    },
}

impl Op {
    fn window(record: &WorkWindow) -> Self {
        Op::Window {
            label: record.label.clone(),
            owner_label: record.owner_label.clone(),
            bounds: record.bounds.clone(),
            maximized: record.maximized,
            always_on_top: record.always_on_top,
            visible: record.visible,
        }
    }

    fn meta(state: &DesktopWorkState) -> Self {
        Op::Meta {
            focused_window: state.focused_window.clone(),
            legacy_aux_import_pending: state.legacy_aux_import_pending,
        }
    }

    fn apply(self, state: &mut DesktopWorkState) {
        match self {
            Op::Header { .. } => {}
            Op::Window {
                label,
                owner_label,
                bounds,
                maximized,
                always_on_top,
                visible,
            } => {
                let record = state
                    .windows
                    .entry(label.clone())
                    .or_insert_with(|| WorkWindow::new(&label, &owner_label));
                record.owner_label = owner_label;
                record.bounds = bounds;
                record.maximized = maximized;
                record.always_on_top = always_on_top;
                record.visible = visible;
            }
            Op::Frontend { label, patch } => {
                if let Some(record) = state.windows.get_mut(&label) {
                    if let Err(error) = apply_frontend_patch(&mut record.frontend, &patch) {
                        eprintln!("[WorkState] skipped a damaged frontend change: {error}");
                    }
                }
            }
            Op::Remove { label } => state.remove_window(&label),
            Op::Meta {
                focused_window,
                legacy_aux_import_pending,
            } => {
                state.focused_window = focused_window;
                state.legacy_aux_import_pending = legacy_aux_import_pending;
            }
        }
    }
}

struct OpsWriter {
    generation: String,
    log_bytes: u64,
    snapshot_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkWindow {
    pub label: String,
    pub owner_label: String,
    #[serde(default)]
    pub bounds: Option<WindowBounds>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub always_on_top: bool,
    #[serde(default = "default_visible")]
    pub visible: bool,
    #[serde(default)]
    pub frontend: serde_json::Value,
}

fn default_visible() -> bool {
    true
}

impl WorkWindow {
    /// Window identity and geometry without the renderer state.
    fn meta(&self) -> Self {
        Self {
            label: self.label.clone(),
            owner_label: self.owner_label.clone(),
            bounds: self.bounds.clone(),
            maximized: self.maximized,
            always_on_top: self.always_on_top,
            visible: self.visible,
            frontend: serde_json::Value::Null,
        }
    }

    pub fn new(label: &str, owner_label: &str) -> Self {
        Self {
            label: label.into(),
            owner_label: owner_label.into(),
            bounds: None,
            maximized: false,
            always_on_top: false,
            visible: true,
            frontend: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWorkState {
    pub schema_version: u32,
    pub windows: BTreeMap<String, WorkWindow>,
    #[serde(default)]
    pub focused_window: Option<String>,
    #[serde(default)]
    pub legacy_aux_import_pending: bool,
    /// Generation of the change log that extends this snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops_generation: Option<String>,
}

impl DesktopWorkState {
    fn from_legacy_state() -> Self {
        // Legacy pane contexts and layout caches do not track whether their
        // native workspace window was closed. Only the main shell is known to
        // exist on first migration; importing historical workspace IDs here
        // would resurrect closed windows. New sessions restore the explicit
        // window manifest instead. Legacy floating windows have a separate
        // open-window registry, which the main renderer imports once.
        let mut windows = BTreeMap::new();
        windows.insert("main".into(), WorkWindow::new("main", "main"));
        Self {
            schema_version: 1,
            windows,
            focused_window: Some("main".into()),
            legacy_aux_import_pending: true,
            ops_generation: None,
        }
    }

    fn remove_window(&mut self, label: &str) {
        self.windows
            .retain(|id, record| id != label && record.owner_label != label);
        if self
            .focused_window
            .as_ref()
            .is_some_and(|id| !self.windows.contains_key(id))
        {
            self.focused_window = None;
        }
    }
}

struct SaveBarrier {
    id: String,
    pending: BTreeSet<String>,
    notify: Arc<tokio::sync::Notify>,
}

pub struct WorkStateStore {
    path: PathBuf,
    ops_path: PathBuf,
    state: Mutex<DesktopWorkState>,
    /// Changes applied to `state` but not yet appended. Recorded while `state`
    /// is locked, so a snapshot taken under that lock includes exactly them.
    pending_ops: Mutex<Vec<Op>>,
    /// Orders appends and compaction. File IO never holds `state`, which the
    /// main thread's window event handler also takes.
    writer: Mutex<OpsWriter>,
    revision: AtomicU64,
    ready_renderers: Mutex<BTreeSet<String>>,
    closed_windows: Mutex<BTreeSet<String>>,
    geometry_pending: Mutex<BTreeSet<String>>,
    barrier: Mutex<Option<SaveBarrier>>,
    // The original target must not be overwritten by incidental startup focus events.
    restore_focus: Mutex<Option<String>>,
    restoring: Mutex<BTreeSet<String>>,
}

impl WorkStateStore {
    pub fn load(dir: &std::path::Path) -> Result<Arc<Self>, String> {
        let path = dir.join(FILE_NAME);
        let ops_path = dir.join(OPS_FILE_NAME);
        let mut state = if path.exists() {
            let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
            match serde_json::from_slice::<DesktopWorkState>(&raw) {
                Ok(state) if state.schema_version == 1 => state,
                Ok(state) => {
                    return Err(format!(
                        "Unsupported desktop work state version: {}",
                        state.schema_version
                    ))
                }
                Err(error) => {
                    // Keep the damaged source for recovery; never silently overwrite it.
                    let backup = dir.join(format!(
                        "desktop_work_state.invalid-{}.json",
                        uuid::Uuid::new_v4()
                    ));
                    std::fs::rename(&path, &backup).map_err(|e| e.to_string())?;
                    eprintln!(
                        "[WorkState] invalid snapshot preserved at {}: {error}",
                        backup.display()
                    );
                    DesktopWorkState::from_legacy_state()
                }
            }
        } else {
            DesktopWorkState::from_legacy_state()
        };
        // Replay changes recorded after this snapshot. A log from another
        // generation predates or postdates it and is already contained in it.
        if let Some(generation) = state.ops_generation.clone() {
            let log = crate::append_log::read_lines(&ops_path).map_err(|e| e.to_string())?;
            let mut lines = log.lines.iter();
            let header = lines.next().and_then(|line| serde_json::from_str::<Op>(line).ok());
            if matches!(header, Some(Op::Header { generation: ref current }) if *current == generation) {
                for line in lines {
                    match serde_json::from_str::<Op>(line) {
                        Ok(op) => op.apply(&mut state),
                        Err(error) => eprintln!("[WorkState] skipped a damaged change: {error}"),
                    }
                }
            }
        }
        let restore_focus = state.focused_window.clone();
        let restoring = state.windows.keys().cloned().collect();
        let store = Arc::new(Self {
            path,
            ops_path,
            state: Mutex::new(state),
            pending_ops: Mutex::new(Vec::new()),
            writer: Mutex::new(OpsWriter {
                generation: String::new(),
                log_bytes: 0,
                snapshot_bytes: 0,
            }),
            revision: AtomicU64::new(0),
            ready_renderers: Mutex::new(BTreeSet::new()),
            barrier: Mutex::new(None),
            closed_windows: Mutex::new(BTreeSet::new()),
            geometry_pending: Mutex::new(BTreeSet::new()),
            restore_focus: Mutex::new(restore_focus),
            restoring: Mutex::new(restoring),
        });
        store.checkpoint()?;
        Ok(store)
    }

    pub fn snapshot(&self) -> DesktopWorkState {
        let mut state = self.state.lock().unwrap().clone();
        state.ops_generation = None;
        state
    }

    /// Records a change applied to `state`; call while holding that lock.
    fn record(&self, op: Op) {
        self.pending_ops.lock().unwrap().push(op);
    }

    /// Appends the recorded changes; compacts once the log outgrows the snapshot.
    pub fn flush(&self) -> Result<(), String> {
        self.write(false)
    }

    fn write(&self, sync: bool) -> Result<(), String> {
        let mut writer = self.writer.lock().map_err(|e| e.to_string())?;
        let ops = std::mem::take(&mut *self.pending_ops.lock().map_err(|e| e.to_string())?);
        let lines: Vec<String> = ops
            .iter()
            .filter_map(|op| serde_json::to_string(op).ok())
            .collect();
        match crate::append_log::append(&self.ops_path, &lines, sync) {
            Ok(bytes) => writer.log_bytes += bytes,
            Err(error) => {
                // The changes are in memory; a snapshot preserves them instead.
                eprintln!("[WorkState] change log append failed: {error}");
                return self.compact(&mut writer);
            }
        }
        if sync && lines.is_empty() {
            // Earlier background appends were not forced to disk.
            crate::append_log::sync(&self.ops_path)?;
        }
        if writer.log_bytes > COMPACT_MIN_BYTES.max(writer.snapshot_bytes * 2) {
            self.compact(&mut writer)?;
        }
        Ok(())
    }

    /// Writes a full snapshot and starts a new change log (startup and compaction).
    pub fn checkpoint(&self) -> Result<(), String> {
        let mut writer = self.writer.lock().map_err(|e| e.to_string())?;
        self.compact(&mut writer)
    }

    fn compact(&self, writer: &mut OpsWriter) -> Result<(), String> {
        let generation = uuid::Uuid::new_v4().to_string();
        let bytes = {
            let mut state = self.state.lock().map_err(|e| e.to_string())?;
            // Everything recorded so far is part of this snapshot.
            self.pending_ops.lock().map_err(|e| e.to_string())?.clear();
            state.ops_generation = Some(generation.clone());
            serde_json::to_vec(&*state).map_err(|e| e.to_string())?
        };
        crate::config::atomic_write_config(&self.path, &bytes)?;
        let header = serde_json::to_string(&Op::Header {
            generation: generation.clone(),
        })
        .map_err(|e| e.to_string())?;
        writer.log_bytes = crate::append_log::rewrite(&self.ops_path, &[header])?;
        writer.snapshot_bytes = bytes.len() as u64;
        writer.generation = generation;
        Ok(())
    }

    fn schedule(self: &Arc<Self>) {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let store = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if store.revision.load(Ordering::SeqCst) == revision {
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = store.flush() {
                        eprintln!("[WorkState] save failed: {error}");
                    }
                })
                .await;
            }
        });
    }

    pub fn register(&self, label: &str, owner: &str) -> Result<(), String> {
        {
            let mut state = self.state.lock().unwrap();
            if !state.windows.contains_key(label) {
                let record = WorkWindow::new(label, owner);
                self.record(Op::window(&record));
                state.windows.insert(label.into(), record);
            }
        }
        self.flush()
    }

    pub fn forget(&self, label: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let mut closed = self.closed_windows.lock().unwrap();
        closed.extend(
            state
                .windows
                .values()
                .filter(|record| record.label == label || record.owner_label == label)
                .map(|record| record.label.clone()),
        );
        closed.insert(label.into());
        state.remove_window(label);
        self.record(Op::Remove {
            label: label.into(),
        });
        drop(closed);
        drop(state);
        self.flush()
    }

    pub fn retain_context(&self, context: &WindowPaneWorkspaceContext) -> bool {
        let state = self.state.lock().unwrap();
        state.windows.contains_key(&context.window_id)
            || (state.legacy_aux_import_pending && context.window_id.starts_with("workbench-"))
    }

    fn update_geometry(&self, captured: &WorkWindow) {
        let mut state = self.state.lock().unwrap();
        let focus = Op::meta(&state);
        let Some(record) = state.windows.get_mut(&captured.label) else {
            return;
        };
        record.bounds = captured.bounds.clone();
        record.maximized = captured.maximized;
        record.always_on_top = captured.always_on_top;
        let op = Op::window(record);
        self.record(op);
        self.record(focus);
    }
}

fn capture_geometry(window: &tauri::Window, record: &mut WorkWindow) {
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(record.maximized);
    record.always_on_top = window.is_always_on_top().unwrap_or(record.always_on_top);
    record.maximized = maximized;
    // Remember the normal rectangle, not the monitor-sized maximized rectangle.
    if !maximized {
        if let (Ok(position), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.inner_size(),
            window.scale_factor(),
        ) {
            let position = position.to_logical::<f64>(scale);
            let size = size.to_logical::<f64>(scale);
            if size.width >= 200.0 && size.height >= 120.0 {
                record.bounds = Some(WindowBounds {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                });
            }
        }
    }
}

pub fn handle_window_event(window: &tauri::Window, event: &WindowEvent) {
    if crate::commands::app_exit_started() {
        return;
    }
    let Some(store) = window.app_handle().try_state::<Arc<WorkStateStore>>() else {
        return;
    };
    if matches!(event, WindowEvent::Destroyed) {
        // A floating window closed for its opener's page reload is reopened
        // by that page from this record.
        if !crate::shared_workbench_window::keeps_state_on_destroy(window.label()) {
            if let Err(error) = store.forget(window.label()) {
                eprintln!("[WorkState] close save failed: {error}");
            }
        }
        store.ready_renderers.lock().unwrap().remove(window.label());
        store.restoring.lock().unwrap().remove(window.label());
        store.schedule();
        return;
    }
    if !matches!(
        event,
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::Focused(true)
    ) {
        return;
    }
    // Startup resize events must not overwrite the saved normal rectangle.
    if store.restoring.lock().unwrap().contains(window.label()) {
        return;
    }
    {
        let mut state = store.state.lock().unwrap();
        let Some(record) = state.windows.get_mut(window.label()) else {
            return;
        };
        if matches!(event, WindowEvent::Focused(true)) {
            record.visible = true;
            state.focused_window = Some(window.label().into());
        }
    }
    if !store
        .geometry_pending
        .lock()
        .unwrap()
        .insert(window.label().into())
    {
        return;
    }
    let store = store.inner().clone();
    let window = window.clone();
    // WebView2 window operations must run outside its synchronous event/IPC
    // callbacks. Coalesce drag/resize events before sampling the final rectangle.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        store
            .geometry_pending
            .lock()
            .unwrap()
            .remove(window.label());
        if crate::commands::app_exit_started() {
            return;
        }
        let geometry = store
            .state
            .lock()
            .unwrap()
            .windows
            .get(window.label())
            .map(WorkWindow::meta);
        let Some(mut geometry) = geometry else {
            return;
        };
        capture_geometry(&window, &mut geometry);
        // Also records the focus/visibility change made above.
        store.update_geometry(&geometry);
        store.schedule();
    });
}

/// Clamp to an attached monitor, retaining negative coordinates on left-hand displays.
pub fn fit_bounds(bounds: &WindowBounds, screens: &[WindowBounds]) -> WindowBounds {
    let screen = screens
        .iter()
        .find(|screen| {
            bounds.x + bounds.width > screen.x + 40.0
                && bounds.x < screen.x + screen.width - 40.0
                && bounds.y >= screen.y
                && bounds.y < screen.y + screen.height - 40.0
        })
        .or_else(|| screens.first());
    let Some(screen) = screen else {
        return bounds.clone();
    };
    let width = bounds.width.max(200.0).min(screen.width);
    let height = bounds.height.max(120.0).min(screen.height);
    WindowBounds {
        x: bounds.x.clamp(screen.x, screen.x + screen.width - width),
        y: bounds.y.clamp(screen.y, screen.y + screen.height - height),
        width,
        height,
    }
}

/// Applies the saved normal rectangle and pin state to a hidden window.
///
/// The maximized state is left to `reveal_window`. On Windows tao maximizes or
/// restores a hidden window with `ShowWindow(SW_MAXIMIZE | SW_RESTORE)` and
/// then hides it again, and `set_size`/`set_position` restore a maximized
/// window first, so each of those calls flashes the window on screen.
pub fn apply_hidden_geometry(
    window: &tauri::WebviewWindow,
    record: &WorkWindow,
) -> Result<(), String> {
    if window.is_visible().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        return Ok(());
    }
    if let Some(bounds) = record.bounds.as_ref().filter(|b| {
        [b.x, b.y, b.width, b.height].iter().all(|v| v.is_finite())
            && b.width > 0.0
            && b.height > 0.0
    }) {
        let screens = window
            .available_monitors()
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                let p = m.position().to_logical::<f64>(m.scale_factor());
                let s = m.size().to_logical::<f64>(m.scale_factor());
                WindowBounds {
                    x: p.x,
                    y: p.y,
                    width: s.width,
                    height: s.height,
                }
            })
            .collect::<Vec<_>>();
        let bounds = fit_bounds(bounds, &screens);
        window
            .set_size(tauri::LogicalSize::new(bounds.width, bounds.height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::LogicalPosition::new(bounds.x, bounds.y))
            .map_err(|e| e.to_string())?;
    }
    window
        .set_always_on_top(record.always_on_top)
        .map_err(|e| e.to_string())
}

/// Delay before a cloaked window becomes visible, so WebView2 can present a
/// frame at the maximized size first.
#[cfg(target_os = "windows")]
const REVEAL_UNCLOAK_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(target_os = "windows")]
fn set_window_cloaked(hwnd: isize, cloaked: bool) {
    use windows::core::BOOL;
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK},
    };
    let value = BOOL::from(cloaked);
    if let Err(error) = unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as _),
            DWMWA_CLOAK,
            &value as *const BOOL as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        )
    } {
        eprintln!("[WorkState] failed to set window cloak={cloaked}: {error}");
    }
}

/// Shows a hidden window in its saved state as one visible transition.
///
/// Showing and then maximizing goes through the restored rectangle, and tao
/// shows windows created without focus with `SW_SHOWNOACTIVATE`, which also
/// restores a maximized window first. The window stays cloaked while tao
/// shows and maximizes it, so DWM composes only the maximized result.
async fn reveal_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    maximized: bool,
) -> Result<(), String> {
    if !maximized {
        return window.show().map_err(|e| e.to_string());
    }
    #[cfg(target_os = "windows")]
    {
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        let target = window.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            set_window_cloaked(hwnd, true);
            let _ = sender.send(target.show().and_then(|()| target.maximize()));
        })
        .map_err(|e| e.to_string())?;
        let result = receiver.await;
        tokio::time::sleep(REVEAL_UNCLOAK_DELAY).await;
        // Uncloak on every path: a cloaked window is invisible but still active.
        app.run_on_main_thread(move || set_window_cloaked(hwnd, false))
            .map_err(|e| e.to_string())?;
        result
            .map_err(|_| "The window reveal was interrupted".to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        window.show().map_err(|e| e.to_string())?;
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn load_desktop_work_state(
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
) -> DesktopWorkState {
    store
        .ready_renderers
        .lock()
        .unwrap()
        .insert(window.label().into());
    store.snapshot()
}

/// Applies a renderer change. Geometry is sampled only for a newly registered
/// window or when requested (reveal): window events and app exit keep it current
/// otherwise, so ordinary saves do not marshal getters to the UI thread.
fn save_frontend_patch(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    store: &Arc<WorkStateStore>,
    label: &str,
    patch: FrontendPatch,
    geometry: bool,
) -> Result<(), String> {
    let owner = window.label();
    if !crate::workspace_window::is_application_window(owner) {
        return Err("Invalid work state owner".into());
    }
    if label != owner && !label.starts_with("workbench-") {
        return Err("Invalid work state window".into());
    }
    let target = app
        .get_webview_window(label)
        .ok_or("Work state window is closed")?;
    let known = store.state.lock().unwrap().windows.get(label).map(WorkWindow::meta);
    let restoring = store.restoring.lock().unwrap().contains(label);
    // Native getters can marshal to the UI thread. Never hold the session lock
    // while waiting for that thread: its resize/focus callbacks use the same lock.
    let captured = (!restoring && (geometry || known.is_none())).then(|| {
        let mut geometry = known.clone().unwrap_or_else(|| WorkWindow::new(label, owner));
        capture_geometry(&target.as_ref().window(), &mut geometry);
        geometry
    });
    {
        let mut state = store.state.lock().unwrap();
        if store.closed_windows.lock().unwrap().contains(label) {
            return Err("Work state window is closed".into());
        }
        let created = !state.windows.contains_key(label);
        let record = state
            .windows
            .entry(label.to_string())
            .or_insert_with(|| WorkWindow::new(label, owner));
        if record.owner_label != owner {
            return Err("Work state belongs to another renderer".into());
        }
        // A rejected text delta leaves the record unchanged; the renderer then
        // resends the whole patch with full text.
        apply_frontend_patch(&mut record.frontend, &patch)?;
        if let Some(geometry) = &captured {
            record.bounds = geometry.bounds.clone();
            record.maximized = geometry.maximized;
            record.always_on_top = geometry.always_on_top;
        }
        let window_op = (created || captured.is_some()).then(|| Op::window(record));
        if let Some(op) = window_op {
            store.record(op);
        }
        if !patch.is_empty() {
            store.record(Op::Frontend {
                label: label.to_string(),
                patch,
            });
        }
    }
    // Durable within the debounce; app exit writes synchronously after every
    // renderer acknowledged its final save.
    store.schedule();
    Ok(())
}

#[tauri::command]
pub async fn save_window_work_state_patch(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
    label: String,
    patch: FrontendPatch,
    geometry: Option<bool>,
) -> Result<(), String> {
    save_frontend_patch(&app, &window, store.inner(), &label, patch, geometry.unwrap_or(false))
}

/// Whole-frontend save kept for older renderers.
#[tauri::command]
pub async fn save_window_work_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
    label: String,
    frontend: serde_json::Value,
) -> Result<(), String> {
    let patch = FrontendPatch {
        replace: Some(frontend),
        ..FrontendPatch::default()
    };
    save_frontend_patch(&app, &window, store.inner(), &label, patch, true)
}

#[tauri::command]
pub async fn show_work_state_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
    label: String,
) -> Result<(), String> {
    let snapshot = store.snapshot();
    let record = snapshot
        .windows
        .get(&label)
        .ok_or("Work state window is not registered")?;
    if record.owner_label != window.label() {
        return Err("Work state belongs to another renderer".into());
    }
    let target = app
        .get_webview_window(&label)
        .ok_or("Work state window is closed")?;
    if !target.is_visible().unwrap_or(false) {
        apply_hidden_geometry(&target, record)?;
        reveal_window(&app, &target, record.maximized).await?;
        // Windows built hidden and unfocused can receive their WebView2 content
        // HWNDs after the page load; a restored window that is not the focus
        // target would otherwise refuse file drops until it is clicked.
        #[cfg(target_os = "windows")]
        crate::webview_drop_target::ensure_on_main_thread(&target);
    }
    store.restoring.lock().unwrap().remove(&label);
    let focused = store.restore_focus.lock().unwrap().clone();
    if let Some(focused) = focused {
        // Restore the saved focus once every restored window that exists has
        // been revealed. Focusing after each reveal pulls the target forward
        // repeatedly while the other renderers are still showing theirs.
        let pending = store
            .restoring
            .lock()
            .unwrap()
            .iter()
            .any(|id| app.get_webview_window(id).is_some());
        if !pending {
            if let Some(target) = app
                .get_webview_window(&focused)
                .filter(|w| w.is_visible().unwrap_or(false))
            {
                let _ = target.set_focus();
            }
            if store.restoring.lock().unwrap().is_empty() {
                *store.restore_focus.lock().unwrap() = None;
            }
        }
    } else {
        let _ = target.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn finish_work_state_migration(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Ok(());
    }
    {
        let mut state = store.state.lock().unwrap();
        state.legacy_aux_import_pending = false;
        store.record(Op::meta(&state));
    }
    let snapshot = store.snapshot();
    let contexts = app.state::<Arc<crate::workspace_service::WindowContextRegistry>>();
    let persistence = app.state::<Arc<crate::commands::WindowContextPersistence>>();
    let _mutation = persistence.mutation.lock().map_err(|e| e.to_string())?;
    for context in contexts.snapshots().map_err(|e| e.to_string())? {
        if !snapshot.windows.contains_key(&context.window_id)
            && app.get_webview_window(&context.window_id).is_none()
        {
            if let Ok(epoch) = contexts.next_window_intent_epoch(&context.window_id) {
                let _ = contexts.remove_window(&context.window_id, epoch);
            }
        }
    }
    crate::commands::persist_window_context_recovery(&app, &contexts)?;
    store.flush()
}

#[tauri::command]
pub fn acknowledge_work_state_save(
    window: tauri::WebviewWindow,
    store: State<'_, Arc<WorkStateStore>>,
    request_id: String,
) {
    if let Some(barrier) = store
        .barrier
        .lock()
        .unwrap()
        .as_mut()
        .filter(|b| b.id == request_id)
    {
        barrier.pending.remove(window.label());
        barrier.notify.notify_one();
    }
}

pub async fn save_before_exit(app: &tauri::AppHandle) {
    let Some(store) = app.try_state::<Arc<WorkStateStore>>() else {
        return;
    };
    let notify = Arc::new(tokio::sync::Notify::new());
    let id = uuid::Uuid::new_v4().to_string();
    let pending = store
        .ready_renderers
        .lock()
        .unwrap()
        .iter()
        .filter(|id| app.get_webview_window(id).is_some())
        .cloned()
        .collect();
    *store.barrier.lock().unwrap() = Some(SaveBarrier {
        id: id.clone(),
        pending,
        notify: notify.clone(),
    });
    let _ = app.emit(SAVE_REQUEST, &id);
    let wait = async {
        loop {
            if store
                .barrier
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|b| b.pending.is_empty())
            {
                break;
            }
            notify.notified().await;
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(4), wait)
        .await
        .is_err()
    {
        eprintln!("[WorkState] renderer save timed out; retaining latest durable snapshots");
    }
    *store.barrier.lock().unwrap() = None;
    let records: Vec<WorkWindow> = store
        .state
        .lock()
        .unwrap()
        .windows
        .values()
        .map(WorkWindow::meta)
        .collect();
    for mut record in records {
        if let Some(window) = app.get_webview_window(&record.label) {
            capture_geometry(&window.as_ref().window(), &mut record);
            store.update_geometry(&record);
        }
    }
    // One full snapshot at exit: the saved state stays complete on its own, so
    // builds that do not replay the change log restore it too.
    if let Err(error) = store.checkpoint() {
        eprintln!("[WorkState] final save failed: {error}");
        if let Err(error) = store.write(true) {
            eprintln!("[WorkState] final change log save failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_legacy_contexts(dir: &std::path::Path) -> Vec<WindowPaneWorkspaceContext> {
        let contexts: Vec<WindowPaneWorkspaceContext> = [
            ("main", "pane-a"),
            ("main", "pane-b"),
            ("workspace-app-closed-a", "pane-a"),
            ("workspace-app-closed-a", "pane-b"),
            ("workspace-app-closed-b", "pane-a"),
            ("workbench-shared-pool-unused", "pane-a"),
            ("sub-pool-1", "pane-a"),
        ]
        .into_iter()
        .map(|(window, pane)| {
            serde_json::from_value(serde_json::json!({
                "windowId":window,"paneId":pane,"focusedCheckoutId":"checkout-a",
                "workspaceGeneration":1,"materializationEpoch":0,"intentEpoch":1,"revision":1
            }))
            .unwrap()
        })
        .collect();
        std::fs::write(
            dir.join("window_workspace_contexts.json"),
            serde_json::to_vec(&contexts).unwrap(),
        )
        .unwrap();
        crate::commands::load_window_context_recovery(dir)
    }

    #[test]
    fn first_migration_does_not_reopen_historical_workspace_windows() {
        let dir = tempfile::tempdir().unwrap();
        let contexts = write_legacy_contexts(dir.path());
        assert_eq!(contexts.len(), 7);
        let store = WorkStateStore::load(dir.path()).unwrap();
        assert_eq!(
            store.snapshot().windows.keys().cloned().collect::<Vec<_>>(),
            vec!["main"]
        );
        // Keep both main panes, including their checkout/session projection.
        // Auxiliary contexts may wait for their explicit frontend open registry,
        // but must not create windows just because a pane context survived.
        let retained: Vec<_> = contexts
            .iter()
            .filter(|context| store.retain_context(context))
            .map(|context| (context.window_id.as_str(), context.pane_id.as_str()))
            .collect();
        assert_eq!(
            retained,
            vec![
                ("main", "pane-a"),
                ("main", "pane-b"),
                ("workbench-shared-pool-unused", "pane-a")
            ]
        );
        assert!(store.snapshot().windows["main"].frontend.is_null());
        let restarted = WorkStateStore::load(dir.path()).unwrap();
        assert_eq!(
            restarted
                .snapshot()
                .windows
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["main"]
        );
    }

    #[test]
    fn persisted_close_is_not_undone_by_legacy_contexts_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let contexts = write_legacy_contexts(dir.path());
        let store = WorkStateStore::load(dir.path()).unwrap();
        store
            .register("workspace-app-closed-a", "workspace-app-closed-a")
            .unwrap();
        store
            .register("workbench-a", "workspace-app-closed-a")
            .unwrap();
        store.forget("workspace-app-closed-a").unwrap();
        let restarted = WorkStateStore::load(dir.path()).unwrap();
        assert_eq!(
            restarted
                .snapshot()
                .windows
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["main"]
        );
        assert!(!restarted.retain_context(&contexts[2]));
    }
    #[test]
    fn session_round_trip_preserves_identity_layout_owner_and_normal_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkStateStore::load(dir.path()).unwrap();
        store
            .register("workspace-app-a", "workspace-app-a")
            .unwrap();
        store
            .register("workbench-shared-pool-a", "workspace-app-a")
            .unwrap();
        {
            let mut state = store.state.lock().unwrap();
            let window = state.windows.get_mut("workspace-app-a").unwrap();
            window.bounds = Some(WindowBounds {
                x: -1400.,
                y: 30.,
                width: 1100.,
                height: 700.,
            });
            window.maximized = true;
            window.frontend = serde_json::json!({"activeWorkspaceId":"checkout-a", "layouts":{"checkout-a":{"focusedPaneId":"pane-b"}}});
            state.focused_window = Some("workbench-shared-pool-a".into());
        }
        // Direct state edits bypass the change log; a snapshot persists them.
        store.checkpoint().unwrap();
        let contexts = write_legacy_contexts(dir.path());
        let restored_store = WorkStateStore::load(dir.path()).unwrap();
        assert!(!restored_store.retain_context(&contexts[2]));
        let restored = restored_store.snapshot();
        assert_eq!(restored.windows.len(), 3);
        assert_eq!(
            restored.windows["workbench-shared-pool-a"].owner_label,
            "workspace-app-a"
        );
        assert!(restored.windows["workspace-app-a"].maximized);
        assert_eq!(
            restored.windows["workspace-app-a"]
                .bounds
                .as_ref()
                .unwrap()
                .x,
            -1400.
        );
        assert_eq!(
            restored.windows["workspace-app-a"].frontend["layouts"]["checkout-a"]["focusedPaneId"],
            "pane-b"
        );
    }

    fn apply_patch(store: &WorkStateStore, label: &str, patch: serde_json::Value) {
        let patch: FrontendPatch = serde_json::from_value(patch).unwrap();
        let mut state = store.state.lock().unwrap();
        let record = state
            .windows
            .entry(label.into())
            .or_insert_with(|| WorkWindow::new(label, label));
        apply_frontend_patch(&mut record.frontend, &patch).unwrap();
        store.record(Op::Frontend {
            label: label.into(),
            patch,
        });
    }

    #[test]
    fn changes_append_only_changed_entries_and_replay_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let ops = dir.path().join(OPS_FILE_NAME);
        let store = WorkStateStore::load(dir.path()).unwrap();
        let buffer = "unsaved line\n".repeat(20_000);
        apply_patch(&store, "main", serde_json::json!({ "replace": {
            "version": 1, "layouts": {}, "editors": {
                "a": { "kind": "workspaceFile", "dirty": true, "text": buffer, "textRevision": 7 },
                "b": { "kind": "session" },
            } } }));
        store.flush().unwrap();
        let before = std::fs::metadata(&ops).unwrap().len();
        apply_patch(&store, "main", serde_json::json!({
            "set": { "activePage": "settings" },
            "maps": { "editors": {
                "a": { "kind": "workspaceFile", "dirty": true, "textDelta": {
                    "baseRevision": 7, "revision": 8,
                    "from": buffer.encode_utf16().count(), "to": buffer.encode_utf16().count(), "insert": "!" } },
                "b": null,
            } } }));
        store.flush().unwrap();
        assert!(std::fs::metadata(&ops).unwrap().len() - before < 600);
        let restarted = WorkStateStore::load(dir.path()).unwrap().snapshot();
        let frontend = &restarted.windows["main"].frontend;
        assert_eq!(frontend["activePage"], "settings");
        assert_eq!(frontend["editors"]["a"]["text"].as_str().unwrap(), format!("{buffer}!"));
        assert_eq!(frontend["editors"]["a"]["textRevision"], 8);
        assert!(frontend["editors"].get("b").is_none());
        assert!(frontend["editors"]["a"].get("textDelta").is_none());
    }

    #[test]
    fn a_change_log_from_another_generation_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkStateStore::load(dir.path()).unwrap();
        apply_patch(&store, "main", serde_json::json!({ "set": { "activePage": "kept" } }));
        store.checkpoint().unwrap();
        // A compaction interrupted after its snapshot leaves the previous log behind.
        let stale = [
            serde_json::to_string(&Op::Header { generation: "older".into() }).unwrap(),
            serde_json::to_string(&Op::Frontend {
                label: "main".into(),
                patch: serde_json::from_value(serde_json::json!({ "set": { "activePage": "stale" } })).unwrap(),
            })
            .unwrap(),
        ];
        crate::append_log::rewrite(&dir.path().join(OPS_FILE_NAME), &stale).unwrap();
        let restarted = WorkStateStore::load(dir.path()).unwrap().snapshot();
        assert_eq!(restarted.windows["main"].frontend["activePage"], "kept");
    }

    #[test]
    fn the_log_is_compacted_once_it_outgrows_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let ops = dir.path().join(OPS_FILE_NAME);
        let store = WorkStateStore::load(dir.path()).unwrap();
        let entry = "x".repeat(16 * 1024);
        for i in 0..40 {
            apply_patch(&store, "main", serde_json::json!({
                "maps": { "editors": { "a": { "kind": "session", "draft": format!("{entry}{i}") } } } }));
            store.flush().unwrap();
        }
        assert!(std::fs::metadata(&ops).unwrap().len() < COMPACT_MIN_BYTES);
        let restarted = WorkStateStore::load(dir.path()).unwrap().snapshot();
        assert_eq!(
            restarted.windows["main"].frontend["editors"]["a"]["draft"].as_str().unwrap(),
            format!("{entry}39")
        );
    }

    #[test]
    fn text_deltas_against_another_buffer_are_rejected() {
        let mut frontend = serde_json::json!({ "activePage": "chat",
            "editors": { "a": { "text": "disk", "textRevision": 3 }, "b": { "kind": "session" } } });
        let before = frontend.clone();
        // Entries before and after the stale delta must not be applied either.
        let patch: FrontendPatch = serde_json::from_value(serde_json::json!({
            "set": { "activePage": "settings" },
            "maps": { "editors": {
                "0": { "kind": "session" },
                "a": { "textDelta": { "baseRevision": 2, "revision": 4, "from": 0, "to": 0, "insert": "x" } },
                "b": null,
            } } }))
        .unwrap();
        assert!(apply_frontend_patch(&mut frontend, &patch).is_err());
        assert_eq!(frontend, before);
    }

    #[test]
    fn interactive_close_removes_owned_windows_but_keeps_other_workspaces() {
        let mut state = DesktopWorkState::from_legacy_state();
        for (id, owner) in [
            ("workspace-app-a", "workspace-app-a"),
            ("workspace-app-b", "workspace-app-b"),
            ("workbench-a", "workspace-app-a"),
        ] {
            state.windows.insert(id.into(), WorkWindow::new(id, owner));
        }
        state.focused_window = Some("workbench-a".into());
        state.remove_window("workspace-app-a");
        assert_eq!(
            state.windows.keys().cloned().collect::<Vec<_>>(),
            vec!["main", "workspace-app-b"]
        );
        assert!(state.focused_window.is_none());
    }

    #[test]
    fn disconnected_monitor_is_clamped_but_valid_negative_coordinates_survive() {
        let primary = WindowBounds {
            x: 0.,
            y: 0.,
            width: 1920.,
            height: 1080.,
        };
        let left = WindowBounds {
            x: -1920.,
            ..primary.clone()
        };
        let saved = WindowBounds {
            x: -1700.,
            y: 70.,
            width: 1000.,
            height: 700.,
        };
        assert_eq!(fit_bounds(&saved, &[primary.clone(), left]), saved);
        assert_eq!(fit_bounds(&saved, &[primary]).x, 0.);
    }

    #[test]
    fn unsupported_version_is_not_overwritten_and_corruption_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, r#"{"schemaVersion":2,"windows":{}}"#).unwrap();
        assert!(WorkStateStore::load(dir.path()).is_err());
        assert!(std::fs::read_to_string(&path).unwrap().contains(":2"));
        std::fs::write(&path, "broken").unwrap();
        let contexts = write_legacy_contexts(dir.path());
        let recovered = WorkStateStore::load(dir.path()).unwrap();
        assert!(!recovered.retain_context(&contexts[2]));
        assert_eq!(
            recovered
                .snapshot()
                .windows
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["main"]
        );
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("desktop_work_state.invalid-")
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "broken");
    }
}
