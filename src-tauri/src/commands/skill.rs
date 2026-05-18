use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use walkdir::WalkDir;

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{self, KnowledgeDocument, KnowledgeType, SkillSurface};
use crate::workspace::Workspace;

use super::knowledge::{
    get_updated_at, load_skill_config, reconcile_and_emit_knowledge_changed, AppKnowledgeDir,
    SkillConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageLocusManifest {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillPackageSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<SkillPackageCommand>,
    #[serde(default)]
    pub capabilities: SkillPackageCapabilities,
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
    #[serde(
        rename = "x-locus",
        alias = "xLocus",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub locus: Option<SkillPackageLocusManifest>,
}

#[derive(Debug, Clone)]
pub struct SkillPackageRecord {
    pub root: PathBuf,
    pub manifest: SkillPackageManifestFile,
    pub doc_levels: SkillPackageDocLevels,
    pub updated_at: i64,
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

pub fn lookup_skill_config(
    configs: &std::collections::HashMap<String, SkillConfig>,
    source: &str,
    dir_name: &str,
) -> SkillConfig {
    let new_key = config_key(source, dir_name);
    configs
        .get(&new_key)
        .cloned()
        .or_else(|| {
            dir_name
                .strip_prefix("builtin/")
                .and_then(|legacy_name| configs.get(&config_key(source, legacy_name)).cloned())
        })
        .unwrap_or_default()
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
        let cfg = (source == "app").then(|| lookup_skill_config(configs, source, &dir_name));
        manifests.push(build_skill_manifest(
            &document,
            &dir_name,
            source,
            &rel_path,
            get_updated_at(&path),
            cfg.as_ref(),
        ));
    }

    manifests
}

fn normalize_command_trigger(value: &str, fallback: &str) -> String {
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

fn resolve_command_trigger(config: &SkillConfig, fallback: &str) -> String {
    normalize_command_trigger(&config.command_trigger, fallback)
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
        .map(|config| resolve_command_trigger(config, &document.title))
        .filter(|value| !value.trim().is_empty())
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

pub(crate) fn app_skill_package_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(debug_assertions)]
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest_dir.join("..").join("skills"));
    }
    if let Ok(config_dir) = super::persistent_config_dir() {
        candidates.push(config_dir.join("skills"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("skills"));
        }
    }

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
    if let Some(locus) = manifest.locus.take() {
        if manifest.schema.trim().is_empty() {
            manifest.schema = locus.schema;
        }
        if manifest.id.trim().is_empty() {
            manifest.id = locus.id;
        }
        if manifest.version.trim().is_empty() {
            manifest.version = locus.version;
        }
        if manifest.source.is_none() {
            manifest.source = locus.source;
        }
        if manifest.command.is_none() {
            manifest.command = locus.command;
        }
        if manifest.capabilities.unity.is_empty()
            && manifest.capabilities.python.is_empty()
            && manifest.capabilities.cli.is_empty()
        {
            manifest.capabilities = locus.capabilities;
        }
    }
    if manifest.id.trim().is_empty() {
        manifest.id = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("skill")
            .to_string();
    }
    manifest.id = normalize_package_id(&manifest.id)?;
    if manifest.name.trim().is_empty() {
        manifest.name = manifest.id.clone();
    } else {
        manifest.name = manifest.name.trim().to_string();
    }
    manifest.description = manifest.description.trim().to_string();
    manifest.version = manifest.version.trim().to_string();
    manifest.argument_hint = manifest
        .argument_hint
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(command) = manifest.command.as_mut() {
        command.trigger = command
            .trigger
            .take()
            .map(|value| normalize_command_trigger(&value, &manifest.id))
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
    Ok(manifest)
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

fn split_skill_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let first_line_end = content
        .find('\n')
        .ok_or_else(|| "Skill package SKILL.md frontmatter is not terminated".to_string())?;
    if content[..first_line_end].trim_end_matches('\r').trim() != "---" {
        return Err("Skill package SKILL.md frontmatter is missing".to_string());
    }

    let mut offset = first_line_end + 1;
    while offset <= content.len() {
        let line_end = content[offset..]
            .find('\n')
            .map(|index| offset + index)
            .unwrap_or(content.len());
        let line = &content[offset..line_end];
        if line.trim_end_matches('\r').trim() == "---" {
            let body_start = if line_end < content.len() {
                line_end + 1
            } else {
                line_end
            };
            return Ok((&content[first_line_end + 1..offset], &content[body_start..]));
        }
        if line_end == content.len() {
            break;
        }
        offset = line_end + 1;
    }

    Err("Skill package SKILL.md frontmatter is not terminated".to_string())
}

