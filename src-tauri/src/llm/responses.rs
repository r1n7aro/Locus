use super::openai_reasoning::{
    apply_explicit_reasoning_effort, apply_reasoning_effort, apply_text_verbosity_default,
};
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

use super::openrouter::LlmResponse;
use crate::session::models::{ChatMessage, ImageData, MessageRole, ToolCallInfo};

pub async fn stream_chat<F, G, H>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    base_url: &str,
    thinking_level: Option<&str>,
    explicit_reasoning_effort: Option<&str>,
    debug: bool,
    session_id: Option<&str>,
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
        .tcp_keepalive(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let body = build_request_body(
        model,
        system_prompt,
        history,
        tools,
        thinking_level,
        explicit_reasoning_effort,
        session_id,
    );

    let raw_request = serde_json::to_string_pretty(&body).unwrap_or_default();

    eprintln!(
        "[Responses] POST model={} input={} tools={}",
        model,
        history.len(),
        tools.len()
    );
    let api_url = format!("{}/responses", base_url.trim_end_matches('/'));

    if debug {
        eprintln!("[DEBUG][Responses] request body:\n{}", &raw_request);
        let mut headers: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
        if !api_key.is_empty() {
            headers.push(("Authorization", "Bearer <token>"));
        }
        super::debug::save_request("openai_responses", &api_url, &headers, &raw_request);
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
                let status = resp.status();
                if is_retryable_response_status(status) {
                    let error_body = resp.text().await.unwrap_or_default();
                    last_error = format!("Responses API error ({}): {}", status, error_body);
                    if debug {
                        eprintln!(
                            "[DEBUG][Responses] retryable API error (status={}):\n{}",
                            status, error_body
                        );
                    }
                    if attempt < MAX_RETRIES {
                        let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                        eprintln!(
                            "[Responses] HTTP {} (attempt {}/{}, retrying in {}ms)",
                            status.as_u16(),
                            attempt + 1,
                            MAX_RETRIES + 1,
                            delay
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    continue;
                }
                response = Some(resp);
                break;
            }
            Err(e) => {
                let is_retryable = e.is_connect() || e.is_timeout();
                last_error = format!("Request failed: {}", e);
                if is_retryable && attempt < MAX_RETRIES {
                    let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                    eprintln!(
                        "[Responses] {} (attempt {}/{}, retrying in {}ms)",
                        last_error,
                        attempt + 1,
                        MAX_RETRIES + 1,
                        delay
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
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
                "[DEBUG][Responses] API error (status={}):\n{}",
                status, error_body
            );
        }
        return Err(format!("Responses API error ({}): {}", status, error_body));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut raw_response = String::new();
    let mut state = ResponsesStreamState::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[Responses] Stream error: {}", e);
                if !state.full_text.is_empty() || !state.tool_calls_map.is_empty() {
                    state.stream_broke_early = true;
                    break;
                }
                return Err(format!("Stream read error: {}", e));
            }
        };

        let chunk_text = String::from_utf8_lossy(&chunk);
        raw_response.push_str(&chunk_text);
        buffer.push_str(&chunk_text);

        drain_sse_buffer(
            &mut buffer,
            false,
            debug,
            &mut state,
            &on_text_delta,
            &on_thinking_delta,
            &on_tool_call_start,
        )?;
    }

    drain_sse_buffer(
        &mut buffer,
        true,
        debug,
        &mut state,
        &on_text_delta,
        &on_thinking_delta,
        &on_tool_call_start,
    )?;

    // If stream broke early without response.completed and we have tool calls,
    // reject — the tool call arguments may be truncated/incomplete.
    if state.stream_broke_early && !state.response_completed && !state.tool_calls_map.is_empty() {
        let tc_count = state.tool_calls_map.len();
        let text_len = state.full_text.len();
        return Err(format!(
            "Stream ended before the response finalized (has {} partial tool calls, text_len={}, complete_tool_calls=0)",
            tc_count, text_len
        ));
    }

    let tool_calls = collect_tool_calls(state.tool_calls_map)?;

    if !tool_calls.is_empty() {
        state.finish_reason = "tool_calls".to_string();
    }

    Ok(LlmResponse {
        text: state.full_text,
        tool_calls,
        finish_reason: state.finish_reason,
        response_id: state.response_id,
        input_tokens: state.input_tokens,
        output_tokens: state.output_tokens,
        cache_read_tokens: state.cached_tokens,
        cache_write_tokens: 0,
        cost_usd: 0.0,
        raw_request,
        raw_response,
        thinking_text: state.thinking_text,
        thinking_duration_secs: state.thinking_duration_secs,
        thinking_signature: String::new(),
        continuation_request: None,
    })
}

