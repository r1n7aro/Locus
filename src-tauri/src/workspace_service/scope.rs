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
    /// Durable checkout assignment identity. Runtime generations can restart
    /// from the same value in another application process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_materialization_epoch: Option<u64>,
}

impl WorkspaceRef {
    pub fn new(checkout_id: CheckoutId, expected_generation: Option<u64>) -> Self {
        Self {
            checkout_id,
            expected_generation,
            expected_materialization_epoch: None,
        }
    }

    pub fn for_runtime(runtime: &WorkspaceRuntime) -> Self {
        Self::new(runtime.checkout_id().clone(), Some(runtime.generation()))
            .with_materialization_epoch(Some(runtime.materialization_epoch()))
    }

    pub fn with_materialization_epoch(mut self, epoch: Option<u64>) -> Self {
        self.expected_materialization_epoch = epoch;
        self
    }

    pub fn validate_materialization_epoch(&self, actual: u64) -> Result<(), WorkspaceResolveError> {
        if self
            .expected_materialization_epoch
            .is_some_and(|expected| expected != actual)
            || (self.expected_materialization_epoch.is_none() && actual > 1)
        {
            return Err(WorkspaceResolveError::StaleMaterialization {
                checkout_id: self.checkout_id.clone(),
                expected_materialization_epoch: self.expected_materialization_epoch,
                actual_materialization_epoch: actual,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceResolveError {
    StaleMaterialization {
        checkout_id: CheckoutId,
        expected_materialization_epoch: Option<u64>,
        actual_materialization_epoch: u64,
    },
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
            Self::StaleMaterialization { checkout_id,expected_materialization_epoch,actual_materialization_epoch } => write!(formatter,
                "checkout '{checkout_id}' assignment is stale (expected {}, actual {actual_materialization_epoch}); reopen the checkout explicitly",
                expected_materialization_epoch.map(|value|value.to_string()).unwrap_or_else(||"empty".into())),
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

    #[test]
    fn materialization_epoch_rejects_old_handles_after_generation_repeats() {
        let old = WorkspaceRef::new(CheckoutId::new("reused-slot").unwrap(), Some(1))
            .with_materialization_epoch(Some(1));
        let roundtrip: WorkspaceRef =
            serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(roundtrip.expected_generation, Some(1));
        assert!(matches!(
            roundtrip.validate_materialization_epoch(2),
            Err(WorkspaceResolveError::StaleMaterialization {
                expected_materialization_epoch: Some(1),
                actual_materialization_epoch: 2,
                ..
            })
        ));
        assert!(roundtrip
            .clone()
            .with_materialization_epoch(Some(2))
            .validate_materialization_epoch(2)
            .is_ok());
    }

    #[test]
    fn legacy_missing_epoch_is_compatible_only_before_slot_reuse() {
        let old: WorkspaceRef = serde_json::from_value(
            serde_json::json!({"checkoutId":"legacy","expectedGeneration":1}),
        )
        .unwrap();
        assert_eq!(old.expected_materialization_epoch, None);
        assert!(old.validate_materialization_epoch(0).is_ok());
        assert!(old.validate_materialization_epoch(1).is_ok());
        assert!(old.validate_materialization_epoch(2).is_err());
        assert!(old
            .clone()
            .with_materialization_epoch(Some(0))
            .validate_materialization_epoch(1)
            .is_err());
    }
}
