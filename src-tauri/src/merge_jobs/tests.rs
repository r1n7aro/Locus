use super::*;
use tempfile::TempDir;

#[path = "performance_tests.rs"]
mod performance;

struct Repo {
    _dir: TempDir,
    root: std::path::PathBuf,
    source: std::path::PathBuf,
}
impl Repo {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("target");
        let source = dir.path().join("source");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "main"]).unwrap();
        git(
            &root,
            &["config", "user.email", "merge-test@example.invalid"],
        )
        .unwrap();
        git(&root, &["config", "user.name", "Merge Test"]).unwrap();
        git(&root, &["config", "core.autocrlf", "false"]).unwrap();
        std::fs::write(root.join("asset.asset"), yaml(100, 5, "base")).unwrap();
        std::fs::write(root.join("notes.txt"), "base notes\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(&root, &["commit", "-m", "base"]).unwrap();
        git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "source",
                "--",
                source.to_str().unwrap(),
            ],
        )
        .unwrap();
        Self {
            _dir: dir,
            root,
            source,
        }
    }
    fn commit(&self, path: &str, content: impl AsRef<[u8]>) -> String {
        if let Some(parent) = self.source.join(path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(self.source.join(path), content).unwrap();
        git(&self.source, &["add", "--", path]).unwrap();
        git(&self.source, &["commit", "-m", "source change"]).unwrap();
        git_text(&self.source, &["rev-parse", "HEAD"]).unwrap()
    }
    fn job(&self, commits: Vec<String>) -> MergeJob {
        prepare(
            &self.root,
            &PrepareRequest {
                sources: vec![SourceSelection {
                    commits,
                    ..Default::default()
                }],
                eager_catalog: true,
                ..Default::default()
            },
        )
        .unwrap()
    }
}
fn yaml(health: u32, energy: u32, name: &str) -> String {
    format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_Name: {name}\n  health: {health}\n  energy: {energy}\n")
}

#[test]
fn selected_noncontiguous_commits_preserve_dirty_index_and_excluded_change() {
    let repo = Repo::new();
    let first = repo.commit("asset.asset", yaml(120, 5, "base"));
    repo.commit("asset.asset", yaml(120, 5, "excluded"));
    let third = repo.commit("asset.asset", yaml(120, 7, "excluded"));
    std::fs::write(repo.root.join("notes.txt"), "staged notes\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "unstaged notes\n").unwrap();
    std::fs::write(repo.root.join("local.txt"), "untracked\n").unwrap();
    std::fs::write(repo.root.join("asset.asset"), yaml(100, 5, "dirty target")).unwrap();
    let before_head = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    let before_index = index_entries(&repo.root).unwrap();
    let job = repo.job(vec![first, third]);
    let empty = preview(&repo.root, &job.id).unwrap();
    assert!(empty.files.is_empty());
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(120, 7, "dirty target")
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("notes.txt")).unwrap(),
        "unstaged notes\n"
    );
    assert_eq!(
        git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(),
        before_head
    );
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    let retried = execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(retried["idempotent"], true);
    assert!(execute(
        &repo.root,
        &job.id,
        "stage",
        json!({"paths":["asset.asset"]})
    )
    .unwrap_err()
    .contains("pre-existing"));
}

#[test]
fn conflicting_field_requires_explicit_resolution_and_stale_preview_is_rejected() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    std::fs::write(repo.root.join("asset.asset"), yaml(130, 5, "base")).unwrap();
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let conflict = preview(&repo.root, &job.id).unwrap();
    assert!(!conflict.ready_to_apply);
    execute(
        &repo.root,
        &job.id,
        "resolve",
        json!({"all":true,"side":"source"}),
    )
    .unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert!(execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":conflict.plan_hash})
    )
    .unwrap_err()
    .contains("stale"));
    std::fs::write(repo.root.join("asset.asset"), yaml(140, 5, "base")).unwrap();
    assert!(execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash})
    )
    .unwrap_err()
    .contains("stale"));
    assert!(std::fs::read_to_string(repo.root.join("asset.asset"))
        .unwrap()
        .contains("140"));
}

#[test]
fn binary_requires_whole_version_and_guards_overlap() {
    let repo = Repo::new();
    let source = repo.commit("model.fbx", b"opaque\0binary");
    let job = repo.job(vec![source.clone()]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    assert!(!preview(&repo.root, &job.id).unwrap().ready_to_apply);
    execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"model.fbx","version":"source","commit":source}),
    )
    .unwrap();
    assert!(preview(&repo.root, &job.id)
        .unwrap()
        .issues
        .iter()
        .any(|v| v["code"] == "overlapping_selection"));
    execute(&repo.root, &job.id, "exclude", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(repo.root.join("model.fbx")).unwrap(),
        b"opaque\0binary"
    );
}

#[test]
fn guarded_abort_does_not_overwrite_later_editor_changes() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    std::fs::write(repo.root.join("asset.asset"), yaml(999, 5, "editor")).unwrap();
    assert!(execute(&repo.root, &job.id, "abort", json!({}))
        .unwrap_err()
        .contains("Later external edits"));
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(999, 5, "editor")
    );
}

