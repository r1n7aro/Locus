//! Independent Unix-socket client; no Windows transport code is compiled here.
use super::super::macos_ipc;
use super::macos_requests::PendingRequests;
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};
use tauri::AppHandle;
use tokio::{
    io::{AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    net::UnixStream,
    sync::oneshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PipeEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(default, rename = "reply_to", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,

    #[serde(default, rename = "type")]
    pub kind: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    #[serde(default, rename = "processId", skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,

    #[serde(
        default,
        rename = "processPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub process_path: Option<String>,
}

struct UnityPipeConnection {
    project_key: String,
    pipe_name: String,
    writer: Mutex<Option<WriteHalf<UnixStream>>>,
    pending: Mutex<PendingRequests>,
    reader_abort: Mutex<Option<tokio::task::AbortHandle>>,
}

struct PendingRequestGuard {
    conn: Arc<UnityPipeConnection>,
    request_id: String,
    armed: bool,
}

impl PendingRequestGuard {
    fn new(conn: Arc<UnityPipeConnection>, request_id: String) -> Self {
        Self {
            conn,
            request_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let conn = self.conn.clone();
        let request_id = self.request_id.clone();
        tokio::spawn(async move {
            conn.pending.lock().await.remove(&request_id);
        });
    }
}

static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<UnityPipeConnection>>>> = OnceLock::new();
static CONNECTION_ATTEMPT_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
static ACTIVE_CONNECTIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static SERVICE_EVENT_SCOPES: OnceLock<
    std::sync::RwLock<HashMap<String, crate::workspace_service::event::WorkspaceEventScope>>,
> = OnceLock::new();
static EVENT_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
const PIPE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const BROKER_REQUEST_ACCEPTED_EVENT: &str = "locus-request-accepted";

pub(super) fn set_event_app_handle(app_handle: AppHandle) {
    let _ = EVENT_APP_HANDLE.set(app_handle);
}

fn connections() -> &'static Mutex<HashMap<String, Arc<UnityPipeConnection>>> {
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn connection_attempt_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    CONNECTION_ATTEMPT_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn connection_attempt_lock(pipe_name: &str) -> Arc<Mutex<()>> {
    let mut locks = connection_attempt_locks().lock().await;
    locks
        .entry(pipe_name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn active_connections() -> &'static Mutex<HashMap<String, String>> {
    ACTIVE_CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn service_event_scopes(
) -> &'static std::sync::RwLock<HashMap<String, crate::workspace_service::event::WorkspaceEventScope>>
{
    SERVICE_EVENT_SCOPES.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

pub(super) fn set_service_event_scope(
    project_path: &str,
    scope: Option<crate::workspace_service::event::WorkspaceEventScope>,
) {
    let key = project_connection_key(project_path);
    if let Ok(mut scopes) = service_event_scopes().write() {
        if let Some(scope) = scope {
            scopes.insert(key, scope);
        } else {
            scopes.remove(&key);
        }
    }
}

fn service_event_scope(
    project_key: &str,
) -> Option<crate::workspace_service::event::WorkspaceEventScope> {
    service_event_scopes()
        .read()
        .ok()
        .and_then(|scopes| scopes.get(project_key).cloned())
}

fn project_connection_key(project_path: &str) -> String {
    std::fs::canonicalize(project_path)
        .unwrap_or_else(|_| std::path::PathBuf::from(project_path))
        .to_string_lossy()
        .into_owned()
}
fn next_request_id() -> String {
    static SESSION: OnceLock<String> = OnceLock::new();
    let session = SESSION.get_or_init(|| {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    format!(
        "req-{session}-{}",
        REQUEST_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}
async fn open_client_with_retry(
    project: &str,
    endpoint: &str,
    max_retries: u32,
) -> Result<UnixStream, String> {
    let mut last_error = String::new();
    for attempt in 0..max_retries.max(1) {
        let result = async {
            let snapshot =
                macos_ipc::read_snapshot(project, endpoint).map_err(|e| e.to_string())?;
            let stream =
                tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(endpoint))
                    .await
                    .map_err(|_| "Unity socket connect timed out".to_string())?
                    .map_err(|e| e.to_string())?;
            let pid = macos_ipc::peer_identity(&stream).map_err(|e| e.to_string())?;
            if snapshot["processId"].as_u64() != Some(u64::from(pid)) {
                return Err("Unity socket peer does not own the published state".to_string());
            }
            // Recheck after connect to reject a process-exit/PID-reuse race.
            let identity = macos_ipc::process_identity(pid).ok_or("Unity peer exited")?;
            if snapshot["processStartSecs"].as_u64() != Some(identity.0)
                || snapshot["processStartMicros"].as_u64() != Some(identity.1)
            {
                return Err("Unity peer identity changed".to_string());
            }
            Ok(stream)
        }
        .await;
        match result {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
        if attempt + 1 < max_retries {
            tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt + 1))).await;
        }
    }
    Err(format!(
        "Failed to connect to Unity Editor ({endpoint}): {last_error}"
    ))
}

async fn remove_connection_if_same(pipe_name: &str, conn: &Arc<UnityPipeConnection>) {
    let mut map = connections().lock().await;
    if map
        .get(pipe_name)
        .map(|existing| Arc::ptr_eq(existing, conn))
        .unwrap_or(false)
    {
        map.remove(pipe_name);
    }
    drop(map);
    remove_active_connection_if_same(conn).await;
}

async fn mark_active_connection(conn: &Arc<UnityPipeConnection>) {
    let mut map = active_connections().lock().await;
    map.insert(conn.project_key.clone(), conn.pipe_name.clone());
}

async fn remove_active_connection_if_same(conn: &Arc<UnityPipeConnection>) {
    let mut map = active_connections().lock().await;
    if map
        .get(&conn.project_key)
        .map(|pipe_name| pipe_name == &conn.pipe_name)
        .unwrap_or(false)
    {
        map.remove(&conn.project_key);
    }
}

async fn is_active_connection(conn: &Arc<UnityPipeConnection>) -> bool {
    let map = active_connections().lock().await;
    map.get(&conn.project_key)
        .map(|pipe_name| pipe_name == &conn.pipe_name)
        .unwrap_or(true)
}

async fn fail_all_pending(conn: &Arc<UnityPipeConnection>, reason: String) {
    conn.pending.lock().await.fail_all(&reason);
}

async fn close_connection(conn: &Arc<UnityPipeConnection>, reason: String) {
    fail_all_pending(conn, reason).await;

    if let Some(abort) = conn.reader_abort.lock().await.take() {
        abort.abort();
    }

    match conn.writer.try_lock() {
        Ok(mut writer) => {
            if let Some(mut writer) = writer.take() {
                let _ = writer.shutdown().await;
            }
        }
        Err(_) => {
            let conn = conn.clone();
            tokio::spawn(async move {
                let mut writer = conn.writer.lock().await;
                if let Some(mut writer) = writer.take() {
                    let _ = writer.shutdown().await;
                }
            });
        }
    }
}

fn unsolicited_payload(env: &PipeEnvelope) -> serde_json::Value {
    if let Some(message) = env.message.as_deref() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(message) {
            return value;
        }
    }

    serde_json::json!({
        "message": env.message,
        "error": env.error
    })
}

fn handle_unsolicited_message(project_key: &str, env: &PipeEnvelope) {
    let event_name = env.kind.trim();
    if event_name.is_empty() {
        eprintln!(
            "[Locus] unsolicited Unity message missing type: message={:?}, error={:?}",
            env.message, env.error
        );
        return;
    }

    if let Some(app_handle) = EVENT_APP_HANDLE.get() {
        let payload = unsolicited_payload(env);
        let Some(scope) = service_event_scope(project_key) else {
            tracing::debug!(
                log_module = "Locus",
                "dropping unsolicited Unity event without a live service scope: project={}, type={}",
                project_key,
                event_name
            );
            return;
        };
        let outcome = crate::workspace_service::event::emit_for_workspace_scope(
            app_handle, &scope, event_name, payload,
        );
        let _ = outcome;
        return;
    }

    tracing::debug!(
        log_module = "Locus",
        "unsolicited Unity message without app handle: type={}, message={:?}, error={:?}",
        env.kind,
        env.message,
        env.error
    );
}

fn broker_accepted_request_id(env: &PipeEnvelope) -> Option<String> {
    if env.kind != BROKER_REQUEST_ACCEPTED_EVENT {
        return None;
    }
    let message = env.message.as_deref()?;
    serde_json::from_str::<serde_json::Value>(message)
        .ok()?
        .get("requestId")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn reader_loop(conn: Arc<UnityPipeConnection>, reader: ReadHalf<UnixStream>) {
    let pipe_name = conn.pipe_name.clone();
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();

    loop {
        line.clear();

        let n = match macos_ipc::read_frame(&mut reader, &mut line, 32 * 1024 * 1024).await {
            Ok(n) => n,
            Err(e) => {
                eprintln!("[Locus] pipe read error ({}): {}", pipe_name, e);
                break;
            }
        };

        if n == 0 {
            eprintln!("[Locus] pipe disconnected: {}", pipe_name);
            break;
        }

        let Ok(line_text) = std::str::from_utf8(&line) else {
            break;
        };
        let trimmed = line_text.trim().trim_start_matches('\u{FEFF}');
        if trimmed.is_empty() {
            continue;
        }

        let env: PipeEnvelope = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[Locus] failed to parse pipe message ({}): {} | raw={}",
                    pipe_name, e, trimmed
                );
                continue;
            }
        };

        if env.kind == BROKER_REQUEST_ACCEPTED_EVENT {
            if let Some(request_id) = broker_accepted_request_id(&env) {
                if !conn.pending.lock().await.accept(&request_id) {
                    tracing::debug!(
                        log_module = "Locus",
                        "received broker acceptance for untracked request id: {} (pipe: {})",
                        request_id,
                        pipe_name
                    );
                }
            } else {
                eprintln!("[Locus] malformed broker request acceptance: {}", trimmed);
            }
            continue;
        }

        let reply_to = env
            .reply_to
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if let Some(reply_to) = reply_to {
            if !conn.pending.lock().await.resolve(&reply_to, Ok(env)) {
                eprintln!(
                    "[Locus] received response for unknown request id: {}",
                    reply_to
                );
            }
        } else {
            if is_active_connection(&conn).await {
                handle_unsolicited_message(&conn.project_key, &env);
            } else {
                tracing::debug!(
                    log_module = "Locus",
                    "dropping unsolicited Unity message from inactive pipe: {}",
                    conn.pipe_name
                );
            }
        }
    }

    remove_connection_if_same(&pipe_name, &conn).await;
    fail_all_pending(&conn, format!("Unity pipe disconnected: {}", pipe_name)).await;
}

const NATIVE_CONNECT_RETRIES: u32 = 3;

async fn connect_pipe(
    project_key: String,
    pipe_name: String,
    max_retries: u32,
) -> Result<Arc<UnityPipeConnection>, String> {
    // A project has one active desktop connection. Serialize concurrent first
    // requests and recheck the cache so every waiter uses the same socket.
    let attempt_lock = connection_attempt_lock(&pipe_name).await;
    let _attempt_guard = attempt_lock.lock().await;
    {
        let map = connections().lock().await;
        if let Some(conn) = map.get(&pipe_name) {
            return Ok(conn.clone());
        }
    }

    let client = open_client_with_retry(&project_key, &pipe_name, max_retries).await?;
    let (reader, writer) = tokio::io::split(client);

    let new_conn = Arc::new(UnityPipeConnection {
        project_key,
        pipe_name: pipe_name.clone(),
        writer: Mutex::new(Some(writer)),
        pending: Mutex::new(PendingRequests::default()),
        reader_abort: Mutex::new(None),
    });

    {
        let mut map = connections().lock().await;
        if let Some(existing) = map.get(&pipe_name) {
            return Ok(existing.clone());
        }
        map.insert(pipe_name.clone(), new_conn.clone());
    }

    let reader_task = tokio::spawn(reader_loop(new_conn.clone(), reader));
    *new_conn.reader_abort.lock().await = Some(reader_task.abort_handle());
    Ok(new_conn)
}

/// Native-only: all desktop Unity traffic goes through the broker pipe
/// served by `locus_native`, so a missing broker is surfaced as a
/// connection error.
async fn get_or_connect(project_path: &str) -> Result<Arc<UnityPipeConnection>, String> {
    let project_key = project_connection_key(project_path);
    if !native_bridge_enabled() {
        return Err("Unity native broker is disabled".to_string());
    }

    let conn = connect_pipe(
        project_key,
        get_native_pipe_name(project_path),
        NATIVE_CONNECT_RETRIES,
    )
    .await?;
    mark_active_connection(&conn).await;
    Ok(conn)
}

async fn send_message_inner(
    project_path: &str,
    msg_type: &str,
    message: &str,
    timeout: Option<Duration>,
    acceptance_tx: Option<oneshot::Sender<()>>,
) -> Result<PipeResponse, String> {
    let trace_exit_play_mode = msg_type == "exit_play_mode";
    if trace_exit_play_mode {
        tracing::info!(log_module = "Locus", "exit_play_mode transport: connecting");
    }
    let conn = get_or_connect(project_path).await?;
    if trace_exit_play_mode {
        tracing::info!(log_module = "Locus", "exit_play_mode transport: connected");
    }
    let request_id = next_request_id();

    let env = PipeEnvelope {
        id: Some(request_id.clone()),
        reply_to: None,
        kind: msg_type.to_string(),
        ok: None,
        message: Some(message.to_string()),
        error: None,
        process_id: None,
        process_path: None,
    };

    let json = serde_json::to_string(&env).map_err(|e| format!("Serialization failed: {}", e))?;

    let (tx, mut response_rx) = oneshot::channel();
    {
        let mut pending = conn.pending.lock().await;
        pending.insert(request_id.clone(), tx, acceptance_tx);
    }
    let mut pending_guard = PendingRequestGuard::new(conn.clone(), request_id.clone());
    if trace_exit_play_mode {
        tracing::info!(
            log_module = "Locus",
            "exit_play_mode transport: writing request"
        );
    }

    let write_result = tokio::time::timeout(PIPE_WRITE_TIMEOUT, async {
        let mut writer_guard = conn.writer.lock().await;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| "Unity pipe connection is closing".to_string())?;
        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| format!("Pipe write failed: {}", e))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| format!("Newline write failed: {}", e))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("Pipe flush failed: {}", e))
    })
    .await
    .unwrap_or_else(|_| Err("Unity pipe write timed out".to_string()));

    if let Err(err) = write_result {
        {
            let mut pending = conn.pending.lock().await;
            pending.remove(&request_id);
        }
        pending_guard.disarm();
        remove_connection_if_same(&conn.pipe_name, &conn).await;
        close_connection(&conn, err.clone()).await;
        return Err(err);
    }
    if trace_exit_play_mode {
        tracing::info!(
            log_module = "Locus",
            "exit_play_mode transport: request written"
        );
    }

    let rx = async {
        match (&mut response_rx).await {
            Ok(Ok(env)) => Ok(env),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("Unity response failed: response channel closed".to_string()),
        }
    };
    tokio::pin!(rx);

    let env = if let Some(timeout) = timeout {
        if trace_exit_play_mode {
            tracing::info!(
                log_module = "Locus",
                "exit_play_mode transport: awaiting response"
            );
        }
        match tokio::time::timeout(timeout, &mut rx).await {
            Ok(Ok(env)) => env,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                let err = "Unity response timed out".to_string();
                let mut pending = conn.pending.lock().await;
                pending.remove(&request_id);
                pending_guard.disarm();
                return Err(err);
            }
        }
    } else {
        match rx.await {
            Ok(env) => env,
            Err(error) => return Err(error),
        }
    };
    pending_guard.disarm();
    if trace_exit_play_mode {
        tracing::info!(
            log_module = "Locus",
            "exit_play_mode transport: response received"
        );
    }

    Ok(PipeResponse {
        ok: env.ok.unwrap_or(false),
        error: env.error,
        message: env.message,
        process_id: env.process_id.filter(|id| *id > 0),
        process_path: env
            .process_path
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

/// Best-effort send for progress polls riding on a connection that has a
/// request in flight. The execute loop polls progress from the same task
/// that drives the in-flight send future inside a `select!` handler, so
/// awaiting the writer lock here deadlocks until the write timeout (the
/// suspended send future holds the guard and is never polled while the
/// handler runs) and then tears down the connection under the in-flight
/// request. Instead: returns `Ok(None)` when the writer is busy so the
/// caller skips this poll, and on response timeout drops only its own
/// pending entry — the connection and other pending requests stay up.
/// Only a write that fails after acquiring the writer (possible partial
/// frame) closes the connection, matching `send_message_inner`.
pub async fn send_message_if_writer_free(
    project_path: &str,
    msg_type: &str,
    message: &str,
    response_timeout: Duration,
) -> Result<Option<PipeResponse>, String> {
    let conn = match tokio::time::timeout(response_timeout, get_or_connect(project_path)).await {
        Ok(result) => result?,
        Err(_) => return Err("Unity pipe connect timed out".to_string()),
    };

    let mut writer_guard = match conn.writer.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(None),
    };

    let request_id = next_request_id();
    let env = PipeEnvelope {
        id: Some(request_id.clone()),
        reply_to: None,
        kind: msg_type.to_string(),
        ok: None,
        message: Some(message.to_string()),
        error: None,
        process_id: None,
        process_path: None,
    };

    let mut frame = serde_json::to_vec(&env).map_err(|e| format!("Serialization failed: {}", e))?;
    frame.push(b'\n');

    let (tx, rx) = oneshot::channel();
    {
        let mut pending = conn.pending.lock().await;
        pending.insert(request_id.clone(), tx, None);
    }
    let mut pending_guard = PendingRequestGuard::new(conn.clone(), request_id.clone());

    let write_timeout = PIPE_WRITE_TIMEOUT.min(response_timeout);
    let write_result = tokio::time::timeout(write_timeout, async {
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| "Unity pipe connection is closing".to_string())?;
        writer
            .write_all(&frame)
            .await
            .map_err(|e| format!("Pipe write failed: {}", e))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("Pipe flush failed: {}", e))
    })
    .await
    .unwrap_or_else(|_| Err("Unity pipe write timed out".to_string()));

    // Release the writer before waiting on the response so the main
    // request's own writes are never queued behind this poll.
    drop(writer_guard);

    if let Err(err) = write_result {
        {
            let mut pending = conn.pending.lock().await;
            pending.remove(&request_id);
        }
        pending_guard.disarm();
        remove_connection_if_same(&conn.pipe_name, &conn).await;
        close_connection(&conn, err.clone()).await;
        return Err(err);
    }

    let env = match tokio::time::timeout(response_timeout, rx).await {
        Ok(Ok(Ok(env))) => env,
        Ok(Ok(Err(e))) => return Err(e),
        Ok(Err(_)) => return Err("Unity response failed: response channel closed".to_string()),
        Err(_) => {
            let mut pending = conn.pending.lock().await;
            pending.remove(&request_id);
            pending_guard.disarm();
            return Err("Unity response timed out".to_string());
        }
    };
    pending_guard.disarm();

    Ok(Some(PipeResponse {
        ok: env.ok.unwrap_or(false),
        error: env.error,
        message: env.message,
        process_id: env.process_id.filter(|id| *id > 0),
        process_path: env
            .process_path
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }))
}

