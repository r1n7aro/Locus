use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::execution::AgentExecutionContext;
use super::identity::{CheckoutId, ProjectId, ServiceInstanceId};
use super::runtime::WorkspaceRuntime;

pub type ServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    Unity,
}

impl ServiceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unity => "unity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceActivationPolicy {
    Disabled,
    Manual,
    Lazy,
    Auto,
}

impl ServiceActivationPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Manual => "manual",
            Self::Lazy => "lazy",
            Self::Auto => "auto",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" => Ok(Self::Disabled),
            "manual" => Ok(Self::Manual),
            "lazy" => Ok(Self::Lazy),
            "auto" => Ok(Self::Auto),
            value => Err(format!(
                "unknown workspace service activation policy '{value}'"
            )),
        }
    }
}

impl std::str::FromStr for ServiceActivationPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Detected,
    Dormant,
    Starting,
    Running,
    Suspending,
    Failed,
    Stopping,
    Stopped,
}

/// Command readiness is intentionally separate from [`ServiceStatus`]. A
/// service can be running (its monitor/process integration is alive) while the
/// checkout-owned command channel is still connecting or crossing a Unity
/// domain reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceReadinessPhase {
    Starting,
    Connected,
    Ready,
    Reloading,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReadinessSnapshot {
    pub phase: ServiceReadinessPhase,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ServiceReadinessSnapshot {
    fn new(phase: ServiceReadinessPhase, revision: u64, detail: Option<String>) -> Self {
        Self {
            phase,
            revision,
            detail,
        }
    }
}

/// A checkout/service-generation-specific proof that the command channel was
/// ready at the point a tool crossed the execution barrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReadyPermit {
    pub service_instance_id: ServiceInstanceId,
    pub checkout_id: CheckoutId,
    pub runtime_generation: u64,
    pub readiness_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceReadinessError {
    #[error(
        "service {service_instance_id} checkout {checkout_id} generation {runtime_generation} did not become ready within {timeout_ms}ms (phase={phase:?}, revision={revision}, detail={detail:?})"
    )]
    Timeout {
        service_instance_id: ServiceInstanceId,
        checkout_id: CheckoutId,
        runtime_generation: u64,
        timeout_ms: u64,
        phase: ServiceReadinessPhase,
        revision: u64,
        detail: Option<String>,
    },
    #[error(
        "service {service_instance_id} checkout {checkout_id} generation {runtime_generation} stopped before becoming ready (revision={revision}, detail={detail:?})"
    )]
    Stopped {
        service_instance_id: ServiceInstanceId,
        checkout_id: CheckoutId,
        runtime_generation: u64,
        revision: u64,
        detail: Option<String>,
    },
}

impl ServiceReadinessError {
    /// Stable JSON diagnostics for IPC/tool output and automated drivers.
    pub fn diagnostic_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.to_string())
    }
}

/// Revisioned readiness barrier shared by one concrete service instance. It is
/// deliberately checkout-local; separate worktrees never share notifications
/// or readiness revisions.
pub(crate) struct ServiceReadinessGate {
    state: Mutex<ServiceReadinessSnapshot>,
    changed: tokio::sync::watch::Sender<u64>,
}

impl ServiceReadinessGate {
    pub(crate) fn new(initial: ServiceReadinessPhase) -> Self {
        let (changed, _) = tokio::sync::watch::channel(1);
        Self {
            state: Mutex::new(ServiceReadinessSnapshot::new(initial, 1, None)),
            changed,
        }
    }

