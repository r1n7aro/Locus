//! A real, cold worktree development loop through the public Python SDK.
//! No SDK ensure/start call is used: ordinary Unity tools request the managed
//! Editor. Each phase uses a new Python process, like separate Agent turns.
use super::*;
use crate::workspace_service::{worktrees, ProjectRegistry, WorkspaceRef};

fn write(root: &Path, path: &str, content: &str) -> Result<(), String> {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, content).map_err(|error| error.to_string())
}

async fn phase(
    app: &AppHandle,
    source: &str,
    reference: &WorkspaceRef,
    name: &str,
    target: &str,
    sink: &DriverEventSink,
) -> Result<Value, String> {
    sink.emit(
        "phase",
        json!({"suite":"headless-development", "phase":name}),
    );
    let output = run_python_sdk_script(
        app,
        source,
        include_str!("headless_development.py.txt"),
        &[
            serde_json::to_string(reference).map_err(|error| error.to_string())?,
            name.into(),
            target.into(),
        ],
        Duration::from_secs(1200),
        "Headless development loop",
    )
    .await?;
    let payload = output
        .lines()
        .find_map(|line| line.strip_prefix("LOCUS_HEADLESS_PHASE:"))
        .ok_or_else(|| format!("Phase {name} returned no evidence: {output}"))?;
    serde_json::from_str(payload).map_err(|error| error.to_string())
}

fn require(value: bool, message: &str) -> Result<(), String> {
    if value {
        Ok(())
    } else {
        Err(message.into())
    }
}

