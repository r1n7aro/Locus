use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::FutureExt;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::auth::CodexAuthStateHandle;
use super::{StreamEvent, TokenUsage};
use crate::agent::definition::{
    canonical_agent_id, is_hidden_legacy_agent_id, AgentDef, AgentDefRegistry,
    GENERIC_PROJECT_TYPE, UNITY_PROJECT_TYPE,
};
use crate::agent::instance::{
    AgentInstance, AgentSystemPromptStats, AssistantStreamSnapshot, KnowledgeAccessMode,
    LlmBackend, MockModelProfile, RawContextStore,
};
use crate::auth::AuthState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::knowledge_store::{self, KnowledgeDocument, KnowledgeInjectMode, KnowledgeType};
use crate::session::models::{
    AssetRefData, ChatMessage, ImageData, KnowledgeProposalItem, KnowledgeProposalItemKind,
    KnowledgeProposalStatus, PendingSessionInput, SessionDetail, SessionEventRecord,
    SessionMessagePage, SessionRunSummary, SessionRuntimeSnapshot, SessionRuntimeStatus,
    SessionSummary, SessionTurnPreview, SessionViewSnapshot, SessionWorkspaceScope, TodoSnapshot,
    UserIntentPayload,
};
use crate::session::pending_inputs::QueuePendingInputRequest;
use crate::session::store::{CompactedContextOutput, SessionStore, CHILD_SESSION_FORK_ERROR};
use crate::tool::ToolRegistry;
use crate::workspace_definition_registry::WorkspaceDefinitionRegistry;
use crate::workspace_service::{
    CheckoutId, ProjectRegistry, ResolvedWorkspaceScope, ServiceKind, WorkspaceRef,
    WorkspaceRuntime,
};
use crate::workspace_tool_registry::WorkspaceToolRegistry;
use crate::{
    ActiveTaskHandle, ActiveTasks, AgentDefRegistryState, ApiKeyState, PendingInputQueueHandle,
    ProviderKeysState, QuestionStore,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub project_types: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentDefinitionSource {
    Checkout,
    LegacyGlobal,
}

fn agent_definition_source(
    existing_session: bool,
    persisted_checkout: bool,
    has_runtime: bool,
) -> AgentDefinitionSource {
    if (existing_session && persisted_checkout) || (!existing_session && has_runtime) {
        AgentDefinitionSource::Checkout
    } else {
        AgentDefinitionSource::LegacyGlobal
    }
}

struct ChatWorkspaceResolution {
    scope: Option<ResolvedWorkspaceScope>,
    definition_source: AgentDefinitionSource,
}

impl ChatWorkspaceResolution {
    fn runtime(&self) -> Option<&Arc<WorkspaceRuntime>> {
        self.scope.as_ref().map(ResolvedWorkspaceScope::runtime)
    }
}

// Keep session switching responsive even when a tool-heavy round expands the
// raw row count while preserving its assistant/tool boundary.
const DEFAULT_SESSION_VIEW_MESSAGE_LIMIT: u32 = 120;

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
        "finishing" => SessionRuntimeStatus::Finishing,
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

fn fallback_runtime_snapshot(session_id: &str, run_id: &str) -> SessionRuntimeSnapshot {
    let now = current_unix_millis() / 1000;
    SessionRuntimeSnapshot {
        active_run: SessionRunSummary {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            status: "running".to_string(),
            started_at: now,
            updated_at: now,
            finished_at: None,
            error_message: None,
        },
        active_tool_calls: Vec::new(),
        streaming_text: String::new(),
        streaming_thinking: String::new(),
        live_render_parts: Vec::new(),
        stream_sequence: 0,
        streaming_text_order: 0,
        thinking_order: 0,
        is_thinking: false,
        thinking_duration: 0,
        pending_question: None,
        pending_tool_confirms: Vec::new(),
        is_compacting: false,
        compact_queued: false,
    }
}

async fn active_task_run_id(active_tasks: &ActiveTasks, session_id: &str) -> Option<String> {
    active_tasks
        .lock()
        .await
        .get(session_id)
        .map(|task| task.run_id.clone())
}

fn runtime_snapshot_for_active_task(
    store: &SessionStore,
    session_id: &str,
    run_id: &str,
) -> SessionRuntimeSnapshot {
    store
        .runtime_snapshot_for_session(session_id)
        .filter(|snapshot| snapshot.active_run.run_id == run_id)
        .unwrap_or_else(|| fallback_runtime_snapshot(session_id, run_id))
}

#[derive(Debug, Clone)]
struct ActiveSessionCopyState {
    run_id: String,
    partial_assistant: AssistantStreamSnapshot,
}

async fn capture_active_session_copy_states(
    session_ids: &[String],
    active_tasks: &ActiveTasks,
) -> HashMap<String, ActiveSessionCopyState> {
    let session_ids = session_ids.iter().collect::<HashSet<_>>();
    let tasks = active_tasks.lock().await;
    tasks
        .iter()
        .filter(|(session_id, _)| session_ids.contains(session_id))
        .map(|(session_id, task)| {
            (
                session_id.clone(),
                ActiveSessionCopyState {
                    run_id: task.run_id.clone(),
                    partial_assistant: task.partial_assistant.snapshot(),
                },
            )
        })
        .collect()
}

fn runtime_snapshot_with_partial_assistant(
    store: &SessionStore,
    session_id: &str,
    active: &ActiveSessionCopyState,
) -> SessionRuntimeSnapshot {
    let mut runtime = runtime_snapshot_for_active_task(store, session_id, &active.run_id);
    if !active.partial_assistant.text.is_empty() {
        runtime.streaming_text = active.partial_assistant.text.clone();
    }
    if !active.partial_assistant.thinking_content.is_empty() {
        runtime.streaming_thinking = active.partial_assistant.thinking_content.clone();
    }
    if let Some(duration) = active.partial_assistant.thinking_duration {
        runtime.thinking_duration = duration;
    }
    runtime
}

async fn capture_context_export_live_snapshot(
    session_ids: &[String],
    store: &SessionStore,
    pending_input_queue: &PendingInputQueueHandle,
    active_tasks: &ActiveTasks,
) -> Result<crate::session::context_export::ContextExportLiveSnapshot, AppError> {
    let active = capture_active_session_copy_states(session_ids, active_tasks).await;
    let (pending_inputs, compact_queued_sessions) = {
        let queue = pending_input_queue.lock().map_err(|error| {
            AppError::new(
                "session.export_runtime_lock_failed",
                "Failed to capture pending session inputs.",
            )
            .detail(error.to_string())
            .operation("exportSessionContext")
        })?;
        let pending_inputs = session_ids
            .iter()
            .map(|session_id| (session_id.clone(), queue.list_session(session_id)))
            .collect::<HashMap<_, _>>();
        let compact_queued_sessions = active
            .iter()
            .filter(|(session_id, state)| queue.has_compact(session_id, &state.run_id))
            .map(|(session_id, _)| session_id.clone())
            .collect::<HashSet<_>>();
        (pending_inputs, compact_queued_sessions)
    };
    let sessions = session_ids
        .iter()
        .map(|session_id| {
            let runtime = active.get(session_id).map(|active| {
                let mut runtime =
                    runtime_snapshot_with_partial_assistant(store, session_id, active);
                runtime.compact_queued = compact_queued_sessions.contains(session_id);
                runtime
            });
            (
                session_id.clone(),
                crate::session::context_export::ContextExportLiveSession {
                    pending_inputs: pending_inputs.get(session_id).cloned().unwrap_or_default(),
                    runtime,
                },
            )
        })
        .collect();
    Ok(crate::session::context_export::ContextExportLiveSnapshot {
        captured_at: current_unix_millis() / 1000,
        sessions,
    })
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
        KnowledgeType::Design | KnowledgeType::Plan => KnowledgeInjectMode::Path,
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
                inject_mode: knowledge_default_inject_mode(doc_type),
                inherit_inject_mode: true,
                inject_mode_source: Default::default(),
                summary_enabled: crate::knowledge_store::default_summary_enabled_for_type(doc_type),
                command_enabled: false,
                read_only: false,
                ai_edit_mode: crate::knowledge_store::KnowledgeAiEditMode::Inherit,
                ai_maintained: crate::knowledge_store::default_ai_maintained_for_type(doc_type),
                storage_source: crate::knowledge_store::KnowledgeStorageSource::Project,
                inherit_ai_config: true,
                ai_config_source: Default::default(),
                explicit_maintenance_rules:
                    crate::knowledge_store::default_explicit_maintenance_rules_for_type(doc_type),
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                tools: Vec::new(),
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

fn workspace_scope_error(operation: &'static str, error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "workspace.scope_resolution_failed",
        "Failed to resolve the requested workspace checkout.",
    )
    .detail(error.to_string())
    .operation(operation)
}

pub(crate) fn resolve_workspace_scope(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<ResolvedWorkspaceScope, AppError> {
    workspace_registry
        .resolve_workspace_ref(workspace_ref)
        .map_err(|error| workspace_scope_error(operation, error))
}

async fn workspace_agent_registry_snapshot(
    workspace_ref: &WorkspaceRef,
    definitions: &WorkspaceDefinitionRegistry,
    workspace_registry: &ProjectRegistry,
    operation: &'static str,
) -> Result<Arc<AgentDefRegistry>, AppError> {
    let scope = resolve_workspace_scope(workspace_registry, workspace_ref, operation)?;
    definitions
        .snapshot(scope.runtime().as_ref())
        .await
        .map_err(|error| {
            AppError::new(
                "agent.workspace_definitions_unavailable",
                "Failed to load Agent definitions for this checkout.",
            )
            .detail(error)
            .operation(operation)
        })
}

fn preferred_agent_id_for_project_type<'a>(
    registry: &'a AgentDefRegistry,
    agents: &[&'a AgentDef],
    project_type: Option<&str>,
) -> &'a str {
    let Some(project_type) = project_type else {
        return registry.default_id();
    };
    let best_score = agents
        .iter()
        .map(|agent| agent.project_type_match_score(project_type))
        .max()
        .unwrap_or_default();
    if best_score == 0 {
        return registry.default_id();
    }
    if let Some(default) = agents.iter().find(|agent| {
        agent.id == registry.default_id() && agent.supports_project_type(project_type)
    }) {
        return default.id.as_str();
    }
    agents
        .iter()
        .filter(|agent| agent.project_type_match_score(project_type) == best_score)
        .min_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)))
        .map(|agent| agent.id.as_str())
        .unwrap_or_else(|| registry.default_id())
}

fn workspace_project_type(runtime: &WorkspaceRuntime) -> &'static str {
    if runtime
        .services()
        .detected_kinds()
        .contains(&ServiceKind::Unity)
    {
        UNITY_PROJECT_TYPE
    } else {
        GENERIC_PROJECT_TYPE
    }
}

fn list_agent_infos(registry: &AgentDefRegistry, project_type: Option<&str>) -> Vec<AgentInfo> {
    let sub_agent_ids: HashSet<&str> = registry
        .list_all()
        .iter()
        .flat_map(|def| def.sub_agents.iter().map(String::as_str))
        .collect();
    let top_level_defs = registry
        .list_all()
        .into_iter()
        .filter(|def| {
            !sub_agent_ids.contains(def.id.as_str()) && !is_hidden_legacy_agent_id(&def.id)
        })
        .collect::<Vec<_>>();
    let default_id = preferred_agent_id_for_project_type(registry, &top_level_defs, project_type);
    let mut agents: Vec<AgentInfo> = top_level_defs
        .into_iter()
        .map(|def| AgentInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            project_types: def.project_types.clone(),
            is_default: def.id == default_id,
            default_effort: def.default_effort.clone(),
            model_recommendation: def.model_recommendation.clone(),
            source: def.source.clone(),
        })
        .collect();
    agents.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    agents
}

