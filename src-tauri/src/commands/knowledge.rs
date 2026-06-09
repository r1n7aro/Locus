use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::agent::definition::canonical_agent_id;
use crate::binary_cache::BinaryCache;
use crate::error::{AppError, ErrorSeverity};
use crate::feishu_docs::{
    self, FeishuReferenceConfigInput, FeishuReferenceConnectionTestResult,
    FeishuReferenceImportRequest, FeishuReferenceImportState, FeishuReferenceImportStatus,
    FeishuReferenceNodeSummary, FeishuReferenceOauthStartResult,
};
use crate::knowledge_index::{
    self, EmbeddingActivationBackfillStrategy, EmbeddingConfig, EmbeddingLocalModelCatalog,
    EmbeddingLocalModelDirectoryInspection, EmbeddingRuntimeTestResult, EmbeddingStatus,
    KnowledgeGeneralConfig, KnowledgeIndexState, KnowledgeOverview, LexicalRebuildStatus,
};
use crate::knowledge_store::{
    self, KnowledgeCreateRequest, KnowledgeDeleteRequest, KnowledgeDirectoryConfig,
    KnowledgeDirectoryConfigPatch, KnowledgeDirectoryConfigRecord, KnowledgeDocumentPatch,
    KnowledgeEditRequest, KnowledgeExternalDirectoryBinding, KnowledgeInjectMode,
    KnowledgeListItem, KnowledgeMoveRequest, KnowledgeMutationResponse, KnowledgeReadRequest,
    KnowledgeReadResponse, KnowledgeSearchHit, KnowledgeSourceProvider, KnowledgeTargetKind,
    KnowledgeType, KnowledgeUpdateOp, KnowledgeUpdateRequest, SkillSurface,
};
use crate::tool::ToolRegistry;
use crate::unity_docs::{
    self, UnityManagedDirectoryStat, UnityReferenceImportState, UnityReferenceImportStatus,
};
use crate::workspace::Workspace;
use crate::AgentDefRegistryState;

#[derive(Clone)]
pub struct AppKnowledgeDir(pub Arc<Option<std::path::PathBuf>>);

pub const KNOWLEDGE_CHANGED_EVENT: &str = "knowledge-changed";
pub const KNOWLEDGE_DOWNLOAD_WINDOW_LABEL: &str = "knowledge-download-progress";
pub const KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_LABEL: &str = "knowledge-lexical-progress";
pub const UNITY_REFERENCE_IMPORT_WINDOW_LABEL: &str = "unity-reference-import-progress";
pub const FEISHU_REFERENCE_IMPORT_WINDOW_LABEL: &str = "feishu-reference-import-progress";
const DEFAULT_KNOWLEDGE_PAGE_SIZE: usize = 200;
const MAX_KNOWLEDGE_PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChangedEvent {
    pub working_dir: String,
    pub source: String,
    pub changed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub subtree: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct KnowledgeChangedTarget {
    pub doc_type: Option<KnowledgeType>,
    pub path: Option<String>,
    pub parent_path: Option<String>,
    pub target_kind: Option<&'static str>,
    pub change_kind: Option<&'static str>,
    pub subtree: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListPageResponse {
    pub items: Vec<KnowledgeListItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

pub(crate) fn emit_knowledge_changed(app_handle: &AppHandle, working_dir: &str, source: &str) {
    emit_knowledge_changed_with_target(
        app_handle,
        working_dir,
        source,
        KnowledgeChangedTarget::default(),
    );
}

pub(crate) fn emit_knowledge_changed_with_target(
    app_handle: &AppHandle,
    working_dir: &str,
    source: &str,
    target: KnowledgeChangedTarget,
) {
    let payload = KnowledgeChangedEvent {
        working_dir: working_dir.to_string(),
        source: source.to_string(),
        changed_at: chrono::Utc::now().timestamp_millis(),
        doc_type: target.doc_type.map(|value| value.as_str().to_string()),
        path: target.path,
        parent_path: target.parent_path,
        target_kind: target.target_kind.map(str::to_string),
        change_kind: target.change_kind.map(str::to_string),
        subtree: target.subtree,
    };
    if let Err(error) = app_handle.emit(KNOWLEDGE_CHANGED_EVENT, payload) {
        eprintln!(
            "[Knowledge] failed to emit {} event for {}: {}",
            KNOWLEDGE_CHANGED_EVENT, source, error
        );
    }
}

pub(crate) async fn reconcile_and_emit_knowledge_changed(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    source: &str,
) -> Result<(), AppError> {
    if working_dir.trim().is_empty() {
        return Ok(());
    }

    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    knowledge_index::reconcile_workspace(
        working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        knowledge_index_state,
    )
    .await
    .map_err(AppError::from)?;
    emit_knowledge_changed(app_handle, working_dir, source);
    Ok(())
}

fn remove_shadowed_documents_for_path(
    knowledge_index_state: Arc<KnowledgeIndexState>,
    doc_type: KnowledgeType,
    path: &str,
    keep_doc_id: Option<&str>,
) -> Result<(), AppError> {
    knowledge_index::remove_shadowed_documents_for_path(
        knowledge_index_state,
        doc_type,
        path,
        keep_doc_id,
    )
    .map_err(AppError::from)
}

async fn restore_visible_document_for_path(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<(), AppError> {
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    let Ok(document) = knowledge_store::load_document_by_path_with_app_root(
        working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        doc_type,
        path,
    ) else {
        return Ok(());
    };
    remove_shadowed_documents_for_path(
        knowledge_index_state.clone(),
        doc_type,
        path,
        Some(&document.id),
    )?;
    knowledge_index::upsert_document(
        knowledge_index_state,
        working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        document,
    )
    .await
    .map_err(AppError::from)
}

pub(crate) async fn sync_visible_document_for_path(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<(), AppError> {
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    match knowledge_store::load_document_by_path_with_app_root(
        working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        doc_type,
        path,
    ) {
        Ok(document) => {
            remove_shadowed_documents_for_path(
                knowledge_index_state.clone(),
                doc_type,
                path,
                Some(&document.id),
            )?;
            knowledge_index::upsert_document(
                knowledge_index_state,
                working_dir,
                app_knowledge_dir.0.as_ref().as_ref(),
                document,
            )
            .await
            .map_err(AppError::from)?;
        }
        Err(_) => {
            remove_shadowed_documents_for_path(knowledge_index_state, doc_type, path, None)?;
        }
    }
    Ok(())
}

pub(crate) async fn sync_visible_documents_for_paths_and_emit(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    source: &str,
    targets: &[(KnowledgeType, String)],
) -> Result<(), AppError> {
    for (doc_type, path) in targets {
        sync_visible_document_for_path(
            app_handle,
            working_dir,
            knowledge_index_state.clone(),
            *doc_type,
            path,
        )
        .await?;
    }
    emit_knowledge_changed(app_handle, working_dir, source);
    Ok(())
}

pub(crate) async fn sync_visible_documents_for_prefix(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    doc_type: KnowledgeType,
    path_prefix: &str,
) -> Result<(), AppError> {
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    let app_root = app_knowledge_dir.0.as_ref().as_ref();
    let visible_documents = knowledge_store::load_documents_with_app_root(
        working_dir,
        app_root,
        Some(doc_type),
        Some(path_prefix),
    )
    .map_err(AppError::from)?;
    let existing_rows = knowledge_index_state
        .db()
        .list_document_catalog_entries_with_prefix(doc_type.as_str(), path_prefix)
        .map_err(AppError::from)?;

    let mut visible_paths = HashSet::with_capacity(visible_documents.len());
    for document in visible_documents {
        visible_paths.insert(document.path.clone());
        remove_shadowed_documents_for_path(
            knowledge_index_state.clone(),
            doc_type,
            &document.path,
            Some(&document.id),
        )?;
        knowledge_index::upsert_document(
            knowledge_index_state.clone(),
            working_dir,
            app_root,
            document,
        )
        .await
        .map_err(AppError::from)?;
    }

    let stale_doc_ids = existing_rows
        .into_iter()
        .filter(|row| !visible_paths.contains(&row.doc_path))
        .map(|row| row.doc_id)
        .collect::<Vec<_>>();
    knowledge_index::remove_documents(knowledge_index_state, &stale_doc_ids)
        .map_err(AppError::from)?;
    Ok(())
}

pub fn resolve_app_knowledge_dir(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    // Dev: anchored to Cargo.toml directory (src-tauri/) at compile time
    #[cfg(debug_assertions)]
    {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest_dir.join("..").join("knowledge")); // repo root: knowledge/
    }
    // Prod: app_data_dir or exe_dir
    candidates.push(data_dir.join("knowledge"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("knowledge"));
        }
    }
    candidates.into_iter().find(|p| p.is_dir())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub surface: SkillSurface,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub command_trigger: String,
    #[serde(default)]
    pub inject_mode: KnowledgeInjectMode,
}

fn default_true() -> bool {
    true
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            surface: SkillSurface::Command,
            description: String::new(),
            command_trigger: String::new(),
            inject_mode: KnowledgeInjectMode::None,
        }
    }
}

fn skill_config_path(working_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join("skill_config.json")
}

fn normalize_skill_config_key(rel_path: &str, source: Option<&str>) -> String {
    let normalized = rel_path
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.contains(":skill/") {
        return normalized;
    }

    if let Some(rest) = normalized.strip_prefix("skill/") {
        let skill_path = rest
            .strip_suffix("/SKILL.md")
            .or_else(|| rest.strip_suffix("/SKILL"))
            .or_else(|| rest.strip_suffix(".md"))
            .unwrap_or(rest);
        if !skill_path.trim().is_empty() {
            return format!("{}:skill/{}", source.unwrap_or("project"), skill_path);
        }
    }

    if let Some(source) = source {
        let skill_path = normalized
            .strip_suffix("/SKILL.md")
            .or_else(|| normalized.strip_suffix("/SKILL"))
            .or_else(|| normalized.strip_suffix(".md"))
            .unwrap_or(&normalized);
        if !skill_path.trim().is_empty() {
            return format!("{}:skill/{}", source, skill_path);
        }
    }

    normalized
}

pub fn load_skill_config(working_dir: &str) -> std::collections::HashMap<String, SkillConfig> {
    let path = skill_config_path(working_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => std::collections::HashMap::new(),
    }
}

pub fn save_skill_config(
    working_dir: &str,
    map: &std::collections::HashMap<String, SkillConfig>,
) -> Result<(), String> {
    let path = skill_config_path(working_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    let json =
        serde_json::to_string_pretty(map).map_err(|e| format!("Serialization failed: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save config: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_skill_config(
    rel_path: String,
    source: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<SkillConfig, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let map = load_skill_config(&working_dir);
    let key = normalize_skill_config_key(&rel_path, source.as_deref());
    Ok(map.get(&key).cloned().unwrap_or_default())
}

#[tauri::command]
pub async fn set_skill_config(
    rel_path: String,
    source: Option<String>,
    enabled: bool,
    surface: SkillSurface,
    description: String,
    command_trigger: String,
    inject_mode: Option<KnowledgeInjectMode>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let mut map = load_skill_config(&working_dir);
    let key = normalize_skill_config_key(&rel_path, source.as_deref());
    let existing = map.get(&key).cloned().unwrap_or_default();
    let config = SkillConfig {
        enabled,
        surface,
        description,
        command_trigger: command_trigger.trim().to_string(),
        inject_mode: inject_mode.unwrap_or(existing.inject_mode),
    };
    let fallback = super::skill::fallback_command_name_for_skill_ref(&key);
    let config = super::skill::normalize_and_validate_skill_config(&config, &fallback)?;
    map.insert(key, config);
    save_skill_config(&working_dir, &map).map_err(Into::into)
}

#[tauri::command]
pub async fn get_all_skill_configs(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<std::collections::HashMap<String, SkillConfig>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    Ok(load_skill_config(&working_dir))
}

fn parse_knowledge_type(value: &str) -> Result<KnowledgeType, String> {
    match value.trim() {
        "design" => Ok(KnowledgeType::Design),
        "memory" => Ok(KnowledgeType::Memory),
        "skill" => Ok(KnowledgeType::Skill),
        "reference" => Ok(KnowledgeType::Reference),
        other => Err(format!("Unsupported knowledge type: {}", other)),
    }
}

fn parse_knowledge_type_from_path(path: &str) -> Option<KnowledgeType> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(&normalized);
    if normalized.starts_with("design/") {
        Some(KnowledgeType::Design)
    } else if normalized.starts_with("memory/") {
        Some(KnowledgeType::Memory)
    } else if normalized.starts_with("skill/") {
        Some(KnowledgeType::Skill)
    } else if normalized.starts_with("reference/") {
        Some(KnowledgeType::Reference)
    } else {
        None
    }
}

fn parse_knowledge_type_from_prefix(path: &str) -> Option<KnowledgeType> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized
        .trim()
        .trim_matches('/')
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(normalized.trim())
        .trim_matches('/');

    match normalized.split('/').next() {
        Some("design") => Some(KnowledgeType::Design),
        Some("memory") => Some(KnowledgeType::Memory),
        Some("skill") => Some(KnowledgeType::Skill),
        Some("reference") => Some(KnowledgeType::Reference),
        _ => None,
    }
}

fn normalize_knowledge_path(path: &str) -> Result<String, String> {
    let normalized = path.replace('\\', "/");
    let stripped = normalized
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(&normalized)
        .strip_prefix("design/")
        .or_else(|| normalized.strip_prefix("memory/"))
        .or_else(|| normalized.strip_prefix("skill/"))
        .or_else(|| normalized.strip_prefix("reference/"))
        .unwrap_or(&normalized);
    knowledge_store::ensure_document_path(stripped)
}

pub(crate) fn require_knowledge_document_path_suffix(path: &str) -> Result<(), String> {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized
        .trim_matches('/')
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(normalized.trim_matches('/'))
        .trim_matches('/');
    let stripped = normalized
        .strip_prefix("design/")
        .or_else(|| normalized.strip_prefix("memory/"))
        .or_else(|| normalized.strip_prefix("skill/"))
        .or_else(|| normalized.strip_prefix("reference/"))
        .unwrap_or(normalized)
        .trim_matches('/');

    if stripped.ends_with(".md") {
        Ok(())
    } else {
        Err(
            "knowledge document paths must end with .md; use paths without .md only for directory operations"
                .to_string(),
        )
    }
}

fn normalize_knowledge_directory_path(path: &str) -> Result<String, String> {
    let normalized = path.replace('\\', "/");
    let stripped = normalized
        .trim()
        .trim_matches('/')
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(normalized.trim())
        .trim_matches('/');
    let stripped = stripped
        .strip_prefix("design/")
        .or_else(|| stripped.strip_prefix("memory/"))
        .or_else(|| stripped.strip_prefix("skill/"))
        .or_else(|| stripped.strip_prefix("reference/"))
        .unwrap_or(stripped)
        .trim_matches('/');

    if stripped.is_empty() {
        return Err("knowledge directory path cannot be empty".to_string());
    }
    if stripped.contains("..") || stripped.starts_with('/') || stripped.starts_with('\\') {
        return Err("knowledge directory path must be relative".to_string());
    }
    Ok(stripped.to_string())
}

fn normalize_knowledge_path_prefix(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.contains("..") || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err("knowledge path prefix must be relative to the knowledge root".to_string());
    }

    let normalized = trimmed.replace('\\', "/");
    let normalized = normalized
        .trim_matches('/')
        .strip_prefix("Locus/knowledge/")
        .unwrap_or(&normalized)
        .trim_matches('/');
    if matches!(normalized, "design" | "memory" | "skill" | "reference") {
        return Ok(String::new());
    }
    let normalized = normalized
        .strip_prefix("design/")
        .or_else(|| normalized.strip_prefix("memory/"))
        .or_else(|| normalized.strip_prefix("skill/"))
        .or_else(|| normalized.strip_prefix("reference/"))
        .unwrap_or(normalized)
        .trim_matches('/');
    Ok(normalized.to_string())
}

pub(crate) fn resolve_knowledge_path_filter(
    doc_type_hint: Option<KnowledgeType>,
    raw_prefix: Option<&str>,
) -> Result<(Option<KnowledgeType>, Option<String>), String> {
    let inferred_type = raw_prefix.and_then(parse_knowledge_type_from_prefix);
    let doc_type = match (doc_type_hint, inferred_type) {
        (Some(explicit), Some(inferred)) if explicit != inferred => {
            return Err(
                "knowledge path prefix type does not match the explicit type filter.".to_string(),
            );
        }
        (Some(explicit), _) => Some(explicit),
        (None, inferred) => inferred,
    };

    let normalized_prefix = raw_prefix
        .map(normalize_knowledge_path_prefix)
        .transpose()?
        .filter(|value| !value.is_empty());

    Ok((doc_type, normalized_prefix))
}

fn normalize_knowledge_page_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_KNOWLEDGE_PAGE_SIZE)
        .clamp(1, MAX_KNOWLEDGE_PAGE_SIZE)
}