fn strip_optional_skill_frontmatter(content: &str) -> &str {
    if content
        .strip_prefix('\u{feff}')
        .unwrap_or(content)
        .starts_with("---")
    {
        split_skill_frontmatter(content)
            .map(|(_, body)| body)
            .unwrap_or(content)
    } else {
        content
    }
}

fn scan_package_document_levels(body: &str) -> SkillPackageDocLevels {
    SkillPackageDocLevels {
        has_l0: markdown_has_l_section(body, "L0"),
        has_l1: markdown_has_l_section(body, "L1"),
        has_l2: markdown_has_l_section(body, "L2"),
    }
}

fn load_skill_package_record(root: &Path) -> Result<SkillPackageRecord, String> {
    let root_doc_path = root.join("SKILL.md");
    let raw = std::fs::read_to_string(&root_doc_path)
        .map_err(|e| format!("Failed to read {}: {}", root_doc_path.display(), e))?;
    let (yaml, body) = split_skill_frontmatter(&raw)?;
    let manifest: SkillPackageManifestFile = serde_yaml::from_str(yaml)
        .map_err(|e| format!("Invalid skill package frontmatter: {}", e))?;
    let manifest = normalize_package_manifest(manifest, root)?;
    let doc_levels = scan_package_document_levels(body);
    Ok(SkillPackageRecord {
        root: root.to_path_buf(),
        updated_at: get_updated_at(&root_doc_path),
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
            if !root.is_dir() || !root.join("SKILL.md").is_file() {
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
        if direct_root.join("SKILL.md").is_file() {
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
            if !root.is_dir() || !root.join("SKILL.md").is_file() {
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

pub fn resolve_skill_package_root_sync(package_id: &str) -> Result<PathBuf, String> {
    find_skill_package(package_id).map(|record| record.root)
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

fn package_document_id(package_id: &str) -> String {
    let normalized = package_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("kd_skill_package_{}", normalized)
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
        &manifest.id,
    )
}

fn package_to_document(
    record: &SkillPackageRecord,
    doc_rel_path: &str,
    body: String,
) -> KnowledgeDocument {
    let manifest = &record.manifest;
    let path = if doc_rel_path == package_root_doc_rel_path(manifest) {
        format!("{}/SKILL.md", manifest.id)
    } else {
        format!("{}/{}", manifest.id, doc_rel_path)
    };
    let command_enabled = package_command_enabled(manifest);
    let skill_surface = package_skill_surface(manifest);
    KnowledgeDocument {
        id: package_document_id(&manifest.id),
        doc_type: KnowledgeType::Skill,
        path,
        title: manifest.name.clone(),
        inject_mode: knowledge_store::KnowledgeInjectMode::None,
        inherit_inject_mode: false,
        inject_mode_source: Default::default(),
        summary_enabled: true,
        command_enabled,
        read_only: true,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::App,
        inherit_ai_config: false,
        ai_config_source: Default::default(),
        explicit_maintenance_rules: false,
        external_source: package_source_summary(manifest),
        skill_enabled: Some(package_skill_enabled(manifest)),
        skill_surface: Some(skill_surface),
        command_trigger: Some(package_command_trigger(manifest)),
        argument_hint: package_argument_hint(manifest),
        summary: (!manifest.description.trim().is_empty()).then(|| manifest.description.clone()),
        body,
        maintenance_rules: None,
        created_at: record.updated_at,
        updated_at: record.updated_at,
    }
}

pub(crate) fn read_skill_package_document_sync(
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

    for record in list_skill_packages_sync() {
        let Some(doc_rel_path) =
            package_doc_rel_path_for_virtual_path(&record.manifest, virtual_path)?
        else {
            continue;
        };
        let file_path = package_file_path(&record.root, &doc_rel_path)?;
        if !file_path.is_file() {
            return Err(format!(
                "Skill package document not found: {}",
                virtual_path
            ));
        }
        let raw = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read skill package document: {}", e))?;
        let body = strip_optional_skill_frontmatter(&raw).to_string();
        let mut document = package_to_document(&record, &doc_rel_path, body);
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

fn package_to_list_item(record: &SkillPackageRecord) -> knowledge_store::KnowledgeListItem {
    let manifest = &record.manifest;
    let command_enabled = package_command_enabled(manifest);
    knowledge_store::KnowledgeListItem {
        id: package_document_id(&manifest.id),
        doc_type: KnowledgeType::Skill,
        path: format!("{}/SKILL.md", manifest.id),
        title: manifest.name.clone(),
        inject_mode: knowledge_store::KnowledgeInjectMode::None,
        summary_enabled: true,
        command_enabled,
        read_only: true,
        ai_maintained: false,
        explicit_maintenance_rules: false,
        storage_source: knowledge_store::KnowledgeStorageSource::App,
        external_source: package_source_summary(manifest),
        skill_enabled: Some(package_skill_enabled(manifest)),
        skill_surface: Some(package_skill_surface(manifest)),
        command_trigger: Some(package_command_trigger(manifest)),
        argument_hint: package_argument_hint(manifest),
        created_at: record.updated_at,
        updated_at: record.updated_at,
        has_summary: !manifest.description.trim().is_empty(),
        has_body_content: true,
        byte_size: package_file_path(&record.root, &package_root_doc_rel_path(manifest))
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len()),
        lexical_search_enabled: Some(false),
        semantic_search_enabled: Some(false),
        summary: (!manifest.description.trim().is_empty()).then(|| manifest.description.clone()),
    }
}

pub(crate) fn list_skill_package_knowledge_items_sync(
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
    list_skill_packages_sync()
        .into_iter()
        .map(|record| package_to_list_item(&record))
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
        .map(|config| resolve_command_trigger(config, &manifest.name))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| package_command_trigger(manifest));

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
        let cfg = lookup_skill_config(&configs, "app", &package.manifest.id);
        manifests.push(build_package_skill_manifest(&package, "app", Some(&cfg)));
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

#[derive(Serialize)]
struct SkillPackageScaffoldFrontmatter {
    name: String,
    description: String,
    #[serde(rename = "argument-hint", skip_serializing_if = "Option::is_none")]
    argument_hint: Option<String>,
    #[serde(
        rename = "disable-model-invocation",
        skip_serializing_if = "Option::is_none"
    )]
    disable_model_invocation: Option<bool>,
    #[serde(rename = "x-locus")]
    locus: SkillPackageLocusManifest,
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
    let command_enabled = request.command_enabled.unwrap_or(true);
    let command_trigger = if command_enabled {
        let trigger = optional_trimmed(request.command_trigger)
            .map(|value| normalize_command_trigger(&value, &name))
            .unwrap_or_else(|| normalize_command_trigger("", &name));
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

fn create_skill_package_in_parent_sync(
    package_parent: &Path,
    request: SkillCreateRequest,
) -> Result<SkillManifest, String> {
    if request.kind != SkillCreateKind::Package {
        return Err("Use kind='package' for app Skill packages".to_string());
    }
    if optional_trimmed(request.path.clone()).is_some() {
        return Err("'path' is only supported for md Skill documents.".to_string());
    }

    let name = required_skill_create_text(Some(request.name), "name")?;
    let package_id = normalize_package_id(&required_skill_create_text(
        request.package_id,
        "packageId",
    )?)?;
    let version = required_skill_create_text(request.version, "version")?;
    let summary = required_skill_create_text(request.summary, "summary")?;
    let argument_hint = optional_trimmed(request.argument_hint);
    let command_enabled = request.command_enabled.unwrap_or(true);
    let default_trigger = default_package_command_name(&package_id);
    let command_trigger = if command_enabled {
        let trigger = optional_trimmed(request.command_trigger)
            .map(|value| normalize_command_trigger(&value, &default_trigger))
            .unwrap_or_else(|| normalize_command_trigger("", &default_trigger));
        (!trigger.is_empty()).then_some(trigger)
    } else {
        None
    };
    let model_invocation_enabled = request.model_invocation_enabled.unwrap_or(true);

    let package_root = package_parent.join(&package_id);
    if package_root.exists() {
        return Err(format!("Skill package already exists: {}", package_id));
    }
    std::fs::create_dir_all(&package_root)
        .map_err(|e| format!("Failed to create Skill package directory: {}", e))?;

    let write_result = (|| {
        let frontmatter = SkillPackageScaffoldFrontmatter {
            name: name.clone(),
            description: summary,
            argument_hint: argument_hint.clone(),
            disable_model_invocation: Some(!model_invocation_enabled),
            locus: SkillPackageLocusManifest {
                schema: "locus.skill.v1".to_string(),
                id: package_id.clone(),
                version,
                source: None,
                command: Some(SkillPackageCommand {
                    enabled: Some(command_enabled),
                    trigger: command_trigger,
                    argument_hint,
                }),
                capabilities: SkillPackageCapabilities::default(),
            },
        };
        let yaml = serde_yaml::to_string(&frontmatter)
            .map_err(|e| format!("Failed to render Skill package frontmatter: {}", e))?;
        let content = format!(
            "---\n{}---\n\n{}",
            yaml,
            package_skill_body(&name, request.body)
        );
        let root_doc_path = package_root.join("SKILL.md");
        std::fs::write(&root_doc_path, content)
            .map_err(|e| format!("Failed to write {}: {}", root_doc_path.display(), e))?;
        let record = load_skill_package_record(&package_root)?;
        Ok(build_package_skill_manifest(&record, "app", None))
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_dir_all(&package_root);
    }
    write_result
}

pub fn create_skill_package_sync(request: SkillCreateRequest) -> Result<SkillManifest, String> {
    let package_parent = writable_app_skill_package_dir()?;
    create_skill_package_in_parent_sync(&package_parent, request)
}

pub fn create_skill_sync(
    working_dir: &str,
    request: SkillCreateRequest,
) -> Result<SkillManifest, String> {
    match request.kind {
        SkillCreateKind::Md => create_skill_document_sync(working_dir, request),
        SkillCreateKind::Package => create_skill_package_sync(request),
    }
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
            let cfg = lookup_skill_config(&configs, "app", &record.manifest.id);
            return Ok(build_package_skill_manifest(&record, "app", Some(&cfg)));
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

    let configs = load_skill_config(working_dir);
    let cfg =
        (source == "app").then(|| lookup_skill_config(&configs, source, &normalized_dir_name));
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
    let manifest = create_skill_sync(
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
        },
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
    use super::{is_valid_skill_scaffold_name, list_skills_sync, read_skill_manifest_sync};
    use crate::knowledge_store::SkillSurface;
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
    fn load_skill_package_record_reads_skill_md_frontmatter() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("SKILL.md"),
            r#"---
name: Asset Audit
description: Audit Unity assets and report cleanup tasks.
argument-hint: <scope>
disable-model-invocation: true
x-locus:
  schema: locus.skill.v1
  id: com.example.asset-audit
  version: 0.1.0
  source:
    type: github
    url: https://github.com/example/locus-skills
    reference: asset-audit
  command:
    enabled: true
    trigger: /asset-audit
  capabilities:
    unity:
      - name: AssetAuditBridge
        path: unity/Editor/SkillBridge.cs
        api: unity_execute
---

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
        assert_eq!(
            super::package_skill_surface(&record.manifest),
            SkillSurface::Command
        );
        assert!(!record.doc_levels.has_l0);
        assert!(!record.doc_levels.has_l2);
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
                ..Default::default()
            },
        )
        .expect("create skill document");
        assert_eq!(manifest.dir_name, "asset-audit");
        assert_eq!(manifest.command_trigger, "/asset-audit");
        assert_eq!(manifest.description, "Audit Unity assets.");

        let saved = crate::knowledge_store::read_document(
            &working_dir,
            crate::knowledge_store::KnowledgeType::Skill,
            "asset-audit.md",
            "full",
        )
        .expect("read created skill document");
        assert_eq!(saved.document.body, "## Instructions");
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
}