fn list_subagent_infos(registry: &AgentDefRegistry) -> Vec<AgentInfo> {
    let sub_agent_ids: HashSet<String> = registry
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
            project_types: def.project_types.clone(),
            is_default: false,
            default_effort: def.default_effort.clone(),
            model_recommendation: def.model_recommendation.clone(),
            source: def.source.clone(),
        })
        .collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

fn agent_system_prompt(registry: &AgentDefRegistry, agent_id: &str) -> Result<String, AppError> {
    registry
        .get(agent_id)
        .map(|def| def.system_prompt.clone())
        .ok_or_else(|| format!("Agent '{}' not found", agent_id).into())
}

fn agent_env_template(registry: &AgentDefRegistry, agent_id: &str) -> Result<String, AppError> {
    registry
        .get(agent_id)
        .map(|def| def.env_template.clone())
        .ok_or_else(|| format!("Agent '{}' not found", agent_id).into())
}

fn validate_session_project_scope(
    persisted: &SessionWorkspaceScope,
    runtime: &WorkspaceRuntime,
    operation: &'static str,
) -> Result<(), AppError> {
    let Some(expected_project_id) = persisted.project_id.as_deref() else {
        return Ok(());
    };
    if runtime.project_id().as_str() == expected_project_id {
        return Ok(());
    }
    Err(AppError::new(
        "session.workspace_scope_conflict",
        "The session checkout belongs to a different project.",
    )
    .detail(format!(
        "session project {}, checkout project {}",
        expected_project_id,
        runtime.project_id()
    ))
    .operation(operation))
}

/// Resolve an explicit checkout binding for a project-owned session and hold
/// its runtime lease for the complete caller operation. When callers omit the
/// binding, the session's last successful checkout is used as its default.
pub(crate) fn resolve_session_workspace_scope(
    store: &SessionStore,
    workspace_registry: &ProjectRegistry,
    session_id: &str,
    requested_workspace_ref: Option<&WorkspaceRef>,
    operation: &'static str,
) -> Result<ResolvedWorkspaceScope, AppError> {
    let persisted = store
        .get_session_workspace_scope(session_id)
        .map_err(AppError::from)?;
    let workspace_ref = if let Some(requested) = requested_workspace_ref {
        requested.clone()
    } else {
        let default_checkout_id = persisted.default_checkout_id.as_deref().ok_or_else(|| {
            AppError::new(
                "session.checkout_selection_required",
                "Select a worktree from this session's project before starting the operation.",
            )
            .detail(session_id.to_string())
            .operation(operation)
        })?;
        let checkout_id = CheckoutId::new(default_checkout_id).map_err(|error| {
            AppError::new(
                "session.checkout_identity_invalid",
                "The session default checkout identity is invalid.",
            )
            .detail(error.to_string())
            .operation(operation)
        })?;
        WorkspaceRef::new(checkout_id, None)
    };
    let runtime = workspace_registry
        .activate_persisted_checkout(&workspace_ref.checkout_id)
        .map_err(|error| {
            AppError::new(
                "session.checkout_registration_failed",
                "The session checkout could not be activated.",
            )
            .detail(error)
            .operation(operation)
        })?;
    let active_workspace_ref = WorkspaceRef::for_runtime(runtime.as_ref());
    let scope = resolve_workspace_scope(workspace_registry, &active_workspace_ref, operation)?;
    validate_session_project_scope(&persisted, scope.runtime().as_ref(), operation)?;
    Ok(scope)
}

/// Resolve the last checkout used by a project session for read-only commands.
pub(crate) fn resolve_optional_session_workspace_scope(
    store: &SessionStore,
    workspace_registry: &ProjectRegistry,
    session_id: &str,
    operation: &'static str,
) -> Result<Option<ResolvedWorkspaceScope>, AppError> {
    let persisted = store
        .get_session_workspace_scope(session_id)
        .map_err(AppError::from)?;
    if persisted.default_checkout_id.as_deref().is_none() {
        return Ok(None);
    }
    resolve_session_workspace_scope(store, workspace_registry, session_id, None, operation)
        .map(Some)
}

fn resolve_chat_workspace_scope(
    store: &SessionStore,
    workspace_registry: &ProjectRegistry,
    session_id: Option<&str>,
    requested_workspace_ref: Option<&WorkspaceRef>,
) -> Result<ChatWorkspaceResolution, AppError> {
    let operation = "chat";
    let Some(session_id) = session_id else {
        let workspace_ref = requested_workspace_ref.ok_or_else(|| {
            AppError::new(
                "session.checkout_selection_required",
                "Select a checkout before starting a new session.",
            )
            .operation(operation)
        })?;
        let scope = Some(resolve_workspace_scope(
            workspace_registry,
            workspace_ref,
            operation,
        )?);
        return Ok(ChatWorkspaceResolution {
            definition_source: agent_definition_source(false, false, scope.is_some()),
            scope,
        });
    };

    let scope = Some(resolve_session_workspace_scope(
        store,
        workspace_registry,
        session_id,
        requested_workspace_ref,
        operation,
    )?);
    Ok(ChatWorkspaceResolution {
        scope,
        definition_source: agent_definition_source(true, true, true),
    })
}

async fn chat_agent_registry_snapshot(
    workspace: &ChatWorkspaceResolution,
    definitions: &WorkspaceDefinitionRegistry,
    legacy_registry: &AgentDefRegistryState,
) -> Result<Arc<AgentDefRegistry>, AppError> {
    match workspace.definition_source {
        AgentDefinitionSource::Checkout => {
            let runtime = workspace.runtime().ok_or_else(|| {
                AppError::new(
                    "agent.workspace_scope_missing",
                    "The checkout Agent definition scope is unavailable.",
                )
                .operation("chat")
            })?;
            definitions
                .snapshot(runtime.as_ref())
                .await
                .map_err(|error| {
                    AppError::new(
                        "agent.workspace_definitions_unavailable",
                        "Failed to load Agent definitions for this checkout.",
                    )
                    .detail(error)
                    .operation("chat")
                })
        }
        AgentDefinitionSource::LegacyGlobal => Ok(legacy_registry.snapshot().await),
    }
}

#[tauri::command]
pub async fn list_agents(
    registry: State<'_, AgentDefRegistryState>,
) -> Result<Vec<AgentInfo>, AppError> {
    let registry = registry.0.read().await;
    Ok(list_agent_infos(&registry, None))
}

#[tauri::command]
pub async fn list_workspace_agents(
    workspace_ref: WorkspaceRef,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<AgentInfo>, AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "listWorkspaceAgents",
    )?;
    let project_type = workspace_project_type(scope.runtime());
    let registry = workspace_agent_registry_snapshot(
        &workspace_ref,
        definitions.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        "listWorkspaceAgents",
    )
    .await?;
    Ok(list_agent_infos(&registry, Some(project_type)))
}

#[tauri::command]
pub async fn list_subagent_defs(
    registry: State<'_, AgentDefRegistryState>,
) -> Result<Vec<AgentInfo>, AppError> {
    let registry = registry.0.read().await;
    Ok(list_subagent_infos(&registry))
}

