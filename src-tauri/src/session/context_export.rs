use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agent::instance::RawRound;
use crate::session::models::{
    ChatMessage, MessageRole, PendingSessionInput, SessionContextAttempt, SessionRuntimeSnapshot,
    ToolCallInfo,
};
use crate::session::store::SessionStore;

const EXPORT_FORMAT: &str = "locus.context_review";
const EXPORT_FORMAT_VERSION: u32 = 2;
const EMPTY: &str = "empty";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextExportResult {
    pub file_path: String,
    pub capture_quality: String,
    pub session_count: usize,
    pub attempt_count: usize,
}

#[derive(Debug, Clone)]
pub struct ContextExportLiveSnapshot {
    pub captured_at: i64,
    pub sessions: HashMap<String, ContextExportLiveSession>,
}

#[derive(Debug, Clone)]
pub struct ContextExportLiveSession {
    pub pending_inputs: Vec<PendingSessionInput>,
    pub runtime: Option<SessionRuntimeSnapshot>,
}

#[derive(Serialize)]
struct ContextExportDocument {
    format: &'static str,
    format_version: u32,
    export: ExportMetadata,
    source: SourceMetadata,
    sessions: Vec<SessionExport>,
    integrity: ExportIntegrity,
}

#[derive(Serialize)]
struct ExportMetadata {
    created_at: String,
    producer: String,
    database_schema_version: i32,
    capture_quality: String,
    missing_fields: Vec<MissingField>,
    redactions: Vec<String>,
    consistency: ExportConsistency,
}

#[derive(Serialize)]
struct ExportConsistency {
    database_copy: &'static str,
    database_snapshot_at: Value,
    runtime_snapshot_at: Value,
}

#[derive(Serialize)]
struct MissingField {
    field: String,
    value: &'static str,
    reason: String,
}

#[derive(Serialize)]
struct SourceMetadata {
    root_session_id: String,
    session_tree_ids: Vec<String>,
    workspace_path: Value,
    workspace_path_state: &'static str,
}

#[derive(Serialize)]
struct SessionExport {
    metadata: Value,
    token_usage: Value,
    todos: Value,
    pending_inputs: Value,
    runtime: Value,
    messages: Vec<Value>,
    compactions: Value,
    context_attempts: Value,
    timeline: Vec<crate::session::models::SessionEventRecord>,
}

#[derive(Serialize)]
struct ContextAttemptExport {
    id: String,
    run_id: String,
    iteration: u32,
    attempt: u32,
    attempt_kind: String,
    status: String,
    backend: String,
    model_id: String,
    effort: Value,
    created_at: String,
    prompt: AttemptPromptExport,
    context_budget: AttemptContextBudgetExport,
    provider_request: Value,
    provider_response_format: &'static str,
    provider_response: Value,
    provider_response_raw: Value,
    error_message: Value,
}

#[derive(Serialize)]
struct AttemptPromptExport {
    system: Value,
    messages: Value,
    tools: Value,
    tool_search: Value,
    model: Value,
}

#[derive(Serialize)]
struct AttemptContextBudgetExport {
    unit: &'static str,
    denominator_chars: usize,
    system_chars: usize,
    history_chars: usize,
    tool_schema_chars: usize,
    tool_result_chars: usize,
    system_share_percent: f64,
    history_share_percent: f64,
    tool_schema_share_percent: f64,
    tool_result_share_percent: f64,
    largest_tool_results: Vec<ToolResultContextShareExport>,
}

#[derive(Serialize)]
struct ToolResultContextShareExport {
    path: String,
    tool_call_id: Value,
    chars: usize,
    prompt_share_percent: f64,
}

#[derive(Serialize)]
struct ExportIntegrity {
    algorithm: &'static str,
    scope: &'static str,
    content_hash: String,
}

