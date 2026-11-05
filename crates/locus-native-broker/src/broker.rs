//! The broker: request queue between pipe clients (Locus) and the host's
//! managed executor (Unity C#, Blender Python), managed lifecycle tracking,
//! main-thread liveness, execution progress and cancellation registers, and
//! the state plane snapshot.
//!
//! Process-global and long-lived: the host keeps the DLL loaded across its
//! own reloads (Unity domain reload, Blender host reload), so the pipe, queue
//! and registers survive while the managed executor comes and goes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;

use locus_native_protocol::envelope::{
    OutEnvelope, REQUEST_ACCEPTED_EVENT, REQUEST_STATE_REQUEST, RESPONSE_KIND,
};
use locus_native_protocol::{errors, managed_state, state_plane as layout};

use crate::host::{HostConfig, HostRequest};
use crate::outbox::{Outbox, EVENT_FRAME_LIMIT, REPLY_BYTE_LIMIT};
use crate::state_plane::StatePlane;

pub(crate) const STATUS_EVENT_BUFFER_LIMIT: usize = 256;
/// Cancellation signals retained for managed execution monitors. Each
/// monitor only reads entries newer than the sequence it started at.
const EXECUTION_CANCEL_LOG_LIMIT: usize = 64;
pub(crate) const MAX_PENDING_REQUESTS: usize = 128;
pub(crate) const MAX_INFLIGHT_REQUESTS: usize = 64;
pub(crate) const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const REQUEST_DEADLINE_MS: i64 = 10 * 60 * 1000;
/// Execution requests carry their own run budget (up to an hour), enforced
/// by the managed executor; the broker deadline only reclaims requests whose
/// executor vanished without answering.
pub(crate) const EXECUTION_REQUEST_DEADLINE_MS: i64 = 2 * 60 * 60 * 1000;
/// Progress snapshots retained for running executions. Executions are
/// serialized by the managed executor, so a handful covers reattach races.
pub(crate) const EXECUTION_PROGRESS_LIMIT: usize = 16;
pub(crate) const EXECUTION_PROGRESS_MAX_BYTES: usize = 64 * 1024;

/// Client id of requests that did not arrive over a pipe connection.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const NO_CLIENT: u64 = 0;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// A request received from a client that must run on the managed executor.
/// `raw` is the original envelope line (no trailing newline) the managed side
/// parses back.
pub(crate) struct QueuedRequest {
    pub(crate) id: String,
    pub(crate) client: u64,
    pub(crate) raw: Vec<u8>,
    pub(crate) deadline_ms: i64,
    pub(crate) reattachable: bool,
    pub(crate) lifetime_ms: i64,
    /// A wait for an execution that is still publishing progress. Its
    /// executor is busy on the main thread on purpose, so a stalled
    /// heartbeat must not reject the wait.
    pub(crate) waits_for_execution: Option<String>,
}

pub(crate) struct InflightRequest {
    pub(crate) client: u64,
    pub(crate) deadline_ms: i64,
    pub(crate) reattachable: bool,
}

/// How the broker queues one managed request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestPolicy {
    pub(crate) reattachable: bool,
    pub(crate) lifetime_ms: i64,
    pub(crate) waits_for_execution: Option<String>,
}

impl RequestPolicy {
    pub(crate) fn ordinary(reattachable: bool) -> Self {
        Self {
            reattachable,
            lifetime_ms: REQUEST_DEADLINE_MS,
            waits_for_execution: None,
        }
    }
}

/// Last progress snapshot a running execution published from a managed
/// monitor thread. The broker answers progress polls from it, so progress,
/// printed output and liveness stay readable while the code holds the main
/// thread.
pub(crate) struct ExecutionProgress {
    pub(crate) snapshot: serde_json::Map<String, Value>,
    pub(crate) published_at_ms: i64,
    pub(crate) last_client_poll_ms: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeStatusEvent {
    pub(crate) seq: u64,
    pub(crate) kind: &'static str,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) domain_generation: i64,
    pub(crate) editor_status: String,
    pub(crate) observed_at_ms: i64,
}

pub struct Broker {
    pub(crate) config: HostConfig,

    pub(crate) shutdown: AtomicBool,
    pub(crate) shutdown_notify: Notify,

    /// Connected pipe clients (their outbound frames), keyed by connection id.
    pub(crate) clients: Mutex<HashMap<u64, Arc<Outbox>>>,
    pub(crate) next_client: AtomicU64,
    /// One of the `managed_state::*` constants.
    pub(crate) managed_state: AtomicI32,
    /// Bumped by the managed side on every reload.
    pub(crate) generation: AtomicI64,
    /// Unix-ms of the last managed heartbeat / state push.
    pub(crate) last_heartbeat_ms: AtomicI64,
    // Hot path: a pulse is only an atomic increment, never a clock read,
    // lock, allocation, JSON serialization or shared-memory publication.
    pub(crate) heartbeat_sequence: AtomicU64,
    pub(crate) heartbeat_age_ms: AtomicI64,
    /// Heartbeat sequence the observer last sampled. A newer pulse proves
    /// the host loop ran again even before the next once-per-second sample
    /// refreshes `heartbeat_age_ms`.
    pub(crate) sampled_heartbeat_sequence: AtomicU64,
    pub(crate) main_thread_id: u32,
    pub(crate) active_executions: AtomicU32,
    pub(crate) snapshot_dirty: AtomicBool,

    /// Last status string the managed side published (Unity:
    /// `editing|playing|…` + optional scene). Answers `status` directly so
    /// the bare status poll never stalls on reload.
    pub(crate) editor_status: Mutex<String>,
    /// Capability string the managed executor registered (merged with the
    /// native caps when answering `bridge_capabilities`).
    pub(crate) managed_capabilities: Mutex<String>,
    /// Host-published JSON object merged into the status snapshot as `host`
    /// (Blender: version, current file, dirty flag).
    pub(crate) host_info: Mutex<Option<Value>>,