#[test]
fn text_merges_independent_changes_and_rejects_overlap() {
    assert_eq!(
        merge_text(b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n").unwrap(),
        b"A\nb\nC\n"
    );
    assert!(merge_text(b"a\nb\n", b"a\nB\n", b"a\nother\n").is_err());
}

#[test]
fn whole_object_take_includes_source_unchanged_fields() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    std::fs::write(repo.root.join("asset.asset"), yaml(100, 9, "dirty target")).unwrap();
    let job = repo.job(vec![source.clone()]);
    execute(
        &repo.root,
        &job.id,
        "objects.take",
        json!({"path":"asset.asset","object_id":"11400000","commit":source,"side":"source"}),
    )
    .unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(120, 5, "base")
    );
}

#[test]
fn new_worktree_preserves_dirty_base_head_index_and_disk_layers() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    std::fs::write(repo.root.join("notes.txt"), "staged\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "working\n").unwrap();
    let original = index_entries(&repo.root).unwrap();
    let path = repo._dir.path().join("new-target");
    let job = prepare_with_destination(
        &repo.root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![source],
                ..Default::default()
            }],
            ..Default::default()
        },
        &json!({"kind":"new_branch","name":"integrated","location":"new_worktree","path":path}),
    )
    .unwrap();
    assert_eq!(index_entries(Path::new(&job.root)).unwrap(), original);
    assert_eq!(index_entries(&repo.root).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(path.join("notes.txt")).unwrap(),
        "working\n"
    );
    assert_eq!(
        git_text(&path, &["symbolic-ref", "HEAD"]).unwrap(),
        "refs/heads/integrated"
    );
}

#[test]
fn project_scope_uses_components_and_accepts_equivalent_windows_prefixes() {
    let repo = Repo::new();
    let project = repo.root.join("Project");
    assert!(path_in_project(&repo.root, &project, "Project/Assets/New.asset"));
    assert!(!path_in_project(&repo.root, &project, "ProjectOther/Assets/New.asset"));
    assert!(!path_in_project(&repo.root, &project, "Project"));
    assert!(!path_in_project(&repo.root, &project, "Project/../Outside.asset"));
    #[cfg(windows)]
    {
        std::fs::create_dir(&project).unwrap();
        let native_project = std::fs::canonicalize(&project).unwrap();
        assert!(path_in_project(&repo.root, &native_project, "Project/Assets/New.asset"));
        let native_repo = std::fs::canonicalize(&repo.root).unwrap();
        assert!(path_in_project(&native_repo, &project, "Project/Assets/Deleted.asset"));
    }
}

#[test]
fn new_worktree_without_explicit_path_creates_container_and_preserves_dirty_base() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let container = repo._dir.path().join("target.worktrees");
    assert!(!container.exists());
    std::fs::write(repo.root.join("notes.txt"), "staged\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "working\n").unwrap();
    let original_index = index_entries(&repo.root).unwrap();
    let original_head = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    let job = prepare_with_destination(
        &repo.root,
        &PrepareRequest {
            sources: vec![SourceSelection { commits: vec![source], ..Default::default() }],
            ..Default::default()
        },
        &json!({"kind":"new_branch","name":"codex/default-destination","location":"new_worktree"}),
    ).unwrap();
    let target = Path::new(&job.root);
    assert_eq!(target.parent().unwrap(), dunce::canonicalize(&container).unwrap());
    assert!(target.file_name().unwrap().to_string_lossy().starts_with("merge-"));
    assert_eq!(index_entries(target).unwrap(), original_index);
    assert_eq!(index_entries(&repo.root).unwrap(), original_index);
    assert_eq!(git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(), original_head);
    assert_eq!(std::fs::read_to_string(target.join("notes.txt")).unwrap(), "working\n");
    assert_eq!(git_text(target, &["symbolic-ref", "HEAD"]).unwrap(), "refs/heads/codex/default-destination");
}

#[test]
#[cfg(windows)]
fn new_worktree_transfers_long_frozen_working_paths_and_preserves_staging() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let relative = format!("Assets/{}/{}/long.asset", "a".repeat(110), "b".repeat(110));
    let existing = repo.root.join(&relative);
    std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
    std::fs::write(&existing, b"staged long-path bytes\n").unwrap();
    git(&repo.root, &["add", "--", &relative]).unwrap();
    std::fs::write(&existing, b"working long-path bytes\n").unwrap();
    let original_index = index_entries(&repo.root).unwrap();
    let job = prepare_with_destination(
        &repo.root,
        &PrepareRequest {
            sources: vec![SourceSelection { commits: vec![source], ..Default::default() }],
            ..Default::default()
        },
        &json!({"kind":"new_branch","name":"codex/long-frozen-destination","location":"new_worktree"}),
    ).unwrap();
    let target = Path::new(&job.root);
    let transferred = target.join(&relative);
    assert!(transferred.to_string_lossy().len() > 260);
    assert_eq!(std::fs::read(&transferred).unwrap(), b"working long-path bytes\n");
    assert_eq!(std::fs::read(&existing).unwrap(), b"working long-path bytes\n");
    assert_eq!(git(target, &["show", &format!(":{relative}")]).unwrap(), b"staged long-path bytes\n");
    assert_eq!(index_entries(target).unwrap(), original_index);
    assert_eq!(index_entries(&repo.root).unwrap(), original_index);
}

