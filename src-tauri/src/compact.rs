use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const AUTO_COMPACT_THRESHOLD: f64 = 0.9;
const AUTO_COMPACT_BUFFER_MIN_TOKENS: u32 = 4_000;
const AUTO_COMPACT_BUFFER_MAX_TOKENS: u32 = 24_000;

// The byte heuristic in `estimate_text_tokens` undercounts CJK and dense JSON
// by 15-35%, so real usage feedback may only raise the estimate, never lower it.
const ESTIMATE_CALIBRATION_RATIO_MIN: f64 = 1.0;
const ESTIMATE_CALIBRATION_RATIO_MAX: f64 = 4.0;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

const MESSAGE_OVERHEAD_TOKENS: u32 = 12;
const TOOL_CALL_OVERHEAD_TOKENS: u32 = 24;
const IMAGE_TOKEN_COST: u32 = 2_000;
const TOOL_SCHEMA_OVERHEAD_TOKENS: u32 = 32;
const APPROX_BYTES_PER_TOKEN: usize = 4;

// Kept for backwards compatibility with previously persisted sessions.
pub const CLEARED_TOOL_RESULT: &str = "[Old tool result content cleared]";
const PERSISTED_OUTPUT_OPEN: &str = "<persisted-output>";

const ANALYSIS_OPEN: &str = "<analysis>";
const ANALYSIS_CLOSE: &str = "</analysis>";
const SUMMARY_OPEN: &str = "<summary>";
const SUMMARY_CLOSE: &str = "</summary>";

const POST_COMPACT_MAX_FILES_TO_RESTORE: usize = 4;
const POST_COMPACT_MAX_TOKENS_PER_FILE: u32 = 1_200;
const POST_COMPACT_TOTAL_FILE_TOKEN_BUDGET: u32 = 4_000;
const COMPACT_REQUEST_BUDGET_MAX_TOKENS: u32 = 150_000;
const COMPACT_REQUEST_BUDGET_MIN_TOKENS: u32 = 32_000;
const COMPACT_RECENT_TAIL_MIN_TOKENS: u32 = 20_000;
const COMPACT_RECENT_TAIL_MAX_TOKENS: u32 = 40_000;
pub const COMPACT_USER_MESSAGE_MAX_TOKENS: u32 = 20_000;
const COMPACT_MAX_USER_MESSAGE_TOKENS: u32 = 2_500;
const COMPACT_MAX_ASSISTANT_MESSAGE_TOKENS: u32 = 1_600;
const COMPACT_MAX_TOOL_OUTPUT_TOKENS: u32 = 900;
const COMPACT_MAX_TOOL_ARGUMENT_TOKENS: u32 = 500;
const EMERGENCY_SUMMARY_MAX_ITEMS: usize = 12;

const COMPACT_PROMPT: &str = r#"CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.

You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Important rules:
- Do NOT call Read, Bash, Grep, List, Edit, Write, or any other tool.
- Prefer the user's working language when it is obvious from the conversation.
- Treat this as handoff context, not as a user-facing status update.
- Be concise, structured, and focused on helping the next LLM continue the work.
- Your final answer must contain exactly two plain-text blocks:
  1. <analysis>...</analysis>
  2. <summary>...</summary>

In <analysis>, identify the durable context worth carrying forward:
- Current progress and key decisions made
- Important constraints, user preferences, and project conventions
- Files, commands, errors, and technical details needed to continue
- What remains to be done and the immediate next step

In <summary>, write only the compact handoff. Include:
- Primary request and current intent
- Current progress and decisions
- Important constraints and references
- Files and code areas that matter
- Open issues, risks, and next steps

Recent user messages may remain after this handoff. If any retained raw user message conflicts with the summary, prefer the raw user message.
"#;

#[derive(Debug, Clone, Copy)]
enum RestorableToolKind {
    Read,
    UnityYamlList,
    UnityYamlSearch,
    UnityYamlRead,
}

#[derive(Debug, Clone)]
struct RestorableToolRequest {
    kind: RestorableToolKind,
    file_path: String,
    offset: usize,
    limit: usize,
    object_path: Option<String>,
    detail: Option<String>,
    summary_options: crate::unity_yaml::HierarchySummaryOptions,
    search_options: crate::unity_yaml::HierarchySearchOptions,
}

#[derive(Debug, Clone)]
struct RestoredFileContext {
    display_path: String,
    content: String,
    source_note: String,
}

pub struct CompactTracker {
    pub consecutive_failures: u32,
    pub compacted: bool,
}

#[derive(Debug, Clone)]
pub struct BudgetedCompactRequest {
    pub messages: Vec<ChatMessage>,
    pub boundary_idx: usize,
    pub estimated_tokens: u32,
    pub budget_tokens: u32,
    pub truncated: bool,
}

impl CompactTracker {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            compacted: false,
        }
    }

    pub fn is_circuit_broken(&self) -> bool {
        self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.compacted = true;
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            eprintln!(
                "[Compact] circuit breaker tripped after {} consecutive failures",
                self.consecutive_failures
            );
        }
    }
}

pub fn should_auto_compact(total_input_tokens: u32, context_limit: u32) -> bool {
    if context_limit == 0 {
        return false;
    }
    let threshold = (context_limit as f64 * AUTO_COMPACT_THRESHOLD) as u32;
    total_input_tokens.saturating_add(auto_compact_buffer(context_limit)) >= threshold
}

pub fn should_codex_auto_compact(total_input_tokens: u32, context_limit: u32) -> bool {
    if context_limit == 0 {
        return false;
    }
    let auto_compact_limit = context_limit.saturating_mul(9) / 10;
    total_input_tokens >= auto_compact_limit
}

pub fn should_codex_block_normal_send(total_input_tokens: u32, context_limit: u32) -> bool {
    if context_limit == 0 {
        return false;
    }
    total_input_tokens >= context_limit.saturating_sub(24_000)
}

pub const CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE: &str =
    "Output exceeded the available model context and was truncated";

/// Codex-style pre-compaction trim, mirroring codex-rs
/// `trim_function_call_history_to_fit_context_window`: walk from the newest
/// message backwards, rewriting tool outputs to a short placeholder until the
/// estimated request fits the context window. Stops at the first non-tool
/// message because everything older was already accepted by the provider in
/// the last successful request.
pub fn trim_tool_outputs_to_fit_context_window(
    messages: &mut [ChatMessage],
    system_parts: &[&str],
    tools: &[serde_json::Value],
    context_limit: u32,
) -> usize {
    if context_limit == 0 || messages.is_empty() {
        return 0;
    }

    let mut rewritten = 0usize;
    for index in (0..messages.len()).rev() {
        if estimate_request_tokens(system_parts, messages, tools) <= context_limit {
            break;
        }
        let message = &mut messages[index];
        if message.role != MessageRole::Tool {
            break;
        }
        if message.content == CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE {
            continue;
        }
        message.content = CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE.to_string();
        rewritten += 1;
    }

    rewritten
}

/// Blend the local byte-heuristic estimate with real usage reported by the
/// provider so auto-compact triggers on the provider's tokenization, not ours.
///
/// `calibration_sample` is `(actual_prompt_tokens, estimated_tokens)` for the
/// most recent completed request in this run, where actual is
/// `input + cache_read + cache_write` (every backend normalizes `input_tokens`
/// to the uncached portion, so the sum is the full prompt without double
/// counting). `persisted_context_tokens` is the session's `last_context_tokens`
/// and covers the first request of a new run, before any in-run sample exists;
/// it is capped relative to the estimate so a stale value recorded before
/// messages were rolled back cannot trigger spurious compaction.
pub fn calibrated_input_tokens(
    estimated_tokens: u32,
    calibration_sample: Option<(u32, u32)>,
    persisted_context_tokens: u32,
) -> u32 {
    let ratio = calibration_sample
        .filter(|(actual, estimated)| *actual > 0 && *estimated > 0)
        .map(|(actual, estimated)| actual as f64 / estimated as f64)
        .unwrap_or(1.0)
        .clamp(
            ESTIMATE_CALIBRATION_RATIO_MIN,
            ESTIMATE_CALIBRATION_RATIO_MAX,
        );

    let calibrated = (estimated_tokens as f64 * ratio).min(u32::MAX as f64) as u32;
    let floor_cap =
        (estimated_tokens as f64 * ESTIMATE_CALIBRATION_RATIO_MAX).min(u32::MAX as f64) as u32;
    let floor = persisted_context_tokens.min(floor_cap);

    calibrated.max(floor).max(estimated_tokens)
}

fn auto_compact_buffer(context_limit: u32) -> u32 {
    (context_limit / 20).clamp(
        AUTO_COMPACT_BUFFER_MIN_TOKENS,
        AUTO_COMPACT_BUFFER_MAX_TOKENS,
    )
}

fn estimate_text_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }

    text.len()
        .saturating_add(APPROX_BYTES_PER_TOKEN.saturating_sub(1))
        .checked_div(APPROX_BYTES_PER_TOKEN)
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX)
}

fn is_real_user_message(message: &ChatMessage) -> bool {
    message.role == MessageRole::User && message.tool_call_id.is_none()
}

