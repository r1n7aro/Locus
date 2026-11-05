//! App-level Blender instance hub. It exists only while at least one Blender
//! workspace service runs: the first `acquire` starts it, dropping the last
//! [`HubHandle`] stops it, so a disabled or unused integration costs nothing.
//!
//! The hub watches the registration directory (event driven), validates each
//! file (PID + process start time), keeps one pipe connection per instance
//! (hello + token, reconnect with backoff, ping every 10 s), routes responses
//! to waiting requests and output events to running executions, and reads the
//! broker's state plane for liveness without a round trip.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::workspace_service::CheckoutId;
use locus_native_protocol::envelope::PipeEnvelope;

pub const PING_INTERVAL: Duration = Duration::from_secs(10);
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(10);
/// Main-thread silence after which an interactive instance counts as stuck.
pub const MAIN_THREAD_STUCK_MS: i64 = 30_000;

#[cfg(windows)]
type PipeClient = tokio::net::windows::named_pipe::NamedPipeClient;
/// Placeholder so the hub compiles elsewhere; Blender integration is
/// Windows-only in the first release (`open_pipe` always fails).
#[cfg(not(windows))]
type PipeClient = tokio::io::DuplexStream;

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn attach_app_handle(app: tauri::AppHandle) {
    let _ = APP_HANDLE.set(app);
}

pub fn app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

fn next_request_id() -> String {
    format!("loc-{}-{}", std::process::id(), REQUEST_SEQ.fetch_add(1, Ordering::Relaxed))
}

// ── registration ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BlenderBuild {
    pub version: String,
    pub version_string: String,
    pub build_hash: String,
    pub build_branch: String,
    pub binary: String,
}

/// Contents of `<pid>-<start ms>.json` written by the host add-on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Registration {
    pub pid: u32,
    pub process_start_ms: u64,
    pub instance_key: Option<String>,
    pub pipe: Option<String>,
    pub token: Option<String>,
    pub protocol: u32,
    pub native_protocol: u32,
    pub native_dll: Option<String>,
    pub host_hash: String,
    pub host_dir: String,
    pub mode: String,
    pub launched_by: Option<Value>,
    pub blender: BlenderBuild,
    pub python: String,
    pub file: Option<String>,
    pub dirty: bool,
    pub saved_version: Option<String>,
    pub started_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
    pub state: String,
    pub error: Option<String>,
    pub capabilities: Vec<String>,
}

impl Registration {
    pub fn is_background(&self) -> bool {
        self.mode == "background"
    }

