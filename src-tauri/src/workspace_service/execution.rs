use std::collections::HashMap;
use std::sync::Arc;

use super::identity::{CheckoutId, ProjectId};
use super::runtime::{WorkspaceLease, WorkspaceLeaseKind, WorkspaceRuntime};
use super::service::{
    ResolvedServiceBinding, ServiceBinding, ServiceBindingError, ServiceBindingSnapshot,
    ServiceKind,
};

/// Immutable checkout and service routing snapshot for one Agent run.
pub struct AgentExecutionContext {
    pub project_id: ProjectId,
    pub checkout_id: CheckoutId,
    pub workspace: Arc<WorkspaceRuntime>,
    pub workspace_generation: u64,
    pub service_bindings: HashMap<ServiceKind, ServiceBinding>,
    _workspace_lease: WorkspaceLease,
}

impl AgentExecutionContext {
    pub(crate) fn new(
        workspace: Arc<WorkspaceRuntime>,
        service_bindings: HashMap<ServiceKind, ServiceBinding>,
    ) -> Self {
        let workspace_lease = workspace.acquire_lease(WorkspaceLeaseKind::RunningTask);
        Self {
            project_id: workspace.project_id().clone(),
            checkout_id: workspace.checkout_id().clone(),
            workspace_generation: workspace.generation(),
            workspace,
            service_bindings,
            _workspace_lease: workspace_lease,
        }
    }

    pub fn root(&self) -> &std::path::Path {
        self.workspace.root()
    }

    pub fn binding(&self, kind: ServiceKind) -> Option<&ServiceBinding> {
        self.service_bindings.get(&kind)
    }

    pub fn resolve_service(
        &self,
        kind: ServiceKind,
    ) -> Result<ResolvedServiceBinding, ServiceBindingError> {
        self.service_bindings
            .get(&kind)
            .ok_or_else(|| ServiceBindingError::Missing { kind })?
            .resolve()
    }

    pub async fn resolve_service_ready(
        &self,
        kind: ServiceKind,
        timeout: std::time::Duration,
    ) -> Result<ResolvedServiceBinding, ServiceBindingError> {
        self.service_bindings
            .get(&kind)
            .ok_or_else(|| ServiceBindingError::Missing { kind })?
            .resolve_ready(timeout)
            .await
    }

    pub fn service_binding_snapshots(&self) -> Vec<ServiceBindingSnapshot> {
        let mut snapshots = self
            .service_bindings
            .values()
            .map(ServiceBinding::snapshot)
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.service_kind);
        snapshots
    }

    pub fn persisted_run_scope(&self) -> crate::session::models::SessionRunScopeSnapshot {
        let head = crate::commands::collect_head_state(&self.root().to_string_lossy());
        crate::session::models::SessionRunScopeSnapshot {
            project_id: self.project_id.to_string(),
            checkout_id: self.checkout_id.to_string(),
            workspace_generation: self.workspace_generation,
            materialization_epoch: Some(self.workspace.materialization_epoch()),
            branch_ref: head.ref_name,
            head_oid: head.hash,
            service_bindings: self
                .service_binding_snapshots()
                .into_iter()
                .map(|binding| crate::session::models::SessionRunServiceBinding {
                    service_kind: binding.service_kind.as_str().to_string(),
                    service_instance_id: binding.service_instance_id.to_string(),
                    runtime_generation: binding.runtime_generation,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::models::WorkspaceCheckoutRecord;
    use crate::session::store::SessionStore;

    #[tokio::test]
    async fn session_branch_snapshot_survives_switches_and_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let output = crate::process_util::command("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &temp.path().join("config.json"),
        ));
        let policy =
            Arc::new(crate::resource_policy::ResourcePolicyStore::from_config(config).unwrap());
        let registry = super::super::ProjectRegistry::new(policy, Vec::new());
        let runtime = registry.register(&root).unwrap();
        let execution = registry
            .execution_context(runtime.checkout_id(), &[])
            .await
            .unwrap();
        // Even a newly initialized repository has a branch before its first commit.
        let initial = execution.persisted_run_scope();
        assert_eq!(initial.branch_ref.as_deref(), Some("main"));
        assert_eq!(initial.head_oid, None);
        git(&[
            "-c",
            "user.name=Locus Test",
            "-c",
            "user.email=test@locus.local",
            "commit",
            "--allow-empty",
            "--no-gpg-sign",
            "-m",
            "initial",
        ]);

        let store = SessionStore::new(temp.path()).unwrap();
        let main_scope = execution.persisted_run_scope();
        store
            .upsert_workspace_checkout(&WorkspaceCheckoutRecord {
                checkout_id: main_scope.checkout_id.clone(),
                project_id: main_scope.project_id.clone(),
                root_path: root.to_string_lossy().into_owned(),
                normalized_root: root.to_string_lossy().into_owned(),
                last_opened_at: 1,
            })
            .unwrap();
        let sid = store
            .create_session_scoped(
                "Branch history",
                None,
                Some(&main_scope.project_id),
                Some(&main_scope.checkout_id),
                "chat",
                None,
            )
            .unwrap();
        store
            .try_start_run_scoped(&sid, "main-run", Some(&main_scope))
            .unwrap();
        store.update_run_status("main-run", "done", None).unwrap();

        git(&["switch", "-c", "feature/ui"]);
        let target = store
            .list_sessions(Some(&main_scope.project_id))
            .unwrap()
            .remove(0)
            .execution_target
            .unwrap();
        assert_eq!(target.branch_ref.as_deref(), Some("main"));
        let feature_scope = execution.persisted_run_scope();
        assert_eq!(feature_scope.checkout_id, main_scope.checkout_id);
        assert_eq!(feature_scope.branch_ref.as_deref(), Some("feature/ui"));
        store
            .try_start_run_scoped(&sid, "feature-run", Some(&feature_scope))
            .unwrap();
        store
            .update_run_status("feature-run", "done", None)
            .unwrap();
        git(&["switch", "main"]);
        drop(store);

        let reopened = SessionStore::new(temp.path()).unwrap();
        let target = reopened
            .list_sessions(Some(&main_scope.project_id))
            .unwrap()
            .remove(0)
            .execution_target
            .unwrap();
        assert_eq!(target.branch_ref.as_deref(), Some("feature/ui"));
        let export = temp.path().join("context.yaml");
        crate::session::context_export::export_session_context_yaml(
            &reopened,
            &sid,
            root.to_str().unwrap(),
            None,
            None,
            &export,
        )
        .unwrap();
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(export).unwrap()).unwrap();
        let runs = yaml["sessions"][0]["runs"].as_sequence().unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs
            .iter()
            .any(|run| run["branchRef"].as_str() == Some("main")));
        assert!(runs
            .iter()
            .any(|run| run["branchRef"].as_str() == Some("feature/ui")));
    }
}
