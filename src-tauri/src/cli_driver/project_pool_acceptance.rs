//! Real Editor acceptance for fixed physical slots. All commits and process
//! shutdowns target directories created by this suite. The supplied project
//! contributes a frozen snapshot without changing its HEAD or real index.
use super::*;
use crate::workspace_service::{pool, worktrees, ProjectRegistry};
use std::io::Write;

fn assert_that(value: bool, detail: &str) -> Result<(), String> {
    if value {
        Ok(())
    } else {
        Err(detail.into())
    }
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    worktrees::git(root, args)
}

/// File identity distinguishes preserving a cache from copying its bytes and
/// timestamps into a replacement. Handles are closed before slot reassignment.
fn file_identity(path: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let file = std::fs::File::open(path)
            .map_err(|error| format!("Open cache identity {}: {error}", path.display()))?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the file owns a valid open handle for the duration of this
        // call, and info points to initialized writable storage.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|error| format!("Read cache identity {}: {error}", path.display()))?;
        Ok(format!(
            "{:08x}:{:08x}{:08x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(format!("Cache file identity is not supported on this platform: {}", path.display()))
    }
}

fn indexed_git(
    root: &Path,
    index: &Path,
    args: &[&str],
    bytes: Option<&[u8]>,
) -> Result<String, String> {
    let mut command = crate::process_util::command("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "Locus Pool Acceptance")
        .env("GIT_AUTHOR_EMAIL", "pool-test@locus.local")
        .env("GIT_COMMITTER_NAME", "Locus Pool Acceptance")
        .env("GIT_COMMITTER_EMAIL", "pool-test@locus.local")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    if let Some(bytes) = bytes {
        child
            .stdin
            .take()
            .ok_or("Git stdin unavailable")?
            .write_all(bytes)
            .map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().into())
        .map_err(|e| e.to_string())
}

fn snapshot_commit(root: &Path) -> Result<String, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = directory.path().join("snapshot-index");
    let parent = worktrees::git_text(root, &["rev-parse", "HEAD"])?;
    indexed_git(root, &index, &["read-tree", &parent], None)?;
    indexed_git(root, &index, &["add", "-A", "--", "."], None)?;
    // The installed plugin is part of the acceptance input even if the host
    // intentionally ignores the generated package in its project repository.
    if root.join("Packages/com.farlocus.locus").is_dir() {
        indexed_git(
            root,
            &index,
            &["add", "-f", "--", "Packages/com.farlocus.locus"],
            None,
        )?;
    }
    let tree = indexed_git(root, &index, &["write-tree"], None)?;
    indexed_git(
        root,
        &index,
        &[
            "commit-tree",
            &tree,
            "-p",
            &parent,
            "-m",
            "Locus pool acceptance working-state snapshot",
        ],
        None,
    )
}

fn patch_commit(root: &Path, parent: &str, path: &str, bytes: &[u8]) -> Result<String, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = directory.path().join("patch-index");
    indexed_git(root, &index, &["read-tree", parent], None)?;
    let blob = indexed_git(root, &index, &["hash-object", "-w", "--stdin"], Some(bytes))?;
    indexed_git(
        root,
        &index,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "100644",
            &blob,
            path,
        ],
        None,
    )?;
    let tree = indexed_git(root, &index, &["write-tree"], None)?;
    indexed_git(
        root,
        &index,
        &[
            "commit-tree",
            &tree,
            "-p",
            parent,
            "-m",
            "Locus pool acceptance next asset revision",
        ],
        None,
    )
}

fn capture_json(output: &str) -> Result<Value, String> {
    for line in output.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
            if value.is_object() {
                return Ok(value);
            }
        }
    }
    let first = output
        .find('{')
        .ok_or_else(|| format!("Editor returned no JSON: {}", clip(output, 300)))?;
    let last = output.rfind('}').ok_or("Editor returned incomplete JSON")?;
    serde_json::from_str(&output[first..=last]).map_err(|e| e.to_string())
}

async fn inspect(project: &str, folder: &str) -> Result<Value, String> {
    capture_json(
        &execute_capture(
            project,
            &format!(
                "print(Locus.MergeTesting.LocusMergeFixtureApi.Inspect({}));",
                json!(folder)
            ),
        )
        .await?,
    )
}

use crate::unity_bridge::remove_closed_editor_marker;

