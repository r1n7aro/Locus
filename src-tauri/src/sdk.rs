//! Local bridge used by the bundled Python `locus` package.
//!
//! The listener is always loopback-only and uses an ephemeral per-process
//! bearer token. The token and endpoint are injected only into Python
//! processes launched through Locus's selected runtime.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::net::TcpListener;

use crate::agent::definition::{canonical_agent_id, AgentDef};
use crate::session::models::{
    MessageRole, SessionEventRecord, SessionRunSummary, SessionRuntimeStatus, SessionSummary,
};
use crate::session::store::SessionStore;
use crate::tool::{
    ToolDef, ToolExecuteFn, ToolExecutionContext, ToolLoadMode, ToolRegistry, ToolResult,
    ToolRuntimeState,
};
use crate::{
    ActiveTasks, AgentDefRegistryState, ApiKeyState, AppAgentDir, ProviderKeysState, QuestionStore,
    RawContextStore, UndoManagerHandle,
};

const SDK_PATH: &str = "/sdk";
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_WAIT_MS: u64 = 30_000;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 120_000;
const MAX_TOOL_TIMEOUT_MS: u64 = 3_600_000;
const DEFAULT_UNITY_ENSURE_TIMEOUT_MS: u64 = 300_000;
const MAX_UNITY_ENSURE_TIMEOUT_MS: u64 = 1_800_000;
const UNITY_ENSURE_POLL_INTERVAL_MS: u64 = 500;

const CLAUDE_STANDARD_EFFORTS: &[&str] = &["none", "low", "medium", "high", "max"];
const CLAUDE_XHIGH_EFFORTS: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_STANDARD_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];
const FAST_SPEED_TIER: &[&str] = &["fast"];
const NO_SPEED_TIERS: &[&str] = &[];

#[derive(Clone, Copy)]
struct StaticModel {
    id: &'static str,
    name: &'static str,
    provider: &'static str,
    context_window: Option<u32>,
    default_effort: Option<&'static str>,
    supported_efforts: &'static [&'static str],
    additional_speed_tiers: &'static [&'static str],
    provider_default: bool,
}

// Keep this inventory aligned with src/stores/model.ts. The SDK filters these
// rows by the live provider login state before returning them by default.
const STATIC_MODELS: &[StaticModel] = &[
    StaticModel {
        id: "openrouter/claude-fable-5",
        name: "Claude Fable 5[1m]",
        provider: "openrouter",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "openrouter/claude-opus-4.8",
        name: "Claude Opus 4.8[1m]",
        provider: "openrouter",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: true,
    },
    StaticModel {
        id: "openrouter/claude-sonnet-5",
        name: "Claude Sonnet 5[1m]",
        provider: "openrouter",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "openrouter/claude-opus-4.6",
        name: "Claude Opus 4.6[1m]",
        provider: "openrouter",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_STANDARD_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "openrouter/glm-5",
        name: "GLM 5",
        provider: "openrouter",
        context_window: None,
        default_effort: None,
        supported_efforts: &[],
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "openrouter/minimax-m2.5",
        name: "MiniMax M2.5",
        provider: "openrouter",
        context_window: None,
        default_effort: None,
        supported_efforts: &[],
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "claude-fable-5",
        name: "Claude Fable 5[1m]",
        provider: "anthropic",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "claude-opus-4.8",
        name: "Claude Opus 4.8[1m]",
        provider: "anthropic",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: true,
    },
    StaticModel {
        id: "claude-sonnet-5",
        name: "Claude Sonnet 5[1m]",
        provider: "anthropic",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_XHIGH_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
    StaticModel {
        id: "claude-opus-4.6",
        name: "Claude Opus 4.6[1m]",
        provider: "anthropic",
        context_window: Some(1_000_000),
        default_effort: None,
        supported_efforts: CLAUDE_STANDARD_EFFORTS,
        additional_speed_tiers: NO_SPEED_TIERS,
        provider_default: false,
    },
];

const CODEX_FALLBACK_MODELS: &[StaticModel] = &[
    StaticModel {
        id: "openai/gpt-6-astra",
        name: "GPT-6 Astra",
        provider: "openai_codex",
        context_window: Some(258_400),
        default_effort: Some("low"),
        supported_efforts: CODEX_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: false,
    },
    StaticModel {
        id: "openai/gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        provider: "openai_codex",
        context_window: Some(353_400),
        default_effort: Some("low"),
        supported_efforts: CODEX_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: true,
    },
    StaticModel {
        id: "openai/gpt-5.6-terra",
        name: "GPT-5.6 Terra",
        provider: "openai_codex",
        context_window: Some(353_400),
        default_effort: Some("medium"),
        supported_efforts: CODEX_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: false,
    },
    StaticModel {
        id: "openai/gpt-5.6-luna",
        name: "GPT-5.6 Luna",
        provider: "openai_codex",
        context_window: Some(353_400),
        default_effort: Some("medium"),
        supported_efforts: CODEX_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: false,
    },
    StaticModel {
        id: "openai/gpt-5.5",
        name: "GPT-5.5",
        provider: "openai_codex",
        context_window: None,
        default_effort: Some("medium"),
        supported_efforts: CODEX_STANDARD_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: false,
    },
    StaticModel {
        id: "openai/gpt-5.4",
        name: "GPT-5.4",
        provider: "openai_codex",
        context_window: None,
        default_effort: Some("medium"),
        supported_efforts: CODEX_STANDARD_EFFORTS,
        additional_speed_tiers: FAST_SPEED_TIER,
        provider_default: false,
    },
];

#[derive(Default)]
pub struct SdkServerHandle {
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default = "empty_object")]
    params: Value,
}

fn empty_object() -> Value {
    json!({})
}

fn rpc_success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn invalid_params(message: impl Into<String>) -> String {
    message.into()
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|error| invalid_params(error.to_string()))
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn token_matches(expected: &str, provided: &str) -> bool {
    let left = expected.as_bytes();
    let right = provided.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn host_allowed(host: Option<&str>) -> bool {
    let Some(host) = host else { return false };
    let bare = host.rsplit_once(':').map(|(host, _)| host).unwrap_or(host);
    matches!(bare, "127.0.0.1" | "localhost")
}

fn plain_response(status: StatusCode, body: impl Into<String>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body.into())))
        .expect("static SDK response builds")
}

