use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use super::identity::CheckoutId;
use super::runtime::{WorkspaceLease, WorkspaceLeaseKind, WorkspaceRuntime};
use super::scope::ResolvedWorkspaceScope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPaneWorkspaceContext {
    pub window_id: String,
    pub pane_id: String,
    pub focused_checkout_id: CheckoutId,
    pub workspace_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
    /// Client-owned monotonic mutation sequence for this window/pane. This is
    /// persisted with the projection so a restored renderer can continue
    /// above the last accepted intent.
    #[serde(default)]
    pub intent_epoch: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowIntentEpochSnapshot {
    pub window_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub intent_epoch: u64,
}

struct WindowPaneContextEntry {
    context: WindowPaneWorkspaceContext,
    _workspace_lease: WorkspaceLease,
}

#[derive(Default)]
struct WindowWorkspaceContext {
    panes: HashMap<String, WindowPaneContextEntry>,
    active_pane_id: Option<String>,
    /// Latest accepted intent for every pane, including removed panes. Keeping
    /// these tombstones prevents a delayed focus request from recreating a
    /// pane after detach completed.
    pane_intent_epochs: HashMap<String, u64>,
    /// Latest accepted whole-window detach. It applies to existing and future
    /// pane ids within this window instance.
    tombstone_intent_epoch: u64,
}

#[derive(Default)]
pub struct WindowContextRegistry {
    windows: RwLock<HashMap<String, WindowWorkspaceContext>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowContextError {
    EmptyWindowId,
    EmptyPaneId,
    PaneUnavailable {
        window_id: String,
        pane_id: String,
    },
    InvalidIntentEpoch {
        window_id: String,
        pane_id: String,
    },
    StaleIntent {
        window_id: String,
        pane_id: Option<String>,
        received_epoch: u64,
        current_epoch: u64,
    },
    RevisionExhausted {
        window_id: String,
        pane_id: String,
    },
    LockPoisoned(String),
}

impl fmt::Display for WindowContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWindowId => formatter.write_str("window id cannot be empty"),
            Self::EmptyPaneId => formatter.write_str("pane id cannot be empty"),
            Self::PaneUnavailable { window_id, pane_id } => write!(
                formatter,
                "workspace context is unavailable for window '{window_id}' pane '{pane_id}'"
            ),
            Self::InvalidIntentEpoch { window_id, pane_id } => write!(
                formatter,
                "workspace intent epoch must be positive for window '{window_id}' pane '{pane_id}'"
            ),
            Self::StaleIntent {
                window_id,
                pane_id,
                received_epoch,
                current_epoch,
            } => write!(
                formatter,
                "stale workspace intent for window '{window_id}'{}: received {received_epoch}, current {current_epoch}",
                pane_id
                    .as_deref()
                    .map(|pane_id| format!(" pane '{pane_id}'"))
                    .unwrap_or_default()
            ),
            Self::RevisionExhausted { window_id, pane_id } => write!(
                formatter,
                "workspace focus revision exhausted for window '{window_id}' pane '{pane_id}'"
            ),
            Self::LockPoisoned(error) => {
                write!(formatter, "window context registry lock poisoned: {error}")
            }
        }
    }
}

impl std::error::Error for WindowContextError {}

