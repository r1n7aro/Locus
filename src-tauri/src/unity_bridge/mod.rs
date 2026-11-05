mod background_hook;
mod capture;
pub(crate) mod dialog;
mod editor_log;
mod flavor;
mod focus;
pub(crate) mod hang_diagnosis;
pub(crate) mod managed_editor;
mod native_selftest;
mod plugin;
mod process;
mod process_markers;
pub(crate) use process_markers::{cleanup_closed_project_markers, remove_closed_editor_marker};
mod state_probe;
mod transport;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tokio::sync::Mutex;

use flavor::EditorFlavor;

pub use background_hook::{UnityBackgroundHookState, UnityBackgroundHookStatus};
pub use capture::{capture_viewport, UnityViewportCapture};
pub use editor_log::{
    read_console_entries as read_editor_log_console_entries, resolve_editor_log_path,
    EditorLogEntry, EditorLogRead, ObservedEditorLogState,
};
pub(crate) use focus::restore_foreground;
pub use plugin::{
    check_plugin_install_plan, check_plugin_status, emit_plugin_status_scoped,
    find_plugin_source_dir, install_or_update_plugin, install_or_update_plugin_with_force_close,
    plugin_install_root, plugin_skills_root, PluginInstallPlan, PluginStatus,
};
pub(crate) use process::UnityProcessIdentityLiveness;
pub(crate) use process::query_current_project_editor_process_uncached;
pub(crate) use process::force_close_editor_identity;
pub use process::{
    close_current_project_unity_processes, force_close_current_project_unity_processes,
    query_current_project_editor_process, UnityEditorProcessInfo, UnityEditorProcessState,
};
pub use state_probe::{
    ObservedMainThreadState, ObservedSafetyState, SemanticState, UnityStateProbeStatus,
    UnityStateProbeTier,
};
pub use transport::{
    disconnect_with_reason, send_message, send_message_with_timeout, send_message_without_timeout,
    send_message_without_timeout_with_acceptance, set_event_app_handle, set_service_event_scope,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityWorkspaceStatus<T> {
    pub checkout_id: String,
    pub workspace_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_generation: Option<u64>,
    #[serde(flatten)]
    pub status: T,
}

impl<T> UnityWorkspaceStatus<T> {
    pub(crate) fn from_scope(
        scope: &crate::workspace_service::event::WorkspaceEventScope,
        status: T,
    ) -> Self {
        Self {
            checkout_id: scope.checkout_id.to_string(),
            workspace_generation: scope.workspace_generation,
            service_instance_id: scope.service_instance_id.as_ref().map(ToString::to_string),
            service_generation: scope.service_generation,
            status,
        }
    }
}

pub fn bind_workspace_observable_status(
    project_path: &str,
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) {
    background_hook::bind_workspace_scope(project_path, scope);
    state_probe::bind_workspace_scope(project_path, scope);
}

pub fn unbind_workspace_observable_status(
    project_path: &str,
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) {
    background_hook::unbind_workspace_scope(project_path, scope);
    state_probe::unbind_workspace_scope(project_path, scope);
}

pub fn initialize_background_hook(enabled: bool) {
    background_hook::initialize(enabled);
}

pub fn set_background_hook_enabled(value: bool) -> Result<UnityBackgroundHookStatus, String> {
    background_hook::set_enabled(value)
}

pub fn background_hook_status_for_scope(
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) -> UnityWorkspaceStatus<UnityBackgroundHookStatus> {
    UnityWorkspaceStatus::from_scope(scope, background_hook::status_for_scope(scope))
}

pub fn restore_background_hook_runtime() -> Result<(), String> {
    background_hook::restore_runtime_patches()
}

pub fn initialize_state_probe(enabled: bool) {
    state_probe::initialize(enabled);
}

pub fn set_state_probe_enabled(value: bool) -> UnityStateProbeStatus {
    state_probe::set_enabled(value)
}

pub fn state_probe_status_for_scope(
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) -> UnityWorkspaceStatus<UnityStateProbeStatus> {
    UnityWorkspaceStatus::from_scope(scope, state_probe::status_for_scope(scope))
}

pub fn start_unity_semantic_state_observer(project_path: &str) {
    state_probe::start_observer(project_path);
}

pub fn stop_unity_semantic_state_observers() {
    state_probe::stop_all_observers();
}

/// Fuse pipe + process + native signals into one semantic editor state.
pub async fn unity_semantic_state(project_path: &str) -> SemanticState {
    let mut state = state_probe::semantic_state_for_project(project_path).await;
    let _ = dialog::ensure_project_observed(project_path).await;
    fold_semantic_overlays(project_path, &mut state);
    state
}

/// Workspace event carrying the full [`SemanticState`] whenever it changes.
pub const UNITY_SEMANTIC_STATE_EVENT: &str = "unity-semantic-state";

/// App-wide event: an observed project's editor process started or exited.
/// Lets resource views refresh instead of polling the process table.
pub const UNITY_EDITOR_PROCESSES_CHANGED_EVENT: &str = "unity-editor-processes-changed";

pub(super) fn note_editor_processes_changed() {
    let _ = transport::emit_app_event(UNITY_EDITOR_PROCESSES_CHANGED_EVENT, ());
}

/// Pushes [`UNITY_SEMANTIC_STATE_EVENT`] from the observer loop. Fields that
/// change on every sample without a state change (ages, latencies, check and
/// log timestamps, observer bookkeeping) do not count as a change.
#[derive(Default)]
pub(super) struct SemanticStatePublisher {
    last_signature: Option<String>,
}

impl SemanticStatePublisher {
    pub(super) fn publish(&mut self, project_path: &str, mut state: SemanticState) {
        fold_semantic_overlays(project_path, &mut state);
        let signature = semantic_state_signature(&state);
        if self.last_signature.as_deref() == Some(signature.as_str()) {
            return;
        }
        // Without a live service scope nothing was delivered; retry next sample.
        if transport::emit_project_event(project_path, UNITY_SEMANTIC_STATE_EVENT, state) {
            self.last_signature = Some(signature);
        }
    }
}

fn semantic_state_signature(state: &SemanticState) -> String {
    const VOLATILE_KEYS: &[&str] = &[
        "checkedAtMs",
        "lastLatencyMs",
        "staleMs",
        "heartbeatAgeMs",
        "quiescentForMs",
        "suspendWindowUs",
        "modifiedAtMs",
        "statePlane",
    ];
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for key in VOLATILE_KEYS {
                    map.remove(*key);
                }
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(state).unwrap_or_default();
    strip(&mut value);
    value.to_string()
}

/// Signals observed outside the fusion: the hang capture and native dialogs.
fn fold_semantic_overlays(project_path: &str, state: &mut SemanticState) {
    // The cached observer state may predate the capture finishing.
    hang_diagnosis::attach(project_path, state);
    // A native Unity dialog can keep the status/ping channel responsive while
    // the managed editor main thread is unavailable for normal API work. Fold
    // that independent window signal into the semantic surface so callers do
    // not mistake "pipe ready" for "Unity API ready".
    if let Some(dialog) = dialog::current_dialog(project_path) {
        // Publish observable blocking and recovery capabilities. The Agent
        // decides whether to choose a dialog action, wait, or restart Unity.
        state.transient = false;
        state.main_thread.state = "blocked".to_string();
        state.main_thread.cpu_active = false;
        state.safety.can_call_unity_api = false;
        state.safety.can_modify_assets_safely = false;
        state.safety.recommended_action = "resolve_dialog".to_string();
        state.detail = Some(if dialog.title.trim().is_empty() {
            "Unity main thread is blocked by a modal dialog".to_string()
        } else {
            format!("Unity main thread is blocked by dialog: {}", dialog.title)
        });
    }
}

pub async fn enrich_unity_tool_error(project_path: &str, original: &str) -> String {
    const MARKER: &str = "[Unity editor diagnostics]";
    if original.contains(MARKER) || project_path.trim().is_empty() {
        return original.to_string();
    }

    let state = unity_semantic_state(project_path).await;
    if !matches!(state.phase.as_str(), "safe_mode" | "crashed") {
        return original.to_string();
    }

    let log_path = state
        .editor_log
        .path
        .clone()
        .or_else(|| {
            editor_log::resolve_editor_log_path(project_path, state.process.pid)
                .map(|(path, _)| path.display().to_string())
        })
        .unwrap_or_else(|| "unavailable".to_string());
    let action = if state.phase == "safe_mode" {
        "Inspect the compiler errors below or call unity_get_console_log with level='error', fix the referenced source files with file tools, then wait for Unity to exit Safe Mode automatically."
    } else {
        "Inspect the Editor log around the crash before restarting Unity; identify the native exception, managed stack, or last executing tool operation that caused the exit."
    };
    let recent_errors = editor_log::recent_error_lines(project_path, state.process.pid, 8);
    let mut output = format!(
        "{}\n\n{}\n- state: {}\n- editor_log: {}\n- action: {}",
        original.trim_end(),
        MARKER,
        state.phase,
        log_path,
        action
    );
    if !recent_errors.is_empty() {
        output.push_str("\n- recent_errors:");
        for error in recent_errors {
            output.push_str("\n  - ");
            output.push_str(&crate::tool::output::flat_text(&error));
        }
    }
    output
}

pub async fn run_state_probe_selftest(
    app: tauri::AppHandle,
    project: String,
) -> Result<(), String> {
    state_probe::selftest::run(app, project).await
}

pub async fn run_state_probe_selftest_scoped(
    app: tauri::AppHandle,
    project: String,
    event_scope: crate::workspace_service::event::WorkspaceEventScope,
) -> Result<(), String> {
    state_probe::selftest::run_scoped(app, project, event_scope).await
}

pub async fn run_native_bridge_selftest(
    app: tauri::AppHandle,
    project: String,
) -> Result<(), String> {
    native_selftest::run(app, project).await
}

pub async fn run_native_bridge_selftest_scoped(
    app: tauri::AppHandle,
    project: String,
    event_scope: crate::workspace_service::event::WorkspaceEventScope,
) -> Result<(), String> {
    native_selftest::run_scoped(app, project, event_scope).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalScriptOpenRequest {
    pub project_path: String,
    pub asset_path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Default)]
pub struct PendingExternalScriptOpenRequest(Arc<StdMutex<Option<ExternalScriptOpenRequest>>>);

impl PendingExternalScriptOpenRequest {
    pub fn new(request: Option<ExternalScriptOpenRequest>) -> Self {
        Self(Arc::new(StdMutex::new(request)))
    }

    pub fn take(&self) -> Option<ExternalScriptOpenRequest> {
        self.0.lock().ok()?.take()
    }

    pub fn stage(&self, request: ExternalScriptOpenRequest) {
        if let Ok(mut pending) = self.0.lock() {
            *pending = Some(request);
        }
    }
}

fn command_line_option(args: &[String], name: &str) -> Option<String> {
    let inline_prefix = format!("{name}=");
    for (index, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(&inline_prefix) {
            return Some(value.to_string());
        }
        if arg == name {
            return args.get(index + 1).cloned();
        }
    }
    None
}

pub fn external_script_open_request_from_env_args() -> Option<ExternalScriptOpenRequest> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project_path = command_line_option(&args, "--locus-project")?;
    let asset_path = command_line_option(&args, "--locus-open-script")?;
    if project_path.trim().is_empty() {
        return None;
    }
    let parse_position = |name: &str| {
        command_line_option(&args, name)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1)
            .max(1)
    };
    Some(ExternalScriptOpenRequest {
        project_path,
        asset_path,
        line: parse_position("--locus-line"),
        column: parse_position("--locus-column"),
    })
}

// ── Native broker bridge ─────────────────────────────────────────────
//
// When enabled, the Tauri↔Unity command channel is served by the native
// broker DLL (`locus_unity_native`) loaded inside the Unity process. The broker's
// pipe outlives domain reloads, so the connection no longer drops every time
// the editor recompiles. The toggle is global (a config flag) but takes effect
// per project via a marker file the Unity plugin checks before loading the DLL;
// the native broker is the required Unity command transport.

static NATIVE_BRIDGE_ENABLED: AtomicBool = AtomicBool::new(false);
static EXTERNAL_EDITOR_DEFAULT_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn initialize_native_bridge(enabled: bool) {
    NATIVE_BRIDGE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_native_bridge_enabled(value: bool) {
    NATIVE_BRIDGE_ENABLED.store(value, Ordering::Relaxed);
}

pub fn native_bridge_enabled() -> bool {
    NATIVE_BRIDGE_ENABLED.load(Ordering::Relaxed)
}

pub fn initialize_external_editor_default(enabled: bool) {
    EXTERNAL_EDITOR_DEFAULT_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_external_editor_default(enabled: bool) {
    EXTERNAL_EDITOR_DEFAULT_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn external_editor_default_enabled() -> bool {
    EXTERNAL_EDITOR_DEFAULT_ENABLED.load(Ordering::Relaxed)
}

/// Broker status as published by the native plugin's shared-memory state
/// plane. `None` means the native bridge is disabled or the broker has not
/// created the state plane for this project.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrokerStatus {
    #[serde(default)]
    pub native_alive: bool,
    #[serde(default)]
    pub observed_at_ms: i64,
    #[serde(default)]
    pub managed_state: String,
    #[serde(default)]
    pub domain_generation: i64,
    #[serde(default)]
    pub editor_status: String,
    #[serde(default)]
    pub last_managed_heartbeat_ms: i64,
    #[serde(default)]
    pub main_thread_heartbeat_age_ms: Option<u64>,
    #[serde(default)]
    pub main_thread_heartbeat_seq: u64,
    #[serde(default)]
    pub main_thread_id: Option<u32>,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub active_executions: u32,
    #[serde(default)]
    pub pending_requests: u32,
    #[serde(default)]
    pub inflight_requests: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub broker_capabilities: Vec<String>,
    #[serde(default)]
    pub managed_capabilities: Vec<String>,
    #[serde(default)]
    pub protocol_version: i32,
    #[serde(default)]
    pub pending_bytes: u32,
    #[serde(default)]
    pub queue_limit: u32,
    #[serde(default)]
    pub inflight_limit: u32,
    #[serde(default)]
    pub payload_limit_bytes: u32,
    #[serde(default)]
    pub pending_byte_limit: u32,
    #[serde(default)]
    pub writer_queue_limit: u32,
    #[serde(default)]
    pub request_deadline_ms: u32,
    /// The broker patched Unity's `IsApplicationActive` symbols in-process
    /// (migration Phase 6). When true the cross-process background hook stands
    /// down — the in-process patch already keeps the editor ticking and it
    /// survives domain reloads without a re-sync.
    #[serde(default)]
    pub background_patched: bool,
    #[serde(default)]
    pub background_symbols: u32,
    /// Executions currently publishing progress through the broker.
    #[serde(default)]
    pub executions: Vec<NativeBrokerExecution>,
}

/// Liveness of one running execution as last published to the broker.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrokerExecution {
    #[serde(default)]
    pub execution_id: String,
    #[serde(default)]
    pub published_age_ms: u64,
    #[serde(default)]
    pub idle_ms: Option<u64>,
    #[serde(default)]
    pub idle_limit_ms: Option<u64>,
    #[serde(default)]
    pub main_thread_blocked_ms: Option<u64>,
    #[serde(default)]
    pub stop_requested: Option<bool>,
}

impl NativeBrokerExecution {
    /// Managed code is still publishing and the snippet has not hit its idle
    /// limit or a stop request: the main thread is busy on purpose.
    pub fn is_live(&self) -> bool {
        self.published_age_ms < 5_000
            && self.stop_requested != Some(true)
            && self.idle_ms.unwrap_or(0) < self.idle_limit_ms.unwrap_or(30_000)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityBridgeReadinessState {
    Starting,
    Connected,
    Ready,
    Reloading,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityBridgeReadinessProbe {
    pub state: UnityBridgeReadinessState,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrokerEvent {
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub domain_generation: i64,
    #[serde(default)]
    pub editor_status: String,
    #[serde(default)]
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct NativeBrokerObservation {
    pub current: NativeBrokerStatus,
    pub events: Vec<NativeBrokerEvent>,
    pub cursor: u64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct NativeBrokerStatusPayload {
    #[serde(flatten)]
    status: NativeBrokerStatus,
    #[serde(default)]
    events: Vec<NativeBrokerEvent>,
    #[serde(default)]
    cursor: u64,
}

fn native_broker_state_mmf_name(project_path: &str) -> String {
    locus_native_protocol::names::unity_state_plane_name(project_path)
}

#[cfg(target_os = "windows")]
fn read_native_broker_status_payload_from_shared_memory(
    project_path: &str,
) -> Option<NativeBrokerStatusPayload> {
    native_state_plane_imp::read_native_broker_status_payload(&native_broker_state_mmf_name(
        project_path,
    ))
}

#[cfg(not(target_os = "windows"))]
fn read_native_broker_status_payload_from_shared_memory(
    _project_path: &str,
) -> Option<NativeBrokerStatusPayload> {
    None
}

#[cfg(target_os = "windows")]
mod native_state_plane_imp {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    use super::NativeBrokerStatusPayload;
    use locus_native_protocol::state_plane;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const FILE_MAP_READ: Dword = 0x0004;

    unsafe extern "system" {
        fn OpenFileMappingW(
            dwDesiredAccess: Dword,
            bInheritHandle: Bool,
            lpName: *const u16,
        ) -> Handle;
        fn MapViewOfFile(
            hFileMappingObject: Handle,
            dwDesiredAccess: Dword,
            dwFileOffsetHigh: Dword,
            dwFileOffsetLow: Dword,
            dwNumberOfBytesToMap: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(lpBaseAddress: *const c_void) -> Bool;
        fn CloseHandle(hObject: Handle) -> Bool;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn new(handle: Handle) -> Option<Self> {
            if handle.is_null() {
                None
            } else {
                Some(Self(handle))
            }
        }

        fn raw(&self) -> Handle {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct MappedView(*mut c_void);

    impl MappedView {
        fn new(handle: Handle, size: usize) -> Option<Self> {
            let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, size) };
            if ptr.is_null() {
                None
            } else {
                Some(Self(ptr))
            }
        }

        fn bytes(&self, len: usize) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.0 as *const u8, len) }
        }
    }

    impl Drop for MappedView {
        fn drop(&mut self) {
            unsafe {
                let _ = UnmapViewOfFile(self.0);
            }
        }
    }

    pub(super) fn read_native_broker_status_payload(
        mapping_name: &str,
    ) -> Option<NativeBrokerStatusPayload> {
        let total_size = state_plane::total_size();
        let name = wide_null(mapping_name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, FALSE, name.as_ptr()) };
        let handle = OwnedHandle::new(handle)?;
        let view = MappedView::new(handle.raw(), total_size)?;
        let snapshot = state_plane::read_latest(view.bytes(total_size))?;
        let payload = std::str::from_utf8(&snapshot.payload).ok()?;
        let mut parsed = serde_json::from_str::<NativeBrokerStatusPayload>(payload).ok()?;
        if parsed.status.observed_at_ms <= 0 {
            parsed.status.observed_at_ms = snapshot.observed_at_ms.min(i64::MAX as u64) as i64;
        }
        Some(parsed)
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }
}

fn native_broker_event_cursors() -> &'static StdMutex<HashMap<String, u64>> {
    static CURSORS: OnceLock<StdMutex<HashMap<String, u64>>> = OnceLock::new();
    CURSORS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn native_broker_event_consumer_key(project_path: &str, consumer: &str) -> String {
    format!("{}\n{}", project_runtime_key(project_path), consumer.trim())
}

fn native_broker_consumer_cursor(project_path: &str, consumer: &str) -> Option<u64> {
    native_broker_event_cursors()
        .lock()
        .ok()
        .and_then(|cursors| {
            cursors
                .get(&native_broker_event_consumer_key(project_path, consumer))
                .copied()
        })
}

fn update_native_broker_consumer_cursor(project_path: &str, consumer: &str, cursor: u64) {
    if let Ok(mut cursors) = native_broker_event_cursors().lock() {
        cursors.insert(
            native_broker_event_consumer_key(project_path, consumer),
            cursor,
        );
    }
}

/// Ask the native broker for its status. Best-effort, short-timeout, and a
/// no-op (returns `None`) when the native bridge is disabled or the broker is
/// not running for this project.
pub async fn query_native_broker_status(project_path: &str) -> Option<NativeBrokerStatus> {
    query_native_broker_status_payload(project_path, None)
        .await
        .map(|payload| payload.status)
}

async fn query_native_broker_status_payload(
    project_path: &str,
    cursor: Option<u64>,
) -> Option<NativeBrokerStatusPayload> {
    if !native_bridge_enabled() {
        return None;
    }
    let mut payload = read_native_broker_status_payload_from_shared_memory(project_path)?;
    if let Some(cursor) = cursor {
        payload.events.retain(|event| event.seq > cursor);
    } else {
        payload.events.clear();
    }
    Some(payload)
}

pub(crate) async fn query_native_broker_observation(
    project_path: &str,
    consumer: &str,
) -> Option<NativeBrokerObservation> {
    let cursor = native_broker_consumer_cursor(project_path, consumer);
    let mut payload = query_native_broker_status_payload(project_path, cursor).await?;
    let events = if cursor.is_some() {
        std::mem::take(&mut payload.events)
    } else {
        Vec::new()
    };
    let next_cursor = events
        .iter()
        .map(|event| event.seq)
        .max()
        .unwrap_or(payload.cursor)
        .max(payload.cursor);
    update_native_broker_consumer_cursor(project_path, consumer, next_cursor);
    Some(NativeBrokerObservation {
        current: payload.status,
        events,
        cursor: next_cursor,
    })
}

/// Reconcile the per-project marker the Unity plugin checks before loading the
/// native DLL. Writing it records the exact pipe name the broker should serve;
/// removing it disables the required native command transport for that project.
pub fn sync_native_bridge_marker(project_path: &str, enabled: bool) -> Result<(), String> {
    let path = native_bridge_marker_path(project_path);
    if enabled {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create native-bridge marker dir '{}': {}",
                    parent.display(),
                    error
                )
            })?;
        }
        let body = format!("{}\n", get_native_pipe_name(project_path));
        std::fs::write(&path, body).map_err(|error| {
            format!(
                "Failed to write native-bridge marker '{}': {}",
                path.display(),
                error
            )
        })?;
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|error| {
            format!(
                "Failed to remove native-bridge marker '{}': {}",
                path.display(),
                error
            )
        })?;
    }
    Ok(())
}

fn native_bridge_marker_path(project_path: &str) -> PathBuf {
    Path::new(strip_extended_path_prefix(project_path))
        .join("Library")
        .join("Locus")
        .join("NativeBridge.enabled")
}

/// Reconcile the per-project marker the Unity plugin checks before asking the
/// native broker to patch the engine's background-activity symbols in-process
/// (migration Phase 6). Present means "apply the in-process hook"; absent means
/// the managed side leaves it to the cross-process Tauri patch. Only meaningful
/// when the native bridge is enabled (the managed hook code only runs then).
pub fn sync_background_hook_marker(project_path: &str, enabled: bool) -> Result<(), String> {
    let path = background_hook_marker_path(project_path);
    if enabled {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create background-hook marker dir '{}': {}",
                    parent.display(),
                    error
                )
            })?;
        }
        std::fs::write(&path, "enabled\n").map_err(|error| {
            format!(
                "Failed to write background-hook marker '{}': {}",
                path.display(),
                error
            )
        })?;
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|error| {
            format!(
                "Failed to remove background-hook marker '{}': {}",
                path.display(),
                error
            )
        })?;
    }
    Ok(())
}

fn background_hook_marker_path(project_path: &str) -> PathBuf {
    Path::new(strip_extended_path_prefix(project_path))
        .join("Library")
        .join("Locus")
        .join("BackgroundHook.enabled")
}

/// Reconcile the marker read by the Unity editor window before it performs
/// HWND discovery or sends overlay control messages. Absence keeps the default
/// enabled behavior for existing projects and older Locus installations.
pub fn sync_unity_embed_enabled_marker(project_path: &str, enabled: bool) -> Result<(), String> {
    let path = unity_embed_disabled_marker_path(project_path);
    if enabled {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| {
                format!(
                    "Failed to remove Unity embed marker '{}': {}",
                    path.display(),
                    error
                )
            })?;
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create Unity embed marker dir '{}': {}",
                    parent.display(),
                    error
                )
            })?;
        }
        std::fs::write(&path, "disabled\n").map_err(|error| {
            format!(
                "Failed to write Unity embed marker '{}': {}",
                path.display(),
                error
            )
        })?;
    }
    Ok(())
}

fn unity_embed_disabled_marker_path(project_path: &str) -> PathBuf {
    Path::new(strip_extended_path_prefix(project_path))
        .join("Library")
        .join("Locus")
        .join("UnityEmbed.disabled")
}

fn native_background_hook_markers_present(project_path: &str) -> bool {
    native_bridge_marker_path(project_path).is_file()
        && background_hook_marker_path(project_path).is_file()
}

/// Transient native-broker errors meaning "the managed executor is briefly
/// unavailable (mid domain reload) — retry" rather than a real failure. Flows
/// that intentionally span a reload (e.g. recompile) treat a broker `ok:false`
/// with one of these codes as "keep waiting": the native pipe stays up and
/// answers with the code while the managed executor is re-registering.
pub(crate) fn is_transient_broker_error(error: &str) -> bool {
    matches!(
        error.trim(),
        "managed_reloading" | "managed_not_ready" | "domain_reload_interrupted"
    )
}

fn is_reload_boundary_broker_error(error: &str) -> bool {
    matches!(
        error.trim(),
        "managed_reloading" | "domain_reload_interrupted"
    )
}

fn pipe_response_transient_broker_error(response: &PipeResponse) -> bool {
    !response.ok
        && response
            .error
            .as_deref()
            .map(is_transient_broker_error)
            .unwrap_or(false)
}

const SHORT_MESSAGE_TRANSIENT_RETRY_ATTEMPTS: u32 = 3;
const SHORT_MESSAGE_TRANSIENT_READY_WAIT: Duration = Duration::from_secs(30);

fn transient_broker_error_from_response(response: &PipeResponse) -> Option<&str> {
    if response.ok {
        return None;
    }
    response
        .error
        .as_deref()
        .filter(|error| is_transient_broker_error(error))
}

async fn wait_before_transient_retry(
    project_path: &str,
    context: &str,
    error: &str,
    attempt: u32,
) -> Result<(), String> {
    eprintln!(
        "[Locus] {context} hit transient Unity broker state on attempt {attempt}: {error}; waiting for bridge readiness"
    );
    wait_for_unity_bridge_ready(project_path, SHORT_MESSAGE_TRANSIENT_READY_WAIT, context).await
}

pub(crate) async fn send_message_with_transient_retry(
    project_path: &str,
    msg_type: &str,
    message: &str,
    timeout: Duration,
    context: &str,
) -> Result<PipeResponse, String> {
    let mut attempt = 1;
    loop {
        let resp = send_message_with_timeout(project_path, msg_type, message, timeout).await?;
        let Some(error) = transient_broker_error_from_response(&resp).map(ToOwned::to_owned) else {
            return Ok(resp);
        };
        if attempt >= SHORT_MESSAGE_TRANSIENT_RETRY_ATTEMPTS {
            return Ok(resp);
        }
        wait_before_transient_retry(project_path, context, &error, attempt).await?;
        attempt += 1;
    }
}

pub async fn configure_locus_external_editor(
    project_path: &str,
    set_default: bool,
) -> Result<String, String> {
    let executable_path = std::env::current_exe()
        .map_err(|error| format!("Failed to resolve the Locus executable: {error}"))?;
    let payload = serde_json::json!({
        "executablePath": executable_path.to_string_lossy(),
        "setDefault": set_default,
    })
    .to_string();
    let response = send_message_with_transient_retry(
        project_path,
        "configure_locus_external_editor",
        &payload,
        Duration::from_secs(30),
        "configure Locus external editor",
    )
    .await?;
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "Unity rejected the external editor configuration".to_string()));
    }
    Ok(response.message.unwrap_or_default())
}

pub async fn sync_project_files(project_path: &str) -> Result<String, String> {
    let response = send_message_with_transient_retry(
        project_path,
        "sync_project_files",
        "",
        Duration::from_secs(120),
        "sync Unity project files",
    )
    .await?;
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "Unity project-file sync failed".to_string()));
    }
    Ok(response.message.unwrap_or_default())
}

fn unity_project_files_present(project_path: &str) -> bool {
    std::fs::read_dir(project_path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| {
                    extension.eq_ignore_ascii_case("sln")
                        || extension.eq_ignore_ascii_case("csproj")
                })
                .unwrap_or(false)
        })
}

async fn send_message_without_timeout_with_transient_retry(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    let mut attempt = 1;
    loop {
        let resp = send_message_without_timeout(project_path, msg_type, message).await?;
        let Some(error) = transient_broker_error_from_response(&resp).map(ToOwned::to_owned) else {
            return Ok(resp);
        };
        if attempt >= SHORT_MESSAGE_TRANSIENT_RETRY_ATTEMPTS {
            return Ok(resp);
        }
        wait_before_transient_retry(project_path, msg_type, &error, attempt).await?;
        attempt += 1;
    }
}

/// Monitors are keyed by normalized project path so one Locus process can
/// observe several Unity checkouts without replacing the previous monitor.
pub type UnityMonitorHandle =
    Arc<tokio::sync::Mutex<HashMap<String, (String, tauri::async_runtime::JoinHandle<()>)>>>;

pub const UNITY_EDITOR_STATUS_DISCONNECTED: &str = "disconnected";
pub const UNITY_EDITOR_STATUS_EDITING: &str = "editing";
pub const UNITY_EDITOR_STATUS_PLAYING: &str = "playing";
pub const UNITY_EDITOR_STATUS_PLAYING_PAUSED: &str = "playing_paused";
pub const UNITY_EDITOR_STATUS_SCHEMA: &str = "disconnected | editing | playing | playing_paused";
const UNITY_STATUS_POLL_TIMEOUT: Duration = Duration::from_millis(800);
/// Connection monitor cadence; process exits and lifecycle events wake it early.
const UNITY_MONITOR_INTERVAL: Duration = Duration::from_secs(3);
/// Longest gap between `unity-connection-status-detail` events while nothing
/// but timing changes.
const UNITY_CONNECTION_DETAIL_HEARTBEAT_MS: u64 = 30_000;
const UNITY_STATUS_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const UNITY_PROCESS_STATUS_TIMEOUT: Duration = Duration::from_millis(1_000);
const UNITY_CONNECTION_STATUS_STALE_MS: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipeResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default, rename = "processId")]
    pub process_id: Option<u32>,
    #[serde(default, rename = "processPath")]
    pub process_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnityTestRunSnapshot {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub unity_run_guid: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub current_test: String,
    #[serde(default)]
    pub started_at_ticks: i64,
    #[serde(default)]
    pub finished_at_ticks: i64,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub total: i32,
    #[serde(default)]
    pub passed: i32,
    #[serde(default)]
    pub failed: i32,
    #[serde(default)]
    pub skipped: i32,
    #[serde(default)]
    pub inconclusive: i32,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub failures: Vec<serde_json::Value>,
    #[serde(default)]
    pub results: Vec<serde_json::Value>,
}

pub const UNITY_EXECUTE_PROGRESS_TAG: &str = "locus-unity-progress";
pub const UNITY_EXECUTE_CANCELLED: &str = "__locus_unity_execute_cancelled__";
const UNITY_EXECUTE_PROGRESS_POLL_MS: u64 = 250;
const UNITY_EXECUTE_START_TIMEOUT_SECS: u64 = 15;
const UNITY_EXECUTE_REATTACH_MAX_ATTEMPTS: u32 = 3;
const UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS: u64 = 120;
const UNITY_EXECUTE_WAITING_STATUS_INTERVAL_MS: u64 = 2_000;
/// Total run budget of one execution (`unity_execute` `timeout`). Unity
/// enforces it; the host only keeps a backstop past it.
pub const UNITY_EXECUTE_DEFAULT_TIMEOUT_MS: u64 = 600_000;
pub const UNITY_EXECUTE_MIN_TIMEOUT_MS: u64 = 1_000;
pub const UNITY_EXECUTE_MAX_TIMEOUT_MS: u64 = 3_600_000;
const UNITY_EXECUTE_TIMEOUT_MARKER: &str = "//__LOCUS_TIMEOUT_MS__:";
/// How long past its budget the host waits for Unity's own stop before it
/// detaches from the execution.
const UNITY_EXECUTE_BUDGET_GRACE: Duration = Duration::from_secs(30);
/// The broker's snapshot is this old and the main thread is this far away:
/// managed code is frozen as a whole, not just busy running the snippet.
const UNITY_EXECUTE_PUBLISHER_STALE_MS: u64 = 30_000;
const UNITY_EXECUTE_MAIN_THREAD_STALL_MS: i64 = 30_000;
/// A result note mentions main-thread blocks at least this long.
pub const UNITY_EXECUTE_LONG_BLOCK_NOTE_MS: u64 = 15_000;
pub const UNITY_EXECUTE_BUDGET_EXCEEDED: &str = "unity_execute_timeout_exceeded";

/// Options of an agent-facing `unity_execute` call.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnityExecuteOptions {
    pub enable_non_public_access: bool,
    /// Total run budget in milliseconds; `None` keeps Unity's default.
    pub timeout_ms: Option<u64>,
}

/// Result of a successful execution plus what the host observed of it.
#[derive(Debug, Clone, Default)]
pub struct UnityExecuteOutcome {
    pub output: String,
    /// Longest reported stretch the snippet kept Unity's main thread away
    /// from the editor loop (the Editor was frozen meanwhile).
    pub max_main_thread_blocked_ms: u64,
}

pub fn clamp_unity_execute_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.clamp(UNITY_EXECUTE_MIN_TIMEOUT_MS, UNITY_EXECUTE_MAX_TIMEOUT_MS)
}

/// Parameters of `tools/unity_execute.json` (the async mode parameter is
/// added by the async task runtime and accepted separately).
pub const UNITY_EXECUTE_PARAMETERS: &[&str] = &[
    "code",
    "readonly",
    "request_editor_status",
    "enable_non_public_access",
    "timeout",
];

fn camel_to_snake_case(key: &str) -> String {
    let mut snake = String::with_capacity(key.len() + 4);
    for (index, ch) in key.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                snake.push('_');
            }
            snake.push(ch.to_ascii_lowercase());
        } else {
            snake.push(ch);
        }
    }
    snake
}

/// Validated `unity_execute` arguments shared by the agent loop, the tool
/// registry and the MCP server.
#[derive(Debug, Clone)]
pub struct UnityExecuteArgs {
    pub code: String,
    pub requested_status: String,
    pub options: UnityExecuteOptions,
}

