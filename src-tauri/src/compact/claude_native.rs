//! Locus additions around Claude Code's native compaction.
//!
//! Claude owns the summary and the post-compaction conversation. Locus adds
//! three things the native compactor cannot know about: instructions for the
//! summarizer, a local copy of the full transcript (the native path lives on
//! the Claude host, which Locus tools cannot read in SSH mode), and a
//! deterministic carry-forward block appended to the summary row. The block
//! restores what repeated summarization loses: the user's own messages, the
//! task list and excerpts of files read through Locus tools, which Claude's
//! own post-compaction file restoration never sees because bridged reads
//! bypass its read state.
use super::{
    build_post_compact_restored_files_section_with_budgets, estimate_text_tokens,
    CONTEXT_HANDOFF_MARKER, CONTEXT_HANDOFF_SUMMARY_HEADING,
};
use crate::session::models::{ChatMessage, MessageRole, TodoItem};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Passed to Claude's summarizer as additional instructions.
pub const SUMMARY_INSTRUCTIONS: &str = "Locus compaction requirements:\n\
- Write the summary in the language the user writes in.\n\
- In Current Work and Pending Tasks, state exactly which task is in progress, what was just changed (files, symbols), what was last verified, and what remains, so work resumes without re-discovery.\n\
- Preserve every confirmed user decision, constraint and preference, including those carried in any earlier summary. Never collapse earlier user messages into a generic placeholder.\n\
- Keep exact paths of plan and design documents being followed, files being edited, commands and error messages.\n\
- Locus appends the user's messages verbatim, the task list and excerpts of recently read files after your summary. Do not reproduce large file contents.";

pub const CARRY_FORWARD_HEADING: &str = "## Locus Carry-Forward Context";
const USER_MESSAGES_BUDGET_MIN_TOKENS: u32 = 3_000;
const USER_MESSAGES_BUDGET_MAX_TOKENS: u32 = 12_000;
const USER_MESSAGE_MAX_TOKENS: u32 = 3_000;
const TASK_ITEMS_MAX: usize = 50;
const RESTORED_FILES_MAX: usize = 8;
const TRANSCRIPT_DIR: &str = "compact-transcripts";

pub struct CarryForwardInput<'a> {
    /// Full display history of the session, oldest first.
    pub display_messages: &'a [ChatMessage],
    /// Messages Claude had in context before this compaction.
    pub context_messages: &'a [ChatMessage],
    pub todos: &'a [TodoItem],
    pub working_dir: &'a str,
    pub transcript_path: Option<&'a str>,
    /// The path Claude names lives on another machine and no local copy
    /// could be made.
    pub transcript_unreachable: bool,
    /// Claude's compaction window (the configured auto-compact window, or
    /// the model window when none is set).
    pub compaction_window: u32,
}

/// File restoration budget for Claude's native compaction. Claude Code itself
/// restores up to 50k tokens of natively read files; Locus-bridged reads get
/// a comparable share, scaled so small compaction windows stay usable.
pub fn restore_budgets(compaction_window: u32) -> (u32, u32, usize) {
    let total = (compaction_window / 16).clamp(6_000, 48_000);
    let per_file = (total / 4).clamp(2_000, 12_000);
    (total, per_file, RESTORED_FILES_MAX)
}

fn user_messages_budget(compaction_window: u32) -> u32 {
    (compaction_window / 32).clamp(USER_MESSAGES_BUDGET_MIN_TOKENS, USER_MESSAGES_BUDGET_MAX_TOKENS)
}