    pub(crate) fn snapshot(&self) -> ServiceReadinessSnapshot {
        self.state
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|_| {
                ServiceReadinessSnapshot::new(
                    ServiceReadinessPhase::Degraded,
                    0,
                    Some("readiness state lock poisoned".to_string()),
                )
            })
    }

    pub(crate) fn transition(
        &self,
        phase: ServiceReadinessPhase,
        detail: impl Into<Option<String>>,
    ) -> ServiceReadinessSnapshot {
        let detail = detail.into();
        let snapshot = match self.state.lock() {
            Ok(mut current) => {
                if current.phase == phase && current.detail == detail {
                    return current.clone();
                }
                current.phase = phase;
                current.revision = current.revision.saturating_add(1);
                current.detail = detail;
                current.clone()
            }
            Err(_) => ServiceReadinessSnapshot::new(
                ServiceReadinessPhase::Degraded,
                0,
                Some("readiness state lock poisoned".to_string()),
            ),
        };
        self.changed.send_replace(snapshot.revision);
        snapshot
    }

    pub(crate) async fn await_ready(
        &self,
        identity: &ServiceRuntimeIdentity,
        timeout: Duration,
    ) -> Result<ServiceReadyPermit, ServiceReadinessError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut changed = self.changed.subscribe();
        loop {
            let snapshot = self.snapshot();
            match snapshot.phase {
                ServiceReadinessPhase::Ready => {
                    return Ok(ServiceReadyPermit {
                        service_instance_id: identity.service_instance_id.clone(),
                        checkout_id: identity.checkout_id.clone(),
                        runtime_generation: identity.runtime_generation,
                        readiness_revision: snapshot.revision,
                    });
                }
                ServiceReadinessPhase::Stopped => {
                    return Err(ServiceReadinessError::Stopped {
                        service_instance_id: identity.service_instance_id.clone(),
                        checkout_id: identity.checkout_id.clone(),
                        runtime_generation: identity.runtime_generation,
                        revision: snapshot.revision,
                        detail: snapshot.detail,
                    });
                }
                _ => {}
            }

            if tokio::time::timeout_at(deadline, changed.changed())
                .await
                .is_err()
            {
                let snapshot = self.snapshot();
                return Err(ServiceReadinessError::Timeout {
                    service_instance_id: identity.service_instance_id.clone(),
                    checkout_id: identity.checkout_id.clone(),
                    runtime_generation: identity.runtime_generation,
                    timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
                    phase: snapshot.phase,
                    revision: snapshot.revision,
                    detail: snapshot.detail,
                });
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionResult {
    pub detected: bool,
    pub activation_policy: ServiceActivationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl DetectionResult {
    pub fn detected(activation_policy: ServiceActivationPolicy) -> Self {
        Self {
            detected: true,
            activation_policy,
            detail: None,
        }
    }

    pub fn absent() -> Self {
        Self {
            detected: false,
            activation_policy: ServiceActivationPolicy::Disabled,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRuntimeIdentity {
    pub project_id: ProjectId,
    pub checkout_id: CheckoutId,
    pub service_instance_id: ServiceInstanceId,
    pub runtime_generation: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCapabilities {
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceToolDefinition {
    pub name: String,
    pub owner_service: ServiceKind,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub resource_locks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFragment {
    pub id: String,
    pub content: String,
}

pub trait ServiceToolProvider: Send + Sync {
    fn tool_definitions(&self) -> Vec<ServiceToolDefinition>;
}

pub trait ServiceContextProvider: Send + Sync {
    fn prompt_fragments(&self, execution: &AgentExecutionContext) -> Vec<PromptFragment>;
}

pub trait WorkspaceServiceFactory: Send + Sync {
    fn kind(&self) -> ServiceKind;
    fn detect(&self, workspace: &WorkspaceRuntime) -> DetectionResult;
    fn create<'a>(
        &'a self,
        workspace: Arc<WorkspaceRuntime>,
        generation: u64,
    ) -> ServiceFuture<'a, Result<Arc<dyn WorkspaceService>, String>>;
}

pub trait WorkspaceService: Send + Sync {
    fn identity(&self) -> ServiceRuntimeIdentity;
    fn status(&self) -> ServiceStatus;
    fn capabilities(&self) -> ServiceCapabilities;
    fn lease_tracker(&self) -> Arc<ServiceLeaseTracker>;

    fn readiness(&self) -> ServiceReadinessSnapshot {
        let phase = match self.status() {
            ServiceStatus::Running => ServiceReadinessPhase::Ready,
            ServiceStatus::Starting => ServiceReadinessPhase::Starting,
            ServiceStatus::Stopping | ServiceStatus::Stopped => ServiceReadinessPhase::Stopped,
            ServiceStatus::Failed => ServiceReadinessPhase::Degraded,
            ServiceStatus::Detected | ServiceStatus::Dormant | ServiceStatus::Suspending => {
                ServiceReadinessPhase::Starting
            }
        };
        ServiceReadinessSnapshot::new(phase, 0, None)
    }

    fn await_ready(
        &self,
        timeout: Duration,
    ) -> ServiceFuture<'_, Result<ServiceReadyPermit, ServiceReadinessError>> {
        let identity = self.identity();
        let snapshot = self.readiness();
        Box::pin(async move {
            match snapshot.phase {
                ServiceReadinessPhase::Ready => Ok(ServiceReadyPermit {
                    service_instance_id: identity.service_instance_id,
                    checkout_id: identity.checkout_id,
                    runtime_generation: identity.runtime_generation,
                    readiness_revision: snapshot.revision,
                }),
                ServiceReadinessPhase::Stopped => Err(ServiceReadinessError::Stopped {
                    service_instance_id: identity.service_instance_id,
                    checkout_id: identity.checkout_id,
                    runtime_generation: identity.runtime_generation,
                    revision: snapshot.revision,
                    detail: snapshot.detail,
                }),
                _ => Err(ServiceReadinessError::Timeout {
                    service_instance_id: identity.service_instance_id,
                    checkout_id: identity.checkout_id,
                    runtime_generation: identity.runtime_generation,
                    timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
                    phase: snapshot.phase,
                    revision: snapshot.revision,
                    detail: snapshot.detail,
                }),
            }
        })
    }

    fn start(&self) -> ServiceFuture<'_, Result<(), String>>;
    fn suspend(&self) -> ServiceFuture<'_, Result<(), String>>;
    fn stop(&self) -> ServiceFuture<'_, Result<(), String>>;

    fn tool_provider(&self) -> Arc<dyn ServiceToolProvider>;
    fn context_provider(&self) -> Arc<dyn ServiceContextProvider>;
}

#[derive(Debug)]
pub struct ServiceLeaseTracker {
    state: AtomicUsize,
    last_used_at: Mutex<Instant>,
}

const SERVICE_LEASE_DRAINING: usize = 1usize << (usize::BITS - 1);
const SERVICE_LEASE_COUNT_MASK: usize = SERVICE_LEASE_DRAINING - 1;

impl Default for ServiceLeaseTracker {
    fn default() -> Self {
        Self {
            state: AtomicUsize::new(0),
            last_used_at: Mutex::new(Instant::now()),
        }
    }
}

impl ServiceLeaseTracker {
    pub fn acquire(self: &Arc<Self>) -> ServiceLease {
        self.try_acquire()
            .expect("cannot acquire a lease while the service is stopping")
    }

    pub fn try_acquire(self: &Arc<Self>) -> Option<ServiceLease> {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current & SERVICE_LEASE_DRAINING != 0
                || current & SERVICE_LEASE_COUNT_MASK == SERVICE_LEASE_COUNT_MASK
            {
                return None;
            }
            match self.state.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        if let Ok(mut last_used_at) = self.last_used_at.lock() {
            *last_used_at = Instant::now();
        }
        Some(ServiceLease {
            tracker: Arc::clone(self),
        })
    }

    pub fn count(&self) -> usize {
        self.state.load(Ordering::Acquire) & SERVICE_LEASE_COUNT_MASK
    }

    pub fn idle_for(&self) -> std::time::Duration {
        self.last_used_at
            .lock()
            .map(|last_used_at| last_used_at.elapsed())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_idle_for_test(&self, idle_for: Duration) {
        *self.last_used_at.lock().expect("service last-used lock") = Instant::now() - idle_for;
    }

    fn begin_stop(self: &Arc<Self>) -> Option<ServiceStopGuard> {
        self.state
            .compare_exchange(
                0,
                SERVICE_LEASE_DRAINING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| ServiceStopGuard {
                tracker: Arc::clone(self),
                committed: false,
            })
    }
}

pub struct ServiceLease {
    tracker: Arc<ServiceLeaseTracker>,
}

impl Drop for ServiceLease {
    fn drop(&mut self) {
        self.tracker.state.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut last_used_at) = self.tracker.last_used_at.lock() {
            *last_used_at = Instant::now();
        }
    }
}

struct ServiceStopGuard {
    tracker: Arc<ServiceLeaseTracker>,
    committed: bool,
}

impl ServiceStopGuard {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ServiceStopGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.tracker.state.store(0, Ordering::Release);
        }
    }
}

#[derive(Clone)]
pub struct ServiceBinding {
    pub service_kind: ServiceKind,
    pub service_instance_id: ServiceInstanceId,
    pub runtime_generation: u64,
    service: Weak<dyn WorkspaceService>,
}

impl ServiceBinding {
    pub fn new(service_kind: ServiceKind, service: &Arc<dyn WorkspaceService>) -> Self {
        let identity = service.identity();
        Self {
            service_kind,
            service_instance_id: identity.service_instance_id,
            runtime_generation: identity.runtime_generation,
            service: Arc::downgrade(service),
        }
    }

    pub fn snapshot(&self) -> ServiceBindingSnapshot {
        ServiceBindingSnapshot {
            service_kind: self.service_kind,
            service_instance_id: self.service_instance_id.clone(),
            runtime_generation: self.runtime_generation,
        }
    }

    pub fn resolve(&self) -> Result<ResolvedServiceBinding, ServiceBindingError> {
        let service = self
            .service
            .upgrade()
            .ok_or_else(|| ServiceBindingError::Stale {
                service_instance_id: self.service_instance_id.clone(),
                expected_generation: self.runtime_generation,
                actual_generation: None,
            })?;
        let identity = service.identity();
        if identity.service_instance_id != self.service_instance_id
            || identity.runtime_generation != self.runtime_generation
        {
            return Err(ServiceBindingError::Stale {
                service_instance_id: self.service_instance_id.clone(),
                expected_generation: self.runtime_generation,
                actual_generation: Some(identity.runtime_generation),
            });
        }
        if service.status() != ServiceStatus::Running {
            return Err(ServiceBindingError::Unavailable {
                service_instance_id: self.service_instance_id.clone(),
                generation: self.runtime_generation,
                status: service.status(),
            });
        }
        let lease = service.lease_tracker().try_acquire().ok_or_else(|| {
            ServiceBindingError::Unavailable {
                service_instance_id: self.service_instance_id.clone(),
                generation: self.runtime_generation,
                status: ServiceStatus::Stopping,
            }
        })?;
        if service.status() != ServiceStatus::Running {
            return Err(ServiceBindingError::Unavailable {
                service_instance_id: self.service_instance_id.clone(),
                generation: self.runtime_generation,
                status: service.status(),
            });
        }
        Ok(ResolvedServiceBinding {
            service,
            lease,
            ready_permit: None,
        })
    }

    /// Resolve the immutable service binding, hold its lifecycle lease, and
    /// wait for the checkout-owned command channel to cross its readiness
    /// barrier. The returned permit stays attached to the resolved binding for
    /// the duration of the tool call.
    pub async fn resolve_ready(
        &self,
        timeout: Duration,
    ) -> Result<ResolvedServiceBinding, ServiceBindingError> {
        let mut resolved = self.resolve()?;
        let permit = resolved
            .service
            .await_ready(timeout)
            .await
            .map_err(|source| ServiceBindingError::Readiness { source })?;

        let identity = resolved.service.identity();
        if identity.service_instance_id != self.service_instance_id
            || identity.runtime_generation != self.runtime_generation
        {
            return Err(ServiceBindingError::Stale {
                service_instance_id: self.service_instance_id.clone(),
                expected_generation: self.runtime_generation,
                actual_generation: Some(identity.runtime_generation),
            });
        }
        if resolved.service.status() != ServiceStatus::Running {
            return Err(ServiceBindingError::Unavailable {
                service_instance_id: self.service_instance_id.clone(),
                generation: self.runtime_generation,
                status: resolved.service.status(),
            });
        }
        resolved.ready_permit = Some(permit);
        Ok(resolved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceBindingSnapshot {
    pub service_kind: ServiceKind,
    pub service_instance_id: ServiceInstanceId,
    pub runtime_generation: u64,
}

/// UI/IPC projection for one checkout-owned service. `status` reports the
/// host lifecycle; `readiness` reports whether editor commands can run. This
/// keeps a connected Unity editor visible while tools continue waiting for
/// `Ready`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServiceStateSnapshot {
    pub service_kind: ServiceKind,
    pub activation_policy: ServiceActivationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ServiceRuntimeIdentity>,
    pub status: ServiceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ServiceReadinessSnapshot>,
    pub lease_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceBindingError {
    #[error("run has no binding for service {kind:?}")]
    Missing { kind: ServiceKind },
    #[error(
        "stale service binding for {service_instance_id}: expected generation {expected_generation}, actual {actual_generation:?}"
    )]
    Stale {
        service_instance_id: ServiceInstanceId,
        expected_generation: u64,
        actual_generation: Option<u64>,
    },
    #[error("service {service_instance_id} generation {generation} is unavailable ({status:?})")]
    Unavailable {
        service_instance_id: ServiceInstanceId,
        generation: u64,
        status: ServiceStatus,
    },
    #[error("service readiness barrier rejected command: {source}")]
    Readiness {
        #[source]
        source: ServiceReadinessError,
    },
}

impl ServiceBindingError {
    pub fn diagnostic_json(&self) -> Option<String> {
        match self {
            Self::Readiness { source } => Some(source.diagnostic_json()),
            _ => None,
        }
    }
}

pub struct ResolvedServiceBinding {
    pub service: Arc<dyn WorkspaceService>,
    lease: ServiceLease,
    ready_permit: Option<ServiceReadyPermit>,
}

impl ResolvedServiceBinding {
    pub fn lease(&self) -> &ServiceLease {
        &self.lease
    }

    pub fn ready_permit(&self) -> Option<&ServiceReadyPermit> {
        self.ready_permit.as_ref()
    }
}

struct ServiceSlotState {
    generation: u64,
    service: Option<Arc<dyn WorkspaceService>>,
}

struct ServiceSlot {
    factory: Arc<dyn WorkspaceServiceFactory>,
    activation_policy: RwLock<ServiceActivationPolicy>,
    state: tokio::sync::Mutex<ServiceSlotState>,
}

#[derive(Clone)]
struct CurrentServiceSnapshot {
    identity: ServiceRuntimeIdentity,
    service: Weak<dyn WorkspaceService>,
}

static NEXT_SERVICE_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_service_generation() -> u64 {
    NEXT_SERVICE_GENERATION
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(1)
        })
        .expect("workspace service generation space exhausted")
}

pub struct WorkspaceServiceHost {
    factories: Vec<Arc<dyn WorkspaceServiceFactory>>,
    slots: Mutex<HashMap<ServiceKind, Arc<ServiceSlot>>>,
    current_services: Mutex<HashMap<ServiceKind, CurrentServiceSnapshot>>,
}

impl WorkspaceServiceHost {
    pub fn new(factories: Vec<Arc<dyn WorkspaceServiceFactory>>) -> Self {
        Self {
            factories,
            slots: Mutex::new(HashMap::new()),
            current_services: Mutex::new(HashMap::new()),
        }
    }

    pub fn detect(&self, workspace: &WorkspaceRuntime) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        for factory in &self.factories {
            let detection = factory.detect(workspace);
            if !detection.detected {
                continue;
            }
            slots.entry(factory.kind()).or_insert_with(|| {
                Arc::new(ServiceSlot {
                    factory: Arc::clone(factory),
                    activation_policy: RwLock::new(detection.activation_policy),
                    state: tokio::sync::Mutex::new(ServiceSlotState {
                        generation: 0,
                        service: None,
                    }),
                })
            });
        }
    }

    pub fn detected_kinds(&self) -> Vec<ServiceKind> {
        self.slots
            .lock()
            .map(|slots| slots.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn activation_policy(&self, kind: ServiceKind) -> Option<ServiceActivationPolicy> {
        self.slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&kind).cloned())
            .and_then(|slot| slot.activation_policy.read().ok().map(|policy| *policy))
    }

    pub fn set_activation_policy(
        &self,
        kind: ServiceKind,
        policy: ServiceActivationPolicy,
    ) -> Result<(), String> {
        let slot = self
            .slots
            .lock()
            .map_err(|error| format!("workspace service slots lock poisoned: {error}"))?
            .get(&kind)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "service '{}' is not detected for this checkout",
                    kind.as_str()
                )
            })?;
        *slot.activation_policy.write().map_err(|error| {
            format!("workspace service activation policy lock poisoned: {error}")
        })? = policy;
        Ok(())
    }

    pub fn is_current_identity(&self, identity: &ServiceRuntimeIdentity) -> bool {
        self.current_services.lock().ok().is_some_and(|services| {
            services.values().any(|current| {
                &current.identity == identity
                    && current.service.upgrade().is_some_and(|service| {
                        matches!(
                            service.status(),
                            ServiceStatus::Starting | ServiceStatus::Running
                        )
                    })
            })
        })
    }

    fn remember_current(&self, kind: ServiceKind, service: &Arc<dyn WorkspaceService>) {
        if let Ok(mut current) = self.current_services.lock() {
            current.insert(
                kind,
                CurrentServiceSnapshot {
                    identity: service.identity(),
                    service: Arc::downgrade(service),
                },
            );
        }
    }

    fn forget_current(&self, kind: ServiceKind, identity: &ServiceRuntimeIdentity) {
        if let Ok(mut current) = self.current_services.lock() {
            if current
                .get(&kind)
                .is_some_and(|snapshot| &snapshot.identity == identity)
            {
                current.remove(&kind);
            }
        }
    }

    pub async fn bind(
        &self,
        workspace: Arc<WorkspaceRuntime>,
        kind: ServiceKind,
        allow_start: bool,
    ) -> Result<ServiceBinding, String> {
        let slot = self
            .slots
            .lock()
            .map_err(|error| format!("workspace service slots lock poisoned: {error}"))?
            .get(&kind)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "service '{}' is not detected for this checkout",
                    kind.as_str()
                )
            })?;
        let activation_policy = *slot.activation_policy.read().map_err(|error| {
            format!("workspace service activation policy lock poisoned: {error}")
        })?;
        if activation_policy == ServiceActivationPolicy::Disabled {
            return Err(format!("service '{}' is disabled", kind.as_str()));
        }

        // Holding the per-slot lock across create/start deliberately merges
        // concurrent cold starts into one lifecycle operation.
        let mut state = slot.state.lock().await;
        if let Some(service) = state.service.as_ref() {
            if service.status() == ServiceStatus::Running {
                self.remember_current(kind, service);
                return Ok(ServiceBinding::new(kind, service));
            }
            if service.lease_tracker().count() > 0 {
                return Err(format!(
                    "service '{}' is busy with active leases",
                    kind.as_str()
                ));
            }
        }
        if !allow_start {
            return Err(format!("service '{}' is dormant", kind.as_str()));
        }

        let generation = next_service_generation();
        let service = slot
            .factory
            .create(workspace, generation)
            .await
            .map_err(|error| format!("failed to create '{}' service: {error}", kind.as_str()))?;
        self.remember_current(kind, &service);
        if let Err(error) = service.start().await {
            self.forget_current(kind, &service.identity());
            return Err(format!(
                "failed to start '{}' service: {error}",
                kind.as_str()
            ));
        }
        state.generation = generation;
        state.service = Some(Arc::clone(&service));
        self.remember_current(kind, &service);
        Ok(ServiceBinding::new(kind, &service))
    }

    pub async fn stop(&self, kind: ServiceKind) -> Result<(), String> {
        let slot = self
            .slots
            .lock()
            .map_err(|error| format!("workspace service slots lock poisoned: {error}"))?
            .get(&kind)
            .cloned();
        let Some(slot) = slot else {
            return Ok(());
        };
        let mut state = slot.state.lock().await;
        let Some(service) = state.service.as_ref().cloned() else {
            return Ok(());
        };
        let identity = service.identity();
        let Some(stop_guard) = service.lease_tracker().begin_stop() else {
            return Err(format!(
                "service '{}' is busy with active leases",
                kind.as_str()
            ));
        };
        self.forget_current(kind, &identity);
        if let Err(error) = service.stop().await {
            drop(stop_guard);
            if service.status() == ServiceStatus::Running {
                self.remember_current(kind, &service);
            }
            return Err(error);
        }
        state.service = None;
        stop_guard.commit();
        Ok(())
    }

    pub async fn is_running(&self, kind: ServiceKind) -> bool {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&kind).cloned());
        let Some(slot) = slot else {
            return false;
        };
        let state = slot.state.lock().await;
        state
            .service
            .as_ref()
            .is_some_and(|service| service.status() == ServiceStatus::Running)
    }

    pub async fn state_snapshot(&self, kind: ServiceKind) -> Option<WorkspaceServiceStateSnapshot> {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&kind).cloned())?;
        let activation_policy = slot
            .activation_policy
            .read()
            .map(|policy| *policy)
            .unwrap_or(ServiceActivationPolicy::Disabled);
        let state = slot.state.lock().await;
        let service = state.service.as_ref();
        Some(WorkspaceServiceStateSnapshot {
            service_kind: kind,
            activation_policy,
            identity: service.map(|service| service.identity()),
            status: service
                .map(|service| service.status())
                .unwrap_or(ServiceStatus::Dormant),
            readiness: service.map(|service| service.readiness()),
            lease_count: service
                .map(|service| service.lease_tracker().count())
                .unwrap_or_default(),
        })
    }

    pub async fn state_snapshots(&self) -> Vec<WorkspaceServiceStateSnapshot> {
        let mut kinds = self.detected_kinds();
        kinds.sort();
        let mut snapshots = Vec::with_capacity(kinds.len());
        for kind in kinds {
            if let Some(snapshot) = self.state_snapshot(kind).await {
                snapshots.push(snapshot);
            }
        }
        snapshots
    }

    pub async fn running_snapshot(
        &self,
    ) -> Vec<(
        ServiceKind,
        ServiceRuntimeIdentity,
        usize,
        std::time::Duration,
    )> {
        let slots = self
            .slots
            .lock()
            .map(|slots| {
                slots
                    .iter()
                    .map(|(kind, slot)| (*kind, slot.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut result = Vec::new();
        for (kind, slot) in slots {
            let state = slot.state.lock().await;
            if let Some(service) = state.service.as_ref() {
                if service.status() == ServiceStatus::Running {
                    let tracker = service.lease_tracker();
                    result.push((
                        kind,
                        service.identity(),
                        tracker.count(),
                        tracker.idle_for(),
                    ));
                }
            }
        }
        result
    }
}

pub fn unity_service_tool_names() -> &'static [&'static str] {
    &[
        "unity_asset_search",
        "unity_set_play_mode",
        "unity_execute",
        "unity_run_states",
        "unity_capture_viewport",
        "unity_get_console_log",
        "unity_test_list",
        "unity_test_run",
        "unity_recompile",
        "unity_hot_reload",
        "unity_ref_search",
        "unity_code_usages",
        "unity_yaml_search",
        "unity_yaml_read",
    ]
}

pub fn owner_service_for_tool(name: &str) -> Option<ServiceKind> {
    unity_service_tool_names()
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name.trim()))
        .then_some(ServiceKind::Unity)
}

