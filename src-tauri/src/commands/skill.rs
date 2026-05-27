use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{
    self, KnowledgeDocument, KnowledgeInjectMode, KnowledgeType, SkillSurface,
};
use crate::process_util::{async_command, augment_path_with_git};
use crate::tool::{ToolDef, ToolExecutionContext, ToolRegistry, ToolResult};
use crate::workspace::Workspace;

use super::knowledge::{
    get_updated_at, load_skill_config, reconcile_and_emit_knowledge_changed, save_skill_config,
    AppKnowledgeDir, SkillConfig,
};

// ── Manifest ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub dir_name: String,
    pub source: String,
    pub rel_path: String,
    pub updated_at: i64,
    pub skill_enabled: bool,
    pub skill_surface: SkillSurface,
    pub skill_description: Option<String>,
    pub command_trigger: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default)]
    pub kind: SkillManifestKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(default)]
    pub has_unity: bool,
    #[serde(default)]
    pub has_l0: bool,
    #[serde(default)]
    pub has_l1: bool,
    #[serde(default)]
    pub has_l2: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum SkillManifestKind {
    #[default]
    Document,
    Package,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageSource {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageCommand {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(
        rename = "argument-hint",
        alias = "argumentHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageCapabilities {
    #[serde(default)]
    pub unity: Vec<SkillPackageUnityCapability>,
    #[serde(default)]
    pub python: Vec<serde_json::Value>,
    #[serde(default)]
    pub cli: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageUnityCapability {
    #[serde(default)]
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
}

fn default_tool_parameters() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageToolManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_editor_status: Option<String>,
    #[serde(default = "default_tool_parameters")]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageManifestFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(
        rename = "argument-hint",
        alias = "argumentHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub argument_hint: Option<String>,
    #[serde(
        rename = "disable-model-invocation",
        alias = "disableModelInvocation",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub disable_model_invocation: Option<bool>,
    #[serde(
        rename = "user-invocable",
        alias = "userInvocable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub user_invocable: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schema: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillPackageSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<SkillPackageCommand>,
    #[serde(default)]
    pub capabilities: SkillPackageCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<SkillPackageToolManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SkillPackageDocumentFrontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillPackageRecord {
    pub root: PathBuf,
    pub manifest: SkillPackageManifestFile,
    pub doc_levels: SkillPackageDocLevels,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillPackageUnityScriptBundle {
    pub package_id: String,
    pub source_hash: String,
    pub script_count: usize,
    pub request: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SkillPackageDocLevels {
    pub has_l0: bool,
    pub has_l1: bool,
    pub has_l2: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUnityFileStatus {
    pub source_path: String,
    pub target_path: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUnityInstallStatus {
    pub package_id: String,
    pub has_unity: bool,
    pub state: String,
    pub plugin_root: String,
    pub install_root: String,
    pub files: Vec<SkillUnityFileStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageArchiveResult {
    pub package_id: String,
    pub path: String,
    pub file_count: usize,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillCreateKind {
    #[serde(rename = "md", alias = "document")]
    Md,
    #[serde(rename = "package")]
    Package,
}

impl Default for SkillCreateKind {
    fn default() -> Self {
        Self::Md
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillCreateRequest {
    #[serde(default)]
    pub kind: SkillCreateKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_invocation_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillReloadRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

// ── Config key helpers ───────────────────────────────────────

const SKILL_DIR_NAME: &str = "skill";
const SKILL_PACKAGE_MANIFEST_FILE_NAME: &str = "skill.json";
const SKILL_PACKAGE_ROOT_DOC_FILE_NAME: &str = "SKILL.md";

/// Build the canonical config key for a skill document.
fn config_key(source: &str, dir_name: &str) -> String {
    format!("{}:skill/{}", source, dir_name)
}

pub fn skill_item_id(source: &str, dir_name: &str) -> String {
    format!("skill:{}:{}", source, dir_name)
}

pub fn parse_skill_item_id(item_id: &str) -> Option<(&str, &str)> {
    let rest = item_id.strip_prefix("skill:")?;
    let (source, dir_name) = rest.split_once(':')?;
    if source.is_empty() || dir_name.is_empty() {
        return None;
    }
    Some((source, dir_name))
}

pub fn lookup_skill_config_override<'a>(
    configs: &'a std::collections::HashMap<String, SkillConfig>,
    source: &str,
    dir_name: &str,
) -> Option<&'a SkillConfig> {
    let new_key = config_key(source, dir_name);
    configs.get(&new_key).or_else(|| {
        dir_name
            .strip_prefix("builtin/")
            .and_then(|legacy_name| configs.get(&config_key(source, legacy_name)))
    })
}

// ── Scanning ─────────────────────────────────────────────────

fn find_skill_dir(knowledge_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let canonical = knowledge_dir.join(SKILL_DIR_NAME);
    canonical.is_dir().then_some(canonical)
}

fn scan_skill_dir(
    knowledge_dir: &std::path::Path,
    source: &str,
    configs: &std::collections::HashMap<String, SkillConfig>,
) -> Vec<SkillManifest> {
    let skill_dir = match find_skill_dir(knowledge_dir) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut manifests = Vec::new();
    let mut files = WalkDir::new(&skill_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
                return None;
            }
            let relative_path = path
                .strip_prefix(&skill_dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let dir_name = relative_path.strip_suffix(".md")?.to_string();
            if dir_name.trim().is_empty() {
                return None;
            }
            Some((path.to_path_buf(), relative_path, dir_name))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.1.cmp(&right.1));

    for (path, document_path, dir_name) in files {
        let rel_path = format!("{}/{}", SKILL_DIR_NAME, document_path);
        let Ok(document) = knowledge_store::load_document_by_root(
            knowledge_dir,
            KnowledgeType::Skill,
            &document_path,
        ) else {
            continue;
        };
        let cfg = if source == "app" {
            lookup_skill_config_override(configs, source, &dir_name)
        } else {
            None
        };
        manifests.push(build_skill_manifest(
            &document,
            &dir_name,
            source,
            &rel_path,
            get_updated_at(&path),
            cfg,
        ));
    }

    manifests
}

fn command_trigger_has_boundary(value: &str) -> bool {
    value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '。'
                    | '！'
                    | '？'
                    | '!'
                    | '?'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '"'
                    | '“'
                    | '”'
                    | '\''
                    | '‘'
                    | '’'
            )
    })
}

fn validate_normalized_command_trigger(value: &str) -> Result<(), String> {
    let normalized = value.trim();
    if !normalized.starts_with('/') || normalized.len() <= 1 {
        return Err("Command trigger must be a single / command token.".to_string());
    }
    if command_trigger_has_boundary(&normalized[1..]) {
        return Err("Command trigger must be a single / command token.".to_string());
    }
    Ok(())
}

pub(crate) fn normalize_command_trigger(value: &str, fallback: &str) -> String {
    let seed = if value.trim().is_empty() {
        fallback.trim()
    } else {
        value.trim()
    };
    let trimmed = seed.trim_start_matches('/').trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{}", trimmed)
    }
}

pub(crate) fn normalize_and_validate_command_trigger(
    value: &str,
    fallback: &str,
) -> Result<String, String> {
    let normalized = normalize_command_trigger(value, fallback);
    validate_normalized_command_trigger(&normalized)?;
    Ok(normalized)
}

fn resolve_config_command_trigger(config: &SkillConfig) -> Option<String> {
    let value = config.command_trigger.trim();
    if value.is_empty() {
        None
    } else {
        Some(normalize_command_trigger(value, ""))
    }
}

pub(crate) fn normalize_and_validate_skill_config(
    config: &SkillConfig,
    fallback: &str,
) -> Result<SkillConfig, String> {
    let mut normalized = config.clone();
    normalized.command_trigger = normalized.command_trigger.trim().to_string();
    if !normalized.command_trigger.is_empty()
        || (normalized.enabled && normalized.surface.allows_command())
    {
        normalized.command_trigger =
            normalize_and_validate_command_trigger(&normalized.command_trigger, fallback)?;
    }
    Ok(normalized)
}

pub(crate) fn fallback_command_name_for_skill_ref(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    let without_source = normalized
        .split_once(":skill/")
        .map(|(_, rest)| rest)
        .unwrap_or(&normalized);
    let without_type = without_source
        .strip_prefix("skill/")
        .unwrap_or(without_source);
    let without_suffix = without_type
        .strip_suffix("/SKILL.md")
        .or_else(|| without_type.strip_suffix("/SKILL"))
        .or_else(|| without_type.strip_suffix(".md"))
        .unwrap_or(without_type);
    let leaf_or_package = without_suffix
        .rsplit('/')
        .next()
        .unwrap_or(without_suffix)
        .trim();
    default_package_command_name(leaf_or_package)
}

fn validated_skill_config_override(
    configs: &std::collections::HashMap<String, SkillConfig>,
    source: &str,
    dir_name: &str,
    fallback: &str,
) -> Result<Option<SkillConfig>, String> {
    lookup_skill_config_override(configs, source, dir_name)
        .map(|config| normalize_and_validate_skill_config(config, fallback))
        .transpose()
}

fn validate_skill_document_config(
    document: &KnowledgeDocument,
    fallback: &str,
) -> Result<(), String> {
    let skill_enabled = document.skill_enabled.unwrap_or(true);
    let skill_surface = document.skill_surface.unwrap_or_default();
    let trigger = document.command_trigger.as_deref().unwrap_or("");
    if !trigger.trim().is_empty()
        || document.command_enabled
        || (skill_enabled && skill_surface.allows_command())
    {
        normalize_and_validate_command_trigger(trigger, fallback)?;
    }
    Ok(())
}

fn resolve_document_command_trigger(document: &KnowledgeDocument, fallback: &str) -> String {
    normalize_command_trigger(document.command_trigger.as_deref().unwrap_or(""), fallback)
}

fn build_skill_manifest(
    document: &KnowledgeDocument,
    dir_name: &str,
    source: &str,
    rel_path: &str,
    updated_at: i64,
    override_config: Option<&SkillConfig>,
) -> SkillManifest {
    let skill_enabled = override_config
        .map(|config| config.enabled)
        .unwrap_or_else(|| document.skill_enabled.unwrap_or(true));
    let skill_surface = override_config
        .map(|config| config.surface)
        .unwrap_or_else(|| document.skill_surface.unwrap_or_default());
    let manifest_description = knowledge_store::active_summary(document)
        .unwrap_or_default()
        .to_string();
    let skill_description = override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .or_else(|| {
            (!manifest_description.trim().is_empty()).then(|| manifest_description.clone())
        });
    let command_trigger = override_config
        .and_then(resolve_config_command_trigger)
        .unwrap_or_else(|| resolve_document_command_trigger(document, &document.title));

    SkillManifest {
        name: document.title.clone(),
        description: manifest_description,
        argument_hint: document.argument_hint.clone().unwrap_or_default(),
        dir_name: dir_name.to_string(),
        source: source.to_string(),
        rel_path: rel_path.to_string(),
        updated_at,
        skill_enabled,
        skill_surface,
        skill_description,
        command_trigger,
        tools: document.tools.clone(),
        kind: SkillManifestKind::Document,
        package_id: None,
        package_version: None,
        has_unity: false,
        has_l0: true,
        has_l1: true,
        has_l2: true,
    }
}

fn normalize_package_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
        || id.ends_with('.')
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err("Invalid skill package id".to_string());
    }
    Ok(id.to_string())
}

fn normalize_default_skill_package_namespace(value: &str) -> Result<String, String> {
    let namespace = value.trim();
    if namespace.is_empty() {
        return Ok(String::new());
    }
    normalize_package_id(namespace)
        .map_err(|_| "Invalid default skill package namespace".to_string())
}

fn skill_package_slug_from_name(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = true;
    for ch in value.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn resolve_skill_create_package_id(
    package_id: Option<String>,
    _default_namespace: Option<&str>,
    name: &str,
) -> Result<String, String> {
    if let Some(package_id) = optional_trimmed(package_id) {
        return normalize_package_id(&package_id);
    }

    let slug = skill_package_slug_from_name(name);
    if slug.is_empty() {
        return Err(
            "Cannot derive Skill package id from package name; provide packageId.".to_string(),
        );
    }
    normalize_package_id(&slug)
}

fn normalize_package_rel_path(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.contains("..")
        || normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("Invalid package relative path: {}", value));
    }
    Ok(normalized)
}

fn package_root_doc_rel_path(_manifest: &SkillPackageManifestFile) -> String {
    "SKILL.md".to_string()
}

fn package_doc_rel_path_for_virtual_path(
    manifest: &SkillPackageManifestFile,
    virtual_path: &str,
) -> Result<Option<String>, String> {
    let normalized = normalize_package_rel_path(virtual_path)?;
    let package_id = normalize_package_id(&manifest.id)?;
    if normalized == package_id || normalized == format!("skill/{}", package_id) {
        return Ok(Some(package_root_doc_rel_path(manifest)));
    }

    let Some(rest) = normalized
        .strip_prefix(&format!("{}/", package_id))
        .or_else(|| normalized.strip_prefix(&format!("skill/{}/", package_id)))
    else {
        return Ok(None);
    };

    if rest.eq_ignore_ascii_case("SKILL.md") {
        return Ok(Some(package_root_doc_rel_path(manifest)));
    }
    Ok(Some(rest.to_string()))
}

fn package_file_path(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let rel_path = normalize_package_rel_path(rel_path)?;
    Ok(root.join(rel_path))
}

fn package_file_modified_at(path: &Path, fallback: i64) -> i64 {
    get_updated_at(path).max(fallback)
}

fn package_doc_is_root(manifest: &SkillPackageManifestFile, doc_rel_path: &str) -> bool {
    doc_rel_path == package_root_doc_rel_path(manifest)
}

fn package_document_virtual_path(
    manifest: &SkillPackageManifestFile,
    doc_rel_path: &str,
) -> String {
    if package_doc_is_root(manifest, doc_rel_path) {
        format!("{}/SKILL.md", manifest.id)
    } else {
        format!("{}/{}", manifest.id, doc_rel_path)
    }
}

fn package_document_title(manifest: &SkillPackageManifestFile, doc_rel_path: &str) -> String {
    if package_doc_is_root(manifest, doc_rel_path) {
        return manifest.name.clone();
    }
    Path::new(doc_rel_path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(doc_rel_path)
        .to_string()
}

fn package_manifest_path(root: &Path) -> PathBuf {
    root.join(SKILL_PACKAGE_MANIFEST_FILE_NAME)
}

fn package_root_doc_path(root: &Path) -> PathBuf {
    root.join(SKILL_PACKAGE_ROOT_DOC_FILE_NAME)
}

fn is_skill_package_root(root: &Path) -> bool {
    root.is_dir() && package_manifest_path(root).is_file() && package_root_doc_path(root).is_file()
}

pub(crate) fn app_skill_package_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    let candidates: Vec<PathBuf> = Vec::new();

    #[cfg(not(test))]
    let candidates: Vec<PathBuf> = {
        let mut candidates = Vec::new();
        if let Ok(config_dir) = super::persistent_config_dir() {
            candidates.push(config_dir.join("skills"));
        }
        #[cfg(debug_assertions)]
        {
            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            candidates.push(manifest_dir.join("..").join("skills"));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("skills"));
            }
        }
        candidates
    };

    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|path| path.is_dir())
        .filter(|path| {
            let key = dunce::canonicalize(path)
                .unwrap_or_else(|_| path.clone())
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            seen.insert(key)
        })
        .collect()
}

pub(crate) fn writable_app_skill_package_dir() -> Result<PathBuf, String> {
    let path = super::persistent_config_dir()?.join("skills");
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create app Skill package directory: {}", e))?;
    Ok(path)
}

fn normalize_package_manifest(
    mut manifest: SkillPackageManifestFile,
    root: &Path,
) -> Result<SkillPackageManifestFile, String> {
    if manifest.id.trim().is_empty() {
        return Err(format!(
            "{} is missing required field 'id'",
            root.join(SKILL_PACKAGE_MANIFEST_FILE_NAME).display()
        ));
    }
    manifest.id = normalize_package_id(&manifest.id)?;
    if manifest.name.trim().is_empty() {
        manifest.name = manifest.id.clone();
    } else {
        manifest.name = manifest.name.trim().to_string();
    }
    manifest.schema = manifest.schema.trim().to_string();
    if manifest.schema.is_empty() {
        manifest.schema = "locus.skill.v1".to_string();
    }
    manifest.description = manifest.description.trim().to_string();
    manifest.version = manifest.version.trim().to_string();
    manifest.argument_hint = manifest
        .argument_hint
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(command) = manifest.command.as_mut() {
        let fallback_trigger = default_package_command_name(&manifest.id);
        command.trigger = command
            .trigger
            .take()
            .map(|value| normalize_and_validate_command_trigger(&value, &fallback_trigger))
            .transpose()?
            .filter(|value| !value.is_empty());
        command.argument_hint = command
            .argument_hint
            .take()
            .or_else(|| manifest.argument_hint.clone())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    } else if manifest.user_invocable.unwrap_or(true) || manifest.argument_hint.is_some() {
        manifest.command = Some(SkillPackageCommand {
            enabled: manifest.user_invocable,
            trigger: None,
            argument_hint: manifest.argument_hint.clone(),
        });
    }
    for item in &manifest.capabilities.unity {
        normalize_package_rel_path(&item.path)?;
    }
    for tool in manifest.tools.iter_mut() {
        normalize_package_tool_manifest(tool)?;
    }
    Ok(manifest)
}

fn normalize_package_tool_manifest(tool: &mut SkillPackageToolManifest) -> Result<(), String> {
    tool.name = tool.name.trim().to_string();
    if tool.name.is_empty() {
        return Err("Skill package tool name is required".to_string());
    }

    tool.runtime = tool.runtime.trim().to_ascii_lowercase();
    if !matches!(tool.runtime.as_str(), "python" | "bash" | "cli" | "unity") {
        return Err(format!(
            "Skill package tool '{}' has invalid runtime '{}'",
            tool.name, tool.runtime
        ));
    }

    tool.description = tool.description.trim().to_string();
    tool.path = tool
        .path
        .take()
        .map(|value| normalize_package_rel_path(&value))
        .transpose()?;
    tool.command = tool
        .command
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    tool.args = tool
        .args
        .drain(..)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    tool.input = tool
        .input
        .take()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    tool.output = tool
        .output
        .take()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    tool.type_name = tool
        .type_name
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    tool.method = tool
        .method
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    tool.request_editor_status = tool
        .request_editor_status
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    match tool.runtime.as_str() {
        "python" => {
            if tool.path.is_none() {
                return Err(format!(
                    "Skill package Python tool '{}' requires 'path'",
                    tool.name
                ));
            }
        }
        "bash" | "cli" => {
            if tool.path.is_none() && tool.command.is_none() {
                return Err(format!(
                    "Skill package {} tool '{}' requires 'path' or 'command'",
                    tool.runtime, tool.name
                ));
            }
        }
        "unity" => {
            if tool.method.is_none() {
                return Err(format!(
                    "Skill package Unity tool '{}' requires 'method'",
                    tool.name
                ));
            }
            if tool.path.is_none() && tool.type_name.is_none() {
                return Err(format!(
                    "Skill package Unity tool '{}' requires 'path' for dynamic execution or 'typeName' for an existing loaded type",
                    tool.name
                ));
            }
            tool.entry_type = tool
                .entry_type
                .take()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(status) = tool.request_editor_status.as_deref() {
                if status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
                    || !crate::unity_bridge::is_known_editor_status(status)
                {
                    return Err(format!(
                        "Skill package Unity tool '{}' has invalid requestEditorStatus '{}'",
                        tool.name, status
                    ));
                }
            }
        }
        _ => unreachable!("runtime checked above"),
    }

    match tool.input.as_deref().unwrap_or("json-stdin") {
        "json-stdin" | "argv-json" | "none" => {}
        other => {
            return Err(format!(
                "Skill package tool '{}' has invalid input '{}'",
                tool.name, other
            ));
        }
    }
    match tool.output.as_deref().unwrap_or("text") {
        "text" | "json-stdout" => {}
        other => {
            return Err(format!(
                "Skill package tool '{}' has invalid output '{}'",
                tool.name, other
            ));
        }
    }
    if !tool.parameters.is_object() {
        return Err(format!(
            "Skill package tool '{}' parameters must be a JSON object schema",
            tool.name
        ));
    }

    Ok(())
}

fn markdown_has_l_section(body: &str, level: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            return false;
        }
        let title = trimmed.trim_start_matches('#').trim_start();
        title == level
            || title.strip_prefix(level).is_some_and(|rest| {
                rest.starts_with(' ') || rest.starts_with(':') || rest.starts_with('-')
            })
    })
}