pub fn build_carry_forward(input: &CarryForwardInput<'_>) -> String {
    let mut sections = Vec::new();
    if let Some(path) = input.transcript_path.filter(|path| !path.trim().is_empty()) {
        sections.push(format!(
            "### Full Pre-Compaction Transcript\n\nA local copy of the complete native transcript (JSONL) is at `{path}`. Use it instead of the transcript path named in the summary, which may be on the Claude host. Search it with grep or read line ranges with the read tool when exact earlier details are needed."
        ));
    } else if input.transcript_unreachable {
        sections.push(
            "### Full Pre-Compaction Transcript\n\nThe transcript path named in the summary is on the Claude host and cannot be read with Locus tools. Do not try to open it; rely on this summary and the context below.".to_string(),
        );
    }
    if let Some(section) = user_messages_section(input.display_messages, user_messages_budget(input.compaction_window)) {
        sections.push(section);
    }
    if let Some(section) = tasks_section(input.todos) {
        sections.push(section);
    }
    let (total, per_file, max_files) = restore_budgets(input.compaction_window);
    let restored = build_post_compact_restored_files_section_with_budgets(
        input.context_messages,
        input.working_dir,
        total,
        per_file,
        max_files,
    );
    if !restored.trim().is_empty() {
        sections.push(restored.trim().to_string());
    }
    sections.push(language_note(input.display_messages));
    format!(
        "{CARRY_FORWARD_HEADING}\n\nLocus captured this state when the conversation was compacted. It is authoritative where it differs from the compaction summary.\n\n{}",
        sections.join("\n\n")
    )
}

struct UserEntry<'a> {
    label: &'static str,
    text: &'a str,
}

/// The user's own words in chronological order: typed messages and answers
/// given through the question tool.
fn user_entries(messages: &[ChatMessage]) -> Vec<UserEntry<'_>> {
    let question_calls: HashSet<&str> = messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .flat_map(|message| message.tool_calls.iter().flatten())
        .filter(|call| call.name == "ask_user_question")
        .map(|call| call.id.as_str())
        .collect();
    messages
        .iter()
        .filter_map(|message| {
            let text = message.content.trim();
            if text.is_empty() {
                return None;
            }
            match message.role {
                MessageRole::User if message.tool_call_id.is_none() => {
                    Some(UserEntry { label: "User", text })
                }
                MessageRole::Tool
                    if message
                        .tool_call_id
                        .as_deref()
                        .is_some_and(|id| question_calls.contains(id)) =>
                {
                    Some(UserEntry { label: "User answer", text })
                }
                _ => None,
            }
        })
        .collect()
}

fn clip_to_tokens(text: &str, max_tokens: u32) -> String {
    if estimate_text_tokens(text) <= max_tokens {
        return text.to_string();
    }
    // Keep the beginning and the end: requests usually start with the goal
    // and end with the latest constraint.
    let budget_bytes = max_tokens as usize * 4;
    let head_bytes = budget_bytes * 2 / 3;
    let tail_bytes = budget_bytes - head_bytes;
    let head_end = (0..=head_bytes.min(text.len()))
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    let tail_start = (text.len().saturating_sub(tail_bytes)..=text.len())
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(text.len())
        .max(head_end);
    format!(
        "{}\n[... middle of this message omitted to fit the carry-forward budget ...]\n{}",
        text[..head_end].trim_end(),
        text[tail_start..].trim_start()
    )
}

fn user_messages_section(messages: &[ChatMessage], budget: u32) -> Option<String> {
    let entries = user_entries(messages);
    if entries.is_empty() {
        return None;
    }
    // Select newest first so the latest requests always survive, then print
    // in chronological order.
    let mut used = 0u32;
    let mut selected = Vec::new();
    for (index, entry) in entries.iter().enumerate().rev() {
        let text = clip_to_tokens(entry.text, USER_MESSAGE_MAX_TOKENS.min(budget));
        let cost = estimate_text_tokens(&text) + 8;
        if used + cost > budget && !selected.is_empty() {
            break;
        }
        used += cost;
        selected.push((index, entry.label, text));
    }
    selected.reverse();
    let omitted = selected.first().map(|(index, ..)| *index).unwrap_or(0);
    let mut section = String::from(
        "### User Messages (verbatim, oldest first)\n\nThe user's own messages in this conversation, copied by Locus. They take precedence over the summary when they conflict.",
    );
    if omitted > 0 {
        section.push_str(&format!(
            "\n\n({omitted} earlier user message(s) omitted for length; they are in the transcript.)"
        ));
    }
    for (index, label, text) in selected {
        section.push_str(&format!("\n\n{}. {label}:\n{text}", index + 1));
    }
    Some(section)
}