    /// `launched_by.checkout` when Locus launched this instance.
    pub fn launched_by_checkout(&self) -> Option<String> {
        self.launched_by
            .as_ref()
            .and_then(|value| value.get("checkout"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

fn read_registration(path: &Path) -> Option<Registration> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ── instances ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    /// The host could not start (e.g. the native layer failed to load).
    HostError,
}

#[derive(Debug, Clone)]
pub struct OutputChunk {
    pub stream: String,
    pub text: String,
}

pub struct Instance {
    pub key: String,
    registration: Mutex<Registration>,
    state: Mutex<ConnectionState>,
    last_error: Mutex<Option<String>>,
    writer: tokio::sync::Mutex<Option<tokio::io::WriteHalf<PipeClient>>>,
    pending: Mutex<HashMap<String, oneshot::Sender<PipeEnvelope>>>,
    accepted: Mutex<HashMap<String, oneshot::Sender<()>>>,
    outputs: Mutex<HashMap<String, mpsc::UnboundedSender<OutputChunk>>>,
    stopped: AtomicBool,
    /// File state came from a pipe event (newer than the registration file,
    /// whose rewrite can lag or fail) since the connection opened.
    file_from_pipe: AtomicBool,
    /// Executions currently running through this hub (lease against reaping).
    pub busy: AtomicUsize,
    last_used: Mutex<Instant>,
    connected_at: Mutex<Option<Instant>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSnapshot {
    pub key: String,
    pub pid: u32,
    pub mode: String,
    pub version: String,
    pub version_string: String,
    pub build_branch: String,
    pub binary: String,
    pub python: String,
    pub file: Option<String>,
    pub dirty: bool,
    pub saved_version: Option<String>,
    pub host_hash: String,
    pub connection: ConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub launched_by_checkout: Option<String>,
    pub busy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_thread_stall_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_requests: Option<u64>,
    pub capabilities: Vec<String>,
}

impl Instance {
    fn new(key: String, registration: Registration) -> Arc<Self> {
        let state = if registration.state == "error" || registration.pipe.is_none() {
            ConnectionState::HostError
        } else {
            ConnectionState::Connecting
        };
        let error = registration.error.clone();
        Arc::new(Self {
            key,
            registration: Mutex::new(registration),
            state: Mutex::new(state),
            last_error: Mutex::new(error),
            writer: tokio::sync::Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            accepted: Mutex::new(HashMap::new()),
            outputs: Mutex::new(HashMap::new()),
            stopped: AtomicBool::new(false),
            file_from_pipe: AtomicBool::new(false),
            busy: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
            connected_at: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(key: &str, registration: Registration) -> Arc<Self> {
        Self::new(key.to_string(), registration)
    }

    pub fn registration(&self) -> Registration {
        self.registration.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn connection(&self) -> ConnectionState {
        self.state.lock().map(|s| *s).unwrap_or(ConnectionState::Disconnected)
    }

    fn set_state(&self, state: ConnectionState) {
        if let Ok(mut current) = self.state.lock() {
            *current = state;
        }
    }

    fn set_error(&self, error: Option<String>) {
        if let Ok(mut current) = self.last_error.lock() {
            *current = error;
        }
    }

    pub fn touch(&self) {
        if let Ok(mut last) = self.last_used.lock() {
            *last = Instant::now();
        }
    }

    pub fn idle_for(&self) -> Duration {
        self.last_used.lock().map(|last| last.elapsed()).unwrap_or_default()
    }

    pub fn snapshot(&self) -> InstanceSnapshot {
        let registration = self.registration();
        let plane = read_state_plane(&self.key);
        InstanceSnapshot {
            key: self.key.clone(),
            pid: registration.pid,
            mode: registration.mode.clone(),
            version: registration.blender.version.clone(),
            version_string: registration.blender.version_string.clone(),
            build_branch: registration.blender.build_branch.clone(),
            binary: registration.blender.binary.clone(),
            python: registration.python.clone(),
            file: registration.file.clone(),
            dirty: registration.dirty,
            saved_version: registration.saved_version.clone(),
            host_hash: registration.host_hash.clone(),
            connection: self.connection(),
            error: self.last_error.lock().ok().and_then(|e| e.clone()),
            launched_by_checkout: registration.launched_by_checkout(),
            busy: self.busy.load(Ordering::Relaxed) > 0
                || plane.as_ref().and_then(|p| p.get("activeExecutions")).and_then(Value::as_u64).unwrap_or(0) > 0,
            main_thread_stall_ms: plane
                .as_ref()
                .and_then(|p| p.get("mainThreadHeartbeatAgeMs"))
                .and_then(Value::as_i64),
            pending_requests: plane.as_ref().and_then(|p| p.get("pendingRequests")).and_then(Value::as_u64),
            capabilities: registration.capabilities.clone(),
        }
    }

    fn update_file_state(&self, event: &Value) {
        self.file_from_pipe.store(true, Ordering::SeqCst);
        if let Ok(mut registration) = self.registration.lock() {
            if let Some(file) = event.get("file") {
                registration.file = file.as_str().map(str::to_string);
            }
            if let Some(dirty) = event.get("dirty").and_then(Value::as_bool) {
                registration.dirty = dirty;
            }
            if let Some(saved) = event.get("saved_version") {
                registration.saved_version = saved.as_str().map(str::to_string);
            }
        }
    }

    // ── requests ───────────────────────────────────────────────────────────

    async fn write_line(&self, line: String) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut guard = self.writer.lock().await;
        let writer = guard.as_mut().ok_or_else(|| "instance is not connected".to_string())?;
        writer.write_all(line.as_bytes()).await.map_err(|e| format!("pipe write failed: {e}"))?;
        writer.write_all(b"\n").await.map_err(|e| format!("pipe write failed: {e}"))?;
        writer.flush().await.map_err(|e| format!("pipe flush failed: {e}"))
    }

    async fn send(&self, id: &str, kind: &str, message: &str) -> Result<oneshot::Receiver<PipeEnvelope>, String> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id.to_string(), tx);
        }
        let line = serde_json::to_string(&PipeEnvelope {
            id: Some(id.to_string()),
            kind: kind.to_string(),
            message: Some(message.to_string()),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        if let Err(error) = self.write_line(line).await {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(id);
            }
            return Err(error);
        }
        Ok(rx)
    }

    /// One request/response round trip.
    pub async fn request(&self, kind: &str, message: &str, timeout: Duration) -> Result<PipeEnvelope, String> {
        if self.connection() != ConnectionState::Connected {
            return Err(format!("Blender instance {} is not connected", self.key));
        }
        let id = next_request_id();
        let rx = self.send(&id, kind, message).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(envelope)) => Ok(envelope),
            Ok(Err(_)) => Err("connection closed before the response".to_string()),
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                Err(format!("no response within {} ms", timeout.as_millis()))
            }
        }
    }

    /// Run an `exec` payload, streaming output to `on_output`.
    pub async fn exec(
        self: &Arc<Self>,
        payload: &Value,
        timeout: Duration,
        mut on_output: impl FnMut(OutputChunk) + Send,
        cancel: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> ExecOutcome {
        if self.connection() != ConnectionState::Connected {
            return ExecOutcome::NotExecuted(format!("Blender instance {} is not connected", self.key));
        }
        self.busy.fetch_add(1, Ordering::SeqCst);
        self.touch();
        let _busy = BusyGuard(Arc::clone(self));
        let id = next_request_id();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (accepted_tx, mut accepted_rx) = oneshot::channel();
        if let Ok(mut outputs) = self.outputs.lock() {
            outputs.insert(id.clone(), out_tx);
        }
        if let Ok(mut accepted) = self.accepted.lock() {
            accepted.insert(id.clone(), accepted_tx);
        }
        let message = payload.to_string();
        let rx = match self.send(&id, "exec", &message).await {
            Ok(rx) => rx,
            Err(error) => {
                self.forget(&id);
                return ExecOutcome::NotExecuted(error);
            }
        };
        let deadline = tokio::time::Instant::now() + timeout;
        let mut rx = rx;
        let mut cancel = cancel;
        let mut was_accepted = false;
        let outcome = loop {
            tokio::select! {
                response = &mut rx => {
                    break match response {
                        Ok(envelope) => envelope_outcome(envelope),
                        Err(_) => ExecOutcome::Unknown("connection closed during the execution".to_string()),
                    };
                }
                Some(chunk) = out_rx.recv() => on_output(chunk),
                _ = &mut accepted_rx, if !was_accepted => { was_accepted = true; }
                _ = tokio::time::sleep_until(deadline) => {
                    break self.abandon(&id, "timed out").await;
                }
                changed = async {
                    match cancel.as_mut() {
                        Some(rx) => rx.changed().await.ok(),
                        None => std::future::pending().await,
                    }
                } => {
                    if changed.is_some() && cancel.as_ref().is_some_and(|rx| *rx.borrow()) {
                        break self.abandon(&id, "cancelled").await;
                    }
                }
            }
        };
        while let Ok(chunk) = out_rx.try_recv() {
            on_output(chunk);
        }
        self.forget(&id);
        self.touch();
        outcome
    }

    /// Native main-thread stack (broker capture while the thread may be
    /// stuck), symbolized with the modules' PDBs when present.
    pub async fn main_thread_stack(&self, max_frames: usize) -> Result<Vec<String>, String> {
        let request = serde_json::json!({"samples": 1, "maxFrames": max_frames, "resolveManaged": false}).to_string();
        let envelope = self.request("main_thread_stack", &request, Duration::from_secs(10)).await?;
        if envelope.ok != Some(true) {
            return Err(envelope.error.unwrap_or_else(|| "stack capture failed".to_string()));
        }
        let capture: Value = serde_json::from_str(envelope.message.as_deref().unwrap_or("{}")).map_err(|e| e.to_string())?;
        let frames = capture
            .pointer("/samples/0/frames")
            .and_then(Value::as_array)
            .map(|frames| frames.iter().filter_map(|frame| frame.get("ip").and_then(Value::as_u64)).collect::<Vec<_>>())
            .unwrap_or_default();
        let pid = self.registration().pid;
        let raw = frames.iter().map(|ip| format!("0x{ip:x}")).collect::<Vec<_>>();
        // First use loads the PDBs; never let a large one hold up the error.
        Ok(tokio::time::timeout(
            Duration::from_secs(8),
            crate::unity_bridge::hang_diagnosis::symbolize_native_frames(pid, frames),
        )
        .await
        .unwrap_or(raw))
    }

    /// Timeout or cancel: signal the host, then tell whether the code ran.
    async fn abandon(&self, id: &str, reason: &str) -> ExecOutcome {
        let progress = self
            .request("exec_progress", id, Duration::from_secs(3))
            .await
            .ok()
            .and_then(|envelope| envelope.message)
            .and_then(|message| serde_json::from_str::<Value>(&message).ok());
        let started = progress
            .as_ref()
            .is_some_and(|snapshot| snapshot.get("servedBy").and_then(Value::as_str) == Some("native_broker"));
        // A main thread that has not returned to Blender's loop for a while is
        // usually inside a C call (render, modal operator): show where.
        let stalled = progress
            .as_ref()
            .and_then(|snapshot| snapshot.get("mainThreadStallMs").and_then(Value::as_i64))
            .is_some_and(|ms| ms >= 2_000);
        let stack = if started && stalled {
            self.main_thread_stack(16).await.ok()
        } else {
            None
        };
        let _ = self.request("cancel", id, Duration::from_secs(3)).await;
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(id);
        }
        if started {
            let mut message = format!("{reason} while running; cancellation was requested and the scene state is uncertain");
            if let Some(frames) = stack.filter(|frames| !frames.is_empty()) {
                message.push_str("\nBlender main thread (native frames):\n  ");
                message.push_str(&frames.join("\n  "));
            }
            ExecOutcome::Unknown(message)
        } else {
            // A queued request that starts later sees its cancel immediately.
            ExecOutcome::NotExecuted(format!("{reason} before Blender started the code; it was not executed"))
        }
    }

    fn forget(&self, id: &str) {
        if let Ok(mut outputs) = self.outputs.lock() {
            outputs.remove(id);
        }
        if let Ok(mut accepted) = self.accepted.lock() {
            accepted.remove(id);
        }
    }

    fn fail_pending(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
        if let Ok(mut outputs) = self.outputs.lock() {
            outputs.clear();
        }
    }
}

struct BusyGuard(Arc<Instance>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
        self.0.touch();
    }
}

/// Result of an `exec` request.
#[derive(Debug)]
pub enum ExecOutcome {
    /// The host ran the code; the value is the host's result object.
    Completed(Value),
    /// The code never started; retrying is safe.
    NotExecuted(String),
    /// The code may have run partly (timeout or lost connection mid-run).
    Unknown(String),
}

fn envelope_outcome(envelope: PipeEnvelope) -> ExecOutcome {
    if envelope.ok == Some(true) {
        match envelope.message.as_deref().map(serde_json::from_str::<Value>) {
            Some(Ok(value)) => ExecOutcome::Completed(value),
            _ => ExecOutcome::Unknown("unreadable execution result".to_string()),
        }
    } else {
        let error = envelope.error.unwrap_or_else(|| "unknown error".to_string());
        if locus_native_protocol::errors::is_not_executed(&error) {
            ExecOutcome::NotExecuted(error)
        } else {
            ExecOutcome::Unknown(error)
        }
    }
}

// ── hub ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum HubEvent {
    /// Instances appeared, disappeared or changed connection/file state.
    InstancesChanged,
    /// `load_post`, `save_pre`, `save_post` from an instance.
    File { instance: String, event: Value },
}

