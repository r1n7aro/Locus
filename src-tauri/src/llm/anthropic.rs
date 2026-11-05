use futures::StreamExt;
use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;

use super::utf8_stream::Utf8StreamDecoder;
use super::web_search::{self, SearchHit, WebSearchTool};
use crate::session::models::{
    ChatMessage, Citation, ImageData, MessageRole, ServerToolKind, ServerToolResultBlock,
    ToolCallInfo,
};

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    /// Source citations attached to non-empty text blocks (UTF-16 offsets).
    pub citations: Vec<Citation>,
    /// Provider-reported request cost (OpenRouter `usage.cost`); 0 when absent.
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallInfo>,
    pub finish_reason: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub raw_request: String,
    pub raw_response: String,
    pub thinking_text: String,
    pub thinking_duration_secs: u32,
    pub thinking_signature: String,
    pub web_search_results: Vec<WebSearchResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchResult {
    pub query: String,
    pub results: Vec<WebSearchHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
}

const BETA_FLAGS: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advanced-tool-use-2025-11-20";

/// Beta flag unlocking `defer_loading`/`tool_reference`. Custom Anthropic
/// endpoints get it exactly when the request carries native lazy features;
/// endpoints that key on the header see it, the rest ignore it.
const NATIVE_LAZY_BETA_FLAG: &str = "advanced-tool-use-2025-11-20";
const API_VERSION: &str = "2023-06-01";
const API_BASE: &str = "https://api.anthropic.com";

// Keep OAuth messages, billing metadata and quota requests on one client version.
pub(super) const CLAUDE_CODE_VERSION: &str = "2.1.302";
const X_APP: &str = "cli";
const X_STAINLESS_LANG: &str = "js";
const X_STAINLESS_PACKAGE_VERSION: &str = "0.80.0";
const X_STAINLESS_RUNTIME: &str = "node";
const X_STAINLESS_RUNTIME_VERSION: &str = "v24.3.0";
const X_STAINLESS_TIMEOUT: &str = "600";
const CACHE_TTL: &str = "1h";
const RAW_CHUNK_DEBUG_PREVIEW_CHARS: usize = 1600;
const WEB_SEARCH_TOOL_RESULT_TYPE: &str = "web_search_tool_result";

const AGENT_SDK_IDENTITY: &str = "You are a Claude agent, built on Anthropic's Claude Agent SDK.";
const OAUTH_COMPAT_FALLBACK_MODEL: &str = "claude-opus-4-1-20250805";

fn next_sse_separator(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|pos| (pos, 2usize));
    let crlf = buffer.find("\r\n\r\n").map(|pos| (pos, 4usize));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn sse_line_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(rest.trim_start())
}

fn summarize_recent_raw_chunk(raw_response: &str, max_chars: usize) -> String {
    if raw_response.is_empty() {
        return "(empty)".to_string();
    }

    let char_count = raw_response.chars().count();
    let mut tail_chars: Vec<char> = raw_response.chars().rev().take(max_chars).collect();
    tail_chars.reverse();
    let tail: String = tail_chars.into_iter().collect();
    let escaped = tail.escape_debug().to_string();

    if char_count > max_chars {
        format!("...{}", escaped)
    } else {
        escaped
    }
}

fn stream_response_header_summary(response: &reqwest::Response) -> String {
    const HEADER_NAMES: &[&str] = &[
        "content-type",
        "content-length",
        "transfer-encoding",
        "connection",
        "date",
        "server",
        "x-request-id",
        "request-id",
        "cf-ray",
        "via",
        "x-cache",
        "eo-log-uuid",
    ];

    let headers = response.headers();
    let summary = HEADER_NAMES
        .iter()
        .filter_map(|name| {
            headers
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .map(|value| format!("{}={}", name, value))
        })
        .collect::<Vec<_>>()
        .join(", ");

    if summary.is_empty() {
        "(none)".to_string()
    } else {
        summary
    }
}

/// Pull the byte offset out of an upstream `... at line 1 column N` JSON parse
/// error. Used to compare where the upstream stopped parsing against the size of
/// the body this client actually sent (#48 truncation diagnostics).
fn extract_json_parse_error_column(error_body: &str) -> Option<usize> {
    let marker = "column ";
    let start = error_body.find(marker)? + marker.len();
    let digits: String = error_body[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Log transport-level facts for a non-success HTTP response: the upstream peer,
/// negotiated protocol, elapsed time, the size of the request body this client
/// sent, and — for body-parse 400s — how far into that body the upstream got
/// before failing. A parse failure far short of the local body size points at
/// truncation in transit rather than a malformed local payload (#48).
#[allow(clippy::too_many_arguments)]
fn log_non_success_diagnostics(
    tag: &str,
    status: reqwest::StatusCode,
    remote_addr: &str,
    http_version: &str,
    elapsed_ms: u128,
    compact_body_len: usize,
    tool_count: usize,
    model: &str,
    response_headers: &str,
    error_body: &str,
) {
    eprintln!(
        "[{}] HTTP {} from {} proto={} elapsed={}ms request_body={} bytes (compact) tools={} model={}",
        tag,
        status.as_u16(),
        remote_addr,
        http_version,
        elapsed_ms,
        compact_body_len,
        tool_count,
        model
    );
    eprintln!("[{}] response headers: {}", tag, response_headers);

    if error_body.contains("Failed to parse the request body as JSON") {
        if let Some(col) = extract_json_parse_error_column(error_body) {
            let pct = if compact_body_len > 0 {
                col.min(compact_body_len) * 100 / compact_body_len
            } else {
                0
            };
            eprintln!(
                "[{}] upstream stopped parsing at column {} (~{}% of the {}-byte request body this client sent). The local body is well-formed JSON, so the upstream most likely received a truncated copy — consistent with truncation in transit rather than a malformed local payload.",
                tag, col, pct, compact_body_len
            );
        }
    }
}

fn parse_anthropic_event<T: DeserializeOwned>(
    tag: &str,
    event_type: &str,
    data: &str,
    debug: bool,
) -> Option<T> {
    match serde_json::from_str::<T>(data) {
        Ok(event) => Some(event),
        Err(error) => {
            if debug {
                eprintln!(
                    "[DEBUG][{}] failed to parse Anthropic stream event: type={} error={} data={}",
                    tag,
                    event_type,
                    error,
                    summarize_recent_raw_chunk(data, RAW_CHUNK_DEBUG_PREVIEW_CHARS)
                );
            }
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
struct OauthToolAliases {
    by_internal: HashMap<String, OauthToolAlias>,
    by_public: HashMap<String, OauthToolAlias>,
}

#[derive(Debug, Clone)]
struct OauthToolAlias {
    internal_name: String,
    public_name: String,
    args: OauthToolArgAliases,
}

#[derive(Debug, Clone, Default)]
struct OauthToolArgAliases {
    internal_to_public: HashMap<String, String>,
    public_to_internal: HashMap<String, String>,
    children: HashMap<String, OauthToolArgAliases>,
    items: Option<Box<OauthToolArgAliases>>,
}

impl OauthToolAliases {
    fn insert(&mut self, alias: OauthToolAlias) {
        self.by_public
            .insert(alias.public_name.clone(), alias.clone());
        self.by_internal.insert(alias.internal_name.clone(), alias);
    }

    fn public_name_for(&self, internal_name: &str) -> Option<&str> {
        self.by_internal
            .get(internal_name)
            .map(|alias| alias.public_name.as_str())
    }

    fn public_input_for(&self, internal_name: &str, input: serde_json::Value) -> serde_json::Value {
        if let Some(alias) = self.by_internal.get(internal_name) {
            alias.args.to_public_value(input)
        } else {
            input
        }
    }

    fn internalize_tool_call(&self, public_name: &str, input_json: &str) -> (String, String) {
        let Some(alias) = self.by_public.get(public_name) else {
            return (public_name.to_string(), input_json.to_string());
        };

        let mapped_json = serde_json::from_str::<serde_json::Value>(input_json)
            .ok()
            .map(|value| alias.args.to_internal_value(value))
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_else(|| input_json.to_string());

        (alias.internal_name.clone(), mapped_json)
    }

    fn internal_name_for(&self, public_name: &str) -> Option<&str> {
        self.by_public
            .get(public_name)
            .map(|alias| alias.internal_name.as_str())
    }
}

impl OauthToolArgAliases {
    fn to_public_value(&self, value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (internal_key, child_value) in map {
                    let public_key = self
                        .internal_to_public
                        .get(&internal_key)
                        .cloned()
                        .unwrap_or_else(|| oauth_public_arg_name(&internal_key));
                    let mapped_value = if let Some(aliases) = self.children.get(&internal_key) {
                        aliases.to_public_value(child_value)
                    } else {
                        child_value
                    };
                    out.insert(public_key, mapped_value);
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                let item_aliases = self.items.as_deref();
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|item| {
                            if let Some(aliases) = item_aliases {
                                aliases.to_public_value(item)
                            } else {
                                item
                            }
                        })
                        .collect(),
                )
            }
            other => other,
        }
    }

    fn to_internal_value(&self, value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (public_key, child_value) in map {
                    let internal_key = self
                        .public_to_internal
                        .get(&public_key)
                        .cloned()
                        .unwrap_or(public_key);
                    let mapped_value = if let Some(aliases) = self.children.get(&internal_key) {
                        aliases.to_internal_value(child_value)
                    } else {
                        child_value
                    };
                    out.insert(internal_key, mapped_value);
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                let item_aliases = self.items.as_deref();
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|item| {
                            if let Some(aliases) = item_aliases {
                                aliases.to_internal_value(item)
                            } else {
                                item
                            }
                        })
                        .collect(),
                )
            }
            other => other,
        }
    }
}

pub async fn stream_chat<F, G, H>(
    access_token: &str,
    model: &str,
    user_metadata: &crate::auth::ClaudeCodeUserMetadata,
    system_parts: &[&str],
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: Option<&str>,
    request_session_id: Option<&str>,
    thinking_level: Option<&str>,
    max_output_tokens: Option<u32>,
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .tcp_keepalive(std::time::Duration::from_secs(15))
            .tcp_nodelay(true)
            .connect_timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(1)
            .http2_adaptive_window(true)
            .http2_keep_alive_interval(std::time::Duration::from_secs(20))
            .http2_keep_alive_timeout(std::time::Duration::from_secs(15)),
    )?;

    let model = model.strip_prefix("anthropic/").unwrap_or(model);
    let mut effective_model = normalize_anthropic_model(model).to_string();

    let tool_reference_names = tool_reference_names_for_request(tools);
    let mut messages = build_anthropic_messages(
        history,
        AnthropicHistoryOptions::standard().with_tool_reference_names(tool_reference_names),
    );
    let (converted_tools, tool_aliases) = convert_tools_to_oauth_sdk_like_anthropic(tools);
    rewrite_oauth_tool_use_blocks(&mut messages, &tool_aliases);
    apply_cache_control(&mut messages, Some(CACHE_TTL));
    let anthropic_tools = converted_tools;
    let oauth_tool_aliases = Some(tool_aliases);

    let system_blocks = build_oauth_system_blocks(system_parts);
    let (thinking_field, output_config, standard_max_tokens) =
        build_thinking_params(&effective_model, thinking_level);
    let max_tokens = u64::from(max_output_tokens.unwrap_or(standard_max_tokens));
    let context_management = Some(serde_json::json!({
        "edits": [{ "type": "clear_thinking_20251015", "keep": "all" }]
    }));

    let mut body = serde_json::json!({
        "model": effective_model,
        "max_tokens": max_tokens,
        "system": system_blocks,
        "messages": messages,
        "stream": true,
    });

    if let Some(thinking) = thinking_field {
        body["thinking"] = serde_json::json!(thinking);
    }
    if let Some(oc) = output_config {
        body["output_config"] = oc;
    }
    if let Some(cm) = context_management {
        body["context_management"] = cm;
    }

    if !anthropic_tools.is_empty() {
        body["tools"] = serde_json::json!(anthropic_tools);
    }

    let effective_base = base_url.unwrap_or(API_BASE);
    let url = format!(
        "{}/v1/messages?beta=true",
        effective_base.trim_end_matches('/')
    );
    let session_id = request_header_session_id(request_session_id);
    let client_request_id = uuid::Uuid::new_v4().to_string();
    body["metadata"] = serde_json::json!({
        "user_id": build_oauth_user_id_metadata(user_metadata, &session_id),
    });

    eprintln!(
        "[Anthropic] POST model={} messages={} tools={}",
        effective_model,
        messages.len(),
        anthropic_tools.len()
    );

    let mut raw_request =
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut last_error = String::new();
    let mut retried_model_fallback = false;
    let mut retried_body_compat = false;
    let mut retried_without_tools = false;
    let mut retried_native_lazy_strip = false;
    let is_first_user_turn = history.len() == 1
        && history[0].role == MessageRole::User
        && history[0].tool_call_id.is_none();

    for attempt in 0..=MAX_RETRIES {
        let headers = build_claude_code_headers(
            access_token,
            None,
            &session_id,
            &client_request_id,
            Some(BETA_FLAGS),
            attempt,
        );
        let mut req = client.post(&url);
        for (key, value) in &headers {
            req = req.header(key.as_str(), value.as_str());
        }

        let resp = match req.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                let mut error_chain = format!("Request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    error_chain.push_str(&format!("\n  caused by: {}", cause));
                    source = std::error::Error::source(cause);
                }

                let is_retryable = e.is_connect() || e.is_timeout();
                if is_retryable && attempt < MAX_RETRIES {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    eprintln!(
                        "[Anthropic] {} (attempt {}/{}, retrying in {}ms...)",
                        error_chain,
                        attempt + 1,
                        MAX_RETRIES + 1,
                        delay
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    continue;
                }
                eprintln!("[Anthropic] {}", error_chain);
                last_error = error_chain;
                continue;
            }
        };

        let status = resp.status();
        if status.is_success() {
            return parse_anthropic_sse(
                "Anthropic",
                &url,
                false,
                resp,
                raw_request,
                oauth_tool_aliases.as_ref(),
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )
            .await;
        }

        let is_retryable_status =
            status.as_u16() == 429 || status.as_u16() == 529 || status.is_server_error();

        let retry_after_secs = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let error_body = resp.text().await.unwrap_or_default();

        // A 400 that names the native lazy-loading surface (deferred tools /
        // tool references) means this endpoint or model does not support it.
        // Stripping is semantics-preserving — every deferred definition
        // becomes an eager one — so the session keeps working, only without
        // the caching win. Loud log so misconfiguration is discoverable.
        if status.as_u16() == 400
            && !retried_native_lazy_strip
            && attempt < MAX_RETRIES
            && body_uses_native_lazy_features(&body)
            && error_mentions_native_lazy_features(&error_body)
        {
            eprintln!(
                "[Anthropic] HTTP 400 rejected native lazy tool loading, retrying with eager tool declarations: {}",
                utf8_prefix_chars(&error_body, 300)
            );
            strip_native_lazy_features(&mut body);
            raw_request =
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
            retried_native_lazy_strip = true;
            continue;
        }

        if is_oauth_generic_bad_request(status, &error_body) && attempt < MAX_RETRIES {
            if !retried_native_lazy_strip && body_uses_native_lazy_features(&body) {
                eprintln!(
                    "[Anthropic] HTTP 400 invalid_request_error, retrying with eager tool declarations"
                );
                strip_native_lazy_features(&mut body);
                raw_request =
                    serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
                retried_native_lazy_strip = true;
                continue;
            }

            if !retried_model_fallback && effective_model != OAUTH_COMPAT_FALLBACK_MODEL {
                eprintln!(
                    "[Anthropic] HTTP 400 invalid_request_error, retrying with fallback model: {} -> {}",
                    effective_model, OAUTH_COMPAT_FALLBACK_MODEL
                );
                effective_model = OAUTH_COMPAT_FALLBACK_MODEL.to_string();
                body["model"] = serde_json::Value::String(effective_model.clone());
                raw_request =
                    serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
                retried_model_fallback = true;
                continue;
            }

            if !retried_body_compat {
                eprintln!(
                    "[Anthropic] HTTP 400 invalid_request_error, retrying with flattened body shape"
                );
                apply_flattened_body_compat(&mut body);
                raw_request =
                    serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
                retried_body_compat = true;
                continue;
            }

            if is_first_user_turn && !retried_without_tools && body.get("tools").is_some() {
                eprintln!(
                    "[Anthropic] HTTP 400 invalid_request_error on first turn, retrying without tools"
                );
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("tools");
                }
                raw_request =
                    serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
                retried_without_tools = true;
                continue;
            }
        }

        if is_retryable_status && attempt < MAX_RETRIES {
            let backoff = BASE_DELAY_MS * 2u64.pow(attempt);
            let delay = retry_after_secs
                .map(|s| s * 1000)
                .unwrap_or(backoff)
                .min(30_000);
            eprintln!(
                "[Anthropic] HTTP {} (attempt {}/{}, retrying in {}ms...): {}",
                status.as_u16(),
                attempt + 1,
                MAX_RETRIES + 1,
                delay,
                utf8_prefix_chars(&error_body, 200)
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            last_error = format!("Anthropic API error ({}): {}", status, error_body);
            continue;
        }

        return Err(format!("Anthropic API error ({}): {}", status, error_body));
    }

    Err(last_error)
}