/// Unity tools that cross the editor command channel. Checkout-local index and
/// YAML searches still keep a Unity service binding for ownership/teardown,
/// while they can run during an editor reload.
pub fn service_ready_required_for_tool(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "unity_set_play_mode"
            | "unity_execute"
            | "unity_run_states"
            | "unity_capture_viewport"
            | "unity_get_console_log"
            | "unity_test_list"
            | "unity_test_run"
            | "unity_recompile"
            | "unity_hot_reload"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity(checkout: &str, generation: u64) -> ServiceRuntimeIdentity {
        let checkout_id = CheckoutId::new(checkout).expect("checkout id");
        ServiceRuntimeIdentity {
            project_id: ProjectId::new("project-ready-test").expect("project id"),
            service_instance_id: ServiceInstanceId::for_service(&checkout_id, "unity"),
            checkout_id,
            runtime_generation: generation,
        }
    }

    #[test]
    fn activation_policy_parses_persisted_values() {
        assert_eq!(
            ServiceActivationPolicy::parse(" Lazy ").expect("lazy policy"),
            ServiceActivationPolicy::Lazy
        );
        assert_eq!(
            "AUTO"
                .parse::<ServiceActivationPolicy>()
                .expect("auto policy"),
            ServiceActivationPolicy::Auto
        );
        assert!(ServiceActivationPolicy::parse("sometimes").is_err());
    }

    #[test]
    fn stop_guard_excludes_new_service_leases_atomically() {
        let tracker = Arc::new(ServiceLeaseTracker::default());
        let lease = tracker.try_acquire().expect("initial lease");
        assert!(tracker.begin_stop().is_none());
        drop(lease);

        let stop_guard = tracker.begin_stop().expect("stop guard");
        assert!(tracker.try_acquire().is_none());
        drop(stop_guard);
        assert!(tracker.try_acquire().is_some());
    }

    #[test]
    fn service_idle_age_starts_when_the_last_lease_is_released() {
        let tracker = Arc::new(ServiceLeaseTracker::default());
        let lease = tracker.try_acquire().expect("service lease");
        tracker.set_idle_for_test(Duration::from_secs(7200));

        drop(lease);

        assert!(tracker.idle_for() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn connected_without_command_readiness_keeps_waiter_blocked_until_ready() {
        let gate = Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Starting));
        let identity = test_identity("checkout-connected", 7);
        gate.transition(
            ServiceReadinessPhase::Connected,
            Some("pipe connected; managed domain initializing".to_string()),
        );

        let wait_gate = Arc::clone(&gate);
        let wait_identity = identity.clone();
        let waiter = tokio::spawn(async move {
            wait_gate
                .await_ready(&wait_identity, Duration::from_secs(2))
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        let ready = gate.transition(
            ServiceReadinessPhase::Ready,
            Some("managed domain and Unity main thread ready".to_string()),
        );
        let permit = waiter.await.expect("waiter task").expect("ready permit");
        assert_eq!(permit.checkout_id, identity.checkout_id);
        assert_eq!(permit.runtime_generation, 7);
        assert_eq!(permit.readiness_revision, ready.revision);
    }

    #[tokio::test]
    async fn reload_resets_barrier_and_a_later_ready_transition_releases_it() {
        let gate = Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Ready));
        let identity = test_identity("checkout-reload", 11);
        gate.await_ready(&identity, Duration::from_millis(20))
            .await
            .expect("initial ready permit");

        gate.transition(
            ServiceReadinessPhase::Reloading,
            Some("domain reload in progress".to_string()),
        );
        let error = gate
            .await_ready(&identity, Duration::from_millis(20))
            .await
            .expect_err("reload must close the barrier");
        assert!(matches!(
            error,
            ServiceReadinessError::Timeout {
                phase: ServiceReadinessPhase::Reloading,
                ..
            }
        ));

        gate.transition(
            ServiceReadinessPhase::Ready,
            Some("new managed domain ready".to_string()),
        );
        gate.await_ready(&identity, Duration::from_millis(20))
            .await
            .expect("post-reload ready permit");
    }

    #[tokio::test]
    async fn readiness_timeout_is_checkout_scoped_and_machine_diagnostic() {
        let gate = ServiceReadinessGate::new(ServiceReadinessPhase::Connected);
        let identity = test_identity("checkout-timeout", 19);
        let error = gate
            .await_ready(&identity, Duration::from_millis(15))
            .await
            .expect_err("connected must time out without ready");
        let diagnostic = error.diagnostic_json();
        assert!(diagnostic.contains("checkout-timeout"));
        assert!(diagnostic.contains("\"kind\":\"timeout\""));
        assert!(matches!(
            error,
            ServiceReadinessError::Timeout {
                runtime_generation: 19,
                phase: ServiceReadinessPhase::Connected,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn readiness_barriers_are_independent_across_checkouts() {
        let gate_a = Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Connected));
        let gate_b = Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Connected));
        let identity_a = test_identity("checkout-a", 23);
        let identity_b = test_identity("checkout-b", 29);

        gate_a.transition(ServiceReadinessPhase::Ready, Some("A ready".to_string()));
        let permit_a = gate_a
            .await_ready(&identity_a, Duration::from_millis(20))
            .await
            .expect("A ready permit");
        assert_eq!(permit_a.checkout_id, identity_a.checkout_id);

        let error_b = gate_b
            .await_ready(&identity_b, Duration::from_millis(15))
            .await
            .expect_err("B remains connected-only");
        assert!(matches!(
            error_b,
            ServiceReadinessError::Timeout {
                phase: ServiceReadinessPhase::Connected,
                ..
            }
        ));
    }
}