pub fn export_session_context_yaml(
    store: &SessionStore,
    root_session_id: &str,
    workspace_path: &str,
    legacy_raw_rounds: Option<&[RawRound]>,
    live_snapshot: Option<&ContextExportLiveSnapshot>,
    file_path: &Path,
) -> Result<ContextExportResult, String> {
    let session_tree_ids = store.session_tree_ids(root_session_id)?;
    if session_tree_ids.is_empty() {
        return Err(format!("Session not found: {}", root_session_id));
    }

    let mut sessions = Vec::with_capacity(session_tree_ids.len());
    let mut attempt_count = 0usize;
    let mut has_persisted_attempts = false;
    let mut used_legacy_raw_rounds = false;
    let mut has_context_capture_gap = false;

    for session_id in &session_tree_ids {
        has_context_capture_gap |= store.session_has_context_capture_gap(session_id)?;
        let detail = store.load_session(session_id)?;
        let live_session = live_snapshot.and_then(|snapshot| snapshot.sessions.get(session_id));
        let usage = store.get_token_usage(session_id).ok();
        let todos = store.get_todos(session_id).ok();
        let mut messages = detail.messages.clone();
        expand_persisted_outputs(store, &mut messages);
        let messages = messages
            .into_iter()
            .map(export_message)
            .collect::<Result<Vec<_>, _>>()?;
        let compactions = store.list_compacted_context_outputs(session_id)?;
        let compactions = if compactions.is_empty() {
            empty_value()
        } else {
            serde_json::to_value(compactions)
                .map_err(|error| format!("Failed to serialize compacted contexts: {}", error))?
        };

        let mut attempts = store.list_context_attempts(session_id)?;
        if !attempts.is_empty() {
            has_persisted_attempts = true;
        } else if session_id == root_session_id {
            if let Some(rounds) = legacy_raw_rounds.filter(|rounds| !rounds.is_empty()) {
                attempts = legacy_attempts(session_id, rounds);
                used_legacy_raw_rounds = true;
            }
        }
        attempt_count += attempts.len();
        let context_attempts = if attempts.is_empty() {
            empty_value()
        } else {
            serde_json::to_value(attempts.into_iter().map(export_attempt).collect::<Vec<_>>())
                .map_err(|error| format!("Failed to serialize context attempts: {}", error))?
        };
        let timeline = load_all_events(store, session_id)?;

        let metadata = json!({
            "sessionId": detail.id,
            "title": non_empty_value(Some(&detail.title)),
            "agentId": non_empty_value(detail.agent_id.as_deref()),
            "lastModelId": non_empty_value(detail.last_model_id.as_deref()),
            "lastEffort": non_empty_value(detail.last_effort.as_deref()),
            "sessionType": non_empty_value(Some(&detail.session_type)),
            "parentSessionId": non_empty_value(detail.parent_session_id.as_deref()),
            "latestCompletedRunId": non_empty_value(detail.latest_completed_run_id.as_deref()),
            "createdAtUnix": detail.created_at,
            "createdAt": format_timestamp(detail.created_at),
            "updatedAtUnix": detail.updated_at,
            "updatedAt": format_timestamp(detail.updated_at),
        });
        let token_usage = usage
            .and_then(|value| serde_json::to_value(value).ok())
            .unwrap_or_else(empty_value);
        let todos = todos
            .and_then(|value| serde_json::to_value(value).ok())
            .unwrap_or_else(empty_value);
        let pending_inputs = live_session
            .map(|session| session.pending_inputs.as_slice())
            .unwrap_or(detail.pending_inputs.as_slice());
        let pending_inputs = if pending_inputs.is_empty() {
            empty_value()
        } else {
            serde_json::to_value(pending_inputs).unwrap_or_else(|_| empty_value())
        };
        let runtime = live_session
            .and_then(|session| session.runtime.as_ref())
            .and_then(|runtime| serde_json::to_value(runtime).ok())
            .unwrap_or_else(empty_value);

        sessions.push(SessionExport {
            metadata,
            token_usage,
            todos,
            pending_inputs,
            runtime,
            messages,
            compactions,
            context_attempts,
            timeline,
        });
    }

    let capture_quality = if has_context_capture_gap {
        if has_persisted_attempts || used_legacy_raw_rounds {
            "partial"
        } else {
            "reconstructed"
        }
    } else if used_legacy_raw_rounds {
        "partial"
    } else {
        "full"
    }
    .to_string();
    let missing_fields = if has_context_capture_gap {
        vec![
            MissingField {
                field: "sessions[].contextAttempts".to_string(),
                value: EMPTY,
                reason: "session predates persisted context-attempt capture".to_string(),
            },
            MissingField {
                field: "historicalSystemPrompt".to_string(),
                value: EMPTY,
                reason: "the exact historical prompt was not persisted".to_string(),
            },
            MissingField {
                field: "historicalToolCatalog".to_string(),
                value: EMPTY,
                reason: "the exact historical tool catalog was not persisted".to_string(),
            },
        ]
    } else {
        Vec::new()
    };

    let workspace_path = if workspace_path.trim().is_empty() {
        empty_value()
    } else {
        Value::String(workspace_path.trim().to_string())
    };
    let mut document = ContextExportDocument {
        format: EXPORT_FORMAT,
        format_version: EXPORT_FORMAT_VERSION,
        export: ExportMetadata {
            created_at: Utc::now().to_rfc3339(),
            producer: format!("Locus {}", env!("CARGO_PKG_VERSION")),
            database_schema_version: SessionStore::schema_version(),
            capture_quality: capture_quality.clone(),
            missing_fields,
            redactions: Vec::new(),
            consistency: ExportConsistency {
                database_copy: if store.export_snapshot_created_at().is_some() {
                    "sqlite_online_backup"
                } else {
                    "direct_read"
                },
                database_snapshot_at: store
                    .export_snapshot_created_at()
                    .map(format_timestamp)
                    .map(Value::String)
                    .unwrap_or_else(empty_value),
                runtime_snapshot_at: live_snapshot
                    .map(|snapshot| format_timestamp(snapshot.captured_at))
                    .map(Value::String)
                    .unwrap_or_else(empty_value),
            },
        },
        source: SourceMetadata {
            root_session_id: root_session_id.to_string(),
            session_tree_ids,
            workspace_path,
            workspace_path_state: "current_at_export",
        },
        sessions,
        integrity: ExportIntegrity {
            algorithm: "sha256",
            scope: "document with integrity.contentHash set to empty",
            content_hash: EMPTY.to_string(),
        },
    };

    let unhashed = serialize_document(&document)?;
    document.integrity.content_hash = format!("{:x}", Sha256::digest(unhashed.as_bytes()));
    let yaml = serialize_document(&document)?;
    write_atomic(file_path, yaml.as_bytes())?;

    Ok(ContextExportResult {
        file_path: file_path.display().to_string(),
        capture_quality,
        session_count: document.sessions.len(),
        attempt_count,
    })
}

