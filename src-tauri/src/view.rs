use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};

pub const VIEW_SCHEMA: &str = "locus.view.v1";
pub const VIEW_BINDINGS_SCHEMA: &str = "locus.view.bindings.v1";
pub const VIEW_ROOT_RELATIVE: &str = "Locus/View";
pub const VIEW_WORKSPACE_SRC_DIR: &str = "src";
pub const TEMP_VIEW_ROOT_RELATIVE: &str = "view-packages";
pub const VIEW_RELOAD_EVENT: &str = "view-package-reloaded";
pub const VIEW_TREE_CHANGED_EVENT: &str = "view-tree-changed";
pub const VIEW_AUTOMATION_REQUEST_EVENT: &str = "view-automation-request";

const MAIN_WINDOW_LABEL: &str = "main";
const VIEW_HOST_ROUTE: &str = "/view-host";
const VIEW_HOST_TABS_SELECT_EVENT: &str = "view-host-tabs-select";
const VIEW_FRONTEND_LOG_REL_PATH: &str = ".locus/logs/frontend.log";
const VIEW_FRONTEND_LOG_MAX_CHARS: usize = 16_384;
const VIEW_PACKAGE_ARCHIVE_MAX_ENTRIES: usize = 20_000;
const VIEW_PACKAGE_ARCHIVE_MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const VIEW_WINDOW_LABEL_PREFIX: &str = "view-";

mod templates;

#[derive(Debug, Default)]
pub struct ViewAutomationStore {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ViewAutomationReply>>>,
}

