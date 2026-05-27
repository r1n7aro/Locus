use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::Utc;
use rand::distributions::Alphanumeric;
use rand::Rng;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use url::Url;
use uuid::Uuid;

use crate::commands;
use crate::keychain;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{
    self, KnowledgeConfigSource, KnowledgeConfigSourceKind, KnowledgeDocument,
    KnowledgeExternalSource, KnowledgeInjectMode, KnowledgeSourceProvider, KnowledgeType,
};

pub const FEISHU_REFERENCE_MANAGED_DIR: &str = "feishu-knowledge-base";
pub const FEISHU_REFERENCE_MANAGED_PATH: &str = "reference/feishu-knowledge-base";
const FEISHU_REFERENCE_CONFIG_FILE: &str = "feishu_reference_config.json";
const FEISHU_REFERENCE_MANIFEST_FILE: &str = "feishu_reference_manifest.json";
const FEISHU_REFERENCE_DIRECTORY_BINDING_ROOT_DIR: &str = "feishu_reference_directory_bindings";
const FEISHU_REFERENCE_DIRECTORY_BINDING_FILE: &str = "feishu_reference_directory_binding.json";
const FEISHU_REFERENCE_TEMP_ROOT_DIR: &str = ".feishu-reference-import";
const FEISHU_REFERENCE_BACKUP_DIR: &str = ".feishu-reference-backup";
const FEISHU_REFERENCE_DIRECTORY_CONFIG_SUFFIX: &str = ".locus-meta";
const FEISHU_REFERENCE_LEGACY_DIRECTORY_CONFIG_SUFFIX: &str = ".meta";
const FEISHU_REFERENCE_DEFAULT_OPEN_BASE_URL: &str = "https://open.feishu.cn";
const FEISHU_REFERENCE_DEFAULT_ACCOUNTS_BASE_URL: &str = "https://accounts.feishu.cn";
const FEISHU_REFERENCE_OAUTH_CALLBACK_HOST: &str = "127.0.0.1";
const FEISHU_REFERENCE_OAUTH_CALLBACK_PATH: &str = "/oauth/feishu/reference/callback";
const FEISHU_REFERENCE_OAUTH_CALLBACK_PORTS: &[u16] = &[39241, 39242, 39243, 39244];
const FEISHU_REFERENCE_OAUTH_WAIT_SECS: u64 = 300;
const FEISHU_REFERENCE_RAW_CONTENT_INTERVAL_MS: u64 = 220;
const FEISHU_REFERENCE_USER_AUTH_BASE_SCOPES: &[&str] = &[
    "wiki:space:retrieve",
    "wiki:wiki:readonly",
    "docx:document:readonly",
];
const FEISHU_REFERENCE_USER_AUTH_OFFLINE_SCOPE: &str = "offline_access";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeishuReferenceAuthMode {
    #[default]
    AppCredentials,
    Oauth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeishuReferenceOauthPersistenceMode {
    #[default]
    Session,
    Offline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeishuReferenceImportStage {
    #[default]
    Idle,
    SavingConfig,
    Authorizing,
    TestingConnection,
    ListingSpaces,
    ListingNodes,
    Importing,
    Reconciling,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeishuReferenceImportStateKind {
    #[default]
    MissingConfig,
    NeedsAuthorization,
    Running,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeishuReferenceImportLastOutcome {
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceRootSelection {
    pub node_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceImportStatus {
    pub state: FeishuReferenceImportStateKind,
    pub stage: FeishuReferenceImportStage,
    pub running: bool,
    pub auth_mode: FeishuReferenceAuthMode,
    pub oauth_persistence_mode: FeishuReferenceOauthPersistenceMode,
    pub app_id: String,
    pub app_secret: Option<String>,
    pub app_secret_configured: bool,
    pub authorized: bool,
    pub authorized_user_name: Option<String>,
    pub authorized_user_open_id: Option<String>,
    pub authorized_user_email: Option<String>,
    pub open_base_url: String,
    pub callback_urls: Vec<String>,
    pub required_scopes: Vec<String>,
    pub granted_scopes: Vec<String>,
    pub missing_scopes: Vec<String>,
    pub access_token_expires_at: Option<i64>,
    pub refresh_token_expires_at: Option<i64>,
    pub can_refresh: bool,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_roots: Vec<FeishuReferenceRootSelection>,
    pub root_node_token: Option<String>,
    pub root_node_title: Option<String>,
    pub imported_space_id: Option<String>,
    pub imported_space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imported_roots: Vec<FeishuReferenceRootSelection>,
    pub imported_root_node_token: Option<String>,
    pub imported_root_node_title: Option<String>,
    pub imported_at: Option<i64>,
    pub imported_doc_count: u32,
    pub managed_path: String,
    pub progress: Option<f32>,
    pub processed_docs: u32,
    pub total_docs: Option<u32>,
    pub current_title: Option<String>,
    pub current_path: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub last_outcome: Option<FeishuReferenceImportLastOutcome>,
}

impl Default for FeishuReferenceImportStatus {
    fn default() -> Self {
        Self {
            state: FeishuReferenceImportStateKind::MissingConfig,
            stage: FeishuReferenceImportStage::Idle,
            running: false,
            auth_mode: FeishuReferenceAuthMode::AppCredentials,
            oauth_persistence_mode: FeishuReferenceOauthPersistenceMode::Session,
            app_id: String::new(),
            app_secret: None,
            app_secret_configured: false,
            authorized: false,
            authorized_user_name: None,
            authorized_user_open_id: None,
            authorized_user_email: None,
            open_base_url: FEISHU_REFERENCE_DEFAULT_OPEN_BASE_URL.to_string(),
            callback_urls: feishu_oauth_callback_urls(),
            required_scopes: feishu_reference_user_auth_scopes(
                FeishuReferenceOauthPersistenceMode::Session,
            ),
            granted_scopes: Vec::new(),
            missing_scopes: feishu_reference_user_auth_scopes(
                FeishuReferenceOauthPersistenceMode::Session,
            ),
            access_token_expires_at: None,
            refresh_token_expires_at: None,
            can_refresh: false,
            space_id: None,
            space_name: None,
            selected_roots: Vec::new(),
            root_node_token: None,
            root_node_title: None,
            imported_space_id: None,
            imported_space_name: None,
            imported_roots: Vec::new(),
            imported_root_node_token: None,
            imported_root_node_title: None,
            imported_at: None,
            imported_doc_count: 0,
            managed_path: FEISHU_REFERENCE_MANAGED_PATH.to_string(),
            progress: None,
            processed_docs: 0,
            total_docs: None,
            current_title: None,
            current_path: None,
            message: "配置飞书应用后可导入知识库文档。".to_string(),
            error: None,
            last_outcome: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeishuReferenceImportRuntime {
    pub working_dir: String,
    pub status: FeishuReferenceImportStatus,
    pub cancel_requested: Arc<AtomicBool>,
    pub oauth_wait_session: Arc<AtomicU64>,
    pub oauth_wait_cancel: Arc<Notify>,
}

impl Default for FeishuReferenceImportRuntime {
    fn default() -> Self {
        Self {
            working_dir: String::new(),
            status: FeishuReferenceImportStatus::default(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            oauth_wait_session: Arc::new(AtomicU64::new(0)),
            oauth_wait_cancel: Arc::new(Notify::new()),
        }
    }
}

#[derive(Clone, Default)]
pub struct FeishuReferenceImportState(pub Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FeishuReferenceConfig {
    #[serde(default)]
    auth_mode: FeishuReferenceAuthMode,
    #[serde(default)]
    oauth_persistence_mode: FeishuReferenceOauthPersistenceMode,
    #[serde(default)]
    app_id: String,
    #[serde(default = "default_feishu_open_base_url")]
    open_base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roots: Vec<FeishuReferenceRootSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FeishuReferenceDirectoryBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roots: Vec<FeishuReferenceRootSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceConfigInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default)]
    pub auth_mode: FeishuReferenceAuthMode,
    #[serde(default)]
    pub oauth_persistence_mode: FeishuReferenceOauthPersistenceMode,
    #[serde(default)]
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub clear_app_secret: bool,
    #[serde(default)]
    pub open_base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<FeishuReferenceRootSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_node_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeishuReferenceImportManifest {
    space_id: String,
    space_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    roots: Vec<FeishuReferenceRootSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_node_title: Option<String>,
    imported_at: i64,
    imported_doc_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceSpaceSummary {
    pub space_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceNodeSummary {
    pub node_token: String,
    pub title: String,
    pub obj_token: String,
    pub obj_type: String,
    pub has_child: bool,
    pub parent_node_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceConnectionTestResult {
    pub summary: String,
    pub open_base_url: String,
    pub space_count: usize,
    pub spaces: Vec<FeishuReferenceSpaceSummary>,
    pub resolved_space_id: Option<String>,
    pub resolved_space_name: Option<String>,
    pub resolved_root_node_token: Option<String>,
    pub resolved_root_node_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceOauthStartResult {
    pub authorize_url: String,
    pub callback_url: String,
    pub callback_urls: Vec<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuReferenceImportRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    pub space_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<FeishuReferenceRootSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_node_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_node_title: Option<String>,
}

#[derive(Debug)]
enum FeishuReferenceImportRunError {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
struct FeishuStoredUserToken {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    refresh_expires_at: i64,
    scope: Option<String>,
    user_name: Option<String>,
    user_open_id: Option<String>,
    user_email: Option<String>,
    client_id: String,
    open_base_url: String,
    redirect_uri: String,
    persistence_mode: FeishuReferenceOauthPersistenceMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeishuStoredUserTokenRecord {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    refresh_expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_open_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_email: Option<String>,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    open_base_url: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    persistence_mode: FeishuReferenceOauthPersistenceMode,
}

#[derive(Debug, Clone)]
struct FeishuPlannedDocument {
    title: String,
    node_token: String,
    obj_token: String,
    relative_path: String,
}

#[derive(Debug, Clone)]
struct FeishuPendingNodeTraversal {
    parent_node_token: Option<String>,
    folder_prefix: String,
}

#[derive(Debug, Clone)]
struct ResolvedFeishuRootSelection {
    selection: FeishuReferenceRootSelection,
    node: FeishuNodeItem,
}

#[derive(Debug, Deserialize)]
struct FeishuTokenEnvelope {
    code: i32,
    msg: String,
    #[serde(default)]
    tenant_access_token: Option<String>,
    #[serde(default)]
    expire: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct FeishuEnvelope<T> {
    code: i32,
    msg: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct FeishuListData<T> {
    #[serde(default)]
    items: Vec<T>,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize, Default)]
struct FeishuSpaceItem {
    #[serde(default)]
    space_id: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct FeishuNodeItem {
    #[serde(default)]
    node_token: String,
    #[serde(default)]
    obj_token: String,
    #[serde(default)]
    obj_type: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    has_child: bool,
    #[serde(default)]
    parent_node_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuNodeGetData {
    node: FeishuNodeItem,
}

#[derive(Debug, Deserialize)]
struct FeishuOauthTokenResponse {
    code: i32,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    refresh_expires_in: i64,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuUserInfoData {
    #[serde(default)]
    name: String,
    #[serde(default)]
    open_id: String,
    #[serde(default)]
    email: String,
}

fn default_feishu_open_base_url() -> String {
    FEISHU_REFERENCE_DEFAULT_OPEN_BASE_URL.to_string()
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn library_dir(working_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(working_dir)
        .join("Library")
        .join("Locus")
}

fn config_path(working_dir: &str) -> std::path::PathBuf {
    library_dir(working_dir).join(FEISHU_REFERENCE_CONFIG_FILE)
}

fn manifest_path(working_dir: &str) -> std::path::PathBuf {
    library_dir(working_dir).join(FEISHU_REFERENCE_MANIFEST_FILE)
}

fn directory_binding_root(working_dir: &str) -> std::path::PathBuf {
    library_dir(working_dir).join(FEISHU_REFERENCE_DIRECTORY_BINDING_ROOT_DIR)
}

fn directory_binding_path(working_dir: &str, target_path: &str) -> std::path::PathBuf {
    let mut path = directory_binding_root(working_dir);
    for segment in target_path
        .trim()
        .trim_matches('/')
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
    {
        path.push(segment);
    }
    path.join(FEISHU_REFERENCE_DIRECTORY_BINDING_FILE)
}

fn knowledge_root(working_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(working_dir)
        .join("Locus")
        .join("knowledge")
}

fn managed_dir_path(working_dir: &str) -> std::path::PathBuf {
    knowledge_root(working_dir)
        .join("reference")
        .join(FEISHU_REFERENCE_MANAGED_DIR)
}

fn managed_directory_config_path(working_dir: &str, suffix: &str) -> std::path::PathBuf {
    knowledge_root(working_dir)
        .join("reference")
        .join(format!("{}{}", FEISHU_REFERENCE_MANAGED_DIR, suffix))
}

fn reference_target_managed_path(target_path: &str) -> String {
    format!("reference/{}", target_path.trim().trim_matches('/'))
}

fn reference_target_dir_path(working_dir: &str, target_path: &str) -> std::path::PathBuf {
    knowledge_root(working_dir)
        .join("reference")
        .join(target_path.trim().trim_matches('/').replace('\\', "/"))
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
    remove_dir_if_exists(&reference_target_dir_path(working_dir, &record.path))?;
    knowledge_store::delete_directory_config_sidecars(
        working_dir,
        KnowledgeType::Reference,
        &record.path,
    )?;
    delete_directory_binding(working_dir, &record.path)?;
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

fn read_feishu_directory_import_snapshot(
    working_dir: &str,
    target_path: &str,
) -> Result<
    (
        crate::knowledge_store::KnowledgeDirectoryConfigRecord,
        Option<String>,
        Vec<FeishuReferenceRootSelection>,
        Option<i64>,
    ),
    String,
> {
    let record = ensure_reference_target_directory(working_dir, target_path)?;
    let sources = record
        .external_sources
        .iter()
        .filter(|source| source.provider == KnowledgeSourceProvider::Feishu)
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return Ok((record, None, Vec::new(), None));
    }

    let mut roots = Vec::new();
    let mut space_id = None;
    let mut imported_at = None;
    for source in sources {
        let parts = parse_locator_parts(source.locator.as_deref());
        if space_id.is_none() {
            space_id = parts.get("space").cloned();
        }
        if imported_at.is_none() {
            imported_at = parts
                .get("importedAt")
                .and_then(|value| value.parse::<i64>().ok());
        }
        if let Some(node_token) = parts.get("node").cloned() {
            roots.push(FeishuReferenceRootSelection {
                node_token,
                node_title: None,
            });
        }
    }

    roots.sort_by(|left, right| left.node_token.cmp(&right.node_token));
    roots.dedup_by(|left, right| left.node_token == right.node_token);
    Ok((record, space_id, roots, imported_at))
}

fn count_reference_markdown_documents(root: &std::path::Path) -> Result<u32, String> {
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

fn temp_root_path(working_dir: &str) -> std::path::PathBuf {
    library_dir(working_dir).join(FEISHU_REFERENCE_TEMP_ROOT_DIR)
}

fn backup_dir_path(working_dir: &str) -> std::path::PathBuf {
    knowledge_root(working_dir)
        .join("reference")
        .join(FEISHU_REFERENCE_BACKUP_DIR)
}

fn normalize_open_base_url(value: &str) -> String {
    let trimmed = value.trim();
    let base = if trimmed.is_empty() {
        FEISHU_REFERENCE_DEFAULT_OPEN_BASE_URL.to_string()
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };
    base.trim_end_matches('/').to_string()
}

fn open_api_url(open_base_url: &str, path: &str) -> String {
    format!(
        "{}/open-apis/{}",
        normalize_open_base_url(open_base_url),
        path.trim_start_matches('/')
    )
}

fn accounts_base_url(open_base_url: &str) -> String {
    let normalized = normalize_open_base_url(open_base_url);
    let Ok(parsed) = Url::parse(&normalized) else {
        return FEISHU_REFERENCE_DEFAULT_ACCOUNTS_BASE_URL.to_string();
    };
    let Some(host) = parsed.host_str() else {
        return FEISHU_REFERENCE_DEFAULT_ACCOUNTS_BASE_URL.to_string();
    };
    let account_host = if let Some(stripped) = host.strip_prefix("open.") {
        format!("accounts.{}", stripped)
    } else if host.starts_with("accounts.") {
        host.to_string()
    } else {
        format!("accounts.{}", host)
    };
    format!("{}://{}", parsed.scheme(), account_host)
}

fn accounts_authorize_url(open_base_url: &str) -> String {
    format!(
        "{}/open-apis/authen/v1/authorize",
        accounts_base_url(open_base_url)
    )
}

fn feishu_oauth_callback_url_for_port(port: u16) -> String {
    format!(
        "http://{}:{}{}",
        FEISHU_REFERENCE_OAUTH_CALLBACK_HOST, port, FEISHU_REFERENCE_OAUTH_CALLBACK_PATH
    )
}

fn feishu_oauth_callback_urls() -> Vec<String> {
    FEISHU_REFERENCE_OAUTH_CALLBACK_PORTS
        .iter()
        .copied()
        .map(feishu_oauth_callback_url_for_port)
        .collect()
}

fn feishu_reference_user_auth_scopes(
    persistence_mode: FeishuReferenceOauthPersistenceMode,
) -> Vec<String> {
    let mut scopes = FEISHU_REFERENCE_USER_AUTH_BASE_SCOPES
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if persistence_mode == FeishuReferenceOauthPersistenceMode::Offline {
        scopes.push(FEISHU_REFERENCE_USER_AUTH_OFFLINE_SCOPE.to_string());
    }
    scopes
}

fn normalize_scope_list<I, S>(items: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized = items
        .into_iter()
        .map(|item| item.as_ref().trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn parse_scope_string(value: Option<&str>) -> Vec<String> {
    normalize_scope_list(
        value
            .unwrap_or_default()
            .split(|ch: char| ch.is_whitespace() || ch == ','),
    )
}

fn compute_missing_scopes(granted_scopes: &[String], required_scopes: &[String]) -> Vec<String> {
    let granted = granted_scopes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    required_scopes
        .iter()
        .filter(|scope| !granted.contains(scope.as_str()))
        .cloned()
        .collect()
}

fn token_can_refresh(token: &FeishuStoredUserToken) -> bool {
    !token.refresh_token.trim().is_empty() && token.refresh_expires_at > now_millis() + 60_000
}

#[derive(Debug, Clone)]
struct FeishuStoredTokenContext {
    granted_scopes: Vec<String>,
    missing_scopes: Vec<String>,
    binding_matches: bool,
}

fn evaluate_stored_user_token(
    token: &FeishuStoredUserToken,
    config: &FeishuReferenceConfig,
) -> FeishuStoredTokenContext {
    let required_scopes = feishu_reference_user_auth_scopes(config.oauth_persistence_mode);
    let granted_scopes = parse_scope_string(token.scope.as_deref());
    let redirect_allowed = feishu_oauth_callback_urls()
        .into_iter()
        .any(|item| item == token.redirect_uri);
    let binding_matches = token.client_id == config.app_id
        && normalize_open_base_url(&token.open_base_url) == config.open_base_url
        && token.persistence_mode == config.oauth_persistence_mode
        && redirect_allowed;
    let missing_scopes = compute_missing_scopes(&granted_scopes, &required_scopes);

    FeishuStoredTokenContext {
        granted_scopes,
        missing_scopes,
        binding_matches,
    }
}

fn oauth_token_is_authorized(
    token: &FeishuStoredUserToken,
    config: &FeishuReferenceConfig,
) -> bool {
    let context = evaluate_stored_user_token(token, config);
    context.binding_matches && context.missing_scopes.is_empty() && oauth_token_is_usable(token)
}

fn apply_oauth_context_to_status(
    status: &mut FeishuReferenceImportStatus,
    config: &FeishuReferenceConfig,
    token: Option<&FeishuStoredUserToken>,
) {
    status.oauth_persistence_mode = config.oauth_persistence_mode;
    status.callback_urls = feishu_oauth_callback_urls();
    status.required_scopes = feishu_reference_user_auth_scopes(config.oauth_persistence_mode);

    if config.auth_mode != FeishuReferenceAuthMode::Oauth {
        status.granted_scopes.clear();
        status.missing_scopes.clear();
        status.access_token_expires_at = None;
        status.refresh_token_expires_at = None;
        status.can_refresh = false;
        return;
    }

    if let Some(token) = token {
        let context = evaluate_stored_user_token(token, config);
        status.granted_scopes = context.granted_scopes;
        status.missing_scopes = context.missing_scopes;
        status.access_token_expires_at = Some(token.expires_at);
        status.refresh_token_expires_at = if token.refresh_token.trim().is_empty() {
            None
        } else {
            Some(token.refresh_expires_at)
        };
        status.can_refresh = token_can_refresh(token);
    } else {
        status.granted_scopes.clear();
        status.missing_scopes = status.required_scopes.clone();
        status.access_token_expires_at = None;
        status.refresh_token_expires_at = None;
        status.can_refresh = false;
    }
}

fn generate_pkce_verifier() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect::<String>()
}

fn pkce_s256(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn workspace_secret_suffix(working_dir: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(working_dir.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

fn feishu_app_secret_key(working_dir: &str) -> String {
    format!(
        "feishu_reference/app_secret/{}",
        workspace_secret_suffix(working_dir)
    )
}

fn feishu_oauth_key(working_dir: &str) -> String {
    format!(
        "feishu_reference/oauth/{}",
        workspace_secret_suffix(working_dir)
    )
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn root_selections_from_legacy_fields(
    node_token: Option<String>,
    node_title: Option<String>,
) -> Vec<FeishuReferenceRootSelection> {
    let Some(node_token) = normalize_optional_text(node_token) else {
        return Vec::new();
    };
    vec![FeishuReferenceRootSelection {
        node_token,
        node_title: normalize_optional_text(node_title),
    }]
}

fn normalize_root_selections(
    roots: Vec<FeishuReferenceRootSelection>,
) -> Vec<FeishuReferenceRootSelection> {
    let mut normalized = Vec::new();
    let mut seen_tokens = HashSet::new();
    for root in roots {
        let Some(node_token) = normalize_optional_text(Some(root.node_token)) else {
            continue;
        };
        if !seen_tokens.insert(node_token.clone()) {
            continue;
        }
        normalized.push(FeishuReferenceRootSelection {
            node_token,
            node_title: normalize_optional_text(root.node_title),
        });
    }
    normalized
}

fn primary_root_fields(roots: &[FeishuReferenceRootSelection]) -> (Option<String>, Option<String>) {
    let Some(first) = roots.first() else {
        return (None, None);
    };
    (Some(first.node_token.clone()), first.node_title.clone())
}

fn normalize_config_roots(config: &mut FeishuReferenceConfig) {
    config.roots = if config.roots.is_empty() {
        root_selections_from_legacy_fields(
            config.root_node_token.clone(),
            config.root_node_title.clone(),
        )
    } else {
        normalize_root_selections(std::mem::take(&mut config.roots))
    };
    (config.root_node_token, config.root_node_title) = primary_root_fields(&config.roots);
}

fn normalize_directory_binding(binding: &mut FeishuReferenceDirectoryBinding) {
    binding.space_id = normalize_optional_text(std::mem::take(&mut binding.space_id));
    binding.space_name = normalize_optional_text(std::mem::take(&mut binding.space_name));
    binding.roots = if binding.roots.is_empty() {
        root_selections_from_legacy_fields(
            binding.root_node_token.clone(),
            binding.root_node_title.clone(),
        )
    } else {
        normalize_root_selections(std::mem::take(&mut binding.roots))
    };
    (binding.root_node_token, binding.root_node_title) = primary_root_fields(&binding.roots);
}

fn directory_binding_has_selection(binding: &FeishuReferenceDirectoryBinding) -> bool {
    binding.space_id.is_some()
}

fn directory_binding_from_selection(
    space_id: Option<String>,
    space_name: Option<String>,
    roots: Vec<FeishuReferenceRootSelection>,
) -> FeishuReferenceDirectoryBinding {
    let mut binding = FeishuReferenceDirectoryBinding {
        space_id,
        space_name,
        roots,
        root_node_token: None,
        root_node_title: None,
    };
    normalize_directory_binding(&mut binding);
    binding
}

fn apply_directory_binding_to_config(
    config: &mut FeishuReferenceConfig,
    binding: Option<&FeishuReferenceDirectoryBinding>,
) {
    if let Some(binding) = binding {
        config.space_id = binding.space_id.clone();
        config.space_name = binding.space_name.clone();
        config.roots = binding.roots.clone();
        config.root_node_token = binding.root_node_token.clone();
        config.root_node_title = binding.root_node_title.clone();
        return;
    }
    config.space_id = None;
    config.space_name = None;
    config.roots.clear();
    config.root_node_token = None;
    config.root_node_title = None;
}

fn normalize_manifest_roots(manifest: &mut FeishuReferenceImportManifest) {
    manifest.roots = if manifest.roots.is_empty() {
        root_selections_from_legacy_fields(
            manifest.root_node_token.clone(),
            manifest.root_node_title.clone(),
        )
    } else {
        normalize_root_selections(std::mem::take(&mut manifest.roots))
    };
    (manifest.root_node_token, manifest.root_node_title) = primary_root_fields(&manifest.roots);
}

fn normalize_request_roots(
    roots: Vec<FeishuReferenceRootSelection>,
    root_node_token: Option<String>,
    root_node_title: Option<String>,
) -> Vec<FeishuReferenceRootSelection> {
    let normalized = normalize_root_selections(roots);
    if normalized.is_empty() {
        return root_selections_from_legacy_fields(root_node_token, root_node_title);
    }
    normalized
}

fn read_config(working_dir: &str) -> Result<FeishuReferenceConfig, String> {
    let path = config_path(working_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let mut config: FeishuReferenceConfig =
                serde_json::from_str(&raw).map_err(|error| {
                    format!(
                        "Failed to parse Feishu reference config '{}': {}",
                        path.display(),
                        error
                    )
                })?;
            config.open_base_url = normalize_open_base_url(&config.open_base_url);
            config.app_id = config.app_id.trim().to_string();
            config.space_id = normalize_optional_text(config.space_id);
            config.space_name = normalize_optional_text(config.space_name);
            normalize_config_roots(&mut config);
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(FeishuReferenceConfig::default())
        }
        Err(error) => Err(format!(
            "Failed to read Feishu reference config '{}': {}",
            path.display(),
            error
        )),
    }
}

fn save_config(working_dir: &str, config: &FeishuReferenceConfig) -> Result<(), String> {
    let path = config_path(working_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Feishu reference config directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }
    let raw = serde_json::to_string_pretty(config)
        .map_err(|error| format!("Failed to serialize Feishu reference config: {}", error))?;
    std::fs::write(&path, raw).map_err(|error| {
        format!(
            "Failed to write Feishu reference config '{}': {}",
            path.display(),
            error
        )
    })
}

fn read_directory_binding(
    working_dir: &str,
    target_path: &str,
) -> Result<Option<FeishuReferenceDirectoryBinding>, String> {
    let path = directory_binding_path(working_dir, target_path);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let mut binding = serde_json::from_str::<FeishuReferenceDirectoryBinding>(&raw)
                .map_err(|error| {
                    format!(
                        "Failed to parse Feishu directory binding '{}': {}",
                        path.display(),
                        error
                    )
                })?;
            normalize_directory_binding(&mut binding);
            if directory_binding_has_selection(&binding) {
                Ok(Some(binding))
            } else {
                Ok(None)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Failed to read Feishu directory binding '{}': {}",
            path.display(),
            error
        )),
    }
}

fn save_directory_binding(
    working_dir: &str,
    target_path: &str,
    binding: &FeishuReferenceDirectoryBinding,
) -> Result<(), String> {
    let path = directory_binding_path(working_dir, target_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Feishu directory binding directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }
    let raw = serde_json::to_string_pretty(binding)
        .map_err(|error| format!("Failed to serialize Feishu directory binding: {}", error))?;
    std::fs::write(&path, raw).map_err(|error| {
        format!(
            "Failed to write Feishu directory binding '{}': {}",
            path.display(),
            error
        )
    })
}

fn delete_directory_binding(working_dir: &str, target_path: &str) -> Result<(), String> {
    let path = directory_binding_path(working_dir, target_path);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete Feishu directory binding '{}': {}",
            path.display(),
            error
        )),
    }
}

fn save_or_delete_directory_binding(
    working_dir: &str,
    target_path: &str,
    binding: &FeishuReferenceDirectoryBinding,
) -> Result<(), String> {
    if directory_binding_has_selection(binding) {
        save_directory_binding(working_dir, target_path, binding)
    } else {
        delete_directory_binding(working_dir, target_path)
    }
}

fn read_manifest(working_dir: &str) -> Result<Option<FeishuReferenceImportManifest>, String> {
    let path = manifest_path(working_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let mut manifest = serde_json::from_str::<FeishuReferenceImportManifest>(&raw)
                .map_err(|error| {
                    format!(
                        "Failed to parse Feishu reference manifest '{}': {}",
                        path.display(),
                        error
                    )
                })?;
            normalize_manifest_roots(&mut manifest);
            Ok(Some(manifest))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Failed to read Feishu reference manifest '{}': {}",
            path.display(),
            error
        )),
    }
}

fn save_manifest(
    working_dir: &str,
    manifest: &FeishuReferenceImportManifest,
) -> Result<(), String> {
    let path = manifest_path(working_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Feishu reference manifest directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }
    let raw = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("Failed to serialize Feishu reference manifest: {}", error))?;
    std::fs::write(&path, raw).map_err(|error| {
        format!(
            "Failed to write Feishu reference manifest '{}': {}",
            path.display(),
            error
        )
    })
}

fn delete_manifest(working_dir: &str) -> Result<(), String> {
    let path = manifest_path(working_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete Feishu reference manifest '{}': {}",
            path.display(),
            error
        )),
    }
}

fn app_secret_configured(working_dir: &str) -> Result<bool, String> {
    Ok(keychain::get_secret(&feishu_app_secret_key(working_dir))?
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false))
}

fn read_app_secret(working_dir: &str) -> Result<Option<String>, String> {
    keychain::get_secret(&feishu_app_secret_key(working_dir))
}

fn write_app_secret(working_dir: &str, secret: &str) -> Result<(), String> {
    keychain::set_secret(&feishu_app_secret_key(working_dir), secret)
}

fn delete_app_secret(working_dir: &str) -> Result<(), String> {
    keychain::delete_secret(&feishu_app_secret_key(working_dir))
}

fn read_stored_user_token(working_dir: &str) -> Result<Option<FeishuStoredUserToken>, String> {
    let Some(raw) = keychain::get_secret(&feishu_oauth_key(working_dir))? else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let record = serde_json::from_str::<FeishuStoredUserTokenRecord>(&raw).map_err(|error| {
        format!(
            "Failed to parse Feishu OAuth token payload from keychain: {}",
            error
        )
    })?;
    Ok(Some(FeishuStoredUserToken {
        access_token: record.access_token,
        refresh_token: record.refresh_token,
        expires_at: record.expires_at,
        refresh_expires_at: record.refresh_expires_at,
        scope: normalize_optional_text(record.scope),
        user_name: normalize_optional_text(record.user_name),
        user_open_id: normalize_optional_text(record.user_open_id),
        user_email: normalize_optional_text(record.user_email),
        client_id: record.client_id.trim().to_string(),
        open_base_url: normalize_open_base_url(&record.open_base_url),
        redirect_uri: record.redirect_uri.trim().to_string(),
        persistence_mode: record.persistence_mode,
    }))
}

fn write_stored_user_token(working_dir: &str, token: &FeishuStoredUserToken) -> Result<(), String> {
    let record = FeishuStoredUserTokenRecord {
        access_token: token.access_token.clone(),
        refresh_token: token.refresh_token.clone(),
        expires_at: token.expires_at,
        refresh_expires_at: token.refresh_expires_at,
        scope: token.scope.clone(),
        user_name: token.user_name.clone(),
        user_open_id: token.user_open_id.clone(),
        user_email: token.user_email.clone(),
        client_id: token.client_id.clone(),
        open_base_url: token.open_base_url.clone(),
        redirect_uri: token.redirect_uri.clone(),
        persistence_mode: token.persistence_mode,
    };
    let raw = serde_json::to_string(&record)
        .map_err(|error| format!("Failed to serialize Feishu OAuth token payload: {}", error))?;
    keychain::set_secret(&feishu_oauth_key(working_dir), &raw)
}

fn delete_stored_user_token(working_dir: &str) -> Result<(), String> {
    keychain::delete_secret(&feishu_oauth_key(working_dir))
}

fn oauth_token_is_usable(token: &FeishuStoredUserToken) -> bool {
    if token.expires_at > now_millis() + 60_000 {
        return true;
    }
    token_can_refresh(token)
}

fn token_has_user_profile(token: &FeishuStoredUserToken) -> bool {
    token.user_name.is_some() || token.user_open_id.is_some() || token.user_email.is_some()
}

fn apply_authorized_user_to_status(
    status: &mut FeishuReferenceImportStatus,
    config: &FeishuReferenceConfig,
    authorized: bool,
    token: Option<&FeishuStoredUserToken>,
) {
    apply_oauth_context_to_status(status, config, token);
    if config.auth_mode != FeishuReferenceAuthMode::Oauth || !authorized {
        status.authorized_user_name = None;
        status.authorized_user_open_id = None;
        status.authorized_user_email = None;
        return;
    }

    status.authorized_user_name = token.and_then(|item| item.user_name.clone());
    status.authorized_user_open_id = token.and_then(|item| item.user_open_id.clone());
    status.authorized_user_email = token.and_then(|item| item.user_email.clone());
}

fn authorization_ready(working_dir: &str, config: &FeishuReferenceConfig) -> Result<bool, String> {
    if config.app_id.trim().is_empty() {
        return Ok(false);
    }
    if !app_secret_configured(working_dir)? {
        return Ok(false);
    }
    match config.auth_mode {
        FeishuReferenceAuthMode::AppCredentials => Ok(true),
        FeishuReferenceAuthMode::Oauth => Ok(read_stored_user_token(working_dir)?
            .map(|token| oauth_token_is_authorized(&token, config))
            .unwrap_or(false)),
    }
}

fn bind_loopback_listener(port: u16) -> Result<Option<TcpListener>, String> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
        .map_err(|error| format!("创建飞书授权回调监听器失败: {}", error))?;
    if let Err(error) = socket.bind(&SockAddr::from(SocketAddr::from((
        Ipv4Addr::LOCALHOST,
        port,
    )))) {
        return match error.kind() {
            ErrorKind::AddrInUse => Ok(None),
            _ => Err(format!("绑定飞书授权回调端口 {} 失败: {}", port, error)),
        };
    }
    socket
        .listen(1)
        .map_err(|error| format!("监听飞书授权回调端口 {} 失败: {}", port, error))?;

    let std_listener: std::net::TcpListener = socket.into();
    std_listener
        .set_nonblocking(true)
        .map_err(|error| format!("配置飞书授权回调端口 {} 为非阻塞失败: {}", port, error))?;
    TcpListener::from_std(std_listener)
        .map(Some)
        .map_err(|error| format!("创建飞书授权回调监听器失败: {}", error))
}

fn bind_feishu_oauth_listener() -> Result<(TcpListener, String), String> {
    for port in FEISHU_REFERENCE_OAUTH_CALLBACK_PORTS.iter().copied() {
        match bind_loopback_listener(port)? {
            Some(listener) => return Ok((listener, feishu_oauth_callback_url_for_port(port))),
            None => continue,
        }
    }

    Err(format!(
        "启动飞书授权回调地址失败。请确认以下地址没有被其他程序占用：{}",
        feishu_oauth_callback_urls().join("、")
    ))
}

fn feishu_client() -> Result<Client, String> {
    crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(45))
            .gzip(true)
            .deflate(true),
    )
    .map_err(|error| format!("Failed to build Feishu HTTP client: {}", error))
}

fn validate_core_config(
    config: &FeishuReferenceConfig,
    missing_secret: bool,
) -> Result<(), String> {
    if config.app_id.trim().is_empty() {
        return Err("请输入飞书 App ID。".to_string());
    }
    if missing_secret {
        return Err("请输入飞书 App Secret 后再继续。".to_string());
    }
    Ok(())
}

fn validate_selection(config: &FeishuReferenceConfig) -> Result<(), String> {
    if config
        .space_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err("请选择要导入的知识空间。".to_string());
    }
    Ok(())
}

fn derive_status_from_snapshot(
    config: &FeishuReferenceConfig,
    secret_configured: bool,
    authorized: bool,
    token: Option<&FeishuStoredUserToken>,
    manifest: Option<&FeishuReferenceImportManifest>,
) -> FeishuReferenceImportStatus {
    let imported_roots = manifest.map(|item| item.roots.clone()).unwrap_or_default();
    let mut status = FeishuReferenceImportStatus {
        auth_mode: config.auth_mode,
        app_id: config.app_id.clone(),
        app_secret_configured: secret_configured,
        authorized,
        open_base_url: config.open_base_url.clone(),
        space_id: config.space_id.clone(),
        space_name: config.space_name.clone(),
        selected_roots: config.roots.clone(),
        root_node_token: config.root_node_token.clone(),
        root_node_title: config.root_node_title.clone(),
        imported_space_id: manifest.map(|item| item.space_id.clone()),
        imported_space_name: manifest.map(|item| item.space_name.clone()),
        imported_roots,
        imported_root_node_token: manifest.and_then(|item| item.root_node_token.clone()),
        imported_root_node_title: manifest.and_then(|item| item.root_node_title.clone()),
        imported_at: manifest.map(|item| item.imported_at),
        imported_doc_count: manifest
            .map(|item| item.imported_doc_count)
            .unwrap_or_default(),
        ..FeishuReferenceImportStatus::default()
    };
    apply_authorized_user_to_status(&mut status, config, authorized, token);

    if config.app_id.trim().is_empty() || !secret_configured {
        status.state = FeishuReferenceImportStateKind::MissingConfig;
        status.message = "配置飞书应用后可导入知识库文档。".to_string();
        return status;
    }

    if config.auth_mode == FeishuReferenceAuthMode::Oauth && !authorized {
        status.state = FeishuReferenceImportStateKind::NeedsAuthorization;
        status.message = "完成用户身份授权后可读取知识空间与文档。".to_string();
        return status;
    }

    status.state = FeishuReferenceImportStateKind::Ready;
    if manifest.is_some() {
        status.stage = FeishuReferenceImportStage::Ready;
        status.message = "飞书知识库文档已导入，可直接检索。".to_string();
    } else if config
        .space_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        status.message = "测试连接后选择知识空间，再开始导入。".to_string();
    } else {
        status.message = "当前配置可用，开始导入后会写入托管 Reference 目录。".to_string();
    }
    status
}

fn derive_persisted_feishu_reference_import_status(
    working_dir: &str,
) -> Result<FeishuReferenceImportStatus, String> {
    let config = read_config(working_dir)?;
    let secret_configured = app_secret_configured(working_dir)?;
    let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
        read_stored_user_token(working_dir)?
    } else {
        None
    };
    let authorized = authorization_ready(working_dir, &config)?;
    let manifest = read_manifest(working_dir)?;
    Ok(derive_status_from_snapshot(
        &config,
        secret_configured,
        authorized,
        stored_user_token.as_ref(),
        manifest.as_ref(),
    ))
}

fn derive_directory_feishu_reference_import_status(
    working_dir: &str,
    target_path: &str,
) -> Result<FeishuReferenceImportStatus, String> {
    let config = read_config(working_dir)?;
    let secret_configured = app_secret_configured(working_dir)?;
    let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
        read_stored_user_token(working_dir)?
    } else {
        None
    };
    let authorized = authorization_ready(working_dir, &config)?;
    let selection = read_directory_binding(working_dir, target_path)?;
    let (_, imported_space_id, imported_roots, imported_at) =
        read_feishu_directory_import_snapshot(working_dir, target_path)?;
    let imported_doc_count =
        count_reference_markdown_documents(&reference_target_dir_path(working_dir, target_path))?;
    let (root_node_token, root_node_title) = selection
        .as_ref()
        .map(|binding| primary_root_fields(&binding.roots))
        .unwrap_or((None, None));
    let (imported_root_node_token, imported_root_node_title) = primary_root_fields(&imported_roots);
    let managed_path = reference_target_managed_path(target_path);

    let mut status = FeishuReferenceImportStatus {
        auth_mode: config.auth_mode,
        oauth_persistence_mode: config.oauth_persistence_mode,
        app_id: config.app_id.clone(),
        app_secret_configured: secret_configured,
        authorized,
        open_base_url: config.open_base_url.clone(),
        space_id: selection
            .as_ref()
            .and_then(|binding| binding.space_id.clone()),
        space_name: selection
            .as_ref()
            .and_then(|binding| binding.space_name.clone()),
        selected_roots: selection
            .as_ref()
            .map(|binding| binding.roots.clone())
            .unwrap_or_default(),
        root_node_token: root_node_token.clone(),
        root_node_title: root_node_title.clone(),
        imported_space_id: (imported_doc_count > 0)
            .then(|| imported_space_id.clone())
            .flatten(),
        imported_space_name: None,
        imported_roots: if imported_doc_count > 0 {
            imported_roots.clone()
        } else {
            Vec::new()
        },
        imported_root_node_token: (imported_doc_count > 0)
            .then_some(imported_root_node_token)
            .flatten(),
        imported_root_node_title: (imported_doc_count > 0)
            .then_some(imported_root_node_title)
            .flatten(),
        imported_at: (imported_doc_count > 0).then_some(imported_at).flatten(),
        imported_doc_count,
        managed_path: managed_path.clone(),
        ..FeishuReferenceImportStatus::default()
    };
    apply_authorized_user_to_status(&mut status, &config, authorized, stored_user_token.as_ref());

    if config.app_id.trim().is_empty() || !secret_configured {
        status.state = FeishuReferenceImportStateKind::MissingConfig;
        status.message = "配置飞书应用后可导入知识库文档。".to_string();
        return Ok(status);
    }

    if config.auth_mode == FeishuReferenceAuthMode::Oauth && !authorized {
        status.state = FeishuReferenceImportStateKind::NeedsAuthorization;
        status.message = "完成用户身份授权后可读取知识空间与文档。".to_string();
        return Ok(status);
    }

    status.state = FeishuReferenceImportStateKind::Ready;
    if imported_doc_count > 0 {
        status.stage = FeishuReferenceImportStage::Ready;
        status.message = "飞书知识库文档已导入，可直接检索。".to_string();
    } else if status
        .space_id
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        status.message = "测试连接后选择知识空间，再开始导入。".to_string();
    } else {
        status.message = "当前配置可用，开始导入后会写入所选 Reference 文件夹。".to_string();
    }
    Ok(status)
}

async fn oauth_wait_session_is_current(
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
    working_dir: &str,
    session: u64,
) -> bool {
    let runtime = state.lock().await;
    runtime.working_dir == working_dir
        && runtime.oauth_wait_session.load(Ordering::Relaxed) == session
}

async fn update_runtime_status<F>(
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
    working_dir: &str,
    mutate: F,
) where
    F: FnOnce(&mut FeishuReferenceImportStatus),
{
    let mut runtime = state.lock().await;
    runtime.working_dir = working_dir.to_string();
    mutate(&mut runtime.status);
}

async fn set_runtime_status(
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
    working_dir: &str,
    status: FeishuReferenceImportStatus,
) {
    let mut runtime = state.lock().await;
    runtime.working_dir = working_dir.to_string();
    runtime.status = status;
}

fn short_token(value: &str) -> String {
    value.chars().take(8).collect::<String>()
}

fn sanitize_segment(title: &str, fallback_prefix: &str, token: &str) -> String {
    let mut sanitized = title
        .trim()
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            _ if ch.is_control() => ' ',
            other => other,
        })
        .collect::<String>();
    sanitized = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('.')
        .trim()
        .to_string();
    if sanitized.is_empty() {
        sanitized = format!("{}-{}", fallback_prefix, short_token(token));
    }
    let upper = sanitized.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
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
        sanitized.push_str(" file");
    }
    if sanitized.len() > 80 {
        sanitized.truncate(80);
        sanitized = sanitized.trim().trim_matches('.').to_string();
    }
    sanitized
}

fn join_relative_path(prefix: &str, segment: &str) -> String {
    if prefix.trim().is_empty() {
        segment.to_string()
    } else {
        format!("{}/{}", prefix.trim_end_matches('/'), segment)
    }
}

fn allocate_unique_markdown_path(
    prefix: &str,
    title: &str,
    token: &str,
    used: &mut HashSet<String>,
) -> String {
    let segment = sanitize_segment(title, "doc", token);
    let base = join_relative_path(prefix, &format!("{}.md", segment));
    if used.insert(base.clone()) {
        return base;
    }
    let mut index = 2u32;
    loop {
        let candidate = join_relative_path(prefix, &format!("{}-{}.md", segment, index));
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

fn allocate_unique_directory_path(
    prefix: &str,
    title: &str,
    token: &str,
    used: &mut HashSet<String>,
) -> String {
    let segment = sanitize_segment(title, "node", token);
    let base = join_relative_path(prefix, &segment);
    if used.insert(base.clone()) {
        return base;
    }
    let mut index = 2u32;
    loop {
        let candidate = join_relative_path(prefix, &format!("{}-{}", segment, index));
        if used.insert(candidate.clone()) {
            return candidate;
        }
        index += 1;
    }
}

async fn parse_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
    label: &str,
) -> Result<T, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("{} 响应读取失败: {}", label, error))?;
    if !status.is_success() {
        return Err(format!(
            "{} 请求失败 (HTTP {}): {}",
            label,
            status.as_u16(),
            body
        ));
    }
    serde_json::from_str::<T>(&body)
        .map_err(|error| format!("{} 响应解析失败: {} | {}", label, error, body))
}

async fn fetch_tenant_access_token(
    client: &Client,
    config: &FeishuReferenceConfig,
    app_secret: &str,
) -> Result<String, String> {
    let response = client
        .post(open_api_url(
            &config.open_base_url,
            "auth/v3/tenant_access_token/internal",
        ))
        .json(&serde_json::json!({
            "app_id": config.app_id,
            "app_secret": app_secret,
        }))
        .send()
        .await
        .map_err(|error| format!("获取 tenant_access_token 失败: {}", error))?;
    let payload: FeishuTokenEnvelope =
        parse_json_response(response, "获取 tenant_access_token").await?;
    if payload.code != 0 {
        return Err(format!(
            "获取 tenant_access_token 失败: {} ({})",
            payload.msg, payload.code
        ));
    }
    payload
        .tenant_access_token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "tenant_access_token 响应缺少有效 token。".to_string())
}

async fn parse_oauth_token_response(
    response: reqwest::Response,
    label: &str,
) -> Result<FeishuOauthTokenResponse, String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("{} 响应读取失败: {}", label, error))?;
    let payload = serde_json::from_str::<FeishuOauthTokenResponse>(&body)
        .map_err(|error| format!("{} 响应解析失败: {} | {}", label, error, body))?;
    if status.is_success() && payload.code == 0 {
        return Ok(payload);
    }

    let detail = payload
        .error_description
        .clone()
        .or(payload.msg.clone())
        .or(payload.error.clone())
        .unwrap_or(body);
    Err(format!(
        "{}失败 (HTTP {}): {}",
        label,
        status.as_u16(),
        detail
    ))
}

