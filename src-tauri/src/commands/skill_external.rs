//! Discovery of generic agent skills (Claude Code / Agent Skills format) from
//! conventional directories such as `~/.claude/skills`, `~/.agents/skills`,
//! `~/.codex/skills` and their workspace-level counterparts.
//!
//! External skills are read-only inside Locus: they are listed, previewable,
//! and can be enabled per workspace, but Locus never edits, installs, or
//! registers executable tools from them. They are disabled by default and the
//! enablement lives in the per-workspace skill config.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{
    self, KnowledgeDocument, KnowledgeInjectMode, KnowledgeType, SkillSurface,
};
use crate::workspace::Workspace;

use super::knowledge::{get_updated_at, reconcile_and_emit_knowledge_changed, SkillConfig};
use super::skill::{
    default_package_command_name, normalize_command_trigger, resolve_config_command_trigger,
    split_optional_frontmatter, strip_utf8_bom, SkillManifest, SkillManifestKind,
};

/// First segment of the virtual knowledge path namespace reserved for
/// external skills: `external/<provider>/<slug>/SKILL.md`.
pub(crate) const EXTERNAL_SKILL_VIRTUAL_ROOT: &str = "external";

const EXTERNAL_SKILL_ROOT_DOC_FILE_NAME: &str = "SKILL.md";

// ── Identity ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalSkillProvider {
    Claude,
    Agents,
    Codex,
    /// Skills bundled inside installed Claude Code plugins
    /// (`~/.claude/plugins/**/skills/<slug>`), one scan root per plugin.
    ClaudePlugin,
}

impl ExternalSkillProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Agents => "agents",
            Self::Codex => "codex",
            Self::ClaudePlugin => "claude-plugin",
        }
    }

    // Only referenced by the non-test scan-root assembly.
    #[cfg_attr(test, allow(dead_code))]
    fn home_component(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Agents => ".agents",
            Self::Codex => ".codex",
            Self::ClaudePlugin => ".claude",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalSkillScope {
    Project,
    User,
}

impl ExternalSkillScope {
    pub fn source(self) -> &'static str {
        match self {
            Self::Project => "externalProject",
            Self::User => "externalUser",
        }
    }
}

pub(crate) fn source_is_external_skill(source: &str) -> bool {
    source == "externalUser" || source == "externalProject"
}

// ── Frontmatter (generic Agent Skills format, parsed leniently) ──

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalSkillFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(rename = "argument-hint", alias = "argumentHint", default)]
    pub argument_hint: Option<String>,
    #[serde(
        rename = "disable-model-invocation",
        alias = "disableModelInvocation",
        default
    )]
    pub disable_model_invocation: Option<bool>,
    #[serde(rename = "user-invocable", alias = "userInvocable", default)]
    pub user_invocable: Option<bool>,
    /// Every frontmatter field Locus does not consume (license, metadata,
    /// allowed-tools, hooks, ...), kept verbatim for display in the UI.
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_yaml::Value>,
}

/// Map a Claude Code tool name from `allowed-tools` to the Locus built-in
/// tool it corresponds to. Names already matching a Locus tool pass through
/// via the registry's case-insensitive lookup; camel-case CC names that don't
/// lowercase into a Locus name get an explicit alias. `None` means no Locus
/// equivalent exists.
fn map_external_tool_name(raw: &str) -> Option<String> {
    // `Bash(git:*)` style entries scope a permission; only the tool name
    // before the parenthesis is meaningful here.
    let name = raw.split('(').next().unwrap_or(raw).trim();
    if name.is_empty() {
        return None;
    }
    let lowered = name.to_ascii_lowercase();
    let candidate = match lowered.as_str() {
        "webfetch" => "web_fetch",
        "askuserquestion" => "ask",
        "multiedit" => "edit",
        "ls" => "list",
        other => other,
    };
    crate::tool::built_in_tool_name_keys()
        .contains(candidate)
        .then(|| candidate.to_string())
}

/// Locus tool names activated by the skill's `allowed-tools` declaration.
/// The CC semantics (permission allow-list) are not enforced; the list is
/// consumed as an activation hint so declared tools are loaded when the skill
/// is selected or read, exactly like package skill `tools`. Unmappable names
/// are dropped silently.
pub(crate) fn external_allowed_tool_activation_names(
    extra_metadata: Option<&serde_json::Value>,
) -> Vec<String> {
    let Some(value) = extra_metadata.and_then(|extra| extra.get("allowed-tools")) else {
        return Vec::new();
    };
    let raw_names: Vec<String> = match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(joined) => {
            joined.split(',').map(str::to_string).collect()
        }
        _ => Vec::new(),
    };
    let mut names = Vec::new();
    for raw in raw_names {
        let Some(mapped) = map_external_tool_name(&raw) else {
            continue;
        };
        if !names.contains(&mapped) {
            names.push(mapped);
        }
    }
    names
}

fn yaml_extra_to_json(
    extra: std::collections::BTreeMap<String, serde_yaml::Value>,
) -> Option<serde_json::Value> {
    if extra.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    for (key, value) in extra {
        let json = serde_json::to_value(&value).unwrap_or_else(|_| {
            serde_json::Value::String(
                serde_yaml::to_string(&value)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            )
        });
        map.insert(key, json);
    }
    Some(serde_json::Value::Object(map))
}

// ── Record ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ExternalSkillRecord {
    pub provider: ExternalSkillProvider,
    pub scope: ExternalSkillScope,
    pub slug: String,
    /// Canonical on-disk skill directory (symlinks resolved).
    pub root: PathBuf,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub argument_hint: Option<String>,
    pub disable_model_invocation: bool,
    pub user_invocable: bool,
    /// Unconsumed frontmatter fields, surfaced verbatim in the UI.
    pub extra_metadata: Option<serde_json::Value>,
    pub updated_at: i64,
}

impl ExternalSkillRecord {
    /// Stable identity used as the manifest `dir_name` and in skill config
    /// keys: `external/<provider>/<slug>`.
    pub fn dir_name(&self) -> String {
        format!(
            "{}/{}/{}",
            EXTERNAL_SKILL_VIRTUAL_ROOT,
            self.provider.as_str(),
            self.slug
        )
    }

    /// Virtual knowledge path of the root document (relative to the skill
    /// knowledge type root): `external/<provider>/<slug>/SKILL.md`.
    pub fn virtual_path(&self) -> String {
        format!("{}/{}", self.dir_name(), EXTERNAL_SKILL_ROOT_DOC_FILE_NAME)
    }

    pub fn skill_md_path(&self) -> PathBuf {
        self.root.join(EXTERNAL_SKILL_ROOT_DOC_FILE_NAME)
    }

    fn locator(&self) -> String {
        format!(
            "external://{}/{}/{}",
            match self.scope {
                ExternalSkillScope::Project => "project",
                ExternalSkillScope::User => "user",
            },
            self.provider.as_str(),
            self.slug
        )
    }
}

