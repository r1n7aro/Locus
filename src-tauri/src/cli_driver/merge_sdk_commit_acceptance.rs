//! Real Unity-created material -> public Python selective merge -> isolated
//! candidate Editor validation -> exact scoped commit in an owned new worktree.
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
        .env("GIT_AUTHOR_NAME", "Locus SDK scope acceptance")
        .env("GIT_AUTHOR_EMAIL", "locus-sdk-test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Locus SDK scope acceptance")
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
            .ok_or("Missing fixture stdin")?
            .write_all(input)
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(output.stdout)
}
fn text(
    root: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<String, String> {
    String::from_utf8(git(root, index, args, input)?)
        .map(|s| s.trim_end().to_string())
        .map_err(|e| e.to_string())
}
fn commit_files(
    repo: &Path,
    parent: &str,
    files: &[(String, Vec<u8>)],
    message: &str,
) -> Result<String, String> {
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = temporary.path().join("index");
    git(repo, Some(&index), &["read-tree", parent], None)?;
    for (path, bytes) in files {
        let oid = text(
            repo,
            None,
            &["hash-object", "-w", "--stdin", "--path", path],
            Some(bytes),
        )?;
        git(
            repo,
            Some(&index),
            &["update-index", "-z", "--index-info"],
            Some(format!("100644 {oid}\t{path}\0").as_bytes()),
        )?;
    }
    let tree = text(repo, Some(&index), &["write-tree"], None)?;
    text(
        repo,
        None,
        &["commit-tree", &tree, "-p", parent],
        Some(message.as_bytes()),
    )
}

pub(super) async fn run(
    app: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<Value, String> {
    let repo = dunce::canonicalize(text(
        Path::new(project),
        None,
        &["rev-parse", "--show-toplevel"],
        None,
    )?)
    .map_err(|e| e.to_string())?;
    let project_root = dunce::canonicalize(project).map_err(|e| e.to_string())?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let folder = format!("Assets/LocusSdkScope-{token}");
    let selected = format!("{folder}/Selected.mat");
    let unrelated = format!("{folder}/Unrelated.mat");
    let marker = format!(".locus-sdk-staged-{token}.txt");
    if project_root.join(&folder).exists() {
        return Err("Unique Unity SDK fixture folder already exists".into());
    }
    let before_head = text(&repo, None, &["rev-parse", "HEAD"], None)?;
    let before_index = git(&repo, None, &["ls-files", "--stage", "-z"], None)?;
    let before_flags = git(&repo, None, &["ls-files", "-v", "-z"], None)?;
    let before_dirty = git(&repo, None, &["diff", "--binary", "HEAD", "--"], None)?;
    execute_capture(project,&format!(r#"
var shader=UnityEngine.Shader.Find("Hidden/InternalErrorShader");
if(shader==null)throw new System.Exception("Built-in fixture shader is unavailable");
UnityEditor.AssetDatabase.CreateFolder("Assets",{folder_name});
var selected=new UnityEngine.Material(shader);selected.name="Locus SDK selected";selected.renderQueue=2100;
UnityEditor.AssetDatabase.CreateAsset(selected,{selected});UnityEditor.AssetDatabase.SaveAssetIfDirty(selected);
var unrelated=new UnityEngine.Material(shader);unrelated.name="Locus SDK unrelated";unrelated.renderQueue=2200;
UnityEditor.AssetDatabase.CreateAsset(unrelated,{unrelated});UnityEditor.AssetDatabase.SaveAssetIfDirty(unrelated);
print("LOCUS_SCOPE_BASE_READY");
"#,folder_name=json!(format!("LocusSdkScope-{token}")),selected=json!(selected),unrelated=json!(unrelated))).await?;
    let relative = |path: &str| -> Result<String, String> {
        Ok(project_root
            .join(path)
            .strip_prefix(&repo)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/"))
    };
    let fixture_paths = vec![
        selected.clone(),
        format!("{selected}.meta"),
        unrelated.clone(),
        format!("{unrelated}.meta"),
        format!("{folder}.meta"),
    ];
    let base_files = fixture_paths
        .iter()
        .map(|path| {
            Ok((
                relative(path)?,
                std::fs::read(project_root.join(path)).map_err(|e| e.to_string())?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let baseline = commit_files(
        &repo,
        &before_head,
        &base_files,
        "Unity-created material scope baseline",
    )?;
    execute_capture(project,&format!(r#"var asset=UnityEditor.AssetDatabase.LoadAssetAtPath<UnityEngine.Material>({selected});asset.renderQueue=2400;UnityEditor.EditorUtility.SetDirty(asset);UnityEditor.AssetDatabase.SaveAssetIfDirty(asset);print("LOCUS_SCOPE_SOURCE_READY");"#,selected=json!(selected))).await?;
    let expected_selected =
        std::fs::read(project_root.join(&selected)).map_err(|e| e.to_string())?;
    let source = commit_files(
        &repo,
        &baseline,
        &[(relative(&selected)?, expected_selected.clone())],
        "Selected material queue change",
    )?;
    let fixture_ref = format!("refs/locus/driver/sdk-scope/{token}");
    git(
        &repo,
        None,
        &[
            "update-ref",
            &fixture_ref,
            &source,
            &"0".repeat(source.len()),
        ],
        None,
    )?;
    execute_capture(project,&format!(r#"
var selected=UnityEditor.AssetDatabase.LoadAssetAtPath<UnityEngine.Material>({selected});selected.renderQueue=2100;UnityEditor.EditorUtility.SetDirty(selected);UnityEditor.AssetDatabase.SaveAssetIfDirty(selected);
var unrelated=UnityEditor.AssetDatabase.LoadAssetAtPath<UnityEngine.Material>({unrelated});unrelated.renderQueue=3000;UnityEditor.EditorUtility.SetDirty(unrelated);UnityEditor.AssetDatabase.SaveAssetIfDirty(unrelated);print("LOCUS_SCOPE_DIRTY_TARGET_READY");
"#,selected=json!(selected),unrelated=json!(unrelated))).await?;
    let expected_unrelated =
        std::fs::read(project_root.join(&unrelated)).map_err(|e| e.to_string())?;
    let runtime = app
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .register(&project_root)?;
    let reference = crate::workspace_service::WorkspaceRef::for_runtime(&runtime);
    let scope_paths = vec![
        relative(&selected)?,
        relative(&format!("{selected}.meta"))?,
        relative(&format!("{folder}.meta"))?,
    ];
    use sha2::Digest;
    let selected_hash: String = sha2::Sha256::digest(&expected_selected)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let unrelated_hash: String = sha2::Sha256::digest(&expected_unrelated)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let inner = format!(
        r#"
import json,os,pathlib,subprocess,hashlib
assert os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"), "Missing real outer mutation delegation"
bootstrap=await locus.merges.prepare(sources=[{{"commits":[{source}]}}],destination={{"kind":"new_branch","name":{branch},"location":"new_worktree","base_workspace_ref":workspace_ref}})
destination=pathlib.Path(bootstrap.destination)
target_ref=bootstrap.workspace_ref
await bootstrap.plan().abort()
env=dict(os.environ,GIT_OPTIONAL_LOCKS="0")
def git(*args,check=True):
    return subprocess.run(["git","-C",str(destination),*args],env=env,check=check,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
marker=destination/{marker}
marker.write_bytes(b"independent staged fixture\n")
git("add","--",{marker})
marker.write_bytes(b"independent unstaged fixture\n")
selected=destination/{selected}
unrelated=destination/{unrelated}
unrelated_before=unrelated.read_bytes()
assert hashlib.sha256(unrelated_before).hexdigest()=={unrelated_hash}, "New destination lost the base's dirty material"
index_before=git("ls-files","--stage","-z").stdout
head_before=git("rev-parse","HEAD").stdout.strip().decode()
job=await locus.merges.prepare(sources=[{{"commits":[{source}]}}],workspace_ref=target_ref)
plan=await job.new_plan(default="keep_target")
await plan.include(path={selected_relative})
preview=await plan.preview()
assert preview.ready_to_apply,str(preview.issues)
await plan.apply(expected_plan_hash=preview.plan_hash)
assert hashlib.sha256(selected.read_bytes()).hexdigest()=={selected_hash}, "Typed material merge lost source data"
assert unrelated.read_bytes()==unrelated_before, "Apply touched the unrelated dirty material"
assert git("ls-files","--stage","-z").stdout==index_before, "Apply changed staged state"
scope={scope_paths}
checked=await plan.validate(level="unity",paths=scope,include_local_changes=True)
assert checked.unity_validated and checked.placement=="isolated_commit_candidate",str(checked)
assert checked.details.get("validated_tree")==checked.tree,str(checked)
committed=await plan.commit(paths=scope,message="Locus acceptance: exact validated material subset",include_local_changes=True)
assert git("rev-parse","HEAD^1").stdout.strip().decode()==head_before, "Selective commit did not retain a single parent"
assert git("rev-parse","HEAD^2",check=False).returncode!=0, "Selective commit incorrectly merged excluded source history"
assert git("rev-parse","HEAD^{{tree}}").stdout.strip().decode()==checked.tree, "Committed a tree different from the verified candidate"
assert unrelated.read_bytes()==unrelated_before, "Scoped commit touched excluded dirty asset"
assert marker.read_bytes()==b"independent unstaged fixture\n", "Scoped commit changed unrelated working bytes"
assert git("show",":"+{marker_relative}).stdout==b"independent staged fixture\n", "Scoped commit consumed unrelated staging"
assert git("cat-file","-e","HEAD:"+{unrelated_relative},check=False).returncode!=0, "Commit included the unrelated dirty asset"
assert git("cat-file","-e","HEAD:"+{marker_relative},check=False).returncode!=0, "Commit included the unrelated staged file"
def without_scope(records):
    return [r for r in records.split(b"\0") if r and r.split(b"\t",1)[1].decode() not in scope]
assert without_scope(git("ls-files","--stage","-z").stdout)==without_scope(index_before), "Commit changed other index entries"
print("LOCUS_MERGE_SCOPE_ACCEPTANCE:"+json.dumps({{"passed":True,"job_id":job.id,"destination":str(destination),"commit":committed.commit,"validated_tree":checked.tree,"scratch":checked.details,"scope":scope,"unrelated_dirty_and_staged_preserved":True,"exact_tree_committed":True}}))
"#,
        source = json!(source),
        branch = json!(format!("codex/locus-sdk-subset-{token}")),
        marker = json!(marker),
        selected = json!(selected),
        unrelated = json!(unrelated),
        selected_relative = json!(relative(&selected)?),
        selected_hash = json!(selected_hash),
        scope_paths = json!(scope_paths),
        unrelated_hash = json!(unrelated_hash),
        unrelated_relative = json!(relative(&unrelated)?),
        marker_relative = json!(relative(&marker)?)
    );
    let outer = r#"import asyncio,json,locus,sys
async def main():
    ref=locus.WorkspaceRef.from_payload(json.loads(sys.argv[2]))
    result=await locus.call_tool("python",{"readonly":False,"timeout":1500000,"code":sys.argv[3]},timeout=1505,workspace_ref=ref)
    result.raise_for_error()
    print(json.dumps({"output":result.output,"is_error":result.is_error}))
asyncio.run(main())
"#;
    sink.emit(
        "merge_sdk_scope_start",
        json!({"project":project,"fixture":folder,"source_commit":source}),
    );
    let started = Instant::now();
    let arguments = [
        serde_json::to_string(&reference).map_err(|e| e.to_string())?,
        inner,
    ];
    let operation = run_python_sdk_script(
        app,
        project,
        outer,
        &arguments,
        config.suite_timeout.max(Duration::from_secs(1800)),
        "Unity SDK candidate commit scope acceptance",
    );
    tokio::pin!(operation);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    let result = loop {
        tokio::select! {result=&mut operation=>break result,_=progress.tick()=>sink.emit("merge_sdk_scope_progress",json!({"elapsed_seconds":started.elapsed().as_secs(),"stage":"isolated candidate Unity validation and scoped commit"}))}
    };
    let cleanup=execute_capture(project,&format!("if (!UnityEditor.AssetDatabase.DeleteAsset({})) throw new System.Exception(\"Fixture cleanup failed\"); print(\"LOCUS_SCOPE_FIXTURE_REMOVED\");",json!(folder))).await;
    let output = result?;
    cleanup?;
    let payload: Value = serde_json::from_str(output.trim())
        .map_err(|e| format!("Scope SDK JSON error: {e}; {output}"))?;
    let text = payload["output"]
        .as_str()
        .ok_or("Scope SDK omitted tool output")?;
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("LOCUS_MERGE_SCOPE_ACCEPTANCE:"))
        .ok_or_else(|| format!("Scope SDK marker missing: {text}"))?;
    let mut details: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if before_head != self::text(&repo, None, &["rev-parse", "HEAD"], None)?
        || before_index != git(&repo, None, &["ls-files", "--stage", "-z"], None)?
        || before_flags != git(&repo, None, &["ls-files", "-v", "-z"], None)?
        || before_dirty != git(&repo, None, &["diff", "--binary", "HEAD", "--"], None)?
    {
        return Err("Scoped SDK probe changed source HEAD/index/dirty tracked files".into());
    }
    details["source_preserved"] = json!(true);
    details["fixture_ref"] = json!(fixture_ref);
    details["passed_checks"] = json!(12);
    sink.emit("merge_sdk_scope_result", &details);
    Ok(details)
}