fn export_message(message: ChatMessage) -> Result<Value, String> {
    fn optional_json<T: Serialize>(value: Option<T>) -> Result<Value, String> {
        value
            .map(|value| {
                serde_json::to_value(value).map_err(|error| {
                    format!("Failed to serialize exported message field: {}", error)
                })
            })
            .transpose()
            .map(|value| value.unwrap_or_else(empty_value))
    }

    Ok(json!({
        "id": message.id,
        "role": match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        },
        "content": message.content,
        "createdAtUnix": message.created_at,
        "createdAt": format_timestamp(message.created_at),
        "promptPrefix": non_empty_value(message.prompt_prefix.as_deref()),
        "promptSuffix": non_empty_value(message.prompt_suffix.as_deref()),
        "responseId": non_empty_value(message.response_id.as_deref()),
        "contentOrder": optional_json(message.content_order)?,
        "thinkingOrder": optional_json(message.thinking_order)?,
        "toolCalls": optional_json(message.tool_calls)?,
        "toolCallId": non_empty_value(message.tool_call_id.as_deref()),
        "images": optional_json(message.images)?,
        "assetRefs": optional_json(message.asset_refs)?,
        "thinkingContent": non_empty_value(message.thinking_content.as_deref()),
        "thinkingDuration": optional_json(message.thinking_duration)?,
        "thinkingSignature": non_empty_value(message.thinking_signature.as_deref()),
        "knowledgeProposal": optional_json(message.knowledge_proposal)?,
        "renderParts": optional_json(message.render_parts)?,
    }))
}

