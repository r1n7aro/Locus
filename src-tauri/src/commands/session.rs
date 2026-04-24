use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;

use futures::FutureExt;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, State};

use super::auth::CodexAuthStateHandle;
use super::{StreamEvent, TokenUsage};
use crate::agent::definition::{canonical_agent_id, is_hidden_legacy_agent_id, AgentDefRegistry};
use crate::agent::instance::{AgentInstance, AgentSystemPromptStats, LlmBackend, RawContextStore};
use crate::auth::AuthState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::knowledge_store::{self, KnowledgeDocument, KnowledgeInjectMode, KnowledgeType};
use crate::session::models::{
    ChatMessage, ImageData, KnowledgeProposalItem, KnowledgeProposalItemKind,
    KnowledgeProposalStatus, SessionDetail, SessionEventRecord, SessionRunSummary,
    SessionRuntimeStatus, SessionSummary, TodoItem, TodoSnapshot, UserIntentPayload,
};
use crate::session::store::SessionStore;
use crate::tool::ToolRegistry;
use crate::workspace::Workspace;
use crate::{ActiveTaskHandle, ActiveTasks, ApiKeyState, ProviderKeysState, QuestionStore};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub default_effort: Option<String>,
    pub model_recommendation: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLaunch {
    pub session_id: String,
    pub run_id: String,
}

fn emit_session_stream(app_handle: &AppHandle, store: &SessionStore, event: StreamEvent) {
    emit_session_stream_with_run_id(
        app_handle,
        store,
        format!(
            "knowledge_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis().to_string())
                .unwrap_or_else(|_| "0".to_string())
        ),
        event,
    );
}

fn emit_session_stream_with_run_id(
    app_handle: &AppHandle,
    store: &SessionStore,
    run_id: String,
    event: StreamEvent,
) {
    crate::session::gateway::emit_stream(app_handle, store, &run_id, event);
}

fn generate_chat_run_id(session_id: &str) -> String {
    format!(
        "{}_{}",
        session_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string())
    )
}

fn session_run_locked_error(detail: impl Into<String>) -> AppError {
    AppError::new(
        "session.run_locked",
        "Session already has an active run. Wait until the current run stops before sending another message.",
    )
    .detail(detail)
    .operation("chat")
    .retryable(true)
}

fn runtime_status_from_run_status(status: &str) -> SessionRuntimeStatus {
    match status {
        "queued" => SessionRuntimeStatus::Queued,
        "starting" => SessionRuntimeStatus::Starting,
        "waiting_input" => SessionRuntimeStatus::WaitingInput,
        "cancelling" => SessionRuntimeStatus::Cancelling,
        "error" => SessionRuntimeStatus::Error,
        _ => SessionRuntimeStatus::Running,
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn emit_knowledge_proposal_message(
    app_handle: &AppHandle,
    store: &SessionStore,
    session_id: &str,
    message: ChatMessage,
) {
    emit_session_stream(
        app_handle,
        store,
        StreamEvent::KnowledgeProposal {
            session_id: session_id.to_string(),
            message,
        },
    );
}

fn current_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn knowledge_title_from_path(path: &str) -> String {
    let candidate = Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    let mut parts = Vec::new();
    for segment in candidate
        .replace(['-', '_'], " ")
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
    {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            let mut word = first.to_uppercase().collect::<String>();
            word.push_str(chars.as_str());
            parts.push(word);
        }
    }
    if parts.is_empty() {
        "Untitled".to_string()
    } else {
        parts.join(" ")
    }
}

fn knowledge_default_inject_mode(doc_type: KnowledgeType) -> KnowledgeInjectMode {
    match doc_type {
        KnowledgeType::Design => KnowledgeInjectMode::Path,
        KnowledgeType::Memory => KnowledgeInjectMode::Full,
        KnowledgeType::Skill | KnowledgeType::Reference => KnowledgeInjectMode::None,
    }
}

fn knowledge_proposal_item_type(item: &KnowledgeProposalItem) -> KnowledgeType {
    knowledge_store::infer_type_from_path(&item.target).unwrap_or(match item.kind {
        KnowledgeProposalItemKind::Memory => KnowledgeType::Memory,
        KnowledgeProposalItemKind::Knowledge => KnowledgeType::Design,
    })
}

fn knowledge_proposal_target_path(path: &str) -> Result<String, String> {
    knowledge_store::ensure_document_path(path)
}

fn snapshot_knowledge_target(
    working_dir: &str,
    doc_type: KnowledgeType,
    target: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    let rel_path = knowledge_proposal_target_path(target)?;
    match knowledge_store::load_document_by_path(working_dir, doc_type, &rel_path) {
        Ok(doc) => Ok(Some(doc)),
        Err(err) if err.contains("not found") => Ok(None),
        Err(err) => Err(err),
    }
}

fn restore_knowledge_target(
    working_dir: &str,
    doc_type: KnowledgeType,
    backup: &Option<KnowledgeDocument>,
    target: &str,
) -> Result<(), String> {
    let rel_path = knowledge_proposal_target_path(target)?;
    match backup {
        Some(doc) => {
            knowledge_store::save_document(working_dir, doc.clone())?;
        }
        None => {
            let path = knowledge_store::document_path(working_dir, doc_type, &rel_path)?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(format!(
                        "Failed to remove knowledge document '{}': {}",
                        path.display(),
                        err
                    ));
                }
            }
        }
    }
    Ok(())
}

fn apply_knowledge_target(
    working_dir: &str,
    doc_type: KnowledgeType,
    target: &str,
    draft: &str,
) -> Result<KnowledgeDocument, String> {
    let rel_path = knowledge_proposal_target_path(target)?;
    match knowledge_store::load_document_by_path(working_dir, doc_type, &rel_path) {
        Ok(mut doc) => {
            doc.body = draft.to_string();
            doc.updated_at = current_unix_millis();
            knowledge_store::save_document(working_dir, doc)
        }
        Err(err) if err.contains("not found") => {
            let now = current_unix_millis();
            let doc = KnowledgeDocument {
                id: format!("kd_{}", uuid::Uuid::new_v4()),
                doc_type,
                path: rel_path,
                title: knowledge_title_from_path(target),
                scope: crate::knowledge_store::KnowledgeScope::Project,
                inject_mode: knowledge_default_inject_mode(doc_type),
                inherit_inject_mode: true,
                inject_mode_source: Default::default(),
                summary_enabled: crate::knowledge_store::default_summary_enabled_for_type(doc_type),
                command_enabled: false,
                read_only: false,
                ai_maintained: crate::knowledge_store::default_ai_maintained_for_type(doc_type),
                inherit_ai_config: true,
                ai_config_source: Default::default(),
                explicit_maintenance_rules:
                    crate::knowledge_store::default_explicit_maintenance_rules_for_type(doc_type),
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: None,
                body: draft.to_string(),
                maintenance_rules: None,
                created_at: now,
                updated_at: now,
            };
            knowledge_store::save_document(working_dir, doc)
        }
        Err(err) => Err(err),
    }
}

#[tauri::command]
pub async fn list_agents(
    registry: State<'_, Arc<AgentDefRegistry>>,
) -> Result<Vec<AgentInfo>, AppError> {
    let default_id = registry.default_id().to_string();
    let sub_agent_ids: std::collections::HashSet<&str> = registry
        .list_all()
        .iter()
        .flat_map(|def| def.sub_agents.iter().map(|s| s.as_str()))
        .collect();
    let mut agents: Vec<AgentInfo> = registry
        .list_all()
        .into_iter()
        .filter(|def| {
            !sub_agent_ids.contains(def.id.as_str()) && !is_hidden_legacy_agent_id(&def.id)
        })
        .map(|def| AgentInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            is_default: def.id == default_id,
            default_effort: def.default_effort.clone(),
            model_recommendation: def.model_recommendation.clone(),
            source: def.source.clone(),
        })
        .collect();
    agents.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    Ok(agents)
}

#[tauri::command]
pub async fn list_subagent_defs(
    registry: State<'_, Arc<AgentDefRegistry>>,
) -> Result<Vec<AgentInfo>, AppError> {
    let sub_agent_ids: std::collections::HashSet<String> = registry
        .list_all()
        .iter()
        .flat_map(|def| def.sub_agents.iter().cloned())
        .collect();
    let mut agents: Vec<AgentInfo> = registry
        .list_all()
        .into_iter()
        .filter(|def| sub_agent_ids.contains(&def.id))
        .map(|def| AgentInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            is_default: false,
            default_effort: def.default_effort.clone(),
            model_recommendation: def.model_recommendation.clone(),
            source: def.source.clone(),
        })
        .collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

#[tauri::command]
pub async fn get_agent_system_prompt(
    registry: State<'_, Arc<AgentDefRegistry>>,
    agent_id: String,
) -> Result<String, AppError> {
    match registry.get(&agent_id) {
        Some(def) => Ok(def.system_prompt.clone()),
        None => Err(format!("Agent '{}' not found", agent_id).into()),
    }
}

#[tauri::command]
pub async fn get_agent_env_template(
    registry: State<'_, Arc<AgentDefRegistry>>,
    agent_id: String,
) -> Result<String, AppError> {
    match registry.get(&agent_id) {
        Some(def) => Ok(def.env_template.clone()),
        None => Err(format!("Agent '{}' not found", agent_id).into()),
    }
}

#[tauri::command]
pub async fn get_agent_system_prompt_stats(
    agent_id: String,
    registry: State<'_, Arc<AgentDefRegistry>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    workspace: State<'_, Arc<Workspace>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<AgentSystemPromptStats, AppError> {
    let def = registry
        .get(&agent_id)
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = workspace.path.read().await.clone();
    let workspace_id = if working_dir.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };

    let instance = AgentInstance::new(
        Arc::new(def.clone()),
        "__agent-preview__",
        LlmBackend::AnthropicAgentSdk,
        false,
        registry.inner().clone(),
        tool_registry.inner().clone(),
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        "__agent-preview__".to_string(),
        None,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        None,
        HashMap::new(),
        tokio::sync::watch::channel(false).1,
    );

    Ok(instance.system_prompt_stats().await)
}

