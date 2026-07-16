//! Streamable HTTP transport for MCP servers (spec revision 2025-06-18).
//!
//! One endpoint speaks everything: JSON-RPC requests go out as POST (the
//! response is either a plain JSON body or an SSE stream carrying the
//! response), the server assigns an `Mcp-Session-Id` during `initialize`
//! that every later request echoes back, and an optional long-lived GET SSE
//! stream carries server-initiated notifications (tools/list_changed). A 404
//! means the session died; the client re-initializes once and replays the
//! request before giving up.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde_json::{json, Value};

use super::config::{expand_env_refs, McpServerConfig};
use super::types::{extract_response_payload, JsonRpcNotification, JsonRpcRequest};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const NOTIFY_STREAM_MAX_RETRIES: u32 = 6;
const NOTIFY_STREAM_BASE_DELAY: Duration = Duration::from_secs(2);
const SESSION_DELETE_TIMEOUT: Duration = Duration::from_millis(1_500);
const ERROR_BODY_MAX: usize = 600;

pub struct HttpMcpClient {
    server_id: String,
    url: String,
    /// Custom headers with `${VAR}` references already expanded.
    headers: Vec<(String, String)>,
    http: reqwest::Client,
    session_id: Arc<Mutex<Option<String>>>,
    protocol_version: Arc<Mutex<Option<String>>>,
    next_id: AtomicI64,
    /// Session is unrecoverable (404 that re-initialize could not heal, or
    /// repeated transport failure); the manager lazily reconnects on the
    /// next tool call, mirroring the stdio has_exited path.
    dead: Arc<AtomicBool>,
    /// Deliberate shutdown: silences the notification stream and blocks
    /// further self-healing.
    closed: Arc<AtomicBool>,
    notify_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

enum PostOutcome {
    /// Full JSON-RPC envelope whose id matched the request.
    Envelope(Value),
    /// 202 Accepted — expected for notifications.
    Accepted,
    /// 404 — the session id is no longer valid.
    SessionExpired,
}

impl HttpMcpClient {
    pub fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let url = config.url.trim().to_string();
        if url.is_empty() {
            return Err("HTTP server URL is empty".to_string());
        }
        let http = crate::network::reqwest_client(
            crate::network::ReqwestClientOptions::new()
                .connect_timeout(CONNECT_TIMEOUT)
                .tcp_keepalive(Duration::from_secs(30)),
        )
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        let headers = config
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), expand_env_refs(v)))
            .collect();
        Ok(Self {
            server_id: config.id.clone(),
            url,
            headers,
            http,
            session_id: Arc::new(Mutex::new(None)),
            protocol_version: Arc::new(Mutex::new(None)),
            next_id: AtomicI64::new(1),
            dead: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
            notify_task: Mutex::new(None),
        })
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Marks the session unusable (ping keepalive found it wedged); the
    /// manager lazily reconnects on the next call.
    pub fn mark_dead(&self) {
        self.dead.store(true, Ordering::SeqCst);
    }

    fn apply_common_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        if let Some(session) = self
            .session_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            req = req.header("Mcp-Session-Id", session);
        }
        if let Some(version) = self
            .protocol_version
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            req = req.header("MCP-Protocol-Version", version);
        }
        req
    }

    /// POSTs one JSON-RPC message. `expect_id` selects the response envelope
    /// out of a JSON body or an SSE response stream; unrelated messages on
    /// that stream (notifications, server pings) are dispatched on the fly.
    async fn post_message(
        &self,
        body: &Value,
        expect_id: Option<i64>,
        capture_session: bool,
    ) -> Result<PostOutcome, String> {
        let req = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(body);
        let response = self
            .apply_common_headers(req)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(PostOutcome::SessionExpired);
        }
        if capture_session {
            if let Some(session) = header_string(&response, "mcp-session-id") {
                *self.session_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(session);
            }
        }
        if status.as_u16() == 202 {
            return Ok(PostOutcome::Accepted);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "HTTP {status} from MCP server: {}",
                truncate_for_error(&body)
            ));
        }

        let content_type = header_string(&response, "content-type").unwrap_or_default();
        if content_type.contains("text/event-stream") {
            let Some(expect_id) = expect_id else {
                // Notification answered with a stream: nothing to wait for.
                return Ok(PostOutcome::Accepted);
            };
            return self.read_sse_response(response, expect_id).await;
        }

        let text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read MCP response body: {e}"))?;
        if expect_id.is_none() {
            return Ok(PostOutcome::Accepted);
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            format!(
                "MCP server returned invalid JSON ({e}): {}",
                truncate_for_error(&text)
            )
        })?;
        Ok(PostOutcome::Envelope(value))
    }

    /// Reads a POST-response SSE stream until the envelope answering
    /// `expect_id` arrives. Interleaved notifications are dispatched.
    async fn read_sse_response(
        &self,
        response: reqwest::Response,
        expect_id: i64,
    ) -> Result<PostOutcome, String> {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("MCP response stream failed: {e}"))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(event) = take_sse_event(&mut buffer) {
                let Some(value) = parse_sse_data(&event) else {
                    continue;
                };
                if envelope_matches_id(&value, expect_id) {
                    return Ok(PostOutcome::Envelope(value));
                }
                self.dispatch_stream_message(value);
            }
        }
        Err("MCP response stream ended without a response".to_string())
    }

    /// Handles non-response messages seen on any SSE stream: notifications
    /// go to the manager, server ping requests get an empty result back.
    fn dispatch_stream_message(&self, value: Value) {
        dispatch_server_message(
            &self.server_id,
            value,
            &self.http,
            &self.url,
            &self.headers,
            &self.session_id,
            &self.protocol_version,
        );
    }

    async fn re_initialize(&self) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("MCP client is shut down".to_string());
        }
        *self.session_id.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::to_value(JsonRpcRequest::new(
            id,
            "initialize",
            Some(super::types::initialize_params()),
        ))
        .map_err(|e| e.to_string())?;
        match self.post_message(&body, Some(id), true).await? {
            PostOutcome::Envelope(envelope) => {
                let result = extract_response_payload(&envelope)?;
                self.store_negotiated_version(&result);
            }
            PostOutcome::SessionExpired => {
                return Err("MCP server answered initialize with 404".to_string());
            }
            PostOutcome::Accepted => {
                return Err("MCP server accepted initialize without a response".to_string());
            }
        }
        let note = serde_json::to_value(JsonRpcNotification::new("notifications/initialized", None))
            .map_err(|e| e.to_string())?;
        let _ = self.post_message(&note, None, false).await?;
        Ok(())
    }

    fn store_negotiated_version(&self, init_result: &Value) {
        if let Some(version) = init_result.get("protocolVersion").and_then(Value::as_str) {
            *self
                .protocol_version
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(version.to_string());
        }
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        let body = serde_json::to_value(JsonRpcNotification::new(method, params))
            .map_err(|e| e.to_string())?;
        match self.post_message(&body, None, false).await? {
            PostOutcome::SessionExpired => Err("MCP session expired".to_string()),
            _ => Ok(()),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.request_cancellable(method, params, timeout, None).await
    }

    /// Sends one request. On session expiry (404) the client re-initializes
    /// once and replays the request; cancellation posts
    /// `notifications/cancelled` best-effort and returns the shared
    /// cancellation marker.
    pub async fn request_cancellable(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<Value, String> {
        if self.is_dead() {
            return Err("MCP session is disconnected".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let is_initialize = method == "initialize";
        let body = serde_json::to_value(JsonRpcRequest::new(id, method, params))
            .map_err(|e| e.to_string())?;

        let work = async {
            let outcome = self.post_message(&body, Some(id), is_initialize).await?;
            let envelope = match outcome {
                PostOutcome::Envelope(envelope) => envelope,
                PostOutcome::Accepted => {
                    return Err(format!(
                        "MCP server accepted '{method}' without returning a response"
                    ));
                }
                PostOutcome::SessionExpired => {
                    if is_initialize {
                        return Err("MCP server answered initialize with 404".to_string());
                    }
                    eprintln!(
                        "[Mcp:{}] session expired (404); re-initializing",
                        self.server_id
                    );
                    self.re_initialize().await.map_err(|e| {
                        self.dead.store(true, Ordering::SeqCst);
                        format!("MCP session expired and could not be recovered: {e}")
                    })?;
                    match self.post_message(&body, Some(id), false).await? {
                        PostOutcome::Envelope(envelope) => envelope,
                        _ => {
                            self.dead.store(true, Ordering::SeqCst);
                            return Err(
                                "MCP session expired and the replayed request failed".to_string()
                            );
                        }
                    }
                }
            };
            let result = extract_response_payload(&envelope)?;
            if is_initialize {
                self.store_negotiated_version(&result);
                self.ensure_notify_stream();
            }
            Ok(result)
        };
        tokio::pin!(work);

        let cancel_wait = async {
            match cancel {
                Some(mut rx) => {
                    let _ = rx.changed().await;
                }
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            outcome = tokio::time::timeout(timeout, &mut work) => match outcome {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "MCP request '{method}' timed out after {}s",
                    timeout.as_secs()
                )),
            },
            _ = cancel_wait => {
                self.spawn_cancel_notification(id);
                Err(super::types::MCP_CALL_CANCELLED.to_string())
            }
        }
    }

    /// Fire-and-forget `notifications/cancelled` for an abandoned request.
    fn spawn_cancel_notification(&self, request_id: i64) {
        let http = self.http.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let session = self
            .session_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let version = self
            .protocol_version
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        tauri::async_runtime::spawn(async move {
            let body = json!({
                "jsonrpc": super::types::JSONRPC_VERSION,
                "method": "notifications/cancelled",
                "params": { "requestId": request_id, "reason": "User requested cancellation" },
            });
            let mut req = http
                .post(&url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .json(&body);
            for (key, value) in &headers {
                req = req.header(key, value);
            }
            if let Some(session) = session {
                req = req.header("Mcp-Session-Id", session);
            }
            if let Some(version) = version {
                req = req.header("MCP-Protocol-Version", version);
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), req.send()).await;
        });
    }

    /// Opens the long-lived GET SSE stream for server-initiated messages.
    /// Servers that do not offer one (405/404) are left alone; a stream that
    /// drops reconnects with exponential backoff (2s doubling, 6 attempts,
    /// counter reset by every successful connection).
    fn ensure_notify_stream(&self) {
        let mut guard = self.notify_task.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(task) = guard.as_ref() {
            if !task.inner().is_finished() {
                return;
            }
        }
        let server_id = self.server_id.clone();
        let http = self.http.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let session_id = self.session_id.clone();
        let protocol_version = self.protocol_version.clone();
        let dead = self.dead.clone();
        let closed = self.closed.clone();
        *guard = Some(tauri::async_runtime::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                if closed.load(Ordering::SeqCst) || dead.load(Ordering::SeqCst) {
                    return;
                }
                let mut req = http.get(&url).header("Accept", "text/event-stream");
                for (key, value) in &headers {
                    req = req.header(key, value);
                }
                if let Some(session) = session_id
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                {
                    req = req.header("Mcp-Session-Id", session);
                }
                if let Some(version) = protocol_version
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                {
                    req = req.header("MCP-Protocol-Version", version);
                }
                match req.send().await {
                    Ok(response) if response.status().is_success() => {
                        attempt = 0;
                        let mut stream = response.bytes_stream();
                        let mut buffer = String::new();
                        while let Some(chunk) = stream.next().await {
                            if closed.load(Ordering::SeqCst) {
                                return;
                            }
                            let Ok(chunk) = chunk else { break };
                            buffer.push_str(&String::from_utf8_lossy(&chunk));
                            while let Some(event) = take_sse_event(&mut buffer) {
                                if let Some(value) = parse_sse_data(&event) {
                                    dispatch_server_message(
                                        &server_id,
                                        value,
                                        &http,
                                        &url,
                                        &headers,
                                        &session_id,
                                        &protocol_version,
                                    );
                                }
                            }
                        }
                        // Stream dropped; fall through to backoff.
                    }
                    Ok(response) => {
                        let status = response.status().as_u16();
                        if matches!(status, 404 | 405 | 401 | 403) {
                            // Server does not expose a notification stream —
                            // a perfectly valid Streamable HTTP setup.
                            return;
                        }
                    }
                    Err(_) => {}
                }
                if attempt >= NOTIFY_STREAM_MAX_RETRIES {
                    eprintln!(
                        "[Mcp:{server_id}] notification stream gave up after {NOTIFY_STREAM_MAX_RETRIES} reconnect attempts"
                    );
                    return;
                }
                let delay = NOTIFY_STREAM_BASE_DELAY * 2u32.saturating_pow(attempt);
                attempt += 1;
                tokio::time::sleep(delay).await;
            }
        }));
    }

    /// Marks the client closed and aborts the notification stream; safe to
    /// call from synchronous exit paths.
    pub fn kill_for_exit(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.dead.store(true, Ordering::SeqCst);
        if let Some(task) = self
            .notify_task
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            task.abort();
        }
    }

    /// Graceful teardown: stop background work, then best-effort DELETE the
    /// session so the server can reclaim it (405 from servers that do not
    /// support explicit termination is fine).
    pub async fn shutdown(&self) {
        self.kill_for_exit();
        let session = self
            .session_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let Some(session) = session else {
            return;
        };
        let mut req = self.http.delete(&self.url).header("Mcp-Session-Id", session);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        let _ = tokio::time::timeout(SESSION_DELETE_TIMEOUT, req.send()).await;
    }
}