fn export_attempt(attempt: SessionContextAttempt) -> ContextAttemptExport {
    let request = attempt.request;
    let (provider_response_format, provider_response) = decode_provider_response(&attempt.response);
    let system = request_field(&request, &["system", "instructions"]);
    let messages = request_field(&request, &["messages", "input"]);
    let tools = request_field(&request, &["tools"]);
    let tool_search = request_tool_search(&request);
    let context_budget = attempt_context_budget(&system, &messages, &tools, &tool_search);
    let prompt = AttemptPromptExport {
        system,
        messages,
        tools,
        tool_search,
        model: request_field(&request, &["model"]),
    };
    ContextAttemptExport {
        id: attempt.id,
        run_id: attempt.run_id,
        iteration: attempt.iteration,
        attempt: attempt.attempt,
        attempt_kind: attempt.attempt_kind,
        status: attempt.status,
        backend: attempt.backend,
        model_id: attempt.model_id,
        effort: non_empty_value(attempt.effort.as_deref()),
        created_at: format_timestamp(attempt.created_at),
        prompt,
        context_budget,
        provider_request: request,
        provider_response_format,
        provider_response,
        provider_response_raw: if attempt.response.is_empty() {
            empty_value()
        } else {
            Value::String(attempt.response)
        },
        error_message: non_empty_value(attempt.error_message.as_deref()),
    }
}

fn attempt_context_budget(
    system: &Value,
    messages: &Value,
    tools: &Value,
    tool_search: &Value,
) -> AttemptContextBudgetExport {
    let system_chars = serialized_chars(system);
    let message_chars = serialized_chars(messages);
    let tool_schema_chars = serialized_chars(tools) + serialized_chars(tool_search);
    let mut tool_results = Vec::new();
    collect_tool_result_context(messages, "prompt.messages", &mut tool_results);
    let tool_result_chars = tool_results.iter().map(|item| item.2).sum::<usize>();
    let history_chars = message_chars.saturating_sub(tool_result_chars);
    let denominator_chars = system_chars + history_chars + tool_schema_chars + tool_result_chars;
    tool_results.sort_by(|left, right| right.2.cmp(&left.2));
    let largest_tool_results = tool_results
        .into_iter()
        .take(5)
        .map(|(path, tool_call_id, chars)| ToolResultContextShareExport {
            path,
            tool_call_id,
            chars,
            prompt_share_percent: percentage(chars, denominator_chars),
        })
        .collect();

    AttemptContextBudgetExport {
        unit: "serialized_json_characters_proxy",
        denominator_chars,
        system_chars,
        history_chars,
        tool_schema_chars,
        tool_result_chars,
        system_share_percent: percentage(system_chars, denominator_chars),
        history_share_percent: percentage(history_chars, denominator_chars),
        tool_schema_share_percent: percentage(tool_schema_chars, denominator_chars),
        tool_result_share_percent: percentage(tool_result_chars, denominator_chars),
        largest_tool_results,
    }
}

fn serialized_chars(value: &Value) -> usize {
    if value.as_str() == Some(EMPTY) {
        return 0;
    }
    serde_json::to_string(value)
        .map(|serialized| serialized.chars().count())
        .unwrap_or(0)
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 10_000.0).round() / 100.0
}

