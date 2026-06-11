use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

pub const PLUGIN_MANIFEST_FILE_NAME: &str = "locus.plugin.json";
pub const PROJECT_PLUGINS_RELATIVE: &str = "Locus/plugins";
pub const APP_PLUGINS_DIR_NAME: &str = "plugins";

const PLUGIN_ARCHIVE_MAX_ENTRIES: usize = 20_000;
const PLUGIN_ARCHIVE_MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
const PLUGIN_STATE_FILE_NAME: &str = "plugin_state.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum PluginInstallScope {
    App,
    Project,
}

impl PluginInstallScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Project => "project",
        }
    }

    pub fn component_source(self) -> &'static str {
        match self {
            Self::App => "pluginApp",
            Self::Project => "pluginProject",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: PluginInstallScope,
    pub enabled: bool,
    pub root: String,
    pub compatibility: LocusPluginCompatibility,
    pub dependencies: LocusPluginDependencies,
    pub agents: Vec<PluginComponentSummary>,
    pub rules: Vec<PluginComponentSummary>,
    pub skills: Vec<PluginComponentSummary>,
    pub views: Vec<PluginComponentSummary>,
    pub drawers: Vec<PluginComponentSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginComponentSummary {
    pub id: Option<String>,
    pub path: String,
    pub root: String,
}

#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: PluginInstallScope,
    pub root: PathBuf,
    pub manifest: LocusPluginManifest,
}

#[derive(Debug, Clone)]
pub struct PluginComponentSource {
    pub plugin_id: String,
    pub plugin_name: String,
    pub plugin_version: String,
    pub scope: PluginInstallScope,
    pub id: Option<String>,
    pub root: PathBuf,
    pub rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocusPluginCompatibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_independent: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocusPluginDependencies {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project: Vec<LocusPluginProjectDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocusPluginProjectDependency {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocusPluginManifest {
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub components: LocusPluginComponents,
    #[serde(default)]
    pub compatibility: LocusPluginCompatibility,
    #[serde(default)]
    pub dependencies: LocusPluginDependencies,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocusPluginComponents {
    #[serde(default)]
    pub agents: Vec<RawPluginComponentRef>,
    #[serde(default)]
    pub rules: Vec<RawPluginComponentRef>,
    #[serde(default)]
    pub skills: Vec<RawPluginComponentRef>,
    #[serde(default)]
    pub views: Vec<RawPluginComponentRef>,
    #[serde(default)]
    pub drawers: Vec<RawPluginComponentRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawPluginComponentRef {
    Path(String),
    Object {
        #[serde(default)]
        id: Option<String>,
        path: String,
    },
}

#[derive(Debug, Clone)]
pub struct PluginComponentRef {
    pub id: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginStateEntry {
    #[serde(default = "default_plugin_enabled")]
    enabled: bool,
}

fn default_plugin_enabled() -> bool {
    true
}

type PluginStateConfig = std::collections::BTreeMap<String, PluginStateEntry>;

impl RawPluginComponentRef {
    fn normalize(&self) -> PluginComponentRef {
        match self {
            Self::Path(path) => PluginComponentRef {
                id: None,
                path: path.clone(),
            },
            Self::Object { id, path } => PluginComponentRef {
                id: id.clone(),
                path: path.clone(),
            },
        }
    }
}

pub(crate) fn normalize_plugin_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
        || id.ends_with('.')
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("Invalid plugin id".to_string());
    }
    Ok(id.to_string())
}

fn normalize_component_rel_path(value: &str) -> Result<String, String> {
    let trimmed = value.trim().replace('\\', "/");
    let trimmed = trimmed.trim_matches('/');
    if trimmed.is_empty() {
        return Err("Plugin component path is empty".to_string());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(format!("Plugin component path must be relative: {}", value));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("Plugin component path is not UTF-8: {}", value))?;
                if part.is_empty() {
                    return Err(format!("Invalid plugin component path: {}", value));
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "Plugin component path escapes plugin root: {}",
                    value
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err("Plugin component path is empty".to_string());
    }
    Ok(parts.join("/"))
}

fn safe_component_root(plugin_root: &Path, value: &str) -> Result<(PathBuf, String), String> {
    let rel_path = normalize_component_rel_path(value)?;
    let mut root = plugin_root.to_path_buf();
    for segment in rel_path.split('/') {
        root.push(segment);
    }
    Ok((root, rel_path))
}

pub fn app_plugins_dir() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(APP_PLUGINS_DIR_NAME))
}

pub fn project_plugins_dir(working_dir: &str) -> Result<PathBuf, String> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return Err("No working directory selected".to_string());
    }
    Ok(Path::new(trimmed).join(PROJECT_PLUGINS_RELATIVE))
}