#[tauri::command]
pub async fn list_workspace_subagent_defs(
    workspace_ref: WorkspaceRef,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<AgentInfo>, AppError> {
    let registry = workspace_agent_registry_snapshot(
        &workspace_ref,
        definitions.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        "listWorkspaceSubagentDefs",
    )
    .await?;
    Ok(list_subagent_infos(&registry))
}

#[tauri::command]
pub async fn get_agent_system_prompt(
    registry: State<'_, AgentDefRegistryState>,
    agent_id: String,
) -> Result<String, AppError> {
    let registry = registry.0.read().await;
    agent_system_prompt(&registry, &agent_id)
}

#[tauri::command]
pub async fn get_workspace_agent_system_prompt(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let registry = workspace_agent_registry_snapshot(
        &workspace_ref,
        definitions.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        "getWorkspaceAgentSystemPrompt",
    )
    .await?;
    agent_system_prompt(&registry, &agent_id)
}

#[tauri::command]
pub async fn get_agent_env_template(
    registry: State<'_, AgentDefRegistryState>,
    agent_id: String,
) -> Result<String, AppError> {
    let registry = registry.0.read().await;
    agent_env_template(&registry, &agent_id)
}

#[tauri::command]
pub async fn get_workspace_agent_env_template(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let registry = workspace_agent_registry_snapshot(
        &workspace_ref,
        definitions.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        "getWorkspaceAgentEnvTemplate",
    )
    .await?;
    agent_env_template(&registry, &agent_id)
}

fn requested_agent_services(runtime: &WorkspaceRuntime, def: &AgentDef) -> Vec<ServiceKind> {
    let needs_unity = runtime
        .services()
        .detected_kinds()
        .contains(&ServiceKind::Unity)
        && (def.env_template.contains("{{#unity}}")
            || def.tools.iter().any(|tool_name| {
                crate::workspace_service::service::owner_service_for_tool(tool_name)
                    == Some(ServiceKind::Unity)
            }));
    if needs_unity {
        vec![ServiceKind::Unity]
    } else {
        Vec::new()
    }
}

#[allow(clippy::too_many_arguments)]
async fn workspace_agent_preview_instance(
    workspace_ref: &WorkspaceRef,
    agent_id: &str,
    knowledge_access_mode: KnowledgeAccessMode,
    selected_model: Option<&str>,
    subagent_models: Option<HashMap<String, String>>,
    operation: &'static str,
    definitions: &WorkspaceDefinitionRegistry,
    workspace_tools: &WorkspaceToolRegistry,
    workspace_registry: &ProjectRegistry,
    config: &AppConfig,
    raw_store: RawContextStore,
    app_knowledge_dir: Arc<Option<PathBuf>>,
    app_agent_dir: Arc<Option<PathBuf>>,
) -> Result<AgentInstance, AppError> {
    let scope = resolve_workspace_scope(workspace_registry, workspace_ref, operation)?;
    let runtime = scope.runtime();
    let registry_snapshot = definitions
        .snapshot(runtime.as_ref())
        .await
        .map_err(|error| {
            AppError::new(
                "agent.workspace_definitions_unavailable",
                "Failed to load Agent definitions for this checkout.",
            )
            .detail(error)
            .operation(operation)
        })?;
    let def = registry_snapshot
        .get(agent_id)
        .cloned()
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let tool_snapshot = workspace_tools
        .snapshot(runtime.as_ref(), registry_snapshot.as_ref())
        .await
        .map_err(|error| {
            AppError::new(
                "tool.workspace_definitions_unavailable",
                "Failed to load tool definitions for this checkout.",
            )
            .detail(error)
            .operation(operation)
        })?;
    let requested_services = requested_agent_services(runtime.as_ref(), &def);
    let execution = workspace_registry
        .execution_context(runtime.checkout_id(), &requested_services)
        .await
        .map_err(|error| workspace_scope_error(operation, error))?;
    let effective_model = selected_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__workspace-agent-preview__")
        .to_string();
    let mut instance = AgentInstance::new(
        Arc::new(def),
        "__workspace-agent-preview__",
        LlmBackend::ClaudeCodeCli,
        false,
        registry_snapshot,
        tool_snapshot,
        runtime.root().to_string_lossy().to_string(),
        raw_store,
        Some(runtime.project_id().to_string()),
        effective_model,
        None,
        app_knowledge_dir,
        app_agent_dir,
        knowledge_access_mode,
        None,
        subagent_models.unwrap_or_default(),
        tokio::sync::watch::channel(false).1,
    );
    instance.set_execution_context(execution);
    instance.set_async_tasks_enabled(config.async_tasks_enabled());
    instance.configure_preview_lazy_tool_renderer(
        selected_model,
        config.dynamic_tool_loading_mode(),
        config.base_url.as_deref(),
    );
    Ok(instance)
}

#[tauri::command]
pub async fn get_agent_rendered_env_prompt(
    agent_id: String,
    registry: State<'_, AgentDefRegistryState>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<String, AppError> {
    let registry_snapshot = registry.snapshot().await;
    let def = registry_snapshot
        .get(&agent_id)
        .cloned()
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = String::new();
    let workspace_id = None;

    let mut instance = AgentInstance::new(
        Arc::new(def),
        "__agent-preview__",
        LlmBackend::ClaudeCodeCli,
        false,
        registry_snapshot,
        tool_registry.inner().clone(),
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        "__agent-preview__".to_string(),
        None,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        KnowledgeAccessMode::Full,
        None,
        HashMap::new(),
        tokio::sync::watch::channel(false).1,
    );
    instance.set_async_tasks_enabled(config.async_tasks_enabled());

    Ok(instance.rendered_env_prompt().await)
}

#[tauri::command]
pub async fn get_workspace_agent_rendered_env_prompt(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    selected_model: Option<String>,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_tools: State<'_, Arc<WorkspaceToolRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<String, AppError> {
    let instance = workspace_agent_preview_instance(
        &workspace_ref,
        &agent_id,
        KnowledgeAccessMode::Full,
        selected_model.as_deref(),
        None,
        "getWorkspaceAgentRenderedEnvPrompt",
        definitions.inner().as_ref(),
        workspace_tools.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        config.inner().as_ref(),
        raw_store.inner().clone(),
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
    )
    .await?;
    Ok(instance.rendered_env_prompt().await)
}

#[tauri::command]
pub async fn get_agent_system_prompt_stats(
    agent_id: String,
    registry: State<'_, AgentDefRegistryState>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<AgentSystemPromptStats, AppError> {
    let registry_snapshot = registry.snapshot().await;
    let def = registry_snapshot
        .get(&agent_id)
        .cloned()
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = String::new();
    let workspace_id = None;

    let mut instance = AgentInstance::new(
        Arc::new(def),
        "__agent-preview__",
        LlmBackend::ClaudeCodeCli,
        false,
        registry_snapshot,
        tool_registry.inner().clone(),
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        "__agent-preview__".to_string(),
        None,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        KnowledgeAccessMode::Full,
        None,
        HashMap::new(),
        tokio::sync::watch::channel(false).1,
    );
    instance.set_async_tasks_enabled(config.async_tasks_enabled());

    Ok(instance.system_prompt_stats().await)
}

#[tauri::command]
pub async fn get_workspace_agent_system_prompt_stats(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    selected_model: Option<String>,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_tools: State<'_, Arc<WorkspaceToolRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<AgentSystemPromptStats, AppError> {
    let instance = workspace_agent_preview_instance(
        &workspace_ref,
        &agent_id,
        KnowledgeAccessMode::Full,
        selected_model.as_deref(),
        None,
        "getWorkspaceAgentSystemPromptStats",
        definitions.inner().as_ref(),
        workspace_tools.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        config.inner().as_ref(),
        raw_store.inner().clone(),
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
    )
    .await?;
    Ok(instance.system_prompt_stats().await)
}

/// Build the Custom backend for a `custom/<provider>/<model>` id (legacy
/// single-segment `custom/<endpoint>` ids resolve to the provider's first
/// model). Provider-level endpoint/format combine with model-level tuning.
fn custom_backend_for_model(selected_model: &str) -> Result<LlmBackend, AppError> {
    let (provider, model) = crate::commands::workspace::find_custom_provider_model(selected_model)?
        .ok_or_else(|| format!("Custom model config not found: {}", selected_model))?;
    let api_key = crate::keychain::get_secret(&crate::keychain::endpoint_key_name(&provider.id))
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(custom_backend_from_config(provider, model, api_key))
}

pub(crate) fn custom_backend_from_config(
    provider: crate::commands::CustomProvider,
    model: crate::commands::CustomProviderModel,
    api_key: String,
) -> LlmBackend {
    LlmBackend::Custom {
        api_key,
        api_model: model.api_model,
        endpoint: provider.endpoint,
        api_format: provider.api_format,
        context_length: model.context_length,
        remote_compaction_mode: model.remote_compaction_mode,
        supports_tool_lazy_loading: model.supports_tool_lazy_loading,
        supported_reasoning_efforts: model.supported_reasoning_efforts,
        reasoning_param_format: model
            .reasoning_param_format
            .unwrap_or(crate::commands::CustomReasoningParamFormat::OpenaiChatReasoningEffort),
        replay_reasoning_content: model.replay_reasoning_content.unwrap_or(false),
        reasoning_replay_field: model.reasoning_replay_field,
        server_tools: model.server_tools,
        supports_vision: model.supports_vision,
    }
}

async fn resolve_model_backend(
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

    let is_mock = selected_model.starts_with("mock/");
    let is_custom = selected_model.starts_with("custom/");
    let is_openrouter = selected_model.starts_with("openrouter/");
    let is_claude_code = selected_model.starts_with("claude_code/");
    let is_openai_codex = selected_model.starts_with("openai/");
    let is_anthropic_direct = !selected_model.contains('/');

    if is_mock {
        if !config.debug_enabled() {
            return Err("Simulated models require Debug mode".to_string().into());
        }
        let profile = MockModelProfile::from_model_id(selected_model)
            .ok_or_else(|| format!("Unknown simulated model preset: {}", selected_model))?;
        return Ok(LlmBackend::Mock { profile });
    }

    if is_custom {
        return custom_backend_for_model(selected_model);
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

    if is_claude_code {
        return Ok(LlmBackend::ClaudeCodeCli);
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
        "Unrecognized model provider: {}. Use openrouter/, claude_code/, or openai/ prefix, or Anthropic direct format",
        selected_model
    )
    .into())
}

/// Resolve fresh credentials and model-level tuning for a spawned/resumed agent.
pub(crate) async fn resolve_model_backend_for_app(
    app_handle: &AppHandle,
    selected_model: &str,
) -> Result<LlmBackend, AppError> {
    resolve_model_backend(
        selected_model,
        app_handle.state::<Arc<AppConfig>>().inner().as_ref(),
        app_handle.state::<Arc<tokio::sync::Mutex<AuthState>>>().inner(),
        app_handle.state::<ApiKeyState>().inner(),
        app_handle.state::<CodexAuthStateHandle>().inner(),
    )
    .await
}

#[tauri::command]
pub async fn list_agent_injected_items(
    agent_id: String,
    knowledge_mode: Option<String>,
    selected_model: Option<String>,
    subagent_models: Option<HashMap<String, String>>,
    registry: State<'_, AgentDefRegistryState>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<Vec<crate::agent::instance::InjectedPromptItem>, AppError> {
    let registry_snapshot = registry.snapshot().await;
    let def = registry_snapshot
        .get(&agent_id)
        .cloned()
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = String::new();
    let workspace_id = None;
    let knowledge_access_mode = KnowledgeAccessMode::from_request(knowledge_mode.as_deref())
        .map_err(|error| AppError::new("agent.invalid_knowledge_mode", error))?;

    let mut instance = AgentInstance::new(
        Arc::new(def),
        "__agent-preview__",
        LlmBackend::ClaudeCodeCli,
        false,
        registry_snapshot,
        tool_registry.inner().clone(),
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        "__agent-preview__".to_string(),
        None,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        knowledge_access_mode,
        None,
        subagent_models.unwrap_or_default(),
        tokio::sync::watch::channel(false).1,
    );
    instance.set_async_tasks_enabled(config.async_tasks_enabled());
    instance.configure_preview_lazy_tool_renderer(
        selected_model.as_deref(),
        config.dynamic_tool_loading_mode(),
        config.base_url.as_deref(),
    );

    Ok(instance.list_injected_prompt_items().await)
}

#[tauri::command]
pub async fn list_workspace_agent_injected_items(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    knowledge_mode: Option<String>,
    selected_model: Option<String>,
    subagent_models: Option<HashMap<String, String>>,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_tools: State<'_, Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<Vec<crate::agent::instance::InjectedPromptItem>, AppError> {
    let knowledge_access_mode = KnowledgeAccessMode::from_request(knowledge_mode.as_deref())
        .map_err(|error| AppError::new("agent.invalid_knowledge_mode", error))?;
    let instance = workspace_agent_preview_instance(
        &workspace_ref,
        &agent_id,
        knowledge_access_mode,
        selected_model.as_deref(),
        subagent_models,
        "listWorkspaceAgentInjectedItems",
        definitions.inner().as_ref(),
        workspace_tools.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        config.inner().as_ref(),
        raw_store.inner().clone(),
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
    )
    .await?;
    Ok(instance.list_injected_prompt_items().await)
}

#[tauri::command]
pub async fn create_session(
    title: String,
    parent_session_id: Option<String>,
    session_type: Option<String>,
    agent_id: Option<String>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<crate::workspace_service::ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<String, AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "createSession",
    )?;
    let runtime = scope.runtime();
    let ws_id = runtime.project_id().to_string();
    let checkout_id = runtime.checkout_id().to_string();
    let trimmed = title.trim();
    let resolved_title = if trimmed.is_empty() {
        "New session"
    } else {
        trimmed
    };
    let resolved_agent_id = agent_id.as_deref().map(canonical_agent_id);
    store
        .create_session_scoped(
            resolved_title,
            parent_session_id.as_deref(),
            Some(&ws_id),
            Some(&checkout_id),
            session_type.as_deref().unwrap_or("chat"),
            resolved_agent_id,
        )
        .map_err(Into::into)
}

#[tauri::command]
pub async fn fork_session(
    session_id: String,
    title: Option<String>,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<String, AppError> {
    let live_store = store.inner().clone();
    let snapshot_source = live_store.clone();
    let snapshot = tokio::task::spawn_blocking(move || snapshot_source.create_export_snapshot())
        .await
        .map_err(|error| {
            AppError::new(
                "session.fork_snapshot_task_failed",
                "Failed to create a session fork snapshot.",
            )
            .detail(error.to_string())
            .operation("forkSession")
        })??;

    let active =
        capture_active_session_copy_states(std::slice::from_ref(&session_id), active_tasks.inner())
            .await
            .remove(&session_id);
    let runtime = active.as_ref().map(|active| {
        runtime_snapshot_with_partial_assistant(store.inner().as_ref(), &session_id, active)
    });
    let pending_inputs = pending_input_queue
        .lock()
        .map_err(|error| {
            AppError::new(
                "session.fork_runtime_lock_failed",
                "Failed to capture pending session inputs for the fork.",
            )
            .detail(error.to_string())
            .operation("forkSession")
        })?
        .list_session(&session_id);
    let title = title.clone();
    let source_id = session_id.clone();
    tokio::task::spawn_blocking(move || {
        let snapshot_message_ids = snapshot
            .get_messages(&source_id)?
            .into_iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let forked_id = live_store.fork_session_from_export_snapshot(
            &snapshot,
            &source_id,
            title.as_deref(),
        )?;

        let append_runtime_state = (|| -> Result<(), String> {
            if let (Some(active), Some(runtime)) = (active, runtime) {
                let partial_was_copied = active
                    .partial_assistant
                    .persisted_message_id
                    .as_ref()
                    .is_some_and(|message_id| snapshot_message_ids.contains(message_id));
                if !partial_was_copied
                    && (!runtime.streaming_text.is_empty()
                        || !runtime.streaming_thinking.is_empty())
                {
                    live_store.add_message_with_thinking_and_render_parts(
                        &forked_id,
                        crate::session::models::MessageRole::Assistant,
                        &runtime.streaming_text,
                        (!runtime.streaming_thinking.is_empty())
                            .then_some(runtime.streaming_thinking.as_str()),
                        Some(runtime.thinking_duration),
                        None,
                        None,
                        None,
                        Some(runtime.streaming_text_order),
                        Some(runtime.thinking_order),
                        &runtime.live_render_parts,
                    )?;
                }
            }

            for pending in pending_inputs {
                let user_intent_signature = pending
                    .user_intent
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| {
                        format!("Failed to serialize pending fork user intent: {}", error)
                    })?;
                live_store.add_message_with_images_asset_refs_and_signature(
                    &forked_id,
                    crate::session::models::MessageRole::User,
                    &pending.text,
                    pending.images.as_deref(),
                    pending.asset_refs.as_deref(),
                    user_intent_signature.as_deref(),
                    None,
                    None,
                )?;
            }
            Ok(())
        })();

        if let Err(error) = append_runtime_state {
            if let Err(cleanup_error) = live_store.delete_session(&forked_id) {
                return Err(format!(
                    "{}; failed to clean incomplete fork: {}",
                    error, cleanup_error
                ));
            }
            return Err(error);
        }
        Ok(forked_id)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "session.fork_copy_task_failed",
            "Failed to copy the session fork snapshot.",
        )
        .detail(error.to_string())
        .operation("forkSession")
    })?
    .map_err(|error| {
        if error == CHILD_SESSION_FORK_ERROR {
            AppError::new("session.fork_child", "Child sessions cannot be forked.")
                .detail(error)
                .operation("forkSession")
        } else {
            AppError::new("session.fork_failed", "Failed to fork session.")
                .detail(error)
                .operation("forkSession")
        }
    })
}

