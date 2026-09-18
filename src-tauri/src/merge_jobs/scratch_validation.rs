//! Real-Editor validation of an immutable commit candidate. The user's dirty
//! checkout is never materialized, staged, or used as the validation project.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::Manager;

use super::eol_proof::{self, Snapshot};
use crate::unity_bridge::{self, UnityEditorProcessState, UnityLaunchMode, UnityLaunchResult};
use crate::workspace_service::{pool, worktrees, CheckoutId, ProjectRegistry};

fn immutable_commit(repo: &Path, tree: &str, parent: &str) -> Result<String, String> {
    for oid in [tree, parent] {
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Scratch validation requires full immutable Git object ids".into());
        }
    }
    if worktrees::git_text(repo, &["cat-file", "-t", tree])? != "tree"
        || worktrees::git_text(repo, &["cat-file", "-t", parent])? != "commit"
    {
        return Err("Scratch validation requires a tree and its parent commit".into());
    }
    let result = crate::process_util::command("git")
        .arg("-C")
        .arg(repo)
        .args([
            "commit-tree",
            tree,
            "-p",
            parent,
            "-m",
            "Locus isolated merge validation candidate",
        ])
        .env("GIT_AUTHOR_NAME", "Locus Validation")
        .env("GIT_AUTHOR_EMAIL", "validation@locus.local")
        .env("GIT_COMMITTER_NAME", "Locus Validation")
        .env("GIT_COMMITTER_EMAIL", "validation@locus.local")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into());
    }
    String::from_utf8(result.stdout)
        .map(|s| s.trim().into())
        .map_err(|e| e.to_string())
}

/// Hash actual checkout bytes, including hydrated LFS and checkout line endings.
/// Enumerating the source directories also catches ignored/new assets and .meta
/// files that Git's ordinary untracked-file listing would hide.
fn source_snapshot(
    repo: &Path,
    project: &Path,
    overlay: Option<&Path>,
) -> Result<Snapshot, String> {
    // Windows may simplify a short root but retain the verbatim prefix of a
    // long child. Compare canonical components in one representation instead
    // of treating that formatting difference as an escape from the checkout.
    let canonical_repo = std::fs::canonicalize(repo).map_err(|e| {
        format!(
            "Could not resolve validation checkout {}: {e}",
            repo.display()
        )
    })?;
    let mut files = BTreeSet::new();
    for bytes in worktrees::git(repo, &["ls-files", "-z"])?
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
    {
        let name = String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
        files.insert(repo.join(worktrees::safe_relative(&name)?));
    }
    for directory in ["Assets", "Packages", "ProjectSettings"] {
        let root = project.join(directory);
        if !root.exists() {
            continue;
        }
        for item in walkdir::WalkDir::new(&root).follow_links(false) {
            let item = item.map_err(|e| e.to_string())?;
            if item.file_type().is_symlink() {
                return Err(format!(
                    "Validation source contains a symbolic link: {}",
                    item.path().display()
                ));
            }
            if item.file_type().is_file() {
                files.insert(item.into_path());
            }
        }
    }
    let mut result = BTreeMap::new();
    for path in files {
        if let Some(root) = overlay {
            if worktrees::path_relative(root, &path)?.is_some()
                || worktrees::path_components_equal(&path, &root.with_extension("meta"))?
            {
                continue;
            }
        }
        let name = worktrees::path_relative(repo, &path)?
            .ok_or_else(|| format!("Validation source escapes its checkout: {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !path.exists() {
            result.insert(name, None);
            continue;
        }
        let canonical_path = std::fs::canonicalize(&path).map_err(|e| {
            format!(
                "Could not resolve validation source {}: {e}",
                path.display()
            )
        })?;
        if worktrees::path_relative(&canonical_repo, &canonical_path)?.is_none()
            || std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err(format!("Validation source escapes its checkout: {name}"));
        }
        result.insert(name, Some(eol_proof::fingerprint(&path)?));
    }
    Ok(result)
}

/// Resolve only an exact tool-owned path before cleanup. Comparing its actual
/// canonical location to the expected path catches links within the checkout
/// as well as links outside it, while accepting Windows verbatim equivalents.
fn owned_validation_path(project: &Path, relative: &str) -> Result<PathBuf, String> {
    let project = std::fs::canonicalize(project).map_err(|e| {
        format!(
            "Could not resolve validation project {}: {e}",
            project.display()
        )
    })?;
    let expected = project.join(worktrees::safe_relative(relative)?);
    let metadata = std::fs::symlink_metadata(&expected).map_err(|e| {
        format!(
            "Could not inspect validation path {}: {e}",
            expected.display()
        )
    })?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0
    };
    let actual = std::fs::canonicalize(&expected).map_err(|e| {
        format!(
            "Could not resolve validation path {}: {e}",
            expected.display()
        )
    })?;
    if linked
        || worktrees::path_relative(&project, &actual)?.is_none()
        || !worktrees::path_components_equal(&expected, &actual)?
    {
        return Err(format!(
            "Validation path changed or escapes its checkout: {}",
            expected.display()
        ));
    }
    Ok(actual)
}