async fn fetch_user_info(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
) -> Result<FeishuUserInfoData, String> {
    let response = client
        .get(open_api_url(&config.open_base_url, "authen/v1/user_info"))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|error| format!("获取飞书授权用户信息失败: {}", error))?;
    let payload: FeishuEnvelope<FeishuUserInfoData> =
        parse_json_response(response, "获取飞书授权用户信息").await?;
    if payload.code != 0 {
        return Err(format!(
            "获取飞书授权用户信息失败: {} ({})",
            payload.msg, payload.code
        ));
    }
    payload
        .data
        .ok_or_else(|| "飞书授权用户信息响应缺少 data。".to_string())
}

async fn populate_user_profile(
    client: &Client,
    config: &FeishuReferenceConfig,
    token: &mut FeishuStoredUserToken,
) -> Result<(), String> {
    if token_has_user_profile(token) {
        return Ok(());
    }
    let profile = fetch_user_info(client, config, &token.access_token).await?;
    token.user_name = normalize_optional_text(Some(profile.name));
    token.user_open_id = normalize_optional_text(Some(profile.open_id));
    token.user_email = normalize_optional_text(Some(profile.email));
    Ok(())
}

async fn exchange_authorization_code(
    client: &Client,
    config: &FeishuReferenceConfig,
    app_secret: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<FeishuStoredUserToken, String> {
    let response = client
        .post(open_api_url(&config.open_base_url, "authen/v2/oauth/token"))
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": config.app_id,
            "client_secret": app_secret,
            "code": code,
            "redirect_uri": redirect_uri,
            "code_verifier": code_verifier,
        }))
        .send()
        .await
        .map_err(|error| format!("换取 user_access_token 失败: {}", error))?;
    let payload = parse_oauth_token_response(response, "换取 user_access_token").await?;
    if payload.access_token.trim().is_empty() {
        return Err("user_access_token 响应缺少 access_token。".to_string());
    }
    let now = now_millis();
    let mut token = FeishuStoredUserToken {
        access_token: payload.access_token,
        refresh_token: payload.refresh_token.trim().to_string(),
        expires_at: now + payload.expires_in.max(1) * 1000,
        refresh_expires_at: if payload.refresh_token.trim().is_empty() {
            0
        } else {
            now + payload.refresh_expires_in.max(1) * 1000
        },
        scope: normalize_optional_text(payload.scope),
        user_name: None,
        user_open_id: None,
        user_email: None,
        client_id: config.app_id.clone(),
        open_base_url: config.open_base_url.clone(),
        redirect_uri: redirect_uri.to_string(),
        persistence_mode: config.oauth_persistence_mode,
    };
    populate_user_profile(client, config, &mut token).await?;
    Ok(token)
}

