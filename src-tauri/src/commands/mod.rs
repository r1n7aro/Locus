pub mod asset;
mod auth;
mod canvas;
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
mod undo;
mod workspace;
mod update;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

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
    TextDelta { session_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta { session_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallDone {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        output: String,
        outcome: ToolCallOutcome,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallDelta {
        session_id: String,
        tool_call_id: String,
        delta: String,
    },
    #[serde(rename_all = "camelCase")]
    SubagentToolCallStart {
        session_id: String,
        parent_tool_call_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
    },
    #[serde(rename_all = "camelCase")]
    SubagentToolCallDone {
        session_id: String,
        parent_tool_call_id: String,
        tool_call_id: String,
        tool_name: String,
        output: String,
        outcome: ToolCallOutcome,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallRoundDone {
        session_id: String,
        message_id: String,
        full_text: String,
        tool_calls: Vec<crate::session::models::ToolCallInfo>,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        session_id: String,
        message_id: String,
        full_text: String,
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
        messages: Vec<crate::session::models::ChatMessage>,
    },
    #[serde(rename_all = "camelCase")]
    Cancelled { session_id: String },
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
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolConfirmDisplay {
    Basic(BasicToolConfirmDisplay),
    Knowledge(KnowledgeToolConfirmPreview),
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
}

pub use asset::*;
pub use auth::*;
pub use canvas::*;
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
pub use undo::*;
pub use workspace::*;
pub use update::*;