fn estimate_tool_call_prompt_tokens(tool_call: &ToolCallInfo) -> u32 {
    // Match provider payload builders: nested subagent history is display metadata,
    // and local tool output is replayed through Tool messages rather than recorded_output.
    TOOL_CALL_OVERHEAD_TOKENS
        .saturating_add(estimate_text_tokens(&tool_call.name))
        .saturating_add(estimate_text_tokens(&tool_call.arguments))
        .saturating_add(estimate_text_tokens(
            tool_call.server_tool_output.as_deref().unwrap_or_default(),
        ))
}

fn estimate_message_prompt_tokens(message: &ChatMessage) -> u32 {
    let mut total = MESSAGE_OVERHEAD_TOKENS
        .saturating_add(estimate_text_tokens(&message.content))
        .saturating_add(estimate_text_tokens(
            message.prompt_prefix.as_deref().unwrap_or_default(),
        ))
        .saturating_add(estimate_text_tokens(
            message.prompt_suffix.as_deref().unwrap_or_default(),
        ));

    if let Some(ref thinking) = message.thinking_content {
        total = total.saturating_add(estimate_text_tokens(thinking));
    }

    if let Some(ref images) = message.images {
        total = total.saturating_add(images.len() as u32 * IMAGE_TOKEN_COST);
    }

    if let Some(ref tool_calls) = message.tool_calls {
        for tc in tool_calls {
            total = total.saturating_add(estimate_tool_call_prompt_tokens(tc));
        }
    }

    total
}

pub fn compact_user_message_token_budget() -> u32 {
    COMPACT_USER_MESSAGE_MAX_TOKENS
}

pub fn select_recent_user_message_ids_for_compact_prompt(
    messages: &[ChatMessage],
    boundary_idx: usize,
    max_tokens: u32,
) -> HashSet<String> {
    let mut selected = HashSet::new();
    if max_tokens == 0 || messages.is_empty() {
        return selected;
    }

    let mut used_tokens = 0u32;
    for message in messages.iter().take(boundary_idx.min(messages.len())).rev() {
        if !is_real_user_message(message) {
            continue;
        }

        let message_tokens = estimate_message_prompt_tokens(message);
        if used_tokens.saturating_add(message_tokens) > max_tokens {
            break;
        }

        used_tokens = used_tokens.saturating_add(message_tokens);
        selected.insert(message.id.clone());
    }

    selected
}

pub fn has_compactable_messages_before_boundary(
    messages: &[ChatMessage],
    boundary_idx: usize,
) -> bool {
    let mut user_tokens = 0u32;
    for message in messages.iter().take(boundary_idx.min(messages.len())) {
        if !is_real_user_message(message) {
            return true;
        }

        user_tokens = user_tokens.saturating_add(estimate_message_prompt_tokens(message));
        if user_tokens > COMPACT_USER_MESSAGE_MAX_TOKENS {
            return true;
        }
    }

    false
}

fn is_persisted_output_reference(content: &str) -> bool {
    content.trim_start().starts_with(PERSISTED_OUTPUT_OPEN)
}

pub fn estimate_request_tokens(
    system_parts: &[&str],
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> u32 {
    let mut total = 0u32;

    for part in system_parts {
        total = total
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
            .saturating_add(estimate_text_tokens(part));
    }

    for msg in messages {
        total = total.saturating_add(estimate_message_prompt_tokens(msg));
    }

    for tool in tools {
        let serialized = serde_json::to_string(tool).unwrap_or_default();
        total = total
            .saturating_add(TOOL_SCHEMA_OVERHEAD_TOKENS)
            .saturating_add(estimate_text_tokens(&serialized));
    }

    total
}

pub fn prepare_messages_for_llm(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let normalized = crate::session::history::normalize_tool_round_history(messages);
    crate::session::history::materialize_prompt_edits(&normalized)
}

#[allow(dead_code)]
pub fn build_compact_request(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut compact_messages = prepare_messages_for_llm(messages);

    compact_messages.push(ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: COMPACT_PROMPT.to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
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
    });

    compact_messages
}

pub fn compact_request_token_budget(context_limit: u32) -> u32 {
    if context_limit == 0 {
        return COMPACT_REQUEST_BUDGET_MIN_TOKENS;
    }
    (context_limit.saturating_mul(3) / 5).clamp(
        COMPACT_REQUEST_BUDGET_MIN_TOKENS,
        COMPACT_REQUEST_BUDGET_MAX_TOKENS,
    )
}

pub fn compact_recent_tail_token_budget(context_limit: u32) -> u32 {
    if context_limit == 0 {
        return COMPACT_RECENT_TAIL_MIN_TOKENS;
    }
    (context_limit / 8).clamp(
        COMPACT_RECENT_TAIL_MIN_TOKENS,
        COMPACT_RECENT_TAIL_MAX_TOKENS,
    )
}

fn truncate_to_token_budget(content: &str, max_tokens: u32) -> (String, bool) {
    let max_chars = max_tokens.max(1) as usize * 4;
    if content.chars().count() <= max_chars {
        return (content.to_string(), false);
    }
    let truncated: String = content.chars().take(max_chars).collect();
    (
        format!(
            "{}\n\n[truncated for compact request: {} chars total]",
            truncated.trim_end(),
            content.chars().count()
        ),
        true,
    )
}

fn sanitize_tool_call_for_compact(tool_call: &ToolCallInfo) -> (ToolCallInfo, bool) {
    let mut sanitized = tool_call.clone();
    let mut truncated = false;
    let (arguments, arguments_truncated) =
        truncate_to_token_budget(&sanitized.arguments, COMPACT_MAX_TOOL_ARGUMENT_TOKENS);
    sanitized.arguments = arguments;
    truncated |= arguments_truncated;
    sanitized.recorded_output = None;

    if let Some(output) = sanitized.server_tool_output.take() {
        let (output, output_truncated) =
            truncate_to_token_budget(&output, COMPACT_MAX_TOOL_OUTPUT_TOKENS);
        sanitized.server_tool_output = Some(output);
        truncated |= output_truncated;
    }

    if let Some(nested) = sanitized.nested_tool_calls.take() {
        let mut nested_truncated = false;
        let nested = nested
            .iter()
            .map(|call| {
                let (call, was_truncated) = sanitize_tool_call_for_compact(call);
                nested_truncated |= was_truncated;
                call
            })
            .collect();
        sanitized.nested_tool_calls = Some(nested);
        truncated |= nested_truncated;
    }

    (sanitized, truncated)
}

fn sanitize_message_for_compact(message: &ChatMessage) -> (ChatMessage, bool) {
    let mut sanitized = message.clone();
    let mut truncated = false;

    let max_tokens = match sanitized.role {
        MessageRole::User => COMPACT_MAX_USER_MESSAGE_TOKENS,
        MessageRole::Assistant => COMPACT_MAX_ASSISTANT_MESSAGE_TOKENS,
        MessageRole::Tool => COMPACT_MAX_TOOL_OUTPUT_TOKENS,
    };
    let (content, content_truncated) = truncate_to_token_budget(&sanitized.content, max_tokens);
    sanitized.content = if sanitized.role == MessageRole::Tool
        && !is_persisted_output_reference(&sanitized.content)
    {
        if content_truncated {
            format!(
                "{}\n\n[full tool output omitted from compact request; rely on persisted references or rerun/read if needed]",
                content.trim_end()
            )
        } else {
            content
        }
    } else {
        content
    };
    truncated |= content_truncated;

    if sanitized.images.take().is_some() {
        sanitized.content = format!(
            "{}\n\n[images omitted from compact request]",
            sanitized.content.trim_end()
        );
        truncated = true;
    }
    if sanitized.thinking_content.take().is_some() {
        truncated = true;
    }
    sanitized.thinking_duration = None;
    sanitized.thinking_signature = None;

    if let Some(tool_calls) = sanitized.tool_calls.take() {
        let mut tool_truncated = false;
        sanitized.tool_calls = Some(
            tool_calls
                .iter()
                .map(|call| {
                    let (call, was_truncated) = sanitize_tool_call_for_compact(call);
                    tool_truncated |= was_truncated;
                    call
                })
                .collect(),
        );
        truncated |= tool_truncated;
    }

    (sanitized, truncated)
}

fn build_omitted_messages_marker(count: usize, created_at: i64) -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: format!(
            "[{} older assistant/tool message(s) omitted from compact request to stay within budget. Preserve durable facts from the visible summary and recent raw tail.]",
            count
        ),
        created_at,
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

fn prune_tool_results_without_visible_calls(messages: &mut Vec<ChatMessage>) -> usize {
    let before = messages.len();
    let mut visible_tool_call_ids: HashSet<String> = HashSet::new();

    messages.retain(|message| match message.role {
        MessageRole::Assistant => {
            if let Some(tool_calls) = message.tool_calls.as_ref() {
                for tool_call in tool_calls {
                    if !tool_call.id.is_empty() {
                        visible_tool_call_ids.insert(tool_call.id.clone());
                    }
                }
            }
            true
        }
        MessageRole::Tool => message
            .tool_call_id
            .as_deref()
            .filter(|tool_call_id| !tool_call_id.is_empty())
            .map(|tool_call_id| visible_tool_call_ids.contains(tool_call_id))
            .unwrap_or(false),
        MessageRole::User => true,
    });

    before.saturating_sub(messages.len())
}