async fn refresh_user_access_token(
    client: &Client,
    config: &FeishuReferenceConfig,
    app_secret: &str,
    refresh_token: &str,
    previous_token: &FeishuStoredUserToken,
) -> Result<FeishuStoredUserToken, String> {
    let response = client
        .post(open_api_url(&config.open_base_url, "authen/v2/oauth/token"))
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": config.app_id,
            "client_secret": app_secret,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|error| format!("刷新 user_access_token 失败: {}", error))?;
    let payload = parse_oauth_token_response(response, "刷新 user_access_token").await?;
    if payload.access_token.trim().is_empty() {
        return Err("刷新 user_access_token 响应缺少 access_token。".to_string());
    }
    let now = now_millis();
    let mut token = FeishuStoredUserToken {
        access_token: payload.access_token,
        refresh_token: payload.refresh_token.trim().to_string(),
        expires_at: now + payload.expires_in.max(1) * 1000,
        refresh_expires_at: if payload.refresh_token.trim().is_empty() {
            0
        } else {
            now + payload.refresh_expires_in.max(1) * 1000
        },
        scope: normalize_optional_text(payload.scope).or_else(|| previous_token.scope.clone()),
        user_name: previous_token.user_name.clone(),
        user_open_id: previous_token.user_open_id.clone(),
        user_email: previous_token.user_email.clone(),
        client_id: previous_token.client_id.clone(),
        open_base_url: previous_token.open_base_url.clone(),
        redirect_uri: previous_token.redirect_uri.clone(),
        persistence_mode: previous_token.persistence_mode,
    };
    populate_user_profile(client, config, &mut token).await?;
    Ok(token)
}

