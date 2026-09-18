use crate::agent::workspace_execution_lock::{
    self as locks, WorkspaceExecutionGuard, WorkspaceExecutionLockOwner,
    WorkspaceExecutionLockRequest,
};
use crate::workspace_service::WorkspaceRuntime;
use std::sync::Arc;
use tauri::AppHandle;

pub(crate) struct OperationGuard {
    _operation: WorkspaceOperationGuard,
    _git_filesystem: tokio::sync::OwnedMutexGuard<()>,
}

pub(crate) struct WorkspaceOperationGuard {
    _workspace: Option<WorkspaceExecutionGuard>,
    _delegation: Option<Arc<locks::SdkExecutionDelegation>>,
    _delegated_operation: Option<tokio::sync::OwnedMutexGuard<()>>,
}

pub(crate) fn verify_target(
    grant: &locks::SdkExecutionDelegation,
    runtime: &WorkspaceRuntime,
) -> Result<bool, String> {
    if grant.project_id != runtime.project_id().to_string() {
        return Err("Python execution delegation cannot cross logical projects".into());
    }
    let repository = crate::workspace_service::identity::resolve_git_common_dir(runtime.root())
        .map(|path| crate::workspace_service::worktrees::path_key(&path));
    if grant.repository != repository
        || (grant.checkout_id != runtime.checkout_id().as_str() && grant.repository.is_none())
    {
        return Err(
            "Python execution delegation cannot cross repositories even when project GUIDs match"
                .into(),
        );
    }
    if grant.checkout_id == runtime.checkout_id().to_string() {
        if grant.epoch != runtime.materialization_epoch()
            || grant.generation != runtime.generation()
        {
            return Err(
                "Python execution delegation belongs to an earlier checkout assignment/runtime"
                    .into(),
            );
        }
        return Ok(true);
    }
    Ok(false)
}

/// Borrow only an existing Python write gate; otherwise acquire exactly the
/// request selected by the caller's established lock policy (including PathWrite).
pub(crate) async fn acquire_workspace(
    app: &AppHandle,
    runtime: &Arc<WorkspaceRuntime>,
    request: WorkspaceExecutionLockRequest,
    mut owner: WorkspaceExecutionLockOwner,
    token: Option<&str>,
) -> Result<WorkspaceOperationGuard, String> {
    let delegation = token.map(locks::resolve_sdk_delegation).transpose()?;
    let mut borrowed = false;
    if let Some(grant) = &delegation {
        borrowed = verify_target(grant, runtime)?;
    }
    let delegated_operation = match &delegation {
        Some(grant) => {
            let gate = grant
                .operations
                .lock()
                .await
                .entry(runtime.checkout_id().to_string())
                .or_default()
                .clone();
            Some(gate.lock_owned().await)
        }
        None => None,
    };
    // Recheck revocation after queueing: a completed Python process cannot start
    // new writes, while requests already admitted retain their actual gate Arc.
    if let Some(token) = token {
        locks::resolve_sdk_delegation(token)?;
    }
    let workspace = if borrowed {
        None
    } else {
        let path = runtime.root().to_string_lossy().into_owned();
        if let Some(grant) = &delegation {
            owner.session_id = grant.session_id.clone();
            owner.run_id = grant.run_id.clone();
        }
        let (_cancel, rx) = tokio::sync::watch::channel(false);
        let gate = locks::process_workspace_execution_lock(&path);
        let waiting = gate.acquire_with_diagnostics(
            request,
            owner,
            rx,
            crate::workspace_service::event::WorkspaceEventScope::for_runtime(runtime),
            app,
        );
        let guard = if delegation.is_some() {
            tokio::time::timeout(std::time::Duration::from_millis(250), waiting).await
                .map_err(|_| "Destination is busy; retry after its active mutation finishes (cross-checkout waits do not hold a circular wait)")?
        } else {
            waiting.await
        };
        Some(guard.map_err(|e| format!("Workspace coordination cancelled: {e:?}"))?)
    };
    Ok(WorkspaceOperationGuard {
        _workspace: workspace,
        _delegation: delegation,
        _delegated_operation: delegated_operation,
    })
}

pub(crate) async fn acquire(
    app: &AppHandle,
    runtime: &Arc<WorkspaceRuntime>,
    action: &str,
    token: Option<&str>,
) -> Result<OperationGuard, String> {
    let owner = WorkspaceExecutionLockOwner {
        session_id: "merge-jobs".into(),
        run_id: format!("merge-{}", uuid::Uuid::new_v4()),
        iteration: 0,
        workspace: runtime.root().to_string_lossy().into_owned(),
        tools: vec![format!("merges.{action}")],
    };
    let operation = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        acquire_workspace(
            app,
            runtime,
            WorkspaceExecutionLockRequest::Exclusive,
            owner,
            token,
        ),
    )
    .await
    .map_err(|_| "Destination is busy; retry the merge operation")??;
    let git_lock = crate::commands::git_named_operation_lock(
        &runtime.core().workspace_fs_lock_name(runtime.checkout_id()),
    );
    let git_filesystem =
        tokio::time::timeout(std::time::Duration::from_secs(10), git_lock.lock_owned())
            .await
            .map_err(|_| "Destination Git filesystem operation is busy; retry")?;
    Ok(OperationGuard {
        _operation: operation,
        _git_filesystem: git_filesystem,
    })
}

pub(crate) fn needs_guard(action: &str, params: &serde_json::Value) -> bool {
    matches!(action, "prepare" | "apply" | "abort" | "stage" | "commit")
        || (action == "validate"
            && params.get("level").and_then(serde_json::Value::as_str) == Some("unity"))
}

/// Dropping the RPC waiter does not stop a blocking worker. Move the real
/// coordination lease into that worker so it protects its entire lifetime.
pub(crate) async fn run_blocking<G, T, F>(guard: G, work: F) -> Result<T, String>
where G: Send + 'static, T: Send + 'static, F: FnOnce() -> T + Send + 'static {
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        work()
    }).await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn cancelled_waiter_keeps_guard_until_worker_finishes() {
        struct Guard(Arc<AtomicBool>, Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
                let _ = self.1.take().unwrap().send(());
            }
        }
        let held = Arc::new(AtomicBool::new(true));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, wait) = tokio::sync::oneshot::channel();
        let (dropped, done) = tokio::sync::oneshot::channel();
        let guard = Guard(held.clone(), Some(dropped));
        let task = tokio::spawn(run_blocking(guard, move || {
            started.send(()).unwrap();
            wait.blocking_recv().unwrap();
        }));
        ready.await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(held.load(Ordering::SeqCst));
        finish.send(()).unwrap();
        done.await.unwrap();
        assert!(!held.load(Ordering::SeqCst));
    }
}