pub struct Hub {
    instances: Mutex<HashMap<String, Arc<Instance>>>,
    events: broadcast::Sender<HubEvent>,
    stopped: AtomicBool,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub(crate) background: super::background::BackgroundPool,
}

/// Keeps the hub alive. Held by each running Blender workspace service.
#[derive(Clone)]
pub struct HubHandle {
    hub: Arc<Hub>,
}

impl HubHandle {
    pub fn hub(&self) -> &Arc<Hub> {
        &self.hub
    }
}

static CURRENT: Mutex<Option<Weak<Hub>>> = Mutex::new(None);

pub fn acquire() -> Result<HubHandle, String> {
    if !crate::blender::blender_integration_enabled() {
        return Err("Blender integration is turned off in Settings > Services".to_string());
    }
    let mut current = CURRENT.lock().map_err(|error| format!("Blender hub lock poisoned: {error}"))?;
    if let Some(hub) = current.as_ref().and_then(Weak::upgrade) {
        return Ok(HubHandle { hub });
    }
    let hub = Hub::start();
    *current = Some(Arc::downgrade(&hub));
    Ok(HubHandle { hub })
}

/// The running hub, if any service holds it.
pub fn current() -> Option<Arc<Hub>> {
    CURRENT.lock().ok().and_then(|current| current.as_ref().and_then(Weak::upgrade))
}

