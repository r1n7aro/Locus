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

    /// Publishes the persisted limits after a generic settings write changed
    /// them without going through `update`. No-op when nothing moved.
    pub fn republish_from_config(&self) -> ResourcePolicySnapshot {
        let _update_guard = self
            .inner
            .update_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = self.snapshot();
        let limits = self.inner.config.workspace_service_resource_limits();
        if limits == current.limits {
            return current;
        }
        let next = ResourcePolicySnapshot {
            revision: current.revision.saturating_add(1),
            limits,
        };
        self.inner.snapshot_tx.send_replace(next.clone());
        next
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
            workspace_idle_timeout_secs: 111,
            service_idle_timeout_secs: 222,
            lsp_idle_timeout_secs: 333,
            ..Default::default()
        }
    }

    #[test]
    fn invalid_persisted_limits_fall_back_to_defaults() {
        let temp = tempfile::tempdir().expect("temp config dir");
        std::fs::write(
            temp.path().join(crate::settings::store::SETTINGS_FILE_NAME),
            r#"{ "version": 1, "workspace": { "resources": { "serviceIdleTimeoutSecs": 0 } } }"#,
        )
        .expect("write settings");
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));

        let store = ResourcePolicyStore::from_config(config).expect("defaults replace invalid values");
        assert_eq!(
            store.configured_limits(),
            WorkspaceServiceResourceLimits::default()
        );
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
        let persisted: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join(crate::settings::store::SETTINGS_FILE_NAME))
                .expect("read persisted settings"),
        )
        .expect("parse persisted settings");
        assert_eq!(persisted["workspace"]["resources"]["lspIdleTimeoutSecs"], 333);
        assert_eq!(store.configured_metrics().revision, 1);
    }

    #[test]
    fn persistence_failure_keeps_snapshot_and_watch_unchanged() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let settings_path = temp.path().join(crate::settings::store::SETTINGS_FILE_NAME);
        std::fs::remove_file(&settings_path).expect("remove settings file");
        std::fs::create_dir(&settings_path).expect("make the settings target unwritable");
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
            vec!["lspIdleTimeoutSecs"]
        );
        assert_eq!(store.snapshot(), before);
        assert!(!updates.has_changed().expect("watch state"));
    }

    #[test]
    fn republish_follows_settings_written_elsewhere() {
        let temp = tempfile::tempdir().expect("temp config dir");
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let store = ResourcePolicyStore::from_config(config.clone()).expect("valid defaults");

        assert_eq!(store.republish_from_config().revision, 0);
        config
            .settings()
            .set(crate::settings::WORKSPACE_RESOURCES_LSP_IDLE_TIMEOUT_SECS, 333)
            .expect("generic write");
        let next = store.republish_from_config();
        assert_eq!(next.revision, 1);
        assert_eq!(next.limits.lsp_idle_timeout_secs, 333);
        assert_eq!(store.snapshot(), next);
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
        assert_eq!(store.configured_limits().lsp_idle_timeout_secs, 333);
    }
}
