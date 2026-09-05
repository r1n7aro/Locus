use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub agent_id: Option<String>,
    pub session_type: String,
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_checkout_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_target: Option<SessionExecutionTarget>,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_status: Option<SessionRuntimeStatus>,
}

/// Sticky Claude Code-style plan mode state for a session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanModeState {
    pub active: bool,
    pub exited_pending_notice: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionRuntimeStatus {
    Running,
    Queued,
    Starting,
    WaitingInput,
    Finishing,
    Cancelling,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub id: String,
    pub title: String,
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fast_mode: Option<bool>,
    pub session_type: String,
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_checkout_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_completed_run_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub pending_inputs: Vec<PendingSessionInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<SessionRuntimeSnapshot>,
}

/// Display-oriented session payload used by the main chat workspace.
///
/// The regular `SessionDetail` remains the full-history contract for exports,
/// context reconstruction, and compatibility callers. The workspace loads a
/// bounded tail page first and requests older pages with the stable SQLite
/// row-id cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionViewSnapshot {
    pub session: SessionDetail,
    #[serde(default)]
    pub user_message_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_message_row_id: Option<i64>,
    pub has_more_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurnPreview {
    pub message_id: String,
    pub prompt: String,
    pub response: String,
    #[serde(default)]
    pub images: Vec<ImageData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessagePage {
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_message_row_id: Option<i64>,
    pub has_more_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunSummary {
    pub run_id: String,
    pub session_id: String,
    pub status: String,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Stable checkout metadata persisted independently from the in-memory
/// workspace runtime so historical sessions and runs keep an auditable path
/// binding across process restarts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckoutRecord {
    pub checkout_id: String,
    pub project_id: String,
    pub root_path: String,
    pub normalized_root: String,
    pub last_opened_at: i64,
}

/// Persisted configuration for one optional service hosted by a checkout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServiceRecord {
    pub checkout_id: String,
    pub service_kind: String,
    pub service_instance_id: String,
    pub enabled: bool,
    pub activation_policy: String,
    pub local_config: serde_json::Value,
}

/// Immutable service identity captured when an Agent run starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunServiceBinding {
    pub service_kind: String,
    pub service_instance_id: String,
    pub runtime_generation: u64,
}

/// Display and audit metadata for the checkout used by a session run. The
/// branch and commit are captured when the execution context is created so a
/// later branch switch or rename cannot rewrite session history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionExecutionTarget {
    pub checkout_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_oid: Option<String>,
}

/// Scoped execution snapshot supplied by the workspace registry when a run
/// starts. An empty `service_bindings` list means the run was known to have no
/// optional services; a NULL database value is reserved for historical runs
/// whose bindings were never captured.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRunScopeSnapshot {
    pub project_id: String,
    pub checkout_id: String,
    pub workspace_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_oid: Option<String>,
    #[serde(default)]
    pub service_bindings: Vec<SessionRunServiceBinding>,
}

/// Full persisted run row used by context export and diagnostics. The
/// existing `SessionRunSummary` remains the compatibility IPC payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSessionRun {
    #[serde(flatten)]
    pub summary: SessionRunSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_oid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_bindings: Option<Vec<SessionRunServiceBinding>>,
}

