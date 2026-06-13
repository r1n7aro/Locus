//! `locus_native` — the in-process Unity Editor broker.
//!
//! Locus talks to the Unity Editor over a Windows named pipe. Today that pipe
//! is a *managed* object living in Unity's Mono domain: it dies on every domain
//! reload and goes silent whenever the editor's main thread wedges — the exact
//! moments a caller most wants the connection to stay up. This crate moves the
//! long-lived comms substrate into a native DLL that is loaded into the Unity
//! process and **persists across domain reloads** (native modules are not
//! unloaded when an AppDomain is torn down). The managed layer keeps doing all
//! the Unity-API work; it just polls this broker for requests instead of owning
//! the pipe.
//!
//! Responsibilities (the "minimum deliverable" slice of the migration plan):
//!   * Run the command pipe **server** at `\\.\pipe\locus_unity_native_{hash}`.
//!   * Answer `ping`, `status`, `bridge_capabilities` directly
//!     — these stay available during a domain reload.
//!   * Queue every other request for the managed executor; hand them out via
//!     `locus_poll_request` and write the managed response via
//!     `locus_complete_request`.
//!   * Track managed lifecycle (state + generation + heartbeat) so the broker
//!     can fail fast with `managed_reloading` / `managed_not_ready` and so
//!     in-flight requests interrupted by a reload get a definite error instead
//!     of hanging.
//!
//! The wire protocol is the *same* newline-delimited JSON envelope the managed
//! pipe already speaks (`{id,type,message,ok,error,reply_to,processId,
//! processPath}`), so the Tauri-side transport reader needs no protocol change
//! to talk to this broker.
//!
//! Everything real is Windows-only; other targets compile inert stubs so the
//! crate still builds in CI.

use std::os::raw::c_int;

// ── Managed lifecycle states (must match the C# `ManagedState` enum) ─────────

/// Native is up; the managed executor has not registered yet (first load).
pub const MANAGED_STATE_INITIALIZING: i32 = 0;
/// Managed executor is registered and pumping requests.
pub const MANAGED_STATE_READY: i32 = 1;
/// A domain reload is in progress; the managed executor is gone for now.
pub const MANAGED_STATE_RELOADING: i32 = 2;
/// Unity is quitting.
pub const MANAGED_STATE_QUITTING: i32 = 3;

/// Protocol the managed side and the broker agree on.
pub const NATIVE_PROTOCOL_VERSION: i32 = 1;

// ── FFI helpers ──────────────────────────────────────────────────────────────

/// # Safety
/// `ptr` must be valid for `len` bytes or null. Returns an empty slice for
/// null / non-positive length.
unsafe fn slice_from_raw<'a>(ptr: *const u8, len: i32) -> &'a [u8] {
    if ptr.is_null() || len <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len as usize)
    }
}

