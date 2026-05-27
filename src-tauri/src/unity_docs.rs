use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{
    self, default_directory_config_for_type, delete_directory_config_sidecars,
    update_directory_config, update_directory_external_sources, FolderIndexRuleSetting,
    KnowledgeDirectoryConfig, KnowledgeDocument, KnowledgeExternalSource, KnowledgeInjectMode,
    KnowledgeSourceProvider, KnowledgeType,
};

pub const UNITY_REFERENCE_MANAGED_DIR: &str = "unity-official-docs";
pub const UNITY_REFERENCE_MANAGED_PATH: &str = "reference/unity-official-docs";
const UNITY_REFERENCE_TEMP_DIR: &str = ".unity-official-docs-import";
const UNITY_REFERENCE_CACHE_DIR: &str = "unity_reference_docs";
const UNITY_REFERENCE_MANIFEST_FILE: &str = "unity_reference_docs_manifest.json";
const UNITY_REFERENCE_STORE_FILE: &str = "unity_reference_docs.db";
const UNITY_REFERENCE_TEMP_STORE_FILE: &str = "unity_reference_docs.import.db";
const UNITY_REFERENCE_DIRECTORY_CONFIG_SUFFIX: &str = ".locus-meta";
const UNITY_REFERENCE_LEGACY_DIRECTORY_CONFIG_SUFFIX: &str = ".meta";
const DOCUMENT_DESERIALIZE_PARALLEL_THRESHOLD: usize = 64;
const UNITY_REFERENCE_CONVERT_PARALLEL_THRESHOLD: usize = 32;
const UNITY_REFERENCE_CONVERT_MAX_WORKERS: usize = 8;
const UNITY_REFERENCE_CONVERT_MIN_BATCH_SIZE: usize = 48;
const UNITY_REFERENCE_CONVERT_BATCHES_PER_WORKER: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityManagedDirectoryStat {
    pub path: String,
    pub direct_child_count: usize,
    pub descendant_document_count: usize,
    #[serde(default)]
    pub direct_document_count: usize,
    #[serde(default)]
    pub child_dir_count: usize,
    #[serde(default)]
    pub descendant_byte_size: u64,
    #[serde(default)]
    pub descendant_estimated_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityManagedStoreSummary {
    pub managed_path: String,
    pub fingerprint: String,
    pub document_count: usize,
    pub directory_count: usize,
    pub total_byte_size: u64,
    pub manual_doc_count: usize,
    pub script_reference_doc_count: usize,
    pub imported_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnityReferenceImportStage {
    #[default]
    Idle,
    ResolvingSource,
    Downloading,
    Extracting,
    Converting,
    Reconciling,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnityReferenceImportStateKind {
    #[default]
    Missing,
    Unavailable,
    MissingCurrentVersion,
    Outdated,
    Running,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityReferenceImportLastOutcome {
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum UnityReferenceImportLocale {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl UnityReferenceImportLocale {
    fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    fn display_name_zh(self) -> &'static str {
        match self {
            Self::En => "英文",
            Self::ZhCn => "中文",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityReferenceImportStatus {
    pub state: UnityReferenceImportStateKind,
    pub stage: UnityReferenceImportStage,
    pub running: bool,
    pub project_version: Option<String>,
    pub docs_version: Option<String>,
    pub selected_locale: Option<String>,
    pub imported_project_version: Option<String>,
    pub imported_docs_version: Option<String>,
    pub imported_locale: Option<String>,
    pub imported_at: Option<i64>,
    pub imported_doc_count: u32,
    pub managed_path: String,
    pub progress: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub processed_docs: u32,
    pub total_docs: Option<u32>,
    pub current_path: Option<String>,
    pub source_url: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub last_outcome: Option<UnityReferenceImportLastOutcome>,
}

impl Default for UnityReferenceImportStatus {
    fn default() -> Self {
        Self {
            state: UnityReferenceImportStateKind::Missing,
            stage: UnityReferenceImportStage::Idle,
            running: false,
            project_version: None,
            docs_version: None,
            selected_locale: None,
            imported_project_version: None,
            imported_docs_version: None,
            imported_locale: None,
            imported_at: None,
            imported_doc_count: 0,
            managed_path: UNITY_REFERENCE_MANAGED_PATH.to_string(),
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            processed_docs: 0,
            total_docs: None,
            current_path: None,
            source_url: None,
            message: String::new(),
            error: None,
            last_outcome: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnityReferenceImportRuntime {
    pub working_dir: String,
    pub status: UnityReferenceImportStatus,
    pub cancel_requested: Arc<AtomicBool>,
}

impl Default for UnityReferenceImportRuntime {
    fn default() -> Self {
        Self {
            working_dir: String::new(),
            status: UnityReferenceImportStatus::default(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Clone, Default)]
pub struct UnityReferenceImportState(pub Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>);

#[derive(Debug)]
enum UnityReferenceImportRunError {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityReferenceImportManifest {
    project_version: String,
    docs_version: String,
    locale: String,
    imported_at: i64,
    imported_doc_count: u32,
    source_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityReferenceManagedSnapshot {
    pub managed_path: String,
    pub doc_path_prefix: String,
    pub fingerprint: String,
    pub document_count: usize,
    pub expected_document_count: usize,
}

#[derive(Debug, Clone)]
struct UnityOfflineDocSource {
    page_url: String,
    zip_url: String,
    locale: String,
}

#[derive(Debug, Clone)]
struct UnityHtmlDocCandidate {
    raw_relative_markdown_path: String,
    relative_markdown_path: String,
    html_relative_path: String,
    file_path: PathBuf,
}

#[derive(Debug, Clone)]
struct UnityExtractedZipEntry {
    original_relative_path: String,
    file_path: PathBuf,
}

pub fn read_project_unity_version(working_dir: &str) -> Result<Option<String>, String> {
    let version_path = Path::new(working_dir)
        .join("ProjectSettings")
        .join("ProjectVersion.txt");
    if !version_path.is_file() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&version_path).map_err(|e| {
        format!(
            "Failed to read Unity ProjectVersion file '{}': {}",
            version_path.display(),
            e
        )
    })?;
    Ok(content.lines().find_map(|line| {
        line.strip_prefix("m_EditorVersion:")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }))
}

pub fn derive_unity_docs_version(project_version: &str) -> Option<String> {
    let trimmed = project_version.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
            continue;
        }
        if ch == '.' {
            if current.is_empty() {
                return None;
            }
            parts.push(std::mem::take(&mut current));
            if parts.len() >= 2 {
                break;
            }
            continue;
        }
        if !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        break;
    }
    if !current.is_empty() && parts.len() < 2 {
        parts.push(current);
    }
    if parts.len() < 2 {
        return None;
    }
    Some(format!("{}.{}", parts[0], parts[1]))
}

fn normalize_requested_locale(value: Option<&str>) -> Result<UnityReferenceImportLocale, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(UnityReferenceImportLocale::En),
        Some("en") => Ok(UnityReferenceImportLocale::En),
        Some("zh-CN") => Ok(UnityReferenceImportLocale::ZhCn),
        Some(other) => Err(format!("不支持的 Unity 文档语言：{}。", other)),
    }
}

fn normalize_requested_target_path(target_path: Option<&str>) -> Option<String> {
    let normalized = target_path?.trim().trim_matches('/').replace('\\', "/");
    if normalized.is_empty() || normalized == UNITY_REFERENCE_MANAGED_DIR {
        return None;
    }
    Some(normalized)
}

pub async fn get_unity_reference_import_status(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
) -> Result<UnityReferenceImportStatus, String> {
    let project_version = read_project_unity_version(working_dir)?;
    let docs_version = project_version
        .as_deref()
        .and_then(derive_unity_docs_version);
    let runtime = state.lock().await.clone();
    let target_path = normalize_requested_target_path(target_path);

    if let Some(target_path) = target_path.as_deref() {
        let managed_path = reference_target_managed_path(target_path);
        if runtime.working_dir == working_dir
            && runtime.status.managed_path == managed_path
            && runtime.status.running
        {
            let mut running = runtime.status;
            running.project_version = project_version;
            running.docs_version = docs_version;
            return Ok(running);
        }

        let (
            imported_project_version,
            imported_docs_version,
            imported_locale,
            imported_at,
            imported_doc_count,
        ) = read_unity_target_import_snapshot(working_dir, target_path)?;

        let mut status = UnityReferenceImportStatus {
            project_version: project_version.clone(),
            docs_version: docs_version.clone(),
            selected_locale: imported_locale.clone(),
            imported_project_version,
            imported_docs_version: imported_docs_version.clone(),
            imported_locale,
            imported_at,
            imported_doc_count,
            managed_path: managed_path.clone(),
            ..UnityReferenceImportStatus::default()
        };

        if runtime.working_dir == working_dir
            && runtime.status.managed_path == managed_path
            && runtime.status.stage == UnityReferenceImportStage::Error
            && runtime.status.error.is_some()
        {
            let mut errored = runtime.status;
            errored.running = false;
            errored.project_version = project_version;
            errored.docs_version = docs_version;
            errored.selected_locale = errored
                .selected_locale
                .clone()
                .or(status.selected_locale.clone());
            errored.imported_project_version = status.imported_project_version;
            errored.imported_docs_version = status.imported_docs_version;
            errored.imported_locale = status.imported_locale;
            errored.imported_at = status.imported_at;
            errored.imported_doc_count = status.imported_doc_count;
            errored.state = UnityReferenceImportStateKind::Error;
            return Ok(errored);
        }

        match (
            status.project_version.as_ref(),
            status.docs_version.as_ref(),
        ) {
            (Some(_), Some(current_docs_version)) => {
                if let Some(imported_docs_version) = status.imported_docs_version.as_ref() {
                    if imported_docs_version == current_docs_version {
                        status.state = UnityReferenceImportStateKind::Ready;
                        status.stage = UnityReferenceImportStage::Ready;
                        status.message = "Unity 文档已导入，可直接检索。".to_string();
                    } else {
                        status.state = UnityReferenceImportStateKind::Outdated;
                        status.message =
                            "已存在旧版本导入结果，可重新导入当前项目版本。".to_string();
                    }
                } else {
                    status.state = UnityReferenceImportStateKind::MissingCurrentVersion;
                    status.message = "尚未导入当前项目对应版本的 Unity 文档。".to_string();
                }
            }
            _ => {
                status.state = UnityReferenceImportStateKind::Unavailable;
                status.message = "未检测到当前项目的 Unity 编辑器版本。".to_string();
            }
        }

        if runtime.working_dir == working_dir
            && runtime.status.managed_path == managed_path
            && runtime.status.last_outcome == Some(UnityReferenceImportLastOutcome::Cancelled)
        {
            status.last_outcome = Some(UnityReferenceImportLastOutcome::Cancelled);
            status.message = "已取消 Unity 文档导入。".to_string();
        }

        return Ok(status);
    }

    if runtime.working_dir == working_dir && runtime.status.running {
        let mut running = runtime.status;
        running.project_version = project_version;
        running.docs_version = docs_version;
        return Ok(running);
    }

    let manifest = read_manifest(working_dir)?;

    let mut status = UnityReferenceImportStatus {
        project_version: project_version.clone(),
        docs_version: docs_version.clone(),
        selected_locale: manifest.as_ref().map(|item| item.locale.clone()),
        imported_project_version: manifest.as_ref().map(|item| item.project_version.clone()),
        imported_docs_version: manifest.as_ref().map(|item| item.docs_version.clone()),
        imported_locale: manifest.as_ref().map(|item| item.locale.clone()),
        imported_at: manifest.as_ref().map(|item| item.imported_at),
        imported_doc_count: manifest
            .as_ref()
            .map(|item| item.imported_doc_count)
            .unwrap_or(0),
        source_url: manifest.as_ref().map(|item| item.source_url.clone()),
        ..UnityReferenceImportStatus::default()
    };

    if runtime.working_dir == working_dir
        && runtime.status.stage == UnityReferenceImportStage::Error
        && runtime.status.error.is_some()
    {
        let mut errored = runtime.status;
        errored.running = false;
        errored.project_version = project_version;
        errored.docs_version = docs_version;
        errored.selected_locale = errored
            .selected_locale
            .clone()
            .or(status.selected_locale.clone());
        errored.imported_project_version = status.imported_project_version;
        errored.imported_docs_version = status.imported_docs_version;
        errored.imported_locale = status.imported_locale;
        errored.imported_at = status.imported_at;
        errored.imported_doc_count = status.imported_doc_count;
        errored.state = UnityReferenceImportStateKind::Error;
        return Ok(errored);
    }

    match (
        status.project_version.as_ref(),
        status.docs_version.as_ref(),
    ) {
        (Some(_), Some(current_docs_version)) => {
            if let Some(imported_docs_version) = status.imported_docs_version.as_ref() {
                if imported_docs_version == current_docs_version {
                    status.state = UnityReferenceImportStateKind::Ready;
                    status.stage = UnityReferenceImportStage::Ready;
                    status.message = "Unity 文档已导入，可直接检索。".to_string();
                } else {
                    status.state = UnityReferenceImportStateKind::Outdated;
                    status.message = "已存在旧版本导入结果，可重新导入当前项目版本。".to_string();
                }
            } else {
                status.state = UnityReferenceImportStateKind::MissingCurrentVersion;
                status.message = "尚未导入当前项目对应版本的 Unity 文档。".to_string();
            }
        }
        _ => {
            status.state = UnityReferenceImportStateKind::Unavailable;
            status.message = "未检测到当前项目的 Unity 编辑器版本。".to_string();
        }
    }

    if runtime.working_dir == working_dir
        && runtime.status.last_outcome == Some(UnityReferenceImportLastOutcome::Cancelled)
    {
        status.last_outcome = Some(UnityReferenceImportLastOutcome::Cancelled);
        status.message = "已取消 Unity 文档导入。".to_string();
    }

    Ok(status)
}

pub async fn start_unity_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    target_path: Option<String>,
    requested_locale: Option<String>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
) -> Result<UnityReferenceImportStatus, String> {
    let target_path = normalize_requested_target_path(target_path.as_deref());
    if let Some(existing_path) = existing_unity_binding_path(&working_dir)? {
        match target_path.as_deref() {
            Some(requested_path) if requested_path != existing_path => {
                return Err(format!(
                    "当前项目已经在 reference/{} 导入 Unity 文档，请直接更新该目录。",
                    existing_path
                ));
            }
            None if existing_path != UNITY_REFERENCE_MANAGED_DIR => {
                return Err(format!(
                    "当前项目已经在 reference/{} 导入 Unity 文档，请直接更新该目录。",
                    existing_path
                ));
            }
            _ => {}
        }
    }
    if target_path.is_some() {
        return Err(format!(
            "Unity 文档仅支持托管目录 {}，请改为导入默认托管目录。",
            UNITY_REFERENCE_MANAGED_PATH
        ));
    }
    let project_version = read_project_unity_version(&working_dir)?
        .ok_or_else(|| "当前项目缺少 ProjectVersion.txt，无法确定 Unity 文档版本。".to_string())?;
    let docs_version = derive_unity_docs_version(&project_version)
        .ok_or_else(|| format!("无法从 Unity 版本 '{}' 推导离线文档版本。", project_version))?;
    let selected_locale = normalize_requested_locale(requested_locale.as_deref())?;
    let managed_path = target_path
        .as_deref()
        .map(reference_target_managed_path)
        .unwrap_or_else(|| UNITY_REFERENCE_MANAGED_PATH.to_string());

    let cancel_requested = {
        let mut runtime = state.lock().await;
        if runtime.status.running {
            return Err("Unity 文档导入任务仍在进行中。".to_string());
        }
        runtime.working_dir = working_dir.clone();
        runtime.cancel_requested.store(false, Ordering::Relaxed);
        runtime.status = UnityReferenceImportStatus {
            state: UnityReferenceImportStateKind::Running,
            stage: UnityReferenceImportStage::ResolvingSource,
            running: true,
            project_version: Some(project_version.clone()),
            docs_version: Some(docs_version.clone()),
            selected_locale: Some(selected_locale.as_str().to_string()),
            message: "正在解析 Unity 官方离线文档来源。".to_string(),
            managed_path: managed_path.clone(),
            current_path: Some(managed_path.clone()),
            ..UnityReferenceImportStatus::default()
        };
        runtime.cancel_requested.clone()
    };

    let state_for_task = state.clone();
    let app_handle_for_task = app_handle.clone();
    let working_dir_for_task = working_dir.clone();
    let target_path_for_task = target_path.clone();
    tauri::async_runtime::spawn(async move {
        match run_unity_reference_import(
            app_handle_for_task,
            working_dir_for_task.clone(),
            target_path_for_task.clone(),
            project_version,
            docs_version,
            selected_locale,
            knowledge_index_state,
            state_for_task.clone(),
            cancel_requested,
        )
        .await
        {
            Ok(()) => {}
            Err(UnityReferenceImportRunError::Cancelled) => {
                let _ = cleanup_import_runtime_artifacts(&working_dir_for_task);
                mark_runtime_cancelled(&state_for_task, &working_dir_for_task).await;
            }
            Err(UnityReferenceImportRunError::Failed(error)) => {
                let _ = cleanup_import_runtime_artifacts(&working_dir_for_task);
                let mut runtime = state_for_task.lock().await;
                if runtime.working_dir == working_dir_for_task {
                    runtime.cancel_requested.store(false, Ordering::Relaxed);
                    runtime.status.running = false;
                    runtime.status.state = UnityReferenceImportStateKind::Error;
                    runtime.status.stage = UnityReferenceImportStage::Error;
                    runtime.status.error = Some(error.clone());
                    runtime.status.message = error;
                    runtime.status.progress = None;
                }
            }
        }
    });

    get_unity_reference_import_status(&working_dir, target_path.as_deref(), state).await
}

pub async fn cancel_unity_reference_import(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
) -> Result<UnityReferenceImportStatus, String> {
    let target_path = normalize_requested_target_path(target_path);
    let mut runtime = state.lock().await;
    let target_matches = target_path
        .as_deref()
        .map(reference_target_managed_path)
        .map(|value| runtime.status.managed_path == value)
        .unwrap_or(true);
    if runtime.working_dir != working_dir || !runtime.status.running || !target_matches {
        drop(runtime);
        return get_unity_reference_import_status(working_dir, target_path.as_deref(), state).await;
    }

    runtime.cancel_requested.store(true, Ordering::Relaxed);
    runtime.status.message = "正在取消 Unity 文档导入。".to_string();
    runtime.status.error = None;
    runtime.status.current_path = None;
    runtime.status.progress = None;
    Ok(runtime.status.clone())
}

pub async fn delete_unity_reference_docs(
    app_handle: AppHandle,
    working_dir: String,
    target_path: Option<String>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
) -> Result<UnityReferenceImportStatus, String> {
    let target_path = normalize_requested_target_path(target_path.as_deref());
    let managed_path = target_path
        .as_deref()
        .map(reference_target_managed_path)
        .unwrap_or_else(|| UNITY_REFERENCE_MANAGED_PATH.to_string());
    {
        let runtime = state.lock().await;
        if runtime.working_dir == working_dir
            && runtime.status.running
            && runtime.status.managed_path == managed_path
        {
            return Err("Unity 文档导入任务仍在进行中。".to_string());
        }
    }

    if let Some(target_path) = target_path.as_deref() {
        delete_target_reference_import_artifacts(&working_dir, target_path)?;
    } else {
        let _removed_any = delete_unity_reference_import_artifacts(&working_dir)?;
        clear_runtime_status(&state, &working_dir).await;
    }

    crate::commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_delete_unity_reference_docs",
    )
    .await
    .map_err(|e| e.to_string())?;

    get_unity_reference_import_status(&working_dir, target_path.as_deref(), state).await
}

async fn run_unity_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    target_path: Option<String>,
    project_version: String,
    docs_version: String,
    selected_locale: UnityReferenceImportLocale,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    cancel_requested: Arc<AtomicBool>,
) -> Result<(), UnityReferenceImportRunError> {
    let use_legacy_managed_store = target_path.is_none();
    let target_path = target_path.unwrap_or_else(|| UNITY_REFERENCE_MANAGED_DIR.to_string());
    let managed_path = reference_target_managed_path(&target_path);
    let had_existing_managed_store = use_legacy_managed_store && has_managed_store(&working_dir);
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .user_agent("Locus/1.0 (Unity Reference Import)"),
    )
    .map_err(|e| {
        UnityReferenceImportRunError::Failed(format!("Failed to build download client: {}", e))
    })?;

    ensure_import_not_cancelled(&cancel_requested)?;
    let source = resolve_offline_source(&client, &docs_version, selected_locale)
        .await
        .map_err(UnityReferenceImportRunError::Failed)?;
    update_status(&state, &working_dir, |status| {
        status.state = UnityReferenceImportStateKind::Running;
        status.stage = UnityReferenceImportStage::Downloading;
        status.running = true;
        status.selected_locale = Some(source.locale.clone());
        status.source_url = Some(source.page_url.clone());
        status.message = format!("正在下载 Unity {} 离线文档。", docs_version);
        status.error = None;
    })
    .await;

    let cache_root = cache_root(&working_dir);
    let download_path = cache_root.join("UnityDocumentation.zip");
    let extract_root = cache_root.join("extracted");

    ensure_import_not_cancelled(&cancel_requested)?;
    prepare_cache_root(&cache_root).map_err(UnityReferenceImportRunError::Failed)?;
    if download_path.exists() {
        let _ = std::fs::remove_file(&download_path);
    }
    if extract_root.exists() {
        let _ = std::fs::remove_dir_all(&extract_root);
    }
    cleanup_temp_managed_dir(&working_dir).map_err(UnityReferenceImportRunError::Failed)?;
    cleanup_temp_managed_store(&working_dir).map_err(UnityReferenceImportRunError::Failed)?;

    download_unity_zip(
        &client,
        &source.zip_url,
        &download_path,
        cancel_requested.clone(),
        |downloaded, total| {
            let state = state.clone();
            let working_dir = working_dir.clone();
            async move {
                update_status(&state, &working_dir, |status| {
                    status.stage = UnityReferenceImportStage::Downloading;
                    status.downloaded_bytes = Some(downloaded);
                    status.total_bytes = total;
                    status.progress = total.map(|sum| {
                        if sum == 0 {
                            0.0
                        } else {
                            (downloaded as f32 / sum as f32).clamp(0.0, 1.0)
                        }
                    });
                    status.message = "正在下载 Unity 官方离线文档。".to_string();
                })
                .await;
            }
        },
    )
    .await?;

    ensure_import_not_cancelled(&cancel_requested)?;
    update_status(&state, &working_dir, |status| {
        status.stage = UnityReferenceImportStage::Extracting;
        status.progress = Some(0.0);
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.current_path = None;
        status.message = "正在解压离线文档。".to_string();
    })
    .await;
    let extracted_entries = extract_unity_zip(
        &download_path,
        &extract_root,
        &cancel_requested,
        |processed, total, current_path| {
            if processed != total && processed % 24 != 0 {
                return;
            }
            let current_path = current_path.to_string();
            update_status_from_sync(&state, &working_dir, |status| {
                status.stage = UnityReferenceImportStage::Extracting;
                status.progress = Some(count_progress_ratio(processed, total));
                status.current_path = Some(current_path.clone());
                status.message = format!("正在解压第 {} / {} 个文件。", processed, total);
            });
        },
    )?;

    ensure_import_not_cancelled(&cancel_requested)?;
    let script_reference_context = build_script_reference_path_context(&extracted_entries);
    let candidates = collect_html_doc_candidates(&extracted_entries, &script_reference_context)
        .map_err(UnityReferenceImportRunError::Failed)?;
    if candidates.is_empty() {
        return Err(UnityReferenceImportRunError::Failed(
            "离线文档压缩包中没有找到可转换的 HTML 页面。".to_string(),
        ));
    }
    let candidates = apply_manual_breadcrumb_hierarchy(&candidates, Some(&download_path))
        .map_err(UnityReferenceImportRunError::Failed)?;
    let relative_markdown_lookup = build_relative_markdown_lookup(&candidates);

    let total_docs = candidates.len() as u32;
    let temp_store = temp_managed_store_path(&working_dir);
    if use_legacy_managed_store {
        initialize_empty_managed_store(&temp_store)
            .map_err(UnityReferenceImportRunError::Failed)?;
    }
    let convert_parallelism = unity_reference_convert_parallelism(candidates.len());
    let convert_batch_size = unity_reference_convert_batch_size(convert_parallelism);
    let source_id = format!("unity-official-docs:{}:{}", docs_version, source.locale);
    let shared_source = Arc::new(source.clone());
    let shared_source_id = Arc::new(source_id.clone());
    let shared_docs_version = Arc::new(docs_version.clone());
    let shared_target_path = Arc::new(target_path.clone());
    let shared_download_path = Arc::new(Some(download_path.clone()));
    let shared_script_reference_context = Arc::new(script_reference_context);
    let shared_relative_markdown_lookup = Arc::new(relative_markdown_lookup);

    update_status(&state, &working_dir, |status| {
        status.stage = UnityReferenceImportStage::Converting;
        status.total_docs = Some(total_docs);
        status.processed_docs = 0;
        status.progress = Some(0.0);
        status.current_path = None;
        status.message = "正在将离线文档转换为 Markdown。".to_string();
    })
    .await;

    for batch_start in (0..candidates.len()).step_by(convert_batch_size) {
        ensure_import_not_cancelled(&cancel_requested)?;
        let batch_end = (batch_start + convert_batch_size).min(candidates.len());
        let batch_documents = build_reference_document_batch(
            &candidates[batch_start..batch_end],
            shared_source.clone(),
            shared_source_id.clone(),
            shared_docs_version.clone(),
            shared_target_path.clone(),
            shared_download_path.clone(),
            shared_script_reference_context.clone(),
            shared_relative_markdown_lookup.clone(),
            cancel_requested.clone(),
            convert_parallelism,
        )
        .await?;
        if use_legacy_managed_store {
            append_documents_to_store(&temp_store, &batch_documents)
                .map_err(UnityReferenceImportRunError::Failed)?;
        } else {
            let temp_root = temp_managed_dir_path(&working_dir);
            for document in batch_documents {
                let target_file = knowledge_store::document_path_in_root(
                    &temp_root,
                    KnowledgeType::Reference,
                    &document.path,
                )
                .map_err(UnityReferenceImportRunError::Failed)?;
                knowledge_store::save_document_to_path(&target_file, document)
                    .map_err(UnityReferenceImportRunError::Failed)?;
            }
        }

        let processed = batch_end as u32;
        let current_path = format!(
            "reference/{}/{}",
            target_path,
            candidates[batch_end - 1].relative_markdown_path
        );
        update_status(&state, &working_dir, |status| {
            status.stage = UnityReferenceImportStage::Converting;
            status.processed_docs = processed;
            status.total_docs = Some(total_docs);
            status.current_path = Some(current_path.clone());
            status.progress = Some((processed as f32 / total_docs as f32).clamp(0.0, 1.0));
            status.message = format!("正在转换第 {} / {} 篇文档。", processed, total_docs);
        })
        .await;
    }

    ensure_import_not_cancelled(&cancel_requested)?;
    update_status(&state, &working_dir, |status| {
        status.stage = UnityReferenceImportStage::Reconciling;
        status.current_path = None;
        status.progress = Some(0.0);
        status.message = "正在切换正式目录并重建知识索引。".to_string();
    })
    .await;

    ensure_import_not_cancelled(&cancel_requested)?;
    let imported_at = Utc::now().timestamp_millis();
    if use_legacy_managed_store {
        let store_conn = open_unity_reference_store(&temp_store)
            .map_err(UnityReferenceImportRunError::Failed)?;
        store_conn.execute_batch("VACUUM").map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to compact Unity reference store: {}",
                e
            ))
        })?;
        drop(store_conn);

        finalize_managed_store(&working_dir).map_err(UnityReferenceImportRunError::Failed)?;
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: project_version.clone(),
                docs_version: docs_version.clone(),
                locale: source.locale.clone(),
                imported_at,
                imported_doc_count: total_docs,
                source_url: source.page_url.clone(),
            },
        )
        .map_err(UnityReferenceImportRunError::Failed)?;
    } else {
        publish_target_directory_from_temp_root(&working_dir, &target_path)
            .map_err(UnityReferenceImportRunError::Failed)?;
    }
    configure_managed_directory(
        &working_dir,
        &target_path,
        &project_version,
        &docs_version,
        &source.locale,
        imported_at,
    )
    .map_err(UnityReferenceImportRunError::Failed)?;
    let _ = std::fs::remove_dir_all(&cache_root);

    let app_knowledge_dir = app_handle.state::<crate::commands::AppKnowledgeDir>();
    let mut last_reconcile_stage = String::new();
    let reconcile_result = if use_legacy_managed_store && !had_existing_managed_store {
        match crate::knowledge_index::reconcile_unity_reference_import(
            &working_dir,
            app_knowledge_dir.0.as_ref().as_ref(),
            knowledge_index_state.clone(),
            |stage, processed, total, current_file| {
                emit_reconcile_progress_update(
                    &state,
                    &working_dir,
                    &mut last_reconcile_stage,
                    stage,
                    processed,
                    total,
                    current_file,
                );
            },
        )
        .await
        {
            Ok(report) => Ok(report),
            Err(error) => {
                eprintln!(
                    "[UnityReferenceImport] bulk reconcile fallback workspace={} target={} error={}",
                    working_dir, managed_path, error
                );
                crate::knowledge_index::reconcile_workspace_internal(
                    &working_dir,
                    app_knowledge_dir.0.as_ref().as_ref(),
                    knowledge_index_state,
                    false,
                    true,
                    true,
                    |stage, processed, total, current_file| {
                        emit_reconcile_progress_update(
                            &state,
                            &working_dir,
                            &mut last_reconcile_stage,
                            stage,
                            processed,
                            total,
                            current_file,
                        );
                    },
                )
                .await
            }
        }
    } else {
        crate::knowledge_index::reconcile_workspace_internal(
            &working_dir,
            app_knowledge_dir.0.as_ref().as_ref(),
            knowledge_index_state,
            false,
            true,
            true,
            |stage, processed, total, current_file| {
                emit_reconcile_progress_update(
                    &state,
                    &working_dir,
                    &mut last_reconcile_stage,
                    stage,
                    processed,
                    total,
                    current_file,
                );
            },
        )
        .await
    };
    reconcile_result.map_err(UnityReferenceImportRunError::Failed)?;
    crate::commands::emit_knowledge_changed(
        &app_handle,
        &working_dir,
        "knowledge_import_unity_reference_docs",
    );

    update_status(&state, &working_dir, |status| {
        status.running = false;
        status.state = UnityReferenceImportStateKind::Ready;
        status.stage = UnityReferenceImportStage::Ready;
        status.imported_project_version = Some(project_version.clone());
        status.imported_docs_version = Some(docs_version.clone());
        status.selected_locale = Some(source.locale.clone());
        status.imported_locale = Some(source.locale.clone());
        status.imported_at = Some(imported_at);
        status.imported_doc_count = total_docs;
        status.processed_docs = total_docs;
        status.total_docs = Some(total_docs);
        status.progress = Some(1.0);
        status.current_path = None;
        status.managed_path = managed_path.clone();
        status.source_url = Some(source.page_url.clone());
        status.error = None;
        status.message = format!("Unity {} 文档已导入完成。", docs_version);
    })
    .await;

    Ok(())
}