fn project_plugin_state_path(working_dir: &str) -> Result<PathBuf, String> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return Err("No working directory selected".to_string());
    }
    Ok(Path::new(trimmed)
        .join("Library")
        .join("Locus")
        .join(PLUGIN_STATE_FILE_NAME))
}

fn plugin_state_path_for_scope(
    working_dir: &str,
    scope: PluginInstallScope,
) -> Result<PathBuf, String> {
    match scope {
        PluginInstallScope::App => {
            Ok(crate::commands::persistent_config_dir()?.join(PLUGIN_STATE_FILE_NAME))
        }
        PluginInstallScope::Project => project_plugin_state_path(working_dir),
    }
}

fn load_plugin_state_for_scope(working_dir: &str, scope: PluginInstallScope) -> PluginStateConfig {
    let Ok(path) = plugin_state_path_for_scope(working_dir, scope) else {
        return PluginStateConfig::new();
    };
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => PluginStateConfig::new(),
    }
}

fn save_plugin_state_for_scope(
    working_dir: &str,
    scope: PluginInstallScope,
    config: &PluginStateConfig,
) -> Result<(), String> {
    let path = plugin_state_path_for_scope(working_dir, scope)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create plugin state directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize plugin state: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to save plugin state: {}", e))?;
    Ok(())
}

pub fn plugin_enabled_for_scope(
    working_dir: &str,
    plugin_id: &str,
    scope: PluginInstallScope,
) -> bool {
    let Ok(plugin_id) = normalize_plugin_id(plugin_id) else {
        return false;
    };
    load_plugin_state_for_scope(working_dir, scope)
        .get(&plugin_id)
        .map(|entry| entry.enabled)
        .unwrap_or(true)
}

fn plugins_dir_for_scope(
    working_dir: &str,
    scope: PluginInstallScope,
    create: bool,
) -> Result<PathBuf, String> {
    let dir = match scope {
        PluginInstallScope::App => app_plugins_dir()?,
        PluginInstallScope::Project => project_plugins_dir(working_dir)?,
    };
    if create {
        fs::create_dir_all(&dir).map_err(|e| {
            format!(
                "Failed to create plugin directory '{}': {}",
                dir.display(),
                e
            )
        })?;
    }
    Ok(dir)
}

fn read_manifest(root: &Path) -> Result<LocusPluginManifest, String> {
    let path = root.join(PLUGIN_MANIFEST_FILE_NAME);
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let mut manifest: LocusPluginManifest = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid plugin manifest {}: {}", path.display(), e))?;
    manifest.id = normalize_plugin_id(&manifest.id)?;
    manifest.name = manifest.name.trim().to_string();
    if manifest.name.is_empty() {
        manifest.name = manifest.id.clone();
    }
    manifest.version = manifest.version.trim().to_string();
    for dependency in &mut manifest.dependencies.project {
        dependency.kind = dependency.kind.trim().to_string();
        if dependency.kind.is_empty() {
            dependency.kind = "custom".to_string();
        }
        dependency.name = dependency.name.trim().to_string();
        dependency.version = dependency
            .version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        dependency.notes = dependency
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    manifest
        .dependencies
        .project
        .retain(|dependency| !dependency.name.is_empty());
    Ok(manifest)
}

fn installed_plugins_in_dir(scope: PluginInstallScope, dir: &Path) -> Vec<InstalledPlugin> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut plugins = Vec::new();
    for entry in entries.flatten() {
        let root = entry.path();
        let Some(dir_name) = root.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if dir_name.starts_with(".install-") || dir_name.starts_with(".backup-") {
            continue;
        }
        if !root.is_dir() || !root.join(PLUGIN_MANIFEST_FILE_NAME).is_file() {
            continue;
        }
        match read_manifest(&root) {
            Ok(manifest) => plugins.push(InstalledPlugin {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                scope,
                root,
                manifest,
            }),
            Err(error) => eprintln!(
                "[Locus] skipped invalid plugin at {}: {}",
                root.display(),
                error
            ),
        }
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id).then(a.scope.cmp(&b.scope)));
    plugins
}

pub fn installed_plugins(working_dir: &str) -> Vec<InstalledPlugin> {
    let mut plugins = Vec::new();
    if let Ok(dir) = app_plugins_dir() {
        plugins.extend(installed_plugins_in_dir(PluginInstallScope::App, &dir));
    }
    if !working_dir.trim().is_empty() {
        if let Ok(dir) = project_plugins_dir(working_dir) {
            plugins.extend(installed_plugins_in_dir(PluginInstallScope::Project, &dir));
        }
    }
    plugins
}

fn fallback_component_refs(
    plugin_root: &Path,
    dir_name: &str,
    manifest_file: &str,
) -> Vec<PluginComponentRef> {
    let root = plugin_root.join(dir_name);
    if !root.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut refs = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() || !path.join(manifest_file).is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            Some(PluginComponentRef {
                id: Some(name.clone()),
                path: format!("{}/{}", dir_name, name),
            })
        })
        .collect::<Vec<_>>();
    refs.sort_by(|a, b| a.path.cmp(&b.path));
    refs
}

