use super::openai_reasoning::{apply_reasoning_effort, apply_text_verbosity_default};
use super::openrouter::LlmResponse;
use super::CODEX_CLIENT_VERSION;
use crate::commands::CodexTransportMode;
use crate::session::models::{ChatMessage, ImageData, MessageRole, ServerToolKind, ToolCallInfo};
use futures::{SinkExt, StreamExt};
use http::Uri;
use hyper_util::client::legacy::connect::proxy::{SocksV4, SocksV5, Tunnel};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio_native_tls::TlsConnector as TokioTlsConnector;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use tower_service::Service;
use url::Url;

const DEFAULT_CODEX_PROVIDER_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const RESPONSES_ENDPOINT_PATH: &str = "/responses";
const RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";
const WEBSOCKET_CONNECTION_LIMIT_REACHED_CODE: &str = "websocket_connection_limit_reached";
const WEBSOCKET_CONNECTION_LIMIT_REACHED_MESSAGE: &str =
    "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue.";
const CODEX_ORIGINATOR_HEADER_VALUE: &str = "opencode";
const MAX_SAFE_STREAM_RECOVERY_RETRIES: u32 = 2;
const SAFE_STREAM_RECOVERY_DELAY_MS: u64 = 1200;

trait CodexAsyncIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

impl<T> CodexAsyncIo for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

type BoxedCodexIo = Box<dyn CodexAsyncIo>;
type CodexWebSocket = tokio_tungstenite::WebSocketStream<BoxedCodexIo>;

#[derive(Debug, Default)]
pub struct TurnState {
    sticky_routing_token: Option<String>,
}

#[derive(Debug, Clone)]
struct LastWebsocketResponse {
    request_signature: serde_json::Value,
    input: Vec<serde_json::Value>,
    response_id: String,
    items_added: Vec<serde_json::Value>,
}

#[derive(Default)]
struct CachedWebsocketSession {
    connection: Option<CodexWebSocket>,
    last_response: Option<LastWebsocketResponse>,
    disable_websockets: bool,
    connection_key: Option<String>,
}

type SharedCachedWebsocketSession = Arc<tokio::sync::Mutex<CachedWebsocketSession>>;

fn cached_websocket_sessions() -> &'static StdMutex<HashMap<String, SharedCachedWebsocketSession>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, SharedCachedWebsocketSession>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn cached_websocket_session(session_id: &str) -> SharedCachedWebsocketSession {
    let mut sessions = cached_websocket_sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(CachedWebsocketSession::default())))
        .clone()
}

fn existing_cached_websocket_session(session_id: &str) -> Option<SharedCachedWebsocketSession> {
    let sessions = cached_websocket_sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions.get(session_id).cloned()
}

pub fn invalidate_cached_session(session_id: &str) {
    let mut sessions = cached_websocket_sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions.remove(session_id);
}

pub async fn reset_cached_session_window(session_id: &str) {
    let Some(shared) = existing_cached_websocket_session(session_id) else {
        return;
    };
    let mut state = shared.lock().await;
    state.connection = None;
    state.last_response = None;
}

fn websocket_connection_key(base_url: Option<&str>, account_id: Option<&str>) -> String {
    format!(
        "{}|{}",
        codex_responses_endpoint(base_url),
        account_id.unwrap_or_default().trim()
    )
}

impl TurnState {
    fn header_value(&self) -> Option<&str> {
        self.sticky_routing_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn store_header(&mut self, turn_state: Option<&str>) {
        let next = turn_state
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if next.is_some() {
            self.sticky_routing_token = next;
        }
    }
}

fn authority_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn build_input(history: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut input = Vec::new();
    for msg in history {
        match msg.role {
            MessageRole::User => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": build_user_input_content(&msg.content, msg.images.as_deref())
                }));
            }
            MessageRole::Assistant => {
                if !msg.content.is_empty() {
                    input.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": msg.content }]
                    }));
                }
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }));
                        if let Some(output) = tc.server_tool_output.as_deref() {
                            input.push(serde_json::json!({
                                "type": "function_call_output",
                                "call_id": tc.id,
                                "output": output,
                            }));
                        }
                    }
                }
            }
            MessageRole::Tool => {
                if let Some(ref call_id) = msg.tool_call_id {
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": msg.content,
                    }));
                }
            }
        }
    }
    input
}

/// Chat Completions: { type:"function", function:{ name, description, parameters } }
/// Responses API:    { type:"function", name, description, parameters }
fn build_user_input_content(text: &str, images: Option<&[ImageData]>) -> Vec<serde_json::Value> {
    let mut content = Vec::new();

    if let Some(images) = images {
        for img in images {
            content.push(serde_json::json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", img.mime_type, img.data),
            }));
        }
    }

    if !text.is_empty() {
        content.push(serde_json::json!({
            "type": "input_text",
            "text": text,
        }));
    }

    if content.is_empty() {
        content.push(serde_json::json!({
            "type": "input_text",
            "text": "",
        }));
    }

    content
}

fn convert_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .filter_map(|tool| {
            if tool.get("type")?.as_str()? != "function" {
                return None;
            }
            let func = tool.get("function")?;
            Some(serde_json::json!({
                "type": "function",
                "name": func.get("name").cloned().unwrap_or(serde_json::Value::Null),
                "description": func.get("description").cloned().unwrap_or(serde_json::Value::Null),
                "parameters": func.get("parameters").cloned().unwrap_or(serde_json::Value::Null),
            }))
        })
        .collect()
}

fn build_request_body(
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    session_id: Option<&str>,
) -> serde_json::Value {
    let input = build_input(history);
    let mut responses_tools = convert_tools(tools);

    // Inject web_search server tool (executed by OpenAI API, not locally).
    responses_tools.push(serde_json::json!({
        "type": "web_search",
        "external_web_access": true,
    }));

    let mut body = serde_json::json!({
        "model": model,
        "input": input,
        "stream": true,
        "store": false,
    });

    if let Some(sid) = session_id {
        body["prompt_cache_key"] = serde_json::json!(sid);
    }

    if !system_prompt.is_empty() {
        body["instructions"] = serde_json::json!(system_prompt);
    }

    apply_reasoning_effort(&mut body, model, thinking_level);
    apply_text_verbosity_default(&mut body, model);

    if !responses_tools.is_empty() {
        body["tools"] = serde_json::json!(responses_tools);
        body["tool_choice"] = serde_json::json!("auto");
    }

    body
}

fn request_without_input(body: &serde_json::Value) -> serde_json::Value {
    let mut request = body.clone();
    if let Some(map) = request.as_object_mut() {
        map.remove("input");
        map.remove("previous_response_id");
        map.remove("type");
    }
    request
}

struct ContinuationRequestInput {
    input: Vec<serde_json::Value>,
    previous_response_id: Option<String>,
}

fn build_history_request_input(
    body: &serde_json::Value,
    history: &[ChatMessage],
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
) -> ContinuationRequestInput {
    let full_input = body
        .get("input")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_else(|| build_input(history));
    let current_request = request_without_input(body);

    if let Some(response_request_metadata) = response_request_metadata {
        for index in (0..history.len()).rev() {
            let message = &history[index];
            if message.role != MessageRole::Assistant {
                continue;
            }
            let Some(response_id) = message
                .response_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            else {
                continue;
            };
            let Some(previous_request) = response_request_metadata.get(&message.id) else {
                continue;
            };
            if previous_request != &current_request {
                continue;
            }

            let baseline = build_input(&history[..=index]);
            if full_input.starts_with(&baseline) {
                return ContinuationRequestInput {
                    input: full_input[baseline.len()..].to_vec(),
                    previous_response_id: Some(response_id),
                };
            }
        }
    }

    ContinuationRequestInput {
        input: full_input,
        previous_response_id: None,
    }
}

fn build_cached_websocket_request_input(
    body: &serde_json::Value,
    last_response: Option<&LastWebsocketResponse>,
) -> ContinuationRequestInput {
    let full_input = body
        .get("input")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let current_request = request_without_input(body);

    if let Some(last_response) = last_response {
        if last_response.request_signature == current_request {
            if last_response.response_id.trim().is_empty() {
                return ContinuationRequestInput {
                    input: full_input,
                    previous_response_id: None,
                };
            }
            let mut baseline = last_response.input.clone();
            baseline.extend(last_response.items_added.clone());
            if full_input.starts_with(&baseline) {
                return ContinuationRequestInput {
                    input: full_input[baseline.len()..].to_vec(),
                    previous_response_id: Some(last_response.response_id.clone()),
                };
            }
        }
    }

    ContinuationRequestInput {
        input: full_input,
        previous_response_id: None,
    }
}

fn apply_transport_request_input(
    body: &serde_json::Value,
    request_input: ContinuationRequestInput,
    include_type_field: bool,
) -> serde_json::Value {
    let mut request = body.clone();
    if let Some(map) = request.as_object_mut() {
        map.insert("input".to_string(), serde_json::json!(request_input.input));
        if let Some(previous_response_id) = request_input.previous_response_id {
            map.insert(
                "previous_response_id".to_string(),
                serde_json::Value::String(previous_response_id),
            );
        } else {
            map.remove("previous_response_id");
        }
        if include_type_field {
            map.insert(
                "type".to_string(),
                serde_json::Value::String("response.create".to_string()),
            );
        } else {
            map.remove("type");
        }
    }
    request
}

fn build_history_transport_request(
    body: &serde_json::Value,
    history: &[ChatMessage],
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    include_type_field: bool,
) -> serde_json::Value {
    let request_input = build_history_request_input(body, history, response_request_metadata);
    apply_transport_request_input(body, request_input, include_type_field)
}

fn build_websocket_transport_request(
    body: &serde_json::Value,
    last_response: Option<&LastWebsocketResponse>,
    include_type_field: bool,
) -> serde_json::Value {
    let request_input = build_cached_websocket_request_input(body, last_response);
    apply_transport_request_input(body, request_input, include_type_field)
}

fn codex_provider_base_url(base_url: Option<&str>) -> &str {
    base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_PROVIDER_BASE_URL)
}

fn codex_responses_endpoint(base_url: Option<&str>) -> String {
    let base_url = codex_provider_base_url(base_url).trim_end_matches('/');
    if base_url.ends_with(RESPONSES_ENDPOINT_PATH) {
        base_url.to_string()
    } else {
        format!("{base_url}{RESPONSES_ENDPOINT_PATH}")
    }
}

