use super::*;
use crate::workspace_service::worktrees::location::WorktreePolicy;
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
        &WorktreePolicy::beside_repository(1),
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
        &WorktreePolicy::beside_repository(1),
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
        &WorktreePolicy::beside_repository(1),
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
    let head = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(commit["removed"], true);
    let retried = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"Ignored retry"}),
    )
    .unwrap_err();
    assert!(retried.contains("no longer exists"), "{retried}");
    assert_eq!(git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(), head);
}

/// Rebuilds the journal an interrupted commit leaves behind: the job as it
/// was before `commit`, now committing `oid` from `before` to `after` index bytes.
fn interrupted_commit(repo: &Repo, prior: &MergeJob, oid: &str, before: &[u8], after: &[u8]) {
    let dir = job_dir(&repo.root, &prior.id).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    init_storage(&repo.root, &dir).unwrap();
    let mut pending = prior.clone();
    pending.state = "committing".into();
    pending.commit_oid = Some(oid.into());
    pending.pending_index_before = Some(blake3::hash(before).to_hex().to_string());
    pending.pending_index_after = Some(store_blob(&dir, after, "index").unwrap());
    save(&dir, &pending).unwrap();
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
fn removed_guid_checks_unparsable_referrers_by_their_guid_keys() {
    let repo = Repo::new();
    let guid = "1234567890abcdef1234567890abcdef";
    let referrers = ["block.asset", "lexical.asset", "text.asset"];
    for (path, body) in referrers.iter().zip([
        format!("  linked:\n    fileID: 11400000\n    guid: {guid}\n"),
        format!("  linked: {{fileID: 11400000, guid: {guid}, type: 2}}\n"),
        format!(
            "  note: 'a wrapped string that\n    mentions guid: {guid} mid-line'\n  wwise:\n    guid: 00FFFCE6-9E61-4205-8AE9-DEDCAD2C0E03\n"
        ),
    ]) {
        std::fs::write(repo.root.join(path), unparsable_unity_yaml(&body)).unwrap();
    }
    std::fs::write(repo.root.join("asset.asset.meta"), format!("fileFormatVersion: 2\nguid: {guid}\n")).unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "references"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let commit = repo.commit("asset.asset.meta", "fileFormatVersion: 2\nguid: fedcba0987654321fedcba0987654321\n");
    let job = repo.job(vec![commit.clone()]);
    execute(&repo.root, &job.id, "files.take", json!({"path":"asset.asset.meta","version":"source","commit":commit})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let found: Vec<_> = plan
        .issues
        .iter()
        .filter(|i| referrers.contains(&i["path"].as_str().unwrap_or_default()))
        .collect();
    let codes: Vec<_> = found.iter().map(|i| (i["code"].as_str().unwrap(), i["path"].as_str().unwrap())).collect();
    // The exact reference is found lexically; a block-form `guid` key naming
    // the removed GUID stays unprovable; mid-line text and dashed GUIDs cannot.
    assert_eq!(
        codes,
        [("dependency_coverage", "block.asset"), ("missing_guid_dependency", "lexical.asset")],
        "{:?}",
        plan.issues
    );
    assert_eq!(found[0]["guids"], json!([guid]));
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
    let prior = load(&repo.root, &job.id).unwrap();
    let committed = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"pending commit"}),
    )
    .unwrap();
    let oid = committed["commit"].as_str().unwrap();
    let after = std::fs::read(&index_path).unwrap();
    git(&repo.root, &["update-ref", "HEAD", &job.snapshot.head, oid]).unwrap();
    std::fs::write(&index_path, &before).unwrap();
    interrupted_commit(&repo, &prior, oid, &before, &after);
    let recovered = execute(
        &repo.root,
        &job.id,
        "commit",
        json!({"paths":["new.txt"],"message":"retry"}),
    )
    .unwrap();
    assert_eq!(recovered["commit"], oid);
    assert_eq!(recovered["recovered"], true);
    assert_eq!(recovered["removed"], true);
    assert_eq!(git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap(), oid);
    assert_eq!(std::fs::read(&index_path).unwrap(), after);
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
fn missing_lfs_objects_are_reported_together_and_preview_recovers_after_fetch() {
    use sha2::Digest;
    let repo = Repo::new();
    git(&repo.root, &["remote", "add", "origin", "https://example.invalid/lfs.git"]).unwrap();
    let objects: [(&str, &[u8]); 2] = [
        ("model.fbx", b"complete fbx bytes\0binary"),
        ("Plugins/native.dll", b"MZ complete dll\0bytes"),
    ];
    let oid = |bytes: &[u8]| {
        sha2::Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let pointers: Vec<String> = objects
        .iter()
        .map(|(_, bytes)| {
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n",
                oid(bytes),
                bytes.len()
            )
        })
        .collect();
    let commit = repo.commit_files(&[
        (objects[0].0, Some(pointers[0].as_str())),
        (objects[1].0, Some(pointers[1].as_str())),
    ]);
    let job = repo.job(vec![commit.clone()]);
    for (path, _) in objects {
        execute(
            &repo.root,
            &job.id,
            "files.take",
            json!({"path":path,"version":"source","commit":commit}),
        )
        .unwrap();
    }

    let blocked = preview(&repo.root, &job.id).unwrap();
    assert!(!blocked.ready_to_apply);
    let missing: Vec<_> = blocked
        .issues
        .iter()
        .filter(|issue| issue["code"] == "lfs_object_missing")
        .collect();
    assert_eq!(missing.len(), 2, "{:?}", blocked.issues);
    let command =
        format!("git lfs fetch origin {commit} --include 'Plugins/native.dll,model.fbx'");
    for (path, bytes) in objects {
        let issue = missing.iter().find(|issue| issue["path"] == path).unwrap();
        assert_eq!(issue["oid"], oid(bytes));
        assert_eq!(issue["size"], bytes.len());
        assert_eq!(issue["commit"], commit.as_str());
        assert_eq!(issue["fetch_command"], command.as_str());
        assert!(!blocked.files.contains_key(path), "pointer bytes must never be planned");
    }

    for (_, bytes) in objects {
        let oid = oid(bytes);
        let local = repo
            .root
            .join(".git/lfs/objects")
            .join(&oid[..2])
            .join(&oid[2..4])
            .join(&oid);
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(local, bytes).unwrap();
    }
    // The job is unchanged, so a cached blocked preview would hide the fetch.
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    for (path, bytes) in objects {
        assert_eq!(std::fs::read(repo.root.join(path)).unwrap(), bytes);
    }
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
    let prior = load(&repo.root, &job.id).unwrap();
    let prior_index = std::fs::read(repo.root.join(".git/index")).unwrap();
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
    let index = std::fs::read(repo.root.join(".git/index")).unwrap();
    interrupted_commit(&repo, &prior, committed["commit"].as_str().unwrap(), &prior_index, &index);
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

/// A Unity document the structural parser rejects (a YAML alias is outside
/// its dialect), so dependency checks must rely on the lexical scan.
fn unparsable_unity_yaml(body: &str) -> String {
    let text = format!("--- !u!114 &11400000\nMonoBehaviour:\n  m_Shared: *anchor\n{body}");
    assert!(crate::unity_asset_core::parse(text.as_bytes()).is_err(), "fixture must be unparsable");
    text
}

const PLUGIN_META: &str = "fileFormatVersion: 2\nguid: 44444444444444444444444444444444\nPluginImporter:\n  platformData:\n  - first:\n      : Any\n    second:\n      enabled: 0\n";

#[test]
fn object_removal_checks_unparsable_assets_lexically_and_exactly() {
    // Unity's empty-key importer metadata now parses like any other asset.
    assert!(crate::unity_asset_core::parse(PLUGIN_META.as_bytes()).unwrap().is_writable());
    let repo = repo_with_external_prefab_reference(EXTERNAL_OTHER_GUID);
    let flow = |file_id: i64| format!("  m_Target: {{fileID: {file_id}, guid: {EXTERNAL_PARENT_GUID}, type: 3}}\n");
    for (path, content) in [
        ("Assets/Plugin.dll.meta", PLUGIN_META.to_owned()),
        // Recognized references: exact verdicts despite the parse failure.
        ("Assets/Lexical.asset", unparsable_unity_yaml(&flow(3))),
        // Spells the prefab GUID, but only in a kept reference, a wrapped
        // string's continuation text, and `guid` keys that cannot name it.
        (
            "Assets/Kept.asset",
            unparsable_unity_yaml(&format!(
                "{}  wwise:\n    guid: 00FFFCE6-9E61-4205-8AE9-DEDCAD2C0E03\n  bank:\n    guid: 33333333333333333333333333333333\n\
                 \x20 m_Text: 'notes that wrap\n    over the guid: {EXTERNAL_PARENT_GUID} line'\n",
                flow(1)
            )),
        ),
        // A block-form `guid` key is not provably a complete reference list.
        (
            "Assets/Legacy.asset",
            unparsable_unity_yaml(&format!("  m_Target:\n    fileID: 1\n    guid: {EXTERNAL_PARENT_GUID}\n")),
        ),
    ] {
        std::fs::write(repo.root.join(path), content).unwrap();
    }
    git(&repo.root, &["add", "Assets"]).unwrap();
    git(&repo.root, &["commit", "-m", "unparsable assets"]).unwrap();
    let baseline = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", &baseline]).unwrap();
    let source = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let codes: Vec<_> = plan.issues.iter().map(|i| (i["code"].as_str().unwrap(), i["path"].as_str().unwrap())).collect();
    assert_eq!(
        codes,
        [
            ("external_object_dependency_coverage", "Assets/Legacy.asset"),
            ("removed_object_dependency", "Assets/Lexical.asset"),
        ],
        "{:?}",
        plan.issues
    );
    let coverage = &plan.issues[0];
    assert_eq!(coverage["guids"], json!([EXTERNAL_PARENT_GUID]));
    assert_eq!(coverage["deleted_from"], json!(["Assets/Parent.prefab"]));
    let lexical = &plan.issues[1];
    assert_eq!((lexical["file_id"].as_str(), lexical["object_id"].as_str()), (Some("3"), Some("11400000")));
    assert_eq!(lexical["line"], 4);
    assert_eq!(lexical["property_path"], "/MonoBehaviour/m_Target");
    assert_eq!(lexical["property_path_approximate"], true);
}

#[test]
fn lfs_stored_yaml_dependencies_are_materialized_before_checking() {
    use sha2::Digest;
    let repo = repo_with_external_prefab_reference(EXTERNAL_OTHER_GUID);
    let content = external_scene_reference(EXTERNAL_PARENT_GUID, 3);
    let oid = sha2::Sha256::digest(content.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let pointer = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {}\n", content.len());
    std::fs::write(repo.root.join("Assets/Stored.unity"), &pointer).unwrap();
    git(&repo.root, &["add", "Assets"]).unwrap();
    git(&repo.root, &["commit", "-m", "LFS scene"]).unwrap();
    let baseline = git_text(&repo.root, &["rev-parse", "HEAD"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", &baseline]).unwrap();
    let source = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
    let issues = |repo: &Repo| {
        let job = repo.job(vec![source.clone()]);
        execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
        preview(&repo.root, &job.id).unwrap().issues
    };
    let missing = issues(&repo);
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0]["code"], "external_object_dependency_coverage");
    assert!(missing[0]["detail"].as_str().unwrap().contains("git lfs fetch"));
    let local = repo.root.join(".git/lfs/objects").join(&oid[..2]).join(&oid[2..4]).join(&oid);
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(local, &content).unwrap();
    let found = issues(&repo);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "removed_object_dependency");
    assert_eq!(found[0]["path"], "Assets/Stored.unity");
    assert_eq!(found[0]["property_path"], "/MonoBehaviour/m_Target");
}

#[test]
fn scene_object_removal_needs_no_cross_asset_proof() {
    let scene = |with_extra: bool| {
        let mut text = external_scene_reference(EXTERNAL_PARENT_GUID, 3);
        if with_extra {
            text.push_str("--- !u!114 &11400001\nMonoBehaviour:\n  m_Enabled: 1\n");
        }
        text
    };
    let repo = repo_with(&[
        ("Assets/Level.unity", scene(true).as_str()),
        ("Assets/Level.unity.meta", "fileFormatVersion: 2\nguid: 55555555555555555555555555555555\n"),
        (
            "Assets/Legacy.asset",
            unparsable_unity_yaml("  m_Scene:\n    fileID: 11400001\n    guid: 55555555555555555555555555555555\n")
                .as_str(),
        ),
    ]);
    let source = repo.commit("Assets/Level.unity", scene(false));
    let job = repo.job(vec![source]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert!(plan.files["Assets/Level.unity"].is_some());
}

#[test]
fn folder_meta_in_scope_reads_its_folder_as_an_absent_file() {
    let repo = Repo::new();
    let source = repo.commit_files(&[
        ("Assets/Materials.meta", Some("fileFormatVersion: 2\nguid: 66666666666666666666666666666666\nfolderAsset: yes\n")),
        ("Assets/Materials/Deck.txt", Some("deck\n")),
    ]);
    let request = |commit: &str| PrepareRequest {
        sources: vec![SourceSelection { commits: vec![commit.to_string()], ..Default::default() }],
        paths: Some(vec!["Assets/Materials.meta".into(), "Assets/Materials/Deck.txt".into()]),
        ..Default::default()
    };
    let job = prepare(&repo.root, &request(&source)).unwrap();
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    assert!(repo.root.join("Assets/Materials").is_dir());
    // Post-apply checks (stage/commit/Unity validation) fingerprint the whole scope.
    let applied = load(&repo.root, &job.id).unwrap();
    check_files(&repo.root, &job_dir(&repo.root, &job.id).unwrap(), &scoped_files(&applied)).unwrap();
    // Snapshotting the same scope once the folder exists also succeeds.
    prepare(&repo.root, &request(&source)).unwrap();
}

#[test]
fn squash_source_integrates_the_branch_net_result_as_one_delta() {
    let repo = Repo::new();
    // The destination moves on after the fork, so the merge base is not HEAD.
    std::fs::write(repo.root.join("target-only.txt"), "destination\n").unwrap();
    git(&repo.root, &["add", "target-only.txt"]).unwrap();
    git(&repo.root, &["commit", "-m", "destination change"]).unwrap();
    let fork = git_text(&repo.source, &["rev-parse", "HEAD"]).unwrap();
    repo.commit_files(&[("notes.txt", Some("draft\n")), ("scratch.txt", Some("temporary\n"))]);
    let tip = repo.commit_files(&[("notes.txt", Some("final notes\n")), ("scratch.txt", None)]);
    let request = |source: SourceSelection| PrepareRequest { sources: vec![source], ..Default::default() };
    let squash = |source: SourceSelection| SourceSelection { squash: true, ..source };
    for source in [
        squash(SourceSelection { branch_ref: Some("source".into()), ..Default::default() }),
        squash(SourceSelection { range: Some(format!("HEAD..{tip}")), ..Default::default() }),
    ] {
        let job = prepare(&repo.root, &request(source)).unwrap();
        assert!(job.deltas.iter().all(|d| d.commit == tip && d.parent == fork));
        // The file the branch added and later deleted is no change at all.
        let paths: BTreeSet<_> = job.deltas.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, BTreeSet::from(["notes.txt"]));
    }
    let job = prepare(
        &repo.root,
        &request(squash(SourceSelection { branch_ref: Some("source".into()), ..Default::default() })),
    )
    .unwrap();
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    assert_eq!(std::fs::read_to_string(repo.root.join("notes.txt")).unwrap(), "final notes\n");
    assert_eq!(std::fs::read_to_string(repo.root.join("target-only.txt")).unwrap(), "destination\n");
    assert!(!repo.root.join("scratch.txt").exists());
    for invalid in [
        squash(SourceSelection { commits: vec![tip.clone()], ..Default::default() }),
        squash(SourceSelection { range: Some(format!("HEAD...{tip}")), ..Default::default() }),
        // Nothing beyond the merge base.
        squash(SourceSelection { range: Some(format!("{tip}..{fork}")), ..Default::default() }),
        squash(SourceSelection::default()),
    ] {
        assert!(prepare(&repo.root, &request(invalid)).is_err());
    }
}

/// A repository whose base commit also contains `files`, with a source worktree.
fn repo_with(files: &[(&str, &str)]) -> Repo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("target");
    let source = dir.path().join("source");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-b", "main"]).unwrap();
    git(&root, &["config", "user.email", "merge-test@example.invalid"]).unwrap();
    git(&root, &["config", "user.name", "Merge Test"]).unwrap();
    git(&root, &["config", "core.autocrlf", "false"]).unwrap();
    std::fs::write(root.join("asset.asset"), yaml(100, 5, "base")).unwrap();
    std::fs::write(root.join("notes.txt"), "base notes\n").unwrap();
    for (path, content) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }
    git(&root, &["add", "."]).unwrap();
    git(&root, &["commit", "-m", "base"]).unwrap();
    git(&root, &["worktree", "add", "-b", "source", "--", source.to_str().unwrap()]).unwrap();
    Repo { _dir: dir, root, source }
}