#[test]
fn default_worktree_container_rejects_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let container = temp.path().join("project.worktrees");
    std::fs::write(&container, b"preserve existing file").unwrap();
    let error = default_worktree_destination(&root).unwrap_err();
    assert!(error.contains("without links") && error.contains("project.worktrees"));
    assert_eq!(std::fs::read(&container).unwrap(), b"preserve existing file");
}

#[test]
fn default_worktree_container_rejects_directory_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let container = temp.path().join("project.worktrees");
    #[cfg(windows)]
    let link = std::os::windows::fs::symlink_dir(&root, &container);
    #[cfg(unix)]
    let link = std::os::unix::fs::symlink(&root, &container);
    if let Err(error) = link {
        if error.kind() == std::io::ErrorKind::PermissionDenied { return; }
        panic!("Could not create directory link fixture: {error}");
    }
    assert!(default_worktree_destination(&root).unwrap_err().contains("without links"));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn explicit_text_commit_does_not_include_unrelated_staged_files() {
    let repo = Repo::new();
    let source = repo.commit("new.txt", "new source\n");
    std::fs::write(repo.root.join("notes.txt"), "user staged\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    let commit = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"Integrate selected text"}),
    )
    .unwrap();
    assert_eq!(
        git_text(&repo.root, &["show", "HEAD:notes.txt"]).unwrap(),
        "base notes"
    );
    assert!(git_text(&repo.root, &["diff", "--cached", "--name-only"])
        .unwrap()
        .contains("notes.txt"));
    let retried = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"Ignored retry"}),
    )
    .unwrap();
    assert_eq!(retried["commit"], commit["commit"]);
    assert_eq!(retried["idempotent"], true);
}

#[test]
fn explicit_field_transform_can_change_a_field_absent_from_source_delta() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    execute(&repo.root,&job.id,"fields.set",json!({"path":"asset.asset","object_id":"11400000","property_path":"/MonoBehaviour/energy","value":99})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(120, 99, "base")
    );
}

#[test]
fn removed_guid_is_detected_in_an_unchanged_referencing_asset() {
    let repo = Repo::new();
    let guid = "1234567890abcdef1234567890abcdef";
    let reference=format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &1\nMonoBehaviour:\n  linked: {{fileID: 11400000, guid: {guid}, type: 2}}\n");
    std::fs::write(repo.root.join("other.asset"), reference).unwrap();
    std::fs::write(
        repo.root.join("asset.asset.meta"),
        format!("fileFormatVersion: 2\nguid: {guid}\n"),
    )
    .unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "references"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let commit = repo.commit(
        "asset.asset.meta",
        "fileFormatVersion: 2\nguid: fedcba0987654321fedcba0987654321\n",
    );
    let job = repo.job(vec![commit.clone()]);
    execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"asset.asset.meta","version":"source","commit":commit}),
    )
    .unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(
        plan.issues
            .iter()
            .any(|i| i["code"] == "missing_guid_dependency" && i["path"] == "other.asset"),
        "{:?}",
        plan.issues
    );
}

#[test]
fn frozen_formerly_serialized_alias_maps_legacy_source_into_dirty_renamed_target() {
    let repo = Repo::new();
    let guid = "11112222333344445555666677778888";
    let base_script="using UnityEngine; public class Example : ScriptableObject { public int legacyValue; public int local; }";
    let target_script="using UnityEngine; using UnityEngine.Serialization; public class Example : ScriptableObject { [FormerlySerializedAs(\"legacyValue\")] public int currentValue; public int local; }";
    let asset = |field: &str, value: u32, local: u32| {
        format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n  {field}: {value}\n  local: {local}\n")
    };
    std::fs::write(repo.root.join("Example.cs"), base_script).unwrap();
    std::fs::write(
        repo.root.join("Example.cs.meta"),
        format!("fileFormatVersion: 2\nguid: {guid}\n"),
    )
    .unwrap();
    std::fs::write(repo.root.join("asset.asset"), asset("legacyValue", 10, 1)).unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "script baseline"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let commit = repo.commit("asset.asset", asset("legacyValue", 20, 1));
    std::fs::write(repo.root.join("Example.cs"), target_script).unwrap();
    std::fs::write(repo.root.join("asset.asset"), asset("currentValue", 10, 99)).unwrap();
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        asset("currentValue", 20, 99)
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Example.cs")).unwrap(),
        target_script
    );
}

#[test]
fn staged_change_hidden_by_head_working_bytes_requires_explicit_scope_consent() {
    let repo = Repo::new();
    let commit = repo.commit("notes.txt", "incoming\n");
    std::fs::write(repo.root.join("notes.txt"), "hidden staged\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "base notes\n").unwrap();
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    let before = index_entries(&repo.root).unwrap();
    assert!(
        execute(&repo.root, &job.id, "stage", json!({"paths":["notes.txt"]}))
            .unwrap_err()
            .contains("pre-existing")
    );
    assert!(execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["notes.txt"],"message":"must not overwrite staged"})
    )
    .unwrap_err()
    .contains("pre-existing"));
    assert_eq!(before, index_entries(&repo.root).unwrap());
}

#[test]
fn changing_branch_at_same_head_invalidates_stage_and_commit_destination() {
    let repo = Repo::new();
    let commit = repo.commit("new.txt", "incoming\n");
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    git(&repo.root, &["switch", "-c", "other-destination"]).unwrap();
    assert!(
        execute(&repo.root, &job.id, "stage", json!({"paths":["new.txt"]}))
            .unwrap_err()
            .contains("branch changed")
    );
    assert!(execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"wrong branch"})
    )
    .unwrap_err()
    .contains("branch changed"));
}

