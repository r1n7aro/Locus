use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCallInfo>,
    pub finish_reason: String,
    pub response_id: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub cost_usd: f64,
    pub raw_request: String,
    pub raw_response: String,
    pub thinking_text: String,
    pub thinking_duration_secs: u32,
    pub thinking_signature: String,
    pub continuation_request: Option<serde_json::Value>,
}

const DEFAULT_BASE: &str = "https://openrouter.ai";

pub async fn stream_chat<F, H>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: Option<&str>,
    api_path: Option<&str>,
    provider_tag: Option<&str>,
    extra_headers: &[(&str, &str)],
    reasoning_effort: Option<&str>,
    debug: bool,
    on_text_delta: F,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let tag = provider_tag.unwrap_or("OpenRouter");
    let client = reqwest::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let messages = build_api_messages(system_prompt, history);

    let body = build_request_body(model, messages, tools, reasoning_effort);

    let raw_request = serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    eprintln!(
        "[{}] POST model={} messages={} tools={}",
        tag,
        model,
        history.len(),
        tools.len()
    );

    let effective_base = base_url.unwrap_or(DEFAULT_BASE);
    let effective_path = api_path.unwrap_or("/api/v1/chat/completions");
    let api_url = format!("{}{}", effective_base.trim_end_matches('/'), effective_path);

    if debug {
        eprintln!("[DEBUG][{}] request body:\n{}", tag, &raw_request);
        let mut headers: Vec<(&str, &str)> = vec![
            ("Authorization", "Bearer <token>"),
            ("Content-Type", "application/json"),
            ("HTTP-Referer", "https://locus.app"),
            ("X-Title", "Locus"),
        ];
        for (k, v) in extra_headers {
            headers.push((*k, *v));
        }
        super::debug::save_request(tag, &api_url, &headers, &raw_request);
    }

    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut last_error = String::new();
    let mut response = None;

    for attempt in 0..=MAX_RETRIES {
        let mut req = client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://locus.app")
            .header("X-Title", "Locus")
            .json(&body);
        for (k, v) in extra_headers {
            req = req.header(*k, *v);
        }
        match req.send().await {
            Ok(resp) => {
                response = Some(resp);
                break;
            }
            Err(e) => {
                let mut error_chain = format!("Request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    error_chain.push_str(&format!("\n  caused by: {}", cause));
                    source = std::error::Error::source(cause);
                }
                let mut hints = Vec::new();
                if e.is_connect() {
                    hints.push("connection_error");
                }
                if e.is_timeout() {
                    hints.push("timeout");
                }
                if e.is_builder() {
                    hints.push("request_builder_error");
                }
                if e.is_redirect() {
                    hints.push("redirect_error");
                }
                if !hints.is_empty() {
                    error_chain.push_str(&format!("\n  error_type: {}", hints.join(", ")));
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

    let response = response.ok_or(last_error)?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        if debug {
            eprintln!(
                "[DEBUG][{}] API error (status={}):\n{}",
                tag, status, error_body
            );
        }
        return Err(format!("{} API error ({}): {}", tag, status, error_body));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_text = String::new();
    let mut raw_response = String::new();
    let mut tool_calls_map: std::collections::HashMap<i64, PartialToolCall> =
        std::collections::HashMap::new();
    let mut finish_reason = String::from("stop");
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut cache_read_tokens: u32 = 0;
    let mut cache_write_tokens: u32 = 0;
    let mut cost_usd: f64 = 0.0;

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
                let mut error_chain = format!("Stream read error: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    error_chain.push_str(&format!("\n  caused by: {}", cause));
                    source = std::error::Error::source(cause);
                }
                eprintln!(
                    "[{}] {} ({}/{})",
                    tag, error_chain, consecutive_errors, MAX_STREAM_ERRORS
                );
                if consecutive_errors >= MAX_STREAM_ERRORS {
                    return Err(error_chain);
                }
                continue;
            }
        };
        let chunk_text = String::from_utf8_lossy(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);

        while let Some(pos) = buffer.find("\n\n") {
            let event_text = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            for line in event_text.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        return finalize_stream_response(
                            tag,
                            debug,
                            true,
                            full_text,
                            &tool_calls_map,
                            finish_reason,
                            input_tokens,
                            output_tokens,
                            cache_read_tokens,
                            cache_write_tokens,
                            cost_usd,
                            raw_request,
                            raw_response,
                        );
                    }

                    if debug {
                        eprintln!("[DEBUG][{}] chunk: {}", tag, data);
                    }

                    match serde_json::from_str::<StreamChunk>(data) {
                        Ok(chunk) => {
                            if let Some(choice) = chunk.choices.first() {
                                if let Some(ref content) = choice.delta.content {
                                    full_text.push_str(content);
                                    on_text_delta(content.clone());
                                }
                                if let Some(ref tcs) = choice.delta.tool_calls {
                                    for tc in tcs {
                                        apply_tool_call_delta(
                                            &mut tool_calls_map,
                                            tc,
                                            &on_tool_call_start,
                                        );
                                    }
                                }
                                if let Some(ref reason) = choice.finish_reason {
                                    finish_reason = reason.clone();
                                }
                            }
                            if let Some(ref usage) = chunk.usage {
                                let usage_totals = usage_totals(usage);
                                input_tokens = usage_totals.input_tokens;
                                output_tokens = usage_totals.output_tokens;
                                cache_read_tokens = usage_totals.cache_read_tokens;
                                cache_write_tokens = usage_totals.cache_write_tokens;
                                cost_usd = usage_totals.cost_usd.unwrap_or(cost_usd);
                            }
                        }
                        Err(e) => {
                            eprintln!("[{}] Failed to parse chunk: {} | data: {}", tag, e, data);
                        }
                    }
                }
            }
        }
    }

    if !buffer.is_empty() {
        let remaining = if buffer.ends_with('\n') {
            buffer.clone()
        } else {
            format!("{}\n", buffer)
        };
        for line in remaining.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data == "[DONE]" {
                    return finalize_stream_response(
                        tag,
                        debug,
                        true,
                        full_text,
                        &tool_calls_map,
                        finish_reason,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        cost_usd,
                        raw_request,
                        raw_response,
                    );
                }
                if debug {
                    eprintln!("[DEBUG][{}] residual chunk: {}", tag, data);
                }
                if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                    if let Some(choice) = chunk.choices.first() {
                        if let Some(ref content) = choice.delta.content {
                            full_text.push_str(content);
                            on_text_delta(content.clone());
                        }
                        if let Some(ref tcs) = choice.delta.tool_calls {
                            for tc in tcs {
                                apply_tool_call_delta(&mut tool_calls_map, tc, &on_tool_call_start);
                            }
                        }
                        if let Some(ref reason) = choice.finish_reason {
                            finish_reason = reason.clone();
                        }
                    }
                    if let Some(ref usage) = chunk.usage {
                        let usage_totals = usage_totals(usage);
                        input_tokens = usage_totals.input_tokens;
                        output_tokens = usage_totals.output_tokens;
                        cache_read_tokens = usage_totals.cache_read_tokens;
                        cache_write_tokens = usage_totals.cache_write_tokens;
                        cost_usd = usage_totals.cost_usd.unwrap_or(cost_usd);
                    }
                }
            }
        }
    }

    finalize_stream_response(
        tag,
        debug,
        false,
        full_text,
        &tool_calls_map,
        finish_reason,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd,
        raw_request,
        raw_response,
    )
}