async fn resolve_model_backend(
    app_handle: &AppHandle,
    _def: &crate::agent::definition::AgentDef,
    selected_model: &str,
    config: &AppConfig,
    auth: &Arc<tokio::sync::Mutex<AuthState>>,
    api_key_state: &ApiKeyState,
    codex: &CodexAuthStateHandle,
) -> Result<LlmBackend, AppError> {
    let selected_model = selected_model.trim();
    if selected_model.is_empty() {
        return Err(
            "No model selected. Select a model before sending a message."
                .to_string()
                .into(),
        );
    }

    let is_custom = selected_model.starts_with("custom/");
    let is_openrouter = selected_model.starts_with("openrouter/");
    let is_anthropic_sdk = selected_model.starts_with("anthropic_sdk/");
    let is_openai_codex = selected_model.starts_with("openai/");
    let is_anthropic_direct = !selected_model.contains('/');

    if is_custom {
        let endpoint_id = selected_model.strip_prefix("custom/").unwrap_or("");
        let endpoints_path = crate::commands::workspace::custom_endpoints_path(app_handle)?;
        let endpoints: Vec<crate::commands::CustomEndpoint> =
            std::fs::read_to_string(&endpoints_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        let endpoint = endpoints
            .iter()
            .find(|item| item.id == endpoint_id)
            .ok_or_else(|| format!("Custom endpoint config not found: {}", endpoint_id))?;
        let endpoint_api_key =
            crate::keychain::get_secret(&crate::keychain::endpoint_key_name(&endpoint.id))
                .ok()
                .flatten()
                .unwrap_or_default();
        return Ok(LlmBackend::Custom {
            api_key: endpoint_api_key,
            api_model: endpoint.api_model.clone(),
            endpoint: endpoint.endpoint.clone(),
            api_format: endpoint.api_format.clone(),
            context_length: endpoint.context_length,
            beta_flags: endpoint.beta_flags.clone(),
        });
    }

    if is_openrouter {
        let api_key = api_key_state.read().await.clone();
        if api_key.is_empty() {
            return Err("OpenRouter API key not configured".to_string().into());
        }
        return Ok(LlmBackend::OpenRouter {
            api_key,
            base_url: config.base_url.clone(),
        });
    }

    if is_openai_codex {
        let mut codex_guard = codex.lock().await;
        return match codex_guard.access_token().await {
            Ok(_) => {
                let transport = crate::commands::load_codex_model_config()
                    .map(|config| config.transport)
                    .unwrap_or_default();
                Ok(LlmBackend::OpenAiCodex {
                    auth: codex.clone(),
                    transport,
                    base_url: config.base_url.clone(),
                })
            }
            Err(error) => {
                Err(format!("OpenAI Codex token failed (please re-login): {}", error).into())
            }
        };
    }

    if is_anthropic_sdk {
        return Ok(LlmBackend::AnthropicAgentSdk);
    }

    if is_anthropic_direct {
        let mut auth_guard = auth.lock().await;
        if !auth_guard.is_authenticated() {
            return Err("Not logged in to Anthropic, please log in from settings"
                .to_string()
                .into());
        }
        return match auth_guard.access_token().await {
            Ok(token) => {
                let user_metadata = auth_guard
                    .claude_code_user_metadata()
                    .map_err(|e| format!("Anthropic OAuth metadata failed: {}", e))?;
                Ok(LlmBackend::Anthropic {
                    access_token: token,
                    base_url: config.base_url.clone(),
                    user_metadata,
                })
            }
            Err(error) => Err(format!("Anthropic OAuth token failed: {}", error).into()),
        };
    }

    Err(format!(
        "Unrecognized model provider: {}. Use openrouter/, anthropic_sdk/, or openai/ prefix, or Anthropic direct format",
        selected_model
    )
    .into())
}

#[tauri::command]
pub async fn list_agent_injected_items(
    agent_id: String,
    registry: State<'_, Arc<AgentDefRegistry>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    workspace: State<'_, Arc<Workspace>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<Vec<crate::agent::instance::InjectedPromptItem>, AppError> {
    let def = registry
        .get(&agent_id)
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = workspace.path.read().await.clone();
    let workspace_id = if working_dir.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };

    let instance = AgentInstance::new(
        Arc::new(def.clone()),
        "__agent-preview__",
        LlmBackend::AnthropicAgentSdk,
        false,
        registry.inner().clone(),
        tool_registry.inner().clone(),
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        "__agent-preview__".to_string(),
        None,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        None,
        HashMap::new(),
        tokio::sync::watch::channel(false).1,
    );

    Ok(instance.list_injected_prompt_items().await)
}

#[tauri::command]
pub async fn create_session(
    title: String,
    parent_session_id: Option<String>,
    session_type: Option<String>,
    agent_id: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    let ws_id = if cwd.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };
    let trimmed = title.trim();
    let resolved_title = if trimmed.is_empty() {
        "New session"
    } else {
        trimmed
    };
    let resolved_agent_id = agent_id.as_deref().map(canonical_agent_id);
    store
        .create_session(
            resolved_title,
            parent_session_id.as_deref(),
            ws_id.as_deref(),
            session_type.as_deref().unwrap_or("chat"),
            resolved_agent_id,
        )
        .map_err(Into::into)
}

#[tauri::command]
pub async fn chat(
    session_id: Option<String>,
    text: String,
    session_title: Option<String>,
    agent_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    images: Option<Vec<ImageData>>,
    session_type: Option<String>,
    mode: Option<String>,
    user_intent: Option<UserIntentPayload>,
    subagent_models: Option<HashMap<String, String>>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<AgentDefRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
    api_key_state: State<'_, ApiKeyState>,
    _provider_keys: State<'_, ProviderKeysState>,
    codex: State<'_, CodexAuthStateHandle>,
    workspace: State<'_, Arc<Workspace>>,
    raw_store: State<'_, RawContextStore>,
    active_tasks: State<'_, ActiveTasks>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
    undo_manager: State<'_, crate::UndoManagerHandle>,
) -> Result<ChatLaunch, AppError> {
    let cwd = workspace.path.read().await.clone();
    let ws_id = if cwd.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };

    let requested_agent_id = agent_id
        .as_deref()
        .map(canonical_agent_id)
        .map(str::to_string);
    let sid = match session_id {
        Some(id) => id,
        None => {
            let title = session_title
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| text.chars().take(20).collect());
            store.create_session(
                &title,
                None,
                ws_id.as_deref(),
                session_type.as_deref().unwrap_or("chat"),
                requested_agent_id.as_deref(),
            )?
        }
    };

    let stale_messages = store.stale_pending_knowledge_proposals(&sid)?;
    for message in stale_messages {
        emit_knowledge_proposal_message(&app_handle, store.inner().as_ref(), &sid, message);
    }

    // Enforce session-agent binding: if session already has an agent, use it
    let effective_agent_id = match store.get_session_agent_id(&sid) {
        Ok(Some(stored)) => Some(canonical_agent_id(&stored).to_string()),
        _ => requested_agent_id.clone(),
    };

    let def = match &effective_agent_id {
        Some(id) => {
            let d = registry
                .get(id)
                .ok_or_else(|| format!("Unknown agent: {}", id))?;
            Arc::new(d.clone())
        }
        None => {
            let d = registry
                .default_def()
                .ok_or_else(|| "No agent definitions found".to_string())?;
            Arc::new(d.clone())
        }
    };

    let selected_model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "No model selected. Select a model before sending a message.".to_string())?
        .to_string();

    // - "openrouter/..." → OpenRouter
    // - "openai/..." → OpenAI Codex
    let is_custom = selected_model.starts_with("custom/");
    let is_openrouter = selected_model.starts_with("openrouter/");
    let is_anthropic_sdk = selected_model.starts_with("anthropic_sdk/");
    let is_openai_codex = selected_model.starts_with("openai/");
    let is_anthropic_direct = !selected_model.contains('/');

    let backend = if is_custom {
        let endpoint_id = selected_model.strip_prefix("custom/").unwrap_or("");
        let endpoints_path = crate::commands::workspace::custom_endpoints_path(&app_handle)?;
        let endpoints: Vec<crate::commands::CustomEndpoint> =
            std::fs::read_to_string(&endpoints_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        let ep = endpoints
            .iter()
            .find(|e| e.id == endpoint_id)
            .ok_or_else(|| format!("Custom endpoint config not found: {}", endpoint_id))?;
        // Load API key from keychain (JSON file no longer stores it)
        let ep_api_key = crate::keychain::get_secret(&crate::keychain::endpoint_key_name(&ep.id))
            .ok()
            .flatten()
            .unwrap_or_default();
        LlmBackend::Custom {
            api_key: ep_api_key,
            api_model: ep.api_model.clone(),
            endpoint: ep.endpoint.clone(),
            api_format: ep.api_format.clone(),
            context_length: ep.context_length,
            beta_flags: ep.beta_flags.clone(),
        }
    } else if is_openrouter {
        let api_key = api_key_state.read().await.clone();
        if !api_key.is_empty() {
            LlmBackend::OpenRouter {
                api_key,
                base_url: config.base_url.clone(),
            }
        } else {
            return Err("OpenRouter API key not configured".to_string().into());
        }
    } else if is_openai_codex {
        let mut codex_guard = codex.lock().await;
        match codex_guard.access_token().await {
            Ok(_) => {
                let transport = crate::commands::load_codex_model_config()
                    .map(|config| config.transport)
                    .unwrap_or_default();
                LlmBackend::OpenAiCodex {
                    auth: codex.inner().clone(),
                    transport,
                    base_url: config.base_url.clone(),
                }
            }
            Err(e) => {
                return Err(format!("OpenAI Codex token failed (please re-login): {}", e).into())
            }
        }
    } else if is_anthropic_sdk {
        LlmBackend::AnthropicAgentSdk
    } else if is_anthropic_direct {
        let mut auth_guard = auth.lock().await;
        if auth_guard.is_authenticated() {
            match auth_guard.access_token().await {
                Ok(token) => {
                    let user_metadata = auth_guard
                        .claude_code_user_metadata()
                        .map_err(|e| format!("Anthropic OAuth metadata failed: {}", e))?;
                    LlmBackend::Anthropic {
                        access_token: token,
                        base_url: config.base_url.clone(),
                        user_metadata,
                    }
                }
                Err(e) => {
                    return Err(format!("Anthropic OAuth token failed: {}", e).into());
                }
            }
        } else {
            return Err("Not logged in to Anthropic, please log in from settings"
                .to_string()
                .into());
        }
    } else {
        return Err(format!(
            "Unrecognized model provider: {}. Use openrouter/, anthropic_sdk/, or openai/ prefix, or Anthropic direct format",
            selected_model
        ).into());
    };
    let reg = registry.inner().clone();
    let tools = tool_registry.inner().clone();
    let raw = raw_store.inner().clone();

    let akd = app_knowledge_dir.0.clone();
    let aad = app_agent_dir.0.clone();
    let um = Some(undo_manager.inner().clone());
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let (done_tx, done_rx) = tokio::sync::watch::channel(false);
    let instance = AgentInstance::new(
        def,
        &sid,
        backend,
        config.debug_enabled(),
        reg,
        tools,
        cwd,
        raw,
        ws_id,
        selected_model,
        effort,
        akd,
        aad,
        um,
        subagent_models.unwrap_or_default(),
        cancel_rx,
    );
    let effective_mode = mode
        .or_else(|| user_intent.as_ref().map(|intent| intent.mode.clone()))
        .unwrap_or_else(|| "build".to_string());

    let handle = app_handle.clone();
    let run_id = generate_chat_run_id(&sid);
    if active_tasks.lock().await.contains_key(&sid) {
        return Err(session_run_locked_error(format!(
            "Session {} is already present in active task registry",
            sid
        )));
    }
    store.try_start_run(&sid, &run_id).map_err(|error| {
        if error.contains("active run") {
            session_run_locked_error(error)
        } else {
            AppError::new("session.run_start_failed", "Failed to start session run.")
                .detail(error)
                .operation("chat")
        }
    })?;
    let store = store.inner().clone();
    let sid_clone = sid.clone();
    let tasks = active_tasks.inner().clone();
    let sid_for_cleanup = sid.clone();
    let images_for_task = images.unwrap_or_default();
    let user_intent_for_task = user_intent;
    let run_id_for_task = run_id.clone();
    let store_for_task = store.clone();
    let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();

    let join_handle = tauri::async_runtime::spawn(async move {
        if start_rx.await.is_err() {
            eprintln!(
                "[Locus] session {} run {} start gate dropped before execution",
                sid_clone, run_id_for_task
            );
            return;
        }

        let task_result = AssertUnwindSafe(instance.run_with_run_id(
            &handle,
            &store_for_task,
            &text,
            if images_for_task.is_empty() {
                None
            } else {
                Some(&images_for_task)
            },
            &effective_mode,
            user_intent_for_task,
            run_id_for_task.clone(),
        ))
        .catch_unwind()
        .await;

        match task_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                eprintln!("[Locus] session {} failed: {}", sid_clone, e);
            }
            Err(panic_payload) => {
                let panic_message = panic_payload_to_string(panic_payload);
                eprintln!("[Locus] session {} panicked: {}", sid_clone, panic_message);
                emit_session_stream_with_run_id(
                    &handle,
                    store_for_task.as_ref(),
                    run_id_for_task.clone(),
                    StreamEvent::Error {
                        session_id: sid_clone.clone(),
                        error: AppError::new(
                            "chat.stream_failed",
                            format!("Session terminated unexpectedly: {}", panic_message),
                        ),
                    },
                );
            }
        }
        let removed = {
            let mut guard = tasks.lock().await;
            match guard.get(&sid_for_cleanup) {
                Some(task) if task.run_id == run_id_for_task => {
                    guard.remove(&sid_for_cleanup).is_some()
                }
                _ => false,
            }
        };
        eprintln!(
            "[Locus] active task cleared for session {} run {} removed={}",
            sid_for_cleanup, run_id_for_task, removed
        );
        let _ = done_tx.send(true);
    });

    {
        let mut task_guard = active_tasks.lock().await;
        if task_guard.contains_key(&sid) {
            join_handle.abort();
            let detail = format!(
                "Session {} became active before run {} was registered",
                sid, run_id
            );
            if let Err(error) = store.update_run_status(&run_id, "error", Some(&detail)) {
                eprintln!(
                    "[Locus] failed to mark unregistered session {} run {} as error: {}",
                    sid, run_id, error
                );
            }
            return Err(session_run_locked_error(format!("{}", detail)));
        }
        task_guard.insert(
            sid.clone(),
            ActiveTaskHandle {
                run_id: run_id.clone(),
                cancel_tx,
                done_rx,
                join_handle,
            },
        );
    }
    let _ = start_tx.send(());
    eprintln!(
        "[Locus] active task registered for session {} run {}",
        sid, run_id
    );

    Ok(ChatLaunch {
        session_id: sid,
        run_id,
    })
}