#[test]
fn interrupted_staging_cannot_abort_worktree_while_leaving_new_index() {
    let repo = Repo::new();
    let commit = repo.commit("new.txt", "incoming\n");
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    execute(&repo.root, &job.id, "stage", json!({"paths":["new.txt"]})).unwrap();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let mut interrupted = load(&repo.root, &job.id).unwrap();
    interrupted.state = "staging".into();
    save(&dir, &interrupted).unwrap();
    assert!(execute(&repo.root, &job.id, "abort", json!({})).is_err());
    assert!(repo.root.join("new.txt").exists());
    execute(&repo.root, &job.id, "stage", json!({"paths":["new.txt"]})).unwrap();
    assert_eq!(load(&repo.root, &job.id).unwrap().state, "staged");
}

#[test]
fn interrupted_commit_before_ref_update_recovers_exact_pending_commit() {
    let repo = Repo::new();
    let commit = repo.commit("new.txt", "incoming\n");
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    let index_path = repo.root.join(".git/index");
    let before = std::fs::read(&index_path).unwrap();
    let committed = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"pending commit"}),
    )
    .unwrap();
    let oid = committed["commit"].as_str().unwrap();
    git(&repo.root, &["update-ref", "HEAD", &job.snapshot.head, oid]).unwrap();
    std::fs::write(&index_path, before).unwrap();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let mut pending = load(&repo.root, &job.id).unwrap();
    pending.state = "committing".into();
    save(&dir, &pending).unwrap();
    let recovered = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"retry"}),
    )
    .unwrap();
    assert_eq!(recovered["commit"], oid);
    assert_eq!(recovered["recovered"], true);
    assert_eq!(git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(), oid);
}

#[test]
fn binary_lfs_take_materializes_verified_local_object_never_pointer() {
    use sha2::Digest;
    let repo = Repo::new();
    let bytes = b"complete fbx bytes\0binary";
    let oid = sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {}\n",
        bytes.len()
    );
    let commit = repo.commit("model.fbx", pointer);
    let local = repo
        .root
        .join(".git/lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(&oid);
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(local, bytes).unwrap();
    let job = repo.job(vec![commit.clone()]);
    execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"model.fbx","version":"source","commit":commit}),
    )
    .unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(std::fs::read(repo.root.join("model.fbx")).unwrap(), bytes);
}

#[test]
fn monorepo_project_scope_reports_sibling_changes_without_writing_them() {
    let repo = Repo::new();
    for name in ["A", "B"] {
        std::fs::create_dir_all(repo.root.join(name)).unwrap();
    }
    let a = repo.commit("A/a.txt", "source A\n");
    let b = repo.commit("B/b.txt", "source B\n");
    let project = repo.root.join("A");
    let job = prepare(
        &project,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![a, b],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    execute(&project, &job.id, "include", json!({"all":true})).unwrap();
    let blocked = preview(&project, &job.id).unwrap();
    assert!(blocked
        .issues
        .iter()
        .any(|i| i["code"] == "outside_destination_scope" && i["path"] == "B/b.txt"));
    execute(&project, &job.id, "exclude", json!({"path":"B/b.txt"})).unwrap();
    let plan = preview(&project, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &project,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "source A\n"
    );
    assert!(!repo.root.join("B/b.txt").exists());
    assert!(load(&repo.root.join("B"), &job.id)
        .unwrap_err()
        .contains("another Unity project root"));
}

#[test]
fn commit_certificate_is_bound_to_the_exact_tree_and_preserves_unrelated_dirty_assets() {
    let repo = Repo::new();
    std::fs::write(
        repo.root.join("unrelated.asset"),
        yaml(10, 1, "base unrelated"),
    )
    .unwrap();
    git(&repo.root, &["add", "unrelated.asset"]).unwrap();
    git(&repo.root, &["commit", "-m", "unrelated baseline"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    std::fs::write(
        repo.root.join("unrelated.asset"),
        yaml(999, 1, "dirty unrelated"),
    )
    .unwrap();
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let mut saved = load(&repo.root, &job.id).unwrap();
    let outputs = saved.applied_files.clone();
    let tree =
        commit_validation::build_tree(&repo.root, &dir, &saved.snapshot.head, &outputs).unwrap();
    let paths = vec!["asset.asset".to_string()];
    let key = commit_validation::key(&saved, &tree, &paths, false).unwrap();
    // The unit test supplies only the already-verified certificate contract;
    // the CLI acceptance verifies certificate production in a real Editor.
    saved.commit_validations.insert(
        key.clone(),
        CommitValidation {
            key,
            tree: tree.clone(),
            plan_hash: saved.applied_hash.clone().unwrap(),
            parent: saved.snapshot.head.clone(),
            paths: paths.clone(),
            include_local_changes: false,
            validated: true,
            details: json!({"test_certificate":true}),
        },
    );
    assert!(commit_validation::covers(&saved, &tree, &paths, false).unwrap());
    assert!(!commit_validation::covers(&saved, &tree, &paths, true).unwrap());
    assert!(!commit_validation::covers(&saved, &saved.snapshot.head, &paths, false).unwrap());
    save(&dir, &saved).unwrap();
    let committed = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":paths,"message":"Commit exactly the validated subset"}),
    )
    .unwrap();
    assert_eq!(
        git_text(&repo.root, &["rev-parse", "HEAD^{tree}"]).unwrap(),
        tree
    );
    assert!(committed["commit"].is_string());
    assert_eq!(
        git_text(&repo.root, &["show", "HEAD:unrelated.asset"]).unwrap(),
        yaml(10, 1, "base unrelated").trim_end()
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("unrelated.asset")).unwrap(),
        yaml(999, 1, "dirty unrelated")
    );
}

#[test]
fn inspect_unchanged_frozen_asset_returns_addressable_managed_reference_fields() {
    let repo = Repo::new();
    let graph = concat!(
        "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n",
        "  node: {rid: 9007199254740993}\n  references:\n    version: 2\n    RefIds:\n",
        "    - rid: 9007199254740993\n      type: {class: Node, ns: Tests, asm: Assembly-CSharp}\n",
        "      data:\n        health: 10\n        next: {rid: 9007199254740993}\n"
    );
    repo.commit("graph.asset", graph);
    let selected = repo.commit("notes.txt", "source notes\n");
    std::fs::write(repo.root.join("graph.asset"), graph).unwrap();
    let job = repo.job(vec![selected.clone()]);
    // A later edit does not turn inspection into a live disk read.
    std::fs::write(
        repo.root.join("graph.asset"),
        graph.replace("health: 10", "health: 88"),
    )
    .unwrap();
    let path = "/MonoBehaviour/references/RefIds/@rid=9007199254740993/data/health";
    for version in ["target", "source", "base"] {
        let result = execute(
            &repo.root,
            &job.id,
            "inspect_asset",
            json!({"path":"graph.asset",
            "version":version,"commit":selected,"object_id":"11400000","property_path":path}),
        )
        .unwrap();
        assert_eq!(result["fields"][0]["scalar_text"], "10");
        assert_eq!(result["fields"][0]["property_path"], path);
        assert_eq!(result["fields"][0]["stable_identity_path"], true);
        assert!(result["references"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["rid"] == "9007199254740993"));
    }
    execute(
        &repo.root,
        &job.id,
        "fields.set",
        json!({"path":"graph.asset","object_id":"11400000",
        "property_path":path,"value":42}),
    )
    .unwrap();
    let result = execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"graph.asset","version":"result",
        "object_id":"11400000","property_path":path}),
    )
    .unwrap();
    assert_eq!(result["fields"][0]["scalar_text"], "42");
    assert_eq!(result["preview"]["ready_to_apply"], true);
    assert_eq!(load(&repo.root, &job.id).unwrap().revision, 1);
}