pub fn is_running() -> bool {
    current().is_some()
}

/// A checkout's Blender service stopped: close the background instances it
/// launched. Interactive instances are never closed by Locus.
pub async fn release_checkout(checkout_id: &CheckoutId) {
    if let Some(hub) = current() {
        hub.background.close_for_checkout(&hub, checkout_id.as_str()).await;
    }
}

impl Hub {
    fn start() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let hub = Arc::new(Self {
            instances: Mutex::new(HashMap::new()),
            events,
            stopped: AtomicBool::new(false),
            watcher: Mutex::new(None),
            background: super::background::BackgroundPool::default(),
        });
        let dir = super::resources::registry_dir();
        let _ = std::fs::create_dir_all(&dir);
        hub.start_watcher(&dir);
        hub.scan(&dir);
        let weak = Arc::downgrade(&hub);
        tauri::async_runtime::spawn(async move {
            super::background::reaper(weak).await;
        });
        eprintln!("[Locus] Blender hub started ({})", dir.display());
        hub
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.events.subscribe()
    }

    fn notify_changed(&self) {
        let _ = self.events.send(HubEvent::InstancesChanged);
        if let Some(app) = APP_HANDLE.get() {
            use tauri::Emitter;
            let _ = app.emit("blender-instances-changed", ());
        }
    }

    fn start_watcher(self: &Arc<Self>, dir: &Path) {
        use notify::Watcher;
        let weak = Arc::downgrade(self);
        let watch_dir = dir.to_path_buf();
        let handler = move |result: notify::Result<notify::Event>| {
            if result.is_err() {
                return;
            }
            if let Some(hub) = weak.upgrade() {
                hub.scan(&watch_dir);
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(mut watcher) => {
                if let Err(error) = watcher.watch(dir, notify::RecursiveMode::NonRecursive) {
                    eprintln!("[Locus] Blender registry watch failed: {error}");
                }
                if let Ok(mut slot) = self.watcher.lock() {
                    *slot = Some(watcher);
                }
            }
            Err(error) => eprintln!("[Locus] Blender registry watcher unavailable: {error}"),
        }
    }

    /// Reconcile instances with the registration files on disk.
    pub fn scan(self: &Arc<Self>, dir: &Path) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut seen = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".json") || name.starts_with('.') {
                    continue;
                }
                let Some(registration) = read_registration(&path) else {
                    continue;
                };
                if !super::process::is_same_process(registration.pid, registration.process_start_ms) {
                    // The process exited or the PID was reused.
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                let key = registration
                    .instance_key
                    .clone()
                    .unwrap_or_else(|| format!("{}-{}", registration.pid, registration.process_start_ms));
                seen.insert(key, registration);
            }
        }
        let mut changed = false;
        let mut to_connect = Vec::new();
        if let Ok(mut instances) = self.instances.lock() {
            let gone = instances
                .keys()
                .filter(|key| !seen.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            for key in gone {
                if let Some(instance) = instances.remove(&key) {
                    instance.stopped.store(true, Ordering::SeqCst);
                    instance.fail_pending();
                    changed = true;
                }
            }
            for (key, registration) in seen {
                match instances.get(&key) {
                    Some(existing) => {
                        let mut registration = registration;
                        let pipe_file_state = existing.file_from_pipe.load(Ordering::SeqCst)
                            && existing.connection() == ConnectionState::Connected;
                        let mut current = existing.registration.lock().unwrap_or_else(|e| e.into_inner());
                        if pipe_file_state {
                            registration.file = current.file.clone();
                            registration.dirty = current.dirty;
                            registration.saved_version = current.saved_version.clone();
                        }
                        if *current != registration {
                            let reconnect = current.pipe != registration.pipe || current.token != registration.token;
                            *current = registration;
                            changed = true;
                            if reconnect && existing.connection() != ConnectionState::Connected {
                                to_connect.push(Arc::clone(existing));
                            }
                        }
                    }
                    None => {
                        let instance = Instance::new(key.clone(), registration);
                        if instance.connection() == ConnectionState::Connecting {
                            to_connect.push(Arc::clone(&instance));
                        }
                        instances.insert(key, instance);
                        changed = true;
                    }
                }
            }
        }
        for instance in to_connect {
            let hub = Arc::downgrade(self);
            tauri::async_runtime::spawn(async move {
                connection_loop(hub, instance).await;
            });
        }
        if changed {
            self.notify_changed();
        }
    }

    pub fn instances(&self) -> Vec<Arc<Instance>> {
        let mut list = self
            .instances
            .lock()
            .map(|instances| instances.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        list.sort_by(|a, b| a.key.cmp(&b.key));
        list
    }

    pub fn instance(&self, key: &str) -> Option<Arc<Instance>> {
        self.instances.lock().ok().and_then(|instances| instances.get(key).cloned())
    }

    pub fn instance_by_pid(&self, pid: u32) -> Option<Arc<Instance>> {
        self.instances().into_iter().find(|instance| instance.registration().pid == pid)
    }

    /// Wait (event driven) until an instance with `pid` connects.
    pub async fn wait_for_pid(self: &Arc<Self>, pid: u32, timeout: Duration) -> Result<Arc<Instance>, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut events = self.subscribe();
        loop {
            self.scan(&super::resources::registry_dir());
            if let Some(instance) = self.instance_by_pid(pid) {
                match instance.connection() {
                    ConnectionState::Connected => return Ok(instance),
                    ConnectionState::HostError => {
                        let error = instance.snapshot().error.unwrap_or_else(|| "host failed to start".to_string());
                        return Err(format!("Blender started but the Locus host failed: {error}"));
                    }
                    _ => {}
                }
            }
            if super::process::identity(pid).is_none() {
                return Err("Blender exited before it connected".to_string());
            }
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(_) => continue,
                Err(_) => return Err(format!("Blender did not connect within {} s", timeout.as_secs())),
            }
        }
    }

    /// Wait (event driven) until `instance` reports `file` open. An add-on in
    /// `startup/` connects before Blender loads the file given on the command
    /// line, and callers target the instance by that file right away.
    pub async fn wait_for_file(&self, instance: &Arc<Instance>, file: &Path, timeout: Duration) -> bool {
        let wanted = super::installations::normalize_path(file);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut events = self.subscribe();
        loop {
            let open = instance.registration().file;
            if open.as_deref().is_some_and(|open| super::installations::normalize_path(Path::new(open)) == wanted) {
                return true;
            }
            if instance.stopped.load(Ordering::SeqCst) {
                return false;
            }
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                _ => return false,
            }
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Ok(mut watcher) = self.watcher.lock() {
            watcher.take();
        }
        if let Ok(mut instances) = self.instances.lock() {
            for instance in instances.values() {
                instance.stopped.store(true, Ordering::SeqCst);
                instance.fail_pending();
            }
            instances.clear();
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop();
        self.background.shutdown_all_blocking();
        eprintln!("[Locus] Blender hub stopped");
    }
}

