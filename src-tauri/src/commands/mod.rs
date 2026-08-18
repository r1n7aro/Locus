pub mod asset;
mod auth;
mod csharp_lsp;
mod diff;
mod extra_workdirs;
mod fonts;
mod git;
mod knowledge;
mod log;
mod mcp;
mod plan;
mod plugin;
mod ref_graph;
mod session;
mod skill;
mod skill_external;
mod storage;
mod sub_window;
mod system;
mod undo;
mod unity_embed;
mod unity_serialized_property;
mod update;
mod view;
mod workspace;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::AppError;

pub const SESSION_CONTENT_CHANGED_EVENT: &str = "session-content-changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContentChangedEvent {
    pub working_dir: String,
    pub session_id: String,
    pub source: String,
    pub changed_at: i64,
}

fn unix_time_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn emit_session_content_changed(
    app_handle: &AppHandle,
    working_dir: &str,
    session_id: &str,
    source: &str,
) {
    let event = SessionContentChangedEvent {
        working_dir: working_dir.to_string(),
        session_id: session_id.to_string(),
        source: source.to_string(),
        changed_at: unix_time_millis(),
    };
    if let Err(error) = app_handle.emit(SESSION_CONTENT_CHANGED_EVENT, event) {
        eprintln!(
            "[Locus] failed to emit session content changed event for session {}: {}",
            session_id, error
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallOutcome {
    Done,
    Error,
    Interrupted,
}

/// What initiated a context compaction. `Reactive` means the request was
/// already sent and the server rejected it as over the context window, so the
/// UI should warn the user instead of compacting silently.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompactTrigger {
    Auto,
    Manual,
    Reactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamEvent {
    #[serde(rename_all = "camelCase")]
    RunStart { session_id: String },
    #[serde(rename_all = "camelCase")]
    UserMessage {
        session_id: String,
        message: crate::session::models::ChatMessage,
    },
    #[serde(rename_all = "camelCase")]
    PendingInputQueued {
        session_id: String,
        input: crate::session::models::PendingSessionInput,
    },
    #[serde(rename_all = "camelCase")]
    PendingInputDeleted {
        session_id: String,
        pending_input_id: String,
    },
    #[serde(rename_all = "camelCase")]
    PendingInputAccepted {
        session_id: String,
        pending_input_id: String,
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    TextDelta {
        session_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_seq: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        session_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_seq: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_seq: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallDone {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        output: String,
        outcome: ToolCallOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<crate::session::models::ImageData>>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallDelta {
        session_id: String,
        tool_call_id: String,
        delta: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallProgress {
        session_id: String,
        tool_call_id: String,
        title: String,
        info: String,
        progress: Option<f32>,
        state: String,
    },
    #[serde(rename_all = "camelCase")]
    SubagentToolCallStart {
        session_id: String,
        parent_tool_call_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_seq: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    SubagentToolCallDone {
        session_id: String,
        parent_tool_call_id: String,
        tool_call_id: String,
        tool_name: String,
        output: String,
        outcome: ToolCallOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<crate::session::models::ImageData>>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallRoundDone {
        session_id: String,
        message_id: String,
        full_text: String,
        tool_calls: Vec<crate::session::models::ToolCallInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_parts: Option<Vec<crate::session::models::AssistantRenderPart>>,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        session_id: String,
        message_id: String,
        full_text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_order: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_parts: Option<Vec<crate::session::models::AssistantRenderPart>>,
    },
    #[serde(rename_all = "camelCase")]
    KnowledgeProposal {
        session_id: String,
        message: crate::session::models::ChatMessage,
    },
    #[serde(rename_all = "camelCase")]
    UsageUpdate {
        session_id: String,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        #[serde(default)]
        cache_invalidated: bool,
        #[serde(default)]
        cache_baseline_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_invalidation_reason: Option<String>,
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_cache_read_tokens: u64,
        total_cache_write_tokens: u64,
        #[serde(default)]
        timed_output_tokens: u64,
        #[serde(default)]
        model_active_duration_ms: u64,
        total_cost_usd: f64,
        priced_rounds: u64,
        context_tokens: u32,
        context_limit: u32,
    },
    #[serde(rename_all = "camelCase")]
    AskUser {
        session_id: String,
        question_id: String,
        tool_call_id: String,
        question: String,
        options: Vec<AskOption>,
    },
    #[serde(rename_all = "camelCase")]
    ToolConfirm {
        session_id: String,
        question_id: String,
        tool_call_id: String,
        display: ToolConfirmDisplay,
    },
    /// Sticky plan-mode transitions (entered via /plan or a plan-tagged
    /// message; exited via approved exit_plan_mode or the user toggle).
    #[serde(rename_all = "camelCase")]
    PlanModeChanged {
        session_id: String,
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_file_path: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    InputAnswered {
        session_id: String,
        question_id: String,
    },
    #[serde(rename_all = "camelCase")]
    UndoAvailable {
        session_id: String,
        assistant_message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    CompactStart {
        session_id: String,
        context_tokens: u32,
        context_limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<CompactTrigger>,
    },
    #[serde(rename_all = "camelCase")]
    CompactDone {
        session_id: String,
        messages_before: u32,
        messages_after: u32,
        #[serde(default)]
        context_tokens: u32,
        #[serde(default)]
        context_limit: u32,
        messages: Vec<crate::session::models::ChatMessage>,
    },
    #[serde(rename_all = "camelCase")]
    Cancelled {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        full_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_duration: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        render_parts: Option<Vec<crate::session::models::AssistantRenderPart>>,
        /// Set when the run was cancelled before producing any assistant
        /// output: the user message was removed from the session so the
        /// composer can take the text back as a draft.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        removed_user_message: Option<crate::session::models::ChatMessage>,
    },
    #[serde(rename_all = "camelCase")]
    Error { session_id: String, error: AppError },
}

/// Wrapper that adds a run_id to every StreamEvent for filtering stale events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEventEnvelope {
    pub run_id: String,
    #[serde(flatten)]
    pub event: StreamEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeToolConfirmDirectoryMode {
    Auto,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeToolConfirmOperation {
    Create,
    Edit,
    Move,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicToolConfirmDisplay {
    pub tool_name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_review: Option<AutoReviewSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoReviewSummary {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeToolConfirmPreview {
    pub operation: KnowledgeToolConfirmOperation,
    pub target_kind: crate::knowledge_store::KnowledgeTargetKind,
    pub doc_type: crate::knowledge_store::KnowledgeType,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_path: Option<String>,
    pub directory_path: String,
    pub directory_mode: KnowledgeToolConfirmDirectoryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_before_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_after_text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_before_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_after_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityEditorStatusChangeConfirmDisplay {
    pub tool_name: String,
    pub current_status: String,
    pub requested_status: String,
}

/// Plan-approval dialog payload for exit_plan_mode: the full plan text plus
/// the plan file location.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalConfirmDisplay {
    pub plan: String,
    pub plan_file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolConfirmDisplay {
    Basic(BasicToolConfirmDisplay),
    Knowledge(KnowledgeToolConfirmPreview),
    UnityEditorStatusChange(UnityEditorStatusChangeConfirmDisplay),
    PlanApproval(PlanApprovalConfirmDisplay),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_write_tokens: u64,
    #[serde(default)]
    pub timed_output_tokens: u64,
    #[serde(default)]
    pub model_active_duration_ms: u64,
    pub total_cost_usd: f64,
    pub priced_rounds: u64,
    pub context_tokens: u32,
    pub context_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextBreakdown {
    pub system_prompt_tokens: u32,
    pub environment_tokens: u32,
    pub rules_tokens: u32,
    pub knowledge_tokens: u32,
    pub runtime_injection_tokens: u32,
    pub conversation_tokens: u32,
    pub tool_definition_tokens: u32,
    pub active_tool_result_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextToolUsage {
    pub name: String,
    pub call_count: u32,
    pub result_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCacheInvalidation {
    pub message_id: String,
    pub message: String,
    pub model_id: String,
    pub baseline_tokens: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub excess_input_tokens: u64,
    pub reason: String,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextUsageReport {
    pub session_id: String,
    pub session_title: String,
    pub agent_id: String,
    pub model_id: String,
    pub context_tokens: u32,
    pub context_limit: u32,
    pub raw_estimated_context_tokens: u32,
    pub reported_context_tokens: u32,
    pub breakdown: SessionContextBreakdown,
    pub tools: Vec<SessionContextToolUsage>,
    pub cache_invalidations: Vec<SessionCacheInvalidation>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageMetrics {
    pub request_count: u64,
    pub session_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageGroup {
    pub model_id: String,
    pub provider: String,
    pub usage: ModelUsageMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageReport {
    pub usage: ModelUsageMetrics,
    pub by_model: Vec<ModelUsageGroup>,
    pub recorded_from: Option<i64>,
    pub recorded_to: Option<i64>,
}

pub use asset::*;
pub use auth::*;
pub use csharp_lsp::*;
pub use diff::*;
pub use extra_workdirs::*;
pub use fonts::*;
pub use git::*;
pub use knowledge::*;
pub use log::*;
pub use mcp::*;
pub use plan::*;
pub use plugin::*;
pub use ref_graph::*;
pub use session::*;
pub use skill::*;
pub use skill_external::*;
pub use storage::*;
pub use sub_window::*;
pub use system::*;
pub use undo::*;
pub use unity_embed::*;
pub use unity_serialized_property::*;
pub use update::*;
pub use view::*;
pub use workspace::*;