async fn update_status<F>(
    state: &Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    working_dir: &str,
    update: F,
) where
    F: FnOnce(&mut UnityReferenceImportStatus),
{
    let mut runtime = state.lock().await;
    runtime.working_dir = working_dir.to_string();
    update(&mut runtime.status);
    if runtime.status.managed_path.is_empty() {
        runtime.status.managed_path = UNITY_REFERENCE_MANAGED_PATH.to_string();
    }
}

fn update_status_from_sync<F>(
    state: &Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    working_dir: &str,
    update: F,
) where
    F: FnOnce(&mut UnityReferenceImportStatus),
{
    // This path is called from synchronous progress callbacks that may already
    // be executing on a Tokio worker thread. Use a best-effort try_lock so we
    // never block the runtime thread and panic inside `blocking_lock()`.
    let Ok(mut runtime) = state.try_lock() else {
        return;
    };
    runtime.working_dir = working_dir.to_string();
    update(&mut runtime.status);
    if runtime.status.managed_path.is_empty() {
        runtime.status.managed_path = UNITY_REFERENCE_MANAGED_PATH.to_string();
    }
}

fn count_progress_ratio(processed: usize, total: usize) -> f32 {
    if total == 0 {
        return 1.0;
    }
    (processed as f32 / total as f32).clamp(0.0, 1.0)
}

fn reconcile_stage_progress(stage: &str, processed: usize, total: usize) -> f32 {
    let ratio = count_progress_ratio(processed, total);
    match stage {
        "preparing" => {
            if total == 0 {
                0.12
            } else {
                (0.12 + 0.18 * ratio).clamp(0.0, 1.0)
            }
        }
        "cleaning" => {
            if total == 0 {
                0.36
            } else {
                (0.30 + 0.14 * ratio).clamp(0.0, 1.0)
            }
        }
        "indexing" => {
            if total == 0 {
                0.52
            } else {
                (0.44 + 0.42 * ratio).clamp(0.0, 1.0)
            }
        }
        "committing" => {
            if total == 0 {
                0.90
            } else {
                (0.86 + 0.14 * ratio).clamp(0.0, 1.0)
            }
        }
        "completed" => 1.0,
        _ => ratio,
    }
}

fn reconcile_stage_message(
    stage: &str,
    processed: usize,
    total: usize,
    current_path: Option<&str>,
) -> String {
    match stage {
        "preparing" => {
            if total == 0 {
                "正在读取文档并分析索引变更。".to_string()
            } else if let Some(path) = current_path {
                format!(
                    "正在分析文档并准备索引（{} / {}）：{}",
                    processed, total, path
                )
            } else {
                format!("正在分析文档并准备索引（{} / {}）。", processed, total)
            }
        }
        "cleaning" => format!("正在清理旧索引记录（{} / {}）。", processed, total),
        "indexing" => match current_path {
            Some(path) => format!("正在更新索引（{} / {}）：{}", processed, total, path),
            None => format!("正在更新索引（{} / {}）。", processed, total),
        },
        "committing" => format!("正在提交索引更新（{} / {}）。", processed, total),
        "completed" => "知识索引更新完成。".to_string(),
        _ => "正在更新知识索引。".to_string(),
    }
}

fn emit_reconcile_progress_update(
    state: &Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    working_dir: &str,
    last_reconcile_stage: &mut String,
    stage: &str,
    processed: usize,
    total: usize,
    current_file: Option<&str>,
) {
    let should_emit = stage != last_reconcile_stage || processed == total || processed % 24 == 0;
    if !should_emit {
        return;
    }
    last_reconcile_stage.clear();
    last_reconcile_stage.push_str(stage);

    let progress = reconcile_stage_progress(stage, processed, total);
    let current_path = current_file
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("reference/{}", value));
    let message = reconcile_stage_message(stage, processed, total, current_path.as_deref());

    update_status_from_sync(state, working_dir, |status| {
        status.stage = UnityReferenceImportStage::Reconciling;
        status.progress = Some(progress);
        status.current_path = current_path.clone();
        status.message = message;
    });
}

fn ensure_import_not_cancelled(
    cancel_requested: &AtomicBool,
) -> Result<(), UnityReferenceImportRunError> {
    if cancel_requested.load(Ordering::Relaxed) {
        return Err(UnityReferenceImportRunError::Cancelled);
    }
    Ok(())
}

async fn clear_runtime_status(
    state: &Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    working_dir: &str,
) {
    let mut runtime = state.lock().await;
    if runtime.working_dir == working_dir {
        runtime.working_dir.clear();
        runtime.cancel_requested.store(false, Ordering::Relaxed);
        runtime.status = UnityReferenceImportStatus::default();
    }
}

async fn mark_runtime_cancelled(
    state: &Arc<tokio::sync::Mutex<UnityReferenceImportRuntime>>,
    working_dir: &str,
) {
    let mut runtime = state.lock().await;
    if runtime.working_dir == working_dir {
        runtime.cancel_requested.store(false, Ordering::Relaxed);
        runtime.status = UnityReferenceImportStatus {
            managed_path: UNITY_REFERENCE_MANAGED_PATH.to_string(),
            message: "已取消 Unity 文档导入。".to_string(),
            last_outcome: Some(UnityReferenceImportLastOutcome::Cancelled),
            ..UnityReferenceImportStatus::default()
        };
    }
}

fn cleanup_import_runtime_artifacts(working_dir: &str) -> Result<(), String> {
    cleanup_temp_managed_dir(working_dir)?;
    cleanup_temp_managed_store(working_dir)?;
    cleanup_cache_root(working_dir)?;
    Ok(())
}

async fn resolve_offline_source(
    client: &reqwest::Client,
    docs_version: &str,
    locale: UnityReferenceImportLocale,
) -> Result<UnityOfflineDocSource, String> {
    for page_url in offline_source_candidates(docs_version, locale) {
        let response = match client.get(&page_url).send().await {
            Ok(response) => response,
            Err(_) => continue,
        };
        if !response.status().is_success() {
            continue;
        }
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read Unity offline page '{}': {}", page_url, e))?;
        if let Some(zip_url) = capture_zip_url(&body) {
            return Ok(UnityOfflineDocSource {
                page_url,
                zip_url,
                locale: locale.as_str().to_string(),
            });
        }
    }

    Err(format!(
        "未找到 Unity {} 的{}官方离线文档下载地址。",
        docs_version,
        locale.display_name_zh()
    ))
}

fn offline_source_candidates(
    docs_version: &str,
    locale: UnityReferenceImportLocale,
) -> Vec<String> {
    match locale {
        UnityReferenceImportLocale::En => vec![
            format!(
                "https://docs.unity3d.com/{}/Documentation/Manual/OfflineDocumentation.html",
                docs_version
            ),
            format!(
                "https://docs.unity3d.com/{}/Manual/OfflineDocumentation.html",
                docs_version
            ),
        ],
        UnityReferenceImportLocale::ZhCn => vec![format!(
            "https://docs.unity3d.com/cn/{}/Manual/OfflineDocumentation.html",
            docs_version
        )],
    }
}