    /// Requests waiting to be handed to the managed executor.
    pub(crate) queue: Mutex<VecDeque<QueuedRequest>>,
    /// Total bytes currently retained by `queue`.
    pub(crate) queued_bytes: Mutex<usize>,
    /// Signals `wait_request` when the queue receives work (or on shutdown).
    pub(crate) queue_signal: (Mutex<u64>, Condvar),
    /// Requests handed out via `poll` but not yet completed — used to
    /// synthesize `domain_reload_interrupted` if a reload cuts them off.
    pub(crate) inflight: Mutex<HashMap<String, InflightRequest>>,

    /// Managed lifecycle edge events retained for cursor-based consumers.
    pub(crate) event_seq: AtomicU64,
    pub(crate) events: Mutex<VecDeque<NativeStatusEvent>>,

    /// Native-owned shared-memory status surface.
    pub(crate) state_plane: Mutex<Option<StatePlane>>,
    pub(crate) state_plane_seq: AtomicU64,

    /// Cancel signals recorded off the main thread. A stuck execution cannot
    /// dequeue its own cancel request, so managed monitors poll this log.
    pub(crate) cancel_seq: AtomicU64,
    pub(crate) cancel_log: Mutex<VecDeque<(u64, String)>>,

    /// Running executions keyed by execution id (see [`ExecutionProgress`]).
    pub(crate) executions: Mutex<HashMap<String, ExecutionProgress>>,

    pub(crate) process_id: u32,
    pub(crate) process_path: String,

    /// When set, a connection must first send `{"type":"hello","message":
    /// <token>}`; everything before that is refused. Hosts that publish
    /// their pipe in a registry file set it (Blender); Unity does not.
    pub(crate) auth_token: Mutex<Option<String>>,
}

unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

impl Broker {
    pub fn new(config: HostConfig) -> Self {
        let state_plane = StatePlane::create(&config.state_plane_name);
        Self {
            config,
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            clients: Mutex::new(HashMap::new()),
            next_client: AtomicU64::new(1),
            managed_state: AtomicI32::new(managed_state::INITIALIZING),
            generation: AtomicI64::new(0),
            last_heartbeat_ms: AtomicI64::new(0),
            heartbeat_sequence: AtomicU64::new(0),
            heartbeat_age_ms: AtomicI64::new(-1),
            sampled_heartbeat_sequence: AtomicU64::new(0),
            main_thread_id: unsafe { GetCurrentThreadId() },
            active_executions: AtomicU32::new(0),
            snapshot_dirty: AtomicBool::new(true),
            editor_status: Mutex::new(String::new()),
            managed_capabilities: Mutex::new(String::new()),
            host_info: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            queued_bytes: Mutex::new(0),
            queue_signal: (Mutex::new(0), Condvar::new()),
            inflight: Mutex::new(HashMap::new()),
            event_seq: AtomicU64::new(0),
            events: Mutex::new(VecDeque::new()),
            state_plane: Mutex::new(state_plane),
            state_plane_seq: AtomicU64::new(0),
            cancel_seq: AtomicU64::new(0),
            cancel_log: Mutex::new(VecDeque::new()),
            executions: Mutex::new(HashMap::new()),
            process_id: std::process::id(),
            process_path: std::env::current_exe()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            auth_token: Mutex::new(None),
        }
    }

    pub fn set_auth_token(&self, token: &str) {
        let token = token.trim();
        if let Ok(mut guard) = self.auth_token.lock() {
            *guard = (!token.is_empty()).then(|| token.to_string());
        }
    }

    pub(crate) fn requires_hello(&self) -> bool {
        self.auth_token.lock().map(|token| token.is_some()).unwrap_or(true)
    }

    /// Check a connection's first frame. Returns whether it authenticated;
    /// answers the frame either way.
    pub(crate) fn authenticate(&self, client: u64, line: &[u8]) -> bool {
        let trimmed = locus_native_protocol::envelope::trim_frame(line);
        let Ok(value) = serde_json::from_slice::<Value>(trimmed) else {
            return false;
        };
        let id = value.get("id").and_then(Value::as_str).unwrap_or("");
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let offered = value.get("message").and_then(Value::as_str).unwrap_or("");
        let expected = self.auth_token.lock().ok().and_then(|token| token.clone());
        let accepted = kind == "hello"
            && expected
                .as_deref()
                .is_some_and(|expected| constant_time_eq(expected.as_bytes(), offered.as_bytes()));
        if !id.is_empty() {
            if accepted {
                self.respond_ok(
                    client,
                    id,
                    Some(
                        serde_json::json!({
                            "hostKind": self.config.host_kind,
                            "instanceKey": self.config.instance_key,
                            "protocolVersion": self.config.protocol_version,
                            "capabilities": self.capabilities_string().split(',').collect::<Vec<_>>(),
                            "processId": self.process_id,
                        })
                        .to_string(),
                    ),
                );
            } else {
                self.respond_error(client, id, "native_auth_required");
            }
        }
        accepted
    }

    pub fn config(&self) -> &HostConfig {
        &self.config
    }

    pub(crate) fn managed_state(&self) -> i32 {
        self.managed_state.load(Ordering::SeqCst)
    }

    // ── Clients and outbound writes ────────────────────────────────────────

    pub(crate) fn register_client(&self) -> (u64, Arc<Outbox>) {
        let client = self.next_client.fetch_add(1, Ordering::SeqCst);
        let outbox = Arc::new(Outbox::new(client));
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(client, Arc::clone(&outbox));
        }
        self.publish_status_snapshot();
        (client, outbox)
    }

    pub(crate) fn unregister_client(&self, client: u64) {
        let outbox = self.clients.lock().ok().and_then(|mut clients| clients.remove(&client));
        if let Some(outbox) = outbox {
            outbox.close();
        }
        self.discard_non_reattachable_on_disconnect(client);
    }

