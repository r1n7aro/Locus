//! Independent macOS broker. The Windows backend and its memory layout are untouched.
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::macos_ipc;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Notify};

use super::{
    MANAGED_STATE_INITIALIZING, MANAGED_STATE_QUITTING, MANAGED_STATE_READY,
    MANAGED_STATE_RELOADING,
};

const NATIVE_CAPABILITIES: &str =
    "broker_v1,broker_state_snapshot_v1,unix_socket_v1,broker_queue_limits_v1,broker_request_accepted_v1";
const REQUEST_ACCEPTED_EVENT: &str = "locus-request-accepted";
const STATUS_EVENT_BUFFER_LIMIT: usize = 256;
const MAX_PENDING_REQUESTS: usize = 128;
const MAX_INFLIGHT_REQUESTS: usize = 64;
const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_DEADLINE_MS: i64 = 10 * 60 * 1000;
const WRITER_CHANNEL_LIMIT: usize = 512;
const NATIVE_STATE_SNAPSHOT_MAX_PAYLOAD: usize = 128 * 1024;
struct NativeStatePlane {
    path: std::path::PathBuf,
}
impl NativeStatePlane {
    fn create(endpoint: &str) -> Option<Self> {
        (!endpoint.is_empty()).then(|| Self {
            path: macos_ipc::state_path(endpoint),
        })
    }
    fn write_json(&mut self, _observed_at_ms: i64, json: &str) {
        if let Err(error) = macos_ipc::write_snapshot(&self.path, json.as_bytes()) {
            eprintln!("[locus-native] state snapshot failed: {error}");
        }
    }
}

/// A request received from the Tauri client that must run on the managed
/// executor. `raw` is the original envelope line (no trailing newline) the
/// managed side parses back with `JsonUtility`.
struct QueuedRequest {
    id: String,
    raw: Vec<u8>,
    deadline_ms: i64,
    reattachable: bool,
}

struct InflightRequest {
    deadline_ms: i64,
    reattachable: bool,
}

/// Process-global broker state. Lives for the lifetime of the Unity process
/// (a domain reload does not unload this dylib), so the pipe + queue survive
/// reloads while the managed executor comes and goes.
pub struct Broker {
    project: String,
    pipe_name: String,
    protocol_version: i32,

    shutdown: AtomicBool,
    shutdown_notify: Notify,
    connection_failed: Notify,

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

