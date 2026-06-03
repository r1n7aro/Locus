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
    pub root: String,
    pub compatibility: LocusPluginCompatibility,
    pub dependencies: LocusPluginDependencies,
    pub agents: Vec<PluginComponentSummary>,
    pub skills: Vec<PluginComponentSummary>,
    pub views: Vec<PluginComponentSummary>,
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
    pub skills: Vec<RawPluginComponentRef>,
    #[serde(default)]
    pub views: Vec<RawPluginComponentRef>,
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
        _ => Vec::new(),
    };
    if !explicit.is_empty() {
        return explicit;
    }
    match kind {
        "agents" => fallback_component_refs(&plugin.root, "agents", "config.json"),
        "skills" => fallback_component_refs(&plugin.root, "skills", "skill.json"),
        "views" => fallback_component_refs(&plugin.root, "views", "view.json"),
        _ => Vec::new(),
    }
}

fn component_sources_for_kind(working_dir: &str, kind: &str) -> Vec<PluginComponentSource> {
    let mut sources = Vec::new();
    for plugin in installed_plugins(working_dir) {
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

pub fn installed_skill_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "skills")
}

pub fn installed_view_sources(working_dir: &str) -> Vec<PluginComponentSource> {
    component_sources_for_kind(working_dir, "views")
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
    pub fn summary(&self) -> InstalledPluginSummary {
        InstalledPluginSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            scope: self.scope,
            root: self.root.display().to_string().replace('\\', "/"),
            compatibility: self.manifest.compatibility.clone(),
            dependencies: self.manifest.dependencies.clone(),
            agents: component_refs_for_kind(self, "agents")
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
        }
    }
}

pub fn list_installed_plugin_summaries(working_dir: &str) -> Vec<InstalledPluginSummary> {
    installed_plugins(working_dir)
        .into_iter()
        .map(|plugin| plugin.summary())
        .collect()
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
            Ok(plugin.summary())
        } else {
            extract_plugin_archive(&source_path, &staging_root)?;
            let source_root = locate_plugin_root(&staging_root)?;
            let manifest = read_manifest(&source_root)?;
            let target_root = target_parent.join(&manifest.id);
            ensure_child_path(&target_parent, &target_root)?;
            let normalized_staging =
                target_parent.join(format!(".install-{}", uuid::Uuid::new_v4()));
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
            Ok(plugin.summary())
        }
    })();

    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
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
    if target_root.exists() {
        fs::remove_dir_all(target_root).map_err(|e| {
            format!(
                "Failed to replace existing plugin '{}': {}",
                target_root.display(),
                e
            )
        })?;
    }
    fs::rename(staging_root, target_root).map_err(|e| {
        format!(
            "Failed to move plugin into {}: {}",
            target_root.display(),
            e
        )
    })
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
    Ok(plugin_id)
}

pub fn plugin_component_sources_by_plugin(working_dir: &str, plugin_id: &str) -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    for source in installed_agent_sources(working_dir)
        .into_iter()
        .chain(installed_skill_sources(working_dir))
        .chain(installed_view_sources(working_dir))
    {
        if source.plugin_id == plugin_id {
            sources.insert(source.scope.component_source().to_string());
        }
    }
    sources
}
