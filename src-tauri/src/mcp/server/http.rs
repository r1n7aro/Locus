//! Minimal Streamable HTTP transport for the Locus MCP server.
//!
//! JSON-response mode of the 2025-06-18 spec: every JSON-RPC request arrives
//! as POST /mcp and is answered with a plain application/json body; GET
//! (server-initiated SSE stream) is 405; DELETE ends the session. Bound to
//! 127.0.0.1 only, guarded by a bearer token plus Host/Origin checks
//! (DNS-rebinding hardening). Tauri-free by design: tool execution and the
//! initialize instructions are injected as closures so tests can drive the
//! full HTTP surface without an app handle.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use super::protocol::{self, RpcAction, ToolListing};

const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
const SESSION_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_CONCURRENT_CALLS: usize = 4;

pub struct ToolCallOutcome {
    pub output: String,
    pub is_error: bool,
    /// (base64 data, mime type)
    pub images: Vec<(String, String)>,
    /// Workspace the call actually ran against (None when no workspace was
    /// needed/selected). Drives the session's workspace-change notice.
    pub workspace_path: Option<String>,
}

pub type ToolDispatcher =
    Arc<dyn Fn(String, Value) -> BoxFuture<'static, ToolCallOutcome> + Send + Sync>;
pub type ToolListProvider = Arc<dyn Fn() -> Vec<ToolListing> + Send + Sync>;
pub type InstructionsProvider = Arc<dyn Fn() -> BoxFuture<'static, String> + Send + Sync>;

struct McpSession {
    workspace_path: Option<String>,
    last_seen: Instant,
}

pub struct ServerContext {
    token: String,
    dispatcher: ToolDispatcher,
    list_tools: ToolListProvider,
    instructions: InstructionsProvider,
    sessions: Mutex<HashMap<String, McpSession>>,
    call_slots: Semaphore,
}

impl ServerContext {
    pub fn new(
        token: String,
        dispatcher: ToolDispatcher,
        list_tools: ToolListProvider,
        instructions: InstructionsProvider,
    ) -> Self {
        Self {
            token,
            dispatcher,
            list_tools,
            instructions,
            sessions: Mutex::new(HashMap::new()),
            call_slots: Semaphore::new(MAX_CONCURRENT_CALLS),
        }
    }

    pub fn active_sessions(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    fn open_session(&self) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        sessions.retain(|_, s| s.last_seen.elapsed() < SESSION_TTL);
        sessions.insert(
            id.clone(),
            McpSession {
                workspace_path: None,
                last_seen: Instant::now(),
            },
        );
        id
    }

