use futures::StreamExt;
use serde::Deserialize;
use std::time::Instant;

use super::openrouter::LlmResponse;
use crate::session::models::{ChatMessage, ImageData, MessageRole, ToolCallInfo};

const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatCompletionsFlavor {
    Generic,
    DeepSeek,
    MiniMax,
}

pub async fn stream_chat<F, G, H>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: &str,
    reasoning_effort: Option<&str>,
    thinking_level: Option<&str>,
    replay_reasoning_content: bool,
    debug: bool,
    on_text_delta: F,
    on_thinking_delta: G,
    on_tool_call_start: H,
) -> Result<LlmResponse, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let tag = "Custom Chat";
    let flavor = detect_flavor(model, base_url);
    let client = reqwest::Client::builder()
        .tcp_keepalive(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let messages = build_api_messages(
        system_prompt,
        history,
        flavor,
        deepseek_thinking_mode_enabled(thinking_level),
        replay_reasoning_content,
    );
    let body = build_request_body(
        model,
        messages,
        tools,
        reasoning_effort,
        thinking_level,
        flavor,
    );
    let raw_request = serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    eprintln!(
        "[{}] POST model={} messages={} tools={} flavor={:?} replay_reasoning_content={}",
        tag,
        model,
        history.len(),
        tools.len(),
        flavor,
        replay_reasoning_content
    );

    let api_url = format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        CHAT_COMPLETIONS_PATH
    );

    if debug {
        eprintln!("[DEBUG][{}] request body:\n{}", tag, &raw_request);
        let mut headers: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
        if !api_key.is_empty() {
            headers.push(("Authorization", "Bearer <token>"));
        }
        super::debug::save_request("custom_chat_completions", &api_url, &headers, &raw_request);
    }

    const MAX_RETRIES: u32 = 3;
    const BASE_DELAY_MS: u64 = 1000;

    let mut last_error = String::new();
    let mut response = None;

    for attempt in 0..=MAX_RETRIES {
        let mut req = client
            .post(&api_url)
            .header("Content-Type", "application/json")
            .json(&body);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
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
    let mut raw_response = String::new();
    let mut state = ChatStreamState::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                return Err(format!("Stream read error: {}", e));
            }
        };

        let chunk_text = String::from_utf8_lossy(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);

        if drain_sse_buffer(
            &mut buffer,
            false,
            debug,
            &mut state,
            &on_text_delta,
            &on_thinking_delta,
            &on_tool_call_start,
        )? {
            return finalize_stream_response(tag, debug, true, state, raw_request, raw_response);
        }
    }

    if drain_sse_buffer(
        &mut buffer,
        true,
        debug,
        &mut state,
        &on_text_delta,
        &on_thinking_delta,
        &on_tool_call_start,
    )? {
        return finalize_stream_response(tag, debug, true, state, raw_request, raw_response);
    }

    let saw_finish_reason = state.saw_finish_reason;
    finalize_stream_response(
        tag,
        debug,
        saw_finish_reason,
        state,
        raw_request,
        raw_response,
    )
}

fn detect_flavor(model: &str, base_url: &str) -> ChatCompletionsFlavor {
    let model = model.trim().to_ascii_lowercase();
    let base_url = base_url.trim().to_ascii_lowercase();
    if model.starts_with("deepseek-") || base_url.contains("deepseek") {
        ChatCompletionsFlavor::DeepSeek
    } else if model.contains("minimax")
        || base_url.contains("minimax")
        || base_url.contains("minimaxi")
    {
        ChatCompletionsFlavor::MiniMax
    } else {
        ChatCompletionsFlavor::Generic
    }
}

fn build_request_body(
    model: &str,
    messages: Vec<serde_json::Value>,
    tools: &[serde_json::Value],
    reasoning_effort: Option<&str>,
    thinking_level: Option<&str>,
    flavor: ChatCompletionsFlavor,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::json!(tools);
    }

    match flavor {
        ChatCompletionsFlavor::DeepSeek => {
            apply_deepseek_thinking_params(&mut body, reasoning_effort, thinking_level);
        }
        ChatCompletionsFlavor::MiniMax => {
            body["reasoning_split"] = serde_json::json!(true);
            apply_generic_reasoning_effort(&mut body, reasoning_effort);
        }
        ChatCompletionsFlavor::Generic => {
            apply_generic_reasoning_effort(&mut body, reasoning_effort);
        }
    }

    body
}