fn finalize_stream_response(
    tag: &str,
    debug: bool,
    got_terminal_event: bool,
    full_text: String,
    tool_calls_map: &std::collections::HashMap<i64, PartialToolCall>,
    mut finish_reason: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost_usd: f64,
    raw_request: String,
    raw_response: String,
) -> Result<LlmResponse, String> {
    if !got_terminal_event {
        if !full_text.is_empty() || !tool_calls_map.is_empty() || !raw_response.is_empty() {
            return Err(format!(
                "Stream ended without [DONE] (incomplete response, text_len={}, tool_calls={}). Refusing to return partial tool calls.",
                full_text.len(),
                tool_calls_map.len()
            ));
        }
        return Err("Stream ended with no data and no [DONE]".to_string());
    }

    let tool_calls = collect_tool_calls(tool_calls_map)?;
    if !tool_calls.is_empty() {
        finish_reason = "tool_calls".to_string();
    }

    let resp = LlmResponse {
        text: full_text,
        tool_calls,
        finish_reason,
        response_id: None,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd,
        raw_request,
        raw_response,
        thinking_text: String::new(),
        thinking_duration_secs: 0,
        thinking_signature: String::new(),
        continuation_request: None,
    };

    if debug {
        eprintln!(
            "[DEBUG][{}] response complete: finish_reason={}, text_len={}, tool_calls={}",
            tag,
            resp.finish_reason,
            resp.text.len(),
            resp.tool_calls.len()
        );
    }

    Ok(resp)
}

