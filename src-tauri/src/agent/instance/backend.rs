use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use crate::session::models::{Citation, ToolCallInfo};

pub type RawContextStore = Arc<tokio::sync::Mutex<HashMap<String, Vec<RawRound>>>>;
type SessionUnityStateStore = tokio::sync::Mutex<HashMap<String, (String, Option<String>)>>;

pub use crate::commands::CodexTransportMode;

pub(super) fn session_unity_state() -> &'static SessionUnityStateStore {
    static STORE: OnceLock<SessionUnityStateStore> = OnceLock::new();
    STORE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub fn resolve_openrouter_model(model: &str) -> String {
    let short = model.strip_prefix("openrouter/").unwrap_or(model);
    match short {
        "claude-fable-5" => "anthropic/claude-fable-5".to_string(),
        "claude-sonnet-5" => "anthropic/claude-sonnet-5".to_string(),
        "claude-opus-4.8" => "anthropic/claude-opus-4.8".to_string(),
        "claude-opus-4.7" => "anthropic/claude-opus-4.7".to_string(),
        "claude-sonnet-4.6" => "anthropic/claude-sonnet-4.6".to_string(),
        "claude-opus-4.6" => "anthropic/claude-opus-4.6".to_string(),
        "claude-haiku-4.5" => "anthropic/claude-haiku-4.5".to_string(),
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
const OPENAI_CODEX_5_6_CONTEXT_LIMIT: u32 = 353_400;

pub(super) fn model_context_limit(model: &str) -> u32 {
    let raw = model.trim().to_ascii_lowercase();
    let is_claude_code = raw.starts_with("claude_code/");
    let m = raw.strip_prefix("openrouter/").unwrap_or(&raw);
    let m = m.strip_prefix("claude_code/").unwrap_or(m);
    let m = m.strip_prefix("anthropic/").unwrap_or(m);
    let m = m.strip_prefix("openai/").unwrap_or(m);
    let has_1m_suffix = m.ends_with("[1m]");
    let m = m.strip_suffix("[1m]").unwrap_or(m);
    // Locus follows the effective context budget currently surfaced by Codex
    // for ChatGPT subscription models, not the larger public API model-page
    // limits. Codex-family variants (-spark, -mini, dated snapshots) share the
    // runtime budget, so match them by family rather than exact version.
    if m == "gpt-5.6" || m.starts_with("gpt-5.6-") {
        OPENAI_CODEX_5_6_CONTEXT_LIMIT
    } else if matches_versioned_model(&m, "gpt-6-astra")
        || matches_versioned_model(&m, "gpt-5.5")
        || matches_versioned_model(&m, "gpt-5.5-pro")
        || matches_versioned_model(&m, "gpt-5.4")
        || matches_versioned_model(&m, "gpt-5.4-pro")
        || (m.starts_with("gpt-5") && m.contains("codex"))
    {
        OPENAI_CODEX_CONTEXT_LIMIT
    } else if m.contains("gpt-5") {
        400_000
    } else if has_1m_suffix
        || m.contains("claude-fable-5")
        || m.contains("claude-mythos-5")
        || m.contains("claude-mythos-preview")
        || m.contains("claude-sonnet-5")
        || (!is_claude_code
            && (m.contains("claude-opus-4.8")
                || m.contains("claude-opus-4-8")
                || m.contains("claude-opus-4.7")
                || m.contains("claude-opus-4-7")
                || m.contains("claude-opus-4.6")
                || m.contains("claude-opus-4-6")
                || m.contains("claude-sonnet-4.6")
                || m.contains("claude-sonnet-4-6")))
    {
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
    use super::{
        build_mock_response_plan, is_prompt_too_long_error, is_retryable_llm_error,
        model_context_limit, resolve_openrouter_model, MockModelProfile,
        OPENAI_CODEX_5_6_CONTEXT_LIMIT, OPENAI_CODEX_CONTEXT_LIMIT,
    };
    use crate::session::models::{ChatMessage, MessageRole};

    fn message(id: &str, role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role,
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
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
            render_parts: None,
        }
    }

    #[test]
    fn prompt_too_long_matches_provider_error_shapes() {
        // Anthropic prose and error `type`.
        assert!(is_prompt_too_long_error(
            "prompt is too long: 213462 tokens > 200000 maximum"
        ));
        assert!(is_prompt_too_long_error(
            "API error: {\"type\":\"invalid_request_error\",\"message\":\"prompt_too_long\"}"
        ));
        // Anthropic combined input + max_tokens validation.
        assert!(is_prompt_too_long_error(
            "input length and `max_tokens` exceed context limit: 195122 + 8192 > 200000"
        ));
        // OpenAI-compatible servers (code and prose).
        assert!(is_prompt_too_long_error(
            "error code: context_length_exceeded"
        ));
        assert!(is_prompt_too_long_error(
            "This model's maximum context length is 65536 tokens. However, you requested 70000 tokens"
        ));
        // Generic relayed phrasings.
        assert!(is_prompt_too_long_error(
            "the request exceeds the context window of this model"
        ));
        assert!(is_prompt_too_long_error(
            "requested tokens are larger than the context size"
        ));
        assert!(is_prompt_too_long_error(
            "Input is too long for requested model."
        ));
    }

    #[test]
    fn prompt_too_long_ignores_unrelated_errors() {
        assert!(!is_prompt_too_long_error("connection reset by peer"));
        assert!(!is_prompt_too_long_error("429 too many requests"));
        assert!(!is_prompt_too_long_error("invalid api key"));
        // Local tool-output placeholder text must never be classified as a
        // provider prompt-length rejection.
        assert!(!is_prompt_too_long_error(
            crate::compact::CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE
        ));
    }

    #[test]
    fn uses_codex_runtime_context_limits_for_openai_subscription_models() {
        assert_eq!(
            model_context_limit("openai/gpt-6-astra"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("gpt-6-astra"),
            OPENAI_CODEX_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("openai/gpt-5.6-sol"),
            OPENAI_CODEX_5_6_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("openai/gpt-5.6-terra"),
            OPENAI_CODEX_5_6_CONTEXT_LIMIT
        );
        assert_eq!(
            model_context_limit("openai/gpt-5.6-luna"),
            OPENAI_CODEX_5_6_CONTEXT_LIMIT
        );
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
        assert_eq!(model_context_limit("openrouter/claude-fable-5"), 1_000_000);
        assert_eq!(model_context_limit("anthropic/claude-sonnet-5"), 1_000_000);
        assert_eq!(
            model_context_limit("claude_code/claude-sonnet-5"),
            1_000_000
        );
        assert_eq!(
            model_context_limit("openrouter/claude-sonnet-4.6"),
            1_000_000
        );
        assert_eq!(model_context_limit("anthropic/claude-opus-4-8"), 1_000_000);
        assert_eq!(model_context_limit("openrouter/claude-haiku-4.5"), 200_000);
        assert_eq!(
            model_context_limit("claude_code/claude-opus-4.6[1m]"),
            1_000_000
        );
        assert_eq!(model_context_limit("claude_code/claude-opus-4.6"), 200_000);
        assert_eq!(model_context_limit("minimax-m2.5"), 196_608);
        assert_eq!(model_context_limit("unknown-model"), 128_000);
    }

    #[test]
    fn resolves_current_openrouter_claude_short_ids() {
        assert_eq!(
            resolve_openrouter_model("openrouter/claude-sonnet-5"),
            "anthropic/claude-sonnet-5"
        );
        assert_eq!(
            resolve_openrouter_model("openrouter/claude-fable-5"),
            "anthropic/claude-fable-5"
        );
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

    #[test]
    fn retries_api_5xx_and_429_status_errors_across_endpoints() {
        // Custom OpenAI-compatible endpoints (issue #101): flaky providers
        // answer 5xx/429 and the agent loop must classify those as retryable.
        assert!(is_retryable_llm_error(
            r#"Custom Chat API error (500 Internal Server Error): {"error":{"message":"upstream broke"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"Custom Chat API error (502 Bad Gateway): <html>bad gateway</html>"#
        ));
        assert!(is_retryable_llm_error(
            r#"Custom Chat API error (503 Service Unavailable): busy"#
        ));
        assert!(is_retryable_llm_error(
            r#"Custom Chat API error (429 Too Many Requests): {"error":{"message":"rate limited"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"Responses API error (429 Too Many Requests): slow down"#
        ));
        assert!(is_retryable_llm_error(
            r#"Custom(Anthropic) API error (529): {"error":{"type":"overloaded_error"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"Anthropic API error (503 Service Unavailable): upstream capacity"#
        ));
    }

    #[test]
    fn never_retries_non_429_4xx_status_errors() {
        // issue #94: a 400 must not loop. The first case carries a keyword
        // ("connection") that the transport heuristics would match — the
        // explicit 4xx status has to win over that.
        assert!(!is_retryable_llm_error(
            r#"Custom Chat API error (400 Bad Request): {"error":{"message":"invalid connection parameter"}}"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Custom Chat API error (400 Bad Request): {"error":{"message":"bad request"}}"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Custom Chat API error (401 Unauthorized): invalid api key"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Custom Chat API error (403 Forbidden): quota exceeded"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Custom Chat API error (404 Not Found): no such model"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Custom(Anthropic) API error (400 Bad Request): {"error":{"type":"invalid_request_error"}}"#
        ));
        assert!(!is_retryable_llm_error(
            r#"Anthropic API error (401 Unauthorized): {"error":{"type":"authentication_error"}}"#
        ));
    }

    #[test]
    fn keyword_heuristics_still_apply_without_a_status_shape() {
        // Errors that don't carry the "<tag> API error (NNN" shape keep the
        // historical keyword classification.
        assert!(is_retryable_llm_error(
            "Stream read error: connection reset"
        ));
        assert!(is_retryable_llm_error(
            "Request failed: error sending request"
        ));
        assert!(!is_retryable_llm_error("tool loop aborted by user"));
    }

    #[test]
    fn recognizes_only_supported_mock_model_ids() {
        assert_eq!(
            MockModelProfile::from_model_id("mock/stream"),
            Some(MockModelProfile::Stream)
        );
        assert_eq!(
            MockModelProfile::from_model_id("mock/tool"),
            Some(MockModelProfile::Tool)
        );
        assert_eq!(
            MockModelProfile::from_model_id("mock/error"),
            Some(MockModelProfile::Error)
        );
        assert_eq!(MockModelProfile::from_model_id("mock/unknown"), None);
    }

    #[test]
    fn mock_stream_plan_uses_a_deterministic_local_response() {
        let messages = vec![
            message("user-1", MessageRole::User, "first"),
            message("assistant-1", MessageRole::Assistant, "done"),
            message("user-2", MessageRole::User, "测试本地回复"),
        ];
        let plan = build_mock_response_plan(MockModelProfile::Stream, &messages, &[]);

        assert_eq!(plan.text, "模拟模型已生成本地流式响应。");
        assert!(!plan.thinking.is_empty());
        assert!(plan.tool_calls.is_empty());
        assert!(plan.error.is_none());
    }

    #[test]
    fn mock_tool_plan_calls_todowrite_then_finishes_after_its_result() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "todowrite" }
        })];
        let mut messages = vec![message("user-1", MessageRole::User, "run tool")];
        let first = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert_eq!(first.tool_calls.len(), 1);
        assert_eq!(first.tool_calls[0].name, "todowrite");
        assert!(first.text.is_empty());

        let mut tool_result = message("tool-1", MessageRole::Tool, "Todos updated");
        tool_result.tool_call_id = Some(first.tool_calls[0].id.clone());
        messages.push(message("assistant-1", MessageRole::Assistant, ""));
        messages.push(tool_result);
        let follow_up = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert!(follow_up.tool_calls.is_empty());
        assert!(follow_up.text.contains("simulated tool call completed"));
    }

    #[test]
    fn mock_tool_plan_can_call_checkout_scoped_python() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "python" }
        })];
        let messages = vec![message(
            "user-python",
            MessageRole::User,
            "[[mock:python-tool]] inspect Unity",
        )];

        let plan = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert_eq!(plan.tool_calls.len(), 1);
        assert_eq!(plan.tool_calls[0].name, "python");
        assert!(plan.tool_calls[0]
            .arguments
            .contains("get_unity_editor_status"));
        assert!(plan.tool_calls[0].arguments.contains("readonly"));
    }

    #[test]
    fn mock_tool_plan_can_reproduce_agent_unity_execute() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "unity_execute" }
        })];
        let messages = vec![message(
            "user-unity-execute",
            MessageRole::User,
            super::MOCK_AGENT_UNITY_EXECUTE_SCENARIO,
        )];

        let plan = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert_eq!(plan.tool_calls.len(), 1);
        assert_eq!(plan.tool_calls[0].name, "unity_execute");
        let arguments: serde_json::Value =
            serde_json::from_str(&plan.tool_calls[0].arguments).expect("unity_execute arguments");
        assert_eq!(arguments["readonly"], true);
        assert_eq!(arguments["request_editor_status"], "editing");
        assert!(arguments["code"]
            .as_str()
            .is_some_and(|code| code.contains("GetActiveScene")));
    }

    #[test]
    fn mock_tool_plan_can_reproduce_agent_unity_yaml_read() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "unity_yaml_read" }
        })];
        let messages = vec![message(
            "user-unity-yaml-read",
            MessageRole::User,
            super::MOCK_AGENT_UNITY_YAML_READ_SCENARIO,
        )];

        let plan = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert_eq!(plan.tool_calls.len(), 1);
        assert_eq!(plan.tool_calls[0].name, "unity_yaml_read");
        let arguments: serde_json::Value =
            serde_json::from_str(&plan.tool_calls[0].arguments).expect("unity_yaml_read arguments");
        assert_eq!(arguments["depth"], 1);
        assert_eq!(arguments["max_array_items"], 2);
        assert_eq!(
            arguments["path"],
            "Assets/Scenes/Examples/AudioExample.unity"
        );
        assert_eq!(
            arguments["hierarchy_fields"],
            serde_json::json!(["components", "active"])
        );
    }

    #[test]
    fn mock_tool_plan_can_reproduce_an_orphaned_stream_start() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": { "name": "todowrite" }
        })];
        let messages = vec![message(
            "user-orphan",
            MessageRole::User,
            "[[mock:orphan-tool-start]] reproduce the handoff",
        )];
        let plan = build_mock_response_plan(MockModelProfile::Tool, &messages, &tools);

        assert_eq!(
            plan.provisional_tool_call,
            Some((
                "mock-orphan-user-orphan-initial".to_string(),
                "edit".to_string(),
            ))
        );
        assert!(plan.tool_starts_after_text);
        assert!(!plan.text.is_empty());
        assert_eq!(plan.tool_calls.len(), 1);
        assert_ne!(plan.tool_calls[0].id, "mock-orphan-user-orphan-initial");
    }

    #[test]
    fn mock_error_plan_returns_a_deterministic_failure() {
        let plan = build_mock_response_plan(MockModelProfile::Error, &[], &[]);
        assert_eq!(plan.error.as_deref(), Some("Simulated model backend error"));
        assert!(plan.text.is_empty());
        assert!(plan.tool_calls.is_empty());
    }
}

