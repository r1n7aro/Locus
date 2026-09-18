//! Models reported by the upstream, kept separate from the requested model.
//! These declarations are diagnostic metadata, not proof of model identity.
use serde_json::{json, Value};

pub(crate) const METADATA_KEY: &str = "upstream_model";

pub(crate) fn nonempty(value: &Value) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Default)]
pub(crate) struct Observer {
    first: Option<String>,
    terminal: Option<String>,
}

impl Observer {
    pub(crate) fn observe(&mut self, event: &Value, event_type: &str) {
        let model = [
            &event["response"]["model"],
            &event["message"]["model"],
            &event["model"],
            &event["modelVersion"],
            &event["response"]["modelVersion"],
        ]
        .into_iter()
        .find_map(nonempty);
        let Some(model) = model else { return };
        if matches!(
            event_type,
            "response.completed"
                | "response.done"
                | "response.incomplete"
                | "response.failed"
                | "response.cancelled"
                | "response.canceled"
        ) || event.get("modelVersion").is_some()
        {
            self.terminal = Some(model);
        } else if self.first.is_none() {
            self.first = Some(model);
        }
    }

    pub(crate) fn model(&self) -> Option<String> {
        self.terminal.as_ref().or(self.first.as_ref()).cloned()
    }
}

/// The non-Codex transports already retain the complete wire response even
/// with debug logging disabled. Read it once after a successful attempt, so a
/// failed retry or another tool round cannot supply this response's model.
pub(crate) fn from_wire(raw: &str) -> Option<String> {
    let mut observer = Observer::default();
    if let Ok(event) = serde_json::from_str::<Value>(raw) {
        observer.observe(&event, event["type"].as_str().unwrap_or_default());
        return observer.model();
    }
    let mut event_type = String::new();
    let mut data = String::new();
    let flush = |observer: &mut Observer, data: &mut String, event_type: &str| {
        if let Ok(event) = serde_json::from_str::<Value>(data) {
            observer.observe(&event, event["type"].as_str().unwrap_or(event_type));
        }
        data.clear();
    };
    for line in raw.lines() {
        if line.is_empty() {
            flush(&mut observer, &mut data, &event_type);
            event_type.clear();
        } else if let Some(kind) = line.strip_prefix("event:") {
            event_type = kind.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("data:") {
            // Chat-completions transports also accept complete JSON chunks
            // on adjacent data lines without the usual blank separator.
            if !data.is_empty() {
                if let Ok(event) = serde_json::from_str::<Value>(&data) {
                    observer.observe(&event, event["type"].as_str().unwrap_or(&event_type));
                    data.clear();
                }
            }
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }
    flush(&mut observer, &mut data, &event_type);
    observer.model()
}

pub(crate) fn from_request(request: &Value) -> Option<String> {
    if let Some(model) = request.get(METADATA_KEY) {
        return nonempty(model);
    }
    super::codex::server_model::from_saved_response(&request["codex_response"])
}

/// Preserve opaque Codex replay data; other protocols need only a small
/// diagnostic record, not another copy of the prompt and tool definitions.
pub(crate) fn record(
    request: &mut Option<Value>,
    raw_request: &str,
    raw_response: &str,
) -> Option<(String, String)> {
    let wire_request = serde_json::from_str::<Value>(raw_request).unwrap_or(Value::Null);
    let sent = nonempty(&wire_request["model"])
        .or_else(|| request.as_ref().and_then(|value| nonempty(&value["model"])));
    let model = request
        .as_ref()
        .and_then(|value| super::codex::server_model::from_saved_response(&value["codex_response"]))
        .or_else(|| from_wire(raw_response));
    let metadata = request.get_or_insert_with(|| json!({"model": sent}));
    metadata[METADATA_KEY] = json!(model);
    sent.zip(model)
}

/// Deduplicate within one agent run, including after the user dismisses a
/// banner. Compare the wire model, never a local provider/display alias.
pub(crate) fn mismatch_warning(
    seen: &mut std::collections::HashSet<(String, String)>,
    pair: Option<(String, String)>,
    session_id: &str,
) -> Option<crate::error::AppError> {
    let (sent, reported) = pair?;
    if sent.eq_ignore_ascii_case(&reported)
        || !seen.insert((sent.to_ascii_lowercase(), reported.to_ascii_lowercase()))
    {
        return None;
    }
    Some(crate::error::AppError::new(
        "llm.upstream_model_mismatch",
        format!("Upstream response model differs from the request: requested {sent}, reported {reported}."),
    )
    .detail(json!({"requestedModel": sent, "reportedModel": reported, "sessionId": session_id}).to_string())
    .operation(format!("upstream-model:{session_id}"))
    .severity(crate::error::ErrorSeverity::Warning))
}

#[cfg(test)]
#[path = "upstream_model_tests.rs"]
mod tests;