fn build_request_body(
    model: &str,
    messages: Vec<serde_json::Value>,
    tools: &[serde_json::Value],
    reasoning_effort: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        // OpenAI-compatible Chat Completions streams only include the final
        // usage chunk when include_usage is explicitly enabled.
        "stream_options": { "include_usage": true },
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::json!(tools);
    }

    if let Some(effort) = reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["reasoning_effort"] = serde_json::json!(effort);
    }

    body
}

fn build_api_messages(system_prompt: &str, history: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": [{
            "type": "text",
            "text": system_prompt,
            "cache_control": { "type": "ephemeral" }
        }],
    })];

    for msg in history {
        match msg.role {
            MessageRole::User => {
                if let Some(ref images) = msg.images {
                    if !images.is_empty() {
                        let mut blocks: Vec<serde_json::Value> = Vec::new();
                        for img in images {
                            blocks.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", img.mime_type, img.data),
                                }
                            }));
                        }
                        if !msg.content.is_empty() {
                            blocks.push(serde_json::json!({
                                "type": "text",
                                "text": msg.content,
                            }));
                        }
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": blocks,
                        }));
                        continue;
                    }
                }
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": msg.content,
                }));
            }
            MessageRole::Assistant => {
                let mut m = serde_json::json!({
                    "role": "assistant",
                });
                if !msg.content.is_empty() {
                    m["content"] = serde_json::json!(msg.content);
                }
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
                        let tcs: Vec<serde_json::Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments,
                                    }
                                })
                            })
                            .collect();
                        m["tool_calls"] = serde_json::json!(tcs);
                    }
                }
                messages.push(m);
            }
            MessageRole::Tool => {
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "content": msg.content,
                }));
            }
        }
    }

    apply_cache_control_openrouter(&mut messages);
    messages
}

fn apply_cache_control_openrouter(messages: &mut Vec<serde_json::Value>) {
    let cacheable_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            !matches!(
                m.get("role").and_then(|r| r.as_str()),
                Some("system" | "tool")
            )
        })
        .map(|(i, _)| i)
        .collect();

    let len = cacheable_indices.len();
    if len == 0 {
        return;
    }

    let start = if len >= 2 { len - 2 } else { 0 };
    for &idx in &cacheable_indices[start..] {
        let msg = &mut messages[idx];
        if let Some(text) = msg
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
        {
            msg["content"] = serde_json::json!([{
                "type": "text",
                "text": text,
                "cache_control": { "type": "ephemeral" }
            }]);
        } else if let Some(content_arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(last_block) = content_arr.last_mut() {
                last_block["cache_control"] = serde_json::json!({ "type": "ephemeral" });
            }
        }
    }
}

fn collect_tool_calls(
    map: &std::collections::HashMap<i64, PartialToolCall>,
) -> Result<Vec<ToolCallInfo>, String> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(idx, _)| *idx);
    let mut tool_calls = Vec::with_capacity(entries.len());

    for (idx, tc) in entries {
        if !tc.has_complete_metadata() {
            return Err(format!(
                "Refusing to execute incomplete tool call at index {}: missing {}",
                idx,
                missing_tool_call_metadata(tc)
            ));
        }
        validate_tool_call_arguments(&tc.name, &tc.arguments)?;
        tool_calls.push(ToolCallInfo {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        });
    }

    Ok(tool_calls)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UsageTotals {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost_usd: Option<f64>,
}

fn usage_totals(usage: &StreamUsage) -> UsageTotals {
    let prompt_details = usage.prompt_tokens_details.as_ref();
    let cache_read_tokens = prompt_details.map(|d| d.cached_tokens).unwrap_or(0);
    let cache_write_tokens = usage
        .cache_write_tokens
        .or_else(|| prompt_details.and_then(|d| d.cache_write_tokens))
        .unwrap_or(0);

    UsageTotals {
        input_tokens: usage
            .prompt_tokens
            .saturating_sub(cache_read_tokens)
            .saturating_sub(cache_write_tokens),
        output_tokens: usage.completion_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd: usage.cost,
    }
}

#[derive(Debug)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
    notified: bool,
}

impl PartialToolCall {
    fn has_complete_metadata(&self) -> bool {
        !self.id.trim().is_empty() && !self.name.trim().is_empty()
    }
}