// ── Scan roots ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct ExternalSkillScanRoot {
    pub provider: ExternalSkillProvider,
    pub scope: ExternalSkillScope,
    pub path: PathBuf,
    /// Namespace prefix folded into every slug found under this root
    /// (`<prefix>--<dir>`), so same-named skills from different Claude Code
    /// plugins keep distinct identities. Empty for plain skill directories.
    pub slug_prefix: String,
}

/// Maximum directory depth (relative to `~/.claude/plugins`) at which a
/// plugin's `skills` directory is searched. Covers the known layouts:
/// `cache/<marketplace>/<plugin>/skills` (depth 4) and
/// `repos/<repo>/plugins/<plugin>/skills` (depth 5).
const CLAUDE_PLUGIN_SKILLS_MAX_DEPTH: usize = 5;

/// Discover `skills` directories bundled inside installed Claude Code
/// plugins. Each hit becomes its own scan root whose slugs are prefixed with
/// the owning plugin directory name.
fn discover_claude_plugin_skill_roots(plugins_root: &Path) -> Vec<ExternalSkillScanRoot> {
    let mut roots = Vec::new();
    if !plugins_root.is_dir() {
        return roots;
    }
    for entry in walkdir::WalkDir::new(plugins_root)
        .max_depth(CLAUDE_PLUGIN_SKILLS_MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !super::skill::is_ignored_package_walk_dir(entry.path())
        })
        .flatten()
    {
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|name| name.eq_ignore_ascii_case("skills"))
            != Some(true)
        {
            continue;
        }
        let Some(plugin_name) = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        // `plugins/skills` (depth 1) has no plugin directory to attribute to.
        if entry.depth() < 2 {
            continue;
        }
        roots.push(ExternalSkillScanRoot {
            provider: ExternalSkillProvider::ClaudePlugin,
            scope: ExternalSkillScope::User,
            path: path.to_path_buf(),
            slug_prefix: plugin_name,
        });
    }
    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots
}

/// Fixed scan-root order: project roots first so a project-level skill wins
/// over a user-level skill with the same provider and slug.
fn external_skill_scan_roots(working_dir: &str) -> Vec<ExternalSkillScanRoot> {
    // Unit tests must not observe the developer machine's real skill
    // directories; they exercise `scan_external_skills_from_roots` directly.
    #[cfg(test)]
    {
        let _ = working_dir;
        Vec::new()
    }

    #[cfg(not(test))]
    {
        let mut roots = Vec::new();
        let trimmed = working_dir.trim();
        if !trimmed.is_empty() {
            let workspace = Path::new(trimmed);
            for provider in [ExternalSkillProvider::Claude, ExternalSkillProvider::Agents] {
                roots.push(ExternalSkillScanRoot {
                    provider,
                    scope: ExternalSkillScope::Project,
                    path: workspace.join(provider.home_component()).join("skills"),
                    slug_prefix: String::new(),
                });
            }
        }
        if let Some(home) = dirs::home_dir() {
            for provider in [
                ExternalSkillProvider::Claude,
                ExternalSkillProvider::Agents,
                ExternalSkillProvider::Codex,
            ] {
                roots.push(ExternalSkillScanRoot {
                    provider,
                    scope: ExternalSkillScope::User,
                    path: home.join(provider.home_component()).join("skills"),
                    slug_prefix: String::new(),
                });
            }
            roots.extend(discover_claude_plugin_skill_roots(
                &home.join(".claude").join("plugins"),
            ));
        }
        roots
    }
}

fn canonical_path_key(path: &Path) -> String {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn load_external_skill_frontmatter(skill_md: &Path) -> Option<ExternalSkillFrontmatter> {
    let raw = match std::fs::read_to_string(skill_md) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::warn!(
                log_module = "Skill",
                path = %skill_md.display(),
                error = %error,
                "failed to read external skill SKILL.md"
            );
            return None;
        }
    };
    match split_optional_frontmatter::<ExternalSkillFrontmatter>(&raw) {
        Ok((frontmatter, _)) => Some(frontmatter),
        Err(error) => {
            // Lenient: an unparsable frontmatter still lists the skill so the
            // user can see and diagnose it, just without metadata.
            tracing::warn!(
                log_module = "Skill",
                path = %skill_md.display(),
                error = %error,
                "failed to parse external skill frontmatter; listing without metadata"
            );
            Some(ExternalSkillFrontmatter::default())
        }
    }
}

fn optional_trimmed_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn build_external_skill_record(
    provider: ExternalSkillProvider,
    scope: ExternalSkillScope,
    slug: &str,
    root: PathBuf,
) -> Option<ExternalSkillRecord> {
    let skill_md = root.join(EXTERNAL_SKILL_ROOT_DOC_FILE_NAME);
    let frontmatter = load_external_skill_frontmatter(&skill_md)?;
    let name = optional_trimmed_text(frontmatter.name).unwrap_or_else(|| slug.to_string());
    let description = optional_trimmed_text(frontmatter.description).unwrap_or_default();
    let updated_at = get_updated_at(&skill_md);
    Some(ExternalSkillRecord {
        provider,
        scope,
        slug: slug.to_string(),
        root,
        name,
        description,
        version: optional_trimmed_text(frontmatter.version),
        argument_hint: optional_trimmed_text(frontmatter.argument_hint),
        disable_model_invocation: frontmatter.disable_model_invocation.unwrap_or(false),
        user_invocable: frontmatter.user_invocable.unwrap_or(true),
        extra_metadata: yaml_extra_to_json(frontmatter.extra),
        updated_at,
    })
}

fn external_slug_is_valid(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.starts_with('.')
        && !slug.contains('/')
        && !slug.contains('\\')
        && slug != ".."
}

/// Scan the given roots for generic skill directories (`<slug>/SKILL.md`).
///
/// Deduplicates both by canonical on-disk entity (a skill installed in
/// `~/.agents/skills` and symlinked into `~/.claude/skills` is listed once)
/// and by `(provider, slug)` identity (a project-level skill shadows a
/// user-level skill with the same identity). When a discovered entry is a
/// link whose canonical target lives under another known root, the record is
/// re-attributed to that root so its identity does not depend on link layout.
pub(crate) fn scan_external_skills_from_roots(
    roots: &[ExternalSkillScanRoot],
) -> Vec<ExternalSkillRecord> {
    let canonical_roots = roots
        .iter()
        .filter(|root| root.path.is_dir())
        .map(|root| {
            (
                format!("{}/", canonical_path_key(&root.path)),
                root.provider,
                root.scope,
                root.slug_prefix.clone(),
            )
        })
        .collect::<Vec<_>>();

    let mut seen_entities = BTreeSet::new();
    let mut seen_identities = BTreeSet::new();
    let mut records = Vec::new();

    for root in roots {
        let entries = match std::fs::read_dir(&root.path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let skill_md = dir.join(EXTERNAL_SKILL_ROOT_DOC_FILE_NAME);
            // `is_file` follows symlinks/junctions, so linked skill dirs work.
            if !skill_md.is_file() {
                continue;
            }
            let canonical = dunce::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
            let entity_key = canonical_path_key(&canonical);
            if !seen_entities.insert(entity_key.clone()) {
                continue;
            }
            let (provider, scope, slug_prefix) = canonical_roots
                .iter()
                .find(|(root_key, _, _, _)| entity_key.starts_with(root_key.as_str()))
                .map(|(_, provider, scope, prefix)| (*provider, *scope, prefix.as_str()))
                .unwrap_or((root.provider, root.scope, root.slug_prefix.as_str()));
            let slug = canonical
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
                .or_else(|| {
                    dir.file_name()
                        .and_then(|value| value.to_str())
                        .map(str::to_string)
                });
            let Some(slug) = slug else {
                continue;
            };
            let slug = if slug_prefix.is_empty() {
                slug
            } else {
                format!("{}--{}", slug_prefix, slug)
            };
            if !external_slug_is_valid(&slug) {
                continue;
            }
            if !seen_identities.insert(format!("{}/{}", provider.as_str(), slug)) {
                continue;
            }
            if let Some(record) = build_external_skill_record(provider, scope, &slug, canonical) {
                records.push(record);
            }
        }
    }

    records.sort_by(|a, b| a.dir_name().cmp(&b.dir_name()));
    records
}