    /// Native-owned atomic status snapshot. Managed code publishes
    /// inputs through FFI; this broker owns the independent status file.
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
        let state_plane = NativeStatePlane::create(&pipe_name);
        Self {
            project,
            pipe_name,
            protocol_version,
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            connection_failed: Notify::new(),
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
                        self.connection_failed.notify_one();
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        self.connection_failed.notify_one();
                    }
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
        let (background_patched, background_symbols) = (false, 0);
        let identity = macos_ipc::process_identity(self.process_id);
        let managed_capabilities = self
            .managed_capabilities
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        serde_json::json!({
            "transport": "native_broker",
            "stateVersion": 1,
            "processStartSecs": identity.map(|i| i.0),
            "processStartMicros": identity.map(|i| i.1),
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
            "overlayConnected": false,
            "events": events,
            "cursor": cursor,
        })
    }

    fn state_snapshot_json(&self) -> (i64, String) {
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
            if json.len() <= NATIVE_STATE_SNAPSHOT_MAX_PAYLOAD || events.is_empty() {
                return (observed_at_ms, json);
            }

            let drop_count = ((events.len() + 1) / 2).max(1);
            events.drain(0..drop_count.min(events.len()));
        }
    }

    fn publish_status_snapshot(&self) {
        let (observed_at_ms, json) = self.state_snapshot_json();
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

        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
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

    fn enqueue(&self, id: String, raw: Vec<u8>, reattachable: bool) -> Result<(), &'static str> {
        self.expire_requests();

        if self
            .queue
            .lock()
            .map(|q| q.iter().any(|r| r.id == id))
            .unwrap_or(true)
            || self
                .inflight
                .lock()
                .map(|m| m.contains_key(&id))
                .unwrap_or(true)
        {
            return Err("native_duplicate_request_id");
        }
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
                reattachable,
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
                    reattachable: req.reattachable,
                },
            );
        }
        self.publish_status_snapshot();
        Some(req)
    }

    fn requeue_front(&self, req: QueuedRequest) {
        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
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
            eprintln!("[locus-native] dropping stale completion for non-inflight request: {id}");
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

    /// Preserve only execution requests that can be reattached by their
    /// managed execution_id. Every other accepted request belongs to the
    /// disconnected transport and must not execute later on a new client.
    fn discard_non_reattachable_on_disconnect(&self) {
        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
            q.retain(|request| request.reattachable);
            *queued_bytes = q.iter().map(|request| request.raw.len()).sum();
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.retain(|_, request| request.reattachable);
        }
        self.publish_status_snapshot();
    }

    // ── Managed lifecycle ──────────────────────────────────────────────

    fn set_managed_state(&self, state: i32, generation: i64, editor_status: Option<String>) {
        let current_generation = self.generation.load(Ordering::SeqCst);
        if generation > 0 && generation < current_generation {
            return;
        }
        if generation > current_generation && current_generation > 0 {
            self.interrupt_for_reload();
        }
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
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
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

    if kind.starts_with("hot_reload_") || kind.starts_with("hot_patch_") {
        broker.respond_error(id, "unsupported_on_macos");
        return;
    }

    match kind {
        "ping" => broker.respond_ok(id, Some("pong".to_string())),
        "status" => broker.respond_status(id),
        "bridge_capabilities" => broker.respond_ok(id, Some(broker.capabilities_string())),
        _ => match broker.managed_state() {
            MANAGED_STATE_READY => match broker.enqueue(
                id.to_string(),
                trimmed.to_vec(),
                kind == "execute_code" || kind == "execute_loaded",
            ) {
                Ok(()) => broker.emit_event(
                    REQUEST_ACCEPTED_EVENT,
                    serde_json::json!({
                        "requestId": id,
                        "requestType": kind,
                    })
                    .to_string(),
                ),
                Err(code) => broker.respond_error(id, code),
            },
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

async fn serve_connection(broker: &Arc<Broker>, server: UnixStream) {
    let (read_half, mut write_half) = tokio::io::split(server);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(WRITER_CHANNEL_LIMIT);

    if let Ok(mut guard) = broker.response_tx.lock() {
        *guard = Some(tx);
    }
    broker.connected.store(true, Ordering::SeqCst);
    broker.publish_status_snapshot();
    eprintln!("[locus-native] client connected: {}", broker.pipe_name);

    let writer_broker = broker.clone();
    let writer = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            let write = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                write_half.write_all(&bytes).await?;
                write_half.flush().await
            })
            .await;
            if !matches!(write, Ok(Ok(()))) {
                writer_broker.connection_failed.notify_one();
                break;
            }
        }
    });

    let mut reader = BufReader::new(read_half);
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        if broker.shutdown.load(Ordering::SeqCst) {
            break;
        }
        buf.clear();
        let read = tokio::select! {
            _ = broker.shutdown_notify.notified() => break,
            _ = broker.connection_failed.notified() => break,
            r = macos_ipc::read_frame(&mut reader, &mut buf, MAX_REQUEST_BYTES) => r,
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
    broker.discard_non_reattachable_on_disconnect();
    writer.abort();
    let _ = writer.await;
    eprintln!("[locus-native] client disconnected: {}", broker.pipe_name);
}

async fn broker_main(
    broker: Arc<Broker>,
    listener: std::os::unix::net::UnixListener,
    _endpoint: macos_ipc::EndpointGuard,
) {
    let listener = match UnixListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("[locus-native] socket registration failed: {error}");
            return;
        }
    };
    let publisher_broker = broker.clone();
    let publisher = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            interval.tick().await;
            if publisher_broker.shutdown.load(Ordering::SeqCst) {
                break;
            }
            publisher_broker.publish_status_snapshot();
        }
    });
    loop {
        if broker.shutdown.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            _ = broker.shutdown_notify.notified() => break,
            incoming = listener.accept() => match incoming {
                Ok((stream, _)) => {
                    if macos_ipc::peer_identity(&stream).is_ok() { serve_connection(&broker, stream).await; }
                }
                Err(error) => { eprintln!("[locus-native] socket accept failed: {error}"); break; }
            }
        }
    }
    publisher.abort();
    let _ = publisher.await;
}

// ── FFI entry points (called from C# on the editor threads) ─────────────

