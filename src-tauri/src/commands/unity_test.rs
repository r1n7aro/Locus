use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{watch, Mutex};

use crate::{
    error::{AppError, AppResult},
    unity_bridge::{
        send_message,
        test_runner::{
            cancel_tests, find_tests, query_progress, read_latest_snapshot, run_tests,
            UnityTestDiscovery, UnityTestFilter, UnityTestProgress, UnityTestRunRequest,
            UnityTestSnapshot,
        },
    },
    Workspace,
};

pub const UNITY_TEST_PROGRESS_EVENT: &str = "unity-test-progress";
pub const UNITY_TEST_SNAPSHOT_CHANGED_EVENT: &str = "unity-test-snapshot-changed";

#[derive(Default)]
pub struct UnityTestDashboardState {
    cancel_tx: Mutex<Option<watch::Sender<bool>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestProgressEvent {
    pub working_dir: String,
    pub source: String,
    pub progress: UnityTestProgress,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestSnapshotChangedEvent {
    pub working_dir: String,
    pub source: String,
    pub run_id: String,
    pub terminal_status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestSourceNavigationResult {
    pub opened: bool,
    pub positioned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityOpenTestSourceResponse {
    opened: bool,
    positioned: bool,
}

pub fn emit_unity_test_progress(
    app_handle: &AppHandle,
    working_dir: &str,
    source: &str,
    progress: UnityTestProgress,
) {
    let event = UnityTestProgressEvent {
        working_dir: working_dir.to_string(),
        source: source.to_string(),
        progress,
    };
    if let Err(error) = app_handle.emit(UNITY_TEST_PROGRESS_EVENT, event) {
        eprintln!("[Locus] failed to emit Unity test progress: {error}");
    }
}

pub fn emit_unity_test_snapshot_changed(
    app_handle: &AppHandle,
    working_dir: &str,
    source: &str,
    snapshot: &UnityTestSnapshot,
) {
    let event = UnityTestSnapshotChangedEvent {
        working_dir: working_dir.to_string(),
        source: source.to_string(),
        run_id: snapshot.run_id.clone(),
        terminal_status: snapshot.terminal_status.clone(),
    };
    if let Err(error) = app_handle.emit(UNITY_TEST_SNAPSHOT_CHANGED_EVENT, event) {
        eprintln!("[Locus] failed to emit Unity test snapshot change: {error}");
    }
}

fn unity_test_app_error(
    error: crate::unity_bridge::test_runner::UnityTestError,
    operation: &str,
) -> AppError {
    AppError::new(
        format!("unity_test.{}", error.code),
        "Unity Test Framework request failed",
    )
    .detail(error.message)
    .operation(operation)
}

async fn current_project_path(workspace: &State<'_, Arc<Workspace>>) -> AppResult<String> {
    let project_path = workspace.path.read().await.clone();
    if project_path.trim().is_empty() {
        return Err(AppError::new(
            "unity_test.workspace_required",
            "Select a Unity project before using the test dashboard",
        )
        .operation("unity_test_workspace"));
    }
    Ok(project_path)
}

#[tauri::command]
pub async fn unity_test_discover(
    filter: UnityTestFilter,
    workspace: State<'_, Arc<Workspace>>,
) -> AppResult<UnityTestDiscovery> {
    let project_path = current_project_path(&workspace).await?;
    find_tests(&project_path, filter)
        .await
        .map_err(|error| unity_test_app_error(error, "unity_test_discover"))
}

#[tauri::command]
pub async fn unity_test_run_dashboard(
    app_handle: AppHandle,
    request: UnityTestRunRequest,
    workspace: State<'_, Arc<Workspace>>,
    dashboard: State<'_, UnityTestDashboardState>,
) -> AppResult<UnityTestSnapshot> {
    let project_path = current_project_path(&workspace).await?;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    {
        let mut active = dashboard.cancel_tx.lock().await;
        if active.is_some() {
            return Err(AppError::new(
                "unity_test.busy",
                "A dashboard Unity test run is already active",
            )
            .operation("unity_test_run_dashboard"));
        }
        *active = Some(cancel_tx);
    }

    let progress_handle = app_handle.clone();
    let progress_path = project_path.clone();
    let result = run_tests(&project_path, request, cancel_rx, move |progress| {
        emit_unity_test_progress(&progress_handle, &progress_path, "dashboard", progress);
    })
    .await;

    *dashboard.cancel_tx.lock().await = None;
    match &result {
        Ok(snapshot) => {
            emit_unity_test_snapshot_changed(&app_handle, &project_path, "dashboard", snapshot);
        }
        Err(error) if error.code != "busy" => {
            if let Ok(Some(snapshot)) = read_latest_snapshot(&project_path) {
                emit_unity_test_snapshot_changed(
                    &app_handle,
                    &project_path,
                    "dashboard",
                    &snapshot,
                );
            }
        }
        Err(_) => {}
    }

    result.map_err(|error| unity_test_app_error(error, "unity_test_run_dashboard"))
}

#[tauri::command]
pub async fn unity_test_cancel_dashboard(
    workspace: State<'_, Arc<Workspace>>,
    dashboard: State<'_, UnityTestDashboardState>,
) -> AppResult<()> {
    let project_path = current_project_path(&workspace).await?;
    if let Some(cancel_tx) = dashboard.cancel_tx.lock().await.as_ref() {
        let _ = cancel_tx.send(true);
    }
    cancel_tests(&project_path)
        .await
        .map_err(|error| unity_test_app_error(error, "unity_test_cancel_dashboard"))
}

#[tauri::command]
pub async fn unity_test_active_progress(
    workspace: State<'_, Arc<Workspace>>,
) -> AppResult<Option<UnityTestProgress>> {
    let project_path = current_project_path(&workspace).await?;
    query_progress(&project_path)
        .await
        .map_err(|error| unity_test_app_error(error, "unity_test_active_progress"))
}

#[tauri::command]
pub async fn unity_test_latest_snapshot(
    workspace: State<'_, Arc<Workspace>>,
) -> AppResult<Option<UnityTestSnapshot>> {
    let project_path = current_project_path(&workspace).await?;
    read_latest_snapshot(&project_path)
        .map_err(|error| unity_test_app_error(error, "unity_test_latest_snapshot"))
}

fn resolve_test_source(project_path: &str, source_path: &str) -> AppResult<(PathBuf, String)> {
    let root = dunce::canonicalize(project_path).map_err(|error| {
        AppError::new(
            "unity_test.invalid_workspace",
            "Failed to resolve Unity project",
        )
        .detail(error.to_string())
        .operation("unity_test_open_source")
    })?;
    let requested = Path::new(source_path);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let canonical = dunce::canonicalize(&candidate).map_err(|error| {
        AppError::new(
            "unity_test.source_not_found",
            "Unity test source file was not found",
        )
        .detail(error.to_string())
        .operation("unity_test_open_source")
    })?;
    if !canonical.is_file() || !canonical.starts_with(&root) {
        return Err(AppError::new(
            "unity_test.source_outside_workspace",
            "Unity test source must be a file inside the current project",
        )
        .operation("unity_test_open_source"));
    }
    let relative = canonical
        .strip_prefix(&root)
        .map_err(|_| {
            AppError::new(
                "unity_test.source_outside_workspace",
                "Invalid Unity test source path",
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");
    Ok((canonical, relative))
}

#[tauri::command]
pub async fn unity_test_open_source(
    path: String,
    line: Option<u32>,
    workspace: State<'_, Arc<Workspace>>,
) -> AppResult<UnityTestSourceNavigationResult> {
    let project_path = current_project_path(&workspace).await?;
    let (canonical, relative) = resolve_test_source(&project_path, &path)?;
    if relative.starts_with("Assets/") || relative.starts_with("Packages/") {
        let payload = serde_json::json!({ "path": relative, "line": line }).to_string();
        if let Ok(response) = send_message(&project_path, "open_test_source", &payload).await {
            if response.ok {
                if let Some(message) = response.message {
                    if let Ok(opened) =
                        serde_json::from_str::<UnityOpenTestSourceResponse>(&message)
                    {
                        if opened.opened {
                            return Ok(UnityTestSourceNavigationResult {
                                opened: true,
                                positioned: opened.positioned,
                            });
                        }
                    }
                }
            }
        }
    }

    super::knowledge::open_file_native(&canonical).map_err(|error| {
        AppError::new(
            "unity_test.source_open_failed",
            "Failed to open Unity test source",
        )
        .detail(error)
        .operation("unity_test_open_source")
    })?;
    Ok(UnityTestSourceNavigationResult {
        opened: true,
        positioned: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_resolution_rejects_files_outside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let error = resolve_test_source(
            workspace.path().to_string_lossy().as_ref(),
            outside.path().to_string_lossy().as_ref(),
        )
        .unwrap_err();
        assert_eq!(error.code, "unity_test.source_outside_workspace");
    }

    #[test]
    fn lifecycle_event_payloads_are_camel_case() {
        let event = UnityTestSnapshotChangedEvent {
            working_dir: "C:/Project".to_string(),
            source: "dashboard".to_string(),
            run_id: "run-1".to_string(),
            terminal_status: "completed".to_string(),
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["workingDir"], "C:/Project");
        assert_eq!(value["terminalStatus"], "completed");
    }
}