async fn download_unity_zip<F, Fut>(
    client: &reqwest::Client,
    zip_url: &str,
    target_path: &Path,
    cancel_requested: Arc<AtomicBool>,
    mut on_progress: F,
) -> Result<(), UnityReferenceImportRunError>
where
    F: FnMut(u64, Option<u64>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    ensure_import_not_cancelled(&cancel_requested)?;
    let response = client.get(zip_url).send().await.map_err(|e| {
        UnityReferenceImportRunError::Failed(format!(
            "Failed to download Unity documentation zip: {}",
            e
        ))
    })?;
    if !response.status().is_success() {
        return Err(UnityReferenceImportRunError::Failed(format!(
            "下载 Unity 离线文档失败，HTTP {}。",
            response.status()
        )));
    }

    let total = response.content_length();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();
    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to create Unity download parent directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }
    let mut file = tokio::fs::File::create(target_path).await.map_err(|e| {
        UnityReferenceImportRunError::Failed(format!(
            "Failed to create download file '{}': {}",
            target_path.display(),
            e
        ))
    })?;
    while let Some(chunk) = stream.next().await {
        ensure_import_not_cancelled(&cancel_requested)?;
        let chunk = chunk.map_err(|e| {
            UnityReferenceImportRunError::Failed(format!("Failed to read download stream: {}", e))
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to write download file '{}': {}",
                target_path.display(),
                e
            ))
        })?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total).await;
        ensure_import_not_cancelled(&cancel_requested)?;
    }
    file.flush().await.map_err(|e| {
        UnityReferenceImportRunError::Failed(format!(
            "Failed to flush download file '{}': {}",
            target_path.display(),
            e
        ))
    })?;
    Ok(())
}

fn extract_unity_zip<F>(
    zip_path: &Path,
    output_dir: &Path,
    cancel_requested: &AtomicBool,
    mut on_progress: F,
) -> Result<Vec<UnityExtractedZipEntry>, UnityReferenceImportRunError>
where
    F: FnMut(usize, usize, &str),
{
    ensure_import_not_cancelled(cancel_requested)?;
    if output_dir.exists() {
        std::fs::remove_dir_all(output_dir).map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to clean Unity extract directory '{}': {}",
                output_dir.display(),
                e
            ))
        })?;
    }
    std::fs::create_dir_all(output_dir).map_err(|e| {
        UnityReferenceImportRunError::Failed(format!(
            "Failed to create Unity extract directory '{}': {}",
            output_dir.display(),
            e
        ))
    })?;

    let file = std::fs::File::open(zip_path).map_err(|e| {
        UnityReferenceImportRunError::Failed(format!(
            "Failed to open Unity zip '{}': {}",
            zip_path.display(),
            e
        ))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        UnityReferenceImportRunError::Failed(format!("Failed to read Unity zip archive: {}", e))
    })?;
    let total_files = count_extractable_zip_entries(&mut archive, cancel_requested)?;
    let mut extracted_entries = Vec::new();
    let mut used_relative_paths = std::collections::HashSet::new();
    let mut processed_files = 0usize;
    for index in 0..archive.len() {
        ensure_import_not_cancelled(cancel_requested)?;
        let mut entry = archive.by_index(index).map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to read zip entry #{}: {}",
                index, e
            ))
        })?;
        let Some(relative_path) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }

        let original_relative_path = relative_path.to_string_lossy().replace('\\', "/");
        let safe_relative_path =
            sanitize_zip_relative_path(&relative_path, &mut used_relative_paths);
        let target_path = output_dir.join(&safe_relative_path);
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                UnityReferenceImportRunError::Failed(format!(
                    "Failed to create extracted file parent '{}': {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        let mut output = std::fs::File::create(&target_path).map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to create extracted file '{}': {}",
                target_path.display(),
                e
            ))
        })?;
        std::io::copy(&mut entry, &mut output).map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to extract zip entry '{}' to '{}': {}",
                entry.name(),
                target_path.display(),
                e
            ))
        })?;
        extracted_entries.push(UnityExtractedZipEntry {
            original_relative_path,
            file_path: target_path,
        });
        processed_files += 1;
        on_progress(
            processed_files,
            total_files,
            extracted_entries
                .last()
                .map(|item| item.original_relative_path.as_str())
                .unwrap_or_default(),
        );
    }
    Ok(extracted_entries)
}

fn count_extractable_zip_entries(
    archive: &mut zip::ZipArchive<std::fs::File>,
    cancel_requested: &AtomicBool,
) -> Result<usize, UnityReferenceImportRunError> {
    let mut total = 0usize;
    for index in 0..archive.len() {
        ensure_import_not_cancelled(cancel_requested)?;
        let entry = archive.by_index(index).map_err(|e| {
            UnityReferenceImportRunError::Failed(format!(
                "Failed to inspect zip entry #{}: {}",
                index, e
            ))
        })?;
        if entry.is_dir() {
            continue;
        }
        if entry.enclosed_name().is_none() {
            continue;
        }
        total += 1;
    }
    Ok(total)
}

fn collect_html_doc_candidates(
    extracted_entries: &[UnityExtractedZipEntry],
    script_reference_context: &ScriptReferencePathContext,
) -> Result<Vec<UnityHtmlDocCandidate>, String> {
    let mut candidates = Vec::new();
    let mut used_relative_markdown_paths = std::collections::HashSet::new();
    for entry in extracted_entries {
        let Some(raw_relative_markdown_path) =
            classify_html_doc_path(&entry.original_relative_path, script_reference_context)
        else {
            continue;
        };
        let relative_markdown_path = sanitize_relative_output_path(
            &raw_relative_markdown_path,
            &mut used_relative_markdown_paths,
        );
        candidates.push(UnityHtmlDocCandidate {
            raw_relative_markdown_path,
            relative_markdown_path,
            html_relative_path: entry.original_relative_path.clone(),
            file_path: entry.file_path.clone(),
        });
    }

    candidates.sort_by(|left, right| {
        left.relative_markdown_path
            .cmp(&right.relative_markdown_path)
    });
    Ok(candidates)
}

fn build_relative_markdown_lookup(candidates: &[UnityHtmlDocCandidate]) -> HashMap<String, String> {
    candidates
        .iter()
        .map(|candidate| {
            (
                candidate.raw_relative_markdown_path.clone(),
                candidate.relative_markdown_path.clone(),
            )
        })
        .collect()
}

fn apply_manual_breadcrumb_hierarchy(
    candidates: &[UnityHtmlDocCandidate],
    zip_path: Option<&Path>,
) -> Result<Vec<UnityHtmlDocCandidate>, String> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let breadcrumb_label_map = build_manual_breadcrumb_label_map(candidates, zip_path)?;
    if breadcrumb_label_map.is_empty() {
        return Ok(candidates.to_vec());
    }

    let mut ancestor_label_paths = std::collections::HashSet::new();
    for labels in breadcrumb_label_map.values() {
        for prefix_len in 1..labels.len() {
            ancestor_label_paths.insert(labels[..prefix_len].to_vec());
        }
    }

    let mut used_relative_paths = std::collections::HashSet::new();
    let mut updated = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let desired_relative_path = breadcrumb_label_map
            .get(&candidate.raw_relative_markdown_path)
            .and_then(|labels| {
                derive_manual_breadcrumb_output_path(candidate, labels, &ancestor_label_paths)
            })
            .unwrap_or_else(|| candidate.relative_markdown_path.clone());
        let relative_markdown_path =
            sanitize_relative_output_path(&desired_relative_path, &mut used_relative_paths);
        let mut updated_candidate = candidate.clone();
        updated_candidate.relative_markdown_path = relative_markdown_path;
        updated.push(updated_candidate);
    }

    updated.sort_by(|left, right| {
        left.relative_markdown_path
            .cmp(&right.relative_markdown_path)
    });
    Ok(updated)
}

fn build_manual_breadcrumb_label_map(
    candidates: &[UnityHtmlDocCandidate],
    zip_path: Option<&Path>,
) -> Result<HashMap<String, Vec<String>>, String> {
    let mut label_map = HashMap::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.raw_relative_markdown_path.starts_with("manual/"))
    {
        let raw_bytes = read_unity_html_bytes(candidate, zip_path)?;
        let raw_html = String::from_utf8_lossy(&raw_bytes);
        let breadcrumb_labels = extract_manual_breadcrumb_labels(&raw_html);
        if !breadcrumb_labels.is_empty() {
            label_map.insert(
                candidate.raw_relative_markdown_path.clone(),
                breadcrumb_labels,
            );
        }
    }
    Ok(label_map)
}

fn derive_manual_breadcrumb_output_path(
    candidate: &UnityHtmlDocCandidate,
    breadcrumb_labels: &[String],
    ancestor_label_paths: &std::collections::HashSet<Vec<String>>,
) -> Option<String> {
    if breadcrumb_labels.is_empty() {
        return None;
    }

    let file_name = Path::new(&candidate.raw_relative_markdown_path)
        .file_name()
        .and_then(|value| value.to_str())?;
    let folder_depth = if ancestor_label_paths.contains(breadcrumb_labels) {
        breadcrumb_labels.len()
    } else {
        breadcrumb_labels.len().saturating_sub(1)
    };

    let mut output = PathBuf::from("manual");
    for label in breadcrumb_labels.iter().take(folder_depth) {
        output.push(sanitize_windows_path_segment(label));
    }
    output.push(file_name);
    Some(output.to_string_lossy().replace('\\', "/"))
}

fn extract_manual_breadcrumb_labels(raw_html: &str) -> Vec<String> {
    let Some(list_html) = capture_group(raw_html, manual_breadcrumbs_regex(), 1) else {
        return Vec::new();
    };

    manual_breadcrumb_item_regex()
        .captures_iter(&list_html)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str()))
        .map(render_html_fragment_text)
        .filter(|value| !value.is_empty())
        .collect()
}

fn render_html_fragment_text(fragment: &str) -> String {
    let markdown = html2md::parse_html(fragment);
    let collapsed = collapse_whitespace(markdown.trim());
    let label = unwrap_single_markdown_link_label(collapsed.as_str())
        .unwrap_or(collapsed.as_str())
        .trim();
    label.to_string()
}

fn sanitize_relative_output_path(
    relative_path: &str,
    used_relative_paths: &mut std::collections::HashSet<PathBuf>,
) -> String {
    sanitize_zip_relative_path(Path::new(relative_path), used_relative_paths)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sanitize_zip_relative_path(
    relative_path: &Path,
    used_relative_paths: &mut std::collections::HashSet<PathBuf>,
) -> PathBuf {
    let mut sanitized = PathBuf::new();
    for segment in relative_path.iter() {
        sanitized.push(sanitize_windows_path_segment(
            segment.to_string_lossy().as_ref(),
        ));
    }

    if used_relative_paths.insert(sanitized.clone()) {
        return sanitized;
    }

    let digest = blake3::hash(relative_path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    let suffix = &digest[..8];
    let mut disambiguated = sanitized.clone();
    let replacement = disambiguate_file_name(
        disambiguated
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("entry"),
        suffix,
    );
    disambiguated.set_file_name(replacement);
    used_relative_paths.insert(disambiguated.clone());
    disambiguated
}

fn sanitize_windows_path_segment(segment: &str) -> String {
    let mut sanitized: String = segment
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();

    while sanitized.ends_with([' ', '.']) {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push('_');
    }

    let reserved_key = sanitized
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    if matches!(
        reserved_key.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        sanitized.push('_');
    }

    sanitized
}

fn disambiguate_file_name(file_name: &str, suffix: &str) -> String {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("entry");
    let extension = path.extension().and_then(|value| value.to_str());
    match extension {
        Some(ext) if !ext.is_empty() => format!("{}__{}.{}", stem, suffix, ext),
        _ => format!("{}__{}", stem, suffix),
    }
}

#[derive(Clone, Default)]
struct ScriptReferencePathContext {
    known_stems: std::collections::HashSet<String>,
    container_keys: std::collections::HashSet<String>,
}

fn build_script_reference_path_context(
    extracted_entries: &[UnityExtractedZipEntry],
) -> ScriptReferencePathContext {
    let known_stems: std::collections::HashSet<String> = extracted_entries
        .iter()
        .filter_map(|entry| script_reference_stem_from_relative_path(&entry.original_relative_path))
        .collect();
    let container_keys = known_stems
        .iter()
        .filter_map(|stem| script_reference_group_key(stem, &known_stems))
        .collect();
    ScriptReferencePathContext {
        known_stems,
        container_keys,
    }
}

fn classify_html_doc_path(
    relative_path: &str,
    script_reference_context: &ScriptReferencePathContext,
) -> Option<String> {
    let normalized = relative_path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let extension = Path::new(&normalized)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("html" | "htm")) {
        return None;
    }
    let file_name = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())?
        .to_ascii_lowercase();
    if matches!(
        file_name.as_str(),
        "offlinedocumentation.html" | "termsofuse.html" | "30_search.html"
    ) {
        return None;
    }

    if let Some(tail) = relative_tail_after(&normalized, &lower, "manual/") {
        return Some(format!("manual/{}", replace_html_extension(tail)));
    }
    if let Some(tail) = relative_tail_after(&normalized, &lower, "scriptreference/") {
        return Some(classify_script_reference_output_path(
            tail,
            script_reference_context,
        ));
    }
    None
}

fn classify_script_reference_output_path(
    tail: &str,
    script_reference_context: &ScriptReferencePathContext,
) -> String {
    let relative_tail = Path::new(tail);
    let file_name = relative_tail
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(tail);
    let stem = relative_tail
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut output = PathBuf::from("script-reference");
    if let Some(parent) = relative_tail
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        output.push(parent);
    }

    let folder_key = script_reference_group_key(stem, &script_reference_context.known_stems)
        .or_else(|| {
            script_reference_context
                .container_keys
                .contains(stem)
                .then(|| stem.to_string())
        });
    if let Some(folder_key) = folder_key {
        for segment in folder_key.split('.') {
            if !segment.is_empty() {
                output.push(segment);
            }
        }
    }

    output.push(replace_html_extension(file_name));
    output.to_string_lossy().replace('\\', "/")
}

fn script_reference_stem_from_relative_path(relative_path: &str) -> Option<String> {
    let normalized = relative_path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let tail = relative_tail_after(&normalized, &lower, "scriptreference/")?;
    Path::new(tail)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
}

fn script_reference_group_key(
    stem: &str,
    known_stems: &std::collections::HashSet<String>,
) -> Option<String> {
    if let Some((owner, _member)) = stem.split_once('-') {
        if !owner.is_empty() {
            return Some(owner.to_string());
        }
    }
    if let Some((owner, _member)) = stem.rsplit_once('.') {
        if !owner.is_empty() && known_stems.contains(owner) {
            return Some(owner.to_string());
        }
    }
    None
}

fn relative_tail_after<'a>(original: &'a str, lowered: &str, marker: &str) -> Option<&'a str> {
    let start = lowered.find(marker)?;
    let tail = &original[start + marker.len()..];
    if tail.is_empty() {
        return None;
    }
    Some(tail)
}

fn replace_html_extension(path: &str) -> String {
    if let Some(stripped) = path.strip_suffix(".html") {
        return format!("{}.md", stripped);
    }
    if let Some(stripped) = path.strip_suffix(".htm") {
        return format!("{}.md", stripped);
    }
    format!("{}.md", path)
}

fn read_unity_html_bytes_from_open_zip(
    archive: &mut zip::ZipArchive<std::fs::File>,
    zip_path: &Path,
    html_relative_path: &str,
) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_name(html_relative_path).map_err(|error| {
        format!(
            "Failed to locate '{}' in Unity documentation zip '{}': {}",
            html_relative_path,
            zip_path.display(),
            error
        )
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|error| {
        format!(
            "Failed to read '{}' from Unity documentation zip '{}': {}",
            html_relative_path,
            zip_path.display(),
            error
        )
    })?;
    Ok(bytes)
}

fn read_unity_html_bytes_from_zip(
    zip_path: &Path,
    html_relative_path: &str,
) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(zip_path).map_err(|error| {
        format!(
            "Failed to open Unity documentation zip '{}': {}",
            zip_path.display(),
            error
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        format!(
            "Failed to read Unity documentation zip '{}': {}",
            zip_path.display(),
            error
        )
    })?;
    read_unity_html_bytes_from_open_zip(&mut archive, zip_path, html_relative_path)
}

fn read_unity_html_bytes(
    candidate: &UnityHtmlDocCandidate,
    zip_path: Option<&Path>,
) -> Result<Vec<u8>, String> {
    match std::fs::read(&candidate.file_path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(zip_path) = zip_path {
                return read_unity_html_bytes_from_zip(zip_path, &candidate.html_relative_path)
                    .map_err(|zip_error| {
                        format!(
                            "Failed to read Unity HTML file '{}': {}. Zip fallback failed: {}",
                            candidate.file_path.display(),
                            error,
                            zip_error
                        )
                    });
            }
            Err(format!(
                "Failed to read Unity HTML file '{}': {}",
                candidate.file_path.display(),
                error
            ))
        }
        Err(error) => Err(format!(
            "Failed to read Unity HTML file '{}': {}",
            candidate.file_path.display(),
            error
        )),
    }
}

fn build_reference_document_from_raw_html(
    candidate: &UnityHtmlDocCandidate,
    raw_html: &str,
    source: &UnityOfflineDocSource,
    source_id: &str,
    docs_version: &str,
    target_path: &str,
    script_reference_context: &ScriptReferencePathContext,
    relative_markdown_lookup: &HashMap<String, String>,
) -> Result<KnowledgeDocument, String> {
    let source_url = extract_canonical_url(raw_html)
        .or_else(|| build_fallback_source_url(source, &candidate.html_relative_path))
        .unwrap_or_else(|| source.page_url.clone());
    let main_html = extract_main_html(raw_html)?;
    let linked_html = rewrite_relative_urls(
        &main_html,
        &source_url,
        target_path,
        script_reference_context,
        relative_markdown_lookup,
    );
    let markdown = clean_markdown(&html2md::parse_html(&linked_html));
    let title = extract_title(raw_html, &markdown, &candidate.relative_markdown_path);
    let summary = extract_summary(raw_html, &main_html, &markdown);
    let final_path = format!("{}/{}", target_path, candidate.relative_markdown_path);

    Ok(KnowledgeDocument {
        id: stable_document_id(docs_version, &final_path),
        doc_type: KnowledgeType::Reference,
        path: final_path,
        title,
        inject_mode: KnowledgeInjectMode::None,
        inherit_inject_mode: false,
        inject_mode_source: knowledge_store::KnowledgeConfigSource {
            kind: knowledge_store::KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        summary_enabled: false,
        command_enabled: false,
        read_only: true,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::Project,
        inherit_ai_config: false,
        ai_config_source: knowledge_store::KnowledgeConfigSource {
            kind: knowledge_store::KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        explicit_maintenance_rules: false,
        external_source: Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::Unity,
            locator: None,
            source_id: Some(source_id.to_string()),
            sync_enabled: true,
        }),
        skill_enabled: None,
        skill_surface: None,
        command_trigger: None,
        argument_hint: None,
        tools: Vec::new(),
        summary: if summary.is_empty() {
            None
        } else {
            Some(summary)
        },
        body: markdown,
        maintenance_rules: None,
        created_at: 0,
        updated_at: 0,
    })
}