pub fn parse_unity_execute_args(args: &serde_json::Value) -> Result<UnityExecuteArgs, String> {
    let object = args
        .as_object()
        .ok_or_else(|| "unity_execute arguments must be a JSON object".to_string())?;
    // An unknown key is a guess the model expects to matter (e.g. a timeout
    // under another name); ignoring it silently hides the mistake. The agent
    // loop mirrors some keys in camelCase (`requestEditorStatus`), so the
    // camelCase spelling of a known parameter is not a guess.
    let unknown = object
        .keys()
        .map(String::as_str)
        .filter(|key| {
            let snake = camel_to_snake_case(key);
            !UNITY_EXECUTE_PARAMETERS.contains(&snake.as_str())
                && *key != crate::async_tasks::ASYNC_MODE_PARAMETER
        })
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(format!(
            "Unknown unity_execute parameter(s): {}. Allowed parameters: code, readonly, request_editor_status, enable_non_public_access, timeout (total run budget in milliseconds).",
            unknown.join(", ")
        ));
    }
    let code = object
        .get("code")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Missing required parameter: code".to_string())?;
    let requested_status = object
        .get("request_editor_status")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing required parameter: request_editor_status".to_string())?;
    if requested_status == UNITY_EDITOR_STATUS_DISCONNECTED
        || !is_known_editor_status(requested_status)
    {
        return Err(format!(
            "Invalid request_editor_status: '{requested_status}'. Allowed values: editing, playing, playing_paused."
        ));
    }
    let enable_non_public_access = crate::csharp_compile::resolve_tool_non_public_access(args)?;
    let timeout_ms = match object.get("timeout") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .or_else(|| {
                    value
                        .as_f64()
                        .filter(|number| number.is_finite() && *number >= 0.0)
                        .map(|number| number as u64)
                })
                .ok_or_else(|| {
                    "Parameter 'timeout' must be a number of milliseconds (1000 to 3600000).".to_string()
                })?,
        ),
    };
    Ok(UnityExecuteArgs {
        code: code.to_string(),
        requested_status: requested_status.to_string(),
        options: UnityExecuteOptions {
            enable_non_public_access,
            timeout_ms: timeout_ms.map(clamp_unity_execute_timeout_ms),
        },
    })
}

/// Agent-facing note for a successful run that froze the Editor for long.
pub fn unity_execute_long_block_note(max_main_thread_blocked_ms: u64) -> Option<String> {
    (max_main_thread_blocked_ms >= UNITY_EXECUTE_LONG_BLOCK_NOTE_MS).then(|| {
        format!(
            "[Locus] Unity's main thread stayed busy for {:.1}s in one stretch while this snippet ran (the Editor was frozen). \
Locus stops a snippet after 30s without print/printJson/ctx.Progress output; for longer batches, print or call ctx.Progress per chunk \
and `await ctx.WaitFrame()` between chunks.",
            max_main_thread_blocked_ms as f64 / 1000.0
        )
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityLaunchResult {
    pub editor_path: String,
    pub project_path: String,
    pub project_version: String,
    pub process_id: u32,
    pub mode: UnityLaunchMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityLaunchMode {
    Interactive,
    Headless,
}

impl UnityLaunchMode {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("interactive") => Ok(Self::Interactive),
            Some("headless") => Ok(Self::Headless),
            Some(value) => Err(format!(
                "mode must be 'interactive' or 'headless'; got '{value}'"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Headless => "headless",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConnectionStatus {
    pub connected: bool,
    pub editor_status: String,
    pub control_channel_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_path: Option<String>,
    pub scene_paths: Vec<String>,
    pub editor_process_state: UnityEditorProcessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_mode: Option<UnityLaunchMode>,
    pub headless: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_checked_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_last_error: Option<String>,
    pub pipe_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub reconnect_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub background_hook: UnityBackgroundHookStatus,
    pub checked_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default)]
    pub source: String,
    #[serde(default, rename = "waitKind")]
    pub wait_kind: String,
    #[serde(default, rename = "waitTarget")]
    pub wait_target: String,
    #[serde(default, rename = "waitCondition")]
    pub wait_condition: String,
    #[serde(default, rename = "sourceLine")]
    pub source_line: u32,
    #[serde(default, rename = "sourceText")]
    pub source_text: String,
    #[serde(default, rename = "waitedMs")]
    pub waited_ms: u64,
    // Liveness published by Unity's thread-pool monitor and served by the
    // native broker without the main thread. Absent from older plugins.
    #[serde(default, rename = "executing", skip_serializing_if = "Option::is_none")]
    pub executing: Option<bool>,
    #[serde(default, rename = "elapsedMs", skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, rename = "idleMs", skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<u64>,
    #[serde(default, rename = "idleLimitMs", skip_serializing_if = "Option::is_none")]
    pub idle_limit_ms: Option<u64>,
    #[serde(default, rename = "mainThreadBlockedMs", skip_serializing_if = "Option::is_none")]
    pub main_thread_blocked_ms: Option<u64>,
    #[serde(default, rename = "maxMainThreadBlockedMs", skip_serializing_if = "Option::is_none")]
    pub max_main_thread_blocked_ms: Option<u64>,
    #[serde(default, rename = "outputChars", skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<u64>,
    #[serde(default, rename = "lastOutput", skip_serializing_if = "String::is_empty")]
    pub last_output: String,
    #[serde(default, rename = "stopRequested", skip_serializing_if = "Option::is_none")]
    pub stop_requested: Option<bool>,
    #[serde(default, rename = "stopReason", skip_serializing_if = "String::is_empty")]
    pub stop_reason: String,
    #[serde(default, rename = "publishedAgeMs", skip_serializing_if = "Option::is_none")]
    pub published_age_ms: Option<u64>,
    #[serde(default, rename = "mainThreadStallMs", skip_serializing_if = "Option::is_none")]
    pub main_thread_stall_ms: Option<i64>,
    #[serde(default, rename = "servedBy", skip_serializing_if = "String::is_empty")]
    pub served_by: String,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetThumbnailRequest<'a> {
    asset_path: &'a str,
    max_size: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetPreviewRenderRequest<'a> {
    asset_path: &'a str,
    width: u32,
    height: u32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pan_x: f32,
    pan_y: f32,
    pan_z: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityAssetThumbnail {
    pub asset_path: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
    pub png_base64: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityAssetPreviewFrame {
    pub asset_path: String,
    pub width: u32,
    pub height: u32,
    pub mime_type: String,
    pub data_base64: String,
}

pub(crate) type ProjectUnityOpLock = Arc<Mutex<()>>;

fn unity_operation_locks() -> &'static Mutex<HashMap<String, ProjectUnityOpLock>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, ProjectUnityOpLock>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn unity_recompile_waits() -> &'static StdMutex<HashMap<String, u32>> {
    static WAITS: OnceLock<StdMutex<HashMap<String, u32>>> = OnceLock::new();
    WAITS.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[derive(Default)]
struct ConnectionStatusCacheEntry {
    status: Option<UnityConnectionStatus>,
    invalidated_at_ms: u64,
}

fn unity_connection_status_cache() -> &'static StdMutex<HashMap<String, ConnectionStatusCacheEntry>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, ConnectionStatusCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn project_runtime_key(project_path: &str) -> String {
    strip_extended_path_prefix(project_path).trim().to_string()
}

fn normalize_project_path_for_state_plane(project_path: &str) -> String {
    locus_native_protocol::names::normalize_project_path(project_path)
}

pub(crate) fn project_state_plane_key(project_path: &str) -> String {
    locus_native_protocol::names::project_key(project_path)
}

struct UnityRecompileWaitGuard {
    key: String,
}

impl UnityRecompileWaitGuard {
    fn new(project_path: &str) -> Self {
        let key = project_runtime_key(project_path);
        if let Ok(mut waits) = unity_recompile_waits().lock() {
            let count = waits.entry(key.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
        Self { key }
    }
}

impl Drop for UnityRecompileWaitGuard {
    fn drop(&mut self) {
        if let Ok(mut waits) = unity_recompile_waits().lock() {
            if let Some(count) = waits.get_mut(&self.key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    waits.remove(&self.key);
                }
            }
        }
    }
}

fn unity_recompile_waiting(project_path: &str) -> bool {
    let key = project_runtime_key(project_path);
    unity_recompile_waits()
        .lock()
        .map(|waits| waits.get(&key).copied().unwrap_or(0) > 0)
        .unwrap_or(false)
}

pub(crate) async fn project_unity_op_lock(project_path: &str) -> ProjectUnityOpLock {
    let key = crate::agent::workspace_execution_lock::normalize_workspace_path_key(project_path, ".");
    let mut locks = unity_operation_locks().lock().await;
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn strip_extended_path_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// Pipe name part (without the `\\.\pipe\` prefix) the native broker serves.
/// Mirrors `LocusBridge.GenerateNativePipeName` on the Unity side.
fn native_pipe_name_part(project_path: &str) -> String {
    locus_native_protocol::names::unity_pipe_name_part(project_path)
}

/// Full client path of the native broker pipe for this project.
pub(crate) fn get_native_pipe_name(project_path: &str) -> String {
    let suffix=std::env::var("LOCUS_UNITY_TEST_PIPE_NAMESPACE").ok()
        .filter(|value| !value.is_empty() && value.len()<=64 && value.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'-'));
    format!(r"\\.\pipe\{}{}", native_pipe_name_part(project_path),suffix.map(|s|format!("_{s}")).unwrap_or_default())
}

pub fn is_unity_project(path: &str) -> bool {
    let p = Path::new(strip_extended_path_prefix(path));
    p.join("Assets").is_dir() && p.join("ProjectSettings").is_dir()
}

/// Mirror of `AppConfig::unity_enabled`, readable from free functions that
/// have no config handle. Defaults to on so code paths that run before setup
/// (tests, CLI drivers) keep the historical behavior.
static UNITY_INTEGRATION_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

pub fn set_unity_integration_enabled(enabled: bool) {
    UNITY_INTEGRATION_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn unity_integration_globally_enabled() -> bool {
    UNITY_INTEGRATION_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Entry check for Unity features: the app-level switch is on and `path` is a
/// Unity project. Code that only describes directory layout (knowledge storage
/// location, merge asset handling) keeps using [`is_unity_project`].
pub fn unity_integration_enabled(path: &str) -> bool {
    unity_integration_globally_enabled() && is_unity_project(path)
}

pub fn read_project_unity_version(project_path: &str) -> Result<Option<String>, String> {
    let version_path = Path::new(strip_extended_path_prefix(project_path))
        .join("ProjectSettings")
        .join("ProjectVersion.txt");
    if !version_path.is_file() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&version_path).map_err(|error| {
        format!(
            "Failed to read Unity project version file '{}': {}",
            version_path.display(),
            error
        )
    })?;

    Ok(content.lines().find_map(|line| {
        line.strip_prefix("m_EditorVersion:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }))
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

fn push_editor_install_root_candidates(paths: &mut Vec<PathBuf>, root: PathBuf) {
    #[cfg(target_os = "windows")]
    {
        // Unity first (byte-for-byte unchanged), then Tuanjie's renamed
        // executable in the identical layout. A given install root only ever
        // contains one flavor's exe, so the other flavor's candidate is silently
        // skipped by the `is_file()` check in `resolve_unity_editor_executable`.
        for flavor in EditorFlavor::ALL {
            push_unique_path(
                paths,
                root.join("Editor").join(flavor.editor_exe_file_name()),
            );
            push_unique_path(paths, root.join(flavor.editor_exe_file_name()));
        }
    }

    #[cfg(target_os = "macos")]
    {
        push_unique_path(
            paths,
            root.join("Unity.app")
                .join("Contents")
                .join("MacOS")
                .join("Unity"),
        );
        push_unique_path(paths, root.join("Contents").join("MacOS").join("Unity"));
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        push_unique_path(paths, root.join("Editor").join("Unity"));
        push_unique_path(paths, root.join("Unity"));
    }
}

fn push_env_editor_candidates(paths: &mut Vec<PathBuf>) {
    let Some(raw_path) = std::env::var_os("LOCUS_UNITY_EDITOR_PATH") else {
        return;
    };
    let path = PathBuf::from(raw_path);
    if path.is_file() {
        push_unique_path(paths, path);
    } else {
        push_editor_install_root_candidates(paths, path);
    }
}

#[cfg(target_os = "windows")]
fn push_windows_registry_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let subkeys = [
        format!(r"SOFTWARE\Unity Technologies\Installer\Unity {version}"),
        format!(r"SOFTWARE\WOW6432Node\Unity Technologies\Installer\Unity {version}"),
    ];
    let hives = [
        RegKey::predef(HKEY_CURRENT_USER),
        RegKey::predef(HKEY_LOCAL_MACHINE),
    ];

    for hive in hives {
        for subkey in &subkeys {
            let Ok(key) = hive.open_subkey(subkey) else {
                continue;
            };
            for value_name in ["Location x64", "Location"] {
                let Ok(location) = key.get_value::<String, _>(value_name) else {
                    continue;
                };
                let location = location.trim();
                if !location.is_empty() {
                    push_editor_install_root_candidates(paths, PathBuf::from(location));
                }
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn push_windows_registry_editor_candidates(_paths: &mut Vec<PathBuf>, _version: &str) {}

fn push_default_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            push_editor_install_root_candidates(
                paths,
                PathBuf::from(program_files)
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
        if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
            push_editor_install_root_candidates(
                paths,
                PathBuf::from(program_files_x86)
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        push_editor_install_root_candidates(
            paths,
            PathBuf::from("/Applications")
                .join("Unity")
                .join("Hub")
                .join("Editor")
                .join(version),
        );
        push_editor_install_root_candidates(paths, PathBuf::from("/Applications").join("Unity"));
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(home_dir) = dirs::home_dir() {
            push_editor_install_root_candidates(
                paths,
                home_dir
                    .join("Unity")
                    .join("Hub")
                    .join("Editor")
                    .join(version),
            );
        }
        push_editor_install_root_candidates(
            paths,
            PathBuf::from("/opt")
                .join("Unity")
                .join("Hub")
                .join("Editor")
                .join(version),
        );
    }
}

#[derive(Deserialize)]
struct UnityHubEditorsCache {
    #[serde(default)]
    data: Vec<UnityHubEditorEntry>,
}

#[derive(Deserialize)]
struct UnityHubEditorEntry {
    #[serde(default)]
    version: String,
    #[serde(default)]
    location: Vec<String>,
}

/// Unity Hub records every editor it manages — including ones installed outside
/// `Program Files` — in `editors-v2.json`. Return the `data[].location[]` paths
/// whose `version` matches, in file order.
fn parse_unity_hub_editor_locations(cache_json: &str, version: &str) -> Vec<PathBuf> {
    let version = version.trim();
    let Ok(cache) = serde_json::from_str::<UnityHubEditorsCache>(cache_json) else {
        return Vec::new();
    };

    let mut locations = Vec::new();
    for entry in cache.data {
        if entry.version.trim() != version {
            continue;
        }
        for location in entry.location {
            let trimmed = location.trim();
            if !trimmed.is_empty() {
                locations.push(PathBuf::from(trimmed));
            }
        }
    }
    locations
}

/// Location of the Unity Hub editor cache. `dirs::config_dir()` maps to the
/// directory Unity Hub actually writes to on each platform: `%APPDATA%`
/// (Roaming) on Windows, `~/Library/Application Support` on macOS, `~/.config`
/// on Linux.
fn unity_hub_editors_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("UnityHub").join("editors-v2.json"))
}

/// Fall back to the Unity Hub editor cache so editors installed in non-default
/// locations (e.g. `D:\Apps\Unity`, `F:\UnityEditor`) are still discovered.
fn push_unity_hub_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    let Some(cache_path) = unity_hub_editors_cache_path() else {
        return;
    };
    let Ok(content) = std::fs::read_to_string(&cache_path) else {
        return;
    };

    for location in parse_unity_hub_editor_locations(&content, version) {
        // Windows/Linux record the executable directly; macOS records the
        // `Unity.app` bundle directory, which the install-root helper expands.
        if location.is_dir() {
            push_editor_install_root_candidates(paths, location);
        } else {
            push_unique_path(paths, location);
        }
    }
}

/// Install roots managed by Tuanjie Hub, read from its `secondaryInstallPath.json`
/// (a JSON string such as `"F:\\tuanjie"`). Empty when the Hub is not installed.
/// Tuanjie Hub stores config under `%APPDATA%\TuanjieHub`, parallel to Unity Hub's
/// `%APPDATA%\UnityHub`, and does NOT use an `editors-v2.json` cache.
#[cfg(target_os = "windows")]
fn tuanjie_hub_install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(config_dir) = dirs::config_dir() else {
        return roots;
    };
    let path = config_dir
        .join("TuanjieHub")
        .join("secondaryInstallPath.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(root) = serde_json::from_str::<String>(content.trim()) {
            let trimmed = root.trim();
            if !trimmed.is_empty() {
                roots.push(PathBuf::from(trimmed));
            }
        }
    }
    roots
}

/// Read the editor executable from the "Tuanjie <version>" Windows uninstall
/// entry's `DisplayIcon` (e.g. `F:\tuanjie\2022.3.62t10\Editor\Tuanjie.exe`).
/// Tuanjie does not register under `Unity Technologies\Installer`, so this is the
/// registry-side discovery path for the fork.
#[cfg(target_os = "windows")]
fn push_tuanjie_registry_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let target_display_name = format!("Tuanjie {version}");
    let uninstall_paths = [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    let hives = [
        RegKey::predef(HKEY_LOCAL_MACHINE),
        RegKey::predef(HKEY_CURRENT_USER),
    ];

    for hive in hives {
        for uninstall_path in uninstall_paths {
            let Ok(uninstall) = hive.open_subkey(uninstall_path) else {
                continue;
            };
            for entry_name in uninstall.enum_keys().flatten() {
                let Ok(entry) = uninstall.open_subkey(&entry_name) else {
                    continue;
                };
                let display_name = entry
                    .get_value::<String, _>("DisplayName")
                    .unwrap_or_default();
                if display_name.trim() != target_display_name {
                    continue;
                }
                let Ok(display_icon) = entry.get_value::<String, _>("DisplayIcon") else {
                    continue;
                };
                // DisplayIcon may be quoted and/or carry a trailing ",<index>".
                let icon = display_icon.trim().trim_matches('"');
                let exe = icon.split(',').next().unwrap_or(icon).trim();
                if !exe.is_empty() {
                    push_unique_path(paths, PathBuf::from(exe));
                }
            }
        }
    }
}

/// Tuanjie (团结引擎, the Unity China fork) is invisible to every Unity-specific
/// discovery source: it is not under `Unity Technologies\Installer`, not in
/// `%ProgramFiles%\Unity\Hub`, and not in Unity Hub's `editors-v2.json`. Add the
/// fork's own locations here. Called AFTER all Unity sources so a standard Unity
/// install always resolves first and is never affected.
#[cfg(target_os = "windows")]
fn push_tuanjie_editor_candidates(paths: &mut Vec<PathBuf>, version: &str) {
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        push_editor_install_root_candidates(
            paths,
            PathBuf::from(program_files)
                .join("Tuanjie")
                .join("Hub")
                .join("Editor")
                .join(version),
        );
    }

    for root in tuanjie_hub_install_roots() {
        push_editor_install_root_candidates(paths, root.join(version));
    }

    push_tuanjie_registry_editor_candidates(paths, version);
}

#[cfg(not(target_os = "windows"))]
fn push_tuanjie_editor_candidates(_paths: &mut Vec<PathBuf>, _version: &str) {}

pub fn resolve_unity_editor_executable(version: &str) -> Result<PathBuf, String> {
    let version = version.trim();
    if version.is_empty() {
        return Err("Unity project version is empty".to_string());
    }

    let mut candidates = Vec::new();
    push_env_editor_candidates(&mut candidates);
    push_windows_registry_editor_candidates(&mut candidates, version);
    push_default_editor_candidates(&mut candidates, version);
    push_unity_hub_editor_candidates(&mut candidates, version);
    // Tuanjie fork sources, after every Unity source so Unity always wins.
    push_tuanjie_editor_candidates(&mut candidates, version);

    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }

    let checked = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let editor_label = if flavor::is_tuanjie_version(version) {
        "Tuanjie Editor"
    } else {
        "Unity Editor"
    };
    Err(format!(
        "{} {} was not found. Checked: {}",
        editor_label, version, checked
    ))
}

fn normalized_project_path_for_launch(project_path: &str) -> PathBuf {
    let trimmed = strip_extended_path_prefix(project_path).trim();
    dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnityLaunchCodeOptimization {
    Debug,
    Release,
}

pub async fn launch_project(project_path: &str) -> Result<UnityLaunchResult, String> {
    launch_project_with_mode_and_options(project_path, UnityLaunchMode::Interactive, None).await
}

pub async fn launch_project_with_mode(
    project_path: &str,
    mode: UnityLaunchMode,
) -> Result<UnityLaunchResult, String> {
    launch_project_with_mode_and_options(project_path, mode, None).await
}

pub async fn launch_project_with_options(
    project_path: &str,
    code_optimization: Option<UnityLaunchCodeOptimization>,
) -> Result<UnityLaunchResult, String> {
    launch_project_with_mode_and_options(
        project_path,
        UnityLaunchMode::Interactive,
        code_optimization,
    )
    .await
}

pub async fn launch_project_with_mode_and_options(
    project_path: &str,
    mode: UnityLaunchMode,
    code_optimization: Option<UnityLaunchCodeOptimization>,
) -> Result<UnityLaunchResult, String> {
    if !unity_integration_enabled(project_path) {
        return Err("Current working directory is not a Unity project".to_string());
    }

    if mode == UnityLaunchMode::Headless {
        managed_editor::prepare_plugin(project_path).await?;
    }
    let project_version = read_project_unity_version(project_path)?
        .ok_or_else(|| "Current Unity project is missing ProjectVersion.txt".to_string())?;
    let editor_path = resolve_unity_editor_executable(&project_version)?;
    let project_path = normalized_project_path_for_launch(project_path);
    let editor_log_path = project_path.join("Logs").join("Editor.log");
    if let Some(parent) = editor_log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Unity Editor log directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }

    let mut command = std::process::Command::new(&editor_path);
    command
        .arg("-projectPath")
        .arg(&project_path)
        .arg("-logFile")
        .arg(&editor_log_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if mode == UnityLaunchMode::Headless {
        command.arg("-batchmode").arg("-nographics");
    }
    match code_optimization {
        Some(UnityLaunchCodeOptimization::Debug) => {
            command.arg("-debugCodeOptimization");
        }
        Some(UnityLaunchCodeOptimization::Release) => {
            command.arg("-releaseCodeOptimization");
        }
        None => {}
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command.spawn().map_err(|error| {
        format!(
            "Failed to launch Unity Editor '{}': {}",
            editor_path.display(),
            error
        )
    })?;
    let process_id = child.id();
    let checked_at_ms = unix_now_ms();
    let editor_path = editor_path.display().to_string();
    let project_path = project_path.display().to_string();

    process::cache_project_editor_process(
        &project_path,
        UnityEditorProcessInfo {
            state: UnityEditorProcessState::Running,
            process_id: Some(process_id),
            executable_path: Some(editor_path.clone()),
            project_path: Some(project_path.clone()),
            checked_at_ms,
            last_error: None,
        },
    )
    .await;
    state_probe::clear_project_observer_state(&project_path);

    eprintln!(
        "[Locus] launched Unity Editor: editor='{}', project='{}', process_id={}, mode={}",
        editor_path,
        project_path,
        process_id,
        mode.as_str()
    );

    let launch = UnityLaunchResult {
        editor_path,
        project_path,
        project_version,
        process_id,
        mode,
    };
    managed_editor::record_launch(&launch);
    Ok(launch)
}

pub(crate) fn launched_unity_process_created_at_ms(process_id: u32) -> Option<u64> {
    process::process_created_at_unix_ms(process_id)
}

pub(crate) fn launched_unity_process_liveness(
    process_id: u32,
    expected_created_at_ms: Option<u64>,
) -> Result<UnityProcessIdentityLiveness, String> {
    process::query_process_identity_liveness(process_id, expected_created_at_ms)
}

pub(crate) async fn reaffirm_launched_unity_editor_process(
    project_path: &str,
    editor_path: &str,
    process_id: u32,
    expected_created_at_ms: Option<u64>,
) -> Result<UnityProcessIdentityLiveness, String> {
    let liveness = launched_unity_process_liveness(process_id, expected_created_at_ms)?;
    if liveness == UnityProcessIdentityLiveness::Alive {
        // Repair any late old-generation disconnect probe that raced with the
        // launch. Clearing the semantic cache prevents its derived `crashed`
        // state from surviving after the expected process was proven alive.
        process::cache_project_editor_process(
            project_path,
            UnityEditorProcessInfo {
                state: UnityEditorProcessState::Running,
                process_id: Some(process_id),
                executable_path: Some(editor_path.to_string()),
                project_path: Some(project_path.to_string()),
                checked_at_ms: unix_now_ms(),
                last_error: None,
            },
        )
        .await;
        state_probe::clear_project_observer_state(project_path);
    }
    Ok(liveness)
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

pub fn play_mode_target_status(mode: &str) -> Result<&'static str, String> {
    match mode.trim() {
        "play" => Ok(UNITY_EDITOR_STATUS_PLAYING),
        "edit" => Ok(UNITY_EDITOR_STATUS_EDITING),
        value => Err(format!(
            "Invalid mode: '{}'. Allowed values: play, edit.",
            value
        )),
    }
}

pub fn format_play_mode_tool_result(mode: &str, changed: bool) -> String {
    match (mode.trim(), changed) {
        ("play", true) => "Unity Editor entered Play Mode (playing).".to_string(),
        ("play", false) => "Unity Editor is already in Play Mode (playing).".to_string(),
        ("edit", true) => "Unity Editor returned to Edit Mode (editing).".to_string(),
        ("edit", false) => "Unity Editor is already in Edit Mode (editing).".to_string(),
        (value, _) => format!("Unity Editor mode request completed: {value}."),
    }
}

fn requested_run_states_editor_status(request: &serde_json::Value) -> Result<&str, String> {
    let requested_status = request
        .get("request_editor_status")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing required parameter: request_editor_status".to_string())?;

    if requested_status == UNITY_EDITOR_STATUS_DISCONNECTED
        || !is_known_editor_status(requested_status)
    {
        return Err(format!(
            "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
            requested_status
        ));
    }

    Ok(requested_status)
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
    if status.eq_ignore_ascii_case("safe_mode") {
        return "`safe_mode` (fix script compilation errors with file tools; Unity APIs are unavailable)";
    }
    if status.eq_ignore_ascii_case("crashed") {
        return "`crashed` (Unity exited abnormally; inspect the Editor log before restarting)";
    }
    match normalize_editor_status(status) {
        UNITY_EDITOR_STATUS_DISCONNECTED => "`disconnected`",
        UNITY_EDITOR_STATUS_PLAYING => "`playing`",
        UNITY_EDITOR_STATUS_PLAYING_PAUSED => "`playing_paused`",
        _ => "`editing`",
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn parse_unity_status_message(message: &str) -> (&'static str, Option<String>, Vec<String>) {
    let mut parts = message.split('|');
    let status = normalize_editor_status(parts.next().unwrap_or_default());
    let scene_path = parts
        .next()
        .map(str::trim)
        .filter(|scene| !scene.is_empty())
        .map(ToOwned::to_owned);
    let mut scene_paths = Vec::new();
    if let Some(active_scene_path) = scene_path.as_ref() {
        scene_paths.push(active_scene_path.clone());
    }
    for scene_path in parts.map(str::trim).filter(|scene| !scene.is_empty()) {
        if !scene_paths
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(scene_path))
        {
            scene_paths.push(scene_path.to_string());
        }
    }
    (status, scene_path, scene_paths)
}

fn apply_unity_process_info(
    status: &mut UnityConnectionStatus,
    process_info: UnityEditorProcessInfo,
) {
    status.editor_process_state = process_info.state;
    status.editor_process_id = process_info.process_id;
    status.editor_process_path = process_info.executable_path;
    status.editor_project_path = process_info.project_path;
    status.process_checked_at_ms = Some(process_info.checked_at_ms);
    status.process_last_error = process_info.last_error;
}

async fn sync_unity_launch_mode_for_status(status: &mut UnityConnectionStatus) {
    if status.editor_process_state != UnityEditorProcessState::Running {
        status.launch_mode = None;
        status.headless = false;
        return;
    }
    status.launch_mode = process::query_unity_editor_launch_mode(status.editor_process_id).await;
    status.headless = status.launch_mode == Some(UnityLaunchMode::Headless);
}

fn unity_process_info_from_status(
    status: &UnityConnectionStatus,
) -> Option<UnityEditorProcessInfo> {
    let process_id = status.editor_process_id?;
    Some(UnityEditorProcessInfo {
        state: status.editor_process_state.clone(),
        process_id: Some(process_id),
        executable_path: status.editor_process_path.clone(),
        project_path: status.editor_project_path.clone(),
        checked_at_ms: status.process_checked_at_ms.unwrap_or(status.checked_at_ms),
        last_error: status.process_last_error.clone(),
    })
}

fn cache_unity_connection_status(project_path: &str, status: &UnityConnectionStatus) {
    if let Ok(mut cache) = unity_connection_status_cache().lock() {
        let entry = cache.entry(project_runtime_key(project_path)).or_default();
        if status.checked_at_ms >= entry.invalidated_at_ms
            && entry.status.as_ref().is_none_or(|old| old.checked_at_ms <= status.checked_at_ms) {
            entry.status = Some(status.clone());
        }
    }
}

/// The kernel exit wait saw the project's editor process end. Drop state that
/// assumed a live editor; the observers publish the exit now instead of on
/// their next interval.
pub(super) fn note_editor_process_exited(project_path: &str) {
    invalidate_closed_editor_status(project_path);
}

/// Re-sample the project's semantic state and connection status immediately.
pub(crate) fn wake_project_observers(project_path: &str) {
    state_probe::wake_observer(project_path);
    wake_unity_monitor(project_path);
}

fn unity_monitor_wakers() -> std::sync::MutexGuard<'static, HashMap<String, Arc<tokio::sync::Notify>>> {
    static WAKERS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> =
        std::sync::OnceLock::new();
    WAKERS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn register_unity_monitor_waker(project_path: &str) -> Arc<tokio::sync::Notify> {
    let wake = Arc::new(tokio::sync::Notify::new());
    unity_monitor_wakers().insert(normalize_project_path_for_state_plane(project_path), wake.clone());
    wake
}

fn unregister_unity_monitor_waker(project_path: &str) {
    unity_monitor_wakers().remove(&normalize_project_path_for_state_plane(project_path));
}

fn wake_unity_monitor(project_path: &str) {
    let wake = unity_monitor_wakers()
        .get(&normalize_project_path_for_state_plane(project_path))
        .cloned();
    // `notify_one` stores a permit, so a wake during a tick is not lost.
    if let Some(wake) = wake {
        wake.notify_one();
    }
}

/// Connection status without per-check timing, for change detection.
fn connection_status_signature(status: &UnityConnectionStatus) -> String {
    let mut stable = status.clone();
    stable.checked_at_ms = 0;
    stable.process_checked_at_ms = None;
    stable.latency_ms = None;
    stable.reconnect_attempts = 0;
    stable.background_hook.updated_at_ms = 0;
    serde_json::to_string(&stable).unwrap_or_default()
}

fn invalidate_closed_editor_status(project_path: &str) {
    if let Ok(mut cache) = unity_connection_status_cache().lock() {
        let entry = cache.entry(project_runtime_key(project_path)).or_default();
        entry.status = None;
        entry.invalidated_at_ms = unix_now_ms().saturating_add(1);
    }
    state_probe::clear_project_observer_state(project_path);
    state_probe::clear_editor_status_intent(project_path);
    wake_project_observers(project_path);
}

fn cached_running_connection_status_for_transient_failure(
    project_path: &str,
    checked_at_ms: u64,
    error: impl Into<String>,
    preserve_connected: bool,
) -> Option<UnityConnectionStatus> {
    let error = error.into();
    let mut status = unity_connection_status_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&project_runtime_key(project_path)).and_then(|entry| entry.status.clone()))?;
    if !matches!(
        status.editor_process_state,
        UnityEditorProcessState::Running
    ) {
        return None;
    }
    if checked_at_ms.saturating_sub(status.checked_at_ms) > UNITY_CONNECTION_STATUS_STALE_MS {
        return None;
    }
    let pid_created_at_ms = status
        .editor_process_id
        .and_then(process::process_created_at_unix_ms);
    if let Some(fallback_status) = state_probe::fallback_editor_status_for_project(
        project_path,
        status.editor_process_id,
        pid_created_at_ms,
    ) {
        status.editor_status = fallback_status;
    }
    status.connected = preserve_connected && status.connected;
    status.control_channel_state = if error.contains("busy") {
        "busy".to_string()
    } else if error.contains("timed out") {
        "timeout".to_string()
    } else {
        "error".to_string()
    };
    status.checked_at_ms = checked_at_ms;
    status.latency_ms = None;
    status.last_error = Some(error);
    Some(status)
}

fn apply_observed_editor_status_fallback(project_path: &str, status: &mut UnityConnectionStatus) {
    if !matches!(
        status.editor_process_state,
        UnityEditorProcessState::Running
    ) {
        return;
    }
    let pid_created_at_ms = status
        .editor_process_id
        .and_then(process::process_created_at_unix_ms);
    if let Some(fallback_status) = state_probe::fallback_editor_status_for_project(
        project_path,
        status.editor_process_id,
        pid_created_at_ms,
    ) {
        status.editor_status = fallback_status;
    }
}

fn process_hint_from_response(
    resp: &PipeResponse,
    project_path: &str,
    checked_at_ms: u64,
) -> Option<UnityEditorProcessInfo> {
    let process_id = resp.process_id.filter(|id| *id > 0)?;
    Some(UnityEditorProcessInfo {
        state: UnityEditorProcessState::Running,
        process_id: Some(process_id),
        executable_path: resp
            .process_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        project_path: Some(strip_extended_path_prefix(project_path).trim().to_string()),
        checked_at_ms,
        last_error: None,
    })
}

fn inactive_background_hook_status(project_path: &str) -> UnityBackgroundHookStatus {
    if background_hook::enabled() {
        UnityBackgroundHookStatus {
            enabled: true,
            supported: cfg!(target_os = "windows"),
            state: UnityBackgroundHookState::Inactive,
            patched: false,
            process_id: None,
            editor_process_path: None,
            symbol_count: 0,
            error: None,
            updated_at_ms: unix_now_ms(),
        }
    } else {
        background_hook::status_for_project(project_path)
    }
}

fn should_defer_background_hook_to_native(
    status: &UnityConnectionStatus,
    project_path: &str,
) -> bool {
    background_hook::enabled()
        && native_bridge_enabled()
        && !status.connected
        && native_background_hook_markers_present(project_path)
}

async fn sync_background_hook_for_status(status: &mut UnityConnectionStatus, project_path: &str) {
    // The in-process native hook (when active) owns the patch and survives
    // domain reloads; the cross-process path then stands down.
    if let Some(native) = native_owned_background_hook(project_path).await {
        background_hook::record_status_for_project(project_path, native.clone());
        status.background_hook = native;
        return;
    }
    let Some(process_id) = status.editor_process_id else {
        let current = background_hook::status_for_project(project_path);
        if matches!(
            status.editor_process_state,
            UnityEditorProcessState::Running
        ) && current.enabled
            && current.patched
        {
            status.background_hook = current;
            return;
        }

        status.background_hook = inactive_background_hook_status(project_path);
        background_hook::record_status_for_project(project_path, status.background_hook.clone());
        return;
    };

    if should_defer_background_hook_to_native(status, project_path) {
        status.background_hook = inactive_background_hook_status(project_path);
        background_hook::record_status_for_project(project_path, status.background_hook.clone());
        return;
    }

    let Some(editor_process_path) = status.editor_process_path.clone() else {
        status.background_hook = UnityBackgroundHookStatus {
            enabled: background_hook::enabled(),
            supported: cfg!(target_os = "windows"),
            state: UnityBackgroundHookState::Failed,
            patched: false,
            process_id: Some(process_id),
            editor_process_path: None,
            symbol_count: 0,
            error: Some("Unity process path is unavailable".to_string()),
            updated_at_ms: unix_now_ms(),
        };
        background_hook::record_status_for_project(project_path, status.background_hook.clone());
        return;
    };

    let hook_project_path = project_path.to_string();
    let hook_status = tauri::async_runtime::spawn_blocking(move || {
        background_hook::sync_for_project(&hook_project_path, process_id, &editor_process_path)
    })
    .await
    .map_err(|error| format!("Unity background hook task failed: {error}"))
    .and_then(|result| result)
    .unwrap_or_else(|error| UnityBackgroundHookStatus {
        enabled: background_hook::enabled(),
        supported: cfg!(target_os = "windows"),
        state: UnityBackgroundHookState::Failed,
        patched: false,
        process_id: Some(process_id),
        editor_process_path: status.editor_process_path.clone(),
        symbol_count: 0,
        error: Some(error),
        updated_at_ms: unix_now_ms(),
    });
    background_hook::record_status_for_project(project_path, hook_status.clone());
    status.background_hook = hook_status;
}

async fn query_process_info_for_connection_status(
    project_path: &str,
    connected: bool,
    process_hint: Option<UnityEditorProcessInfo>,
) -> UnityEditorProcessInfo {
    if !connected && unity_recompile_waiting(project_path) {
        if let Some(cached) = process::cached_project_editor_process(project_path).await {
            // An exit recorded during the recompile window (the exit wait or a
            // probe; a reload keeps the process alive) is a crash, not a reload.
            if cached.process_id.is_some()
                || cached.state == UnityEditorProcessState::NotRunning
            {
                return cached;
            }
        }
        return UnityEditorProcessInfo::inferred_running(unix_now_ms());
    }

    let probe = query_current_project_editor_process(project_path).await;
    let Some(hint) = process_hint else {
        return probe;
    };

    if !connected {
        return probe;
    }

    match (&probe.state, probe.process_id, hint.process_id) {
        (UnityEditorProcessState::Running, Some(probe_id), Some(hint_id))
            if probe_id == hint_id =>
        {
            let mut info = probe;
            if info.executable_path.is_none() {
                info.executable_path = hint.executable_path;
            }
            process::cache_project_editor_process(project_path, info.clone()).await;
            info
        }
        (UnityEditorProcessState::Running, Some(probe_id), Some(hint_id)) => {
            let info = UnityEditorProcessInfo {
                state: UnityEditorProcessState::Running,
                process_id: Some(hint_id),
                executable_path: hint.executable_path.or(probe.executable_path),
                project_path: hint.project_path.or(probe.project_path),
                checked_at_ms: probe.checked_at_ms,
                last_error: Some(format!(
                    "Unity process probe PID {probe_id} does not match pipe PID {hint_id}"
                )),
            };
            process::cache_project_editor_process(project_path, info.clone()).await;
            info
        }
        _ => {
            let mut info = hint;
            info.checked_at_ms = probe.checked_at_ms.max(info.checked_at_ms);
            info.last_error = probe.last_error;
            process::cache_project_editor_process(project_path, info.clone()).await;
            info
        }
    }
}

async fn query_process_info_for_connection_status_bounded(
    project_path: &str,
    connected: bool,
    process_hint: Option<UnityEditorProcessInfo>,
) -> UnityEditorProcessInfo {
    let fallback_hint = process_hint.clone();
    match tokio::time::timeout(
        UNITY_PROCESS_STATUS_TIMEOUT,
        query_process_info_for_connection_status(project_path, connected, process_hint),
    )
    .await
    {
        Ok(info) => info,
        Err(_) => {
            let checked_at_ms = unix_now_ms();
            if let Some(mut hint) = fallback_hint {
                hint.checked_at_ms = checked_at_ms;
                hint.last_error = Some("Unity process probe timed out".to_string());
                return hint;
            }
            UnityEditorProcessInfo {
                state: UnityEditorProcessState::Unknown,
                process_id: None,
                executable_path: None,
                project_path: None,
                checked_at_ms,
                last_error: Some("Unity process probe timed out".to_string()),
            }
        }
    }
}

/// Opportunistic status probe for observers and progress polling. `Ok(None)`
/// means another request currently owns the pipe writer; callers must preserve
/// that distinction instead of treating it as a disconnected editor.
async fn try_query_unity_status_response_with_timeout(
    project_path: &str,
    timeout: Duration,
) -> Result<Option<(PipeResponse, u64)>, String> {
    let started_at = std::time::Instant::now();
    let response =
        transport::send_message_if_writer_free(project_path, "status", "", timeout).await?;
    let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Ok(response.map(|resp| (resp, latency_ms)))
}

/// Authoritative status request for command preconditions. This path waits for
/// the short-lived pipe writer lock, so concurrent requests cannot turn local
/// writer contention into a false disconnected result.
async fn query_unity_status_response_waiting_with_timeout(
    project_path: &str,
    timeout: Duration,
) -> Result<(PipeResponse, u64), String> {
    let started_at = std::time::Instant::now();
    let response = send_message_with_timeout(project_path, "status", "", timeout).await?;
    let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Ok((response, latency_ms))
}

pub async fn query_unity_connection_status(project_path: &str) -> UnityConnectionStatus {
    let pipe_name = get_native_pipe_name(project_path);
    let checked_at_ms = unix_now_ms();

    match try_query_unity_status_response_with_timeout(project_path, UNITY_STATUS_POLL_TIMEOUT)
        .await
    {
        Ok(Some((resp, latency_ms))) if resp.ok => {
            let process_hint = process_hint_from_response(&resp, project_path, checked_at_ms);
            let message = resp.message.unwrap_or_default();
            let (editor_status, scene_path, scene_paths) = parse_unity_status_message(&message);
            state_probe::note_pipe_editor_status(
                project_path,
                editor_status,
                resp.process_id,
                checked_at_ms,
            );
            let mut status = UnityConnectionStatus {
                connected: true,
                editor_status: editor_status.to_string(),
                control_channel_state: "ready".to_string(),
                scene_path,
                scene_paths,
                editor_process_state: UnityEditorProcessState::Running,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                launch_mode: None,
                headless: false,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: None,
                background_hook: background_hook::status_for_project(project_path),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, true, process_hint)
                    .await;
            apply_unity_process_info(&mut status, process_info);
            sync_unity_launch_mode_for_status(&mut status).await;
            sync_background_hook_for_status(&mut status, project_path).await;
            cache_unity_connection_status(project_path, &status);
            status
        }
        Ok(Some((resp, latency_ms))) => {
            let error = resp
                .error
                .unwrap_or_else(|| "Unity status returned ok=false".to_string());
            let control_channel_state = if is_transient_broker_error(&error) {
                if error == "managed_reloading" || error == "domain_reload_interrupted" {
                    "reloading".to_string()
                } else {
                    "starting".to_string()
                }
            } else {
                "error".to_string()
            };
            let mut status = UnityConnectionStatus {
                connected: false,
                editor_status: UNITY_EDITOR_STATUS_DISCONNECTED.to_string(),
                control_channel_state,
                scene_path: None,
                scene_paths: Vec::new(),
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                launch_mode: None,
                headless: false,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status_for_project(project_path),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
            sync_unity_launch_mode_for_status(&mut status).await;
            apply_observed_editor_status_fallback(project_path, &mut status);
            sync_background_hook_for_status(&mut status, project_path).await;
            cache_unity_connection_status(project_path, &status);
            status
        }
        Ok(None) => {
            let error = "Unity status poll skipped because the pipe writer is busy".to_string();
            if let Some(status) = cached_running_connection_status_for_transient_failure(
                project_path,
                checked_at_ms,
                error.clone(),
                true,
            ) {
                return status;
            }
            let native_editor_state = query_native_broker_status(project_path)
                .await
                .filter(|status| status.native_alive)
                .map(|status| parse_unity_status_message(&status.editor_status));
            let (editor_status, scene_path, scene_paths) =
                native_editor_state.unwrap_or((UNITY_EDITOR_STATUS_DISCONNECTED, None, Vec::new()));
            let mut status = UnityConnectionStatus {
                // `Ok(None)` is returned only after get_or_connect succeeded and
                // the existing connection's writer lock was observed busy. It
                // therefore proves a live local connection, even when this is
                // the first poll and no ready status has been cached yet.
                connected: true,
                editor_status: editor_status.to_string(),
                control_channel_state: "busy".to_string(),
                scene_path,
                scene_paths,
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                launch_mode: None,
                headless: false,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: None,
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status_for_project(project_path),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
            sync_unity_launch_mode_for_status(&mut status).await;
            apply_observed_editor_status_fallback(project_path, &mut status);
            sync_background_hook_for_status(&mut status, project_path).await;
            // Keep the cache anchored to the last authoritative ready/error
            // sample. Caching a busy observation would let repeated contention
            // extend the freshness window indefinitely.
            status
        }
        Err(error) => {
            if let Some(status) = cached_running_connection_status_for_transient_failure(
                project_path,
                checked_at_ms,
                error.clone(),
                false,
            ) {
                return status;
            }
            let mut status = UnityConnectionStatus {
                connected: false,
                editor_status: UNITY_EDITOR_STATUS_DISCONNECTED.to_string(),
                control_channel_state: if error.contains("timed out") {
                    "timeout".to_string()
                } else {
                    "disconnected".to_string()
                },
                scene_path: None,
                scene_paths: Vec::new(),
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                launch_mode: None,
                headless: false,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: None,
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status_for_project(project_path),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
            sync_unity_launch_mode_for_status(&mut status).await;
            apply_observed_editor_status_fallback(project_path, &mut status);
            sync_background_hook_for_status(&mut status, project_path).await;
            cache_unity_connection_status(project_path, &status);
            status
        }
    }
}

/// When the native broker has patched the background symbols in-process
/// (migration Phase 6), returns a synthesized "patched" status so the
/// cross-process Tauri hook stands down. `None` means the native path is
/// inactive (bridge off, broker absent, or it did not patch) and the caller
/// should fall back to the cross-process patch — this gating fails open.
async fn native_owned_background_hook(project_path: &str) -> Option<UnityBackgroundHookStatus> {
    if !native_bridge_enabled() {
        return None;
    }
    let status = query_native_broker_status(project_path).await?;
    if !status.background_patched {
        return None;
    }
    Some(UnityBackgroundHookStatus {
        enabled: true,
        supported: cfg!(target_os = "windows"),
        state: UnityBackgroundHookState::Patched,
        patched: true,
        process_id: None,
        editor_process_path: None,
        symbol_count: status.background_symbols,
        error: None,
        updated_at_ms: unix_now_ms(),
    })
}

pub async fn ensure_background_hook_for_project(
    project_path: &str,
) -> Result<UnityBackgroundHookStatus, String> {
    if !background_hook::enabled() {
        return Ok(background_hook::status_for_project(project_path));
    }
    if let Some(native) = native_owned_background_hook(project_path).await {
        background_hook::record_status_for_project(project_path, native.clone());
        return Ok(native);
    }
    if native_bridge_enabled() && native_background_hook_markers_present(project_path) {
        let status = inactive_background_hook_status(project_path);
        background_hook::record_status_for_project(project_path, status.clone());
        return Ok(status);
    }
    let process_info = query_current_project_editor_process(project_path).await;
    let process_id = process_info.process_id.ok_or_else(|| {
        process_info
            .last_error
            .unwrap_or_else(|| "Unity Editor process was not found".to_string())
    })?;
    let editor_process_path = process_info
        .executable_path
        .ok_or_else(|| "Unity process path is unavailable".to_string())?;
    let hook_project_path = project_path.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        background_hook::sync_for_project(&hook_project_path, process_id, &editor_process_path)
    })
    .await
    .map_err(|error| format!("Unity background hook task failed: {error}"))?
}

pub async fn background_hook_effective_for_project(project_path: &str) -> bool {
    match ensure_background_hook_for_project(project_path).await {
        Ok(status) => status.enabled && status.patched,
        Err(error) => {
            eprintln!("[Locus] Unity background hook unavailable: {error}");
            false
        }
    }
}

pub async fn is_unity_connected(project_path: &str) -> bool {
    query_unity_status(project_path).await.0
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

pub async fn asset_thumbnail(
    project_path: &str,
    asset_path: &str,
    max_size: u32,
) -> Result<UnityAssetThumbnail, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&AssetThumbnailRequest {
        asset_path,
        max_size,
    })
    .map_err(|e| format!("Failed to serialize asset_thumbnail request: {}", e))?;
    let resp = send_message(project_path, "asset_thumbnail", &payload).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "asset_thumbnail failed".to_string()));
    }
    let message = resp
        .message
        .ok_or_else(|| "asset_thumbnail returned an empty response".to_string())?;
    serde_json::from_str::<UnityAssetThumbnail>(&message)
        .map_err(|e| format!("Failed to parse asset_thumbnail response: {}", e))
}

pub async fn asset_preview_render(
    project_path: &str,
    asset_path: &str,
    width: u32,
    height: u32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    pan_x: f32,
    pan_y: f32,
    pan_z: f32,
) -> Result<UnityAssetPreviewFrame, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&AssetPreviewRenderRequest {
        asset_path,
        width,
        height,
        yaw,
        pitch,
        distance,
        pan_x,
        pan_y,
        pan_z,
    })
    .map_err(|e| format!("Failed to serialize asset_preview_render request: {}", e))?;
    let resp = send_message(project_path, "asset_preview_render", &payload).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "asset_preview_render failed".to_string()));
    }
    let message = resp
        .message
        .ok_or_else(|| "asset_preview_render returned an empty response".to_string())?;
    serde_json::from_str::<UnityAssetPreviewFrame>(&message)
        .map_err(|e| format!("Failed to parse asset_preview_render response: {}", e))
}

/// Small lifecycle messages only. Frames and input use the GPU transport and
/// never acquire the project-wide compilation/execution operation lock.
pub(crate) async fn render_output_control(project_path: &str, operation: &str, id: &str) -> Result<serde_json::Value, String> {
    let payload = serde_json::json!({"operation": operation, "id": id}).to_string();
    let response = send_message(project_path, "render_output", &payload).await?;
    if !response.ok { return Err(response.error.unwrap_or_else(|| "GPU output request failed".into())); }
    serde_json::from_str(&response.message.ok_or("Missing GPU output response")?).map_err(|e| e.to_string())
}

pub async fn select_scene_object(
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
    let resp = send_message(project_path, "select_scene_object", &payload).await?;
    if resp.ok {
        let _ = focus::bring_unity_to_foreground();
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "select_scene_object failed".to_string()))
    }
}

pub async fn validate_scene_object(
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
    let resp = send_message(project_path, "validate_scene_object", &payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "validate_scene_object failed".to_string()))
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

pub async fn start_asset_drag(project_path: &str, payload: &str) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message(project_path, "start_asset_drag", payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "start_asset_drag failed".to_string()))
    }
}

