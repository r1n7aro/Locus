use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::AppHandle;

use crate::config::AppConfig;
use crate::unity_bridge::UnityMonitorHandle;

use super::execution::AgentExecutionContext;
use super::identity::ServiceInstanceId;
use super::runtime::WorkspaceRuntime;
use super::scope::WorkspaceRef;
use super::service::{
    service_ready_required_for_tool, unity_service_tool_names, DetectionResult, PromptFragment,
    ServiceActivationPolicy, ServiceCapabilities, ServiceContextProvider, ServiceFuture,
    ServiceKind, ServiceLeaseTracker, ServiceReadinessError, ServiceReadinessGate,
    ServiceReadinessPhase, ServiceReadinessSnapshot, ServiceReadyPermit, ServiceRuntimeIdentity,
    ServiceStatus, ServiceToolDefinition, ServiceToolProvider, WorkspaceService,
    WorkspaceServiceFactory,
};

const UNITY_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(350);
const UNITY_READY_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnityReadinessObservation {
    phase: ServiceReadinessPhase,
    detail: String,
}

trait UnityReadinessProbe: Send + Sync {
    fn probe<'a>(&'a self, root: &'a str) -> ServiceFuture<'a, UnityReadinessObservation>;
}

struct BridgeUnityReadinessProbe;

impl UnityReadinessProbe for BridgeUnityReadinessProbe {
    fn probe<'a>(&'a self, root: &'a str) -> ServiceFuture<'a, UnityReadinessObservation> {
        Box::pin(async move {
            let probe = crate::unity_bridge::probe_unity_bridge_readiness(root).await;
            let phase = match probe.state {
                crate::unity_bridge::UnityBridgeReadinessState::Starting => {
                    ServiceReadinessPhase::Starting
                }
                crate::unity_bridge::UnityBridgeReadinessState::Connected => {
                    ServiceReadinessPhase::Connected
                }
                crate::unity_bridge::UnityBridgeReadinessState::Ready => {
                    ServiceReadinessPhase::Ready
                }
                crate::unity_bridge::UnityBridgeReadinessState::Reloading => {
                    ServiceReadinessPhase::Reloading
                }
                crate::unity_bridge::UnityBridgeReadinessState::Degraded => {
                    ServiceReadinessPhase::Degraded
                }
            };
            UnityReadinessObservation {
                phase,
                detail: probe.detail,
            }
        })
    }
}

async fn observe_readiness_once(
    root: &str,
    probe: &dyn UnityReadinessProbe,
    readiness: &ServiceReadinessGate,
) -> ServiceReadinessSnapshot {
    let observation = probe.probe(root).await;
    readiness.transition(observation.phase, Some(observation.detail))
}

async fn run_readiness_observer(
    root: String,
    probe: Arc<dyn UnityReadinessProbe>,
    readiness: Arc<ServiceReadinessGate>,
) {
    loop {
        let snapshot = observe_readiness_once(&root, probe.as_ref(), &readiness).await;
        let interval = if snapshot.phase == ServiceReadinessPhase::Ready {
            UNITY_READY_POLL_INTERVAL
        } else {
            UNITY_READINESS_POLL_INTERVAL
        };
        tokio::time::sleep(interval).await;
    }
}

pub struct UnityServiceFactory {
    app_handle: AppHandle,
    config: Arc<AppConfig>,
}

impl UnityServiceFactory {
    pub fn new(app_handle: AppHandle, config: Arc<AppConfig>) -> Self {
        Self { app_handle, config }
    }
}