// ── Cache ────────────────────────────────────────────────────

struct ExternalSkillCacheEntry {
    working_dir: String,
    records: Arc<Vec<ExternalSkillRecord>>,
}

static EXTERNAL_SKILL_CACHE: Mutex<Option<ExternalSkillCacheEntry>> = Mutex::new(None);

fn cache_lock() -> std::sync::MutexGuard<'static, Option<ExternalSkillCacheEntry>> {
    EXTERNAL_SKILL_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// External skill list for the workspace. Scans the disk once per workspace
/// and serves the cached result afterwards so hot paths (agent turns) never
/// re-scan; `invalidate_external_skill_cache` or a workspace switch forces a
/// fresh scan.
pub(crate) fn list_external_skills_cached(working_dir: &str) -> Arc<Vec<ExternalSkillRecord>> {
    {
        let guard = cache_lock();
        if let Some(entry) = guard.as_ref() {
            if entry.working_dir == working_dir {
                return entry.records.clone();
            }
        }
    }
    let records = Arc::new(scan_external_skills_from_roots(&external_skill_scan_roots(
        working_dir,
    )));
    *cache_lock() = Some(ExternalSkillCacheEntry {
        working_dir: working_dir.to_string(),
        records: records.clone(),
    });
    records
}

pub(crate) fn invalidate_external_skill_cache() {
    *cache_lock() = None;
}

// ── Configured values (workspace override on top of author defaults) ──

/// External skills are disabled by default; enabling one is an explicit
/// per-workspace user action stored in the skill config.
pub(crate) fn configured_external_skill_enabled(
    override_config: Option<&SkillConfig>,
) -> bool {
    override_config.map(|config| config.enabled).unwrap_or(false)
}

/// Default surface honors the skill author's declaration.
fn external_skill_default_surface(record: &ExternalSkillRecord) -> SkillSurface {
    let command = record.user_invocable;
    let auto = !record.disable_model_invocation;
    match (command, auto) {
        (true, true) => SkillSurface::Both,
        (true, false) => SkillSurface::Command,
        (false, true) => SkillSurface::Auto,
        (false, false) => SkillSurface::Command,
    }
}

