use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const ID_HEX_LEN: usize = 24;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceIdentityError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(WorkspaceIdentityError::EmptyId {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

string_id!(ProjectId);
string_id!(CheckoutId);
string_id!(ServiceInstanceId);

impl CheckoutId {
    pub fn from_normalized_root(root: &NormalizedWorkspaceRoot) -> Self {
        Self(stable_id("checkout", root.key()))
    }
}

impl ServiceInstanceId {
    pub fn for_service(checkout_id: &CheckoutId, service_kind: &str) -> Self {
        let service_kind = service_kind.trim().to_ascii_lowercase();
        let material = format!("{}\0{}", checkout_id.as_str(), service_kind);
        Self(stable_id("service", &material))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWorkspaceRoot {
    path: PathBuf,
    key: String,
}

impl NormalizedWorkspaceRoot {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn into_path(self) -> PathBuf {
        self.path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectIdSource {
    ExistingWorkspaceConfig,
    UnityProjectGuid,
    GitCommonDir,
    CheckoutFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspaceIdentity {
    pub project_id: ProjectId,
    pub checkout_id: CheckoutId,
    pub root: PathBuf,
    pub normalized_root: String,
    pub project_id_source: ProjectIdSource,
    pub git_common_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub enum WorkspaceIdentityError {
    EmptyPath,
    RootNotFound(PathBuf),
    RootNotDirectory(PathBuf),
    CanonicalizeRoot {
        path: PathBuf,
        error: std::io::Error,
    },
    EmptyId {
        kind: &'static str,
    },
}

impl fmt::Display for WorkspaceIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("Workspace path cannot be empty"),
            Self::RootNotFound(path) => {
                write!(
                    formatter,
                    "Workspace directory does not exist: {}",
                    path.display()
                )
            }
            Self::RootNotDirectory(path) => {
                write!(
                    formatter,
                    "Workspace path is not a directory: {}",
                    path.display()
                )
            }
            Self::CanonicalizeRoot { path, error } => write!(
                formatter,
                "Failed to normalize workspace directory '{}': {}",
                path.display(),
                error
            ),
            Self::EmptyId { kind } => write!(formatter, "{} cannot be empty", kind),
        }
    }
}

impl std::error::Error for WorkspaceIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CanonicalizeRoot { error, .. } => Some(error),
            _ => None,
        }
    }
}

pub struct ProjectIdResolver;

impl ProjectIdResolver {
    pub fn resolve(
        path: impl AsRef<Path>,
    ) -> Result<ResolvedWorkspaceIdentity, WorkspaceIdentityError> {
        let root = normalize_existing_workspace_root(path)?;
        let checkout_id = CheckoutId::from_normalized_root(&root);
        let git_common_dir = resolve_git_common_dir(root.path());

        let (project_id, project_id_source) =
            if let Some(workspace_id) = read_existing_workspace_id(root.path()) {
                (
                    ProjectId::new(workspace_id)?,
                    ProjectIdSource::ExistingWorkspaceConfig,
                )
            } else if let Some(seed) = unity_project_seed(root.path()) {
                (
                    ProjectId::new(stable_id("unity", &seed))?,
                    ProjectIdSource::UnityProjectGuid,
                )
            } else if let Some(common_dir) = git_common_dir.as_deref() {
                let common_dir_key = normalized_path_key(common_dir);
                (
                    ProjectId::new(stable_id("git", &common_dir_key))?,
                    ProjectIdSource::GitCommonDir,
                )
            } else {
                (
                    ProjectId::new(checkout_id.as_str().to_string())?,
                    ProjectIdSource::CheckoutFallback,
                )
            };

        Ok(ResolvedWorkspaceIdentity {
            project_id,
            checkout_id,
            root: root.path().to_path_buf(),
            normalized_root: root.key().to_string(),
            project_id_source,
            git_common_dir,
        })
    }
}

pub fn normalize_existing_workspace_root(
    path: impl AsRef<Path>,
) -> Result<NormalizedWorkspaceRoot, WorkspaceIdentityError> {
    let input = path.as_ref();
    if input.as_os_str().is_empty() {
        return Err(WorkspaceIdentityError::EmptyPath);
    }

    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(input))
            .unwrap_or_else(|_| input.to_path_buf())
    };

    if !absolute.exists() {
        return Err(WorkspaceIdentityError::RootNotFound(absolute));
    }
    if !absolute.is_dir() {
        return Err(WorkspaceIdentityError::RootNotDirectory(absolute));
    }

    let canonical = dunce::canonicalize(&absolute).map_err(|error| {
        WorkspaceIdentityError::CanonicalizeRoot {
            path: absolute.clone(),
            error,
        }
    })?;
    let path = dunce::simplified(&canonical).to_path_buf();
    let key = normalized_path_key(&path);
    Ok(NormalizedWorkspaceRoot { path, key })
}

