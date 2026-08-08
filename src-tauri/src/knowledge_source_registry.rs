use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::knowledge_store::{KnowledgeStorageSource, KnowledgeType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnowledgeSourceKind {
    WorkspaceKnowledge,
    AppKnowledge,
    AppSkillPackage,
    PluginSkillPackage,
    ExternalSkill,
    ManagedReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnowledgeSourceMutability {
    Writable,
    ReadOnly,
    Managed,
}

impl KnowledgeSourceMutability {
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Writable)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Writable => "writable",
            Self::ReadOnly => "read-only",
            Self::Managed => "managed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct KnowledgeSource {
    pub source_id: String,
    pub kind: KnowledgeSourceKind,
    pub doc_type: KnowledgeType,
    pub physical_root: PathBuf,
    /// Path prefix relative to the knowledge type. Package sources use their
    /// package/external namespace; ordinary type roots use an empty prefix.
    pub logical_prefix: String,
    pub storage_source: KnowledgeStorageSource,
    pub mutability: KnowledgeSourceMutability,
    pub watch: bool,
    pub priority: u16,
}

#[derive(Debug, Clone)]
pub struct ResolvedKnowledgePath {
    pub source_id: String,
    pub kind: KnowledgeSourceKind,
    pub doc_type: KnowledgeType,
    pub logical_path: String,
    pub physical_path: PathBuf,
    pub display_path: String,
    pub workspace_relative_path: Option<String>,
    pub storage_source: KnowledgeStorageSource,
    pub mutability: KnowledgeSourceMutability,
}

#[derive(Debug, Clone)]
pub struct KnowledgeSourceRegistry {
    workspace_root: Option<PathBuf>,
    sources: Vec<KnowledgeSource>,
    discovery_roots: Vec<PathBuf>,
}

impl KnowledgeSourceRegistry {
    pub fn build(working_dir: &str, app_knowledge_dir: Option<&PathBuf>) -> Self {
        let workspace_root = canonical_workspace_root(working_dir);
        let mut sources = Vec::new();

        if let Some(workspace) = workspace_root.as_ref() {
            let knowledge_root = workspace.join("Locus").join("knowledge");
            for doc_type in KnowledgeType::all() {
                sources.push(KnowledgeSource {
                    source_id: format!("workspace-knowledge:{}", doc_type.as_str()),
                    kind: KnowledgeSourceKind::WorkspaceKnowledge,
                    doc_type,
                    physical_root: normalize_absolute_path(&knowledge_root.join(doc_type.as_str())),
                    logical_prefix: String::new(),
                    storage_source: KnowledgeStorageSource::Project,
                    mutability: KnowledgeSourceMutability::Writable,
                    watch: true,
                    priority: 500,
                });
            }

            sources.push(KnowledgeSource {
                source_id: "managed-reference-materialization".to_string(),
                kind: KnowledgeSourceKind::ManagedReference,
                doc_type: KnowledgeType::Reference,
                physical_root: normalize_absolute_path(
                    &workspace
                        .join("Library")
                        .join("Locus")
                        .join("KnowledgeSources")
                        .join("reference"),
                ),
                logical_prefix: String::new(),
                storage_source: KnowledgeStorageSource::Project,
                mutability: KnowledgeSourceMutability::Managed,
                watch: false,
                priority: 250,
            });
        }

        if let Some(app_root) = app_knowledge_dir {
            let app_root = normalize_absolute_path(app_root);
            for doc_type in KnowledgeType::all() {
                sources.push(KnowledgeSource {
                    source_id: format!("app-knowledge:{}", doc_type.as_str()),
                    kind: KnowledgeSourceKind::AppKnowledge,
                    doc_type,
                    physical_root: normalize_absolute_path(&app_root.join(doc_type.as_str())),
                    logical_prefix: String::new(),
                    storage_source: KnowledgeStorageSource::App,
                    mutability: KnowledgeSourceMutability::ReadOnly,
                    watch: true,
                    priority: 50,
                });
            }
        }

        let writable_app_package_root = crate::commands::persistent_config_dir()
            .ok()
            .map(|path| normalize_absolute_path(&path.join("skills")));
        for package in crate::commands::list_skill_packages_sync_for_working_dir(working_dir) {
            let root = normalize_absolute_path(&package.root);
            let is_plugin = package.plugin_id.is_some();
            let writable = !is_plugin
                && writable_app_package_root
                    .as_ref()
                    .is_some_and(|candidate| path_is_within(&root, candidate));
            sources.push(KnowledgeSource {
                source_id: if is_plugin {
                    format!(
                        "plugin-skill:{}:{}",
                        package.plugin_id.as_deref().unwrap_or("unknown"),
                        package.manifest.id
                    )
                } else {
                    format!("app-skill:{}", package.manifest.id)
                },
                kind: if is_plugin {
                    KnowledgeSourceKind::PluginSkillPackage
                } else {
                    KnowledgeSourceKind::AppSkillPackage
                },
                doc_type: KnowledgeType::Skill,
                physical_root: root,
                logical_prefix: package.manifest.id,
                storage_source: KnowledgeStorageSource::App,
                mutability: if writable {
                    KnowledgeSourceMutability::Writable
                } else {
                    KnowledgeSourceMutability::ReadOnly
                },
                watch: true,
                priority: 300,
            });
        }

        for record in crate::commands::list_external_skills_cached(working_dir).iter() {
            sources.push(KnowledgeSource {
                source_id: format!(
                    "external:{}:{}:{}",
                    record.scope.source(),
                    record.provider.as_str(),
                    record.slug
                ),
                kind: KnowledgeSourceKind::ExternalSkill,
                doc_type: KnowledgeType::Skill,
                physical_root: normalize_absolute_path(&record.root),
                logical_prefix: record.dir_name(),
                storage_source: match record.scope {
                    crate::commands::ExternalSkillScope::Project => KnowledgeStorageSource::Project,
                    crate::commands::ExternalSkillScope::User => KnowledgeStorageSource::App,
                },
                mutability: KnowledgeSourceMutability::ReadOnly,
                watch: true,
                priority: 400,
            });
        }

        dedupe_and_sort_sources(&mut sources);
        let mut discovery_roots = crate::commands::app_skill_package_dirs();
        discovery_roots.extend(crate::commands::external_skill_watch_roots(working_dir));
        discovery_roots.extend(
            sources
                .iter()
                .filter(|source| {
                    matches!(
                        source.kind,
                        KnowledgeSourceKind::AppSkillPackage
                            | KnowledgeSourceKind::PluginSkillPackage
                            | KnowledgeSourceKind::ExternalSkill
                    )
                })
                .filter_map(|source| source.physical_root.parent().map(Path::to_path_buf)),
        );
        dedupe_paths(&mut discovery_roots);
        Self {
            workspace_root,
            sources,
            discovery_roots,
        }
    }

    #[cfg(test)]
    pub fn from_sources_for_test(
        workspace_root: Option<PathBuf>,
        mut sources: Vec<KnowledgeSource>,
    ) -> Self {
        let workspace_root = workspace_root.map(|path| normalize_absolute_path(&path));
        for source in &mut sources {
            source.physical_root = normalize_absolute_path(&source.physical_root);
        }
        dedupe_and_sort_sources(&mut sources);
        Self {
            workspace_root,
            sources,
            discovery_roots: Vec::new(),
        }
    }

    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    pub fn sources(&self) -> &[KnowledgeSource] {
        &self.sources
    }

    pub fn discovery_roots(&self) -> &[PathBuf] {
        &self.discovery_roots
    }

    pub fn watch_roots(&self) -> Vec<(String, PathBuf)> {
        let mut seen = HashSet::new();
        self.sources
            .iter()
            .filter(|source| source.watch)
            .filter_map(|source| {
                let key = path_key(&source.physical_root);
                seen.insert(key)
                    .then(|| (source.source_id.clone(), source.physical_root.clone()))
            })
            .collect()
    }

    pub fn display_path(&self, physical_path: &Path) -> String {
        let physical_path = normalize_absolute_path(physical_path);
        if let Some(workspace) = self.workspace_root.as_ref() {
            if let Ok(relative) = physical_path.strip_prefix(workspace) {
                let value = slash_path(relative);
                if !value.is_empty() {
                    return value;
                }
            }
        }
        slash_path(&physical_path)
    }

    pub fn classify_path(&self, path: &Path) -> Option<ResolvedKnowledgePath> {
        let physical_path = if path.is_absolute() {
            normalize_absolute_path(path)
        } else {
            normalize_absolute_path(&self.workspace_root.as_ref()?.join(path))
        };
        let source = self
            .sources
            .iter()
            .find(|source| path_is_within(&physical_path, &source.physical_root))?;
        let relative = physical_path.strip_prefix(&source.physical_root).ok()?;
        let relative = slash_path(relative);
        let logical_path = join_logical(&source.logical_prefix, &relative);
        Some(self.resolved_from_source(source, logical_path, physical_path))
    }

    pub fn classify_path_string(&self, path: &str) -> Option<ResolvedKnowledgePath> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return None;
        }
        self.classify_path(Path::new(trimmed))
    }

    pub fn resolve_logical(
        &self,
        doc_type: KnowledgeType,
        logical_path: &str,
    ) -> Option<ResolvedKnowledgePath> {
        let logical_path = normalize_logical_path(logical_path)?;
        let mut fallback = None;
        for source in self
            .sources
            .iter()
            .filter(|source| source.doc_type == doc_type)
        {
            let Some(relative) = strip_logical_prefix(&logical_path, &source.logical_prefix) else {
                continue;
            };
            let physical_path = normalize_absolute_path(&source.physical_root.join(relative));
            let resolved =
                self.resolved_from_source(source, logical_path.clone(), physical_path.clone());
            if physical_path.exists() {
                return Some(resolved);
            }
            if fallback.is_none() && source.mutability.is_writable() {
                fallback = Some(resolved);
            }
        }
        fallback
    }

    pub fn resolve_display_path(&self, display_path: &str) -> Option<ResolvedKnowledgePath> {
        self.classify_path_string(display_path)
    }

    pub fn managed_materialization_target(
        &self,
        doc_type: KnowledgeType,
        logical_path: &str,
    ) -> Option<ResolvedKnowledgePath> {
        let logical_path = normalize_logical_path(logical_path)?;
        let source = self.sources.iter().find(|source| {
            source.doc_type == doc_type && source.kind == KnowledgeSourceKind::ManagedReference
        })?;
        let relative = strip_logical_prefix(&logical_path, &source.logical_prefix)?;
        let physical_path = normalize_absolute_path(&source.physical_root.join(relative));
        Some(self.resolved_from_source(source, logical_path, physical_path))
    }

    fn resolved_from_source(
        &self,
        source: &KnowledgeSource,
        logical_path: String,
        physical_path: PathBuf,
    ) -> ResolvedKnowledgePath {
        let workspace_relative_path = self.workspace_root.as_ref().and_then(|workspace| {
            physical_path
                .strip_prefix(workspace)
                .ok()
                .map(slash_path)
                .filter(|value| !value.is_empty())
        });
        let display_path = workspace_relative_path
            .clone()
            .unwrap_or_else(|| slash_path(&physical_path));
        ResolvedKnowledgePath {
            source_id: source.source_id.clone(),
            kind: source.kind,
            doc_type: source.doc_type,
            logical_path,
            physical_path,
            display_path,
            workspace_relative_path,
            storage_source: source.storage_source,
            mutability: source.mutability,
        }
    }
}

