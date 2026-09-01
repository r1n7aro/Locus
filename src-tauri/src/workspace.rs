use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::workspace_service::identity::ProjectIdResolver;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(rename = "workspace_id", alias = "workspaceId")]
    pub workspace_id: String,
    #[serde(
        default,
        rename = "unity_test_tools_enabled",
        alias = "unityTestToolsEnabled"
    )]
    pub unity_test_tools_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestToolsWorkspaceStatus {
    pub enabled: bool,
    pub package_installed: bool,
    pub package_version: Option<String>,
    pub package_supported: bool,
    pub available: bool,
}

pub const UNITY_TEST_FRAMEWORK_MIN_VERSION: &str = "1.4.0";

pub fn workspace_config_path(dir: &str) -> std::path::PathBuf {
    Path::new(dir).join("Locus").join("config.json")
}

pub fn read_workspace_config(dir: &str) -> Result<WorkspaceConfig, String> {
    let config_path = workspace_config_path(dir);
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read workspace config: {}", e))?;
    serde_json::from_str::<WorkspaceConfig>(&content)
        .map_err(|e| format!("Failed to parse workspace config: {}", e))
}

pub fn write_workspace_config(dir: &str, config: &WorkspaceConfig) -> Result<(), String> {
    let config_path = workspace_config_path(dir);
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create Locus directory: {}", e))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize workspace config: {}", e))?;
    std::fs::write(&config_path, &json)
        .map_err(|e| format!("Failed to write workspace config: {}", e))
}

fn read_json_object(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&content)
        .ok()?
        .as_object()
        .cloned()
}