fn strip_utf8_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

fn split_optional_package_frontmatter(
    content: &str,
) -> Result<(SkillPackageDocumentFrontmatter, String), String> {
    let content = strip_utf8_bom(content);
    let normalized = content.strip_prefix("\r\n").unwrap_or(content);
    let Some(after_open) = normalized
        .strip_prefix("---\r\n")
        .or_else(|| normalized.strip_prefix("---\n"))
    else {
        return Ok((
            SkillPackageDocumentFrontmatter::default(),
            content.to_string(),
        ));
    };

    let (yaml, rest) = if let Some(index) = after_open.find("\r\n---\r\n") {
        (&after_open[..index], &after_open[index + 7..])
    } else if let Some(index) = after_open.find("\n---\n") {
        (&after_open[..index], &after_open[index + 5..])
    } else {
        return Err("Skill package document frontmatter is not terminated".to_string());
    };
    let frontmatter = serde_yaml::from_str::<SkillPackageDocumentFrontmatter>(yaml)
        .map_err(|e| format!("Failed to parse Skill package document frontmatter: {}", e))?;
    Ok((frontmatter, rest.to_string()))
}

fn normalize_package_document_tool_names(
    record: &SkillPackageRecord,
    values: &[String],
) -> Vec<String> {
    let built_ins = crate::tool::built_in_tool_name_keys();
    let mut names = values
        .iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }
            let lower = trimmed.to_ascii_lowercase();
            if built_ins.contains(&lower) {
                return Some(lower);
            }
            if let Some(tool) = record
                .manifest
                .tools
                .iter()
                .find(|tool| tool.name.eq_ignore_ascii_case(trimmed))
            {
                return Some(package_tool_api_name(&record.manifest.id, &tool.name));
            }
            canonical_skill_package_tool_name(trimmed).or_else(|| Some(trimmed.to_string()))
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn package_manifest_tool_names(record: &SkillPackageRecord) -> Vec<String> {
    let mut names = record
        .manifest
        .tools
        .iter()
        .map(|tool| package_tool_api_name(&record.manifest.id, &tool.name))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn package_document_tool_names(
    record: &SkillPackageRecord,
    doc_rel_path: &str,
    frontmatter: &SkillPackageDocumentFrontmatter,
) -> Vec<String> {
    let mut names = if package_doc_is_root(&record.manifest, doc_rel_path) {
        package_manifest_tool_names(record)
    } else {
        Vec::new()
    };
    names.extend(normalize_package_document_tool_names(
        record,
        &frontmatter.tools,
    ));
    names.sort();
    names.dedup();
    names
}

fn scan_package_document_levels(body: &str) -> SkillPackageDocLevels {
    SkillPackageDocLevels {
        has_l0: markdown_has_l_section(body, "L0"),
        has_l1: markdown_has_l_section(body, "L1"),
        has_l2: markdown_has_l_section(body, "L2"),
    }
}

fn load_skill_package_record(root: &Path) -> Result<SkillPackageRecord, String> {
    let manifest_path = package_manifest_path(root);
    let raw_manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {}", manifest_path.display(), e))?;
    let manifest: SkillPackageManifestFile = serde_json::from_str(&raw_manifest)
        .map_err(|e| format!("Invalid skill package manifest: {}", e))?;
    let manifest = normalize_package_manifest(manifest, root)?;

    let root_doc_path = package_root_doc_path(root);
    let raw = std::fs::read_to_string(&root_doc_path)
        .map_err(|e| format!("Failed to read {}: {}", root_doc_path.display(), e))?;
    let (_, body) = split_optional_package_frontmatter(&raw)?;
    let doc_levels = scan_package_document_levels(&body);
    let updated_at = get_updated_at(&manifest_path).max(get_updated_at(&root_doc_path));
    Ok(SkillPackageRecord {
        root: root.to_path_buf(),
        updated_at,
        doc_levels,
        manifest,
    })
}

pub(crate) fn list_skill_packages_sync() -> Vec<SkillPackageRecord> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();

    for package_dir in app_skill_package_dirs() {
        let Ok(entries) = std::fs::read_dir(&package_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let root = entry.path();
            if !is_skill_package_root(&root) {
                continue;
            }
            let Ok(record) = load_skill_package_record(&root) else {
                continue;
            };
            if seen.insert(record.manifest.id.clone()) {
                records.push(record);
            }
        }
    }

    records.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    records
}

fn find_skill_package(package_id: &str) -> Result<SkillPackageRecord, String> {
    let normalized_id = normalize_package_id(package_id)?;
    for package_dir in app_skill_package_dirs() {
        let direct_root = package_dir.join(&normalized_id);
        if is_skill_package_root(&direct_root) {
            return load_skill_package_record(&direct_root)
                .map_err(|error| format!("Invalid Skill package '{}': {}", normalized_id, error));
        }
    }

    for package_dir in app_skill_package_dirs() {
        let Ok(entries) = std::fs::read_dir(&package_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let root = entry.path();
            if !is_skill_package_root(&root) {
                continue;
            }
            let Ok(record) = load_skill_package_record(&root) else {
                continue;
            };
            if record.manifest.id == normalized_id {
                return Ok(record);
            }
        }
    }

    Err(format!("Skill package not found: {}", normalized_id))
}

fn find_skill_package_in_parent(
    package_parent: &Path,
    package_id: &str,
) -> Result<SkillPackageRecord, String> {
    let normalized_id = normalize_package_id(package_id)?;
    let direct_root = package_parent.join(&normalized_id);
    if is_skill_package_root(&direct_root) {
        return load_skill_package_record(&direct_root)
            .map_err(|error| format!("Invalid Skill package '{}': {}", normalized_id, error));
    }

    let entries = std::fs::read_dir(package_parent).map_err(|e| {
        format!(
            "Failed to read Skill package directory '{}': {}",
            package_parent.display(),
            e
        )
    })?;
    for entry in entries.flatten() {
        let root = entry.path();
        if !is_skill_package_root(&root) {
            continue;
        }
        let Ok(record) = load_skill_package_record(&root) else {
            continue;
        };
        if record.manifest.id == normalized_id {
            return Ok(record);
        }
    }

    Err(format!("Skill package not found: {}", normalized_id))
}

pub fn resolve_skill_package_root_sync(package_id: &str) -> Result<PathBuf, String> {
    find_skill_package(package_id).map(|record| record.root)
}

pub(crate) fn resolve_skill_package_document_path_sync(
    virtual_path: &str,
) -> Result<Option<PathBuf>, String> {
    for record in list_skill_packages_sync() {
        let Some(doc_rel_path) =
            package_doc_rel_path_for_virtual_path(&record.manifest, virtual_path)?
        else {
            continue;
        };
        let file_path = package_file_path(&record.root, &doc_rel_path)?;
        if file_path.is_file() {
            return Ok(Some(file_path));
        }
        return Err(format!(
            "Skill package document not found: {}",
            virtual_path
        ));
    }
    Ok(None)
}

fn sanitize_tool_name_segment(value: &str) -> String {
    let mut out = String::new();
    let mut previous_underscore = false;
    for ch in value.trim().chars() {
        let next = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if next == '_' {
            if previous_underscore {
                continue;
            }
            previous_underscore = true;
        } else {
            previous_underscore = false;
        }
        out.push(next);
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    }
}

fn truncate_ascii_segment(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    value.chars().take(max_len).collect()
}

const MAX_PACKAGE_TOOL_API_NAME_LEN: usize = 64;

fn package_tool_name_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn package_tool_hash_suffix(package_id: &str, tool_name: &str, salt: &str) -> String {
    let hash = blake3::hash(format!("{}:{}:{}", package_id, tool_name, salt).as_bytes())
        .to_hex()
        .to_string();
    format!("_{}", &hash[..8])
}

fn truncate_package_tool_api_name(name: String, package_id: &str, tool_name: &str) -> String {
    if name.len() <= MAX_PACKAGE_TOOL_API_NAME_LEN {
        return name;
    }

    let suffix = package_tool_hash_suffix(package_id, tool_name, &name);
    let budget = MAX_PACKAGE_TOOL_API_NAME_LEN
        .saturating_sub(suffix.len())
        .max(1);
    format!("{}{}", truncate_ascii_segment(&name, budget), suffix)
}

fn package_tool_short_api_name(tool_name: &str) -> String {
    let tool_segment = sanitize_tool_name_segment(tool_name);
    truncate_package_tool_api_name(tool_segment, "", tool_name)
}

fn package_tool_qualified_api_name(package_id: &str, tool_name: &str) -> String {
    let package_segment = sanitize_tool_name_segment(package_id);
    let tool_segment = sanitize_tool_name_segment(tool_name);
    truncate_package_tool_api_name(
        format!("{}_{}", package_segment, tool_segment),
        package_id,
        tool_name,
    )
}

fn legacy_package_tool_api_name(package_id: &str, tool_name: &str) -> String {
    const MAX_TOOL_NAME_LEN: usize = 64;
    let package_segment = sanitize_tool_name_segment(package_id);
    let tool_segment = sanitize_tool_name_segment(tool_name);
    let name = format!("skill_{}__{}", package_segment, tool_segment);
    if name.len() <= MAX_TOOL_NAME_LEN {
        return name;
    }

    let hash = blake3::hash(format!("{}:{}", package_id, tool_name).as_bytes())
        .to_hex()
        .to_string();
    let suffix = format!("__{}", &hash[..8]);
    let reserved = "skill_".len() + "__".len() + suffix.len();
    let budget = MAX_TOOL_NAME_LEN.saturating_sub(reserved).max(2);
    let tool_budget = tool_segment.len().min(24).min(budget / 2);
    let package_budget = budget.saturating_sub(tool_budget).max(1);

    format!(
        "skill_{}__{}{}",
        truncate_ascii_segment(&package_segment, package_budget),
        truncate_ascii_segment(&tool_segment, tool_budget.max(1)),
        suffix
    )
}

fn default_package_tool_reserved_names() -> BTreeSet<String> {
    let mut names = crate::tool::built_in_tool_name_keys();
    names.insert("task".to_string());
    names
}

fn package_tool_record_key(package_id: &str, tool_name: &str) -> (String, String) {
    (package_id.to_string(), tool_name.to_string())
}

fn package_tool_unique_api_name(
    preferred: String,
    package_id: &str,
    tool_name: &str,
    used: &mut BTreeSet<String>,
) -> String {
    let preferred_key = package_tool_name_key(&preferred);
    if used.insert(preferred_key) {
        return preferred;
    }

    let qualified = package_tool_qualified_api_name(package_id, tool_name);
    for attempt in 0..128 {
        let suffix = package_tool_hash_suffix(package_id, tool_name, &attempt.to_string());
        let budget = MAX_PACKAGE_TOOL_API_NAME_LEN
            .saturating_sub(suffix.len())
            .max(1);
        let candidate = format!("{}{}", truncate_ascii_segment(&qualified, budget), suffix);
        if used.insert(package_tool_name_key(&candidate)) {
            return candidate;
        }
    }

    qualified
}

