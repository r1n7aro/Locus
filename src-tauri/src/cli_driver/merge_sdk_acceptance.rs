//! Real HTTP SDK -> first-class writable Python tool -> nested merge SDK probe.
//! Only two unique ordinary text paths and an internal fixture ref are created.
use super::*;
use std::io::Write;
use std::process::Stdio;

fn git(
    root: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut command = crate::process_util::command("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "Locus SDK acceptance")
        .env("GIT_AUTHOR_EMAIL", "locus-sdk-test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Locus SDK acceptance")
        .env("GIT_COMMITTER_EMAIL", "locus-sdk-test@example.invalid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or("Fixture Git stdin missing")?
            .write_all(input)
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "SDK fixture Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
fn git_text(
    root: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<String, String> {
    String::from_utf8(git(root, index, args, input)?)
        .map(|s| s.trim_end().to_string())
        .map_err(|e| e.to_string())
}

pub(super) async fn run(
    app: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<Value, String> {
    let root = dunce::canonicalize(git_text(
        Path::new(project),
        None,
        &["rev-parse", "--show-toplevel"],
        None,
    )?)
    .map_err(|e| e.to_string())?;
    let project_root = dunce::canonicalize(project).map_err(|e| e.to_string())?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let filename = format!(".locus-sdk-merge-{token}.txt");
    let unrelated_name = format!(".locus-sdk-unrelated-{token}.txt");
    let target = project_root.join(&filename);
    let unrelated = project_root.join(&unrelated_name);
    if target.exists() || unrelated.exists() {
        return Err("Unique SDK fixture path unexpectedly exists".into());
    }
    let relative = target
        .strip_prefix(&root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let before_head = git_text(&root, None, &["rev-parse", "HEAD"], None)?;
    let before_index = git(&root, None, &["ls-files", "--stage", "-z"], None)?;
    let before_flags = git(&root, None, &["ls-files", "-v", "-z"], None)?;
    let before_dirty = git(&root, None, &["diff", "--binary", "HEAD", "--"], None)?;
    let source_bytes = b"source change through the public nested Python SDK\n";
    let unrelated_bytes = b"pre-existing local untracked content for SDK integration\n";
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = temporary.path().join("index");
    git(&root, Some(&index), &["read-tree", &before_head], None)?;
    let blob = git_text(
        &root,
        None,
        &["hash-object", "-w", "--stdin"],
        Some(source_bytes),
    )?;
    let record = format!("100644 {blob}\t{relative}\0");
    git(
        &root,
        Some(&index),
        &["update-index", "-z", "--index-info"],
        Some(record.as_bytes()),
    )?;
    let tree = git_text(&root, Some(&index), &["write-tree"], None)?;
    let commit = git_text(
        &root,
        None,
        &["commit-tree", &tree, "-p", &before_head],
        Some(b"Locus nested SDK integration acceptance\n"),
    )?;
    let fixture_ref = format!("refs/locus/driver/sdk-merge/{token}");
    git(
        &root,
        None,
        &[
            "update-ref",
            &fixture_ref,
            &commit,
            &"0".repeat(commit.len()),
        ],
        None,
    )?;
    std::fs::write(&unrelated, unrelated_bytes).map_err(|e| e.to_string())?;
    let runtime = app
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .register(&project_root)?;
    let reference = crate::workspace_service::WorkspaceRef::for_runtime(&runtime);
    let inner = format!(
        r#"
import json, os, pathlib
assert os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"), "Writable Python tool did not receive an active execution delegation"
target = pathlib.Path({target})
unrelated = pathlib.Path({unrelated})
before = unrelated.read_bytes()
job = await locus.merges.prepare(sources=[{{"commits":[{commit}]}}], workspace_ref=workspace_ref)
assert job.workspace_ref["checkoutId"] == workspace_ref.checkout_id, "Merge SDK changed destination checkout"
assert job.workspace_ref.get("expectedMaterializationEpoch") == workspace_ref.expected_materialization_epoch, "Merge SDK changed checkout materialization epoch"
plan = await job.new_plan(default="keep_target")
empty = await plan.preview()
assert not empty.files, "New plans must not silently include source changes"
catalog = await job.changes(limit=1000)
ids = [change["id"] for change in catalog.changes if change["path"] == {relative}]
assert len(ids) == 1, "Fixture source delta is missing or ambiguous"
await plan.include(change_ids=ids)
preview = await plan.preview()
assert preview.ready_to_apply, str(preview.issues)
applied = False
try:
    await plan.apply(expected_plan_hash=preview.plan_hash)
    applied = True
    assert target.read_bytes() == {source_bytes}, "SDK applied the wrong source bytes"
    retry = await plan.apply(expected_plan_hash=preview.plan_hash)
    assert retry.idempotent, "Successful SDK apply retry was not idempotent"
    assert unrelated.read_bytes() == before, "SDK merge changed unrelated dirty content"
    await plan.abort()
    applied = False
    assert not target.exists(), "Guarded abort did not restore the originally absent file"
    assert unrelated.read_bytes() == before, "Abort changed unrelated dirty content"
finally:
    if applied:
        await plan.abort()
print("LOCUS_MERGE_SDK_ACCEPTANCE:" + json.dumps({{"passed":True,"job_id":job.id,"checkout_id":job.workspace_ref["checkoutId"],"delegation_active":True,"default_kept_target":True,"idempotent_apply":True,"guarded_abort":True}}))
"#,
        target = json!(target.to_string_lossy()),
        unrelated = json!(unrelated.to_string_lossy()),
        commit = json!(commit),
        relative = json!(relative),
        source_bytes = format!(
            "{}.encode('utf-8')",
            json!(String::from_utf8_lossy(source_bytes))
        )
    );
    let script = r#"import asyncio,json,locus,sys
async def main():
    reference=locus.WorkspaceRef.from_payload(json.loads(sys.argv[2]))
    result=await locus.call_tool("python",{"readonly":False,"timeout":240000,"code":sys.argv[3]},timeout=245,workspace_ref=reference)
    result.raise_for_error()
    print(json.dumps({"tool":result.name,"output":result.output,"is_error":result.is_error}))
asyncio.run(main())
"#;
    sink.emit(
        "merge_sdk_acceptance_start",
        json!({"project":project,"fixture_ref":fixture_ref,"commit":commit,"path":relative}),
    );
    let result = run_python_sdk_script(
        app,
        project,
        script,
        &[
            serde_json::to_string(&reference).map_err(|e| e.to_string())?,
            inner,
        ],
        config.suite_timeout.max(Duration::from_secs(300)),
        "Nested Python merge SDK acceptance",
    )
    .await;
    // Remove only exact fixture bytes. Later external edits are retained.
    let unrelated_intact = std::fs::read(&unrelated)
        .map(|bytes| bytes == unrelated_bytes)
        .unwrap_or(false);
    if unrelated_intact {
        std::fs::remove_file(&unrelated).map_err(|e| e.to_string())?;
    }
    let target_restored = !target.exists();
    if !target_restored
        && std::fs::read(&target)
            .map(|bytes| bytes == source_bytes)
            .unwrap_or(false)
    {
        std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    }
    let output = result?;
    let payload: Value = serde_json::from_str(output.trim())
        .map_err(|e| format!("SDK acceptance returned invalid JSON: {e}; {output}"))?;
    let tool_output = payload["output"]
        .as_str()
        .ok_or("SDK acceptance omitted Python tool output")?;
    let line = tool_output
        .lines()
        .find_map(|line| line.strip_prefix("LOCUS_MERGE_SDK_ACCEPTANCE:"))
        .ok_or_else(|| format!("SDK acceptance marker missing: {tool_output}"))?;
    let mut details: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let preserved = before_head == git_text(&root, None, &["rev-parse", "HEAD"], None)?
        && before_index == git(&root, None, &["ls-files", "--stage", "-z"], None)?
        && before_flags == git(&root, None, &["ls-files", "-v", "-z"], None)?
        && before_dirty == git(&root, None, &["diff", "--binary", "HEAD", "--"], None)?;
    if !preserved || !target_restored || !unrelated_intact || details["passed"] != true {
        return Err(format!("SDK acceptance failed preservation checks: git={preserved}, target={target_restored}, unrelated={unrelated_intact}, details={details}"));
    }
    details["head_index_dirty_preserved"] = json!(true);
    details["fixture_ref"] = json!(fixture_ref);
    details["source_commit"] = json!(commit);
    details["passed_checks"] = json!(8);
    sink.emit("merge_sdk_acceptance_result", &details);
    Ok(details)
}
