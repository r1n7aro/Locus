mod anthropic_agent_sdk;
mod backend;
mod prompt_context;
mod read_file;
mod unity_capture;

pub use backend::resolve_openrouter_model;
pub use backend::{LlmBackend, RawContextStore, RawRound};

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use tauri::{AppHandle, Manager};

use crate::agent::definition::{AgentDef, AgentDefRegistry};
use crate::commands::{
    BasicToolConfirmDisplay, KnowledgeToolConfirmDirectoryMode, KnowledgeToolConfirmOperation,
    KnowledgeToolConfirmPreview, StreamEvent, ToolConfirmDisplay,
    UnityEditorStatusChangeConfirmDisplay,
};
use crate::compact;
use crate::llm::{anthropic, chat_completions, codex, openrouter, responses};
use crate::session::models::{
    AssistantRenderPart, ChatMessage, ImageData, MessageRole, PendingSessionInput, RenderOrderKey,
    TodoItem, ToolCallInfo,
};
use crate::session::store::SessionStore;
use crate::tool::{ToolExecutionContext, ToolLoadMode, ToolRegistry, ToolResult, ToolRuntimeState};

use backend::{
    is_prompt_too_long_error, is_retryable_llm_error, model_context_limit, normalize_tool_args,
    session_unity_state, LlmCallResult, MAX_TOOL_ITERATIONS,
};
use prompt_context::{
    detect_input_system, detect_render_pipeline, parse_physics_config, parse_tag_manager,
};

fn is_codex_unauthorized_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("401 unauthorized")
        || lower.contains("http error: 401")
        || lower.contains("api error (401")
}

fn is_recoverable_compact_llm_error(error: &str) -> bool {
    is_prompt_too_long_error(error) || is_tool_call_output_reference_error(error)
}

fn is_tool_call_output_reference_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("no tool call found for function call output")
        || (lower.contains("no tool call found") && lower.contains("function_call_output"))
}

fn messages_have_images(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .any(|msg| msg.images.as_ref().is_some_and(|images| !images.is_empty()))
}

fn no_vision_endpoint_error() -> String {
    "This model endpoint is configured without image understanding. Enable Image understanding in the custom endpoint settings or select a vision-capable model before using screenshots or image attachments.".to_string()
}

fn is_vision_unsupported_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let image_term = lower.contains("image")
        || lower.contains("vision")
        || lower.contains("multimodal")
        || lower.contains("modality")
        || lower.contains("image_url");
    image_term
        && (lower.contains("unsupported")
            || lower.contains("not support")
            || lower.contains("does not support")
            || lower.contains("not allowed")
            || lower.contains("invalid")
            || lower.contains("unknown content type")
            || lower.contains("content type"))
}

fn user_friendly_llm_error(error: &str) -> String {
    if is_vision_unsupported_error(error) {
        return no_vision_endpoint_error();
    }
    error.to_string()
}

async fn resolve_codex_request_auth(
    auth: &crate::commands::CodexAuthStateHandle,
    force_refresh: bool,
) -> Result<(String, Option<String>), String> {
    let mut guard = auth.lock().await;
    if force_refresh {
        guard.retry_validation().await?;
    }
    let access_token = guard.access_token().await?;
    Ok((access_token, guard.account_id()))
}

/// Emit a StreamEvent through the session gateway with the given run_id.
fn emit_stream(handle: &AppHandle, run_id: &str, event: StreamEvent) {
    let store: tauri::State<'_, Arc<SessionStore>> = handle.state();
    crate::session::gateway::emit_stream(handle, store.inner().as_ref(), run_id, event);
}

fn emit_tool_progress(
    handle: &AppHandle,
    run_id: &str,
    session_id: &str,
    tool_call_id: &str,
    title: impl Into<String>,
    info: impl Into<String>,
    progress: Option<f32>,
    state: impl Into<String>,
) {
    emit_stream(
        handle,
        run_id,
        StreamEvent::ToolCallProgress {
            session_id: session_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            title: title.into(),
            info: info.into(),
            progress,
            state: state.into(),
        },
    );
}

fn log_stage_elapsed(
    agent_id: &str,
    session_id: &str,
    run_id: &str,
    stage: &str,
    started_at: Instant,
) {
    eprintln!(
        "[Agent {}] stage={} session={} run={} elapsed_ms={}",
        agent_id,
        stage,
        session_id,
        run_id,
        started_at.elapsed().as_millis()
    );
}

fn push_unique_tool_name(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_string());
    }
}

fn requested_tool_load_names(arguments: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };
    value
        .get("tools")
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.as_str())
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_meta_tool_call_arguments(arguments: &str) -> Result<(String, serde_json::Value), String> {
    let value = serde_json::from_str::<serde_json::Value>(arguments)
        .map_err(|e| format!("tool_call arguments must be valid JSON: {}", e))?;
    let tool_name = value
        .get("toolName")
        .or_else(|| value.get("tool_name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "tool_call requires a non-empty toolName".to_string())?
        .to_string();
    let target_args = value
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !target_args.is_object() {
        return Err("tool_call.arguments must be an object".to_string());
    }
    Ok((tool_name, target_args))
}

pub struct AgentInstance {
    #[allow(dead_code)]
    id: String,
    def: Arc<AgentDef>,
    effective_model: String,
    session_id: String,
    backend: LlmBackend,
    debug: bool,
    #[allow(dead_code)]
    registry: Arc<AgentDefRegistry>,
    tool_registry: Arc<ToolRegistry>,
    working_dir: String,
    raw_store: RawContextStore,
    workspace_id: Option<String>,
    parent_tool_call: Option<ParentToolCall>,
    effort: Option<String>,
    app_knowledge_dir: Arc<Option<std::path::PathBuf>>,
    app_agent_dir: Arc<Option<std::path::PathBuf>>,
    knowledge_access_mode: KnowledgeAccessMode,
    undo_manager: Option<Arc<crate::vcs::UndoManager>>,
    subagent_model_overrides: std::collections::HashMap<String, String>,
    tool_runtime_state: Arc<ToolRuntimeState>,
    loaded_tool_names: Arc<tokio::sync::Mutex<HashSet<String>>>,
    partial_assistant: Arc<AssistantStreamState>,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnowledgeAccessMode {
    Disabled,
    ReadOnly,
    Full,
}

impl Default for KnowledgeAccessMode {
    fn default() -> Self {
        Self::Full
    }
}

impl KnowledgeAccessMode {
    pub fn from_request(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            None | Some("full") => Ok(Self::Full),
            Some("read_only") | Some("readonly") | Some("read-only") => Ok(Self::ReadOnly),
            Some("disabled") | Some("off") => Ok(Self::Disabled),
            Some(other) => Err(format!("Unsupported knowledge mode: {}", other)),
        }
    }

    fn allows_context(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    fn allows_tool(self, name: &str) -> bool {
        if !AgentInstance::is_knowledge_tool_name(name) {
            return true;
        }
        match self {
            Self::Disabled => false,
            Self::ReadOnly => !AgentInstance::is_knowledge_mutation_tool_name(name),
            Self::Full => true,
        }
    }
}

#[derive(Debug, Clone)]
struct ParentToolCall {
    session_id: String,
    run_id: String,
    tool_call_id: String,
}

struct ParentStreamEvent {
    run_id: String,
    event: StreamEvent,
}

#[derive(Debug, Default)]
struct StreamRenderOrderTracker {
    next_seq: u32,
    part_orders: HashMap<String, u32>,
}

#[derive(Debug, Clone)]
struct RenderPartMark {
    id: String,
    seq: u32,
}

impl StreamRenderOrderTracker {
    fn next(&mut self) -> u32 {
        self.next_seq = self.next_seq.saturating_add(1).max(1);
        self.next_seq
    }

    fn mark_part(&mut self, run_id: &str, stable_key: &str) -> RenderPartMark {
        if let Some(seq) = self.part_orders.get(stable_key).copied() {
            return RenderPartMark {
                id: format!("{}:{}", run_id, stable_key),
                seq,
            };
        }
        let seq = self.next();
        self.part_orders.insert(stable_key.to_string(), seq);
        RenderPartMark {
            id: format!("{}:{}", run_id, stable_key),
            seq,
        }
    }

    fn mark_text(&mut self, run_id: &str, block_id: &str) -> RenderPartMark {
        self.mark_part(run_id, &format!("text:{}", block_id))
    }

    fn mark_thinking(&mut self, run_id: &str, block_id: &str) -> RenderPartMark {
        self.mark_part(run_id, &format!("thinking:{}", block_id))
    }

    fn mark_tool(&mut self, run_id: &str, tool_call_id: &str) -> RenderPartMark {
        self.mark_part(run_id, &format!("tool:{}", tool_call_id))
    }

    fn assign_tool_orders_for_run(
        &mut self,
        run_id: &str,
        tool_calls: &[ToolCallInfo],
    ) -> Vec<ToolCallInfo> {
        tool_calls
            .iter()
            .map(|tool_call| self.assign_tool_order(run_id, tool_call))
            .collect()
    }

    fn assign_tool_order(&mut self, run_id: &str, tool_call: &ToolCallInfo) -> ToolCallInfo {
        let mut tool_call = tool_call.clone();
        if tool_call.order.is_none() {
            let mark = self.mark_tool(run_id, &tool_call.id);
            tool_call.order = Some(mark.seq);
        }
        if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_ref() {
            tool_call.nested_tool_calls = Some(
                nested_tool_calls
                    .iter()
                    .map(|nested| self.assign_tool_order(run_id, nested))
                    .collect(),
            );
        }
        tool_call
    }
}

fn render_order_key(run_id: &str, seq: u32) -> RenderOrderKey {
    RenderOrderKey {
        run_id: run_id.to_string(),
        seq,
    }
}

fn assistant_render_parts_for_response(
    run_id: &str,
    text_part: Option<RenderPartMark>,
    text: &str,
    thinking_part: Option<RenderPartMark>,
    thinking_text: &str,
    thinking_duration: Option<u32>,
    thinking_signature: Option<&str>,
    tool_calls: &[ToolCallInfo],
) -> Vec<AssistantRenderPart> {
    let mut parts = Vec::new();
    if let Some(mark) = thinking_part.filter(|_| !thinking_text.is_empty()) {
        parts.push(AssistantRenderPart::Thinking {
            id: mark.id,
            order: render_order_key(run_id, mark.seq),
            content: thinking_text.to_string(),
            active: Some(false),
            duration: thinking_duration,
            signature: thinking_signature
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        });
    }
    if let Some(mark) = text_part.filter(|_| !text.is_empty()) {
        parts.push(AssistantRenderPart::Text {
            id: mark.id,
            order: render_order_key(run_id, mark.seq),
            content: text.to_string(),
        });
    }
    for tool_call in tool_calls {
        if let Some(seq) = tool_call.order {
            parts.push(AssistantRenderPart::ToolCall {
                id: tool_call.id.clone(),
                order: render_order_key(run_id, seq),
                tool_call: tool_call.clone(),
            });
        }
    }
    parts.sort_by(|left, right| render_part_seq(left).cmp(&render_part_seq(right)));
    parts
}

fn render_part_seq(part: &AssistantRenderPart) -> u32 {
    match part {
        AssistantRenderPart::Thinking { order, .. }
        | AssistantRenderPart::Text { order, .. }
        | AssistantRenderPart::ToolCall { order, .. }
        | AssistantRenderPart::KnowledgeProposal { order, .. } => order.seq,
    }
}

impl ParentToolCall {
    fn new(session_id: String, run_id: String, tool_call_id: String) -> Self {
        Self {
            session_id,
            run_id,
            tool_call_id,
        }
    }

    fn tool_call_delta(&self, delta: String) -> ParentStreamEvent {
        ParentStreamEvent {
            run_id: self.run_id.clone(),
            event: StreamEvent::ToolCallDelta {
                session_id: self.session_id.clone(),
                tool_call_id: self.tool_call_id.clone(),
                delta,
            },
        }
    }

    fn subagent_tool_call_start(
        &self,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
        order: Option<u32>,
        part_id: Option<String>,
        render_seq: Option<u32>,
    ) -> ParentStreamEvent {
        ParentStreamEvent {
            run_id: self.run_id.clone(),
            event: StreamEvent::SubagentToolCallStart {
                session_id: self.session_id.clone(),
                parent_tool_call_id: self.tool_call_id.clone(),
                tool_call_id,
                tool_name,
                arguments,
                order,
                part_id,
                render_seq,
            },
        }
    }

    fn subagent_tool_call_done(
        &self,
        tool_call_id: String,
        tool_name: String,
        output: String,
        outcome: crate::commands::ToolCallOutcome,
        images: Option<Vec<ImageData>>,
    ) -> ParentStreamEvent {
        ParentStreamEvent {
            run_id: self.run_id.clone(),
            event: StreamEvent::SubagentToolCallDone {
                session_id: self.session_id.clone(),
                parent_tool_call_id: self.tool_call_id.clone(),
                tool_call_id,
                tool_name,
                output,
                outcome,
                images,
            },
        }
    }
}

fn emit_parent_stream(handle: &AppHandle, event: ParentStreamEvent) {
    emit_stream(handle, &event.run_id, event.event);
}

#[derive(Debug, Default, Clone)]
pub struct AssistantStreamSnapshot {
    pub text: String,
    pub thinking_content: String,
    pub thinking_duration: Option<u32>,
    pub persisted_message_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct AssistantStreamState {
    inner: Mutex<AssistantStreamSnapshot>,
}

#[derive(Debug, Clone)]
pub struct InterruptedAssistantMessage {
    pub message_id: String,
    pub full_text: String,
    pub thinking_content: Option<String>,
    pub thinking_duration: Option<u32>,
}

impl AssistantStreamState {
    pub fn append_text(&self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.persisted_message_id = None;
            inner.text.push_str(delta);
        }
    }

    pub fn append_thinking(&self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.persisted_message_id = None;
            inner.thinking_content.push_str(delta);
        }
    }

    pub fn mark_persisted(
        &self,
        message_id: String,
        text: String,
        thinking_content: Option<String>,
        thinking_duration: Option<u32>,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.text = text;
            inner.thinking_content = thinking_content.unwrap_or_default();
            inner.thinking_duration = thinking_duration;
            inner.persisted_message_id = Some(message_id);
        }
    }

    pub fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = AssistantStreamSnapshot::default();
        }
    }

    pub fn snapshot(&self) -> AssistantStreamSnapshot {
        self.inner
            .lock()
            .map(|inner| inner.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolRunOutcome {
    Done,
    Error,
    Interrupted,
}

#[derive(Debug, Clone)]
pub(super) struct ExecutedToolResult {
    output: String,
    is_error: bool,
    outcome: ToolRunOutcome,
    nested_tool_calls: Option<Vec<ToolCallInfo>>,
    images: Option<Vec<ImageData>>,
}

impl ToolRunOutcome {
    pub(super) fn as_stream_outcome(self) -> crate::commands::ToolCallOutcome {
        match self {
            Self::Done => crate::commands::ToolCallOutcome::Done,
            Self::Error => crate::commands::ToolCallOutcome::Error,
            Self::Interrupted => crate::commands::ToolCallOutcome::Interrupted,
        }
    }
}

impl ExecutedToolResult {
    pub(super) fn from_tool_result(result: ToolResult) -> Self {
        let is_interrupted_placeholder =
            !result.is_error && result.output == crate::session::history::INTERRUPTED_TOOL_RESULT;
        Self {
            outcome: if is_interrupted_placeholder {
                ToolRunOutcome::Interrupted
            } else if result.is_error {
                ToolRunOutcome::Error
            } else {
                ToolRunOutcome::Done
            },
            output: result.output,
            is_error: result.is_error,
            nested_tool_calls: None,
            images: None,
        }
    }

    pub(super) fn into_tool_result(self) -> ToolResult {
        ToolResult {
            output: self.output,
            is_error: self.is_error,
        }
    }

    pub(super) fn with_images(mut self, images: Vec<ImageData>) -> Self {
        if !images.is_empty() {
            self.images = Some(images);
        }
        self
    }

    pub(super) fn with_nested_tool_calls(mut self, nested_tool_calls: Vec<ToolCallInfo>) -> Self {
        if !nested_tool_calls.is_empty() {
            self.nested_tool_calls = Some(nested_tool_calls);
        }
        self
    }
}

pub(super) fn finalize_tool_call_record(
    tool_call: &ToolCallInfo,
    result: Option<&ExecutedToolResult>,
) -> ToolCallInfo {
    let mut finalized = tool_call.clone();

    if finalized.is_server_tool() {
        finalized
            .outcome
            .get_or_insert(crate::commands::ToolCallOutcome::Done);
        return finalized;
    }

    if let Some(result) = result {
        finalized.outcome = Some(result.outcome.as_stream_outcome());
        finalized.nested_tool_calls = result.nested_tool_calls.clone();
    }

    finalized
}

fn validate_llm_tool_calls(tool_calls: &[ToolCallInfo]) -> Result<(), String> {
    for (index, tool_call) in tool_calls.iter().enumerate() {
        let mut missing = Vec::new();
        if tool_call.id.trim().is_empty() {
            missing.push("id");
        }
        if tool_call.name.trim().is_empty() {
            missing.push("name");
        }
        if !missing.is_empty() {
            return Err(format!(
                "LLM returned incomplete tool call metadata at index {}: missing {}",
                index,
                missing.join(", ")
            ));
        }

        let arguments = tool_call.arguments.trim();
        if !arguments.is_empty() {
            serde_json::from_str::<serde_json::Value>(arguments).map_err(|error| {
                format!(
                    "LLM returned malformed arguments for tool '{}' (id={}): {}",
                    tool_call.name, tool_call.id, error
                )
            })?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectedPromptItem {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub content: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentKnowledgeReadArgs {
    path: String,
    #[serde(default)]
    part: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentKnowledgeDocumentContentPatch {
    #[serde(default)]
    summary: Option<Option<String>>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    maintenance_rules: Option<Option<String>>,
}

impl AgentKnowledgeDocumentContentPatch {
    fn is_empty(&self) -> bool {
        self.summary.is_none() && self.body.is_none() && self.maintenance_rules.is_none()
    }

    fn is_noop_for_create(&self) -> bool {
        let summary_empty = match self.summary.as_ref() {
            None => true,
            Some(None) => true,
            Some(Some(text)) => text.trim().is_empty(),
        };
        let body_empty = match self.body.as_ref() {
            None => true,
            Some(text) => text.trim().is_empty(),
        };
        let maintenance_rules_empty = match self.maintenance_rules.as_ref() {
            None => true,
            Some(None) => true,
            Some(Some(text)) => text.trim().is_empty(),
        };

        summary_empty && body_empty && maintenance_rules_empty
    }

    fn into_document_patch(self) -> crate::knowledge_store::KnowledgeDocumentPatch {
        crate::knowledge_store::KnowledgeDocumentPatch {
            summary: self.summary,
            body: self.body.map(Some),
            maintenance_rules: self.maintenance_rules,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentKnowledgeCreateArgs {
    kind: crate::knowledge_store::KnowledgeTargetKind,
    path: String,
    #[serde(default)]
    document: Option<AgentKnowledgeDocumentContentPatch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentKnowledgeEditArgs {
    path: String,
    document: AgentKnowledgeDocumentContentPatch,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentSkillListArgs {
    #[serde(default)]
    source: Option<String>,
}

enum ToolConfirmDecision {
    Allow,
    Deny { feedback: Option<String> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentKnowledgeListItem {
    #[serde(rename = "type")]
    doc_type: crate::knowledge_store::KnowledgeType,
    path: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentKnowledgeSearchHit {
    #[serde(rename = "type")]
    doc_type: crate::knowledge_store::KnowledgeType,
    path: String,
    title: String,
    snippet: String,
    matched_section: Option<crate::knowledge_store::KnowledgeSearchMatchSection>,
    score: f32,
    match_kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentKnowledgeDocumentContent {
    #[serde(rename = "type")]
    doc_type: crate::knowledge_store::KnowledgeType,
    path: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maintenance_rules: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentKnowledgeReadResponse {
    #[serde(flatten)]
    document: AgentKnowledgeDocumentContent,
    part: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentKnowledgeMutationResponse {
    kind: crate::knowledge_store::KnowledgeTargetKind,
    #[serde(rename = "type")]
    doc_type: crate::knowledge_store::KnowledgeType,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document: Option<AgentKnowledgeDocumentContent>,
}

#[derive(Debug, Clone)]
struct KnowledgeToolConfirmAssessment {
    governance_requires_confirm: bool,
    preview: KnowledgeToolConfirmPreview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BashGitKnowledgeAssessment {
    touches_knowledge: bool,
    requires_confirm: bool,
    reconcile_after_success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GitCommandEffect {
    requires_confirm: bool,
    reconcile_after_success: bool,
    broad_scope: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolConfirmReason {
    UserPermission,
    KnowledgeGovernance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionModeSetting {
    Auto,
    Ask,
}

const PERMISSION_BEHAVIOR_UNITY_EDITOR_STATUS_CHANGE: &str = "behavior.unity_editor_status_change";
const PERMISSION_BEHAVIOR_KNOWLEDGE_GOVERNANCE: &str = "behavior.knowledge_governance";

#[derive(Debug, Clone)]
struct UserWaitTarget {
    session_id: String,
    run_id: String,
}

#[derive(Debug, Clone)]
struct ToolConfirmAssessment {
    reasons: Vec<ToolConfirmReason>,
    display: ToolConfirmDisplay,
}

fn preview_has_summary_content(summary: Option<&str>) -> bool {
    summary
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn parent_knowledge_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map(|value| value.trim_matches('/').to_string())
        .filter(|value| !value.is_empty() && value != ".")
}

fn resolve_knowledge_document_target(
    raw_path: &str,
) -> Result<(crate::knowledge_store::KnowledgeType, String), String> {
    let normalized = raw_path.trim().replace('\\', "/");
    let doc_type = crate::knowledge_store::guess_type_from_path(&normalized)
        .ok_or_else(|| "knowledge document target requires a type-prefixed path.".to_string())?;
    crate::commands::require_knowledge_document_path_suffix(&normalized)?;
    let rel_path = crate::knowledge_store::ensure_document_path(
        AgentInstance::strip_knowledge_type_prefix(&normalized),
    )?;
    Ok((doc_type, rel_path))
}

fn resolve_knowledge_directory_target(
    raw_path: &str,
) -> Result<(crate::knowledge_store::KnowledgeType, String), String> {
    let normalized = raw_path.trim().replace('\\', "/");
    let doc_type = crate::knowledge_store::guess_type_from_path(&normalized)
        .ok_or_else(|| "knowledge directory target requires a type-prefixed path.".to_string())?;
    let rel_path = crate::knowledge_store::ensure_directory_path(
        AgentInstance::strip_knowledge_type_prefix(&normalized),
    )?;
    Ok((doc_type, rel_path))
}

fn directory_mode_from_ai_maintained(ai_maintained: bool) -> KnowledgeToolConfirmDirectoryMode {
    if ai_maintained {
        KnowledgeToolConfirmDirectoryMode::Auto
    } else {
        KnowledgeToolConfirmDirectoryMode::Approval
    }
}

fn resolve_existing_directory_mode(
    working_dir: &str,
    doc_type: crate::knowledge_store::KnowledgeType,
    directory_path: Option<&str>,
) -> Result<(String, KnowledgeToolConfirmDirectoryMode), String> {
    let directory_path = directory_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::knowledge_store::ensure_directory_path)
        .transpose()?;

    let config = if let Some(path) = directory_path.as_deref() {
        crate::knowledge_store::read_directory_config(working_dir, doc_type, path)?.config
    } else {
        crate::knowledge_store::default_directory_config_for_type(doc_type)
    };

    Ok((
        directory_path
            .as_deref()
            .map(|path| AgentInstance::prefix_knowledge_tool_path(doc_type, path))
            .unwrap_or_else(|| doc_type.as_str().to_string()),
        directory_mode_from_ai_maintained(config.ai_maintained),
    ))
}

fn resolve_child_directory_mode(
    working_dir: &str,
    doc_type: crate::knowledge_store::KnowledgeType,
    parent_path: Option<&str>,
) -> Result<(String, KnowledgeToolConfirmDirectoryMode), String> {
    let parent_path = parent_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::knowledge_store::ensure_directory_path)
        .transpose()?;
    let config = crate::knowledge_store::effective_child_directory_config(
        working_dir,
        doc_type,
        parent_path.as_deref(),
    )?;
    Ok((
        parent_path
            .as_deref()
            .map(|path| AgentInstance::prefix_knowledge_tool_path(doc_type, path))
            .unwrap_or_else(|| doc_type.as_str().to_string()),
        directory_mode_from_ai_maintained(config.ai_maintained),
    ))
}

fn collect_directory_structure_paths(
    working_dir: &str,
    doc_type: crate::knowledge_store::KnowledgeType,
    path: &str,
) -> Result<Vec<String>, String> {
    let dir_path = crate::knowledge_store::ensure_directory_path(path)?;
    let type_root = crate::knowledge_store::knowledge_root(working_dir).join(doc_type.as_str());
    let target_dir = type_root.join(&dir_path);
    if !target_dir.is_dir() {
        return Err(format!("Knowledge directory not found: {}", dir_path));
    }

    let mut paths = vec![AgentInstance::prefix_knowledge_tool_path(
        doc_type, &dir_path,
    )];
    for entry in walkdir::WalkDir::new(&target_dir)
        .min_depth(1)
        .into_iter()
        .flatten()
    {
        let rel_path = entry
            .path()
            .strip_prefix(&type_root)
            .map_err(|e| format!("Failed to resolve knowledge preview path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");

        if entry.file_type().is_dir() {
            paths.push(AgentInstance::prefix_knowledge_tool_path(
                doc_type, &rel_path,
            ));
            continue;
        }

        let is_markdown = entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if is_markdown {
            paths.push(AgentInstance::prefix_knowledge_tool_path(
                doc_type, &rel_path,
            ));
        }
    }

    paths.sort();
    Ok(paths)
}

fn relocate_structure_paths(paths: &[String], source_path: &str, target_path: &str) -> Vec<String> {
    let source_prefix = format!("{}/", source_path.trim_end_matches('/'));
    let target_prefix = format!("{}/", target_path.trim_end_matches('/'));
    let mut next = paths
        .iter()
        .map(|path| {
            if path == source_path {
                target_path.to_string()
            } else if let Some(suffix) = path.strip_prefix(&source_prefix) {
                format!("{}{}", target_prefix, suffix)
            } else {
                path.clone()
            }
        })
        .collect::<Vec<_>>();
    next.sort();
    next
}

fn build_knowledge_document_create_preview(
    working_dir: &str,
    parsed: &AgentKnowledgeCreateArgs,
) -> Result<KnowledgeToolConfirmPreview, String> {
    let (doc_type, normalized_path) = resolve_knowledge_document_target(&parsed.path)?;
    let parent_path = parent_knowledge_path(&normalized_path);
    let (directory_path, directory_mode) =
        resolve_child_directory_mode(working_dir, doc_type, parent_path.as_deref())?;

    let mut patch = crate::knowledge_store::default_document_create_patch(
        working_dir,
        doc_type,
        &normalized_path,
    )?;
    if let Some(document) = parsed.document.clone() {
        if let Some(summary) = document.summary {
            patch.summary = Some(summary);
        }
        if let Some(body) = document.body {
            patch.body = Some(Some(body));
        }
        if let Some(maintenance_rules) = document.maintenance_rules {
            patch.maintenance_rules = Some(maintenance_rules);
        }
    }
    if doc_type == crate::knowledge_store::KnowledgeType::Memory {
        patch.explicit_maintenance_rules = Some(true);
        let has_rules = patch
            .maintenance_rules
            .as_ref()
            .and_then(|value| value.as_deref())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !has_rules {
            patch.maintenance_rules = Some(
                crate::knowledge_store::default_maintenance_rules_for_type(doc_type)
                    .map(str::to_string),
            );
        }
    }

    let summary = patch.summary.take().unwrap_or(None);
    let body = patch
        .body
        .take()
        .unwrap_or_else(|| Some(String::new()))
        .unwrap_or_default();
    let maintenance_rules = patch.maintenance_rules.take().unwrap_or(None);
    let title = match patch.title.take() {
        Some(title) => title,
        None => crate::knowledge_store::default_document_title_from_path(&normalized_path)
            .unwrap_or_else(|_| normalized_path.clone()),
    };
    let mut document = crate::knowledge_store::KnowledgeDocument {
        id: patch.id.take().unwrap_or_else(|| "__preview__".to_string()),
        doc_type,
        path: normalized_path.clone(),
        title,
        inject_mode: patch.inject_mode.unwrap_or_else(|| {
            crate::knowledge_store::default_document_inject_mode_for_type(doc_type)
        }),
        inherit_inject_mode: patch.inherit_inject_mode.unwrap_or(true),
        inject_mode_source: Default::default(),
        summary_enabled: patch.summary_enabled.unwrap_or_else(|| {
            preview_has_summary_content(summary.as_deref())
                || crate::knowledge_store::default_summary_enabled_for_type(doc_type)
        }),
        command_enabled: patch.command_enabled.unwrap_or(false),
        read_only: patch.read_only.unwrap_or(false),
        ai_maintained: patch
            .ai_maintained
            .unwrap_or_else(|| crate::knowledge_store::default_ai_maintained_for_type(doc_type)),
        storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
        inherit_ai_config: patch.inherit_ai_config.unwrap_or(true),
        ai_config_source: Default::default(),
        explicit_maintenance_rules: patch.explicit_maintenance_rules.unwrap_or_else(|| {
            crate::knowledge_store::default_explicit_maintenance_rules_for_type(doc_type)
        }),
        external_source: patch.external_source.take().unwrap_or(None),
        skill_enabled: patch.skill_enabled,
        skill_surface: patch.skill_surface,
        command_trigger: patch.command_trigger.take().unwrap_or(None),
        argument_hint: patch.argument_hint.take().unwrap_or(None),
        summary,
        body,
        maintenance_rules,
        created_at: 0,
        updated_at: 0,
    };
    document = crate::knowledge_store::prepare_document_preview(document)?;

    Ok(KnowledgeToolConfirmPreview {
        operation: KnowledgeToolConfirmOperation::Create,
        target_kind: crate::knowledge_store::KnowledgeTargetKind::Document,
        doc_type,
        path: AgentInstance::prefix_knowledge_tool_path(doc_type, &normalized_path),
        new_path: None,
        directory_path,
        directory_mode,
        document_before_text: None,
        document_after_text: Some(crate::knowledge_store::render_document_preview(&document)?),
        structure_before_paths: Vec::new(),
        structure_after_paths: Vec::new(),
    })
}

fn build_knowledge_document_edit_preview(
    working_dir: &str,
    parsed: &AgentKnowledgeEditArgs,
) -> Result<KnowledgeToolConfirmPreview, String> {
    let (doc_type, normalized_path) = resolve_knowledge_document_target(&parsed.path)?;
    let parent_path = parent_knowledge_path(&normalized_path);
    let (directory_path, directory_mode) =
        resolve_existing_directory_mode(working_dir, doc_type, parent_path.as_deref())?;

    let current =
        crate::knowledge_store::load_document_by_path(working_dir, doc_type, &normalized_path)?;
    let before_text = crate::knowledge_store::render_document_preview(&current)?;

    let mut next = current;
    if let Some(summary) = parsed.document.summary.clone() {
        next.summary = summary;
    }
    if let Some(body) = parsed.document.body.clone() {
        next.body = body;
    }
    if let Some(maintenance_rules) = parsed.document.maintenance_rules.clone() {
        next.maintenance_rules = maintenance_rules;
    }
    next = crate::knowledge_store::prepare_document_preview(next)?;

    Ok(KnowledgeToolConfirmPreview {
        operation: KnowledgeToolConfirmOperation::Edit,
        target_kind: crate::knowledge_store::KnowledgeTargetKind::Document,
        doc_type,
        path: AgentInstance::prefix_knowledge_tool_path(doc_type, &normalized_path),
        new_path: None,
        directory_path,
        directory_mode,
        document_before_text: Some(before_text),
        document_after_text: Some(crate::knowledge_store::render_document_preview(&next)?),
        structure_before_paths: Vec::new(),
        structure_after_paths: Vec::new(),
    })
}

fn build_knowledge_move_preview(
    working_dir: &str,
    request: &crate::knowledge_store::KnowledgeMoveRequest,
) -> Result<KnowledgeToolConfirmPreview, String> {
    match request.kind {
        crate::knowledge_store::KnowledgeTargetKind::Document => {
            let (doc_type, source_path) = resolve_knowledge_document_target(&request.path)?;
            let (target_type, target_path) = resolve_knowledge_document_target(&request.new_path)?;
            if target_type != doc_type {
                return Err(
                    "knowledge move target path type prefix does not match the source type."
                        .to_string(),
                );
            }
            let target_parent = parent_knowledge_path(&target_path);
            let (directory_path, directory_mode) =
                resolve_child_directory_mode(working_dir, doc_type, target_parent.as_deref())?;

            Ok(KnowledgeToolConfirmPreview {
                operation: KnowledgeToolConfirmOperation::Move,
                target_kind: crate::knowledge_store::KnowledgeTargetKind::Document,
                doc_type,
                path: AgentInstance::prefix_knowledge_tool_path(doc_type, &source_path),
                new_path: Some(AgentInstance::prefix_knowledge_tool_path(
                    doc_type,
                    &target_path,
                )),
                directory_path,
                directory_mode,
                document_before_text: None,
                document_after_text: None,
                structure_before_paths: vec![AgentInstance::prefix_knowledge_tool_path(
                    doc_type,
                    &source_path,
                )],
                structure_after_paths: vec![AgentInstance::prefix_knowledge_tool_path(
                    doc_type,
                    &target_path,
                )],
            })
        }
        crate::knowledge_store::KnowledgeTargetKind::Directory => {
            let (doc_type, source_path) = resolve_knowledge_directory_target(&request.path)?;
            let (target_type, target_path) = resolve_knowledge_directory_target(&request.new_path)?;
            if target_type != doc_type {
                return Err(
                    "knowledge move target path type prefix does not match the source type."
                        .to_string(),
                );
            }
            let target_parent = parent_knowledge_path(&target_path);
            let (directory_path, directory_mode) =
                resolve_child_directory_mode(working_dir, doc_type, target_parent.as_deref())?;

            let before_paths =
                collect_directory_structure_paths(working_dir, doc_type, &source_path)?;
            let source_prefixed = AgentInstance::prefix_knowledge_tool_path(doc_type, &source_path);
            let target_prefixed = AgentInstance::prefix_knowledge_tool_path(doc_type, &target_path);
            let after_paths =
                relocate_structure_paths(&before_paths, &source_prefixed, &target_prefixed);

            Ok(KnowledgeToolConfirmPreview {
                operation: KnowledgeToolConfirmOperation::Move,
                target_kind: crate::knowledge_store::KnowledgeTargetKind::Directory,
                doc_type,
                path: source_prefixed,
                new_path: Some(target_prefixed),
                directory_path,
                directory_mode,
                document_before_text: None,
                document_after_text: None,
                structure_before_paths: before_paths,
                structure_after_paths: after_paths,
            })
        }
    }
}

fn build_knowledge_delete_preview(
    working_dir: &str,
    request: &crate::knowledge_store::KnowledgeDeleteRequest,
) -> Result<KnowledgeToolConfirmPreview, String> {
    match request.kind {
        crate::knowledge_store::KnowledgeTargetKind::Document => {
            let (doc_type, normalized_path) = resolve_knowledge_document_target(&request.path)?;
            let parent_path = parent_knowledge_path(&normalized_path);
            let (directory_path, directory_mode) =
                resolve_existing_directory_mode(working_dir, doc_type, parent_path.as_deref())?;
            let document = crate::knowledge_store::load_document_by_path(
                working_dir,
                doc_type,
                &normalized_path,
            )?;

            Ok(KnowledgeToolConfirmPreview {
                operation: KnowledgeToolConfirmOperation::Delete,
                target_kind: crate::knowledge_store::KnowledgeTargetKind::Document,
                doc_type,
                path: AgentInstance::prefix_knowledge_tool_path(doc_type, &normalized_path),
                new_path: None,
                directory_path,
                directory_mode,
                document_before_text: Some(crate::knowledge_store::render_document_preview(
                    &document,
                )?),
                document_after_text: None,
                structure_before_paths: vec![AgentInstance::prefix_knowledge_tool_path(
                    doc_type,
                    &normalized_path,
                )],
                structure_after_paths: Vec::new(),
            })
        }
        crate::knowledge_store::KnowledgeTargetKind::Directory => {
            let (doc_type, normalized_path) = resolve_knowledge_directory_target(&request.path)?;
            let (directory_path, directory_mode) =
                resolve_existing_directory_mode(working_dir, doc_type, Some(&normalized_path))?;
            let before_paths =
                collect_directory_structure_paths(working_dir, doc_type, &normalized_path)?;

            Ok(KnowledgeToolConfirmPreview {
                operation: KnowledgeToolConfirmOperation::Delete,
                target_kind: crate::knowledge_store::KnowledgeTargetKind::Directory,
                doc_type,
                path: AgentInstance::prefix_knowledge_tool_path(doc_type, &normalized_path),
                new_path: None,
                directory_path,
                directory_mode,
                document_before_text: None,
                document_after_text: None,
                structure_before_paths: before_paths,
                structure_after_paths: Vec::new(),
            })
        }
    }
}

fn assess_knowledge_tool_confirmation(
    working_dir: &str,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<Option<KnowledgeToolConfirmAssessment>, String> {
    let preview = match tool_name {
        "knowledge_create" => {
            let parsed = serde_json::from_value::<AgentKnowledgeCreateArgs>(args.clone())
                .map_err(|error| format!("Error parsing knowledge_create arguments: {}", error))?;
            Some(match parsed.kind {
                crate::knowledge_store::KnowledgeTargetKind::Document => {
                    build_knowledge_document_create_preview(working_dir, &parsed)?
                }
                crate::knowledge_store::KnowledgeTargetKind::Directory => {
                    let (doc_type, normalized_path) =
                        resolve_knowledge_directory_target(&parsed.path)?;
                    let parent_path = parent_knowledge_path(&normalized_path);
                    let (directory_path, directory_mode) = resolve_child_directory_mode(
                        working_dir,
                        doc_type,
                        parent_path.as_deref(),
                    )?;
                    KnowledgeToolConfirmPreview {
                        operation: KnowledgeToolConfirmOperation::Create,
                        target_kind: crate::knowledge_store::KnowledgeTargetKind::Directory,
                        doc_type,
                        path: AgentInstance::prefix_knowledge_tool_path(doc_type, &normalized_path),
                        new_path: None,
                        directory_path,
                        directory_mode,
                        document_before_text: None,
                        document_after_text: None,
                        structure_before_paths: Vec::new(),
                        structure_after_paths: vec![AgentInstance::prefix_knowledge_tool_path(
                            doc_type,
                            &normalized_path,
                        )],
                    }
                }
            })
        }
        "knowledge_edit" => {
            let parsed = serde_json::from_value::<AgentKnowledgeEditArgs>(args.clone())
                .map_err(|error| format!("Error parsing knowledge_edit arguments: {}", error))?;
            Some(build_knowledge_document_edit_preview(working_dir, &parsed)?)
        }
        "knowledge_move" => {
            let parsed = serde_json::from_value::<crate::knowledge_store::KnowledgeMoveRequest>(
                args.clone(),
            )
            .map_err(|error| format!("Error parsing knowledge_move arguments: {}", error))?;
            Some(build_knowledge_move_preview(working_dir, &parsed)?)
        }
        "knowledge_delete" => {
            let parsed = serde_json::from_value::<crate::knowledge_store::KnowledgeDeleteRequest>(
                args.clone(),
            )
            .map_err(|error| format!("Error parsing knowledge_delete arguments: {}", error))?;
            Some(build_knowledge_delete_preview(working_dir, &parsed)?)
        }
        _ => None,
    };

    Ok(preview.map(|preview| KnowledgeToolConfirmAssessment {
        governance_requires_confirm: preview.directory_mode
            == KnowledgeToolConfirmDirectoryMode::Approval,
        preview,
    }))
}

#[derive(Debug, Clone)]
struct PromptTreeFile {
    name: String,
    desc: String,
}

#[derive(Debug, Default, Clone)]
struct PromptTreeNode {
    desc: Option<String>,
    dirs: BTreeMap<String, PromptTreeNode>,
    notes: Vec<String>,
    files: Vec<PromptTreeFile>,
}

fn clip_single_line(value: &str, max_chars: usize) -> String {
    let merged = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = merged.chars();
    let clipped: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", clipped.trim_end())
    } else {
        clipped
    }
}

fn pluralize_files(count: usize) -> &'static str {
    if count == 1 {
        "file"
    } else {
        "files"
    }
}

fn prompt_flattened_skill_file_name(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if let Some(prefix) = normalized.strip_suffix("/SKILL.md") {
        let slug = prefix.rsplit('/').next().unwrap_or(prefix);
        format!("{}.md", slug)
    } else {
        normalized
            .rsplit('/')
            .next()
            .unwrap_or(&normalized)
            .to_string()
    }
}

fn prompt_file_name(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .to_string()
}

fn prompt_file_desc(item: &crate::knowledge_store::KnowledgeListItem) -> String {
    let title = item.title.trim();
    if title.is_empty() {
        prompt_file_name(&item.path)
    } else {
        clip_single_line(title, 80)
    }
}

fn prompt_directory_desc(
    record: &crate::knowledge_store::KnowledgeDirectoryConfigRecord,
) -> Option<String> {
    let show_desc = matches!(
        record.config.inject_mode,
        crate::knowledge_store::KnowledgeInjectMode::Excerpt
            | crate::knowledge_store::KnowledgeInjectMode::Full
    ) || matches!(
        (record.doc_type, record.path.as_str()),
        (
            crate::knowledge_store::KnowledgeType::Memory,
            "unity-project-understanding"
        )
    );

    if !show_desc {
        return None;
    }

    let summary = record.config.summary.trim();
    let maintenance_rules = record.config.maintenance_rules.trim();
    match (summary.is_empty(), maintenance_rules.is_empty()) {
        (false, false) => Some(format!(
            "{} | {}",
            clip_single_line(summary, 120),
            clip_single_line(maintenance_rules, 120)
        )),
        (false, true) => Some(clip_single_line(summary, 120)),
        (true, false) => Some(clip_single_line(maintenance_rules, 120)),
        (true, true) => None,
    }
}

fn insert_prompt_tree_file(node: &mut PromptTreeNode, parts: &[String], file: PromptTreeFile) {
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        node.files.push(file);
        return;
    }
    let child = node.dirs.entry(parts[0].clone()).or_default();
    insert_prompt_tree_file(child, &parts[1..], file);
}

fn insert_prompt_tree_directory(node: &mut PromptTreeNode, parts: &[String], desc: Option<&str>) {
    if parts.is_empty() {
        return;
    }
    let child = node.dirs.entry(parts[0].clone()).or_default();
    if parts.len() == 1 {
        child.desc = desc.map(str::to_string);
        return;
    }
    insert_prompt_tree_directory(child, &parts[1..], desc);
}

fn insert_prompt_tree_note(node: &mut PromptTreeNode, parts: &[String], note: &str) {
    if parts.is_empty() {
        return;
    }
    let child = node.dirs.entry(parts[0].clone()).or_default();
    if parts.len() == 1 {
        child.notes.push(note.to_string());
        return;
    }
    insert_prompt_tree_note(child, &parts[1..], note);
}

fn sort_prompt_tree(node: &mut PromptTreeNode) {
    node.notes.sort();
    node.files
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.desc.cmp(&b.desc)));
    for child in node.dirs.values_mut() {
        sort_prompt_tree(child);
    }
}

fn build_prompt_tree(
    items: &[crate::knowledge_store::KnowledgeListItem],
    directories: &[crate::knowledge_store::KnowledgeDirectoryConfigRecord],
    flatten_skill: bool,
) -> PromptTreeNode {
    let mut root = PromptTreeNode::default();
    if !flatten_skill {
        for directory in directories {
            let parts: Vec<String> = directory
                .path
                .split('/')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_string())
                .collect();
            let desc = prompt_directory_desc(directory);
            insert_prompt_tree_directory(&mut root, &parts, desc.as_deref());
        }
    }
    for item in items {
        let file_name =
            if flatten_skill && item.doc_type == crate::knowledge_store::KnowledgeType::Skill {
                prompt_flattened_skill_file_name(&item.path)
            } else {
                prompt_file_name(&item.path)
            };
        let file = PromptTreeFile {
            name: file_name,
            desc: prompt_file_desc(item),
        };
        let parts: Vec<String> = if flatten_skill {
            vec![file.name.clone()]
        } else {
            item.path.split('/').map(|part| part.to_string()).collect()
        };
        insert_prompt_tree_file(&mut root, &parts, file);
    }
    sort_prompt_tree(&mut root);
    root
}

fn render_tree_lines(
    node: &PromptTreeNode,
    show_files: bool,
    max_visible_files: usize,
) -> Vec<String> {
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();

    for (dir_name, child) in &node.dirs {
        let label = if let Some(desc) = child.desc.as_deref() {
            format!("{}/ :: {}", dir_name, desc)
        } else {
            format!("{}/", dir_name)
        };
        entries.push((
            label,
            render_tree_lines(child, show_files, max_visible_files),
        ));
    }

    for note in &node.notes {
        entries.push((note.clone(), Vec::new()));
    }

    if show_files {
        for file in node.files.iter().take(max_visible_files) {
            entries.push((format!("{} :: {}", file.name, file.desc), Vec::new()));
        }
        let hidden = node.files.len().saturating_sub(max_visible_files);
        if hidden > 0 {
            entries.push((
                format!("<{} {} hidden>", hidden, pluralize_files(hidden)),
                Vec::new(),
            ));
        }
    } else if !node.files.is_empty() {
        entries.push((
            format!(
                "<{} {} hidden>",
                node.files.len(),
                pluralize_files(node.files.len())
            ),
            Vec::new(),
        ));
    }

    if entries.is_empty() {
        entries.push(("<empty>".to_string(), Vec::new()));
    }

    let mut lines = Vec::new();
    for (index, (label, nested)) in entries.iter().enumerate() {
        let is_last = index + 1 == entries.len();
        let branch = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last { "   " } else { "│  " };
        lines.push(format!("{}{}", branch, label));
        for line in nested {
            lines.push(format!("{}{}", child_prefix, line));
        }
    }
    lines
}

fn build_structure_section(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    access_mode: KnowledgeAccessMode,
) -> Result<String, String> {
    crate::knowledge_store::ensure_memory_builtin_documents(working_dir)?;

    let excluded_reference_prefixes = if crate::unity_docs::has_managed_store(working_dir) {
        vec![(
            crate::knowledge_store::KnowledgeType::Reference,
            crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR.to_string(),
        )]
    } else {
        Vec::new()
    };

    let design_items = crate::knowledge_store::list_documents_with_app_root(
        working_dir,
        app_knowledge_dir,
        Some(crate::knowledge_store::KnowledgeType::Design),
        None,
    )?;
    let reference_items = crate::knowledge_store::list_documents_with_app_root_excluding_prefixes(
        working_dir,
        app_knowledge_dir,
        Some(crate::knowledge_store::KnowledgeType::Reference),
        None,
        &excluded_reference_prefixes,
    )?;
    let mut skill_items = crate::knowledge_store::list_documents_with_app_root(
        working_dir,
        app_knowledge_dir,
        Some(crate::knowledge_store::KnowledgeType::Skill),
        None,
    )?;
    skill_items.extend(crate::commands::list_skill_package_knowledge_items_sync(
        working_dir,
        None,
    ));
    let memory_items = crate::knowledge_store::list_documents_with_app_root(
        working_dir,
        app_knowledge_dir,
        Some(crate::knowledge_store::KnowledgeType::Memory),
        None,
    )?;

    let design_directories = crate::knowledge_store::list_directory_configs_with_app_root(
        working_dir,
        app_knowledge_dir,
        crate::knowledge_store::KnowledgeType::Design,
    )?;
    let reference_directory_exclusions = excluded_reference_prefixes
        .iter()
        .filter(|(doc_type, _)| *doc_type == crate::knowledge_store::KnowledgeType::Reference)
        .map(|(_, prefix)| prefix.clone())
        .collect::<Vec<_>>();
    let reference_directories =
        crate::knowledge_store::list_directory_configs_with_app_root_excluding_prefixes(
            working_dir,
            app_knowledge_dir,
            crate::knowledge_store::KnowledgeType::Reference,
            &reference_directory_exclusions,
        )?;
    let memory_directories = crate::knowledge_store::list_directory_configs_with_app_root(
        working_dir,
        app_knowledge_dir,
        crate::knowledge_store::KnowledgeType::Memory,
    )?;
    let empty_skill_directories: &[crate::knowledge_store::KnowledgeDirectoryConfigRecord] = &[];

    let design_tree = build_prompt_tree(&design_items, &design_directories, false);
    let mut reference_tree = build_prompt_tree(&reference_items, &reference_directories, false);
    if crate::unity_docs::has_managed_store(working_dir) {
        insert_prompt_tree_directory(
            &mut reference_tree,
            &[crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR.to_string()],
            Some(
                "Unity official reference library. Keep the always-on prompt compact here and use `knowledge_query` or concrete `reference/unity-official-docs/...` paths when needed.",
            ),
        );
        let unity_note = crate::unity_docs::managed_document_count_hint(working_dir)?
            .map(|count| format!("<{} {} managed externally>", count, pluralize_files(count)))
            .unwrap_or_else(|| "<managed externally>".to_string());
        insert_prompt_tree_note(
            &mut reference_tree,
            &[crate::unity_docs::UNITY_REFERENCE_MANAGED_DIR.to_string()],
            &unity_note,
        );
    }
    let skill_tree = build_prompt_tree(&skill_items, empty_skill_directories, false);
    let memory_tree = build_prompt_tree(&memory_items, &memory_directories, false);

    let top_entries = if access_mode == KnowledgeAccessMode::ReadOnly {
        vec![
            (
                "design/ :: Project design direction discussed with the user, including game design and technical architecture".to_string(),
                render_tree_lines(&design_tree, true, 2),
            ),
            (
                "reference/ :: External material".to_string(),
                render_tree_lines(&reference_tree, false, 0),
            ),
            (
                "skill/ :: Standard workflows for getting work done".to_string(),
                render_tree_lines(&skill_tree, true, 3),
            ),
            (
                "memory/ :: Project memory and long-term working context".to_string(),
                render_tree_lines(&memory_tree, true, 3),
            ),
        ]
    } else {
        vec![
            (
                "design/ :: Project design direction discussed with the user, including game design and technical architecture | Update only when the user introduces design direction. The user reviews the update".to_string(),
                render_tree_lines(&design_tree, true, 2),
            ),
            (
                "reference/ :: External material | Read-only".to_string(),
                render_tree_lines(&reference_tree, false, 0),
            ),
            (
                "skill/ :: Standard workflows for getting work done. Update a skill when technical changes affect its flow. Suggest a new skill when a task looks reusable".to_string(),
                render_tree_lines(&skill_tree, true, 3),
            ),
            (
                "memory/ :: All of your memory | Very important. Update and maintain it frequently".to_string(),
                render_tree_lines(&memory_tree, true, 3),
            ),
        ]
    };

    let mut lines = vec![
        "### Structure".to_string(),
        String::new(),
        "```tree".to_string(),
        "knowledge/".to_string(),
    ];
    let mut rendered = Vec::new();
    for (index, (label, nested)) in top_entries.iter().enumerate() {
        let is_last = index + 1 == top_entries.len();
        let branch = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last { "   " } else { "│  " };
        rendered.push(format!("{}{}", branch, label));
        for line in nested {
            rendered.push(format!("{}{}", child_prefix, line));
        }
    }
    lines.extend(rendered);
    lines.push("```".to_string());
    Ok(lines.join("\n"))
}

fn build_search_section() -> String {
    [
        "### Search",
        "1. Start with `knowledge_query` and split exact terms into `lexicalQuery` and intent-style retrieval into `semanticQuery` when useful.",
        "2. Use `knowledge_read` when you know the target document path or need a specific document. For Skill packages, `skill/<package-id>` reads the package root `SKILL.md`.",
        "3. Use `knowledge_list` to browse entries under a type-prefixed directory path prefix such as `design/` or `skill/unity/`.",
    ]
    .join("\n")
}

fn build_maintenance_section(access_mode: KnowledgeAccessMode) -> String {
    if access_mode == KnowledgeAccessMode::ReadOnly {
        return [
            "### Access",
            "- Knowledge is read-only for this request.",
            "- Use knowledge search and read tools for context when useful.",
        ]
        .join("\n");
    }

    [
        "### Maintenance",
        "- When the user gives you new project information, or your changes affect the correctness of knowledge documents, keep the knowledge base current and structurally sound, and report your update to the user.",
        "- For Memory, think of it as all of yourself. Read and write it actively so future work goes more smoothly.",
        "- Respect existing maintenance rules on any document or folder you maintain.",
    ]
    .join("\n")
}

fn build_tools_section(access_mode: KnowledgeAccessMode) -> String {
    let mut lines = vec![
        "### Tools",
        "- `knowledge_query`: Search knowledge with separate `lexicalQuery` and `semanticQuery` inputs, plus an optional type-prefixed `pathPrefix`.",
        "- `knowledge_read`: Read a specific document by type-prefixed path. For Skill packages, `skill/<package-id>` reads the package root `SKILL.md` and `skill/<package-id>/docs/file.md` reads package child docs.",
        "- `knowledge_list`: Browse entries under a type-prefixed directory path prefix.",
    ];
    if access_mode == KnowledgeAccessMode::Full {
        lines.extend([
            "- `knowledge_edit`: Update an existing document's content sections by type-prefixed `.md` path.",
            "- `knowledge_create`: Create a new non-Skill document or folder at a type-prefixed path. Document paths end with `.md`; directory paths do not. Document creation only accepts initial content.",
            "- `knowledge_move`: Move a document or folder using type-prefixed paths. Document paths end with `.md`; directory paths do not.",
            "- `knowledge_delete`: Delete an obsolete document or folder by type-prefixed path. Document paths end with `.md`; directory paths do not.",
            "- `skill_create`: Create a Skill as `kind: \"md\"` with metadata or `kind: \"package\"` with package metadata. Use `kind: \"package\"` for Skills that require CLI tools, binaries, scripts, Unity C# files, bundled docs, or any package-local dependency environment.",
            "- `skill_reload`: Load or reload a Skill manifest after edits and return validation errors.",
            "- `skill_list`: List discoverable project Skills, built-in app Skills, and app Skill packages.",
        ]);
    }
    lines.join("\n")
}

fn build_l2_memory_section(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Result<String, String> {
    crate::knowledge_store::ensure_memory_builtin_documents(working_dir)?;
    let items = crate::knowledge_store::list_documents_with_app_root(
        working_dir,
        app_knowledge_dir,
        Some(crate::knowledge_store::KnowledgeType::Memory),
        None,
    )?;

    let mut blocks = Vec::new();
    for item in items {
        if item.inject_mode != crate::knowledge_store::KnowledgeInjectMode::Full {
            continue;
        }

        let doc = crate::knowledge_store::read_document_with_app_root(
            working_dir,
            app_knowledge_dir,
            crate::knowledge_store::KnowledgeType::Memory,
            &item.path,
            "full",
        )?
        .document;

        let rules = doc
            .maintenance_rules
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("<empty>");
        let body = if doc.body.trim().is_empty() {
            "<empty>".to_string()
        } else {
            remap_document_body_headings(&doc.body, 4)
        };

        blocks.push(
            [
                format!("#### memory/{}", doc.path),
                String::new(),
                "Rules:".to_string(),
                rules.to_string(),
                String::new(),
                "Body:".to_string(),
                body,
            ]
            .join("\n"),
        );
    }

    if blocks.is_empty() {
        return Ok(String::new());
    }

    Ok(format!(
        "### L2 Memory\nThese memory documents stay in the always-on knowledge context as full injections.\n\n{}",
        blocks.join("\n\n")
    ))
}

fn parse_markdown_heading_level(line: &str) -> Option<(usize, &str, &str)> {
    let trimmed_start = line.trim_start_matches(' ');
    let indent_len = line.len().saturating_sub(trimmed_start.len());
    if indent_len > 3 {
        return None;
    }

    let level = trimmed_start
        .bytes()
        .take_while(|byte| *byte == b'#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }

    let remainder = &trimmed_start[level..];
    if !remainder.starts_with(' ') && !remainder.starts_with('\t') {
        return None;
    }

    Some((level, &line[..indent_len], remainder.trim_start()))
}

fn remap_document_body_headings(body: &str, target_min_level: usize) -> String {
    let normalized = body.replace("\r\n", "\n");
    let mut in_fence: Option<char> = None;
    let mut min_level: Option<usize> = None;

    for line in normalized.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = trimmed.chars().next().filter(|ch| {
            (*ch == '`' || *ch == '~') && trimmed.chars().take_while(|c| *c == *ch).count() >= 3
        }) {
            if in_fence == Some(marker) {
                in_fence = None;
            } else if in_fence.is_none() {
                in_fence = Some(marker);
            }
            continue;
        }

        if in_fence.is_some() {
            continue;
        }

        if let Some((level, _, _)) = parse_markdown_heading_level(line) {
            min_level = Some(min_level.map_or(level, |current| current.min(level)));
        }
    }

    let Some(min_level) = min_level else {
        return normalized.trim().to_string();
    };
    let shift = target_min_level as isize - min_level as isize;

    let mut remapped = Vec::new();
    in_fence = None;
    for line in normalized.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = trimmed.chars().next().filter(|ch| {
            (*ch == '`' || *ch == '~') && trimmed.chars().take_while(|c| *c == *ch).count() >= 3
        }) {
            if in_fence == Some(marker) {
                in_fence = None;
            } else if in_fence.is_none() {
                in_fence = Some(marker);
            }
            remapped.push(line.to_string());
            continue;
        }

        if in_fence.is_none() {
            if let Some((level, indent, text)) = parse_markdown_heading_level(line) {
                let new_level = (level as isize + shift).clamp(1, 6) as usize;
                remapped.push(format!("{}{} {}", indent, "#".repeat(new_level), text));
                continue;
            }
        }

        remapped.push(line.to_string());
    }

    remapped.join("\n").trim().to_string()
}

fn humanize_document_title(value: &str) -> String {
    let words = value
        .split(|ch: char| ch == '-' || ch == '_' || ch.is_whitespace())
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let rest = chars.collect::<String>();
            if segment.is_ascii() {
                format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    rest.to_ascii_lowercase()
                )
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>();
    if words.is_empty() {
        value.trim().to_string()
    } else {
        words.join(" ")
    }
}

fn l3_rule_display_title(doc: &crate::knowledge_store::KnowledgeDocument) -> String {
    match (doc.doc_type, doc.path.trim()) {
        (crate::knowledge_store::KnowledgeType::Memory, "project-mistake-note.md")
        | (crate::knowledge_store::KnowledgeType::Memory, "project_mistake_note.md") => {
            "Mistake Notebook".to_string()
        }
        (crate::knowledge_store::KnowledgeType::Memory, "user-preference.md")
        | (crate::knowledge_store::KnowledgeType::Memory, "user_preference.md") => {
            "User Preferences".to_string()
        }
        _ => {
            let title = doc.title.trim();
            if title.is_empty() {
                return humanize_document_title(&doc.path);
            }
            let default_title = crate::knowledge_store::default_document_title_from_path(&doc.path)
                .unwrap_or_else(|_| title.to_string());
            if title == default_title {
                humanize_document_title(title)
            } else {
                title.to_string()
            }
        }
    }
}

fn format_l3_rule_heading(doc: &crate::knowledge_store::KnowledgeDocument) -> String {
    let path = format!("{}/{}", doc.doc_type, doc.path.trim());
    let title = l3_rule_display_title(doc);
    if title.is_empty() {
        path
    } else {
        format!("{} ({})", title, path)
    }
}

struct L3RuleEntry {
    doc_type: crate::knowledge_store::KnowledgeType,
    path: String,
    title: String,
    content: String,
}

fn build_l3_rule_entries(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Result<Vec<L3RuleEntry>, String> {
    crate::knowledge_store::ensure_memory_builtin_documents(working_dir)?;
    let mut entries = Vec::new();

    for doc_type in crate::knowledge_store::KnowledgeType::all() {
        let items = crate::knowledge_store::list_documents_with_app_root(
            working_dir,
            app_knowledge_dir,
            Some(doc_type),
            None,
        )?;

        for item in items {
            if item.inject_mode != crate::knowledge_store::KnowledgeInjectMode::Rule {
                continue;
            }
            let doc = crate::knowledge_store::read_document_with_app_root(
                working_dir,
                app_knowledge_dir,
                doc_type,
                &item.path,
                "full",
            )?
            .document;

            let title = format_l3_rule_heading(&doc);
            let rules = doc
                .maintenance_rules
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("<empty>");
            let body = if doc.body.trim().is_empty() {
                "<empty>".to_string()
            } else {
                remap_document_body_headings(&doc.body, 4)
            };
            let content = [
                format!("### {}", title),
                String::new(),
                "Maintenance Rules:".to_string(),
                rules.to_string(),
                String::new(),
                "Full Document:".to_string(),
                body,
            ]
            .join("\n");

            entries.push(L3RuleEntry {
                doc_type,
                path: doc.path.clone(),
                title,
                content,
            });
        }
    }

    Ok(entries)
}

fn build_l3_rule_section(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Result<String, String> {
    let entries = build_l3_rule_entries(working_dir, app_knowledge_dir)?;
    if entries.is_empty() {
        return Ok(String::new());
    }

    Ok(format!(
        "## L3 Rules\nThese rule-injected documents are active session rules. Treat both maintenance rules and full document content below as always-on instructions.\n\n{}",
        entries
            .into_iter()
            .map(|entry| entry.content)
            .collect::<Vec<_>>()
            .join("\n\n")
    ))
}

fn extract_api_tool_name_and_description(value: &serde_json::Value) -> Option<(String, String)> {
    let function = value.get("function").unwrap_or(value);
    let name = function
        .get("name")
        .and_then(|field| field.as_str())
        .or_else(|| value.get("name").and_then(|field| field.as_str()))?
        .trim();
    if name.is_empty() {
        return None;
    }

    let description = function
        .get("description")
        .and_then(|field| field.as_str())
        .or_else(|| value.get("description").and_then(|field| field.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();

    Some((name.to_string(), description))
}

fn env_block_position(env_template: &str, tags: &[&str]) -> usize {
    tags.iter()
        .filter_map(|tag| env_template.find(tag))
        .min()
        .unwrap_or(usize::MAX)
}

fn injected_item_prompt_sort_key(env_template: &str, item_id: &str) -> (u8, usize) {
    match item_id {
        id if id.starts_with("knowledge_rule::") => (1, usize::MAX),
        "knowledge_context" => (2, env_block_position(env_template, &["{{#knowledge}}"])),
        "lazy_tool_names" => (3, usize::MAX),
        _ => (4, usize::MAX),
    }
}

struct SubagentTaskResult {
    output: String,
    tool_calls: Vec<ToolCallInfo>,
    is_error: bool,
}

struct SystemPromptParts {
    base_prompt: String,
    rules_prompt: String,
    knowledge_prompt: String,
    env_prompt: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSystemPromptStats {
    pub base_chars: usize,
    pub env_chars: usize,
    pub rules_chars: usize,
    pub knowledge_chars: usize,
    pub total_chars: usize,
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
impl AgentInstance {
    fn is_dev_runtime_knowledge_rule(file_name: &str) -> bool {
        matches!(file_name, "知识库使用.md" | "知识维护.md")
    }

    fn has_selected_working_dir_value(working_dir: &str) -> bool {
        !working_dir.trim().is_empty()
    }

    fn has_selected_working_dir(&self) -> bool {
        Self::has_selected_working_dir_value(&self.working_dir)
    }

    fn display_working_dir_value(working_dir: &str) -> String {
        let trimmed = working_dir.trim();
        if trimmed.is_empty() {
            "(not selected)".to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn resolve_path_against_working_dir(&self, raw_path: &str) -> Option<String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return None;
        }

        let path = std::path::Path::new(trimmed);
        if path.is_absolute() {
            return Some(trimmed.to_string());
        }

        if !self.has_selected_working_dir() {
            return None;
        }
        if !self.knowledge_access_mode.allows_context() {
            return None;
        }

        Some(
            std::path::Path::new(&self.working_dir)
                .join(path)
                .display()
                .to_string(),
        )
    }

    fn normalize_path_lexically(path: &std::path::Path) -> std::path::PathBuf {
        use std::path::Component;

        let mut normalized = std::path::PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                Component::RootDir => normalized.push(component.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => {
                    let can_pop = normalized
                        .components()
                        .next_back()
                        .map(|last| matches!(last, Component::Normal(_)))
                        .unwrap_or(false);
                    if can_pop {
                        normalized.pop();
                    }
                }
                Component::Normal(part) => normalized.push(part),
            }
        }
        normalized
    }

    fn has_drive_prefix_without_root(path: &std::path::Path) -> bool {
        use std::path::Component;

        matches!(path.components().next(), Some(Component::Prefix(_))) && !path.has_root()
    }

    fn resolve_workspace_scoped_path(
        working_dir: &str,
        raw_path: &str,
    ) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
        let canonical_root = dunce::canonicalize(working_dir)
            .map_err(|e| format!("the selected working directory is unavailable: {}", e))?;
        let requested = std::path::Path::new(raw_path.trim());

        if Self::has_drive_prefix_without_root(requested) {
            return Err(
                "drive-relative paths are not allowed; use a workspace-relative path or an absolute path inside the workspace"
                    .to_string(),
            );
        }

        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            canonical_root.join(requested)
        };
        let normalized = Self::normalize_path_lexically(&candidate);

        let mut anchor = normalized.clone();
        let mut suffix = Vec::new();
        while !anchor.exists() {
            let Some(name) = anchor.file_name() else {
                break;
            };
            suffix.push(name.to_os_string());
            let Some(parent) = anchor.parent() else {
                break;
            };
            anchor = parent.to_path_buf();
        }

        let mut resolved = if anchor.exists() {
            dunce::canonicalize(&anchor).unwrap_or(anchor)
        } else {
            normalized.clone()
        };
        for part in suffix.iter().rev() {
            resolved.push(part);
        }

        Ok((resolved, canonical_root))
    }

    fn validate_workspace_or_app_bound_path(
        working_dir: &str,
        tool_name: &str,
        raw_path: &str,
    ) -> Option<String> {
        let (resolved, canonical_root) =
            match Self::resolve_workspace_scoped_path(working_dir, raw_path) {
                Ok(value) => value,
                Err(error) => {
                    return Some(format!(
                        "Tool '{}' cannot access '{}': {}.",
                        tool_name, raw_path, error
                    ))
                }
            };

        if Self::path_is_within_root(&resolved, &canonical_root) {
            None
        } else {
            for root in crate::commands::app_skill_package_dirs() {
                let canonical_skill_root = dunce::canonicalize(&root).unwrap_or(root);
                if Self::path_is_within_root(&resolved, &canonical_skill_root) {
                    return None;
                }
            }
            if let Ok(root) = crate::commands::app_temp_dir() {
                let canonical_temp_root = dunce::canonicalize(&root).unwrap_or(root);
                if Self::path_is_within_root(&resolved, &canonical_temp_root) {
                    return None;
                }
            }
            Some(format!(
                "Tool '{}' cannot access '{}': direct filesystem tools may only operate within the selected working directory '{}', an app Skill package directory, or the app temp directory.",
                tool_name,
                raw_path,
                canonical_root.display()
            ))
        }
    }

    fn validate_tool_path_requirements(
        working_dir: &str,
        tool_name: &str,
        args: &serde_json::Value,
        enforce_file_workspace_boundary: bool,
    ) -> Option<String> {
        let has_working_dir = Self::has_selected_working_dir_value(working_dir);
        let arg_str = |key: &str| {
            args.get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };

        let require_non_empty = |key: &str, scope: &str| {
            if arg_str(key).is_none() {
                Some(format!(
                    "Tool '{}' requires a non-empty '{}' {}.",
                    tool_name, key, scope
                ))
            } else {
                None
            }
        };

        let require_absolute_without_workspace = |key: &str| {
            let value = arg_str(key)?;
            if std::path::Path::new(value).is_absolute() || has_working_dir {
                None
            } else {
                Some(format!(
                    "Tool '{}' requires an absolute '{}' when no working directory is selected.",
                    tool_name, key
                ))
            }
        };

        let require_workspace_bound = |key: &str| {
            if !enforce_file_workspace_boundary || !has_working_dir {
                return None;
            }
            let value = arg_str(key)?;
            Self::validate_workspace_or_app_bound_path(working_dir, tool_name, value)
        };

        match tool_name {
            "bash" => require_non_empty("workdir", "to be set explicitly")
                .or_else(|| require_absolute_without_workspace("workdir")),
            "grep" | "list" => require_non_empty("path", "to be set explicitly")
                .or_else(|| require_absolute_without_workspace("path"))
                .or_else(|| require_workspace_bound("path")),
            "read" | "write" | "edit" => require_non_empty("filePath", "to be set explicitly")
                .or_else(|| require_absolute_without_workspace("filePath"))
                .or_else(|| require_workspace_bound("filePath")),
            "unity_recompile" => require_non_empty("project_path", "to be set explicitly")
                .or_else(|| require_absolute_without_workspace("project_path")),
            "unity_execute"
            | "unity_run_states"
            | "unity_capture_viewport"
            | "unity_ref_search"
            | "unity_asset_search"
            | "unity_yaml_list"
            | "unity_yaml_search"
            | "unity_yaml_read"
            | "knowledge_list"
            | "knowledge_query"
            | "knowledge_read"
            | "knowledge_create"
            | "knowledge_delete"
            | "knowledge_move"
            | "knowledge_edit" => {
                if has_working_dir {
                    None
                } else {
                    Some(format!(
                        "Tool '{}' requires a selected working directory because it operates on workspace-scoped project data.",
                        tool_name
                    ))
                }
            }
            _ => None,
        }
    }

    async fn build_runtime_knowledge_block(
        &self,
        _include_index: bool,
        _include_memory: bool,
    ) -> Option<String> {
        if !self.has_selected_working_dir() {
            return None;
        }

        let started_at = Instant::now();
        eprintln!(
            "[Agent {}] knowledge context build start: session={} cwd={} include_index={} include_memory={}",
            self.id,
            self.session_id,
            self.working_dir,
            _include_index,
            _include_memory
        );
        let mut sections = Vec::new();

        let structure_started_at = Instant::now();
        if let Ok(structure) = build_structure_section(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            self.knowledge_access_mode,
        ) {
            sections.push(structure);
        }
        eprintln!(
            "[Agent {}] knowledge context structure ready: session={} elapsed_ms={} sections={}",
            self.id,
            self.session_id,
            structure_started_at.elapsed().as_millis(),
            sections.len()
        );

        sections.push(build_search_section());
        sections.push(build_maintenance_section(self.knowledge_access_mode));
        sections.push(build_tools_section(self.knowledge_access_mode));

        if _include_memory {
            if let Ok(memory_section) =
                build_l2_memory_section(&self.working_dir, self.app_knowledge_dir.as_ref().as_ref())
            {
                if !memory_section.trim().is_empty() {
                    sections.push(memory_section);
                }
            }
        }

        if sections.is_empty() {
            eprintln!(
                "[Agent {}] knowledge context build finished empty: session={} elapsed_ms={}",
                self.id,
                self.session_id,
                started_at.elapsed().as_millis()
            );
            None
        } else {
            let content = format!("## Knowledge\n\n{}", sections.join("\n\n"));
            eprintln!(
                "[Agent {}] knowledge context build finished: session={} elapsed_ms={} sections={} chars={}",
                self.id,
                self.session_id,
                started_at.elapsed().as_millis(),
                sections.len(),
                content.len()
            );
            Some(content)
        }
    }

    pub fn new(
        def: Arc<AgentDef>,
        session_id: &str,
        backend: LlmBackend,
        debug: bool,
        registry: Arc<AgentDefRegistry>,
        tool_registry: Arc<ToolRegistry>,
        working_dir: String,
        raw_store: RawContextStore,
        workspace_id: Option<String>,
        effective_model: String,
        effort: Option<String>,
        app_knowledge_dir: Arc<Option<std::path::PathBuf>>,
        app_agent_dir: Arc<Option<std::path::PathBuf>>,
        knowledge_access_mode: KnowledgeAccessMode,
        undo_manager: Option<Arc<crate::vcs::UndoManager>>,
        subagent_model_overrides: std::collections::HashMap<String, String>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        let effective_effort = effort.or_else(|| def.default_effort.clone());
        AgentInstance {
            id: uuid::Uuid::new_v4().to_string(),
            def,
            effective_model,
            session_id: session_id.to_string(),
            backend,
            debug,
            registry,
            tool_registry,
            working_dir,
            raw_store,
            workspace_id,
            parent_tool_call: None,
            effort: effective_effort,
            app_knowledge_dir,
            app_agent_dir,
            knowledge_access_mode,
            undo_manager,
            subagent_model_overrides,
            tool_runtime_state: Arc::new(ToolRuntimeState::default()),
            loaded_tool_names: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            partial_assistant: Arc::new(AssistantStreamState::default()),
            cancel_rx,
        }
    }

    pub fn partial_assistant_state(&self) -> Arc<AssistantStreamState> {
        self.partial_assistant.clone()
    }

    async fn build_tool_execution_context(
        &self,
        app_handle: &AppHandle,
        tool_name: &str,
    ) -> ToolExecutionContext {
        let unity_connected = if tool_name == "read" {
            Some(crate::unity_bridge::is_unity_connected(&self.working_dir).await)
        } else {
            None
        };

        ToolExecutionContext {
            app_handle: Some(app_handle.clone()),
            working_dir: if self.has_selected_working_dir() {
                Some(self.working_dir.clone())
            } else {
                None
            },
            unity_connected,
            runtime_state: Some(self.tool_runtime_state.clone()),
        }
    }

    async fn resolve_effective_tool_names(&self) -> Vec<String> {
        let mut tools: Vec<String> = self
            .def
            .tools
            .iter()
            .filter(|tool_name| match tool_name.as_str() {
                "knowledge_list" | "knowledge_query" | "knowledge_read" | "knowledge_create"
                | "knowledge_delete" | "knowledge_move" | "knowledge_edit" | "skill_create"
                | "skill_reload" | "skill_list" => {
                    self.has_selected_working_dir()
                        && self.knowledge_access_mode.allows_tool(tool_name.as_str())
                }
                _ => true,
            })
            .cloned()
            .collect();
        for tool_name in self.tool_registry.skill_tool_names() {
            push_unique_tool_name(&mut tools, &tool_name);
        }
        tools
    }

    fn supports_native_tool_lazy_loading(&self) -> bool {
        match &self.backend {
            LlmBackend::OpenAiCodex { .. } => true,
            LlmBackend::Custom {
                supports_tool_lazy_loading,
                ..
            } => *supports_tool_lazy_loading,
            _ => false,
        }
    }

    fn is_meta_tool(name: &str) -> bool {
        matches!(name, "tool_load" | "tool_call")
    }

    fn is_knowledge_tool_name(name: &str) -> bool {
        matches!(
            name,
            "knowledge_list"
                | "knowledge_query"
                | "knowledge_read"
                | "knowledge_create"
                | "knowledge_delete"
                | "knowledge_move"
                | "knowledge_edit"
                | "skill_create"
                | "skill_reload"
                | "skill_list"
        )
    }

    fn is_knowledge_mutation_tool_name(name: &str) -> bool {
        matches!(
            name,
            "knowledge_create"
                | "knowledge_delete"
                | "knowledge_move"
                | "knowledge_edit"
                | "skill_create"
        )
    }

    fn tool_direct_load_overrides(&self) -> HashMap<String, bool> {
        let config = crate::commands::merged_tool_load_config_for_agent(
            self.app_agent_dir.as_ref(),
            &self.working_dir,
            &self.def.id,
        );
        config
            .direct_load
            .into_iter()
            .filter_map(|(name, direct_load)| {
                self.tool_registry
                    .canonical_name(&name)
                    .map(|canonical| (canonical, direct_load))
            })
            .collect()
    }

    fn can_configure_direct_load_tool(&self, name: &str) -> bool {
        !Self::is_meta_tool(name)
            && self.tool_registry.is_built_in(name)
            && matches!(
                self.default_tool_load_mode(name),
                ToolLoadMode::Direct | ToolLoadMode::Lazy
            )
    }

    fn default_tool_load_mode(&self, name: &str) -> ToolLoadMode {
        if Self::is_meta_tool(name) {
            return ToolLoadMode::Direct;
        }
        self.tool_registry.default_load_mode(name)
    }

    fn configured_tool_load_mode(
        &self,
        name: &str,
        overrides: &HashMap<String, bool>,
    ) -> ToolLoadMode {
        let default_mode = self.default_tool_load_mode(name);
        if Self::is_meta_tool(name) || default_mode == ToolLoadMode::Skill {
            return default_mode;
        }
        if !self.tool_registry.is_built_in(name) {
            return default_mode;
        }
        match overrides.get(name).copied() {
            Some(true) => ToolLoadMode::Direct,
            Some(false) => ToolLoadMode::Lazy,
            None => default_mode,
        }
    }

    fn default_direct_load_for_tool(&self, name: &str) -> bool {
        self.default_tool_load_mode(name) == ToolLoadMode::Direct
    }

    fn should_direct_load_tool(&self, name: &str, overrides: &HashMap<String, bool>) -> bool {
        self.configured_tool_load_mode(name, overrides) == ToolLoadMode::Direct
    }

    async fn allowed_tool_set(&self) -> HashSet<String> {
        self.resolve_effective_tool_names()
            .await
            .into_iter()
            .filter_map(|name| self.tool_registry.canonical_name(&name))
            .collect()
    }

    async fn is_allowed_agent_tool(&self, tool_name: &str) -> bool {
        self.allowed_tool_set().await.contains(tool_name)
    }

    async fn seed_loaded_tools_from_history(
        &self,
        messages: &[crate::session::models::ChatMessage],
    ) {
        let allowed = self.allowed_tool_set().await;
        let mut loaded = HashSet::new();
        for message in messages {
            let Some(tool_calls) = message.tool_calls.as_ref() else {
                continue;
            };
            for tool_call in tool_calls {
                if tool_call.name != "tool_load" {
                    continue;
                }
                if tool_call
                    .outcome
                    .is_some_and(|outcome| outcome != crate::commands::ToolCallOutcome::Done)
                {
                    continue;
                }
                for name in requested_tool_load_names(&tool_call.arguments) {
                    let Some(canonical) = self.tool_registry.canonical_name(&name) else {
                        continue;
                    };
                    if allowed.contains(&canonical) {
                        loaded.insert(canonical);
                    }
                }
            }
        }
        let mut guard = self.loaded_tool_names.lock().await;
        *guard = loaded;
    }

    async fn build_request_tool_names(&self) -> Vec<String> {
        let allowed = self.allowed_tool_set().await;
        let native_lazy = self.supports_native_tool_lazy_loading();
        let direct_overrides = self.tool_direct_load_overrides();
        let loaded = self.loaded_tool_names.lock().await.clone();
        let mut names = Vec::new();
        push_unique_tool_name(&mut names, "tool_load");
        if !native_lazy {
            push_unique_tool_name(&mut names, "tool_call");
        }

        let mut allowed_sorted: Vec<_> = allowed.into_iter().collect();
        allowed_sorted.sort();
        for name in allowed_sorted {
            if self.should_direct_load_tool(&name, &direct_overrides)
                || (native_lazy && loaded.contains(&name))
            {
                push_unique_tool_name(&mut names, &name);
            }
        }
        names
    }

    async fn lazy_tool_manifest_names(&self) -> Vec<String> {
        let direct_overrides = self.tool_direct_load_overrides();
        let mut names: Vec<_> = self
            .allowed_tool_set()
            .await
            .into_iter()
            .filter(|name| {
                !Self::is_meta_tool(name)
                    && self.configured_tool_load_mode(name, &direct_overrides) == ToolLoadMode::Lazy
            })
            .collect();
        names.sort();
        names
    }

    fn format_lazy_tool_manifest(tool_names: &[String]) -> String {
        let mut lines = vec![
            "## Lazy Loaded Tools".to_string(),
            String::new(),
            "These tool schemas are available by name through `tool_load`:".to_string(),
        ];
        lines.extend(tool_names.iter().map(|name| format!("- `{}`", name)));
        lines.join("\n")
    }

    async fn lazy_tool_manifest_prompt(&self) -> Option<String> {
        let tool_names = self.lazy_tool_manifest_names().await;
        if tool_names.is_empty() {
            return None;
        }
        Some(Self::format_lazy_tool_manifest(&tool_names))
    }

    fn normalize_tool_call_names(&self, tool_calls: &mut [ToolCallInfo]) {
        for tool_call in tool_calls {
            if let Some(canonical) = self.tool_registry.canonical_name(&tool_call.name) {
                tool_call.name = canonical;
            }

            if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_mut() {
                self.normalize_tool_call_names(nested_tool_calls);
            }
        }
    }

    async fn rewrite_meta_tool_calls_for_execution(&self, tool_calls: &mut [ToolCallInfo]) {
        let allowed = self.allowed_tool_set().await;
        for tool_call in tool_calls {
            if tool_call.name == "tool_call" {
                match parse_meta_tool_call_arguments(&tool_call.arguments) {
                    Ok((target_name, mut target_args)) => {
                        let Some(canonical) = self.tool_registry.canonical_name(&target_name)
                        else {
                            eprintln!(
                                "[Agent {}] meta-call target '{}' is not registered",
                                self.id, target_name
                            );
                            continue;
                        };
                        if Self::is_meta_tool(&canonical) || !allowed.contains(&canonical) {
                            eprintln!(
                                "[Agent {}] meta-call target '{}' is not allowed for this agent",
                                self.id, canonical
                            );
                            continue;
                        }
                        normalize_tool_args(&mut target_args);
                        let target_arguments = serde_json::to_string(&target_args)
                            .unwrap_or_else(|_| "{}".to_string());
                        eprintln!(
                            "[Agent {}] meta-call dispatch: tool_call -> '{}' args_len={}",
                            self.id,
                            canonical,
                            target_arguments.len()
                        );
                        tool_call.name = canonical;
                        tool_call.arguments = target_arguments;
                    }
                    Err(error) => {
                        eprintln!(
                            "[Agent {}] invalid meta-call arguments for tool_call id={}: {}",
                            self.id, tool_call.id, error
                        );
                    }
                }
            }

            if let Some(nested_tool_calls) = tool_call.nested_tool_calls.as_mut() {
                Box::pin(self.rewrite_meta_tool_calls_for_execution(nested_tool_calls)).await;
            }
        }
    }

    async fn available_tool_prompt_items(&self) -> Vec<InjectedPromptItem> {
        let native_lazy = self.supports_native_tool_lazy_loading();
        let direct_overrides = self.tool_direct_load_overrides();
        let request_tool_names = self.build_request_tool_names().await;
        let mut direct_tool_names = HashSet::new();
        let mut tool_names = Vec::new();

        for name in request_tool_names {
            let Some(canonical) = self.tool_registry.canonical_name(&name) else {
                continue;
            };
            direct_tool_names.insert(canonical.clone());
            push_unique_tool_name(&mut tool_names, &canonical);
        }

        let mut allowed_tool_names: Vec<_> = self.allowed_tool_set().await.into_iter().collect();
        let allowed_tool_set: HashSet<String> = allowed_tool_names.iter().cloned().collect();
        allowed_tool_names.sort();
        for name in allowed_tool_names {
            push_unique_tool_name(&mut tool_names, &name);
        }

        if tool_names.is_empty() {
            return Vec::new();
        }

        tool_names
            .iter()
            .filter_map(|name| self.tool_registry.resolve_api_tool(name))
            .filter_map(|tool| {
                let (name, description) = extract_api_tool_name_and_description(&tool)?;
                let direct_loaded = direct_tool_names.contains(&name);
                let default_direct_load = self.default_direct_load_for_tool(&name);
                let can_configure_direct_load =
                    allowed_tool_set.contains(&name) && self.can_configure_direct_load_tool(&name);
                let direct_load_override = if can_configure_direct_load {
                    direct_overrides.get(&name).copied()
                } else {
                    None
                };
                let configured_load_mode = self.configured_tool_load_mode(&name, &direct_overrides);
                let load_mode = match configured_load_mode {
                    ToolLoadMode::Direct => "direct",
                    ToolLoadMode::Lazy => "lazy",
                    ToolLoadMode::Skill => "skill",
                };
                let is_built_in_tool = self.tool_registry.is_built_in(&name);
                let tool_source = if is_built_in_tool { "builtIn" } else { "skill" };
                let load_reason = if Self::is_meta_tool(&name) {
                    "meta_tool"
                } else if direct_load_override == Some(true) {
                    "override_direct"
                } else if direct_load_override == Some(false) {
                    "override_lazy"
                } else if configured_load_mode == ToolLoadMode::Skill {
                    "skill_only"
                } else if configured_load_mode == ToolLoadMode::Direct {
                    "default_direct"
                } else {
                    "default_lazy"
                };
                Some(InjectedPromptItem {
                    id: format!("available_tool::{}", name),
                    title: name,
                    kind: "tools".to_string(),
                    content: if description.is_empty() {
                        "*(No description provided)*".to_string()
                    } else {
                        description
                    },
                    source: "runtime".to_string(),
                    meta: Some(serde_json::json!({
                        "function": tool.get("function").cloned().unwrap_or_else(|| tool.clone()),
                        "loadMode": load_mode,
                        "loadReason": load_reason,
                        "directLoaded": direct_loaded,
                        "directLoadDefault": default_direct_load,
                        "directLoadOverride": direct_load_override,
                        "canConfigureDirectLoad": can_configure_direct_load,
                        "nativeLazy": native_lazy,
                        "toolSource": tool_source,
                    })),
                })
            })
            .collect()
    }

    pub async fn list_injected_prompt_items(&self) -> Vec<InjectedPromptItem> {
        if !self.has_selected_working_dir() {
            return Vec::new();
        }

        let mut items = Vec::new();
        let env_template = self.def.env_template.as_str();

        if self.knowledge_access_mode.allows_context() {
            if let Ok(rule_entries) =
                build_l3_rule_entries(&self.working_dir, self.app_knowledge_dir.as_ref().as_ref())
            {
                items.extend(rule_entries.into_iter().map(|entry| InjectedPromptItem {
                    id: format!("knowledge_rule::{}::{}", entry.doc_type, entry.path),
                    title: entry.title,
                    kind: "rule".to_string(),
                    content: entry.content,
                    source: "system".to_string(),
                    meta: Some(serde_json::json!({
                        "docType": entry.doc_type.as_str(),
                        "path": entry.path,
                        "injectMode": "rule",
                    })),
                }));
            }

            let include_index = env_template.contains("{{#knowledge_index}}");
            let include_memory = env_template.contains("{{#knowledge_memory}}");
            if env_template.contains("{{#knowledge}}") {
                if let Some(content) = self
                    .build_runtime_knowledge_block(include_index, include_memory)
                    .await
                {
                    items.push(InjectedPromptItem {
                        id: "knowledge_context".to_string(),
                        title: "Knowledge".to_string(),
                        kind: "context".to_string(),
                        content,
                        source: "system".to_string(),
                        meta: None,
                    });
                }
            }
        }

        if let Some(content) = self.lazy_tool_manifest_prompt().await {
            let tool_names = self.lazy_tool_manifest_names().await;
            items.push(InjectedPromptItem {
                id: "lazy_tool_names".to_string(),
                title: "Lazy Loaded Tools".to_string(),
                kind: "context".to_string(),
                content,
                source: "runtime".to_string(),
                meta: Some(serde_json::json!({
                    "toolNames": tool_names,
                    "loadMode": "lazy_manifest",
                })),
            });
        }

        items.extend(self.available_tool_prompt_items().await);

        let mut indexed_items: Vec<(usize, InjectedPromptItem)> =
            items.into_iter().enumerate().collect();
        indexed_items.sort_by_key(|(idx, item)| {
            let (section_order, prompt_order) =
                injected_item_prompt_sort_key(env_template, &item.id);
            (section_order, prompt_order, *idx)
        });

        indexed_items.into_iter().map(|(_, item)| item).collect()
    }

    pub async fn system_prompt_stats(&self) -> AgentSystemPromptStats {
        let parts = self.build_system_prompt_parts().await;
        let base_chars = parts.base_prompt.len();
        let env_chars = parts.env_prompt.len();
        let rules_chars = parts.rules_prompt.len();
        let knowledge_chars = parts.knowledge_prompt.len();

        AgentSystemPromptStats {
            base_chars,
            env_chars,
            rules_chars,
            knowledge_chars,
            total_chars: base_chars + env_chars + rules_chars + knowledge_chars,
        }
    }

    async fn build_api_tools(&self, tool_names: &[String]) -> Vec<serde_json::Value> {
        self.tool_registry.resolve_api_tools(tool_names)
    }

    ///
    async fn build_system_prompt_parts(&self) -> SystemPromptParts {
        let started_at = Instant::now();
        let has_working_dir = self.has_selected_working_dir();
        let os = std::env::consts::OS.to_string();
        let arch = std::env::consts::ARCH.to_string();
        let shell = crate::tool::builtins::shell_display_name().to_string();
        let python = crate::python_runtime::python_prompt_display(None);
        eprintln!(
            "[Agent {}] system prompt build start: session={} cwd={} has_working_dir={}",
            self.id, self.session_id, self.working_dir, has_working_dir
        );

        let unity_version = if has_working_dir {
            let version_path = std::path::Path::new(&self.working_dir)
                .join("ProjectSettings")
                .join("ProjectVersion.txt");
            std::fs::read_to_string(&version_path)
                .ok()
                .and_then(|content| {
                    content.lines().find_map(|line| {
                        line.strip_prefix("m_EditorVersion:")
                            .map(|v| v.trim().to_string())
                    })
                })
        } else {
            None
        };

        let mut unity_status = String::new();
        let mut unity_active_scene = String::new();
        let mut custom_tags = String::new();
        let mut layer_list = String::new();
        let mut physics_config = String::new();
        let mut input_system = String::new();
        let mut render_pipeline = String::new();

        // ── Unity-specific data ──────────────────────────────────
        let unity_started_at = Instant::now();
        if unity_version.is_some() {
            let (connected, status, active_scene) =
                crate::unity_bridge::query_unity_status(&self.working_dir).await;
            eprintln!(
                "[Agent {}] Unity status: connected={}, status={}, scene={:?}, cwd={}",
                self.id, connected, status, active_scene, self.working_dir
            );
            unity_status = crate::unity_bridge::format_editor_status_for_prompt(status).to_string();
            unity_active_scene = active_scene.unwrap_or_else(|| "unknown".to_string());

            let project_path = std::path::Path::new(&self.working_dir);
            let (tags, layers) = parse_tag_manager(project_path);

            {
                let tag_entries: Vec<String> = tags
                    .iter()
                    .map(|(idx, name)| format!("{}: {}", idx, name))
                    .collect();
                custom_tags = tag_entries.join(" | ");
            }

            if layers.is_empty() {
                layer_list = "(default only)".to_string();
            } else {
                let layer_entries: Vec<String> = layers
                    .iter()
                    .map(|(idx, name)| format!("{}: {}", idx, name))
                    .collect();
                layer_list = layer_entries.join(" | ");
            }

            physics_config = parse_physics_config(project_path, &layers);
            input_system = detect_input_system(project_path);
            render_pipeline = detect_render_pipeline(project_path);
        }
        eprintln!(
            "[Agent {}] system prompt unity context ready: session={} elapsed_ms={} unity_project={} tags_len={} layers_len={} physics_len={}",
            self.id,
            self.session_id,
            unity_started_at.elapsed().as_millis(),
            unity_version.is_some(),
            custom_tags.len(),
            layer_list.len(),
            physics_config.len()
        );

        let mut env = self.def.env_template.clone();

        env = env.replace("<os>", &os);
        env = env.replace("<arch>", &arch);
        env = env.replace("<shell>", &shell);
        env = env.replace("<python>", &python);
        env = env.replace(
            "<working_dir>",
            &Self::display_working_dir_value(&self.working_dir),
        );

        // Helper to remove a mustache block (e.g. {{#tag}}...{{/tag}})
        fn remove_block(env: &mut String, tag: &str) {
            let open = format!("{{{{#{}}}}}", tag);
            let close = format!("{{{{/{}}}}}", tag);
            while let (Some(start), Some(end)) = (env.find(&open), env.find(&close)) {
                env.replace_range(start..end + close.len(), "");
            }
        }

        // ── Git context ──────────────────────────────────────────
        let git_started_at = Instant::now();
        let mut git_available = false;
        let mut git_context_included = false;
        if has_working_dir {
            use crate::vcs::git::GitProvider;
            use crate::vcs::VcsProvider;
            let is_git = GitProvider.is_available(&self.working_dir).await;
            git_available = is_git;
            if is_git {
                let branch = GitProvider::current_branch(&self.working_dir)
                    .await
                    .unwrap_or_default();
                let commits = GitProvider::recent_commits(&self.working_dir, 10)
                    .await
                    .unwrap_or_default();

                if !branch.is_empty() || !commits.is_empty() {
                    git_context_included = true;
                    env = env.replace("{{#git}}", "");
                    env = env.replace("{{/git}}", "");
                    let branch_display = if branch.is_empty() {
                        "(detached HEAD)".to_string()
                    } else {
                        branch
                    };
                    env = env.replace("<git_branch>", &branch_display);
                    env = env.replace(
                        "<git_recent_commits>",
                        if commits.is_empty() {
                            "(no commits yet)"
                        } else {
                            &commits
                        },
                    );

                    let stat = GitProvider::uncommitted_summary(&self.working_dir)
                        .await
                        .unwrap_or_default();
                    if stat.is_empty() {
                        remove_block(&mut env, "git_uncommitted");
                    } else {
                        env = env.replace("{{#git_uncommitted}}", "");
                        env = env.replace("{{/git_uncommitted}}", "");
                        env = env.replace("<git_uncommitted_stat>", &stat);
                    }
                } else {
                    remove_block(&mut env, "git");
                }
            } else {
                remove_block(&mut env, "git");
            }
        } else {
            remove_block(&mut env, "git");
        }
        eprintln!(
            "[Agent {}] system prompt git context ready: session={} elapsed_ms={} git_available={} git_included={}",
            self.id,
            self.session_id,
            git_started_at.elapsed().as_millis(),
            git_available,
            git_context_included
        );

        remove_block(&mut env, "skills");

        let include_index = env.contains("{{#knowledge_index}}");
        let include_memory = env.contains("{{#knowledge_memory}}");
        let mut knowledge_prompt = String::new();
        let include_knowledge = env.contains("{{#knowledge}}") || include_index || include_memory;
        let knowledge_started_at = Instant::now();
        if has_working_dir && include_knowledge && self.knowledge_access_mode.allows_context() {
            if let Some(knowledge_block) = self
                .build_runtime_knowledge_block(include_index, include_memory)
                .await
            {
                knowledge_prompt = knowledge_block.trim().to_string();
                if env.contains("{{#knowledge}}") {
                    env = env.replace("{{#knowledge}}", "");
                    env = env.replace("{{/knowledge}}", "");
                    env = env.replace("<knowledge_context>", "");
                }
            } else {
                remove_block(&mut env, "knowledge");
            }
        } else {
            remove_block(&mut env, "knowledge");
        }
        remove_block(&mut env, "knowledge_index");
        remove_block(&mut env, "knowledge_memory");
        eprintln!(
            "[Agent {}] system prompt knowledge context ready: session={} elapsed_ms={} requested={} chars={}",
            self.id,
            self.session_id,
            knowledge_started_at.elapsed().as_millis(),
            include_knowledge,
            knowledge_prompt.len()
        );
        // ── Unity blocks ─────────────────────────────────────────
        if let Some(ver) = &unity_version {
            env = env.replace("{{#unity}}", "");
            env = env.replace("{{/unity}}", "");
            env = env.replace("<unity_version>", ver);
            env = env.replace("<unity_status>", &unity_status);
            env = env.replace("<unity_active_scene>", &unity_active_scene);
            env = env.replace("<render_pipeline>", &render_pipeline);
            env = env.replace("<input_system>", &input_system);
            env = env.replace("<custom_tags>", &custom_tags);
            env = env.replace("<layer_list>", &layer_list);
            env = env.replace("<physics_config>", &physics_config);
        } else {
            remove_block(&mut env, "unity");
        }

        if !has_working_dir {
            env.push_str(
                "\n\n## Workspace Status\nNo working directory is selected. Do not assume project files, Git state, Unity project metadata, knowledge base contents, or workspace-relative paths. If you need to inspect the runtime environment, use tools with an explicit working directory or absolute paths.",
            );
        }

        if has_working_dir {
            if let Some(lazy_tool_manifest) = self.lazy_tool_manifest_prompt().await {
                env.push_str("\n\n");
                env.push_str(&lazy_tool_manifest);
            }
        }

        let rules_started_at = Instant::now();
        let rules_prompt = {
            let rule_configs = crate::commands::merged_rule_config_for_agent(
                self.app_agent_dir.as_ref(),
                &self.working_dir,
                &self.def.id,
            );
            let mut rules_content = String::new();

            let mut rules_dirs: Vec<std::path::PathBuf> = Vec::new();
            if has_working_dir {
                let project_rules = std::path::Path::new(&self.working_dir)
                    .join("Locus")
                    .join("agent")
                    .join(&self.def.id)
                    .join("rule");
                if project_rules.is_dir() {
                    rules_dirs.push(project_rules);
                }
            }
            if let Some(app_dir) = self.app_agent_dir.as_ref() {
                let app_rules = app_dir.join(&self.def.id).join("rule");
                if app_rules.is_dir() {
                    rules_dirs.push(app_rules);
                }
            }

            if !rules_dirs.is_empty() {
                let mut rule_entries: Vec<(
                    String,
                    std::path::PathBuf,
                    crate::commands::RuleConfig,
                )> = Vec::new();
                for rules_path in &rules_dirs {
                    if let Ok(entries) = std::fs::read_dir(rules_path) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file()
                                && path.extension().and_then(|e| e.to_str()) == Some("md")
                            {
                                let file_name = entry.file_name().to_string_lossy().to_string();
                                if rule_entries.iter().any(|(n, _, _)| n == &file_name) {
                                    continue;
                                }
                                if Self::is_dev_runtime_knowledge_rule(&file_name) {
                                    continue;
                                }
                                let cfg = rule_configs.get(&file_name).cloned().unwrap_or_default();
                                if cfg.enabled {
                                    rule_entries.push((file_name, path, cfg));
                                }
                            }
                        }
                    }
                }

                rule_entries.sort_by(|a, b| a.2.order.cmp(&b.2.order).then(a.0.cmp(&b.0)));

                for (i, (_, path, _)) in rule_entries.iter().enumerate() {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        let content = content.trim();
                        if !content.is_empty() {
                            if !rules_content.is_empty() {
                                rules_content.push('\n');
                            }
                            rules_content.push_str(&format!("{}. {}\n", i + 1, content));
                        }
                    }
                }
            }

            let mut sections = Vec::new();
            if !rules_content.is_empty() {
                sections.push(format!(
                    "## Rules (IMPORTANT — follow these rules strictly)\n{}",
                    rules_content
                ));
            }
            if has_working_dir && self.knowledge_access_mode.allows_context() {
                if let Ok(l3_rules) = build_l3_rule_section(
                    &self.working_dir,
                    self.app_knowledge_dir.as_ref().as_ref(),
                ) {
                    if !l3_rules.trim().is_empty() {
                        sections.push(l3_rules);
                    }
                }
            }

            sections.join("\n\n")
        };
        eprintln!(
            "[Agent {}] system prompt rules ready: session={} elapsed_ms={} chars={}",
            self.id,
            self.session_id,
            rules_started_at.elapsed().as_millis(),
            rules_prompt.len()
        );

        while env.contains("\n\n\n") {
            env = env.replace("\n\n\n", "\n\n");
        }
        let env = env.trim().to_string();
        eprintln!(
            "[Agent {}] system prompt build finished: session={} elapsed_ms={} base_chars={} env_chars={} rules_chars={} knowledge_chars={}",
            self.id,
            self.session_id,
            started_at.elapsed().as_millis(),
            self.def.system_prompt.len(),
            env.len(),
            rules_prompt.len(),
            knowledge_prompt.len()
        );

        SystemPromptParts {
            base_prompt: self.def.system_prompt.clone(),
            rules_prompt,
            knowledge_prompt,
            env_prompt: env,
        }
    }

    fn inject_working_dir(&self, tool_name: &str, args: &mut serde_json::Value) {
        match tool_name {
            "bash" => {
                if let Some(dir) = args.get("workdir").and_then(|v| v.as_str()) {
                    if let Some(resolved) = self.resolve_path_against_working_dir(dir) {
                        args["workdir"] = serde_json::Value::String(resolved);
                    }
                } else if self.has_selected_working_dir() {
                    args["workdir"] = serde_json::Value::String(self.working_dir.clone());
                }
            }
            "grep" | "list" => {
                if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                    if let Some(resolved) = self.resolve_path_against_working_dir(p) {
                        args["path"] = serde_json::Value::String(resolved);
                    }
                } else if self.has_selected_working_dir() {
                    args["path"] = serde_json::Value::String(self.working_dir.clone());
                }
            }
            "read" | "write" | "edit" => {
                if let Some(fp) = args.get("filePath").and_then(|v| v.as_str()) {
                    if let Some(resolved) = self.resolve_path_against_working_dir(fp) {
                        args["filePath"] = serde_json::Value::String(resolved);
                    }
                }
            }
            "unity_log" | "unity_recompile" => {
                if let Some(path) = args.get("project_path").and_then(|v| v.as_str()) {
                    if let Some(resolved) = self.resolve_path_against_working_dir(path) {
                        args["project_path"] = serde_json::Value::String(resolved);
                    }
                } else if self.has_selected_working_dir() {
                    args["project_path"] = serde_json::Value::String(self.working_dir.clone());
                }
            }
            _ => {}
        }
    }

    fn normalize_path_for_compare(path: &std::path::Path) -> String {
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    }

    fn path_is_within_root(path: &std::path::Path, root: &std::path::Path) -> bool {
        let path_norm = Self::normalize_path_for_compare(path);
        let root_norm = Self::normalize_path_for_compare(root);
        path_norm == root_norm || path_norm.starts_with(&(root_norm + "/"))
    }

    fn shell_split_simple_segments(command: &str) -> Option<Vec<String>> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        let mut escaped = false;
        let mut chars = command.chars().peekable();

        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && quote != Some('\'') {
                current.push(ch);
                escaped = true;
                continue;
            }

            if let Some(active_quote) = quote {
                current.push(ch);
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }

            if ch == '\'' || ch == '"' {
                quote = Some(ch);
                current.push(ch);
                continue;
            }

            if (ch == '&' && chars.peek() == Some(&'&'))
                || (ch == '|' && chars.peek() == Some(&'|'))
            {
                chars.next();
                let segment = current.trim();
                if !segment.is_empty() {
                    segments.push(segment.to_string());
                }
                current.clear();
                continue;
            }

            if matches!(ch, '\n' | '\r') && current.trim_end().ends_with('|') {
                current.push(' ');
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }

            if matches!(ch, ';' | '\n' | '\r') {
                let segment = current.trim();
                if !segment.is_empty() {
                    segments.push(segment.to_string());
                }
                current.clear();
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }

            if matches!(ch, '<' | '>') {
                return None;
            }

            current.push(ch);
        }

        if quote.is_some() || escaped {
            return None;
        }

        let segment = current.trim();
        if !segment.is_empty() {
            segments.push(segment.to_string());
        }

        if segments.is_empty() {
            None
        } else {
            Some(segments)
        }
    }

    fn shell_split_pipeline(segment: &str) -> Option<Vec<String>> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        let mut escaped = false;
        let mut chars = segment.chars().peekable();

        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && quote != Some('\'') {
                current.push(ch);
                escaped = true;
                continue;
            }

            if let Some(active_quote) = quote {
                current.push(ch);
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }

            if ch == '\'' || ch == '"' {
                quote = Some(ch);
                current.push(ch);
                continue;
            }

            if ch == '|' {
                if chars.peek() == Some(&'|') {
                    return None;
                }
                let part = current.trim();
                if part.is_empty() {
                    return None;
                }
                parts.push(part.to_string());
                current.clear();
                continue;
            }

            if matches!(ch, '<' | '>') {
                return None;
            }

            current.push(ch);
        }

        if quote.is_some() || escaped {
            return None;
        }

        let part = current.trim();
        if part.is_empty() {
            return None;
        }
        parts.push(part.to_string());
        Some(parts)
    }

    fn shell_split_words(segment: &str) -> Option<Vec<String>> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut quote: Option<char> = None;
        let mut escaped = false;

        for ch in segment.chars() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }

            if let Some(active_quote) = quote {
                if ch == active_quote {
                    quote = None;
                } else {
                    current.push(ch);
                }
                continue;
            }

            if ch == '\'' || ch == '"' {
                quote = Some(ch);
                continue;
            }

            if ch.is_whitespace() {
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
                continue;
            }

            current.push(ch);
        }

        if quote.is_some() || escaped {
            return None;
        }

        if !current.is_empty() {
            words.push(current);
        }

        Some(words)
    }

    fn shell_tokens_contain_knowledge_literal(tokens: &[String]) -> bool {
        tokens.iter().any(|token| {
            token
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("locus/knowledge")
        })
    }

    fn shell_tokens_contain_command_substitution(tokens: &[String]) -> bool {
        tokens
            .iter()
            .any(|token| token.contains("$(") || token.contains('`'))
    }

    fn classify_safe_shell_tokens(tokens: &[String]) -> Option<GitCommandEffect> {
        let command = tokens.first().map(|value| value.to_ascii_lowercase())?;
        if matches!(command.as_str(), "echo" | "printf" | "true" | "false")
            && !Self::shell_tokens_contain_knowledge_literal(tokens)
            && !Self::shell_tokens_contain_command_substitution(tokens)
        {
            Some(GitCommandEffect {
                requires_confirm: false,
                reconcile_after_success: false,
                broad_scope: false,
            })
        } else {
            None
        }
    }

    fn shell_pipeline_filter_is_safe(tokens: &[String]) -> bool {
        let Some(command) = tokens.first().map(|value| value.to_ascii_lowercase()) else {
            return false;
        };
        if Self::shell_tokens_contain_knowledge_literal(tokens)
            || Self::shell_tokens_contain_command_substitution(tokens)
        {
            return false;
        }

        match command.as_str() {
            "cat" => tokens.iter().skip(1).all(|arg| arg.starts_with('-')),
            "head" | "tail" | "wc" | "sort" | "uniq" => tokens
                .iter()
                .skip(1)
                .all(|arg| arg.starts_with('-') || arg.chars().all(|ch| ch.is_ascii_digit())),
            "sed" => tokens.iter().skip(1).all(|arg| {
                arg.starts_with('-')
                    || arg
                        .chars()
                        .all(|ch| ch.is_ascii_digit() || matches!(ch, ',' | 'p' | 'q'))
            }),
            _ => false,
        }
    }

    fn is_git_executable(value: &str) -> bool {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "git" | "git.exe" | "git.cmd" | "git.bat"
        )
    }

    fn git_subcommand_index(tokens: &[String]) -> Option<usize> {
        if !tokens
            .first()
            .map(|value| Self::is_git_executable(value))
            .unwrap_or(false)
        {
            return None;
        }

        let mut index = 1;
        while index < tokens.len() {
            let token = tokens[index].as_str();
            match token {
                "-c" | "-C" | "--git-dir" | "--work-tree" | "--namespace" => {
                    index += 2;
                }
                "--no-pager" | "--paginate" => {
                    index += 1;
                }
                value
                    if value.starts_with("-c")
                        || value.starts_with("--git-dir=")
                        || value.starts_with("--work-tree=")
                        || value.starts_with("--namespace=") =>
                {
                    index += 1;
                }
                value if value.starts_with('-') => {
                    index += 1;
                }
                _ => return Some(index),
            }
        }
        None
    }

    fn git_args_contain_flag(args: &[String], long_flag: &str, short_flag: &str) -> bool {
        args.iter().any(|arg| arg == long_flag || arg == short_flag)
    }

    fn git_args_contain_any(args: &[String], flags: &[&str]) -> bool {
        args.iter().any(|arg| {
            flags
                .iter()
                .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))
        })
    }

    fn git_args_have_pathspec_separator(args: &[String]) -> bool {
        args.iter()
            .position(|arg| arg == "--")
            .map(|index| {
                args.iter()
                    .skip(index + 1)
                    .any(|arg| !arg.trim().is_empty())
            })
            .unwrap_or(false)
    }

    fn git_args_after_separator(args: &[String]) -> Vec<&str> {
        args.iter()
            .position(|arg| arg == "--")
            .map(|index| {
                args.iter()
                    .skip(index + 1)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn git_option_takes_value(arg: &str) -> bool {
        matches!(
            arg,
            "-m" | "-F"
                | "-C"
                | "-c"
                | "-S"
                | "-s"
                | "--message"
                | "--file"
                | "--author"
                | "--date"
                | "--reuse-message"
                | "--reedit-message"
                | "--source"
                | "--pathspec-from-file"
        )
    }

    fn git_collect_pathspec_like_args(args: &[String]) -> Vec<&str> {
        if let Some(index) = args.iter().position(|arg| arg == "--") {
            return args.iter().skip(index + 1).map(String::as_str).collect();
        }

        let mut pathspecs = Vec::new();
        let mut skip_next = false;
        for arg in args {
            if skip_next {
                skip_next = false;
                continue;
            }

            if arg.starts_with("--") {
                if !arg.contains('=') && Self::git_option_takes_value(arg) {
                    skip_next = true;
                }
                continue;
            }

            if arg.starts_with('-') {
                if Self::git_option_takes_value(arg) {
                    skip_next = true;
                }
                continue;
            }

            pathspecs.push(arg.as_str());
        }
        pathspecs
    }

    fn git_pathspecs_are_broad(pathspecs: &[&str]) -> bool {
        pathspecs.iter().any(|pathspec| {
            let trimmed = pathspec.trim();
            trimmed == "."
                || trimmed == ":/"
                || trimmed == ":(top)"
                || trimmed == "*"
                || trimmed == ":(glob)**"
        })
    }

    fn classify_simple_git_tokens(tokens: &[String]) -> Option<GitCommandEffect> {
        let subcommand_index = Self::git_subcommand_index(tokens)?;
        let subcommand = tokens.get(subcommand_index)?.to_ascii_lowercase();
        let args = &tokens[subcommand_index + 1..];

        if matches!(
            subcommand.as_str(),
            "status" | "diff" | "log" | "show" | "ls-files" | "rev-parse"
        ) {
            return Some(GitCommandEffect {
                requires_confirm: false,
                reconcile_after_success: false,
                broad_scope: false,
            });
        }

        match subcommand.as_str() {
            "add" => {
                let pathspecs = Self::git_collect_pathspec_like_args(args);
                let broad_scope = Self::git_args_contain_any(
                    args,
                    &["-A", "--all", "-u", "--update", "--renormalize"],
                ) || Self::git_pathspecs_are_broad(&pathspecs);
                Some(GitCommandEffect {
                    requires_confirm: false,
                    reconcile_after_success: false,
                    broad_scope,
                })
            }
            "restore" => {
                let staged = Self::git_args_contain_flag(args, "--staged", "-S");
                let worktree = Self::git_args_contain_flag(args, "--worktree", "-W");
                let pathspecs = Self::git_collect_pathspec_like_args(args);
                let broad_scope = Self::git_pathspecs_are_broad(&pathspecs);
                if staged && !worktree {
                    Some(GitCommandEffect {
                        requires_confirm: false,
                        reconcile_after_success: false,
                        broad_scope,
                    })
                } else {
                    Some(GitCommandEffect {
                        requires_confirm: true,
                        reconcile_after_success: true,
                        broad_scope,
                    })
                }
            }
            "reset" => {
                let hard = Self::git_args_contain_any(args, &["--hard"]);
                if hard {
                    return Some(GitCommandEffect {
                        requires_confirm: true,
                        reconcile_after_success: true,
                        broad_scope: true,
                    });
                }

                if Self::git_args_have_pathspec_separator(args) {
                    let pathspecs = Self::git_args_after_separator(args);
                    return Some(GitCommandEffect {
                        requires_confirm: false,
                        reconcile_after_success: false,
                        broad_scope: Self::git_pathspecs_are_broad(&pathspecs),
                    });
                }

                Some(GitCommandEffect {
                    requires_confirm: true,
                    reconcile_after_success: true,
                    broad_scope: true,
                })
            }
            "checkout" => {
                let pathspecs = Self::git_args_after_separator(args);
                Some(GitCommandEffect {
                    requires_confirm: true,
                    reconcile_after_success: true,
                    broad_scope: pathspecs.is_empty() || Self::git_pathspecs_are_broad(&pathspecs),
                })
            }
            "stash" => {
                let action = args
                    .iter()
                    .find(|arg| !arg.starts_with('-'))
                    .map(|arg| arg.to_ascii_lowercase())
                    .unwrap_or_else(|| "push".to_string());
                if matches!(action.as_str(), "list" | "show") {
                    Some(GitCommandEffect {
                        requires_confirm: false,
                        reconcile_after_success: false,
                        broad_scope: false,
                    })
                } else {
                    Some(GitCommandEffect {
                        requires_confirm: matches!(
                            action.as_str(),
                            "apply" | "pop" | "push" | "save"
                        ),
                        reconcile_after_success: matches!(
                            action.as_str(),
                            "apply" | "pop" | "push" | "save"
                        ),
                        broad_scope: true,
                    })
                }
            }
            "merge" | "rebase" | "cherry-pick" | "revert" => Some(GitCommandEffect {
                requires_confirm: true,
                reconcile_after_success: true,
                broad_scope: true,
            }),
            "commit" => Some(GitCommandEffect {
                requires_confirm: false,
                reconcile_after_success: false,
                broad_scope: false,
            }),
            _ => None,
        }
    }

    fn classify_bash_git_command(command: &str) -> Option<GitCommandEffect> {
        let segments = Self::shell_split_simple_segments(command)?;
        let mut combined = GitCommandEffect {
            requires_confirm: false,
            reconcile_after_success: false,
            broad_scope: false,
        };

        for segment in segments {
            let pipeline = Self::shell_split_pipeline(&segment)?;
            let tokens = Self::shell_split_words(pipeline.first()?)?;
            if tokens.is_empty() {
                continue;
            }
            let effect = Self::classify_safe_shell_tokens(&tokens)
                .or_else(|| Self::classify_simple_git_tokens(&tokens))?;
            for filter in pipeline.iter().skip(1) {
                let filter_tokens = Self::shell_split_words(filter)?;
                if !Self::shell_pipeline_filter_is_safe(&filter_tokens) {
                    return None;
                }
            }
            combined.requires_confirm |= effect.requires_confirm;
            combined.reconcile_after_success |= effect.reconcile_after_success;
            combined.broad_scope |= effect.broad_scope;
        }

        Some(combined)
    }

    fn assess_bash_git_knowledge_command(
        working_dir: &str,
        app_knowledge_dir: Option<&std::path::PathBuf>,
        args: &serde_json::Value,
    ) -> Option<BashGitKnowledgeAssessment> {
        let workdir = args
            .get("workdir")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let command = args.get("command").and_then(|value| value.as_str())?;
        let effect = Self::classify_bash_git_command(command)?;
        let workdir_targets_knowledge =
            Self::path_targets_knowledge_root(working_dir, app_knowledge_dir, workdir);
        let command_targets_knowledge =
            Self::shell_command_mentions_knowledge_root(working_dir, app_knowledge_dir, command);
        let touches_knowledge =
            workdir_targets_knowledge || command_targets_knowledge || effect.broad_scope;

        Some(BashGitKnowledgeAssessment {
            touches_knowledge,
            requires_confirm: touches_knowledge && effect.requires_confirm,
            reconcile_after_success: touches_knowledge && effect.reconcile_after_success,
        })
    }

    fn collect_knowledge_roots(
        working_dir: &str,
        app_knowledge_dir: Option<&std::path::PathBuf>,
    ) -> Vec<std::path::PathBuf> {
        let mut roots = Vec::new();
        if Self::has_selected_working_dir_value(working_dir) {
            roots.push(crate::knowledge_store::knowledge_root(working_dir));
        }
        if let Some(app_root) = app_knowledge_dir {
            roots.push(app_root.clone());
        }
        roots
    }

    fn path_targets_knowledge_root(
        working_dir: &str,
        app_knowledge_dir: Option<&std::path::PathBuf>,
        raw_path: &str,
    ) -> bool {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return false;
        }

        let resolved_path = if std::path::Path::new(trimmed).is_absolute() {
            std::path::PathBuf::from(trimmed)
        } else if Self::has_selected_working_dir_value(working_dir) {
            std::path::Path::new(working_dir).join(trimmed)
        } else {
            std::path::PathBuf::from(trimmed)
        };

        Self::collect_knowledge_roots(working_dir, app_knowledge_dir)
            .iter()
            .any(|root| Self::path_is_within_root(&resolved_path, root))
    }

    fn is_shell_path_boundary(ch: Option<char>, allow_path_child: bool) -> bool {
        match ch {
            None => true,
            Some('/') if allow_path_child => true,
            Some(ch) => ch.is_whitespace() || matches!(ch, '\'' | '"' | '`' | '(' | ')' | '='),
        }
    }

    fn shell_command_mentions_path(command_norm: &str, path_norm: &str) -> bool {
        let needle = path_norm.trim_matches('/');
        if needle.is_empty() {
            return false;
        }

        let mut search_from = 0;
        while let Some(offset) = command_norm[search_from..].find(needle) {
            let start = search_from + offset;
            let end = start + needle.len();
            let before = command_norm[..start].chars().next_back();
            let after = command_norm[end..].chars().next();
            if Self::is_shell_path_boundary(before, false)
                && Self::is_shell_path_boundary(after, true)
            {
                return true;
            }
            search_from = end;
            if search_from >= command_norm.len() {
                break;
            }
        }
        false
    }

    fn shell_command_mentions_knowledge_root(
        working_dir: &str,
        app_knowledge_dir: Option<&std::path::PathBuf>,
        command: &str,
    ) -> bool {
        let command_norm = command.replace('\\', "/").to_ascii_lowercase();
        if Self::shell_command_mentions_path(&command_norm, "locus/knowledge") {
            return true;
        }

        for root in Self::collect_knowledge_roots(working_dir, app_knowledge_dir) {
            let root_norm = Self::normalize_path_for_compare(&root);
            if Self::shell_command_mentions_path(&command_norm, &root_norm) {
                return true;
            }

            if Self::has_selected_working_dir_value(working_dir) {
                if let Ok(relative) = root.strip_prefix(working_dir) {
                    let relative_norm = Self::normalize_path_for_compare(relative);
                    if Self::shell_command_mentions_path(&command_norm, &relative_norm) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn validate_knowledge_tool_routing(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<String> {
        fn knowledge_tool_routing_error() -> String {
            "Knowledge roots are reserved for knowledge tools. Use `knowledge_list` / `knowledge_query` / `knowledge_read` for inspection, `knowledge_create` / `knowledge_edit` / `knowledge_move` / `knowledge_delete` for non-Skill writes, and `skill_create` / `skill_reload` for Skill lifecycle work."
                .to_string()
        }

        let app_root = self.app_knowledge_dir.as_ref().as_ref();
        match tool_name {
            "read" | "write" | "edit" => {
                let file_path = args.get("filePath").and_then(|value| value.as_str())?;
                if Self::path_targets_knowledge_root(&self.working_dir, app_root, file_path) {
                    Some(knowledge_tool_routing_error())
                } else {
                    None
                }
            }
            "grep" | "list" => {
                let path = args.get("path").and_then(|value| value.as_str())?;
                if Self::path_targets_knowledge_root(&self.working_dir, app_root, path) {
                    Some(knowledge_tool_routing_error())
                } else {
                    None
                }
            }
            "bash" => {
                let workdir = args
                    .get("workdir")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let command = args
                    .get("command")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if Self::path_targets_knowledge_root(&self.working_dir, app_root, workdir)
                    || Self::shell_command_mentions_knowledge_root(
                        &self.working_dir,
                        app_root,
                        command,
                    )
                {
                    if Self::assess_bash_git_knowledge_command(&self.working_dir, app_root, args)
                        .is_some()
                    {
                        None
                    } else {
                        Some(knowledge_tool_routing_error())
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn strip_knowledge_type_prefix(path: &str) -> &str {
        path.trim()
            .strip_prefix("design/")
            .or_else(|| path.trim().strip_prefix("memory/"))
            .or_else(|| path.trim().strip_prefix("skill/"))
            .or_else(|| path.trim().strip_prefix("reference/"))
            .unwrap_or(path.trim())
    }

    fn prefix_knowledge_tool_path(
        doc_type: crate::knowledge_store::KnowledgeType,
        path: &str,
    ) -> String {
        let trimmed = path.trim().trim_matches('/');
        let normalized = trimmed.strip_prefix("Locus/knowledge/").unwrap_or(trimmed);
        if crate::knowledge_store::infer_type_from_path(normalized) == Some(doc_type) {
            normalized.to_string()
        } else {
            let suffix = Self::strip_knowledge_type_prefix(normalized).trim_matches('/');
            if suffix.is_empty() {
                doc_type.as_str().to_string()
            } else {
                format!("{}/{}", doc_type.as_str(), suffix)
            }
        }
    }

    fn prefix_knowledge_list_item_paths(items: &mut [crate::knowledge_store::KnowledgeListItem]) {
        for item in items {
            item.path = Self::prefix_knowledge_tool_path(item.doc_type, &item.path);
        }
    }

    fn prefix_knowledge_search_hit_paths(items: &mut [crate::knowledge_store::KnowledgeSearchHit]) {
        for item in items {
            item.path = Self::prefix_knowledge_tool_path(item.doc_type, &item.path);
        }
    }

    fn prefix_knowledge_document_path(document: &mut crate::knowledge_store::KnowledgeDocument) {
        document.path = Self::prefix_knowledge_tool_path(document.doc_type, &document.path);
    }

    fn prefix_knowledge_directory_record_path(
        directory: &mut crate::knowledge_store::KnowledgeDirectoryConfigRecord,
    ) {
        directory.path = Self::prefix_knowledge_tool_path(directory.doc_type, &directory.path);
        directory.config_path =
            Self::prefix_knowledge_tool_path(directory.doc_type, &directory.config_path);
    }

    fn prefix_knowledge_read_response_paths(
        response: &mut crate::knowledge_store::KnowledgeReadResponse,
    ) {
        if let Some(document) = response.document.as_mut() {
            Self::prefix_knowledge_document_path(&mut document.document);
        }
        if let Some(directory) = response.directory.as_mut() {
            Self::prefix_knowledge_directory_record_path(directory);
        }
    }

    fn prefix_knowledge_mutation_response_paths(
        response: &mut crate::knowledge_store::KnowledgeMutationResponse,
    ) {
        response.path = Self::prefix_knowledge_tool_path(response.doc_type, &response.path);
        if let Some(result_path) = response.result_path.as_mut() {
            *result_path = Self::prefix_knowledge_tool_path(response.doc_type, result_path);
        }
        if let Some(document) = response.document.as_mut() {
            Self::prefix_knowledge_document_path(document);
        }
        if let Some(directory) = response.directory.as_mut() {
            Self::prefix_knowledge_directory_record_path(directory);
        }
    }

    fn sanitize_knowledge_list_items(
        items: Vec<crate::knowledge_store::KnowledgeListItem>,
    ) -> Vec<AgentKnowledgeListItem> {
        items
            .into_iter()
            .map(|item| AgentKnowledgeListItem {
                doc_type: item.doc_type,
                path: item.path,
                title: item.title,
            })
            .collect()
    }

    fn sanitize_knowledge_search_hits(
        items: Vec<crate::knowledge_store::KnowledgeSearchHit>,
    ) -> Vec<AgentKnowledgeSearchHit> {
        items
            .into_iter()
            .map(|item| AgentKnowledgeSearchHit {
                doc_type: item.doc_type,
                path: item.path,
                title: item.title,
                snippet: item.snippet,
                matched_section: item.matched_section,
                score: item.score,
                match_kind: item.match_kind,
            })
            .collect()
    }

    fn sanitize_knowledge_document_content(
        document: &crate::knowledge_store::KnowledgeDocument,
        part: &str,
    ) -> AgentKnowledgeDocumentContent {
        let summary = if part == "body" {
            None
        } else {
            crate::knowledge_store::active_summary(document).map(str::to_string)
        };
        let maintenance_rules = if part == "full" {
            crate::knowledge_store::active_maintenance_rules(document).map(str::to_string)
        } else {
            None
        };
        let body = if part == "summary" {
            None
        } else {
            Some(document.body.clone())
        };

        AgentKnowledgeDocumentContent {
            doc_type: document.doc_type,
            path: document.path.clone(),
            title: document.title.clone(),
            summary,
            maintenance_rules,
            body,
        }
    }

    fn sanitize_knowledge_read_response(
        response: crate::knowledge_store::KnowledgeReadResponse,
    ) -> Result<AgentKnowledgeReadResponse, String> {
        let document = response
            .document
            .ok_or_else(|| "knowledge_read returned no document".to_string())?;
        Ok(AgentKnowledgeReadResponse {
            document: Self::sanitize_knowledge_document_content(
                &document.document,
                document.part.as_str(),
            ),
            part: document.part,
        })
    }

    fn sanitize_knowledge_mutation_response(
        response: crate::knowledge_store::KnowledgeMutationResponse,
    ) -> AgentKnowledgeMutationResponse {
        let crate::knowledge_store::KnowledgeMutationResponse {
            kind,
            doc_type,
            path,
            result_path,
            document,
            ..
        } = response;

        AgentKnowledgeMutationResponse {
            kind,
            doc_type,
            path,
            result_path,
            document: document
                .as_ref()
                .map(|document| Self::sanitize_knowledge_document_content(document, "full")),
        }
    }

    fn format_knowledge_list_output(items: &[AgentKnowledgeListItem]) -> String {
        if items.is_empty() {
            return "(empty)".to_string();
        }

        items
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_knowledge_query_output(items: &[AgentKnowledgeSearchHit]) -> String {
        if items.is_empty() {
            return "No results.".to_string();
        }

        let mut output = String::new();
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                output.push_str("\n\n");
            }

            output.push_str(&item.path);
            output.push('\n');
            output.push_str("  ");
            output.push_str(item.title.trim());
            output.push('\n');
            output.push_str("  match=");
            output.push_str(item.match_kind.trim());
            if let Some(section) = item.matched_section {
                output.push_str(" | section=");
                output.push_str(match section {
                    crate::knowledge_store::KnowledgeSearchMatchSection::Summary => "summary",
                    crate::knowledge_store::KnowledgeSearchMatchSection::MaintenanceRules => {
                        "maintenance_rules"
                    }
                    crate::knowledge_store::KnowledgeSearchMatchSection::Body => "body",
                });
            }
            output.push_str(&format!(" | score={:.3}", item.score));

            let snippet = item.snippet.trim();
            if !snippet.is_empty() {
                for line in snippet.lines() {
                    output.push('\n');
                    output.push_str("  ");
                    output.push_str(line.trim_end());
                }
            }
        }

        output
    }

    fn format_knowledge_read_output(response: &AgentKnowledgeReadResponse) -> String {
        match response.part.as_str() {
            "summary" => response
                .document
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("<empty>")
                .to_string(),
            "body" => response
                .document
                .body
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("<empty>")
                .to_string(),
            _ => {
                let mut output = String::new();
                output.push_str("# ");
                output.push_str(response.document.title.trim());
                output.push_str("\n\n");

                if let Some(summary) = response
                    .document
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    output.push_str("## Summary\n");
                    output.push_str(summary);
                    output.push_str("\n\n");
                }

                if let Some(rules) = response
                    .document
                    .maintenance_rules
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    output.push_str("## Maintenance Rules\n");
                    output.push_str(rules);
                    output.push_str("\n\n");
                }

                output.push_str("## Content\n");
                let body = response.document.body.as_deref().unwrap_or("").trim_end();
                output.push_str(body);
                output.push('\n');
                output
            }
        }
    }

    fn format_knowledge_mutation_output(
        action: &str,
        response: &AgentKnowledgeMutationResponse,
    ) -> String {
        let target = match response.kind {
            crate::knowledge_store::KnowledgeTargetKind::Document => "knowledge document",
            crate::knowledge_store::KnowledgeTargetKind::Directory => "knowledge directory",
        };

        if action == "Moved" {
            let result_path = response.result_path.as_deref().unwrap_or(&response.path);
            if result_path != response.path {
                return format!("{action} {target} {} -> {}", response.path, result_path);
            }
            return format!("{action} {target} {}", response.path);
        }

        let path = response.result_path.as_deref().unwrap_or(&response.path);
        format!("{action} {target} {path}")
    }

    fn is_unity_asset_write_call(&self, tc: &ToolCallInfo, args: &serde_json::Value) -> bool {
        if tc.name != "write" && tc.name != "edit" {
            return false;
        }

        let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return false,
        };

        let path = std::path::Path::new(file_path);
        let working_dir = std::path::Path::new(&self.working_dir);
        let assets_dir = working_dir.join("Assets");
        let packages_dir = working_dir.join("Packages");
        path.starts_with(&assets_dir) || path.starts_with(&packages_dir)
    }

    fn unity_asset_relative_path(
        &self,
        tc: &ToolCallInfo,
        args: &serde_json::Value,
        result: &ExecutedToolResult,
    ) -> Option<String> {
        if result.outcome != ToolRunOutcome::Done || !self.is_unity_asset_write_call(tc, args) {
            return None;
        }

        let file_path = args.get("filePath").and_then(|v| v.as_str())?;
        let working_dir = std::path::Path::new(&self.working_dir);
        let path = std::path::Path::new(file_path);
        let relative = path.strip_prefix(working_dir).ok()?;
        Some(relative.to_string_lossy().replace('\\', "/"))
    }

    async fn cleanup_unity_edit_session(&self) {
        if !crate::unity_bridge::is_unity_project(&self.working_dir) {
            return;
        }

        match crate::unity_bridge::end_edit_session(&self.working_dir, &self.session_id).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "[Agent {}] failed to clean up Unity edit session for {}: {}",
                    self.id, self.session_id, e
                );
                Self::retry_unity_edit_session_cleanup(
                    self.id.clone(),
                    self.working_dir.clone(),
                    self.session_id.clone(),
                );
            }
        }
    }

    fn retry_unity_edit_session_cleanup(agent_id: String, working_dir: String, session_id: String) {
        tokio::spawn(async move {
            const MAX_ATTEMPTS: u32 = 24;

            for attempt in 1..=MAX_ATTEMPTS {
                let delay_secs = if attempt <= 5 { 1 } else { 5 };
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

                match crate::unity_bridge::end_edit_session(&working_dir, &session_id).await {
                    Ok(msg) => {
                        eprintln!(
                            "[Agent {}] Unity edit session cleanup retry succeeded for {} after attempt {}: {}",
                            agent_id, session_id, attempt, msg
                        );
                        return;
                    }
                    Err(error) => {
                        if attempt == MAX_ATTEMPTS || attempt % 5 == 0 {
                            eprintln!(
                                "[Agent {}] Unity edit session cleanup retry failed for {} attempt {}/{}: {}",
                                agent_id, session_id, attempt, MAX_ATTEMPTS, error
                            );
                        }
                    }
                }
            }
        });
    }

    #[allow(dead_code)]
    pub fn spawn_child(
        &self,
        child_def_id: &str,
        store: &SessionStore,
    ) -> Result<AgentInstance, String> {
        let child_def = self
            .registry
            .get(child_def_id)
            .ok_or_else(|| format!("AgentDef '{}' not found", child_def_id))?;

        let child_session_id = store.create_session(
            &format!("sub:{}", child_def_id),
            Some(&self.session_id),
            self.workspace_id.as_deref(),
            "chat",
            Some(child_def_id),
        )?;

        Ok(AgentInstance::new(
            Arc::new(child_def.clone()),
            &child_session_id,
            self.backend.clone(),
            self.debug,
            self.registry.clone(),
            self.tool_registry.clone(),
            self.working_dir.clone(),
            self.raw_store.clone(),
            self.workspace_id.clone(),
            self.resolve_subagent_model_name(child_def_id)
                .unwrap_or_else(|| self.effective_model.clone()),
            self.effort.clone(),
            self.app_knowledge_dir.clone(),
            self.app_agent_dir.clone(),
            self.knowledge_access_mode,
            self.undo_manager.clone(),
            self.subagent_model_overrides.clone(),
            self.cancel_waiter(),
        ))
    }

    async fn call_llm(
        &self,
        store: &SessionStore,
        codex_turn_state: Option<&mut codex::TurnState>,
        system_parts: &[&str],
        messages: &[crate::session::models::ChatMessage],
        api_tools: &[serde_json::Value],
        on_text_delta: impl Fn(String) + Send + Sync + 'static,
        on_thinking_delta: impl Fn(String) + Send + Sync + 'static,
        on_tool_call_start: impl Fn(String, String) + Send + Sync + 'static,
    ) -> Result<LlmCallResult, String> {
        match &self.backend {
            LlmBackend::OpenRouter { api_key, base_url } => {
                let system_prompt = system_parts.join("\n\n");
                let api_model = resolve_openrouter_model(&self.effective_model);
                let resp = openrouter::stream_chat(
                    api_key,
                    &api_model,
                    &system_prompt,
                    messages,
                    api_tools,
                    base_url.as_deref(),
                    None, // api_path: defaults to /api/v1/chat/completions
                    None, // provider_tag
                    &[],  // extra_headers
                    None, // reasoning_effort
                    self.debug,
                    on_text_delta,
                    on_tool_call_start,
                )
                .await?;
                Ok(LlmCallResult {
                    text: resp.text,
                    tool_calls: resp.tool_calls,
                    finish_reason: resp.finish_reason,
                    response_id: resp.response_id,
                    input_tokens: resp.input_tokens,
                    output_tokens: resp.output_tokens,
                    cache_read_tokens: resp.cache_read_tokens,
                    cache_write_tokens: resp.cache_write_tokens,
                    cost_usd: resp.cost_usd,
                    raw_request: resp.raw_request,
                    raw_response: resp.raw_response,
                    thinking_text: resp.thinking_text,
                    thinking_duration_secs: resp.thinking_duration_secs,
                    thinking_signature: resp.thinking_signature,
                    continuation_request: None,
                })
            }
            LlmBackend::Anthropic {
                access_token,
                base_url,
                user_metadata,
            } => {
                let resp = anthropic::stream_chat(
                    access_token,
                    &self.effective_model,
                    user_metadata,
                    system_parts,
                    messages,
                    api_tools,
                    base_url.as_deref(),
                    Some(&self.session_id),
                    self.effort.as_deref(),
                    on_text_delta,
                    on_thinking_delta,
                    on_tool_call_start,
                )
                .await?;
                Ok(LlmCallResult {
                    text: resp.text,
                    tool_calls: resp.tool_calls,
                    finish_reason: resp.finish_reason,
                    response_id: None,
                    input_tokens: resp.input_tokens,
                    output_tokens: resp.output_tokens,
                    cache_read_tokens: resp.cache_read_tokens,
                    cache_write_tokens: resp.cache_write_tokens,
                    cost_usd: 0.0,
                    raw_request: resp.raw_request,
                    raw_response: resp.raw_response,
                    thinking_text: resp.thinking_text,
                    thinking_duration_secs: resp.thinking_duration_secs,
                    thinking_signature: resp.thinking_signature,
                    continuation_request: None,
                })
            }
            LlmBackend::AnthropicAgentSdk => {
                Err("Anthropic Agent SDK backend uses a dedicated run path".to_string())
            }
            LlmBackend::OpenAiCodex {
                auth,
                transport,
                base_url,
            } => {
                let system_prompt = system_parts.join("\n\n");
                let model_name = &self.effective_model;
                let actual_model = model_name.strip_prefix("openai/").unwrap_or(model_name);
                let mut owned_turn_state = codex::TurnState::default();
                let codex_turn_state = codex_turn_state.unwrap_or(&mut owned_turn_state);
                let response_request_metadata =
                    store.get_response_request_metadata(&self.session_id)?;
                let (access_token, account_id) = resolve_codex_request_auth(auth, false)
                    .await
                    .map_err(|e| format!("OpenAI Codex token failed (please re-login): {}", e))?;
                let resp = match codex::stream_chat(
                    &access_token,
                    account_id.as_deref(),
                    *transport,
                    base_url.as_deref(),
                    actual_model,
                    &system_prompt,
                    messages,
                    api_tools,
                    self.effort.as_deref(),
                    self.debug,
                    Some(&self.session_id),
                    Some(&response_request_metadata),
                    codex_turn_state,
                    &on_text_delta,
                    &on_thinking_delta,
                    &on_tool_call_start,
                )
                .await
                {
                    Ok(resp) => resp,
                    Err(error) if is_codex_unauthorized_error(&error) => {
                        eprintln!(
                            "[OpenAI Codex] received unauthorized response, refreshing auth and retrying once"
                        );
                        let (access_token, account_id) = resolve_codex_request_auth(auth, true)
                            .await
                            .map_err(|e| format!("OpenAI Codex token refresh failed: {}", e))?;
                        codex::stream_chat(
                            &access_token,
                            account_id.as_deref(),
                            *transport,
                            base_url.as_deref(),
                            actual_model,
                            &system_prompt,
                            messages,
                            api_tools,
                            self.effort.as_deref(),
                            self.debug,
                            Some(&self.session_id),
                            Some(&response_request_metadata),
                            codex_turn_state,
                            &on_text_delta,
                            &on_thinking_delta,
                            &on_tool_call_start,
                        )
                        .await?
                    }
                    Err(error) => return Err(error),
                };
                Ok(LlmCallResult {
                    text: resp.text,
                    tool_calls: resp.tool_calls,
                    finish_reason: resp.finish_reason,
                    response_id: resp.response_id,
                    input_tokens: resp.input_tokens,
                    output_tokens: resp.output_tokens,
                    cache_read_tokens: resp.cache_read_tokens,
                    cache_write_tokens: resp.cache_write_tokens,
                    cost_usd: 0.0,
                    raw_request: resp.raw_request,
                    raw_response: resp.raw_response,
                    thinking_text: resp.thinking_text,
                    thinking_duration_secs: resp.thinking_duration_secs,
                    thinking_signature: resp.thinking_signature,
                    continuation_request: resp.continuation_request,
                })
            }
            LlmBackend::Custom {
                api_key,
                api_model,
                endpoint,
                api_format,
                beta_flags,
                supported_reasoning_efforts,
                reasoning_param_format,
                replay_reasoning_content,
                server_tools,
                supports_tool_lazy_loading: _,
                supports_vision,
                ..
            } => {
                use crate::commands::{ApiFormat, CustomReasoningParamFormat};
                if !supports_vision && messages_have_images(messages) {
                    return Err(no_vision_endpoint_error());
                }
                let custom_reasoning_effort = crate::llm::openai_reasoning::custom_reasoning_effort(
                    self.effort.as_deref(),
                    supported_reasoning_efforts,
                );
                match api_format {
                    ApiFormat::OpenaiChat => {
                        let system_prompt = system_parts.join("\n\n");
                        let reasoning_effort = matches!(
                            reasoning_param_format,
                            CustomReasoningParamFormat::OpenaiChatReasoningEffort
                        )
                        .then_some(custom_reasoning_effort.as_deref())
                        .flatten();
                        let thinking_level = matches!(
                            reasoning_param_format,
                            CustomReasoningParamFormat::OpenaiChatReasoningEffort
                        )
                        .then_some(self.effort.as_deref())
                        .flatten();
                        let resp = chat_completions::stream_chat(
                            api_key,
                            api_model,
                            &system_prompt,
                            messages,
                            api_tools,
                            endpoint.as_str(),
                            reasoning_effort,
                            thinking_level,
                            *replay_reasoning_content,
                            self.debug,
                            on_text_delta,
                            on_thinking_delta,
                            on_tool_call_start,
                        )
                        .await?;
                        Ok(LlmCallResult {
                            text: resp.text,
                            tool_calls: resp.tool_calls,
                            finish_reason: resp.finish_reason,
                            response_id: resp.response_id,
                            input_tokens: resp.input_tokens,
                            output_tokens: resp.output_tokens,
                            cache_read_tokens: resp.cache_read_tokens,
                            cache_write_tokens: resp.cache_write_tokens,
                            cost_usd: resp.cost_usd,
                            raw_request: resp.raw_request,
                            raw_response: resp.raw_response,
                            thinking_text: resp.thinking_text,
                            thinking_duration_secs: resp.thinking_duration_secs,
                            thinking_signature: resp.thinking_signature,
                            continuation_request: None,
                        })
                    }
                    ApiFormat::OpenaiResponses => {
                        let system_prompt = system_parts.join("\n\n");
                        let reasoning_effort = matches!(
                            reasoning_param_format,
                            CustomReasoningParamFormat::OpenaiResponsesReasoningEffort
                        )
                        .then_some(custom_reasoning_effort.as_deref())
                        .flatten();
                        let resp = responses::stream_chat(
                            api_key,
                            api_model,
                            &system_prompt,
                            messages,
                            api_tools,
                            endpoint.as_str(),
                            self.effort.as_deref(),
                            reasoning_effort,
                            self.debug,
                            Some(&self.session_id),
                            on_text_delta,
                            on_thinking_delta,
                            on_tool_call_start,
                        )
                        .await?;
                        Ok(LlmCallResult {
                            text: resp.text,
                            tool_calls: resp.tool_calls,
                            finish_reason: resp.finish_reason,
                            response_id: resp.response_id,
                            input_tokens: resp.input_tokens,
                            output_tokens: resp.output_tokens,
                            cache_read_tokens: resp.cache_read_tokens,
                            cache_write_tokens: resp.cache_write_tokens,
                            cost_usd: resp.cost_usd,
                            raw_request: resp.raw_request,
                            raw_response: resp.raw_response,
                            thinking_text: resp.thinking_text,
                            thinking_duration_secs: resp.thinking_duration_secs,
                            thinking_signature: resp.thinking_signature,
                            continuation_request: None,
                        })
                    }
                    ApiFormat::AnthropicMessages => {
                        let system_prompt = system_parts.join("\n\n");
                        let thinking_level = matches!(
                            reasoning_param_format,
                            CustomReasoningParamFormat::AnthropicThinking
                        )
                        .then_some(custom_reasoning_effort.as_deref())
                        .flatten();
                        let resp = anthropic::stream_chat_native(
                            api_key,
                            api_model,
                            &system_prompt,
                            messages,
                            api_tools,
                            endpoint.as_str(),
                            beta_flags,
                            thinking_level,
                            *replay_reasoning_content,
                            server_tools.web_search,
                            Some(&self.session_id),
                            "Custom(Anthropic)",
                            self.debug,
                            on_text_delta,
                            on_thinking_delta,
                            on_tool_call_start,
                        )
                        .await?;
                        Ok(LlmCallResult {
                            text: resp.text,
                            tool_calls: resp.tool_calls,
                            finish_reason: resp.finish_reason,
                            response_id: None,
                            input_tokens: resp.input_tokens,
                            output_tokens: resp.output_tokens,
                            cache_read_tokens: resp.cache_read_tokens,
                            cache_write_tokens: resp.cache_write_tokens,
                            cost_usd: 0.0,
                            raw_request: resp.raw_request,
                            raw_response: resp.raw_response,
                            thinking_text: resp.thinking_text,
                            thinking_duration_secs: resp.thinking_duration_secs,
                            thinking_signature: resp.thinking_signature,
                            continuation_request: None,
                        })
                    }
                }
            }
        }
    }

    async fn record_raw_attempt(
        &self,
        kind: &str,
        iteration: usize,
        attempt: u32,
        system_parts: &[&str],
        messages: &[crate::session::models::ChatMessage],
        api_tools: &[serde_json::Value],
        estimated_tokens: u32,
        completed: bool,
        response_or_error: &str,
        used_previous_response_id: Option<bool>,
    ) {
        let request = serde_json::json!({
            "_locusAttempt": {
                "kind": kind,
                "attempt": attempt,
                "completed": completed,
                "estimatedTokens": estimated_tokens,
                "usedPreviousResponseId": used_previous_response_id,
                "responseOrError": response_or_error,
            },
            "model": self.effective_model.clone(),
            "system": system_parts,
            "messages": messages,
            "tools": api_tools,
        });
        let round = RawRound {
            round: iteration,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            request,
            response: response_or_error.to_string(),
        };
        self.raw_store
            .lock()
            .await
            .entry(self.session_id.clone())
            .or_insert_with(Vec::new)
            .push(round);
    }

    async fn call_compact_llm(
        &self,
        store: &SessionStore,
        system_parts: &[&str],
        messages: &[crate::session::models::ChatMessage],
    ) -> Result<LlmCallResult, String> {
        if let LlmBackend::OpenAiCodex {
            auth,
            transport,
            base_url,
        } = &self.backend
        {
            let system_prompt = system_parts.join("\n\n");
            let model_name = &self.effective_model;
            let actual_model = model_name.strip_prefix("openai/").unwrap_or(model_name);
            let mut compact_turn_state = codex::TurnState::default();
            let (access_token, account_id) = resolve_codex_request_auth(auth, false)
                .await
                .map_err(|e| format!("OpenAI Codex token failed (please re-login): {}", e))?;
            let resp = match codex::stream_chat_with_options(
                &access_token,
                account_id.as_deref(),
                *transport,
                base_url.as_deref(),
                actual_model,
                &system_prompt,
                messages,
                &[],
                self.effort.as_deref(),
                self.debug,
                None,
                None,
                &mut compact_turn_state,
                codex::CodexStreamOptions::compact(),
                &|_| {},
                &|_| {},
                &|_, _| {},
            )
            .await
            {
                Ok(resp) => resp,
                Err(error) if is_codex_unauthorized_error(&error) => {
                    eprintln!(
                        "[OpenAI Codex] compact received unauthorized response, refreshing auth and retrying once"
                    );
                    let (access_token, account_id) =
                        resolve_codex_request_auth(auth, true)
                            .await
                            .map_err(|e| format!("OpenAI Codex token refresh failed: {}", e))?;
                    codex::stream_chat_with_options(
                        &access_token,
                        account_id.as_deref(),
                        *transport,
                        base_url.as_deref(),
                        actual_model,
                        &system_prompt,
                        messages,
                        &[],
                        self.effort.as_deref(),
                        self.debug,
                        None,
                        None,
                        &mut compact_turn_state,
                        codex::CodexStreamOptions::compact(),
                        &|_| {},
                        &|_| {},
                        &|_, _| {},
                    )
                    .await?
                }
                Err(error) => return Err(error),
            };
            return Ok(LlmCallResult {
                text: resp.text,
                tool_calls: resp.tool_calls,
                finish_reason: resp.finish_reason,
                response_id: resp.response_id,
                input_tokens: resp.input_tokens,
                output_tokens: resp.output_tokens,
                cache_read_tokens: resp.cache_read_tokens,
                cache_write_tokens: resp.cache_write_tokens,
                cost_usd: 0.0,
                raw_request: resp.raw_request,
                raw_response: resp.raw_response,
                thinking_text: resp.thinking_text,
                thinking_duration_secs: resp.thinking_duration_secs,
                thinking_signature: String::new(),
                continuation_request: resp.continuation_request,
            });
        }

        self.call_llm(
            store,
            None,
            system_parts,
            messages,
            &[],
            |_| {},
            |_| {},
            |_, _| {},
        )
        .await
    }

    async fn estimate_current_context_tokens(
        &self,
        store: &SessionStore,
        system_parts: &[&str],
    ) -> Result<u32, String> {
        let messages = store.get_messages_for_prompt(&self.session_id)?;
        let prepared_messages = compact::prepare_messages_for_llm(&messages);
        self.seed_loaded_tools_from_history(&messages).await;
        let request_tools = self.build_request_tool_names().await;
        let api_tools = self.build_api_tools(&request_tools).await;
        Ok(compact::estimate_request_tokens(
            system_parts,
            &prepared_messages,
            &api_tools,
        ))
    }

    async fn persist_compacted_context_usage(
        &self,
        store: &SessionStore,
        system_parts: &[&str],
        context_limit: u32,
    ) -> u32 {
        let context_tokens = match self
            .estimate_current_context_tokens(store, system_parts)
            .await
        {
            Ok(tokens) => tokens,
            Err(error) => {
                eprintln!(
                    "[Agent {}] failed to estimate compacted context usage: {}",
                    self.id, error
                );
                return 0;
            }
        };

        if context_tokens > 0 && context_limit > 0 {
            match store.record_token_usage(
                &self.session_id,
                0,
                0,
                0,
                0,
                0.0,
                0,
                Some(context_tokens),
                Some(context_limit),
            ) {
                Ok(_) => {
                    eprintln!(
                        "[Agent {}] compacted context usage persisted: {}/{}",
                        self.id, context_tokens, context_limit
                    );
                }
                Err(error) => {
                    eprintln!(
                        "[Agent {}] failed to persist compacted context usage: {}",
                        self.id, error
                    );
                }
            }
        }

        context_tokens
    }

    async fn execute_auto_compact(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        system_parts: &[&str],
        context_tokens: u32,
        context_limit: u32,
        force_compact: bool,
        run_id: &str,
        attempt_kind: &str,
        iteration: usize,
    ) -> Result<bool, String> {
        let messages = store.get_messages_for_prompt(&self.session_id)?;
        if messages.len() < 2 {
            return Ok(false);
        }

        let messages_before = messages.len() as u32;
        let compact_label = if force_compact {
            "manual-compact"
        } else {
            "auto-compact"
        };

        eprintln!(
            "[Agent {}] {} requested: context_tokens={}, limit={}, messages={}",
            self.id,
            compact_label,
            context_tokens,
            context_limit,
            messages.len()
        );

        let mut compact_plan = match compact::build_compact_request_with_budget(
            &messages,
            system_parts,
            context_limit,
        ) {
            Ok(plan) => plan,
            Err(e) => {
                eprintln!(
                    "[Agent {}] budgeted compact request unavailable, using emergency compact: {}",
                    self.id, e
                );
                let mut boundary_idx = compact::find_compact_boundary_by_budget(
                    &messages,
                    compact::compact_recent_tail_token_budget(context_limit),
                );
                if force_compact
                    && !compact::has_compactable_messages_before_boundary(&messages, boundary_idx)
                {
                    boundary_idx = messages.len().saturating_sub(1);
                    while boundary_idx > 0 && messages[boundary_idx].role == MessageRole::Tool {
                        boundary_idx -= 1;
                    }
                }
                if !compact::has_compactable_messages_before_boundary(&messages, boundary_idx) {
                    eprintln!(
                        "[Agent {}] emergency {} skipped: no compactable messages before boundary {}",
                        self.id, compact_label, boundary_idx
                    );
                    return Ok(false);
                }
                emit_stream(
                    app_handle,
                    run_id,
                    StreamEvent::CompactStart {
                        session_id: self.session_id.clone(),
                        context_tokens,
                        context_limit,
                    },
                );
                let summary = compact::build_emergency_compact_summary(&messages, boundary_idx, &e);
                let keep_from_msg = &messages[boundary_idx];
                let restored_files_section = compact::build_post_compact_restored_files_section(
                    &messages,
                    &self.working_dir,
                );
                let summary_msg = compact::build_post_compact_message(
                    &summary,
                    &restored_files_section,
                    keep_from_msg.created_at,
                    boundary_idx + 1 < messages.len(),
                );
                let (count_before, count_after) =
                    store.compact_messages(&self.session_id, &summary_msg, &keep_from_msg.id)?;
                if matches!(self.backend, LlmBackend::OpenAiCodex { .. }) {
                    crate::llm::codex::reset_cached_session_window(&self.session_id).await;
                }
                let compacted_context_tokens = self
                    .persist_compacted_context_usage(store, system_parts, context_limit)
                    .await;
                let compacted_messages = store.get_messages(&self.session_id)?;
                eprintln!(
                    "[Agent {}] emergency {} done: {} → {} messages, summary len={}",
                    self.id,
                    compact_label,
                    count_before,
                    count_after,
                    summary.len()
                );
                emit_stream(
                    app_handle,
                    run_id,
                    StreamEvent::CompactDone {
                        session_id: self.session_id.clone(),
                        messages_before,
                        messages_after: count_after,
                        context_tokens: compacted_context_tokens,
                        context_limit,
                        messages: compacted_messages,
                    },
                );
                return Ok(true);
            }
        };

        if force_compact
            && !compact::has_compactable_messages_before_boundary(
                &messages,
                compact_plan.boundary_idx,
            )
        {
            compact_plan.boundary_idx = messages.len().saturating_sub(1);
            while compact_plan.boundary_idx > 0
                && messages[compact_plan.boundary_idx].role == MessageRole::Tool
            {
                compact_plan.boundary_idx -= 1;
            }
        }

        eprintln!(
            "[Agent {}] compact request budget: estimated_tokens={}, budget={}, boundary_idx={}, truncated={}",
            self.id,
            compact_plan.estimated_tokens,
            compact_plan.budget_tokens,
            compact_plan.boundary_idx,
            compact_plan.truncated
        );

        if !compact::has_compactable_messages_before_boundary(&messages, compact_plan.boundary_idx)
        {
            eprintln!(
                "[Agent {}] {} skipped: no compactable messages before boundary {}",
                self.id, compact_label, compact_plan.boundary_idx
            );
            return Ok(false);
        }

        emit_stream(
            app_handle,
            run_id,
            StreamEvent::CompactStart {
                session_id: self.session_id.clone(),
                context_tokens,
                context_limit,
            },
        );

        let summary_result = self
            .call_compact_llm(store, system_parts, &compact_plan.messages)
            .await;
        match &summary_result {
            Ok(resp) => {
                self.record_raw_attempt(
                    attempt_kind,
                    iteration,
                    1,
                    system_parts,
                    &compact_plan.messages,
                    &[],
                    compact_plan.estimated_tokens,
                    true,
                    &resp.raw_response,
                    Some(false),
                )
                .await;
            }
            Err(e) => {
                self.record_raw_attempt(
                    attempt_kind,
                    iteration,
                    1,
                    system_parts,
                    &compact_plan.messages,
                    &[],
                    compact_plan.estimated_tokens,
                    false,
                    e,
                    Some(false),
                )
                .await;
            }
        }

        let summary_response = match summary_result {
            Ok(resp) => resp,
            Err(e) if is_recoverable_compact_llm_error(&e) => {
                eprintln!(
                    "[Agent {}] compact LLM call could not be safely sent, using emergency compact: {}",
                    self.id, e
                );
                let boundary_idx = compact_plan.boundary_idx;
                if !compact::has_compactable_messages_before_boundary(&messages, boundary_idx) {
                    eprintln!(
                        "[Agent {}] emergency auto-compact skipped after compact error: no compactable messages before boundary {}",
                        self.id, boundary_idx
                    );
                    return Ok(false);
                }
                let summary = compact::build_emergency_compact_summary(&messages, boundary_idx, &e);
                let keep_from_msg = &messages[boundary_idx];
                let restored_files_section = compact::build_post_compact_restored_files_section(
                    &messages,
                    &self.working_dir,
                );
                let summary_msg = compact::build_post_compact_message(
                    &summary,
                    &restored_files_section,
                    keep_from_msg.created_at,
                    boundary_idx + 1 < messages.len(),
                );
                let (count_before, count_after) =
                    store.compact_messages(&self.session_id, &summary_msg, &keep_from_msg.id)?;
                if matches!(self.backend, LlmBackend::OpenAiCodex { .. }) {
                    crate::llm::codex::reset_cached_session_window(&self.session_id).await;
                }
                let compacted_context_tokens = self
                    .persist_compacted_context_usage(store, system_parts, context_limit)
                    .await;
                let compacted_messages = store.get_messages(&self.session_id)?;
                eprintln!(
                    "[Agent {}] emergency auto-compact done after compact error: {} → {} messages, summary len={}",
                    self.id,
                    count_before,
                    count_after,
                    summary.len()
                );
                emit_stream(
                    app_handle,
                    run_id,
                    StreamEvent::CompactDone {
                        session_id: self.session_id.clone(),
                        messages_before,
                        messages_after: count_after,
                        context_tokens: compacted_context_tokens,
                        context_limit,
                        messages: compacted_messages,
                    },
                );
                return Ok(true);
            }
            Err(e) => {
                eprintln!("[Agent {}] compact LLM call failed: {}", self.id, e);
                return Err(e);
            }
        };

        if summary_response.input_tokens > 0
            || summary_response.output_tokens > 0
            || summary_response.cache_read_tokens > 0
            || summary_response.cache_write_tokens > 0
        {
            let priced_rounds = if matches!(&self.backend, LlmBackend::OpenRouter { .. }) {
                1
            } else {
                0
            };
            match store.record_token_usage(
                &self.session_id,
                summary_response.input_tokens as u64,
                summary_response.output_tokens as u64,
                summary_response.cache_read_tokens as u64,
                summary_response.cache_write_tokens as u64,
                summary_response.cost_usd,
                priced_rounds,
                None,
                None,
            ) {
                Ok(totals) => {
                    eprintln!(
                        "[Agent {}] compact tokens: +{}in/+{}out/+{}cache_r/+{}cache_w, cost=${:.6}, total: {}in/{}out/{}cache_r/{}cache_w/${:.6}",
                        self.id,
                        summary_response.input_tokens,
                        summary_response.output_tokens,
                        summary_response.cache_read_tokens,
                        summary_response.cache_write_tokens,
                        summary_response.cost_usd,
                        totals.total_input_tokens,
                        totals.total_output_tokens,
                        totals.total_cache_read_tokens,
                        totals.total_cache_write_tokens,
                        totals.total_cost_usd,
                    );
                    emit_stream(
                        app_handle,
                        run_id,
                        StreamEvent::UsageUpdate {
                            session_id: self.session_id.clone(),
                            input_tokens: summary_response.input_tokens,
                            output_tokens: summary_response.output_tokens,
                            cache_read_tokens: summary_response.cache_read_tokens,
                            cache_write_tokens: summary_response.cache_write_tokens,
                            total_input_tokens: totals.total_input_tokens,
                            total_output_tokens: totals.total_output_tokens,
                            total_cache_read_tokens: totals.total_cache_read_tokens,
                            total_cache_write_tokens: totals.total_cache_write_tokens,
                            total_cost_usd: totals.total_cost_usd,
                            priced_rounds: totals.priced_rounds,
                            // Compact is an internal summarization call; do not replace the
                            // visible live context estimate with the compact-request context.
                            context_tokens: 0,
                            context_limit,
                        },
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[Agent {}] failed to record compact token usage: {}",
                        self.id, e
                    );
                }
            }
        }

        let boundary_idx = compact_plan.boundary_idx;
        let mut summary = compact::extract_summary(&summary_response.text);
        if !compact::is_valid_compact_summary(&summary) {
            eprintln!(
                "[Agent {}] compact returned invalid summary, using emergency compact: summary_len={}",
                self.id,
                summary.len()
            );
            summary = compact::build_emergency_compact_summary(
                &messages,
                boundary_idx,
                "compact LLM returned an invalid summary",
            );
        }

        let keep_from_msg = &messages[boundary_idx];
        let restored_files_section =
            compact::build_post_compact_restored_files_section(&messages, &self.working_dir);
        let summary_msg = compact::build_post_compact_message(
            &summary,
            &restored_files_section,
            keep_from_msg.created_at,
            boundary_idx + 1 < messages.len(),
        );

        let (count_before, count_after) =
            store.compact_messages(&self.session_id, &summary_msg, &keep_from_msg.id)?;
        if matches!(self.backend, LlmBackend::OpenAiCodex { .. }) {
            crate::llm::codex::reset_cached_session_window(&self.session_id).await;
        }
        let compacted_context_tokens = self
            .persist_compacted_context_usage(store, system_parts, context_limit)
            .await;
        let compacted_messages = store.get_messages(&self.session_id)?;

        eprintln!(
            "[Agent {}] auto-compact done: {} → {} messages, summary len={}",
            self.id,
            count_before,
            count_after,
            summary.len()
        );

        emit_stream(
            app_handle,
            run_id,
            StreamEvent::CompactDone {
                session_id: self.session_id.clone(),
                messages_before,
                messages_after: count_after,
                context_tokens: compacted_context_tokens,
                context_limit,
                messages: compacted_messages,
            },
        );

        Ok(true)
    }

    fn wrap_system_reminder(content: &str) -> Option<String> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(format!(
            "<system-reminder>\n{}\n</system-reminder>\n",
            trimmed
        ))
    }

    fn build_selected_skill_reminder(
        &self,
        intent: &crate::session::models::UserIntentPayload,
    ) -> String {
        let mut blocks = Vec::new();
        let skills = crate::commands::list_skills_sync(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
        );
        let app_knowledge_dir = self.app_knowledge_dir.as_ref().as_ref();

        for skill in &intent.skills {
            let manifest = Self::find_selected_skill_manifest(&skills, skill);
            let (source, dir_name, rel_path) = if let Some(manifest) = manifest {
                (
                    manifest.source.as_str(),
                    manifest.dir_name.as_str(),
                    manifest.rel_path.clone(),
                )
            } else {
                (
                    skill.source.as_str(),
                    skill.dir_name.as_str(),
                    Self::resolve_selected_skill_reminder_path(&skills, app_knowledge_dir, skill),
                )
            };

            let content_result = crate::commands::read_skill_manifest_sync(
                &self.working_dir,
                app_knowledge_dir,
                dir_name,
                Some(source),
            );
            let escaped_name = skill.name.replace('\n', " ").trim().to_string();
            let escaped_source = source.replace('\n', " ").trim().to_string();
            let escaped_path = rel_path.replace('\n', " ").trim().to_string();

            match content_result {
                Ok(content) => blocks.push(format!(
                    "<selected-skill>\nName: {}\nSource: {}\nPath: {}\n\n{}\n</selected-skill>",
                    escaped_name,
                    escaped_source,
                    escaped_path,
                    content.trim()
                )),
                Err(error) => blocks.push(format!(
                    "<selected-skill-error>\nName: {}\nSource: {}\nPath: {}\nError: {}\n</selected-skill-error>",
                    escaped_name,
                    escaped_source,
                    escaped_path,
                    error.replace('\n', " ")
                )),
            }
        }

        if blocks.is_empty() {
            return String::new();
        }

        format!(
            "<system-reminder>\nThe user explicitly selected Skill workflows for this request. The injected Skill content is complete for the selected workflows. Use it directly.\n\n{}\n</system-reminder>",
            blocks.join("\n\n"),
        )
    }

    fn intent_skill_source_matches(manifest_source: &str, intent_source: &str) -> bool {
        manifest_source == intent_source
            || (manifest_source == "app" && matches!(intent_source, "builtin" | "builtIn"))
    }

    fn find_selected_skill_manifest<'a>(
        skills: &'a [crate::commands::SkillManifest],
        skill: &crate::session::models::UserIntentSkill,
    ) -> Option<&'a crate::commands::SkillManifest> {
        skills
            .iter()
            .find(|manifest| {
                Self::intent_skill_source_matches(&manifest.source, &skill.source)
                    && manifest.dir_name == skill.dir_name
            })
            .or_else(|| {
                let legacy_app_skill = Self::intent_skill_source_matches("app", &skill.source)
                    && !skill.dir_name.contains('/');
                if !legacy_app_skill {
                    return None;
                }
                let builtin_dir_name = format!("builtin/{}", skill.dir_name);
                skills.iter().find(|manifest| {
                    manifest.source == "app" && manifest.dir_name == builtin_dir_name
                })
            })
    }

    fn resolve_selected_skill_reminder_path(
        skills: &[crate::commands::SkillManifest],
        app_knowledge_dir: Option<&std::path::PathBuf>,
        skill: &crate::session::models::UserIntentSkill,
    ) -> String {
        if let Some(manifest) = Self::find_selected_skill_manifest(skills, skill) {
            return manifest.rel_path.clone();
        }

        let normalized_dir_name = skill.dir_name.trim().trim_matches('/').replace('\\', "/");
        if Self::intent_skill_source_matches("app", &skill.source)
            && !normalized_dir_name.is_empty()
            && !normalized_dir_name.contains('/')
        {
            if let Some(app_root) = app_knowledge_dir {
                let builtin_rel_path = format!("skill/builtin/{}.md", normalized_dir_name);
                if app_root.join(&builtin_rel_path).is_file() {
                    return builtin_rel_path;
                }
            }
        }

        format!("skill/{}.md", normalized_dir_name)
    }

    fn build_user_prompt_suffix(
        &self,
        mode: &str,
        user_intent: Option<&crate::session::models::UserIntentPayload>,
    ) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(intent) = user_intent {
            let skill_reminder = self.build_selected_skill_reminder(intent);
            if !skill_reminder.is_empty() {
                parts.push(skill_reminder);
            }
        }
        if mode == "plan" {
            parts.push(crate::prompt::plan::PLAN_REMINDER.to_string());
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!("\n\n{}", parts.join("\n\n")))
        }
    }

    fn pending_input_mode(input: &PendingSessionInput) -> String {
        input
            .mode
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                input
                    .user_intent
                    .as_ref()
                    .map(|intent| intent.mode.as_str())
            })
            .unwrap_or("build")
            .to_string()
    }

    fn persisted_message_by_id(
        &self,
        store: &SessionStore,
        message_id: &str,
    ) -> Result<ChatMessage, String> {
        store
            .get_messages(&self.session_id)?
            .into_iter()
            .find(|message| message.id == message_id)
            .ok_or_else(|| {
                format!(
                    "Persisted message not found: session={} message={}",
                    self.session_id, message_id
                )
            })
    }

    fn persist_claimed_pending_inputs(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        run_id: &str,
        env_prompt_prefix: Option<&str>,
        pending_inputs: Vec<PendingSessionInput>,
    ) -> Result<bool, String> {
        if pending_inputs.is_empty() {
            return Ok(false);
        }

        let claimed_inputs = pending_inputs.clone();
        let result = (|| -> Result<(), String> {
            for input in pending_inputs {
                let user_intent_signature = input
                    .user_intent
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|e| format!("Failed to serialize pending user intent: {}", e))?;
                let effective_mode = Self::pending_input_mode(&input);
                let user_prompt_suffix =
                    self.build_user_prompt_suffix(&effective_mode, input.user_intent.as_ref());
                let first_user_message_id = store.first_user_message_id(&self.session_id)?;
                let current_prompt_prefix = if first_user_message_id.is_none() {
                    env_prompt_prefix
                } else {
                    None
                };
                let message_id = store.add_message_with_images_asset_refs_and_signature(
                    &self.session_id,
                    MessageRole::User,
                    &input.text,
                    input.images.as_deref(),
                    input.asset_refs.as_deref(),
                    user_intent_signature.as_deref(),
                    current_prompt_prefix,
                    user_prompt_suffix.as_deref(),
                )?;
                if let Some(first_user_message_id) = first_user_message_id.as_deref() {
                    store.update_message_prompt_prefix(
                        &self.session_id,
                        first_user_message_id,
                        env_prompt_prefix,
                    )?;
                }
                let current_user_message = self.persisted_message_by_id(store, &message_id)?;
                emit_stream(
                    app_handle,
                    run_id,
                    StreamEvent::UserMessage {
                        session_id: self.session_id.clone(),
                        message: current_user_message,
                    },
                );
                emit_stream(
                    app_handle,
                    run_id,
                    StreamEvent::PendingInputAccepted {
                        session_id: self.session_id.clone(),
                        pending_input_id: input.id,
                        message_id,
                    },
                );
            }
            Ok(())
        })();

        if let Err(error) = result {
            let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> = app_handle.state();
            match queue_state.lock() {
                Ok(mut queue) => queue.restore_claimed(claimed_inputs),
                Err(restore_error) => {
                    eprintln!(
                        "[Agent {}] failed to restore claimed pending inputs after error: {}",
                        self.id, restore_error
                    );
                }
            }
            return Err(error);
        }

        Ok(true)
    }

    fn drain_queued_pending_inputs(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        run_id: &str,
        env_prompt_prefix: Option<&str>,
    ) -> Result<bool, String> {
        let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> = app_handle.state();
        let pending_inputs = {
            let mut queue = queue_state
                .lock()
                .map_err(|e| format!("Failed to lock pending input queue: {}", e))?;
            queue.claim_immediate(&self.session_id, run_id)
        };
        self.persist_claimed_pending_inputs(
            app_handle,
            store,
            run_id,
            env_prompt_prefix,
            pending_inputs,
        )
    }

    fn new_run_id(&self) -> String {
        format!(
            "{}_{}",
            self.session_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis().to_string())
                .unwrap_or_else(|_| "0".to_string())
        )
    }

    pub(super) fn is_cancel_requested(&self) -> bool {
        *self.cancel_rx.borrow()
    }

    pub(super) fn run_is_current_for_session(
        &self,
        store: &SessionStore,
        run_id: &str,
        stage: &str,
        tool_call_id: Option<&str>,
    ) -> bool {
        match store.active_run_for_session(&self.session_id) {
            Ok(Some(active_run)) if active_run.run_id == run_id => true,
            Ok(active_run) => {
                let active_run_id = active_run
                    .as_ref()
                    .map(|run| run.run_id.as_str())
                    .unwrap_or("none");
                let active_status = active_run
                    .as_ref()
                    .map(|run| run.status.as_str())
                    .unwrap_or("none");
                eprintln!(
                    "[Agent {}] discarding stale tool result: session={} run={} active_run={} active_status={} stage={} tool_call_id={}",
                    self.id,
                    self.session_id,
                    run_id,
                    active_run_id,
                    active_status,
                    stage,
                    tool_call_id.unwrap_or("")
                );
                false
            }
            Err(error) => {
                eprintln!(
                    "[Agent {}] failed to validate active run before accepting tool result; discarding result: session={} run={} stage={} tool_call_id={} error={}",
                    self.id,
                    self.session_id,
                    run_id,
                    stage,
                    tool_call_id.unwrap_or(""),
                    error
                );
                false
            }
        }
    }

    fn cancel_waiter(&self) -> tokio::sync::watch::Receiver<bool> {
        self.cancel_rx.clone()
    }

    fn interrupted_tool_result() -> ExecutedToolResult {
        ExecutedToolResult {
            output: crate::session::history::INTERRUPTED_TOOL_RESULT.to_string(),
            is_error: false,
            outcome: ToolRunOutcome::Interrupted,
            nested_tool_calls: None,
            images: None,
        }
    }

    pub fn persist_interrupted_assistant_snapshot(
        store: &SessionStore,
        session_id: &str,
        snapshot: &AssistantStreamSnapshot,
    ) -> Option<InterruptedAssistantMessage> {
        if snapshot.text.is_empty() && snapshot.thinking_content.is_empty() {
            return None;
        }

        let thinking_content =
            (!snapshot.thinking_content.is_empty()).then(|| snapshot.thinking_content.clone());
        if let Some(message_id) = snapshot.persisted_message_id.as_ref() {
            return Some(InterruptedAssistantMessage {
                message_id: message_id.clone(),
                full_text: snapshot.text.clone(),
                thinking_content,
                thinking_duration: snapshot.thinking_duration,
            });
        }

        match store.add_message_with_thinking(
            session_id,
            MessageRole::Assistant,
            &snapshot.text,
            thinking_content.as_deref(),
            snapshot.thinking_duration,
            None,
            None,
            None,
        ) {
            Ok(message_id) => Some(InterruptedAssistantMessage {
                message_id,
                full_text: snapshot.text.clone(),
                thinking_content,
                thinking_duration: snapshot.thinking_duration,
            }),
            Err(error) => {
                eprintln!(
                    "[Locus] failed to persist interrupted assistant message for session {}: {}",
                    session_id, error
                );
                None
            }
        }
    }

    fn emit_cancelled(&self, app_handle: &AppHandle, store: &SessionStore, run_id: &str) {
        let interrupted = Self::persist_interrupted_assistant_snapshot(
            store,
            &self.session_id,
            &self.partial_assistant.snapshot(),
        );
        self.partial_assistant.reset();
        eprintln!(
            "[Agent {}] emitting Cancelled for session {} run {}",
            self.id, self.session_id, run_id
        );
        emit_stream(
            app_handle,
            run_id,
            StreamEvent::Cancelled {
                session_id: self.session_id.clone(),
                message_id: interrupted
                    .as_ref()
                    .map(|message| message.message_id.clone()),
                full_text: interrupted
                    .as_ref()
                    .map(|message| message.full_text.clone()),
                thinking_content: interrupted
                    .as_ref()
                    .and_then(|message| message.thinking_content.clone()),
                thinking_duration: interrupted.and_then(|message| message.thinking_duration),
                render_parts: None,
            },
        );
    }

    pub fn run<'a>(
        &'a self,
        app_handle: &'a AppHandle,
        store: &'a SessionStore,
        user_text: &'a str,
        images: Option<&'a [crate::session::models::ImageData]>,
        asset_refs: Option<&'a [crate::session::models::AssetRefData]>,
        initial_mode: &'a str,
        user_intent: Option<crate::session::models::UserIntentPayload>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let run_id = self.new_run_id();
            store.try_start_run(&self.session_id, &run_id)?;
            self.run_with_run_id(
                app_handle,
                store,
                user_text,
                images,
                asset_refs,
                initial_mode,
                user_intent,
                run_id,
                None,
            )
            .await
        })
    }

    pub fn run_with_run_id<'a>(
        &'a self,
        app_handle: &'a AppHandle,
        store: &'a SessionStore,
        user_text: &'a str,
        images: Option<&'a [crate::session::models::ImageData]>,
        asset_refs: Option<&'a [crate::session::models::AssetRefData]>,
        initial_mode: &'a str,
        user_intent: Option<crate::session::models::UserIntentPayload>,
        run_id: String,
        accepted_pending_input_id: Option<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let run_started_at = Instant::now();
            self.partial_assistant.reset();
            eprintln!(
                "[Agent {}] run pipeline start: session={} run={} mode={} images={} asset_refs={} has_user_intent={}",
                self.id,
                self.session_id,
                run_id,
                initial_mode,
                images.map(|items| items.len()).unwrap_or(0),
                asset_refs.map(|items| items.len()).unwrap_or(0),
                user_intent.is_some()
            );
            let clear_started_at = Instant::now();
            self.clear_pending_knowledge_proposal(app_handle).await;
            log_stage_elapsed(
                &self.id,
                &self.session_id,
                &run_id,
                "clearPendingKnowledgeProposal",
                clear_started_at,
            );
            let run_result: Result<String, String> = async {
        // Notify frontend of the new run_id
        eprintln!(
            "[Agent {}] emitting RunStart for session {} run {}",
            self.id, self.session_id, run_id
        );
        emit_stream(app_handle, &run_id, StreamEvent::RunStart {
            session_id: self.session_id.clone(),
        });

        if initial_mode == "compact" {
            let prompt_parts = self.build_system_prompt_parts().await;
            let system_parts: Vec<&str> = {
                let mut parts = vec![prompt_parts.base_prompt.as_str()];
                if !prompt_parts.rules_prompt.is_empty() {
                    parts.push(prompt_parts.rules_prompt.as_str());
                }
                if !prompt_parts.knowledge_prompt.is_empty() {
                    parts.push(prompt_parts.knowledge_prompt.as_str());
                }
                parts
            };
            let ctx_limit = if let LlmBackend::Custom { context_length, .. } = &self.backend {
                *context_length
            } else {
                model_context_limit(&self.effective_model)
            };
            let messages = store.get_messages_for_prompt(&self.session_id)?;
            let prepared_messages = compact::prepare_messages_for_llm(&messages);
            self.seed_loaded_tools_from_history(&messages).await;
            let request_tools = self.build_request_tool_names().await;
            let api_tools = self.build_api_tools(&request_tools).await;
            let estimated_input_tokens =
                compact::estimate_request_tokens(&system_parts, &prepared_messages, &api_tools);
            let compacted = self
                .execute_auto_compact(
                    app_handle,
                    store,
                    &system_parts,
                    estimated_input_tokens,
                    ctx_limit,
                    true,
                    &run_id,
                    "compact",
                    1,
                )
                .await?;
            if !compacted {
                eprintln!(
                    "[Agent {}] manual compact finished without changes: session={} run={} messages={}",
                    self.id,
                    self.session_id,
                    run_id,
                    messages.len()
                );
            }
            if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(&run_id)) {
                eprintln!(
                    "[Agent {}] failed to persist latest completed run id for manual compact {} run {}: {}",
                    self.id, self.session_id, run_id, error
                );
            }
            emit_stream(
                app_handle,
                &run_id,
                StreamEvent::Done {
                    session_id: self.session_id.clone(),
                    message_id: String::new(),
                    full_text: String::new(),
                    content_order: None,
                    thinking_order: None,
                    render_parts: None,
                },
            );
            return Ok(String::new());
        }

        let user_text_started_at = Instant::now();
        let actual_user_text: String;
        if crate::unity_bridge::is_unity_project(&self.working_dir) {
            let (_connected, status, active_scene) =
                crate::unity_bridge::query_unity_status(&self.working_dir).await;
            let current_state = (status.to_string(), active_scene.clone());

            let mut state_map = session_unity_state().lock().await;
            let prev = state_map.get(&self.session_id);

            if let Some((prev_status, prev_scene)) = prev {
                if prev_status != &current_state.0 || prev_scene != &current_state.1 {
                    let status_text = crate::unity_bridge::format_editor_status_for_event(status);
                    let scene_info = active_scene
                        .as_deref()
                        .map(|s| format!(", Active Scene: {}", s))
                        .unwrap_or_default();
                    actual_user_text = format!(
                        "[Unity Editor Status Changed] Unity Editor Status: {}{}\n\n{}",
                        status_text, scene_info, user_text
                    );
                    eprintln!(
                        "[Agent {}] Unity state changed: {:?} -> {:?}",
                        self.id, (prev_status, prev_scene), &current_state
                    );
                } else {
                    actual_user_text = user_text.to_string();
                }
            } else {
                actual_user_text = user_text.to_string();
            }

            state_map.insert(self.session_id.clone(), current_state);
        } else {
            actual_user_text = user_text.to_string();
        }
        eprintln!(
            "[Agent {}] user text prepared: session={} run={} elapsed_ms={} original_chars={} actual_chars={}",
            self.id,
            self.session_id,
            run_id,
            user_text_started_at.elapsed().as_millis(),
            user_text.len(),
            actual_user_text.len()
        );

        let prompt_parts_started_at = Instant::now();
        let user_intent_signature = user_intent
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("Failed to serialize user intent: {}", e))?;
        let prompt_parts = self.build_system_prompt_parts().await;
        eprintln!(
            "[Agent {}] prompt parts ready: session={} run={} elapsed_ms={} base_chars={} env_chars={} rules_chars={} knowledge_chars={}",
            self.id,
            self.session_id,
            run_id,
            prompt_parts_started_at.elapsed().as_millis(),
            prompt_parts.base_prompt.len(),
            prompt_parts.env_prompt.len(),
            prompt_parts.rules_prompt.len(),
            prompt_parts.knowledge_prompt.len()
        );
        let persist_user_message_started_at = Instant::now();
        let env_prompt_prefix = Self::wrap_system_reminder(&prompt_parts.env_prompt);
        let user_prompt_suffix = self.build_user_prompt_suffix(initial_mode, user_intent.as_ref());
        let first_user_message_id = store.first_user_message_id(&self.session_id)?;
        let current_prompt_prefix = if first_user_message_id.is_none() {
            env_prompt_prefix.as_deref()
        } else {
            None
        };
        let current_message_id = store.add_message_with_images_asset_refs_and_signature(
            &self.session_id,
            MessageRole::User,
            &actual_user_text,
            images,
            asset_refs,
            user_intent_signature.as_deref(),
            current_prompt_prefix,
            user_prompt_suffix.as_deref(),
        )?;
        if let Some(first_user_message_id) = first_user_message_id.as_deref() {
            store.update_message_prompt_prefix(
                &self.session_id,
                first_user_message_id,
                env_prompt_prefix.as_deref(),
            )?;
        }
        let current_user_message = store
            .get_messages(&self.session_id)?
            .into_iter()
            .find(|message| message.id == current_message_id)
            .ok_or_else(|| {
                format!(
                    "Persisted user message not found: session={} message={}",
                    self.session_id, current_message_id
                )
            })?;
        emit_stream(app_handle, &run_id, StreamEvent::UserMessage {
            session_id: self.session_id.clone(),
            message: current_user_message,
        });
        if let Some(pending_input_id) = accepted_pending_input_id.as_deref() {
            emit_stream(app_handle, &run_id, StreamEvent::PendingInputAccepted {
                session_id: self.session_id.clone(),
                pending_input_id: pending_input_id.to_string(),
                message_id: current_message_id.clone(),
            });
        }
        eprintln!(
            "[Agent {}] user message persisted: session={} run={} elapsed_ms={} prefix_chars={} suffix_chars={} updated_first_message_prefix={}",
            self.id,
            self.session_id,
            run_id,
            persist_user_message_started_at.elapsed().as_millis(),
            current_prompt_prefix.map(str::len).unwrap_or(0),
            user_prompt_suffix.as_deref().map(str::len).unwrap_or(0),
            first_user_message_id.is_some()
        );

        let prompt_messages_after_user = store.get_messages_for_prompt(&self.session_id)?;
        self.seed_loaded_tools_from_history(&prompt_messages_after_user).await;

        if matches!(&self.backend, LlmBackend::AnthropicAgentSdk) {
            let prompt_text = crate::session::history::render_prompt_content(
                &actual_user_text,
                current_prompt_prefix,
                user_prompt_suffix.as_deref(),
            );
            let system_prompt = {
                let mut parts = vec![prompt_parts.base_prompt.as_str()];
                if !prompt_parts.rules_prompt.is_empty() {
                    parts.push(prompt_parts.rules_prompt.as_str());
                }
                if !prompt_parts.knowledge_prompt.is_empty() {
                    parts.push(prompt_parts.knowledge_prompt.as_str());
                }
                parts.join("\n\n")
            };
            return self
                .run_anthropic_agent_sdk(
                    app_handle,
                    store,
                    &prompt_text,
                    &system_prompt,
                    images,
                    initial_mode,
                    &run_id,
                )
                .await;
        }

        // Filter tools based on gating config
        let api_tools_started_at = Instant::now();
        let request_tools = self.build_request_tool_names().await;
        let api_tools = self.build_api_tools(&request_tools).await;
        eprintln!(
            "[Agent {}] api tools ready: session={} run={} elapsed_ms={} native_lazy={} request_tools={} api_tools={}",
            self.id,
            self.session_id,
            run_id,
            api_tools_started_at.elapsed().as_millis(),
            self.supports_native_tool_lazy_loading(),
            request_tools.len(),
            api_tools.len()
        );

        let backend_name = match &self.backend {
            LlmBackend::OpenRouter { .. } => "OpenRouter",
            LlmBackend::Anthropic { .. } => "Anthropic",
            LlmBackend::AnthropicAgentSdk => "Anthropic Agent SDK",
            LlmBackend::OpenAiCodex { .. } => "OpenAI Codex",
            LlmBackend::Custom { .. } => "Custom",
        };
        eprintln!(
            "[Agent {}] starting loop, backend={}, model={}, tools={}, mode={}, cwd={}",
            self.id,
            backend_name,
            self.effective_model,
            self.def.tools.join(","),
            initial_mode,
            self.working_dir
        );
        log_stage_elapsed(
            &self.id,
            &self.session_id,
            &run_id,
            "enterAgentLoop",
            run_started_at,
        );

        let mode = initial_mode.to_string();

        // 3. Agent Loop
        let mut iteration = 0;
        let mut compact_tracker = compact::CompactTracker::new();
        let final_text;
        let final_thinking_text;
        let final_thinking_duration: u32;
        let final_thinking_signature;
        let final_response_id;
        let final_continuation_request;
        let final_content_order;
        let final_thinking_order;
        let mut done_already_emitted = false;
        let mut terminal_done_message_id: Option<String> = None;
        let mut codex_turn_state = matches!(self.backend, LlmBackend::OpenAiCodex { .. })
            .then(codex::TurnState::default);
        let render_order_tracker = Arc::new(Mutex::new(StreamRenderOrderTracker::default()));

        'agent_loop: loop {
            iteration += 1;
            if iteration > MAX_TOOL_ITERATIONS {
                return Err(format!(
                    "Agent loop exceeded max iterations ({})",
                    MAX_TOOL_ITERATIONS
                ));
            }

            if self.is_cancel_requested() {
                self.clear_pending_knowledge_proposal(app_handle).await;
                self.emit_cancelled(app_handle, store, &run_id);
                return Ok(String::new());
            }

            let messages = store.get_messages_for_prompt(&self.session_id)?;

            let session_id = self.session_id.clone();
            let handle = app_handle.clone();
            let parent_tc = self.parent_tool_call.clone();
            let system_parts: Vec<&str> = {
                let mut parts = vec![prompt_parts.base_prompt.as_str()];
                if !prompt_parts.rules_prompt.is_empty() {
                    parts.push(prompt_parts.rules_prompt.as_str());
                }
                if !prompt_parts.knowledge_prompt.is_empty() {
                    parts.push(prompt_parts.knowledge_prompt.as_str());
                }
                parts
            };
            let ctx_limit = if let LlmBackend::Custom { context_length, .. } = &self.backend {
                *context_length
            } else {
                model_context_limit(&self.effective_model)
            };
            let prepared_messages = compact::prepare_messages_for_llm(&messages);
            let request_tools = self.build_request_tool_names().await;
            let api_tools = self.build_api_tools(&request_tools).await;
            let estimated_input_tokens =
                compact::estimate_request_tokens(&system_parts, &prepared_messages, &api_tools);
            let is_codex_backend = matches!(self.backend, LlmBackend::OpenAiCodex { .. });
            let should_preflight_compact = if is_codex_backend {
                compact::should_codex_auto_compact(estimated_input_tokens, ctx_limit)
            } else {
                compact::should_auto_compact(estimated_input_tokens, ctx_limit)
            };
            let mut preflight_compact_error: Option<String> = None;

            if !compact_tracker.is_circuit_broken()
                && should_preflight_compact
            {
                eprintln!(
                    "[Agent {}] preflight auto-compact candidate: estimated_tokens={}, limit={}, messages={} -> {}",
                    self.id,
                    estimated_input_tokens,
                    ctx_limit,
                    messages.len(),
                    prepared_messages.len()
                );
                match self
                    .execute_auto_compact(
                        app_handle,
                        store,
                        &system_parts,
                        estimated_input_tokens,
                        ctx_limit,
                        false,
                        &run_id,
                        "compact",
                        iteration,
                    )
                    .await
                {
                    Ok(true) => {
                        compact_tracker.record_success();
                        eprintln!("[Agent {}] preflight auto-compact succeeded", self.id);
                        continue 'agent_loop;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        compact_tracker.record_failure();
                        eprintln!("[Agent {}] preflight auto-compact failed: {}", self.id, e);
                        preflight_compact_error = Some(e);
                    }
                }
            }

            if is_codex_backend
                && compact::should_codex_block_normal_send(estimated_input_tokens, ctx_limit)
            {
                let reason = preflight_compact_error
                    .unwrap_or_else(|| "Codex request is too close to the context limit".to_string());
                return Err(format!(
                    "Refusing to send oversized Codex request after compact failed or was unavailable: estimated_input_tokens={}, limit={}, reason={}",
                    estimated_input_tokens, ctx_limit, reason
                ));
            }

            eprintln!(
                "[Agent {}] iteration {}, messages={}, prepared_messages={}, estimated_input_tokens={}",
                self.id,
                iteration,
                messages.len(),
                prepared_messages.len(),
                estimated_input_tokens
            );

            const LLM_RETRIES: u32 = 2;
            let mut response = None;
            let mut response_text_part: Option<RenderPartMark> = None;
            let mut response_thinking_part: Option<RenderPartMark> = None;
            let mut last_llm_error = String::new();
            let mut needs_reactive_compact = false;

            for llm_attempt in 0..=LLM_RETRIES {
                let attempt_number = llm_attempt + 1;
                let llm_call_started_at = Instant::now();
                eprintln!(
                    "[Agent {}] LLM attempt start: session={} run={} iteration={} attempt={}/{} backend={} prepared_messages={} api_tools={} estimated_input_tokens={}",
                    self.id,
                    self.session_id,
                    run_id,
                    iteration,
                    attempt_number,
                    LLM_RETRIES + 1,
                    backend_name,
                    prepared_messages.len(),
                    api_tools.len(),
                    estimated_input_tokens
                );
                let sid = session_id.clone();
                let hdl = handle.clone();
                let ptc = parent_tc.clone();
                let rid = run_id.clone();
                let render_order_for_text = render_order_tracker.clone();
                let text_block_id = format!("iteration:{}:attempt:{}:text", iteration, attempt_number);
                let partial_for_text = self.partial_assistant.clone();
                let agent_id_for_text = self.id.clone();
                let first_text_delta_logged = Arc::new(AtomicBool::new(false));
                let first_text_delta_logged_for_cb = first_text_delta_logged.clone();
                let attempt_emitted_output = Arc::new(AtomicBool::new(false));
                let emitted_output_for_text = attempt_emitted_output.clone();

                let sid2 = session_id.clone();
                let hdl2 = handle.clone();
                let rid2 = run_id.clone();
                let render_order_for_thinking = render_order_tracker.clone();
                let thinking_block_id =
                    format!("iteration:{}:attempt:{}:thinking", iteration, attempt_number);
                let partial_for_thinking = self.partial_assistant.clone();
                let agent_id_for_thinking = self.id.clone();
                let first_thinking_delta_logged = Arc::new(AtomicBool::new(false));
                let first_thinking_delta_logged_for_cb = first_thinking_delta_logged.clone();
                let emitted_output_for_thinking = attempt_emitted_output.clone();

                let sid3 = session_id.clone();
                let hdl3 = handle.clone();
                let ptc3 = parent_tc.clone();
                let rid3 = run_id.clone();
                let render_order_for_tool = render_order_tracker.clone();
                let agent_id_for_tool_start = self.id.clone();
                let first_tool_call_logged = Arc::new(AtomicBool::new(false));
                let first_tool_call_logged_for_cb = first_tool_call_logged.clone();
                let emitted_output_for_tool = attempt_emitted_output.clone();
                let tool_registry_for_tool_start = self.tool_registry.clone();

                let mut cancel_rx = self.cancel_waiter();
                let result = tokio::select! {
                    result = self.call_llm(
                        store,
                        codex_turn_state.as_mut(),
                        &system_parts,
                        &prepared_messages,
                        &api_tools,
                        move |delta| {
                            emitted_output_for_text.store(true, Ordering::Relaxed);
                            let mark = render_order_for_text
                                .lock()
                                .map(|mut tracker| tracker.mark_text(&rid, &text_block_id))
                                .unwrap_or(RenderPartMark {
                                    id: format!("{}:text:{}", rid, text_block_id),
                                    seq: 1,
                                });
                            if !first_text_delta_logged_for_cb.swap(true, Ordering::Relaxed) {
                                eprintln!(
                                    "[Agent {}] first text delta: session={} run={} iteration={} attempt={}/{} elapsed_ms={} delta_len={}",
                                    agent_id_for_text,
                                    sid,
                                    rid,
                                    iteration,
                                    attempt_number,
                                    LLM_RETRIES + 1,
                                    llm_call_started_at.elapsed().as_millis(),
                                    delta.len()
                                );
                            }
                            emit_stream(&hdl, &rid, StreamEvent::TextDelta {
                                session_id: sid.clone(),
                                text: delta.clone(),
                                order: Some(mark.seq),
                                part_id: Some(mark.id.clone()),
                                render_seq: Some(mark.seq),
                            });
                            partial_for_text.append_text(&delta);
                            if let Some(ref parent) = ptc {
                                emit_parent_stream(&hdl, parent.tool_call_delta(delta));
                            }
                        },
                        move |thinking| {
                            emitted_output_for_thinking.store(true, Ordering::Relaxed);
                            let mark = render_order_for_thinking
                                .lock()
                                .map(|mut tracker| {
                                    tracker.mark_thinking(&rid2, &thinking_block_id)
                                })
                                .unwrap_or(RenderPartMark {
                                    id: format!("{}:thinking:{}", rid2, thinking_block_id),
                                    seq: 1,
                                });
                            if !first_thinking_delta_logged_for_cb.swap(true, Ordering::Relaxed) {
                                eprintln!(
                                    "[Agent {}] first thinking delta: session={} run={} iteration={} attempt={}/{} elapsed_ms={} delta_len={}",
                                    agent_id_for_thinking,
                                    sid2,
                                    rid2,
                                    iteration,
                                    attempt_number,
                                    LLM_RETRIES + 1,
                                    llm_call_started_at.elapsed().as_millis(),
                                    thinking.len()
                                );
                            }
                            emit_stream(&hdl2, &rid2, StreamEvent::ThinkingDelta {
                                session_id: sid2.clone(),
                                text: thinking.clone(),
                                order: Some(mark.seq),
                                part_id: Some(mark.id.clone()),
                                render_seq: Some(mark.seq),
                            });
                            partial_for_thinking.append_thinking(&thinking);
                        },
                        move |tool_call_id, tool_name| {
                            let tool_name = tool_registry_for_tool_start
                                .canonical_name(&tool_name)
                                .unwrap_or(tool_name);
                            emitted_output_for_tool.store(true, Ordering::Relaxed);
                            let mark = render_order_for_tool
                                .lock()
                                .map(|mut tracker| tracker.mark_tool(&rid3, &tool_call_id))
                                .unwrap_or(RenderPartMark {
                                    id: tool_call_id.clone(),
                                    seq: 1,
                                });
                            if !first_tool_call_logged_for_cb.swap(true, Ordering::Relaxed) {
                                eprintln!(
                                    "[Agent {}] first tool call start: session={} run={} iteration={} attempt={}/{} elapsed_ms={} tool_call_id={} tool_name={}",
                                    agent_id_for_tool_start,
                                    sid3,
                                    rid3,
                                    iteration,
                                    attempt_number,
                                    LLM_RETRIES + 1,
                                    llm_call_started_at.elapsed().as_millis(),
                                    tool_call_id,
                                    tool_name
                                );
                            }
                            emit_stream(&hdl3, &rid3, StreamEvent::ToolCallStart {
                                session_id: sid3.clone(),
                                tool_call_id: tool_call_id.clone(),
                                tool_name: tool_name.clone(),
                                arguments: String::new(),
                                order: Some(mark.seq),
                                part_id: Some(tool_call_id.clone()),
                                render_seq: Some(mark.seq),
                            });
                            if let Some(ref parent) = ptc3 {
                                emit_parent_stream(
                                    &hdl3,
                                    parent.subagent_tool_call_start(
                                        tool_call_id,
                                        tool_name,
                                        String::new(),
                                        Some(mark.seq),
                                        Some(mark.id),
                                        Some(mark.seq),
                                    ),
                                );
                            }
                        },
                    ) => Some(result),
                    _ = cancel_rx.changed() => None,
                };

                match result {
                    None => {
                        eprintln!(
                            "[Agent {}] LLM attempt cancelled before completion: session={} run={} iteration={} attempt={}/{} elapsed_ms={}",
                            self.id,
                            self.session_id,
                            run_id,
                            iteration,
                            attempt_number,
                            LLM_RETRIES + 1,
                            llm_call_started_at.elapsed().as_millis()
                        );
                        self.clear_pending_knowledge_proposal(app_handle).await;
                        self.emit_cancelled(app_handle, store, &run_id);
                        return Ok(String::new());
                    }
                    Some(Ok(resp)) => {
                        if let Err(e) = validate_llm_tool_calls(&resp.tool_calls) {
                            let attempt_had_output = attempt_emitted_output.load(Ordering::Relaxed);
                            eprintln!(
                                "[Agent {}] LLM attempt returned invalid tool calls: session={} run={} iteration={} attempt={}/{} elapsed_ms={} error={}",
                                self.id,
                                self.session_id,
                                run_id,
                                iteration,
                                attempt_number,
                                LLM_RETRIES + 1,
                                llm_call_started_at.elapsed().as_millis(),
                                e
                            );
                            last_llm_error = e.clone();
                            if !attempt_had_output && llm_attempt < LLM_RETRIES {
                                let delay = 2000 * (llm_attempt as u64 + 1);
                                eprintln!(
                                    "[Agent {}] invalid tool calls (attempt {}/{}), retrying in {}ms: {}",
                                    self.id,
                                    llm_attempt + 1,
                                    LLM_RETRIES + 1,
                                    delay,
                                    e
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                continue;
                            }
                            if attempt_had_output {
                                eprintln!(
                                    "[Agent {}] invalid tool calls after streamed output; stopping retry to avoid duplicate visible output",
                                    self.id
                                );
                                break;
                            }
                            continue;
                        }
                        eprintln!(
                            "[Agent {}] LLM attempt success: session={} run={} iteration={} attempt={}/{} elapsed_ms={} text_len={} thinking_len={} tool_calls={} finish_reason={}",
                            self.id,
                            self.session_id,
                            run_id,
                            iteration,
                            attempt_number,
                            LLM_RETRIES + 1,
                            llm_call_started_at.elapsed().as_millis(),
                            resp.text.len(),
                            resp.thinking_text.len(),
                            resp.tool_calls.len(),
                            resp.finish_reason
                        );
                        if !resp.text.is_empty() {
                            response_text_part = render_order_tracker
                                .lock()
                                .ok()
                                .map(|mut tracker| {
                                    tracker.mark_text(
                                        &run_id,
                                        &format!(
                                            "iteration:{}:attempt:{}:text",
                                            iteration, attempt_number
                                        ),
                                    )
                                });
                        }
                        if !resp.thinking_text.is_empty() {
                            response_thinking_part = render_order_tracker
                                .lock()
                                .ok()
                                .map(|mut tracker| {
                                    tracker.mark_thinking(
                                        &run_id,
                                        &format!(
                                            "iteration:{}:attempt:{}:thinking",
                                            iteration, attempt_number
                                        ),
                                    )
                                });
                        }
                        response = Some(resp);
                        break;
                    }
                    Some(Err(e)) => {
                        let e = user_friendly_llm_error(&e);
                        eprintln!(
                            "[Agent {}] LLM attempt error: session={} run={} iteration={} attempt={}/{} elapsed_ms={} error={}",
                            self.id,
                            self.session_id,
                            run_id,
                            iteration,
                            attempt_number,
                            LLM_RETRIES + 1,
                            llm_call_started_at.elapsed().as_millis(),
                            e
                        );
                        self.record_raw_attempt(
                            "normal",
                            iteration,
                            attempt_number,
                            &system_parts,
                            &prepared_messages,
                            &api_tools,
                            estimated_input_tokens,
                            false,
                            &e,
                            None,
                        )
                        .await;
                        if is_prompt_too_long_error(&e) && !compact_tracker.is_circuit_broken() {
                            eprintln!(
                                "[Agent {}] prompt-too-long detected on iteration {}, scheduling reactive compact: {}",
                                self.id, iteration, e
                            );
                            last_llm_error = e;
                            needs_reactive_compact = true;
                            break;
                        }

                        let is_retryable = is_retryable_llm_error(&e);
                        let attempt_had_output = attempt_emitted_output.load(Ordering::Relaxed);

                        if is_retryable && !attempt_had_output && llm_attempt < LLM_RETRIES {
                            let delay = 2000 * (llm_attempt as u64 + 1);
                            eprintln!(
                                "[Agent {}] LLM stream error (attempt {}/{}), retrying in {}ms: {}",
                                self.id, llm_attempt + 1, LLM_RETRIES + 1, delay, e
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            continue;
                        }
                        if is_retryable && attempt_had_output {
                            eprintln!(
                                "[Agent {}] retryable LLM error after streamed output; stopping retry to avoid duplicate visible output",
                                self.id
                            );
                        }

                        eprintln!("[Agent {}] LLM error (iteration {}):\n{}", self.id, iteration, e);
                        last_llm_error = e;
                    }
                }
            }

            let response = match response {
                Some(r) => r,
                None if needs_reactive_compact => {
                    match self
                        .execute_auto_compact(
                            app_handle,
                            store,
                            &system_parts,
                            estimated_input_tokens,
                            ctx_limit,
                            false,
                            &run_id,
                            "reactive_compact",
                            iteration,
                        )
                        .await
                    {
                        Ok(true) => {
                            compact_tracker.record_success();
                            eprintln!("[Agent {}] reactive auto-compact succeeded", self.id);
                            continue 'agent_loop;
                        }
                        Ok(false) => {}
                        Err(e) => {
                            compact_tracker.record_failure();
                            eprintln!("[Agent {}] reactive auto-compact failed: {}", self.id, e);
                        }
                    }
                    return Err(last_llm_error);
                }
                None => return Err(last_llm_error),
            };

            {
                let round = RawRound {
                    round: iteration,
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                    request: serde_json::from_str(&response.raw_request)
                        .unwrap_or_else(|_| serde_json::Value::String(response.raw_request.clone())),
                    response: response.raw_response.clone(),
                };
                self.raw_store.lock().await
                    .entry(self.session_id.clone())
                    .or_insert_with(Vec::new)
                    .push(round);
            }

            if !response.text.is_empty() && response_text_part.is_none() {
                response_text_part = render_order_tracker.lock().ok().map(|mut tracker| {
                    tracker.mark_text(&run_id, &format!("iteration:{}:text", iteration))
                });
            }
            if !response.thinking_text.is_empty() && response_thinking_part.is_none() {
                response_thinking_part = render_order_tracker.lock().ok().map(|mut tracker| {
                    tracker.mark_thinking(&run_id, &format!("iteration:{}:thinking", iteration))
                });
            }
            let mut ordered_tool_calls = render_order_tracker
                .lock()
                .map(|mut tracker| {
                    tracker.assign_tool_orders_for_run(&run_id, &response.tool_calls)
                })
                .unwrap_or_else(|_| response.tool_calls.clone());
            self.normalize_tool_call_names(&mut ordered_tool_calls);
            self.rewrite_meta_tool_calls_for_execution(&mut ordered_tool_calls)
                .await;
            let response_content_order = response_text_part.as_ref().map(|part| part.seq);
            let response_thinking_order = response_thinking_part.as_ref().map(|part| part.seq);
            let response_render_parts = assistant_render_parts_for_response(
                &run_id,
                response_text_part.clone(),
                &response.text,
                response_thinking_part.clone(),
                &response.thinking_text,
                (response.thinking_duration_secs > 0).then_some(response.thinking_duration_secs),
                (!response.thinking_signature.is_empty()).then_some(response.thinking_signature.as_str()),
                &ordered_tool_calls,
            );

            if response.input_tokens > 0 || response.output_tokens > 0
                || response.cache_read_tokens > 0 || response.cache_write_tokens > 0
            {
                let priced_rounds = if matches!(&self.backend, LlmBackend::OpenRouter { .. }) {
                    1
                } else {
                    0
                };
                let context_tokens = response.input_tokens
                    + response.cache_read_tokens
                    + response.cache_write_tokens
                    + response.output_tokens;
                let context_limit = if let LlmBackend::Custom { context_length, .. } = &self.backend {
                    *context_length
                } else {
                    model_context_limit(&self.effective_model)
                };
                match store.record_token_usage(
                    &self.session_id,
                    response.input_tokens as u64,
                    response.output_tokens as u64,
                    response.cache_read_tokens as u64,
                    response.cache_write_tokens as u64,
                    response.cost_usd,
                    priced_rounds,
                    Some(context_tokens),
                    Some(context_limit),
                ) {
                    Ok(totals) => {
                        eprintln!(
                            "[Agent {}] tokens: +{}in/+{}out/+{}cache_r/+{}cache_w, cost=${:.6}, total: {}in/{}out/{}cache_r/{}cache_w/${:.6}",
                            self.id,
                            response.input_tokens, response.output_tokens,
                            response.cache_read_tokens, response.cache_write_tokens,
                            response.cost_usd,
                            totals.total_input_tokens, totals.total_output_tokens,
                            totals.total_cache_read_tokens, totals.total_cache_write_tokens,
                            totals.total_cost_usd,
                        );
                        emit_stream(app_handle, &run_id, StreamEvent::UsageUpdate {
                            session_id: self.session_id.clone(),
                            input_tokens: response.input_tokens,
                            output_tokens: response.output_tokens,
                            cache_read_tokens: response.cache_read_tokens,
                            cache_write_tokens: response.cache_write_tokens,
                            total_input_tokens: totals.total_input_tokens,
                            total_output_tokens: totals.total_output_tokens,
                            total_cache_read_tokens: totals.total_cache_read_tokens,
                            total_cache_write_tokens: totals.total_cache_write_tokens,
                            total_cost_usd: totals.total_cost_usd,
                            priced_rounds: totals.priced_rounds,
                            context_tokens,
                            context_limit,
                        });
                    }
                    Err(e) => {
                        eprintln!("[Agent {}] failed to record token usage: {}", self.id, e);
                    }
                }
            }

            // Emit ToolCallStart (with arguments) + ToolCallDone for server tool calls (e.g. web_search)
            // that have pre-computed output. These don't need local execution. Output is embedded
            // as text in the assistant message for API history, so no separate Tool message is needed.
            for tc in &ordered_tool_calls {
                if let Some(ref output) = tc.server_tool_output {
                    eprintln!(
                        "[Agent {}] server tool '{}' (id={}) has pre-computed output ({} chars)",
                        self.id, tc.name, tc.id, output.len()
                    );
                    // Re-emit ToolCallStart with arguments so the frontend can display them.
                    emit_stream(app_handle, &run_id, StreamEvent::ToolCallStart {
                        session_id: self.session_id.clone(),
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                        order: tc.order,
                        part_id: Some(tc.id.clone()),
                        render_seq: tc.order,
                    });
                    emit_stream(app_handle, &run_id, StreamEvent::ToolCallDone {
                        session_id: self.session_id.clone(),
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        output: output.clone(),
                        outcome: crate::commands::ToolCallOutcome::Done,
                        images: None,
                    });
                    if let Some(ref parent) = self.parent_tool_call {
                        emit_parent_stream(
                            app_handle,
                            parent.subagent_tool_call_start(
                                tc.id.clone(),
                                tc.name.clone(),
                                tc.arguments.clone(),
                                tc.order,
                                Some(tc.id.clone()),
                                tc.order,
                            ),
                        );
                        emit_parent_stream(
                            app_handle,
                            parent.subagent_tool_call_done(
                                tc.id.clone(),
                                tc.name.clone(),
                                output.clone(),
                                crate::commands::ToolCallOutcome::Done,
                                None,
                            ),
                        );
                    }
                }
            }

            let has_executable_tool_calls = ordered_tool_calls.iter()
                .any(|tc| !tc.is_server_tool());

            if !ordered_tool_calls.is_empty() {
                eprintln!(
                    "[Agent {}] got {} tool calls ({} executable, {} server)",
                    self.id,
                    ordered_tool_calls.len(),
                    ordered_tool_calls.iter().filter(|tc| !tc.is_server_tool()).count(),
                    ordered_tool_calls.iter().filter(|tc| tc.is_server_tool()).count(),
                );

                let thinking_opt = if response.thinking_text.is_empty() { None } else { Some(response.thinking_text.as_str()) };
                let thinking_dur = if response.thinking_duration_secs > 0 { Some(response.thinking_duration_secs) } else { None };
                let thinking_sig = if response.thinking_signature.is_empty() { None } else { Some(response.thinking_signature.as_str()) };
                let assistant_msg_id = store.add_assistant_with_tool_calls_and_render_parts(
                    &self.session_id,
                    &response.text,
                    &ordered_tool_calls,
                    thinking_opt,
                    thinking_dur,
                    thinking_sig,
                    response.response_id.as_deref(),
                    response.continuation_request.as_ref(),
                    response_content_order,
                    response_thinking_order,
                    &response_render_parts,
                )?;
                self.partial_assistant.mark_persisted(
                    assistant_msg_id.clone(),
                    response.text.clone(),
                    thinking_opt.map(str::to_string),
                    thinking_dur,
                );

                let direct_overrides = self.tool_direct_load_overrides();
                let mut prepared: Vec<(ToolCallInfo, serde_json::Value)> = Vec::new();
                for tc in &ordered_tool_calls {
                    // Skip server tools that already have pre-computed output.
                    if tc.is_server_tool() {
                        continue;
                    }

                    let native_lazy_invocation = self.supports_native_tool_lazy_loading()
                        && !self.should_direct_load_tool(&tc.name, &direct_overrides)
                        && self.loaded_tool_names.lock().await.contains(&tc.name);
                    if native_lazy_invocation {
                        eprintln!(
                            "[Agent {}] native lazy tool invocation '{}' (id={})",
                            self.id, tc.name, tc.id
                        );
                    }

                    eprintln!(
                        "[Agent {}] executing tool '{}' (id={})",
                        self.id, tc.name, tc.id
                    );

                    emit_stream(app_handle, &run_id, StreamEvent::ToolCallStart {
                        session_id: self.session_id.clone(),
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                        order: tc.order,
                        part_id: Some(tc.id.clone()),
                        render_seq: tc.order,
                    });
                    if let Some(ref parent) = self.parent_tool_call {
                        emit_parent_stream(
                            app_handle,
                            parent.subagent_tool_call_start(
                                tc.id.clone(),
                                tc.name.clone(),
                                tc.arguments.clone(),
                                tc.order,
                                Some(tc.id.clone()),
                                tc.order,
                            ),
                        );
                    }

                    let mut args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                        Ok(v) => v,
                        Err(parse_err) if tc.arguments.trim().is_empty() => {
                            eprintln!(
                                "[Agent {}] tool '{}' emitted empty arguments payload; defaulting to {{}}",
                                self.id, tc.name
                            );
                            let _ = parse_err;
                            serde_json::json!({})
                        }
                        Err(parse_err) => {
                            eprintln!(
                                "[Agent {}] tool '{}' arguments JSON parse failed: {} | raw({} chars): {}",
                                self.id, tc.name, parse_err,
                                tc.arguments.len(),
                                &tc.arguments[..tc.arguments.len().min(200)]
                            );
                            let mut fallback = serde_json::json!({});
                            fallback["__parse_error"] = serde_json::Value::String(
                                format!(
                                    "Tool arguments JSON was truncated or malformed during streaming (received {} chars). Parse error: {}. Please retry this tool call with the same arguments.",
                                    tc.arguments.len(), parse_err
                                )
                            );
                            fallback
                        }
                    };
                    normalize_tool_args(&mut args);
                    self.inject_working_dir(&tc.name, &mut args);
                    prepared.push((tc.clone(), args));
                }

                let needs_undo = prepared
                    .iter()
                    .any(|(tc, _)| Self::needs_undo_tracking(&tc.name));
                let has_unity_execute = prepared
                    .iter()
                    .any(|(tc, _)| tc.name == "unity_execute" || tc.name == "unity_run_states");

                let pre_checkpoint = if needs_undo {
                    if let Some(ref undo_mgr) = self.undo_manager {
                        match undo_mgr.before_round(&self.working_dir, "agent round").await {
                            Ok(cp) => cp,
                            Err(e) => {
                                eprintln!("[Agent {}] undo checkpoint failed: {}", self.id, e);
                                let lower = e.to_ascii_lowercase();
                                let message = if lower.contains("unable to index file 'nul'")
                                    || lower.contains("short read while indexing nul")
                                {
                                    "Undo is unavailable for this round because Git could not snapshot the workspace. Remove or rename reserved Windows file names such as NUL in the repository."
                                } else {
                                    "Undo may be unavailable for this round because the workspace snapshot failed."
                                };
                                crate::error::AppError::emit_background(
                                    app_handle,
                                    &crate::error::AppError::new(
                                        "undo.checkpoint_failed",
                                        message,
                                    )
                                    .detail(e)
                                    .operation("undo")
                                    .severity(crate::error::ErrorSeverity::Warning),
                                );
                                None
                            }
                        }
                    } else { None }
                } else { None };

                let has_unity_asset_writes = crate::unity_bridge::is_unity_project(&self.working_dir)
                    && prepared
                        .iter()
                        .any(|(tc, args)| self.is_unity_asset_write_call(tc, args));
                if has_unity_asset_writes {
                    match crate::unity_bridge::begin_edit_session(&self.working_dir, &self.session_id).await {
                        Ok(msg) => eprintln!(
                            "[Agent {}] Unity edit session active for {}: {}",
                            self.id, self.session_id, msg
                        ),
                        Err(e) => eprintln!(
                            "[Agent {}] failed to begin Unity edit session for {}: {}",
                            self.id, self.session_id, e
                        ),
                    }
                }

                let has_unity_recompile = prepared.iter().any(|(tc, _)| tc.name == "unity_recompile");
                let results = if has_unity_recompile {
                    eprintln!(
                        "[Agent {}] executing tool round sequentially because unity_recompile is a barrier",
                        self.id
                    );
                    let mut results = Vec::with_capacity(prepared.len());
                    let mut queued_asset_paths: Vec<String> = Vec::new();
                    for (tc, args) in &prepared {
                        if tc.name == "unity_recompile" && !queued_asset_paths.is_empty() {
                            match crate::unity_bridge::import_assets(&self.working_dir, &queued_asset_paths).await {
                                Ok(msg) => eprintln!(
                                    "[Agent {}] queued changed Unity assets before recompile: {}",
                                    self.id, msg
                                ),
                                Err(e) => eprintln!(
                                    "[Agent {}] failed to queue changed Unity assets before recompile: {}",
                                    self.id, e
                                ),
                            }
                            queued_asset_paths.clear();
                        }

                        let result = self.execute_single_tool(app_handle, store, tc, args, &run_id, &mode).await;
                        if let Some(asset_path) = self.unity_asset_relative_path(tc, args, &result) {
                            queued_asset_paths.push(asset_path);
                        }
                        results.push(result);
                    }

                    if !queued_asset_paths.is_empty() {
                        crate::unity_bridge::import_assets_fire_and_forget(
                            &self.working_dir,
                            queued_asset_paths,
                        );
                    }
                    results
                } else {
                    let mode_ref = mode.as_str();
                    let futures: Vec<_> = prepared.iter().map(|(tc, args)| {
                        self.execute_single_tool(app_handle, store, tc, args, &run_id, mode_ref)
                    }).collect();
                    futures::future::join_all(futures).await
                };

                if !self.run_is_current_for_session(store, &run_id, "tool_round_results", None) {
                    return Ok(String::new());
                }
                if self.is_cancel_requested() {
                    self.clear_pending_knowledge_proposal(app_handle).await;
                    self.emit_cancelled(app_handle, store, &run_id);
                    return Ok(String::new());
                }

                if !has_unity_recompile {
                    let queued_asset_paths: Vec<String> = prepared
                        .iter()
                        .zip(results.iter())
                        .filter_map(|((tc, args), result)| {
                            self.unity_asset_relative_path(tc, args, result)
                        })
                        .collect();

                    if !queued_asset_paths.is_empty() {
                        crate::unity_bridge::import_assets_fire_and_forget(
                            &self.working_dir,
                            queued_asset_paths,
                        );
                    }
                }

                for ((tc, _), result) in prepared.iter().zip(results.iter()) {
                    let stored_output = match store.rewrite_tool_result_for_storage(
                        &self.session_id,
                        &tc.id,
                        &tc.name,
                        &result.output,
                    ) {
                        Ok(output) => output,
                        Err(e) => {
                            eprintln!(
                                "[Agent {}] failed to persist tool_result for '{}' (id={}): {}",
                                self.id, tc.name, tc.id, e
                            );
                            result.output.clone()
                        }
                    };
                    eprintln!(
                        "[Agent {}] tool '{}' result: outcome={:?}, is_error={}, output_len={} (stored={})",
                        self.id,
                        tc.name,
                        result.outcome,
                        result.is_error,
                        result.output.len(),
                        stored_output.len()
                    );

                    match store.add_tool_result_with_images_for_run(
                        &self.session_id,
                        &run_id,
                        &tc.id,
                        &stored_output,
                        result.images.as_deref(),
                    ) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            eprintln!(
                                "[Agent {}] discarding stale tool result before save: session={} run={} tool_call_id={}",
                                self.id, self.session_id, run_id, tc.id
                            );
                            return Ok(String::new());
                        }
                        Err(e) => {
                            eprintln!(
                                "[Agent {}] failed to save tool_result for '{}' (id={}): {}",
                                self.id, tc.name, tc.id, e
                            );
                        }
                    }

                    emit_stream(app_handle, &run_id, StreamEvent::ToolCallDone {
                        session_id: self.session_id.clone(),
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        output: stored_output.clone(),
                        outcome: result.outcome.as_stream_outcome(),
                        images: result.images.clone(),
                    });
                    if let Some(ref parent) = self.parent_tool_call {
                        let truncated_output = if stored_output.chars().count() > 500 {
                            let s: String = stored_output.chars().take(500).collect();
                            format!("{}…({} chars)", s, result.output.chars().count())
                        } else {
                            stored_output.clone()
                        };
                        emit_parent_stream(
                            app_handle,
                            parent.subagent_tool_call_done(
                                tc.id.clone(),
                                tc.name.clone(),
                                truncated_output,
                                result.outcome.as_stream_outcome(),
                                result.images.clone(),
                            ),
                        );
                    }
                }

                let results_by_id: BTreeMap<&str, &ExecutedToolResult> = prepared
                    .iter()
                    .zip(results.iter())
                    .map(|((tool_call, _), result)| (tool_call.id.as_str(), result))
                    .collect();
                let finalized_tool_calls: Vec<ToolCallInfo> = ordered_tool_calls
                    .iter()
                    .map(|tool_call| {
                        finalize_tool_call_record(
                            tool_call,
                            results_by_id.get(tool_call.id.as_str()).copied(),
                        )
                    })
                    .collect();

                let finalized_render_parts = assistant_render_parts_for_response(
                    &run_id,
                    response_text_part.clone(),
                    &response.text,
                    response_thinking_part.clone(),
                    &response.thinking_text,
                    (response.thinking_duration_secs > 0)
                        .then_some(response.thinking_duration_secs),
                    (!response.thinking_signature.is_empty())
                        .then_some(response.thinking_signature.as_str()),
                    &finalized_tool_calls,
                );

                if let Err(e) = store.update_message_tool_calls_and_render_parts(
                    &assistant_msg_id,
                    &finalized_tool_calls,
                    &finalized_render_parts,
                ) {
                    eprintln!(
                        "[Agent {}] failed to update tool_calls/render_parts for assistant message {}: {}",
                        self.id, assistant_msg_id, e
                    );
                }

                if let Some(checkpoint) = pre_checkpoint {
                    if let Some(ref undo_mgr) = self.undo_manager {
                        let recorded = undo_mgr
                            .after_round(
                                &self.session_id,
                                &assistant_msg_id,
                                Some(run_id.as_str()),
                                checkpoint,
                                has_unity_execute,
                                &self.working_dir,
                            )
                            .await;
                        match recorded {
                            Ok(true) => {
                                eprintln!(
                                    "[Agent {}] emitting UndoAvailable for session {} run {} message {}",
                                    self.id, self.session_id, run_id, assistant_msg_id
                                );
                                emit_stream(app_handle, &run_id, StreamEvent::UndoAvailable {
                                    session_id: self.session_id.clone(),
                                    assistant_message_id: assistant_msg_id.clone(),
                                });
                            }
                            Ok(false) => {}
                            Err(e) => {
                                eprintln!(
                                    "[Agent {}] failed to record undo state for session {} message {}: {}",
                                    self.id, self.session_id, assistant_msg_id, e
                                );
                                crate::error::AppError::emit_background(
                                    app_handle,
                                    &crate::error::AppError::new(
                                        "undo.record_failed",
                                        "Undo may be unavailable for this round because file-change capture failed.",
                                    )
                                    .detail(e)
                                    .operation("undo")
                                    .severity(crate::error::ErrorSeverity::Warning),
                                );
                            }
                        }
                    }
                }

                emit_stream(app_handle, &run_id, StreamEvent::ToolCallRoundDone {
                    session_id: self.session_id.clone(),
                    message_id: assistant_msg_id.clone(),
                    full_text: response.text.clone(),
                    tool_calls: finalized_tool_calls,
                    content_order: response_content_order,
                    thinking_order: response_thinking_order,
                    render_parts: Some(finalized_render_parts),
                });
                self.partial_assistant.reset();

                if self.is_cancel_requested() {
                    self.clear_pending_knowledge_proposal(app_handle).await;
                    self.emit_cancelled(app_handle, store, &run_id);
                    return Ok(String::new());
                }

                if !has_executable_tool_calls {
                    store.close_run_pending_input_queue(&run_id)?;
                }

                if self.drain_queued_pending_inputs(
                    app_handle,
                    store,
                    &run_id,
                    env_prompt_prefix.as_deref(),
                )? {
                    store.update_run_status(&run_id, "running", None)?;
                    continue 'agent_loop;
                }

                if has_executable_tool_calls {
                    continue;
                }
                // Server-tool-only round: model already provided its answer alongside the
                // server tool results. toolCallRoundDone already emitted, message already stored.
                final_text = response.text;
                final_thinking_text = response.thinking_text;
                final_thinking_duration = response.thinking_duration_secs;
                final_thinking_signature = response.thinking_signature;
                final_response_id = response.response_id;
                final_continuation_request = response.continuation_request;
                final_content_order = response_content_order;
                final_thinking_order = response_thinking_order;
                done_already_emitted = true;
                terminal_done_message_id = Some(assistant_msg_id);
                break;
            }

            store.close_run_pending_input_queue(&run_id)?;
            let pending_inputs = {
                let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                    app_handle.state();
                let mut queue = queue_state
                    .lock()
                    .map_err(|e| format!("Failed to lock pending input queue: {}", e))?;
                queue.claim_immediate(&self.session_id, &run_id)
            };
            if !pending_inputs.is_empty() {
                let thinking_opt = if response.thinking_text.is_empty() {
                    None
                } else {
                    Some(response.thinking_text.as_str())
                };
                let thinking_dur = if response.thinking_duration_secs > 0 {
                    Some(response.thinking_duration_secs)
                } else {
                    None
                };
                let thinking_sig = if response.thinking_signature.is_empty() {
                    None
                } else {
                    Some(response.thinking_signature.as_str())
                };
                let assistant_msg_id = store.add_message_with_thinking_and_render_parts(
                    &self.session_id,
                    MessageRole::Assistant,
                    &response.text,
                    thinking_opt,
                    thinking_dur,
                    thinking_sig,
                    response.response_id.as_deref(),
                    response.continuation_request.as_ref(),
                    response_content_order,
                    response_thinking_order,
                    &response_render_parts,
                )?;
                self.partial_assistant.mark_persisted(
                    assistant_msg_id.clone(),
                    response.text.clone(),
                    thinking_opt.map(str::to_string),
                    thinking_dur,
                );
                emit_stream(app_handle, &run_id, StreamEvent::ToolCallRoundDone {
                    session_id: self.session_id.clone(),
                    message_id: assistant_msg_id,
                    full_text: response.text.clone(),
                    tool_calls: Vec::new(),
                    content_order: response_content_order,
                    thinking_order: response_thinking_order,
                    render_parts: Some(response_render_parts),
                });
                self.partial_assistant.reset();
                self.persist_claimed_pending_inputs(
                    app_handle,
                    store,
                    &run_id,
                    env_prompt_prefix.as_deref(),
                    pending_inputs,
                )?;
                store.update_run_status(&run_id, "running", None)?;
                continue 'agent_loop;
            }

            final_thinking_text = response.thinking_text;
            final_thinking_duration = response.thinking_duration_secs;
            final_thinking_signature = response.thinking_signature;
            final_text = response.text;
            final_response_id = response.response_id;
            final_continuation_request = response.continuation_request;
            final_content_order = response_content_order;
            final_thinking_order = response_thinking_order;
            break;
        }

        if !done_already_emitted {
            let thinking_opt = if final_thinking_text.is_empty() {
                None
            } else {
                Some(final_thinking_text.as_str())
            };
            let thinking_dur = if final_thinking_duration > 0 {
                Some(final_thinking_duration)
            } else {
                None
            };
            let thinking_sig = if final_thinking_signature.is_empty() {
                None
            } else {
                Some(final_thinking_signature.as_str())
            };
            let final_render_parts = assistant_render_parts_for_response(
                &run_id,
                final_content_order.map(|seq| RenderPartMark {
                    id: format!("{}:text:final", run_id),
                    seq,
                }),
                &final_text,
                final_thinking_order.map(|seq| RenderPartMark {
                    id: format!("{}:thinking:final", run_id),
                    seq,
                }),
                thinking_opt.unwrap_or_default(),
                thinking_dur,
                thinking_sig,
                &[],
            );
            let msg_id = store.add_message_with_thinking_and_render_parts(
                &self.session_id,
                MessageRole::Assistant,
                &final_text,
                thinking_opt,
                thinking_dur,
                thinking_sig,
                final_response_id.as_deref(),
                final_continuation_request.as_ref(),
                final_content_order,
                final_thinking_order,
                &final_render_parts,
            )?;
            self.partial_assistant.mark_persisted(
                msg_id.clone(),
                final_text.clone(),
                thinking_opt.map(str::to_string),
                thinking_dur,
            );

            if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(&run_id)) {
                eprintln!(
                    "[Agent {}] failed to persist latest completed run id for session {} run {}: {}",
                    self.id, self.session_id, run_id, error
                );
                crate::error::AppError::emit_background(
                    app_handle,
                    &crate::error::AppError::new(
                        "session.latest_run_persist_failed",
                        "Latest run boundary may be unavailable for this session.",
                    )
                    .detail(error)
                    .operation("session")
                    .severity(crate::error::ErrorSeverity::Warning),
                );
            }

            eprintln!(
                "[Agent {}] emitting Done for session {} run {} message {} text_len={}",
                self.id,
                self.session_id,
                run_id,
                msg_id,
                final_text.len()
            );
            emit_stream(
                app_handle,
                &run_id,
                StreamEvent::Done {
                    session_id: self.session_id.clone(),
                    message_id: msg_id,
                    full_text: final_text.clone(),
                    content_order: final_content_order,
                    thinking_order: final_thinking_order,
                    render_parts: Some(final_render_parts),
                },
            );
            self.partial_assistant.reset();
        } else {
            // Server-tool-only rounds already persisted their assistant message via
            // ToolCallRoundDone. The explicit Done event still needs to fire with the
            // same message id so the frontend can clear its in-flight run state while
            // still seeing the terminal response text.
            let terminal_message_id = terminal_done_message_id.clone().unwrap_or_default();

            if let Err(error) = store.set_latest_completed_run_id(&self.session_id, Some(&run_id)) {
                eprintln!(
                    "[Agent {}] failed to persist latest completed run id for session {} run {}: {}",
                    self.id, self.session_id, run_id, error
                );
                crate::error::AppError::emit_background(
                    app_handle,
                    &crate::error::AppError::new(
                        "session.latest_run_persist_failed",
                        "Latest run boundary may be unavailable for this session.",
                    )
                    .detail(error)
                    .operation("session")
                    .severity(crate::error::ErrorSeverity::Warning),
                );
            }

            eprintln!(
                "[Agent {}] emitting Done for session {} run {} message {} (server-tool-only round) text_len={}",
                self.id,
                self.session_id,
                run_id,
                terminal_message_id,
                final_text.len()
            );
            emit_stream(
                app_handle,
                &run_id,
                StreamEvent::Done {
                    session_id: self.session_id.clone(),
                    message_id: terminal_message_id,
                    full_text: final_text.clone(),
                    content_order: final_content_order,
                    thinking_order: final_thinking_order,
                    render_parts: None,
                },
            );
            self.partial_assistant.reset();
        }

        if let Err(error) = self
            .flush_pending_knowledge_proposal(app_handle, store, &run_id)
            .await
        {
            eprintln!(
                "[Agent {}] failed to flush knowledge proposal for session {}: {}",
                self.id, self.session_id, error
            );
        }

        eprintln!(
            "[Agent {}] loop finished after {} iterations",
            self.id, iteration
        );

        eprintln!(
            "[Agent {}] raw rounds already stored incrementally for session {}",
            self.id, self.session_id
        );

        Ok(final_text)
        }.await;

            self.cleanup_unity_edit_session().await;
            eprintln!(
                "[Agent {}] run pipeline finished: session={} run={} elapsed_ms={} ok={}",
                self.id,
                self.session_id,
                run_id,
                run_started_at.elapsed().as_millis(),
                run_result.is_ok()
            );

            if let Err(ref err) = run_result {
                self.clear_pending_knowledge_proposal(app_handle).await;
                let interrupted = Self::persist_interrupted_assistant_snapshot(
                    store,
                    &self.session_id,
                    &self.partial_assistant.snapshot(),
                );
                if interrupted.is_some() {
                    self.partial_assistant.reset();
                }
                eprintln!(
                    "[Agent {}] emitting Error for session {} run {}: {}",
                    self.id, self.session_id, run_id, err
                );
                emit_stream(
                    app_handle,
                    &run_id,
                    StreamEvent::Error {
                        session_id: self.session_id.clone(),
                        error: crate::error::AppError::new("chat.stream_failed", err),
                    },
                );
            }

            run_result
        }) // end Box::pin(async move { ... })
    }

    fn is_readonly_tool(name: &str) -> bool {
        matches!(
            name,
            "read"
                | "grep"
                | "list"
                | "ask_user_question"
                | "todowrite"
                | "graph_view"
                | "unity_ref_search"
                | "unity_asset_search"
                | "unity_capture_viewport"
                | "unity_yaml_list"
                | "unity_yaml_search"
                | "unity_yaml_read"
                | "unity_recompile"
                | "knowledge_list"
                | "knowledge_query"
                | "knowledge_read"
                | "skill_list"
                | "skill_reload"
                | "config_query"
                | "tool_load"
        )
    }

    fn needs_undo_tracking(name: &str) -> bool {
        matches!(
            name,
            "unity_execute"
                | "unity_run_states"
                | "write"
                | "edit"
                | "bash"
                | "knowledge_create"
                | "knowledge_edit"
                | "knowledge_move"
                | "knowledge_delete"
                | "skill_create"
        )
    }

    fn default_tool_requires_confirm(name: &str) -> bool {
        match name {
            "knowledge_create" | "knowledge_edit" | "knowledge_move" | "knowledge_delete" => false,
            _ => !Self::is_readonly_tool(name),
        }
    }

    fn permission_requires_confirm(
        global_mode: &str,
        tool_mode: Option<&str>,
        tool_name: &str,
    ) -> bool {
        if Self::normalize_global_permission_mode(global_mode) == PermissionModeSetting::Auto {
            return false;
        }

        match Self::normalize_tool_permission_mode(tool_mode) {
            Some(PermissionModeSetting::Auto) => false,
            Some(PermissionModeSetting::Ask) => true,
            _ => Self::default_tool_requires_confirm(tool_name),
        }
    }

    fn normalize_global_permission_mode(mode: &str) -> PermissionModeSetting {
        if mode.trim().eq_ignore_ascii_case("ask") {
            PermissionModeSetting::Ask
        } else {
            PermissionModeSetting::Auto
        }
    }

    fn normalize_tool_permission_mode(mode: Option<&str>) -> Option<PermissionModeSetting> {
        match mode.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("auto") => Some(PermissionModeSetting::Auto),
            Some(value) if value.eq_ignore_ascii_case("ask") => Some(PermissionModeSetting::Ask),
            _ => None,
        }
    }

    fn permission_setting_requires_confirm(
        mode: Option<&str>,
        default_requires_confirm: bool,
    ) -> bool {
        match Self::normalize_tool_permission_mode(mode) {
            Some(PermissionModeSetting::Auto) => false,
            Some(PermissionModeSetting::Ask) => true,
            None => default_requires_confirm,
        }
    }

    fn user_wait_target(&self, run_id: &str) -> UserWaitTarget {
        match self.parent_tool_call.as_ref() {
            Some(parent) => UserWaitTarget {
                session_id: parent.session_id.clone(),
                run_id: parent.run_id.clone(),
            },
            None => UserWaitTarget {
                session_id: self.session_id.clone(),
                run_id: run_id.to_string(),
            },
        }
    }

    fn permission_confirm_reason(
        global_mode: &str,
        tool_mode: Option<&str>,
        tool_name: &str,
    ) -> Option<ToolConfirmReason> {
        Self::permission_requires_confirm(global_mode, tool_mode, tool_name)
            .then_some(ToolConfirmReason::UserPermission)
    }

    fn build_tool_confirm_display(
        tool_name: &str,
        arguments: &str,
        knowledge_preview: Option<KnowledgeToolConfirmPreview>,
    ) -> ToolConfirmDisplay {
        match knowledge_preview {
            Some(preview) => ToolConfirmDisplay::Knowledge(preview),
            None => ToolConfirmDisplay::Basic(BasicToolConfirmDisplay {
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
            }),
        }
    }

    fn assess_tool_confirmation(
        global_mode: &str,
        tool_mode: Option<&str>,
        tool_name: &str,
        arguments: &str,
        knowledge_preview: Option<KnowledgeToolConfirmPreview>,
        knowledge_governance_requires_confirm: bool,
    ) -> ToolConfirmAssessment {
        let mut reasons = Vec::new();
        if let Some(reason) = Self::permission_confirm_reason(global_mode, tool_mode, tool_name) {
            reasons.push(reason);
        }
        if knowledge_governance_requires_confirm {
            reasons.push(ToolConfirmReason::KnowledgeGovernance);
        }

        ToolConfirmAssessment {
            reasons,
            display: Self::build_tool_confirm_display(tool_name, arguments, knowledge_preview),
        }
    }

    fn parse_tool_confirm_answer(answer: &str) -> ToolConfirmDecision {
        if answer == "allow" {
            return ToolConfirmDecision::Allow;
        }
        if answer == "deny" {
            return ToolConfirmDecision::Deny { feedback: None };
        }
        if let Some(feedback) = answer.strip_prefix("feedback:") {
            let trimmed = feedback.trim();
            return ToolConfirmDecision::Deny {
                feedback: (!trimmed.is_empty()).then(|| trimmed.to_string()),
            };
        }
        let trimmed = answer.trim();
        ToolConfirmDecision::Deny {
            feedback: (!trimmed.is_empty()).then(|| trimmed.to_string()),
        }
    }

    async fn request_tool_confirm(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &str,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ToolConfirmDecision {
        let mode_state: tauri::State<crate::ToolPermissionMode> = app_handle.state();
        let global_mode = mode_state.0.read().await.clone();
        let normalized_global_mode = Self::normalize_global_permission_mode(&global_mode);

        let mut knowledge_preview: Option<KnowledgeToolConfirmPreview> = None;
        let mut knowledge_governance_triggered = false;
        if matches!(
            tool_name,
            "knowledge_create" | "knowledge_edit" | "knowledge_move" | "knowledge_delete"
        ) {
            match assess_knowledge_tool_confirmation(&self.working_dir, tool_name, args) {
                Ok(Some(assessment)) => {
                    knowledge_governance_triggered = assessment.governance_requires_confirm;
                    knowledge_preview = Some(assessment.preview);
                }
                Ok(None) => {}
                Err(error) => {
                    knowledge_governance_triggered = true;
                    eprintln!(
                        "[Agent {}] knowledge tool confirm preview failed for '{}' (id={}): {}",
                        self.id, tool_name, tool_call_id, error
                    );
                }
            }
        }
        if tool_name == "bash" {
            if let Some(assessment) = Self::assess_bash_git_knowledge_command(
                &self.working_dir,
                self.app_knowledge_dir.as_ref().as_ref(),
                args,
            ) {
                if assessment.requires_confirm {
                    knowledge_governance_triggered = true;
                }
            }
        }

        let perms_state: tauri::State<crate::ToolPermissions> = app_handle.state();
        let perms = perms_state.0.read().await;
        let tool_mode = perms.get(tool_name).cloned();
        let knowledge_governance_requires_confirm = knowledge_governance_triggered
            && Self::permission_setting_requires_confirm(
                perms
                    .get(PERMISSION_BEHAVIOR_KNOWLEDGE_GOVERNANCE)
                    .map(String::as_str),
                true,
            );
        drop(perms);

        if normalized_global_mode == PermissionModeSetting::Auto
            && !knowledge_governance_requires_confirm
        {
            eprintln!(
                "[Agent {}] tool confirm skipped for '{}' (global_mode=auto)",
                self.id, tool_name
            );
            return ToolConfirmDecision::Allow;
        }

        let assessment = Self::assess_tool_confirmation(
            &global_mode,
            tool_mode.as_deref(),
            tool_name,
            arguments,
            knowledge_preview,
            knowledge_governance_requires_confirm,
        );

        if assessment.reasons.is_empty() {
            eprintln!(
                "[Agent {}] tool confirm skipped for '{}' (global_mode='{}', tool_mode={:?})",
                self.id, tool_name, global_mode, tool_mode
            );
            return ToolConfirmDecision::Allow;
        }

        eprintln!(
            "[Agent {}] tool confirm required for '{}' (global_mode='{}', tool_mode={:?}, reasons={:?})",
            self.id, tool_name, global_mode, tool_mode, assessment.reasons
        );

        let question_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let wait_target = self.user_wait_target(run_id);

        {
            let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
            let mut store = question_store.lock().await;
            store.insert(
                question_id.clone(),
                crate::PendingQuestionResponse {
                    session_id: wait_target.session_id.clone(),
                    run_id: wait_target.run_id.clone(),
                    tx,
                },
            );
        }

        emit_stream(
            app_handle,
            &wait_target.run_id,
            crate::commands::StreamEvent::ToolConfirm {
                session_id: wait_target.session_id.clone(),
                question_id: question_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                display: assessment.display,
            },
        );

        eprintln!(
            "[Agent {}] tool confirm: waiting for user approval of '{}' (question_id={})",
            self.id, tool_name, question_id
        );

        let mut cancel_rx = self.cancel_waiter();
        let answer_result = tokio::select! {
            result = rx => Some(result),
            _ = cancel_rx.changed() => None,
        };

        match answer_result {
            Some(Ok(answer)) => {
                let decision = Self::parse_tool_confirm_answer(&answer);
                let status = match &decision {
                    ToolConfirmDecision::Allow => "allowed".to_string(),
                    ToolConfirmDecision::Deny {
                        feedback: Some(feedback),
                    } => format!("rejected with feedback: {}", feedback),
                    ToolConfirmDecision::Deny { feedback: None } => "denied".to_string(),
                };
                eprintln!(
                    "[Agent {}] tool confirm: user {} '{}' (question_id={})",
                    self.id, status, tool_name, question_id
                );
                decision
            }
            Some(Err(_)) => {
                eprintln!(
                    "[Agent {}] tool confirm: cancelled for '{}' (question_id={})",
                    self.id, tool_name, question_id
                );
                ToolConfirmDecision::Deny { feedback: None }
            }
            None => {
                let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
                let mut store = question_store.lock().await;
                store.remove(&question_id);
                eprintln!(
                    "[Agent {}] tool confirm: interrupted for '{}' (question_id={})",
                    self.id, tool_name, question_id
                );
                ToolConfirmDecision::Deny { feedback: None }
            }
        }
    }

    async fn request_unity_editor_status_change_confirm(
        &self,
        app_handle: &AppHandle,
        tool_name: &str,
        tool_call_id: &str,
        current_status: &str,
        requested_status: &str,
        run_id: &str,
    ) -> ToolConfirmDecision {
        let perms_state: tauri::State<crate::ToolPermissions> = app_handle.state();
        let perms = perms_state.0.read().await;
        let requires_confirm = Self::permission_setting_requires_confirm(
            perms
                .get(PERMISSION_BEHAVIOR_UNITY_EDITOR_STATUS_CHANGE)
                .map(String::as_str),
            true,
        );
        drop(perms);

        if !requires_confirm {
            eprintln!(
                "[Agent {}] {} status change confirm skipped (permission behavior=auto)",
                self.id, tool_name
            );
            return ToolConfirmDecision::Allow;
        }

        let question_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let wait_target = self.user_wait_target(run_id);

        {
            let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
            let mut store = question_store.lock().await;
            store.insert(
                question_id.clone(),
                crate::PendingQuestionResponse {
                    session_id: wait_target.session_id.clone(),
                    run_id: wait_target.run_id.clone(),
                    tx,
                },
            );
        }

        emit_stream(
            app_handle,
            &wait_target.run_id,
            crate::commands::StreamEvent::ToolConfirm {
                session_id: wait_target.session_id.clone(),
                question_id: question_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                display: ToolConfirmDisplay::UnityEditorStatusChange(
                    UnityEditorStatusChangeConfirmDisplay {
                        tool_name: tool_name.to_string(),
                        current_status: current_status.to_string(),
                        requested_status: requested_status.to_string(),
                    },
                ),
            },
        );

        eprintln!(
            "[Agent {}] {} status change confirm: waiting for user approval (question_id={})",
            self.id, tool_name, question_id
        );

        let mut cancel_rx = self.cancel_waiter();
        let answer_result = tokio::select! {
            result = rx => Some(result),
            _ = cancel_rx.changed() => None,
        };

        match answer_result {
            Some(Ok(answer)) => {
                let decision = Self::parse_tool_confirm_answer(&answer);
                let status = match &decision {
                    ToolConfirmDecision::Allow => "allowed".to_string(),
                    ToolConfirmDecision::Deny {
                        feedback: Some(feedback),
                    } => format!("rejected with feedback: {}", feedback),
                    ToolConfirmDecision::Deny { feedback: None } => "denied".to_string(),
                };
                eprintln!(
                    "[Agent {}] {} status change confirm: user {} (question_id={})",
                    self.id, tool_name, status, question_id
                );
                decision
            }
            Some(Err(_)) => {
                eprintln!(
                    "[Agent {}] {} status change confirm: cancelled (question_id={})",
                    self.id, tool_name, question_id
                );
                ToolConfirmDecision::Deny { feedback: None }
            }
            None => {
                let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
                let mut store = question_store.lock().await;
                store.remove(&question_id);
                eprintln!(
                    "[Agent {}] {} status change confirm: interrupted (question_id={})",
                    self.id, tool_name, question_id
                );
                ToolConfirmDecision::Deny { feedback: None }
            }
        }
    }

    async fn await_tool_result<F>(&self, future: F) -> ExecutedToolResult
    where
        F: std::future::Future<Output = ToolResult> + Send,
    {
        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        let mut cancel_rx = self.cancel_waiter();
        tokio::select! {
            result = future => ExecutedToolResult::from_tool_result(result),
            _ = cancel_rx.changed() => Self::interrupted_tool_result(),
        }
    }

    async fn await_executed_tool_result<F>(&self, future: F) -> ExecutedToolResult
    where
        F: std::future::Future<Output = ExecutedToolResult> + Send,
    {
        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        let mut cancel_rx = self.cancel_waiter();
        tokio::select! {
            result = future => result,
            _ = cancel_rx.changed() => Self::interrupted_tool_result(),
        }
    }

    async fn execute_single_tool(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        tc: &ToolCallInfo,
        args: &serde_json::Value,
        run_id: &str,
        mode: &str,
    ) -> ExecutedToolResult {
        if let Some(parse_err) = args.get("__parse_error").and_then(|v| v.as_str()) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: parse_err.to_string(),
                is_error: true,
            });
        }

        if tc.name == "tool_load" {
            return ExecutedToolResult::from_tool_result(self.execute_tool_load(args).await);
        }

        if tc.name == "tool_call" {
            let output = match parse_meta_tool_call_arguments(&tc.arguments) {
                Ok((target, _)) => format!(
                    "tool_call target '{}' could not be dispatched. Check that the target tool exists and is allowed for this agent.",
                    target
                ),
                Err(error) => error,
            };
            return ExecutedToolResult::from_tool_result(ToolResult {
                output,
                is_error: true,
            });
        }

        if !self.is_allowed_agent_tool(&tc.name).await {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!("Tool '{}' is not allowed for this agent.", tc.name),
                is_error: true,
            });
        }

        // Plan mode enforcement: block non-readonly tools
        if mode == "plan" && !Self::is_readonly_tool(&tc.name) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Tool '{}' is not allowed in plan mode. Plan mode is read-only.",
                    tc.name
                ),
                is_error: true,
            });
        }

        let file_workspace_boundary_enabled = app_handle
            .try_state::<Arc<crate::config::AppConfig>>()
            .map(|config| config.file_tool_workspace_boundary_enabled())
            .unwrap_or(false);
        if let Some(error) = Self::validate_tool_path_requirements(
            &self.working_dir,
            &tc.name,
            args,
            file_workspace_boundary_enabled,
        ) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: error,
                is_error: true,
            });
        }

        if let Some(error) = self.validate_knowledge_tool_routing(&tc.name, args) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: error,
                is_error: true,
            });
        }

        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        match self
            .request_tool_confirm(app_handle, &tc.id, &tc.name, &tc.arguments, args, run_id)
            .await
        {
            ToolConfirmDecision::Allow => {}
            ToolConfirmDecision::Deny { feedback } => {
                if self.is_cancel_requested() {
                    return Self::interrupted_tool_result();
                }
                let output = match feedback {
                    Some(feedback) => format!(
                        "Tool '{}' was rejected by user feedback. Revise the proposal before trying again.\nUser feedback: {}",
                        tc.name, feedback
                    ),
                    None => format!("Tool '{}' was denied by user", tc.name),
                };
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output,
                    is_error: true,
                });
            }
        }

        if tc.name == "read" {
            self.await_executed_tool_result(self.execute_read(app_handle, args))
                .await
        } else if tc.name == "task" {
            self.await_executed_tool_result(
                self.execute_task(app_handle, store, args, &tc.id, run_id),
            )
            .await
        } else if tc.name == "ask_user_question" {
            self.await_tool_result(self.execute_ask(app_handle, &tc.id, args, run_id))
                .await
        } else if tc.name == "graph_view" {
            self.execute_graph_view(app_handle, &tc.id, args).await
        } else if tc.name == "todowrite" {
            ExecutedToolResult::from_tool_result(self.execute_todowrite(store, args, run_id))
        } else if tc.name == "config_query" {
            ExecutedToolResult::from_tool_result(self.execute_config_query(app_handle, args))
        } else if tc.name == "knowledge_list" {
            ExecutedToolResult::from_tool_result(self.execute_knowledge_list(args))
        } else if tc.name == "knowledge_query" {
            self.await_tool_result(self.execute_knowledge_query(app_handle, args))
                .await
        } else if tc.name == "knowledge_read" {
            ExecutedToolResult::from_tool_result(self.execute_knowledge_read(args))
        } else if tc.name == "knowledge_create" {
            self.await_tool_result(self.execute_knowledge_create(app_handle, args))
                .await
        } else if tc.name == "knowledge_edit" {
            self.await_tool_result(self.execute_knowledge_edit(app_handle, args))
                .await
        } else if tc.name == "knowledge_move" {
            self.await_tool_result(self.execute_knowledge_move(app_handle, args))
                .await
        } else if tc.name == "knowledge_delete" {
            self.await_tool_result(self.execute_knowledge_delete(app_handle, args))
                .await
        } else if tc.name == "skill_create" {
            self.await_tool_result(self.execute_skill_create(app_handle, args))
                .await
        } else if tc.name == "skill_reload" {
            self.await_tool_result(self.execute_skill_reload(app_handle, args))
                .await
        } else if tc.name == "skill_list" {
            ExecutedToolResult::from_tool_result(self.execute_skill_list(args))
        } else if tc.name == "unity_execute" {
            self.execute_unity_execute(app_handle, &tc.id, args, run_id)
                .await
        } else if tc.name == "unity_recompile" {
            self.await_tool_result(self.execute_unity_recompile(app_handle, &tc.id, args, run_id))
                .await
        } else if tc.name == "unity_run_states" {
            self.await_tool_result(self.execute_unity_run_states(app_handle, &tc.id, args, run_id))
                .await
        } else if tc.name == "unity_capture_viewport" {
            self.await_executed_tool_result(self.execute_unity_capture_viewport(args))
                .await
        } else if tc.name == "unity_ref_search" {
            ExecutedToolResult::from_tool_result(self.execute_unity_ref_search(app_handle, args))
        } else if tc.name == "unity_asset_search" {
            ExecutedToolResult::from_tool_result(self.execute_unity_asset_search(app_handle, args))
        } else if tc.name == "unity_yaml_list" {
            self.await_tool_result(self.execute_unity_yaml_list(app_handle, args))
                .await
        } else if tc.name == "unity_yaml_search" {
            self.await_tool_result(self.execute_unity_yaml_search(app_handle, args))
                .await
        } else if tc.name == "unity_yaml_read" {
            self.await_tool_result(self.execute_unity_yaml_read(app_handle, args))
                .await
        } else {
            let bash_git_knowledge_assessment = if tc.name == "bash" {
                Self::assess_bash_git_knowledge_command(
                    &self.working_dir,
                    self.app_knowledge_dir.as_ref().as_ref(),
                    args,
                )
            } else {
                None
            };
            let tool_context = self
                .build_tool_execution_context(app_handle, &tc.name)
                .await;
            let mut result = self
                .await_tool_result(self.tool_registry.execute_with_context(
                    &tc.name,
                    args,
                    tool_context,
                ))
                .await;

            if result.outcome == ToolRunOutcome::Done
                && bash_git_knowledge_assessment
                    .map(|assessment| assessment.reconcile_after_success)
                    .unwrap_or(false)
            {
                match self
                    .reconcile_knowledge_workspace_with_source(app_handle, "agent_git")
                    .await
                {
                    Ok(()) => {
                        eprintln!(
                            "[Agent {}] reconciled knowledge index after bash git operation",
                            self.id
                        );
                    }
                    Err(error) => {
                        let suffix = format!(
                            "\n\nWarning: knowledge index reconcile failed after git operation: {}",
                            error
                        );
                        result.output.push_str(&suffix);
                    }
                }
            }

            result
        }
    }

    async fn execute_tool_load(&self, args: &serde_json::Value) -> ToolResult {
        let native_lazy = self.supports_native_tool_lazy_loading();
        let allowed = self.allowed_tool_set().await;
        let requested: Vec<String> = args
            .get("tools")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        if requested.is_empty() {
            return ToolResult {
                output: "Error: tool_load requires a non-empty tools array.".to_string(),
                is_error: true,
            };
        }

        let direct_overrides = self.tool_direct_load_overrides();
        let mut loaded_guard = self.loaded_tool_names.lock().await;
        let mut items = Vec::new();
        for requested_name in requested {
            let Some(canonical) = self.tool_registry.canonical_name(&requested_name) else {
                items.push(serde_json::json!({
                    "requested": requested_name,
                    "status": "unknown_tool",
                }));
                continue;
            };

            if Self::is_meta_tool(&canonical) || !allowed.contains(&canonical) {
                items.push(serde_json::json!({
                    "requested": requested_name,
                    "name": canonical,
                    "status": "not_allowed",
                }));
                continue;
            }

            let configured_load_mode =
                self.configured_tool_load_mode(&canonical, &direct_overrides);
            let already_available = configured_load_mode == ToolLoadMode::Direct
                || (native_lazy && loaded_guard.contains(&canonical));
            let loaded = native_lazy && !already_available;
            if loaded {
                loaded_guard.insert(canonical.clone());
            }

            let mut item = serde_json::Map::new();
            item.insert("name".to_string(), serde_json::json!(canonical.clone()));
            item.insert(
                "loadMode".to_string(),
                serde_json::json!(match configured_load_mode {
                    ToolLoadMode::Direct => "direct",
                    ToolLoadMode::Lazy => "lazy",
                    ToolLoadMode::Skill => "skill",
                }),
            );
            item.insert(
                "status".to_string(),
                serde_json::json!(if native_lazy { "loaded" } else { "described" }),
            );
            item.insert("loaded".to_string(), serde_json::json!(loaded));
            item.insert(
                "alreadyAvailable".to_string(),
                serde_json::json!(already_available),
            );

            if !native_lazy {
                let Some((description, parameters)) =
                    self.tool_registry.tool_description(&canonical)
                else {
                    items.push(serde_json::json!({
                        "requested": requested_name,
                        "name": canonical,
                        "status": "unknown_tool",
                    }));
                    continue;
                };
                item.insert("description".to_string(), serde_json::json!(description));
                item.insert("parameters".to_string(), parameters.clone());
                item.insert("callWith".to_string(), serde_json::json!("tool_call"));
            }

            items.push(serde_json::Value::Object(item));
        }
        drop(loaded_guard);

        eprintln!(
            "[Agent {}] tool_load mode={} requested={} loaded_now={}",
            self.id,
            if native_lazy {
                "native_lazy"
            } else {
                "meta_call"
            },
            items.len(),
            items
                .iter()
                .filter(|item| item
                    .get("loaded")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false))
                .count()
        );

        ToolResult {
            output: serde_json::to_string_pretty(&serde_json::json!({
                "mode": if native_lazy { "native_lazy" } else { "meta_call" },
                "tools": items,
            }))
            .unwrap_or_else(|_| "{\"tools\":[]}".to_string()),
            is_error: false,
        }
    }

    fn execute_todowrite(
        &self,
        store: &SessionStore,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ToolResult {
        let todos_value = match args.get("todos") {
            Some(v) => v,
            None => {
                return ToolResult {
                    output: "Error: todowrite requires a 'todos' array parameter".to_string(),
                    is_error: true,
                };
            }
        };

        let todos_arr = match todos_value.as_array() {
            Some(arr) => arr,
            None => {
                return ToolResult {
                    output: "Error: 'todos' must be an array".to_string(),
                    is_error: true,
                };
            }
        };

        let mut items: Vec<TodoItem> = Vec::new();
        for item in todos_arr {
            let content = item
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string();
            let priority = item
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string();

            if content.is_empty() {
                continue;
            }

            items.push(TodoItem {
                content,
                status,
                priority,
            });
        }

        match store.update_todos(&self.session_id, Some(run_id), &items) {
            Ok(()) => {
                let pending_count = items
                    .iter()
                    .filter(|t| t.status != "completed" && t.status != "cancelled")
                    .count();
                let output =
                    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string());
                ToolResult {
                    output: format!(
                        "{} todos ({} remaining)\n{}",
                        items.len(),
                        pending_count,
                        output
                    ),
                    is_error: false,
                }
            }
            Err(e) => ToolResult {
                output: format!("Error updating todos: {}", e),
                is_error: true,
            },
        }
    }

    pub(super) async fn clear_pending_knowledge_proposal(&self, app_handle: &AppHandle) {
        let drafts: tauri::State<crate::KnowledgeProposalDraftStore> = app_handle.state();
        let mut draft_store = drafts.lock().await;
        draft_store.remove(&self.session_id);
    }

    pub(super) async fn flush_pending_knowledge_proposal(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        run_id: &str,
    ) -> Result<Option<crate::session::models::ChatMessage>, String> {
        let drafts: tauri::State<crate::KnowledgeProposalDraftStore> = app_handle.state();
        let staged = {
            let mut draft_store = drafts.lock().await;
            match draft_store.get(&self.session_id) {
                Some(entry) if entry.run_id == run_id => draft_store.remove(&self.session_id),
                _ => None,
            }
        };

        let Some(staged) = staged else {
            return Ok(None);
        };

        let _message_id =
            store.add_knowledge_proposal_message(&self.session_id, &staged.proposal)?;
        let message = store
            .get_knowledge_proposal_message(&self.session_id, &staged.proposal.proposal_id)?
            .ok_or_else(|| "Knowledge proposal message was not found after insert".to_string())?;

        emit_stream(
            app_handle,
            run_id,
            StreamEvent::KnowledgeProposal {
                session_id: self.session_id.clone(),
                message: message.clone(),
            },
        );
        Ok(Some(message))
    }

    // ─── config_query ────────────────────────────────────────────────────────

    fn execute_config_query(&self, app_handle: &AppHandle, args: &serde_json::Value) -> ToolResult {
        let category = args.get("category").and_then(|v| v.as_str());

        let entries = match category {
            Some(cat) => crate::config_registry::collect_by_category(app_handle, cat),
            None => crate::config_registry::collect_all(app_handle),
        };

        match entries {
            Ok(items) => {
                let mut out = String::new();
                let mut current_cat = String::new();
                for e in &items {
                    if e.category != current_cat {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&format!("## {}\n\n", e.category));
                        current_cat = e.category.clone();
                    }
                    out.push_str(&format!(
                        "**{}** (`{}`)\n  {}\n  Storage: `{}`\n  Value: {}\n\n",
                        e.label, e.key, e.description, e.storage, e.current_value
                    ));
                }
                if out.is_empty() {
                    out = "No configuration entries found.".to_string();
                }
                ToolResult {
                    output: out,
                    is_error: false,
                }
            }
            Err(err) => ToolResult {
                output: format!("Error querying config: {}", err.message),
                is_error: true,
            },
        }
    }

    fn execute_knowledge_list(&self, args: &serde_json::Value) -> ToolResult {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct KnowledgeListArgs {
            path_prefix: Option<String>,
        }

        let parsed = match serde_json::from_value::<KnowledgeListArgs>(args.clone()) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_list arguments: {}", error),
                    is_error: true,
                };
            }
        };

        if let Err(error) =
            crate::knowledge_store::ensure_memory_builtin_documents(&self.working_dir)
        {
            return ToolResult {
                output: format!("Error preparing memory documents: {}", error),
                is_error: true,
            };
        }

        let (resolved_type, resolved_prefix) = match crate::commands::resolve_knowledge_path_filter(
            None,
            parsed.path_prefix.as_deref(),
        ) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_list pathPrefix: {}", error),
                    is_error: true,
                };
            }
        };

        match crate::knowledge_store::list_documents_with_app_root(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            resolved_type,
            resolved_prefix.as_deref(),
        ) {
            Ok(mut items) => {
                Self::prefix_knowledge_list_item_paths(&mut items);
                let items = Self::sanitize_knowledge_list_items(items);
                ToolResult {
                    output: Self::format_knowledge_list_output(&items),
                    is_error: false,
                }
            }
            Err(error) => ToolResult {
                output: format!("Error listing knowledge documents: {}", error),
                is_error: true,
            },
        }
    }

    // ─── knowledge_query ─────────────────────────────────────────────────────

    async fn execute_knowledge_query(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct KnowledgeQueryArgs {
            query: Option<String>,
            lexical_query: Option<String>,
            semantic_query: Option<String>,
            limit: Option<usize>,
            path_prefix: Option<String>,
        }

        let parsed = match serde_json::from_value::<KnowledgeQueryArgs>(args.clone()) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_query arguments: {}", error),
                    is_error: true,
                };
            }
        };

        let lexical_query = parsed
            .lexical_query
            .or_else(|| parsed.query.clone())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let semantic_query = parsed
            .semantic_query
            .or(parsed.query)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if lexical_query.is_none() && semantic_query.is_none() {
            return ToolResult {
                output: "Error: 'lexicalQuery' or 'semanticQuery' parameter is required."
                    .to_string(),
                is_error: true,
            };
        }

        let (prefix_type, normalized_prefix) = match crate::commands::resolve_knowledge_path_filter(
            None,
            parsed.path_prefix.as_deref(),
        ) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_query pathPrefix: {}", error),
                    is_error: true,
                };
            }
        };

        let mut parsed_types: Option<Vec<crate::knowledge_store::KnowledgeType>> = None;
        if let Some(prefix_type) = prefix_type {
            parsed_types = Some(vec![prefix_type]);
        }

        let knowledge_index_state = {
            let state: tauri::State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>> =
                app_handle.state();
            state.inner().clone()
        };

        match crate::knowledge_index::query_documents(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            lexical_query.as_deref(),
            semantic_query.as_deref(),
            parsed_types.as_deref(),
            normalized_prefix.as_deref(),
            parsed.limit.unwrap_or(5).min(20),
            knowledge_index_state,
        )
        .await
        {
            Ok(mut items) => {
                Self::prefix_knowledge_search_hit_paths(&mut items);
                let items = Self::sanitize_knowledge_search_hits(items);
                ToolResult {
                    output: Self::format_knowledge_query_output(&items),
                    is_error: false,
                }
            }
            Err(error) => ToolResult {
                output: format!("Error querying knowledge documents: {}", error),
                is_error: true,
            },
        }
    }

    // ─── knowledge_read ──────────────────────────────────────────────────────

    fn execute_knowledge_read(&self, args: &serde_json::Value) -> ToolResult {
        let parsed = match serde_json::from_value::<AgentKnowledgeReadArgs>(args.clone()) {
            Ok(value) if !value.path.trim().is_empty() => value,
            Ok(_) => {
                return ToolResult {
                    output: "Error: 'path' parameter is required.".to_string(),
                    is_error: true,
                };
            }
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_read arguments: {}", error),
                    is_error: true,
                };
            }
        };

        let request = crate::knowledge_store::KnowledgeReadRequest {
            kind: crate::knowledge_store::KnowledgeTargetKind::Document,
            path: parsed.path,
            doc_type: None,
            part: parsed.part,
        };

        match crate::commands::execute_knowledge_read_request(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            request.clone(),
        ) {
            Ok(mut result) => {
                Self::prefix_knowledge_read_response_paths(&mut result);
                let sanitized = match Self::sanitize_knowledge_read_response(result) {
                    Ok(value) => value,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Error reading knowledge document: {}", error),
                            is_error: true,
                        };
                    }
                };
                ToolResult {
                    output: Self::format_knowledge_read_output(&sanitized),
                    is_error: false,
                }
            }
            Err(error) => ToolResult {
                output: format!("Error reading knowledge document: {}", error),
                is_error: true,
            },
        }
    }

    async fn reconcile_knowledge_workspace_with_source(
        &self,
        app_handle: &AppHandle,
        source: &str,
    ) -> Result<(), String> {
        let knowledge_index_state = {
            let state: tauri::State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>> =
                app_handle.state();
            state.inner().clone()
        };
        crate::commands::reconcile_and_emit_knowledge_changed(
            app_handle,
            &self.working_dir,
            knowledge_index_state,
            source,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn reconcile_knowledge_workspace(&self, app_handle: &AppHandle) -> Result<(), String> {
        self.reconcile_knowledge_workspace_with_source(app_handle, "agent_knowledge_tool")
            .await
    }

    async fn execute_knowledge_create(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed = match serde_json::from_value::<AgentKnowledgeCreateArgs>(args.clone()) {
            Ok(value) if !value.path.trim().is_empty() => value,
            Ok(_) => {
                return ToolResult {
                    output: "Error: 'path' parameter is required.".to_string(),
                    is_error: true,
                };
            }
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_create arguments: {}", error),
                    is_error: true,
                };
            }
        };

        if parsed.kind == crate::knowledge_store::KnowledgeTargetKind::Directory
            && parsed
                .document
                .as_ref()
                .is_some_and(|patch| !patch.is_noop_for_create())
        {
            return ToolResult {
                output: "Error: knowledge_create for directories does not accept document content."
                    .to_string(),
                is_error: true,
            };
        }

        let request = crate::knowledge_store::KnowledgeCreateRequest {
            kind: parsed.kind,
            path: parsed.path,
            doc_type: None,
            document: parsed
                .document
                .filter(|patch| !patch.is_noop_for_create())
                .map(AgentKnowledgeDocumentContentPatch::into_document_patch),
        };

        match crate::commands::execute_knowledge_create_request(&self.working_dir, request) {
            Ok(mut result) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => {
                    Self::prefix_knowledge_mutation_response_paths(&mut result);
                    let sanitized = Self::sanitize_knowledge_mutation_response(result);
                    ToolResult {
                        output: Self::format_knowledge_mutation_output("Created", &sanitized),
                        is_error: false,
                    }
                }
                Err(error) => ToolResult {
                    output: format!("Error reconciling knowledge index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error creating knowledge entry: {}", error),
                is_error: true,
            },
        }
    }

    async fn execute_knowledge_edit(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed = match serde_json::from_value::<AgentKnowledgeEditArgs>(args.clone()) {
            Ok(value) if !value.path.trim().is_empty() => value,
            Ok(_) => {
                return ToolResult {
                    output: "Error: 'path' parameter is required.".to_string(),
                    is_error: true,
                };
            }
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_edit arguments: {}", error),
                    is_error: true,
                };
            }
        };

        if parsed.document.is_empty() {
            return ToolResult {
                output: "Error: knowledge_edit requires at least one document content field."
                    .to_string(),
                is_error: true,
            };
        }

        let request = crate::knowledge_store::KnowledgeEditRequest {
            kind: crate::knowledge_store::KnowledgeTargetKind::Document,
            path: parsed.path,
            doc_type: None,
            document: Some(parsed.document.into_document_patch()),
            config: None,
        };

        match crate::commands::execute_knowledge_edit_request(&self.working_dir, request) {
            Ok(mut result) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => {
                    Self::prefix_knowledge_mutation_response_paths(&mut result);
                    let sanitized = Self::sanitize_knowledge_mutation_response(result);
                    ToolResult {
                        output: Self::format_knowledge_mutation_output("Edited", &sanitized),
                        is_error: false,
                    }
                }
                Err(error) => ToolResult {
                    output: format!("Error reconciling knowledge index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error editing knowledge entry: {}", error),
                is_error: true,
            },
        }
    }

    async fn execute_knowledge_move(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed = match serde_json::from_value::<crate::knowledge_store::KnowledgeMoveRequest>(
            args.clone(),
        ) {
            Ok(value) if !value.path.trim().is_empty() => value,
            Ok(_) => {
                return ToolResult {
                    output: "Error: 'path' parameter is required.".to_string(),
                    is_error: true,
                };
            }
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_move arguments: {}", error),
                    is_error: true,
                };
            }
        };

        match crate::commands::execute_knowledge_move_request(&self.working_dir, parsed) {
            Ok(mut result) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => {
                    Self::prefix_knowledge_mutation_response_paths(&mut result);
                    let sanitized = Self::sanitize_knowledge_mutation_response(result);
                    ToolResult {
                        output: Self::format_knowledge_mutation_output("Moved", &sanitized),
                        is_error: false,
                    }
                }
                Err(error) => ToolResult {
                    output: format!("Error reconciling knowledge index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error moving knowledge entry: {}", error),
                is_error: true,
            },
        }
    }

    async fn execute_knowledge_delete(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed = match serde_json::from_value::<crate::knowledge_store::KnowledgeDeleteRequest>(
            args.clone(),
        ) {
            Ok(value) if !value.path.trim().is_empty() => value,
            Ok(_) => {
                return ToolResult {
                    output: "Error: 'path' parameter is required.".to_string(),
                    is_error: true,
                };
            }
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing knowledge_delete arguments: {}", error),
                    is_error: true,
                };
            }
        };

        match crate::commands::execute_knowledge_delete_request(&self.working_dir, parsed) {
            Ok(mut result) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => {
                    Self::prefix_knowledge_mutation_response_paths(&mut result);
                    let sanitized = Self::sanitize_knowledge_mutation_response(result);
                    ToolResult {
                        output: Self::format_knowledge_mutation_output("Deleted", &sanitized),
                        is_error: false,
                    }
                }
                Err(error) => ToolResult {
                    output: format!("Error reconciling knowledge index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error deleting knowledge entry: {}", error),
                is_error: true,
            },
        }
    }

    fn format_skill_manifest_line(skill: &crate::commands::SkillManifest) -> String {
        let kind = match skill.kind {
            crate::commands::SkillManifestKind::Document => "document",
            crate::commands::SkillManifestKind::Package => "package",
        };
        let command = if skill.command_trigger.trim().is_empty() {
            "<none>"
        } else {
            skill.command_trigger.trim()
        };
        format!(
            "{} {} {} | command={} | path={} | name={}",
            skill.source, kind, skill.dir_name, command, skill.rel_path, skill.name
        )
    }

    fn format_skill_manifest_detail(
        action: &str,
        skill: &crate::commands::SkillManifest,
    ) -> String {
        format!(
            "{} Skill\n{}",
            action,
            Self::format_skill_manifest_line(skill)
        )
    }

    fn format_skill_manifest_detail_with_package_root(
        action: &str,
        skill: &crate::commands::SkillManifest,
    ) -> String {
        let mut output = Self::format_skill_manifest_detail(action, skill);
        if let Some(package_id) = skill.package_id.as_deref() {
            if let Ok(root) = crate::commands::resolve_skill_package_root_sync(package_id) {
                output.push_str("\npackageRoot=");
                output.push_str(&root.to_string_lossy().replace('\\', "/"));
            }
        }
        output
    }

    fn format_skill_manifest_list(skills: &[crate::commands::SkillManifest]) -> String {
        if skills.is_empty() {
            return "(empty)".to_string();
        }
        skills
            .iter()
            .map(Self::format_skill_manifest_line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn execute_skill_create(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed =
            match serde_json::from_value::<crate::commands::SkillCreateRequest>(args.clone()) {
                Ok(value) if !value.name.trim().is_empty() => value,
                Ok(_) => {
                    return ToolResult {
                        output: "Error: 'name' parameter is required.".to_string(),
                        is_error: true,
                    };
                }
                Err(error) => {
                    return ToolResult {
                        output: format!("Error parsing skill_create arguments: {}", error),
                        is_error: true,
                    };
                }
            };

        if args
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return ToolResult {
                output: "Error: 'kind' parameter is required.".to_string(),
                is_error: true,
            };
        }

        match crate::commands::create_skill_sync(&self.working_dir, parsed) {
            Ok(skill) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => ToolResult {
                    output: Self::format_skill_manifest_detail_with_package_root("Created", &skill),
                    is_error: false,
                },
                Err(error) => ToolResult {
                    output: format!("Error reconciling skill index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error creating Skill: {}", error),
                is_error: true,
            },
        }
    }

    async fn execute_skill_reload(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        let parsed =
            match serde_json::from_value::<crate::commands::SkillReloadRequest>(args.clone()) {
                Ok(value) if !value.name.trim().is_empty() => value,
                Ok(_) => {
                    return ToolResult {
                        output: "Error: 'name' parameter is required.".to_string(),
                        is_error: true,
                    };
                }
                Err(error) => {
                    return ToolResult {
                        output: format!("Error parsing skill_reload arguments: {}", error),
                        is_error: true,
                    };
                }
            };

        match crate::commands::reload_skill_manifest_sync(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            parsed,
        ) {
            Ok(skill) => match self.reconcile_knowledge_workspace(app_handle).await {
                Ok(()) => ToolResult {
                    output: Self::format_skill_manifest_detail_with_package_root("Loaded", &skill),
                    is_error: false,
                },
                Err(error) => ToolResult {
                    output: format!("Error reconciling skill index: {}", error),
                    is_error: true,
                },
            },
            Err(error) => ToolResult {
                output: format!("Error loading Skill: {}", error),
                is_error: true,
            },
        }
    }

    fn execute_skill_list(&self, args: &serde_json::Value) -> ToolResult {
        let parsed = match serde_json::from_value::<AgentSkillListArgs>(args.clone()) {
            Ok(value) => value,
            Err(error) => {
                return ToolResult {
                    output: format!("Error parsing skill_list arguments: {}", error),
                    is_error: true,
                };
            }
        };

        match crate::commands::list_skills_filtered_sync(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            parsed.source.as_deref(),
        ) {
            Ok(skills) => ToolResult {
                output: Self::format_skill_manifest_list(&skills),
                is_error: false,
            },
            Err(error) => ToolResult {
                output: format!("Error listing Skills: {}", error),
                is_error: true,
            },
        }
    }

    async fn execute_graph_view(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        let request = match crate::commands::agent_graph_tool_request_from_args(args, tool_call_id)
        {
            Ok(request) => request,
            Err(error) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!("Error parsing graph_view arguments: {}", error),
                    is_error: true,
                });
            }
        };
        let request_id = request.request_id.clone();
        let editable = request.editable;
        let (tx, rx) = if editable {
            let (tx, rx) = tokio::sync::oneshot::channel::<crate::commands::AgentGraphToolAnswer>();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let graph_store: tauri::State<'_, crate::commands::AgentGraphToolStore> =
            app_handle.state();
        let graph_store = graph_store.inner().clone();
        crate::commands::insert_agent_graph_tool_request(&graph_store, request.clone(), tx).await;

        let open_result = match crate::commands::open_agent_graph_tool_window(app_handle, &request)
        {
            Ok(result) => result,
            Err(error) => {
                let _ = crate::commands::remove_agent_graph_tool_request(&graph_store, &request_id)
                    .await;
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: error,
                    is_error: true,
                });
            }
        };

        if !editable {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: serde_json::to_string_pretty(&serde_json::json!({
                    "status": "opened",
                    "requestId": open_result.request_id,
                    "windowLabel": open_result.window_label,
                    "hostUrl": open_result.host_url,
                    "editable": false,
                }))
                .unwrap_or_else(|_| "Graph window opened.".to_string()),
                is_error: false,
            });
        }

        eprintln!(
            "[Agent {}] graph_view: waiting for editable graph response (request_id={})",
            self.id, request_id
        );

        let mut cancel_rx = self.cancel_waiter();
        let Some(rx) = rx else {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "Internal error: editable graph_view missing receiver.".to_string(),
                is_error: true,
            });
        };
        let answer_result = tokio::select! {
            result = rx => Some(result),
            _ = cancel_rx.changed() => None,
        };

        match answer_result {
            Some(Ok(crate::commands::AgentGraphToolAnswer::Submitted(answer))) => {
                eprintln!(
                    "[Agent {}] graph_view: user submitted graph (request_id={})",
                    self.id, request_id
                );
                ExecutedToolResult::from_tool_result(ToolResult {
                    output: serde_json::to_string_pretty(&serde_json::json!({
                        "status": "submitted",
                        "requestId": answer.request_id,
                        "option": answer.option,
                        "graph": answer.graph,
                    }))
                    .unwrap_or_else(|_| "Graph submitted.".to_string()),
                    is_error: false,
                })
            }
            Some(Ok(crate::commands::AgentGraphToolAnswer::Cancelled)) => {
                eprintln!(
                    "[Agent {}] graph_view: graph window cancelled (request_id={})",
                    self.id, request_id
                );
                ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Graph editing was cancelled before confirmation.".to_string(),
                    is_error: true,
                })
            }
            Some(Err(_)) => ExecutedToolResult::from_tool_result(ToolResult {
                output: "Graph response channel was closed.".to_string(),
                is_error: true,
            }),
            None => {
                let _ = crate::commands::cancel_agent_graph_tool_request_by_id(
                    &graph_store,
                    &request_id,
                )
                .await;
                crate::commands::close_agent_graph_tool_window(app_handle, &request_id);
                Self::interrupted_tool_result()
            }
        }
    }

    fn resolve_subagent_model_name(&self, subagent_type: &str) -> Option<String> {
        self.registry.get(subagent_type)?;
        match self.subagent_model_overrides.get(subagent_type) {
            Some(override_model) if !override_model.is_empty() => Some(override_model.clone()),
            _ => Some(self.effective_model.clone()),
        }
    }

    async fn execute_ask(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ToolResult {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if question.is_empty() {
            return ToolResult {
                output: "Missing required parameter: question".to_string(),
                is_error: true,
            };
        }

        let options: Vec<crate::commands::AskOption> = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| crate::commands::AskOption {
                        label: item
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        description: item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .filter(|o| !o.label.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        if options.is_empty() {
            return ToolResult {
                output: "At least one option is required".to_string(),
                is_error: true,
            };
        }

        let question_id = uuid::Uuid::new_v4().to_string();

        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let wait_target = self.user_wait_target(run_id);

        {
            let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
            let mut store = question_store.lock().await;
            store.insert(
                question_id.clone(),
                crate::PendingQuestionResponse {
                    session_id: wait_target.session_id.clone(),
                    run_id: wait_target.run_id.clone(),
                    tx,
                },
            );
        }

        emit_stream(
            app_handle,
            &wait_target.run_id,
            crate::commands::StreamEvent::AskUser {
                session_id: wait_target.session_id.clone(),
                question_id: question_id.clone(),
                tool_call_id: tool_call_id.to_string(),
                question: question.clone(),
                options: options.clone(),
            },
        );

        eprintln!(
            "[Agent {}] ask tool: waiting for user answer (question_id={})",
            self.id, question_id
        );

        let mut cancel_rx = self.cancel_waiter();
        let answer_result = tokio::select! {
            result = rx => Some(result),
            _ = cancel_rx.changed() => None,
        };

        match answer_result {
            Some(Ok(answer)) => {
                eprintln!("[Agent {}] ask tool: got user answer: {}", self.id, answer);
                ToolResult {
                    output: format!("User answered: {}", answer),
                    is_error: false,
                }
            }
            Some(Err(_)) => ToolResult {
                output: "Question was cancelled".to_string(),
                is_error: true,
            },
            None => {
                let question_store: tauri::State<crate::QuestionStore> = app_handle.state();
                let mut store = question_store.lock().await;
                store.remove(&question_id);
                Self::interrupted_tool_result().into_tool_result()
            }
        }
    }

    async fn execute_unity_execute(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ExecutedToolResult {
        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        let code = match args.get("code").and_then(|value| value.as_str()) {
            Some(code) => code,
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: code".to_string(),
                    is_error: true,
                });
            }
        };

        let requested_status = match args
            .get("request_editor_status")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(status) => status,
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: request_editor_status".to_string(),
                    is_error: true,
                });
            }
        };

        if requested_status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
            || !crate::unity_bridge::is_known_editor_status(requested_status)
        {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
                    requested_status
                ),
                is_error: true,
            });
        }

        if !self.has_selected_working_dir() {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "Tool 'unity_execute' requires a selected Unity project working directory."
                    .to_string(),
                is_error: true,
            });
        }

        let (connected, current_status, _scene) =
            crate::unity_bridge::query_unity_status(&self.working_dir).await;
        if !connected {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "Unity Editor not connected".to_string(),
                is_error: true,
            });
        }

        if current_status != requested_status {
            match self
                .request_unity_editor_status_change_confirm(
                    app_handle,
                    "unity_execute",
                    tool_call_id,
                    current_status,
                    requested_status,
                    run_id,
                )
                .await
            {
                ToolConfirmDecision::Allow => {}
                ToolConfirmDecision::Deny { feedback } => {
                    if self.is_cancel_requested() {
                        return Self::interrupted_tool_result();
                    }
                    let output = match feedback {
                        Some(feedback) => format!(
                            "Unity Editor status change was rejected by user feedback.\nUser feedback: {}",
                            feedback
                        ),
                        None => "user_denied_editor_state_change".to_string(),
                    };
                    return ExecutedToolResult::from_tool_result(ToolResult {
                        output,
                        is_error: true,
                    });
                }
            }

            if let Err(error) =
                crate::unity_bridge::set_editor_status(&self.working_dir, requested_status).await
            {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!("Failed to change Unity Editor status: {}", error),
                    is_error: true,
                });
            }
        }

        emit_tool_progress(
            app_handle,
            run_id,
            &self.session_id,
            tool_call_id,
            "Waiting for Unity execute slot",
            "",
            None,
            "running",
        );

        let progress_handle = app_handle.clone();
        let result_handle = app_handle.clone();
        let session_id = self.session_id.clone();
        let result_session_id = self.session_id.clone();
        let tool_call_id = tool_call_id.to_string();
        let result_tool_call_id = tool_call_id.clone();
        let progress_run_id = run_id.to_string();
        let result_run_id = progress_run_id.clone();

        let cancel_rx = self.cancel_waiter();
        match crate::unity_bridge::unity_execute_code_with_progress_cancellable(
            &self.working_dir,
            code,
            cancel_rx,
            move |snapshot| {
                if !snapshot.active {
                    return;
                }
                let progress = if snapshot.source == "api" {
                    Some(snapshot.progress)
                } else {
                    None
                };
                emit_tool_progress(
                    &progress_handle,
                    &progress_run_id,
                    &session_id,
                    &tool_call_id,
                    snapshot.title,
                    snapshot.info,
                    progress,
                    "running",
                );
            },
        )
        .await
        {
            Ok(output) => {
                let trimmed = output.trim();
                ExecutedToolResult::from_tool_result(ToolResult {
                    output: if trimmed.is_empty() {
                        "Code executed successfully (no output).".to_string()
                    } else {
                        trimmed.to_string()
                    },
                    is_error: false,
                })
            }
            Err(error) if error == crate::unity_bridge::UNITY_EXECUTE_CANCELLED => {
                Self::interrupted_tool_result()
            }
            Err(error) => {
                let title = if error.contains("compilation")
                    || error.contains("Compilation")
                    || error.contains("compile")
                {
                    "Compilation failed"
                } else {
                    "Execution failed"
                };
                emit_tool_progress(
                    &result_handle,
                    &result_run_id,
                    &result_session_id,
                    &result_tool_call_id,
                    title,
                    "",
                    None,
                    "error",
                );
                ExecutedToolResult::from_tool_result(ToolResult {
                    output: error,
                    is_error: true,
                })
            }
        }
    }

    async fn execute_unity_recompile(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        _args: &serde_json::Value,
        run_id: &str,
    ) -> ToolResult {
        let (connected, status, _) =
            crate::unity_bridge::query_unity_status(&self.working_dir).await;

        if !connected {
            return ToolResult {
                output: "Unity Editor not connected".to_string(),
                is_error: true,
            };
        }

        if crate::unity_bridge::is_play_mode_status(status) {
            match self
                .request_unity_editor_status_change_confirm(
                    app_handle,
                    "unity_recompile",
                    tool_call_id,
                    status,
                    crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING,
                    run_id,
                )
                .await
            {
                ToolConfirmDecision::Allow => {}
                ToolConfirmDecision::Deny { feedback } => {
                    let is_error = feedback.is_some();
                    let output = match feedback {
                        Some(feedback) => format!(
                            "Unity Editor status change was rejected by user feedback.\nUser feedback: {}",
                            feedback
                        ),
                        None => "User cancelled compilation".to_string(),
                    };
                    return ToolResult { output, is_error };
                }
            }

            if let Err(e) = crate::unity_bridge::exit_play_mode(&self.working_dir).await {
                return ToolResult {
                    output: format!("Failed to exit play mode: {}", e),
                    is_error: true,
                };
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        match crate::unity_bridge::recompile_and_wait(&self.working_dir).await {
            Ok(msg) => ToolResult {
                output: msg,
                is_error: false,
            },
            Err(e) => ToolResult {
                output: format!("Compilation failed:\n{}", e),
                is_error: true,
            },
        }
    }

    async fn execute_unity_run_states(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ToolResult {
        if !self.has_selected_working_dir() {
            return ToolResult {
                output: "unity_run_states requires a selected Unity project working directory."
                    .to_string(),
                is_error: true,
            };
        }

        let requested_status = match args
            .get("request_editor_status")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(status) => status,
            None => {
                return ToolResult {
                    output: "Missing required parameter: request_editor_status".to_string(),
                    is_error: true,
                };
            }
        };

        if requested_status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
            || !crate::unity_bridge::is_known_editor_status(requested_status)
        {
            return ToolResult {
                output: format!(
                    "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
                    requested_status
                ),
                is_error: true,
            };
        }

        let (connected, _current_status, _) =
            crate::unity_bridge::query_unity_status(&self.working_dir).await;
        if !connected {
            return ToolResult {
                output: "Unity Editor not connected".to_string(),
                is_error: true,
            };
        }

        emit_tool_progress(
            app_handle,
            run_id,
            &self.session_id,
            tool_call_id,
            "Compiling states",
            "",
            None,
            "running",
        );

        if let Err(error) = crate::unity_bridge::compile_run_states(&self.working_dir, args).await {
            emit_tool_progress(
                app_handle,
                run_id,
                &self.session_id,
                tool_call_id,
                "Compilation failed",
                "",
                None,
                "error",
            );
            return ToolResult {
                output: error,
                is_error: true,
            };
        }

        let (connected, current_status, _) =
            crate::unity_bridge::query_unity_status(&self.working_dir).await;
        if !connected {
            return ToolResult {
                output: "Unity Editor not connected".to_string(),
                is_error: true,
            };
        }

        if current_status != requested_status {
            emit_tool_progress(
                app_handle,
                run_id,
                &self.session_id,
                tool_call_id,
                "Changing editor status",
                format!("{} -> {}", current_status, requested_status),
                None,
                "running",
            );
            match self
                .request_unity_editor_status_change_confirm(
                    app_handle,
                    "unity_run_states",
                    tool_call_id,
                    &current_status,
                    requested_status,
                    run_id,
                )
                .await
            {
                ToolConfirmDecision::Allow => {}
                ToolConfirmDecision::Deny { feedback } => {
                    let output = match feedback {
                        Some(feedback) => format!(
                            "Unity Editor status change was rejected by user feedback.\nUser feedback: {}",
                            feedback
                        ),
                        None => "user_denied_editor_state_change".to_string(),
                    };
                    return ToolResult {
                        output,
                        is_error: true,
                    };
                }
            }

            if let Err(error) =
                crate::unity_bridge::set_editor_status(&self.working_dir, requested_status).await
            {
                emit_tool_progress(
                    app_handle,
                    run_id,
                    &self.session_id,
                    tool_call_id,
                    "Editor status change failed",
                    "",
                    None,
                    "error",
                );
                return ToolResult {
                    output: format!("Failed to change Unity Editor status: {}", error),
                    is_error: true,
                };
            }
        }

        emit_tool_progress(
            app_handle,
            run_id,
            &self.session_id,
            tool_call_id,
            "Running state machine",
            requested_status,
            None,
            "running",
        );

        match crate::unity_bridge::unity_run_states(&self.working_dir, args).await {
            Ok(output) => ToolResult {
                output: if output.trim().is_empty() {
                    "unity_run_states completed with no output.".to_string()
                } else {
                    output
                },
                is_error: false,
            },
            Err(error) => {
                emit_tool_progress(
                    app_handle,
                    run_id,
                    &self.session_id,
                    tool_call_id,
                    "Runtime failed",
                    "",
                    None,
                    "error",
                );
                ToolResult {
                    output: error,
                    is_error: true,
                }
            }
        }
    }

    fn execute_unity_ref_search(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        use crate::asset_db::types::{guid_to_hex, AssetKind};
        use crate::asset_db::AssetDbState;

        let asset_path = match args.get("asset_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult {
                    output: "Missing required parameter: asset_path".to_string(),
                    is_error: true,
                };
            }
        };

        let direction = match args.get("direction").and_then(|v| v.as_str()) {
            Some(d @ ("references" | "dependencies")) => d.to_string(),
            Some(other) => {
                return ToolResult {
                    output: format!(
                        "Invalid direction '{}'. Must be 'references' or 'dependencies'.",
                        other
                    ),
                    is_error: true,
                };
            }
            None => {
                return ToolResult {
                    output: "Missing required parameter: direction (must be 'references' or 'dependencies')".to_string(),
                    is_error: true,
                };
            }
        };

        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(10) as u32;

        let type_filter: Option<Vec<AssetKind>> = match args
            .get("type_filter")
            .and_then(|v| v.as_str())
        {
            Some(filter_str) => {
                let mut kinds = Vec::new();
                for part in filter_str.split('|') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    match Self::parse_asset_kind(part) {
                        Some(k) => kinds.push(k),
                        None => {
                            return ToolResult {
                                output: format!(
                                    "Unknown type '{}' in type_filter. Valid types: scene, prefab, material, animation, controller, genericAsset, script, texture, audio, shader, model, otherYaml, metaOnly",
                                    part
                                ),
                                is_error: true,
                            };
                        }
                    }
                }
                if kinds.is_empty() {
                    None
                } else {
                    Some(kinds)
                }
            }
            None => None,
        };

        let ref_graph_state: tauri::State<'_, AssetDbState> = match app_handle.try_state() {
            Some(s) => s,
            None => {
                return ToolResult {
                    output:
                        "AssetDbState not available. The reference graph has not been initialized."
                            .to_string(),
                    is_error: true,
                };
            }
        };

        let guard = match ref_graph_state.0.lock() {
            Ok(g) => g,
            Err(e) => {
                return ToolResult {
                    output: format!("Failed to lock AssetDb: {}", e),
                    is_error: true,
                };
            }
        };

        let graph = match guard.as_ref() {
            Some(g) => g,
            None => {
                return ToolResult {
                    output: "AssetDb not initialized. Please run a reference graph scan first (use the scan button in the UI).".to_string(),
                    is_error: true,
                };
            }
        };

        let guid = match graph.resolve_guid_by_path(&asset_path) {
            Ok(Some(g)) => g,
            Ok(None) => {
                return ToolResult {
                    output: format!("Asset not found in reference graph: '{}'. Make sure the path is relative to the project root (e.g. 'Assets/Scenes/Main.unity').", asset_path),
                    is_error: true,
                };
            }
            Err(e) => {
                return ToolResult {
                    output: format!("Failed to resolve GUID for '{}': {}", asset_path, e),
                    is_error: true,
                };
            }
        };

        let is_refs = direction == "references";

        if max_depth > 1 {
            let guids = if is_refs {
                graph.walk_refs(&guid, max_depth)
            } else {
                graph.walk_deps(&guid, max_depth)
            };

            let guids = match guids {
                Ok(g) => g,
                Err(e) => {
                    return ToolResult {
                        output: format!("Failed to walk graph: {}", e),
                        is_error: true,
                    };
                }
            };

            if guids.is_empty() {
                let label = if is_refs {
                    "references to"
                } else {
                    "dependencies of"
                };
                return ToolResult {
                    output: format!("No {} '{}' (depth {}).", label, asset_path, max_depth),
                    is_error: false,
                };
            }

            let mut paths: Vec<String> = guids
                .iter()
                .filter_map(|g| {
                    let (path, kind) = graph
                        .resolve_path_and_kind_by_guid(g)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| (guid_to_hex(g), AssetKind::OtherYaml));
                    if let Some(ref filter) = type_filter {
                        if !filter.contains(&kind) {
                            return None;
                        }
                    }
                    Some(path)
                })
                .collect();
            paths.sort();

            let label = if is_refs {
                "references to"
            } else {
                "dependencies of"
            };
            let filter_label = type_filter
                .as_ref()
                .map(|f| {
                    let names: Vec<&str> = f.iter().map(|k| k.camel_str()).collect();
                    format!(", type: {}", names.join("|"))
                })
                .unwrap_or_default();
            let mut out = format!(
                "{} {} '{}' (depth {}{}):\n",
                paths.len(),
                label,
                asset_path,
                max_depth,
                filter_label
            );
            for p in &paths {
                out.push_str("\n  ");
                out.push_str(p);
            }

            return ToolResult {
                output: out,
                is_error: false,
            };
        }

        let edges = if is_refs {
            graph.get_direct_refs(&guid)
        } else {
            graph.get_direct_deps(&guid)
        };

        let edges = match edges {
            Ok(e) => e,
            Err(e) => {
                return ToolResult {
                    output: format!("Failed to query graph: {}", e),
                    is_error: true,
                };
            }
        };

        if edges.is_empty() {
            let label = if is_refs {
                "references to"
            } else {
                "dependencies of"
            };
            return ToolResult {
                output: format!("No {} '{}'.", label, asset_path),
                is_error: false,
            };
        }

        struct GroupEntry {
            path: String,
            ref_paths: Vec<String>,
        }

        let mut groups: Vec<GroupEntry> = Vec::new();
        for e in &edges {
            let other_guid = if is_refs { &e.src_guid } else { &e.dst_guid };
            let (other_path, other_kind) = graph
                .resolve_path_and_kind_by_guid(other_guid)
                .ok()
                .flatten()
                .unwrap_or_else(|| (guid_to_hex(other_guid), AssetKind::OtherYaml));

            if let Some(ref filter) = type_filter {
                if !filter.contains(&other_kind) {
                    continue;
                }
            }
            let display = e
                .ref_path
                .as_deref()
                .or(e.field_hint.as_deref())
                .unwrap_or("-")
                .to_string();

            let group = match groups.iter_mut().find(|g| g.path == other_path) {
                Some(g) => g,
                None => {
                    groups.push(GroupEntry {
                        path: other_path,
                        ref_paths: Vec::new(),
                    });
                    groups.last_mut().unwrap()
                }
            };

            if !group.ref_paths.contains(&display) {
                group.ref_paths.push(display);
            }
        }

        let label = if is_refs {
            "references to"
        } else {
            "dependencies of"
        };
        let filter_label = type_filter
            .as_ref()
            .map(|f| {
                let names: Vec<&str> = f.iter().map(|k| k.camel_str()).collect();
                format!(", type: {}", names.join("|"))
            })
            .unwrap_or_default();
        let mut out = format!(
            "{} {} '{}'{filter_label}:\n",
            groups.len(),
            label,
            asset_path
        );
        for group in &groups {
            out.push('\n');
            out.push_str(&group.path);
            out.push('\n');
            for rp in &group.ref_paths {
                out.push_str("  ");
                out.push_str(rp);
                out.push('\n');
            }
        }

        ToolResult {
            output: out,
            is_error: false,
        }
    }

    fn parse_asset_kind(s: &str) -> Option<crate::asset_db::types::AssetKind> {
        use crate::asset_db::types::AssetKind;
        match s.to_lowercase().as_str() {
            "scene" => Some(AssetKind::Scene),
            "prefab" => Some(AssetKind::Prefab),
            "genericasset" | "scriptableobject" | "asset" => Some(AssetKind::GenericAsset),
            "material" | "mat" => Some(AssetKind::Material),
            "animation" | "anim" => Some(AssetKind::Animation),
            "animatorcontroller" | "controller" => Some(AssetKind::Controller),
            "otheryaml" => Some(AssetKind::OtherYaml),
            "metaonly" => Some(AssetKind::MetaOnly),
            "script" | "cs" => Some(AssetKind::Script),
            "texture" | "tex" | "image" => Some(AssetKind::Texture),
            "audio" | "sound" => Some(AssetKind::Audio),
            "shader" => Some(AssetKind::Shader),
            "model" | "mesh" | "fbx" => Some(AssetKind::Model),
            _ => None,
        }
    }

    fn parse_unity_yaml_summary_options(
        args: &serde_json::Value,
    ) -> crate::unity_yaml::HierarchySummaryOptions {
        fn positive_usize(args: &serde_json::Value, key: &str) -> Option<usize> {
            args.get(key)
                .and_then(|value| value.as_u64())
                .filter(|value| *value > 0)
                .map(|value| value as usize)
        }

        fn trimmed_string(args: &serde_json::Value, key: &str) -> Option<String> {
            args.get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        }

        fn push_component_filters(out: &mut Vec<String>, value: &str) {
            out.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(|entry| entry.to_string()),
            );
        }

        let mut component_filters = Vec::new();
        match args.get("component_filter") {
            Some(serde_json::Value::String(value)) => {
                push_component_filters(&mut component_filters, value);
            }
            Some(serde_json::Value::Array(values)) => {
                for value in values {
                    if let Some(text) = value.as_str() {
                        push_component_filters(&mut component_filters, text);
                    }
                }
            }
            _ => {}
        }

        crate::unity_yaml::HierarchySummaryOptions {
            max_depth: positive_usize(args, "max_depth"),
            max_nodes: positive_usize(args, "max_nodes"),
            query: trimmed_string(args, "query"),
            component_filters,
            path_prefix: trimmed_string(args, "path_prefix"),
        }
    }

    fn parse_unity_yaml_search_options(
        args: &serde_json::Value,
    ) -> crate::unity_yaml::HierarchySearchOptions {
        fn positive_usize(args: &serde_json::Value, key: &str) -> Option<usize> {
            args.get(key)
                .and_then(|value| value.as_u64())
                .filter(|value| *value > 0)
                .map(|value| value as usize)
        }

        fn trimmed_string(args: &serde_json::Value, key: &str) -> Option<String> {
            args.get(key)
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
        match args.get("component_filter") {
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
        match args.get("match_fields") {
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

        crate::unity_yaml::HierarchySearchOptions {
            query: trimmed_string(args, "query"),
            component_filters,
            match_fields,
            path_prefix: trimmed_string(args, "path_prefix"),
            limit: positive_usize(args, "limit"),
        }
    }

    fn unity_yaml_project_context<'a>(
        &self,
        app_handle: &'a AppHandle,
        file_path_arg: &str,
    ) -> (
        Option<tauri::State<'a, crate::asset_db::AssetDbState>>,
        Option<std::path::PathBuf>,
        std::path::PathBuf,
    ) {
        let ref_graph_state: Option<tauri::State<'_, crate::asset_db::AssetDbState>> =
            app_handle.try_state();

        let project_root: Option<std::path::PathBuf> = ref_graph_state
            .as_ref()
            .and_then(|s| s.0.lock().ok())
            .and_then(|g| g.as_ref().map(|rg| rg.project_root().to_path_buf()));

        let abs_path = if std::path::Path::new(file_path_arg).is_absolute() {
            std::path::PathBuf::from(file_path_arg)
        } else if let Some(ref root) = project_root {
            root.join(file_path_arg)
        } else {
            std::path::PathBuf::from(file_path_arg)
        };

        (ref_graph_state, project_root, abs_path)
    }

    fn read_unity_yaml_content(abs_path: &std::path::Path) -> Result<Vec<u8>, ToolResult> {
        std::fs::read(abs_path).map_err(|e| ToolResult {
            output: format!("Failed to read file '{}': {}", abs_path.display(), e),
            is_error: true,
        })
    }

    fn is_unity_yaml_content(content: &[u8]) -> bool {
        let header = String::from_utf8_lossy(&content[..content.len().min(128)]);
        header.contains("%YAML") || header.contains("!u!") || header.contains("--- !u!")
    }

    fn format_plain_text_excerpt(content: &[u8]) -> String {
        let text = String::from_utf8_lossy(content);
        let lines: Vec<&str> = text.lines().collect();
        let limit = 2000;
        let mut out = String::new();
        for (i, line) in lines.iter().take(limit).enumerate() {
            out.push_str(&format!("{:>5}\t{}\n", i + 1, line));
        }
        if lines.len() > limit {
            out.push_str(&format!("... ({} more lines)\n", lines.len() - limit));
        }
        out
    }

    fn unity_yaml_file_path_arg(args: &serde_json::Value) -> Result<String, ToolResult> {
        args.get("file_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .ok_or_else(|| ToolResult {
                output: "Missing required parameter: file_path".to_string(),
                is_error: true,
            })
    }

    fn unity_yaml_file_extension(abs_path: &std::path::Path) -> String {
        abs_path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    }

    fn is_unity_editor_yaml_candidate(abs_path: &std::path::Path) -> bool {
        matches!(
            Self::unity_yaml_file_extension(abs_path).as_str(),
            "unity" | "prefab"
        )
    }

    async fn try_unity_yaml_editor_tool(
        &self,
        message_type: &str,
        payload: serde_json::Value,
    ) -> Result<ToolResult, String> {
        let payload_text =
            serde_json::to_string(&payload).map_err(|e| format!("invalid tool payload: {}", e))?;
        let resp =
            crate::unity_bridge::send_message(&self.working_dir, message_type, &payload_text)
                .await?;

        if !resp.ok {
            return Err(resp
                .error
                .unwrap_or_else(|| format!("{} failed", message_type)));
        }

        let output = resp
            .message
            .filter(|message| !message.trim().is_empty())
            .ok_or_else(|| format!("{} returned an empty response", message_type))?;

        Ok(ToolResult {
            output: output.trim_end().to_string(),
            is_error: false,
        })
    }

    fn unity_yaml_list_editor_payload(
        file_path_arg: &str,
        options: &crate::unity_yaml::HierarchySummaryOptions,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({ "file_path": file_path_arg });
        if let Some(path_prefix) = options.path_prefix.as_deref() {
            payload["path_prefix"] = serde_json::json!(path_prefix);
        }
        if let Some(max_depth) = options.max_depth {
            payload["max_depth"] = serde_json::json!(max_depth);
        }
        if let Some(max_nodes) = options.max_nodes {
            payload["max_nodes"] = serde_json::json!(max_nodes);
        }
        payload
    }

    fn unity_yaml_search_editor_payload(
        file_path_arg: &str,
        options: &crate::unity_yaml::HierarchySearchOptions,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({ "file_path": file_path_arg });
        if let Some(query) = options.query.as_deref() {
            payload["query"] = serde_json::json!(query);
        }
        if !options.component_filters.is_empty() {
            payload["component_filter"] = serde_json::json!(options.component_filters.join(","));
        }
        if !options.match_fields.is_empty() {
            payload["match_fields"] = serde_json::json!(options.match_fields.join(","));
        }
        if let Some(path_prefix) = options.path_prefix.as_deref() {
            payload["path_prefix"] = serde_json::json!(path_prefix);
        }
        if let Some(limit) = options.limit {
            payload["limit"] = serde_json::json!(limit);
        }
        payload
    }

    fn unity_yaml_read_editor_payload(
        file_path_arg: &str,
        object_path: &str,
        args: &serde_json::Value,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "file_path": file_path_arg,
            "object_path": object_path,
        });
        if let Some(max_field_depth) = args
            .get("max_field_depth")
            .and_then(|value| value.as_u64())
            .filter(|value| *value > 0)
        {
            payload["max_field_depth"] = serde_json::json!(max_field_depth.min(6));
        }
        if let Some(max_array_items) = args
            .get("max_array_items")
            .and_then(|value| value.as_u64())
            .filter(|value| *value > 0)
        {
            payload["max_array_items"] = serde_json::json!(max_array_items.min(200));
        }
        payload
    }

    async fn execute_unity_yaml_list(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        use crate::unity_yaml as yaml_parser;

        let file_path_arg = match Self::unity_yaml_file_path_arg(args) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let summary_options = Self::parse_unity_yaml_summary_options(args);
        let (ref_graph_state, _project_root, abs_path) =
            self.unity_yaml_project_context(app_handle, &file_path_arg);
        if Self::is_unity_editor_yaml_candidate(&abs_path) {
            let payload = Self::unity_yaml_list_editor_payload(&file_path_arg, &summary_options);
            match self.try_unity_yaml_editor_tool("list_yaml", payload).await {
                Ok(result) => return result,
                Err(err) => eprintln!(
                    "[unity_yaml_list] Unity plugin path unavailable for '{}': {}",
                    file_path_arg, err
                ),
            }
        }

        let content = match Self::read_unity_yaml_content(&abs_path) {
            Ok(content) => content,
            Err(result) => return result,
        };
        if !Self::is_unity_yaml_content(&content) {
            return ToolResult {
                output: format!(
                    "unity_yaml_list only supports Unity text-serialized .unity/.prefab YAML files. '{}' does not look like Unity YAML.",
                    file_path_arg
                ),
                is_error: true,
            };
        }

        let ext = Self::unity_yaml_file_extension(&abs_path);
        if !yaml_parser::is_hierarchical_file(&ext) {
            return ToolResult {
                output: format!(
                    "unity_yaml_list only supports scene/prefab hierarchy files. Use unity_yaml_read for '{}'.",
                    file_path_arg
                ),
                is_error: true,
            };
        }

        let docs = yaml_parser::parse_yaml_docs(&content);
        let text = String::from_utf8_lossy(&content);
        let lines: Vec<&str> = text.lines().collect();
        let tree = yaml_parser::build_go_tree(&docs);
        if tree.is_empty() {
            return ToolResult {
                output: format!(
                    "No GameObjects found in '{}'. The file may be empty or not a scene/prefab.",
                    file_path_arg
                ),
                is_error: false,
            };
        }

        let has_prefab_instances = docs.iter().any(|d| d.class_id == 1001 && !d.is_stripped);
        let guid_map = if has_prefab_instances {
            self.build_guid_map_for_docs(app_handle, &ref_graph_state, &docs, &lines)
        } else {
            std::collections::HashMap::new()
        };
        let guid_resolver =
            |guid: &crate::asset_db::types::Guid| -> Option<String> { guid_map.get(guid).cloned() };

        ToolResult {
            output: yaml_parser::format_scene_summary_with_options(
                &tree,
                &docs,
                &lines,
                &guid_resolver,
                &file_path_arg,
                &summary_options,
            ),
            is_error: false,
        }
    }

    async fn execute_unity_yaml_search(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        use crate::unity_yaml as yaml_parser;

        let file_path_arg = match Self::unity_yaml_file_path_arg(args) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let search_options = Self::parse_unity_yaml_search_options(args);
        if !search_options.has_search_filters() {
            return ToolResult {
                output: "unity_yaml_search requires query or component_filter. Use unity_yaml_list to inspect a subtree without a search filter.".to_string(),
                is_error: true,
            };
        }

        let (ref_graph_state, _project_root, abs_path) =
            self.unity_yaml_project_context(app_handle, &file_path_arg);
        if Self::is_unity_editor_yaml_candidate(&abs_path) {
            let payload = Self::unity_yaml_search_editor_payload(&file_path_arg, &search_options);
            match self
                .try_unity_yaml_editor_tool("search_yaml", payload)
                .await
            {
                Ok(result) => return result,
                Err(err) => eprintln!(
                    "[unity_yaml_search] Unity plugin path unavailable for '{}': {}",
                    file_path_arg, err
                ),
            }
        }

        let content = match Self::read_unity_yaml_content(&abs_path) {
            Ok(content) => content,
            Err(result) => return result,
        };
        if !Self::is_unity_yaml_content(&content) {
            return ToolResult {
                output: format!(
                    "unity_yaml_search only supports Unity text-serialized .unity/.prefab YAML files. '{}' does not look like Unity YAML.",
                    file_path_arg
                ),
                is_error: true,
            };
        }

        let ext = Self::unity_yaml_file_extension(&abs_path);
        if !yaml_parser::is_hierarchical_file(&ext) {
            return ToolResult {
                output: format!(
                    "unity_yaml_search only supports scene/prefab hierarchy files. Use unity_yaml_read for '{}'.",
                    file_path_arg
                ),
                is_error: true,
            };
        }

        let docs = yaml_parser::parse_yaml_docs(&content);
        let text = String::from_utf8_lossy(&content);
        let lines: Vec<&str> = text.lines().collect();
        let tree = yaml_parser::build_go_tree(&docs);
        if tree.is_empty() {
            return ToolResult {
                output: format!(
                    "No GameObjects found in '{}'. The file may be empty or not a scene/prefab.",
                    file_path_arg
                ),
                is_error: false,
            };
        }

        let has_prefab_instances = docs.iter().any(|d| d.class_id == 1001 && !d.is_stripped);
        let guid_map = if has_prefab_instances {
            self.build_guid_map_for_docs(app_handle, &ref_graph_state, &docs, &lines)
        } else {
            std::collections::HashMap::new()
        };
        let guid_resolver =
            |guid: &crate::asset_db::types::Guid| -> Option<String> { guid_map.get(guid).cloned() };

        ToolResult {
            output: yaml_parser::format_hierarchy_search_results(
                &tree,
                &docs,
                &lines,
                &guid_resolver,
                &file_path_arg,
                &search_options,
            ),
            is_error: false,
        }
    }

    async fn execute_unity_yaml_read(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        use crate::unity_yaml as yaml_parser;

        let file_path_arg = match Self::unity_yaml_file_path_arg(args) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let object_path = args
            .get("object_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|s| s.to_string());
        let detail = args
            .get("detail")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");

        let (ref_graph_state, project_root, abs_path) =
            self.unity_yaml_project_context(app_handle, &file_path_arg);
        let ext = Self::unity_yaml_file_extension(&abs_path);
        let is_hierarchical = yaml_parser::is_hierarchical_file(&ext);

        if is_hierarchical && object_path.is_none() {
            return ToolResult {
                output: "unity_yaml_read requires object_path for .unity/.prefab files. Use unity_yaml_list for hierarchy listing or unity_yaml_search to locate a target.".to_string(),
                is_error: true,
            };
        }

        if Self::is_unity_editor_yaml_candidate(&abs_path) && detail != "document" {
            if let Some(obj_path) = object_path.as_deref() {
                let payload = Self::unity_yaml_read_editor_payload(&file_path_arg, obj_path, args);
                match self.try_unity_yaml_editor_tool("read_yaml", payload).await {
                    Ok(result) => return result,
                    Err(err) => eprintln!(
                        "[unity_yaml_read] Unity plugin path unavailable for '{}': {}",
                        file_path_arg, err
                    ),
                }
            }
        }

        let content = match Self::read_unity_yaml_content(&abs_path) {
            Ok(content) => content,
            Err(result) => return result,
        };
        if !Self::is_unity_yaml_content(&content) {
            return ToolResult {
                output: Self::format_plain_text_excerpt(&content),
                is_error: false,
            };
        }

        let docs = yaml_parser::parse_yaml_docs(&content);
        let text = String::from_utf8_lossy(&content);
        let lines: Vec<&str> = text.lines().collect();
        let world_transform_map = yaml_parser::build_world_transform_map(&docs, &lines);

        let guid_map = self.build_guid_map_for_docs(app_handle, &ref_graph_state, &docs, &lines);
        let internal_map = yaml_parser::build_internal_id_map(&docs);
        let internal_resolver = |fid: i64| -> Option<String> { internal_map.get(&fid).cloned() };
        let transform_hierarchy_labels = if is_hierarchical {
            Self::build_transform_hierarchy_labels(&docs, &internal_map)
        } else {
            std::collections::HashMap::new()
        };

        let (output_header, doc_ranges): (String, Vec<usize>) = if is_hierarchical {
            let tree = yaml_parser::build_go_tree(&docs);
            let obj_path = object_path.as_deref().unwrap();
            let target_file_id = match yaml_parser::find_go_by_path(&tree, obj_path) {
                Some(id) => id,
                None => {
                    let roots: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
                    return ToolResult {
                        output: format!(
                            "GameObject '{}' not found in '{}'. Available root objects: {}",
                            obj_path,
                            file_path_arg,
                            roots.join(", ")
                        ),
                        is_error: true,
                    };
                }
            };

            let target_doc_idx = docs.iter().position(|d| d.file_id == target_file_id);
            let Some(target_doc_idx) = target_doc_idx else {
                return ToolResult {
                    output: format!(
                        "Target '{}' was found in the hierarchy but its YAML document was unavailable in '{}'.",
                        obj_path, file_path_arg
                    ),
                    is_error: true,
                };
            };
            let target_doc = &docs[target_doc_idx];
            let label = obj_path.to_string();

            if detail == "document" {
                (
                    format!("Document fields of '{}' ({}):\n", label, file_path_arg),
                    vec![target_doc_idx],
                )
            } else if target_doc.class_id == 1001 || detail == "prefab_overrides" {
                if target_doc.class_id != 1001 {
                    return ToolResult {
                        output: format!(
                            "Target '{}' is {}, not a PrefabInstance.",
                            label, target_doc.type_name
                        ),
                        is_error: true,
                    };
                }
                let irs = yaml_parser::extract_prefab_instance_irs(&docs, &lines);
                if let Some(ir) = irs.iter().find(|ir| ir.local_file_id == target_file_id) {
                    let guid_resolver_fn = |guid: &crate::asset_db::types::Guid| -> Option<String> {
                        guid_map.get(guid).cloned()
                    };
                    let source_ctx = self.load_source_prefab_context(
                        &ir.source_prefab_guid,
                        &guid_map,
                        &project_root,
                    );
                    let stripped = yaml_parser::extract_stripped_mappings(&docs, &lines);
                    let detail = yaml_parser::format_prefab_instance_detail(
                        ir,
                        &guid_resolver_fn,
                        source_ctx.as_ref(),
                        &stripped,
                    );
                    return ToolResult {
                        output: detail,
                        is_error: false,
                    };
                }
                return ToolResult {
                    output: format!("PrefabInstance '{}' could not be parsed.", label),
                    is_error: true,
                };
            } else if target_doc.class_id == 1 {
                let component_indices = yaml_parser::get_components_for_go(&docs, target_file_id);
                if component_indices.is_empty() {
                    return ToolResult {
                        output: format!("No components found for '{}'.", label),
                        is_error: false,
                    };
                }
                (
                    format!("Components of '{}' ({}):\n", label, file_path_arg),
                    component_indices,
                )
            } else {
                (
                    format!("YAML document '{}' ({}):\n", label, file_path_arg),
                    vec![target_doc_idx],
                )
            }
        } else {
            if detail == "prefab_overrides" {
                return ToolResult {
                    output: "detail='prefab_overrides' only applies to PrefabInstance targets in scene/prefab YAML files.".to_string(),
                    is_error: true,
                };
            }
            (
                format!(
                    "Content of '{}' ({} documents):\n",
                    file_path_arg,
                    docs.len()
                ),
                (0..docs.len()).collect(),
            )
        };

        let guid_resolver = |hex: &str| -> Option<String> {
            let guid = crate::asset_db::types::parse_guid_hex(hex)?;
            guid_map.get(&guid).cloned()
        };

        let mut output = output_header;
        let mut wrote_transform_hierarchy = false;
        for &idx in &doc_ranges {
            let doc = &docs[idx];
            if !wrote_transform_hierarchy && (doc.class_id == 4 || doc.class_id == 224) {
                output.push_str(&Self::format_transform_hierarchy_section(
                    doc,
                    &transform_hierarchy_labels,
                ));
                wrote_transform_hierarchy = true;
            }
            output.push_str(&format!("\n--- {} ---\n", doc.type_name));
            output.push_str(&yaml_parser::format_doc_state_lines(doc));
            if let Some(info) = world_transform_map.get(&doc.file_id) {
                if doc.class_id == 4 || doc.class_id == 224 {
                    output.push_str(&Self::format_world_transform_lines(info));
                }
            }
            let content_start = (doc.line_start + 2).min(doc.line_end);
            let skipped_fields = if doc.m_enabled.is_some() {
                &["m_Enabled"][..]
            } else {
                &[][..]
            };
            let resolved = yaml_parser::resolve_references_in_lines_skipping_fields(
                &lines,
                content_start,
                doc.line_end,
                &guid_resolver,
                &internal_resolver,
                skipped_fields,
            );
            output.push_str(&resolved);
        }

        ToolResult {
            output,
            is_error: false,
        }
    }

    fn build_transform_hierarchy_labels(
        docs: &[crate::unity_yaml::YamlDoc],
        internal_map: &std::collections::HashMap<i64, String>,
    ) -> std::collections::HashMap<i64, (Option<String>, Option<String>, Vec<i64>)> {
        let node_label_map = Self::build_hierarchy_node_label_map(docs);
        let mut transform_to_owner: std::collections::HashMap<i64, i64> =
            std::collections::HashMap::new();

        for doc in docs {
            if doc.class_id != 4 && doc.class_id != 224 {
                continue;
            }

            if let Some(game_object_id) = doc.m_game_object_id.filter(|id| *id != 0) {
                transform_to_owner.insert(doc.file_id, game_object_id);
                continue;
            }

            if let Some(prefab_instance_id) = doc.prefab_instance_id.filter(|id| *id != 0) {
                transform_to_owner.insert(doc.file_id, prefab_instance_id);
            }
        }

        let mut result = std::collections::HashMap::new();
        for doc in docs {
            if doc.class_id != 4 && doc.class_id != 224 {
                continue;
            }

            let current = transform_to_owner.get(&doc.file_id).and_then(|owner_id| {
                Self::format_hierarchy_node_label_for_owner(
                    *owner_id,
                    internal_map,
                    &node_label_map,
                )
            });

            let parent = doc
                .m_father_id
                .filter(|id| *id != 0)
                .and_then(|transform_id| transform_to_owner.get(&transform_id))
                .and_then(|owner_id| {
                    Self::format_hierarchy_owner_name(*owner_id, internal_map, &node_label_map)
                });

            let mut seen = std::collections::HashSet::new();
            let mut children = Vec::new();
            for child_transform_id in &doc.transform_children {
                let Some(owner_id) = transform_to_owner.get(child_transform_id) else {
                    continue;
                };
                if seen.insert(*owner_id) {
                    children.push(*child_transform_id);
                }
            }

            result.insert(doc.file_id, (current, parent, children));
        }

        result
    }

    fn format_transform_hierarchy_section(
        doc: &crate::unity_yaml::YamlDoc,
        transform_hierarchy_labels: &std::collections::HashMap<
            i64,
            (Option<String>, Option<String>, Vec<i64>),
        >,
    ) -> String {
        if doc.class_id != 4 && doc.class_id != 224 {
            return String::new();
        }

        let (current, parent, children) = transform_hierarchy_labels
            .get(&doc.file_id)
            .cloned()
            .unwrap_or_else(|| (None, None, Vec::new()));

        let mut out = String::new();
        out.push_str("\n--- Hierarchy ---\n");
        match parent {
            Some(label) => out.push_str(&format!("  parent: {}\n", label)),
            None => out.push_str("  parent: none\n"),
        }

        if let Some(label) = current {
            out.push_str(&format!("  {}\n", label));
        }

        for (idx, child_transform_id) in children.iter().copied().enumerate() {
            let is_last = idx + 1 == children.len();
            let branch = if is_last { "└─" } else { "├─" };
            let child_label = transform_hierarchy_labels
                .get(&child_transform_id)
                .and_then(|(label, _, _)| label.as_deref())
                .unwrap_or("?");

            out.push_str(&format!("  {} {}\n", branch, child_label));

            let mut visiting = std::collections::HashSet::new();
            visiting.insert(doc.file_id);
            let hidden = Self::count_transform_hierarchy_descendants(
                transform_hierarchy_labels,
                child_transform_id,
                &mut visiting,
            );
            if hidden > 0 {
                let continuation = if is_last { "   " } else { "│  " };
                out.push_str(&format!(
                    "  {}... ({} child nodes hidden by max_depth)\n",
                    continuation, hidden
                ));
            }
        }
        out
    }

    fn build_hierarchy_node_label_map(
        docs: &[crate::unity_yaml::YamlDoc],
    ) -> std::collections::HashMap<i64, String> {
        fn visit(
            node: &crate::unity_yaml::HierarchyNode,
            out: &mut std::collections::HashMap<i64, String>,
        ) {
            out.insert(
                node.file_id,
                AgentInstance::format_hierarchy_node_label(node),
            );
            for child in &node.children {
                visit(child, out);
            }
        }

        let mut out = std::collections::HashMap::new();
        for root in crate::unity_yaml::build_go_tree(docs) {
            visit(&root, &mut out);
        }
        out
    }

    fn count_transform_hierarchy_descendants(
        transform_hierarchy_labels: &std::collections::HashMap<
            i64,
            (Option<String>, Option<String>, Vec<i64>),
        >,
        transform_id: i64,
        visiting: &mut std::collections::HashSet<i64>,
    ) -> usize {
        if !visiting.insert(transform_id) {
            return 0;
        }

        let count = transform_hierarchy_labels
            .get(&transform_id)
            .map(|(_, _, children)| {
                children
                    .iter()
                    .map(|child_id| {
                        1 + Self::count_transform_hierarchy_descendants(
                            transform_hierarchy_labels,
                            *child_id,
                            visiting,
                        )
                    })
                    .sum()
            })
            .unwrap_or(0);

        visiting.remove(&transform_id);
        count
    }

    fn format_hierarchy_node_label(node: &crate::unity_yaml::HierarchyNode) -> String {
        let mut label = node.name.clone();
        if !node.components.is_empty() {
            label.push_str(&format!(" ({})", node.components.join(", ")));
        }

        let annotations = Self::format_hierarchy_node_annotations(node);
        if !annotations.is_empty() {
            label.push_str(&annotations);
        }
        label
    }

    fn format_hierarchy_node_annotations(node: &crate::unity_yaml::HierarchyNode) -> String {
        let mut parts = Vec::new();
        if node.is_static {
            parts.push("Static".to_string());
        }
        if !node.is_active {
            parts.push("Inactive".to_string());
        }
        if let Some(tag) = &node.tag {
            parts.push(format!("Tag:{}", tag));
        }
        if let Some(layer) = node.layer {
            let layer_name = Self::unity_layer_name(layer);
            if layer_name.is_empty() {
                parts.push(format!("Layer:{}", layer));
            } else {
                parts.push(format!("Layer:{}", layer_name));
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("  [{}]", parts.join(", "))
        }
    }

    fn unity_layer_name(layer: i32) -> &'static str {
        match layer {
            0 => "Default",
            1 => "TransparentFX",
            2 => "Ignore Raycast",
            3 => "Layer3",
            4 => "Water",
            5 => "UI",
            6 => "Layer6",
            7 => "Layer7",
            _ => "",
        }
    }

    fn format_hierarchy_node_label_for_owner(
        owner_id: i64,
        internal_map: &std::collections::HashMap<i64, String>,
        node_label_map: &std::collections::HashMap<i64, String>,
    ) -> Option<String> {
        node_label_map.get(&owner_id).cloned().or_else(|| {
            internal_map
                .get(&owner_id)
                .map(|label| Self::format_hierarchy_leaf_label(label).to_string())
        })
    }

    fn format_hierarchy_owner_name(
        owner_id: i64,
        internal_map: &std::collections::HashMap<i64, String>,
        node_label_map: &std::collections::HashMap<i64, String>,
    ) -> Option<String> {
        internal_map
            .get(&owner_id)
            .map(|label| Self::format_hierarchy_leaf_label(label).to_string())
            .or_else(|| node_label_map.get(&owner_id).cloned())
    }

    fn format_hierarchy_display_label(label: &str) -> &str {
        label
            .strip_prefix("GO:")
            .or_else(|| label.strip_prefix("Prefab:"))
            .unwrap_or(label)
    }

    fn format_hierarchy_leaf_label(label: &str) -> &str {
        let display = Self::format_hierarchy_display_label(label);
        display.rsplit('/').next().unwrap_or(display)
    }

    fn format_world_transform_lines(info: &crate::unity_yaml::TransformWorldInfo) -> String {
        format!(
            "  World Position: {{x: {}, y: {}, z: {}}}\n  World Rotation: {{x: {}, y: {}, z: {}}}\n  World Scale: {{x: {}, y: {}, z: {}}}\n",
            Self::format_transform_scalar(info.position[0]),
            Self::format_transform_scalar(info.position[1]),
            Self::format_transform_scalar(info.position[2]),
            Self::format_transform_scalar(info.rotation_euler[0]),
            Self::format_transform_scalar(info.rotation_euler[1]),
            Self::format_transform_scalar(info.rotation_euler[2]),
            Self::format_transform_scalar(info.scale[0]),
            Self::format_transform_scalar(info.scale[1]),
            Self::format_transform_scalar(info.scale[2]),
        )
    }

    fn format_transform_scalar(value: f64) -> String {
        let rounded = if value.abs() < 0.000_000_5 {
            0.0
        } else {
            value
        };
        if rounded.fract().abs() < 0.000_000_5 {
            format!("{:.0}", rounded)
        } else {
            format!("{:.2}", rounded)
        }
    }

    fn load_source_prefab_context(
        &self,
        source_guid: &crate::asset_db::types::Guid,
        guid_map: &std::collections::HashMap<crate::asset_db::types::Guid, String>,
        project_root: &Option<std::path::PathBuf>,
    ) -> Option<crate::unity_yaml::SourcePrefabContext> {
        let rel_path = guid_map.get(source_guid)?;
        let root = project_root.as_ref()?;
        let abs_path = root.join(rel_path);

        let ext = abs_path.extension()?.to_string_lossy().to_lowercase();
        if !matches!(ext.as_str(), "prefab" | "unity") {
            return None;
        }

        let content = std::fs::read(&abs_path).ok()?;
        let docs = crate::unity_yaml::parse_yaml_docs(&content);
        if docs.is_empty() {
            return None;
        }
        let tree = crate::unity_yaml::build_go_tree(&docs);

        Some(crate::unity_yaml::SourcePrefabContext { tree, docs })
    }

    fn ensure_ref_graph_initialized(&self, app_handle: &AppHandle) {
        use crate::asset_db::{AssetDb, AssetDbState};

        let project_root = std::path::Path::new(&self.working_dir);
        if !project_root.join("Assets").is_dir() {
            eprintln!("[unity_yaml_read] Not a Unity project, skip auto-scan");
            return;
        }

        eprintln!("[unity_yaml_read] AssetDb DB not available, running auto-scan...");

        let mut graph = match AssetDb::open(project_root) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[unity_yaml_read] Failed to open AssetDb DB: {}", e);
                return;
            }
        };

        match graph.full_scan(|_phase| { /* silent scan, no events */ }) {
            Ok(stats) => {
                match crate::asset_db::watcher::reconcile_loaded_db(project_root, graph) {
                    Ok((reconciled, reconcile_stats)) => {
                        graph = reconciled;
                        eprintln!(
                            "[unity_yaml_read] auto-scan reconcile complete: queued={}, processed={}, failed={}",
                            reconcile_stats.queued,
                            reconcile_stats.processed,
                            reconcile_stats.failed
                        );
                    }
                    Err(e) => {
                        eprintln!("[unity_yaml_read] auto-scan reconcile failed: {}", e);
                        return;
                    }
                }
                eprintln!(
                    "[unity_yaml_read] auto-scan complete: {} nodes, {} edges, {}ms",
                    stats.nodes_added, stats.edges_added, stats.elapsed_ms
                );
                if let Some(ref_graph_state) = app_handle.try_state::<AssetDbState>() {
                    if let Ok(mut guard) = ref_graph_state.0.lock() {
                        *guard = Some(graph);
                    }
                }
            }
            Err(e) => {
                eprintln!("[unity_yaml_read] auto-scan failed: {}", e);
            }
        }
    }

    fn build_guid_map_for_docs(
        &self,
        app_handle: &AppHandle,
        ref_graph_state: &Option<tauri::State<'_, crate::asset_db::AssetDbState>>,
        docs: &[crate::unity_yaml::YamlDoc],
        lines: &[&str],
    ) -> std::collections::HashMap<crate::asset_db::types::Guid, String> {
        use crate::unity_yaml as yaml_parser;

        let all_ranges: Vec<(usize, usize)> = docs
            .iter()
            .map(|d| {
                let start = (d.line_start + 2).min(d.line_end);
                (start, d.line_end)
            })
            .collect();
        let all_guids = yaml_parser::collect_guids_from_ranges(lines, &all_ranges);

        let mut extra_guids: Vec<crate::asset_db::types::Guid> =
            docs.iter().filter_map(|d| d.source_prefab_guid).collect();
        let mut combined = all_guids;
        for g in extra_guids.drain(..) {
            if !combined.contains(&g) {
                combined.push(g);
            }
        }

        if combined.is_empty() {
            return std::collections::HashMap::new();
        }

        let db_map = ref_graph_state
            .as_ref()
            .and_then(|rgs| rgs.0.lock().ok())
            .and_then(|guard| {
                guard
                    .as_ref()
                    .and_then(|graph| graph.batch_resolve_paths(&combined).ok())
            })
            .unwrap_or_default();

        if !db_map.is_empty() {
            db_map
        } else {
            self.ensure_ref_graph_initialized(app_handle);
            ref_graph_state
                .as_ref()
                .and_then(|rgs| rgs.0.lock().ok())
                .and_then(|guard| {
                    guard
                        .as_ref()
                        .and_then(|graph| graph.batch_resolve_paths(&combined).ok())
                })
                .unwrap_or_default()
        }
    }

    fn execute_unity_asset_search(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ToolResult {
        use crate::asset_db::AssetDbState;

        let q = match args.get("q").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return ToolResult {
                    output: "Missing required parameter: q".to_string(),
                    is_error: true,
                };
            }
        };

        let range_str = args.get("range").and_then(|v| v.as_str()).unwrap_or("1-20");

        let (offset, limit) = if range_str == "0" {
            (0u64, 0u32)
        } else if let Some((start_s, end_s)) = range_str.split_once('-') {
            let start = start_s.parse::<u64>().unwrap_or(1).max(1);
            let end = end_s.parse::<u64>().unwrap_or(20);
            if end < start {
                return ToolResult {
                    output: format!("Invalid range '{}': end must be >= start", range_str),
                    is_error: true,
                };
            }
            let count = (end - start + 1).min(200) as u32;
            (start - 1, count)
        } else {
            return ToolResult {
                output: format!(
                    "Invalid range '{}'. Use '<start>-<end>' (e.g. '1-20') or '0' for count only.",
                    range_str
                ),
                is_error: true,
            };
        };

        let fields: Vec<String> = args
            .get("fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let ref_graph_state: tauri::State<'_, AssetDbState> = match app_handle.try_state() {
            Some(s) => s,
            None => {
                return ToolResult {
                    output: "AssetDbState not available. The asset index has not been initialized."
                        .to_string(),
                    is_error: true,
                };
            }
        };

        let guard = match ref_graph_state.0.lock() {
            Ok(g) => g,
            Err(e) => {
                return ToolResult {
                    output: format!("Failed to lock database: {}", e),
                    is_error: true,
                };
            }
        };

        let graph = match guard.as_ref() {
            Some(g) => g,
            None => {
                return ToolResult {
                    output: "Asset index not initialized. Please run a scan first (use the scan button in the UI).".to_string(),
                    is_error: true,
                };
            }
        };

        match graph.search_assets(&q, &fields, limit, offset) {
            Ok(result) => {
                if limit == 0 {
                    return ToolResult {
                        output: format!("total:{}", result.total),
                        is_error: false,
                    };
                }

                if result.rows.is_empty() {
                    return ToolResult {
                        output: "No Result".to_string(),
                        is_error: false,
                    };
                }

                let actual_end = offset + result.rows.len() as u64;
                let mut output = format!(
                    "total:{} showing:{}-{}",
                    result.total,
                    offset + 1,
                    actual_end
                );

                for row in &result.rows {
                    output.push('\n');
                    let mut parts: Vec<&str> = Vec::new();
                    if let Some(ref tp) = row.tp {
                        parts.push(tp);
                    }
                    if let Some(ref n) = row.n {
                        parts.push(n);
                    }
                    if let Some(ref p) = row.p {
                        parts.push(p);
                    }
                    if let Some(ref guid) = row.guid {
                        parts.push(guid);
                    }
                    output.push_str(&parts.join("\t"));
                }

                ToolResult {
                    output,
                    is_error: false,
                }
            }
            Err(e) => ToolResult {
                output: format!("Search failed: {}", e),
                is_error: true,
            },
        }
    }

    async fn run_subagent_task(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        description: &str,
        prompt: &str,
        subagent_type: &str,
        tool_call_id: &str,
        run_id: &str,
    ) -> Result<SubagentTaskResult, String> {
        let agent_def = match self.registry.get(subagent_type) {
            Some(def) => def.clone(),
            None => {
                return Err(format!(
                    "Unknown agent type: '{}'. Available: {:?}",
                    subagent_type,
                    self.registry.list_ids()
                ));
            }
        };

        eprintln!(
            "[Agent {}] spawning subagent '{}' (type={}): {}",
            self.id, agent_def.name, subagent_type, description
        );

        let child_session_id = match store.create_session(
            &format!("sub:{}", description),
            Some(&self.session_id),
            self.workspace_id.as_deref(),
            "chat",
            Some(&agent_def.id),
        ) {
            Ok(id) => id,
            Err(e) => {
                return Err(format!("Failed to create subagent session: {}", e));
            }
        };

        let mut child = AgentInstance::new(
            Arc::new(agent_def),
            &child_session_id,
            self.backend.clone(),
            self.debug,
            self.registry.clone(),
            self.tool_registry.clone(),
            self.working_dir.clone(),
            self.raw_store.clone(),
            self.workspace_id.clone(),
            self.resolve_subagent_model_name(subagent_type)
                .unwrap_or_else(|| self.effective_model.clone()),
            self.effort.clone(),
            self.app_knowledge_dir.clone(),
            self.app_agent_dir.clone(),
            self.knowledge_access_mode,
            self.undo_manager.clone(),
            self.subagent_model_overrides.clone(),
            self.cancel_waiter(),
        );
        child.parent_tool_call = Some(ParentToolCall::new(
            self.session_id.clone(),
            run_id.to_string(),
            tool_call_id.to_string(),
        ));

        match child
            .run(app_handle, store, prompt, None, None, "build", None)
            .await
        {
            Ok(result_text) => {
                eprintln!(
                    "[Agent {}] subagent '{}' completed, output_len={}",
                    self.id,
                    subagent_type,
                    result_text.len()
                );

                if let Ok(child_usage) = store.get_token_usage(&child_session_id) {
                    if child_usage.total_input_tokens > 0
                        || child_usage.total_output_tokens > 0
                        || child_usage.total_cache_read_tokens > 0
                        || child_usage.total_cache_write_tokens > 0
                    {
                        match store.record_token_usage(
                            &self.session_id,
                            child_usage.total_input_tokens,
                            child_usage.total_output_tokens,
                            child_usage.total_cache_read_tokens,
                            child_usage.total_cache_write_tokens,
                            child_usage.total_cost_usd,
                            child_usage.priced_rounds,
                            None,
                            None,
                        ) {
                            Ok(parent_totals) => {
                                eprintln!(
                                    "[Agent {}] merged subagent tokens: +{}in/+{}out/+{}cache_r/+{}cache_w/${:.6} -> parent total: {}in/{}out/{}cache_r/{}cache_w/${:.6}",
                                    self.id,
                                    child_usage.total_input_tokens, child_usage.total_output_tokens,
                                    child_usage.total_cache_read_tokens, child_usage.total_cache_write_tokens,
                                    child_usage.total_cost_usd,
                                    parent_totals.total_input_tokens, parent_totals.total_output_tokens,
                                    parent_totals.total_cache_read_tokens, parent_totals.total_cache_write_tokens,
                                    parent_totals.total_cost_usd,
                                );
                                emit_stream(
                                    app_handle,
                                    run_id,
                                    StreamEvent::UsageUpdate {
                                        session_id: self.session_id.clone(),
                                        input_tokens: child_usage
                                            .total_input_tokens
                                            .min(u32::MAX as u64)
                                            as u32,
                                        output_tokens: child_usage
                                            .total_output_tokens
                                            .min(u32::MAX as u64)
                                            as u32,
                                        cache_read_tokens: child_usage
                                            .total_cache_read_tokens
                                            .min(u32::MAX as u64)
                                            as u32,
                                        cache_write_tokens: child_usage
                                            .total_cache_write_tokens
                                            .min(u32::MAX as u64)
                                            as u32,
                                        total_input_tokens: parent_totals.total_input_tokens,
                                        total_output_tokens: parent_totals.total_output_tokens,
                                        total_cache_read_tokens: parent_totals
                                            .total_cache_read_tokens,
                                        total_cache_write_tokens: parent_totals
                                            .total_cache_write_tokens,
                                        total_cost_usd: parent_totals.total_cost_usd,
                                        priced_rounds: parent_totals.priced_rounds,
                                        context_tokens: 0,
                                        context_limit: 0,
                                    },
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "[Agent {}] failed to merge subagent token usage: {}",
                                    self.id, e
                                );
                            }
                        }
                    }
                }
                let tool_calls = store
                    .load_session(&child_session_id)
                    .map(|detail| {
                        crate::session::history::collect_assistant_tool_calls(&detail.messages)
                    })
                    .unwrap_or_default();
                Ok(SubagentTaskResult {
                    output: result_text,
                    tool_calls,
                    is_error: false,
                })
            }
            Err(e) => {
                eprintln!(
                    "[Agent {}] subagent '{}' failed: {}",
                    self.id, subagent_type, e
                );

                if let Ok(child_usage) = store.get_token_usage(&child_session_id) {
                    if child_usage.total_input_tokens > 0
                        || child_usage.total_output_tokens > 0
                        || child_usage.total_cache_read_tokens > 0
                        || child_usage.total_cache_write_tokens > 0
                    {
                        if let Ok(parent_totals) = store.record_token_usage(
                            &self.session_id,
                            child_usage.total_input_tokens,
                            child_usage.total_output_tokens,
                            child_usage.total_cache_read_tokens,
                            child_usage.total_cache_write_tokens,
                            child_usage.total_cost_usd,
                            child_usage.priced_rounds,
                            None,
                            None,
                        ) {
                            emit_stream(
                                app_handle,
                                run_id,
                                StreamEvent::UsageUpdate {
                                    session_id: self.session_id.clone(),
                                    input_tokens: child_usage
                                        .total_input_tokens
                                        .min(u32::MAX as u64)
                                        as u32,
                                    output_tokens: child_usage
                                        .total_output_tokens
                                        .min(u32::MAX as u64)
                                        as u32,
                                    cache_read_tokens: child_usage
                                        .total_cache_read_tokens
                                        .min(u32::MAX as u64)
                                        as u32,
                                    cache_write_tokens: child_usage
                                        .total_cache_write_tokens
                                        .min(u32::MAX as u64)
                                        as u32,
                                    total_input_tokens: parent_totals.total_input_tokens,
                                    total_output_tokens: parent_totals.total_output_tokens,
                                    total_cache_read_tokens: parent_totals.total_cache_read_tokens,
                                    total_cache_write_tokens: parent_totals
                                        .total_cache_write_tokens,
                                    total_cost_usd: parent_totals.total_cost_usd,
                                    priced_rounds: parent_totals.priced_rounds,
                                    context_tokens: 0,
                                    context_limit: 0,
                                },
                            );
                        }
                    }
                }

                let tool_calls = store
                    .load_session(&child_session_id)
                    .map(|detail| {
                        crate::session::history::collect_assistant_tool_calls(&detail.messages)
                    })
                    .unwrap_or_default();
                Ok(SubagentTaskResult {
                    output: format!("Subagent error: {}", e),
                    tool_calls,
                    is_error: true,
                })
            }
        }
    }

    async fn execute_task(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        args: &serde_json::Value,
        tool_call_id: &str,
        run_id: &str,
    ) -> ExecutedToolResult {
        let description = args["description"].as_str().unwrap_or("unknown task");
        let prompt = match args["prompt"].as_str() {
            Some(p) if !p.is_empty() => p,
            _ => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Error: task tool requires a non-empty 'prompt' parameter".to_string(),
                    is_error: true,
                });
            }
        };
        let subagent_type = match args["subagent_type"].as_str() {
            Some(t) => t,
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Error: task tool requires 'subagent_type' parameter".to_string(),
                    is_error: true,
                });
            }
        };

        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }

        match self
            .run_subagent_task(
                app_handle,
                store,
                description,
                prompt,
                subagent_type,
                tool_call_id,
                run_id,
            )
            .await
        {
            Ok(_) if self.is_cancel_requested() => Self::interrupted_tool_result(),
            Ok(result) => ExecutedToolResult::from_tool_result(ToolResult {
                output: result.output,
                is_error: result.is_error,
            })
            .with_nested_tool_calls(result.tool_calls),
            Err(_) if self.is_cancel_requested() => Self::interrupted_tool_result(),
            Err(error) => ExecutedToolResult::from_tool_result(ToolResult {
                output: error,
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assess_knowledge_tool_confirmation, build_l2_memory_section, build_l3_rule_section,
        build_structure_section, finalize_tool_call_record, AgentInstance,
        AgentKnowledgeDocumentContent, AgentKnowledgeDocumentContentPatch, AgentKnowledgeListItem,
        AgentKnowledgeMutationResponse, AgentKnowledgeReadResponse, AgentKnowledgeSearchHit,
        ExecutedToolResult, InjectedPromptItem, KnowledgeAccessMode, ParentToolCall,
        RawContextStore, ToolRunOutcome,
    };
    use crate::agent::definition::{AgentDef, AgentDefRegistry};
    use crate::commands::{
        KnowledgeToolConfirmDirectoryMode, KnowledgeToolConfirmOperation, StreamEvent,
        ToolCallOutcome,
    };
    use crate::knowledge_store::{
        create_directory, default_directory_config_for_type, save_document,
        update_directory_config, KnowledgeDocument, KnowledgeInjectMode, KnowledgeReadResponse,
        KnowledgeReadResult, KnowledgeSearchMatchSection, KnowledgeTargetKind, KnowledgeType,
    };
    use crate::session::models::{ToolCallInfo, UserIntentPayload, UserIntentSkill};
    use crate::tool::{ToolDef, ToolRegistry, ToolResult};
    use crate::unity_docs::seed_managed_documents_for_tests;
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};
    use tempfile::tempdir;

    #[test]
    fn preserves_interrupted_outcome_from_reserved_tool_result_text() {
        let result = ExecutedToolResult::from_tool_result(ToolResult {
            output: crate::session::history::INTERRUPTED_TOOL_RESULT.to_string(),
            is_error: false,
        });

        assert_eq!(result.outcome, ToolRunOutcome::Interrupted);
    }

    #[test]
    fn parent_tool_call_events_use_parent_run_context() {
        let parent = ParentToolCall::new(
            "parent-session".to_string(),
            "parent-run".to_string(),
            "task-1".to_string(),
        );

        let start = parent.subagent_tool_call_start(
            "read-1".to_string(),
            "read".to_string(),
            "{}".to_string(),
            Some(3),
            Some("read-1".to_string()),
            Some(3),
        );
        assert_eq!(start.run_id, "parent-run");
        match start.event {
            StreamEvent::SubagentToolCallStart {
                session_id,
                parent_tool_call_id,
                tool_call_id,
                tool_name,
                arguments,
                order,
                part_id,
                render_seq,
            } => {
                assert_eq!(session_id, "parent-session");
                assert_eq!(parent_tool_call_id, "task-1");
                assert_eq!(tool_call_id, "read-1");
                assert_eq!(tool_name, "read");
                assert_eq!(arguments, "{}");
                assert_eq!(order, Some(3));
                assert_eq!(part_id, Some("read-1".to_string()));
                assert_eq!(render_seq, Some(3));
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let delta = parent.tool_call_delta("partial".to_string());
        assert_eq!(delta.run_id, "parent-run");
        match delta.event {
            StreamEvent::ToolCallDelta {
                session_id,
                tool_call_id,
                delta,
            } => {
                assert_eq!(session_id, "parent-session");
                assert_eq!(tool_call_id, "task-1");
                assert_eq!(delta, "partial");
            }
            other => panic!("unexpected event: {:?}", other),
        }

        let done = parent.subagent_tool_call_done(
            "read-1".to_string(),
            "read".to_string(),
            "ok".to_string(),
            ToolCallOutcome::Done,
            None,
        );
        assert_eq!(done.run_id, "parent-run");
        match done.event {
            StreamEvent::SubagentToolCallDone {
                session_id,
                parent_tool_call_id,
                tool_call_id,
                tool_name,
                output,
                outcome,
                images,
            } => {
                assert_eq!(session_id, "parent-session");
                assert_eq!(parent_tool_call_id, "task-1");
                assert_eq!(tool_call_id, "read-1");
                assert_eq!(tool_name, "read");
                assert_eq!(output, "ok");
                assert_eq!(outcome, ToolCallOutcome::Done);
                assert!(images.is_none());
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn user_wait_target_uses_parent_context_for_subagents() {
        let mut agent = test_agent_instance(String::new());
        let target = agent.user_wait_target("child-run");
        assert_eq!(target.session_id, "session-test");
        assert_eq!(target.run_id, "child-run");

        agent.parent_tool_call = Some(ParentToolCall::new(
            "parent-session".to_string(),
            "parent-run".to_string(),
            "task-1".to_string(),
        ));

        let target = agent.user_wait_target("child-run");
        assert_eq!(target.session_id, "parent-session");
        assert_eq!(target.run_id, "parent-run");
    }

    #[test]
    fn finalize_tool_call_record_preserves_nested_subagent_history() {
        let tool_call = ToolCallInfo {
            id: "task-1".to_string(),
            name: "task".to_string(),
            arguments: "{}".to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: None,
            recorded_output: None,
            nested_tool_calls: None,
        };
        let nested_tool_call = ToolCallInfo {
            id: "read-1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
            order: None,
            server_tool: None,
            server_tool_output: None,
            outcome: Some(crate::commands::ToolCallOutcome::Done),
            recorded_output: Some("ok".to_string()),
            nested_tool_calls: None,
        };
        let result = ExecutedToolResult::from_tool_result(ToolResult {
            output: "subagent result".to_string(),
            is_error: false,
        })
        .with_nested_tool_calls(vec![nested_tool_call.clone()]);

        let finalized = finalize_tool_call_record(&tool_call, Some(&result));

        assert_eq!(
            finalized.outcome,
            Some(crate::commands::ToolCallOutcome::Done)
        );
        let nested_tool_calls = finalized
            .nested_tool_calls
            .as_ref()
            .expect("nested tool calls");
        assert_eq!(nested_tool_calls.len(), 1);
        assert_eq!(nested_tool_calls[0].id, nested_tool_call.id);
        assert_eq!(
            nested_tool_calls[0].recorded_output.as_deref(),
            nested_tool_call.recorded_output.as_deref()
        );
    }

    #[test]
    fn needs_undo_tracking_includes_workspace_mutations() {
        assert!(AgentInstance::needs_undo_tracking("bash"));
        assert!(AgentInstance::needs_undo_tracking("write"));
        assert!(AgentInstance::needs_undo_tracking("edit"));
        assert!(AgentInstance::needs_undo_tracking("unity_execute"));
        assert!(AgentInstance::needs_undo_tracking("knowledge_create"));
        assert!(AgentInstance::needs_undo_tracking("knowledge_edit"));
        assert!(AgentInstance::needs_undo_tracking("knowledge_move"));
        assert!(AgentInstance::needs_undo_tracking("knowledge_delete"));
        assert!(!AgentInstance::needs_undo_tracking("read"));
        assert!(!AgentInstance::needs_undo_tracking("grep"));
    }

    #[test]
    fn display_working_dir_is_explicit_when_missing() {
        assert_eq!(
            AgentInstance::display_working_dir_value(""),
            "(not selected)"
        );
        assert_eq!(
            AgentInstance::display_working_dir_value("  "),
            "(not selected)"
        );
        assert_eq!(
            AgentInstance::display_working_dir_value("C:/Proj"),
            "C:/Proj"
        );
    }

    #[test]
    fn transform_hierarchy_lines_include_parent_and_children_for_gameobjects() {
        let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Children:
  - {fileID: 21}
  m_Father: {fileID: 0}
--- !u!1 &20
GameObject:
  m_Name: Child
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_Children:
  - {fileID: 31}
  m_Father: {fileID: 11}
--- !u!1 &30
GameObject:
  m_Name: Grandchild
--- !u!4 &31
Transform:
  m_GameObject: {fileID: 30}
  m_Children: []
  m_Father: {fileID: 21}
"#;

        let docs = crate::unity_yaml::parse_yaml_docs(yaml);
        let internal_map = crate::unity_yaml::build_internal_id_map(&docs);
        let labels = AgentInstance::build_transform_hierarchy_labels(&docs, &internal_map);

        let root_transform = docs
            .iter()
            .find(|doc| doc.file_id == 11)
            .expect("root transform");
        let child_transform = docs
            .iter()
            .find(|doc| doc.file_id == 21)
            .expect("child transform");

        assert_eq!(
            AgentInstance::format_transform_hierarchy_section(root_transform, &labels),
            "\n--- Hierarchy ---\n  parent: none\n  Root\n  └─ Child\n     ... (1 child nodes hidden by max_depth)\n"
        );
        assert_eq!(
            AgentInstance::format_transform_hierarchy_section(child_transform, &labels),
            "\n--- Hierarchy ---\n  parent: Root\n  Child\n  └─ Grandchild\n"
        );
    }

    #[test]
    fn transform_hierarchy_lines_resolve_prefab_instance_children() {
        let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Children:
  - {fileID: 600}
  m_Father: {fileID: 0}
--- !u!4 &600 stripped
Transform:
  m_PrefabInstance: {fileID: 9000}
  m_GameObject: {fileID: 0}
  m_Father: {fileID: 11}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 11}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: ChildPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;

        let docs = crate::unity_yaml::parse_yaml_docs(yaml);
        let internal_map = crate::unity_yaml::build_internal_id_map(&docs);
        let labels = AgentInstance::build_transform_hierarchy_labels(&docs, &internal_map);
        let root_transform = docs
            .iter()
            .find(|doc| doc.file_id == 11)
            .expect("root transform");

        assert_eq!(
            AgentInstance::format_transform_hierarchy_section(root_transform, &labels),
            "\n--- Hierarchy ---\n  parent: none\n  Root\n  └─ ChildPrefab\n"
        );
    }

    #[test]
    fn validate_tool_paths_when_workspace_is_missing() {
        assert!(AgentInstance::validate_tool_path_requirements(
            "",
            "bash",
            &json!({"command":"pwd"}),
            false
        )
        .is_some());
        assert!(AgentInstance::validate_tool_path_requirements(
            "",
            "bash",
            &json!({"command":"pwd","workdir":"C:/Temp"}),
            false
        )
        .is_none());
        assert!(AgentInstance::validate_tool_path_requirements(
            "",
            "read",
            &json!({"filePath":"relative.txt"}),
            false
        )
        .is_some());
        assert!(AgentInstance::validate_tool_path_requirements(
            "",
            "read",
            &json!({"filePath":"C:/Temp/file.txt"}),
            false
        )
        .is_none());
        assert!(AgentInstance::validate_tool_path_requirements(
            "",
            "knowledge_query",
            &json!({"query":"player"}),
            false
        )
        .is_some());
    }

    #[test]
    fn fs_tools_allow_outside_working_dir_when_boundary_is_disabled() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&outside).expect("create outside");

        let outside_file = outside.join("outside.txt");
        std::fs::write(&outside_file, "ok").expect("write outside file");

        let workspace_str = workspace.to_string_lossy().to_string();
        let outside_file_str = outside_file.to_string_lossy().to_string();
        let outside_dir_str = outside.to_string_lossy().to_string();

        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "read",
            &json!({"filePath":outside_file_str}),
            false
        )
        .is_none());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "list",
            &json!({"path":outside_dir_str}),
            false
        )
        .is_none());
    }

    #[test]
    fn workspace_scoped_fs_tools_stay_within_working_dir_when_boundary_is_enabled() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&outside).expect("create outside");

        let inside_file = workspace.join("inside.txt");
        std::fs::write(&inside_file, "ok").expect("write inside file");

        let outside_file = outside.join("outside.txt");
        std::fs::write(&outside_file, "nope").expect("write outside file");

        let workspace_str = workspace.to_string_lossy().to_string();
        let outside_file_str = outside_file.to_string_lossy().to_string();
        let outside_dir_str = outside.to_string_lossy().to_string();

        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "read",
            &json!({"filePath":"inside.txt"}),
            true
        )
        .is_none());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "write",
            &json!({"filePath":"nested/new.txt","content":"ok"}),
            true
        )
        .is_none());

        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "read",
            &json!({"filePath":outside_file_str}),
            true
        )
        .is_some());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "edit",
            &json!({"filePath":"../outside/outside.txt","oldString":"nope","newString":"ok"}),
            true
        )
        .is_some());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "list",
            &json!({"path":"../outside"}),
            true
        )
        .is_some());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "grep",
            &json!({"pattern":"nope","path":outside_dir_str}),
            true
        )
        .is_some());
    }

    #[test]
    fn workspace_scoped_fs_tools_allow_app_temp_dir_when_boundary_is_enabled() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace");

        let app_temp = crate::commands::app_temp_dir().expect("app temp dir");
        let temp_file = app_temp.join("agent-boundary-test").join("output.txt");
        let temp_dir = app_temp.join("agent-boundary-test");

        let workspace_str = workspace.to_string_lossy().to_string();
        let temp_file_str = temp_file.to_string_lossy().to_string();
        let temp_dir_str = temp_dir.to_string_lossy().to_string();

        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "write",
            &json!({"filePath":temp_file_str,"content":"ok"}),
            true
        )
        .is_none());
        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "list",
            &json!({"path":temp_dir_str}),
            true
        )
        .is_none());
    }

    #[cfg(windows)]
    #[test]
    fn workspace_scoped_fs_tools_reject_drive_relative_paths() {
        use std::path::Component;

        let workspace = tempdir().expect("temp dir");
        let workspace_str = workspace.path().to_string_lossy().to_string();
        let drive_relative = workspace
            .path()
            .components()
            .find_map(|component| match component {
                Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().to_string()),
                _ => None,
            })
            .expect("windows drive prefix");

        assert!(AgentInstance::validate_tool_path_requirements(
            &workspace_str,
            "list",
            &json!({"path":drive_relative}),
            true
        )
        .is_some());
    }

    #[test]
    fn path_targets_knowledge_root_for_workspace_documents() {
        assert!(AgentInstance::path_targets_knowledge_root(
            "C:/Repo",
            None,
            "C:/Repo/Locus/knowledge/skill/builtin/create-skill.md"
        ));
        assert!(AgentInstance::path_targets_knowledge_root(
            "C:/Repo",
            None,
            "Locus/knowledge/design/core-loop.md"
        ));
        assert!(!AgentInstance::path_targets_knowledge_root(
            "C:/Repo",
            None,
            "C:/Repo/src/main.rs"
        ));
    }

    #[test]
    fn shell_command_mentions_workspace_and_app_knowledge_roots() {
        let app_root = std::path::PathBuf::from("C:/Repo/knowledge");
        assert!(AgentInstance::shell_command_mentions_knowledge_root(
            "C:/Repo",
            Some(&app_root),
            "mv knowledge/skill/builtin/create-skill.md knowledge/skill/builtin/new-skill.md"
        ));
        assert!(AgentInstance::shell_command_mentions_knowledge_root(
            "C:/Repo",
            Some(&app_root),
            "mkdir Locus/knowledge/skill/unity"
        ));
        assert!(AgentInstance::shell_command_mentions_knowledge_root(
            "C:/Repo",
            Some(&app_root),
            "rm Locus/knowledge"
        ));
        assert!(!AgentInstance::shell_command_mentions_knowledge_root(
            "C:/Repo",
            Some(&app_root),
            "cargo test"
        ));
    }

    #[test]
    fn bash_git_knowledge_assessment_separates_index_and_worktree_operations() {
        let app_root = std::path::PathBuf::from("C:/Repo/knowledge");
        let cases = [
            (
                "git status --short Locus/knowledge/design/core.md",
                false,
                false,
            ),
            ("git add Locus/knowledge/design/core.md", false, false),
            (
                "git restore --staged Locus/knowledge/design/core.md",
                false,
                false,
            ),
            ("git reset -- Locus/knowledge/design/core.md", false, false),
            ("git restore -- Locus/knowledge/design/core.md", true, true),
            ("git checkout -- Locus/knowledge/design/core.md", true, true),
            ("git reset --hard HEAD", true, true),
            ("git stash apply", true, true),
            ("git stash pop", true, true),
            ("git merge feature/notes", true, true),
            ("git rebase main", true, true),
            ("git cherry-pick HEAD~1", true, true),
            ("git revert HEAD", true, true),
        ];

        for (command, requires_confirm, reconcile_after_success) in cases {
            let assessment = AgentInstance::assess_bash_git_knowledge_command(
                "C:/Repo",
                Some(&app_root),
                &json!({"workdir":"C:/Repo","command":command}),
            )
            .expect("git command should be classified");
            assert!(assessment.touches_knowledge, "command: {command}");
            assert_eq!(
                assessment.requires_confirm, requires_confirm,
                "command: {command}"
            );
            assert_eq!(
                assessment.reconcile_after_success, reconcile_after_success,
                "command: {command}"
            );
        }
    }

    #[test]
    fn bash_git_knowledge_assessment_handles_simple_git_sequences() {
        let assessment = AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git add Locus/knowledge/design/core.md && git commit -m 'docs: update knowledge'"
            }),
        )
        .expect("git sequence should be classified");

        assert!(assessment.touches_knowledge);
        assert!(!assessment.requires_confirm);
        assert!(!assessment.reconcile_after_success);

        assert!(AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git -c core.quotePath=false diff -- Locus/knowledge/design/core.md | sed -n '1,220p' || true"
            }),
        )
        .is_some());

        assert!(AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git -c core.quotePath=false diff -- Locus/knowledge/design/core.md |\nsed -n '1,220p'"
            }),
        )
        .is_some());

        assert!(AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git add Locus/knowledge/design/core.md && rm Locus/knowledge/design/core.md"
            }),
        )
        .is_none());
    }

    #[test]
    fn bash_git_knowledge_assessment_allows_output_wrappers() {
        let assessment = AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"echo '--- diff ---' && git -c core.quotePath=false diff -- Locus/knowledge/design/core.md | sed -n '1,220p'"
            }),
        )
        .expect("wrapped git diff should be classified");

        assert!(assessment.touches_knowledge);
        assert!(!assessment.requires_confirm);
        assert!(!assessment.reconcile_after_success);

        assert!(AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"echo '--- docs ---' && find Locus/knowledge/design -maxdepth 3 -type f | sort"
            }),
        )
        .is_none());
    }

    #[test]
    fn bash_git_knowledge_assessment_allows_multiline_git_status_after_diff() {
        let assessment = AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git -c core.quotePath=false diff --unified=1 -- Assets/PlayerHealthBar.cs Assets/PlayerPlatformerController.cs ProjectSettings/ProjectSettings.asset | sed -n '220,520p'\nprintf '%s\\n' '--- knowledge status ---'\ngit -c core.quotePath=false status --short -- Locus/knowledge"
            }),
        )
        .expect("multiline git status should be classified");

        assert!(assessment.touches_knowledge);
        assert!(!assessment.requires_confirm);
        assert!(!assessment.reconcile_after_success);
    }

    #[test]
    fn bash_git_knowledge_assessment_rejects_multiline_find_after_diff() {
        assert!(AgentInstance::assess_bash_git_knowledge_command(
            "C:/Repo",
            None,
            &json!({
                "workdir":"C:/Repo",
                "command":"git -c core.quotePath=false diff --unified=1 -- Assets/PlayerHealthBar.cs Assets/PlayerPlatformerController.cs ProjectSettings/ProjectSettings.asset | sed -n '220,520p'\nprintf '%s\\n' '--- knowledge files ---'\nfind Locus/knowledge/design/system -maxdepth 3 -type f | sort"
            }),
        )
        .is_none());
    }

    fn test_agent_instance_with_prompts_and_app_knowledge_dir(
        working_dir: String,
        system_prompt: &str,
        env_template: &str,
        app_knowledge_dir: Option<std::path::PathBuf>,
    ) -> AgentInstance {
        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: system_prompt.to_string(),
                env_template: env_template.to_string(),
                tools: Vec::new(),
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(ToolRegistry::new()),
            working_dir,
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(app_knowledge_dir),
            Arc::new(None),
            KnowledgeAccessMode::Full,
            None,
            HashMap::new(),
            cancel_rx,
        )
    }

    fn test_agent_instance_with_prompts(
        working_dir: String,
        system_prompt: &str,
        env_template: &str,
    ) -> AgentInstance {
        test_agent_instance_with_prompts_and_app_knowledge_dir(
            working_dir,
            system_prompt,
            env_template,
            None,
        )
    }

    fn test_agent_instance(working_dir: String) -> AgentInstance {
        test_agent_instance_with_prompts(working_dir, "", "")
    }

    fn test_agent_instance_with_tools_and_mode(
        working_dir: String,
        tools: Vec<String>,
        knowledge_access_mode: KnowledgeAccessMode,
    ) -> AgentInstance {
        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: String::new(),
                env_template: "{{#knowledge}}\n<knowledge_context>\n{{/knowledge}}".to_string(),
                tools,
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(ToolRegistry::with_builtins()),
            working_dir,
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(None),
            Arc::new(None),
            knowledge_access_mode,
            None,
            HashMap::new(),
            cancel_rx,
        )
    }

    fn noop_tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("{} description", name),
            parameters: serde_json::json!({"type": "object"}),
            execute: Arc::new(|_, _| {
                Box::pin(async {
                    ToolResult {
                        output: String::new(),
                        is_error: false,
                    }
                })
            }),
        }
    }

    fn tool_load_mode(items: &[InjectedPromptItem], name: &str) -> String {
        items
            .iter()
            .find(|item| item.title == name)
            .and_then(|item| item.meta.as_ref())
            .and_then(|meta| meta.get("loadMode"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn tool_meta_bool(items: &[InjectedPromptItem], name: &str, key: &str) -> Option<bool> {
        items
            .iter()
            .find(|item| item.title == name)
            .and_then(|item| item.meta.as_ref())
            .and_then(|meta| meta.get(key))
            .and_then(|value| value.as_bool())
    }

    #[tokio::test]
    async fn available_tool_prompt_items_marks_direct_and_lazy_tools() {
        let temp = tempdir().expect("temp dir");
        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        let instance = AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: String::new(),
                env_template: String::new(),
                tools: vec![
                    "read".to_string(),
                    "edit".to_string(),
                    "unity_execute".to_string(),
                    "unity_run_states".to_string(),
                    "unity_capture_viewport".to_string(),
                    "graph_view".to_string(),
                    "web_fetch".to_string(),
                    "knowledge_create".to_string(),
                    "knowledge_edit".to_string(),
                    "knowledge_move".to_string(),
                    "knowledge_delete".to_string(),
                ],
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(ToolRegistry::with_builtins()),
            temp.path().to_string_lossy().to_string(),
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(None),
            Arc::new(None),
            KnowledgeAccessMode::Full,
            None,
            HashMap::new(),
            cancel_rx,
        );

        let request_tool_names = instance.build_request_tool_names().await;
        assert!(!request_tool_names.contains(&"unity_capture_viewport".to_string()));
        assert!(!request_tool_names.contains(&"graph_view".to_string()));

        let items = instance.available_tool_prompt_items().await;

        assert_eq!(tool_load_mode(&items, "tool_load"), "direct");
        assert_eq!(tool_load_mode(&items, "tool_call"), "direct");
        assert_eq!(tool_load_mode(&items, "read"), "direct");
        assert_eq!(tool_load_mode(&items, "edit"), "direct");
        assert_eq!(tool_load_mode(&items, "unity_execute"), "direct");
        assert_eq!(tool_load_mode(&items, "knowledge_edit"), "direct");
        assert_eq!(tool_load_mode(&items, "knowledge_create"), "lazy");
        assert_eq!(tool_load_mode(&items, "knowledge_move"), "lazy");
        assert_eq!(tool_load_mode(&items, "knowledge_delete"), "lazy");
        assert_eq!(tool_load_mode(&items, "unity_run_states"), "lazy");
        assert_eq!(tool_load_mode(&items, "unity_capture_viewport"), "lazy");
        assert_eq!(tool_load_mode(&items, "graph_view"), "lazy");
        assert_eq!(tool_load_mode(&items, "web_fetch"), "lazy");
        assert_eq!(
            tool_meta_bool(&items, "unity_capture_viewport", "directLoaded"),
            Some(false)
        );
        assert_eq!(
            tool_meta_bool(&items, "edit", "canConfigureDirectLoad"),
            Some(true)
        );
        assert_eq!(
            tool_meta_bool(&items, "tool_load", "canConfigureDirectLoad"),
            Some(false)
        );
    }

    #[tokio::test]
    async fn lazy_tool_names_are_injected_into_prompt_and_preview_items() {
        let temp = tempdir().expect("temp dir");
        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        let instance = AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: String::new(),
                env_template: String::new(),
                tools: vec![
                    "read".to_string(),
                    "web_fetch".to_string(),
                    "knowledge_create".to_string(),
                    "knowledge_delete".to_string(),
                    "unity_run_states".to_string(),
                    "unity_capture_viewport".to_string(),
                    "graph_view".to_string(),
                ],
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(ToolRegistry::with_builtins()),
            temp.path().to_string_lossy().to_string(),
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(None),
            Arc::new(None),
            KnowledgeAccessMode::Full,
            None,
            HashMap::new(),
            cancel_rx,
        );

        let parts = instance.build_system_prompt_parts().await;
        assert!(parts.env_prompt.contains("## Lazy Loaded Tools"));
        assert!(parts.env_prompt.contains("- `knowledge_create`"));
        assert!(parts.env_prompt.contains("- `knowledge_delete`"));
        assert!(parts.env_prompt.contains("- `unity_run_states`"));
        assert!(parts.env_prompt.contains("- `unity_capture_viewport`"));
        assert!(parts.env_prompt.contains("- `graph_view`"));
        assert!(parts.env_prompt.contains("- `web_fetch`"));
        assert!(!parts.env_prompt.contains("- `read`"));

        let items = instance.list_injected_prompt_items().await;
        let manifest = items
            .iter()
            .find(|item| item.id == "lazy_tool_names")
            .expect("lazy tool manifest item");
        assert_eq!(manifest.kind, "context");
        assert!(manifest.content.contains("- `knowledge_create`"));
        assert!(manifest.content.contains("- `knowledge_delete`"));
        assert!(manifest.content.contains("- `unity_run_states`"));
        assert!(manifest.content.contains("- `unity_capture_viewport`"));
        assert!(manifest.content.contains("- `graph_view`"));
        assert!(manifest.content.contains("- `web_fetch`"));
        assert!(!manifest.content.contains("- `read`"));
    }

    #[tokio::test]
    async fn skill_mode_tools_are_not_in_default_lazy_manifest() {
        let temp = tempdir().expect("temp dir");
        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        let mut registry = ToolRegistry::with_builtins();
        registry.register(noop_tool("skill_special"));
        let instance = AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: String::new(),
                env_template: String::new(),
                tools: vec![
                    "read".to_string(),
                    "knowledge_create".to_string(),
                    "skill_special".to_string(),
                ],
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(registry),
            temp.path().to_string_lossy().to_string(),
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(None),
            Arc::new(None),
            KnowledgeAccessMode::Full,
            None,
            HashMap::new(),
            cancel_rx,
        );

        let manifest_names = instance.lazy_tool_manifest_names().await;
        assert!(manifest_names.contains(&"knowledge_create".to_string()));
        assert!(!manifest_names.contains(&"read".to_string()));
        assert!(!manifest_names.contains(&"skill_special".to_string()));

        let items = instance.available_tool_prompt_items().await;
        assert_eq!(tool_load_mode(&items, "skill_special"), "skill");
        assert_eq!(
            tool_meta_bool(&items, "skill_special", "canConfigureDirectLoad"),
            Some(false)
        );
    }

    #[tokio::test]
    async fn knowledge_access_mode_filters_knowledge_tools() {
        let temp = tempdir().expect("temp dir");
        let tools = vec![
            "knowledge_list".to_string(),
            "knowledge_query".to_string(),
            "knowledge_read".to_string(),
            "knowledge_create".to_string(),
            "knowledge_edit".to_string(),
            "knowledge_move".to_string(),
            "knowledge_delete".to_string(),
        ];

        let disabled = test_agent_instance_with_tools_and_mode(
            temp.path().to_string_lossy().to_string(),
            tools.clone(),
            KnowledgeAccessMode::Disabled,
        );
        let disabled_tools = disabled.allowed_tool_set().await;
        assert!(disabled_tools
            .iter()
            .all(|name| !AgentInstance::is_knowledge_tool_name(name)));

        let read_only = test_agent_instance_with_tools_and_mode(
            temp.path().to_string_lossy().to_string(),
            tools,
            KnowledgeAccessMode::ReadOnly,
        );
        let read_only_tools = read_only.allowed_tool_set().await;
        for tool in ["knowledge_list", "knowledge_query", "knowledge_read"] {
            assert!(read_only_tools.contains(tool));
        }
        for tool in [
            "knowledge_create",
            "knowledge_edit",
            "knowledge_move",
            "knowledge_delete",
        ] {
            assert!(!read_only_tools.contains(tool));
        }
    }

    #[tokio::test]
    async fn knowledge_access_disabled_omits_context_injection() {
        let temp = tempdir().expect("temp dir");
        let instance = test_agent_instance_with_tools_and_mode(
            temp.path().to_string_lossy().to_string(),
            vec!["read".to_string(), "knowledge_read".to_string()],
            KnowledgeAccessMode::Disabled,
        );

        let parts = instance.build_system_prompt_parts().await;
        assert!(parts.knowledge_prompt.is_empty());
        assert!(!parts.env_prompt.contains("<knowledge_context>"));

        let items = instance.list_injected_prompt_items().await;
        assert!(items.iter().all(|item| item.id != "knowledge_context"));
        assert!(items
            .iter()
            .all(|item| !item.id.starts_with("knowledge_rule::")));
    }

    #[tokio::test]
    async fn available_tool_prompt_items_applies_tool_load_overrides() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();
        crate::commands::save_tool_direct_load_override(
            &working_dir,
            "test",
            "unity_run_states",
            true,
            false,
        )
        .expect("save unity override");
        crate::commands::save_tool_direct_load_override(&working_dir, "test", "edit", false, true)
            .expect("save edit override");

        let (_, cancel_rx) = tokio::sync::watch::channel(false);
        let instance = AgentInstance::new(
            Arc::new(AgentDef {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: String::new(),
                system_prompt: String::new(),
                env_template: String::new(),
                tools: vec![
                    "edit".to_string(),
                    "unity_run_states".to_string(),
                    "knowledge_create".to_string(),
                ],
                sub_agents: Vec::new(),
                default: false,
                default_effort: None,
                model_recommendation: None,
                source: "test".to_string(),
            }),
            "session-test",
            crate::agent::instance::LlmBackend::AnthropicAgentSdk,
            false,
            Arc::new(AgentDefRegistry::load(None, None)),
            Arc::new(ToolRegistry::with_builtins()),
            working_dir,
            RawContextStore::default(),
            None,
            "test-model".to_string(),
            None,
            Arc::new(None),
            Arc::new(None),
            KnowledgeAccessMode::Full,
            None,
            HashMap::new(),
            cancel_rx,
        );

        let items = instance.available_tool_prompt_items().await;

        assert_eq!(tool_load_mode(&items, "edit"), "lazy");
        assert_eq!(tool_load_mode(&items, "unity_run_states"), "direct");
        assert_eq!(tool_load_mode(&items, "knowledge_create"), "lazy");
        assert_eq!(
            tool_meta_bool(&items, "edit", "directLoadOverride"),
            Some(false)
        );
        assert_eq!(
            tool_meta_bool(&items, "unity_run_states", "directLoadOverride"),
            Some(true)
        );
    }

    #[test]
    fn selected_skill_reminder_injects_legacy_app_builtin_skill_content() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        let app_knowledge_dir = root.path().join("app-knowledge");
        let skill_dir = app_knowledge_dir.join("skill").join("builtin");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("profiler.md"),
            r#"---
id: kd_skill_builtin_profiler
type: skill
path: builtin/profiler.md
title: Unity Profiler Runtime Sampling
injectMode: none
summaryEnabled: true
commandEnabled: false
readOnly: true
aiMaintained: false
skillEnabled: true
skillSurface: auto
commandTrigger:
argumentHint:
createdAt: 1
updatedAt: 1
---

# Unity Profiler Runtime Sampling

## Summary
Profiler helper skill.

## Content
Use profiler helpers.
"#,
        )
        .expect("write profiler skill");

        let agent = test_agent_instance_with_prompts_and_app_knowledge_dir(
            workspace.to_string_lossy().to_string(),
            "",
            "",
            Some(app_knowledge_dir),
        );
        let intent = UserIntentPayload {
            kind: "user_intent_v1".to_string(),
            mode: "build".to_string(),
            skills: vec![UserIntentSkill {
                dir_name: "profiler".to_string(),
                source: "app".to_string(),
                name: "Unity Profiler Runtime Sampling".to_string(),
            }],
            client_message_id: None,
        };

        let reminder = agent.build_selected_skill_reminder(&intent);

        assert!(
            reminder.contains("Path: skill/builtin/profiler.md"),
            "{}",
            reminder
        );
        assert!(reminder.contains("Use profiler helpers."), "{}", reminder);
        assert!(!reminder.contains("knowledge_read"), "{}", reminder);
        assert!(
            !reminder.contains("Path: skill/profiler.md"),
            "{}",
            reminder
        );
    }

    fn sample_agent_knowledge_document(path: &str, title: &str) -> KnowledgeDocument {
        KnowledgeDocument {
            id: format!("kd_{}", title.replace(' ', "_").to_lowercase()),
            doc_type: KnowledgeType::Design,
            path: path.to_string(),
            title: title.to_string(),
            inject_mode: KnowledgeInjectMode::Excerpt,
            inherit_inject_mode: false,
            inject_mode_source: Default::default(),
            summary_enabled: true,
            command_enabled: true,
            read_only: false,
            ai_maintained: true,
            storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: Default::default(),
            explicit_maintenance_rules: true,
            external_source: None,
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            summary: Some("Summary".to_string()),
            body: "Body".to_string(),
            maintenance_rules: Some("Rules".to_string()),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn knowledge_routing_rejects_generic_fs_tools_for_knowledge_root() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join("Locus/knowledge/design"))
            .expect("create knowledge dir");
        std::fs::write(
            workspace.join("Locus/knowledge/design/core-loop.md"),
            "# Core Loop\n",
        )
        .expect("write knowledge doc");

        let agent = test_agent_instance(workspace.to_string_lossy().to_string());
        let expected = "Knowledge roots are reserved for knowledge tools. Use `knowledge_list` / `knowledge_query` / `knowledge_read` for inspection, `knowledge_create` / `knowledge_edit` / `knowledge_move` / `knowledge_delete` for non-Skill writes, and `skill_create` / `skill_reload` for Skill lifecycle work.".to_string();

        for (tool_name, args) in [
            (
                "read",
                json!({"filePath":"Locus/knowledge/design/core-loop.md"}),
            ),
            ("list", json!({"path":"Locus/knowledge/design"})),
            (
                "grep",
                json!({"path":"Locus/knowledge/design","pattern":"Core"}),
            ),
            (
                "write",
                json!({"filePath":"Locus/knowledge/design/core-loop.md","content":"updated"}),
            ),
            (
                "edit",
                json!({"filePath":"Locus/knowledge/design/core-loop.md","oldString":"Core","newString":"Loop"}),
            ),
            (
                "bash",
                json!({"workdir":"Locus/knowledge/design","command":"ls"}),
            ),
            (
                "bash",
                json!({"workdir":".","command":"cat Locus/knowledge/design/core-loop.md"}),
            ),
            (
                "bash",
                json!({"workdir":".","command":"mv Locus/knowledge/design/core-loop.md Locus/knowledge/design/core-loop-next.md"}),
            ),
            (
                "bash",
                json!({"workdir":".","command":"rm Locus/knowledge/design/core-loop.md"}),
            ),
            (
                "bash",
                json!({"workdir":".","command":"rm Locus/knowledge"}),
            ),
        ] {
            assert_eq!(
                agent.validate_knowledge_tool_routing(tool_name, &args),
                Some(expected.clone()),
                "tool {tool_name} should be rejected for knowledge paths"
            );
        }
    }

    #[test]
    fn knowledge_routing_allows_simple_git_commands_for_knowledge_paths() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join("Locus/knowledge/design"))
            .expect("create knowledge dir");
        std::fs::write(
            workspace.join("Locus/knowledge/design/core-loop.md"),
            "# Core Loop\n",
        )
        .expect("write knowledge doc");

        let agent = test_agent_instance(workspace.to_string_lossy().to_string());

        for command in [
            "git status --short Locus/knowledge/design/core-loop.md",
            "git diff -- Locus/knowledge/design/core-loop.md",
            "git add Locus/knowledge/design/core-loop.md",
            "git restore --staged Locus/knowledge/design/core-loop.md",
            "git reset -- Locus/knowledge/design/core-loop.md",
            "git restore -- Locus/knowledge/design/core-loop.md",
            "git checkout -- Locus/knowledge/design/core-loop.md",
            "git reset --hard HEAD",
            "git stash apply",
            "git stash pop",
            "git merge feature/knowledge",
            "git rebase main",
            "git cherry-pick HEAD~1",
            "git revert HEAD",
            "git add Locus/knowledge/design/core-loop.md && git commit -m 'docs: update knowledge'",
            "echo '--- diff ---' && git -c core.quotePath=false diff -- Locus/knowledge/design/core-loop.md | sed -n '1,80p'",
            "git -c core.quotePath=false diff --unified=1 -- Assets/PlayerHealthBar.cs Assets/PlayerPlatformerController.cs ProjectSettings/ProjectSettings.asset | sed -n '220,520p'\nprintf '%s\\n' '--- knowledge status ---'\ngit -c core.quotePath=false status --short -- Locus/knowledge",
        ] {
            assert_eq!(
                agent.validate_knowledge_tool_routing(
                    "bash",
                    &json!({"workdir":".","command":command})
                ),
                None,
                "git command should be allowed through routing: {command}"
            );
        }

        assert!(agent
            .validate_knowledge_tool_routing(
                "bash",
                &json!({"workdir":".","command":"git clean -fd Locus/knowledge/design"})
            )
            .is_some());
        assert!(agent
            .validate_knowledge_tool_routing(
                "bash",
                &json!({"workdir":".","command":"git -c core.quotePath=false diff --unified=1 -- Assets/PlayerHealthBar.cs Assets/PlayerPlatformerController.cs ProjectSettings/ProjectSettings.asset | sed -n '220,520p'\nprintf '%s\\n' '--- knowledge files ---'\nfind Locus/knowledge/design/system -maxdepth 3 -type f | sort"})
            )
            .is_some());
    }

    #[test]
    fn knowledge_routing_allows_generic_fs_tools_outside_knowledge_root() {
        let root = tempdir().expect("temp dir");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join("src")).expect("create src dir");
        std::fs::write(workspace.join("src/main.rs"), "fn main() {}\n").expect("write src file");

        let agent = test_agent_instance(workspace.to_string_lossy().to_string());

        for (tool_name, args) in [
            ("read", json!({"filePath":"src/main.rs"})),
            ("list", json!({"path":"src"})),
            ("grep", json!({"path":"src","pattern":"main"})),
            (
                "write",
                json!({"filePath":"src/next.rs","content":"fn next() {}\n"}),
            ),
            (
                "edit",
                json!({"filePath":"src/main.rs","oldString":"main","newString":"start"}),
            ),
            ("bash", json!({"workdir":"src","command":"pwd"})),
        ] {
            assert_eq!(
                agent.validate_knowledge_tool_routing(tool_name, &args),
                None,
                "tool {tool_name} should remain available outside knowledge roots"
            );
        }
    }

    #[test]
    fn knowledge_path_filter_accepts_type_prefixed_roots() {
        assert_eq!(
            crate::commands::resolve_knowledge_path_filter(None, Some("design/")).unwrap(),
            (Some(KnowledgeType::Design), None)
        );
        assert_eq!(
            crate::commands::resolve_knowledge_path_filter(None, Some("skill/unity/")).unwrap(),
            (Some(KnowledgeType::Skill), Some("unity".to_string()))
        );
    }

    #[test]
    fn prefix_knowledge_tool_path_includes_top_level_type() {
        assert_eq!(
            AgentInstance::prefix_knowledge_tool_path(KnowledgeType::Design, "core-loop.md"),
            "design/core-loop.md"
        );
        assert_eq!(
            AgentInstance::prefix_knowledge_tool_path(
                KnowledgeType::Skill,
                "skill/unity/import-pipeline.md"
            ),
            "skill/unity/import-pipeline.md"
        );
    }

    #[test]
    fn knowledge_create_preview_requires_approval_for_design_root() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let inspection = assess_knowledge_tool_confirmation(
            &working_dir,
            "knowledge_create",
            &json!({
                "kind": "document",
                "path": "design/combat/core-loop.md",
                "document": {
                    "body": "Body"
                }
            }),
        )
        .expect("inspect knowledge create")
        .expect("knowledge preview");

        assert!(inspection.governance_requires_confirm);
        let preview = inspection.preview;
        assert_eq!(preview.operation, KnowledgeToolConfirmOperation::Create);
        assert_eq!(
            preview.directory_mode,
            KnowledgeToolConfirmDirectoryMode::Approval
        );
        assert_eq!(preview.directory_path, "design/combat");
        assert_eq!(preview.path, "design/combat/core-loop.md");
        assert!(preview
            .document_after_text
            .as_deref()
            .is_some_and(|text| text.contains("## Content")));
    }

    #[test]
    fn knowledge_create_directory_preview_shows_structure_after_for_design_root() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let inspection = assess_knowledge_tool_confirmation(
            &working_dir,
            "knowledge_create",
            &json!({
                "kind": "directory",
                "path": "design/combat"
            }),
        )
        .expect("inspect knowledge create")
        .expect("knowledge preview");

        assert!(inspection.governance_requires_confirm);
        let preview = inspection.preview;
        assert_eq!(preview.operation, KnowledgeToolConfirmOperation::Create);
        assert_eq!(preview.path, "design/combat");
        assert_eq!(preview.directory_path, "design");
        assert_eq!(
            preview.directory_mode,
            KnowledgeToolConfirmDirectoryMode::Approval
        );
        assert!(preview.structure_before_paths.is_empty());
        assert_eq!(
            preview.structure_after_paths,
            vec!["design/combat".to_string()]
        );
    }

    #[test]
    fn knowledge_move_directory_preview_shows_before_and_after_structure() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(&working_dir, KnowledgeType::Design, "combat").expect("create dir");
        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_design_combat_move".to_string(),
                doc_type: KnowledgeType::Design,
                path: "combat/core-loop.md".to_string(),
                title: "Core Loop".to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: "Body".to_string(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save document");

        let inspection = assess_knowledge_tool_confirmation(
            &working_dir,
            "knowledge_move",
            &json!({
                "kind": "directory",
                "path": "design/combat",
                "newPath": "design/gameplay/combat"
            }),
        )
        .expect("inspect knowledge move")
        .expect("knowledge preview");

        assert!(inspection.governance_requires_confirm);
        let preview = inspection.preview;
        assert_eq!(preview.operation, KnowledgeToolConfirmOperation::Move);
        assert_eq!(preview.path, "design/combat");
        assert_eq!(preview.new_path.as_deref(), Some("design/gameplay/combat"));
        assert!(preview
            .structure_before_paths
            .contains(&"design/combat/core-loop.md".to_string()));
        assert!(preview
            .structure_after_paths
            .contains(&"design/gameplay/combat/core-loop.md".to_string()));
    }

    #[test]
    fn global_auto_mode_skips_confirmation_for_unity_execute_even_if_tool_is_ask() {
        assert!(!AgentInstance::permission_requires_confirm(
            "auto",
            Some("ask"),
            "unity_execute"
        ));
        assert!(!AgentInstance::permission_requires_confirm(
            "auto",
            None,
            "unity_execute"
        ));
    }

    #[test]
    fn global_auto_mode_skips_confirmation_for_write_even_if_tool_is_ask() {
        assert!(!AgentInstance::permission_requires_confirm(
            "auto",
            Some("ask"),
            "write"
        ));
        assert!(!AgentInstance::permission_requires_confirm(
            " auto ",
            Some(" ask "),
            "write"
        ));
    }

    #[test]
    fn global_auto_mode_keeps_governance_confirmation_for_design_knowledge_writes() {
        let assessment = AgentInstance::assess_tool_confirmation(
            "auto",
            Some("auto"),
            "knowledge_edit",
            "{\"path\":\"design/core.md\"}",
            None,
            true,
        );
        assert_eq!(
            assessment.reasons,
            vec![super::ToolConfirmReason::KnowledgeGovernance]
        );
    }

    #[test]
    fn global_auto_mode_skips_memory_knowledge_writes_when_governance_allows_it() {
        let assessment = AgentInstance::assess_tool_confirmation(
            "auto",
            Some("auto"),
            "knowledge_edit",
            "{\"path\":\"memory/project.md\"}",
            None,
            false,
        );
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn global_ask_mode_uses_per_tool_permission_settings() {
        assert!(!AgentInstance::permission_requires_confirm(
            "ask",
            Some("auto"),
            "unity_execute"
        ));
        assert!(AgentInstance::permission_requires_confirm(
            "ask",
            Some("ask"),
            "unity_execute"
        ));
        assert!(AgentInstance::permission_requires_confirm(
            "ask",
            None,
            "unity_execute"
        ));
        assert!(!AgentInstance::permission_requires_confirm(
            "ask",
            None,
            "unity_yaml_read"
        ));
        assert!(!AgentInstance::permission_requires_confirm(
            "ask", None, "list"
        ));
    }

    #[test]
    fn behavior_permission_defaults_to_approval_and_allows_auto_override() {
        assert!(AgentInstance::permission_setting_requires_confirm(
            None, true
        ));
        assert!(AgentInstance::permission_setting_requires_confirm(
            Some("ask"),
            true
        ));
        assert!(!AgentInstance::permission_setting_requires_confirm(
            Some("auto"),
            true
        ));
    }

    #[test]
    fn structure_section_renders_directory_summary_and_rules_on_same_line() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_design_combat_rules".to_string(),
                doc_type: KnowledgeType::Design,
                path: "combat/core-loop.md".to_string(),
                title: "Combat Core Loop".to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: "Body".to_string(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save combat doc");

        let mut config = default_directory_config_for_type(KnowledgeType::Design);
        config.summary = "Combat systems summary".to_string();
        config.inherit_inject_mode = false;
        config.inherit_ai_config = false;
        config.explicit_maintenance_rules = true;
        config.maintenance_rules = "- Keep verified combat structure only".to_string();
        update_directory_config(&working_dir, KnowledgeType::Design, "combat", config)
            .expect("update directory config");

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        assert!(structure
            .contains("combat/ :: Combat systems summary | - Keep verified combat structure only"));
    }

    #[test]
    fn structure_section_respects_directory_path_injection() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_design_combat".to_string(),
                doc_type: KnowledgeType::Design,
                path: "combat/core-loop.md".to_string(),
                title: "Combat Core Loop".to_string(),
                inject_mode: KnowledgeInjectMode::Excerpt,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: "Body".to_string(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save combat doc");

        let mut config = default_directory_config_for_type(KnowledgeType::Design);
        config.summary = "Combat systems summary".to_string();
        config.inject_mode = KnowledgeInjectMode::Path;
        config.inherit_inject_mode = false;
        update_directory_config(&working_dir, KnowledgeType::Design, "combat", config)
            .expect("update directory config");

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        assert!(structure.contains("combat/"));
        assert!(!structure.contains("combat/ :: Combat systems summary"));
        assert!(!structure.contains("Keep verified combat structure only"));
    }

    #[test]
    fn structure_section_keeps_skill_subdirectories_in_readable_paths() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_skill_builtin_create_skill".to_string(),
                doc_type: KnowledgeType::Skill,
                path: "builtin/create-skill.md".to_string(),
                title: "Create Skill".to_string(),
                inject_mode: KnowledgeInjectMode::None,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: true,
                command_enabled: true,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: Some(true),
                skill_surface: None,
                command_trigger: Some("/create-skill".to_string()),
                argument_hint: None,
                summary: Some("Create reusable Skills.".to_string()),
                body: "Body".to_string(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save skill doc");

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        let skill_index = structure.find("skill/ ::").unwrap_or_else(|| {
            panic!("expected skill section in structure:\n{}", structure);
        });
        let builtin_index = structure.find("builtin/").unwrap_or_else(|| {
            panic!("expected builtin directory in structure:\n{}", structure);
        });
        let doc_index = structure
            .find("create-skill.md :: create-skill")
            .unwrap_or_else(|| {
                panic!(
                    "expected create-skill document in structure:\n{}",
                    structure
                );
            });

        assert!(skill_index < builtin_index);
        assert!(builtin_index < doc_index);
    }

    #[test]
    fn structure_section_shows_builtin_memory_directory_rules_for_path_injection() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        assert!(structure.contains(
            "unity-project-understanding/ :: Maintains a structural cache of Unity project understanding"
        ));
        assert!(structure.contains(
            "Record only Unity project structure knowledge and lookup info that reduce repeated exploration"
        ));
    }

    #[test]
    fn structure_section_marks_empty_directories_explicitly() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        create_directory(&working_dir, KnowledgeType::Design, "combat").expect("create directory");

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        assert!(structure.contains("combat/"));
        assert!(structure.contains("└─ <empty>"));
    }

    #[test]
    fn structure_section_summarizes_managed_unity_reference_library() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let documents = vec![KnowledgeDocument {
            id: "kd_unity_execution_order".to_string(),
            doc_type: KnowledgeType::Reference,
            path: "unity-official-docs/manual/ExecutionOrder.md".to_string(),
            title: "Execution Order".to_string(),
            inject_mode: KnowledgeInjectMode::None,
            inherit_inject_mode: false,
            inject_mode_source: Default::default(),
            summary_enabled: false,
            command_enabled: false,
            read_only: true,
            ai_maintained: false,
            storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: Default::default(),
            explicit_maintenance_rules: false,
            external_source: None,
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            summary: None,
            body: "Execution order details".to_string(),
            maintenance_rules: None,
            created_at: 0,
            updated_at: 0,
        }];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed unity docs");
        let manifest_path = std::path::Path::new(&working_dir)
            .join("Locus")
            .join("knowledge")
            .join("reference")
            .join("unity_reference_docs_manifest.json");
        std::fs::create_dir_all(
            manifest_path
                .parent()
                .expect("unity manifest parent directory"),
        )
        .expect("create unity manifest parent");
        std::fs::write(
            &manifest_path,
            r#"{
  "projectVersion": "2022.3.47f1",
  "docsVersion": "2022.3",
  "locale": "zh-CN",
  "importedAt": 1,
  "importedDocCount": 1,
  "sourceUrl": "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
}"#,
        )
        .expect("write unity manifest");

        let structure = build_structure_section(&working_dir, None, KnowledgeAccessMode::Full)
            .expect("build structure");
        assert!(
            structure.contains("unity-official-docs/ :: Unity official reference library."),
            "{}",
            structure
        );
        assert!(
            structure.contains(
                "use `knowledge_query` or concrete `reference/unity-official-docs/...` paths when needed."
            ),
            "{}",
            structure
        );
        assert!(
            structure.contains("<1 file managed externally>"),
            "{}",
            structure
        );
        assert!(!structure.contains("ExecutionOrder.md"), "{}", structure);
        assert!(!structure.contains("manual/"), "{}", structure);
        assert!(
            !structure.contains("unity-official-docs/\n│  └─ <empty>"),
            "{}",
            structure
        );
    }

    #[test]
    fn l2_memory_section_keeps_project_mistake_note_in_knowledge_context() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let memory = build_l2_memory_section(&working_dir, None).expect("build l2 memory");
        assert!(memory.contains("### L2 Memory"));
        assert!(memory.contains("#### memory/project-mistake-note.md"));
        assert!(memory.contains("Rules:"));
        assert!(memory.contains("Body:\n<empty>"));
        assert!(!memory.contains("user-preference.md"));
    }

    #[test]
    fn l3_rule_section_shows_rules_and_empty_body_for_builtin_memory() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        let rules = build_l3_rule_section(&working_dir, None).expect("build l3 rules");
        assert!(rules.contains("## L3 Rules"));
        assert!(rules.contains("### User Preferences (memory/user-preference.md)"));
        assert!(rules.contains("Maintenance Rules:"));
        assert!(rules
            .contains("- Record only long-term user preferences that stay stable across tasks"));
        assert!(rules.contains(
            "- Keep each entry short and limited to stable preferences or hard constraints"
        ));
        assert!(rules.contains("- Keep the list within 20 items and merge similar preferences"));
        assert!(rules.contains("Full Document:\n<empty>"));
    }

    #[test]
    fn l3_rule_section_marks_empty_rules_and_body_explicitly() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_memory_empty".to_string(),
                doc_type: KnowledgeType::Memory,
                path: "empty-memory.md".to_string(),
                title: "Empty Memory".to_string(),
                inject_mode: KnowledgeInjectMode::Rule,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: String::new(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save empty memory");

        let rules = build_l3_rule_section(&working_dir, None).expect("build l3 rules");
        assert!(rules.contains("### Empty Memory (memory/empty-memory.md)"));
        assert!(rules.contains("Maintenance Rules:\n<empty>"));
        assert!(rules.contains("Full Document:\n<empty>"));
    }

    #[test]
    fn l3_rule_section_remaps_body_headings_relative_to_injected_context() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();

        save_document(
            &working_dir,
            crate::knowledge_store::KnowledgeDocument {
                id: "kd_memory_heading_map".to_string(),
                doc_type: crate::knowledge_store::KnowledgeType::Memory,
                path: "heading-map.md".to_string(),
                title: "Heading Map".to_string(),
                inject_mode: crate::knowledge_store::KnowledgeInjectMode::Rule,
                inherit_inject_mode: false,
                inject_mode_source: Default::default(),
                summary_enabled: false,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: Default::default(),
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: "# 一级\n## 二级\n正文".to_string(),
                maintenance_rules: None,
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save mapped memory");

        let rules = build_l3_rule_section(&working_dir, None).expect("build mapped rules");
        assert!(rules.contains("### Heading Map (memory/heading-map.md)"));
        assert!(rules.contains("Full Document:\n#### 一级\n##### 二级\n正文"));
    }

    #[test]
    fn system_prompt_parts_move_knowledge_out_of_env_prompt() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();
        let agent = test_agent_instance_with_prompts(
            working_dir,
            "You are a test agent.",
            "# Environment\nWorking directory: <working_dir>\n{{#knowledge}}\n\n<knowledge_context>\n{{/knowledge}}\n{{#knowledge_index}}{{/knowledge_index}}\n{{#knowledge_memory}}{{/knowledge_memory}}\n",
        );

        let prompt_parts = tokio::runtime::Runtime::new()
            .expect("create runtime")
            .block_on(agent.build_system_prompt_parts());

        assert_eq!(prompt_parts.base_prompt, "You are a test agent.");
        assert!(prompt_parts.knowledge_prompt.contains("## Knowledge"));
        assert!(prompt_parts.knowledge_prompt.contains("### L2 Memory"));
        assert!(prompt_parts
            .knowledge_prompt
            .contains("#### memory/project-mistake-note.md"));
        let search_index = prompt_parts
            .knowledge_prompt
            .find("### Search")
            .expect("search section");
        let tools_index = prompt_parts
            .knowledge_prompt
            .find("### Tools")
            .expect("tools section");
        let memory_index = prompt_parts
            .knowledge_prompt
            .find("### L2 Memory")
            .expect("l2 memory section");
        assert!(search_index < tools_index);
        assert!(tools_index < memory_index);
        assert!(!prompt_parts.knowledge_prompt.contains("## L3 Rules"));
        assert!(!prompt_parts.knowledge_prompt.contains("Full Document:"));
        assert!(prompt_parts.rules_prompt.contains("## L3 Rules"));
        assert!(prompt_parts
            .rules_prompt
            .contains("### User Preferences (memory/user-preference.md)"));
        assert!(prompt_parts.env_prompt.contains("Working directory:"));
        assert!(!prompt_parts.env_prompt.contains("## Knowledge"));
        assert!(!prompt_parts.env_prompt.contains("project-mistake-note.md"));
        assert!(!prompt_parts.env_prompt.contains("user-preference.md"));
    }

    #[test]
    fn knowledge_list_output_uses_one_path_per_line() {
        let output = AgentInstance::format_knowledge_list_output(&[
            AgentKnowledgeListItem {
                doc_type: KnowledgeType::Design,
                path: "design/project-overview.md".to_string(),
                title: "Project Overview".to_string(),
            },
            AgentKnowledgeListItem {
                doc_type: KnowledgeType::Skill,
                path: "skill/create-skill.md".to_string(),
                title: "Create Skill".to_string(),
            },
        ]);

        assert_eq!(output, "design/project-overview.md\nskill/create-skill.md");
    }

    #[test]
    fn knowledge_create_patch_treats_blank_fields_as_noop() {
        let patch = AgentKnowledgeDocumentContentPatch {
            summary: Some(None),
            body: Some(String::new()),
            maintenance_rules: Some(None),
        };

        assert!(!patch.is_empty());
        assert!(patch.is_noop_for_create());
    }

    #[test]
    fn knowledge_create_patch_keeps_real_content_non_noop() {
        let patch = AgentKnowledgeDocumentContentPatch {
            summary: Some(Some("Summary".to_string())),
            body: Some(String::new()),
            maintenance_rules: Some(None),
        };

        assert!(!patch.is_noop_for_create());
    }

    #[test]
    fn knowledge_query_output_uses_plain_text_blocks() {
        let output = AgentInstance::format_knowledge_query_output(&[AgentKnowledgeSearchHit {
            doc_type: KnowledgeType::Design,
            path: "design/project-overview.md".to_string(),
            title: "Project Overview".to_string(),
            snippet: "Core loop summary".to_string(),
            matched_section: Some(KnowledgeSearchMatchSection::Summary),
            score: 0.875,
            match_kind: "lexical".to_string(),
        }]);

        assert!(output.contains("design/project-overview.md"));
        assert!(output.contains("Project Overview"));
        assert!(output.contains("match=lexical | section=summary | score=0.875"));
        assert!(output.contains("Core loop summary"));
        assert!(!output.trim_start().starts_with('{'));
        assert!(!output.trim_start().starts_with('['));
    }

    #[test]
    fn knowledge_read_output_renders_markdown_for_full_part() {
        let output = AgentInstance::format_knowledge_read_output(&AgentKnowledgeReadResponse {
            document: AgentKnowledgeDocumentContent {
                doc_type: KnowledgeType::Design,
                path: "design/project-overview.md".to_string(),
                title: "Project Overview".to_string(),
                summary: Some("Summary".to_string()),
                maintenance_rules: Some("Rules".to_string()),
                body: Some("Body".to_string()),
            },
            part: "full".to_string(),
        });

        assert_eq!(
            output,
            "# Project Overview\n\n## Summary\nSummary\n\n## Maintenance Rules\nRules\n\n## Content\nBody\n"
        );
    }

    #[test]
    fn knowledge_read_output_returns_plain_text_for_summary_part() {
        let output = AgentInstance::format_knowledge_read_output(&AgentKnowledgeReadResponse {
            document: AgentKnowledgeDocumentContent {
                doc_type: KnowledgeType::Design,
                path: "design/project-overview.md".to_string(),
                title: "Project Overview".to_string(),
                summary: Some("Summary".to_string()),
                maintenance_rules: None,
                body: None,
            },
            part: "summary".to_string(),
        });

        assert_eq!(output, "Summary");
    }

    #[test]
    fn knowledge_mutation_output_uses_plain_text_for_create() {
        let output = AgentInstance::format_knowledge_mutation_output(
            "Created",
            &AgentKnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Document,
                doc_type: KnowledgeType::Design,
                path: "design/project-overview.md".to_string(),
                result_path: Some("design/project-overview.md".to_string()),
                document: None,
            },
        );

        assert_eq!(
            output,
            "Created knowledge document design/project-overview.md"
        );
    }

    #[test]
    fn knowledge_mutation_output_uses_arrow_for_move() {
        let output = AgentInstance::format_knowledge_mutation_output(
            "Moved",
            &AgentKnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Directory,
                doc_type: KnowledgeType::Design,
                path: "design/combat".to_string(),
                result_path: Some("design/gameplay/combat".to_string()),
                document: None,
            },
        );

        assert_eq!(
            output,
            "Moved knowledge directory design/combat -> design/gameplay/combat"
        );
    }

    #[test]
    fn execute_knowledge_list_returns_plain_text_paths() {
        let temp = tempdir().expect("temp dir");
        let working_dir = temp.path().to_string_lossy().to_string();
        save_document(
            &working_dir,
            sample_agent_knowledge_document("project-overview.md", "Project Overview"),
        )
        .expect("save knowledge doc");

        let agent = test_agent_instance(working_dir);
        let result = agent.execute_knowledge_list(&json!({"pathPrefix":"design/"}));

        assert!(!result.is_error);
        assert_eq!(result.output, "design/project-overview.md");
    }

    #[test]
    fn sanitize_knowledge_read_response_keeps_only_document_content() {
        let response = KnowledgeReadResponse {
            kind: KnowledgeTargetKind::Document,
            document: Some(KnowledgeReadResult {
                document: KnowledgeDocument {
                    id: "kd_design_test".to_string(),
                    doc_type: KnowledgeType::Design,
                    path: "design/project-overview.md".to_string(),
                    title: "Project Overview".to_string(),
                    inject_mode: KnowledgeInjectMode::Excerpt,
                    inherit_inject_mode: false,
                    inject_mode_source: Default::default(),
                    summary_enabled: true,
                    command_enabled: true,
                    read_only: false,
                    ai_maintained: true,
                    storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                    inherit_ai_config: false,
                    ai_config_source: Default::default(),
                    explicit_maintenance_rules: true,
                    external_source: None,
                    skill_enabled: None,
                    skill_surface: None,
                    command_trigger: None,
                    argument_hint: None,
                    summary: Some("Summary".to_string()),
                    body: "Body".to_string(),
                    maintenance_rules: Some("Rules".to_string()),
                    created_at: 0,
                    updated_at: 0,
                },
                part: "full".to_string(),
                file_metadata: None,
            }),
            directory: None,
        };

        let sanitized = AgentInstance::sanitize_knowledge_read_response(response)
            .expect("sanitize read response");
        let value = serde_json::to_value(sanitized).expect("serialize sanitized read");

        assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("design"));
        assert_eq!(
            value.get("path").and_then(|v| v.as_str()),
            Some("design/project-overview.md")
        );
        assert!(value.get("summary").is_some());
        assert!(value.get("maintenanceRules").is_some());
        assert!(value.get("body").is_some());
        assert!(value.get("scope").is_none());
        assert!(value.get("injectMode").is_none());
        assert!(value.get("aiMaintained").is_none());
    }

    #[test]
    fn sanitize_knowledge_mutation_response_omits_directory_metadata() {
        let response = crate::knowledge_store::KnowledgeMutationResponse {
            kind: KnowledgeTargetKind::Directory,
            doc_type: KnowledgeType::Design,
            path: "design/combat".to_string(),
            result_path: Some("design/combat".to_string()),
            document: None,
            directory: Some(crate::knowledge_store::KnowledgeDirectoryConfigRecord {
                doc_type: KnowledgeType::Design,
                path: "combat".to_string(),
                config_path: "combat".to_string(),
                exists: true,
                read_only: false,
                updated_at: 0,
                inject_mode_source: Default::default(),
                ai_config_source: Default::default(),
                effective_lexical_search: crate::knowledge_store::EffectiveCapabilityState {
                    enabled: true,
                    source: "default".to_string(),
                    reason_code: None,
                    source_dir: None,
                },
                effective_vector_search: crate::knowledge_store::EffectiveCapabilityState {
                    enabled: true,
                    source: "default".to_string(),
                    reason_code: None,
                    source_dir: None,
                },
                external_sources: Vec::new(),
                config: default_directory_config_for_type(KnowledgeType::Design),
            }),
        };

        let sanitized = AgentInstance::sanitize_knowledge_mutation_response(response);
        let value = serde_json::to_value(sanitized).expect("serialize sanitized mutation");

        assert_eq!(
            value.get("kind").and_then(|v| v.as_str()),
            Some("directory")
        );
        assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("design"));
        assert_eq!(
            value.get("path").and_then(|v| v.as_str()),
            Some("design/combat")
        );
        assert!(value.get("document").is_none());
        assert!(value.get("directory").is_none());
    }
}