fn package_tool_api_names_for_records(
    records: &[SkillPackageRecord],
    reserved: &BTreeSet<String>,
) -> BTreeMap<(String, String), String> {
    let mut short_name_counts = BTreeMap::<String, usize>::new();
    let mut candidates = Vec::<(String, String, String)>::new();

    for record in records {
        for tool in &record.manifest.tools {
            let short_name = package_tool_short_api_name(&tool.name);
            *short_name_counts
                .entry(package_tool_name_key(&short_name))
                .or_insert(0) += 1;
            candidates.push((record.manifest.id.clone(), tool.name.clone(), short_name));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut used = reserved
        .iter()
        .map(|name| package_tool_name_key(name))
        .collect::<BTreeSet<_>>();
    let mut names = BTreeMap::new();

    for (package_id, tool_name, short_name) in candidates {
        let short_key = package_tool_name_key(&short_name);
        let short_conflicts = used.contains(&short_key)
            || short_name_counts.get(&short_key).copied().unwrap_or(0) > 1;
        let preferred = if short_conflicts {
            package_tool_qualified_api_name(&package_id, &tool_name)
        } else {
            short_name
        };
        let api_name = package_tool_unique_api_name(preferred, &package_id, &tool_name, &mut used);
        names.insert(package_tool_record_key(&package_id, &tool_name), api_name);
    }

    names
}

fn package_tool_api_name_with_records(
    package_id: &str,
    tool_name: &str,
    records: &[SkillPackageRecord],
    reserved: &BTreeSet<String>,
) -> String {
    let names = package_tool_api_names_for_records(records, reserved);
    names
        .get(&package_tool_record_key(package_id, tool_name))
        .cloned()
        .unwrap_or_else(|| {
            let short_name = package_tool_short_api_name(tool_name);
            if reserved.contains(&package_tool_name_key(&short_name)) {
                package_tool_qualified_api_name(package_id, tool_name)
            } else {
                short_name
            }
        })
}

fn package_tool_api_name(package_id: &str, tool_name: &str) -> String {
    let records = list_skill_packages_sync();
    package_tool_api_name_with_records(
        package_id,
        tool_name,
        &records,
        &default_package_tool_reserved_names(),
    )
}

pub(crate) fn register_skill_package_tools(registry: &mut ToolRegistry) -> usize {
    let mut count = 0usize;
    let records = list_skill_packages_sync();
    let tool_names =
        package_tool_api_names_for_records(&records, &default_package_tool_reserved_names());
    for record in records {
        for tool in &record.manifest.tools {
            let tool_name = tool_names
                .get(&package_tool_record_key(&record.manifest.id, &tool.name))
                .cloned()
                .unwrap_or_else(|| package_tool_api_name(&record.manifest.id, &tool.name));
            if registry.get(&tool_name).is_some() {
                eprintln!(
                    "[Locus] skipped duplicate Skill package tool '{}'",
                    tool_name
                );
                continue;
            }
            match build_skill_package_tool_def(&record, tool) {
                Ok(definition) => {
                    registry.register(definition);
                    count += 1;
                }
                Err(error) => eprintln!(
                    "[Locus] skipped Skill package tool '{}': {}",
                    tool_name, error
                ),
            }
        }
    }
    count
}

fn find_skill_package_tool_by_api_name(
    name: &str,
) -> Option<(SkillPackageRecord, SkillPackageToolManifest, String)> {
    let requested = name.trim().to_ascii_lowercase();
    if requested.is_empty() {
        return None;
    }
    let records = list_skill_packages_sync();
    let tool_names =
        package_tool_api_names_for_records(&records, &default_package_tool_reserved_names());
    for record in records {
        for tool in &record.manifest.tools {
            let api_name = tool_names
                .get(&package_tool_record_key(&record.manifest.id, &tool.name))
                .cloned()
                .unwrap_or_else(|| package_tool_api_name(&record.manifest.id, &tool.name));
            let legacy_name = legacy_package_tool_api_name(&record.manifest.id, &tool.name);
            if api_name.to_ascii_lowercase() == requested
                || legacy_name.to_ascii_lowercase() == requested
            {
                return Some((record.clone(), tool.clone(), api_name));
            }
        }
    }
    None
}

pub(crate) fn skill_package_tool_names_sync() -> Vec<String> {
    let records = list_skill_packages_sync();
    let names_by_key =
        package_tool_api_names_for_records(&records, &default_package_tool_reserved_names());
    let mut names = records
        .iter()
        .flat_map(|record| {
            record.manifest.tools.iter().filter_map(|tool| {
                names_by_key
                    .get(&package_tool_record_key(&record.manifest.id, &tool.name))
                    .cloned()
            })
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub(crate) fn skill_package_tool_names_for_package_sync(package_id: &str) -> Vec<String> {
    let Ok(record) = find_skill_package(package_id) else {
        return Vec::new();
    };
    let records = list_skill_packages_sync();
    let names_by_key =
        package_tool_api_names_for_records(&records, &default_package_tool_reserved_names());
    let mut names = record
        .manifest
        .tools
        .iter()
        .filter_map(|tool| {
            names_by_key
                .get(&package_tool_record_key(&record.manifest.id, &tool.name))
                .cloned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub(crate) fn canonical_skill_package_tool_name(name: &str) -> Option<String> {
    find_skill_package_tool_by_api_name(name).map(|(_, _, api_name)| api_name)
}

fn skill_package_tool_description(
    record: &SkillPackageRecord,
    tool: &SkillPackageToolManifest,
) -> String {
    if tool.description.trim().is_empty() {
        format!(
            "Run Skill package tool '{}' from '{}'.",
            tool.name, record.manifest.name
        )
    } else {
        format!(
            "{}\n\nSkill package: {} ({})",
            tool.description, record.manifest.name, record.manifest.id
        )
    }
}

pub(crate) fn skill_package_tool_description_sync(
    name: &str,
) -> Option<(String, serde_json::Value)> {
    let (record, tool, _) = find_skill_package_tool_by_api_name(name)?;
    Some((
        skill_package_tool_description(&record, &tool),
        tool.parameters,
    ))
}

pub(crate) fn resolve_skill_package_api_tool_sync(name: &str) -> Option<serde_json::Value> {
    let (record, tool, api_name) = find_skill_package_tool_by_api_name(name)?;
    Some(serde_json::json!({
        "type": "function",
        "function": {
            "name": api_name,
            "description": skill_package_tool_description(&record, &tool),
            "parameters": tool.parameters,
        }
    }))
}

pub(crate) async fn execute_skill_package_tool_by_api_name(
    name: &str,
    args: serde_json::Value,
    ctx: ToolExecutionContext,
) -> Option<ToolResult> {
    let (record, tool, _) = find_skill_package_tool_by_api_name(name)?;
    Some(execute_skill_package_tool(&record.root, &record.manifest.id, &tool, args, ctx).await)
}

fn build_skill_package_tool_def(
    record: &SkillPackageRecord,
    tool: &SkillPackageToolManifest,
) -> Result<ToolDef, String> {
    let name = package_tool_api_name(&record.manifest.id, &tool.name);
    let package_id = record.manifest.id.clone();
    let package_root = record.root.clone();
    let tool_manifest = tool.clone();
    let description = skill_package_tool_description(record, tool);
    let parameters = tool.parameters.clone();

    Ok(ToolDef {
        name,
        description,
        parameters,
        execute: Arc::new(move |args, ctx| {
            let package_root = package_root.clone();
            let package_id = package_id.clone();
            let tool_manifest = tool_manifest.clone();
            Box::pin(async move {
                execute_skill_package_tool(&package_root, &package_id, &tool_manifest, args, ctx)
                    .await
            })
        }),
    })
}

async fn execute_skill_package_tool(
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: serde_json::Value,
    ctx: ToolExecutionContext,
) -> ToolResult {
    let result = match tool.runtime.as_str() {
        "python" => {
            run_skill_package_python_tool(package_root, package_id, tool, &args, &ctx).await
        }
        "bash" => run_skill_package_bash_tool(package_root, package_id, tool, &args, &ctx).await,
        "cli" => run_skill_package_cli_tool(package_root, package_id, tool, &args, &ctx).await,
        "unity" => run_skill_package_unity_tool(package_root, package_id, tool, &args, &ctx).await,
        other => Err(format!(
            "Skill package tool '{}' has unsupported runtime '{}'",
            tool.name, other
        )),
    };

    match result {
        Ok(output) => ToolResult {
            output,
            is_error: false,
        },
        Err(output) => ToolResult {
            output,
            is_error: true,
        },
    }
}

fn skill_tool_timeout(tool: &SkillPackageToolManifest) -> Duration {
    Duration::from_millis(tool.timeout_ms.unwrap_or(120_000))
}

fn skill_tool_input_mode(tool: &SkillPackageToolManifest) -> &str {
    tool.input.as_deref().unwrap_or("json-stdin")
}

fn skill_tool_output_mode(tool: &SkillPackageToolManifest) -> &str {
    tool.output.as_deref().unwrap_or("text")
}

fn configure_skill_process_env(
    cmd: &mut tokio::process::Command,
    package_root: &Path,
    package_id: &str,
    ctx: &ToolExecutionContext,
    python: Option<&crate::python_runtime::ResolvedPythonRuntime>,
) {
    cmd.env("LOCUS_SKILL_PACKAGE_ROOT", package_root);
    cmd.env("LOCUS_SKILL_PACKAGE_ID", package_id);
    if let Some(working_dir) = ctx.working_dir.as_ref() {
        cmd.env("LOCUS_WORKING_DIR", working_dir);
    }
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");

    if let Some(python) = python {
        cmd.env("LOCUS_PYTHON", &python.path);
        if let Some(home) = python.home.as_ref() {
            cmd.env("PYTHONHOME", home);
        }
        if matches!(
            &python.source,
            crate::python_runtime::PythonRuntimeSource::Managed
        ) {
            cmd.env("PYTHONNOUSERSITE", "1");
            cmd.env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
            cmd.env("PIP_NO_WARN_SCRIPT_LOCATION", "1");
            if let Some(package_dir) = python.package_dir.as_ref() {
                cmd.env("PIP_TARGET", package_dir);
                if let Some(python_path) = crate::python_runtime::managed_python_path_env(
                    std::env::var_os("PYTHONPATH"),
                    python,
                ) {
                    cmd.env("PYTHONPATH", python_path);
                }
            }
        }
    }

    let mut path =
        augment_path_with_git(std::env::var_os("PATH")).or_else(|| std::env::var_os("PATH"));
    if let Some(python) = python {
        path = crate::python_runtime::prepend_python_to_path(path, &python.path);
    }
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
}

fn apply_skill_tool_input_args(
    cmd: &mut tokio::process::Command,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
) -> Result<Option<String>, String> {
    let payload = serde_json::to_string(args)
        .map_err(|e| format!("Failed to serialize Skill package tool arguments: {}", e))?;
    match skill_tool_input_mode(tool) {
        "json-stdin" => {
            cmd.stdin(Stdio::piped());
            Ok(Some(payload))
        }
        "argv-json" => {
            cmd.arg(payload);
            cmd.stdin(Stdio::null());
            Ok(None)
        }
        "none" => {
            cmd.stdin(Stdio::null());
            Ok(None)
        }
        other => Err(format!(
            "Skill package tool '{}' has invalid input mode '{}'",
            tool.name, other
        )),
    }
}

async fn run_skill_process(
    mut cmd: tokio::process::Command,
    stdin_payload: Option<String>,
    timeout: Duration,
    output_mode: &str,
) -> Result<String, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start Skill package tool process: {}", e))?;

    if let Some(payload) = stdin_payload {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| format!("Failed to write Skill package tool stdin: {}", e))?;
        }
    }

    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "Skill package tool process timed out after {}ms",
                timeout.as_millis()
            )
        })?
        .map_err(|e| format!("Failed to wait for Skill package tool process: {}", e))?;

    format_skill_process_output(output, output_mode)
}

fn format_skill_process_output(
    output: std::process::Output,
    output_mode: &str,
) -> Result<String, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        let mut combined = String::new();
        combined.push_str(&stdout);
        combined.push_str(&stderr);
        if combined.trim().is_empty() {
            combined = "(no output)".to_string();
        }
        return Err(format!(
            "Exit code: {}\n{}",
            exit_code,
            truncate_skill_tool_output(combined)
        ));
    }

    match output_mode {
        "json-stdout" => {
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                return Ok("(no output)".to_string());
            }
            let value = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|e| {
                format!(
                    "Skill package tool declared json-stdout but returned invalid JSON: {}",
                    e
                )
            })?;
            let mut text =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string());
            if !stderr.trim().is_empty() {
                text.push_str("\n\nstderr:\n");
                text.push_str(stderr.trim());
            }
            Ok(truncate_skill_tool_output(text))
        }
        _ => {
            let mut combined = String::new();
            combined.push_str(&stdout);
            combined.push_str(&stderr);
            if combined.trim().is_empty() {
                combined = "(no output)".to_string();
            }
            Ok(truncate_skill_tool_output(combined))
        }
    }
}

fn truncate_skill_tool_output(mut output: String) -> String {
    const MAX_OUTPUT_BYTES: usize = 50_000;
    if output.len() <= MAX_OUTPUT_BYTES {
        return output;
    }
    let total_bytes = output.len();
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    format!(
        "{}...\n\n(output truncated, {} bytes total)",
        output, total_bytes
    )
}

async fn run_skill_package_python_tool(
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> Result<String, String> {
    let script_rel = tool
        .path
        .as_deref()
        .ok_or_else(|| format!("Skill package Python tool '{}' is missing path", tool.name))?;
    let script_path = package_file_path(package_root, script_rel)?;
    if !script_path.is_file() {
        return Err(format!(
            "Skill package Python script not found: {}",
            script_rel
        ));
    }

    let python = crate::python_runtime::resolve_effective_python(ctx.app_handle.as_ref())
        .ok_or_else(|| {
            "Python runtime is unavailable for Skill package tool execution".to_string()
        })?;
    crate::python_runtime::ensure_runtime_package_environment(&python)?;

    let program = python.path.to_string_lossy().to_string();
    let mut cmd = async_command(&program);
    cmd.arg(&script_path)
        .args(&tool.args)
        .current_dir(package_root);
    configure_skill_process_env(&mut cmd, package_root, package_id, ctx, Some(&python));
    let stdin_payload = apply_skill_tool_input_args(&mut cmd, tool, args)?;
    run_skill_process(
        cmd,
        stdin_payload,
        skill_tool_timeout(tool),
        skill_tool_output_mode(tool),
    )
    .await
}

async fn run_skill_package_cli_tool(
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> Result<String, String> {
    let program = if let Some(path) = tool.path.as_deref() {
        let program_path = package_file_path(package_root, path)?;
        if !program_path.is_file() {
            return Err(format!("Skill package CLI not found: {}", path));
        }
        program_path.to_string_lossy().to_string()
    } else {
        tool.command
            .clone()
            .ok_or_else(|| format!("Skill package CLI tool '{}' is missing command", tool.name))?
    };

    let python = crate::python_runtime::resolve_effective_python(ctx.app_handle.as_ref());
    if let Some(python) = python.as_ref() {
        crate::python_runtime::ensure_runtime_package_environment(python)?;
    }

    let mut cmd = async_command(&program);
    cmd.args(&tool.args).current_dir(package_root);
    configure_skill_process_env(&mut cmd, package_root, package_id, ctx, python.as_ref());
    let stdin_payload = apply_skill_tool_input_args(&mut cmd, tool, args)?;
    run_skill_process(
        cmd,
        stdin_payload,
        skill_tool_timeout(tool),
        skill_tool_output_mode(tool),
    )
    .await
}

fn shell_quote_posix(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn run_skill_package_bash_tool(
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> Result<String, String> {
    let mut command = if let Some(command) = tool.command.as_ref() {
        command.clone()
    } else {
        let script_rel = tool
            .path
            .as_deref()
            .ok_or_else(|| format!("Skill package bash tool '{}' is missing path", tool.name))?;
        let script_path = package_file_path(package_root, script_rel)?;
        if !script_path.is_file() {
            return Err(format!(
                "Skill package bash script not found: {}",
                script_rel
            ));
        }
        shell_quote_posix(&script_path.to_string_lossy().replace('\\', "/"))
    };

    for arg in &tool.args {
        command.push(' ');
        command.push_str(&shell_quote_posix(arg));
    }

    let payload = serde_json::to_string(args)
        .map_err(|e| format!("Failed to serialize Skill package tool arguments: {}", e))?;
    let stdin_payload = match skill_tool_input_mode(tool) {
        "json-stdin" => Some(payload),
        "argv-json" => {
            command.push(' ');
            command.push_str(&shell_quote_posix(&payload));
            None
        }
        "none" => None,
        other => {
            return Err(format!(
                "Skill package tool '{}' has invalid input mode '{}'",
                tool.name, other
            ));
        }
    };

    let python = crate::python_runtime::resolve_effective_python(ctx.app_handle.as_ref());
    if let Some(python) = python.as_ref() {
        crate::python_runtime::ensure_runtime_package_environment(python)?;
    }

    let mut cmd = async_command("sh");
    cmd.arg("-c").arg(command).current_dir(package_root);
    if stdin_payload.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    configure_skill_process_env(&mut cmd, package_root, package_id, ctx, python.as_ref());
    run_skill_process(
        cmd,
        stdin_payload,
        skill_tool_timeout(tool),
        skill_tool_output_mode(tool),
    )
    .await
}

async fn run_skill_package_unity_tool(
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> Result<String, String> {
    let project_path = ctx
        .working_dir
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "Skill package Unity tool '{}' requires a selected Unity project working directory.",
                tool.name
            )
        })?;

    let requested_status = tool
        .request_editor_status
        .as_deref()
        .unwrap_or(crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING);
    let (connected, actual_status, _scene) =
        crate::unity_bridge::query_unity_status(project_path).await;
    if !connected {
        return Err("Unity Editor not connected".to_string());
    }
    if actual_status != requested_status {
        return Err(format!(
            "Unity Editor status is \"{}\". Skill package Unity tool '{}' requires \"{}\".",
            actual_status, tool.name, requested_status
        ));
    }

    if tool.path.is_some() {
        return run_skill_package_dynamic_unity_tool(
            project_path,
            package_root,
            package_id,
            tool,
            args,
        )
        .await;
    }

    run_skill_package_loaded_unity_tool(project_path, package_id, tool, args).await
}

async fn run_skill_package_dynamic_unity_tool(
    project_path: &str,
    package_root: &Path,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
) -> Result<String, String> {
    let script_rel = tool
        .path
        .as_deref()
        .ok_or_else(|| format!("Skill package Unity tool '{}' is missing path", tool.name))?;
    let script_path = package_file_path(package_root, script_rel)?;
    if !script_path.is_file() {
        return Err(format!(
            "Skill package Unity C# script not found: {}",
            script_rel
        ));
    }
    let entry_type = tool
        .entry_type
        .as_deref()
        .or(tool.type_name.as_deref())
        .map(str::to_string)
        .or_else(|| {
            Path::new(script_rel)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            format!(
                "Skill package Unity tool '{}' cannot infer entryType from path '{}'",
                tool.name, script_rel
            )
        })?;
    let method = tool
        .method
        .as_deref()
        .ok_or_else(|| format!("Skill package Unity tool '{}' is missing method", tool.name))?;

    let record = find_skill_package(package_id)?;
    let bundle = skill_package_unity_script_bundle_for_record(&record)?.ok_or_else(|| {
        format!(
            "Skill package '{}' has no Unity C# scripts to compile",
            package_id
        )
    })?;
    let compile_raw =
        crate::unity_bridge::compile_skill_package(project_path, &bundle.request).await?;
    let compile_json = serde_json::from_str::<serde_json::Value>(&compile_raw)
        .map_err(|error| format!("Failed to parse Skill C# compile response: {}", error))?;
    crate::unity_bridge::update_unity_type_index_after_skill_package_compile(
        project_path,
        &compile_json,
    )
    .await?;

    let assembly_id = compile_json
        .get("assemblyId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if assembly_id.is_empty() {
        return Err("Skill package compile response is missing assemblyId".to_string());
    }
    let payload =
        skill_package_invoke_payload(package_id, Some(assembly_id), &entry_type, method, args)?;
    let raw = crate::unity_bridge::invoke_skill_package(project_path, &payload).await?;

    Ok(format_json_or_text(&raw))
}

fn format_json_or_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "(no output)".to_string();
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| trimmed.to_string())
}