fn codex_websocket_url(base_url: Option<&str>) -> Result<Url, String> {
    let mut url = Url::parse(&codex_responses_endpoint(base_url))
        .map_err(|e| format!("Failed to parse websocket endpoint: {}", e))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" | "wss" => return Ok(url),
        other => {
            return Err(format!(
                "Unsupported websocket endpoint scheme for Codex transport: {}",
                other
            ));
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| "Failed to convert websocket endpoint scheme".to_string())?;
    Ok(url)
}

fn build_codex_websocket_handshake_request(
    ws_url: &Url,
    access_token: &str,
    account_id: Option<&str>,
    session_id: Option<&str>,
    turn_state: Option<&str>,
) -> Result<http::Request<()>, String> {
    let mut request = ws_url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("Failed to build websocket request: {}", e))?;
    request.headers_mut().insert(
        "Authorization",
        http::HeaderValue::from_str(&format!("Bearer {}", access_token))
            .map_err(|e| format!("Failed to build authorization header: {}", e))?,
    );
    request.headers_mut().insert(
        "Content-Type",
        http::HeaderValue::from_static("application/json"),
    );
    request.headers_mut().insert(
        "OpenAI-Beta",
        http::HeaderValue::from_static(RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE),
    );
    request.headers_mut().insert(
        "originator",
        http::HeaderValue::from_static(CODEX_ORIGINATOR_HEADER_VALUE),
    );
    request.headers_mut().insert(
        "version",
        http::HeaderValue::from_static(CODEX_CLIENT_VERSION),
    );
    if let Some(turn_state) = turn_state {
        request.headers_mut().insert(
            X_CODEX_TURN_STATE_HEADER,
            http::HeaderValue::from_str(turn_state)
                .map_err(|e| format!("Failed to build turn-state header: {}", e))?,
        );
    }
    if let Some(sid) = session_id {
        let header_value = http::HeaderValue::from_str(sid)
            .map_err(|e| format!("Failed to build session header: {}", e))?;
        request
            .headers_mut()
            .insert("x-client-request-id", header_value.clone());
        request.headers_mut().insert("session_id", header_value);
    }
    if let Some(aid) = account_id {
        request.headers_mut().insert(
            "ChatGPT-Account-ID",
            http::HeaderValue::from_str(aid)
                .map_err(|e| format!("Failed to build account header: {}", e))?,
        );
    }

    Ok(request)
}

async fn take_cached_websocket_session_state(
    session_id: &str,
    connection_key: &str,
) -> (
    SharedCachedWebsocketSession,
    Option<CodexWebSocket>,
    Option<LastWebsocketResponse>,
    bool,
) {
    let shared = cached_websocket_session(session_id);
    let mut state = shared.lock().await;
    if state.connection_key.as_deref() != Some(connection_key) {
        state.connection = None;
        state.last_response = None;
        state.disable_websockets = false;
        state.connection_key = Some(connection_key.to_string());
    }

    let socket = state.connection.take();
    let last_response = if socket.is_some() {
        state.last_response.clone()
    } else {
        state.last_response = None;
        None
    };
    let disable_websockets = state.disable_websockets;
    drop(state);
    (shared, socket, last_response, disable_websockets)
}

async fn store_cached_websocket_session_state(
    shared: &SharedCachedWebsocketSession,
    connection_key: &str,
    socket: CodexWebSocket,
    last_response: LastWebsocketResponse,
) {
    let mut state = shared.lock().await;
    state.connection = Some(socket);
    state.last_response = Some(last_response);
    state.disable_websockets = false;
    state.connection_key = Some(connection_key.to_string());
}

async fn clear_cached_websocket_session_state(
    shared: &SharedCachedWebsocketSession,
    connection_key: &str,
    disable_websockets: bool,
) {
    let mut state = shared.lock().await;
    state.connection = None;
    state.last_response = None;
    state.disable_websockets = disable_websockets;
    state.connection_key = Some(connection_key.to_string());
}

fn websocket_proxy_match_uri(uri: &Uri) -> Result<Uri, String> {
    let scheme = match uri.scheme_str() {
        Some("ws") => "http",
        Some("wss") => "https",
        Some(other) => {
            return Err(format!(
                "Unsupported websocket endpoint scheme for proxy matching: {}",
                other
            ));
        }
        None => return Err("Websocket endpoint is missing a scheme".to_string()),
    };

    let authority = uri
        .authority()
        .cloned()
        .ok_or_else(|| "Websocket endpoint is missing an authority".to_string())?;
    let path_and_query = uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    http::Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to build proxy match URI: {}", e))
}

fn uri_host_port(uri: &Uri) -> Result<(String, u16), String> {
    let host = uri
        .host()
        .ok_or_else(|| "URI is missing host".to_string())?
        .to_string();
    let port = match uri.port_u16() {
        Some(port) => port,
        None => match uri.scheme_str() {
            Some("http") | Some("ws") => 80,
            Some("https") | Some("wss") => 443,
            Some("socks4") | Some("socks4a") | Some("socks5") | Some("socks5h") => 1080,
            Some(other) => {
                return Err(format!(
                    "Unsupported URI scheme for port resolution: {}",
                    other
                ));
            }
            None => return Err("URI is missing a scheme".to_string()),
        },
    };
    Ok((host, port))
}

fn uri_with_resolved_port(uri: &Uri) -> Result<Uri, String> {
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| "URI is missing a scheme".to_string())?;
    let (host, port) = uri_host_port(uri)?;
    let authority = match uri.authority() {
        Some(authority) if authority.as_str().contains('@') => {
            let userinfo = authority
                .as_str()
                .split('@')
                .next()
                .ok_or_else(|| "URI authority is invalid".to_string())?;
            format!("{userinfo}@{}:{}", authority_host(&host), port)
        }
        _ => format!("{}:{}", authority_host(&host), port),
    };
    let path_and_query = uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    http::Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to normalize URI port: {}", e))
}

fn build_tcp_connector() -> HttpConnector {
    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    connector.set_connect_timeout(Some(Duration::from_secs(30)));
    connector.set_keepalive(Some(Duration::from_secs(20)));
    connector.set_keepalive_interval(Some(Duration::from_secs(15)));
    connector.set_keepalive_retries(Some(3));
    connector.set_nodelay(true);
    connector
}

fn tls_connector() -> Result<TokioTlsConnector, String> {
    let connector = native_tls::TlsConnector::new()
        .map_err(|e| format!("Failed to create TLS connector: {}", e))?;
    Ok(TokioTlsConnector::from(connector))
}

fn ws_io_error(message: impl Into<String>) -> WsError {
    WsError::Io(io::Error::other(message.into()))
}

fn proxy_display_uri(uri: &Uri) -> String {
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host().unwrap_or("<missing-host>");
    match uri.port_u16() {
        Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
        None => format!("{}://{}", scheme, authority_host(host)),
    }
}

async fn connect_tcp_stream(uri: &Uri) -> Result<tokio::net::TcpStream, String> {
    let normalized_uri = uri_with_resolved_port(uri)?;
    let mut connector = build_tcp_connector();
    let connection = connector
        .call(normalized_uri.clone())
        .await
        .map_err(|e| format!("Failed to connect to {}: {}", normalized_uri, e))?;
    Ok(connection.into_inner())
}

async fn establish_http_connect_tunnel<S>(
    mut stream: S,
    host: &str,
    port: u16,
    proxy_auth: Option<&http::HeaderValue>,
) -> Result<S, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(auth) = proxy_auth {
        request.push_str("Proxy-Authorization: ");
        request.push_str(auth.to_str().unwrap_or_default());
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes())
        .await
        .map_err(|e| format!("Failed to send proxy CONNECT request: {}", e))?;

    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        let n = tokio::io::AsyncReadExt::read(&mut stream, &mut chunk)
            .await
            .map_err(|e| format!("Failed to read proxy CONNECT response: {}", e))?;
        if n == 0 {
            return Err("Proxy CONNECT response ended unexpectedly".to_string());
        }
        response.extend_from_slice(&chunk[..n]);
        if response.len() > 8192 {
            return Err("Proxy CONNECT response headers exceeded 8 KiB".to_string());
        }
    }

    if response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.0 200") {
        return Ok(stream);
    }
    if response.starts_with(b"HTTP/1.1 407") || response.starts_with(b"HTTP/1.0 407") {
        return Err("Proxy requires authentication for websocket CONNECT".to_string());
    }

    let status_line = response
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_string())
        .unwrap_or_else(|| "unknown proxy response".to_string());
    Err(format!("Proxy CONNECT failed: {}", status_line))
}

async fn connect_via_http_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let mut tunnel = Tunnel::new(proxy_uri, build_tcp_connector());
    if let Some(auth) = proxy.basic_auth().cloned() {
        tunnel = tunnel.with_auth(auth);
    }
    let connection = tunnel
        .call(target_uri.clone())
        .await
        .map_err(|e| format!("Failed to establish HTTP proxy tunnel: {}", e))?;
    Ok(Box::new(connection.into_inner()))
}

async fn connect_via_https_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let (proxy_host, _) = uri_host_port(&proxy_uri)?;
    let (target_host, target_port) = uri_host_port(target_uri)?;

    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let proxy_tls = tls_connector()?
        .connect(&proxy_host, tcp)
        .await
        .map_err(|e| format!("Failed to establish TLS to HTTPS proxy: {}", e))?;
    let tunneled =
        establish_http_connect_tunnel(proxy_tls, &target_host, target_port, proxy.basic_auth())
            .await?;

    Ok(Box::new(tunneled))
}

async fn connect_via_socks4_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let mut socks = SocksV4::new(proxy_uri, build_tcp_connector());
    if proxy.uri().scheme_str() == Some("socks4") {
        socks = socks.local_dns(true);
    }
    let connection = socks
        .call(target_uri.clone())
        .await
        .map_err(|e| format!("Failed to establish SOCKS4 proxy tunnel: {}", e))?;
    Ok(Box::new(connection.into_inner()))
}

async fn connect_via_socks5_proxy(
    target_uri: &Uri,
    proxy: &Intercept,
) -> Result<BoxedCodexIo, String> {
    let proxy_uri = uri_with_resolved_port(proxy.uri())?;
    eprintln!(
        "[OpenAI Codex][websocket] SYSTEM_PROXY {}",
        proxy_display_uri(&proxy_uri)
    );

    let mut socks = SocksV5::new(proxy_uri, build_tcp_connector());
    if proxy.uri().scheme_str() == Some("socks5") {
        socks = socks.local_dns(true);
    }
    if let Some((user, pass)) = proxy.raw_auth() {
        socks = socks.with_auth(user.to_string(), pass.to_string());
    }
    let connection = socks
        .call(target_uri.clone())
        .await
        .map_err(|e| format!("Failed to establish SOCKS5 proxy tunnel: {}", e))?;
    Ok(Box::new(connection.into_inner()))
}