/// Retry only when the transport failed before we can trust the streamed payload.
pub(super) fn is_retryable_llm_error(error: &str) -> bool {
    // HTTP status failures carry an explicit verdict, so let it override the
    // keyword heuristics below: a non-429 4xx means the endpoint rejected
    // this exact request (bad request/auth/model — issue #94's 400s) and
    // replaying it can only fail the same way, while 5xx/529/429 are
    // transient upstream states worth another attempt.
    if let Some(status) = api_error_status(error) {
        if (400..500).contains(&status) && status != 429 {
            return false;
        }
        if status >= 500 || status == 429 {
            return true;
        }
    }

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

/// Extract `NNN` from error strings shaped like `<tag> API error (NNN ...): body`,
/// the format every HTTP transport in `llm::*` uses for status failures
/// ("Custom Chat", "Responses", "Anthropic", "Custom(Anthropic)"). Returns
/// `None` for strings that don't lead with that shape.
fn api_error_status(error: &str) -> Option<u16> {
    const MARKER: &str = " api error (";
    let lower = error.to_ascii_lowercase();
    let idx = lower.find(MARKER)?;
    let digits: String = lower[idx + MARKER.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() != 3 {
        return None;
    }
    digits.parse().ok()
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
        // Anthropic error `type` and OpenAI error `code` identifiers appear
        // verbatim in relayed error strings.
        || lower.contains("prompt_too_long")
        || lower.contains("context_length_exceeded")
        || lower.contains("exceeds the context")
        || lower.contains("larger than the context")
        // Anthropic's combined input + max_tokens validation ("input length
        // and `max_tokens` exceed context limit: X + Y > Z") — compaction
        // shrinks the input side, so it is recoverable.
        || lower.contains("exceed context limit")
}

/// Deterministic, local-only model behavior exposed while Locus Debug mode is
/// enabled. Each profile exercises a different part of the regular agent
/// pipeline without opening a network connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockModelProfile {
    Stream,
    Tool,
    Error,
}

impl MockModelProfile {
    pub fn from_model_id(model: &str) -> Option<Self> {
        match model.trim() {
            "mock/stream" => Some(Self::Stream),
            "mock/tool" => Some(Self::Tool),
            "mock/error" => Some(Self::Error),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Tool => "tool",
            Self::Error => "error",
        }
    }
}

/// LLM backend type
#[derive(Debug, Clone)]
pub enum LlmBackend {
    /// In-process deterministic backend available only in Debug mode.
    Mock { profile: MockModelProfile },
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
    /// Local Claude Code CLI process controlled through stream-json.
    ClaudeCodeCli,
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
        remote_compaction_mode: crate::commands::RemoteCompactionMode,
        /// Model-level opt-in for protocol-native lazy tool loading
        /// (`defer_loading` + `tool_reference`) on Anthropic-format endpoints.
        supports_tool_lazy_loading: bool,
        supported_reasoning_efforts: Vec<String>,
        reasoning_param_format: crate::commands::CustomReasoningParamFormat,
        replay_reasoning_content: bool,
        /// Explicit message-level replay field (models.dev `interleaved.field`);
        /// None falls back to model-name flavor detection.
        reasoning_replay_field: Option<crate::commands::ReasoningReplayField>,
        server_tools: crate::commands::CustomEndpointServerTools,
        supports_vision: bool,
    },
}