/// # Safety
/// See [`slice_from_raw`]. Decodes loss-tolerantly; the broker never trusts the
/// bytes to be valid UTF-8.
unsafe fn string_from_raw(ptr: *const u8, len: i32) -> String {
    String::from_utf8_lossy(slice_from_raw(ptr, len)).into_owned()
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows implementation
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use std::collections::{HashMap, VecDeque};
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Serialize;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
    use tokio::sync::{mpsc, Notify};

    use super::{
        MANAGED_STATE_INITIALIZING, MANAGED_STATE_QUITTING, MANAGED_STATE_READY,
        MANAGED_STATE_RELOADING,
    };

    const NATIVE_CAPABILITIES: &str = "broker_v1,broker_state_mmf_v1,broker_queue_limits_v1";
    const STATUS_EVENT_BUFFER_LIMIT: usize = 256;
    const MAX_PENDING_REQUESTS: usize = 128;
    const MAX_INFLIGHT_REQUESTS: usize = 64;
    const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
    const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
    const REQUEST_DEADLINE_MS: i64 = 10 * 60 * 1000;
    const WRITER_CHANNEL_LIMIT: usize = 512;
    const NATIVE_STATE_MMF_MAGIC: u32 = 0x424e_434c; // "LCNB" little-endian.
    const NATIVE_STATE_MMF_VERSION: u16 = 1;
    const NATIVE_STATE_MMF_HEADER_SIZE: usize = 64;
    const NATIVE_STATE_MMF_SLOT_COUNT: usize = 8;
    const NATIVE_STATE_MMF_SLOT_SIZE: usize = 128 * 1024;
    const NATIVE_STATE_MMF_SLOT_PAYLOAD_OFFSET: usize = 24;
    const NATIVE_STATE_MMF_MAX_PAYLOAD: usize =
        NATIVE_STATE_MMF_SLOT_SIZE - NATIVE_STATE_MMF_SLOT_PAYLOAD_OFFSET;
    const NATIVE_STATE_SOURCE_BROKER: u32 = 0x1;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;

    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const PAGE_READWRITE: Dword = 0x04;
    const FILE_MAP_WRITE: Dword = 0x0002;

    unsafe extern "system" {
        fn CreateFileMappingW(
            hFile: Handle,
            lpFileMappingAttributes: *mut c_void,
            flProtect: Dword,
            dwMaximumSizeHigh: Dword,
            dwMaximumSizeLow: Dword,
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

    struct NativeStatePlane {
        handle: Handle,
        view: *mut u8,
        total_size: usize,
        seq: u64,
    }

    unsafe impl Send for NativeStatePlane {}

    impl NativeStatePlane {
        fn create(project_path: &str) -> Option<Self> {
            if project_path.trim().is_empty() {
                return None;
            }
            let total_size = NATIVE_STATE_MMF_HEADER_SIZE.saturating_add(
                NATIVE_STATE_MMF_SLOT_COUNT.saturating_mul(NATIVE_STATE_MMF_SLOT_SIZE),
            );
            let name = native_state_mmf_name(project_path);
            let wide_name = wide_null(&name);
            let handle = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    null_mut(),
                    PAGE_READWRITE,
                    0,
                    total_size.min(u32::MAX as usize) as Dword,
                    wide_name.as_ptr(),
                )
            };
            if handle.is_null() {
                eprintln!("[locus-native] failed to create state plane: {name}");
                return None;
            }
            let view =
                unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, total_size) as *mut u8 };
            if view.is_null() {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                eprintln!("[locus-native] failed to map state plane: {name}");
                return None;
            }

            let mut plane = Self {
                handle,
                view,
                total_size,
                seq: 0,
            };
            plane.write_header();
            Some(plane)
        }

        fn write_header(&mut self) {
            self.write_u32(0, NATIVE_STATE_MMF_MAGIC);
            self.write_u16(4, NATIVE_STATE_MMF_VERSION);
            self.write_u16(6, NATIVE_STATE_MMF_SLOT_COUNT.min(u16::MAX as usize) as u16);
            self.write_u32(8, NATIVE_STATE_MMF_SLOT_SIZE.min(u32::MAX as usize) as u32);
            self.write_u32(12, std::process::id());
            self.write_u64(16, 0);
            self.write_u64(24, now_ms().max(0) as u64);
        }

        fn write_json(&mut self, observed_at_ms: i64, json: &str) {
            let bytes = json.as_bytes();
            let max_payload = NATIVE_STATE_MMF_MAX_PAYLOAD;
            if bytes.is_empty() || bytes.len() > max_payload {
                eprintln!(
                    "[locus-native] state plane payload too large: {} bytes (max {})",
                    bytes.len(),
                    max_payload
                );
                return;
            }

            self.seq = self.seq.saturating_add(1).max(1);
            let slot = ((self.seq - 1) as usize) % NATIVE_STATE_MMF_SLOT_COUNT;
            let slot_offset = NATIVE_STATE_MMF_HEADER_SIZE + slot * NATIVE_STATE_MMF_SLOT_SIZE;
            let observed_at_ms = observed_at_ms.max(0) as u64;

            self.write_u64(slot_offset, 0);
            self.write_u64(slot_offset + 8, observed_at_ms);
            self.write_u32(slot_offset + 16, NATIVE_STATE_SOURCE_BROKER);
            self.write_u32(slot_offset + 20, bytes.len().min(u32::MAX as usize) as u32);
            self.write_bytes(slot_offset + NATIVE_STATE_MMF_SLOT_PAYLOAD_OFFSET, bytes);
            self.write_u64(slot_offset, self.seq);
            self.write_u64(16, self.seq);
            self.write_u64(24, observed_at_ms);
        }

        fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
            if offset.saturating_add(bytes.len()) > self.total_size {
                return;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.view.add(offset), bytes.len());
            }
        }

        fn write_u16(&mut self, offset: usize, value: u16) {
            self.write_bytes(offset, &value.to_le_bytes());
        }

        fn write_u32(&mut self, offset: usize, value: u32) {
            self.write_bytes(offset, &value.to_le_bytes());
        }

        fn write_u64(&mut self, offset: usize, value: u64) {
            self.write_bytes(offset, &value.to_le_bytes());
        }
    }

    impl Drop for NativeStatePlane {
        fn drop(&mut self) {
            unsafe {
                let _ = UnmapViewOfFile(self.view as *const c_void);
                let _ = CloseHandle(self.handle);
            }
        }
    }

    fn native_state_mmf_name(project_path: &str) -> String {
        format!(
            r"Local\LocusNativeBrokerState_{}",
            project_state_plane_key(project_path)
        )
    }

    fn project_state_plane_key(project_path: &str) -> String {
        let normalized = normalize_project_path_for_state_plane(project_path);
        let digest = Sha256::digest(normalized.as_bytes());
        let mut out = String::with_capacity(32);
        for byte in digest.iter().take(16) {
            use std::fmt::Write;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }

    fn normalize_project_path_for_state_plane(project_path: &str) -> String {
        let mut value = project_path.trim().replace('/', "\\");
        while value.ends_with('\\') && value.len() > 3 {
            value.pop();
        }
        value.to_ascii_lowercase()
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    /// A request received from the Tauri client that must run on the managed
    /// executor. `raw` is the original envelope line (no trailing newline) the
    /// managed side parses back with `JsonUtility`.
    struct QueuedRequest {
        id: String,
        raw: Vec<u8>,
        deadline_ms: i64,
    }

    struct InflightRequest {
        deadline_ms: i64,
    }

    /// Process-global broker state. Lives for the lifetime of the Unity process
    /// (a domain reload does not unload this DLL), so the pipe + queue survive
    /// reloads while the managed executor comes and goes.
    pub struct Broker {
        project: String,
        pipe_name: String,
        protocol_version: i32,

        shutdown: AtomicBool,
        shutdown_notify: Notify,

        /// A Tauri client is connected to the pipe right now.
        connected: AtomicBool,
        /// One of the `MANAGED_STATE_*` constants.
        managed_state: AtomicI32,
        /// Bumped by the managed side on every domain reload.
        generation: AtomicI64,
        /// Unix-ms of the last managed heartbeat / state push.
        last_heartbeat_ms: AtomicI64,

        /// Last editor status string the managed side published
        /// (`editing|playing|playing_paused` + optional `|scenePath`). Answers
        /// `status` directly so the bare status poll never stalls on reload.
        editor_status: Mutex<String>,
        /// Capability string the managed executor registered (merged with the
        /// native caps when answering `bridge_capabilities`).
        managed_capabilities: Mutex<String>,

        /// Requests waiting to be handed to the managed executor.
        queue: Mutex<VecDeque<QueuedRequest>>,
        /// Total bytes currently retained by `queue`.
        queued_bytes: Mutex<usize>,
        /// Requests handed out via `poll` but not yet completed — used to
        /// synthesize `domain_reload_interrupted` if a reload cuts them off.
        inflight: Mutex<HashMap<String, InflightRequest>>,

        /// Sender into the current connection's writer task. `None` when no
        /// client is connected.
        response_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,

        /// Managed lifecycle edge events retained for cursor-based consumers.
        event_seq: AtomicU64,
        events: Mutex<VecDeque<NativeStatusEvent>>,

        /// Native-owned shared-memory status surface. Managed code publishes
        /// inputs through FFI; this broker owns the cross-process memory plane.
        state_plane: Mutex<Option<NativeStatePlane>>,

        process_id: u32,
        process_path: String,
    }

    static BROKER: OnceLock<Arc<Broker>> = OnceLock::new();

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0)
    }

    /// The envelope written back to the Tauri client. Mirrors the managed
    /// `PipeEnvelope` field names exactly so the transport reader is unchanged.
    #[derive(Serialize)]
    struct OutEnvelope<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to: Option<&'a str>,
        #[serde(rename = "type")]
        kind: &'a str,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a str>,
        #[serde(rename = "processId", skip_serializing_if = "Option::is_none")]
        process_id: Option<u32>,
        #[serde(rename = "processPath", skip_serializing_if = "Option::is_none")]
        process_path: Option<String>,
    }

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct NativeStatusEvent {
        seq: u64,
        kind: &'static str,
        from: String,
        to: String,
        domain_generation: i64,
        editor_status: String,
        observed_at_ms: i64,
    }

    impl Broker {
        fn new(project: String, pipe_name: String, protocol_version: i32) -> Self {
            let state_plane = NativeStatePlane::create(&project);
            Self {
                project,
                pipe_name,
                protocol_version,
                shutdown: AtomicBool::new(false),
                shutdown_notify: Notify::new(),
                connected: AtomicBool::new(false),
                managed_state: AtomicI32::new(MANAGED_STATE_INITIALIZING),
                generation: AtomicI64::new(0),
                last_heartbeat_ms: AtomicI64::new(0),
                editor_status: Mutex::new(String::new()),
                managed_capabilities: Mutex::new(String::new()),
                queue: Mutex::new(VecDeque::new()),
                queued_bytes: Mutex::new(0),
                inflight: Mutex::new(HashMap::new()),
                response_tx: Mutex::new(None),
                event_seq: AtomicU64::new(0),
                events: Mutex::new(VecDeque::new()),
                state_plane: Mutex::new(state_plane),
                process_id: std::process::id(),
                process_path: std::env::current_exe()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            }
        }

        fn managed_state(&self) -> i32 {
            self.managed_state.load(Ordering::SeqCst)
        }

        fn managed_state_name(&self) -> &'static str {
            managed_state_name(self.managed_state())
        }

        // ── Outbound writes ────────────────────────────────────────────────

        /// Push a complete envelope line (newline appended here) to the client.
        /// No-ops when no client is connected — the client is gone, the answer
        /// has nowhere to go.
        fn push_line(&self, mut bytes: Vec<u8>) {
            bytes.push(b'\n');
            if let Ok(guard) = self.response_tx.lock() {
                if let Some(tx) = guard.as_ref() {
                    match tx.try_send(bytes) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            eprintln!(
                                "[locus-native] response writer queue full (limit {})",
                                WRITER_CHANNEL_LIMIT
                            );
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                    }
                }
            }
        }

        fn send_envelope(&self, env: &OutEnvelope) {
            match serde_json::to_vec(env) {
                Ok(bytes) => self.push_line(bytes),
                Err(e) => eprintln!("[locus-native] envelope serialize failed: {e}"),
            }
        }

        fn respond_ok(&self, id: &str, message: Option<String>) {
            self.send_envelope(&OutEnvelope {
                reply_to: Some(id),
                kind: "response",
                ok: true,
                message,
                error: None,
                process_id: None,
                process_path: None,
            });
        }

        fn respond_error(&self, id: &str, code: &str) {
            self.send_envelope(&OutEnvelope {
                reply_to: Some(id),
                kind: "response",
                ok: false,
                message: None,
                error: Some(code),
                process_id: None,
                process_path: None,
            });
        }

        fn respond_status(&self, id: &str) {
            match self.managed_state() {
                MANAGED_STATE_READY => {}
                MANAGED_STATE_RELOADING => {
                    self.respond_error(id, "managed_reloading");
                    return;
                }
                MANAGED_STATE_QUITTING => {
                    self.respond_error(id, "unity_process_exiting");
                    return;
                }
                _ => {
                    self.respond_error(id, "managed_not_ready");
                    return;
                }
            }

            let message = self
                .editor_status
                .lock()
                .ok()
                .map(|s| s.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "editing".to_string());
            self.send_envelope(&OutEnvelope {
                reply_to: Some(id),
                kind: "response",
                ok: true,
                message: Some(message),
                error: None,
                process_id: Some(self.process_id),
                process_path: if self.process_path.is_empty() {
                    None
                } else {
                    Some(self.process_path.clone())
                },
            });
        }

        fn capabilities_string(&self) -> String {
            let managed = self
                .managed_capabilities
                .lock()
                .ok()
                .map(|s| s.clone())
                .unwrap_or_default();
            if managed.is_empty() {
                NATIVE_CAPABILITIES.to_string()
            } else {
                format!("{NATIVE_CAPABILITIES},{managed}")
            }
        }

        fn status_json_value(
            &self,
            observed_at_ms: i64,
            events: Vec<NativeStatusEvent>,
            cursor: u64,
        ) -> Value {
            let active_request_id = self
                .inflight
                .lock()
                .ok()
                .and_then(|m| m.keys().next().cloned());
            let (background_patched, background_symbols) = crate::hook::snapshot();
            let managed_capabilities = self
                .managed_capabilities
                .lock()
                .ok()
                .map(|s| s.clone())
                .unwrap_or_default();
            serde_json::json!({
                "transport": "native_broker",
                "nativeAlive": true,
                "observedAtMs": observed_at_ms,
                "managedState": self.managed_state_name(),
                "domainGeneration": self.generation.load(Ordering::SeqCst),
                "editorStatus": self.editor_status.lock().map(|s| s.clone()).unwrap_or_default(),
                "lastManagedHeartbeatMs": self.last_heartbeat_ms.load(Ordering::SeqCst),
                "pendingRequests": self.queue.lock().map(|q| q.len()).unwrap_or(0),
                "pendingBytes": self.queued_bytes.lock().map(|bytes| *bytes).unwrap_or(0),
                "inflightRequests": self.inflight.lock().map(|m| m.len()).unwrap_or(0),
                "activeRequestId": active_request_id,
                "capabilities": self.capabilities_string().split(',').collect::<Vec<_>>(),
                "brokerCapabilities": NATIVE_CAPABILITIES.split(',').collect::<Vec<_>>(),
                "managedCapabilities": managed_capabilities
                    .split(',')
                    .filter(|capability| !capability.is_empty())
                    .collect::<Vec<_>>(),
                "protocolVersion": self.protocol_version,
                "pipeName": self.pipe_name,
                "project": self.project,
                "processId": self.process_id,
                "processPath": self.process_path,
                "queueLimit": MAX_PENDING_REQUESTS,
                "inflightLimit": MAX_INFLIGHT_REQUESTS,
                "payloadLimitBytes": MAX_REQUEST_BYTES,
                "pendingByteLimit": MAX_PENDING_BYTES,
                "writerQueueLimit": WRITER_CHANNEL_LIMIT,
                "requestDeadlineMs": REQUEST_DEADLINE_MS,
                "backgroundPatched": background_patched,
                "backgroundSymbols": background_symbols,
                "overlayConnected": crate::overlay::connected(),
                "events": events,
                "cursor": cursor,
            })
        }

        fn shared_memory_status_json(&self) -> (i64, String) {
            self.expire_requests();
            let observed_at_ms = now_ms();
            let cursor = self.event_seq.load(Ordering::SeqCst);
            let mut events = self
                .events
                .lock()
                .ok()
                .map(|events| events.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();

            loop {
                let value = self.status_json_value(observed_at_ms, events.clone(), cursor);
                let json = value.to_string();
                if json.len() <= NATIVE_STATE_MMF_MAX_PAYLOAD || events.is_empty() {
                    return (observed_at_ms, json);
                }

                let drop_count = ((events.len() + 1) / 2).max(1);
                events.drain(0..drop_count.min(events.len()));
            }
        }

        fn publish_status_snapshot(&self) {
            let (observed_at_ms, json) = self.shared_memory_status_json();
            if let Ok(mut guard) = self.state_plane.lock() {
                if let Some(plane) = guard.as_mut() {
                    plane.write_json(observed_at_ms, &json);
                }
            }
        }

        /// Emit an unsolicited event (no `reply_to`) to the client, e.g. the
        /// editor-update push. No-ops when nothing is connected.
        fn emit_event(&self, event_type: &str, payload: String) {
            self.send_envelope(&OutEnvelope {
                reply_to: None,
                kind: event_type,
                ok: true,
                message: Some(payload),
                error: None,
                process_id: None,
                process_path: None,
            });
        }

        // ── Request queue ──────────────────────────────────────────────────

        fn expire_requests(&self) {
            let now = now_ms();
            let mut expired: Vec<String> = Vec::new();

            if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock())
            {
                let mut retained = VecDeque::with_capacity(q.len());
                while let Some(req) = q.pop_front() {
                    if req.deadline_ms <= now {
                        *queued_bytes = queued_bytes.saturating_sub(req.raw.len());
                        expired.push(req.id);
                    } else {
                        retained.push_back(req);
                    }
                }
                *q = retained;
            }

            if let Ok(mut inflight) = self.inflight.lock() {
                let ids = inflight
                    .iter()
                    .filter_map(|(id, req)| {
                        if req.deadline_ms <= now {
                            Some(id.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                for id in ids {
                    if inflight.remove(&id).is_some() {
                        expired.push(id);
                    }
                }
            }

            for id in expired {
                self.respond_error(&id, "native_request_timed_out");
            }
        }

        fn enqueue(&self, id: String, raw: Vec<u8>) -> Result<(), &'static str> {
            self.expire_requests();

            if raw.len() > MAX_REQUEST_BYTES {
                return Err("native_payload_too_large");
            }
            if self
                .inflight
                .lock()
                .map(|inflight| inflight.len() >= MAX_INFLIGHT_REQUESTS)
                .unwrap_or(true)
            {
                return Err("native_inflight_full");
            }

            let deadline_ms = now_ms().saturating_add(REQUEST_DEADLINE_MS);
            {
                let mut q = self.queue.lock().map_err(|_| "native_queue_full")?;
                let mut queued_bytes = self.queued_bytes.lock().map_err(|_| "native_queue_full")?;
                if q.len() >= MAX_PENDING_REQUESTS
                    || queued_bytes.saturating_add(raw.len()) > MAX_PENDING_BYTES
                {
                    return Err("native_queue_full");
                }
                *queued_bytes = queued_bytes.saturating_add(raw.len());
                q.push_back(QueuedRequest {
                    id,
                    raw,
                    deadline_ms,
                });
            }
            self.publish_status_snapshot();
            Ok(())
        }

        fn next_request_len(&self) -> Option<usize> {
            self.expire_requests();
            if self
                .inflight
                .lock()
                .ok()
                .map(|inflight| inflight.len() >= MAX_INFLIGHT_REQUESTS)
                .unwrap_or(true)
            {
                return None;
            }
            self.queue.lock().ok()?.front().map(|r| r.raw.len())
        }

        fn take_next_request(&self) -> Option<QueuedRequest> {
            self.expire_requests();
            if self
                .inflight
                .lock()
                .ok()
                .map(|inflight| inflight.len() >= MAX_INFLIGHT_REQUESTS)
                .unwrap_or(true)
            {
                return None;
            }
            let req = self.queue.lock().ok()?.pop_front()?;
            if let Ok(mut queued_bytes) = self.queued_bytes.lock() {
                *queued_bytes = queued_bytes.saturating_sub(req.raw.len());
            }
            if let Ok(mut inflight) = self.inflight.lock() {
                inflight.insert(
                    req.id.clone(),
                    InflightRequest {
                        deadline_ms: now_ms().saturating_add(REQUEST_DEADLINE_MS),
                    },
                );
            }
            self.publish_status_snapshot();
            Some(req)
        }

        fn requeue_front(&self, req: QueuedRequest) {
            if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock())
            {
                *queued_bytes = queued_bytes.saturating_add(req.raw.len());
                q.push_front(req);
            }
            self.publish_status_snapshot();
        }

        fn complete(&self, id: &str, response: Vec<u8>) {
            let removed = self
                .inflight
                .lock()
                .map(|mut inflight| inflight.remove(id).is_some())
                .unwrap_or(false);
            if removed {
                self.push_line(response);
                self.publish_status_snapshot();
            } else {
                eprintln!(
                    "[locus-native] dropping stale completion for non-inflight request: {id}"
                );
            }
        }

        /// A reload is starting: every request the managed side will never get
        /// to (queued or already handed out) gets a definite error so the
        /// client retries instead of hanging.
        fn interrupt_for_reload(&self) {
            let mut ids: Vec<String> = Vec::new();
            if let Ok(mut q) = self.queue.lock() {
                while let Some(req) = q.pop_front() {
                    ids.push(req.id);
                }
            }
            if let Ok(mut queued_bytes) = self.queued_bytes.lock() {
                *queued_bytes = 0;
            }
            if let Ok(mut inflight) = self.inflight.lock() {
                for id in inflight.drain() {
                    ids.push(id.0);
                }
            }
            for id in ids {
                self.respond_error(&id, "domain_reload_interrupted");
            }
            self.publish_status_snapshot();
        }

        /// The client disconnected: its answers have nowhere to go, so drop
        /// queued + in-flight work silently.
        fn clear_requests_on_disconnect(&self) {
            if let Ok(mut q) = self.queue.lock() {
                q.clear();
            }
            if let Ok(mut queued_bytes) = self.queued_bytes.lock() {
                *queued_bytes = 0;
            }
            if let Ok(mut inflight) = self.inflight.lock() {
                inflight.clear();
            }
            self.publish_status_snapshot();
        }

        // ── Managed lifecycle ──────────────────────────────────────────────

        fn set_managed_state(&self, state: i32, generation: i64, editor_status: Option<String>) {
            let previous = self.managed_state.swap(state, Ordering::SeqCst);
            let observed_at_ms = now_ms();
            if generation > 0 {
                self.generation.store(generation, Ordering::SeqCst);
            }
            self.last_heartbeat_ms
                .store(observed_at_ms, Ordering::SeqCst);
            if let Some(status) = editor_status {
                if !status.is_empty() {
                    if let Ok(mut guard) = self.editor_status.lock() {
                        *guard = status;
                    }
                }
            }
            if state != previous {
                let editor_status = self
                    .editor_status
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                self.push_status_event(
                    "managed_state_changed",
                    managed_state_name(previous),
                    managed_state_name(state),
                    self.generation.load(Ordering::SeqCst),
                    editor_status,
                    observed_at_ms,
                );
            }
            if state == MANAGED_STATE_RELOADING || state == MANAGED_STATE_QUITTING {
                if let Ok(mut guard) = self.managed_capabilities.lock() {
                    guard.clear();
                }
            }
            // Entering a reload (or quit) strands any in-flight work.
            if state != previous
                && (state == MANAGED_STATE_RELOADING || state == MANAGED_STATE_QUITTING)
            {
                self.interrupt_for_reload();
            }
            self.publish_status_snapshot();
        }

        fn heartbeat(&self, generation: i64) {
            if generation > 0 {
                self.generation.store(generation, Ordering::SeqCst);
            }
            self.last_heartbeat_ms.store(now_ms(), Ordering::SeqCst);
        }

        fn set_capabilities(&self, caps: String) {
            if let Ok(mut guard) = self.managed_capabilities.lock() {
                *guard = caps;
            }
            self.publish_status_snapshot();
        }

        fn push_status_event(
            &self,
            kind: &'static str,
            from: &str,
            to: &str,
            domain_generation: i64,
            editor_status: String,
            observed_at_ms: i64,
        ) {
            let seq = self.event_seq.fetch_add(1, Ordering::SeqCst) + 1;
            if let Ok(mut events) = self.events.lock() {
                if events.len() >= STATUS_EVENT_BUFFER_LIMIT {
                    events.pop_front();
                }
                events.push_back(NativeStatusEvent {
                    seq,
                    kind,
                    from: from.to_string(),
                    to: to.to_string(),
                    domain_generation,
                    editor_status,
                    observed_at_ms,
                });
            }
        }
    }

    fn managed_state_name(state: i32) -> &'static str {
        match state {
            MANAGED_STATE_READY => "ready",
            MANAGED_STATE_RELOADING => "reloading",
            MANAGED_STATE_QUITTING => "quitting",
            _ => "initializing",
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            Broker, InflightRequest, MANAGED_STATE_READY, MANAGED_STATE_RELOADING,
            MAX_PENDING_REQUESTS, REQUEST_DEADLINE_MS,
        };
        use serde_json::Value;
        use tokio::sync::mpsc;

        #[test]
        fn complete_drops_stale_or_already_interrupted_requests() {
            let broker = Broker::new(
                "F:/Project".to_string(),
                r"\\.\pipe\locus_unity_native_test".to_string(),
                1,
            );
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);
            *broker.response_tx.lock().unwrap() = Some(tx);

            broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
            assert!(rx.try_recv().is_err());

            broker.inflight.lock().unwrap().insert(
                "req-1".to_string(),
                InflightRequest {
                    deadline_ms: super::now_ms() + REQUEST_DEADLINE_MS,
                },
            );
            broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
            assert_eq!(
                rx.try_recv().unwrap(),
                br#"{"reply_to":"req-1","ok":true}
"#
            );

            broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn enqueue_applies_pending_and_payload_limits() {
            let broker = Broker::new(
                "F:/Project".to_string(),
                r"\\.\pipe\locus_unity_native_test".to_string(),
                1,
            );

            assert_eq!(
                broker.enqueue("too-big".to_string(), vec![0; super::MAX_REQUEST_BYTES + 1]),
                Err("native_payload_too_large")
            );

            for i in 0..MAX_PENDING_REQUESTS {
                broker
                    .enqueue(
                        format!("req-{i}"),
                        br#"{"id":"req","type":"ping"}"#.to_vec(),
                    )
                    .unwrap();
            }
            assert_eq!(
                broker.enqueue(
                    "overflow".to_string(),
                    br#"{"id":"overflow","type":"ping"}"#.to_vec(),
                ),
                Err("native_queue_full")
            );
        }

        #[test]
        fn shared_memory_status_contains_events_and_cursor() {
            let broker = Broker::new(
                "F:/Project".to_string(),
                r"\\.\pipe\locus_unity_native_test".to_string(),
                1,
            );

            broker.set_managed_state(MANAGED_STATE_READY, 1, Some("editing".to_string()));
            let (_, initial_json) = broker.shared_memory_status_json();
            let initial: Value = serde_json::from_str(&initial_json).unwrap();
            let cursor = initial["cursor"].as_u64().unwrap();
            assert!(cursor >= 1);
            assert_eq!(initial["events"].as_array().unwrap().len(), 1);

            broker.set_managed_state(MANAGED_STATE_RELOADING, 1, Some("editing".to_string()));
            let (_, with_events_json) = broker.shared_memory_status_json();
            let with_events: Value = serde_json::from_str(&with_events_json).unwrap();
            let events = with_events["events"].as_array().unwrap();
            let event = events
                .iter()
                .find(|event| event["seq"].as_u64().unwrap_or(0) > cursor)
                .expect("new lifecycle event");
            assert_eq!(event["kind"], "managed_state_changed");
            assert_eq!(event["from"], "ready");
            assert_eq!(event["to"], "reloading");

            let next_cursor = with_events["cursor"].as_u64().unwrap();
            assert!(next_cursor > cursor);
        }

        #[test]
        fn shared_memory_status_trims_events_to_payload_budget() {
            let broker = Broker::new(
                "F:/Project".to_string(),
                r"\\.\pipe\locus_unity_native_test".to_string(),
                1,
            );

            for i in 0..super::STATUS_EVENT_BUFFER_LIMIT {
                broker.push_status_event(
                    "managed_state_changed",
                    "ready",
                    "reloading",
                    i as i64,
                    "editing".repeat(1024),
                    super::now_ms(),
                );
            }

            let (_, json) = broker.shared_memory_status_json();
            assert!(json.len() <= super::NATIVE_STATE_MMF_MAX_PAYLOAD);
            let value: Value = serde_json::from_str(&json).unwrap();
            assert!(value["nativeAlive"].as_bool().unwrap());
            assert_eq!(value["managedState"], "initializing");
            assert!(value["events"].as_array().unwrap().len() < super::STATUS_EVENT_BUFFER_LIMIT);
        }
    }

    // ── Pipe server (background tokio runtime) ──────────────────────────────

    /// Dispatch one inbound envelope line: answer the cheap/liveness commands
    /// directly (they must work during a reload), queue everything else for the
    /// managed executor, or fail fast when the executor cannot serve it.
    fn handle_line(broker: &Arc<Broker>, line: &[u8]) {
        let trimmed = trim_frame(line);
        if trimmed.is_empty() {
            return;
        }

        let value: Value = match serde_json::from_slice(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[locus-native] failed to parse request: {e}");
                return;
            }
        };

        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let id = value.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            eprintln!("[locus-native] request missing id: type={kind}");
            return;
        }

        match kind {
            "ping" => broker.respond_ok(id, Some("pong".to_string())),
            "status" => broker.respond_status(id),
            "bridge_capabilities" => broker.respond_ok(id, Some(broker.capabilities_string())),
            _ => match broker.managed_state() {
                MANAGED_STATE_READY => {
                    if let Err(code) = broker.enqueue(id.to_string(), trimmed.to_vec()) {
                        broker.respond_error(id, code);
                    }
                }
                MANAGED_STATE_RELOADING => broker.respond_error(id, "managed_reloading"),
                MANAGED_STATE_QUITTING => broker.respond_error(id, "unity_process_exiting"),
                _ => broker.respond_error(id, "managed_not_ready"),
            },
        }
    }

    fn trim_frame(line: &[u8]) -> &[u8] {
        let mut start = 0;
        let mut end = line.len();
        // Strip a UTF-8 BOM and surrounding ASCII whitespace.
        if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
            start = 3;
        }
        while start < end && line[start].is_ascii_whitespace() {
            start += 1;
        }
        while end > start && line[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        &line[start..end]
    }

    async fn serve_connection(broker: &Arc<Broker>, server: NamedPipeServer) {
        let (read_half, mut write_half) = tokio::io::split(server);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(WRITER_CHANNEL_LIMIT);

        if let Ok(mut guard) = broker.response_tx.lock() {
            *guard = Some(tx);
        }
        broker.connected.store(true, Ordering::SeqCst);
        broker.publish_status_snapshot();
        eprintln!("[locus-native] client connected: {}", broker.pipe_name);

        let writer = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if write_half.write_all(&bytes).await.is_err() {
                    break;
                }
                if write_half.flush().await.is_err() {
                    break;
                }
            }
        });

        let mut reader = BufReader::new(read_half);
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        loop {
            buf.clear();
            let read = tokio::select! {
                _ = broker.shutdown_notify.notified() => break,
                r = reader.read_until(b'\n', &mut buf) => r,
            };
            match read {
                Ok(0) => break, // client closed
                Ok(_) => handle_line(broker, &buf),
                Err(e) => {
                    eprintln!("[locus-native] pipe read error: {e}");
                    break;
                }
            }
        }

        broker.connected.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = broker.response_tx.lock() {
            *guard = None; // drops the only sender → the writer task ends
        }
        broker.clear_requests_on_disconnect();
        broker.publish_status_snapshot();
        let _ = writer.await;
        eprintln!("[locus-native] client disconnected: {}", broker.pipe_name);
    }

    async fn broker_main(broker: Arc<Broker>, pipe_name: String) {
        eprintln!("[locus-native] broker listening on {pipe_name}");
        loop {
            if broker.shutdown.load(Ordering::SeqCst) {
                break;
            }

            let server = match ServerOptions::new()
                .access_inbound(true)
                .access_outbound(true)
                .pipe_mode(PipeMode::Byte)
                .create(&pipe_name)
            {
                Ok(server) => server,
                Err(e) => {
                    eprintln!("[locus-native] create pipe failed: {e}");
                    tokio::select! {
                        _ = broker.shutdown_notify.notified() => break,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => continue,
                    }
                }
            };

            tokio::select! {
                _ = broker.shutdown_notify.notified() => break,
                connect = server.connect() => match connect {
                    Ok(()) => serve_connection(&broker, server).await,
                    Err(e) => {
                        eprintln!("[locus-native] wait-for-connection failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                },
            }
        }
        eprintln!("[locus-native] broker loop exited");
    }

    // ── FFI entry points (called from C# on the editor threads) ─────────────

    fn normalize_pipe_name(pipe_name: String) -> String {
        let value = pipe_name.trim().to_string();
        if value.starts_with(r"\\.\pipe\") {
            value
        } else {
            format!(r"\\.\pipe\{}", value.trim_start_matches('\\'))
        }
    }

    pub fn init(project: String, pipe_name: String, protocol_version: i32) -> i32 {
        let pipe_name = normalize_pipe_name(pipe_name);
        if BROKER.get().is_some() {
            // Already running (e.g. re-called after a domain reload). Keep the
            // live pipe + queue; the managed side re-registers via state pushes.
            return 0;
        }

        let broker = Arc::new(Broker::new(project, pipe_name.clone(), protocol_version));
        if BROKER.set(broker.clone()).is_err() {
            return 0; // lost an init race; the winner is running
        }
        broker.publish_status_snapshot();

        let thread_broker = broker;
        let spawn = std::thread::Builder::new()
            .name("locus-native-broker".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("[locus-native] failed to build runtime: {e}");
                        return;
                    }
                };
                runtime.block_on(broker_main(thread_broker, pipe_name));
            });

        match spawn {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[locus-native] failed to spawn broker thread: {e}");
                -1
            }
        }
    }

    pub fn shutdown() {
        if let Some(broker) = BROKER.get() {
            let _ = crate::hook::set_active(false);
            broker.shutdown.store(true, Ordering::SeqCst);
            broker.shutdown_notify.notify_waiters();
            broker.set_managed_state(MANAGED_STATE_QUITTING, 0, None);
        }
    }

    pub fn set_managed_state(state: i32, generation: i64, editor_status: Option<String>) {
        if let Some(broker) = BROKER.get() {
            broker.set_managed_state(state, generation, editor_status);
        }
    }

    pub fn managed_heartbeat(generation: i64) {
        if let Some(broker) = BROKER.get() {
            broker.heartbeat(generation);
        }
    }

    pub fn set_capabilities(caps: String) {
        if let Some(broker) = BROKER.get() {
            broker.set_capabilities(caps);
        }
    }

    pub fn complete_request(id: &str, response: Vec<u8>) {
        if let Some(broker) = BROKER.get() {
            broker.complete(id, response);
        }
    }

    pub fn emit_event(event_type: &str, payload: String) {
        if let Some(broker) = BROKER.get() {
            broker.emit_event(event_type, payload);
        }
    }

    /// Copy the next queued request line into `buf`. Returns the byte length
    /// written, `0` when the queue is empty, or `-1` when `buf` is too small
    /// (the request is left queued and `*out_required` is set to the size the
    /// caller must allocate).
    ///
    /// # Safety
    /// `buf` must be valid for `buf_len` bytes (or null with `buf_len == 0`);
    /// `out_required` must be valid or null.
    pub unsafe fn poll_request(buf: *mut u8, buf_len: i32, out_required: *mut i32) -> i32 {
        let Some(broker) = BROKER.get() else {
            return 0;
        };
        let Some(needed) = broker.next_request_len() else {
            return 0;
        };
        if !out_required.is_null() {
            *out_required = needed.min(i32::MAX as usize) as i32;
        }
        if buf.is_null() || (buf_len as usize) < needed {
            return -1; // leave it queued; caller grows the buffer and retries
        }
        let Some(req) = broker.take_next_request() else {
            return 0; // drained out from under us (reload/disconnect)
        };
        // Re-check the size in case the front changed; it never grows, so this
        // is belt-and-suspenders.
        if (buf_len as usize) < req.raw.len() {
            if let Ok(mut inflight) = broker.inflight.lock() {
                inflight.remove(&req.id);
            }
            broker.requeue_front(req);
            return -1;
        }
        std::ptr::copy_nonoverlapping(req.raw.as_ptr(), buf, req.raw.len());
        req.raw.len() as i32
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// In-process background hook (migration Phase 6)
// ─────────────────────────────────────────────────────────────────────────────
//
// Unity stops pumping the editor loop when its window is not the foreground OS
// app (`IsApplicationActive` / `IsApplicationActiveOSImpl` return 0). Locus
// needs the editor to keep ticking while the Locus window has focus, so those
// two engine functions are patched to unconditionally return 1.
//
// The Tauri side already does this cross-process (OpenProcess +
// WriteProcessMemory; see `unity_bridge/background_hook.rs`). Doing it *here* —
// inside the Unity process, from the broker DLL that persists across domain
// reloads — means the patch lives in native engine memory: it survives domain
// reloads with no re-sync and needs no cross-process memory rights. The managed
// side calls `locus_set_background_active(1)` when the feature marker is present;
// Tauri only falls back to its cross-process patch when this path is inactive,
// so the in-process hook is a pure optimization that fails open.
//
// The symbol list and patch bytes MUST stay identical to
// `background_hook.rs` (a parity test pins this).

#[cfg(windows)]
mod hook {
    use std::ffi::{c_void, CString, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use std::sync::{Mutex, OnceLock};

    type Bool = i32;
    type Dword = u32;
    type Dword64 = u64;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const TH32CS_SNAPMODULE: Dword = 0x0000_0008;
    const TH32CS_SNAPMODULE32: Dword = 0x0000_0010;
    const MAX_MODULE_NAME32: usize = 255;
    const MAX_PATH: usize = 260;
    const PAGE_EXECUTE_READWRITE: Dword = 0x40;
    const SYMOPT_UNDNAME: Dword = 0x0000_0002;
    const SYMOPT_DEFERRED_LOADS: Dword = 0x0000_0004;
    const SYMOPT_FAIL_CRITICAL_ERRORS: Dword = 0x0000_0200;
    const SYMOPT_AUTO_PUBLICS: Dword = 0x0001_0000;
    const MAX_SYM_NAME: usize = 2048;

    /// `mov eax, 1; ret` — the patched function unconditionally reports "active".
    /// MUST match `background_hook.rs::PATCH_BYTES`.
    const PATCH_BYTES: [u8; 6] = [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3];
    /// MUST match `background_hook.rs::SYMBOLS`.
    const SYMBOLS: [&str; 2] = [
        "Unity!IsApplicationActive",
        "Unity!IsApplicationActiveOSImpl",
    ];

    /// A dbghelp "process" key private to this module. Resolution is
    /// non-invasive (`fInvadeProcess = FALSE` plus an explicit
    /// `SymLoadModuleExW` base/size), so the key need not be a real process
    /// handle — using a private sentinel instead of `GetCurrentProcess()`
    /// guarantees our `SymInitialize`/`SymCleanup` never tears down a dbghelp
    /// session Unity itself opened on the real current-process handle.
    const SYM_HANDLE: Handle = 0x4C4F_4355 as Handle; // 'LOCU'

    #[allow(non_snake_case)]
    #[repr(C)]
    struct ModuleEntry32W {
        dwSize: Dword,
        th32ModuleID: Dword,
        th32ProcessID: Dword,
        GlblcntUsage: Dword,
        ProccntUsage: Dword,
        modBaseAddr: *mut u8,
        modBaseSize: Dword,
        hModule: Handle,
        szModule: [u16; MAX_MODULE_NAME32 + 1],
        szExePath: [u16; MAX_PATH],
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfo {
        SizeOfStruct: Dword,
        TypeIndex: Dword,
        Reserved: [Dword64; 2],
        Index: Dword,
        Size: Dword,
        ModBase: Dword64,
        Flags: Dword,
        Value: Dword64,
        Address: Dword64,
        Register: Dword,
        Scope: Dword,
        Tag: Dword,
        NameLen: Dword,
        MaxNameLen: Dword,
        Name: [u8; 1],
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfoBuffer {
        Symbol: SymbolInfo,
        NameBuffer: [u8; MAX_SYM_NAME],
    }

    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn SymInitializeW(
            hProcess: Handle,
            UserSearchPath: *const u16,
            fInvadeProcess: Bool,
        ) -> Bool;
        fn SymSetOptions(SymOptions: Dword) -> Dword;
        fn SymLoadModuleExW(
            hProcess: Handle,
            hFile: Handle,
            ImageName: *const u16,
            ModuleName: *const u16,
            BaseOfDll: Dword64,
            DllSize: Dword,
            Data: *mut c_void,
            Flags: Dword,
        ) -> Dword64;
        fn SymFromName(hProcess: Handle, Name: *const u8, Symbol: *mut SymbolInfo) -> Bool;
        fn SymCleanup(hProcess: Handle) -> Bool;
    }

    unsafe extern "system" {
        fn CloseHandle(hObject: Handle) -> Bool;
        fn CreateToolhelp32Snapshot(dwFlags: Dword, th32ProcessID: Dword) -> Handle;
        fn Module32FirstW(hSnapshot: Handle, lpme: *mut ModuleEntry32W) -> Bool;
        fn Module32NextW(hSnapshot: Handle, lpme: *mut ModuleEntry32W) -> Bool;
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> Dword;
        fn VirtualProtect(
            lpAddress: *mut c_void,
            dwSize: usize,
            flNewProtect: Dword,
            lpflOldProtect: *mut Dword,
        ) -> Bool;
        fn FlushInstructionCache(
            hProcess: Handle,
            lpBaseAddress: *const c_void,
            dwSize: usize,
        ) -> Bool;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn new(handle: Handle) -> Result<Self, String> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(last_error("handle open"))
            } else {
                Ok(Self(handle))
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

    struct SymSession;

    impl SymSession {
        fn new(search_path: &Path) -> Result<Self, String> {
            let search = wide_null(search_path.as_os_str());
            unsafe {
                SymSetOptions(
                    SYMOPT_UNDNAME
                        | SYMOPT_DEFERRED_LOADS
                        | SYMOPT_FAIL_CRITICAL_ERRORS
                        | SYMOPT_AUTO_PUBLICS,
                );
                if SymInitializeW(SYM_HANDLE, search.as_ptr(), FALSE) == 0 {
                    return Err(last_error("SymInitializeW"));
                }
            }
            Ok(Self)
        }
    }

    impl Drop for SymSession {
        fn drop(&mut self) {
            unsafe {
                let _ = SymCleanup(SYM_HANDLE);
            }
        }
    }

    struct ModuleInfo {
        base: u64,
        size: u32,
        path: String,
    }

    struct PatchRecord {
        address: u64,
        original: [u8; PATCH_BYTES.len()],
        /// True when *we* wrote the patch (so restore should put the original
        /// back). False when the bytes were already patched on entry.
        applied: bool,
    }

    #[derive(Default)]
    struct HookState {
        records: Vec<PatchRecord>,
        symbol_count: u32,
        last_error: Option<String>,
    }

    fn state() -> &'static Mutex<HookState> {
        static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(HookState::default()))
    }

    /// `(patched, symbol_count)` for the broker status JSON.
    pub fn snapshot() -> (bool, u32) {
        state()
            .lock()
            .map(|s| (!s.records.is_empty(), s.symbol_count))
            .unwrap_or((false, 0))
    }

    /// Apply (or restore) the in-process background patch. Idempotent: a second
    /// `set_active(true)` returns the cached patch state without resolving PDB
    /// symbols or touching code pages again.
    pub fn set_active(active: bool) -> Result<u32, String> {
        let mut st = state()
            .lock()
            .map_err(|e| format!("background hook state poisoned: {e}"))?;
        if !active {
            restore_locked(&mut st);
            st.last_error = None;
            return Ok(0);
        }
        if !st.records.is_empty() {
            st.last_error = None;
            return Ok(st.symbol_count);
        }

        st.records.clear();
        match apply_all() {
            Ok(records) => {
                st.symbol_count = records.len() as u32;
                st.records = records;
                st.last_error = None;
                Ok(st.symbol_count)
            }
            Err(error) => {
                st.symbol_count = 0;
                st.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn restore_locked(st: &mut HookState) {
        for record in st.records.drain(..).rev() {
            if record.applied {
                let _ = write_code(record.address, &record.original);
            }
        }
        st.symbol_count = 0;
    }

    fn apply_all() -> Result<Vec<PatchRecord>, String> {
        let module = find_unity_engine_module()?;
        let image_path = Path::new(&module.path);
        let symbol_dir = image_path.parent().ok_or_else(|| {
            format!(
                "Unity engine module has no parent directory: {}",
                module.path
            )
        })?;
        let pdb_path = symbol_dir.join("unity_x64.pdb");
        if !pdb_path.is_file() {
            return Err(format!("Unity PDB is missing: {}", pdb_path.display()));
        }

        let mut records: Vec<PatchRecord> = Vec::new();
        for symbol in SYMBOLS {
            let address = match resolve_symbol(image_path, symbol_dir, &module, symbol) {
                Ok(address) => address,
                Err(error) => {
                    rollback(&records);
                    return Err(error);
                }
            };
            if address < module.base || address >= module.base.saturating_add(module.size as u64) {
                rollback(&records);
                return Err(format!(
                    "Resolved symbol {symbol} outside Unity module: 0x{address:X}"
                ));
            }

            let original = read_code(address);
            let already_patched = original.starts_with(&PATCH_BYTES);
            if !already_patched {
                if let Err(error) = write_code(address, &PATCH_BYTES) {
                    rollback(&records);
                    return Err(error);
                }
            }
            records.push(PatchRecord {
                address,
                original,
                applied: !already_patched,
            });
        }
        Ok(records)
    }

    fn rollback(records: &[PatchRecord]) {
        for record in records.iter().rev() {
            if record.applied {
                let _ = write_code(record.address, &record.original);
            }
        }
    }

    fn find_unity_engine_module() -> Result<ModuleInfo, String> {
        let pid = unsafe { GetCurrentProcessId() };
        let snapshot =
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
        let snapshot = OwnedHandle::new(snapshot)
            .map_err(|error| format!("Failed to snapshot own modules (PID {pid}): {error}"))?;

        let mut entry: ModuleEntry32W = unsafe { zeroed() };
        entry.dwSize = size_of::<ModuleEntry32W>() as u32;
        let mut has_entry = unsafe { Module32FirstW(snapshot.raw(), &mut entry) != 0 };
        let mut exe_module = None;
        while has_entry {
            let name = wide_to_string(&entry.szModule);
            if name.eq_ignore_ascii_case("Unity.dll") {
                return Ok(module_info_from_entry(&entry));
            }
            if name.eq_ignore_ascii_case("Unity.exe") {
                exe_module = Some(module_info_from_entry(&entry));
            }
            has_entry = unsafe { Module32NextW(snapshot.raw(), &mut entry) != 0 };
        }
        exe_module.ok_or_else(|| "Unity engine module was not found in this process".to_string())
    }

    fn module_info_from_entry(entry: &ModuleEntry32W) -> ModuleInfo {
        ModuleInfo {
            base: entry.modBaseAddr as usize as u64,
            size: entry.modBaseSize,
            path: wide_to_string(&entry.szExePath),
        }
    }

    fn resolve_symbol(
        image_path: &Path,
        symbol_path: &Path,
        module: &ModuleInfo,
        symbol_name: &str,
    ) -> Result<u64, String> {
        // Held for its Drop (SymCleanup); resolution uses the SYM_HANDLE const.
        let _session = SymSession::new(symbol_path)?;
        let image = wide_null(image_path.as_os_str());
        let module_name = wide_null(OsStr::new("Unity"));
        let loaded = unsafe {
            SymLoadModuleExW(
                SYM_HANDLE,
                null_mut(),
                image.as_ptr(),
                module_name.as_ptr(),
                module.base,
                module.size,
                null_mut(),
                0,
            )
        };
        if loaded == 0 {
            return Err(last_error("SymLoadModuleExW"));
        }

        let mut storage: Box<SymbolInfoBuffer> = unsafe { Box::new(zeroed()) };
        let symbol = &mut storage.Symbol as *mut SymbolInfo;
        unsafe {
            (*symbol).SizeOfStruct = size_of::<SymbolInfo>() as u32;
            (*symbol).MaxNameLen = MAX_SYM_NAME as u32;
        }
        let name = CString::new(symbol_name)
            .map_err(|_| format!("Symbol name contains interior nul: {symbol_name}"))?;
        let ok = unsafe { SymFromName(SYM_HANDLE, name.as_ptr() as *const u8, symbol) != 0 };
        if !ok {
            return Err(format!(
                "Failed to resolve Unity symbol {symbol_name}: {}",
                last_error("SymFromName")
            ));
        }
        Ok(unsafe { (*symbol).Address })
    }

    /// Read the leading `PATCH_BYTES.len()` bytes of an in-process code address.
    fn read_code(address: u64) -> [u8; PATCH_BYTES.len()] {
        unsafe { std::ptr::read_unaligned(address as usize as *const [u8; PATCH_BYTES.len()]) }
    }

    /// Overwrite an in-process code address, flipping page protection around the
    /// write and flushing the instruction cache. Own address space, so no
    /// cross-process memory APIs are needed.
    fn write_code(address: u64, bytes: &[u8]) -> Result<(), String> {
        let ptr = address as usize as *mut c_void;
        let mut old_protect = 0u32;
        let unprotected = unsafe {
            VirtualProtect(ptr, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old_protect) != 0
        };
        if !unprotected {
            return Err(format!(
                "Failed to unprotect Unity code at 0x{address:X}: {}",
                last_error("VirtualProtect")
            ));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as usize as *mut u8, bytes.len());
            let mut ignored = 0u32;
            let _ = FlushInstructionCache(GetCurrentProcess(), ptr, bytes.len());
            let _ = VirtualProtect(ptr, bytes.len(), old_protect, &mut ignored);
        }
        Ok(())
    }

    fn last_error(label: &str) -> String {
        format!("{label}: {}", std::io::Error::last_os_error())
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn wide_to_string(value: &[u16]) -> String {
        let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..len])
    }

    #[cfg(test)]
    mod tests {
        use super::{PATCH_BYTES, SYMBOLS};

        /// Parity with `unity_bridge/background_hook.rs`: the cross-process and
        /// in-process patches must touch the same symbols with the same bytes.
        #[test]
        fn patch_constants_match_cross_process_hook() {
            assert_eq!(PATCH_BYTES, [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3]);
            assert_eq!(
                SYMBOLS,
                [
                    "Unity!IsApplicationActive",
                    "Unity!IsApplicationActiveOSImpl",
                ]
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Overlay control client (migration Phase 5)
// ─────────────────────────────────────────────────────────────────────────────
//
// The Locus overlay is positioned by control messages the Unity editor pushes
// to a Tauri-hosted pipe (`\\.\pipe\locus_tauri_unity_embed_{proj}`). Today the
// managed `LocusEditorWindow` owns that client connection, so it drops on every
// domain reload and the overlay flickers/desyncs until the new domain
// reconnects. Here the broker DLL owns a *write-only* client that persists
// across reloads: the managed side still computes all geometry and builds the
// same JSON, but pushes it through `locus_overlay_push` instead of owning the
// pipe. On a (re)connect the last open/update line is replayed so the overlay
// re-syncs immediately, and because the connection never drops on reload the
// managed side can send a `managedOverlayState=reloading` message that the Tauri
// server uses to hold the overlay in place. Fails open: when the broker or this
// client is unavailable the managed side keeps using its own pipe client.

#[cfg(windows)]
mod overlay {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::sync::{mpsc, Notify};

    const OVERLAY_WRITER_CHANNEL_LIMIT: usize = 256;

    struct OverlayClient {
        pipe_name: String,
        shutdown: AtomicBool,
        shutdown_notify: Notify,
        connected: AtomicBool,
        /// Writer channel for the current connection; `None` while disconnected.
        tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
        /// The last non-`close` control line (no trailing newline). Replayed
        /// right after a (re)connect so a freshly (re)started Tauri overlay
        /// server re-syncs the overlay without waiting for the next push.
        last_sync: Mutex<Option<Vec<u8>>>,
    }

    static OVERLAY: OnceLock<Arc<OverlayClient>> = OnceLock::new();

    fn frame(mut line: Vec<u8>) -> Vec<u8> {
        line.push(b'\n');
        line
    }

    /// A `close` message ends the overlay session and must not be replayed on a
    /// later reconnect. Everything else (`open`/`update`/reloading markers) is
    /// the current desired state and is cached for replay.
    fn is_close(line: &[u8]) -> bool {
        // Managed JSON is produced by `JsonUtility.ToJson` (no spaces), so this
        // exact-substring match is stable.
        line.windows(14).any(|w| w == br#""type":"close""#)
    }

    /// Idempotent: a second call (e.g. after a domain reload) keeps the live
    /// connection. Returns 0 on success, -1 on a bad pipe name.
    pub fn connect(pipe_name: String) -> i32 {
        let pipe_name = pipe_name.trim().to_string();
        if pipe_name.is_empty() {
            return -1;
        }
        if OVERLAY.get().is_some() {
            return 0;
        }
        let client = Arc::new(OverlayClient {
            pipe_name: pipe_name.clone(),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            connected: AtomicBool::new(false),
            tx: Mutex::new(None),
            last_sync: Mutex::new(None),
        });
        if OVERLAY.set(client.clone()).is_err() {
            return 0;
        }
        let spawn = std::thread::Builder::new()
            .name("locus-native-overlay".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("[locus-native] overlay runtime build failed: {e}");
                        return;
                    }
                };
                runtime.block_on(client_loop(client));
            });
        match spawn {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[locus-native] failed to spawn overlay thread: {e}");
                -1
            }
        }
    }

    /// Forward a control line to the Tauri overlay server, caching it for replay
    /// unless it is a `close`. No-op (the line is still cached) when the client
    /// is momentarily disconnected — the cached line is replayed on reconnect.
    pub fn push(line: Vec<u8>) {
        let Some(client) = OVERLAY.get() else {
            return;
        };
        if is_close(&line) {
            if let Ok(mut guard) = client.last_sync.lock() {
                *guard = None;
            }
        } else if let Ok(mut guard) = client.last_sync.lock() {
            *guard = Some(line.clone());
        }
        if let Ok(guard) = client.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                match tx.try_send(frame(line)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        eprintln!(
                            "[locus-native] overlay writer queue full (limit {})",
                            OVERLAY_WRITER_CHANNEL_LIMIT
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {}
                }
            }
        }
    }

    pub fn shutdown() {
        if let Some(client) = OVERLAY.get() {
            client.shutdown.store(true, Ordering::SeqCst);
            client.shutdown_notify.notify_waiters();
        }
    }

    pub fn connected() -> bool {
        OVERLAY
            .get()
            .map(|c| c.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    async fn client_loop(client: Arc<OverlayClient>) {
        eprintln!(
            "[locus-native] overlay client targeting {}",
            client.pipe_name
        );
        loop {
            if client.shutdown.load(Ordering::SeqCst) {
                break;
            }
            match ClientOptions::new().open(&client.pipe_name) {
                Ok(pipe) => serve(&client, pipe).await,
                Err(_) => {
                    tokio::select! {
                        _ = client.shutdown_notify.notified() => break,
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    }
                }
            }
        }
        eprintln!("[locus-native] overlay client loop exited");
    }

    async fn serve(
        client: &Arc<OverlayClient>,
        mut pipe: tokio::net::windows::named_pipe::NamedPipeClient,
    ) {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OVERLAY_WRITER_CHANNEL_LIMIT);
        if let Ok(mut guard) = client.tx.lock() {
            *guard = Some(tx.clone());
        }
        client.connected.store(true, Ordering::SeqCst);

        // Replay the current overlay geometry so the (re)connected server syncs
        // immediately rather than waiting for the next geometry change.
        if let Some(last) = client.last_sync.lock().ok().and_then(|g| g.clone()) {
            let _ = tx.send(frame(last));
        }

        loop {
            let next = tokio::select! {
                _ = client.shutdown_notify.notified() => break,
                msg = rx.recv() => msg,
            };
            let Some(bytes) = next else { break };
            if pipe.write_all(&bytes).await.is_err() {
                break;
            }
            if pipe.flush().await.is_err() {
                break;
            }
        }

        client.connected.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = client.tx.lock() {
            *guard = None;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_close;

        #[test]
        fn is_close_matches_only_close_messages() {
            assert!(is_close(br#"{"type":"close","reason":"windowClosed"}"#));
            assert!(!is_close(br#"{"type":"update","x":1,"y":2}"#));
            assert!(!is_close(br#"{"type":"open","visible":true}"#));
            assert!(!is_close(b""));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FFI surface (stable C ABI). Bodies route to the Windows impl or inert stubs.
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize the broker and start the pipe server. Idempotent: a second call
/// (e.g. after a domain reload) is a no-op that keeps the live pipe. Returns 0
/// on success, non-zero on failure.
///
/// # Safety
/// `project`/`pipe` must each be valid for their stated lengths or null.
#[no_mangle]
pub unsafe extern "C" fn locus_init(
    project: *const u8,
    project_len: i32,
    pipe: *const u8,
    pipe_len: i32,
    protocol_version: c_int,
) -> c_int {
    #[cfg(windows)]
    {
        let project = string_from_raw(project, project_len);
        let pipe_name = string_from_raw(pipe, pipe_len);
        if pipe_name.is_empty() {
            return -1;
        }
        imp::init(project, pipe_name, protocol_version)
    }
    #[cfg(not(windows))]
    {
        let _ = (project, project_len, pipe, pipe_len, protocol_version);
        -1
    }
}

/// Stop the pipe server and release the background thread. Not called on domain
/// reload (the broker must persist) — only on Unity quit.
#[no_mangle]
pub extern "C" fn locus_shutdown() {
    #[cfg(windows)]
    {
        imp::shutdown();
        overlay::shutdown();
    }
}

/// Register the managed lifecycle state. `state` is one of the
/// `MANAGED_STATE_*` values; `generation` is the domain generation (ignored if
/// `<= 0`); `editor_status` is the cached `editing|playing|…|scene` string used
/// to answer the bare `status` command during a reload.
///
/// # Safety
/// `editor_status` must be valid for `editor_status_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_set_managed_state(
    state: c_int,
    generation: i64,
    editor_status: *const u8,
    editor_status_len: i32,
) {
    #[cfg(windows)]
    {
        let status = if editor_status.is_null() {
            None
        } else {
            Some(string_from_raw(editor_status, editor_status_len))
        };
        imp::set_managed_state(state, generation, status);
    }
    #[cfg(not(windows))]
    {
        let _ = (state, generation, editor_status, editor_status_len);
    }
}

/// Refresh the managed liveness timestamp (and generation if `> 0`).
#[no_mangle]
pub extern "C" fn locus_managed_heartbeat(generation: i64) {
    #[cfg(windows)]
    {
        imp::managed_heartbeat(generation);
    }
    #[cfg(not(windows))]
    {
        let _ = generation;
    }
}

/// Register the managed executor's capability string (merged with native caps
/// when answering `bridge_capabilities`).
///
/// # Safety
/// `caps` must be valid for `caps_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_set_capabilities(caps: *const u8, caps_len: i32) {
    #[cfg(windows)]
    {
        imp::set_capabilities(string_from_raw(caps, caps_len));
    }
    #[cfg(not(windows))]
    {
        let _ = (caps, caps_len);
    }
}

/// Dequeue the next request line for the managed executor. See
/// [`imp::poll_request`] for the buffer protocol.
///
/// # Safety
/// `buffer` must be valid for `buffer_len` bytes (or null when `buffer_len`
/// is 0); `out_required_len` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn locus_poll_request(
    buffer: *mut u8,
    buffer_len: i32,
    out_required_len: *mut i32,
) -> c_int {
    #[cfg(windows)]
    {
        imp::poll_request(buffer, buffer_len, out_required_len)
    }
    #[cfg(not(windows))]
    {
        let _ = (buffer, buffer_len, out_required_len);
        0
    }
}

/// Hand a completed managed response (a full envelope JSON line) back to the
/// broker, which writes it to the client and clears the in-flight entry.
///
/// # Safety
/// `id`/`response` must each be valid for their stated lengths or null.
#[no_mangle]
pub unsafe extern "C" fn locus_complete_request(
    id: *const u8,
    id_len: i32,
    response: *const u8,
    response_len: i32,
) {
    #[cfg(windows)]
    {
        let id = string_from_raw(id, id_len);
        let response = slice_from_raw(response, response_len).to_vec();
        imp::complete_request(&id, response);
    }
    #[cfg(not(windows))]
    {
        let _ = (id, id_len, response, response_len);
    }
}

/// Emit an unsolicited event (no `reply_to`) to the connected client.
///
/// # Safety
/// `event_type`/`payload` must each be valid for their stated lengths or null.
#[no_mangle]
pub unsafe extern "C" fn locus_emit_event(
    event_type: *const u8,
    type_len: i32,
    payload: *const u8,
    payload_len: i32,
) {
    #[cfg(windows)]
    {
        let event_type = string_from_raw(event_type, type_len);
        if event_type.is_empty() {
            return;
        }
        imp::emit_event(&event_type, string_from_raw(payload, payload_len));
    }
    #[cfg(not(windows))]
    {
        let _ = (event_type, type_len, payload, payload_len);
    }
}

/// Apply (`active != 0`) or restore (`active == 0`) the in-process background
/// hook that keeps the Unity editor ticking while its window is not focused.
/// Returns the number of patched engine symbols on success, or `-1` on failure.
/// On failure the managed caller leaves the cross-process Tauri hook to do the
/// patch instead — this path fails open and is a pure optimization.
///
/// # Safety
/// Takes no pointers; safe to call from any managed (editor) thread.
#[no_mangle]
pub extern "C" fn locus_set_background_active(active: c_int) -> c_int {
    #[cfg(windows)]
    {
        match hook::set_active(active != 0) {
            Ok(count) => count as c_int,
            Err(error) => {
                eprintln!("[locus-native] background hook set_active failed: {error}");
                -1
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = active;
        -1
    }
}

/// Open (idempotently) the persistent write-only client to the Tauri overlay
/// control pipe. The connection survives domain reloads, so the managed side
/// stops owning it. Returns 0 on success, -1 on a bad pipe name / spawn failure.
///
/// # Safety
/// `pipe` must be valid for `pipe_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_overlay_connect(pipe: *const u8, pipe_len: i32) -> c_int {
    #[cfg(windows)]
    {
        overlay::connect(string_from_raw(pipe, pipe_len))
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe, pipe_len);
        -1
    }
}

/// Forward one overlay control line (a full `EmbedControlMessage` JSON, no
/// trailing newline) to the Tauri overlay server, caching it for replay on the
/// next reconnect unless it is a `close`. No-op when the overlay client is not
/// connected (the cached line is replayed once it reconnects).
///
/// # Safety
/// `line` must be valid for `line_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_overlay_push(line: *const u8, line_len: i32) {
    #[cfg(windows)]
    {
        let bytes = slice_from_raw(line, line_len).to_vec();
        if !bytes.is_empty() {
            overlay::push(bytes);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (line, line_len);
    }
}