#[tauri::command]
pub async fn load_session(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<SessionDetail, AppError> {
    store.load_session(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn list_sessions(
    store: State<'_, Arc<SessionStore>>,
    workspace: State<'_, Arc<Workspace>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Vec<SessionSummary>, AppError> {
    let cwd = workspace.path.read().await.clone();
    let ws_id = if cwd.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };
    let mut sessions = store
        .list_sessions(ws_id.as_deref())
        .map_err(AppError::from)?;
    let active_session_ids: HashSet<String> = active_tasks.lock().await.keys().cloned().collect();
    for session in &mut sessions {
        session.runtime_status = if active_session_ids.contains(&session.id) {
            store
                .active_run_for_session(&session.id)
                .ok()
                .flatten()
                .map(|run| runtime_status_from_run_status(&run.status))
                .or(Some(SessionRuntimeStatus::Running))
        } else {
            None
        };
    }
    Ok(sessions)
}

#[tauri::command]
pub async fn list_archived_sessions(
    store: State<'_, Arc<SessionStore>>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<SessionSummary>, AppError> {
    let cwd = workspace.path.read().await.clone();
    let ws_id = if cwd.trim().is_empty() {
        None
    } else {
        workspace.workspace_id.read().await.clone()
    };
    store
        .list_archived_sessions(ws_id.as_deref())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_session(
    session_id: String,
    title: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    store
        .rename_session(&session_id, &title)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn archive_session(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    store.archive_session(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn unarchive_session(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    store.unarchive_session(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_session(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    store.delete_session(&session_id).map_err(AppError::from)?;
    crate::llm::codex::invalidate_cached_session(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn get_session_usage(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<TokenUsage, AppError> {
    store.get_token_usage(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_active_run(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Option<SessionRunSummary>, AppError> {
    store
        .active_run_for_session(&session_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_session_events(
    session_id: String,
    after_seq: Option<i64>,
    limit: Option<u32>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<SessionEventRecord>, AppError> {
    store
        .list_session_events(&session_id, after_seq, limit)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_todos(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<TodoSnapshot, AppError> {
    store.get_todos(&session_id).map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_chat(
    session_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<(), AppError> {
    let graceful_wait = {
        let tasks = active_tasks.lock().await;
        tasks.get(&session_id).map(|task| {
            let _ = task.cancel_tx.send(true);
            (task.run_id.clone(), task.done_rx.clone())
        })
    };

    let Some((run_id, mut done_rx)) = graceful_wait else {
        return Ok(());
    };

    if *done_rx.borrow() {
        return Ok(());
    }

    if let Err(error) = store.update_run_status(
        &run_id,
        crate::session::gateway::RUN_STATUS_CANCELLING,
        None,
    ) {
        eprintln!(
            "[Locus] failed to mark session {} run {} as cancelling: {}",
            session_id, run_id, error
        );
    }

    let graceful_finished =
        match tokio::time::timeout(std::time::Duration::from_millis(1500), done_rx.changed()).await
        {
            Ok(Ok(())) => true,
            Ok(Err(_)) => true,
            Err(_) => false,
        };

    if graceful_finished {
        eprintln!(
            "[Locus] cancellation finished gracefully for session {}",
            session_id
        );
        return Ok(());
    }

    let handle = active_tasks.lock().await.remove(&session_id);
    if let Some(task) = handle {
        task.join_handle.abort();
        eprintln!(
            "[Locus] cancellation timed out; aborted task for session {}",
            session_id
        );
        crate::llm::codex::reset_cached_session_window(&session_id).await;
        emit_session_stream_with_run_id(
            &app_handle,
            store.inner().as_ref(),
            run_id,
            StreamEvent::Cancelled { session_id },
        );
    }

    Ok(())
}

#[tauri::command]
pub async fn stale_knowledge_proposals(
    session_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    let updated = store.stale_pending_knowledge_proposals(&session_id)?;
    for message in updated {
        emit_knowledge_proposal_message(&app_handle, store.inner().as_ref(), &session_id, message);
    }
    Ok(())
}

#[tauri::command]
pub async fn ignore_knowledge_proposal(
    session_id: String,
    proposal_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    let updated = store.update_knowledge_proposal_status(
        &session_id,
        &proposal_id,
        KnowledgeProposalStatus::Invalidated,
    )?;
    if let Some(message) = updated {
        emit_knowledge_proposal_message(&app_handle, store.inner().as_ref(), &session_id, message);
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_knowledge_proposal(
    session_id: String,
    proposal_id: String,
    _verification_confirmed: Option<bool>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>>,
) -> Result<(), AppError> {
    let Some(message) = store.get_knowledge_proposal_message(&session_id, &proposal_id)? else {
        return Err(format!("Knowledge proposal not found: {}", proposal_id).into());
    };
    let Some(proposal) = message.knowledge_proposal.clone() else {
        return Err(format!(
            "Message does not contain a knowledge proposal: {}",
            proposal_id
        )
        .into());
    };
    if proposal.status != KnowledgeProposalStatus::Pending {
        return Err(format!(
            "Knowledge proposal '{}' is not pending (current status: {:?})",
            proposal_id, proposal.status
        )
        .into());
    }

    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Err("No working directory selected.".into());
    }

    let current_workspace_id = workspace.workspace_id.read().await.clone();
    let session_workspace_id = store.get_session_workspace_id(&session_id)?;
    match (
        session_workspace_id.as_deref(),
        current_workspace_id.as_deref(),
    ) {
        (Some(expected), Some(current)) if expected == current => {}
        (Some(_), Some(_)) => return Err(
            "Current workspace does not match the session that created this knowledge proposal."
                .into(),
        ),
        _ => return Err(
            "Knowledge proposals can only be applied while the original workspace is still open."
                .into(),
        ),
    }

    let mut proposal_targets: Vec<(KnowledgeType, String)> = Vec::new();
    let mut seen_targets = HashSet::new();

    for item in &proposal.items {
        let doc_type = knowledge_proposal_item_type(item);
        let rel_path = knowledge_proposal_target_path(&item.target)?;
        let dedupe_key = format!("{}/{}", doc_type.as_str(), rel_path);
        if !seen_targets.insert(dedupe_key) {
            return Err(format!("Duplicate knowledge proposal target: {}", item.target).into());
        }
        proposal_targets.push((doc_type, item.target.clone()));
    }

    let mut knowledge_backups = HashMap::new();
    for (doc_type, target) in &proposal_targets {
        let backup = snapshot_knowledge_target(&working_dir, *doc_type, target)?;
        knowledge_backups.insert(target.clone(), backup);
    }

    if let Some(applying_message) = store.update_knowledge_proposal_status(
        &session_id,
        &proposal_id,
        KnowledgeProposalStatus::Applying,
    )? {
        emit_knowledge_proposal_message(
            &app_handle,
            store.inner().as_ref(),
            &session_id,
            applying_message,
        );
    }

    let mut apply_error: Option<String> = None;

    for item in &proposal.items {
        let doc_type = knowledge_proposal_item_type(item);
        if !knowledge_backups.contains_key(&item.target) {
            apply_error = Some(format!("Missing knowledge backup for {}", item.target));
            break;
        }
        if let Err(err) = apply_knowledge_target(&working_dir, doc_type, &item.target, &item.draft)
        {
            apply_error = Some(err);
            break;
        }
    }

    if apply_error.is_none() {
        if let Err(error) = super::knowledge::reconcile_and_emit_knowledge_changed(
            &app_handle,
            &working_dir,
            knowledge_index_state.inner().clone(),
            "apply_knowledge_proposal",
        )
        .await
        {
            apply_error = Some(format!("Failed to reconcile knowledge index: {}", error));
        }
    }

    match apply_error {
        None => {
            if let Some(message) = store.update_knowledge_proposal_status(
                &session_id,
                &proposal_id,
                KnowledgeProposalStatus::Applied,
            )? {
                emit_knowledge_proposal_message(
                    &app_handle,
                    store.inner().as_ref(),
                    &session_id,
                    message,
                );
            }
            Ok(())
        }
        Some(error) => {
            let mut rollback_errors = Vec::new();
            for (doc_type, target) in proposal_targets.iter().rev() {
                let backup = knowledge_backups.get(target).cloned().unwrap_or(None);
                if let Err(rollback_error) =
                    restore_knowledge_target(&working_dir, *doc_type, &backup, target)
                {
                    rollback_errors.push(format!(
                        "knowledge rollback failed for {}: {}",
                        target, rollback_error
                    ));
                }
            }

            let next_status = if rollback_errors.is_empty() {
                KnowledgeProposalStatus::Pending
            } else {
                KnowledgeProposalStatus::Invalidated
            };
            if let Some(message) =
                store.update_knowledge_proposal_status(&session_id, &proposal_id, next_status)?
            {
                emit_knowledge_proposal_message(
                    &app_handle,
                    store.inner().as_ref(),
                    &session_id,
                    message,
                );
            }
            if rollback_errors.is_empty() {
                Err(error.into())
            } else {
                Err(format!("{}; rollback failed: {}", error, rollback_errors.join("; ")).into())
            }
        }
    }
}

#[tauri::command]
pub async fn save_raw_context(
    session_id: String,
    file_path: String,
    include_system_prompt: bool,
    raw_store: State<'_, RawContextStore>,
    store: State<'_, Arc<SessionStore>>,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, Arc<AgentDefRegistry>>,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let project_config = load_export_project_config(&working_dir);
    let usage = store.get_token_usage(&session_id).ok();
    let raw_markdown = {
        let raw = raw_store.lock().await;
        raw.get(&session_id)
            .filter(|rounds| !rounds.is_empty())
            .map(|rounds| {
                format_rounds_as_markdown(
                    &session_id,
                    rounds,
                    usage.as_ref(),
                    project_config.as_ref(),
                    include_system_prompt,
                )
            })
    };

    let (markdown, export_mode) = if let Some(markdown) = raw_markdown {
        (markdown, "raw-rounds")
    } else {
        let detail = store.load_session(&session_id)?;
        let todos = store
            .get_todos(&session_id)
            .map(|snapshot| snapshot.items)
            .unwrap_or_default();
        let system_prompt = if include_system_prompt {
            resolve_export_system_prompt(registry.inner(), detail.agent_id.as_deref())
        } else {
            None
        };
        (
            format_session_detail_as_markdown(
                &detail,
                &todos,
                usage.as_ref(),
                project_config.as_ref(),
                include_system_prompt,
                system_prompt.as_deref(),
            ),
            "session-store-fallback",
        )
    };

    std::fs::write(&file_path, markdown.as_bytes())
        .map_err(|e| format!("Failed to write file: {}", e))?;

    eprintln!(
        "[Locus] saved context export ({}, system_prompt={}) for session {} to {}",
        export_mode, include_system_prompt, session_id, file_path
    );
    Ok(file_path)
}

#[derive(Debug, Clone)]
struct ExportProjectConfig {
    working_dir: String,
    knowledge_enabled: bool,
    full_text_search_enabled: bool,
    semantic_search_enabled: bool,
}

#[derive(Debug, Clone)]
struct ExportEnabledTool {
    name: String,
    description: String,
}

fn load_export_project_config(working_dir: &str) -> Option<ExportProjectConfig> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return None;
    }

    let knowledge_root = knowledge_store::knowledge_root(trimmed);
    let knowledge_enabled = knowledge_root.is_dir()
        && std::fs::read_dir(&knowledge_root)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().is_dir())
                    .then_some(())
            })
            .is_some();

    Some(ExportProjectConfig {
        working_dir: trimmed.to_string(),
        knowledge_enabled,
        full_text_search_enabled: knowledge_enabled,
        semantic_search_enabled: knowledge_enabled,
    })
}

fn format_enabled_state(enabled: bool) -> &'static str {
    if enabled {
        "Enabled"
    } else {
        "Disabled"
    }
}

const EMPTY_EXPORT_FIELD: &str = "empty";

fn append_project_config_markdown(out: &mut String, project_config: Option<&ExportProjectConfig>) {
    out.push_str("## Current Project Configuration\n\n");
    if let Some(config) = project_config {
        out.push_str(&format!("- **Workspace:** `{}`\n", config.working_dir));
        out.push_str(&format!(
            "- **Knowledge:** {}\n",
            format_enabled_state(config.knowledge_enabled)
        ));
        out.push_str(&format!(
            "- **Full-text Search:** {}\n",
            format_enabled_state(config.full_text_search_enabled)
        ));
        out.push_str(&format!(
            "- **Semantic Search:** {}\n",
            format_enabled_state(config.semantic_search_enabled)
        ));
    } else {
        out.push_str("- Project configuration unavailable: no workspace is currently selected.\n");
    }
    out.push_str("\n---\n\n");
}

fn extract_enabled_tools(rounds: &[crate::agent::instance::RawRound]) -> Vec<ExportEnabledTool> {
    let Some(tool_values) = rounds.iter().rev().find_map(|round| {
        round
            .request
            .get("tools")
            .and_then(|value| value.as_array())
    }) else {
        return Vec::new();
    };

    tool_values
        .iter()
        .filter_map(parse_export_enabled_tool)
        .collect()
}

fn parse_export_enabled_tool(value: &serde_json::Value) -> Option<ExportEnabledTool> {
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

    Some(ExportEnabledTool {
        name: name.to_string(),
        description,
    })
}

fn append_enabled_tools_markdown(out: &mut String, tools: &[ExportEnabledTool]) {
    out.push_str("## Enabled Tools\n\n");
    if tools.is_empty() {
        out.push_str("- No tools were enabled in the latest captured request.\n");
        out.push_str("\n---\n\n");
        return;
    }

    out.push_str(&format!("- **Count:** {}\n\n", tools.len()));
    for tool in tools {
        out.push_str(&format!("### `{}`\n\n", tool.name));
        if tool.description.is_empty() {
            out.push_str("*(No description provided)*\n\n");
        } else {
            out.push_str(&tool.description);
            out.push_str("\n\n");
        }
    }
    out.push_str("---\n\n");
}

fn format_export_timestamp(ts: i64) -> String {
    use chrono::{Local, TimeZone};

    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn export_optional_text(value: Option<&str>) -> serde_json::Value {
    let trimmed = value.unwrap_or("").trim();
    if trimmed.is_empty() {
        json!(EMPTY_EXPORT_FIELD)
    } else {
        json!(trimmed)
    }
}

fn export_optional_u32(value: Option<u32>) -> serde_json::Value {
    match value {
        Some(value) => json!(value),
        None => json!(EMPTY_EXPORT_FIELD),
    }
}

fn export_optional_tool_outcome(
    value: Option<crate::commands::ToolCallOutcome>,
) -> serde_json::Value {
    match value {
        Some(value) => json!(value),
        None => json!(EMPTY_EXPORT_FIELD),
    }
}

fn export_optional_server_tool(
    value: Option<&crate::session::models::ServerToolKind>,
) -> serde_json::Value {
    match value {
        Some(value) => json!(value),
        None => json!(EMPTY_EXPORT_FIELD),
    }
}

fn export_tool_call(tool_call: &crate::session::models::ToolCallInfo) -> serde_json::Value {
    json!({
        "id": tool_call.id,
        "name": tool_call.name,
        "arguments": tool_call.arguments,
        "serverTool": export_optional_server_tool(tool_call.server_tool.as_ref()),
        "serverToolOutput": export_optional_text(tool_call.server_tool_output.as_deref()),
        "outcome": export_optional_tool_outcome(tool_call.outcome),
        "recordedOutput": export_optional_text(tool_call.recorded_output.as_deref()),
        "nestedToolCalls": export_tool_calls(tool_call.nested_tool_calls.as_deref()),
    })
}

fn export_tool_calls(
    tool_calls: Option<&[crate::session::models::ToolCallInfo]>,
) -> serde_json::Value {
    match tool_calls {
        Some(tool_calls) if !tool_calls.is_empty() => {
            json!(tool_calls.iter().map(export_tool_call).collect::<Vec<_>>())
        }
        _ => json!(EMPTY_EXPORT_FIELD),
    }
}

fn export_images(images: Option<&[ImageData]>) -> serde_json::Value {
    match images {
        Some(images) if !images.is_empty() => json!(images
            .iter()
            .map(|image| json!({
                "mimeType": image.mime_type,
                "dataLength": image.data.len(),
            }))
            .collect::<Vec<_>>()),
        _ => json!(EMPTY_EXPORT_FIELD),
    }
}

fn append_json_block(out: &mut String, title: &str, value: &serde_json::Value, level: usize) {
    let heading = "#".repeat(level.clamp(1, 6));
    out.push_str(&format!("{} {}\n\n```json\n", heading, title));
    match serde_json::to_string_pretty(value) {
        Ok(text) => out.push_str(&text),
        Err(_) => out.push_str("{\"error\":\"failed to serialize export block\"}"),
    }
    out.push_str("\n```\n\n");
}

fn append_text_block(out: &mut String, title: &str, value: Option<&str>, level: usize) {
    let heading = "#".repeat(level.clamp(1, 6));
    out.push_str(&format!("{} {}\n\n", heading, title));

    let raw = value.unwrap_or("");
    if raw.trim().is_empty() {
        out.push_str("`empty`\n\n");
        return;
    }

    out.push_str("```text\n");
    out.push_str(raw);
    if !raw.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");
}

fn append_system_prompt_block(out: &mut String, system_prompt: Option<&str>, level: usize) {
    let heading = "#".repeat(level.clamp(1, 6));
    out.push_str(&format!("{} System Prompt\n\n", heading));

    match system_prompt.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            out.push_str(text);
            out.push_str("\n\n");
        }
        None => out.push_str("`empty`\n\n"),
    }
}

fn resolve_export_system_prompt(
    registry: &Arc<AgentDefRegistry>,
    agent_id: Option<&str>,
) -> Option<String> {
    let canonical_id = canonical_agent_id(agent_id?);
    let prompt = registry.get(canonical_id)?.system_prompt.trim().to_string();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt)
    }
}

fn format_session_detail_as_markdown(
    detail: &SessionDetail,
    todos: &[TodoItem],
    usage: Option<&TokenUsage>,
    project_config: Option<&ExportProjectConfig>,
    include_system_prompt: bool,
    system_prompt: Option<&str>,
) -> String {
    let mut out = String::with_capacity(16 * 1024);

    out.push_str("# Locus Conversation Log\n\n");
    out.push_str(&format!("- **Session:** `{}`\n", detail.id));
    out.push_str("- **Export Source:** `session-store-fallback`\n");
    out.push_str("- **Raw Rounds:** `empty`\n");
    out.push_str(&format!("- **Messages:** {}\n", detail.messages.len()));
    out.push_str(&format!(
        "- **Missing Field Marker:** `{}`\n\n",
        EMPTY_EXPORT_FIELD
    ));
    out.push_str(
        "## Export Note\n\nRaw request/response rounds were unavailable in memory for this session. \
This export was reconstructed from the persisted session store. Any field unavailable after \
migration is written as `empty`.\n\n",
    );
    if include_system_prompt {
        out.push_str(
            "System Prompt reflects the current agent definition for this session when available.\n\n",
        );
    }
    out.push_str("---\n\n");

    append_project_config_markdown(&mut out, project_config);
    if include_system_prompt {
        append_system_prompt_block(&mut out, system_prompt, 2);
        out.push_str("---\n\n");
    }

    let session_metadata = json!({
        "sessionId": detail.id,
        "title": export_optional_text(Some(&detail.title)),
        "agentId": export_optional_text(detail.agent_id.as_deref()),
        "sessionType": export_optional_text(Some(&detail.session_type)),
        "parentSessionId": export_optional_text(detail.parent_session_id.as_deref()),
        "latestCompletedRunId": export_optional_text(detail.latest_completed_run_id.as_deref()),
        "createdAtUnix": detail.created_at,
        "createdAtLocal": format_export_timestamp(detail.created_at),
        "updatedAtUnix": detail.updated_at,
        "updatedAtLocal": format_export_timestamp(detail.updated_at),
    });
    append_json_block(&mut out, "Session Metadata", &session_metadata, 2);

    let usage_json = match usage {
        Some(usage) => json!({
            "totalInputTokens": usage.total_input_tokens,
            "totalOutputTokens": usage.total_output_tokens,
            "totalCacheReadTokens": usage.total_cache_read_tokens,
            "totalCacheWriteTokens": usage.total_cache_write_tokens,
            "totalCostUsd": usage.total_cost_usd,
            "pricedRounds": usage.priced_rounds,
        }),
        None => json!({
            "totalInputTokens": EMPTY_EXPORT_FIELD,
            "totalOutputTokens": EMPTY_EXPORT_FIELD,
            "totalCacheReadTokens": EMPTY_EXPORT_FIELD,
            "totalCacheWriteTokens": EMPTY_EXPORT_FIELD,
            "totalCostUsd": EMPTY_EXPORT_FIELD,
            "pricedRounds": EMPTY_EXPORT_FIELD,
        }),
    };
    append_json_block(&mut out, "Token Usage", &usage_json, 2);

    let todos_json = if todos.is_empty() {
        json!(EMPTY_EXPORT_FIELD)
    } else {
        json!(todos)
    };
    append_json_block(&mut out, "Todos", &todos_json, 2);

    out.push_str("## Messages\n\n");
    if detail.messages.is_empty() {
        out.push_str("`empty`\n\n");
        return out;
    }

    for (index, message) in detail.messages.iter().enumerate() {
        let metadata = json!({
            "messageIndex": index + 1,
            "id": message.id,
            "role": message.role,
            "createdAtUnix": message.created_at,
            "createdAtLocal": format_export_timestamp(message.created_at),
            "promptPrefix": export_optional_text(message.prompt_prefix.as_deref()),
            "promptSuffix": export_optional_text(message.prompt_suffix.as_deref()),
            "responseId": export_optional_text(message.response_id.as_deref()),
            "toolCalls": export_tool_calls(message.tool_calls.as_deref()),
            "toolCallId": export_optional_text(message.tool_call_id.as_deref()),
            "images": export_images(message.images.as_deref()),
            "thinkingContent": export_optional_text(message.thinking_content.as_deref()),
            "thinkingDuration": export_optional_u32(message.thinking_duration),
            "thinkingSignature": export_optional_text(message.thinking_signature.as_deref()),
            "knowledgeProposal": message
                .knowledge_proposal
                .as_ref()
                .map(|proposal| json!(proposal))
                .unwrap_or_else(|| json!(EMPTY_EXPORT_FIELD)),
        });

        append_json_block(&mut out, &format!("Message {}", index + 1), &metadata, 3);
        append_text_block(&mut out, "Content", Some(&message.content), 4);
        out.push_str("---\n\n");
    }

    out
}

fn format_rounds_as_markdown(
    session_id: &str,
    rounds: &[crate::agent::instance::RawRound],
    usage: Option<&TokenUsage>,
    project_config: Option<&ExportProjectConfig>,
    include_system_prompt: bool,
) -> String {
    let mut out = String::with_capacity(16 * 1024);

    out.push_str("# Locus Conversation Log\n\n");
    out.push_str(&format!("- **Session:** `{}`\n", session_id));
    out.push_str("- **Export Source:** `raw-rounds`\n");
    out.push_str(&format!("- **Rounds:** {}\n", rounds.len()));
    let model = rounds
        .first()
        .and_then(|first| first.request.get("model"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(EMPTY_EXPORT_FIELD);
    out.push_str(&format!("- **Model:** `{}`\n", model));
    if let Some(u) = usage {
        out.push_str(&format!(
            "- **Total Tokens:** {} input / {} output / {} cache read / {} cache write\n",
            u.total_input_tokens,
            u.total_output_tokens,
            u.total_cache_read_tokens,
            u.total_cache_write_tokens
        ));
        out.push_str(&format!("- **Total Cost:** ${:.4}\n", u.total_cost_usd));
    } else {
        out.push_str(&format!("- **Total Tokens:** `{}`\n", EMPTY_EXPORT_FIELD));
        out.push_str(&format!("- **Total Cost:** `{}`\n", EMPTY_EXPORT_FIELD));
    }
    out.push_str("\n\n");
    append_project_config_markdown(&mut out, project_config);
    let enabled_tools = extract_enabled_tools(rounds);
    append_enabled_tools_markdown(&mut out, &enabled_tools);

    if include_system_prompt {
        if let Some(first) = rounds.first() {
            out.push_str("## System Prompt\n\n");
            if !write_system_prompt_markdown(&mut out, &first.request) {
                out.push_str("`empty`\n\n");
            }
            out.push_str("---\n\n");
        }
    }

    let mut prev_msg_count: usize = 0;

    for round in rounds {
        let time_str = format_export_timestamp(round.timestamp);
        out.push_str(&format!("## Round {} ({})\n\n", round.round, time_str));

        if let Some(messages) = extract_request_history_items(&round.request) {
            let new_messages = if prev_msg_count < messages.len() {
                &messages[prev_msg_count..]
            } else {
                &messages[..]
            };
            prev_msg_count = messages.len();

            format_request_history_items(&mut out, new_messages);
        }

        out.push_str("### 🤖 Assistant\n\n");
        parse_sse_response(&mut out, &round.response);
        out.push_str("\n---\n\n");
    }

    out
}

fn extract_request_history_items(request: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    request
        .get("messages")
        .and_then(|value| value.as_array())
        .or_else(|| request.get("input").and_then(|value| value.as_array()))
}

#[derive(Clone, Copy)]
struct ExportRequestToolCall<'a> {
    name: Option<&'a str>,
    call_id: Option<&'a str>,
    arguments: Option<&'a serde_json::Value>,
}

impl<'a> ExportRequestToolCall<'a> {
    fn from_item(item: &'a serde_json::Value) -> Self {
        Self {
            name: item.get("name").and_then(|value| value.as_str()),
            call_id: item
                .get("call_id")
                .and_then(|value| value.as_str())
                .or_else(|| item.get("id").and_then(|value| value.as_str())),
            arguments: item.get("arguments"),
        }
    }
}

#[derive(Clone, Copy)]
struct ExportRequestToolOutput<'a> {
    call_id: Option<&'a str>,
    output: Option<&'a serde_json::Value>,
}

impl<'a> ExportRequestToolOutput<'a> {
    fn from_item(item: &'a serde_json::Value) -> Self {
        Self {
            call_id: item
                .get("call_id")
                .and_then(|value| value.as_str())
                .or_else(|| item.get("tool_use_id").and_then(|value| value.as_str())),
            output: item.get("output").or_else(|| item.get("content")),
        }
    }
}

fn format_request_history_items(out: &mut String, items: &[serde_json::Value]) {
    let mut index = 0usize;
    while index < items.len() {
        if is_request_function_call_item(&items[index]) {
            index = format_request_tool_call_batch(out, items, index);
            continue;
        }
        format_request_history_item(out, &items[index]);
        index += 1;
    }
}

fn is_request_function_call_item(item: &serde_json::Value) -> bool {
    item.get("type").and_then(|value| value.as_str()) == Some("function_call")
}

fn is_request_function_call_output_item(item: &serde_json::Value) -> bool {
    item.get("type").and_then(|value| value.as_str()) == Some("function_call_output")
}

fn format_request_tool_call_batch(
    out: &mut String,
    items: &[serde_json::Value],
    start_index: usize,
) -> usize {
    let mut index = start_index;
    let mut tool_calls: Vec<ExportRequestToolCall<'_>> = Vec::new();
    while index < items.len() && is_request_function_call_item(&items[index]) {
        tool_calls.push(ExportRequestToolCall::from_item(&items[index]));
        index += 1;
    }

    let mut pending_outputs: Vec<ExportRequestToolOutput<'_>> = Vec::new();
    while index < items.len() && is_request_function_call_output_item(&items[index]) {
        pending_outputs.push(ExportRequestToolOutput::from_item(&items[index]));
        index += 1;
    }

    for tool_call in tool_calls {
        format_assistant_tool_call_message(out, tool_call.name, tool_call.arguments);

        let Some(call_id) = tool_call.call_id.filter(|value| !value.is_empty()) else {
            continue;
        };

        let mut remaining_outputs = Vec::with_capacity(pending_outputs.len());
        for tool_output in pending_outputs {
            if tool_output.call_id == Some(call_id) {
                format_tool_output_message(out, tool_output.call_id, tool_output.output);
            } else {
                remaining_outputs.push(tool_output);
            }
        }
        pending_outputs = remaining_outputs;
    }

    for tool_output in pending_outputs {
        format_tool_output_message(out, tool_output.call_id, tool_output.output);
    }

    index
}

fn write_system_prompt_markdown(out: &mut String, request: &serde_json::Value) -> bool {
    write_text_blocks_markdown(out, request.get("system"))
        || write_text_blocks_markdown(out, request.get("instructions"))
}

fn write_text_blocks_markdown(out: &mut String, value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::String(text)) if !text.is_empty() => {
            out.push_str(text);
            out.push_str("\n\n");
            true
        }
        Some(serde_json::Value::Array(items)) if !items.is_empty() => {
            let mut wrote_any = false;
            for item in items {
                if let Some(text) = extract_text_block(item) {
                    if !text.is_empty() {
                        out.push_str(text);
                        out.push_str("\n\n");
                        wrote_any = true;
                    }
                }
            }
            wrote_any
        }
        _ => false,
    }
}

fn extract_text_block(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::String(text) => Some(text.as_str()),
        serde_json::Value::Object(_) => value.get("text").and_then(|inner| inner.as_str()),
        _ => None,
    }
}

fn format_request_history_item(out: &mut String, item: &serde_json::Value) {
    if let Some(role) = item.get("role").and_then(|value| value.as_str()) {
        match role {
            "user" => format_user_message(out, item.get("content")),
            "assistant" => format_assistant_from_request(out, item.get("content")),
            _ => {}
        }
        return;
    }

    match item.get("type").and_then(|value| value.as_str()) {
        Some("function_call") => format_assistant_tool_call_message(
            out,
            item.get("name").and_then(|value| value.as_str()),
            item.get("arguments"),
        ),
        Some("function_call_output") => format_tool_output_message(
            out,
            item.get("call_id").and_then(|value| value.as_str()),
            item.get("output"),
        ),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_enabled_tools_markdown, append_project_config_markdown, extract_enabled_tools,
        format_rounds_as_markdown, format_session_detail_as_markdown, parse_sse_response,
        ExportEnabledTool, ExportProjectConfig, EMPTY_EXPORT_FIELD,
    };
    use crate::session::models::{ChatMessage, MessageRole, SessionDetail, ToolCallInfo};
    use crate::session::store::SessionStore;
    use rusqlite::{params, Connection};
    use tempfile::tempdir;

    #[test]
    fn project_config_section_includes_workspace_flags() {
        let mut out = String::new();
        append_project_config_markdown(
            &mut out,
            Some(&ExportProjectConfig {
                working_dir: "F:/Proj".to_string(),
                knowledge_enabled: true,
                full_text_search_enabled: false,
                semantic_search_enabled: true,
            }),
        );

        assert!(out.contains("## Current Project Configuration"));
        assert!(out.contains("- **Workspace:** `F:/Proj`"));
        assert!(out.contains("- **Knowledge:** Enabled"));
        assert!(out.contains("- **Full-text Search:** Disabled"));
        assert!(out.contains("- **Semantic Search:** Enabled"));
    }

    #[test]
    fn project_config_section_reports_missing_workspace() {
        let mut out = String::new();
        append_project_config_markdown(&mut out, None);

        assert!(out.contains("Project configuration unavailable"));
    }

    #[test]
    fn extract_enabled_tools_supports_openai_and_anthropic_shapes() {
        let rounds = vec![crate::agent::instance::RawRound {
            round: 1,
            timestamp: 0,
            request: serde_json::json!({
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "read",
                            "description": "Read a file from disk"
                        }
                    },
                    {
                        "name": "bash",
                        "description": "Run a shell command"
                    }
                ]
            }),
            response: String::new(),
        }];

        let tools = extract_enabled_tools(&rounds);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read");
        assert_eq!(tools[0].description, "Read a file from disk");
        assert_eq!(tools[1].name, "bash");
        assert_eq!(tools[1].description, "Run a shell command");
    }

    #[test]
    fn enabled_tools_markdown_lists_names_and_descriptions() {
        let mut out = String::new();
        append_enabled_tools_markdown(
            &mut out,
            &[
                ExportEnabledTool {
                    name: "read".to_string(),
                    description: "Read a file from disk".to_string(),
                },
                ExportEnabledTool {
                    name: "bash".to_string(),
                    description: "Run a shell command".to_string(),
                },
            ],
        );

        assert!(out.contains("## Enabled Tools"));
        assert!(out.contains("- **Count:** 2"));
        assert!(out.contains("### `read`"));
        assert!(out.contains("Read a file from disk"));
        assert!(out.contains("### `bash`"));
        assert!(out.contains("Run a shell command"));
    }

    #[test]
    fn raw_context_export_marks_missing_model_and_system_as_empty() {
        let markdown = format_rounds_as_markdown(
            "session-1",
            &[crate::agent::instance::RawRound {
                round: 1,
                timestamp: 0,
                request: serde_json::json!({
                    "messages": []
                }),
                response: String::new(),
            }],
            None,
            None,
            true,
        );

        assert!(markdown.contains("- **Model:** `empty`"));
        assert!(markdown.contains("- **Total Tokens:** `empty`"));
        assert!(markdown.contains("## System Prompt"));
        assert!(markdown.contains("`empty`"));
    }

    #[test]
    fn raw_context_export_omits_system_prompt_section_when_disabled() {
        let markdown = format_rounds_as_markdown(
            "session-1",
            &[crate::agent::instance::RawRound {
                round: 1,
                timestamp: 0,
                request: serde_json::json!({
                    "instructions": "You are a helpful assistant.",
                    "messages": [
                        {
                            "role": "user",
                            "content": "hello"
                        }
                    ]
                }),
                response: String::new(),
            }],
            None,
            None,
            false,
        );

        assert!(!markdown.contains("## System Prompt"));
        assert!(!markdown.contains("You are a helpful assistant."));
        assert!(markdown.contains("### 👤 User"));
    }

    #[test]
    fn raw_context_export_supports_codex_request_and_response_shapes() {
        let markdown = format_rounds_as_markdown(
            "session-codex",
            &[crate::agent::instance::RawRound {
                round: 1,
                timestamp: 0,
                request: serde_json::json!({
                    "model": "gpt-5.3-codex-spark",
                    "instructions": "You are a helpful assistant.",
                    "input": [
                        {
                            "role": "user",
                            "content": [
                                {
                                    "type": "input_text",
                                    "text": "尘之回声是什么游戏"
                                }
                            ]
                        }
                    ]
                }),
                response: concat!(
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"《尘之回声》是\"}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"一款俯视角动作冒险游戏。\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4}}}\n\n"
                )
                .to_string(),
            }],
            None,
            None,
            true,
        );

        assert!(markdown.contains("## System Prompt"));
        assert!(markdown.contains("You are a helpful assistant."));
        assert!(markdown.contains("### 👤 User"));
        assert!(markdown.contains("尘之回声是什么游戏"));
        assert!(markdown.contains("### 🤖 Assistant"));
        assert!(markdown.contains("《尘之回声》是一款俯视角动作冒险游戏。"));
    }

    #[test]
    fn raw_context_export_keeps_full_tool_output() {
        let long_output = "A".repeat(2500);
        let markdown = format_rounds_as_markdown(
            "session-tool-output",
            &[crate::agent::instance::RawRound {
                round: 1,
                timestamp: 0,
                request: serde_json::json!({
                    "model": "gpt-5.4",
                    "input": [
                        {
                            "type": "function_call_output",
                            "call_id": "call_1",
                            "output": long_output.clone()
                        }
                    ]
                }),
                response: String::new(),
            }],
            None,
            None,
            false,
        );

        assert!(markdown.contains("### Tool Output"));
        assert!(markdown.contains("*Call ID: `call_1`*"));
        assert!(markdown.contains(&long_output));
        assert!(!markdown.contains("... (truncated)"));
    }

    #[test]
    fn raw_context_export_interleaves_parallel_tool_outputs_with_matching_calls() {
        let markdown = format_rounds_as_markdown(
            "session-parallel-tools",
            &[crate::agent::instance::RawRound {
                round: 1,
                timestamp: 0,
                request: serde_json::json!({
                    "model": "gpt-5.4",
                    "input": [
                        {
                            "type": "function_call",
                            "call_id": "call_list",
                            "name": "list",
                            "arguments": {
                                "path": "C:\\\\repo"
                            }
                        },
                        {
                            "type": "function_call",
                            "call_id": "call_grep",
                            "name": "grep",
                            "arguments": {
                                "pattern": "TODO"
                            }
                        },
                        {
                            "type": "function_call_output",
                            "call_id": "call_grep",
                            "output": "grep output"
                        },
                        {
                            "type": "function_call_output",
                            "call_id": "call_list",
                            "output": "list output"
                        }
                    ]
                }),
                response: String::new(),
            }],
            None,
            None,
            false,
        );

        let list_call_index = markdown
            .find("**Tool call: `list`**")
            .expect("list tool call");
        let list_output_index = markdown.find("list output").expect("list output");
        let grep_call_index = markdown
            .find("**Tool call: `grep`**")
            .expect("grep tool call");
        let grep_output_index = markdown.find("grep output").expect("grep output");

        assert!(list_call_index < list_output_index);
        assert!(list_output_index < grep_call_index);
        assert!(grep_call_index < grep_output_index);
    }

    #[test]
    fn parse_sse_response_supports_responses_event_blocks() {
        let mut out = String::new();
        parse_sse_response(
            &mut out,
            concat!(
                "event: response.output_text.delta\n",
                "data: {\"delta\":\"First answer.\"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"delta\":\" More detail.\"}\n\n",
                "event: response.completed\n",
                "data: {\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
            ),
        );

        assert!(out.contains("First answer. More detail."));
    }

    #[test]
    fn session_export_marks_missing_optional_fields_as_empty() {
        let detail = SessionDetail {
            id: "session-1".to_string(),
            title: "Migrated Session".to_string(),
            agent_id: None,
            session_type: "chat".to_string(),
            parent_session_id: None,
            latest_completed_run_id: None,
            created_at: 10,
            updated_at: 20,
            messages: vec![ChatMessage {
                id: "message-1".to_string(),
                role: MessageRole::Assistant,
                content: "hello".to_string(),
                created_at: 10,
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
            }],
        };

        let markdown = format_session_detail_as_markdown(&detail, &[], None, None, true, None);

        assert!(markdown.contains("session-store-fallback"));
        assert!(markdown.contains("\"agentId\": \"empty\""));
        assert!(markdown.contains("\"parentSessionId\": \"empty\""));
        assert!(markdown.contains("\"latestCompletedRunId\": \"empty\""));
        assert!(markdown.contains("\"promptPrefix\": \"empty\""));
        assert!(markdown.contains("\"promptSuffix\": \"empty\""));
        assert!(markdown.contains("\"responseId\": \"empty\""));
        assert!(markdown.contains("\"toolCalls\": \"empty\""));
        assert!(markdown.contains("\"toolCallId\": \"empty\""));
        assert!(markdown.contains("\"images\": \"empty\""));
        assert!(markdown.contains("\"thinkingContent\": \"empty\""));
        assert!(markdown.contains("\"thinkingDuration\": \"empty\""));
        assert!(markdown.contains("\"knowledgeProposal\": \"empty\""));
        assert!(markdown.contains(&format!("`{}`", EMPTY_EXPORT_FIELD)));
    }

    #[test]
    fn session_export_marks_missing_tool_call_fields_as_empty() {
        let detail = SessionDetail {
            id: "session-1".to_string(),
            title: "Migrated Session".to_string(),
            agent_id: None,
            session_type: "chat".to_string(),
            parent_session_id: None,
            latest_completed_run_id: None,
            created_at: 10,
            updated_at: 20,
            messages: vec![ChatMessage {
                id: "message-1".to_string(),
                role: MessageRole::Assistant,
                content: "hello".to_string(),
                created_at: 10,
                prompt_prefix: None,
                prompt_suffix: None,
                response_id: None,
                tool_calls: Some(vec![ToolCallInfo {
                    id: "tc-1".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                    server_tool: None,
                    server_tool_output: None,
                    outcome: None,
                    recorded_output: None,
                    nested_tool_calls: None,
                }]),
                tool_call_id: None,
                images: None,
                thinking_content: None,
                thinking_duration: None,
                thinking_signature: None,
                knowledge_proposal: None,
            }],
        };

        let markdown = format_session_detail_as_markdown(&detail, &[], None, None, true, None);

        assert!(markdown.contains("\"serverTool\": \"empty\""));
        assert!(markdown.contains("\"serverToolOutput\": \"empty\""));
        assert!(markdown.contains("\"outcome\": \"empty\""));
        assert!(markdown.contains("\"recordedOutput\": \"empty\""));
        assert!(markdown.contains("\"nestedToolCalls\": \"empty\""));
    }

    #[test]
    fn session_export_includes_system_prompt_when_provided() {
        let detail = SessionDetail {
            id: "session-2".to_string(),
            title: "Session With Agent".to_string(),
            agent_id: Some("dev".to_string()),
            session_type: "chat".to_string(),
            parent_session_id: None,
            latest_completed_run_id: Some("run-2".to_string()),
            created_at: 10,
            updated_at: 20,
            messages: vec![],
        };

        let markdown = format_session_detail_as_markdown(
            &detail,
            &[],
            None,
            None,
            true,
            Some("You are a helpful assistant."),
        );

        assert!(markdown.contains("## System Prompt"));
        assert!(markdown.contains("You are a helpful assistant."));
        assert!(markdown.contains("current agent definition"));
    }

    #[test]
    fn migrated_v9_session_can_still_export_after_store_upgrade() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("locus.db");
        let conn = Connection::open(&db_path).expect("create db");

        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
                workspace_id TEXT,
                session_type TEXT NOT NULL DEFAULT 'chat',
                agent_id TEXT,
                archived_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
             CREATE INDEX idx_sessions_workspace ON sessions(workspace_id);

             CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                prompt_prefix TEXT,
                prompt_suffix TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                images TEXT,
                thinking_content TEXT,
                thinking_duration INTEGER,
                thinking_signature TEXT,
                metadata_json TEXT
             );
             CREATE INDEX idx_messages_session ON messages(session_id);

             CREATE TABLE token_usage (
                session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                total_input_tokens INTEGER NOT NULL DEFAULT 0,
                total_output_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                priced_rounds INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE todos (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                position INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                PRIMARY KEY (session_id, position)
             );
             CREATE INDEX idx_todos_session ON todos(session_id);
             PRAGMA user_version = 9;",
        )
        .expect("create v9 schema");

        conn.execute(
            "INSERT INTO sessions (id, title, parent_session_id, workspace_id, session_type, agent_id, archived_at, created_at, updated_at)
             VALUES (?1, ?2, NULL, NULL, 'chat', NULL, NULL, 100, 100)",
            params!["session-1", "Migrated Session"],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, created_at, prompt_prefix, prompt_suffix, metadata_json)
             VALUES (?1, ?2, 'user', '历史消息', 100, NULL, NULL, NULL)",
            params!["message-1", "session-1"],
        )
        .expect("insert message");
        drop(conn);

        let store = SessionStore::new(dir.path()).expect("migrate store");
        let detail = store.load_session("session-1").expect("load session");
        let markdown = format_session_detail_as_markdown(&detail, &[], None, None, true, None);

        assert!(markdown.contains("session-store-fallback"));
        assert!(markdown.contains("历史消息"));
        assert!(markdown.contains("\"latestCompletedRunId\": \"empty\""));
        assert!(markdown.contains("\"promptPrefix\": \"empty\""));
        assert!(markdown.contains("\"promptSuffix\": \"empty\""));
    }
}

fn format_user_message(out: &mut String, content: Option<&serde_json::Value>) {
    out.push_str("### 👤 User\n\n");
    let mut wrote_any = false;
    match content {
        Some(serde_json::Value::String(s)) if !s.is_empty() => {
            out.push_str(s);
            out.push_str("\n\n");
            wrote_any = true;
        }
        Some(serde_json::Value::Array(arr)) => {
            for block in arr {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") | Some("input_text") => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                out.push_str(text);
                                out.push_str("\n\n");
                                wrote_any = true;
                            }
                        }
                    }
                    Some("tool_result") => {
                        let tool_id = block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        out.push_str(&format!(
                            "<details><summary>Tool Result ({})</summary>\n\n",
                            tool_id
                        ));
                        if let Some(serde_json::Value::Array(parts)) = block.get("content") {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    write_text_code_block(out, text);
                                }
                            }
                        } else if let Some(serde_json::Value::String(s)) = block.get("content") {
                            write_text_code_block(out, s);
                        }
                        out.push_str("</details>\n\n");
                        wrote_any = true;
                    }
                    Some("image") | Some("input_image") => {
                        out.push_str("*(image)*\n\n");
                        wrote_any = true;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    if !wrote_any {
        out.push_str("*(empty message)*\n\n");
    }
}