pub async fn cancel_asset_drag(project_path: &str) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message(project_path, "cancel_asset_drag", "").await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "cancel_asset_drag failed".to_string()))
    }
}

pub async fn open_frontend_window(project_path: &str, payload: &str) -> Result<(), String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let resp = send_message(project_path, "open_frontend_window", payload).await?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "open_frontend_window failed".to_string()))
    }
}

/// Canonical status values: "disconnected" | "editing" | "playing" | "playing_paused"
pub async fn query_unity_status(project_path: &str) -> (bool, &'static str, Option<String>) {
    query_unity_status_with_timeout(project_path, UNITY_STATUS_REQUEST_TIMEOUT).await
}

/// Like `query_unity_status` but with an explicit (short) timeout, so a wedged
/// editor or a half-open pipe cannot stall the caller for the default 35s. A
/// timeout reads as disconnected — the out-of-process native probe is then the
/// authority for what the editor is actually doing.
pub async fn query_unity_status_with_timeout(
    project_path: &str,
    timeout: Duration,
) -> (bool, &'static str, Option<String>) {
    match query_unity_status_response_waiting_with_timeout(project_path, timeout).await {
        Ok((resp, _)) if resp.ok => {
            let msg = resp.message.unwrap_or_default();
            let (status, scene_part, _scene_paths) = parse_unity_status_message(&msg);
            state_probe::note_pipe_editor_status(
                project_path,
                status,
                resp.process_id,
                unix_now_ms(),
            );
            (true, status, scene_part)
        }
        _ => (false, UNITY_EDITOR_STATUS_DISCONNECTED, None),
    }
}

pub async fn exit_play_mode(project_path: &str) -> Result<(), String> {
    let resp =
        send_message_with_timeout(project_path, "exit_play_mode", "", Duration::from_secs(45))
            .await?;
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

    state_probe::note_editor_status_intent(project_path, desired_status);
    let resp = match send_message(project_path, "set_editor_status", desired_status).await {
        Ok(resp) => {
            state_probe::note_editor_status_intent_acked(project_path);
            resp
        }
        Err(error) => {
            state_probe::clear_editor_status_intent(project_path);
            return Err(error);
        }
    };
    if !resp.ok {
        state_probe::clear_editor_status_intent(project_path);
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
            state_probe::clear_editor_status_intent(project_path);
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

/// Compile a prepared unity_run_states request in the sidecar and build the
/// `run_states_loaded` payload. Compile-stage error wording mirrors the
/// Unity-side `HandleRunStates`/`HandleCompileRunStates`
/// ("run_states compilation exception: " + message); validation messages
/// pass through verbatim, as Unity returns them unprefixed.
async fn sidecar_compile_for_run_states(
    project_path: &str,
    prepared_request: &serde_json::Value,
    cache_mode: RunStatesCompileCacheMode,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
) -> SidecarCompileAttempt {
    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_unavailable(reason),
    };

    let cache_key = run_states_compile_cache_key(
        project_path,
        &params,
        prepared_request,
        non_public_access_probe_mode,
    );
    if cache_mode == RunStatesCompileCacheMode::Consume {
        if let Some(key) = cache_key.as_deref() {
            if let Some(cached) = take_cached_run_states_compile(key) {
                return SidecarCompileAttempt::Compiled {
                    payload: cached.payload,
                };
            }
        }
    }

    match crate::csharp_compile::compile_run_states_with_access_probe(
        &params,
        prepared_request,
        false,
        false,
        non_public_access_probe_mode,
    )
    .await
    {
        Ok(Ok(assembly)) => {
            let assembly_b64 = assembly.assembly_b64;
            let assembly_path = assembly.assembly_path;
            let entry_type = assembly
                .entry_type
                .unwrap_or_else(|| RUN_STATES_ENTRY_TYPE_FALLBACK.to_string());
            let mut payload = serde_json::json!({
                "entry_type": entry_type,
                "request_editor_status": prepared_request
                    .get("request_editor_status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "initial_state": prepared_request
                    .get("initial_state")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            });
            if let Some(object) = payload.as_object_mut() {
                if let Some(path) = assembly_path {
                    object.insert("assembly_path".to_string(), serde_json::Value::String(path));
                } else {
                    object.insert(
                        "assembly_b64".to_string(),
                        serde_json::Value::String(assembly_b64),
                    );
                }
            }
            let payload = payload.to_string();
            let compiled = SidecarCompileAttempt::Compiled {
                payload: payload.clone(),
            };
            if cache_mode == RunStatesCompileCacheMode::Store {
                if let Some(key) = cache_key {
                    store_cached_run_states_compile(
                        key,
                        CachedRunStatesAssembly {
                            payload,
                            inserted_at_ms: unix_now_ms(),
                        },
                    );
                }
            }
            compiled
        }
        Ok(Err(failure)) => SidecarCompileAttempt::CompileError(if failure.stage == "validation" {
            failure.message
        } else {
            format!("run_states compilation exception: {}", failure.message)
        }),
        Err(error) => sidecar_unavailable(error),
    }
}

pub async fn unity_run_states(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    unity_run_states_with_mode(project_path, request, None, false).await
}

/// Tool-facing path. `enable_non_public_access` selects the production
/// SkipVerification policy while retaining graceful legacy fallback when the
/// compile server or a loaded-assembly plugin handler is unavailable.
pub async fn unity_run_states_with_non_public_access(
    project_path: &str,
    request: &serde_json::Value,
    enable_non_public_access: bool,
) -> Result<String, String> {
    unity_run_states_with_mode(
        project_path,
        request,
        production_non_public_access_mode(enable_non_public_access),
        false,
    )
    .await
}

/// Integration-test-only path: compile direct private/internal references in
/// the generated host and let Unity's normal run_states_loaded path perform
/// the runtime JIT check. This path never falls back to in-Unity compilation.
pub(crate) async fn unity_run_states_with_access_probe(
    project_path: &str,
    request: &serde_json::Value,
    mode: crate::csharp_compile::NonPublicAccessProbeMode,
) -> Result<String, String> {
    unity_run_states_with_mode(project_path, request, Some(mode), true).await
}

async fn unity_run_states_with_mode(
    project_path: &str,
    request: &serde_json::Value,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
    require_sidecar: bool,
) -> Result<String, String> {
    requested_run_states_editor_status(request)?;

    let prepared = prepare_unity_run_states_request_for_send(project_path, request).await;

    let mut msg_type = "run_states";
    let mut payload = serde_json::to_string(&prepared.request)
        .map_err(|error| format!("Failed to serialize unity_run_states request: {}", error))?;
    if crate::csharp_compile::is_enabled() {
        match sidecar_compile_for_run_states(
            project_path,
            &prepared.request,
            RunStatesCompileCacheMode::Consume,
            non_public_access_probe_mode,
        )
        .await
        {
            SidecarCompileAttempt::Compiled { payload: loaded } => {
                msg_type = "run_states_loaded";
                payload = loaded;
            }
            SidecarCompileAttempt::CompileError(message) => {
                return Err(crate::unity_type_index::append_auto_using_notes(
                    message,
                    &prepared.prepared_code,
                ));
            }
            SidecarCompileAttempt::Unavailable(reason) => {
                if require_sidecar {
                    return Err(format!(
                        "non-public access probe requires the sidecar compiler: {reason}"
                    ));
                }
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    } else if require_sidecar {
        return Err("non-public access probe requires the sidecar compiler".to_string());
    }

    eprintln!(
        "[Locus] unity_run_states sending {} ({} bytes)",
        msg_type,
        payload.len()
    );
    let mut resp = send_message_without_timeout(project_path, msg_type, &payload).await?;
    if msg_type == "run_states_loaded" && unity_plugin_lacks_message(&resp) {
        if require_sidecar {
            return Err(
                "non-public access probe requires a Unity plugin with run_states_loaded support"
                    .to_string(),
            );
        }
        crate::csharp_compile::note_fallback(
            "Unity plugin lacks run_states_loaded; update the Locus Unity plugin",
        );
        let legacy_payload = serde_json::to_string(&prepared.request)
            .map_err(|error| format!("Failed to serialize unity_run_states request: {}", error))?;
        resp = send_message_without_timeout(project_path, "run_states", &legacy_payload).await?;
    }
    let output = if resp.ok {
        resp.message.unwrap_or_default()
    } else {
        resp.error
            .unwrap_or_else(|| "unity_run_states failed".to_string())
    };

    let rewritten = match rewrite_run_states_output_for_size(project_path, output) {
        Ok(output) => output,
        Err(error) if resp.ok => return Err(error),
        Err(error) => {
            return Err(crate::unity_type_index::append_auto_using_notes(
                error,
                &prepared.prepared_code,
            ));
        }
    };
    if resp.ok {
        Ok(rewritten)
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            rewritten,
            &prepared.prepared_code,
        ))
    }
}

pub async fn compile_run_states(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    compile_run_states_with_mode(project_path, request, None, false).await
}

pub async fn compile_run_states_with_non_public_access(
    project_path: &str,
    request: &serde_json::Value,
    enable_non_public_access: bool,
) -> Result<String, String> {
    compile_run_states_with_mode(
        project_path,
        request,
        production_non_public_access_mode(enable_non_public_access),
        false,
    )
    .await
}

async fn compile_run_states_with_mode(
    project_path: &str,
    request: &serde_json::Value,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
    require_sidecar: bool,
) -> Result<String, String> {
    requested_run_states_editor_status(request)?;

    let prepared = prepare_unity_run_states_request_for_send(project_path, request).await;

    // Pre-check in the sidecar when available: compile errors come back
    // without occupying the Unity Editor (only the cheap params roundtrip
    // touches it). The pre-check image is never loaded into Unity, so it
    // must not enter the session image registry.
    if crate::csharp_compile::is_enabled() {
        match sidecar_compile_for_run_states(
            project_path,
            &prepared.request,
            RunStatesCompileCacheMode::Store,
            non_public_access_probe_mode,
        )
        .await
        {
            SidecarCompileAttempt::Compiled { .. } => {
                return Ok("run_states compilation ok".to_string());
            }
            SidecarCompileAttempt::CompileError(message) => {
                return Err(crate::unity_type_index::append_auto_using_notes(
                    message,
                    &prepared.prepared_code,
                ));
            }
            SidecarCompileAttempt::Unavailable(reason) => {
                if require_sidecar {
                    return Err(format!(
                        "non-public access probe requires the sidecar compiler: {reason}"
                    ));
                }
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    } else if require_sidecar {
        return Err("non-public access probe requires the sidecar compiler".to_string());
    }

    let payload = serde_json::to_string(&prepared.request).map_err(|error| {
        format!(
            "Failed to serialize unity_run_states compilation request: {}",
            error
        )
    })?;
    let resp = send_message_without_timeout(project_path, "compile_run_states", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error
                .unwrap_or_else(|| "unity_run_states compilation failed".to_string()),
            &prepared.prepared_code,
        ))
    }
}

/// Pre-compile a View Script (compile_named / invoke_named) request in the
/// sidecar. On success the request gains `assembly_path` (or a base64
/// fallback) plus `assembly_id`: a current Unity plugin loads the artifact on
/// a cache miss instead of compiling, an older plugin ignores the extra
/// fields and compiles from source exactly as before — so no fallback
/// handshake is needed.
///
/// Returns `Ok(Some(augmented))` to send, `Ok(None)` to send the original
/// request (sidecar unavailable), or `Err` with a deterministic compile
/// error in the Unity-side wording (View Script errors carry no prefix).
/// View/Skill precompile counterpart of `sidecar_unavailable`: a graceful
/// fallback sends the raw source (`Ok(None)` → Unity compiles in-process),
/// unless the operator disabled the in-process fallback, in which case the
/// unavailability is returned as an error so no in-Unity compile runs.
fn sidecar_augment_unavailable(reason: String) -> Result<Option<serde_json::Value>, String> {
    if crate::csharp_compile::block_in_process_fallback() {
        Err(format!(
            "sidecar compile unavailable and in-process fallback disabled: {reason}"
        ))
    } else {
        crate::csharp_compile::note_fallback(&reason);
        Ok(None)
    }
}

async fn augment_view_script_request_with_sidecar(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    if !crate::csharp_compile::is_enabled() {
        return Ok(None);
    }

    let source = request
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if source.trim().is_empty() {
        // invoke_named without source never reaches Unity's compiler either.
        return Ok(None);
    }

    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_augment_unavailable(reason),
    };

    let source_path = request
        .get("path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .unwrap_or("ViewScript.cs");
    let script_name = request
        .get("scriptName")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    match crate::csharp_compile::compile_view_script(&params, source, source_path, script_name)
        .await
    {
        Ok(Ok(assembly)) => {
            let mut augmented = request.clone();
            if let Some(object) = augmented.as_object_mut() {
                if let Some(path) = assembly.assembly_path {
                    object.insert("assembly_path".to_string(), serde_json::Value::String(path));
                } else {
                    object.insert(
                        "assembly_b64".to_string(),
                        serde_json::Value::String(assembly.assembly_b64),
                    );
                }
                object.insert(
                    "assembly_id".to_string(),
                    serde_json::Value::String(assembly.assembly_name),
                );
                Ok(Some(augmented))
            } else {
                Ok(None)
            }
        }
        Ok(Err(failure)) => Err(failure.message),
        Err(error) => sidecar_augment_unavailable(error),
    }
}

/// Pre-compile a Skill Package Unity script bundle in the sidecar. The
/// augmented request keeps the source payload so current Unity plugins can
/// fall back to their local compiler if the precompiled assembly cannot be
/// loaded.
async fn augment_skill_package_request_with_sidecar(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    if !crate::csharp_compile::is_enabled() {
        return Ok(None);
    }

    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_augment_unavailable(reason),
    };

    match crate::csharp_compile::compile_skill_package(&params, request).await {
        Ok(Ok(assembly)) => {
            let mut augmented = request.clone();
            if let Some(object) = augmented.as_object_mut() {
                if let Some(path) = assembly.assembly_path {
                    object.insert("assembly_path".to_string(), serde_json::Value::String(path));
                } else {
                    object.insert(
                        "assembly_b64".to_string(),
                        serde_json::Value::String(assembly.assembly_b64),
                    );
                }
                object.insert(
                    "assembly_id".to_string(),
                    serde_json::Value::String(assembly.assembly_name),
                );
                Ok(Some(augmented))
            } else {
                Ok(None)
            }
        }
        Ok(Err(failure)) => Err(failure.message),
        Err(error) => sidecar_augment_unavailable(error),
    }
}

pub async fn compile_named(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let augmented = augment_view_script_request_with_sidecar(project_path, request).await?;
    let effective_request = augmented.as_ref().unwrap_or(request);
    let payload = serde_json::to_string(effective_request)
        .map_err(|error| format!("Failed to serialize compile_named request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "compile_named", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "compile_named failed".to_string()))
    }
}

pub async fn compile_skill_package(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let augmented = augment_skill_package_request_with_sidecar(project_path, request).await?;
    let effective_request = augmented.as_ref().unwrap_or(request);
    let payload = serde_json::to_string(effective_request).map_err(|error| {
        format!(
            "Failed to serialize compile_skill_package request: {}",
            error
        )
    })?;
    let resp =
        send_message_without_timeout(project_path, "compile_skill_package", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "compile_skill_package failed".to_string()))
    }
}

pub async fn invoke_skill_package(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request).map_err(|error| {
        format!(
            "Failed to serialize invoke_skill_package request: {}",
            error
        )
    })?;
    let resp = send_message_without_timeout(project_path, "invoke_skill_package", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_skill_package failed".to_string()))
    }
}

pub async fn invoke_named(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let augmented = augment_view_script_request_with_sidecar(project_path, request).await?;
    let effective_request = augmented.as_ref().unwrap_or(request);
    let payload = serde_json::to_string(effective_request)
        .map_err(|error| format!("Failed to serialize invoke_named request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "invoke_named", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_named failed".to_string()))
    }
}

pub async fn invoke_named_cached(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize invoke_named_cached request: {}", error))?;
    let resp = send_message_without_timeout(project_path, "invoke_named_cached", &payload).await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "invoke_named_cached failed".to_string()))
    }
}

pub async fn view_binding_read(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message(project_path, "view_binding_read", request).await
}

pub async fn property_tree_read(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message_with_legacy_alias(
        project_path,
        "property_tree_read",
        "view_binding_read",
        request,
    )
    .await
}

pub async fn property_objects(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    if request["action"] == "read" {
        return send_property_tree_message(project_path, "property_objects", request).await;
    }
    let lock = project_unity_op_lock(project_path).await;
    let _guard = lock.lock().await;
    let mut payload = request.clone();
    exact_property_tree_ids(&mut payload)?;
    let payload = serde_json::to_string(&payload).map_err(|e|e.to_string())?;
    // A reload after apply can lose the acknowledgement. Do not replay writes,
    // even when the native broker reports a transient managed-executor state.
    let response = send_message_with_timeout(project_path, "property_objects", &payload, Duration::from_secs(120)).await
        .map_err(|e|format!("property.outcome_unknown: {e}; read before retrying"))?;
    if response.ok { Ok(response.message.unwrap_or_default()) }
    else { Err(response.error.unwrap_or_else(|| "property object write failed".into())) }
}

pub async fn property_tree_read_many(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message(project_path, "property_tree_read_many", request).await
}

pub async fn view_binding_discover(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message(project_path, "view_binding_discover", request).await
}

/// Send mutations once: retrying an uncertain array operation is unsafe.
pub async fn asset_api(project_path: &str, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let lock=project_unity_op_lock(project_path).await;
    let _guard=lock.lock().await;
    let payload=serde_json::to_string(request).map_err(|e|e.to_string())?;
    let response=send_message_with_timeout(project_path,"asset_api",&payload,Duration::from_secs(120)).await
        .map_err(|e|format!("assets.outcome_unknown: {e}; read the asset before retrying"))?;
    if !response.ok { return Err(response.error.unwrap_or_else(||"assets.editor_error".into())); }
    serde_json::from_str(response.message.as_deref().unwrap_or(""))
        .map_err(|e|format!("assets.invalid_response: {e}"))
}

pub async fn property_tree_discover(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message_with_legacy_alias(
        project_path,
        "property_tree_discover",
        "view_binding_discover",
        request,
    )
    .await
}

pub async fn view_binding_write(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message(project_path, "view_binding_write", request).await
}

pub async fn property_tree_write(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message_with_legacy_alias(
        project_path,
        "property_tree_write",
        "view_binding_write",
        request,
    )
    .await
}

pub async fn view_binding_apply(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message(project_path, "view_binding_apply", request).await
}

pub async fn property_tree_apply(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_property_tree_message_with_legacy_alias(
        project_path,
        "property_tree_apply",
        "view_binding_apply",
        request,
    )
    .await
}