fn canonical_workspace_root(working_dir: &str) -> Option<PathBuf> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_absolute_path(Path::new(trimmed)))
}

pub fn normalize_absolute_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if absolute.exists() {
        return dunce::canonicalize(&absolute).unwrap_or_else(|_| clean_path(&absolute));
    }

    let mut anchor = absolute.as_path();
    let mut suffix = Vec::new();
    while !anchor.exists() {
        let Some(name) = anchor.file_name() else {
            break;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = anchor.parent() else {
            break;
        };
        anchor = parent;
    }
    let mut resolved = if anchor.exists() {
        dunce::canonicalize(anchor).unwrap_or_else(|_| clean_path(anchor))
    } else {
        clean_path(&absolute)
    };
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    clean_path(&resolved)
}

fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = normalize_absolute_path(path);
    let root = normalize_absolute_path(root);
    if cfg!(windows) {
        let path = slash_path(&path).to_ascii_lowercase();
        let root = slash_path(&root).trim_end_matches('/').to_ascii_lowercase();
        path == root || path.starts_with(&format!("{root}/"))
    } else {
        path == root || path.starts_with(root)
    }
}

fn dedupe_and_sort_sources(sources: &mut Vec<KnowledgeSource>) {
    let mut seen = HashSet::new();
    sources.retain(|source| {
        seen.insert((
            source.source_id.clone(),
            path_key(&source.physical_root),
            source.doc_type,
        ))
    });
    sources.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| {
                right
                    .physical_root
                    .components()
                    .count()
                    .cmp(&left.physical_root.components().count())
            })
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
}