pub(super) struct LlmCallResult {
    pub text: String,
    pub citations: Vec<Citation>,
    pub tool_calls: Vec<ToolCallInfo>,
    #[allow(dead_code)]
    pub finish_reason: String,
    pub end_turn: Option<bool>,
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

#[derive(Debug, Clone, Serialize)]
pub struct RawRound {
    pub round: usize,
    pub timestamp: i64,
    pub request: serde_json::Value,
    pub response: String,
}

#[derive(Debug)]
struct MockResponsePlan {
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCallInfo>,
    provisional_tool_call: Option<(String, String)>,
    tool_starts_after_text: bool,
    error: Option<String>,
}

const MOCK_ORPHAN_TOOL_START_SCENARIO: &str = "[[mock:orphan-tool-start]]";
pub(crate) const MOCK_WORKSPACE_SWITCH_HOLD_SCENARIO: &str = "[[mock:workspace-switch-hold]]";
pub(crate) const MOCK_PYTHON_TOOL_SCENARIO: &str = "[[mock:python-tool]]";
pub(crate) const MOCK_AGENT_UNITY_EXECUTE_SCENARIO: &str = "[[mock:agent-unity-execute]]";
pub(crate) const MOCK_AGENT_UNITY_YAML_READ_SCENARIO: &str = "[[mock:agent-unity-yaml-read]]";
pub(crate) const MOCK_SESSION_UNDO_FILE_SCENARIO: &str = "[[mock:session-undo-file]]";
const MOCK_WORKSPACE_SWITCH_HOLD_DELAY_MS: u64 = 8_000;

fn api_tool_name(tool: &serde_json::Value) -> Option<&str> {
    tool.get("function")
        .and_then(|function| function.get("name"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| tool.get("name").and_then(serde_json::Value::as_str))
}

fn tool_is_exposed(api_tools: &[serde_json::Value], name: &str) -> bool {
    api_tools
        .iter()
        .any(|tool| api_tool_name(tool) == Some(name))
}

fn latest_user_message(
    messages: &[crate::session::models::ChatMessage],
) -> Option<(usize, &crate::session::models::ChatMessage)> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role == crate::session::models::MessageRole::User)
}