async fn run_skill_package_loaded_unity_tool(
    project_path: &str,
    package_id: &str,
    tool: &SkillPackageToolManifest,
    args: &serde_json::Value,
) -> Result<String, String> {
    let type_name = tool.type_name.as_deref().ok_or_else(|| {
        format!(
            "Skill package Unity tool '{}' is missing typeName",
            tool.name
        )
    })?;
    let method = tool
        .method
        .as_deref()
        .ok_or_else(|| format!("Skill package Unity tool '{}' is missing method", tool.name))?;
    let payload = skill_package_invoke_payload(package_id, None, type_name, method, args)?;
    let raw = crate::unity_bridge::invoke_skill_package(project_path, &payload).await?;
    Ok(format_json_or_text(&raw))
}

fn skill_package_invoke_payload(
    package_id: &str,
    assembly_id: Option<&str>,
    type_name: &str,
    method: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "packageId": package_id,
        "assemblyId": assembly_id.unwrap_or(""),
        "typeName": type_name,
        "method": method,
        "argsJson": serde_json::to_string(args)
            .map_err(|e| format!("Failed to serialize Skill package Unity arguments: {}", e))?,
    }))
}

fn package_source_summary(
    manifest: &SkillPackageManifestFile,
) -> Option<knowledge_store::KnowledgeExternalSource> {
    Some(knowledge_store::KnowledgeExternalSource {
        provider: knowledge_store::KnowledgeSourceProvider::Package,
        locator: manifest
            .source
            .as_ref()
            .and_then(|source| source.url.clone()),
        source_id: Some(manifest.id.clone()),
        sync_enabled: false,
    })
}

fn package_document_id(package_id: &str, doc_rel_path: &str) -> String {
    let normalized = package_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if doc_rel_path == SKILL_PACKAGE_ROOT_DOC_FILE_NAME {
        return format!("kd_skill_package_{}", normalized);
    }

    let mut rel_segment = doc_rel_path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if rel_segment.is_empty() {
        rel_segment = "file".to_string();
    }
    if rel_segment.len() > 48 {
        rel_segment = rel_segment.chars().take(48).collect();
    }
    let hash = blake3::hash(format!("{}:{}", package_id, doc_rel_path).as_bytes())
        .to_hex()
        .to_string();
    format!(
        "kd_skill_package_{}__{}_{}",
        normalized,
        rel_segment,
        &hash[..8]
    )
}

fn package_command_enabled(manifest: &SkillPackageManifestFile) -> bool {
    manifest
        .command
        .as_ref()
        .and_then(|item| item.enabled)
        .unwrap_or_else(|| manifest.user_invocable.unwrap_or(true))
}

fn package_auto_enabled(manifest: &SkillPackageManifestFile) -> bool {
    !manifest.disable_model_invocation.unwrap_or(false)
}

fn package_skill_enabled(manifest: &SkillPackageManifestFile) -> bool {
    package_command_enabled(manifest) || package_auto_enabled(manifest)
}

fn package_skill_surface(manifest: &SkillPackageManifestFile) -> SkillSurface {
    match (
        package_command_enabled(manifest),
        package_auto_enabled(manifest),
    ) {
        (true, true) => SkillSurface::Both,
        (true, false) => SkillSurface::Command,
        (false, true) => SkillSurface::Auto,
        (false, false) => SkillSurface::Command,
    }
}

fn skill_surface_allows_command(surface: SkillSurface) -> bool {
    matches!(surface, SkillSurface::Command | SkillSurface::Both)
}

fn configured_package_skill_enabled(
    manifest: &SkillPackageManifestFile,
    override_config: Option<&SkillConfig>,
) -> bool {
    override_config
        .map(|config| config.enabled)
        .unwrap_or_else(|| package_skill_enabled(manifest))
}

fn configured_package_skill_surface(
    manifest: &SkillPackageManifestFile,
    override_config: Option<&SkillConfig>,
) -> SkillSurface {
    override_config
        .map(|config| config.surface)
        .unwrap_or_else(|| package_skill_surface(manifest))
}

fn configured_package_command_enabled(
    manifest: &SkillPackageManifestFile,
    override_config: Option<&SkillConfig>,
) -> bool {
    configured_package_skill_enabled(manifest, override_config)
        && skill_surface_allows_command(configured_package_skill_surface(manifest, override_config))
}

fn configured_package_description(
    manifest: &SkillPackageManifestFile,
    override_config: Option<&SkillConfig>,
) -> String {
    override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .unwrap_or_else(|| manifest.description.clone())
}

fn configured_package_command_trigger(
    manifest: &SkillPackageManifestFile,
    override_config: Option<&SkillConfig>,
) -> String {
    override_config
        .and_then(resolve_config_command_trigger)
        .unwrap_or_else(|| package_command_trigger(manifest))
}

fn configured_package_inject_mode(override_config: Option<&SkillConfig>) -> KnowledgeInjectMode {
    override_config
        .map(|config| config.inject_mode)
        .unwrap_or(KnowledgeInjectMode::None)
}

fn package_argument_hint(manifest: &SkillPackageManifestFile) -> Option<String> {
    manifest
        .command
        .as_ref()
        .and_then(|item| item.argument_hint.clone())
        .or_else(|| manifest.argument_hint.clone())
}

fn package_command_trigger(manifest: &SkillPackageManifestFile) -> String {
    normalize_command_trigger(
        manifest
            .command
            .as_ref()
            .and_then(|item| item.trigger.as_deref())
            .unwrap_or(""),
        &default_package_command_name(&manifest.id),
    )
}

fn package_to_document(
    record: &SkillPackageRecord,
    doc_rel_path: &str,
    raw_body: String,
    override_config: Option<&SkillConfig>,
) -> Result<KnowledgeDocument, String> {
    let (frontmatter, body) = split_optional_package_frontmatter(&raw_body)?;
    let manifest = &record.manifest;
    let is_root = package_doc_is_root(manifest, doc_rel_path);
    let command_enabled = is_root && configured_package_command_enabled(manifest, override_config);
    let skill_enabled = if is_root {
        configured_package_skill_enabled(manifest, override_config)
    } else {
        false
    };
    let skill_surface = if is_root {
        configured_package_skill_surface(manifest, override_config)
    } else {
        SkillSurface::Command
    };
    let description = if is_root {
        configured_package_description(manifest, override_config)
    } else {
        String::new()
    };
    let file_path = package_file_path(&record.root, doc_rel_path).ok();
    let updated_at = file_path
        .as_ref()
        .map(|path| package_file_modified_at(path, record.updated_at))
        .unwrap_or(record.updated_at);
    Ok(KnowledgeDocument {
        id: package_document_id(&manifest.id, doc_rel_path),
        doc_type: KnowledgeType::Skill,
        path: package_document_virtual_path(manifest, doc_rel_path),
        title: frontmatter
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| package_document_title(manifest, doc_rel_path)),
        inject_mode: if is_root {
            configured_package_inject_mode(override_config)
        } else {
            KnowledgeInjectMode::None
        },
        inherit_inject_mode: false,
        inject_mode_source: Default::default(),
        summary_enabled: is_root,
        command_enabled,
        read_only: true,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::App,
        inherit_ai_config: false,
        ai_config_source: Default::default(),
        explicit_maintenance_rules: false,
        external_source: package_source_summary(manifest),
        skill_enabled: Some(skill_enabled),
        skill_surface: Some(skill_surface),
        command_trigger: is_root
            .then(|| configured_package_command_trigger(manifest, override_config)),
        argument_hint: is_root.then(|| package_argument_hint(manifest)).flatten(),
        tools: package_document_tool_names(record, doc_rel_path, &frontmatter),
        summary: (!description.trim().is_empty()).then_some(description),
        body,
        maintenance_rules: None,
        created_at: updated_at,
        updated_at,
    })
}

pub(crate) fn read_skill_package_document_sync(
    working_dir: &str,
    virtual_path: &str,
    part: &str,
) -> Result<Option<knowledge_store::KnowledgeReadResult>, String> {
    let normalized_part = match part.trim() {
        "" | "full" => "full",
        "summary" => "summary",
        "body" => "body",
        other => {
            return Err(format!(
                "knowledge_read part must be one of: full, summary, body (got '{}')",
                other
            ))
        }
    };

    let configs = load_skill_config(working_dir);
    for record in list_skill_packages_sync() {
        let Some(doc_rel_path) =
            package_doc_rel_path_for_virtual_path(&record.manifest, virtual_path)?
        else {
            continue;
        };
        let config = lookup_skill_config_override(&configs, "app", &record.manifest.id);
        let file_path = package_file_path(&record.root, &doc_rel_path)?;
        if !file_path.is_file() {
            return Err(format!(
                "Skill package document not found: {}",
                virtual_path
            ));
        }
        let raw = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read skill package document: {}", e))?;
        let body = strip_utf8_bom(&raw).to_string();
        let mut document = package_to_document(&record, &doc_rel_path, body, config)?;
        match normalized_part {
            "full" => {}
            "summary" => {
                document.body.clear();
                document.maintenance_rules = None;
                document.explicit_maintenance_rules = false;
            }
            "body" => {
                document.summary = None;
                document.summary_enabled = false;
                document.maintenance_rules = None;
                document.explicit_maintenance_rules = false;
            }
            _ => unreachable!("normalized_part only returns known values"),
        }
        return Ok(Some(knowledge_store::KnowledgeReadResult {
            document,
            part: normalized_part.to_string(),
            file_metadata: None,
        }));
    }

    Ok(None)
}

fn package_unity_script_rel_paths(record: &SkillPackageRecord) -> Result<Vec<String>, String> {
    let mut paths = BTreeSet::new();

    for capability in &record.manifest.capabilities.unity {
        if capability.path.to_ascii_lowercase().ends_with(".cs") {
            paths.insert(normalize_package_rel_path(&capability.path)?);
        }
    }

    for tool in &record.manifest.tools {
        if tool.runtime == "unity" {
            if let Some(path) = tool.path.as_deref() {
                if path.to_ascii_lowercase().ends_with(".cs") {
                    paths.insert(normalize_package_rel_path(path)?);
                }
            }
        }
    }

    let unity_editor_root = record.root.join("unity").join("Editor");
    if unity_editor_root.is_dir() {
        for entry in WalkDir::new(&unity_editor_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let is_csharp = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("cs"))
                .unwrap_or(false);
            if !is_csharp {
                continue;
            }

            let rel = path
                .strip_prefix(&record.root)
                .map_err(|e| format!("Failed to resolve Skill package Unity script path: {}", e))?;
            paths.insert(normalize_package_rel_path(
                &rel.to_string_lossy().replace('\\', "/"),
            )?);
        }
    }

    Ok(paths.into_iter().collect())
}

pub(crate) fn skill_package_unity_script_bundle_for_document_sync(
    virtual_path: &str,
) -> Result<Option<SkillPackageUnityScriptBundle>, String> {
    for record in list_skill_packages_sync() {
        let Some(_doc_rel_path) =
            package_doc_rel_path_for_virtual_path(&record.manifest, virtual_path)?
        else {
            continue;
        };

        return skill_package_unity_script_bundle_for_record(&record);
    }

    Ok(None)
}

fn skill_package_unity_script_bundle_for_record(
    record: &SkillPackageRecord,
) -> Result<Option<SkillPackageUnityScriptBundle>, String> {
    let script_paths = package_unity_script_rel_paths(record)?;
    if script_paths.is_empty() {
        return Ok(None);
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(record.manifest.id.as_bytes());
    let mut scripts = Vec::with_capacity(script_paths.len());
    for rel_path in script_paths {
        let source_path = package_file_path(&record.root, &rel_path)?;
        let source = std::fs::read_to_string(&source_path).map_err(|e| {
            format!(
                "Failed to read Skill package Unity script '{}': {}",
                rel_path, e
            )
        })?;
        hasher.update(rel_path.as_bytes());
        hasher.update(source.as_bytes());
        scripts.push(serde_json::json!({
            "path": rel_path,
            "source": source,
        }));
    }

    let source_hash = hasher.finalize().to_hex().to_string();
    let script_count = scripts.len();
    Ok(Some(SkillPackageUnityScriptBundle {
        package_id: record.manifest.id.clone(),
        source_hash: source_hash.clone(),
        script_count,
        request: serde_json::json!({
            "packageId": record.manifest.id.clone(),
            "sourceHash": source_hash,
            "scripts": scripts,
        }),
    }))
}

fn package_to_list_item(
    record: &SkillPackageRecord,
    doc_rel_path: &str,
    override_config: Option<&SkillConfig>,
) -> knowledge_store::KnowledgeListItem {
    let manifest = &record.manifest;
    let is_root = package_doc_is_root(manifest, doc_rel_path);
    let command_enabled = is_root && configured_package_command_enabled(manifest, override_config);
    let skill_enabled = if is_root {
        configured_package_skill_enabled(manifest, override_config)
    } else {
        false
    };
    let skill_surface = if is_root {
        configured_package_skill_surface(manifest, override_config)
    } else {
        SkillSurface::Command
    };
    let description = if is_root {
        configured_package_description(manifest, override_config)
    } else {
        String::new()
    };
    let file_path = package_file_path(&record.root, doc_rel_path).ok();
    let body = file_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let (frontmatter, body) = split_optional_package_frontmatter(&body).unwrap_or_default();
    let updated_at = file_path
        .as_ref()
        .map(|path| package_file_modified_at(path, record.updated_at))
        .unwrap_or(record.updated_at);
    knowledge_store::KnowledgeListItem {
        id: package_document_id(&manifest.id, doc_rel_path),
        doc_type: KnowledgeType::Skill,
        path: package_document_virtual_path(manifest, doc_rel_path),
        title: frontmatter
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| package_document_title(manifest, doc_rel_path)),
        inject_mode: if is_root {
            configured_package_inject_mode(override_config)
        } else {
            KnowledgeInjectMode::None
        },
        summary_enabled: is_root,
        command_enabled,
        read_only: true,
        ai_maintained: false,
        explicit_maintenance_rules: false,
        storage_source: knowledge_store::KnowledgeStorageSource::App,
        external_source: package_source_summary(manifest),
        skill_enabled: Some(skill_enabled),
        skill_surface: Some(skill_surface),
        command_trigger: is_root
            .then(|| configured_package_command_trigger(manifest, override_config)),
        argument_hint: is_root.then(|| package_argument_hint(manifest)).flatten(),
        created_at: updated_at,
        updated_at,
        has_summary: !description.trim().is_empty(),
        has_body_content: !body.trim().is_empty(),
        byte_size: file_path
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len()),
        lexical_search_enabled: Some(false),
        semantic_search_enabled: Some(false),
        summary: (!description.trim().is_empty()).then_some(description),
    }
}