fn dedupe_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| path.is_dir() && seen.insert(path_key(path)));
    paths.sort_by(|left, right| path_key(left).cmp(&path_key(right)));
}

fn path_key(path: &Path) -> String {
    let value = slash_path(&normalize_absolute_path(path));
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn join_logical(prefix: &str, relative: &str) -> String {
    match (prefix.trim_matches('/'), relative.trim_matches('/')) {
        ("", relative) => relative.to_string(),
        (prefix, "") => prefix.to_string(),
        (prefix, relative) => format!("{prefix}/{relative}"),
    }
}

fn normalize_logical_path(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    if normalized.is_empty()
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(normalized.to_string())
}

fn strip_logical_prefix<'a>(logical_path: &'a str, prefix: &str) -> Option<&'a str> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return Some(logical_path);
    }
    if logical_path == prefix {
        return Some("");
    }
    logical_path.strip_prefix(&format!("{prefix}/"))
}

#[cfg(test)]
mod tests {
    use super::{
        KnowledgeSource, KnowledgeSourceKind, KnowledgeSourceMutability, KnowledgeSourceRegistry,
    };
    use crate::knowledge_store::{KnowledgeStorageSource, KnowledgeType};
    use tempfile::tempdir;

    #[test]
    fn workspace_documents_use_workspace_relative_display_paths() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path().join("Locus/knowledge/design");
        std::fs::create_dir_all(&root).expect("create root");
        let file = root.join("core-loop.md");
        std::fs::write(&file, "# Core Loop\n").expect("write doc");
        let registry = KnowledgeSourceRegistry::from_sources_for_test(
            Some(temp.path().to_path_buf()),
            vec![KnowledgeSource {
                source_id: "workspace-design".to_string(),
                kind: KnowledgeSourceKind::WorkspaceKnowledge,
                doc_type: KnowledgeType::Design,
                physical_root: root,
                logical_prefix: String::new(),
                storage_source: KnowledgeStorageSource::Project,
                mutability: KnowledgeSourceMutability::Writable,
                watch: true,
                priority: 100,
            }],
        );

