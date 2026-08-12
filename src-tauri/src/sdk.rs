//! Local bridge used by the bundled Python `locus` package.
//!
//! The listener is always loopback-only and uses an ephemeral per-process
//! bearer token. The token and endpoint are injected only into Python
//! processes launched through Locus's selected runtime.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::net::TcpListener;

use crate::agent::definition::{canonical_agent_id, AgentDef};
use crate::session::models::{MessageRole, SessionEventRecord, SessionRunSummary};
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
    OsRng.fill_bytes(&mut bytes);
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSessionsParams {
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionParams {
    session_id: String,
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
    if canonical_agent_id(id) != id {
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

async fn list_tools(app: &AppHandle) -> Result<Value, String> {
    crate::mcp::manager::ensure_fresh().await;
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
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
) -> Result<Vec<String>, String> {
    crate::mcp::manager::ensure_fresh().await;
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    let working_dir = workspace.path.read().await.clone();
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
            system_prompt: self.system_prompt.clone(),
            env_template: String::new(),
            tools,
            sub_agents: self.sub_agents.clone(),
            default: false,
            default_effort: self.default_effort.clone(),
            model_recommendation: self.model_recommendation.clone(),
            source: "python".to_string(),
        }
    }
}

async fn prepare_agent_spec(
    app: &AppHandle,
    mut spec: SdkAgentSpec,
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
    spec.locus_tools = canonical_tool_names(app, spec.locus_tools).await?;

    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
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

async fn prompt_agent(app: &AppHandle, mut params: PromptAgentParams) -> Result<Value, String> {
    let prompt = params.prompt.trim();
    if prompt.is_empty() {
        return Err("prompt cannot be empty".to_string());
    }
    let agent_spec = match params.agent_spec.take() {
        Some(spec) => {
            let spec = prepare_agent_spec(app, spec).await?;
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
        let agent_id = canonical_agent_id(params.agent_id.trim()).to_string();
        let registry = app.state::<AgentDefRegistryState>();
        if registry.0.read().await.get(&agent_id).is_none() {
            return Err(format!("Unknown agent '{agent_id}'"));
        }
        agent_id
    };

    let store = app.state::<Arc<SessionStore>>();
    let model = resolve_prompt_model(app, store.inner().as_ref(), &params).await?;
    let effort =
        resolve_prompt_effort(app, store.inner().as_ref(), &params, agent_spec.as_ref()).await;
    let fast_mode = match params.fast_mode {
        Some(value) => Some(value),
        None => crate::commands::get_codex_fast_mode(app.clone()).await.ok(),
    };
    let model_defaults = crate::commands::get_model_defaults(app.clone())
        .await
        .unwrap_or_default();

    let launch = crate::commands::chat(
        nonempty(params.session_id),
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
        Some(params.session_type.unwrap_or_else(|| "chat".to_string())),
        Some(params.mode.unwrap_or_else(|| "build".to_string())),
        None,
        Some(
            params
                .subagent_models
                .unwrap_or(model_defaults.subagent_models),
        ),
        Some(params.knowledge_mode.unwrap_or_else(|| "full".to_string())),
        None,
        None,
        app.clone(),
        app.state::<Arc<SessionStore>>(),
        app.state::<AgentDefRegistryState>(),
        app.state::<Arc<crate::config::AppConfig>>(),
        app.state::<Arc<ToolRegistry>>(),
        app.state::<Arc<tokio::sync::Mutex<crate::auth::AuthState>>>(),
        app.state::<ApiKeyState>(),
        app.state::<ProviderKeysState>(),
        app.state::<crate::commands::CodexAuthStateHandle>(),
        app.state::<Arc<crate::workspace::Workspace>>(),
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

async fn current_workspace(app: &AppHandle) -> (Option<String>, Option<String>) {
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    let path = workspace.path.read().await.trim().to_string();
    if path.is_empty() {
        return (None, None);
    }
    let workspace_id = workspace.workspace_id.read().await.clone();
    (Some(path), workspace_id)
}

async fn workspace_snapshot(app: &AppHandle) -> Result<Value, String> {
    let (path, workspace_id) = current_workspace(app).await;
    let unity_connected = match path.as_deref() {
        Some(path) => crate::unity_bridge::is_unity_connected(path).await,
        None => false,
    };
    Ok(json!({
        "path": path,
        "workspaceId": workspace_id,
        "unityConnected": unity_connected,
    }))
}

async fn list_sdk_sessions(app: &AppHandle, params: ListSessionsParams) -> Result<Value, String> {
    let (_, workspace_id) = current_workspace(app).await;
    let store = app.state::<Arc<SessionStore>>();
    let mut sessions = if params.archived {
        store.list_archived_sessions(workspace_id.as_deref())?
    } else {
        store.list_sessions(workspace_id.as_deref())?
    };
    if let Some(limit) = params.limit {
        sessions.truncate(limit.clamp(1, 1_000) as usize);
    }
    serde_json::to_value(sessions).map_err(|error| error.to_string())
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

async fn call_knowledge_query(app: &AppHandle, arguments: &Value) -> ToolResult {
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
    match crate::commands::knowledge_query(
        None,
        string_arg("lexicalQuery"),
        string_arg("semanticQuery"),
        limit,
        None,
        string_arg("pathPrefix"),
        Some(false),
        app.state::<Arc<crate::workspace::Workspace>>(),
        app.state::<crate::commands::AppKnowledgeDir>(),
        app.state::<Arc<crate::knowledge_index::KnowledgeIndexState>>(),
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

fn call_config_query(app: &AppHandle, arguments: &Value) -> ToolResult {
    let category = arguments
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let entries = match category {
        Some(category) => crate::config_registry::collect_by_category(app, category),
        None => crate::config_registry::collect_all(app),
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
    let canonical = canonical_tool_names(app, vec![name.to_string()])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "name cannot be empty".to_string())?;
    if is_agent_only_tool(&canonical) {
        return Err(format!(
            "Tool '{canonical}' requires an active Agent run and cannot be called directly"
        ));
    }
    let timeout_ms = params
        .timeout_ms
        .unwrap_or(DEFAULT_TOOL_TIMEOUT_MS)
        .clamp(1_000, MAX_TOOL_TIMEOUT_MS);

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
        let outcome = crate::mcp::server::tools::execute_tool(
            app.clone(),
            canonical.clone(),
            params.arguments,
            timeout_ms,
            Arc::new(ToolRuntimeState::default()),
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

    let (working_dir, _) = current_workspace(app).await;
    if canonical == "knowledge_query" {
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            call_knowledge_query(app, &params.arguments),
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
        let result = call_config_query(app, &params.arguments);
        return Ok(direct_tool_result(
            &canonical,
            result.output,
            result.is_error,
            Value::Array(Vec::new()),
            working_dir,
        ));
    }
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
    let context = ToolExecutionContext {
        app_handle: Some(app.clone()),
        working_dir: working_dir.clone(),
        unity_connected,
        runtime_state: Some(Arc::new(ToolRuntimeState::default())),
        cancel_rx: None,
        progress: None,
    };
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
    let lock_mode = if registry.mutates_workspace(&canonical) {
        crate::agent::workspace_execution_lock::WorkspaceExecutionLockMode::Write
    } else {
        crate::agent::workspace_execution_lock::WorkspaceExecutionLockMode::Read
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
        let guard = match crate::agent::workspace_execution_lock::process_workspace_execution_lock()
            .acquire_with_diagnostics(lock_mode, owner, lock_cancel_rx, &app)
            .await
        {
            Ok(guard) => guard,
            Err(_) => {
                return ToolResult {
                    output: format!(
                        "Tool '{canonical}' was cancelled while waiting for the workspace lock"
                    ),
                    is_error: true,
                }
            }
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

async fn dispatch(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "agents.list" => list_agents(app).await,
        "agents.prompt" => prompt_agent(app, parse_params(params)?).await,
        "models.list" => list_models(app, parse_params(params)?).await,
        "tools.list" => list_tools(app).await,
        "tools.call" => call_tool(app, parse_params(params)?).await,
        "workspace.get" => workspace_snapshot(app).await,
        "sessions.list" => list_sdk_sessions(app, parse_params(params)?).await,
        "sessions.get" => get_sdk_session(app, parse_params(params)?),
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
        host_allowed, is_agent_only_tool, token_matches, validate_agent_id, ListModelsParams,
        CODEX_FALLBACK_MODELS, STATIC_MODELS,
    };

    #[test]
    fn validates_runtime_agent_ids() {
        assert_eq!(validate_agent_id("reviewer").unwrap(), "reviewer");
        assert_eq!(validate_agent_id("sdk-agent_2").unwrap(), "sdk-agent_2");
        assert!(validate_agent_id("Reviewer").is_err());
        assert!(validate_agent_id("doc").is_err());
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
    fn stateful_agent_tools_are_not_directly_callable() {
        assert!(is_agent_only_tool("subagent"));
        assert!(is_agent_only_tool("ask_user_question"));
        assert!(!is_agent_only_tool("read"));
        assert!(!is_agent_only_tool("config_query"));
        assert!(!is_agent_only_tool("unity_execute"));
    }
}
