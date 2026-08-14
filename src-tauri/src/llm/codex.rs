use super::openai_reasoning::{apply_reasoning_effort, apply_text_verbosity_default};
use super::openrouter::LlmResponse;
use super::CODEX_CLIENT_VERSION;
use crate::commands::CodexTransportMode;
use crate::session::models::{ChatMessage, ImageData, MessageRole, ServerToolKind, ToolCallInfo};
use futures::{SinkExt, StreamExt};
use http::Uri;
use hyper_util::client::legacy::connect::proxy::SocksV4;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::proxy::matcher::Intercept;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_native_tls::TlsConnector as TokioTlsConnector;
use tokio_tungstenite::client_async_with_config;
use tokio_tungstenite::proxy::connect_via_proxy;
use tokio_tungstenite::tungstenite::extensions::compression::deflate::DeflateConfig;
use tokio_tungstenite::tungstenite::extensions::ExtensionsConfig;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::proxy::{
    ProxyAuth as TungsteniteProxyAuth, ProxyConfig as TungsteniteProxyConfig,
    ProxyScheme as TungsteniteProxyScheme,
};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use tower_service::Service;
use url::Url;

const DEFAULT_CODEX_PROVIDER_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const RESPONSES_ENDPOINT_PATH: &str = "/responses";
const RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const CODEX_BETA_FEATURES_HEADER: &str = "x-codex-beta-features";
const REMOTE_COMPACTION_V2_BETA_FEATURE: &str = "remote_compaction_v2";
const X_CODEX_ROUTING_HINT_HEADER: &str = "x-codex-routing-hint";
const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";
const WEBSOCKET_CONNECTION_LIMIT_REACHED_CODE: &str = "websocket_connection_limit_reached";
const WEBSOCKET_CONNECTION_LIMIT_REACHED_MESSAGE: &str = "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue.";
const PREVIOUS_RESPONSE_NOT_FOUND_CODE: &str = "previous_response_not_found";
const PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE: &str =
    "Previous response was not found. Retrying the full request.";
const CODEX_ORIGINATOR_HEADER_VALUE: &str = "opencode";
const MAX_SAFE_STREAM_RECOVERY_RETRIES: u32 = 2;
const SAFE_STREAM_RECOVERY_DELAY_MS: u64 = 1200;
const WEBSOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const WEBSOCKET_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

trait CodexAsyncIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

impl<T> CodexAsyncIo for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

type BoxedCodexIo = Box<dyn CodexAsyncIo>;
type CodexWebSocket = tokio_tungstenite::WebSocketStream<BoxedCodexIo>;

struct CodexWebsocketStream {
    tx_command: mpsc::Sender<CodexWebsocketCommand>,
    rx_message: mpsc::UnboundedReceiver<Result<Message, WsError>>,
    pump_task: tokio::task::JoinHandle<()>,
}

enum CodexWebsocketCommand {
    Send {
        message: Message,
        tx_result: oneshot::Sender<Result<(), WsError>>,
    },
}

impl CodexWebsocketStream {
    fn new(inner: CodexWebSocket) -> Self {
        let (tx_command, mut rx_command) = mpsc::channel::<CodexWebsocketCommand>(32);
        let (tx_message, rx_message) = mpsc::unbounded_channel::<Result<Message, WsError>>();

        let pump_task = tokio::spawn(async move {
            let mut inner = inner;
            loop {
                tokio::select! {
                    command = rx_command.recv() => {
                        let Some(command) = command else {
                            break;
                        };
                        match command {
                            CodexWebsocketCommand::Send { message, tx_result } => {
                                let result = inner.send(message).await;
                                let should_break = result.is_err();
                                let _ = tx_result.send(result);
                                if should_break {
                                    break;
                                }
                            }
                        }
                    }
                    message = inner.next() => {
                        let Some(message) = message else {
                            break;
                        };
                        match message {
                            Ok(Message::Ping(payload)) => {
                                if let Err(error) = inner.send(Message::Pong(payload)).await {
                                    let _ = tx_message.send(Err(error));
                                    break;
                                }
                            }
                            Ok(Message::Pong(_)) => {}
                            Ok(message @ (Message::Text(_)
                            | Message::Binary(_)
                            | Message::Close(_)
                            | Message::Frame(_))) => {
                                let is_close = matches!(message, Message::Close(_));
                                if tx_message.send(Ok(message)).is_err() {
                                    break;
                                }
                                if is_close {
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = tx_message.send(Err(error));
                                break;
                            }
                        }
                    }
                }
            }
        });

        Self {
            tx_command,
            rx_message,
            pump_task,
        }
    }

    async fn send(&self, message: Message) -> Result<(), WsError> {
        let (tx_result, rx_result) = oneshot::channel();
        if self
            .tx_command
            .send(CodexWebsocketCommand::Send { message, tx_result })
            .await
            .is_err()
        {
            return Err(WsError::ConnectionClosed);
        }
        rx_result.await.unwrap_or(Err(WsError::ConnectionClosed))
    }

    async fn next(&mut self) -> Option<Result<Message, WsError>> {
        self.rx_message.recv().await
    }
}

impl Drop for CodexWebsocketStream {
    fn drop(&mut self) {
        self.pump_task.abort();
    }
}

#[derive(Debug, Default)]
pub struct TurnState {
    sticky_routing_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodexStreamOptions {
    pub include_web_search: bool,
    pub use_session_continuation: bool,
    pub fast_mode: bool,
    remote_compaction_v2: bool,
    structured_output: Option<CodexStructuredOutput>,
}

#[derive(Debug, Clone)]
struct CodexStructuredOutput {
    name: String,
    schema: serde_json::Value,
}

impl Default for CodexStreamOptions {
    fn default() -> Self {
        Self {
            include_web_search: true,
            use_session_continuation: true,
            fast_mode: false,
            remote_compaction_v2: false,
            structured_output: None,
        }
    }
}

impl CodexStreamOptions {
    pub fn compact() -> Self {
        Self {
            include_web_search: false,
            use_session_continuation: false,
            fast_mode: false,
            remote_compaction_v2: false,
            structured_output: None,
        }
    }

    /// Codex main's stable remote-compaction path: send the current prompt to
    /// `/responses` with a terminal `compaction_trigger` request item and
    /// expect exactly one encrypted compaction output item.
    pub fn remote_compaction_v2() -> Self {
        Self {
            include_web_search: true,
            use_session_continuation: false,
            fast_mode: false,
            remote_compaction_v2: true,
            structured_output: None,
        }
    }

    pub fn with_fast_mode(mut self, enabled: bool) -> Self {
        self.fast_mode = enabled;
        self
    }

    pub fn with_output_schema(
        mut self,
        name: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        self.structured_output = Some(CodexStructuredOutput {
            name: name.into(),
            schema,
        });
        self
    }
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
    connection: Option<CodexWebsocketStream>,
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

/// Extracts the canonical replacement window stored on a context-handoff
/// message. Standalone `/responses/compact` output must be replayed as-is; it
/// generally includes retained response items in addition to the opaque
/// compaction item.
fn codex_compaction_output(metadata: &serde_json::Value) -> Option<&[serde_json::Value]> {
    metadata
        .get("codex_compaction")?
        .get("output")?
        .as_array()
        .filter(|output| !output.is_empty())
        .map(Vec::as_slice)
}

/// Reads the legacy single-item representation and the optional diagnostic
/// copy retained alongside canonical replacement windows.
fn codex_compaction_encrypted_content(metadata: &serde_json::Value) -> Option<&str> {
    metadata
        .get("codex_compaction")?
        .get("encrypted_content")?
        .as_str()
        .filter(|content| !content.is_empty())
}

/// Reserved name tagging client `tool_search` rounds in session history.
/// Matching assistant tool calls and their outputs replay as the typed
/// `tool_search_call` / `tool_search_output` wire items instead of
/// `function_call` items (the API requires the round-trip in that shape).
pub const TOOL_SEARCH_HISTORY_TOOL_NAME: &str = "tool_search";

fn build_tool_search_call_item(tc: &ToolCallInfo) -> serde_json::Value {
    // The wire item carries `arguments` as a JSON value (unlike
    // function_call's string form); the stored string is its serialization.
    let arguments = serde_json::from_str::<serde_json::Value>(&tc.arguments)
        .unwrap_or_else(|_| serde_json::json!({}));
    serde_json::json!({
        "type": "tool_search_call",
        "call_id": tc.id,
        "status": "completed",
        "execution": "client",
        "arguments": arguments,
    })
}

fn build_tool_search_output_item(call_id: &str, content: Option<&str>) -> serde_json::Value {
    // Missing or unparsable output (interrupted round, error text) degrades
    // to the empty patch — the same normalization codex itself applies.
    let tools = content
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|value| value.get("tools").cloned())
        .filter(|tools| tools.is_array())
        .unwrap_or_else(|| serde_json::json!([]));
    serde_json::json!({
        "type": "tool_search_output",
        "call_id": call_id,
        "status": "completed",
        "execution": "client",
        "tools": tools,
    })
}

fn build_input(history: &[ChatMessage]) -> Vec<serde_json::Value> {
    build_input_with_metadata(history, None)
}

fn build_input_with_metadata(
    history: &[ChatMessage],
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
) -> Vec<serde_json::Value> {
    // Match codex-rs history normalization at the final transport boundary.
    // A historical fork or interrupted run may end with a persisted function
    // call whose tool message was never copied. Responses requires a matching
    // output for every call, including remote-compaction requests.
    let normalized_history =
        crate::session::history::normalize_tool_round_history_for_request(history);
    let history = normalized_history.as_slice();
    // call_ids of tool_search rounds; their Tool messages must replay as
    // tool_search_output items rather than function_call_output.
    let tool_search_call_ids: std::collections::HashSet<&str> = history
        .iter()
        .filter(|msg| msg.role == MessageRole::Assistant)
        .filter_map(|msg| msg.tool_calls.as_ref())
        .flatten()
        .filter(|tc| tc.name == TOOL_SEARCH_HISTORY_TOOL_NAME && !tc.is_server_tool())
        .map(|tc| tc.id.as_str())
        .collect();
    let tool_output_call_ids: std::collections::HashSet<&str> = history
        .iter()
        .filter(|msg| msg.role == MessageRole::Tool)
        .filter_map(|msg| msg.tool_call_id.as_deref())
        .collect();

    let mut input = Vec::new();
    for msg in history {
        if let Some(output) = response_request_metadata
            .and_then(|metadata| metadata.get(&msg.id))
            .and_then(codex_compaction_output)
        {
            input.extend(output.iter().cloned());
            continue;
        }
        if let Some(encrypted_content) = response_request_metadata
            .and_then(|metadata| metadata.get(&msg.id))
            .and_then(codex_compaction_encrypted_content)
        {
            // The handoff text is a local fallback for non-Codex backends; the
            // Codex API receives the original compaction item instead.
            input.push(serde_json::json!({
                "type": "compaction",
                "encrypted_content": encrypted_content,
            }));
            continue;
        }
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
                        // Hosted tools have already executed inside the provider response.
                        // Replaying them as client function calls fabricates an unresolved
                        // tool round and can make the model answer the same request again.
                        if tc.is_server_tool() {
                            continue;
                        }
                        if tc.name == TOOL_SEARCH_HISTORY_TOOL_NAME {
                            input.push(build_tool_search_call_item(tc));
                            if !tool_output_call_ids.contains(tc.id.as_str()) {
                                // A call without its output would 400; patch
                                // with the empty result like codex normalize.
                                input.push(build_tool_search_output_item(&tc.id, None));
                            }
                            continue;
                        }
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }));
                    }
                }
            }
            MessageRole::Tool => {
                if let Some(ref call_id) = msg.tool_call_id {
                    if tool_search_call_ids.contains(call_id.as_str()) {
                        input.push(build_tool_search_output_item(call_id, Some(&msg.content)));
                        continue;
                    }
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": build_tool_output_content(&msg.content, msg.images.as_deref()),
                    }));
                }
            }
        }
    }
    input
}