fn build_request_body(
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    tools: &[serde_json::Value],
    thinking_level: Option<&str>,
    explicit_reasoning_effort: Option<&str>,
    session_id: Option<&str>,
) -> serde_json::Value {
    let request_input = build_request_input(history);

    let mut body = serde_json::json!({
        "model": model,
        "input": request_input.input,
        "stream": true,
    });

    if let Some(previous_response_id) = request_input.previous_response_id {
        body["previous_response_id"] = serde_json::json!(previous_response_id);
    }

    if let Some(sid) = session_id {
        body["prompt_cache_key"] = serde_json::json!(sid);
    }

    if !system_prompt.is_empty() {
        body["instructions"] = serde_json::json!(system_prompt);
    }

    if explicit_reasoning_effort.is_some() {
        apply_explicit_reasoning_effort(&mut body, explicit_reasoning_effort);
    } else {
        apply_reasoning_effort(&mut body, model, thinking_level);
    }
    apply_text_verbosity_default(&mut body, model);

    if !tools.is_empty() {
        let converted: Vec<serde_json::Value> = tools
            .iter()
            .map(|tool| {
                if let Some(func) = tool.get("function") {
                    let mut flattened = serde_json::json!({ "type": "function" });
                    if let Some(name) = func.get("name") {
                        flattened["name"] = name.clone();
                    }
                    if let Some(desc) = func.get("description") {
                        flattened["description"] = desc.clone();
                    }
                    if let Some(params) = func.get("parameters") {
                        flattened["parameters"] = params.clone();
                    }
                    flattened
                } else {
                    tool.clone()
                }
            })
            .collect();
        body["tools"] = serde_json::json!(converted);
    }

    body
}

struct RequestInput {
    input: Vec<serde_json::Value>,
    previous_response_id: Option<String>,
}

fn build_request_input(history: &[ChatMessage]) -> RequestInput {
    if let Some((previous_response_id, start_index)) = find_previous_response_tail(history) {
        if start_index < history.len() {
            let input = build_input_messages(&history[start_index..]);
            if !input.is_empty() {
                return RequestInput {
                    input,
                    previous_response_id: Some(previous_response_id),
                };
            }
        }
    }

    RequestInput {
        input: build_input_messages(history),
        previous_response_id: None,
    }
}

fn find_previous_response_tail(history: &[ChatMessage]) -> Option<(String, usize)> {
    let index = history.iter().rposition(|message| {
        message.role == MessageRole::Assistant
            && message
                .response_id
                .as_deref()
                .map(|value| !value.is_empty())
                .unwrap_or(false)
    })?;

    if history[index + 1..]
        .iter()
        .any(|message| message.role == MessageRole::Assistant)
    {
        return None;
    }

    history[index]
        .response_id
        .clone()
        .map(|response_id| (response_id, index + 1))
}

fn build_input_messages(history: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut input = Vec::new();
    for msg in history {
        match msg.role {
            MessageRole::User => {
                input.push(serde_json::json!({
                    "role": "user",
                    "content": build_user_input_content(&msg.content, msg.images.as_deref()),
                }));
            }
            MessageRole::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    if !tool_calls.is_empty() {
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
                        if !msg.content.is_empty() {
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "content": msg.content,
                            }));
                        }
                        continue;
                    }
                }

                input.push(serde_json::json!({
                    "role": "assistant",
                    "content": msg.content,
                }));
            }
            MessageRole::Tool => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "output": msg.content,
                }));
            }
        }
    }
    input
}

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

struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
    notified: bool,
}

impl PendingToolCall {
    fn has_complete_metadata(&self) -> bool {
        !self.id.trim().is_empty() && !self.name.trim().is_empty()
    }
}