async fn resolve_access_token(
    working_dir: &str,
    config: &FeishuReferenceConfig,
    client: &Client,
) -> Result<String, String> {
    let app_secret = read_app_secret(working_dir)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "请输入飞书 App Secret 后再继续。".to_string())?;

    match config.auth_mode {
        FeishuReferenceAuthMode::AppCredentials => {
            fetch_tenant_access_token(client, config, &app_secret).await
        }
        FeishuReferenceAuthMode::Oauth => {
            let mut stored = read_stored_user_token(working_dir)?
                .ok_or_else(|| "当前鉴权模式需要先完成飞书授权。".to_string())?;
            let context = evaluate_stored_user_token(&stored, config);
            if !context.binding_matches {
                delete_stored_user_token(working_dir)?;
                return Err("当前用户授权与现有飞书应用配置不一致，请重新授权。".to_string());
            }
            if !context.missing_scopes.is_empty() {
                delete_stored_user_token(working_dir)?;
                return Err(format!(
                    "当前用户授权缺少必要权限，请重新授权：{}",
                    context.missing_scopes.join("、")
                ));
            }
            if stored.expires_at > now_millis() + 60_000 {
                if !token_has_user_profile(&stored) {
                    populate_user_profile(client, config, &mut stored).await?;
                    write_stored_user_token(working_dir, &stored)?;
                }
                return Ok(stored.access_token);
            }
            if !token_can_refresh(&stored) {
                delete_stored_user_token(working_dir)?;
                return Err("飞书用户身份授权已过期，请重新授权。".to_string());
            }
            let refreshed = refresh_user_access_token(
                client,
                config,
                &app_secret,
                &stored.refresh_token,
                &stored,
            )
            .await?;
            write_stored_user_token(working_dir, &refreshed)?;
            Ok(refreshed.access_token)
        }
    }
}

async fn fetch_all_spaces(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
) -> Result<Vec<FeishuReferenceSpaceSummary>, String> {
    let mut page_token: Option<String> = None;
    let mut spaces = Vec::new();
    loop {
        let mut request = client
            .get(open_api_url(&config.open_base_url, "wiki/v2/spaces"))
            .header("Authorization", format!("Bearer {}", access_token))
            .query(&[("page_size", "50")]);
        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("page_token", token)]);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("获取知识空间列表失败: {}", error))?;
        let payload: FeishuEnvelope<FeishuListData<FeishuSpaceItem>> =
            parse_json_response(response, "获取知识空间列表").await?;
        if payload.code != 0 {
            return Err(format!(
                "获取知识空间列表失败: {} ({})",
                payload.msg, payload.code
            ));
        }
        let data = payload
            .data
            .ok_or_else(|| "知识空间列表响应缺少 data。".to_string())?;
        spaces.extend(
            data.items
                .into_iter()
                .filter(|item| !item.space_id.trim().is_empty())
                .map(|item| FeishuReferenceSpaceSummary {
                    space_id: item.space_id,
                    name: if item.name.trim().is_empty() {
                        "未命名知识空间".to_string()
                    } else {
                        item.name
                    },
                }),
        );
        if !data.has_more {
            break;
        }
        page_token = data.page_token;
        if page_token.as_deref().unwrap_or_default().trim().is_empty() {
            break;
        }
    }
    spaces.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(spaces)
}

