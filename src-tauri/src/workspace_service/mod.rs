pub mod context;
pub mod event;
pub mod execution;
pub mod identity;
pub mod project_resources;
pub mod runtime;
pub mod scope;
pub mod service;
pub mod unity;

pub use context::{
    WindowContextError, WindowContextRegistry, WindowIntentEpochSnapshot,
    WindowPaneWorkspaceContext,
};
pub use execution::AgentExecutionContext;
pub use identity::{CheckoutId, ProjectId, ServiceInstanceId};
pub use project_resources::{
    ProjectCollaborationCheckout, ProjectCollaborationHub, ProjectCollaborationSnapshot,
    ProjectKnowledgeCatalog, ProjectKnowledgeDocument,
};
pub use runtime::{
    ProjectRegistry, WorkspaceActivitySnapshot, WorkspaceKnowledgeOperationStates, WorkspaceLease,
    WorkspaceLeaseKind, WorkspaceRuntime,
};
pub use scope::{ResolvedWorkspaceScope, WorkspaceRef, WorkspaceResolveError};
pub use service::{
    owner_service_for_tool, service_ready_required_for_tool, ServiceBinding, ServiceKind,
    ServiceReadinessError, ServiceReadinessPhase, ServiceReadinessSnapshot, ServiceReadyPermit,
    ServiceStatus, WorkspaceService, WorkspaceServiceStateSnapshot,
};

/// Restore durable service policy for a checkout. Detection defaults are
/// inserted only once, preserving later enabled/policy/local-config changes.
pub fn restore_or_persist_service_settings(
    store: &crate::session::store::SessionStore,
    runtime: &std::sync::Arc<WorkspaceRuntime>,
) -> Result<(), String> {
    let existing = store
        .list_workspace_services(runtime.checkout_id().as_str())?
        .into_iter()
        .map(|record| (record.service_kind.clone(), record))
        .collect::<std::collections::HashMap<_, _>>();
    for kind in runtime.services().detected_kinds() {
        let stable_instance_id =
            ServiceInstanceId::for_service(runtime.checkout_id(), kind.as_str()).to_string();
        if let Some(record) = existing.get(kind.as_str()) {
            if record.service_instance_id != stable_instance_id {
                return Err(format!(
                    "persisted service instance mismatch for checkout {} service {}",
                    runtime.checkout_id(),
                    kind.as_str()
                ));
            }
            let policy = if record.enabled {
                service::ServiceActivationPolicy::parse(&record.activation_policy)?
            } else {
                service::ServiceActivationPolicy::Disabled
            };
            runtime.services().set_activation_policy(kind, policy)?;
            continue;
        }

        let activation_policy = runtime
            .services()
            .activation_policy(kind)
            .unwrap_or(service::ServiceActivationPolicy::Disabled);
        store.upsert_workspace_service(&crate::session::models::WorkspaceServiceRecord {
            checkout_id: runtime.checkout_id().to_string(),
            service_kind: kind.as_str().to_string(),
            service_instance_id: stable_instance_id,
            enabled: activation_policy != service::ServiceActivationPolicy::Disabled,
            activation_policy: activation_policy.as_str().to_string(),
            local_config: serde_json::json!({}),
        })?;
    }
    Ok(())
}