impl WorkspaceServiceFactory for UnityServiceFactory {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Unity
    }

    fn detect(&self, workspace: &WorkspaceRuntime) -> DetectionResult {
        if crate::unity_bridge::is_unity_project(&workspace.root().to_string_lossy()) {
            DetectionResult::detected(ServiceActivationPolicy::Lazy)
        } else {
            DetectionResult::absent()
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
                ServiceKind::Unity.as_str(),
            ),
            runtime_generation: generation,
        };
        let event_scope = super::event::WorkspaceEventScope::for_service(&workspace, &identity);
        let service: Arc<dyn WorkspaceService> = Arc::new(UnityServiceInstance {
            identity,
            event_scope,
            root: workspace.root().to_path_buf(),
            status: Mutex::new(ServiceStatus::Dormant),
            lifecycle: tokio::sync::Mutex::new(()),
            leases: Arc::new(ServiceLeaseTracker::default()),
            monitor: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            readiness: Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Stopped)),
            readiness_probe: Arc::new(BridgeUnityReadinessProbe),
            readiness_task: tokio::sync::Mutex::new(None),
            app_handle: self.app_handle.clone(),
            config: Arc::clone(&self.config),
        });
        Box::pin(async move { Ok(service) })
    }
}

pub struct UnityServiceInstance {
    identity: ServiceRuntimeIdentity,
    event_scope: super::event::WorkspaceEventScope,
    root: std::path::PathBuf,
    status: Mutex<ServiceStatus>,
    lifecycle: tokio::sync::Mutex<()>,
    leases: Arc<ServiceLeaseTracker>,
    monitor: UnityMonitorHandle,
    readiness: Arc<ServiceReadinessGate>,
    readiness_probe: Arc<dyn UnityReadinessProbe>,
    readiness_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    app_handle: AppHandle,
    config: Arc<AppConfig>,
}

impl UnityServiceInstance {
    fn set_status(&self, status: ServiceStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    fn root_text(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    fn workspace_ref(&self) -> WorkspaceRef {
        WorkspaceRef::new(
            self.identity.checkout_id.clone(),
            Some(self.event_scope.workspace_generation),
        )
    }

    async fn start_readiness_observer(&self) {
        let mut task = self.readiness_task.lock().await;
        if task.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        if let Some(handle) = task.take() {
            let _ = handle.await;
        }
        let root = self.root_text();
        let probe = Arc::clone(&self.readiness_probe);
        let readiness = Arc::clone(&self.readiness);
        *task = Some(tokio::spawn(async move {
            run_readiness_observer(root, probe, readiness).await;
        }));
    }

    async fn stop_readiness_observer(&self, detail: &str) {
        if let Some(handle) = self.readiness_task.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        self.readiness
            .transition(ServiceReadinessPhase::Stopped, Some(detail.to_string()));
    }
}

impl WorkspaceService for UnityServiceInstance {
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
            values: vec![
                "editor_transport".to_string(),
                "unity_ready".to_string(),
                "asset_database".to_string(),
                "state_probe".to_string(),
                "hot_reload".to_string(),
                "csharp_lsp".to_string(),
                "compile_scope".to_string(),
            ],
        }
    }

    fn lease_tracker(&self) -> Arc<ServiceLeaseTracker> {
        Arc::clone(&self.leases)
    }

    fn readiness(&self) -> ServiceReadinessSnapshot {
        self.readiness.snapshot()
    }