async fn fetch_space_nodes_page(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    space_id: &str,
    parent_node_token: Option<&str>,
    page_token: Option<&str>,
) -> Result<FeishuListData<FeishuNodeItem>, String> {
    let mut request = client
        .get(open_api_url(
            &config.open_base_url,
            &format!("wiki/v2/spaces/{}/nodes", space_id),
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&[("page_size", "50")]);
    if let Some(token) = parent_node_token.filter(|value| !value.trim().is_empty()) {
        request = request.query(&[("parent_node_token", token)]);
    }
    if let Some(token) = page_token.filter(|value| !value.trim().is_empty()) {
        request = request.query(&[("page_token", token)]);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("获取知识空间节点列表失败: {}", error))?;
    let payload: FeishuEnvelope<FeishuListData<FeishuNodeItem>> =
        parse_json_response(response, "获取知识空间节点列表").await?;
    if payload.code != 0 {
        return Err(format!(
            "获取知识空间节点列表失败: {} ({})",
            payload.msg, payload.code
        ));
    }
    payload
        .data
        .ok_or_else(|| "知识空间节点列表响应缺少 data。".to_string())
}

async fn fetch_all_space_nodes(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    space_id: &str,
    parent_node_token: Option<&str>,
) -> Result<Vec<FeishuNodeItem>, String> {
    let mut page_token: Option<String> = None;
    let mut items = Vec::new();
    loop {
        let data = fetch_space_nodes_page(
            client,
            config,
            access_token,
            space_id,
            parent_node_token,
            page_token.as_deref(),
        )
        .await?;
        items.extend(
            data.items
                .into_iter()
                .filter(|item| !item.node_token.trim().is_empty()),
        );
        if !data.has_more {
            break;
        }
        page_token = data.page_token;
        if page_token.as_deref().unwrap_or_default().trim().is_empty() {
            break;
        }
    }
    Ok(items)
}

async fn fetch_node_detail(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    node_token: &str,
) -> Result<FeishuNodeItem, String> {
    let response = client
        .get(open_api_url(
            &config.open_base_url,
            "wiki/v2/spaces/get_node",
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&[("token", node_token)])
        .send()
        .await
        .map_err(|error| format!("获取知识空间节点详情失败: {}", error))?;
    let payload: FeishuEnvelope<FeishuNodeGetData> =
        parse_json_response(response, "获取知识空间节点详情").await?;
    if payload.code != 0 {
        return Err(format!(
            "获取知识空间节点详情失败: {} ({})",
            payload.msg, payload.code
        ));
    }
    payload
        .data
        .map(|data| data.node)
        .ok_or_else(|| "知识空间节点详情响应缺少 data.node。".to_string())
}

async fn fetch_docx_raw_content(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    document_id: &str,
) -> Result<String, String> {
    let response = client
        .get(open_api_url(
            &config.open_base_url,
            &format!("docx/v1/documents/{}/raw_content", document_id),
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&[("lang", "0")])
        .send()
        .await
        .map_err(|error| format!("获取飞书文档正文失败: {}", error))?;
    let payload: FeishuEnvelope<serde_json::Value> =
        parse_json_response(response, "获取飞书文档正文").await?;
    if payload.code != 0 {
        return Err(format!(
            "获取飞书文档正文失败: {} ({})",
            payload.msg, payload.code
        ));
    }
    payload
        .data
        .and_then(|data| {
            data.get("content")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .ok_or_else(|| "飞书文档正文响应缺少 content。".to_string())
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FeishuDocxBlocksPage {
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Default)]
struct FeishuMarkdownRenderContext {
    list_indent: usize,
    quote_depth: usize,
}

impl FeishuMarkdownRenderContext {
    fn nested_list(self) -> Self {
        Self {
            list_indent: self.list_indent + 1,
            ..self
        }
    }

    fn nested_quote(self) -> Self {
        Self {
            quote_depth: self.quote_depth + 1,
            ..self
        }
    }
}

async fn fetch_docx_blocks(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    document_id: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut page_token: Option<String> = None;
    let mut items = Vec::new();

    loop {
        let mut request = client
            .get(open_api_url(
                &config.open_base_url,
                &format!("docx/v1/documents/{}/blocks", document_id),
            ))
            .header("Authorization", format!("Bearer {}", access_token))
            .query(&[("page_size", "500")]);
        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("page_token", token)]);
        }

        let response = request
            .send()
            .await
            .map_err(|error| format!("获取飞书文档块失败: {}", error))?;
        let payload: FeishuEnvelope<FeishuDocxBlocksPage> =
            parse_json_response(response, "获取飞书文档块").await?;
        if payload.code != 0 {
            return Err(format!(
                "获取飞书文档块失败: {} ({})",
                payload.msg, payload.code
            ));
        }
        let data = payload
            .data
            .ok_or_else(|| "飞书文档块响应缺少 data。".to_string())?;
        items.extend(data.items);

        if !data.has_more {
            break;
        }

        page_token = data.page_token.filter(|value| !value.trim().is_empty());
        if page_token.is_none() {
            return Err("飞书文档块响应缺少下一页 page_token。".to_string());
        }
        tokio::time::sleep(Duration::from_millis(
            FEISHU_REFERENCE_RAW_CONTENT_INTERVAL_MS,
        ))
        .await;
    }

    Ok(items)
}

async fn fetch_docx_markdown_content(
    client: &Client,
    config: &FeishuReferenceConfig,
    access_token: &str,
    document_id: &str,
) -> Result<String, String> {
    match fetch_docx_blocks(client, config, access_token, document_id).await {
        Ok(items) => {
            if let Ok(markdown) = render_docx_blocks_markdown(items) {
                if !markdown.trim().is_empty() {
                    return Ok(markdown);
                }
            }
        }
        Err(_) => {}
    }

    Ok(normalize_markdown_body(
        &fetch_docx_raw_content(client, config, access_token, document_id).await?,
    ))
}

fn supported_obj_type(value: &str) -> bool {
    value.eq_ignore_ascii_case("docx")
}

fn stable_document_id(space_id: &str, node_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(space_id.as_bytes());
    hasher.update(b":");
    hasher.update(node_token.as_bytes());
    format!(
        "feishu-{}",
        hasher
            .finalize()
            .iter()
            .take(16)
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>()
    )
}

fn normalize_markdown_body(content: &str) -> String {
    let trimmed_content = content.trim_matches('\n');
    if trimmed_content.is_empty() {
        String::new()
    } else {
        format!("{trimmed_content}\n")
    }
}

fn feishu_block_id(block: &serde_json::Value) -> Option<&str> {
    block.get("block_id").and_then(|value| value.as_str())
}

fn feishu_block_parent_id(block: &serde_json::Value) -> Option<&str> {
    block
        .get("parent_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
}

fn feishu_block_children(block: &serde_json::Value) -> Vec<String> {
    block
        .get("children")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn feishu_block_kind(block: &serde_json::Value) -> Option<&'static str> {
    [
        "page",
        "text",
        "heading1",
        "heading2",
        "heading3",
        "heading4",
        "heading5",
        "heading6",
        "heading7",
        "heading8",
        "heading9",
        "bullet",
        "ordered",
        "code",
        "quote",
        "todo",
        "bitable",
        "callout",
        "chat_card",
        "diagram",
        "divider",
        "file",
        "grid",
        "grid_column",
        "iframe",
        "image",
        "isv",
        "mindnote",
        "sheet",
        "table",
        "table_cell",
        "view",
        "undefined",
        "quote_container",
        "task",
        "okr",
        "okr_objective",
        "okr_key_result",
        "okr_progress",
    ]
    .into_iter()
    .find(|key| block.get(*key).is_some())
}

fn string_field<'a>(value: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
    value
        .and_then(|item| item.get(key))
        .and_then(|item| item.as_str())
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = bytes[index + 1];
            let lo = bytes[index + 2];
            let hex = [hi, lo];
            if let Some(value) = std::str::from_utf8(&hex)
                .ok()
                .and_then(|item| u8::from_str_radix(item, 16).ok())
            {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn escape_markdown_text(content: &str) -> String {
    let mut escaped = String::with_capacity(content.len());
    for ch in content.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn escape_markdown_link_text(content: &str) -> String {
    content.replace('[', "\\[").replace(']', "\\]")
}

fn wrap_text_style(content: String, style: Option<&serde_json::Value>) -> String {
    let Some(style) = style else {
        return content;
    };
    if content.is_empty() {
        return content;
    }

    let mut rendered = content;
    let has_inline_code = style
        .get("inline_code")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if has_inline_code {
        return format!("`{}`", rendered.replace('`', "\\`"));
    }

    if style
        .get("underline")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        rendered = format!("<u>{}</u>", rendered);
    }
    if style
        .get("strikethrough")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        rendered = format!("~~{}~~", rendered);
    }
    if style
        .get("italic")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        rendered = format!("*{}*", rendered);
    }
    if style
        .get("bold")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        rendered = format!("**{}**", rendered);
    }

    let link_url = style
        .get("link")
        .and_then(|value| value.get("url"))
        .and_then(|value| value.as_str())
        .map(percent_decode)
        .filter(|value| !value.trim().is_empty());
    if let Some(url) = link_url {
        rendered = format!("[{}](<{}>)", escape_markdown_link_text(&rendered), url);
    }

    rendered
}

fn render_docx_inline_element(element: &serde_json::Value) -> String {
    if let Some(text_run) = element.get("text_run") {
        let content = string_field(Some(text_run), "content")
            .map(escape_markdown_text)
            .unwrap_or_default();
        return wrap_text_style(content, text_run.get("text_element_style"));
    }
    if let Some(mention_user) = element.get("mention_user") {
        let content = string_field(Some(mention_user), "user_id")
            .map(|value| format!("@{}", escape_markdown_text(value)))
            .unwrap_or_else(|| "@用户".to_string());
        return wrap_text_style(content, mention_user.get("text_element_style"));
    }
    if let Some(mention_doc) = element.get("mention_doc") {
        let label = "@文档".to_string();
        let styled = wrap_text_style(label, mention_doc.get("text_element_style"));
        let url = string_field(Some(mention_doc), "url")
            .map(percent_decode)
            .filter(|value| !value.trim().is_empty());
        return url
            .map(|item| format!("[{}](<{}>)", escape_markdown_link_text(&styled), item))
            .unwrap_or(styled);
    }
    if let Some(reminder) = element.get("reminder") {
        return wrap_text_style("提醒".to_string(), reminder.get("text_element_style"));
    }
    if let Some(file) = element.get("file") {
        return wrap_text_style("附件".to_string(), file.get("text_element_style"));
    }
    if let Some(inline_block) = element.get("inline_block") {
        return wrap_text_style("块".to_string(), inline_block.get("text_element_style"));
    }
    if let Some(equation) = element.get("equation") {
        let content = string_field(Some(equation), "content")
            .map(|value| format!("${}$", value))
            .unwrap_or_else(|| "$$".to_string());
        return wrap_text_style(content, equation.get("text_element_style"));
    }
    String::new()
}

fn render_docx_text_content(data: Option<&serde_json::Value>) -> String {
    data.and_then(|value| value.get("elements"))
        .and_then(|value| value.as_array())
        .map(|elements| {
            elements
                .iter()
                .map(render_docx_inline_element)
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn render_docx_plain_text_content(data: Option<&serde_json::Value>) -> String {
    data.and_then(|value| value.get("elements"))
        .and_then(|value| value.as_array())
        .map(|elements| {
            elements
                .iter()
                .map(|element| {
                    if let Some(text_run) = element.get("text_run") {
                        string_field(Some(text_run), "content")
                            .unwrap_or_default()
                            .to_string()
                    } else if let Some(mention_user) = element.get("mention_user") {
                        string_field(Some(mention_user), "user_id")
                            .map(|value| format!("@{}", value))
                            .unwrap_or_else(|| "@用户".to_string())
                    } else if let Some(mention_doc) = element.get("mention_doc") {
                        string_field(Some(mention_doc), "url")
                            .map(percent_decode)
                            .unwrap_or_else(|| "@文档".to_string())
                    } else if element.get("reminder").is_some() {
                        "提醒".to_string()
                    } else if element.get("file").is_some() {
                        "附件".to_string()
                    } else if element.get("inline_block").is_some() {
                        "块".to_string()
                    } else if let Some(equation) = element.get("equation") {
                        string_field(Some(equation), "content")
                            .unwrap_or_default()
                            .to_string()
                    } else {
                        String::new()
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn prefix_markdown_block(content: &str, context: FeishuMarkdownRenderContext) -> String {
    let indent = "  ".repeat(context.list_indent);
    let quote_prefix = "> ".repeat(context.quote_depth);
    let line_prefix = format!("{}{}", indent, quote_prefix);
    if line_prefix.is_empty() {
        return content.to_string();
    }

    let quote_blank_prefix = if context.quote_depth > 0 {
        format!("{}{}", indent, quote_prefix.trim_end())
    } else {
        String::new()
    };

    content
        .lines()
        .map(|line| {
            if line.is_empty() {
                quote_blank_prefix.clone()
            } else {
                format!("{}{}", line_prefix, line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_list_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("bullet" | "ordered" | "todo"))
}

fn render_docx_blocks_children(
    child_ids: &[String],
    blocks: &HashMap<String, serde_json::Value>,
    context: FeishuMarkdownRenderContext,
) -> String {
    let mut rendered = String::new();
    let mut previous_kind: Option<&str> = None;

    for child_id in child_ids {
        let Some(block) = blocks.get(child_id) else {
            continue;
        };
        let kind = feishu_block_kind(block);
        let block_markdown = render_docx_block(child_id, blocks, context);
        if block_markdown.trim().is_empty() {
            continue;
        }
        if !rendered.is_empty() {
            if is_list_kind(previous_kind) && is_list_kind(kind) {
                rendered.push('\n');
            } else {
                rendered.push_str("\n\n");
            }
        }
        rendered.push_str(block_markdown.trim_end());
        previous_kind = kind;
    }

    rendered
}

fn render_docx_list_item(
    marker: &str,
    text: &str,
    children: &[String],
    blocks: &HashMap<String, serde_json::Value>,
    context: FeishuMarkdownRenderContext,
) -> String {
    let indent = "  ".repeat(context.list_indent);
    let quote_prefix = "> ".repeat(context.quote_depth);
    let base_prefix = format!("{}{}", indent, quote_prefix);
    let continuation_prefix = format!("{}{}", base_prefix, " ".repeat(marker.len()));
    let mut lines = text.lines();
    let first_line = lines.next().unwrap_or_default();
    let mut rendered = if first_line.is_empty() {
        format!("{}{}", base_prefix, marker)
    } else {
        format!("{}{}{}", base_prefix, marker, first_line)
    };
    for line in lines {
        rendered.push('\n');
        if line.is_empty() {
            rendered.push_str(&continuation_prefix);
        } else {
            rendered.push_str(&continuation_prefix);
            rendered.push_str(line);
        }
    }

    let child_markdown = render_docx_blocks_children(children, blocks, context.nested_list());
    if !child_markdown.trim().is_empty() {
        rendered.push('\n');
        rendered.push_str(child_markdown.trim_end());
    }

    rendered
}

fn render_docx_table(
    block: &serde_json::Value,
    blocks: &HashMap<String, serde_json::Value>,
    context: FeishuMarkdownRenderContext,
) -> String {
    let Some(table) = block.get("table") else {
        return String::new();
    };
    let row_size = table
        .get("property")
        .and_then(|value| value.get("row_size"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let column_size = table
        .get("property")
        .and_then(|value| value.get("column_size"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let cell_ids = table
        .get("cells")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if row_size == 0 || column_size == 0 || cell_ids.is_empty() {
        return String::new();
    }

    let mut rows = Vec::new();
    for row_index in 0..row_size {
        let mut row = Vec::new();
        for column_index in 0..column_size {
            let cell_index = row_index * column_size + column_index;
            let cell_content = cell_ids
                .get(cell_index)
                .and_then(|cell_id| blocks.get(*cell_id))
                .map(|cell_block| {
                    let content = render_docx_blocks_children(
                        &feishu_block_children(cell_block),
                        blocks,
                        FeishuMarkdownRenderContext::default(),
                    );
                    content
                        .trim()
                        .replace("\n\n", "<br>")
                        .replace('\n', "<br>")
                        .replace('|', "\\|")
                })
                .unwrap_or_default();
            row.push(cell_content);
        }
        rows.push(row);
    }

    if rows.is_empty() {
        return String::new();
    }

    let header = rows.remove(0);
    let separator = std::iter::repeat("---")
        .take(column_size)
        .collect::<Vec<_>>()
        .join(" | ");
    let mut lines = vec![
        format!("| {} |", header.join(" | ")),
        format!("| {} |", separator),
    ];
    for row in rows {
        lines.push(format!("| {} |", row.join(" | ")));
    }
    prefix_markdown_block(&lines.join("\n"), context)
}

fn render_docx_block(
    block_id: &str,
    blocks: &HashMap<String, serde_json::Value>,
    context: FeishuMarkdownRenderContext,
) -> String {
    let Some(block) = blocks.get(block_id) else {
        return String::new();
    };
    let children = feishu_block_children(block);
    match feishu_block_kind(block) {
        Some("page") => render_docx_blocks_children(&children, blocks, context),
        Some("text") => {
            let text = render_docx_text_content(block.get("text"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&text, context)
            }
        }
        Some("heading1") => {
            let text = render_docx_text_content(block.get("heading1"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("# {}", text), context)
            }
        }
        Some("heading2") => {
            let text = render_docx_text_content(block.get("heading2"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("## {}", text), context)
            }
        }
        Some("heading3") => {
            let text = render_docx_text_content(block.get("heading3"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("### {}", text), context)
            }
        }
        Some("heading4") => {
            let text = render_docx_text_content(block.get("heading4"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("#### {}", text), context)
            }
        }
        Some("heading5") => {
            let text = render_docx_text_content(block.get("heading5"));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("##### {}", text), context)
            }
        }
        Some("heading6" | "heading7" | "heading8" | "heading9") => {
            let text =
                render_docx_text_content(block.get(feishu_block_kind(block).unwrap_or_default()));
            if text.trim().is_empty() {
                String::new()
            } else {
                prefix_markdown_block(&format!("###### {}", text), context)
            }
        }
        Some("bullet") => render_docx_list_item(
            "- ",
            &render_docx_text_content(block.get("bullet")),
            &children,
            blocks,
            context,
        ),
        Some("ordered") => render_docx_list_item(
            "1. ",
            &render_docx_text_content(block.get("ordered")),
            &children,
            blocks,
            context,
        ),
        Some("todo") => {
            let marker = if block
                .get("todo")
                .and_then(|value| value.get("style"))
                .and_then(|value| value.get("done"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                "- [x] "
            } else {
                "- [ ] "
            };
            render_docx_list_item(
                marker,
                &render_docx_text_content(block.get("todo")),
                &children,
                blocks,
                context,
            )
        }
        Some("code") => {
            let code = render_docx_plain_text_content(block.get("code"));
            if code.trim().is_empty() {
                String::new()
            } else {
                let language = block
                    .get("code")
                    .and_then(|value| value.get("style"))
                    .and_then(|value| value.get("language"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                prefix_markdown_block(
                    &format!("```{}\n{}\n```", language, code.trim_end_matches('\n')),
                    context,
                )
            }
        }
        Some("quote") => {
            let inner = context.nested_quote();
            let mut parts = Vec::new();
            let text = render_docx_text_content(block.get("quote"));
            if !text.trim().is_empty() {
                parts.push(prefix_markdown_block(&text, inner));
            }
            let child_markdown = render_docx_blocks_children(&children, blocks, inner);
            if !child_markdown.trim().is_empty() {
                parts.push(child_markdown);
            }
            parts.join("\n\n")
        }
        Some("quote_container" | "callout") => {
            render_docx_blocks_children(&children, blocks, context.nested_quote())
        }
        Some("divider") => prefix_markdown_block("---", context),
        Some("table") => render_docx_table(block, blocks, context),
        Some("table_cell" | "grid" | "grid_column" | "view") => {
            render_docx_blocks_children(&children, blocks, context)
        }
        Some("image") => prefix_markdown_block("图片", context),
        Some("file") => {
            let name = block
                .get("file")
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
                .unwrap_or("附件");
            prefix_markdown_block(&format!("附件：{}", escape_markdown_text(name)), context)
        }
        Some("sheet") => prefix_markdown_block("电子表格", context),
        Some("bitable") => prefix_markdown_block("多维表格", context),
        Some("diagram") => prefix_markdown_block("流程图", context),
        Some("mindnote") => prefix_markdown_block("思维笔记", context),
        Some("iframe") => prefix_markdown_block("内嵌内容", context),
        Some("task") => prefix_markdown_block("任务", context),
        Some("okr" | "okr_objective" | "okr_key_result" | "okr_progress") => {
            render_docx_blocks_children(&children, blocks, context)
        }
        _ => render_docx_blocks_children(&children, blocks, context),
    }
}

fn render_docx_blocks_markdown(items: Vec<serde_json::Value>) -> Result<String, String> {
    let mut blocks = HashMap::new();
    for item in items {
        let Some(block_id) = feishu_block_id(&item).map(str::to_string) else {
            continue;
        };
        blocks.insert(block_id, item);
    }
    if blocks.is_empty() {
        return Ok(String::new());
    }

    let mut root_ids = blocks
        .values()
        .filter(|block| {
            feishu_block_kind(block) == Some("page") || feishu_block_parent_id(block).is_none()
        })
        .filter_map(feishu_block_id)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if root_ids.is_empty() {
        if let Some(first) = blocks.keys().next() {
            root_ids.push(first.clone());
        }
    }

    root_ids.sort();
    root_ids.dedup();

    let mut rendered_roots = Vec::new();
    for root_id in root_ids {
        let rendered = render_docx_block(&root_id, &blocks, FeishuMarkdownRenderContext::default());
        if !rendered.trim().is_empty() {
            rendered_roots.push(rendered);
        }
    }

    Ok(normalize_markdown_body(&rendered_roots.join("\n\n")))
}

fn is_cancelled(cancel_requested: &Arc<AtomicBool>) -> bool {
    cancel_requested.load(Ordering::Relaxed)
}

fn ensure_not_cancelled(
    cancel_requested: &Arc<AtomicBool>,
) -> Result<(), FeishuReferenceImportRunError> {
    if is_cancelled(cancel_requested) {
        return Err(FeishuReferenceImportRunError::Cancelled);
    }
    Ok(())
}

fn push_planned_document(
    planned: &mut Vec<FeishuPlannedDocument>,
    planned_node_tokens: &mut HashSet<String>,
    used_doc_paths: &mut HashSet<String>,
    target_path: &str,
    base_prefix: &str,
    node: &FeishuNodeItem,
) {
    if !supported_obj_type(&node.obj_type) || !planned_node_tokens.insert(node.node_token.clone()) {
        return;
    }
    let relative =
        allocate_unique_markdown_path(base_prefix, &node.title, &node.node_token, used_doc_paths);
    planned.push(FeishuPlannedDocument {
        title: if node.title.trim().is_empty() {
            "未命名文档".to_string()
        } else {
            node.title.clone()
        },
        node_token: node.node_token.clone(),
        obj_token: node.obj_token.clone(),
        relative_path: join_relative_path(target_path, &relative),
    });
}

fn queue_node_traversal(
    stack: &mut Vec<FeishuPendingNodeTraversal>,
    used_dir_paths: &mut HashSet<String>,
    base_prefix: &str,
    node: &FeishuNodeItem,
) {
    if !node.has_child {
        return;
    }
    let child_prefix =
        allocate_unique_directory_path(base_prefix, &node.title, &node.node_token, used_dir_paths);
    stack.push(FeishuPendingNodeTraversal {
        parent_node_token: Some(node.node_token.clone()),
        folder_prefix: child_prefix,
    });
}

fn directory_summary(space_name: &str, roots: &[FeishuReferenceRootSelection]) -> String {
    if roots.is_empty() {
        return format!("飞书知识库导入结果，来源于 {}。", space_name);
    }
    if roots.len() == 1 {
        let root_label = roots[0]
            .node_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(roots[0].node_token.as_str());
        return format!(
            "飞书知识库导入结果，来源于 {} / {}。",
            space_name, root_label
        );
    }
    format!(
        "飞书知识库导入结果，来源于 {} 下选择的 {} 个目录。",
        space_name,
        roots.len()
    )
}

fn configure_managed_directory(
    working_dir: &str,
    target_path: &str,
    space_name: &str,
    roots: &[FeishuReferenceRootSelection],
) -> Result<(), String> {
    let mut config = knowledge_store::default_directory_config_for_type(KnowledgeType::Reference);
    config.summary = directory_summary(space_name, roots);
    config.inject_mode = KnowledgeInjectMode::None;
    config.inherit_inject_mode = false;
    config.ai_maintained = false;
    config.inherit_ai_config = false;
    config.allow_create_documents = false;
    config.allow_create_directories = false;
    config.allow_move_documents = false;
    config.allow_move_directories = false;
    knowledge_store::update_directory_config(
        working_dir,
        KnowledgeType::Reference,
        target_path,
        config,
    )
    .map(|_| ())
}

fn build_directory_external_sources(
    space_id: &str,
    roots: &[FeishuReferenceRootSelection],
    imported_at: Option<i64>,
) -> Vec<KnowledgeExternalSource> {
    let imported_at_segment = imported_at
        .map(|value| format!(";importedAt:{}", value))
        .unwrap_or_default();
    if roots.is_empty() {
        return vec![KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::Feishu,
            locator: Some(format!("space:{}{}", space_id, imported_at_segment)),
            source_id: Some(space_id.to_string()),
            sync_enabled: true,
        }];
    }

    roots
        .iter()
        .map(|root| KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::Feishu,
            locator: Some(format!(
                "space:{};node:{}{}",
                space_id, root.node_token, imported_at_segment
            )),
            source_id: Some(root.node_token.clone()),
            sync_enabled: true,
        })
        .collect()
}

async fn collect_planned_documents(
    working_dir: &str,
    target_path: &str,
    config: &FeishuReferenceConfig,
    access_token: &str,
    client: &Client,
    space_prefix: &str,
    roots: &[ResolvedFeishuRootSelection],
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
    cancel_requested: &Arc<AtomicBool>,
) -> Result<Vec<FeishuPlannedDocument>, FeishuReferenceImportRunError> {
    let mut planned = Vec::new();
    let mut planned_node_tokens = HashSet::new();
    let mut used_doc_paths = HashSet::new();
    let mut used_dir_paths = HashSet::new();
    let mut visited_listing_tokens = HashSet::new();
    let mut stack = Vec::new();

    if roots.is_empty() {
        stack.push(FeishuPendingNodeTraversal {
            parent_node_token: None,
            folder_prefix: space_prefix.to_string(),
        });
    } else {
        for root in roots {
            push_planned_document(
                &mut planned,
                &mut planned_node_tokens,
                &mut used_doc_paths,
                target_path,
                space_prefix,
                &root.node,
            );
            queue_node_traversal(&mut stack, &mut used_dir_paths, space_prefix, &root.node);
        }
    }

    while let Some(pending) = stack.pop() {
        ensure_not_cancelled(cancel_requested)?;
        let traversal_key = pending
            .parent_node_token
            .clone()
            .unwrap_or_else(|| "__space_root__".to_string());
        if !visited_listing_tokens.insert(traversal_key) {
            continue;
        }
        update_runtime_status(state.clone(), working_dir, |status| {
            status.stage = FeishuReferenceImportStage::ListingNodes;
            status.running = true;
            status.state = FeishuReferenceImportStateKind::Running;
            status.current_path = Some(join_relative_path(target_path, &pending.folder_prefix));
            status.message = "正在遍历飞书知识空间节点。".to_string();
            status.processed_docs = planned.len() as u32;
            status.total_docs = None;
        })
        .await;

        let items = fetch_all_space_nodes(
            client,
            config,
            access_token,
            config.space_id.as_deref().unwrap_or_default(),
            pending.parent_node_token.as_deref(),
        )
        .await
        .map_err(FeishuReferenceImportRunError::Failed)?;

        for item in items.into_iter().rev() {
            ensure_not_cancelled(cancel_requested)?;
            push_planned_document(
                &mut planned,
                &mut planned_node_tokens,
                &mut used_doc_paths,
                target_path,
                &pending.folder_prefix,
                &item,
            );
            queue_node_traversal(
                &mut stack,
                &mut used_dir_paths,
                &pending.folder_prefix,
                &item,
            );
        }
    }

    Ok(planned)
}

fn remove_dir_if_exists(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove directory '{}': {}",
            path.display(),
            error
        )),
    }
}

fn remove_file_if_exists(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove file '{}': {}",
            path.display(),
            error
        )),
    }
}

fn prepare_temp_root(working_dir: &str) -> Result<std::path::PathBuf, String> {
    let root = temp_root_path(working_dir);
    remove_dir_if_exists(&root)?;
    std::fs::create_dir_all(&root).map_err(|error| {
        format!(
            "Failed to create Feishu temporary import root '{}': {}",
            root.display(),
            error
        )
    })?;
    Ok(root)
}

fn swap_managed_directory(
    working_dir: &str,
    target_path: &str,
    temp_root: &std::path::Path,
) -> Result<(), String> {
    let incoming = temp_root.join("reference").join(target_path);
    if !incoming.is_dir() {
        return Err(format!(
            "Feishu temporary managed directory is missing: {}",
            incoming.display()
        ));
    }

    let managed = reference_target_dir_path(working_dir, target_path);
    let backup = backup_dir_path(working_dir);
    remove_dir_if_exists(&backup)?;
    if let Some(parent) = managed.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Feishu managed directory parent '{}': {}",
                parent.display(),
                error
            )
        })?;
    }
    if managed.exists() {
        std::fs::rename(&managed, &backup).map_err(|error| {
            format!(
                "Failed to move existing Feishu managed directory '{}' to '{}': {}",
                managed.display(),
                backup.display(),
                error
            )
        })?;
    }
    if let Err(error) = std::fs::rename(&incoming, &managed) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &managed);
        }
        return Err(format!(
            "Failed to activate imported Feishu managed directory '{}': {}",
            managed.display(),
            error
        ));
    }
    remove_dir_if_exists(&backup)?;
    Ok(())
}

fn parse_request_line_path(request: &str) -> Option<String> {
    let mut lines = request.lines();
    let request_line = lines.next()?.trim();
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    Some(path.to_string())
}

async fn respond_loopback_html(socket: &mut tokio::net::TcpStream, title: &str, message: &str) {
    let escaped_title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body style=\"font-family:Segoe UI, sans-serif;padding:32px;background:#f6f7f9;color:#1f2328\"><h2 style=\"margin:0 0 12px\">{}</h2><p style=\"margin:0;line-height:1.6\">{}</p><p style=\"margin-top:18px;color:#6a737d\">可以返回 Locus 继续操作。</p></body></html>",
        escaped_title, escaped_title, escaped_message
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

async fn run_feishu_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    config: FeishuReferenceConfig,
    target_path: Option<String>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    cancel_requested: Arc<AtomicBool>,
) -> Result<FeishuReferenceImportStatus, FeishuReferenceImportRunError> {
    let mut config = config;
    let use_workspace_config = target_path.is_none();
    let target_path = target_path.unwrap_or_else(|| FEISHU_REFERENCE_MANAGED_DIR.to_string());
    let managed_path = reference_target_managed_path(&target_path);
    ensure_not_cancelled(&cancel_requested)?;
    let client = feishu_client().map_err(FeishuReferenceImportRunError::Failed)?;
    let access_token = resolve_access_token(&working_dir, &config, &client)
        .await
        .map_err(FeishuReferenceImportRunError::Failed)?;
    ensure_not_cancelled(&cancel_requested)?;

    let spaces = fetch_all_spaces(&client, &config, &access_token)
        .await
        .map_err(FeishuReferenceImportRunError::Failed)?;
    let resolved_space_name = config
        .space_name
        .clone()
        .or_else(|| {
            spaces
                .iter()
                .find(|item| item.space_id == config.space_id.as_deref().unwrap_or_default())
                .map(|item| item.name.clone())
        })
        .unwrap_or_else(|| format!("space-{}", config.space_id.as_deref().unwrap_or("unknown")));
    let space_prefix = sanitize_segment(
        &resolved_space_name,
        "space",
        config.space_id.as_deref().unwrap_or_default(),
    );
    let mut resolved_roots = Vec::new();
    for root in &config.roots {
        let node = fetch_node_detail(&client, &config, &access_token, &root.node_token)
            .await
            .map_err(FeishuReferenceImportRunError::Failed)?;
        let node_title = normalize_optional_text(root.node_title.clone())
            .or_else(|| normalize_optional_text(Some(node.title.clone())));
        resolved_roots.push(ResolvedFeishuRootSelection {
            selection: FeishuReferenceRootSelection {
                node_token: root.node_token.clone(),
                node_title,
            },
            node,
        });
    }
    let resolved_selected_roots = resolved_roots
        .iter()
        .map(|root| root.selection.clone())
        .collect::<Vec<_>>();
    config.space_name = Some(resolved_space_name.clone());
    config.roots = resolved_selected_roots.clone();
    (config.root_node_token, config.root_node_title) = primary_root_fields(&config.roots);
    if use_workspace_config {
        save_config(&working_dir, &config).map_err(FeishuReferenceImportRunError::Failed)?;
    } else {
        let binding = directory_binding_from_selection(
            config.space_id.clone(),
            config.space_name.clone(),
            config.roots.clone(),
        );
        save_or_delete_directory_binding(&working_dir, &target_path, &binding)
            .map_err(FeishuReferenceImportRunError::Failed)?;
    }

    let planned = collect_planned_documents(
        &working_dir,
        &target_path,
        &config,
        &access_token,
        &client,
        &space_prefix,
        &resolved_roots,
        state.clone(),
        &cancel_requested,
    )
    .await?;

    if planned.is_empty() {
        return Err(FeishuReferenceImportRunError::Failed(
            "当前选择范围内没有可导入的 docx 文档。".to_string(),
        ));
    }

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = FeishuReferenceImportStage::Importing;
        status.progress = Some(0.0);
        status.total_docs = Some(planned.len() as u32);
        status.processed_docs = 0;
        status.message = "正在抓取飞书文档正文并写入托管目录。".to_string();
    })
    .await;

    let temp_root =
        prepare_temp_root(&working_dir).map_err(FeishuReferenceImportRunError::Failed)?;
    for (index, planned_doc) in planned.iter().enumerate() {
        ensure_not_cancelled(&cancel_requested)?;
        update_runtime_status(state.clone(), &working_dir, |status| {
            status.stage = FeishuReferenceImportStage::Importing;
            status.current_title = Some(planned_doc.title.clone());
            status.current_path = Some(planned_doc.relative_path.clone());
            status.processed_docs = index as u32;
            status.total_docs = Some(planned.len() as u32);
            status.progress = Some(index as f32 / planned.len() as f32);
            status.message = format!("正在导入 {}", planned_doc.title);
        })
        .await;

        let content =
            fetch_docx_markdown_content(&client, &config, &access_token, &planned_doc.obj_token)
                .await
                .map_err(FeishuReferenceImportRunError::Failed)?;
        let target_path = knowledge_store::document_path_in_root(
            &temp_root,
            KnowledgeType::Reference,
            &planned_doc.relative_path,
        )
        .map_err(FeishuReferenceImportRunError::Failed)?;
        let document = KnowledgeDocument {
            id: stable_document_id(
                config.space_id.as_deref().unwrap_or_default(),
                &planned_doc.node_token,
            ),
            doc_type: KnowledgeType::Reference,
            path: planned_doc.relative_path.clone(),
            title: planned_doc.title.clone(),
            inject_mode: KnowledgeInjectMode::None,
            inherit_inject_mode: false,
            inject_mode_source: KnowledgeConfigSource {
                kind: KnowledgeConfigSourceKind::SelfValue,
                path: None,
            },
            summary_enabled: false,
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
                provider: KnowledgeSourceProvider::Feishu,
                locator: Some(format!(
                    "space:{};node:{};obj:{}",
                    config.space_id.as_deref().unwrap_or_default(),
                    planned_doc.node_token,
                    planned_doc.obj_token
                )),
                source_id: Some(planned_doc.node_token.clone()),
                sync_enabled: true,
            }),
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            tools: Vec::new(),
            summary: None,
            body: content,
            maintenance_rules: None,
            created_at: now_millis(),
            updated_at: now_millis(),
        };
        knowledge_store::save_document_to_path(&target_path, document)
            .map_err(FeishuReferenceImportRunError::Failed)?;

        if index + 1 < planned.len() {
            tokio::time::sleep(Duration::from_millis(
                FEISHU_REFERENCE_RAW_CONTENT_INTERVAL_MS,
            ))
            .await;
        }
    }

    ensure_not_cancelled(&cancel_requested)?;
    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = FeishuReferenceImportStage::Reconciling;
        status.progress = Some(1.0);
        status.processed_docs = planned.len() as u32;
        status.total_docs = Some(planned.len() as u32);
        status.current_title = None;
        status.current_path = Some(managed_path.clone());
        status.message = "正在切换托管目录并刷新知识索引。".to_string();
    })
    .await;

    swap_managed_directory(&working_dir, &target_path, &temp_root)
        .map_err(FeishuReferenceImportRunError::Failed)?;
    remove_dir_if_exists(&temp_root).map_err(FeishuReferenceImportRunError::Failed)?;

    configure_managed_directory(
        &working_dir,
        &target_path,
        &resolved_space_name,
        &resolved_selected_roots,
    )
    .map_err(FeishuReferenceImportRunError::Failed)?;
    let imported_at = now_millis();
    knowledge_store::update_directory_external_sources(
        &working_dir,
        KnowledgeType::Reference,
        &target_path,
        build_directory_external_sources(
            config.space_id.as_deref().unwrap_or_default(),
            &resolved_selected_roots,
            Some(imported_at),
        ),
    )
    .map_err(FeishuReferenceImportRunError::Failed)?;

    if target_path == FEISHU_REFERENCE_MANAGED_DIR {
        let mut manifest = FeishuReferenceImportManifest {
            space_id: config.space_id.clone().unwrap_or_default(),
            space_name: resolved_space_name.clone(),
            roots: resolved_selected_roots.clone(),
            root_node_token: None,
            root_node_title: None,
            imported_at,
            imported_doc_count: planned.len() as u32,
        };
        (manifest.root_node_token, manifest.root_node_title) = primary_root_fields(&manifest.roots);
        save_manifest(&working_dir, &manifest).map_err(FeishuReferenceImportRunError::Failed)?;
    }
    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_import_feishu_reference_docs",
    )
    .await
    .map_err(|error| FeishuReferenceImportRunError::Failed(error.message))?;

    let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
        read_stored_user_token(&working_dir).map_err(FeishuReferenceImportRunError::Failed)?
    } else {
        None
    };
    let mut status = FeishuReferenceImportStatus {
        state: FeishuReferenceImportStateKind::Ready,
        stage: FeishuReferenceImportStage::Ready,
        running: false,
        auth_mode: config.auth_mode,
        oauth_persistence_mode: config.oauth_persistence_mode,
        app_id: config.app_id.clone(),
        app_secret: None,
        app_secret_configured: true,
        authorized: authorization_ready(&working_dir, &config)
            .map_err(FeishuReferenceImportRunError::Failed)?,
        authorized_user_name: None,
        authorized_user_open_id: None,
        authorized_user_email: None,
        open_base_url: config.open_base_url.clone(),
        callback_urls: feishu_oauth_callback_urls(),
        required_scopes: feishu_reference_user_auth_scopes(config.oauth_persistence_mode),
        granted_scopes: Vec::new(),
        missing_scopes: Vec::new(),
        access_token_expires_at: None,
        refresh_token_expires_at: None,
        can_refresh: false,
        space_id: config.space_id.clone(),
        space_name: Some(resolved_space_name.clone()),
        selected_roots: resolved_selected_roots.clone(),
        root_node_token: config.root_node_token.clone(),
        root_node_title: config.root_node_title.clone(),
        imported_space_id: config.space_id.clone(),
        imported_space_name: Some(resolved_space_name.clone()),
        imported_roots: resolved_selected_roots.clone(),
        imported_root_node_token: config.root_node_token.clone(),
        imported_root_node_title: config.root_node_title.clone(),
        imported_at: Some(imported_at),
        imported_doc_count: planned.len() as u32,
        managed_path: managed_path.clone(),
        progress: Some(1.0),
        processed_docs: planned.len() as u32,
        total_docs: Some(planned.len() as u32),
        current_title: None,
        current_path: Some(managed_path),
        message: "飞书知识库文档导入完成。".to_string(),
        error: None,
        last_outcome: None,
    };
    let status_authorized = status.authorized;
    apply_authorized_user_to_status(
        &mut status,
        &config,
        status_authorized,
        stored_user_token.as_ref(),
    );
    Ok(status)
}