/// Request body for `stream_chat_native`.
///
/// A struct rather than `serde_json::json!` so the serialized field order is
/// fixed at the declaration site: `serde_json` maps serialize keys
/// alphabetically, which sinks each block's `type` below its multi-KB `text`
/// and buries `system` after `messages`. Some Anthropic-compatible gateways
/// (e.g. DeepSeek) rejected that shape on large requests with
/// `system: missing field type` (issue #91). Keep `type` the first key of
/// every block and `system` ahead of `messages` — the shape official SDKs
/// emit.
#[derive(Debug, serde::Serialize)]
struct NativeChatRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: [NativeSystemBlock<'a>; 1],
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingParam>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, serde::Serialize)]
struct NativeSystemBlock<'a> {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: &'a str,
}

pub async fn stream_chat_native<F, G, H>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: &str,
    thinking_level: Option<&str>,
    replay_thinking_blocks: bool,
    web_search: Option<&WebSearchTool>,
    request_session_id: Option<&str>,
    tag: &str,
    debug: bool,
    max_output_tokens: Option<u32>,
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .tcp_keepalive(std::time::Duration::from_secs(15))
            .tcp_nodelay(true)
            .connect_timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(1)
            .http2_adaptive_window(true)
            .http2_keep_alive_interval(std::time::Duration::from_secs(20))
            .http2_keep_alive_timeout(std::time::Duration::from_secs(15)),
    )?;

    let tool_reference_names = tool_reference_names_for_request(tools);
    let uses_native_lazy_tools = tool_reference_names.is_some();
    // Recorded search result blocks go back natively only to the endpoint
    // that produced them, and only while its Anthropic search tool is on.
    let server_tool_replay_origin = matches!(web_search, Some(WebSearchTool::AnthropicNative))
        .then(|| endpoint_host(base_url))
        .flatten();
    let mut messages = build_anthropic_messages(
        history,
        AnthropicHistoryOptions::custom_endpoint(replay_thinking_blocks)
            .with_tool_reference_names(tool_reference_names)
            .with_server_tool_replay_origin(server_tool_replay_origin),
    );
    // No ttl on custom endpoints: the 1h value needs the official
    // `extended-cache-ttl-2025-04-11` beta header, which this path does not
    // send, and third-party gateways expect the plain ephemeral form.
    apply_cache_control(&mut messages, None);

    let anthropic_tools = build_native_anthropic_tools(tools, web_search);
    let tool_count = anthropic_tools.len();

    let (thinking_field, output_config, standard_max_tokens) =
        build_thinking_params(model, thinking_level);
    let body = NativeChatRequest {
        model,
        max_tokens: max_output_tokens.unwrap_or(standard_max_tokens),
        system: [NativeSystemBlock {
            block_type: "text",
            text: system_prompt,
        }],
        messages,
        stream: true,
        thinking: thinking_field,
        output_config,
        tools: (!anthropic_tools.is_empty()).then_some(anthropic_tools),
    };
    // `serde_json::Value` from here on so the strip-and-retry below can
    // degrade the body in place, mirroring the official-endpoint path.
    let mut body =
        serde_json::to_value(&body).map_err(|e| format!("Failed to serialize request: {}", e))?;

    let mut raw_request =
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));
    // Compact body matches the bytes reqwest actually sends (`.json()` serializes
    // compact). Its length shares the same offset scale as the `line 1 column N`
    // position an upstream JSON parser reports, so it is the number to compare
    // against when diagnosing mid-transit truncation (#48).
    let mut compact_body = serde_json::to_string(&body).unwrap_or_default();
    let mut compact_body_len = compact_body.len();

    eprintln!(
        "[{}] POST model={} messages={} tools={} body_bytes={}",
        tag,
        model,
        history.len(),
        tool_count,
        compact_body_len
    );

    let api_url = format!("{}/messages", base_url.trim_end_matches('/'));
    let beta_flags = uses_native_lazy_tools.then_some(NATIVE_LAZY_BETA_FLAG);
    let session_id = request_header_session_id(request_session_id);
    let client_request_id = uuid::Uuid::new_v4().to_string();

    if debug {
        let mut debug_headers: Vec<(&str, &str)> = vec![
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer <token>"),
            ("x-api-key", "<token>"),
            ("anthropic-version", API_VERSION),
        ];
        if let Some(flags) = beta_flags {
            debug_headers.push(("anthropic-beta", flags));
        }
        super::debug::save_request(
            "custom_anthropic_messages",
            &api_url,
            &debug_headers,
            &raw_request,
        );
    }

    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut last_error = String::new();
    let mut retried_native_lazy_strip = false;

    for attempt in 0..=MAX_RETRIES {
        let headers = build_claude_code_headers(
            api_key,
            Some(api_key),
            &session_id,
            &client_request_id,
            beta_flags,
            attempt,
        );
        let mut req = client.post(&api_url);
        for (key, value) in &headers {
            req = req.header(key.as_str(), value.as_str());
        }
        let started = std::time::Instant::now();
        match req.json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return parse_anthropic_sse(
                        tag,
                        &api_url,
                        debug,
                        resp,
                        raw_request,
                        None,
                        on_text_delta,
                        on_thinking_delta,
                        on_tool_call_start,
                    )
                    .await;
                }

                // Capture transport-level facts before `text()` consumes `resp`.
                let http_version = format!("{:?}", resp.version());
                let remote_addr = resp
                    .remote_addr()
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let response_headers = stream_response_header_summary(&resp);

                let is_retryable =
                    status.as_u16() == 429 || status.as_u16() == 529 || status.is_server_error();
                let error_body = resp.text().await.unwrap_or_default();
                let elapsed_ms = started.elapsed().as_millis();

                log_non_success_diagnostics(
                    tag,
                    status,
                    &remote_addr,
                    &http_version,
                    elapsed_ms,
                    compact_body_len,
                    tool_count,
                    model,
                    &response_headers,
                    &error_body,
                );

                // When the failure looks like the gateway-truncation class (#48),
                // drop a copy-pasteable curl plus the exact request bytes so the
                // user (or the endpoint operator) can reproduce it directly.
                if status.as_u16() == 400
                    && error_body.contains("Failed to parse the request body as JSON")
                {
                    let note = format!(
                        "HTTP 400 from {} proto={}; upstream JSON parse error{}; local request body = {} bytes (compact). The .body.json file holds the exact bytes this client sent.",
                        remote_addr,
                        http_version,
                        extract_json_parse_error_column(&error_body)
                            .map(|col| format!(" at column {}", col))
                            .unwrap_or_default(),
                        compact_body_len
                    );
                    if let Some(path) =
                        super::debug::save_repro_curl(tag, &api_url, &headers, &compact_body, &note)
                    {
                        eprintln!(
                            "[{}] saved reproduction (curl + request body) to: {}",
                            tag,
                            path.display()
                        );
                    }
                }

                // A 400 naming the native lazy surface means this endpoint
                // does not support `defer_loading`/`tool_reference` after
                // all. Stripping is semantics-preserving — every deferred
                // definition becomes an eager one — so the session keeps
                // working; loud log so the misconfigured toggle is found.
                if status.as_u16() == 400
                    && !retried_native_lazy_strip
                    && attempt < MAX_RETRIES
                    && body_uses_native_lazy_features(&body)
                    && error_mentions_native_lazy_features(&error_body)
                {
                    eprintln!(
                        "[{}] HTTP 400 rejected native lazy tool loading, retrying with eager tool declarations: {}",
                        tag,
                        utf8_prefix_chars(&error_body, 300)
                    );
                    strip_native_lazy_features(&mut body);
                    raw_request = serde_json::to_string_pretty(&body)
                        .unwrap_or_else(|_| format!("{:?}", body));
                    compact_body = serde_json::to_string(&body).unwrap_or_default();
                    compact_body_len = compact_body.len();
                    retried_native_lazy_strip = true;
                    continue;
                }

                if is_retryable && attempt < MAX_RETRIES {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    eprintln!(
                        "[{}] HTTP {} (attempt {}/{}, retrying in {}ms...)",
                        tag,
                        status.as_u16(),
                        attempt + 1,
                        MAX_RETRIES + 1,
                        delay
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    last_error = format!("{} API error ({}): {}", tag, status, error_body);
                    continue;
                }

                return Err(format!("{} API error ({}): {}", tag, status, error_body));
            }
            Err(e) => {
                let mut error_chain = format!("Request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    error_chain.push_str(&format!("\n  caused by: {}", cause));
                    source = std::error::Error::source(cause);
                }

                let is_retryable = e.is_connect() || e.is_timeout();
                if is_retryable && attempt < MAX_RETRIES {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    eprintln!(
                        "[{}] {} (attempt {}/{}, retrying in {}ms...)",
                        tag,
                        error_chain,
                        attempt + 1,
                        MAX_RETRIES + 1,
                        delay
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                } else {
                    eprintln!("[{}] {}", tag, error_chain);
                    last_error = error_chain;
                }
            }
        }
    }

    Err(last_error)
}

