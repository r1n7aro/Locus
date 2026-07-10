pub mod anthropic;
pub mod anthropic_usage;
pub mod chat_completions;
pub mod claude_code_cli;
pub mod codex;
pub mod codex_models;
pub mod codex_usage;
pub mod debug;
pub mod openai_reasoning;
pub mod openrouter;
pub mod responses;
pub mod retry;
pub mod streaming;
pub mod think_tag_filter;

pub(crate) const CODEX_CLIENT_VERSION: &str = "0.144.1";

/// Normalize historical tool call arguments before replaying them to an
/// OpenAI-shaped endpoint. Strict servers (vLLM-based providers such as
/// MiMo) run `json.loads` on `function.arguments` and reject the whole
/// request when it is not valid JSON, so empty or malformed strings are
/// replaced with `{}`. Valid JSON is returned as-is, byte for byte.
pub(crate) fn normalize_tool_call_arguments<'a>(tool_name: &str, arguments: &'a str) -> &'a str {
    if arguments.trim().is_empty() {
        return "{}";
    }
    if serde_json::from_str::<serde_json::Value>(arguments).is_ok() {
        return arguments;
    }
    eprintln!(
        "[LLM] Replacing malformed tool call arguments for '{}' with {{}} before replay",
        tool_name
    );
    "{}"
}

#[cfg(test)]
mod tests {
    use super::normalize_tool_call_arguments;

    #[test]
    fn empty_arguments_normalize_to_empty_object() {
        assert_eq!(normalize_tool_call_arguments("read", ""), "{}");
    }

    #[test]
    fn whitespace_arguments_normalize_to_empty_object() {
        assert_eq!(normalize_tool_call_arguments("read", " \n\t "), "{}");
    }

    #[test]
    fn valid_json_arguments_pass_through_unchanged() {
        let arguments = "{\"b\": 1,  \"a\":\t\"x\"}";
        let normalized = normalize_tool_call_arguments("read", arguments);
        assert_eq!(normalized, arguments);
        assert!(std::ptr::eq(normalized, arguments));
    }

    #[test]
    fn malformed_arguments_normalize_to_empty_object() {
        assert_eq!(normalize_tool_call_arguments("read", "{\"path\":"), "{}");
    }
}