fn apply_tool_call_delta<H>(
    tool_calls_map: &mut std::collections::HashMap<i64, PartialToolCall>,
    tc: &StreamToolCallDelta,
    on_tool_call_start: &H,
) where
    H: Fn(String, String) + Send + 'static,
{
    let entry = tool_calls_map
        .entry(tc.index)
        .or_insert_with(|| PartialToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
            notified: false,
        });
    if let Some(ref id) = tc.id {
        assign_non_empty(&mut entry.id, id);
    }
    if let Some(ref func) = tc.function {
        if let Some(ref name) = func.name {
            assign_non_empty(&mut entry.name, name);
        }
        if let Some(ref args) = func.arguments {
            entry.arguments.push_str(args);
        }
    }
    if !entry.notified && !entry.id.is_empty() && !entry.name.is_empty() {
        on_tool_call_start(entry.id.clone(), entry.name.clone());
        entry.notified = true;
    }
}

fn assign_non_empty(target: &mut String, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        *target = trimmed.to_string();
    }
}

fn missing_tool_call_metadata(tc: &PartialToolCall) -> String {
    let mut missing = Vec::new();
    if tc.id.trim().is_empty() {
        missing.push("id");
    }
    if tc.name.trim().is_empty() {
        missing.push("name");
    }
    missing.join(", ")
}