async fn connect_websocket_transport(request: &http::Request<()>) -> Result<BoxedCodexIo, String> {
    let target_uri = websocket_proxy_match_uri(request.uri())?;
    let matcher = Matcher::from_system();

    if let Some(proxy) = matcher.intercept(&target_uri) {
        match proxy.uri().scheme_str().unwrap_or("http") {
            "http" => connect_via_http_proxy(&target_uri, &proxy).await,
            "https" => connect_via_https_proxy(&target_uri, &proxy).await,
            "socks4" | "socks4a" => connect_via_socks4_proxy(&target_uri, &proxy).await,
            "socks5" | "socks5h" => connect_via_socks5_proxy(&target_uri, &proxy).await,
            other => Err(format!(
                "Unsupported system proxy scheme for Codex websocket: {}",
                other
            )),
        }
    } else {
        Ok(Box::new(connect_tcp_stream(&target_uri).await?))
    }
}

async fn wrap_websocket_transport_tls(
    request: &http::Request<()>,
    stream: BoxedCodexIo,
) -> Result<BoxedCodexIo, String> {
    match request.uri().scheme_str() {
        Some("wss") => {
            let host = request
                .uri()
                .host()
                .ok_or_else(|| "Websocket endpoint is missing host".to_string())?;
            let tls_stream = tls_connector()?
                .connect(host, stream)
                .await
                .map_err(|e| format!("Failed to establish TLS to websocket endpoint: {}", e))?;
            Ok(Box::new(tls_stream))
        }
        Some("ws") => Ok(stream),
        Some(other) => Err(format!("Unsupported websocket scheme: {}", other)),
        None => Err("Websocket endpoint is missing scheme".to_string()),
    }
}

enum WebsocketConnectOutcome<S> {
    Connected(S),
    FallbackToHttp,
}

async fn connect_codex_websocket(
    request: http::Request<()>,
    turn_state: &mut TurnState,
) -> Result<WebsocketConnectOutcome<CodexWebSocket>, String> {
    let connect = async move {
        let transport = connect_websocket_transport(&request)
            .await
            .map_err(ws_io_error)?;
        let transport = wrap_websocket_transport_tls(&request, transport)
            .await
            .map_err(ws_io_error)?;
        client_async(request, transport).await
    };

    match tokio::time::timeout(Duration::from_secs(30), connect).await {
        Ok(Ok((socket, response))) => {
            turn_state.store_header(
                response
                    .headers()
                    .get(X_CODEX_TURN_STATE_HEADER)
                    .and_then(|value| value.to_str().ok()),
            );

            if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
                return Err(format!(
                    "OpenAI Codex websocket handshake failed (HTTP {} {}): {:?}",
                    response.status().as_u16(),
                    response.status().canonical_reason().unwrap_or(""),
                    response.headers()
                ));
            }

            Ok(WebsocketConnectOutcome::Connected(socket))
        }
        Ok(Err(WsError::Http(response)))
            if response.status() == http::StatusCode::UPGRADE_REQUIRED =>
        {
            Ok(WebsocketConnectOutcome::FallbackToHttp)
        }
        Ok(Err(err)) => Err(format!("Codex websocket connect failed: {}", err)),
        Err(_) => Err("Codex websocket connect timed out".to_string()),
    }
}