fn fallback_rule_refs(plugin_root: &Path) -> Vec<PluginComponentRef> {
    let root = plugin_root.join("rules");
    if !root.is_dir() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut refs = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            Some(PluginComponentRef {
                id: Some(name.trim_end_matches(".md").to_string()),
                path: format!("rules/{}", name),
            })
        })
        .collect::<Vec<_>>();
    refs.sort_by(|a, b| a.path.cmp(&b.path));
    refs
}

fn component_refs_for_kind(plugin: &InstalledPlugin, kind: &str) -> Vec<PluginComponentRef> {
    let explicit = match kind {
        "agents" => plugin
            .manifest
            .components
            .agents
            .iter()
            .map(RawPluginComponentRef::normalize)
            .collect::<Vec<_>>(),
        "skills" => plugin
            .manifest
            .components
            .skills
            .iter()
            .map(RawPluginComponentRef::normalize)
            .collect::<Vec<_>>(),
        "views" => plugin
            .manifest
            .components
            .views
            .iter()
            .map(RawPluginComponentRef::normalize)
            .collect::<Vec<_>>(),
        "rules" => plugin
            .manifest
            .components
            .rules
            .iter()
            .map(RawPluginComponentRef::normalize)
            .collect::<Vec<_>>(),
        "drawers" => plugin
            .manifest
            .components
            .drawers
            .iter()
            .map(RawPluginComponentRef::normalize)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if !explicit.is_empty() {
        return explicit;
    }
    match kind {
        "agents" => fallback_component_refs(&plugin.root, "agents", "config.json"),
        "rules" => fallback_rule_refs(&plugin.root),
        "skills" => fallback_component_refs(&plugin.root, "skills", "skill.json"),
        "views" => fallback_component_refs(&plugin.root, "views", "view.json"),
        "drawers" => fallback_component_refs(&plugin.root, "drawers", PLUGIN_DRAWER_MANIFEST_FILE),
        _ => Vec::new(),
    }
}

fn component_sources_for_kind(working_dir: &str, kind: &str) -> Vec<PluginComponentSource> {
    let mut sources = Vec::new();
    for plugin in installed_plugins(working_dir) {
        if !plugin_enabled_for_scope(working_dir, &plugin.id, plugin.scope) {
            continue;
        }
        for component in component_refs_for_kind(&plugin, kind) {
            match safe_component_root(&plugin.root, &component.path) {
                Ok((root, rel_path)) if root.exists() => sources.push(PluginComponentSource {
                    plugin_id: plugin.id.clone(),
                    plugin_name: plugin.name.clone(),
                    plugin_version: plugin.version.clone(),
                    scope: plugin.scope,
                    id: component.id.clone(),
                    root,
                    rel_path,
                }),
                Ok((root, _)) => eprintln!(
                    "[Locus] skipped missing plugin component {} in plugin {}",
                    root.display(),
                    plugin.id
                ),
                Err(error) => eprintln!(
                    "[Locus] skipped invalid plugin component in plugin {}: {}",
                    plugin.id, error
                ),
            }
        }
    }
    sources.sort_by(|a, b| {
        a.scope
            .cmp(&b.scope)
            .then(a.plugin_id.cmp(&b.plugin_id))
            .then(a.rel_path.cmp(&b.rel_path))
    });
    sources
}

pub fn installed_agent_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "agents")
}

pub fn installed_rule_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "rules")
}

pub fn installed_skill_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "skills")
}

pub fn installed_view_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "views")
}