fn is_ignored_package_walk_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        name,
        "__pycache__" | ".git" | ".hg" | ".svn" | "node_modules" | ".venv" | "venv"
    ) || name.starts_with('.')
}

fn is_ignored_package_walk_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    name.starts_with('.')
}

fn list_package_document_rel_paths(record: &SkillPackageRecord) -> Vec<String> {
    let mut paths = BTreeSet::new();
    let root = &record.root;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !entry.file_type().is_dir() || !is_ignored_package_walk_dir(entry.path())
        })
        .flatten()
    {
        if !entry.file_type().is_file() || is_ignored_package_walk_file(entry.path()) {
            continue;
        }
        let Ok(rel_path) = entry.path().strip_prefix(root) else {
            continue;
        };
        let raw_rel_path = rel_path.to_string_lossy().replace('\\', "/");
        let Ok(normalized_rel_path) = normalize_package_rel_path(&raw_rel_path) else {
            continue;
        };
        if std::fs::read_to_string(entry.path()).is_err() {
            continue;
        }
        paths.insert(normalized_rel_path);
    }
    paths.into_iter().collect()
}

fn package_to_list_items(
    record: &SkillPackageRecord,
    override_config: Option<&SkillConfig>,
) -> Vec<knowledge_store::KnowledgeListItem> {
    list_package_document_rel_paths(record)
        .into_iter()
        .map(|doc_rel_path| package_to_list_item(record, &doc_rel_path, override_config))
        .collect()
}

pub(crate) fn list_skill_package_knowledge_items_sync(
    working_dir: &str,
    path_prefix: Option<&str>,
) -> Vec<knowledge_store::KnowledgeListItem> {
    let normalized_prefix = path_prefix
        .map(|value| {
            value
                .trim()
                .replace('\\', "/")
                .trim_matches('/')
                .to_string()
        })
        .unwrap_or_default();
    let configs = load_skill_config(working_dir);
    list_skill_packages_sync()
        .into_iter()
        .flat_map(|record| {
            let config = lookup_skill_config_override(&configs, "app", &record.manifest.id);
            package_to_list_items(&record, config)
        })
        .filter(|item| normalized_prefix.is_empty() || item.path.starts_with(&normalized_prefix))
        .collect()
}

fn build_package_skill_manifest(
    record: &SkillPackageRecord,
    source: &str,
    override_config: Option<&SkillConfig>,
) -> SkillManifest {
    let manifest = &record.manifest;
    let package_id = manifest.id.trim();
    let skill_enabled = override_config
        .map(|config| config.enabled)
        .unwrap_or_else(|| package_skill_enabled(manifest));
    let skill_surface = override_config
        .map(|config| config.surface)
        .unwrap_or_else(|| package_skill_surface(manifest));
    let manifest_description = manifest.description.trim().to_string();
    let skill_description = override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .or_else(|| (!manifest_description.is_empty()).then(|| manifest_description.clone()));
    let command_trigger = override_config
        .and_then(resolve_config_command_trigger)
        .unwrap_or_else(|| package_command_trigger(manifest));
    let root_doc_tools = std::fs::read_to_string(package_root_doc_path(&record.root))
        .ok()
        .and_then(|raw| split_optional_package_frontmatter(&raw).ok())
        .map(|(frontmatter, _)| {
            package_document_tool_names(record, &package_root_doc_rel_path(manifest), &frontmatter)
        })
        .unwrap_or_else(|| package_manifest_tool_names(record));

    SkillManifest {
        name: manifest.name.clone(),
        description: manifest_description,
        argument_hint: package_argument_hint(manifest).unwrap_or_default(),
        dir_name: package_id.to_string(),
        source: source.to_string(),
        rel_path: format!("{}/{}/SKILL.md", SKILL_DIR_NAME, package_id),
        updated_at: record.updated_at,
        skill_enabled,
        skill_surface,
        skill_description,
        command_trigger,
        tools: root_doc_tools,
        kind: SkillManifestKind::Package,
        package_id: Some(package_id.to_string()),
        package_version: (!manifest.version.trim().is_empty()).then(|| manifest.version.clone()),
        has_unity: !manifest.capabilities.unity.is_empty(),
        has_l0: record.doc_levels.has_l0,
        has_l1: record.doc_levels.has_l1,
        has_l2: record.doc_levels.has_l2,
    }
}

// ── Tauri commands ───────────────────────────────────────────

#[tauri::command]
pub async fn list_skills(
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<Vec<SkillManifest>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    Ok(list_skills_sync(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
    ))
}

pub fn list_skills_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Vec<SkillManifest> {
    let configs = load_skill_config(working_dir);
    let mut manifests = Vec::new();

    for package in list_skill_packages_sync() {
        let cfg = lookup_skill_config_override(&configs, "app", &package.manifest.id);
        manifests.push(build_package_skill_manifest(&package, "app", cfg));
    }

    if let Some(app_dir) = app_knowledge_dir {
        manifests.extend(scan_skill_dir(app_dir, "app", &configs));
    }

    let project_dir = std::path::Path::new(working_dir)
        .join("Locus")
        .join("knowledge");
    if project_dir.is_dir() {
        let project_skills = scan_skill_dir(&project_dir, "project", &configs);
        for ps in project_skills {
            manifests.retain(|m| !skill_manifest_overridden_by_project(m, &ps));
            manifests.push(ps);
        }
    }

    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    manifests
}

fn skill_manifest_overridden_by_project(existing: &SkillManifest, project: &SkillManifest) -> bool {
    if existing.dir_name == project.dir_name {
        return true;
    }
    existing.source == "app" && existing.dir_name == format!("builtin/{}", project.dir_name)
}

fn normalize_skill_manifest_name(dir_name: &str) -> Result<String, String> {
    let normalized = dir_name.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    if normalized.is_empty()
        || normalized.contains("..")
        || normalized.split('/').any(|segment| {
            segment.is_empty()
                || segment == "."
                || segment == ".."
                || !segment.chars().all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_'
                })
        })
    {
        return Err("Invalid skill name".to_string());
    }
    Ok(normalized.to_string())
}

pub fn resolve_skill_manifest_path_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    dir_name: &str,
    source: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let normalized_dir_name = normalize_skill_manifest_name(dir_name)?;

    let src = source.unwrap_or("project");
    let knowledge_dir = if src == "app" {
        app_knowledge_dir
            .cloned()
            .ok_or_else(|| "App knowledge directory not found".to_string())?
    } else {
        std::path::Path::new(working_dir)
            .join("Locus")
            .join("knowledge")
    };

    let file_path = knowledge_dir
        .join(SKILL_DIR_NAME)
        .join(format!("{}.md", normalized_dir_name));
    if file_path.is_file() {
        return Ok(file_path);
    }
    if src == "app" && !normalized_dir_name.contains('/') {
        let builtin_file_path = knowledge_dir
            .join(SKILL_DIR_NAME)
            .join("builtin")
            .join(format!("{}.md", normalized_dir_name));
        if builtin_file_path.is_file() {
            return Ok(builtin_file_path);
        }
    }

    Err(format!("Skill not found: {}", normalized_dir_name))
}

pub fn read_skill_manifest_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    dir_name: &str,
    source: Option<&str>,
) -> Result<String, String> {
    if source.unwrap_or("project") == "app" {
        if let Ok(package_id) = normalize_package_id(dir_name) {
            if let Ok(record) = find_skill_package(&package_id) {
                let root_doc = package_root_doc_rel_path(&record.manifest);
                let path = package_file_path(&record.root, &root_doc)?;
                return std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read skill package root document: {}", e));
            }
        }
    }
    let path = resolve_skill_manifest_path_sync(working_dir, app_knowledge_dir, dir_name, source)?;
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read skill: {}", e))
}

#[tauri::command]
pub async fn read_skill_manifest(
    dir_name: String,
    source: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    read_skill_manifest_sync(
        &working_dir,
        app_knowledge_dir.0.as_ref().as_ref(),
        &dir_name,
        source.as_deref(),
    )
    .map_err(Into::into)
}

const COMMAND_SKILL_SCAFFOLD_BODY: &str = r#"## Instructions
"#;

const AUTO_SKILL_SCAFFOLD_BODY: &str = r#"## When to use

## When NOT to use

## Instructions
"#;

fn default_skill_scaffold_body(command_enabled: bool) -> &'static str {
    if command_enabled {
        COMMAND_SKILL_SCAFFOLD_BODY
    } else {
        AUTO_SKILL_SCAFFOLD_BODY
    }
}

fn is_valid_skill_scaffold_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn normalize_skill_create_request_path(
    request: &SkillCreateRequest,
) -> Result<(String, String), String> {
    let name = request.name.trim();
    if name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !is_valid_skill_scaffold_name(name)
    {
        return Err("Invalid skill name: use lowercase-kebab-case".to_string());
    }

    let raw_path = request
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}.md", name));
    let normalized_raw_path = raw_path.replace('\\', "/");
    let trimmed_path = normalized_raw_path.trim_start_matches('/');
    let without_type = trimmed_path
        .strip_prefix("skill/")
        .unwrap_or(trimmed_path)
        .to_string();
    let without_suffix = without_type
        .strip_suffix(".md")
        .unwrap_or(&without_type)
        .to_string();
    let dir_name = normalize_skill_manifest_name(&without_suffix)?;
    let leaf = dir_name.rsplit('/').next().unwrap_or(&dir_name);
    if leaf != name {
        return Err("Skill document path file name must match the skill name".to_string());
    }
    Ok((dir_name.clone(), format!("{}.md", dir_name)))
}