pub async fn get_feishu_reference_import_status(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    if let Some(target_path) = target_path.map(str::trim).filter(|value| !value.is_empty()) {
        let runtime = state.lock().await.clone();
        let requested_managed_path = reference_target_managed_path(target_path);
        let selection = read_directory_binding(working_dir, target_path)?;
        if runtime.working_dir == working_dir
            && runtime.status.managed_path == requested_managed_path
            && (runtime.status.running
                || runtime.status.stage == FeishuReferenceImportStage::Authorizing
                || runtime.status.stage == FeishuReferenceImportStage::Error)
        {
            let mut status = runtime.status;
            let config = read_config(working_dir)?;
            let app_secret = read_app_secret(working_dir)?;
            let secret_configured = app_secret_configured(working_dir)?;
            let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
                read_stored_user_token(working_dir)?
            } else {
                None
            };
            let authorized = authorization_ready(working_dir, &config)?;
            let (_, imported_space_id, imported_roots, imported_at) =
                read_feishu_directory_import_snapshot(working_dir, target_path)?;
            let (root_node_token, root_node_title) = selection
                .as_ref()
                .map(|binding| primary_root_fields(&binding.roots))
                .unwrap_or((None, None));
            let (imported_root_node_token, imported_root_node_title) =
                primary_root_fields(&imported_roots);
            let imported_doc_count = count_reference_markdown_documents(
                &reference_target_dir_path(working_dir, target_path),
            )?;

            status.auth_mode = config.auth_mode;
            status.oauth_persistence_mode = config.oauth_persistence_mode;
            status.app_id = config.app_id.clone();
            status.app_secret = app_secret;
            status.app_secret_configured = secret_configured;
            status.authorized = authorized;
            apply_authorized_user_to_status(
                &mut status,
                &config,
                authorized,
                stored_user_token.as_ref(),
            );
            status.open_base_url = config.open_base_url;
            status.space_id = selection
                .as_ref()
                .and_then(|binding| binding.space_id.clone());
            status.space_name = selection
                .as_ref()
                .and_then(|binding| binding.space_name.clone());
            status.selected_roots = selection
                .as_ref()
                .map(|binding| binding.roots.clone())
                .unwrap_or_default();
            status.root_node_token = root_node_token.clone();
            status.root_node_title = root_node_title.clone();
            status.imported_space_id = (imported_doc_count > 0)
                .then(|| imported_space_id)
                .flatten();
            status.imported_space_name = None;
            status.imported_roots = if imported_doc_count > 0 {
                imported_roots
            } else {
                Vec::new()
            };
            status.imported_root_node_token = if imported_doc_count > 0 {
                imported_root_node_token
            } else {
                None
            };
            status.imported_root_node_title = if imported_doc_count > 0 {
                imported_root_node_title
            } else {
                None
            };
            status.imported_at = if imported_doc_count > 0 {
                imported_at
            } else {
                None
            };
            status.imported_doc_count = imported_doc_count;
            return Ok(status);
        }

        let mut status = derive_directory_feishu_reference_import_status(working_dir, target_path)?;
        status.app_secret = read_app_secret(working_dir)?;
        if runtime.working_dir == working_dir
            && runtime.status.managed_path == requested_managed_path
            && runtime.status.last_outcome == Some(FeishuReferenceImportLastOutcome::Cancelled)
        {
            status.last_outcome = Some(FeishuReferenceImportLastOutcome::Cancelled);
            status.message = "已取消飞书知识库文档导入。".to_string();
        }
        return Ok(status);
    }

    let config = read_config(working_dir)?;
    let app_secret = read_app_secret(working_dir)?;
    let secret_configured = app_secret_configured(working_dir)?;
    let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
        read_stored_user_token(working_dir)?
    } else {
        None
    };
    let authorized = authorization_ready(working_dir, &config)?;
    let manifest = read_manifest(working_dir)?;
    let runtime = state.lock().await.clone();

    if runtime.working_dir == working_dir
        && (runtime.status.running
            || runtime.status.stage == FeishuReferenceImportStage::Authorizing
            || runtime.status.stage == FeishuReferenceImportStage::Error)
    {
        let mut status = runtime.status;
        status.auth_mode = config.auth_mode;
        status.app_id = config.app_id.clone();
        status.app_secret = app_secret.clone();
        status.app_secret_configured = secret_configured;
        status.authorized = authorized;
        apply_authorized_user_to_status(
            &mut status,
            &config,
            authorized,
            stored_user_token.as_ref(),
        );
        status.open_base_url = config.open_base_url;
        status.space_id = config.space_id;
        status.space_name = config.space_name;
        status.selected_roots = config.roots.clone();
        status.root_node_token = config.root_node_token;
        status.root_node_title = config.root_node_title;
        if let Some(manifest) = manifest.as_ref() {
            status.imported_space_id = Some(manifest.space_id.clone());
            status.imported_space_name = Some(manifest.space_name.clone());
            status.imported_roots = manifest.roots.clone();
            status.imported_root_node_token = manifest.root_node_token.clone();
            status.imported_root_node_title = manifest.root_node_title.clone();
            status.imported_at = Some(manifest.imported_at);
            status.imported_doc_count = manifest.imported_doc_count;
        }
        return Ok(status);
    }

    let mut status = derive_status_from_snapshot(
        &config,
        secret_configured,
        authorized,
        stored_user_token.as_ref(),
        manifest.as_ref(),
    );
    status.app_secret = app_secret;
    if runtime.working_dir == working_dir
        && runtime.status.last_outcome == Some(FeishuReferenceImportLastOutcome::Cancelled)
    {
        status.last_outcome = Some(FeishuReferenceImportLastOutcome::Cancelled);
        status.message = "已取消飞书知识库文档导入。".to_string();
    }
    Ok(status)
}