/// Routes one server-initiated JSON-RPC message: notifications go to the
/// manager (tools/list_changed drives the reconcile), ping requests get an
/// empty result POSTed back, anything else is ignored.
fn dispatch_server_message(
    server_id: &str,
    value: Value,
    http: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    session_id: &Arc<Mutex<Option<String>>>,
    protocol_version: &Arc<Mutex<Option<String>>>,
) {
    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id").cloned().filter(|v| !v.is_null());
    match (method, id) {
        (Some(method), None) => {
            super::manager::handle_server_notification(server_id, method, value.get("params"));
        }
        (Some("ping"), Some(id)) => {
            let http = http.clone();
            let url = url.to_string();
            let headers = headers.to_vec();
            let session = session_id.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let version = protocol_version
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            tauri::async_runtime::spawn(async move {
                let body = json!({
                    "jsonrpc": super::types::JSONRPC_VERSION,
                    "id": id,
                    "result": {},
                });
                let mut req = http
                    .post(&url)
                    .header("Accept", "application/json, text/event-stream")
                    .header("Content-Type", "application/json")
                    .json(&body);
                for (key, value) in &headers {
                    req = req.header(key, value);
                }
                if let Some(session) = session {
                    req = req.header("Mcp-Session-Id", session);
                }
                if let Some(version) = version {
                    req = req.header("MCP-Protocol-Version", version);
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), req.send()).await;
            });
        }
        _ => {}
    }
}