fn tasks_section(todos: &[TodoItem]) -> Option<String> {
    let items: Vec<_> = todos
        .iter()
        .filter(|item| !item.content.trim().is_empty())
        .take(TASK_ITEMS_MAX)
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut section = String::from("### Task List at Compaction");
    for item in items {
        section.push_str(&format!("\n- [{}] {}", item.status, item.content.trim()));
    }
    Some(section)
}

fn language_note(messages: &[ChatMessage]) -> String {
    let (mut letters, mut cjk) = (0usize, 0usize);
    for entry in user_entries(messages).iter().rev().take(20) {
        for ch in entry.text.chars().filter(|ch| ch.is_alphabetic()) {
            letters += 1;
            if matches!(ch as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af) {
                cjk += 1;
            }
        }
    }
    if letters > 0 && cjk * 5 >= letters {
        "### Reply Language\n\nThe user writes in Chinese, Japanese or Korean (see the messages above). Reply to the user in that language even though parts of this summary are in English.".into()
    } else {
        "### Reply Language\n\nReply in the language the user writes in (see the messages above), not necessarily the language of this summary.".into()
    }
}

/// One message Claude kept verbatim after its summary, as the Mod reports it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeptMessage {
    pub role: String,
    pub text: String,
    pub tool_use_ids: Vec<String>,
    pub tool_result_ids: Vec<String>,
}

/// Locus messages that correspond to Claude's kept suffix. Kept messages are
/// always the most recent ones, so the first kept message that can be anchored
/// to a local row marks the boundary; everything from there on stays in the
/// prompt mirror. Unanchored leading rows (native reminders without a local
/// copy) only make the retained range more conservative.
pub fn resolve_kept_message_ids(context: &[ChatMessage], kept: &[KeptMessage]) -> Vec<String> {
    // A tool id belongs to both its call and its result; the earliest row
    // (the assistant call) is the anchor.
    let mut tool_owner: HashMap<&str, usize> = HashMap::new();
    for (index, message) in context.iter().enumerate() {
        let calls = message.tool_calls.iter().flatten().map(|call| call.id.as_str());
        for id in calls.chain(message.tool_call_id.as_deref()) {
            tool_owner.entry(id).or_insert(index);
        }
    }
    let boundary = kept.iter().find_map(|message| {
        let by_tool = message
            .tool_use_ids
            .iter()
            .chain(message.tool_result_ids.iter())
            .filter_map(|id| tool_owner.get(id.as_str()).copied())
            .min();
        if by_tool.is_some() {
            return by_tool;
        }
        let text = message.text.trim();
        if text.is_empty() {
            return None;
        }
        let role = match message.role.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            _ => return None,
        };
        context.iter().rposition(|local| {
            let content = local.content.trim();
            local.role == role
                && local.tool_call_id.is_none()
                && !content.is_empty()
                && !content.starts_with(CONTEXT_HANDOFF_MARKER)
                && (text == content || text.contains(content) || content.contains(text))
        })
    });
    boundary
        .map(|start| context[start..].iter().map(|message| message.id.clone()).collect())
        .unwrap_or_default()
}