fn mock_response_text(user_text: &str, after_tool: bool) -> String {
    let user_text = user_text.trim();
    let contains_cjk = user_text
        .chars()
        .any(|ch| matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF));
    if after_tool {
        if contains_cjk {
            "模拟工具调用已完成，工具结果已进入后续模型轮次。".to_string()
        } else {
            "The simulated tool call completed and its result reached the follow-up model round."
                .to_string()
        }
    } else if contains_cjk {
        "模拟模型已生成本地流式响应。".to_string()
    } else {
        "The mock model generated a local streaming response.".to_string()
    }
}

fn build_mock_response_plan(
    profile: MockModelProfile,
    messages: &[crate::session::models::ChatMessage],
    api_tools: &[serde_json::Value],
) -> MockResponsePlan {
    let (latest_user_index, latest_user) = latest_user_message(messages)
        .map(|(index, message)| (index, Some(message)))
        .unwrap_or((0, None));
    let user_text = latest_user
        .map(|message| message.content.as_str())
        .unwrap_or("");
    let has_tool_result = latest_user.is_some()
        && messages[latest_user_index.saturating_add(1)..]
            .iter()
            .any(|message| message.role == crate::session::models::MessageRole::Tool);
    let simulate_orphan_tool_start = user_text.contains(MOCK_ORPHAN_TOOL_START_SCENARIO);
    let provisional_tool_call = simulate_orphan_tool_start.then(|| {
        (
            format!(
                "mock-orphan-{}-{}",
                latest_user
                    .map(|message| message.id.as_str())
                    .unwrap_or("turn"),
                if has_tool_result {
                    "follow-up"
                } else {
                    "initial"
                },
            ),
            "edit".to_string(),
        )
    });
    match profile {
        MockModelProfile::Stream => MockResponsePlan {
            text: mock_response_text(user_text, false),
            thinking: "Synthesizing a deterministic local response.".to_string(),
            tool_calls: Vec::new(),
            provisional_tool_call,
            tool_starts_after_text: simulate_orphan_tool_start,
            error: None,
        },
        MockModelProfile::Tool if has_tool_result => MockResponsePlan {
            text: mock_response_text(user_text, true),
            thinking: "Inspecting the simulated tool result.".to_string(),
            tool_calls: Vec::new(),
            provisional_tool_call,
            tool_starts_after_text: simulate_orphan_tool_start,
            error: None,
        },
        MockModelProfile::Tool => {
            let tool_call = if user_text.contains(MOCK_AGENT_UNITY_YAML_READ_SCENARIO)
                && tool_is_exposed(api_tools, "unity_yaml_read")
            {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-unity-yaml-read-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "unity_yaml_read".to_string(),
                    arguments: serde_json::json!({
                        "depth": 1,
                        "hierarchy_fields": ["components", "active"],
                        "max_array_items": 2,
                        "path": "Assets/Scenes/Examples/AudioExample.unity"
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else if user_text.contains(MOCK_AGENT_UNITY_EXECUTE_SCENARIO)
                && tool_is_exposed(api_tools, "unity_execute")
            {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-unity-execute-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "unity_execute".to_string(),
                    arguments: serde_json::json!({
                        "async": "sync",
                        "code": "var scene = UnityEditor.SceneManagement.EditorSceneManager.GetActiveScene(); print(scene.path); print(scene.name);",
                        "enable_non_public_access": true,
                        "readonly": true,
                        "request_editor_status": "editing"
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else if user_text.contains(MOCK_PYTHON_TOOL_SCENARIO)
                && tool_is_exposed(api_tools, "python")
            {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-python-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "python".to_string(),
                    arguments: serde_json::json!({
                        "action": "run",
                        "code": "status = await locus.get_unity_editor_status(project=project)\nprint(f'LOCUS_PYTHON_TOOL_OK:{status.process_state}:{status.ready}')",
                        "description": "Probe the injected Locus SDK and Unity lifecycle state",
                        "readonly": true,
                        "timeout": 30_000
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else if user_text.contains(MOCK_SESSION_UNDO_FILE_SCENARIO)
                && tool_is_exposed(api_tools, "write")
            {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-session-undo-file-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "write".to_string(),
                    arguments: serde_json::json!({
                        "filePath": ".locus-session-undo-driver-probe.txt",
                        "content": "LOCUS_SESSION_UNDO_DRIVER_PROBE\n"
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else if tool_is_exposed(api_tools, "todowrite") {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-tool-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "todowrite".to_string(),
                    arguments: serde_json::json!({
                        "todos": [{
                            "content": "Verify simulated model tool execution",
                            "status": "completed",
                            "priority": "medium"
                        }]
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else if tool_is_exposed(api_tools, "tool_load") {
                Some(ToolCallInfo {
                    id: format!(
                        "mock-tool-{}",
                        latest_user
                            .map(|message| message.id.as_str())
                            .unwrap_or("turn")
                    ),
                    name: "tool_load".to_string(),
                    arguments: serde_json::json!({ "tools": ["todowrite"] }).to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                })
            } else {
                None
            };
            MockResponsePlan {
                text: if simulate_orphan_tool_start {
                    mock_response_text(user_text, false)
                } else {
                    tool_call
                        .is_none()
                        .then(|| "No compatible local tool is exposed for this agent.".to_string())
                        .unwrap_or_default()
                },
                thinking: "Preparing a deterministic local tool call.".to_string(),
                tool_calls: tool_call.into_iter().collect(),
                provisional_tool_call,
                tool_starts_after_text: simulate_orphan_tool_start,
                error: None,
            }
        }
        MockModelProfile::Error => MockResponsePlan {
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            provisional_tool_call,
            tool_starts_after_text: simulate_orphan_tool_start,
            error: Some("Simulated model backend error".to_string()),
        },
    }
}

fn mock_text_chunks(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for ch in text.chars() {
        chunk.push(ch);
        if chunk.chars().count() >= max_chars {
            chunks.push(std::mem::take(&mut chunk));
        }
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

fn mock_token_count(text: &str) -> u32 {
    ((text.chars().count() as u32).saturating_add(3) / 4).max(1)
}

pub(super) async fn stream_mock_response(
    profile: MockModelProfile,
    messages: &[crate::session::models::ChatMessage],
    api_tools: &[serde_json::Value],
    on_text_delta: impl Fn(String) + Send + Sync + 'static,
    on_thinking_delta: impl Fn(String) + Send + Sync + 'static,
    on_tool_call_start: impl Fn(String, String) + Send + Sync + 'static,
) -> Result<LlmCallResult, String> {
    const INITIAL_DELAY_MS: u64 = 320;
    const THINKING_DELAY_MS: u64 = 90;
    const TEXT_CHUNK_DELAY_MS: u64 = 45;

    let hold_for_workspace_switch = latest_user_message(messages).is_some_and(|(_, message)| {
        message
            .content
            .contains(MOCK_WORKSPACE_SWITCH_HOLD_SCENARIO)
    });
    let plan = build_mock_response_plan(profile, messages, api_tools);
    tokio::time::sleep(std::time::Duration::from_millis(
        if hold_for_workspace_switch {
            MOCK_WORKSPACE_SWITCH_HOLD_DELAY_MS
        } else {
            INITIAL_DELAY_MS
        },
    ))
    .await;
    if let Some(error) = plan.error.as_ref() {
        return Err(error.clone());
    }

    for chunk in mock_text_chunks(&plan.thinking, 18) {
        on_thinking_delta(chunk);
        tokio::time::sleep(std::time::Duration::from_millis(THINKING_DELAY_MS)).await;
    }

    if let Some((id, name)) = &plan.provisional_tool_call {
        on_tool_call_start(id.clone(), name.clone());
    }

    if !plan.tool_starts_after_text {
        for tool_call in &plan.tool_calls {
            on_tool_call_start(tool_call.id.clone(), tool_call.name.clone());
        }
    }

    for chunk in mock_text_chunks(&plan.text, 10) {
        on_text_delta(chunk);
        tokio::time::sleep(std::time::Duration::from_millis(TEXT_CHUNK_DELAY_MS)).await;
    }

    if plan.tool_starts_after_text {
        for tool_call in &plan.tool_calls {
            on_tool_call_start(tool_call.id.clone(), tool_call.name.clone());
        }
    }

    let input_text = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let raw_request = serde_json::json!({
        "backend": "mock",
        "profile": profile.label(),
        "messageCount": messages.len(),
        "toolCount": api_tools.len(),
    })
    .to_string();
    let raw_response = serde_json::json!({
        "text": &plan.text,
        "thinking": &plan.thinking,
        "toolCalls": &plan.tool_calls,
    })
    .to_string();
    let has_tool_calls = !plan.tool_calls.is_empty();
    let output_tokens = mock_token_count(&format!("{}{}", plan.thinking, plan.text));
    let thinking_signature = format!("mock:{}", profile.label());

    Ok(LlmCallResult {
        text: plan.text,
        citations: Vec::new(),
        tool_calls: plan.tool_calls,
        finish_reason: if has_tool_calls { "tool_calls" } else { "stop" }.to_string(),
        end_turn: Some(!has_tool_calls),
        response_id: latest_user_message(messages)
            .map(|(_, message)| format!("mock-{}-{}", profile.label(), message.id)),
        input_tokens: mock_token_count(&input_text),
        output_tokens,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: 0.0,
        raw_request,
        raw_response,
        thinking_text: plan.thinking,
        thinking_duration_secs: 1,
        thinking_signature,
        continuation_request: None,
    })
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