pub async fn save_feishu_reference_config(
    working_dir: &str,
    input: FeishuReferenceConfigInput,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    let previous = read_config(working_dir)?;
    let target_path = normalize_optional_text(input.target_path.clone())
        .map(|path| ensure_reference_target_directory(working_dir, &path).map(|record| record.path))
        .transpose()?;
    let requested_space_id = normalize_optional_text(input.space_id.clone());
    let requested_space_name = normalize_optional_text(input.space_name.clone());
    let requested_roots = normalize_request_roots(
        input.roots.clone(),
        input.root_node_token.clone(),
        input.root_node_title.clone(),
    );
    let directory_binding = directory_binding_from_selection(
        requested_space_id.clone(),
        requested_space_name.clone(),
        requested_roots.clone(),
    );
    let mut next_config = FeishuReferenceConfig {
        auth_mode: input.auth_mode,
        oauth_persistence_mode: input.oauth_persistence_mode,
        app_id: input.app_id.trim().to_string(),
        open_base_url: normalize_open_base_url(&input.open_base_url),
        space_id: if target_path.is_some() {
            previous.space_id.clone()
        } else {
            requested_space_id.clone()
        },
        space_name: if target_path.is_some() {
            previous.space_name.clone()
        } else {
            requested_space_name.clone()
        },
        roots: if target_path.is_some() {
            previous.roots.clone()
        } else {
            requested_roots.clone()
        },
        root_node_token: None,
        root_node_title: None,
    };
    normalize_config_roots(&mut next_config);

    if input.clear_app_secret {
        delete_app_secret(working_dir)?;
    } else if let Some(secret) = input
        .app_secret
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        write_app_secret(working_dir, &secret)?;
    }

    if previous.app_id != next_config.app_id
        || previous.open_base_url != next_config.open_base_url
        || previous.oauth_persistence_mode != next_config.oauth_persistence_mode
    {
        delete_stored_user_token(working_dir)?;
    }

    save_config(working_dir, &next_config)?;
    if let Some(target_path) = target_path.as_deref() {
        save_or_delete_directory_binding(working_dir, target_path, &directory_binding)?;
    }
    let status =
        get_feishu_reference_import_status(working_dir, target_path.as_deref(), state.clone())
            .await?;
    if !status.running {
        set_runtime_status(state, working_dir, status.clone()).await;
    }
    Ok(status)
}

pub async fn test_feishu_reference_connection(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceConnectionTestResult, String> {
    let target_path = target_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            ensure_reference_target_directory(working_dir, value).map(|record| record.path)
        })
        .transpose()?;
    let mut config = read_config(working_dir)?;
    if let Some(target_path) = target_path.as_deref() {
        let binding = read_directory_binding(working_dir, target_path)?;
        apply_directory_binding_to_config(&mut config, binding.as_ref());
    }
    validate_core_config(&config, !app_secret_configured(working_dir)?)?;
    let client = feishu_client()?;

    update_runtime_status(state.clone(), working_dir, |status| {
        status.stage = FeishuReferenceImportStage::TestingConnection;
        status.running = false;
        status.state = FeishuReferenceImportStateKind::Ready;
        status.error = None;
        status.message = "正在验证飞书连接并读取知识空间列表。".to_string();
    })
    .await;

    let access_token = resolve_access_token(working_dir, &config, &client).await?;
    let spaces = fetch_all_spaces(&client, &config, &access_token).await?;
    let resolved_space = config.space_id.as_deref().and_then(|space_id| {
        spaces
            .iter()
            .find(|item| item.space_id == space_id)
            .cloned()
    });
    let resolved_root_title = if let Some(token) = config.root_node_token.as_deref() {
        Some(
            fetch_node_detail(&client, &config, &access_token, token)
                .await?
                .title,
        )
    } else {
        None
    };

    if let Some(space) = resolved_space.as_ref() {
        if let Some(target_path) = target_path.as_deref() {
            let mut updated = directory_binding_from_selection(
                config.space_id.clone(),
                Some(space.name.clone()),
                config.roots.clone(),
            );
            if let Some(root) = updated.roots.first_mut() {
                if resolved_root_title.is_some() {
                    root.node_title = resolved_root_title.clone();
                }
            } else if resolved_root_title.is_some() {
                updated.root_node_title = resolved_root_title.clone();
            }
            normalize_directory_binding(&mut updated);
            save_or_delete_directory_binding(working_dir, target_path, &updated)?;
        } else {
            let mut updated = config.clone();
            updated.space_name = Some(space.name.clone());
            if let Some(root) = updated.roots.first_mut() {
                if resolved_root_title.is_some() {
                    root.node_title = resolved_root_title.clone();
                }
            } else if resolved_root_title.is_some() {
                updated.root_node_title = resolved_root_title.clone();
            }
            normalize_config_roots(&mut updated);
            save_config(working_dir, &updated)?;
        }
    }

    Ok(FeishuReferenceConnectionTestResult {
        summary: if spaces.is_empty() {
            "连接成功，当前鉴权范围内没有可访问的知识空间。".to_string()
        } else {
            format!("连接成功，共读取到 {} 个知识空间。", spaces.len())
        },
        open_base_url: config.open_base_url.clone(),
        space_count: spaces.len(),
        spaces,
        resolved_space_id: resolved_space.as_ref().map(|item| item.space_id.clone()),
        resolved_space_name: resolved_space.as_ref().map(|item| item.name.clone()),
        resolved_root_node_token: config.root_node_token.clone(),
        resolved_root_node_title: resolved_root_title.or(config.root_node_title.clone()),
    })
}

pub async fn start_feishu_reference_oauth(
    working_dir: String,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceOauthStartResult, String> {
    let config = read_config(&working_dir)?;
    validate_core_config(&config, !app_secret_configured(&working_dir)?)?;

    let callback_urls = feishu_oauth_callback_urls();
    let (listener, callback_url) = bind_feishu_oauth_listener()?;
    let oauth_state = Uuid::new_v4().to_string();
    let code_verifier = generate_pkce_verifier();
    let code_challenge = pkce_s256(&code_verifier);
    let requested_scopes = feishu_reference_user_auth_scopes(config.oauth_persistence_mode);
    let (oauth_wait_session, oauth_wait_cancel) = {
        let mut runtime = state.lock().await;
        if runtime.working_dir == working_dir
            && runtime.status.stage == FeishuReferenceImportStage::Authorizing
        {
            return Err("当前正在等待飞书授权回调。".to_string());
        }
        runtime.working_dir = working_dir.clone();
        (
            runtime.oauth_wait_session.fetch_add(1, Ordering::Relaxed) + 1,
            runtime.oauth_wait_cancel.clone(),
        )
    };

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.running = false;
        status.stage = FeishuReferenceImportStage::Authorizing;
        status.state = FeishuReferenceImportStateKind::NeedsAuthorization;
        status.error = None;
        status.last_outcome = None;
        status.message = "用户身份授权已启动，等待飞书回调。".to_string();
    })
    .await;

    let mut authorize_url = Url::parse(&accounts_authorize_url(&config.open_base_url))
        .map_err(|error| format!("构造飞书授权地址失败: {}", error))?;
    authorize_url
        .query_pairs_mut()
        .append_pair("client_id", &config.app_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &callback_url)
        .append_pair("scope", &requested_scopes.join(" "))
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    let authorize_url_string = authorize_url.to_string();
    let oauth_state_for_task = oauth_state.clone();
    let working_dir_for_task = working_dir.clone();
    let state_for_task = state.clone();
    let oauth_wait_cancel_for_task = oauth_wait_cancel.clone();
    let callback_url_for_task = callback_url.clone();
    let code_verifier_for_task = code_verifier.clone();
    tauri::async_runtime::spawn(async move {
        enum FeishuOauthWaitOutcome {
            Authorized,
            Cancelled,
            Failed(String),
        }

        let result = tokio::time::timeout(
            Duration::from_secs(FEISHU_REFERENCE_OAUTH_WAIT_SECS),
            async {
                tokio::select! {
                    _ = oauth_wait_cancel_for_task.notified() => {
                        FeishuOauthWaitOutcome::Cancelled
                    }
                    outcome = async {
                        let (mut socket, _) = listener
                            .accept()
                            .await
                            .map_err(|error| format!("等待飞书授权回调失败: {}", error))?;
                        let mut buffer = vec![0u8; 8192];
                        let size = socket
                            .read(&mut buffer)
                            .await
                            .map_err(|error| format!("读取飞书授权回调失败: {}", error))?;
                        let request = String::from_utf8_lossy(&buffer[..size]).to_string();
                        let path = parse_request_line_path(&request)
                            .ok_or_else(|| "飞书授权回调请求无效。".to_string())?;
                        let callback_request_url = Url::parse(&format!("http://127.0.0.1{}", path))
                            .map_err(|error| format!("解析飞书授权回调失败: {}", error))?;
                        if callback_request_url.path() != FEISHU_REFERENCE_OAUTH_CALLBACK_PATH {
                            respond_loopback_html(
                                &mut socket,
                                "授权失败",
                                "飞书授权回调路径无效，请回到 Locus 重新发起授权。",
                            )
                            .await;
                            return Err("飞书授权回调路径无效，请重新发起授权。".to_string());
                        }
                        let state_value = callback_request_url
                            .query_pairs()
                            .find(|(key, _)| key == "state")
                            .map(|(_, value)| value.to_string())
                            .unwrap_or_default();
                        let callback_error = callback_request_url
                            .query_pairs()
                            .find(|(key, _)| key == "error")
                            .map(|(_, value)| value.to_string())
                            .unwrap_or_default();
                        let callback_error_description = callback_request_url
                            .query_pairs()
                            .find(|(key, _)| key == "error_description")
                            .map(|(_, value)| value.to_string())
                            .unwrap_or_default();
                        let code = callback_request_url
                            .query_pairs()
                            .find(|(key, _)| key == "code")
                            .map(|(_, value)| value.to_string())
                            .unwrap_or_default();
                        if state_value != oauth_state_for_task {
                            respond_loopback_html(
                                &mut socket,
                                "授权失败",
                                "授权状态校验失败，请回到 Locus 重新发起授权。",
                            )
                            .await;
                            return Err("飞书授权状态校验失败，请重新发起授权。".to_string());
                        }
                        if !callback_error.trim().is_empty() {
                            let error_message = if callback_error_description.trim().is_empty() {
                                format!("飞书授权失败：{}", callback_error)
                            } else {
                                format!(
                                    "飞书授权失败：{} ({})",
                                    callback_error_description, callback_error
                                )
                            };
                            respond_loopback_html(&mut socket, "授权失败", &error_message).await;
                            return Err(error_message);
                        }
                        if code.trim().is_empty() {
                            respond_loopback_html(
                                &mut socket,
                                "授权失败",
                                "飞书没有返回授权码，请回到 Locus 重新发起授权。",
                            )
                            .await;
                            return Err("飞书没有返回授权码，请重新发起授权。".to_string());
                        }
                        let client = feishu_client()?;
                        let config = read_config(&working_dir_for_task)?;
                        let app_secret = read_app_secret(&working_dir_for_task)?
                            .filter(|value| !value.trim().is_empty())
                            .ok_or_else(|| "请输入飞书 App Secret 后再继续。".to_string())?;
                        let token = exchange_authorization_code(
                            &client,
                            &config,
                            &app_secret,
                            &code,
                            &callback_url_for_task,
                            &code_verifier_for_task,
                        )
                        .await?;
                        write_stored_user_token(&working_dir_for_task, &token)?;
                        respond_loopback_html(
                            &mut socket,
                            "授权完成",
                            "飞书用户身份授权已完成，返回 Locus 后可以继续测试连接或开始导入。",
                        )
                        .await;
                        Ok::<(), String>(())
                    } => match outcome {
                        Ok(()) => FeishuOauthWaitOutcome::Authorized,
                        Err(error) => FeishuOauthWaitOutcome::Failed(error),
                    }
                }
            },
        )
        .await;

        match result {
            Ok(FeishuOauthWaitOutcome::Authorized) => {
                if !oauth_wait_session_is_current(
                    state_for_task.clone(),
                    &working_dir_for_task,
                    oauth_wait_session,
                )
                .await
                {
                    return;
                }
                if let Ok(status) = get_feishu_reference_import_status(
                    &working_dir_for_task,
                    None,
                    state_for_task.clone(),
                )
                .await
                {
                    set_runtime_status(
                        state_for_task,
                        &working_dir_for_task,
                        FeishuReferenceImportStatus {
                            stage: FeishuReferenceImportStage::Idle,
                            message: "飞书用户身份授权已完成，可继续测试连接或开始导入。"
                                .to_string(),
                            ..status
                        },
                    )
                    .await;
                }
            }
            Ok(FeishuOauthWaitOutcome::Failed(error)) => {
                if !oauth_wait_session_is_current(
                    state_for_task.clone(),
                    &working_dir_for_task,
                    oauth_wait_session,
                )
                .await
                {
                    return;
                }
                update_runtime_status(state_for_task, &working_dir_for_task, |status| {
                    status.running = false;
                    status.stage = FeishuReferenceImportStage::Error;
                    status.state = FeishuReferenceImportStateKind::Error;
                    status.error = Some(error.clone());
                    status.message = error;
                })
                .await;
            }
            Ok(FeishuOauthWaitOutcome::Cancelled) => {}
            Err(_) => {
                if !oauth_wait_session_is_current(
                    state_for_task.clone(),
                    &working_dir_for_task,
                    oauth_wait_session,
                )
                .await
                {
                    return;
                }
                update_runtime_status(state_for_task, &working_dir_for_task, |status| {
                    status.running = false;
                    status.stage = FeishuReferenceImportStage::Error;
                    status.state = FeishuReferenceImportStateKind::Error;
                    status.error = Some("飞书授权等待超时，请重新发起授权。".to_string());
                    status.message = "飞书授权等待超时，请重新发起授权。".to_string();
                })
                .await;
            }
        }
    });

    Ok(FeishuReferenceOauthStartResult {
        authorize_url: authorize_url_string,
        callback_url,
        callback_urls,
        state: oauth_state,
    })
}

pub async fn cancel_feishu_reference_oauth_wait(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    let runtime = state.lock().await;
    if runtime.working_dir != working_dir
        || runtime.status.stage != FeishuReferenceImportStage::Authorizing
    {
        drop(runtime);
        return get_feishu_reference_import_status(working_dir, target_path, state).await;
    }

    runtime.oauth_wait_session.fetch_add(1, Ordering::Relaxed);
    runtime.oauth_wait_cancel.notify_waiters();
    drop(runtime);

    let mut status = if let Some(target_path) = target_path {
        derive_directory_feishu_reference_import_status(working_dir, target_path)?
    } else {
        derive_persisted_feishu_reference_import_status(working_dir)?
    };
    status.message = "已停止等待飞书授权，可重新发起授权。".to_string();
    set_runtime_status(state, working_dir, status.clone()).await;
    Ok(status)
}

