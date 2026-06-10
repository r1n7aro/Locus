use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use crate::session::models::ToolCallInfo;

pub type RawContextStore = Arc<tokio::sync::Mutex<HashMap<String, Vec<RawRound>>>>;
type SessionUnityStateStore = tokio::sync::Mutex<HashMap<String, (String, Option<String>)>>;

pub(super) const MAX_TOOL_ITERATIONS: usize = 200;

pub use crate::commands::CodexTransportMode;

pub(super) fn session_unity_state() -> &'static SessionUnityStateStore {
    static STORE: OnceLock<SessionUnityStateStore> = OnceLock::new();
    STORE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub fn resolve_openrouter_model(model: &str) -> String {
    let short = model.strip_prefix("openrouter/").unwrap_or(model);
    match short {
        "claude-sonnet-4.6" => "anthropic/claude-sonnet-4.6".to_string(),
        "claude-opus-4.6" => "anthropic/claude-opus-4.6".to_string(),
        "glm-5" => "z-ai/glm-5".to_string(),
        "minimax-m2.5" => "minimax/minimax-m2.5".to_string(),
        other => other.to_string(),
    }
}

fn matches_versioned_model(model: &str, base: &str) -> bool {
    if model == base {
        return true;
    }

    model
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.chars().next())
        .map(|ch| ch.is_ascii_digit())
        .unwrap_or(false)
}

const OPENAI_CODEX_CONTEXT_LIMIT: u32 = 258_400;

pub(super) fn model_context_limit(model: &str) -> u32 {
    let m = model.strip_prefix("openrouter/").unwrap_or(model);
    let m = m.strip_prefix("anthropic_sdk/").unwrap_or(m);
    let m = m.strip_prefix("anthropic/").unwrap_or(m);
    let m = m.strip_prefix("openai/").unwrap_or(m);
    let m = m.to_ascii_lowercase();
    // Locus follows the effective context budget currently surfaced by Codex
    // for ChatGPT subscription models, not the larger public API model-page
    // limits. Codex-family variants (-spark, -mini, dated snapshots) share the
    // runtime budget, so match them by family rather than exact version.
    if matches_versioned_model(&m, "gpt-5.5")
        || matches_versioned_model(&m, "gpt-5.5-pro")
        || matches_versioned_model(&m, "gpt-5.4")
        || matches_versioned_model(&m, "gpt-5.4-pro")
        || (m.starts_with("gpt-5") && m.contains("codex"))
    {
        OPENAI_CODEX_CONTEXT_LIMIT
    } else if m.contains("gpt-5") {
        400_000
    } else if m.contains("claude-sonnet-4.6") || m.contains("claude-opus-4.6") {
        1_000_000
    } else if m.contains("claude-opus-4-1") || m.contains("claude-opus-4-20250514") {
        200_000
    } else if m.contains("claude-sonnet-4-20250514") {
        200_000
    } else if m.contains("minimax-m2.5") {
        196_608
    } else if m.contains("minimax-m1") {
        1_000_000
    } else if m.contains("glm-5") {
        202_752
    } else if m.contains("opus") {
        200_000
    } else if m.contains("sonnet") {
        200_000
    } else if m.contains("haiku") {
        200_000
    } else if m.contains("claude") {
        200_000
    } else {
        128_000
    }
}

#[cfg(test)]
mod tests {
    use super::{is_retryable_llm_error, model_context_limit, OPENAI_CODEX_CONTEXT_LIMIT};