#[tauri::command]
pub async fn fork_session_from_message(
    session_id: String,
    message_id: String,
    title: Option<String>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<String, AppError> {
    store
        .fork_session_from_message(&session_id, &message_id, title.as_deref())
        .map_err(|error| {
            if error == CHILD_SESSION_FORK_ERROR {
                AppError::new("session.fork_child", "Child sessions cannot be forked.")
                    .detail(error)
                    .operation("forkSession")
            } else {
                AppError::new("session.fork_failed", "Failed to fork session.")
                    .detail(error)
                    .operation("forkSession")
            }
        })
}

#[tauri::command]
pub async fn chat(
    session_id: Option<String>,
    workspace_ref: Option<WorkspaceRef>,
    text: String,
    resume: Option<bool>,
    session_title: Option<String>,
    agent_id: Option<String>,
    sdk_agent: Option<crate::sdk::SdkAgentSpec>,
    model: Option<String>,
    effort: Option<String>,
    fast_mode: Option<bool>,
    multi_agent_enabled: Option<bool>,
    images: Option<Vec<ImageData>>,
    asset_refs: Option<Vec<AssetRefData>>,
    session_type: Option<String>,
    mode: Option<String>,
    user_intent: Option<UserIntentPayload>,
    subagent_models: Option<HashMap<String, String>>,
    subagent_efforts: Option<HashMap<String, String>>,
    subagent_fast_modes: Option<HashMap<String, bool>>,
    knowledge_mode: Option<String>,
    knowledge_doc_type: Option<crate::knowledge_store::KnowledgeType>,
    knowledge_doc_path: Option<String>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, AgentDefRegistryState>,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    workspace_tools: State<'_, Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
    api_key_state: State<'_, ApiKeyState>,
    _provider_keys: State<'_, ProviderKeysState>,
    codex: State<'_, CodexAuthStateHandle>,
    workspace_registry: State<'_, Arc<crate::workspace_service::ProjectRegistry>>,
    raw_store: State<'_, RawContextStore>,
    active_tasks: State<'_, ActiveTasks>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
    undo_manager: State<'_, crate::UndoManagerHandle>,
) -> Result<ChatLaunch, AppError> {
    let chat_workspace = resolve_chat_workspace_scope(
        store.inner().as_ref(),
        workspace_registry.inner().as_ref(),
        session_id.as_deref(),
        workspace_ref.as_ref(),
    )?;
    let scoped_runtime = chat_workspace.runtime().cloned();
    let registry_snapshot = chat_agent_registry_snapshot(
        &chat_workspace,
        definitions.inner().as_ref(),
        registry.inner(),
    )
    .await?;
    let cwd = scoped_runtime
        .as_ref()
        .map(|runtime| runtime.root().display().to_string())
        .unwrap_or_default();
    let ws_id = scoped_runtime
        .as_ref()
        .map(|runtime| runtime.project_id().to_string());
    let checkout_id = scoped_runtime
        .as_ref()
        .map(|runtime| runtime.checkout_id().to_string());

    let is_new_session = session_id.is_none();
    let resume_requested = resume.unwrap_or(false);
    if resume_requested && is_new_session {
        return Err(AppError::new(
            "session.resume_requires_existing",
            "An existing session is required to resume interrupted work.",
        )
        .operation("chat"));
    }
    if resume_requested
        && (!text.trim().is_empty()
            || images.as_ref().is_some_and(|items| !items.is_empty())
            || asset_refs.as_ref().is_some_and(|items| !items.is_empty()))
    {
        return Err(AppError::new(
            "session.resume_requires_empty_input",
            "Resume requires an empty composer.",
        )
        .operation("chat"));
    }
    let session_kind = session_type.as_deref().unwrap_or("chat");
    let mock_model_requested = model
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| value.starts_with("mock/"));
    let explicit_session_title = session_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let codex_model_config = crate::commands::load_codex_model_config().unwrap_or_default();
    let codex_title_generation_enabled =
        if codex_model_config.generate_session_titles && !mock_model_requested {
            let status = codex.lock().await.status();
            status.authenticated && !status.validation_failed
        } else {
            false
        };
    let prepared_title_prompt = (is_new_session
        && session_kind == "chat"
        && explicit_session_title.is_none()
        && codex_title_generation_enabled)
        .then(|| crate::session::title::prepare_session_title_prompt(&text))
        .flatten();
    let generated_title_fallback = prepared_title_prompt
        .as_deref()
        .and_then(crate::session::title::fallback_session_title);

    let inline_agent_def = sdk_agent.as_ref().map(crate::sdk::SdkAgentSpec::agent_def);
    let requested_agent_id = inline_agent_def
        .as_ref()
        .map(|def| def.id.clone())
        .or_else(|| {
            agent_id
                .as_deref()
                .map(canonical_agent_id)
                .map(str::to_string)
        });
    let sid = match session_id {
        Some(id) => id,
        None => {
            let title = explicit_session_title.unwrap_or_else(|| text.chars().take(20).collect());
            let title = generated_title_fallback.unwrap_or(title);
            store.create_session_scoped(
                &title,
                None,
                ws_id.as_deref(),
                checkout_id.as_deref(),
                session_kind,
                requested_agent_id.as_deref(),
            )?
        }
    };

    if resume_requested && !store.latest_run_is_interrupted(&sid)? {
        return Err(AppError::new(
            "session.resume_unavailable",
            "The latest session run is no longer interrupted.",
        )
        .operation("chat"));
    }

    let stale_messages = store.stale_pending_knowledge_proposals(&sid)?;
    for message in stale_messages {
        emit_knowledge_proposal_message(&app_handle, store.inner().as_ref(), &sid, message);
    }

    // Enforce session-agent binding. Python-defined agents resend the full
    // definition for every turn, while retaining the same session id so the
    // provider conversation/prompt cache remains reusable.
    let stored_agent_id = store
        .get_session_agent_id(&sid)
        .ok()
        .flatten()
        .map(|stored| canonical_agent_id(&stored).to_string());
    if let (Some(inline), Some(stored)) = (inline_agent_def.as_ref(), stored_agent_id.as_ref()) {
        if inline.id != *stored {
            return Err(format!(
                "Session {} belongs to agent '{}', not '{}'",
                sid, stored, inline.id
            )
            .into());
        }
    }
    let effective_agent_id = inline_agent_def
        .as_ref()
        .map(|def| def.id.clone())
        .or(stored_agent_id)
        .or(requested_agent_id.clone());

    let def = if let Some(inline) = inline_agent_def {
        Arc::new(inline)
    } else {
        match &effective_agent_id {
            Some(id) => {
                let d = registry_snapshot
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("Unknown agent: {}", id))?;
                Arc::new(d)
            }
            None => {
                let d = registry_snapshot
                    .default_def()
                    .cloned()
                    .ok_or_else(|| "No agent definitions found".to_string())?;
                Arc::new(d)
            }
        }
    };

    let selected_model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "No model selected. Select a model before sending a message.".to_string())?
        .to_string();

    let backend = resolve_model_backend(
        &selected_model,
        config.inner().as_ref(),
        auth.inner(),
        api_key_state.inner(),
        codex.inner(),
    )
    .await?;
    if let Some(title_prompt) = prepared_title_prompt {
        let expected_title = store
            .get_session_title(&sid)?
            .unwrap_or_else(|| title_prompt.chars().take(20).collect());
        crate::session::title::spawn_codex_session_title_generation(
            app_handle.clone(),
            store.inner().clone(),
            scoped_runtime
                .as_deref()
                .map(crate::workspace_service::event::WorkspaceEventScope::for_runtime),
            codex.inner().clone(),
            codex_model_config.transport,
            config.base_url.clone(),
            sid.clone(),
            expected_title,
            crate::session::title::SessionTitleGenerationRequest::codex_default(title_prompt),
            config.debug_enabled(),
        );
    }

    let effective_tool_registry = match scoped_runtime.as_ref() {
        Some(runtime) => workspace_tools
            .snapshot(runtime.as_ref(), registry_snapshot.as_ref())
            .await
            .map_err(|detail| {
                AppError::new(
                    "tool.workspace_definitions_unavailable",
                    "Failed to load tool definitions for this checkout.",
                )
                .detail(detail)
                .operation("chat")
            })?,
        None => tool_registry.inner().clone(),
    };
    let reg = registry_snapshot;
    let tools = match sdk_agent.as_ref() {
        Some(spec) => crate::sdk::tool_registry_for_agent(effective_tool_registry.as_ref(), spec)?,
        None => effective_tool_registry,
    };
    let raw = raw_store.inner().clone();

    let akd = app_knowledge_dir.0.clone();
    let aad = app_agent_dir.0.clone();
    let knowledge_access_mode = KnowledgeAccessMode::from_request(knowledge_mode.as_deref())
        .map_err(|error| AppError::new("chat.invalid_knowledge_mode", error).operation("chat"))?;
    let um = Some(undo_manager.inner().clone());
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let idle_cancel_rx = cancel_rx.clone();
    let (done_tx, done_rx) = tokio::sync::watch::channel(false);
    let execution_binding_request = scoped_runtime.as_ref().map(|runtime| {
        let requested_services = requested_agent_services(runtime.as_ref(), def.as_ref());
        (runtime.checkout_id().clone(), requested_services)
    });
    let execution_context =
        if let Some((checkout_id, requested_services)) = execution_binding_request.as_ref() {
            Some(
                workspace_registry
                    .execution_context(checkout_id, requested_services)
                    .await
                    .map_err(|error| {
                        AppError::new(
                            "workspace.execution_context_failed",
                            "Failed to bind the Agent run to its workspace services.",
                        )
                        .detail(error)
                        .operation("chat")
                    })?,
            )
        } else {
            None
        };
    let persisted_run_scope = execution_context
        .as_ref()
        .map(|execution| execution.persisted_run_scope());
    let mut instance = AgentInstance::new(
        def,
        &sid,
        backend,
        config.debug_enabled(),
        reg,
        tools,
        cwd,
        raw,
        ws_id,
        selected_model.clone(),
        effort.clone(),
        akd,
        aad,
        knowledge_access_mode,
        um,
        subagent_models.unwrap_or_default(),
        cancel_rx,
    );
    if let Some(execution_context) = execution_context {
        instance.set_execution_context(execution_context);
    }
    let effective_fast_mode = fast_mode.unwrap_or(false);
    let effective_multi_agent_enabled = match multi_agent_enabled {
        Some(enabled) => enabled,
        None => store.get_session_multi_agent_enabled(&sid)
            .map_err(|error| AppError::new("session.execution_state_load_failed", "Failed to load session execution state.").detail(error).operation("chat"))?
            .unwrap_or(false),
    };
    instance.set_multi_agent_enabled(effective_multi_agent_enabled);
    instance.set_codex_fast_mode(effective_fast_mode);
    instance.set_session_undo_enabled(config.session_undo_enabled());
    instance.set_subagent_runtime_overrides(
        subagent_efforts.unwrap_or_default(),
        subagent_fast_modes.unwrap_or_default(),
    );
    instance.set_async_tasks_enabled(config.async_tasks_enabled());
    let knowledge_focus = match (knowledge_doc_type, knowledge_doc_path) {
        (Some(doc_type), Some(path)) if !path.trim().is_empty() => {
            Some(crate::agent::instance::KnowledgeFocusDoc {
                doc_type,
                path: path.trim().to_string(),
            })
        }
        _ => None,
    };
    instance.set_knowledge_focus(knowledge_focus);
    let partial_assistant = instance.partial_assistant_state();
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
    store
        .try_start_run_scoped(&sid, &run_id, persisted_run_scope.as_ref())
        .map_err(|error| {
            if error.contains("active run") {
                session_run_locked_error(error)
            } else {
                AppError::new("session.run_start_failed", "Failed to start session run.")
                    .detail(error)
                    .operation("chat")
            }
        })?;
    if let Err(error) = store.set_session_execution_state(
        &sid,
        &selected_model,
        effort.as_deref(),
        effective_fast_mode,
        Some(effective_multi_agent_enabled),
    ) {
        let _ = store.update_run_status(&run_id, "error", Some(&error));
        return Err(AppError::new(
            "session.execution_state_persist_failed",
            "Failed to save the session execution state.",
        )
        .detail(error)
        .operation("chat"));
    }
    let store = store.inner().clone();
    let sid_clone = sid.clone();
    let tasks = active_tasks.inner().clone();
    let sid_for_cleanup = sid.clone();
    let images_for_task = images.unwrap_or_default();
    let asset_refs_for_task = asset_refs.unwrap_or_default();
    let user_intent_for_task = user_intent;
    let run_id_for_task = run_id.clone();
    let store_for_task = store.clone();
    let execution_binding_request_for_task = execution_binding_request.clone();
    let initial_system_reminder = resume_requested.then(|| {
        "<system-reminder>\nThe previous run was interrupted before the task was complete. Continue from the existing conversation context. Inspect the current state, finish the remaining work, and verify the result. Do not repeat completed work.\n</system-reminder>"
            .to_string()
    });
    let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();

    let join_handle = tauri::async_runtime::spawn(async move {
        if start_rx.await.is_err() {
            eprintln!(
                "[Locus] session {} run {} start gate dropped before execution",
                sid_clone, run_id_for_task
            );
            return;
        }

        let mut current_run_id = run_id_for_task.clone();
        let mut next_text = text;
        let mut next_images = images_for_task;
        let mut next_asset_refs = asset_refs_for_task;
        let mut next_mode = effective_mode;
        let mut next_user_intent = user_intent_for_task;
        let mut accepted_pending_input_id: Option<String> = None;
        let mut next_internal_system_reminder = initial_system_reminder;
        let mut idle_cancel_rx = idle_cancel_rx;

        loop {
            let task_result = AssertUnwindSafe(instance.run_with_run_id(
                &handle,
                &store_for_task,
                &next_text,
                if next_images.is_empty() {
                    None
                } else {
                    Some(&next_images)
                },
                if next_asset_refs.is_empty() {
                    None
                } else {
                    Some(&next_asset_refs)
                },
                &next_mode,
                next_user_intent.take(),
                current_run_id.clone(),
                accepted_pending_input_id.take(),
                next_internal_system_reminder.take(),
            ))
            .catch_unwind()
            .await;

            match task_result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    eprintln!("[Locus] session {} failed: {}", sid_clone, e);
                    break;
                }
                Err(panic_payload) => {
                    let panic_message = panic_payload_to_string(panic_payload);
                    eprintln!("[Locus] session {} panicked: {}", sid_clone, panic_message);
                    emit_session_stream_with_run_id(
                        &handle,
                        store_for_task.as_ref(),
                        current_run_id.clone(),
                        StreamEvent::Error {
                            session_id: sid_clone.clone(),
                            error: AppError::new(
                                "chat.stream_failed",
                                format!("Session terminated unexpectedly: {}", panic_message),
                            ),
                        },
                    );
                    break;
                }
            }

            let mut async_reminder: Option<String> = None;
            let mut compact_requested = false;
            let follow_up = loop {
                if *idle_cancel_rx.borrow() { break None; }
                let (claimed_compact, claimed) = {
                    let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                        handle.state();
                    let result = match queue_state.lock() {
                        Ok(mut queue) => {
                            if queue.claim_compact(&sid_clone, &current_run_id) {
                                (true, None)
                            } else {
                                (false, queue.claim_after_run(&sid_clone, &current_run_id))
                            }
                        }
                        Err(error) => {
                            eprintln!(
                                "[Locus] failed to lock pending input queue for session {} run {}: {}",
                                sid_clone, current_run_id, error
                            );
                            (false, None)
                        }
                    };
                    result
                };
                if claimed_compact {
                    compact_requested = true;
                    break None;
                }
                if claimed.is_some() {
                    break claimed;
                }

                let async_tasks: tauri::State<'_, Arc<crate::async_tasks::AsyncTaskManager>> =
                    handle.state();
                let (notifications, has_pending_notifications) =
                    async_tasks.take_notifications_and_pending(&sid_clone);
                if !notifications.is_empty() {
                    async_reminder = Some("<system-reminder>\nBackground task results are ready. Process the delivered results and continue the unfinished work.\n</system-reminder>".to_string());
                    break None;
                }
                if !has_pending_notifications || *idle_cancel_rx.borrow() {
                    break None;
                }

                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                    _ = idle_cancel_rx.changed() => {}
                }
            };
            if !compact_requested && follow_up.is_none() && async_reminder.is_none() {
                break;
            }
            if *idle_cancel_rx.borrow() { break; }

            // Every queued input/reminder/compact is a new Agent run. Rebind
            // the checkout's current service generation before persisting it;
            // the previous run may have crossed an idle stop or service restart.
            let next_persisted_run_scope = if let Some((checkout_id, requested_services)) =
                execution_binding_request_for_task.as_ref()
            {
                let workspace_registry: tauri::State<
                    '_,
                    Arc<crate::workspace_service::ProjectRegistry>,
                > = handle.state();
                match workspace_registry
                    .execution_context(checkout_id, requested_services)
                    .await
                {
                    Ok(execution) => {
                        let scope = execution.persisted_run_scope();
                        instance.set_execution_context(execution);
                        Some(scope)
                    }
                    Err(error) => {
                        if compact_requested {
                            let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                                handle.state();
                            if let Ok(mut queue) = queue_state.lock() {
                                queue.restore_compact(&sid_clone, &current_run_id);
                            };
                        } else if let Some(follow_up) = follow_up {
                            let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                                handle.state();
                            if let Ok(mut queue) = queue_state.lock() {
                                queue.restore_claimed(vec![follow_up]);
                            };
                        }
                        eprintln!(
                            "[Locus] failed to rebind queued follow-up for session {} after run {}: {}",
                            sid_clone, current_run_id, error
                        );
                        break;
                    }
                }
            } else {
                None
            };

            let next_run_id = generate_chat_run_id(&sid_clone);
            if let Err(error) = store_for_task.try_start_run_scoped(
                &sid_clone,
                &next_run_id,
                next_persisted_run_scope.as_ref(),
            ) {
                if compact_requested {
                    let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                        handle.state();
                    if let Ok(mut queue) = queue_state.lock() {
                        queue.restore_compact(&sid_clone, &current_run_id);
                    };
                } else if let Some(follow_up) = follow_up {
                    let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                        handle.state();
                    if let Ok(mut queue) = queue_state.lock() {
                        queue.restore_claimed(vec![follow_up]);
                    };
                }
                eprintln!(
                    "[Locus] failed to start queued follow-up for session {} after run {}: {}",
                    sid_clone, current_run_id, error
                );
                break;
            }

            {
                let mut guard = tasks.lock().await;
                match guard.get_mut(&sid_for_cleanup) {
                    Some(task) if task.run_id == current_run_id => {
                        task.run_id = next_run_id.clone();
                    }
                    _ => {
                        if compact_requested {
                            let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                                handle.state();
                            if let Ok(mut queue) = queue_state.lock() {
                                queue.restore_compact(&sid_clone, &current_run_id);
                            };
                        } else if let Some(follow_up) = follow_up {
                            let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                                handle.state();
                            if let Ok(mut queue) = queue_state.lock() {
                                queue.restore_claimed(vec![follow_up]);
                            };
                        }
                        if let Err(error) = store_for_task.update_run_status(
                            &next_run_id,
                            "error",
                            Some("Active task changed before queued follow-up could start"),
                        ) {
                            eprintln!(
                                "[Locus] failed to mark queued follow-up run {} as error: {}",
                                next_run_id, error
                            );
                        }
                        break;
                    }
                }
            }

            if compact_requested {
                let rebound_input = {
                    let queue_state: tauri::State<'_, crate::PendingInputQueueHandle> =
                        handle.state();
                    let result = match queue_state.lock() {
                        Ok(mut queue) => {
                            queue.rebind_input_run(&sid_clone, &current_run_id, &next_run_id)
                        }
                        Err(error) => {
                            eprintln!(
                                "[Locus] failed to rebind pending input after queued compact for session {} run {}: {}",
                                sid_clone, current_run_id, error
                            );
                            None
                        }
                    };
                    result
                };
                if let Some(input) = rebound_input {
                    emit_session_stream_with_run_id(
                        &handle,
                        store_for_task.as_ref(),
                        next_run_id.clone(),
                        StreamEvent::PendingInputQueued {
                            session_id: sid_clone.clone(),
                            input,
                        },
                    );
                }
                accepted_pending_input_id = None;
                next_text.clear();
                next_images.clear();
                next_asset_refs.clear();
                next_mode = "compact".to_string();
                next_user_intent = None;
                next_internal_system_reminder = None;
            } else if let Some(follow_up) = follow_up {
                accepted_pending_input_id = Some(follow_up.id);
                next_text = follow_up.text;
                next_images = follow_up.images.unwrap_or_default();
                next_asset_refs = follow_up.asset_refs.unwrap_or_default();
                next_mode = follow_up
                    .mode
                    .clone()
                    .or_else(|| {
                        follow_up
                            .user_intent
                            .as_ref()
                            .map(|intent| intent.mode.clone())
                    })
                    .unwrap_or_else(|| "build".to_string());
                next_user_intent = follow_up.user_intent;
            } else {
                next_text.clear();
                next_images.clear();
                next_asset_refs.clear();
                next_mode = "build".to_string();
                next_user_intent = None;
                next_internal_system_reminder = async_reminder;
            }
            current_run_id = next_run_id;
        }
        let removed = {
            let mut guard = tasks.lock().await;
            match guard.get(&sid_for_cleanup) {
                Some(task) if task.run_id == current_run_id => {
                    guard.remove(&sid_for_cleanup).is_some()
                }
                _ => false,
            }
        };
        eprintln!(
            "[Locus] active task cleared for session {} run {} removed={}",
            sid_for_cleanup, current_run_id, removed
        );
        store_for_task.clear_runtime_run_if_current(&sid_for_cleanup, &current_run_id);
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
                partial_assistant,
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
pub async fn queue_chat_input(
    session_id: String,
    run_id: String,
    merge_group_id: String,
    text: String,
    display_text: Option<String>,
    images: Option<Vec<ImageData>>,
    asset_refs: Option<Vec<AssetRefData>>,
    mode: Option<String>,
    user_intent: Option<UserIntentPayload>,
    client_message_id: Option<String>,
    delivery: Option<String>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<PendingSessionInput, AppError> {
    let trimmed_merge_group_id = merge_group_id.trim();
    if trimmed_merge_group_id.is_empty() {
        return Err(AppError::new(
            "session.pending_input.invalid_group",
            "Pending input merge group is required.",
        )
        .operation("chat"));
    }

    let images = images.unwrap_or_default();
    let asset_refs = asset_refs.unwrap_or_default();
    let requested_delivery = if delivery.as_deref() == Some("immediate") {
        "immediate"
    } else {
        "after_run"
    };
    if text.trim().is_empty() && images.is_empty() && asset_refs.is_empty() {
        return Err(AppError::new(
            "session.pending_input.empty",
            "Pending input cannot be empty.",
        )
        .operation("chat"));
    }

    {
        let tasks = active_tasks.lock().await;
        let Some(task) = tasks.get(&session_id) else {
            return Err(AppError::new(
                "session.pending_input.no_active_run",
                "Session has no active run for queued input.",
            )
            .operation("chat")
            .retryable(true));
        };
        if task.run_id != run_id {
            return Err(AppError::new(
                "session.pending_input.run_mismatch",
                "Queued input targets a stale run.",
            )
            .detail(format!(
                "expected active run {}, got {}",
                task.run_id, run_id
            ))
            .operation("chat")
            .retryable(true));
        }
    }

    let run =
        runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id).active_run;
    if !matches!(
        run.status.as_str(),
        "queued" | "starting" | "running" | "waiting_input"
    ) && !(run.status == "finishing" && requested_delivery == "after_run")
    {
        return Err(AppError::new(
            "session.pending_input.run_closed",
            "The active run is no longer accepting queued input.",
        )
        .detail(format!("run {} status {}", run_id, run.status))
        .operation("chat")
        .retryable(true));
    }

    if run.status == "finishing" && requested_delivery == "immediate" {
        return Err(AppError::new(
            "session.pending_input.run_closed",
            "The active run is no longer accepting queued input.",
        )
        .detail(format!("run {} status {}", run_id, run.status))
        .operation("chat")
        .retryable(true));
    }

    let display_text = display_text.unwrap_or_else(|| text.clone());
    let pending = {
        let mut queue = pending_input_queue.lock().map_err(|e| {
            AppError::new(
                "session.pending_input.lock_failed",
                "Pending input queue is unavailable.",
            )
            .detail(e.to_string())
            .operation("chat")
            .retryable(true)
        })?;
        queue.queue_input(QueuePendingInputRequest {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            merge_group_id: trimmed_merge_group_id.to_string(),
            text,
            display_text,
            images,
            asset_refs,
            mode,
            user_intent,
            client_message_id,
            delivery,
        })
    };

    emit_session_stream_with_run_id(
        &app_handle,
        store.inner().as_ref(),
        run_id,
        StreamEvent::PendingInputQueued {
            session_id,
            input: pending.clone(),
        },
    );

    Ok(pending)
}