async fn parse_anthropic_sse<F, G, H>(
    tag: &str,
    api_url: &str,
    debug: bool,
    response: reqwest::Response,
    raw_request: String,
    oauth_tool_aliases: Option<&OauthToolAliases>,
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let status = response.status();
    let response_headers = stream_response_header_summary(&response);
    if debug {
        eprintln!(
            "[DEBUG][{}] stream response accepted: status={} url={} headers={}",
            tag, status, api_url, response_headers
        );
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_text = String::new();
    let mut full_thinking = String::new();
    let mut thinking_signature = String::new();
    let mut raw_response = String::new();
    let mut utf8_decoder = Utf8StreamDecoder::default();
    let mut tool_calls: Vec<PartialToolCall> = Vec::new();
    let mut _current_block_index: Option<usize> = None;
    let mut stop_reason = String::from("end_turn");
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut cache_read_tokens: u32 = 0;
    let mut cache_write_tokens: u32 = 0;
    let mut thinking_start: Option<std::time::Instant> = None;
    let mut thinking_duration_secs: u32 = 0;
    let mut web_search_results: Vec<WebSearchResult> = Vec::new();
    let mut message_id = String::new();
    let mut full_text_utf16: u32 = 0;
    // UTF-16 span of each text block inside `full_text`, keyed by block index.
    let mut text_spans: HashMap<usize, (u32, u32)> = HashMap::new();
    // Every search-result citation in arrival order, with its text block index.
    let mut cited_hits: Vec<(usize, SearchHit)> = Vec::new();
    // Citations for searches that never surfaced a tool block (OpenRouter
    // Google-native grounding only reports them as citations plus usage).
    let mut unattached_hits: Vec<SearchHit> = Vec::new();
    let mut provider_search_requests: u32 = 0;
    let mut cost_usd: f64 = 0.0;

    const CHUNK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
    let mut got_message_stop = false;

    loop {
        let chunk_result = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;

        let chunk = match chunk_result {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                let mut error_chain = format!("Stream read error: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    error_chain.push_str(&format!("\n  caused by: {}", cause));
                    source = std::error::Error::source(cause);
                }
                if raw_response.is_empty() {
                    eprintln!(
                        "[{}] {} (no response data received, status={}, url={}, headers={})",
                        tag, error_chain, status, api_url, response_headers
                    );
                } else {
                    eprintln!(
                        "[{}] {}\n  status={} url={} headers={}\n  raw_response_len={} recent_raw_chunk={}",
                        tag,
                        error_chain,
                        status,
                        api_url,
                        response_headers,
                        raw_response.len(),
                        summarize_recent_raw_chunk(&raw_response, RAW_CHUNK_DEBUG_PREVIEW_CHARS)
                    );
                }

                return Err(format!(
                    "{}\n  partial_data: text_len={}, tools={}",
                    error_chain,
                    full_text.len(),
                    tool_calls.len()
                ));
            }
            Ok(None) => {
                break;
            }
            Err(_) => {
                eprintln!(
                    "[{}] Stream chunk read timed out after {}s (status={}, url={}, headers={}, raw_response_len={}, pending_buffer_len={})",
                    tag,
                    CHUNK_TIMEOUT.as_secs(),
                    status,
                    api_url,
                    response_headers,
                    raw_response.len(),
                    buffer.len()
                );
                return Err(format!(
                    "Stream read timed out after {}s, partial_data: text_len={}, tools={}",
                    CHUNK_TIMEOUT.as_secs(),
                    full_text.len(),
                    tool_calls.len()
                ));
            }
        };

        let chunk_text = utf8_decoder.push(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);

        while let Some((pos, sep_len)) = next_sse_separator(&buffer) {
            let event_text = buffer[..pos].to_string();
            buffer = buffer[pos + sep_len..].to_string();

            let mut event_type = String::new();
            let mut data_str = String::new();

            for line in event_text.lines() {
                let line = line.trim();
                if let Some(et) = sse_line_value(line, "event") {
                    event_type = et.trim().to_string();
                } else if let Some(d) = sse_line_value(line, "data") {
                    if !data_str.is_empty() {
                        data_str.push('\n');
                    }
                    data_str.push_str(d.trim());
                }
            }

            if data_str.is_empty() {
                continue;
            }

            match event_type.as_str() {
                "content_block_start" => {
                    if let Some(ev) = parse_anthropic_event::<ContentBlockStartEvent>(
                        tag,
                        event_type.as_str(),
                        &data_str,
                        debug,
                    ) {
                        _current_block_index = Some(ev.index);
                        match ev.content_block.block_type.as_str() {
                            "tool_use" => {
                                let public_name = ev.content_block.name.unwrap_or_default();
                                let name = oauth_tool_aliases
                                    .and_then(|aliases| aliases.internal_name_for(&public_name))
                                    .unwrap_or(public_name.as_str())
                                    .to_string();
                                let id = ev.content_block.id.unwrap_or_default();
                                on_tool_call_start(id.clone(), name.clone());
                                tool_calls.push(PartialToolCall {
                                    index: ev.index,
                                    id,
                                    name,
                                    input_json: String::new(),
                                    server_tool_output: None,
                                    server_tool: false,
                                    search_hits: Vec::new(),
                                    server_tool_result: None,
                                });
                            }
                            "server_tool_use" => {
                                // Provider-executed: never dispatched locally,
                                // whether or not a result block follows
                                // (OpenRouter sends results only as citations).
                                let raw_name = ev.content_block.name.unwrap_or_default();
                                let name = if web_search::is_web_search_tool_name(&raw_name) {
                                    web_search::WEB_SEARCH_TOOL_NAME.to_string()
                                } else {
                                    raw_name
                                };
                                let id = ev.content_block.id.unwrap_or_default();
                                on_tool_call_start(id.clone(), name.clone());
                                tool_calls.push(PartialToolCall {
                                    index: ev.index,
                                    id,
                                    name,
                                    input_json: String::new(),
                                    server_tool_output: None,
                                    server_tool: true,
                                    search_hits: Vec::new(),
                                    server_tool_result: None,
                                });
                            }
                            "web_search_tool_result" => {
                                // Attach results to the call they name, else to
                                // the most recent search call.
                                let result_for = ev.content_block.tool_use_id.as_deref();
                                let is_search = |tc: &PartialToolCall| {
                                    tc.server_tool && tc.name == web_search::WEB_SEARCH_TOOL_NAME
                                };
                                let target = result_for
                                    .and_then(|id| {
                                        tool_calls
                                            .iter()
                                            .rposition(|tc| is_search(tc) && tc.id == id)
                                    })
                                    .or_else(|| tool_calls.iter().rposition(is_search));
                                let mut server_tc = target.map(|index| &mut tool_calls[index]);
                                if let (Some(tc), Some(content)) =
                                    (server_tc.as_deref_mut(), &ev.content_block.content)
                                {
                                    tc.server_tool_result = Some(content.clone());
                                }
                                if let Some(content) = &ev.content_block.content {
                                    if let Some(arr) = content.as_array() {
                                        let query = server_tc
                                            .as_ref()
                                            .and_then(|tc| search_query_from_input(&tc.input_json))
                                            .unwrap_or_default();
                                        let hits: Vec<WebSearchHit> = arr
                                            .iter()
                                            .filter_map(|item| {
                                                Some(WebSearchHit {
                                                    title: item.get("title")?.as_str()?.to_string(),
                                                    url: item.get("url")?.as_str()?.to_string(),
                                                })
                                            })
                                            .collect();
                                        let summary = if hits.is_empty() {
                                            format!("{}\nNo results.", query).trim().to_string()
                                        } else {
                                            let search_hits: Vec<SearchHit> = hits
                                                .iter()
                                                .map(|hit| SearchHit {
                                                    url: hit.url.clone(),
                                                    title: Some(hit.title.clone()),
                                                    excerpt: None,
                                                })
                                                .collect();
                                            web_search::format_search_output(
                                                Some(&query),
                                                &search_hits,
                                            )
                                        };
                                        if let Some(tc) = server_tc {
                                            if !query.is_empty() {
                                                tc.input_json =
                                                    serde_json::json!({ "query": &query })
                                                        .to_string();
                                            }
                                            tc.server_tool_output = Some(summary);
                                        }
                                        if !hits.is_empty() {
                                            web_search_results.push(WebSearchResult {
                                                query,
                                                results: hits,
                                            });
                                        }
                                    } else if let Some(obj) = content.as_object() {
                                        if let Some(err) =
                                            obj.get("error_code").and_then(|e| e.as_str())
                                        {
                                            let msg = format!("Web search error: {}", err);
                                            if let Some(tc) = server_tc {
                                                tc.server_tool_output = Some(msg);
                                            }
                                        }
                                    }
                                }
                            }
                            "text" => {
                                text_spans.insert(ev.index, (full_text_utf16, full_text_utf16));
                            }
                            _ => {} // thinking, redacted_thinking, etc.
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(ev) = parse_anthropic_event::<ContentBlockDeltaEvent>(
                        tag,
                        event_type.as_str(),
                        &data_str,
                        debug,
                    ) {
                        match ev.delta.delta_type.as_str() {
                            "text_delta" => {
                                if let Some(ref text) = ev.delta.text {
                                    if thinking_start.is_some() && thinking_duration_secs == 0 {
                                        thinking_duration_secs = if let Some(start) = thinking_start
                                        {
                                            start.elapsed().as_secs() as u32
                                        } else {
                                            0
                                        };
                                    }
                                    let span_start = full_text_utf16;
                                    full_text.push_str(text);
                                    full_text_utf16 =
                                        full_text_utf16.saturating_add(web_search::utf16_len(text));
                                    text_spans
                                        .entry(ev.index)
                                        .or_insert((span_start, span_start))
                                        .1 = full_text_utf16;
                                    on_text_delta(text.clone());
                                }
                            }
                            "citations_delta" => {
                                if let Some(hit) = ev
                                    .delta
                                    .citation
                                    .as_ref()
                                    .and_then(SearchHit::from_citation)
                                {
                                    // Results belong to the latest search that has
                                    // no `web_search_tool_result` of its own.
                                    match tool_calls.iter_mut().rev().find(|tc| {
                                        tc.server_tool
                                            && tc.name == web_search::WEB_SEARCH_TOOL_NAME
                                    }) {
                                        Some(tc) if tc.server_tool_output.is_none() => {
                                            web_search::push_hit(&mut tc.search_hits, hit.clone());
                                        }
                                        Some(_) => {}
                                        None => {
                                            web_search::push_hit(&mut unattached_hits, hit.clone());
                                        }
                                    }
                                    cited_hits.push((ev.index, hit));
                                }
                            }
                            "input_json_delta" => {
                                if let Some(ref json) = ev.delta.partial_json {
                                    if let Some(tc) =
                                        tool_calls.iter_mut().rev().find(|tc| tc.index == ev.index)
                                    {
                                        tc.input_json.push_str(json);
                                    }
                                }
                            }
                            "thinking_delta" => {
                                if let Some(ref thinking) = ev.delta.thinking {
                                    if thinking_start.is_none() {
                                        thinking_start = Some(std::time::Instant::now());
                                    }
                                    full_thinking.push_str(thinking);
                                    on_thinking_delta(thinking.clone());
                                }
                            }
                            "signature_delta" => {
                                if let Some(ref sig) = ev.delta.signature {
                                    thinking_signature.push_str(sig);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_stop" => {
                    _current_block_index = None;
                }
                "message_delta" => {
                    if let Some(ev) = parse_anthropic_event::<MessageDeltaEvent>(
                        tag,
                        event_type.as_str(),
                        &data_str,
                        debug,
                    ) {
                        if let Some(ref reason) = ev.delta.stop_reason {
                            stop_reason = reason.clone();
                        }
                        if let Some(ref usage) = ev.usage {
                            output_tokens = usage.output_tokens;
                            // message_delta usage is cumulative. OpenRouter reports
                            // input/cache totals only here (message_start says 0),
                            // and server-tool loops grow them after message_start.
                            if usage.input_tokens > 0 {
                                input_tokens = usage.input_tokens;
                            }
                            if usage.cache_read_input_tokens > 0 {
                                cache_read_tokens = usage.cache_read_input_tokens;
                            }
                            if usage.cache_creation_input_tokens > 0 {
                                cache_write_tokens = usage.cache_creation_input_tokens;
                            }
                            if let Some(ref server_tool_use) = usage.server_tool_use {
                                provider_search_requests = provider_search_requests
                                    .max(server_tool_use.web_search_requests);
                            }
                            if let Some(cost) = usage.cost.filter(|cost| cost.is_finite()) {
                                cost_usd = cost;
                            }
                        }
                    }
                }
                "message_stop" => {
                    got_message_stop = true;
                }
                "message_start" => {
                    if let Some(ev) = parse_anthropic_event::<MessageStartEvent>(
                        tag,
                        event_type.as_str(),
                        &data_str,
                        debug,
                    ) {
                        if let Some(id) = ev.message.id.filter(|id| !id.is_empty()) {
                            message_id = id;
                        }
                        if let Some(ref usage) = ev.message.usage {
                            input_tokens = usage.input_tokens;
                            output_tokens = usage.output_tokens;
                            cache_read_tokens = usage.cache_read_input_tokens;
                            cache_write_tokens = usage.cache_creation_input_tokens;
                        }
                    }
                }
                "ping" => {}
                "error" => {
                    return Err(format!("Anthropic stream error: {}", data_str));
                }
                _ => {
                    if debug {
                        eprintln!(
                            "[DEBUG][{}] ignored Anthropic stream event: type={} data_len={}",
                            tag,
                            event_type,
                            data_str.len()
                        );
                    }
                }
            }
        }
    }

    if !got_message_stop {
        eprintln!(
            "[{}] stream ended before message_stop: status={} url={} headers={} parsed_text_len={} parsed_thinking_len={} parsed_tool_calls={} raw_response_len={} pending_buffer_len={} recent_raw_chunk={} pending_buffer={}",
            tag,
            status,
            api_url,
            response_headers,
            full_text.len(),
            full_thinking.len(),
            tool_calls.len(),
            raw_response.len(),
            buffer.len(),
            summarize_recent_raw_chunk(&raw_response, RAW_CHUNK_DEBUG_PREVIEW_CHARS),
            summarize_recent_raw_chunk(&buffer, RAW_CHUNK_DEBUG_PREVIEW_CHARS)
        );
        if !full_text.is_empty() || !tool_calls.is_empty() {
            return Err(format!(
                "Stream ended without message_stop (incomplete response, text_len={}, tools={})",
                full_text.len(),
                tool_calls.len()
            ));
        }
        if !raw_response.is_empty() {
            return Err(format!(
                "Stream ended with no parsed Anthropic events and no message_stop (raw_response_len={}, pending_buffer_len={})",
                raw_response.len(),
                buffer.len()
            ));
        }
        return Err(format!(
            "Stream ended with no data and no message_stop (status={}, headers={})",
            status, response_headers
        ));
    }

    let surfaced_searches = tool_calls
        .iter()
        .filter(|tc| tc.server_tool && tc.name == web_search::WEB_SEARCH_TOOL_NAME)
        .count() as u32;
    let origin = endpoint_host(api_url);
    let mut final_tool_calls: Vec<ToolCallInfo> = tool_calls
        .into_iter()
        .map(|tc| {
            if tc.server_tool {
                return finish_server_tool_call(tc, origin.as_deref());
            }
            let (name, arguments) = if let Some(aliases) = oauth_tool_aliases {
                aliases.internalize_tool_call(&tc.name, &tc.input_json)
            } else {
                (tc.name, tc.input_json)
            };

            ToolCallInfo {
                id: tc.id,
                name,
                arguments,
                order: None,
                server_tool: None,
                server_tool_output: None,
                server_tool_result: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }
        })
        .collect();

    // Searches the provider ran without a `server_tool_use` block (OpenRouter
    // with Google-native grounding) still get a record, so the UI shows them
    // and their sources are replayed to later rounds.
    if provider_search_requests > surfaced_searches || !unattached_hits.is_empty() {
        let id = if message_id.is_empty() {
            format!("srvtoolu_{}", uuid::Uuid::new_v4().simple())
        } else {
            format!("{}_web_search", message_id)
        };
        final_tool_calls.push(web_search::provider_search_record(
            id,
            provider_search_requests.saturating_sub(surfaced_searches),
            &unattached_hits,
        ));
    }

    // Only citations on text the user sees become inline references; ones on
    // empty carrier blocks (OpenRouter Exa results) live in the search record.
    let citations = web_search::finalize_citations(
        cited_hits
            .iter()
            .filter_map(|(index, hit)| {
                let (start, end) = *text_spans.get(index)?;
                (end > start).then(|| web_search::url_citation(hit, start, end))
            })
            .collect(),
    );

    let finish_reason = if final_tool_calls
        .iter()
        .any(|tool_call| !tool_call.is_server_tool())
    {
        "tool_calls".to_string()
    } else {
        stop_reason
    };

    if thinking_start.is_some() && thinking_duration_secs == 0 && !full_thinking.is_empty() {
        thinking_duration_secs = if let Some(start) = thinking_start {
            start.elapsed().as_secs() as u32
        } else {
            0
        };
    }

    Ok(LlmResponse {
        text: full_text,
        citations,
        cost_usd,
        tool_calls: final_tool_calls,
        finish_reason,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        raw_request,
        raw_response,
        thinking_text: full_thinking,
        thinking_duration_secs,
        thinking_signature,
        web_search_results,
    })
}

fn search_query_from_input(input_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(input_json)
        .ok()?
        .get("query")?
        .as_str()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_string)
}

/// Lower-cased host of an endpoint URL: the scope within which a provider's
/// opaque server-tool content (encrypted search excerpts) can be replayed.
fn endpoint_host(url: &str) -> Option<String> {
    url::Url::parse(url.trim())
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
        .filter(|host| !host.is_empty())
}

/// Provider-executed tool call: always carries `server_tool_output`, so it is
/// never dispatched locally and never forces a follow-up round. `origin` is the
/// endpoint host, recorded with the raw result block for native replay.
fn finish_server_tool_call(tc: PartialToolCall, origin: Option<&str>) -> ToolCallInfo {
    let is_search = tc.name == web_search::WEB_SEARCH_TOOL_NAME;
    let server_tool_result = match (origin, tc.server_tool_result) {
        (Some(origin), Some(content)) if is_search => Some(ServerToolResultBlock {
            origin: origin.to_string(),
            block_type: WEB_SEARCH_TOOL_RESULT_TYPE.to_string(),
            content,
        }),
        _ => None,
    };
    let output = match tc.server_tool_output {
        Some(output) => output,
        None if is_search => web_search::format_search_output(
            search_query_from_input(&tc.input_json).as_deref(),
            &tc.search_hits,
        ),
        None => format!("Provider-side tool `{}` ran on the server.", tc.name),
    };
    let arguments = if tc.input_json.trim().is_empty() {
        "{}".to_string()
    } else {
        tc.input_json
    };
    ToolCallInfo {
        id: tc.id,
        name: tc.name,
        arguments,
        order: None,
        server_tool: is_search.then_some(ServerToolKind::WebSearch),
        server_tool_output: Some(output),
        server_tool_result,
        outcome: None,
        recorded_output: None,
        nested_tool_calls: None,
    }
}

///
///
///
/// Internally tagged so serde always writes `type` as the first key — the
/// same gateway-compat constraint as `NativeChatRequest` (issue #91).
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ThinkingParam {
    Adaptive,
    Disabled,
    Enabled { budget_tokens: u32 },
}

fn build_thinking_params(
    model: &str,
    thinking_level: Option<&str>,
) -> (Option<ThinkingParam>, Option<serde_json::Value>, u32) {
    let normalized_model = model.to_ascii_lowercase().replace('.', "-");
    let is_fable = normalized_model.contains("claude-fable-5");
    let is_sonnet_5 = normalized_model.contains("claude-sonnet-5");
    let is_always_adaptive = is_fable
        || is_sonnet_5
        || normalized_model.contains("claude-mythos-5")
        || normalized_model.contains("claude-mythos-preview");
    let is_adaptive = is_always_adaptive
        || normalized_model.contains("opus-5-5")
        || normalized_model.contains("sonnet-4-6")
        || normalized_model.contains("opus-4-8")
        || normalized_model.contains("opus-4-7")
        || normalized_model.contains("opus-4-6");

    let level = match thinking_level {
        Some(l) if !l.is_empty() => l,
        _ if is_adaptive => {
            return (Some(ThinkingParam::Adaptive), None, 32000);
        }
        _ => return (None, None, 16384),
    };

    if level == "none" && is_always_adaptive {
        return (Some(ThinkingParam::Adaptive), None, 32000);
    }

    if level == "none" {
        return (Some(ThinkingParam::Disabled), None, 8192);
    }

    if is_adaptive {
        // https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking
        let output_config = match level {
            "low" | "medium" | "high" | "xhigh" | "max" => {
                Some(serde_json::json!({ "effort": level }))
            }
            _ => None,
        };
        // xhigh/max run long on Opus 4.7/4.8 — a 32k output cap truncates
        // mid-task, so give those tiers the headroom the docs call for (these
        // models support up to 128k output; the OAuth path streams, so the
        // larger cap carries no HTTP-timeout risk).
        let max_tokens = if matches!(level, "xhigh" | "max") {
            64000
        } else {
            32000
        };
        (Some(ThinkingParam::Adaptive), output_config, max_tokens)
    } else {
        let (budget_tokens, max_tokens) = match level {
            "low" => (2048, 8192),
            "medium" => (5000, 12000),
            "high" => (16000, 32000),
            "max" => (24000, 32000),
            _ => return (None, None, 16384),
        };
        (
            Some(ThinkingParam::Enabled { budget_tokens }),
            None,
            max_tokens,
        )
    }
}

#[derive(Debug, Clone)]
struct AnthropicHistoryOptions {
    replay_thinking_blocks: bool,
    require_thinking_signature: bool,
    /// Internal names of the tools declared in this request. When set,
    /// tool-reference marker lines in tool results are stripped and re-emitted
    /// as `tool_reference` content blocks — filtered against this set so a
    /// reference to an undeclared tool (deleted skill, renamed tool) degrades
    /// to plain text instead of a `Tool reference not found` 400.
    tool_reference_names: Option<std::collections::HashSet<String>>,
    /// Endpoint host whose recorded server-tool result blocks may be replayed
    /// natively. Set only when this request declares that endpoint's own
    /// Anthropic search tool; other results fall back to user-side text.
    server_tool_replay_origin: Option<String>,
}

impl AnthropicHistoryOptions {
    fn standard() -> Self {
        Self {
            replay_thinking_blocks: true,
            require_thinking_signature: false,
            tool_reference_names: None,
            server_tool_replay_origin: None,
        }
    }

    fn custom_endpoint(replay_thinking_blocks: bool) -> Self {
        Self {
            replay_thinking_blocks,
            require_thinking_signature: true,
            tool_reference_names: None,
            server_tool_replay_origin: None,
        }
    }

    fn with_tool_reference_names(
        mut self,
        names: Option<std::collections::HashSet<String>>,
    ) -> Self {
        self.tool_reference_names = names;
        self
    }

    fn with_server_tool_replay_origin(mut self, origin: Option<String>) -> Self {
        self.server_tool_replay_origin = origin;
        self
    }
}

/// Strongly typed message history for the Anthropic Messages API. Structs and
/// internally tagged enums serialize in declaration order with `type` leading
/// every block — the same gateway-compat constraint as `NativeChatRequest`
/// (issue #91); `serde_json::Value` would re-sort keys alphabetically and sink
/// the tag below multi-KB payloads.
#[derive(Debug, serde::Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: Vec<HistoryContentBlock>,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HistoryContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Image {
        source: ImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: ToolResultContent,
    },
    /// Provider-executed call, replayed together with its result block.
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// The provider's own search result, `encrypted_content` included.
    WebSearchToolResult {
        tool_use_id: String,
        content: serde_json::Value,
    },
    /// Reference to a deferred tool (`defer_loading: true`); the API expands
    /// the full definition in place. Only emitted when the native lazy
    /// renderer is active and the referenced tool is declared in the request.
    ToolReference { tool_name: String },
}

impl HistoryContentBlock {
    fn text(text: impl Into<String>) -> Self {
        HistoryContentBlock::Text {
            text: text.into(),
            cache_control: None,
        }
    }

    fn image(img: &ImageData) -> Self {
        HistoryContentBlock::Image {
            source: ImageSource {
                source_type: "base64",
                media_type: img.mime_type.clone(),
                data: img.data.clone(),
            },
            cache_control: None,
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
enum ToolResultContent {
    Text(String),
    Blocks(Vec<HistoryContentBlock>),
}

#[derive(Debug, serde::Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: &'static str,
    media_type: String,
    data: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    control_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<&'static str>,
}

fn build_anthropic_messages(
    history: &[ChatMessage],
    options: AnthropicHistoryOptions,
) -> Vec<AnthropicMessage> {
    let history = crate::session::history::normalize_tool_round_history_for_request(history);
    let mut messages: Vec<AnthropicMessage> = Vec::new();
    // Server-tool output that cannot be replayed natively rides in the next
    // user-role message. Replayed inside the assistant turn it read as text
    // the model had written itself, and DeepSeek started writing fabricated
    // `[Web Search Result]` lists instead of calling the tool.
    let mut pending_server_notes: Vec<HistoryContentBlock> = Vec::new();

    let mut i = 0;
    while i < history.len() {
        let msg = &history[i];
        match msg.role {
            MessageRole::User => {
                let mut blocks = std::mem::take(&mut pending_server_notes);
                for img in msg.images.iter().flatten() {
                    blocks.push(HistoryContentBlock::image(img));
                }
                blocks.extend(build_text_blocks(&msg.content));
                if !blocks.is_empty() {
                    messages.push(AnthropicMessage {
                        role: "user",
                        content: blocks,
                    });
                }
                i += 1;
            }
            MessageRole::Assistant => {
                if !pending_server_notes.is_empty() {
                    messages.push(AnthropicMessage {
                        role: "user",
                        content: std::mem::take(&mut pending_server_notes),
                    });
                }
                let has_text = !msg.content.is_empty();
                let thinking_text = msg
                    .thinking_content
                    .as_deref()
                    .filter(|value| !value.is_empty());
                let thinking_signature = msg
                    .thinking_signature
                    .as_deref()
                    .filter(|value| !value.is_empty());
                let has_thinking = options.replay_thinking_blocks
                    && thinking_text.is_some()
                    && (!options.require_thinking_signature || thinking_signature.is_some());

                let mut server_blocks: Vec<HistoryContentBlock> = Vec::new();
                let mut tool_use_blocks: Vec<HistoryContentBlock> = Vec::new();
                for tc in msg.tool_calls.iter().flatten() {
                    // Server tools are never sent back as client tool_use blocks.
                    if tc.is_server_tool() {
                        if let Some(native) = native_server_tool_blocks(
                            tc,
                            options.server_tool_replay_origin.as_deref(),
                        ) {
                            server_blocks.extend(native);
                        } else if let Some(note) = server_tool_note(tc) {
                            pending_server_notes.push(note);
                        }
                        continue;
                    }
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));
                    tool_use_blocks.push(HistoryContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input,
                    });
                }

                let mut content_blocks: Vec<HistoryContentBlock> = Vec::new();
                if has_thinking {
                    content_blocks.push(HistoryContentBlock::Thinking {
                        thinking: thinking_text.unwrap_or_default().to_string(),
                        signature: thinking_signature.map(str::to_string),
                    });
                }
                // Searches run before the text that discusses their results.
                content_blocks.extend(server_blocks);
                if has_text {
                    content_blocks.extend(build_text_blocks(&msg.content));
                }
                // Client tool_use blocks close the message: their tool_result
                // blocks open the next user message, and compatible gateways
                // (DeepSeek) reject any block that follows a pending tool_use.
                content_blocks.extend(tool_use_blocks);

                if content_blocks.is_empty() {
                    i += 1;
                    continue;
                }

                messages.push(AnthropicMessage {
                    role: "assistant",
                    content: content_blocks,
                });
                i += 1;
            }
            MessageRole::Tool => {
                let mut tool_result_blocks: Vec<HistoryContentBlock> = Vec::new();

                while i < history.len() && history[i].role == MessageRole::Tool {
                    let tool_msg = &history[i];
                    if let Some(tool_use_id) = tool_msg.tool_call_id.as_deref() {
                        if !tool_use_id.is_empty() {
                            tool_result_blocks.push(HistoryContentBlock::ToolResult {
                                tool_use_id: tool_use_id.to_string(),
                                content: build_tool_result_content(
                                    &tool_msg.content,
                                    tool_msg.images.as_deref(),
                                    options.tool_reference_names.as_ref(),
                                ),
                            });
                        } else {
                            eprintln!("[Anthropic] skipped tool_result with empty tool_use_id");
                        }
                    } else {
                        eprintln!("[Anthropic] skipped tool_result without tool_use_id");
                    }
                    i += 1;
                }
                // tool_result blocks must lead the user message; notes follow.
                tool_result_blocks.extend(std::mem::take(&mut pending_server_notes));

                if !tool_result_blocks.is_empty() {
                    messages.push(AnthropicMessage {
                        role: "user",
                        content: tool_result_blocks,
                    });
                }
            }
        }
    }

    if !pending_server_notes.is_empty() {
        messages.push(AnthropicMessage {
            role: "user",
            content: pending_server_notes,
        });
    }

    messages
}

/// The provider's own `server_tool_use` + result pair. Only replayed toward
/// the endpoint that produced it (its encrypted excerpts decode nowhere else)
/// and only while that endpoint's search tool is declared in the request.
fn native_server_tool_blocks(
    tc: &ToolCallInfo,
    replay_origin: Option<&str>,
) -> Option<[HistoryContentBlock; 2]> {
    let result = tc.server_tool_result.as_ref()?;
    if tc.id.is_empty()
        || result.block_type != WEB_SEARCH_TOOL_RESULT_TYPE
        || replay_origin != Some(result.origin.as_str())
    {
        return None;
    }
    let input = serde_json::from_str::<serde_json::Value>(&tc.arguments)
        .ok()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    Some([
        HistoryContentBlock::ServerToolUse {
            id: tc.id.clone(),
            name: tc.name.clone(),
            input,
        },
        HistoryContentBlock::WebSearchToolResult {
            tool_use_id: tc.id.clone(),
            content: result.content.clone(),
        },
    ])
}

/// User-side text replay of a server search that cannot be replayed natively
/// (another endpoint, search tool off, or no recorded result block). Tool
/// search output is provider control-plane state and is not replayed at all.
fn server_tool_note(tc: &ToolCallInfo) -> Option<HistoryContentBlock> {
    if tc.server_tool.as_ref() != Some(&ServerToolKind::WebSearch) {
        return None;
    }
    let output = tc.server_tool_output.as_deref().map(str::trim)?;
    if output.is_empty() {
        return None;
    }
    Some(HistoryContentBlock::text(format!(
        "<system-reminder>\nThe provider ran web_search during your previous response. \
         Its output follows; this is tool output, not text you wrote. Call web_search \
         again for new results.\n\n{}\n</system-reminder>",
        output
    )))
}

fn build_oauth_system_blocks(system_parts: &[&str]) -> serde_json::Value {
    let mut blocks: Vec<serde_json::Value> = Vec::new();

    blocks.push(serde_json::json!({
        "type": "text",
        "text": format!(
            "x-anthropic-billing-header: cc_version={CLAUDE_CODE_VERSION}; cc_entrypoint=sdk-rs; cch=00000;"
        ),
    }));

    blocks.push(serde_json::json!({
        "type": "text",
        "text": AGENT_SDK_IDENTITY,
        "cache_control": { "type": "ephemeral", "ttl": CACHE_TTL },
    }));

    let cleaned = system_parts
        .iter()
        .map(|part| sanitize_system_text(part))
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if !cleaned.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": cleaned,
            "cache_control": { "type": "ephemeral", "ttl": CACHE_TTL },
        }));
    }

    serde_json::Value::Array(blocks)
}