fn collect_tool_result_context(
    value: &Value,
    path: &str,
    results: &mut Vec<(String, Value, usize)>,
) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_tool_result_context(item, &format!("{}[{}]", path, index), results);
            }
        }
        Value::Object(object) => {
            let role_is_tool = object.get("role").and_then(Value::as_str) == Some("tool");
            let type_is_tool_result =
                object
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(kind, "tool_result" | "function_call_output" | "tool_output")
                    });
            if role_is_tool || type_is_tool_result {
                let tool_call_id = ["tool_call_id", "tool_use_id", "call_id", "id"]
                    .iter()
                    .find_map(|key| object.get(*key).cloned())
                    .unwrap_or_else(empty_value);
                results.push((path.to_string(), tool_call_id, serialized_chars(value)));
                return;
            }
            for (key, child) in object {
                collect_tool_result_context(child, &format!("{}.{}", path, key), results);
            }
        }
        _ => {}
    }
}

fn legacy_attempts(session_id: &str, rounds: &[RawRound]) -> Vec<SessionContextAttempt> {
    rounds
        .iter()
        .enumerate()
        .map(|(index, round)| {
            let meta = round.request.get("_locusAttempt");
            let attempt = meta
                .and_then(|value| value.get("attempt"))
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(1);
            let completed = meta
                .and_then(|value| value.get("completed"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            SessionContextAttempt {
                id: format!("legacy-raw-round-{}", index + 1),
                session_id: session_id.to_string(),
                run_id: EMPTY.to_string(),
                iteration: u32::try_from(round.round).unwrap_or_default(),
                attempt,
                attempt_kind: meta
                    .and_then(|value| value.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("normal")
                    .to_string(),
                status: if completed { "completed" } else { "failed" }.to_string(),
                backend: round
                    .request
                    .get("backend")
                    .and_then(Value::as_str)
                    .unwrap_or(EMPTY)
                    .to_string(),
                model_id: round
                    .request
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(EMPTY)
                    .to_string(),
                effort: None,
                request: round.request.clone(),
                response: round.response.clone(),
                error_message: (!completed).then(|| round.response.clone()),
                created_at: round.timestamp,
            }
        })
        .collect()
}

fn expand_persisted_outputs(store: &SessionStore, messages: &mut [ChatMessage]) {
    for message in messages {
        if message.role == MessageRole::Tool {
            message.content = store.expand_persisted_tool_output_for_export(&message.content);
        }
        if let Some(tool_calls) = message.tool_calls.as_mut() {
            expand_tool_calls(store, tool_calls);
        }
    }
}

fn expand_tool_calls(store: &SessionStore, tool_calls: &mut [ToolCallInfo]) {
    for tool_call in tool_calls {
        if let Some(output) = tool_call.recorded_output.as_mut() {
            *output = store.expand_persisted_tool_output_for_export(output);
        }
        if let Some(output) = tool_call.server_tool_output.as_mut() {
            *output = store.expand_persisted_tool_output_for_export(output);
        }
        if let Some(nested) = tool_call.nested_tool_calls.as_mut() {
            expand_tool_calls(store, nested);
        }
    }
}

fn load_all_events(
    store: &SessionStore,
    session_id: &str,
) -> Result<Vec<crate::session::models::SessionEventRecord>, String> {
    let mut events = Vec::new();
    let mut after_seq = 0i64;
    loop {
        let page = store.list_session_events(session_id, Some(after_seq), Some(2_000))?;
        if page.is_empty() {
            break;
        }
        after_seq = page.last().map(|event| event.seq).unwrap_or(after_seq);
        let page_len = page.len();
        events.extend(page);
        if page_len < 2_000 {
            break;
        }
    }
    Ok(events)
}

fn request_field(request: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| request.get(*key))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(empty_value)
}

fn request_tool_search(request: &Value) -> Value {
    let direct = request_field(request, &["tool_search", "toolSearch"]);
    if direct != empty_value() {
        return direct;
    }
    request
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.get("type").and_then(Value::as_str) == Some("tool_search"))
        })
        .cloned()
        .unwrap_or_else(empty_value)
}