#[derive(Debug)]
pub struct ViewAutomationReply {
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl ViewAutomationStore {
    pub fn insert(
        &self,
        request_id: String,
        tx: tokio::sync::oneshot::Sender<ViewAutomationReply>,
    ) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "View automation store is unavailable".to_string())?;
        pending.insert(request_id, tx);
        Ok(())
    }

    pub fn complete(&self, request_id: &str, reply: ViewAutomationReply) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let Some(tx) = pending.remove(request_id) else {
            return false;
        };
        tx.send(reply).is_ok()
    }

    pub fn cancel(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewAutomationRequestEvent {
    pub request_id: String,
    pub view_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCaptureResult {
    pub view_id: String,
    pub window_label: String,
    pub mime_type: String,
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub byte_size: usize,
    #[serde(skip_serializing)]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewScriptManifest {
    pub name: String,
    pub path: String,
    pub entry_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewCapabilities {
    #[serde(default)]
    pub unity: bool,
    #[serde(default)]
    pub bindings: bool,
    #[serde(default)]
    pub write_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewRequirements {
    #[serde(default)]
    pub unity_connection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewManifest {
    pub schema: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub entry: String,
    pub style: String,
    pub bindings: String,
    #[serde(default)]
    pub scripts: Vec<ViewScriptManifest>,
    #[serde(default)]
    pub capabilities: ViewCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<ViewRequirements>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ViewCreateRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewTemplateSummary {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPackageSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub package_rel_path: String,
    pub package_root: String,
    pub manifest_path: String,
    pub updated_at: i64,
    pub capabilities: ViewCapabilities,
    pub requirements: ViewRequirements,
    #[serde(default)]
    pub temporary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewFolderSummary {
    pub rel_path: String,
    pub name: String,
    pub package_root: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewTreeSnapshot {
    pub views: Vec<ViewPackageSummary>,
    pub folders: Vec<ViewFolderSummary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCreateFolderRequest {
    #[serde(default)]
    pub parent_rel_path: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDeleteEntryRequest {
    pub rel_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewMoveEntryRequest {
    pub source_rel_path: String,
    #[serde(default)]
    pub target_dir_rel_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewExportPackageRequest {
    pub view_id: String,
    pub file_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewImportPackageRequest {
    pub file_path: String,
    #[serde(default)]
    pub target_dir_rel_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPackageFile {
    pub rel_path: String,
    pub kind: String,
    pub content: String,
    pub size: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPackageDetail {
    pub summary: ViewPackageSummary,
    pub manifest: ViewManifest,
    pub files: Vec<ViewPackageFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewPackageImportResult {
    pub summary: ViewPackageSummary,
    pub snapshot: ViewTreeSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewRunResult {
    pub id: String,
    pub window_label: String,
    pub host_url: String,
    pub package_root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSetTabHostRequest {
    pub host_label: String,
    pub view_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDetachTabRequest {
    pub view_id: String,
    #[serde(default)]
    pub source_host_label: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCompileScriptRequest {
    pub view_id: String,
    pub script_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewCompileScriptResult {
    pub name: String,
    pub hash: String,
    pub cache_hit: bool,
    pub assembly_id: String,
    #[serde(default)]
    pub domain_fingerprint: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCallScriptRequest {
    pub view_id: String,
    pub script_name: String,
    pub method: String,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCallScriptResult {
    pub compile: ViewCompileScriptResult,
    pub method: String,
    #[serde(default)]
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewFrontendLogRequest {
    pub view_id: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewFrontendLogReadRequest {
    pub view_id: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewFrontendLogEntry {
    pub time: i64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingTarget {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingReadRequest {
    pub view_id: String,
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub target: Option<ViewBindingTarget>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingDiscoverRequest {
    pub view_id: String,
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub target: Option<ViewBindingTarget>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub field_name: Option<String>,
    #[serde(default)]
    pub field_type: Option<String>,
    #[serde(default)]
    pub max_depth: Option<i32>,
    #[serde(default)]
    pub max_results: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewManagedReferenceTypeOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub assembly: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewEnumOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub index: i32,
    #[serde(default)]
    pub numeric_value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewSerializedPropertySnapshot {
    #[serde(default)]
    pub property_path: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub property_type: String,
    #[serde(default)]
    pub value_type: String,
    #[serde(default)]
    pub field_type_full_name: String,
    #[serde(default)]
    pub field_type_assembly: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub display_value: String,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub has_children: bool,
    #[serde(default)]
    pub is_array: bool,
    #[serde(default)]
    pub array_size: i32,
    #[serde(default)]
    pub is_flags_enum: bool,
    #[serde(default)]
    pub enum_value_index: i32,
    #[serde(default)]
    pub enum_value_flag: i64,
    #[serde(default)]
    pub enum_options: Vec<ViewEnumOption>,
    #[serde(default)]
    pub children: Vec<ViewSerializedPropertySnapshot>,
    #[serde(default)]
    pub is_managed_reference: bool,
    #[serde(default)]
    pub managed_reference_full_typename: String,
    #[serde(default)]
    pub managed_reference_field_typename: String,
    #[serde(default)]
    pub managed_reference_display_name: String,
    #[serde(default)]
    pub managed_reference_types: Vec<ViewManagedReferenceTypeOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingReadResult {
    pub ok: bool,
    #[serde(default)]
    pub binding_id: Option<String>,
    pub message: String,
    pub target: ViewBindingTarget,
    #[serde(flatten)]
    pub property: ViewSerializedPropertySnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingDiscoverMatch {
    #[serde(default)]
    pub property_path: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub property_type: String,
    #[serde(default)]
    pub value_type: String,
    #[serde(default)]
    pub field_type_full_name: String,
    #[serde(default)]
    pub field_type_assembly: String,
    #[serde(default)]
    pub display_value: String,
    #[serde(default)]
    pub editable: bool,
    #[serde(default)]
    pub has_children: bool,
    #[serde(default)]
    pub is_array: bool,
    #[serde(default)]
    pub is_managed_reference: bool,
    #[serde(default)]
    pub depth: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingDiscoverResult {
    pub ok: bool,
    #[serde(default)]
    pub binding_id: Option<String>,
    pub message: String,
    pub target: ViewBindingTarget,
    #[serde(default)]
    pub matches: Vec<ViewBindingDiscoverMatch>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingWriteRequest {
    pub view_id: String,
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub target: Option<ViewBindingTarget>,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingWriteResult {
    #[serde(flatten)]
    pub read: ViewBindingReadResult,
    pub saved: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingApplyWrite {
    #[serde(default)]
    pub binding_id: Option<String>,
    #[serde(default)]
    pub target: Option<ViewBindingTarget>,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingApplyRequest {
    pub view_id: String,
    pub writes: Vec<ViewBindingApplyWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingApplyResult {
    pub ok: bool,
    pub message: String,
    pub results: Vec<ViewBindingWriteResult>,
}

#[derive(Debug, Clone)]
struct ResolvedViewScript {
    view_id: String,
    script_name: String,
    path: String,
    entry_type: String,
    source: String,
    source_hash: String,
}

#[derive(Debug, Clone)]
struct ResolvedViewBinding {
    target: ViewBindingTarget,
    mode: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedViewScriptSource {
    modified: Option<SystemTime>,
    len: u64,
    resolved: ResolvedViewScript,
}

#[derive(Debug, Clone, Default)]
struct LoadedViewBindings {
    by_id: HashMap<String, serde_json::Value>,
}

pub fn supported_view_templates() -> Vec<ViewTemplateSummary> {
    templates::supported_view_templates()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn updated_at(path: &Path) -> i64 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn is_valid_view_id(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() || id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        return false;
    }
    id.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub fn normalize_view_id(id: &str) -> Result<String, String> {
    let normalized = id.trim();
    if !is_valid_view_id(normalized) {
        return Err("Invalid view id: use lowercase kebab-case.".to_string());
    }
    Ok(normalized.to_string())
}

pub fn normalize_package_rel_path(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized.contains("//")
        || normalized
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("Invalid package relative path: {}", value));
    }
    Ok(normalized)
}

fn normalize_view_tree_rel_path(value: &str, allow_empty: bool) -> Result<String, String> {
    let normalized = value
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        return if allow_empty {
            Ok(String::new())
        } else {
            Err("View path cannot be empty.".to_string())
        };
    }
    if normalized.contains(':')
        || normalized.contains("//")
        || normalized
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("Invalid View path: {}", value));
    }
    Ok(normalized)
}

fn normalize_view_folder_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() {
        return Err("Folder name cannot be empty.".to_string());
    }
    if name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        return Err(format!("Invalid View folder name: {}", value));
    }
    Ok(name.to_string())
}

pub fn validate_view_manifest(manifest: &ViewManifest) -> Result<(), String> {
    if manifest.schema != VIEW_SCHEMA {
        return Err(format!("Unsupported View schema: {}", manifest.schema));
    }
    normalize_view_id(&manifest.id)?;
    if manifest.name.trim().is_empty() {
        return Err("View name cannot be empty.".to_string());
    }
    if manifest.version.trim().is_empty() {
        return Err("View version cannot be empty.".to_string());
    }
    if !templates::is_supported_template(&manifest.template) {
        return Err(format!("Unsupported View template: {}", manifest.template));
    }
    if let Some(icon) = manifest
        .icon
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_view_icon_name(icon)?;
    }
    normalize_package_rel_path(&manifest.entry)?;
    normalize_package_rel_path(&manifest.style)?;
    normalize_package_rel_path(&manifest.bindings)?;

    let mut script_names = BTreeSet::new();
    for script in &manifest.scripts {
        if script.name.trim().is_empty() {
            return Err("View script name cannot be empty.".to_string());
        }
        if !script_names.insert(script.name.trim().to_string()) {
            return Err(format!("Duplicate View script name: {}", script.name));
        }
        normalize_package_rel_path(&script.path)?;
        if !script.path.replace('\\', "/").starts_with("unity/") {
            return Err(format!(
                "View script path must stay under unity/: {}",
                script.path
            ));
        }
        if script.entry_type.trim().is_empty() {
            return Err(format!(
                "View script entryType cannot be empty: {}",
                script.name
            ));
        }
    }

    Ok(())
}

fn validate_view_icon_name(icon: &str) -> Result<(), String> {
    let len = icon.chars().count();
    if len == 0 || len > 64 {
        return Err("View icon must be between 1 and 64 characters.".to_string());
    }
    if !icon
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!("Invalid View icon name: {}", icon));
    }
    Ok(())
}

fn workspace_root(working_dir: &str) -> Result<PathBuf, String> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return Err("A Unity project working directory is required.".to_string());
    }
    let path = Path::new(trimmed);
    if !path.is_dir() {
        return Err(format!("Working directory not found: {}", trimmed));
    }
    Ok(dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

pub fn views_root_for_workspace(working_dir: &str) -> Result<PathBuf, String> {
    Ok(workspace_root(working_dir)?.join(VIEW_ROOT_RELATIVE))
}

fn default_view_package_name(working_dir: &str) -> Result<String, String> {
    let root = workspace_root(working_dir)?;
    let raw_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Project");
    normalize_view_folder_name(raw_name).or_else(|_| Ok("Project".to_string()))
}

fn normalize_view_package_name(value: &str) -> Result<String, String> {
    normalize_view_folder_name(value)
}

fn request_view_package_name(
    working_dir: &str,
    request: &ViewCreateRequest,
) -> Result<String, String> {
    match request
        .package_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(package_name) => normalize_view_package_name(package_name),
        None => default_view_package_name(working_dir),
    }
}

fn view_package_workspace_root(working_dir: &str, package_name: &str) -> Result<PathBuf, String> {
    let package_name = normalize_view_package_name(package_name)?;
    Ok(views_root_for_workspace(working_dir)?.join(package_name))
}

fn temp_workspace_dir_name(working_dir: &str) -> Result<String, String> {
    let root = workspace_root(working_dir)?;
    let normalized = root.display().to_string().replace('\\', "/");
    let hash = blake3::hash(normalized.to_ascii_lowercase().as_bytes())
        .to_hex()
        .to_string();
    let raw_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in raw_name.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch == ' ' {
            Some('-')
        } else {
            None
        };
        let Some(next) = next else {
            continue;
        };
        if next == '-' {
            if slug.is_empty() || previous_dash {
                continue;
            }
            previous_dash = true;
        } else {
            previous_dash = false;
        }
        slug.push(next);
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("workspace");
    }
    Ok(format!("{}-{}", slug, &hash[..12]))
}

pub fn temporary_views_root_for_workspace(working_dir: &str) -> Result<PathBuf, String> {
    Ok(crate::commands::app_temp_dir()?
        .join(TEMP_VIEW_ROOT_RELATIVE)
        .join(temp_workspace_dir_name(working_dir)?))
}

pub fn view_package_root(working_dir: &str, id: &str) -> Result<PathBuf, String> {
    let id = normalize_view_id(id)?;
    let package_name = default_view_package_name(working_dir)?;
    let views_root = views_root_for_workspace(working_dir)?;
    let direct_root = views_root.join(package_name).join(&id);
    let matches = find_view_package_roots_by_id(&views_root, &id)?;
    match matches.len() {
        0 => Ok(direct_root),
        1 => Ok(matches[0].clone()),
        _ => Err(format!(
            "Multiple View packages use id '{}': {}",
            id,
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn view_package_root_for_request(
    working_dir: &str,
    request: &ViewCreateRequest,
    id: &str,
) -> Result<PathBuf, String> {
    let id = normalize_view_id(id)?;
    let package_name = request_view_package_name(working_dir, request)?;
    Ok(view_package_workspace_root(working_dir, &package_name)?.join(id))
}

pub fn resolve_view_package_root(working_dir: &str, id: &str) -> Result<PathBuf, String> {
    let id = normalize_view_id(id)?;
    let persistent_root = view_package_root(working_dir, &id)?;
    if manifest_matches_id(&persistent_root, &id) {
        return Ok(persistent_root);
    }

    let temp_root = temporary_views_root_for_workspace(working_dir)?;
    let matches = find_view_package_roots_by_id(&temp_root, &id)?;
    match matches.len() {
        0 => Ok(persistent_root),
        1 => Ok(matches[0].clone()),
        _ => Err(format!(
            "Multiple temporary View packages use id '{}': {}",
            id,
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn parse_view_create_request(
    value: serde_json::Value,
) -> Result<(ViewCreateRequest, bool), String> {
    let temporary = match value.get("temporary") {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| "temporary must be a boolean.".to_string())?,
        None => false,
    };

    let mut request_value = value;
    if let Some(object) = request_value.as_object_mut() {
        object.remove("temporary");
    }
    let request = serde_json::from_value::<ViewCreateRequest>(request_value)
        .map_err(|error| error.to_string())?;
    Ok((request, temporary))
}

fn package_path(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let rel_path = normalize_package_rel_path(rel_path)?;
    Ok(root.join(rel_path))
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join("view.json")
}

fn view_tree_path(views_root: &Path, rel_path: &str, allow_empty: bool) -> Result<PathBuf, String> {
    let rel_path = normalize_view_tree_rel_path(rel_path, allow_empty)?;
    if rel_path.is_empty() {
        Ok(views_root.to_path_buf())
    } else {
        Ok(views_root.join(rel_path))
    }
}

fn view_rel_path_for_root(views_root: &Path, root: &Path) -> Result<String, String> {
    let rel_path = root
        .strip_prefix(views_root)
        .map_err(|_| format!("Path is outside View root: {}", root.display()))?
        .display()
        .to_string()
        .replace('\\', "/");
    normalize_view_tree_rel_path(&rel_path, false)
}

fn inferred_view_requirements(capabilities: &ViewCapabilities) -> ViewRequirements {
    ViewRequirements {
        unity_connection: capabilities.unity || capabilities.bindings || capabilities.write_back,
    }
}

fn normalize_view_requirements(manifest: &mut ViewManifest) {
    if manifest.requirements.is_none() {
        manifest.requirements = Some(inferred_view_requirements(&manifest.capabilities));
    }
}

fn view_manifest_requirements(manifest: &ViewManifest) -> ViewRequirements {
    manifest
        .requirements
        .clone()
        .unwrap_or_else(|| inferred_view_requirements(&manifest.capabilities))
}

fn load_manifest_from_root(root: &Path) -> Result<ViewManifest, String> {
    let path = manifest_path(root);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let mut manifest: ViewManifest = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid View manifest {}: {}", path.display(), e))?;
    normalize_view_requirements(&mut manifest);
    validate_view_manifest(&manifest)?;
    Ok(manifest)
}

fn summary_from_manifest(
    views_root: &Path,
    root: &Path,
    manifest: &ViewManifest,
    temporary: bool,
) -> ViewPackageSummary {
    ViewPackageSummary {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        template: manifest.template.clone(),
        icon: manifest.icon.clone(),
        package_rel_path: root
            .strip_prefix(views_root)
            .ok()
            .map(|path| path.display().to_string().replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| manifest.id.clone()),
        package_root: root.display().to_string().replace('\\', "/"),
        manifest_path: manifest_path(root).display().to_string().replace('\\', "/"),
        updated_at: updated_at(&manifest_path(root)),
        capabilities: manifest.capabilities.clone(),
        requirements: view_manifest_requirements(manifest),
        temporary,
    }
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.starts_with(root)
}

fn is_skippable_view_scan_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | ".git" | "dist" | "target" | "Library" | "Temp"
    )
}

fn is_view_workspace_source_dir(_scan_root: &Path, path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == VIEW_WORKSPACE_SRC_DIR)
        .unwrap_or(false)
}

fn is_skippable_view_scan_entry(scan_root: &Path, entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    if is_view_workspace_source_dir(scan_root, entry.path()) {
        return true;
    }
    entry
        .file_name()
        .to_str()
        .map(is_skippable_view_scan_dir)
        .unwrap_or(false)
}

fn manifest_matches_id(root: &Path, id: &str) -> bool {
    if !manifest_path(root).is_file() {
        return false;
    }
    load_manifest_from_root(root)
        .map(|manifest| manifest.id == id)
        .unwrap_or(false)
}

fn find_view_package_roots_by_id(views_root: &Path, id: &str) -> Result<Vec<PathBuf>, String> {
    if !views_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut roots = Vec::new();
    for entry in walkdir::WalkDir::new(views_root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.file_type().is_file() || !is_skippable_view_scan_entry(views_root, entry)
        })
    {
        let entry = entry.map_err(|error| format!("Failed to scan View packages: {}", error))?;
        if !entry.file_type().is_file() || entry.file_name() != "view.json" {
            continue;
        }
        let Some(root) = entry.path().parent() else {
            continue;
        };
        if manifest_matches_id(root, id) {
            roots.push(root.to_path_buf());
        }
    }

    roots.sort();
    Ok(roots)
}

pub fn list_views_sync(working_dir: &str) -> Result<Vec<ViewPackageSummary>, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    if !views_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut views = Vec::new();
    for entry in walkdir::WalkDir::new(&views_root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.file_type().is_file() || !is_skippable_view_scan_entry(&views_root, entry)
        })
    {
        let entry = entry.map_err(|error| format!("Failed to scan View packages: {}", error))?;
        if !entry.file_type().is_file() || entry.file_name() != "view.json" {
            continue;
        }
        let Some(root) = entry.path().parent() else {
            continue;
        };
        match load_manifest_from_root(root) {
            Ok(manifest) => views.push(summary_from_manifest(&views_root, root, &manifest, false)),
            Err(error) => {
                eprintln!(
                    "[Locus] skipped invalid View package {}: {}",
                    root.display(),
                    error
                );
            }
        }
    }

    views.sort_by(|left, right| {
        left.package_rel_path
            .cmp(&right.package_rel_path)
            .then(left.name.cmp(&right.name))
            .then(left.id.cmp(&right.id))
    });
    Ok(views)
}

pub fn list_view_tree_sync(working_dir: &str) -> Result<ViewTreeSnapshot, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    let views = list_views_sync(working_dir)?;
    if !views_root.is_dir() {
        return Ok(ViewTreeSnapshot {
            views,
            folders: Vec::new(),
        });
    }

    let mut folders = Vec::new();
    for entry in walkdir::WalkDir::new(&views_root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_file() {
                return false;
            }
            if is_skippable_view_scan_entry(&views_root, entry) {
                return false;
            }
            !manifest_path(entry.path()).is_file()
        })
    {
        let entry = entry.map_err(|error| format!("Failed to scan View folders: {}", error))?;
        if !entry.file_type().is_dir() {
            continue;
        }
        let rel_path = view_rel_path_for_root(&views_root, entry.path())?;
        let name = entry
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&rel_path)
            .to_string();
        folders.push(ViewFolderSummary {
            rel_path,
            name,
            package_root: entry.path().display().to_string().replace('\\', "/"),
            updated_at: updated_at(entry.path()),
        });
    }

    folders.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    Ok(ViewTreeSnapshot { views, folders })
}

pub fn create_view_folder_sync(
    working_dir: &str,
    request: ViewCreateFolderRequest,
) -> Result<ViewFolderSummary, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    let parent_rel_path = request.parent_rel_path.as_deref().unwrap_or("").trim();
    let parent_rel_path = normalize_view_tree_rel_path(parent_rel_path, true)?;
    let folder_name = normalize_view_folder_name(&request.name)?;
    let parent = view_tree_path(&views_root, &parent_rel_path, true)?;
    if parent_rel_path.is_empty() {
        std::fs::create_dir_all(&parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    if !parent.is_dir() {
        return Err(format!("View folder not found: {}", parent_rel_path));
    }
    if manifest_path(&parent).is_file() {
        return Err("Cannot create a folder inside a View package.".to_string());
    }

    let folder = parent.join(&folder_name);
    if folder.exists() {
        return Err(format!("View folder already exists: {}", folder.display()));
    }
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Failed to create {}: {}", folder.display(), e))?;
    let rel_path = view_rel_path_for_root(&views_root, &folder)?;
    Ok(ViewFolderSummary {
        rel_path,
        name: folder_name,
        package_root: folder.display().to_string().replace('\\', "/"),
        updated_at: updated_at(&folder),
    })
}

pub fn delete_view_entry_sync(
    working_dir: &str,
    request: ViewDeleteEntryRequest,
) -> Result<ViewTreeSnapshot, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    let rel_path = normalize_view_tree_rel_path(&request.rel_path, false)?;
    let target = view_tree_path(&views_root, &rel_path, false)?;
    if !target.is_dir() {
        return Err(format!("View entry not found: {}", rel_path));
    }
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|e| format!("Failed to inspect {}: {}", target.display(), e))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to delete symlinked View entry: {}",
            rel_path
        ));
    }
    std::fs::remove_dir_all(&target)
        .map_err(|e| format!("Failed to delete {}: {}", target.display(), e))?;
    list_view_tree_sync(working_dir)
}

pub fn move_view_entry_sync(
    working_dir: &str,
    request: ViewMoveEntryRequest,
) -> Result<ViewTreeSnapshot, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    let source_rel_path = normalize_view_tree_rel_path(&request.source_rel_path, false)?;
    let target_dir_rel_path = request.target_dir_rel_path.as_deref().unwrap_or("");
    let target_dir_rel_path = normalize_view_tree_rel_path(target_dir_rel_path, true)?;
    if source_rel_path == target_dir_rel_path
        || target_dir_rel_path.starts_with(&format!("{}/", source_rel_path))
    {
        return Err("Cannot move a View entry into itself.".to_string());
    }

    let source = view_tree_path(&views_root, &source_rel_path, false)?;
    let target_dir = view_tree_path(&views_root, &target_dir_rel_path, true)?;
    if !source.is_dir() {
        return Err(format!("View entry not found: {}", source_rel_path));
    }
    if !target_dir.is_dir() {
        return Err(format!(
            "Target View folder not found: {}",
            target_dir_rel_path
        ));
    }
    if manifest_path(&target_dir).is_file() {
        return Err("Cannot move a View entry inside a View package.".to_string());
    }
    let metadata = std::fs::symlink_metadata(&source)
        .map_err(|e| format!("Failed to inspect {}: {}", source.display(), e))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to move symlinked View entry: {}",
            source_rel_path
        ));
    }

    let source_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid View entry path: {}", source_rel_path))?;
    let target = target_dir.join(source_name);
    if source == target {
        return Ok(list_view_tree_sync(working_dir)?);
    }
    if target.exists() {
        return Err(format!(
            "Target View entry already exists: {}",
            target.display()
        ));
    }

    if manifest_path(&source).is_file() {
        if target_dir_rel_path.is_empty() {
            return Err("Cannot move a View outside a package workspace.".to_string());
        }
        ensure_view_package_workspace(&target_dir)?;
    }

    std::fs::rename(&source, &target).map_err(|e| {
        format!(
            "Failed to move {} to {}: {}",
            source.display(),
            target.display(),
            e
        )
    })?;
    list_view_tree_sync(working_dir)
}

fn view_archive_output_path(file_path: &str) -> Result<PathBuf, String> {
    let trimmed = file_path.trim();
    if trimmed.is_empty() {
        return Err("Export path cannot be empty.".to_string());
    }
    let mut path = PathBuf::from(trimmed);
    let has_zip_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if !has_zip_extension {
        path.set_extension("zip");
    }
    Ok(path)
}

fn zip_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{}: {}", context, error)
}

pub fn export_view_package_sync(
    working_dir: &str,
    request: ViewExportPackageRequest,
) -> Result<String, String> {
    let root = resolve_view_package_root(working_dir, &request.view_id)?;
    if !root.is_dir() {
        return Err(format!("View package not found: {}", request.view_id));
    }
    let manifest = load_manifest_from_root(&root)?;
    if manifest.id != normalize_view_id(&request.view_id)? {
        return Err(format!(
            "View id mismatch: requested {}, manifest has {}",
            request.view_id, manifest.id
        ));
    }

    let output_path = view_archive_output_path(&request.file_path)?;
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let output = std::fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create {}: {}", output_path.display(), e))?;
    let mut archive = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(&root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
    {
        let entry =
            entry.map_err(|error| format!("Failed to scan View package for export: {}", error))?;
        paths.push(entry.path().to_path_buf());
    }
    paths.sort();

    for path in paths {
        if is_view_internal_path(&path) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to export symlinked View package entry: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "Unsupported View package entry type: {}",
                path.display()
            ));
        }

        let rel_path = path
            .strip_prefix(&root)
            .map_err(|error| format!("Failed to resolve View package path: {}", error))?
            .to_string_lossy()
            .replace('\\', "/");
        let rel_path = normalize_package_rel_path(&rel_path)?;
        archive
            .start_file(rel_path, options)
            .map_err(|error| zip_error("Failed to write View package archive entry", error))?;
        let mut input = std::fs::File::open(&path)
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        std::io::copy(&mut input, &mut archive)
            .map_err(|e| format!("Failed to write archive data for {}: {}", path.display(), e))?;
    }

    archive
        .finish()
        .map_err(|error| zip_error("Failed to finish View package archive", error))?;
    Ok(output_path.display().to_string().replace('\\', "/"))
}

fn zip_entry_rel_path(file: &zip::read::ZipFile<'_>) -> Result<String, String> {
    let path = file
        .enclosed_name()
        .ok_or_else(|| format!("Unsafe archive entry path: {}", file.name()))?;
    let rel_path = path.to_string_lossy().replace('\\', "/");
    normalize_package_rel_path(&rel_path)
}

fn view_archive_manifest_candidate(candidates: &[String]) -> Result<String, String> {
    if candidates.iter().any(|path| path == "view.json") {
        return Ok("view.json".to_string());
    }

    let top_level = candidates
        .iter()
        .filter(|path| {
            let mut parts = path.split('/');
            parts.next().is_some() && parts.next() == Some("view.json") && parts.next().is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    if top_level.len() == 1 {
        return Ok(top_level[0].clone());
    }

    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }

    Err("View package archive must contain one view.json manifest.".to_string())
}

fn view_archive_package_prefix(manifest_rel_path: &str) -> String {
    manifest_rel_path
        .strip_suffix("view.json")
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string()
}

fn strip_view_archive_prefix(entry_rel_path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(entry_rel_path.to_string());
    }
    if entry_rel_path == prefix {
        return Some(String::new());
    }
    entry_rel_path
        .strip_prefix(&format!("{}/", prefix))
        .map(str::to_string)
}

fn read_view_archive_manifest(
    archive: &mut zip::ZipArchive<std::fs::File>,
) -> Result<(String, ViewManifest), String> {
    let mut candidates = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| zip_error("Failed to read View package archive", error))?;
        if file.is_dir() {
            continue;
        }
        let rel_path = zip_entry_rel_path(&file)?;
        if rel_path == "view.json" || rel_path.ends_with("/view.json") {
            candidates.push(rel_path);
        }
    }

    let manifest_rel_path = view_archive_manifest_candidate(&candidates)?;
    let mut manifest_file = archive
        .by_name(&manifest_rel_path)
        .map_err(|error| zip_error("Failed to read View package manifest from archive", error))?;
    let mut raw = String::new();
    manifest_file
        .read_to_string(&mut raw)
        .map_err(|error| format!("Failed to read View package manifest: {}", error))?;
    let mut manifest: ViewManifest = serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid View manifest in archive: {}", error))?;
    normalize_view_requirements(&mut manifest);
    validate_view_manifest(&manifest)?;
    Ok((view_archive_package_prefix(&manifest_rel_path), manifest))
}

fn view_import_target_workspace_root(
    working_dir: &str,
    target_dir_rel_path: Option<&str>,
) -> Result<PathBuf, String> {
    let views_root = views_root_for_workspace(working_dir)?;
    let target_dir_rel_path = target_dir_rel_path.unwrap_or("").trim();
    if target_dir_rel_path.is_empty() {
        return Ok(views_root.join(default_view_package_name(working_dir)?));
    }

    let target_dir_rel_path = normalize_view_tree_rel_path(target_dir_rel_path, false)?;
    let target_dir = view_tree_path(&views_root, &target_dir_rel_path, false)?;
    if !target_dir.is_dir() {
        return Err(format!(
            "Target View folder not found: {}",
            target_dir_rel_path
        ));
    }
    if manifest_path(&target_dir).is_file() {
        return Err("Cannot import a View inside a View package.".to_string());
    }
    Ok(target_dir)
}

fn is_zip_entry_symlink(file: &zip::read::ZipFile<'_>) -> bool {
    const UNIX_FILE_TYPE_MASK: u32 = 0o170000;
    const UNIX_SYMLINK_TYPE: u32 = 0o120000;
    file.unix_mode()
        .map(|mode| mode & UNIX_FILE_TYPE_MASK == UNIX_SYMLINK_TYPE)
        .unwrap_or(false)
}

fn extract_view_package_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    package_prefix: &str,
    target_root: &Path,
) -> Result<(), String> {
    let mut extracted_entries = 0usize;
    let mut uncompressed_bytes = 0u64;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| zip_error("Failed to read View package archive", error))?;
        let entry_rel_path = zip_entry_rel_path(&file)?;
        let Some(package_rel_path) = strip_view_archive_prefix(&entry_rel_path, package_prefix)
        else {
            continue;
        };
        if package_rel_path.is_empty() {
            continue;
        }
        let package_rel_path = normalize_package_rel_path(&package_rel_path)?;
        if is_view_internal_path(Path::new(&package_rel_path)) {
            continue;
        }
        if is_zip_entry_symlink(&file) {
            return Err(format!(
                "Refusing to import symlinked View package entry: {}",
                entry_rel_path
            ));
        }

        extracted_entries += 1;
        if extracted_entries > VIEW_PACKAGE_ARCHIVE_MAX_ENTRIES {
            return Err("View package archive has too many entries.".to_string());
        }
        uncompressed_bytes = uncompressed_bytes.saturating_add(file.size());
        if uncompressed_bytes > VIEW_PACKAGE_ARCHIVE_MAX_UNCOMPRESSED_BYTES {
            return Err("View package archive is too large.".to_string());
        }

        let output_path = target_root.join(&package_rel_path);
        if !path_is_under_root(&output_path, target_root) {
            return Err(format!(
                "Archive entry resolves outside of View package: {}",
                entry_rel_path
            ));
        }
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .map_err(|e| format!("Failed to create {}: {}", output_path.display(), e))?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        let mut output = std::fs::File::create(&output_path)
            .map_err(|e| format!("Failed to create {}: {}", output_path.display(), e))?;
        std::io::copy(&mut file, &mut output)
            .map_err(|e| format!("Failed to extract {}: {}", entry_rel_path, e))?;
    }

    if !manifest_path(target_root).is_file() {
        return Err("View package archive did not extract a root view.json.".to_string());
    }
    Ok(())
}

pub fn import_view_package_sync(
    working_dir: &str,
    request: ViewImportPackageRequest,
) -> Result<ViewPackageImportResult, String> {
    let archive_path = PathBuf::from(request.file_path.trim());
    if archive_path.as_os_str().is_empty() {
        return Err("Import path cannot be empty.".to_string());
    }
    if !archive_path.is_file() {
        return Err(format!(
            "View package archive not found: {}",
            archive_path.display()
        ));
    }

    let archive_file = std::fs::File::open(&archive_path)
        .map_err(|e| format!("Failed to open {}: {}", archive_path.display(), e))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| zip_error("Invalid View package archive", error))?;
    let (package_prefix, manifest) = read_view_archive_manifest(&mut archive)?;

    let views_root = views_root_for_workspace(working_dir)?;
    let existing = find_view_package_roots_by_id(&views_root, &manifest.id)?;
    if !existing.is_empty() {
        return Err(format!("View package id already exists: {}", manifest.id));
    }

    let workspace_root =
        view_import_target_workspace_root(working_dir, request.target_dir_rel_path.as_deref())?;
    if manifest_path(&workspace_root).is_file() {
        return Err("Cannot import a View inside a View package.".to_string());
    }
    ensure_view_package_workspace(&workspace_root)?;
    let target_root = workspace_root.join(&manifest.id);
    if target_root.exists() {
        return Err(format!(
            "View package already exists: {}",
            target_root.display()
        ));
    }
    std::fs::create_dir_all(&target_root)
        .map_err(|e| format!("Failed to create {}: {}", target_root.display(), e))?;

    if let Err(error) = extract_view_package_archive(&mut archive, &package_prefix, &target_root) {
        let _ = std::fs::remove_dir_all(&target_root);
        return Err(error);
    }

    let imported_manifest = match load_manifest_from_root(&target_root) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&target_root);
            return Err(error);
        }
    };
    if imported_manifest.id != manifest.id {
        let _ = std::fs::remove_dir_all(&target_root);
        return Err(format!(
            "View id mismatch after import: archive has {}, extracted manifest has {}",
            manifest.id, imported_manifest.id
        ));
    }

    let summary = summary_from_manifest(&views_root, &target_root, &imported_manifest, false);
    let snapshot = list_view_tree_sync(working_dir)?;
    Ok(ViewPackageImportResult { summary, snapshot })
}

pub fn create_view_sync(
    working_dir: &str,
    request: ViewCreateRequest,
) -> Result<ViewPackageDetail, String> {
    create_view_sync_with_scope(working_dir, request, false)
}

pub fn create_view_sync_with_scope(
    working_dir: &str,
    request: ViewCreateRequest,
    temporary: bool,
) -> Result<ViewPackageDetail, String> {
    let requested_id = normalize_view_id(&request.id)?;
    let id = if temporary {
        unique_temporary_view_id(&requested_id)
    } else {
        requested_id.clone()
    };
    let template = request
        .template
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("blank");
    if !templates::is_supported_template(template) {
        return Err(format!("Unsupported View template: {}", template));
    }

    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title_from_id(&requested_id));
    let icon = request
        .icon
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(icon) = icon.as_deref() {
        validate_view_icon_name(icon)?;
    }

    let root = if temporary {
        temporary_view_package_root(working_dir, request.package_name.as_deref(), &id)?
    } else {
        view_package_root_for_request(working_dir, &request, &id)?
    };
    if !temporary {
        let views_root = views_root_for_workspace(working_dir)?;
        let existing = find_view_package_roots_by_id(&views_root, &id)?;
        if !existing.is_empty() {
            return Err(format!(
                "View package id already exists: {} at {}",
                id,
                existing
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if root.exists() {
        return Err(format!("View package already exists: {}", root.display()));
    }
    let workspace_root = root
        .parent()
        .ok_or_else(|| format!("Invalid View package root: {}", root.display()))?
        .to_path_buf();
    if !temporary {
        ensure_view_package_workspace(&workspace_root)?;
    } else {
        std::fs::create_dir_all(&workspace_root)
            .map_err(|e| format!("Failed to create {}: {}", workspace_root.display(), e))?;
    }
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create {}: {}", root.display(), e))?;

    let manifest = templates::template_manifest(&id, &name, template, icon.as_deref());
    let manifest_raw = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Failed to serialize View manifest: {}", e))?;
    write_package_file(&root, "view.json", &(manifest_raw + "\n"))?;

    for (rel_path, content) in templates::template_files(&id, &name, template) {
        write_package_file(&root, rel_path, &content)?;
    }

    read_view_sync(working_dir, &id)
}

fn ensure_view_package_workspace(workspace_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(workspace_root)
        .map_err(|e| format!("Failed to create {}: {}", workspace_root.display(), e))?;
    let src_root = workspace_root.join(VIEW_WORKSPACE_SRC_DIR);
    std::fs::create_dir_all(&src_root)
        .map_err(|e| format!("Failed to create {}: {}", src_root.display(), e))?;

    let package_json_path = workspace_root.join("package.json");
    if !package_json_path.exists() {
        std::fs::write(&package_json_path, templates::view_workspace_package_json())
            .map_err(|e| format!("Failed to write {}: {}", package_json_path.display(), e))?;
    }

    let tsconfig_path = workspace_root.join("tsconfig.json");
    if !tsconfig_path.exists() {
        std::fs::write(&tsconfig_path, templates::view_workspace_tsconfig_json())
            .map_err(|e| format!("Failed to write {}: {}", tsconfig_path.display(), e))?;
    }

    let index_path = src_root.join("index.ts");
    if !index_path.exists() {
        std::fs::write(&index_path, templates::view_workspace_index_ts())
            .map_err(|e| format!("Failed to write {}: {}", index_path.display(), e))?;
    }

    let property_draw_path = src_root.join("propertyDraw.ts");
    if !property_draw_path.exists() {
        std::fs::write(
            &property_draw_path,
            templates::view_workspace_property_draw_ts(),
        )
        .map_err(|e| format!("Failed to write {}: {}", property_draw_path.display(), e))?;
    }

    let readme_path = workspace_root.join("README.md");
    if !readme_path.exists() {
        std::fs::write(&readme_path, templates::view_workspace_readme_md())
            .map_err(|e| format!("Failed to write {}: {}", readme_path.display(), e))?;
    }

    Ok(())
}

fn unique_temporary_view_id(base_id: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-tmp-{}", base_id, &suffix[..8])
}

fn temporary_view_package_root(
    working_dir: &str,
    package_name: Option<&str>,
    id: &str,
) -> Result<PathBuf, String> {
    let id = normalize_view_id(id)?;
    let root = temporary_views_root_for_workspace(working_dir)?;
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create {}: {}", root.display(), e))?;
    let package_name = match package_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(package_name) => normalize_view_package_name(package_name)?,
        None => default_view_package_name(working_dir)?,
    };
    Ok(root.join(package_name).join(id))
}

pub fn read_view_sync(working_dir: &str, view_id: &str) -> Result<ViewPackageDetail, String> {
    let root = resolve_view_package_root(working_dir, view_id)?;
    if !root.is_dir() {
        return Err(format!("View package not found: {}", view_id));
    }
    let manifest = load_manifest_from_root(&root)?;
    if manifest.id != normalize_view_id(view_id)? {
        return Err(format!(
            "View id mismatch: requested {}, manifest has {}",
            view_id, manifest.id
        ));
    }

    let views_root = views_root_for_workspace(working_dir)?;
    let temp_root = temporary_views_root_for_workspace(working_dir)?;
    let temporary = path_is_under_root(&root, &temp_root);
    let summary_root = if temporary { &temp_root } else { &views_root };
    let summary = summary_from_manifest(summary_root, &root, &manifest, temporary);
    let workspace_root = root
        .parent()
        .ok_or_else(|| format!("Invalid View package root: {}", root.display()))?;
    let mut rel_paths = BTreeSet::new();
    rel_paths.insert("view.json".to_string());
    rel_paths.insert("README.md".to_string());
    rel_paths.insert(manifest.entry.clone());
    rel_paths.insert(manifest.style.clone());
    rel_paths.insert(manifest.bindings.clone());
    rel_paths.insert("src/App.vue".to_string());
    rel_paths.insert("src/store.ts".to_string());
    collect_view_runtime_source_paths(&root, &mut rel_paths)?;
    for script in &manifest.scripts {
        rel_paths.insert(script.path.clone());
    }

    let mut files = Vec::new();
    for rel_path in rel_paths {
        let path = package_path(&root, &rel_path)?;
        if !path.is_file() {
            continue;
        }
        let workspace_rel_path = workspace_relative_view_path(summary_root, &path)?;
        files.push(read_view_file(&path, &workspace_rel_path)?);
    }
    collect_view_package_workspace_source_files(summary_root, workspace_root, &mut files)?;

    Ok(ViewPackageDetail {
        summary,
        manifest,
        files,
    })
}

fn collect_view_runtime_source_paths(
    root: &Path,
    rel_paths: &mut BTreeSet<String>,
) -> Result<(), String> {
    let src_root = root.join("src");
    if !src_root.is_dir() {
        return Ok(());
    }

    for entry in walkdir::WalkDir::new(&src_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.file_type().is_file()
                || entry
                    .file_name()
                    .to_str()
                    .map(|name| !matches!(name, "node_modules" | ".git" | "dist" | "target"))
                    .unwrap_or(true)
        })
    {
        let entry = entry.map_err(|error| format!("Failed to scan View src: {}", error))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_view_runtime_source_file(path) {
            continue;
        }
        let rel_path = path
            .strip_prefix(root)
            .map_err(|error| format!("Failed to resolve View source path: {}", error))?
            .to_string_lossy()
            .replace('\\', "/");
        rel_paths.insert(normalize_package_rel_path(&rel_path)?);
    }

    Ok(())
}

fn collect_view_package_workspace_source_files(
    root_base: &Path,
    workspace_root: &Path,
    files: &mut Vec<ViewPackageFile>,
) -> Result<(), String> {
    let src_root = workspace_root.join(VIEW_WORKSPACE_SRC_DIR);
    if !src_root.is_dir() {
        return Ok(());
    }

    for entry in walkdir::WalkDir::new(&src_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.file_type().is_file()
                || entry
                    .file_name()
                    .to_str()
                    .map(|name| !matches!(name, "node_modules" | ".git" | "dist" | "target"))
                    .unwrap_or(true)
        })
    {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to scan View package workspace source {}: {}",
                src_root.display(),
                error
            )
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_view_runtime_source_file(path) {
            continue;
        }
        let rel_path = workspace_relative_view_path(root_base, path)?;
        files.push(read_view_file(path, &rel_path)?);
    }

    Ok(())
}

fn workspace_relative_view_path(root_base: &Path, path: &Path) -> Result<String, String> {
    let rel_path = path
        .strip_prefix(root_base)
        .map_err(|error| format!("Failed to resolve View workspace path: {}", error))?
        .to_string_lossy()
        .replace('\\', "/");
    normalize_package_rel_path(&rel_path)
}

fn is_view_runtime_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "vue" | "ts" | "js" | "css" | "json"
            )
        })
        .unwrap_or(false)
}

pub fn reload_view_sync(working_dir: &str, view_id: &str) -> Result<ViewPackageSummary, String> {
    let detail = read_view_sync(working_dir, view_id)?;
    Ok(detail.summary)
}

fn view_tab_hosts() -> &'static Mutex<HashMap<String, String>> {
    static VIEW_TAB_HOSTS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    VIEW_TAB_HOSTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sanitize_view_host_label(label: &str) -> Result<String, String> {
    let normalized = label.trim();
    if !normalized.starts_with(VIEW_WINDOW_LABEL_PREFIX)
        || normalized.len() <= VIEW_WINDOW_LABEL_PREFIX.len()
    {
        return Err(format!("Invalid View host window label: {}", label));
    }
    Ok(normalized.to_string())
}

pub fn set_view_tab_host_sync(request: ViewSetTabHostRequest) -> Result<(), String> {
    let host_label = sanitize_view_host_label(&request.host_label)?;
    let mut view_ids = Vec::new();
    for view_id in request.view_ids {
        let normalized = normalize_view_id(&view_id)?;
        if !view_ids.contains(&normalized) {
            view_ids.push(normalized);
        }
    }
    if view_ids.is_empty() {
        return Err("View tab host must contain at least one View id.".to_string());
    }

    let mut hosts = view_tab_hosts()
        .lock()
        .map_err(|_| "View tab host registry is unavailable".to_string())?;
    hosts.retain(|view_id, label| label != &host_label && !view_ids.contains(view_id));
    for view_id in view_ids {
        hosts.insert(view_id, host_label.clone());
    }
    Ok(())
}

fn registered_view_host_label(view_id: &str) -> Option<String> {
    view_tab_hosts()
        .lock()
        .ok()
        .and_then(|hosts| hosts.get(view_id).cloned())
}

fn clear_registered_view_host(view_id: &str) {
    if let Ok(mut hosts) = view_tab_hosts().lock() {
        hosts.remove(view_id);
    }
}

fn active_view_window_label(app_handle: &AppHandle, view_id: &str) -> String {
    let default_label = view_window_label(view_id);
    let Some(host_label) = registered_view_host_label(view_id) else {
        return default_label;
    };
    if app_handle.get_webview_window(&host_label).is_some() {
        return host_label;
    }
    clear_registered_view_host(view_id);
    default_label
}

fn emit_view_host_tab_select(app_handle: &AppHandle, window_label: &str, view_id: &str) {
    if let Some(window) = app_handle.get_webview_window(window_label) {
        let _ = window.emit(
            VIEW_HOST_TABS_SELECT_EVENT,
            serde_json::json!({ "viewId": view_id }),
        );
    }
}

fn detached_view_window_label(view_id: &str) -> String {
    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>();
    format!("{}{}-{}", VIEW_WINDOW_LABEL_PREFIX, view_id, suffix)
}

fn build_view_window(
    app_handle: &AppHandle,
    label: &str,
    host_url: &str,
    title: &str,
    position: Option<(f64, f64)>,
    view_windows_above_main: bool,
) -> Result<(), String> {
    let builder = tauri::WebviewWindowBuilder::new(
        app_handle,
        label,
        WebviewUrl::App(host_url.to_string().into()),
    )
    .title(title.to_string());
    let main_window = if view_windows_above_main {
        app_handle.get_webview_window(MAIN_WINDOW_LABEL)
    } else {
        None
    };
    let builder = if let Some(main_window) = main_window {
        builder
            .parent(&main_window)
            .map_err(|e| format!("Failed to attach View window to main window: {}", e))?
    } else {
        builder
    };
    let builder = if let Some((x, y)) = position {
        builder.position(x, y)
    } else {
        builder
    };

    builder
        .inner_size(1180.0, 760.0)
        .min_inner_size(760.0, 480.0)
        .decorations(false)
        .resizable(true)
        .visible(true)
        .disable_drag_drop_handler()
        .build()
        .map(|_| ())
        .map_err(|e| format!("Failed to open View window: {}", e))
}

pub async fn open_view_window(
    app_handle: &AppHandle,
    working_dir: &str,
    view_id: &str,
    view_windows_above_main: bool,
) -> Result<ViewRunResult, String> {
    let detail = read_view_sync(working_dir, view_id)?;
    ensure_view_open_requirements(working_dir, &detail.manifest).await?;
    let id = detail.summary.id.clone();
    let label = view_window_label(&id);
    let host_url = format!("{}?id={}", VIEW_HOST_ROUTE, id);

    if let Some(host_label) = registered_view_host_label(&id) {
        if host_label != label {
            if let Some(window) = app_handle.get_webview_window(&host_label) {
                emit_view_host_tab_select(app_handle, &host_label, &id);
                window
                    .set_focus()
                    .map_err(|e| format!("Failed to focus View window: {}", e))?;
                if let Err(error) = start_view_file_watcher(app_handle, working_dir, &id) {
                    eprintln!(
                        "[Locus] failed to watch View package '{}' for reload: {}",
                        id, error
                    );
                }
                return Ok(ViewRunResult {
                    id,
                    window_label: host_label,
                    host_url,
                    package_root: detail.summary.package_root,
                });
            }
            clear_registered_view_host(&id);
        }
    }

    if let Some(window) = app_handle.get_webview_window(&label) {
        emit_view_host_tab_select(app_handle, &label, &id);
        window
            .set_focus()
            .map_err(|e| format!("Failed to focus View window: {}", e))?;
    } else {
        build_view_window(
            app_handle,
            &label,
            &host_url,
            &format!("{} - Locus View", detail.summary.name),
            None,
            view_windows_above_main,
        )?;
    }

    let _ = set_view_tab_host_sync(ViewSetTabHostRequest {
        host_label: label.clone(),
        view_ids: vec![id.clone()],
    });

    if let Err(error) = start_view_file_watcher(app_handle, working_dir, &id) {
        eprintln!(
            "[Locus] failed to watch View package '{}' for reload: {}",
            id, error
        );
    }

    Ok(ViewRunResult {
        id,
        window_label: label,
        host_url,
        package_root: detail.summary.package_root,
    })
}

pub async fn detach_view_tab_window(
    app_handle: &AppHandle,
    working_dir: &str,
    request: ViewDetachTabRequest,
    view_windows_above_main: bool,
) -> Result<ViewRunResult, String> {
    let detail = read_view_sync(working_dir, &request.view_id)?;
    ensure_view_open_requirements(working_dir, &detail.manifest).await?;
    let id = detail.summary.id.clone();
    let default_label = view_window_label(&id);
    let source_label = request
        .source_host_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let label = if source_label == default_label {
        detached_view_window_label(&id)
    } else {
        default_label
    };
    let host_url = format!("{}?id={}", VIEW_HOST_ROUTE, id);
    let position = match (request.x, request.y) {
        (Some(x), Some(y)) => Some((x.max(0.0), y.max(0.0))),
        _ => None,
    };

    if let Some(window) = app_handle.get_webview_window(&label) {
        emit_view_host_tab_select(app_handle, &label, &id);
        window
            .set_focus()
            .map_err(|e| format!("Failed to focus View window: {}", e))?;
    } else {
        build_view_window(
            app_handle,
            &label,
            &host_url,
            &format!("{} - Locus View", detail.summary.name),
            position,
            view_windows_above_main,
        )?;
    }

    let _ = set_view_tab_host_sync(ViewSetTabHostRequest {
        host_label: label.clone(),
        view_ids: vec![id.clone()],
    });

    if let Err(error) = start_view_file_watcher(app_handle, working_dir, &id) {
        eprintln!(
            "[Locus] failed to watch View package '{}' for reload: {}",
            id, error
        );
    }

    Ok(ViewRunResult {
        id,
        window_label: label,
        host_url,
        package_root: detail.summary.package_root,
    })
}

async fn ensure_view_open_requirements(
    working_dir: &str,
    manifest: &ViewManifest,
) -> Result<(), String> {
    if !view_manifest_requirements(manifest).unity_connection {
        return Ok(());
    }

    let status = crate::unity_bridge::query_unity_connection_status(working_dir).await;
    if status.connected {
        return Ok(());
    }

    Err(format!(
        "View '{}' requires a Unity Editor connection.",
        manifest.name
    ))
}

pub fn view_window_label(view_id: &str) -> String {
    format!("view-{}", view_id)
}

pub async fn request_view_automation(
    app_handle: &AppHandle,
    view_id: &str,
    kind: &str,
    payload: serde_json::Value,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    let label = active_view_window_label(app_handle, view_id);
    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| format!("View '{}' is not open. Use view_run first.", view_id))?;
    emit_view_host_tab_select(app_handle, &label, view_id);
    let store = app_handle.state::<std::sync::Arc<ViewAutomationStore>>();
    let request_id = format!("view-auto-{}", uuid::Uuid::new_v4());
    let (tx, rx) = tokio::sync::oneshot::channel();
    store.insert(request_id.clone(), tx)?;
    let event = ViewAutomationRequestEvent {
        request_id: request_id.clone(),
        view_id: view_id.to_string(),
        kind: kind.to_string(),
        payload,
    };

    let timeout = Duration::from_millis(timeout_ms.clamp(250, 60_000));
    let retry_interval = Duration::from_millis(200);
    let started_at = Instant::now();
    let mut rx = rx;
    let reply = loop {
        if let Err(error) = window.emit(VIEW_AUTOMATION_REQUEST_EVENT, event.clone()) {
            store.cancel(&request_id);
            return Err(format!("Failed to send View automation request: {}", error));
        }

        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            store.cancel(&request_id);
            return Err(format!(
                "View automation request timed out after {} ms",
                timeout.as_millis(),
            ));
        }

        let wait_for = std::cmp::min(timeout - elapsed, retry_interval);
        match tokio::time::timeout(wait_for, &mut rx).await {
            Ok(Ok(reply)) => break reply,
            Ok(Err(_)) => {
                store.cancel(&request_id);
                return Err("View automation response channel closed".to_string());
            }
            Err(_) => continue,
        }
    };

    if reply.ok {
        Ok(reply.result.unwrap_or_else(|| serde_json::json!({})))
    } else {
        Err(reply
            .error
            .unwrap_or_else(|| "View automation request failed".to_string()))
    }
}