fn build_tool_output_content(text: &str, images: Option<&[ImageData]>) -> serde_json::Value {
    let Some(images) = images.filter(|images| !images.is_empty()) else {
        return serde_json::Value::String(text.to_string());
    };

    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(serde_json::json!({
            "type": "input_text",
            "text": text,
        }));
    }
    for img in images {
        content.push(serde_json::json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", img.mime_type, img.data),
        }));
    }
    if content.is_empty() {
        content.push(serde_json::json!({
            "type": "input_text",
            "text": "",
        }));
    }
    serde_json::Value::Array(content)
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
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    options: CodexStreamOptions,
) -> serde_json::Value {
    let input = build_input_with_metadata(history, response_request_metadata);
    let mut responses_tools = convert_tools(tools);

    if options.include_web_search {
        // Inject web_search server tool (executed by OpenAI API, not locally).
        responses_tools.push(serde_json::json!({
            "type": "web_search",
            "external_web_access": true,
        }));
    }

    let mut body = serde_json::json!({
        "model": model,
        "input": input,
        "stream": true,
        "store": false,
    });

    if options.remote_compaction_v2 {
        body["input"]
            .as_array_mut()
            .expect("Codex request input must be an array")
            .push(serde_json::json!({ "type": "compaction_trigger" }));
    }

    if options.use_session_continuation {
        if let Some(sid) = session_id {
            body["prompt_cache_key"] = serde_json::json!(sid);
        }
    }

    if !system_prompt.is_empty() {
        body["instructions"] = serde_json::json!(system_prompt);
    }

    apply_reasoning_effort(&mut body, model, thinking_level);
    apply_text_verbosity_default(&mut body, model);
    if let Some(output) = options.structured_output.as_ref() {
        body["text"]["format"] = serde_json::json!({
            "type": "json_schema",
            "name": output.name,
            "strict": true,
            "schema": output.schema,
        });
    }
    if options.fast_mode {
        body["service_tier"] = serde_json::json!("priority");
    }

    if !responses_tools.is_empty() {
        body["tools"] = serde_json::json!(responses_tools);
        body["tool_choice"] = serde_json::json!("auto");
    }

    body
}

/// Typed `tool_search` declaration (client execution). The description is
/// source-level and deterministic — request-signature stability is what keeps
/// websocket continuation and the server prompt cache alive.
fn build_tool_search_declaration(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": description,
        "parameters": {
            "type": "object",
            "properties": {
                "wire_names": {
                    "type": "array",
                    "description": "One to eight complete deferred-tool wire names copied verbatim from the prompt, a Skill document, or a tool result. Include only tools required for the current step. Natural-language queries and aliases are rejected.",
                    "items": {
                        "type": "string",
                        "minLength": 1
                    },
                    "minItems": 1,
                    "maxItems": 8,
                    "uniqueItems": true
                }
            },
            "required": ["wire_names"],
            "additionalProperties": false
        }
    })
}

fn build_request_body_with_tool_search(
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    tool_search_description: Option<&str>,
    thinking_level: Option<&str>,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    options: CodexStreamOptions,
) -> serde_json::Value {
    let mut body = build_request_body(
        model,
        system_prompt,
        history,
        tools,
        thinking_level,
        session_id,
        response_request_metadata,
        options,
    );

    if let Some(description) = tool_search_description {
        let declaration = build_tool_search_declaration(description);
        match body.get_mut("tools").and_then(|tools| tools.as_array_mut()) {
            Some(existing) => existing.push(declaration),
            None => {
                body["tools"] = serde_json::json!([declaration]);
                body["tool_choice"] = serde_json::json!("auto");
            }
        }
    }

    body
}

fn request_without_input(body: &serde_json::Value) -> serde_json::Value {
    let mut request = body.clone();
    if let Some(map) = request.as_object_mut() {
        map.remove("input");
        map.remove("previous_response_id");
        map.remove("type");
        map.remove("tools");
        map.remove("tool_choice");
    }
    request
}

// Keep websocket reuse checks aligned with codex-rs: input is compared item by
// item, while every request property that affects the response remains part of
// the signature. Transport-only metadata does not invalidate a continuation.
fn websocket_request_signature(body: &serde_json::Value) -> serde_json::Value {
    let mut request = body.clone();
    if let Some(map) = request.as_object_mut() {
        map.remove("input");
        map.remove("previous_response_id");
        map.remove("type");
        map.remove("client_metadata");
        map.remove("stream_options");
    }
    request
}

fn clear_internal_response_item_metadata(item: &mut serde_json::Value) {
    if let Some(map) = item.as_object_mut() {
        map.remove("internal_chat_message_metadata_passthrough");
    }
}

fn response_items_equal_ignoring_internal_metadata(
    previous: &serde_json::Value,
    current: &serde_json::Value,
) -> bool {
    if previous == current {
        return true;
    }

    let mut previous = previous.clone();
    clear_internal_response_item_metadata(&mut previous);
    let mut current = current.clone();
    clear_internal_response_item_metadata(&mut current);
    previous == current
}