    fn await_ready(
        &self,
        timeout: Duration,
    ) -> ServiceFuture<'_, Result<ServiceReadyPermit, ServiceReadinessError>> {
        Box::pin(async move { self.readiness.await_ready(&self.identity, timeout).await })
    }

    fn start(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle.lock().await;
            if self.status() == ServiceStatus::Running {
                return Ok(());
            }
            self.set_status(ServiceStatus::Starting);
            self.readiness.transition(
                ServiceReadinessPhase::Starting,
                Some("Unity service monitor is starting".to_string()),
            );
            let root = self.root_text();
            crate::unity_bridge::bind_workspace_observable_status(&root, &self.event_scope);
            let result = (|| -> Result<(), String> {
                crate::unity_bridge::sync_native_bridge_marker(
                    &root,
                    self.config.unity_native_bridge_enabled(),
                )?;
                crate::unity_bridge::sync_background_hook_marker(
                    &root,
                    self.config.unity_background_hook_enabled(),
                )?;
                crate::unity_bridge::sync_unity_embed_enabled_marker(
                    &root,
                    self.config.unity_embed_enabled(),
                )?;
                Ok(())
            })();
            if let Err(error) = result {
                crate::unity_bridge::unbind_workspace_observable_status(&root, &self.event_scope);
                self.readiness
                    .transition(ServiceReadinessPhase::Degraded, Some(error.clone()));
                self.set_status(ServiceStatus::Failed);
                return Err(error);
            }
            crate::unity_bridge::set_service_event_scope(&root, Some(self.event_scope.clone()));
            if self.config.unity_embed_enabled() {
                crate::commands::ensure_unity_embed_control_server_for_scope(
                    self.app_handle.clone(),
                    self.workspace_ref(),
                    root.clone(),
                );
            }
            crate::unity_bridge::start_unity_monitor(
                self.app_handle.clone(),
                root.clone(),
                &self.monitor,
                self.event_scope.clone(),
            )
            .await;
            crate::unity_bridge::emit_plugin_status_scoped(
                &self.app_handle,
                &root,
                &self.event_scope,
            );
            crate::csharp_lsp::warm_up_in_background(root);
            self.start_readiness_observer().await;
            // Running means the checkout monitor and lifecycle integration are
            // active. Editor commands still cross `await_ready`.
            self.set_status(ServiceStatus::Running);
            Ok(())
        })
    }

    fn suspend(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle.lock().await;
            if matches!(
                self.status(),
                ServiceStatus::Dormant | ServiceStatus::Stopped
            ) {
                return Ok(());
            }
            if self.leases.count() > 0 {
                return Err("Unity service has active leases".to_string());
            }
            self.set_status(ServiceStatus::Suspending);
            self.stop_readiness_observer("Unity service suspended")
                .await;
            crate::unity_bridge::stop_unity_monitor_for_project(&self.monitor, &self.root_text())
                .await;
            crate::commands::stop_unity_embed_control_server(&self.workspace_ref());
            crate::unity_bridge::unbind_workspace_observable_status(
                &self.root_text(),
                &self.event_scope,
            );
            crate::unity_bridge::set_service_event_scope(&self.root_text(), None);
            if let Err(error) = crate::csharp_lsp::stop_workspace(&self.root_text()).await {
                eprintln!("[Locus] failed to stop checkout LSP: {error}");
            }
            if let Err(error) =
                crate::csharp_compile::release_scopes_for_checkout(&self.identity.checkout_id).await
            {
                eprintln!("[Locus] failed to release checkout compile scopes: {error}");
            }
            crate::csharp_compile::params::invalidate(&self.root_text()).await;
            self.set_status(ServiceStatus::Dormant);
            Ok(())
        })
    }

    fn stop(&self) -> ServiceFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _lifecycle = self.lifecycle.lock().await;
            if self.status() == ServiceStatus::Stopped {
                return Ok(());
            }
            if self.leases.count() > 0 {
                return Err("Unity service has active leases".to_string());
            }
            self.set_status(ServiceStatus::Stopping);
            self.stop_readiness_observer("Unity service stopped").await;
            let root = self.root_text();
            crate::unity_bridge::stop_unity_monitor_for_project(&self.monitor, &root).await;
            crate::unity_bridge::disconnect_with_reason(&root, "workspace service stopped").await;
            crate::commands::stop_unity_embed_control_server(&self.workspace_ref());
            crate::unity_bridge::unbind_workspace_observable_status(&root, &self.event_scope);
            crate::unity_bridge::set_service_event_scope(&root, None);
            if let Err(error) = crate::csharp_lsp::stop_workspace(&root).await {
                eprintln!("[Locus] failed to stop checkout LSP: {error}");
            }
            if let Err(error) =
                crate::csharp_compile::release_scopes_for_checkout(&self.identity.checkout_id).await
            {
                eprintln!("[Locus] failed to release checkout compile scopes: {error}");
            }
            crate::csharp_compile::params::invalidate(&root).await;
            self.set_status(ServiceStatus::Stopped);
            Ok(())
        })
    }

    fn tool_provider(&self) -> Arc<dyn ServiceToolProvider> {
        Arc::new(UnityServiceToolProvider {
            service_instance_id: self.identity.service_instance_id.clone(),
        })
    }

    fn context_provider(&self) -> Arc<dyn ServiceContextProvider> {
        Arc::new(UnityServiceContextProvider {
            identity: self.identity.clone(),
            root: self.root.clone(),
        })
    }
}