pub fn complete_view_automation_request(
    store: &ViewAutomationStore,
    request_id: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
) -> bool {
    store.complete(&request_id, ViewAutomationReply { ok, result, error })
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != PNG_SIGNATURE {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

#[cfg(target_os = "windows")]
pub async fn capture_view_window(
    app_handle: &AppHandle,
    view_id: &str,
) -> Result<ViewCaptureResult, String> {
    use base64::Engine as _;
    use webview2_com::{
        CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::ICoreWebView2,
    };

    let label = active_view_window_label(app_handle, view_id);
    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| format!("View '{}' is not open. Use view_run first.", view_id))?;
    emit_view_host_tab_select(app_handle, &label, view_id);
    let _ = request_view_automation(
        app_handle,
        view_id,
        "wait",
        serde_json::json!({
            "condition": "runtimeReady",
            "timeoutMs": 3000,
        }),
        3500,
    )
    .await;
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));

    window
        .with_webview(move |webview| {
            let controller = webview.controller();
            let core: ICoreWebView2 = match unsafe { controller.CoreWebView2() } {
                Ok(core) => core,
                Err(error) => {
                    if let Ok(mut guard) = tx.lock() {
                        if let Some(tx) = guard.take() {
                            let _ =
                                tx.send(Err(format!("Failed to access WebView2 core: {}", error)));
                        }
                    }
                    return;
                }
            };
            let method = CoTaskMemPWSTR::from("Page.captureScreenshot");
            let params = CoTaskMemPWSTR::from(
                serde_json::json!({
                    "format": "png",
                    "fromSurface": true,
                    "captureBeyondViewport": false
                })
                .to_string()
                .as_str(),
            );
            let handler_tx = std::sync::Arc::clone(&tx);
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |error_code, result_json| {
                    if let Ok(mut guard) = handler_tx.lock() {
                        let Some(tx) = guard.take() else {
                            return Ok(());
                        };
                        let result = match error_code {
                            Ok(()) => Ok(result_json),
                            Err(error) => {
                                Err(format!("WebView2 captureScreenshot failed: {}", error))
                            }
                        };
                        let _ = tx.send(result);
                    }
                    Ok(())
                },
            ));
            if let Err(error) = unsafe {
                core.CallDevToolsProtocolMethod(
                    *method.as_ref().as_pcwstr(),
                    *params.as_ref().as_pcwstr(),
                    &handler,
                )
            } {
                if let Ok(mut guard) = tx.lock() {
                    if let Some(tx) = guard.take() {
                        let _ =
                            tx.send(Err(format!("Failed to request View screenshot: {}", error)));
                    }
                }
            }
        })
        .map_err(|error| format!("Failed to access View webview: {}", error))?;

    let result_json = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| "View screenshot timed out after 10000 ms".to_string())?
        .map_err(|_| "View screenshot response channel closed".to_string())??;
    let payload = serde_json::from_str::<serde_json::Value>(&result_json)
        .map_err(|error| format!("Invalid screenshot response: {}", error))?;
    let data = payload
        .get("data")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Screenshot response did not include image data".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("Failed to decode screenshot PNG: {}", error))?;
    let dimensions = png_dimensions(&bytes);
    Ok(ViewCaptureResult {
        view_id: view_id.to_string(),
        window_label: label,
        mime_type: "image/png".to_string(),
        format: "png".to_string(),
        width: dimensions.map(|item| item.0),
        height: dimensions.map(|item| item.1),
        byte_size: bytes.len(),
        bytes,
    })
}