async fn wait_stopped(project: &str, pid: u32) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let current = unity_bridge::query_current_project_editor_process(project).await;
        if current.state == UnityEditorProcessState::NotRunning {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Idle TTL did not release headless Editor {pid}: {:?}",
                current
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub(super) async fn run(
    app: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let reference_project = resolve_project_path(config.project_path.as_deref(), app).await?;
    let version = unity_bridge::read_project_unity_version(&reference_project)?
        .ok_or("Missing Unity version")?;
    let root = std::env::var_os("LOCUS_HEADLESS_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("E:/LocusTemp"))
        .join(format!("headless-loop-{}", uuid::Uuid::new_v4().simple()));
    let source = root.join("source");
    let target = root.join("worktree");
    std::fs::create_dir_all(&source).map_err(|error| error.to_string())?;
    write(&source, ".gitignore", "/Library/\n/Temp/\n/Logs/\n/obj/\n/UserSettings/\n/Packages/com.farlocus.locus/\n/*.sln\n/*.csproj\n")?;
    write(
        &source,
        "ProjectSettings/ProjectVersion.txt",
        &format!("m_EditorVersion: {version}\n"),
    )?;
    write(
        &source,
        "Packages/manifest.json",
        "{\"dependencies\":{\"com.unity.test-framework\":\"1.7.0\"}}",
    )?;
    write(
        &source,
        "Locus/config.json",
        "{\"workspace_id\":\"headless-loop\",\"unity_test_tools_enabled\":true}",
    )?;
    write(
        &source,
        "Assets/Runtime/HeadlessLoop.Runtime.asmdef",
        "{\"name\":\"HeadlessLoop.Runtime\"}",
    )?;
    write(&source, "Assets/Runtime/LoopLogic.cs", "namespace HeadlessLoop { public static class LoopLogic { public static int Add(int left, int right) { return left - right; } } }")?;
    for args in [
        vec!["init"],
        vec!["config", "user.name", "Locus Headless Test"],
        vec!["config", "user.email", "headless-test@example.invalid"],
        vec!["add", "--all"],
        vec![
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Headless development fixture",
        ],
    ] {
        worktrees::git(&source, &args)?;
    }
    let registry = app.state::<Arc<ProjectRegistry>>();
    let source_runtime = registry.register(&source)?;
    let source_ref = WorkspaceRef::for_runtime(&source_runtime);
    let source = source.to_string_lossy().into_owned();
    let target = target.to_string_lossy().into_owned();
    let policy = registry.resource_policy();
    let original_limits = policy.snapshot().limits;
    let mut limits = original_limits.clone();
    limits.service_idle_timeout_secs = 60;
    policy
        .update(limits.clone())
        .map_err(|error| error.to_string())?;
    registry.notify_policy_changed();
    let foreground = unity_bridge::managed_editor::resources()
        .await?
        .into_iter()
        .filter(|row| row.mode == unity_bridge::UnityLaunchMode::Interactive)
        .map(|row| {
            (
                row.process_id,
                unity_bridge::launched_unity_process_created_at_ms(row.process_id),
            )
        })
        .collect::<Vec<_>>();
    let mut report = json!({"root":root,"version":version,"foregroundPids":foreground.iter().map(|row|row.0).collect::<Vec<_>>(),"phases":[]});
    sink.emit(
        "suite_start",
        json!({"suite":"headless-development","fixtureRoot":root}),
    );
    let result: Result<(), String> = async {
        let created = phase(app, &source, &source_ref, "create", &target, sink).await?;
        let reference: WorkspaceRef = serde_json::from_value(json!({
            "checkoutId":created["worktree"]["checkout_id"],
            "expectedGeneration":created["worktree"]["workspace_ref"]["expected_generation"],
            "expectedMaterializationEpoch":created["worktree"]["materialization_epoch"],
        }))
        .map_err(|error| error.to_string())?;
        report["phases"].as_array_mut().unwrap().push(created);
        let mut first_pid = 0;
        for name in ["start", "develop", "repair"] {
            let evidence = phase(app, &source, &reference, name, &target, sink).await?;
            let pid = evidence["process_id"]
                .as_u64()
                .ok_or("Missing Editor PID")? as u32;
            if first_pid == 0 {
                first_pid = pid;
            }
            require(
                pid == first_pid,
                "A new Editor was launched between SDK turns",
            )?;
            report["phases"].as_array_mut().unwrap().push(evidence);
            write(
                &root,
                "acceptance.json",
                &serde_json::to_string_pretty(&report).unwrap(),
            )?;
        }
        let resources = unity_bridge::managed_editor::resources().await?;
        require(
            resources.iter().any(|row| {
                row.process_id == first_pid
                    && row.managed
                    && row.working_set_bytes.is_some_and(|bytes| bytes > 0)
            }),
            "Managed Editor memory was unavailable",
        )?;
        report["resources"] = serde_json::to_value(resources).unwrap();
        limits.service_idle_timeout_secs = 2;
        policy
            .update(limits.clone())
            .map_err(|error| error.to_string())?;
        let play = phase(app, &source, &reference, "play", &target, sink);
        let during = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let runtime = registry
                .runtime(&reference.checkout_id)
                .ok_or("Missing checkout")?;
            require(
                runtime
                    .activity_snapshot(Duration::from_secs(2))
                    .running_task_leases
                    > 0,
                "Test request did not hold a checkout lease",
            )?;
            require(
                unity_bridge::query_current_project_editor_process(&target)
                    .await
                    .process_id
                    == Some(first_pid),
                "TTL interrupted an active PlayMode test",
            )
        };
        let (play, during) = tokio::join!(play, during);
        during?;
        report["phases"].as_array_mut().unwrap().push(play?);
        report["activeTestProtected"] = json!(true);
        report["phases"]
            .as_array_mut()
            .unwrap()
            .push(phase(app, &source, &reference, "dirty", &target, sink).await?);
        tokio::time::sleep(Duration::from_secs(7)).await;
        let retained = unity_bridge::managed_editor::resources().await?;
        require(
            retained.iter().any(|row| {
                row.process_id == first_pid
                    && row
                        .last_error
                        .as_deref()
                        .is_some_and(|error| error.contains("Unsaved"))
            }),
            "Unsaved scene did not prevent idle shutdown",
        )?;
        report["unsavedSceneProtected"] = json!(true);
        report["phases"]
            .as_array_mut()
            .unwrap()
            .push(phase(app, &source, &reference, "save", &target, sink).await?);
        let target_runtime = registry
            .runtime(&reference.checkout_id)
            .ok_or("Missing checkout")?;
        let _visible_pane = target_runtime
            .acquire_lease(crate::workspace_service::runtime::WorkspaceLeaseKind::VisiblePane);
        wait_stopped(&target, first_pid).await?;
        report["ttlReleased"] = json!(true);
        report["visiblePaneAllowsIdleRelease"] = json!(true);
        let restarted = phase(app, &source, &reference, "restart", &target, sink).await?;
        let second_pid = restarted["process_id"]
            .as_u64()
            .ok_or("Missing restarted PID")? as u32;
        require(
            second_pid != first_pid,
            "Editor was not relaunched after TTL",
        )?;
        report["phases"].as_array_mut().unwrap().push(restarted);
        wait_stopped(&target, second_pid).await?;
        for (pid, created) in &foreground {
            require(
                unity_bridge::launched_unity_process_liveness(*pid, *created)?
                    == unity_bridge::UnityProcessIdentityLiveness::Alive,
                "A user-managed foreground Editor changed",
            )?;
        }
        report["foregroundPreserved"] = json!(true);
        Ok(())
    }
    .await;
    if result.is_err() && Path::new(&target).is_dir() {
        // Only this suite's owned checkout; retain source and logs on failure.
        if let Err(error) =
            unity_bridge::close_current_project_unity_processes(&target, Duration::from_secs(30))
                .await
        {
            report["cleanupError"] = json!(error);
        }
    }
    policy
        .update(original_limits)
        .map_err(|error| error.to_string())?;
    report["error"] = json!(result.as_ref().err());
    write(
        &root,
        "acceptance.json",
        &serde_json::to_string_pretty(&report).unwrap(),
    )?;
    sink.emit("suite_result", json!({"suite":"headless-development","passed":report["phases"].as_array().map_or(0,Vec::len),"failed":usize::from(result.is_err()),"fixtureRoot":root,"details":report}));
    result
}