/// Resolve the Git common directory without spawning Git. This covers normal
/// repositories, linked worktrees (`.git` file + `commondir`) and submodules.
/// The search walks ancestors so a workspace nested inside a repository still
/// joins the repository's logical project.
pub fn resolve_git_common_dir(workspace_root: &Path) -> Option<PathBuf> {
    for ancestor in workspace_root.ancestors() {
        let dot_git = ancestor.join(".git");
        let git_dir = if dot_git.is_dir() {
            dunce::canonicalize(&dot_git).ok()?
        } else if dot_git.is_file() {
            resolve_git_dir_file(&dot_git)?
        } else {
            continue;
        };

        let common_dir_file = git_dir.join("commondir");
        let common_dir = if common_dir_file.is_file() {
            let raw = std::fs::read_to_string(&common_dir_file).ok()?;
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let candidate = Path::new(raw);
            let candidate = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                git_dir.join(candidate)
            };
            dunce::canonicalize(candidate).ok()?
        } else {
            git_dir
        };

        return Some(dunce::simplified(&common_dir).to_path_buf());
    }
    None
}

fn resolve_git_dir_file(dot_git_file: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(dot_git_file).ok()?;
    let first_line = content.lines().next()?.trim();
    let (label, raw_path) = first_line.split_once(':')?;
    if !label.trim().eq_ignore_ascii_case("gitdir") {
        return None;
    }
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let candidate = Path::new(raw_path);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        dot_git_file.parent()?.join(candidate)
    };
    let canonical = dunce::canonicalize(candidate).ok()?;
    canonical
        .is_dir()
        .then(|| dunce::simplified(&canonical).to_path_buf())
}

#[derive(Deserialize)]
struct ExistingWorkspaceConfig {
    #[serde(default, rename = "workspace_id", alias = "workspaceId")]
    workspace_id: Option<String>,
}

fn read_existing_workspace_id(root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("Locus").join("config.json")).ok()?;
    let config = serde_json::from_str::<ExistingWorkspaceConfig>(&content).ok()?;
    config
        .workspace_id
        .filter(|workspace_id| !workspace_id.trim().is_empty())
}

fn unity_project_seed(root: &Path) -> Option<String> {
    let content =
        std::fs::read_to_string(root.join("ProjectSettings").join("ProjectSettings.asset")).ok()?;
    for key in [
        "productGUID",
        "projectGUID",
        "projectGuid",
        "cloudProjectId",
    ] {
        if let Some(value) = extract_unity_yaml_scalar(&content, key) {
            return Some(format!("unity:{}={}", key, value));
        }
    }
    None
}

