use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub agent_id: Option<String>,
    pub session_type: String,
    pub parent_session_id: Option<String>,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_status: Option<SessionRuntimeStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionRuntimeStatus {
    Running,
    Queued,
    Starting,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub id: String,
    pub title: String,
    pub agent_id: Option<String>,
    pub session_type: String,
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_completed_run_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: Vec<ChatMessage>,
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
pub struct ImageData {
    pub data: String,
    pub mime_type: String,
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
    pub tool_calls: Option<Vec<ToolCallInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_proposal: Option<KnowledgeProposal>,
}