pub fn find_compact_boundary_by_budget(
    messages: &[ChatMessage],
    target_recent_tokens: u32,
) -> usize {
    if messages.len() <= 1 {
        return 0;
    }

    let mut total = 0u32;
    let mut boundary = messages.len().saturating_sub(1);
    let mut reached_target = false;
    for (idx, message) in messages.iter().enumerate().rev() {
        total = total.saturating_add(estimate_message_prompt_tokens(message));
        boundary = idx;
        if total >= target_recent_tokens {
            reached_target = true;
            break;
        }
    }

    if boundary == 0 && reached_target && messages.len() > 1 {
        boundary = 1;
    }

    while boundary > 0 && messages[boundary].role == MessageRole::Tool {
        boundary -= 1;
    }

    boundary.min(messages.len().saturating_sub(1))
}

pub fn build_compact_request_with_budget(
    messages: &[ChatMessage],
    system_parts: &[&str],
    context_limit: u32,
) -> Result<BudgetedCompactRequest, String> {
    if messages.is_empty() {
        return Err("Cannot compact an empty message history".to_string());
    }

    let boundary_idx =
        find_compact_boundary_by_budget(messages, compact_recent_tail_token_budget(context_limit));
    let budget_tokens = compact_request_token_budget(context_limit);
    let prepared = prepare_messages_for_llm(messages);
    let mut truncated = false;
    let mut compact_messages: Vec<ChatMessage> = prepared
        .iter()
        .map(|message| {
            let (message, was_truncated) = sanitize_message_for_compact(message);
            truncated |= was_truncated;
            message
        })
        .collect();
    if prune_tool_results_without_visible_calls(&mut compact_messages) > 0 {
        truncated = true;
    }

    compact_messages.push(ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: COMPACT_PROMPT.to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
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
    });

    let mut estimated_tokens = estimate_request_tokens(system_parts, &compact_messages, &[]);
    if estimated_tokens <= budget_tokens {
        return Ok(BudgetedCompactRequest {
            messages: compact_messages,
            boundary_idx,
            estimated_tokens,
            budget_tokens,
            truncated,
        });
    }

    let prompt = compact_messages
        .pop()
        .expect("compact prompt should have been appended");
    let first_user = compact_messages
        .iter()
        .position(|message| message.role == MessageRole::User && message.tool_call_id.is_none());
    let mut reduced = Vec::new();
    if let Some(first_user_idx) = first_user {
        reduced.push(compact_messages[first_user_idx].clone());
    }

    let mut tail = Vec::new();
    let used_without_tail = estimate_request_tokens(system_parts, &reduced, &[])
        .saturating_add(MESSAGE_OVERHEAD_TOKENS)
        .saturating_add(estimate_text_tokens(&prompt.content));
    let tail_budget = budget_tokens.saturating_sub(used_without_tail).max(4_000);
    let mut tail_tokens = 0u32;
    for (idx, message) in compact_messages.iter().enumerate().rev() {
        if Some(idx) == first_user {
            continue;
        }
        let candidate_tokens = estimate_request_tokens(&[], std::slice::from_ref(message), &[]);
        if !tail.is_empty() && tail_tokens.saturating_add(candidate_tokens) > tail_budget {
            break;
        }
        tail_tokens = tail_tokens.saturating_add(candidate_tokens);
        tail.push(message.clone());
    }
    tail.reverse();

    let marker_insert_idx = reduced.len();
    reduced.extend(tail);
    prune_tool_results_without_visible_calls(&mut reduced);

    let omitted = compact_messages.len().saturating_sub(reduced.len());
    if omitted > 0 {
        let marker_ts = reduced
            .get(marker_insert_idx)
            .or_else(|| reduced.first())
            .map(|message| message.created_at.saturating_sub(1))
            .unwrap_or_default();
        reduced.insert(
            marker_insert_idx.min(reduced.len()),
            build_omitted_messages_marker(omitted, marker_ts),
        );
    }
    reduced.push(prompt);

    estimated_tokens = estimate_request_tokens(system_parts, &reduced, &[]);
    if estimated_tokens > budget_tokens {
        return Err(format!(
            "Budgeted compact request still exceeds budget: estimated={} budget={}",
            estimated_tokens, budget_tokens
        ));
    }

    Ok(BudgetedCompactRequest {
        messages: reduced,
        boundary_idx,
        estimated_tokens,
        budget_tokens,
        truncated: true,
    })
}

fn strip_tag_block(input: &str, open: &str, close: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    loop {
        let Some(start) = rest.find(open) else {
            output.push_str(rest);
            break;
        };

        output.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        let Some(end) = after_open.find(close) else {
            // Tag opened but never closed: keep the remainder so later summary
            // content or trailing text is not lost.
            output.push_str(after_open);
            break;
        };
        rest = &after_open[end + close.len()..];
    }

    output
}

fn extract_tag_contents(input: &str, open: &str, close: &str) -> Option<String> {
    let start = input.find(open)?;
    let after_open = &input[start + open.len()..];
    let end = after_open.find(close)?;
    Some(after_open[..end].trim().to_string())
}

pub fn extract_summary(raw_response: &str) -> String {
    let without_analysis = strip_tag_block(raw_response.trim(), ANALYSIS_OPEN, ANALYSIS_CLOSE);
    let extracted = extract_tag_contents(&without_analysis, SUMMARY_OPEN, SUMMARY_CLOSE)
        .unwrap_or_else(|| without_analysis.trim().to_string());

    extracted
        .replace(SUMMARY_OPEN, "")
        .replace(SUMMARY_CLOSE, "")
        .replace("\r\n", "\n")
        .trim()
        .to_string()
}

pub fn is_valid_compact_summary(summary: &str) -> bool {
    let trimmed = summary.trim();
    if trimmed.len() < 64 {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "..." | "…" | "(no response content)" | "no response content" | "empty" | "n/a"
    ) {
        return false;
    }

    let meaningful_chars = trimmed.chars().filter(|ch| ch.is_alphanumeric()).count();
    meaningful_chars >= 24
}

fn compact_line(value: &str, max_tokens: u32) -> String {
    let (value, truncated) = truncate_to_token_budget(value.trim(), max_tokens);
    let value = value.replace('\r', "").replace('\n', " ");
    if truncated {
        value
    } else {
        value.trim().to_string()
    }
}

pub fn build_emergency_compact_summary(
    messages: &[ChatMessage],
    boundary_idx: usize,
    reason: &str,
) -> String {
    let compacted_prefix = &messages[..boundary_idx.min(messages.len())];
    let recent_tail = &messages[boundary_idx.min(messages.len())..];

    let mut user_messages = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.role == MessageRole::User && message.tool_call_id.is_none())
    {
        user_messages.push(compact_line(&message.content, 600));
        if user_messages.len() >= EMERGENCY_SUMMARY_MAX_ITEMS {
            break;
        }
    }

    let mut assistant_notes = Vec::new();
    for message in compacted_prefix.iter().rev().filter(|message| {
        message.role == MessageRole::Assistant && !message.content.trim().is_empty()
    }) {
        assistant_notes.push(compact_line(&message.content, 500));
        if assistant_notes.len() >= 4 {
            break;
        }
    }
    assistant_notes.reverse();

    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut large_outputs = Vec::new();
    for message in compacted_prefix {
        if let Some(tool_calls) = message.tool_calls.as_ref() {
            for tool_call in tool_calls {
                *tool_counts.entry(tool_call.name.clone()).or_default() += 1;
            }
        }
        if message.role == MessageRole::Tool {
            if is_persisted_output_reference(&message.content) {
                large_outputs.push(compact_line(&message.content, 350));
            } else if estimate_text_tokens(&message.content) > COMPACT_MAX_TOOL_OUTPUT_TOKENS {
                large_outputs.push(format!(
                    "tool_call_id={} large output omitted ({} chars)",
                    message.tool_call_id.as_deref().unwrap_or("unknown"),
                    message.content.chars().count()
                ));
            }
        }
        if large_outputs.len() >= 6 {
            break;
        }
    }

    let mut tool_summary: Vec<String> = tool_counts
        .into_iter()
        .map(|(name, count)| format!("{} x{}", name, count))
        .collect();
    tool_summary.sort();

    let latest_user = messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User && message.tool_call_id.is_none())
        .map(|message| compact_line(&message.content, 700))
        .unwrap_or_else(|| "No explicit user message found.".to_string());

    let recent_tail_summary = recent_tail
        .iter()
        .take(8)
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            format!("- {}: {}", role, compact_line(&message.content, 260))
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "1. Primary Request and Intent\n{}\n\n2. All User Messages\n{}\n\n3. Current State of the Work\nEmergency local compact was used because the compact LLM request could not be safely sent: {}.\n\n4. Important Technical Context\nRecent raw messages after this handoff remain in the prompt and take precedence over this summary.\n\n5. Files and Code Areas Touched\nNo deterministic file list is available from local emergency compact. Use recent raw tool calls and restored file context below.\n\n6. Recent Decisions and Why They Matter\n{}\n\n7. Open Issues, Risks, or Follow-ups\nThe LLM summary was skipped. Some middle conversation details may be omitted; large tool outputs are represented by persisted references or short previews.\n\n8. Latest User Feedback and Constraints\n{}\n\n9. Immediate Next Step\nContinue from the recent raw tail, using the latest user request and any surviving todo/tool context.\n\nTool Calls Made Before Compact\n{}\n\nLarge Outputs Omitted\n{}\n\nRecent Raw Tail Preview\n{}",
        latest_user,
        if user_messages.is_empty() {
            "- empty".to_string()
        } else {
            user_messages
                .iter()
                .map(|message| format!("- {}", message))
                .collect::<Vec<_>>()
                .join("\n")
        },
        reason,
        if assistant_notes.is_empty() {
            "- empty".to_string()
        } else {
            assistant_notes
                .iter()
                .map(|note| format!("- {}", note))
                .collect::<Vec<_>>()
                .join("\n")
        },
        latest_user,
        if tool_summary.is_empty() {
            "- empty".to_string()
        } else {
            tool_summary
                .iter()
                .map(|entry| format!("- {}", entry))
                .collect::<Vec<_>>()
                .join("\n")
        },
        if large_outputs.is_empty() {
            "- empty".to_string()
        } else {
            large_outputs
                .iter()
                .map(|entry| format!("- {}", entry))
                .collect::<Vec<_>>()
                .join("\n")
        },
        if recent_tail_summary.is_empty() {
            "- empty".to_string()
        } else {
            recent_tail_summary
        }
    )
}