async fn close_owned(
    project: &str,
    expected_pid: Option<u32>,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let current = unity_bridge::query_current_project_editor_process_uncached(project.into());
    if current.state != UnityEditorProcessState::NotRunning && (current.state != UnityEditorProcessState::Running
        || expected_pid.is_some_and(|pid| Some(pid) != current.process_id)
    ) {
        return Err("The owned pool Editor process identity changed; refusing to close an unverified process".into());
    }
    let process_id = current.process_id.or(expected_pid);
    sink.emit("pool_editor_closing", json!({"project":project,"processId":process_id,"state":format!("{:?}",current.state)}));
    if current.state != UnityEditorProcessState::NotRunning {
    let result =
        unity_bridge::close_current_project_unity_processes(project, Duration::from_secs(45)).await;
    if let Ok(closed) = &result {
        // The bridge waits for every matching Editor/worker PID, including
        // workers that may hold import database or project lock handles.
        sink.emit("pool_editor_processes_exited", json!({"project":project,"processIds":closed.process_ids,"forcedProcessIds":closed.forced_process_ids}));
    }
    if result.is_err() {
        let current = unity_bridge::query_current_project_editor_process_uncached(project.into());
        if current.state == UnityEditorProcessState::Running && current.process_id == process_id {
            unity_bridge::force_close_current_project_unity_processes(
                project,
                Duration::from_secs(20),
            )
            .await.map_err(|error| format!("Force close pool Editor {project}: {error}"))?;
        } else if current.state != UnityEditorProcessState::NotRunning {
            return result.map(|_| ()).map_err(|error| format!("Close pool Editor {project}: {error}"));
        }
    }
    }
    let final_state = unity_bridge::query_current_project_editor_process_uncached(project.into());
    assert_that(
        final_state.state == UnityEditorProcessState::NotRunning,
        "Owned Editor did not stop",
    )?;
    sink.emit("pool_editor_stopped", json!({"project":project,"processId":process_id}));
    // Only these ephemeral files, inside this suite's newly created directory,
    // may be removed after native process enumeration confirms shutdown.
    for relative in ["Temp/UnityLockfile", "Library/EditorInstance.json"] {
        let path = Path::new(project).join(relative);
        sink.emit("pool_marker_cleanup", json!({"project":project,"path":path}));
        let project = project.to_string();
        tokio::task::spawn_blocking(move || remove_closed_editor_marker(&path, || {
            let state = unity_bridge::query_current_project_editor_process_uncached(project.clone());
            if state.state != UnityEditorProcessState::NotRunning {
                return Err(format!("Refuse marker cleanup while pool Editor ownership is {:?}: {project}", state.state));
            }
            Ok(())
        })).await.map_err(|error| format!("Pool marker cleanup worker failed: {error}"))??;
    }
    sink.emit(
        "pool_editor_closed",
        json!({"project":project,"processId":process_id}),
    );
    Ok(())
}