pub fn init(project: String, pipe_name: String, protocol_version: i32) -> i32 {
    static INIT_LOCK: Mutex<()> = Mutex::new(());
    let Ok(_init) = INIT_LOCK.lock() else {
        return -1;
    };
    if protocol_version != super::NATIVE_PROTOCOL_VERSION
        || macos_ipc::validate_endpoint(&project, &pipe_name).is_err()
    {
        return -1;
    }
    if let Some(broker) = BROKER.get() {
        return if broker.pipe_name == pipe_name && !broker.shutdown.load(Ordering::SeqCst) {
            0
        } else {
            -1
        };
    }
    let (listener, endpoint) = match macos_ipc::bind_endpoint(&project, &pipe_name) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[locus-native] endpoint rejected: {error}");
            return -1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[locus-native] runtime failed: {error}");
            return -1;
        }
    };
    let broker = Arc::new(Broker::new(project, pipe_name, protocol_version));
    let thread_broker = broker.clone();
    match std::thread::Builder::new()
        .name("locus-native-macos".into())
        .spawn(move || {
            runtime.block_on(broker_main(thread_broker, listener, endpoint));
        }) {
        Ok(_) => {
            let _ = BROKER.set(broker);
            0
        }
        Err(error) => {
            eprintln!("[locus-native] broker thread failed: {error}");
            -1
        }
    }
}