fn websocket_event_error_message(payload: &str) -> Option<String> {
    let event: serde_json::Value = serde_json::from_str(payload).ok()?;
    if event.get("type").and_then(|value| value.as_str()) != Some("error") {
        return None;
    }

    let code = event
        .get("code")
        .and_then(|value| value.as_str())
        .or_else(|| {
            event
                .get("error")
                .and_then(|value| value.get("code"))
                .and_then(|value| value.as_str())
        });
    if code == Some(WEBSOCKET_CONNECTION_LIMIT_REACHED_CODE) {
        return Some(WEBSOCKET_CONNECTION_LIMIT_REACHED_MESSAGE.to_string());
    }

    let status = event.get("status").and_then(|value| value.as_u64());
    let message = event
        .get("message")
        .and_then(|value| value.as_str())
        .or_else(|| {
            event
                .get("error")
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("Unknown error");

    Some(match status {
        Some(status) => format!(
            "OpenAI Codex websocket error (HTTP {}): {}",
            status, message
        ),
        None => format!("OpenAI Codex websocket error: {}", message),
    })
}

struct PartialToolCall {
    call_id: String,
    name: String,
    arguments: String,
    arguments_done: bool,
    item_done: bool,
    notified: bool,
    start_order: Option<usize>,
}

impl PartialToolCall {
    fn is_complete(&self) -> bool {
        (self.arguments_done || self.item_done)
            && !self.call_id.trim().is_empty()
            && !self.name.trim().is_empty()
            && valid_tool_arguments(&self.arguments)
    }

    fn notify_started<H>(&mut self, next_tool_start_order: &mut usize, on_tool_call_start: &H)
    where
        H: Fn(String, String) + Send,
    {
        if self.notified || self.call_id.is_empty() || self.name.is_empty() {
            return;
        }
        self.start_order = Some(*next_tool_start_order);
        *next_tool_start_order += 1;
        on_tool_call_start(self.call_id.clone(), self.name.clone());
        self.notified = true;
    }

    fn display_order(&self) -> usize {
        self.start_order.unwrap_or(usize::MAX)
    }
}

struct OrderedToolCall {
    start_order: usize,
    tool_call: ToolCallInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReasoningContentKind {
    Summary,
    Text,
}

fn collect_complete_tool_calls(
    tool_calls_map: &std::collections::HashMap<String, PartialToolCall>,
) -> (Vec<OrderedToolCall>, usize) {
    let mut collected = Vec::new();
    let mut incomplete = 0usize;

    for tc in tool_calls_map.values() {
        if tc.is_complete() {
            collected.push(OrderedToolCall {
                start_order: tc.display_order(),
                tool_call: ToolCallInfo {
                    id: tc.call_id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                },
            });
        } else {
            incomplete += 1;
        }
    }

    collected.sort_by_key(|entry| entry.start_order);

    (collected, incomplete)
}

fn valid_tool_arguments(arguments: &str) -> bool {
    let trimmed = arguments.trim();
    trimmed.is_empty() || serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
}

struct CodexStreamState {
    full_text: String,
    thinking_text: String,
    thinking_kind: Option<ReasoningContentKind>,
    thinking_started_at: Option<Instant>,
    thinking_duration_secs: u32,
    tool_calls_map: std::collections::HashMap<String, PartialToolCall>,
    next_tool_start_order: usize,
    pending_server_tool_start_orders: std::collections::HashMap<String, usize>,
    /// Completed web_search_call server tool calls (no local execution needed).
    web_search_tool_calls: Vec<OrderedToolCall>,
    finish_reason: String,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    response_id: Option<String>,
    items_added: Vec<serde_json::Value>,
    got_terminal_event: bool,
}

impl CodexStreamState {
    fn new() -> Self {
        Self {
            full_text: String::new(),
            thinking_text: String::new(),
            thinking_kind: None,
            thinking_started_at: None,
            thinking_duration_secs: 0,
            tool_calls_map: std::collections::HashMap::new(),
            next_tool_start_order: 0,
            pending_server_tool_start_orders: std::collections::HashMap::new(),
            web_search_tool_calls: Vec::new(),
            finish_reason: "stop".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            response_id: None,
            items_added: Vec::new(),
            got_terminal_event: false,
        }
    }

    fn finish_thinking_timing(&mut self) {
        if self.thinking_duration_secs > 0 || self.thinking_text.is_empty() {
            return;
        }
        if let Some(started_at) = self.thinking_started_at {
            self.thinking_duration_secs = started_at.elapsed().as_secs() as u32;
        }
    }

    fn accepts_reasoning_kind(&mut self, kind: ReasoningContentKind) -> bool {
        match self.thinking_kind {
            Some(current) => current == kind,
            None => {
                self.thinking_kind = Some(kind);
                true
            }
        }
    }

    fn push_reasoning_delta<G>(
        &mut self,
        kind: ReasoningContentKind,
        delta: &str,
        on_thinking_delta: &G,
    ) where
        G: Fn(String) + Send + 'static,
    {
        if delta.is_empty() || !self.accepts_reasoning_kind(kind) {
            return;
        }
        if self.thinking_started_at.is_none() {
            self.thinking_started_at = Some(Instant::now());
        }
        self.thinking_text.push_str(delta);
        on_thinking_delta(delta.to_string());
    }

    fn sync_reasoning_text<G>(
        &mut self,
        kind: ReasoningContentKind,
        text: &str,
        on_thinking_delta: &G,
    ) where
        G: Fn(String) + Send + 'static,
    {
        if text.is_empty() || !self.accepts_reasoning_kind(kind) {
            return;
        }

        if self.thinking_started_at.is_none() {
            self.thinking_started_at = Some(Instant::now());
        }

        if self.thinking_text.is_empty() {
            self.thinking_text.push_str(text);
            on_thinking_delta(text.to_string());
            return;
        }

        if self.thinking_text == text {
            return;
        }

        if let Some(suffix) = text.strip_prefix(&self.thinking_text) {
            if !suffix.is_empty() {
                self.thinking_text.push_str(suffix);
                on_thinking_delta(suffix.to_string());
            }
        }
    }

    fn allocate_tool_start_order(&mut self) -> usize {
        let order = self.next_tool_start_order;
        self.next_tool_start_order += 1;
        order
    }
}

fn next_sse_separator(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|pos| (pos, 2usize));
    let crlf = buffer.find("\r\n\r\n").map(|pos| (pos, 4usize));

    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn process_sse_event_block<F, G, H>(
    event_text: &str,
    debug: bool,
    state: &mut CodexStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<bool, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send,
{
    for line in event_text.lines() {
        let line = line.trim();

        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(true);
            }

            let event: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(e) => {
                    if debug {
                        eprintln!(
                            "[DEBUG][OpenAI Codex] failed to parse SSE data: {} | raw: {}",
                            e, data
                        );
                    }
                    continue;
                }
            };

            match event.get("type").and_then(|t| t.as_str()) {
                Some("response.output_text.delta") => {
                    if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                        state.finish_thinking_timing();
                        state.full_text.push_str(delta);
                        on_text_delta(delta.to_string());
                    }
                }
                Some("response.reasoning_summary_text.delta") => {
                    if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                        state.push_reasoning_delta(
                            ReasoningContentKind::Summary,
                            delta,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.reasoning_summary_text.done") => {
                    if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                        state.sync_reasoning_text(
                            ReasoningContentKind::Summary,
                            text,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.reasoning_summary_part.done") => {
                    if let Some(text) = event
                        .get("part")
                        .and_then(|part| part.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        state.sync_reasoning_text(
                            ReasoningContentKind::Summary,
                            text,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.reasoning_text.delta") => {
                    if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                        state.push_reasoning_delta(
                            ReasoningContentKind::Text,
                            delta,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.reasoning_text.done") => {
                    if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                        state.sync_reasoning_text(
                            ReasoningContentKind::Text,
                            text,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.content_part.done") => {
                    let maybe_reasoning_text = event
                        .get("part")
                        .filter(|part| {
                            part.get("type").and_then(|v| v.as_str()) == Some("reasoning_text")
                        })
                        .and_then(|part| part.get("text"))
                        .and_then(|v| v.as_str());
                    if let Some(text) = maybe_reasoning_text {
                        state.sync_reasoning_text(
                            ReasoningContentKind::Text,
                            text,
                            on_thinking_delta,
                        );
                    }
                }
                Some("response.output_item.added") => {
                    if let Some(item) = event.get("item") {
                        let item_type = item.get("type").and_then(|t| t.as_str());
                        if item_type == Some("function_call") {
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let arguments = item
                                .get("arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let item_id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&call_id)
                                .to_string();

                            state.tool_calls_map.insert(
                                item_id,
                                PartialToolCall {
                                    call_id,
                                    name,
                                    arguments,
                                    arguments_done: false,
                                    item_done: false,
                                    notified: false,
                                    start_order: None,
                                },
                            );
                        } else if item_type == Some("web_search_call") {
                            // Server-side web search started; notify frontend.
                            let id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !id.is_empty() {
                                let start_order = state.allocate_tool_start_order();
                                on_tool_call_start(id.clone(), "web_search".to_string());
                                state
                                    .pending_server_tool_start_orders
                                    .insert(id, start_order);
                            }
                        }
                    }
                }
                Some("response.function_call_arguments.delta") => {
                    let item_id = event.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    let delta = event.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(tc) = state.tool_calls_map.get_mut(item_id) {
                        tc.arguments.push_str(delta);
                        if !delta.is_empty() {
                            tc.notify_started(&mut state.next_tool_start_order, on_tool_call_start);
                        }
                    }
                }
                Some("response.function_call_arguments.done") => {
                    let item_id = event.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(arguments) = event.get("arguments").and_then(|v| v.as_str()) {
                        if let Some(tc) = state.tool_calls_map.get_mut(item_id) {
                            tc.arguments = arguments.to_string();
                            tc.arguments_done = true;
                            tc.notify_started(&mut state.next_tool_start_order, on_tool_call_start);
                        }
                    }
                }
                Some("response.output_item.done") => {
                    if let Some(item) = event.get("item") {
                        state.items_added.push(item.clone());
                        let item_type = item.get("type").and_then(|t| t.as_str());
                        if item_type == Some("function_call") {
                            let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(arguments) = item.get("arguments").and_then(|v| v.as_str())
                            {
                                if let Some(tc) = state.tool_calls_map.get_mut(item_id) {
                                    tc.arguments = arguments.to_string();
                                    tc.item_done = true;
                                    tc.notify_started(
                                        &mut state.next_tool_start_order,
                                        on_tool_call_start,
                                    );
                                }
                            } else if let Some(tc) = state.tool_calls_map.get_mut(item_id) {
                                tc.item_done = true;
                                tc.notify_started(
                                    &mut state.next_tool_start_order,
                                    on_tool_call_start,
                                );
                            }
                        } else if item_type == Some("web_search_call") {
                            // Server-side web search completed. Extract query from action.
                            let id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let action = item.get("action");
                            let action_type = action
                                .and_then(|a| a.get("type"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let query = action
                                .and_then(|a| a.get("query"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            let detail = match action_type {
                                "search" => format!("Searched: {}", query),
                                "open_page" => {
                                    let url = action
                                        .and_then(|a| a.get("url"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    format!("Opened page: {}", url)
                                }
                                "find_in_page" => {
                                    let pattern = action
                                        .and_then(|a| a.get("pattern"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    format!("Find in page: {}", pattern)
                                }
                                _ => format!("Web search completed: {}", query),
                            };

                            let start_order = state
                                .pending_server_tool_start_orders
                                .remove(&id)
                                .unwrap_or_else(|| state.allocate_tool_start_order());

                            state.web_search_tool_calls.push(OrderedToolCall {
                                start_order,
                                tool_call: ToolCallInfo {
                                    id: id.clone(),
                                    name: "web_search".to_string(),
                                    arguments: serde_json::json!({"query": query}).to_string(),
                                    server_tool: Some(ServerToolKind::WebSearch),
                                    server_tool_output: Some(detail),
                                    outcome: None,
                                    recorded_output: None,
                                    nested_tool_calls: None,
                                },
                            });
                        }
                    }
                }
                Some("response.completed") | Some("response.incomplete") => {
                    state.got_terminal_event = true;
                    state.finish_thinking_timing();
                    if let Some(response) = event.get("response") {
                        state.response_id = response
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|value| !value.is_empty())
                            .map(|value| value.to_string());
                        if let Some(usage) = response.get("usage") {
                            state.cached_tokens = usage
                                .get("input_tokens_details")
                                .and_then(|d| d.get("cached_tokens"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as u32;
                            state.input_tokens = usage
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                .saturating_sub(state.cached_tokens as u64)
                                as u32;
                            state.output_tokens = usage
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as u32;
                        }
                    }
                    if event.get("type").and_then(|t| t.as_str()) == Some("response.incomplete") {
                        state.finish_reason = "length".to_string();
                    }
                    return Ok(true);
                }
                Some("error") => {
                    let msg = event
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(format!("OpenAI Codex stream error: {}", msg));
                }
                _ => {}
            }
        }
    }

    Ok(false)
}

fn drain_sse_buffer<F, G, H>(
    buffer: &mut String,
    flush_final_block: bool,
    debug: bool,
    state: &mut CodexStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<bool, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send,
{
    while let Some((pos, sep_len)) = next_sse_separator(buffer) {
        let event_text = buffer[..pos].to_string();
        *buffer = buffer[pos + sep_len..].to_string();
        if process_sse_event_block(
            &event_text,
            debug,
            state,
            on_text_delta,
            on_thinking_delta,
            on_tool_call_start,
        )? {
            return Ok(true);
        }
    }

    if flush_final_block {
        let trailing = buffer.trim_matches(|c| c == '\r' || c == '\n').to_string();
        if !trailing.is_empty() {
            if process_sse_event_block(
                &trailing,
                debug,
                state,
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn should_retry_safe_codex_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();

    if lower.contains("stream ended with no data and no response.completed") {
        return true;
    }

    if lower.contains("responses websocket connection limit reached")
        || lower.contains("websocket connection limit reached")
    {
        return true;
    }

    if lower.contains("previous response with id") && lower.contains("not found") {
        return true;
    }

    // "error sending request" is a reqwest transport failure with no partial output
    if lower.contains("error sending request") {
        return true;
    }

    if lower.contains("codex request failed:") {
        return lower.contains("timed out")
            || lower.contains("connection")
            || lower.contains("eof")
            || lower.contains("reset")
            || lower.contains("closed");
    }

    let no_visible_output = lower.contains("text_len=0") && lower.contains("complete_tool_calls=0");

    no_visible_output
        && (lower.contains("stream ended without response.completed")
            || lower.contains("stream ended before the response finalized")
            || lower.contains("response completed with"))
}

enum CodexWebsocketAttempt {
    Response(LlmResponse),
    FallbackToHttp,
}

pub async fn stream_chat<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    debug: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    turn_state: &mut TurnState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + Sync + 'static,
    G: Fn(String) + Send + Sync + 'static,
    H: Fn(String, String) + Send + Sync,
{
    let mut last_error = String::new();

    for attempt in 0..=MAX_SAFE_STREAM_RECOVERY_RETRIES {
        match stream_chat_once(
            access_token,
            account_id,
            transport,
            base_url,
            model,
            system_prompt,
            history,
            tools,
            thinking_level,
            debug,
            session_id,
            response_request_metadata,
            turn_state,
            on_text_delta,
            on_thinking_delta,
            on_tool_call_start,
        )
        .await
        {
            Ok(resp) => return Ok(resp),
            Err(err) => {
                last_error = err;
                if should_retry_safe_codex_error(&last_error)
                    && attempt < MAX_SAFE_STREAM_RECOVERY_RETRIES
                {
                    let delay = SAFE_STREAM_RECOVERY_DELAY_MS * (attempt as u64 + 1);
                    eprintln!(
                        "[OpenAI Codex] retrying safe stream interruption (attempt {}/{}, retrying in {}ms): {}",
                        attempt + 1,
                        MAX_SAFE_STREAM_RECOVERY_RETRIES + 1,
                        delay,
                        last_error
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                return Err(last_error);
            }
        }
    }

    Err(last_error)
}

async fn stream_chat_once<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    debug: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    turn_state: &mut TurnState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + Sync + 'static,
    G: Fn(String) + Send + Sync + 'static,
    H: Fn(String, String) + Send + Sync,
{
    let body = build_request_body(
        model,
        system_prompt,
        history,
        tools,
        thinking_level,
        session_id,
    );

    match transport {
        CodexTransportMode::Http => {
            stream_chat_http_once(
                access_token,
                account_id,
                base_url,
                model,
                history,
                tools,
                debug,
                body,
                response_request_metadata,
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )
            .await
        }
        CodexTransportMode::Websocket => {
            match stream_chat_websocket_once(
                access_token,
                account_id,
                base_url,
                model,
                history,
                tools,
                session_id,
                debug,
                body.clone(),
                turn_state,
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )
            .await?
            {
                CodexWebsocketAttempt::Response(resp) => Ok(resp),
                CodexWebsocketAttempt::FallbackToHttp => {
                    stream_chat_http_once(
                        access_token,
                        account_id,
                        base_url,
                        model,
                        history,
                        tools,
                        debug,
                        body,
                        response_request_metadata,
                        on_text_delta,
                        on_thinking_delta,
                        on_tool_call_start,
                    )
                    .await
                }
            }
        }
    }
}

async fn stream_chat_http_once<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    model: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    debug: bool,
    body: serde_json::Value,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send,
{
    let client = reqwest::Client::builder()
        .tcp_keepalive(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let continuation_request = request_without_input(&body);
    let request_body = build_history_transport_request(
        &body,
        history,
        response_request_metadata,
        /*include_type_field*/ false,
    );
    let raw_request = serde_json::to_string_pretty(&request_body).unwrap_or_default();
    let api_url = codex_responses_endpoint(base_url);

    eprintln!(
        "[OpenAI Codex][http] POST model={} messages={} tools={}",
        model,
        history.len(),
        tools.len()
    );
    if debug {
        eprintln!("[DEBUG][OpenAI Codex] request body:\n{}", &raw_request);
        let mut headers: Vec<(&str, &str)> = vec![
            ("Authorization", "Bearer <token>"),
            ("Content-Type", "application/json"),
            ("originator", CODEX_ORIGINATOR_HEADER_VALUE),
            ("version", CODEX_CLIENT_VERSION),
        ];
        if let Some(aid) = account_id {
            headers.push(("ChatGPT-Account-ID", aid));
        }
        super::debug::save_request("openai_codex", &api_url, &headers, &raw_request);
    }

    let mut req = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("originator", CODEX_ORIGINATOR_HEADER_VALUE)
        .header("version", CODEX_CLIENT_VERSION)
        .json(&request_body);

    if let Some(aid) = account_id {
        req = req.header("ChatGPT-Account-ID", aid);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Codex request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "OpenAI Codex API error ({} {}): {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            err_body
        ));
    }

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut stream_state = CodexStreamState::new();
    let mut raw_response = String::new();

    let mut terminal_stream_error: Option<String> = None;
    let mut consecutive_errors = 0u32;
    const MAX_STREAM_ERRORS: u32 = 3;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => {
                consecutive_errors = 0;
                c
            }
            Err(e) => {
                consecutive_errors += 1;
                eprintln!(
                    "[OpenAI Codex] stream read error ({}/{}): {}",
                    consecutive_errors, MAX_STREAM_ERRORS, e
                );
                if consecutive_errors >= MAX_STREAM_ERRORS {
                    if !stream_state.full_text.is_empty() || !stream_state.tool_calls_map.is_empty()
                    {
                        terminal_stream_error = Some(format!("Stream read error: {}", e));
                        break;
                    }
                    return Err(format!("Stream read error: {}", e));
                }
                continue;
            }
        };

        let chunk_text = String::from_utf8_lossy(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);
        if drain_sse_buffer(
            &mut buffer,
            false,
            debug,
            &mut stream_state,
            on_text_delta,
            on_thinking_delta,
            on_tool_call_start,
        )? {
            break;
        }

        /* while let Some(pos) = buffer.find("\n\n") {
            let event_text = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            for line in event_text.lines() {
                let line = line.trim();

                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        break 'outer;
                    }

                    let event: serde_json::Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            if debug {
                                eprintln!("[DEBUG][OpenAI Codex] failed to parse SSE data: {} | raw: {}", e, data);
                            }
                            continue;
                        }
                    };

                    match event.get("type").and_then(|t| t.as_str()) {
                        Some("response.output_text.delta") => {
                            if let Some(delta) = event.get("delta").and_then(|d| d.as_str()) {
                                full_text.push_str(delta);
                                on_text_delta(delta.to_string());
                            }
                        }

                        Some("response.output_item.added") => {
                            if let Some(item) = event.get("item") {
                                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let arguments = item
                                        .get("arguments")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let item_id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(&call_id)
                                        .to_string();

                                    on_tool_call_start(call_id.clone(), name.clone());
                                    tool_calls_map.insert(
                                        item_id,
                                        PartialToolCall {
                                            call_id,
                                            name,
                                            arguments,
                                            arguments_done: false,
                                            item_done: false,
                                        },
                                    );
                                }
                            }
                        }

                        Some("response.function_call_arguments.delta") => {
                            let item_id = event
                                .get("item_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let delta = event.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(tc) = tool_calls_map.get_mut(item_id) {
                                tc.arguments.push_str(delta);
                            }
                        }

                        Some("response.function_call_arguments.done") => {
                            let item_id = event
                                .get("item_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if let Some(arguments) = event.get("arguments").and_then(|v| v.as_str()) {
                                if let Some(tc) = tool_calls_map.get_mut(item_id) {
                                    tc.arguments = arguments.to_string();
                                    tc.arguments_done = true;
                                }
                            }
                        }

                        Some("response.output_item.done") => {
                            if let Some(item) = event.get("item") {
                                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                                    let item_id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if let Some(arguments) = item.get("arguments").and_then(|v| v.as_str()) {
                                        if let Some(tc) = tool_calls_map.get_mut(item_id) {
                                            tc.arguments = arguments.to_string();
                                            tc.item_done = true;
                                        }
                                    } else if let Some(tc) = tool_calls_map.get_mut(item_id) {
                                        tc.item_done = true;
                                    }
                                }
                            }
                        }

                        Some("response.completed") | Some("response.incomplete") => {
                            got_terminal_event = true;
                            if let Some(r) = event.get("response") {
                                if let Some(usage) = r.get("usage") {
                                    cached_tokens = usage
                                        .get("input_tokens_details")
                                        .and_then(|d| d.get("cached_tokens"))
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                    input_tokens = usage
                                        .get("input_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        .saturating_sub(cached_tokens as u64)
                                        as u32;
                                    output_tokens = usage
                                        .get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                }
                                if event.get("type").and_then(|t| t.as_str())
                                    == Some("response.incomplete")
                                {
                                    finish_reason = "length".to_string();
                                }
                            }
                            break 'outer;
                        }

                        Some("error") => {
                            let msg = event
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown error");
                            return Err(format!("OpenAI Codex stream error: {}", msg));
                        }

                        _ => {}
                    }
                }
            }
        }
        */
    }

    let _ = drain_sse_buffer(
        &mut buffer,
        true,
        debug,
        &mut stream_state,
        on_text_delta,
        on_thinking_delta,
        on_tool_call_start,
    )?;

    let (collected, incomplete_tool_calls) =
        collect_complete_tool_calls(&stream_state.tool_calls_map);

    if let Some(stream_error) = terminal_stream_error {
        return Err(format!(
            "{}. OpenAI Codex stream ended before the response finalized (text_len={}, complete_tool_calls={}, incomplete_tool_calls={}). Refusing to execute partial tool arguments.",
            stream_error,
            stream_state.full_text.len(),
            collected.len(),
            incomplete_tool_calls
        ));
    }

    if !stream_state.got_terminal_event {
        if !stream_state.full_text.is_empty() || !stream_state.tool_calls_map.is_empty() {
            return Err(format!(
                "Stream ended without response.completed (incomplete response, text_len={}, complete_tool_calls={}, incomplete_tool_calls={}).",
                stream_state.full_text.len(),
                collected.len(),
                incomplete_tool_calls
            ));
        }
        return Err("Stream ended with no data and no response.completed".to_string());
    }

    if incomplete_tool_calls > 0 {
        return Err(format!(
            "Response completed with {} incomplete tool call(s) (text_len={}, complete_tool_calls={}). Refusing to execute partial tool arguments.",
            incomplete_tool_calls,
            stream_state.full_text.len(),
            collected.len()
        ));
    }

    // Merge server-side web_search_call results into collected tool calls.
    let mut collected = collected;
    collected.extend(stream_state.web_search_tool_calls.drain(..));
    collected.sort_by_key(|entry| entry.start_order);
    let tool_calls: Vec<ToolCallInfo> =
        collected.into_iter().map(|entry| entry.tool_call).collect();

    if !tool_calls.is_empty() {
        stream_state.finish_reason = "tool_calls".to_string();
    }

    if debug {
        eprintln!(
            "[DEBUG][OpenAI Codex] response complete: finish_reason={}, text_len={}, tool_calls={}",
            stream_state.finish_reason,
            stream_state.full_text.len(),
            tool_calls.len()
        );
    }

    Ok(LlmResponse {
        text: stream_state.full_text,
        tool_calls,
        finish_reason: stream_state.finish_reason,
        response_id: stream_state.response_id,
        input_tokens: stream_state.input_tokens,
        output_tokens: stream_state.output_tokens,
        cache_read_tokens: stream_state.cached_tokens,
        cache_write_tokens: 0,
        cost_usd: 0.0,
        raw_request,
        raw_response,
        thinking_text: stream_state.thinking_text,
        thinking_duration_secs: stream_state.thinking_duration_secs,
        thinking_signature: String::new(),
        continuation_request: Some(continuation_request),
    })
}

async fn stream_chat_websocket_once<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    model: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    session_id: Option<&str>,
    debug: bool,
    body: serde_json::Value,
    turn_state: &mut TurnState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<CodexWebsocketAttempt, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send,
{
    let continuation_request = request_without_input(&body);
    let ws_url = codex_websocket_url(base_url)?;
    let connection_key = websocket_connection_key(base_url, account_id);
    let (shared_session, cached_socket, last_response, disable_websockets) = match session_id {
        Some(session_id) => take_cached_websocket_session_state(session_id, &connection_key).await,
        None => (
            Arc::new(tokio::sync::Mutex::new(CachedWebsocketSession::default())),
            None,
            None,
            false,
        ),
    };
    if disable_websockets {
        return Ok(CodexWebsocketAttempt::FallbackToHttp);
    }

    let ws_request = build_websocket_transport_request(
        &body,
        last_response.as_ref(),
        /*include_type_field*/ true,
    );
    let raw_request = serde_json::to_string_pretty(&ws_request).unwrap_or_default();
    let cached_turn_state = turn_state.header_value().map(str::to_string);

    eprintln!(
        "[OpenAI Codex][websocket] CONNECT model={} messages={} tools={}",
        model,
        history.len(),
        tools.len()
    );

    if debug {
        let mut headers: Vec<(&str, &str)> = vec![
            ("Authorization", "Bearer <token>"),
            ("Content-Type", "application/json"),
            ("OpenAI-Beta", RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE),
            ("originator", CODEX_ORIGINATOR_HEADER_VALUE),
            ("version", CODEX_CLIENT_VERSION),
        ];
        if cached_turn_state.is_some() {
            headers.push((X_CODEX_TURN_STATE_HEADER, "<sticky>"));
        }
        if let Some(sid) = session_id {
            headers.push(("x-client-request-id", sid));
            headers.push(("session_id", sid));
        }
        if let Some(aid) = account_id {
            headers.push(("ChatGPT-Account-ID", aid));
        }
        super::debug::save_request(
            "openai_codex_websocket",
            ws_url.as_str(),
            &headers,
            &raw_request,
        );
    }

    let request = build_codex_websocket_handshake_request(
        &ws_url,
        access_token,
        account_id,
        session_id,
        cached_turn_state.as_deref(),
    )?;

    let mut socket = match cached_socket {
        Some(socket) => socket,
        None => match connect_codex_websocket(request, turn_state).await? {
            WebsocketConnectOutcome::Connected(socket) => socket,
            WebsocketConnectOutcome::FallbackToHttp => {
                clear_cached_websocket_session_state(
                    &shared_session,
                    &connection_key,
                    /*disable_websockets*/ true,
                )
                .await;
                return Ok(CodexWebsocketAttempt::FallbackToHttp);
            }
        },
    };

    let request_text = match serde_json::to_string(&ws_request) {
        Ok(text) => text,
        Err(e) => {
            clear_cached_websocket_session_state(
                &shared_session,
                &connection_key,
                /*disable_websockets*/ false,
            )
            .await;
            return Err(format!("Failed to encode websocket request body: {}", e));
        }
    };
    if let Err(e) = socket.send(Message::Text(request_text.into())).await {
        clear_cached_websocket_session_state(
            &shared_session,
            &connection_key,
            /*disable_websockets*/ false,
        )
        .await;
        return Err(format!("Failed to send websocket request: {}", e));
    }

    let mut stream_state = CodexStreamState::new();
    let mut raw_response = String::new();
    let mut terminal_stream_error: Option<String> = None;
    let mut consecutive_errors = 0u32;
    const MAX_WEBSOCKET_ERRORS: u32 = 3;

    loop {
        let message = match tokio::time::timeout(Duration::from_secs(90), socket.next()).await {
            Ok(Some(Ok(message))) => {
                consecutive_errors = 0;
                message
            }
            Ok(Some(Err(e))) => {
                consecutive_errors += 1;
                eprintln!(
                    "[OpenAI Codex] websocket read error ({}/{}): {}",
                    consecutive_errors, MAX_WEBSOCKET_ERRORS, e
                );
                if consecutive_errors >= MAX_WEBSOCKET_ERRORS {
                    if !stream_state.full_text.is_empty() || !stream_state.tool_calls_map.is_empty()
                    {
                        terminal_stream_error = Some(format!("WebSocket read error: {}", e));
                        break;
                    }
                    clear_cached_websocket_session_state(
                        &shared_session,
                        &connection_key,
                        /*disable_websockets*/ false,
                    )
                    .await;
                    return Err(format!("WebSocket read error: {}", e));
                }
                continue;
            }
            Ok(None) => {
                terminal_stream_error =
                    Some("WebSocket closed before response.completed".to_string());
                break;
            }
            Err(_) => {
                if !stream_state.full_text.is_empty() || !stream_state.tool_calls_map.is_empty() {
                    terminal_stream_error =
                        Some("WebSocket read timed out before the response finalized".to_string());
                    break;
                }
                clear_cached_websocket_session_state(
                    &shared_session,
                    &connection_key,
                    /*disable_websockets*/ false,
                )
                .await;
                return Err("WebSocket read timed out".to_string());
            }
        };

        match message {
            Message::Text(text) => {
                let payload = text.to_string();
                raw_response.push_str(&payload);
                raw_response.push('\n');
                if let Some(error_message) = websocket_event_error_message(&payload) {
                    clear_cached_websocket_session_state(
                        &shared_session,
                        &connection_key,
                        /*disable_websockets*/ false,
                    )
                    .await;
                    return Err(error_message);
                }
                let event_text = format!("data: {}", payload);
                if match process_sse_event_block(
                    &event_text,
                    debug,
                    &mut stream_state,
                    on_text_delta,
                    on_thinking_delta,
                    on_tool_call_start,
                ) {
                    Ok(done) => done,
                    Err(error) => {
                        clear_cached_websocket_session_state(
                            &shared_session,
                            &connection_key,
                            /*disable_websockets*/ false,
                        )
                        .await;
                        return Err(error);
                    }
                } {
                    break;
                }
            }
            Message::Binary(bytes) => {
                let payload = match String::from_utf8(bytes.to_vec()) {
                    Ok(payload) => payload,
                    Err(_) => {
                        clear_cached_websocket_session_state(
                            &shared_session,
                            &connection_key,
                            /*disable_websockets*/ false,
                        )
                        .await;
                        return Err("WebSocket returned non-UTF8 binary payload".to_string());
                    }
                };
                raw_response.push_str(&payload);
                raw_response.push('\n');
                if let Some(error_message) = websocket_event_error_message(&payload) {
                    clear_cached_websocket_session_state(
                        &shared_session,
                        &connection_key,
                        /*disable_websockets*/ false,
                    )
                    .await;
                    return Err(error_message);
                }
                let event_text = format!("data: {}", payload);
                if match process_sse_event_block(
                    &event_text,
                    debug,
                    &mut stream_state,
                    on_text_delta,
                    on_thinking_delta,
                    on_tool_call_start,
                ) {
                    Ok(done) => done,
                    Err(error) => {
                        clear_cached_websocket_session_state(
                            &shared_session,
                            &connection_key,
                            /*disable_websockets*/ false,
                        )
                        .await;
                        return Err(error);
                    }
                } {
                    break;
                }
            }
            Message::Ping(payload) => {
                if let Err(e) = socket.send(Message::Pong(payload)).await {
                    clear_cached_websocket_session_state(
                        &shared_session,
                        &connection_key,
                        /*disable_websockets*/ false,
                    )
                    .await;
                    return Err(format!("Failed to respond to websocket ping: {}", e));
                }
            }
            Message::Pong(_) | Message::Frame(_) => {}
            Message::Close(frame) => {
                if !stream_state.got_terminal_event {
                    terminal_stream_error = Some(match frame {
                        Some(frame) if !frame.reason.is_empty() => {
                            format!("WebSocket closed by server: {}", frame.reason)
                        }
                        Some(frame) => format!("WebSocket closed by server ({})", frame.code),
                        None => "WebSocket closed by server".to_string(),
                    });
                }
                break;
            }
        }
    }

    let (collected, incomplete_tool_calls) =
        collect_complete_tool_calls(&stream_state.tool_calls_map);

    if let Some(stream_error) = terminal_stream_error {
        clear_cached_websocket_session_state(
            &shared_session,
            &connection_key,
            /*disable_websockets*/ false,
        )
        .await;
        return Err(format!(
            "{}. OpenAI Codex websocket ended before the response finalized (text_len={}, complete_tool_calls={}, incomplete_tool_calls={}). Refusing to execute partial tool arguments.",
            stream_error,
            stream_state.full_text.len(),
            collected.len(),
            incomplete_tool_calls
        ));
    }

    if !stream_state.got_terminal_event {
        if !stream_state.full_text.is_empty() || !stream_state.tool_calls_map.is_empty() {
            clear_cached_websocket_session_state(
                &shared_session,
                &connection_key,
                /*disable_websockets*/ false,
            )
            .await;
            return Err(format!(
                "WebSocket ended without response.completed (incomplete response, text_len={}, complete_tool_calls={}, incomplete_tool_calls={}).",
                stream_state.full_text.len(),
                collected.len(),
                incomplete_tool_calls
            ));
        }
        clear_cached_websocket_session_state(
            &shared_session,
            &connection_key,
            /*disable_websockets*/ false,
        )
        .await;
        return Err("WebSocket ended with no data and no response.completed".to_string());
    }

    if incomplete_tool_calls > 0 {
        clear_cached_websocket_session_state(
            &shared_session,
            &connection_key,
            /*disable_websockets*/ false,
        )
        .await;
        return Err(format!(
            "Response completed with {} incomplete tool call(s) over websocket (text_len={}, complete_tool_calls={}). Refusing to execute partial tool arguments.",
            incomplete_tool_calls,
            stream_state.full_text.len(),
            collected.len()
        ));
    }

    store_cached_websocket_session_state(
        &shared_session,
        &connection_key,
        socket,
        LastWebsocketResponse {
            request_signature: request_without_input(&body),
            input: body
                .get("input")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default(),
            response_id: stream_state.response_id.clone().unwrap_or_default(),
            items_added: stream_state.items_added.clone(),
        },
    )
    .await;

    let mut collected = collected;
    collected.extend(stream_state.web_search_tool_calls.drain(..));
    collected.sort_by_key(|entry| entry.start_order);
    let tool_calls: Vec<ToolCallInfo> =
        collected.into_iter().map(|entry| entry.tool_call).collect();

    if !tool_calls.is_empty() {
        stream_state.finish_reason = "tool_calls".to_string();
    }

    if debug {
        eprintln!(
            "[DEBUG][OpenAI Codex][websocket] response complete: finish_reason={}, text_len={}, tool_calls={}",
            stream_state.finish_reason,
            stream_state.full_text.len(),
            tool_calls.len()
        );
    }

    Ok(CodexWebsocketAttempt::Response(LlmResponse {
        text: stream_state.full_text,
        tool_calls,
        finish_reason: stream_state.finish_reason,
        response_id: stream_state.response_id,
        input_tokens: stream_state.input_tokens,
        output_tokens: stream_state.output_tokens,
        cache_read_tokens: stream_state.cached_tokens,
        cache_write_tokens: 0,
        cost_usd: 0.0,
        raw_request,
        raw_response,
        thinking_text: stream_state.thinking_text,
        thinking_duration_secs: stream_state.thinking_duration_secs,
        thinking_signature: String::new(),
        continuation_request: Some(continuation_request),
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        build_codex_websocket_handshake_request, build_history_transport_request, build_input,
        build_request_body, build_websocket_transport_request, codex_websocket_url,
        collect_complete_tool_calls, drain_sse_buffer, establish_http_connect_tunnel,
        process_sse_event_block, request_without_input, uri_host_port,
        websocket_event_error_message, websocket_proxy_match_uri, CodexStreamState,
        LastWebsocketResponse, PartialToolCall, CODEX_ORIGINATOR_HEADER_VALUE,
        RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE, X_CODEX_TURN_STATE_HEADER,
    };
    use crate::llm::CODEX_CLIENT_VERSION;
    use crate::session::models::{
        ChatMessage, ImageData, MessageRole, ServerToolKind, ToolCallInfo,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn ignore_text(_: String) {}
    fn ignore_thinking(_: String) {}
    fn ignore_tool(_: String, _: String) {}

    fn user_message_with_images(text: &str, images: Vec<ImageData>) -> ChatMessage {
        ChatMessage {
            id: "msg_user".to_string(),
            role: MessageRole::User,
            content: text.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            tool_calls: None,
            tool_call_id: None,
            images: Some(images),
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }
    }

    fn assistant_message(id: &str, content: &str, response_id: Option<&str>) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Assistant,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: response_id.map(|value| value.to_string()),
            tool_calls: None,
            tool_call_id: None,
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }
    }

    fn assistant_message_with_tool_calls(
        id: &str,
        content: &str,
        response_id: Option<&str>,
        tool_calls: Vec<ToolCallInfo>,
    ) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Assistant,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: response_id.map(|value| value.to_string()),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }
    }

    fn tool_message(id: &str, tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Tool,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }
    }

    fn response_request_metadata(
        message_id: &str,
        body: &serde_json::Value,
    ) -> HashMap<String, serde_json::Value> {
        HashMap::from([(message_id.to_string(), request_without_input(body))])
    }

    fn websocket_last_response(
        body: &serde_json::Value,
        response_id: &str,
        items_added: &[serde_json::Value],
    ) -> LastWebsocketResponse {
        LastWebsocketResponse {
            request_signature: request_without_input(body),
            input: body
                .get("input")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default(),
            response_id: response_id.to_string(),
            items_added: items_added.to_vec(),
        }
    }

    #[test]
    fn build_input_includes_function_call_output_for_server_tool_calls() {
        let input = build_input(&[assistant_message_with_tool_calls(
            "assistant-1",
            "查完了",
            Some("resp_prev"),
            vec![ToolCallInfo {
                id: "ws_1".to_string(),
                name: "web_search".to_string(),
                arguments: r#"{"query":"rust async await"}"#.to_string(),
                server_tool: Some(ServerToolKind::WebSearch),
                server_tool_output: Some("Searched: rust async await".to_string()),
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }],
        )]);

        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], serde_json::json!("assistant"));
        assert_eq!(input[1]["type"], serde_json::json!("function_call"));
        assert_eq!(input[1]["call_id"], serde_json::json!("ws_1"));
        assert_eq!(input[2]["type"], serde_json::json!("function_call_output"));
        assert_eq!(input[2]["call_id"], serde_json::json!("ws_1"));
        assert_eq!(
            input[2]["output"],
            serde_json::json!("Searched: rust async await")
        );
    }

    #[test]
    fn ignores_incomplete_tool_calls() {
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "complete".to_string(),
            PartialToolCall {
                call_id: "call_complete".to_string(),
                name: "write".to_string(),
                arguments: r#"{"filePath":"Assets/Test.cs","content":"ok"}"#.to_string(),
                arguments_done: true,
                item_done: false,
                notified: true,
                start_order: Some(0),
            },
        );
        tool_calls.insert(
            "partial".to_string(),
            PartialToolCall {
                call_id: "call_partial".to_string(),
                name: "write".to_string(),
                arguments: r#"{"content":"half"}"#.to_string(),
                arguments_done: false,
                item_done: false,
                notified: false,
                start_order: None,
            },
        );

        let (collected, incomplete) = collect_complete_tool_calls(&tool_calls);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].tool_call.id, "call_complete");
        assert_eq!(incomplete, 1);
    }

    #[test]
    fn treats_complete_tool_calls_with_empty_name_as_incomplete() {
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "missing-name".to_string(),
            PartialToolCall {
                call_id: "call_1".to_string(),
                name: String::new(),
                arguments: "{}".to_string(),
                arguments_done: true,
                item_done: false,
                notified: false,
                start_order: None,
            },
        );

        let (collected, incomplete) = collect_complete_tool_calls(&tool_calls);
        assert!(collected.is_empty());
        assert_eq!(incomplete, 1);
    }

    #[test]
    fn collects_complete_tool_calls_in_start_order() {
        let mut tool_calls = std::collections::HashMap::new();
        tool_calls.insert(
            "second".to_string(),
            PartialToolCall {
                call_id: "call_second".to_string(),
                name: "write".to_string(),
                arguments: r#"{"filePath":"Assets/Second.cs"}"#.to_string(),
                arguments_done: true,
                item_done: false,
                notified: true,
                start_order: Some(1),
            },
        );
        tool_calls.insert(
            "first".to_string(),
            PartialToolCall {
                call_id: "call_first".to_string(),
                name: "read".to_string(),
                arguments: r#"{"path":"Assets/First.cs"}"#.to_string(),
                arguments_done: true,
                item_done: false,
                notified: true,
                start_order: Some(0),
            },
        );

        let (collected, incomplete) = collect_complete_tool_calls(&tool_calls);
        let ids: Vec<_> = collected
            .into_iter()
            .map(|entry| entry.tool_call.id)
            .collect();
        assert_eq!(ids, vec!["call_first", "call_second"]);
        assert_eq!(incomplete, 0);
    }

    #[test]
    fn delays_tool_start_until_arguments_arrive() {
        let mut state = CodexStreamState::new();
        let started = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let captured = started.clone();
        let on_tool = move |id: String, name: String| {
            captured
                .lock()
                .expect("tool mutex poisoned")
                .push((id, name));
        };

        process_sse_event_block(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("output_item.added should parse");

        assert!(
            started.lock().expect("tool mutex poisoned").is_empty(),
            "tool start should wait until arguments begin streaming"
        );

        process_sse_event_block(
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"delta\":\"{\"}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("arguments delta should parse");

        let started = started.lock().expect("tool mutex poisoned");
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].0, "call_1");
        assert_eq!(started[0].1, "read");
    }

    #[test]
    fn notifies_tool_start_only_once() {
        let mut state = CodexStreamState::new();
        let started = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let captured = started.clone();
        let on_tool = move |id: String, name: String| {
            captured
                .lock()
                .expect("tool mutex poisoned")
                .push((id, name));
        };

        process_sse_event_block(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("output_item.added should parse");
        process_sse_event_block(
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"delta\":\"{\"}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("first arguments delta should parse");
        process_sse_event_block(
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"item_1\",\"arguments\":\"{\\\"path\\\":\\\"Assets/Test.cs\\\"}\"}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("arguments done should parse");

        let started = started.lock().expect("tool mutex poisoned");
        assert_eq!(started.len(), 1);
    }

    #[test]
    fn flushes_terminal_event_without_trailing_separator() {
        let mut state = CodexStreamState::new();
        let mut buffer = concat!(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"item_1\",\"arguments\":\"{\\\"path\\\":\\\"Assets/Test.cs\\\"}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":4,\"input_tokens_details\":{\"cached_tokens\":3}}}}"
        ).to_string();

        let stopped = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("trailing terminal event should parse");

        let (collected, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
        assert!(stopped);
        assert!(state.got_terminal_event);
        assert_eq!(state.input_tokens, 9);
        assert_eq!(state.output_tokens, 4);
        assert_eq!(state.cached_tokens, 3);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].tool_call.id, "call_1");
        assert_eq!(
            collected[0].tool_call.arguments,
            r#"{"path":"Assets/Test.cs"}"#
        );
        assert_eq!(incomplete, 0);
    }

    #[test]
    fn supports_crlf_separated_sse_blocks() {
        let mut state = CodexStreamState::new();
        let mut buffer = concat!(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}\r\n\r\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"arguments\":\"{\\\"path\\\":\\\"Assets/Test.cs\\\"}\"}}\r\n\r\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}"
        ).to_string();

        let stopped = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("CRLF-delimited events should parse");

        let (collected, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
        assert!(stopped);
        assert!(state.got_terminal_event);
        assert_eq!(collected.len(), 1);
        assert_eq!(
            collected[0].tool_call.arguments,
            r#"{"path":"Assets/Test.cs"}"#
        );
        assert_eq!(incomplete, 0);
    }

    #[test]
    fn keeps_server_tool_calls_in_started_order_when_mixed_with_function_calls() {
        let mut state = CodexStreamState::new();

        process_sse_event_block(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\"}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("web search add should parse");

        process_sse_event_block(
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"\"}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("function call add should parse");

        process_sse_event_block(
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"item_1\",\"arguments\":\"{\\\"path\\\":\\\"Assets/Test.cs\\\"}\"}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("function call done should parse");

        process_sse_event_block(
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"ws_1\",\"type\":\"web_search_call\",\"action\":{\"type\":\"search\",\"query\":\"unity\"}}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("web search done should parse");

        let (mut collected, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
        collected.extend(state.web_search_tool_calls.drain(..));
        collected.sort_by_key(|entry| entry.start_order);

        let ids: Vec<_> = collected
            .into_iter()
            .map(|entry| entry.tool_call.id)
            .collect();

        assert_eq!(ids, vec!["ws_1", "call_1"]);
        assert_eq!(incomplete, 0);
    }

    #[test]
    fn builds_user_input_blocks_with_images() {
        let input = build_input(&[user_message_with_images(
            "Describe this image",
            vec![ImageData {
                data: "YWJj".to_string(),
                mime_type: "image/png".to_string(),
            }],
        )]);

        let content = input[0]
            .get("content")
            .and_then(|v| v.as_array())
            .expect("user content should be a block array");

        assert_eq!(content.len(), 2);
        assert_eq!(
            content[0].get("type").and_then(|v| v.as_str()),
            Some("input_image")
        );
        assert_eq!(
            content[0].get("image_url").and_then(|v| v.as_str()),
            Some("data:image/png;base64,YWJj")
        );
        assert_eq!(
            content[1].get("type").and_then(|v| v.as_str()),
            Some("input_text")
        );
        assert_eq!(
            content[1].get("text").and_then(|v| v.as_str()),
            Some("Describe this image")
        );
    }

    #[test]
    fn streams_reasoning_summary_into_thinking_channel() {
        let mut state = CodexStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };
        let mut buffer = concat!(
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Plan first.\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Answer.\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":1}}}"
        )
        .to_string();

        let stopped = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &on_thinking,
            &ignore_tool,
        )
        .expect("reasoning summary should parse");

        assert!(stopped);
        assert_eq!(state.thinking_text, "Plan first.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Plan first."
        );
        assert_eq!(state.full_text, "Answer.");
    }

    #[test]
    fn build_request_body_includes_low_text_verbosity_for_gpt5_models() {
        let body = build_request_body(
            "gpt-5.4",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            None,
            None,
        );

        assert_eq!(body["text"]["verbosity"].as_str(), Some("low"));
    }

    #[test]
    fn recovers_reasoning_text_from_done_event() {
        let mut state = CodexStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };
        let mut buffer = concat!(
            "data: {\"type\":\"response.reasoning_text.done\",\"text\":\"Need to inspect the file.\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}"
        )
        .to_string();

        let stopped = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &on_thinking,
            &ignore_tool,
        )
        .expect("reasoning done event should parse");

        assert!(stopped);
        assert_eq!(state.thinking_text, "Need to inspect the file.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Need to inspect the file."
        );
    }

    #[test]
    fn captures_response_id_from_terminal_event() {
        let mut state = CodexStreamState::new();
        let mut buffer = concat!(
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_456\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}"
        )
        .to_string();

        let stopped = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("terminal event should parse");

        assert!(stopped);
        assert_eq!(state.response_id.as_deref(), Some("resp_456"));
    }

    #[test]
    fn websocket_request_wraps_response_create_type() {
        let body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [],
            "stream": true,
            "store": false,
        });
        let request =
            build_websocket_transport_request(&body, None, /*include_type_field*/ true);

        assert_eq!(
            request.get("type").and_then(|value| value.as_str()),
            Some("response.create")
        );
        assert_eq!(
            request.get("model").and_then(|value| value.as_str()),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn codex_websocket_url_uses_chatgpt_backend_endpoint() {
        let ws_url = codex_websocket_url(None).expect("websocket url");

        assert_eq!(
            ws_url.as_str(),
            "wss://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn codex_websocket_url_derives_from_provider_base_url() {
        let ws_url = codex_websocket_url(Some("https://example.test/backend-api/codex"))
            .expect("websocket url");

        assert_eq!(
            ws_url.as_str(),
            "wss://example.test/backend-api/codex/responses"
        );
    }

    #[test]
    fn websocket_handshake_request_includes_default_headers() {
        let ws_url = codex_websocket_url(None).expect("websocket url");
        let request = build_codex_websocket_handshake_request(
            &ws_url,
            "test-token",
            Some("account-123"),
            Some("session-456"),
            Some("sticky-turn"),
        )
        .expect("websocket request");

        assert_eq!(
            request
                .headers()
                .get("Authorization")
                .expect("authorization header")
                .to_str()
                .ok(),
            Some("Bearer test-token")
        );
        assert_eq!(
            request
                .headers()
                .get("originator")
                .expect("originator header")
                .to_str()
                .ok(),
            Some(CODEX_ORIGINATOR_HEADER_VALUE)
        );
        assert_eq!(
            request
                .headers()
                .get("version")
                .expect("version header")
                .to_str()
                .ok(),
            Some(CODEX_CLIENT_VERSION)
        );
        assert_eq!(
            request
                .headers()
                .get("OpenAI-Beta")
                .expect("OpenAI-Beta header")
                .to_str()
                .ok(),
            Some(RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE)
        );
        assert_eq!(
            request
                .headers()
                .get(X_CODEX_TURN_STATE_HEADER)
                .expect("turn-state header")
                .to_str()
                .ok(),
            Some("sticky-turn")
        );
        assert_eq!(
            request
                .headers()
                .get("ChatGPT-Account-ID")
                .expect("account header")
                .to_str()
                .ok(),
            Some("account-123")
        );
    }

    #[test]
    fn websocket_event_error_message_supports_wrapped_error_shape() {
        let message = websocket_event_error_message(
            r#"{"type":"error","status":429,"error":{"message":"usage limit reached"}}"#,
        );

        assert_eq!(
            message.as_deref(),
            Some("OpenAI Codex websocket error (HTTP 429): usage limit reached")
        );
    }

    #[test]
    fn history_transport_request_uses_previous_response_id_when_request_signature_matches() {
        let body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&[
                assistant_message("assistant-1", "call tools", Some("resp_prev")),
                tool_message("tool-1", "call_1", "done"),
                user_message_with_images("继续", vec![]),
            ]),
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
            "tools": [{"type":"function","name":"read","description":"Read a file","parameters":{"type":"object"}}],
            "tool_choice": "auto",
        });
        let request = build_history_transport_request(
            &body,
            &[
                assistant_message("assistant-1", "call tools", Some("resp_prev")),
                tool_message("tool-1", "call_1", "done"),
                user_message_with_images("继续", vec![]),
            ],
            Some(&response_request_metadata("assistant-1", &body)),
            /*include_type_field*/ true,
        );

        assert_eq!(
            request
                .get("previous_response_id")
                .and_then(|value| value.as_str()),
            Some("resp_prev")
        );
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(2)
        );
    }

    #[test]
    fn history_transport_request_uses_previous_response_id_with_server_tool_output() {
        let previous_assistant = assistant_message_with_tool_calls(
            "assistant-1",
            "",
            Some("resp_prev"),
            vec![ToolCallInfo {
                id: "ws_1".to_string(),
                name: "web_search".to_string(),
                arguments: r#"{"query":"rust async await"}"#.to_string(),
                server_tool: Some(ServerToolKind::WebSearch),
                server_tool_output: Some("Searched: rust async await".to_string()),
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }],
        );
        let history = vec![
            user_message_with_images("hello", vec![]),
            previous_assistant.clone(),
            user_message_with_images("继续", vec![]),
        ];
        let previous_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&history[..1]),
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
        });
        let current_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&history),
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
        });

        let request = build_history_transport_request(
            &current_body,
            &history,
            Some(&response_request_metadata("assistant-1", &previous_body)),
            /*include_type_field*/ true,
        );

        assert_eq!(
            request
                .get("previous_response_id")
                .and_then(|value| value.as_str()),
            Some("resp_prev")
        );
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(1)
        );
        assert_eq!(request["input"][0]["role"], serde_json::json!("user"));
    }

    #[test]
    fn history_transport_request_falls_back_to_full_replay_when_request_signature_differs() {
        let body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&[
                assistant_message("assistant-1", "server response", Some("resp_prev")),
                assistant_message("assistant-2", "local compact summary", None),
                user_message_with_images("继续", vec![]),
            ]),
            "stream": true,
            "store": false,
            "instructions": "new instructions",
        });
        let previous_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [],
            "stream": true,
            "store": false,
            "instructions": "old instructions",
        });
        let request = build_history_transport_request(
            &body,
            &[
                assistant_message("assistant-1", "server response", Some("resp_prev")),
                assistant_message("assistant-2", "local compact summary", None),
                user_message_with_images("继续", vec![]),
            ],
            Some(&response_request_metadata("assistant-1", &previous_body)),
            /*include_type_field*/ true,
        );

        assert!(request.get("previous_response_id").is_none());
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(3)
        );
    }

    #[test]
    fn websocket_transport_request_uses_cached_previous_response_id_when_request_signature_matches()
    {
        let previous_assistant = serde_json::json!({
            "role": "assistant",
            "content": [{ "type": "output_text", "text": "assistant output" }]
        });
        let previous_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [serde_json::json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello" }]
            })],
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
        });
        let current_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [
                serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }),
                previous_assistant.clone(),
                serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "second" }]
                })
            ],
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
        });

        let request = build_websocket_transport_request(
            &current_body,
            Some(&websocket_last_response(
                &previous_body,
                "resp_prev",
                std::slice::from_ref(&previous_assistant),
            )),
            /*include_type_field*/ true,
        );

        assert_eq!(
            request
                .get("previous_response_id")
                .and_then(|value| value.as_str()),
            Some("resp_prev")
        );
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(1)
        );
    }

    #[test]
    fn websocket_transport_request_starts_full_replay_without_cached_response() {
        let body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&[
                assistant_message("assistant-1", "call tools", Some("resp_prev")),
                tool_message("tool-1", "call_1", "done"),
                user_message_with_images("继续", vec![]),
            ]),
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
        });

        let request =
            build_websocket_transport_request(&body, None, /*include_type_field*/ true);

        assert!(request.get("previous_response_id").is_none());
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(3)
        );
    }

    #[test]
    fn websocket_event_error_message_supports_connection_limit_code() {
        let message = websocket_event_error_message(
            r#"{"type":"error","status":400,"error":{"code":"websocket_connection_limit_reached","message":"retry on a new connection"}}"#,
        );

        assert_eq!(
            message.as_deref(),
            Some(
                "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue."
            )
        );
    }

    #[test]
    fn websocket_proxy_match_uri_converts_wss_to_https() {
        let uri: http::Uri = "wss://api.openai.com/v1/responses?stream=1"
            .parse()
            .expect("valid websocket uri");

        let proxy_uri = websocket_proxy_match_uri(&uri).expect("proxy uri");

        assert_eq!(proxy_uri.scheme_str(), Some("https"));
        assert_eq!(proxy_uri.host(), Some("api.openai.com"));
        assert_eq!(proxy_uri.path(), "/v1/responses");
        assert_eq!(proxy_uri.query(), Some("stream=1"));
    }

    #[test]
    fn uri_host_port_uses_default_ports() {
        let https_uri: http::Uri = "https://api.openai.com/v1/responses"
            .parse()
            .expect("https uri");
        let socks_uri: http::Uri = "socks5://127.0.0.1".parse().expect("socks uri");

        assert_eq!(
            uri_host_port(&https_uri).expect("https host/port"),
            ("api.openai.com".to_string(), 443)
        );
        assert_eq!(
            uri_host_port(&socks_uri).expect("socks host/port"),
            ("127.0.0.1".to_string(), 1080)
        );
    }

    #[tokio::test]
    async fn establish_http_connect_tunnel_accepts_success_response() {
        let (client, mut server) = tokio::io::duplex(512);

        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; 256];
            let n = tokio::io::AsyncReadExt::read(&mut server, &mut buf)
                .await
                .expect("read connect request");
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.starts_with("CONNECT api.openai.com:443 HTTP/1.1\r\n"));
            tokio::io::AsyncWriteExt::write_all(&mut server, b"HTTP/1.1 200 OK\r\n\r\n")
                .await
                .expect("write connect response");
        });

        establish_http_connect_tunnel(client, "api.openai.com", 443, None)
            .await
            .expect("connect tunnel should succeed");

        server_task.await.expect("server task");
    }
}