fn apply_generic_reasoning_effort(body: &mut serde_json::Value, reasoning_effort: Option<&str>) {
    if let Some(effort) = reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"))
    {
        body["reasoning_effort"] = serde_json::json!(effort);
    }
}

fn apply_deepseek_thinking_params(
    body: &mut serde_json::Value,
    reasoning_effort: Option<&str>,
    thinking_level: Option<&str>,
) {
    if thinking_level
        .map(str::trim)
        .is_some_and(|level| level.eq_ignore_ascii_case("none"))
    {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
        return;
    }

    let Some(effort) = reasoning_effort.and_then(normalize_deepseek_reasoning_effort) else {
        return;
    };
    body["thinking"] = serde_json::json!({ "type": "enabled" });
    body["reasoning_effort"] = serde_json::json!(effort);
}

fn normalize_deepseek_reasoning_effort(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" => Some("high"),
        "xhigh" | "max" => Some("max"),
        _ => None,
    }
}

fn deepseek_thinking_mode_enabled(thinking_level: Option<&str>) -> bool {
    !thinking_level
        .map(str::trim)
        .is_some_and(|level| level.eq_ignore_ascii_case("none"))
}

fn build_api_messages(
    system_prompt: &str,
    history: &[ChatMessage],
    flavor: ChatCompletionsFlavor,
    deepseek_thinking_enabled: bool,
    replay_reasoning_content: bool,
) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }));
    }

    let mut i = 0usize;
    while i < history.len() {
        let msg = &history[i];
        match msg.role {
            MessageRole::User => {
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": build_user_content(msg, flavor),
                }));
            }
            MessageRole::Assistant => {
                if should_flatten_deepseek_tool_round(msg, flavor, deepseek_thinking_enabled) {
                    let (content, consumed) = render_legacy_tool_round(history, i);
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    }));
                    i += consumed;
                    continue;
                }

                let mut m = serde_json::json!({
                    "role": "assistant",
                    "content": msg.content,
                });
                if replay_reasoning_content {
                    if let Some(thinking) = msg
                        .thinking_content
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        if flavor == ChatCompletionsFlavor::MiniMax {
                            m["reasoning_details"] = serde_json::json!([{
                                "type": "reasoning.text",
                                "id": "reasoning-text-1",
                                "format": "MiniMax-response-v1",
                                "index": 0,
                                "text": thinking,
                            }]);
                        } else {
                            m["reasoning_content"] = serde_json::json!(thinking);
                        }
                    }
                }
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
                        m["tool_calls"] = serde_json::json!(build_tool_call_values(tool_calls));
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
        i += 1;
    }

    messages
}

fn should_flatten_deepseek_tool_round(
    msg: &ChatMessage,
    flavor: ChatCompletionsFlavor,
    deepseek_thinking_enabled: bool,
) -> bool {
    if flavor != ChatCompletionsFlavor::DeepSeek || !deepseek_thinking_enabled {
        return false;
    }
    if msg.role != MessageRole::Assistant {
        return false;
    }
    let has_tool_calls = msg
        .tool_calls
        .as_ref()
        .is_some_and(|tool_calls| !tool_calls.is_empty());
    if !has_tool_calls {
        return false;
    }
    msg.thinking_content
        .as_deref()
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
}

fn render_legacy_tool_round(history: &[ChatMessage], assistant_index: usize) -> (String, usize) {
    let assistant = &history[assistant_index];
    let mut content = String::new();
    if !assistant.content.trim().is_empty() {
        content.push_str(&assistant.content);
        content.push_str("\n\n");
    }
    content.push_str("[Earlier tool call transcript]");

    let mut consumed = 1usize;
    let following_tools =
        collect_following_tool_outputs(history, assistant_index + 1, &mut consumed);
    if let Some(tool_calls) = assistant.tool_calls.as_ref() {
        for tool_call in tool_calls {
            content.push_str("\n\nTool call ");
            content.push_str(&tool_call.name);
            if !tool_call.id.is_empty() {
                content.push_str(" (");
                content.push_str(&tool_call.id);
                content.push(')');
            }
            if !tool_call.arguments.trim().is_empty() {
                content.push_str("\nArguments:\n");
                content.push_str(&tool_call.arguments);
            }
            let output = tool_call
                .server_tool_output
                .as_deref()
                .or_else(|| {
                    following_tools
                        .get(tool_call.id.as_str())
                        .map(String::as_str)
                })
                .or(tool_call.recorded_output.as_deref());
            if let Some(output) = output.filter(|value| !value.is_empty()) {
                content.push_str("\nOutput:\n");
                content.push_str(output);
            }
        }
    }

    (content, consumed)
}