fn changed_paths(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

/// Refresh only the stat entries of already-proved EOL saves in the closed
/// validation checkout. A shadow index prevents Git from staging any changed
/// blob, mode or flags even if the on-disk inputs race the refresh.
fn refresh_proved_eol_index(
    repo: &Path,
    project: &Path,
    tree: &str,
    snapshot: &Snapshot,
    evidence: &[eol_proof::EolNormalization],
    expected_records: &[u8],
    proof_dir: &Path,
) -> Result<Vec<String>, String> {
    if evidence.is_empty() {
        return Ok(Vec::new());
    }
    let paths: BTreeSet<_> = evidence.iter().map(|item| item.path.clone()).collect();
    if paths.len() != evidence.len() {
        return Err("EOL stat refresh has duplicate proof paths".into());
    }
    for item in evidence {
        worktrees::safe_relative(&item.path)?;
        let fingerprint = snapshot.get(&item.path).and_then(Option::as_ref)
            .ok_or("EOL stat refresh is missing the final source fingerprint")?;
        let record = format!("{} {} 0\t{}", item.candidate_mode, item.candidate_blob_oid, item.path);
        if item.transformation != "crlf_to_lf_only" || item.candidate_tree != tree
            || item.after_raw_blob_oid != item.candidate_blob_oid
            || item.after_path_clean_oid != item.candidate_blob_oid
            || fingerprint.raw_hash != item.after_raw_blake3
            || fingerprint.bytes != item.after_bytes
            || !expected_records.split(|byte| *byte == 0).any(|entry| entry == record.as_bytes())
        {
            return Err(format!("EOL stat refresh proof does not match the candidate: {}", item.path));
        }
    }
    let transaction = super::io::IndexTransaction::begin(repo)?;
    let records = worktrees::git(repo, &["ls-files", "--stage", "-z"])?;
    let flags = worktrees::git(repo, &["ls-files", "-v", "-z"])?;
    let head = worktrees::git_text(repo, &["rev-parse", "HEAD"])?;
    if records != expected_records || worktrees::git_text(repo, &["rev-parse", "HEAD^{tree}"])? != tree
        || source_snapshot(repo, project, None)? != *snapshot
    {
        return Err("Candidate changed before the EOL stat refresh".into());
    }
    // Refuse ordinary dirty/untracked state; this is not pool cleanup. Only
    // stat-dirty paths covered by the final immutable-tree proof are eligible.
    let status = worktrees::git(repo, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    if status.is_empty() {
        return Ok(Vec::new());
    }
    for entry in status.split(|byte| *byte == 0).filter(|entry| !entry.is_empty())
    {
        let path = entry.get(3..).and_then(|bytes| std::str::from_utf8(bytes).ok());
        if entry.get(..3) != Some(b" M ") || !path.is_some_and(|path| paths.contains(path)) {
            return Err("Candidate has changes outside the proved EOL stat paths".into());
        }
    }
    worktrees::git(repo, &["diff", "--quiet", "--no-ext-diff", tree, "--"])
        .map_err(|error| format!("EOL stat refresh cannot stage source changes: {error}"))?;
    let temporary = tempfile::Builder::new().prefix("eol-stat-").tempdir_in(proof_dir)
        .map_err(|e| e.to_string())?;
    let shadow = temporary.path().join("index");
    std::fs::write(&shadow, &transaction.original).map_err(|e| e.to_string())?;
    let input = paths.iter().map(|path| format!("{path}\0")).collect::<String>();
    // --refresh alone cannot refresh a CRLF-sized stat entry after an LF save
    // on Git for Windows. Exact literal add on a shadow index updates the stat;
    // unchanged whole-index records, flags and tree are required before install.
    super::io::git_input(repo,
        &["--literal-pathspecs", "add", "--pathspec-from-file=-", "--pathspec-file-nul"],
        input.as_bytes(), Some(&shadow))?;
    let shadow_tree = super::io::git_input(repo, &["write-tree"], b"", Some(&shadow))?;
    let shadow_records = super::io::git_input(repo, &["ls-files", "--stage", "-z"], b"", Some(&shadow))?;
    let shadow_flags = super::io::git_input(repo, &["ls-files", "-v", "-z"], b"", Some(&shadow))?;
    if shadow_tree != tree || shadow_records.as_bytes() != records || shadow_flags.as_bytes() != flags
        || worktrees::git_text(repo, &["rev-parse", "HEAD"])? != head
        || source_snapshot(repo, project, None)? != *snapshot
    {
        return Err("EOL stat refresh changed candidate records, flags, HEAD or source bytes".into());
    }
    transaction.install(&std::fs::read(&shadow).map_err(|e| e.to_string())?)?;
    if worktrees::git(repo, &["ls-files", "--stage", "-z"])? != records
        || worktrees::git(repo, &["ls-files", "-v", "-z"])? != flags
        || worktrees::git_text(repo, &["rev-parse", "HEAD"])? != head
        || worktrees::git_text(repo, &["rev-parse", "HEAD^{tree}"])? != tree
        || source_snapshot(repo, project, None)? != *snapshot
    {
        return Err("Candidate changed while installing the proved EOL stat refresh".into());
    }
    Ok(paths.into_iter().collect())
}

fn journal(path: &Path, value: &Value) -> Result<(), String> {
    super::io::atomic_json(path, value)
}

fn harness_source(tree: &str, version: &str, nonce: &str, paths: &[String]) -> String {
    let body = super::validation_scope::script(paths, "message => UnityEngine.Debug.Log(message)");
    // This assembly references Unity APIs only. It does not need the Locus
    // bridge or an entry in Packages/manifest.json / packages-lock.json.
    r#"namespace Locus.IsolatedMergeValidation.N__NONCE__ {
internal static class Harness {
    private static bool started;
    private static double readySince = -1;
    private static readonly System.Collections.Generic.List<string> importErrors = new System.Collections.Generic.List<string>();
    private static string ResultPath { get { return System.IO.Path.GetFullPath(System.IO.Path.Combine(UnityEngine.Application.dataPath, "../Library/Locus/merge-validation-__NONCE__.json")); } }
    [System.Serializable] private sealed class Result {
        public string tree = __TREE__;
        public string nonce = "__NONCE__";
        public string editor_version;
        public string marker;
        public string error;
        public bool ok;
        public int asset_roots_count = __COUNT__;
    }
    [UnityEditor.InitializeOnLoadMethod] private static void Initialize() {
        if (!UnityEngine.Application.isBatchMode || System.IO.File.Exists(ResultPath)) return;
        UnityEngine.Application.logMessageReceived += OnLog;
        UnityEditor.EditorApplication.delayCall += Arm;
    }
    private static void OnLog(string message, string trace, UnityEngine.LogType type) {
        if (type == UnityEngine.LogType.Error || type == UnityEngine.LogType.Exception || type == UnityEngine.LogType.Assert)
            importErrors.Add(message + "\n" + trace);
    }
    private static void Arm() { UnityEditor.EditorApplication.update += Tick; }
    private static void Tick() {
        if (started) return;
        if (UnityEditor.EditorApplication.isCompiling || UnityEditor.EditorApplication.isUpdating) { readySince = -1; return; }
        if (readySince < 0) { readySince = UnityEditor.EditorApplication.timeSinceStartup; return; }
        if (UnityEditor.EditorApplication.timeSinceStartup - readySince < 2) return;
        started = true;
        UnityEditor.EditorApplication.update -= Tick;
        var result = new Result();
        result.editor_version = UnityEngine.Application.unityVersion;
        try {
            if (result.editor_version != __VERSION__) throw new System.Exception("Candidate Editor version mismatch: " + result.editor_version);
            // Global compilation has completed before Tick can run. Import
            // diagnostics below belong to the selected validation window;
            // unrelated startup asset warnings/errors do not expand its scope.
            importErrors.Clear();
            result.marker = ValidateBody();
            if (importErrors.Count != 0) throw new System.Exception("Unity reported import/serialization errors:\n" + string.Join("\n", importErrors));
            result.ok = result.marker == "LOCUS_MERGE_VALIDATED";
        } catch (System.Exception error) { result.ok = false; result.error = error.ToString(); }
        UnityEngine.Application.logMessageReceived -= OnLog;
        try {
            System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(ResultPath));
            System.IO.File.WriteAllText(ResultPath + ".tmp", UnityEngine.JsonUtility.ToJson(result, true));
            System.IO.File.Move(ResultPath + ".tmp", ResultPath);
        } finally { UnityEditor.EditorApplication.Exit(result.ok ? 0 : 1); }
    }
    private static string ValidateBody() {
__BODY__
    }
}}
"#
    .replace("__NONCE__", nonce)
    .replace("__TREE__", &json!(tree).to_string())
    .replace("__VERSION__", &json!(version).to_string())
    .replace("__COUNT__", &paths.len().to_string())
    .replace("__BODY__", &body)
}