fn normalize_anthropic_model(model: &str) -> &str {
    match model {
        "claude-opus-4.8" => "claude-opus-4-8",
        "claude-opus-4.7" => "claude-opus-4-7",
        "claude-sonnet-4.6" => "claude-sonnet-4-6",
        "claude-opus-4.6" => "claude-opus-4-6",
        "claude-haiku-4.5" => "claude-haiku-4-5",
        other => other,
    }
}

fn is_oauth_generic_bad_request(status: reqwest::StatusCode, error_body: &str) -> bool {
    if status.as_u16() != 400 {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(error_body) else {
        return false;
    };
    let err = v.get("error");
    let err_type = err
        .and_then(|e| e.get("type"))
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    let msg = err
        .and_then(|e| e.get("message"))
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    err_type == "invalid_request_error" && msg.eq_ignore_ascii_case("Error")
}

fn utf8_prefix_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn apply_flattened_body_compat(body: &mut serde_json::Value) {
    if let Some(system) = body.get_mut("system") {
        if let Some(arr) = system.as_array() {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|block| {
                    let is_text = block.get("type").and_then(|t| t.as_str()) == Some("text");
                    if is_text {
                        block
                            .get("text")
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect();
            if !texts.is_empty() {
                *system = serde_json::Value::String(texts.join("\n\n"));
            }
        }
    }

    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for msg in messages.iter_mut() {
            let Some(content) = msg.get_mut("content") else {
                continue;
            };
            let Some(arr) = content.as_array() else {
                continue;
            };
            let text_parts: Option<Vec<&str>> = arr
                .iter()
                .map(|block| {
                    block
                        .get("type")
                        .and_then(|t| t.as_str())
                        .filter(|t| *t == "text")
                        .and_then(|_| block.get("text"))
                        .and_then(|t| t.as_str())
                })
                .collect();
            if let Some(parts) = text_parts {
                *content = serde_json::Value::String(parts.concat());
            }
        }
    }
}

/// Whether the serialized request body carries native lazy-loading features:
/// `defer_loading` on any tool or a `tool_reference` block in any tool result.
fn body_uses_native_lazy_features(body: &serde_json::Value) -> bool {
    let has_deferred_tool = body
        .get("tools")
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("defer_loading").and_then(|v| v.as_bool()) == Some(true))
        })
        .unwrap_or(false);
    if has_deferred_tool {
        return true;
    }

    body.get("messages")
        .and_then(|messages| messages.as_array())
        .map(|messages| messages.iter().any(message_has_tool_reference_block))
        .unwrap_or(false)
}