async fn send_property_tree_message_with_legacy_alias(
    project_path: &str,
    canonical_type: &str,
    legacy_type: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    match send_property_tree_message(project_path, canonical_type, request).await {
        Err(error) if error.contains("unknown message type:") => {
            send_property_tree_message(project_path, legacy_type, request).await
        }
        result => result,
    }
}

fn exact_property_tree_ids(value:&mut serde_json::Value)->Result<(),String> {
    match value {
        serde_json::Value::Object(map)=>{
            for (key,value) in map {
                if matches!(key.as_str(),"objectFileId"|"targetFileId"|"managedReferenceId") {
                    if let Some(id)=value.as_str() {*value=serde_json::json!(id.parse::<i64>().map_err(|_|"Invalid exact Unity identifier")?);}
                } else {exact_property_tree_ids(value)?;}
            }
        },
        serde_json::Value::Array(items)=>{for value in items {exact_property_tree_ids(value)?;}},
        _=>{},
    }
    Ok(())
}

async fn send_property_tree_message(
    project_path: &str,
    message_type: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let mut internal_request=request.clone();
    exact_property_tree_ids(&mut internal_request)?;
    let payload = serde_json::to_string(&internal_request)
        .map_err(|error| format!("Failed to serialize {} request: {}", message_type, error))?;
    let resp =
        send_message_without_timeout_with_transient_retry(project_path, message_type, &payload)
            .await?;
    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| format!("{} failed", message_type)))
    }
}

fn require_unity_test_tools_available(project_path: &str) -> Result<(), String> {
    let status = crate::workspace::unity_test_tools_workspace_status(project_path);
    if !status.enabled {
        return Err(
            "Unity Test tools are disabled for this workspace. Enable them in Settings > Unity Connection."
                .to_string(),
        );
    }
    if !status.package_installed {
        return Err(
            "The Unity Test Framework package (com.unity.test-framework) is not installed in this project."
                .to_string(),
        );
    }
    if !status.package_supported {
        return Err(format!(
            "Unity Test tools require com.unity.test-framework {} or newer so interrupted runs can be cancelled (found {}).",
            crate::workspace::UNITY_TEST_FRAMEWORK_MIN_VERSION,
            status.package_version.as_deref().unwrap_or("unknown version")
        ));
    }
    Ok(())
}

async fn require_unity_test_sources_converged(project_path: &str) -> Result<(), String> {
    if crate::workspace::unity_test_sources_pending(project_path)
        || crate::unity_hotreload::coordinator::has_pending_state(project_path).await
    {
        return Err(
            "Unity Test discovery is waiting for C# convergence. Call unity_recompile first; newly added test files, methods, attributes, and hot-applied test changes become discoverable after compilation and domain reload complete."
                .to_string(),
        );
    }
    Ok(())
}

async fn ensure_unity_test_start_status(project_path: &str) -> Result<(), String> {
    let (connected, status, _) = query_unity_status(project_path).await;
    if !connected {
        return Err("Unity Editor not connected".to_string());
    }
    if status == UNITY_EDITOR_STATUS_EDITING {
        return Ok(());
    }
    if is_play_mode_status(status) {
        exit_play_mode(project_path).await?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let (connected, current, _) = query_unity_status(project_path).await;
            if connected && current == UNITY_EDITOR_STATUS_EDITING {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(
                    "Unity Editor did not return to Edit Mode before running tests".to_string(),
                );
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    Err(format!(
        "Unity Editor status is '{status}'. Unity Test runs start from Edit Mode."
    ))
}

pub async fn unity_test_list(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    require_unity_test_tools_available(project_path)?;
    require_unity_test_sources_converged(project_path).await?;
    ensure_unity_test_start_status(project_path).await?;
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize Unity Test list request: {error}"))?;
    let response = send_message_without_timeout_with_transient_retry(
        project_path,
        "unity_test_list",
        &payload,
    )
    .await?;
    if !response.ok {
        return Err(response.error.unwrap_or_else(|| {
            "Unity Test listing failed. Update the Locus Unity plugin and recompile the project."
                .to_string()
        }));
    }
    let message = response.message.unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&message)
        .map_err(|error| format!("Unity Test list returned invalid JSON: {error}"))?;
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to format Unity Test list: {error}"))
}

pub async fn yaml_preview_cache_selftest(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
        .map_err(|error| format!("Failed to serialize YAML parity request: {error}"))?;
    let response = send_message_with_transient_retry(
        project_path,
        "yaml_preview_cache_selftest",
        &payload,
        Duration::from_secs(180),
        "YAML preview cache self-test",
    )
    .await?;
    if !response.ok {
        return Err(response.error.unwrap_or_else(|| {
            "YAML preview cache self-test failed. Update the Locus Unity plugin and recompile the project."
                .to_string()
        }));
    }
    Ok(response.message.unwrap_or_default())
}

pub async fn unity_test_run(
    project_path: &str,
    request: &serde_json::Value,
    timeout: Duration,
) -> Result<UnityTestRunSnapshot, String> {
    unity_test_run_controlled(project_path, request, Some(timeout), None, None).await
}

const UNITY_TEST_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const UNITY_TEST_CANCEL_ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);
const UNITY_TEST_CANCEL_RESPONSE_GRACE: Duration = Duration::from_secs(1);
const UNITY_TEST_CANCEL_CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnityTestPollWake {
    Tick,
    DialogChanged,
    Cancelled,
}

fn unity_test_start_request(
    request: &serde_json::Value,
    requested_run_id: &str,
) -> Result<serde_json::Value, String> {
    let mut start_request = request.clone();
    let object = start_request
        .as_object_mut()
        .ok_or_else(|| "Unity Test run request must be an object".to_string())?;
    object.insert(
        "run_id".to_string(),
        serde_json::Value::String(requested_run_id.to_string()),
    );
    Ok(start_request)
}

fn unity_test_run_is_terminal(status: &str) -> bool {
    matches!(status, "cancelled" | "error" | "passed" | "failed")
}

fn unity_test_cancel_error(snapshot: &UnityTestRunSnapshot) -> Option<&str> {
    let error = snapshot.error.trim();
    if error.is_empty() {
        return None;
    }
    if error.starts_with("Unity Test cancellation requires com.unity.test-framework")
        || !unity_test_run_is_terminal(&snapshot.status)
    {
        return Some(error);
    }
    None
}

fn unity_test_cancel_snapshot(
    response: &PipeResponse,
) -> Result<Option<UnityTestRunSnapshot>, String> {
    if !response.ok {
        return Err(format!(
            "failed: {}",
            response
                .error
                .as_deref()
                .unwrap_or("Unity rejected the cancellation request")
        ));
    }
    let Some(message) = response
        .message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
    else {
        return Ok(None);
    };
    let snapshot: UnityTestRunSnapshot = serde_json::from_str(message)
        .map_err(|error| format!("failed: Unity Test cancel returned invalid JSON: {error}"))?;
    if let Some(error) = unity_test_cancel_error(&snapshot) {
        return Err(format!("unavailable: {error}"));
    }
    Ok(Some(snapshot))
}

fn unity_test_cancel_dispatch_response(
    response: PipeResponse,
) -> Result<Option<PipeResponse>, String> {
    if let Some(error) = transient_broker_error_from_response(&response) {
        Err(error.to_string())
    } else {
        Ok(Some(response))
    }
}

fn unity_test_cancel_dispatch_error(error: String) -> String {
    if is_transient_broker_error(&error) {
        error
    } else {
        format!("failed: {error}")
    }
}

async fn dispatch_unity_test_cancel(
    project_path: &str,
    status_payload: &str,
) -> Result<Option<PipeResponse>, String> {
    let (acceptance_tx, acceptance_rx) = tokio::sync::oneshot::channel();
    let project_path = project_path.to_string();
    let status_payload = status_payload.to_string();
    let response_task = tokio::spawn(async move {
        send_message_without_timeout_with_acceptance(
            &project_path,
            "unity_test_cancel",
            &status_payload,
            acceptance_tx,
        )
        .await
    });

    match tokio::time::timeout(UNITY_TEST_CANCEL_ACCEPT_TIMEOUT, acceptance_rx).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            return response_task
                .await
                .map_err(|error| format!("failed: cancellation task failed: {error}"))?
                .map_err(unity_test_cancel_dispatch_error)
                .and_then(unity_test_cancel_dispatch_response);
        }
        Err(_) => {
            return Err(
                "failed: Unity native broker did not accept the cancellation request within 5s"
                    .to_string(),
            );
        }
    }

    match tokio::time::timeout(UNITY_TEST_CANCEL_RESPONSE_GRACE, response_task).await {
        Ok(Ok(response)) => response
            .map_err(unity_test_cancel_dispatch_error)
            .and_then(unity_test_cancel_dispatch_response),
        Ok(Err(error)) => Err(format!("failed: cancellation task failed: {error}")),
        Err(_) => Ok(None),
    }
}

fn unity_test_cancel_queued_dialog(
    project_path: &str,
    snapshot: &UnityTestRunSnapshot,
) -> Option<String> {
    dialog::blocked_error(
        project_path,
        "test_run_cancel_queued",
        Some(snapshot.run_id.as_str()),
    )
    .map(|blocked| {
        format!(
            "Unity Test cancellation queued for run {}.\n{}",
            snapshot.run_id, blocked
        )
    })
}

async fn cancel_unity_test_run(
    project_path: &str,
    snapshot: &UnityTestRunSnapshot,
) -> Result<String, String> {
    let status_payload = serde_json::json!({ "run_id": snapshot.run_id }).to_string();
    let mut dispatch_attempt = 1;
    let cancel_response = loop {
        match dispatch_unity_test_cancel(project_path, &status_payload).await {
            Err(error)
                if is_transient_broker_error(&error)
                    && dispatch_attempt < SHORT_MESSAGE_TRANSIENT_RETRY_ATTEMPTS =>
            {
                wait_before_transient_retry(
                    project_path,
                    "cancel Unity Test run",
                    &error,
                    dispatch_attempt,
                )
                .await
                .map_err(|wait_error| format!("failed: {wait_error}"))?;
                dispatch_attempt += 1;
            }
            result => break result?,
        }
    };
    if let Some(response) = cancel_response {
        if let Some(cancelled) = unity_test_cancel_snapshot(&response)? {
            if unity_test_run_is_terminal(&cancelled.status) {
                return Ok(format!(
                    "Unity Test run {} stopped with status {}",
                    snapshot.run_id, cancelled.status
                ));
            }
        }
    }

    if let Some(queued) = unity_test_cancel_queued_dialog(project_path, snapshot) {
        return Ok(queued);
    }

    let cancel_started = Instant::now();
    while cancel_started.elapsed() < UNITY_TEST_CANCEL_CONFIRM_TIMEOUT {
        tokio::time::sleep(UNITY_TEST_STATUS_POLL_INTERVAL).await;
        let remaining = UNITY_TEST_CANCEL_CONFIRM_TIMEOUT.saturating_sub(cancel_started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let response = match send_message_with_timeout(
            project_path,
            "unity_test_status",
            &status_payload,
            remaining.min(Duration::from_secs(5)),
        )
        .await
        {
            Ok(response) => response,
            Err(error) if dialog::is_unity_modal_dialog_blocked_error(&error) => {
                return Ok(unity_test_cancel_queued_dialog(project_path, snapshot)
                    .unwrap_or_else(|| {
                        format!(
                            "Unity Test cancellation queued for run {}. Resolve the Unity modal dialog so cancellation can complete.",
                            snapshot.run_id
                        )
                    }));
            }
            Err(_) => continue,
        };
        if pipe_response_transient_broker_error(&response) {
            continue;
        }
        if !response.ok {
            continue;
        }
        let Ok(cancelled) = serde_json::from_str::<UnityTestRunSnapshot>(
            response.message.as_deref().unwrap_or_default(),
        ) else {
            continue;
        };
        if let Some(error) = unity_test_cancel_error(&cancelled) {
            return Err(format!("unavailable: {error}"));
        }
        if unity_test_run_is_terminal(&cancelled.status) {
            return Ok(format!(
                "Unity Test run {} stopped with status {}",
                snapshot.run_id, cancelled.status
            ));
        }
    }

    Err(format!(
        "failed: run {} did not stop within 30s",
        snapshot.run_id
    ))
}

async fn unity_test_abort_error(
    project_path: &str,
    snapshot: &UnityTestRunSnapshot,
    source_error: &str,
) -> String {
    match cancel_unity_test_run(project_path, snapshot).await {
        Ok(report) if dialog::is_unity_modal_dialog_blocked_error(&report) => report,
        Ok(report) => format!("{}\n{}", source_error.trim_end(), report),
        Err(error) => format!(
            "{}\nUnity Test cancellation {}",
            source_error.trim_end(),
            error
        ),
    }
}

/// Forwards Unity Test progress steps, skipping repeats of the previous step so
/// the 250ms status poll only produces events when the run actually advances.
struct UnityTestProgress {
    report: Option<crate::tool::ToolProgressReporter>,
    last: Option<crate::tool::ToolProgressUpdate>,
}

impl UnityTestProgress {
    fn new(report: Option<crate::tool::ToolProgressReporter>) -> Self {
        Self { report, last: None }
    }

    fn stage(&mut self, title: &str, info: impl Into<String>) {
        self.send(crate::tool::ToolProgressUpdate {
            title: title.to_string(),
            info: info.into(),
            progress: None,
        });
    }

    fn snapshot(&mut self, snapshot: &UnityTestRunSnapshot) {
        self.send(unity_test_progress_update(snapshot));
    }

    fn send(&mut self, update: crate::tool::ToolProgressUpdate) {
        let Some(report) = self.report.as_ref() else {
            return;
        };
        if self.last.as_ref() == Some(&update) {
            return;
        }
        report(update.clone());
        self.last = Some(update);
    }
}

/// `Namespace.Fixture.Method(args)` -> `Fixture.Method(args)`; dots inside the
/// argument list belong to parameter values, not to the type path.
fn unity_test_short_name(full_name: &str) -> &str {
    let full_name = full_name.trim();
    let path_end = full_name.find('(').unwrap_or(full_name.len());
    let path = &full_name[..path_end];
    let start = path
        .rfind('.')
        .and_then(|method_dot| path[..method_dot].rfind('.'))
        .map_or(0, |fixture_dot| fixture_dot + 1);
    &full_name[start..]
}

fn unity_test_progress_update(snapshot: &UnityTestRunSnapshot) -> crate::tool::ToolProgressUpdate {
    let title = match snapshot.status.as_str() {
        "starting" => "Starting Unity tests",
        "cancelling" => "Cancelling Unity tests",
        _ => "Running Unity tests",
    };
    let completed = snapshot.passed + snapshot.failed + snapshot.skipped + snapshot.inconclusive;
    let mut parts = Vec::new();
    if snapshot.total > 0 {
        parts.push(format!("{}/{}", completed.min(snapshot.total), snapshot.total));
    }
    if snapshot.failed > 0 {
        parts.push(format!("{} failed", snapshot.failed));
    }
    let current = unity_test_short_name(&snapshot.current_test);
    if !current.is_empty() {
        parts.push(current.to_string());
    }
    let progress = (snapshot.total > 0 && snapshot.status != "starting")
        .then(|| (completed as f32 / snapshot.total as f32).clamp(0.0, 1.0));
    crate::tool::ToolProgressUpdate {
        title: title.to_string(),
        info: parts.join(" · "),
        progress,
    }
}

async fn wait_for_unity_test_poll_wake(
    dialog_events: &mut tokio::sync::watch::Receiver<u64>,
    cancel_rx: Option<&mut tokio::sync::watch::Receiver<bool>>,
) -> UnityTestPollWake {
    let sleep = tokio::time::sleep(UNITY_TEST_STATUS_POLL_INTERVAL);
    tokio::pin!(sleep);
    match cancel_rx {
        Some(cancel_rx) => {
            tokio::select! {
                biased;
                changed = dialog_events.changed() => {
                    if changed.is_ok() {
                        UnityTestPollWake::DialogChanged
                    } else {
                        UnityTestPollWake::Tick
                    }
                }
                _ = cancel_rx.changed() => UnityTestPollWake::Cancelled,
                _ = &mut sleep => UnityTestPollWake::Tick,
            }
        }
        None => {
            tokio::select! {
                biased;
                changed = dialog_events.changed() => {
                    if changed.is_ok() {
                        UnityTestPollWake::DialogChanged
                    } else {
                        UnityTestPollWake::Tick
                    }
                }
                _ = &mut sleep => UnityTestPollWake::Tick,
            }
        }
    }
}

pub async fn unity_test_run_controlled(
    project_path: &str,
    request: &serde_json::Value,
    timeout: Option<Duration>,
    mut cancel_rx: Option<tokio::sync::watch::Receiver<bool>>,
    progress: Option<crate::tool::ToolProgressReporter>,
) -> Result<UnityTestRunSnapshot, String> {
    let started = Instant::now();
    let mut progress = UnityTestProgress::new(progress);
    require_unity_test_tools_available(project_path)?;
    if cancel_rx
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return Err("Unity Test run cancelled before start".to_string());
    }

    progress.stage("Waiting for Unity Editor", "");
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = tokio::select! {
        biased;
        _ = async {
            match cancel_rx.as_mut() {
                Some(receiver) => { let _ = receiver.wait_for(|cancelled| *cancelled).await; }
                None => std::future::pending::<()>().await,
            }
        } => return Err("Unity Test run cancelled while waiting for the Editor".into()),
        _ = async {
            match timeout {
                Some(limit) => tokio::time::sleep_until(tokio::time::Instant::from_std(started + limit)).await,
                None => std::future::pending::<()>().await,
            }
        } => return Err("Unity Test run timed out while waiting for the Editor".into()),
        guard = op_lock.lock() => guard,
    };
    if cancel_rx
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return Err("Unity Test run cancelled before start".to_string());
    }

    // Recheck the source/state after queueing, before changing Play Mode or
    // starting a run; an earlier check can become stale while another tool runs.
    progress.stage("Preparing Unity tests", "Checking Editor state");
    require_unity_test_sources_converged(project_path).await?;
    ensure_unity_test_start_status(project_path).await?;
    progress.stage(
        "Starting Unity tests",
        request
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    );
    let requested_run_id = uuid::Uuid::new_v4().simple().to_string();
    let start_request = unity_test_start_request(request, &requested_run_id)?;
    let payload = serde_json::to_string(&start_request)
        .map_err(|error| format!("Failed to serialize Unity Test run request: {error}"))?;
    let start = match send_message_with_transient_retry(
        project_path,
        "unity_test_start",
        &payload,
        Duration::from_secs(30),
        "start Unity Test run",
    )
    .await
    {
        Ok(start) => start,
        Err(error)
            if dialog::is_unity_modal_dialog_blocked_error(&error)
                && !error.contains("request_state=not_sent") =>
        {
            let placeholder = UnityTestRunSnapshot {
                run_id: requested_run_id.clone(),
                status: "starting".to_string(),
                ..Default::default()
            };
            return Err(unity_test_abort_error(project_path, &placeholder, &error).await);
        }
        Err(error) => return Err(error),
    };
    if !start.ok {
        return Err(start.error.unwrap_or_else(|| {
            "Unity Test run could not start. Update the Locus Unity plugin and recompile the project."
                .to_string()
        }));
    }

    let mut snapshot: UnityTestRunSnapshot =
        match serde_json::from_str(start.message.as_deref().unwrap_or_default()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let placeholder = UnityTestRunSnapshot {
                    run_id: requested_run_id.clone(),
                    status: "starting".to_string(),
                    ..Default::default()
                };
                let source = format!("Unity Test start returned invalid JSON: {error}");
                return Err(unity_test_abort_error(project_path, &placeholder, &source).await);
            }
        };
    if snapshot.run_id.trim().is_empty() {
        snapshot.run_id = requested_run_id;
        return Err(unity_test_abort_error(
            project_path,
            &snapshot,
            "Unity Test run did not return a run id",
        )
        .await);
    };

    let status_payload = serde_json::json!({ "run_id": snapshot.run_id.clone() }).to_string();
    // The WinEventHook publishes only on native window changes. Waiting on its
    // watch revision adds no polling work during normal test execution and can
    // interrupt the 250ms status cadence as soon as a modal dialog is observed.
    let mut dialog_events = dialog::subscribe();
    if dialog::current_dialog(project_path).is_some() {
        return Err(unity_test_abort_error(
            project_path,
            &snapshot,
            "Unity Test run was interrupted by a Unity modal dialog",
        )
        .await);
    }

    loop {
        if unity_test_run_is_terminal(&snapshot.status) {
            return Ok(snapshot);
        }
        if cancel_rx
            .as_ref()
            .is_some_and(|receiver| *receiver.borrow())
        {
            progress.stage("Cancelling Unity tests", "");
            return match cancel_unity_test_run(project_path, &snapshot).await {
                Ok(report) => Err(report),
                Err(error) => Err(format!("Unity Test cancellation {error}")),
            };
        }
        progress.snapshot(&snapshot);
        if dialog_events.has_changed().unwrap_or(false) {
            let _ = dialog_events.borrow_and_update();
            if dialog::current_dialog(project_path).is_some() {
                return Err(unity_test_abort_error(
                    project_path,
                    &snapshot,
                    "Unity Test run was interrupted by a Unity modal dialog",
                )
                .await);
            }
        }
        if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
            let timeout = timeout.unwrap_or_default();
            let source = format!(
                "Unity Test run {} timed out after {}s (current test: {})",
                snapshot.run_id,
                timeout.as_secs(),
                if snapshot.current_test.is_empty() {
                    "unknown"
                } else {
                    snapshot.current_test.as_str()
                }
            );
            return Err(unity_test_abort_error(project_path, &snapshot, &source).await);
        }

        match wait_for_unity_test_poll_wake(&mut dialog_events, cancel_rx.as_mut()).await {
            UnityTestPollWake::DialogChanged => {
                if dialog::current_dialog(project_path).is_some() {
                    return Err(unity_test_abort_error(
                        project_path,
                        &snapshot,
                        "Unity Test run was interrupted by a Unity modal dialog",
                    )
                    .await);
                }
            }
            UnityTestPollWake::Cancelled => {
                progress.stage("Cancelling Unity tests", "");
                return match cancel_unity_test_run(project_path, &snapshot).await {
                    Ok(report) => Err(report),
                    Err(error) => Err(format!("Unity Test cancellation {error}")),
                };
            }
            UnityTestPollWake::Tick => {}
        }
        let response = match send_message_with_timeout(
            project_path,
            "unity_test_status",
            &status_payload,
            Duration::from_secs(5),
        )
        .await
        {
            Ok(response) => response,
            Err(error) if dialog::is_unity_modal_dialog_blocked_error(&error) => {
                return Err(unity_test_abort_error(project_path, &snapshot, &error).await);
            }
            Err(_) => {
                // Play Mode transitions and tests that explicitly reload the
                // domain briefly drop the managed executor. The persisted run
                // state is available again after the bridge reconnects.
                continue;
            }
        };
        if pipe_response_transient_broker_error(&response) {
            continue;
        }
        if !response.ok {
            let source = response
                .error
                .unwrap_or_else(|| "Unity Test status query failed".to_string());
            return Err(unity_test_abort_error(project_path, &snapshot, &source).await);
        }
        snapshot = match serde_json::from_str(response.message.as_deref().unwrap_or_default()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let source = format!("Unity Test status returned invalid JSON: {error}");
                return Err(unity_test_abort_error(project_path, &snapshot, &source).await);
            }
        };
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

type PendingEditSessionStarts = HashMap<(String, String), tokio::task::JoinHandle<()>>;

fn pending_edit_session_starts() -> &'static StdMutex<PendingEditSessionStarts> {
    static STARTS: OnceLock<StdMutex<PendingEditSessionStarts>> = OnceLock::new();
    STARTS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// File writes must not wait for the Editor's main thread when snapshots are
/// disabled. Retain the start task until cleanup so an end request cannot race
/// ahead of a delayed begin request and leave Auto Refresh suppressed forever.
pub(crate) fn begin_edit_session_in_background(project_path: &str, owner: &str) {
    let key = (crate::agent::workspace_execution_lock::normalize_workspace_path_key(project_path, "."), owner.to_string());
    let mut starts = pending_edit_session_starts().lock().unwrap_or_else(|error| error.into_inner());
    starts.entry(key).or_insert_with(|| {
        let project = project_path.to_string();
        let owner = owner.to_string();
        tokio::spawn(async move {
            if let Err(error) = begin_edit_session(&project, &owner).await {
                eprintln!("[Locus] deferred Unity edit session start failed owner={owner}: {error}");
            }
        })
    });
}

async fn wait_for_edit_session_starts(project_path: &str, owner: &str) {
    let project = crate::agent::workspace_execution_lock::normalize_workspace_path_key(project_path, ".");
    let pending = {
        let mut starts = pending_edit_session_starts().lock().unwrap_or_else(|error| error.into_inner());
        let keys = starts.keys().filter(|(path, id)| path == &project && (owner.is_empty() || id == owner)).cloned().collect::<Vec<_>>();
        keys.into_iter().filter_map(|key| starts.remove(&key)).collect::<Vec<_>>()
    };
    for start in pending { let _ = start.await; }
}

#[cfg(test)]
mod deferred_edit_session_tests {
    use super::*;

    #[tokio::test]
    async fn cleanup_waits_for_its_queued_begin_and_preserves_other_owners() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().to_string_lossy().to_string();
        let key = crate::agent::workspace_execution_lock::normalize_workspace_path_key(&project, ".");
        let (finish, wait) = tokio::sync::oneshot::channel();
        {
            let mut starts = pending_edit_session_starts().lock().unwrap();
            starts.insert((key.clone(), "owner-a".into()), tokio::spawn(async move { wait.await.unwrap(); }));
            starts.insert((key.clone(), "owner-b".into()), tokio::spawn(async {}));
        }
        let mut cleanup = Box::pin(wait_for_edit_session_starts(&project, "owner-a"));
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut cleanup).await.is_err());
        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), cleanup).await.unwrap();
        assert!(pending_edit_session_starts().lock().unwrap().contains_key(&(key.clone(), "owner-b".into())));
        wait_for_edit_session_starts(&project, "").await;
        assert!(!pending_edit_session_starts().lock().unwrap().keys().any(|(project, _)| project == &key));
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
pub(crate) async fn end_edit_session_after_execution(project_path: &str, owner: &str) -> Result<String, String> {
    crate::agent::unity_execution_scope::run(
        project_path, "unity_edit_session_cleanup", &serde_json::json!({}), None, true,
        end_edit_session(project_path, owner),
    ).await.map_err(str::to_string)?
}

pub async fn end_edit_session(project_path: &str, owner: &str) -> Result<String, String> {
    wait_for_edit_session_starts(project_path, owner).await;
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

    crate::agent::unity_execution_scope::run(
        project_path, "unity_import_assets", &serde_json::json!({}), None, true,
        import_assets_inner(project_path, asset_paths),
    ).await.map_err(str::to_string)?
}

async fn import_assets_inner(project_path: &str, asset_paths: &[String]) -> Result<String, String> {

    crate::workspace_changes::record_known_paths(
        Path::new(project_path),
        asset_paths,
        crate::workspace_changes::WorkspaceChangeSource::LocusWrite,
    );

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
        "{\"active\":false,\"title\":\"\",\"info\":\"\",\"progress\":0,\"revision\":0,\"source\":\"\"}".to_string()
    });
    format!(
        "<{tag}>{payload}</{tag}>\n",
        tag = UNITY_EXECUTE_PROGRESS_TAG,
        payload = payload
    )
}

fn rust_unity_execute_progress(
    title: impl Into<String>,
    info: impl Into<String>,
    revision: u64,
) -> UnityExecuteProgressSnapshot {
    UnityExecuteProgressSnapshot {
        active: true,
        title: title.into(),
        info: info.into(),
        progress: 0.0,
        revision,
        source: "rust".to_string(),
        ..Default::default()
    }
}

/// Readable text for a failed progress poll in the waiting status line. A
/// timed-out poll only means Unity's main thread has not served it yet.
fn describe_progress_poll_error(error: &str) -> String {
    match error.trim() {
        "Unity response timed out" | "Unity request timed out" => {
            "Unity's main thread has not picked up the request yet".to_string()
        }
        other => other.to_string(),
    }
}

fn truncate_progress_text(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

/// Fill the status line from liveness fields when the snippet itself did not
/// report anything: how long the main thread has been busy, the last printed
/// line, what an await is waiting for, or why the snippet is stopping.
fn presentable_unity_execute_progress(
    mut snapshot: UnityExecuteProgressSnapshot,
) -> UnityExecuteProgressSnapshot {
    if snapshot.stop_requested == Some(true) && snapshot.source != "await" {
        snapshot.title = "Stopping snippet".to_string();
        snapshot.info = match snapshot.stop_reason.as_str() {
            "idle_timeout" => "no print/ctx.Progress output for 30s".to_string(),
            "timeout" => "timeout budget reached".to_string(),
            "cancelled" => "cancel requested".to_string(),
            other => other.to_string(),
        };
        return snapshot;
    }
    if !snapshot.info.trim().is_empty() {
        return snapshot;
    }
    if snapshot.source == "await" {
        let target = snapshot.wait_target.trim();
        if !target.is_empty() {
            snapshot.info = format!("{target} · {}s", snapshot.waited_ms / 1_000);
        }
        return snapshot;
    }
    if snapshot.executing == Some(true) {
        let mut parts = Vec::new();
        if let Some(blocked) = snapshot.main_thread_blocked_ms.filter(|ms| *ms >= 2_000) {
            parts.push(format!("Unity main thread busy {}s", blocked / 1_000));
        }
        if !snapshot.last_output.trim().is_empty() {
            parts.push(format!(
                "last output: {}",
                truncate_progress_text(&snapshot.last_output, 160)
            ));
        }
        snapshot.info = parts.join("; ");
    }
    snapshot
}

/// What the host learns from Unity's progress snapshots during one call.
#[derive(Default)]
struct UnityExecuteProgressTracker {
    max_main_thread_blocked_ms: u64,
    last_emitted: Option<(u64, String, String, u32)>,
}

impl UnityExecuteProgressTracker {
    fn observe(&mut self, snapshot: &UnityExecuteProgressSnapshot) {
        let current = if snapshot.executing == Some(true) {
            snapshot.main_thread_blocked_ms.unwrap_or(0)
        } else {
            0
        };
        self.max_main_thread_blocked_ms = self
            .max_main_thread_blocked_ms
            .max(snapshot.max_main_thread_blocked_ms.unwrap_or(0))
            .max(current);
    }

    /// Liveness text changes without a new revision, so deduplicate on the
    /// visible fields instead.
    fn should_emit(&mut self, snapshot: &UnityExecuteProgressSnapshot) -> bool {
        let key = (
            snapshot.revision,
            snapshot.title.clone(),
            snapshot.info.clone(),
            (snapshot.progress.clamp(0.0, 1.0) * 1000.0) as u32,
        );
        if self.last_emitted.as_ref() == Some(&key) {
            return false;
        }
        self.last_emitted = Some(key);
        true
    }
}

/// `Some(error)` when the snapshot proves Unity cannot finish the call by
/// itself: the broker still serves the last snapshot, but managed code stopped
/// publishing and the main thread is away (the whole runtime is frozen).
fn unity_execute_frozen_error(
    snapshot: &UnityExecuteProgressSnapshot,
    execution_id: &str,
) -> Option<String> {
    let published_age = snapshot.published_age_ms?;
    let stall = snapshot.main_thread_stall_ms?;
    (published_age >= UNITY_EXECUTE_PUBLISHER_STALE_MS && stall >= UNITY_EXECUTE_MAIN_THREAD_STALL_MS)
        .then(|| {
            format!(
                "native_main_thread_unresponsive; execution_id={execution_id}; request_state=detached; Unity stopped reporting progress {}s ago and its main thread has not returned to the editor loop for {}s, so Locus can no longer track this execution. If this snippet should not finish, stop it with `await locus.cancel_unity_execution(project=..., execution_id=\"{execution_id}\")` (loops in unity_execute code stop at their next iteration); otherwise read its result later with `locus.wait_unity_execution`.",
                published_age / 1_000,
                stall / 1_000
            )
        })
}

fn unity_execute_budget_exceeded_error(execution_id: &str, budget_ms: u64) -> String {
    format!(
        "{UNITY_EXECUTE_BUDGET_EXCEEDED}; execution_id={execution_id}; request_state=detached; the execution ran past its {}s timeout plus {}s and Unity has not returned a result, so it has not reached a cancellation point yet (loops stop at their next iteration). Stop it with `await locus.cancel_unity_execution(project=..., execution_id=\"{execution_id}\")` or read its result later with `locus.wait_unity_execution`.",
        budget_ms / 1_000,
        UNITY_EXECUTE_BUDGET_GRACE.as_secs()
    )
}

async fn query_unity_execute_progress(
    project_path: &str,
    execution_id: &str,
) -> Result<Option<UnityExecuteProgressSnapshot>, String> {
    let started = std::time::Instant::now();
    // Writer-free variant: this poll runs in a `select!` handler on the same
    // task that drives the in-flight execute send future. Waiting on the
    // shared writer lock here would deadlock against that suspended future
    // (it holds the guard mid-write and is not polled while this handler
    // runs) and then tear down the connection under the in-flight request.
    let resp = transport::send_message_if_writer_free(
        project_path,
        "execute_code_progress",
        execution_id,
        Duration::from_secs(2),
    )
    .await
    .map_err(|error| {
        let elapsed_ms = started.elapsed().as_millis();
        eprintln!(
            "[Locus] unity_execute progress poll failed after {}ms: {}",
            elapsed_ms, error
        );
        error
    })?;

    let Some(resp) = resp else {
        // Writer busy — the execute payload is still streaming out on this
        // task; skip this poll instead of contending for the lock.
        return Ok(None);
    };

    if !resp.ok {
        let error = resp
            .error
            .unwrap_or_else(|| "Unity progress response returned ok=false".to_string());
        eprintln!(
            "[Locus] unity_execute progress poll returned error after {}ms: {}",
            started.elapsed().as_millis(),
            error
        );
        return Err(error);
    }

    let message = resp
        .message
        .ok_or_else(|| "Unity progress response missing message".to_string())?;
    let snapshot = serde_json::from_str(&message)
        .map_err(|error| format!("Unity progress response parse failed: {}", error))?;
    let elapsed_ms = started.elapsed().as_millis();
    if elapsed_ms >= 500 {
        eprintln!("[Locus] unity_execute progress poll took {}ms", elapsed_ms);
    }
    Ok(Some(snapshot))
}

/// Reattach to an execution that Locus detached from after detecting a Unity
/// modal dialog. The Unity plugin retains completed responses for its
/// idempotency window, so this retrieves the original result without running
/// the snippet again.
pub async fn wait_unity_execution(
    project_path: &str,
    execution_id: &str,
) -> Result<String, String> {
    let execution_id = execution_id.trim();
    if execution_id.is_empty() {
        return Err("execution_id cannot be empty".to_string());
    }
    let response =
        send_message_without_timeout(project_path, "execute_code_wait", execution_id).await?;
    if response.ok {
        Ok(response.message.unwrap_or_default())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Unity execution wait failed".to_string()))
    }
}

/// Strict checkout-local command readiness probe. A successful pipe status is
/// connection evidence; command readiness additionally requires a loaded
/// managed domain and a usable Unity main thread from the fused state probe.
pub(crate) async fn probe_unity_bridge_readiness(project_path: &str) -> UnityBridgeReadinessProbe {
    let state = unity_semantic_state(project_path).await;
    let detail = format!(
        "phase={}, channel={}, domain={}, mainThread={}, canCallUnityApi={}, action={}",
        state.phase,
        state.channel.control_pipe,
        state.domain.phase,
        state.main_thread.state,
        state.safety.can_call_unity_api,
        state.safety.recommended_action
    );

    if state.phase == "reloading"
        || state.domain.phase == "reloading"
        || state.channel.control_pipe == "reloading"
    {
        return UnityBridgeReadinessProbe {
            state: UnityBridgeReadinessState::Reloading,
            detail,
        };
    }

    let channel_reached_managed_executor =
        matches!(state.channel.control_pipe.as_str(), "ready" | "busy");
    let main_thread_usable = !matches!(state.main_thread.state.as_str(), "hung" | "stalled");
    if channel_reached_managed_executor
        && state.domain.phase == "none"
        && main_thread_usable
        && state.safety.can_call_unity_api
    {
        return UnityBridgeReadinessProbe {
            state: UnityBridgeReadinessState::Ready,
            detail,
        };
    }

    let process_running = state.process.state == "running";
    let connection_observed = channel_reached_managed_executor
        || matches!(
            state.channel.control_pipe.as_str(),
            "starting" | "reloading"
        );
    UnityBridgeReadinessProbe {
        state: if process_running && connection_observed {
            UnityBridgeReadinessState::Connected
        } else if process_running && state.phase == "starting" {
            UnityBridgeReadinessState::Starting
        } else {
            UnityBridgeReadinessState::Degraded
        },
        detail,
    }
}

pub(crate) async fn wait_for_unity_bridge_ready(
    project_path: &str,
    max_wait: Duration,
    context: &str,
) -> Result<(), String> {
    let start = std::time::Instant::now();

    loop {
        let readiness = probe_unity_bridge_readiness(project_path).await;
        if readiness.state == UnityBridgeReadinessState::Ready {
            return Ok(());
        }

        if start.elapsed() > max_wait {
            return Err(format!(
                "Timed out waiting for Unity bridge to become ready {} ({}s): {}",
                context,
                max_wait.as_secs(),
                readiness.detail
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn reconnect_unity_pipe_for_execute(project_path: &str, reason: &str) -> Result<(), String> {
    transport::disconnect_with_reason(project_path, reason).await;
    wait_for_unity_bridge_ready(
        project_path,
        Duration::from_secs(20),
        "after execute pipe reset",
    )
    .await
}

async fn reconnect_unity_pipe_for_execute_cancellable(
    project_path: &str,
    reason: &str,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let reconnect = reconnect_unity_pipe_for_execute(project_path, reason);
    tokio::pin!(reconnect);
    loop {
        tokio::select! {
            result = &mut reconnect => return result,
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
            }
        }
    }
}

fn append_execute_reconnect_result(reason: &str, reconnect: Result<(), String>) -> String {
    match reconnect {
        Ok(()) => format!("{}; Unity pipe reconnected.", reason),
        Err(error) => format!("{}; Unity pipe reconnect failed: {}", reason, error),
    }
}

pub async fn cancel_unity_execute_code(
    project_path: &str,
    execution_id: &str,
) -> Result<String, String> {
    let resp = send_message_with_timeout(
        project_path,
        "cancel_execute_code",
        execution_id,
        Duration::from_secs(5),
    )
    .await?;

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "cancel_execute_code failed".to_string()))
    }
}

pub async fn refresh_unity_type_index(
    project_path: &str,
) -> Result<Arc<crate::unity_type_index::UnityTypeIndex>, String> {
    // TI-B: build the base index from reference metadata in the sidecar —
    // no AppDomain reflection sweep, no multi-MB pipe payload. The Unity
    // export below stays as the always-available degradation path (and the
    // source that includes in-memory skill-package assemblies).
    if crate::csharp_compile::is_enabled() {
        match sidecar_type_index(project_path).await {
            Ok(index) => return Ok(index),
            Err(reason) => {
                eprintln!(
                    "[Locus] sidecar type index unavailable; using the Unity export: {reason}"
                );
            }
        }
    }

    let resp = send_message_with_transient_retry(
        project_path,
        "export_type_index",
        "",
        Duration::from_secs(30),
        "while exporting the Unity type index",
    )
    .await?;

    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "export_type_index failed".to_string()));
    }

    let message = resp.message.unwrap_or_default();
    crate::unity_type_index::persist_exported_type_index(project_path, &message).await
}