    pub fn connected(&self) -> bool {
        self.clients.lock().map(|clients| !clients.is_empty()).unwrap_or(false)
    }

    pub(crate) fn client_count(&self) -> usize {
        self.clients.lock().map(|clients| clients.len()).unwrap_or(0)
    }

    /// Send one reply line (newline appended here) to `client`. When that
    /// client is gone (or unknown) the line goes to every connected client:
    /// a reconnecting client still receives the answer to a reattachable
    /// execution, and clients ignore replies they did not ask for. Replies
    /// are never dropped; a client too far behind is disconnected instead.
    pub(crate) fn send_line(&self, client: u64, mut bytes: Vec<u8>) {
        bytes.push(b'\n');
        let mut overflowed = Vec::new();
        {
            let Ok(clients) = self.clients.lock() else {
                return;
            };
            if let Some(outbox) = clients.get(&client) {
                if !outbox.push_reply(bytes) {
                    overflowed.push(client);
                }
            } else {
                for (id, outbox) in clients.iter() {
                    if !outbox.push_reply(bytes.clone()) {
                        overflowed.push(*id);
                    }
                }
            }
        }
        for client in overflowed {
            self.close_client(client);
        }
    }

    /// Send an event line to every client; a client that is not reading
    /// keeps only the latest events of each type.
    pub(crate) fn broadcast_line(&self, kind: &str, mut bytes: Vec<u8>) {
        bytes.push(b'\n');
        let Ok(clients) = self.clients.lock() else {
            return;
        };
        for outbox in clients.values() {
            outbox.push_event(kind, bytes.clone());
        }
    }

    /// Disconnect a client that left more than [`REPLY_BYTE_LIMIT`] of
    /// replies unread: its requests then fail on its side and it reconnects,
    /// instead of waiting for replies the broker would have to drop.
    pub(crate) fn close_client(&self, client: u64) {
        let outbox = self.clients.lock().ok().and_then(|clients| clients.get(&client).cloned());
        if let Some(outbox) = outbox {
            if !outbox.is_closed() {
                eprintln!(
                    "[locus-native] client {client} left more than {} MiB of replies unread; disconnecting it",
                    REPLY_BYTE_LIMIT / (1024 * 1024)
                );
                outbox.close();
            }
        }
    }

    fn clients_status_json(&self, now: i64) -> Value {
        let mut clients = self
            .clients
            .lock()
            .map(|clients| clients.iter().map(|(id, outbox)| (*id, Arc::clone(outbox))).collect::<Vec<_>>())
            .unwrap_or_default();
        clients.sort_by_key(|(id, _)| *id);
        Value::Array(clients.iter().map(|(_, outbox)| outbox.status_json(now)).collect())
    }