#[test]
fn inspect_pages_all_fields_and_requires_explicit_source_version() {
    let repo = Repo::new();
    let first = repo.commit("asset.asset", yaml(110, 5, "one"));
    let second = repo.commit("asset.asset", yaml(120, 5, "two"));
    let job = repo.job(vec![first.clone(), second]);
    let first_page = execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"asset.asset","limit":2}),
    )
    .unwrap();
    assert_eq!(first_page["fields"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["next_offset"], 2);
    assert_eq!(first_page["objects"][0]["object_id"], "11400000");
    assert!(execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"asset.asset","version":"source"})
    )
    .unwrap_err()
    .contains("exact selected commit"));
    let first_source = execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"asset.asset","version":"source",
        "commit":first,"property_path":"/MonoBehaviour/health"}),
    )
    .unwrap();
    assert_eq!(first_source["fields"][0]["scalar_text"], "110");
    assert!(execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"../asset.asset"})
    )
    .is_err());
    assert_eq!(load(&repo.root, &job.id).unwrap().revision, 0);
}

#[test]
fn typed_reference_values_preserve_i64_and_reject_quoted_numeric_strings() {
    let repo = Repo::new();
    let graph = concat!(
        "--- !u!114 &11400000\nMonoBehaviour:\n",
        "  object: {fileID: 9007199254740993}\n  node: {rid: 9007199254740993}\n",
        "  references:\n    version: 2\n    RefIds:\n",
        "    - rid: 9007199254740993\n      type: {class: Node, ns: Tests, asm: Assembly-CSharp}\n",
        "      data:\n        value: 1\n",
        "--- !u!114 &9007199254740993\nMonoBehaviour:\n  value: 1\n"
    );
    repo.commit("graph.asset", graph);
    let selected = repo.commit("notes.txt", "selected notes\n");
    std::fs::write(repo.root.join("graph.asset"), graph).unwrap();
    let job = repo.job(vec![selected]);
    for (path, diagnostic) in [
        ("/MonoBehaviour/object/fileID", "invalid_file_id"),
        ("/MonoBehaviour/node/rid", "invalid_rid"),
    ] {
        execute(&repo.root, &job.id, "fields.set", json!({"path":"graph.asset","object_id":"11400000","property_path":path,"value":"9007199254740993"})).unwrap();
        let invalid = preview(&repo.root, &job.id).unwrap();
        assert!(!invalid.ready_to_apply);
        assert!(serde_json::to_string(&invalid.issues)
            .unwrap()
            .contains(diagnostic));
        execute(&repo.root, &job.id, "fields.set", json!({"path":"graph.asset","object_id":"11400000","property_path":path,"value":9007199254740993i64})).unwrap();
        let view = execute(&repo.root, &job.id, "inspect_asset", json!({"path":"graph.asset","version":"result","object_id":"11400000","property_path":path})).unwrap();
        assert_eq!(view["preview"]["ready_to_apply"], true);
        assert_eq!(view["fields"][0]["scalar_text"], "9007199254740993");
        assert_eq!(view["fields"][0]["scalar_style"], "plain");
    }
}