fn skill_title_from_name(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_tool_name_list(values: Vec<String>) -> Vec<String> {
    let mut names = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn required_skill_create_text(value: Option<String>, field: &str) -> Result<String, String> {
    optional_trimmed(value).ok_or_else(|| format!("'{}' parameter is required.", field))
}

fn default_package_command_name(package_id: &str) -> String {
    package_id
        .rsplit('.')
        .next()
        .unwrap_or(package_id)
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
        .to_string()
}

fn package_skill_body(name: &str, body: Option<String>) -> String {
    let body = optional_trimmed(body).unwrap_or_else(|| "## Instructions\n".to_string());
    let body = if body.trim_start().starts_with("# ") {
        body
    } else {
        format!("# {}\n\n{}", name, body.trim_start())
    };
    if body.ends_with('\n') {
        body
    } else {
        format!("{}\n", body)
    }
}

pub fn create_skill_document_sync(
    working_dir: &str,
    request: SkillCreateRequest,
) -> Result<SkillManifest, String> {
    if request.kind == SkillCreateKind::Package {
        return Err("Use kind='md' for project Skill documents".to_string());
    }
    let (dir_name, document_path) = normalize_skill_create_request_path(&request)?;
    let manifest_path =
        knowledge_store::document_path(working_dir, KnowledgeType::Skill, &document_path)?;
    if manifest_path.exists() {
        return Err(format!("Skill already exists: {}", document_path));
    }

    let name = request.name.trim().to_string();
    let title = skill_title_from_name(&name);
    let summary = required_skill_create_text(request.summary, "summary")?;
    let argument_hint = optional_trimmed(request.argument_hint);
    let tools = normalize_tool_name_list(request.tools);
    let command_enabled = request.command_enabled.unwrap_or(true);
    let command_trigger = if command_enabled {
        let trigger = optional_trimmed(request.command_trigger)
            .map(|value| normalize_and_validate_command_trigger(&value, &name))
            .unwrap_or_else(|| normalize_and_validate_command_trigger("", &name))?;
        (!trigger.is_empty()).then_some(trigger)
    } else {
        None
    };

    let document = knowledge_store::KnowledgeDocument {
        id: format!("kd_{}", uuid::Uuid::new_v4()),
        doc_type: KnowledgeType::Skill,
        path: document_path.clone(),
        title,
        inject_mode: knowledge_store::KnowledgeInjectMode::None,
        inherit_inject_mode: true,
        inject_mode_source: Default::default(),
        summary_enabled: true,
        command_enabled,
        read_only: false,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::Project,
        inherit_ai_config: true,
        ai_config_source: Default::default(),
        explicit_maintenance_rules: false,
        external_source: None,
        skill_enabled: Some(true),
        skill_surface: Some(if command_enabled {
            SkillSurface::Command
        } else {
            SkillSurface::Auto
        }),
        command_trigger,
        argument_hint,
        tools,
        summary: Some(summary),
        body: request
            .body
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_skill_scaffold_body(command_enabled).to_string()),
        maintenance_rules: None,
        created_at: 0,
        updated_at: 0,
    };
    let saved = knowledge_store::save_document(&working_dir, document)?;

    Ok(build_skill_manifest(
        &saved,
        &dir_name,
        "project",
        &format!("{}/{}", SKILL_DIR_NAME, document_path),
        get_updated_at(&manifest_path),
        None,
    ))
}

fn create_skill_package_in_parent_sync_with_default_namespace(
    package_parent: &Path,
    request: SkillCreateRequest,
    default_namespace: Option<&str>,
) -> Result<SkillManifest, String> {
    if request.kind != SkillCreateKind::Package {
        return Err("Use kind='package' for app Skill packages".to_string());
    }
    if optional_trimmed(request.path.clone()).is_some() {
        return Err("'path' is only supported for md Skill documents.".to_string());
    }

    let name = required_skill_create_text(Some(request.name), "name")?;
    let package_id = resolve_skill_create_package_id(request.package_id, default_namespace, &name)?;
    let version = required_skill_create_text(request.version, "version")?;
    let summary = required_skill_create_text(request.summary, "summary")?;
    let argument_hint = optional_trimmed(request.argument_hint);
    let command_enabled = request.command_enabled.unwrap_or(true);
    let default_trigger = default_package_command_name(&package_id);
    let command_trigger = if command_enabled {
        let trigger = optional_trimmed(request.command_trigger)
            .map(|value| normalize_and_validate_command_trigger(&value, &default_trigger))
            .unwrap_or_else(|| normalize_and_validate_command_trigger("", &default_trigger))?;
        (!trigger.is_empty()).then_some(trigger)
    } else {
        None
    };
    let model_invocation_enabled = request.model_invocation_enabled.unwrap_or(true);

    let package_root = package_parent.join(&package_id);
    if package_root.exists()
        || find_skill_package_in_parent(package_parent, &package_id).is_ok()
        || find_skill_package(&package_id).is_ok()
    {
        return Err(format!("Skill package already exists: {}", package_id));
    }
    std::fs::create_dir_all(&package_root)
        .map_err(|e| format!("Failed to create Skill package directory: {}", e))?;

    let write_result = (|| {
        let manifest = SkillPackageManifestFile {
            schema: "locus.skill.v1".to_string(),
            id: package_id.clone(),
            version,
            name: name.clone(),
            description: summary,
            argument_hint: argument_hint.clone(),
            disable_model_invocation: Some(!model_invocation_enabled),
            user_invocable: Some(command_enabled),
            source: None,
            command: Some(SkillPackageCommand {
                enabled: Some(command_enabled),
                trigger: command_trigger,
                argument_hint,
            }),
            capabilities: SkillPackageCapabilities::default(),
            tools: Vec::new(),
        };
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Failed to render Skill package manifest: {}", e))?;
        let manifest_path = package_manifest_path(&package_root);
        std::fs::write(&manifest_path, format!("{}\n", manifest_json))
            .map_err(|e| format!("Failed to write {}: {}", manifest_path.display(), e))?;

        let root_doc_path = package_root.join(SKILL_PACKAGE_ROOT_DOC_FILE_NAME);
        std::fs::write(&root_doc_path, package_skill_body(&name, request.body))
            .map_err(|e| format!("Failed to write {}: {}", root_doc_path.display(), e))?;
        let record = load_skill_package_record(&package_root)?;
        Ok(build_package_skill_manifest(&record, "app", None))
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_dir_all(&package_root);
    }
    write_result
}

fn create_skill_package_in_parent_sync(
    package_parent: &Path,
    request: SkillCreateRequest,
) -> Result<SkillManifest, String> {
    create_skill_package_in_parent_sync_with_default_namespace(package_parent, request, None)
}

pub fn create_skill_package_sync_with_default_namespace(
    request: SkillCreateRequest,
    default_namespace: Option<&str>,
) -> Result<SkillManifest, String> {
    let package_parent = writable_app_skill_package_dir()?;
    create_skill_package_in_parent_sync_with_default_namespace(
        &package_parent,
        request,
        default_namespace,
    )
}

pub fn create_skill_package_sync(request: SkillCreateRequest) -> Result<SkillManifest, String> {
    create_skill_package_sync_with_default_namespace(request, None)
}

pub fn create_skill_sync_with_default_package_namespace(
    working_dir: &str,
    request: SkillCreateRequest,
    default_namespace: Option<&str>,
) -> Result<SkillManifest, String> {
    match request.kind {
        SkillCreateKind::Md => create_skill_document_sync(working_dir, request),
        SkillCreateKind::Package => {
            create_skill_package_sync_with_default_namespace(request, default_namespace)
        }
    }
}

pub fn create_skill_sync(
    working_dir: &str,
    request: SkillCreateRequest,
) -> Result<SkillManifest, String> {
    create_skill_sync_with_default_package_namespace(working_dir, request, None)
}

fn delete_skill_package_from_parent_sync(
    working_dir: &str,
    package_parent: &Path,
    package_id: &str,
) -> Result<String, String> {
    let record = find_skill_package_in_parent(package_parent, package_id)?;
    let canonical_parent = dunce::canonicalize(package_parent).map_err(|e| {
        format!(
            "Failed to resolve Skill package directory '{}': {}",
            package_parent.display(),
            e
        )
    })?;
    let canonical_root = dunce::canonicalize(&record.root).map_err(|e| {
        format!(
            "Failed to resolve Skill package root '{}': {}",
            record.root.display(),
            e
        )
    })?;
    if canonical_root == canonical_parent || !canonical_root.starts_with(&canonical_parent) {
        return Err(
            "Skill package root resolves outside of the writable package directory".to_string(),
        );
    }

    std::fs::remove_dir_all(&canonical_root).map_err(|e| {
        format!(
            "Failed to delete Skill package '{}': {}",
            canonical_root.display(),
            e
        )
    })?;

    let mut configs = load_skill_config(working_dir);
    if configs
        .remove(&config_key("app", &record.manifest.id))
        .is_some()
    {
        save_skill_config(working_dir, &configs)?;
    }

    Ok(record.manifest.id)
}

pub fn delete_skill_package_sync(working_dir: &str, package_id: &str) -> Result<String, String> {
    let package_parent = writable_app_skill_package_dir()?;
    delete_skill_package_from_parent_sync(working_dir, &package_parent, package_id)
}

fn archive_output_path(file_path: &str) -> Result<PathBuf, String> {
    let trimmed = file_path.trim();
    if trimmed.is_empty() {
        return Err("Skill package export path is empty".to_string());
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

fn package_archive_entries(
    record: &SkillPackageRecord,
    target_path: &Path,
) -> Result<Vec<(PathBuf, String, u64)>, String> {
    let target_canonical = target_path
        .is_file()
        .then(|| dunce::canonicalize(target_path).ok())
        .flatten();
    let mut entries = Vec::new();

    for entry in WalkDir::new(&record.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !entry.file_type().is_dir() || !is_ignored_package_walk_dir(entry.path())
        })
    {
        let entry = entry.map_err(|e| format!("Failed to read Skill package files: {}", e))?;
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(format!(
                "Skill package export does not support symlinks: {}",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() || is_ignored_package_walk_file(entry.path()) {
            continue;
        }
        if target_canonical
            .as_ref()
            .is_some_and(|target| dunce::canonicalize(entry.path()).ok().as_ref() == Some(target))
        {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&record.root)
            .map_err(|e| format!("Failed to resolve package file path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        let rel_path = normalize_package_rel_path(&rel_path)?;
        let archive_path = format!("{}/{}", record.manifest.id, rel_path);
        let size = entry
            .metadata()
            .map_err(|e| format!("Failed to read {} metadata: {}", entry.path().display(), e))?
            .len();
        entries.push((entry.path().to_path_buf(), archive_path, size));
    }

    entries.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(entries)
}

fn export_skill_package_record_to_path(
    record: &SkillPackageRecord,
    file_path: &str,
) -> Result<SkillPackageArchiveResult, String> {
    let target_path = archive_output_path(file_path)?;
    let entries = package_archive_entries(record, &target_path)?;
    if entries.is_empty() {
        return Err(format!(
            "Skill package '{}' has no exportable files",
            record.manifest.id
        ));
    }

    if let Some(parent) = target_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create export directory: {}", e))?;
    }

    let zip_file = std::fs::File::create(&target_path)
        .map_err(|e| format!("Failed to create {}: {}", target_path.display(), e))?;
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut buffer = Vec::new();

    for (source_path, archive_path, _) in &entries {
        zip_writer
            .start_file(archive_path, options)
            .map_err(|e| format!("Failed to write archive entry '{}': {}", archive_path, e))?;
        let mut source = std::fs::File::open(source_path)
            .map_err(|e| format!("Failed to read {}: {}", source_path.display(), e))?;
        buffer.clear();
        source
            .read_to_end(&mut buffer)
            .map_err(|e| format!("Failed to read {}: {}", source_path.display(), e))?;
        zip_writer
            .write_all(&buffer)
            .map_err(|e| format!("Failed to write archive entry '{}': {}", archive_path, e))?;
    }

    zip_writer
        .finish()
        .map_err(|e| format!("Failed to finish Skill package archive: {}", e))?;
    let byte_size = std::fs::metadata(&target_path)
        .map(|meta| meta.len())
        .unwrap_or(0);

    Ok(SkillPackageArchiveResult {
        package_id: record.manifest.id.clone(),
        path: target_path.to_string_lossy().to_string(),
        file_count: entries.len(),
        byte_size,
    })
}

#[cfg(test)]
fn export_skill_package_from_parent_sync(
    package_parent: &Path,
    package_id: &str,
    file_path: &str,
) -> Result<SkillPackageArchiveResult, String> {
    let record = find_skill_package_in_parent(package_parent, package_id)?;
    export_skill_package_record_to_path(&record, file_path)
}

pub fn export_skill_package_sync(
    package_id: &str,
    file_path: &str,
) -> Result<SkillPackageArchiveResult, String> {
    let record = find_skill_package(package_id)?;
    export_skill_package_record_to_path(&record, file_path)
}

fn normalize_import_entry_path(name: &str) -> Result<Option<String>, String> {
    let normalized = name.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Ok(None);
    }
    if normalized.contains('\0')
        || normalized.contains(':')
        || normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("Invalid archive entry path: {}", name));
    }
    Ok(Some(normalized))
}

fn extract_skill_package_archive(source_path: &Path, staging_root: &Path) -> Result<(), String> {
    let zip_file = std::fs::File::open(source_path)
        .map_err(|e| format!("Failed to open Skill package archive: {}", e))?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| format!("Invalid Skill package archive: {}", e))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let Some(rel_path) = normalize_import_entry_path(entry.name())? else {
            continue;
        };
        let output_path = staging_root.join(&rel_path);
        if entry.is_dir() || rel_path.ends_with('/') {
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
        std::io::copy(&mut entry, &mut output)
            .map_err(|e| format!("Failed to extract {}: {}", output_path.display(), e))?;
    }
    Ok(())
}

fn locate_imported_skill_package_root(root: &Path) -> Result<PathBuf, String> {
    if is_skill_package_root(root) {
        return Ok(root.to_path_buf());
    }

    let mut package_roots = Vec::new();
    let entries = std::fs::read_dir(root)
        .map_err(|e| format!("Failed to read imported Skill package: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && is_skill_package_root(&path) {
            package_roots.push(path);
        }
    }
    if package_roots.len() == 1 {
        return Ok(package_roots.remove(0));
    }

    Err("Imported Skill package must contain skill.json and SKILL.md".to_string())
}

fn copy_skill_package_dir(source_root: &Path, target_root: &Path) -> Result<usize, String> {
    let mut copied_files = 0usize;
    for entry in WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !entry.file_type().is_dir() || !is_ignored_package_walk_dir(entry.path())
        })
    {
        let entry = entry.map_err(|e| format!("Failed to read Skill package files: {}", e))?;
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(format!(
                "Skill package import does not support symlinks: {}",
                entry.path().display()
            ));
        }
        let rel_path = entry
            .path()
            .strip_prefix(source_root)
            .map_err(|e| format!("Failed to resolve package file path: {}", e))?;
        let target_path = target_root.join(rel_path);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target_path)
                .map_err(|e| format!("Failed to create {}: {}", target_path.display(), e))?;
            continue;
        }
        if !entry.file_type().is_file() || is_ignored_package_walk_file(entry.path()) {
            continue;
        }
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        std::fs::copy(entry.path(), &target_path).map_err(|e| {
            format!(
                "Failed to copy {} to {}: {}",
                entry.path().display(),
                target_path.display(),
                e
            )
        })?;
        copied_files += 1;
    }
    Ok(copied_files)
}

fn import_skill_package_to_parent_sync(
    package_parent: &Path,
    source_path: &Path,
) -> Result<SkillManifest, String> {
    if !source_path.exists() {
        return Err(format!(
            "Skill package import source not found: {}",
            source_path.display()
        ));
    }

    let staging_root =
        std::env::temp_dir().join(format!("locus-skill-import-{}", uuid::Uuid::new_v4()));
    let source_package_root = if source_path.is_dir() {
        locate_imported_skill_package_root(source_path)?
    } else {
        std::fs::create_dir_all(&staging_root)
            .map_err(|e| format!("Failed to create import staging directory: {}", e))?;
        let extract_result = extract_skill_package_archive(source_path, &staging_root);
        if let Err(error) = extract_result {
            let _ = std::fs::remove_dir_all(&staging_root);
            return Err(error);
        }
        match locate_imported_skill_package_root(&staging_root) {
            Ok(root) => root,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging_root);
                return Err(error);
            }
        }
    };

    let source_record = load_skill_package_record(&source_package_root)?;
    let package_id = source_record.manifest.id.clone();
    std::fs::create_dir_all(package_parent)
        .map_err(|e| format!("Failed to create app Skill package directory: {}", e))?;
    let target_root = package_parent.join(&package_id);
    if target_root.exists() {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(format!("Skill package already exists: {}", package_id));
    }

    let install_result = (|| {
        std::fs::create_dir_all(&target_root).map_err(|e| {
            format!(
                "Failed to create Skill package directory '{}': {}",
                target_root.display(),
                e
            )
        })?;
        copy_skill_package_dir(&source_package_root, &target_root)?;
        let record = load_skill_package_record(&target_root)?;
        Ok(build_package_skill_manifest(&record, "app", None))
    })();

    let _ = std::fs::remove_dir_all(&staging_root);
    if install_result.is_err() {
        let _ = std::fs::remove_dir_all(&target_root);
    }
    install_result
}

pub fn import_skill_package_sync(source_path: &str) -> Result<SkillManifest, String> {
    let source_path = source_path.trim();
    if source_path.is_empty() {
        return Err("Skill package import source path is empty".to_string());
    }
    let package_parent = writable_app_skill_package_dir()?;
    import_skill_package_to_parent_sync(&package_parent, &PathBuf::from(source_path))
}

fn normalize_skill_source(source: Option<&str>) -> Result<&str, String> {
    match source.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok("project"),
        Some("project") => Ok("project"),
        Some("app") => Ok("app"),
        Some(other) => Err(format!("Invalid skill source: {}", other)),
    }
}

pub fn reload_skill_manifest_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    request: SkillReloadRequest,
) -> Result<SkillManifest, String> {
    let source = normalize_skill_source(request.source.as_deref())?;
    if source == "app" {
        if let Ok(record) = find_skill_package(&request.name) {
            let configs = load_skill_config(working_dir);
            let fallback = default_package_command_name(&record.manifest.id);
            let cfg =
                validated_skill_config_override(&configs, "app", &record.manifest.id, &fallback)?;
            return Ok(build_package_skill_manifest(&record, "app", cfg.as_ref()));
        }
    }

    let normalized_dir_name = normalize_skill_manifest_name(&request.name)?;
    let knowledge_dir = if source == "app" {
        app_knowledge_dir
            .cloned()
            .ok_or_else(|| "App knowledge directory not found".to_string())?
    } else {
        std::path::Path::new(working_dir)
            .join("Locus")
            .join("knowledge")
    };
    let skill_dir = knowledge_dir.join(SKILL_DIR_NAME);

    let mut document_path = format!("{}.md", normalized_dir_name);
    let mut manifest_path = skill_dir.join(&document_path);
    if source == "app" && !manifest_path.is_file() && !normalized_dir_name.contains('/') {
        document_path = format!("builtin/{}.md", normalized_dir_name);
        manifest_path = skill_dir.join(&document_path);
    }
    if !manifest_path.is_file() {
        return Err(format!("Skill not found: {}", normalized_dir_name));
    }

    let document = knowledge_store::load_document_by_root(
        &knowledge_dir,
        KnowledgeType::Skill,
        &document_path,
    )?;
    if document.path != document_path {
        return Err(format!(
            "Skill frontmatter path '{}' does not match '{}'",
            document.path, document_path
        ));
    }
    validate_skill_document_config(&document, document_path.trim_end_matches(".md"))?;

    let configs = load_skill_config(working_dir);
    let cfg = if source == "app" {
        validated_skill_config_override(
            &configs,
            source,
            &normalized_dir_name,
            document_path.trim_end_matches(".md"),
        )?
    } else {
        None
    };
    Ok(build_skill_manifest(
        &document,
        document_path.trim_end_matches(".md"),
        source,
        &format!("{}/{}", SKILL_DIR_NAME, document_path),
        get_updated_at(&manifest_path),
        cfg.as_ref(),
    ))
}

pub fn list_skills_filtered_sync(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    source: Option<&str>,
) -> Result<Vec<SkillManifest>, String> {
    let source = source.map(str::trim).filter(|value| !value.is_empty());
    if let Some(source) = source {
        normalize_skill_source(Some(source))?;
    }
    let mut skills = list_skills_sync(working_dir, app_knowledge_dir);
    if let Some(source) = source {
        skills.retain(|skill| skill.source == source);
    }
    Ok(skills)
}