impl WindowContextRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update only the UI focus projection for one pane. Runtime lifecycle and
    /// workspace-owned services remain untouched. The pane retains one runtime
    /// lease until it changes focus or is removed.
    pub fn focus(
        &self,
        window_id: &str,
        pane_id: &str,
        runtime: Arc<WorkspaceRuntime>,
        intent_epoch: u64,
    ) -> Result<WindowPaneWorkspaceContext, WindowContextError> {
        let lease = runtime.acquire_lease(WorkspaceLeaseKind::VisiblePane);
        self.focus_with_lease(window_id, pane_id, runtime, lease, intent_epoch)
    }

    pub fn focus_scope(
        &self,
        window_id: &str,
        pane_id: &str,
        scope: ResolvedWorkspaceScope,
        intent_epoch: u64,
    ) -> Result<WindowPaneWorkspaceContext, WindowContextError> {
        let (runtime, request_lease) = scope.into_parts();
        let pane_lease = runtime.acquire_lease(WorkspaceLeaseKind::VisiblePane);
        drop(request_lease);
        self.focus_with_lease(window_id, pane_id, runtime, pane_lease, intent_epoch)
    }

    /// Restore a persisted pane without claiming foreground visibility. The
    /// background lease keeps the checkout runtime alive until the renderer
    /// focuses the pane or explicitly detaches it.
    pub fn restore_background(
        &self,
        mut context: WindowPaneWorkspaceContext,
        runtime: Arc<WorkspaceRuntime>,
    ) -> Result<WindowPaneWorkspaceContext, WindowContextError> {
        let window_id = context.window_id.trim().to_string();
        let pane_id = context.pane_id.trim().to_string();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        if context.focused_checkout_id != *runtime.checkout_id() {
            return Err(WindowContextError::PaneUnavailable { window_id, pane_id });
        }
        context.window_id = window_id.clone();
        context.pane_id = pane_id.clone();
        context.workspace_generation = runtime.generation();
        context.intent_epoch = context.intent_epoch.max(1);
        context.revision = context.revision.max(1);
        let lease = runtime.acquire_lease(WorkspaceLeaseKind::BackgroundOpen);

        let mut windows = self
            .windows
            .write()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let window = windows.entry(window_id.clone()).or_default();
        let current_epoch = Self::current_pane_intent(window, &pane_id);
        if current_epoch >= context.intent_epoch {
            return Err(WindowContextError::StaleIntent {
                window_id,
                pane_id: Some(pane_id),
                received_epoch: context.intent_epoch,
                current_epoch,
            });
        }

        let should_be_active = window
            .active_pane_id
            .as_deref()
            .and_then(|active_pane_id| window.panes.get(active_pane_id))
            .is_none_or(|active| active.context.intent_epoch <= context.intent_epoch);
        window
            .pane_intent_epochs
            .insert(pane_id.clone(), context.intent_epoch);
        window.panes.insert(
            pane_id.clone(),
            WindowPaneContextEntry {
                context: context.clone(),
                _workspace_lease: lease,
            },
        );
        if should_be_active {
            window.active_pane_id = Some(pane_id);
        }
        Ok(context)
    }

    fn focus_with_lease(
        &self,
        window_id: &str,
        pane_id: &str,
        runtime: Arc<WorkspaceRuntime>,
        lease: WorkspaceLease,
        intent_epoch: u64,
    ) -> Result<WindowPaneWorkspaceContext, WindowContextError> {
        let window_id = window_id.trim();
        let pane_id = pane_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        if intent_epoch == 0 {
            return Err(WindowContextError::InvalidIntentEpoch {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            });
        }

        let mut windows = self
            .windows
            .write()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let window = windows.entry(window_id.to_string()).or_default();
        Self::reject_stale_pane_intent(window_id, pane_id, window, intent_epoch)?;

        if let Some(current) = window.panes.get_mut(pane_id) {
            if &current.context.focused_checkout_id == runtime.checkout_id()
                && current.context.workspace_generation == runtime.generation()
            {
                current.context.revision =
                    current.context.revision.checked_add(1).ok_or_else(|| {
                        WindowContextError::RevisionExhausted {
                            window_id: window_id.to_string(),
                            pane_id: pane_id.to_string(),
                        }
                    })?;
                current.context.intent_epoch = intent_epoch;
                // Repeated renderer focus also upgrades a recovered
                // BackgroundOpen lease to VisiblePane.
                current._workspace_lease = lease;
                let context = current.context.clone();
                window
                    .pane_intent_epochs
                    .insert(pane_id.to_string(), intent_epoch);
                window.active_pane_id = Some(pane_id.to_string());
                return Ok(context);
            }
        }

        let revision = match window.panes.get(pane_id) {
            Some(current) => current.context.revision.checked_add(1).ok_or_else(|| {
                WindowContextError::RevisionExhausted {
                    window_id: window_id.to_string(),
                    pane_id: pane_id.to_string(),
                }
            })?,
            None => 1,
        };
        let context = WindowPaneWorkspaceContext {
            window_id: window_id.to_string(),
            pane_id: pane_id.to_string(),
            focused_checkout_id: runtime.checkout_id().clone(),
            workspace_generation: runtime.generation(),
            active_session_id: None,
            intent_epoch,
            revision,
        };
        window
            .pane_intent_epochs
            .insert(pane_id.to_string(), intent_epoch);
        window.active_pane_id = Some(pane_id.to_string());
        window.panes.insert(
            pane_id.to_string(),
            WindowPaneContextEntry {
                context: context.clone(),
                _workspace_lease: lease,
            },
        );
        Ok(context)
    }

    pub fn set_active_session(
        &self,
        window_id: &str,
        pane_id: &str,
        active_session_id: Option<String>,
        intent_epoch: u64,
    ) -> Result<WindowPaneWorkspaceContext, WindowContextError> {
        let window_id = window_id.trim();
        let pane_id = pane_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        if intent_epoch == 0 {
            return Err(WindowContextError::InvalidIntentEpoch {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            });
        }
        let active_session_id = active_session_id
            .map(|session_id| session_id.trim().to_string())
            .filter(|session_id| !session_id.is_empty());

        let mut windows = self
            .windows
            .write()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let window =
            windows
                .get_mut(window_id)
                .ok_or_else(|| WindowContextError::PaneUnavailable {
                    window_id: window_id.to_string(),
                    pane_id: pane_id.to_string(),
                })?;
        Self::reject_stale_pane_intent(window_id, pane_id, window, intent_epoch)?;
        let pane =
            window
                .panes
                .get_mut(pane_id)
                .ok_or_else(|| WindowContextError::PaneUnavailable {
                    window_id: window_id.to_string(),
                    pane_id: pane_id.to_string(),
                })?;
        pane.context.revision = pane.context.revision.checked_add(1).ok_or_else(|| {
            WindowContextError::RevisionExhausted {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            }
        })?;
        pane.context.active_session_id = active_session_id;
        pane.context.intent_epoch = intent_epoch;
        window
            .pane_intent_epochs
            .insert(pane_id.to_string(), intent_epoch);
        Ok(pane.context.clone())
    }

    pub fn remove_pane(
        &self,
        window_id: &str,
        pane_id: &str,
        intent_epoch: u64,
    ) -> Result<bool, WindowContextError> {
        let window_id = window_id.trim();
        let pane_id = pane_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        if intent_epoch == 0 {
            return Err(WindowContextError::InvalidIntentEpoch {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            });
        }
        let mut windows = self
            .windows
            .write()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let window = windows.entry(window_id.to_string()).or_default();
        Self::reject_stale_pane_intent(window_id, pane_id, window, intent_epoch)?;
        let removed = window.panes.remove(pane_id).is_some();
        window
            .pane_intent_epochs
            .insert(pane_id.to_string(), intent_epoch);
        if window.active_pane_id.as_deref() == Some(pane_id) {
            window.active_pane_id = window
                .panes
                .values()
                .max_by_key(|entry| entry.context.intent_epoch)
                .map(|entry| entry.context.pane_id.clone());
        }
        Ok(removed)
    }

    pub fn remove_window(
        &self,
        window_id: &str,
        intent_epoch: u64,
    ) -> Result<usize, WindowContextError> {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if intent_epoch == 0 {
            return Err(WindowContextError::InvalidIntentEpoch {
                window_id: window_id.to_string(),
                pane_id: "*".to_string(),
            });
        }
        let mut windows = self
            .windows
            .write()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let window = windows.entry(window_id.to_string()).or_default();
        let current_epoch = window
            .pane_intent_epochs
            .values()
            .copied()
            .chain(std::iter::once(window.tombstone_intent_epoch))
            .max()
            .unwrap_or(0);
        if intent_epoch <= current_epoch {
            return Err(WindowContextError::StaleIntent {
                window_id: window_id.to_string(),
                pane_id: None,
                received_epoch: intent_epoch,
                current_epoch,
            });
        }
        let removed = window.panes.len();
        window.panes.clear();
        window.pane_intent_epochs.clear();
        window.active_pane_id = None;
        window.tombstone_intent_epoch = intent_epoch;
        Ok(removed)
    }

    /// Check an intent before doing checkout/session validation outside the
    /// registry write lock. The mutating method performs the same CAS again.
    pub fn validate_pane_intent(
        &self,
        window_id: &str,
        pane_id: &str,
        intent_epoch: u64,
    ) -> Result<(), WindowContextError> {
        let window_id = window_id.trim();
        let pane_id = pane_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        if intent_epoch == 0 {
            return Err(WindowContextError::InvalidIntentEpoch {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            });
        }
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        if let Some(window) = windows.get(window_id) {
            Self::reject_stale_pane_intent(window_id, pane_id, window, intent_epoch)?;
        }
        Ok(())
    }

    pub fn next_pane_intent_epoch(
        &self,
        window_id: &str,
        pane_id: &str,
    ) -> Result<u64, WindowContextError> {
        let window_id = window_id.trim();
        let pane_id = pane_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        if pane_id.is_empty() {
            return Err(WindowContextError::EmptyPaneId);
        }
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let current = windows
            .get(window_id)
            .map(|window| Self::current_pane_intent(window, pane_id))
            .unwrap_or(0);
        current
            .checked_add(1)
            .ok_or_else(|| WindowContextError::RevisionExhausted {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            })
    }

    pub fn next_window_intent_epoch(&self, window_id: &str) -> Result<u64, WindowContextError> {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            return Err(WindowContextError::EmptyWindowId);
        }
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let current = windows
            .get(window_id)
            .map(|window| {
                window
                    .pane_intent_epochs
                    .values()
                    .copied()
                    .chain(std::iter::once(window.tombstone_intent_epoch))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        current
            .checked_add(1)
            .ok_or_else(|| WindowContextError::RevisionExhausted {
                window_id: window_id.to_string(),
                pane_id: "*".to_string(),
            })
    }

    fn current_pane_intent(window: &WindowWorkspaceContext, pane_id: &str) -> u64 {
        window
            .pane_intent_epochs
            .get(pane_id)
            .copied()
            .unwrap_or(0)
            .max(window.tombstone_intent_epoch)
    }

    fn reject_stale_pane_intent(
        window_id: &str,
        pane_id: &str,
        window: &WindowWorkspaceContext,
        intent_epoch: u64,
    ) -> Result<(), WindowContextError> {
        let current_epoch = Self::current_pane_intent(window, pane_id);
        if intent_epoch <= current_epoch {
            return Err(WindowContextError::StaleIntent {
                window_id: window_id.to_string(),
                pane_id: Some(pane_id.to_string()),
                received_epoch: intent_epoch,
                current_epoch,
            });
        }
        Ok(())
    }

    pub fn pane(
        &self,
        window_id: &str,
        pane_id: &str,
    ) -> Result<Option<WindowPaneWorkspaceContext>, WindowContextError> {
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        Ok(windows
            .get(window_id.trim())
            .and_then(|window| window.panes.get(pane_id.trim()))
            .map(|pane| pane.context.clone()))
    }

    pub fn active_pane(
        &self,
        window_id: &str,
    ) -> Result<Option<WindowPaneWorkspaceContext>, WindowContextError> {
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        Ok(windows.get(window_id.trim()).and_then(|window| {
            window
                .active_pane_id
                .as_deref()
                .and_then(|pane_id| window.panes.get(pane_id))
                .map(|pane| pane.context.clone())
        }))
    }

    pub fn intent_epoch_snapshots(
        &self,
    ) -> Result<Vec<WindowIntentEpochSnapshot>, WindowContextError> {
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let mut snapshots = Vec::new();
        for (window_id, window) in windows.iter() {
            if window.tombstone_intent_epoch > 0 {
                snapshots.push(WindowIntentEpochSnapshot {
                    window_id: window_id.clone(),
                    pane_id: None,
                    intent_epoch: window.tombstone_intent_epoch,
                });
            }
            snapshots.extend(
                window
                    .pane_intent_epochs
                    .iter()
                    .map(|(pane_id, intent_epoch)| WindowIntentEpochSnapshot {
                        window_id: window_id.clone(),
                        pane_id: Some(pane_id.clone()),
                        intent_epoch: *intent_epoch,
                    }),
            );
        }
        snapshots.sort_by(|left, right| {
            (&left.window_id, &left.pane_id).cmp(&(&right.window_id, &right.pane_id))
        });
        Ok(snapshots)
    }

    pub fn snapshots(&self) -> Result<Vec<WindowPaneWorkspaceContext>, WindowContextError> {
        let windows = self
            .windows
            .read()
            .map_err(|error| WindowContextError::LockPoisoned(error.to_string()))?;
        let mut contexts = windows
            .values()
            .flat_map(|window| window.panes.values().map(|pane| pane.context.clone()))
            .collect::<Vec<_>>();
        contexts.sort_by(|left, right| {
            (&left.window_id, &left.pane_id).cmp(&(&right.window_id, &right.pane_id))
        });
        Ok(contexts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_policy::WorkspaceActivityPriority;
    use crate::workspace_service::identity::ProjectIdResolver;

    fn runtime(
        parent: &std::path::Path,
        name: &str,
        generation: u64,
    ) -> std::sync::Arc<WorkspaceRuntime> {
        let root = parent.join(name);
        std::fs::create_dir_all(&root).expect("workspace directory");
        let identity = ProjectIdResolver::resolve(&root).expect("workspace identity");
        WorkspaceRuntime::new(identity, Vec::new(), generation)
    }

    #[test]
    fn pane_focus_intents_are_monotonic_and_revisions_are_isolated() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = runtime(temp.path(), "first", 1);
        let second = runtime(temp.path(), "second", 2);
        let contexts = WindowContextRegistry::new();

        assert_eq!(first.lease_count(), 0);
        let initial = contexts
            .focus("main", "left", Arc::clone(&first), 1)
            .expect("focus first checkout");
        assert_eq!(initial.revision, 1);
        assert_eq!(first.lease_count(), 1);
        assert_eq!(
            first.activity_snapshot(std::time::Duration::MAX).priority,
            WorkspaceActivityPriority::VisiblePane
        );
        let repeated = contexts
            .focus("main", "left", Arc::clone(&first), 2)
            .expect("repeat focus");
        assert_eq!(repeated.revision, 2);
        assert_eq!(repeated.intent_epoch, 2);
        assert_eq!(first.lease_count(), 1);

        let changed = contexts
            .focus("main", "left", Arc::clone(&second), 3)
            .expect("focus second checkout");
        assert_eq!(changed.revision, 3);
        assert_eq!(&changed.focused_checkout_id, second.checkout_id());
        assert_eq!(first.lease_count(), 0);
        assert_eq!(second.lease_count(), 1);
        assert_eq!(
            second.activity_snapshot(std::time::Duration::MAX).priority,
            WorkspaceActivityPriority::VisiblePane
        );

        let other_pane = contexts
            .focus("main", "right", Arc::clone(&first), 1)
            .expect("focus other pane");
        assert_eq!(other_pane.revision, 1);
        assert_eq!(first.lease_count(), 1);
        assert_eq!(contexts.snapshots().expect("context snapshots").len(), 2);
        assert_eq!(
            contexts
                .active_pane("main")
                .expect("active pane")
                .expect("active pane context")
                .pane_id,
            "right"
        );

        let active = contexts
            .set_active_session("main", "left", Some("session-1".to_string()), 4)
            .expect("set active session");
        assert_eq!(active.active_session_id.as_deref(), Some("session-1"));
        assert_eq!(active.revision, 4);
        let repeated_active = contexts
            .set_active_session("main", "left", Some("session-1".to_string()), 5)
            .expect("repeat active session");
        assert_eq!(repeated_active.revision, 5);

        assert!(contexts
            .remove_pane("main", "right", 2)
            .expect("remove right pane"));
        assert_eq!(
            contexts
                .active_pane("main")
                .expect("fallback active pane")
                .expect("fallback pane context")
                .pane_id,
            "left"
        );
        assert_eq!(first.lease_count(), 0);
        assert_eq!(contexts.remove_window("main", 6).expect("remove window"), 1);
        assert_eq!(second.lease_count(), 0);
    }

    #[test]
    fn focus_scope_converts_request_lease_to_visible_pane_lease() {
        let temp = tempfile::tempdir().expect("tempdir");
        let runtime = runtime(temp.path(), "checkout", 1);
        let contexts = WindowContextRegistry::new();
        let request_lease = runtime.acquire_lease(WorkspaceLeaseKind::RunningTask);
        let scope = ResolvedWorkspaceScope::new(Arc::clone(&runtime), request_lease);

        contexts
            .focus_scope("main", "pane", scope, 1)
            .expect("focus resolved scope");

        let activity = runtime.activity_snapshot(std::time::Duration::MAX);
        assert_eq!(activity.priority, WorkspaceActivityPriority::VisiblePane);
        assert_eq!(activity.running_task_leases, 0);
        assert_eq!(activity.visible_pane_leases, 1);
    }

    #[test]
    fn recovery_keeps_all_panes_background_open_and_focus_upgrades_one_pane() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = runtime(temp.path(), "first", 11);
        let second = runtime(temp.path(), "second", 12);
        let contexts = WindowContextRegistry::new();
        let recovered = [
            ("main", "main", Arc::clone(&first), Some("session-a")),
            ("main", "secondary", Arc::clone(&second), None),
            (
                "knowledge-window",
                "main",
                Arc::clone(&second),
                Some("session-b"),
            ),
        ];

        for (index, (window_id, pane_id, runtime, active_session_id)) in
            recovered.into_iter().enumerate()
        {
            contexts
                .restore_background(
                    WindowPaneWorkspaceContext {
                        window_id: window_id.to_string(),
                        pane_id: pane_id.to_string(),
                        focused_checkout_id: runtime.checkout_id().clone(),
                        workspace_generation: 1,
                        active_session_id: active_session_id.map(str::to_string),
                        intent_epoch: index as u64 + 1,
                        revision: index as u64 + 3,
                    },
                    runtime,
                )
                .expect("restore background pane");
        }

        assert_eq!(contexts.snapshots().expect("all snapshots").len(), 3);
        assert_eq!(
            first.activity_snapshot(std::time::Duration::MAX).priority,
            WorkspaceActivityPriority::BackgroundOpen
        );
        assert_eq!(
            second
                .activity_snapshot(std::time::Duration::MAX)
                .background_open_leases,
            2
        );

        let next_epoch = contexts
            .next_pane_intent_epoch("main", "main")
            .expect("next focus epoch");
        let focused = contexts
            .focus("main", "main", Arc::clone(&first), next_epoch)
            .expect("upgrade main pane");
        assert_eq!(focused.active_session_id.as_deref(), Some("session-a"));
        let activity = first.activity_snapshot(std::time::Duration::MAX);
        assert_eq!(activity.priority, WorkspaceActivityPriority::VisiblePane);
        assert_eq!(activity.background_open_leases, 0);
        assert_eq!(activity.visible_pane_leases, 1);
    }

    #[test]
    fn focus_validates_window_and_pane_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let runtime = runtime(temp.path(), "checkout", 1);
        let contexts = WindowContextRegistry::new();

        assert!(matches!(
            contexts.focus(" ", "pane", Arc::clone(&runtime), 1),
            Err(WindowContextError::EmptyWindowId)
        ));
        assert!(matches!(
            contexts.focus("main", " ", Arc::clone(&runtime), 1),
            Err(WindowContextError::EmptyPaneId)
        ));
        assert!(matches!(
            contexts.set_active_session("main", "missing", Some("session".to_string()), 1),
            Err(WindowContextError::PaneUnavailable { .. })
        ));
        assert_eq!(runtime.lease_count(), 0);
    }

    #[test]
    fn reverse_completion_rejects_the_late_focus_and_keeps_checkout_b() {
        let temp = tempfile::tempdir().expect("tempdir");
        let checkout_a = runtime(temp.path(), "checkout-a", 1);
        let checkout_b = runtime(temp.path(), "checkout-b", 1);
        let contexts = WindowContextRegistry::new();

        // Intent 1 (A) was issued first but finishes after intent 2 (B).
        contexts
            .focus("main", "main", Arc::clone(&checkout_b), 2)
            .expect("newer B focus completes first");
        assert!(matches!(
            contexts.focus("main", "main", Arc::clone(&checkout_a), 1),
            Err(WindowContextError::StaleIntent {
                received_epoch: 1,
                current_epoch: 2,
                ..
            })
        ));

        let final_context = contexts
            .pane("main", "main")
            .expect("registry read")
            .expect("final pane context");
        assert_eq!(final_context.focused_checkout_id, *checkout_b.checkout_id());
        assert_eq!(final_context.intent_epoch, 2);
        assert_eq!(checkout_a.lease_count(), 0);
        assert_eq!(checkout_b.lease_count(), 1);
    }

    #[test]
    fn pane_and_window_tombstones_prevent_old_focus_from_reappearing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let checkout = runtime(temp.path(), "checkout", 1);
        let contexts = WindowContextRegistry::new();

        contexts
            .focus("window", "pane", Arc::clone(&checkout), 1)
            .expect("initial focus");
        assert!(contexts
            .remove_pane("window", "pane", 2)
            .expect("detach pane"));
        assert!(matches!(
            contexts.focus("window", "pane", Arc::clone(&checkout), 1),
            Err(WindowContextError::StaleIntent {
                current_epoch: 2,
                ..
            })
        ));

        contexts
            .focus("window", "pane", Arc::clone(&checkout), 3)
            .expect("newer focus may create pane");
        assert_eq!(
            contexts.remove_window("window", 4).expect("detach window"),
            1
        );
        assert!(matches!(
            contexts.focus("window", "other-pane", checkout, 3),
            Err(WindowContextError::StaleIntent {
                current_epoch: 4,
                ..
            })
        ));
        assert!(contexts
            .pane("window", "pane")
            .expect("registry read")
            .is_none());
        assert_eq!(
            contexts.intent_epoch_snapshots().expect("intent snapshots"),
            vec![WindowIntentEpochSnapshot {
                window_id: "window".to_string(),
                pane_id: None,
                intent_epoch: 4,
            }]
        );
    }
}
