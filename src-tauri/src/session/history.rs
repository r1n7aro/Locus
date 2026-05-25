use std::collections::{HashMap, HashSet};

use super::models::{ChatMessage, MessageRole, ToolCallInfo};
use crate::commands::ToolCallOutcome;

pub const INTERRUPTED_TOOL_RESULT: &str = "工具执行被用户中止，未返回结果。";

pub fn render_prompt_content(content: &str, prefix: Option<&str>, suffix: Option<&str>) -> String {
    let mut rendered = String::new();
    if let Some(prefix) = prefix.filter(|value| !value.is_empty()) {
        rendered.push_str(prefix);
    }
    rendered.push_str(content);
    if let Some(suffix) = suffix.filter(|value| !value.is_empty()) {
        rendered.push_str(suffix);
    }
    rendered
}

pub fn materialize_prompt_edits(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            if message.prompt_prefix.is_some() || message.prompt_suffix.is_some() {
                message.content = render_prompt_content(
                    &message.content,
                    message.prompt_prefix.as_deref(),
                    message.prompt_suffix.as_deref(),
                );
            }
            message.prompt_prefix = None;
            message.prompt_suffix = None;
            message
        })
        .collect()
}

pub fn collect_assistant_tool_calls(messages: &[ChatMessage]) -> Vec<ToolCallInfo> {
    messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .flat_map(|message| message.tool_calls.clone().unwrap_or_default())
        .collect()
}

pub fn normalize_tool_round_history(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut normalized = Vec::with_capacity(messages.len());
    let mut i = 0usize;

    while i < messages.len() {
        let msg = &messages[i];
        if msg.role == MessageRole::Tool {
            i += 1;
            continue;
        }
        if msg.role != MessageRole::Assistant {
            normalized.push(msg.clone());
            i += 1;
            continue;
        }

        let expected_tool_call_ids: Vec<String> = msg
            .tool_calls
            .as_ref()
            .into_iter()
            .flat_map(|calls| calls.iter())
            .filter(|call| !call.is_server_tool() && !call.id.is_empty())
            .map(|call| call.id.clone())
            .collect();
        let expected_tool_call_set: HashSet<String> =
            expected_tool_call_ids.iter().cloned().collect();

        i += 1;

        let mut present_tool_ids: HashSet<String> = HashSet::new();
        let mut observed_tool_outputs: HashMap<String, String> = HashMap::new();
        let mut following_tool_messages: Vec<ChatMessage> = Vec::new();
        while i < messages.len() && messages[i].role == MessageRole::Tool {
            if let Some(tool_call_id) = messages[i].tool_call_id.as_deref() {
                if !tool_call_id.is_empty()
                    && expected_tool_call_set.contains(tool_call_id)
                    && !present_tool_ids.contains(tool_call_id)
                {
                    present_tool_ids.insert(tool_call_id.to_string());
                    observed_tool_outputs
                        .entry(tool_call_id.to_string())
                        .or_insert_with(|| messages[i].content.clone());
                    following_tool_messages.push(messages[i].clone());
                }
            }
            i += 1;
        }

        let has_following_message = i < messages.len();
        let interrupted_tool_ids: HashSet<String> = if has_following_message {
            expected_tool_call_ids
                .iter()
                .filter(|tool_call_id| !present_tool_ids.contains(tool_call_id.as_str()))
                .cloned()
                .collect()
        } else {
            HashSet::new()
        };

        let mut assistant_message = msg.clone();
        if let Some(tool_calls) = assistant_message.tool_calls.as_ref() {
            assistant_message.tool_calls = Some(enrich_tool_calls(
                tool_calls,
                &observed_tool_outputs,
                &interrupted_tool_ids,
                has_following_message,
            ));
        }

        normalized.push(assistant_message);
        normalized.extend(following_tool_messages);

        if !has_following_message {
            continue;
        }

        for tool_call_id in expected_tool_call_ids {
            if present_tool_ids.contains(tool_call_id.as_str()) {
                continue;
            }
            normalized.push(build_interrupted_tool_result_message(
                msg.created_at,
                &msg.id,
                tool_call_id.as_str(),
            ));
        }
    }

    normalized
}

