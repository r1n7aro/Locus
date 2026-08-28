use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::watch;

use crate::config::{
    AppConfig, WorkspaceServiceResourceLimitFieldError, WorkspaceServiceResourceLimits,
    WorkspaceServiceResourceLimitsUpdateError, WorkspaceServiceResourceLimitsValidationErrors,
};

/// Checkout activity ordered by retention priority. Resource convergence
/// reclaims lower-priority checkouts first and treats pane/task activity as a
/// hard lifecycle barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceActivityPriority {
    Idle,
    BackgroundOpen,
    VisiblePane,
    RunningTask,
}

impl WorkspaceActivityPriority {
    pub fn protects_resources(self) -> bool {
        matches!(self, Self::VisiblePane | Self::RunningTask)
    }

    pub fn is_idle(self) -> bool {
        self == Self::Idle
    }
}

/// Immutable, validated policy generation consumed by runtime pools and
/// schedulers. Revisions are process-local and increase after each successful
/// persisted update.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicySnapshot {
    pub revision: u64,
    pub limits: WorkspaceServiceResourceLimits,
}

/// Configured portion of the resource metrics surface. Runtime owners append
/// their current usage and waiting counts without copying policy defaults.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePolicyConfiguredMetrics {
    pub revision: u64,
    pub configured_limits: WorkspaceServiceResourceLimits,
}

struct ResourcePolicyStoreInner {
    config: Arc<AppConfig>,
    snapshot_tx: watch::Sender<ResourcePolicySnapshot>,
    update_lock: Mutex<()>,
}

/// Process-wide source of validated workspace-service resource policy.
///
/// Consumers keep a `watch::Receiver` or take short-lived snapshots. Updates
/// are serialized and reach observers only after `AppConfig` has atomically
/// replaced the persisted config file and committed the same value in memory.
#[derive(Clone)]
pub struct ResourcePolicyStore {
    inner: Arc<ResourcePolicyStoreInner>,
}

impl ResourcePolicyStore {
    pub fn new(
        config: Arc<AppConfig>,
    ) -> Result<Self, WorkspaceServiceResourceLimitsValidationErrors> {
        Self::from_config(config)
    }

    pub fn from_config(
        config: Arc<AppConfig>,
    ) -> Result<Self, WorkspaceServiceResourceLimitsValidationErrors> {
        let limits = config
            .try_workspace_service_resource_limits()
            .map_err(|message| WorkspaceServiceResourceLimitsValidationErrors {
                fields: vec![WorkspaceServiceResourceLimitFieldError {
                    field: "workspaceServiceResourceLimits".to_string(),
                    message,
                }],
            })?;
        limits.validate()?;
        let (snapshot_tx, _) = watch::channel(ResourcePolicySnapshot {
            revision: 0,
            limits,
        });
        Ok(Self {
            inner: Arc::new(ResourcePolicyStoreInner {
                config,
                snapshot_tx,
                update_lock: Mutex::new(()),
            }),
        })
    }