fn build_reference_document(
    candidate: &UnityHtmlDocCandidate,
    source: &UnityOfflineDocSource,
    source_id: &str,
    docs_version: &str,
    target_path: &str,
    zip_path: Option<&Path>,
    script_reference_context: &ScriptReferencePathContext,
    relative_markdown_lookup: &HashMap<String, String>,
) -> Result<KnowledgeDocument, String> {
    let raw_bytes = read_unity_html_bytes(candidate, zip_path)?;
    let raw_html = String::from_utf8_lossy(&raw_bytes).into_owned();
    build_reference_document_from_raw_html(
        candidate,
        &raw_html,
        source,
        source_id,
        docs_version,
        target_path,
        script_reference_context,
        relative_markdown_lookup,
    )
}

fn bounded_reference_convert_parallelism(
    document_count: usize,
    available_parallelism: usize,
) -> usize {
    if document_count < UNITY_REFERENCE_CONVERT_PARALLEL_THRESHOLD {
        return 1;
    }
    available_parallelism
        .max(1)
        .min(UNITY_REFERENCE_CONVERT_MAX_WORKERS)
}

fn unity_reference_convert_parallelism(document_count: usize) -> usize {
    bounded_reference_convert_parallelism(
        document_count,
        std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1),
    )
}

fn unity_reference_convert_batch_size(parallelism: usize) -> usize {
    std::cmp::max(
        UNITY_REFERENCE_CONVERT_MIN_BATCH_SIZE,
        parallelism.max(1) * UNITY_REFERENCE_CONVERT_BATCHES_PER_WORKER,
    )
}

async fn build_reference_document_batch(
    candidates: &[UnityHtmlDocCandidate],
    source: Arc<UnityOfflineDocSource>,
    source_id: Arc<String>,
    docs_version: Arc<String>,
    target_path: Arc<String>,
    zip_path: Arc<Option<PathBuf>>,
    script_reference_context: Arc<ScriptReferencePathContext>,
    relative_markdown_lookup: Arc<HashMap<String, String>>,
    cancel_requested: Arc<AtomicBool>,
    parallelism: usize,
) -> Result<Vec<KnowledgeDocument>, UnityReferenceImportRunError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let should_prefer_zip_batch_reads = zip_path.is_some()
        && candidates
            .iter()
            .any(|candidate| !candidate.file_path.is_file());
    if should_prefer_zip_batch_reads {
        let worker_count = parallelism.max(1).min(candidates.len());
        let chunk_size = std::cmp::max(1, candidates.len().div_ceil(worker_count));
        let candidate_chunks = candidates
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>();
        let mut stream = futures::stream::iter(candidate_chunks.into_iter().map(|chunk| {
            let source = source.clone();
            let source_id = source_id.clone();
            let docs_version = docs_version.clone();
            let target_path = target_path.clone();
            let zip_path = zip_path.clone();
            let script_reference_context = script_reference_context.clone();
            let relative_markdown_lookup = relative_markdown_lookup.clone();
            let cancel_requested = cancel_requested.clone();
            async move {
                if cancel_requested.load(Ordering::Relaxed) {
                    return Err(UnityReferenceImportRunError::Cancelled);
                }

                tokio::task::spawn_blocking(move || {
                    if cancel_requested.load(Ordering::Relaxed) {
                        return Err(UnityReferenceImportRunError::Cancelled);
                    }

                    let zip_path = zip_path.as_ref().as_deref().ok_or_else(|| {
                        UnityReferenceImportRunError::Failed(
                            "Unity documentation zip path missing for batch fallback".to_string(),
                        )
                    })?;
                    let zip_file = std::fs::File::open(zip_path).map_err(|error| {
                        UnityReferenceImportRunError::Failed(format!(
                            "Failed to open Unity documentation zip '{}': {}",
                            zip_path.display(),
                            error
                        ))
                    })?;
                    let mut archive = zip::ZipArchive::new(zip_file).map_err(|error| {
                        UnityReferenceImportRunError::Failed(format!(
                            "Failed to read Unity documentation zip '{}': {}",
                            zip_path.display(),
                            error
                        ))
                    })?;

                    let mut documents = Vec::with_capacity(chunk.len());
                    for candidate in chunk {
                        if cancel_requested.load(Ordering::Relaxed) {
                            return Err(UnityReferenceImportRunError::Cancelled);
                        }

                        let raw_bytes = match std::fs::read(&candidate.file_path) {
                            Ok(bytes) => bytes,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                read_unity_html_bytes_from_open_zip(
                                    &mut archive,
                                    zip_path,
                                    &candidate.html_relative_path,
                                )
                                .map_err(UnityReferenceImportRunError::Failed)?
                            }
                            Err(error) => {
                                return Err(UnityReferenceImportRunError::Failed(format!(
                                    "Failed to read Unity HTML file '{}': {}",
                                    candidate.file_path.display(),
                                    error
                                )))
                            }
                        };
                        let raw_html = String::from_utf8_lossy(&raw_bytes).into_owned();
                        let document = build_reference_document_from_raw_html(
                            &candidate,
                            &raw_html,
                            source.as_ref(),
                            source_id.as_ref().as_str(),
                            docs_version.as_ref().as_str(),
                            target_path.as_ref().as_str(),
                            script_reference_context.as_ref(),
                            relative_markdown_lookup.as_ref(),
                        )
                        .map_err(UnityReferenceImportRunError::Failed)?;
                        documents.push(document);
                    }

                    Ok(documents)
                })
                .await
                .map_err(|error| {
                    UnityReferenceImportRunError::Failed(format!(
                        "Unity reference conversion worker failed to join: {}",
                        error
                    ))
                })?
            }
        }))
        .buffered(worker_count);

        let mut documents = Vec::with_capacity(candidates.len());
        while let Some(result) = stream.next().await {
            documents.extend(result?);
        }
        return Ok(documents);
    }

    let mut stream = futures::stream::iter(candidates.iter().cloned().map(|candidate| {
        let source = source.clone();
        let source_id = source_id.clone();
        let docs_version = docs_version.clone();
        let target_path = target_path.clone();
        let zip_path = zip_path.clone();
        let script_reference_context = script_reference_context.clone();
        let relative_markdown_lookup = relative_markdown_lookup.clone();
        let cancel_requested = cancel_requested.clone();
        async move {
            if cancel_requested.load(Ordering::Relaxed) {
                return Err(UnityReferenceImportRunError::Cancelled);
            }

            tokio::task::spawn_blocking(move || {
                if cancel_requested.load(Ordering::Relaxed) {
                    return Err(UnityReferenceImportRunError::Cancelled);
                }

                build_reference_document(
                    &candidate,
                    source.as_ref(),
                    source_id.as_ref().as_str(),
                    docs_version.as_ref().as_str(),
                    target_path.as_ref().as_str(),
                    zip_path.as_deref(),
                    script_reference_context.as_ref(),
                    relative_markdown_lookup.as_ref(),
                )
                .map_err(UnityReferenceImportRunError::Failed)
            })
            .await
            .map_err(|error| {
                UnityReferenceImportRunError::Failed(format!(
                    "Unity reference conversion worker failed to join: {}",
                    error
                ))
            })?
        }
    }))
    .buffered(parallelism.max(1));

    let mut documents = Vec::with_capacity(candidates.len());
    while let Some(result) = stream.next().await {
        documents.push(result?);
    }
    Ok(documents)
}

fn stable_document_id(docs_version: &str, path: &str) -> String {
    let digest = blake3::hash(format!("{}::{}", docs_version, path).as_bytes())
        .to_hex()
        .to_string();
    format!("kd_unity_{}", &digest[..24])
}

fn extract_title(raw_html: &str, markdown: &str, fallback_path: &str) -> String {
    if let Some(value) = capture_group(raw_html, title_regex(), 1) {
        let cleaned = value
            .replace(" - Unity 手册", "")
            .replace(" - Unity Manual", "")
            .replace(" - Unity 脚本 API", "")
            .replace(" - Unity 脚本API", "")
            .replace(" - Unity 脚本参考", "")
            .replace(" - Unity Scripting API", "")
            .replace(" - Unity Script Reference", "")
            .trim()
            .to_string();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }

    if let Some(line) = markdown
        .lines()
        .find(|line| line.trim_start().starts_with("# "))
    {
        let cleaned = line.trim_start().trim_start_matches("# ").trim();
        if !cleaned.is_empty() {
            return cleaned.to_string();
        }
    }

    Path::new(fallback_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unity Document")
        .to_string()
}

fn extract_summary(raw_html: &str, main_html: &str, markdown: &str) -> String {
    if let Some(value) = capture_group(raw_html, meta_description_regex(), 1) {
        let cleaned = collapse_whitespace(&value);
        if !cleaned.is_empty() {
            return truncate_text(&cleaned, 220);
        }
    }

    if let Some(value) = capture_group(main_html, first_paragraph_regex(), 1) {
        let plain = clean_markdown(&html2md::parse_html(&value));
        let cleaned = collapse_whitespace(&plain);
        if !cleaned.is_empty() {
            return truncate_text(&cleaned, 220);
        }
    }

    let first_paragraph = markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('>'));
    first_paragraph
        .map(|line| truncate_text(&collapse_whitespace(line), 220))
        .unwrap_or_default()
}

fn extract_main_html(raw_html: &str) -> Result<String, String> {
    let Some(content_index) = raw_html.find("<div id=\"content-wrap\"") else {
        return Err("Unity HTML page missing content-wrap container".to_string());
    };
    let wrapped = &raw_html[content_index..];
    let end_index = wrapped
        .find("<div id=\"_content\"")
        .or_else(|| wrapped.find("<div class=\"footer-wrapper\""))
        .unwrap_or(wrapped.len());
    let clipped = &wrapped[..end_index];
    let start_index = clipped.find("<h1").unwrap_or(0);
    let mut main = clipped[start_index..].to_string();

    if let Some(feedback_index) = main.find("<div class=\"scrollToFeedback\">") {
        if let Some(subsection_index) = main[feedback_index..].find("<div class=\"subsection\">") {
            let restart = feedback_index + subsection_index;
            main = format!("{}{}", &main[..feedback_index], &main[restart..]);
        }
    }

    Ok(main)
}

fn rewrite_relative_urls(
    html: &str,
    source_url: &str,
    target_path: &str,
    script_reference_context: &ScriptReferencePathContext,
    relative_markdown_lookup: &HashMap<String, String>,
) -> String {
    let Some(base_url) = Url::parse(source_url).ok() else {
        return html.to_string();
    };

    link_attr_regex()
        .replace_all(html, |caps: &regex::Captures<'_>| {
            let attribute = caps.get(1).map(|m| m.as_str()).unwrap_or("href");
            let target = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if target.is_empty()
                || target.starts_with('#')
                || target.starts_with("mailto:")
                || target.starts_with("javascript:")
                || target.starts_with("data:")
            {
                return caps
                    .get(0)
                    .map(|value| value.as_str().to_string())
                    .unwrap_or_default();
            }

            let resolved_url = if target.starts_with("http://") || target.starts_with("https://") {
                Url::parse(target).ok()
            } else {
                base_url.join(target).ok()
            };

            if attribute.eq_ignore_ascii_case("href") {
                if let Some(local_path) = resolved_url.as_ref().and_then(|url| {
                    resolve_unity_doc_url_to_managed_path(
                        url,
                        target_path,
                        script_reference_context,
                        relative_markdown_lookup,
                    )
                }) {
                    return format!(r#"{}="{}""#, attribute, local_path);
                }
            }

            match resolved_url {
                Some(url) => format!(r#"{}="{}""#, attribute, url),
                None => caps
                    .get(0)
                    .map(|value| value.as_str().to_string())
                    .unwrap_or_default(),
            }
        })
        .into_owned()
}

fn resolve_unity_doc_url_to_managed_path(
    url: &Url,
    target_path: &str,
    script_reference_context: &ScriptReferencePathContext,
    relative_markdown_lookup: &HashMap<String, String>,
) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "docs.unity3d.com" {
        return None;
    }

    let raw_relative_path =
        classify_html_doc_path(url.path().trim_start_matches('/'), script_reference_context)?;
    let relative_path = relative_markdown_lookup
        .get(&raw_relative_path)
        .cloned()
        .unwrap_or(raw_relative_path);
    let mut managed_path = format!("reference/{}/{}", target_path, relative_path);
    if let Some(fragment) = url.fragment().filter(|value| !value.is_empty()) {
        managed_path.push('#');
        managed_path.push_str(fragment);
    }
    Some(managed_path)
}

fn clean_markdown(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    let without_html_anchors = html_anchor_regex().replace_all(&normalized, "");
    let mut cleaned_lines = Vec::new();
    let mut in_code_fence = false;
    for line in without_html_anchors.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            cleaned_lines.push(line.trim_end().to_string());
            continue;
        }
        if !in_code_fence && is_unity_markdown_noise_line(trimmed) {
            continue;
        }
        cleaned_lines.push(line.trim_end().to_string());
    }
    multiline_break_regex()
        .replace_all(cleaned_lines.join("\n").trim(), "\n\n")
        .to_string()
}

fn is_unity_markdown_noise_line(line: &str) -> bool {
    let candidate = line.trim_start_matches(['-', '*', '+', ' ']).trim();
    if candidate.is_empty() {
        return false;
    }
    let label = unwrap_single_markdown_link_label(candidate).unwrap_or(candidate);
    matches!(
        label.trim(),
        "Leave feedback"
            | "Suggest a change"
            | "Switch to Manual"
            | "Switch to Scripting API"
            | "切换到手册"
            | "切换到脚本 API"
            | "切换到脚本API"
    )
}

fn unwrap_single_markdown_link_label(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(')') {
        return None;
    }
    let separator = trimmed.find("](")?;
    Some(&trimmed[1..separator])
}

fn build_fallback_source_url(
    source: &UnityOfflineDocSource,
    relative_html_path: &str,
) -> Option<String> {
    let mut base = Url::parse(&source.page_url).ok()?;
    let lower = relative_html_path.to_ascii_lowercase();
    if lower.contains("scriptreference/") {
        base = base.join("../ScriptReference/").ok()?;
        let tail = relative_tail_after(relative_html_path, &lower, "scriptreference/")?;
        return base.join(tail).ok().map(|url| url.to_string());
    }
    if lower.contains("manual/") {
        base = base.join("./").ok()?;
        let tail = relative_tail_after(relative_html_path, &lower, "manual/")?;
        return base.join(tail).ok().map(|url| url.to_string());
    }
    None
}

fn capture_zip_url(body: &str) -> Option<String> {
    capture_group(body, zip_url_regex(), 0)
}

fn extract_canonical_url(body: &str) -> Option<String> {
    capture_group(body, canonical_url_regex(), 1)
}

fn capture_group(input: &str, regex: &Regex, group: usize) -> Option<String> {
    regex
        .captures(input)
        .and_then(|captures| captures.get(group).map(|value| value.as_str().to_string()))
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn collapse_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn prepare_cache_root(cache_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(cache_root).map_err(|e| {
        format!(
            "Failed to create Unity reference cache directory '{}': {}",
            cache_root.display(),
            e
        )
    })
}

fn finalize_managed_store(working_dir: &str) -> Result<(), String> {
    let temp_store = temp_managed_store_path(working_dir);
    if temp_store.is_file() {
        let mut conn = open_unity_reference_store(&temp_store)?;
        rebuild_managed_directory_index(&temp_store, &mut conn)?;
    }
    let final_store = managed_store_path(working_dir);
    if final_store.exists() {
        std::fs::remove_file(&final_store).map_err(|e| {
            format!(
                "Failed to replace old Unity reference store '{}': {}",
                final_store.display(),
                e
            )
        })?;
    }
    std::fs::rename(&temp_store, &final_store).map_err(|e| {
        format!(
            "Failed to publish Unity reference store '{}' -> '{}': {}",
            temp_store.display(),
            final_store.display(),
            e
        )
    })
}

fn cleanup_temp_managed_dir(working_dir: &str) -> Result<(), String> {
    let temp_dir = temp_managed_dir_path(working_dir);
    if !temp_dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&temp_dir).map_err(|e| {
        format!(
            "Failed to clean temporary Unity reference directory '{}': {}",
            temp_dir.display(),
            e
        )
    })
}

fn cleanup_temp_managed_store(working_dir: &str) -> Result<(), String> {
    let temp_store = temp_managed_store_path(working_dir);
    if !temp_store.exists() {
        return Ok(());
    }
    std::fs::remove_file(&temp_store).map_err(|e| {
        format!(
            "Failed to clean temporary Unity reference store '{}': {}",
            temp_store.display(),
            e
        )
    })
}

fn cleanup_cache_root(working_dir: &str) -> Result<(), String> {
    let cache_dir = cache_root(working_dir);
    if !cache_dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&cache_dir).map_err(|e| {
        format!(
            "Failed to clean Unity reference cache directory '{}': {}",
            cache_dir.display(),
            e
        )
    })
}

fn delete_unity_reference_import_artifacts(working_dir: &str) -> Result<bool, String> {
    let mut removed_any = false;
    removed_any |= remove_directory_if_exists(&managed_dir_path(working_dir), "managed directory")?;
    removed_any |=
        remove_directory_if_exists(&temp_managed_dir_path(working_dir), "temporary directory")?;
    removed_any |= remove_directory_if_exists(&cache_root(working_dir), "cache directory")?;
    removed_any |= remove_file_if_exists(&managed_store_path(working_dir), "document store")?;
    removed_any |= remove_file_if_exists(&temp_managed_store_path(working_dir), "temporary store")?;
    removed_any |= remove_file_if_exists(&manifest_path(working_dir), "manifest")?;
    removed_any |= remove_file_if_exists(&legacy_manifest_path(working_dir), "legacy manifest")?;
    removed_any |= remove_file_if_exists(
        &managed_directory_config_path(working_dir, UNITY_REFERENCE_DIRECTORY_CONFIG_SUFFIX),
        "directory config",
    )?;
    removed_any |= remove_file_if_exists(
        &managed_directory_config_path(working_dir, UNITY_REFERENCE_LEGACY_DIRECTORY_CONFIG_SUFFIX),
        "legacy directory config",
    )?;
    Ok(removed_any)
}

fn remove_directory_if_exists(path: &Path, label: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(path).map_err(|e| {
        format!(
            "Failed to delete Unity reference {} '{}': {}",
            label,
            path.display(),
            e
        )
    })?;
    Ok(true)
}

fn remove_file_if_exists(path: &Path, label: &str) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path).map_err(|e| {
        format!(
            "Failed to delete Unity reference {} '{}': {}",
            label,
            path.display(),
            e
        )
    })?;
    Ok(true)
}

fn configure_managed_directory(
    working_dir: &str,
    target_path: &str,
    project_version: &str,
    docs_version: &str,
    locale: &str,
    imported_at: i64,
) -> Result<(), String> {
    let mut config: KnowledgeDirectoryConfig =
        default_directory_config_for_type(KnowledgeType::Reference);
    config.summary = format!(
        "Unity {} 官方文档导入结果，来源于当前项目 Unity {} 对应的官方离线文档。",
        docs_version, project_version
    );
    config.inject_mode = KnowledgeInjectMode::None;
    config.inherit_inject_mode = false;
    config.ai_maintained = false;
    config.inherit_ai_config = false;
    config.explicit_maintenance_rules = true;
    config.vector_search = FolderIndexRuleSetting::Disabled;
    config.maintenance_rules =
        "该目录由 Unity 官方离线文档自动生成。通过导入功能更新版本，不在这里手动维护页面内容。"
            .to_string();
    config.allow_create_documents = false;
    config.allow_create_directories = false;
    config.allow_move_documents = false;
    config.allow_move_directories = false;
    update_directory_config(working_dir, KnowledgeType::Reference, target_path, config)
        .and_then(|_| {
            update_directory_external_sources(
                working_dir,
                KnowledgeType::Reference,
                target_path,
                vec![KnowledgeExternalSource {
                    provider: KnowledgeSourceProvider::Unity,
                    locator: Some(format!(
                        "project:{};docs:{};locale:{};importedAt:{}",
                        project_version, docs_version, locale, imported_at
                    )),
                    source_id: Some(docs_version.to_string()),
                    sync_enabled: true,
                }],
            )
        })
        .map(|_| ())
}