    /// Answer `request_state`: for each request id (a JSON array in
    /// `message`) whether this client's request is still `queued` for or
    /// `inflight` on the managed executor, or `none` — the broker holds no
    /// work for it, so its reply was already sent (or the id is unknown).
    /// Replies to one connection are written in order, so a reply sent
    /// before this one has reached the client by the time this one does.
    pub(crate) fn respond_request_state(&self, client: u64, id: &str, message: &str) {
        let Ok(ids) = serde_json::from_str::<Vec<String>>(message) else {
            self.respond_error(client, id, errors::INVALID_REQUEST);
            return;
        };
        let queued = self
            .queue
            .lock()
            .map(|queue| {
                queue
                    .iter()
                    .filter(|request| request.client == client)
                    .map(|request| request.id.clone())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let inflight = self
            .inflight
            .lock()
            .map(|inflight| {
                inflight
                    .iter()
                    .filter(|(_, request)| request.client == client)
                    .map(|(id, _)| id.clone())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let states = ids
            .into_iter()
            .map(|request_id| {
                let state = if inflight.contains(&request_id) {
                    "inflight"
                } else if queued.contains(&request_id) {
                    "queued"
                } else {
                    "none"
                };
                (request_id, Value::from(state))
            })
            .collect::<serde_json::Map<_, _>>();
        self.respond_ok(client, id, Some(Value::Object(states).to_string()));
    }

    fn envelope_bytes(env: &OutEnvelope) -> Option<Vec<u8>> {
        match serde_json::to_vec(env) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                eprintln!("[locus-native] envelope serialize failed: {e}");
                None
            }
        }
    }

    pub(crate) fn respond_ok(&self, client: u64, id: &str, message: Option<String>) {
        if let Some(bytes) = Self::envelope_bytes(&OutEnvelope {
            reply_to: Some(id),
            kind: RESPONSE_KIND,
            ok: true,
            message,
            error: None,
            process_id: None,
            process_path: None,
        }) {
            self.send_line(client, bytes);
        }
    }

    pub(crate) fn respond_error(&self, client: u64, id: &str, code: &str) {
        if let Some(bytes) = Self::envelope_bytes(&OutEnvelope {
            reply_to: Some(id),
            kind: RESPONSE_KIND,
            ok: false,
            message: None,
            error: Some(code),
            process_id: None,
            process_path: None,
        }) {
            self.send_line(client, bytes);
        }
    }

    /// Error code a request gets while the managed executor cannot take it.
    pub(crate) fn unavailable_error(&self) -> Option<&'static str> {
        match self.managed_state() {
            managed_state::READY => None,
            managed_state::RELOADING => Some(errors::MANAGED_RELOADING),
            managed_state::QUITTING => Some(self.config.quitting_error),
            _ => Some(errors::MANAGED_NOT_READY),
        }
    }

    pub(crate) fn respond_status(&self, client: u64, id: &str) {
        if let Some(code) = self.unavailable_error() {
            self.respond_error(client, id, code);
            return;
        }
        let message = self
            .editor_status
            .lock()
            .ok()
            .map(|s| s.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.config.default_status.to_string());
        if let Some(bytes) = Self::envelope_bytes(&OutEnvelope {
            reply_to: Some(id),
            kind: RESPONSE_KIND,
            ok: true,
            message: Some(message),
            error: None,
            process_id: Some(self.process_id),
            process_path: if self.process_path.is_empty() {
                None
            } else {
                Some(self.process_path.clone())
            },
        }) {
            self.send_line(client, bytes);
        }
    }

    pub(crate) fn native_capabilities(&self) -> String {
        self.config.capabilities().join(",")
    }

    pub(crate) fn capabilities_string(&self) -> String {
        let native = self.native_capabilities();
        let managed = self
            .managed_capabilities
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        if managed.is_empty() {
            native
        } else {
            format!("{native},{managed}")
        }
    }

    pub(crate) fn status_json_value(
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
        let managed_capabilities = self
            .managed_capabilities
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        let capabilities = self.capabilities_string();
        let native_capabilities = self.native_capabilities();
        let mut value = serde_json::json!({
            "transport": "native_broker",
            "nativeAlive": true,
            "hostKind": self.config.host_kind,
            "instanceKey": self.config.instance_key,
            "observedAtMs": observed_at_ms,
            "managedState": managed_state::name(self.managed_state()),
            "domainGeneration": self.generation.load(Ordering::SeqCst),
            "editorStatus": self.editor_status.lock().map(|s| s.clone()).unwrap_or_default(),
            "lastManagedHeartbeatMs": self.last_heartbeat_ms.load(Ordering::SeqCst),
            "mainThreadId": self.main_thread_id,
            "mainThreadHeartbeatSeq": self.heartbeat_sequence.load(Ordering::Relaxed),
            "mainThreadHeartbeatAgeMs": match self.heartbeat_age_ms.load(Ordering::Relaxed) { age if age >= 0 => Some(age), _ => None },
            "activeExecutions": self.active_executions.load(Ordering::Relaxed),
            "executions": self.executions_status_json(observed_at_ms),
            "pendingRequests": self.queue.lock().map(|q| q.len()).unwrap_or(0),
            "pendingBytes": self.queued_bytes.lock().map(|bytes| *bytes).unwrap_or(0),
            "inflightRequests": self.inflight.lock().map(|m| m.len()).unwrap_or(0),
            "activeRequestId": active_request_id,
            "connectedClients": self.client_count(),
            "capabilities": capabilities.split(',').collect::<Vec<_>>(),
            "brokerCapabilities": native_capabilities.split(',').collect::<Vec<_>>(),
            "managedCapabilities": managed_capabilities
                .split(',')
                .filter(|capability| !capability.is_empty())
                .collect::<Vec<_>>(),
            "protocolVersion": self.config.protocol_version,
            "pipeName": self.config.pipe_name,
            "project": self.config.project,
            "processId": self.process_id,
            "processPath": self.process_path,
            "queueLimit": MAX_PENDING_REQUESTS,
            "inflightLimit": MAX_INFLIGHT_REQUESTS,
            "payloadLimitBytes": MAX_REQUEST_BYTES,
            "pendingByteLimit": MAX_PENDING_BYTES,
            "eventQueueLimit": EVENT_FRAME_LIMIT,
            "replyBacklogLimitBytes": REPLY_BYTE_LIMIT,
            "clients": self.clients_status_json(observed_at_ms),
            "requestDeadlineMs": REQUEST_DEADLINE_MS,
            "events": events,
            "cursor": cursor,
        });
        if let Value::Object(map) = &mut value {
            if let Some(host) = self.host_info.lock().ok().and_then(|info| info.clone()) {
                map.insert("host".to_string(), host);
            }
            if let Some(extras) = self.config.status_extras {
                extras(map);
            }
        }
        value
    }

    pub(crate) fn shared_memory_status_json(&self) -> (i64, String) {
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
            if json.len() <= layout::MAX_PAYLOAD || events.is_empty() {
                return (observed_at_ms, json);
            }

            let drop_count = ((events.len() + 1) / 2).max(1);
            events.drain(0..drop_count.min(events.len()));
        }
    }

    pub(crate) fn publish_status_snapshot(&self) {
        // Called from both managed FFI and the broker. Publication must
        // never serialize or acquire the state-plane lock on the host thread.
        self.snapshot_dirty.store(true, Ordering::Relaxed);
    }

    pub(crate) fn flush_status_snapshot(&self) {
        let (observed_at_ms, json) = self.shared_memory_status_json();
        if let Ok(mut guard) = self.state_plane.lock() {
            if let Some(plane) = guard.as_mut() {
                let seq = self.state_plane_seq.fetch_add(1, Ordering::SeqCst) + 1;
                plane.write_json(seq, observed_at_ms, &json);
            }
        }
    }

    fn event_bytes(event_type: &str, payload: String) -> Option<Vec<u8>> {
        Self::envelope_bytes(&OutEnvelope {
            reply_to: None,
            kind: event_type,
            ok: true,
            message: Some(payload),
            error: None,
            process_id: None,
            process_path: None,
        })
    }

    /// Emit an unsolicited event (no `reply_to`) to every connected client.
    pub fn emit_event(&self, event_type: &str, payload: String) {
        if let Some(bytes) = Self::event_bytes(event_type, payload) {
            self.broadcast_line(event_type, bytes);
        }
    }

    /// Emit an event to one client (falls back to all when it is gone).
    pub(crate) fn emit_event_to(&self, client: u64, event_type: &str, payload: String) {
        if let Some(bytes) = Self::event_bytes(event_type, payload) {
            self.send_line(client, bytes);
        }
    }

    // ── Request queue ──────────────────────────────────────────────────────