// ── connection ─────────────────────────────────────────────────────────────

async fn connection_loop(hub: Weak<Hub>, instance: Arc<Instance>) {
    let mut backoff = RECONNECT_MIN;
    loop {
        if instance.stopped.load(Ordering::SeqCst) || hub.upgrade().is_none_or(|hub| hub.stopped.load(Ordering::SeqCst)) {
            return;
        }
        instance.set_state(ConnectionState::Connecting);
        match connect_once(&hub, &instance).await {
            Ok(()) => backoff = RECONNECT_MIN,
            Err(error) => {
                instance.set_error(Some(error));
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
        instance.set_state(ConnectionState::Disconnected);
        instance.file_from_pipe.store(false, Ordering::SeqCst);
        instance.fail_pending();
        if let Ok(mut writer) = instance.writer.try_lock() {
            writer.take();
        }
        // A killed or crashed Blender leaves its registration file behind and
        // produces no directory event; the broken pipe is the signal.
        let registration = instance.registration();
        if !super::process::is_same_process(registration.pid, registration.process_start_ms) {
            if let Some(hub) = hub.upgrade() {
                hub.scan(&super::resources::registry_dir());
            }
            return;
        }
        if let Some(hub) = hub.upgrade() {
            hub.notify_changed();
        }
        tokio::time::sleep(backoff).await;
    }
}

#[cfg(not(windows))]
async fn open_pipe(_pipe: &str) -> Result<PipeClient, String> {
    Err("Blender integration is only available on Windows".to_string())
}

#[cfg(windows)]
async fn open_pipe(pipe: &str) -> Result<PipeClient, String> {
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_PIPE_BUSY: i32 = 231;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(pipe) {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                return Err("pipe not found (instance closing or not ready)".to_string());
            }
            Err(error) => return Err(format!("pipe open failed: {error}")),
        }
    }
}

