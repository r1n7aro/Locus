use crate::session::models::{ChatMessage, MessageRole, ToolCallInfo};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const AUTO_COMPACT_THRESHOLD: f64 = 0.9;
const AUTO_COMPACT_BUFFER_MIN_TOKENS: u32 = 4_000;
const AUTO_COMPACT_BUFFER_MAX_TOKENS: u32 = 24_000;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

const MESSAGE_OVERHEAD_TOKENS: u32 = 12;
const TOOL_CALL_OVERHEAD_TOKENS: u32 = 24;
const IMAGE_TOKEN_COST: u32 = 2_000;
const TOOL_SCHEMA_OVERHEAD_TOKENS: u32 = 32;

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

const COMPACT_PROMPT: &str = r#"CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.

You are writing a handoff summary so the same assistant can continue the work after context compaction.
Preserve the user's intent, constraints, unfinished work, recent decisions, file references, and technical details needed to continue without losing momentum.

Important rules:
- Do NOT call Read, Bash, Grep, List, Edit, Write, or any other tool.
- Prefer the user's working language when it is obvious from the conversation.
- Treat this as a handoff note, not as a status update to the user.
- Your final answer must contain exactly two plain-text blocks:
  1. <analysis>...</analysis>
  2. <summary>...</summary>

In <analysis>, work through the conversation chronologically and verify:
- The user's explicit requests and changing intent
- Every non-tool user message, preserving the user's original wording when it matters
- What work was completed vs. still pending
- Important technical decisions, code patterns, and architectural constraints
- Specific files, functions, classes, commands, and edits that matter
- Errors, regressions, retries, and user corrections
- What was being worked on immediately before compaction

In <summary>, write a precise handoff with these sections:
1. Primary Request and Intent
2. All User Messages
   - Include every non-tool user message in chronological order.
   - Keep the user's original wording verbatim when practical.
   - If you must shorten, preserve intent, constraints, and exact asks with high fidelity.
3. Current State of the Work
4. Important Technical Context
5. Files and Code Areas Touched
6. Recent Decisions and Why They Matter
7. Open Issues, Risks, or Follow-ups
8. Latest User Feedback and Constraints
9. Immediate Next Step

If some recent raw messages remain after this summary, assume they will appear below the handoff and should take precedence if there is any conflict.
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

fn auto_compact_buffer(context_limit: u32) -> u32 {
    (context_limit / 20).clamp(
        AUTO_COMPACT_BUFFER_MIN_TOKENS,
        AUTO_COMPACT_BUFFER_MAX_TOKENS,
    )
}

fn estimate_text_tokens(text: &str) -> u32 {
    if text.is_empty() {
        0
    } else {
        ((text.len() + 3) / 4) as u32
    }
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
        total = total
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
            .saturating_add(estimate_text_tokens(&msg.content));

        if let Some(ref thinking) = msg.thinking_content {
            total = total.saturating_add(estimate_text_tokens(thinking));
        }

        if let Some(ref images) = msg.images {
            total = total.saturating_add(images.len() as u32 * IMAGE_TOKEN_COST);
        }

        if let Some(ref tool_calls) = msg.tool_calls {
            total = total.saturating_add(tool_calls.len() as u32 * TOOL_CALL_OVERHEAD_TOKENS);
            for tc in tool_calls {
                total = total
                    .saturating_add(estimate_text_tokens(&tc.name))
                    .saturating_add(estimate_text_tokens(&tc.arguments));
            }
        }
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
        tool_calls: None,
        tool_call_id: None,
        images: None,
        thinking_content: None,
        thinking_duration: None,
        thinking_signature: None,
        knowledge_proposal: None,
    });

    compact_messages
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
        tool_calls: None,
        tool_call_id: None,
        images: None,
        thinking_content: None,
        thinking_duration: None,
        thinking_signature: None,
        knowledge_proposal: None,
    }
}

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
            tool_calls,
            tool_call_id: tool_call_id.map(|s| s.to_string()),
            images: None,
            thinking_content: None,
            thinking_duration: None,
            thinking_signature: None,
            knowledge_proposal: None,
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
    fn compact_prompt_requires_all_user_messages() {
        assert!(COMPACT_PROMPT.contains("All User Messages"));
        assert!(COMPACT_PROMPT.contains("every non-tool user message"));
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