fn collect_following_tool_outputs(
    history: &[ChatMessage],
    start_index: usize,
    consumed: &mut usize,
) -> std::collections::HashMap<String, String> {
    let mut outputs = std::collections::HashMap::new();
    let mut index = start_index;
    while index < history.len() && history[index].role == MessageRole::Tool {
        if let Some(tool_call_id) = history[index]
            .tool_call_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            outputs
                .entry(tool_call_id.to_string())
                .or_insert_with(|| history[index].content.clone());
        }
        *consumed += 1;
        index += 1;
    }
    outputs
}

fn build_user_content(msg: &ChatMessage, flavor: ChatCompletionsFlavor) -> serde_json::Value {
    match flavor {
        ChatCompletionsFlavor::DeepSeek => {
            serde_json::Value::String(deepseek_text_content(&msg.content, msg.images.as_deref()))
        }
        ChatCompletionsFlavor::Generic | ChatCompletionsFlavor::MiniMax => {
            if let Some(images) = msg.images.as_deref().filter(|images| !images.is_empty()) {
                let mut blocks: Vec<serde_json::Value> = images
                    .iter()
                    .map(|img| {
                        serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", img.mime_type, img.data),
                            }
                        })
                    })
                    .collect();
                if !msg.content.is_empty() {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": msg.content,
                    }));
                }
                serde_json::Value::Array(blocks)
            } else {
                serde_json::Value::String(msg.content.clone())
            }
        }
    }
}

fn deepseek_text_content(text: &str, images: Option<&[ImageData]>) -> String {
    let Some(images) = images.filter(|images| !images.is_empty()) else {
        return text.to_string();
    };

    let note = format!(
        "[{} image attachment(s) omitted because this Chat Completions endpoint accepts text only.]",
        images.len()
    );
    if text.trim().is_empty() {
        note
    } else {
        format!("{}\n\n{}", text, note)
    }
}

fn build_tool_call_values(tool_calls: &[ToolCallInfo]) -> Vec<serde_json::Value> {
    tool_calls
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
        .collect()
}

struct ChatStreamState {
    full_text: String,
    thinking_text: String,
    thinking_started_at: Option<Instant>,
    thinking_duration_secs: u32,
    reasoning_detail_text_by_key: std::collections::HashMap<String, String>,
    tool_calls_map: std::collections::HashMap<i64, PartialToolCall>,
    finish_reason: String,
    saw_finish_reason: bool,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost_usd: f64,
}

impl ChatStreamState {
    fn new() -> Self {
        Self {
            full_text: String::new(),
            thinking_text: String::new(),
            thinking_started_at: None,
            thinking_duration_secs: 0,
            reasoning_detail_text_by_key: std::collections::HashMap::new(),
            tool_calls_map: std::collections::HashMap::new(),
            finish_reason: String::from("stop"),
            saw_finish_reason: false,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
        }
    }

    fn push_thinking<G>(&mut self, delta: &str, on_thinking_delta: &G)
    where
        G: Fn(String) + Send + 'static,
    {
        if delta.is_empty() {
            return;
        }
        if self.thinking_started_at.is_none() {
            self.thinking_started_at = Some(Instant::now());
        }
        self.thinking_text.push_str(delta);
        on_thinking_delta(delta.to_string());
    }