#[cfg(not(target_os = "windows"))]
pub async fn capture_view_window(
    _app_handle: &AppHandle,
    _view_id: &str,
) -> Result<ViewCaptureResult, String> {
    Err("view_capture currently requires the Windows WebView2 runtime.".to_string())
}

pub fn emit_view_reload(app_handle: &AppHandle, summary: &ViewPackageSummary) {
    let _ = app_handle.emit(VIEW_RELOAD_EVENT, summary);
    emit_view_tree_changed(app_handle);
}

pub fn emit_view_tree_changed(app_handle: &AppHandle) {
    let _ = app_handle.emit(VIEW_TREE_CHANGED_EVENT, serde_json::json!({}));
}

fn view_file_watcher_keys() -> &'static Mutex<BTreeSet<String>> {
    static WATCHERS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    WATCHERS.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn should_reload_for_view_event(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
        return false;
    }

    if !event.paths.is_empty() && event.paths.iter().all(|path| is_view_internal_path(path)) {
        return false;
    }

    true
}

fn is_view_internal_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".locus")
    })
}

fn canonical_view_watch_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn view_file_watch_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = canonical_view_watch_path(root);
    let mut roots = vec![root.clone()];
    let workspace_src_root = root
        .parent()
        .ok_or_else(|| format!("Invalid View package root: {}", root.display()))?
        .join(VIEW_WORKSPACE_SRC_DIR);
    if workspace_src_root.is_dir() {
        let workspace_src_root = canonical_view_watch_path(&workspace_src_root);
        if !roots.iter().any(|path| path == &workspace_src_root) {
            roots.push(workspace_src_root);
        }
    }
    Ok(roots)
}