pub(crate) fn configured_external_skill_surface(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> SkillSurface {
    override_config
        .map(|config| config.surface)
        .unwrap_or_else(|| external_skill_default_surface(record))
}

pub(crate) fn configured_external_model_recall_enabled(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> bool {
    configured_external_skill_enabled(override_config)
        && configured_external_skill_surface(record, override_config).allows_auto()
        && configured_external_inject_mode(override_config) != KnowledgeInjectMode::None
}

fn configured_external_command_enabled(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> bool {
    configured_external_skill_enabled(override_config)
        && configured_external_skill_surface(record, override_config).allows_command()
}

fn configured_external_command_trigger(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> String {
    override_config
        .and_then(resolve_config_command_trigger)
        .unwrap_or_else(|| {
            normalize_command_trigger("", &default_package_command_name(&record.slug))
        })
}

fn configured_external_summary(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> String {
    override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .unwrap_or_else(|| record.description.clone())
}

fn configured_external_inject_mode(override_config: Option<&SkillConfig>) -> KnowledgeInjectMode {
    override_config
        .and_then(|config| config.inject_mode)
        .unwrap_or(KnowledgeInjectMode::Excerpt)
}

// ── Manifest ─────────────────────────────────────────────────

pub(crate) fn build_external_skill_manifest(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> SkillManifest {
    let skill_description = override_config
        .and_then(|config| {
            (!config.description.trim().is_empty()).then(|| config.description.clone())
        })
        .or_else(|| (!record.description.trim().is_empty()).then(|| record.description.clone()));
    SkillManifest {
        name: record.name.clone(),
        description: record.description.clone(),
        argument_hint: record.argument_hint.clone().unwrap_or_default(),
        dir_name: record.dir_name(),
        source: record.scope.source().to_string(),
        rel_path: format!("skill/{}", record.virtual_path()),
        updated_at: record.updated_at,
        skill_enabled: configured_external_skill_enabled(override_config),
        skill_surface: configured_external_skill_surface(record, override_config),
        skill_description,
        command_trigger: configured_external_command_trigger(record, override_config),
        tools: external_allowed_tool_activation_names(record.extra_metadata.as_ref()),
        kind: SkillManifestKind::External,
        package_id: None,
        package_version: record.version.clone(),
        has_unity: false,
        has_l0: false,
        has_l1: false,
        has_l2: false,
        plugin_id: None,
        plugin_scope: None,
        origin_path: Some(record.root.to_string_lossy().replace('\\', "/")),
        extra_metadata: record.extra_metadata.clone(),
    }
}

// ── Virtual knowledge documents ──────────────────────────────

fn normalize_external_virtual_path(virtual_path: &str) -> String {
    let normalized = virtual_path.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    normalized
        .strip_prefix("skill/")
        .unwrap_or(normalized)
        .to_string()
}

/// True when the virtual path falls inside the reserved `external/`
/// namespace, whether or not a matching skill exists.
pub(crate) fn external_skill_virtual_path_in_namespace(virtual_path: &str) -> bool {
    let normalized = normalize_external_virtual_path(virtual_path);
    normalized == EXTERNAL_SKILL_VIRTUAL_ROOT
        || normalized.starts_with(&format!("{}/", EXTERNAL_SKILL_VIRTUAL_ROOT))
}

/// Resolve a virtual path to the external skill that owns it (the skill
/// directory itself or its root `SKILL.md`). Deeper document paths resolve
/// through `external_record_and_doc_rel_path_for_virtual_path`.
pub(crate) fn external_record_for_virtual_path(
    records: &[ExternalSkillRecord],
    virtual_path: &str,
) -> Option<ExternalSkillRecord> {
    let normalized = normalize_external_virtual_path(virtual_path);
    records
        .iter()
        .find(|record| {
            normalized == record.dir_name() || normalized == record.virtual_path()
        })
        .cloned()
}

/// Resolve a virtual path to the owning external skill plus the on-disk
/// relative document path. The skill directory and its root `SKILL.md` map to
/// `SKILL.md`; deeper paths must be markdown documents and are validated
/// against traversal (`..`, absolute segments) before touching the disk.
pub(crate) fn external_record_and_doc_rel_path_for_virtual_path(
    records: &[ExternalSkillRecord],
    virtual_path: &str,
) -> Option<(ExternalSkillRecord, String)> {
    let normalized = normalize_external_virtual_path(virtual_path);
    for record in records {
        if normalized == record.dir_name() || normalized == record.virtual_path() {
            return Some((
                record.clone(),
                EXTERNAL_SKILL_ROOT_DOC_FILE_NAME.to_string(),
            ));
        }
        let Some(rest) = normalized.strip_prefix(&format!("{}/", record.dir_name())) else {
            continue;
        };
        if rest.eq_ignore_ascii_case(EXTERNAL_SKILL_ROOT_DOC_FILE_NAME) {
            return Some((
                record.clone(),
                EXTERNAL_SKILL_ROOT_DOC_FILE_NAME.to_string(),
            ));
        }
        if !super::skill::package_rel_path_is_markdown_document(rest) {
            return None;
        }
        let Ok(rel_path) = super::skill::normalize_package_rel_path(rest) else {
            return None;
        };
        return Some((record.clone(), rel_path));
    }
    None
}

/// Markdown documents bundled in the skill directory, root `SKILL.md`
/// included, using the same walk rules as skill packages.
pub(crate) fn list_external_document_rel_paths(record: &ExternalSkillRecord) -> Vec<String> {
    let mut paths = BTreeSet::new();
    let root = &record.root;
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !entry.file_type().is_dir() || !super::skill::is_ignored_package_walk_dir(entry.path())
        })
        .flatten()
    {
        if !entry.file_type().is_file() || super::skill::is_ignored_package_walk_file(entry.path())
        {
            continue;
        }
        let Ok(rel_path) = entry.path().strip_prefix(root) else {
            continue;
        };
        let raw_rel_path = rel_path.to_string_lossy().replace('\\', "/");
        let Ok(normalized_rel_path) = super::skill::normalize_package_rel_path(&raw_rel_path)
        else {
            continue;
        };
        if !super::skill::package_rel_path_is_markdown_document(&normalized_rel_path) {
            continue;
        }
        paths.insert(normalized_rel_path);
    }
    paths.into_iter().collect()
}

fn external_doc_is_root(doc_rel_path: &str) -> bool {
    doc_rel_path.eq_ignore_ascii_case(EXTERNAL_SKILL_ROOT_DOC_FILE_NAME)
}

fn external_document_virtual_path(record: &ExternalSkillRecord, doc_rel_path: &str) -> String {
    if external_doc_is_root(doc_rel_path) {
        // Canonical casing regardless of how the file is named on a
        // case-insensitive filesystem.
        return record.virtual_path();
    }
    format!("{}/{}", record.dir_name(), doc_rel_path)
}

fn external_document_title(record: &ExternalSkillRecord, doc_rel_path: &str) -> String {
    if external_doc_is_root(doc_rel_path) {
        return record.name.clone();
    }
    doc_rel_path
        .rsplit('/')
        .next()
        .unwrap_or(doc_rel_path)
        .trim_end_matches(".md")
        .to_string()
}

pub(crate) fn external_record_for_dir_name(
    records: &[ExternalSkillRecord],
    dir_name: &str,
) -> Option<ExternalSkillRecord> {
    let normalized = dir_name.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    records
        .iter()
        .find(|record| record.dir_name() == normalized)
        .cloned()
}

fn external_document_id(record: &ExternalSkillRecord) -> String {
    let sanitized = format!("{}_{}", record.provider.as_str(), record.slug)
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let hash = blake3::hash(record.dir_name().as_bytes()).to_hex().to_string();
    format!("kd_skill_external_{}_{}", sanitized, &hash[..8])
}

/// Document id for a bundled document. The root `SKILL.md` keeps the
/// historical id shape so existing index rows stay stable.
fn external_document_id_for(record: &ExternalSkillRecord, doc_rel_path: &str) -> String {
    if external_doc_is_root(doc_rel_path) {
        return external_document_id(record);
    }
    let hash = blake3::hash(external_document_virtual_path(record, doc_rel_path).as_bytes())
        .to_hex()
        .to_string();
    format!("{}_{}", external_document_id(record), &hash[..8])
}

fn external_source_summary(
    record: &ExternalSkillRecord,
) -> Option<knowledge_store::KnowledgeExternalSource> {
    Some(knowledge_store::KnowledgeExternalSource {
        provider: knowledge_store::KnowledgeSourceProvider::Package,
        locator: Some(record.locator()),
        source_id: Some(record.dir_name()),
        sync_enabled: false,
        ..Default::default()
    })
}

fn external_storage_source(record: &ExternalSkillRecord) -> knowledge_store::KnowledgeStorageSource {
    match record.scope {
        ExternalSkillScope::Project => knowledge_store::KnowledgeStorageSource::Project,
        ExternalSkillScope::User => knowledge_store::KnowledgeStorageSource::App,
    }
}

fn external_to_list_item_for(
    record: &ExternalSkillRecord,
    doc_rel_path: &str,
    override_config: Option<&SkillConfig>,
) -> knowledge_store::KnowledgeListItem {
    let is_root = external_doc_is_root(doc_rel_path);
    let enabled = is_root && configured_external_skill_enabled(override_config);
    let surface = if is_root {
        configured_external_skill_surface(record, override_config)
    } else {
        SkillSurface::Command
    };
    let summary = if is_root {
        configured_external_summary(record, override_config)
    } else {
        String::new()
    };
    let file_path = record.root.join(doc_rel_path);
    let updated_at = get_updated_at(&file_path).max(record.updated_at);
    knowledge_store::KnowledgeListItem {
        id: external_document_id_for(record, doc_rel_path),
        doc_type: KnowledgeType::Skill,
        path: external_document_virtual_path(record, doc_rel_path),
        title: external_document_title(record, doc_rel_path),
        inject_mode: if is_root {
            configured_external_inject_mode(override_config)
        } else {
            KnowledgeInjectMode::None
        },
        summary_enabled: is_root,
        command_enabled: is_root && configured_external_command_enabled(record, override_config),
        read_only: true,
        ai_maintained: false,
        explicit_maintenance_rules: false,
        storage_source: external_storage_source(record),
        external_source: external_source_summary(record),
        skill_enabled: Some(enabled),
        skill_surface: Some(surface),
        command_trigger: is_root
            .then(|| configured_external_command_trigger(record, override_config)),
        argument_hint: is_root.then(|| record.argument_hint.clone()).flatten(),
        created_at: updated_at,
        updated_at,
        has_summary: !summary.trim().is_empty(),
        has_body_content: true,
        byte_size: std::fs::metadata(&file_path).ok().map(|meta| meta.len()),
        lexical_search_enabled: Some(false),
        semantic_search_enabled: Some(false),
        summary: (!summary.trim().is_empty()).then_some(summary),
    }
}

/// All bundled markdown documents as list items.
pub(crate) fn external_to_list_items(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> Vec<knowledge_store::KnowledgeListItem> {
    list_external_document_rel_paths(record)
        .into_iter()
        .map(|doc_rel_path| external_to_list_item_for(record, &doc_rel_path, override_config))
        .collect()
}

pub(crate) fn external_to_document_for(
    record: &ExternalSkillRecord,
    doc_rel_path: &str,
    override_config: Option<&SkillConfig>,
) -> Option<KnowledgeDocument> {
    let is_root = external_doc_is_root(doc_rel_path);
    let file_path = record.root.join(doc_rel_path);
    let raw = std::fs::read_to_string(&file_path).ok()?;
    let body = match split_optional_frontmatter::<ExternalSkillFrontmatter>(strip_utf8_bom(&raw)) {
        Ok((_, body)) => body,
        Err(_) => strip_utf8_bom(&raw).to_string(),
    };
    let enabled = is_root && configured_external_skill_enabled(override_config);
    let surface = if is_root {
        configured_external_skill_surface(record, override_config)
    } else {
        SkillSurface::Command
    };
    let summary = if is_root {
        configured_external_summary(record, override_config)
    } else {
        String::new()
    };
    let updated_at = get_updated_at(&file_path).max(record.updated_at);
    Some(KnowledgeDocument {
        id: external_document_id_for(record, doc_rel_path),
        doc_type: KnowledgeType::Skill,
        path: external_document_virtual_path(record, doc_rel_path),
        title: external_document_title(record, doc_rel_path),
        inject_mode: if is_root {
            configured_external_inject_mode(override_config)
        } else {
            KnowledgeInjectMode::None
        },
        inherit_inject_mode: false,
        inject_mode_source: Default::default(),
        summary_enabled: is_root,
        command_enabled: is_root && configured_external_command_enabled(record, override_config),
        read_only: true,
        ai_maintained: false,
        storage_source: external_storage_source(record),
        inherit_ai_config: false,
        ai_config_source: Default::default(),
        explicit_maintenance_rules: false,
        external_source: external_source_summary(record),
        skill_enabled: Some(enabled),
        skill_surface: Some(surface),
        command_trigger: is_root
            .then(|| configured_external_command_trigger(record, override_config)),
        argument_hint: is_root.then(|| record.argument_hint.clone()).flatten(),
        tools: if is_root {
            external_allowed_tool_activation_names(record.extra_metadata.as_ref())
        } else {
            Vec::new()
        },
        summary: (!summary.trim().is_empty()).then_some(summary),
        body,
        maintenance_rules: None,
        created_at: updated_at,
        updated_at,
    })
}

/// All bundled markdown documents.
pub(crate) fn external_to_documents(
    record: &ExternalSkillRecord,
    override_config: Option<&SkillConfig>,
) -> Vec<KnowledgeDocument> {
    list_external_document_rel_paths(record)
        .into_iter()
        .filter_map(|doc_rel_path| external_to_document_for(record, &doc_rel_path, override_config))
        .collect()
}

/// Read the raw `SKILL.md` for full-content injection when the skill is
/// explicitly selected (slash command or picker). Returned verbatim: the
/// generic frontmatter is useful context for the model.
pub(crate) fn read_external_skill_manifest(record: &ExternalSkillRecord) -> Result<String, String> {
    std::fs::read_to_string(record.skill_md_path())
        .map(|raw| strip_utf8_bom(&raw).to_string())
        .map_err(|e| format!("Failed to read external skill SKILL.md: {}", e))
}

/// Directory config record for paths inside the external namespace: the
/// skill directory itself plus the synthetic `external` and
/// `external/<provider>` grouping levels. `None` when the path does not name
/// an existing external skill or group.
pub(crate) fn read_external_skill_directory(
    records: &[ExternalSkillRecord],
    virtual_path: &str,
) -> Option<knowledge_store::KnowledgeDirectoryConfigRecord> {
    let normalized = normalize_external_virtual_path(virtual_path);
    if let Some(record) = external_record_for_virtual_path(records, virtual_path) {
        return Some(super::skill::read_only_skill_directory_config_record(
            record.dir_name(),
            record.updated_at,
            external_source_summary(&record).into_iter().collect(),
        ));
    }
    // Subdirectories inside a skill (references/, docs/, ...) resolve when
    // they exist on disk; traversal-unsafe paths never reach the filesystem.
    for record in records {
        let Some(rest) = normalized.strip_prefix(&format!("{}/", record.dir_name())) else {
            continue;
        };
        let Ok(rel_path) = super::skill::normalize_package_rel_path(rest) else {
            continue;
        };
        if record.root.join(&rel_path).is_dir() {
            return Some(super::skill::read_only_skill_directory_config_record(
                normalized,
                record.updated_at,
                external_source_summary(record).into_iter().collect(),
            ));
        }
        return None;
    }
    let is_group = normalized == EXTERNAL_SKILL_VIRTUAL_ROOT
        || records
            .iter()
            .any(|record| record.dir_name().starts_with(&format!("{}/", normalized)));
    if !is_group {
        return None;
    }
    let updated_at = records
        .iter()
        .filter(|record| {
            normalized == EXTERNAL_SKILL_VIRTUAL_ROOT
                || record.dir_name().starts_with(&format!("{}/", normalized))
        })
        .map(|record| record.updated_at)
        .max()
        .unwrap_or_default();
    Some(super::skill::read_only_skill_directory_config_record(
        normalized,
        updated_at,
        Vec::new(),
    ))
}

// ── Agent-facing lookups ─────────────────────────────────────

/// Absolute on-disk root of the external skill owning the virtual path, for
/// annotating knowledge_read output. `None` when the path is not an existing
/// external skill document.
pub(crate) fn external_skill_origin_root_for_virtual_path(
    working_dir: &str,
    virtual_path: &str,
) -> Option<String> {
    if !external_skill_virtual_path_in_namespace(virtual_path) {
        return None;
    }
    let records = list_external_skills_cached(working_dir);
    external_record_and_doc_rel_path_for_virtual_path(&records, virtual_path)
        .map(|(record, _)| record.root.to_string_lossy().replace('\\', "/"))
}

/// Reload an external skill by its `external/<provider>/<slug>` dir name (a
/// bare slug resolves when unambiguous). Forces a fresh scan so frontmatter
/// edits and newly installed skills are picked up, making `skill_reload` the
/// model-side rescan entry point.
/// Resolve a skill_reload name against the external record set: the full
/// `external/<provider>/<slug>` dir name, or a bare slug when unambiguous.
fn resolve_external_record_by_name(
    records: &[ExternalSkillRecord],
    name: &str,
    expected_source: Option<&str>,
) -> Result<ExternalSkillRecord, String> {
    let normalized = name.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    let normalized = normalized.strip_prefix("skill/").unwrap_or(normalized);

    let record = if let Some(record) = external_record_for_dir_name(records, normalized) {
        Some(record)
    } else if !normalized.contains('/') {
        let matches: Vec<&ExternalSkillRecord> = records
            .iter()
            .filter(|record| record.slug == normalized)
            .collect();
        match matches.len() {
            0 => None,
            1 => Some(matches[0].clone()),
            _ => {
                return Err(format!(
                    "External skill name '{}' is ambiguous; use the full dir name: {}",
                    normalized,
                    matches
                        .iter()
                        .map(|record| record.dir_name())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    } else {
        None
    };
    let Some(record) = record else {
        return Err(format!("External skill not found: {}", normalized));
    };
    if let Some(expected) = expected_source {
        if record.scope.source() != expected {
            return Err(format!(
                "External skill '{}' has source '{}', not '{}'",
                record.dir_name(),
                record.scope.source(),
                expected
            ));
        }
    }
    Ok(record)
}

pub(crate) fn reload_external_skill_manifest(
    working_dir: &str,
    name: &str,
    expected_source: Option<&str>,
) -> Result<SkillManifest, String> {
    invalidate_external_skill_cache();
    let records = list_external_skills_cached(working_dir);
    let record = resolve_external_record_by_name(&records, name, expected_source)?;
    let configs = super::knowledge::load_skill_config(working_dir);
    let config = super::skill::lookup_skill_config_override(
        &configs,
        record.scope.source(),
        &record.dir_name(),
    );
    Ok(build_external_skill_manifest(&record, config))
}

// ── Commands ─────────────────────────────────────────────────

#[tauri::command]
pub async fn refresh_external_skills(
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<(), AppError> {
    invalidate_external_skill_cache();
    let working_dir = workspace.path.read().await.clone();
    reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        "refresh_external_skills",
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, slug: &str, frontmatter: &str, body: &str) -> PathBuf {
        let skill_dir = dir.join(slug);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\n{}\n---\n\n{}", frontmatter, body),
        )
        .unwrap();
        skill_dir
    }

    fn scan_root(
        provider: ExternalSkillProvider,
        scope: ExternalSkillScope,
        path: &Path,
    ) -> ExternalSkillScanRoot {
        ExternalSkillScanRoot {
            provider,
            scope,
            path: path.to_path_buf(),
            slug_prefix: String::new(),
        }
    }

    #[test]
    fn scan_parses_generic_frontmatter_and_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(
            &root,
            "grill-me",
            "name: grill-me\nversion: 1.2.0\ndescription: |\n  Interrogate a plan before coding.\n  Use before large changes.\nlicense: MIT\nallowed-tools:\n  - Read\n  - Bash\nmetadata:\n  requires:\n    bins: [\"jq\"]\nargument-hint: <plan>\n",
            "# Grill Me\n\nBody.",
        );

        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            &root,
        )]);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.slug, "grill-me");
        assert_eq!(record.name, "grill-me");
        assert_eq!(record.version.as_deref(), Some("1.2.0"));
        assert_eq!(record.argument_hint.as_deref(), Some("<plan>"));
        assert!(record
            .description
            .starts_with("Interrogate a plan before coding."));
        assert!(!record.disable_model_invocation);
        assert!(record.user_invocable);
        assert_eq!(record.dir_name(), "external/claude/grill-me");
        assert_eq!(record.virtual_path(), "external/claude/grill-me/SKILL.md");

        // Unconsumed frontmatter fields surface verbatim for UI display, and
        // consumed fields stay out of the extra map.
        let extra = record.extra_metadata.as_ref().expect("extra metadata");
        assert_eq!(extra.get("license"), Some(&serde_json::json!("MIT")));
        assert_eq!(
            extra.get("allowed-tools"),
            Some(&serde_json::json!(["Read", "Bash"]))
        );
        assert_eq!(
            extra
                .get("metadata")
                .and_then(|value| value.pointer("/requires/bins/0")),
            Some(&serde_json::json!("jq"))
        );
        assert!(extra.get("name").is_none());
        assert!(extra.get("description").is_none());
        assert!(extra.get("argument-hint").is_none());

        let manifest = build_external_skill_manifest(record, None);
        assert_eq!(
            manifest
                .extra_metadata
                .as_ref()
                .and_then(|value| value.get("license")),
            Some(&serde_json::json!("MIT"))
        );
    }

    #[test]
    fn scan_survives_bom_crlf_and_broken_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();

        let bom_dir = root.join("bom-skill");
        std::fs::create_dir_all(&bom_dir).unwrap();
        std::fs::write(
            bom_dir.join("SKILL.md"),
            "\u{feff}---\r\nname: bom-skill\r\ndescription: handles bom\r\n---\r\n\r\nBody",
        )
        .unwrap();

        let broken_dir = root.join("broken");
        std::fs::create_dir_all(&broken_dir).unwrap();
        std::fs::write(
            broken_dir.join("SKILL.md"),
            "---\ndescription: [unclosed\n---\nBody",
        )
        .unwrap();

        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Agents,
            ExternalSkillScope::User,
            &root,
        )]);
        assert_eq!(records.len(), 2);
        let bom = records
            .iter()
            .find(|record| record.slug == "bom-skill")
            .unwrap();
        assert_eq!(bom.description, "handles bom");
        // Broken frontmatter still lists the skill, without metadata.
        let broken = records
            .iter()
            .find(|record| record.slug == "broken")
            .unwrap();
        assert_eq!(broken.name, "broken");
        assert_eq!(broken.description, "");
    }

    #[test]
    fn scan_skips_dirs_without_skill_md_and_hidden_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(root.join("not-a-skill")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden").join("SKILL.md"), "body").unwrap();
        std::fs::write(root.join("loose.md"), "body").unwrap();

        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            &root,
        )]);
        assert!(records.is_empty());
    }

    #[test]
    fn project_scope_shadows_user_scope_for_same_identity() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let user_root = temp.path().join("user");
        write_skill(&project_root, "dup", "description: project copy", "P");
        write_skill(&user_root, "dup", "description: user copy", "U");

        let records = scan_external_skills_from_roots(&[
            scan_root(
                ExternalSkillProvider::Claude,
                ExternalSkillScope::Project,
                &project_root,
            ),
            scan_root(
                ExternalSkillProvider::Claude,
                ExternalSkillScope::User,
                &user_root,
            ),
        ]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].scope, ExternalSkillScope::Project);
        assert_eq!(records[0].description, "project copy");
    }

    #[cfg(windows)]
    #[test]
    fn symlinked_entity_dedupes_and_reattributes_to_canonical_root() {
        let temp = tempfile::tempdir().unwrap();
        let agents_root = temp.path().join("agents-skills");
        let claude_root = temp.path().join("claude-skills");
        std::fs::create_dir_all(&claude_root).unwrap();
        let entity = write_skill(&agents_root, "linked", "description: entity", "Body");
        // Junctions don't need special privileges on Windows.
        let link = claude_root.join("linked");
        if std::os::windows::fs::symlink_dir(&entity, &link).is_err() {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&entity)
                .status();
            if !status.map(|s| s.success()).unwrap_or(false) {
                return; // Environment cannot create links; skip.
            }
        }

        let records = scan_external_skills_from_roots(&[
            scan_root(
                ExternalSkillProvider::Claude,
                ExternalSkillScope::User,
                &claude_root,
            ),
            scan_root(
                ExternalSkillProvider::Agents,
                ExternalSkillScope::User,
                &agents_root,
            ),
        ]);
        assert_eq!(records.len(), 1);
        // Re-attributed to the canonical entity root (.agents), not the link.
        assert_eq!(records[0].provider, ExternalSkillProvider::Agents);
        assert_eq!(records[0].dir_name(), "external/agents/linked");
    }

    #[test]
    fn default_surface_honors_author_declaration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(
            &root,
            "command-only",
            "description: c\ndisable-model-invocation: true",
            "B",
        );
        write_skill(
            &root,
            "auto-only",
            "description: a\nuser-invocable: false",
            "B",
        );
        write_skill(&root, "both", "description: b", "B");

        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            &root,
        )]);
        let surface = |slug: &str| {
            let record = records.iter().find(|r| r.slug == slug).unwrap();
            configured_external_skill_surface(record, None)
        };
        assert_eq!(surface("command-only"), SkillSurface::Command);
        assert_eq!(surface("auto-only"), SkillSurface::Auto);
        assert_eq!(surface("both"), SkillSurface::Both);
    }

    #[test]
    fn external_skills_are_disabled_by_default_and_config_enables() {
        assert!(!configured_external_skill_enabled(None));
        let enabled = SkillConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(configured_external_skill_enabled(Some(&enabled)));
    }

    #[test]
    fn virtual_path_namespace_and_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(&root, "grill-me", "description: d", "Body");
        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            &root,
        )]);

        assert!(external_skill_virtual_path_in_namespace(
            "external/claude/grill-me/SKILL.md"
        ));
        assert!(external_skill_virtual_path_in_namespace(
            "skill/external/claude/grill-me"
        ));
        assert!(!external_skill_virtual_path_in_namespace("externals/x"));
        assert!(!external_skill_virtual_path_in_namespace("view/SKILL.md"));

        assert!(
            external_record_for_virtual_path(&records, "external/claude/grill-me/SKILL.md")
                .is_some()
        );
        assert!(external_record_for_virtual_path(&records, "external/claude/grill-me").is_some());
        assert!(
            external_record_for_virtual_path(&records, "skill/external/claude/grill-me").is_some()
        );
        assert!(external_record_for_virtual_path(&records, "external/claude/other").is_none());
        assert!(
            external_record_for_virtual_path(&records, "external/claude/grill-me/references/x.md")
                .is_none()
        );

        assert!(external_record_for_dir_name(&records, "external/claude/grill-me").is_some());
        assert!(external_record_for_dir_name(&records, "external/agents/grill-me").is_none());
    }

    #[test]
    fn list_item_and_document_reflect_enablement() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(
            &root,
            "grill-me",
            "name: grill-me\ndescription: interrogate plans",
            "# Grill Me\n\nBody text.",
        );
        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            &root,
        )]);
        let record = &records[0];

        let disabled_item =
            external_to_list_item_for(record, EXTERNAL_SKILL_ROOT_DOC_FILE_NAME, None);
        assert_eq!(disabled_item.path, "external/claude/grill-me/SKILL.md");
        assert_eq!(disabled_item.skill_enabled, Some(false));
        assert!(!disabled_item.command_enabled);
        assert!(disabled_item.read_only);
        assert_eq!(
            disabled_item
                .external_source
                .as_ref()
                .and_then(|source| source.locator.clone())
                .as_deref(),
            Some("external://user/claude/grill-me")
        );
        assert!(!configured_external_model_recall_enabled(record, None));

        let enabled_config = SkillConfig {
            enabled: true,
            surface: SkillSurface::Both,
            ..Default::default()
        };
        let enabled_item = external_to_list_item_for(
            record,
            EXTERNAL_SKILL_ROOT_DOC_FILE_NAME,
            Some(&enabled_config),
        );
        assert_eq!(enabled_item.skill_enabled, Some(true));
        assert!(enabled_item.command_enabled);
        assert_eq!(enabled_item.command_trigger.as_deref(), Some("/grill-me"));
        assert!(configured_external_model_recall_enabled(
            record,
            Some(&enabled_config)
        ));

        // Enabled with an auto surface but injectMode none: the auto channel
        // is off, so model recall is gated even though the surface allows it.
        let inject_none_config = SkillConfig {
            enabled: true,
            surface: SkillSurface::Both,
            inject_mode: Some(KnowledgeInjectMode::None),
            ..Default::default()
        };
        assert!(!configured_external_model_recall_enabled(
            record,
            Some(&inject_none_config)
        ));

        let document = external_to_document_for(
            record,
            EXTERNAL_SKILL_ROOT_DOC_FILE_NAME,
            Some(&enabled_config),
        )
        .unwrap();
        assert!(document.body.contains("Body text."));
        assert!(!document.body.contains("description:"));
        assert!(document.read_only);
        assert!(document.tools.is_empty());
    }

    fn write_bundled_file(record_root: &Path, rel_path: &str, content: &str) {
        let path = record_root.join(rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn single_record(root: &Path) -> ExternalSkillRecord {
        let records = scan_external_skills_from_roots(&[scan_root(
            ExternalSkillProvider::Claude,
            ExternalSkillScope::User,
            root,
        )]);
        assert_eq!(records.len(), 1);
        records.into_iter().next().unwrap()
    }

    #[test]
    fn bundled_documents_are_listed_and_resolvable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let skill_dir = write_skill(&root, "grill-me", "description: d", "Body");
        write_bundled_file(&skill_dir, "references/details.md", "# Details");
        write_bundled_file(&skill_dir, "scripts/run.py", "print('x')");
        write_bundled_file(&skill_dir, ".hidden.md", "secret");
        let record = single_record(&root);

        let rel_paths = list_external_document_rel_paths(&record);
        assert!(rel_paths.contains(&"SKILL.md".to_string()));
        assert!(rel_paths.contains(&"references/details.md".to_string()));
        // Scripts are reachable through the disk root, not the knowledge tree.
        assert!(!rel_paths.iter().any(|path| path.ends_with("run.py")));
        assert!(!rel_paths.iter().any(|path| path.contains(".hidden")));

        let items = external_to_list_items(&record, None);
        let deep = items
            .iter()
            .find(|item| item.path == "external/claude/grill-me/references/details.md")
            .expect("deep item");
        assert_eq!(deep.inject_mode, KnowledgeInjectMode::None);
        assert_eq!(deep.skill_enabled, Some(false));
        assert!(!deep.command_enabled);
        assert!(deep.read_only);
        let root_item = items
            .iter()
            .find(|item| item.path == "external/claude/grill-me/SKILL.md")
            .expect("root item");
        assert_ne!(root_item.id, deep.id);

        let records = vec![record.clone()];
        let (_, rel) = external_record_and_doc_rel_path_for_virtual_path(
            &records,
            "skill/external/claude/grill-me/references/details.md",
        )
        .expect("deep path resolves");
        assert_eq!(rel, "references/details.md");
        let (_, root_rel) = external_record_and_doc_rel_path_for_virtual_path(
            &records,
            "external/claude/grill-me",
        )
        .expect("dir resolves to root doc");
        assert_eq!(root_rel, "SKILL.md");

        // Traversal and non-markdown paths never resolve.
        assert!(external_record_and_doc_rel_path_for_virtual_path(
            &records,
            "external/claude/grill-me/../grill-me/SKILL.md",
        )
        .is_none());
        assert!(external_record_and_doc_rel_path_for_virtual_path(
            &records,
            "external/claude/grill-me/scripts/run.py",
        )
        .is_none());

        let deep_document =
            external_to_document_for(&record, "references/details.md", None).expect("document");
        assert!(deep_document.body.contains("# Details"));
        assert_eq!(deep_document.inject_mode, KnowledgeInjectMode::None);
        assert!(deep_document.tools.is_empty());
    }

    #[test]
    fn deep_subdirectories_resolve_as_read_only_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let skill_dir = write_skill(&root, "grill-me", "description: d", "Body");
        write_bundled_file(&skill_dir, "references/details.md", "# Details");
        let record = single_record(&root);
        let records = vec![record];

        let directory =
            read_external_skill_directory(&records, "external/claude/grill-me/references")
                .expect("references directory resolves");
        assert!(directory.read_only);
        assert!(
            read_external_skill_directory(&records, "external/claude/grill-me/missing").is_none()
        );
        assert!(read_external_skill_directory(
            &records,
            "external/claude/grill-me/../grill-me/references"
        )
        .is_none());
    }

    #[test]
    fn allowed_tools_map_to_locus_activation_names() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        write_skill(
            &root,
            "tooled",
            "description: d\nallowed-tools:\n  - Read\n  - Bash(git:*)\n  - WebFetch\n  - TodoWrite\n  - Task\n  - mcp__server__thing\n",
            "Body",
        );
        let record = single_record(&root);

        let manifest = build_external_skill_manifest(&record, None);
        assert_eq!(
            manifest.tools,
            vec![
                "read".to_string(),
                "bash".to_string(),
                "web_fetch".to_string(),
                "todowrite".to_string(),
            ]
        );
        // The raw declaration stays visible in the UI metadata.
        assert!(record
            .extra_metadata
            .as_ref()
            .and_then(|extra| extra.get("allowed-tools"))
            .is_some());

        let document =
            external_to_document_for(&record, EXTERNAL_SKILL_ROOT_DOC_FILE_NAME, None).unwrap();
        assert_eq!(document.tools, manifest.tools);

        // Comma-separated string form is accepted too.
        let string_form = serde_json::json!({ "allowed-tools": "Edit, Grep, Nope" });
        assert_eq!(
            external_allowed_tool_activation_names(Some(&string_form)),
            vec!["edit".to_string(), "grep".to_string()]
        );
        assert!(external_allowed_tool_activation_names(None).is_empty());
    }

    #[test]
    fn claude_plugin_skill_roots_discovered_with_prefixed_slugs() {
        let temp = tempfile::tempdir().unwrap();
        let plugins = temp.path().join("plugins");
        let cache_skills = plugins
            .join("cache")
            .join("anthropics")
            .join("code-toolkit")
            .join("skills");
        write_skill(&cache_skills, "docx", "description: word docs", "Body");
        let repo_skills = plugins
            .join("repos")
            .join("owner--repo")
            .join("plugins")
            .join("deploy-kit")
            .join("skills");
        write_skill(&repo_skills, "release", "description: releases", "Body");
        // Ignored container names never contribute roots.
        write_skill(
            &plugins.join("cache").join(".git").join("skills"),
            "ghost",
            "description: g",
            "Body",
        );

        let roots = discover_claude_plugin_skill_roots(&plugins);
        assert_eq!(roots.len(), 2);
        assert!(roots
            .iter()
            .all(|root| root.provider == ExternalSkillProvider::ClaudePlugin));

        let records = scan_external_skills_from_roots(&roots);
        let dir_names: Vec<String> = records.iter().map(|record| record.dir_name()).collect();
        assert_eq!(
            dir_names,
            vec![
                "external/claude-plugin/code-toolkit--docx".to_string(),
                "external/claude-plugin/deploy-kit--release".to_string(),
            ]
        );
        assert_eq!(
            records[0].virtual_path(),
            "external/claude-plugin/code-toolkit--docx/SKILL.md"
        );
    }

    #[test]
    fn reload_resolves_full_dir_name_and_unambiguous_slug() {
        let temp = tempfile::tempdir().unwrap();
        let claude_root = temp.path().join("claude");
        let agents_root = temp.path().join("agents");
        write_skill(&claude_root, "grill-me", "description: c", "Body");
        write_skill(&agents_root, "grill-me", "description: a", "Body");
        write_skill(&claude_root, "solo", "description: s", "Body");
        let records = scan_external_skills_from_roots(&[
            scan_root(
                ExternalSkillProvider::Claude,
                ExternalSkillScope::User,
                &claude_root,
            ),
            scan_root(
                ExternalSkillProvider::Agents,
                ExternalSkillScope::User,
                &agents_root,
            ),
        ]);
        assert_eq!(records.len(), 3);

        let by_dir =
            resolve_external_record_by_name(&records, "external/claude/grill-me", None).unwrap();
        assert_eq!(by_dir.provider, ExternalSkillProvider::Claude);
        let with_prefix =
            resolve_external_record_by_name(&records, "skill/external/agents/grill-me", None)
                .unwrap();
        assert_eq!(with_prefix.provider, ExternalSkillProvider::Agents);

        let solo = resolve_external_record_by_name(&records, "solo", None).unwrap();
        assert_eq!(solo.dir_name(), "external/claude/solo");

        let ambiguous = resolve_external_record_by_name(&records, "grill-me", None);
        assert!(ambiguous
            .unwrap_err()
            .contains("ambiguous"));

        let missing = resolve_external_record_by_name(&records, "nope", None);
        assert!(missing.unwrap_err().contains("not found"));

        let wrong_source =
            resolve_external_record_by_name(&records, "external/claude/solo", Some("externalProject"));
        assert!(wrong_source.unwrap_err().contains("externalUser"));
        assert!(resolve_external_record_by_name(
            &records,
            "external/claude/solo",
            Some("externalUser")
        )
        .is_ok());
    }
}