/// TI-B path: sidecar-built entry set keyed by the Unity-side fingerprint
/// (one cheap pipe roundtrip — TI-A moved it off the editor main thread).
async fn sidecar_type_index(
    project_path: &str,
) -> Result<Arc<crate::unity_type_index::UnityTypeIndex>, String> {
    let params = sidecar_compile_params(project_path).await?;
    let fingerprint = current_unity_type_index_fingerprint(project_path).await?;
    let types = crate::csharp_compile::index_types(&params).await?;

    // A Unity project's reference set always carries thousands of public
    // types (UnityEngine alone); a tiny result means a broken reference
    // set — fail over to the Unity export rather than degrade auto-usings.
    if types.len() < 100 {
        return Err(format!(
            "suspiciously small sidecar type index ({} entries)",
            types.len()
        ));
    }

    crate::unity_type_index::persist_sidecar_type_index(project_path, fingerprint, types).await
}

pub struct UnityTypeIndexUpdateResult {
    pub mode: String,
}

async fn current_unity_type_index_fingerprint(project_path: &str) -> Result<String, String> {
    let resp = send_message_with_transient_retry(
        project_path,
        "export_type_index_fingerprint",
        "",
        Duration::from_secs(10),
        "while refreshing the Unity type-index fingerprint",
    )
    .await?;

    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "export_type_index_fingerprint failed".to_string()));
    }

    let message = resp.message.unwrap_or_default();
    crate::unity_type_index::parse_exported_type_index_fingerprint(&message)
}

async fn cached_unity_type_index_is_current(
    project_path: &str,
    index: &crate::unity_type_index::UnityTypeIndex,
) -> Result<bool, String> {
    let current_fingerprint = current_unity_type_index_fingerprint(project_path).await?;
    Ok(!index.fingerprint.is_empty() && index.fingerprint == current_fingerprint)
}

pub async fn ensure_unity_type_index_current(
    project_path: &str,
) -> Result<UnityTypeIndexUpdateResult, String> {
    match crate::unity_type_index::load_cached_type_index(project_path).await {
        Ok(Some(index)) if cached_unity_type_index_is_current(project_path, &index).await? => {
            Ok(UnityTypeIndexUpdateResult {
                mode: "current".to_string(),
            })
        }
        Ok(Some(_)) | Ok(None) => {
            refresh_unity_type_index(project_path).await?;
            Ok(UnityTypeIndexUpdateResult {
                mode: "full".to_string(),
            })
        }
        Err(error) => Err(error),
    }
}