    pub fn snapshot(&self) -> ResourcePolicySnapshot {
        self.inner.snapshot_tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<ResourcePolicySnapshot> {
        self.inner.snapshot_tx.subscribe()
    }

    pub fn update(
        &self,
        candidate: WorkspaceServiceResourceLimits,
    ) -> Result<ResourcePolicySnapshot, WorkspaceServiceResourceLimitsUpdateError> {
        candidate.validate()?;
        let _update_guard = self.inner.update_lock.lock().map_err(|error| {
            WorkspaceServiceResourceLimitsUpdateError::Persistence {
                message: format!("resource policy update lock poisoned: {error}"),
            }
        })?;

        let next_revision = self.snapshot().revision.saturating_add(1);
        let next = ResourcePolicySnapshot {
            revision: next_revision,
            limits: candidate.clone(),
        };

        // AppConfig performs candidate serialization and an atomic file
        // replacement before changing its in-memory value. No observer sees
        // `next` when this call fails.
        self.inner
            .config
            .set_workspace_service_resource_limits(candidate)?;
        self.inner.snapshot_tx.send_replace(next.clone());
        Ok(next)
    }

    pub fn configured_limits(&self) -> WorkspaceServiceResourceLimits {
        self.snapshot().limits
    }

    pub fn revision(&self) -> u64 {
        self.snapshot().revision
    }

    pub fn configured_metrics(&self) -> ResourcePolicyConfiguredMetrics {
        let snapshot = self.snapshot();
        ResourcePolicyConfiguredMetrics {
            revision: snapshot.revision,
            configured_limits: snapshot.limits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed_limits() -> WorkspaceServiceResourceLimits {
        WorkspaceServiceResourceLimits {
            max_running_workspace_services: 6,
            max_watched_workspaces: 3,
            max_lsp_processes: 2,
            max_concurrent_service_starts: 3,
            max_concurrent_compile_jobs: 2,
            max_compile_queue_depth: 17,
            workspace_idle_timeout_secs: 111,
            service_idle_timeout_secs: 222,
            lsp_idle_timeout_secs: 333,
        }
    }

    #[test]
    fn from_config_rejects_an_invalid_persisted_snapshot() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let path = temp.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
                "model": "test-model",
                "dynamic_tool_loading_native_migrated": true,
                "workspace_service_ttl_hour_migrated": true,
                "workspace_service_resource_limits": {
                    "maxRunningWorkspaceServices": 0
                }
            }"#,
        )
        .expect("write config");
        let config = Arc::new(AppConfig::load_from_path(&path));

        let errors = ResourcePolicyStore::from_config(config)
            .err()
            .expect("invalid persisted policy must be rejected");
        assert_eq!(errors.fields.len(), 1);
        assert_eq!(errors.fields[0].field, "maxRunningWorkspaceServices");
    }

    #[tokio::test]
    async fn successful_update_persists_before_publishing() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let path = temp.path().join("config.json");
        let config = Arc::new(AppConfig::load_from_path(&path));
        let store = ResourcePolicyStore::from_config(config.clone()).expect("valid policy");
        let mut updates = store.subscribe();

        let next = store.update(changed_limits()).expect("update policy");
        updates.changed().await.expect("published policy");

        assert_eq!(next.revision, 1);
        assert_eq!(*updates.borrow(), next);
        assert_eq!(config.workspace_service_resource_limits(), next.limits);
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read persisted config"))
                .expect("parse persisted config");
        assert_eq!(
            persisted["workspace_service_resource_limits"]["maxLspProcesses"],
            2
        );
        assert_eq!(store.configured_metrics().revision, 1);
    }

    #[test]
    fn persistence_failure_keeps_snapshot_and_watch_unchanged() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let path = temp.path().join("config-target-is-a-directory");
        std::fs::create_dir(&path).expect("create invalid config target");
        let config = Arc::new(AppConfig::load_from_path(&path));
        let store = ResourcePolicyStore::from_config(config.clone()).expect("valid defaults");
        let updates = store.subscribe();
        let before = store.snapshot();

        let error = store
            .update(changed_limits())
            .expect_err("directory target must reject persistence");

        assert!(matches!(
            error,
            WorkspaceServiceResourceLimitsUpdateError::Persistence { .. }
        ));
        assert_eq!(store.snapshot(), before);
        assert_eq!(config.workspace_service_resource_limits(), before.limits);
        assert!(!updates.has_changed().expect("watch state"));
    }

    #[test]
    fn invalid_update_is_field_scoped_and_never_published() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let path = temp.path().join("config.json");
        let config = Arc::new(AppConfig::load_from_path(&path));
        let store = ResourcePolicyStore::from_config(config).expect("valid defaults");
        let updates = store.subscribe();
        let before = store.snapshot();
        let mut invalid = changed_limits();
        invalid.max_compile_queue_depth = 0;
        invalid.lsp_idle_timeout_secs = 0;

        let error = store
            .update(invalid)
            .expect_err("invalid policy must be rejected");

        assert_eq!(
            error
                .validation_fields()
                .iter()
                .map(|field| field.field.as_str())
                .collect::<Vec<_>>(),
            vec!["maxCompileQueueDepth", "lspIdleTimeoutSecs"]
        );
        assert_eq!(store.snapshot(), before);
        assert!(!updates.has_changed().expect("watch state"));
    }

    #[test]
    fn clones_share_revision_and_publication_state() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let store = ResourcePolicyStore::from_config(config).expect("valid defaults");
        let clone = store.clone();

        clone
            .update(changed_limits())
            .expect("update through clone");

        assert_eq!(store.revision(), 1);
        assert_eq!(store.configured_limits().max_lsp_processes, 2);
    }
}