    fn push_reasoning_details<G>(
        &mut self,
        details: &[StreamReasoningDetail],
        on_thinking_delta: &G,
    ) where
        G: Fn(String) + Send + 'static,
    {
        for detail in details {
            let Some(text) = detail.text.as_deref().filter(|value| !value.is_empty()) else {
                continue;
            };
            let key = detail
                .id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("index:{}", detail.index.unwrap_or(0)));
            let previous = self
                .reasoning_detail_text_by_key
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let delta = if text.starts_with(&previous) {
                text[previous.len()..].to_string()
            } else {
                text.to_string()
            };
            if delta.is_empty() {
                self.reasoning_detail_text_by_key
                    .insert(key, text.to_string());
                continue;
            }
            let next_accumulated = if text.starts_with(&previous) {
                text.to_string()
            } else {
                format!("{}{}", previous, text)
            };
            self.push_thinking(&delta, on_thinking_delta);
            self.reasoning_detail_text_by_key
                .insert(key, next_accumulated);
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

fn next_sse_separator(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|pos| (pos, 2usize));
    let crlf = buffer.find("\r\n\r\n").map(|pos| (pos, 4usize));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn drain_sse_buffer<F, G, H>(
    buffer: &mut String,
    flush_final_block: bool,
    debug: bool,
    state: &mut ChatStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<bool, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    while let Some((pos, sep_len)) = next_sse_separator(buffer) {
        let event_block = buffer[..pos].to_string();
        *buffer = buffer[pos + sep_len..].to_string();
        if process_sse_event_block(
            &event_block,
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
        buffer.clear();
        if !trailing.is_empty()
            && process_sse_event_block(
                &trailing,
                debug,
                state,
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )?
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn process_sse_event_block<F, G, H>(
    event_block: &str,
    debug: bool,
    state: &mut ChatStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<bool, String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    for line in event_block.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(true);
        }
        if debug {
            eprintln!("[DEBUG][Custom Chat] chunk: {}", data);
        }
        match serde_json::from_str::<StreamChunk>(data) {
            Ok(chunk) => {
                apply_stream_chunk(
                    chunk,
                    state,
                    on_text_delta,
                    on_thinking_delta,
                    on_tool_call_start,
                );
            }
            Err(e) => {
                eprintln!(
                    "[Custom Chat] Failed to parse chunk: {} | data: {}",
                    e, data
                );
            }
        }
    }

    Ok(false)
}

fn apply_stream_chunk<F, G, H>(
    chunk: StreamChunk,
    state: &mut ChatStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    if let Some(choice) = chunk.choices.first() {
        if let Some(ref reasoning) = choice.delta.reasoning_content {
            state.push_thinking(reasoning, on_thinking_delta);
        }
        if let Some(ref reasoning_details) = choice.delta.reasoning_details {
            state.push_reasoning_details(reasoning_details, on_thinking_delta);
        }
        if let Some(ref content) = choice.delta.content {
            state.finish_thinking_timing();
            state.full_text.push_str(content);
            on_text_delta(content.clone());
        }
        if let Some(ref tcs) = choice.delta.tool_calls {
            for tc in tcs {
                let entry =
                    state
                        .tool_calls_map
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
        }
        if let Some(ref reason) = choice.finish_reason {
            let reason = reason.trim();
            if !reason.is_empty() {
                state.finish_reason = reason.to_string();
                state.saw_finish_reason = true;
            }
        }
    }
    if let Some(ref usage) = chunk.usage {
        let usage_totals = usage_totals(usage);
        state.input_tokens = usage_totals.input_tokens;
        state.output_tokens = usage_totals.output_tokens;
        state.cache_read_tokens = usage_totals.cache_read_tokens;
        state.cache_write_tokens = usage_totals.cache_write_tokens;
        state.cost_usd = usage_totals.cost_usd.unwrap_or(state.cost_usd);
    }
}

fn finalize_stream_response(
    tag: &str,
    debug: bool,
    got_terminal_event: bool,
    mut state: ChatStreamState,
    raw_request: String,
    raw_response: String,
) -> Result<LlmResponse, String> {
    state.finish_thinking_timing();

    if !got_terminal_event {
        if !state.full_text.is_empty()
            || !state.thinking_text.is_empty()
            || !state.tool_calls_map.is_empty()
            || !raw_response.is_empty()
        {
            return Err(format!(
                "Stream ended without [DONE] (incomplete response, text_len={}, thinking_len={}, tool_calls={}). Refusing to return partial output.",
                state.full_text.len(),
                state.thinking_text.len(),
                state.tool_calls_map.len()
            ));
        }
        return Err("Stream ended with no data and no [DONE]".to_string());
    }

    let tool_calls = collect_tool_calls(&state.tool_calls_map)?;
    if !tool_calls.is_empty() {
        state.finish_reason = "tool_calls".to_string();
    }

    let resp = LlmResponse {
        text: state.full_text,
        tool_calls,
        finish_reason: state.finish_reason,
        response_id: None,
        input_tokens: state.input_tokens,
        output_tokens: state.output_tokens,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
        cost_usd: state.cost_usd,
        raw_request,
        raw_response,
        thinking_text: state.thinking_text,
        thinking_duration_secs: state.thinking_duration_secs,
        thinking_signature: String::new(),
        continuation_request: None,
    };

    if debug {
        eprintln!(
            "[DEBUG][{}] response complete: finish_reason={}, text_len={}, thinking_len={}, tool_calls={}",
            tag,
            resp.finish_reason,
            resp.text.len(),
            resp.thinking_text.len(),
            resp.tool_calls.len()
        );
    }

    Ok(resp)
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

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
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
    reasoning_content: Option<String>,
    reasoning_details: Option<Vec<StreamReasoningDetail>>,
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamReasoningDetail {
    id: Option<String>,
    #[serde(default)]
    index: Option<i64>,
    text: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::{
        apply_stream_chunk, build_api_messages, build_request_body, collect_tool_calls,
        detect_flavor, drain_sse_buffer, finalize_stream_response, ChatCompletionsFlavor,
        ChatStreamState, PartialToolCall, StreamChunk,
    };
    use crate::session::models::{ChatMessage, ImageData, MessageRole, ToolCallInfo};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn ignore_text(_: String) {}
    fn ignore_thinking(_: String) {}
    fn ignore_tool(_: String, _: String) {}

    fn user_message(content: &str, images: Option<Vec<ImageData>>) -> ChatMessage {
        ChatMessage {
            id: "user".to_string(),
            role: MessageRole::User,
            content: content.to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: None,
            tool_call_id: None,
            images,
            asset_refs: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    fn assistant_message_with_tool_and_thinking() -> ChatMessage {
        ChatMessage {
            id: "assistant".to_string(),
            role: MessageRole::Assistant,
            content: "Need a tool.".to_string(),
            created_at: 0,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: Some(vec![ToolCallInfo {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: r#"{"path":"a.txt"}"#.to_string(),
                order: None,
                server_tool: None,
                server_tool_output: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }]),
            tool_call_id: None,
            images: None,
            asset_refs: None,
            thinking_content: Some("I should inspect the file.".to_string()),
            thinking_duration: Some(1),
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    fn tool_message(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: "tool".to_string(),
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

    #[test]
    fn deepseek_messages_use_text_content_and_replay_reasoning() {
        let messages = build_api_messages(
            "system",
            &[
                user_message(
                    "Describe this",
                    Some(vec![ImageData {
                        data: "YWJj".to_string(),
                        mime_type: "image/png".to_string(),
                    }]),
                ),
                assistant_message_with_tool_and_thinking(),
            ],
            ChatCompletionsFlavor::DeepSeek,
            true,
            true,
        );

        assert_eq!(messages[0]["content"], serde_json::json!("system"));
        assert_eq!(messages[1]["role"], serde_json::json!("user"));
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("Describe this"));
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("image attachment"));
        assert_eq!(
            messages[2]["reasoning_content"],
            serde_json::json!("I should inspect the file.")
        );
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            serde_json::json!("read_file")
        );
    }

    #[test]
    fn deepseek_replays_reasoning_content_without_trimming() {
        let mut assistant = assistant_message_with_tool_and_thinking();
        assistant.thinking_content = Some("\n  Keep exact reasoning.  \n".to_string());

        let messages = build_api_messages(
            "",
            &[assistant],
            ChatCompletionsFlavor::DeepSeek,
            true,
            true,
        );

        assert_eq!(
            messages[0]["reasoning_content"],
            serde_json::json!("\n  Keep exact reasoning.  \n")
        );
    }

    #[test]
    fn generic_messages_replay_reasoning_content_when_enabled() {
        let assistant = assistant_message_with_tool_and_thinking();

        let enabled = build_api_messages(
            "",
            &[assistant.clone()],
            ChatCompletionsFlavor::Generic,
            false,
            true,
        );
        assert_eq!(
            enabled[0]["reasoning_content"],
            serde_json::json!("I should inspect the file.")
        );

        let disabled = build_api_messages(
            "",
            &[assistant],
            ChatCompletionsFlavor::Generic,
            false,
            false,
        );
        assert!(disabled[0].get("reasoning_content").is_none());
    }

    #[test]
    fn minimax_messages_replay_reasoning_details_when_enabled() {
        let assistant = assistant_message_with_tool_and_thinking();

        let messages = build_api_messages(
            "",
            &[assistant],
            ChatCompletionsFlavor::MiniMax,
            false,
            true,
        );

        assert!(messages[0].get("reasoning_content").is_none());
        assert_eq!(
            messages[0]["reasoning_details"][0]["text"],
            serde_json::json!("I should inspect the file.")
        );
        assert_eq!(
            messages[0]["reasoning_details"][0]["format"],
            serde_json::json!("MiniMax-response-v1")
        );
    }

    #[test]
    fn deepseek_thinking_flattens_legacy_tool_round_without_reasoning() {
        let mut assistant = assistant_message_with_tool_and_thinking();
        assistant.thinking_content = None;
        let messages = build_api_messages(
            "",
            &[
                assistant,
                tool_message("call_1", "file contents"),
                user_message("Continue", None),
            ],
            ChatCompletionsFlavor::DeepSeek,
            true,
            true,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], serde_json::json!("assistant"));
        assert!(messages[0].get("tool_calls").is_none());
        assert!(messages[0].get("reasoning_content").is_none());
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("Tool call read_file"));
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("file contents"));
        assert_eq!(messages[1]["role"], serde_json::json!("user"));
    }

    #[test]
    fn deepseek_disabled_thinking_keeps_legacy_tool_protocol() {
        let mut assistant = assistant_message_with_tool_and_thinking();
        assistant.thinking_content = None;
        let messages = build_api_messages(
            "",
            &[assistant, tool_message("call_1", "file contents")],
            ChatCompletionsFlavor::DeepSeek,
            false,
            true,
        );

        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["name"],
            serde_json::json!("read_file")
        );
        assert_eq!(messages[1]["role"], serde_json::json!("tool"));
    }

    #[test]
    fn generic_messages_keep_image_url_blocks() {
        let messages = build_api_messages(
            "system",
            &[user_message(
                "Describe this",
                Some(vec![ImageData {
                    data: "YWJj".to_string(),
                    mime_type: "image/png".to_string(),
                }]),
            )],
            ChatCompletionsFlavor::Generic,
            false,
            false,
        );

        let blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], serde_json::json!("image_url"));
        assert_eq!(blocks[1]["type"], serde_json::json!("text"));
    }

    #[test]
    fn deepseek_request_body_controls_thinking_mode() {
        let enabled = build_request_body(
            "deepseek-v4-pro",
            Vec::new(),
            &[],
            Some("low"),
            Some("low"),
            ChatCompletionsFlavor::DeepSeek,
        );
        assert_eq!(
            enabled["thinking"],
            serde_json::json!({ "type": "enabled" })
        );
        assert_eq!(enabled["reasoning_effort"], serde_json::json!("high"));

        let disabled = build_request_body(
            "deepseek-v4-pro",
            Vec::new(),
            &[],
            None,
            Some("none"),
            ChatCompletionsFlavor::DeepSeek,
        );
        assert_eq!(
            disabled["thinking"],
            serde_json::json!({ "type": "disabled" })
        );
        assert!(disabled.get("reasoning_effort").is_none());
    }

    #[test]
    fn detects_minimax_flavor_by_model_or_endpoint() {
        assert_eq!(
            detect_flavor("MiniMax-M2.7", "https://example.com/v1"),
            ChatCompletionsFlavor::MiniMax
        );
        assert_eq!(
            detect_flavor("custom-model", "https://api.minimax.io/v1"),
            ChatCompletionsFlavor::MiniMax
        );
        assert_eq!(
            detect_flavor("custom-model", "https://api.minimaxi.com/v1"),
            ChatCompletionsFlavor::MiniMax
        );
    }

    #[test]
    fn minimax_request_body_enables_reasoning_split() {
        let body = build_request_body(
            "MiniMax-M2.7",
            Vec::new(),
            &[],
            Some("high"),
            Some("high"),
            ChatCompletionsFlavor::MiniMax,
        );

        assert_eq!(body["reasoning_split"], serde_json::json!(true));
        assert_eq!(body["reasoning_effort"], serde_json::json!("high"));
    }

    #[test]
    fn stream_reasoning_content_updates_thinking_channel() {
        let mut state = ChatStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };
        let chunk: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": { "reasoning_content": "Think first." },
                "finish_reason": null
            }]
        }))
        .expect("chunk should parse");

        apply_stream_chunk(chunk, &mut state, &ignore_text, &on_thinking, &ignore_tool);

        assert_eq!(state.thinking_text, "Think first.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Think first."
        );
    }

    #[test]
    fn stream_reasoning_details_updates_thinking_channel_without_duplicates() {
        let mut state = ChatStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };

        let first: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": {
                    "reasoning_details": [{
                        "type": "reasoning.text",
                        "id": "reasoning-text-1",
                        "format": "MiniMax-response-v1",
                        "index": 0,
                        "text": "Think"
                    }]
                },
                "finish_reason": null
            }]
        }))
        .expect("first chunk should parse");
        apply_stream_chunk(first, &mut state, &ignore_text, &on_thinking, &ignore_tool);

        let second: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": {
                    "reasoning_details": [{
                        "type": "reasoning.text",
                        "id": "reasoning-text-1",
                        "format": "MiniMax-response-v1",
                        "index": 0,
                        "text": "Think more."
                    }]
                },
                "finish_reason": null
            }]
        }))
        .expect("second chunk should parse");
        apply_stream_chunk(second, &mut state, &ignore_text, &on_thinking, &ignore_tool);

        assert_eq!(state.thinking_text, "Think more.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Think more."
        );
    }

    #[test]
    fn stream_content_finishes_thinking_timing() {
        let mut state = ChatStreamState::new();
        let thinking: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": { "reasoning_content": "Think." },
                "finish_reason": null
            }]
        }))
        .expect("thinking chunk should parse");
        apply_stream_chunk(
            thinking,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        );

        let content: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": { "content": "Answer." },
                "finish_reason": "stop"
            }]
        }))
        .expect("content chunk should parse");
        apply_stream_chunk(
            content,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        );

        assert_eq!(state.full_text, "Answer.");
        assert_eq!(state.finish_reason, "stop");
    }

    #[test]
    fn stream_empty_tool_name_delta_preserves_existing_name() {
        let mut state = ChatStreamState::new();
        let started = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let captured = started.clone();
        let on_tool = move |id: String, name: String| {
            captured
                .lock()
                .expect("tool mutex poisoned")
                .push((id, name));
        };

        let start: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": { "name": "list", "arguments": "" }
                    }]
                },
                "finish_reason": null
            }]
        }))
        .expect("start chunk should parse");
        apply_stream_chunk(start, &mut state, &ignore_text, &ignore_thinking, &on_tool);

        let args: StreamChunk = serde_json::from_value(serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": { "name": "", "arguments": "{\"path\":\"Assets\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }))
        .expect("arguments chunk should parse");
        apply_stream_chunk(args, &mut state, &ignore_text, &ignore_thinking, &on_tool);

        let tool_calls =
            collect_tool_calls(&state.tool_calls_map).expect("tool call should be complete");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "list");
        assert_eq!(
            started.lock().expect("tool mutex poisoned").as_slice(),
            &[("call_1".to_string(), "list".to_string())]
        );
    }

    #[test]
    fn stream_eof_after_finish_reason_finalizes_without_done_marker() {
        let mut state = ChatStreamState::new();
        let chunk = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": {
                            "name": "list",
                            "arguments": "{\"path\":\"Assets\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let raw_response = format!("data: {}\n\n", chunk);
        let mut buffer = raw_response.clone();

        let got_done = drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &ignore_thinking,
            &ignore_tool,
        )
        .expect("stream chunk should parse");

        assert!(!got_done);
        assert!(state.saw_finish_reason);
        let saw_finish_reason = state.saw_finish_reason;
        let response = finalize_stream_response(
            "Custom Chat",
            false,
            saw_finish_reason,
            state,
            String::new(),
            raw_response,
        )
        .expect("finish_reason should be accepted as terminal at EOF");

        assert_eq!(response.finish_reason, "tool_calls");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "list");
        assert_eq!(response.tool_calls[0].arguments, "{\"path\":\"Assets\"}");
    }

    #[test]
    fn collect_tool_calls_rejects_empty_name() {
        let mut tool_calls = HashMap::new();
        tool_calls.insert(
            0,
            PartialToolCall {
                id: "call_1".to_string(),
                name: String::new(),
                arguments: "{}".to_string(),
                notified: false,
            },
        );

        let err = collect_tool_calls(&tool_calls).expect_err("empty name should be rejected");
        assert!(err.contains("missing name"), "unexpected error: {}", err);
    }
}