#[test]
fn structural_field_selection_preserves_target_mode_and_mode_only_commits_are_visible() {
    let repo = Repo::new();
    git(&repo.root, &["update-index", "--chmod=+x", "asset.asset"]).unwrap();
    git(&repo.root, &["commit", "-m", "Target executable mode"]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            repo.root.join("asset.asset"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert_eq!(plan.files["asset.asset"].as_ref().unwrap().mode, "100755");
    let before_index = index_entries(&repo.root).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        execute(
            &repo.root,
            &job.id,
            "apply",
            json!({"expected_plan_hash":plan.plan_hash})
        )
        .unwrap()["idempotent"],
        true
    );
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    execute(&repo.root, &job.id, "abort", json!({})).unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(100, 5, "base")
    );
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    git(&repo.source, &["update-index", "--chmod=+x", "asset.asset"]).unwrap();
    git(&repo.source, &["commit", "-m", "Source mode-only change"]).unwrap();
    let source = git_text(&repo.source, &["rev-parse", "HEAD"]).unwrap();
    let mode_job = repo.job(vec![source.clone()]);
    assert_eq!(mode_job.deltas[0].changes[0]["kind"], "file_mode");
    execute(&repo.root, &mode_job.id, "include", json!({"all":true})).unwrap();
    assert!(!preview(&repo.root, &mode_job.id).unwrap().ready_to_apply);
    execute(&repo.root, &mode_job.id, "new_plan", json!({})).unwrap();
    execute(
        &repo.root,
        &mode_job.id,
        "files.take",
        json!({"path":"asset.asset","version":"source","commit":source}),
    )
    .unwrap();
    assert!(preview(&repo.root, &mode_job.id).unwrap().ready_to_apply);
}

#[test]
fn content_inclusion_keeps_target_mode_when_source_also_changes_mode() {
    for (path, content) in [
        ("asset.asset", yaml(120, 5, "base")),
        ("notes.txt", "source notes\n".into()),
    ] {
        let repo = Repo::new();
        std::fs::write(repo.source.join(path), content.as_bytes()).unwrap();
        git(&repo.source, &["add", "--", path]).unwrap();
        git(&repo.source, &["update-index", "--chmod=+x", path]).unwrap();
        git(
            &repo.source,
            &["commit", "-m", "Content and executable mode"],
        )
        .unwrap();
        let commit = git_text(&repo.source, &["rev-parse", "HEAD"]).unwrap();
        let job = repo.job(vec![commit]);
        let mode = job.deltas[0]
            .changes
            .iter()
            .find(|c| c["kind"] == "file_mode")
            .unwrap();
        execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
        assert!(!preview(&repo.root, &job.id).unwrap().ready_to_apply);
        execute(
            &repo.root,
            &job.id,
            "exclude",
            json!({"change_ids":[mode["id"]]}),
        )
        .unwrap();
        let plan = preview(&repo.root, &job.id).unwrap();
        assert!(plan.ready_to_apply, "{path}: {:?}", plan.issues);
        assert_eq!(plan.files[path].as_ref().unwrap().mode, "100644");
        assert_eq!(
            read_blob(
                &job_dir(&repo.root, &job.id).unwrap(),
                plan.files[path].as_ref().unwrap()
            )
            .unwrap(),
            content.as_bytes()
        );
        let before_index = index_entries(&repo.root).unwrap();
        execute(
            &repo.root,
            &job.id,
            "apply",
            json!({"expected_plan_hash":plan.plan_hash}),
        )
        .unwrap();
        assert_eq!(index_entries(&repo.root).unwrap(), before_index);
        assert_eq!(
            execute(
                &repo.root,
                &job.id,
                "apply",
                json!({"expected_plan_hash":plan.plan_hash})
            )
            .unwrap()["idempotent"],
            true
        );
    }
}