pub const PLUGIN_DRAWER_MANIFEST_FILE: &str = "drawer.json";
const DRAWER_PACKAGE_MAX_FILES: usize = 64;
const DRAWER_PACKAGE_MAX_FILE_BYTES: u64 = 512 * 1024;
const DRAWER_PACKAGE_DEFAULT_ENTRY: &str = "src/index.ts";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginDrawerManifest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    entry: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDrawerPackageFile {
    pub rel_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDrawerPackage {
    pub plugin_id: String,
    pub plugin_name: String,
    pub scope: String,
    pub id: String,
    pub root: String,
    pub entry: String,
    pub files: Vec<PluginDrawerPackageFile>,
}

/// Collects enabled plugins' inspector drawer packages with their source
/// files so any window can compile and register them locally.
pub fn installed_inspector_drawer_packages(working_dir: &str) -> Vec<PluginDrawerPackage> {
    let mut packages = Vec::new();
    for source in component_sources_for_kind(working_dir, "drawers") {
        match load_drawer_package(&source) {
            Ok(package) => packages.push(package),
            Err(error) => eprintln!(
                "[Locus] skipped plugin drawer {} in plugin {}: {}",
                source.rel_path, source.plugin_id, error
            ),
        }
    }
    packages
}

fn load_drawer_package(source: &PluginComponentSource) -> Result<PluginDrawerPackage, String> {
    let manifest_path = source.root.join(PLUGIN_DRAWER_MANIFEST_FILE);
    let manifest: PluginDrawerManifest = if manifest_path.is_file() {
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("failed to read {}: {}", PLUGIN_DRAWER_MANIFEST_FILE, error))?;
        serde_json::from_str(&raw)
            .map_err(|error| format!("invalid {}: {}", PLUGIN_DRAWER_MANIFEST_FILE, error))?
    } else {
        PluginDrawerManifest::default()
    };

    let entry = normalize_component_rel_path(
        manifest
            .entry
            .as_deref()
            .unwrap_or(DRAWER_PACKAGE_DEFAULT_ENTRY),
    )?;

    let mut files = Vec::new();
    for dir_entry in WalkDir::new(&source.root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !dir_entry.file_type().is_file() {
            continue;
        }
        let path = dir_entry.path();
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "ts" | "js" | "vue" | "css" | "json") {
            continue;
        }
        if dir_entry
            .metadata()
            .map(|metadata| metadata.len() > DRAWER_PACKAGE_MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            eprintln!(
                "[Locus] skipped oversized plugin drawer file {}",
                path.display()
            );
            continue;
        }
        let Ok(rel) = path.strip_prefix(&source.root) else {
            continue;
        };
        let rel_path = rel.to_string_lossy().replace('\\', "/");
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        files.push(PluginDrawerPackageFile { rel_path, content });
        if files.len() >= DRAWER_PACKAGE_MAX_FILES {
            break;
        }
    }
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    if !files.iter().any(|file| file.rel_path == entry) {
        return Err(format!("drawer entry not found: {}", entry));
    }

    let id = manifest
        .id
        .filter(|value| !value.trim().is_empty())
        .or_else(|| source.id.clone())
        .unwrap_or_else(|| {
            source
                .rel_path
                .rsplit('/')
                .next()
                .unwrap_or("drawer")
                .to_string()
        });

    Ok(PluginDrawerPackage {
        plugin_id: source.plugin_id.clone(),
        plugin_name: source.plugin_name.clone(),
        scope: source.scope.as_str().to_string(),
        id,
        root: source.root.display().to_string().replace('\\', "/"),
        entry,
        files,
    })
}

fn component_summary(
    plugin: &InstalledPlugin,
    component: PluginComponentRef,
) -> Option<PluginComponentSummary> {
    let (root, rel_path) = safe_component_root(&plugin.root, &component.path).ok()?;
    Some(PluginComponentSummary {
        id: component.id,
        path: rel_path,
        root: root.display().to_string().replace('\\', "/"),
    })
}

impl InstalledPlugin {
    pub fn summary(&self, working_dir: &str) -> InstalledPluginSummary {
        InstalledPluginSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            scope: self.scope,
            enabled: plugin_enabled_for_scope(working_dir, &self.id, self.scope),
            root: self.root.display().to_string().replace('\\', "/"),
            compatibility: self.manifest.compatibility.clone(),
            dependencies: self.manifest.dependencies.clone(),
            agents: component_refs_for_kind(self, "agents")
                .into_iter()
                .filter_map(|component| component_summary(self, component))
                .collect(),
            rules: component_refs_for_kind(self, "rules")
                .into_iter()
                .filter_map(|component| component_summary(self, component))
                .collect(),
            skills: component_refs_for_kind(self, "skills")
                .into_iter()
                .filter_map(|component| component_summary(self, component))
                .collect(),
            views: component_refs_for_kind(self, "views")
                .into_iter()
                .filter_map(|component| component_summary(self, component))
                .collect(),
            drawers: component_refs_for_kind(self, "drawers")
                .into_iter()
                .filter_map(|component| component_summary(self, component))
                .collect(),
        }
    }
}

pub fn list_installed_plugin_summaries(working_dir: &str) -> Vec<InstalledPluginSummary> {
    installed_plugins(working_dir)
        .into_iter()
        .map(|plugin| plugin.summary(working_dir))
        .collect()
}

pub fn set_plugin_enabled_sync(
    working_dir: &str,
    plugin_id: &str,
    scope: PluginInstallScope,
    enabled: bool,
) -> Result<InstalledPluginSummary, String> {
    let plugin_id = normalize_plugin_id(plugin_id)?;
    let plugin = installed_plugins(working_dir)
        .into_iter()
        .find(|plugin| plugin.id == plugin_id && plugin.scope == scope)
        .ok_or_else(|| format!("Plugin not installed: {}", plugin_id))?;

    let mut config = load_plugin_state_for_scope(working_dir, scope);
    if enabled {
        config.remove(&plugin_id);
    } else {
        config.insert(plugin_id.clone(), PluginStateEntry { enabled });
    }
    save_plugin_state_for_scope(working_dir, scope, &config)?;
    Ok(plugin.summary(working_dir))
}