fn decode_provider_response(raw: &str) -> (&'static str, Value) {
    if raw.trim().is_empty() {
        return ("empty", empty_value());
    }
    if let Ok(value) = serde_json::from_str(raw) {
        return ("json", value);
    }
    let events = parse_sse_events(raw);
    if !events.is_empty() {
        return ("sse", Value::Array(events));
    }
    ("text", Value::String(raw.to_string()))
}

fn parse_sse_events(raw: &str) -> Vec<Value> {
    let normalized = raw.replace("\r\n", "\n");
    normalized
        .split("\n\n")
        .filter_map(|block| {
            let mut event_name = None;
            let mut data_lines = Vec::new();
            let mut has_sse_field = false;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    has_sse_field = true;
                    event_name = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    has_sse_field = true;
                    data_lines.push(value.trim_start());
                }
            }
            if !has_sse_field || data_lines.is_empty() {
                return None;
            }
            let data_raw = data_lines.join("\n");
            let data =
                serde_json::from_str(&data_raw).unwrap_or_else(|_| Value::String(data_raw.clone()));
            Some(json!({
                "event": event_name.unwrap_or_else(|| EMPTY.to_string()),
                "data": data,
            }))
        })
        .collect()
}

fn non_empty_value(value: Option<&str>) -> Value {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or_else(empty_value)
}

fn empty_value() -> Value {
    Value::String(EMPTY.to_string())
}

fn format_timestamp(timestamp: i64) -> String {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| EMPTY.to_string())
}

fn serialize_document(document: &ContextExportDocument) -> Result<String, String> {
    let body = serde_yaml::to_string(document)
        .map_err(|error| format!("Failed to serialize context export: {}", error))?;
    Ok(format!("---\n{}", body))
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Context export path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create '{}': {}", parent.display(), error))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Invalid context export path: {}", path.display()))?;
    let temp_path = path.with_file_name(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4()));
    let mut temp_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| format!("Failed to create '{}': {}", temp_path.display(), error))?;
    if let Err(error) = temp_file
        .write_all(contents)
        .and_then(|_| temp_file.sync_all())
    {
        drop(temp_file);
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "Failed to write '{}': {}",
            temp_path.display(),
            error
        ));
    }
    drop(temp_file);

    #[cfg(target_os = "windows")]
    let replace_result = {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        use windows_core::PCWSTR;

        let source = temp_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| error.to_string())
    };

    #[cfg(not(target_os = "windows"))]
    let replace_result = std::fs::rename(&temp_path, path).map_err(|error| error.to_string());

    replace_result.map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to replace '{}': {}", path.display(), error)
    })
}

pub fn default_review_export_path(
    app_temp_dir: &Path,
    session_id: &str,
    session_title: &str,
) -> Result<PathBuf, String> {
    let safe_id = session_id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric() || *value == '-' || *value == '_')
        .collect::<String>();
    if safe_id.is_empty() {
        return Err("Session ID cannot produce a context export file name".to_string());
    }
    let safe_title = context_export_title_fragment(session_title);
    Ok(app_temp_dir.join("context-reviews").join(format!(
        "context-{}-{}-{}.yaml",
        safe_id,
        safe_title,
        Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
    )))
}