#[test]
fn whole_file_mode_survives_apply_stage_commit_and_journal_recovery() {
    let repo = Repo::new();
    git(&repo.source, &["update-index", "--chmod=+x", "notes.txt"]).unwrap();
    git(&repo.source, &["commit", "-m", "Source mode only"]).unwrap();
    let source = git_text(&repo.source, &["rev-parse", "HEAD"]).unwrap();
    std::fs::write(repo.root.join("asset.asset"), yaml(101, 5, "staged local")).unwrap();
    git(&repo.root, &["add", "asset.asset"]).unwrap();
    std::fs::write(
        repo.root.join("asset.asset"),
        yaml(102, 5, "unstaged local"),
    )
    .unwrap();
    let before_index = index_entries(&repo.root).unwrap();
    let local_entry = before_index
        .0
        .split('\0')
        .find(|s| s.ends_with("\tasset.asset"))
        .unwrap()
        .to_string();
    let head = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    let job = repo.job(vec![source.clone()]);
    assert_eq!(job.deltas[0].changes.len(), 1);
    assert_eq!(job.deltas[0].changes[0]["kind"], "file_mode");
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    assert!(!preview(&repo.root, &job.id).unwrap().ready_to_apply);
    execute(&repo.root, &job.id, "new_plan", json!({})).unwrap();
    execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"notes.txt","version":"source","commit":source}),
    )
    .unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert_eq!(plan.files["notes.txt"].as_ref().unwrap().mode, "100755");
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    assert_eq!(git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(
        execute(
            &repo.root,
            &job.id,
            "apply",
            json!({"expected_plan_hash":plan.plan_hash})
        )
        .unwrap()["idempotent"],
        true
    );
    execute(&repo.root, &job.id, "stage", json!({"paths":["notes.txt"]})).unwrap();
    assert!(index_entries(&repo.root)
        .unwrap()
        .0
        .split('\0')
        .any(|s| s.starts_with("100755 ") && s.ends_with("\tnotes.txt")));
    // Simulate a crash after installing the index but before recording staged.
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let mut pending = load(&repo.root, &job.id).unwrap();
    pending.state = "staging".into();
    save(&dir, &pending).unwrap();
    execute(&repo.root, &job.id, "stage", json!({"paths":["notes.txt"]})).unwrap();
    let params = json!({"paths":["notes.txt"],"message":"Explicit executable mode only"});
    let committed = execute(&repo.root, &job.id, "commit", params.clone()).unwrap();
    assert!(
        git_text(&repo.root, &["ls-tree", "HEAD", "--", "notes.txt"])
            .unwrap()
            .starts_with("100755 ")
    );
    assert_eq!(
        git_text(&repo.root, &["show", "HEAD:asset.asset"]).unwrap(),
        yaml(100, 5, "base").trim_end()
    );
    assert!(index_entries(&repo.root)
        .unwrap()
        .0
        .split('\0')
        .any(|s| s == local_entry));
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(102, 5, "unstaged local")
    );
    // Likewise recover a post-ref/post-index commit without absorbing local I/W.
    let mut pending = load(&repo.root, &job.id).unwrap();
    pending.state = "committing".into();
    save(&dir, &pending).unwrap();
    let retry = execute(&repo.root, &job.id, "commit", params).unwrap();
    assert_eq!(retry["commit"], committed["commit"]);
    assert_eq!(retry["idempotent"], true);
    assert!(index_entries(&repo.root)
        .unwrap()
        .0
        .split('\0')
        .any(|s| s == local_entry));
}

#[test]
fn malformed_selectors_never_broaden_selection_or_mutate_the_plan() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 7, "changed"));
    let job = repo.job(vec![source]);
    for params in [
        json!({"path":"asset.asset","object_id":11400000}),
        json!({"path":"asset.asset","property_path":42}),
        json!({"path":"asset.asset","commit":true}),
        json!({"path":"asset.asset","change_ids":"wrong"}),
        json!({"path":"asset.asset","change_ids":[42]}),
        json!({"path":"asset.asset","all":"true"}),
        json!({"path":"asset.asset","objectId":"11400000"}),
        json!({"path":"asset.asset","propertyPath":"/MonoBehaviour/health"}),
        json!({"path":"asset.asset","change_ids":[job.deltas[0].changes[0]["id"]]}),
        json!({"object_id":"11400000","change_ids":[job.deltas[0].changes[0]["id"]]}),
    ] {
        assert!(
            execute(&repo.root, &job.id, "include", params.clone()).is_err(),
            "{params}"
        );
        let saved = load(&repo.root, &job.id).unwrap();
        assert_eq!(saved.revision, 0);
        assert!(saved.selection.decisions.is_empty());
    }
    for (action, params) in [
        (
            "files.take",
            json!({"path":"asset.asset","version":{"side":"source","commit_id":"ignored"}}),
        ),
        (
            "files.take",
            json!({"path":"asset.asset","version":{"side":"source","commit":42}}),
        ),
        (
            "files.take",
            json!({"path":"asset.asset","commit":"one","version":{"side":"source","commit":"another"}}),
        ),
        (
            "objects.take",
            json!({"path":"asset.asset","object_id":"11400000","commit_id":"ignored"}),
        ),
        (
            "fields.set",
            json!({"path":"asset.asset","object_id":"11400000","property_path":"/MonoBehaviour/health","value":42,"commit_id":"ignored"}),
        ),
        (
            "objects.move",
            json!({"path":"asset.asset","object_id":"11400000","parent_id":"0","position":-1}),
        ),
        (
            "inspect_asset",
            json!({"path":"asset.asset","objectId":"11400000"}),
        ),
    ] {
        assert!(
            execute(&repo.root, &job.id, action, params).is_err(),
            "{action}"
        );
        assert_eq!(load(&repo.root, &job.id).unwrap().revision, 0);
    }
    assert!(execute(
        &repo.root,
        &job.id,
        "inspect_asset",
        json!({"path":"asset.asset","object_id":11400000})
    )
    .is_err());
    assert!(
        execute(&repo.root, &job.id, "changes", json!({"offset":u64::MAX})).unwrap()["next_offset"]
            .is_null()
    );
    execute(
        &repo.root,
        &job.id,
        "include",
        json!({"path":"asset.asset","change_ids":null,"object_id":null,"commit":null}),
    )
    .unwrap();
    assert_eq!(
        load(&repo.root, &job.id).unwrap().selection.decisions.len(),
        job.deltas[0].changes.len()
    );
}

