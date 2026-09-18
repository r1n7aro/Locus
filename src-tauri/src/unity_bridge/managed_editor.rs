//! Locus-owned headless Editors outlive individual requests. The checkout
//! service's leases and idle TTL govern retirement; foreign Editors are never
//! adopted merely because they have the same project path.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{
    UnityEditorProcessState, UnityLaunchMode, UnityLaunchResult, UnityProcessIdentityLiveness,
};
use serde::Serialize;

#[derive(Clone)]
struct OwnedEditor {
    process_id: u32,
    created_at_ms: u64,
    epoch: u64,
    last_error: Option<String>,
}

fn editors() -> &'static Mutex<HashMap<String, OwnedEditor>> {
    static EDITORS: OnceLock<Mutex<HashMap<String, OwnedEditor>>> = OnceLock::new();
    EDITORS.get_or_init(Mutex::default)
}

fn key(project: &str) -> String {
    super::process::normalize_project_identity(project).unwrap_or_else(|| project.to_string())
}

pub(crate) async fn lifecycle_lock(checkout_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().await;
    Arc::clone(locks.entry(checkout_id.to_string()).or_default())
}

pub(crate) fn record_launch(launch: &UnityLaunchResult) {
    if launch.mode != UnityLaunchMode::Headless {
        return;
    }
    // Without a creation timestamp we cannot prove ownership after PID reuse.
    let Some(created_at_ms) = super::launched_unity_process_created_at_ms(launch.process_id) else {
        return;
    };
    let epoch = crate::workspace_service::worktrees::record_for_root(std::path::Path::new(
        &launch.project_path,
    ))
    .ok()
    .flatten()
    .map(|record| record.materialization_epoch)
    .unwrap_or(0);
    if let Ok(mut entries) = editors().lock() {
        entries.insert(
            key(&launch.project_path),
            OwnedEditor {
                process_id: launch.process_id,
                created_at_ms,
                epoch,
                last_error: None,
            },
        );
    }
}

fn owned(project: &str, epoch: u64) -> Option<OwnedEditor> {
    editors()
        .lock()
        .ok()?
        .get(&key(project))
        .filter(|entry| entry.epoch == epoch)
        .cloned()
}

pub(crate) fn was_managed(project: &str, epoch: u64) -> bool {
    owned(project, epoch).is_some()
}

pub(crate) fn has_owned_process(project: &str, epoch: u64) -> bool {
    owned(project, epoch).is_some_and(|owner| {
        super::launched_unity_process_liveness(owner.process_id, Some(owner.created_at_ms))
            == Ok(UnityProcessIdentityLiveness::Alive)
    })
}