async fn connect_once(hub: &Weak<Hub>, instance: &Arc<Instance>) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let registration = instance.registration();
    let pipe = registration.pipe.clone().ok_or_else(|| "instance has no pipe".to_string())?;
    let token = registration.token.clone().unwrap_or_default();
    let client = open_pipe(&pipe).await?;
    let (read_half, write_half) = tokio::io::split(client);
    *instance.writer.lock().await = Some(write_half);

    let reader_instance = Arc::clone(instance);
    let reader_hub = hub.clone();
    let reader = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(read_half).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            dispatch_line(&reader_hub, &reader_instance, &line);
        }
    });

    // hello must be the first frame; nothing else is served before it.
    let id = next_request_id();
    let rx = instance.send(&id, "hello", &token).await?;
    let hello = tokio::time::timeout(HELLO_TIMEOUT, rx)
        .await
        .map_err(|_| "hello timed out".to_string())?
        .map_err(|_| "connection closed during hello".to_string())?;
    if hello.ok != Some(true) {
        reader.abort();
        return Err(format!("hello rejected: {}", hello.error.unwrap_or_default()));
    }
    instance.set_state(ConnectionState::Connected);
    instance.set_error(None);
    if let Ok(mut connected_at) = instance.connected_at.lock() {
        *connected_at = Some(Instant::now());
    }
    if let Some(hub) = hub.upgrade() {
        hub.notify_changed();
    }

    let mut reader = reader;
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    loop {
        tokio::select! {
            _ = &mut reader => return Ok(()),
            _ = ping.tick() => {
                if instance.stopped.load(Ordering::SeqCst) {
                    reader.abort();
                    return Ok(());
                }
                if instance.request("ping", "", Duration::from_secs(5)).await.is_err() {
                    reader.abort();
                    return Err("ping failed".to_string());
                }
            }
        }
    }
}