fn header_string(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn truncate_for_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty body)".to_string();
    }
    let mut out: String = trimmed.chars().take(ERROR_BODY_MAX).collect();
    if trimmed.chars().count() > ERROR_BODY_MAX {
        out.push('…');
    }
    out
}

/// Pops one complete SSE event block off the front of `buffer` (LF and CRLF
/// separators both accepted); `None` when no full event is buffered yet.
fn take_sse_event(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|pos| (pos, 2usize));
    let crlf = buffer.find("\r\n\r\n").map(|pos| (pos, 4usize));
    let (pos, sep_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
        (Some(found), None) | (None, Some(found)) => found,
        (None, None) => return None,
    };
    let event = buffer[..pos].to_string();
    buffer.drain(..pos + sep_len);
    Some(event)
}

/// Joins the `data:` lines of one SSE event and parses them as JSON.
fn parse_sse_data(event: &str) -> Option<Value> {
    let mut data = String::new();
    for line in event.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    serde_json::from_str(&data).ok()
}

fn envelope_matches_id(value: &Value, expect_id: i64) -> bool {
    value.get("id").and_then(Value::as_i64) == Some(expect_id)
        && (value.get("result").is_some() || value.get("error").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_sse_event_handles_lf_and_crlf_and_partial() {
        let mut buffer = "event: message\ndata: {\"a\":1}\n\nrest".to_string();
        assert_eq!(
            take_sse_event(&mut buffer).as_deref(),
            Some("event: message\ndata: {\"a\":1}")
        );
        assert_eq!(buffer, "rest");

        let mut crlf = "data: {}\r\n\r\ntail".to_string();
        assert_eq!(take_sse_event(&mut crlf).as_deref(), Some("data: {}"));
        assert_eq!(crlf, "tail");

        let mut partial = "data: {\"a\"".to_string();
        assert!(take_sse_event(&mut partial).is_none());
    }

    #[test]
    fn parse_sse_data_joins_multiline_data() {
        let event = "event: message\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":1}";
        let value = parse_sse_data(event).unwrap();
        assert_eq!(value.get("id").and_then(Value::as_i64), Some(1));
        assert!(parse_sse_data("event: ping").is_none());
    }

    #[test]
    fn envelope_matching_requires_result_or_error() {
        let response = serde_json::json!({"jsonrpc": "2.0", "id": 3, "result": {}});
        assert!(envelope_matches_id(&response, 3));
        assert!(!envelope_matches_id(&response, 4));
        // A server request carrying an id must not be mistaken for the response.
        let request = serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
        assert!(!envelope_matches_id(&request, 3));
    }
}
