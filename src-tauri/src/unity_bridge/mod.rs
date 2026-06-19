mod background_hook;
mod capture;
mod focus;
mod native_selftest;
mod plugin;
mod process;
mod state_probe;
mod transport;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

pub use background_hook::{UnityBackgroundHookState, UnityBackgroundHookStatus};
pub use capture::{capture_viewport, UnityViewportCapture};
pub use plugin::{
    check_plugin_install_plan, check_plugin_status, emit_plugin_status, find_plugin_source_dir,
    install_or_update_plugin, install_or_update_plugin_with_force_close, plugin_install_root,
    plugin_skills_root, PluginInstallPlan, PluginStatus,
};
pub use process::{
    query_current_project_editor_process, UnityEditorProcessInfo, UnityEditorProcessState,
};
pub use state_probe::{SemanticState, UnityStateProbeStatus, UnityStateProbeTier};
pub use transport::{
    send_message, send_message_with_timeout, send_message_without_timeout, set_event_app_handle,
};

pub fn initialize_background_hook(enabled: bool) {
    background_hook::initialize(enabled);
}

pub fn set_background_hook_enabled(value: bool) -> Result<UnityBackgroundHookStatus, String> {
    background_hook::set_enabled(value)
}

pub fn background_hook_status() -> UnityBackgroundHookStatus {
    background_hook::status()
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

pub fn state_probe_status() -> UnityStateProbeStatus {
    state_probe::status()
}

pub fn start_unity_semantic_state_observer(project_path: &str) {
    state_probe::start_observer(project_path);
}

pub fn stop_unity_semantic_state_observers() {
    state_probe::stop_all_observers();
}

/// Fuse pipe + process + native signals into one semantic editor state.
pub async fn unity_semantic_state(project_path: &str) -> SemanticState {
    state_probe::semantic_state_for_project(project_path).await
}

pub async fn run_state_probe_selftest(
    app: tauri::AppHandle,
    project: String,
) -> Result<(), String> {
    state_probe::selftest::run(app, project).await
}

pub async fn run_native_bridge_selftest(
    app: tauri::AppHandle,
    project: String,
) -> Result<(), String> {
    native_selftest::run(app, project).await
}

// ── Native broker bridge ─────────────────────────────────────────────
//
// When enabled, the Tauri↔Unity command channel is served by the native
// broker DLL (`locus_native`) loaded inside the Unity process. The broker's
// pipe outlives domain reloads, so the connection no longer drops every time
// the editor recompiles. The toggle is global (a config flag) but takes effect
// per project via a marker file the Unity plugin checks before loading the DLL;
// the native broker is the required Unity command transport.

static NATIVE_BRIDGE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn initialize_native_bridge(enabled: bool) {
    NATIVE_BRIDGE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_native_bridge_enabled(value: bool) {
    NATIVE_BRIDGE_ENABLED.store(value, Ordering::Relaxed);
}

pub fn native_bridge_enabled() -> bool {
    NATIVE_BRIDGE_ENABLED.load(Ordering::Relaxed)
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

const NATIVE_BROKER_STATE_MMF_MAGIC: u32 = 0x424e_434c; // "LCNB" little-endian.
const NATIVE_BROKER_STATE_MMF_VERSION: u16 = 1;
const NATIVE_BROKER_STATE_MMF_HEADER_SIZE: usize = 64;
const NATIVE_BROKER_STATE_MMF_SLOT_COUNT: usize = 8;
const NATIVE_BROKER_STATE_MMF_SLOT_SIZE: usize = 128 * 1024;

fn native_broker_state_mmf_name(project_path: &str) -> String {
    format!(
        r"Local\LocusNativeBrokerState_{}",
        project_state_plane_key(project_path)
    )
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

    use super::{
        NativeBrokerStatusPayload, NATIVE_BROKER_STATE_MMF_HEADER_SIZE,
        NATIVE_BROKER_STATE_MMF_MAGIC, NATIVE_BROKER_STATE_MMF_SLOT_COUNT,
        NATIVE_BROKER_STATE_MMF_SLOT_SIZE, NATIVE_BROKER_STATE_MMF_VERSION,
    };

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
        let total_size = NATIVE_BROKER_STATE_MMF_HEADER_SIZE.saturating_add(
            NATIVE_BROKER_STATE_MMF_SLOT_COUNT.saturating_mul(NATIVE_BROKER_STATE_MMF_SLOT_SIZE),
        );
        let name = wide_null(mapping_name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, FALSE, name.as_ptr()) };
        let handle = OwnedHandle::new(handle)?;
        let view = MappedView::new(handle.raw(), total_size)?;
        let bytes = view.bytes(total_size);

        let magic = read_u32(bytes, 0)?;
        let version = read_u16(bytes, 4)?;
        let slot_count = read_u16(bytes, 6)? as usize;
        let slot_size = read_u32(bytes, 8)? as usize;
        let writer_seq = read_u64(bytes, 16)?;
        if magic != NATIVE_BROKER_STATE_MMF_MAGIC
            || version != NATIVE_BROKER_STATE_MMF_VERSION
            || slot_count == 0
            || slot_count > NATIVE_BROKER_STATE_MMF_SLOT_COUNT
            || slot_size < 64
            || slot_size > NATIVE_BROKER_STATE_MMF_SLOT_SIZE
            || writer_seq == 0
        {
            return None;
        }

        let slot_index = ((writer_seq - 1) as usize) % slot_count;
        let slot_offset =
            NATIVE_BROKER_STATE_MMF_HEADER_SIZE.checked_add(slot_index.checked_mul(slot_size)?)?;
        let slot_end = slot_offset.checked_add(slot_size)?;
        if slot_end > bytes.len() {
            return None;
        }
        let slot = &bytes[slot_offset..slot_end];
        let slot_seq_before = read_u64(slot, 0)?;
        if slot_seq_before != writer_seq {
            return None;
        }
        let observed_at_ms = read_u64(slot, 8)?;
        let payload_len = read_u32(slot, 20)? as usize;
        let payload_offset = 24;
        if payload_len == 0 || payload_len > slot_size.saturating_sub(payload_offset) {
            return None;
        }
        let payload_bytes = slot
            .get(payload_offset..payload_offset + payload_len)?
            .to_vec();
        let slot_seq_after = read_u64(slot, 0)?;
        let writer_seq_after = read_u64(bytes, 16)?;
        if slot_seq_after != slot_seq_before || writer_seq_after != writer_seq {
            return None;
        }
        let payload = std::str::from_utf8(&payload_bytes).ok()?;
        let mut parsed = serde_json::from_str::<NativeBrokerStatusPayload>(payload).ok()?;
        if parsed.status.observed_at_ms <= 0 {
            parsed.status.observed_at_ms = observed_at_ms.min(i64::MAX as u64) as i64;
        }
        Some(parsed)
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ))
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }

    fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
        Some(u64::from_le_bytes(
            bytes.get(offset..offset + 8)?.try_into().ok()?,
        ))
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