    #[test]
    fn uses_codex_runtime_context_limits_for_openai_subscription_models() {
        assert_eq!(
            model_context_limit("openai/gpt-5.5"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-5.5-2026-04-24"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-5.5-pro"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("openai/gpt-5.4"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-5.4-2026-03-05"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-5.4-pro"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("openai/gpt-5.3-codex"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        // Codex-family speed/size variants share the runtime budget instead of
        // falling through to the 400k general gpt-5 bucket.
        assert_eq!(
            model_context_limit("openai/gpt-5.3-codex-spark"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-5.1-codex-mini"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(model_context_limit("gpt-5.2"), 400_000);
    }

    #[test]
    fn keeps_non_openai_limits_unchanged() {
        assert_eq!(
            model_context_limit("openrouter/claude-sonnet-4.6"),
            1_000_000
        );
        assert_eq!(model_context_limit("minimax-m2.5"), 196_608);
        assert_eq!(model_context_limit("unknown-model"), 128_000);
    }

    #[test]
    fn retries_custom_responses_5xx_status_errors() {
        assert!(is_retryable_llm_error(
            r#"Responses API error (502 Bad Gateway): {"error":{"code":"upstream_error","message":"Upstream request failed"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"Responses API error (503 Service Unavailable): temporarily unavailable"#
        ));
        assert!(is_retryable_llm_error(
            r#"Responses API error (529): {"error":{"message":"overloaded"}}"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Responses API error (400 Bad Request): invalid request"#
        ));
    }
}

/// Retry only when the transport failed before we can trust the streamed payload.
pub(super) fn is_retryable_llm_error(error: &str) -> bool {
    error.contains("Stream read error")
        || error.contains("Stream read timed out")
        || error.contains("Stream ended without response.completed")
        || error.contains("Stream ended before the response finalized")
        // Safe to retry because no text or tool-call payload was emitted yet.
        || error.contains("Stream ended with no data and no response.completed")
        || error.contains("Stream ended without message_stop")
        || error.contains("Stream ended with no data and no message_stop")
        || error.contains("Response completed with")
        || error.contains("Refusing to execute partial tool arguments")
        || error.contains("connection")
        || error.contains("EOF")
        || error.contains("overloaded")
        || error.contains("529")
        || error.contains("server error")
        || is_retryable_responses_api_status_error(error)
        // reqwest transport errors (no partial output)
        || error.contains("error sending request")
        || error.contains("Request failed:")
}

fn is_retryable_responses_api_status_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    if !lower.contains("responses api error (") {
        return false;
    }

    lower.contains("responses api error (5")
        || lower.contains("bad gateway")
        || lower.contains("upstream_error")
        || lower.contains("upstream error")
        || lower.contains("upstream request failed")
}

pub(super) fn is_prompt_too_long_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
        || lower.contains("too many tokens")
        || lower.contains("input is too long")
        || lower.contains("input exceeds")
        || lower.contains("maximum number of input")
        || lower.contains("reduce the length")
}

/// LLM backend type
#[derive(Debug, Clone)]
pub enum LlmBackend {
    /// OpenRouter API
    OpenRouter {
        api_key: String,
        base_url: Option<String>,
    },
    /// Anthropic API
    Anthropic {
        access_token: String,
        base_url: Option<String>,
        user_metadata: crate::auth::ClaudeCodeUserMetadata,
    },
    /// Anthropic Agent SDK via Claude Code CLI
    AnthropicAgentSdk,
    /// OpenAI Codex
    OpenAiCodex {
        auth: crate::commands::CodexAuthStateHandle,
        transport: CodexTransportMode,
        base_url: Option<String>,
    },
    /// Custom endpoint
    Custom {
        api_key: String,
        api_model: String,
        endpoint: String,
        api_format: crate::commands::ApiFormat,
        context_length: u32,
        beta_flags: Vec<String>,
        supported_reasoning_efforts: Vec<String>,
        reasoning_param_format: crate::commands::CustomReasoningParamFormat,
        replay_reasoning_content: bool,
        server_tools: crate::commands::CustomEndpointServerTools,
        supports_vision: bool,
    },
}

pub(super) struct LlmCallResult {
    pub text: String,
    pub tool_calls: Vec<ToolCallInfo>,
    #[allow(dead_code)]
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

#[derive(Debug, Serialize)]
pub struct RawRound {
    pub round: usize,
    pub timestamp: i64,
    pub request: serde_json::Value,
    pub response: String,
}

pub(super) fn normalize_tool_args(args: &mut serde_json::Value) {
    const ALIASES: &[(&str, &str)] = &[
        ("file_path", "filePath"),
        ("old_string", "oldString"),
        ("new_string", "newString"),
        ("replace_all", "replaceAll"),
        ("editor_status", "editorStatus"),
        ("request_editor_status", "requestEditorStatus"),
        ("window_title", "windowTitle"),
        ("asset_path", "assetPath"),
        ("max_depth", "maxDepth"),
        ("type_filter", "typeFilter"),
        ("object_path", "objectPath"),
        ("include_files", "includeFiles"),
        ("max_items", "maxItems"),
        ("max_total", "maxTotal"),
        ("scene_path", "scenePath"),
        ("source_field", "sourceField"),
        ("subagent_type", "subagentType"),
    ];

    fn apply_aliases(obj: &mut serde_json::Map<String, serde_json::Value>) {
        let snapshot: Vec<(String, serde_json::Value)> =
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (snake, camel) in ALIASES {
            for (key, val) in &snapshot {
                if key == snake && !obj.contains_key(*camel) {
                    obj.insert(camel.to_string(), val.clone());
                } else if key == camel && !obj.contains_key(*snake) {
                    obj.insert(snake.to_string(), val.clone());
                }
            }
        }
    }

    if let serde_json::Value::Object(ref mut map) = args {
        apply_aliases(map);
        if let Some(serde_json::Value::Array(ref mut arr)) = map.get_mut("edits") {
            for item in arr.iter_mut() {
                if let serde_json::Value::Object(ref mut inner) = item {
                    apply_aliases(inner);
                }
            }
        }
    }
}