#[tauri::command]
pub fn get_default_skill_package_namespace(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<String, AppError> {
    Ok(config.default_skill_package_namespace())
}

#[tauri::command]
pub fn set_default_skill_package_namespace(
    value: String,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<String, AppError> {
    let normalized = normalize_default_skill_package_namespace(&value).map_err(AppError::from)?;
    config
        .set_default_skill_package_namespace(normalized.clone())
        .map_err(AppError::from)?;
    Ok(normalized)
}

#[tauri::command]
pub async fn create_skill_scaffold(
    kind: Option<SkillCreateKind>,
    name: String,
    path: Option<String>,
    package_id: Option<String>,
    version: Option<String>,
    summary: Option<String>,
    body: Option<String>,
    argument_hint: Option<String>,
    command_trigger: Option<String>,
    command_enabled: Option<bool>,
    model_invocation_enabled: Option<bool>,
    tools: Option<Vec<String>>,
    config: State<'_, Arc<crate::config::AppConfig>>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<SkillManifest, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let kind = kind.unwrap_or_default();
    let fallback_summary = skill_title_from_name(&name);
    let summary = if kind == SkillCreateKind::Md {
        summary.or(Some(fallback_summary))
    } else {
        summary
    };
    let default_namespace = config.default_skill_package_namespace();
    let manifest = create_skill_sync_with_default_package_namespace(
        &working_dir,
        SkillCreateRequest {
            kind,
            name,
            path,
            package_id,
            version,
            summary,
            body,
            argument_hint,
            command_trigger,
            command_enabled,
            model_invocation_enabled,
            tools: tools.unwrap_or_default(),
        },
        Some(&default_namespace),
    )?;
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "create_skill_scaffold",
    )
    .await?;
    Ok(manifest)
}

#[tauri::command]
pub async fn delete_skill_package(
    package_id: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    delete_skill_package_sync(&working_dir, &package_id).map_err(AppError::from)?;
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "delete_skill_package",
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn import_skill_package(
    source_path: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<SkillManifest, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let manifest = import_skill_package_sync(&source_path).map_err(AppError::from)?;
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "import_skill_package",
    )
    .await?;
    Ok(manifest)
}

#[tauri::command]
pub async fn export_skill_package(
    package_id: String,
    file_path: String,
) -> Result<SkillPackageArchiveResult, AppError> {
    export_skill_package_sync(&package_id, &file_path).map_err(AppError::from)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let content =
        std::fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    Ok(blake3::hash(&content).to_hex().to_string())
}

fn unity_target_relative_path(source_path: &str) -> Result<String, String> {
    let normalized = normalize_package_rel_path(source_path)?;
    let stripped = normalized
        .strip_prefix("unity/Editor/")
        .or_else(|| normalized.strip_prefix("unity/"))
        .unwrap_or(&normalized);
    normalize_package_rel_path(stripped)
}

fn package_unity_install_root(project_path: &Path, package_id: &str) -> PathBuf {
    crate::unity_bridge::plugin_skills_root(project_path).join(package_id)
}

fn package_unity_file_status(
    project_path: &Path,
    record: &SkillPackageRecord,
    capability: &SkillPackageUnityCapability,
) -> Result<SkillUnityFileStatus, String> {
    let source_path = package_file_path(&record.root, &capability.path)?;
    let target_rel = unity_target_relative_path(&capability.path)?;
    let target_path =
        package_unity_install_root(project_path, &record.manifest.id).join(&target_rel);
    let source_hash = source_path
        .is_file()
        .then(|| hash_file(&source_path))
        .transpose()?;
    let installed_hash = target_path
        .is_file()
        .then(|| hash_file(&target_path))
        .transpose()?;
    let state = match (source_hash.as_deref(), installed_hash.as_deref()) {
        (Some(source), Some(installed)) if source == installed => "installed",
        (Some(_), Some(_)) => "modified",
        (Some(_), None) => "missing",
        (None, _) => "sourceMissing",
    };
    Ok(SkillUnityFileStatus {
        source_path: capability.path.clone(),
        target_path: target_path
            .strip_prefix(project_path)
            .unwrap_or(&target_path)
            .to_string_lossy()
            .replace('\\', "/"),
        state: state.to_string(),
        source_hash,
        installed_hash,
    })
}

fn skill_unity_install_status_sync(
    working_dir: &str,
    package_id: &str,
) -> Result<SkillUnityInstallStatus, String> {
    let record = find_skill_package(package_id)?;
    let project_path = Path::new(working_dir);
    let plugin_root = crate::unity_bridge::plugin_install_root(project_path);
    let install_root = package_unity_install_root(project_path, &record.manifest.id);
    let has_unity = !record.manifest.capabilities.unity.is_empty();

    if !has_unity {
        return Ok(SkillUnityInstallStatus {
            package_id: record.manifest.id,
            has_unity,
            state: "notApplicable".to_string(),
            plugin_root: plugin_root.to_string_lossy().replace('\\', "/"),
            install_root: install_root.to_string_lossy().replace('\\', "/"),
            files: Vec::new(),
            message: None,
        });
    }

    if !plugin_root.is_dir() {
        return Ok(SkillUnityInstallStatus {
            package_id: record.manifest.id,
            has_unity,
            state: "pluginMissing".to_string(),
            plugin_root: plugin_root.to_string_lossy().replace('\\', "/"),
            install_root: install_root.to_string_lossy().replace('\\', "/"),
            files: Vec::new(),
            message: Some("Locus Unity plugin is not installed in this project.".to_string()),
        });
    }

    let files = record
        .manifest
        .capabilities
        .unity
        .iter()
        .map(|capability| package_unity_file_status(project_path, &record, capability))
        .collect::<Result<Vec<_>, _>>()?;

    let state = if files.is_empty() {
        "notApplicable"
    } else if files.iter().all(|file| file.state == "installed") {
        "installed"
    } else if files.iter().all(|file| file.state == "missing") && !install_root.is_dir() {
        "notInstalled"
    } else if files.iter().any(|file| file.state == "modified") {
        "modified"
    } else if files.iter().any(|file| file.state == "sourceMissing") {
        "sourceMissing"
    } else {
        "partial"
    };

    Ok(SkillUnityInstallStatus {
        package_id: record.manifest.id,
        has_unity,
        state: state.to_string(),
        plugin_root: plugin_root.to_string_lossy().replace('\\', "/"),
        install_root: install_root.to_string_lossy().replace('\\', "/"),
        files,
        message: None,
    })
}

fn remove_dir_and_meta(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
    }
    let mut meta = path.as_os_str().to_os_string();
    meta.push(".meta");
    let meta = PathBuf::from(meta);
    if meta.exists() {
        std::fs::remove_file(&meta)
            .map_err(|e| format!("Failed to remove {}: {}", meta.display(), e))?;
    }
    Ok(())
}

fn install_skill_unity_files_sync(
    working_dir: &str,
    package_id: &str,
) -> Result<SkillUnityInstallStatus, String> {
    let record = find_skill_package(package_id)?;
    if record.manifest.capabilities.unity.is_empty() {
        return skill_unity_install_status_sync(working_dir, package_id);
    }

    let project_path = Path::new(working_dir);
    let plugin_root = crate::unity_bridge::plugin_install_root(project_path);
    if !plugin_root.is_dir() {
        return Err("Locus Unity plugin is not installed in this project".to_string());
    }

    let install_root = package_unity_install_root(project_path, &record.manifest.id);
    remove_dir_and_meta(&install_root)?;
    std::fs::create_dir_all(&install_root)
        .map_err(|e| format!("Failed to create {}: {}", install_root.display(), e))?;

    for capability in &record.manifest.capabilities.unity {
        let source_path = package_file_path(&record.root, &capability.path)?;
        if !source_path.is_file() {
            return Err(format!(
                "Skill Unity source file not found: {}",
                capability.path
            ));
        }
        let target_rel = unity_target_relative_path(&capability.path)?;
        let target_path = install_root.join(target_rel);
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        std::fs::copy(&source_path, &target_path).map_err(|e| {
            format!(
                "Failed to install {} to {}: {}",
                source_path.display(),
                target_path.display(),
                e
            )
        })?;
    }

    skill_unity_install_status_sync(working_dir, package_id)
}

fn remove_skill_unity_files_sync(
    working_dir: &str,
    package_id: &str,
) -> Result<SkillUnityInstallStatus, String> {
    let record = find_skill_package(package_id)?;
    let project_path = Path::new(working_dir);
    let install_root = package_unity_install_root(project_path, &record.manifest.id);
    remove_dir_and_meta(&install_root)?;
    skill_unity_install_status_sync(working_dir, package_id)
}

#[tauri::command]
pub async fn get_skill_unity_install_status(
    package_id: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<SkillUnityInstallStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    skill_unity_install_status_sync(&working_dir, &package_id).map_err(Into::into)
}

#[tauri::command]
pub async fn install_skill_unity_files(
    package_id: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<SkillUnityInstallStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    install_skill_unity_files_sync(&working_dir, &package_id).map_err(Into::into)
}

#[tauri::command]
pub async fn remove_skill_unity_files(
    package_id: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<SkillUnityInstallStatus, AppError> {
    let working_dir = workspace.path.read().await.clone();
    remove_skill_unity_files_sync(&working_dir, &package_id).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_skill_scaffold_name, list_skills_sync, read_skill_manifest_sync,
        SkillPackageDocLevels, SkillPackageManifestFile, SkillPackageRecord,
        SkillPackageToolManifest,
    };
    use crate::commands::knowledge::SkillConfig;
    use crate::knowledge_store::{KnowledgeInjectMode, SkillSurface};
    use std::io::Write as _;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn skill_scaffold_name_validation_rejects_non_kebab_case_inputs() {
        assert!(is_valid_skill_scaffold_name("asset-audit"));
        assert!(is_valid_skill_scaffold_name("asset-audit-2"));
        assert!(!is_valid_skill_scaffold_name("AssetAudit"));
        assert!(!is_valid_skill_scaffold_name("asset_audit"));
        assert!(!is_valid_skill_scaffold_name("asset--audit"));
        assert!(!is_valid_skill_scaffold_name("-asset-audit"));
        assert!(!is_valid_skill_scaffold_name("asset-audit-"));
    }

    #[test]
    fn list_skills_sync_reads_project_root_skill() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let skill_dir = temp.path().join("Locus").join("knowledge").join("skill");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let raw = r#"---
id: kd_skill_create_skill
type: skill
path: create-skill.md
title: Create Skill
scope: project
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-skill
argumentHint: <skill-name>
createdAt: 1
updatedAt: 1
---

# Create Skill

## Summary
Create a project skill.

## Content
## When to use

- Reuse a workflow.
"#;
        std::fs::write(skill_dir.join("create-skill.md"), raw).unwrap();

        let skills = list_skills_sync(&working_dir, None);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "create-skill");
        assert_eq!(skills[0].source, "project");
        assert_eq!(skills[0].command_trigger, "/create-skill");
    }

    #[test]
    fn package_root_doc_level_detection_treats_levels_as_optional() {
        let body = "## Instructions\nDo the work.\n";

        let levels = super::scan_package_document_levels(body);
        assert!(!levels.has_l0);
        assert!(!levels.has_l1);
        assert!(!levels.has_l2);
    }

    #[test]
    fn load_skill_package_record_reads_skill_json_manifest() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("skill.json"),
            r#"{
  "schema": "locus.skill.v1",
  "id": "com.example.asset-audit",
  "version": "0.1.0",
  "name": "Asset Audit",
  "description": "Audit Unity assets and report cleanup tasks.",
  "argumentHint": "<scope>",
  "disableModelInvocation": true,
  "source": {
    "type": "github",
    "url": "https://github.com/example/locus-skills",
    "reference": "asset-audit"
  },
  "command": {
    "enabled": true,
    "trigger": "/asset-audit"
  },
  "capabilities": {
    "unity": [
      {
        "name": "AssetAuditBridge",
        "path": "unity/Editor/SkillBridge.cs",
        "api": "unity_execute"
      }
    ]
  },
  "tools": [
    {
      "name": "capture-frame",
      "description": "Capture and analyze a RenderDoc frame.",
      "runtime": "unity",
      "path": "unity/Editor/RenderDocBridge.cs",
      "entryType": "Locus.Skills.RenderDocBridge",
      "method": "CaptureFrame",
      "requestEditorStatus": "playing",
      "parameters": {
        "type": "object",
        "properties": {
          "view": { "type": "string" }
        }
      }
    }
  ]
}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("SKILL.md"),
            r#"
# Asset Audit