fn message_has_tool_reference_block(message: &serde_json::Value) -> bool {
    let Some(blocks) = message.get("content").and_then(|c| c.as_array()) else {
        return false;
    };
    blocks.iter().any(|block| {
        if block.get("type").and_then(|t| t.as_str()) == Some("tool_reference") {
            return true;
        }
        block
            .get("content")
            .and_then(|c| c.as_array())
            .map(|nested| {
                nested
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_reference"))
            })
            .unwrap_or(false)
    })
}

fn error_mentions_native_lazy_features(error_body: &str) -> bool {
    let lowered = error_body.to_ascii_lowercase();
    lowered.contains("defer_loading")
        || lowered.contains("tool_reference")
        || lowered.contains("tool reference")
}

/// Degrades a native-lazy request to eager declarations in place: removes
/// every `defer_loading` flag and drops `tool_reference` blocks from tool
/// results. Definitions stay declared, so history references losing their
/// blocks cannot dangle.
fn strip_native_lazy_features(body: &mut serde_json::Value) {
    if let Some(tools) = body.get_mut("tools").and_then(|tools| tools.as_array_mut()) {
        for tool in tools {
            if let Some(map) = tool.as_object_mut() {
                map.remove("defer_loading");
            }
        }
    }

    let Some(messages) = body
        .get_mut("messages")
        .and_then(|messages| messages.as_array_mut())
    else {
        return;
    };
    for message in messages {
        let Some(blocks) = message.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for block in blocks.iter_mut() {
            if let Some(nested) = block.get_mut("content").and_then(|c| c.as_array_mut()) {
                nested.retain(|b| b.get("type").and_then(|t| t.as_str()) != Some("tool_reference"));
                if nested.is_empty() {
                    // An empty content array is invalid; keep a placeholder
                    // text block so the tool_result stays well-formed.
                    nested.push(serde_json::json!({ "type": "text", "text": "(empty)" }));
                }
            }
        }
        blocks.retain(|b| b.get("type").and_then(|t| t.as_str()) != Some("tool_reference"));
    }
}

fn apply_cache_control(messages: &mut [AnthropicMessage], ttl: Option<&'static str>) {
    let len = messages.len();
    if len == 0 {
        return;
    }

    let marker = CacheControl {
        control_type: "ephemeral",
        ttl,
    };

    let start = if len >= 2 { len - 2 } else { 0 };
    for msg in &mut messages[start..] {
        if let Some(slot) = msg.content.iter_mut().rev().find_map(|block| match block {
            HistoryContentBlock::Text { cache_control, .. }
            | HistoryContentBlock::Image { cache_control, .. } => Some(cache_control),
            _ => None,
        }) {
            *slot = Some(marker.clone());
        }
    }
}

/// Whether an OpenAI-format tool entry carries the native lazy-loading marker
/// set by the request assembly (`defer_loading: true` on the wrapper).
fn is_deferred_api_tool(tool: &serde_json::Value) -> bool {
    tool.get("defer_loading").and_then(|v| v.as_bool()) == Some(true)
}

/// Internal names of the function tools in an OpenAI-format tool list.
fn api_tool_names(openai_tools: &[serde_json::Value]) -> std::collections::HashSet<String> {
    openai_tools
        .iter()
        .filter_map(|tool| tool.get("function")?.get("name")?.as_str())
        .map(str::to_string)
        .collect()
}

/// Tool-reference declared set for a request: only populated when at least one
/// tool is deferred (native lazy renderer active); `None` keeps tool results
/// byte-verbatim on every other path.
fn tool_reference_names_for_request(
    openai_tools: &[serde_json::Value],
) -> Option<std::collections::HashSet<String>> {
    openai_tools
        .iter()
        .any(is_deferred_api_tool)
        .then(|| api_tool_names(openai_tools))
}

/// OpenAI: { type: "function", function: { name, description, parameters } }
/// Anthropic: { name, description, input_schema }
fn convert_tools_to_anthropic(openai_tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let tools: Vec<serde_json::Value> = openai_tools
        .iter()
        .filter_map(|tool| {
            let deferred = is_deferred_api_tool(tool);
            let func = tool.get("function")?;
            let name = func.get("name")?.as_str()?;
            let description = func.get("description")?.as_str().unwrap_or("");
            let parameters = func
                .get("parameters")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            let mut converted = serde_json::json!({
                "name": name,
                "description": description,
                "input_schema": parameters,
            });
            if deferred {
                converted["defer_loading"] = serde_json::json!(true);
            }
            Some(converted)
        })
        .collect();

    tools
}

fn build_native_anthropic_tools(
    openai_tools: &[serde_json::Value],
    web_search: Option<&WebSearchTool>,
) -> Vec<serde_json::Value> {
    // Deferred tools sort after every eager tool and must never carry
    // `cache_control` (the API rejects the combination with a 400); the cache
    // breakpoint goes on the last eager tool so appending deferred
    // declarations mid-session cannot move it.
    let (mut tools, deferred): (Vec<_>, Vec<_>) = convert_tools_to_anthropic(openai_tools)
        .into_iter()
        .partition(|tool| !is_deferred_api_tool(tool));

    // Server-side search tool. Some custom-compatible endpoints reject this
    // type; OpenRouter's own tool is not documented to take `cache_control`,
    // so it goes after the breakpoint.
    let trailing_search = match web_search {
        Some(tool) if tool.accepts_cache_control() => {
            tools.push(tool.tool_definition());
            None
        }
        Some(tool) => Some(tool.tool_definition()),
        None => None,
    };

    if let Some(last_tool) = tools.last_mut() {
        last_tool["cache_control"] = serde_json::json!({ "type": "ephemeral", "ttl": CACHE_TTL });
    }

    tools.extend(trailing_search);
    tools.extend(deferred);
    tools
}

fn convert_tools_to_oauth_sdk_like_anthropic(
    openai_tools: &[serde_json::Value],
) -> (Vec<serde_json::Value>, OauthToolAliases) {
    let mut tools: Vec<serde_json::Value> = Vec::new();
    let mut aliases = OauthToolAliases::default();

    for tool in openai_tools {
        let deferred = is_deferred_api_tool(tool);
        let Some(function) = tool.get("function") else {
            continue;
        };
        let Some(internal_name) = function.get("name").and_then(|v| v.as_str()) else {
            continue;
        };

        let public_name = oauth_public_tool_name(internal_name);
        let description = function
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let parameters = function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let (public_input_schema, arg_aliases) = convert_oauth_input_schema(&parameters);

        let mut converted = serde_json::json!({
            "name": public_name,
            "description": description,
            "input_schema": public_input_schema,
        });
        if deferred {
            converted["defer_loading"] = serde_json::json!(true);
        }
        tools.push(converted);

        aliases.insert(OauthToolAlias {
            internal_name: internal_name.to_string(),
            public_name,
            args: arg_aliases,
        });
    }

    (tools, aliases)
}

fn convert_oauth_input_schema(
    schema: &serde_json::Value,
) -> (serde_json::Value, OauthToolArgAliases) {
    let Some(obj) = schema.as_object() else {
        return (schema.clone(), OauthToolArgAliases::default());
    };

    let mut aliases = OauthToolArgAliases::default();
    let mut converted = serde_json::Map::new();

    if let Some(properties) = obj.get("properties").and_then(|v| v.as_object()) {
        let mut public_properties = serde_json::Map::new();
        for (internal_name, property_schema) in properties {
            let public_name = oauth_public_arg_name(internal_name);
            let (public_schema, child_aliases) = convert_oauth_input_schema(property_schema);
            aliases
                .internal_to_public
                .insert(internal_name.clone(), public_name.clone());
            aliases
                .public_to_internal
                .insert(public_name.clone(), internal_name.clone());
            aliases
                .children
                .insert(internal_name.clone(), child_aliases);
            public_properties.insert(public_name, public_schema);
        }
        converted.insert(
            "properties".to_string(),
            serde_json::Value::Object(public_properties),
        );
    }

    if let Some(items) = obj.get("items") {
        let (public_items, item_aliases) = convert_oauth_input_schema(items);
        aliases.items = Some(Box::new(item_aliases));
        converted.insert("items".to_string(), public_items);
    }

    for (key, value) in obj {
        if key == "properties" || key == "items" || key == "required" {
            continue;
        }
        converted.insert(key.clone(), value.clone());
    }

    if let Some(required) = obj.get("required").and_then(|v| v.as_array()) {
        let mapped_required = required
            .iter()
            .map(|item| {
                item.as_str()
                    .map(|name| {
                        serde_json::Value::String(
                            aliases
                                .internal_to_public
                                .get(name)
                                .cloned()
                                .unwrap_or_else(|| oauth_public_arg_name(name)),
                        )
                    })
                    .unwrap_or_else(|| item.clone())
            })
            .collect();
        converted.insert(
            "required".to_string(),
            serde_json::Value::Array(mapped_required),
        );
    }

    (serde_json::Value::Object(converted), aliases)
}

