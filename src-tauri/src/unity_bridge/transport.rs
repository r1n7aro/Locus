use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use super::{get_native_pipe_name, native_bridge_enabled, PipeResponse};

// ── Windows: named-pipe transport ────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicU64, Ordering},
            OnceLock,
        },
    };
    use tauri::{AppHandle, Emitter};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
        net::windows::named_pipe::{ClientOptions, NamedPipeClient},
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
        writer: Mutex<Option<WriteHalf<NamedPipeClient>>>,
        pending: Mutex<HashMap<String, oneshot::Sender<Result<PipeEnvelope, String>>>>,
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

    static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<UnityPipeConnection>>>> =
        OnceLock::new();
    static ACTIVE_CONNECTIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    static EVENT_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
    static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);
    const PIPE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

    pub(super) fn set_event_app_handle(app_handle: AppHandle) {
        let _ = EVENT_APP_HANDLE.set(app_handle);
    }

    fn connections() -> &'static Mutex<HashMap<String, Arc<UnityPipeConnection>>> {
        CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn active_connections() -> &'static Mutex<HashMap<String, String>> {
        ACTIVE_CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn project_connection_key(project_path: &str) -> String {
        let trimmed = project_path
            .strip_prefix(r"\\?\")
            .unwrap_or(project_path)
            .trim();
        let mut value = trimmed.replace('/', "\\");
        while value.ends_with('\\') && value.len() > 3 {
            value.pop();
        }
        value.to_ascii_lowercase()
    }

    fn next_request_id() -> String {
        format!("req-{}", REQUEST_SEQ.fetch_add(1, Ordering::Relaxed))
    }

    async fn open_client_with_retry(
        pipe_name: &str,
        max_retries: u32,
    ) -> Result<NamedPipeClient, String> {
        const ERROR_PIPE_BUSY: i32 = 231;
        let max_retries = max_retries.max(1);

        let mut last_err = String::new();

        for attempt in 0..max_retries {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(client),
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempt + 1 < max_retries =>
                {
                    last_err = format!("Failed to connect to Unity Editor ({}): {}", pipe_name, e);
                    let delay_ms = (100 * (attempt as u64 + 1)).min(1_000);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to connect to Unity Editor ({}): {}",
                        pipe_name, e
                    ));
                }
            }
        }

        Err(last_err)
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
        let mut pending = conn.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(reason.clone()));
        }
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

    fn handle_unsolicited_message(env: &PipeEnvelope) {
        let event_name = env.kind.trim();
        if event_name.is_empty() {
            eprintln!(
                "[Locus] unsolicited Unity message missing type: message={:?}, error={:?}",
                env.message, env.error
            );
            return;
        }

        if let Some(app_handle) = EVENT_APP_HANDLE.get() {
            let _ = app_handle.emit(event_name, unsolicited_payload(env));
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

    async fn reader_loop(conn: Arc<UnityPipeConnection>, reader: ReadHalf<NamedPipeClient>) {
        let pipe_name = conn.pipe_name.clone();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();

            let n = match reader.read_line(&mut line).await {
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

            let trimmed = line.trim().trim_start_matches('\u{FEFF}');
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

            let reply_to = env
                .reply_to
                .clone()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());

            if let Some(reply_to) = reply_to {
                let tx = {
                    let mut pending = conn.pending.lock().await;
                    pending.remove(&reply_to)
                };

                if let Some(tx) = tx {
                    let _ = tx.send(Ok(env));
                } else {
                    eprintln!(
                        "[Locus] received response for unknown request id: {}",
                        reply_to
                    );
                }
            } else {
                if is_active_connection(&conn).await {
                    handle_unsolicited_message(&env);
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
        {
            let map = connections().lock().await;
            if let Some(conn) = map.get(&pipe_name) {
                return Ok(conn.clone());
            }
        }

        let client = open_client_with_retry(&pipe_name, max_retries).await?;
        let (reader, writer) = tokio::io::split(client);

        let new_conn = Arc::new(UnityPipeConnection {
            project_key,
            pipe_name: pipe_name.clone(),
            writer: Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
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
    ) -> Result<PipeResponse, String> {
        let conn = get_or_connect(project_path).await?;
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

        let json =
            serde_json::to_string(&env).map_err(|e| format!("Serialization failed: {}", e))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = conn.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }
        let mut pending_guard = PendingRequestGuard::new(conn.clone(), request_id.clone());

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

        let env = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(Ok(env))) => env,
                Ok(Ok(Err(e))) => return Err(e),
                Ok(Err(_)) => {
                    return Err("Unity response failed: response channel closed".to_string())
                }
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
                Ok(Ok(env)) => env,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err("Unity response failed: response channel closed".to_string()),
            }
        };
        pending_guard.disarm();

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
        let conn = match tokio::time::timeout(response_timeout, get_or_connect(project_path)).await
        {
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

        let mut frame =
            serde_json::to_vec(&env).map_err(|e| format!("Serialization failed: {}", e))?;
        frame.push(b'\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = conn.pending.lock().await;
            pending.insert(request_id.clone(), tx);
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
        send_message_inner(project_path, msg_type, message, Some(timeout)).await
    }

    pub async fn send_message_without_timeout(
        project_path: &str,
        msg_type: &str,
        message: &str,
    ) -> Result<PipeResponse, String> {
        send_message_inner(project_path, msg_type, message, None).await
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
}

// ── Public dispatch ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub fn set_event_app_handle(app_handle: tauri::AppHandle) {
    windows_impl::set_event_app_handle(app_handle);
}

#[cfg(not(target_os = "windows"))]
pub fn set_event_app_handle(_app_handle: tauri::AppHandle) {}

#[cfg(target_os = "windows")]
pub async fn send_message(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    windows_impl::send_message(project_path, msg_type, message).await
}

#[cfg(target_os = "windows")]
pub async fn send_message_with_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
    timeout: Duration,
) -> Result<PipeResponse, String> {
    windows_impl::send_message_with_timeout(project_path, msg_type, message, timeout).await
}

#[cfg(target_os = "windows")]
pub async fn send_message_without_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    windows_impl::send_message_without_timeout(project_path, msg_type, message).await
}

#[cfg(target_os = "windows")]
pub async fn send_message_if_writer_free(
    project_path: &str,
    msg_type: &str,
    message: &str,
    response_timeout: Duration,
) -> Result<Option<PipeResponse>, String> {
    windows_impl::send_message_if_writer_free(project_path, msg_type, message, response_timeout)
        .await
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message_if_writer_free(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
    _response_timeout: Duration,
) -> Result<Option<PipeResponse>, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message_with_timeout(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
    _timeout: Duration,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message_without_timeout(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(target_os = "windows")]
pub async fn disconnect(project_path: &str) {
    windows_impl::disconnect(project_path).await;
}

#[cfg(target_os = "windows")]
pub async fn disconnect_with_reason(project_path: &str, reason: &str) {
    windows_impl::disconnect_with_reason(project_path, reason).await;
}

#[cfg(not(target_os = "windows"))]
pub async fn disconnect(_project_path: &str) {}

#[cfg(not(target_os = "windows"))]
pub async fn disconnect_with_reason(_project_path: &str, _reason: &str) {}