/// Session-level project grouping and default checkout binding. Historical
/// rows can have no checkout, while retaining their original project id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionWorkspaceScope {
    pub project_id: Option<String>,
    pub default_checkout_id: Option<String>,
    pub checkout_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerNode {
    pub node_id: String,
    pub project_id: String,
    pub node_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerPresetSummary {
    pub preset_id: String,
    pub name: String,
    pub revision: i64,
    pub active: bool,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerSnapshot {
    pub project_id: String,
    pub preset_id: String,
    pub preset_name: String,
    pub manifest_path: String,
    pub revision: i64,
    pub nodes: Vec<ProjectExplorerNode>,
    pub presets: Vec<ProjectExplorerPresetSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProjectExplorerOperation {
    CreateFolder {
        #[serde(default)]
        node_id: Option<String>,
        #[serde(default)]
        parent_node_id: Option<String>,
        name: String,
        position: i64,
    },
    RenameFolder {
        node_id: String,
        name: String,
    },
    DeleteFolder {
        node_id: String,
    },
    MoveNode {
        node_id: String,
        #[serde(default)]
        parent_node_id: Option<String>,
        position: i64,
    },
    PlaceResource {
        #[serde(default)]
        node_id: Option<String>,
        resource_kind: String,
        resource_id: String,
        #[serde(default)]
        source_kind: Option<String>,
        #[serde(default)]
        parent_node_id: Option<String>,
        position: i64,
    },
    RemoveResourcePlacement {
        resource_kind: String,
        resource_id: String,
    },
    MountPath {
        #[serde(default)]
        node_id: Option<String>,
        #[serde(default)]
        parent_node_id: Option<String>,
        path: String,
        #[serde(default)]
        source_kind: Option<String>,
        #[serde(default)]
        name: Option<String>,
        position: i64,
    },
    SetNodeHidden {
        node_id: String,
        hidden: bool,
    },
    RemoveNode {
        node_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerMutationResult {
    pub operation_id: String,
    pub snapshot: ProjectExplorerSnapshot,
}

#[cfg(test)]
mod project_explorer_operation_tests {
    use super::ProjectExplorerOperation;

    #[test]
    fn deserializes_camel_case_frontend_fields() {
        let operation = serde_json::from_value::<ProjectExplorerOperation>(serde_json::json!({
            "kind": "placeResource",
            "nodeId": "knowledge-node",
            "resourceKind": "knowledge",
            "resourceId": "memory-a",
            "parentNodeId": "knowledge-type:project-a:memory",
            "position": 2
        }))
        .expect("frontend explorer operation should deserialize");

        assert_eq!(
            operation,
            ProjectExplorerOperation::PlaceResource {
                node_id: Some("knowledge-node".to_string()),
                resource_kind: "knowledge".to_string(),
                resource_id: "memory-a".to_string(),
                source_kind: None,
                parent_node_id: Some("knowledge-type:project-a:memory".to_string()),
                position: 2,
            }
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallDisplayStatus {
    Running,
    Done,
    Error,
    Interrupted,
}

impl ToolCallDisplayStatus {
    pub fn from_outcome(outcome: crate::commands::ToolCallOutcome) -> Self {
        match outcome {
            crate::commands::ToolCallOutcome::Done => Self::Done,
            crate::commands::ToolCallOutcome::Error => Self::Error,
            crate::commands::ToolCallOutcome::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallProgressSnapshot {
    pub title: String,
    pub info: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub status: ToolCallDisplayStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ToolCallProgressSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_tool_calls: Option<Vec<ToolCallDisplay>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub question_id: String,
    pub tool_call_id: String,
    pub question: String,
    pub options: Vec<crate::commands::AskOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextAttempt {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub iteration: u32,
    pub attempt: u32,
    pub attempt_kind: String,
    pub status: String,
    pub backend: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub request: serde_json::Value,
    pub response: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingToolConfirm {
    pub question_id: String,
    pub tool_call_id: String,
    pub display: crate::commands::ToolConfirmDisplay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeSnapshot {
    pub active_run: SessionRunSummary,
    #[serde(default)]
    pub active_tool_calls: Vec<ToolCallDisplay>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub streaming_text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub streaming_thinking: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub live_render_parts: Vec<AssistantRenderPart>,
    #[serde(default)]
    pub stream_sequence: u32,
    #[serde(default)]
    pub streaming_text_order: u32,
    #[serde(default)]
    pub thinking_order: u32,
    #[serde(default)]
    pub is_thinking: bool,
    #[serde(default)]
    pub thinking_duration: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestion>,
    #[serde(default)]
    pub pending_tool_confirms: Vec<PendingToolConfirm>,
    #[serde(default)]
    pub is_compacting: bool,
    #[serde(default)]
    pub compact_queued: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventRecord {
    pub session_id: String,
    pub run_id: String,
    pub seq: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "user" => Ok(MessageRole::User),
            "assistant" => Ok(MessageRole::Assistant),
            "tool" => Ok(MessageRole::Tool),
            _ => Err(format!("Unknown role: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerToolKind {
    WebSearch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_tool: Option<ServerToolKind>,
    /// Pre-computed output for server tools (e.g. web_search) that don't need local execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<crate::commands::ToolCallOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_tool_calls: Option<Vec<ToolCallInfo>>,
}

impl ToolCallInfo {
    pub fn is_server_tool(&self) -> bool {
        self.server_tool.is_some() || self.server_tool_output.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOrderKey {
    pub run_id: String,
    pub seq: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CitationKind {
    Url,
    File,
    ContainerFile,
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub id: String,
    pub kind: CitationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AssistantRenderPart {
    #[serde(rename_all = "camelCase")]
    Thinking {
        id: String,
        order: RenderOrderKey,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        id: String,
        order: RenderOrderKey,
        content: String,
        #[serde(default)]
        citations: Vec<Citation>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        id: String,
        order: RenderOrderKey,
        tool_call: ToolCallInfo,
    },
    #[serde(rename_all = "camelCase")]
    KnowledgeProposal {
        id: String,
        order: RenderOrderKey,
        message: Box<ChatMessage>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageData {
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetRefData {
    pub path: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIntentSkill {
    pub dir_name: String,
    pub source: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIntentPayload {
    pub kind: String,
    pub mode: String,
    #[serde(default)]
    pub skills: Vec<UserIntentSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSessionInput {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub merge_group_id: String,
    pub status: String,
    #[serde(default = "default_pending_input_delivery")]
    pub delivery: String,
    pub text: String,
    pub display_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_refs: Option<Vec<AssetRefData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<UserIntentPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_pending_input_delivery() -> String {
    "after_run".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoSnapshot {
    pub items: Vec<TodoItem>,
    pub latest_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProposalVerify {
    None,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProposalStatus {
    Pending,
    Applying,
    Applied,
    Invalidated,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProposalItemKind {
    Memory,
    #[serde(alias = "wiki")]
    Knowledge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProposalItemMode {
    Replace,
    CreateSource,
    UpdateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeProposalItem {
    pub kind: KnowledgeProposalItemKind,
    pub mode: KnowledgeProposalItemMode,
    pub target: String,
    pub draft: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeProposal {
    pub proposal_id: String,
    pub status: KnowledgeProposalStatus,
    pub confidence: f32,
    pub verify: KnowledgeProposalVerify,
    pub est_tokens: u32,
    #[serde(default)]
    pub items: Vec<KnowledgeProposalItem>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_refs: Option<Vec<AssetRefData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_proposal: Option<KnowledgeProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_parts: Option<Vec<AssistantRenderPart>>,
}