#[tauri::command]
pub async fn queue_session_compact(
    session_id: String,
    run_id: String,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<bool, AppError> {
    {
        let tasks = active_tasks.lock().await;
        let Some(task) = tasks.get(&session_id) else {
            return Err(AppError::new(
                "session.pending_compact.no_active_run",
                "Session has no active run for queued compaction.",
            )
            .operation("compact")
            .retryable(true));
        };
        if task.run_id != run_id {
            return Err(AppError::new(
                "session.pending_compact.run_mismatch",
                "Queued compaction targets a stale run.",
            )
            .detail(format!(
                "expected active run {}, got {}",
                task.run_id, run_id
            ))
            .operation("compact")
            .retryable(true));
        }
    }

    let run =
        runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id).active_run;
    if !matches!(
        run.status.as_str(),
        "queued" | "starting" | "running" | "waiting_input" | "finishing"
    ) {
        return Err(AppError::new(
            "session.pending_compact.run_closed",
            "The active run is no longer accepting queued compaction.",
        )
        .detail(format!("run {} status {}", run_id, run.status))
        .operation("compact")
        .retryable(true));
    }

    let mut queue = pending_input_queue.lock().map_err(|error| {
        AppError::new(
            "session.pending_compact.lock_failed",
            "Pending compaction queue is unavailable.",
        )
        .detail(error.to_string())
        .operation("compact")
        .retryable(true)
    })?;
    queue.queue_compact(&session_id, &run_id);
    Ok(true)
}