// Locus persists assistant text and tool calls in ChatMessage rather than the
// raw Responses output item. Project server output back to the request shape
// produced by build_input_with_metadata before comparing the next full input.
fn cached_response_item_for_request(
    response_item: &serde_json::Value,
) -> Option<serde_json::Value> {
    let item_type = response_item.get("type").and_then(|value| value.as_str());
    if matches!(item_type, Some("reasoning" | "web_search_call")) {
        // The active websocket keeps reasoning and hosted-tool state in the
        // previous response. Locus does not replay either item from its
        // reconstructed ChatMessage history.
        return None;
    }

    let mut item = response_item.clone();
    clear_internal_response_item_metadata(&mut item);
    let Some(map) = item.as_object_mut() else {
        return Some(item);
    };

    map.remove("id");
    match item_type {
        Some("message") => {
            map.remove("type");
            map.remove("status");
            map.remove("phase");
            if let Some(content) = map
                .get_mut("content")
                .and_then(|value| value.as_array_mut())
            {
                for part in content {
                    if let Some(part) = part.as_object_mut() {
                        part.remove("annotations");
                        part.remove("logprobs");
                    }
                }
            }
        }
        Some("function_call") => {
            map.remove("status");
        }
        _ => {}
    }
    Some(item)
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
        .unwrap_or_else(|| build_input_with_metadata(history, response_request_metadata));
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

            let baseline =
                build_input_with_metadata(&history[..=index], Some(response_request_metadata));
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
    let current_request = websocket_request_signature(body);

    if let Some(last_response) = last_response {
        if last_response.request_signature == current_request {
            if last_response.response_id.trim().is_empty() {
                return ContinuationRequestInput {
                    input: full_input,
                    previous_response_id: None,
                };
            }
            let previous_input_matches = last_response.input.len() <= full_input.len()
                && last_response
                    .input
                    .iter()
                    .zip(&full_input)
                    .all(|(previous, current)| {
                        response_items_equal_ignoring_internal_metadata(previous, current)
                    });
            let mut incremental_start = last_response.input.len();
            let response_items_match = previous_input_matches
                && last_response.items_added.iter().all(|response_item| {
                    let Some(previous) = cached_response_item_for_request(response_item) else {
                        return true;
                    };
                    let Some(current) = full_input.get(incremental_start) else {
                        return false;
                    };
                    if !response_items_equal_ignoring_internal_metadata(&previous, current) {
                        return false;
                    }
                    incremental_start += 1;
                    true
                });
            if response_items_match {
                return ContinuationRequestInput {
                    input: full_input[incremental_start..].to_vec(),
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
    use_previous_response_id: bool,
) -> serde_json::Value {
    let request_input = if use_previous_response_id {
        build_history_request_input(body, history, response_request_metadata)
    } else {
        ContinuationRequestInput {
            input: body
                .get("input")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_else(|| build_input_with_metadata(history, response_request_metadata)),
            previous_response_id: None,
        }
    };
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
    let configured_base_url = codex_provider_base_url(base_url).trim_end_matches('/');
    // `base_url` is shared with other providers in the app. Normalize the
    // common ChatGPT backend root to Codex's provider prefix so a valid global
    // override cannot accidentally produce `/backend-api/responses` (404).
    let base_url = if configured_base_url.eq_ignore_ascii_case("https://chatgpt.com/backend-api") {
        DEFAULT_CODEX_PROVIDER_BASE_URL
    } else {
        configured_base_url
    };
    if base_url.ends_with(RESPONSES_ENDPOINT_PATH) {
        base_url.to_string()
    } else {
        format!("{base_url}{RESPONSES_ENDPOINT_PATH}")
    }
}

fn codex_compact_endpoint(base_url: Option<&str>) -> String {
    format!("{}/compact", codex_responses_endpoint(base_url))
}

// `/responses/compact` is unary, so this covers the full response rather than
// one idle period between stream events (codex-rs uses idle timeout x4).
const COMPACT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

pub struct CodexRemoteCompactOutcome {
    pub output: Vec<serde_json::Value>,
    pub encrypted_content: Option<String>,
    pub raw_request: String,
    pub raw_response: String,
}

#[derive(Debug)]
pub struct CodexRemoteCompactError {
    pub message: String,
    pub raw_request: String,
    pub raw_response: String,
}

impl CodexRemoteCompactError {
    fn new(
        message: impl Into<String>,
        raw_request: impl Into<String>,
        raw_response: impl Into<String>,
    ) -> Self {
        Self {
            message: message.into(),
            raw_request: raw_request.into(),
            raw_response: raw_response.into(),
        }
    }
}

impl fmt::Display for CodexRemoteCompactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodexRemoteCompactError {}

/// Mirrors codex-rs `ApiCompactionInput`: the same shape as a normal Responses
/// request minus `stream`/`store`, sent to the unary compact endpoint.
fn build_compact_request_body(
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    fast_mode: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let input = build_input_with_metadata(history, response_request_metadata);
    let mut body = serde_json::json!({
        "model": model,
        "input": input,
        "tools": convert_tools(tools),
        // Locus does not receive the server models manifest; codex-rs defaults
        // ModelInfo.supports_parallel_tool_calls to false.
        "parallel_tool_calls": false,
    });
    if !system_prompt.is_empty() {
        body["instructions"] = serde_json::json!(system_prompt);
    }
    if let Some(sid) = session_id {
        body["prompt_cache_key"] = serde_json::json!(sid);
    }
    apply_reasoning_effort(&mut body, model, thinking_level);
    apply_text_verbosity_default(&mut body, model);
    if fast_mode {
        body["service_tier"] = serde_json::json!("priority");
    }
    body
}

fn extract_compaction_encrypted_content(output: &[serde_json::Value]) -> Option<String> {
    output
        .iter()
        .rev()
        .find(|item| {
            matches!(
                item.get("type").and_then(|value| value.as_str()),
                Some("compaction" | "compaction_summary" | "context_compaction")
            )
        })
        .and_then(|item| item.get("encrypted_content"))
        .and_then(|value| value.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| content.to_string())
}

fn compact_output_type_summary(output: &[serde_json::Value]) -> String {
    let mut counts = BTreeMap::<&str, usize>::new();
    for item in output {
        let item_type = item
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("<missing>");
        *counts.entry(item_type).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(item_type, count)| format!("{}={}", item_type, count))
        .collect::<Vec<_>>()
        .join(",")
}

fn codex_routing_hint(model: &str, fast_mode: bool) -> String {
    if fast_mode {
        format!("model={model};tier=priority")
    } else {
        format!("model={model}")
    }
}

fn parse_compact_response(
    raw_request: String,
    response_text: String,
) -> Result<CodexRemoteCompactOutcome, CodexRemoteCompactError> {
    let parsed: serde_json::Value = serde_json::from_str(&response_text).map_err(|error| {
        CodexRemoteCompactError::new(
            format!("Codex compact response was not valid JSON: {}", error),
            raw_request.clone(),
            response_text.clone(),
        )
    })?;
    let output = parsed
        .get("output")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if output.is_empty() {
        return Err(CodexRemoteCompactError::new(
            "Codex compact response contained an empty canonical output window",
            raw_request,
            response_text,
        ));
    }
    let encrypted_content = extract_compaction_encrypted_content(&output);
    Ok(CodexRemoteCompactOutcome {
        output,
        encrypted_content,
        raw_request,
        raw_response: response_text,
    })
}

/// Runs remote conversation compaction against the dedicated Codex
/// `POST /responses/compact` endpoint (the default codex-rs strategy for
/// ChatGPT subscription providers). Returns the complete canonical replacement
/// window that must be replayed to the API in subsequent requests.
pub async fn compact_conversation_history(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    fast_mode: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    debug: bool,
) -> Result<CodexRemoteCompactOutcome, CodexRemoteCompactError> {
    let body = build_compact_request_body(
        model,
        system_prompt,
        history,
        tools,
        thinking_level,
        fast_mode,
        session_id,
        response_request_metadata,
    );
    let raw_request = serde_json::to_string_pretty(&body).unwrap_or_default();
    let api_url = codex_compact_endpoint(base_url);
    let routing_hint = codex_routing_hint(model, fast_mode);

    eprintln!(
        "[OpenAI Codex][compact] POST model={} messages={} tools={}",
        model,
        history.len(),
        tools.len()
    );
    if debug {
        eprintln!(
            "[DEBUG][OpenAI Codex][compact] request body:\n{}",
            &raw_request
        );
        let mut headers: Vec<(&str, &str)> = vec![
            ("Authorization", "Bearer <token>"),
            ("Content-Type", "application/json"),
            ("originator", CODEX_ORIGINATOR_HEADER_VALUE),
            ("version", CODEX_CLIENT_VERSION),
            (X_CODEX_ROUTING_HINT_HEADER, routing_hint.as_str()),
        ];
        if let Some(sid) = session_id {
            headers.push(("session-id", sid));
            headers.push(("thread-id", sid));
        }
        if let Some(aid) = account_id {
            headers.push(("ChatGPT-Account-ID", aid));
        }
        super::debug::save_request("openai_codex_compact", &api_url, &headers, &raw_request);
    }

    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .tcp_keepalive(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(30)),
    )
    .map_err(|error| CodexRemoteCompactError::new(error, raw_request.clone(), ""))?;
    let mut req = client
        .post(&api_url)
        .timeout(COMPACT_REQUEST_TIMEOUT)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("originator", CODEX_ORIGINATOR_HEADER_VALUE)
        .header("version", CODEX_CLIENT_VERSION)
        .header(X_CODEX_ROUTING_HINT_HEADER, &routing_hint)
        .json(&body);
    if let Some(sid) = session_id {
        req = req.header("session-id", sid).header("thread-id", sid);
    }
    if let Some(aid) = account_id {
        req = req.header("ChatGPT-Account-ID", aid);
    }

    let resp = req.send().await.map_err(|error| {
        CodexRemoteCompactError::new(
            format!("Codex compact request failed: {}", error),
            raw_request.clone(),
            "",
        )
    })?;
    let status = resp.status();
    let response_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CodexRemoteCompactError::new(
            format!(
                "OpenAI Codex API error ({} {}): {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                response_text
            ),
            raw_request,
            response_text,
        ));
    }

    let outcome = parse_compact_response(raw_request, response_text)?;
    eprintln!(
        "[OpenAI Codex][compact] canonical output items={} types={}",
        outcome.output.len(),
        compact_output_type_summary(&outcome.output)
    );
    Ok(outcome)
}

const REMOTE_COMPACTION_V2_RETAINED_BYTES: usize = 64_000 * 4;

fn retained_remote_compaction_v2_window(
    history: &[ChatMessage],
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    compaction_item: serde_json::Value,
) -> Vec<serde_json::Value> {
    let input = build_input_with_metadata(history, response_request_metadata);
    let mut remaining = REMOTE_COMPACTION_V2_RETAINED_BYTES;
    let mut retained_reversed = Vec::new();

    for item in input.into_iter().rev() {
        let role = item.get("role").and_then(|value| value.as_str());
        // Locus stores assistant entries as completed answers. Codex main
        // excludes final agent answers from the retained V2 window, while
        // retaining user/developer/system context around the opaque summary.
        if !matches!(role, Some("user" | "developer" | "system")) {
            continue;
        }
        let serialized_bytes = serde_json::to_vec(&item).map_or(usize::MAX, |value| value.len());
        if serialized_bytes > remaining {
            continue;
        }
        remaining = remaining.saturating_sub(serialized_bytes);
        retained_reversed.push(item);
    }

    retained_reversed.reverse();
    retained_reversed.push(compaction_item);
    retained_reversed
}

fn validate_remote_compaction_v2_output(
    response_items: &[serde_json::Value],
    response_completed: bool,
) -> Result<serde_json::Value, String> {
    if !response_completed {
        return Err(
            "Codex remote compaction V2 stream ended without response.completed".to_string(),
        );
    }
    if response_items.len() != 1
        || response_items[0]
            .get("type")
            .and_then(|value| value.as_str())
            != Some("compaction")
    {
        return Err(format!(
            "Codex remote compaction V2 expected exactly one compaction output item, got {} output items with types {:?}",
            response_items.len(),
            response_items
                .iter()
                .map(|item| item.get("type").and_then(|value| value.as_str()))
                .collect::<Vec<_>>()
        ));
    }
    Ok(response_items[0].clone())
}

/// Runs Codex main's current stable remote-compaction protocol. V2 uses the
/// ordinary Responses transport with a terminal `compaction_trigger` instead
/// of the legacy unary `/responses/compact` endpoint.
pub async fn compact_conversation_history_v2(
    access_token: &str,
    account_id: Option<&str>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    fast_mode: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    debug: bool,
) -> Result<CodexRemoteCompactOutcome, CodexRemoteCompactError> {
    let mut turn_state = TurnState::default();
    let response = stream_chat_with_options(
        access_token,
        account_id,
        transport,
        base_url,
        model,
        system_prompt,
        history,
        tools,
        None,
        thinking_level,
        debug,
        session_id,
        response_request_metadata,
        &mut turn_state,
        CodexStreamOptions::remote_compaction_v2().with_fast_mode(fast_mode),
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .await
    .map_err(|error| CodexRemoteCompactError::new(error, "", ""))?;

    let compaction_item =
        validate_remote_compaction_v2_output(&response.response_items, response.response_completed)
            .map_err(|error| {
                CodexRemoteCompactError::new(
                    error,
                    response.raw_request.clone(),
                    response.raw_response.clone(),
                )
            })?;
    let output =
        retained_remote_compaction_v2_window(history, response_request_metadata, compaction_item);
    let encrypted_content = extract_compaction_encrypted_content(&output);
    Ok(CodexRemoteCompactOutcome {
        output,
        encrypted_content,
        raw_request: response.raw_request,
        raw_response: response.raw_response,
    })
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
    routing_hint: Option<&str>,
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
        CODEX_BETA_FEATURES_HEADER,
        http::HeaderValue::from_static(REMOTE_COMPACTION_V2_BETA_FEATURE),
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
        request
            .headers_mut()
            .insert("session-id", header_value.clone());
        request.headers_mut().insert("thread-id", header_value);
    }
    if let Some(routing_hint) = routing_hint {
        request.headers_mut().insert(
            X_CODEX_ROUTING_HINT_HEADER,
            http::HeaderValue::from_str(routing_hint)
                .map_err(|e| format!("Failed to build routing-hint header: {}", e))?,
        );
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
    Option<CodexWebsocketStream>,
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
    socket: CodexWebsocketStream,
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

async fn cached_websocket_http_fallback_enabled(
    session_id: Option<&str>,
    base_url: Option<&str>,
    account_id: Option<&str>,
) -> bool {
    let Some(session_id) = session_id else {
        return false;
    };
    let Some(shared) = existing_cached_websocket_session(session_id) else {
        return false;
    };
    let connection_key = websocket_connection_key(base_url, account_id);
    let state = shared.lock().await;
    state.connection_key.as_deref() == Some(connection_key.as_str()) && state.disable_websockets
}

async fn enable_cached_websocket_http_fallback(
    session_id: Option<&str>,
    base_url: Option<&str>,
    account_id: Option<&str>,
) {
    let Some(session_id) = session_id else {
        return;
    };
    let connection_key = websocket_connection_key(base_url, account_id);
    let shared = cached_websocket_session(session_id);
    clear_cached_websocket_session_state(
        &shared,
        &connection_key,
        /*disable_websockets*/ true,
    )
    .await;
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

fn tungstenite_proxy_config(
    proxy: &Intercept,
    scheme: TungsteniteProxyScheme,
) -> Result<TungsteniteProxyConfig, String> {
    let (host, port) = uri_host_port(proxy.uri())?;
    let auth = proxy
        .raw_auth()
        .map(|(username, password)| TungsteniteProxyAuth {
            username: username.to_string(),
            password: password.to_string(),
        });
    Ok(TungsteniteProxyConfig {
        scheme,
        host,
        port,
        auth,
    })
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

    let (target_host, target_port) = uri_host_port(target_uri)?;
    let proxy_config = tungstenite_proxy_config(proxy, TungsteniteProxyScheme::Http)?;
    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let tunneled = connect_via_proxy(tcp, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish HTTP proxy tunnel: {}", e))?;
    Ok(Box::new(tunneled))
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
    let proxy_config = tungstenite_proxy_config(proxy, TungsteniteProxyScheme::Http)?;

    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let proxy_tls = tls_connector()?
        .connect(&proxy_host, tcp)
        .await
        .map_err(|e| format!("Failed to establish TLS to HTTPS proxy: {}", e))?;
    let tunneled = connect_via_proxy(proxy_tls, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish HTTPS proxy tunnel: {}", e))?;

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

    let scheme = match proxy.uri().scheme_str() {
        Some("socks5h") => TungsteniteProxyScheme::Socks5h,
        _ => TungsteniteProxyScheme::Socks5,
    };
    let (target_host, target_port) = uri_host_port(target_uri)?;
    let proxy_config = tungstenite_proxy_config(proxy, scheme)?;
    let tcp = connect_tcp_stream(&proxy_uri).await?;
    let tunneled = connect_via_proxy(tcp, &proxy_config, &target_host, target_port)
        .await
        .map_err(|e| format!("Failed to establish SOCKS5 proxy tunnel: {}", e))?;
    Ok(Box::new(tunneled))
}

async fn connect_websocket_transport(request: &http::Request<()>) -> Result<BoxedCodexIo, String> {
    let target_uri = websocket_proxy_match_uri(request.uri())?;
    let matcher = crate::network::proxy_matcher();

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
) -> Result<WebsocketConnectOutcome<CodexWebsocketStream>, String> {
    let connect = async move {
        let transport = connect_websocket_transport(&request)
            .await
            .map_err(ws_io_error)?;
        let transport = wrap_websocket_transport_tls(&request, transport)
            .await
            .map_err(ws_io_error)?;
        client_async_with_config(request, transport, Some(websocket_config())).await
    };

    match tokio::time::timeout(WEBSOCKET_CONNECT_TIMEOUT, connect).await {
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

            Ok(WebsocketConnectOutcome::Connected(
                CodexWebsocketStream::new(socket),
            ))
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

fn websocket_config() -> WebSocketConfig {
    let mut extensions = ExtensionsConfig::default();
    extensions.permessage_deflate = Some(DeflateConfig::default());

    let mut config = WebSocketConfig::default();
    config.extensions = extensions;
    config
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
    if code == Some(PREVIOUS_RESPONSE_NOT_FOUND_CODE) {
        return Some(PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE.to_string());
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
                    order: None,
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
    end_turn: Option<bool>,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    response_id: Option<String>,
    items_added: Vec<serde_json::Value>,
    got_terminal_event: bool,
    got_completed_event: bool,
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
            end_turn: None,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            response_id: None,
            items_added: Vec::new(),
            got_terminal_event: false,
            got_completed_event: false,
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
                        } else if item_type == Some("tool_search_call") {
                            // Client-executed tool discovery. The arguments
                            // arrive as a JSON value; the authoritative copy
                            // lands on output_item.done, which upgrades this
                            // entry and fires the start notification.
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let item_id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&call_id)
                                .to_string();
                            if !call_id.is_empty() {
                                state.tool_calls_map.insert(
                                    item_id,
                                    PartialToolCall {
                                        call_id,
                                        name: TOOL_SEARCH_HISTORY_TOOL_NAME.to_string(),
                                        arguments: item
                                            .get("arguments")
                                            .map(|v| {
                                                if let Some(text) = v.as_str() {
                                                    text.to_string()
                                                } else {
                                                    v.to_string()
                                                }
                                            })
                                            .unwrap_or_default(),
                                        arguments_done: false,
                                        item_done: false,
                                        notified: false,
                                        start_order: None,
                                    },
                                );
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
                        } else if item_type == Some("tool_search_call") {
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let item_id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&call_id)
                                .to_string();
                            // Arguments arrive as a JSON value; store its
                            // serialization (the executor and history replay
                            // parse it back).
                            let arguments = item
                                .get("arguments")
                                .map(|v| {
                                    if let Some(text) = v.as_str() {
                                        text.to_string()
                                    } else {
                                        v.to_string()
                                    }
                                })
                                .unwrap_or_default();
                            if !call_id.is_empty() {
                                let entry =
                                    state.tool_calls_map.entry(item_id).or_insert_with(|| {
                                        PartialToolCall {
                                            call_id,
                                            name: TOOL_SEARCH_HISTORY_TOOL_NAME.to_string(),
                                            arguments: String::new(),
                                            arguments_done: false,
                                            item_done: false,
                                            notified: false,
                                            start_order: None,
                                        }
                                    });
                                if !arguments.is_empty() {
                                    entry.arguments = arguments;
                                }
                                entry.item_done = true;
                                entry.notify_started(
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
                                    order: None,
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
                    state.got_completed_event = event.get("type").and_then(|value| value.as_str())
                        == Some("response.completed");
                    state.finish_thinking_timing();
                    if let Some(response) = event.get("response") {
                        state.end_turn = response.get("end_turn").and_then(|value| value.as_bool());
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

    if lower.contains("previous response was not found")
        || (lower.contains("previous response with id") && lower.contains("not found"))
    {
        return true;
    }

    if lower.contains("codex websocket connect failed")
        || lower.contains("codex websocket connect timed out")
        || lower.contains("failed to send websocket request")
        || lower.starts_with("websocket read error:")
        || lower == "websocket read timed out"
    {
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
            || lower.contains("websocket ended before the response finalized")
            || lower.contains("response completed with"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeStreamRecoveryAction {
    Retry,
    FallbackToHttp,
    Fail,
}

fn safe_stream_recovery_action(
    transport: CodexTransportMode,
    retries: u32,
    error: &str,
) -> SafeStreamRecoveryAction {
    if !should_retry_safe_codex_error(error) {
        return SafeStreamRecoveryAction::Fail;
    }
    if retries < MAX_SAFE_STREAM_RECOVERY_RETRIES {
        return SafeStreamRecoveryAction::Retry;
    }
    if transport == CodexTransportMode::Websocket {
        return SafeStreamRecoveryAction::FallbackToHttp;
    }
    SafeStreamRecoveryAction::Fail
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
    tool_search_description: Option<&str>,
    thinking_level: Option<&str>,
    fast_mode: bool,
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
    stream_chat_with_options(
        access_token,
        account_id,
        transport,
        base_url,
        model,
        system_prompt,
        history,
        tools,
        tool_search_description,
        thinking_level,
        debug,
        session_id,
        response_request_metadata,
        turn_state,
        CodexStreamOptions::default().with_fast_mode(fast_mode),
        on_text_delta,
        on_thinking_delta,
        on_tool_call_start,
    )
    .await
}

pub async fn stream_chat_with_options<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    tool_search_description: Option<&str>,
    thinking_level: Option<&str>,
    debug: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    turn_state: &mut TurnState,
    options: CodexStreamOptions,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + Sync + 'static,
    G: Fn(String) + Send + Sync + 'static,
    H: Fn(String, String) + Send + Sync,
{
    let transport_session_id = options
        .use_session_continuation
        .then_some(session_id)
        .flatten();
    let mut active_transport = transport;
    if active_transport == CodexTransportMode::Websocket
        && cached_websocket_http_fallback_enabled(transport_session_id, base_url, account_id).await
    {
        active_transport = CodexTransportMode::Http;
    }
    let mut retries = 0u32;

    loop {
        match stream_chat_once(
            access_token,
            account_id,
            active_transport,
            base_url,
            model,
            system_prompt,
            history,
            tools,
            tool_search_description,
            thinking_level,
            debug,
            session_id,
            response_request_metadata,
            turn_state,
            options.clone(),
            on_text_delta,
            on_thinking_delta,
            on_tool_call_start,
        )
        .await
        {
            Ok(resp) => return Ok(resp),
            Err(err) => {
                if active_transport == CodexTransportMode::Websocket
                    && cached_websocket_http_fallback_enabled(
                        transport_session_id,
                        base_url,
                        account_id,
                    )
                    .await
                {
                    active_transport = CodexTransportMode::Http;
                    retries = 0;
                    continue;
                }

                match safe_stream_recovery_action(active_transport, retries, &err) {
                    SafeStreamRecoveryAction::Retry => {
                        retries += 1;
                        let delay = SAFE_STREAM_RECOVERY_DELAY_MS * retries as u64;
                        eprintln!(
                            "[OpenAI Codex] retrying safe stream interruption ({}/{}, retrying in {}ms): {}",
                            retries,
                            MAX_SAFE_STREAM_RECOVERY_RETRIES,
                            delay,
                            err
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    SafeStreamRecoveryAction::FallbackToHttp => {
                        enable_cached_websocket_http_fallback(
                            transport_session_id,
                            base_url,
                            account_id,
                        )
                        .await;
                        eprintln!(
                            "[OpenAI Codex] websocket recovery exhausted; falling back to HTTPS for this session: {}",
                            err
                        );
                        active_transport = CodexTransportMode::Http;
                        retries = 0;
                    }
                    SafeStreamRecoveryAction::Fail => return Err(err),
                }
            }
        }
    }
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
    tool_search_description: Option<&str>,
    thinking_level: Option<&str>,
    debug: bool,
    session_id: Option<&str>,
    response_request_metadata: Option<&HashMap<String, serde_json::Value>>,
    turn_state: &mut TurnState,
    options: CodexStreamOptions,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + Sync + 'static,
    G: Fn(String) + Send + Sync + 'static,
    H: Fn(String, String) + Send + Sync,
{
    // Compaction items must be replayed in every request shape (including full
    // replay), so the body builder always receives the metadata map even when
    // session continuation is disabled.
    let body = build_request_body_with_tool_search(
        model,
        system_prompt,
        history,
        tools,
        tool_search_description,
        thinking_level,
        session_id,
        response_request_metadata,
        options.clone(),
    );
    let transport_session_id = options
        .use_session_continuation
        .then_some(session_id)
        .flatten();
    let transport_response_request_metadata = if options.use_session_continuation {
        response_request_metadata
    } else {
        None
    };

    match transport {
        CodexTransportMode::Http => {
            stream_chat_http_once(
                access_token,
                account_id,
                base_url,
                model,
                history,
                tools,
                session_id,
                debug,
                body,
                transport_response_request_metadata,
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
                transport_session_id,
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
                        session_id,
                        debug,
                        body,
                        transport_response_request_metadata,
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
    session_id: Option<&str>,
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
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .tcp_keepalive(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(30)),
    )?;

    let continuation_request = request_without_input(&body);
    let request_body = build_history_transport_request(
        &body,
        history,
        response_request_metadata,
        /*include_type_field*/ false,
        /*use_previous_response_id*/ false,
    );
    let raw_request = serde_json::to_string_pretty(&request_body).unwrap_or_default();
    let api_url = codex_responses_endpoint(base_url);
    let fast_mode = request_body
        .get("service_tier")
        .and_then(|value| value.as_str())
        == Some("priority");
    let routing_hint = codex_routing_hint(model, fast_mode);

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
            (
                CODEX_BETA_FEATURES_HEADER,
                REMOTE_COMPACTION_V2_BETA_FEATURE,
            ),
            (X_CODEX_ROUTING_HINT_HEADER, routing_hint.as_str()),
        ];
        if let Some(sid) = session_id {
            headers.push(("x-client-request-id", sid));
            headers.push(("session-id", sid));
            headers.push(("thread-id", sid));
        }
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
        .header(
            CODEX_BETA_FEATURES_HEADER,
            REMOTE_COMPACTION_V2_BETA_FEATURE,
        )
        .header(X_CODEX_ROUTING_HINT_HEADER, &routing_hint)
        .json(&request_body);

    if let Some(sid) = session_id {
        req = req
            .header("x-client-request-id", sid)
            .header("session-id", sid)
            .header("thread-id", sid);
    }
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

    if tool_calls
        .iter()
        .any(|tool_call| !tool_call.is_server_tool())
    {
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
        end_turn: stream_state.end_turn,
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
        response_items: stream_state.items_added,
        response_completed: stream_state.got_completed_event,
    })
}

async fn stream_chat_websocket_once<F, G, H>(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    model: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    request_session_id: Option<&str>,
    cache_session_id: Option<&str>,
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
    let (shared_session, cached_socket, last_response, disable_websockets) = match cache_session_id
    {
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
    let fast_mode = ws_request
        .get("service_tier")
        .and_then(|value| value.as_str())
        == Some("priority");
    let routing_hint = codex_routing_hint(model, fast_mode);

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
            (
                CODEX_BETA_FEATURES_HEADER,
                REMOTE_COMPACTION_V2_BETA_FEATURE,
            ),
            (X_CODEX_ROUTING_HINT_HEADER, routing_hint.as_str()),
        ];
        if cached_turn_state.is_some() {
            headers.push((X_CODEX_TURN_STATE_HEADER, "<sticky>"));
        }
        if let Some(sid) = request_session_id {
            headers.push(("x-client-request-id", sid));
            headers.push(("session-id", sid));
            headers.push(("thread-id", sid));
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
        request_session_id,
        Some(&routing_hint),
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
        let message = match tokio::time::timeout(WEBSOCKET_STREAM_IDLE_TIMEOUT, socket.next()).await
        {
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
            // The dedicated websocket pump consumes Ping/Pong and sends Pong
            // immediately, so response processing only observes data frames.
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
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
            request_signature: websocket_request_signature(&body),
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
        end_turn: stream_state.end_turn,
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
        response_items: stream_state.items_added,
        response_completed: stream_state.got_completed_event,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        build_codex_websocket_handshake_request, build_compact_request_body,
        build_history_transport_request, build_input, build_input_with_metadata,
        build_request_body, build_request_body_with_tool_search, build_websocket_transport_request,
        cached_websocket_http_fallback_enabled, codex_responses_endpoint, codex_routing_hint,
        codex_websocket_url, collect_complete_tool_calls, drain_sse_buffer,
        enable_cached_websocket_http_fallback, extract_compaction_encrypted_content,
        parse_compact_response, process_sse_event_block, request_without_input,
        retained_remote_compaction_v2_window, safe_stream_recovery_action, uri_host_port,
        validate_remote_compaction_v2_output, websocket_config, websocket_event_error_message,
        websocket_proxy_match_uri, websocket_request_signature, BoxedCodexIo, CodexStreamOptions,
        CodexStreamState, CodexWebsocketStream, LastWebsocketResponse, PartialToolCall,
        SafeStreamRecoveryAction, CODEX_BETA_FEATURES_HEADER, CODEX_ORIGINATOR_HEADER_VALUE,
        MAX_SAFE_STREAM_RECOVERY_RETRIES, PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE,
        REMOTE_COMPACTION_V2_BETA_FEATURE, RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE,
        TOOL_SEARCH_HISTORY_TOOL_NAME, X_CODEX_ROUTING_HINT_HEADER, X_CODEX_TURN_STATE_HEADER,
    };
    use crate::commands::{CodexTransportMode, ToolCallOutcome};
    use crate::llm::CODEX_CLIENT_VERSION;
    use crate::session::models::{
        ChatMessage, ImageData, MessageRole, ServerToolKind, ToolCallInfo,
    };
    use futures::{SinkExt, StreamExt};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::proxy::connect_via_proxy;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::proxy::{
        ProxyConfig as TungsteniteProxyConfig, ProxyScheme as TungsteniteProxyScheme,
    };
    use tokio_tungstenite::tungstenite::Message;

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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: Some(images),
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
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
            content_order: None,
            thinking_order: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
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
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    fn tool_message_with_images(
        id: &str,
        tool_call_id: &str,
        content: &str,
        images: Vec<ImageData>,
    ) -> ChatMessage {
        let mut message = tool_message(id, tool_call_id, content);
        message.images = Some(images);
        message
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
            request_signature: websocket_request_signature(body),
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
    fn build_input_keeps_server_tools_out_of_client_function_history() {
        let input = build_input(&[assistant_message_with_tool_calls(
            "assistant-1",
            "查完了",
            Some("resp_prev"),
            vec![ToolCallInfo {
                id: "ws_1".to_string(),
                name: "web_search".to_string(),
                arguments: r#"{"query":"rust async await"}"#.to_string(),
                order: None,
                server_tool: Some(ServerToolKind::WebSearch),
                server_tool_output: Some("Searched: rust async await".to_string()),
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }],
        )]);

        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], serde_json::json!("assistant"));
        assert_eq!(input[0]["content"][0]["text"], serde_json::json!("查完了"));
    }

    #[test]
    fn build_input_closes_dangling_local_function_call() {
        let input = build_input(&[assistant_message_with_tool_calls(
            "assistant-1",
            "",
            Some("resp_prev"),
            vec![ToolCallInfo {
                id: "call-missing".to_string(),
                name: "read".to_string(),
                arguments: r#"{"filePath":"src/main.rs"}"#.to_string(),
                order: None,
                server_tool: None,
                server_tool_output: None,
                outcome: Some(ToolCallOutcome::Done),
                recorded_output: None,
                nested_tool_calls: None,
            }],
        )]);

        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], serde_json::json!("function_call"));
        assert_eq!(input[1]["type"], serde_json::json!("function_call_output"));
        assert_eq!(input[1]["call_id"], serde_json::json!("call-missing"));
        assert_eq!(
            input[1]["output"],
            serde_json::json!(crate::session::history::INTERRUPTED_TOOL_RESULT)
        );
    }

    fn tool_search_call_info(call_id: &str, arguments: &str) -> ToolCallInfo {
        ToolCallInfo {
            id: call_id.to_string(),
            name: TOOL_SEARCH_HISTORY_TOOL_NAME.to_string(),
            arguments: arguments.to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }
    }

    #[test]
    fn build_input_replays_tool_search_round_as_typed_items() {
        let output_json = r#"{"tools":[{"type":"function","name":"pdf_export","description":"Export PDF.","parameters":{"type":"object"},"defer_loading":true},{"type":"function","name":"pdf_preview","description":"Preview PDF.","parameters":{"type":"object"},"defer_loading":true}]}"#;
        let input = build_input(&[
            assistant_message_with_tool_calls(
                "assistant-1",
                "",
                Some("resp_prev"),
                vec![tool_search_call_info(
                    "search-1",
                    r#"{"wire_names":["pdf_export","pdf_preview"]}"#,
                )],
            ),
            tool_message("tool-1", "search-1", output_json),
        ]);

        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], serde_json::json!("tool_search_call"));
        assert_eq!(input[0]["call_id"], serde_json::json!("search-1"));
        assert_eq!(input[0]["execution"], serde_json::json!("client"));
        assert_eq!(
            input[0]["arguments"],
            serde_json::json!({"wire_names": ["pdf_export", "pdf_preview"]})
        );
        assert_eq!(input[1]["type"], serde_json::json!("tool_search_output"));
        assert_eq!(input[1]["call_id"], serde_json::json!("search-1"));
        assert_eq!(input[1]["status"], serde_json::json!("completed"));
        assert_eq!(
            input[1]["tools"][0]["name"],
            serde_json::json!("pdf_export")
        );
        assert_eq!(
            input[1]["tools"][0]["defer_loading"],
            serde_json::json!(true)
        );
        assert_eq!(
            input[1]["tools"][1]["name"],
            serde_json::json!("pdf_preview")
        );
    }

    #[test]
    fn build_input_patches_missing_tool_search_output_with_empty_tools() {
        let input = build_input(&[assistant_message_with_tool_calls(
            "assistant-1",
            "",
            Some("resp_prev"),
            vec![tool_search_call_info(
                "search-1",
                r#"{"wire_names":["pdf_export"]}"#,
            )],
        )]);

        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], serde_json::json!("tool_search_call"));
        assert_eq!(input[1]["type"], serde_json::json!("tool_search_output"));
        assert_eq!(input[1]["tools"], serde_json::json!([]));
    }

    #[test]
    fn build_input_degrades_error_tool_search_output_to_empty_tools() {
        let input = build_input(&[
            assistant_message_with_tool_calls(
                "assistant-1",
                "",
                Some("resp_prev"),
                vec![tool_search_call_info(
                    "search-1",
                    r#"{"wire_names":["pdf_export"]}"#,
                )],
            ),
            tool_message(
                "tool-1",
                "search-1",
                "tool_search requires a `wire_names` array of exact deferred-tool names.",
            ),
        ]);

        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["type"], serde_json::json!("tool_search_output"));
        assert_eq!(input[1]["tools"], serde_json::json!([]));
    }

    #[test]
    fn request_body_appends_tool_search_declaration() {
        let body = build_request_body_with_tool_search(
            "gpt-5.4",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            Some("# Tool discovery\n\nsources:\n- locus_skills"),
            None,
            Some("session-1"),
            None,
            CodexStreamOptions::default(),
        );

        let tools = body["tools"].as_array().expect("tools declared");
        let search = tools
            .iter()
            .find(|tool| tool["type"] == serde_json::json!("tool_search"))
            .expect("tool_search declaration present");
        assert_eq!(search["execution"], serde_json::json!("client"));
        assert_eq!(
            search["parameters"]["required"],
            serde_json::json!(["wire_names"])
        );
        assert!(
            search["parameters"]["properties"]["wire_names"]["description"]
                .as_str()
                .unwrap_or_default()
                .contains("complete deferred-tool wire names")
        );
        assert_eq!(
            search["parameters"]["properties"]["wire_names"]["items"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            search["parameters"]["properties"]["wire_names"]["items"]["minLength"],
            serde_json::json!(1)
        );
        assert_eq!(
            search["parameters"]["properties"]["wire_names"]["minItems"],
            serde_json::json!(1)
        );
        assert_eq!(
            search["parameters"]["properties"]["wire_names"]["maxItems"],
            serde_json::json!(8)
        );
        assert_eq!(
            search["parameters"]["properties"]["wire_names"]["uniqueItems"],
            serde_json::json!(true)
        );
        assert!(search["parameters"]["properties"]
            .get("wire_name")
            .is_none());
        assert!(search["parameters"]["properties"].get("query").is_none());
        assert!(search["parameters"]["properties"].get("limit").is_none());

        // The declaration is excluded from the continuation signature the
        // same way every tool is.
        let signature = request_without_input(&body);
        assert!(signature.get("tools").is_none());
    }

    #[test]
    fn request_body_without_description_matches_legacy_shape() {
        let with = build_request_body_with_tool_search(
            "gpt-5.4",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            None,
            None,
            Some("session-1"),
            None,
            CodexStreamOptions::default(),
        );
        let legacy = build_request_body(
            "gpt-5.4",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            None,
            Some("session-1"),
            None,
            CodexStreamOptions::default(),
        );

        assert_eq!(with, legacy);
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
    fn collects_tool_search_call_from_sse_items() {
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
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"tool_search_call\",\"call_id\":\"search_1\",\"execution\":\"client\",\"arguments\":{}}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("output_item.added should parse");
        process_sse_event_block(
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"item_1\",\"type\":\"tool_search_call\",\"call_id\":\"search_1\",\"status\":\"completed\",\"execution\":\"client\",\"arguments\":{\"wire_names\":[\"pdf_export\",\"pdf_preview\"]}}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &on_tool,
        )
        .expect("output_item.done should parse");

        let (collected, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
        assert_eq!(incomplete, 0);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].tool_call.id, "search_1");
        assert_eq!(collected[0].tool_call.name, TOOL_SEARCH_HISTORY_TOOL_NAME);
        let arguments: serde_json::Value =
            serde_json::from_str(&collected[0].tool_call.arguments).expect("arguments JSON");
        assert_eq!(
            arguments["wire_names"],
            serde_json::json!(["pdf_export", "pdf_preview"])
        );

        let started = started.lock().expect("tool mutex poisoned");
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].0, "search_1");
        assert_eq!(started[0].1, TOOL_SEARCH_HISTORY_TOOL_NAME);
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
        assert!(state.got_completed_event);
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
    fn parses_end_turn_false_from_terminal_event() {
        let mut state = CodexStreamState::new();

        let stopped = process_sse_event_block(
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_continue\",\"end_turn\":false}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("terminal event should parse");

        assert!(stopped);
        assert_eq!(state.end_turn, Some(false));
        assert!(state.got_completed_event);
    }

    #[test]
    fn distinguishes_incomplete_from_completed_terminal_event() {
        let mut state = CodexStreamState::new();

        let stopped = process_sse_event_block(
            "data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp_partial\"}}",
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("incomplete event should parse");

        assert!(stopped);
        assert!(state.got_terminal_event);
        assert!(!state.got_completed_event);
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
    fn builds_function_call_output_with_image_content() {
        let input = build_input(&[tool_message_with_images(
            "tool-1",
            "call_1",
            "Unity screenshot captured.",
            vec![ImageData {
                data: "YWJj".to_string(),
                mime_type: "image/png".to_string(),
            }],
        )]);

        assert_eq!(input[0]["type"], serde_json::json!("function_call_output"));
        let output = input[0]["output"]
            .as_array()
            .expect("tool output should be a content array");
        assert_eq!(output[0]["type"], serde_json::json!("input_text"));
        assert_eq!(output[1]["type"], serde_json::json!("input_image"));
        assert_eq!(
            output[1]["image_url"],
            serde_json::json!("data:image/png;base64,YWJj")
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
            None,
            CodexStreamOptions::default(),
        );

        assert_eq!(body["text"]["verbosity"].as_str(), Some("low"));
        assert!(body.get("service_tier").is_none());
    }

    #[test]
    fn request_body_includes_strict_structured_output_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "maxLength": 36 }
            },
            "required": ["title"],
            "additionalProperties": false
        });
        let body = build_request_body(
            "gpt-5.6-luna",
            "Generate a title",
            &[user_message_with_images("Fix OAuth callback", vec![])],
            &[],
            Some("low"),
            None,
            None,
            CodexStreamOptions::compact().with_output_schema("session_title", schema.clone()),
        );

        assert_eq!(body["text"]["verbosity"].as_str(), Some("low"));
        assert_eq!(body["text"]["format"]["type"].as_str(), Some("json_schema"));
        assert_eq!(
            body["text"]["format"]["name"].as_str(),
            Some("session_title")
        );
        assert_eq!(body["text"]["format"]["strict"].as_bool(), Some(true));
        assert_eq!(body["text"]["format"]["schema"], schema);
        assert!(body.get("tools").is_none());
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn build_request_body_injects_priority_service_tier_for_fast_mode() {
        let body = build_request_body(
            "gpt-5.6-sol",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            Some("low"),
            None,
            None,
            CodexStreamOptions::default().with_fast_mode(true),
        );

        assert_eq!(body["service_tier"].as_str(), Some("priority"));
        assert!(body.get("max_output_tokens").is_none());
    }

    #[test]
    fn build_input_replays_codex_compaction_item_for_handoff_message() {
        let handoff = assistant_message("handoff-1", "## Context Handoff\n\nlocal digest", None);
        let user = user_message_with_images("继续", vec![]);
        let mut metadata = HashMap::new();
        metadata.insert(
            "handoff-1".to_string(),
            serde_json::json!({
                "codex_compaction": { "encrypted_content": "opaque-blob" }
            }),
        );

        let input = build_input_with_metadata(&[handoff.clone(), user], Some(&metadata));

        assert_eq!(input[0]["type"].as_str(), Some("compaction"));
        assert_eq!(input[0]["encrypted_content"].as_str(), Some("opaque-blob"));
        assert!(
            !input
                .iter()
                .any(|item| item["content"][0]["text"].as_str() == Some(handoff.content.as_str())),
            "handoff text must not be sent alongside the compaction item"
        );
        assert_eq!(input[1]["role"].as_str(), Some("user"));

        // Without metadata the handoff is sent as a regular assistant message.
        let plain = build_input(&[handoff]);
        assert_eq!(plain[0]["role"].as_str(), Some("assistant"));
    }

    #[test]
    fn build_input_replays_canonical_compact_window_as_is() {
        let handoff = assistant_message("handoff-1", "## Context Handoff\n\nlocal digest", None);
        let next_user = user_message_with_images("继续", vec![]);
        let canonical_output = serde_json::json!([
            {
                "type": "compaction_summary",
                "encrypted_content": "opaque-blob"
            },
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "retained request" }]
            }
        ]);
        let mut metadata = HashMap::new();
        metadata.insert(
            "handoff-1".to_string(),
            serde_json::json!({
                "codex_compaction": {
                    "output": canonical_output,
                    "encrypted_content": "opaque-blob"
                }
            }),
        );

        let input = build_input_with_metadata(&[handoff, next_user], Some(&metadata));

        assert_eq!(input.len(), 3);
        assert_eq!(input[0], canonical_output[0]);
        assert_eq!(input[1], canonical_output[1]);
        assert_eq!(input[2]["role"].as_str(), Some("user"));
        assert_eq!(input[2]["content"][0]["text"].as_str(), Some("继续"));
    }

    #[test]
    fn remote_compaction_v2_uses_responses_with_terminal_trigger() {
        let body = build_request_body(
            "gpt-5.6-sol",
            "You are Codex",
            &[user_message_with_images("compact this context", vec![])],
            &[],
            Some("high"),
            Some("session-1"),
            None,
            CodexStreamOptions::remote_compaction_v2(),
        );
        let input = body["input"].as_array().expect("responses input array");

        assert_eq!(
            codex_responses_endpoint(None),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            input.last().and_then(|item| item["type"].as_str()),
            Some("compaction_trigger")
        );
        assert_eq!(body["stream"].as_bool(), Some(true));
        assert!(body.get("previous_response_id").is_none());
    }

    #[test]
    fn chatgpt_backend_root_is_normalized_to_codex_responses() {
        assert_eq!(
            codex_responses_endpoint(Some("https://chatgpt.com/backend-api/")),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn remote_compaction_v2_requires_completed_single_compaction_output() {
        let compaction = serde_json::json!({
            "type": "compaction",
            "encrypted_content": "opaque"
        });

        assert_eq!(
            validate_remote_compaction_v2_output(std::slice::from_ref(&compaction), true)
                .expect("valid V2 output"),
            compaction
        );
        assert!(
            validate_remote_compaction_v2_output(std::slice::from_ref(&compaction), false)
                .expect_err("incomplete stream must fail")
                .contains("without response.completed")
        );
        assert!(validate_remote_compaction_v2_output(
            &[
                compaction,
                serde_json::json!({ "type": "message", "role": "assistant", "content": [] })
            ],
            true,
        )
        .expect_err("extra output items must fail")
        .contains("exactly one compaction output item"));
    }

    #[test]
    fn remote_compaction_v2_retains_user_context_before_opaque_output() {
        let history = vec![
            user_message_with_images("latest requirement", vec![]),
            assistant_message("answer-1", "completed answer", None),
        ];
        let compaction = serde_json::json!({
            "type": "compaction",
            "encrypted_content": "opaque"
        });
        let retained = retained_remote_compaction_v2_window(&history, None, compaction.clone());

        assert_eq!(retained.len(), 2);
        assert_eq!(retained[0]["role"].as_str(), Some("user"));
        assert_eq!(retained[1], compaction);
    }

    #[test]
    fn compact_request_body_matches_codex_compaction_input_shape() {
        let body = build_compact_request_body(
            "gpt-5.5",
            "You are Codex",
            &[user_message_with_images("排查阴影闪烁", vec![])],
            &[serde_json::json!({
                "type": "function",
                "function": { "name": "read", "description": "Read file", "parameters": {} }
            })],
            Some("high"),
            false,
            Some("session-1"),
            None,
        );

        assert_eq!(body["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(body["instructions"].as_str(), Some("You are Codex"));
        assert_eq!(body["prompt_cache_key"].as_str(), Some("session-1"));
        assert_eq!(body["parallel_tool_calls"].as_bool(), Some(false));
        assert_eq!(body["tools"][0]["name"].as_str(), Some("read"));
        assert_eq!(body["reasoning"]["effort"].as_str(), Some("high"));
        assert!(body.get("stream").is_none());
        assert!(body.get("store").is_none());
        assert!(body.get("type").is_none());
    }

    #[test]
    fn compact_request_body_reuses_fast_service_tier() {
        let body = build_compact_request_body(
            "gpt-5.6-sol",
            "You are Codex",
            &[user_message_with_images("compact", vec![])],
            &[],
            Some("low"),
            true,
            Some("session-1"),
            None,
        );

        assert_eq!(body["service_tier"].as_str(), Some("priority"));
    }

    #[test]
    fn extract_compaction_encrypted_content_takes_newest_compaction_item() {
        let output = vec![
            serde_json::json!({ "type": "message", "role": "user", "content": [] }),
            serde_json::json!({ "type": "compaction", "encrypted_content": "old" }),
            serde_json::json!({ "type": "compaction", "encrypted_content": "new" }),
        ];
        assert_eq!(
            extract_compaction_encrypted_content(&output).as_deref(),
            Some("new")
        );
        assert_eq!(
            extract_compaction_encrypted_content(&[serde_json::json!({
                "type": "compaction",
                "encrypted_content": ""
            })]),
            None
        );
        assert_eq!(extract_compaction_encrypted_content(&[]), None);
    }

    #[test]
    fn extract_compaction_encrypted_content_accepts_codex_aliases() {
        assert_eq!(
            extract_compaction_encrypted_content(&[serde_json::json!({
                "type": "compaction_summary",
                "encrypted_content": "summary"
            })])
            .as_deref(),
            Some("summary")
        );
        assert_eq!(
            extract_compaction_encrypted_content(&[serde_json::json!({
                "type": "context_compaction",
                "encrypted_content": "context"
            })])
            .as_deref(),
            Some("context")
        );
    }

    #[test]
    fn compact_response_accepts_nonempty_canonical_window_without_exact_compaction_type() {
        let response = serde_json::json!({
            "output": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "retained" }]
            }]
        })
        .to_string();

        let outcome = parse_compact_response("request".to_string(), response.clone())
            .expect("accept canonical output");

        assert_eq!(outcome.output.len(), 1);
        assert_eq!(outcome.encrypted_content, None);
        assert_eq!(outcome.raw_request, "request");
        assert_eq!(outcome.raw_response, response);
    }

    #[test]
    fn fast_compact_routing_hint_matches_codex() {
        assert_eq!(
            codex_routing_hint("gpt-5.6-sol", true),
            "model=gpt-5.6-sol;tier=priority"
        );
        assert_eq!(
            codex_routing_hint("gpt-5.6-sol", false),
            "model=gpt-5.6-sol"
        );
    }

    #[test]
    fn compact_request_body_omits_web_search_and_prompt_cache_key() {
        let options = CodexStreamOptions::compact();
        let body = build_request_body(
            "gpt-5.4",
            "Summarize",
            &[user_message_with_images("hello", vec![])],
            &[],
            None,
            Some("session-1"),
            None,
            options.clone(),
        );

        assert!(!options.include_web_search);
        assert!(!options.use_session_continuation);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("max_output_tokens").is_none());
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
    fn websocket_config_enables_permessage_deflate() {
        let config = websocket_config();
        assert!(config.extensions.permessage_deflate.is_some());
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
            Some("model=gpt-5.4"),
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
                .get(CODEX_BETA_FEATURES_HEADER)
                .expect("Codex beta-features header")
                .to_str()
                .ok(),
            Some(REMOTE_COMPACTION_V2_BETA_FEATURE)
        );
        assert_eq!(
            request
                .headers()
                .get(X_CODEX_ROUTING_HINT_HEADER)
                .expect("routing-hint header")
                .to_str()
                .ok(),
            Some("model=gpt-5.4")
        );
        for header_name in ["x-client-request-id", "session-id", "thread-id"] {
            assert_eq!(
                request
                    .headers()
                    .get(header_name)
                    .expect("session identity header")
                    .to_str()
                    .ok(),
                Some("session-456")
            );
        }
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
    fn websocket_event_error_message_recovers_missing_previous_response() {
        let message = websocket_event_error_message(
            r#"{"type":"error","status":400,"error":{"code":"previous_response_not_found","message":"Previous response with id 'resp_old' not found."}}"#,
        );

        assert_eq!(
            message.as_deref(),
            Some(PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE)
        );
    }

    #[test]
    fn websocket_recovery_falls_back_only_after_safe_retries() {
        let keepalive_timeout = concat!(
            "WebSocket closed by server: keepalive ping timeout. ",
            "OpenAI Codex websocket ended before the response finalized ",
            "(text_len=0, complete_tool_calls=0, incomplete_tool_calls=0)."
        );

        assert_eq!(
            safe_stream_recovery_action(CodexTransportMode::Websocket, 0, keepalive_timeout),
            SafeStreamRecoveryAction::Retry
        );
        assert_eq!(
            safe_stream_recovery_action(
                CodexTransportMode::Websocket,
                MAX_SAFE_STREAM_RECOVERY_RETRIES,
                keepalive_timeout,
            ),
            SafeStreamRecoveryAction::FallbackToHttp
        );
        assert_eq!(
            safe_stream_recovery_action(
                CodexTransportMode::Http,
                MAX_SAFE_STREAM_RECOVERY_RETRIES,
                keepalive_timeout,
            ),
            SafeStreamRecoveryAction::Fail
        );

        let partial_response = concat!(
            "WebSocket closed by server. OpenAI Codex websocket ended before the response ",
            "finalized (text_len=3, complete_tool_calls=0, incomplete_tool_calls=1)."
        );
        assert_eq!(
            safe_stream_recovery_action(
                CodexTransportMode::Websocket,
                MAX_SAFE_STREAM_RECOVERY_RETRIES,
                partial_response,
            ),
            SafeStreamRecoveryAction::Fail
        );
    }

    #[tokio::test]
    async fn websocket_pump_answers_ping_while_connection_is_idle() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind websocket test server");
        let address = listener.local_addr().expect("websocket server address");
        let expected_payload = vec![1, 2, 3, 4];
        let server_payload = expected_payload.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept websocket client");
            let mut socket = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("accept websocket handshake");
            socket
                .send(Message::Ping(server_payload.clone().into()))
                .await
                .expect("send server ping");
            let reply = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
                .await
                .expect("timed out waiting for pong")
                .expect("websocket closed before pong")
                .expect("failed reading pong");
            assert_eq!(reply, Message::Pong(server_payload.into()));
        });

        let tcp = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect websocket test client");
        let request = format!("ws://{address}/responses")
            .into_client_request()
            .expect("build websocket test request");
        let transport: BoxedCodexIo = Box::new(tcp);
        let (socket, _) = tokio_tungstenite::client_async(request, transport)
            .await
            .expect("connect websocket test client");
        let _idle_stream = CodexWebsocketStream::new(socket);

        server.await.expect("websocket test server task");
    }

    #[tokio::test]
    async fn websocket_http_fallback_is_sticky_for_session() {
        let session_id = format!("fallback-test-{}", uuid::Uuid::new_v4());
        enable_cached_websocket_http_fallback(Some(&session_id), None, Some("account-1")).await;

        assert!(
            cached_websocket_http_fallback_enabled(Some(&session_id), None, Some("account-1"))
                .await
        );
        assert!(
            !cached_websocket_http_fallback_enabled(Some(&session_id), None, Some("account-2"))
                .await
        );

        super::invalidate_cached_session(&session_id);
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
            /*use_previous_response_id*/ true,
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
    fn history_transport_request_replays_full_input_when_previous_response_id_disabled() {
        let body = serde_json::json!({
            "model": "gpt-5.4",
            "input": build_input(&[
                assistant_message("assistant-1", "call tools", Some("resp_prev")),
                tool_message("tool-1", "call_1", "done"),
                user_message_with_images("continue", vec![]),
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
                user_message_with_images("continue", vec![]),
            ],
            Some(&response_request_metadata("assistant-1", &body)),
            /*include_type_field*/ false,
            /*use_previous_response_id*/ false,
        );

        assert!(request.get("previous_response_id").is_none());
        assert!(request.get("type").is_none());
        assert_eq!(
            request
                .get("input")
                .and_then(|value| value.as_array())
                .map(|items| items.len()),
            Some(3)
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
                order: None,
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
            /*use_previous_response_id*/ true,
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
            /*use_previous_response_id*/ true,
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
    fn websocket_transport_request_ignores_hosted_state_for_incremental_input() {
        let previous_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello" }]
            }],
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
            "tools": [{
                "type": "function",
                "name": "read",
                "description": "Read a file",
                "parameters": { "type": "object" }
            }],
            "tool_choice": "auto",
        });
        let response_items = serde_json::json!([
            {
                "id": "rs_1",
                "type": "reasoning",
                "content": [],
                "encrypted_content": "encrypted",
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-1" }
            },
            {
                "id": "ws_1",
                "type": "web_search_call",
                "status": "completed",
                "action": { "type": "search", "query": "unity" },
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-1" }
            },
            {
                "id": "msg_1",
                "type": "message",
                "status": "completed",
                "phase": "commentary",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "checking",
                    "annotations": [],
                    "logprobs": []
                }],
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-1" }
            },
            {
                "id": "fc_1",
                "type": "function_call",
                "status": "completed",
                "call_id": "call_1",
                "name": "read",
                "arguments": "{\"path\":\"a.rs\"}",
                "internal_chat_message_metadata_passthrough": { "turn_id": "turn-1" }
            }
        ]);
        let current_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [
                previous_body["input"][0].clone(),
                {
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "checking" }]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "read",
                    "arguments": "{\"path\":\"a.rs\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "file contents"
                }
            ],
            "stream": true,
            "store": false,
            "instructions": "You are Codex",
            "tools": previous_body["tools"].clone(),
            "tool_choice": "auto",
        });

        let request = build_websocket_transport_request(
            &current_body,
            Some(&websocket_last_response(
                &previous_body,
                "resp_prev",
                response_items.as_array().expect("response items"),
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
            request.get("input").and_then(|value| value.as_array()),
            Some(
                serde_json::json!([{
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "file contents"
                }])
                .as_array()
                .expect("incremental input")
            )
        );
    }

    #[test]
    fn websocket_transport_request_replays_full_input_when_tools_change() {
        let previous_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [],
            "stream": true,
            "store": false,
            "tools": [{ "type": "function", "name": "read" }],
            "tool_choice": "auto",
        });
        let current_body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [{ "role": "user", "content": [{ "type": "input_text", "text": "go" }] }],
            "stream": true,
            "store": false,
            "tools": [{ "type": "function", "name": "write" }],
            "tool_choice": "auto",
        });
        let request = build_websocket_transport_request(
            &current_body,
            Some(&websocket_last_response(&previous_body, "resp_prev", &[])),
            /*include_type_field*/ true,
        );

        assert!(request.get("previous_response_id").is_none());
        assert_eq!(request["input"], current_body["input"]);
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
    async fn native_proxy_connector_accepts_success_response() {
        let (client, mut server) = tokio::io::duplex(512);
        let proxy = TungsteniteProxyConfig {
            scheme: TungsteniteProxyScheme::Http,
            host: "127.0.0.1".to_string(),
            port: 7897,
            auth: None,
        };

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

        connect_via_proxy(client, &proxy, "api.openai.com", 443)
            .await
            .expect("native proxy connector should succeed");

        server_task.await.expect("server task");
    }
}
