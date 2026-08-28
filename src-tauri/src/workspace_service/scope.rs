use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::identity::CheckoutId;
use super::runtime::{WorkspaceLease, WorkspaceRuntime};

/// Stable checkout identity carried by workspace-scoped requests.
///
/// `expected_generation` lets callers reject a runtime that was evicted and
/// recreated between reading its descriptor and issuing a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    pub checkout_id: CheckoutId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_generation: Option<u64>,
}

impl WorkspaceRef {
    pub fn new(checkout_id: CheckoutId, expected_generation: Option<u64>) -> Self {
        Self {
            checkout_id,
            expected_generation,
        }
    }

    pub fn for_runtime(runtime: &WorkspaceRuntime) -> Self {
        Self::new(runtime.checkout_id().clone(), Some(runtime.generation()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceResolveError {
    RegistryUnavailable {
        detail: String,
    },
    CheckoutUnavailable {
        checkout_id: CheckoutId,
    },
    StaleGeneration {
        checkout_id: CheckoutId,
        expected_generation: u64,
        actual_generation: u64,
    },
}

impl fmt::Display for WorkspaceResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryUnavailable { detail } => {
                write!(formatter, "workspace registry is unavailable: {detail}")
            }
            Self::CheckoutUnavailable { checkout_id } => {
                write!(formatter, "checkout '{checkout_id}' is not registered")
            }
            Self::StaleGeneration {
                checkout_id,
                expected_generation,
                actual_generation,
            } => write!(
                formatter,
                "checkout '{checkout_id}' runtime generation is stale (expected {expected_generation}, actual {actual_generation})"
            ),
        }
    }
}

impl std::error::Error for WorkspaceResolveError {}

/// Resolved runtime plus the lease that keeps it registered for the lifetime
/// of the scoped operation.
pub struct ResolvedWorkspaceScope {
    runtime: Arc<WorkspaceRuntime>,
    lease: WorkspaceLease,
}

impl ResolvedWorkspaceScope {
    pub(crate) fn new(runtime: Arc<WorkspaceRuntime>, lease: WorkspaceLease) -> Self {
        Self { runtime, lease }
    }

    pub fn runtime(&self) -> &Arc<WorkspaceRuntime> {
        &self.runtime
    }

    pub fn workspace_ref(&self) -> WorkspaceRef {
        WorkspaceRef::for_runtime(&self.runtime)
    }

    pub fn into_parts(self) -> (Arc<WorkspaceRuntime>, WorkspaceLease) {
        (self.runtime, self.lease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_ref_uses_camel_case_wire_fields() {
        let reference = WorkspaceRef::new(
            CheckoutId::new("checkout-test").expect("checkout id"),
            Some(7),
        );
        let value = serde_json::to_value(reference).expect("serialize workspace ref");
        assert_eq!(value["checkoutId"], "checkout-test");
        assert_eq!(value["expectedGeneration"], 7);
    }
}