pub fn is_view_frontend_log_workspace_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let view_root = VIEW_ROOT_RELATIVE.to_ascii_lowercase();
    let log_rel_path = VIEW_FRONTEND_LOG_REL_PATH.to_ascii_lowercase();

    lower.starts_with(&(view_root + "/")) && lower.ends_with(&format!("/{}", log_rel_path))
}

fn start_view_file_watcher(
    app_handle: &AppHandle,
    working_dir: &str,
    view_id: &str,
) -> Result<(), String> {
    let root = resolve_view_package_root(working_dir, view_id)?;
    let roots = view_file_watch_roots(&root)?;
    let key = roots
        .iter()
        .map(|root| root.display().to_string().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join("|");
    {
        let mut keys = view_file_watcher_keys()
            .lock()
            .map_err(|_| "View file watcher registry is poisoned.".to_string())?;
        if !keys.insert(key.clone()) {
            return Ok(());
        }
    }

    let app_handle = app_handle.clone();
    let working_dir = working_dir.to_string();
    let view_id = view_id.to_string();
    let thread_name = format!("locus-view-watch-{}", view_id);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = match RecommendedWatcher::new(
        move |result| {
            let _ = tx.send(result);
        },
        Config::default(),
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            remove_view_file_watcher_key(&key);
            return Err(format!("Failed to create watcher: {}", error));
        }
    };

    for root in &roots {
        if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
            remove_view_file_watcher_key(&key);
            return Err(format!("Failed to watch {}: {}", root.display(), error));
        }
    }

    let key_for_thread = key.clone();
    match std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let _watcher = watcher;
            let mut pending = false;
            let mut last_event_at = Instant::now();
            loop {
                match rx.recv_timeout(Duration::from_millis(160)) {
                    Ok(Ok(event)) => {
                        if should_reload_for_view_event(&event) {
                            pending = true;
                            last_event_at = Instant::now();
                        }
                    }
                    Ok(Err(error)) => {
                        eprintln!("[Locus] View file watcher error: {}", error);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if pending && last_event_at.elapsed() >= Duration::from_millis(150) {
                            pending = false;
                            match reload_view_sync(&working_dir, &view_id) {
                                Ok(summary) => emit_view_reload(&app_handle, &summary),
                                Err(error) => eprintln!(
                                    "[Locus] failed to reload watched View '{}': {}",
                                    view_id, error
                                ),
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        remove_view_file_watcher_key(&key_for_thread);
                        break;
                    }
                }
            }
        }) {
        Ok(_) => Ok(()),
        Err(error) => {
            remove_view_file_watcher_key(&key);
            Err(format!("Failed to spawn watcher thread: {}", error))
        }
    }
}

fn remove_view_file_watcher_key(key: &str) {
    if let Ok(mut keys) = view_file_watcher_keys().lock() {
        keys.remove(key);
    }
}

fn view_script_source_cache() -> &'static Mutex<HashMap<String, CachedViewScriptSource>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedViewScriptSource>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn compile_view_script(
    working_dir: &str,
    request: ViewCompileScriptRequest,
) -> Result<ViewCompileScriptResult, String> {
    let resolved = resolve_view_script_sync(working_dir, &request.view_id, &request.script_name)?;
    let payload = view_script_bridge_payload(&resolved, None)?;
    let raw = crate::unity_bridge::compile_named(working_dir, &payload).await?;
    parse_view_compile_result(&raw, &resolved.path)
}

pub async fn call_view_script(
    working_dir: &str,
    request: ViewCallScriptRequest,
) -> Result<ViewCallScriptResult, String> {
    let method = request.method.trim();
    if method.is_empty() {
        return Err("View script method cannot be empty.".to_string());
    }
    let resolved = resolve_view_script_sync(working_dir, &request.view_id, &request.script_name)?;
    let args = request.args.unwrap_or_else(|| serde_json::json!({}));
    let cached_payload = view_script_cached_invoke_payload(&resolved, method, &args)?;
    let raw = match crate::unity_bridge::invoke_named_cached(working_dir, &cached_payload).await {
        Ok(raw) => raw,
        Err(error) if is_view_script_compile_required(&error) => {
            let payload = view_script_bridge_payload(&resolved, Some((method, &args)))?;
            crate::unity_bridge::invoke_named(working_dir, &payload).await?
        }
        Err(error) => return Err(error),
    };
    parse_view_call_result(&raw, &resolved.path)
}

pub fn append_view_frontend_log_sync(
    working_dir: &str,
    request: ViewFrontendLogRequest,
) -> Result<(), String> {
    let log_path = frontend_log_path_for_view(working_dir, &request.view_id)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
    }

    let entry = serde_json::json!({
        "time": now_millis(),
        "level": normalize_frontend_log_level(&request.level),
        "message": truncate_frontend_log_message(&request.message),
    });
    let mut line = serde_json::to_string(&entry)
        .map_err(|error| format!("Failed to serialize View frontend log entry: {}", error))?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("Failed to open {}: {}", log_path.display(), error))?;
    file.write_all(line.as_bytes())
        .map_err(|error| format!("Failed to write {}: {}", log_path.display(), error))
}

pub fn read_view_frontend_log_sync(
    working_dir: &str,
    request: ViewFrontendLogReadRequest,
) -> Result<Vec<ViewFrontendLogEntry>, String> {
    let log_path = frontend_log_path_for_view(working_dir, &request.view_id)?;
    if !log_path.is_file() {
        return Ok(Vec::new());
    }

    let raw = std::fs::read_to_string(&log_path)
        .map_err(|error| format!("Failed to read {}: {}", log_path.display(), error))?;
    let limit = request.limit.unwrap_or(20).clamp(1, 200);
    let mut entries = Vec::new();
    for line in raw.lines().rev() {
        if entries.len() >= limit {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<ViewFrontendLogEntry>(line) {
            entries.push(entry);
        }
    }
    entries.reverse();
    Ok(entries)
}

pub fn open_view_frontend_log_sync(working_dir: &str, view_id: &str) -> Result<(), String> {
    let log_path = frontend_log_path_for_view(working_dir, view_id)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
    }
    if !log_path.exists() {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| format!("Failed to create {}: {}", log_path.display(), error))?;
    }
    crate::commands::open_file_native(&log_path)
}

fn frontend_log_path_for_view(working_dir: &str, view_id: &str) -> Result<PathBuf, String> {
    let root = resolve_view_package_root(working_dir, view_id)?;
    if !root.is_dir() || !manifest_path(&root).is_file() {
        return Err(format!("View package not found: {}", view_id));
    }
    load_manifest_from_root(&root)?;
    package_path(&root, VIEW_FRONTEND_LOG_REL_PATH)
}

pub async fn view_binding_read(
    working_dir: &str,
    request: ViewBindingReadRequest,
) -> Result<ViewBindingReadResult, String> {
    let binding = resolve_view_binding(
        working_dir,
        &request.view_id,
        request.binding_id.as_deref(),
        request.target,
    )?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": binding.target,
    });
    let raw = crate::unity_bridge::view_binding_read(working_dir, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid view_binding_read response: {}", error))
}

pub async fn view_binding_discover(
    working_dir: &str,
    request: ViewBindingDiscoverRequest,
) -> Result<ViewBindingDiscoverResult, String> {
    let ViewBindingDiscoverRequest {
        view_id,
        binding_id,
        target,
        query,
        field_name,
        field_type,
        max_depth,
        max_results,
    } = request;
    let target = if let Some(target) = target {
        validate_view_binding_object_target(&target)?;
        target
    } else {
        resolve_view_binding(working_dir, &view_id, binding_id.as_deref(), None)?.target
    };
    let payload = serde_json::json!({
        "bindingId": binding_id,
        "target": target,
        "query": query.unwrap_or_default(),
        "fieldName": field_name.unwrap_or_default(),
        "fieldType": field_type.unwrap_or_default(),
        "maxDepth": max_depth.unwrap_or_default(),
        "maxResults": max_results.unwrap_or_default(),
    });
    let raw = crate::unity_bridge::view_binding_discover(working_dir, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid view_binding_discover response: {}", error))
}

pub async fn view_binding_write(
    working_dir: &str,
    request: ViewBindingWriteRequest,
) -> Result<ViewBindingWriteResult, String> {
    let binding = resolve_view_binding(
        working_dir,
        &request.view_id,
        request.binding_id.as_deref(),
        request.target,
    )?;
    ensure_view_binding_write_allowed(binding.mode.as_deref())?;
    let value_json = serde_json::to_string(&request.value)
        .map_err(|error| format!("Failed to serialize binding value: {}", error))?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": binding.target,
        "valueJson": value_json,
    });
    let raw = crate::unity_bridge::view_binding_write(working_dir, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid view_binding_write response: {}", error))
}