fn reference_root(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Locus")
        .join("knowledge")
        .join(KnowledgeType::Reference.as_str())
}

fn managed_dir_path(working_dir: &str) -> PathBuf {
    reference_root(working_dir).join(UNITY_REFERENCE_MANAGED_DIR)
}

fn reference_target_managed_path(target_path: &str) -> String {
    format!("reference/{}", target_path.trim().trim_matches('/'))
}

fn reference_target_dir_path(working_dir: &str, target_path: &str) -> PathBuf {
    reference_root(working_dir).join(target_path.trim().trim_matches('/').replace('\\', "/"))
}

fn ensure_reference_target_directory(
    working_dir: &str,
    target_path: &str,
) -> Result<crate::knowledge_store::KnowledgeDirectoryConfigRecord, String> {
    let record =
        knowledge_store::read_directory_config(working_dir, KnowledgeType::Reference, target_path)?;
    if record.read_only {
        return Err("当前 Reference 文件夹是只读目录，无法配置外部导入。".to_string());
    }
    Ok(record)
}

fn delete_target_reference_import_artifacts(
    working_dir: &str,
    target_path: &str,
) -> Result<(), String> {
    let record = ensure_reference_target_directory(working_dir, target_path)?;
    remove_directory_if_exists(
        &reference_target_dir_path(working_dir, &record.path),
        "managed directory",
    )?;
    delete_directory_config_sidecars(working_dir, KnowledgeType::Reference, &record.path)?;
    Ok(())
}

fn parse_locator_parts(locator: Option<&str>) -> std::collections::HashMap<String, String> {
    let mut parts = std::collections::HashMap::new();
    let Some(locator) = locator else {
        return parts;
    };
    for segment in locator.split(';') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }
        parts.insert(key.to_string(), value.to_string());
    }
    parts
}

fn read_unity_directory_binding(
    working_dir: &str,
    target_path: &str,
) -> Result<
    (
        crate::knowledge_store::KnowledgeDirectoryConfigRecord,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    ),
    String,
> {
    let record = ensure_reference_target_directory(working_dir, target_path)?;
    let source = record
        .external_sources
        .iter()
        .find(|item| item.provider == KnowledgeSourceProvider::Unity);
    let parts = parse_locator_parts(source.and_then(|item| item.locator.as_deref()));
    Ok((
        record,
        parts.get("project").cloned(),
        parts.get("docs").cloned(),
        parts.get("locale").cloned(),
        parts
            .get("importedAt")
            .and_then(|value| value.parse::<i64>().ok()),
    ))
}

fn read_unity_target_import_snapshot(
    working_dir: &str,
    target_path: &str,
) -> Result<
    (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        u32,
    ),
    String,
> {
    if !knowledge_store::directory_exists(working_dir, KnowledgeType::Reference, target_path)? {
        return Ok((None, None, None, None, 0));
    }

    let (_, imported_project_version, imported_docs_version, imported_locale, imported_at) =
        read_unity_directory_binding(working_dir, target_path)?;
    let imported_doc_count =
        count_reference_markdown_documents(&reference_target_dir_path(working_dir, target_path))?;
    Ok((
        imported_project_version,
        imported_docs_version,
        imported_locale,
        imported_at,
        imported_doc_count,
    ))
}

fn existing_unity_binding_path(working_dir: &str) -> Result<Option<String>, String> {
    Ok(
        knowledge_store::find_reference_directory_by_external_provider(
            working_dir,
            KnowledgeSourceProvider::Unity,
        )?
        .map(|record| record.path),
    )
}

fn count_reference_markdown_documents(root: &Path) -> Result<u32, String> {
    if !root.exists() {
        return Ok(0);
    }
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0_u32;
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| {
            format!(
                "Failed to read Reference directory '{}': {}",
                current.display(),
                error
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "Failed to inspect Reference directory '{}': {}",
                    current.display(),
                    error
                )
            })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
            {
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

fn publish_target_directory_from_temp_root(
    working_dir: &str,
    target_path: &str,
) -> Result<(), String> {
    let temp_root = temp_managed_dir_path(working_dir);
    let incoming = temp_root.join("reference").join(target_path);
    if !incoming.is_dir() {
        return Err(format!(
            "Unity temporary import directory is missing: {}",
            incoming.display()
        ));
    }
    let managed = reference_target_dir_path(working_dir, target_path);
    remove_directory_if_exists(&managed, "managed directory")?;
    if let Some(parent) = managed.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Unity reference directory parent '{}': {}",
                parent.display(),
                error
            )
        })?;
    }
    std::fs::rename(&incoming, &managed).map_err(|error| {
        format!(
            "Failed to activate Unity reference directory '{}' -> '{}': {}",
            incoming.display(),
            managed.display(),
            error
        )
    })?;
    remove_directory_if_exists(&temp_root, "temporary directory")?;
    Ok(())
}

pub(crate) fn managed_store_path(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join(UNITY_REFERENCE_STORE_FILE)
}

fn managed_directory_config_path(working_dir: &str, suffix: &str) -> PathBuf {
    reference_root(working_dir).join(format!("{}{}", UNITY_REFERENCE_MANAGED_DIR, suffix))
}

fn temp_managed_dir_path(working_dir: &str) -> PathBuf {
    reference_root(working_dir).join(UNITY_REFERENCE_TEMP_DIR)
}

fn temp_managed_store_path(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join(UNITY_REFERENCE_TEMP_STORE_FILE)
}

fn cache_root(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join(UNITY_REFERENCE_CACHE_DIR)
}

fn manifest_path(working_dir: &str) -> PathBuf {
    reference_root(working_dir).join(UNITY_REFERENCE_MANIFEST_FILE)
}

fn legacy_manifest_path(working_dir: &str) -> PathBuf {
    Path::new(working_dir)
        .join("Library")
        .join("Locus")
        .join(UNITY_REFERENCE_MANIFEST_FILE)
}

pub fn is_unity_reference_managed_relative_path(path: &str) -> bool {
    let normalized = path.trim().trim_matches('/').replace('\\', "/");
    let normalized = normalized.strip_prefix("reference/").unwrap_or(&normalized);
    normalized == UNITY_REFERENCE_MANAGED_DIR
        || normalized
            .strip_prefix(UNITY_REFERENCE_MANAGED_DIR)
            .map(|suffix| suffix.starts_with('/'))
            .unwrap_or(false)
}

pub fn has_managed_store(working_dir: &str) -> bool {
    managed_store_path(working_dir).is_file() && has_managed_reference_anchor(working_dir)
}

fn has_managed_reference_anchor(working_dir: &str) -> bool {
    managed_dir_path(working_dir).is_dir()
        || managed_directory_config_path(working_dir, UNITY_REFERENCE_DIRECTORY_CONFIG_SUFFIX)
            .is_file()
        || manifest_path(working_dir).is_file()
        || legacy_manifest_path(working_dir).is_file()
}

fn existing_manifest_path(working_dir: &str) -> Option<PathBuf> {
    let workspace_path = manifest_path(working_dir);
    if workspace_path.is_file() {
        return Some(workspace_path);
    }

    let legacy_path = legacy_manifest_path(working_dir);
    if legacy_path.is_file() {
        return Some(legacy_path);
    }

    None
}

fn read_manifest_file(path: &Path) -> Result<UnityReferenceImportManifest, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read Unity reference manifest '{}': {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&content).map_err(|e| {
        format!(
            "Failed to parse Unity reference manifest '{}': {}",
            path.display(),
            e
        )
    })
}

fn write_manifest_file(path: &Path, manifest: &UnityReferenceImportManifest) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create Unity reference manifest directory '{}': {}",
                parent.display(),
                e
            )
        })?;
    }
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("Failed to serialize Unity reference manifest: {}", e))?;
    std::fs::write(path, body).map_err(|e| {
        format!(
            "Failed to write Unity reference manifest '{}': {}",
            path.display(),
            e
        )
    })
}

fn unity_reference_store_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn open_unity_reference_store(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create Unity reference store directory '{}': {}",
                parent.display(),
                e
            )
        })?;
    }
    let conn = Connection::open(path).map_err(|e| {
        format!(
            "Failed to open Unity reference store '{}': {}",
            path.display(),
            e
        )
    })?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         CREATE TABLE IF NOT EXISTS documents (
             path TEXT PRIMARY KEY,
             doc_id TEXT NOT NULL,
             title TEXT NOT NULL,
             payload_json TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS directories (
             path TEXT PRIMARY KEY,
             direct_child_count INTEGER NOT NULL DEFAULT 0,
             descendant_document_count INTEGER NOT NULL DEFAULT 0,
             direct_document_count INTEGER NOT NULL DEFAULT 0,
             child_dir_count INTEGER NOT NULL DEFAULT 0,
             descendant_byte_size INTEGER NOT NULL DEFAULT 0,
             descendant_estimated_tokens INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS store_summary (
             managed_path TEXT PRIMARY KEY,
             fingerprint TEXT NOT NULL DEFAULT '',
             document_count INTEGER NOT NULL DEFAULT 0,
             directory_count INTEGER NOT NULL DEFAULT 0,
             total_byte_size INTEGER NOT NULL DEFAULT 0,
             manual_doc_count INTEGER NOT NULL DEFAULT 0,
             script_reference_doc_count INTEGER NOT NULL DEFAULT 0,
             imported_at INTEGER,
             updated_at INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_unity_reference_documents_id
             ON documents(doc_id);",
    )
    .map_err(|e| {
        format!(
            "Failed to initialize Unity reference store '{}': {}",
            path.display(),
            e
        )
    })?;
    ensure_unity_reference_store_schema(&conn)?;
    Ok(conn)
}

fn ensure_unity_reference_store_schema(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(directories)")
        .map_err(|e| format!("Failed to inspect Unity reference directory schema: {}", e))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("Failed to read Unity reference directory schema: {}", e))?;
    let columns = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to decode Unity reference directory schema: {}", e))?;
    drop(stmt);

    if !columns.iter().any(|column| column == "direct_child_count") {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN direct_child_count INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference direct child count column: {}",
                e
            )
        })?;
    }
    if !columns
        .iter()
        .any(|column| column == "descendant_document_count")
    {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN descendant_document_count INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference descendant document count column: {}",
                e
            )
        })?;
    }
    if !columns
        .iter()
        .any(|column| column == "direct_document_count")
    {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN direct_document_count INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference direct document count column: {}",
                e
            )
        })?;
    }
    if !columns.iter().any(|column| column == "child_dir_count") {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN child_dir_count INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference child directory count column: {}",
                e
            )
        })?;
    }
    if !columns
        .iter()
        .any(|column| column == "descendant_byte_size")
    {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN descendant_byte_size INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference descendant byte size column: {}",
                e
            )
        })?;
    }
    if !columns
        .iter()
        .any(|column| column == "descendant_estimated_tokens")
    {
        conn.execute(
            "ALTER TABLE directories ADD COLUMN descendant_estimated_tokens INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| {
            format!(
                "Failed to add Unity reference descendant estimated tokens column: {}",
                e
            )
        })?;
    }
    Ok(())
}

fn open_unity_reference_store_readonly(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
        format!(
            "Failed to open Unity reference store '{}': {}",
            path.display(),
            e
        )
    })
}

fn insert_managed_document(
    statement: &mut rusqlite::Statement<'_>,
    document: &KnowledgeDocument,
) -> Result<(), String> {
    let mut normalized = document.clone();
    knowledge_store::apply_external_source_defaults(&mut normalized);
    let payload_json = serde_json::to_string(&normalized)
        .map_err(|e| format!("Failed to serialize Unity reference document: {}", e))?;
    statement
        .execute(params![
            normalized.path,
            normalized.id,
            normalized.title,
            payload_json
        ])
        .map_err(|e| {
            format!(
                "Failed to write Unity reference document '{}': {}",
                normalized.path, e
            )
        })?;
    Ok(())
}

fn collect_managed_directory_paths(doc_path: &str) -> Vec<String> {
    let normalized = doc_path.trim().trim_matches('/').replace('\\', "/");
    let mut current = match Path::new(&normalized).parent() {
        Some(parent) => parent.to_string_lossy().replace('\\', "/"),
        None => String::new(),
    };
    let mut paths = Vec::new();
    while !current.is_empty() && current != "." {
        paths.push(current.clone());
        current = match Path::new(&current).parent() {
            Some(parent) => parent.to_string_lossy().replace('\\', "/"),
            None => String::new(),
        };
    }
    paths.reverse();
    paths
}

fn document_parent_directory(path: &str) -> Option<String> {
    let normalized = path.trim().trim_matches('/').replace('\\', "/");
    let parent = Path::new(&normalized).parent()?;
    let normalized_parent = parent.to_string_lossy().replace('\\', "/");
    let trimmed = normalized_parent.trim_matches('/').trim();
    if trimmed.is_empty() || trimmed == "." {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn insert_managed_directory(
    statement: &mut rusqlite::Statement<'_>,
    path: &str,
    stat: &UnityManagedDirectoryStat,
) -> Result<(), String> {
    statement
        .execute(params![
            path,
            stat.direct_child_count as i64,
            stat.descendant_document_count as i64,
            stat.direct_document_count as i64,
            stat.child_dir_count as i64,
            stat.descendant_byte_size as i64,
            stat.descendant_estimated_tokens as i64,
        ])
        .map_err(|e| {
            format!(
                "Failed to write Unity reference virtual directory '{}': {}",
                path, e
            )
        })?;
    Ok(())
}

#[derive(Default)]
struct ManagedDirectoryAccumulator {
    direct_document_count: usize,
    child_dirs: HashSet<String>,
    descendant_document_count: usize,
    descendant_byte_size: u64,
    descendant_estimated_tokens: u64,
}

#[derive(Debug, Clone, Default)]
struct UnityManagedStoreSummaryMetrics {
    document_count: usize,
    directory_count: usize,
    total_byte_size: u64,
    manual_doc_count: usize,
    script_reference_doc_count: usize,
}

fn estimate_unity_document_tokens(document: &KnowledgeDocument) -> u64 {
    let mut text = String::new();
    if let Some(summary) = knowledge_store::active_summary(document) {
        text.push_str(summary);
        text.push('\n');
    }
    if let Some(rules) = knowledge_store::active_maintenance_rules(document) {
        text.push_str(rules);
        text.push('\n');
    }
    text.push_str(&document.body);
    if text.is_empty() {
        0
    } else {
        ((text.as_bytes().len() as f64) / 3.5).ceil() as u64
    }
}

fn build_managed_directory_stats(
    documents: &[KnowledgeDocument],
) -> (
    std::collections::BTreeMap<String, UnityManagedDirectoryStat>,
    UnityManagedStoreSummaryMetrics,
) {
    let mut acc = std::collections::BTreeMap::<String, ManagedDirectoryAccumulator>::new();
    let mut summary = UnityManagedStoreSummaryMetrics::default();

    for document in documents {
        let document_path = &document.path;
        let normalized = document_path.trim().trim_matches('/').replace('\\', "/");
        if normalized.is_empty() {
            continue;
        }
        summary.document_count += 1;
        summary.total_byte_size +=
            knowledge_store::rendered_document_size_bytes(document).unwrap_or(0);
        if normalized == format!("{}/manual", UNITY_REFERENCE_MANAGED_DIR)
            || normalized.starts_with(&format!("{}/manual/", UNITY_REFERENCE_MANAGED_DIR))
        {
            summary.manual_doc_count += 1;
        } else if normalized == format!("{}/script-reference", UNITY_REFERENCE_MANAGED_DIR)
            || normalized.starts_with(&format!(
                "{}/script-reference/",
                UNITY_REFERENCE_MANAGED_DIR
            ))
        {
            summary.script_reference_doc_count += 1;
        }
        let document_byte_size =
            knowledge_store::rendered_document_size_bytes(document).unwrap_or(0);
        let document_estimated_tokens = estimate_unity_document_tokens(document);

        let directory_paths = collect_managed_directory_paths(&normalized);
        for directory_path in &directory_paths {
            let entry = acc.entry(directory_path.clone()).or_default();
            entry.descendant_document_count += 1;
            entry.descendant_byte_size += document_byte_size;
            entry.descendant_estimated_tokens += document_estimated_tokens;
        }

        if let Some(parent_dir) = document_parent_directory(&normalized) {
            acc.entry(parent_dir).or_default().direct_document_count += 1;
        }

        for directory_path in directory_paths {
            if let Some(parent_dir) = document_parent_directory(&directory_path) {
                acc.entry(parent_dir)
                    .or_default()
                    .child_dirs
                    .insert(directory_path.clone());
            }
        }
    }

    let directory_stats = acc
        .into_iter()
        .map(|(path, value)| {
            (
                path.clone(),
                UnityManagedDirectoryStat {
                    path,
                    direct_child_count: value.direct_document_count + value.child_dirs.len(),
                    descendant_document_count: value.descendant_document_count,
                    direct_document_count: value.direct_document_count,
                    child_dir_count: value.child_dirs.len(),
                    descendant_byte_size: value.descendant_byte_size,
                    descendant_estimated_tokens: value.descendant_estimated_tokens,
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    summary.directory_count = directory_stats.len();

    (directory_stats, summary)
}

fn append_managed_directories(
    tx: &rusqlite::Transaction<'_>,
    documents: &[KnowledgeDocument],
) -> Result<(), String> {
    if documents.is_empty() {
        return Ok(());
    }

    let (directory_stats, _) = build_managed_directory_stats(documents);
    if directory_stats.is_empty() {
        return Ok(());
    }

    let mut insert = tx
        .prepare(
            "INSERT OR IGNORE INTO directories (
                path,
                direct_child_count,
                descendant_document_count,
                direct_document_count,
                child_dir_count,
                descendant_byte_size,
                descendant_estimated_tokens
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|e| format!("Failed to prepare Unity reference directory insert: {}", e))?;
    for stat in directory_stats.values() {
        insert_managed_directory(&mut insert, &stat.path, stat)?;
    }
    Ok(())
}

fn upsert_managed_store_summary(
    tx: &rusqlite::Transaction<'_>,
    summary: &UnityManagedStoreSummary,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO store_summary (
            managed_path,
            fingerprint,
            document_count,
            directory_count,
            total_byte_size,
            manual_doc_count,
            script_reference_doc_count,
            imported_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(managed_path) DO UPDATE SET
            fingerprint = excluded.fingerprint,
            document_count = excluded.document_count,
            directory_count = excluded.directory_count,
            total_byte_size = excluded.total_byte_size,
            manual_doc_count = excluded.manual_doc_count,
            script_reference_doc_count = excluded.script_reference_doc_count,
            imported_at = excluded.imported_at,
            updated_at = excluded.updated_at",
        params![
            summary.managed_path,
            summary.fingerprint,
            summary.document_count as i64,
            summary.directory_count as i64,
            summary.total_byte_size as i64,
            summary.manual_doc_count as i64,
            summary.script_reference_doc_count as i64,
            summary.imported_at,
            summary.updated_at,
        ],
    )
    .map_err(|e| format!("Failed to upsert Unity reference store summary: {}", e))?;
    Ok(())
}

fn rebuild_managed_directory_index(store_path: &Path, conn: &mut Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT payload_json FROM documents ORDER BY path")
        .map_err(|e| {
            format!(
                "Failed to prepare Unity reference directory rebuild query: {}",
                e
            )
        })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            format!(
                "Failed to query Unity reference documents for directory rebuild: {}",
                e
            )
        })?;
    let payloads = rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
        format!(
            "Failed to decode Unity reference documents for directory rebuild: {}",
            e
        )
    })?;
    drop(stmt);
    let documents = deserialize_store_documents(store_path, payloads)?;
    let (directory_stats, summary_metrics) = build_managed_directory_stats(&documents);

    let tx = conn.transaction().map_err(|e| {
        format!(
            "Failed to start Unity reference directory rebuild transaction: {}",
            e
        )
    })?;
    tx.execute("DELETE FROM directories", [])
        .map_err(|e| format!("Failed to reset Unity reference directory index: {}", e))?;
    tx.execute("DELETE FROM store_summary", [])
        .map_err(|e| format!("Failed to reset Unity reference store summary: {}", e))?;

    let mut insert = tx
        .prepare(
            "INSERT OR REPLACE INTO directories (
                path,
                direct_child_count,
                descendant_document_count,
                direct_document_count,
                child_dir_count,
                descendant_byte_size,
                descendant_estimated_tokens
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|e| {
            format!(
                "Failed to prepare Unity reference directory rebuild insert: {}",
                e
            )
        })?;
    for stat in directory_stats.values() {
        insert_managed_directory(&mut insert, &stat.path, stat)?;
    }
    drop(insert);
    upsert_managed_store_summary(
        &tx,
        &UnityManagedStoreSummary {
            managed_path: UNITY_REFERENCE_MANAGED_PATH.to_string(),
            fingerprint: String::new(),
            document_count: summary_metrics.document_count,
            directory_count: summary_metrics.directory_count,
            total_byte_size: summary_metrics.total_byte_size,
            manual_doc_count: summary_metrics.manual_doc_count,
            script_reference_doc_count: summary_metrics.script_reference_doc_count,
            imported_at: None,
            updated_at: Utc::now().timestamp_millis(),
        },
    )?;
    tx.commit()
        .map_err(|e| format!("Failed to commit Unity reference directory rebuild: {}", e))?;
    Ok(())
}

fn sync_managed_store_summary_metadata(working_dir: &str) -> Result<(), String> {
    let Some(snapshot) = current_unity_reference_managed_snapshot(working_dir)? else {
        return Ok(());
    };
    let imported_at = read_manifest(working_dir)?.map(|manifest| manifest.imported_at);
    let store_path = managed_store_path(working_dir);
    if !store_path.is_file() {
        return Ok(());
    }

    let conn = open_unity_reference_store(&store_path)?;
    conn.execute(
        "UPDATE store_summary
         SET fingerprint = ?1,
             imported_at = ?2,
             updated_at = ?3
         WHERE managed_path = ?4",
        params![
            snapshot.fingerprint,
            imported_at,
            Utc::now().timestamp_millis(),
            UNITY_REFERENCE_MANAGED_PATH,
        ],
    )
    .map_err(|e| {
        format!(
            "Failed to sync Unity reference store summary metadata: {}",
            e
        )
    })?;
    Ok(())
}

fn ensure_managed_directory_index_available(working_dir: &str) -> Result<(), String> {
    let store_path = managed_store_path(working_dir);
    if !store_path.is_file() {
        return Ok(());
    }

    let conn = open_unity_reference_store(&store_path)?;
    let directory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM directories", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count Unity reference virtual directories: {}", e))?;
    let document_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count Unity reference documents: {}", e))?;
    let valid_stats_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM directories
             WHERE direct_child_count > 0 OR descendant_document_count > 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| {
            format!(
                "Failed to count Unity reference directory stats rows: {}",
                e
            )
        })?;
    let summary_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM store_summary WHERE managed_path = ?1",
            params![UNITY_REFERENCE_MANAGED_PATH],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count Unity reference store summary rows: {}", e))?;
    if directory_count > 0 && (document_count <= 0 || valid_stats_count > 0) && summary_count > 0 {
        drop(conn);
        return sync_managed_store_summary_metadata(working_dir);
    }
    drop(conn);

    let _guard = unity_reference_store_lock()
        .lock()
        .map_err(|_| "Unity reference store migration lock is poisoned".to_string())?;
    let mut conn = open_unity_reference_store(&store_path)?;
    let directory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM directories", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count Unity reference virtual directories: {}", e))?;
    let document_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count Unity reference documents: {}", e))?;
    let valid_stats_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM directories
             WHERE direct_child_count > 0 OR descendant_document_count > 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| {
            format!(
                "Failed to count Unity reference directory stats rows: {}",
                e
            )
        })?;
    let summary_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM store_summary WHERE managed_path = ?1",
            params![UNITY_REFERENCE_MANAGED_PATH],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count Unity reference store summary rows: {}", e))?;
    if directory_count > 0 && (document_count <= 0 || valid_stats_count > 0) && summary_count > 0 {
        drop(conn);
        return sync_managed_store_summary_metadata(working_dir);
    }
    if document_count <= 0 {
        return Ok(());
    }

    rebuild_managed_directory_index(&store_path, &mut conn)?;
    drop(conn);
    sync_managed_store_summary_metadata(working_dir)
}