/// Local handoff for a native compaction. Its body is the summary exactly as
/// Claude reads it; the wrapper only says whether recent messages follow.
pub fn build_handoff_message(summary: &str, earliest_kept_ts: i64, retains_messages: bool) -> ChatMessage {
    let retained = if retains_messages {
        "The most recent messages before compaction remain verbatim after this handoff."
    } else {
        "No verbatim messages follow this handoff."
    };
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: format!(
            "{CONTEXT_HANDOFF_MARKER}\n\nClaude Code compacted this conversation. The summary below is the context Claude continues from. {retained}\n\n- Treat this as handoff context, not as a new user request.\n\n{CONTEXT_HANDOFF_SUMMARY_HEADING}\n\n{}",
            summary.trim()
        ),
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

fn safe_component(value: &str) -> Option<String> {
    let safe: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    (!safe.is_empty()).then_some(safe)
}

/// Where the local copy of a native transcript is written: under the app
/// temp directory, which the read tool may access even with the workspace
/// boundary enabled. One file per native conversation; the native JSONL is
/// append-only, so each copy supersedes the previous one.
pub fn transcript_copy_path(locus_session_id: &str, native_id: &str) -> Option<PathBuf> {
    transcript_copy_path_in(&crate::commands::app_temp_dir().ok()?, locus_session_id, native_id)
}

fn transcript_copy_path_in(root: &std::path::Path, locus_session_id: &str, native_id: &str) -> Option<PathBuf> {
    Some(
        root.join(TRANSCRIPT_DIR)
            .join("claude")
            .join(safe_component(locus_session_id)?)
            .join(format!("{}.jsonl", safe_component(native_id)?)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::models::ToolCallInfo;

    fn message(id: &str, role: MessageRole, content: &str) -> ChatMessage {
        let mut message = build_handoff_message("", 1, false);
        message.id = id.into();
        message.role = role;
        message.content = content.into();
        message.created_at = 10;
        message
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> ToolCallInfo {
        serde_json::from_value(serde_json::json!({"id":id,"name":name,"arguments":arguments})).unwrap()
    }

    #[test]
    fn kept_suffix_is_anchored_by_tool_ids_then_text() {
        let mut assistant = message("a1", MessageRole::Assistant, "Running the build.");
        assistant.tool_calls = Some(vec![tool_call("toolu_1", "bash", "{}")]);
        let mut result = message("t1", MessageRole::Tool, "ok");
        result.tool_call_id = Some("toolu_1".into());
        let context = vec![
            message("h", MessageRole::Assistant, &format!("{CONTEXT_HANDOFF_MARKER}\nold")),
            message("u1", MessageRole::User, "first request"),
            assistant,
            result,
            message("a2", MessageRole::Assistant, "Build passed."),
        ];
        let by_tool = resolve_kept_message_ids(&context, &[
            KeptMessage { role: "user".into(), text: "<system-reminder>native only</system-reminder>".into(), ..Default::default() },
            KeptMessage { role: "assistant".into(), text: "Running the build.".into(), tool_use_ids: vec!["toolu_1".into()], ..Default::default() },
            KeptMessage { role: "user".into(), tool_result_ids: vec!["toolu_1".into()], ..Default::default() },
        ]);
        assert_eq!(by_tool, vec!["a1", "t1", "a2"]);
        let by_text = resolve_kept_message_ids(&context, &[
            KeptMessage { role: "user".into(), text: "<locus-pending-user>first request</locus-pending-user>".into(), ..Default::default() },
        ]);
        assert_eq!(by_text, vec!["u1", "a1", "t1", "a2"]);
        assert!(resolve_kept_message_ids(&context, &[]).is_empty());
        // The previous handoff is never treated as a kept user/assistant row.
        assert!(resolve_kept_message_ids(&context, &[KeptMessage { role: "assistant".into(), text: "old".into(), ..Default::default() }]).is_empty());
    }

    #[test]
    fn carry_forward_keeps_user_words_tasks_files_and_language() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("plan.md"), "PLAN_LINE_1\nPLAN_LINE_2\n").unwrap();
        let mut read = message("a1", MessageRole::Assistant, "");
        read.tool_calls = Some(vec![
            tool_call("r1", "read", r#"{"filePath":"plan.md","offset":1,"limit":2000}"#),
            tool_call("q1", "ask_user_question", r#"{"questions":[]}"#),
        ]);
        let mut read_result = message("t1", MessageRole::Tool, "1\tPLAN_LINE_1\n2\tPLAN_LINE_2");
        read_result.tool_call_id = Some("r1".into());
        let mut answer = message("t2", MessageRole::Tool, "两个都有");
        answer.tool_call_id = Some("q1".into());
        let display = vec![
            message("u1", MessageRole::User, "先使用SC1010实现Team原型"),
            read.clone(),
            read_result.clone(),
            answer,
            message("u2", MessageRole::User, "使用中文向我汇报"),
        ];
        let todos = vec![TodoItem { content: "Phase 1".into(), status: "in_progress".into(), priority: "medium".into() }];
        let text = build_carry_forward(&CarryForwardInput {
            display_messages: &display,
            context_messages: &[read, read_result],
            todos: &todos,
            working_dir: &root.path().to_string_lossy(),
            transcript_path: Some("C:/temp/compact-transcripts/claude/s/n.jsonl"),
            transcript_unreachable: false,
            compaction_window: 600_000,
        });
        assert!(text.starts_with(CARRY_FORWARD_HEADING));
        assert!(text.contains("`C:/temp/compact-transcripts/claude/s/n.jsonl`"));
        let unreachable = build_carry_forward(&CarryForwardInput {
            display_messages: &display,
            context_messages: &[],
            todos: &[],
            working_dir: &root.path().to_string_lossy(),
            transcript_path: None,
            transcript_unreachable: true,
            compaction_window: 600_000,
        });
        assert!(unreachable.contains("cannot be read with Locus tools"));
        assert!(!unreachable.contains("A local copy"));
        let first = text.find("先使用SC1010实现Team原型").unwrap();
        let answer_at = text.find("User answer:\n两个都有").unwrap();
        let latest = text.find("使用中文向我汇报").unwrap();
        assert!(first < answer_at && answer_at < latest, "user words are chronological");
        assert!(text.contains("- [in_progress] Phase 1"));
        assert!(text.contains("PLAN_LINE_2"), "files read before compaction are restored");
        assert!(text.contains("Reply to the user in that language"));
    }

    #[test]
    fn long_user_history_keeps_latest_messages_within_budget() {
        let long = "x".repeat(USER_MESSAGE_MAX_TOKENS as usize * 4 * 3);
        let mut display: Vec<_> = (0..20).map(|index| message(&format!("u{index}"), MessageRole::User, &format!("REQUEST_{index} {long}"))).collect();
        display.push(message("latest", MessageRole::User, "LATEST_REQUEST"));
        let budget = user_messages_budget(600_000);
        let section = user_messages_section(&display, budget).unwrap();
        assert!(section.contains("LATEST_REQUEST"));
        assert!(section.contains("earlier user message(s) omitted"));
        assert!(!section.contains("REQUEST_0 "));
        assert!(estimate_text_tokens(&section) <= budget + 200);
        assert!(section.contains("middle of this message omitted"));
    }

    #[test]
    fn carry_forward_budgets_scale_with_the_compaction_window() {
        assert_eq!(restore_budgets(100_000), (6_250, 2_000, RESTORED_FILES_MAX));
        assert_eq!(restore_budgets(600_000), (37_500, 9_375, RESTORED_FILES_MAX));
        assert_eq!(restore_budgets(1_000_000), (48_000, 12_000, RESTORED_FILES_MAX));
        assert_eq!(user_messages_budget(100_000), 3_125);
        assert_eq!(user_messages_budget(600_000), USER_MESSAGES_BUDGET_MAX_TOKENS);
    }

    #[test]
    fn handoff_carries_the_exact_summary_and_retention_note() {
        let handoff = build_handoff_message("SUMMARY_BODY", 100, true);
        assert!(handoff.content.starts_with(CONTEXT_HANDOFF_MARKER));
        assert!(handoff.content.ends_with("SUMMARY_BODY"));
        assert!(handoff.content.contains("remain verbatim after this handoff"));
        assert_eq!(handoff.created_at, 99);
        assert!(build_handoff_message("S", 1, false).content.contains("No verbatim messages follow"));
    }

    #[test]
    fn transcript_copy_path_is_sanitized_under_temp() {
        let root = std::path::Path::new("T:/tmp");
        let path = transcript_copy_path_in(root, "3e53a588-9842", "../12acad17").unwrap();
        assert_eq!(path, root.join("compact-transcripts").join("claude").join("3e53a588-9842").join("12acad17.jsonl"));
        assert!(transcript_copy_path_in(root, "///", "x").is_none());
    }
}