fn missing_tool_call_metadata(tc: &PendingToolCall) -> String {
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

fn collect_tool_calls(map: HashMap<u32, PendingToolCall>) -> Result<Vec<ToolCallInfo>, String> {
    let mut entries: Vec<_> = map.into_iter().collect();
    entries.sort_by_key(|(idx, _)| *idx);
    let mut tool_calls = Vec::with_capacity(entries.len());

    for (idx, tc) in entries {
        if !tc.has_complete_metadata() {
            return Err(format!(
                "Refusing to execute incomplete tool call at index {}: missing {}",
                idx,
                missing_tool_call_metadata(&tc)
            ));
        }
        validate_tool_call_arguments(&tc.name, &tc.arguments)?;
        tool_calls.push(ToolCallInfo {
            id: tc.id,
            name: tc.name,
            arguments: tc.arguments,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        });
    }

    Ok(tool_calls)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReasoningContentKind {
    Summary,
    Text,
}

struct ResponsesStreamState {
    full_text: String,
    thinking_text: String,
    thinking_kind: Option<ReasoningContentKind>,
    thinking_started_at: Option<Instant>,
    thinking_duration_secs: u32,
    finish_reason: String,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
    tool_calls_map: HashMap<u32, PendingToolCall>,
    response_id: Option<String>,
    response_completed: bool,
    stream_broke_early: bool,
}

impl ResponsesStreamState {
    fn new() -> Self {
        Self {
            full_text: String::new(),
            thinking_text: String::new(),
            thinking_kind: None,
            thinking_started_at: None,
            thinking_duration_secs: 0,
            finish_reason: "stop".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            tool_calls_map: HashMap::new(),
            response_id: None,
            response_completed: false,
            stream_broke_early: false,
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

fn parse_event_block(event_block: &str) -> Option<(String, String)> {
    let mut event_type = String::new();
    let mut data_lines = Vec::new();

    for line in event_block.lines() {
        let line = line.trim();
        if let Some(et) = line.strip_prefix("event: ") {
            event_type = et.trim().to_string();
        } else if let Some(data) = line.strip_prefix("data: ") {
            data_lines.push(data.trim().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some((event_type, data_lines.join("\n")))
    }
}

fn parse_json_text_field(data: &str, field: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| value.get(field).and_then(|v| v.as_str()).map(str::to_owned))
}

fn parse_part_text(data: &str, expected_part_type: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| value.get("part").cloned())
        .filter(|part| part.get("type").and_then(|v| v.as_str()) == Some(expected_part_type))
        .and_then(|part| part.get("text").and_then(|v| v.as_str()).map(str::to_owned))
}

fn process_sse_event_block<F, G, H>(
    event_block: &str,
    debug: bool,
    state: &mut ResponsesStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<(), String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    let Some((event_type, data_str)) = parse_event_block(event_block) else {
        return Ok(());
    };

    if debug {
        eprintln!("[DEBUG][Responses] event={} data={}", event_type, &data_str);
    }

    match event_type.as_str() {
        "response.output_text.delta" => {
            if let Ok(ev) = serde_json::from_str::<TextDeltaEvent>(&data_str) {
                state.finish_thinking_timing();
                state.full_text.push_str(&ev.delta);
                on_text_delta(ev.delta);
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let Ok(ev) = serde_json::from_str::<TextDeltaEvent>(&data_str) {
                state.push_reasoning_delta(
                    ReasoningContentKind::Summary,
                    &ev.delta,
                    on_thinking_delta,
                );
            }
        }
        "response.reasoning_summary_text.done" => {
            if let Some(text) = parse_json_text_field(&data_str, "text") {
                state.sync_reasoning_text(ReasoningContentKind::Summary, &text, on_thinking_delta);
            }
        }
        "response.reasoning_summary_part.done" => {
            if let Some(text) = parse_part_text(&data_str, "reasoning_summary_text") {
                state.sync_reasoning_text(ReasoningContentKind::Summary, &text, on_thinking_delta);
            }
        }
        "response.reasoning_text.delta" => {
            if let Ok(ev) = serde_json::from_str::<TextDeltaEvent>(&data_str) {
                state.push_reasoning_delta(
                    ReasoningContentKind::Text,
                    &ev.delta,
                    on_thinking_delta,
                );
            }
        }
        "response.reasoning_text.done" => {
            if let Some(text) = parse_json_text_field(&data_str, "text") {
                state.sync_reasoning_text(ReasoningContentKind::Text, &text, on_thinking_delta);
            }
        }
        "response.content_part.done" => {
            if let Some(text) = parse_part_text(&data_str, "reasoning_text") {
                state.sync_reasoning_text(ReasoningContentKind::Text, &text, on_thinking_delta);
            }
        }
        "response.output_item.added" => {
            if let Ok(ev) = serde_json::from_str::<OutputItemAddedEvent>(&data_str) {
                if ev.item.item_type == "function_call" {
                    let call_id = ev.item.call_id.unwrap_or_default().trim().to_string();
                    let name = ev.item.name.unwrap_or_default().trim().to_string();
                    state.tool_calls_map.insert(
                        ev.output_index,
                        PendingToolCall {
                            id: call_id,
                            name,
                            arguments: String::new(),
                            notified: false,
                        },
                    );
                }
            }
        }
        "response.function_call_arguments.delta" => {
            if let Ok(ev) = serde_json::from_str::<FuncArgsDeltaEvent>(&data_str) {
                if let Some(tc) = state.tool_calls_map.get_mut(&ev.output_index) {
                    tc.arguments.push_str(&ev.delta);
                    if !tc.notified && !tc.id.is_empty() && !tc.name.is_empty() {
                        on_tool_call_start(tc.id.clone(), tc.name.clone());
                        tc.notified = true;
                    }
                }
            }
        }
        "response.function_call_arguments.done" => {
            if let Ok(ev) = serde_json::from_str::<FuncArgsDoneEvent>(&data_str) {
                if let Some(tc) = state.tool_calls_map.get_mut(&ev.output_index) {
                    tc.arguments = ev.arguments;
                    if !tc.notified && !tc.id.is_empty() && !tc.name.is_empty() {
                        on_tool_call_start(tc.id.clone(), tc.name.clone());
                        tc.notified = true;
                    }
                }
            }
        }
        "response.completed" | "response.incomplete" => {
            if let Ok(ev) = serde_json::from_str::<CompletedEvent>(&data_str) {
                state.finish_thinking_timing();
                state.response_id = ev.response.id.filter(|value| !value.is_empty());
                if let Some(usage) = ev.response.usage {
                    state.cached_tokens = usage
                        .input_tokens_details
                        .map(|d| d.cached_tokens)
                        .unwrap_or(0);
                    state.input_tokens = usage.input_tokens.saturating_sub(state.cached_tokens);
                    state.output_tokens = usage.output_tokens;
                }
                if event_type == "response.completed" {
                    state.response_completed = true;
                }
                state.finish_reason = if event_type == "response.incomplete" {
                    "length".to_string()
                } else {
                    ev.response.status.unwrap_or_else(|| "stop".to_string())
                };
            }
        }
        _ => {}
    }

    Ok(())
}

fn drain_sse_buffer<F, G, H>(
    buffer: &mut String,
    flush_final_block: bool,
    debug: bool,
    state: &mut ResponsesStreamState,
    on_text_delta: &F,
    on_thinking_delta: &G,
    on_tool_call_start: &H,
) -> Result<(), String>
where
    F: Fn(String) + Send + 'static,
    G: Fn(String) + Send + 'static,
    H: Fn(String, String) + Send + 'static,
{
    while let Some((pos, sep_len)) = next_sse_separator(buffer) {
        let event_block = buffer[..pos].to_string();
        *buffer = buffer[pos + sep_len..].to_string();
        process_sse_event_block(
            &event_block,
            debug,
            state,
            on_text_delta,
            on_thinking_delta,
            on_tool_call_start,
        )?;
    }

    if flush_final_block {
        let trailing = buffer.trim_matches(|c| c == '\r' || c == '\n').to_string();
        if !trailing.is_empty() {
            process_sse_event_block(
                &trailing,
                debug,
                state,
                on_text_delta,
                on_thinking_delta,
                on_tool_call_start,
            )?;
        }
    }

    Ok(())
}

fn is_retryable_response_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
}

#[derive(Debug, Deserialize)]
struct TextDeltaEvent {
    delta: String,
}

#[derive(Debug, Deserialize)]
struct OutputItemAddedEvent {
    output_index: u32,
    item: OutputItem,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(rename = "type")]
    item_type: String,
    call_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FuncArgsDeltaEvent {
    output_index: u32,
    delta: String,
}

#[derive(Debug, Deserialize)]
struct FuncArgsDoneEvent {
    output_index: u32,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct CompletedEvent {
    response: CompletedResponse,
}

#[derive(Debug, Deserialize)]
struct CompletedResponse {
    id: Option<String>,
    usage: Option<ResponseUsage>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<InputTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct InputTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::{
        build_input_messages, build_request_body, build_request_input, collect_tool_calls,
        drain_sse_buffer, is_retryable_response_status, PendingToolCall, ResponsesStreamState,
    };
    use crate::session::models::{
        ChatMessage, ImageData, MessageRole, ServerToolKind, ToolCallInfo,
    };
    use reqwest::StatusCode;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn ignore_text(_: String) {}
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

    #[test]
    fn builds_user_input_blocks_with_images() {
        let input = build_input_messages(&[user_message_with_images(
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
    fn build_input_messages_includes_function_call_output_for_server_tool_calls() {
        let input = build_input_messages(&[assistant_message_with_tool_calls(
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
        )]);

        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], serde_json::json!("function_call"));
        assert_eq!(input[0]["call_id"], serde_json::json!("ws_1"));
        assert_eq!(input[1]["type"], serde_json::json!("function_call_output"));
        assert_eq!(input[1]["call_id"], serde_json::json!("ws_1"));
        assert_eq!(
            input[1]["output"],
            serde_json::json!("Searched: rust async await")
        );
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
        );

        assert_eq!(body["text"]["verbosity"].as_str(), Some("low"));
    }

    #[test]
    fn build_request_body_includes_explicit_custom_reasoning_effort() {
        let body = build_request_body(
            "deepseek-v4-pro",
            "You are Codex",
            &[user_message_with_images("hello", vec![])],
            &[],
            Some("high"),
            Some("max"),
            None,
        );

        assert_eq!(body["reasoning"], serde_json::json!({ "effort": "max" }));
    }

    #[test]
    fn streams_reasoning_summary_into_thinking_channel() {
        let mut state = ResponsesStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };
        let mut buffer = concat!(
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"delta\":\"Plan first.\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"delta\":\"Answer.\"}\n\n",
            "event: response.completed\n",
            "data: {\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2},\"status\":\"completed\"}}"
        )
        .to_string();

        drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &on_thinking,
            &ignore_tool,
        )
        .expect("reasoning summary should parse");

        assert_eq!(state.thinking_text, "Plan first.");
        assert_eq!(state.full_text, "Answer.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Plan first."
        );
    }

    #[test]
    fn collect_tool_calls_rejects_empty_name() {
        let mut tool_calls = HashMap::new();
        tool_calls.insert(
            0,
            PendingToolCall {
                id: "call_1".to_string(),
                name: String::new(),
                arguments: "{}".to_string(),
                notified: false,
            },
        );

        let err = collect_tool_calls(tool_calls).expect_err("empty name should be rejected");
        assert!(err.contains("missing name"), "unexpected error: {}", err);
    }

    #[test]
    fn recovers_reasoning_text_from_done_event() {
        let mut state = ResponsesStreamState::new();
        let thinking = Arc::new(Mutex::new(String::new()));
        let captured = thinking.clone();
        let on_thinking = move |delta: String| {
            captured
                .lock()
                .expect("thinking mutex poisoned")
                .push_str(&delta);
        };
        let mut buffer = concat!(
            "event: response.reasoning_text.done\r\n",
            "data: {\"text\":\"Need to inspect the file.\"}\r\n\r\n",
            "event: response.completed\r\n",
            "data: {\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1},\"status\":\"completed\"}}"
        )
        .to_string();

        drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &on_thinking,
            &ignore_tool,
        )
        .expect("reasoning done event should parse");

        assert_eq!(state.thinking_text, "Need to inspect the file.");
        assert_eq!(
            thinking.lock().expect("thinking mutex poisoned").as_str(),
            "Need to inspect the file."
        );
    }

    #[test]
    fn subtracts_cached_tokens_from_input_usage() {
        let mut state = ResponsesStreamState::new();
        let mut buffer = concat!(
            "event: response.completed\n",
            "data: {\"response\":{\"id\":\"resp_123\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4,\"input_tokens_details\":{\"cached_tokens\":3}},\"status\":\"completed\"}}"
        )
        .to_string();

        drain_sse_buffer(
            &mut buffer,
            true,
            false,
            &mut state,
            &ignore_text,
            &ignore_text,
            &ignore_tool,
        )
        .expect("cached usage should parse");

        assert_eq!(state.input_tokens, 9);
        assert_eq!(state.cached_tokens, 3);
        assert_eq!(state.output_tokens, 4);
        assert_eq!(state.response_id.as_deref(), Some("resp_123"));
    }

    #[test]
    fn uses_previous_response_id_when_tail_has_no_local_assistant_messages() {
        let request_input = build_request_input(&[
            assistant_message("assistant-1", "call tools", Some("resp_prev")),
            tool_message("tool-1", "call_1", "done"),
            user_message_with_images("继续", vec![]),
        ]);

        assert_eq!(
            request_input.previous_response_id.as_deref(),
            Some("resp_prev")
        );
        assert_eq!(request_input.input.len(), 2);
    }

    #[test]
    fn falls_back_to_full_replay_after_local_assistant_message() {
        let request_input = build_request_input(&[
            assistant_message("assistant-1", "server response", Some("resp_prev")),
            assistant_message("assistant-2", "local compact summary", None),
            user_message_with_images("继续", vec![]),
        ]);

        assert!(request_input.previous_response_id.is_none());
        assert_eq!(request_input.input.len(), 3);
    }

    #[test]
    fn retryable_response_statuses_cover_5xx_only() {
        assert!(is_retryable_response_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(is_retryable_response_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_response_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(is_retryable_response_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(is_retryable_response_status(
            StatusCode::from_u16(529).expect("529 should be a valid extension status")
        ));
        assert!(!is_retryable_response_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_retryable_response_status(StatusCode::BAD_REQUEST));
    }
}