fn decode_knowledge_page_cursor(raw_cursor: Option<&str>) -> Result<usize, String> {
    let Some(raw_cursor) = raw_cursor else {
        return Ok(0);
    };
    let trimmed = raw_cursor.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    trimmed
        .parse::<usize>()
        .map_err(|_| "knowledge page cursor is invalid".to_string())
}

fn encode_knowledge_page_cursor(offset: Option<usize>) -> Option<String> {
    offset.map(|value| value.to_string())
}

fn resolve_knowledge_document_target(
    doc_type_hint: Option<KnowledgeType>,
    raw_path: &str,
) -> Result<(KnowledgeType, String), String> {
    let inferred_type = parse_knowledge_type_from_path(raw_path);
    let doc_type = doc_type_hint.or(inferred_type).ok_or_else(|| {
        "knowledge document target requires a type-prefixed path or an explicit type.".to_string()
    })?;
    if let Some(path_type) = inferred_type {
        if path_type != doc_type {
            return Err(
                "knowledge document path type prefix does not match the explicit type.".to_string(),
            );
        }
    }

    require_knowledge_document_path_suffix(raw_path)?;
    let normalized_path = normalize_knowledge_path(raw_path)?;
    Ok((doc_type, normalized_path))
}

fn resolve_knowledge_directory_target(
    doc_type_hint: Option<KnowledgeType>,
    raw_path: &str,
) -> Result<(KnowledgeType, String), String> {
    let inferred_type = parse_knowledge_type_from_path(raw_path);
    let doc_type = doc_type_hint.or(inferred_type).ok_or_else(|| {
        "knowledge directory target requires a type-prefixed path or an explicit type.".to_string()
    })?;
    if let Some(path_type) = inferred_type {
        if path_type != doc_type {
            return Err(
                "knowledge directory path type prefix does not match the explicit type."
                    .to_string(),
            );
        }
    }

    let normalized_path = normalize_knowledge_directory_path(raw_path)?;
    Ok((doc_type, normalized_path))
}

fn merge_directory_config(
    doc_type: KnowledgeType,
    existing: Option<KnowledgeDirectoryConfigRecord>,
    patch: &KnowledgeDirectoryConfigPatch,
) -> KnowledgeDirectoryConfig {
    let base = existing
        .map(|record| KnowledgeDirectoryConfig {
            version: record.config.version,
            summary: record.config.summary,
            inject_mode: record.config.inject_mode,
            inherit_inject_mode: record.config.inherit_inject_mode,
            ai_maintained: record.config.ai_maintained,
            inherit_ai_config: record.config.inherit_ai_config,
            explicit_maintenance_rules: record.config.explicit_maintenance_rules,
            lexical_search: record.config.lexical_search,
            vector_search: record.config.vector_search,
            inherit_to_children: record.config.inherit_to_children,
            allow_create_documents: record.config.allow_create_documents,
            allow_create_directories: record.config.allow_create_directories,
            allow_move_documents: record.config.allow_move_documents,
            allow_move_directories: record.config.allow_move_directories,
            maintenance_rules: record.config.maintenance_rules,
        })
        .unwrap_or_else(|| knowledge_store::default_directory_config_for_type(doc_type));
    knowledge_store::merge_directory_config_patch(base, patch)
}

fn parent_directory_from_document_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map(|value| value.trim_matches('/').to_string())
        .filter(|value| !value.is_empty() && value != ".")
}

fn parent_directory_from_directory_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map(|value| value.trim_matches('/').to_string())
        .filter(|value| !value.is_empty() && value != ".")
}

fn merge_document_create_patch(
    mut base: KnowledgeDocumentPatch,
    patch: KnowledgeDocumentPatch,
) -> KnowledgeDocumentPatch {
    if let Some(id) = patch.id {
        base.id = Some(id);
    }
    if let Some(doc_type) = patch.doc_type {
        base.doc_type = Some(doc_type);
    }
    if let Some(title) = patch.title {
        base.title = Some(title);
    }
    if let Some(inject_mode) = patch.inject_mode {
        base.inject_mode = Some(inject_mode);
    }
    if let Some(inherit_inject_mode) = patch.inherit_inject_mode {
        base.inherit_inject_mode = Some(inherit_inject_mode);
    }
    if let Some(summary_enabled) = patch.summary_enabled {
        base.summary_enabled = Some(summary_enabled);
    }
    if let Some(command_enabled) = patch.command_enabled {
        base.command_enabled = Some(command_enabled);
    }
    if let Some(read_only) = patch.read_only {
        base.read_only = Some(read_only);
    }
    if let Some(ai_maintained) = patch.ai_maintained {
        base.ai_maintained = Some(ai_maintained);
    }
    if let Some(inherit_ai_config) = patch.inherit_ai_config {
        base.inherit_ai_config = Some(inherit_ai_config);
    }
    if let Some(explicit_maintenance_rules) = patch.explicit_maintenance_rules {
        base.explicit_maintenance_rules = Some(explicit_maintenance_rules);
    }
    if let Some(external_source) = patch.external_source {
        base.external_source = Some(external_source);
    }
    if let Some(skill_enabled) = patch.skill_enabled {
        base.skill_enabled = Some(skill_enabled);
    }
    if let Some(skill_surface) = patch.skill_surface {
        base.skill_surface = Some(skill_surface);
    }
    if let Some(command_trigger) = patch.command_trigger {
        base.command_trigger = Some(command_trigger);
    }
    if let Some(argument_hint) = patch.argument_hint {
        base.argument_hint = Some(argument_hint);
    }
    if let Some(summary) = patch.summary {
        base.summary = Some(summary);
    }
    if let Some(body) = patch.body {
        base.body = Some(body);
    }
    if let Some(maintenance_rules) = patch.maintenance_rules {
        base.maintenance_rules = Some(maintenance_rules);
    }
    if let Some(new_path) = patch.new_path {
        base.new_path = Some(new_path);
    }
    base
}

fn ensure_parent_directory_allows_create(
    working_dir: &str,
    doc_type: KnowledgeType,
    parent_path: Option<&str>,
    kind: KnowledgeTargetKind,
) -> Result<(), String> {
    let Some(parent_path) = parent_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };

    let parent_dir = knowledge_store::knowledge_root(working_dir)
        .join(doc_type.as_str())
        .join(parent_path);
    if !parent_dir.is_dir()
        && !knowledge_store::directory_exists(working_dir, doc_type, parent_path)?
    {
        return Ok(());
    }

    let parent = knowledge_store::read_directory_config(working_dir, doc_type, parent_path)?;
    match kind {
        KnowledgeTargetKind::Document if !parent.config.allow_create_documents => Err(format!(
            "Knowledge directory '{}' does not allow creating child documents",
            parent_path
        )),
        KnowledgeTargetKind::Directory if !parent.config.allow_create_directories => Err(format!(
            "Knowledge directory '{}' does not allow creating child directories",
            parent_path
        )),
        _ => Ok(()),
    }
}

pub(crate) fn execute_knowledge_read_request(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    request: KnowledgeReadRequest,
) -> Result<KnowledgeReadResponse, String> {
    if request.path.trim().is_empty() {
        return Err("knowledge_read requires 'path'".to_string());
    }

    match request.kind {
        KnowledgeTargetKind::Document => {
            let requested_part = request.part.as_deref().unwrap_or("full");
            let inferred_type = parse_knowledge_type_from_path(&request.path);
            let requested_type = match (request.doc_type, inferred_type) {
                (Some(explicit), Some(inferred)) if explicit != inferred => {
                    return Err(
                        "knowledge document path type prefix does not match the explicit type."
                            .to_string(),
                    );
                }
                (Some(explicit), _) => Some(explicit),
                (None, inferred) => inferred,
            };
            if requested_type == Some(KnowledgeType::Skill) {
                let virtual_path = normalize_knowledge_directory_path(&request.path)?;
                if let Some(result) = super::skill::read_skill_package_document_sync(
                    working_dir,
                    &virtual_path,
                    requested_part,
                )? {
                    return Ok(KnowledgeReadResponse {
                        kind: KnowledgeTargetKind::Document,
                        document: Some(result),
                        directory: None,
                    });
                }
            }

            let (doc_type, normalized_path) =
                resolve_knowledge_document_target(request.doc_type, &request.path)?;
            if doc_type == KnowledgeType::Skill {
                if let Some(result) = super::skill::read_skill_package_document_sync(
                    working_dir,
                    &normalized_path,
                    requested_part,
                )? {
                    return Ok(KnowledgeReadResponse {
                        kind: KnowledgeTargetKind::Document,
                        document: Some(result),
                        directory: None,
                    });
                }
            }
            ensure_memory_builtins_for_type(working_dir, Some(doc_type))?;
            let result = knowledge_store::read_document_with_app_root(
                working_dir,
                app_knowledge_dir,
                doc_type,
                &normalized_path,
                requested_part,
            )?;
            Ok(KnowledgeReadResponse {
                kind: KnowledgeTargetKind::Document,
                document: Some(result),
                directory: None,
            })
        }
        KnowledgeTargetKind::Directory => {
            let (doc_type, normalized_path) =
                resolve_knowledge_directory_target(request.doc_type, &request.path)?;
            let result = knowledge_store::read_directory_config_with_app_root(
                working_dir,
                app_knowledge_dir,
                doc_type,
                &normalized_path,
            )?;
            Ok(KnowledgeReadResponse {
                kind: KnowledgeTargetKind::Directory,
                document: None,
                directory: Some(result),
            })
        }
    }
}

pub(crate) fn execute_knowledge_create_request(
    working_dir: &str,
    request: KnowledgeCreateRequest,
) -> Result<KnowledgeMutationResponse, String> {
    if request.path.trim().is_empty() {
        return Err("knowledge_create requires 'path'".to_string());
    }

    match request.kind {
        KnowledgeTargetKind::Document => {
            let document_patch = request.document.unwrap_or_default();
            let (doc_type, normalized_path) = resolve_knowledge_document_target(
                request.doc_type.or(document_patch.doc_type),
                &request.path,
            )?;
            if doc_type == KnowledgeType::Skill {
                return Err(
                    "knowledge_create cannot create Skill documents; use skill_create instead."
                        .to_string(),
                );
            }
            ensure_memory_builtins_for_type(working_dir, Some(doc_type))?;
            let parent_path = parent_directory_from_document_path(&normalized_path);
            ensure_parent_directory_allows_create(
                working_dir,
                doc_type,
                parent_path.as_deref(),
                KnowledgeTargetKind::Document,
            )?;
            let mut document_patch = merge_document_create_patch(
                knowledge_store::default_document_create_patch(
                    working_dir,
                    doc_type,
                    &normalized_path,
                )?,
                document_patch,
            );
            let document = knowledge_store::update_document(
                working_dir,
                KnowledgeUpdateRequest {
                    op: KnowledgeUpdateOp::Create,
                    path: normalized_path.clone(),
                    id: document_patch.id.take(),
                    doc_type: Some(doc_type),
                    title: document_patch.title.take(),
                    inject_mode: document_patch.inject_mode,
                    inherit_inject_mode: document_patch.inherit_inject_mode,
                    summary_enabled: document_patch.summary_enabled,
                    command_enabled: document_patch.command_enabled,
                    read_only: document_patch.read_only,
                    ai_maintained: document_patch.ai_maintained,
                    inherit_ai_config: document_patch.inherit_ai_config,
                    explicit_maintenance_rules: document_patch.explicit_maintenance_rules,
                    external_source: document_patch.external_source.take(),
                    skill_enabled: document_patch.skill_enabled,
                    skill_surface: document_patch.skill_surface,
                    command_trigger: document_patch.command_trigger.take(),
                    argument_hint: document_patch.argument_hint.take(),
                    summary: document_patch.summary.take(),
                    body: document_patch.body.take(),
                    maintenance_rules: document_patch.maintenance_rules.take(),
                    new_path: document_patch.new_path.take(),
                },
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Document,
                doc_type: document.doc_type,
                path: normalized_path,
                result_path: Some(document.path.clone()),
                document: Some(document),
                directory: None,
            })
        }
        KnowledgeTargetKind::Directory => {
            let (doc_type, normalized_path) =
                resolve_knowledge_directory_target(request.doc_type, &request.path)?;
            if doc_type == KnowledgeType::Skill {
                return Err(
                    "knowledge_create cannot create Skill directories; use skill_create instead."
                        .to_string(),
                );
            }
            let parent_path = parent_directory_from_directory_path(&normalized_path);
            ensure_parent_directory_allows_create(
                working_dir,
                doc_type,
                parent_path.as_deref(),
                KnowledgeTargetKind::Directory,
            )?;
            let result_path =
                knowledge_store::create_directory(working_dir, doc_type, &normalized_path)?;
            let directory = Some(knowledge_store::read_directory_config(
                working_dir,
                doc_type,
                &result_path,
            )?);
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Directory,
                doc_type,
                path: normalized_path,
                result_path: Some(result_path),
                document: None,
                directory,
            })
        }
    }
}