fn parse_restorable_tool_request(tc: &ToolCallInfo) -> Option<RestorableToolRequest> {
    match tc.name.as_str() {
        "read" => parse_read_tool_request(tc),
        "unity_yaml_list" | "unity_yaml_search" | "unity_yaml_read" => {
            parse_unity_yaml_tool_request(tc)
        }
        _ => None,
    }
}

fn parse_read_tool_request(tc: &ToolCallInfo) -> Option<RestorableToolRequest> {
    if tc.name != "read" {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(&tc.arguments).ok()?;
    let file_path = parsed.get("filePath")?.as_str()?.trim();
    if file_path.is_empty() {
        return None;
    }

    let offset = parsed
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
    let limit = parsed
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(2000)
        .clamp(1, 2000) as usize;

    Some(RestorableToolRequest {
        kind: RestorableToolKind::Read,
        file_path: file_path.to_string(),
        offset,
        limit,
        object_path: None,
        detail: None,
        summary_options: crate::unity_yaml::HierarchySummaryOptions::default(),
        search_options: crate::unity_yaml::HierarchySearchOptions::default(),
    })
}

fn parse_unity_yaml_tool_request(tc: &ToolCallInfo) -> Option<RestorableToolRequest> {
    if !matches!(
        tc.name.as_str(),
        "unity_yaml_list" | "unity_yaml_search" | "unity_yaml_read"
    ) {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(&tc.arguments).ok()?;
    let file_path = parsed.get("file_path")?.as_str()?.trim();
    if file_path.is_empty() {
        return None;
    }

    let object_path = parsed
        .get("object_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let detail = parsed
        .get("detail")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    fn positive_usize(parsed: &serde_json::Value, key: &str) -> Option<usize> {
        parsed
            .get(key)
            .and_then(|value| value.as_u64())
            .filter(|value| *value > 0)
            .map(|value| value as usize)
    }

    fn trimmed_string(parsed: &serde_json::Value, key: &str) -> Option<String> {
        parsed
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn push_csv_values(out: &mut Vec<String>, value: &str) {
        out.extend(
            value
                .split([',', '|'])
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(|entry| entry.to_string()),
        );
    }

    let mut component_filters = Vec::new();
    match parsed.get("component_filter") {
        Some(serde_json::Value::String(value)) => {
            push_csv_values(&mut component_filters, value);
        }
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                if let Some(text) = value.as_str() {
                    push_csv_values(&mut component_filters, text);
                }
            }
        }
        _ => {}
    }

    let mut match_fields = Vec::new();
    match parsed.get("match_fields") {
        Some(serde_json::Value::String(value)) => {
            push_csv_values(&mut match_fields, value);
        }
        Some(serde_json::Value::Array(values)) => {
            for value in values {
                if let Some(text) = value.as_str() {
                    push_csv_values(&mut match_fields, text);
                }
            }
        }
        _ => {}
    }

    let kind = match tc.name.as_str() {
        "unity_yaml_list" => RestorableToolKind::UnityYamlList,
        "unity_yaml_search" => RestorableToolKind::UnityYamlSearch,
        "unity_yaml_read" => RestorableToolKind::UnityYamlRead,
        _ => return None,
    };

    Some(RestorableToolRequest {
        kind,
        file_path: file_path.to_string(),
        offset: 1,
        limit: 2000,
        object_path,
        detail,
        summary_options: crate::unity_yaml::HierarchySummaryOptions {
            max_depth: positive_usize(&parsed, "max_depth"),
            max_nodes: positive_usize(&parsed, "max_nodes"),
            query: None,
            component_filters: Vec::new(),
            path_prefix: trimmed_string(&parsed, "path_prefix"),
        },
        search_options: crate::unity_yaml::HierarchySearchOptions {
            query: trimmed_string(&parsed, "query"),
            component_filters,
            match_fields,
            path_prefix: trimmed_string(&parsed, "path_prefix"),
            limit: positive_usize(&parsed, "limit"),
        },
    })
}

fn truncate_for_token_budget(content: &str, max_tokens: u32) -> String {
    let max_chars = (max_tokens as usize).saturating_mul(4);
    if content.chars().count() <= max_chars {
        return content.to_string();
    }

    let truncated: String = content.chars().take(max_chars).collect();
    format!(
        "{}\n\n(Post-compact restored context truncated to fit budget.)",
        truncated.trim_end()
    )
}

fn resolve_read_path(working_dir: &Path, file_path: &str) -> PathBuf {
    let path = Path::new(file_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    }
}

fn dedupe_key_for_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

fn display_path_for_handoff(path: &Path, working_dir: &Path) -> String {
    path.strip_prefix(working_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn load_persisted_tool_output(content: &str) -> Option<String> {
    const PREFIX: &str = "Full output saved to: ";
    if !is_persisted_output_reference(content) {
        return None;
    }

    let path = content
        .lines()
        .find_map(|line| line.split_once(PREFIX).map(|(_, path)| path))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    std::fs::read_to_string(path).ok()
}

fn resolve_prior_tool_output(content: &str) -> Option<String> {
    if content == CLEARED_TOOL_RESULT {
        return None;
    }

    if let Some(persisted) = load_persisted_tool_output(content) {
        return Some(persisted);
    }

    Some(content.to_string())
}

fn prior_tool_result_excerpt(content: &str, kind: &RestorableToolKind) -> Option<String> {
    let raw_output = resolve_prior_tool_output(content)?;
    match kind {
        RestorableToolKind::Read => {
            if !raw_output.trim_start().starts_with("<content>") {
                return None;
            }
            Some(truncate_for_token_budget(
                &raw_output,
                POST_COMPACT_MAX_TOKENS_PER_FILE,
            ))
        }
        RestorableToolKind::UnityYamlList
        | RestorableToolKind::UnityYamlSearch
        | RestorableToolKind::UnityYamlRead => {
            if raw_output.trim().is_empty() {
                return None;
            }
            Some(truncate_for_token_budget(
                &raw_output,
                POST_COMPACT_MAX_TOKENS_PER_FILE,
            ))
        }
    }
}

fn read_current_file_excerpt(
    resolved_path: &Path,
    offset: usize,
    limit: usize,
    display_path: &str,
) -> Option<String> {
    let metadata = std::fs::metadata(resolved_path).ok()?;
    if !metadata.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(resolved_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = (offset.saturating_sub(1)).min(total);
    let end = (start + limit).min(total);
    let selected = &lines[start..end];

    let max_chars = (POST_COMPACT_MAX_TOKENS_PER_FILE as usize).saturating_mul(4);
    let mut result_lines = Vec::new();
    let mut used_chars = 0usize;
    let mut truncated = false;

    for line in selected {
        let line_len = line.chars().count();
        if used_chars + line_len + 1 > max_chars {
            truncated = true;
            break;
        }
        result_lines.push((*line).to_string());
        used_chars += line_len + 1;
    }

    if result_lines.is_empty() {
        return None;
    }

    let shown_end = start + result_lines.len();
    let continuation = if truncated || end < total {
        format!(
            "(Recovered post-compact from current file state for {}. Showing lines {}-{} of {}.)",
            display_path, offset, shown_end, total
        )
    } else {
        format!(
            "(Recovered post-compact from current file state for {}. End of file, {} lines total.)",
            display_path, total
        )
    };

    Some(format!(
        "<content>\n{}\n\n{}\n</content>",
        result_lines.join("\n"),
        continuation
    ))
}

fn read_current_unity_yaml_excerpt(
    resolved_path: &Path,
    request: &RestorableToolRequest,
    display_path: &str,
) -> Option<String> {
    let metadata = std::fs::metadata(resolved_path).ok()?;
    if !metadata.is_file() {
        return None;
    }

    let content = std::fs::read(resolved_path).ok()?;
    let header = String::from_utf8_lossy(&content[..content.len().min(128)]);
    if !header.contains("%YAML") && !header.contains("!u!") && !header.contains("--- !u!") {
        let text = String::from_utf8_lossy(&content);
        let lines: Vec<&str> = text.lines().collect();
        let mut output = String::new();
        for (i, line) in lines.iter().take(2000).enumerate() {
            output.push_str(&format!("{:>5}\t{}\n", i + 1, line));
        }
        if lines.len() > 2000 {
            output.push_str(&format!("... ({} more lines)\n", lines.len() - 2000));
        }
        return Some(truncate_for_token_budget(
            output.trim_end(),
            POST_COMPACT_MAX_TOKENS_PER_FILE,
        ));
    }

    let docs = crate::unity_yaml::parse_yaml_docs(&content);
    let text = String::from_utf8_lossy(&content);
    let lines: Vec<&str> = text.lines().collect();
    let ext = resolved_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let is_hierarchical = crate::unity_yaml::is_hierarchical_file(&ext);

    if is_hierarchical
        && matches!(
            request.kind,
            RestorableToolKind::UnityYamlList | RestorableToolKind::UnityYamlSearch
        )
    {
        let tree = crate::unity_yaml::build_go_tree(&docs);
        if tree.is_empty() {
            return Some(format!(
                "No GameObjects found in '{}'. The file may be empty or not a scene/prefab.",
                display_path
            ));
        }

        let guid_resolver = |_guid: &crate::asset_db::types::Guid| -> Option<String> { None };
        let output = match request.kind {
            RestorableToolKind::UnityYamlList => {
                crate::unity_yaml::format_scene_summary_with_options(
                    &tree,
                    &docs,
                    &lines,
                    &guid_resolver,
                    display_path,
                    &request.summary_options,
                )
            }
            RestorableToolKind::UnityYamlSearch => {
                crate::unity_yaml::format_hierarchy_search_results(
                    &tree,
                    &docs,
                    &lines,
                    &guid_resolver,
                    display_path,
                    &request.search_options,
                )
            }
            _ => unreachable!(),
        };
        return Some(truncate_for_token_budget(
            &output,
            POST_COMPACT_MAX_TOKENS_PER_FILE,
        ));
    }

    if is_hierarchical && request.object_path.is_none() {
        return Some("unity_yaml_read requires object_path for .unity/.prefab files.".to_string());
    }

    let internal_map = crate::unity_yaml::build_internal_id_map(&docs);
    let internal_resolver = |fid: i64| -> Option<String> { internal_map.get(&fid).cloned() };

    let (output_header, doc_ranges) = if is_hierarchical {
        let object_path = request.object_path.as_deref()?;
        let tree = crate::unity_yaml::build_go_tree(&docs);
        let go_file_id = match crate::unity_yaml::find_go_by_path(&tree, object_path) {
            Some(id) => id,
            None => {
                let roots: Vec<&str> = tree.iter().map(|node| node.name.as_str()).collect();
                return Some(format!(
                    "GameObject '{}' not found in '{}'. Available root objects: {}",
                    object_path,
                    display_path,
                    roots.join(", ")
                ));
            }
        };

        let target_doc_idx = docs.iter().position(|doc| doc.file_id == go_file_id);
        if request.detail.as_deref() == Some("document") {
            let Some(target_doc_idx) = target_doc_idx else {
                return Some(format!(
                    "Target '{}' was found in the hierarchy but its YAML document was unavailable in '{}'.",
                    object_path, display_path
                ));
            };
            (
                format!("Document fields of '{}' ({}):\n", object_path, display_path),
                vec![target_doc_idx],
            )
        } else {
            let is_prefab_instance = docs
                .iter()
                .any(|doc| doc.file_id == go_file_id && doc.class_id == 1001);
            if is_prefab_instance {
                let prefab_instances =
                    crate::unity_yaml::extract_prefab_instance_irs(&docs, &lines);
                if let Some(prefab_instance) = prefab_instances
                    .iter()
                    .find(|instance| instance.local_file_id == go_file_id)
                {
                    let guid_resolver =
                        |_guid: &crate::asset_db::types::Guid| -> Option<String> { None };
                    let stripped = crate::unity_yaml::extract_stripped_mappings(&docs, &lines);
                    let detail = crate::unity_yaml::format_prefab_instance_detail(
                        prefab_instance,
                        &guid_resolver,
                        None,
                        &stripped,
                    );
                    return Some(truncate_for_token_budget(
                        &detail,
                        POST_COMPACT_MAX_TOKENS_PER_FILE,
                    ));
                }
            }

            let component_indices = crate::unity_yaml::get_components_for_go(&docs, go_file_id);
            if component_indices.is_empty() {
                return Some(format!("No components found for '{}'.", object_path));
            }

            (
                format!("Components of '{}' ({}):\n", object_path, display_path),
                component_indices,
            )
        }
    } else {
        (
            format!(
                "Content of '{}' ({} documents):\n",
                display_path,
                docs.len()
            ),
            (0..docs.len()).collect(),
        )
    };

    let guid_resolver = |_hex: &str| -> Option<String> { None };
    let mut output = output_header;
    for idx in doc_ranges {
        let doc = &docs[idx];
        output.push_str(&format!("\n--- {} ---\n", doc.type_name));
        output.push_str(&crate::unity_yaml::format_doc_state_lines(doc));
        let content_start = (doc.line_start + 2).min(doc.line_end);
        let skipped_fields = if doc.m_enabled.is_some() {
            &["m_Enabled"][..]
        } else {
            &[][..]
        };
        let resolved = crate::unity_yaml::resolve_references_in_lines_skipping_fields(
            &lines,
            content_start,
            doc.line_end,
            &guid_resolver,
            &internal_resolver,
            skipped_fields,
        );
        output.push_str(&resolved);
    }

    Some(truncate_for_token_budget(
        &output,
        POST_COMPACT_MAX_TOKENS_PER_FILE,
    ))
}

pub fn build_post_compact_restored_files_section(
    pruned_messages: &[ChatMessage],
    working_dir: &str,
) -> String {
    if pruned_messages.is_empty() || working_dir.trim().is_empty() {
        return String::new();
    }

    let tool_results: HashMap<&str, &str> = pruned_messages
        .iter()
        .filter(|msg| msg.role == MessageRole::Tool)
        .filter_map(|msg| Some((msg.tool_call_id.as_deref()?, msg.content.as_str())))
        .collect();

    let working_dir_path = Path::new(working_dir);
    let mut restored: Vec<RestoredFileContext> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut used_tokens = 0u32;

    'outer: for msg in pruned_messages.iter().rev() {
        if msg.role != MessageRole::Assistant {
            continue;
        }

        let Some(tool_calls) = msg.tool_calls.as_ref() else {
            continue;
        };

        for tc in tool_calls.iter().rev() {
            let Some(request) = parse_restorable_tool_request(tc) else {
                continue;
            };

            let resolved_path = resolve_read_path(working_dir_path, &request.file_path);
            let dedupe_key = dedupe_key_for_path(&resolved_path);
            if !seen_paths.insert(dedupe_key) {
                continue;
            }

            let display_path = display_path_for_handoff(&resolved_path, working_dir_path);
            let (content, source_note) = if let Some(raw_tool_result) =
                tool_results.get(tc.id.as_str())
            {
                if let Some(exact_excerpt) =
                    prior_tool_result_excerpt(raw_tool_result, &request.kind)
                {
                    let source_note = match request.kind {
                        RestorableToolKind::Read => {
                            "Source: exact file excerpt preserved from the pre-compact read result."
                        }
                        RestorableToolKind::UnityYamlList => {
                            "Source: exact `unity_yaml_list` result preserved from the pre-compact tool output."
                        }
                        RestorableToolKind::UnityYamlSearch => {
                            "Source: exact `unity_yaml_search` result preserved from the pre-compact tool output."
                        }
                        RestorableToolKind::UnityYamlRead => {
                            "Source: exact `unity_yaml_read` result preserved from the pre-compact tool output."
                        }
                    };
                    (exact_excerpt, source_note.to_string())
                } else {
                    let refreshed = match request.kind {
                        RestorableToolKind::Read => read_current_file_excerpt(
                            &resolved_path,
                            request.offset,
                            request.limit,
                            &display_path,
                        ),
                        RestorableToolKind::UnityYamlList
                        | RestorableToolKind::UnityYamlSearch
                        | RestorableToolKind::UnityYamlRead => {
                            read_current_unity_yaml_excerpt(&resolved_path, &request, &display_path)
                        }
                    };

                    let Some(refreshed_content) = refreshed else {
                        continue;
                    };
                    let source_note = match request.kind {
                        RestorableToolKind::Read => {
                            "Source: original read result had already been compacted, so this was refreshed from the current file state."
                        }
                        RestorableToolKind::UnityYamlList => {
                            "Source: original `unity_yaml_list` result was unavailable, so this was refreshed from the current file state."
                        }
                        RestorableToolKind::UnityYamlSearch => {
                            "Source: original `unity_yaml_search` result was unavailable, so this was refreshed from the current file state."
                        }
                        RestorableToolKind::UnityYamlRead => {
                            "Source: original `unity_yaml_read` result was unavailable, so this was refreshed from the current file state."
                        }
                    };
                    (refreshed_content, source_note.to_string())
                }
            } else {
                let rebuilt = match request.kind {
                    RestorableToolKind::Read => read_current_file_excerpt(
                        &resolved_path,
                        request.offset,
                        request.limit,
                        &display_path,
                    ),
                    RestorableToolKind::UnityYamlList
                    | RestorableToolKind::UnityYamlSearch
                    | RestorableToolKind::UnityYamlRead => {
                        read_current_unity_yaml_excerpt(&resolved_path, &request, &display_path)
                    }
                };

                let Some(rebuilt_content) = rebuilt else {
                    continue;
                };
                let source_note = match request.kind {
                    RestorableToolKind::Read => {
                        "Source: rebuilt from the current file state because no pre-compact tool result was available."
                    }
                    RestorableToolKind::UnityYamlList => {
                        "Source: rebuilt from the current file state because no pre-compact `unity_yaml_list` result was available."
                    }
                    RestorableToolKind::UnityYamlSearch => {
                        "Source: rebuilt from the current file state because no pre-compact `unity_yaml_search` result was available."
                    }
                    RestorableToolKind::UnityYamlRead => {
                        "Source: rebuilt from the current file state because no pre-compact `unity_yaml_read` result was available."
                    }
                };
                (rebuilt_content, source_note.to_string())
            };

            let candidate_tokens = estimate_text_tokens(&display_path)
                .saturating_add(estimate_text_tokens(&source_note))
                .saturating_add(estimate_text_tokens(&content))
                .saturating_add(24);

            if used_tokens.saturating_add(candidate_tokens) > POST_COMPACT_TOTAL_FILE_TOKEN_BUDGET {
                continue;
            }

            used_tokens = used_tokens.saturating_add(candidate_tokens);
            restored.push(RestoredFileContext {
                display_path,
                content,
                source_note,
            });

            if restored.len() >= POST_COMPACT_MAX_FILES_TO_RESTORE {
                break 'outer;
            }
        }
    }

    if restored.is_empty() {
        return String::new();
    }

    let mut section = String::from(
        "### Restored File Context\n\nThe snippets below were auto-restored because these files or Unity assets were inspected before compaction. They are partial carry-forward context for continuation after compact and may be truncated.\n",
    );

    for file in restored {
        section.push_str("\n\n#### ");
        section.push_str(&file.display_path);
        section.push_str("\n");
        section.push_str(&file.source_note);
        section.push_str("\n\n");
        section.push_str(&file.content);
    }

    section
}

fn build_handoff_content(
    summary: &str,
    restored_files_section: &str,
    has_recent_messages: bool,
) -> String {
    let recent_note = if has_recent_messages {
        "Recent verbatim messages remain below this handoff. If anything conflicts, prefer the newer verbatim messages."
    } else {
        "No newer verbatim messages follow this handoff. Treat the summary below as the full carry-forward context."
    };

    let restored_files_block = if restored_files_section.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n{}", restored_files_section.trim())
    };

    format!(
        "## Context Handoff\n\nThis session was compacted to stay within the model context window. The note below is a handoff summary of the earlier conversation so work can continue without losing context.\n\n- Treat this as handoff context, not as a new user request.\n- Preserve the user's goals, constraints, file references, and unfinished work.\n- {}\n\n### Earlier Conversation Summary\n\n{}{}",
        recent_note,
        summary.trim(),
        restored_files_block
    )
}

pub fn build_post_compact_message(
    summary: &str,
    restored_files_section: &str,
    earliest_kept_ts: i64,
    has_recent_messages: bool,
) -> ChatMessage {
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: build_handoff_content(summary, restored_files_section, has_recent_messages),
        created_at: earliest_kept_ts - 1,
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

#[allow(dead_code)]
pub fn find_compact_boundary(messages: &[ChatMessage]) -> usize {
    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == MessageRole::User && msg.tool_call_id.is_none() {
            return i;
        }
    }
    messages.len() / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_message(
        id: &str,
        role: MessageRole,
        content: &str,
        created_at: i64,
        tool_calls: Option<Vec<ToolCallInfo>>,
        tool_call_id: Option<&str>,
    ) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role,
            content: content.to_string(),
            created_at,
            prompt_prefix: None,
            prompt_suffix: None,
            response_id: None,
            content_order: None,
            thinking_order: None,
            tool_calls,
            tool_call_id: tool_call_id.map(|s| s.to_string()),
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
    fn extract_summary_strips_analysis_and_summary_wrappers() {
        let raw = "<analysis>\ninternal draft\n</analysis>\n<summary>\nPrimary Request and Intent\n</summary>";
        assert_eq!(extract_summary(raw), "Primary Request and Intent");
    }

    #[test]
    fn extract_summary_preserves_later_content_when_analysis_close_is_missing() {
        let raw = "<analysis>\ninternal draft\n<summary>\nPrimary Request and Intent\n</summary>\nTrailing detail";
        let stripped = strip_tag_block(raw, ANALYSIS_OPEN, ANALYSIS_CLOSE);

        assert!(stripped.contains("<summary>"));
        assert!(stripped.contains("Trailing detail"));
        assert_eq!(extract_summary(raw), "Primary Request and Intent");
    }

    #[test]
    fn prepare_messages_for_llm_materializes_persisted_prompt_edits() {
        let mut messages = vec![make_message(
            "user-1",
            MessageRole::User,
            "检查这个问题",
            100,
            None,
            None,
        )];
        messages[0].prompt_prefix =
            Some("<system-reminder>\nenv\n</system-reminder>\n".to_string());
        messages[0].prompt_suffix =
            Some("\n\n<system-reminder>\nplan\n</system-reminder>".to_string());

        let prepared = prepare_messages_for_llm(&messages);
        assert_eq!(
            prepared[0].content,
            "<system-reminder>\nenv\n</system-reminder>\n检查这个问题\n\n<system-reminder>\nplan\n</system-reminder>"
        );
        assert_eq!(prepared[0].prompt_prefix, None);
        assert_eq!(prepared[0].prompt_suffix, None);
    }

    #[test]
    fn build_post_compact_message_creates_assistant_handoff() {
        let msg = build_post_compact_message("Continue editing src/main.rs", "", 100, true);
        assert_eq!(msg.role, MessageRole::Assistant);
        assert!(msg.content.contains("## Context Handoff"));
        assert!(msg.content.contains("Continue editing src/main.rs"));
    }

    #[test]
    fn should_auto_compact_uses_buffer() {
        assert!(should_auto_compact(87_000, 100_000));
        assert!(!should_auto_compact(60_000, 100_000));
    }

    #[test]
    fn trim_tool_outputs_rewrites_newest_tool_outputs_until_request_fits() {
        let huge = "工具输出".repeat(40_000);
        let mut messages = vec![
            make_message("user-1", MessageRole::User, "排查", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "running",
                101,
                None,
                None,
            ),
            make_message(
                "tool-old",
                MessageRole::Tool,
                &huge,
                102,
                None,
                Some("tc-1"),
            ),
            make_message(
                "assistant-2",
                MessageRole::Assistant,
                "running",
                103,
                None,
                None,
            ),
            make_message(
                "tool-new-1",
                MessageRole::Tool,
                &huge,
                104,
                None,
                Some("tc-2"),
            ),
            make_message(
                "tool-new-2",
                MessageRole::Tool,
                &huge,
                105,
                None,
                Some("tc-3"),
            ),
        ];

        let rewritten =
            trim_tool_outputs_to_fit_context_window(&mut messages, &["system"], &[], 200_000);

        // Only the trailing tool-output run is rewritten; the walk stops at
        // assistant-2, so tool-old keeps its content even though it is large.
        assert_eq!(rewritten, 2);
        assert_eq!(messages[5].content, CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE);
        assert_eq!(messages[4].content, CONTEXT_WINDOW_TRUNCATED_OUTPUT_MESSAGE);
        assert_eq!(messages[2].content, huge);
        assert!(estimate_request_tokens(&["system"], &messages, &[]) <= 200_000);

        // Already-fitting histories are untouched.
        let mut small = vec![make_message(
            "user-1",
            MessageRole::User,
            "排查",
            100,
            None,
            None,
        )];
        assert_eq!(
            trim_tool_outputs_to_fit_context_window(&mut small, &["system"], &[], 200_000),
            0
        );
    }

    #[test]
    fn calibrated_input_tokens_scales_by_actual_usage_ratio() {
        // CJK/JSON-heavy histories: provider counted 130k where we estimated 100k.
        assert_eq!(
            calibrated_input_tokens(120_000, Some((130_000, 100_000)), 0),
            156_000
        );
        // Never calibrates downward, even when the provider counted fewer tokens.
        assert_eq!(
            calibrated_input_tokens(120_000, Some((80_000, 100_000)), 0),
            120_000
        );
        // Degenerate samples are ignored.
        assert_eq!(
            calibrated_input_tokens(120_000, Some((0, 100_000)), 0),
            120_000
        );
        assert_eq!(
            calibrated_input_tokens(120_000, Some((100_000, 0)), 0),
            120_000
        );
        assert_eq!(calibrated_input_tokens(120_000, None, 0), 120_000);
    }

    #[test]
    fn calibrated_input_tokens_floors_at_persisted_context_usage() {
        // First request of a new run: no in-run sample yet, but the session's
        // recorded real usage already shows the estimate is too low.
        assert_eq!(calibrated_input_tokens(100_000, None, 150_000), 150_000);
        // Stale persisted usage (e.g. recorded before a history rollback) is
        // capped relative to the current estimate.
        assert_eq!(calibrated_input_tokens(10_000, None, 1_000_000), 40_000);
        assert_eq!(calibrated_input_tokens(100_000, None, 0), 100_000);
    }

    #[test]
    fn calibrated_input_tokens_recovers_missed_auto_compact_trigger() {
        let limit = 200_000;
        // The raw estimate alone stays below the trigger line while real usage
        // is already near the context limit.
        assert!(!should_auto_compact(120_000, limit));
        let effective = calibrated_input_tokens(120_000, Some((190_000, 120_000)), 0);
        assert!(should_auto_compact(effective, limit));
        let effective_from_floor = calibrated_input_tokens(120_000, None, 190_000);
        assert!(should_auto_compact(effective_from_floor, limit));
    }

    #[test]
    fn codex_auto_compact_uses_ninety_percent_context_limit() {
        assert!(!should_codex_auto_compact(180_000, 258_400));
        assert!(!should_codex_auto_compact(232_559, 258_400));
        assert!(should_codex_auto_compact(232_560, 258_400));
        assert!(should_codex_block_normal_send(235_000, 258_400));
    }

    #[test]
    fn token_estimator_uses_codex_byte_heuristic_for_json() {
        let schema = r#"{"type":"function","function":{"name":"knowledge_query","parameters":{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"number"}}}}}"#
            .repeat(200);

        let estimated = estimate_text_tokens(&schema);
        assert_eq!(estimated, ((schema.len() + 3) / 4) as u32);
        assert!(estimated < (schema.len() as u32 / 2));
    }

    #[test]
    fn codex_short_turn_with_large_tool_schemas_does_not_preflight_compact() {
        let tools = (0..20)
            .map(|idx| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": format!("tool_{}", idx),
                        "description": "tool schema description ".repeat(120),
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "query": {
                                    "type": "string",
                                    "description": "query field ".repeat(80),
                                },
                                "limit": {
                                    "type": "number",
                                    "description": "limit field ".repeat(40),
                                }
                            }
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        let messages = vec![make_message(
            "user-1",
            MessageRole::User,
            "analyze project structure",
            100,
            None,
            None,
        )];

        let estimated = estimate_request_tokens(&["system"], &messages, &tools);
        assert!(estimated < 60_000);
        assert!(!should_codex_auto_compact(estimated, 258_400));
    }

    #[test]
    fn token_estimator_ignores_nested_subagent_tool_history() {
        let task_arguments =
            r#"{"description":"scan","prompt":"inspect project","subagent_type":"explorer"}"#;
        let task_call = ToolCallInfo {
            id: "task-1".to_string(),
            name: "task".to_string(),
            arguments: task_arguments.to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        };
        let mut task_call_with_nested = task_call.clone();
        task_call_with_nested.nested_tool_calls = Some(vec![ToolCallInfo {
            id: "read-1".to_string(),
            name: "read".to_string(),
            arguments: "large nested args ".repeat(20_000),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: Some(crate::commands::ToolCallOutcome::Done),
            recorded_output: Some("nested output ".repeat(60_000)),
            nested_tool_calls: None,
        }]);

        let without_nested = vec![
            make_message("user-1", MessageRole::User, "delegate", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                101,
                Some(vec![task_call]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "subagent final answer",
                102,
                None,
                Some("task-1"),
            ),
        ];
        let with_nested = vec![
            make_message("user-1", MessageRole::User, "delegate", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                101,
                Some(vec![task_call_with_nested]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "subagent final answer",
                102,
                None,
                Some("task-1"),
            ),
        ];

        let baseline = estimate_request_tokens(&["system"], &without_nested, &[]);
        let estimated = estimate_request_tokens(&["system"], &with_nested, &[]);

        assert_eq!(estimated, baseline);
        assert!(!should_codex_auto_compact(estimated, 258_400));
    }

    #[test]
    fn token_estimator_ignores_recorded_output_replayed_by_tool_message() {
        let tool_call = ToolCallInfo {
            id: "tc-1".to_string(),
            name: "read".to_string(),
            arguments: r#"{"path":"src/main.rs"}"#.to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        };
        let mut tool_call_with_recorded_output = tool_call.clone();
        tool_call_with_recorded_output.recorded_output = Some("duplicated output ".repeat(80_000));

        let without_recorded_output = vec![
            make_message("user-1", MessageRole::User, "inspect", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "running tool",
                101,
                Some(vec![tool_call]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "actual tool output",
                102,
                None,
                Some("tc-1"),
            ),
        ];
        let with_recorded_output = vec![
            make_message("user-1", MessageRole::User, "inspect", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "running tool",
                101,
                Some(vec![tool_call_with_recorded_output]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "actual tool output",
                102,
                None,
                Some("tc-1"),
            ),
        ];

        assert_eq!(
            estimate_request_tokens(&["system"], &with_recorded_output, &[]),
            estimate_request_tokens(&["system"], &without_recorded_output, &[])
        );
    }

    #[test]
    fn compact_request_is_budgeted_when_history_exceeds_limit() {
        let mut messages = vec![make_message(
            "user-1",
            MessageRole::User,
            "请分析这个 Unity 问题",
            100,
            None,
            None,
        )];
        for i in 0..80 {
            messages.push(make_message(
                &format!("assistant-{}", i),
                MessageRole::Assistant,
                &"assistant output ".repeat(200),
                101 + i,
                Some(vec![ToolCallInfo {
                    id: format!("tc-{}", i),
                    name: "bash".to_string(),
                    arguments: "large args".repeat(300),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: Some("recorded output should be stripped".repeat(200)),
                    nested_tool_calls: None,
                }]),
                None,
            ));
            messages.push(make_message(
                &format!("tool-{}", i),
                MessageRole::Tool,
                &"tool output ".repeat(2_000),
                101 + i,
                None,
                Some(&format!("tc-{}", i)),
            ));
        }

        let plan = build_compact_request_with_budget(&messages, &["system"], 258_400)
            .expect("budgeted compact request");
        assert!(plan.estimated_tokens <= plan.budget_tokens);
        assert!(plan.truncated);
        assert!(plan.boundary_idx > 0);
    }

    #[test]
    fn compact_request_prunes_tool_outputs_when_their_calls_are_omitted() {
        let mut messages = vec![make_message(
            "user-1",
            MessageRole::User,
            "排查长会话 compact 失败",
            100,
            None,
            None,
        )];
        for i in 0..80 {
            messages.push(make_message(
                &format!("assistant-{}", i),
                MessageRole::Assistant,
                "running tool",
                101 + i,
                Some(vec![ToolCallInfo {
                    id: format!("tc-{}", i),
                    name: "read".to_string(),
                    arguments: "large args".repeat(300),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ));
            messages.push(make_message(
                &format!("tool-{}", i),
                MessageRole::Tool,
                &"tool output ".repeat(2_000),
                101 + i,
                None,
                Some(&format!("tc-{}", i)),
            ));
        }

        let plan = build_compact_request_with_budget(&messages, &["system"], 258_400)
            .expect("budgeted compact request");
        let mut visible_tool_call_ids = HashSet::new();
        for message in &plan.messages {
            if let Some(tool_calls) = message.tool_calls.as_ref() {
                visible_tool_call_ids.extend(tool_calls.iter().map(|call| call.id.clone()));
            }
            if message.role == MessageRole::Tool {
                let tool_call_id = message
                    .tool_call_id
                    .as_deref()
                    .expect("tool result should have an id");
                assert!(
                    visible_tool_call_ids.contains(tool_call_id),
                    "tool result without visible function call: {}",
                    tool_call_id
                );
            }
        }
    }

    #[test]
    fn single_user_many_tool_rounds_compact_boundary_prunes_history() {
        let mut messages = vec![make_message(
            "user-1",
            MessageRole::User,
            "排查阴影闪烁",
            100,
            None,
            None,
        )];
        for i in 0..30 {
            messages.push(make_message(
                &format!("assistant-{}", i),
                MessageRole::Assistant,
                "running tool",
                101 + i,
                Some(vec![ToolCallInfo {
                    id: format!("tc-{}", i),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ));
            messages.push(make_message(
                &format!("tool-{}", i),
                MessageRole::Tool,
                &"tool output ".repeat(600),
                101 + i,
                None,
                Some(&format!("tc-{}", i)),
            ));
        }

        let boundary = find_compact_boundary_by_budget(&messages, 8_000);
        assert!(boundary > 0);
        assert!(boundary < messages.len() - 1);
        assert_ne!(messages[boundary].role, MessageRole::Tool);
    }

    #[test]
    fn compact_boundary_counts_tool_arguments() {
        let messages = vec![
            make_message("user-1", MessageRole::User, "分析工具结果", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "running tool",
                101,
                Some(vec![ToolCallInfo {
                    id: "tc-large".to_string(),
                    name: "read".to_string(),
                    arguments: "large args".repeat(4_000),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message("user-2", MessageRole::User, "继续", 102, None, None),
        ];

        let boundary = find_compact_boundary_by_budget(&messages, 8_000);

        assert_eq!(boundary, 1);
    }

    #[test]
    fn compact_boundary_counts_server_tool_output() {
        let messages = vec![
            make_message("user-1", MessageRole::User, "搜索资料", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "searched",
                101,
                Some(vec![ToolCallInfo {
                    id: "tc-web".to_string(),
                    name: "web_search".to_string(),
                    arguments: "{}".to_string(),
                    order: None,
                    server_tool: Some(crate::session::models::ServerToolKind::WebSearch),
                    server_tool_output: Some("search result ".repeat(4_000)),
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message("user-2", MessageRole::User, "继续", 102, None, None),
        ];

        let boundary = find_compact_boundary_by_budget(&messages, 8_000);

        assert_eq!(boundary, 1);
    }

    #[test]
    fn compact_boundary_does_not_prune_when_recent_tail_is_already_small() {
        let messages = vec![
            make_message("user-1", MessageRole::User, "分析项目结构", 100, None, None),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "我会检查目录",
                101,
                None,
                None,
            ),
        ];

        let boundary = find_compact_boundary_by_budget(&messages, 8_000);
        assert_eq!(boundary, 0);
        assert!(!has_compactable_messages_before_boundary(
            &messages, boundary
        ));
    }

    #[test]
    fn compact_boundary_moves_past_oversized_first_user_message() {
        let messages = vec![
            make_message(
                "user-1",
                MessageRole::User,
                &"a".repeat(100_000),
                100,
                None,
                None,
            ),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "旧回答",
                101,
                None,
                None,
            ),
            make_message("user-2", MessageRole::User, "继续", 102, None, None),
        ];

        let boundary = find_compact_boundary_by_budget(&messages, 20_000);

        assert_eq!(boundary, 1);
        assert!(has_compactable_messages_before_boundary(
            &messages, boundary
        ));
    }

    #[test]
    fn compactable_boundary_detects_user_history_over_codex_budget() {
        let messages = vec![
            make_message(
                "user-1",
                MessageRole::User,
                &"a".repeat(60_000),
                100,
                None,
                None,
            ),
            make_message(
                "user-2",
                MessageRole::User,
                &"b".repeat(60_000),
                101,
                None,
                None,
            ),
        ];

        assert!(has_compactable_messages_before_boundary(
            &messages,
            messages.len()
        ));
    }

    #[test]
    fn recent_user_selector_caps_prompt_history_by_codex_budget() {
        let messages = vec![
            make_message(
                "user-1",
                MessageRole::User,
                &"a".repeat(32),
                100,
                None,
                None,
            ),
            make_message(
                "user-2",
                MessageRole::User,
                &"b".repeat(32),
                101,
                None,
                None,
            ),
            make_message(
                "user-3",
                MessageRole::User,
                &"c".repeat(32),
                102,
                None,
                None,
            ),
        ];

        let selected =
            select_recent_user_message_ids_for_compact_prompt(&messages, messages.len(), 40);

        assert!(!selected.contains("user-1"));
        assert!(selected.contains("user-2"));
        assert!(selected.contains("user-3"));
    }

    #[test]
    fn invalid_compact_summary_rejects_ellipsis() {
        assert!(!is_valid_compact_summary("..."));
        assert!(!is_valid_compact_summary("<summary>...</summary>"));
        assert!(is_valid_compact_summary(
            "1. Primary Request and Intent\nFix context compaction token estimation.\n\n2. All User Messages\n- Thoroughly repair token estimation and compaction behavior."
        ));
    }

    #[test]
    fn emergency_compact_summary_preserves_user_and_recent_tail() {
        let messages = vec![
            make_message(
                "user-1",
                MessageRole::User,
                "修复上下文压缩",
                100,
                None,
                None,
            ),
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "已检查 compact.rs",
                101,
                None,
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                &"A".repeat(5_000),
                102,
                None,
                Some("tc-1"),
            ),
            make_message(
                "assistant-2",
                MessageRole::Assistant,
                "继续检查 store.rs",
                103,
                None,
                None,
            ),
        ];

        let summary = build_emergency_compact_summary(&messages, 2, "compact request too large");
        assert!(summary.contains("修复上下文压缩"));
        assert!(summary.contains("compact request too large"));
        assert!(summary.contains("Recent Raw Tail Preview"));
    }

    #[test]
    fn compact_prompt_requires_concise_checkpoint_handoff() {
        assert!(COMPACT_PROMPT.contains("CONTEXT CHECKPOINT COMPACTION"));
        assert!(COMPACT_PROMPT.contains("Be concise"));
        assert!(!COMPACT_PROMPT.contains("All User Messages"));
    }

    #[test]
    fn restored_files_section_uses_prior_read_result_when_available() {
        let temp_root =
            std::env::temp_dir().join(format!("locus-compact-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_root).expect("temp dir should be created");

        let messages = vec![
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                1,
                Some(vec![ToolCallInfo {
                    id: "tc-read".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({
                        "filePath": "src/main.ts",
                        "offset": 1,
                        "limit": 20
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "<content>\nconst foo = 1;\n</content>",
                2,
                None,
                Some("tc-read"),
            ),
        ];

        let section =
            build_post_compact_restored_files_section(&messages, &temp_root.display().to_string());

        assert!(section.contains("Restored File Context"));
        assert!(section.contains("src/main.ts"));
        assert!(section.contains("const foo = 1;"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn restored_files_section_falls_back_to_current_file_state() {
        let temp_root =
            std::env::temp_dir().join(format!("locus-compact-test-{}", uuid::Uuid::new_v4()));
        let src_dir = temp_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("temp dir should be created");
        std::fs::write(src_dir.join("main.ts"), "line one\nline two\nline three\n")
            .expect("file should be written");

        let messages = vec![
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                1,
                Some(vec![ToolCallInfo {
                    id: "tc-read".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({
                        "filePath": "src/main.ts",
                        "offset": 2,
                        "limit": 2
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                CLEARED_TOOL_RESULT,
                2,
                None,
                Some("tc-read"),
            ),
        ];

        let section =
            build_post_compact_restored_files_section(&messages, &temp_root.display().to_string());

        assert!(section.contains("src/main.ts"));
        assert!(section.contains("line two"));
        assert!(section.contains("current file state"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn restored_files_section_uses_prior_unity_yaml_read_result_when_available() {
        let temp_root =
            std::env::temp_dir().join(format!("locus-compact-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_root).expect("temp dir should be created");

        let messages = vec![
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                1,
                Some(vec![ToolCallInfo {
                    id: "tc-unity-read".to_string(),
                    name: "unity_yaml_read".to_string(),
                    arguments: serde_json::json!({
                        "file_path": "Assets/Data/Test.asset"
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                "Content of 'Assets/Data/Test.asset' (1 documents):\n\n--- MonoBehaviour ---\n  m_Name: TestAsset\n  value: 42\n",
                2,
                None,
                Some("tc-unity-read"),
            ),
        ];

        let section =
            build_post_compact_restored_files_section(&messages, &temp_root.display().to_string());

        assert!(section.contains("Assets/Data/Test.asset"));
        assert!(section.contains("exact `unity_yaml_read` result"));
        assert!(section.contains("value: 42"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn restored_files_section_can_reload_persisted_unity_yaml_result() {
        let temp_root =
            std::env::temp_dir().join(format!("locus-compact-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(temp_root.join("tool-results"))
            .expect("temp dir should be created");
        let persisted_path = temp_root.join("tool-results/unity.txt");
        std::fs::write(
            &persisted_path,
            "Content of 'Assets/Data/Test.asset' (1 documents):\n\n--- MonoBehaviour ---\n  m_Name: PersistedAsset\n",
        )
        .expect("persisted output should be written");

        let persisted_ref = format!(
            "<persisted-output>\nOutput too large (123 chars). Full output saved to: {}\nUse the Read tool with this exact path if you need the full output.\n\nPreview (first 10 chars):\npreview\n</persisted-output>",
            persisted_path.display()
        );

        let messages = vec![
            make_message(
                "assistant-1",
                MessageRole::Assistant,
                "",
                1,
                Some(vec![ToolCallInfo {
                    id: "tc-unity-read".to_string(),
                    name: "unity_yaml_read".to_string(),
                    arguments: serde_json::json!({
                        "file_path": "Assets/Data/Test.asset"
                    })
                    .to_string(),
                    order: None,
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                None,
            ),
            make_message(
                "tool-1",
                MessageRole::Tool,
                &persisted_ref,
                2,
                None,
                Some("tc-unity-read"),
            ),
        ];

        let section =
            build_post_compact_restored_files_section(&messages, &temp_root.display().to_string());

        assert!(section.contains("PersistedAsset"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