fn context_export_title_fragment(title: &str) -> String {
    let mut output = String::new();
    let mut previous_was_separator = false;
    for ch in title.trim().chars() {
        let invalid =
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*');
        if invalid || ch.is_whitespace() || ch == '_' {
            if !output.is_empty() && !previous_was_separator {
                output.push('_');
                previous_was_separator = true;
            }
            continue;
        }
        output.push(ch);
        previous_was_separator = false;
        if output.chars().count() >= 72 {
            break;
        }
    }
    let trimmed = output.trim_matches(|ch: char| ch == '.' || ch == '_' || ch.is_whitespace());
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::models::{MessageRole, SessionRunSummary, SessionRuntimeSnapshot};
    use tempfile::tempdir;

    #[test]
    fn exports_parseable_yaml_for_session_without_provider_attempts() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("create store");
        let session_id = store
            .create_session("Legacy", None, Some("workspace"), "chat", Some("dev"))
            .expect("create session");
        store
            .add_message(&session_id, MessageRole::User, "Review this")
            .expect("add message");
        let output = dir.path().join("context.yaml");

        let result =
            export_session_context_yaml(&store, &session_id, "F:/Project", None, None, &output)
                .expect("export context");

        assert_eq!(result.capture_quality, "full");
        let raw = std::fs::read_to_string(output).expect("read export");
        let yaml: serde_yaml::Value = serde_yaml::from_str(&raw).expect("parse yaml");
        assert_eq!(yaml["format"].as_str(), Some(EXPORT_FORMAT));
        assert_eq!(yaml["export"]["capture_quality"].as_str(), Some("full"));
        assert_eq!(
            yaml["sessions"][0]["context_attempts"].as_str(),
            Some(EMPTY)
        );
        assert_eq!(yaml["sessions"][0]["compactions"].as_str(), Some(EMPTY));
    }

    #[test]
    fn exports_persisted_context_attempts_after_store_reopen() {
        let dir = tempdir().expect("create temp dir");
        let session_id;
        {
            let store = SessionStore::new(dir.path()).expect("create store");
            session_id = store
                .create_session("Captured", None, None, "chat", Some("dev"))
                .expect("create session");
            store
                .record_context_attempt(
                    &session_id,
                    "run-1",
                    1,
                    1,
                    "normal",
                    "completed",
                    "openai_codex",
                    "gpt-test",
                    Some("high"),
                    &json!({
                        "model": "gpt-test",
                        "instructions": "system",
                        "input": [{"role": "user", "content": "hello"}],
                        "tools": [{"type": "function", "name": "read"}],
                    }),
                    "response",
                    None,
                )
                .expect("record attempt");
        }

        let store = SessionStore::new(dir.path()).expect("reopen store");
        let output = dir.path().join("captured.yaml");
        let result = export_session_context_yaml(&store, &session_id, "", None, None, &output)
            .expect("export context");
        assert_eq!(result.capture_quality, "full");
        assert_eq!(result.attempt_count, 1);
        let raw = std::fs::read_to_string(output).expect("read export");
        assert!(raw.contains("instructions: system"));
        assert!(raw.contains("inputSchema") || raw.contains("tools:"));
    }

    #[test]
    fn exports_online_backup_with_separately_timestamped_live_runtime() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("create store");
        let session_id = store
            .create_session("Running", None, None, "chat", Some("dev"))
            .expect("create session");
        store
            .add_message(&session_id, MessageRole::User, "before snapshot")
            .expect("add snapshot message");
        let snapshot = store
            .create_export_snapshot()
            .expect("create online backup");
        store
            .add_message(&session_id, MessageRole::Assistant, "after snapshot")
            .expect("add live-only message");

        let runtime = SessionRuntimeSnapshot {
            active_run: SessionRunSummary {
                run_id: "run-live".to_string(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                started_at: 100,
                updated_at: 101,
                finished_at: None,
                error_message: None,
            },
            active_tool_calls: Vec::new(),
            streaming_text: "live answer".to_string(),
            streaming_thinking: "live reasoning".to_string(),
            live_render_parts: Vec::new(),
            stream_sequence: 4,
            streaming_text_order: 2,
            thinking_order: 1,
            is_thinking: true,
            thinking_duration: 3,
            pending_question: None,
            pending_tool_confirms: Vec::new(),
            is_compacting: false,
        };
        let live = ContextExportLiveSnapshot {
            captured_at: 102,
            sessions: HashMap::from([(
                session_id.clone(),
                ContextExportLiveSession {
                    pending_inputs: Vec::new(),
                    runtime: Some(runtime),
                },
            )]),
        };
        let output = dir.path().join("runtime.yaml");
        export_session_context_yaml(&snapshot, &session_id, "", None, Some(&live), &output)
            .expect("export runtime context");

        let raw = std::fs::read_to_string(output).expect("read runtime export");
        let yaml: serde_yaml::Value = serde_yaml::from_str(&raw).expect("parse runtime export");
        assert_eq!(
            yaml["export"]["consistency"]["database_copy"].as_str(),
            Some("sqlite_online_backup")
        );
        assert_ne!(
            yaml["export"]["consistency"]["database_snapshot_at"].as_str(),
            Some(EMPTY)
        );
        assert_ne!(
            yaml["export"]["consistency"]["runtime_snapshot_at"].as_str(),
            Some(EMPTY)
        );
        assert_eq!(
            yaml["sessions"][0]["runtime"]["streamingText"].as_str(),
            Some("live answer")
        );
        let messages = yaml["sessions"][0]["messages"]
            .as_sequence()
            .expect("exported messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"].as_str(), Some("before snapshot"));
        assert_eq!(messages[0]["promptPrefix"].as_str(), Some(EMPTY));
        assert_eq!(messages[0]["toolCalls"].as_str(), Some(EMPTY));
        assert_eq!(messages[0]["thinkingContent"].as_str(), Some(EMPTY));
    }

    #[test]
    fn structures_json_and_sse_provider_responses_without_losing_raw_text() {
        let (format, value) = decode_provider_response(r#"{"id":"response-1"}"#);
        assert_eq!(format, "json");
        assert_eq!(value["id"].as_str(), Some("response-1"));

        let raw =
            "event: response.output_text.delta\ndata: {\"delta\":\"hello\"}\n\ndata: [DONE]\n\n";
        let (format, value) = decode_provider_response(raw);
        assert_eq!(format, "sse");
        let events = value.as_array().expect("SSE event list");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["data"]["delta"].as_str(), Some("hello"));
        assert_eq!(events[1]["data"].as_str(), Some("[DONE]"));
    }

    #[test]
    fn measures_tool_result_context_share_without_double_counting_nested_payloads() {
        let system = json!("system prompt");
        let messages = json!([
            {"role": "user", "content": "inspect"},
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "call-1",
                        "content": "large result"
                    }
                ]
            },
            {
                "type": "function_call_output",
                "call_id": "call-2",
                "output": "second result"
            }
        ]);
        let tools = json!([{"name": "read", "description": "Read a file"}]);
        let budget = attempt_context_budget(&system, &messages, &tools, &empty_value());

        assert!(budget.denominator_chars > 0);
        assert!(budget.tool_result_chars > 0);
        assert_eq!(budget.largest_tool_results.len(), 2);
        assert!(budget
            .largest_tool_results
            .iter()
            .any(|result| result.tool_call_id.as_str() == Some("call-1")));
        assert!(budget
            .largest_tool_results
            .iter()
            .any(|result| result.tool_call_id.as_str() == Some("call-2")));
        let share_total = budget.system_share_percent
            + budget.history_share_percent
            + budget.tool_schema_share_percent
            + budget.tool_result_share_percent;
        assert!((share_total - 100.0).abs() < 0.05);
    }

    #[test]
    fn review_export_path_includes_session_id_and_sanitized_title() {
        let path = default_review_export_path(
            Path::new("C:/Temp"),
            "6201ad9e-1234",
            "场景: Player/A*?  .",
        )
        .expect("build review export path");
        let file_name = path.file_name().and_then(|value| value.to_str()).unwrap();
        assert!(file_name.starts_with("context-6201ad9e-1234-场景_Player_A-"));
        assert!(file_name.ends_with(".yaml"));
    }
}
