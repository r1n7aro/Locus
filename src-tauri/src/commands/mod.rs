mod agent_graph;
pub mod asset;
mod auth;
mod diff;
mod fonts;
mod git;
mod knowledge;
mod log;
mod plan;
mod ref_graph;
mod session;
mod skill;
mod storage;
mod system;
mod undo;
mod unity_embed;
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
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_cache_read_tokens: u64,
        total_cache_write_tokens: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolConfirmDisplay {
    Basic(BasicToolConfirmDisplay),
    Knowledge(KnowledgeToolConfirmPreview),
    UnityEditorStatusChange(UnityEditorStatusChangeConfirmDisplay),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_write_tokens: u64,
    pub total_cost_usd: f64,
    pub priced_rounds: u64,
    pub context_tokens: u32,
    pub context_limit: u32,
}

pub use agent_graph::*;
pub use asset::*;
pub use auth::*;
pub use diff::*;
pub use fonts::*;
pub use git::*;
pub use knowledge::*;
pub use log::*;
pub use plan::*;
pub use ref_graph::*;
pub use session::*;
pub use skill::*;
pub use storage::*;
pub use system::*;
pub use undo::*;
pub use unity_embed::*;
pub use update::*;
pub use view::*;
pub use workspace::*;