    pub(crate) fn expire_requests(&self) {
        let now = now_ms();
        let mut expired: Vec<(u64, String)> = Vec::new();

        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
            let mut retained = VecDeque::with_capacity(q.len());
            while let Some(req) = q.pop_front() {
                if req.deadline_ms <= now {
                    *queued_bytes = queued_bytes.saturating_sub(req.raw.len());
                    expired.push((req.client, req.id));
                } else {
                    retained.push_back(req);
                }
            }
            *q = retained;
        }

        if let Ok(mut inflight) = self.inflight.lock() {
            let ids = inflight
                .iter()
                .filter(|(_, req)| req.deadline_ms <= now)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in ids {
                if let Some(req) = inflight.remove(&id) {
                    expired.push((req.client, id));
                }
            }
        }

        for (client, id) in expired {
            self.respond_error(client, &id, errors::REQUEST_TIMED_OUT);
        }
    }

    /// A request id already waiting or running for another connection. Two
    /// Locus processes may use the same counter-based ids; the managed side
    /// completes by id, so such a request must not be accepted.
    fn id_in_use_by_other_client(&self, id: &str, client: u64) -> bool {
        let queued = self
            .queue
            .lock()
            .map(|q| q.iter().any(|req| req.id == id && req.client != client))
            .unwrap_or(false);
        queued
            || self
                .inflight
                .lock()
                .map(|inflight| inflight.get(id).is_some_and(|req| req.client != client))
                .unwrap_or(false)
    }

    pub(crate) fn enqueue_with(
        &self,
        client: u64,
        id: String,
        raw: Vec<u8>,
        policy: RequestPolicy,
    ) -> Result<(), &'static str> {
        self.expire_requests();

        if self.main_thread_unresponsive() && !self.waits_for_live_execution(&policy) {
            return Err(errors::MAIN_THREAD_UNRESPONSIVE);
        }

        if raw.len() > MAX_REQUEST_BYTES {
            return Err(errors::PAYLOAD_TOO_LARGE);
        }
        if self
            .inflight
            .lock()
            .map(|inflight| inflight.len() >= MAX_INFLIGHT_REQUESTS)
            .unwrap_or(true)
        {
            return Err(errors::INFLIGHT_FULL);
        }
        if self.id_in_use_by_other_client(&id, client) {
            return Err("native_request_id_conflict");
        }

        let deadline_ms = now_ms().saturating_add(policy.lifetime_ms);
        {
            let mut q = self.queue.lock().map_err(|_| errors::QUEUE_FULL)?;
            let mut queued_bytes = self.queued_bytes.lock().map_err(|_| errors::QUEUE_FULL)?;
            if q.len() >= MAX_PENDING_REQUESTS
                || queued_bytes.saturating_add(raw.len()) > MAX_PENDING_BYTES
            {
                return Err(errors::QUEUE_FULL);
            }
            *queued_bytes = queued_bytes.saturating_add(raw.len());
            q.push_back(QueuedRequest {
                id,
                client,
                raw,
                deadline_ms,
                reattachable: policy.reattachable,
                lifetime_ms: policy.lifetime_ms,
                waits_for_execution: policy.waits_for_execution,
            });
        }
        self.signal_queue();
        self.publish_status_snapshot();
        Ok(())
    }

    fn signal_queue(&self) {
        let (lock, condvar) = &self.queue_signal;
        if let Ok(mut generation) = lock.lock() {
            *generation = generation.wrapping_add(1);
        }
        condvar.notify_all();
    }

    pub(crate) fn has_pending_request(&self) -> bool {
        self.queue.lock().map(|q| !q.is_empty()).unwrap_or(false)
    }

    /// Block the calling (host) thread until a request is queued, the broker
    /// shuts down, or `timeout` elapses. Returns whether a request is ready.
    /// Background hosts (Blender `-b`) loop on this instead of polling.
    pub fn wait_request(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (lock, condvar) = &self.queue_signal;
        let Ok(mut guard) = lock.lock() else {
            return false;
        };
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                return false;
            }
            if self.has_pending_request() && self.next_request_len().is_some() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match condvar.wait_timeout(guard, deadline - now) {
                Ok((next, _)) => guard = next,
                Err(_) => return false,
            }
        }
    }

    pub(crate) fn next_request_len(&self) -> Option<usize> {
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

    pub(crate) fn take_next_request(&self) -> Option<QueuedRequest> {
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
                    client: req.client,
                    deadline_ms: now_ms().saturating_add(req.lifetime_ms),
                    reattachable: req.reattachable,
                },
            );
        }
        self.publish_status_snapshot();
        Some(req)
    }

    pub(crate) fn requeue_front(&self, req: QueuedRequest) {
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.remove(&req.id);
        }
        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
            *queued_bytes = queued_bytes.saturating_add(req.raw.len());
            q.push_front(req);
        }
        self.publish_status_snapshot();
    }

    pub fn complete(&self, id: &str, response: Vec<u8>) {
        let removed = self
            .inflight
            .lock()
            .map(|mut inflight| inflight.remove(id))
            .unwrap_or(None);
        if let Some(request) = removed {
            self.send_line(request.client, response);
            self.publish_status_snapshot();
        } else {
            eprintln!("[locus-native] dropping stale completion for non-inflight request: {id}");
        }
    }

    /// A reload is starting: every request the managed side will never get
    /// to (queued or already handed out) gets a definite error so the client
    /// retries instead of hanging.
    pub(crate) fn interrupt_for_reload(&self) {
        let mut ids: Vec<(u64, String)> = Vec::new();
        if let Ok(mut q) = self.queue.lock() {
            while let Some(req) = q.pop_front() {
                ids.push((req.client, req.id));
            }
        }
        if let Ok(mut queued_bytes) = self.queued_bytes.lock() {
            *queued_bytes = 0;
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            for (id, req) in inflight.drain() {
                ids.push((req.client, id));
            }
        }
        for (client, id) in ids {
            self.respond_error(client, &id, errors::RELOAD_INTERRUPTED);
        }
        self.publish_status_snapshot();
    }

    /// Preserve only this client's execution requests that can be reattached
    /// by execution id. Every other request it sent belongs to the closed
    /// transport and must not execute later for a new client.
    pub(crate) fn discard_non_reattachable_on_disconnect(&self, client: u64) {
        if let (Ok(mut q), Ok(mut queued_bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
            q.retain(|request| request.client != client || request.reattachable);
            *queued_bytes = q.iter().map(|request| request.raw.len()).sum();
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.retain(|_, request| request.client != client || request.reattachable);
        }
        self.publish_status_snapshot();
    }

    // ── Managed lifecycle ──────────────────────────────────────────────────

    pub fn set_managed_state(&self, state: i32, generation: i64, editor_status: Option<String>) {
        let previous = self.managed_state.swap(state, Ordering::SeqCst);
        let observed_at_ms = now_ms();
        if generation > 0 && self.generation.swap(generation, Ordering::SeqCst) != generation {
            self.active_executions.store(0, Ordering::Relaxed);
            self.heartbeat_age_ms.store(-1, Ordering::Relaxed);
            self.clear_executions();
        }
        self.last_heartbeat_ms.store(observed_at_ms, Ordering::SeqCst);
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
                managed_state::name(previous),
                managed_state::name(state),
                self.generation.load(Ordering::SeqCst),
                editor_status,
                observed_at_ms,
            );
        }
        if state == managed_state::RELOADING || state == managed_state::QUITTING {
            if let Ok(mut guard) = self.managed_capabilities.lock() {
                guard.clear();
            }
        }
        // Entering a reload (or quit) strands any in-flight work.
        if state != previous
            && (state == managed_state::RELOADING || state == managed_state::QUITTING)
        {
            self.clear_executions();
            self.interrupt_for_reload();
        }
        self.publish_status_snapshot();
    }

    pub fn heartbeat(&self, _generation: i64) {
        self.heartbeat_sequence.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn main_thread_unresponsive(&self) -> bool {
        self.managed_state() == managed_state::READY
            && self.heartbeat_age_ms.load(Ordering::Relaxed)
                >= crate::heartbeat::UNRESPONSIVE_AFTER.as_millis() as i64
            && !self.pulsed_since_sample()
    }

    /// The main thread pulsed after the observer's last sample, so a stall
    /// that sample measured has already ended.
    fn pulsed_since_sample(&self) -> bool {
        self.heartbeat_sequence.load(Ordering::Relaxed)
            != self.sampled_heartbeat_sequence.load(Ordering::Relaxed)
    }

    /// Milliseconds since the host loop last pulsed, or `-1` when the
    /// managed executor is not registered or no pulse has been observed in
    /// this generation.
    pub fn main_thread_stall_ms(&self) -> i64 {
        if self.managed_state() != managed_state::READY {
            return -1;
        }
        let age = self.heartbeat_age_ms.load(Ordering::Relaxed);
        if age > 0 && self.pulsed_since_sample() {
            0
        } else {
            age
        }
    }

    // ── Execution cancellation signals ─────────────────────────────────────

    /// Record a cancel for `execution_id` (empty = every active execution).
    /// Readers only see entries newer than their own start.
    pub(crate) fn signal_execution_cancel(&self, execution_id: &str) -> u64 {
        let seq = self.cancel_seq.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut log) = self.cancel_log.lock() {
            if log.len() >= EXECUTION_CANCEL_LOG_LIMIT {
                log.pop_front();
            }
            log.push_back((seq, execution_id.trim().to_string()));
        }
        seq
    }

    /// A cancel naming this execution always applies (ids are unique, and
    /// the cancel may arrive before the managed monitor starts). A cancel for
    /// every execution only applies to monitors older than it.
    pub fn execution_cancel_requested(&self, execution_id: &str, since_seq: u64) -> bool {
        let execution_id = execution_id.trim();
        self.cancel_log
            .lock()
            .map(|log| {
                log.iter().any(|(seq, target)| {
                    if target.is_empty() {
                        *seq > since_seq
                    } else {
                        !execution_id.is_empty() && target == execution_id
                    }
                })
            })
            .unwrap_or(false)
    }

    pub fn execution_activity(&self, active: bool) {
        if active {
            self.active_executions.fetch_add(1, Ordering::Relaxed);
        } else {
            let _ = self.active_executions.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
        }
    }

    pub(crate) fn reject_stalled_queue(&self) {
        let rejected = if let (Ok(mut queue), Ok(mut bytes)) = (self.queue.lock(), self.queued_bytes.lock()) {
            let mut kept = VecDeque::new();
            let mut rejected = Vec::new();
            for request in queue.drain(..) {
                let keep = request
                    .waits_for_execution
                    .as_deref()
                    .is_some_and(|execution_id| self.has_execution(execution_id));
                if keep {
                    kept.push_back(request);
                } else {
                    rejected.push((request.client, request.id));
                }
            }
            *bytes = kept.iter().map(|request| request.raw.len()).sum();
            *queue = kept;
            rejected
        } else {
            Vec::new()
        };
        for (client, id) in rejected {
            self.respond_error(client, &id, errors::MAIN_THREAD_UNRESPONSIVE);
        }
    }

    // ── Execution progress register ────────────────────────────────────────

    /// Record the latest snapshot an execution published. Called from a
    /// managed monitor thread a few times per second; parsing happens before
    /// the lock so readers never wait on it.
    pub fn publish_execution(&self, execution_id: &str, json: &str) {
        let execution_id = execution_id.trim();
        if execution_id.is_empty() || json.len() > EXECUTION_PROGRESS_MAX_BYTES {
            return;
        }
        let Ok(Value::Object(snapshot)) = serde_json::from_str::<Value>(json) else {
            return;
        };
        let now = now_ms();
        if let Ok(mut executions) = self.executions.lock() {
            if !executions.contains_key(execution_id) && executions.len() >= EXECUTION_PROGRESS_LIMIT {
                if let Some(oldest) = executions
                    .iter()
                    .min_by_key(|(_, record)| record.published_at_ms)
                    .map(|(id, _)| id.clone())
                {
                    executions.remove(&oldest);
                }
            }
            let last_client_poll_ms = executions
                .get(execution_id)
                .map(|record| record.last_client_poll_ms)
                .unwrap_or(0);
            executions.insert(
                execution_id.to_string(),
                ExecutionProgress {
                    snapshot,
                    published_at_ms: now,
                    last_client_poll_ms,
                },
            );
        }
    }

    pub fn end_execution(&self, execution_id: &str) {
        if let Ok(mut executions) = self.executions.lock() {
            executions.remove(execution_id.trim());
        }
    }

    pub(crate) fn clear_executions(&self) {
        if let Ok(mut executions) = self.executions.lock() {
            executions.clear();
        }
    }

    pub(crate) fn has_execution(&self, execution_id: &str) -> bool {
        let execution_id = execution_id.trim();
        !execution_id.is_empty()
            && self
                .executions
                .lock()
                .map(|executions| executions.contains_key(execution_id))
                .unwrap_or(false)
    }

    fn waits_for_live_execution(&self, policy: &RequestPolicy) -> bool {
        policy
            .waits_for_execution
            .as_deref()
            .is_some_and(|execution_id| self.has_execution(execution_id))
    }

    /// Answer a progress poll from the register. Returns `false` when the
    /// execution never published (older managed side, or not started yet) so
    /// the caller keeps the managed path.
    pub(crate) fn respond_execution_progress(&self, client: u64, request_id: &str, execution_id: &str) -> bool {
        let now = now_ms();
        let snapshot = {
            let Ok(mut executions) = self.executions.lock() else {
                return false;
            };
            let Some(record) = executions.get_mut(execution_id.trim()) else {
                return false;
            };
            record.last_client_poll_ms = now;
            let mut snapshot = record.snapshot.clone();
            snapshot.insert(
                "publishedAgeMs".to_string(),
                Value::from(now.saturating_sub(record.published_at_ms).max(0)),
            );
            snapshot
        };
        let mut snapshot = snapshot;
        snapshot.insert("mainThreadStallMs".to_string(), Value::from(self.main_thread_stall_ms()));
        snapshot.insert("servedBy".to_string(), Value::from("native_broker"));
        self.respond_ok(client, request_id, Some(Value::Object(snapshot).to_string()));
        true
    }

    /// Milliseconds since a client last polled this execution's progress
    /// through the broker, or `-1` when none did (or it is unknown).
    pub fn execution_client_poll_age_ms(&self, execution_id: &str) -> i64 {
        self.executions
            .lock()
            .ok()
            .and_then(|executions| {
                executions
                    .get(execution_id.trim())
                    .filter(|record| record.last_client_poll_ms > 0)
                    .map(|record| now_ms().saturating_sub(record.last_client_poll_ms).max(0))
            })
            .unwrap_or(-1)
    }

    fn executions_status_json(&self, now: i64) -> Vec<Value> {
        let Ok(executions) = self.executions.lock() else {
            return Vec::new();
        };
        let mut list = executions
            .iter()
            .map(|(execution_id, record)| {
                let field = |name: &str| record.snapshot.get(name).cloned().unwrap_or(Value::Null);
                serde_json::json!({
                    "executionId": execution_id,
                    "publishedAgeMs": now.saturating_sub(record.published_at_ms).max(0),
                    "idleMs": field("idleMs"),
                    "idleLimitMs": field("idleLimitMs"),
                    "mainThreadBlockedMs": field("mainThreadBlockedMs"),
                    "stopRequested": field("stopRequested"),
                })
            })
            .collect::<Vec<_>>();
        list.sort_by(|a, b| {
            a["executionId"].as_str().unwrap_or("").cmp(b["executionId"].as_str().unwrap_or(""))
        });
        list
    }

    pub(crate) async fn observe_main_thread(self: Arc<Self>) {
        let mut observer = crate::heartbeat::HeartbeatObserver::new(Instant::now());
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = self.shutdown_notify.notified() => break,
                _ = interval.tick() => {}
            }
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            let sequence = self.heartbeat_sequence.load(Ordering::Relaxed);
            let age = observer.sample(sequence, self.generation.load(Ordering::Relaxed), Instant::now());
            self.heartbeat_age_ms.store(
                age.map(|age| age.as_millis().min(i64::MAX as u128) as i64).unwrap_or(-1),
                Ordering::Relaxed,
            );
            self.sampled_heartbeat_sequence.store(sequence, Ordering::Relaxed);
            if age.is_some_and(|age| age.is_zero()) {
                self.last_heartbeat_ms.store(now_ms(), Ordering::Relaxed);
            }
            if self.main_thread_unresponsive() {
                self.reject_stalled_queue();
            }
            // Publish once per second even when stalled so readers can
            // distinguish a live broker from a stale mapped snapshot.
            if self.snapshot_dirty.swap(false, Ordering::Relaxed) || age.is_some() {
                self.flush_status_snapshot();
            }
        }
    }

    /// Answer `main_thread_stack` from a dedicated thread: the capture sleeps
    /// between samples and may wait on the managed runtime, and the broker
    /// runtime must keep serving the pipe and the heartbeat observer.
    pub(crate) fn capture_main_thread_stack(self: &Arc<Self>, client: u64, id: &str, message: &str) {
        let mut request = Some(message)
            .filter(|message| !message.trim().is_empty())
            .and_then(|message| serde_json::from_str::<crate::hang_stack::CaptureRequest>(message).ok())
            .unwrap_or_default();
        if self.managed_state() != managed_state::READY {
            // The managed runtime is tearing down or rebuilding; naming
            // frames then could race the unload. Native frames are useful.
            request.resolve_managed = Some(false);
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let broker = Arc::clone(self);
        let request_id = id.to_string();
        let thread_id = self.main_thread_id;
        let namer = self.config.managed_namer;
        let spawned = std::thread::Builder::new()
            .name("locus-hang-capture".to_string())
            .spawn(move || match crate::hang_stack::capture(thread_id, &request, generation, namer) {
                Ok(result) => match serde_json::to_string(&result) {
                    Ok(json) => broker.respond_ok(client, &request_id, Some(json)),
                    Err(error) => broker.respond_error(
                        client,
                        &request_id,
                        &format!("{}: serialize: {error}", errors::STACK_FAILED),
                    ),
                },
                Err(error) => {
                    broker.respond_error(client, &request_id, &format!("{}: {error}", errors::STACK_FAILED))
                }
            });
        if spawned.is_err() {
            self.respond_error(
                client,
                id,
                &format!("{}: capture thread could not start", errors::STACK_FAILED),
            );
        }
    }

    pub fn set_capabilities(&self, caps: String) {
        if let Ok(mut guard) = self.managed_capabilities.lock() {
            *guard = caps;
        }
        self.publish_status_snapshot();
    }

    /// Host fields for the status snapshot (`host`). Invalid JSON clears them.
    pub fn set_host_info(&self, json: &str) {
        let value = serde_json::from_str::<Value>(json).ok().filter(Value::is_object);
        if let Ok(mut guard) = self.host_info.lock() {
            *guard = value;
        }
        self.publish_status_snapshot();
    }

    pub(crate) fn push_status_event(
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

    pub fn shutdown(&self) {
        if let Some(on_shutdown) = self.config.on_shutdown {
            on_shutdown();
        }
        self.shutdown.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
        self.signal_queue();
        self.set_managed_state(managed_state::QUITTING, 0, None);
    }
}

#[cfg(test)]
pub(crate) fn constant_time_eq_for_test(a: &[u8], b: &[u8]) -> bool {
    constant_time_eq(a, b)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (index, byte) in a.iter().enumerate() {
        diff |= usize::from(byte ^ b.get(index).copied().unwrap_or(0));
    }
    diff == 0
}

// ── Inbound dispatch ────────────────────────────────────────────────────────

/// Dispatch one inbound envelope line from `client`: answer the cheap and
/// liveness commands directly (they must work during a reload), let the host
/// hook answer its own registers, queue everything else for the managed
/// executor, or fail fast when the executor cannot serve it.
pub(crate) fn handle_line(broker: &Arc<Broker>, client: u64, line: &[u8]) {
    let trimmed = locus_native_protocol::envelope::trim_frame(line);
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

    let message = value.get("message").and_then(Value::as_str).unwrap_or("");
    let execution = broker.config.execution;
    match kind {
        "ping" => broker.respond_ok(client, id, Some("pong".to_string())),
        "status" => broker.respond_status(client, id),
        "bridge_capabilities" => broker.respond_ok(client, id, Some(broker.capabilities_string())),
        REQUEST_STATE_REQUEST => broker.respond_request_state(client, id, message),
        // Diagnosis of a wedged main thread must not depend on it.
        "main_thread_stack" => broker.capture_main_thread_stack(client, id, message),
        _ if kind == execution.cancel => {
            // Signal first: an execution stuck on the main thread can never
            // dequeue this request, but its monitor thread polls the signal.
            // The managed handler still runs when it can.
            broker.signal_execution_cancel(message);
            if route_to_managed(broker, client, id, kind, message, trimmed).is_err() {
                broker.respond_ok(
                    client,
                    id,
                    Some("execution cancellation signaled through the native broker".to_string()),
                );
            }
        }
        // A running execution publishes its progress from a monitor thread;
        // answering here keeps progress, output and liveness readable while
        // the code holds the main thread. Executions that never published
        // keep the managed path.
        _ if kind == execution.progress && broker.respond_execution_progress(client, id, message) => {}
        _ => {
            if let Some(hook) = broker.config.request_hook {
                let request = HostRequest {
                    id,
                    kind,
                    message,
                    raw: trimmed,
                };
                if let Some(reply) = hook(&request) {
                    match reply {
                        Ok(message) => broker.respond_ok(client, id, message),
                        Err(code) => broker.respond_error(client, id, &code),
                    }
                    return;
                }
            }
            if let Err(code) = route_to_managed(broker, client, id, kind, message, trimmed) {
                broker.respond_error(client, id, code);
            }
        }
    }
}

pub(crate) fn request_policy(broker: &Broker, kind: &str, message: &str) -> RequestPolicy {
    let execution = broker.config.execution;
    if broker.config.is_execute(kind) {
        RequestPolicy {
            reattachable: true,
            lifetime_ms: EXECUTION_REQUEST_DEADLINE_MS,
            waits_for_execution: None,
        }
    } else if kind == execution.wait {
        RequestPolicy {
            reattachable: false,
            lifetime_ms: EXECUTION_REQUEST_DEADLINE_MS,
            waits_for_execution: Some(message.trim().to_string())
                .filter(|execution_id| broker.has_execution(execution_id)),
        }
    } else {
        RequestPolicy::ordinary(false)
    }
}

/// Queue a request for the managed executor, or return why it cannot take it.
pub(crate) fn route_to_managed(
    broker: &Arc<Broker>,
    client: u64,
    id: &str,
    kind: &str,
    message: &str,
    raw: &[u8],
) -> Result<(), &'static str> {
    if let Some(code) = broker.unavailable_error() {
        return Err(code);
    }
    broker.enqueue_with(client, id.to_string(), raw.to_vec(), request_policy(broker, kind, message))?;
    broker.emit_event_to(
        client,
        REQUEST_ACCEPTED_EVENT,
        serde_json::json!({
            "requestId": id,
            "requestType": kind,
        })
        .to_string(),
    );
    Ok(())
}