fn dispatch_line(hub: &Weak<Hub>, instance: &Arc<Instance>, line: &str) {
    let Ok(envelope) = serde_json::from_str::<PipeEnvelope>(line.trim()) else {
        return;
    };
    if let Some(reply_to) = envelope.reply_to.clone() {
        let sender = instance.pending.lock().ok().and_then(|mut pending| pending.remove(&reply_to));
        if let Some(sender) = sender {
            let _ = sender.send(envelope);
        }
        return;
    }
    let payload = envelope
        .message
        .as_deref()
        .and_then(|message| serde_json::from_str::<Value>(message).ok())
        .unwrap_or(Value::Null);
    match envelope.kind.as_str() {
        "blender-output" => {
            let execution = payload.get("executionId").and_then(Value::as_str).unwrap_or("");
            let sender = instance.outputs.lock().ok().and_then(|outputs| outputs.get(execution).cloned());
            if let Some(sender) = sender {
                let _ = sender.send(OutputChunk {
                    stream: payload.get("stream").and_then(Value::as_str).unwrap_or("stdout").to_string(),
                    text: payload.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                });
            }
        }
        locus_native_protocol::envelope::REQUEST_ACCEPTED_EVENT => {
            let request = payload.get("requestId").and_then(Value::as_str).unwrap_or("");
            let sender = instance.accepted.lock().ok().and_then(|mut accepted| accepted.remove(request));
            if let Some(sender) = sender {
                let _ = sender.send(());
            }
        }
        "blender-file" => {
            instance.update_file_state(&payload);
            if let Some(hub) = hub.upgrade() {
                let _ = hub.events.send(HubEvent::File {
                    instance: instance.key.clone(),
                    event: payload,
                });
                hub.notify_changed();
            }
        }
        _ => {}
    }
}