fn enrich_tool_calls(
    tool_calls: &[ToolCallInfo],
    observed_tool_outputs: &HashMap<String, String>,
    interrupted_tool_ids: &HashSet<String>,
    has_following_message: bool,
) -> Vec<ToolCallInfo> {
    tool_calls
        .iter()
        .map(|tool_call| {
            let mut tool_call = tool_call.clone();

            if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_ref() {
                tool_call.nested_tool_calls = Some(enrich_embedded_tool_calls(nested_tool_calls));
            }

            if let Some(output) = observed_tool_outputs.get(tool_call.id.as_str()) {
                tool_call.recorded_output = Some(output.clone());
            }

            if tool_call.outcome.is_none() {
                tool_call.outcome = infer_top_level_tool_outcome(
                    &tool_call,
                    observed_tool_outputs
                        .get(tool_call.id.as_str())
                        .map(String::as_str),
                    interrupted_tool_ids.contains(tool_call.id.as_str()),
                    has_following_message,
                );
            }

            tool_call
        })
        .collect()
}

fn enrich_embedded_tool_calls(tool_calls: &[ToolCallInfo]) -> Vec<ToolCallInfo> {
    tool_calls
        .iter()
        .map(|tool_call| {
            let mut tool_call = tool_call.clone();
            if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_ref() {
                tool_call.nested_tool_calls = Some(enrich_embedded_tool_calls(nested_tool_calls));
            }
            if tool_call.outcome.is_none() {
                tool_call.outcome = infer_embedded_tool_outcome(&tool_call);
            }
            tool_call
        })
        .collect()
}

fn infer_top_level_tool_outcome(
    tool_call: &ToolCallInfo,
    observed_output: Option<&str>,
    is_interrupted: bool,
    has_following_message: bool,
) -> Option<ToolCallOutcome> {
    if tool_call.is_server_tool() {
        return Some(ToolCallOutcome::Done);
    }
    if is_interrupted {
        return Some(ToolCallOutcome::Interrupted);
    }
    match observed_output {
        Some(INTERRUPTED_TOOL_RESULT) => Some(ToolCallOutcome::Interrupted),
        Some(_) => Some(ToolCallOutcome::Done),
        None if has_following_message => Some(ToolCallOutcome::Interrupted),
        None => None,
    }
}

fn infer_embedded_tool_outcome(tool_call: &ToolCallInfo) -> Option<ToolCallOutcome> {
    if tool_call.is_server_tool() {
        return Some(ToolCallOutcome::Done);
    }
    match tool_call.recorded_output.as_deref() {
        Some(INTERRUPTED_TOOL_RESULT) => Some(ToolCallOutcome::Interrupted),
        Some(_) => Some(ToolCallOutcome::Done),
        None => None,
    }
}