fn initialize_empty_managed_store(path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| {
            format!(
                "Failed to reset Unity reference store '{}': {}",
                path.display(),
                e
            )
        })?;
    }
    drop(open_unity_reference_store(path)?);
    Ok(())
}

fn append_documents_to_store(path: &Path, documents: &[KnowledgeDocument]) -> Result<(), String> {
    if documents.is_empty() {
        return Ok(());
    }
    let mut conn = open_unity_reference_store(path)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to start Unity reference store transaction: {}", e))?;
    {
        let mut insert = tx
            .prepare(
                "INSERT OR REPLACE INTO documents (path, doc_id, title, payload_json)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| format!("Failed to prepare Unity reference store insert: {}", e))?;
        for document in documents {
            insert_managed_document(&mut insert, document)?;
        }
    }
    append_managed_directories(&tx, documents)?;
    tx.commit()
        .map_err(|e| format!("Failed to commit Unity reference store transaction: {}", e))?;
    Ok(())
}

fn load_document_from_store_by_path(
    store_path: &Path,
    rel_path: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    if !store_path.is_file() {
        return Ok(None);
    }
    let conn = open_unity_reference_store_readonly(store_path)?;
    let mut stmt = conn
        .prepare("SELECT payload_json FROM documents WHERE path = ?1")
        .map_err(|e| {
            format!(
                "Failed to prepare Unity reference store lookup '{}': {}",
                store_path.display(),
                e
            )
        })?;
    let payload = stmt
        .query_row(params![rel_path], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|e| {
            format!(
                "Failed to read Unity reference document '{}' from '{}': {}",
                rel_path,
                store_path.display(),
                e
            )
        })?;
    payload
        .map(|value| {
            serde_json::from_str::<KnowledgeDocument>(&value)
                .map(|mut document| {
                    knowledge_store::apply_external_source_defaults(&mut document);
                    document
                })
                .map_err(|e| {
                    format!(
                        "Failed to decode Unity reference document '{}' from '{}': {}",
                        rel_path,
                        store_path.display(),
                        e
                    )
                })
        })
        .transpose()
}

fn should_parallelize_document_deserialize(count: usize) -> bool {
    count >= DOCUMENT_DESERIALIZE_PARALLEL_THRESHOLD
        && std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1)
            > 1
}

fn deserialize_store_documents(
    store_path: &Path,
    payloads: Vec<String>,
) -> Result<Vec<KnowledgeDocument>, String> {
    let decode_payload = |payload_json: String| {
        serde_json::from_str::<KnowledgeDocument>(&payload_json)
            .map(|mut document| {
                knowledge_store::apply_external_source_defaults(&mut document);
                document
            })
            .map_err(|e| {
                format!(
                    "Failed to decode Unity reference document from '{}': {}",
                    store_path.display(),
                    e
                )
            })
    };

    if should_parallelize_document_deserialize(payloads.len()) {
        let documents = payloads
            .into_par_iter()
            .map(decode_payload)
            .collect::<Vec<_>>();
        documents.into_iter().collect()
    } else {
        payloads.into_iter().map(decode_payload).collect()
    }
}

fn list_documents_from_store(
    store_path: &Path,
    path_prefix: Option<&str>,
) -> Result<Vec<KnowledgeDocument>, String> {
    if !store_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = open_unity_reference_store_readonly(store_path)?;
    let normalized_prefix = path_prefix
        .map(|value| value.trim().trim_matches('/').replace('\\', "/"))
        .filter(|value| !value.is_empty());

    match normalized_prefix {
        Some(prefix) => {
            let like = format!("{}/%", prefix);
            let mut stmt = conn
                .prepare(
                    "SELECT payload_json
                     FROM documents
                     WHERE path = ?1 OR path LIKE ?2
                     ORDER BY path",
                )
                .map_err(|e| {
                    format!(
                        "Failed to prepare Unity reference prefix query '{}': {}",
                        store_path.display(),
                        e
                    )
                })?;
            let rows = stmt
                .query_map(params![prefix, like], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    format!(
                        "Failed to query Unity reference documents '{}': {}",
                        store_path.display(),
                        e
                    )
                })?;
            let payloads = rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
                format!(
                    "Failed to decode Unity reference store row '{}': {}",
                    store_path.display(),
                    e
                )
            })?;
            deserialize_store_documents(store_path, payloads)
        }
        None => {
            let mut stmt = conn
                .prepare("SELECT payload_json FROM documents ORDER BY path")
                .map_err(|e| {
                    format!(
                        "Failed to prepare Unity reference full scan '{}': {}",
                        store_path.display(),
                        e
                    )
                })?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    format!(
                        "Failed to query Unity reference documents '{}': {}",
                        store_path.display(),
                        e
                    )
                })?;
            let payloads = rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
                format!(
                    "Failed to decode Unity reference store row '{}': {}",
                    store_path.display(),
                    e
                )
            })?;
            deserialize_store_documents(store_path, payloads)
        }
    }
}

pub fn ensure_managed_store_available(_working_dir: &str) -> Result<(), String> {
    Ok(())
}

pub fn load_managed_document(
    working_dir: &str,
    rel_path: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    let normalized_path = knowledge_store::ensure_document_path(rel_path)?;
    if !is_unity_reference_managed_relative_path(&normalized_path) {
        return Ok(None);
    }
    ensure_managed_store_available(working_dir)?;
    if !has_managed_store(working_dir) {
        return Ok(None);
    }
    load_document_from_store_by_path(&managed_store_path(working_dir), &normalized_path)
}

pub fn list_managed_documents(
    working_dir: &str,
    path_prefix: Option<&str>,
) -> Result<Vec<KnowledgeDocument>, String> {
    let normalized_prefix = path_prefix
        .map(|value| value.trim().trim_matches('/').replace('\\', "/"))
        .filter(|value| !value.is_empty());
    if normalized_prefix
        .as_deref()
        .map(|value| !is_unity_reference_managed_relative_path(value))
        .unwrap_or(false)
    {
        return Ok(Vec::new());
    }
    ensure_managed_store_available(working_dir)?;
    if !has_managed_store(working_dir) {
        return Ok(Vec::new());
    }
    list_documents_from_store(
        &managed_store_path(working_dir),
        normalized_prefix.as_deref(),
    )
}

pub fn list_managed_directories(working_dir: &str) -> Result<Vec<String>, String> {
    ensure_managed_store_available(working_dir)?;
    if !has_managed_store(working_dir) {
        return Ok(Vec::new());
    }
    ensure_managed_directory_index_available(working_dir)?;

    let store_path = managed_store_path(working_dir);
    if !store_path.is_file() {
        return Ok(Vec::new());
    }

    let conn = open_unity_reference_store_readonly(&store_path)?;
    let mut stmt = conn
        .prepare("SELECT path FROM directories ORDER BY path")
        .map_err(|e| {
            format!(
                "Failed to prepare Unity reference virtual directory query '{}': {}",
                store_path.display(),
                e
            )
        })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            format!(
                "Failed to query Unity reference virtual directories '{}': {}",
                store_path.display(),
                e
            )
        })?;
    let mut directories = rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
        format!(
            "Failed to decode Unity reference virtual directories '{}': {}",
            store_path.display(),
            e
        )
    })?;
    if directories.is_empty() {
        directories.push(UNITY_REFERENCE_MANAGED_DIR.to_string());
    }
    Ok(directories)
}

pub fn list_managed_directory_stats(
    working_dir: &str,
) -> Result<Vec<UnityManagedDirectoryStat>, String> {
    ensure_managed_store_available(working_dir)?;
    if !has_managed_store(working_dir) {
        return Ok(Vec::new());
    }
    ensure_managed_directory_index_available(working_dir)?;

    let store_path = managed_store_path(working_dir);
    if !store_path.is_file() {
        return Ok(Vec::new());
    }

    let conn = open_unity_reference_store_readonly(&store_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT path, direct_child_count, descendant_document_count,
                    direct_document_count, child_dir_count,
                    descendant_byte_size, descendant_estimated_tokens
             FROM directories
             ORDER BY path",
        )
        .map_err(|e| {
            format!(
                "Failed to prepare Unity reference virtual directory stats query '{}': {}",
                store_path.display(),
                e
            )
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok(UnityManagedDirectoryStat {
                path: row.get(0)?,
                direct_child_count: row.get::<_, i64>(1)?.max(0) as usize,
                descendant_document_count: row.get::<_, i64>(2)?.max(0) as usize,
                direct_document_count: row.get::<_, i64>(3)?.max(0) as usize,
                child_dir_count: row.get::<_, i64>(4)?.max(0) as usize,
                descendant_byte_size: row.get::<_, i64>(5)?.max(0) as u64,
                descendant_estimated_tokens: row.get::<_, i64>(6)?.max(0) as u64,
            })
        })
        .map_err(|e| {
            format!(
                "Failed to query Unity reference virtual directory stats '{}': {}",
                store_path.display(),
                e
            )
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
        format!(
            "Failed to decode Unity reference virtual directory stats '{}': {}",
            store_path.display(),
            e
        )
    })
}

pub fn managed_directory_exists(working_dir: &str, rel_path: &str) -> Result<bool, String> {
    let normalized = rel_path.trim().trim_matches('/').replace('\\', "/");
    let normalized = normalized
        .strip_prefix("reference/")
        .unwrap_or(&normalized)
        .to_string();
    if !is_unity_reference_managed_relative_path(&normalized) {
        return Ok(false);
    }
    ensure_managed_store_available(working_dir)?;
    if normalized == UNITY_REFERENCE_MANAGED_DIR {
        return Ok(has_managed_store(working_dir));
    }
    if !has_managed_store(working_dir) {
        return Ok(false);
    }
    ensure_managed_directory_index_available(working_dir)?;

    let store_path = managed_store_path(working_dir);
    let conn = open_unity_reference_store_readonly(&store_path)?;
    let exists = conn
        .query_row(
            "SELECT 1 FROM directories WHERE path = ?1 LIMIT 1",
            params![normalized],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| {
            format!(
                "Failed to query Unity reference virtual directory '{}': {}",
                rel_path, e
            )
        })?
        .is_some();
    Ok(exists)
}

#[cfg(test)]
pub(crate) fn seed_managed_documents_for_tests(
    working_dir: &str,
    documents: &[KnowledgeDocument],
) -> Result<(), String> {
    remove_directory_if_exists(&managed_dir_path(working_dir), "managed directory")?;
    remove_file_if_exists(&managed_store_path(working_dir), "document store")?;
    let temp_store = temp_managed_store_path(working_dir);
    initialize_empty_managed_store(&temp_store)?;
    append_documents_to_store(&temp_store, documents)?;
    finalize_managed_store(working_dir)?;
    Ok(())
}

pub fn current_unity_reference_managed_snapshot(
    working_dir: &str,
) -> Result<Option<UnityReferenceManagedSnapshot>, String> {
    let Some(manifest) = read_manifest(working_dir)? else {
        return Ok(None);
    };
    ensure_managed_store_available(working_dir)?;
    let store_path = managed_store_path(working_dir);
    if !store_path.is_file() {
        return Ok(None);
    }
    let (fingerprint, document_count) =
        build_managed_store_fingerprint(working_dir, &store_path, &manifest)?;

    Ok(Some(UnityReferenceManagedSnapshot {
        managed_path: UNITY_REFERENCE_MANAGED_PATH.to_string(),
        doc_path_prefix: UNITY_REFERENCE_MANAGED_DIR.to_string(),
        fingerprint,
        document_count,
        expected_document_count: manifest.imported_doc_count as usize,
    }))
}

pub fn managed_document_count_hint(working_dir: &str) -> Result<Option<usize>, String> {
    Ok(read_manifest(working_dir)?.map(|manifest| manifest.imported_doc_count as usize))
}

fn read_manifest(working_dir: &str) -> Result<Option<UnityReferenceImportManifest>, String> {
    let Some(path) = existing_manifest_path(working_dir) else {
        return Ok(None);
    };
    read_manifest_file(&path).map(Some)
}

fn build_managed_store_fingerprint(
    working_dir: &str,
    store_path: &Path,
    manifest: &UnityReferenceImportManifest,
) -> Result<(String, usize), String> {
    let mut hasher = blake3::Hasher::new();
    let manifest_bytes = serde_json::to_vec(manifest).map_err(|e| {
        format!(
            "Failed to serialize Unity reference manifest for fingerprint: {}",
            e
        )
    })?;
    hasher.update(&manifest_bytes);

    let store_meta = std::fs::metadata(store_path).map_err(|e| {
        format!(
            "Failed to stat Unity reference store '{}': {}",
            store_path.display(),
            e
        )
    })?;
    hasher.update(&store_meta.len().to_le_bytes());
    hasher.update(&system_time_millis(store_meta.modified().ok()).to_le_bytes());

    let manifest_path = existing_manifest_path(working_dir)
        .ok_or_else(|| "Unity reference manifest is missing".to_string())?;
    let manifest_meta = std::fs::metadata(&manifest_path).map_err(|e| {
        format!(
            "Failed to stat Unity reference manifest '{}': {}",
            manifest_path.display(),
            e
        )
    })?;
    hasher.update(&manifest_meta.len().to_le_bytes());
    hasher.update(&system_time_millis(manifest_meta.modified().ok()).to_le_bytes());

    Ok((
        hasher.finalize().to_hex().to_string(),
        manifest.imported_doc_count as usize,
    ))
}

fn system_time_millis(value: Option<std::time::SystemTime>) -> u128 {
    value
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn write_manifest(
    working_dir: &str,
    manifest: &UnityReferenceImportManifest,
) -> Result<(), String> {
    let path = manifest_path(working_dir);
    write_manifest_file(&path, manifest)?;
    remove_file_if_exists(&legacy_manifest_path(working_dir), "legacy manifest")?;
    Ok(())
}

fn zip_url_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"https://storage\.googleapis\.com/[^"' )]+UnityDocumentation\.zip"#)
            .expect("zip url regex")
    })
}

