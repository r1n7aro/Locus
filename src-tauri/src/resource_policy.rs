use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}};

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
    active_sessions: AtomicUsize,
    session_notify: tokio::sync::Notify,
}

static APPLICATION_POLICY: OnceLock<Arc<ResourcePolicyStore>> = OnceLock::new();

pub fn install_application_policy(policy: Arc<ResourcePolicyStore>) {
    let _ = APPLICATION_POLICY.set(policy);
}

pub fn application_policy() -> Option<&'static Arc<ResourcePolicyStore>> { APPLICATION_POLICY.get() }

/// Existing runs keep their permit when a user lowers the budget. Only new
/// top-level runs queue; child Agents have their independent subagent budget.
pub struct SessionBudgetPermit { policy: ResourcePolicyStore }
impl Drop for SessionBudgetPermit {
    fn drop(&mut self) {
        self.policy.inner.active_sessions.fetch_sub(1, Ordering::AcqRel);
        self.policy.inner.session_notify.notify_waiters();
    }
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
                active_sessions: AtomicUsize::new(0),
                session_notify: tokio::sync::Notify::new(),
            }),
        })
    }

    pub fn snapshot(&self) -> ResourcePolicySnapshot {
        self.inner.snapshot_tx.borrow().clone()
    }

    pub async fn acquire_session(&self, mut cancel: watch::Receiver<bool>) -> Result<SessionBudgetPermit, String> {
        let mut updates = self.subscribe();
        loop {
            if *cancel.borrow() { return Err("Session cancelled while waiting for concurrency capacity".into()); }
            let notified = self.inner.session_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let limit = updates.borrow_and_update().limits.max_running_sessions;
            let active = self.inner.active_sessions.load(Ordering::Acquire);
            if active < limit {
                if self.inner.active_sessions.compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    return Ok(SessionBudgetPermit { policy: self.clone() });
                }
                continue;
            }
            tokio::select! {
                _ = &mut notified => {},
                result = updates.changed() => { if result.is_err() { return Err("Resource policy closed".into()); } },
                result = cancel.changed() => { if result.is_err() || *cancel.borrow() { return Err("Session cancelled while waiting for concurrency capacity".into()); } },
            }
        }
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

    #[tokio::test]
    async fn lowering_session_limit_waits_without_revoking_existing_runs() {
        let temp = tempfile::tempdir().unwrap();
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let policy = ResourcePolicyStore::from_config(config).unwrap();
        let mut limits = policy.configured_limits(); limits.max_running_sessions = 2;
        policy.update(limits.clone()).unwrap();
        let (_cancel, receiver) = watch::channel(false);
        let first = policy.acquire_session(receiver.clone()).await.unwrap();
        let second = policy.acquire_session(receiver.clone()).await.unwrap();
        limits.max_running_sessions = 1; policy.update(limits).unwrap();
        let third = policy.acquire_session(receiver.clone()); tokio::pin!(third);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut third).await.is_err());
        drop(first);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut third).await.is_err());
        drop(second);
        let third = tokio::time::timeout(std::time::Duration::from_secs(1), third).await.unwrap().unwrap();
        assert_eq!(policy.inner.active_sessions.load(Ordering::Acquire), 1);
        drop(third);
        assert_eq!(policy.inner.active_sessions.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn queued_session_reacts_to_budget_increase_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let policy = ResourcePolicyStore::from_config(config).unwrap();
        let mut limits = policy.configured_limits(); limits.max_running_sessions = 1;
        policy.update(limits.clone()).unwrap();
        let (_cancel, receiver) = watch::channel(false);
        let first = policy.acquire_session(receiver.clone()).await.unwrap();
        let (cancel, cancelled) = watch::channel(false);
        let waiting = policy.acquire_session(cancelled); tokio::pin!(waiting);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting).await.is_err());
        cancel.send(true).unwrap();
        assert!(waiting.await.is_err());
        assert_eq!(policy.inner.active_sessions.load(Ordering::Acquire), 1);
        let next = policy.acquire_session(receiver); tokio::pin!(next);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut next).await.is_err());
        limits.max_running_sessions = 2; policy.update(limits).unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), next).await.unwrap().unwrap();
        drop((first, second));
    }

    fn changed_limits() -> WorkspaceServiceResourceLimits {
        WorkspaceServiceResourceLimits {
            max_running_sessions: 6,
            max_unity_editors: 6,
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