#[tauri::command]
pub async fn insert_pending_chat_input(
    session_id: String,
    run_id: String,
    pending_input_id: Option<String>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<PendingSessionInput, AppError> {
    {
        let tasks = active_tasks.lock().await;
        let Some(task) = tasks.get(&session_id) else {
            return Err(AppError::new(
                "session.pending_input.no_active_run",
                "Session has no active run for queued input.",
            )
            .operation("chat")
            .retryable(true));
        };
        if task.run_id != run_id {
            return Err(AppError::new(
                "session.pending_input.run_mismatch",
                "Queued input targets a stale run.",
            )
            .detail(format!(
                "expected active run {}, got {}",
                task.run_id, run_id
            ))
            .operation("chat")
            .retryable(true));
        }
    }

    let run =
        runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id).active_run;
    if !matches!(
        run.status.as_str(),
        "queued" | "starting" | "running" | "waiting_input"
    ) {
        return Err(AppError::new(
            "session.pending_input.run_closed",
            "The active run is no longer accepting queued input.",
        )
        .detail(format!("run {} status {}", run_id, run.status))
        .operation("chat")
        .retryable(true));
    }

    let pending = {
        let mut queue = pending_input_queue.lock().map_err(|e| {
            AppError::new(
                "session.pending_input.lock_failed",
                "Pending input queue is unavailable.",
            )
            .detail(e.to_string())
            .operation("chat")
            .retryable(true)
        })?;
        queue.promote_to_immediate(&session_id, &run_id, pending_input_id.as_deref())
    };
    let Some(pending) = pending else {
        return Err(AppError::new(
            "session.pending_input.not_found",
            "Queued input was not found for the active run.",
        )
        .operation("chat")
        .retryable(true));
    };

    emit_session_stream_with_run_id(
        &app_handle,
        store.inner().as_ref(),
        run_id,
        StreamEvent::PendingInputQueued {
            session_id,
            input: pending.clone(),
        },
    );

    Ok(pending)
}

#[tauri::command]
pub async fn delete_pending_chat_input(
    session_id: String,
    run_id: String,
    pending_input_id: Option<String>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
) -> Result<bool, AppError> {
    let deleted = {
        let mut queue = pending_input_queue.lock().map_err(|e| {
            AppError::new(
                "session.pending_input.lock_failed",
                "Pending input queue is unavailable.",
            )
            .detail(e.to_string())
            .operation("chat")
            .retryable(true)
        })?;
        queue.delete_input(&session_id, &run_id, pending_input_id.as_deref())
    };

    let Some(deleted) = deleted else {
        return Ok(false);
    };

    emit_session_stream_with_run_id(
        &app_handle,
        store.inner().as_ref(),
        run_id,
        StreamEvent::PendingInputDeleted {
            session_id,
            pending_input_id: deleted.id,
        },
    );

    Ok(true)
}

#[tauri::command]
pub async fn save_session_execution_state(
    session_id: String,
    model_id: String,
    effort: Option<String>,
    fast_mode: bool,
    multi_agent_enabled: Option<bool>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    store
        .set_session_execution_state(&session_id, &model_id, effort.as_deref(), fast_mode, multi_agent_enabled)
        .map_err(|error| {
            AppError::new(
                "session.execution_state_persist_failed",
                "Failed to save the session execution state.",
            )
            .detail(error)
            .operation("saveSessionExecutionState")
        })
}

#[tauri::command]
pub async fn load_session(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<SessionDetail, AppError> {
    let store_handle = store.inner().clone();
    let load_session_id = session_id.clone();
    let mut detail =
        tokio::task::spawn_blocking(move || store_handle.load_session(&load_session_id))
            .await
            .map_err(|error| {
                AppError::new("session.load.join_failed", "Failed to load the session.")
                    .detail(error.to_string())
                    .operation("loadSession")
            })?
            .map_err(AppError::from)?;
    detail.pending_inputs = pending_input_queue
        .lock()
        .map_err(|e| {
            AppError::new(
                "session.pending_input.lock_failed",
                "Pending input queue is unavailable.",
            )
            .detail(e.to_string())
            .operation("loadSession")
        })?
        .list_session(&session_id);
    if let Some(run_id) = active_task_run_id(active_tasks.inner(), &session_id).await {
        let mut runtime =
            runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id);
        runtime.compact_queued = pending_input_queue
            .lock()
            .map_err(|error| {
                AppError::new(
                    "session.pending_compact.lock_failed",
                    "Pending compaction queue is unavailable.",
                )
                .detail(error.to_string())
                .operation("loadSession")
            })?
            .has_compact(&session_id, &run_id);
        detail.runtime = Some(runtime);
    } else {
        store.clear_runtime_session(&session_id);
    }
    Ok(detail)
}