pub(crate) fn execute_knowledge_edit_request(
    working_dir: &str,
    request: KnowledgeEditRequest,
) -> Result<KnowledgeMutationResponse, String> {
    if request.path.trim().is_empty() {
        return Err("knowledge_edit requires 'path'".to_string());
    }

    match request.kind {
        KnowledgeTargetKind::Document => {
            let document_patch = request
                .document
                .ok_or_else(|| "knowledge_edit document requires 'document'.".to_string())?;
            let (doc_type, normalized_path) =
                resolve_knowledge_document_target(request.doc_type, &request.path)?;
            ensure_memory_builtins_for_type(working_dir, Some(doc_type))?;
            let document = knowledge_store::edit_document(
                working_dir,
                &normalized_path,
                Some(doc_type),
                document_patch,
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Document,
                doc_type: document.doc_type,
                path: normalized_path,
                result_path: Some(document.path.clone()),
                document: Some(document),
                directory: None,
            })
        }
        KnowledgeTargetKind::Directory => {
            let config_patch = request
                .config
                .ok_or_else(|| "knowledge_edit directory requires 'config'.".to_string())?;
            let (doc_type, normalized_path) =
                resolve_knowledge_directory_target(request.doc_type, &request.path)?;
            let current =
                knowledge_store::read_directory_config(working_dir, doc_type, &normalized_path)?;
            let merged = merge_directory_config(doc_type, Some(current), &config_patch);
            let directory = knowledge_store::update_directory_config(
                working_dir,
                doc_type,
                &normalized_path,
                merged,
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Directory,
                doc_type,
                path: normalized_path,
                result_path: None,
                document: None,
                directory: Some(directory),
            })
        }
    }
}

pub(crate) fn execute_knowledge_move_request(
    working_dir: &str,
    request: KnowledgeMoveRequest,
) -> Result<KnowledgeMutationResponse, String> {
    if request.path.trim().is_empty() {
        return Err("knowledge_move requires 'path'".to_string());
    }
    if request.new_path.trim().is_empty() {
        return Err("knowledge_move requires 'newPath'".to_string());
    }

    match request.kind {
        KnowledgeTargetKind::Document => {
            let (doc_type, normalized_path) =
                resolve_knowledge_document_target(request.doc_type, &request.path)?;
            ensure_memory_builtins_for_type(working_dir, Some(doc_type))?;
            let (_, normalized_target_path) =
                resolve_knowledge_document_target(Some(doc_type), &request.new_path)?;
            let document = knowledge_store::edit_document(
                working_dir,
                &normalized_path,
                Some(doc_type),
                KnowledgeDocumentPatch {
                    new_path: Some(normalized_target_path.clone()),
                    ..Default::default()
                },
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Document,
                doc_type: document.doc_type,
                path: normalized_path,
                result_path: Some(document.path.clone()),
                document: Some(document),
                directory: None,
            })
        }
        KnowledgeTargetKind::Directory => {
            let (doc_type, normalized_path) =
                resolve_knowledge_directory_target(request.doc_type, &request.path)?;
            let (_, normalized_target_path) =
                resolve_knowledge_directory_target(Some(doc_type), &request.new_path)?;
            let result_path = knowledge_store::move_directory(
                working_dir,
                doc_type,
                &normalized_path,
                &normalized_target_path,
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Directory,
                doc_type,
                path: normalized_path,
                result_path: Some(result_path),
                document: None,
                directory: None,
            })
        }
    }
}

pub(crate) fn execute_knowledge_delete_request(
    working_dir: &str,
    request: KnowledgeDeleteRequest,
) -> Result<KnowledgeMutationResponse, String> {
    if request.path.trim().is_empty() {
        return Err("knowledge_delete requires 'path'".to_string());
    }

    match request.kind {
        KnowledgeTargetKind::Document => {
            let (doc_type, normalized_path) =
                resolve_knowledge_document_target(request.doc_type, &request.path)?;
            ensure_memory_builtins_for_type(working_dir, Some(doc_type))?;
            let document = knowledge_store::update_document(
                working_dir,
                KnowledgeUpdateRequest {
                    op: KnowledgeUpdateOp::Delete,
                    path: normalized_path.clone(),
                    doc_type: Some(doc_type),
                    ..Default::default()
                },
            )?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Document,
                doc_type,
                path: normalized_path,
                result_path: None,
                document: Some(document),
                directory: None,
            })
        }
        KnowledgeTargetKind::Directory => {
            let (doc_type, normalized_path) =
                resolve_knowledge_directory_target(request.doc_type, &request.path)?;
            let result_path =
                knowledge_store::delete_directory(working_dir, doc_type, &normalized_path)?;
            Ok(KnowledgeMutationResponse {
                kind: KnowledgeTargetKind::Directory,
                doc_type,
                path: normalized_path,
                result_path: Some(result_path),
                document: None,
                directory: None,
            })
        }
    }
}

fn ensure_memory_builtins_for_type(
    working_dir: &str,
    doc_type: Option<KnowledgeType>,
) -> Result<(), String> {
    if matches!(doc_type, Some(KnowledgeType::Memory)) {
        knowledge_store::ensure_memory_builtin_documents(working_dir)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn knowledge_query(
    query: Option<String>,
    lexical_query: Option<String>,
    semantic_query: Option<String>,
    limit: Option<usize>,
    types: Option<Vec<String>>,
    path_prefix: Option<String>,
    include_hidden: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<Vec<KnowledgeSearchHit>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let lexical_query = lexical_query
        .or_else(|| query.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let semantic_query = semantic_query
        .or(query)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if lexical_query.is_none() && semantic_query.is_none() {
        return Err(AppError::from(
            "knowledge_query requires lexicalQuery or semanticQuery".to_string(),
        ));
    }

    let mut parsed_types = types
        .as_ref()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| parse_knowledge_type(value).ok())
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty());
    let (prefix_type, normalized_prefix) =
        resolve_knowledge_path_filter(None, path_prefix.as_deref())?;

    if let Some(prefix_type) = prefix_type {
        if let Some(ref existing) = parsed_types {
            if !existing.contains(&prefix_type) {
                return Err(AppError::from(
                    "knowledge_query pathPrefix conflicts with the provided types filter"
                        .to_string(),
                ));
            }
        }
        parsed_types = Some(vec![prefix_type]);
    }

    knowledge_index::query_documents(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        lexical_query.as_deref(),
        semantic_query.as_deref(),
        parsed_types.as_deref(),
        normalized_prefix.as_deref(),
        limit.unwrap_or(5),
        include_hidden.unwrap_or(false),
        knowledge_index_state.inner().clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_get_general_config(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<KnowledgeGeneralConfig, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(&working_dir)
    };
    Ok(knowledge_index::load_general_config(&library_dir))
}

#[tauri::command]
pub async fn knowledge_save_general_config(
    config: KnowledgeGeneralConfig,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeGeneralConfig, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(&working_dir)
    };
    knowledge_index::save_general_config(&library_dir, &config)?;
    if !working_dir.trim().is_empty() {
        knowledge_index::reconcile_workspace(
            &working_dir,
            app_knowledge_dir.0.as_ref().as_ref(),
            knowledge_index_state.inner().clone(),
        )
        .await?;
    }
    Ok(config)
}

#[tauri::command]
pub async fn knowledge_get_embedding_config(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<EmbeddingConfig, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(&working_dir)
    };
    Ok(knowledge_index::embedding::load_config(&library_dir))
}

#[tauri::command]
pub async fn knowledge_save_embedding_config(
    config: EmbeddingConfig,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<EmbeddingConfig, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(&working_dir)
    };
    knowledge_index::embedding::save_config(&library_dir, &config)?;
    let normalized_config = knowledge_index::embedding::load_config(&library_dir);
    let (should_activate, backfill_strategy) = {
        let mgr_handle = knowledge_index_state.embedding_mgr();
        let mut mgr = mgr_handle.lock().await;
        let previous_config = mgr.config().clone();
        let previous_signature = mgr.backend_signature_json();
        let restart_required = mgr.update_config(normalized_config.clone());
        let next_signature = mgr.backend_signature_json();
        let should_activate = normalized_config.enabled
            && !working_dir.trim().is_empty()
            && (restart_required || !mgr.is_ready());
        let backfill_strategy = if normalized_config.enabled && !working_dir.trim().is_empty() {
            if !previous_config.enabled || previous_signature != next_signature {
                EmbeddingActivationBackfillStrategy::VectorOnly
            } else {
                EmbeddingActivationBackfillStrategy::None
            }
        } else {
            EmbeddingActivationBackfillStrategy::None
        };
        knowledge_index_state.set_embedding_status(mgr.status());
        (should_activate, backfill_strategy)
    };
    if should_activate {
        knowledge_index::activate_embedding_runtime(
            knowledge_index_state.inner().clone(),
            &working_dir,
            app_knowledge_dir.0.as_ref().as_ref(),
            backfill_strategy,
        )
        .await?;
    } else if !normalized_config.enabled {
        knowledge_index::deactivate_embedding_runtime(knowledge_index_state.inner().clone())
            .await?;
    }
    Ok(normalized_config)
}