pub async fn view_binding_apply(
    working_dir: &str,
    request: ViewBindingApplyRequest,
) -> Result<ViewBindingApplyResult, String> {
    let mut writes = Vec::with_capacity(request.writes.len());
    let loaded_bindings = if request.writes.iter().any(|write| write.target.is_none()) {
        Some(load_view_bindings(working_dir, &request.view_id)?)
    } else {
        None
    };
    for write in request.writes {
        let binding = match write.target {
            Some(target) => {
                validate_view_binding_target(&target)?;
                ResolvedViewBinding { target, mode: None }
            }
            None => resolve_view_binding_from_loaded(
                loaded_bindings
                    .as_ref()
                    .ok_or_else(|| "View bindings were not loaded.".to_string())?,
                write.binding_id.as_deref(),
            )?,
        };
        ensure_view_binding_write_allowed(binding.mode.as_deref())?;
        writes.push(serde_json::json!({
            "bindingId": write.binding_id,
            "target": binding.target,
            "valueJson": serde_json::to_string(&write.value)
                .map_err(|error| format!("Failed to serialize binding value: {}", error))?,
        }));
    }

    let payload = serde_json::json!({ "writes": writes });
    let raw = crate::unity_bridge::view_binding_apply(working_dir, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid view_binding_apply response: {}", error))
}

fn resolve_view_binding_target(
    working_dir: &str,
    view_id: &str,
    binding_id: Option<&str>,
    target: Option<ViewBindingTarget>,
) -> Result<ViewBindingTarget, String> {
    Ok(resolve_view_binding(working_dir, view_id, binding_id, target)?.target)
}

fn resolve_view_binding(
    working_dir: &str,
    view_id: &str,
    binding_id: Option<&str>,
    target: Option<ViewBindingTarget>,
) -> Result<ResolvedViewBinding, String> {
    if let Some(target) = target {
        validate_view_binding_target(&target)?;
        return Ok(ResolvedViewBinding { target, mode: None });
    }

    let loaded = load_view_bindings(working_dir, view_id)?;
    resolve_view_binding_from_loaded(&loaded, binding_id)
}

fn load_view_bindings(working_dir: &str, view_id: &str) -> Result<LoadedViewBindings, String> {
    let root = resolve_view_package_root(working_dir, view_id)?;
    let manifest = load_manifest_from_root(&root)?;
    let bindings_path = package_path(&root, &manifest.bindings)?;
    let raw = std::fs::read_to_string(&bindings_path)
        .map_err(|error| format!("Failed to read {}: {}", bindings_path.display(), error))?;
    let bindings: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Invalid bindings.json {}: {}",
            bindings_path.display(),
            error
        )
    })?;
    let by_id = bindings
        .get("bindings")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|id| (id.to_string(), item.clone()))
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(LoadedViewBindings { by_id })
}

fn resolve_view_binding_from_loaded(
    loaded: &LoadedViewBindings,
    binding_id: Option<&str>,
) -> Result<ResolvedViewBinding, String> {
    let binding_id = binding_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "View binding request requires target or bindingId.".to_string())?;
    let binding = loaded
        .by_id
        .get(binding_id)
        .ok_or_else(|| format!("View binding not found: {}", binding_id))?;
    let target_value = binding
        .get("target")
        .cloned()
        .ok_or_else(|| format!("View binding has no target: {}", binding_id))?;
    let target: ViewBindingTarget = serde_json::from_value(target_value)
        .map_err(|error| format!("Invalid target for binding {}: {}", binding_id, error))?;
    validate_view_binding_target(&target)?;
    let mode = binding
        .get("mode")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(ResolvedViewBinding { target, mode })
}

fn ensure_view_binding_write_allowed(mode: Option<&str>) -> Result<(), String> {
    if matches!(mode.map(|value| value.to_ascii_lowercase()), Some(value) if value == "readonly") {
        return Err("View binding is readOnly and cannot be written.".to_string());
    }
    Ok(())
}

fn validate_view_binding_target(target: &ViewBindingTarget) -> Result<(), String> {
    validate_view_binding_object_target(target)?;
    if target
        .property_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err("View binding target propertyPath is required.".to_string());
    }
    Ok(())
}

fn validate_view_binding_object_target(target: &ViewBindingTarget) -> Result<(), String> {
    if target.kind.trim().is_empty() {
        return Err("View binding target kind cannot be empty.".to_string());
    }
    if matches!(target.component_index, Some(index) if index < 0) {
        return Err("View binding target componentIndex cannot be negative.".to_string());
    }
    for path in [
        target.path.as_deref(),
        target.scene_path.as_deref(),
        target.object_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if path.contains('\0') {
            return Err("View binding target path contains an invalid character.".to_string());
        }
    }
    Ok(())
}

fn resolve_view_script_sync(
    working_dir: &str,
    view_id: &str,
    script_name: &str,
) -> Result<ResolvedViewScript, String> {
    let script_name = script_name.trim();
    if script_name.is_empty() {
        return Err("View script name cannot be empty.".to_string());
    }

    let root = resolve_view_package_root(working_dir, view_id)?;
    let manifest = load_manifest_from_root(&root)?;
    let script = manifest
        .scripts
        .iter()
        .find(|candidate| candidate.name == script_name)
        .ok_or_else(|| format!("View script not found: {}", script_name))?;
    let path = normalize_package_rel_path(&script.path)?;
    let source_path = package_path(&root, &path)?;
    let metadata = source_path
        .metadata()
        .map_err(|error| format!("Failed to stat {}: {}", source_path.display(), error))?;
    let modified = metadata.modified().ok();
    let len = metadata.len();
    let cache_key = format!(
        "{}|{}|{}|{}|{}",
        root.display().to_string().replace('\\', "/"),
        manifest.id,
        script.name,
        script.entry_type,
        path
    );
    if let Ok(cache) = view_script_source_cache().lock() {
        if let Some(cached) = cache.get(&cache_key) {
            if cached.modified == modified && cached.len == len {
                return Ok(cached.resolved.clone());
            }
        }
    }

    let source = std::fs::read_to_string(&source_path)
        .map_err(|error| format!("Failed to read {}: {}", source_path.display(), error))?;
    let source_hash = blake3::hash(source.as_bytes()).to_hex().to_string();

    let resolved = ResolvedViewScript {
        view_id: manifest.id,
        script_name: script.name.clone(),
        path,
        entry_type: script.entry_type.clone(),
        source,
        source_hash,
    };

    if let Ok(mut cache) = view_script_source_cache().lock() {
        cache.insert(
            cache_key,
            CachedViewScriptSource {
                modified,
                len,
                resolved: resolved.clone(),
            },
        );
    }

    Ok(resolved)
}

fn view_script_bridge_payload(
    resolved: &ResolvedViewScript,
    invocation: Option<(&str, &serde_json::Value)>,
) -> Result<serde_json::Value, String> {
    let mut payload = serde_json::json!({
        "viewId": resolved.view_id,
        "scriptName": resolved.script_name,
        "entryType": resolved.entry_type,
        "source": resolved.source,
        "sourceHash": resolved.source_hash,
        "path": resolved.path,
    });

    if let Some((method, args)) = invocation {
        payload["method"] = serde_json::Value::String(method.to_string());
        payload["argsJson"] = serde_json::Value::String(
            serde_json::to_string(args)
                .map_err(|error| format!("Failed to serialize View script args: {}", error))?,
        );
    }

    Ok(payload)
}

fn view_script_cached_invoke_payload(
    resolved: &ResolvedViewScript,
    method: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "viewId": resolved.view_id,
        "scriptName": resolved.script_name,
        "entryType": resolved.entry_type,
        "sourceHash": resolved.source_hash,
        "path": resolved.path,
        "method": method,
        "argsJson": serde_json::to_string(args)
            .map_err(|error| format!("Failed to serialize View script args: {}", error))?,
    }))
}

fn is_view_script_compile_required(error: &str) -> bool {
    error.trim().starts_with("compile_required:") || error.trim() == "compile_required"
}

fn normalize_frontend_log_level(level: &str) -> &'static str {
    match level.trim() {
        "debug" => "debug",
        "info" => "info",
        "warn" => "warn",
        "error" => "error",
        _ => "log",
    }
}

fn truncate_frontend_log_message(message: &str) -> String {
    if message.chars().count() <= VIEW_FRONTEND_LOG_MAX_CHARS {
        return message.to_string();
    }
    let mut truncated = message
        .chars()
        .take(VIEW_FRONTEND_LOG_MAX_CHARS)
        .collect::<String>();
    truncated.push_str("... (truncated)");
    truncated
}

fn parse_view_compile_result(raw: &str, path: &str) -> Result<ViewCompileScriptResult, String> {
    let mut result: ViewCompileScriptResult = serde_json::from_str(raw)
        .map_err(|error| format!("Invalid compile_named response: {}", error))?;
    result.path = path.to_string();
    Ok(result)
}

fn parse_view_call_result(raw: &str, path: &str) -> Result<ViewCallScriptResult, String> {
    let mut result: ViewCallScriptResult = serde_json::from_str(raw)
        .map_err(|error| format!("Invalid invoke_named response: {}", error))?;
    result.compile.path = path.to_string();
    Ok(result)
}