        let resolved = registry.classify_path(&file).expect("classify");
        assert_eq!(resolved.logical_path, "core-loop.md");
        assert_eq!(resolved.display_path, "Locus/knowledge/design/core-loop.md");
        assert!(resolved.workspace_relative_path.is_some());
    }

    #[test]
    fn outside_sources_use_absolute_display_paths() {
        let workspace = tempdir().expect("workspace");
        let external = tempdir().expect("external");
        let root = external.path().join("pdf");
        std::fs::create_dir_all(&root).expect("create external root");
        let file = root.join("SKILL.md");
        std::fs::write(&file, "# PDF\n").expect("write skill");
        let registry = KnowledgeSourceRegistry::from_sources_for_test(
            Some(workspace.path().to_path_buf()),
            vec![KnowledgeSource {
                source_id: "external-pdf".to_string(),
                kind: KnowledgeSourceKind::ExternalSkill,
                doc_type: KnowledgeType::Skill,
                physical_root: root,
                logical_prefix: "external/codex/pdf".to_string(),
                storage_source: KnowledgeStorageSource::App,
                mutability: KnowledgeSourceMutability::ReadOnly,
                watch: true,
                priority: 400,
            }],
        );

        let resolved = registry.classify_path(&file).expect("classify");
        assert_eq!(resolved.logical_path, "external/codex/pdf/SKILL.md");
        assert_eq!(
            resolved.display_path,
            file.to_string_lossy().replace('\\', "/")
        );
        assert!(resolved.workspace_relative_path.is_none());
        assert!(!resolved.mutability.is_writable());
    }

    #[test]
    fn logical_resolution_skips_unrelated_higher_priority_prefixes() {
        let workspace = tempdir().expect("workspace");
        let package = tempdir().expect("package");
        let package_file = package.path().join("SKILL.md");
        std::fs::write(&package_file, "# Package\n").expect("write package file");
        let unrelated = tempdir().expect("unrelated");
        let registry = KnowledgeSourceRegistry::from_sources_for_test(
            Some(workspace.path().to_path_buf()),
            vec![
                KnowledgeSource {
                    source_id: "unrelated".to_string(),
                    kind: KnowledgeSourceKind::ExternalSkill,
                    doc_type: KnowledgeType::Skill,
                    physical_root: unrelated.path().to_path_buf(),
                    logical_prefix: "other".to_string(),
                    storage_source: KnowledgeStorageSource::App,
                    mutability: KnowledgeSourceMutability::ReadOnly,
                    watch: true,
                    priority: 500,
                },
                KnowledgeSource {
                    source_id: "package".to_string(),
                    kind: KnowledgeSourceKind::AppSkillPackage,
                    doc_type: KnowledgeType::Skill,
                    physical_root: package.path().to_path_buf(),
                    logical_prefix: "package".to_string(),
                    storage_source: KnowledgeStorageSource::App,
                    mutability: KnowledgeSourceMutability::ReadOnly,
                    watch: true,
                    priority: 300,
                },
            ],
        );

        let resolved = registry
            .resolve_logical(KnowledgeType::Skill, "package/SKILL.md")
            .expect("resolve package path");
        assert_eq!(resolved.source_id, "package");
        assert_eq!(resolved.physical_path, package_file);
    }

    #[test]
    fn managed_reference_target_is_workspace_relative_and_managed() {
        let workspace = tempdir().expect("workspace");
        let registry =
            KnowledgeSourceRegistry::build(workspace.path().to_string_lossy().as_ref(), None);

        let target = registry
            .managed_materialization_target(
                KnowledgeType::Reference,
                "unity-official-docs/manual/execution-order.md",
            )
            .expect("managed target");
        assert_eq!(target.kind, KnowledgeSourceKind::ManagedReference);
        assert_eq!(target.mutability, KnowledgeSourceMutability::Managed);
        assert_eq!(
            target.display_path,
            "Library/Locus/KnowledgeSources/reference/unity-official-docs/manual/execution-order.md"
        );
    }
}