#[tauri::command]
pub async fn knowledge_activate_embedding(
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    knowledge_index::activate_embedding_runtime(
        knowledge_index_state.inner().clone(),
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        EmbeddingActivationBackfillStrategy::VectorOnly,
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_deactivate_embedding(
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    knowledge_index::deactivate_embedding_runtime(knowledge_index_state.inner().clone())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_get_embedding_status(
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<EmbeddingStatus, AppError> {
    Ok(knowledge_index_state.embedding_status_snapshot())
}

#[tauri::command]
pub async fn knowledge_test_embedding_runtime(
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<EmbeddingRuntimeTestResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(&working_dir)
    };
    let config = knowledge_index::embedding::load_config(&library_dir);

    let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;

    match knowledge_index::embedding::run_embedding_runtime_self_test(&config, &model_storage_dir) {
        Ok(result) => {
            let mut status = knowledge_index_state.embedding_status_snapshot();
            status.last_test_summary = Some(result.summary.clone());
            status.last_test_passed = Some(result.passed);
            knowledge_index_state.set_embedding_status(status);
            Ok(result)
        }
        Err(error) => {
            let mut status = knowledge_index_state.embedding_status_snapshot();
            status.last_test_summary = Some(error.clone());
            status.last_test_passed = Some(false);
            knowledge_index_state.set_embedding_status(status);
            Err(
                AppError::new("knowledge.embedding_runtime_test_failed", error)
                    .operation("knowledge_test_embedding_runtime"),
            )
        }
    }
}

#[tauri::command]
pub async fn knowledge_get_local_embedding_model_catalog(
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<EmbeddingLocalModelCatalog, AppError> {
    let _working_dir = workspace.path.read().await.clone();
    let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;
    Ok(knowledge_index::embedding::local_model_catalog(
        &model_storage_dir,
    ))
}

#[tauri::command]
pub async fn knowledge_download_local_embedding_model(
    model_id: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;
    match knowledge_index::download_local_embedding_model(
        knowledge_index_state.inner().clone(),
        &model_storage_dir,
        &model_id,
    )
    .await
    {
        Ok(()) => {
            emit_knowledge_changed(
                &app_handle,
                &working_dir,
                "knowledge_download_local_embedding_model",
            );
            Ok(())
        }
        Err(knowledge_index::EmbeddingDownloadError::Cancelled) => Err(AppError::new(
            "knowledge.embedding_model_download_cancelled",
            "Model download cancelled",
        )
        .operation("knowledge_download_local_embedding_model")
        .severity(ErrorSeverity::Info)),
        Err(knowledge_index::EmbeddingDownloadError::Failed(message)) => Err(AppError::new(
            "knowledge.embedding_model_download_failed",
            message,
        )
        .operation("knowledge_download_local_embedding_model")),
    }
}

#[tauri::command]
pub async fn knowledge_cancel_local_embedding_model_download(
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    knowledge_index_state
        .inner()
        .request_embedding_download_cancel();
    let mut status = knowledge_index_state.inner().embedding_status_snapshot();
    if status.activating {
        status.stage = Some("cancelling".to_string());
        status.detail = Some("正在取消下载并清理已下载文件。".to_string());
        status.error = None;
        knowledge_index_state.inner().set_embedding_status(status);
    }
    Ok(())
}

#[tauri::command]
pub async fn knowledge_close_download_progress_window(
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let Some(window) = app_handle.get_webview_window(KNOWLEDGE_DOWNLOAD_WINDOW_LABEL) else {
        return Ok(());
    };
    window.close().map_err(|error| {
        AppError::new(
            "knowledge.download_window_close_failed",
            format!("Failed to close download progress window: {}", error),
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn knowledge_close_lexical_progress_window(
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let Some(window) = app_handle.get_webview_window(KNOWLEDGE_LEXICAL_PROGRESS_WINDOW_LABEL)
    else {
        return Ok(());
    };
    window
        .destroy()
        .or_else(|_| window.close())
        .map_err(|error| {
            AppError::new(
                "knowledge.lexical_window_close_failed",
                format!("Failed to close lexical progress window: {}", error),
            )
        })?;
    Ok(())
}

#[tauri::command]
pub async fn knowledge_close_unity_reference_import_progress_window(
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let Some(window) = app_handle.get_webview_window(UNITY_REFERENCE_IMPORT_WINDOW_LABEL) else {
        return Ok(());
    };
    window.close().map_err(|error| {
        AppError::new(
            "knowledge.unity_reference_import_window_close_failed",
            format!(
                "Failed to close unity reference import progress window: {}",
                error
            ),
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn knowledge_close_feishu_reference_import_progress_window(
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let Some(window) = app_handle.get_webview_window(FEISHU_REFERENCE_IMPORT_WINDOW_LABEL) else {
        return Ok(());
    };
    window.close().map_err(|error| {
        AppError::new(
            "knowledge.feishu_reference_import_window_close_failed",
            format!(
                "Failed to close feishu reference import progress window: {}",
                error
            ),
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn knowledge_inspect_local_embedding_model_directory(
    path: String,
) -> Result<EmbeddingLocalModelDirectoryInspection, AppError> {
    Ok(knowledge_index::embedding::inspect_local_model_directory(
        std::path::Path::new(&path),
    ))
}

#[tauri::command]
pub async fn knowledge_rebuild_lexical_index(
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<usize, AppError> {
    let working_dir = workspace.path.read().await.clone();
    knowledge_index::rebuild_lexical_index_runtime(
        knowledge_index_state.inner().clone(),
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_get_lexical_rebuild_status(
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<LexicalRebuildStatus, AppError> {
    Ok(knowledge_index_state.lexical_rebuild_status_snapshot())
}

#[tauri::command]
pub async fn knowledge_get_overview(
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeOverview, AppError> {
    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Ok(KnowledgeOverview::default());
    }
    let started_at = Instant::now();
    eprintln!(
        "[KnowledgeCommand] knowledge_get_overview start workspace={}",
        working_dir
    );
    let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;
    let overview = knowledge_index::build_overview(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        knowledge_index_state.inner().clone(),
        &model_storage_dir,
    )
    .await
    .map_err(AppError::from)?;
    eprintln!(
        "[KnowledgeCommand] knowledge_get_overview finished workspace={} elapsed_ms={} total_documents={}",
        working_dir,
        started_at.elapsed().as_millis(),
        overview.total_document_count
    );
    Ok(overview)
}

#[tauri::command]
pub async fn knowledge_get_unity_reference_import_status(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    unity_reference_import_state: State<'_, UnityReferenceImportState>,
) -> Result<UnityReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    unity_docs::get_unity_reference_import_status(
        &working_dir,
        target_path.as_deref(),
        unity_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_find_unity_reference_directory(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Option<KnowledgeDirectoryConfigRecord>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    knowledge_store::find_reference_directory_by_external_provider(
        &working_dir,
        KnowledgeSourceProvider::Unity,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_cancel_unity_reference_import(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    unity_reference_import_state: State<'_, UnityReferenceImportState>,
) -> Result<UnityReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    unity_docs::cancel_unity_reference_import(
        &working_dir,
        target_path.as_deref(),
        unity_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_get_feishu_reference_import_status(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::get_feishu_reference_import_status(
        &working_dir,
        target_path.as_deref(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_save_feishu_reference_config(
    config: FeishuReferenceConfigInput,
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::save_feishu_reference_config(
        &working_dir,
        config,
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_test_feishu_reference_connection(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceConnectionTestResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::test_feishu_reference_connection(
        &working_dir,
        target_path.as_deref(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_start_feishu_reference_oauth(
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceOauthStartResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::start_feishu_reference_oauth(
        working_dir,
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_cancel_feishu_reference_oauth_wait(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::cancel_feishu_reference_oauth_wait(
        &working_dir,
        target_path.as_deref(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_list_feishu_reference_space_nodes(
    space_id: String,
    parent_node_token: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<FeishuReferenceNodeSummary>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::list_feishu_reference_space_nodes(&working_dir, space_id, parent_node_token)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_cancel_feishu_reference_import(
    target_path: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::cancel_feishu_reference_import(
        &working_dir,
        target_path.as_deref(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_read(
    request: KnowledgeReadRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<KnowledgeReadResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    execute_knowledge_read_request(&working_dir, app_knowledge_dir.0.as_ref().as_ref(), request)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_import_unity_reference_docs(
    target_path: Option<String>,
    locale: Option<String>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
    unity_reference_import_state: State<'_, UnityReferenceImportState>,
) -> Result<UnityReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    unity_docs::start_unity_reference_import(
        app_handle,
        working_dir,
        target_path,
        locale,
        knowledge_index_state.inner().clone(),
        unity_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_import_feishu_reference_docs(
    request: FeishuReferenceImportRequest,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::start_feishu_reference_import(
        app_handle,
        working_dir,
        request,
        knowledge_index_state.inner().clone(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_delete_unity_reference_docs(
    target_path: Option<String>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
    unity_reference_import_state: State<'_, UnityReferenceImportState>,
) -> Result<UnityReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    unity_docs::delete_unity_reference_docs(
        app_handle,
        working_dir,
        target_path,
        knowledge_index_state.inner().clone(),
        unity_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_delete_feishu_reference_docs(
    target_path: Option<String>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
    feishu_reference_import_state: State<'_, FeishuReferenceImportState>,
) -> Result<FeishuReferenceImportStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    feishu_docs::delete_feishu_reference_docs(
        app_handle,
        working_dir,
        target_path,
        knowledge_index_state.inner().clone(),
        feishu_reference_import_state.inner().0.clone(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_list(
    doc_type: Option<String>,
    path_prefix: Option<String>,
    include_hidden: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<Vec<KnowledgeListItem>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let parsed_type = doc_type.as_deref().map(parse_knowledge_type).transpose()?;
    let (resolved_type, resolved_prefix) =
        resolve_knowledge_path_filter(parsed_type, path_prefix.as_deref())?;
    ensure_memory_builtins_for_type(&working_dir, resolved_type)?;
    let started_at = Instant::now();
    eprintln!(
        "[KnowledgeCommand] knowledge_list start workspace={} doc_type={:?} path_prefix={:?}",
        working_dir, resolved_type, resolved_prefix
    );
    let items = knowledge_index::list_cached_documents(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        resolved_type,
        resolved_prefix.as_deref(),
        knowledge_index_state.inner().clone(),
    )
    .await
    .map_err(AppError::from)?;
    let mut items = items;
    if resolved_type.is_none() || resolved_type == Some(KnowledgeType::Skill) {
        let existing_paths = items
            .iter()
            .filter(|item| item.doc_type == KnowledgeType::Skill)
            .map(|item| item.path.clone())
            .collect::<HashSet<_>>();
        items.extend(
            super::skill::list_skill_package_knowledge_items_sync_with_hidden(
                &working_dir,
                resolved_prefix.as_deref(),
                include_hidden.unwrap_or(false),
            )
            .into_iter()
            .filter(|item| !existing_paths.contains(&item.path)),
        );
        items.sort_by(|a, b| {
            a.doc_type
                .as_str()
                .cmp(b.doc_type.as_str())
                .then(a.path.cmp(&b.path))
                .then(a.title.cmp(&b.title))
        });
    }
    if !include_hidden.unwrap_or(false) {
        items.retain(|item| {
            item.inject_mode != KnowledgeInjectMode::None
                && item_model_recall_allowed(&working_dir, item).unwrap_or(false)
        });
    }
    enrich_knowledge_list_items(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &mut items,
    );
    eprintln!(
        "[KnowledgeCommand] knowledge_list finished workspace={} elapsed_ms={} count={}",
        working_dir,
        started_at.elapsed().as_millis(),
        items.len()
    );
    Ok(items)
}

#[tauri::command]
pub async fn knowledge_list_page(
    doc_type: Option<String>,
    path_prefix: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeListPageResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let parsed_type = doc_type.as_deref().map(parse_knowledge_type).transpose()?;
    let (resolved_type, resolved_prefix) =
        resolve_knowledge_path_filter(parsed_type, path_prefix.as_deref())?;
    ensure_memory_builtins_for_type(&working_dir, resolved_type)?;
    let started_at = Instant::now();
    let resolved_limit = normalize_knowledge_page_limit(limit);
    let resolved_offset =
        decode_knowledge_page_cursor(cursor.as_deref()).map_err(AppError::from)?;
    eprintln!(
        "[KnowledgeCommand] knowledge_list_page start workspace={} doc_type={:?} path_prefix={:?} offset={} limit={}",
        working_dir, resolved_type, resolved_prefix, resolved_offset, resolved_limit
    );
    let page = knowledge_index::list_cached_documents_page(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        resolved_type,
        resolved_prefix.as_deref(),
        resolved_limit,
        resolved_offset,
        knowledge_index_state.inner().clone(),
    )
    .await
    .map_err(AppError::from)?;
    let mut items = page.items;
    enrich_knowledge_list_items(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &mut items,
    );
    eprintln!(
        "[KnowledgeCommand] knowledge_list_page finished workspace={} elapsed_ms={} count={} next_cursor={:?}",
        working_dir,
        started_at.elapsed().as_millis(),
        items.len(),
        page.next_offset
    );
    Ok(KnowledgeListPageResponse {
        items,
        next_cursor: encode_knowledge_page_cursor(page.next_offset),
    })
}

fn enrich_knowledge_list_items(
    working_dir: &str,
    app_root: Option<&std::path::PathBuf>,
    items: &mut [KnowledgeListItem],
) {
    let library_dir = if working_dir.trim().is_empty() {
        knowledge_index::no_workspace_library_dir()
    } else {
        knowledge_index::library_dir_for_working_dir(working_dir)
    };
    let general_config = knowledge_index::load_general_config(&library_dir);

    for item in items {
        if item.byte_size.is_none() {
            if let Ok(document) = knowledge_store::load_document_by_path_with_app_root(
                working_dir,
                app_root,
                item.doc_type,
                &item.path,
            ) {
                item.byte_size = knowledge_store::rendered_document_size_bytes(&document).ok();
            }
        }
        if let Ok(access) = knowledge_store::effective_document_search_access_with_app_root(
            working_dir,
            app_root,
            item.doc_type,
            &item.path,
        ) {
            item.lexical_search_enabled = Some(
                general_config.enabled
                    && general_config.lexical_search_enabled
                    && access.lexical_enabled,
            );
            item.semantic_search_enabled = Some(
                general_config.enabled
                    && general_config.semantic_search_enabled
                    && access.vector_enabled,
            );
        }
    }
}

fn item_model_recall_allowed(working_dir: &str, item: &KnowledgeListItem) -> Result<bool, String> {
    if item.doc_type != KnowledgeType::Skill {
        return Ok(true);
    }
    if let Some(allowed) =
        super::skill::skill_package_virtual_path_allows_model_recall_sync(working_dir, &item.path)?
    {
        return Ok(allowed);
    }
    Ok(knowledge_store::list_item_allows_model_recall(item))
}

#[tauri::command]
pub async fn knowledge_list_directories(
    doc_type: String,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<Vec<String>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let parsed_type = parse_knowledge_type(&doc_type)?;
    ensure_memory_builtins_for_type(&working_dir, Some(parsed_type))?;
    let started_at = Instant::now();
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directories start workspace={} doc_type={}",
        working_dir, doc_type
    );
    let directories = knowledge_store::list_directories_with_app_root(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        parsed_type,
    )
    .map_err(AppError::from)?;
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directories finished workspace={} doc_type={} elapsed_ms={} count={}",
        working_dir,
        doc_type,
        started_at.elapsed().as_millis(),
        directories.len()
    );
    Ok(directories)
}

#[tauri::command]
pub async fn knowledge_list_directory_documents(
    doc_type: String,
    path: String,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<Vec<KnowledgeListItem>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let parsed_type = parse_knowledge_type(&doc_type)?;
    ensure_memory_builtins_for_type(&working_dir, Some(parsed_type))?;
    let normalized_path = path
        .trim()
        .trim_matches('/')
        .is_empty()
        .then_some(String::new())
        .or_else(|| normalize_knowledge_directory_path(&path).ok());
    let normalized_path = match normalized_path {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
        None if path.trim().trim_matches('/').is_empty() => None,
        None => {
            return Err(AppError::from(
                "knowledge directory path must be relative".to_string(),
            ));
        }
    };
    let started_at = Instant::now();
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directory_documents start workspace={} doc_type={} path={:?}",
        working_dir, doc_type, normalized_path
    );
    let mut items = knowledge_index::list_cached_directory_documents(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        parsed_type,
        normalized_path.as_deref(),
        knowledge_index_state.inner().clone(),
    )
    .await
    .map_err(AppError::from)?;
    enrich_knowledge_list_items(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &mut items,
    );
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directory_documents finished workspace={} doc_type={} path={:?} elapsed_ms={} count={}",
        working_dir,
        doc_type,
        normalized_path,
        started_at.elapsed().as_millis(),
        items.len()
    );
    Ok(items)
}

#[tauri::command]
pub async fn knowledge_list_directory_documents_page(
    doc_type: String,
    path: String,
    cursor: Option<String>,
    limit: Option<usize>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeListPageResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let parsed_type = parse_knowledge_type(&doc_type)?;
    ensure_memory_builtins_for_type(&working_dir, Some(parsed_type))?;
    let normalized_path = path
        .trim()
        .trim_matches('/')
        .is_empty()
        .then_some(String::new())
        .or_else(|| normalize_knowledge_directory_path(&path).ok());
    let normalized_path = match normalized_path {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
        None if path.trim().trim_matches('/').is_empty() => None,
        None => {
            return Err(AppError::from(
                "knowledge directory path must be relative".to_string(),
            ));
        }
    };
    let started_at = Instant::now();
    let resolved_limit = normalize_knowledge_page_limit(limit);
    let resolved_offset =
        decode_knowledge_page_cursor(cursor.as_deref()).map_err(AppError::from)?;
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directory_documents_page start workspace={} doc_type={} path={:?} offset={} limit={}",
        working_dir, doc_type, normalized_path, resolved_offset, resolved_limit
    );
    let page = knowledge_index::list_cached_directory_documents_page(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        parsed_type,
        normalized_path.as_deref(),
        resolved_limit,
        resolved_offset,
        knowledge_index_state.inner().clone(),
    )
    .await
    .map_err(AppError::from)?;
    let mut items = page.items;
    enrich_knowledge_list_items(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &mut items,
    );
    eprintln!(
        "[KnowledgeCommand] knowledge_list_directory_documents_page finished workspace={} doc_type={} path={:?} elapsed_ms={} count={} next_cursor={:?}",
        working_dir,
        doc_type,
        normalized_path,
        started_at.elapsed().as_millis(),
        items.len(),
        page.next_offset
    );
    Ok(KnowledgeListPageResponse {
        items,
        next_cursor: encode_knowledge_page_cursor(page.next_offset),
    })
}

#[tauri::command]
pub async fn knowledge_list_external_reference_directories(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<KnowledgeExternalDirectoryBinding>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    knowledge_store::list_reference_external_directory_bindings(&working_dir)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn knowledge_list_unity_managed_directory_stats(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<UnityManagedDirectoryStat>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    unity_docs::list_managed_directory_stats(&working_dir).map_err(AppError::from)
}

#[tauri::command]
pub async fn knowledge_create(
    request: KnowledgeCreateRequest,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeMutationResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    let result = execute_knowledge_create_request(&working_dir, request).map_err(AppError::from)?;
    match result.kind {
        KnowledgeTargetKind::Document => {
            if let Some(document) = result.document.clone() {
                remove_shadowed_documents_for_path(
                    knowledge_index_state.inner().clone(),
                    document.doc_type,
                    &document.path,
                    Some(&document.id),
                )?;
                knowledge_index::upsert_document(
                    knowledge_index_state.inner().clone(),
                    &working_dir,
                    app_knowledge_dir.0.as_ref().as_ref(),
                    document,
                )
                .await
                .map_err(AppError::from)?;
            }
            emit_knowledge_changed(&app_handle, &working_dir, "knowledge_create");
        }
        KnowledgeTargetKind::Directory => {
            reconcile_and_emit_knowledge_changed(
                &app_handle,
                &working_dir,
                knowledge_index_state.inner().clone(),
                "knowledge_create",
            )
            .await?;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn knowledge_edit(
    request: KnowledgeEditRequest,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeMutationResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    let result = execute_knowledge_edit_request(&working_dir, request).map_err(AppError::from)?;
    match result.kind {
        KnowledgeTargetKind::Document => {
            if let Some(document) = result.document.clone() {
                let previous_path = result.path.clone();
                remove_shadowed_documents_for_path(
                    knowledge_index_state.inner().clone(),
                    document.doc_type,
                    &document.path,
                    Some(&document.id),
                )?;
                knowledge_index::upsert_document(
                    knowledge_index_state.inner().clone(),
                    &working_dir,
                    app_knowledge_dir.0.as_ref().as_ref(),
                    document.clone(),
                )
                .await
                .map_err(AppError::from)?;
                if previous_path != document.path {
                    restore_visible_document_for_path(
                        &app_handle,
                        &working_dir,
                        knowledge_index_state.inner().clone(),
                        document.doc_type,
                        &previous_path,
                    )
                    .await?;
                }
            }
            emit_knowledge_changed(&app_handle, &working_dir, "knowledge_edit");
        }
        KnowledgeTargetKind::Directory => {
            reconcile_and_emit_knowledge_changed(
                &app_handle,
                &working_dir,
                knowledge_index_state.inner().clone(),
                "knowledge_edit",
            )
            .await?;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn knowledge_move(
    request: KnowledgeMoveRequest,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeMutationResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let app_knowledge_dir: State<'_, AppKnowledgeDir> = app_handle.state();
    let result = execute_knowledge_move_request(&working_dir, request).map_err(AppError::from)?;
    match result.kind {
        KnowledgeTargetKind::Document => {
            if let Some(document) = result.document.clone() {
                let previous_path = result.path.clone();
                remove_shadowed_documents_for_path(
                    knowledge_index_state.inner().clone(),
                    document.doc_type,
                    &document.path,
                    Some(&document.id),
                )?;
                knowledge_index::upsert_document(
                    knowledge_index_state.inner().clone(),
                    &working_dir,
                    app_knowledge_dir.0.as_ref().as_ref(),
                    document.clone(),
                )
                .await
                .map_err(AppError::from)?;
                restore_visible_document_for_path(
                    &app_handle,
                    &working_dir,
                    knowledge_index_state.inner().clone(),
                    document.doc_type,
                    &previous_path,
                )
                .await?;
            }
            emit_knowledge_changed(&app_handle, &working_dir, "knowledge_move");
        }
        KnowledgeTargetKind::Directory => {
            reconcile_and_emit_knowledge_changed(
                &app_handle,
                &working_dir,
                knowledge_index_state.inner().clone(),
                "knowledge_move",
            )
            .await?;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn knowledge_delete(
    request: KnowledgeDeleteRequest,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<KnowledgeMutationResponse, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let result = execute_knowledge_delete_request(&working_dir, request).map_err(AppError::from)?;
    match result.kind {
        KnowledgeTargetKind::Document => {
            if let Some(document) = result.document.clone() {
                knowledge_index::remove_documents(
                    knowledge_index_state.inner().clone(),
                    &[document.id],
                )
                .map_err(AppError::from)?;
                restore_visible_document_for_path(
                    &app_handle,
                    &working_dir,
                    knowledge_index_state.inner().clone(),
                    document.doc_type,
                    &result.path,
                )
                .await?;
            }
            emit_knowledge_changed(&app_handle, &working_dir, "knowledge_delete");
        }
        KnowledgeTargetKind::Directory => {
            reconcile_and_emit_knowledge_changed(
                &app_handle,
                &working_dir,
                knowledge_index_state.inner().clone(),
                "knowledge_delete",
            )
            .await?;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn knowledge_delete_external_reference_directory(
    path: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let (_, normalized_path) =
        resolve_knowledge_directory_target(Some(KnowledgeType::Reference), &path)?;
    knowledge_store::delete_external_reference_directory(&working_dir, &normalized_path)
        .map_err(AppError::from)?;
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "knowledge_delete_external_reference_directory",
    )
    .await?;
    Ok(())
}

fn extract_title_from_file(path: &std::path::Path, file_name: &str) -> String {
    if let Ok(file) = std::fs::File::open(path) {
        let reader = std::io::BufReader::new(file);
        for line in reader.lines().take(20) {
            if let Ok(line) = line {
                let trimmed = line.trim();
                if let Some(heading) = trimmed.strip_prefix("# ") {
                    let title = heading.trim();
                    if !title.is_empty() {
                        return title.to_string();
                    }
                }
            }
        }
    }
    file_name
        .strip_suffix(".md")
        .unwrap_or(file_name)
        .to_string()
}

pub fn get_updated_at(path: &std::path::Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
        .unwrap_or(0)
}

pub(crate) fn open_file_native(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        crate::process_util::command("cmd")
            .raw_arg(format!("/c start \"\" \"{}\"", path.to_string_lossy()))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open file: {}", e))?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Err("Unsupported operating system".to_string())
}

pub(crate) fn reveal_path_native(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut command = crate::process_util::command("explorer");
        if path.is_dir() {
            command
                .raw_arg(format!("\"{}\"", path.to_string_lossy()))
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open file location: {}", e))?;
        } else {
            command
                .raw_arg(format!("/select,\"{}\"", path.to_string_lossy()))
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to reveal file location: {}", e))?;
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        if path.is_file() {
            command.arg("-R");
        }
        command
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open file location: {}", e))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| {
                    format!("Failed to resolve parent directory for {}", path.display())
                })?
        };
        std::process::Command::new("xdg-open")
            .arg(&target)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open file location: {}", e))?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Err("Unsupported operating system".to_string())
}

/// Validate that a user-supplied path is relative and resolves within the workspace.
/// Returns the canonicalized full path on success.
fn validate_workspace_relative_path(file_path: &str) -> Result<(), AppError> {
    let p = std::path::Path::new(file_path);

    // Reject absolute paths, UNC paths, and path traversals
    if p.is_absolute() || file_path.starts_with("\\\\") || file_path.contains("..") {
        return Err("Invalid path: must be a relative workspace path"
            .to_string()
            .into());
    }

    Ok(())
}

fn validate_workspace_path(
    file_path: &str,
    working_dir: &str,
) -> Result<std::path::PathBuf, AppError> {
    validate_workspace_relative_path(file_path)?;

    let full = std::path::Path::new(working_dir).join(file_path);
    let canonical =
        dunce::canonicalize(&full).map_err(|e| format!("Failed to resolve path: {}", e))?;

    // Ensure the resolved path is still within the workspace
    let ws_canonical = dunce::canonicalize(working_dir)
        .map_err(|e| format!("Failed to resolve workspace: {}", e))?;
    if !canonical.starts_with(&ws_canonical) {
        return Err("Path resolves outside workspace".to_string().into());
    }

    Ok(canonical)
}

fn is_absolute_local_path(file_path: &str) -> bool {
    let path = std::path::Path::new(file_path);
    path.is_absolute() || file_path.starts_with("\\\\")
}

fn resolve_openable_file_ref_path(
    file_path: &str,
    working_dir: &str,
) -> Result<std::path::PathBuf, AppError> {
    if is_absolute_local_path(file_path) {
        return canonicalize_existing_path(std::path::Path::new(file_path));
    }

    validate_workspace_path(file_path, working_dir)
}

/// Resolve a workspace-relative path for "show in folder" behavior.
/// Existing files are revealed directly; missing files fall back to the nearest
/// existing parent directory within the workspace.
fn resolve_workspace_reveal_path(
    file_path: &str,
    working_dir: &str,
) -> Result<std::path::PathBuf, AppError> {
    validate_workspace_relative_path(file_path)?;

    let workspace_canonical = dunce::canonicalize(working_dir)
        .map_err(|e| format!("Failed to resolve workspace: {}", e))?;
    let full = std::path::Path::new(working_dir).join(file_path);

    if full.exists() {
        let canonical =
            dunce::canonicalize(&full).map_err(|e| format!("Failed to resolve path: {}", e))?;
        if !canonical.starts_with(&workspace_canonical) {
            return Err("Path resolves outside workspace".to_string().into());
        }
        return Ok(canonical);
    }

    let mut candidate = full.parent();
    while let Some(path) = candidate {
        if path.exists() {
            let canonical =
                dunce::canonicalize(path).map_err(|e| format!("Failed to resolve path: {}", e))?;
            if !canonical.starts_with(&workspace_canonical) {
                return Err("Path resolves outside workspace".to_string().into());
            }
            return Ok(canonical);
        }
        candidate = path.parent();
    }

    Ok(workspace_canonical)
}

fn resolve_absolute_reveal_path(file_path: &str) -> Result<std::path::PathBuf, AppError> {
    let full = std::path::Path::new(file_path);
    if full.exists() {
        return canonicalize_existing_path(full);
    }

    let mut candidate = full.parent();
    while let Some(path) = candidate {
        if path.exists() {
            return canonicalize_existing_path(path);
        }
        candidate = path.parent();
    }

    Err(format!("Path not found: {}", file_path).into())
}

fn resolve_file_ref_reveal_path(
    file_path: &str,
    working_dir: &str,
) -> Result<std::path::PathBuf, AppError> {
    if is_absolute_local_path(file_path) {
        return resolve_absolute_reveal_path(file_path);
    }

    resolve_workspace_reveal_path(file_path, working_dir)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRevealRequest {
    pub kind: KnowledgeTargetKind,
    pub doc_type: String,
    pub path: String,
}

fn canonicalize_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, AppError> {
    dunce::canonicalize(path).map_err(|e| format!("Failed to resolve path: {}", e).into())
}

fn resolve_knowledge_reveal_path(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: KnowledgeType,
    kind: KnowledgeTargetKind,
    raw_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    if kind == KnowledgeTargetKind::Directory && raw_path.trim().is_empty() {
        let workspace_root = knowledge_store::knowledge_root(working_dir).join(doc_type.as_str());
        if workspace_root.is_dir() {
            return canonicalize_existing_path(&workspace_root);
        }

        if let Some(app_root) = app_knowledge_dir {
            let app_type_root = app_root.join(doc_type.as_str());
            if app_type_root.is_dir() {
                return canonicalize_existing_path(&app_type_root);
            }
        }

        return Err(format!("Knowledge root not found for {}", doc_type.as_str()).into());
    }

    let normalized_path = match kind {
        KnowledgeTargetKind::Document => {
            if doc_type == KnowledgeType::Skill {
                match resolve_knowledge_document_target(Some(doc_type), raw_path) {
                    Ok((_, path)) => path,
                    Err(error) => {
                        let virtual_path =
                            normalize_knowledge_directory_path(raw_path).map_err(AppError::from)?;
                        if let Some(package_path) =
                            super::skill::resolve_skill_package_document_path_sync_for_working_dir(
                                working_dir,
                                &virtual_path,
                            )
                            .map_err(AppError::from)?
                        {
                            return canonicalize_existing_path(&package_path);
                        }
                        return Err(AppError::from(error));
                    }
                }
            } else {
                let (_, path) = resolve_knowledge_document_target(Some(doc_type), raw_path)
                    .map_err(AppError::from)?;
                path
            }
        }
        KnowledgeTargetKind::Directory => {
            let (_, path) = resolve_knowledge_directory_target(Some(doc_type), raw_path)
                .map_err(AppError::from)?;
            path
        }
    };

    let workspace_path = match kind {
        KnowledgeTargetKind::Document => {
            knowledge_store::document_path(working_dir, doc_type, &normalized_path)
                .map_err(AppError::from)?
        }
        KnowledgeTargetKind::Directory => knowledge_store::knowledge_root(working_dir)
            .join(doc_type.as_str())
            .join(&normalized_path),
    };
    if workspace_path.exists() {
        return canonicalize_existing_path(&workspace_path);
    }

    if doc_type == KnowledgeType::Skill {
        match kind {
            KnowledgeTargetKind::Document => {
                if let Some(package_path) =
                    super::skill::resolve_skill_package_document_path_sync_for_working_dir(
                        working_dir,
                        &normalized_path,
                    )
                    .map_err(AppError::from)?
                {
                    return canonicalize_existing_path(&package_path);
                }
            }
            KnowledgeTargetKind::Directory => {
                if !normalized_path.contains('/') {
                    if let Ok(package_root) =
                        super::skill::resolve_skill_package_root_sync_for_working_dir(
                            working_dir,
                            &normalized_path,
                        )
                    {
                        return canonicalize_existing_path(&package_root);
                    }
                }
            }
        }
    }

    if kind == KnowledgeTargetKind::Document
        && doc_type == KnowledgeType::Reference
        && unity_docs::is_unity_reference_managed_relative_path(&normalized_path)
    {
        let bundle_path = unity_docs::managed_store_path(working_dir);
        if bundle_path.is_file() {
            return canonicalize_existing_path(&bundle_path);
        }

        let managed_root = knowledge_store::knowledge_root(working_dir)
            .join(doc_type.as_str())
            .join(unity_docs::UNITY_REFERENCE_MANAGED_DIR);
        if managed_root.is_dir() {
            return canonicalize_existing_path(&managed_root);
        }
    }

    if let Some(app_root) = app_knowledge_dir {
        let app_path = match kind {
            KnowledgeTargetKind::Document => {
                knowledge_store::document_path_in_root(app_root, doc_type, &normalized_path)
                    .map_err(AppError::from)?
            }
            KnowledgeTargetKind::Directory => {
                app_root.join(doc_type.as_str()).join(&normalized_path)
            }
        };
        if app_path.exists() {
            return canonicalize_existing_path(&app_path);
        }
    }

    Err(format!(
        "Knowledge target not found: {}/{}",
        doc_type.as_str(),
        normalized_path
    )
    .into())
}

#[tauri::command]
pub async fn open_file_external(
    file_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let canonical = resolve_openable_file_ref_path(&file_path, &working_dir)?;

    if !canonical.exists() {
        return Err(format!("File not found: {}", file_path).into());
    }

    open_file_native(&canonical).map_err(Into::into)
}

#[tauri::command]
pub async fn reveal_workspace_file(
    file_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let reveal_path = resolve_file_ref_reveal_path(&file_path, &working_dir)?;
    reveal_path_native(&reveal_path).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_reveal_target(
    request: KnowledgeRevealRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    let doc_type = parse_knowledge_type(&request.doc_type).map_err(AppError::from)?;
    let reveal_path = resolve_knowledge_reveal_path(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        doc_type,
        request.kind,
        &request.path,
    )?;
    reveal_path_native(&reveal_path).map_err(Into::into)
}

const BINARY_EXTS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tga", ".psd", ".tif", ".tiff", ".exr", ".hdr",
    ".webp", ".ico", ".svg", ".fbx", ".obj", ".blend", ".dae", ".3ds", ".wav", ".mp3", ".ogg",
    ".aif", ".aiff", ".flac", ".mp4", ".avi", ".mov", ".wmv", ".webm", ".dll", ".so", ".dylib",
    ".exe", ".a", ".lib", ".ttf", ".otf", ".woff", ".woff2", ".zip", ".rar", ".7z", ".gz", ".tar",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx",
];

const MARKDOWN_IMAGE_MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownImagePreview {
    pub url: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub display_path: String,
}

const CODE_EXTS: &[&str] = &[
    ".cs", ".js", ".ts", ".jsx", ".tsx", ".py", ".rs", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
    ".lua", ".rb", ".sh", ".bat", ".ps1", ".json", ".xml", ".yaml", ".yml", ".toml", ".ini",
    ".cfg", ".html", ".css", ".scss", ".less", ".vue", ".svelte", ".md", ".txt", ".log", ".csv",
    ".csproj", ".sln", ".asmdef", ".asmref", ".shader", ".hlsl", ".glsl", ".cginc", ".compute",
];

fn ext_lower(path: &str) -> Option<String> {
    let dot = path.rfind('.')?;
    Some(path[dot..].to_lowercase())
}

fn is_binary_ext(ext: &str) -> bool {
    BINARY_EXTS.contains(&ext)
}

fn markdown_image_mime_for_ext(ext: &str) -> Option<&'static str> {
    match ext.trim_start_matches('.') {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn markdown_image_mime_for_path(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    markdown_image_mime_for_ext(&ext)
}

fn markdown_image_path_from_file_url(source: &str) -> Option<std::path::PathBuf> {
    let url = url::Url::parse(source).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

fn resolve_markdown_image_path(
    source: &str,
    working_dir: &str,
) -> Result<std::path::PathBuf, AppError> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "markdown_image.empty_source",
            "Image path is empty",
        ));
    }

    if let Some(file_path) = markdown_image_path_from_file_url(trimmed) {
        return canonicalize_existing_path(&file_path);
    }

    let normalized = trimmed.replace('\\', std::path::MAIN_SEPARATOR_STR);
    if is_absolute_local_path(&normalized) {
        return canonicalize_existing_path(std::path::Path::new(&normalized));
    }

    if working_dir.trim().is_empty() {
        return Err(AppError::new(
            "markdown_image.no_workspace",
            "A workspace is required for relative image paths",
        ));
    }

    validate_workspace_path(&normalized, working_dir)
}

#[tauri::command]
pub async fn resolve_markdown_image(
    source: String,
    workspace: State<'_, Arc<Workspace>>,
    binary_cache: State<'_, Arc<BinaryCache>>,
) -> Result<MarkdownImagePreview, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let canonical = resolve_markdown_image_path(&source, &working_dir)?;
    if !canonical.is_file() {
        return Err(AppError::new(
            "markdown_image.not_file",
            format!("Image path is not a file: {}", canonical.display()),
        ));
    }

    let mime = markdown_image_mime_for_path(&canonical).ok_or_else(|| {
        AppError::new(
            "markdown_image.unsupported_type",
            format!("Unsupported image type: {}", source),
        )
    })?;

    let metadata = std::fs::metadata(&canonical).map_err(|error| {
        AppError::new(
            "markdown_image.metadata_failed",
            format!("Failed to read image metadata: {}", error),
        )
    })?;
    let byte_size = metadata.len();
    if byte_size > MARKDOWN_IMAGE_MAX_FILE_BYTES {
        return Err(AppError::new(
            "markdown_image.too_large",
            format!(
                "Image is larger than {} MB",
                MARKDOWN_IMAGE_MAX_FILE_BYTES / 1024 / 1024
            ),
        ));
    }

    let bytes = std::fs::read(&canonical).map_err(|error| {
        AppError::new(
            "markdown_image.read_failed",
            format!("Failed to read image: {}", error),
        )
    })?;
    let blob_id = binary_cache.insert(bytes, mime.to_string());
    let display_path = canonical.to_string_lossy().replace('\\', "/");

    Ok(MarkdownImagePreview {
        url: format!("http://locus-binary.localhost/blob/{}", blob_id),
        mime_type: mime.to_string(),
        byte_size,
        display_path,
    })
}

fn is_code_ext(ext: &str) -> bool {
    CODE_EXTS.contains(&ext)
}

fn lang_from_ext(ext: &str) -> Option<&'static str> {
    match ext {
        ".rs" => Some("rust"),
        ".ts" | ".tsx" => Some("typescript"),
        ".js" | ".jsx" => Some("javascript"),
        ".cs" => Some("csharp"),
        ".py" => Some("python"),
        ".go" => Some("go"),
        ".java" => Some("java"),
        ".c" | ".h" => Some("c"),
        ".cpp" | ".hpp" => Some("cpp"),
        ".lua" => Some("lua"),
        ".rb" => Some("ruby"),
        ".sh" | ".bat" => Some("shell"),
        ".json" => Some("json"),
        ".xml" | ".csproj" | ".sln" => Some("xml"),
        ".yaml" | ".yml" => Some("yaml"),
        ".toml" => Some("toml"),
        ".html" => Some("html"),
        ".css" | ".scss" | ".less" => Some("css"),
        ".vue" | ".svelte" => Some("html"),
        ".md" => Some("markdown"),
        ".sql" => Some("sql"),
        ".shader" | ".hlsl" | ".glsl" | ".cginc" | ".compute" => Some("hlsl"),
        ".unity" | ".prefab" | ".asset" | ".mat" | ".anim" | ".controller" | ".physicMaterial"
        | ".preset" | ".fontsettings" | ".guiskin" | ".mask" | ".flare" | ".renderTexture"
        | ".lighting" | ".meta" | ".locus-meta" => Some("yaml"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePreview {
    pub display_path: String,
    pub exists: bool,
    pub kind: String, // "text" | "binary" | "not_found"
    pub language: Option<String>,
    pub snippet: Option<String>,
    pub truncated: bool,
    pub is_unity_asset: bool,
    pub preferred_action: String, // "editor" | "unity" | "external"
    pub file_size: Option<u64>,
    pub snippet_start_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_suppressed: Option<String>,
}

const HOVER_PREVIEW_MAX_FILE_BYTES: u64 = 256 * 1024;

#[tauri::command]
pub async fn preview_workspace_file(
    file_path: String,
    line: Option<u32>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<WorkspaceFilePreview, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let canonical = match resolve_openable_file_ref_path(&file_path, &working_dir) {
        Ok(p) => p,
        Err(_) => {
            return Ok(WorkspaceFilePreview {
                display_path: file_path,
                exists: false,
                kind: "not_found".into(),
                language: None,
                snippet: None,
                truncated: false,
                is_unity_asset: false,
                preferred_action: "external".into(),
                file_size: None,
                snippet_start_line: 1,
                preview_suppressed: None,
            });
        }
    };

    if !canonical.is_file() {
        return Ok(WorkspaceFilePreview {
            display_path: file_path,
            exists: false,
            kind: "not_found".into(),
            language: None,
            snippet: None,
            truncated: false,
            is_unity_asset: false,
            preferred_action: "external".into(),
            file_size: None,
            snippet_start_line: 1,
            preview_suppressed: None,
        });
    }

    let metadata =
        std::fs::metadata(&canonical).map_err(|e| format!("Failed to read metadata: {}", e))?;
    let file_size = metadata.len();

    let ext = ext_lower(&file_path).unwrap_or_default();
    let is_binary = is_binary_ext(&ext);
    let is_code = is_code_ext(&ext);
    let language = lang_from_ext(&ext).map(|s| s.to_string());

    // Unity asset: under Assets/ or Packages/ but not a code file
    let is_unity_asset =
        (file_path.starts_with("Assets/") || file_path.starts_with("Packages/")) && !is_code;

    let preferred_action = if is_code {
        "editor"
    } else if is_unity_asset {
        "unity"
    } else {
        "external"
    };

    if file_size > HOVER_PREVIEW_MAX_FILE_BYTES {
        return Ok(WorkspaceFilePreview {
            display_path: file_path,
            exists: true,
            kind: if is_binary { "binary" } else { "text" }.into(),
            language,
            snippet: None,
            truncated: true,
            is_unity_asset,
            preferred_action: preferred_action.into(),
            file_size: Some(file_size),
            snippet_start_line: 1,
            preview_suppressed: Some("largeFile".into()),
        });
    }

    if is_binary {
        return Ok(WorkspaceFilePreview {
            display_path: file_path,
            exists: true,
            kind: "binary".into(),
            language,
            snippet: None,
            truncated: false,
            is_unity_asset,
            preferred_action: preferred_action.into(),
            file_size: Some(file_size),
            snippet_start_line: 1,
            preview_suppressed: None,
        });
    }

    // Text file: read snippet
    const MAX_LINES: usize = 50;
    const MAX_BYTES: u64 = 5 * 1024;

    let content = std::fs::read_to_string(&canonical).unwrap_or_default();
    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();

    // Determine snippet window based on optional line number
    let (start_idx, end_idx) = if let Some(target) = line {
        let target = target.saturating_sub(1) as usize; // 0-indexed
        let half = MAX_LINES / 2;
        let start = target.saturating_sub(half);
        let end = (start + MAX_LINES).min(total_lines);
        (start, end)
    } else {
        (0, MAX_LINES.min(total_lines))
    };

    let window = &all_lines[start_idx..end_idx];
    let mut snippet = String::new();
    let mut byte_count = 0u64;
    let mut truncated = end_idx < total_lines;

    for line_str in window {
        if byte_count + line_str.len() as u64 + 1 > MAX_BYTES {
            truncated = true;
            break;
        }
        if !snippet.is_empty() {
            snippet.push('\n');
            byte_count += 1;
        }
        snippet.push_str(line_str);
        byte_count += line_str.len() as u64;
    }

    Ok(WorkspaceFilePreview {
        display_path: file_path,
        exists: true,
        kind: "text".into(),
        language,
        snippet: Some(snippet),
        truncated,
        is_unity_asset,
        preferred_action: preferred_action.into(),
        file_size: Some(file_size),
        snippet_start_line: (start_idx + 1) as u32,
        preview_suppressed: None,
    })
}

// ══════════════════════════════════════════════════════════════════
// ══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolLoadConfig {
    #[serde(default)]
    pub direct_load: HashMap<String, bool>,
}

fn tool_load_config_path(working_dir: &str, agent_id: &str) -> std::path::PathBuf {
    let agent_id = canonical_agent_id(agent_id);
    std::path::Path::new(working_dir)
        .join("Locus")
        .join("agent")
        .join(agent_id)
        .join("tool_load_config.json")
}

pub fn load_tool_load_config(working_dir: &str, agent_id: &str) -> AgentToolLoadConfig {
    let path = tool_load_config_path(working_dir, agent_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => AgentToolLoadConfig::default(),
    }
}

fn save_tool_load_config(
    working_dir: &str,
    agent_id: &str,
    config: &AgentToolLoadConfig,
) -> Result<(), String> {
    let path = tool_load_config_path(working_dir, agent_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    let json =
        serde_json::to_string_pretty(config).map_err(|e| format!("Serialization failed: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save config: {}", e))?;
    Ok(())
}

fn load_app_tool_load_config(
    app_agent_dir: &Option<std::path::PathBuf>,
    agent_id: &str,
) -> AgentToolLoadConfig {
    let agent_id = canonical_agent_id(agent_id);
    if let Some(app_dir) = app_agent_dir {
        let path = app_dir.join(agent_id).join("tool_load_config.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&content).unwrap_or_default();
        }
    }
    AgentToolLoadConfig::default()
}

pub fn merged_tool_load_config_for_agent(
    app_agent_dir: &Option<std::path::PathBuf>,
    working_dir: &str,
    agent_id: &str,
) -> AgentToolLoadConfig {
    let agent_id = canonical_agent_id(agent_id);
    let mut config = load_app_tool_load_config(app_agent_dir, agent_id);
    if !working_dir.trim().is_empty() {
        let ws_config = load_tool_load_config(working_dir, agent_id);
        for (name, direct_load) in ws_config.direct_load {
            config.direct_load.insert(name, direct_load);
        }
    }
    config
}

pub fn save_tool_direct_load_override(
    working_dir: &str,
    agent_id: &str,
    tool_name: &str,
    direct_load: bool,
    default_direct_load: bool,
) -> Result<(), String> {
    let mut config = load_tool_load_config(working_dir, agent_id);
    let key = tool_name.trim().to_string();
    if direct_load == default_direct_load {
        config.direct_load.remove(&key);
    } else {
        config.direct_load.insert(key, direct_load);
    }
    save_tool_load_config(working_dir, agent_id, &config)
}

#[tauri::command]
pub async fn set_agent_tool_direct_load(
    agent_id: String,
    tool_name: String,
    direct_load: bool,
    registry: State<'_, AgentDefRegistryState>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let agent_id = canonical_agent_id(&agent_id).to_string();
    let registry = registry.0.read().await;
    let def = registry
        .get(&agent_id)
        .ok_or_else(|| format!("Agent '{}' not found", agent_id))?;
    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Err("No working directory selected".to_string().into());
    }

    let canonical = tool_registry
        .canonical_name(&tool_name)
        .ok_or_else(|| format!("Tool '{}' not found", tool_name))?;
    if matches!(canonical.as_str(), "tool_load" | "tool_call") {
        return Err(format!("Tool '{}' load mode is fixed", canonical).into());
    }
    if !tool_registry.is_built_in(&canonical) {
        return Err(format!(
            "Tool '{}' is provided by a skill and is loaded only through skills",
            canonical
        )
        .into());
    }
    if tool_registry.default_load_mode(&canonical) == crate::tool::ToolLoadMode::Skill {
        return Err(format!(
            "Tool '{}' load mode is controlled by the tool registry",
            canonical
        )
        .into());
    }

    let enabled_for_agent = def.tools.iter().any(|name| {
        tool_registry
            .canonical_name(name)
            .is_some_and(|tool| tool == canonical)
    });
    if !enabled_for_agent {
        return Err(format!(
            "Tool '{}' is not enabled for agent '{}'",
            canonical, agent_id
        )
        .into());
    }

    save_tool_direct_load_override(
        &working_dir,
        &agent_id,
        &canonical,
        direct_load,
        tool_registry.default_load_mode(&canonical) == crate::tool::ToolLoadMode::Direct,
    )
    .map_err(Into::into)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub order: i32,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            order: 0,
        }
    }
}

pub type AgentRuleConfig = std::collections::HashMap<String, RuleConfig>;

const PLUGIN_RULE_KEY_PREFIX: &str = "plugin:";
const PLUGIN_RULE_DEFAULT_ORDER_BASE: i32 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleItem {
    pub key: String,
    pub file_name: String,
    pub title: String,
    pub order: i32,
    pub enabled: bool,
    pub updated_at: i64,
    #[serde(default = "default_source_project")]
    pub source: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_scope: Option<String>,
}

fn default_source_project() -> String {
    "project".to_string()
}

#[derive(Debug, Clone)]
pub struct AgentRuleFileEntry {
    pub key: String,
    pub file_name: String,
    pub title: String,
    pub order: i32,
    pub enabled: bool,
    pub updated_at: i64,
    pub source: String,
    pub read_only: bool,
    pub path: std::path::PathBuf,
    pub plugin_id: Option<String>,
    pub plugin_scope: Option<String>,
}

impl AgentRuleFileEntry {
    fn into_item(self) -> RuleItem {
        RuleItem {
            key: self.key,
            file_name: self.file_name,
            title: self.title,
            order: self.order,
            enabled: self.enabled,
            updated_at: self.updated_at,
            source: self.source,
            read_only: self.read_only,
            plugin_id: self.plugin_id,
            plugin_scope: self.plugin_scope,
        }
    }
}

fn rules_dir(working_dir: &str, agent_id: &str) -> Result<std::path::PathBuf, String> {
    let agent_id = canonical_agent_id(agent_id);
    let dir = std::path::Path::new(working_dir)
        .join("Locus")
        .join("agent")
        .join(agent_id)
        .join("rule");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create rules directory: {}", e))?;
    }
    Ok(dir)
}

fn rule_config_path(working_dir: &str, agent_id: &str) -> std::path::PathBuf {
    let agent_id = canonical_agent_id(agent_id);
    std::path::Path::new(working_dir)
        .join("Locus")
        .join("agent")
        .join(agent_id)
        .join("rule_config.json")
}

pub fn load_rule_config(working_dir: &str, agent_id: &str) -> AgentRuleConfig {
    let path = rule_config_path(working_dir, agent_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => AgentRuleConfig::new(),
    }
}

fn save_rule_config(
    working_dir: &str,
    agent_id: &str,
    configs: &AgentRuleConfig,
) -> Result<(), String> {
    let path = rule_config_path(working_dir, agent_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(configs)
        .map_err(|e| format!("Serialization failed: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save config: {}", e))?;
    Ok(())
}

fn load_app_rule_config(
    app_agent_dir: &Option<std::path::PathBuf>,
    agent_id: &str,
) -> AgentRuleConfig {
    let agent_id = canonical_agent_id(agent_id);
    if let Some(app_dir) = app_agent_dir {
        let path = app_dir.join(agent_id).join("rule_config.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return serde_json::from_str(&content).unwrap_or_default();
        }
    }
    AgentRuleConfig::new()
}

pub fn merged_rule_config_for_agent(
    app_agent_dir: &Option<std::path::PathBuf>,
    working_dir: &str,
    agent_id: &str,
) -> AgentRuleConfig {
    let agent_id = canonical_agent_id(agent_id);
    let mut configs = load_app_rule_config(app_agent_dir, agent_id);
    if !working_dir.trim().is_empty() {
        let ws_configs = load_rule_config(working_dir, agent_id);
        for (k, v) in ws_configs {
            configs.insert(k, v);
        }
    }
    configs
}

fn hide_from_static_rule_list(agent_id: &str, file_name: &str) -> bool {
    agent_id == "dev" && matches!(file_name, "知识库使用.md" | "知识维护.md")
}

fn rule_config_or_default(
    configs: &std::collections::HashMap<String, RuleConfig>,
    key: &str,
    enabled: bool,
    order: i32,
) -> RuleConfig {
    configs
        .get(key)
        .cloned()
        .unwrap_or(RuleConfig { enabled, order })
}

fn plugin_rule_key(
    scope: crate::plugin::PluginInstallScope,
    plugin_id: &str,
    rel_path: &str,
) -> String {
    format!(
        "{}{}:{}:{}",
        PLUGIN_RULE_KEY_PREFIX,
        scope.as_str(),
        plugin_id,
        rel_path.replace('\\', "/")
    )
}

fn plugin_rule_file_paths(
    source: &crate::plugin::PluginComponentSource,
) -> Vec<(std::path::PathBuf, String)> {
    if source.root.is_file() {
        if source.root.extension().and_then(|value| value.to_str()) == Some("md") {
            return vec![(source.root.clone(), source.rel_path.clone())];
        }
        return Vec::new();
    }

    if !source.root.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&source.root) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
                return None;
            }
            let file_name = path.file_name()?.to_str()?.to_string();
            let rel_path = format!("{}/{}", source.rel_path.trim_end_matches('/'), file_name);
            Some((path, rel_path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| a.1.cmp(&b.1));
    files
}

fn scan_static_rules_dir(
    agent_id: &str,
    dir: &std::path::Path,
    source: &str,
    configs: &std::collections::HashMap<String, RuleConfig>,
    seen_names: &mut std::collections::HashSet<String>,
    items: &mut Vec<AgentRuleFileEntry>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !seen_names.insert(file_name.clone()) {
                continue;
            }
            if hide_from_static_rule_list(agent_id, &file_name) {
                continue;
            }
            let title = extract_title_from_file(&path, &file_name);
            let updated_at = get_updated_at(&path);
            let cfg = configs.get(&file_name).cloned().unwrap_or_default();

            items.push(AgentRuleFileEntry {
                key: file_name.clone(),
                file_name,
                title,
                order: cfg.order,
                enabled: cfg.enabled,
                updated_at,
                source: source.to_string(),
                read_only: false,
                path,
                plugin_id: None,
                plugin_scope: None,
            });
        }
    }
}

fn scan_plugin_rule_sources(
    working_dir: &str,
    configs: &std::collections::HashMap<String, RuleConfig>,
    items: &mut Vec<AgentRuleFileEntry>,
) {
    let mut seen_keys = std::collections::HashSet::new();
    let mut default_order = PLUGIN_RULE_DEFAULT_ORDER_BASE;
    for source in crate::plugin::installed_rule_sources(working_dir) {
        for (path, rel_path) in plugin_rule_file_paths(&source) {
            let key = plugin_rule_key(source.scope, &source.plugin_id, &rel_path);
            if !seen_keys.insert(key.clone()) {
                continue;
            }
            let Some(file_name) = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let title = extract_title_from_file(&path, &file_name);
            let updated_at = get_updated_at(&path);
            let cfg = rule_config_or_default(configs, &key, false, default_order);
            default_order = default_order.saturating_add(1);

            items.push(AgentRuleFileEntry {
                key,
                file_name,
                title,
                order: cfg.order,
                enabled: cfg.enabled,
                updated_at,
                source: source.scope.component_source().to_string(),
                read_only: true,
                path,
                plugin_id: Some(source.plugin_id.clone()),
                plugin_scope: Some(source.scope.as_str().to_string()),
            });
        }
    }
}

pub fn collect_agent_rule_files(
    app_agent_dir: &Option<std::path::PathBuf>,
    working_dir: &str,
    agent_id: &str,
    create_project_dir: bool,
) -> Result<Vec<AgentRuleFileEntry>, String> {
    let agent_id = canonical_agent_id(agent_id).to_string();
    let configs = merged_rule_config_for_agent(app_agent_dir, working_dir, &agent_id);
    let mut items = Vec::new();
    let mut seen_static_names = std::collections::HashSet::new();

    if !working_dir.trim().is_empty() {
        if create_project_dir {
            let project_dir = rules_dir(working_dir, &agent_id)?;
            scan_static_rules_dir(
                &agent_id,
                &project_dir,
                "project",
                &configs,
                &mut seen_static_names,
                &mut items,
            );
        } else {
            let project_dir = std::path::Path::new(working_dir)
                .join("Locus")
                .join("agent")
                .join(&agent_id)
                .join("rule");
            if project_dir.is_dir() {
                scan_static_rules_dir(
                    &agent_id,
                    &project_dir,
                    "project",
                    &configs,
                    &mut seen_static_names,
                    &mut items,
                );
            }
        }
    }

    scan_plugin_rule_sources(working_dir, &configs, &mut items);

    if let Some(app_dir) = app_agent_dir {
        let app_rules = app_dir.join(&agent_id).join("rule");
        if app_rules.is_dir() {
            scan_static_rules_dir(
                &agent_id,
                &app_rules,
                "app",
                &configs,
                &mut seen_static_names,
                &mut items,
            );
        }
    }

    items.sort_by(|a, b| a.order.cmp(&b.order).then(a.key.cmp(&b.key)));
    Ok(items)
}

#[tauri::command]
pub async fn list_rules(
    agent_id: String,
    workspace: State<'_, Arc<Workspace>>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<Vec<RuleItem>, AppError> {
    let agent_id = canonical_agent_id(&agent_id).to_string();
    let working_dir = workspace.path.read().await.clone();
    let items = collect_agent_rule_files(app_agent_dir.0.as_ref(), &working_dir, &agent_id, true)?
        .into_iter()
        .map(AgentRuleFileEntry::into_item)
        .collect();
    Ok(items)
}

#[tauri::command]
pub async fn save_rule(
    agent_id: String,
    file_name: String,
    content: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<RuleItem, AppError> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err("Invalid file name".to_string().into());
    }

    let file_name = if file_name.ends_with(".md") {
        file_name
    } else {
        format!("{}.md", file_name)
    };

    let working_dir = workspace.path.read().await.clone();
    let dir = rules_dir(&working_dir, &agent_id)?;
    let path = dir.join(&file_name);

    let is_new = !path.is_file();
    std::fs::write(&path, &content).map_err(|e| format!("Failed to save rule: {}", e))?;

    let mut configs = load_rule_config(&working_dir, &agent_id);
    if is_new && !configs.contains_key(&file_name) {
        let max_order = configs.values().map(|c| c.order).max().unwrap_or(-1);
        configs.insert(
            file_name.clone(),
            RuleConfig {
                enabled: true,
                order: max_order + 1,
            },
        );
        save_rule_config(&working_dir, &agent_id, &configs)?;
    }

    let title = extract_title_from_file(&path, &file_name);
    let updated_at = get_updated_at(&path);
    let cfg = configs.get(&file_name).cloned().unwrap_or_default();

    Ok(RuleItem {
        key: file_name.clone(),
        file_name,
        title,
        order: cfg.order,
        enabled: cfg.enabled,
        updated_at,
        source: "project".to_string(),
        read_only: false,
        plugin_id: None,
        plugin_scope: None,
    })
}

#[tauri::command]
pub async fn read_rule(
    agent_id: String,
    file_name: String,
    workspace: State<'_, Arc<Workspace>>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<String, AppError> {
    let agent_id = canonical_agent_id(&agent_id).to_string();
    let working_dir = workspace.path.read().await.clone();
    if file_name.starts_with(PLUGIN_RULE_KEY_PREFIX) {
        let entries =
            collect_agent_rule_files(app_agent_dir.0.as_ref(), &working_dir, &agent_id, false)?;
        if let Some(entry) = entries.into_iter().find(|entry| entry.key == file_name) {
            return std::fs::read_to_string(&entry.path)
                .map_err(|e| format!("Failed to read rule: {}", e))
                .map_err(Into::into);
        }
        return Err(format!("Rule file not found: {}", file_name).into());
    }
    if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
        return Err("Invalid file name".to_string().into());
    }
    let project_path = rules_dir(&working_dir, &agent_id)?.join(&file_name);
    if project_path.is_file() {
        return std::fs::read_to_string(&project_path)
            .map_err(|e| format!("Failed to read rule: {}", e))
            .map_err(Into::into);
    }
    if let Some(app_dir) = app_agent_dir.0.as_ref() {
        let app_path = app_dir.join(&agent_id).join("rule").join(&file_name);
        if app_path.is_file() {
            return std::fs::read_to_string(&app_path)
                .map_err(|e| format!("Failed to read rule: {}", e))
                .map_err(Into::into);
        }
    }
    Err(format!("Rule file not found: {}", file_name).into())
}

#[tauri::command]
pub async fn delete_rule(
    agent_id: String,
    file_name: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
        return Err("Invalid file name".to_string().into());
    }
    let working_dir = workspace.path.read().await.clone();
    let dir = rules_dir(&working_dir, &agent_id)?;
    let path = dir.join(&file_name);
    if !path.is_file() {
        return Err(format!("Rule file not found: {}", file_name).into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete rule: {}", e))?;

    let mut configs = load_rule_config(&working_dir, &agent_id);
    configs.remove(&file_name);
    save_rule_config(&working_dir, &agent_id, &configs)?;
    Ok(())
}

#[tauri::command]
pub async fn set_rule_enabled(
    agent_id: String,
    file_name: String,
    enabled: bool,
    workspace: State<'_, Arc<Workspace>>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<(), AppError> {
    if file_name.trim().is_empty() {
        return Err("Invalid rule key".to_string().into());
    }
    let agent_id = canonical_agent_id(&agent_id).to_string();
    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Err("No working directory selected".to_string().into());
    }
    let existing_entry =
        collect_agent_rule_files(app_agent_dir.0.as_ref(), &working_dir, &agent_id, false)?
            .into_iter()
            .find(|entry| entry.key == file_name || entry.file_name == file_name);
    let mut configs = load_rule_config(&working_dir, &agent_id);
    let max_order = configs.values().map(|cfg| cfg.order).max().unwrap_or(-1);
    let cfg = configs.entry(file_name).or_insert_with(|| RuleConfig {
        enabled,
        order: existing_entry
            .as_ref()
            .map(|entry| entry.order)
            .unwrap_or(max_order.saturating_add(1)),
    });
    cfg.enabled = enabled;
    save_rule_config(&working_dir, &agent_id, &configs).map_err(Into::into)
}

#[tauri::command]
pub async fn set_rule_order(
    agent_id: String,
    file_names: Vec<String>,
    workspace: State<'_, Arc<Workspace>>,
    app_agent_dir: State<'_, crate::AppAgentDir>,
) -> Result<(), AppError> {
    let agent_id = canonical_agent_id(&agent_id).to_string();
    let working_dir = workspace.path.read().await.clone();
    if working_dir.trim().is_empty() {
        return Err("No working directory selected".to_string().into());
    }
    let entries =
        collect_agent_rule_files(app_agent_dir.0.as_ref(), &working_dir, &agent_id, false)?;
    let mut configs = load_rule_config(&working_dir, &agent_id);
    for (i, name) in file_names.iter().enumerate() {
        let default_enabled = entries
            .iter()
            .find(|entry| entry.key == *name || entry.file_name == *name)
            .map(|entry| entry.enabled)
            .unwrap_or(true);
        let cfg = configs.entry(name.clone()).or_insert_with(|| RuleConfig {
            enabled: default_enabled,
            order: i as i32,
        });
        cfg.order = i as i32;
    }
    save_rule_config(&working_dir, &agent_id, &configs).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_store::KnowledgeInjectMode;
    use tempfile::TempDir;

    fn sample_parent_config() -> KnowledgeDirectoryConfig {
        KnowledgeDirectoryConfig {
            version: 4,
            summary: "父目录摘要".to_string(),
            inject_mode: KnowledgeInjectMode::Path,
            inherit_inject_mode: false,
            ai_maintained: true,
            inherit_ai_config: false,
            explicit_maintenance_rules: true,
            lexical_search: knowledge_store::FolderIndexRuleSetting::Enabled,
            vector_search: knowledge_store::FolderIndexRuleSetting::Disabled,
            inherit_to_children: true,
            allow_create_documents: true,
            allow_create_directories: true,
            allow_move_documents: true,
            allow_move_directories: true,
            maintenance_rules: "- Keep inherited child knowledge stable".to_string(),
        }
    }

    fn write_plugin_rule(workspace: &TempDir, plugin_id: &str, file_name: &str, content: &str) {
        let plugin_root = workspace
            .path()
            .join(crate::plugin::PROJECT_PLUGINS_RELATIVE)
            .join(plugin_id);
        std::fs::create_dir_all(plugin_root.join("rules")).expect("create plugin rules");
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": plugin_id,
            "name": plugin_id,
            "version": "1.0.0",
            "components": {
                "agents": [],
                "rules": [],
                "skills": [],
                "views": []
            }
        });
        std::fs::write(
            plugin_root.join(crate::plugin::PLUGIN_MANIFEST_FILE_NAME),
            serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write plugin manifest");
        std::fs::write(plugin_root.join("rules").join(file_name), content)
            .expect("write plugin rule");
    }

    #[test]
    fn plugin_rules_are_optional_per_agent() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        write_plugin_rule(
            &workspace,
            "com.example.rules",
            "risk_control.md",
            "# Risk Control\n\nUse extra caution.",
        );

        let listed =
            collect_agent_rule_files(&None, &working_dir, "dev", false).expect("collect rules");
        let plugin_rule = listed
            .iter()
            .find(|item| item.plugin_id.as_deref() == Some("com.example.rules"))
            .expect("plugin rule should be listed");

        assert_eq!(plugin_rule.file_name, "risk_control.md");
        assert_eq!(plugin_rule.source, "pluginProject");
        assert!(plugin_rule.read_only);
        assert!(!plugin_rule.enabled);
        assert!(plugin_rule
            .key
            .starts_with("plugin:project:com.example.rules:"));

        let mut config = AgentRuleConfig::new();
        config.insert(
            plugin_rule.key.clone(),
            RuleConfig {
                enabled: true,
                order: 3,
            },
        );
        save_rule_config(&working_dir, "dev", &config).expect("save rule config");

        let enabled =
            collect_agent_rule_files(&None, &working_dir, "dev", false).expect("collect enabled");
        let plugin_rule = enabled
            .iter()
            .find(|item| item.plugin_id.as_deref() == Some("com.example.rules"))
            .expect("plugin rule should still be listed");
        assert!(plugin_rule.enabled);
        assert_eq!(plugin_rule.order, 3);
    }

    #[test]
    fn resolve_workspace_reveal_path_returns_existing_file() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let file_path = temp.path().join("Assets").join("Player.cs");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "class Player {}").unwrap();

        let resolved = resolve_workspace_reveal_path("Assets/Player.cs", &working_dir).unwrap();

        assert_eq!(resolved, dunce::canonicalize(file_path).unwrap());
    }

    #[test]
    fn resolve_workspace_reveal_path_falls_back_to_existing_parent_directory() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let assets_dir = temp.path().join("Assets");
        std::fs::create_dir_all(&assets_dir).unwrap();

        let resolved =
            resolve_workspace_reveal_path("Assets/DeletedFolder/Missing.prefab", &working_dir)
                .unwrap();

        assert_eq!(resolved, dunce::canonicalize(assets_dir).unwrap());
    }

    #[test]
    fn resolve_openable_file_ref_path_allows_absolute_file() {
        let workspace = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let file_path = external.path().join("locus-temp-test.txt");
        std::fs::write(&file_path, "external").unwrap();

        let resolved =
            resolve_openable_file_ref_path(&file_path.to_string_lossy(), &working_dir).unwrap();

        assert_eq!(resolved, dunce::canonicalize(file_path).unwrap());
    }

    #[test]
    fn resolve_file_ref_reveal_path_allows_absolute_directory() {
        let workspace = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();

        let resolved =
            resolve_file_ref_reveal_path(&external.path().to_string_lossy(), &working_dir).unwrap();

        assert_eq!(resolved, dunce::canonicalize(external.path()).unwrap());
    }

    #[test]
    fn resolve_file_ref_reveal_path_falls_back_for_missing_absolute_child() {
        let workspace = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let missing = external.path().join("missing").join("file.txt");

        let resolved =
            resolve_file_ref_reveal_path(&missing.to_string_lossy(), &working_dir).unwrap();

        assert_eq!(resolved, dunce::canonicalize(external.path()).unwrap());
    }

    #[test]
    fn resolve_knowledge_reveal_path_returns_workspace_document() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let target = temp
            .path()
            .join("Locus")
            .join("knowledge")
            .join("design")
            .join("combat")
            .join("core-loop.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "# Core Loop").unwrap();

        let resolved = resolve_knowledge_reveal_path(
            &working_dir,
            None,
            KnowledgeType::Design,
            KnowledgeTargetKind::Document,
            "combat/core-loop.md",
        )
        .unwrap();

        assert_eq!(resolved, dunce::canonicalize(target).unwrap());
    }

    #[test]
    fn resolve_knowledge_reveal_path_falls_back_to_app_document() {
        let workspace = TempDir::new().unwrap();
        let app_root = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let app_knowledge_root = app_root.path().join("knowledge");
        let target = app_knowledge_root
            .join("reference")
            .join("unity")
            .join("api.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "# API").unwrap();

        let resolved = resolve_knowledge_reveal_path(
            &working_dir,
            Some(&app_knowledge_root),
            KnowledgeType::Reference,
            KnowledgeTargetKind::Document,
            "unity/api.md",
        )
        .unwrap();

        assert_eq!(resolved, dunce::canonicalize(target).unwrap());
    }

    #[test]
    fn resolve_knowledge_reveal_path_falls_back_to_app_directory() {
        let workspace = TempDir::new().unwrap();
        let app_root = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        let app_knowledge_root = app_root.path().join("knowledge");
        let target = app_knowledge_root.join("skill").join("workflow");
        std::fs::create_dir_all(&target).unwrap();

        let resolved = resolve_knowledge_reveal_path(
            &working_dir,
            Some(&app_knowledge_root),
            KnowledgeType::Skill,
            KnowledgeTargetKind::Directory,
            "workflow",
        )
        .unwrap();

        assert_eq!(resolved, dunce::canonicalize(target).unwrap());
    }

    #[test]
    fn resolve_knowledge_document_target_requires_md_suffix() {
        let (doc_type, path) = resolve_knowledge_document_target(None, "skill/builtin/profiler.md")
            .expect("resolve document path");
        assert_eq!(doc_type, KnowledgeType::Skill);
        assert_eq!(path, "builtin/profiler.md");

        let err = resolve_knowledge_document_target(None, "skill/builtin/profiler")
            .expect_err("document path without suffix should fail");
        assert!(err.contains(".md"));
    }

    #[test]
    fn execute_knowledge_create_allows_path_only_document_creation() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let result = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Document,
                path: "design/core-loop.md".to_string(),
                ..Default::default()
            },
        )
        .expect("create document");

        let doc = result.document.expect("document");
        assert_eq!(doc.title, "core-loop");
        assert_eq!(doc.body, "");
        assert_eq!(doc.inject_mode, KnowledgeInjectMode::Path);
        assert!(doc.inherit_inject_mode);
        assert_eq!(
            doc.inject_mode_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::TypeDefault
        );
        assert!(!doc.ai_maintained);
        assert!(doc.inherit_ai_config);
        assert_eq!(
            doc.ai_config_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::TypeDefault
        );
        assert!(!doc.summary_enabled);
        assert!(!doc.explicit_maintenance_rules);
        assert!(doc.maintenance_rules.is_none());
    }

    #[test]
    fn execute_knowledge_create_rejects_document_path_without_md_suffix() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let err = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Document,
                path: "design/core-loop".to_string(),
                ..Default::default()
            },
        )
        .expect_err("document path without suffix should fail");

        assert!(err.contains(".md"));
    }

    #[test]
    fn execute_knowledge_create_inherits_parent_rules_for_path_only_document() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        knowledge_store::create_directory(&working_dir, KnowledgeType::Design, "combat")
            .expect("create parent");
        knowledge_store::update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "combat",
            sample_parent_config(),
        )
        .expect("save parent config");

        let result = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Document,
                path: "design/combat/core-loop.md".to_string(),
                ..Default::default()
            },
        )
        .expect("create document");

        let doc = result.document.expect("document");
        assert_eq!(doc.title, "core-loop");
        assert_eq!(doc.inject_mode, KnowledgeInjectMode::Path);
        assert!(doc.inherit_inject_mode);
        assert_eq!(
            doc.inject_mode_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(doc.inject_mode_source.path.as_deref(), Some("combat"));
        assert!(doc.ai_maintained);
        assert!(doc.inherit_ai_config);
        assert_eq!(
            doc.ai_config_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(doc.ai_config_source.path.as_deref(), Some("combat"));
        assert!(doc.explicit_maintenance_rules);
        assert_eq!(
            doc.maintenance_rules.as_deref(),
            Some("- Keep inherited child knowledge stable")
        );
    }

    #[test]
    fn execute_knowledge_create_inherits_parent_rules_for_new_directory() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        knowledge_store::create_directory(&working_dir, KnowledgeType::Design, "combat")
            .expect("create parent");
        knowledge_store::update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "combat",
            sample_parent_config(),
        )
        .expect("save parent config");

        let result = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Directory,
                path: "design/combat/notes".to_string(),
                ..Default::default()
            },
        )
        .expect("create directory");

        let directory = result.directory.expect("directory");
        assert!(!directory.exists);
        assert_eq!(directory.path, "combat/notes");
        assert_eq!(directory.config.summary, "");
        assert_eq!(directory.config.inject_mode, KnowledgeInjectMode::Path);
        assert!(directory.config.inherit_inject_mode);
        assert_eq!(
            directory.inject_mode_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(directory.inject_mode_source.path.as_deref(), Some("combat"));
        assert!(directory.config.ai_maintained);
        assert!(directory.config.inherit_ai_config);
        assert_eq!(
            directory.ai_config_source.kind,
            knowledge_store::KnowledgeConfigSourceKind::ParentDirectory
        );
        assert_eq!(directory.ai_config_source.path.as_deref(), Some("combat"));
        assert!(directory.config.explicit_maintenance_rules);
        assert_eq!(
            directory.config.maintenance_rules,
            "- Keep inherited child knowledge stable"
        );
    }

    #[test]
    fn execute_knowledge_create_allows_memory_documents_to_opt_out_of_inherited_ai_config() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let result = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Document,
                path: "memory/project-understanding.md".to_string(),
                document: Some(KnowledgeDocumentPatch {
                    inherit_ai_config: Some(false),
                    ai_maintained: Some(false),
                    explicit_maintenance_rules: Some(false),
                    body: Some(Some("长期项目记忆".to_string())),
                    maintenance_rules: Some(None),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .expect("create memory document");

        let doc = result.document.expect("document");
        assert!(!doc.ai_maintained);
        assert!(!doc.inherit_ai_config);
        assert!(!doc.explicit_maintenance_rules);
        assert!(doc.maintenance_rules.is_none());

        let rendered = knowledge_store::read_document_part(
            &working_dir,
            KnowledgeType::Memory,
            "project-understanding.md",
            "full",
        )
        .expect("read full");
        assert!(!rendered.contains("## Maintenance Rules"));
    }

    #[test]
    fn execute_knowledge_create_rejects_document_when_parent_disallows_creation() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let mut parent = sample_parent_config();
        parent.allow_create_documents = false;
        knowledge_store::create_directory(&working_dir, KnowledgeType::Design, "combat")
            .expect("create parent");
        knowledge_store::update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "combat",
            parent,
        )
        .expect("save parent config");

        let err = execute_knowledge_create_request(
            &working_dir,
            KnowledgeCreateRequest {
                kind: KnowledgeTargetKind::Document,
                path: "design/combat/core-loop.md".to_string(),
                ..Default::default()
            },
        )
        .expect_err("create should fail");

        assert!(err.contains("does not allow creating child documents"));
    }
}