fn canonical_url_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"<link\s+rel="canonical"\s+href="([^"]+)""#).expect("canonical url regex")
    })
}

fn title_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?s)<title>(.*?)</title>"#).expect("title regex"))
}

fn meta_description_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"<meta\s+name="description"\s+content="([^"]+)""#)
            .expect("meta description regex")
    })
}

fn first_paragraph_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"(?s)<p[^>]*>(.*?)</p>"#).expect("first paragraph regex"))
}

fn link_attr_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r#"(href|src)=["']([^"']+)["']"#).expect("link attribute regex"))
}

fn multiline_break_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"\n{3,}"#).expect("multiline break regex"))
}

fn html_anchor_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)<a\s+(?:name|id)=["'][^"']+["']\s*></a>"#).expect("html anchor regex")
    })
}

fn manual_breadcrumbs_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?is)<div[^>]*class=["'][^"']*\bbreadcrumbs\b[^"']*["'][^>]*>\s*<ul>(.*?)</ul>\s*</div>"#,
        )
        .expect("manual breadcrumbs regex")
    })
}

fn manual_breadcrumb_item_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?is)<li[^>]*>(.*?)</li>"#).expect("manual breadcrumb item regex")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use tempfile::tempdir;
    use url::Url;

    use super::{
        apply_manual_breadcrumb_hierarchy, bounded_reference_convert_parallelism,
        build_reference_document_batch, build_script_reference_path_context, cache_root,
        classify_html_doc_path, clean_markdown, configure_managed_directory, count_progress_ratio,
        current_unity_reference_managed_snapshot, derive_unity_docs_version,
        ensure_managed_store_available, extract_main_html, extract_manual_breadcrumb_labels,
        extract_title, get_unity_reference_import_status, legacy_manifest_path,
        list_managed_directories, list_managed_directory_stats, list_managed_documents,
        load_managed_document, managed_store_path, manifest_path, normalize_requested_locale,
        normalize_requested_target_path, offline_source_candidates, reconcile_stage_progress,
        resolve_unity_doc_url_to_managed_path, rewrite_relative_urls,
        sanitize_relative_output_path, sanitize_zip_relative_path,
        seed_managed_documents_for_tests, unity_reference_convert_batch_size,
        update_status_from_sync, write_manifest, ScriptReferencePathContext,
        UnityExtractedZipEntry, UnityHtmlDocCandidate, UnityOfflineDocSource,
        UnityReferenceImportLocale, UnityReferenceImportManifest, UnityReferenceImportRuntime,
        UnityReferenceImportStage, UnityReferenceImportStateKind, UnityReferenceImportStatus,
        UNITY_REFERENCE_MANAGED_DIR, UNITY_REFERENCE_MANAGED_PATH,
    };
    use crate::knowledge_store::{
        self, FolderIndexRuleSetting, KnowledgeConfigSource, KnowledgeConfigSourceKind,
        KnowledgeDocument, KnowledgeExternalSource, KnowledgeInjectMode, KnowledgeSourceProvider,
        KnowledgeType,
    };

    fn test_unity_document(path: &str, title: &str) -> KnowledgeDocument {
        KnowledgeDocument {
            id: format!("kd_test_{}", title.replace(' ', "_").to_lowercase()),
            doc_type: KnowledgeType::Reference,
            path: path.to_string(),
            title: title.to_string(),
            inject_mode: KnowledgeInjectMode::None,
            inherit_inject_mode: false,
            inject_mode_source: KnowledgeConfigSource {
                kind: KnowledgeConfigSourceKind::SelfValue,
                path: None,
            },
            summary_enabled: true,
            command_enabled: false,
            read_only: true,
            ai_maintained: false,
            storage_source: knowledge_store::KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: KnowledgeConfigSource {
                kind: KnowledgeConfigSourceKind::SelfValue,
                path: None,
            },
            explicit_maintenance_rules: false,
            external_source: Some(KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Unity,
                locator: Some(
                    "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html".to_string(),
                ),
                source_id: Some("unity-2022.3".to_string()),
                sync_enabled: true,
            }),
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            tools: Vec::new(),
            summary: Some(format!("{} 摘要", title)),
            body: format!("{} 正文", title),
            maintenance_rules: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn delete_target_reference_import_artifacts_removes_directory_and_sidecars() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let target_path = "reference-folder";

        std::fs::create_dir_all(super::reference_target_dir_path(&working_dir, target_path))
            .expect("create reference directory");
        knowledge_store::update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            target_path,
            vec![KnowledgeExternalSource {
                provider: KnowledgeSourceProvider::Unity,
                locator: Some(
                    "project:2022.3.47f1;docs:2022.3;locale:zh-CN;importedAt:123".to_string(),
                ),
                source_id: Some("unity-2022.3".to_string()),
                sync_enabled: true,
            }],
        )
        .expect("seed directory config");

        let legacy_config = crate::knowledge_store::knowledge_root(&working_dir)
            .join("reference")
            .join("reference-folder.meta");
        std::fs::write(&legacy_config, "legacy").expect("write legacy config");

        super::delete_target_reference_import_artifacts(&working_dir, target_path)
            .expect("delete target artifacts");

        assert!(!super::reference_target_dir_path(&working_dir, target_path).exists());
        assert!(!crate::knowledge_store::knowledge_root(&working_dir)
            .join("reference")
            .join("reference-folder.locus-meta")
            .exists());
        assert!(!legacy_config.exists());
    }

    #[test]
    fn derive_unity_docs_version_keeps_major_minor_stream() {
        assert_eq!(
            derive_unity_docs_version("2022.3.21f1"),
            Some("2022.3".to_string())
        );
        assert_eq!(
            derive_unity_docs_version("6000.0.17f1"),
            Some("6000.0".to_string())
        );
        assert_eq!(derive_unity_docs_version(""), None);
    }

    #[test]
    fn normalize_requested_target_path_routes_default_managed_dir_to_sqlite_store() {
        assert_eq!(normalize_requested_target_path(None), None);
        assert_eq!(normalize_requested_target_path(Some("")), None);
        assert_eq!(
            normalize_requested_target_path(Some("unity-official-docs")),
            None
        );
        assert_eq!(
            normalize_requested_target_path(Some("external/unity-official-docs")),
            Some("external/unity-official-docs".to_string())
        );
    }

    #[test]
    fn classify_html_doc_path_maps_manual_and_script_reference() {
        let script_reference_context = build_script_reference_path_context(&[
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform.html".to_string(),
                file_path: PathBuf::from("Transform.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform-position.html"
                    .to_string(),
                file_path: PathBuf::from("Transform-position.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/AI.NavMesh.html".to_string(),
                file_path: PathBuf::from("AI.NavMesh.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/AI.NavMesh.AddLink.html"
                    .to_string(),
                file_path: PathBuf::from("AI.NavMesh.AddLink.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path:
                    "Documentation/ScriptReference/AI.NavMesh-avoidancePredictionTime.html"
                        .to_string(),
                file_path: PathBuf::from("AI.NavMesh-avoidancePredictionTime.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path:
                    "Documentation/ScriptReference/Advertisements.AdvertisementSettings.html"
                        .to_string(),
                file_path: PathBuf::from("Advertisements.AdvertisementSettings.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path:
                    "Documentation/ScriptReference/Advertisements.AdvertisementSettings.GetGameId.html"
                        .to_string(),
                file_path: PathBuf::from(
                    "Advertisements.AdvertisementSettings.GetGameId.html",
                ),
            },
        ]);
        assert_eq!(
            classify_html_doc_path("Manual/ExecutionOrder.html", &script_reference_context),
            Some("manual/ExecutionOrder.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/Transform.html",
                &script_reference_context,
            ),
            Some("script-reference/Transform/Transform.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/Transform-position.html",
                &script_reference_context,
            ),
            Some("script-reference/Transform/Transform-position.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/AI.NavMesh.html",
                &script_reference_context,
            ),
            Some("script-reference/AI/NavMesh/AI.NavMesh.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/AI.NavMesh.AddLink.html",
                &script_reference_context,
            ),
            Some("script-reference/AI/NavMesh/AI.NavMesh.AddLink.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/AI.NavMesh-avoidancePredictionTime.html",
                &script_reference_context,
            ),
            Some("script-reference/AI/NavMesh/AI.NavMesh-avoidancePredictionTime.md".to_string())
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/Advertisements.AdvertisementSettings.html",
                &script_reference_context,
            ),
            Some(
                "script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.md"
                    .to_string()
            )
        );
        assert_eq!(
            classify_html_doc_path(
                "Documentation/ScriptReference/Advertisements.AdvertisementSettings.GetGameId.html",
                &script_reference_context,
            ),
            Some(
                "script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.GetGameId.md"
                    .to_string()
            )
        );
        assert_eq!(
            classify_html_doc_path(
                "Manual/OfflineDocumentation.html",
                &script_reference_context
            ),
            None
        );
        assert_eq!(
            classify_html_doc_path("StaticFiles/js/core.js", &script_reference_context),
            None
        );
    }

    #[test]
    fn resolve_unity_doc_url_to_managed_path_maps_manual_and_script_reference_links() {
        let script_reference_context = build_script_reference_path_context(&[
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform.html".to_string(),
                file_path: PathBuf::from("Transform.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform-position.html"
                    .to_string(),
                file_path: PathBuf::from("Transform-position.html"),
            },
        ]);
        let relative_markdown_lookup = HashMap::from([
            (
                "manual/ExecutionOrder.md".to_string(),
                "manual/Scripting/ExecutionOrder.md".to_string(),
            ),
            (
                "script-reference/Transform/Transform.md".to_string(),
                "script-reference/Transform/Transform.md".to_string(),
            ),
            (
                "script-reference/Transform/Transform-position.md".to_string(),
                "script-reference/Transform/Transform-position.md".to_string(),
            ),
        ]);

        let manual_url = Url::parse(
            "https://docs.unity3d.com/cn/2022.3/Documentation/Manual/ExecutionOrder.html#order-details",
        )
        .expect("manual url");
        assert_eq!(
            resolve_unity_doc_url_to_managed_path(
                &manual_url,
                UNITY_REFERENCE_MANAGED_DIR,
                &script_reference_context,
                &relative_markdown_lookup,
            ),
            Some(
                "reference/unity-official-docs/manual/Scripting/ExecutionOrder.md#order-details"
                    .to_string(),
            )
        );

        let script_reference_url =
            Url::parse("https://docs.unity3d.com/ScriptReference/Transform.html")
                .expect("script reference url");
        assert_eq!(
            resolve_unity_doc_url_to_managed_path(
                &script_reference_url,
                UNITY_REFERENCE_MANAGED_DIR,
                &script_reference_context,
                &relative_markdown_lookup,
            ),
            Some(
                "reference/unity-official-docs/script-reference/Transform/Transform.md".to_string(),
            )
        );
    }

    #[test]
    fn rewrite_relative_urls_localizes_doc_links_and_keeps_assets_absolute() {
        let script_reference_context = build_script_reference_path_context(&[
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform.html".to_string(),
                file_path: PathBuf::from("Transform.html"),
            },
            UnityExtractedZipEntry {
                original_relative_path: "Documentation/ScriptReference/Transform-position.html"
                    .to_string(),
                file_path: PathBuf::from("Transform-position.html"),
            },
        ]);
        let relative_markdown_lookup = HashMap::from([(
            "script-reference/Transform/Transform.md".to_string(),
            "script-reference/Transform/Transform.md".to_string(),
        )]);
        let html = r#"
            <p><a href="../ScriptReference/Transform.html#api">Transform</a></p>
            <p><img src="../uploads/Main/2D-primitive-triangle.png" /></p>
        "#;

        let rewritten = rewrite_relative_urls(
            html,
            "https://docs.unity3d.com/cn/2022.3/Documentation/Manual/2DPrimitiveObjects.html",
            UNITY_REFERENCE_MANAGED_DIR,
            &script_reference_context,
            &relative_markdown_lookup,
        );

        assert!(rewritten.contains(
            r#"href="reference/unity-official-docs/script-reference/Transform/Transform.md#api""#
        ));
        assert!(rewritten.contains(r#"src="https://docs.unity3d.com/"#));
        assert!(rewritten.contains("2D-primitive-triangle.png"));
    }

    #[test]
    fn extract_title_strips_chinese_unity_script_api_suffix() {
        let raw_html = r#"<html><head><title>Shader.Find - Unity 脚本 API</title></head></html>"#;
        let markdown = "# Shader.Find";
        assert_eq!(
            extract_title(raw_html, markdown, "script-reference/Shader/Shader.Find.md"),
            "Shader.Find".to_string()
        );
    }

    #[test]
    fn clean_markdown_removes_import_noise_lines_and_html_anchors() {
        let markdown = r#"
[切换到手册](reference/unity-official-docs/manual/Shader.Find.md)
<a name="find"></a>
# Shader.Find

Leave feedback
Suggest a change

```csharp
// Switch to Manual
```
"#;

        let cleaned = clean_markdown(markdown);
        assert!(!cleaned.contains("切换到手册"));
        assert!(!cleaned.contains("Leave feedback"));
        assert!(!cleaned.contains("Suggest a change"));
        assert!(!cleaned.contains(r#"<a name="find"></a>"#));
        assert!(cleaned.contains("# Shader.Find"));
        assert!(cleaned.contains("// Switch to Manual"));
    }

    #[test]
    fn extract_manual_breadcrumb_labels_reads_nested_manual_hierarchy() {
        let raw_html = r#"
            <div id="content-wrap" class="content-wrap">
              <div class="section">
                <div class="breadcrumbs clear">
                  <ul>
                    <li><a href="Graphics.html">Graphics</a></li>
                    <li><a href="mesh.html">Meshes</a></li>
                    <li>Mesh data</li>
                  </ul>
                </div>
                <h1>Mesh data</h1>
              </div>
            </div>
        "#;

        assert_eq!(
            extract_manual_breadcrumb_labels(raw_html),
            vec![
                "Graphics".to_string(),
                "Meshes".to_string(),
                "Mesh data".to_string(),
            ]
        );
    }

    #[test]
    fn extract_main_html_uses_content_wrap_and_trims_footer() {
        let html = r#"
            <html>
              <body>
                <div id="content-wrap" class="content-wrap">
                  <div class="content-block">
                    <div class="content">
                      <div class="section">
                        <h1>Transform</h1>
                        <p>Position, rotation and scale.</p>
                        <div id="_content"></div>
                        <div class="footer-wrapper"></div>
                      </div>
                    </div>
                  </div>
                </div>
              </body>
            </html>
        "#;
        let extracted = extract_main_html(html).expect("main html");
        assert!(extracted.contains("<h1>Transform</h1>"));
        assert!(extracted.contains("Position, rotation and scale."));
        assert!(!extracted.contains("footer-wrapper"));
    }

    #[test]
    fn sanitize_zip_relative_path_rewrites_windows_unsafe_names() {
        let mut used = HashSet::new();
        let sanitized = sanitize_zip_relative_path(
            Path::new(
                "ScriptReference/Unity.Collections.NativeSlice_1-operator_NativeArray<T>.html",
            ),
            &mut used,
        );
        assert_eq!(
            sanitized.to_string_lossy().replace('\\', "/"),
            "ScriptReference/Unity.Collections.NativeSlice_1-operator_NativeArray_T_.html"
        );
    }

    #[test]
    fn sanitize_relative_output_path_rewrites_windows_unsafe_markdown_names() {
        let mut used = HashSet::new();
        let sanitized = sanitize_relative_output_path(
            "script-reference/Unity.Collections.NativeSlice_1-operator_NativeArray<T>.md",
            &mut used,
        );
        assert_eq!(
            sanitized,
            "script-reference/Unity.Collections.NativeSlice_1-operator_NativeArray_T_.md"
        );
    }

    #[test]
    fn count_progress_ratio_clamps_by_total() {
        assert_eq!(count_progress_ratio(0, 10), 0.0);
        assert_eq!(count_progress_ratio(5, 10), 0.5);
        assert_eq!(count_progress_ratio(12, 10), 1.0);
        assert_eq!(count_progress_ratio(0, 0), 1.0);
    }

    #[test]
    fn bounded_reference_convert_parallelism_limits_worker_count() {
        assert_eq!(bounded_reference_convert_parallelism(8, 16), 1);
        assert_eq!(bounded_reference_convert_parallelism(32, 1), 1);
        assert_eq!(bounded_reference_convert_parallelism(32, 4), 4);
        assert_eq!(bounded_reference_convert_parallelism(128, 32), 8);
    }

    #[test]
    fn unity_reference_convert_batch_size_scales_with_parallelism() {
        assert_eq!(unity_reference_convert_batch_size(1), 48);
        assert_eq!(unity_reference_convert_batch_size(4), 48);
        assert_eq!(unity_reference_convert_batch_size(8), 96);
    }

    #[test]
    fn apply_manual_breadcrumb_hierarchy_places_container_pages_inside_folders() {
        let workspace = tempdir().expect("workspace");
        let graphics_path = workspace.path().join("Graphics.html");
        let meshes_path = workspace.path().join("mesh.html");
        let mesh_data_path = workspace.path().join("AnatomyofaMesh.html");
        std::fs::write(
            &graphics_path,
            r#"
                <div id="content-wrap">
                  <div class="section">
                    <div class="breadcrumbs clear">
                      <ul>
                        <li>Graphics</li>
                      </ul>
                    </div>
                    <h1>Graphics</h1>
                    <div id="_content"></div>
                  </div>
                </div>
            "#,
        )
        .expect("write graphics html");
        std::fs::write(
            &meshes_path,
            r#"
                <div id="content-wrap">
                  <div class="section">
                    <div class="breadcrumbs clear">
                      <ul>
                        <li><a href="Graphics.html">Graphics</a></li>
                        <li>Meshes</li>
                      </ul>
                    </div>
                    <h1>Meshes</h1>
                    <div id="_content"></div>
                  </div>
                </div>
            "#,
        )
        .expect("write meshes html");
        std::fs::write(
            &mesh_data_path,
            r#"
                <div id="content-wrap">
                  <div class="section">
                    <div class="breadcrumbs clear">
                      <ul>
                        <li><a href="Graphics.html">Graphics</a></li>
                        <li><a href="mesh.html">Meshes</a></li>
                        <li>Mesh data</li>
                      </ul>
                    </div>
                    <h1>Mesh data</h1>
                    <div id="_content"></div>
                  </div>
                </div>
            "#,
        )
        .expect("write mesh data html");

        let candidates = vec![
            UnityHtmlDocCandidate {
                raw_relative_markdown_path: "manual/Graphics.md".to_string(),
                relative_markdown_path: "manual/Graphics.md".to_string(),
                html_relative_path: "Documentation/Manual/Graphics.html".to_string(),
                file_path: graphics_path,
            },
            UnityHtmlDocCandidate {
                raw_relative_markdown_path: "manual/mesh.md".to_string(),
                relative_markdown_path: "manual/mesh.md".to_string(),
                html_relative_path: "Documentation/Manual/mesh.html".to_string(),
                file_path: meshes_path,
            },
            UnityHtmlDocCandidate {
                raw_relative_markdown_path: "manual/AnatomyofaMesh.md".to_string(),
                relative_markdown_path: "manual/AnatomyofaMesh.md".to_string(),
                html_relative_path: "Documentation/Manual/AnatomyofaMesh.html".to_string(),
                file_path: mesh_data_path,
            },
        ];

        let updated = apply_manual_breadcrumb_hierarchy(&candidates, None)
            .expect("apply breadcrumb hierarchy");
        let updated_paths = updated
            .iter()
            .map(|candidate| candidate.relative_markdown_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            updated_paths,
            vec![
                "manual/Graphics/Graphics.md".to_string(),
                "manual/Graphics/Meshes/AnatomyofaMesh.md".to_string(),
                "manual/Graphics/Meshes/mesh.md".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_requested_locale_defaults_to_english() {
        assert_eq!(
            normalize_requested_locale(None).expect("default locale"),
            UnityReferenceImportLocale::En
        );
        assert_eq!(
            normalize_requested_locale(Some("zh-CN")).expect("zh locale"),
            UnityReferenceImportLocale::ZhCn
        );
        assert!(normalize_requested_locale(Some("jp")).is_err());
    }

    #[test]
    fn offline_source_candidates_follow_requested_language() {
        assert_eq!(
            offline_source_candidates("2022.3", UnityReferenceImportLocale::En),
            vec![
                "https://docs.unity3d.com/2022.3/Documentation/Manual/OfflineDocumentation.html"
                    .to_string(),
                "https://docs.unity3d.com/2022.3/Manual/OfflineDocumentation.html".to_string(),
            ]
        );
        assert_eq!(
            offline_source_candidates("2022.3", UnityReferenceImportLocale::ZhCn),
            vec!["https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html".to_string(),]
        );
    }

    #[tokio::test]
    async fn build_reference_document_batch_preserves_candidate_order() {
        let workspace = tempdir().expect("workspace");
        let html_root = workspace.path();
        let first_path = html_root.join("beta.html");
        let second_path = html_root.join("alpha.html");
        std::fs::write(
            &first_path,
            r#"
                <html>
                  <body>
                    <div id="content-wrap">
                      <h1>Beta</h1>
                      <p>Beta summary.</p>
                      <div id="_content"></div>
                    </div>
                  </body>
                </html>
            "#,
        )
        .expect("write beta html");
        std::fs::write(
            &second_path,
            r#"
                <html>
                  <body>
                    <div id="content-wrap">
                      <h1>Alpha</h1>
                      <p>Alpha summary.</p>
                      <div id="_content"></div>
                    </div>
                  </body>
                </html>
            "#,
        )
        .expect("write alpha html");

        let candidates = vec![
            UnityHtmlDocCandidate {
                raw_relative_markdown_path: "manual/Beta.md".to_string(),
                relative_markdown_path: "manual/Beta.md".to_string(),
                html_relative_path: "Documentation/Manual/Beta.html".to_string(),
                file_path: first_path,
            },
            UnityHtmlDocCandidate {
                raw_relative_markdown_path: "manual/Alpha.md".to_string(),
                relative_markdown_path: "manual/Alpha.md".to_string(),
                html_relative_path: "Documentation/Manual/Alpha.html".to_string(),
                file_path: second_path,
            },
        ];
        let relative_markdown_lookup = HashMap::from([
            ("manual/Beta.md".to_string(), "manual/Beta.md".to_string()),
            ("manual/Alpha.md".to_string(), "manual/Alpha.md".to_string()),
        ]);

        let documents = build_reference_document_batch(
            &candidates,
            Arc::new(UnityOfflineDocSource {
                page_url: "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html"
                    .to_string(),
                zip_url: "https://docs.unity3d.com/UnityDocumentation.zip".to_string(),
                locale: "en".to_string(),
            }),
            Arc::new("unity-official-docs:2022.3:en".to_string()),
            Arc::new("2022.3".to_string()),
            Arc::new(UNITY_REFERENCE_MANAGED_DIR.to_string()),
            Arc::new(None),
            Arc::new(ScriptReferencePathContext::default()),
            Arc::new(relative_markdown_lookup),
            Arc::new(AtomicBool::new(false)),
            4,
        )
        .await
        .expect("build batch");

        assert_eq!(documents.len(), 2);
        assert_eq!(
            documents[0].path,
            "unity-official-docs/manual/Beta.md".to_string()
        );
        assert_eq!(
            documents[1].path,
            "unity-official-docs/manual/Alpha.md".to_string()
        );
        assert_eq!(documents[0].title, "Beta");
        assert_eq!(documents[1].title, "Alpha");
    }

    #[tokio::test]
    async fn build_reference_document_batch_falls_back_to_zip_when_extracted_file_is_missing() {
        use std::io::Write as _;

        let workspace = tempdir().expect("workspace");
        let zip_path = workspace.path().join("UnityDocumentation.zip");
        let zip_file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip_writer = zip::ZipWriter::new(zip_file);
        zip_writer
            .start_file(
                "Documentation/en/Manual/2D-introduction.html",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start zip file");
        zip_writer
            .write_all(
                br#"
                    <html>
                      <body>
                        <div id="content-wrap">
                          <h1>2D Introduction</h1>
                          <p>Welcome to Unity 2D.</p>
                          <div id="_content"></div>
                        </div>
                      </body>
                    </html>
                "#,
            )
            .expect("write html entry");
        zip_writer.finish().expect("finish zip");

        let candidates = vec![UnityHtmlDocCandidate {
            raw_relative_markdown_path: "manual/2D-introduction.md".to_string(),
            relative_markdown_path: "manual/2D-introduction.md".to_string(),
            html_relative_path: "Documentation/en/Manual/2D-introduction.html".to_string(),
            file_path: workspace
                .path()
                .join("extracted")
                .join("Documentation")
                .join("en")
                .join("Manual")
                .join("2D-introduction.html"),
        }];
        let relative_markdown_lookup = HashMap::from([(
            "manual/2D-introduction.md".to_string(),
            "manual/2D-introduction.md".to_string(),
        )]);

        let documents = build_reference_document_batch(
            &candidates,
            Arc::new(UnityOfflineDocSource {
                page_url: "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html"
                    .to_string(),
                zip_url:
                    "https://storage.googleapis.com/docscloudstorage/2022.3/UnityDocumentation.zip"
                        .to_string(),
                locale: "en".to_string(),
            }),
            Arc::new("unity-official-docs:2022.3:en".to_string()),
            Arc::new("2022.3".to_string()),
            Arc::new(UNITY_REFERENCE_MANAGED_DIR.to_string()),
            Arc::new(Some(zip_path)),
            Arc::new(ScriptReferencePathContext::default()),
            Arc::new(relative_markdown_lookup),
            Arc::new(AtomicBool::new(false)),
            1,
        )
        .await
        .expect("build batch from zip fallback");

        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].path,
            "unity-official-docs/manual/2D-introduction.md".to_string()
        );
        assert_eq!(documents[0].title, "2D-introduction");
        assert!(documents[0].body.contains("Welcome to Unity 2D."));
    }

    #[test]
    fn reconcile_stage_progress_advances_across_internal_phases() {
        let preparing = reconcile_stage_progress("preparing", 0, 0);
        let preparing_mid = reconcile_stage_progress("preparing", 50, 100);
        let cleaning = reconcile_stage_progress("cleaning", 10, 100);
        let indexing = reconcile_stage_progress("indexing", 60, 100);
        let committing = reconcile_stage_progress("committing", 100, 100);

        assert!(preparing > 0.0);
        assert!(preparing_mid > preparing);
        assert!(cleaning > preparing_mid);
        assert!(indexing > cleaning);
        assert_eq!(committing, 1.0);
    }

    #[test]
    fn managed_store_round_trips_documents() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "Execution Order",
        )];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "en".to_string(),
                imported_at: 1,
                imported_doc_count: documents.len() as u32,
                source_url: "https://docs.unity3d.com".to_string(),
            },
        )
        .expect("write manifest");

        let loaded =
            load_managed_document(&working_dir, "unity-official-docs/manual/ExecutionOrder.md")
                .expect("load managed doc")
                .expect("managed doc exists");
        assert_eq!(loaded.id, "kd_test_execution_order");
        assert!(!loaded.summary_enabled);

        let listed = list_managed_documents(&working_dir, Some("unity-official-docs/manual"))
            .expect("list managed docs");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].path,
            "unity-official-docs/manual/ExecutionOrder.md"
        );
        assert!(!listed[0].summary_enabled);
    }

    #[test]
    fn ensure_managed_store_available_preserves_legacy_markdown_layout() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let document = test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "ExecutionOrder",
        );
        knowledge_store::save_document(&working_dir, document.clone()).expect("save legacy doc");
        let legacy_file =
            knowledge_store::document_path(&working_dir, KnowledgeType::Reference, &document.path)
                .expect("legacy path");
        assert!(legacy_file.is_file());

        ensure_managed_store_available(&working_dir).expect("preserve legacy layout");

        assert!(!managed_store_path(&working_dir).is_file());
        assert!(legacy_file.is_file());
        assert!(load_managed_document(&working_dir, &document.path)
            .expect("load preserved legacy doc")
            .is_none());
    }

    #[test]
    fn current_unity_reference_managed_snapshot_supports_legacy_manifest_without_migration() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "Execution Order",
        )];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");

        let legacy_manifest = legacy_manifest_path(&working_dir);
        std::fs::create_dir_all(legacy_manifest.parent().expect("legacy manifest parent"))
            .expect("create legacy manifest parent");
        std::fs::write(
            &legacy_manifest,
            serde_json::json!({
                "projectVersion": "2022.3.47f1",
                "docsVersion": "2022.3",
                "locale": "zh-CN",
                "importedAt": 1,
                "importedDocCount": 1,
                "sourceUrl": "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
            })
            .to_string(),
        )
        .expect("write legacy manifest");

        let snapshot = current_unity_reference_managed_snapshot(&working_dir)
            .expect("load snapshot with legacy manifest")
            .expect("snapshot exists");

        assert_eq!(snapshot.document_count, 1);
        assert!(!manifest_path(&working_dir).is_file());
        assert!(legacy_manifest.is_file());
    }

    #[test]
    fn ensure_managed_store_available_preserves_orphaned_store_when_locus_is_deleted() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "Execution Order",
        )];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "zh-CN".to_string(),
                imported_at: 1,
                imported_doc_count: 1,
                source_url: "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
                    .to_string(),
            },
        )
        .expect("write workspace manifest");
        assert!(managed_store_path(&working_dir).is_file());

        std::fs::remove_dir_all(Path::new(&working_dir).join("Locus")).expect("remove Locus");

        ensure_managed_store_available(&working_dir).expect("preserve orphaned store");

        assert!(managed_store_path(&working_dir).is_file());
        assert!(
            list_managed_documents(&working_dir, Some("unity-official-docs"))
                .expect("list after orphan preservation")
                .is_empty()
        );
    }

    #[test]
    fn snapshot_reads_document_count_from_managed_store() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "Execution Order",
        )];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");

        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "en".to_string(),
                imported_at: 1,
                imported_doc_count: 1,
                source_url: "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html"
                    .to_string(),
            },
        )
        .expect("write manifest");

        let snapshot = current_unity_reference_managed_snapshot(&working_dir)
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(snapshot.managed_path, "reference/unity-official-docs");
        assert_eq!(snapshot.doc_path_prefix, UNITY_REFERENCE_MANAGED_DIR);
        assert_eq!(snapshot.document_count, 1);
        assert_eq!(snapshot.expected_document_count, 1);
    }

    #[test]
    fn managed_directory_defaults_disable_vector_search() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![test_unity_document(
            "unity-official-docs/manual/ExecutionOrder.md",
            "Execution Order",
        )];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "zh-CN".to_string(),
                imported_at: 1,
                imported_doc_count: 1,
                source_url: "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
                    .to_string(),
            },
        )
        .expect("write manifest");

        configure_managed_directory(
            &working_dir,
            UNITY_REFERENCE_MANAGED_DIR,
            "2022.3.47f1",
            "2022.3",
            "zh-CN",
            1,
        )
        .expect("configure managed directory");

        let record = knowledge_store::read_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            UNITY_REFERENCE_MANAGED_DIR,
        )
        .expect("read managed directory config");

        assert_eq!(
            record.config.lexical_search,
            FolderIndexRuleSetting::Inherit
        );
        assert_eq!(
            record.config.vector_search,
            FolderIndexRuleSetting::Disabled
        );
        assert!(record.effective_lexical_search.enabled);
        assert!(!record.effective_vector_search.enabled);
        assert_eq!(record.external_sources.len(), 1);
        assert_eq!(
            record.external_sources[0].provider,
            KnowledgeSourceProvider::Unity
        );
        assert_eq!(
            record.external_sources[0].source_id.as_deref(),
            Some("2022.3")
        );
    }

    #[test]
    fn managed_virtual_directories_are_listed_from_store() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![
            test_unity_document(
                "unity-official-docs/manual/ExecutionOrder.md",
                "Execution Order",
            ),
            test_unity_document(
                "unity-official-docs/script-reference/Transform/Transform.md",
                "Transform",
            ),
        ];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "zh-CN".to_string(),
                imported_at: 1,
                imported_doc_count: 2,
                source_url: "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
                    .to_string(),
            },
        )
        .expect("write manifest");

        let directories = list_managed_directories(&working_dir).expect("list managed dirs");
        assert!(directories.contains(&"unity-official-docs".to_string()));
        assert!(directories.contains(&"unity-official-docs/manual".to_string()));
        assert!(directories.contains(&"unity-official-docs/script-reference".to_string()));
        assert!(directories.contains(&"unity-official-docs/script-reference/Transform".to_string()));
    }

    #[test]
    fn managed_directory_stats_are_cached_in_store() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let documents = vec![
            test_unity_document(
                "unity-official-docs/manual/ExecutionOrder.md",
                "Execution Order",
            ),
            test_unity_document(
                "unity-official-docs/script-reference/Transform/Transform.md",
                "Transform",
            ),
        ];
        seed_managed_documents_for_tests(&working_dir, &documents).expect("seed managed store");
        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "zh-CN".to_string(),
                imported_at: 1,
                imported_doc_count: 2,
                source_url: "https://docs.unity3d.com/cn/2022.3/Manual/OfflineDocumentation.html"
                    .to_string(),
            },
        )
        .expect("write manifest");

        let stats =
            list_managed_directory_stats(&working_dir).expect("list managed directory stats");
        assert!(stats.iter().any(|stat| {
            stat.path == "unity-official-docs"
                && stat.direct_child_count == 2
                && stat.descendant_document_count == 2
                && stat.direct_document_count == 0
                && stat.child_dir_count == 2
        }));
        assert!(stats.iter().any(|stat| {
            stat.path == "unity-official-docs/manual"
                && stat.direct_child_count == 1
                && stat.descendant_document_count == 1
                && stat.direct_document_count == 1
                && stat.child_dir_count == 0
        }));
        assert!(stats.iter().any(|stat| {
            stat.path == "unity-official-docs/script-reference"
                && stat.direct_child_count == 1
                && stat.descendant_document_count == 1
                && stat.direct_document_count == 0
                && stat.child_dir_count == 1
        }));
        assert!(stats.iter().any(|stat| {
            stat.path == "unity-official-docs/script-reference/Transform"
                && stat.direct_child_count == 1
                && stat.descendant_document_count == 1
                && stat.direct_document_count == 1
                && stat.child_dir_count == 0
        }));
    }

    #[tokio::test]
    async fn sync_status_update_does_not_block_inside_runtime() {
        let state = Arc::new(tokio::sync::Mutex::new(
            UnityReferenceImportRuntime::default(),
        ));

        update_status_from_sync(&state, "F:/workspace", |status| {
            status.message = "正在重建索引".to_string();
            status.progress = Some(0.5);
        });

        let runtime = state.lock().await;
        assert_eq!(runtime.working_dir, "F:/workspace");
        assert_eq!(runtime.status.message, "正在重建索引");
        assert_eq!(runtime.status.progress, Some(0.5));
    }

    #[tokio::test]
    async fn get_status_keeps_cache_root_while_import_is_running() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let project_settings = Path::new(&working_dir).join("ProjectSettings");
        std::fs::create_dir_all(&project_settings).expect("create project settings");
        std::fs::write(
            project_settings.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\n",
        )
        .expect("write project version");

        let cache_dir = cache_root(&working_dir);
        std::fs::create_dir_all(&cache_dir).expect("create cache root");

        let state = Arc::new(tokio::sync::Mutex::new(UnityReferenceImportRuntime {
            working_dir: working_dir.clone(),
            status: UnityReferenceImportStatus {
                running: true,
                stage: UnityReferenceImportStage::Downloading,
                message: "正在下载 Unity 官方离线文档。".to_string(),
                ..UnityReferenceImportStatus::default()
            },
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }));

        let status = get_unity_reference_import_status(&working_dir, None, state)
            .await
            .expect("get status");

        assert!(cache_dir.is_dir());
        assert!(status.running);
        assert_eq!(status.stage, UnityReferenceImportStage::Downloading);
    }

    #[tokio::test]
    async fn get_status_for_missing_target_directory_returns_missing_state() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let project_settings = Path::new(&working_dir).join("ProjectSettings");
        std::fs::create_dir_all(&project_settings).expect("create project settings");
        std::fs::write(
            project_settings.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\n",
        )
        .expect("write project version");

        let state = Arc::new(tokio::sync::Mutex::new(
            UnityReferenceImportRuntime::default(),
        ));
        let status = get_unity_reference_import_status(
            &working_dir,
            Some("external/unity-official-docs"),
            state,
        )
        .await
        .expect("get status");

        assert!(!status.running);
        assert_eq!(
            status.state,
            UnityReferenceImportStateKind::MissingCurrentVersion
        );
        assert_eq!(status.stage, UnityReferenceImportStage::Idle);
        assert_eq!(
            status.managed_path,
            "reference/external/unity-official-docs"
        );
        assert_eq!(status.imported_doc_count, 0);
        assert!(status.error.is_none());
    }

    #[tokio::test]
    async fn get_status_for_default_managed_target_path_uses_sqlite_managed_slot() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let project_settings = Path::new(&working_dir).join("ProjectSettings");
        std::fs::create_dir_all(&project_settings).expect("create project settings");
        std::fs::write(
            project_settings.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\n",
        )
        .expect("write project version");

        write_manifest(
            &working_dir,
            &UnityReferenceImportManifest {
                project_version: "2022.3.47f1".to_string(),
                docs_version: "2022.3".to_string(),
                locale: "zh-CN".to_string(),
                imported_at: 123,
                imported_doc_count: 42,
                source_url: "https://docs.unity3d.com".to_string(),
            },
        )
        .expect("write manifest");

        let state = Arc::new(tokio::sync::Mutex::new(
            UnityReferenceImportRuntime::default(),
        ));
        let status =
            get_unity_reference_import_status(&working_dir, Some("unity-official-docs"), state)
                .await
                .expect("get status");

        assert!(!status.running);
        assert_eq!(status.state, UnityReferenceImportStateKind::Ready);
        assert_eq!(status.stage, UnityReferenceImportStage::Ready);
        assert_eq!(status.managed_path, UNITY_REFERENCE_MANAGED_PATH);
        assert_eq!(status.imported_doc_count, 42);
        assert_eq!(status.imported_locale.as_deref(), Some("zh-CN"));
    }
}
