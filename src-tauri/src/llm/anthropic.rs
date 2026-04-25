use futures::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::session::models::{ChatMessage, MessageRole, ServerToolKind, ToolCallInfo};

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
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

const BETA_FLAGS: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advanced-tool-use-2025-11-20,effort-2025-11-24";
const API_VERSION: &str = "2023-06-01";
const API_BASE: &str = "https://api.anthropic.com";

const USER_AGENT: &str = "claude-cli/2.1.92 (external, sdk-cli)";
const X_APP: &str = "cli";
const X_STAINLESS_LANG: &str = "js";
const X_STAINLESS_PACKAGE_VERSION: &str = "0.80.0";
const X_STAINLESS_RUNTIME: &str = "node";
const X_STAINLESS_RUNTIME_VERSION: &str = "v24.3.0";
const X_STAINLESS_TIMEOUT: &str = "600";
const CACHE_TTL: &str = "1h";

const BILLING_HEADER_BLOCK: &str =
    "x-anthropic-billing-header: cc_version=2.1.92.19c; cc_entrypoint=sdk-rs; cch=00000;";
const AGENT_SDK_IDENTITY: &str = "You are a Claude agent, built on Anthropic's Claude Agent SDK.";
const OAUTH_COMPAT_FALLBACK_MODEL: &str = "claude-opus-4-1-20250805";

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
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let client = reqwest::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .tcp_nodelay(true)
        .connect_timeout(std::time::Duration::from_secs(30))
        .pool_idle_timeout(std::time::Duration::from_secs(60))
        .pool_max_idle_per_host(1)
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(std::time::Duration::from_secs(20))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let model = model.strip_prefix("anthropic/").unwrap_or(model);
    let mut effective_model = normalize_anthropic_model(model).to_string();

    let mut messages = build_anthropic_messages(history);
    let (converted_tools, tool_aliases) = convert_tools_to_oauth_sdk_like_anthropic(tools);
    rewrite_oauth_tool_use_blocks(&mut messages, &tool_aliases);
    let anthropic_tools = converted_tools;
    let oauth_tool_aliases = Some(tool_aliases);

    let system_blocks = build_oauth_system_blocks(system_parts);
    let (thinking_field, output_config, standard_max_tokens) =
        build_thinking_params(&effective_model, thinking_level);
    let max_tokens = u64::from(standard_max_tokens);
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
        body["thinking"] = thinking;
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

    if let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        apply_cache_control(msgs);
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
    let is_first_user_turn = history.len() == 1
        && history[0].role == MessageRole::User
        && history[0].tool_call_id.is_none();

    for attempt in 0..=MAX_RETRIES {
        let headers = build_claude_code_headers(
            access_token,
            None,
            &session_id,
            &client_request_id,
            BETA_FLAGS,
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

        if is_oauth_generic_bad_request(status, &error_body) && attempt < MAX_RETRIES {
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
                &error_body[..error_body.len().min(200)]
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            last_error = format!("Anthropic API error ({}): {}", status, error_body);
            continue;
        }

        return Err(format!("Anthropic API error ({}): {}", status, error_body));
    }

    Err(last_error)
}

