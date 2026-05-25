use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};

pub const VIEW_SCHEMA: &str = "locus.view.v1";
pub const VIEW_BINDINGS_SCHEMA: &str = "locus.view.bindings.v1";
pub const VIEW_ROOT_RELATIVE: &str = "Locus/views";
pub const TEMP_VIEW_ROOT_RELATIVE: &str = "view-packages";
pub const VIEW_RELOAD_EVENT: &str = "view-package-reloaded";
pub const VIEW_TREE_CHANGED_EVENT: &str = "view-tree-changed";

const VIEW_HOST_ROUTE: &str = "/view-host";
const VIEW_FRONTEND_LOG_REL_PATH: &str = ".locus/logs/frontend.log";
const VIEW_FRONTEND_LOG_MAX_CHARS: usize = 16_384;

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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ViewCreateRequest {
    pub id: String,
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
pub struct ViewRunResult {
    pub id: String,
    pub window_label: String,
    pub host_url: String,
    pub package_root: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewBindingReadResult {
    pub ok: bool,
    #[serde(default)]
    pub binding_id: Option<String>,
    pub message: String,
    pub target: ViewBindingTarget,
    pub property_path: String,
    pub display_name: String,
    pub value_type: String,
    #[serde(default)]
    pub value: serde_json::Value,
    pub editable: bool,
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
    vec![
        ViewTemplateSummary {
            id: "blank".to_string(),
            name: "Blank".to_string(),
            description: "Minimal editable View package.".to_string(),
        },
        ViewTemplateSummary {
            id: "inspector-form".to_string(),
            name: "Inspector Form".to_string(),
            description: "Field-oriented form scaffold for Unity data.".to_string(),
        },
        ViewTemplateSummary {
            id: "node-graph".to_string(),
            name: "Node Graph".to_string(),
            description: "Draggable node graph scaffold with serialized edges.".to_string(),
        },
        ViewTemplateSummary {
            id: "link-board".to_string(),
            name: "Link Board".to_string(),
            description: "Two-column link mapping scaffold with serialized connections."
                .to_string(),
        },
    ]
}

fn is_supported_template(template: &str) -> bool {
    matches!(
        template,
        "blank" | "inspector-form" | "node-graph" | "link-board"
    )
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
    if !is_supported_template(&manifest.template) {
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
    let views_root = views_root_for_workspace(working_dir)?;
    let direct_root = views_root.join(&id);
    if manifest_matches_id(&direct_root, &id) {
        return Ok(direct_root);
    }

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

fn load_manifest_from_root(root: &Path) -> Result<ViewManifest, String> {
    let path = manifest_path(root);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let manifest: ViewManifest = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid View manifest {}: {}", path.display(), e))?;
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
            entry.file_type().is_file()
                || entry
                    .file_name()
                    .to_str()
                    .map(|name| !is_skippable_view_scan_dir(name))
                    .unwrap_or(true)
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
            entry.file_type().is_file()
                || entry
                    .file_name()
                    .to_str()
                    .map(|name| !is_skippable_view_scan_dir(name))
                    .unwrap_or(true)
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
            if entry
                .file_name()
                .to_str()
                .map(is_skippable_view_scan_dir)
                .unwrap_or(false)
            {
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
    if !is_supported_template(template) {
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
        temporary_view_package_root(working_dir, &id)?
    } else {
        view_package_root(working_dir, &id)?
    };
    if root.exists() {
        return Err(format!("View package already exists: {}", root.display()));
    }
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create {}: {}", root.display(), e))?;

    let manifest = template_manifest(&id, &name, template, icon.as_deref());
    let manifest_raw = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Failed to serialize View manifest: {}", e))?;
    write_package_file(&root, "view.json", &(manifest_raw + "\n"))?;

    for (rel_path, content) in template_files(&id, &name, template) {
        write_package_file(&root, rel_path, &content)?;
    }

    read_view_sync(working_dir, &id)
}

fn unique_temporary_view_id(base_id: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-tmp-{}", base_id, &suffix[..8])
}

fn temporary_view_package_root(working_dir: &str, id: &str) -> Result<PathBuf, String> {
    let id = normalize_view_id(id)?;
    let root = temporary_views_root_for_workspace(working_dir)?;
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create {}: {}", root.display(), e))?;
    Ok(root.join(id))
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
        files.push(read_package_file(&root, &rel_path)?);
    }

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

pub fn open_view_window(
    app_handle: &AppHandle,
    working_dir: &str,
    view_id: &str,
) -> Result<ViewRunResult, String> {
    let detail = read_view_sync(working_dir, view_id)?;
    let id = detail.summary.id.clone();
    let label = view_window_label(&id);
    let host_url = format!("{}?id={}", VIEW_HOST_ROUTE, id);

    if let Some(window) = app_handle.get_webview_window(&label) {
        window
            .set_focus()
            .map_err(|e| format!("Failed to focus View window: {}", e))?;
    } else {
        tauri::WebviewWindowBuilder::new(
            app_handle,
            &label,
            WebviewUrl::App(host_url.clone().into()),
        )
        .title(format!("{} - Locus View", detail.summary.name))
        .inner_size(1180.0, 760.0)
        .min_inner_size(760.0, 480.0)
        .resizable(true)
        .visible(true)
        .disable_drag_drop_handler()
        .build()
        .map_err(|e| format!("Failed to open View window: {}", e))?;
    }

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

pub fn view_window_label(view_id: &str) -> String {
    format!("view-{}", view_id)
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
    !matches!(event.kind, EventKind::Access(_) | EventKind::Other)
}

fn start_view_file_watcher(
    app_handle: &AppHandle,
    working_dir: &str,
    view_id: &str,
) -> Result<(), String> {
    let root = resolve_view_package_root(working_dir, view_id)?;
    let root = dunce::canonicalize(&root).unwrap_or(root);
    let key = root.display().to_string().replace('\\', "/");
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

    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        remove_view_file_watcher_key(&key);
        return Err(format!("Failed to watch {}: {}", root.display(), error));
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
    let root = resolve_view_package_root(working_dir, &request.view_id)?;
    if !root.is_dir() || !manifest_path(&root).is_file() {
        return Err(format!("View package not found: {}", request.view_id));
    }
    load_manifest_from_root(&root)?;

    let log_path = package_path(&root, VIEW_FRONTEND_LOG_REL_PATH)?;
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
    if target.kind.trim().is_empty() {
        return Err("View binding target kind cannot be empty.".to_string());
    }
    if target
        .property_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err("View binding target propertyPath is required.".to_string());
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

fn read_package_file(root: &Path, rel_path: &str) -> Result<ViewPackageFile, String> {
    let path = package_path(root, rel_path)?;
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
    if rel_path == "view.json" {
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

fn default_icon_for_template(template: &str) -> &'static str {
    match template {
        "inspector-form" => "InspectionPanel",
        "node-graph" => "Network",
        "link-board" => "Link2",
        _ => "View",
    }
}

fn template_manifest(id: &str, name: &str, template: &str, icon: Option<&str>) -> ViewManifest {
    let scripts = if template == "inspector-form" {
        vec![ViewScriptManifest {
            name: "InspectorViewApi".to_string(),
            path: "unity/ViewApi.cs".to_string(),
            entry_type: "InspectorViewApi".to_string(),
        }]
    } else {
        Vec::new()
    };

    ViewManifest {
        schema: VIEW_SCHEMA.to_string(),
        id: id.to_string(),
        name: name.to_string(),
        version: "0.1.0".to_string(),
        template: template.to_string(),
        icon: Some(
            icon.unwrap_or_else(|| default_icon_for_template(template))
                .to_string(),
        ),
        entry: "src/main.ts".to_string(),
        style: "src/style.css".to_string(),
        bindings: "bindings.json".to_string(),
        scripts,
        capabilities: ViewCapabilities {
            unity: template == "inspector-form",
            bindings: template == "inspector-form",
            write_back: false,
        },
    }
}

fn template_files(id: &str, name: &str, template: &str) -> Vec<(&'static str, String)> {
    let created_at = now_millis();
    let readme = format!(
        "# {name}\n\nView Package `{id}` generated from `{template}`.\n\nEdit files under this directory and reload the View from Locus.\n"
    );
    let bindings = format!(
        "{{\n  \"schema\": \"{}\",\n  \"bindings\": []\n}}\n",
        VIEW_BINDINGS_SCHEMA
    );

    let mut files = vec![
        ("README.md", readme),
        ("src/main.ts", main_ts()),
        ("src/store.ts", store_ts()),
        ("bindings.json", bindings),
    ];

    match template {
        "inspector-form" => {
            files.push(("src/App.vue", inspector_app_vue(name)));
            files.push(("src/style.css", inspector_style_css()));
            files.push(("unity/ViewApi.cs", inspector_view_api_cs()));
        }
        "node-graph" => {
            files.push(("src/App.vue", node_graph_app_vue(name)));
            files.push(("src/style.css", node_graph_style_css()));
        }
        "link-board" => {
            files.push(("src/App.vue", link_board_app_vue(name)));
            files.push(("src/style.css", link_board_style_css()));
        }
        _ => {
            files.push(("src/App.vue", blank_app_vue(name)));
            files.push(("src/style.css", blank_style_css()));
        }
    }

    files.push((
        ".locus-view",
        format!("createdAt={created_at}\ntemplate={template}\n"),
    ));
    files
}

fn main_ts() -> String {
    r##"import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";

createApp(App).mount("#app");
"##
    .to_string()
}

fn store_ts() -> String {
    r#"import { reactive } from "vue";

export const viewState = reactive({
  dirty: false,
  status: "idle",
});
"#
    .to_string()
}

fn blank_app_vue(name: &str) -> String {
    format!(
        r#"<template>
  <main class="view-shell">
    <header class="view-header">
      <span class="view-kicker">View Package</span>
      <h1>{name}</h1>
    </header>

    <section class="view-panel">
      <div class="view-row">
        <label>Context</label>
        <span>Waiting for Unity data</span>
      </div>
      <div class="view-row">
        <label>Status</label>
        <span>Ready</span>
      </div>
    </section>
  </main>
</template>
"#
    )
}

fn blank_style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

.view-shell {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  box-sizing: border-box;
}

.view-header {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.view-kicker {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

h1 {
  margin: 0;
  font-size: 20px;
  line-height: 1.25;
}

.view-panel {
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.view-row {
  display: grid;
  grid-template-columns: 140px minmax(0, 1fr);
  gap: 12px;
  padding: 10px 12px;
  font-size: 13px;
}

.view-row + .view-row {
  border-top: 1px solid var(--border-color);
}

label {
  color: var(--text-secondary);
}
"#
    .to_string()
}

fn inspector_app_vue(name: &str) -> String {
    format!(
        r##"<template>
  <main class="view-shell inspector-view">
    <header class="view-toolbar">
      <div>
        <span class="view-kicker">Inspector Form</span>
        <h1>{name}</h1>
      </div>
      <button type="button">Apply</button>
    </header>

    <section class="inspector-grid">
      <label>
        <span>Target</span>
        <input value="Assets/Materials/Example.mat" />
      </label>
      <label>
        <span>Base Color</span>
        <input value="#d9dde5" />
      </label>
      <label>
        <span>Metallic</span>
        <input value="0.00" />
      </label>
      <label>
        <span>Smoothness</span>
        <input value="0.50" />
      </label>
    </section>
  </main>
</template>
"##
    )
}

fn inspector_style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

.view-shell {
  min-height: 100vh;
  padding: 18px;
  box-sizing: border-box;
}

.view-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
}

.view-kicker {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

h1 {
  margin: 2px 0 0;
  font-size: 18px;
  line-height: 1.25;
}

button {
  min-height: 30px;
  padding: 0 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
}

.inspector-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 10px;
  max-width: 620px;
  padding-top: 16px;
}

label {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  font-size: 13px;
}

label span {
  color: var(--text-secondary);
}

input {
  min-height: 30px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
}
"#
    .to_string()
}

fn inspector_view_api_cs() -> String {
    r#"using System;
using UnityEditor;
using UnityEngine;

public static class InspectorViewApi
{
    public static object Read(object args)
    {
        return new
        {
            ok = true,
            message = "View Script runtime is ready for this package."
        };
    }
}
"#
    .to_string()
}

fn node_graph_app_vue(name: &str) -> String {
    r##"<script setup lang="ts">
import { GraphView, GraphViewController, defineGraphView } from "@locus/view-runtime";

class TemplateGraphView extends GraphViewController {
  loadGraph() {
    return {
      layout: { auto: "missing", direction: "right" },
      nodes: [
        {
          id: "selection",
          type: "source",
          title: "Selection",
          subtitle: "Unity data",
          outputs: [
            { id: "object", label: "Object", type: "Unity.Object" },
            { id: "path", label: "Path", type: "string" }
          ],
          parameters: [
            { id: "mode", label: "Mode", type: "select", value: "active", options: [
              { label: "Active", value: "active" },
              { label: "Pinned", value: "pinned" }
            ] }
          ]
        },
        {
          id: "process",
          type: "processor",
          title: "Process",
          subtitle: "Transform",
          inputs: [
            { id: "input", label: "Input", type: "Unity.Object" }
          ],
          outputs: [
            { id: "result", label: "Result", type: "object" }
          ],
          parameters: [
            { id: "enabled", label: "Enabled", type: "boolean", value: true },
            { id: "weight", label: "Weight", type: "number", value: 1, min: 0, max: 4, step: 0.1 }
          ]
        },
        {
          id: "apply",
          type: "output",
          title: "Apply",
          subtitle: "Write back",
          inputs: [
            { id: "value", label: "Value", type: "object" }
          ],
          parameters: [
            { id: "target", label: "Target", type: "string", value: "Selection" }
          ]
        }
      ],
      connections: [
        {
          id: "selection-process",
          from: { nodeId: "selection", portId: "object" },
          to: { nodeId: "process", portId: "input" }
        },
        {
          id: "process-apply",
          from: { nodeId: "process", portId: "result" },
          to: { nodeId: "apply", portId: "value" }
        }
      ]
    };
  }

  validateConnection(connection, graph) {
    const targetIsUsed = graph.connections.some((item) => {
      return item.to.nodeId === connection.to.nodeId
        && item.to.portId === connection.to.portId
        && item.id !== connection.id;
    });
    return targetIsUsed ? "Input port already has a connection." : true;
  }

  saveGraph(graph) {
    console.info("Graph saved", graph);
  }

  applyGraph(graph) {
    console.info("Graph applied", graph);
  }
}

const graphView = defineGraphView(new TemplateGraphView());
</script>

<template>
  <GraphView :controller="graphView" title="__VIEW_NAME__" />
</template>
"##
    .replace("__VIEW_NAME__", name)
}

fn node_graph_style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}
"#
    .to_string()
}

fn link_board_app_vue(name: &str) -> String {
    format!(
        r##"<template>
  <main class="view-shell link-board-view" data-locus-template="link-board">
    <header class="view-toolbar">
      <div>
        <span class="view-kicker">Link Board</span>
        <h1>{name}</h1>
      </div>
      <button type="button" data-link-save>Save Links</button>
    </header>

    <section class="link-board" data-link-board>
      <div class="link-column">
        <div class="link-column-title">Sources</div>
        <button type="button" class="link-item" data-link-source="albedo">Albedo Map</button>
        <button type="button" class="link-item" data-link-source="normal">Normal Map</button>
        <button type="button" class="link-item" data-link-source="mask">Mask Texture</button>
      </div>

      <svg class="link-lines" data-link-lines aria-hidden="true"></svg>

      <div class="link-column">
        <div class="link-column-title">Targets</div>
        <button type="button" class="link-item" data-link-target="_BaseMap">_BaseMap</button>
        <button type="button" class="link-item" data-link-target="_BumpMap">_BumpMap</button>
        <button type="button" class="link-item" data-link-target="_MaskMap">_MaskMap</button>
      </div>
    </section>

    <section class="link-data-panel">
      <div class="view-section-title">Link Data</div>
      <pre data-link-output></pre>
    </section>
  </main>
</template>
"##
    )
}

fn link_board_style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

.view-shell {
  min-height: 100vh;
  padding: 18px;
  box-sizing: border-box;
}

.view-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
}

.view-kicker {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

h1 {
  margin: 2px 0 0;
  font-size: 18px;
  line-height: 1.25;
}

.link-board {
  position: relative;
  min-height: 330px;
  display: grid;
  grid-template-columns: minmax(180px, 240px) minmax(120px, 1fr) minmax(180px, 240px);
  gap: 18px;
  margin-top: 14px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.link-column {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.link-column-title {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.link-item {
  min-height: 36px;
  justify-content: flex-start;
  text-align: left;
}

.link-item.active,
.link-item.linked {
  border-color: var(--accent-color);
  background: var(--accent-soft);
}

.link-lines {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

.link-data-panel {
  margin-top: 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.view-section-title {
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

pre {
  margin: 0;
  min-height: 88px;
  padding: 10px;
  overflow: auto;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        append_view_frontend_log_sync, create_view_folder_sync, create_view_sync,
        delete_view_entry_sync, ensure_view_binding_write_allowed, is_valid_view_id,
        list_view_tree_sync, list_views_sync, move_view_entry_sync, normalize_package_rel_path,
        parse_view_create_request, read_view_sync, resolve_view_binding_target,
        resolve_view_script_sync, supported_view_templates, validate_view_manifest,
        view_package_root, view_script_bridge_payload, view_script_cached_invoke_payload,
        ViewBindingTarget, ViewFrontendLogRequest, ViewManifest, VIEW_BINDINGS_SCHEMA, VIEW_SCHEMA,
    };
    use serde_json::json;
    use tempfile::tempdir;

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
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.id, "material-inspector");
        assert_eq!(created.manifest.icon.as_deref(), Some("View"));
        assert!(temp
            .path()
            .join("Locus/views/material-inspector/view.json")
            .is_file());

        let read = read_view_sync(&working_dir, "material-inspector").expect("read view");
        assert!(read.files.iter().any(|file| file.rel_path == "src/App.vue"));
    }

    #[test]
    fn read_view_includes_importable_src_modules() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let root = temp.path().join("Locus/views/material-inspector");
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

        assert!(paths.contains(&"src/components/FieldRow.vue"));
        assert!(paths.contains(&"src/runtime.ts"));
        assert!(paths.contains(&"src/theme.css"));
    }

    #[test]
    fn append_frontend_log_writes_jsonl_under_view_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
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
            temp.path()
                .join("Locus/views/material-inspector/.locus/logs/frontend.log"),
        )
        .expect("read log");
        assert!(log.contains("\"level\":\"warn\""));
        assert!(log.contains("\"message\":\"shader property failed\""));
    }

    #[test]
    fn list_and_resolve_nested_view_packages() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let root = temp.path().join("Locus/views/tools/material-inspector");
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
        assert_eq!(summary.package_rel_path, "tools/material-inspector");

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
        assert!(temp.path().join("Locus/views/Tools").is_dir());

        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        let tree = list_view_tree_sync(&working_dir).expect("list tree");
        assert!(tree.folders.iter().any(|item| item.rel_path == "Tools"));
        assert!(tree
            .views
            .iter()
            .any(|item| item.package_rel_path == "material-inspector"));

        let moved = move_view_entry_sync(
            &working_dir,
            super::ViewMoveEntryRequest {
                source_rel_path: "material-inspector".to_string(),
                target_dir_rel_path: Some("Tools".to_string()),
            },
        )
        .expect("move view into folder");
        assert!(temp
            .path()
            .join("Locus/views/Tools/material-inspector")
            .is_dir());
        assert!(!temp.path().join("Locus/views/material-inspector").exists());
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
        assert!(!temp.path().join("Locus/views/Tools").exists());
        assert!(deleted.views.is_empty());
        assert!(deleted.folders.is_empty());
    }

    #[test]
    fn supported_templates_include_graph_and_link_board() {
        let ids = supported_view_templates()
            .into_iter()
            .map(|template| template.id)
            .collect::<Vec<_>>();

        assert!(ids.contains(&"node-graph".to_string()));
        assert!(ids.contains(&"link-board".to_string()));
    }

    #[test]
    fn create_view_writes_loadable_node_graph_package() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let created = create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "flow-editor".to_string(),
                name: Some("Flow Editor".to_string()),
                template: Some("node-graph".to_string()),
                icon: None,
            },
        )
        .expect("create view");

        assert_eq!(created.manifest.template, "node-graph");
        let app = created
            .files
            .iter()
            .find(|file| file.rel_path == "src/App.vue")
            .expect("app file");
        assert!(app.content.contains("GraphViewController"));
        assert!(app.content.contains("<GraphView :controller=\"graphView\""));
        assert!(app.content.contains("validateConnection"));
        assert!(app.content.contains("parameters:"));
        assert!(app.content.contains("portId: \"object\""));
    }

    #[test]
    fn view_script_payload_reads_manifest_script_and_hashes_source() {
        let temp = tempdir().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        create_view_sync(
            &working_dir,
            super::ViewCreateRequest {
                id: "material-inspector".to_string(),
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
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");
        std::fs::write(
            temp.path()
                .join("Locus/views/material-inspector/bindings.json"),
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
                name: Some("Material Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
            },
        )
        .expect("create view");
        std::fs::write(
            temp.path()
                .join("Locus/views/material-inspector/bindings.json"),
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
}