pub async fn update_unity_type_index_after_skill_package_compile(
    project_path: &str,
    compile_response: &serde_json::Value,
) -> Result<UnityTypeIndexUpdateResult, String> {
    let package_id = compile_response
        .get("packageId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let source_hash = compile_response
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let assembly_id = compile_response
        .get("assemblyId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let previous_assembly_id = compile_response
        .get("previousAssemblyId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let previous_fingerprint = compile_response
        .get("previousTypeIndexFingerprint")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let current_fingerprint = compile_response
        .get("typeIndexFingerprint")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let types = serde_json::from_value::<Vec<crate::unity_type_index::UnityTypeIndexEntry>>(
        compile_response
            .get("types")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|error| format!("Failed to parse Skill package type index delta: {}", error))?;

    if package_id.is_empty() || source_hash.is_empty() || assembly_id.is_empty() {
        refresh_unity_type_index(project_path).await?;
        return Ok(UnityTypeIndexUpdateResult {
            mode: "full".to_string(),
        });
    }

    let cached = crate::unity_type_index::load_cached_type_index(project_path).await?;
    if let Some(index) = cached.as_ref() {
        if !current_fingerprint.is_empty() && index.fingerprint == current_fingerprint {
            return Ok(UnityTypeIndexUpdateResult {
                mode: "current".to_string(),
            });
        }
    }

    if !previous_fingerprint.is_empty() && !current_fingerprint.is_empty() {
        if let Some(index) = cached.as_ref() {
            if index.fingerprint == previous_fingerprint {
                if crate::unity_type_index::persist_skill_package_type_index_delta(
                    project_path,
                    previous_fingerprint,
                    current_fingerprint,
                    package_id,
                    source_hash,
                    assembly_id,
                    previous_assembly_id,
                    types,
                )
                .await?
                .is_some()
                {
                    return Ok(UnityTypeIndexUpdateResult {
                        mode: "incremental".to_string(),
                    });
                }
            }
        }
    }

    refresh_unity_type_index(project_path).await?;
    Ok(UnityTypeIndexUpdateResult {
        mode: "full".to_string(),
    })
}

async fn unity_type_index_for_execute(
    project_path: &str,
) -> Option<Arc<crate::unity_type_index::UnityTypeIndex>> {
    match crate::unity_type_index::load_cached_type_index(project_path).await {
        Ok(Some(index)) => match cached_unity_type_index_is_current(project_path, &index).await {
            Ok(true) => return Some(index),
            Ok(false) => {
                eprintln!("[Locus] Unity type index cache is stale; refreshing.");
                crate::unity_type_index::invalidate_cached_type_index(project_path).await;
            }
            Err(error) => {
                eprintln!(
                    "[Locus] Unity type index cache validation failed; refreshing: {}",
                    error
                );
                crate::unity_type_index::invalidate_cached_type_index(project_path).await;
            }
        },
        Ok(None) => {}
        Err(error) => eprintln!("[Locus] Unity type index cache ignored: {}", error),
    }

    let refresh_started = std::time::Instant::now();
    match refresh_unity_type_index(project_path).await {
        Ok(index) => {
            eprintln!(
                "[Locus] Unity type index refreshed in {}ms",
                refresh_started.elapsed().as_millis()
            );
            Some(index)
        }
        Err(error) => {
            eprintln!(
                "[Locus] Unity type index export skipped after {}ms: {}",
                refresh_started.elapsed().as_millis(),
                error
            );
            None
        }
    }
}

async fn prepare_unity_execute_code_for_send(
    project_path: &str,
    code: &str,
) -> crate::unity_type_index::PreparedUnityCode {
    let index = unity_type_index_for_execute(project_path).await;
    crate::unity_type_index::prepare_unity_execute_code(code, index.as_deref())
}

// ── compile-server sidecar path (unity_execute / unity_run_states) ───

/// Outcome of attempting the sidecar compile for an execute/run_states call.
enum SidecarCompileAttempt {
    /// Compiled: ship `payload` via the `*_loaded` pipe message.
    Compiled { payload: String },
    /// Deterministic compile/validation failure — surface to the agent
    /// directly (both compile paths accept the same C#9 input, so the
    /// legacy path would fail identically).
    CompileError(String),
    /// Sidecar infrastructure unavailable — use the legacy in-Unity path.
    Unavailable(String),
}

fn production_non_public_access_mode(
    enabled: bool,
) -> Option<crate::csharp_compile::NonPublicAccessProbeMode> {
    enabled.then_some(crate::csharp_compile::NonPublicAccessProbeMode::SkipVerification)
}

#[derive(Clone)]
struct CachedRunStatesAssembly {
    payload: String,
    inserted_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunStatesCompileCacheMode {
    Store,
    Consume,
}

const RUN_STATES_COMPILE_CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const RUN_STATES_COMPILE_CACHE_MAX: usize = 16;

fn run_states_compile_cache() -> &'static StdMutex<HashMap<String, CachedRunStatesAssembly>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, CachedRunStatesAssembly>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn run_states_compile_cache_key(
    project_path: &str,
    params: &crate::csharp_compile::CompileParams,
    prepared_request: &serde_json::Value,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
) -> Option<String> {
    let request_bytes = serde_json::to_vec(prepared_request).ok()?;
    Some(format!(
        "{}\n{}\n{}\nnon-public-access={}\n{}",
        project_runtime_key(project_path),
        params.fingerprint,
        params.domain_generation,
        non_public_access_probe_mode
            .map(crate::csharp_compile::NonPublicAccessProbeMode::as_str)
            .unwrap_or("none"),
        sha256_hex(&request_bytes)
    ))
}

fn prune_run_states_compile_cache(cache: &mut HashMap<String, CachedRunStatesAssembly>) {
    let now = unix_now_ms();
    cache.retain(|_, entry| {
        now.saturating_sub(entry.inserted_at_ms) <= RUN_STATES_COMPILE_CACHE_TTL_MS
    });
    if cache.len() <= RUN_STATES_COMPILE_CACHE_MAX {
        return;
    }
    let mut entries: Vec<(String, u64)> = cache
        .iter()
        .map(|(key, entry)| (key.clone(), entry.inserted_at_ms))
        .collect();
    entries.sort_by_key(|(_, inserted_at)| *inserted_at);
    let remove_count = cache.len().saturating_sub(RUN_STATES_COMPILE_CACHE_MAX);
    for (key, _) in entries.into_iter().take(remove_count) {
        cache.remove(&key);
    }
}

fn take_cached_run_states_compile(key: &str) -> Option<CachedRunStatesAssembly> {
    let mut cache = run_states_compile_cache().lock().ok()?;
    prune_run_states_compile_cache(&mut cache);
    cache.remove(key)
}

fn store_cached_run_states_compile(key: String, entry: CachedRunStatesAssembly) {
    if let Ok(mut cache) = run_states_compile_cache().lock() {
        prune_run_states_compile_cache(&mut cache);
        cache.insert(key, entry);
        prune_run_states_compile_cache(&mut cache);
    }
}

const SNIPPET_ENTRY_TYPE_FALLBACK: &str = "Locus.RuntimeSnippets.__LocusAsyncSnippetHost";
const RUN_STATES_ENTRY_TYPE_FALLBACK: &str = "Locus.RuntimeStateMachines.__LocusRunStatesHost";

fn unity_plugin_lacks_message(resp: &PipeResponse) -> bool {
    !resp.ok
        && resp
            .error
            .as_deref()
            .map(|error| error.starts_with("unknown message type"))
            .unwrap_or(false)
}

async fn sidecar_compile_params(
    project_path: &str,
) -> Result<crate::csharp_compile::CompileParams, String> {
    if !crate::csharp_compile::is_enabled() {
        return Err("sidecar compiler disabled".to_string());
    }
    // While a recompile is in flight Unity is rewriting ScriptAssemblies;
    // let those calls take the legacy path instead of racing the file set.
    if unity_recompile_waiting(project_path) {
        return Err("unity recompile in progress".to_string());
    }
    crate::csharp_compile::params::get_params(project_path).await
}

/// Compile a prepared unity_execute snippet in the sidecar. Error texts
/// mirror the Unity-side `HandleExecuteCode` wording exactly ("async snippet
/// compilation exception: " + the combined two-mode compile error).
/// Map a sidecar "unavailable" (sidecar down / transport error) to either a
/// graceful in-Unity fallback (`Unavailable`) or a hard error (`CompileError`)
/// when the operator disabled the in-process fallback (pure-sidecar / A-B).
fn sidecar_unavailable(reason: String) -> SidecarCompileAttempt {
    if crate::csharp_compile::block_in_process_fallback() {
        SidecarCompileAttempt::CompileError(format!(
            "sidecar compile unavailable and in-process fallback disabled: {reason}"
        ))
    } else {
        SidecarCompileAttempt::Unavailable(reason)
    }
}

async fn sidecar_compile_for_execute(
    project_path: &str,
    prepared_code: &str,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
) -> SidecarCompileAttempt {
    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_unavailable(reason),
    };

    let compile_started = std::time::Instant::now();
    match crate::csharp_compile::compile_snippet_with_access_probe(
        &params,
        prepared_code,
        false,
        false,
        non_public_access_probe_mode,
    )
    .await
    {
        Ok(Ok(assembly)) => {
            let assembly_b64 = assembly.assembly_b64;
            let assembly_path = assembly.assembly_path;
            let entry_type = assembly
                .entry_type
                .unwrap_or_else(|| SNIPPET_ENTRY_TYPE_FALLBACK.to_string());
            eprintln!(
                "[CsharpCompile] snippet compiled in {}ms ({} KB, mode {})",
                compile_started.elapsed().as_millis(),
                assembly_b64.len() / 1024,
                assembly.mode.as_deref().unwrap_or("?")
            );
            let mut payload = serde_json::json!({
                "entry_type": entry_type,
            });
            if let Some(object) = payload.as_object_mut() {
                if let Some(path) = assembly_path {
                    object.insert("assembly_path".to_string(), serde_json::Value::String(path));
                } else {
                    object.insert(
                        "assembly_b64".to_string(),
                        serde_json::Value::String(assembly_b64),
                    );
                }
            }
            let payload = payload.to_string();
            SidecarCompileAttempt::Compiled { payload }
        }
        Ok(Err(failure)) => {
            eprintln!(
                "[CsharpCompile] snippet compile diagnostics in {}ms (stage {})",
                compile_started.elapsed().as_millis(),
                failure.stage
            );
            SidecarCompileAttempt::CompileError(format!(
                "async snippet compilation exception: {}",
                failure.message
            ))
        }
        Err(error) => sidecar_unavailable(error),
    }
}

async fn prepare_unity_run_states_request_for_send(
    project_path: &str,
    request: &serde_json::Value,
) -> crate::unity_type_index::PreparedUnityRunStatesRequest {
    let index = unity_type_index_for_execute(project_path).await;
    crate::unity_type_index::prepare_unity_run_states_request(request, index.as_deref())
}

const UNITY_EXECUTE_EXECUTION_ID_MARKER: &str = "//__LOCUS_EXECUTION_ID__:";

fn unity_execute_code_with_markers(
    code: &str,
    execution_id: &str,
    timeout_ms: Option<u64>,
) -> String {
    let mut prepared = format!(
        "{}\n{}{}",
        code.trim_end(),
        UNITY_EXECUTE_EXECUTION_ID_MARKER,
        execution_id
    );
    if let Some(timeout_ms) = timeout_ms {
        prepared.push('\n');
        prepared.push_str(UNITY_EXECUTE_TIMEOUT_MARKER);
        prepared.push_str(&timeout_ms.to_string());
    }
    prepared
}

fn unity_execute_loaded_payload_with_execution_id(
    payload: String,
    execution_id: &str,
    source_code: &str,
    timeout_ms: Option<u64>,
) -> Result<String, String> {
    let mut value: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|error| format!("Failed to parse execute_loaded payload: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "execute_loaded payload is not an object".to_string())?;
    object.insert(
        "execution_id".to_string(),
        serde_json::Value::String(execution_id.to_string()),
    );
    object.insert(
        "source_code".to_string(),
        serde_json::Value::String(source_code.to_string()),
    );
    if let Some(timeout_ms) = timeout_ms {
        object.insert("timeout_ms".to_string(), serde_json::Value::from(timeout_ms));
    }
    serde_json::to_string(&value)
        .map_err(|error| format!("Failed to serialize execute_loaded payload: {error}"))
}

pub async fn unity_execute_code_with_progress<F>(
    project_path: &str,
    code: &str,
    on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    unity_execute_code_with_progress_mode(project_path, code, None, false, on_progress).await
}

pub async fn unity_execute_code_with_progress_non_public_access<F>(
    project_path: &str,
    code: &str,
    enable_non_public_access: bool,
    on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    unity_execute_code_with_progress_mode(
        project_path,
        code,
        production_non_public_access_mode(enable_non_public_access),
        false,
        on_progress,
    )
    .await
}

/// Integration-test-only direct-access probe. The sidecar suppresses
/// compile-time accessibility and the standard execute_loaded pipeline then
/// measures whether Unity Mono can JIT and execute the emitted operation.
pub(crate) async fn unity_execute_code_with_access_probe(
    project_path: &str,
    code: &str,
    mode: crate::csharp_compile::NonPublicAccessProbeMode,
) -> Result<String, String> {
    unity_execute_code_with_progress_mode(project_path, code, Some(mode), true, |_| {}).await
}

async fn unity_execute_code_with_progress_mode<F>(
    project_path: &str,
    code: &str,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
    require_sidecar: bool,
    on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    // Never cancelled; the sender must outlive the call or `changed()` would
    // report a cancellation.
    let (_never_cancelled, cancel_rx) = tokio::sync::watch::channel(false);
    unity_execute_code_core(
        project_path,
        code,
        non_public_access_probe_mode,
        require_sidecar,
        None,
        cancel_rx,
        on_progress,
    )
    .await
    .map(|outcome| outcome.output)
}

pub async fn unity_execute_code_with_progress_cancellable<F>(
    project_path: &str,
    code: &str,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    unity_execute_code_core(project_path, code, None, false, None, cancel_rx, on_progress)
        .await
        .map(|outcome| outcome.output)
}

pub async fn unity_execute_code_with_progress_cancellable_non_public_access<F>(
    project_path: &str,
    code: &str,
    enable_non_public_access: bool,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    unity_execute_code_with_options(
        project_path,
        code,
        UnityExecuteOptions {
            enable_non_public_access,
            timeout_ms: None,
        },
        cancel_rx,
        on_progress,
    )
    .await
    .map(|outcome| outcome.output)
}

/// Agent-facing execution: explicit run budget, cancellation, and what the
/// host observed (for result notes).
pub async fn unity_execute_code_with_options<F>(
    project_path: &str,
    code: &str,
    options: UnityExecuteOptions,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    on_progress: F,
) -> Result<UnityExecuteOutcome, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    unity_execute_code_core(
        project_path,
        code,
        production_non_public_access_mode(options.enable_non_public_access),
        false,
        options.timeout_ms,
        cancel_rx,
        on_progress,
    )
    .await
}

async fn unity_execute_code_core<F>(
    project_path: &str,
    code: &str,
    non_public_access_probe_mode: Option<crate::csharp_compile::NonPublicAccessProbeMode>,
    require_sidecar: bool,
    timeout_ms: Option<u64>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    mut on_progress: F,
) -> Result<UnityExecuteOutcome, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    if *cancel_rx.borrow() {
        return Err(UNITY_EXECUTE_CANCELLED.to_string());
    }

    let timeout_ms = timeout_ms.map(clamp_unity_execute_timeout_ms);
    let execution_id = format!("exec-{}", uuid::Uuid::new_v4().simple());
    let mut rust_progress_revision = 1u64;
    on_progress(rust_unity_execute_progress(
        "Waiting for Locus Unity operation lock",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let op_lock = project_unity_op_lock(project_path).await;
    let guard = tokio::select! {
        guard = op_lock.lock() => guard,
        _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
    };

    on_progress(rust_unity_execute_progress(
        "Preparing Unity type index",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let prepared = tokio::select! {
        prepared = prepare_unity_execute_code_for_send(project_path, code) => prepared,
        _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
    };
    let prepared_code =
        unity_execute_code_with_markers(&prepared.code, &execution_id, timeout_ms);

    let mut execute_msg_type = "execute_code";
    let mut execute_payload = prepared_code.clone();
    if crate::csharp_compile::is_enabled() {
        on_progress(rust_unity_execute_progress(
            "Compiling snippet in compile server",
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;
        let attempt = tokio::select! {
            attempt = sidecar_compile_for_execute(
                project_path,
                &prepared_code,
                non_public_access_probe_mode,
            ) => attempt,
            _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
        };
        match attempt {
            SidecarCompileAttempt::Compiled { payload } => {
                let payload = unity_execute_loaded_payload_with_execution_id(
                    payload,
                    &execution_id,
                    &prepared_code,
                    timeout_ms,
                )?;
                on_progress(rust_unity_execute_progress(
                    "Compile server returned snippet assembly",
                    format!("{} bytes execute_loaded payload", payload.len()),
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                execute_msg_type = "execute_loaded";
                execute_payload = payload;
            }
            SidecarCompileAttempt::CompileError(message) => {
                return Err(crate::unity_type_index::append_auto_using_notes(
                    message, &prepared,
                ));
            }
            SidecarCompileAttempt::Unavailable(reason) => {
                if require_sidecar {
                    return Err(format!(
                        "non-public access probe requires the sidecar compiler: {reason}"
                    ));
                }
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    } else if require_sidecar {
        return Err("non-public access probe requires the sidecar compiler".to_string());
    }

    // Only preparation and compilation are serialized. Once Unity owns a
    // request-scoped execution id, frame-spanning awaits no longer occupy the
    // project-wide operation lock.
    drop(guard);

    // Unity enforces the run budget itself and answers with a timeout error;
    // this backstop only detaches when Unity cannot even do that.
    let budget_ms = timeout_ms.unwrap_or(UNITY_EXECUTE_DEFAULT_TIMEOUT_MS);
    let backstop = Duration::from_millis(budget_ms) + UNITY_EXECUTE_BUDGET_GRACE;
    let run_started_at = std::time::Instant::now();
    let mut tracker = UnityExecuteProgressTracker::default();

    let mut send_attempt = 1u32;
    let mut reattach_attempts = 0u32;
    let resp = loop {
        on_progress(rust_unity_execute_progress(
            if execute_msg_type == "execute_code_wait" {
                "Reattaching to the original Unity execution".to_string()
            } else if send_attempt == 1 {
                format!("Sending {execute_msg_type} to Unity")
            } else {
                format!("Retrying {execute_msg_type} after Unity pipe reconnect")
            },
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;

        // Owned per-attempt copy: the pinned send future must not borrow
        // `execute_payload`, which the old-plugin fallback arm reassigns.
        let attempt_payload = execute_payload.clone();
        eprintln!(
            "[Locus] unity_execute sending {} ({} bytes, attempt {})",
            execute_msg_type,
            attempt_payload.len(),
            send_attempt
        );
        let (acceptance_tx, mut acceptance_rx) = tokio::sync::oneshot::channel();
        let execute = send_message_without_timeout_with_acceptance(
            project_path,
            execute_msg_type,
            &attempt_payload,
            acceptance_tx,
        );
        tokio::pin!(execute);

        let mut progress_tick =
            tokio::time::interval(Duration::from_millis(UNITY_EXECUTE_PROGRESS_POLL_MS));
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut saw_unity_progress = false;
        let execute_started_at = std::time::Instant::now();
        let mut last_waiting_status_at = execute_started_at;
        let mut last_progress_poll_error: Option<String> = None;
        let mut progress_unavailable_since: Option<std::time::Instant> = None;
        let mut broker_accepted = false;
        let mut acceptance_pending = true;

        let attempt_result: Result<PipeResponse, String> = loop {
            tokio::select! {
                biased;
                accepted = &mut acceptance_rx, if acceptance_pending => {
                    acceptance_pending = false;
                    if accepted.is_ok() {
                        broker_accepted = true;
                        eprintln!(
                            "[Locus] native Broker accepted {} for execution_id={}",
                            execute_msg_type, execution_id
                        );
                        on_progress(rust_unity_execute_progress(
                            "Broker accepted Unity execute request",
                            format!("{}; execution_id={}", execute_msg_type, execution_id),
                            rust_progress_revision,
                        ));
                        rust_progress_revision += 1;
                    }
                },
                result = &mut execute => break result,
                changed = cancel_rx.changed() => {
                    let cancelled = changed.is_err() || *cancel_rx.borrow();
                    if !cancelled {
                        continue;
                    }

                    if let Err(error) = cancel_unity_execute_code(project_path, &execution_id).await {
                        eprintln!("[Locus] cancel_execute_code skipped: {}", error);
                    }

                    let drain = tokio::time::sleep(Duration::from_secs(5));
                    tokio::pin!(drain);
                    loop {
                        tokio::select! {
                            result = &mut execute => {
                                if let Err(error) = result {
                                    eprintln!("[Locus] execute_code after cancel ended with transport error: {}", error);
                                }
                                break;
                            },
                            _ = &mut drain => {
                                eprintln!("[Locus] execute_code cancel drain timed out");
                                transport::disconnect_with_reason(
                                    project_path,
                                    "execute_code cancel drain timed out",
                                ).await;
                                break;
                            },
                            _ = progress_tick.tick() => {
                                if let Ok(Some(snapshot)) = query_unity_execute_progress(project_path, &execution_id).await {
                                    let snapshot = presentable_unity_execute_progress(snapshot);
                                    if tracker.should_emit(&snapshot) {
                                        on_progress(snapshot);
                                    }
                                }
                            }
                        }
                    }

                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                },
                _ = progress_tick.tick() => {
                    match query_unity_execute_progress(project_path, &execution_id).await {
                        Ok(Some(snapshot)) => {
                            last_progress_poll_error = None;
                            progress_unavailable_since = None;
                            if snapshot.active {
                                if !saw_unity_progress {
                                    eprintln!(
                                        "[Locus] unity_execute first Unity progress after {}ms",
                                        execute_started_at.elapsed().as_millis()
                                    );
                                }
                                saw_unity_progress = true;
                            }
                            tracker.observe(&snapshot);
                            if let Some(error) = unity_execute_frozen_error(&snapshot, &execution_id) {
                                break Err(error);
                            }
                            let snapshot = presentable_unity_execute_progress(snapshot);
                            if tracker.should_emit(&snapshot) {
                                on_progress(snapshot);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            // Only brokers that cannot serve progress without
                            // the main thread get here while a snippet holds it.
                            if error.contains("native_main_thread_unresponsive") {
                                break Err(format!("native_main_thread_unresponsive; execution_id={execution_id}; request_state=detached; execution has not been confirmed stopped. If this snippet should not finish, stop it with `await locus.cancel_unity_execution(project=..., execution_id=\"{execution_id}\")` (loops in unity_execute code stop at their next iteration); otherwise read its result later with `locus.wait_unity_execution`."));
                            }
                            last_progress_poll_error = Some(error);
                            if saw_unity_progress {
                                let unavailable_since = progress_unavailable_since
                                    .get_or_insert_with(std::time::Instant::now);
                                if unavailable_since.elapsed()
                                    > Duration::from_secs(UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS)
                                {
                                    let reason = format!(
                                        "Unity execute progress was unavailable for {}s; reconnecting Unity pipe",
                                        UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS
                                    );
                                    break Err(reason);
                                }
                            }
                        }
                    }

                    if !saw_unity_progress
                        && last_waiting_status_at.elapsed()
                            >= Duration::from_millis(UNITY_EXECUTE_WAITING_STATUS_INTERVAL_MS)
                    {
                        let elapsed_ms = execute_started_at.elapsed().as_millis();
                        let detail = last_progress_poll_error
                            .as_deref()
                            .map(describe_progress_poll_error)
                            .unwrap_or_else(|| "no active Unity execute progress yet".to_string());
                        eprintln!(
                            "[Locus] unity_execute still waiting for Unity progress after {}ms while sending {}: {}",
                            elapsed_ms, execute_msg_type, detail
                        );
                        on_progress(rust_unity_execute_progress(
                            if broker_accepted {
                                "Waiting for Unity main thread after Broker acceptance".to_string()
                            } else {
                                format!("Waiting for Unity progress after sending {execute_msg_type}")
                            },
                            format!("{}s elapsed; {}", elapsed_ms / 1_000, detail),
                            rust_progress_revision,
                        ));
                        rust_progress_revision += 1;
                        last_waiting_status_at = std::time::Instant::now();
                    }

                    if !broker_accepted
                        && !saw_unity_progress
                        && execute_started_at.elapsed()
                            > Duration::from_secs(UNITY_EXECUTE_START_TIMEOUT_SECS)
                    {
                        eprintln!(
                            "[Locus] unity_execute saw no Unity progress within {}s after sending {}; resetting pipe",
                            UNITY_EXECUTE_START_TIMEOUT_SECS, execute_msg_type
                        );
                        break Err(format!(
                            "Unity execute did not leave the sending stage within {}s",
                            UNITY_EXECUTE_START_TIMEOUT_SECS
                        ));
                    }

                    if run_started_at.elapsed() > backstop {
                        break Err(unity_execute_budget_exceeded_error(&execution_id, budget_ms));
                    }
                }
            }
        };

        match attempt_result {
            // The user restarted the hung Editor: the execution is gone with
            // it, so never reattach to the replacement process.
            Err(error) if hang_diagnosis::is_user_restart_interrupt(&error) => return Err(error),
            Err(error)
                if error.contains("native_main_thread_unresponsive")
                    || error.starts_with(UNITY_EXECUTE_BUDGET_EXCEEDED) =>
            {
                return Err(hang_diagnosis::append_to_unresponsive_error(project_path, error).await)
            }
            Err(error) if dialog::is_unity_modal_dialog_blocked_error(&error) => {
                let request_state = if broker_accepted || saw_unity_progress {
                    "detached"
                } else if error.contains("request_state=not_sent") {
                    "not_sent"
                } else {
                    "unknown"
                };
                return Err(dialog::blocked_error(
                    project_path,
                    request_state,
                    Some(&execution_id),
                )
                .unwrap_or(error));
            }
            // An older Unity plugin without the execute_loaded handler:
            // retry the same request through the legacy compile path.
            Ok(resp)
                if execute_msg_type == "execute_loaded" && unity_plugin_lacks_message(&resp) =>
            {
                if require_sidecar {
                    return Err(
                        "non-public access probe requires a Unity plugin with execute_loaded support"
                            .to_string(),
                    );
                }
                crate::csharp_compile::note_fallback(
                    "Unity plugin lacks execute_loaded; update the Locus Unity plugin",
                );
                execute_msg_type = "execute_code";
                execute_payload = prepared_code.clone();
            }
            // An older broker reclaims long requests after 10 minutes while
            // the execution keeps running; the managed side still holds its
            // result under the execution id.
            Ok(resp)
                if pipe_response_is_request_deadline(&resp)
                    && (broker_accepted || saw_unity_progress)
                    && reattach_attempts < UNITY_EXECUTE_REATTACH_MAX_ATTEMPTS =>
            {
                on_progress(rust_unity_execute_progress(
                    "Reattaching to the original Unity execution",
                    "the native broker request deadline expired",
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                execute_msg_type = "execute_code_wait";
                execute_payload = execution_id.clone();
                send_attempt += 1;
                reattach_attempts += 1;
            }
            Ok(resp)
                if pipe_response_transient_broker_error(&resp)
                    && !broker_accepted
                    && !saw_unity_progress
                    && send_attempt == 1 =>
            {
                let error = resp
                    .error
                    .unwrap_or_else(|| "native broker managed executor unavailable".to_string());
                on_progress(rust_unity_execute_progress(
                    "Reconnecting Unity pipe",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                if let Err(reconnect_error) = reconnect {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Ok(resp) => break resp,
            Err(error)
                if (broker_accepted || saw_unity_progress)
                    && reattach_attempts < UNITY_EXECUTE_REATTACH_MAX_ATTEMPTS =>
            {
                on_progress(rust_unity_execute_progress(
                    "Reconnecting to the original Unity execution",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                if let Err(reconnect_error) = reconnect {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                execute_msg_type = "execute_code_wait";
                execute_payload = execution_id.clone();
                send_attempt += 1;
                reattach_attempts += 1;
            }
            Err(error) if !broker_accepted && !saw_unity_progress && send_attempt == 1 => {
                on_progress(rust_unity_execute_progress(
                    "Reconnecting Unity pipe",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                if let Err(reconnect_error) = reconnect {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Err(error) => {
                let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                    project_path,
                    &error,
                    &mut cancel_rx,
                )
                .await;
                if reconnect
                    .as_ref()
                    .err()
                    .map(|error| error == UNITY_EXECUTE_CANCELLED)
                    .unwrap_or(false)
                {
                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                }
                return Err(append_execute_reconnect_result(&error, reconnect));
            }
        }
    };

    if resp.ok {
        Ok(UnityExecuteOutcome {
            output: resp.message.unwrap_or_default(),
            max_main_thread_blocked_ms: tracker.max_main_thread_blocked_ms,
        })
    } else {
        let error = resp.error.unwrap_or_else(|| "unknown error".to_string());
        if error.contains("native_main_thread_unresponsive") {
            return Err(hang_diagnosis::append_to_unresponsive_error(project_path, error).await);
        }
        Err(crate::unity_type_index::append_auto_using_notes(error, &prepared))
    }
}

fn pipe_response_is_request_deadline(response: &PipeResponse) -> bool {
    !response.ok && response.error.as_deref().map(str::trim) == Some("native_request_timed_out")
}

pub async fn unity_execute_code(project_path: &str, code: &str) -> Result<String, String> {
    unity_execute_code_with_progress(project_path, code, |_| {}).await
}

pub async fn unity_execute_code_with_non_public_access(
    project_path: &str,
    code: &str,
    enable_non_public_access: bool,
) -> Result<String, String> {
    unity_execute_code_with_progress_non_public_access(
        project_path,
        code,
        enable_non_public_access,
        |_| {},
    )
    .await
}

async fn wait_for_unity_bridge_ready_after_recompile(project_path: &str) -> Result<(), String> {
    wait_for_unity_bridge_ready(project_path, Duration::from_secs(30), "after recompile").await
}

async fn refresh_unity_type_index_after_recompile(project_path: &str) -> Result<(), String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_error = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match refresh_unity_type_index(project_path).await {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = error;
                eprintln!(
                    "[Locus] Unity type index refresh after recompile attempt {}/{} failed: {}",
                    attempt, MAX_ATTEMPTS, last_error
                );
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    Err(last_error)
}

/// Project-relative, forward-slash asset paths for absolute file paths under
/// `project_path`. Windows paths reach the tracker with inconsistent drive or
/// directory casing; the prefix match is case-insensitive but the returned
/// remainder keeps the on-disk casing for Unity. Paths outside the project
/// are dropped.
fn relative_asset_paths(project_path: &str, absolute_paths: &[String]) -> Vec<String> {
    let root = project_path
        .trim_end_matches(['/', '\\'])
        .replace('\\', "/");
    let root_lower = root.to_ascii_lowercase();
    let mut rels: Vec<String> = Vec::new();
    for path in absolute_paths {
        let normalized = path.replace('\\', "/");
        if normalized.to_ascii_lowercase().starts_with(&root_lower) {
            let rel = normalized[root.len()..].trim_start_matches('/');
            if !rel.is_empty() {
                rels.push(rel.to_string());
            }
        }
    }
    rels
}

const RECOMPILE_START_CONFIRM_TIMEOUT: Duration = Duration::from_secs(90);
const RECOMPILE_START_STATE_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
const RECOMPILE_START_STATE_HISTORY_LIMIT: usize = 6;
const RECOMPILE_POLL_TIMEOUT: Duration = Duration::from_secs(4);
const RECOMPILE_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
/// Continuous main-thread unresponsiveness one recompile tolerates. Compiles,
/// imports and post-reload initializers legitimately hold the main thread for
/// tens of seconds; only a stall past this budget (or one the hang timeline
/// says cannot recover) ends the wait.
const RECOMPILE_UNRESPONSIVE_BUDGET: Duration = Duration::from_secs(180);
const RECOMPILE_BUSY_RECHECK_INTERVAL: Duration = Duration::from_secs(1);
const RECOMPILE_BUSY_PERIODIC_CHECK: Duration = Duration::from_secs(5);
const RECOMPILE_OPERATION_MISMATCH_LIMIT: u32 = 20;
/// After a "not needed" verdict, how long to wait for Unity to finish any
/// compile or import that follows it, and how many consecutive idle reads
/// (500ms apart) count as settled.
const RECOMPILE_SETTLE_TIMEOUT: Duration = Duration::from_secs(120);
const RECOMPILE_SETTLE_IDLE_READS: u32 = 2;
/// Broker rejection for requests that arrive while the editor loop has not
/// pumped for 30s. Such a request was never queued for Unity.
const MAIN_THREAD_UNRESPONSIVE_ERROR: &str = "native_main_thread_unresponsive";

#[derive(Debug)]
struct RecompileStateSample {
    first_elapsed_secs: u64,
    last_elapsed_secs: u64,
    signature: String,
    detail: String,
}

#[derive(Debug, Default)]
struct RecompileStartDiagnostics {
    broker_accepted: bool,
    samples: Vec<RecompileStateSample>,
}

impl RecompileStartDiagnostics {
    fn record_formatted_state(&mut self, elapsed_secs: u64, signature: String, detail: String) {
        if let Some(last) = self.samples.last_mut() {
            if last.signature == signature {
                last.last_elapsed_secs = elapsed_secs;
                last.detail = detail;
                return;
            }
        }

        self.samples.push(RecompileStateSample {
            first_elapsed_secs: elapsed_secs,
            last_elapsed_secs: elapsed_secs,
            signature,
            detail,
        });
        if self.samples.len() > RECOMPILE_START_STATE_HISTORY_LIMIT {
            self.samples.remove(0);
        }
    }

    fn record_state(&mut self, elapsed_secs: u64, state: &SemanticState) {
        self.record_formatted_state(
            elapsed_secs,
            recompile_state_signature(state),
            format_recompile_state_detail(state),
        );
    }

    fn format_log(&self, summary: &str) -> String {
        let mut output = format!(
            "{summary}\n- Native Broker 已接收请求：{}",
            if self.broker_accepted { "是" } else { "否" }
        );
        if self.samples.is_empty() {
            output.push_str("\n- Unity 状态探测：尚未取得有效样本");
            return output;
        }

        output.push_str("\n- Unity 状态探测：");
        for sample in &self.samples {
            let elapsed = if sample.first_elapsed_secs == sample.last_elapsed_secs {
                format!("{}s", sample.first_elapsed_secs)
            } else {
                format!(
                    "{}-{}s",
                    sample.first_elapsed_secs, sample.last_elapsed_secs
                )
            };
            output.push_str(&format!("\n  - {elapsed}: {}", sample.detail));
        }
        output
    }
}

fn compact_recompile_state_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.replace(['\r', '\n'], " "))
        .unwrap_or_else(|| "none".to_string())
}

fn recompile_state_signature(state: &SemanticState) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        state.phase,
        state.reload_phase.as_deref().unwrap_or("none"),
        state.process.state,
        state.channel.control_pipe,
        state.domain.phase,
        state.editor_mode.value,
        state.main_thread.state,
        state.safety.recommended_action,
    )
}

fn format_recompile_state_detail(state: &SemanticState) -> String {
    format!(
        "phase={}/{} source={} confidence={}; process={} pid={}; channel={} latency={}ms; domain={} reload={}; editor={}; main_thread={} cpu_active={} quiescent={}ms stack={}; action={}; broker={} hook={}; detail={}; channel_error={}",
        state.phase,
        state.reload_phase.as_deref().unwrap_or("none"),
        state.source,
        state.confidence,
        state.process.state,
        state
            .process
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "none".to_string()),
        state.channel.control_pipe,
        state
            .channel
            .last_latency_ms
            .map(|latency| latency.to_string())
            .unwrap_or_else(|| "none".to_string()),
        state.domain.phase,
        state
            .domain
            .reload_sub_phase
            .as_deref()
            .unwrap_or("none"),
        state.editor_mode.value,
        state.main_thread.state,
        state.main_thread.cpu_active,
        state.main_thread.quiescent_for_ms,
        state.main_thread.stack_class.as_deref().unwrap_or("none"),
        state.safety.recommended_action,
        state.state_plane.native_broker,
        state.state_plane.native_hook,
        compact_recompile_state_text(state.detail.as_deref()),
        compact_recompile_state_text(state.channel.last_error.as_deref()),
    )
}

fn recompile_timeout_reason(state: &SemanticState) -> String {
    if state.process.state == "not_running" || matches!(state.phase.as_str(), "quit" | "crashed") {
        return "Unity Editor 进程已退出，重编译无法完成。".to_string();
    }
    if state.domain.phase == "reloading" || state.phase == "reloading" {
        return "Unity 持续处于域重载阶段，未能确认本次重编译完成。".to_string();
    }
    if matches!(state.main_thread.state.as_str(), "hung" | "stalled") {
        return "Unity 主线程无响应，编译请求未能继续处理。".to_string();
    }
    if matches!(state.channel.control_pipe.as_str(), "busy" | "timeout") {
        return "Unity 控制通道持续繁忙，未能读取本次重编译结果。".to_string();
    }
    if state.main_thread.state == "active" {
        return "Unity 主线程持续处理导入、刷新或其他 Editor 工作，未能确认本次重编译完成。"
            .to_string();
    }
    if state.editor_mode.value == "editing" && state.main_thread.state == "idle" {
        return "Unity 已恢复编辑状态，但本次编译请求没有产生可确认的完成结果。".to_string();
    }
    "Unity 未能在等待上限内返回可确认的重编译结果。".to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum RecompileStartAck {
    Started,
    Unconfirmed,
    ReloadBoundary,
    /// The broker refused the request because the main thread had not pumped
    /// the editor loop for 30s. It never reached Unity, so resending after the
    /// loop returns cannot start a second compile.
    EditorBusyNotSent,
    /// Another recompile owns the editor. Wait for it, then request ours: its
    /// compile may predate the sources this caller needs.
    InProgress { operation_id: Option<String> },
}

#[derive(Debug, PartialEq, Eq)]
enum RecompilePollState {
    Waiting,
    Completed,
    NotNeeded,
    Transient,
    /// The broker could not queue the read (old plugin, main thread away).
    EditorBusy,
    /// Unity's register belongs to another operation.
    OperationMismatch,
}

fn is_main_thread_unresponsive_error(error: &str) -> bool {
    error.trim() == MAIN_THREAD_UNRESPONSIVE_ERROR
}

/// `Some(operation)` when Unity refused because another recompile is active.
/// Current plugins name the active operation; older ones only say so in text.
fn recompile_in_progress_operation(error: &str) -> Option<Option<String>> {
    let error = error.trim();
    if let Some(rest) = error.strip_prefix("recompile_in_progress") {
        let operation = rest
            .split(';')
            .filter_map(|part| part.trim().strip_prefix("operation_id="))
            .map(str::trim)
            .find(|id| !id.is_empty())
            .map(str::to_string);
        return Some(operation);
    }
    error.contains("重编译请求正在进行中").then_some(None)
}

fn classify_recompile_start_response(resp: &PipeResponse) -> Result<RecompileStartAck, String> {
    if resp.ok {
        return match resp.message.as_deref() {
            Some("recompile_started") => Ok(RecompileStartAck::Started),
            Some(other) => Err(format!(
                "Unity 没有开始编译。启动确认返回了意外状态：{other}"
            )),
            None => Err("Unity 没有开始编译。未收到 recompile_started 启动确认。".to_string()),
        };
    }

    let error = resp
        .error
        .clone()
        .unwrap_or_else(|| "Unity 没有开始编译。启动确认失败。".to_string());
    if is_reload_boundary_broker_error(&error) {
        Ok(RecompileStartAck::ReloadBoundary)
    } else if is_main_thread_unresponsive_error(&error) {
        Ok(RecompileStartAck::EditorBusyNotSent)
    } else if let Some(operation_id) = recompile_in_progress_operation(&error) {
        Ok(RecompileStartAck::InProgress { operation_id })
    } else {
        Err(error)
    }
}

async fn request_recompile_and_wait_for_start(
    project_path: &str,
    asset_sync: &crate::workspace_changes::UnitySyncSnapshot,
    operation_id: &str,
) -> Result<RecompileStartAck, String> {
    let request_payload = serde_json::json!({
        "schema": 1,
        "syncMode": asset_sync.mode.as_str(),
        "paths": &asset_sync.paths,
        "reason": &asset_sync.reason,
        // Optional in schema 1: older plugins ignore this field, while current
        // plugins use it to reject a stale result from another recompile.
        "operationId": operation_id,
    })
    .to_string();
    let (acceptance_tx, mut acceptance_rx) = tokio::sync::oneshot::channel();
    let request = send_message_without_timeout_with_acceptance(
        project_path,
        "request_recompile",
        &request_payload,
        acceptance_tx,
    );
    tokio::pin!(request);

    let started_at = Instant::now();
    let deadline = tokio::time::sleep(RECOMPILE_START_CONFIRM_TIMEOUT);
    tokio::pin!(deadline);
    let mut sample_tick = tokio::time::interval(RECOMPILE_START_STATE_SAMPLE_INTERVAL);
    sample_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut acceptance_pending = true;
    let mut diagnostics = RecompileStartDiagnostics::default();

    loop {
        tokio::select! {
            biased;
            result = &mut request => {
                let classified = match result {
                    Ok(resp) => classify_recompile_start_response(&resp).inspect(|ack| {
                        if *ack == RecompileStartAck::Started {
                            eprintln!(
                                "[Locus] request_recompile started after {}ms",
                                started_at.elapsed().as_millis()
                            );
                        }
                    }),
                    Err(error) if is_reload_boundary_broker_error(&error) => {
                        Ok(RecompileStartAck::ReloadBoundary)
                    }
                    Err(error) => Err(error),
                };
                return match classified {
                    Ok(ack) => Ok(ack),
                    Err(error) => {
                        let state = state_probe::semantic_state_for_project(project_path).await;
                        diagnostics.record_state(started_at.elapsed().as_secs(), &state);
                        eprintln!(
                            "[Locus] request_recompile start failed:\n{}",
                            diagnostics.format_log(&error)
                        );
                        Err(error)
                    }
                };
            }
            accepted = &mut acceptance_rx, if acceptance_pending => {
                acceptance_pending = false;
                if accepted.is_ok() {
                    diagnostics.broker_accepted = true;
                    eprintln!(
                        "[Locus] native Broker accepted request_recompile after {}ms; waiting for CompilationPipeline.compilationStarted",
                        started_at.elapsed().as_millis()
                    );
                }
            }
            _ = &mut deadline => {
                let state = state_probe::semantic_state_for_project(project_path).await;
                diagnostics.record_state(started_at.elapsed().as_secs(), &state);
                eprintln!(
                    "[Locus] request_recompile start acknowledgement exceeded {}s; switching to persisted compile-result reconciliation:\n{}",
                    RECOMPILE_START_CONFIRM_TIMEOUT.as_secs(),
                    diagnostics.format_log("start acknowledgement deadline reached")
                );
                // The editor may be blocked in AssetDatabase.Refresh ("Hold on")
                // and raise compilationStarted after this foreground handshake
                // window. Continue through get_compile_result: its persisted
                // starting/pending/ok state remains authoritative even when the
                // original response arrives too late for this request future.
                return Ok(RecompileStartAck::Unconfirmed);
            }
            _ = sample_tick.tick() => {
                let state = state_probe::semantic_state_for_project(project_path).await;
                let elapsed_secs = started_at.elapsed().as_secs();
                diagnostics.record_state(elapsed_secs, &state);
                eprintln!(
                    "[Locus] request_recompile still waiting for compilationStarted after {}s (broker_accepted={}): {}",
                    elapsed_secs,
                    diagnostics.broker_accepted,
                    format_recompile_state_detail(&state)
                );
            }
        }
    }
}

fn classify_recompile_poll_response(resp: &PipeResponse) -> Result<RecompilePollState, String> {
    if resp.ok {
        return match resp.message.as_deref().unwrap_or_default() {
            "starting" | "pending" => Ok(RecompilePollState::Waiting),
            "ok" => Ok(RecompilePollState::Completed),
            "not_needed" => Ok(RecompilePollState::NotNeeded),
            other => Err(format!("Unexpected Unity compile result: {other}")),
        };
    }

    let error = resp
        .error
        .clone()
        .unwrap_or_else(|| "Compilation failed (unknown error)".to_string());
    if is_main_thread_unresponsive_error(&error) {
        Ok(RecompilePollState::EditorBusy)
    } else if error.trim() == "recompile_result_operation_mismatch" {
        Ok(RecompilePollState::OperationMismatch)
    } else if is_transient_broker_error(&error) {
        Ok(RecompilePollState::Transient)
    } else {
        Err(error)
    }
}

fn is_recompile_poll_response_timeout(error: &str) -> bool {
    matches!(
        error,
        "Unity response timed out" | "Unity request timed out"
    )
}

async fn finish_recompile_success(
    project_path: &str,
    unity_test_pending_seq: u64,
    asset_sync: &crate::workspace_changes::UnitySyncSnapshot,
) -> Result<String, String> {
    crate::unity_type_index::invalidate_cached_type_index(project_path).await;
    crate::unity_hotreload::coordinator::on_recompile_converged(project_path).await;
    crate::workspace::clear_unity_test_pending_sources_through(
        project_path,
        unity_test_pending_seq,
    );
    // The rebuilt assemblies are loaded, so this snapshot's changes are in
    // Unity. Acknowledge before waiting on the bridge: a busy editor must not
    // leave them pending, which would force a full refresh on every later
    // recompile.
    crate::workspace_changes::acknowledge_unity_sync(Path::new(project_path), asset_sync);
    if let Err(error) = wait_for_unity_bridge_ready_after_recompile(project_path).await {
        let state = unity_semantic_state(project_path).await;
        eprintln!(
            "[Locus] Unity recompile completed but the editor is still busy after the reload: {}",
            format_recompile_state_detail(&state)
        );
        return Ok(format!(
            "Unity recompile status:\n- status: compiled\n- compilation: completed\n- detection: rebuilt_assembly_output\n- asset_sync: {}\n- asset_sync_reason: {}\n- domain_reload: completed\n- bridge: recovering\n- editor: {}\n- phase: {}\n- action: wait_for_editor\n- detail: The rebuilt assemblies are loaded. Unity's main thread is still busy after the reload (initializers, imports or repaint), so the next Unity API call may wait. {}",
            asset_sync.mode.as_str(),
            asset_sync.reason,
            state.editor_mode.value,
            state.phase,
            error.replace(['\r', '\n'], " ")
        ));
    }
    if let Err(error) = refresh_unity_type_index_after_recompile(project_path).await {
        eprintln!(
            "[Locus] Unity type index refresh after recompile skipped: {}",
            error
        );
    }
    // Post-reload work (initializers, Burst, imports) can occupy the main
    // thread again after the bridge first answered; give it a short window so
    // the reported bridge state and action agree.
    let mut state = unity_semantic_state(project_path).await;
    if !state.safety.can_call_unity_api {
        let _ = wait_for_unity_bridge_ready(
            project_path,
            RECOMPILE_FINAL_READY_WAIT,
            "after the recompile type index refresh",
        )
        .await;
        state = unity_semantic_state(project_path).await;
    }
    Ok(format_compiled_recompile_status(asset_sync, &state))
}

const RECOMPILE_FINAL_READY_WAIT: Duration = Duration::from_secs(10);

fn format_compiled_recompile_status(
    asset_sync: &crate::workspace_changes::UnitySyncSnapshot,
    state: &SemanticState,
) -> String {
    let mut status = format!(
        "Unity recompile status:\n- status: compiled\n- compilation: completed\n- detection: rebuilt_assembly_output\n- asset_sync: {}\n- asset_sync_reason: {}\n- domain_reload: completed\n- bridge: {}\n- editor: {}\n- phase: {}\n- action: {}",
        asset_sync.mode.as_str(),
        asset_sync.reason,
        if state.safety.can_call_unity_api { "ready" } else { "busy" },
        state.editor_mode.value,
        state.phase,
        state.safety.recommended_action
    );
    if !state.safety.can_call_unity_api {
        status.push_str(&format!(
            "\n- detail: The rebuilt assemblies are loaded, but Unity's main thread is still busy after the reload (main thread: {}). The next Unity call waits for it.",
            state.main_thread.state
        ));
    }
    status
}

#[derive(Debug, Default)]
struct ExistingReloadContext {
    observed: bool,
    compile_converged: bool,
}

fn semantic_state_is_reloading(state: &SemanticState) -> bool {
    state.phase == "reloading"
        || state.domain.phase == "reloading"
        || state.channel.control_pipe == "reloading"
}

async fn wait_for_existing_reload(project_path: &str) -> Result<ExistingReloadContext, String> {
    let state = unity_semantic_state(project_path).await;
    if !semantic_state_is_reloading(&state) {
        return Ok(ExistingReloadContext::default());
    }

    let before = fetch_reload_state(project_path).await;
    if let Some((session_id, domain_generation, converged_serial)) = before.as_ref() {
        crate::unity_hotreload::coordinator::observe_reload_state(
            project_path,
            session_id.clone(),
            domain_generation.clone(),
            *converged_serial,
        )
        .await;
    }
    eprintln!(
        "[Locus] unity_recompile found an existing reload; waiting before deciding whether another compile is required: {}",
        format_recompile_state_detail(&state)
    );

    wait_for_unity_bridge_ready(
        project_path,
        Duration::from_secs(180),
        "before requesting another recompile; an existing reload was already in progress",
    )
    .await
    .map_err(|error| {
        format!(
            "Unity recompile status:\n- status: existing_reload_incomplete\n- compilation: not_requested\n- domain_reload: incomplete\n- bridge: not_ready\n- editor: {}\n- phase: {}\n- action: {}\n- detail: {}",
            state.editor_mode.value,
            state.phase,
            state.safety.recommended_action,
            error.replace(['\r', '\n'], " ")
        )
    })?;

    let after = fetch_reload_state(project_path).await;
    if let Some((session_id, domain_generation, converged_serial)) = after.as_ref() {
        crate::unity_hotreload::coordinator::observe_reload_state(
            project_path,
            session_id.clone(),
            domain_generation.clone(),
            *converged_serial,
        )
        .await;
    }
    let compile_converged = match (before.as_ref(), after.as_ref()) {
        (Some((before_session, _, before_serial)), Some((after_session, _, after_serial))) => {
            before_session == after_session && after_serial > before_serial
                || before_session != after_session && *after_serial > 0
        }
        _ => false,
    };
    Ok(ExistingReloadContext {
        observed: true,
        compile_converged,
    })
}

async fn request_script_reload_and_wait(project_path: &str) -> Result<(), String> {
    let (before_session, before_generation, _) = fetch_reload_state(project_path)
        .await
        .ok_or_else(|| {
            "Unity recompile status:\n- status: reload_only_incomplete\n- compilation: not_needed\n- domain_reload: not_started\n- bridge: ready\n- action: retry_recompile\n- detail: Could not read the domain generation before requesting the reload-only convergence pass."
                .to_string()
        })?;

    let response = send_message_with_timeout(
        project_path,
        "request_script_reload",
        "",
        RECOMPILE_POLL_TIMEOUT,
    )
    .await
    .map_err(|error| {
        format!(
            "Unity recompile status:\n- status: reload_only_incomplete\n- compilation: not_needed\n- domain_reload: not_started\n- bridge: ready\n- action: retry_recompile\n- detail: Failed to request the reload-only convergence pass: {}",
            error.replace(['\r', '\n'], " ")
        )
    })?;
    if !response.ok {
        return Err(format!(
            "Unity recompile status:\n- status: reload_only_incomplete\n- compilation: not_needed\n- domain_reload: not_started\n- bridge: ready\n- action: retry_recompile\n- detail: {}",
            response
                .error
                .unwrap_or_else(|| "Unity rejected the reload-only convergence pass.".to_string())
                .replace(['\r', '\n'], " ")
        ));
    }

    let started_at = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some((session_id, domain_generation, _)) = fetch_reload_state(project_path).await {
            if session_id != before_session || domain_generation != before_generation {
                wait_for_unity_bridge_ready(
                    project_path,
                    Duration::from_secs(30),
                    "after reload-only convergence",
                )
                .await
                .map_err(|error| {
                    format!(
                        "Unity recompile status:\n- status: reload_only_bridge_recovering\n- compilation: not_needed\n- domain_reload: completed\n- bridge: not_ready\n- action: wait_reload\n- detail: {}",
                        error.replace(['\r', '\n'], " ")
                    )
                })?;
                return Ok(());
            }
        }

        if started_at.elapsed() >= Duration::from_secs(60) {
            let state = unity_semantic_state(project_path).await;
            return Err(format!(
                "Unity recompile status:\n- status: reload_only_incomplete\n- compilation: not_needed\n- domain_reload: incomplete\n- bridge: {}\n- editor: {}\n- phase: {}\n- action: {}\n- detail: The reload-only convergence pass did not publish a new domain generation within 60 seconds.",
                state.channel.control_pipe,
                state.editor_mode.value,
                state.phase,
                state.safety.recommended_action
            ));
        }
    }
}

async fn finish_recompile_not_needed(
    project_path: &str,
    existing_reload: &ExistingReloadContext,
    unity_test_pending_seq: u64,
    asset_sync: &crate::workspace_changes::UnitySyncSnapshot,
) -> Result<String, String> {
    let active_patches =
        crate::unity_hotreload::coordinator::project_active_patches(project_path).await;
    let reloaded_to_drop_patches = if active_patches > 0 {
        request_script_reload_and_wait(project_path).await?;
        crate::unity_hotreload::coordinator::on_recompile_converged(project_path).await;
        true
    } else {
        crate::unity_hotreload::coordinator::on_recompile_not_needed(project_path).await;
        false
    };
    crate::workspace::clear_unity_test_pending_sources_through(
        project_path,
        unity_test_pending_seq,
    );
    crate::workspace_changes::acknowledge_unity_sync(Path::new(project_path), asset_sync);
    let state = unity_semantic_state(project_path).await;

    let status = if reloaded_to_drop_patches {
        "reloaded_without_compile"
    } else if existing_reload.compile_converged {
        "already_converged"
    } else {
        "up_to_date"
    };
    Ok(format!(
        "Unity recompile status:\n- status: {status}\n- compilation: not_needed\n- detection: unity_incremental_build_graph\n- asset_sync: {}\n- asset_sync_reason: {}\n- domain_reload: {}\n- bridge: ready\n- editor: {}\n- phase: {}\n- action: none\n- detail: {}",
        asset_sync.mode.as_str(),
        asset_sync.reason,
        if reloaded_to_drop_patches {
            "completed_to_drop_live_patches"
        } else if existing_reload.observed {
            "existing_reload_completed"
        } else {
            "unchanged"
        },
        state.editor_mode.value,
        state.phase,
        if reloaded_to_drop_patches {
            "Unity's incremental build found every assembly up to date; a reload-only pass removed live hot-patch detours."
        } else if existing_reload.compile_converged {
            "The compile already in progress converged the project; another recompile was unnecessary."
        } else {
            "Unity's incremental build graph found no assembly that requires recompilation."
        }
    ))
}

#[derive(Debug, PartialEq, Eq)]
enum EditorBusyVerdict {
    /// The editor loop is pumping.
    Responsive,
    /// Away, but within budget and not known to be stuck.
    KeepWaiting,
    /// Away past the budget, or the hang timeline says it cannot recover.
    GiveUp,
}

fn editor_busy_verdict(unresponsive: bool, blocks_progress: bool, away_for: Duration) -> EditorBusyVerdict {
    if !unresponsive {
        EditorBusyVerdict::Responsive
    } else if blocks_progress || away_for >= RECOMPILE_UNRESPONSIVE_BUDGET {
        EditorBusyVerdict::GiveUp
    } else {
        EditorBusyVerdict::KeepWaiting
    }
}

fn semantic_state_main_thread_unresponsive(state: &SemanticState) -> bool {
    state.phase == "unresponsive" || state.main_thread.state.starts_with("unresponsive")
}

/// Tracks how long Unity's main thread has stayed away from the editor loop
/// during one recompile. Reads only the observer's cached semantic state.
struct EditorBusyTracker {
    away_since: Option<Instant>,
    last_check: Option<Instant>,
}

impl EditorBusyTracker {
    fn new() -> Self {
        Self {
            away_since: None,
            last_check: None,
        }
    }

    /// `Ok(true)` while the main thread is away but waiting is reasonable,
    /// `Ok(false)` when it is pumping, a structured error when waiting stops
    /// being reasonable.
    async fn check(&mut self, project_path: &str) -> Result<bool, String> {
        let state = Box::pin(unity_semantic_state(project_path)).await;
        self.last_check = Some(Instant::now());
        let unresponsive = semantic_state_main_thread_unresponsive(&state);
        if !unresponsive {
            self.away_since = None;
            return Ok(false);
        }
        let away_for = self.away_since.get_or_insert_with(Instant::now).elapsed();
        let blocks_progress = state
            .hang_diagnosis
            .as_ref()
            .and_then(|diagnosis| diagnosis.stall.as_ref())
            .is_some_and(|stall| stall.blocks_progress);
        match editor_busy_verdict(true, blocks_progress, away_for) {
            EditorBusyVerdict::GiveUp => Err(Box::pin(editor_unresponsive_recompile_error(
                project_path,
                &state,
                away_for,
            ))
            .await),
            _ => Ok(true),
        }
    }

    async fn check_periodically(&mut self, project_path: &str) -> Result<(), String> {
        if self
            .last_check
            .is_some_and(|at| at.elapsed() < RECOMPILE_BUSY_PERIODIC_CHECK)
        {
            return Ok(());
        }
        Box::pin(self.check(project_path)).await.map(|_| ())
    }
}

async fn editor_unresponsive_recompile_error(
    project_path: &str,
    state: &SemanticState,
    away_for: Duration,
) -> String {
    let stall = state
        .hang_diagnosis
        .as_ref()
        .and_then(|diagnosis| diagnosis.stall.as_ref());
    let blocks_progress = stall.is_some_and(|stall| stall.blocks_progress);
    let away_secs = state
        .main_thread
        .heartbeat_age_ms
        .map(|age| age / 1_000)
        .unwrap_or(0)
        .max(away_for.as_secs());
    let header = format!(
        "Unity recompile status:\n- status: editor_unresponsive\n- compilation: unconfirmed\n- domain_reload: unconfirmed\n- bridge: main_thread_unresponsive\n- stall: {}\n- action: {}\n- detail: Unity's main thread has not returned to the editor loop for {away_secs}s{}. The compile was not confirmed; check the stall location below before retrying.",
        stall.map(|stall| stall.kind.as_str()).unwrap_or("unknown"),
        if blocks_progress { "fix_blocking_code_then_restart_editor" } else { "diagnose_hang" },
        if blocks_progress { " and the hang timeline shows it will not recover by itself" } else { "" },
    );
    hang_diagnosis::append_to_unresponsive_error(project_path, header).await
}

/// After a `native_main_thread_unresponsive` rejection: wait until the editor
/// loop pumps again (or waiting stops being reasonable).
async fn wait_for_editor_loop(project_path: &str, busy: &mut EditorBusyTracker) -> Result<(), String> {
    loop {
        if let Some(error) = dialog::blocked_error(project_path, "not_sent", None) {
            return Err(error);
        }
        tokio::time::sleep(RECOMPILE_BUSY_RECHECK_INTERVAL).await;
        if !busy.check(project_path).await? {
            return Ok(());
        }
    }
}

/// Domain identity + convergence serial, read before a recompile request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReloadBaseline {
    session_id: String,
    converged_serial: i64,
}

async fn fetch_reload_baseline(project_path: &str) -> Option<ReloadBaseline> {
    fetch_reload_state(project_path)
        .await
        .map(|(session_id, _, converged_serial)| ReloadBaseline {
            session_id,
            converged_serial,
        })
}

/// A compile that ran after the request rebuilt and loaded assemblies: the
/// serial only advances once new assemblies are loaded (or the editor
/// process changed, which also loaded the current sources).
fn follow_up_compile_converged(baseline: Option<&ReloadBaseline>, settled: &ReloadStateMessage) -> bool {
    match baseline {
        Some(baseline) if baseline.session_id == settled.session_id => {
            settled.converged_serial > baseline.converged_serial
        }
        Some(_) => !settled.session_id.is_empty(),
        None => false,
    }
}

/// Wait until Unity is neither compiling nor importing, after a "not needed"
/// verdict. Unity can finish the requested (no-op) compile and immediately
/// run another one for imports that landed meanwhile; returning before that
/// lets the caller's next Unity call hit the follow-up domain reload. Returns
/// the settled reload state, or `None` when it could not be read (older
/// plugin or the settle window ran out).
async fn settle_after_not_needed(
    project_path: &str,
    busy: &mut EditorBusyTracker,
) -> Result<Option<ReloadStateMessage>, String> {
    let started = Instant::now();
    let mut idle_reads = 0u32;
    loop {
        if let Some(error) = dialog::blocked_error(project_path, "unknown", None) {
            return Err(error);
        }
        if started.elapsed() >= RECOMPILE_SETTLE_TIMEOUT {
            eprintln!(
                "[Locus] Unity kept compiling or importing for {}s after a no-op recompile; reporting the verdict as is",
                RECOMPILE_SETTLE_TIMEOUT.as_secs()
            );
            return Ok(None);
        }
        match send_message_with_timeout(project_path, "get_reload_state", "", RECOMPILE_POLL_TIMEOUT)
            .await
        {
            Ok(resp) if resp.ok => {
                let parsed = resp
                    .message
                    .as_deref()
                    .and_then(|message| serde_json::from_str::<ReloadStateMessage>(message).ok());
                let Some(parsed) = parsed else {
                    return Ok(None);
                };
                if parsed.is_compiling || parsed.is_updating {
                    idle_reads = 0;
                } else {
                    idle_reads += 1;
                    if idle_reads >= RECOMPILE_SETTLE_IDLE_READS {
                        return Ok(Some(parsed));
                    }
                }
            }
            Ok(resp) => {
                idle_reads = 0;
                let error = resp.error.unwrap_or_default();
                if is_main_thread_unresponsive_error(&error) {
                    busy.check(project_path).await?;
                } else if !is_transient_broker_error(&error) {
                    // A plugin without the message: keep its old behavior.
                    return Ok(None);
                }
            }
            Err(error) => {
                idle_reads = 0;
                if !is_recompile_poll_response_timeout(&error) {
                    transport::disconnect(project_path).await;
                }
                busy.check_periodically(project_path).await?;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RecompileOutcome {
    Completed,
    NotNeeded,
    /// Unity's terminal error for the operation (compile errors included).
    Failed(String),
}

/// Poll Unity's persisted register for `operation_id` until it is terminal.
/// `adopting` waits on another caller's operation and stops as soon as the
/// register moves to a different one.
async fn poll_recompile_outcome(
    project_path: &str,
    operation_id: &str,
    mut disconnected: bool,
    busy: &mut EditorBusyTracker,
    adopting: bool,
) -> Result<RecompileOutcome, String> {
    let mut mismatches = 0u32;
    loop {
        if let Some(error) = dialog::blocked_error(project_path, "unknown", None) {
            return Err(error);
        }
        if disconnected {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match send_message_with_timeout(project_path, "ping", "", RECOMPILE_POLL_TIMEOUT).await
            {
                Ok(resp) if resp.ok => {
                    eprintln!("[Locus] Unity reconnected after domain reload");
                    disconnected = false;
                }
                _ => {
                    busy.check_periodically(project_path).await?;
                    continue;
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
        match send_message_with_timeout(
            project_path,
            "get_compile_result",
            operation_id,
            RECOMPILE_POLL_TIMEOUT,
        )
        .await
        {
            Ok(resp) => match classify_recompile_poll_response(&resp) {
                Ok(RecompilePollState::Waiting | RecompilePollState::Transient) => {
                    mismatches = 0;
                    // Current brokers answer from their register even while
                    // the main thread is away, so watch for a stuck editor here.
                    busy.check_periodically(project_path).await?;
                }
                Ok(RecompilePollState::EditorBusy) => {
                    if busy.check(project_path).await? {
                        tokio::time::sleep(RECOMPILE_BUSY_RECHECK_INTERVAL).await;
                    }
                }
                Ok(RecompilePollState::OperationMismatch) => {
                    if adopting {
                        return Ok(RecompileOutcome::Failed(
                            "the adopted recompile was replaced by another request".to_string(),
                        ));
                    }
                    mismatches += 1;
                    if mismatches >= RECOMPILE_OPERATION_MISMATCH_LIMIT {
                        return Err(format!(
                            "Unity recompile status:\n- status: superseded\n- compilation: unconfirmed\n- domain_reload: unconfirmed\n- action: retry_recompile\n- detail: Unity's compile register belongs to another request; operation {operation_id} was replaced before it reported a result."
                        ));
                    }
                }
                Ok(RecompilePollState::Completed) => return Ok(RecompileOutcome::Completed),
                Ok(RecompilePollState::NotNeeded) => return Ok(RecompileOutcome::NotNeeded),
                Err(error) => return Ok(RecompileOutcome::Failed(error)),
            },
            Err(error) => {
                if is_recompile_poll_response_timeout(&error) {
                    // The native broker accepted the request and may still execute the
                    // queued main-thread read after this local response deadline. The
                    // Unity-side result register is idempotent, so keep the healthy pipe
                    // and retry without turning a busy main thread into a reconnect loop.
                    eprintln!(
                        "[Locus] Unity compile-result response timed out; keeping the native bridge and retrying the persisted result"
                    );
                    busy.check_periodically(project_path).await?;
                    continue;
                }
                disconnected = true;
                transport::disconnect(project_path).await;
                eprintln!(
                    "[Locus] Unity compile-result polling lost the bridge ({error}); reconnecting before re-checking persisted state"
                );
            }
        }
    }
}

async fn recompile_and_wait_inner(project_path: &str) -> Result<String, String> {
    let existing_reload = Box::pin(wait_for_existing_reload(project_path)).await?;
    let mut busy = EditorBusyTracker::new();
    let mut adopted_operation = false;

    let (operation_id, asset_sync, unity_test_pending_seq, disconnected, reload_baseline) = loop {
        let operation_id = format!("recompile-{}", uuid::Uuid::new_v4().simple());
        // Before the request, so a compile that converges after a no-op
        // verdict can be recognized (see settle_after_not_needed).
        let reload_baseline = Box::pin(fetch_reload_baseline(project_path)).await;

        // Hot-reload edits bypass the AssetDatabase entirely; forward every
        // tracked dirty path so the plugin imports created files (and refreshes
        // away deleted ones) before compiling. Without this, files created or
        // deleted during a hot-reload session would be missing from (or stale
        // in) the converged assembly. Older plugins ignore the message body.
        // Taken per attempt: a resend after waiting must cover newer edits.
        let (unity_test_pending_seq, unity_test_pending_paths) =
            crate::workspace::unity_test_pending_source_snapshot(project_path);
        let mut tracked_dirty_sources =
            crate::unity_hotreload::coordinator::pending_paths(project_path).await;
        tracked_dirty_sources.extend(unity_test_pending_paths);
        tracked_dirty_sources.sort();
        tracked_dirty_sources.dedup();
        let tracked_dirty_paths = relative_asset_paths(project_path, &tracked_dirty_sources);
        let asset_sync = crate::workspace_changes::snapshot_unity_sync(
            Path::new(project_path),
            &tracked_dirty_paths,
        );
        eprintln!(
            "[Locus] Unity asset sync decision: mode={}, paths={}, generation={}, through_seq={}, health={:?}, reason={}",
            asset_sync.mode.as_str(),
            asset_sync.paths.len(),
            asset_sync.generation,
            asset_sync.through_seq,
            asset_sync.health,
            asset_sync.reason,
        );

        match Box::pin(request_recompile_and_wait_for_start(project_path, &asset_sync, &operation_id)).await? {
            RecompileStartAck::Started => {
                break (operation_id, asset_sync, unity_test_pending_seq, false, reload_baseline)
            }
            RecompileStartAck::Unconfirmed => {
                eprintln!(
                    "[Locus] compilation start acknowledgement was not observed; reconciling persisted compile state"
                );
                break (operation_id, asset_sync, unity_test_pending_seq, false, reload_baseline);
            }
            RecompileStartAck::ReloadBoundary => {
                transport::disconnect(project_path).await;
                eprintln!(
                    "[Locus] request_recompile crossed a domain reload before the start acknowledgement; waiting for persisted state"
                );
                break (operation_id, asset_sync, unity_test_pending_seq, true, reload_baseline);
            }
            RecompileStartAck::EditorBusyNotSent => {
                eprintln!(
                    "[Locus] request_recompile was not delivered: Unity's main thread is away from the editor loop; resending once it pumps again"
                );
                Box::pin(wait_for_editor_loop(project_path, &mut busy)).await?;
            }
            RecompileStartAck::InProgress { operation_id: active } => {
                if adopted_operation {
                    return Err(
                        "Unity recompile status:\n- status: recompile_in_progress\n- compilation: not_requested\n- domain_reload: unconfirmed\n- action: retry_recompile\n- detail: Unity still reports another recompile in progress after this request waited for it."
                            .to_string(),
                    );
                }
                adopted_operation = true;
                eprintln!(
                    "[Locus] another recompile ({}) is in progress; waiting for it before requesting this one",
                    active.as_deref().unwrap_or("operation id not reported")
                );
                // The outcome belongs to the other request. This request still
                // compiles afterwards so the current sources are what loads.
                let outcome = Box::pin(poll_recompile_outcome(
                    project_path,
                    active.as_deref().unwrap_or(""),
                    false,
                    &mut busy,
                    true,
                ))
                .await?;
                eprintln!("[Locus] adopted recompile finished: {outcome:?}");
            }
        }
    };

    match Box::pin(poll_recompile_outcome(project_path, &operation_id, disconnected, &mut busy, false)).await? {
        RecompileOutcome::Completed => {
            Box::pin(finish_recompile_success(project_path, unity_test_pending_seq, &asset_sync)).await
        }
        RecompileOutcome::NotNeeded => {
            let settled = Box::pin(settle_after_not_needed(project_path, &mut busy)).await?;
            if settled
                .as_ref()
                .is_some_and(|settled| follow_up_compile_converged(reload_baseline.as_ref(), settled))
            {
                eprintln!(
                    "[Locus] the requested compile was a no-op, but a follow-up compile rebuilt and loaded assemblies; reporting it as compiled"
                );
                return Box::pin(finish_recompile_success(
                    project_path,
                    unity_test_pending_seq,
                    &asset_sync,
                ))
                .await;
            }
            Box::pin(finish_recompile_not_needed(
                project_path,
                &existing_reload,
                unity_test_pending_seq,
                &asset_sync,
            ))
            .await
        }
        RecompileOutcome::Failed(error) => Err(error),
    }
}

/// Trigger a Unity recompile and wait until the requested compilation starts,
/// finishes, and loads into a new AppDomain. `request_recompile` acknowledges
/// only from Unity's `CompilationPipeline.compilationStarted` event. A pipe
/// reconnect is connectivity evidence only; completion still requires the
/// persisted `get_compile_result == ok` state from the reloaded domain.
pub async fn recompile_and_wait(project_path: &str) -> Result<String, String> {
    // Callers (tools, CLI acceptance suites awaiting several recompiles in one
    // future) keep only this pointer; the full state machine is large in
    // debug builds and overflowed the tokio worker stack when inlined.
    Box::pin(recompile_and_wait_boxed(project_path)).await
}

async fn recompile_and_wait_boxed(project_path: &str) -> Result<String, String> {
    // Recompile releases all Editor edit owners. Consume deferred starts first
    // so later source-edit rounds can establish a fresh suppression session.
    wait_for_edit_session_starts(project_path, "").await;
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let _recompile_wait_guard = UnityRecompileWaitGuard::new(project_path);
    if let Err(error) = dialog::ensure_project_observed(project_path).await {
        eprintln!("[Locus] Unity modal dialog observation could not be refreshed: {error}");
    }
    if let Some(error) = dialog::blocked_error(project_path, "not_sent", None) {
        return Err(error);
    }
    let hook_effective = background_hook_effective_for_project(project_path).await;
    let prev_foreground = if hook_effective {
        None
    } else {
        focus::bring_unity_to_foreground()
    };

    let result = match tokio::time::timeout(
        RECOMPILE_TOTAL_TIMEOUT,
        Box::pin(recompile_and_wait_inner(project_path)),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            let state = state_probe::semantic_state_for_project(project_path).await;
            eprintln!(
                "[Locus] Unity recompile exceeded {}s: {}",
                RECOMPILE_TOTAL_TIMEOUT.as_secs(),
                format_recompile_state_detail(&state)
            );
            let error = format!(
                "Unity recompile status:\n- status: timed_out\n- compilation: unconfirmed\n- domain_reload: unconfirmed\n- editor: {}\n- phase: {}\n- action: {}\n- detail: {}",
                state.editor_mode.value,
                state.phase,
                state.safety.recommended_action,
                recompile_timeout_reason(&state)
            );
            if semantic_state_main_thread_unresponsive(&state) {
                Err(hang_diagnosis::append_to_unresponsive_error(project_path, error).await)
            } else {
                Err(error)
            }
        }
    };
    if let Err(error) = &result {
        // Every terminal failure leaves one line with the editor state, so a
        // misreported recompile can be reconstructed from the log afterwards.
        let state = state_probe::semantic_state_for_project(project_path).await;
        eprintln!(
            "[Locus] unity_recompile failed: {} || state: {}",
            error.lines().take(10).collect::<Vec<_>>().join(" | "),
            format_recompile_state_detail(&state)
        );
    }

    if let Some(hwnd) = prev_foreground {
        focus::restore_foreground(hwnd);
    }
    result
}

/// C# source written by [`run_recompile_probe`]. `__CLASS__`/`__STAMP__` are
/// substituted per run so each probe file is unique and harmless (a plain,
/// inert type in its own namespace — no `MonoBehaviour`, no side effects).
const RECOMPILE_PROBE_TEMPLATE: &str = "// Auto-generated by the Locus recompile probe — safe to delete.\nnamespace Locus.RecompileProbe\n{\n    internal static class __CLASS__\n    {\n        public const long Stamp = __STAMP__L;\n    }\n}\n";

/// One-shot "does a recompile actually converge?" probe for the test page.
///
/// Writes a throwaway, harmless `.cs` into the project's `Assets`, drives a real
/// [`recompile_and_wait`] (so it exercises the exact background-hook / foreground
/// path normal convergence uses), then deletes the file and converges the
/// deletion. Returns a line-oriented report (`PASS`/`FAIL`/`WARN` prefixes so the
/// UI can colour them). The probe file is always cleaned up, even when the
/// recompile fails.
pub async fn run_recompile_probe(project_path: &str) -> Result<String, String> {
    if !is_unity_connected(project_path).await {
        return Err("Unity Editor is not connected — cannot run the recompile probe.".to_string());
    }

    let assets_dir = std::path::Path::new(project_path).join("Assets");
    if !assets_dir.is_dir() {
        return Err(format!(
            "No Assets directory under the project: {}",
            assets_dir.display()
        ));
    }

    let stamp = unix_now_ms();
    let class_name = format!("LocusRecompileProbe_{stamp}");
    let file_name = format!("{class_name}.cs");
    let probe_path = assets_dir.join(&file_name);
    let meta_path = assets_dir.join(format!("{file_name}.meta"));
    let source = RECOMPILE_PROBE_TEMPLATE
        .replace("__CLASS__", &class_name)
        .replace("__STAMP__", &stamp.to_string());

    let mut report: Vec<String> = Vec::new();
    report.push(format!("Recompile probe · project {project_path}"));

    // Surface the hook regime up front. This also patches when possible,
    // mirroring what recompile_and_wait does internally a moment later.
    let hook_effective = background_hook_effective_for_project(project_path).await;
    report.push(format!(
        "Background hook: {}",
        describe_background_hook(project_path)
    ));

    // 1) Write the throwaway source.
    if let Err(error) = std::fs::write(&probe_path, source) {
        return Err(format!(
            "Failed to write probe file {}: {error}",
            probe_path.display()
        ));
    }
    report.push(format!("PASS wrote probe source Assets/{file_name}"));

    // 2) The actual test: drive a real recompile and time it.
    let started = std::time::Instant::now();
    let recompile = recompile_and_wait(project_path).await;
    let secs = started.elapsed().as_secs_f64();
    let recompile_ok = recompile.is_ok();
    match &recompile {
        Ok(message) => report.push(format!(
            "PASS recompile converged in {secs:.1}s — {message}"
        )),
        Err(error) => report.push(format!(
            "FAIL recompile did not converge after {secs:.1}s — {error}"
        )),
    }
    if recompile_ok {
        if hook_effective {
            report.push(
                "→ Converged via the background hook; Unity did not need the foreground."
                    .to_string(),
            );
        } else {
            report.push(
                "→ Background hook inactive: recompile_and_wait pulled Unity to the foreground to converge. A background-triggered convergence would stall here until Unity is focused."
                    .to_string(),
            );
        }
    }

    // 3) Delete the probe source (+ the .meta Unity generated for it). Always
    // attempted, even when the recompile above failed.
    let mut cleanup_errors: Vec<String> = Vec::new();
    for path in [&probe_path, &meta_path] {
        if path.exists() {
            if let Err(error) = std::fs::remove_file(path) {
                cleanup_errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
    if cleanup_errors.is_empty() {
        report.push("PASS deleted probe file (.cs + .meta)".to_string());
    } else {
        report.push(format!("FAIL probe cleanup: {}", cleanup_errors.join("; ")));
    }

    // 4) Converge the deletion so the assembly drops the probe type. Best-effort.
    let cleanup_started = std::time::Instant::now();
    match recompile_and_wait(project_path).await {
        Ok(_) => report.push(format!(
            "PASS cleanup recompile converged in {:.1}s",
            cleanup_started.elapsed().as_secs_f64()
        )),
        Err(error) => report.push(format!("WARN cleanup recompile skipped — {error}")),
    }

    Ok(report.join("\n"))
}

/// Human-readable one-liner for the current background-hook patch state, used in
/// the recompile-probe report so the user can see which regime they are in.
fn describe_background_hook(project_path: &str) -> String {
    let status = background_hook::status_for_project(project_path);
    match status.state {
        background_hook::UnityBackgroundHookState::Patched => {
            format!("active (patched, {} symbol(s))", status.symbol_count)
        }
        background_hook::UnityBackgroundHookState::Inactive => {
            "inactive (not patched yet)".to_string()
        }
        background_hook::UnityBackgroundHookState::Disabled => "disabled in settings".to_string(),
        background_hook::UnityBackgroundHookState::Unsupported => {
            "unsupported on this OS".to_string()
        }
        background_hook::UnityBackgroundHookState::Failed => format!(
            "failed — {}",
            status.error.unwrap_or_else(|| "unknown error".to_string())
        ),
    }
}

#[derive(serde::Deserialize)]
struct ReloadStateMessage {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    domain_generation: String,
    #[serde(default)]
    converged_serial: i64,
    /// `EditorApplication.isCompiling` / `isUpdating` at the read. Older
    /// plugins omit them, which reads as settled (their previous behavior).
    #[serde(default)]
    is_compiling: bool,
    #[serde(default)]
    is_updating: bool,
}

/// Read Unity's reload lifecycle — the per-process session id, per-domain
/// generation, and the serial that advances on every successful compilation —
/// for the hot-reload coordinator. Best-effort: any failure (pipe down,
/// mid-reload, a plugin predating the message) returns None and the caller
/// retries on the next poll.
async fn fetch_reload_state(project_path: &str) -> Option<(String, String, i64)> {
    let resp =
        send_message_with_timeout(project_path, "get_reload_state", "", Duration::from_secs(4))
            .await
            .ok()?;
    if !resp.ok {
        return None;
    }
    let parsed: ReloadStateMessage = serde_json::from_str(resp.message.as_deref()?).ok()?;
    if parsed.session_id.is_empty() || parsed.domain_generation.is_empty() {
        return None;
    }
    Some((
        parsed.session_id,
        parsed.domain_generation,
        parsed.converged_serial,
    ))
}

pub async fn start_unity_monitor(
    app_handle: AppHandle,
    project_path: String,
    monitor: &UnityMonitorHandle,
    event_scope: crate::workspace_service::event::WorkspaceEventScope,
) {
    stop_unity_monitor_for_project(monitor, &project_path).await;
    set_event_app_handle(app_handle.clone());

    let pipe_name = get_native_pipe_name(&project_path);
    eprintln!(
        "[Locus] Unity project detected, starting connection monitor (pipe: {})",
        pipe_name
    );
    state_probe::start_observer(&project_path);
    let monitor_project_path = project_path.clone();
    let wake = register_unity_monitor_waker(&project_path);

    let handle = tauri::async_runtime::spawn(async move {
        let mut last_status: Option<bool> = None;
        let mut last_detected_editor_process: Option<UnityEditorProcessInfo> = None;
        let mut disconnected_attempts: u32 = 0;
        let mut last_play_mode: Option<bool> = None;
        // Whether a reload-state baseline has landed since the current connection
        // came up. Stays false until a fetch actually succeeds, so a failed
        // connect-time probe keeps retrying every poll instead of leaving the
        // first successful sample to coincide with a post-edit state.
        let mut reload_state_seeded = false;
        // Editor process whose exit bookkeeping already ran, so the disconnect
        // transition and a later process-exit tick never both run it.
        let mut exit_handled_process_id: Option<u32> = None;
        let mut last_published_detail: Option<(String, u64)> = None;

        loop {
            let previous_running_process_id = last_detected_editor_process
                .as_ref()
                .filter(|info| info.state == UnityEditorProcessState::Running)
                .and_then(|info| info.process_id);
            let mut status = query_unity_connection_status(&project_path).await;
            let connected = status.connected;
            let disconnected_transition = last_status == Some(true) && !connected;
            let recompile_waiting = unity_recompile_waiting(&project_path);

            // H6: a play-mode EXIT is a convergence point for hot reload —
            // deferred/in-flight patch state turns into a silent recompile.
            if connected {
                let playing = is_play_mode_status(&status.editor_status);
                if last_play_mode == Some(true) && !playing {
                    let play_exit_project = project_path.clone();
                    tokio::spawn(async move {
                        crate::unity_hotreload::coordinator::on_play_mode_exited(
                            &play_exit_project,
                        )
                        .await;
                    });
                }
                last_play_mode = Some(playing);
            } else {
                last_play_mode = None;
            }

            if connected {
                let just_connected = last_status != Some(true);
                if just_connected {
                    eprintln!("[Locus] Unity Editor connected! (pipe: {})", pipe_name);
                    let editor_project = project_path.clone();
                    tokio::spawn(async move {
                        let set_default = external_editor_default_enabled();
                        match configure_locus_external_editor(&editor_project, set_default).await {
                            Ok(status) => {
                                eprintln!("[Locus] Unity external editor registration: {}", status)
                            }
                            Err(error) => eprintln!(
                                "[Locus] Unity external editor registration skipped: {}",
                                error
                            ),
                        }
                        if !unity_project_files_present(&editor_project) {
                            match sync_project_files(&editor_project).await {
                                Ok(report) => eprintln!(
                                    "[Locus] generated Unity project files on connect: {}",
                                    report.replace('\n', " | ")
                                ),
                                Err(error) => eprintln!(
                                    "[Locus] Unity project-file generation on connect failed: {}",
                                    error
                                ),
                            }
                            crate::csharp_lsp::warm_up_in_background(editor_project);
                        }
                    });
                    // Pre-start the compile-server sidecar (and JIT-warm
                    // Roslyn) so the first unity_execute does not pay the
                    // cold-start cost. No-op while the feature is off.
                    crate::csharp_compile::warm_up_in_background();
                    // Also prefetch the compile params: the first collection
                    // walks every reference assembly on the Unity main
                    // thread — do it now, off the first tool call's path.
                    if crate::csharp_compile::is_enabled() {
                        let params_project = project_path.clone();
                        tokio::spawn(async move {
                            if let Err(error) =
                                crate::csharp_compile::params::get_params(&params_project).await
                            {
                                eprintln!(
                                    "[CsharpCompile] compile params prefetch skipped: {error}"
                                );
                            }
                        });
                    }
                }
                // Reconcile the hot-reload "unapplied" set against the editor's
                // reload lifecycle on every poll (not only on reconnect): a
                // Unity-initiated recompile (manual Ctrl+R, save, focus
                // auto-refresh) converges it like a Locus recompile, while a
                // bare domain reload (entering play mode) keeps edits pending —
                // detected whether or not the pipe dropped across the reload,
                // and a transient pipe drop within one domain keeps detours.
                //
                // ALWAYS establish a reload-state baseline before any edit:
                // otherwise an edit that compiles before the first sample would
                // be the first sample and only seed, missing the convergence (or,
                // worse, be mistaken for a startup-compiled survivor). Keep
                // retrying until a fetch lands (a connect-time probe can fail
                // mid-startup); afterwards keep observing whenever the feature is
                // on OR there is outstanding tracking (so toggling hot reload off
                // with pending work does not strand a stale count).
                if !reload_state_seeded
                    || crate::unity_hotreload::is_enabled()
                    || crate::unity_hotreload::coordinator::has_pending_state(&project_path).await
                {
                    if let Some((session, generation, serial)) =
                        fetch_reload_state(&project_path).await
                    {
                        crate::unity_hotreload::coordinator::observe_reload_state(
                            &project_path,
                            session,
                            generation,
                            serial,
                        )
                        .await;
                        reload_state_seeded = true;
                    }
                }
                disconnected_attempts = 0;
            } else {
                disconnected_attempts = disconnected_attempts.saturating_add(1);
                status.reconnect_attempts = disconnected_attempts;
                // Lost the editor: a relaunch is a fresh instance, so force a new
                // baseline on reconnect rather than judging it against the dead
                // session's trackers.
                reload_state_seeded = false;

                match status.last_error.as_deref() {
                    Some(error) if last_status != Some(false) => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity Editor not connected (pipe: {}): {}",
                            pipe_name,
                            error
                        );
                    }
                    Some(error) if disconnected_attempts % 10 == 0 => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity reconnect still failing after {} attempt(s) (pipe: {}): {}",
                            disconnected_attempts,
                            pipe_name,
                            error
                        );
                    }
                    None if last_status != Some(false) => {
                        tracing::debug!(
                            log_module = "Locus",
                            "Unity Editor not connected (pipe: {}): status returned disconnected",
                            pipe_name
                        );
                    }
                    None => {}
                    _ => {}
                }
            }

            // The exit-liveness check runs on the disconnect TRANSITION even
            // while a recompile is waiting: a crash during the compile window
            // is the most common crash point, and this transition tick is the
            // only one that fires — swallowing it froze the dead editor's
            // active-patch tallies forever (poisoning the sidecar swap gate
            // and the lost-session clear condition). A domain-reload pipe drop
            // keeps the process alive, so the liveness refresh distinguishes
            // the two and the normal recompile flow is unaffected.
            if disconnected_transition {
                if let Some(refresh) = process::refresh_known_project_editor_process_liveness(
                    &project_path,
                    last_detected_editor_process.clone(),
                )
                .await
                {
                    let observed_process_not_running =
                        matches!(refresh.observed.state, UnityEditorProcessState::NotRunning);
                    apply_unity_process_info(&mut status, refresh.effective);
                    sync_unity_launch_mode_for_status(&mut status).await;
                    if observed_process_not_running {
                        sync_background_hook_for_status(&mut status, &project_path).await;
                        // The editor is gone: reset its dead detour state but KEEP
                        // the tracked edits — they are still not in any running
                        // editor. A relaunch's startup recompile loads them, and
                        // the next reload-state sample converges them then (or
                        // keeps them if that compile fails).
                        crate::unity_hotreload::coordinator::on_editor_exited(&project_path).await;
                        exit_handled_process_id = previous_running_process_id;
                    } else if recompile_waiting {
                        // Alive mid-recompile: keep the hook sync the
                        // recompile-waiting branch below would have run on this
                        // same tick before the exit check took precedence.
                        sync_background_hook_for_status(&mut status, &project_path).await;
                    }
                }
            } else if recompile_waiting && !connected {
                if let Some(process_info) = last_detected_editor_process
                    .clone()
                    .filter(|info| info.process_id.is_some())
                {
                    apply_unity_process_info(&mut status, process_info);
                    sync_unity_launch_mode_for_status(&mut status).await;
                    sync_background_hook_for_status(&mut status, &project_path).await;
                }
            }

            // Unity closes the pipe before its native shutdown finishes, so the
            // process usually exits after the disconnect transition. The kernel
            // exit wait wakes this loop; run the exit bookkeeping once then.
            if !connected
                && status.editor_process_state == UnityEditorProcessState::NotRunning
            {
                if let Some(process_id) = previous_running_process_id
                    .filter(|process_id| exit_handled_process_id != Some(*process_id))
                {
                    sync_background_hook_for_status(&mut status, &project_path).await;
                    crate::unity_hotreload::coordinator::on_editor_exited(&project_path).await;
                    exit_handled_process_id = Some(process_id);
                }
            }

            if connected {
                status.reconnect_attempts = 0;
            }

            last_detected_editor_process = unity_process_info_from_status(&status);
            crate::view::sync_unity_owned_view_windows_for_project(
                &app_handle,
                &project_path,
                status.editor_process_id,
                matches!(
                    status.editor_process_state,
                    UnityEditorProcessState::Running
                ),
            );

            // Push on change; timing-only updates (check timestamps, latency,
            // reconnect counters) ride along at most every heartbeat interval.
            let signature = connection_status_signature(&status);
            let now_ms = unix_now_ms();
            let publish_detail = last_published_detail.as_ref().is_none_or(
                |(previous, published_at_ms)| {
                    *previous != signature
                        || now_ms.saturating_sub(*published_at_ms)
                            >= UNITY_CONNECTION_DETAIL_HEARTBEAT_MS
                },
            );
            if publish_detail {
                crate::workspace_service::event::emit_for_workspace_scope(
                    &app_handle,
                    &event_scope,
                    "unity-connection-status-detail",
                    status.clone(),
                );
                last_published_detail = Some((signature, now_ms));
            }

            if last_status != Some(connected) {
                crate::workspace_service::event::emit_for_workspace_scope(
                    &app_handle,
                    &event_scope,
                    "unity-connection-status",
                    connected,
                );
            }
            last_status = Some(connected);

            tokio::select! {
                _ = tokio::time::sleep(UNITY_MONITOR_INTERVAL) => {}
                _ = wake.notified() => {}
            }
        }
    });

    monitor.lock().await.insert(
        normalize_project_path_for_state_plane(&monitor_project_path),
        (monitor_project_path, handle),
    );
}

pub async fn stop_unity_monitor_for_project(monitor: &UnityMonitorHandle, project_path: &str) {
    let handle = monitor
        .lock()
        .await
        .remove(&normalize_project_path_for_state_plane(project_path));
    if let Some((project_path, handle)) = handle {
        handle.abort();
        eprintln!(
            "[Locus] Unity connection monitor stopped for {}",
            project_path
        );
        crate::unity_hotreload::coordinator::on_monitor_stopped_background(project_path);
    }
    unregister_unity_monitor_waker(project_path);
    state_probe::stop_observer(project_path);
}

pub async fn stop_unity_monitor(monitor: &UnityMonitorHandle) {
    let handles = {
        let mut monitors = monitor.lock().await;
        std::mem::take(&mut *monitors)
    };
    for (_, (project, handle)) in handles {
        handle.abort();
        unregister_unity_monitor_waker(&project);
        eprintln!("[Locus] Unity connection monitor stopped for {}", project);
        crate::unity_hotreload::coordinator::on_monitor_stopped_background(project);
    }
    state_probe::stop_all_observers();
}

#[cfg(test)]
mod tests {
    use super::{
        cache_unity_connection_status, cached_running_connection_status_for_transient_failure,
        classify_recompile_poll_response, classify_recompile_start_response, editor_busy_verdict,
        is_recompile_poll_response_timeout, is_transient_broker_error,
        recompile_in_progress_operation, EditorBusyVerdict, RECOMPILE_TOTAL_TIMEOUT,
        RECOMPILE_UNRESPONSIVE_BUDGET, follow_up_compile_converged, ReloadBaseline,
        ReloadStateMessage,
        native_background_hook_markers_present, parse_unity_hub_editor_locations,
        parse_unity_status_message, pipe_response_transient_broker_error, play_mode_target_status,
        read_project_unity_version, relative_asset_paths, requested_run_states_editor_status,
        rewrite_run_states_output_for_size, unity_test_cancel_dispatch_response,
        unity_test_cancel_snapshot, unity_test_progress_update, unity_test_run_is_terminal,
        unity_test_short_name, unity_test_start_request, wait_for_unity_test_poll_wake,
        PipeResponse, RecompilePollState, RecompileStartAck, UnityTestProgress,
        UnityTestRunSnapshot,
        RecompileStartDiagnostics, UnityBackgroundHookState, UnityBackgroundHookStatus,
        UnityConnectionStatus, UnityEditorProcessState, UnityLaunchMode, UnityTestPollWake,
        RECOMPILE_START_STATE_HISTORY_LIMIT, UNITY_EDITOR_STATUS_EDITING,
        UNITY_EDITOR_STATUS_PLAYING,
    };
    use serde_json::json;
    use std::time::Duration;

    fn result_file(summary: &str) -> String {
        summary
            .lines()
            .find_map(|line| line.strip_prefix("result_file: "))
            .expect("result_file field")
            .to_string()
    }

    fn pipe_response(ok: bool, message: Option<&str>, error: Option<&str>) -> PipeResponse {
        PipeResponse {
            ok,
            error: error.map(ToOwned::to_owned),
            message: message.map(ToOwned::to_owned),
            process_id: None,
            process_path: None,
        }
    }

    fn test_connection_status(project_path: &str, checked_at_ms: u64) -> UnityConnectionStatus {
        UnityConnectionStatus {
            connected: true,
            editor_status: super::UNITY_EDITOR_STATUS_PLAYING.to_string(),
            control_channel_state: "ready".to_string(),
            scene_path: Some("Assets/Scenes/Main.unity".to_string()),
            scene_paths: vec!["Assets/Scenes/Main.unity".to_string()],
            editor_process_state: UnityEditorProcessState::Running,
            editor_process_id: Some(42),
            editor_process_path: Some("C:/Unity/Unity.exe".to_string()),
            editor_project_path: Some(project_path.to_string()),
            launch_mode: Some(UnityLaunchMode::Interactive),
            headless: false,
            process_checked_at_ms: Some(checked_at_ms),
            process_last_error: None,
            pipe_name: "test-pipe".to_string(),
            latency_ms: Some(12),
            reconnect_attempts: 0,
            last_error: None,
            background_hook: UnityBackgroundHookStatus {
                enabled: false,
                supported: true,
                state: UnityBackgroundHookState::Disabled,
                patched: false,
                process_id: None,
                editor_process_path: None,
                symbol_count: 0,
                error: None,
                updated_at_ms: checked_at_ms,
            },
            checked_at_ms,
        }
    }

    #[test]
    fn unity_status_message_includes_all_open_scene_paths() {
        let (status, active_scene_path, scene_paths) = parse_unity_status_message(
            "editing|Assets/Scenes/Main.unity|Assets/Scenes/World_SubScene.unity|Assets/Scenes/Main.unity",
        );

        assert_eq!(status, UNITY_EDITOR_STATUS_EDITING);
        assert_eq!(
            active_scene_path.as_deref(),
            Some("Assets/Scenes/Main.unity")
        );
        assert_eq!(
            scene_paths,
            vec![
                "Assets/Scenes/Main.unity".to_string(),
                "Assets/Scenes/World_SubScene.unity".to_string(),
            ]
        );
    }

    #[test]
    fn managed_reload_errors_are_retryable_transient_broker_responses() {
        for error in [
            "managed_reloading",
            "managed_not_ready",
            "domain_reload_interrupted",
        ] {
            assert!(is_transient_broker_error(error), "{error}");
            assert!(pipe_response_transient_broker_error(&PipeResponse {
                ok: false,
                error: Some(error.to_string()),
                message: None,
                process_id: None,
                process_path: None,
            }));
        }

        assert!(!pipe_response_transient_broker_error(&PipeResponse {
            ok: false,
            error: Some("native_queue_full".to_string()),
            message: None,
            process_id: None,
            process_path: None,
        }));
    }

    #[test]
    fn recompile_start_ack_requires_the_compilation_started_event_response() {
        assert_eq!(
            classify_recompile_start_response(&pipe_response(
                true,
                Some("recompile_started"),
                None,
            )),
            Ok(RecompileStartAck::Started)
        );
        let error =
            classify_recompile_start_response(&pipe_response(true, Some("request_queued"), None))
                .expect_err("a queued request is not a started compilation");
        assert!(error.contains("没有开始编译"), "{error}");

        assert_eq!(
            classify_recompile_start_response(&pipe_response(
                false,
                None,
                Some("managed_reloading"),
            )),
            Ok(RecompileStartAck::ReloadBoundary)
        );
    }

    #[test]
    fn recompile_polling_requires_persisted_ok_after_reconnect() {
        for state in ["starting", "pending"] {
            assert_eq!(
                classify_recompile_poll_response(&pipe_response(true, Some(state), None)),
                Ok(RecompilePollState::Waiting)
            );
        }
        assert_eq!(
            classify_recompile_poll_response(&pipe_response(true, Some("ok"), None)),
            Ok(RecompilePollState::Completed)
        );
        assert_eq!(
            classify_recompile_poll_response(&pipe_response(true, Some("not_needed"), None)),
            Ok(RecompilePollState::NotNeeded)
        );
        assert_eq!(
            classify_recompile_poll_response(&pipe_response(
                false,
                None,
                Some("managed_not_ready"),
            )),
            Ok(RecompilePollState::Transient)
        );
        assert_eq!(
            classify_recompile_poll_response(&pipe_response(
                false,
                None,
                Some("recompile_result_operation_mismatch"),
            )),
            Ok(RecompilePollState::OperationMismatch)
        );
        // A stalled main thread is a reason to keep waiting, not a failure.
        assert_eq!(
            classify_recompile_poll_response(&pipe_response(
                false,
                None,
                Some("native_main_thread_unresponsive"),
            )),
            Ok(RecompilePollState::EditorBusy)
        );

        let error = classify_recompile_poll_response(&pipe_response(
            false,
            None,
            Some("Unity 没有开始编译。"),
        ))
        .expect_err("no compilation is terminal");
        assert!(error.contains("没有开始编译"), "{error}");
    }

    #[test]
    fn recompile_start_rejections_are_resendable_or_adoptable() {
        // Rejected before it reached Unity: safe to resend later.
        assert_eq!(
            classify_recompile_start_response(&pipe_response(
                false,
                None,
                Some("native_main_thread_unresponsive"),
            )),
            Ok(RecompileStartAck::EditorBusyNotSent)
        );
        // Current plugins name the active operation.
        assert_eq!(
            classify_recompile_start_response(&pipe_response(
                false,
                None,
                Some("recompile_in_progress; operation_id=recompile-abc; Unity 重编译请求正在进行中。"),
            )),
            Ok(RecompileStartAck::InProgress {
                operation_id: Some("recompile-abc".to_string())
            })
        );
        // Older plugins only say so in text.
        assert_eq!(
            classify_recompile_start_response(&pipe_response(
                false,
                None,
                Some("Unity 重编译请求正在进行中。"),
            )),
            Ok(RecompileStartAck::InProgress { operation_id: None })
        );
        assert_eq!(
            recompile_in_progress_operation("recompile_in_progress; operation_id=; text"),
            Some(None)
        );
        let error = classify_recompile_start_response(&pipe_response(
            false,
            None,
            Some("Unity 没有开始编译。 请求编译失败：boom"),
        ))
        .expect_err("a real start failure stays terminal");
        assert!(error.contains("boom"), "{error}");
    }

    #[test]
    fn a_follow_up_compile_after_a_no_op_verdict_counts_as_compiled() {
        let settled = |session: &str, serial: i64| ReloadStateMessage {
            session_id: session.to_string(),
            domain_generation: "g".to_string(),
            converged_serial: serial,
            is_compiling: false,
            is_updating: false,
        };
        let baseline = ReloadBaseline {
            session_id: "s1".to_string(),
            converged_serial: 4,
        };
        assert!(!follow_up_compile_converged(Some(&baseline), &settled("s1", 4)));
        assert!(follow_up_compile_converged(Some(&baseline), &settled("s1", 5)));
        // A different editor process loaded the current sources from scratch.
        assert!(follow_up_compile_converged(Some(&baseline), &settled("s2", 0)));
        // Without a baseline nothing can be claimed.
        assert!(!follow_up_compile_converged(None, &settled("s1", 9)));

        // Older plugins omit the flags: settled, as before.
        let legacy: ReloadStateMessage = serde_json::from_str(
            r#"{"session_id":"s1","domain_generation":"g","converged_serial":4}"#,
        )
        .unwrap();
        assert!(!legacy.is_compiling && !legacy.is_updating);
    }

    #[test]
    fn editor_busy_verdict_waits_within_budget_unless_the_timeline_says_stuck() {
        assert_eq!(
            editor_busy_verdict(false, true, Duration::from_secs(600)),
            EditorBusyVerdict::Responsive
        );
        assert_eq!(
            editor_busy_verdict(true, false, Duration::from_secs(45)),
            EditorBusyVerdict::KeepWaiting
        );
        assert_eq!(
            editor_busy_verdict(true, true, Duration::from_secs(5)),
            EditorBusyVerdict::GiveUp
        );
        assert_eq!(
            editor_busy_verdict(true, false, RECOMPILE_UNRESPONSIVE_BUDGET),
            EditorBusyVerdict::GiveUp
        );
        assert!(RECOMPILE_UNRESPONSIVE_BUDGET < RECOMPILE_TOTAL_TIMEOUT);
    }

    #[test]
    fn recompile_poll_response_timeout_keeps_the_idempotent_result_path() {
        assert!(is_recompile_poll_response_timeout(
            "Unity response timed out"
        ));
        assert!(is_recompile_poll_response_timeout(
            "Unity request timed out"
        ));
        assert!(!is_recompile_poll_response_timeout(
            "Unity pipe connection closed"
        ));
    }

    #[test]
    fn recompile_start_diagnostics_coalesce_stable_state_and_bound_history() {
        let mut diagnostics = RecompileStartDiagnostics::default();
        diagnostics.broker_accepted = true;
        diagnostics.record_formatted_state(5, "editing".to_string(), "idle at 5s".to_string());
        diagnostics.record_formatted_state(10, "editing".to_string(), "idle at 10s".to_string());

        assert_eq!(diagnostics.samples.len(), 1);
        assert_eq!(diagnostics.samples[0].first_elapsed_secs, 5);
        assert_eq!(diagnostics.samples[0].last_elapsed_secs, 10);
        let output = diagnostics.format_log("start timed out");
        assert!(output.contains("Native Broker 已接收请求：是"), "{output}");
        assert!(output.contains("5-10s: idle at 10s"), "{output}");

        for index in 0..RECOMPILE_START_STATE_HISTORY_LIMIT + 2 {
            diagnostics.record_formatted_state(
                20 + index as u64,
                format!("state-{index}"),
                format!("detail-{index}"),
            );
        }
        assert_eq!(
            diagnostics.samples.len(),
            RECOMPILE_START_STATE_HISTORY_LIMIT
        );
        assert_eq!(
            diagnostics
                .samples
                .last()
                .map(|sample| sample.signature.as_str()),
            Some("state-7")
        );
    }

    #[test]
    fn transient_status_failure_reuses_recent_running_status() {
        let project_path = format!("F:/Proj/Game/cache-test-{}", std::process::id());
        let status = test_connection_status(&project_path, 1_000);
        cache_unity_connection_status(&project_path, &status);

        let cached = cached_running_connection_status_for_transient_failure(
            &project_path,
            1_500,
            "writer busy",
            true,
        )
        .expect("recent running status should be reused");

        assert!(cached.connected);
        assert_eq!(cached.control_channel_state, "busy");
        assert_eq!(cached.editor_status, super::UNITY_EDITOR_STATUS_PLAYING);
        assert_eq!(cached.checked_at_ms, 1_500);
        assert_eq!(cached.latency_ms, None);
        assert_eq!(cached.last_error.as_deref(), Some("writer busy"));
        assert!(cached_running_connection_status_for_transient_failure(
            &project_path,
            20_000,
            "stale",
            true,
        )
        .is_none());

        let disconnected = cached_running_connection_status_for_transient_failure(
            &project_path,
            1_500,
            "pipe closed",
            false,
        )
        .expect("recent process metadata should remain available");
        assert!(!disconnected.connected);
        assert_eq!(disconnected.control_channel_state, "error");
    }

    #[test]
    fn connection_detail_signature_ignores_check_timing_only() {
        let path = "F:/Proj/Game/signature";
        let first = test_connection_status(path, 1_000);
        let mut later = test_connection_status(path, 9_000);
        later.latency_ms = Some(80);
        later.reconnect_attempts = 3;
        assert_eq!(
            super::connection_status_signature(&first),
            super::connection_status_signature(&later)
        );
        later.editor_process_state = UnityEditorProcessState::NotRunning;
        assert_ne!(
            super::connection_status_signature(&first),
            super::connection_status_signature(&later)
        );
    }

    #[test]
    fn semantic_push_signature_ignores_sample_timing_but_not_the_state() {
        let state = |phase: &str, checked_at_ms: u64, heartbeat: u64| {
            let mut value = serde_json::json!({
                "phase": phase, "source": "pipe", "confidence": "high", "transient": false,
                "checkedAtMs": checked_at_ms,
                "process": { "state": "running" },
                "channel": { "controlPipe": "ready", "lastLatencyMs": heartbeat },
                "domain": { "phase": "none", "source": "pipe", "confidence": "high" },
                "editorMode": { "value": "editing", "source": "pipe", "confidence": "high" },
                "mainThread": { "state": "responsive", "cpuActive": false, "quiescentForMs": heartbeat,
                    "heartbeatAgeMs": heartbeat },
                "safety": { "canCallUnityApi": true, "canModifyAssetsSafely": true, "recommendedAction": "proceed" },
                "statePlane": { "observer": "cache", "dataPlane": "native", "nativeBroker": format!("ready:{checked_at_ms}"),
                    "nativeHook": "available", "historySamples": checked_at_ms },
                "editorLog": { "source": "editor_log", "safeMode": false, "modifiedAtMs": checked_at_ms },
            });
            if phase == "quit" {
                value["process"]["state"] = serde_json::json!("not_running");
            }
            serde_json::from_value::<super::SemanticState>(value).expect("semantic state")
        };
        let signature = super::semantic_state_signature;
        assert_eq!(signature(&state("editing", 1, 10)), signature(&state("editing", 2, 900)));
        assert_ne!(signature(&state("editing", 1, 10)), signature(&state("quit", 1, 10)));
    }

    #[test]
    fn closed_editor_does_not_reuse_or_republish_an_older_running_sample() {
        let path = format!("F:/Proj/Game/closed-cache-{}", uuid::Uuid::new_v4());
        let old = test_connection_status(&path, 1_000);
        cache_unity_connection_status(&path, &old);
        super::invalidate_closed_editor_status(&path);
        cache_unity_connection_status(&path, &old);
        assert!(cached_running_connection_status_for_transient_failure(&path,
            super::unix_now_ms(), "pipe closed", false).is_none());
        let fresh = test_connection_status(&path, super::unix_now_ms() + 1);
        cache_unity_connection_status(&path, &fresh);
        assert!(cached_running_connection_status_for_transient_failure(&path,
            fresh.checked_at_ms, "writer busy", true).is_some());
    }

    #[test]
    fn native_background_hook_markers_require_native_and_hook_markers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_path = temp.path().to_string_lossy().to_string();

        assert!(!native_background_hook_markers_present(&project_path));

        super::sync_native_bridge_marker(&project_path, true).expect("write native marker");
        assert!(!native_background_hook_markers_present(&project_path));

        super::sync_background_hook_marker(&project_path, true).expect("write hook marker");
        assert!(native_background_hook_markers_present(&project_path));

        super::sync_background_hook_marker(&project_path, false).expect("remove hook marker");
        assert!(!native_background_hook_markers_present(&project_path));
    }

    #[test]
    fn unity_embed_disabled_marker_preserves_default_enabled_behavior() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_path = temp.path().to_string_lossy().to_string();
        let marker = temp
            .path()
            .join("Library")
            .join("Locus")
            .join("UnityEmbed.disabled");

        super::sync_unity_embed_enabled_marker(&project_path, true).expect("keep embed enabled");
        assert!(!marker.exists());

        super::sync_unity_embed_enabled_marker(&project_path, false).expect("disable embed");
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "disabled\n");

        super::sync_unity_embed_enabled_marker(&project_path, true).expect("enable embed");
        assert!(!marker.exists());
    }

    #[test]
    fn relative_asset_paths_strip_root_case_insensitively_keeping_disk_casing() {
        let rels = relative_asset_paths(
            r"F:\Proj\Game",
            &[
                r"f:\proj\game\Assets\Scripts\Foo.cs".to_string(),
                "F:/Proj/Game/Assets/Bar.cs".to_string(),
                r"D:\Elsewhere\Assets\Baz.cs".to_string(),
                r"F:\Proj\Game".to_string(),
            ],
        );
        assert_eq!(rels, vec!["Assets/Scripts/Foo.cs", "Assets/Bar.cs"]);
    }

    #[test]
    fn read_project_unity_version_extracts_editor_version() {
        let project = tempfile::tempdir().expect("temp project");
        let settings_dir = project.path().join("ProjectSettings");
        std::fs::create_dir_all(&settings_dir).expect("create settings dir");
        std::fs::write(
            settings_dir.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\nm_EditorVersionWithRevision: 2022.3.47f1 (88c277b85d21)\n",
        )
        .expect("write version");

        let version =
            read_project_unity_version(&project.path().to_string_lossy()).expect("read version");
        assert_eq!(version.as_deref(), Some("2022.3.47f1"));
    }

    #[test]
    fn run_states_requested_editor_status_accepts_supported_statuses() {
        let request = json!({ "request_editor_status": " playing_paused " });

        assert_eq!(
            requested_run_states_editor_status(&request).unwrap(),
            "playing_paused"
        );
    }

    #[test]
    fn play_mode_tool_modes_map_to_canonical_editor_statuses() {
        assert_eq!(
            play_mode_target_status("play").unwrap(),
            UNITY_EDITOR_STATUS_PLAYING
        );
        assert_eq!(
            play_mode_target_status(" edit ").unwrap(),
            UNITY_EDITOR_STATUS_EDITING
        );
        assert!(play_mode_target_status("pause")
            .unwrap_err()
            .contains("Allowed values: play, edit"));
    }

    #[test]
    fn run_states_requested_editor_status_rejects_missing_or_invalid_status() {
        assert!(requested_run_states_editor_status(&json!({}))
            .unwrap_err()
            .contains("Missing required parameter"));

        assert!(requested_run_states_editor_status(&json!({
            "request_editor_status": "disconnected"
        }))
        .unwrap_err()
        .contains("Invalid request_editor_status"));

        assert!(requested_run_states_editor_status(&json!({
            "request_editor_status": "compiling"
        }))
        .unwrap_err()
        .contains("Invalid request_editor_status"));
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

    #[test]
    fn unity_test_start_request_has_single_start_semantics() {
        let request = json!({
            "assemblies": ["SampleGame.Replay.Tests"],
            "async": "sync",
            "categories": [],
            "groups": ["CameraRuntimeTests"],
            "mode": "edit",
            "result_detail": "failures",
            "run_id": "caller-value",
            "tests": [],
            "timeout_ms": 600000
        });

        let start_request = unity_test_start_request(&request, "new-run-id").unwrap();
        assert_eq!(start_request["run_id"], "new-run-id");
        assert_eq!(start_request["groups"], json!(["CameraRuntimeTests"]));
    }

    #[test]
    fn unity_test_cancel_snapshot_surfaces_compile_time_unavailable_cancellation() {
        let response = PipeResponse {
            ok: true,
            error: None,
            message: Some(
                json!({
                    "run_id": "run-1",
                    "status": "error",
                    "error": "Unity Test cancellation requires com.unity.test-framework 1.4.0 or newer."
                })
                .to_string(),
            ),
            process_id: None,
            process_path: None,
        };

        let error = unity_test_cancel_snapshot(&response).unwrap_err();
        assert!(error.starts_with("unavailable:"));
        assert!(error.contains("requires com.unity.test-framework 1.4.0"));
    }

    #[test]
    fn unity_test_cancel_snapshot_accepts_pending_utf_reconciliation() {
        let response = PipeResponse {
            ok: true,
            error: None,
            message: Some(
                json!({
                    "run_id": "run-1",
                    "status": "cancelling",
                    "error": ""
                })
                .to_string(),
            ),
            process_id: None,
            process_path: None,
        };

        let snapshot = unity_test_cancel_snapshot(&response)
            .unwrap()
            .expect("pending cancellation snapshot");
        assert_eq!(snapshot.status, "cancelling");
        assert!(snapshot.error.is_empty());
    }

    #[test]
    fn unity_test_cancel_dispatch_retries_transient_broker_states() {
        let response = PipeResponse {
            ok: false,
            error: Some("managed_reloading".to_string()),
            message: None,
            process_id: None,
            process_path: None,
        };

        let error = unity_test_cancel_dispatch_response(response).unwrap_err();
        assert!(error.contains("managed_reloading"));
        assert!(is_transient_broker_error(&error));
    }

    #[test]
    fn unity_test_terminal_statuses_cover_cancel_and_completion() {
        for status in ["cancelled", "error", "passed", "failed"] {
            assert!(unity_test_run_is_terminal(status), "{status}");
        }
        assert!(!unity_test_run_is_terminal("running"));
        assert!(!unity_test_run_is_terminal("cancelling"));
    }

    #[test]
    fn unity_test_short_name_keeps_fixture_and_parameter_dots() {
        assert_eq!(
            unity_test_short_name("Game.Tests.PlayerTests.Moves"),
            "PlayerTests.Moves"
        );
        assert_eq!(
            unity_test_short_name("Game.Tests.MathTests.Scale(1.5f,\"a.b\")"),
            "MathTests.Scale(1.5f,\"a.b\")"
        );
        assert_eq!(unity_test_short_name("Fixture.Case"), "Fixture.Case");
        assert_eq!(unity_test_short_name("Case"), "Case");
        assert_eq!(unity_test_short_name(""), "");
    }

    #[test]
    fn unity_test_progress_reports_completed_ratio_failures_and_current_test() {
        let snapshot = UnityTestRunSnapshot {
            status: "running".to_string(),
            current_test: "Game.Tests.PlayerTests.Jumps".to_string(),
            total: 8,
            passed: 2,
            failed: 1,
            skipped: 1,
            ..Default::default()
        };
        let update = unity_test_progress_update(&snapshot);
        assert_eq!(update.title, "Running Unity tests");
        assert_eq!(update.info, "4/8 · 1 failed · PlayerTests.Jumps");
        assert_eq!(update.progress, Some(0.5));

        let starting = unity_test_progress_update(&UnityTestRunSnapshot {
            status: "starting".to_string(),
            total: 8,
            ..Default::default()
        });
        assert_eq!(starting.title, "Starting Unity tests");
        assert_eq!(starting.progress, None);

        let undiscovered = unity_test_progress_update(&UnityTestRunSnapshot {
            status: "running".to_string(),
            ..Default::default()
        });
        assert_eq!(undiscovered.info, "");
        assert_eq!(undiscovered.progress, None);
    }

    #[test]
    fn unity_test_progress_skips_repeated_poll_snapshots() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = sent.clone();
        let mut progress = UnityTestProgress::new(Some(std::sync::Arc::new(move |update| {
            sink.lock().unwrap().push(update);
        })));
        let mut snapshot = UnityTestRunSnapshot {
            status: "running".to_string(),
            total: 2,
            ..Default::default()
        };
        progress.snapshot(&snapshot);
        progress.snapshot(&snapshot);
        snapshot.passed = 1;
        progress.snapshot(&snapshot);
        progress.stage("Cancelling Unity tests", "");
        progress.stage("Cancelling Unity tests", "");

        let sent = sent.lock().unwrap();
        let infos = sent
            .iter()
            .map(|update| (update.title.as_str(), update.info.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            infos,
            vec![
                ("Running Unity tests", "0/2"),
                ("Running Unity tests", "1/2"),
                ("Cancelling Unity tests", ""),
            ]
        );
    }

    #[tokio::test]
    async fn unity_test_poll_wait_is_interrupted_by_a_dialog_revision() {
        let (dialog_tx, mut dialog_rx) = tokio::sync::watch::channel(0u64);
        dialog_tx.send_replace(1);
        let wake = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            wait_for_unity_test_poll_wake(&mut dialog_rx, None),
        )
        .await
        .expect("dialog revision should interrupt the 250ms status interval");
        assert_eq!(wake, UnityTestPollWake::DialogChanged);
    }

    const UNITY_HUB_EDITORS_SAMPLE: &str = r#"{
      "schema_version": "2",
      "data": [
        {"version":"2022.3.2f1","location":["E:\\Unity 2022.3.2f1\\Editor\\Unity.exe"],"manual":true,"architecture":"x86_64","buildPlatforms":[],"requiresUlf":false},
        {"version":"6000.3.14f1","location":["E:/Unity 6.3/Editor/Unity.exe"],"manual":true,"architecture":"x86_64"},
        {"version":"2021.3.45f1","location":["F:\\UnityEditor\\2021.3.45f1\\Editor\\Unity.exe"],"manual":true}
      ]
    }"#;

    #[test]
    fn unity_hub_locations_match_version_including_non_default_drives() {
        use std::path::PathBuf;
        // Editor installed on a non-default drive is still resolved.
        assert_eq!(
            parse_unity_hub_editor_locations(UNITY_HUB_EDITORS_SAMPLE, "2021.3.45f1"),
            vec![PathBuf::from(
                r"F:\UnityEditor\2021.3.45f1\Editor\Unity.exe"
            )]
        );
        // ProjectVersion.txt parsing may leave surrounding whitespace.
        assert_eq!(
            parse_unity_hub_editor_locations(UNITY_HUB_EDITORS_SAMPLE, "  2022.3.2f1 "),
            vec![PathBuf::from(r"E:\Unity 2022.3.2f1\Editor\Unity.exe")]
        );
        // Forward-slash locations (common for Unity 6 manual adds) are preserved.
        assert_eq!(
            parse_unity_hub_editor_locations(UNITY_HUB_EDITORS_SAMPLE, "6000.3.14f1"),
            vec![PathBuf::from("E:/Unity 6.3/Editor/Unity.exe")]
        );
    }

    #[test]
    fn unity_hub_locations_ignore_unknown_versions_and_invalid_cache() {
        assert!(
            parse_unity_hub_editor_locations(UNITY_HUB_EDITORS_SAMPLE, "2019.4.0f1").is_empty()
        );
        assert!(parse_unity_hub_editor_locations("not json", "2022.3.2f1").is_empty());
        assert!(parse_unity_hub_editor_locations("{}", "2022.3.2f1").is_empty());
        assert!(parse_unity_hub_editor_locations(r#"{"data":[]}"#, "2022.3.2f1").is_empty());
    }

    #[test]
    fn unity_hub_locations_skip_blank_paths_and_collect_all_matches() {
        let cache = r#"{"data":[
            {"version":"2022.3.2f1","location":["","   "]},
            {"version":"2022.3.2f1","location":["D:\\Apps\\Unity\\Hub\\Editor\\2022.3.2f1\\Editor\\Unity.exe"]}
        ]}"#;
        assert_eq!(
            parse_unity_hub_editor_locations(cache, "2022.3.2f1"),
            vec![std::path::PathBuf::from(
                r"D:\Apps\Unity\Hub\Editor\2022.3.2f1\Editor\Unity.exe"
            )]
        );
    }
}

#[cfg(test)]
mod unity_execute_contract_tests {
    use super::*;
    use serde_json::json;

    fn args(extra: serde_json::Value) -> serde_json::Value {
        let mut value = json!({
            "code": "print(1);",
            "readonly": true,
            "request_editor_status": "editing",
        });
        for (key, item) in extra.as_object().unwrap() {
            value[key] = item.clone();
        }
        value
    }

    #[test]
    fn parameters_match_the_published_tool_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(crate::prompt::tools::UNITY_EXECUTE).unwrap();
        let mut published = schema["parameters"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        published.sort();
        let mut accepted = UNITY_EXECUTE_PARAMETERS
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        accepted.sort();
        assert_eq!(published, accepted);
        assert_eq!(schema["parameters"]["additionalProperties"], false);
        assert_eq!(
            schema["parameters"]["properties"]["timeout"]["default"],
            UNITY_EXECUTE_DEFAULT_TIMEOUT_MS
        );
    }

    #[test]
    fn unknown_parameters_are_rejected_with_the_allowed_list() {
        let error = parse_unity_execute_args(&args(json!({"timeout_ms": 5000}))).unwrap_err();
        assert!(error.contains("Unknown unity_execute parameter(s): timeout_ms"), "{error}");
        assert!(error.contains("timeout (total run budget in milliseconds)"), "{error}");
        // The async task runtime adds its own parameter.
        assert!(parse_unity_execute_args(&args(json!({"async": "sync"}))).is_ok());
        // The agent loop mirrors known keys in camelCase.
        let mirrored = args(json!({"requestEditorStatus": "editing", "enableNonPublicAccess": true}));
        assert_eq!(parse_unity_execute_args(&mirrored).unwrap().requested_status, "editing");
        let error = parse_unity_execute_args(&args(json!({"timeoutMs": 5000}))).unwrap_err();
        assert!(error.contains("timeoutMs"), "{error}");
    }


    #[test]
    fn timeout_is_optional_clamped_and_numeric() {
        let parsed = parse_unity_execute_args(&args(json!({}))).unwrap();
        assert_eq!(parsed.options.timeout_ms, None);
        assert_eq!(parsed.code, "print(1);");
        assert_eq!(parsed.requested_status, "editing");
        for (given, expected) in [
            (json!(120000), 120_000),
            (json!(1500000.0), 1_500_000),
            (json!(10), UNITY_EXECUTE_MIN_TIMEOUT_MS),
            (json!(99_000_000), UNITY_EXECUTE_MAX_TIMEOUT_MS),
        ] {
            let parsed = parse_unity_execute_args(&args(json!({"timeout": given}))).unwrap();
            assert_eq!(parsed.options.timeout_ms, Some(expected));
        }
        assert!(parse_unity_execute_args(&args(json!({"timeout": null})))
            .unwrap()
            .options
            .timeout_ms
            .is_none());
        for invalid in [json!("600000"), json!(-1), json!(true)] {
            let error = parse_unity_execute_args(&args(json!({"timeout": invalid}))).unwrap_err();
            assert!(error.contains("'timeout'"), "{error}");
        }
    }

    #[test]
    fn required_arguments_and_status_are_validated() {
        assert_eq!(
            parse_unity_execute_args(&json!({"request_editor_status": "editing"})).unwrap_err(),
            "Missing required parameter: code"
        );
        assert_eq!(
            parse_unity_execute_args(&json!({"code": "x"})).unwrap_err(),
            "Missing required parameter: request_editor_status"
        );
        let error =
            parse_unity_execute_args(&args(json!({"request_editor_status": "disconnected"})))
                .unwrap_err();
        assert!(error.starts_with("Invalid request_editor_status"), "{error}");
        assert!(parse_unity_execute_args(&json!("print(1);")).is_err());
    }

    #[test]
    fn payloads_carry_the_execution_id_and_optional_budget() {
        let code = unity_execute_code_with_markers("print(1);\n\n", "exec-1", Some(90_000));
        assert_eq!(
            code,
            "print(1);\n//__LOCUS_EXECUTION_ID__:exec-1\n//__LOCUS_TIMEOUT_MS__:90000"
        );
        assert_eq!(
            unity_execute_code_with_markers("print(1);", "exec-1", None),
            "print(1);\n//__LOCUS_EXECUTION_ID__:exec-1"
        );

        let payload = unity_execute_loaded_payload_with_execution_id(
            r#"{"assembly_b64":"AA=="}"#.to_string(),
            "exec-2",
            "print(2);",
            Some(30_000),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["execution_id"], "exec-2");
        assert_eq!(value["source_code"], "print(2);");
        assert_eq!(value["timeout_ms"], 30_000);
        let payload =
            unity_execute_loaded_payload_with_execution_id("{}".to_string(), "exec-3", "", None)
                .unwrap();
        assert!(!payload.contains("timeout_ms"));
    }

    fn native_snapshot(json: serde_json::Value) -> UnityExecuteProgressSnapshot {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn native_snapshots_parse_and_explain_a_busy_main_thread() {
        let snapshot = native_snapshot(json!({
            "active": true, "title": "Executing snippet", "info": "", "progress": 0,
            "revision": 9, "source": "stage", "executing": true, "elapsedMs": 21000,
            "timeoutMs": 600000, "idleMs": 1800, "idleLimitMs": 30000,
            "mainThreadBlockedMs": 20500, "maxMainThreadBlockedMs": 20500,
            "outputChars": 512, "lastOutput": "[walk-in s=7] maxGap avg=95",
            "stopRequested": false, "stopReason": "", "publishedAgeMs": 120,
            "mainThreadStallMs": 20000, "servedBy": "native_broker",
        }));
        assert_eq!(snapshot.main_thread_blocked_ms, Some(20_500));
        assert_eq!(snapshot.served_by, "native_broker");
        let shown = presentable_unity_execute_progress(snapshot.clone());
        assert_eq!(shown.title, "Executing snippet");
        assert_eq!(
            shown.info,
            "Unity main thread busy 20s; last output: [walk-in s=7] maxGap avg=95"
        );
        assert!(unity_execute_frozen_error(&snapshot, "exec-1").is_none());

        // Snippet-reported progress is shown as written.
        let api = native_snapshot(json!({
            "active": true, "title": "Locus", "info": "seed 3/12", "source": "api_info",
            "executing": true, "mainThreadBlockedMs": 9000,
        }));
        assert_eq!(presentable_unity_execute_progress(api).info, "seed 3/12");

        // Await snapshots name what they wait for.
        let await_snapshot = native_snapshot(json!({
            "active": true, "title": "Awaiting Unity", "source": "await",
            "waitTarget": "next EditorApplication.update", "waitedMs": 3400,
        }));
        assert_eq!(
            presentable_unity_execute_progress(await_snapshot).info,
            "next EditorApplication.update · 3s"
        );

        let stopping = native_snapshot(json!({
            "active": true, "title": "Executing snippet", "source": "stage",
            "executing": true, "stopRequested": true, "stopReason": "idle_timeout",
        }));
        let shown = presentable_unity_execute_progress(stopping);
        assert_eq!(shown.title, "Stopping snippet");
        assert_eq!(shown.info, "no print/ctx.Progress output for 30s");

        // Older plugins publish only the classic fields.
        let legacy: UnityExecuteProgressSnapshot = serde_json::from_str(
            r#"{"active":true,"title":"Executing snippet","info":"","progress":0,"revision":3,"source":"stage"}"#,
        )
        .unwrap();
        assert_eq!(presentable_unity_execute_progress(legacy).info, "");
    }

    #[test]
    fn tracker_keeps_the_longest_block_and_emits_liveness_changes() {
        let mut tracker = UnityExecuteProgressTracker::default();
        let mut snapshot = native_snapshot(json!({
            "active": true, "title": "Executing snippet", "revision": 4, "source": "stage",
            "executing": true, "mainThreadBlockedMs": 3000, "maxMainThreadBlockedMs": 2000,
        }));
        tracker.observe(&snapshot);
        assert_eq!(tracker.max_main_thread_blocked_ms, 3000);
        let shown = presentable_unity_execute_progress(snapshot.clone());
        assert!(tracker.should_emit(&shown));
        assert!(!tracker.should_emit(&shown));

        snapshot.main_thread_blocked_ms = Some(5_000);
        tracker.observe(&snapshot);
        assert!(tracker.should_emit(&presentable_unity_execute_progress(snapshot.clone())));

        snapshot.executing = Some(false);
        snapshot.main_thread_blocked_ms = Some(90_000);
        snapshot.max_main_thread_blocked_ms = Some(4_000);
        tracker.observe(&snapshot);
        assert_eq!(tracker.max_main_thread_blocked_ms, 5_000);
    }

    #[test]
    fn only_a_frozen_runtime_detaches_and_says_why() {
        let snapshot = |published_age: u64, stall: i64| {
            native_snapshot(json!({
                "active": true, "publishedAgeMs": published_age, "mainThreadStallMs": stall,
            }))
        };
        assert!(unity_execute_frozen_error(&snapshot(500, 120_000), "exec-1").is_none());
        assert!(unity_execute_frozen_error(&snapshot(40_000, 5_000), "exec-1").is_none());
        let error = unity_execute_frozen_error(&snapshot(31_000, 45_000), "exec-1").unwrap();
        assert!(error.starts_with(
            "native_main_thread_unresponsive; execution_id=exec-1; request_state=detached"
        ));
        assert!(error.contains("stopped reporting progress 31s ago"), "{error}");
        // Managed-served snapshots carry no broker timing.
        assert!(
            unity_execute_frozen_error(&native_snapshot(json!({"active": true})), "exec-1")
                .is_none()
        );

        let budget = unity_execute_budget_exceeded_error("exec-2", 600_000);
        assert!(budget.starts_with(UNITY_EXECUTE_BUDGET_EXCEEDED));
        assert!(budget.contains("600s timeout"), "{budget}");
        assert!(budget.contains("locus.wait_unity_execution"), "{budget}");
    }

    #[test]
    fn notes_and_waiting_text_are_readable() {
        assert!(unity_execute_long_block_note(14_999).is_none());
        let note = unity_execute_long_block_note(23_400).unwrap();
        assert!(note.contains("23.4s"), "{note}");
        assert!(note.contains("await ctx.WaitFrame()"), "{note}");
        assert_eq!(
            describe_progress_poll_error("Unity response timed out"),
            "Unity's main thread has not picked up the request yet"
        );
        assert_eq!(describe_progress_poll_error("pipe closed"), "pipe closed");
        assert_eq!(truncate_progress_text("  abcdef ", 3), "abc…");
        assert_eq!(truncate_progress_text("ab", 3), "ab");

        let deadline: PipeResponse =
            serde_json::from_value(json!({"ok": false, "error": "native_request_timed_out"}))
                .unwrap();
        assert!(pipe_response_is_request_deadline(&deadline));
        let reloading: PipeResponse =
            serde_json::from_value(json!({"ok": false, "error": "managed_reloading"})).unwrap();
        assert!(!pipe_response_is_request_deadline(&reloading));
    }
}