fn rewrite_oauth_tool_use_blocks(messages: &mut [AnthropicMessage], aliases: &OauthToolAliases) {
    for message in messages {
        for block in &mut message.content {
            match block {
                HistoryContentBlock::ToolUse { name, input, .. } => {
                    let Some(public_name) = aliases.public_name_for(name) else {
                        continue;
                    };

                    let internal_name = std::mem::replace(name, public_name.to_string());
                    let current = std::mem::take(input);
                    *input = aliases.public_input_for(&internal_name, current);
                }
                // References are recorded under internal names; the request
                // declares tools under their public aliases, so rewrite here
                // as well or the API cannot resolve them.
                HistoryContentBlock::ToolResult {
                    content: ToolResultContent::Blocks(blocks),
                    ..
                } => {
                    for nested in blocks {
                        let HistoryContentBlock::ToolReference { tool_name } = nested else {
                            continue;
                        };
                        if let Some(public_name) = aliases.public_name_for(tool_name) {
                            *tool_name = public_name.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn oauth_public_tool_name(internal_name: &str) -> String {
    match internal_name {
        "ask_user_question" => "AskUserQuestion".to_string(),
        "config_query" => "ConfigQuery".to_string(),
        "config_set" => "ConfigSet".to_string(),
        "knowledge_list" => "KnowledgeList".to_string(),
        "knowledge_query" => "KnowledgeQuery".to_string(),
        "knowledge_read" => "KnowledgeRead".to_string(),
        "knowledge_create" => "KnowledgeCreate".to_string(),
        "knowledge_delete" => "KnowledgeDelete".to_string(),
        "knowledge_move" => "KnowledgeMove".to_string(),
        "knowledge_edit" => "KnowledgeEdit".to_string(),
        "todowrite" => "TodoWrite".to_string(),
        "unity_asset_search" => "UnityAssetSearch".to_string(),
        "unity_capture_viewport" => "UnityCaptureViewport".to_string(),
        "unity_execute" => "UnityExecute".to_string(),
        "unity_set_play_mode" => "UnitySetPlayMode".to_string(),
        "unity_run_states" => "UnityRunStates".to_string(),
        "unity_recompile" => "UnityRecompile".to_string(),
        "unity_ref_search" => "UnityRefSearch".to_string(),
        "unity_yaml_search" => "UnityYamlSearch".to_string(),
        "unity_yaml_read" => "UnityYamlRead".to_string(),
        "web_fetch" => "WebFetch".to_string(),
        other => oauth_pascal_case(other),
    }
}

fn oauth_public_arg_name(internal_name: &str) -> String {
    let mut out = String::new();
    let mut prev_was_separator = false;

    for (idx, ch) in internal_name.chars().enumerate() {
        match ch {
            '_' | '-' | ' ' => {
                if !out.is_empty() && !prev_was_separator {
                    out.push('_');
                }
                prev_was_separator = true;
            }
            c if c.is_ascii_uppercase() => {
                if idx > 0 && !prev_was_separator && !out.ends_with('_') {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
                prev_was_separator = false;
            }
            c => {
                out.push(c);
                prev_was_separator = false;
            }
        }
    }

    out
}

fn oauth_pascal_case(value: &str) -> String {
    oauth_public_arg_name(value)
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.push(first.to_ascii_uppercase());
                    out.extend(chars);
                    out
                }
                None => String::new(),
            }
        })
        .collect()
}

fn sanitize_system_text(text: &str) -> String {
    let text = text.replace("OpenCode", "Claude Code");
    let re_oc = Regex::new(r"(?i)opencode").unwrap();
    re_oc.replace_all(&text, "Claude").to_string()
}

fn build_text_blocks(text: &str) -> Vec<HistoryContentBlock> {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";

    let mut blocks: Vec<HistoryContentBlock> = Vec::new();
    let mut remaining = text;

    while let Some(start_idx) = remaining.find(START) {
        push_text_block(&mut blocks, &remaining[..start_idx]);

        let Some(end_rel) = remaining[start_idx..].find(END) else {
            push_text_block(&mut blocks, &remaining[start_idx..]);
            remaining = "";
            break;
        };

        let mut block_end = start_idx + end_rel + END.len();
        while let Some(ch) = remaining[block_end..].chars().next() {
            if ch == '\n' || ch == '\r' {
                block_end += ch.len_utf8();
            } else {
                break;
            }
        }

        push_text_block(&mut blocks, &remaining[start_idx..block_end]);
        remaining = &remaining[block_end..];
    }

    push_text_block(&mut blocks, remaining);
    blocks
}

fn build_tool_result_content(
    text: &str,
    images: Option<&[ImageData]>,
    tool_reference_names: Option<&std::collections::HashSet<String>>,
) -> ToolResultContent {
    // Native lazy renderer: marker lines are protocol, not content — strip
    // them and re-emit references for tools declared in this request.
    let (text, reference_names) = match tool_reference_names {
        Some(declared) => {
            let (clean, names) = crate::llm::tool_references::split_tool_reference_marker(text);
            let names: Vec<String> = names
                .into_iter()
                .filter(|name| declared.contains(name))
                .collect();
            (clean, names)
        }
        None => (text.to_string(), Vec::new()),
    };

    let images = images.filter(|images| !images.is_empty());
    if images.is_none() && reference_names.is_empty() {
        return ToolResultContent::Text(text);
    }

    let mut blocks = build_text_blocks(&text);
    for img in images.into_iter().flatten() {
        blocks.push(HistoryContentBlock::image(img));
    }
    for tool_name in reference_names {
        blocks.push(HistoryContentBlock::ToolReference { tool_name });
    }
    if blocks.is_empty() {
        return ToolResultContent::Text(text);
    }
    ToolResultContent::Blocks(blocks)
}

fn push_text_block(blocks: &mut Vec<HistoryContentBlock>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    blocks.push(HistoryContentBlock::text(text));
}

fn request_header_session_id(request_session_id: Option<&str>) -> String {
    let trimmed = request_session_id.unwrap_or_default().trim();
    if trimmed.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_oauth_user_id_metadata(
    user_metadata: &crate::auth::ClaudeCodeUserMetadata,
    session_id: &str,
) -> String {
    serde_json::json!({
        "device_id": user_metadata.device_id.clone(),
        "account_uuid": user_metadata.account_uuid.clone(),
        "session_id": session_id,
    })
    .to_string()
}

fn build_claude_code_headers(
    bearer_token: &str,
    x_api_key: Option<&str>,
    session_id: &str,
    client_request_id: &str,
    beta_flags: Option<&str>,
    retry_count: u32,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".to_string(), "application/json".to_string()),
        (
            "Authorization".to_string(),
            format!("Bearer {}", bearer_token),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "anthropic-dangerous-direct-browser-access".to_string(),
            "true".to_string(),
        ),
        ("anthropic-version".to_string(), API_VERSION.to_string()),
        (
            "User-Agent".to_string(),
            format!("claude-cli/{CLAUDE_CODE_VERSION} (external, sdk-cli)"),
        ),
        ("x-app".to_string(), X_APP.to_string()),
        (
            "x-claude-code-session-id".to_string(),
            session_id.to_string(),
        ),
        (
            "x-client-request-id".to_string(),
            client_request_id.to_string(),
        ),
        (
            "x-stainless-arch".to_string(),
            stainless_arch_header_value(),
        ),
        ("x-stainless-lang".to_string(), X_STAINLESS_LANG.to_string()),
        ("x-stainless-os".to_string(), stainless_os_header_value()),
        (
            "x-stainless-package-version".to_string(),
            X_STAINLESS_PACKAGE_VERSION.to_string(),
        ),
        (
            "x-stainless-retry-count".to_string(),
            retry_count.to_string(),
        ),
        (
            "x-stainless-runtime".to_string(),
            X_STAINLESS_RUNTIME.to_string(),
        ),
        (
            "x-stainless-runtime-version".to_string(),
            X_STAINLESS_RUNTIME_VERSION.to_string(),
        ),
        (
            "x-stainless-timeout".to_string(),
            X_STAINLESS_TIMEOUT.to_string(),
        ),
    ];

    if let Some(beta_flags) = beta_flags.filter(|value| !value.trim().is_empty()) {
        headers.push(("anthropic-beta".to_string(), beta_flags.to_string()));
    }

    if let Some(api_key) = x_api_key {
        headers.push(("x-api-key".to_string(), api_key.to_string()));
    }

    headers
}

fn stainless_arch_header_value() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x64".to_string(),
        "aarch64" => "arm64".to_string(),
        other => other.to_string(),
    }
}

fn stainless_os_header_value() -> String {
    match std::env::consts::OS {
        "windows" => "Windows".to_string(),
        "macos" => "Mac OS X".to_string(),
        "linux" => "Linux".to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug)]
struct PartialToolCall {
    index: usize,
    id: String,
    name: String,
    input_json: String,
    /// Pre-computed output for server tools (e.g. web_search).
    server_tool_output: Option<String>,
    /// Came from a `server_tool_use` block (provider-executed).
    server_tool: bool,
    /// Results delivered as citations instead of a `web_search_tool_result`.
    search_hits: Vec<SearchHit>,
    /// Raw `content` of this call's `web_search_tool_result`, kept for
    /// native replay to the same endpoint.
    server_tool_result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ContentBlockStartEvent {
    index: usize,
    content_block: ContentBlock,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    id: Option<String>,
    name: Option<String>,
    /// Server tool result blocks name the `server_tool_use` they answer.
    tool_use_id: Option<String>,
    content: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ContentBlockDeltaEvent {
    index: usize,
    delta: DeltaContent,
}

#[derive(Debug, Deserialize)]
struct DeltaContent {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
    partial_json: Option<String>,
    thinking: Option<String>,
    signature: Option<String>,
    /// `citations_delta` payload (e.g. `web_search_result_location`).
    citation: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct MessageStartEvent {
    message: MessageStartMessage,
}

#[derive(Debug, Deserialize)]
struct MessageStartMessage {
    #[serde(default)]
    id: Option<String>,
    usage: Option<UsageInfo>,
}

/// Gateways (OpenRouter) send explicit `null` token counts; plain
/// `#[serde(default)]` only covers missing keys, so a `null` would drop the
/// whole event (stop reason and usage included).
#[derive(Debug, Deserialize)]
struct UsageInfo {
    #[serde(default, deserialize_with = "null_as_zero")]
    input_tokens: u32,
    #[serde(default, deserialize_with = "null_as_zero")]
    output_tokens: u32,
    #[serde(default, deserialize_with = "null_as_zero")]
    cache_read_input_tokens: u32,
    #[serde(default, deserialize_with = "null_as_zero")]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    server_tool_use: Option<ServerToolUsage>,
    /// OpenRouter extension: total request cost in USD, search fees included.
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ServerToolUsage {
    #[serde(default, deserialize_with = "null_as_zero")]
    web_search_requests: u32,
}

fn null_as_zero<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    Ok(Option::<u32>::deserialize(deserializer)?.unwrap_or(0))
}

#[derive(Debug, Deserialize)]
struct MessageDeltaEvent {
    delta: MessageDelta,
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[cfg(test)]
mod web_search_tests;

#[cfg(test)]
mod tests {
    use super::{
        apply_cache_control, build_anthropic_messages, build_claude_code_headers,
        build_native_anthropic_tools, build_oauth_system_blocks, build_text_blocks,
        build_thinking_params, convert_tools_to_oauth_sdk_like_anthropic, next_sse_separator,
        normalize_anthropic_model, rewrite_oauth_tool_use_blocks, sse_line_value,
        utf8_prefix_chars, AnthropicHistoryOptions, AnthropicMessage, HistoryContentBlock,
        NativeChatRequest, NativeSystemBlock, ThinkingParam, WebSearchTool, BETA_FLAGS, CACHE_TTL,
        CLAUDE_CODE_VERSION,
    };
    use crate::session::models::{
        ChatMessage, ImageData, MessageRole, ServerToolKind, ServerToolResultBlock, ToolCallInfo,
    };
    use serde_json::json;

    /// Assertions below inspect the serialized request shape — the actual
    /// wire contract — so convert the typed history back to JSON values.
    fn to_json(messages: Vec<AnthropicMessage>) -> Vec<serde_json::Value> {
        messages
            .into_iter()
            .map(|message| serde_json::to_value(message).expect("message serializes"))
            .collect()
    }

    #[test]
    fn native_chat_request_keeps_type_first_and_system_before_messages() {
        let body = NativeChatRequest {
            model: "m",
            max_tokens: 64,
            system: [NativeSystemBlock {
                block_type: "text",
                text: "sys",
            }],
            messages: vec![AnthropicMessage {
                role: "user",
                content: vec![HistoryContentBlock::text("hi")],
            }],
            stream: true,
            thinking: None,
            output_config: None,
            tools: None,
        };
        let serialized = serde_json::to_string(&body).unwrap();
        // Gateways that mis-parse alphabetized blocks (issue #91) require the
        // tag to lead each block and system to precede the bulky messages.
        assert!(serialized.contains(r#""system":[{"type":"text","text":"sys"}]"#));
        assert!(serialized.contains(r#""content":[{"type":"text","text":"hi"}]"#));
        let system_pos = serialized.find("\"system\"").unwrap();
        let messages_pos = serialized.find("\"messages\"").unwrap();
        assert!(system_pos < messages_pos);
        assert!(!serialized.contains("\"thinking\""));
        assert!(!serialized.contains("\"output_config\""));
        assert!(!serialized.contains("\"tools\""));
    }

    #[test]
    fn thinking_param_serializes_tag_first() {
        assert_eq!(
            serde_json::to_string(&ThinkingParam::Enabled {
                budget_tokens: 5000
            })
            .unwrap(),
            r#"{"type":"enabled","budget_tokens":5000}"#
        );
        assert_eq!(
            serde_json::to_string(&ThinkingParam::Adaptive).unwrap(),
            r#"{"type":"adaptive"}"#
        );
        assert_eq!(
            serde_json::to_string(&ThinkingParam::Disabled).unwrap(),
            r#"{"type":"disabled"}"#
        );
    }

    #[test]
    fn oauth_client_version_is_current_and_consistent() {
        assert_eq!(CLAUDE_CODE_VERSION, "2.1.302");
        let headers = build_claude_code_headers(
            "test-token", None, "session", "request", Some(BETA_FLAGS), 0,
        );
        assert_eq!(
            headers.iter().find(|(name, _)| name == "User-Agent").unwrap().1,
            "claude-cli/2.1.302 (external, sdk-cli)"
        );
        let system = build_oauth_system_blocks(&["test system"]);
        assert_eq!(
            system[0]["text"],
            "x-anthropic-billing-header: cc_version=2.1.302; cc_entrypoint=sdk-rs; cch=00000;"
        );
    }

    #[test]
    fn current_oauth_model_ids_pass_through_unchanged() {
        for model in [
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
        ] {
            assert_eq!(normalize_anthropic_model(model), model);
        }
        // Historical sessions keep their existing model mapping.
        assert_eq!(normalize_anthropic_model("claude-opus-4.8"), "claude-opus-4-8");
    }

    #[test]
    fn current_oauth_models_preserve_effort_params() {
        for model in ["claude-opus-5-5", "claude-fable-5-1", "claude-sonnet-5-5"] {
            for level in ["low", "medium", "high", "xhigh", "max"] {
                let (thinking, output_config, max_tokens) = build_thinking_params(model, Some(level));
                assert_eq!(
                    serde_json::to_value(thinking.unwrap()).unwrap(),
                    json!({ "type": "adaptive" })
                );
                assert_eq!(output_config, Some(json!({ "effort": level })));
                let expected_max_tokens = if matches!(level, "xhigh" | "max") { 64000 } else { 32000 };
                assert_eq!(max_tokens, expected_max_tokens);
            }
        }
        let (thinking, output_config, _) = build_thinking_params("claude-opus-5-5", None);
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "adaptive" })
        );
        assert_eq!(output_config, None);

        let (thinking, output_config, _) = build_thinking_params("claude-haiku-5-5", Some("high"));
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "enabled", "budget_tokens": 16000 })
        );
        assert_eq!(output_config, None);
    }

    #[test]
    fn current_claude_models_use_adaptive_effort_params() {
        let (thinking, output_config, max_tokens) =
            build_thinking_params("claude-opus-4-8", Some("xhigh"));
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "adaptive" })
        );
        assert_eq!(output_config, Some(json!({ "effort": "xhigh" })));
        assert_eq!(max_tokens, 64000);

        let (thinking, output_config, _) = build_thinking_params("claude-fable-5", Some("none"));
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "adaptive" })
        );
        assert_eq!(output_config, None);

        let (thinking, output_config, _) = build_thinking_params("claude-sonnet-5", Some("none"));
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "adaptive" })
        );
        assert_eq!(output_config, None);

        let (thinking, output_config, max_tokens) =
            build_thinking_params("claude-sonnet-5", Some("xhigh"));
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            json!({ "type": "adaptive" })
        );
        assert_eq!(output_config, Some(json!({ "effort": "xhigh" })));
        assert_eq!(max_tokens, 64000);
    }

    #[test]
    fn apply_cache_control_without_ttl_omits_the_field() {
        let mut messages = vec![AnthropicMessage {
            role: "user",
            content: vec![HistoryContentBlock::text("hi")],
        }];
        apply_cache_control(&mut messages, None);
        let messages = to_json(messages);
        assert_eq!(
            messages[0]["content"][0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
    }

    fn assistant_message(
        content: &str,
        thinking_content: Option<&str>,
        thinking_signature: Option<&str>,
    ) -> ChatMessage {
        ChatMessage {
            id: "assistant".to_string(),
            role: MessageRole::Assistant,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: thinking_content.map(str::to_string),
            thinking_duration: None,
            thinking_signature: thinking_signature.map(str::to_string),
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    #[test]
    fn utf8_prefix_chars_handles_unicode_error_body() {
        let value = format!("{}x", "错".repeat(200));

        assert_eq!(utf8_prefix_chars(&value, 200), "错".repeat(200));
    }

    fn tool_message(tool_call_id: &str, content: &str) -> ChatMessage {
        let mut message = assistant_message(content, None, None);
        message.role = MessageRole::Tool;
        message.tool_call_id = Some(tool_call_id.to_string());
        message
    }

    fn openai_tool(name: &str, deferred: bool) -> serde_json::Value {
        let mut tool = json!({
            "type": "function",
            "function": {
                "name": name,
                "description": format!("{} description", name),
                "parameters": { "type": "object", "properties": {} }
            }
        });
        if deferred {
            tool["defer_loading"] = json!(true);
        }
        tool
    }

    #[test]
    fn native_tools_keep_cache_breakpoint_on_last_eager_tool() {
        let tools = build_native_anthropic_tools(
            &[
                openai_tool("read", false),
                openai_tool("pdf_export", true),
                openai_tool("edit", false),
            ],
            None,
        );

        // Eager tools first (input order), deferred after; breakpoint on the
        // last eager tool and never on a deferred one.
        assert_eq!(tools[0]["name"], json!("read"));
        assert_eq!(tools[1]["name"], json!("edit"));
        assert_eq!(tools[2]["name"], json!("pdf_export"));
        assert!(tools[0].get("cache_control").is_none());
        assert!(tools[1].get("cache_control").is_some());
        assert!(tools[2].get("cache_control").is_none());
        assert_eq!(tools[2]["defer_loading"], json!(true));
        assert!(tools[0].get("defer_loading").is_none());
    }

    #[test]
    fn native_tools_without_deferred_match_legacy_shape() {
        let tools = build_native_anthropic_tools(
            &[openai_tool("read", false), openai_tool("edit", false)],
            Some(&WebSearchTool::AnthropicNative),
        );

        assert_eq!(tools.len(), 3);
        assert_eq!(tools[2]["name"], json!("web_search"));
        assert!(tools[2].get("cache_control").is_some());
        assert!(tools.iter().all(|tool| tool.get("defer_loading").is_none()));
    }

    #[test]
    fn oauth_tools_carry_defer_loading_marker() {
        let (tools, _aliases) = convert_tools_to_oauth_sdk_like_anthropic(&[
            openai_tool("read", false),
            openai_tool("pdf_export", true),
        ]);

        assert!(tools[0].get("defer_loading").is_none());
        assert_eq!(tools[1]["defer_loading"], json!(true));
        // No cache_control on the OAuth tools section (breakpoints live in
        // system/messages there).
        assert!(tools.iter().all(|tool| tool.get("cache_control").is_none()));
    }

    #[test]
    fn tool_reference_marker_becomes_reference_blocks_for_declared_tools() {
        let content = format!(
            "pdf_export is ready.\n\n{}pdf_export, ghost_tool{}",
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_OPEN,
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_CLOSE
        );
        let history = vec![tool_message("toolu_1", &content)];
        let declared: std::collections::HashSet<String> =
            ["pdf_export".to_string(), "read".to_string()].into();

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard().with_tool_reference_names(Some(declared)),
        ));

        let blocks = messages[0]["content"][0]["content"]
            .as_array()
            .expect("tool_result content blocks");
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(blocks[0]["text"], json!("pdf_export is ready."));
        // ghost_tool is not declared in this request: reference dropped, no
        // dangling `Tool reference not found` 400 possible.
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["type"], json!("tool_reference"));
        assert_eq!(blocks[1]["tool_name"], json!("pdf_export"));
    }

    #[test]
    fn tool_reference_marker_stays_verbatim_text_without_declared_set() {
        let content = format!(
            "done\n\n{}pdf_export{}",
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_OPEN,
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_CLOSE
        );
        let history = vec![tool_message("toolu_1", &content)];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard(),
        ));

        // Fallback path: content is a plain string, byte-identical.
        assert_eq!(messages[0]["content"][0]["content"], json!(content));
    }

    #[test]
    fn oauth_rewrite_maps_tool_reference_names_to_public_aliases() {
        let content = format!(
            "ready\n\n{}knowledge_move{}",
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_OPEN,
            crate::llm::tool_references::TOOL_REFERENCE_MARKER_CLOSE
        );
        let history = vec![tool_message("toolu_1", &content)];
        let declared: std::collections::HashSet<String> = ["knowledge_move".to_string()].into();
        let mut messages = build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard().with_tool_reference_names(Some(declared)),
        );
        let (_tools, aliases) =
            convert_tools_to_oauth_sdk_like_anthropic(&[openai_tool("knowledge_move", true)]);

        rewrite_oauth_tool_use_blocks(&mut messages, &aliases);

        let messages = to_json(messages);
        let blocks = messages[0]["content"][0]["content"]
            .as_array()
            .expect("blocks");
        assert_eq!(blocks[1]["type"], json!("tool_reference"));
        assert_eq!(blocks[1]["tool_name"], json!("KnowledgeMove"));
    }

    #[test]
    fn strip_native_lazy_features_degrades_to_eager_declarations() {
        let mut body = json!({
            "tools": [
                { "name": "read", "input_schema": {} },
                { "name": "pdf_export", "input_schema": {}, "defer_loading": true }
            ],
            "messages": [
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": [
                        { "type": "text", "text": "ready" },
                        { "type": "tool_reference", "tool_name": "pdf_export" }
                    ]}
                ]}
            ]
        });

        assert!(super::body_uses_native_lazy_features(&body));
        super::strip_native_lazy_features(&mut body);
        assert!(!super::body_uses_native_lazy_features(&body));

        assert!(body["tools"][1].get("defer_loading").is_none());
        let blocks = body["messages"][0]["content"][0]["content"]
            .as_array()
            .expect("tool_result blocks");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], json!("text"));
    }

    #[test]
    fn splits_system_reminder_text_into_separate_blocks() {
        let blocks = build_text_blocks(
            "<system-reminder>\nalpha\n</system-reminder>\n\nreply with exactly: ok",
        );

        assert_eq!(blocks.len(), 2);
        let blocks = serde_json::to_value(&blocks).unwrap();
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(
            blocks[0]["text"],
            json!("<system-reminder>\nalpha\n</system-reminder>\n\n")
        );
        assert_eq!(blocks[1]["text"], json!("reply with exactly: ok"));
    }

    #[test]
    fn anthropic_sse_separator_accepts_lf_and_crlf_blocks() {
        let lf = "event: ping\ndata: {}\n\nrest";
        let (pos, sep_len) = next_sse_separator(lf).expect("lf separator");
        assert_eq!(&lf[..pos], "event: ping\ndata: {}");
        assert_eq!(sep_len, 2);
        assert_eq!(&lf[pos + sep_len..], "rest");

        let crlf = "event: ping\r\ndata: {}\r\n\r\nrest";
        let (pos, sep_len) = next_sse_separator(crlf).expect("crlf separator");
        assert_eq!(&crlf[..pos], "event: ping\r\ndata: {}");
        assert_eq!(sep_len, 4);
        assert_eq!(&crlf[pos + sep_len..], "rest");
    }

    #[test]
    fn anthropic_sse_line_values_accept_optional_space_after_colon() {
        assert_eq!(
            sse_line_value("event:message_stop", "event"),
            Some("message_stop")
        );
        assert_eq!(
            sse_line_value("event: message_stop", "event"),
            Some("message_stop")
        );
        assert_eq!(
            sse_line_value("data:{\"type\":\"message_stop\"}", "data"),
            Some("{\"type\":\"message_stop\"}")
        );
        assert_eq!(
            sse_line_value("data: {\"type\":\"message_stop\"}", "data"),
            Some("{\"type\":\"message_stop\"}")
        );
    }

    #[test]
    fn native_lazy_beta_flag_is_part_of_the_oauth_beta_set() {
        // The custom-endpoint path derives its only beta header from this
        // flag; keep it in lockstep with the official BETA_FLAGS set.
        assert!(super::BETA_FLAGS.contains(super::NATIVE_LAZY_BETA_FLAG));
    }

    #[test]
    fn encodes_plain_text_messages_as_block_arrays_and_marks_cache_ttl() {
        let history = vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::User,
            content: "<system-reminder>\nalpha\n</system-reminder>\n\nreply with exactly: ok"
                .to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
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
        }];

        let mut messages = build_anthropic_messages(&history, AnthropicHistoryOptions::standard());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.len(), 2);

        apply_cache_control(&mut messages, Some(CACHE_TTL));

        let messages = to_json(messages);
        let blocks = messages[0]["content"]
            .as_array()
            .expect("content should be block array");
        assert_eq!(blocks[1]["cache_control"]["type"], json!("ephemeral"));
        assert_eq!(blocks[1]["cache_control"]["ttl"], json!(CACHE_TTL));
    }

    #[test]
    fn converts_oauth_tools_to_sdk_like_names_and_keys() {
        let api_tools = vec![json!({
            "type": "function",
            "function": {
                "name": "edit",
                "description": "Edit file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filePath": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "oldString": { "type": "string" },
                                    "newString": { "type": "string" },
                                    "replaceAll": { "type": "boolean" }
                                },
                                "required": ["oldString", "newString"]
                            }
                        }
                    },
                    "required": ["filePath", "edits"]
                }
            }
        })];

        let (tools, aliases) = convert_tools_to_oauth_sdk_like_anthropic(&api_tools);

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("Edit"));
        assert_eq!(
            tools[0]["input_schema"]["required"],
            json!(["file_path", "edits"])
        );
        assert!(tools[0]["input_schema"]["properties"]
            .get("file_path")
            .is_some());
        assert!(
            tools[0]["input_schema"]["properties"]["edits"]["items"]["properties"]
                .get("old_string")
                .is_some()
        );

        let public_input = aliases.public_input_for(
            "edit",
            json!({
                "filePath": "a.txt",
                "edits": [{
                    "oldString": "foo",
                    "newString": "bar",
                    "replaceAll": true
                }]
            }),
        );
        assert_eq!(
            public_input,
            json!({
                "file_path": "a.txt",
                "edits": [{
                    "old_string": "foo",
                    "new_string": "bar",
                    "replace_all": true
                }]
            })
        );

        let (internal_name, internal_args) = aliases.internalize_tool_call(
            "Edit",
            r#"{"file_path":"a.txt","edits":[{"old_string":"foo","new_string":"bar","replace_all":true}]}"#,
        );
        assert_eq!(internal_name, "edit");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&internal_args).unwrap(),
            json!({
                "filePath": "a.txt",
                "edits": [{
                    "oldString": "foo",
                    "newString": "bar",
                    "replaceAll": true
                }]
            })
        );
    }

    #[test]
    fn native_anthropic_tools_omit_web_search_when_disabled() {
        let tools = build_native_anthropic_tools(&[], None);

        assert!(tools.is_empty());
    }

    #[test]
    fn native_anthropic_tools_include_web_search_when_enabled() {
        let tools = build_native_anthropic_tools(&[], Some(&WebSearchTool::AnthropicNative));

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], json!("web_search_20250305"));
        assert_eq!(tools[0]["name"], json!("web_search"));
        assert_eq!(tools[0]["cache_control"]["type"], json!("ephemeral"));
    }

    #[test]
    fn custom_endpoint_history_omits_thinking_when_replay_disabled() {
        let history = vec![assistant_message("answer", Some("thinking"), Some("sig"))];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::custom_endpoint(false),
        ));

        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(blocks[0]["text"], json!("answer"));
    }

    #[test]
    fn custom_endpoint_history_skips_unsigned_thinking_when_replay_enabled() {
        let history = vec![assistant_message("answer", Some("thinking"), None)];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::custom_endpoint(true),
        ));

        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(blocks[0]["text"], json!("answer"));
    }

    #[test]
    fn custom_endpoint_history_replays_signed_thinking_when_enabled() {
        let history = vec![assistant_message("answer", Some("thinking"), Some("sig"))];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::custom_endpoint(true),
        ));

        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], json!("thinking"));
        assert_eq!(blocks[0]["thinking"], json!("thinking"));
        assert_eq!(blocks[0]["signature"], json!("sig"));
        assert_eq!(blocks[1]["type"], json!("text"));
        assert_eq!(blocks[1]["text"], json!("answer"));
    }

    #[test]
    fn standard_history_preserves_unsigned_thinking() {
        let history = vec![assistant_message("answer", Some("thinking"), None)];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard(),
        ));

        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], json!("thinking"));
        assert_eq!(blocks[0]["thinking"], json!("thinking"));
        assert!(blocks[0].get("signature").is_none());
    }

    #[test]
    fn oauth_request_cache_control_stays_within_anthropic_limit() {
        let history = vec![
            ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::User,
                content: "hello".to_string(),
                created_at: 0,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
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
            },
            ChatMessage {
                id: "m2".to_string(),
                role: MessageRole::Assistant,
                content: "hi".to_string(),
                created_at: 1,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
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
            },
            ChatMessage {
                id: "m3".to_string(),
                role: MessageRole::User,
                content: "show tools".to_string(),
                created_at: 2,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
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
            },
        ];

        let mut messages = build_anthropic_messages(&history, AnthropicHistoryOptions::standard());
        apply_cache_control(&mut messages, Some(CACHE_TTL));

        let messages = to_json(messages);
        let message_cache_blocks = messages
            .iter()
            .flat_map(|msg| {
                msg["content"]
                    .as_array()
                    .into_iter()
                    .flat_map(|blocks| blocks.iter())
            })
            .filter(|block| block.get("cache_control").is_some())
            .count();
        let system_cache_blocks = build_oauth_system_blocks(&["system prompt"])
            .as_array()
            .into_iter()
            .flat_map(|blocks| blocks.iter())
            .filter(|block| block.get("cache_control").is_some())
            .count();

        assert_eq!(message_cache_blocks, 2);
        assert_eq!(system_cache_blocks, 2);
        assert_eq!(message_cache_blocks + system_cache_blocks, 4);
    }

    #[test]
    fn rewrites_oauth_tool_use_blocks_to_public_shape() {
        let history = vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Assistant,
            content: String::new(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: Some(vec![ToolCallInfo {
                id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: r#"{"filePath":"Assets/Test.txt","limit":10}"#.to_string(),
                order: None,
                server_tool: None,
                server_tool_output: None,
                server_tool_result: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }]),
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }];

        let mut messages = build_anthropic_messages(&history, AnthropicHistoryOptions::standard());
        let api_tools = vec![json!({
            "type": "function",
            "function": {
                "name": "read",
                "description": "Read file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filePath": { "type": "string" },
                        "limit": { "type": "integer" }
                    },
                    "required": ["filePath"]
                }
            }
        })];
        let (_, aliases) = convert_tools_to_oauth_sdk_like_anthropic(&api_tools);

        rewrite_oauth_tool_use_blocks(&mut messages, &aliases);

        let messages = to_json(messages);
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["name"], json!("Read"));
        assert_eq!(
            content[0]["input"],
            json!({
                "file_path": "Assets/Test.txt",
                "limit": 10
            })
        );
    }

    #[test]
    fn inserts_missing_tool_results_as_separate_user_tool_blocks() {
        let history = vec![
            ChatMessage {
                id: "assistant-1".to_string(),
                role: MessageRole::Assistant,
                content: String::new(),
                created_at: 0,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                content_order: None,
                thinking_order: None,
                tool_calls: Some(vec![
                    ToolCallInfo {
                        id: "call_1".to_string(),
                        name: "read".to_string(),
                        arguments: r#"{"filePath":"Assets/Test.txt"}"#.to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        server_tool_result: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "call_2".to_string(),
                        name: "grep".to_string(),
                        arguments: r#"{"pattern":"Test","path":"Assets"}"#.to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        server_tool_result: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ]),
                tool_call_id: None,
                images: None,
                asset_refs: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
                render_parts: None,
            },
            ChatMessage {
                id: "user-1".to_string(),
                role: MessageRole::User,
                content: "继续".to_string(),
                created_at: 1,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
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
            },
        ];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard(),
        ));

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], json!("assistant"));
        assert_eq!(messages[1]["role"], json!("user"));
        assert_eq!(messages[2]["role"], json!("user"));

        let tool_result_blocks = messages[1]["content"]
            .as_array()
            .expect("tool result message should be block array");
        assert_eq!(tool_result_blocks.len(), 2);
        assert_eq!(tool_result_blocks[0]["type"], json!("tool_result"));
        assert_eq!(tool_result_blocks[0]["tool_use_id"], json!("call_1"));
        assert_eq!(tool_result_blocks[1]["type"], json!("tool_result"));
        assert_eq!(tool_result_blocks[1]["tool_use_id"], json!("call_2"));

        let user_blocks = messages[2]["content"]
            .as_array()
            .expect("user text message should be block array");
        assert_eq!(user_blocks.len(), 1);
        assert_eq!(user_blocks[0]["type"], json!("text"));
        assert_eq!(user_blocks[0]["text"], json!("继续"));
    }

    const SEARCH_OUTPUT: &str = "老舍 茶馆\n1. 茶馆 第一幕 (https://example.com/chaguan)";

    fn search_call(result_origin: Option<&str>) -> ToolCallInfo {
        ToolCallInfo {
            id: "call_01_search".to_string(),
            name: "web_search".to_string(),
            arguments: r#"{"query":"老舍 茶馆"}"#.to_string(),
            order: Some(103),
            server_tool: Some(ServerToolKind::WebSearch),
            server_tool_output: Some(SEARCH_OUTPUT.to_string()),
            server_tool_result: result_origin.map(|origin| ServerToolResultBlock {
                origin: origin.to_string(),
                block_type: "web_search_tool_result".to_string(),
                content: json!([{
                    "type": "web_search_result",
                    "title": "茶馆 第一幕",
                    "url": "https://example.com/chaguan",
                    "encrypted_content": "enc",
                    "page_age": null
                }]),
            }),
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        }
    }

    /// The DeepSeek turn that failed: a client `todowrite` streamed before a
    /// server `web_search` that came back with its result block.
    fn deepseek_mixed_turn(result_origin: Option<&str>) -> Vec<ChatMessage> {
        let mut assistant = assistant_message("现在处理现代中文作品。", Some("plan"), Some("sig"));
        assistant.tool_calls = Some(vec![
            ToolCallInfo {
                id: "call_00_todo".to_string(),
                name: "todowrite".to_string(),
                arguments: r#"{"todos":[]}"#.to_string(),
                order: Some(102),
                server_tool: None,
                server_tool_output: None,
                server_tool_result: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            },
            search_call(result_origin),
        ]);
        vec![assistant, tool_message("call_00_todo", "Todos updated.")]
    }

    fn block_types(message: &serde_json::Value) -> Vec<&str> {
        message["content"]
            .as_array()
            .expect("content blocks")
            .iter()
            .map(|block| block["type"].as_str().expect("block type"))
            .collect()
    }

    fn assert_search_note(block: &serde_json::Value) {
        let note = block["text"].as_str().expect("note text");
        assert!(note.starts_with("<system-reminder>"), "{note}");
        assert!(note.contains("not text you wrote"), "{note}");
        assert!(note.contains(SEARCH_OUTPUT), "{note}");
        assert!(!note.contains("[Web Search Result]"), "{note}");
    }

    #[test]
    fn server_search_without_result_block_replays_as_user_side_note() {
        // Search text after the pending tool_use was rejected ("messages.N.3:
        // tool_use ids were found without tool_result blocks immediately
        // after"), and in the assistant's voice it taught DeepSeek to write
        // fabricated `[Web Search Result]` lists itself.
        let messages = to_json(build_anthropic_messages(
            &deepseek_mixed_turn(None),
            AnthropicHistoryOptions::custom_endpoint(true),
        ));

        assert_eq!(messages.len(), 2);
        assert_eq!(
            block_types(&messages[0]),
            vec!["thinking", "text", "tool_use"]
        );
        assert_eq!(messages[0]["content"][2]["id"], json!("call_00_todo"));
        assert_eq!(messages[1]["role"], json!("user"));
        assert_eq!(block_types(&messages[1]), vec!["tool_result", "text"]);
        assert_eq!(
            messages[1]["content"][0]["tool_use_id"],
            json!("call_00_todo")
        );
        assert_search_note(&messages[1]["content"][1]);
    }

    #[test]
    fn server_search_replays_natively_to_the_endpoint_that_ran_it() {
        let messages = to_json(build_anthropic_messages(
            &deepseek_mixed_turn(Some("api.deepseek.com")),
            AnthropicHistoryOptions::custom_endpoint(true)
                .with_server_tool_replay_origin(Some("api.deepseek.com".to_string())),
        ));

        assert_eq!(messages.len(), 2);
        assert_eq!(
            block_types(&messages[0]),
            vec![
                "thinking",
                "server_tool_use",
                "web_search_tool_result",
                "text",
                "tool_use"
            ]
        );
        let server_use = &messages[0]["content"][1];
        assert_eq!(server_use["id"], json!("call_01_search"));
        assert_eq!(server_use["name"], json!("web_search"));
        assert_eq!(server_use["input"], json!({ "query": "老舍 茶馆" }));
        assert_eq!(
            messages[0]["content"][2]["content"][0]["encrypted_content"],
            json!("enc")
        );
        assert_eq!(block_types(&messages[1]), vec!["tool_result"]);
    }

    #[test]
    fn server_search_from_another_endpoint_falls_back_to_user_side_note() {
        let messages = to_json(build_anthropic_messages(
            &deepseek_mixed_turn(Some("openrouter.ai")),
            AnthropicHistoryOptions::custom_endpoint(true)
                .with_server_tool_replay_origin(Some("api.deepseek.com".to_string())),
        ));

        assert_eq!(
            block_types(&messages[0]),
            vec!["thinking", "text", "tool_use"]
        );
        assert_eq!(block_types(&messages[1]), vec!["tool_result", "text"]);
        assert_search_note(&messages[1]["content"][1]);
    }

    #[test]
    fn server_search_note_leads_the_next_user_message_or_stands_alone() {
        let mut assistant = assistant_message("搜到了。", None, None);
        assistant.tool_calls = Some(vec![search_call(None)]);
        let mut user = assistant_message("继续", None, None);
        user.role = MessageRole::User;

        let messages = to_json(build_anthropic_messages(
            &[assistant.clone(), user],
            AnthropicHistoryOptions::standard(),
        ));
        assert_eq!(messages.len(), 2);
        assert_eq!(block_types(&messages[0]), vec!["text"]);
        assert_eq!(block_types(&messages[1]), vec!["text", "text"]);
        assert_search_note(&messages[1]["content"][0]);
        assert_eq!(messages[1]["content"][1]["text"], json!("继续"));

        let trailing = to_json(build_anthropic_messages(
            &[assistant],
            AnthropicHistoryOptions::standard(),
        ));
        assert_eq!(trailing.len(), 2);
        assert_eq!(trailing[1]["role"], json!("user"));
        assert_search_note(&trailing[1]["content"][0]);
    }

    #[test]
    fn tool_result_images_are_nested_in_anthropic_tool_result_content() {
        let history = vec![
            ChatMessage {
                id: "assistant-1".to_string(),
                role: MessageRole::Assistant,
                content: String::new(),
                created_at: 0,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                content_order: None,
                thinking_order: None,
                tool_calls: Some(vec![ToolCallInfo {
                    id: "call_1".to_string(),
                    name: "capture_unity_screenshot".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    server_tool_result: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                tool_call_id: None,
                images: None,
                asset_refs: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
                render_parts: None,
            },
            ChatMessage {
                id: "tool-1".to_string(),
                role: MessageRole::Tool,
                content: "Unity screenshot captured.".to_string(),
                created_at: 1,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                content_order: None,
                thinking_order: None,
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                images: Some(vec![ImageData { capture: None,
                    data: "YWJj".to_string(),
                    mime_type: "image/png".to_string(),
                }]),
                asset_refs: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
                render_parts: None,
            },
        ];

        let messages = to_json(build_anthropic_messages(
            &history,
            AnthropicHistoryOptions::standard(),
        ));
        let tool_result = &messages[1]["content"][0];
        assert_eq!(tool_result["type"], json!("tool_result"));
        let content = tool_result["content"]
            .as_array()
            .expect("tool_result content should be a block array");
        assert_eq!(content[0]["type"], json!("text"));
        assert_eq!(content[1]["type"], json!("image"));
        assert_eq!(content[1]["source"]["media_type"], json!("image/png"));
        assert_eq!(content[1]["source"]["data"], json!("YWJj"));
    }
}