const EXTERNAL_PARENT_GUID: &str = "11111111111111111111111111111111";
const EXTERNAL_OTHER_GUID: &str = "22222222222222222222222222222222";

fn externally_referenced_prefab(with_child: bool) -> String {
    let children = if with_child {
        "\n  - {fileID: 4}"
    } else {
        " []"
    };
    let mut prefab = format!(
        "--- !u!1 &1\nGameObject:\n  m_Name: Parent\n  m_Component:\n  - component: {{fileID: 2}}\n\
         --- !u!4 &2\nTransform:\n  m_GameObject: {{fileID: 1}}\n  m_Father: {{fileID: 0}}\n  m_Children:{children}\n"
    );
    if with_child {
        prefab.push_str(
            "--- !u!1 &3\nGameObject:\n  m_Name: Child\n  m_Component:\n  - component: {fileID: 4}\n\
             --- !u!4 &4\nTransform:\n  m_GameObject: {fileID: 3}\n  m_Father: {fileID: 2}\n  m_Children: []\n"
        );
    }
    prefab
}

fn external_scene_reference(guid: &str, file_id: i64) -> String {
    format!("--- !u!114 &11400000\nMonoBehaviour:\n  m_Target: {{fileID: {file_id}, guid: {guid}, type: 3}}\n")
}

fn repo_with_external_prefab_reference(referrer_guid: &str) -> Repo {
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("Assets")).unwrap();
    for (path, content) in [
        ("Assets/Parent.prefab", externally_referenced_prefab(true)),
        (
            "Assets/Parent.prefab.meta",
            format!("fileFormatVersion: 2\nguid: {EXTERNAL_PARENT_GUID}\n"),
        ),
        ("Assets/Other.prefab", externally_referenced_prefab(true)),
        (
            "Assets/Other.prefab.meta",
            format!("fileFormatVersion: 2\nguid: {EXTERNAL_OTHER_GUID}\n"),
        ),
        (
            "Assets/Referrer.unity",
            external_scene_reference(referrer_guid, 3),
        ),
        (
            "Assets/Referrer.unity.meta",
            "fileFormatVersion: 2\nguid: 33333333333333333333333333333333\n".into(),
        ),
    ] {
        std::fs::write(repo.root.join(path), content).unwrap();
    }
    git(&repo.root, &["add", "Assets"]).unwrap();
    git(
        &repo.root,
        &["commit", "-m", "Prefab with an external scene reference"],
    )
    .unwrap();
    let baseline = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", &baseline]).unwrap();
    repo
}

#[test]
fn removed_prefab_document_blocks_unchanged_external_referrer() {
    let repo = repo_with_external_prefab_reference(EXTERNAL_PARENT_GUID);
    let source = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
    let before_head = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    let before_index = index_entries(&repo.root).unwrap();
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let issue = plan
        .issues
        .iter()
        .find(|issue| issue["code"] == "removed_object_dependency")
        .unwrap_or_else(|| {
            panic!(
                "Expected exact external object dependency: {:?}",
                plan.issues
            )
        });
    assert_eq!(issue["path"], "Assets/Referrer.unity");
    assert_eq!(issue["guid"], EXTERNAL_PARENT_GUID);
    assert_eq!(issue["file_id"], "3");
    assert_eq!(issue["object_id"], "11400000");
    assert_eq!(issue["property_path"], "/MonoBehaviour/m_Target");
    assert_eq!(issue["deleted_from"], "Assets/Parent.prefab");
    assert!(!plan.ready_to_apply);
    assert!(execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash})
    )
    .is_err());
    assert_eq!(
        git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(),
        before_head
    );
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Parent.prefab")).unwrap(),
        externally_referenced_prefab(true)
    );
}

#[test]
fn explicitly_selected_referrer_update_resolves_removed_document_dependency() {
    let repo = repo_with_external_prefab_reference(EXTERNAL_PARENT_GUID);
    let removed = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
    let fixed = repo.commit(
        "Assets/Referrer.unity",
        external_scene_reference(EXTERNAL_PARENT_GUID, 1),
    );
    let job = repo.job(vec![removed.clone(), fixed.clone()]);
    execute(&repo.root, &job.id, "include", json!({"commit":removed})).unwrap();
    let blocked = preview(&repo.root, &job.id).unwrap();
    assert!(blocked
        .issues
        .iter()
        .any(|issue| issue["code"] == "removed_object_dependency"));
    assert!(!blocked.files.contains_key("Assets/Referrer.unity"));
    execute(&repo.root, &job.id, "include", json!({"commit":fixed})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let before_index = index_entries(&repo.root).unwrap();
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(index_entries(&repo.root).unwrap(), before_index);
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Parent.prefab")).unwrap(),
        externally_referenced_prefab(false)
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Referrer.unity")).unwrap(),
        external_scene_reference(EXTERNAL_PARENT_GUID, 1)
    );
}

#[test]
fn removed_document_does_not_match_same_file_id_under_another_guid() {
    let repo = repo_with_external_prefab_reference(EXTERNAL_OTHER_GUID);
    let source = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert!(!plan.files.contains_key("Assets/Referrer.unity"));
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Referrer.unity")).unwrap(),
        external_scene_reference(EXTERNAL_OTHER_GUID, 3)
    );
}
