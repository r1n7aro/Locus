//! Desktop file watchers for devices.
//!
//! Workspace file watching runs on demand: only while some owner (a desktop
//! explorer pane, or here a connected device) holds a lease. A device that
//! shows a file or folder of a workspace holds one lease for that workspace,
//! released when it stops showing anything there or disconnects.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::workspace_service::{CheckoutId, ProjectRegistry, WorkspaceRef};

use super::connections::Connection;

fn registry(app: &AppHandle) -> Option<tauri::State<'_, Arc<ProjectRegistry>>> {
    app.try_state::<Arc<ProjectRegistry>>()
}

fn release(registry: &ProjectRegistry, owner: &str, token: &str) {
    // Released on every runtime: the checkout may have been reassigned.
    for runtime in registry.runtimes() {
        runtime.core().file_watch().release(owner, Some(token));
    }
}

/// Holds a watcher lease for every checkout the connection watches, and only
/// for those.
pub fn sync(app: &AppHandle, connection: &Connection) {
    let Some(registry) = registry(app) else {
        return;
    };
    let owner = connection.watcher_owner();
    let wanted = connection.watched_checkouts();
    let mut leases = connection.watcher_leases.lock().unwrap_or_else(|error| error.into_inner());
    leases.retain(|checkout, token| {
        let keep = wanted.contains(checkout);
        if !keep {
            release(&registry, &owner, token);
        }
        keep
    });
    for checkout in wanted {
        if leases.contains_key(&checkout) {
            continue;
        }
        let Ok(checkout_id) = CheckoutId::new(&checkout) else {
            continue;
        };
        let workspace_ref = WorkspaceRef { checkout_id, expected_generation: None, expected_materialization_epoch: None };
        let Ok(scope) = registry.resolve_workspace_ref(&workspace_ref) else {
            continue;
        };
        // `None`: the workspace is watched another way (Unity asset database).
        if let Ok(Some(token)) = scope.runtime().core().file_watch().acquire(scope.runtime(), app, &owner) {
            leases.insert(checkout, token);
        }
    }
}

/// Releases every lease of a closed connection.
pub fn release_all(app: &AppHandle, connection: &Connection) {
    let leases = std::mem::take(&mut *connection.watcher_leases.lock().unwrap_or_else(|error| error.into_inner()));
    if leases.is_empty() {
        return;
    }
    if let Some(registry) = registry(app) {
        let owner = connection.watcher_owner();
        for token in leases.values() {
            release(&registry, &owner, token);
        }
    }
}
