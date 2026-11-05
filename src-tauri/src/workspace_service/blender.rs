//! Blender workspace service. One instance per checkout; it holds a handle on
//! the app-level instance hub (connections to running Blender processes) for
//! as long as it runs, and owns the checkout's background instances.

use std::sync::{Arc, Mutex};

use super::execution::AgentExecutionContext;
use super::identity::ServiceInstanceId;
use super::runtime::WorkspaceRuntime;
use super::service::{
    blender_service_tool_names, DetectionResult, EffectiveServiceMode, PromptFragment,
    ServiceActivationPolicy, ServiceCapabilities, ServiceContextProvider, ServiceFuture,
    ServiceKind, ServiceLeaseTracker, ServiceRuntimeIdentity, ServiceStatus,
    ServiceToolDefinition, ServiceToolProvider, WorkspaceService, WorkspaceServiceFactory,
};

#[derive(Default)]
pub struct BlenderServiceFactory;

impl WorkspaceServiceFactory for BlenderServiceFactory {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Blender
    }

    /// Blender needs nothing in the workspace, so the app switch alone
    /// decides availability. The workspace mode starts from the persisted
    /// record or [`Self::initial_activation_policy`].
    fn detect(&self, _workspace: &WorkspaceRuntime) -> DetectionResult {
        if crate::blender::blender_integration_enabled() {
            DetectionResult::detected(ServiceActivationPolicy::Disabled)
        } else {
            DetectionResult::absent()
        }
    }

    /// Lazy when the workspace contains `.blend` files, otherwise off. The
    /// scan runs once, when the checkout has no persisted record yet.
    fn initial_activation_policy(&self, workspace: &WorkspaceRuntime) -> ServiceActivationPolicy {
        if crate::blender::blender_integration_enabled()
            && crate::blender::detect::workspace_has_blend_files(workspace.root())
        {
            ServiceActivationPolicy::Lazy
        } else {
            ServiceActivationPolicy::Disabled
        }
    }

    fn create<'a>(
        &'a self,
        workspace: Arc<WorkspaceRuntime>,
        generation: u64,
    ) -> ServiceFuture<'a, Result<Arc<dyn WorkspaceService>, String>> {
        let identity = ServiceRuntimeIdentity {
            project_id: workspace.project_id().clone(),
            checkout_id: workspace.checkout_id().clone(),
            service_instance_id: ServiceInstanceId::for_service(
                workspace.checkout_id(),
                ServiceKind::Blender.as_str(),
            ),
            runtime_generation: generation,
        };
        let service: Arc<dyn WorkspaceService> = Arc::new(BlenderServiceInstance {
            identity,
            root: workspace.root().to_path_buf(),
            status: Mutex::new(ServiceStatus::Dormant),
            lifecycle: tokio::sync::Mutex::new(()),
            leases: Arc::new(ServiceLeaseTracker::default()),
            hub: Mutex::new(None),
        });
        Box::pin(async move { Ok(service) })
    }
}

pub struct BlenderServiceInstance {
    identity: ServiceRuntimeIdentity,
    root: std::path::PathBuf,
    status: Mutex<ServiceStatus>,
    lifecycle: tokio::sync::Mutex<()>,
    leases: Arc<ServiceLeaseTracker>,
    /// Keeps the app-level hub alive while this checkout's service runs.
    hub: Mutex<Option<crate::blender::hub::HubHandle>>,
}

impl BlenderServiceInstance {
    fn set_status(&self, status: ServiceStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn checkout_id(&self) -> &super::identity::CheckoutId {
        &self.identity.checkout_id
    }
}

impl WorkspaceService for BlenderServiceInstance {
    fn identity(&self) -> ServiceRuntimeIdentity {
        self.identity.clone()
    }

    fn status(&self) -> ServiceStatus {
        self.status
            .lock()
            .map(|status| *status)
            .unwrap_or(ServiceStatus::Failed)
    }

    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities {
            values: vec!["blender_instances".to_string()],
        }
    }

    fn lease_tracker(&self) -> Arc<ServiceLeaseTracker> {
        Arc::clone(&self.leases)
    }

    fn start(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle.lock().await;
            if self.status() == ServiceStatus::Running {
                return Ok(());
            }
            self.set_status(ServiceStatus::Starting);
            match crate::blender::hub::acquire() {
                Ok(handle) => {
                    if let Ok(mut hub) = self.hub.lock() {
                        *hub = Some(handle);
                    }
                    self.set_status(ServiceStatus::Running);
                    Ok(())
                }
                Err(error) => {
                    self.set_status(ServiceStatus::Failed);
                    Err(error)
                }
            }
        })
    }

    fn suspend(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move { Ok(()) })
    }

    fn stop(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle.lock().await;
            if self.status() == ServiceStatus::Stopped {
                return Ok(());
            }
            if self.leases.count() > 0 {
                return Err("Blender service has active leases".to_string());
            }
            self.set_status(ServiceStatus::Stopping);
            crate::blender::hub::release_checkout(&self.identity.checkout_id).await;
            if let Ok(mut hub) = self.hub.lock() {
                hub.take();
            }
            self.set_status(ServiceStatus::Stopped);
            Ok(())
        })
    }

    fn tool_provider(&self) -> Arc<dyn ServiceToolProvider> {
        Arc::new(BlenderServiceToolProvider {
            service_instance_id: self.identity.service_instance_id.clone(),
        })
    }

    fn context_provider(&self) -> Arc<dyn ServiceContextProvider> {
        Arc::new(BlenderServiceContextProvider)
    }
}

struct BlenderServiceToolProvider {
    service_instance_id: ServiceInstanceId,
}

impl ServiceToolProvider for BlenderServiceToolProvider {
    fn tool_definitions(&self) -> Vec<ServiceToolDefinition> {
        blender_service_tool_names()
            .iter()
            .map(|name| ServiceToolDefinition {
                name: (*name).to_string(),
                owner_service: ServiceKind::Blender,
                required_capabilities: vec!["blender_instances".to_string()],
                resource_locks: vec![format!("service:{}", self.service_instance_id)],
            })
            .collect()
    }
}

/// Injection id of the Blender usage note: the key of its toggle in the
/// agent injection config and the id of its entry in the injected-items list.
pub const PROMPT_INJECTION_ID: &str = "blender_service";

/// Blender usage note for the agent env prompt, chosen by the workspace's
/// effective service mode alone. It must not depend on run bindings: a lazy
/// service binds mid-run, and the prompt prefix has to be identical before
/// and after that. Off adds nothing. Instance details (version, PID, file)
/// travel with each `blender_execute` result instead.
pub fn prompt_fragment(mode: EffectiveServiceMode) -> Option<PromptFragment> {
    let content = match mode {
        EffectiveServiceMode::Off => return None,
        EffectiveServiceMode::Auto => crate::prompt::blender::SERVICE_AUTO,
        EffectiveServiceMode::Lazy => crate::prompt::blender::SERVICE_LAZY,
    };
    Some(PromptFragment {
        id: PROMPT_INJECTION_ID.to_string(),
        content: content.trim().to_string(),
    })
}

/// Adds nothing through the binding path: the usage note comes from
/// [`prompt_fragment`], which exists before a lazy service binds.
struct BlenderServiceContextProvider;

impl ServiceContextProvider for BlenderServiceContextProvider {
    fn prompt_fragments(&self, _execution: &AgentExecutionContext) -> Vec<PromptFragment> {
        Vec::new()
    }
}