pub async fn send_message_with_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
    timeout: Duration,
) -> Result<PipeResponse, String> {
    let result = tokio::time::timeout(
        timeout,
        send_message_inner(project_path, msg_type, message, Some(timeout), None),
    )
    .await
    .map_err(|_| "Unity request timed out".to_string())?;
    if msg_type == "exit_play_mode" && result.is_err() {
        tracing::info!(
            log_module = "Locus",
            "exit_play_mode transport: request returned error"
        );
    }
    result
}

pub async fn send_message_without_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    send_message_inner(project_path, msg_type, message, None, None).await
}

pub async fn send_message_without_timeout_with_acceptance(
    project_path: &str,
    msg_type: &str,
    message: &str,
    acceptance_tx: oneshot::Sender<()>,
) -> Result<PipeResponse, String> {
    send_message_inner(project_path, msg_type, message, None, Some(acceptance_tx)).await
}

pub async fn send_message(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    send_message_with_timeout(project_path, msg_type, message, Duration::from_secs(35)).await
}

pub async fn disconnect_with_reason(project_path: &str, reason: &str) {
    let native_pipe_name = get_native_pipe_name(project_path);
    let project_key = project_connection_key(project_path);

    let conns = {
        let mut map = connections().lock().await;
        map.remove(&native_pipe_name)
            .into_iter()
            .collect::<Vec<_>>()
    };

    active_connections().lock().await.remove(&project_key);

    for conn in conns {
        close_connection(&conn, reason.to_string()).await;
    }
}

pub async fn disconnect(project_path: &str) {
    disconnect_with_reason(project_path, "disconnected for recompile").await;
}
