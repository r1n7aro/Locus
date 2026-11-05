//! Codex's standalone web.run service. The client forwards commands with OAuth;
//! it never downloads target pages or falls back to the Locus web_fetch tool.
use crate::session::models::{ChatMessage, MessageRole};
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::OnceLock;
#[cfg(test)]
use std::time::Duration;

#[path = "web_commands.rs"]
mod commands;
#[cfg(test)]
#[path = "web_live_tests.rs"]
mod live_tests;
#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;

pub(crate) const TOOL_NAME: &str = "web.run";
const MAX_OUTPUT_TOKENS: u64 = 12_000;

pub(crate) fn tool_spec() -> Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(|| {
        // web.run is reserved by the model. Load the reference definition
        // verbatim: handwritten constraints or schema compaction cause HTTP 400.
        // Regenerate with `bun scripts/sync-codex-web-tool.mjs`.
        serde_json::from_str(include_str!("../../../../tools/codex_web.json"))
            .expect("valid Codex web tool schema")
    })
    .clone()
}

pub(super) fn is_tool_spec(tool: &Value) -> bool {
    tool["type"] == "namespace"
        && tool["name"] == "web"
        && tool["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "run"))
}

pub(super) fn remove_local_fetch_declarations(tools: &mut Vec<Value>) {
    tools.retain(|tool| tool["name"] != "web_fetch" && tool["function"]["name"] != "web_fetch");
    for tool in tools.iter_mut() {
        if let Some(children) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            remove_local_fetch_declarations(children);
        }
    }
    tools.retain(|tool| {
        tool["type"] != "namespace"
            || tool["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty())
    });
}

pub(super) fn remove_local_fetch_from_history(items: &mut [Value]) {
    for item in items {
        if matches!(
            item["type"].as_str(),
            Some("tool_search_output" | "additional_tools")
        ) {
            if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                remove_local_fetch_declarations(tools);
            }
        }
    }
}

pub(crate) fn request_body(
    session_id: &str,
    model: &str,
    history: &[ChatMessage],
    input: &Value,
) -> Result<Value, String> {
    let commands: commands::SearchCommands = serde_json::from_value(input.clone())
        .map_err(|error| format!("Invalid web.run commands: {error}"))?;
    let commands = serde_json::to_value(commands).map_err(|error| error.to_string())?;
    if !commands.as_object().is_some_and(|fields| {
        fields.iter().any(|(key, value)| {
            key != "response_length" && value.as_array().is_some_and(|items| !items.is_empty())
        })
    }) {
        return Err(
            "web.run requires at least one operation, such as open or search_query.".into(),
        );
    }
    let mut request = json!({"id":session_id,"model":super::protocol_model_name(model),
        "commands":commands,"settings":{"external_web_access":true,"allowed_callers":["direct"]},
        "max_output_tokens":MAX_OUTPUT_TOKENS});
    let input = recent_input(history);
    if !input.is_empty() {
        request["input"] = json!(input);
    }
    Ok(request)
}

// Match the reference client: two user text messages, with at most 1k tokens
// of the intervening assistant text. Do not send tools, images or system prompts.
fn recent_input(history: &[ChatMessage]) -> Vec<Value> {
    let mut users =
        history.iter().enumerate().rev().filter(|(_, message)| {
            message.role == MessageRole::User && !message.content.is_empty()
        });
    let Some((last, _)) = users.next() else {
        return Vec::new();
    };
    let first = users.next().map_or(last, |(index, _)| index);
    let mut budget = 1_000;
    history[first..=last]
        .iter()
        .filter_map(|message| {
            let (role, kind, text) = match message.role {
                MessageRole::User => ("user", "input_text", message.content.as_str()),
                MessageRole::Assistant => {
                    let text = bounded_text(&message.content, budget);
                    budget = budget.saturating_sub(crate::compact::estimate_text_tokens(text));
                    ("assistant", "output_text", text)
                }
                _ => return None,
            };
            (!text.is_empty()).then(|| json!({"role":role,"content":[{"type":kind,"text":text}]}))
        })
        .collect()
}

fn bounded_text(text: &str, budget: u32) -> &str {
    if budget == 0 {
        return "";
    }
    if crate::compact::estimate_text_tokens(text) <= budget {
        return text;
    }
    let mut boundaries: Vec<_> = text.char_indices().map(|(index, _)| index).collect();
    boundaries.push(text.len());
    let (mut low, mut high) = (0, boundaries.len() - 1);
    while low < high {
        let mid = (low + high + 1) / 2;
        if crate::compact::estimate_text_tokens(&text[..boundaries[mid]]) <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    &text[..boundaries[low]]
}

fn endpoint(base_url: Option<&str>) -> String {
    let responses = super::codex_responses_endpoint(base_url);
    format!("{}/alpha/search", responses.trim_end_matches("/responses"))
}

#[derive(Deserialize)]
struct SearchResponse {
    output: String,
}

/// Resolve the current OAuth identity, then refresh once on an HTTP 401.
/// Other failures are returned without substituting another tool or provider.
pub(crate) async fn execute<F, Fut>(
    base_url: Option<&str>,
    body: &Value,
    mut resolve_auth: F,
) -> Result<String, String>
where
    F: FnMut(bool) -> Fut,
    Fut: Future<Output = Result<(String, Option<String>), String>>,
{
    let client = crate::network::rustls_reqwest_client(
        locus_remote_core::network::codex_http_options(locus_remote_core::network::CodexHttpProfile::Web),
    )?;
    let url = endpoint(base_url);
    for refresh in [false, true] {
        let (access_token, account_id) = resolve_auth(refresh).await?;
        let mut request = client
            .post(&url)
            .bearer_auth(access_token)
            .header("originator", super::CODEX_ORIGINATOR_HEADER_VALUE)
            .header("version", super::CODEX_CLIENT_VERSION)
            .json(body);
        if let Some(account_id) = account_id {
            request = request.header("ChatGPT-Account-ID", account_id);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("Codex web.run request failed: {error}"))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && !refresh && !locus_remote_core::network::upstream_auth_failure(status,response.headers()) {
            continue;
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(format!(
                "Codex web.run HTTP error: {}: {}",
                status.as_u16(),
                text.chars().take(2_000).collect::<String>()
            ));
        }
        let response: SearchResponse = response
            .json()
            .await
            .map_err(|error| format!("Invalid Codex web.run response: {error}"))?;
        return Ok(response.output);
    }
    unreachable!("the refreshed request always returns its result")
}