pub fn inspect_plugin_source_manifest_sync(
    source_path: &str,
) -> Result<LocusPluginManifest, String> {
    let source_path = PathBuf::from(source_path.trim());
    if !source_path.exists() {
        return Err(format!(
            "Plugin source not found: {}",
            source_path.display()
        ));
    }

    if source_path.is_dir() {
        let source_root = locate_plugin_root(&source_path)?;
        return read_manifest(&source_root);
    }

    let staging_root =
        std::env::temp_dir().join(format!("locus-plugin-inspect-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create plugin inspection directory: {}", e))?;
    let result = (|| -> Result<LocusPluginManifest, String> {
        extract_plugin_archive(&source_path, &staging_root)?;
        let source_root = locate_plugin_root(&staging_root)?;
        read_manifest(&source_root)
    })();
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    result
}

fn copy_plugin_dir(source_root: &Path, target_root: &Path) -> Result<(), String> {
    for entry in WalkDir::new(source_root).follow_links(false) {
        let entry = entry.map_err(|e| format!("Failed to read plugin files: {}", e))?;
        let rel_path = entry
            .path()
            .strip_prefix(source_root)
            .map_err(|e| format!("Failed to resolve plugin file path: {}", e))?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        if rel_path
            .components()
            .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        let target = target_root.join(rel_path);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(format!(
                "Plugin install does not support symlinks: {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("Failed to create {}: {}", target.display(), e))?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
            }
            fs::copy(entry.path(), &target).map_err(|e| {
                format!(
                    "Failed to copy plugin file {} to {}: {}",
                    entry.path().display(),
                    target.display(),
                    e
                )
            })?;
        }
    }
    Ok(())
}

fn zip_entry_rel_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let path = Path::new(normalized.trim_start_matches('/'));
    if path.is_absolute() {
        return Err(format!("Archive entry must be relative: {}", name));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Archive entry escapes plugin root: {}", name));
            }
        }
    }
    Ok(out)
}