/// Record all import-generated source changes as an explicit test commit in
/// the owned slot. Nothing is discarded, and the caller's index is untouched.
fn seal_owned_slot(root: &Path) -> Result<String, String> {
    let commit = snapshot_commit(root)?;
    let old_head = worktrees::git_text(root, &["rev-parse", "HEAD"])?;
    git(root, &["update-ref", "HEAD", &commit, &old_head])?;
    git(root, &["read-tree", &commit])?;
    assert_that(
        git(
            root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?
        .is_empty(),
        "Post-import slot snapshot did not preserve a clean source state",
    )?;
    Ok(commit)
}

pub(super) async fn run(
    app: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let source = Path::new(project);
    let before_head = git(source, &["rev-parse", "HEAD"])?;
    let before_index = git(source, &["ls-files", "--stage", "-z"])?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let folder = format!("Assets/LocusMergeDriver-pool-{token}");
    let generated = capture_json(
        &execute_capture(
            project,
            &format!(
                "print(Locus.MergeTesting.LocusMergeFixtureApi.Generate({}));",
                json!(folder)
            ),
        )
        .await?,
    )?;
    assert_that(
        generated["unityVersion"]
            .as_str()
            .is_some_and(|v| v.starts_with("6000.5.")),
        "Pool acceptance requires a Unity 6.5 source clone",
    )?;
    let snapshots = Path::new(
        generated["snapshots"]
            .as_str()
            .ok_or("Fixture omitted snapshot directory")?,
    );
    let next_graph =
        std::fs::read(snapshots.join("graph-source.yaml")).map_err(|e| e.to_string())?;
    let baseline = snapshot_commit(source)?;
    let pool_base = std::env::var_os("LOCUS_UNITY_POOL_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| source.parent().unwrap_or(source).join(".locus-pool-driver"));
    assert_that(pool_base.is_absolute(), "Pool test root must be absolute")?;
    std::fs::create_dir_all(&pool_base).map_err(|e| e.to_string())?;
    let pool_root = pool_base.join(&token);
    std::fs::create_dir(&pool_root).map_err(|e| e.to_string())?;
    sink.emit("suite_start", json!({"suite":"project-pool","source":project,"poolRoot":pool_root,"fixture":folder,"baseline":baseline}));
    let registry = app.state::<Arc<ProjectRegistry>>().inner().clone();
    let store = app
        .state::<Arc<crate::session::store::SessionStore>>()
        .inner()
        .clone();
    let mut request = pool::AcquirePoolRequest {
        source_root: project.into(),
        pool_root: pool_root.to_string_lossy().into(),
        commit: baseline,
        branch: None,
        max_slots: 1,
        assignment_id: Some(format!("pool-{token}-a")),
    };
    let first = pool::acquire(&request)?;
    let owned_project = first.worktree.root.clone();
    let owned_root = Path::new(&owned_project);
    let mut owned_pid = None;
    let outcome: Result<Value, String> = async {
        assert_that(!first.reused && first.worktree.materialization_epoch == 1, "First pool assignment did not materialize a new slot")?;
        assert_that(unity_bridge::query_current_project_editor_process_uncached(owned_project.clone()).state == UnityEditorProcessState::NotRunning, "New pool slot was already owned by an Editor")?;
        let runtime = registry.register(owned_root)?;
        let first_workspace_ref = crate::workspace_service::WorkspaceRef::for_runtime(runtime.as_ref());
        let first_window_context = {
            let windows = crate::workspace_service::WindowContextRegistry::new();
            windows.focus("pool-acceptance", "main", Arc::clone(&runtime), 1)
                .map_err(|error| error.to_string())?
        };
        let session = create_workspace_driver_session(app, 100, runtime.as_ref()).await?;
        let execution = registry.execution_context(runtime.checkout_id(), &[crate::workspace_service::ServiceKind::Unity]).await?;
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let mut connect_config = config.clone();
        connect_config.open_unity = true;
        connect_config.connect_timeout = config.connect_timeout.max(Duration::from_secs(600));
        connect_config.no_progress_timeout = config.no_progress_timeout.max(Duration::from_secs(180));
        let connection = ensure_connected(&owned_project, &connect_config, PluginPrepareOutcome::UpToDate, sink, &mut cancel_rx).await?;
        owned_pid = connection.editor_process_id;
        wait_for_unity_editor_idle(&owned_project, &connect_config, sink, &mut cancel_rx).await?;
        let first_report = inspect(&owned_project, &folder).await?;
        sink.emit("pool_first_inspected", json!({"project":owned_project,"report":first_report}));
        assert_that(first_report["localValue"] == 10 && first_report["incomingValue"] == 20 && first_report["health"] == 50, "Initial asset snapshot failed real Unity deserialization")?;
        assert_that(first_report["sharedIdentity"] == true && first_report["cycleIdentity"] == true, "Initial SerializeReference aliases/cycles failed")?;
        let run_id = format!("pool-driver-{token}-epoch-one");
        store.try_start_run_scoped(&session, &run_id, Some(&execution.persisted_run_scope()))?;
        store.update_run_status(&run_id, "done", None)?;
        assert_that(store.get_run_scope(&run_id)?.is_some_and(|scope| scope.materialization_epoch == Some(1)), "First run did not persist its materialization epoch")?;
        let old_generation = runtime.generation();
        drop(execution);
        close_owned(&owned_project, owned_pid, sink).await?;
        owned_pid = None;
        registry.retire_managed_checkout(runtime.checkout_id()).await.map_err(|error| format!("Retire first pool runtime {owned_project}: {error}"))?;
        drop(runtime);
        sink.emit("pool_first_runtime_retired", json!({"project":owned_project}));
        let accepted = seal_owned_slot(owned_root).map_err(|error| format!("Seal first pool source {owned_project}: {error}"))?;
        sink.emit("pool_first_source_sealed", json!({"project":owned_project,"commit":accepted}));
        let sentinel = owned_root.join("Library/LocusPoolAcceptance.bin");
        std::fs::create_dir_all(sentinel.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&sentinel, format!("slot-cache-{token}")).map_err(|e| e.to_string())?;
        let sentinel_bytes = std::fs::read(&sentinel).map_err(|e| e.to_string())?;
        let sentinel_created = std::fs::metadata(&sentinel).map_err(|e| e.to_string())?.created().ok();
        let sentinel_identity = file_identity(&sentinel)?;
        let artifact_database = owned_root.join("Library/ArtifactDB");
        let artifact_identity = file_identity(&artifact_database)?;
        let artifact_bytes = std::fs::metadata(&artifact_database).map_err(|e| e.to_string())?.len();
        assert_that(artifact_bytes > 0, "First Editor did not produce an import artifact database")?;
        pool::release(source, &first.worktree.checkout_id, request.assignment_id.as_deref().unwrap(), 1)?;
        let next = patch_commit(source, &accepted, &format!("{folder}/Graph.asset"), &next_graph)?;
        request.commit = next.clone(); request.assignment_id = Some(format!("pool-{token}-b"));
        let second = pool::acquire(&request)?;
        assert_that(second.reused && second.preserved_library && second.worktree.root == owned_project, "Second assignment did not reuse the physical project directory")?;
        assert_that(second.worktree.materialization_epoch == 2, "Reassignment did not advance the content epoch")?;
        assert_that(std::fs::read(&sentinel).map_err(|e| e.to_string())? == sentinel_bytes && std::fs::metadata(&sentinel).map_err(|e| e.to_string())?.created().ok() == sentinel_created, "Library cache was copied or recreated during checkout")?;
        assert_that(file_identity(&sentinel)? == sentinel_identity && file_identity(&artifact_database)? == artifact_identity && std::fs::metadata(&artifact_database).map_err(|e| e.to_string())?.len() == artifact_bytes, "Slot checkout replaced the Library sentinel or Unity import artifact database")?;
        let runtime = registry.register(owned_root)?;
        assert_that(runtime.generation() != old_generation, "Reused slot retained the previous runtime generation")?;
        assert_that(matches!(registry.resolve_workspace_ref(&first_workspace_ref), Err(crate::workspace_service::WorkspaceResolveError::StaleMaterialization { .. })), "Old WorkspaceRef was not rejected by materialization identity")?;
        // A renderer or MCP client restored in another process may have the
        // same runtime generation. Reject the persisted content epoch anyway.
        let stale_epoch_ref = crate::workspace_service::WorkspaceRef::for_runtime(runtime.as_ref())
            .with_materialization_epoch(Some(1));
        assert_that(matches!(registry.resolve_workspace_ref(&stale_epoch_ref), Err(crate::workspace_service::WorkspaceResolveError::StaleMaterialization { .. })), "A matching generation bypassed the stale materialization epoch")?;
        let missing_epoch_ref = crate::workspace_service::WorkspaceRef::new(runtime.checkout_id().clone(), Some(runtime.generation()));
        assert_that(matches!(registry.resolve_workspace_ref(&missing_epoch_ref), Err(crate::workspace_service::WorkspaceResolveError::StaleMaterialization { .. })), "Legacy WorkspaceRef without an epoch was accepted for a reused slot")?;
        assert_that(registry.resolve_workspace_ref(&crate::workspace_service::WorkspaceRef::for_runtime(runtime.as_ref())).is_ok(), "Fresh epoch-two WorkspaceRef could not resolve")?;
        {
            let windows = crate::workspace_service::WindowContextRegistry::new();
            assert_that(matches!(windows.restore_background(first_window_context, Arc::clone(&runtime)), Err(crate::workspace_service::WindowContextError::StaleMaterialization { .. })), "An old persisted window silently rebound to the new slot assignment")?;
            let current = windows.focus("pool-acceptance", "main", Arc::clone(&runtime), 2)
                .map_err(|error| error.to_string())?;
            assert_that(current.materialization_epoch == Some(2), "Explicit window selection did not capture epoch two")?;
        }
        assert_that(store.validate_session_materialization(&session, runtime.checkout_id().as_str(), runtime.materialization_epoch()).is_err(), "Old session was allowed to use the new slot assignment")?;
        let second_session = create_workspace_driver_session(app, 101, runtime.as_ref()).await?;
        let execution = registry.execution_context(runtime.checkout_id(), &[crate::workspace_service::ServiceKind::Unity]).await?;
        let second_scope = execution.persisted_run_scope();
        assert_that(store.try_start_run_scoped(&session, &format!("pool-driver-{token}-stale-session"), Some(&second_scope)).is_err(), "An epoch-one session started a run in the epoch-two slot")?;
        let second_run_id = format!("pool-driver-{token}-epoch-two");
        store.try_start_run_scoped(&second_session, &second_run_id, Some(&second_scope))?;
        store.update_run_status(&second_run_id, "done", None)?;
        assert_that(store.get_run_scope(&second_run_id)?.is_some_and(|scope| scope.materialization_epoch == Some(2)) && store.get_run_scope(&run_id)?.is_some_and(|scope| scope.materialization_epoch == Some(1)), "New run binding overwrote the historical materialization epoch")?;
        let connection = ensure_connected(&owned_project, &connect_config, PluginPrepareOutcome::UpToDate, sink, &mut cancel_rx).await?;
        owned_pid = connection.editor_process_id;
        wait_for_unity_editor_idle(&owned_project, &connect_config, sink, &mut cancel_rx).await?;
        let second_report = inspect(&owned_project, &folder).await?;
        assert_that(second_report["incomingValue"] == 88 && second_report["speed"].as_f64() == Some(2.5) && second_report["alternateLabel"] == "source alternate", "Reused Unity cache returned stale asset values")?;
        assert_that(second_report["sharedIdentity"] == true && second_report["cycleIdentity"] == true && second_report["nullPreserved"] == true && second_report["missingTypes"] == false, "Reimported SerializeReference graph lost identity or types")?;
        assert_that(second_report["sharedId"] == "9007199254740993", "Reimport lost the 64-bit managed reference id")?;
        drop(execution);
        close_owned(&owned_project, owned_pid, sink).await?; owned_pid = None;
        registry.retire_managed_checkout(runtime.checkout_id()).await?; drop(runtime);
        let final_commit = seal_owned_slot(owned_root)?;
        pool::release(source, &second.worktree.checkout_id, request.assignment_id.as_deref().unwrap(), 2)?;
        assert_that(before_head == git(source, &["rev-parse", "HEAD"])? && before_index == git(source, &["ls-files", "--stage", "-z"])?, "Pool acceptance modified the supplied source HEAD or index")?;
        Ok(json!({"suite":"project-pool","passed":25,"failed":0,"poolRoot":pool_root,"slot":owned_project,
            "first":first,"second":second,"acceptedImportCommit":accepted,"selectedCommit":next,"finalCommit":final_commit,
            "firstUnity":first_report,"secondUnity":second_report,"oldSession":session,"newSession":second_session,
            "oldSessionRejected":true,"oldWorkspaceRefRejected":true,"oldWindowRestoreRejected":true,
            "missingEpochRejected":true,"newSessionRunEpoch":2,"historicalRunEpoch":1,
            "libraryPreserved":true,"librarySentinelFileId":sentinel_identity,
            "artifactDatabaseFileId":artifact_identity,"artifactDatabaseBytes":artifact_bytes,
            "sourceHeadPreserved":true,"sourceIndexPreserved":true}))
    }.await;
    if outcome.is_err() {
        if let Err(cleanup) = close_owned(&owned_project, owned_pid, sink).await {
            sink.emit(
                "pool_cleanup_warning",
                json!({"project":owned_project,"error":cleanup}),
            );
        }
    }
    let report = outcome?;
    std::fs::write(
        pool_root.join("acceptance.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    sink.emit("suite_result", report);
    Ok(())
}

#[cfg(test)]
mod marker_cleanup_tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn sharing_violation_retries_after_exit_and_rechecks_ownership() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("UnityLockfile");
        std::fs::write(&path, b"owned marker").unwrap();
        let blocker = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            drop(blocker);
        });
        let mut checks = 0;
        remove_closed_editor_marker(&path, || { checks += 1; Ok(()) }).unwrap();
        release.join().unwrap();
        assert!(checks >= 2);
        assert!(!path.exists());
        // Retrying close after a prior failure still handles absent markers.
        remove_closed_editor_marker(&path, || Ok(())).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn a_replacement_editor_aborts_the_retry_without_removing_its_marker() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("UnityLockfile");
        std::fs::write(&path, b"new owner marker").unwrap();
        let blocker = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        let mut checks = 0;
        let error = remove_closed_editor_marker(&path, || {
            checks += 1;
            if checks == 1 { Ok(()) } else { Err("replacement Editor is running".into()) }
        }).unwrap_err();
        assert!(error.contains("replacement Editor"));
        drop(blocker);
        assert_eq!(std::fs::read(path).unwrap(), b"new owner marker");
    }

    #[test]
    fn unexpected_marker_type_reports_its_path_and_preserves_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("UnityLockfile");
        std::fs::create_dir(&path).unwrap();
        let error = remove_closed_editor_marker(&path, || Ok(())).unwrap_err();
        assert!(error.contains(&path.to_string_lossy().to_string()));
        assert!(path.is_dir());
    }
}