fn build_interrupted_tool_result_message(
    created_at: i64,
    assistant_message_id: &str,
    tool_call_id: &str,
) -> ChatMessage {
    ChatMessage {
        id: synthetic_tool_result_message_id(assistant_message_id, tool_call_id),
        role: MessageRole::Tool,
        content: INTERRUPTED_TOOL_RESULT.to_string(),
        created_at,
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

fn synthetic_tool_result_message_id(assistant_message_id: &str, tool_call_id: &str) -> String {
    format!(
        "synthetic_tool_result:{}:{}",
        assistant_message_id, tool_call_id
    )
}

#[cfg(test)]
mod tests {
    use super::{
        collect_assistant_tool_calls, normalize_tool_round_history, INTERRUPTED_TOOL_RESULT,
    };
    use crate::commands::ToolCallOutcome;
    use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};

    fn assistant_with_tools(id: &str, tool_calls: Vec<ToolCallInfo>) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Assistant,
            content: "assistant".to_string(),
            created_at: 10,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls: Some(tool_calls),
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

    fn user_message(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::User,
            content: content.to_string(),
            created_at: 11,
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

    fn tool_message(id: &str, tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Tool,
            content: content.to_string(),
            created_at: 10,
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
    fn inserts_missing_tool_results_before_following_user_message() {
        let messages = vec![
            assistant_with_tools(
                "assistant-1",
                vec![
                    ToolCallInfo {
                        id: "tc-1".to_string(),
                        name: "read".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "tc-2".to_string(),
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ],
            ),
            user_message("user-1", "继续"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 4);
        assert_eq!(normalized[1].role, MessageRole::Tool);
        assert_eq!(normalized[1].tool_call_id.as_deref(), Some("tc-1"));
        assert_eq!(normalized[1].content, INTERRUPTED_TOOL_RESULT);
        assert_eq!(normalized[2].role, MessageRole::Tool);
        assert_eq!(normalized[2].tool_call_id.as_deref(), Some("tc-2"));
        assert_eq!(normalized[3].role, MessageRole::User);
    }

    #[test]
    fn keeps_existing_tool_results_and_only_fills_missing_ones() {
        let messages = vec![
            assistant_with_tools(
                "assistant-1",
                vec![
                    ToolCallInfo {
                        id: "tc-1".to_string(),
                        name: "read".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "tc-2".to_string(),
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ],
            ),
            tool_message("tool-1", "tc-1", "done"),
            user_message("user-1", "继续"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 4);
        assert_eq!(normalized[1].id, "tool-1");
        assert_eq!(normalized[2].tool_call_id.as_deref(), Some("tc-2"));
        assert_eq!(normalized[2].content, INTERRUPTED_TOOL_RESULT);
    }

    #[test]
    fn drops_tool_results_without_matching_visible_tool_call() {
        let messages = vec![
            tool_message("orphan-start", "tc-orphan-start", "stale start"),
            assistant_with_tools(
                "assistant-1",
                vec![ToolCallInfo {
                    id: "tc-1".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }],
            ),
            tool_message("tool-valid", "tc-1", "valid"),
            tool_message("tool-stale", "tc-stale", "stale"),
            user_message("user-1", "继续"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].id, "assistant-1");
        assert_eq!(normalized[1].id, "tool-valid");
        assert_eq!(normalized[1].tool_call_id.as_deref(), Some("tc-1"));
        assert_eq!(normalized[2].id, "user-1");
    }

    #[test]
    fn drops_duplicate_tool_results_for_the_same_tool_call() {
        let messages = vec![
            assistant_with_tools(
                "assistant-1",
                vec![ToolCallInfo {
                    id: "tc-1".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }],
            ),
            tool_message("tool-first", "tc-1", "first"),
            tool_message("tool-duplicate", "tc-1", "duplicate"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[1].id, "tool-first");
        let tool_calls = normalized[0]
            .tool_calls
            .as_ref()
            .expect("assistant tool calls");
        assert_eq!(tool_calls[0].recorded_output.as_deref(), Some("first"));
    }

    #[test]
    fn skips_server_tool_outputs_when_filling_missing_results() {
        let messages = vec![
            assistant_with_tools(
                "assistant-1",
                vec![
                    ToolCallInfo {
                        id: "tc-server".to_string(),
                        name: "web_search".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: Some(crate::session::models::ServerToolKind::WebSearch),
                        server_tool_output: Some("cached".to_string()),
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "tc-local".to_string(),
                        name: "read".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ],
            ),
            user_message("user-1", "继续"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[1].tool_call_id.as_deref(), Some("tc-local"));
    }

    #[test]
    fn keeps_trailing_tool_round_pending_without_synthetic_interrupt() {
        let messages = vec![assistant_with_tools(
            "assistant-1",
            vec![ToolCallInfo {
                id: "tc-1".to_string(),
                name: "task".to_string(),
                arguments: "{}".to_string(),
                order: None,
                server_tool: None,
                server_tool_output: None,
                outcome: None,
                recorded_output: None,
                nested_tool_calls: None,
            }],
        )];

        let normalized = normalize_tool_round_history(&messages);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].role, MessageRole::Assistant);
    }

    #[test]
    fn annotates_tool_outcomes_and_transient_outputs_for_history_display() {
        let messages = vec![
            assistant_with_tools(
                "assistant-1",
                vec![
                    ToolCallInfo {
                        id: "tc-done".to_string(),
                        name: "read".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                    ToolCallInfo {
                        id: "tc-missing".to_string(),
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                        order: None,
                        server_tool: None,
                        server_tool_output: None,
                        outcome: None,
                        recorded_output: None,
                        nested_tool_calls: None,
                    },
                ],
            ),
            tool_message("tool-1", "tc-done", "done output"),
            user_message("user-1", "继续"),
        ];

        let normalized = normalize_tool_round_history(&messages);
        let tool_calls = normalized[0]
            .tool_calls
            .as_ref()
            .expect("assistant tool calls");
        assert_eq!(tool_calls[0].outcome, Some(ToolCallOutcome::Done));
        assert_eq!(
            tool_calls[0].recorded_output.as_deref(),
            Some("done output")
        );
        assert_eq!(tool_calls[1].outcome, Some(ToolCallOutcome::Interrupted));
    }

    #[test]
    fn collects_assistant_tool_calls_in_message_order() {
        let messages = normalize_tool_round_history(&[
            assistant_with_tools(
                "assistant-1",
                vec![ToolCallInfo {
                    id: "tc-1".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }],
            ),
            tool_message("tool-1", "tc-1", "done"),
            assistant_with_tools(
                "assistant-2",
                vec![ToolCallInfo {
                    id: "tc-2".to_string(),
                    name: "grep".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }],
            ),
        ]);

        let collected = collect_assistant_tool_calls(&messages);
        assert_eq!(
            collected
                .iter()
                .map(|tool_call| tool_call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["tc-1", "tc-2"]
        );
        assert_eq!(collected[0].recorded_output.as_deref(), Some("done"));
    }
}