fn extract_plugin_archive(source_path: &Path, staging_root: &Path) -> Result<(), String> {
    let file = fs::File::open(source_path).map_err(|e| {
        format!(
            "Failed to open plugin archive '{}': {}",
            source_path.display(),
            e
        )
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Invalid plugin archive '{}': {}", source_path.display(), e))?;
    if archive.len() > PLUGIN_ARCHIVE_MAX_ENTRIES {
        return Err(format!(
            "Plugin archive has too many entries: {}",
            archive.len()
        ));
    }
    let mut total_bytes = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Failed to read archive entry {}: {}", index, e))?;
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() {
            return Err(format!(
                "Plugin archive contains a symlink entry: {}",
                entry.name()
            ));
        }
        total_bytes = total_bytes.saturating_add(entry.size());
        if total_bytes > PLUGIN_ARCHIVE_MAX_UNCOMPRESSED_BYTES {
            return Err("Plugin archive is too large".to_string());
        }
        let rel_path = zip_entry_rel_path(entry.name())?;
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        let target = staging_root.join(rel_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        let mut out = fs::File::create(&target)
            .map_err(|e| format!("Failed to create {}: {}", target.display(), e))?;
        let mut buffer = Vec::new();
        entry
            .read_to_end(&mut buffer)
            .map_err(|e| format!("Failed to read archive entry {}: {}", entry.name(), e))?;
        out.write_all(&buffer)
            .map_err(|e| format!("Failed to write {}: {}", target.display(), e))?;
    }
    Ok(())
}

fn locate_plugin_root(root: &Path) -> Result<PathBuf, String> {
    if root.join(PLUGIN_MANIFEST_FILE_NAME).is_file() {
        return Ok(root.to_path_buf());
    }
    let mut candidates = Vec::new();
    for entry in WalkDir::new(root)
        .min_depth(1)
        .max_depth(2)
        .follow_links(false)
    {
        let entry = entry.map_err(|e| format!("Failed to inspect plugin source: {}", e))?;
        if entry.file_type().is_file() && entry.file_name() == PLUGIN_MANIFEST_FILE_NAME {
            if let Some(parent) = entry.path().parent() {
                candidates.push(parent.to_path_buf());
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(format!("{} not found", PLUGIN_MANIFEST_FILE_NAME)),
        _ => Err(format!(
            "Plugin source contains multiple {} files",
            PLUGIN_MANIFEST_FILE_NAME
        )),
    }
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<(), String> {
    let parent = dunce::canonicalize(parent)
        .map_err(|e| format!("Failed to resolve {}: {}", parent.display(), e))?;
    let child = if child.exists() {
        dunce::canonicalize(child)
            .map_err(|e| format!("Failed to resolve {}: {}", child.display(), e))?
    } else {
        child.to_path_buf()
    };
    if child == parent || !child.starts_with(&parent) {
        return Err(format!(
            "Plugin target resolves outside plugin directory: {}",
            child.display()
        ));
    }
    Ok(())
}

pub fn install_plugin_from_path_sync(
    working_dir: &str,
    source_path: &str,
    scope: PluginInstallScope,
) -> Result<InstalledPluginSummary, String> {
    let source_path = PathBuf::from(source_path.trim());
    if !source_path.exists() {
        return Err(format!(
            "Plugin source not found: {}",
            source_path.display()
        ));
    }
    let target_parent = plugins_dir_for_scope(working_dir, scope, true)?;
    let staging_root = target_parent.join(format!(".install-{}", uuid::Uuid::new_v4()));
    let mut cleanup_roots = vec![staging_root.clone()];
    fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create staging plugin directory: {}", e))?;

    let install_result = (|| -> Result<InstalledPluginSummary, String> {
        if source_path.is_dir() {
            let source_root = locate_plugin_root(&source_path)?;
            let manifest = read_manifest(&source_root)?;
            let target_root = target_parent.join(&manifest.id);
            ensure_child_path(&target_parent, &target_root)?;
            copy_plugin_dir(&source_root, &staging_root)?;
            let staged_manifest = read_manifest(&staging_root)?;
            if staged_manifest.id != manifest.id {
                return Err("Staged plugin manifest id changed during install".to_string());
            }
            replace_plugin_dir(&target_parent, &target_root, &staging_root)?;
            let plugin = InstalledPlugin {
                id: staged_manifest.id.clone(),
                name: staged_manifest.name.clone(),
                version: staged_manifest.version.clone(),
                scope,
                root: target_root,
                manifest: staged_manifest,
            };
            Ok(plugin.summary(working_dir))
        } else {
            extract_plugin_archive(&source_path, &staging_root)?;
            let source_root = locate_plugin_root(&staging_root)?;
            let manifest = read_manifest(&source_root)?;
            let target_root = target_parent.join(&manifest.id);
            ensure_child_path(&target_parent, &target_root)?;
            let normalized_staging =
                target_parent.join(format!(".install-{}", uuid::Uuid::new_v4()));
            cleanup_roots.push(normalized_staging.clone());
            fs::create_dir_all(&normalized_staging)
                .map_err(|e| format!("Failed to create plugin staging directory: {}", e))?;
            copy_plugin_dir(&source_root, &normalized_staging)?;
            let staged_manifest = read_manifest(&normalized_staging)?;
            replace_plugin_dir(&target_parent, &target_root, &normalized_staging)?;
            let plugin = InstalledPlugin {
                id: staged_manifest.id.clone(),
                name: staged_manifest.name.clone(),
                version: staged_manifest.version.clone(),
                scope,
                root: target_root,
                manifest: staged_manifest,
            };
            Ok(plugin.summary(working_dir))
        }
    })();

    for cleanup_root in cleanup_roots {
        if cleanup_root.exists() {
            let _ = fs::remove_dir_all(cleanup_root);
        }
    }
    install_result
}

fn replace_plugin_dir(
    parent: &Path,
    target_root: &Path,
    staging_root: &Path,
) -> Result<(), String> {
    ensure_child_path(parent, target_root)?;
    ensure_child_path(parent, staging_root)?;
    if !staging_root.is_dir() {
        return Err(format!(
            "Plugin staging directory is missing: {}",
            staging_root.display()
        ));
    }
    let backup_root = parent.join(format!(
        ".backup-{}-{}",
        target_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("plugin"),
        uuid::Uuid::new_v4()
    ));
    ensure_child_path(parent, &backup_root)?;

    let had_existing = target_root.exists();
    if target_root.exists() {
        fs::rename(target_root, &backup_root).map_err(|e| {
            format!(
                "Failed to move existing plugin '{}' aside: {}",
                target_root.display(),
                e
            )
        })?;
    }

    match fs::rename(staging_root, target_root) {
        Ok(()) => {
            if backup_root.exists() {
                let _ = fs::remove_dir_all(&backup_root);
            }
            Ok(())
        }
        Err(error) => {
            let move_error = format!(
                "Failed to move plugin into {}: {}",
                target_root.display(),
                error
            );
            if target_root.exists() {
                let _ = fs::remove_dir_all(target_root);
                let _ = fs::remove_file(target_root);
            }
            if had_existing {
                fs::rename(&backup_root, target_root).map_err(|rollback_error| {
                    format!(
                        "{}; failed to restore previous plugin from '{}': {}",
                        move_error,
                        backup_root.display(),
                        rollback_error
                    )
                })?;
                return Err(format!("{}; restored previous plugin", move_error));
            }
            Err(move_error)
        }
    }
}

pub fn uninstall_plugin_sync(
    working_dir: &str,
    plugin_id: &str,
    scope: PluginInstallScope,
) -> Result<String, String> {
    let plugin_id = normalize_plugin_id(plugin_id)?;
    let target_parent = plugins_dir_for_scope(working_dir, scope, false)?;
    let target_root = target_parent.join(&plugin_id);
    ensure_child_path(&target_parent, &target_root)?;
    if !target_root.join(PLUGIN_MANIFEST_FILE_NAME).is_file() {
        return Err(format!("Plugin not installed: {}", plugin_id));
    }
    fs::remove_dir_all(&target_root).map_err(|e| {
        format!(
            "Failed to uninstall plugin '{}': {}",
            target_root.display(),
            e
        )
    })?;
    let mut config = load_plugin_state_for_scope(working_dir, scope);
    if config.remove(&plugin_id).is_some() {
        save_plugin_state_for_scope(working_dir, scope, &config)?;
    }
    Ok(plugin_id)
}

pub fn plugin_component_sources_by_plugin(working_dir: &str, plugin_id: &str) -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    for source in installed_agent_sources(working_dir)
        .into_iter()
        .chain(installed_skill_sources(working_dir))
        .chain(installed_view_sources(working_dir))
        .chain(component_sources_for_kind(working_dir, "drawers"))
    {
        if source.plugin_id == plugin_id {
            sources.insert(source.scope.component_source().to_string());
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_plugin_source(root: &Path, id: &str, version: &str, marker: &str) {
        fs::create_dir_all(root).expect("create plugin source");
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "name": id,
            "version": version,
            "components": {
                "agents": [],
                "rules": [],
                "skills": [],
                "views": []
            }
        });
        fs::write(
            root.join(PLUGIN_MANIFEST_FILE_NAME),
            serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write plugin manifest");
        fs::write(root.join("marker.txt"), marker).expect("write marker");
    }

    fn workspace_path(workspace: &TempDir) -> String {
        workspace.path().to_string_lossy().to_string()
    }

    fn project_plugin_parent(workspace: &TempDir) -> PathBuf {
        project_plugins_dir(&workspace_path(workspace)).expect("project plugin dir")
    }

    fn rule_source_count(working_dir: &str, plugin_id: &str) -> usize {
        installed_rule_sources(working_dir)
            .into_iter()
            .filter(|source| source.plugin_id == plugin_id)
            .count()
    }

    fn assert_no_install_artifacts(parent: &Path) {
        if !parent.is_dir() {
            return;
        }
        for entry in fs::read_dir(parent).expect("read plugin parent") {
            let entry = entry.expect("read plugin parent entry");
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with(".install-") && !name.starts_with(".backup-"),
                "left plugin install artifact: {}",
                name
            );
        }
    }

    #[test]
    fn plugin_id_accepts_package_manager_style_names() {
        assert_eq!(
            normalize_plugin_id(" locus-workspace ").expect("valid plugin id"),
            "locus-workspace"
        );
        assert_eq!(
            normalize_plugin_id("asset_browser.tools").expect("valid plugin id"),
            "asset_browser.tools"
        );

        for invalid in [
            "",
            ".hidden",
            "trailing.",
            "bad/name",
            "bad\\name",
            "bad..name",
        ] {
            assert!(
                normalize_plugin_id(invalid).is_err(),
                "expected invalid plugin id: {invalid}"
            );
        }
    }

    #[test]
    fn install_project_plugin_accepts_concise_package_id() {
        let workspace = TempDir::new().expect("workspace");
        let source_parent = TempDir::new().expect("source parent");
        let source = source_parent.path().join("plugin");
        write_plugin_source(&source, "locus-workspace", "0.1.0", "simple");

        let installed = install_plugin_from_path_sync(
            &workspace_path(&workspace),
            &source.to_string_lossy(),
            PluginInstallScope::Project,
        )
        .expect("install plugin");

        assert_eq!(installed.id, "locus-workspace");
        assert_eq!(installed.version, "0.1.0");
        assert!(project_plugin_parent(&workspace)
            .join("locus-workspace")
            .join(PLUGIN_MANIFEST_FILE_NAME)
            .is_file());
    }

    #[test]
    fn install_replaces_existing_project_plugin_and_cleans_artifacts() {
        let workspace = TempDir::new().expect("workspace");
        let source_parent = TempDir::new().expect("source parent");
        let source = source_parent.path().join("plugin");
        write_plugin_source(&source, "com.example.lifecycle", "1.0.0", "v1");

        let installed = install_plugin_from_path_sync(
            &workspace_path(&workspace),
            &source.to_string_lossy(),
            PluginInstallScope::Project,
        )
        .expect("install v1");
        assert_eq!(installed.version, "1.0.0");

        write_plugin_source(&source, "com.example.lifecycle", "2.0.0", "v2");
        let updated = install_plugin_from_path_sync(
            &workspace_path(&workspace),
            &source.to_string_lossy(),
            PluginInstallScope::Project,
        )
        .expect("install v2");

        assert_eq!(updated.version, "2.0.0");
        let target = project_plugin_parent(&workspace).join("com.example.lifecycle");
        assert_eq!(
            fs::read_to_string(target.join("marker.txt")).expect("read marker"),
            "v2"
        );
        assert_no_install_artifacts(&project_plugin_parent(&workspace));
    }

    #[test]
    fn installed_plugin_summary_includes_rule_files() {
        let workspace = TempDir::new().expect("workspace");
        let plugin_root = project_plugin_parent(&workspace).join("com.example.rules");
        write_plugin_source(&plugin_root, "com.example.rules", "1.0.0", "");
        fs::create_dir_all(plugin_root.join("rules")).expect("create rules dir");
        fs::write(
            plugin_root.join("rules").join("risk_control.md"),
            "# Risk Control",
        )
        .expect("write rule");

        let summaries = list_installed_plugin_summaries(&workspace_path(&workspace));
        let summary = summaries
            .iter()
            .find(|plugin| plugin.id == "com.example.rules")
            .expect("plugin summary");

        assert_eq!(summary.rules.len(), 1);
        assert_eq!(summary.rules[0].path, "rules/risk_control.md");
    }

    #[test]
    fn plugin_enabled_state_filters_project_components() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace_path(&workspace);
        let plugin_root = project_plugin_parent(&workspace).join("com.example.toggle");
        write_plugin_source(&plugin_root, "com.example.toggle", "1.0.0", "");
        fs::create_dir_all(plugin_root.join("rules")).expect("create rules dir");
        fs::write(plugin_root.join("rules").join("style.md"), "# Style").expect("write rule");

        let summaries = list_installed_plugin_summaries(&working_dir);
        let summary = summaries
            .iter()
            .find(|plugin| plugin.id == "com.example.toggle")
            .expect("plugin summary");
        assert!(summary.enabled);
        assert_eq!(rule_source_count(&working_dir, "com.example.toggle"), 1);

        let disabled = set_plugin_enabled_sync(
            &working_dir,
            "com.example.toggle",
            PluginInstallScope::Project,
            false,
        )
        .expect("disable plugin");
        assert!(!disabled.enabled);
        assert_eq!(disabled.rules.len(), 1);
        assert_eq!(rule_source_count(&working_dir, "com.example.toggle"), 0);

        let summaries = list_installed_plugin_summaries(&working_dir);
        let summary = summaries
            .iter()
            .find(|plugin| plugin.id == "com.example.toggle")
            .expect("plugin summary");
        assert!(!summary.enabled);
        assert_eq!(summary.rules.len(), 1);

        let enabled = set_plugin_enabled_sync(
            &working_dir,
            "com.example.toggle",
            PluginInstallScope::Project,
            true,
        )
        .expect("enable plugin");
        assert!(enabled.enabled);
        assert_eq!(rule_source_count(&working_dir, "com.example.toggle"), 1);
    }

    #[test]
    fn installed_plugin_scan_skips_install_and_backup_artifacts() {
        let workspace = TempDir::new().expect("workspace");
        let parent = project_plugin_parent(&workspace);
        write_plugin_source(
            &parent.join("com.example.normal"),
            "com.example.normal",
            "1.0.0",
            "",
        );
        write_plugin_source(
            &parent.join(".install-temp"),
            "com.example.temp",
            "1.0.0",
            "",
        );
        write_plugin_source(
            &parent.join(".backup-temp"),
            "com.example.backup",
            "1.0.0",
            "",
        );

        let plugins = installed_plugins_in_dir(PluginInstallScope::Project, &parent);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "com.example.normal");
    }

    #[test]
    fn uninstall_project_plugin_removes_installed_directory() {
        let workspace = TempDir::new().expect("workspace");
        let source_parent = TempDir::new().expect("source parent");
        let source = source_parent.path().join("plugin");
        write_plugin_source(&source, "com.example.uninstall", "1.0.0", "");
        install_plugin_from_path_sync(
            &workspace_path(&workspace),
            &source.to_string_lossy(),
            PluginInstallScope::Project,
        )
        .expect("install plugin");

        let removed = uninstall_plugin_sync(
            &workspace_path(&workspace),
            "com.example.uninstall",
            PluginInstallScope::Project,
        )
        .expect("uninstall plugin");

        assert_eq!(removed, "com.example.uninstall");
        assert!(!project_plugin_parent(&workspace)
            .join("com.example.uninstall")
            .exists());
    }
}
