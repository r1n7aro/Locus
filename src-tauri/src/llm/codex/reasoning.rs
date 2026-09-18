//! Astra configuration updates are input items, not request-header changes.
//! Persist their positions for HTTP replay and WebSocket reconnects.
use super::*;
use serde_json::{json, Value};

pub(super) const METADATA_KEY: &str = "codex_reasoning";

fn effort(value: &Value) -> Option<&'static str> {
    super::super::openai_reasoning::reasoning_effort_for_model("gpt-6-astra", value.as_str())
}

fn update(level: &str) -> Value {
    json!({"type":"configuration_update", "reasoning":{"effort":level}})
}

fn insert_update(input: &mut Vec<Value>, position: usize, level: &str) -> usize {
    // Empty responses or edited histories can leave no content separating two
    // settings. Coalesce that boundary; Responses rejects adjacent updates.
    let mut start = position;
    let mut end = position;
    while start > 0 && input[start - 1]["type"] == "configuration_update" {
        start -= 1;
    }
    while end < input.len() && input[end]["type"] == "configuration_update" {
        end += 1;
    }
    input.splice(start..end, [update(level)]);
    start
}

pub(super) fn replay_update(input: &mut Vec<Value>, metadata: Option<&Value>) {
    let Some(metadata) = metadata else { return };
    if !supports_reasoning_updates(metadata["model"].as_str().unwrap_or_default()) {
        return;
    }
    let state = &metadata[METADATA_KEY];
    if state["version"] != 1 {
        return;
    }
    let Some(level) = effort(&state["effort"]) else {
        return;
    };
    let Some(trailing) = state["update_trailing_items"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
    else {
        return;
    };
    let Some(position) = input.len().checked_sub(trailing) else {
        return;
    };
    insert_update(input, position, level);
}

pub(super) fn prepare(
    body: &mut Value,
    history: &[ChatMessage],
    metadata: Option<&HashMap<String, Value>>,
) {
    let desired = effort(&body["reasoning"]["effort"]).unwrap_or("low");
    let model = body["model"].as_str().unwrap_or_default();
    // Older conversations start from their last actual request header, never
    // from today's UI selection. Schema migration marks absent state explicitly.
    let initial = history
        .iter()
        .rev()
        .find_map(|message| {
            let request = metadata?.get(&message.id)?;
            (request["model"].as_str() == Some(model))
                .then(|| effort(&request["reasoning"]["effort"]))
                .flatten()
        })
        .unwrap_or(desired);
    body["reasoning"]["effort"] = json!(initial);

    let input = body["input"].as_array_mut().expect("Responses input array");
    let window_start = input
        .iter()
        .rposition(|item| item["type"] == "compaction")
        .map(|index| index + 1);
    let last_update = input[window_start.unwrap_or(0)..]
        .iter()
        .rev()
        .find(|item| item["type"] == "configuration_update")
        .and_then(|item| effort(&item["reasoning"]["effort"]));
    let needs_update = last_update.unwrap_or(initial) != desired
        || (window_start.is_some() && last_update.is_none());
    let mut trailing = None;
    if needs_update {
        let pending_start = history
            .iter()
            .rposition(|message| message.role == MessageRole::Assistant)
            .map_or(0, |index| index + 1);
        let pending_users = history[pending_start..]
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count();
        // Insert before the first new user message; for tool-only continuations
        // insert after tool results. Keep a compaction trigger terminal.
        let position = pending_users
            .checked_sub(1)
            .and_then(|skip| {
                input
                    .iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, item)| item["role"] == "user")
                    .nth(skip)
                    .map(|(index, _)| index)
            })
            .unwrap_or(input.len())
            .max(window_start.unwrap_or(0));
        let position = input
            .iter()
            .position(|item| item["type"] == "compaction_trigger")
            .map_or(position, |trigger| position.min(trigger));
        let position = insert_update(input, position, desired);
        trailing = Some(input.len() - position - 1);
    }
    // Tail counts exclude the Lite prefix and survive forked message IDs.
    // Only completed responses persist these local replay records.
    body[METADATA_KEY] = json!({"version":1, "initial_effort":initial, "effort":desired,
        "update_trailing_items":trailing});
}

pub(super) fn continuation_metadata(body: &Value) -> Value {
    let mut metadata = request_without_input(body);
    metadata[METADATA_KEY] = body.get(METADATA_KEY).cloned().unwrap_or(Value::Null);
    metadata
}

pub(super) fn inherited_metadata(mut metadata: Value) -> Value {
    if metadata[METADATA_KEY]["version"] == 1 {
        // An automatic successor inherits the effective setting, but the input
        // update belongs only to the parent response's history boundary.
        metadata[METADATA_KEY]["update_trailing_items"] = Value::Null;
    }
    metadata
}

/// Retain the baseline across replacement windows. The next response adds a
/// fresh update after the compaction item, as required by Responses.
pub(crate) fn compaction_metadata(outcome: &CodexRemoteCompactOutcome) -> Value {
    let mut metadata = json!({"codex_compaction":{
        "output":outcome.output, "encrypted_content":outcome.encrypted_content},
        METADATA_KEY:Value::Null});
    if let Ok(request) = serde_json::from_str::<Value>(&outcome.raw_request) {
        if supports_reasoning_updates(request["model"].as_str().unwrap_or_default()) {
            metadata["model"] = request["model"].clone();
            metadata["reasoning"] = request["reasoning"].clone();
        }
    }
    metadata
}

#[cfg(test)]
#[path = "reasoning_tests.rs"]
mod tests;