impl Repo {
    /// One source commit writing (`Some`) or deleting (`None`) several files.
    fn commit_files(&self, files: &[(&str, Option<&str>)]) -> String {
        for (path, content) in files {
            let file = self.source.join(path);
            match content {
                Some(content) => {
                    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                    std::fs::write(&file, content).unwrap();
                    git(&self.source, &["add", "--", path]).unwrap();
                }
                None => {
                    git(&self.source, &["rm", "-q", "--", path]).unwrap();
                }
            }
        }
        git(&self.source, &["commit", "-m", "source change"]).unwrap();
        git_text(&self.source, &["rev-parse", "HEAD"]).unwrap()
    }
}

fn catalog_change(repo: &Repo, job: &MergeJob, path: &str) -> Value {
    let changes = execute(&repo.root, &job.id, "changes", json!({"limit":1000})).unwrap();
    changes["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == path)
        .cloned()
        .unwrap_or_else(|| panic!("no change for {path}: {changes:#}"))
}

#[test]
fn text_catalog_classifies_line_merges_line_endings_and_conflicts() {
    let repo = repo_with(&[
        ("eol.txt", "one\ntwo\nthree\n"),
        ("merge.txt", "1\n2\n3\n4\n5\n6\n"),
        ("conflict.txt", "a\nb\nc\n"),
    ]);
    let source = repo.commit_files(&[
        ("eol.txt", Some("one\nTWO\nthree\n")),
        ("merge.txt", Some("1\n2\n3\n4\n5\nSIX\n")),
        ("conflict.txt", Some("a\nX\nc\n")),
    ]);
    // A Windows checkout that only differs by line endings, an independent
    // destination edit, and an edit to the same line.
    std::fs::write(repo.root.join("eol.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
    std::fs::write(repo.root.join("merge.txt"), "ONE\n2\n3\n4\n5\n6\n").unwrap();
    std::fs::write(repo.root.join("conflict.txt"), "a\nB\nc\n").unwrap();
    let job = repo.job(vec![source]);
    let eol = catalog_change(&repo, &job, "eol.txt");
    assert_eq!((eol["status"].as_str(), eol["merge"].as_str()), (Some("clean"), Some("source_lines")), "{eol:#}");
    let merged = catalog_change(&repo, &job, "merge.txt");
    assert_eq!((merged["status"].as_str(), merged["merge"].as_str()), (Some("clean"), Some("line_merge")), "{merged:#}");
    assert!(merged["reason"].as_str().unwrap().contains("different lines"));
    let conflict = catalog_change(&repo, &job, "conflict.txt");
    assert_eq!(conflict["status"], "conflict");
    assert!(conflict["reason"].as_str().unwrap().contains("files.set_text"));
    assert_eq!(conflict["hunks"][0]["base_lines"], json!([2, 3]));

    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let blocked = preview(&repo.root, &job.id).unwrap();
    assert_eq!(blocked.issues.len(), 1, "{:#?}", blocked.issues);
    assert_eq!(blocked.issues[0]["code"], "text_conflict");
    assert_eq!(blocked.issues[0]["change_id"], conflict["id"]);
    assert_eq!(blocked.issues[0]["hunks"][0]["source"], "X\n");
    let marked = execute(&repo.root, &job.id, "inspect_asset", json!({"path":"conflict.txt","version":"result"})).unwrap();
    assert_eq!(marked["conflicted"], true);
    assert_eq!(
        marked["text"],
        "a\n<<<<<<< destination\nB\n||||||| base\nb\n=======\nX\n>>>>>>> source\nc\n"
    );
    assert!(execute(&repo.root, &job.id, "files.set_text", json!({"path":"conflict.txt","content":marked["text"]}))
        .unwrap_err()
        .contains("conflict marker"));
    execute(&repo.root, &job.id, "files.set_text", json!({"path":"conflict.txt","content":"a\nB\nX\nc\n"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    assert_eq!(std::fs::read(repo.root.join("eol.txt")).unwrap(), b"one\r\nTWO\r\nthree\r\n");
    assert_eq!(std::fs::read(repo.root.join("merge.txt")).unwrap(), b"ONE\n2\n3\n4\n5\nSIX\n");
    assert_eq!(std::fs::read(repo.root.join("conflict.txt")).unwrap(), b"a\nB\nX\nc\n");
    // Supplied text is part of the journal: abort restores it with the rest.
    execute(&repo.root, &job.id, "abort", json!({})).unwrap();
    assert_eq!(std::fs::read(repo.root.join("conflict.txt")).unwrap(), b"a\nB\nc\n");
    assert_eq!(std::fs::read(repo.root.join("eol.txt")).unwrap(), b"one\r\ntwo\r\nthree\r\n");
}

#[test]
fn text_deletions_require_a_side_and_set_text_is_validated() {
    let repo = repo_with(&[("gone.txt", "keep\n"), ("crlf.txt", "x\ny\n")]);
    let source = repo.commit_files(&[("gone.txt", None), ("crlf.txt", Some("x\nY\n"))]);
    std::fs::write(repo.root.join("gone.txt"), "keep\nlocal\n").unwrap();
    std::fs::write(repo.root.join("crlf.txt"), "x\r\nz\r\n").unwrap();
    let job = repo.job(vec![source]);
    let gone = catalog_change(&repo, &job, "gone.txt");
    assert_eq!(gone["status"], "conflict");
    assert!(gone["reason"].as_str().unwrap().contains("source deletes this file"));
    execute(&repo.root, &job.id, "include", json!({"path":"gone.txt"})).unwrap();
    let blocked = preview(&repo.root, &job.id).unwrap();
    assert!(blocked.issues[0]["detail"].as_str().unwrap().contains("source deletes"));
    execute(&repo.root, &job.id, "resolve", json!({"path":"gone.txt","side":"source"})).unwrap();
    let deleted = preview(&repo.root, &job.id).unwrap();
    assert!(deleted.ready_to_apply, "{:?}", deleted.issues);
    assert_eq!(deleted.files.get("gone.txt"), Some(&None));

    for (params, error) in [
        (json!({"path":"asset.asset","content":"x"}), "has no change"),
        (json!({"path":"crlf.txt","content":"x\n<<<<<<< destination\n"}), "conflict marker"),
        (json!({"path":"crlf.txt","content":"x","eol":"auto"}), "eol must be"),
    ] {
        let message = execute(&repo.root, &job.id, "files.set_text", params).unwrap_err();
        assert!(message.contains(error), "{message}");
    }
    // Agent-written LF text follows the destination's CRLF convention.
    execute(&repo.root, &job.id, "files.set_text", json!({"path":"crlf.txt","content":"x\nY\nz\n"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let state = plan.files["crlf.txt"].clone().unwrap();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(read_blob(&dir, &state).unwrap(), b"x\r\nY\r\nz\r\n");
    execute(&repo.root, &job.id, "files.set_text", json!({"path":"crlf.txt","content":"x\nY\n","eol":"exact"})).unwrap();
    let exact = preview(&repo.root, &job.id).unwrap();
    assert_eq!(read_blob(&dir, exact.files["crlf.txt"].as_ref().unwrap()).unwrap(), b"x\nY\n");
}

#[test]
fn set_text_refuses_unity_yaml() {
    let repo = Repo::new();
    let source = repo.commit("asset.asset", yaml(120, 5, "base"));
    let job = repo.job(vec![source]);
    let message = execute(&repo.root, &job.id, "files.set_text", json!({"path":"asset.asset","content":"x"})).unwrap_err();
    assert!(message.contains("Unity YAML"), "{message}");
}

#[test]
fn field_operation_refines_a_selected_change_and_overlaps_name_their_change_ids() {
    let repo = Repo::new();
    let list = |values: &[u32]| {
        let items: String = values.iter().map(|v| format!("  - {v}\n")).collect();
        format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_Name: base\n  values:\n{items}")
    };
    std::fs::write(repo.root.join("list.asset"), list(&[1, 1, 2])).unwrap();
    git(&repo.root, &["add", "list.asset"]).unwrap();
    git(&repo.root, &["commit", "-m", "list"]).unwrap();
    git(&repo.source, &["merge", "-q", "--ff-only", "main"]).unwrap();
    let source = repo.commit("list.asset", list(&[9, 1, 2]));
    std::fs::write(repo.root.join("list.asset"), list(&[1, 1, 2, 5])).unwrap();
    let job = repo.job(vec![source]);
    let change = execute(&repo.root, &job.id, "changes", json!({})).unwrap()["changes"][0].clone();
    assert_eq!(change["property_path"], "/MonoBehaviour/values");
    assert_eq!(change["status"], "conflict");
    assert!(change["reason"].as_str().unwrap().contains("no Unity identity"));
    execute(&repo.root, &job.id, "resolve", json!({"change_ids":[change["id"]],"side":"source"})).unwrap();
    // Refine the selected source list: delete one of its elements.
    execute(&repo.root, &job.id, "fields.delete", json!({"path":"list.asset","object_id":"11400000","property_path":"/MonoBehaviour/values/1"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(
        String::from_utf8(read_blob(&dir, plan.files["list.asset"].as_ref().unwrap()).unwrap()).unwrap(),
        list(&[9, 2])
    );
    // An operation on an enclosing node is ambiguous and names the change.
    execute(&repo.root, &job.id, "fields.set", json!({"path":"list.asset","object_id":"11400000","property_path":"/MonoBehaviour","value":{"m_Name":"x","values":[]}})).unwrap();
    let overlap = preview(&repo.root, &job.id).unwrap();
    let issue = overlap.issues.iter().find(|i| i["code"] == "overlapping_selection").unwrap();
    assert!(issue["overlaps"].as_array().unwrap().iter().any(|o| o["change_id"] == change["id"]), "{issue:#}");
}

#[test]
fn editor_sync_reimports_assets_and_leaves_scripts_to_compilation() {
    let repo = Repo::new();
    let mut job = repo.job(vec![repo.commit("notes.txt", "selected\n")]);
    let state = |blob: &str| Some(FileState { blob: blob.into(), mode: "100644".into() });
    job.applied_files = BTreeMap::from([
        ("Assets/Scene.unity".to_string(), state("after")),
        ("Assets/Scene.unity.meta".to_string(), state("after")),
        ("Assets/Code/Motor.cs".to_string(), state("after")),
        ("Assets/Code/Motor.cs.meta".to_string(), state("after")),
        ("Assets/Unchanged.asset".to_string(), state("same")),
        ("notes.txt".to_string(), state("after")),
    ]);
    job.applied_before = job
        .applied_files
        .keys()
        .map(|path| (path.clone(), if path.ends_with("Unchanged.asset") { state("same") } else { state("before") }))
        .collect();
    let (assets, scripts) = editor_sync_paths(&job).unwrap();
    assert_eq!(assets, ["Assets/Scene.unity"]);
    assert_eq!(scripts, ["Assets/Code/Motor.cs"]);
}

#[tokio::test]
async fn editor_sync_is_skipped_for_editor_transactions_and_non_unity_roots() {
    let repo = Repo::new();
    let job = repo.job(vec![repo.commit("notes.txt", "selected\n")]);
    let editor = json!({"state":"applied","editor_applied":true});
    assert_eq!(sync_editor_after_apply(&repo.root, &job.id, editor.clone()).await, editor);
    let direct = sync_editor_after_apply(&repo.root, &job.id, json!({"state":"applied","editor_applied":false})).await;
    assert_eq!(direct["editor_sync"]["status"], "not_unity_project");
    assert!(direct.get("warnings").is_none());
}

fn override_item(target: &str, property: &str, value: &str) -> String {
    format!("    - target: {{fileID: {target}, guid: cba0fbbfd3cdbce499475af4af43a2e3,\n        type: 3}}\n      propertyPath: {property}\n      value: {value}\n      objectReference: {{fileID: 0}}\n")
}

fn instance(id: &str, overrides: &[(&str, &str, &str)]) -> String {
    let items: String = overrides.iter().map(|(t, p, v)| override_item(t, p, v)).collect();
    format!("--- !u!1001 &{id}\nPrefabInstance:\n  m_ObjectHideFlags: 0\n  serializedVersion: 2\n  m_Modification:\n    serializedVersion: 3\n    m_TransformParent: {{fileID: 0}}\n    m_Modifications:\n{items}    m_RemovedComponents: []\n  m_SourcePrefab: {{fileID: 100100000, guid: cba0fbbfd3cdbce499475af4af43a2e3, type: 3}}\n")
}

fn scene(enemy: &[(&str, &str, &str)], player: &[(&str, &str, &str)]) -> String {
    format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n{}{}", instance("1050404689", enemy), instance("2040113738", player))
}

/// The 白盒-7 integration distilled: Prefab override lists changed on both
/// sides, code edited on both sides, a CRLF-only working file and a deleted
/// document all merge through the plan with decisions only for real conflicts.
#[test]
fn session_regression_integrates_overrides_text_and_line_endings_through_the_plan() {
    const ROOT: &str = "600249992675133040";
    const NAME: &str = "1937218400633567911";
    const VIEW: &str = "7468865217963383891";
    const PLAYER: &str = "8726720174468721065";
    let name = "\"C1010_\\u9B54\\u65CF\\u58EB\\u5175_01\"";
    let base_enemy = [
        (ROOT, "m_LocalPosition.x", "-126.74875"),
        (ROOT, "m_LocalRotation.x", "-0"),
        (ROOT, "m_LocalRotation.z", "-0"),
        (NAME, "m_Name", name),
        (VIEW, "legacyRenderers.Array.size", "0"),
    ];
    let base_player = [
        (PLAYER, "m_LocalPosition.x", "-108.739"),
        (PLAYER, "m_LocalPosition.y", "55.691"),
        (VIEW, "legacyRenderers.Array.size", "0"),
    ];
    let code = "using System;\n\nclass Motor\n{\n    void Step() { }\n\n    void Land() { }\n}\n";
    let replay = "class Replay\n{\n    void Seek() { }\n}\n";
    let repo = repo_with(&[
        ("Assets/Scenes/Entities.unity", &scene(&base_enemy, &base_player)),
        ("Assets/Code/Motor.cs", code),
        ("Assets/Code/Replay.cs", replay),
        ("Docs/Design.md", "# Squads\n"),
    ]);
    let source = repo.commit_files(&[
        (
            "Assets/Scenes/Entities.unity",
            Some(&scene(
                &[(ROOT, "m_LocalPosition.x", "-127.19"), (NAME, "m_Name", name), (VIEW, "legacyRenderers.Array.size", "0")],
                &[(PLAYER, "m_LocalPosition.x", "-108.42"), (PLAYER, "m_LocalPosition.y", "48.188004"), (VIEW, "legacyRenderers.Array.size", "0")],
            )),
        ),
        ("Assets/Code/Motor.cs", Some(&code.replace("void Land() { }", "void Land() { Validate(); }"))),
        ("Assets/Code/Replay.cs", Some(&replay.replace("void Seek() { }", "void Seek() { Reset(); }"))),
        ("Docs/Design.md", None),
    ]);
    // Destination: removed the legacy overrides, moved the player, edited
    // another Motor method, and holds Replay.cs with CRLF line endings.
    let destination_scene = scene(
        &base_enemy[..4],
        &[(PLAYER, "m_LocalPosition.x", "-144.65"), (PLAYER, "m_LocalPosition.y", "36")],
    );
    std::fs::write(repo.root.join("Assets/Scenes/Entities.unity"), &destination_scene).unwrap();
    std::fs::write(repo.root.join("Assets/Code/Motor.cs"), code.replace("using System;", "using System;\nusing System.Linq;")).unwrap();
    std::fs::write(repo.root.join("Assets/Code/Replay.cs"), replay.replace('\n', "\r\n")).unwrap();
    let job = repo.job(vec![source]);
    let changes = execute(&repo.root, &job.id, "changes", json!({"limit":1000})).unwrap();
    let conflicts: Vec<&Value> = changes["changes"].as_array().unwrap().iter().filter(|c| c["status"] == "conflict").collect();
    // Only the two player position values were changed differently.
    assert_eq!(conflicts.len(), 2, "{conflicts:#?}");
    assert!(conflicts.iter().all(|c| c["property_path"].as_str().unwrap().contains(":m_LocalPosition.") && c["reason"].is_string()));
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let blocked = preview(&repo.root, &job.id).unwrap();
    let issue = blocked.issues.iter().find(|i| i["code"] == "asset_conflict").unwrap();
    assert_eq!(issue["change_ids"].as_array().unwrap().len(), 2, "{issue:#}");
    execute(&repo.root, &job.id, "resolve", json!({"change_ids":issue["change_ids"],"side":"target"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let applied = execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    assert_eq!(applied["editor_applied"], false);
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Scenes/Entities.unity")).unwrap(),
        scene(
            &[(ROOT, "m_LocalPosition.x", "-127.19"), (NAME, "m_Name", name)],
            &[(PLAYER, "m_LocalPosition.x", "-144.65"), (PLAYER, "m_LocalPosition.y", "36")],
        )
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Code/Motor.cs")).unwrap(),
        code.replace("using System;", "using System;\nusing System.Linq;").replace("void Land() { }", "void Land() { Validate(); }")
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Assets/Code/Replay.cs")).unwrap(),
        replay.replace("void Seek() { }", "void Seek() { Reset(); }").replace('\n', "\r\n")
    );
    assert!(!repo.root.join("Docs/Design.md").exists());
}

/// Scoped preparation lists only the requested files from Git; their names
/// are literal paths even when they contain pattern characters.
#[test]
fn scoped_prepare_reads_paths_with_pattern_characters_literally() {
    let repo = repo_with(&[("Assets/odd [1].txt", "base\n"), ("Assets/odd 1.txt", "other\n")]);
    let source = repo.commit_files(&[("Assets/odd [1].txt", Some("source\n")), ("Assets/odd 1.txt", Some("changed\n"))]);
    let job = prepare(
        &repo.root,
        &PrepareRequest {
            sources: vec![SourceSelection { commits: vec![source], ..Default::default() }],
            paths: Some(vec!["Assets/odd [1].txt".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    // The asset and its paired meta enter the scope; the similarly named file does not.
    let paths: Vec<_> = job.deltas.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["Assets/odd [1].txt", "Assets/odd [1].txt.meta"]);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(state_bytes(&dir, &job.deltas[0].base).unwrap(), b"base\n");
    assert_eq!(state_bytes(&dir, &job.deltas[0].source).unwrap(), b"source\n");
}

/// An app-managed file rewritten after the snapshot makes apply stale. Replan
/// keeps the source commits pinned (a moved branch adds nothing), carries the
/// decisions of unchanged files and reports the changed file's decision.
#[test]
fn replan_carries_decisions_of_unchanged_files_and_reports_the_rest() {
    let repo = repo_with(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("state.json", "{\"revision\":1}\n")]);
    let source = repo.commit_files(&[
        ("a.txt", Some("A\n")),
        ("b.txt", Some("B\n")),
        ("state.json", Some("{\"revision\":2}\n")),
    ]);
    let job = repo.job(vec![source.clone()]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    execute(&repo.root, &job.id, "files.set_text", json!({"path":"b.txt","content":"merged b\n"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);

    std::fs::write(repo.root.join("state.json"), "{\"revision\":7}\n").unwrap();
    let stale = execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap_err();
    assert!(stale.starts_with("stale: state.json changed after the snapshot"), "{stale}");
    repo.commit_files(&[("later.txt", Some("not selected\n"))]);

    let (replanned, report) = replan(&repo.root, &job.id).unwrap();
    assert_eq!(report["replaced_job_id"], job.id.as_str());
    assert_eq!(report["carried"]["files"], 1);
    assert_eq!(report["dropped_total"], 1, "{report:#}");
    assert_eq!(report["dropped"][0]["path"], "state.json");
    assert_eq!(report["dropped"][0]["reason"], "destination_changed");
    assert_eq!(report["changed_paths"], json!(["state.json"]));
    assert!(replanned.deltas.iter().all(|delta| delta.commit == source));
    assert!(load(&repo.root, &job.id).unwrap_err().contains("no longer exists"));

    let plan = preview(&repo.root, &replanned.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(&repo.root, &replanned.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    assert_eq!(std::fs::read_to_string(repo.root.join("a.txt")).unwrap(), "A\n");
    assert_eq!(std::fs::read_to_string(repo.root.join("b.txt")).unwrap(), "merged b\n");
    assert_eq!(std::fs::read_to_string(repo.root.join("state.json")).unwrap(), "{\"revision\":7}\n");
    assert!(!repo.root.join("later.txt").exists());
    assert!(replan(&repo.root, &replanned.id).unwrap_err().starts_with("Cannot replan: job is applied"));
}

fn scene_text(docs: &[(u64, &str)]) -> String {
    let mut text = String::from("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n");
    for (id, name) in docs {
        text.push_str(&format!("--- !u!1 &{id}\nGameObject:\n  m_Name: {name}\n"));
    }
    text
}

/// A scene the destination never changed takes the source file byte for byte,
/// even where re-rendering would keep the destination's document order.
#[test]
fn untouched_destination_takes_the_source_scene_byte_for_byte() {
    let base = scene_text(&[(100, "a"), (300, "c"), (500, "e")]);
    let repo = repo_with(&[("Assets/Level.unity", &base)]);
    // Additions between existing objects, and one existing object listed earlier.
    let source = scene_text(&[(100, "a"), (200, "b"), (500, "e"), (300, "C"), (400, "d")]);
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&source))]);
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap(), source.as_bytes());
}

/// With destination changes the file is rendered; source additions still sit
/// next to their source neighbours rather than at the end of the scene.
#[test]
fn changed_destination_places_source_additions_beside_their_neighbours() {
    let base = scene_text(&[(100, "a"), (300, "c"), (500, "e")]);
    let repo = repo_with(&[("Assets/Level.unity", &base)]);
    let source = scene_text(&[(100, "a"), (200, "b"), (300, "C"), (400, "d"), (500, "e")]);
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&source))]);
    std::fs::write(repo.root.join("Assets/Level.unity"), scene_text(&[(100, "A"), (300, "c"), (500, "e")])).unwrap();
    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let merged = String::from_utf8(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap()).unwrap();
    assert_eq!(merged, scene_text(&[(100, "A"), (200, "b"), (300, "C"), (400, "d"), (500, "e")]));
}

/// The summary rolls the catalog up per file and the change filters return
/// only what needs a decision, while totals still describe the whole catalog.
#[test]
fn catalog_summary_and_filters_point_at_the_changes_that_need_a_decision() {
    let base = scene_text(&[(100, "a"), (300, "c")]);
    let repo = repo_with(&[("Assets/Quiet.unity", &base), ("Assets/Busy.unity", &base)]);
    let source = scene_text(&[(100, "A"), (200, "b"), (300, "C")]);
    let commit = repo.commit_files(&[("Assets/Quiet.unity", Some(&source)), ("Assets/Busy.unity", Some(&source))]);
    std::fs::write(repo.root.join("Assets/Busy.unity"), scene_text(&[(100, "X"), (300, "c")])).unwrap();
    let job = repo.job(vec![commit]);

    let summary = execute(&repo.root, &job.id, "change_summary", json!({})).unwrap();
    let files = summary["files"].as_array().unwrap();
    assert_eq!(files.iter().map(|f| f["path"].as_str().unwrap()).collect::<Vec<_>>(), ["Assets/Busy.unity", "Assets/Quiet.unity"]);
    let (busy, quiet) = (&files[0], &files[1]);
    assert_eq!(busy["destination_changed"], true);
    assert_eq!(quiet["destination_changed"], false);
    assert_eq!(busy["statuses"]["conflict"], 1, "{busy}");
    assert!(quiet["statuses"].get("conflict").is_none(), "{quiet}");
    assert_eq!(quiet["kinds"]["add_object"], 1, "{quiet}");
    assert_eq!(summary["file_total"], 2);
    assert_eq!(summary["changes"], busy["changes"].as_u64().unwrap() + quiet["changes"].as_u64().unwrap());
    assert!(summary["next_offset"].is_null());
    let second = execute(&repo.root, &job.id, "change_summary", json!({"offset":1,"limit":1})).unwrap();
    assert_eq!(second["files"][0]["path"], "Assets/Quiet.unity");

    let conflicts = execute(&repo.root, &job.id, "changes", json!({"status":"conflict"})).unwrap();
    assert_eq!(conflicts["total"], 1);
    assert_eq!(conflicts["catalog_total"], summary["changes"]);
    assert_eq!(conflicts["changes"][0]["path"], "Assets/Busy.unity");
    assert!(conflicts["changes"][0]["reason"].as_str().is_some_and(|r| !r.is_empty()));
    let quiet_changes = execute(&repo.root, &job.id, "changes", json!({"paths":["Assets/Quiet.unity"],"status":["clean","already_applied"]})).unwrap();
    assert_eq!(quiet_changes["total"], quiet["changes"]);
    let error = execute(&repo.root, &job.id, "changes", json!({"status":"conflicts"})).unwrap_err();
    assert!(error.contains("Unknown change status `conflicts`"), "{error}");
    assert!(execute(&repo.root, &job.id, "changes", json!({"paths":"Assets"})).unwrap()["total"] == 0);
}

/// Selecting consecutive commits of one scene: the later commit edits an
/// object the earlier one added, while the destination edited another object.
#[test]
fn consecutive_commits_edit_what_the_earlier_commit_added() {
    let base = scene_text(&[(100, "a"), (300, "c")]);
    let repo = repo_with(&[("Assets/Level.unity", &base)]);
    let first = repo.commit_files(&[("Assets/Level.unity", Some(&scene_text(&[(100, "a"), (200, "b"), (300, "c")])))]);
    let second = repo.commit_files(&[("Assets/Level.unity", Some(&scene_text(&[(100, "a"), (200, "B"), (300, "C")])))]);
    std::fs::write(repo.root.join("Assets/Level.unity"), scene_text(&[(100, "A"), (300, "c")])).unwrap();
    let job = repo.job(vec![first.clone(), second.clone()]);
    let conflicts = execute(&repo.root, &job.id, "changes", json!({"status":"conflict"})).unwrap();
    assert_eq!(conflicts["total"], 0, "{conflicts}");
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let merged = String::from_utf8(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap()).unwrap();
    assert_eq!(merged, scene_text(&[(100, "A"), (200, "B"), (300, "C")]));

    // Leaving out the commit that added the object: the later edit cannot
    // apply as listed, and preview says why instead of failing to render.
    execute(&repo.root, &job.id, "new_plan", json!({})).unwrap();
    execute(&repo.root, &job.id, "include", json!({"commit":second})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let issue = plan.issues.iter().find(|i| i["code"] == "earlier_commit_decisions").unwrap_or_else(|| panic!("{:?}", plan.issues));
    assert_eq!(issue["earlier_commits"], json!([first]));
    assert!(!issue["change_ids"].as_array().unwrap().is_empty());
    assert!(plan.issues.iter().all(|i| i["code"] != "asset_render"), "{:?}", plan.issues);
}

/// Consecutive commits of one text file: the later commit edits lines the
/// earlier one wrote, which merges cleanly once the earlier one is included.
#[test]
fn consecutive_text_commits_are_cataloged_against_the_earlier_commit() {
    let repo = repo_with(&[("Assets/notes.md", "one
two
three
four
five
")]);
    let first = repo.commit_files(&[("Assets/notes.md", Some("one
TWO
three
four
five
"))]);
    let second = repo.commit_files(&[("Assets/notes.md", Some("one
TWO!
three
four
five
"))]);
    std::fs::write(repo.root.join("Assets/notes.md"), "one
two
three
four
FIVE
").unwrap();
    let job = repo.job(vec![first, second]);
    let conflicts = execute(&repo.root, &job.id, "changes", json!({"status":"conflict"})).unwrap();
    assert_eq!(conflicts["total"], 0, "{conflicts}");
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(state_bytes(&dir, &plan.files["Assets/notes.md"]).unwrap(), b"one
TWO!
three
four
FIVE
");
}

/// GameObject 100 with Transform 400 and, if `size` is set, BoxCollider 500.
fn collider_scene(size: Option<u32>) -> String {
    let listed = if size.is_some() { "  - component: {fileID: 500}\n" } else { "" };
    let mut text = format!(
        "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!1 &100\nGameObject:\n  m_Component:\n  - component: {{fileID: 400}}\n{listed}  m_Name: A\n\
         --- !u!4 &400\nTransform:\n  m_GameObject: {{fileID: 100}}\n  m_Children: []\n  m_Father: {{fileID: 0}}\n"
    );
    if let Some(size) = size {
        text.push_str(&format!("--- !u!65 &500\nBoxCollider:\n  m_GameObject: {{fileID: 100}}\n  m_Size: {{x: {size}, y: 1, z: 1}}\n"));
    }
    text
}

/// A validation finding names an object rather than a change; the issue
/// lists the catalog changes of that object so the agent has an ID to act on.
#[test]
fn validation_conflicts_name_the_changes_of_the_reported_object() {
    let repo = repo_with(&[("Assets/Level.unity", &collider_scene(Some(1)))]);
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&collider_scene(Some(3))))]);
    std::fs::write(repo.root.join("Assets/Level.unity"), collider_scene(None)).unwrap();
    let job = repo.job(vec![commit]);
    let conflicts = execute(&repo.root, &job.id, "changes", json!({"status":"conflict"})).unwrap();
    let restored = conflicts["changes"].as_array().unwrap().iter()
        .find(|c| c["object_id"] == "500").unwrap_or_else(|| panic!("{conflicts}"))["id"].clone();
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    execute(&repo.root, &job.id, "resolve", json!({"change_ids":[restored],"side":"source"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    let issue = plan.issues.iter().find(|i| i["code"] == "asset_conflict").unwrap_or_else(|| panic!("{:?}", plan.issues));
    assert!(issue["change_ids"].as_array().unwrap().contains(&restored), "{issue}");
    assert!(issue["conflicts"].as_array().unwrap().iter().any(|c| c["related_change_ids"].as_array().is_some_and(|ids| ids.contains(&restored))), "{issue}");

    // The conflict's reason names the restore that brings its listing along.
    let reason = conflicts["changes"].as_array().unwrap().iter().find(|c| c["id"] == restored).unwrap()["reason"].as_str().unwrap().to_string();
    assert!(reason.contains("objects.restore(path, object_id='500')"), "{reason}");
    let outcome = execute(&repo.root, &job.id, "objects.restore", json!({"path":"Assets/Level.unity","object_id":"500"})).unwrap();
    assert_eq!(outcome["restored"]["attached"], json!([{"object_id":"100","property_path":"/GameObject/m_Component/@component=500"}]));
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(String::from_utf8(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap()).unwrap(), collider_scene(Some(3)));
}

fn journal_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn job_dirs(repo: &Repo) -> Vec<String> {
    journal_entries(&journal_root(&repo.root).unwrap()).into_iter().filter(|n| uuid::Uuid::parse_str(n).is_ok()).collect()
}

fn job_refs(repo: &Repo) -> String {
    git_text(&repo.root, &["for-each-ref", "--format=%(refname)", "refs/locus/merge-jobs/"]).unwrap()
}

fn backdate(path: &Path, days: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 3600))
        .unwrap();
}

/// The large parts of a job are written once; a plan change rewrites only the
/// small record, which stores a file whose changes share one decision once.
#[test]
fn plan_changes_rewrite_only_the_job_record() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 6, "source"));
    let job = repo.job(vec![commit]);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let names = journal_entries(&dir);
    let prefixes: Vec<&str> = names.iter().map(|n| n.rsplit_once('-').map_or(n.as_str(), |(p, _)| p)).collect();
    assert_eq!(prefixes, ["deltas", "dependencies", "files", "index-flags", "index-records", "job.json", "schemas"], "{names:?}");
    let stamps = |names: &[String]| -> Vec<std::time::SystemTime> {
        names.iter().filter(|n| *n != "job.json").map(|n| std::fs::metadata(dir.join(n)).unwrap().modified().unwrap()).collect()
    };
    let before = stamps(&names);
    std::thread::sleep(std::time::Duration::from_millis(20));
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let after_names: Vec<String> = journal_entries(&dir).into_iter().filter(|n| n != "preview-latest.json").collect();
    assert_eq!(after_names, names);
    assert_eq!(stamps(&after_names), before);
    let record: Value = serde_json::from_slice(&std::fs::read(dir.join("job.json")).unwrap()).unwrap();
    let decisions = &record["selection"]["decisions"];
    assert_eq!(decisions["files"].as_array().unwrap().len(), 1, "{decisions}");
    assert!(decisions["changes"].as_object().unwrap().is_empty(), "{decisions}");
    let loaded = load(&repo.root, &job.id).unwrap();
    let changes = loaded.deltas.iter().map(|d| d.changes.len()).sum::<usize>();
    assert!(changes > 1);
    assert_eq!(loaded.selection.decisions.len(), changes);
    assert!(preview(&repo.root, &job.id).unwrap().ready_to_apply);
}

/// A failed prepare, an abort and a commit each leave no job directory, ref
/// or blob behind.
#[test]
fn ended_jobs_leave_no_journal_files() {
    let repo = Repo::new();
    let commit = repo.commit("notes.txt", "source notes\n");
    let failed = prepare(&repo.root, &PrepareRequest {
        sources: vec![SourceSelection { commits: vec![commit.clone()], ..Default::default() }],
        paths: Some(vec!["absent.txt".into()]),
        ..Default::default()
    })
    .unwrap_err();
    assert!(failed.contains("absent"), "{failed}");
    assert!(job_dirs(&repo).is_empty());
    assert!(job_refs(&repo).is_empty());

    let aborted = repo.job(vec![commit.clone()]);
    assert_eq!(execute(&repo.root, &aborted.id, "abort", json!({})).unwrap()["removed"], true);
    assert!(job_dirs(&repo).is_empty());
    assert!(job_refs(&repo).is_empty());
    assert_eq!(execute(&repo.root, &aborted.id, "abort", json!({})).unwrap()["state"], "removed");
    let gone = load(&repo.root, &aborted.id).unwrap_err();
    assert!(gone.contains("no longer exists"), "{gone}");

    let job = repo.job(vec![commit]);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    execute(&repo.root, &job.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    let committed = execute(&repo.root, &job.id, "commit", json!({"paths":["notes.txt"],"message":"Take source notes"})).unwrap();
    assert_eq!(committed["removed"], true);
    assert_eq!(git_text(&repo.root, &["show", "HEAD:notes.txt"]).unwrap(), "source notes");
    assert!(job_dirs(&repo).is_empty());
    assert!(job_refs(&repo).is_empty());
    let journal = journal_root(&repo.root).unwrap();
    assert!(journal_entries(&journal.join(BLOB_STORE)).is_empty(), "{:?}", journal_entries(&journal.join(BLOB_STORE)));
    assert!(journal_entries(&journal).iter().all(|n| !n.starts_with("trash-")));
}

/// An applied job stays usable until its destination moves on; a prepared
/// job expires when unused; a job needing recovery is never removed.
#[test]
fn unfinished_jobs_expire_by_their_own_rules() {
    let repo = Repo::new();
    let commit = repo.commit("notes.txt", "source notes\n");
    let applied = repo.job(vec![commit.clone()]);
    execute(&repo.root, &applied.id, "include", json!({"all":true})).unwrap();
    let plan = preview(&repo.root, &applied.id).unwrap();
    execute(&repo.root, &applied.id, "apply", json!({"expected_plan_hash":plan.plan_hash})).unwrap();
    let prepared = repo.job(vec![commit.clone()]);
    assert!(job_dirs(&repo).contains(&applied.id), "a new prepare keeps an applied job whose destination is unchanged");

    git(&repo.root, &["commit", "-qam", "user commits the merge"]).unwrap();
    let next = repo.job(vec![repo.commit("asset.asset", yaml(103, 5, "later"))]);
    let live = job_dirs(&repo);
    assert!(!live.contains(&applied.id), "HEAD moved: the applied job is finished");
    assert!(live.contains(&prepared.id) && live.contains(&next.id));
    assert!(next.prepare_metrics["phases"]["prune_ms"].is_number());

    backdate(&job_dir(&repo.root, &prepared.id).unwrap().join("job.json"), 4);
    let dir = job_dir(&repo.root, &next.id).unwrap();
    let mut recovering = load(&repo.root, &next.id).unwrap();
    recovering.state = "recovery_required".into();
    save(&dir, &recovering).unwrap();
    backdate(&dir.join("job.json"), 30);
    repo.job(vec![commit]);
    let live = job_dirs(&repo);
    assert!(!live.contains(&prepared.id), "unused for three days");
    assert!(live.contains(&next.id), "recovery data is kept");
    assert!(job_refs(&repo).contains(&next.id));
}

/// A full snapshot freezes raw bytes only for writable targets. Clean Unity
/// files stay Git-backed inputs: readable, promoted when a plan writes one,
/// and still stale when they change.
#[test]
fn full_snapshots_keep_clean_inputs_in_git() {
    let meta = "fileFormatVersion: 2\nguid: 0123456789abcdef0123456789abcdef\n";
    let repo = repo_with(&[("Assets/Other.prefab", &scene_text(&[(1, "o")])), ("Assets/Other.prefab.meta", meta)]);
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    std::fs::write(repo.root.join("notes.txt"), "dirty notes\n").unwrap();
    let job = repo.job(vec![commit]);
    assert!(job.snapshot.files.contains_key("asset.asset"));
    assert!(job.snapshot.files.contains_key("notes.txt"));
    assert!(!job.snapshot.files.contains_key("Assets/Other.prefab"));
    assert!(git_blob_oid(job.dependency_files["Assets/Other.prefab"].as_ref().unwrap()).is_some());
    let view = execute(&repo.root, &job.id, "inspect_asset", json!({"path":"Assets/Other.prefab"})).unwrap();
    assert_eq!(view["kind"], "unity_yaml");

    execute(&repo.root, &job.id, "files.delete", json!({"path":"Assets/Other.prefab"})).unwrap();
    let promoted = load(&repo.root, &job.id).unwrap();
    let state = promoted.snapshot.files["Assets/Other.prefab"].as_ref().unwrap();
    assert!(git_blob_oid(state).is_none());

    std::fs::write(repo.root.join("Assets/Other.prefab.meta"), meta.replace("0123", "fedc")).unwrap();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let error = check_target(&repo.root, &dir, &promoted).unwrap_err();
    assert!(error.contains("stale"), "{error}");
    let error = execute(&repo.root, &job.id, "files.delete", json!({"path":"Assets/Other.prefab.meta"})).unwrap_err();
    assert!(error.contains("stale"), "{error}");
}


/// Kept root K and root R > child C > grandchild D with a collider.
fn restore_level(r: bool, c: bool, collider: Option<u32>) -> String {
    let object = |id: i64, name: &str, father: i64, children: &[i64], collider: Option<u32>| {
        let mut text = format!("--- !u!1 &{id}\nGameObject:\n  m_Component:\n  - component: {{fileID: {}}}\n", id + 1);
        if collider.is_some() {
            text.push_str(&format!("  - component: {{fileID: {}}}\n", id + 2));
        }
        text.push_str(&format!("  m_Name: {name}\n--- !u!4 &{}\nTransform:\n  m_GameObject: {{fileID: {id}}}\n", id + 1));
        if children.is_empty() {
            text.push_str("  m_Children: []\n");
        } else {
            text.push_str("  m_Children:\n");
            for child in children {
                text.push_str(&format!("  - {{fileID: {child}}}\n"));
            }
        }
        text.push_str(&format!("  m_Father: {{fileID: {father}}}\n"));
        if let Some(size) = collider {
            text.push_str(&format!("--- !u!65 &{}\nBoxCollider:\n  m_GameObject: {{fileID: {id}}}\n  m_Size: {{x: {size}, y: 1, z: 1}}\n", id + 2));
        }
        text
    };
    let mut text = String::from("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n");
    if r {
        text.push_str(&object(100, "R", 0, if c { &[201] } else { &[] }, None));
    }
    if c {
        text.push_str(&object(200, "C", 101, &[301], None));
        text.push_str(&object(300, "D", 201, &[], collider));
    }
    text.push_str(&object(1000, "K", 0, &[], None));
    text.push_str("--- !u!1660057539 &9223372036854775807\nSceneRoots:\n  m_Roots:\n");
    for root in if r { vec![101, 1001] } else { vec![1001] } {
        text.push_str(&format!("  - {{fileID: {root}}}\n"));
    }
    text
}

/// The destination deleted a root subtree while the source edited a
/// component deep inside it. One restore brings the subtree back with its
/// scene root entry, and the result is exactly the source scene.
#[test]
fn restoring_a_deleted_subtree_is_one_operation() {
    let repo = repo_with(&[("Assets/Level.unity", &restore_level(true, true, Some(1)))]);
    let source = restore_level(true, true, Some(3));
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&source))]);
    std::fs::write(repo.root.join("Assets/Level.unity"), restore_level(false, false, None)).unwrap();
    let job = repo.job(vec![commit]);
    let conflicts = execute(&repo.root, &job.id, "changes", json!({"status":"conflict"})).unwrap();
    assert_eq!(conflicts["total"], 1, "{conflicts}");
    let conflict = &conflicts["changes"][0];
    assert_eq!(conflict["object_id"], "302");
    assert!(conflict["reason"].as_str().unwrap().contains("objects.restore(path, object_id='302')"));

    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    let outcome = execute(&repo.root, &job.id, "objects.restore", json!({"path":"Assets/Level.unity","object_id":"302"})).unwrap();
    let restored = &outcome["restored"];
    assert_eq!(restored["objects"], json!(["100", "101", "200", "201", "300", "301", "302"]));
    assert_eq!(restored["resolved_change_ids"], json!([conflict["id"]]));
    assert_eq!(restored["attached"], json!([{"object_id":"9223372036854775807","property_path":"/SceneRoots/m_Roots/@root=101"}]));
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(String::from_utf8(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap()).unwrap(), source);

    // Keeping it deleted instead stays one decision.
    execute(&repo.root, &job.id, "new_plan", json!({})).unwrap();
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    execute(&repo.root, &job.id, "resolve", json!({"change_ids":[conflict["id"]],"side":"target"})).unwrap();
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    assert!(!plan.files.contains_key("Assets/Level.unity"), "the destination keeps its file");
}

/// The destination deleted a child subtree under a kept parent; the restore
/// stops at that parent and adds the child to its m_Children.
#[test]
fn restoring_a_deleted_child_subtree_attaches_it_to_the_kept_parent() {
    let repo = repo_with(&[("Assets/Level.unity", &restore_level(true, true, Some(1)))]);
    let source = restore_level(true, true, Some(3));
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&source))]);
    std::fs::write(repo.root.join("Assets/Level.unity"), restore_level(true, false, None)).unwrap();
    let job = repo.job(vec![commit]);
    let outcome = execute(&repo.root, &job.id, "objects.restore", json!({"path":"Assets/Level.unity","object_id":"302"})).unwrap();
    assert_eq!(outcome["restored"]["attached"], json!([{"object_id":"101","property_path":"/Transform/m_Children/@child=201"}]));
    let plan = preview(&repo.root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(String::from_utf8(state_bytes(&dir, &plan.files["Assets/Level.unity"]).unwrap()).unwrap(), source);
    let error = execute(&repo.root, &job.id, "objects.restore", json!({"path":"Assets/Level.unity","object_id":"1000"})).unwrap_err();
    assert!(error.contains("still has object 1000"), "{error}");
}

/// Only files the Editor serializes can hold unsaved Editor changes, so only
/// they make an abort wait for the Editor to close.
#[test]
fn abort_waits_for_the_editor_only_for_files_it_serializes() {
    for path in ["Assets/Level.unity", "Assets/A.prefab", "Assets/Data.asset", "Assets/Tex.png.meta", "Assets/M.mat"] {
        assert!(editor_serialized(path), "{path}");
    }
    for path in [".locus-sdk-merge.txt", "Assets/Code.cs", "Assets/Game.asmdef", "Packages/manifest.json", "README.md"] {
        assert!(!editor_serialized(path), "{path}");
    }
}