struct UnityServiceToolProvider {
    service_instance_id: ServiceInstanceId,
}

impl ServiceToolProvider for UnityServiceToolProvider {
    fn tool_definitions(&self) -> Vec<ServiceToolDefinition> {
        unity_service_tool_names()
            .iter()
            .map(|name| ServiceToolDefinition {
                name: (*name).to_string(),
                owner_service: ServiceKind::Unity,
                required_capabilities: service_ready_required_for_tool(name)
                    .then(|| vec!["unity_ready".to_string()])
                    .unwrap_or_default(),
                resource_locks: vec![format!("service:{}", self.service_instance_id)],
            })
            .collect()
    }
}

struct UnityServiceContextProvider {
    identity: ServiceRuntimeIdentity,
    root: std::path::PathBuf,
}

impl ServiceContextProvider for UnityServiceContextProvider {
    fn prompt_fragments(&self, execution: &AgentExecutionContext) -> Vec<PromptFragment> {
        if execution.checkout_id != self.identity.checkout_id {
            return Vec::new();
        }
        vec![PromptFragment {
            id: "unity-service-binding".to_string(),
            content: format!(
                "Unity service binding: checkout={}, generation={}, root={}",
                self.identity.checkout_id,
                self.identity.runtime_generation,
                self.root.display()
            ),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct ScriptedReadinessProbe {
        observations: Mutex<VecDeque<UnityReadinessObservation>>,
    }

    impl ScriptedReadinessProbe {
        fn new(phases: impl IntoIterator<Item = ServiceReadinessPhase>) -> Self {
            Self {
                observations: Mutex::new(
                    phases
                        .into_iter()
                        .map(|phase| UnityReadinessObservation {
                            phase,
                            detail: format!("scripted {phase:?}"),
                        })
                        .collect(),
                ),
            }
        }
    }

    impl UnityReadinessProbe for ScriptedReadinessProbe {
        fn probe<'a>(&'a self, _root: &'a str) -> ServiceFuture<'a, UnityReadinessObservation> {
            let observation = self
                .observations
                .lock()
                .expect("scripted readiness observations")
                .pop_front()
                .expect("scripted readiness observation");
            Box::pin(async move { observation })
        }
    }

    #[tokio::test]
    async fn injectable_probe_drives_connected_ready_and_reload_transitions() {
        let probe = ScriptedReadinessProbe::new([
            ServiceReadinessPhase::Connected,
            ServiceReadinessPhase::Ready,
            ServiceReadinessPhase::Reloading,
        ]);
        let gate = ServiceReadinessGate::new(ServiceReadinessPhase::Starting);

        assert_eq!(
            observe_readiness_once("A", &probe, &gate).await.phase,
            ServiceReadinessPhase::Connected
        );
        assert_eq!(
            observe_readiness_once("A", &probe, &gate).await.phase,
            ServiceReadinessPhase::Ready
        );
        assert_eq!(
            observe_readiness_once("A", &probe, &gate).await.phase,
            ServiceReadinessPhase::Reloading
        );
    }

    #[test]
    fn only_editor_command_tools_declare_the_ready_capability() {
        let provider = UnityServiceToolProvider {
            service_instance_id: ServiceInstanceId::new("service-test").expect("service id"),
        };
        let tools = provider.tool_definitions();
        let execute = tools
            .iter()
            .find(|tool| tool.name == "unity_execute")
            .expect("unity_execute definition");
        let yaml = tools
            .iter()
            .find(|tool| tool.name == "unity_yaml_read")
            .expect("unity_yaml_read definition");
        assert_eq!(execute.required_capabilities, ["unity_ready"]);
        assert!(yaml.required_capabilities.is_empty());
    }
}