fn unity_test_framework_package_info(dir: &str) -> (bool, Option<String>) {
    let packages = Path::new(dir).join("Packages");
    let mut installed = false;
    for file_name in ["packages-lock.json", "manifest.json"] {
        let Some(root) = read_json_object(&packages.join(file_name)) else {
            continue;
        };
        let Some(dependency) = root
            .get("dependencies")
            .and_then(serde_json::Value::as_object)
            .and_then(|dependencies| dependencies.get("com.unity.test-framework"))
        else {
            continue;
        };
        installed = true;
        let version = dependency
            .as_str()
            .or_else(|| {
                dependency
                    .get("version")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(version) = version {
            if semver::Version::parse(version.trim_start_matches('v')).is_ok() {
                return (true, Some(version.to_string()));
            }
        }
    }
    (installed, None)
}

fn unity_test_framework_package_supported(version: Option<&str>) -> bool {
    let Some(version) =
        version.and_then(|value| semver::Version::parse(value.trim_start_matches('v')).ok())
    else {
        return false;
    };
    let minimum = semver::Version::parse(UNITY_TEST_FRAMEWORK_MIN_VERSION)
        .expect("Unity Test Framework minimum version must be valid semver");
    version >= minimum
}

pub fn unity_test_tools_workspace_status(dir: &str) -> UnityTestToolsWorkspaceStatus {
    let enabled = read_workspace_config(dir)
        .map(|config| config.unity_test_tools_enabled)
        .unwrap_or(false);
    let (package_installed, package_version) = unity_test_framework_package_info(dir);
    let package_supported = unity_test_framework_package_supported(package_version.as_deref());
    UnityTestToolsWorkspaceStatus {
        enabled,
        package_installed,
        package_version,
        package_supported,
        available: enabled && package_installed && package_supported,
    }
}

pub fn unity_test_tools_available(dir: &str) -> bool {
    unity_test_tools_workspace_status(dir).available
}

#[derive(Default)]
struct UnityTestPendingSourceState {
    edit_seq: u64,
    paths: BTreeMap<String, (String, u64)>,
}

fn unity_test_pending_sources() -> &'static Mutex<HashMap<String, UnityTestPendingSourceState>> {
    static PENDING: OnceLock<Mutex<HashMap<String, UnityTestPendingSourceState>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalized_path_key(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn unity_test_source_path(project_path: &str, file_path: &str) -> Option<String> {
    let project = Path::new(project_path);
    let absolute = if Path::new(file_path).is_absolute() {
        Path::new(file_path).to_path_buf()
    } else {
        project.join(file_path)
    };
    let absolute_text = absolute.to_string_lossy().to_string();
    let project_key = normalized_path_key(project_path);
    let file_key = normalized_path_key(&absolute_text);
    let relative = file_key.strip_prefix(&format!("{project_key}/"))?;
    if !relative.ends_with(".cs")
        || !(relative.starts_with("assets/") || relative.starts_with("packages/"))
        || relative.starts_with("packages/com.farlocus.locus/")
    {
        return None;
    }
    Some(absolute_text)
}

/// Track C# files written by Locus while Unity Test tools are available.
/// Test discovery is held until a real Unity compilation and domain reload
/// converge those files into the Test Framework's test-list cache.
pub fn note_unity_test_source_written(project_path: &str, file_path: &str) {
    if !unity_test_tools_available(project_path) {
        return;
    }
    let Some(absolute_path) = unity_test_source_path(project_path, file_path) else {
        return;
    };
    if let Ok(mut pending) = unity_test_pending_sources().lock() {
        let state = pending
            .entry(normalized_path_key(project_path))
            .or_default();
        state.edit_seq += 1;
        let edit_seq = state.edit_seq;
        state.paths.insert(
            normalized_path_key(&absolute_path),
            (absolute_path, edit_seq),
        );
    }
}

pub fn unity_test_sources_pending(project_path: &str) -> bool {
    unity_test_pending_sources()
        .lock()
        .ok()
        .and_then(|pending| {
            pending
                .get(&normalized_path_key(project_path))
                .map(|state| !state.paths.is_empty())
        })
        .unwrap_or(false)
}

/// Snapshot pending paths and the highest write sequence they represent.
/// A successful recompile clears only through this sequence, preserving edits
/// that race the compilation window for the next convergence cycle.
pub fn unity_test_pending_source_snapshot(project_path: &str) -> (u64, Vec<String>) {
    unity_test_pending_sources()
        .lock()
        .ok()
        .and_then(|pending| {
            pending
                .get(&normalized_path_key(project_path))
                .map(|state| {
                    (
                        state.edit_seq,
                        state.paths.values().map(|(path, _)| path.clone()).collect(),
                    )
                })
        })
        .unwrap_or_default()
}

pub fn clear_unity_test_pending_sources_through(project_path: &str, seq_bound: u64) {
    if let Ok(mut pending) = unity_test_pending_sources().lock() {
        let project_key = normalized_path_key(project_path);
        let remove_project = if let Some(state) = pending.get_mut(&project_key) {
            state.paths.retain(|_, (_, seq)| *seq > seq_bound);
            state.paths.is_empty()
        } else {
            false
        };
        if remove_project {
            pending.remove(&project_key);
        }
    }
}

pub fn set_unity_test_tools_enabled(dir: &str, enabled: bool) -> Result<(), String> {
    let config_path = workspace_config_path(dir);
    let content = std::fs::read_to_string(&config_path)
        .map_err(|error| format!("Failed to read workspace config: {error}"))?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| format!("Failed to parse workspace config: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Workspace config must be a JSON object".to_string())?;
    object.insert(
        "unity_test_tools_enabled".to_string(),
        serde_json::Value::Bool(enabled),
    );
    let json = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to serialize workspace config: {error}"))?;
    std::fs::write(&config_path, json)
        .map_err(|error| format!("Failed to write workspace config: {error}"))
}

fn generated_workspace_id(dir: &str) -> String {
    ProjectIdResolver::resolve(dir)
        .expect("generated workspace identity requires an existing directory")
        .project_id
        .into_string()
}

pub fn load_or_create_workspace(dir: &str) -> Result<String, String> {
    let config_path = workspace_config_path(dir);
    let mut should_write_config = !config_path.exists();

    match read_workspace_config(dir) {
        Ok(cfg) if !cfg.workspace_id.is_empty() => {
            return Ok(cfg.workspace_id);
        }
        Ok(_) => {
            eprintln!("[Workspace] legacy config missing workspace_id, creating workspace id");
            should_write_config = true;
        }
        Err(err) => {
            if config_path.exists() {
                eprintln!("[Workspace] failed to read legacy config.json: {}", err);
            }
        }
    }

    let workspace_id = ProjectIdResolver::resolve(dir)
        .map_err(|error| error.to_string())?
        .project_id
        .into_string();
    if should_write_config {
        write_workspace_config(
            dir,
            &WorkspaceConfig {
                workspace_id: workspace_id.clone(),
                unity_test_tools_enabled: false,
            },
        )?;
    }
    eprintln!("[Workspace] resolved workspace {} at {}", workspace_id, dir);
    Ok(workspace_id)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        clear_unity_test_pending_sources_through, generated_workspace_id, load_or_create_workspace,
        note_unity_test_source_written, read_workspace_config, set_unity_test_tools_enabled,
        unity_test_pending_source_snapshot, unity_test_sources_pending,
        unity_test_tools_workspace_status, WorkspaceConfig,
    };

    fn write_project_settings(root: &tempfile::TempDir, body: &str) {
        let settings_dir = root.path().join("ProjectSettings");
        fs::create_dir_all(&settings_dir).unwrap();
        fs::write(settings_dir.join("ProjectSettings.asset"), body).unwrap();
    }

    #[test]
    fn workspace_config_accepts_legacy_and_camel_case_keys() {
        let legacy = r#"{"workspace_id":"legacy-id","memory":{"enabled":true}}"#;
        let legacy_cfg: WorkspaceConfig =
            serde_json::from_str(legacy).expect("legacy workspace config should parse");
        assert_eq!(legacy_cfg.workspace_id, "legacy-id");
        assert!(!legacy_cfg.unity_test_tools_enabled);

        let camel =
            r#"{"workspaceId":"camel-id","unityTestToolsEnabled":true,"memory":{"enabled":false}}"#;
        let camel_cfg: WorkspaceConfig =
            serde_json::from_str(camel).expect("camelCase workspace config should parse");
        assert_eq!(camel_cfg.workspace_id, "camel-id");
        assert!(camel_cfg.unity_test_tools_enabled);
    }

    #[test]
    fn workspace_config_serializes_workspace_id_in_snake_case() {
        let cfg = WorkspaceConfig {
            workspace_id: "stable-id".to_string(),
            unity_test_tools_enabled: true,
        };
        let value = serde_json::to_value(&cfg).expect("workspace config should serialize");
        assert_eq!(
            value.get("workspace_id").and_then(|v| v.as_str()),
            Some("stable-id")
        );
        assert!(value.get("workspaceId").is_none());
        assert_eq!(
            value
                .get("unity_test_tools_enabled")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(value.get("memory").is_none());
    }

    #[test]
    fn unity_test_workspace_setting_preserves_unrelated_config_fields() {
        let dir = tempfile::tempdir().unwrap();
        let locus_dir = dir.path().join("Locus");
        fs::create_dir_all(&locus_dir).unwrap();
        fs::write(
            locus_dir.join("config.json"),
            r#"{"workspace_id":"stable","memory":{"enabled":true}}"#,
        )
        .unwrap();

        set_unity_test_tools_enabled(&dir.path().to_string_lossy(), true).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(locus_dir.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(value["unity_test_tools_enabled"], true);
        assert_eq!(value["memory"]["enabled"], true);
    }

    #[test]
    fn unity_test_tools_require_workspace_setting_and_supported_package() {
        let dir = tempfile::tempdir().unwrap();
        let locus_dir = dir.path().join("Locus");
        let packages_dir = dir.path().join("Packages");
        fs::create_dir_all(&locus_dir).unwrap();
        fs::create_dir_all(&packages_dir).unwrap();
        fs::write(
            locus_dir.join("config.json"),
            r#"{"workspace_id":"stable","unity_test_tools_enabled":true}"#,
        )
        .unwrap();

        let missing = unity_test_tools_workspace_status(&dir.path().to_string_lossy());
        assert!(missing.enabled);
        assert!(!missing.package_installed);
        assert!(!missing.package_supported);
        assert!(!missing.available);

        fs::write(
            packages_dir.join("packages-lock.json"),
            r#"{"dependencies":{"com.unity.test-framework":{"version":"1.7.0"}}}"#,
        )
        .unwrap();
        let installed = unity_test_tools_workspace_status(&dir.path().to_string_lossy());
        assert_eq!(installed.package_version.as_deref(), Some("1.7.0"));
        assert!(installed.package_supported);
        assert!(installed.available);

        fs::write(
            packages_dir.join("packages-lock.json"),
            r#"{"dependencies":{"com.unity.test-framework":{"version":"1.3.9"}}}"#,
        )
        .unwrap();
        let unsupported = unity_test_tools_workspace_status(&dir.path().to_string_lossy());
        assert!(unsupported.package_installed);
        assert_eq!(unsupported.package_version.as_deref(), Some("1.3.9"));
        assert!(!unsupported.package_supported);
        assert!(!unsupported.available);
    }

    #[test]
    fn unity_test_source_edits_wait_for_explicit_convergence() {
        let dir = tempfile::tempdir().unwrap();
        let locus_dir = dir.path().join("Locus");
        let packages_dir = dir.path().join("Packages");
        let tests_dir = dir.path().join("Assets").join("Tests");
        fs::create_dir_all(&locus_dir).unwrap();
        fs::create_dir_all(&packages_dir).unwrap();
        fs::create_dir_all(&tests_dir).unwrap();
        fs::write(
            locus_dir.join("config.json"),
            r#"{"workspace_id":"stable","unity_test_tools_enabled":true}"#,
        )
        .unwrap();
        fs::write(
            packages_dir.join("manifest.json"),
            r#"{"dependencies":{"com.unity.test-framework":"1.4.0"}}"#,
        )
        .unwrap();

        let project = dir.path().to_string_lossy().to_string();
        let source = tests_dir.join("NewTest.cs").to_string_lossy().to_string();
        note_unity_test_source_written(&project, &source);

        assert!(unity_test_sources_pending(&project));
        let (first_seq, first_paths) = unity_test_pending_source_snapshot(&project);
        assert_eq!(first_paths, vec![source]);

        let racing_source = tests_dir
            .join("RacingTest.cs")
            .to_string_lossy()
            .to_string();
        note_unity_test_source_written(&project, &racing_source);
        clear_unity_test_pending_sources_through(&project, first_seq);
        let (second_seq, remaining_paths) = unity_test_pending_source_snapshot(&project);
        assert_eq!(remaining_paths, vec![racing_source]);
        assert!(unity_test_sources_pending(&project));

        clear_unity_test_pending_sources_through(&project, second_seq);
        assert!(!unity_test_sources_pending(&project));
    }

    #[test]
    fn generated_workspace_id_prefers_unity_project_guid_like_fields() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir_a,
            "PlayerSettings:\n  productGUID: 2d9a8f42f0da40f2a22b9c4c93ce7d34\n",
        );
        write_project_settings(
            &dir_b,
            "PlayerSettings:\n  productGUID: 2d9a8f42f0da40f2a22b9c4c93ce7d34\n",
        );

        let left = generated_workspace_id(&dir_a.path().to_string_lossy());
        let right = generated_workspace_id(&dir_b.path().to_string_lossy());
        assert_eq!(left, right);
    }

    #[test]
    fn generated_workspace_id_falls_back_to_checkout_id_without_unity_guid() {
        let dir = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir,
            "PlayerSettings:\n  companyName: OpenAI\n  productName: Locus\n  applicationIdentifier:\n    Standalone: com.openai.locus\n",
        );

        let id = generated_workspace_id(&dir.path().to_string_lossy());
        assert!(id.starts_with("checkout-"));
        assert_eq!(id.len(), "checkout-".len() + 24);
    }

    #[test]
    fn load_or_create_workspace_persists_checkout_id_without_unity_guid() {
        let dir = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir,
            "PlayerSettings:\n  companyName: OpenAI\n  productName: Locus\n  applicationIdentifier:\n    Standalone: com.openai.locus\n",
        );

        let first = load_or_create_workspace(&dir.path().to_string_lossy()).unwrap();
        let second = load_or_create_workspace(&dir.path().to_string_lossy()).unwrap();
        let cfg = read_workspace_config(&dir.path().to_string_lossy()).unwrap();

        assert!(first.starts_with("checkout-"));
        assert_eq!(first, second);
        assert_eq!(cfg.workspace_id, first);
    }

    #[test]
    fn load_or_create_workspace_persists_unity_guid_id_when_config_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir,
            "PlayerSettings:\n  productGUID: 2d9a8f42f0da40f2a22b9c4c93ce7d34\n",
        );

        let workspace_id = load_or_create_workspace(&dir.path().to_string_lossy()).unwrap();
        let cfg = read_workspace_config(&dir.path().to_string_lossy()).unwrap();

        assert!(workspace_id.starts_with("unity-"));
        assert_eq!(cfg.workspace_id, workspace_id);
    }
}