fn validate_tool_call_arguments(tool_name: &str, arguments: &str) -> Result<(), String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "Refusing to execute malformed tool arguments for '{}': {}",
                tool_name, error
            )
        })
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    cache_write_tokens: Option<u32>,
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
    #[serde(default)]
    cache_write_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    index: i64,
    id: Option<String>,
    function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<serde_json::Value>,
    stream: bool,
    stream_options: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::{
        apply_tool_call_delta, build_api_messages, build_request_body, collect_tool_calls,
        finalize_stream_response, usage_totals, PartialToolCall, StreamToolCallDelta, StreamUsage,
    };
    use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn chat_message(
        role: MessageRole,
        content: &str,
        tool_calls: Option<Vec<ToolCallInfo>>,
        tool_call_id: Option<&str>,
    ) -> ChatMessage {
        ChatMessage {
            id: "msg".to_string(),
            role,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls,
            tool_call_id: tool_call_id.map(str::to_string),
            images: None,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    fn complete_tool_call(id: &str, name: &str, arguments: &str) -> PartialToolCall {
        PartialToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            notified: false,
        }
    }

    #[test]
    fn chat_completion_requests_enable_stream_usage() {
        let body = build_request_body("model-x", Vec::new(), &[], None);

        assert_eq!(body["stream"], serde_json::json!(true));
        assert_eq!(
            body["stream_options"],
            serde_json::json!({ "include_usage": true })
        );
    }

    #[test]
    fn chat_completion_requests_include_custom_reasoning_effort() {
        let body = build_request_body("model-x", Vec::new(), &[], Some("max"));

        assert_eq!(body["reasoning_effort"], serde_json::json!("max"));
    }

    #[test]
    fn usage_totals_split_uncached_read_and_write_tokens() {
        let usage: StreamUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 120,
            "completion_tokens": 30,
            "prompt_tokens_details": {
                "cached_tokens": 40,
                "cache_write_tokens": 10
            },
            "cost": 0.12
        }))
        .expect("usage should deserialize");

        let totals = usage_totals(&usage);
        assert_eq!(totals.input_tokens, 70);
        assert_eq!(totals.cache_read_tokens, 40);
        assert_eq!(totals.cache_write_tokens, 10);
        assert_eq!(totals.output_tokens, 30);
        assert_eq!(totals.cost_usd, Some(0.12));
    }

    #[test]
    fn top_level_cache_write_tokens_take_precedence() {
        let usage: StreamUsage = serde_json::from_value(serde_json::json!({
            "prompt_tokens": 120,
            "completion_tokens": 30,
            "prompt_tokens_details": {
                "cached_tokens": 40,
                "cache_write_tokens": 10
            },
            "cache_write_tokens": 15
        }))
        .expect("usage should deserialize");

        let totals = usage_totals(&usage);
        assert_eq!(totals.input_tokens, 65);
        assert_eq!(totals.cache_write_tokens, 15);
    }

    #[test]
    fn cache_control_marks_system_and_last_two_cacheable_messages() {
        let messages = build_api_messages(
            "system prompt",
            &[
                chat_message(MessageRole::User, "first user", None, None),
                chat_message(MessageRole::Assistant, "assistant text", None, None),
                chat_message(MessageRole::Tool, "tool output", None, Some("tool_1")),
                chat_message(MessageRole::User, "last user", None, None),
            ],
        );

        assert_eq!(
            messages[0]["content"][0]["cache_control"],
            serde_json::json!({ "type": "ephemeral" })
        );
        assert!(messages[1]["content"][0].get("cache_control").is_none());
        assert_eq!(
            messages[2]["content"][0]["cache_control"],
            serde_json::json!({ "type": "ephemeral" })
        );
        assert_eq!(messages[3]["role"], serde_json::json!("tool"));
        assert!(messages[3]
            .get("content")
            .and_then(|v| v.as_str())
            .is_some());
        assert_eq!(
            messages[4]["content"][0]["cache_control"],
            serde_json::json!({ "type": "ephemeral" })
        );
    }

    #[test]
    fn finalize_stream_response_rejects_partial_tool_calls_before_done() {
        let mut tool_calls = HashMap::new();
        tool_calls.insert(
            0,
            complete_tool_call("call_partial", "write_file", r#"{"path":"#),
        );

        let err = finalize_stream_response(
            "OpenRouter",
            false,
            false,
            String::from("hello"),
            &tool_calls,
            String::from("stop"),
            0,
            0,
            0,
            0,
            0.0,
            String::from("request"),
            String::from("partial response"),
        )
        .expect_err("partial tool calls should be rejected");

        assert!(
            err.contains("Refusing to return partial tool calls"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn finalize_stream_response_allows_text_only_and_completed_tool_calls() {
        let text_only = finalize_stream_response(
            "OpenRouter",
            false,
            true,
            String::from("plain text"),
            &HashMap::new(),
            String::from("stop"),
            1,
            2,
            3,
            4,
            0.5,
            String::from("request"),
            String::from("response"),
        )
        .expect("text-only response should complete");
        assert_eq!(text_only.text, "plain text");
        assert!(text_only.tool_calls.is_empty());
        assert_eq!(text_only.finish_reason, "stop");
        assert_eq!(text_only.input_tokens, 1);
        assert_eq!(text_only.output_tokens, 2);

        let mut tool_calls = HashMap::new();
        tool_calls.insert(
            0,
            complete_tool_call("call_1", "write_file", r#"{"path":"A"}"#),
        );

        let tool_response = finalize_stream_response(
            "OpenRouter",
            false,
            true,
            String::new(),
            &tool_calls,
            String::from("stop"),
            0,
            0,
            0,
            0,
            0.0,
            String::from("request"),
            String::from("response"),
        )
        .expect("completed tool call response should complete");
        assert_eq!(tool_response.tool_calls.len(), 1);
        assert_eq!(tool_response.tool_calls[0].id, "call_1");
        assert_eq!(tool_response.tool_calls[0].name, "write_file");
        assert_eq!(tool_response.finish_reason, "tool_calls");
    }

    #[test]
    fn tool_call_delta_preserves_existing_name_and_notifies_once() {
        let mut tool_calls = HashMap::new();
        let started = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let captured = started.clone();
        let on_tool = move |id: String, name: String| {
            captured
                .lock()
                .expect("tool mutex poisoned")
                .push((id, name));
        };

        let start: StreamToolCallDelta = serde_json::from_value(serde_json::json!({
            "index": 0,
            "id": "call_1",
            "function": { "name": "list", "arguments": "" }
        }))
        .expect("start delta should parse");
        apply_tool_call_delta(&mut tool_calls, &start, &on_tool);

        let args: StreamToolCallDelta = serde_json::from_value(serde_json::json!({
            "index": 0,
            "function": { "name": "", "arguments": "{\"path\":\"Assets\"}" }
        }))
        .expect("arguments delta should parse");
        apply_tool_call_delta(&mut tool_calls, &args, &on_tool);

        let collected = collect_tool_calls(&tool_calls).expect("tool call should be valid");
        assert_eq!(collected[0].name, "list");
        assert_eq!(
            started.lock().expect("tool mutex poisoned").as_slice(),
            &[("call_1".to_string(), "list".to_string())]
        );
    }

    #[test]
    fn collect_tool_calls_rejects_empty_name() {
        let mut tool_calls = HashMap::new();
        tool_calls.insert(0, complete_tool_call("call_1", "", "{}"));

        let err = collect_tool_calls(&tool_calls).expect_err("empty name should be rejected");
        assert!(err.contains("missing name"), "unexpected error: {}", err);
    }
}