    fn close_session(&self, id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    /// Returns a notice line when the active Locus workspace changed since
    /// this session's previous tool call, and records the new one.
    fn workspace_notice(
        &self,
        session_id: Option<&str>,
        current: &Option<String>,
    ) -> Option<String> {
        let session_id = session_id?;
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        let session = sessions.get_mut(session_id)?;
        session.last_seen = Instant::now();
        let previous = session.workspace_path.clone();
        session.workspace_path = current.clone();
        match (previous, current) {
            (Some(prev), Some(now)) if prev != *now => Some(format!(
                "[notice] The active Locus workspace changed to '{}' (was '{}'). Call unity_project_info to confirm the target project.\n\n",
                now, prev
            )),
            _ => None,
        }
    }
}

/// Binds 127.0.0.1:`port` (0 = ephemeral) and serves until the returned
/// task is aborted. Returns the actually bound address.
pub async fn start(
    port: u16,
    ctx: Arc<ServerContext>,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("Failed to bind 127.0.0.1:{port}: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("Failed to read bound address: {e}"))?;
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            match listener.accept().await {
                Ok((stream, _peer)) => {
                    let ctx = ctx.clone();
                    connections.spawn(async move {
                        let io = TokioIo::new(stream);
                        let service = service_fn(move |req| {
                            let ctx = ctx.clone();
                            async move { handle_request(req, ctx).await }
                        });
                        if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                            // Connection-level errors (client vanished) are routine.
                            eprintln!("[McpServer] connection ended: {e}");
                        }
                    });
                    // Reap finished connection tasks without blocking accept.
                    while connections.try_join_next().is_some() {}
                }
                Err(e) => {
                    eprintln!("[McpServer] accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    });
    Ok((addr, task))
}

fn plain_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("static response builds")
}

fn json_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("json response builds")
}

/// Constant-time token comparison (length leak is fine: tokens are fixed length).
fn token_matches(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn host_allowed(host: Option<&str>) -> bool {
    let Some(host) = host else { return false };
    let bare = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    matches!(bare, "127.0.0.1" | "localhost")
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    ctx: Arc<ServerContext>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    Ok(handle_request_inner(req, ctx).await)
}

async fn handle_request_inner(
    req: Request<hyper::body::Incoming>,
    ctx: Arc<ServerContext>,
) -> Response<Full<Bytes>> {
    if req.uri().path() != "/mcp" {
        return plain_response(StatusCode::NOT_FOUND, "not found");
    }
    // Browser-driven requests always carry Origin; no legitimate MCP harness
    // does. Rejecting them (plus the Host check) kills DNS-rebinding attacks.
    if req.headers().get("origin").is_some() {
        return plain_response(StatusCode::FORBIDDEN, "browser origins are not allowed");
    }
    let host = req.headers().get("host").and_then(|v| v.to_str().ok());
    if !host_allowed(host) {
        return plain_response(StatusCode::FORBIDDEN, "invalid host");
    }
    let authorized = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let v = v.trim();
            let (scheme, rest) = v.split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| rest.trim().to_string())
        })
        .map(|token| token_matches(&ctx.token, &token))
        .unwrap_or(false);
    if !authorized {
        return plain_response(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
    }

    let session_id = req
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let method = req.method().clone();
    if method == Method::DELETE {
        if let Some(id) = session_id.as_deref() {
            ctx.close_session(id);
        }
        return plain_response(StatusCode::NO_CONTENT, "");
    }
    if method == Method::GET {
        // No server-initiated notification stream in v1.
        return plain_response(StatusCode::METHOD_NOT_ALLOWED, "SSE stream not supported");
    }
    if method != Method::POST {
        return plain_response(StatusCode::METHOD_NOT_ALLOWED, "unsupported method");
    }

    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            return plain_response(
                StatusCode::BAD_REQUEST,
                &format!("failed to read body: {e}"),
            )
        }
    };
    if body.len() > MAX_BODY_BYTES {
        return plain_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(e) => {
            return json_response(
                StatusCode::OK,
                &protocol::error_response(
                    &Value::Null,
                    protocol::ERR_PARSE,
                    &format!("invalid JSON: {e}"),
                ),
            )
        }
    };

    match protocol::parse_message(&message) {
        RpcAction::Initialize {
            id,
            requested_version,
        } => {
            let instructions = (ctx.instructions)().await;
            let new_session = ctx.open_session();
            let value =
                protocol::initialize_response(&id, requested_version.as_deref(), &instructions);
            let mut response = json_response(StatusCode::OK, &value);
            if let Ok(header_value) = new_session.parse() {
                response
                    .headers_mut()
                    .insert("mcp-session-id", header_value);
            }
            response
        }
        RpcAction::Notification => plain_response(StatusCode::ACCEPTED, ""),
        RpcAction::Ping { id } => json_response(StatusCode::OK, &protocol::pong_response(&id)),
        RpcAction::ToolsList { id } => {
            let tools = (ctx.list_tools)();
            json_response(StatusCode::OK, &protocol::tools_list_response(&id, &tools))
        }
        RpcAction::ToolsCall {
            id,
            name,
            arguments,
        } => {
            let known = (ctx.list_tools)().iter().any(|tool| tool.name == name);
            if !known {
                return json_response(
                    StatusCode::OK,
                    &protocol::error_response(
                        &id,
                        protocol::ERR_INVALID_PARAMS,
                        &format!("Unknown or disabled tool '{name}'"),
                    ),
                );
            }
            let _slot = match ctx.call_slots.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    return plain_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down")
                }
            };
            let mut outcome = (ctx.dispatcher)(name, arguments).await;
            if let Some(notice) =
                ctx.workspace_notice(session_id.as_deref(), &outcome.workspace_path)
            {
                outcome.output = format!("{notice}{}", outcome.output);
            }
            json_response(
                StatusCode::OK,
                &protocol::tool_result_response(
                    &id,
                    &outcome.output,
                    outcome.is_error,
                    &outcome.images,
                ),
            )
        }
        RpcAction::UnknownMethod { id, method } => json_response(
            StatusCode::OK,
            &protocol::error_response(
                &id,
                protocol::ERR_METHOD_NOT_FOUND,
                &format!("Unsupported MCP method '{method}'"),
            ),
        ),
        RpcAction::Invalid => json_response(
            StatusCode::OK,
            &protocol::error_response(
                &Value::Null,
                protocol::ERR_INVALID_REQUEST,
                "Expected a single JSON-RPC request object",
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_is_exact() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc12"));
        assert!(!token_matches("abc123", ""));
    }

    #[test]
    fn host_check_accepts_loopback_names_only() {
        assert!(host_allowed(Some("127.0.0.1:27121")));
        assert!(host_allowed(Some("localhost:27121")));
        assert!(host_allowed(Some("localhost")));
        assert!(!host_allowed(Some("evil.example:27121")));
        assert!(!host_allowed(Some("192.168.1.4:27121")));
        assert!(!host_allowed(None));
    }
}