fn json_response(value: &Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("SDK JSON response builds")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SdkPythonToolSpec {
    name: String,
    callback_key: String,
    description: String,
    input_schema: Value,
    #[serde(default)]
    mutates_workspace: bool,
    #[serde(default = "default_python_tool_timeout_ms")]
    timeout_ms: u64,
}

fn default_python_tool_timeout_ms() -> u64 {
    120_000
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SdkAgentSpec {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    system_prompt: String,
    #[serde(default)]
    locus_tools: Vec<String>,
    #[serde(default)]
    python_tools: Vec<SdkPythonToolSpec>,
    #[serde(default)]
    callback_url: Option<String>,
    #[serde(default)]
    callback_token: Option<String>,
    #[serde(default)]
    sub_agents: Vec<String>,
    #[serde(default)]
    default_effort: Option<String>,
    #[serde(default)]
    model_recommendation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptAgentParams {
    agent_id: String,
    #[serde(default)]
    agent_spec: Option<SdkAgentSpec>,
    prompt: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    workspace_ref: Option<crate::workspace_service::WorkspaceRef>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    fast_mode: Option<bool>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    session_type: Option<String>,
    #[serde(default)]
    knowledge_mode: Option<String>,
    #[serde(default)]
    subagent_models: Option<HashMap<String, String>>,
    #[serde(default)]
    subagent_efforts: Option<HashMap<String, String>>,
    #[serde(default)]
    subagent_fast_modes: Option<HashMap<String, bool>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunParams {
    run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitRunParams {
    run_id: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunEventsParams {
    run_id: String,
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerParams {
    question_id: String,
    answer: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListModelsParams {
    #[serde(default = "default_true")]
    available_only: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListToolsParams {
    #[serde(default)]
    workspace_ref: Option<crate::workspace_service::WorkspaceRef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSessionsParams {
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    running_only: bool,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    workspace_ref: Option<crate::workspace_service::WorkspaceRef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceScopeParams {
    workspace_ref: crate::workspace_service::WorkspaceRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityDialogParams {
    project: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChooseUnityDialogParams {
    project: String,
    dialog_id: String,
    choice_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitUnityExecutionParams {
    project: String,
    execution_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityEditorStatusParams {
    project: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureUnityEditorParams {
    project: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    wait_until: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestartUnityEditorParams {
    project: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    wait_until: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnityEnsureTarget {
    Process,
    Connected,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnityLaunchWaitState {
    Satisfied,
    Waiting,
    Exited,
}

fn unity_launch_wait_state(
    target_satisfied: bool,
    status_process_id: Option<u32>,
    launch_process_id: u32,
    launch_liveness: Option<crate::unity_bridge::UnityProcessIdentityLiveness>,
) -> UnityLaunchWaitState {
    if matches!(
        launch_liveness,
        Some(
            crate::unity_bridge::UnityProcessIdentityLiveness::Exited
                | crate::unity_bridge::UnityProcessIdentityLiveness::Replaced
        )
    ) {
        return UnityLaunchWaitState::Exited;
    }
    if target_satisfied
        && status_process_id == Some(launch_process_id)
        && launch_liveness == Some(crate::unity_bridge::UnityProcessIdentityLiveness::Alive)
    {
        return UnityLaunchWaitState::Satisfied;
    }
    UnityLaunchWaitState::Waiting
}

fn sdk_semantic_status_matches_launch(
    status: &SdkUnityEditorStatus,
    launch_process_id: u32,
) -> bool {
    status.process_id == Some(launch_process_id)
        && status.semantic.process.pid == Some(launch_process_id)
}

impl UnityEnsureTarget {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("ready") => Ok(Self::Ready),
            Some("process") => Ok(Self::Process),
            Some("connected") => Ok(Self::Connected),
            Some(value) => Err(format!(
                "waitUntil must be one of 'process', 'connected', or 'ready'; got '{value}'"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Connected => "connected",
            Self::Ready => "ready",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkUnityEditorStatus {
    project_path: String,
    checkout_id: String,
    workspace_generation: u64,
    connected: bool,
    ready: bool,
    process_state: crate::unity_bridge::UnityEditorProcessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    editor_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_mode: Option<crate::unity_bridge::UnityLaunchMode>,
    headless: bool,
    safe_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    editor_log_path: Option<String>,
    semantic_phase: String,
    main_thread_blocked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocking_reason: Option<String>,
    main_thread: crate::unity_bridge::ObservedMainThreadState,
    safety: crate::unity_bridge::ObservedSafetyState,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocking_dialog: Option<crate::unity_bridge::dialog::UnityModalDialog>,
    blocking_dialog_recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_status: Option<crate::workspace_service::service::ServiceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    readiness: Option<crate::workspace_service::service::ServiceReadinessSnapshot>,
    connection: crate::unity_bridge::UnityConnectionStatus,
    semantic: crate::unity_bridge::SemanticState,
}

impl SdkUnityEditorStatus {
    fn satisfies(&self, target: UnityEnsureTarget) -> bool {
        match target {
            UnityEnsureTarget::Process => matches!(
                self.process_state,
                crate::unity_bridge::UnityEditorProcessState::Running
            ),
            UnityEnsureTarget::Connected => self.connected,
            UnityEnsureTarget::Ready => self.ready,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkEnsureUnityEditorResult {
    launched: bool,
    wait_until: String,
    waited_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch: Option<crate::unity_bridge::UnityLaunchResult>,
    status: SdkUnityEditorStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkRestartUnityEditorResult {
    closed_process_ids: Vec<u32>,
    forced_process_ids: Vec<u32>,
    wait_until: String,
    waited_ms: u64,
    launch: crate::unity_bridge::UnityLaunchResult,
    status: SdkUnityEditorStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionParams {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendSessionMessageParams {
    session_id: String,
    source_session_id: String,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SdkSessionMessageDelivery {
    pending_input_id: String,
    source_session_id: String,
    source_session_title: String,
    target_session_id: String,
    target_session_title: String,
    target_run_id: String,
    delivery: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionEventsParams {
    session_id: String,
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallToolParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    workspace_ref: Option<crate::workspace_service::WorkspaceRef>,
}

fn validate_agent_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty() || id.len() > 64 {
        return Err("Agent id must contain 1-64 characters".to_string());
    }
    if !id.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
    }) {
        return Err("Agent id may contain lowercase letters, digits, '-' and '_'".to_string());
    }
    if crate::agent::definition::is_hidden_legacy_agent_id(id) {
        return Err(format!("Agent id '{id}' is reserved"));
    }
    Ok(id.to_string())
}

async fn list_agents(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<AgentDefRegistryState>();
    let registry = state.0.read().await;
    let default_id = registry.default_id().to_string();
    let mut agents = registry
        .list_all()
        .into_iter()
        .map(|def| {
            json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "tools": def.tools,
                "subAgents": def.sub_agents,
                "isDefault": def.id == default_id,
                "defaultEffort": def.default_effort,
                "modelRecommendation": def.model_recommendation,
                "source": def.source,
            })
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });
    Ok(Value::Array(agents))
}

fn static_model_value(
    model: StaticModel,
    available: bool,
    unavailable_reason: Option<&str>,
    default_model: Option<&str>,
) -> Value {
    json!({
        "id": model.id,
        "name": model.name,
        "provider": model.provider,
        "contextWindow": model.context_window,
        "defaultEffort": model.default_effort,
        "supportedEfforts": model.supported_efforts,
        "additionalSpeedTiers": model.additional_speed_tiers,
        "available": available,
        "unavailableReason": unavailable_reason,
        "isDefault": default_model
            .map(|value| value == model.id)
            .unwrap_or(model.provider_default),
    })
}

async fn codex_model_values(
    app: &AppHandle,
    available: bool,
    default_model: Option<&str>,
) -> Vec<Value> {
    let mut remote = Vec::new();
    if available {
        let credentials = {
            let codex = app.state::<crate::commands::CodexAuthStateHandle>();
            let mut guard = codex.lock().await;
            match guard.access_token().await {
                Ok(access_token) => Some((access_token, guard.account_id())),
                Err(_) => None,
            }
        };
        if let Some((access_token, account_id)) = credentials {
            if let Ok(cache_dir) = crate::commands::persistent_config_dir() {
                let config = app.state::<Arc<crate::config::AppConfig>>();
                if let Ok(models) = crate::llm::codex_models::list_codex_available_models(
                    &access_token,
                    account_id.as_deref(),
                    config.base_url.as_deref(),
                    &cache_dir,
                )
                .await
                {
                    remote = models
                        .into_iter()
                        .map(|model| {
                            let is_default = default_model
                                .map(|value| value == model.id)
                                .unwrap_or(model.is_default);
                            json!({
                                "id": model.id,
                                "name": model.name,
                                "provider": model.provider,
                                "contextWindow": model.effective_context_window,
                                "defaultEffort": model.default_effort,
                                "supportedEfforts": model.supported_efforts,
                                "additionalSpeedTiers": model.additional_speed_tiers,
                                "available": true,
                                "unavailableReason": Value::Null,
                                "isDefault": is_default,
                            })
                        })
                        .collect();
                }
            }
        }
    }
    if remote.is_empty() {
        let reason = (!available).then_some("Codex login is not configured");
        return CODEX_FALLBACK_MODELS
            .iter()
            .copied()
            .map(|model| static_model_value(model, available, reason, default_model))
            .collect();
    }
    remote
}

async fn list_models(app: &AppHandle, params: ListModelsParams) -> Result<Value, String> {
    let defaults = crate::commands::get_model_defaults(app.clone())
        .await
        .unwrap_or_default();
    let last_model = crate::commands::get_last_model(app.clone())
        .await
        .ok()
        .and_then(|model| nonempty(Some(model)));
    let default_model = nonempty(Some(defaults.main_model)).or(last_model);

    let openrouter_available = !app.state::<ApiKeyState>().read().await.trim().is_empty();
    let anthropic_available = app
        .state::<Arc<tokio::sync::Mutex<crate::auth::AuthState>>>()
        .lock()
        .await
        .is_authenticated();
    let codex_available = app
        .state::<crate::commands::CodexAuthStateHandle>()
        .lock()
        .await
        .is_authenticated();

    let mut models = STATIC_MODELS
        .iter()
        .copied()
        .map(|model| {
            let (available, reason) = match model.provider {
                "openrouter" => (
                    openrouter_available,
                    (!openrouter_available).then_some("OpenRouter API key is not configured"),
                ),
                "anthropic" => (
                    anthropic_available,
                    (!anthropic_available).then_some("Anthropic login is not configured"),
                ),
                _ => (false, Some("Model provider is unavailable")),
            };
            static_model_value(model, available, reason, default_model.as_deref())
        })
        .collect::<Vec<_>>();
    models.extend(
        codex_model_values(app, codex_available, default_model.as_deref())
            .await
            .into_iter(),
    );

    for provider in crate::commands::load_custom_providers()? {
        let multiple_models = provider.models.len() > 1;
        for model in provider.models {
            let id = format!("custom/{}/{}", provider.id, model.id);
            let is_default = default_model.as_deref() == Some(id.as_str());
            let name = if multiple_models {
                format!("{} / {}", provider.name, model.name)
            } else {
                provider.name.clone()
            };
            let supported_efforts = if matches!(
                model.reasoning_param_format.as_ref(),
                Some(crate::commands::CustomReasoningParamFormat::None)
            ) {
                Vec::new()
            } else {
                model.supported_reasoning_efforts
            };
            models.push(json!({
                "id": id,
                "name": name,
                "provider": "custom",
                "contextWindow": model.context_length,
                "defaultEffort": Value::Null,
                "supportedEfforts": supported_efforts,
                "additionalSpeedTiers": [],
                "available": true,
                "unavailableReason": Value::Null,
                "isDefault": is_default,
                "customProviderId": provider.id,
                "customProviderName": provider.name,
                "customModelName": model.name,
            }));
        }
    }

    if params.available_only {
        models.retain(|model| model["available"].as_bool() == Some(true));
    }
    Ok(Value::Array(models))
}

fn is_agent_only_tool(name: &str) -> bool {
    matches!(
        name,
        "subagent"
            | "ask_user_question"
            | "todowrite"
            | "exit_plan_mode"
            | "tool_load"
            | "tool_call"
    )
}

async fn list_tools(app: &AppHandle, params: ListToolsParams) -> Result<Value, String> {
    crate::mcp::manager::ensure_fresh().await;
    let registry = match params.workspace_ref.as_ref() {
        Some(workspace_ref) => {
            let scope = resolve_sdk_workspace_scope(app, workspace_ref, "tools.list")?;
            let definitions = app
                .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
                .snapshot(scope.runtime().as_ref())
                .await
                .map_err(|error| format!("Failed to load checkout Agent definitions: {error}"))?;
            app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>()
                .snapshot(scope.runtime().as_ref(), definitions.as_ref())
                .await?
        }
        None => app.state::<Arc<ToolRegistry>>().inner().clone(),
    };
    let mut names = registry.tool_names();
    names.extend(crate::mcp::manager::wire_tool_names());
    names.sort();
    names.dedup();

    let tools = names
        .into_iter()
        .filter_map(|name| {
            let detail = registry
                .tool_description(&name)
                .or_else(|| crate::mcp::manager::resolve_tool_description(&name));
            let (description, input_schema) = detail?;
            let source = if name.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX) {
                "mcp"
            } else if registry.is_built_in(&name) {
                "builtin"
            } else {
                "skill"
            };
            Some(json!({
                "name": name,
                "description": description,
                "inputSchema": input_schema,
                "source": source,
                "mutatesWorkspace": registry.mutates_workspace(&name),
                "agentOnly": is_agent_only_tool(&name),
            }))
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(tools))
}

async fn canonical_tool_names(
    app: &AppHandle,
    requested: Vec<String>,
    workspace_ref: Option<&crate::workspace_service::WorkspaceRef>,
) -> Result<Vec<String>, String> {
    crate::mcp::manager::ensure_fresh().await;
    let workspace_scope = workspace_ref
        .map(|workspace_ref| {
            app.state::<Arc<crate::workspace_service::ProjectRegistry>>()
                .resolve_workspace_ref(workspace_ref)
                .map_err(|error| format!("Failed to resolve SDK workspace: {error}"))
        })
        .transpose()?;
    let registry = match workspace_scope.as_ref() {
        Some(scope) => {
            let definitions = app
                .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
                .snapshot(scope.runtime().as_ref())
                .await
                .map_err(|error| format!("Failed to load checkout Agent definitions: {error}"))?;
            app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>()
                .snapshot(scope.runtime().as_ref(), definitions.as_ref())
                .await?
        }
        None => app.state::<Arc<ToolRegistry>>().inner().clone(),
    };
    let working_dir = workspace_scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .unwrap_or_default();
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for requested_name in requested {
        let trimmed = requested_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canonical = registry
            .canonical_name(trimmed)
            .or_else(|| {
                crate::commands::canonical_skill_package_tool_name_for_working_dir(
                    &working_dir,
                    trimmed,
                )
            })
            .or_else(|| crate::mcp::manager::resolve_wire_tool(trimmed).map(|tool| tool.wire_name))
            .ok_or_else(|| format!("Unknown Locus tool '{trimmed}'"))?;
        if seen.insert(canonical.clone()) {
            names.push(canonical);
        }
    }
    Ok(names)
}

fn validate_python_tool_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("Python tool name must contain 1-64 characters".to_string());
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!(
            "Python tool name '{name}' may contain letters, digits, '-' and '_'"
        ));
    }
    Ok(name.to_string())
}

fn validate_callback_url(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value.trim())
        .map_err(|error| format!("Invalid Python tool callback URL: {error}"))?;
    if parsed.scheme() != "http" {
        return Err("Python tool callback URL must use http".to_string());
    }
    if !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err("Python tool callback URL must target loopback".to_string());
    }
    if parsed.port().is_none() {
        return Err("Python tool callback URL must include an explicit port".to_string());
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Python tool callback URL contains unsupported components".to_string());
    }
    Ok(parsed.to_string())
}

impl SdkAgentSpec {
    pub(crate) fn agent_def(&self) -> AgentDef {
        let mut tools = self.locus_tools.clone();
        tools.extend(self.python_tools.iter().map(|tool| tool.name.clone()));
        AgentDef {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            project_types: Vec::new(),
            system_prompt: self.system_prompt.clone(),
            env_template: String::new(),
            tools,
            sub_agents: self.sub_agents.clone(),
            default: false,
            default_effort: self.default_effort.clone(),
            model_recommendation: self.model_recommendation.clone(),
            tool_description_overrides: HashMap::new(),
            source: "python".to_string(),
        }
    }
}

async fn prepare_agent_spec(
    app: &AppHandle,
    mut spec: SdkAgentSpec,
    workspace_ref: Option<&crate::workspace_service::WorkspaceRef>,
) -> Result<SdkAgentSpec, String> {
    spec.id = validate_agent_id(&spec.id)?;
    spec.name = spec.name.trim().to_string();
    if spec.name.is_empty() {
        spec.name = spec.id.clone();
    }
    spec.description = spec.description.trim().to_string();
    spec.system_prompt = spec.system_prompt.trim().to_string();
    if spec.system_prompt.is_empty() {
        return Err("agentSpec.systemPrompt cannot be empty".to_string());
    }
    spec.locus_tools = canonical_tool_names(app, spec.locus_tools, workspace_ref).await?;

    let workspace_scope = workspace_ref
        .map(|workspace_ref| resolve_sdk_workspace_scope(app, workspace_ref, "agentSpec"))
        .transpose()?;
    let registry = match workspace_scope.as_ref() {
        Some(scope) => {
            let definitions = app
                .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
                .snapshot(scope.runtime().as_ref())
                .await
                .map_err(|error| format!("Failed to load checkout Agent definitions: {error}"))?;
            app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>()
                .snapshot(scope.runtime().as_ref(), definitions.as_ref())
                .await?
        }
        None => app.state::<Arc<ToolRegistry>>().inner().clone(),
    };
    let mut seen = spec.locus_tools.iter().cloned().collect::<HashSet<_>>();
    for tool in &mut spec.python_tools {
        tool.name = validate_python_tool_name(&tool.name)?;
        tool.callback_key = tool.callback_key.trim().to_string();
        if tool.callback_key.is_empty() || tool.callback_key.len() > 128 {
            return Err(format!(
                "Python tool '{}' has an invalid callback key",
                tool.name
            ));
        }
        tool.description = tool.description.trim().to_string();
        if !tool.input_schema.is_object()
            || tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
        {
            return Err(format!(
                "Python tool '{}' inputSchema must be an object schema",
                tool.name
            ));
        }
        if registry.canonical_name(&tool.name).is_some()
            || crate::mcp::manager::resolve_wire_tool(&tool.name).is_some()
        {
            return Err(format!(
                "Python tool '{}' conflicts with an existing Locus tool",
                tool.name
            ));
        }
        if !seen.insert(tool.name.clone()) {
            return Err(format!("Duplicate agent tool '{}'", tool.name));
        }
        tool.timeout_ms = tool.timeout_ms.clamp(1_000, 3_600_000);
    }

    if spec.python_tools.is_empty() {
        spec.callback_url = None;
        spec.callback_token = None;
    } else {
        spec.callback_url = Some(validate_callback_url(
            spec.callback_url.as_deref().unwrap_or_default(),
        )?);
        spec.callback_token = Some(
            spec.callback_token
                .as_deref()
                .map(str::trim)
                .filter(|value| value.len() >= 32)
                .ok_or_else(|| "Python tool callback token is missing or too short".to_string())?
                .to_string(),
        );
    }
    Ok(spec)
}

pub(crate) fn tool_registry_for_agent(
    base: &ToolRegistry,
    spec: &SdkAgentSpec,
) -> Result<Arc<ToolRegistry>, String> {
    if spec.python_tools.is_empty() {
        return Ok(Arc::new(base.clone()));
    }
    let callback_url = spec
        .callback_url
        .as_ref()
        .ok_or_else(|| "Python tool callback URL is missing".to_string())?
        .clone();
    let callback_token = spec
        .callback_token
        .as_ref()
        .ok_or_else(|| "Python tool callback token is missing".to_string())?
        .clone();
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|error| format!("Failed to create Python tool callback client: {error}"))?;
    let mut registry = base.clone();

    for tool in &spec.python_tools {
        let name = tool.name.clone();
        let callback_key = tool.callback_key.clone();
        let callback_url = callback_url.clone();
        let callback_token = callback_token.clone();
        let http = http.clone();
        let timeout = Duration::from_millis(tool.timeout_ms);
        let execute: ToolExecuteFn = Arc::new(move |arguments, context| {
            let name = name.clone();
            let callback_key = callback_key.clone();
            let callback_url = callback_url.clone();
            let callback_token = callback_token.clone();
            let http = http.clone();
            Box::pin(async move {
                context.report_progress(format!("Running Python tool {name}"));
                let request = http
                    .post(callback_url)
                    .bearer_auth(callback_token)
                    .timeout(timeout)
                    .json(&json!({
                        "toolKey": callback_key,
                        "arguments": arguments,
                    }));
                let response = match request.send().await {
                    Ok(response) => response,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Python tool '{name}' callback failed: {error}"),
                            is_error: true,
                        }
                    }
                };
                let status = response.status();
                let bytes = match response.bytes().await {
                    Ok(bytes) if bytes.len() <= MAX_BODY_BYTES => bytes,
                    Ok(_) => {
                        return ToolResult {
                            output: format!("Python tool '{name}' returned too much data"),
                            is_error: true,
                        }
                    }
                    Err(error) => {
                        return ToolResult {
                            output: format!("Failed to read Python tool '{name}' result: {error}"),
                            is_error: true,
                        }
                    }
                };
                let payload = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(payload) => payload,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Python tool '{name}' returned invalid JSON: {error}"),
                            is_error: true,
                        }
                    }
                };
                if !status.is_success() || payload.get("ok").and_then(Value::as_bool) != Some(true)
                {
                    return ToolResult {
                        output: payload
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Python tool callback failed")
                            .to_string(),
                        is_error: true,
                    };
                }
                let result = payload.get("result").cloned().unwrap_or(Value::Null);
                ToolResult {
                    output: result.as_str().map(str::to_string).unwrap_or_else(|| {
                        serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                    }),
                    is_error: false,
                }
            })
        });
        registry.register_runtime(
            ToolDef {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
                mutates_workspace: tool.mutates_workspace,
                execute,
            },
            ToolLoadMode::Direct,
        );
    }
    Ok(Arc::new(registry))
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn resolve_prompt_model(
    app: &AppHandle,
    store: &SessionStore,
    params: &PromptAgentParams,
) -> Result<String, String> {
    if let Some(model) = nonempty(params.model.clone()) {
        return Ok(model);
    }
    if let Some(session_id) = params.session_id.as_deref() {
        if let Ok(detail) = store.load_session(session_id) {
            if let Some(model) = nonempty(detail.last_model_id) {
                return Ok(model);
            }
        }
    }
    if let Ok(model) = crate::commands::get_last_model(app.clone()).await {
        if let Some(model) = nonempty(Some(model)) {
            return Ok(model);
        }
    }
    if let Ok(defaults) = crate::commands::get_model_defaults(app.clone()).await {
        if let Some(model) = nonempty(Some(defaults.main_model)) {
            return Ok(model);
        }
    }
    let config = app.state::<Arc<crate::config::AppConfig>>();
    nonempty(Some(config.model.clone()))
        .ok_or_else(|| "No model is configured in Locus".to_string())
}

async fn resolve_prompt_effort(
    app: &AppHandle,
    store: &SessionStore,
    params: &PromptAgentParams,
    agent_spec: Option<&SdkAgentSpec>,
) -> Option<String> {
    if let Some(effort) = nonempty(params.effort.clone()) {
        return Some(effort);
    }
    if let Some(session_id) = params.session_id.as_deref() {
        if let Ok(detail) = store.load_session(session_id) {
            if let Some(effort) = nonempty(detail.last_effort) {
                return Some(effort);
            }
        }
    }
    if let Some(effort) = agent_spec.and_then(|spec| nonempty(spec.default_effort.clone())) {
        return Some(effort);
    }
    crate::commands::get_last_effort(app.clone())
        .await
        .ok()
        .and_then(|value| nonempty(Some(value)))
}

async fn resolve_prompt_fast_mode(
    app: &AppHandle,
    store: &SessionStore,
    params: &PromptAgentParams,
) -> Option<bool> {
    if let Some(fast_mode) = params.fast_mode {
        return Some(fast_mode);
    }
    if let Some(session_id) = params.session_id.as_deref() {
        if let Ok(detail) = store.load_session(session_id) {
            if detail.last_fast_mode.is_some() {
                return detail.last_fast_mode;
            }
        }
    }
    crate::commands::get_codex_fast_mode(app.clone()).await.ok()
}

async fn prompt_agent(app: &AppHandle, mut params: PromptAgentParams) -> Result<Value, String> {
    let prompt = params.prompt.trim();
    if prompt.is_empty() {
        return Err("prompt cannot be empty".to_string());
    }
    let agent_spec = match params.agent_spec.take() {
        Some(spec) => {
            let spec = prepare_agent_spec(app, spec, params.workspace_ref.as_ref()).await?;
            if spec.id != params.agent_id.trim() {
                return Err("agentId must match agentSpec.id".to_string());
            }
            Some(spec)
        }
        None => None,
    };
    let agent_id = if let Some(spec) = agent_spec.as_ref() {
        spec.id.clone()
    } else {
        canonical_agent_id(params.agent_id.trim()).to_string()
    };

    let store = app.state::<Arc<SessionStore>>();
    let model = resolve_prompt_model(app, store.inner().as_ref(), &params).await?;
    let effort =
        resolve_prompt_effort(app, store.inner().as_ref(), &params, agent_spec.as_ref()).await;
    let fast_mode = resolve_prompt_fast_mode(app, store.inner().as_ref(), &params).await;
    let model_defaults = crate::commands::get_model_defaults(app.clone())
        .await
        .unwrap_or_default();
    let subagent_models = params
        .subagent_models
        .unwrap_or(model_defaults.subagent_models);
    let subagent_efforts = params
        .subagent_efforts
        .unwrap_or(model_defaults.subagent_efforts);
    let subagent_fast_modes = params
        .subagent_fast_modes
        .unwrap_or(model_defaults.subagent_fast_modes);

    let launch = crate::commands::chat(
        nonempty(params.session_id),
        params.workspace_ref,
        prompt.to_string(),
        None,
        nonempty(params.title),
        Some(agent_id),
        agent_spec,
        Some(model),
        effort,
        fast_mode,
        None,
        None,
        None,
        Some(params.session_type.unwrap_or_else(|| "chat".to_string())),
        Some(params.mode.unwrap_or_else(|| "build".to_string())),
        None,
        Some(subagent_models),
        Some(subagent_efforts),
        Some(subagent_fast_modes),
        Some(params.knowledge_mode.unwrap_or_else(|| "full".to_string())),
        None,
        None,
        app.clone(),
        app.state::<Arc<SessionStore>>(),
        app.state::<AgentDefRegistryState>(),
        app.state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>(),
        app.state::<Arc<crate::config::AppConfig>>(),
        app.state::<Arc<ToolRegistry>>(),
        app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>(),
        app.state::<Arc<tokio::sync::Mutex<crate::auth::AuthState>>>(),
        app.state::<ApiKeyState>(),
        app.state::<ProviderKeysState>(),
        app.state::<crate::commands::CodexAuthStateHandle>(),
        app.state::<Arc<crate::workspace_service::ProjectRegistry>>(),
        app.state::<RawContextStore>(),
        app.state::<ActiveTasks>(),
        app.state::<crate::commands::AppKnowledgeDir>(),
        app.state::<AppAgentDir>(),
        app.state::<UndoManagerHandle>(),
    )
    .await
    .map_err(|error| error.to_string())?;

    serde_json::to_value(launch).map_err(|error| error.to_string())
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "done" | "cancelled" | "error")
}

fn result_from_events(events: &[SessionEventRecord]) -> (Option<String>, Option<String>) {
    for event in events.iter().rev() {
        match event.event_type.as_str() {
            "done" => {
                return (
                    event
                        .payload
                        .get("fullText")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    event
                        .payload
                        .get("messageId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            }
            "cancelled" => {
                return (
                    event
                        .payload
                        .get("fullText")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    event
                        .payload
                        .get("messageId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            }
            _ => {}
        }
    }
    (None, None)
}

fn fallback_assistant_message(
    store: &SessionStore,
    run: &SessionRunSummary,
) -> (Option<String>, Option<String>) {
    let Ok(detail) = store.load_session(&run.session_id) else {
        return (None, None);
    };
    detail
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| (Some(message.content.clone()), Some(message.id.clone())))
        .unwrap_or((None, None))
}

fn run_snapshot(app: &AppHandle, run_id: &str) -> Result<Value, String> {
    let store = app.state::<Arc<SessionStore>>();
    let run = store
        .run_by_id(run_id)?
        .ok_or_else(|| format!("Run '{run_id}' not found"))?;
    let completed = is_terminal_status(&run.status);
    let events = store.list_run_events(run_id, None, Some(2_000))?;
    let (mut text, mut message_id) = result_from_events(&events);
    if completed && text.is_none() {
        (text, message_id) = fallback_assistant_message(store.inner().as_ref(), &run);
    }
    let runtime = store
        .runtime_snapshot_for_session(&run.session_id)
        .filter(|snapshot| snapshot.active_run.run_id == run.run_id);
    Ok(json!({
        "runId": run.run_id,
        "sessionId": run.session_id,
        "status": run.status,
        "completed": completed,
        "text": text,
        "messageId": message_id,
        "error": run.error_message,
        "runtime": runtime,
    }))
}

async fn wait_run(app: &AppHandle, params: WaitRunParams) -> Result<Value, String> {
    let timeout_ms = params.timeout_ms.unwrap_or(MAX_WAIT_MS).min(MAX_WAIT_MS);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let snapshot = run_snapshot(app, &params.run_id)?;
        if snapshot["completed"].as_bool().unwrap_or(false) || timeout_ms == 0 {
            return Ok(snapshot);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(snapshot);
        }
        tokio::time::sleep(Duration::from_millis(100).min(deadline - now)).await;
    }
}

async fn cancel_run(app: &AppHandle, params: RunParams) -> Result<Value, String> {
    let store = app.state::<Arc<SessionStore>>();
    let run = store
        .run_by_id(&params.run_id)?
        .ok_or_else(|| format!("Run '{}' not found", params.run_id))?;
    crate::commands::cancel_chat(
        run.session_id,
        app.clone(),
        app.state::<Arc<SessionStore>>(),
        app.state::<ActiveTasks>(),
    )
    .await
    .map_err(|error| error.to_string())?;
    run_snapshot(app, &params.run_id)
}

async fn answer_run(app: &AppHandle, params: AnswerParams) -> Result<Value, String> {
    crate::commands::answer_question(
        params.question_id,
        params.answer,
        app.clone(),
        app.state::<QuestionStore>(),
        app.state::<Arc<SessionStore>>(),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(json!({ "answered": true }))
}

fn resolve_sdk_workspace_scope(
    app: &AppHandle,
    workspace_ref: &crate::workspace_service::WorkspaceRef,
    operation: &str,
) -> Result<crate::workspace_service::ResolvedWorkspaceScope, String> {
    app.state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .resolve_workspace_ref(workspace_ref)
        .map_err(|error| format!("{operation} workspace resolution failed: {error}"))
}

async fn workspace_snapshot(
    app: &AppHandle,
    params: WorkspaceScopeParams,
) -> Result<Value, String> {
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "workspace.get")?;
    let runtime = scope.runtime();
    let services = runtime.services().state_snapshots().await;
    Ok(json!({
        "path": runtime.root().to_string_lossy(),
        "projectId": runtime.project_id(),
        "checkoutId": runtime.checkout_id(),
        "workspaceGeneration": runtime.generation(),
        "services": services,
    }))
}

fn resolve_sdk_unity_runtime(
    app: &AppHandle,
    project: &str,
    operation: &str,
) -> Result<Arc<crate::workspace_service::WorkspaceRuntime>, String> {
    let project = project.trim();
    if project.is_empty() {
        return Err(format!("{operation} requires a non-empty project path"));
    }
    let registry = app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
    let runtime = registry
        .runtime_for_root(std::path::Path::new(project))
        .ok_or_else(|| {
            format!("{operation} project is not an active Locus workspace: {project}")
        })?;
    let resolved_project = runtime.root().to_string_lossy().to_string();
    if !crate::unity_bridge::is_unity_project(&resolved_project) {
        return Err(format!(
            "{operation} requires an active Unity project: {resolved_project}"
        ));
    }
    Ok(runtime)
}

fn resolve_sdk_unity_project(
    app: &AppHandle,
    project: &str,
    operation: &str,
) -> Result<String, String> {
    resolve_sdk_unity_runtime(app, project, operation)
        .map(|runtime| runtime.root().to_string_lossy().to_string())
}

fn unity_ensure_locks() -> &'static tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>
{
    static LOCKS: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    LOCKS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

async fn unity_ensure_lock(checkout_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = unity_ensure_locks().lock().await;
    Arc::clone(
        locks
            .entry(checkout_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

async fn sdk_unity_editor_status(
    runtime: &Arc<crate::workspace_service::WorkspaceRuntime>,
) -> SdkUnityEditorStatus {
    use crate::workspace_service::{ServiceKind, ServiceReadinessPhase};

    let project_path = runtime.root().to_string_lossy().to_string();
    let service = runtime.services().state_snapshot(ServiceKind::Unity).await;
    let readiness = service
        .as_ref()
        .and_then(|snapshot| snapshot.readiness.clone());
    let service_status = service.as_ref().map(|snapshot| snapshot.status);
    // Keep the two pipe probes sequential. Running them concurrently makes
    // one observer see the other's short-lived writer lock as channel busy.
    let connection = crate::unity_bridge::query_unity_connection_status(&project_path).await;
    let semantic = crate::unity_bridge::unity_semantic_state(&project_path).await;
    let blocking_dialog = crate::unity_bridge::dialog::current_dialog(&project_path);

    let process_state = match &connection.editor_process_state {
        crate::unity_bridge::UnityEditorProcessState::Unknown => {
            match semantic.process.state.as_str() {
                "running" => crate::unity_bridge::UnityEditorProcessState::Running,
                "not_running" => crate::unity_bridge::UnityEditorProcessState::NotRunning,
                _ => crate::unity_bridge::UnityEditorProcessState::Unknown,
            }
        }
        state => state.clone(),
    };
    let process_running = matches!(
        process_state,
        crate::unity_bridge::UnityEditorProcessState::Running
    );
    let channel_connected =
        connection.connected || matches!(semantic.channel.control_pipe.as_str(), "ready" | "busy");
    let service_connected = readiness.as_ref().is_some_and(|snapshot| {
        matches!(
            snapshot.phase,
            ServiceReadinessPhase::Connected
                | ServiceReadinessPhase::Ready
                | ServiceReadinessPhase::Reloading
        )
    });
    let connected = channel_connected || (process_running && service_connected);
    let main_thread_blocked = blocking_dialog.is_some()
        || matches!(
            semantic.main_thread.state.as_str(),
            "blocked" | "hung" | "stalled"
        );
    let safe_mode = semantic.phase == "safe_mode" || semantic.editor_log.safe_mode;
    let blocking_reason = if safe_mode {
        Some("safe_mode".to_string())
    } else if blocking_dialog.is_some() {
        Some("modal_dialog".to_string())
    } else if matches!(semantic.main_thread.state.as_str(), "hung" | "stalled") {
        Some(semantic.main_thread.state.clone())
    } else {
        None
    };
    let blocking_dialog_recoverable = blocking_dialog
        .as_ref()
        .is_some_and(|dialog| !dialog.choices.is_empty());
    let ready = process_running
        && channel_connected
        && semantic.safety.can_call_unity_api
        && !main_thread_blocked;

    SdkUnityEditorStatus {
        project_path,
        checkout_id: runtime.checkout_id().to_string(),
        workspace_generation: runtime.generation(),
        connected,
        ready,
        process_state,
        process_id: connection.editor_process_id.or(semantic.process.pid),
        editor_path: connection
            .editor_process_path
            .clone()
            .or_else(|| semantic.process.path.clone()),
        launch_mode: connection.launch_mode,
        headless: connection.headless,
        safe_mode,
        editor_log_path: semantic.editor_log.path.clone(),
        semantic_phase: semantic.phase.clone(),
        main_thread_blocked,
        blocking_reason,
        main_thread: semantic.main_thread.clone(),
        safety: semantic.safety.clone(),
        blocking_dialog,
        blocking_dialog_recoverable,
        service_status,
        readiness,
        connection,
        semantic,
    }
}

fn unity_safe_mode_wait_error(status: &SdkUnityEditorStatus, target: UnityEnsureTarget) -> String {
    let log = status.editor_log_path.as_deref().unwrap_or("unavailable");
    format!(
        "Unity Editor entered Safe Mode before reaching '{}': project={}, editorLog={}. Read the Editor log or call unity_get_console_log with level='error', fix the compiler errors with file tools, then wait for Unity to exit Safe Mode automatically.",
        target.as_str(), status.project_path, log
    )
}

async fn get_unity_editor_status(
    app: &AppHandle,
    params: UnityEditorStatusParams,
) -> Result<Value, String> {
    let runtime = resolve_sdk_unity_runtime(app, &params.project, "unity.editor.status")?;
    serde_json::to_value(sdk_unity_editor_status(&runtime).await).map_err(|error| error.to_string())
}

async fn ensure_unity_editor(
    app: &AppHandle,
    params: EnsureUnityEditorParams,
) -> Result<Value, String> {
    use crate::workspace_service::ServiceKind;

    let target = UnityEnsureTarget::parse(params.wait_until.as_deref())?;
    let launch_mode = crate::unity_bridge::UnityLaunchMode::parse(params.mode.as_deref())?;
    let timeout_ms = params.timeout_ms.unwrap_or(DEFAULT_UNITY_ENSURE_TIMEOUT_MS);
    if timeout_ms == 0 || timeout_ms > MAX_UNITY_ENSURE_TIMEOUT_MS {
        return Err(format!(
            "timeoutMs must be between 1 and {MAX_UNITY_ENSURE_TIMEOUT_MS}"
        ));
    }
    let started_at = std::time::Instant::now();
    let runtime = resolve_sdk_unity_runtime(app, &params.project, "unity.editor.ensure")?;
    let checkout_id = runtime.checkout_id().to_string();
    let ensure_lock = unity_ensure_lock(&checkout_id).await;
    let _ensure_guard = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        ensure_lock.lock(),
    )
    .await
    .map_err(|_| {
        format!(
            "Timed out after {timeout_ms}ms waiting for another Unity ensure operation in checkout {checkout_id}"
        )
    })?;

    // Starting the checkout service establishes the monitor and readiness
    // observer before a newly spawned editor begins connecting.
    let registry = app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
    let execution = registry
        .execution_context(runtime.checkout_id(), &[ServiceKind::Unity])
        .await
        .map_err(|error| format!("unity.editor.ensure could not start Unity service: {error}"))?;
    let _service_binding = execution
        .resolve_service(ServiceKind::Unity)
        .map_err(|error| format!("unity.editor.ensure Unity service is unavailable: {error}"))?;

    let initial_status = sdk_unity_editor_status(&runtime).await;
    if matches!(
        initial_status.process_state,
        crate::unity_bridge::UnityEditorProcessState::Running
    ) {
        match (launch_mode, initial_status.launch_mode) {
            (crate::unity_bridge::UnityLaunchMode::Headless, Some(mode))
                if mode != crate::unity_bridge::UnityLaunchMode::Headless =>
            {
                return Err(
                    "unity.editor.ensure requested headless mode, but this checkout is already open in an interactive editor"
                        .to_string(),
                );
            }
            (crate::unity_bridge::UnityLaunchMode::Headless, None) => {
                return Err(
                    "unity.editor.ensure cannot verify that the running editor is headless"
                        .to_string(),
                );
            }
            (crate::unity_bridge::UnityLaunchMode::Interactive, Some(mode))
                if mode == crate::unity_bridge::UnityLaunchMode::Headless =>
            {
                return Err(
                    "unity.editor.ensure requested interactive mode, but this checkout is already open headless"
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    if initial_status.satisfies(target) {
        return serde_json::to_value(SdkEnsureUnityEditorResult {
            launched: false,
            wait_until: target.as_str().to_string(),
            waited_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            launch: None,
            status: initial_status,
        })
        .map_err(|error| error.to_string());
    }
    if initial_status.safe_mode && target == UnityEnsureTarget::Ready {
        return Err(unity_safe_mode_wait_error(&initial_status, target));
    }

    let launch = match initial_status.process_state {
        crate::unity_bridge::UnityEditorProcessState::Running => None,
        crate::unity_bridge::UnityEditorProcessState::NotRunning => Some(
            crate::unity_bridge::launch_project_with_mode(
                &initial_status.project_path,
                launch_mode,
            )
            .await
            .map_err(|error| format!("unity.editor.ensure failed to launch Unity: {error}"))?,
        ),
        crate::unity_bridge::UnityEditorProcessState::Unknown => {
            return Err(format!(
                "unity.editor.ensure cannot safely launch while the Unity process state is unknown: {}",
                initial_status
                    .connection
                    .process_last_error
                    .as_deref()
                    .or(initial_status.connection.last_error.as_deref())
                    .unwrap_or("process probe returned no diagnostic")
            ));
        }
    };
    let launched = launch.is_some();
    let launch_created_at_ms = launch.as_ref().and_then(|launch| {
        crate::unity_bridge::launched_unity_process_created_at_ms(launch.process_id)
    });
    let mut launch_liveness_probe_error: Option<String> = None;

    loop {
        let status = sdk_unity_editor_status(&runtime).await;
        let mut launch_liveness = None;
        if let Some(expected_launch) = launch.as_ref() {
            if status.process_id != Some(expected_launch.process_id) {
                match crate::unity_bridge::reaffirm_launched_unity_editor_process(
                    &expected_launch.project_path,
                    &expected_launch.editor_path,
                    expected_launch.process_id,
                    launch_created_at_ms,
                )
                .await
                {
                    Ok(liveness) => {
                        launch_liveness_probe_error = None;
                        launch_liveness = Some(liveness);
                    }
                    Err(error) => launch_liveness_probe_error = Some(error),
                }
            } else if status.satisfies(target) {
                match crate::unity_bridge::launched_unity_process_liveness(
                    expected_launch.process_id,
                    launch_created_at_ms,
                ) {
                    Ok(liveness) => {
                        launch_liveness_probe_error = None;
                        launch_liveness = Some(liveness);
                    }
                    Err(error) => launch_liveness_probe_error = Some(error),
                }
            }
            match unity_launch_wait_state(
                status.satisfies(target),
                status.process_id,
                expected_launch.process_id,
                launch_liveness,
            ) {
                UnityLaunchWaitState::Satisfied => {
                    return serde_json::to_value(SdkEnsureUnityEditorResult {
                        launched,
                        wait_until: target.as_str().to_string(),
                        waited_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX))
                            as u64,
                        launch,
                        status,
                    })
                    .map_err(|error| error.to_string());
                }
                UnityLaunchWaitState::Exited => {
                    return Err(format!(
                        "Unity Editor process {} exited before reaching '{}': phase={}, project={}",
                        expected_launch.process_id,
                        target.as_str(),
                        status.semantic_phase,
                        status.project_path
                    ));
                }
                UnityLaunchWaitState::Waiting => {}
            }
        } else if status.satisfies(target) {
            return serde_json::to_value(SdkEnsureUnityEditorResult {
                launched,
                wait_until: target.as_str().to_string(),
                waited_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                launch,
                status,
            })
            .map_err(|error| error.to_string());
        }
        let safe_mode_is_authoritative = launch.as_ref().map_or(true, |expected_launch| {
            sdk_semantic_status_matches_launch(&status, expected_launch.process_id)
        });
        if status.safe_mode && target == UnityEnsureTarget::Ready && safe_mode_is_authoritative {
            return Err(unity_safe_mode_wait_error(&status, target));
        }

        if launch.is_none()
            && matches!(
                status.process_state,
                crate::unity_bridge::UnityEditorProcessState::NotRunning
            )
        {
            return Err(format!(
                "Unity Editor exited before reaching '{}': phase={}, project={}",
                target.as_str(),
                status.semantic_phase,
                status.project_path
            ));
        }

        if started_at.elapsed() >= Duration::from_millis(timeout_ms) {
            return Err(format!(
                "Timed out after {}ms waiting for Unity Editor to reach '{}': process={:?}, phase={}, channel={}, project={}, launchProbe={}",
                timeout_ms,
                target.as_str(),
                status.process_state,
                status.semantic_phase,
                status.semantic.channel.control_pipe,
                status.project_path,
                launch_liveness_probe_error.as_deref().unwrap_or("ok")
            ));
        }

        tokio::time::sleep(Duration::from_millis(UNITY_ENSURE_POLL_INTERVAL_MS)).await;
    }
}

async fn restart_unity_editor(
    app: &AppHandle,
    params: RestartUnityEditorParams,
) -> Result<Value, String> {
    use crate::workspace_service::ServiceKind;

    let target = UnityEnsureTarget::parse(params.wait_until.as_deref())?;
    let launch_mode = crate::unity_bridge::UnityLaunchMode::parse(params.mode.as_deref())?;
    let timeout_ms = params.timeout_ms.unwrap_or(DEFAULT_UNITY_ENSURE_TIMEOUT_MS);
    if timeout_ms == 0 || timeout_ms > MAX_UNITY_ENSURE_TIMEOUT_MS {
        return Err(format!(
            "timeoutMs must be between 1 and {MAX_UNITY_ENSURE_TIMEOUT_MS}"
        ));
    }
    let started_at = std::time::Instant::now();
    let runtime = resolve_sdk_unity_runtime(app, &params.project, "unity.editor.restart")?;
    let checkout_id = runtime.checkout_id().to_string();
    let restart_lock = unity_ensure_lock(&checkout_id).await;
    let _restart_guard = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        restart_lock.lock(),
    )
    .await
    .map_err(|_| {
        format!(
            "Timed out after {timeout_ms}ms waiting for another Unity lifecycle operation in checkout {checkout_id}"
        )
    })?;

    // Keep the workspace's Unity monitor alive throughout close and launch so
    // the replacement process can immediately reconnect to the same checkout.
    let registry = app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
    let execution = registry
        .execution_context(runtime.checkout_id(), &[ServiceKind::Unity])
        .await
        .map_err(|error| format!("unity.editor.restart could not start Unity service: {error}"))?;
    let _service_binding = execution
        .resolve_service(ServiceKind::Unity)
        .map_err(|error| format!("unity.editor.restart Unity service is unavailable: {error}"))?;

    let project_path = runtime.root().to_string_lossy().to_string();
    let close_timeout = Duration::from_millis(timeout_ms.min(60_000));
    let close = if params.force {
        crate::unity_bridge::force_close_current_project_unity_processes(
            &project_path,
            close_timeout,
        )
        .await
    } else {
        crate::unity_bridge::close_current_project_unity_processes(&project_path, close_timeout)
            .await
    }
    .map_err(|error| format!("unity.editor.restart failed to close Unity: {error}"))?;

    let launch = crate::unity_bridge::launch_project_with_mode(&project_path, launch_mode)
        .await
        .map_err(|error| format!("unity.editor.restart failed to launch Unity: {error}"))?;
    let launch_created_at_ms =
        crate::unity_bridge::launched_unity_process_created_at_ms(launch.process_id);
    let mut launch_liveness_probe_error: Option<String> = None;

    loop {
        let status = sdk_unity_editor_status(&runtime).await;
        let mut launch_liveness = None;
        if status.process_id != Some(launch.process_id) {
            match crate::unity_bridge::reaffirm_launched_unity_editor_process(
                &launch.project_path,
                &launch.editor_path,
                launch.process_id,
                launch_created_at_ms,
            )
            .await
            {
                Ok(liveness) => {
                    launch_liveness_probe_error = None;
                    launch_liveness = Some(liveness);
                }
                Err(error) => launch_liveness_probe_error = Some(error),
            }
        } else if status.satisfies(target) {
            match crate::unity_bridge::launched_unity_process_liveness(
                launch.process_id,
                launch_created_at_ms,
            ) {
                Ok(liveness) => {
                    launch_liveness_probe_error = None;
                    launch_liveness = Some(liveness);
                }
                Err(error) => launch_liveness_probe_error = Some(error),
            }
        }
        match unity_launch_wait_state(
            status.satisfies(target),
            status.process_id,
            launch.process_id,
            launch_liveness,
        ) {
            UnityLaunchWaitState::Satisfied => {
                return serde_json::to_value(SdkRestartUnityEditorResult {
                    closed_process_ids: close.process_ids,
                    forced_process_ids: close.forced_process_ids,
                    wait_until: target.as_str().to_string(),
                    waited_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    launch,
                    status,
                })
                .map_err(|error| error.to_string());
            }
            UnityLaunchWaitState::Exited => {
                return Err(format!(
                    "Restarted Unity Editor process {} exited before reaching '{}': phase={}, project={}",
                    launch.process_id,
                    target.as_str(),
                    status.semantic_phase,
                    status.project_path
                ));
            }
            UnityLaunchWaitState::Waiting => {}
        }
        if status.safe_mode
            && target == UnityEnsureTarget::Ready
            && sdk_semantic_status_matches_launch(&status, launch.process_id)
        {
            return Err(unity_safe_mode_wait_error(&status, target));
        }

        if started_at.elapsed() >= Duration::from_millis(timeout_ms) {
            return Err(format!(
                "Timed out after {}ms waiting for restarted Unity Editor to reach '{}': process={:?}, phase={}, channel={}, project={}, launchPid={}, launchProbe={}",
                timeout_ms,
                target.as_str(),
                status.process_state,
                status.semantic_phase,
                status.semantic.channel.control_pipe,
                status.project_path,
                launch.process_id,
                launch_liveness_probe_error.as_deref().unwrap_or("ok")
            ));
        }

        tokio::time::sleep(Duration::from_millis(UNITY_ENSURE_POLL_INTERVAL_MS)).await;
    }
}

async fn get_unity_dialog(app: &AppHandle, params: UnityDialogParams) -> Result<Value, String> {
    let project = resolve_sdk_unity_project(app, &params.project, "unity.dialog.get")?;
    crate::unity_bridge::dialog::ensure_project_observed(&project).await?;
    serde_json::to_value(crate::unity_bridge::dialog::current_dialog(&project))
        .map_err(|error| error.to_string())
}

async fn choose_unity_dialog(
    app: &AppHandle,
    params: ChooseUnityDialogParams,
) -> Result<Value, String> {
    let project = resolve_sdk_unity_project(app, &params.project, "unity.dialog.choose")?;
    let dialog_id = params.dialog_id.trim();
    let choice_id = params.choice_id.trim();
    if dialog_id.is_empty() {
        return Err("unity.dialog.choose requires dialogId".to_string());
    }
    if choice_id.is_empty() {
        return Err("unity.dialog.choose requires choiceId".to_string());
    }
    let result = crate::unity_bridge::dialog::choose_dialog(&project, dialog_id, choice_id).await?;
    serde_json::to_value(result).map_err(|error| error.to_string())
}

async fn wait_unity_execution(
    app: &AppHandle,
    params: WaitUnityExecutionParams,
) -> Result<Value, String> {
    let project = resolve_sdk_unity_project(app, &params.project, "unity.execution.wait")?;
    let execution_id = params.execution_id.trim();
    if execution_id.is_empty() {
        return Err("unity.execution.wait requires executionId".to_string());
    }
    let output = crate::unity_bridge::wait_unity_execution(&project, execution_id).await?;
    Ok(Value::String(output))
}

fn sdk_runtime_status(status: &str) -> SessionRuntimeStatus {
    match status {
        "queued" => SessionRuntimeStatus::Queued,
        "starting" => SessionRuntimeStatus::Starting,
        "waiting_input" => SessionRuntimeStatus::WaitingInput,
        "finishing" => SessionRuntimeStatus::Finishing,
        "cancelling" => SessionRuntimeStatus::Cancelling,
        "error" => SessionRuntimeStatus::Error,
        _ => SessionRuntimeStatus::Running,
    }
}

async fn populate_sdk_session_runtime_statuses(
    app: &AppHandle,
    sessions: &mut [SessionSummary],
) -> Result<(), String> {
    let active_tasks = app.state::<ActiveTasks>();
    let active_runs = active_tasks
        .lock()
        .await
        .iter()
        .map(|(session_id, task)| (session_id.clone(), task.run_id.clone()))
        .collect::<HashMap<_, _>>();
    let store = app.state::<Arc<SessionStore>>();

    for session in sessions {
        session.runtime_status = match active_runs.get(&session.id) {
            Some(run_id) => store
                .run_by_id(run_id)?
                .map(|run| sdk_runtime_status(&run.status))
                .or(Some(SessionRuntimeStatus::Running)),
            None => None,
        };
    }
    Ok(())
}

async fn list_sdk_sessions(app: &AppHandle, params: ListSessionsParams) -> Result<Value, String> {
    let store = app.state::<Arc<SessionStore>>();
    let mut sessions = match params.workspace_ref.as_ref() {
        Some(workspace_ref) => {
            let scope = resolve_sdk_workspace_scope(app, workspace_ref, "sessions.list")?;
            if params.archived {
                store.list_archived_sessions_for_checkout(scope.runtime().checkout_id().as_str())?
            } else {
                store.list_sessions_for_checkout(scope.runtime().checkout_id().as_str())?
            }
        }
        None if params.archived => store.list_archived_sessions(None)?,
        None => store.list_sessions(None)?,
    };
    populate_sdk_session_runtime_statuses(app, &mut sessions).await?;
    if params.running_only {
        sessions.retain(|session| session.runtime_status.is_some());
    }
    if let Some(limit) = params.limit {
        sessions.truncate(limit.clamp(1, 1_000) as usize);
    }
    serde_json::to_value(sessions).map_err(|error| error.to_string())
}

fn single_line_session_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if title.is_empty() {
        "未命名会话".to_string()
    } else {
        title
    };
    title.chars().take(160).collect()
}

fn cross_session_message(
    source_session_id: &str,
    source_session_title: &str,
    message: &str,
) -> String {
    format!(
        "[来自 Locus session：{}（{}）]\n\n{}",
        single_line_session_title(source_session_title),
        source_session_id,
        message.trim()
    )
}

async fn send_sdk_session_message(
    app: &AppHandle,
    params: SendSessionMessageParams,
) -> Result<Value, String> {
    let target_session_id = params.session_id.trim();
    let source_session_id = params.source_session_id.trim();
    let message = params.message.trim();
    if target_session_id.is_empty() {
        return Err("sessionId cannot be empty".to_string());
    }
    if source_session_id.is_empty() {
        return Err(
            "sourceSessionId is required; call this API from a running Locus session".to_string(),
        );
    }
    if target_session_id == source_session_id {
        return Err("A session cannot send a message to itself".to_string());
    }
    if message.is_empty() {
        return Err("message cannot be empty".to_string());
    }

    let store = app.state::<Arc<SessionStore>>();
    let source_session_title = store
        .get_session_title(source_session_id)?
        .ok_or_else(|| format!("Source session not found: {source_session_id}"))?;
    let target_session_title = store
        .get_session_title(target_session_id)?
        .ok_or_else(|| format!("Target session not found: {target_session_id}"))?;

    {
        let active_tasks = app.state::<ActiveTasks>();
        let tasks = active_tasks.lock().await;
        if !tasks.contains_key(source_session_id) {
            return Err(format!(
                "Source session '{source_session_id}' is no longer running"
            ));
        }
    }

    let text = cross_session_message(source_session_id, &source_session_title, message);
    let target_run_id = {
        let active_tasks = app.state::<ActiveTasks>();
        let run_id = active_tasks
            .lock()
            .await
            .get(target_session_id)
            .map(|task| task.run_id.clone())
            .ok_or_else(|| format!("Target session '{target_session_id}' is not running"))?;
        run_id
    };
    let pending = crate::commands::queue_chat_input(
        target_session_id.to_string(),
        target_run_id,
        format!(
            "sdk-session-message:{}:{}",
            source_session_id,
            uuid::Uuid::new_v4()
        ),
        text.clone(),
        Some(text),
        None,
        None,
        Some("build".to_string()),
        None,
        None,
        Some("immediate".to_string()),
        app.clone(),
        app.state::<Arc<SessionStore>>(),
        app.state::<crate::PendingInputQueueHandle>(),
        app.state::<ActiveTasks>(),
    )
    .await
    .map_err(|error| error.to_string())?;

    serde_json::to_value(SdkSessionMessageDelivery {
        pending_input_id: pending.id,
        source_session_id: source_session_id.to_string(),
        source_session_title,
        target_session_id: target_session_id.to_string(),
        target_session_title,
        target_run_id: pending.run_id.clone(),
        delivery: pending.delivery,
    })
    .map_err(|error| error.to_string())
}

fn get_sdk_session(app: &AppHandle, params: SessionParams) -> Result<Value, String> {
    let session_id = params.session_id.trim();
    if session_id.is_empty() {
        return Err("sessionId cannot be empty".to_string());
    }
    let store = app.state::<Arc<SessionStore>>();
    let mut detail = store.load_session(session_id)?;
    detail.runtime = store.runtime_snapshot_for_session(session_id);
    serde_json::to_value(detail).map_err(|error| error.to_string())
}

fn list_sdk_session_events(app: &AppHandle, params: SessionEventsParams) -> Result<Value, String> {
    let session_id = params.session_id.trim();
    if session_id.is_empty() {
        return Err("sessionId cannot be empty".to_string());
    }
    let store = app.state::<Arc<SessionStore>>();
    serde_json::to_value(store.list_session_events(
        session_id,
        params.after_seq,
        params.limit.map(|value| value.clamp(1, 2_000)),
    )?)
    .map_err(|error| error.to_string())
}

fn direct_tool_result(
    name: &str,
    output: String,
    is_error: bool,
    images: Value,
    workspace_path: Option<String>,
) -> Value {
    json!({
        "name": name,
        "output": output,
        "isError": is_error,
        "images": images,
        "workspacePath": workspace_path,
    })
}

async fn call_knowledge_query(
    app: &AppHandle,
    workspace_ref: crate::workspace_service::WorkspaceRef,
    arguments: &Value,
) -> ToolResult {
    let string_arg = |name: &str| {
        arguments
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 20) as usize);
    let types = arguments
        .get("types")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    match crate::commands::knowledge_query(
        workspace_ref,
        string_arg("query"),
        string_arg("lexicalQuery"),
        string_arg("semanticQuery"),
        limit,
        types,
        string_arg("pathPrefix"),
        Some(false),
        app.clone(),
        app.state::<Arc<crate::workspace_service::ProjectRegistry>>(),
        app.state::<crate::commands::AppKnowledgeDir>(),
    )
    .await
    {
        Ok(hits) => ToolResult {
            output: serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string()),
            is_error: false,
        },
        Err(error) => ToolResult {
            output: error.to_string(),
            is_error: true,
        },
    }
}

fn call_config_query(
    app: &AppHandle,
    arguments: &Value,
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
) -> ToolResult {
    let category = arguments
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let entries = match category {
        Some(category) => {
            crate::config_registry::collect_by_category(app, category, workspace_scope)
        }
        None => crate::config_registry::collect_all(app, workspace_scope),
    };
    match entries {
        Ok(entries) => ToolResult {
            output: serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string()),
            is_error: false,
        },
        Err(error) => ToolResult {
            output: error.to_string(),
            is_error: true,
        },
    }
}

async fn call_tool(app: &AppHandle, params: CallToolParams) -> Result<Value, String> {
    if !params.arguments.is_object() {
        return Err("arguments must be an object".to_string());
    }
    let name = params.name.trim();
    if name.is_empty() {
        return Err("name cannot be empty".to_string());
    }
    let timeout_ms = params
        .timeout_ms
        .unwrap_or(DEFAULT_TOOL_TIMEOUT_MS)
        .clamp(1_000, MAX_TOOL_TIMEOUT_MS);
    let workspace_scope = params
        .workspace_ref
        .as_ref()
        .map(|workspace_ref| resolve_sdk_workspace_scope(app, workspace_ref, "tools.call"))
        .transpose()?;
    let working_dir = workspace_scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string());
    let registry = match workspace_scope.as_ref() {
        Some(scope) => {
            let definitions = app
                .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
                .snapshot(scope.runtime().as_ref())
                .await
                .map_err(|error| format!("Failed to load checkout Agent definitions: {error}"))?;
            app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>()
                .snapshot(scope.runtime().as_ref(), definitions.as_ref())
                .await?
        }
        None => app.state::<Arc<ToolRegistry>>().inner().clone(),
    };
    let canonical = registry
        .canonical_name(name)
        .or_else(|| crate::mcp::manager::resolve_wire_tool(name).map(|tool| tool.wire_name))
        .ok_or_else(|| format!("Unknown Locus tool '{name}'"))?;
    if is_agent_only_tool(&canonical) {
        return Err(format!(
            "Tool '{canonical}' requires an active Agent run and cannot be called directly"
        ));
    }

    if canonical.starts_with(crate::mcp::manager::MCP_TOOL_PREFIX) {
        let outcome = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            crate::mcp::manager::call_tool(&canonical, params.arguments, None),
        )
        .await;
        return match outcome {
            Ok(Ok(outcome)) => Ok(direct_tool_result(
                &canonical,
                outcome.text,
                false,
                serde_json::to_value(outcome.images).map_err(|error| error.to_string())?,
                None,
            )),
            Ok(Err(error)) => Ok(direct_tool_result(
                &canonical,
                error,
                true,
                Value::Array(Vec::new()),
                None,
            )),
            Err(_) => Ok(direct_tool_result(
                &canonical,
                format!("Tool '{canonical}' timed out after {}s", timeout_ms / 1_000),
                true,
                Value::Array(Vec::new()),
                None,
            )),
        };
    }

    if crate::mcp::server::tools::EXPOSED_TOOLS.contains(&canonical.as_str()) {
        if workspace_scope.is_none() {
            return Err(format!(
                "Tool '{canonical}' requires workspaceRef with a live checkout generation"
            ));
        }
        let outcome = crate::mcp::server::tools::execute_tool(
            app.clone(),
            canonical.clone(),
            params.arguments,
            timeout_ms,
            Arc::new(ToolRuntimeState::default()),
            workspace_scope
                .as_ref()
                .expect("workspace scope is required above")
                .workspace_ref(),
        )
        .await;
        let images = outcome
            .images
            .into_iter()
            .map(|(data, mime_type)| json!({ "data": data, "mimeType": mime_type }))
            .collect::<Vec<_>>();
        return Ok(direct_tool_result(
            &canonical,
            outcome.output,
            outcome.is_error,
            Value::Array(images),
            outcome.workspace_path,
        ));
    }

    if canonical == "knowledge_query" {
        let workspace_ref = workspace_scope
            .as_ref()
            .map(|scope| scope.workspace_ref())
            .ok_or_else(|| {
                "Tool 'knowledge_query' requires workspaceRef with a live checkout generation"
                    .to_string()
            })?;
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            call_knowledge_query(app, workspace_ref, &params.arguments),
        )
        .await
        .unwrap_or_else(|_| ToolResult {
            output: format!("Tool '{canonical}' timed out after {}s", timeout_ms / 1_000),
            is_error: true,
        });
        return Ok(direct_tool_result(
            &canonical,
            result.output,
            result.is_error,
            Value::Array(Vec::new()),
            working_dir,
        ));
    }
    if canonical == "config_query" {
        let result = call_config_query(app, &params.arguments, workspace_scope.as_ref());
        return Ok(direct_tool_result(
            &canonical,
            result.output,
            result.is_error,
            Value::Array(Vec::new()),
            working_dir,
        ));
    }
    if workspace_scope.is_none() {
        return Err(format!(
            "Tool '{canonical}' requires workspaceRef with a live checkout generation"
        ));
    }
    let workspace_event_scope = crate::workspace_service::event::WorkspaceEventScope::for_runtime(
        workspace_scope
            .as_ref()
            .expect("workspace scope is required above")
            .runtime()
            .as_ref(),
    );
    let unity_connected = if canonical == "read"
        && params
            .arguments
            .get("filePath")
            .and_then(Value::as_str)
            .is_some_and(crate::tool::is_unity_yaml_candidate_path)
    {
        match working_dir.as_deref() {
            Some(path) => Some(crate::unity_bridge::is_unity_connected(path).await),
            None => Some(false),
        }
    } else {
        None
    };
    let execution = match working_dir.as_deref() {
        Some(working_dir) => Some(
            app.state::<Arc<crate::workspace_service::ProjectRegistry>>()
                .tool_execution_context(std::path::Path::new(working_dir), &canonical)
                .await?,
        ),
        None => None,
    };
    let context = ToolExecutionContext {
        app_handle: Some(app.clone()),
        execution,
        working_dir: working_dir.clone(),
        process_owner: Some(crate::process_util::ProcessOwner {
            working_dir: working_dir.clone(),
            ..Default::default()
        }),
        unity_connected,
        runtime_state: Some(Arc::new(ToolRuntimeState::default())),
        cancel_rx: None,
        progress: None,
        output: None,
        output_path: None,
        background: false,
    };
    let lock_request = if matches!(canonical.as_str(), "write" | "edit") {
        Some(
            params
                .arguments
                .get("filePath")
                .and_then(Value::as_str)
                .map(|path| {
                    crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::PathWrite(
                        vec![crate::agent::workspace_execution_lock::normalize_workspace_path_key(
                            working_dir.as_deref().unwrap_or_default(),
                            path,
                        )],
                    )
                })
                .unwrap_or(
                    crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::Exclusive,
                ),
        )
    } else if canonical == "bash" {
        crate::agent::instance::AgentInstance::bash_needs_primary_workspace_tracking_for(
            working_dir.as_deref().unwrap_or_default(),
            &params.arguments,
        )
        .then_some(crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::Exclusive)
    } else if canonical == "python" {
        (!crate::tool::builtins::python_is_readonly(&params.arguments)).then_some(
            crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::Exclusive,
        )
    } else if canonical == "unity_execute" {
        (!crate::agent::instance::AgentInstance::unity_execute_is_readonly(&params.arguments))
            .then_some(
                crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::Exclusive,
            )
    } else if registry.mutates_workspace(&canonical)
        || crate::agent::instance::AgentInstance::is_unity_execution_barrier_tool(&canonical)
    {
        Some(crate::agent::workspace_execution_lock::WorkspaceExecutionLockRequest::Exclusive)
    } else {
        None
    };
    let owner = crate::agent::workspace_execution_lock::WorkspaceExecutionLockOwner {
        session_id: "python-sdk".to_string(),
        run_id: format!("sdk-tool-{}", uuid::Uuid::new_v4()),
        iteration: 0,
        workspace: working_dir.clone().unwrap_or_default(),
        tools: vec![canonical.clone()],
    };
    let (_lock_cancel_tx, lock_cancel_rx) = tokio::sync::watch::channel(false);
    let execute = async {
        let guard = if let Some(request) = lock_request {
            match crate::agent::workspace_execution_lock::process_workspace_execution_lock(
                &owner.workspace,
            )
            .acquire_with_diagnostics(
                request,
                owner,
                lock_cancel_rx,
                workspace_event_scope,
                &app,
            )
            .await
            {
                Ok(guard) => Some(guard),
                Err(_) => {
                    return ToolResult {
                        output: format!(
                            "Tool '{canonical}' was cancelled while waiting for workspace mutation coordination"
                        ),
                        is_error: true,
                    }
                }
            }
        } else {
            None
        };
        let result = registry
            .execute_with_context(&canonical, &params.arguments, context)
            .await;
        drop(guard);
        result
    };
    let result = tokio::time::timeout(Duration::from_millis(timeout_ms), execute)
        .await
        .unwrap_or_else(|_| ToolResult {
            output: format!("Tool '{canonical}' timed out after {}s", timeout_ms / 1_000),
            is_error: true,
        });
    Ok(direct_tool_result(
        &canonical,
        result.output,
        result.is_error,
        Value::Array(Vec::new()),
        working_dir,
    ))
}

/// Start an idle root receiver with its persisted settings. The actual message
/// stays in the durable mailbox and is injected as an agent reminder.
pub(crate) async fn wake_session_for_agent_message(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let store = app.state::<Arc<SessionStore>>();
    if store.pending_agent_messages(session_id)?.is_empty() && store.pending_async_notifications(session_id)?.is_empty() { return Ok(()); }
    let detail = store.load_session(session_id)?;
    crate::commands::chat(
        Some(session_id.to_string()),
        None,
        String::new(),
        None, None,
        detail.agent_id,
        None,
        detail.last_model_id,
        detail.last_effort,
        detail.last_fast_mode,
        detail.last_multi_agent_enabled,
        None, None,
        Some(detail.session_type),
        Some(if store.get_plan_mode_state(session_id)?.active { "plan" } else { "build" }.into()),
        None, None, None, None, None, None, None,
        app.clone(),
        app.state::<Arc<SessionStore>>(),
        app.state::<AgentDefRegistryState>(),
        app.state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>(),
        app.state::<Arc<crate::config::AppConfig>>(),
        app.state::<Arc<ToolRegistry>>(),
        app.state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>(),
        app.state::<Arc<tokio::sync::Mutex<crate::auth::AuthState>>>(),
        app.state::<ApiKeyState>(),
        app.state::<ProviderKeysState>(),
        app.state::<crate::commands::CodexAuthStateHandle>(),
        app.state::<Arc<crate::workspace_service::ProjectRegistry>>(),
        app.state::<RawContextStore>(),
        app.state::<ActiveTasks>(),
        app.state::<crate::commands::AppKnowledgeDir>(),
        app.state::<AppAgentDir>(),
        app.state::<UndoManagerHandle>(),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn dispatch(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "tasks.list" | "tasks.get" | "tasks.cancel" | "tasks.resume" | "tasks.wait" | "tasks.send_message" => {
            let session_id = params.get("sessionId").and_then(Value::as_str).map(str::trim)
                .filter(|id| !id.is_empty()).ok_or("Task APIs require the current sessionId")?;
            let manager = app.state::<Arc<crate::async_tasks::AsyncTaskManager>>();
            if method == "tasks.list" {
                let tasks = manager.list_session_tasks(session_id)?.into_iter()
                    .map(crate::async_tasks::AsyncTaskManager::task_payload).collect::<Result<Vec<_>, _>>()?;
                return Ok(Value::Array(tasks));
            }
            let task_id = params.get("taskId").and_then(Value::as_str).map(str::trim)
                .filter(|id| !id.is_empty()).ok_or("taskId is required")?;
            if method == "tasks.send_message" {
                let message = params.get("message").and_then(Value::as_str).ok_or("message is required")?;
                let (receipt, task) = manager.queue_task_message(session_id, task_id, message)?;
                if let Some(task) = task {
                    manager.inner().ensure_message_delivery(app.clone(), task, receipt["messageId"].as_str().unwrap().to_string());
                } else {
                    let (target, _, _) = manager.resolve_message_target(session_id, task_id)?;
                    manager.inner().ensure_parent_message_delivery(app.clone(), target, receipt["messageId"].as_str().unwrap().to_string());
                }
                return Ok(receipt);
            }
            if method == "tasks.wait" {
                let timeout = params.get("timeoutMs").and_then(Value::as_u64).unwrap_or(30_000);
                return crate::async_tasks::AsyncTaskManager::task_payload(manager.wait_task(session_id, task_id, timeout).await?);
            }
            let current = manager.get_session_task(session_id, task_id)?;
            let snapshot = if method == "tasks.cancel" {
                manager.cancel(&current.task_id)?
            } else if method == "tasks.resume" {
                if !app.state::<Arc<crate::config::AppConfig>>().async_tasks_enabled() {
                    return Err("Async tasks are disabled in Settings > Experimental.".into());
                }
                let message = params.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
                manager.inner().resume_task(session_id, task_id, message, app.clone())?
            } else {
                current
            };
            crate::async_tasks::AsyncTaskManager::task_payload(snapshot)
        }
        "agents.list" => list_agents(app).await,
        "agents.prompt" => prompt_agent(app, parse_params(params)?).await,
        "models.list" => list_models(app, parse_params(params)?).await,
        "tools.list" => {
            let params = if params.is_null() {
                ListToolsParams::default()
            } else {
                parse_params(params)?
            };
            list_tools(app, params).await
        }
        "tools.call" => call_tool(app, parse_params(params)?).await,
        "workspace.get" => workspace_snapshot(app, parse_params(params)?).await,
        "unity.editor.status" => get_unity_editor_status(app, parse_params(params)?).await,
        "unity.editor.ensure" => ensure_unity_editor(app, parse_params(params)?).await,
        "unity.editor.restart" => restart_unity_editor(app, parse_params(params)?).await,
        "unity.dialog.get" => get_unity_dialog(app, parse_params(params)?).await,
        "unity.dialog.choose" => choose_unity_dialog(app, parse_params(params)?).await,
        "unity.execution.wait" => wait_unity_execution(app, parse_params(params)?).await,
        "sessions.list" => list_sdk_sessions(app, parse_params(params)?).await,
        "sessions.get" => get_sdk_session(app, parse_params(params)?),
        "sessions.send" => send_sdk_session_message(app, parse_params(params)?).await,
        "sessions.events" => list_sdk_session_events(app, parse_params(params)?),
        "runs.get" => {
            let params: RunParams = parse_params(params)?;
            run_snapshot(app, &params.run_id)
        }
        "runs.wait" => wait_run(app, parse_params(params)?).await,
        "runs.events" => {
            let params: RunEventsParams = parse_params(params)?;
            let store = app.state::<Arc<SessionStore>>();
            serde_json::to_value(store.list_run_events(
                &params.run_id,
                params.after_seq,
                params.limit,
            )?)
            .map_err(|error| error.to_string())
        }
        "runs.cancel" => cancel_run(app, parse_params(params)?).await,
        "runs.answer" => answer_run(app, parse_params(params)?).await,
        _ => Err(format!("Unknown SDK method '{method}'")),
    }
}

async fn handle_request(
    request: Request<hyper::body::Incoming>,
    app: AppHandle,
    token: Arc<String>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    Ok(handle_request_inner(request, app, token).await)
}

async fn handle_request_inner(
    request: Request<hyper::body::Incoming>,
    app: AppHandle,
    token: Arc<String>,
) -> Response<Full<Bytes>> {
    if request.uri().path() != SDK_PATH {
        return plain_response(StatusCode::NOT_FOUND, "not found");
    }
    if request.method() != Method::POST {
        return plain_response(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if request.headers().get("origin").is_some() {
        return plain_response(StatusCode::FORBIDDEN, "browser origins are not allowed");
    }
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok());
    if !host_allowed(host) {
        return plain_response(StatusCode::FORBIDDEN, "invalid host");
    }
    let authorized = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, provided)| token_matches(token.as_str(), provided.trim()))
        .unwrap_or(false);
    if !authorized {
        return plain_response(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
    }

    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => return plain_response(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if body.len() > MAX_BODY_BYTES {
        return plain_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    }
    let rpc: RpcRequest = match serde_json::from_slice(&body) {
        Ok(rpc) => rpc,
        Err(error) => return json_response(&rpc_error(Value::Null, -32700, error.to_string())),
    };
    let id = rpc.id.clone();
    let response = match dispatch(&app, &rpc.method, rpc.params).await {
        Ok(result) => rpc_success(id, result),
        Err(error) => {
            let code = if error.starts_with("Unknown SDK method") {
                -32601
            } else {
                -32602
            };
            rpc_error(id, code, error)
        }
    };
    json_response(&response)
}

pub async fn start(app: AppHandle) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("Failed to bind the Locus SDK bridge: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Failed to read the Locus SDK bridge address: {error}"))?;
    let token = Arc::new(generate_token());
    crate::python_runtime::set_locus_sdk_connection(
        format!("http://{address}{SDK_PATH}"),
        token.as_str().to_string(),
    );

    let task_app = app.clone();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app = task_app.clone();
                    let token = token.clone();
                    connections.spawn(async move {
                        let service = service_fn(move |request| {
                            handle_request(request, app.clone(), token.clone())
                        });
                        if let Err(error) = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await
                        {
                            eprintln!("[LocusSdk] connection ended: {error}");
                        }
                    });
                    while connections.try_join_next().is_some() {}
                }
                Err(error) => {
                    eprintln!("[LocusSdk] accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    });

    let handle = app.state::<Arc<SdkServerHandle>>();
    let mut current = handle
        .task
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = current.replace(task) {
        previous.abort();
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::{
        cross_session_message, host_allowed, is_agent_only_tool, sdk_runtime_status, token_matches,
        unity_launch_wait_state, validate_agent_id, ListModelsParams, ListSessionsParams,
        UnityEnsureTarget, UnityLaunchWaitState, CODEX_FALLBACK_MODELS, STATIC_MODELS,
    };
    use crate::session::models::SessionRuntimeStatus;
    use crate::unity_bridge::UnityProcessIdentityLiveness;

    #[test]
    fn validates_runtime_agent_ids() {
        assert_eq!(validate_agent_id("reviewer").unwrap(), "reviewer");
        assert_eq!(validate_agent_id("sdk-agent_2").unwrap(), "sdk-agent_2");
        assert!(validate_agent_id("Reviewer").is_err());
        assert!(validate_agent_id("doc").is_err());
        assert!(validate_agent_id("dev").is_err());
        assert!(validate_agent_id("with space").is_err());
    }

    #[test]
    fn bridge_security_checks_are_exact() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(host_allowed(Some("127.0.0.1:1234")));
        assert!(host_allowed(Some("localhost:1234")));
        assert!(!host_allowed(Some("example.com:1234")));
    }

    #[test]
    fn sdk_model_inventory_has_unique_wire_ids() {
        let mut ids = std::collections::HashSet::new();
        for model in STATIC_MODELS.iter().chain(CODEX_FALLBACK_MODELS) {
            assert!(ids.insert(model.id), "duplicate SDK model id {}", model.id);
        }
        assert!(ids.contains("openai/gpt-6-astra"));
        assert!(ids.contains("openai/gpt-5.6-sol"));
        assert!(ids.contains("claude-opus-4.8"));
        assert!(ids.contains("openrouter/claude-opus-4.8"));
    }

    #[test]
    fn model_listing_defaults_to_available_rows() {
        let params: ListModelsParams = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(params.available_only);
    }

    #[test]
    fn running_session_filter_defaults_off_and_accepts_camel_case() {
        let default_params: ListSessionsParams =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!default_params.running_only);

        let filtered: ListSessionsParams =
            serde_json::from_value(serde_json::json!({ "runningOnly": true })).unwrap();
        assert!(filtered.running_only);
    }

    #[test]
    fn cross_session_messages_carry_source_title_and_id() {
        assert_eq!(
            cross_session_message("session-source", " Source\n workflow ", " check this "),
            "[来自 Locus session：Source workflow（session-source）]\n\ncheck this"
        );
        assert_eq!(
            sdk_runtime_status("waiting_input"),
            SessionRuntimeStatus::WaitingInput
        );
    }

    #[test]
    fn unity_ensure_target_defaults_to_ready_and_rejects_unknown_values() {
        assert_eq!(
            UnityEnsureTarget::parse(None).unwrap(),
            UnityEnsureTarget::Ready
        );
        assert_eq!(
            UnityEnsureTarget::parse(Some("process")).unwrap(),
            UnityEnsureTarget::Process
        );
        assert_eq!(
            UnityEnsureTarget::parse(Some("connected")).unwrap(),
            UnityEnsureTarget::Connected
        );
        assert!(UnityEnsureTarget::parse(Some("running")).is_err());
    }

    #[test]
    fn launched_editor_waits_through_old_generation_crash_observation() {
        assert_eq!(
            unity_launch_wait_state(false, None, 5252, Some(UnityProcessIdentityLiveness::Alive),),
            UnityLaunchWaitState::Waiting
        );
        assert_eq!(
            unity_launch_wait_state(
                true,
                Some(4242),
                5252,
                Some(UnityProcessIdentityLiveness::Alive),
            ),
            UnityLaunchWaitState::Waiting
        );
        assert_eq!(
            unity_launch_wait_state(
                true,
                Some(5252),
                5252,
                Some(UnityProcessIdentityLiveness::Alive),
            ),
            UnityLaunchWaitState::Satisfied
        );
        assert_eq!(
            unity_launch_wait_state(true, Some(5252), 5252, None),
            UnityLaunchWaitState::Waiting
        );
    }

    #[test]
    fn launched_editor_exit_requires_identity_confirmation() {
        assert_eq!(
            unity_launch_wait_state(
                false,
                None,
                5252,
                Some(UnityProcessIdentityLiveness::Exited),
            ),
            UnityLaunchWaitState::Exited
        );
        assert_eq!(
            unity_launch_wait_state(
                true,
                Some(5252),
                5252,
                Some(UnityProcessIdentityLiveness::Exited),
            ),
            UnityLaunchWaitState::Exited
        );
        assert_eq!(
            unity_launch_wait_state(
                false,
                None,
                5252,
                Some(UnityProcessIdentityLiveness::Replaced),
            ),
            UnityLaunchWaitState::Exited
        );
        assert_eq!(
            unity_launch_wait_state(false, None, 5252, None),
            UnityLaunchWaitState::Waiting
        );
    }

    #[test]
    fn stateful_agent_tools_are_not_directly_callable() {
        assert!(is_agent_only_tool("subagent"));
        assert!(is_agent_only_tool("ask_user_question"));
        assert!(!is_agent_only_tool("read"));
        assert!(!is_agent_only_tool("config_query"));
        assert!(!is_agent_only_tool("unity_execute"));
    }
}
