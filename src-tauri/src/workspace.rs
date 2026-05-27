use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(rename = "workspace_id", alias = "workspaceId")]
    pub workspace_id: String,
}

pub struct Workspace {
    pub path: tokio::sync::RwLock<String>,
    pub workspace_id: tokio::sync::RwLock<Option<String>>,
    generation: AtomicU64,
    generation_lock: Mutex<()>,
}

impl Workspace {
    pub fn new(path: String, workspace_id: Option<String>) -> Self {
        Self {
            path: tokio::sync::RwLock::new(path),
            workspace_id: tokio::sync::RwLock::new(workspace_id),
            generation: AtomicU64::new(0),
            generation_lock: Mutex::new(()),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn lock_generation(&self) -> Result<WorkspaceGenerationGuard<'_>, String> {
        let guard = self
            .generation_lock
            .lock()
            .map_err(|e| format!("Workspace generation lock error: {}", e))?;
        Ok(WorkspaceGenerationGuard {
            workspace: self,
            _guard: guard,
        })
    }
}

pub struct WorkspaceGenerationGuard<'a> {
    workspace: &'a Workspace,
    _guard: MutexGuard<'a, ()>,
}

impl WorkspaceGenerationGuard<'_> {
    pub fn is_current(&self, generation: u64) -> bool {
        self.workspace.generation() == generation
    }

    pub fn bump_generation(&self) -> u64 {
        self.workspace.bump_generation()
    }
}

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

fn extract_unity_yaml_scalar(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{}:", key);
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix(&prefix)?.trim();
        let value = value.trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
}

fn unity_workspace_seed(dir: &str) -> Option<String> {
    let settings_path = Path::new(dir)
        .join("ProjectSettings")
        .join("ProjectSettings.asset");
    let content = std::fs::read_to_string(&settings_path).ok()?;

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

fn workspace_id_from_seed(seed: &str) -> String {
    let digest = blake3::hash(seed.as_bytes()).to_hex().to_string();
    format!("unity-{}", &digest[..24])
}

fn random_workspace_id() -> String {
    format!("workspace-{}", uuid::Uuid::new_v4().simple())
}

fn generated_workspace_id(dir: &str) -> String {
    unity_workspace_seed(dir)
        .map(|seed| workspace_id_from_seed(&seed))
        .unwrap_or_else(random_workspace_id)
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

    let workspace_id = generated_workspace_id(dir);
    if should_write_config {
        write_workspace_config(
            dir,
            &WorkspaceConfig {
                workspace_id: workspace_id.clone(),
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
        generated_workspace_id, load_or_create_workspace, read_workspace_config, Workspace,
        WorkspaceConfig,
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

        let camel = r#"{"workspaceId":"camel-id","memory":{"enabled":false}}"#;
        let camel_cfg: WorkspaceConfig =
            serde_json::from_str(camel).expect("camelCase workspace config should parse");
        assert_eq!(camel_cfg.workspace_id, "camel-id");
    }

    #[test]
    fn workspace_config_serializes_workspace_id_in_snake_case() {
        let cfg = WorkspaceConfig {
            workspace_id: "stable-id".to_string(),
        };
        let value = serde_json::to_value(&cfg).expect("workspace config should serialize");
        assert_eq!(
            value.get("workspace_id").and_then(|v| v.as_str()),
            Some("stable-id")
        );
        assert!(value.get("workspaceId").is_none());
        assert!(value.get("memory").is_none());
    }

    #[test]
    fn workspace_generation_advances_on_bump() {
        let workspace = Workspace::new("A".to_string(), Some("workspace-a".to_string()));
        let initial = workspace.generation();
        assert_eq!(workspace.bump_generation(), initial + 1);
        assert_eq!(workspace.generation(), initial + 1);
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
    fn generated_workspace_id_falls_back_to_random_id_without_unity_guid() {
        let dir = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir,
            "PlayerSettings:\n  companyName: OpenAI\n  productName: Locus\n  applicationIdentifier:\n    Standalone: com.openai.locus\n",
        );

        let id = generated_workspace_id(&dir.path().to_string_lossy());
        assert!(id.starts_with("workspace-"));
        assert_eq!(id.len(), "workspace-".len() + 32);
    }

    #[test]
    fn load_or_create_workspace_persists_random_id_without_unity_guid() {
        let dir = tempfile::tempdir().unwrap();
        write_project_settings(
            &dir,
            "PlayerSettings:\n  companyName: OpenAI\n  productName: Locus\n  applicationIdentifier:\n    Standalone: com.openai.locus\n",
        );

        let first = load_or_create_workspace(&dir.path().to_string_lossy()).unwrap();
        let second = load_or_create_workspace(&dir.path().to_string_lossy()).unwrap();
        let cfg = read_workspace_config(&dir.path().to_string_lossy()).unwrap();

        assert!(first.starts_with("workspace-"));
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