pub async fn stream_chat_native<F, G, H>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: &str,
    extra_beta_flags: &[String],
    request_session_id: Option<&str>,
    tag: &str,
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let client = reqwest::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .tcp_nodelay(true)
        .connect_timeout(std::time::Duration::from_secs(30))
        .pool_idle_timeout(std::time::Duration::from_secs(60))
        .pool_max_idle_per_host(1)
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(std::time::Duration::from_secs(20))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let messages = build_anthropic_messages(history);

    let mut anthropic_tools = convert_tools_to_anthropic(tools);

    // Inject web search server tool schema (Anthropic API native)
    anthropic_tools.push(serde_json::json!({
        "type": "web_search_20250305",
        "name": "web_search",
        "max_uses": 5
    }));

    if let Some(last_tool) = anthropic_tools.last_mut() {
        last_tool["cache_control"] = serde_json::json!({ "type": "ephemeral", "ttl": CACHE_TTL });
    }

    let system_blocks = serde_json::json!([{
        "type": "text",
        "text": system_prompt,
    }]);

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 16384,
        "system": system_blocks,
        "messages": messages,
        "stream": true,
    });

    if !anthropic_tools.is_empty() {
        body["tools"] = serde_json::json!(anthropic_tools);
    }

    if let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        apply_cache_control(msgs);
    }

    let raw_request = serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    eprintln!(
        "[{}] POST model={} messages={} tools={}",
        tag,
        model,
        history.len(),
        anthropic_tools.len()
    );

    let api_url = format!("{}/messages", base_url.trim_end_matches('/'));
    let beta_flags = resolve_native_beta_flags(extra_beta_flags);
    let session_id = request_header_session_id(request_session_id);
    let client_request_id = uuid::Uuid::new_v4().to_string();

    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut last_error = String::new();

    for attempt in 0..=MAX_RETRIES {
        let headers = build_claude_code_headers(
            api_key,
            Some(api_key),
            &session_id,
            &client_request_id,
            &beta_flags,
            attempt,
        );
        let mut req = client.post(&api_url);
        for (key, value) in &headers {
            req = req.header(key.as_str(), value.as_str());
        }
        match req.json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return parse_anthropic_sse(
                        resp,
                        raw_request,
                        None,
                        on_text_delta,
                        on_thinking_delta,
                        on_tool_call_start,
                    )
                    .await;
                }

                let is_retryable =
                    status.as_u16() == 429 || status.as_u16() == 529 || status.is_server_error();
                let error_body = resp.text().await.unwrap_or_default();

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
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_text = String::new();
    let mut full_thinking = String::new();
    let mut thinking_signature = String::new();
    let mut raw_response = String::new();
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
    let mut server_tool_inputs: HashMap<usize, String> = HashMap::new();

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
                    eprintln!("[Anthropic] {} (no response data received)", error_chain);
                } else {
                    let tail = if raw_response.len() > 2000 {
                        &raw_response[raw_response.len() - 2000..]
                    } else {
                        &raw_response
                    };
                    eprintln!(
                        "[Anthropic] {}\n  raw_response_len={}, last_chunk:\n{}",
                        error_chain,
                        raw_response.len(),
                        tail
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
                    "[Anthropic] Stream chunk read timed out after {}s",
                    CHUNK_TIMEOUT.as_secs()
                );
                return Err(format!(
                    "Stream read timed out after {}s, partial_data: text_len={}, tools={}",
                    CHUNK_TIMEOUT.as_secs(),
                    full_text.len(),
                    tool_calls.len()
                ));
            }
        };

        let chunk_text = String::from_utf8_lossy(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);

        while let Some(pos) = buffer.find("\n\n") {
            let event_text = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            let mut event_type = String::new();
            let mut data_str = String::new();

            for line in event_text.lines() {
                let line = line.trim();
                if let Some(et) = line.strip_prefix("event: ") {
                    event_type = et.trim().to_string();
                } else if let Some(d) = line.strip_prefix("data: ") {
                    data_str = d.trim().to_string();
                }
            }

            if data_str.is_empty() {
                continue;
            }

            match event_type.as_str() {
                "content_block_start" => {
                    if let Ok(ev) = serde_json::from_str::<ContentBlockStartEvent>(&data_str) {
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
                                });
                            }
                            "server_tool_use" => {
                                let name = ev.content_block.name.unwrap_or_default();
                                let id = ev.content_block.id.unwrap_or_default();
                                on_tool_call_start(id.clone(), name.clone());
                                tool_calls.push(PartialToolCall {
                                    index: ev.index,
                                    id,
                                    name,
                                    input_json: String::new(),
                                    server_tool_output: None,
                                });
                                server_tool_inputs.insert(ev.index, String::new());
                            }
                            "web_search_tool_result" => {
                                // Find the most recent server tool call to attach results to.
                                let server_tc = tool_calls
                                    .iter_mut()
                                    .rev()
                                    .find(|tc| tc.name == "web_search");
                                if let Some(content) = &ev.content_block.content {
                                    if let Some(arr) = content.as_array() {
                                        let query = server_tool_inputs
                                            .values()
                                            .last()
                                            .and_then(|json| {
                                                serde_json::from_str::<serde_json::Value>(json).ok()
                                            })
                                            .and_then(|v| {
                                                v.get("query")
                                                    .and_then(|q| q.as_str().map(String::from))
                                            })
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
                                        if !hits.is_empty() {
                                            let summary = format!(
                                                "{}\n{}",
                                                query,
                                                hits.iter()
                                                    .enumerate()
                                                    .map(|(i, h)| format!(
                                                        "{}. {} ({})",
                                                        i + 1,
                                                        h.title,
                                                        h.url
                                                    ))
                                                    .collect::<Vec<_>>()
                                                    .join("\n")
                                            );
                                            if let Some(tc) = server_tc {
                                                tc.input_json =
                                                    serde_json::json!({ "query": &query })
                                                        .to_string();
                                                tc.server_tool_output = Some(summary);
                                            }
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
                            _ => {} // text block, thinking block, etc.
                        }
                    }
                }
                "content_block_delta" => {
                    if let Ok(ev) = serde_json::from_str::<ContentBlockDeltaEvent>(&data_str) {
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
                                    full_text.push_str(text);
                                    on_text_delta(text.clone());
                                }
                            }
                            "input_json_delta" => {
                                if let Some(ref json) = ev.delta.partial_json {
                                    if let Some(tc) =
                                        tool_calls.iter_mut().find(|tc| tc.index == ev.index)
                                    {
                                        tc.input_json.push_str(json);
                                    }
                                    // Also accumulate for server tool inputs (web search)
                                    if let Some(input) = server_tool_inputs.get_mut(&ev.index) {
                                        input.push_str(json);
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
                    if let Ok(ev) = serde_json::from_str::<MessageDeltaEvent>(&data_str) {
                        if let Some(ref reason) = ev.delta.stop_reason {
                            stop_reason = reason.clone();
                        }
                        if let Some(ref usage) = ev.usage {
                            output_tokens = usage.output_tokens;
                        }
                    }
                }
                "message_stop" => {
                    got_message_stop = true;
                }
                "message_start" => {
                    if let Ok(ev) = serde_json::from_str::<MessageStartEvent>(&data_str) {
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
                _ => {}
            }
        }
    }

    if !got_message_stop {
        if !full_text.is_empty() || !tool_calls.is_empty() {
            return Err(format!(
                "Stream ended without message_stop (incomplete response, text_len={}, tools={})",
                full_text.len(),
                tool_calls.len()
            ));
        }
        return Err("Stream ended with no data and no message_stop".to_string());
    }

    let final_tool_calls: Vec<ToolCallInfo> = tool_calls
        .into_iter()
        .map(|tc| {
            let (name, arguments) = if tc.server_tool_output.is_some() {
                (tc.name, tc.input_json)
            } else if let Some(aliases) = oauth_tool_aliases {
                aliases.internalize_tool_call(&tc.name, &tc.input_json)
            } else {
                (tc.name, tc.input_json)
            };
            let server_tool = if tc.server_tool_output.is_some() && name == "web_search" {
                Some(ServerToolKind::WebSearch)
            } else {
                None
            };

            ToolCallInfo {
                id: tc.id,
                name,
                arguments,
                server_tool,
                server_tool_output: tc.server_tool_output,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }
        })
        .collect();

    let finish_reason = if !final_tool_calls.is_empty() {
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

///
///
///
fn build_thinking_params(
    model: &str,
    thinking_level: Option<&str>,
) -> (Option<serde_json::Value>, Option<serde_json::Value>, u32) {
    let is_adaptive = model.contains("sonnet-4.6")
        || model.contains("sonnet-4-6")
        || model.contains("opus-4.6")
        || model.contains("opus-4-6");

    let level = match thinking_level {
        Some(l) if !l.is_empty() => l,
        _ if is_adaptive => {
            let thinking = serde_json::json!({ "type": "adaptive" });
            return (Some(thinking), None, 32000);
        }
        _ => return (None, None, 16384),
    };

    if level == "none" {
        let thinking = serde_json::json!({ "type": "disabled" });
        return (Some(thinking), None, 8192);
    }

    if is_adaptive {
        // https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking
        let thinking = serde_json::json!({ "type": "adaptive" });
        let output_config = match level {
            "low" | "medium" | "high" => Some(serde_json::json!({ "effort": level })),
            _ => None,
        };
        (Some(thinking), output_config, 32000)
    } else {
        let (budget, max_tokens) = match level {
            "low" => (2048, 8192),
            "medium" => (5000, 12000),
            "high" => (16000, 32000),
            _ => return (None, None, 16384),
        };
        let thinking = serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget,
        });
        (Some(thinking), None, max_tokens)
    }
}

fn build_anthropic_messages(history: &[ChatMessage]) -> Vec<serde_json::Value> {
    let history = crate::session::history::normalize_tool_round_history(history);
    let mut messages: Vec<serde_json::Value> = Vec::new();

    let mut i = 0;
    while i < history.len() {
        let msg = &history[i];
        match msg.role {
            MessageRole::User => {
                if let Some(ref images) = msg.images {
                    if !images.is_empty() {
                        let mut blocks: Vec<serde_json::Value> = Vec::new();
                        for img in images {
                            blocks.push(serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": img.mime_type,
                                    "data": img.data,
                                }
                            }));
                        }
                        blocks.extend(build_text_blocks(&msg.content));
                        if !blocks.is_empty() {
                            messages.push(serde_json::json!({
                                "role": "user",
                                "content": blocks,
                            }));
                        }
                        i += 1;
                        continue;
                    }
                }
                let blocks = build_text_blocks(&msg.content);
                if !blocks.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": blocks,
                    }));
                }
                i += 1;
            }
            MessageRole::Assistant => {
                let has_text = !msg.content.is_empty();
                let has_thinking = msg.thinking_content.as_ref().is_some_and(|t| !t.is_empty());
                let tool_calls = msg.tool_calls.as_ref();
                let has_tool_calls = tool_calls.is_some_and(|calls| !calls.is_empty());

                if !has_tool_calls && !has_thinking {
                    if has_text {
                        let text_blocks = build_text_blocks(&msg.content);
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": text_blocks,
                        }));
                    }
                    i += 1;
                    continue;
                }

                let mut content_blocks: Vec<serde_json::Value> = Vec::new();

                if has_thinking {
                    let thinking_text = msg.thinking_content.as_deref().unwrap_or_default();
                    let mut thinking_block = serde_json::json!({
                        "type": "thinking",
                        "thinking": thinking_text,
                    });
                    if let Some(ref sig) = msg.thinking_signature {
                        if !sig.is_empty() {
                            thinking_block["signature"] = serde_json::json!(sig);
                        }
                    }
                    content_blocks.push(thinking_block);
                }

                if has_text {
                    content_blocks.extend(build_text_blocks(&msg.content));
                }

                if let Some(tool_calls) = tool_calls {
                    for tc in tool_calls {
                        // Server tools (e.g. web_search) should not be sent back as tool_use
                        // blocks — include their output as text for context continuity.
                        if tc.is_server_tool() {
                            if let Some(ref output) = tc.server_tool_output {
                                content_blocks.push(serde_json::json!({
                                    "type": "text",
                                    "text": format!("[Web Search Result]\n{}", output),
                                }));
                            }
                            continue;
                        }
                        let input: serde_json::Value =
                            serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));
                        content_blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": input,
                        }));
                    }
                }

                if content_blocks.is_empty() {
                    i += 1;
                    continue;
                }

                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks,
                }));
                i += 1;
            }
            MessageRole::Tool => {
                let mut tool_result_blocks: Vec<serde_json::Value> = Vec::new();

                while i < history.len() && history[i].role == MessageRole::Tool {
                    let tool_msg = &history[i];
                    if let Some(tool_use_id) = tool_msg.tool_call_id.as_deref() {
                        if !tool_use_id.is_empty() {
                            tool_result_blocks.push(serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": tool_msg.content,
                            }));
                        } else {
                            eprintln!("[Anthropic] skipped tool_result with empty tool_use_id");
                        }
                    } else {
                        eprintln!("[Anthropic] skipped tool_result without tool_use_id");
                    }
                    i += 1;
                }

                if !tool_result_blocks.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": tool_result_blocks,
                    }));
                }
            }
        }
    }

    messages
}