fn format_assistant_from_request(out: &mut String, content: Option<&serde_json::Value>) {
    let mut text_blocks: Vec<String> = Vec::new();
    let mut tool_names: Vec<String> = Vec::new();

    match content {
        Some(serde_json::Value::String(text)) if !text.is_empty() => {
            text_blocks.push(text.clone());
        }
        Some(serde_json::Value::Array(arr)) => {
            for block in arr {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") | Some("output_text") => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                text_blocks.push(text.to_string());
                            }
                        }
                    }
                    Some("tool_use") => {
                        if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                            tool_names.push(name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    if !text_blocks.is_empty() || !tool_names.is_empty() {
        out.push_str("### Assistant (prior context)\n\n");
        for text in text_blocks {
            out.push_str(&text);
            out.push_str("\n\n");
        }
        if !tool_names.is_empty() {
            out.push_str(&format!("*Tool calls: {}*\n\n", tool_names.join(", ")));
        }
    }
}

fn format_assistant_tool_call_message(
    out: &mut String,
    name: Option<&str>,
    arguments: Option<&serde_json::Value>,
) {
    out.push_str("### Assistant (prior context)\n\n");
    out.push_str(&format!(
        "**Tool call: `{}`**\n\n",
        name.filter(|value| !value.is_empty()).unwrap_or("unknown")
    ));
    if let Some(arguments) = arguments {
        write_jsonish_code_block(out, arguments);
    } else {
        out.push_str("`empty`\n\n");
    }
}

fn format_tool_output_message(
    out: &mut String,
    call_id: Option<&str>,
    output: Option<&serde_json::Value>,
) {
    out.push_str("### Tool Output\n\n");
    if let Some(call_id) = call_id.filter(|value| !value.is_empty()) {
        out.push_str(&format!("*Call ID: `{}`*\n\n", call_id));
    }
    match output {
        Some(serde_json::Value::String(text)) => write_text_code_block(out, text),
        Some(value) => write_jsonish_code_block(out, value),
        None => out.push_str("`empty`\n\n"),
    }
}

fn write_jsonish_code_block(out: &mut String, value: &serde_json::Value) {
    let pretty = match value {
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|parsed| serde_json::to_string_pretty(&parsed).ok())
            .unwrap_or_else(|| text.clone()),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };

    if serde_json::from_str::<serde_json::Value>(&pretty).is_ok() {
        out.push_str("```json\n");
        out.push_str(&pretty);
        out.push_str("\n```\n\n");
    } else {
        write_text_code_block(out, &pretty);
    }
}

fn write_text_code_block(out: &mut String, text: &str) {
    out.push_str("```\n");
    out.push_str(text);
    out.push_str("\n```\n\n");
}

fn parse_sse_response(out: &mut String, raw_sse: &str) {
    let mut current_blocks: HashMap<usize, ContentBlock> = HashMap::new();
    let mut finished_blocks: Vec<(usize, ContentBlock)> = Vec::new();

    #[derive(Debug)]
    enum ContentBlock {
        Thinking(String),
        Text(String),
        ToolUse { name: String, input_json: String },
    }

    #[derive(Debug)]
    struct ResponseToolCall {
        order: usize,
        name: String,
        arguments: String,
    }

    let mut openai_text = String::new();
    let mut openai_tool_calls: HashMap<i64, (String, String)> = HashMap::new(); // index -> (name, arguments)
    let mut saw_openai_chat_format = false;
    let mut responses_text = String::new();
    let mut responses_thinking = String::new();
    let mut response_tool_calls: HashMap<String, ResponseToolCall> = HashMap::new();
    let mut next_response_tool_order = 0usize;

    let mut remaining = raw_sse;
    loop {
        let (event_block, tail) = match next_export_sse_separator(remaining) {
            Some((pos, sep_len)) => (&remaining[..pos], &remaining[pos + sep_len..]),
            None => (remaining, ""),
        };
        remaining = tail;

        if let Some((event_name, event)) = parse_export_sse_block(event_block) {
            let event_type = event_name
                .or_else(|| {
                    event
                        .get("type")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_default();

            if event.get("choices").is_some() {
                saw_openai_chat_format = true;
                if let Some(choices) = event.get("choices").and_then(|value| value.as_array()) {
                    for choice in choices {
                        if let Some(delta) = choice.get("delta") {
                            if let Some(content) =
                                delta.get("content").and_then(|value| value.as_str())
                            {
                                openai_text.push_str(content);
                            }
                            if let Some(tool_calls) =
                                delta.get("tool_calls").and_then(|value| value.as_array())
                            {
                                for tool_call in tool_calls {
                                    let idx = tool_call
                                        .get("index")
                                        .and_then(|value| value.as_i64())
                                        .unwrap_or(0);
                                    let entry = openai_tool_calls
                                        .entry(idx)
                                        .or_insert_with(|| (String::new(), String::new()));
                                    if let Some(function) = tool_call.get("function") {
                                        if let Some(name) =
                                            function.get("name").and_then(|value| value.as_str())
                                        {
                                            entry.0 = name.to_string();
                                        }
                                        if let Some(arguments) = function
                                            .get("arguments")
                                            .and_then(|value| value.as_str())
                                        {
                                            entry.1.push_str(arguments);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                match event_type.as_str() {
                    "content_block_start" => {
                        let index = event
                            .get("index")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0) as usize;
                        if let Some(block) = event.get("content_block") {
                            match block.get("type").and_then(|value| value.as_str()) {
                                Some("thinking") => {
                                    current_blocks
                                        .insert(index, ContentBlock::Thinking(String::new()));
                                }
                                Some("text") => {
                                    current_blocks.insert(index, ContentBlock::Text(String::new()));
                                }
                                Some("tool_use") => {
                                    let name = block
                                        .get("name")
                                        .and_then(|value| value.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    current_blocks.insert(
                                        index,
                                        ContentBlock::ToolUse {
                                            name,
                                            input_json: String::new(),
                                        },
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_delta" => {
                        let index = event
                            .get("index")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0) as usize;
                        if let Some(delta) = event.get("delta") {
                            match delta.get("type").and_then(|value| value.as_str()) {
                                Some("thinking_delta") => {
                                    if let Some(ContentBlock::Thinking(ref mut text)) =
                                        current_blocks.get_mut(&index)
                                    {
                                        if let Some(delta_text) =
                                            delta.get("thinking").and_then(|value| value.as_str())
                                        {
                                            text.push_str(delta_text);
                                        }
                                    }
                                }
                                Some("text_delta") => {
                                    if let Some(ContentBlock::Text(ref mut text)) =
                                        current_blocks.get_mut(&index)
                                    {
                                        if let Some(delta_text) =
                                            delta.get("text").and_then(|value| value.as_str())
                                        {
                                            text.push_str(delta_text);
                                        }
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(ContentBlock::ToolUse {
                                        ref mut input_json, ..
                                    }) = current_blocks.get_mut(&index)
                                    {
                                        if let Some(delta_text) = delta
                                            .get("partial_json")
                                            .and_then(|value| value.as_str())
                                        {
                                            input_json.push_str(delta_text);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_stop" => {
                        let index = event
                            .get("index")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0) as usize;
                        if let Some(block) = current_blocks.remove(&index) {
                            finished_blocks.push((index, block));
                        }
                    }
                    "response.output_text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|value| value.as_str()) {
                            responses_text.push_str(delta);
                        }
                    }
                    "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                        if let Some(delta) = event.get("delta").and_then(|value| value.as_str()) {
                            responses_thinking.push_str(delta);
                        }
                    }
                    "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
                        if let Some(text) = event.get("text").and_then(|value| value.as_str()) {
                            sync_export_event_text(&mut responses_thinking, text);
                        }
                    }
                    "response.reasoning_summary_part.done" => {
                        if let Some(text) =
                            extract_export_response_part_text(&event, "reasoning_summary_text")
                        {
                            sync_export_event_text(&mut responses_thinking, text);
                        }
                    }
                    "response.content_part.done" => {
                        if let Some(text) =
                            extract_export_response_part_text(&event, "reasoning_text")
                        {
                            sync_export_event_text(&mut responses_thinking, text);
                        }
                    }
                    "response.output_item.added" => {
                        if event
                            .get("item")
                            .and_then(|item| item.get("type"))
                            .and_then(|value| value.as_str())
                            == Some("function_call")
                        {
                            if let Some(key) = response_tool_call_key(&event) {
                                let order = event
                                    .get("output_index")
                                    .and_then(|value| value.as_u64())
                                    .map(|value| value as usize)
                                    .unwrap_or_else(|| {
                                        let order = next_response_tool_order;
                                        next_response_tool_order += 1;
                                        order
                                    });
                                next_response_tool_order = next_response_tool_order.max(order + 1);
                                let entry = response_tool_calls.entry(key).or_insert_with(|| {
                                    ResponseToolCall {
                                        order,
                                        name: String::new(),
                                        arguments: String::new(),
                                    }
                                });
                                entry.order = order;
                                entry.name = event
                                    .get("item")
                                    .and_then(|item| item.get("name"))
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                if let Some(arguments) = event
                                    .get("item")
                                    .and_then(|item| item.get("arguments"))
                                    .and_then(|value| value.as_str())
                                {
                                    entry.arguments = arguments.to_string();
                                }
                            }
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        if let Some(key) = response_tool_call_key(&event) {
                            if let Some(entry) = response_tool_calls.get_mut(&key) {
                                if let Some(delta) =
                                    event.get("delta").and_then(|value| value.as_str())
                                {
                                    entry.arguments.push_str(delta);
                                }
                            }
                        }
                    }
                    "response.function_call_arguments.done" => {
                        if let Some(key) = response_tool_call_key(&event) {
                            if let Some(entry) = response_tool_calls.get_mut(&key) {
                                if let Some(arguments) =
                                    event.get("arguments").and_then(|value| value.as_str())
                                {
                                    entry.arguments = arguments.to_string();
                                }
                            }
                        }
                    }
                    "response.output_item.done" => {
                        match event
                            .get("item")
                            .and_then(|item| item.get("type"))
                            .and_then(|value| value.as_str())
                        {
                            Some("function_call") => {
                                if let Some(key) = response_tool_call_key(&event) {
                                    if let Some(entry) = response_tool_calls.get_mut(&key) {
                                        if let Some(arguments) = event
                                            .get("item")
                                            .and_then(|item| item.get("arguments"))
                                            .and_then(|value| value.as_str())
                                        {
                                            entry.arguments = arguments.to_string();
                                        }
                                    }
                                }
                            }
                            Some("message") => {
                                if let Some(parts) = event
                                    .get("item")
                                    .and_then(|item| item.get("content"))
                                    .and_then(|value| value.as_array())
                                {
                                    for part in parts {
                                        match part.get("type").and_then(|value| value.as_str()) {
                                            Some("output_text") | Some("text") => {
                                                if let Some(text) = part
                                                    .get("text")
                                                    .and_then(|value| value.as_str())
                                                {
                                                    sync_export_event_text(
                                                        &mut responses_text,
                                                        text,
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        if remaining.is_empty() {
            break;
        }
    }

    if !current_blocks.is_empty() {
        let mut open_blocks: Vec<_> = current_blocks.into_iter().collect();
        open_blocks.sort_by_key(|(index, _)| *index);
        finished_blocks.extend(open_blocks);
    }

    let mut append_index = finished_blocks
        .iter()
        .map(|(index, _)| *index)
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    if saw_openai_chat_format {
        if !openai_text.is_empty() {
            finished_blocks.push((append_index, ContentBlock::Text(openai_text)));
            append_index += 1;
        }
        let mut tool_indices: Vec<i64> = openai_tool_calls.keys().copied().collect();
        tool_indices.sort();
        for idx in tool_indices {
            if let Some((name, args)) = openai_tool_calls.remove(&idx) {
                finished_blocks.push((
                    append_index,
                    ContentBlock::ToolUse {
                        name,
                        input_json: args,
                    },
                ));
                append_index += 1;
            }
        }
    }

    if !responses_thinking.is_empty() {
        finished_blocks.push((append_index, ContentBlock::Thinking(responses_thinking)));
        append_index += 1;
    }

    if !responses_text.is_empty() {
        finished_blocks.push((append_index, ContentBlock::Text(responses_text)));
        append_index += 1;
    }

    if !response_tool_calls.is_empty() {
        let mut tool_calls: Vec<_> = response_tool_calls.into_values().collect();
        tool_calls.sort_by_key(|tool_call| tool_call.order);
        for tool_call in tool_calls {
            finished_blocks.push((
                append_index,
                ContentBlock::ToolUse {
                    name: tool_call.name,
                    input_json: tool_call.arguments,
                },
            ));
            append_index += 1;
        }
    }

    finished_blocks.sort_by_key(|(idx, _)| *idx);

    for (_, block) in &finished_blocks {
        match block {
            ContentBlock::Thinking(text) => {
                if !text.is_empty() {
                    out.push_str("<details><summary>Thinking</summary>\n\n");
                    out.push_str(text);
                    out.push_str("\n\n</details>\n\n");
                }
            }
            ContentBlock::Text(text) => {
                if !text.is_empty() {
                    out.push_str(text);
                    out.push_str("\n\n");
                }
            }
            ContentBlock::ToolUse { name, input_json } => {
                let pretty = serde_json::from_str::<serde_json::Value>(input_json)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or_else(|| input_json.clone());
                out.push_str(&format!("**Tool call: `{}`**\n\n", name));
                out.push_str("```json\n");
                out.push_str(&pretty);
                out.push_str("\n```\n\n");
            }
        }
    }

    if finished_blocks.is_empty() {
        out.push_str("*(no response content)*\n\n");
    }
}

fn next_export_sse_separator(buffer: &str) -> Option<(usize, usize)> {
    let lf = buffer.find("\n\n").map(|position| (position, 2usize));
    let crlf = buffer.find("\r\n\r\n").map(|position| (position, 4usize));

    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

fn parse_export_sse_block(event_block: &str) -> Option<(Option<String>, serde_json::Value)> {
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in event_block.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("event: ") {
            event_name = Some(name.trim().to_string());
        } else if let Some(data) = line.strip_prefix("data: ") {
            let trimmed = data.trim();
            if trimmed == "[DONE]" {
                return None;
            }
            data_lines.push(trimmed.to_string());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    serde_json::from_str::<serde_json::Value>(&data)
        .ok()
        .map(|event| (event_name, event))
}

fn sync_export_event_text(target: &mut String, text: &str) {
    if text.is_empty() || target == text {
        return;
    }

    if target.is_empty() {
        target.push_str(text);
        return;
    }

    if let Some(suffix) = text.strip_prefix(target.as_str()) {
        target.push_str(suffix);
        return;
    }

    target.clear();
    target.push_str(text);
}

fn extract_export_response_part_text<'a>(
    event: &'a serde_json::Value,
    expected_type: &str,
) -> Option<&'a str> {
    event
        .get("part")
        .filter(|part| part.get("type").and_then(|value| value.as_str()) == Some(expected_type))
        .and_then(|part| part.get("text").and_then(|value| value.as_str()))
}

fn response_tool_call_key(event: &serde_json::Value) -> Option<String> {
    event
        .get("item_id")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .or_else(|| {
            event
                .get("output_index")
                .and_then(|value| value.as_u64())
                .map(|value| value.to_string())
        })
        .or_else(|| {
            event
                .get("item")
                .and_then(|item| item.get("id"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .or_else(|| {
            event
                .get("item")
                .and_then(|item| item.get("call_id"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
}

#[tauri::command]
pub async fn answer_question(
    question_id: String,
    answer: String,
    question_store: State<'_, QuestionStore>,
) -> Result<(), AppError> {
    let sender = {
        let mut store = question_store.lock().await;
        store.remove(&question_id)
    };
    match sender {
        Some(tx) => tx
            .send(answer)
            .map_err(|_| "Question receiver dropped".to_string().into()),
        None => Err(format!("Question '{}' not found or already answered", question_id).into()),
    }
}