## Instructions
Do the work.
"#,
        )
        .unwrap();

        let record = super::load_skill_package_record(temp.path()).unwrap();
        assert_eq!(record.manifest.id, "com.example.asset-audit");
        assert_eq!(record.manifest.name, "Asset Audit");
        assert_eq!(record.manifest.version, "0.1.0");
        assert_eq!(
            record.manifest.description,
            "Audit Unity assets and report cleanup tasks."
        );
        assert_eq!(
            record.manifest.command.as_ref().unwrap().trigger.as_deref(),
            Some("/asset-audit")
        );
        assert_eq!(
            record
                .manifest
                .command
                .as_ref()
                .unwrap()
                .argument_hint
                .as_deref(),
            Some("<scope>")
        );
        assert_eq!(
            record.manifest.capabilities.unity[0].path,
            "unity/Editor/SkillBridge.cs"
        );
        assert_eq!(record.manifest.tools.len(), 1);
        assert_eq!(
            super::package_tool_api_name(&record.manifest.id, &record.manifest.tools[0].name),
            "capture_frame"
        );
        assert_eq!(
            super::package_skill_surface(&record.manifest),
            SkillSurface::Command
        );
        assert!(!record.doc_levels.has_l0);
        assert!(!record.doc_levels.has_l2);
    }

    #[test]
    fn package_knowledge_item_applies_workspace_config() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("skill.json"),
            r#"{
  "schema": "locus.skill.v1",
  "id": "com.feishu.cli",
  "version": "0.1.0",
  "name": "Feishu CLI",
  "description": "Use Feishu safely.",
  "command": {
    "enabled": true,
    "trigger": "/feishu"
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("SKILL.md"),
            r#"
# Feishu CLI

## L0
Use Feishu safely.
"#,
        )
        .unwrap();

        let record = super::load_skill_package_record(temp.path()).unwrap();
        let item = super::package_to_list_item(
            &record,
            "SKILL.md",
            Some(&SkillConfig {
                enabled: true,
                surface: SkillSurface::Auto,
                description: "Workspace override.".to_string(),
                command_trigger: "/lark".to_string(),
                inject_mode: KnowledgeInjectMode::Path,
            }),
        );

        assert_eq!(item.inject_mode, KnowledgeInjectMode::Path);
        assert_eq!(item.skill_surface, Some(SkillSurface::Auto));
        assert_eq!(item.command_enabled, false);
        assert_eq!(item.command_trigger.as_deref(), Some("/lark"));
        assert_eq!(item.summary.as_deref(), Some("Workspace override."));
    }

    #[test]
    fn package_doc_rel_path_resolves_root_and_nested_documents() {
        let manifest = SkillPackageManifestFile {
            id: "com.example.asset-audit".to_string(),
            name: "Asset Audit".to_string(),
            ..Default::default()
        };

        assert_eq!(
            super::package_doc_rel_path_for_virtual_path(&manifest, "com.example.asset-audit")
                .unwrap()
                .as_deref(),
            Some("SKILL.md")
        );
        assert_eq!(
            super::package_doc_rel_path_for_virtual_path(
                &manifest,
                "skill/com.example.asset-audit"
            )
            .unwrap()
            .as_deref(),
            Some("SKILL.md")
        );
        assert_eq!(
            super::package_doc_rel_path_for_virtual_path(
                &manifest,
                "com.example.asset-audit/docs/usage.md"
            )
            .unwrap()
            .as_deref(),
            Some("docs/usage.md")
        );
    }

    #[test]
    fn package_knowledge_items_include_utf8_subfiles() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("references")).unwrap();
        std::fs::create_dir_all(temp.path().join("scripts").join("__pycache__")).unwrap();
        std::fs::create_dir_all(temp.path().join("unity").join("Editor")).unwrap();
        std::fs::write(
            temp.path().join("skill.json"),
            r#"{
  "schema": "locus.skill.v1",
  "id": "com.locus.psd-to-ugui",
  "version": "0.1.0",
  "name": "PSD to uGUI",
  "description": "Parse PSD files.",
  "command": {
    "enabled": true,
    "trigger": "/psd-to-ugui"
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("SKILL.md"),
            "# PSD to uGUI\n\n## Instructions\nConvert PSD files.",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("references").join("details.md"),
            "# Details\n\nCoordinate mapping notes.",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("scripts").join("parse_psd.py"),
            "print('parse')\n",
        )
        .unwrap();
        std::fs::write(
            temp.path()
                .join("scripts")
                .join("__pycache__")
                .join("parse.pyc"),
            [0, 159, 146, 150],
        )
        .unwrap();
        std::fs::write(
            temp.path()
                .join("unity")
                .join("Editor")
                .join("PsdToUguiBridge.cs"),
            "public static class PsdToUguiBridge {}\n",
        )
        .unwrap();

        let record = super::load_skill_package_record(temp.path()).unwrap();
        let items = super::package_to_list_items(&record, None);
        let paths = items
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                "com.locus.psd-to-ugui/SKILL.md",
                "com.locus.psd-to-ugui/references/details.md",
                "com.locus.psd-to-ugui/scripts/parse_psd.py",
                "com.locus.psd-to-ugui/skill.json",
                "com.locus.psd-to-ugui/unity/Editor/PsdToUguiBridge.cs",
            ]
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            items.len()
        );
        let script = items
            .iter()
            .find(|item| item.path.ends_with("scripts/parse_psd.py"))
            .expect("script item");
        assert_eq!(script.title, "parse_psd.py");
        assert_eq!(script.skill_enabled, Some(false));
        assert_eq!(script.command_trigger, None);
        assert!(!script.summary_enabled);

        let document = super::package_to_document(
            &record,
            "scripts/parse_psd.py",
            "print('parse')\n".to_string(),
            None,
        )
        .expect("package document");
        assert_eq!(document.title, "parse_psd.py");
        assert_eq!(document.body, "print('parse')\n");
        assert_eq!(document.path, "com.locus.psd-to-ugui/scripts/parse_psd.py");
    }

    #[test]
    fn package_document_frontmatter_declares_document_scoped_tools() {
        let record = SkillPackageRecord {
            root: PathBuf::new(),
            updated_at: 0,
            doc_levels: SkillPackageDocLevels::default(),
            manifest: SkillPackageManifestFile {
                id: "view".to_string(),
                name: "View".to_string(),
                ..Default::default()
            },
        };

        let document = super::package_to_document(
            &record,
            "debug.md",
            "---\ntitle: View Debug\ntools:\n  - view_capture\n  - view_snapshot\n---\n\n# Debug\n"
                .to_string(),
            None,
        )
        .expect("package document");

        assert_eq!(document.title, "View Debug");
        assert_eq!(document.body.trim(), "# Debug");
        assert_eq!(
            document.tools,
            vec!["view_capture".to_string(), "view_snapshot".to_string()]
        );
    }

    #[test]
    fn package_unity_bundle_includes_unity_tool_script_paths() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("tools")).unwrap();
        std::fs::write(
            temp.path().join("skill.json"),
            r#"{
  "schema": "locus.skill.v1",
  "id": "com.example.tool",
  "version": "0.1.0",
  "name": "Example Tool",
  "description": "Run a Unity helper.",
  "tools": [
    {
      "name": "read",
      "description": "Read state.",
      "runtime": "unity",
      "path": "tools/Bridge.cs",
      "entryType": "ExampleBridge",
      "method": "Read",
      "parameters": { "type": "object", "properties": {} }
    }
  ]
}"#,
        )
        .unwrap();
        std::fs::write(temp.path().join("SKILL.md"), "# Example Tool\n").unwrap();
        std::fs::write(
            temp.path().join("tools").join("Bridge.cs"),
            "public static class ExampleBridge { public static string Read() => \"ok\"; }\n",
        )
        .unwrap();

        let record = super::load_skill_package_record(temp.path()).expect("load package");
        let bundle = super::skill_package_unity_script_bundle_for_record(&record)
            .expect("bundle")
            .expect("has scripts");
        let scripts = bundle
            .request
            .get("scripts")
            .and_then(serde_json::Value::as_array)
            .expect("scripts");

        assert_eq!(bundle.package_id, "com.example.tool");
        assert_eq!(bundle.script_count, 1);
        assert_eq!(
            scripts[0].get("path").and_then(serde_json::Value::as_str),
            Some("tools/Bridge.cs")
        );
    }

    #[test]
    fn skill_package_invoke_payload_targets_compiled_package_assembly() {
        let payload = super::skill_package_invoke_payload(
            "com.example.tool",
            Some("__LocusSkillPackage_com_example_tool_hash"),
            "ExampleBridge",
            "Read",
            &serde_json::json!({ "id": 1 }),
        )
        .expect("payload");

        assert_eq!(payload["packageId"], "com.example.tool");
        assert_eq!(
            payload["assemblyId"],
            "__LocusSkillPackage_com_example_tool_hash"
        );
        assert_eq!(payload["typeName"], "ExampleBridge");
        assert_eq!(payload["method"], "Read");
        assert_eq!(payload["argsJson"], "{\"id\":1}");
        assert!(payload.get("source").is_none());
    }

    #[test]
    fn create_skill_document_sync_requires_summary_metadata() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let err = super::create_skill_document_sync(
            &working_dir,
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Md,
                name: "asset-audit".to_string(),
                ..Default::default()
            },
        )
        .expect_err("missing summary should be rejected");
        assert!(err.contains("'summary' parameter is required"));

        let manifest = super::create_skill_document_sync(
            &working_dir,
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Md,
                name: "asset-audit".to_string(),
                summary: Some("Audit Unity assets.".to_string()),
                tools: vec!["skill_create".to_string(), "skill_reload".to_string()],
                ..Default::default()
            },
        )
        .expect("create skill document");
        assert_eq!(manifest.dir_name, "asset-audit");
        assert_eq!(manifest.command_trigger, "/asset-audit");
        assert_eq!(manifest.description, "Audit Unity assets.");
        assert_eq!(manifest.tools, vec!["skill_create", "skill_reload"]);

        let saved = crate::knowledge_store::read_document(
            &working_dir,
            crate::knowledge_store::KnowledgeType::Skill,
            "asset-audit.md",
            "full",
        )
        .expect("read created skill document");
        assert_eq!(saved.document.body, "## Instructions");
        assert_eq!(saved.document.tools, vec!["skill_create", "skill_reload"]);
    }

    #[test]
    fn package_tool_api_names_use_package_prefix_only_for_conflicts() {
        fn test_python_tool(name: &str, path: &str) -> SkillPackageToolManifest {
            SkillPackageToolManifest {
                name: name.to_string(),
                description: String::new(),
                runtime: "python".to_string(),
                path: Some(path.to_string()),
                command: None,
                args: Vec::new(),
                input: None,
                output: None,
                timeout_ms: None,
                type_name: None,
                method: None,
                entry_type: None,
                request_editor_status: None,
                parameters: super::default_tool_parameters(),
            }
        }

        let records = vec![
            SkillPackageRecord {
                root: PathBuf::new(),
                updated_at: 0,
                doc_levels: SkillPackageDocLevels::default(),
                manifest: SkillPackageManifestFile {
                    id: "psd-to-ugui".to_string(),
                    name: "PSD to uGUI".to_string(),
                    tools: vec![test_python_tool(
                        "extract-psd-layer-tree",
                        "scripts/extract.py",
                    )],
                    ..Default::default()
                },
            },
            SkillPackageRecord {
                root: PathBuf::new(),
                updated_at: 0,
                doc_levels: SkillPackageDocLevels::default(),
                manifest: SkillPackageManifestFile {
                    id: "ui-audit".to_string(),
                    name: "UI Audit".to_string(),
                    tools: vec![test_python_tool(
                        "extract-psd-layer-tree",
                        "scripts/extract.py",
                    )],
                    ..Default::default()
                },
            },
            SkillPackageRecord {
                root: PathBuf::new(),
                updated_at: 0,
                doc_levels: SkillPackageDocLevels::default(),
                manifest: SkillPackageManifestFile {
                    id: "capture-tools".to_string(),
                    name: "Capture Tools".to_string(),
                    tools: vec![
                        test_python_tool("capture-frame", "scripts/capture.py"),
                        test_python_tool("read", "scripts/read.py"),
                    ],
                    ..Default::default()
                },
            },
        ];
        let names = super::package_tool_api_names_for_records(
            &records,
            &super::default_package_tool_reserved_names(),
        );

        assert_eq!(
            names.get(&super::package_tool_record_key(
                "capture-tools",
                "capture-frame"
            )),
            Some(&"capture_frame".to_string())
        );
        assert_eq!(
            names.get(&super::package_tool_record_key("capture-tools", "read")),
            Some(&"capture_tools_read".to_string())
        );
        assert_eq!(
            names.get(&super::package_tool_record_key(
                "psd-to-ugui",
                "extract-psd-layer-tree"
            )),
            Some(&"psd_to_ugui_extract_psd_layer_tree".to_string())
        );
        assert_eq!(
            names.get(&super::package_tool_record_key(
                "ui-audit",
                "extract-psd-layer-tree"
            )),
            Some(&"ui_audit_extract_psd_layer_tree".to_string())
        );
    }

    #[test]
    fn create_skill_rejects_invalid_command_trigger() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();

        let err = super::create_skill_document_sync(
            &working_dir,
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Md,
                name: "asset-audit".to_string(),
                summary: Some("Audit Unity assets.".to_string()),
                command_trigger: Some("/asset audit".to_string()),
                ..Default::default()
            },
        )
        .expect_err("invalid command trigger should be rejected");

        assert!(err.contains("Command trigger must be a single / command token"));
    }

    #[test]
    fn create_skill_package_writes_loadable_metadata() {
        let temp = TempDir::new().unwrap();
        let manifest = super::create_skill_package_in_parent_sync(
            temp.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Asset Audit".to_string(),
                package_id: Some("com.example.asset-audit".to_string()),
                version: Some("0.1.0".to_string()),
                summary: Some("Audit Unity assets and cleanup risks.".to_string()),
                argument_hint: Some("<scope>".to_string()),
                command_trigger: Some("/asset-audit".to_string()),
                command_enabled: Some(true),
                model_invocation_enabled: Some(false),
                body: Some("## Instructions\nRun the audit.".to_string()),
                ..Default::default()
            },
        )
        .expect("create skill package");

        assert_eq!(manifest.kind, super::SkillManifestKind::Package);
        assert_eq!(
            manifest.package_id.as_deref(),
            Some("com.example.asset-audit")
        );
        assert_eq!(manifest.package_version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.command_trigger, "/asset-audit");
        assert_eq!(manifest.argument_hint, "<scope>");

        let package_root = temp.path().join("com.example.asset-audit");
        assert!(package_root.join("skill.json").is_file());
        let root_skill = std::fs::read_to_string(package_root.join("SKILL.md")).unwrap();
        assert!(!root_skill.trim_start().starts_with("---"));
        let record = super::load_skill_package_record(&package_root).expect("load package");
        assert_eq!(record.manifest.name, "Asset Audit");
        assert_eq!(record.manifest.version, "0.1.0");
        assert_eq!(record.manifest.disable_model_invocation, Some(true));
        assert_eq!(
            record
                .manifest
                .command
                .as_ref()
                .and_then(|command| command.enabled),
            Some(true)
        );
    }

    #[test]
    fn create_skill_package_derives_short_id_from_name() {
        let temp = TempDir::new().unwrap();
        let manifest = super::create_skill_package_in_parent_sync_with_default_namespace(
            temp.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Asset Audit".to_string(),
                version: Some("0.1.0".to_string()),
                summary: Some("Audit Unity assets.".to_string()),
                ..Default::default()
            },
            Some("studio.tools"),
        )
        .expect("create package from name");

        assert_eq!(manifest.package_id.as_deref(), Some("asset-audit"));
        assert!(temp.path().join("asset-audit").is_dir());
    }

    #[test]
    fn create_skill_package_uses_short_id_when_default_namespace_is_empty() {
        let temp = TempDir::new().unwrap();
        let manifest = super::create_skill_package_in_parent_sync_with_default_namespace(
            temp.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Asset Audit".to_string(),
                version: Some("0.1.0".to_string()),
                summary: Some("Audit Unity assets.".to_string()),
                ..Default::default()
            },
            Some(""),
        )
        .expect("create package from name");

        assert_eq!(manifest.package_id.as_deref(), Some("asset-audit"));
        assert!(temp.path().join("asset-audit").is_dir());
    }

    #[test]
    fn skill_package_ids_allow_single_segment_namespaces() {
        assert_eq!(
            super::normalize_package_id("studio.asset-audit").unwrap(),
            "studio.asset-audit"
        );
        assert_eq!(super::normalize_package_id("studio").unwrap(), "studio");
    }

    #[test]
    fn delete_skill_package_removes_package_root_and_config() {
        let workspace = TempDir::new().unwrap();
        let package_parent = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        super::create_skill_package_in_parent_sync(
            package_parent.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Asset Audit".to_string(),
                package_id: Some("com.example.asset-audit".to_string()),
                version: Some("0.1.0".to_string()),
                summary: Some("Audit Unity assets.".to_string()),
                ..Default::default()
            },
        )
        .expect("create package");

        let mut configs = std::collections::HashMap::new();
        configs.insert(
            "app:skill/com.example.asset-audit".to_string(),
            SkillConfig {
                enabled: false,
                surface: SkillSurface::Auto,
                description: "override".to_string(),
                command_trigger: "/audit".to_string(),
                inject_mode: KnowledgeInjectMode::Path,
            },
        );
        crate::commands::knowledge::save_skill_config(&working_dir, &configs).expect("save config");

        let deleted = super::delete_skill_package_from_parent_sync(
            &working_dir,
            package_parent.path(),
            "com.example.asset-audit",
        )
        .expect("delete package");

        assert_eq!(deleted, "com.example.asset-audit");
        assert!(!package_parent
            .path()
            .join("com.example.asset-audit")
            .exists());
        assert!(!crate::commands::knowledge::load_skill_config(&working_dir)
            .contains_key("app:skill/com.example.asset-audit"));
    }

    #[test]
    fn create_skill_package_rejects_invalid_command_trigger() {
        let temp = TempDir::new().unwrap();

        let err = super::create_skill_package_in_parent_sync(
            temp.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Feishu CLI".to_string(),
                package_id: Some("com.feishu.cli".to_string()),
                version: Some("0.1.0".to_string()),
                summary: Some("Use Feishu safely.".to_string()),
                command_trigger: Some("/Feishu CLI".to_string()),
                ..Default::default()
            },
        )
        .expect_err("invalid package command trigger should be rejected");

        assert!(err.contains("Command trigger must be a single / command token"));
    }

    #[test]
    fn package_default_command_uses_package_tail_not_display_name() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("skill.json"),
            r#"{
  "schema": "locus.skill.v1",
  "id": "com.feishu.cli",
  "version": "0.1.0",
  "name": "Feishu CLI",
  "description": "Use Feishu safely.",
  "command": {
    "enabled": true
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("SKILL.md"),
            r#"
# Feishu CLI

## Instructions
Use Feishu safely.
"#,
        )
        .unwrap();

        let record = super::load_skill_package_record(temp.path()).unwrap();
        let item = super::package_to_list_item(&record, "SKILL.md", None);

        assert_eq!(item.command_trigger.as_deref(), Some("/cli"));
    }

    #[test]
    fn reload_skill_manifest_rejects_invalid_command_trigger() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().to_string_lossy().to_string();
        let skill_dir = temp.path().join("Locus").join("knowledge").join("skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("bad-skill.md"),
            r#"---
id: kd_skill_bad_skill
type: skill
path: bad-skill.md
title: Bad Skill
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /bad skill
createdAt: 1
updatedAt: 1
---

# Bad Skill

## Instructions
Do the work.
"#,
        )
        .unwrap();

        let err = super::reload_skill_manifest_sync(
            &working_dir,
            None,
            super::SkillReloadRequest {
                name: "bad-skill".to_string(),
                source: None,
            },
        )
        .expect_err("invalid command trigger should fail reload");

        assert!(err.contains("Command trigger must be a single / command token"));
    }

    #[test]
    fn list_skills_sync_reads_nested_app_builtin_skill() {
        let temp = TempDir::new().unwrap();
        let working_dir = temp.path().join("workspace");
        let app_knowledge_dir = temp.path().join("app-knowledge");
        let skill_dir = app_knowledge_dir.join("skill").join("builtin");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let raw = r#"---
id: kd_skill_create_skill
type: skill
path: builtin/create-skill.md
title: Create Skill
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: true
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-skill
argumentHint: <skill-name>
createdAt: 1
updatedAt: 1
---

# Create Skill

## Summary
Create a project skill.

## Content
## When to use

- Reuse a workflow.
        "#;
        std::fs::write(skill_dir.join("create-skill.md"), raw).unwrap();

        let working_dir = working_dir.to_string_lossy().to_string();
        let skills = list_skills_sync(&working_dir, Some(&app_knowledge_dir));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "builtin/create-skill");
        assert_eq!(skills[0].source, "app");
        assert_eq!(skills[0].rel_path, "skill/builtin/create-skill.md");
        assert_eq!(skills[0].command_trigger, "/create-skill");

        let content = read_skill_manifest_sync(
            &working_dir,
            Some(&app_knowledge_dir),
            "create-skill",
            Some("app"),
        )
        .expect("read legacy app builtin skill name");
        assert!(content.contains("path: builtin/create-skill.md"));
    }

    #[test]
    fn export_and_import_skill_package_round_trip_zip() {
        let source_parent = TempDir::new().unwrap();
        let target_parent = TempDir::new().unwrap();
        super::create_skill_package_in_parent_sync(
            source_parent.path(),
            super::SkillCreateRequest {
                kind: super::SkillCreateKind::Package,
                name: "Asset Audit".to_string(),
                package_id: Some("com.example.asset-audit".to_string()),
                version: Some("0.1.0".to_string()),
                summary: Some("Audit assets.".to_string()),
                body: Some("## Instructions\nAudit imported assets.".to_string()),
                ..Default::default()
            },
        )
        .expect("create package");
        let package_root = source_parent.path().join("com.example.asset-audit");
        std::fs::create_dir_all(package_root.join("docs")).unwrap();
        std::fs::write(package_root.join("docs").join("usage.md"), "# Usage\n").unwrap();
        std::fs::create_dir_all(package_root.join("scripts")).unwrap();
        std::fs::write(
            package_root.join("scripts").join("audit.py"),
            "print('ok')\n",
        )
        .unwrap();

        let zip_path = source_parent.path().join("asset-audit.zip");
        let exported = super::export_skill_package_from_parent_sync(
            source_parent.path(),
            "com.example.asset-audit",
            &zip_path.to_string_lossy(),
        )
        .expect("export package");
        assert_eq!(exported.package_id, "com.example.asset-audit");
        assert!(exported.file_count >= 4);
        assert!(zip_path.is_file());

        let imported = super::import_skill_package_to_parent_sync(target_parent.path(), &zip_path)
            .expect("import package");
        assert_eq!(
            imported.package_id.as_deref(),
            Some("com.example.asset-audit")
        );
        let imported_root = target_parent.path().join("com.example.asset-audit");
        assert!(imported_root.join("skill.json").is_file());
        assert!(imported_root.join("SKILL.md").is_file());
        assert!(imported_root.join("docs").join("usage.md").is_file());
        assert!(imported_root.join("scripts").join("audit.py").is_file());
    }

    #[test]
    fn import_skill_package_rejects_archive_traversal() {
        let target_parent = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let zip_path = archive_dir.path().join("bad.zip");
        let zip_file = std::fs::File::create(&zip_path).expect("create zip");
        let mut zip_writer = zip::ZipWriter::new(zip_file);
        zip_writer
            .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .expect("start bad entry");
        zip_writer.write_all(b"escape").expect("write bad entry");
        zip_writer.finish().expect("finish zip");

        let err = super::import_skill_package_to_parent_sync(target_parent.path(), &zip_path)
            .expect_err("archive traversal should fail");
        assert!(err.contains("Invalid archive entry path"));
        assert!(!target_parent.path().join("escape.txt").exists());
    }
}