fn extract_unity_yaml_scalar(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{}:", key);
    content.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        let value = value.trim_matches('"').trim_matches('\'').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn stable_id(prefix: &str, material: &str) -> String {
    let digest = blake3::hash(material.as_bytes()).to_hex().to_string();
    format!("{}-{}", prefix, &digest[..ID_HEX_LEN])
}

fn normalized_path_key(path: &Path) -> String {
    let mut key = dunce::simplified(path).to_string_lossy().replace('\\', "/");
    if let Some(stripped) = key.strip_prefix("//?/") {
        key = stripped.to_string();
    }
    while key.len() > 1 && key.ends_with('/') && !is_windows_drive_root(&key) {
        key.pop();
    }
    if cfg!(windows) {
        key.make_ascii_lowercase();
    }
    key
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_unity_guid(root: &Path, guid: &str) {
        let settings = root.join("ProjectSettings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("ProjectSettings.asset"),
            format!("PlayerSettings:\n  productGUID: {guid}\n"),
        )
        .unwrap();
    }

    fn write_workspace_config(root: &Path, body: &str) {
        let locus = root.join("Locus");
        std::fs::create_dir_all(&locus).unwrap();
        std::fs::write(locus.join("config.json"), body).unwrap();
    }

    #[test]
    fn ids_are_transparent_strings() {
        let id = ProjectId::new("project-a").unwrap();
        assert_eq!(id.as_str(), "project-a");
        assert_eq!(id.to_string(), "project-a");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""project-a""#);
        assert_eq!(
            serde_json::from_str::<ProjectId>(r#""project-a""#).unwrap(),
            id
        );
        assert!(ProjectId::new("  ").is_err());
    }

    #[test]
    fn checkout_id_is_stable_for_equivalent_existing_paths() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        let direct = normalize_existing_workspace_root(&nested).unwrap();
        let dotted = normalize_existing_workspace_root(nested.join(".")).unwrap();
        assert_eq!(direct, dotted);
        assert_eq!(
            CheckoutId::from_normalized_root(&direct),
            CheckoutId::from_normalized_root(&dotted)
        );
    }

    #[test]
    fn relative_workspace_root_matches_its_absolute_path() {
        let current = std::env::current_dir().unwrap();
        let relative = ProjectIdResolver::resolve(".").unwrap();
        let absolute = ProjectIdResolver::resolve(current).unwrap();
        assert_eq!(relative.checkout_id, absolute.checkout_id);
        assert_eq!(relative.normalized_root, absolute.normalized_root);
    }

    #[cfg(windows)]
    #[test]
    fn checkout_id_normalizes_windows_case_separators_and_extended_prefix() {
        let root = tempfile::tempdir().unwrap();
        let canonical = dunce::canonicalize(root.path()).unwrap();
        let normal = canonical.to_string_lossy().to_string();
        let slash_and_case = normal.replace('\\', "/").to_ascii_uppercase();
        let extended = format!(r"\\?\{}\", normal);

        let normal = ProjectIdResolver::resolve(&normal).unwrap();
        let slash_and_case = ProjectIdResolver::resolve(&slash_and_case).unwrap();
        let extended = ProjectIdResolver::resolve(&extended).unwrap();
        assert_eq!(normal.checkout_id, slash_and_case.checkout_id);
        assert_eq!(normal.checkout_id, extended.checkout_id);
    }

    #[test]
    fn workspace_root_must_exist_and_be_a_directory() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(matches!(
            normalize_existing_workspace_root(root.path().join("missing")),
            Err(WorkspaceIdentityError::RootNotFound(_))
        ));
        assert!(matches!(
            normalize_existing_workspace_root(file),
            Err(WorkspaceIdentityError::RootNotDirectory(_))
        ));
    }

    #[test]
    fn existing_workspace_config_has_highest_priority() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        write_unity_guid(root.path(), "guid-from-unity");
        write_workspace_config(root.path(), r#"{"workspace_id":"existing-project"}"#);

        let resolved = ProjectIdResolver::resolve(root.path()).unwrap();
        assert_eq!(resolved.project_id.as_str(), "existing-project");
        assert_eq!(
            resolved.project_id_source,
            ProjectIdSource::ExistingWorkspaceConfig
        );
    }

    #[test]
    fn existing_workspace_config_accepts_camel_case_alias() {
        let root = tempfile::tempdir().unwrap();
        write_workspace_config(root.path(), r#"{"workspaceId":"camel-project"}"#);
        let resolved = ProjectIdResolver::resolve(root.path()).unwrap();
        assert_eq!(resolved.project_id.as_str(), "camel-project");
    }

    #[test]
    fn empty_workspace_config_id_falls_through_to_unity_guid() {
        let root = tempfile::tempdir().unwrap();
        write_workspace_config(root.path(), r#"{"workspace_id":"  "}"#);
        write_unity_guid(root.path(), "unity-after-empty-config");
        let resolved = ProjectIdResolver::resolve(root.path()).unwrap();
        assert_eq!(
            resolved.project_id_source,
            ProjectIdSource::UnityProjectGuid
        );
    }

    #[test]
    fn unity_guid_groups_distinct_checkouts_before_git_identity() {
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        for root in [left.path(), right.path()] {
            std::fs::create_dir_all(root.join(".git")).unwrap();
            write_unity_guid(root, "same-unity-guid");
        }

        let left = ProjectIdResolver::resolve(left.path()).unwrap();
        let right = ProjectIdResolver::resolve(right.path()).unwrap();
        assert_eq!(left.project_id, right.project_id);
        assert_ne!(left.checkout_id, right.checkout_id);
        assert_eq!(left.project_id_source, ProjectIdSource::UnityProjectGuid);
        assert!(left.project_id.as_str().starts_with("unity-"));
    }

    #[test]
    fn linked_git_worktrees_share_project_and_keep_distinct_checkouts() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        let linked = root.path().join("feature");
        let main_git = main.join(".git");
        let linked_git_dir = main_git.join("worktrees").join("feature");
        std::fs::create_dir_all(&main_git).unwrap();
        std::fs::create_dir_all(&linked_git_dir).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked_git_dir.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            linked.join(".git"),
            "gitdir: ../main/.git/worktrees/feature\n",
        )
        .unwrap();

        let main = ProjectIdResolver::resolve(&main).unwrap();
        let linked = ProjectIdResolver::resolve(&linked).unwrap();
        assert_eq!(main.project_id_source, ProjectIdSource::GitCommonDir);
        assert_eq!(linked.project_id_source, ProjectIdSource::GitCommonDir);
        assert_eq!(main.project_id, linked.project_id);
        assert_ne!(main.checkout_id, linked.checkout_id);
        assert_eq!(
            normalized_path_key(main.git_common_dir.as_deref().unwrap()),
            normalized_path_key(linked.git_common_dir.as_deref().unwrap())
        );
    }

    #[test]
    fn nested_git_workspace_resolves_ancestor_common_dir() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        let nested = root.path().join("packages").join("game");
        std::fs::create_dir_all(&nested).unwrap();

        let top = ProjectIdResolver::resolve(root.path()).unwrap();
        let nested = ProjectIdResolver::resolve(&nested).unwrap();
        assert_eq!(top.project_id, nested.project_id);
        assert_ne!(top.checkout_id, nested.checkout_id);
    }

    #[test]
    fn ordinary_directory_falls_back_to_its_checkout_identity() {
        let root = tempfile::tempdir().unwrap();
        let resolved = ProjectIdResolver::resolve(root.path()).unwrap();
        assert_eq!(
            resolved.project_id_source,
            ProjectIdSource::CheckoutFallback
        );
        assert_eq!(resolved.project_id.as_str(), resolved.checkout_id.as_str());
        assert!(resolved.project_id.as_str().starts_with("checkout-"));
    }

    #[test]
    fn service_instance_id_is_stable_per_checkout_and_service_kind() {
        let root = tempfile::tempdir().unwrap();
        let checkout = ProjectIdResolver::resolve(root.path()).unwrap().checkout_id;
        let unity = ServiceInstanceId::for_service(&checkout, "Unity");
        assert_eq!(unity, ServiceInstanceId::for_service(&checkout, " unity "));
        assert_ne!(unity, ServiceInstanceId::for_service(&checkout, "unreal"));

        let other_root = tempfile::tempdir().unwrap();
        let other_checkout = ProjectIdResolver::resolve(other_root.path())
            .unwrap()
            .checkout_id;
        assert_ne!(
            unity,
            ServiceInstanceId::for_service(&other_checkout, "unity")
        );
    }
}