fn write_package_file(root: &Path, rel_path: &str, content: &str) -> Result<(), String> {
    let path = package_path(root, rel_path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn read_view_file(path: &Path, rel_path: &str) -> Result<ViewPackageFile, String> {
    let metadata = path
        .metadata()
        .map_err(|e| format!("Failed to stat {}: {}", path.display(), e))?;
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let max_bytes = 96 * 1024;
    let truncated = bytes.len() > max_bytes;
    let slice = if truncated {
        &bytes[..max_bytes]
    } else {
        bytes.as_slice()
    };
    let content = String::from_utf8_lossy(slice).replace("\r\n", "\n");
    Ok(ViewPackageFile {
        rel_path: normalize_package_rel_path(rel_path)?,
        kind: package_file_kind(rel_path),
        content,
        size: metadata.len(),
        truncated,
    })
}

fn package_file_kind(rel_path: &str) -> String {
    if rel_path == "view.json" || rel_path.ends_with("/view.json") {
        "manifest"
    } else if rel_path.ends_with(".vue") || rel_path.ends_with(".ts") {
        "source"
    } else if rel_path.ends_with(".css") {
        "style"
    } else if rel_path.ends_with(".json") {
        "data"
    } else if rel_path.ends_with(".cs") {
        "script"
    } else {
        "document"
    }
    .to_string()
}

fn title_from_id(id: &str) -> String {
    id.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        append_view_frontend_log_sync, create_view_folder_sync, create_view_sync,
        default_view_package_name, delete_view_entry_sync, ensure_view_binding_write_allowed,
        export_view_package_sync, import_view_package_sync, is_valid_view_id,
        is_view_frontend_log_workspace_path, list_view_tree_sync, list_views_sync,
        move_view_entry_sync, normalize_package_rel_path, parse_view_create_request,
        read_view_frontend_log_sync, read_view_sync, resolve_view_binding_target,
        resolve_view_script_sync, should_reload_for_view_event, supported_view_templates,
        validate_view_binding_object_target, validate_view_binding_target, validate_view_manifest,
        view_file_watch_roots, view_manifest_requirements, view_package_root,
        view_script_bridge_payload, view_script_cached_invoke_payload, ViewBindingDiscoverResult,
        ViewBindingTarget, ViewBindingWriteResult, ViewExportPackageRequest,
        ViewFrontendLogReadRequest, ViewFrontendLogRequest, ViewImportPackageRequest, ViewManifest,
        VIEW_BINDINGS_SCHEMA, VIEW_ROOT_RELATIVE, VIEW_SCHEMA,
    };
    use notify::{
        event::{DataChange, ModifyKind},
        Event, EventKind,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn default_test_view_package_root(working_dir: &str) -> PathBuf {
        PathBuf::from(working_dir)
            .join(VIEW_ROOT_RELATIVE)
            .join(default_view_package_name(working_dir).expect("default package name"))
    }

    #[test]
    fn view_id_validation_uses_kebab_case() {
        assert!(is_valid_view_id("material-inspector"));
        assert!(is_valid_view_id("view-2"));
        assert!(!is_valid_view_id("MaterialInspector"));
        assert!(!is_valid_view_id("material_inspector"));
        assert!(!is_valid_view_id("-material"));
        assert!(!is_valid_view_id("material-"));
        assert!(!is_valid_view_id("material--inspector"));
    }

    #[test]
    fn package_relative_path_rejects_escapes_and_absolute_paths() {
        assert_eq!(
            normalize_package_rel_path("src\\App.vue").unwrap(),
            "src/App.vue"
        );
        assert!(normalize_package_rel_path("../App.vue").is_err());
        assert!(normalize_package_rel_path("src/../App.vue").is_err());
        assert!(normalize_package_rel_path("F:/App.vue").is_err());
        assert!(normalize_package_rel_path("/tmp/App.vue").is_err());
        assert!(normalize_package_rel_path("src//App.vue").is_err());
    }

    #[test]
    fn view_frontend_log_workspace_path_matches_log_only() {
        assert!(is_view_frontend_log_workspace_path(
            "Locus/View/ProjectName/material-inspector/.locus/logs/frontend.log"
        ));
        assert!(is_view_frontend_log_workspace_path(
            "Locus\\View\\Tools\\material-inspector\\.locus\\logs\\frontend.log"
        ));
        assert!(!is_view_frontend_log_workspace_path(
            "Locus/View/ProjectName/material-inspector/.locus/data/state.json"
        ));
        assert!(!is_view_frontend_log_workspace_path(
            "Locus/View/ProjectName/material-inspector/src/App.vue"
        ));
        assert!(!is_view_frontend_log_workspace_path(
            "Locus/knowledge/memory/.locus/notes.md"
        ));
    }

    #[test]
    fn view_watcher_ignores_internal_locus_files() {
        let event = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(
            PathBuf::from("Locus/View/ProjectName/material-inspector/.locus/logs/frontend.log"),
        );
        assert!(!should_reload_for_view_event(&event));

        let event = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(
            PathBuf::from("Locus/View/ProjectName/material-inspector/src/App.vue"),
        );
        assert!(should_reload_for_view_event(&event));
    }

    #[test]
    fn view_create_request_parses_temporary_flag() {
        let (request, temporary) = parse_view_create_request(json!({
            "id": "scratch-panel",
            "name": "Scratch Panel",
            "template": "blank",
            "temporary": true
        }))
        .expect("parse view_create request");

        assert!(temporary);
        assert_eq!(request.id, "scratch-panel");
        assert_eq!(request.name.as_deref(), Some("Scratch Panel"));
        assert_eq!(request.template.as_deref(), Some("blank"));

        let (request, temporary) = parse_view_create_request(json!({
            "id": "package-panel",
            "packageName": "Gameplay",
            "template": "blank"
        }))
        .expect("parse package view_create request");
        assert!(!temporary);
        assert_eq!(request.package_name.as_deref(), Some("Gameplay"));
    }

    #[test]
    fn manifest_validation_checks_schema_id_and_paths() {
        let mut manifest = ViewManifest {
            schema: VIEW_SCHEMA.to_string(),
            id: "material-inspector".to_string(),
            name: "Material Inspector".to_string(),
            version: "0.1.0".to_string(),
            template: "blank".to_string(),
            icon: None,
            entry: "src/main.ts".to_string(),
            style: "src/style.css".to_string(),
            bindings: "bindings.json".to_string(),
            scripts: Vec::new(),
            capabilities: Default::default(),
            requirements: None,
        };
        validate_view_manifest(&manifest).unwrap();

        manifest.entry = "../main.ts".to_string();
        assert!(validate_view_manifest(&manifest).is_err());
    }

    #[test]
    fn create_view_writes_loadable_blank_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.id, "material-inspector");
        assert_eq!(created.manifest.icon.as_deref(), Some("View"));
        let package_root = default_test_view_package_root(&working_dir);
        assert!(package_root.join("material-inspector/view.json").is_file());

        let read = read_view_sync(&working_dir, "material-inspector").expect("read view");
        assert!(read
            .files
            .iter()
            .any(|file| file.rel_path.ends_with("/material-inspector/src/App.vue")));
    }

    #[test]
    fn create_view_writes_package_workspace_library_and_hides_workspace_src_from_tree() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let package_root = default_test_view_package_root(&working_dir);
        assert!(package_root.join("package.json").is_file());
        assert!(package_root.join("tsconfig.json").is_file());
        assert!(package_root.join("src/index.ts").is_file());
        assert!(package_root.join("src/propertyDraw.ts").is_file());

        let read = read_view_sync(&working_dir, "material-inspector").expect("read view");
        assert!(read
            .files
            .iter()
            .any(|file| file.rel_path.ends_with("/src/index.ts")));
        assert!(read
            .files
            .iter()
            .any(|file| file.rel_path.ends_with("/src/propertyDraw.ts")));

        let tree = list_view_tree_sync(&working_dir).expect("list view tree");
        assert!(!tree
            .folders
            .iter()
            .any(|folder| folder.rel_path.ends_with("/src")));
    }

    #[test]
    fn view_watcher_roots_include_package_workspace_src() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let package_root = default_test_view_package_root(&working_dir);
        let roots =
            view_file_watch_roots(&package_root.join("material-inspector")).expect("watch roots");
        let root_paths = roots
            .iter()
            .map(|path| path.display().to_string().replace('\\', "/"))
            .collect::<Vec<_>>();

        assert!(root_paths
            .iter()
            .any(|path| path.ends_with("/material-inspector")));
        assert!(root_paths.iter().any(|path| path.ends_with("/src")));
    }

    #[test]
    fn create_view_can_place_multiple_views_in_one_package_workspace() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: Some("Gameplay".to_string()),
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create first view");
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "stat-table".to_string(),
                package_name: Some("Gameplay".to_string()),
                name: Some("Stat Table".to_string()),
                template: Some("serialized-table".to_string()),
                icon: None,
            },
        )
        .expect("create second view");

        let package_root = temp.path().join("Locus/View/Gameplay");
        assert!(package_root.join("src/index.ts").is_file());
        assert!(package_root.join("material-inspector/view.json").is_file());
        assert!(package_root.join("stat-table/view.json").is_file());

        let listed = list_views_sync(&working_dir).expect("list views");
        assert!(listed
            .iter()
            .any(|view| view.package_rel_path == "Gameplay/material-inspector"));
        assert!(listed
            .iter()
            .any(|view| view.package_rel_path == "Gameplay/stat-table"));

        let read = read_view_sync(&working_dir, "stat-table").expect("read second view");
        assert!(read
            .files
            .iter()
            .any(|file| file.rel_path == "Gameplay/src/propertyDraw.ts"));
        assert!(read
            .files
            .iter()
            .any(|file| file.rel_path == "Gameplay/stat-table/src/App.vue"));
    }

    #[test]
    fn create_view_rejects_duplicate_id_across_package_workspaces() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: Some("Gameplay".to_string()),
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create first view");

        let error = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: Some("Tools".to_string()),
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect_err("duplicate view id should fail");

        assert!(error.contains("View package id already exists"));
    }

    #[test]
    fn read_view_includes_importable_src_modules() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let package_root = default_test_view_package_root(&working_dir);
        let root = package_root.join("material-inspector");
        std::fs::create_dir_all(root.join("src/components")).expect("create components dir");
        std::fs::write(
            root.join("src/components/FieldRow.vue"),
            "<template><div>{{ label }}</div></template>",
        )
        .expect("write component");
        std::fs::write(root.join("src/runtime.ts"), "export const ready = true;\n")
            .expect("write runtime module");
        std::fs::write(root.join("src/theme.css"), ".field-row{display:grid}\n")
            .expect("write css module");

        let read = read_view_sync(&working_dir, "material-inspector").expect("read view");
        let paths = read
            .files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect::<Vec<_>>();

        assert!(paths
            .iter()
            .any(|path| path.ends_with("/material-inspector/src/components/FieldRow.vue")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("/material-inspector/src/runtime.ts")));
        assert!(paths
            .iter()
            .any(|path| path.ends_with("/material-inspector/src/theme.css")));
    }

    #[test]
    fn export_and_import_view_package_zip_round_trip() {
        let source = tempdir().unwrap();
        let source_working_dir = source.path().to_string_lossy().to_string();
        create_view_sync(
            &source_working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let source_package_root = default_test_view_package_root(&source_working_dir);
        std::fs::write(
            source_package_root.join("material-inspector/src/custom.ts"),
            "export const value = 42;\n",
        )
        .expect("write custom source");
        append_view_frontend_log_sync(
            &source_working_dir,
            ViewFrontendLogRequest {
                view_id: "material-inspector".to_string(),
                level: "warn".to_string(),
                message: "runtime log".to_string(),
            },
        )
        .expect("write frontend log");

        let archive_path = source.path().join("material-inspector.zip");
        let saved_path = export_view_package_sync(
            &source_working_dir,
            ViewExportPackageRequest {
                view_id: "material-inspector".to_string(),
                file_path: archive_path.to_string_lossy().to_string(),
            },
        )
        .expect("export view package");
        assert!(PathBuf::from(saved_path).is_file());

        let target = tempdir().unwrap();
        let target_working_dir = target.path().to_string_lossy().to_string();
        let imported = import_view_package_sync(
            &target_working_dir,
            ViewImportPackageRequest {
                file_path: archive_path.to_string_lossy().to_string(),
                target_dir_rel_path: None,
            },
        )
        .expect("import view package");

        assert_eq!(imported.summary.id, "material-inspector");
        assert_eq!(imported.summary.name, "Material Inspector");
        assert!(imported
            .snapshot
            .views
            .iter()
            .any(|view| view.id == "material-inspector"));

        let imported_root =
            default_test_view_package_root(&target_working_dir).join("material-inspector");
        assert!(imported_root.join("view.json").is_file());
        assert!(imported_root.join("src/custom.ts").is_file());
        assert!(!imported_root.join(".locus/logs/frontend.log").exists());
        assert!(default_test_view_package_root(&target_working_dir)
            .join("package.json")
            .is_file());
    }

    #[test]
    fn import_view_package_rejects_duplicate_view_id() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let archive_path = temp.path().join("material-inspector.zip");
        export_view_package_sync(
            &working_dir,
            ViewExportPackageRequest {
                view_id: "material-inspector".to_string(),
                file_path: archive_path.to_string_lossy().to_string(),
            },
        )
        .expect("export view package");

        let error = import_view_package_sync(
            &working_dir,
            ViewImportPackageRequest {
                file_path: archive_path.to_string_lossy().to_string(),
                target_dir_rel_path: None,
            },
        )
        .expect_err("duplicate view id should fail");

        assert!(error.contains("View package id already exists"));
    }

    #[test]
    fn append_frontend_log_writes_jsonl_under_view_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        append_view_frontend_log_sync(
            &working_dir,
            ViewFrontendLogRequest {
                view_id: "material-inspector".to_string(),
                level: "warn".to_string(),
                message: "shader property failed".to_string(),
            },
        )
        .expect("append log");

        let log = std::fs::read_to_string(
            default_test_view_package_root(&working_dir)
                .join("material-inspector/.locus/logs/frontend.log"),
        )
        .expect("read log");
        assert!(log.contains("\"level\":\"warn\""));
        assert!(log.contains("\"message\":\"shader property failed\""));

        let entries = read_view_frontend_log_sync(
            &working_dir,
            ViewFrontendLogReadRequest {
                view_id: "material-inspector".to_string(),
                limit: Some(1),
            },
        )
        .expect("read frontend log");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "warn");
        assert_eq!(entries[0].message, "shader property failed");
    }

    #[test]
    fn list_and_resolve_nested_view_packages() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = temp.path().join("Locus/View/Tools/material-inspector");
        std::fs::create_dir_all(root.join("src")).expect("create nested view");

        let manifest = ViewManifest {
            schema: VIEW_SCHEMA.to_string(),
            id: "material-inspector".to_string(),
            name: "Material Inspector".to_string(),
            version: "0.1.0".to_string(),
            template: "blank".to_string(),
            icon: None,
            entry: "src/main.ts".to_string(),
            style: "src/style.css".to_string(),
            bindings: "bindings.json".to_string(),
            scripts: Vec::new(),
            capabilities: Default::default(),
            requirements: None,
        };
        std::fs::write(
            root.join("view.json"),
            serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        std::fs::write(
            root.join("bindings.json"),
            format!(
                "{{\"schema\":\"{}\",\"bindings\":[]}}\n",
                VIEW_BINDINGS_SCHEMA
            ),
        )
        .expect("write bindings");

        let listed = list_views_sync(&working_dir).expect("list views");
        let summary = listed
            .iter()
            .find(|view| view.id == "material-inspector")
            .expect("nested view listed");
        assert_eq!(summary.package_rel_path, "Tools/material-inspector");

        let resolved = view_package_root(&working_dir, "material-inspector").expect("resolve view");
        assert_eq!(
            resolved.display().to_string().replace('\\', "/"),
            root.display().to_string().replace('\\', "/")
        );
    }

    #[test]
    fn view_tree_folders_create_delete_and_move_disk_entries() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let folder = create_view_folder_sync(
            &working_dir,
            super::ViewCreateFolderRequest {
                parent_rel_path: None,
                name: "Tools".to_string(),
            },
        )
        .expect("create root folder");
        assert_eq!(folder.rel_path, "Tools");
        assert!(temp.path().join("Locus/View/Tools").is_dir());

        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let tree = list_view_tree_sync(&working_dir).expect("list tree");
        assert!(tree.folders.iter().any(|item| item.rel_path == "Tools"));
        let default_package = default_view_package_name(&working_dir).expect("default package");
        assert!(tree
            .views
            .iter()
            .any(|item| item.package_rel_path == format!("{default_package}/material-inspector")));

        let moved = move_view_entry_sync(
            &working_dir,
            super::ViewMoveEntryRequest {
                source_rel_path: format!("{default_package}/material-inspector"),
                target_dir_rel_path: Some("Tools".to_string()),
            },
        )
        .expect("move view into folder");
        assert!(temp
            .path()
            .join("Locus/View/Tools/material-inspector")
            .is_dir());
        assert!(!default_test_view_package_root(&working_dir)
            .join("material-inspector")
            .exists());
        assert!(moved
            .views
            .iter()
            .any(|item| item.package_rel_path == "Tools/material-inspector"));

        let deleted = delete_view_entry_sync(
            &working_dir,
            super::ViewDeleteEntryRequest {
                rel_path: "Tools".to_string(),
            },
        )
        .expect("delete folder");
        assert!(!temp.path().join("Locus/View/Tools").exists());
        assert!(deleted.views.is_empty());
        assert!(deleted
            .folders
            .iter()
            .any(|item| item.rel_path == default_package));
    }

    #[test]
    fn supported_templates_include_graph_link_board_and_serialized_table() {
        let ids = supported_view_templates()
            .into_iter()
            .map(|template| template.id)
            .collect::<Vec<_>>();

        assert!(ids.contains(&"canvas-board".to_string()));
        assert!(ids.contains(&"field-blocks".to_string()));
        assert!(ids.contains(&"node-graph".to_string()));
        assert!(ids.contains(&"link-board".to_string()));
        assert!(ids.contains(&"serialized-table".to_string()));
    }

    #[test]
    fn create_view_writes_loadable_node_graph_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "flow-editor".to_string(),
                package_name: None,
                name: Some("Flow Editor".to_string()),
                template: Some("node-graph".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.template, "node-graph");
        assert!(created.manifest.capabilities.unity);
        assert!(created.manifest.capabilities.write_back);
        assert!(view_manifest_requirements(&created.manifest).unity_connection);
        assert_eq!(created.manifest.scripts[0].name, "GraphViewApi");
        let app = created
            .files
            .iter()
            .find(|file| file.rel_path.ends_with("/flow-editor/src/App.vue"))
            .expect("app file");
        assert!(app.content.contains("GraphViewController"));
        assert!(app.content.contains("<GraphView :controller=\"graphView\""));
        assert!(app
            .content
            .contains("view.callScript(\"GraphViewApi\", \"Read\""));
        assert!(app
            .content
            .contains("view.callScript(\"GraphViewApi\", \"Save\""));
        assert!(app.content.contains("validateConnection"));
        assert!(app.content.contains("parameters:"));
        assert!(app.content.contains("portId: \"object\""));

        let api = created
            .files
            .iter()
            .find(|file| file.rel_path.ends_with("/flow-editor/unity/ViewApi.cs"))
            .expect("api file");
        assert!(api.content.contains("public static class GraphViewApi"));
        assert!(api.content.contains("public static object Apply"));
    }

    #[test]
    fn create_view_writes_loadable_canvas_board_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "canvas-board".to_string(),
                package_name: None,
                name: Some("Canvas Board".to_string()),
                template: Some("canvas-board".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.template, "canvas-board");
        assert_eq!(created.manifest.icon.as_deref(), Some("PanelsTopLeft"));
        assert!(!created.manifest.capabilities.unity);
        assert!(!view_manifest_requirements(&created.manifest).unity_connection);

        let app = created
            .files
            .iter()
            .find(|file| file.rel_path.ends_with("/canvas-board/src/App.vue"))
            .expect("app file");
        assert!(app.content.contains("CanvasView"));
        assert!(app.content.contains("data-locus-template=\"canvas-board\""));
        assert!(app.content.contains("v-model:selected-item-ids"));
        assert!(app.content.contains(":edit-behavior=\"canvasBehavior\""));
        assert!(app.content.contains("@copy-selection=\"copySelection\""));
        assert!(app
            .content
            .contains("@context-menu=\"onCanvasContextMenu\""));
    }

    #[test]
    fn create_view_writes_loadable_field_blocks_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "field-blocks".to_string(),
                package_name: None,
                name: Some("Field Blocks".to_string()),
                template: Some("field-blocks".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.template, "field-blocks");
        assert_eq!(created.manifest.icon.as_deref(), Some("FormInput"));
        assert!(created.manifest.capabilities.unity);
        assert!(created.manifest.capabilities.bindings);
        assert!(created.manifest.capabilities.write_back);
        assert!(view_manifest_requirements(&created.manifest).unity_connection);

        let app = created
            .files
            .iter()
            .find(|file| file.rel_path.ends_with("/field-blocks/src/App.vue"))
            .expect("app file");
        assert!(app.content.contains("CanvasView"));
        assert!(app.content.contains("UnityPropertyEditor"));
        assert!(app.content.contains("view.binding.read"));
        assert!(app.content.contains("view.binding.write"));
        assert!(app.content.contains("data-locus-template=\"field-blocks\""));
    }

    #[test]
    fn create_view_writes_loadable_serialized_table_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "serialized-browser".to_string(),
                package_name: None,
                name: Some("Serialized Browser".to_string()),
                template: Some("serialized-table".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.template, "serialized-table");
        assert_eq!(created.manifest.icon.as_deref(), Some("TableProperties"));
        assert!(created.manifest.capabilities.unity);
        assert!(created.manifest.capabilities.write_back);
        assert!(view_manifest_requirements(&created.manifest).unity_connection);
        assert_eq!(created.manifest.scripts[0].name, "SerializedTableApi");

        let app = created
            .files
            .iter()
            .find(|file| file.rel_path.ends_with("/serialized-browser/src/App.vue"))
            .expect("app file");
        assert!(app.content.contains("SerializedTableApi"));
        assert!(app.content.contains("view.assets.search"));
        assert!(app
            .content
            .contains("data-locus-template=\"serialized-table\""));
        assert!(app.content.contains("UnityPropertyEditor"));
        assert!(app.content.contains("commitCell"));
        assert!(app.content.contains("TableLoadProgress"));
        assert!(app.content.contains("Preparing C# reader"));
        assert!(app.content.contains("table-progress-status"));
        assert!(!app.content.contains("Add Row"));
        assert!(!app.content.contains("Add Column"));
        assert!(!app.content.contains("config-pane"));
        assert!(app.content.contains("localStorage"));
        assert!(app.content.contains("persistColumnWidths"));
        assert!(!app.content.contains("cache hit"));

        let config = created
            .files
            .iter()
            .find(|file| {
                file.rel_path
                    .ends_with("/serialized-browser/src/tableConfig.ts")
            })
            .expect("config file");
        assert!(config.content.contains("tableColumns"));
        assert!(config.content.contains("tableSources"));
        assert!(config.content.contains("tableSourceProviders"));
        assert!(config.content.contains("t:prefab component:Entity"));
        assert!(config.content.contains("t:scriptableObject inherits:IData"));
        assert!(config.content.contains("maxRows: 1000"));

        let api = created
            .files
            .iter()
            .find(|file| {
                file.rel_path
                    .ends_with("/serialized-browser/unity/ViewApi.cs")
            })
            .expect("api file");
        assert!(api
            .content
            .contains("public static class SerializedTableApi"));
        assert!(api.content.contains("SerializedProperty"));
        assert!(api.content.contains("public static object Write"));
        assert!(api.content.contains("TypeMatches"));
        assert!(!api.content.contains("Selection.active"));
    }

    #[test]
    fn view_script_payload_reads_manifest_script_and_hashes_source() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("inspector-form".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let resolved =
            resolve_view_script_sync(&working_dir, "material-inspector", "InspectorViewApi")
                .expect("resolve script");
        let payload = view_script_bridge_payload(&resolved, None).expect("payload");

        assert_eq!(payload["viewId"], "material-inspector");
        assert_eq!(payload["scriptName"], "InspectorViewApi");
        assert_eq!(payload["entryType"], "InspectorViewApi");
        assert_eq!(payload["path"], "unity/ViewApi.cs");
        assert_eq!(
            payload["sourceHash"],
            blake3::hash(resolved.source.as_bytes())
                .to_hex()
                .to_string()
        );
    }

    #[test]
    fn view_script_cached_invoke_payload_omits_source() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("inspector-form".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let resolved =
            resolve_view_script_sync(&working_dir, "material-inspector", "InspectorViewApi")
                .expect("resolve script");
        let payload =
            view_script_cached_invoke_payload(&resolved, "Read", &serde_json::json!({"id": 1}))
                .expect("cached payload");

        assert_eq!(payload["viewId"], "material-inspector");
        assert_eq!(payload["scriptName"], "InspectorViewApi");
        assert_eq!(payload["entryType"], "InspectorViewApi");
        assert_eq!(payload["path"], "unity/ViewApi.cs");
        assert_eq!(payload["method"], "Read");
        assert_eq!(payload["argsJson"], "{\"id\":1}");
        assert!(payload.get("source").is_none());
    }

    #[test]
    fn resolve_view_binding_target_reads_bindings_json() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");
        std::fs::write(
            default_test_view_package_root(&working_dir).join("material-inspector/bindings.json"),
            r#"{
  "schema": "locus.view.bindings.v1",
  "bindings": [
    {
      "id": "object-name",
      "statePath": "selection.name",
      "target": {
        "kind": "gameObject",
        "scenePath": "Assets/Scenes/Main.unity",
        "objectPath": "Root/Player",
        "propertyPath": "m_Name"
      },
      "mode": "readWrite"
    }
  ]
}
"#,
        )
        .expect("write bindings");

        let target = resolve_view_binding_target(
            &working_dir,
            "material-inspector",
            Some("object-name"),
            None,
        )
        .expect("resolve target");

        assert_eq!(
            target,
            ViewBindingTarget {
                kind: "gameObject".to_string(),
                path: None,
                scene_path: Some("Assets/Scenes/Main.unity".to_string()),
                object_path: Some("Root/Player".to_string()),
                component_type: None,
                component_index: None,
                property_path: Some("m_Name".to_string()),
            }
        );
    }

    #[test]
    fn loaded_view_bindings_resolve_valid_id_without_parsing_unused_targets() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                package_name: None,
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");
        std::fs::write(
            default_test_view_package_root(&working_dir).join("material-inspector/bindings.json"),
            r#"{
  "schema": "locus.view.bindings.v1",
  "bindings": [
    {
      "id": "object-name",
      "target": {
        "kind": "gameObject",
        "scenePath": "Assets/Scenes/Main.unity",
        "objectPath": "Root/Player",
        "propertyPath": "m_Name"
      },
      "mode": "readWrite"
    },
    {
      "id": "unused-broken-binding",
      "mode": "readWrite"
    }
  ]
}
"#,
        )
        .expect("write bindings");

        let loaded =
            super::load_view_bindings(&working_dir, "material-inspector").expect("load bindings");
        let binding =
            super::resolve_view_binding_from_loaded(&loaded, Some("object-name")).unwrap();

        assert_eq!(binding.mode.as_deref(), Some("readWrite"));
        assert_eq!(binding.target.kind, "gameObject");
        assert_eq!(binding.target.property_path.as_deref(), Some("m_Name"));
    }

    #[test]
    fn read_only_view_binding_rejects_write_path() {
        assert!(ensure_view_binding_write_allowed(Some("readWrite")).is_ok());
        assert!(ensure_view_binding_write_allowed(None).is_ok());
        assert!(ensure_view_binding_write_allowed(Some("readOnly")).is_err());
        assert!(ensure_view_binding_write_allowed(Some("readonly")).is_err());
    }

    #[test]
    fn view_binding_object_target_validation_allows_property_discovery_root() {
        let target = ViewBindingTarget {
            kind: "component".to_string(),
            path: None,
            scene_path: Some("Assets/Scenes/Main.unity".to_string()),
            object_path: Some("Root/Player".to_string()),
            component_type: Some("Game.Settings".to_string()),
            component_index: Some(0),
            property_path: None,
        };

        assert!(validate_view_binding_object_target(&target).is_ok());
        assert!(validate_view_binding_target(&target).is_err());
    }

    #[test]
    fn view_binding_read_result_preserves_serialized_property_tree_snapshot() {
        let raw = r#"{
  "ok": true,
  "bindingId": "settings",
  "message": "ok",
  "target": {
    "kind": "component",
    "scenePath": "Assets/Scenes/Main.unity",
    "objectPath": "Root/Player[1]",
    "componentType": "Game.Settings",
    "componentIndex": 2,
    "propertyPath": "items"
  },
  "propertyPath": "items",
  "displayName": "Items",
  "name": "items",
  "type": "Generic",
  "valueType": "Generic",
  "fieldTypeFullName": "Game.Inventory.Items",
  "fieldTypeAssembly": "Assembly-CSharp",
  "value": null,
  "displayValue": "Array (1)",
  "editable": true,
  "hasChildren": true,
  "isArray": true,
  "arraySize": 1,
  "children": [
    {
      "propertyPath": "items.Array.data[0]",
      "displayName": "Element 0",
      "name": "data[0]",
      "type": "String",
      "valueType": "String",
      "fieldTypeFullName": "System.String",
      "fieldTypeAssembly": "mscorlib",
      "value": "alpha",
      "displayValue": "alpha",
      "editable": true,
      "hasChildren": false,
      "isArray": false,
      "arraySize": -1,
      "children": [],
      "isManagedReference": false,
      "managedReferenceFullTypename": "",
      "managedReferenceFieldTypename": "",
      "managedReferenceDisplayName": "",
      "managedReferenceTypes": []
    }
  ],
  "isManagedReference": false,
  "managedReferenceFullTypename": "",
  "managedReferenceFieldTypename": "",
  "managedReferenceDisplayName": "",
  "managedReferenceTypes": [],
  "saved": false
}
"#;

        let result: ViewBindingWriteResult = serde_json::from_str(raw).expect("deserialize result");
        assert_eq!(result.read.target.component_index, Some(2));
        assert!(result.read.property.is_array);
        assert_eq!(result.read.property.array_size, 1);
        assert_eq!(result.read.property.children.len(), 1);
        assert_eq!(
            result.read.property.children[0].property_path,
            "items.Array.data[0]"
        );
        assert_eq!(
            result.read.property.field_type_full_name,
            "Game.Inventory.Items"
        );
        assert_eq!(
            result.read.property.children[0].field_type_full_name,
            "System.String"
        );

        let encoded = serde_json::to_value(&result).expect("serialize result");
        assert_eq!(encoded["children"][0]["value"], "alpha");
        assert_eq!(encoded["type"], "Generic");
        assert_eq!(encoded["fieldTypeFullName"], "Game.Inventory.Items");
        assert_eq!(encoded["target"]["componentIndex"], 2);
    }

    #[test]
    fn view_binding_discover_result_preserves_field_type_metadata() {
        let raw = r#"{
  "ok": true,
  "bindingId": "settings",
  "message": "ok",
  "target": {
    "kind": "component",
    "scenePath": "Assets/Scenes/Main.unity",
    "objectPath": "Root/Player",
    "componentType": "Game.Settings"
  },
  "matches": [
    {
      "propertyPath": "stats",
      "displayName": "Stats",
      "name": "stats",
      "type": "Generic",
      "valueType": "Generic",
      "fieldTypeFullName": "Game.SharedStat",
      "fieldTypeAssembly": "Assembly-CSharp",
      "displayValue": "",
      "editable": true,
      "hasChildren": true,
      "isArray": false,
      "isManagedReference": false,
      "depth": 0
    }
  ]
}
"#;

        let result: ViewBindingDiscoverResult =
            serde_json::from_str(raw).expect("deserialize discover result");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].property_path, "stats");
        assert_eq!(result.matches[0].property_type, "Generic");
        assert_eq!(result.matches[0].field_type_full_name, "Game.SharedStat");

        let encoded = serde_json::to_value(&result).expect("serialize discover result");
        assert_eq!(encoded["matches"][0]["type"], "Generic");
        assert_eq!(
            encoded["matches"][0]["fieldTypeAssembly"],
            "Assembly-CSharp"
        );
    }
}
