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