pub fn shutdown() {
    if let Some(broker) = BROKER.get() {
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

pub fn connected() -> bool {
    BROKER
        .get()
        .map(|broker| broker.connected.load(Ordering::SeqCst))
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    fn broker() -> (Arc<Broker>, mpsc::Receiver<Vec<u8>>) {
        let broker = Arc::new(Broker::new("/test-project".into(), String::new(), 1));
        let (tx, rx) = mpsc::channel(64);
        *broker.response_tx.lock().unwrap() = Some(tx);
        broker.set_managed_state(MANAGED_STATE_READY, 1, Some("editing".into()));
        (broker, rx)
    }
    fn frame(rx: &mut mpsc::Receiver<Vec<u8>>) -> Value {
        serde_json::from_slice(&rx.try_recv().expect("response frame")).unwrap()
    }
    #[test]
    fn accepted_work_completes_once_and_late_completion_is_dropped() {
        let (broker, mut rx) = broker();
        handle_line(
            &broker,
            br#"{"id":"one","type":"select_asset","message":"Assets/A"}"#,
        );
        assert_eq!(frame(&mut rx)["type"], REQUEST_ACCEPTED_EVENT);
        assert_eq!(broker.take_next_request().unwrap().id, "one");
        broker.complete("one", br#"{"reply_to":"one","ok":true}"#.to_vec());
        assert_eq!(frame(&mut rx)["reply_to"], "one");
        broker.complete("one", br#"{"reply_to":"one","ok":false}"#.to_vec());
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn reload_interrupts_queued_and_inflight_requests_but_keeps_ping_available() {
        let (broker, mut rx) = broker();
        broker
            .enqueue("inflight".into(), b"one".to_vec(), false)
            .unwrap();
        broker.take_next_request().unwrap();
        broker
            .enqueue("queued".into(), b"two".to_vec(), false)
            .unwrap();
        broker.set_managed_state(MANAGED_STATE_RELOADING, 1, None);
        assert_eq!(frame(&mut rx)["error"], "domain_reload_interrupted");
        assert_eq!(frame(&mut rx)["error"], "domain_reload_interrupted");
        handle_line(&broker, br#"{"id":"ping","type":"ping"}"#);
        assert_eq!(frame(&mut rx)["message"], "pong");
        handle_line(&broker, br#"{"id":"blocked","type":"select_asset"}"#);
        assert_eq!(frame(&mut rx)["error"], "managed_reloading");
        broker.set_managed_state(MANAGED_STATE_READY, 2, Some("editing".into()));
        broker.complete("inflight", b"stale".to_vec());
        assert!(rx.try_recv().is_err());
        assert_eq!(broker.queued_bytes.lock().unwrap().to_owned(), 0);
    }
    #[test]
    fn skipped_reload_edge_and_old_generation_cannot_resurrect_work() {
        let (broker, mut rx) = broker();
        broker
            .enqueue("old".into(), b"one".to_vec(), false)
            .unwrap();
        broker.take_next_request().unwrap();
        broker.set_managed_state(MANAGED_STATE_READY, 2, None);
        assert_eq!(frame(&mut rx)["error"], "domain_reload_interrupted");
        broker.set_managed_state(MANAGED_STATE_RELOADING, 1, None);
        broker.heartbeat(1);
        assert_eq!(broker.managed_state(), MANAGED_STATE_READY);
        assert_eq!(broker.generation.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn disconnect_discards_ordinary_work_and_retains_execution_for_reattachment() {
        let (broker, _) = broker();
        broker
            .enqueue("ordinary".into(), b"a".to_vec(), false)
            .unwrap();
        broker
            .enqueue("execution".into(), b"bb".to_vec(), true)
            .unwrap();
        broker.discard_non_reattachable_on_disconnect();
        assert_eq!(broker.queue.lock().unwrap().len(), 1);
        assert_eq!(*broker.queued_bytes.lock().unwrap(), 2);
        assert_eq!(broker.take_next_request().unwrap().id, "execution");
    }
    #[test]
    fn duplicate_ids_and_expired_requests_do_not_reexecute() {
        let (broker, mut rx) = broker();
        broker.enqueue("dup".into(), b"a".to_vec(), false).unwrap();
        assert_eq!(
            broker.enqueue("dup".into(), b"b".to_vec(), false),
            Err("native_duplicate_request_id")
        );
        broker
            .queue
            .lock()
            .unwrap()
            .front_mut()
            .unwrap()
            .deadline_ms = 0;
        assert!(broker.take_next_request().is_none());
        assert_eq!(frame(&mut rx)["error"], "native_request_timed_out");
    }
    #[test]
    fn hot_reload_is_rejected_before_managed_dispatch() {
        let (broker, mut rx) = broker();
        for kind in ["hot_reload_probe", "hot_patch_loaded", "hot_patch_dispose"] {
            handle_line(
                &broker,
                format!(r#"{{"id":"blocked","type":"{kind}"}}"#).as_bytes(),
            );
            assert_eq!(frame(&mut rx)["error"], "unsupported_on_macos");
        }
        assert!(broker.queue.lock().unwrap().is_empty());
        let (_, json) = broker.state_snapshot_json();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["backgroundPatched"], false);
        assert_eq!(value["overlayConnected"], false);
    }
    #[tokio::test]
    async fn unix_socket_reconnect_and_independent_state_survive_reload() {
        use tokio::io::AsyncBufReadExt;
        let unique = format!("locus-mac-broker-test-{}-{}", std::process::id(), now_ms());
        let project = std::env::temp_dir().join(unique);
        std::fs::create_dir(&project).unwrap();
        let project = project.to_string_lossy().into_owned();
        let endpoint = macos_ipc::endpoint(&project);
        let (listener, guard) = macos_ipc::bind_endpoint(&project, &endpoint).unwrap();
        let broker = Arc::new(Broker::new(project.clone(), endpoint.clone(), 1));
        broker.set_managed_state(MANAGED_STATE_READY, 1, Some("editing".into()));
        let server = tokio::spawn(broker_main(broker.clone(), listener, guard));
        let mut client = UnixStream::connect(&endpoint).await.unwrap();
        assert_eq!(
            macos_ipc::peer_identity(&client).unwrap(),
            std::process::id()
        );
        client
            .write_all(b"{\"id\":\"p1\",\"type\":\"ping\"}\n")
            .await
            .unwrap();
        let mut reader = BufReader::new(client);
        let mut response = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            reader.read_line(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["message"],
            "pong"
        );
        broker.set_managed_state(MANAGED_STATE_RELOADING, 1, None);
        assert_eq!(
            macos_ipc::read_snapshot(&project, &endpoint).unwrap()["managedState"],
            "reloading"
        );
        drop(reader);
        let mut client = UnixStream::connect(&endpoint).await.unwrap();
        client
            .write_all(b"{\"id\":\"p2\",\"type\":\"ping\"}\n")
            .await
            .unwrap();
        let mut reader = BufReader::new(client);
        response.clear();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            reader.read_line(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["reply_to"],
            "p2"
        );
        drop(reader);
        broker.shutdown.store(true, Ordering::SeqCst);
        broker.shutdown_notify.notify_waiters();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(!std::path::Path::new(&endpoint).exists());
        assert!(!macos_ipc::state_path(&endpoint).exists());
        std::fs::remove_file(std::path::Path::new(&endpoint).with_extension("lock")).unwrap();
        std::fs::remove_dir(project).unwrap();
    }
}