async fn probe(project: &str) -> Result<super::UnityEditorProcessInfo, String> {
    let project = project.to_string();
    tokio::task::spawn_blocking(move || {
        super::query_current_project_editor_process_uncached(project)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Called only behind an Editor command's service lease. Read-only
/// status/list operations and merely opening a pane never launch an Editor.
pub(crate) async fn ensure_for_tool(
    project: &str,
    checkout_id: &str,
    epoch: u64,
) -> Result<bool, String> {
    let record =
        crate::workspace_service::worktrees::record_for_root(std::path::Path::new(project))?;
    let managed_checkout = record.as_ref().is_some_and(|record| {
        record.managed && record.lifecycle == "active" && record.materialization_epoch == epoch
    });
    if !managed_checkout && !was_managed(project, epoch) {
        return Ok(false);
    }
    let lock = lifecycle_lock(checkout_id).await;
    let _guard = lock.lock().await;
    let current = probe(project).await?;
    match current.state {
        UnityEditorProcessState::Running => Ok(has_owned_process(project, epoch)),
        UnityEditorProcessState::Unknown => Err(format!(
            "Cannot start managed Unity Editor: {}",
            current.last_error.unwrap_or_default()
        )),
        UnityEditorProcessState::NotRunning => {
            super::launch_project_with_mode(project, UnityLaunchMode::Headless).await?;
            Ok(true)
        }
    }
}

pub(crate) async fn prepare_plugin(project: &str) -> Result<(), String> {
    if probe(project).await?.state != UnityEditorProcessState::NotRunning {
        return Err("Headless startup requires a closed Unity project".into());
    }
    let project = project.to_string();
    tokio::task::spawn_blocking(move || super::plugin::prepare_headless_plugin(&project))
        .await
        .map_err(|error| error.to_string())?
}

/// The service host is already draining leases. Never wait for a lifecycle
/// lock here: SDK ensure may own it while waiting for this same service slot.
pub(crate) async fn retire(project: &str, checkout_id: &str, epoch: u64) -> Result<(), String> {
    let Some(owner) = owned(project, epoch) else {
        return Ok(());
    };
    let lock = lifecycle_lock(checkout_id).await;
    let _guard = lock
        .try_lock()
        .map_err(|_| "Unity lifecycle operation is still active")?;
    if super::launched_unity_process_liveness(owner.process_id, Some(owner.created_at_ms))?
        != UnityProcessIdentityLiveness::Alive
    {
        return Ok(());
    }
    let current = probe(project).await?;
    if current.process_id != Some(owner.process_id) {
        return Err("Managed Unity process identity changed".into());
    }
    let result = retire_owned(project, &owner).await;
    if let Ok(mut entries) = editors().lock() {
        if let Some(entry) = entries.get_mut(&key(project)).filter(|entry| {
            entry.process_id == owner.process_id && entry.created_at_ms == owner.created_at_ms
        }) {
            if entry.last_error.as_ref() != result.as_ref().err() || result.is_ok() {
                eprintln!(
                    "[ManagedUnity] idle retirement: project={project}, pid={}, result={}",
                    owner.process_id,
                    result
                        .as_ref()
                        .err()
                        .map(String::as_str)
                        .unwrap_or("released")
                );
            }
            entry.last_error = result.as_ref().err().cloned();
        }
    }
    result
}

async fn retire_owned(project: &str, owner: &OwnedEditor) -> Result<(), String> {
    // Unity itself checks dirty scenes/assets and busy state on its main
    // thread. Automatic retirement never falls back to taskkill / force-close.
    let response = super::send_message_with_timeout(
        project,
        "managed_editor_close",
        "",
        Duration::from_secs(5),
    )
    .await;
    if let Ok(response) = &response {
        if !response.ok {
            return Err(response
                .error
                .clone()
                .unwrap_or_else(|| "Unity refused idle shutdown".into()));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match super::launched_unity_process_liveness(owner.process_id, Some(owner.created_at_ms))? {
            UnityProcessIdentityLiveness::Exited | UnityProcessIdentityLiveness::Replaced => {
                super::invalidate_closed_editor_status(project);
                super::cleanup_closed_project_markers(project).await?;
                return Ok(());
            }
            UnityProcessIdentityLiveness::Alive => {}
        }
        if Instant::now() >= deadline {
            return Err(response
                .err()
                .unwrap_or_else(|| "Unity did not finish idle shutdown".into()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorResource {
    pub project_path: String,
    pub process_id: u32,
    pub mode: UnityLaunchMode,
    pub managed: bool,
    pub working_set_bytes: Option<u64>,
    pub import_worker_count: usize,
    pub last_error: Option<String>,
}

pub(crate) async fn resources() -> Result<Vec<EditorResource>, String> {
    tokio::task::spawn_blocking(|| {
        let mut rows = super::process::editor_resources()?;
        let entries = editors().lock().map_err(|error| error.to_string())?;
        for row in &mut rows {
            if let Some(owner) = entries.get(&key(&row.project_path)) {
                row.managed = row.mode == UnityLaunchMode::Headless
                    && row.process_id == owner.process_id
                    && super::launched_unity_process_liveness(
                        owner.process_id,
                        Some(owner.created_at_ms),
                    ) == Ok(UnityProcessIdentityLiveness::Alive);
                if row.managed {
                    row.last_error = owner.last_error.clone();
                }
            }
        }
        Ok(rows)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interactive_editors_are_never_adopted_for_ttl_retirement() {
        let project = "F:/locus-user-managed-editor-test";
        record_launch(&UnityLaunchResult {
            project_path: project.into(),
            editor_path: "Unity.exe".into(),
            project_version: "6000.5.8f1".into(),
            process_id: std::process::id(),
            mode: UnityLaunchMode::Interactive,
        });
        assert!(!was_managed(project, 0));
    }
    #[test]
    fn an_old_assignment_cannot_manage_a_reused_checkout() {
        let project = "F:/locus-managed-editor-epoch-test";
        editors().lock().unwrap().insert(
            key(project),
            OwnedEditor {
                process_id: 1,
                created_at_ms: 1,
                epoch: 7,
                last_error: None,
            },
        );
        assert!(was_managed(project, 7));
        assert!(!was_managed(project, 8));
        editors().lock().unwrap().remove(&key(project));
    }
    #[tokio::test]
    async fn lifecycle_serializes_one_checkout_without_blocking_siblings() {
        let a = lifecycle_lock("managed-a").await;
        let a_again = lifecycle_lock("managed-a").await;
        let b = lifecycle_lock("managed-b").await;
        let _lease = a.lock().await;
        assert!(a_again.try_lock().is_err());
        assert!(b.try_lock().is_ok());
    }
}