pub async fn list_feishu_reference_space_nodes(
    working_dir: &str,
    space_id: String,
    parent_node_token: Option<String>,
) -> Result<Vec<FeishuReferenceNodeSummary>, String> {
    let config = read_config(working_dir)?;
    validate_core_config(&config, !app_secret_configured(working_dir)?)?;
    let client = feishu_client()?;
    let access_token = resolve_access_token(working_dir, &config, &client).await?;
    let items = fetch_all_space_nodes(
        &client,
        &config,
        &access_token,
        space_id.trim(),
        parent_node_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .await?;
    let mut nodes = items
        .into_iter()
        .map(|item| FeishuReferenceNodeSummary {
            node_token: item.node_token,
            title: if item.title.trim().is_empty() {
                "未命名节点".to_string()
            } else {
                item.title
            },
            obj_token: item.obj_token,
            obj_type: item.obj_type,
            has_child: item.has_child,
            parent_node_token: item.parent_node_token,
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.title.cmp(&right.title));
    Ok(nodes)
}

pub async fn start_feishu_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    request: FeishuReferenceImportRequest,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    let target_path = normalize_optional_text(request.target_path.clone());
    if let Some(target_path) = target_path.as_deref() {
        ensure_reference_target_directory(&working_dir, target_path)?;
    }
    let mut config = read_config(&working_dir)?;
    config.space_id = normalize_optional_text(Some(request.space_id));
    config.space_name = normalize_optional_text(request.space_name);
    config.roots = normalize_request_roots(
        request.roots,
        request.root_node_token,
        request.root_node_title,
    );
    (config.root_node_token, config.root_node_title) = primary_root_fields(&config.roots);
    if target_path.is_none() {
        save_config(&working_dir, &config)?;
    }
    validate_core_config(&config, !app_secret_configured(&working_dir)?)?;
    validate_selection(&config)?;

    let authorized = authorization_ready(&working_dir, &config)?;
    if config.auth_mode == FeishuReferenceAuthMode::Oauth && !authorized {
        return Err("当前鉴权模式需要先完成飞书授权。".to_string());
    }

    let prior_status =
        get_feishu_reference_import_status(&working_dir, target_path.as_deref(), state.clone())
            .await?;
    let stored_user_token = if config.auth_mode == FeishuReferenceAuthMode::Oauth {
        read_stored_user_token(&working_dir)?
    } else {
        None
    };
    let managed_path = target_path
        .as_deref()
        .map(reference_target_managed_path)
        .unwrap_or_else(|| FEISHU_REFERENCE_MANAGED_PATH.to_string());
    let mut starting_status = FeishuReferenceImportStatus {
        state: FeishuReferenceImportStateKind::Running,
        stage: FeishuReferenceImportStage::ListingNodes,
        running: true,
        auth_mode: config.auth_mode,
        oauth_persistence_mode: config.oauth_persistence_mode,
        app_id: config.app_id.clone(),
        app_secret: None,
        app_secret_configured: true,
        authorized,
        authorized_user_name: None,
        authorized_user_open_id: None,
        authorized_user_email: None,
        open_base_url: config.open_base_url.clone(),
        callback_urls: feishu_oauth_callback_urls(),
        required_scopes: feishu_reference_user_auth_scopes(config.oauth_persistence_mode),
        granted_scopes: Vec::new(),
        missing_scopes: Vec::new(),
        access_token_expires_at: None,
        refresh_token_expires_at: None,
        can_refresh: false,
        space_id: config.space_id.clone(),
        space_name: config.space_name.clone(),
        selected_roots: config.roots.clone(),
        root_node_token: config.root_node_token.clone(),
        root_node_title: config.root_node_title.clone(),
        imported_space_id: prior_status.imported_space_id.clone(),
        imported_space_name: prior_status.imported_space_name.clone(),
        imported_roots: prior_status.imported_roots.clone(),
        imported_root_node_token: prior_status.imported_root_node_token.clone(),
        imported_root_node_title: prior_status.imported_root_node_title.clone(),
        imported_at: prior_status.imported_at,
        imported_doc_count: prior_status.imported_doc_count,
        managed_path: managed_path.clone(),
        progress: None,
        processed_docs: 0,
        total_docs: None,
        current_title: None,
        current_path: Some(managed_path.clone()),
        message: "正在读取飞书知识空间结构。".to_string(),
        error: None,
        last_outcome: None,
    };
    apply_authorized_user_to_status(
        &mut starting_status,
        &config,
        authorized,
        stored_user_token.as_ref(),
    );

    {
        let mut runtime = state.lock().await;
        if runtime.status.running {
            return Err("飞书知识库导入任务仍在进行中。".to_string());
        }
        runtime.working_dir = working_dir.clone();
        runtime.cancel_requested.store(false, Ordering::Relaxed);
        runtime.status = starting_status.clone();
    }

    let cancel_requested = {
        let runtime = state.lock().await;
        runtime.cancel_requested.clone()
    };
    let state_for_task = state.clone();
    let working_dir_for_task = working_dir.clone();
    let config_for_task = config.clone();
    let target_path_for_task = target_path.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = run_feishu_reference_import(
            app_handle.clone(),
            working_dir_for_task.clone(),
            config_for_task.clone(),
            target_path_for_task.clone(),
            state_for_task.clone(),
            knowledge_index_state,
            cancel_requested.clone(),
        )
        .await;
        match outcome {
            Ok(status) => {
                set_runtime_status(state_for_task, &working_dir_for_task, status).await;
            }
            Err(FeishuReferenceImportRunError::Cancelled) => {
                if let Ok(status) = get_feishu_reference_import_status(
                    &working_dir_for_task,
                    target_path_for_task.as_deref(),
                    state_for_task.clone(),
                )
                .await
                {
                    set_runtime_status(
                        state_for_task,
                        &working_dir_for_task,
                        FeishuReferenceImportStatus {
                            running: false,
                            state: status.state,
                            stage: FeishuReferenceImportStage::Idle,
                            last_outcome: Some(FeishuReferenceImportLastOutcome::Cancelled),
                            message: "已取消飞书知识库文档导入。".to_string(),
                            error: None,
                            progress: None,
                            processed_docs: 0,
                            total_docs: None,
                            current_title: None,
                            current_path: None,
                            ..status
                        },
                    )
                    .await;
                }
            }
            Err(FeishuReferenceImportRunError::Failed(error)) => {
                if let Ok(status) = get_feishu_reference_import_status(
                    &working_dir_for_task,
                    target_path_for_task.as_deref(),
                    state_for_task.clone(),
                )
                .await
                {
                    set_runtime_status(
                        state_for_task,
                        &working_dir_for_task,
                        FeishuReferenceImportStatus {
                            running: false,
                            state: FeishuReferenceImportStateKind::Error,
                            stage: FeishuReferenceImportStage::Error,
                            message: error.clone(),
                            error: Some(error),
                            progress: None,
                            current_title: None,
                            current_path: None,
                            ..status
                        },
                    )
                    .await;
                }
            }
        }
    });

    Ok(starting_status)
}

pub async fn cancel_feishu_reference_import(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    let runtime = state.lock().await;
    let target_matches = target_path
        .map(reference_target_managed_path)
        .map(|value| runtime.status.managed_path == value)
        .unwrap_or(true);
    if runtime.working_dir == working_dir && runtime.status.running && target_matches {
        runtime.cancel_requested.store(true, Ordering::Relaxed);
        let mut status = runtime.status.clone();
        status.message = "正在取消飞书知识库文档导入。".to_string();
        return Ok(status);
    }
    drop(runtime);
    get_feishu_reference_import_status(working_dir, target_path, state).await
}

pub async fn delete_feishu_reference_docs(
    app_handle: AppHandle,
    working_dir: String,
    target_path: Option<String>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<FeishuReferenceImportRuntime>>,
) -> Result<FeishuReferenceImportStatus, String> {
    let target_path = normalize_optional_text(target_path);
    let target_managed_path = target_path
        .as_deref()
        .map(reference_target_managed_path)
        .unwrap_or_else(|| FEISHU_REFERENCE_MANAGED_PATH.to_string());
    {
        let runtime = state.lock().await;
        if runtime.working_dir == working_dir
            && runtime.status.running
            && runtime.status.managed_path == target_managed_path
        {
            return Err("飞书知识库导入任务仍在进行中，无法删除导入结果。".to_string());
        }
    }

    if let Some(target_path) = target_path.as_deref() {
        delete_target_reference_import_artifacts(&working_dir, target_path)?;
    } else {
        remove_dir_if_exists(&managed_dir_path(&working_dir))?;
        remove_dir_if_exists(&temp_root_path(&working_dir))?;
        remove_dir_if_exists(&backup_dir_path(&working_dir))?;
        remove_file_if_exists(&managed_directory_config_path(
            &working_dir,
            FEISHU_REFERENCE_DIRECTORY_CONFIG_SUFFIX,
        ))?;
        remove_file_if_exists(&managed_directory_config_path(
            &working_dir,
            FEISHU_REFERENCE_LEGACY_DIRECTORY_CONFIG_SUFFIX,
        ))?;
        delete_manifest(&working_dir)?;
    }
    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_delete_feishu_reference_docs",
    )
    .await
    .map_err(|error| error.message)?;
    let status =
        get_feishu_reference_import_status(&working_dir, target_path.as_deref(), state.clone())
            .await?;
    set_runtime_status(state, &working_dir, status.clone()).await;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn seed_reference_directory(working_dir: &str, target_path: &str) {
        let root = reference_target_dir_path(working_dir, target_path);
        std::fs::create_dir_all(&root).expect("create reference directory");
        std::fs::write(root.join("Imported.md"), "# Imported").expect("seed imported markdown");
    }

    #[tokio::test]
    async fn saving_directory_config_keeps_imported_snapshot_separate() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let target_path = "reference-folder";

        seed_reference_directory(&working_dir, target_path);

        let mut config = FeishuReferenceConfig::default();
        config.app_id = "app-test".to_string();
        save_config(&working_dir, &config).expect("seed global config");

        knowledge_store::update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            target_path,
            build_directory_external_sources(
                "imported-space",
                &[FeishuReferenceRootSelection {
                    node_token: "imported-root".to_string(),
                    node_title: Some("Imported Root".to_string()),
                }],
                Some(123),
            ),
        )
        .expect("seed imported snapshot");

        let status = save_feishu_reference_config(
            &working_dir,
            FeishuReferenceConfigInput {
                target_path: Some(target_path.to_string()),
                auth_mode: FeishuReferenceAuthMode::AppCredentials,
                oauth_persistence_mode: FeishuReferenceOauthPersistenceMode::Session,
                app_id: "app-test".to_string(),
                app_secret: None,
                clear_app_secret: false,
                open_base_url: FEISHU_REFERENCE_DEFAULT_OPEN_BASE_URL.to_string(),
                space_id: Some("selected-space".to_string()),
                space_name: Some("Selected Space".to_string()),
                roots: vec![FeishuReferenceRootSelection {
                    node_token: "selected-root".to_string(),
                    node_title: Some("Selected Root".to_string()),
                }],
                root_node_token: None,
                root_node_title: None,
            },
            Arc::new(tokio::sync::Mutex::new(
                FeishuReferenceImportRuntime::default(),
            )),
        )
        .await
        .expect("save directory config");

        assert_eq!(status.space_id.as_deref(), Some("selected-space"));
        assert_eq!(status.selected_roots.len(), 1);
        assert_eq!(status.selected_roots[0].node_token, "selected-root");
        assert_eq!(status.imported_space_id.as_deref(), Some("imported-space"));
        assert_eq!(status.imported_roots.len(), 1);
        assert_eq!(status.imported_roots[0].node_token, "imported-root");
        assert_eq!(status.imported_at, Some(123));

        let binding = read_directory_binding(&working_dir, target_path)
            .expect("read directory binding")
            .expect("directory binding exists");
        assert_eq!(binding.space_id.as_deref(), Some("selected-space"));
        assert_eq!(binding.space_name.as_deref(), Some("Selected Space"));
        assert_eq!(binding.roots.len(), 1);
        assert_eq!(binding.roots[0].node_token, "selected-root");

        let (_, imported_space_id, imported_roots, imported_at) =
            read_feishu_directory_import_snapshot(&working_dir, target_path)
                .expect("read imported snapshot");
        assert_eq!(imported_space_id.as_deref(), Some("imported-space"));
        assert_eq!(imported_roots.len(), 1);
        assert_eq!(imported_roots[0].node_token, "imported-root");
        assert_eq!(imported_at, Some(123));
    }

    #[test]
    fn delete_target_reference_import_artifacts_removes_directory_and_sidecars() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let target_path = "reference-folder";

        seed_reference_directory(&working_dir, target_path);
        knowledge_store::update_directory_external_sources(
            &working_dir,
            KnowledgeType::Reference,
            target_path,
            build_directory_external_sources(
                "imported-space",
                &[FeishuReferenceRootSelection {
                    node_token: "imported-root".to_string(),
                    node_title: Some("Imported Root".to_string()),
                }],
                Some(123),
            ),
        )
        .expect("seed imported snapshot");
        save_directory_binding(
            &working_dir,
            target_path,
            &FeishuReferenceDirectoryBinding {
                space_id: Some("selected-space".to_string()),
                space_name: Some("Selected Space".to_string()),
                roots: vec![FeishuReferenceRootSelection {
                    node_token: "selected-root".to_string(),
                    node_title: Some("Selected Root".to_string()),
                }],
                root_node_token: None,
                root_node_title: None,
            },
        )
        .expect("save directory binding");

        let legacy_config = knowledge_root(&working_dir)
            .join("reference")
            .join("reference-folder.meta");
        std::fs::write(&legacy_config, "legacy").expect("write legacy config");

        delete_target_reference_import_artifacts(&working_dir, target_path)
            .expect("delete target artifacts");

        assert!(!reference_target_dir_path(&working_dir, target_path).exists());
        assert!(!knowledge_root(&working_dir)
            .join("reference")
            .join("reference-folder.locus-meta")
            .exists());
        assert!(!legacy_config.exists());
        assert!(!directory_binding_path(&working_dir, target_path).exists());
    }

    #[test]
    fn render_docx_blocks_markdown_preserves_common_structure() {
        let markdown = render_docx_blocks_markdown(vec![
            json!({
                "block_id": "doc",
                "block_type": 1,
                "parent_id": "",
                "children": ["heading", "bullet", "quote", "todo", "code", "divider", "table"],
                "page": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "战斗设计改进草案",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "heading",
                "parent_id": "doc",
                "heading2": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "核心机制",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "bullet",
                "parent_id": "doc",
                "children": ["bullet_nested"],
                "bullet": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "受击打断",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "bullet_nested",
                "parent_id": "bullet",
                "bullet": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "BOSS也应该响应玩家动作",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "quote",
                "parent_id": "doc",
                "quote": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "强调动作游戏的主动反应",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "todo",
                "parent_id": "doc",
                "todo": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "能量循环没有实装",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {
                        "done": true
                    }
                }
            }),
            json!({
                "block_id": "code",
                "parent_id": "doc",
                "code": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "println!(\"hit\");\n",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {
                        "language": "rust"
                    }
                }
            }),
            json!({
                "block_id": "divider",
                "parent_id": "doc",
                "divider": {}
            }),
            json!({
                "block_id": "table",
                "parent_id": "doc",
                "table": {
                    "cells": ["cell_1", "cell_2", "cell_3", "cell_4"],
                    "property": {
                        "row_size": 2,
                        "column_size": 2
                    }
                }
            }),
            json!({
                "block_id": "cell_1",
                "parent_id": "table",
                "children": ["cell_1_text"],
                "table_cell": {}
            }),
            json!({
                "block_id": "cell_1_text",
                "parent_id": "cell_1",
                "text": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "模块",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "cell_2",
                "parent_id": "table",
                "children": ["cell_2_text"],
                "table_cell": {}
            }),
            json!({
                "block_id": "cell_2_text",
                "parent_id": "cell_2",
                "text": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "状态",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "cell_3",
                "parent_id": "table",
                "children": ["cell_3_text"],
                "table_cell": {}
            }),
            json!({
                "block_id": "cell_3_text",
                "parent_id": "cell_3",
                "text": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "能量循环",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
            json!({
                "block_id": "cell_4",
                "parent_id": "table",
                "children": ["cell_4_text"],
                "table_cell": {}
            }),
            json!({
                "block_id": "cell_4_text",
                "parent_id": "cell_4",
                "text": {
                    "elements": [
                        {
                            "text_run": {
                                "content": "未实现",
                                "text_element_style": {}
                            }
                        }
                    ],
                    "style": {}
                }
            }),
        ])
        .expect("render markdown");

        assert_eq!(
            markdown,
            concat!(
                "## 核心机制\n\n",
                "- 受击打断\n",
                "  - BOSS也应该响应玩家动作\n\n",
                "> 强调动作游戏的主动反应\n\n",
                "- [x] 能量循环没有实装\n\n",
                "```rust\n",
                "println!(\"hit\");\n",
                "```\n\n",
                "---\n\n",
                "| 模块 | 状态 |\n",
                "| --- | --- |\n",
                "| 能量循环 | 未实现 |\n",
            )
        );
        assert!(!markdown.contains("战斗设计改进草案"));
    }
}