fn editor_log_tail(project: &Path) -> String {
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(project.join("Logs/Editor.log")) else {
        return String::new();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file
        .seek(SeekFrom::Start(length.saturating_sub(256 * 1024)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into()
}

async fn wait_for_harness(
    project: &Path,
    launch: &UnityLaunchResult,
    created: Option<u64>,
    tree: &str,
    version: &str,
    nonce: &str,
) -> Result<Value, String> {
    let path = project.join(format!("Library/Locus/merge-validation-{nonce}.json"));
    let started = std::time::Instant::now();
    loop {
        if path.is_file() {
            let result: Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if result["tree"] != tree
                || result["nonce"] != nonce
                || result["editor_version"] != version
            {
                return Err(format!(
                    "Validation harness returned a stale or mismatched identity: {result}"
                ));
            }
            if result["ok"] != true || result["marker"] != "LOCUS_MERGE_VALIDATED" {
                return Err(format!(
                    "Unity rejected candidate tree: {}",
                    result["error"]
                ));
            }
            return Ok(result);
        }
        let liveness = unity_bridge::launched_unity_process_liveness(launch.process_id, created)?;
        if liveness != unity_bridge::UnityProcessIdentityLiveness::Alive {
            return Err(format!("Validation Editor exited without its result (compilation/import failed); Editor log:\n{}", editor_log_tail(project)));
        }
        if started.elapsed() >= Duration::from_secs(1200) {
            return Err(format!(
                "Validation Editor did not publish a result in 1200 seconds; Editor log:\n{}",
                editor_log_tail(project)
            ));
        }
        // The new assembly cannot initialize when the candidate fails C#
        // compilation. Report the compiler diagnostics instead of waiting for
        // a bridge that intentionally was never installed in this checkout.
        if started.elapsed() >= Duration::from_secs(5) {
            let errors: Vec<_> = editor_log_tail(project)
                .lines()
                .filter(|line| {
                    line.contains("error CS")
                        || line.contains("Scripts have compiler errors")
                        || line.contains("Aborting batchmode due to failure")
                })
                .map(str::to_string)
                .collect();
            if !errors.is_empty() {
                return Err(format!(
                    "Candidate compilation/import failed: {}",
                    errors.join("\n")
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn close_owned(
    project: &str,
    launch: &UnityLaunchResult,
    created: Option<u64>,
) -> Result<(), String> {
    let current = unity_bridge::query_current_project_editor_process_uncached(project.into());
    if current.state != UnityEditorProcessState::NotRunning {
        let liveness = unity_bridge::launched_unity_process_liveness(launch.process_id, created)?;
        if current.state != UnityEditorProcessState::Running
            || current.process_id != Some(launch.process_id)
            || liveness != unity_bridge::UnityProcessIdentityLiveness::Alive
        {
            return Err(
                "Owned validation Editor identity changed; its process was retained for inspection"
                    .into(),
            );
        }
        if let Err(error) =
            unity_bridge::close_current_project_unity_processes(project, Duration::from_secs(45))
                .await
        {
            let current =
                unity_bridge::query_current_project_editor_process_uncached(project.into());
            if current.state != UnityEditorProcessState::NotRunning {
                if current.state != UnityEditorProcessState::Running
                    || current.process_id != Some(launch.process_id)
                    || unity_bridge::launched_unity_process_liveness(launch.process_id, created)?
                        != unity_bridge::UnityProcessIdentityLiveness::Alive
                {
                    return Err(error);
                }
                // A compiler failure can prevent the Editor-only harness from
                // loading at all. Only the still-verified owned process may be
                // terminated when neither the harness nor graceful close works.
                unity_bridge::force_close_current_project_unity_processes(
                    project,
                    Duration::from_secs(20),
                )
                .await?;
            }
        }
    }
    if unity_bridge::query_current_project_editor_process_uncached(project.into()).state
        != UnityEditorProcessState::NotRunning
    {
        return Err(
            "Owned validation Editor did not exit; its pool assignment was retained".into(),
        );
    }
    for relative in ["Temp/UnityLockfile", "Library/EditorInstance.json"] {
        let path = Path::new(project).join(relative);
        if path.is_file() {
            let owned = owned_validation_path(Path::new(project), relative)?;
            std::fs::remove_file(&owned).map_err(|e| format!("{}: {e}", owned.display()))?;
        }
    }
    Ok(())
}

/// Retaining a spawned task means cancellation of an SDK request does not skip
/// the owned-Editor cleanup. Process crashes retain the durable pool/journal.
pub(crate) async fn validate(
    app: &tauri::AppHandle,
    source_project: &Path,
    tree: &str,
    parent: &str,
    paths: &[String],
) -> Result<Value, String> {
    let app = app.clone();
    let source = source_project.to_path_buf();
    let tree = tree.to_string();
    let parent = parent.to_string();
    let paths = paths.to_vec();
    tauri::async_runtime::spawn(async move {
        validate_inner(&app, &source, &tree, &parent, &paths).await
    })
    .await
    .map_err(|e| format!("Scratch validation task failed: {e}"))?
}

async fn validate_inner(
    app: &tauri::AppHandle,
    source: &Path,
    tree: &str,
    parent: &str,
    paths: &[String],
) -> Result<Value, String> {
    let source = worktrees::canonical(source)?;
    let repo = worktrees::canonical(Path::new(&worktrees::git_text(
        &source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let commit = immutable_commit(&repo, tree, parent)?;
    let registry = app.state::<Arc<ProjectRegistry>>().inner().clone();
    let source_identity = crate::workspace_service::identity::ProjectIdResolver::resolve(&source)
        .map_err(|e| e.to_string())?;
    let pool_root = repo
        .parent()
        .ok_or("Repository has no parent for the validation pool")?
        .join(".locus-merge-validation")
        .join(source_identity.project_id.as_str());
    std::fs::create_dir_all(&pool_root).map_err(|e| e.to_string())?;
    let assignment = format!("merge-validation-{}", uuid::Uuid::new_v4().simple());
    let common = crate::workspace_service::identity::resolve_git_common_dir(&source)
        .ok_or("Source Git common directory missing")?;
    let journal_path = common
        .join("locus-worktrees/merge-validation")
        .join(format!("{assignment}.json"));
    std::fs::create_dir_all(
        journal_path
            .parent()
            .ok_or("Validation journal has no parent")?,
    )
    .map_err(|e| e.to_string())?;
    let request = pool::AcquirePoolRequest {
        source_root: source.to_string_lossy().into(),
        pool_root: pool_root.to_string_lossy().into(),
        commit: commit.clone(),
        branch: None,
        max_slots: registry
            .resource_policy()
            .snapshot()
            .limits
            .max_unity_editors,
        assignment_id: Some(assignment.clone()),
    };
    let acquisition = tauri::async_runtime::spawn_blocking(move || pool::acquire(&request))
        .await
        .map_err(|e| e.to_string())??;
    let project = acquisition.worktree.root.clone();
    let scratch = PathBuf::from(&acquisition.worktree.repo_root);
    let scratch_project = PathBuf::from(&project);
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let harness_relative = format!("Assets/LocusMergeValidation-{nonce}");
    let harness_root = scratch_project.join(&harness_relative);
    let mut record = json!({"version":1,"assignment_id":assignment,"state":"acquired","candidate_tree":tree,"candidate_commit":commit,
        "source_project":source,"scratch_root":project,"acquisition":acquisition,"editor":null});
    let mut launch = None;
    let mut created = None;
    let mut execution = None;
    let mut overlay_created = false;
    let mut expected_snapshot = None;
    let mut expected_index = None;
    let mut outcome: Result<Value, String> = async {
        journal(&journal_path, &record)?;
        if unity_bridge::query_current_project_editor_process_uncached(project.clone()).state != UnityEditorProcessState::NotRunning {
            return Err("Validation pool acquired a project with an existing or unverified Editor".into());
        }
        let version = worktrees::editor_version(&scratch_project).ok_or("Candidate has no Unity version")?;
        if !version.starts_with("6000.5.") { return Err(format!("Validation requires Unity 6.5; candidate specifies {version}")); }
        if worktrees::git_text(&scratch, &["rev-parse", "HEAD^{tree}"])? != tree {
            return Err("Validation pool did not materialize the candidate tree".into());
        }
        let index_before = worktrees::git(&scratch, &["ls-files", "--stage", "-z"])?;
        let before = source_snapshot(&scratch, &scratch_project, None)?;
        record["source_snapshot_hashes"] = json!({"before":eol_proof::snapshot_hash(&before)?});
        let tracked: BTreeSet<String> = worktrees::git(&scratch, &["ls-files", "-z"])?
            .split(|b| *b == 0).filter(|p| !p.is_empty())
            .map(|p| String::from_utf8(p.to_vec()).map_err(|e| e.to_string())).collect::<Result<_,_>>()?;
        let unexpected: Vec<_> = before.keys().filter(|p| !tracked.contains(*p)).cloned().collect();
        if !unexpected.is_empty() { return Err(format!("Validation pool retained source absent from the candidate tree: {}", unexpected.join(", "))); }
        if !worktrees::git(&scratch, &["diff", "--no-ext-diff", "--name-only", "-z", &commit, "--"] )?.is_empty() {
            return Err("Validation pool checkout differs from its candidate before Editor startup".into());
        }
        expected_snapshot = Some(before.clone());
        expected_index = Some(index_before.clone());
        // Ordinary asset changes inspect only their explicit scope and its
        // Unity dependency closure; scripts/import settings expand this set.
        let (asset_paths, validation_scope) = super::validation_scope::paths(&scratch, &scratch_project, paths)?;
        if harness_root.exists() || harness_root.with_extension("meta").exists() { return Err("Unique validation harness path already exists".into()); }
        std::fs::create_dir(&harness_root).map_err(|e| e.to_string())?;
        overlay_created = true;
        std::fs::create_dir(harness_root.join("Editor")).map_err(|e| e.to_string())?;
        std::fs::write(harness_root.join("Editor/Harness.cs"), harness_source(tree, &version, &nonce, &asset_paths)).map_err(|e| e.to_string())?;
        std::fs::write(harness_root.join("Editor/Harness.asmdef"), serde_json::to_vec_pretty(&json!({"name":format!("Locus.IsolatedMergeValidation.{nonce}"),"includePlatforms":["Editor"],"autoReferenced":false})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let after_overlay = source_snapshot(&scratch, &scratch_project, Some(&harness_root))?;
        let changed = changed_paths(&before, &after_overlay);
        if !changed.is_empty() { return Err(format!("Validation tool installation changed candidate source: {}", changed.join(", "))); }
        record["tool_overlay"] = json!([harness_relative, format!("{harness_relative}.meta")]);
        record["harness_nonce"] = json!(nonce);
        record["state"] = json!("prepared");
        journal(&journal_path, &record)?;
        let runtime = registry.register(&scratch_project)?;
        if runtime.project_id() != &source_identity.project_id {
            return Err("Validation checkout resolved to a different logical project".into());
        }
        // Keep the checkout lease without starting a Unity bridge service: this
        // validation works even when the candidate has no Locus plugin.
        execution = Some(registry.execution_context(runtime.checkout_id(), &[]).await?);
        let started = unity_bridge::launch_project_with_mode(&project, UnityLaunchMode::Headless).await?;
        created = unity_bridge::launched_unity_process_created_at_ms(started.process_id);
        record["editor"] = json!(started);
        record["editor_created_at_ms"] = json!(created);
        record["state"] = json!("validating");
        launch = Some(started);
        journal(&journal_path, &record)?;
        if launch.as_ref().unwrap().project_version != version { return Err("Validation Editor version differs from the candidate".into()); }
        let output = wait_for_harness(&scratch_project, launch.as_ref().unwrap(), created, tree, &version, &nonce).await?;
        let after = source_snapshot(&scratch, &scratch_project, Some(&harness_root))?;
        record["source_snapshot_hashes"]["after_validation"] = json!(eol_proof::snapshot_hash(&after)?);
        record["source_changed_paths"] = json!(changed_paths(&before, &after));
        let normalizations = eol_proof::prove(&scratch, tree, &before, &after, journal_path.parent().ok_or("Missing validation journal directory")?)
            .map_err(|error|format!("Unity importer/validation changed candidate source: {error}"))?;
        record["source_eol_normalizations"] = json!(normalizations);
        if worktrees::git(&scratch, &["ls-files", "--stage", "-z"])? != index_before
            || worktrees::git_text(&scratch, &["rev-parse", "HEAD^{tree}"])? != tree
        {
            return Err("Unity validation changed its candidate index or HEAD".into());
        }
        Ok(json!({"validated_tree":tree,"candidate_commit":commit,"placement":"isolated_pool","scratch_root":project,
            "editor_version":version,"editor":launch,"compilation":"candidate compiled and loaded with a unique Editor validation assembly","output":output,
            "requested_paths":paths,"asset_roots":asset_paths,"validation_scope":validation_scope,"tool_overlay":record["tool_overlay"],"journal":journal_path}))
    }.await;
    let mut cleanup_errors = Vec::new();
    if let Some(started) = launch.as_ref() {
        if let Err(error) = close_owned(&project, started, created).await {
            cleanup_errors.push(error);
        }
    }
    drop(execution.take());
    let checkout = CheckoutId::new(&acquisition.worktree.checkout_id).map_err(|e| e.to_string())?;
    if let Err(error) = registry.retire_managed_checkout(&checkout).await {
        cleanup_errors.push(error);
    }
    if outcome.is_ok() {
        // Shutdown callbacks may save assets or ProjectSettings. They belong
        // to the verification window too, before releasing the physical slot.
        let final_check: Result<(), String> = (|| {
            let after = source_snapshot(
                &scratch,
                &scratch_project,
                overlay_created.then_some(harness_root.as_path()),
            )?;
            let before = expected_snapshot
                .as_ref()
                .ok_or("Missing candidate snapshot")?;
            record["source_snapshot_hashes"]["after_shutdown"] =
                json!(eol_proof::snapshot_hash(&after)?);
            record["source_changed_paths"] = json!(changed_paths(before, &after));
            record["source_eol_normalizations"] = json!(eol_proof::prove(
                &scratch,
                tree,
                before,
                &after,
                journal_path
                    .parent()
                    .ok_or("Missing validation journal directory")?,
            )
            .map_err(|error| format!("Unity shutdown changed candidate source: {error}"))?);
            if Some(worktrees::git(&scratch, &["ls-files", "--stage", "-z"])?).as_ref()
                != expected_index.as_ref()
                || worktrees::git_text(&scratch, &["rev-parse", "HEAD^{tree}"])? != tree
            {
                return Err("Unity shutdown changed the candidate index or HEAD".into());
            }
            Ok(())
        })();
        if let Err(error) = final_check {
            outcome = Err(error);
        }
    }
    if cleanup_errors.is_empty() && overlay_created {
        // This exact directory was absent before our own tool installation.
        // Preserve it when Unity is still running or its ownership is uncertain.
        if harness_root.exists() {
            match owned_validation_path(&scratch_project, &harness_relative) {
                Ok(path) => {
                    if let Err(error) = std::fs::remove_dir_all(&path) {
                        cleanup_errors.push(error.to_string());
                    }
                }
                Err(error) => cleanup_errors.push(format!(
                    "Validation overlay retained for inspection: {error}"
                )),
            }
        }
        let meta = harness_root.with_extension("meta");
        if cleanup_errors.is_empty() && meta.exists() {
            match owned_validation_path(&scratch_project, &format!("{harness_relative}.meta")) {
                Ok(path) => {
                    if let Err(error) = std::fs::remove_file(path) {
                        cleanup_errors.push(error.to_string());
                    }
                }
                Err(error) => cleanup_errors.push(format!(
                    "Validation harness metadata retained for inspection: {error}"
                )),
            }
        }
    }
    if cleanup_errors.is_empty() {
        if outcome.is_ok() {
            match source_snapshot(&scratch, &scratch_project, None) {
                Ok(after) => {
                    let checked = (|| -> Result<(), String> {
                        record["source_snapshot_hashes"]["after_cleanup"] =
                            json!(eol_proof::snapshot_hash(&after)?);
                        let normalizations = eol_proof::prove(
                            &scratch,
                            tree,
                            expected_snapshot
                                .as_ref()
                                .ok_or("Missing candidate snapshot")?,
                            &after,
                            journal_path
                                .parent()
                                .ok_or("Missing validation journal directory")?,
                        )?;
                        let refreshed = refresh_proved_eol_index(
                            &scratch, &scratch_project, tree, &after, &normalizations,
                            expected_index.as_ref().ok_or("Missing candidate index records")?,
                            journal_path.parent().ok_or("Missing validation journal directory")?,
                        )?;
                        record["source_eol_normalizations"] = json!(normalizations);
                        record["source_eol_stat_refreshed_paths"] = json!(refreshed);
                        Ok(())
                    })();
                    if let Err(error) = checked {
                        outcome = Err(format!(
                            "Validation cleanup left candidate source changes: {error}"
                        ));
                    }
                }
                Err(error) => outcome = Err(error),
            }
        }
        if let Err(error) = pool::release(
            &source,
            &acquisition.worktree.checkout_id,
            &assignment,
            acquisition.worktree.materialization_epoch,
        ) {
            cleanup_errors.push(error);
        } else {
            record["pool_released"] = json!(true);
        }
    }
    record["state"] = json!(if outcome.is_ok() {
        "validated"
    } else {
        "failed"
    });
    record["validation_error"] = json!(outcome.as_ref().err());
    record["cleanup_errors"] = json!(cleanup_errors);
    if let Err(error) = journal(&journal_path, &record) {
        cleanup_errors.push(format!("Could not persist validation outcome: {error}"));
    }
    match outcome {
        Ok(mut value) => {
            value["source_snapshot_hashes"] = record["source_snapshot_hashes"].clone();
            value["source_eol_normalizations"] = record["source_eol_normalizations"].clone();
            value["source_eol_stat_refreshed_paths"] = record["source_eol_stat_refreshed_paths"].clone();
            value["cleanup_errors"] = json!(cleanup_errors);
            value["pool_released"] = record["pool_released"].clone();
            Ok(value)
        }
        Err(error) => Err(format!(
            "{error}; validation checkout retained at {project}; journal={}; cleanup={}",
            journal_path.display(),
            cleanup_errors.join("; ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eol_stat_fixture() -> (tempfile::TempDir, PathBuf, String, Snapshot, Vec<eol_proof::EolNormalization>) {
        let (temp, repo) = worktrees::tests::fixture();
        worktrees::git(&repo, &["config", "core.autocrlf", "true"]).unwrap();
        let chosen = "Assets/chosen[1].asset";
        std::fs::write(repo.join(chosen), b"value: 1\nnext: 2\n").unwrap();
        worktrees::git(&repo, &["--literal-pathspecs", "add", "--", chosen]).unwrap();
        worktrees::git(&repo, &["commit", "-m", "EOL stat fixture"]).unwrap();
        std::fs::remove_file(repo.join(chosen)).unwrap();
        worktrees::git(&repo, &["--literal-pathspecs", "checkout", "--", chosen]).unwrap();
        assert_eq!(std::fs::read(repo.join(chosen)).unwrap(), b"value: 1\r\nnext: 2\r\n");
        worktrees::git(&repo, &["update-index", "--assume-unchanged", "Assets/test.asset"]).unwrap();
        worktrees::git(&repo, &["update-index", "--skip-worktree", ".gitignore"]).unwrap();
        let before = source_snapshot(&repo, &repo, None).unwrap();
        std::fs::write(repo.join(chosen), b"value: 1\nnext: 2\n").unwrap();
        let after = source_snapshot(&repo, &repo, None).unwrap();
        let tree = worktrees::git_text(&repo, &["rev-parse", "HEAD^{tree}"]).unwrap();
        let proof = eol_proof::prove(&repo, &tree, &before, &after, &temp.path().join("proof")).unwrap();
        assert_eq!(proof.len(), 1);
        (temp, repo, tree, after, proof)
    }

    #[test]
    fn proved_eol_stat_refresh_preserves_records_flags_tree_and_all_source_bytes() {
        let (temp, repo, tree, snapshot, proof) = eol_stat_fixture();
        let records = worktrees::git(&repo, &["ls-files", "--stage", "-z"]).unwrap();
        let flags = worktrees::git(&repo, &["ls-files", "-v", "-z"]).unwrap();
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        let head = worktrees::git_text(&repo, &["rev-parse", "HEAD"]).unwrap();
        assert!(!worktrees::git(&repo, &["status", "--porcelain=v1", "-z"]).unwrap().is_empty());
        let refreshed = refresh_proved_eol_index(&repo, &repo, &tree, &snapshot, &proof, &records, &temp.path().join("proof")).unwrap();
        assert_eq!(refreshed, vec!["Assets/chosen[1].asset"]);
        assert!(worktrees::git(&repo, &["status", "--porcelain=v1", "-z"]).unwrap().is_empty());
        assert_ne!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        assert_eq!(worktrees::git(&repo, &["ls-files", "--stage", "-z"]).unwrap(), records);
        assert_eq!(worktrees::git(&repo, &["ls-files", "-v", "-z"]).unwrap(), flags);
        assert_eq!(worktrees::git_text(&repo, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(worktrees::git_text(&repo, &["rev-parse", "HEAD^{tree}"]).unwrap(), tree);
        assert_eq!(source_snapshot(&repo, &repo, None).unwrap(), snapshot);
        let refreshed_index = std::fs::read(repo.join(".git/index")).unwrap();
        assert!(refresh_proved_eol_index(&repo, &repo, &tree, &snapshot, &proof, &records, &temp.path().join("proof")).unwrap().is_empty());
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), refreshed_index);
    }

    #[test]
    fn proved_eol_stat_refresh_rejects_later_source_changes_without_writing_index() {
        let (temp, repo, tree, snapshot, proof) = eol_stat_fixture();
        let records = worktrees::git(&repo, &["ls-files", "--stage", "-z"]).unwrap();
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        std::fs::write(repo.join("Assets/test.asset"), b"later source edit\n").unwrap();
        assert!(refresh_proved_eol_index(&repo, &repo, &tree, &snapshot, &proof, &records, &temp.path().join("proof")).unwrap_err().contains("changed before"));
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(repo.join("Assets/test.asset")).unwrap(), b"later source edit\n");
        assert!(!repo.join(".git/index.lock").exists());
    }

    #[test]
    fn proved_eol_stat_refresh_rejects_unproven_dirty_files_and_invalid_proof() {
        let (temp, repo, tree, _snapshot, mut proof) = eol_stat_fixture();
        let records = worktrees::git(&repo, &["ls-files", "--stage", "-z"]).unwrap();
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        std::fs::write(repo.join("unrelated.txt"), b"independent untracked file\n").unwrap();
        let snapshot = source_snapshot(&repo, &repo, None).unwrap();
        assert!(refresh_proved_eol_index(&repo, &repo, &tree, &snapshot, &proof, &records, &temp.path().join("proof")).unwrap_err().contains("outside the proved"));
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        std::fs::remove_file(repo.join("unrelated.txt")).unwrap();
        proof[0].candidate_tree = "0".repeat(40);
        assert!(refresh_proved_eol_index(&repo, &repo, &tree, &snapshot, &proof, &records, &temp.path().join("proof")).unwrap_err().contains("proof does not match"));
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
        assert!(!repo.join(".git/index.lock").exists());
    }

    #[test]
    #[cfg(windows)]
    fn snapshot_accepts_long_files_inside_short_and_verbatim_checkout_roots() {
        let (_temp, repo) = worktrees::tests::fixture();
        let relative = format!(
            "Assets/{}/{}/{}/long.asset.meta",
            "a".repeat(90),
            "b".repeat(90),
            "c".repeat(90)
        );
        let target = repo.join(&relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"guid: long-owned-asset\n").unwrap();
        assert!(target.to_string_lossy().len() > 260);
        let snapshot = source_snapshot(&repo, &repo, None).unwrap();
        assert_eq!(
            snapshot[&relative],
            Some(eol_proof::fingerprint(&target).unwrap())
        );
        let canonical_repo = std::fs::canonicalize(&repo).unwrap();
        assert_eq!(
            snapshot,
            source_snapshot(&canonical_repo, &canonical_repo, None).unwrap()
        );
        assert_eq!(
            snapshot,
            source_snapshot(&repo, &canonical_repo, None).unwrap()
        );
        assert_eq!(
            snapshot,
            source_snapshot(&canonical_repo, &repo, None).unwrap()
        );
    }

    #[test]
    #[cfg(windows)]
    fn owned_cleanup_paths_accept_long_markers_and_harness_paths() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp
            .path()
            .join("a".repeat(90))
            .join("b".repeat(90))
            .join("c".repeat(90));
        for relative in [
            "Temp/UnityLockfile",
            "Library/EditorInstance.json",
            "Assets/LocusMergeValidation-test.meta",
        ] {
            let path = project.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"owned fixture").unwrap();
            assert_eq!(
                owned_validation_path(&project, relative).unwrap(),
                std::fs::canonicalize(&path).unwrap()
            );
        }
        let harness = project.join("Assets/LocusMergeValidation-test");
        std::fs::create_dir(&harness).unwrap();
        assert_eq!(
            owned_validation_path(&project, "Assets/LocusMergeValidation-test").unwrap(),
            std::fs::canonicalize(harness).unwrap()
        );
        assert!(owned_validation_path(&project, "../outside").is_err());
    }

    #[test]
    fn owned_cleanup_paths_reject_internal_and_external_parent_links() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let internal = project.join("real");
        let external = temp.path().join("outside");
        for target in [&internal, &external] {
            std::fs::create_dir_all(target).unwrap();
            std::fs::write(target.join("UnityLockfile"), b"preserve target").unwrap();
        }
        let link = project.join("Temp");
        for target in [&internal, &external] {
            #[cfg(windows)]
            let linked = std::os::windows::fs::symlink_dir(target, &link);
            #[cfg(unix)]
            let linked = std::os::unix::fs::symlink(target, &link);
            if let Err(error) = linked {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    return;
                }
                panic!("Could not create cleanup link fixture: {error}");
            }
            assert!(owned_validation_path(&project, "Temp/UnityLockfile")
                .unwrap_err()
                .contains("changed or escapes"));
            #[cfg(windows)]
            std::fs::remove_dir(&link).unwrap();
            #[cfg(unix)]
            std::fs::remove_file(&link).unwrap();
            assert_eq!(
                std::fs::read(target.join("UnityLockfile")).unwrap(),
                b"preserve target"
            );
        }
    }

    #[test]
    fn snapshot_rejects_a_sibling_project_even_with_a_matching_name_prefix() {
        let (temp, repo) = worktrees::tests::fixture();
        let sibling = temp.path().join("source-other");
        std::fs::create_dir_all(sibling.join("Assets")).unwrap();
        std::fs::write(sibling.join("Assets/outside.asset"), b"outside source").unwrap();
        assert!(source_snapshot(&repo, &sibling, None).is_err());
    }

    #[test]
    fn snapshot_rejects_symbolic_links_to_both_internal_and_external_files() {
        let (temp, repo) = worktrees::tests::fixture();
        let outside = temp.path().join("outside.asset");
        std::fs::write(&outside, b"preserve external source").unwrap();
        let link = repo.join("Assets/link.asset");
        for target in [repo.join("Assets/test.asset"), outside.clone()] {
            #[cfg(windows)]
            let linked = std::os::windows::fs::symlink_file(&target, &link);
            #[cfg(unix)]
            let linked = std::os::unix::fs::symlink(&target, &link);
            if let Err(error) = linked {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    return;
                }
                panic!("Could not create source link fixture: {error}");
            }
            let error = source_snapshot(&repo, &repo, None).unwrap_err();
            assert!(
                error.contains("symbolic link") || error.contains("escapes its checkout"),
                "{error}"
            );
            std::fs::remove_file(&link).unwrap();
        }
        assert_eq!(std::fs::read(outside).unwrap(), b"preserve external source");
    }

    #[test]
    fn snapshot_detects_ignored_assets_and_missing_tracked_files() {
        let (_temp, repo) = worktrees::tests::fixture();
        let before = source_snapshot(&repo, &repo, None).unwrap();
        std::fs::write(repo.join(".git/info/exclude"), b"Assets/new.asset\n").unwrap();
        std::fs::write(repo.join("Assets/new.asset"), b"new importer output").unwrap();
        std::fs::remove_file(repo.join("Assets/test.asset")).unwrap();
        assert_eq!(
            changed_paths(&before, &source_snapshot(&repo, &repo, None).unwrap()),
            vec!["Assets/new.asset", "Assets/test.asset"]
        );
    }

    #[test]
    fn harness_overlay_does_not_whitelist_other_untracked_sources() {
        let (_temp, repo) = worktrees::tests::fixture();
        let before = source_snapshot(&repo, &repo, None).unwrap();
        let harness = repo.join("Assets/LocusMergeValidation-test");
        std::fs::create_dir_all(&harness).unwrap();
        std::fs::write(harness.join("Harness.cs"), b"// harness").unwrap();
        std::fs::write(harness.with_extension("meta"), b"guid: harness").unwrap();
        assert!(changed_paths(
            &before,
            &source_snapshot(&repo, &repo, Some(&harness)).unwrap()
        )
        .is_empty());
        std::fs::write(repo.join("Assets/unselected.cs"), b"class Unselected {}").unwrap();
        assert_eq!(
            changed_paths(
                &before,
                &source_snapshot(&repo, &repo, Some(&harness)).unwrap()
            ),
            vec!["Assets/unselected.cs"]
        );
    }

    #[test]
    fn harness_has_no_locus_plugin_dependency_and_binds_result_identity() {
        let script = harness_source(
            "abc123",
            "6000.5.8f1",
            "deadbeef",
            &["Assets/Graph.asset".into()],
        );
        assert!(script.contains("[UnityEditor.InitializeOnLoadMethod]"));
        assert!(script.contains("public string tree = \"abc123\""));
        assert!(script.contains("public string nonce = \"deadbeef\""));
        assert!(script.contains("UnityEditor.EditorApplication.Exit(result.ok ? 0 : 1)"));
        assert!(script.contains("HasManagedReferencesWithMissingTypes"));
        assert!(!script.contains("__BODY__"));
        assert!(!script.contains("__LOCUS_PATHS__"));
        assert!(!script.contains("__LOCUS_PROGRESS__"));
        assert!(!script.contains("print("));
        assert!(!script.contains("Locus.Editor"));
    }
}