pub type UnityMonitorHandle = Arc<tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>;

pub const UNITY_EDITOR_STATUS_DISCONNECTED: &str = "disconnected";
pub const UNITY_EDITOR_STATUS_EDITING: &str = "editing";
pub const UNITY_EDITOR_STATUS_PLAYING: &str = "playing";
pub const UNITY_EDITOR_STATUS_PLAYING_PAUSED: &str = "playing_paused";
pub const UNITY_EDITOR_STATUS_SCHEMA: &str = "disconnected | editing | playing | playing_paused";
const UNITY_STATUS_POLL_TIMEOUT: Duration = Duration::from_millis(800);
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

pub const UNITY_EXECUTE_PROGRESS_TAG: &str = "locus-unity-progress";
pub const UNITY_EXECUTE_CANCELLED: &str = "__locus_unity_execute_cancelled__";
const UNITY_EXECUTE_PROGRESS_POLL_MS: u64 = 250;
const UNITY_EXECUTE_START_TIMEOUT_SECS: u64 = 15;
const UNITY_EXECUTE_PROGRESS_LOST_TIMEOUT_SECS: u64 = 120;
const UNITY_EXECUTE_WAITING_STATUS_INTERVAL_MS: u64 = 2_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityLaunchResult {
    pub editor_path: String,
    pub project_path: String,
    pub project_version: String,
    pub process_id: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConnectionStatus {
    pub connected: bool,
    pub editor_status: String,
    pub control_channel_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_path: Option<String>,
    pub editor_process_state: UnityEditorProcessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_project_path: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

fn unity_connection_status_cache() -> &'static StdMutex<HashMap<String, UnityConnectionStatus>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, UnityConnectionStatus>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn project_runtime_key(project_path: &str) -> String {
    strip_extended_path_prefix(project_path).trim().to_string()
}

fn normalize_project_path_for_state_plane(project_path: &str) -> String {
    let trimmed = strip_extended_path_prefix(project_path).trim();
    let mut value = trimmed.replace('/', "\\");
    while value.ends_with('\\') && value.len() > 3 {
        value.pop();
    }
    value.to_ascii_lowercase()
}

pub(crate) fn project_state_plane_key(project_path: &str) -> String {
    let normalized = normalize_project_path_for_state_plane(project_path);
    let digest = Sha256::digest(normalized.as_bytes());
    let mut out = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
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
    let key = project_runtime_key(project_path);
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
    format!(
        "locus_unity_native_{}",
        project_state_plane_key(project_path)
    )
}

/// Full client path of the native broker pipe for this project.
pub(crate) fn get_native_pipe_name(project_path: &str) -> String {
    format!(r"\\.\pipe\{}", native_pipe_name_part(project_path))
}

pub fn is_unity_project(path: &str) -> bool {
    let p = Path::new(strip_extended_path_prefix(path));
    p.join("Assets").is_dir() && p.join("ProjectSettings").is_dir()
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
        push_unique_path(paths, root.join("Editor").join("Unity.exe"));
        push_unique_path(paths, root.join("Unity.exe"));
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

pub fn resolve_unity_editor_executable(version: &str) -> Result<PathBuf, String> {
    let version = version.trim();
    if version.is_empty() {
        return Err("Unity project version is empty".to_string());
    }

    let mut candidates = Vec::new();
    push_env_editor_candidates(&mut candidates);
    push_windows_registry_editor_candidates(&mut candidates, version);
    push_default_editor_candidates(&mut candidates, version);

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
    Err(format!(
        "Unity Editor {} was not found. Checked: {}",
        version, checked
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
    launch_project_with_options(project_path, None).await
}

pub async fn launch_project_with_options(
    project_path: &str,
    code_optimization: Option<UnityLaunchCodeOptimization>,
) -> Result<UnityLaunchResult, String> {
    if !is_unity_project(project_path) {
        return Err("Current working directory is not a Unity project".to_string());
    }

    let project_version = read_project_unity_version(project_path)?
        .ok_or_else(|| "Current Unity project is missing ProjectVersion.txt".to_string())?;
    let editor_path = resolve_unity_editor_executable(&project_version)?;
    let project_path = normalized_project_path_for_launch(project_path);

    let mut command = std::process::Command::new(&editor_path);
    command
        .arg("-projectPath")
        .arg(&project_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
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
        "[Locus] launched Unity Editor: editor='{}', project='{}', process_id={}",
        editor_path, project_path, process_id
    );

    Ok(UnityLaunchResult {
        editor_path,
        project_path,
        project_version,
        process_id,
    })
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

fn parse_unity_status_message(message: &str) -> (&'static str, Option<String>) {
    let (status_part, scene_part) = match message.split_once('|') {
        Some((status, scene)) => (status, Some(scene.trim().to_string())),
        None => (message, None),
    };
    (
        normalize_editor_status(status_part),
        scene_part.filter(|scene| !scene.is_empty()),
    )
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
        cache.insert(project_runtime_key(project_path), status.clone());
    }
}

fn cached_running_connection_status_for_transient_failure(
    project_path: &str,
    checked_at_ms: u64,
    error: impl Into<String>,
) -> Option<UnityConnectionStatus> {
    let error = error.into();
    let mut status = unity_connection_status_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&project_runtime_key(project_path)).cloned())?;
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
    status.connected = false;
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

async fn sync_background_hook_for_status(status: &mut UnityConnectionStatus, project_path: &str) {
    // The in-process native hook (when active) owns the patch and survives
    // domain reloads; the cross-process path then stands down.
    if let Some(native) = native_owned_background_hook(project_path).await {
        status.background_hook = native;
        return;
    }
    let Some(process_id) = status.editor_process_id else {
        let current = background_hook::status();
        if matches!(
            status.editor_process_state,
            UnityEditorProcessState::Running
        ) && current.enabled
            && current.patched
        {
            status.background_hook = current;
            return;
        }

        status.background_hook = if background_hook::enabled() {
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
            background_hook::status()
        };
        return;
    };

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
        return;
    };

    let hook_status = tauri::async_runtime::spawn_blocking(move || {
        background_hook::sync_for_process(process_id, &editor_process_path)
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
    status.background_hook = hook_status;
}

async fn query_process_info_for_connection_status(
    project_path: &str,
    connected: bool,
    process_hint: Option<UnityEditorProcessInfo>,
) -> UnityEditorProcessInfo {
    if !connected && unity_recompile_waiting(project_path) {
        if let Some(cached) = process::cached_project_editor_process(project_path).await {
            if cached.process_id.is_some() {
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

async fn query_unity_status_response_with_timeout(
    project_path: &str,
    timeout: Duration,
) -> Result<Option<(PipeResponse, u64)>, String> {
    let started_at = std::time::Instant::now();
    let response =
        transport::send_message_if_writer_free(project_path, "status", "", timeout).await?;
    let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Ok(response.map(|resp| (resp, latency_ms)))
}

pub async fn query_unity_connection_status(project_path: &str) -> UnityConnectionStatus {
    let pipe_name = get_native_pipe_name(project_path);
    let checked_at_ms = unix_now_ms();

    match query_unity_status_response_with_timeout(project_path, UNITY_STATUS_POLL_TIMEOUT).await {
        Ok(Some((resp, latency_ms))) if resp.ok => {
            let process_hint = process_hint_from_response(&resp, project_path, checked_at_ms);
            let message = resp.message.unwrap_or_default();
            let (editor_status, scene_path) = parse_unity_status_message(&message);
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
                editor_process_state: UnityEditorProcessState::Running,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: None,
                background_hook: background_hook::status(),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, true, process_hint)
                    .await;
            apply_unity_process_info(&mut status, process_info);
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
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: Some(latency_ms),
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status(),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
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
            ) {
                return status;
            }
            let mut status = UnityConnectionStatus {
                connected: false,
                editor_status: UNITY_EDITOR_STATUS_DISCONNECTED.to_string(),
                control_channel_state: "busy".to_string(),
                scene_path: None,
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: None,
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status(),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
            apply_observed_editor_status_fallback(project_path, &mut status);
            sync_background_hook_for_status(&mut status, project_path).await;
            cache_unity_connection_status(project_path, &status);
            status
        }
        Err(error) => {
            if let Some(status) = cached_running_connection_status_for_transient_failure(
                project_path,
                checked_at_ms,
                error.clone(),
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
                editor_process_state: UnityEditorProcessState::Unknown,
                editor_process_id: None,
                editor_process_path: None,
                editor_project_path: None,
                process_checked_at_ms: None,
                process_last_error: None,
                pipe_name,
                latency_ms: None,
                reconnect_attempts: 0,
                last_error: Some(error),
                background_hook: background_hook::status(),
                checked_at_ms,
            };
            let process_info =
                query_process_info_for_connection_status_bounded(project_path, false, None).await;
            apply_unity_process_info(&mut status, process_info);
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
        return Ok(background_hook::status());
    }
    if let Some(native) = native_owned_background_hook(project_path).await {
        return Ok(native);
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
    tauri::async_runtime::spawn_blocking(move || {
        background_hook::sync_for_process(process_id, &editor_process_path)
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
    query_unity_status_with_timeout(project_path, UNITY_STATUS_POLL_TIMEOUT).await
}

/// Like `query_unity_status` but with an explicit (short) timeout, so a wedged
/// editor or a half-open pipe cannot stall the caller for the default 35s. A
/// timeout reads as disconnected — the out-of-process native probe is then the
/// authority for what the editor is actually doing.
pub async fn query_unity_status_with_timeout(
    project_path: &str,
    timeout: Duration,
) -> (bool, &'static str, Option<String>) {
    match query_unity_status_response_with_timeout(project_path, timeout).await {
        Ok(Some((resp, _))) if resp.ok => {
            let msg = resp.message.unwrap_or_default();
            let (status, scene_part) = parse_unity_status_message(&msg);
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
    let resp = send_message(project_path, "exit_play_mode", "").await?;
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
) -> SidecarCompileAttempt {
    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_unavailable(reason),
    };

    let cache_key = run_states_compile_cache_key(project_path, &params, prepared_request);
    if cache_mode == RunStatesCompileCacheMode::Consume {
        if let Some(key) = cache_key.as_deref() {
            if let Some(cached) = take_cached_run_states_compile(key) {
                return SidecarCompileAttempt::Compiled {
                    payload: cached.payload,
                };
            }
        }
    }

    match crate::csharp_compile::compile_run_states(&params, prepared_request, false, false).await {
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
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    }

    eprintln!(
        "[Locus] unity_run_states sending {} ({} bytes)",
        msg_type,
        payload.len()
    );
    let mut resp = send_message_without_timeout(project_path, msg_type, &payload).await?;
    if msg_type == "run_states_loaded" && unity_plugin_lacks_message(&resp) {
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
                crate::csharp_compile::note_fallback(&reason);
            }
        }
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
    send_view_binding_message(project_path, "view_binding_read", request).await
}

pub async fn view_binding_discover(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_discover", request).await
}

pub async fn view_binding_write(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_write", request).await
}

pub async fn view_binding_apply(
    project_path: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    send_view_binding_message(project_path, "view_binding_apply", request).await
}

async fn send_view_binding_message(
    project_path: &str,
    message_type: &str,
    request: &serde_json::Value,
) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(request)
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
pub async fn end_edit_session(project_path: &str, owner: &str) -> Result<String, String> {
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
    }
}

async fn query_unity_execute_progress(
    project_path: &str,
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
        "",
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

async fn wait_for_unity_bridge_ready(
    project_path: &str,
    max_wait: Duration,
    context: &str,
) -> Result<(), String> {
    let start = std::time::Instant::now();

    loop {
        let mut detail = "Unity bridge status poll returned disconnected".to_string();
        let mut native_managed_not_ready = false;
        if native_bridge_enabled() {
            if let Some(status) = query_native_broker_status(project_path).await {
                if status.native_alive && status.managed_state == "ready" {
                    return Ok(());
                }
                native_managed_not_ready = true;
                detail = format!("Native broker managed state is '{}'", status.managed_state);
            }
        }

        if !native_managed_not_ready {
            let (connected, _, _) =
                query_unity_status_with_timeout(project_path, UNITY_STATUS_POLL_TIMEOUT).await;
            if connected {
                return Ok(());
            }
        }

        if start.elapsed() > max_wait {
            return Err(format!(
                "Timed out waiting for Unity bridge to become ready {} ({}s): {}",
                context,
                max_wait.as_secs(),
                detail
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

pub async fn cancel_unity_execute_code(project_path: &str) -> Result<String, String> {
    let resp = send_message_with_timeout(
        project_path,
        "cancel_execute_code",
        "",
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
) -> Option<String> {
    let request_bytes = serde_json::to_vec(prepared_request).ok()?;
    Some(format!(
        "{}\n{}\n{}\n{}",
        project_runtime_key(project_path),
        params.fingerprint,
        params.domain_generation,
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
) -> SidecarCompileAttempt {
    let params = match sidecar_compile_params(project_path).await {
        Ok(params) => params,
        Err(reason) => return sidecar_unavailable(reason),
    };

    let compile_started = std::time::Instant::now();
    match crate::csharp_compile::compile_snippet(&params, prepared_code, false, false).await {
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

pub async fn unity_execute_code_with_progress<F>(
    project_path: &str,
    code: &str,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    let mut rust_progress_revision = 1u64;
    on_progress(rust_unity_execute_progress(
        "Waiting for Locus Unity operation lock",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;

    on_progress(rust_unity_execute_progress(
        "Preparing Unity type index",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let prepared = prepare_unity_execute_code_for_send(project_path, code).await;

    let mut execute_msg_type = "execute_code";
    let mut execute_payload = prepared.code.clone();
    if crate::csharp_compile::is_enabled() {
        on_progress(rust_unity_execute_progress(
            "Compiling snippet in compile server",
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;
        match sidecar_compile_for_execute(project_path, &prepared.code).await {
            SidecarCompileAttempt::Compiled { payload } => {
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
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    }

    let mut send_attempt = 1u32;
    let resp = loop {
        on_progress(rust_unity_execute_progress(
            if send_attempt == 1 {
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
        let execute =
            send_message_without_timeout(project_path, execute_msg_type, &attempt_payload);
        tokio::pin!(execute);

        let mut progress_tick =
            tokio::time::interval(Duration::from_millis(UNITY_EXECUTE_PROGRESS_POLL_MS));
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_progress_revision = 0u64;
        let mut saw_unity_progress = false;
        let execute_started_at = std::time::Instant::now();
        let mut last_waiting_status_at = execute_started_at;
        let mut last_progress_poll_error: Option<String> = None;
        let mut progress_unavailable_since: Option<std::time::Instant> = None;

        let attempt_result: Result<PipeResponse, String> = loop {
            tokio::select! {
                result = &mut execute => break result,
                _ = progress_tick.tick() => {
                    match query_unity_execute_progress(project_path).await {
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
                            if snapshot.revision != last_progress_revision {
                                last_progress_revision = snapshot.revision;
                                on_progress(snapshot);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
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
                                    return Err(append_execute_reconnect_result(
                                        &reason,
                                        reconnect_unity_pipe_for_execute(project_path, &reason).await,
                                    ));
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
                            .unwrap_or("no active Unity execute progress yet");
                        eprintln!(
                            "[Locus] unity_execute still waiting for Unity progress after {}ms while sending {}: {}",
                            elapsed_ms, execute_msg_type, detail
                        );
                        on_progress(rust_unity_execute_progress(
                            format!("Waiting for Unity progress after sending {execute_msg_type}"),
                            format!("{}ms elapsed; {}", elapsed_ms, detail),
                            rust_progress_revision,
                        ));
                        rust_progress_revision += 1;
                        last_waiting_status_at = std::time::Instant::now();
                    }

                    if !saw_unity_progress
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
                }
            }
        };

        match attempt_result {
            // An older Unity plugin without the execute_loaded handler:
            // retry the same request through the legacy compile path.
            Ok(resp)
                if execute_msg_type == "execute_loaded" && unity_plugin_lacks_message(&resp) =>
            {
                crate::csharp_compile::note_fallback(
                    "Unity plugin lacks execute_loaded; update the Locus Unity plugin",
                );
                execute_msg_type = "execute_code";
                execute_payload = prepared.code.clone();
            }
            Ok(resp)
                if pipe_response_transient_broker_error(&resp)
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
                if let Err(reconnect_error) =
                    reconnect_unity_pipe_for_execute(project_path, &error).await
                {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Ok(resp) => break resp,
            Err(error) if !saw_unity_progress && send_attempt == 1 => {
                on_progress(rust_unity_execute_progress(
                    "Reconnecting Unity pipe",
                    &error,
                    rust_progress_revision,
                ));
                rust_progress_revision += 1;
                if let Err(reconnect_error) =
                    reconnect_unity_pipe_for_execute(project_path, &error).await
                {
                    return Err(format!(
                        "{}; Unity pipe reconnect failed: {}",
                        error, reconnect_error
                    ));
                }
                send_attempt += 1;
            }
            Err(error) => {
                return Err(append_execute_reconnect_result(
                    &error,
                    reconnect_unity_pipe_for_execute(project_path, &error).await,
                ));
            }
        }
    };

    if resp.ok {
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error.unwrap_or_else(|| "unknown error".to_string()),
            &prepared,
        ))
    }
}

pub async fn unity_execute_code_with_progress_cancellable<F>(
    project_path: &str,
    code: &str,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(UnityExecuteProgressSnapshot) + Send,
{
    if *cancel_rx.borrow() {
        return Err(UNITY_EXECUTE_CANCELLED.to_string());
    }

    let mut rust_progress_revision = 1u64;
    on_progress(rust_unity_execute_progress(
        "Waiting for Locus Unity operation lock",
        "",
        rust_progress_revision,
    ));
    rust_progress_revision += 1;

    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = tokio::select! {
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

    let mut execute_msg_type = "execute_code";
    let mut execute_payload = prepared.code.clone();
    if crate::csharp_compile::is_enabled() {
        on_progress(rust_unity_execute_progress(
            "Compiling snippet in compile server",
            "",
            rust_progress_revision,
        ));
        rust_progress_revision += 1;
        let attempt = tokio::select! {
            attempt = sidecar_compile_for_execute(project_path, &prepared.code) => attempt,
            _ = cancel_rx.changed() => return Err(UNITY_EXECUTE_CANCELLED.to_string()),
        };
        match attempt {
            SidecarCompileAttempt::Compiled { payload } => {
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
                crate::csharp_compile::note_fallback(&reason);
            }
        }
    }

    let mut send_attempt = 1u32;
    let resp = loop {
        on_progress(rust_unity_execute_progress(
            if send_attempt == 1 {
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
        let execute =
            send_message_without_timeout(project_path, execute_msg_type, &attempt_payload);
        tokio::pin!(execute);

        let mut progress_tick =
            tokio::time::interval(Duration::from_millis(UNITY_EXECUTE_PROGRESS_POLL_MS));
        progress_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_progress_revision = 0u64;
        let mut saw_unity_progress = false;
        let execute_started_at = std::time::Instant::now();
        let mut last_waiting_status_at = execute_started_at;
        let mut last_progress_poll_error: Option<String> = None;
        let mut progress_unavailable_since: Option<std::time::Instant> = None;

        let attempt_result: Result<PipeResponse, String> = loop {
            tokio::select! {
                result = &mut execute => break result,
                changed = cancel_rx.changed() => {
                    let cancelled = changed.is_err() || *cancel_rx.borrow();
                    if !cancelled {
                        continue;
                    }

                    if let Err(error) = cancel_unity_execute_code(project_path).await {
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
                                if let Ok(Some(snapshot)) = query_unity_execute_progress(project_path).await {
                                    if snapshot.revision != last_progress_revision {
                                        last_progress_revision = snapshot.revision;
                                        on_progress(snapshot);
                                    }
                                }
                            }
                        }
                    }

                    return Err(UNITY_EXECUTE_CANCELLED.to_string());
                },
                _ = progress_tick.tick() => {
                    match query_unity_execute_progress(project_path).await {
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
                            if snapshot.revision != last_progress_revision {
                                last_progress_revision = snapshot.revision;
                                on_progress(snapshot);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
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
                                    let reconnect = reconnect_unity_pipe_for_execute_cancellable(
                                        project_path,
                                        &reason,
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
                                    return Err(append_execute_reconnect_result(&reason, reconnect));
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
                            .unwrap_or("no active Unity execute progress yet");
                        eprintln!(
                            "[Locus] unity_execute still waiting for Unity progress after {}ms while sending {}: {}",
                            elapsed_ms, execute_msg_type, detail
                        );
                        on_progress(rust_unity_execute_progress(
                            format!("Waiting for Unity progress after sending {execute_msg_type}"),
                            format!("{}ms elapsed; {}", elapsed_ms, detail),
                            rust_progress_revision,
                        ));
                        rust_progress_revision += 1;
                        last_waiting_status_at = std::time::Instant::now();
                    }

                    if !saw_unity_progress
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
                }
            }
        };

        match attempt_result {
            // An older Unity plugin without the execute_loaded handler:
            // retry the same request through the legacy compile path.
            Ok(resp)
                if execute_msg_type == "execute_loaded" && unity_plugin_lacks_message(&resp) =>
            {
                crate::csharp_compile::note_fallback(
                    "Unity plugin lacks execute_loaded; update the Locus Unity plugin",
                );
                execute_msg_type = "execute_code";
                execute_payload = prepared.code.clone();
            }
            Ok(resp)
                if pipe_response_transient_broker_error(&resp)
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
            Err(error) if !saw_unity_progress && send_attempt == 1 => {
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
        Ok(resp.message.unwrap_or_default())
    } else {
        Err(crate::unity_type_index::append_auto_using_notes(
            resp.error.unwrap_or_else(|| "unknown error".to_string()),
            &prepared,
        ))
    }
}

pub async fn unity_execute_code(project_path: &str, code: &str) -> Result<String, String> {
    unity_execute_code_with_progress(project_path, code, |_| {}).await
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

/// Trigger a Unity recompile and wait until the new domain is ready.
///
/// Flow:
/// 1. Release every edit session so Unity can see the full batch of file writes.
/// 2. Send `request_recompile`.
/// 3. Poll `get_compile_result`.
///    - `pending`: compilation or reload is still in progress.
///    - `ok`: compilation succeeded and the reloaded AppDomain reported completion.
///    - `error:*`: compilation failed; surface the compiler errors immediately.
/// 4. If the pipe drops during reload, wait for Unity to reconnect as a fallback signal.
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

pub async fn recompile_and_wait(project_path: &str) -> Result<String, String> {
    let op_lock = project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let _recompile_wait_guard = UnityRecompileWaitGuard::new(project_path);
    let hook_effective = background_hook_effective_for_project(project_path).await;
    let prev_foreground = if hook_effective {
        None
    } else {
        focus::bring_unity_to_foreground()
    };

    let finish = |result: Result<String, String>| -> Result<String, String> {
        if let Some(hwnd) = prev_foreground {
            focus::restore_foreground(hwnd);
        }
        result
    };

    if let Err(e) = end_edit_session(project_path, "").await {
        eprintln!(
            "[Locus] failed to end edit sessions before recompile (continuing): {}",
            e
        );
    }

    // Hot-reload edits bypass the AssetDatabase entirely; forward every
    // tracked dirty path so the plugin imports created files (and refreshes
    // away deleted ones) before compiling. Without this, files created or
    // deleted during a hot-reload session would be missing from (or stale
    // in) the converged assembly. Older plugins ignore the message body.
    let tracked_dirty_paths = relative_asset_paths(
        project_path,
        &crate::unity_hotreload::coordinator::pending_paths(project_path).await,
    )
    .join("\n");

    let resp = match send_message(project_path, "request_recompile", &tracked_dirty_paths).await {
        Ok(resp) => resp,
        Err(error) => return finish(Err(error)),
    };
    let mut request_recompile_reloading = false;
    if !resp.ok {
        let error = resp
            .error
            .unwrap_or_else(|| "request_recompile failed".to_string());
        if is_reload_boundary_broker_error(&error) {
            request_recompile_reloading = true;
            transport::disconnect(project_path).await;
            eprintln!(
                "[Locus] request_recompile hit native reload boundary, waiting for reconnect: {}",
                error
            );
        } else {
            return finish(Err(error));
        }
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    let max_wait = Duration::from_secs(120);
    let start = std::time::Instant::now();
    let mut disconnected = request_recompile_reloading;

    loop {
        if start.elapsed() > max_wait {
            return finish(Err("Compilation timed out (120s)".to_string()));
        }

        if disconnected {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match send_message(project_path, "ping", "").await {
                Ok(resp) if resp.ok => {
                    eprintln!("[Locus] Unity reconnected after domain reload");
                    crate::unity_type_index::invalidate_cached_type_index(project_path).await;
                    crate::unity_hotreload::coordinator::on_recompile_converged(project_path).await;
                    if let Err(error) =
                        wait_for_unity_bridge_ready_after_recompile(project_path).await
                    {
                        return finish(Err(error));
                    }
                    if let Err(error) = refresh_unity_type_index_after_recompile(project_path).await
                    {
                        eprintln!(
                            "[Locus] Unity type index refresh after recompile skipped: {}",
                            error
                        );
                    }
                    return finish(Ok(
                        "Compilation succeeded, domain reload complete".to_string()
                    ));
                }
                _ => continue,
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
        match send_message(project_path, "get_compile_result", "").await {
            Ok(resp) => {
                if resp.ok {
                    let msg = resp.message.unwrap_or_default();
                    match msg.as_str() {
                        "pending" => continue,
                        "ok" => {
                            crate::unity_type_index::invalidate_cached_type_index(project_path)
                                .await;
                            crate::unity_hotreload::coordinator::on_recompile_converged(
                                project_path,
                            )
                            .await;
                            if let Err(error) =
                                wait_for_unity_bridge_ready_after_recompile(project_path).await
                            {
                                return finish(Err(error));
                            }
                            if let Err(error) =
                                refresh_unity_type_index_after_recompile(project_path).await
                            {
                                eprintln!(
                                    "[Locus] Unity type index refresh after recompile skipped: {}",
                                    error
                                );
                            }
                            return finish(Ok(
                                "Compilation succeeded, domain reload complete".to_string()
                            ));
                        }
                        other => {
                            eprintln!("[Locus] unexpected compile result: {}", other);
                            continue;
                        }
                    }
                } else {
                    let error = resp
                        .error
                        .unwrap_or_else(|| "Compilation failed (unknown error)".to_string());
                    // Native broker is up but the managed executor is mid
                    // domain reload: keep polling — the result resolves once the
                    // new domain re-registers.
                    if is_transient_broker_error(&error) {
                        continue;
                    }
                    return finish(Err(error));
                }
            }
            Err(_) => {
                disconnected = true;
                transport::disconnect(project_path).await;
                eprintln!("[Locus] Unity disconnected during recompile, waiting for reconnect...");
            }
        }
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
) {
    stop_unity_monitor(monitor).await;
    set_event_app_handle(app_handle.clone());

    let pipe_name = get_native_pipe_name(&project_path);
    eprintln!(
        "[Locus] Unity project detected, starting connection monitor (pipe: {})",
        pipe_name
    );
    state_probe::start_observer(&project_path);
    // The status badge's unapplied count reflects the workspace this monitor
    // watches (not stale pending from a prior project / another editor).
    crate::unity_hotreload::coordinator::set_active_project(&project_path);

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

        loop {
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
                        crate::unity_hotreload::on_play_mode_exited(&play_exit_project).await;
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

            if recompile_waiting && !connected {
                if let Some(process_info) = last_detected_editor_process
                    .clone()
                    .filter(|info| info.process_id.is_some())
                {
                    apply_unity_process_info(&mut status, process_info);
                    sync_background_hook_for_status(&mut status, &project_path).await;
                }
            } else if disconnected_transition {
                if let Some(process_info) = process::refresh_known_project_editor_process_liveness(
                    &project_path,
                    last_detected_editor_process.clone(),
                )
                .await
                {
                    let process_not_running =
                        matches!(process_info.state, UnityEditorProcessState::NotRunning);
                    apply_unity_process_info(&mut status, process_info);
                    if process_not_running {
                        sync_background_hook_for_status(&mut status, &project_path).await;
                        // The editor is gone: reset its dead detour state but KEEP
                        // the tracked edits — they are still not in any running
                        // editor. A relaunch's startup recompile loads them, and
                        // the next reload-state sample converges them then (or
                        // keeps them if that compile fails).
                        crate::unity_hotreload::coordinator::on_editor_exited(&project_path).await;
                    }
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

            let _ = app_handle.emit("unity-connection-status-detail", status.clone());

            if last_status != Some(connected) {
                last_status = Some(connected);
                let _ = app_handle.emit("unity-connection-status", connected);
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    monitor.lock().await.replace(handle);
}

pub async fn stop_unity_monitor(monitor: &UnityMonitorHandle) {
    if let Some(handle) = monitor.lock().await.take() {
        handle.abort();
        eprintln!("[Locus] Unity connection monitor stopped");
    }
    state_probe::stop_all_observers();
}

#[cfg(test)]
mod tests {
    use super::{
        cache_unity_connection_status, cached_running_connection_status_for_transient_failure,
        is_transient_broker_error, pipe_response_transient_broker_error,
        read_project_unity_version, relative_asset_paths, requested_run_states_editor_status,
        rewrite_run_states_output_for_size, PipeResponse, UnityBackgroundHookState,
        UnityBackgroundHookStatus, UnityConnectionStatus, UnityEditorProcessState,
    };
    use serde_json::json;

    fn result_file(summary: &str) -> String {
        summary
            .lines()
            .find_map(|line| line.strip_prefix("result_file: "))
            .expect("result_file field")
            .to_string()
    }

    fn test_connection_status(project_path: &str, checked_at_ms: u64) -> UnityConnectionStatus {
        UnityConnectionStatus {
            connected: true,
            editor_status: super::UNITY_EDITOR_STATUS_PLAYING.to_string(),
            control_channel_state: "ready".to_string(),
            scene_path: Some("Assets/Scenes/Main.unity".to_string()),
            editor_process_state: UnityEditorProcessState::Running,
            editor_process_id: Some(42),
            editor_process_path: Some("C:/Unity/Unity.exe".to_string()),
            editor_project_path: Some(project_path.to_string()),
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
    fn transient_status_failure_reuses_recent_running_status() {
        let project_path = format!("F:/Proj/Game/cache-test-{}", std::process::id());
        let status = test_connection_status(&project_path, 1_000);
        cache_unity_connection_status(&project_path, &status);

        let cached = cached_running_connection_status_for_transient_failure(
            &project_path,
            1_500,
            "writer busy",
        )
        .expect("recent running status should be reused");

        assert!(!cached.connected);
        assert_eq!(cached.control_channel_state, "busy");
        assert_eq!(cached.editor_status, super::UNITY_EDITOR_STATUS_PLAYING);
        assert_eq!(cached.checked_at_ms, 1_500);
        assert_eq!(cached.latency_ms, None);
        assert_eq!(cached.last_error.as_deref(), Some("writer busy"));
        assert!(cached_running_connection_status_for_transient_failure(
            &project_path,
            20_000,
            "stale",
        )
        .is_none());
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
}