// ── state plane ────────────────────────────────────────────────────────────

/// Latest status snapshot the instance's broker published, read straight
/// from shared memory (no pipe round trip; works while the main thread is
/// stuck).
pub fn read_state_plane(instance_key: &str) -> Option<Value> {
    let name = locus_native_protocol::names::blender_state_plane_name(instance_key);
    let bytes = read_shared_memory(&name, locus_native_protocol::state_plane::total_size())?;
    let snapshot = locus_native_protocol::state_plane::read_latest(&bytes)?;
    serde_json::from_slice(&snapshot.payload).ok()
}

#[cfg(windows)]
fn read_shared_memory(name: &str, size: usize) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    type Handle = *mut c_void;
    unsafe extern "system" {
        fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
        fn MapViewOfFile(mapping: Handle, access: u32, high: u32, low: u32, bytes: usize) -> *mut c_void;
        fn UnmapViewOfFile(base: *const c_void) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    const FILE_MAP_READ: u32 = 0x0004;
    let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(Some(0)).collect();
    unsafe {
        let mapping = OpenFileMappingW(FILE_MAP_READ, 0, wide.as_ptr());
        if mapping.is_null() {
            return None;
        }
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, size);
        if view.is_null() {
            CloseHandle(mapping);
            return None;
        }
        let bytes = std::slice::from_raw_parts(view as *const u8, size).to_vec();
        UnmapViewOfFile(view);
        CloseHandle(mapping);
        Some(bytes)
    }
}

#[cfg(not(windows))]
fn read_shared_memory(_name: &str, _size: usize) -> Option<Vec<u8>> {
    None
}

/// Unregistered Blender processes (running without the host add-on).
pub fn unregistered_processes(hub: &Hub) -> Vec<super::process::ProcessInfo> {
    let registered = hub
        .instances()
        .iter()
        .map(|instance| instance.registration().pid)
        .collect::<std::collections::BTreeSet<_>>();
    super::process::list_named("blender.exe")
        .into_iter()
        .filter(|process| !registered.contains(&process.pid))
        .collect()
}

pub fn registry_dir() -> PathBuf {
    super::resources::registry_dir()
}
