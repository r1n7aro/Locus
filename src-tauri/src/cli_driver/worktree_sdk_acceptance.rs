//! Public Python SDK acceptance. All writes and Editor closes target newly
//! created siblings; the supplied source's HEAD, index and files are preserved.
use super::*;

pub(super) async fn run(
    app: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let project = resolve_project_path(config.project_path.as_deref(), app).await?;
    set_workspace_for_driver(app, &project).await?;
    let registry = app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
    let runtime = registry.register(&project)?;
    let root = std::env::var_os("LOCUS_WORKTREE_SDK_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(if cfg!(windows) {
                "E:/LocusTemp"
            } else {
                "/tmp"
            })
        })
        .join(format!("worktree-sdk-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let timeout = config.suite_timeout.as_secs().clamp(60, 1800);
    let inner = format!(
        "fixture_root = {}\nsdk_timeout = {}\n{}",
        json!(root.to_string_lossy()),
        config.connect_timeout.as_secs().clamp(30, 1800),
        include_str!("worktree_sdk_acceptance.py.txt")
    );
    let script = r#"import asyncio,json,locus,sys
async def main():
    reference=locus.WorkspaceRef.from_payload(json.loads(sys.argv[2]))
    timeout=float(sys.argv[4])
    result=await locus.call_tool("python", {"readonly":False, "timeout":int(timeout*1000),
        "code":sys.argv[3]}, timeout=timeout+5, workspace_ref=reference)
    result.raise_for_error()
    print(result.output)
asyncio.run(main())
"#;
    sink.emit(
        "suite_start",
        json!({"suite":"worktree-sdk", "project":project, "fixtureRoot":root}),
    );
    let output = run_python_sdk_script(
        app,
        &project,
        script,
        &[
            serde_json::to_string(&crate::workspace_service::WorkspaceRef::for_runtime(
                &runtime,
            ))
            .map_err(|e| e.to_string())?,
            inner,
            timeout.to_string(),
        ],
        Duration::from_secs(timeout + 15),
        "Worktree SDK acceptance",
    )
    .await;
    match output {
        Ok(output) => {
            std::fs::write(root.join("python.log"), &output).map_err(|e| e.to_string())?;
            let result = output
                .lines()
                .find_map(|line| line.strip_prefix("LOCUS_WORKTREE_SDK_ACCEPTANCE:"))
                .ok_or_else(|| format!("SDK acceptance marker missing: {output}"))?;
            let result: Value = serde_json::from_str(result).map_err(|e| e.to_string())?;
            sink.emit(
                "suite_result",
                json!({"suite":"worktree-sdk", "passed":result["passed"],
                "failed":0, "fixtureRoot":root, "details":result}),
            );
            Ok(())
        }
        Err(error) => {
            std::fs::write(root.join("error.log"), &error).map_err(|e| e.to_string())?;
            sink.emit(
                "suite_result",
                json!({"suite":"worktree-sdk", "passed":0, "failed":1,
                "fixtureRoot":root, "error":error}),
            );
            Err(error)
        }
    }
}
