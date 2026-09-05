//! Codex rust-v0.153.4 Responses wire transformations. Keep provider items
//! opaque in storage; transformations below apply only at the request boundary.
use crate::session::models::{ChatMessage, ToolCallInfo};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) const LITE_HEADER: &str = "x-openai-internal-codex-responses-lite";
pub(super) const LITE_METADATA: &str = "ws_request_header_x_openai_internal_codex_responses_lite";
pub(super) const TURN_STATE_METADATA: &str = "x-codex-turn-state";

pub(super) fn uses_lite(model: &str) -> bool {
    let model = model.strip_prefix("openai/").unwrap_or(model);
    model == "gpt-6-astra" || model.starts_with("gpt-6-astra-")
}

pub(super) fn namespace_tools(tools: Vec<Value>) -> Vec<Value> {
    let mut functions = Vec::new();
    let mut result = Vec::new();
    let mut index = None;
    let mut description = String::new();
    for tool in tools {
        match tool["type"].as_str() {
            Some("function" | "custom") => functions.push(tool),
            Some("namespace") if tool["name"] == "functions" => {
                if let Some(text) = tool["description"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                {
                    description = text.to_string();
                }
                functions.extend(tool["tools"].as_array().into_iter().flatten().cloned());
            }
            _ => {
                result.push(tool);
                continue;
            }
        }
        index.get_or_insert(result.len());
    }
    if let Some(index) = index.filter(|_| !functions.is_empty()) {
        result.insert(
            index,
            json!({"type":"namespace", "name":"functions",
            "description":description, "tools":functions}),
        );
    }
    result
}

pub(super) fn apply_lite(body: &mut Value, session_id: Option<&str>) {
    let namespace = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        session_id.unwrap_or_default().as_bytes(),
    );
    let tools = namespace_tools(
        body.as_object_mut()
            .unwrap()
            .remove("tools")
            .and_then(|tools| tools.as_array().cloned())
            .unwrap_or_default(),
    );
    let tools_id = Uuid::new_v5(&namespace, &serde_json::to_vec(&tools).expect("JSON tools"));
    let mut prefix = vec![
        json!({"type":"additional_tools", "id":format!("at_{tools_id}"),
        "role":"developer", "tools":tools}),
    ];
    if let Some(instructions) = body
        .as_object_mut()
        .unwrap()
        .remove("instructions")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
    {
        let id = Uuid::new_v5(&namespace, instructions.as_bytes());
        prefix.push(
            json!({"type":"message", "id":format!("msg_{id}"), "role":"developer",
            "content":[{"type":"input_text", "text":instructions}]}),
        );
    }
    let input = body["input"].as_array_mut().expect("Responses input array");
    for item in input.iter_mut() {
        for key in ["content", "output"] {
            if let Some(parts) = item.get_mut(key).and_then(Value::as_array_mut) {
                for part in parts {
                    if part["type"] == "input_image" {
                        part.as_object_mut().unwrap().remove("detail");
                    }
                }
            }
        }
        if item["type"] == "tool_search_output" {
            if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                *tools = namespace_tools(std::mem::take(tools));
            }
        }
    }
    input.splice(0..0, prefix);
    body["parallel_tool_calls"] = json!(false);
    body["tool_choice"] = json!("auto");
    body["reasoning"]["context"] = json!("all_turns");
    body["client_metadata"][LITE_METADATA] = json!("true");
}

pub(super) fn is_lite_request(body: &Value) -> bool {
    body["client_metadata"][LITE_METADATA] == "true"
        || body["input"][0]["type"] == "additional_tools"
}

pub(super) fn add_turn_state(request: &mut Value, turn_state: Option<&str>) {
    if let Some(state) = turn_state {
        request["client_metadata"][TURN_STATE_METADATA] = json!(state);
    }
}

pub(super) fn event_turn_state(event: &Value) -> Option<&str> {
    let headers = event
        .get("headers")
        .or_else(|| event.get("response")?.get("headers"))?;
    let value = headers
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(TURN_STATE_METADATA))?
        .1;
    value
        .as_str()
        .or_else(|| value.as_array()?.first()?.as_str())
}

pub(super) fn prepare_items(input: &mut [Value]) {
    for item in input {
        if item["id"].as_str().is_some_and(|id| {
            !id.split_once('_')
                .is_some_and(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
        }) {
            item.as_object_mut().unwrap().remove("id");
        }
    }
}

/// Default namespace maps onto Locus's existing registry. An explicit foreign
/// namespace stays qualified so it cannot accidentally execute a local tool.
pub(super) fn local_function_name(item: &Value) -> String {
    let name = item["name"].as_str().unwrap_or_default().trim();
    match item["namespace"].as_str() {
        None | Some("" | "functions") => {
            name.strip_prefix("functions.").unwrap_or(name).to_string()
        }
        Some(namespace) => format!("{namespace}.{name}"),
    }
}

fn tool_snapshot(tools: &[ToolCallInfo]) -> Value {
    json!(tools
        .iter()
        .filter(|tc| !tc.is_server_tool())
        .map(|tc| json!({"id":tc.id, "name":tc.name, "arguments":tc.arguments}))
        .collect::<Vec<_>>())
}

pub(super) fn response_metadata(
    mut request: Value,
    output: &[Value],
    text: &str,
    tools: &[ToolCallInfo],
    events: &Value,
) -> Value {
    request["codex_response"] = json!({"version":1, "output":output,
        "text":text, "tool_calls":tool_snapshot(tools), "events":events});
    request
}

pub(super) fn replay_output<'a>(metadata: &'a Value, message: &ChatMessage) -> Option<&'a [Value]> {
    let response = metadata.get("codex_response")?;
    // Editing a message or its tool calls invalidates the opaque provider copy.
    if response["version"] != 1
        || response["text"].as_str() != Some(message.content.as_str())
        || response["tool_calls"]
            != tool_snapshot(message.tool_calls.as_deref().unwrap_or_default())
    {
        return None;
    }
    response["output"]
        .as_array()
        .filter(|v| !v.is_empty())
        .map(Vec::as_slice)
}

pub(super) fn encode_http_body(body: &Value) -> Result<Vec<u8>, String> {
    let bytes =
        serde_json::to_vec(body).map_err(|e| format!("Failed to encode Codex request: {e}"))?;
    let compressed = zstd::stream::encode_all(bytes.as_slice(), 3)
        .map_err(|e| format!("Failed to compress Codex request: {e}"))?;
    tracing::debug!(
        uncompressed_bytes = bytes.len(),
        compressed_bytes = compressed.len(),
        "Codex HTTP request compression"
    );
    Ok(compressed)
}

pub(super) fn http_error(error: reqwest::Error) -> String {
    let mut message = format!("Codex request failed: {error}");
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    message
}

pub(super) fn event_error(event: &Value) -> Option<String> {
    let kind = event["type"].as_str()?;
    if kind == "response.incomplete" {
        let reason = event["response"]["incomplete_details"]["reason"]
            .as_str()
            .unwrap_or("unknown");
        return Some(format!("OpenAI Codex incomplete response: {reason}"));
    }
    if !matches!(kind, "response.failed" | "response.cancelled" | "error") {
        return None;
    }
    let error = event
        .get("response")
        .and_then(|r| r.get("error"))
        .or_else(|| event.get("error"))
        .filter(|v| v.is_object())
        .unwrap_or(event);
    let code = error["code"].as_str().unwrap_or(kind);
    let message = error["message"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(kind);
    Some(format!("OpenAI Codex stream error ({code}): {message}"))
}