#[tauri::command]
pub async fn load_session_view(
    session_id: String,
    message_limit: Option<u32>,
    store: State<'_, Arc<SessionStore>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<SessionViewSnapshot, AppError> {
    let store_handle = store.inner().clone();
    let load_session_id = session_id.clone();
    let limit = message_limit.unwrap_or(DEFAULT_SESSION_VIEW_MESSAGE_LIMIT);
    let mut snapshot = tokio::task::spawn_blocking(move || {
        store_handle.load_session_view(&load_session_id, limit)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "session.view_load.join_failed",
            "Failed to load the session view.",
        )
        .detail(error.to_string())
        .operation("loadSessionView")
    })?
    .map_err(AppError::from)?;

    snapshot.session.pending_inputs = pending_input_queue
        .lock()
        .map_err(|error| {
            AppError::new(
                "session.pending_input.lock_failed",
                "Pending input queue is unavailable.",
            )
            .detail(error.to_string())
            .operation("loadSessionView")
        })?
        .list_session(&session_id);
    if let Some(run_id) = active_task_run_id(active_tasks.inner(), &session_id).await {
        let mut runtime =
            runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id);
        runtime.compact_queued = pending_input_queue
            .lock()
            .map_err(|error| {
                AppError::new(
                    "session.pending_compact.lock_failed",
                    "Pending compaction queue is unavailable.",
                )
                .detail(error.to_string())
                .operation("loadSessionView")
            })?
            .has_compact(&session_id, &run_id);
        snapshot.session.runtime = Some(runtime);
    } else {
        store.clear_runtime_session(&session_id);
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn load_session_message_page(
    session_id: String,
    before_row_id: i64,
    message_limit: Option<u32>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<SessionMessagePage, AppError> {
    let store_handle = store.inner().clone();
    let limit = message_limit.unwrap_or(DEFAULT_SESSION_VIEW_MESSAGE_LIMIT);
    tokio::task::spawn_blocking(move || {
        store_handle.load_session_message_page(&session_id, before_row_id, limit)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "session.history_load.join_failed",
            "Failed to load older session history.",
        )
        .detail(error.to_string())
        .operation("loadSessionHistory")
    })?
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn load_session_message_images(
    message_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<ImageData>, AppError> {
    let message_id = message_id.trim().to_string();
    if message_id.is_empty() {
        return Err(AppError::new(
            "session.message_images.invalid_target",
            "A message is required to load its images.",
        )
        .operation("loadSessionMessageImages"));
    }
    let store_handle = store.inner().clone();
    tokio::task::spawn_blocking(move || store_handle.load_session_message_images(&message_id))
        .await
        .map_err(|error| {
            AppError::new(
                "session.message_images.join_failed",
                "Failed to load session message images.",
            )
            .detail(error.to_string())
            .operation("loadSessionMessageImages")
        })?
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn load_session_turn_preview(
    session_id: String,
    message_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<SessionTurnPreview, AppError> {
    let store_handle = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        store_handle.load_session_turn_preview(&session_id, &message_id)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "session.turn_preview.join_failed",
            "Failed to load the user turn preview.",
        )
        .detail(error.to_string())
        .operation("loadSessionTurnPreview")
    })?
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn get_compacted_context_output(
    session_id: String,
    message_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<CompactedContextOutput, AppError> {
    let session_id = session_id.trim();
    let message_id = message_id.trim();
    if session_id.is_empty() || message_id.is_empty() {
        return Err(AppError::new(
            "session.compacted_context.invalid_target",
            "A session and compacted message are required.",
        )
        .operation("getCompactedContextOutput"));
    }

    store
        .get_compacted_context_output(session_id, message_id)
        .map_err(|error| {
            AppError::new(
                "session.compacted_context.read_failed",
                "Failed to read the compacted context.",
            )
            .detail(error)
            .operation("getCompactedContextOutput")
        })?
        .ok_or_else(|| {
            AppError::new(
                "session.compacted_context.not_found",
                "The compacted context is no longer available.",
            )
            .operation("getCompactedContextOutput")
        })
}

#[tauri::command]
pub async fn undo_latest_conversation_turn(
    session_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<SessionDetail, AppError> {
    let scope = resolve_optional_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        "undo_latest_conversation_turn",
    )?;
    let working_dir = scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .unwrap_or_default();
    let event_scope = scope.as_ref().map(|scope| {
        crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime())
    });
    let deleted = store
        .truncate_latest_conversation_turn(&session_id)
        .map_err(AppError::from)?;
    if deleted == 0 {
        return Err(AppError::new(
            "session.undo.empty",
            "No conversation round is available to undo.",
        )
        .operation("undo"));
    }
    crate::llm::codex::reset_cached_session_window(&session_id).await;
    let detail = store.load_session(&session_id).map_err(AppError::from)?;
    super::emit_session_content_changed(
        &app_handle,
        event_scope.as_ref(),
        &working_dir,
        &session_id,
        "undo_latest_conversation_turn",
    );
    Ok(detail)
}

#[tauri::command]
pub async fn rollback_session_to_message(
    session_id: String,
    message_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<SessionDetail, AppError> {
    let scope = resolve_optional_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        "rollback_session_to_message",
    )?;
    let working_dir = scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .unwrap_or_default();
    let event_scope = scope.as_ref().map(|scope| {
        crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime())
    });
    store
        .truncate_after_message(&session_id, &message_id)
        .map_err(AppError::from)?;
    crate::llm::codex::reset_cached_session_window(&session_id).await;
    let detail = store.load_session(&session_id).map_err(AppError::from)?;
    super::emit_session_content_changed(
        &app_handle,
        event_scope.as_ref(),
        &working_dir,
        &session_id,
        "rollback_session_to_message",
    );
    Ok(detail)
}

async fn populate_session_runtime_statuses(
    sessions: &mut [SessionSummary],
    store: &SessionStore,
    active_tasks: &ActiveTasks,
) {
    let active_session_runs: HashMap<String, String> = active_tasks
        .lock()
        .await
        .iter()
        .map(|(session_id, task)| (session_id.clone(), task.run_id.clone()))
        .collect();
    for session in sessions {
        session.runtime_status = if let Some(run_id) = active_session_runs.get(&session.id) {
            let snapshot = store.runtime_snapshot_for_session(&session.id);
            snapshot
                .filter(|snapshot| snapshot.active_run.run_id == *run_id)
                .map(|snapshot| runtime_status_from_run_status(&snapshot.active_run.status))
                .or(Some(SessionRuntimeStatus::Running))
        } else {
            store.clear_runtime_session(&session.id);
            None
        };
    }
}

#[tauri::command]
pub async fn list_sessions(
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Vec<SessionSummary>, AppError> {
    let mut sessions = store.list_sessions(None).map_err(AppError::from)?;
    populate_session_runtime_statuses(&mut sessions, store.inner().as_ref(), active_tasks.inner())
        .await;
    Ok(sessions)
}

#[tauri::command]
pub async fn list_checkout_sessions(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Vec<SessionSummary>, AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "listCheckoutSessions",
    )?;
    let project = workspace_registry
        .project(scope.runtime().project_id())
        .ok_or_else(|| {
            AppError::new(
                "workspace.project_unavailable",
                "The checkout project context is unavailable.",
            )
        })?;
    let mut sessions = project.sessions().list().map_err(AppError::from)?;
    populate_session_runtime_statuses(&mut sessions, store.inner().as_ref(), active_tasks.inner())
        .await;
    Ok(sessions)
}

#[tauri::command]
pub async fn list_project_sessions(
    project_id: String,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Vec<SessionSummary>, AppError> {
    let project_id = crate::workspace_service::ProjectId::new(project_id).map_err(|error| {
        AppError::new(
            "workspace.project_identity_invalid",
            "The project identity is invalid.",
        )
        .detail(error.to_string())
        .operation("listProjectSessions")
    })?;
    let project = workspace_registry.project(&project_id).ok_or_else(|| {
        AppError::new(
            "workspace.project_unavailable",
            "The project context is unavailable.",
        )
        .detail(project_id.to_string())
        .operation("listProjectSessions")
    })?;
    let mut sessions = project.sessions().list().map_err(AppError::from)?;
    populate_session_runtime_statuses(&mut sessions, store.inner().as_ref(), active_tasks.inner())
        .await;
    Ok(sessions)
}

#[tauri::command]
pub async fn list_archived_sessions(
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<SessionSummary>, AppError> {
    store.list_archived_sessions(None).map_err(Into::into)
}

#[tauri::command]
pub async fn list_archived_checkout_sessions(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Vec<SessionSummary>, AppError> {
    let scope = resolve_workspace_scope(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "listArchivedCheckoutSessions",
    )?;
    let project = workspace_registry
        .project(scope.runtime().project_id())
        .ok_or_else(|| {
            AppError::new(
                "workspace.project_unavailable",
                "The checkout project context is unavailable.",
            )
        })?;
    let mut sessions = project.sessions().list_archived().map_err(AppError::from)?;
    populate_session_runtime_statuses(&mut sessions, store.inner().as_ref(), active_tasks.inner())
        .await;
    Ok(sessions)
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
    undo_manager: State<'_, crate::UndoManagerHandle>,
) -> Result<(), AppError> {
    store.delete_session(&session_id).map_err(AppError::from)?;
    crate::llm::codex::invalidate_cached_session(&session_id);
    undo_manager.on_session_delete(&session_id).await;
    // The delete cascades to messages/events; reclaim file space in the
    // background once enough of the database is dead freelist pages.
    store.inner().clone().spawn_vacuum_if_fragmented();
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
pub async fn get_session_context_usage_report(
    session_id: String,
    model_id: Option<String>,
    knowledge_mode: Option<String>,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, AgentDefRegistryState>,
    workspace_definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    workspace_tools: State<'_, Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>,
    config: State<'_, Arc<AppConfig>>,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
    api_key_state: State<'_, ApiKeyState>,
    codex: State<'_, CodexAuthStateHandle>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    raw_store: State<'_, RawContextStore>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<crate::commands::SessionContextUsageReport, AppError> {
    let detail = store.load_session(&session_id)?;
    let workspace_scope = resolve_optional_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        "get_session_context_usage_report",
    )?;
    let registry_snapshot = match workspace_scope.as_ref() {
        Some(scope) => workspace_definitions
            .snapshot(scope.runtime().as_ref())
            .await
            .map_err(|detail| {
                AppError::new(
                    "agent.workspace_definitions_unavailable",
                    "Failed to load Agent definitions for this checkout.",
                )
                .detail(detail)
                .operation("get_session_context_usage_report")
            })?,
        None => registry.snapshot().await,
    };
    let def = detail
        .agent_id
        .as_deref()
        .map(canonical_agent_id)
        .and_then(|agent_id| registry_snapshot.get(agent_id).cloned())
        .or_else(|| registry_snapshot.default_def().cloned())
        .ok_or_else(|| "No agent definitions found".to_string())?;
    let selected_model = model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(detail.last_model_id.as_deref())
        .ok_or_else(|| "This session has no model usage yet".to_string())?
        .to_string();
    let backend = resolve_model_backend(
        &selected_model,
        config.inner().as_ref(),
        auth.inner(),
        api_key_state.inner(),
        codex.inner(),
    )
    .await?;
    let working_dir = workspace_scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .unwrap_or_default();
    let workspace_id = workspace_scope
        .as_ref()
        .map(|scope| scope.runtime().project_id().to_string());
    let knowledge_access_mode = KnowledgeAccessMode::from_request(knowledge_mode.as_deref())
        .map_err(|error| AppError::new("session.invalid_knowledge_mode", error))?;
    let prompt_messages = store.get_messages_for_prompt(&session_id)?;
    let session_messages = store.get_messages(&session_id)?;
    let usage = store.get_token_usage(&session_id)?;
    let cache_invalidations = store.list_cache_invalidations(&session_id)?;
    let effective_tool_registry = match workspace_scope.as_ref() {
        Some(scope) => workspace_tools
            .snapshot(scope.runtime().as_ref(), registry_snapshot.as_ref())
            .await
            .map_err(|detail| {
                AppError::new(
                    "tool.workspace_definitions_unavailable",
                    "Failed to load tool definitions for this checkout.",
                )
                .detail(detail)
                .operation("get_session_context_usage_report")
            })?,
        None => tool_registry.inner().clone(),
    };
    let execution_context = match workspace_scope.as_ref() {
        Some(scope) => Some(
            workspace_registry
                .execution_context(scope.runtime().checkout_id(), &[])
                .await
                .map_err(|detail| {
                    AppError::new(
                        "session.workspace_execution_unavailable",
                        "Failed to bind the context report to its checkout runtime.",
                    )
                    .detail(detail)
                    .operation("get_session_context_usage_report")
                })?,
        ),
        None => None,
    };

    let mut instance = AgentInstance::new(
        Arc::new(def),
        &session_id,
        backend,
        config.debug_enabled(),
        registry_snapshot,
        effective_tool_registry,
        working_dir,
        raw_store.inner().clone(),
        workspace_id,
        selected_model,
        detail.last_effort,
        app_knowledge_dir.0.clone(),
        app_agent_dir.0.clone(),
        knowledge_access_mode,
        None,
        HashMap::new(),
        tokio::sync::watch::channel(false).1,
    );
    if let Some(execution_context) = execution_context {
        instance.set_execution_context(execution_context);
    }
    instance.set_async_tasks_enabled(config.async_tasks_enabled());

    instance.set_multi_agent_enabled(detail.last_multi_agent_enabled.unwrap_or(false));
    Ok(instance
        .session_context_usage_report(
            &app_handle,
            &prompt_messages,
            &session_messages,
            detail.title,
            cache_invalidations,
            usage,
        )
        .await)
}

#[tauri::command]
pub async fn get_model_usage_stats(
    days: Option<u32>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<crate::commands::ModelUsageReport, AppError> {
    if days.is_some_and(|value| value == 0 || value > 3650) {
        return Err("Usage statistics range must be between 1 and 3650 days".into());
    }
    store.get_model_usage_report(days).map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_active_run(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<Option<SessionRunSummary>, AppError> {
    let Some(run_id) = active_task_run_id(active_tasks.inner(), &session_id).await else {
        store.clear_runtime_session(&session_id);
        return Ok(None);
    };
    Ok(Some(
        runtime_snapshot_for_active_task(store.inner().as_ref(), &session_id, &run_id).active_run,
    ))
}

#[tauri::command]
pub async fn get_session_resume_available(
    session_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<bool, AppError> {
    store
        .session_resume_available(&session_id)
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

fn emit_cancelled_session_run(
    app_handle: &AppHandle,
    store: &SessionStore,
    session_id: String,
    run_id: String,
    interrupted: Option<crate::agent::instance::InterruptedAssistantMessage>,
) {
    emit_session_stream_with_run_id(
        app_handle,
        store,
        run_id,
        StreamEvent::Cancelled {
            session_id,
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
            removed_user_message: None,
        },
    );
}

async fn request_descendant_cancellations(
    root_session_id: &str,
    app_handle: &AppHandle,
    store: &SessionStore,
    active_tasks: &ActiveTasks,
) {
    let descendant_runs = match store.active_descendant_runs(root_session_id) {
        Ok(runs) => runs,
        Err(error) => {
            eprintln!(
                "[Locus] failed to query active descendant runs for session {}: {}",
                root_session_id, error
            );
            return;
        }
    };

    if descendant_runs.is_empty() {
        return;
    }

    let run_by_session: HashMap<String, String> = descendant_runs
        .iter()
        .map(|run| (run.session_id.clone(), run.run_id.clone()))
        .collect();

    let async_tasks = app_handle.state::<Arc<crate::async_tasks::AsyncTaskManager>>();
    for run in &descendant_runs {
        async_tasks.cancel_session(&run.session_id);
    }

    for run in &descendant_runs {
        if let Err(error) = store.update_run_status(
            &run.run_id,
            crate::session::gateway::RUN_STATUS_CANCELLING,
            None,
        ) {
            eprintln!(
                "[Locus] failed to mark descendant session {} run {} as cancelling: {}",
                run.session_id, run.run_id, error
            );
        }
    }

    let tasks = active_tasks.lock().await;
    for (session_id, run_id) in run_by_session {
        if let Some(task) = tasks.get(&session_id) {
            if task.run_id == run_id {
                let _ = task.cancel_tx.send(true);
            }
        }
        crate::process_util::terminate_managed_processes_for_session(&session_id);
    }
}

async fn finish_cancelled_descendant_runs(
    root_session_id: &str,
    app_handle: &AppHandle,
    store: &SessionStore,
    active_tasks: &ActiveTasks,
) {
    let descendant_runs = match store.active_descendant_runs(root_session_id) {
        Ok(runs) => runs,
        Err(error) => {
            eprintln!(
                "[Locus] failed to query active descendant runs for session {} during cancellation finish: {}",
                root_session_id, error
            );
            return;
        }
    };

    if descendant_runs.is_empty() {
        return;
    }

    let mut removed_tasks = Vec::new();
    {
        let mut tasks = active_tasks.lock().await;
        for run in &descendant_runs {
            let remove = tasks
                .get(&run.session_id)
                .map(|task| task.run_id == run.run_id)
                .unwrap_or(false);
            if remove {
                if let Some(task) = tasks.remove(&run.session_id) {
                    removed_tasks.push((run.session_id.clone(), run.run_id.clone(), task));
                }
            }
        }
    }

    let mut interrupted_by_run = HashMap::new();
    for (session_id, run_id, task) in removed_tasks {
        let interrupted = AgentInstance::persist_interrupted_assistant_snapshot(
            store,
            &session_id,
            &task.partial_assistant.snapshot(),
        );
        task.partial_assistant.reset();
        task.join_handle.abort();
        crate::llm::codex::reset_cached_session_window(&session_id).await;
        interrupted_by_run.insert(run_id, interrupted);
    }

    for run in descendant_runs {
        let interrupted = interrupted_by_run.remove(&run.run_id).flatten();
        eprintln!(
            "[Locus] emitting descendant cancellation for session {} run {} under parent {}",
            run.session_id, run.run_id, root_session_id
        );
        crate::llm::codex::reset_cached_session_window(&run.session_id).await;
        emit_cancelled_session_run(app_handle, store, run.session_id, run.run_id, interrupted);
    }
}

#[tauri::command]
pub async fn cancel_chat(
    session_id: String,
    app_handle: AppHandle,
    store: State<'_, Arc<SessionStore>>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<(), AppError> {
    let async_tasks = app_handle.state::<Arc<crate::async_tasks::AsyncTaskManager>>();
    async_tasks.cancel_session(&session_id);

    let graceful_wait = {
        let tasks = active_tasks.lock().await;
        tasks.get(&session_id).map(|task| {
            let _ = task.cancel_tx.send(true);
            (task.run_id.clone(), task.done_rx.clone())
        })
    };
    crate::process_util::terminate_managed_processes_for_session(&session_id);

    let Some((run_id, mut done_rx)) = graceful_wait else {
        finish_cancelled_descendant_runs(
            &session_id,
            &app_handle,
            store.inner().as_ref(),
            active_tasks.inner(),
        )
        .await;
        return Ok(());
    };

    if *done_rx.borrow() {
        finish_cancelled_descendant_runs(
            &session_id,
            &app_handle,
            store.inner().as_ref(),
            active_tasks.inner(),
        )
        .await;
        return Ok(());
    }

    request_descendant_cancellations(
        &session_id,
        &app_handle,
        store.inner().as_ref(),
        active_tasks.inner(),
    )
    .await;

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
        finish_cancelled_descendant_runs(
            &session_id,
            &app_handle,
            store.inner().as_ref(),
            active_tasks.inner(),
        )
        .await;
        return Ok(());
    }

    let handle = active_tasks.lock().await.remove(&session_id);
    if let Some(task) = handle {
        let interrupted = AgentInstance::persist_interrupted_assistant_snapshot(
            store.inner().as_ref(),
            &session_id,
            &task.partial_assistant.snapshot(),
        );
        task.partial_assistant.reset();
        task.join_handle.abort();
        eprintln!(
            "[Locus] cancellation timed out; aborted task for session {}",
            session_id
        );
        crate::llm::codex::reset_cached_session_window(&session_id).await;
        emit_cancelled_session_run(
            &app_handle,
            store.inner().as_ref(),
            session_id.clone(),
            run_id,
            interrupted,
        );
    }

    finish_cancelled_descendant_runs(
        &session_id,
        &app_handle,
        store.inner().as_ref(),
        active_tasks.inner(),
    )
    .await;

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
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
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

    let scope = resolve_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        None,
        "apply_knowledge_proposal",
    )?;
    let working_dir = scope.runtime().root().to_string_lossy().to_string();
    let knowledge_index_state = scope
        .runtime()
        .knowledge_index(&app_handle)
        .map_err(|detail| {
            AppError::new(
                "knowledge.index_unavailable",
                "Failed to initialize the checkout knowledge index.",
            )
            .detail(detail)
            .operation("apply_knowledge_proposal")
        })?;

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
            knowledge_index_state,
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
pub async fn export_session_context(
    session_id: String,
    file_path: Option<String>,
    raw_store: State<'_, RawContextStore>,
    store: State<'_, Arc<SessionStore>>,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    pending_input_queue: State<'_, PendingInputQueueHandle>,
    active_tasks: State<'_, ActiveTasks>,
) -> Result<crate::session::context_export::ContextExportResult, AppError> {
    let workspace_scope = resolve_optional_session_workspace_scope(
        store.inner(),
        workspace_registry.inner(),
        &session_id,
        "export_session_context",
    )?;
    let working_dir = workspace_scope
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
        .unwrap_or_default();
    let output_path = match file_path {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => {
            let session_title = store.load_session(&session_id)?.title;
            crate::session::context_export::default_review_export_path(
                &super::app_temp_dir()?,
                &session_id,
                &session_title,
            )?
        }
    };
    let legacy_rounds = {
        let raw = raw_store.lock().await;
        raw.get(&session_id)
            .filter(|rounds| !rounds.is_empty())
            .cloned()
    };
    let live_store = store.inner().clone();
    let snapshot_store = tokio::task::spawn_blocking(move || live_store.create_export_snapshot())
        .await
        .map_err(|error| {
            AppError::new(
                "session.export_snapshot_task_failed",
                "Failed to create a session export snapshot.",
            )
            .detail(error.to_string())
            .operation("exportSessionContext")
        })??;
    let session_tree_ids = snapshot_store.session_tree_ids(&session_id)?;
    let live_snapshot = capture_context_export_live_snapshot(
        &session_tree_ids,
        store.inner().as_ref(),
        pending_input_queue.inner(),
        active_tasks.inner(),
    )
    .await?;

    tokio::task::spawn_blocking(move || {
        let _workspace_scope = workspace_scope;
        crate::session::context_export::export_session_context_yaml(
            &snapshot_store,
            &session_id,
            &working_dir,
            legacy_rounds.as_deref(),
            Some(&live_snapshot),
            &output_path,
        )
    })
    .await
    .map_err(|error| {
        AppError::new(
            "session.export_write_task_failed",
            "Failed to write the session context export.",
        )
        .detail(error.to_string())
        .operation("exportSessionContext")
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn answer_question(
    question_id: String,
    answer: String,
    app_handle: AppHandle,
    question_store: State<'_, QuestionStore>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<(), AppError> {
    let pending = {
        let mut store = question_store.lock().await;
        store.remove(&question_id)
    };
    match pending {
        Some(pending) => {
            let crate::PendingQuestionResponse {
                session_id,
                run_id,
                tx,
            } = pending;

            tx.send(answer)
                .map_err(|_| "Question receiver dropped".to_string())?;

            crate::session::gateway::emit_stream(
                &app_handle,
                store.inner().as_ref(),
                &run_id,
                StreamEvent::InputAnswered {
                    session_id,
                    question_id,
                },
            );

            Ok(())
        }
        None => Err(format!("Question '{}' not found or already answered", question_id).into()),
    }
}

#[cfg(test)]
mod workspace_definition_scope_tests {
    use super::{agent_definition_source, list_agent_infos, AgentDefinitionSource};
    use crate::agent::definition::AgentDefRegistry;
    use std::path::PathBuf;

    fn repo_agent_registry() -> AgentDefRegistry {
        let agent_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agent");
        AgentDefRegistry::load(Some(agent_dir.as_path()), None)
    }

    #[test]
    fn checkout_sessions_always_use_checkout_definitions() {
        assert_eq!(
            agent_definition_source(true, true, true),
            AgentDefinitionSource::Checkout
        );
    }

    #[test]
    fn historical_sessions_without_checkout_keep_the_legacy_registry() {
        assert_eq!(
            agent_definition_source(true, false, true),
            AgentDefinitionSource::LegacyGlobal
        );
        assert_eq!(
            agent_definition_source(true, false, false),
            AgentDefinitionSource::LegacyGlobal
        );
    }

    #[test]
    fn new_workspace_sessions_use_checkout_definitions() {
        assert_eq!(
            agent_definition_source(false, false, true),
            AgentDefinitionSource::Checkout
        );
        assert_eq!(
            agent_definition_source(false, false, false),
            AgentDefinitionSource::LegacyGlobal
        );
    }

    #[test]
    fn workspace_project_type_selects_the_compatible_builtin_default() {
        let registry = repo_agent_registry();
        let generic = list_agent_infos(&registry, Some("generic"));
        let unity = list_agent_infos(&registry, Some("unity"));

        assert_eq!(
            generic
                .iter()
                .find(|agent| agent.is_default)
                .map(|agent| agent.id.as_str()),
            Some("simple")
        );
        assert_eq!(
            unity
                .iter()
                .find(|agent| agent.is_default)
                .map(|agent| agent.id.as_str()),
            Some("unity")
        );
    }
}