fn build_oauth_system_blocks(system_parts: &[&str]) -> serde_json::Value {
    let mut blocks: Vec<serde_json::Value> = Vec::new();

    blocks.push(serde_json::json!({
        "type": "text",
        "text": BILLING_HEADER_BLOCK,
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
        "claude-sonnet-4.6" => "claude-sonnet-4-6",
        "claude-opus-4.6" => "claude-opus-4-6",
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

fn apply_cache_control(messages: &mut [serde_json::Value]) {
    let len = messages.len();
    if len == 0 {
        return;
    }

    let start = if len >= 2 { len - 2 } else { 0 };
    for msg in &mut messages[start..] {
        if let Some(content_arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(last_block) = content_arr.iter_mut().rev().find(|b| {
                matches!(
                    b.get("type").and_then(|t| t.as_str()),
                    Some("text") | Some("image")
                )
            }) {
                last_block["cache_control"] =
                    serde_json::json!({ "type": "ephemeral", "ttl": CACHE_TTL });
            }
        }
    }
}

/// OpenAI: { type: "function", function: { name, description, parameters } }
/// Anthropic: { name, description, input_schema }
fn convert_tools_to_anthropic(openai_tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let tools: Vec<serde_json::Value> = openai_tools
        .iter()
        .filter_map(|tool| {
            let func = tool.get("function")?;
            let name = func.get("name")?.as_str()?;
            let description = func.get("description")?.as_str().unwrap_or("");
            let parameters = func
                .get("parameters")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            Some(serde_json::json!({
                "name": name,
                "description": description,
                "input_schema": parameters,
            }))
        })
        .collect();

    tools
}

fn convert_tools_to_oauth_sdk_like_anthropic(
    openai_tools: &[serde_json::Value],
) -> (Vec<serde_json::Value>, OauthToolAliases) {
    let mut tools: Vec<serde_json::Value> = Vec::new();
    let mut aliases = OauthToolAliases::default();

    for tool in openai_tools {
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

        tools.push(serde_json::json!({
            "name": public_name,
            "description": description,
            "input_schema": public_input_schema,
        }));

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

fn rewrite_oauth_tool_use_blocks(messages: &mut [serde_json::Value], aliases: &OauthToolAliases) {
    for message in messages {
        let Some(content_blocks) = message.get_mut("content").and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for block in content_blocks {
            if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                continue;
            }
            let Some(internal_name) = block
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            else {
                continue;
            };
            let Some(public_name) = aliases.public_name_for(&internal_name) else {
                continue;
            };

            block["name"] = serde_json::Value::String(public_name.to_string());
            if let Some(input) = block.get_mut("input") {
                let current = std::mem::take(input);
                *input = aliases.public_input_for(&internal_name, current);
            }
        }
    }
}

fn oauth_public_tool_name(internal_name: &str) -> String {
    match internal_name {
        "ask_user_question" => "AskUserQuestion".to_string(),
        "config_query" => "ConfigQuery".to_string(),
        "knowledge_list" => "KnowledgeList".to_string(),
        "knowledge_query" => "KnowledgeQuery".to_string(),
        "knowledge_read" => "KnowledgeRead".to_string(),
        "knowledge_create" => "KnowledgeCreate".to_string(),
        "knowledge_delete" => "KnowledgeDelete".to_string(),
        "knowledge_move" => "KnowledgeMove".to_string(),
        "knowledge_edit" => "KnowledgeEdit".to_string(),
        "todowrite" => "TodoWrite".to_string(),
        "unity_asset_search" => "UnityAssetSearch".to_string(),
        "unity_execute" => "UnityExecute".to_string(),
        "unity_run_states" => "UnityRunStates".to_string(),
        "unity_recompile" => "UnityRecompile".to_string(),
        "unity_ref_search" => "UnityRefSearch".to_string(),
        "unity_yaml_read" => "UnityYamlRead".to_string(),
        "webfetch" => "WebFetch".to_string(),
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

fn build_text_blocks(text: &str) -> Vec<serde_json::Value> {
    const START: &str = "<system-reminder>";
    const END: &str = "</system-reminder>";

    let mut blocks: Vec<serde_json::Value> = Vec::new();
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

fn push_text_block(blocks: &mut Vec<serde_json::Value>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    blocks.push(serde_json::json!({
        "type": "text",
        "text": text,
    }));
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

fn resolve_native_beta_flags(extra_beta_flags: &[String]) -> String {
    if extra_beta_flags.is_empty() {
        BETA_FLAGS.to_string()
    } else {
        extra_beta_flags.join(",")
    }
}

fn build_claude_code_headers(
    bearer_token: &str,
    x_api_key: Option<&str>,
    session_id: &str,
    client_request_id: &str,
    beta_flags: &str,
    retry_count: u32,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Accept".to_string(), "application/json".to_string()),
        (
            "Authorization".to_string(),
            format!("Bearer {}", bearer_token),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("anthropic-beta".to_string(), beta_flags.to_string()),
        (
            "anthropic-dangerous-direct-browser-access".to_string(),
            "true".to_string(),
        ),
        ("anthropic-version".to_string(), API_VERSION.to_string()),
        ("User-Agent".to_string(), USER_AGENT.to_string()),
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
}

#[derive(Debug, Deserialize)]
struct MessageStartEvent {
    message: MessageStartMessage,
}

#[derive(Debug, Deserialize)]
struct MessageStartMessage {
    usage: Option<UsageInfo>,
}

#[derive(Debug, Deserialize)]
struct UsageInfo {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
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
mod tests {
    use super::{
        apply_cache_control, build_anthropic_messages, build_oauth_system_blocks,
        build_text_blocks, convert_tools_to_oauth_sdk_like_anthropic,
        rewrite_oauth_tool_use_blocks, CACHE_TTL,
    };
    use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};
    use serde_json::json;

    #[test]
    fn splits_system_reminder_text_into_separate_blocks() {
        let blocks = build_text_blocks(
            "<system-reminder>\nalpha\n</system-reminder>\n\nreply with exactly: ok",
        );

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(
            blocks[0]["text"],
            json!("<system-reminder>\nalpha\n</system-reminder>\n\n")
        );
        assert_eq!(blocks[1]["text"], json!("reply with exactly: ok"));
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
            tool_calls: None,
            tool_call_id: None,
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }];

        let mut messages = build_anthropic_messages(&history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"].as_array().map(|v| v.len()), Some(2));

        apply_cache_control(&mut messages);

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
                        "oldString": { "type": "string" },
                        "newString": { "type": "string" },
                        "replaceAll": { "type": "boolean" }
                    },
                    "required": ["filePath"]
                }
            }
        })];

        let (tools, aliases) = convert_tools_to_oauth_sdk_like_anthropic(&api_tools);

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("Edit"));
        assert_eq!(tools[0]["input_schema"]["required"], json!(["file_path"]));
        assert!(tools[0]["input_schema"]["properties"]
            .get("file_path")
            .is_some());
        assert!(tools[0]["input_schema"]["properties"]
            .get("old_string")
            .is_some());

        let public_input = aliases.public_input_for(
            "edit",
            json!({
                "filePath": "a.txt",
                "oldString": "foo",
                "newString": "bar",
                "replaceAll": true
            }),
        );
        assert_eq!(
            public_input,
            json!({
                "file_path": "a.txt",
                "old_string": "foo",
                "new_string": "bar",
                "replace_all": true
            })
        );

        let (internal_name, internal_args) = aliases.internalize_tool_call(
            "Edit",
            r#"{"file_path":"a.txt","old_string":"foo","new_string":"bar","replace_all":true}"#,
        );
        assert_eq!(internal_name, "edit");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&internal_args).unwrap(),
            json!({
                "filePath": "a.txt",
                "oldString": "foo",
                "newString": "bar",
                "replaceAll": true
            })
        );
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
                tool_calls: None,
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            },
            ChatMessage {
                id: "m2".to_string(),
                role: MessageRole::Assistant,
                content: "hi".to_string(),
                created_at: 1,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                tool_calls: None,
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            },
            ChatMessage {
                id: "m3".to_string(),
                role: MessageRole::User,
                content: "show tools".to_string(),
                created_at: 2,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                tool_calls: None,
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            },
        ];

        let mut messages = build_anthropic_messages(&history);
        apply_cache_control(&mut messages);

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
            tool_calls: Some(vec![ToolCallInfo {
                id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: r#"{"filePath":"Assets/Test.txt","limit":10}"#.to_string(),
                server_tool: None,
                server_tool_output: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }]),
            tool_call_id: None,
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
        }];

        let mut messages = build_anthropic_messages(&history);
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
                tool_calls: Some(vec![
                    ToolCallInfo {
                        id: "call_1".to_string(),
                        name: "read".to_string(),
                        arguments: r#"{"filePath":"Assets/Test.txt"}"#.to_string(),
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "call_2".to_string(),
                        name: "grep".to_string(),
                        arguments: r#"{"pattern":"Test","path":"Assets"}"#.to_string(),
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ]),
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            },
            ChatMessage {
                id: "user-1".to_string(),
                role: MessageRole::User,
                content: "继续".to_string(),
                created_at: 1,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                tool_calls: None,
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            },
        ];

        let messages = build_anthropic_messages(&history);

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
}
