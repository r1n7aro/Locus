use super::*;

fn scoped(repo: &Repo, commit: String, path: &str, mode: PrepareMode) -> MergeJob {
    prepare(
        &repo.root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![commit],
                ..Default::default()
            }],
            paths: Some(vec![path.into()]),
            mode,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn file_scope_skips_unrelated_binary_and_keeps_raw_target_and_index() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(123, 5, "source"));
    let target = yaml(100, 5, "dirty").replace('\n', "\r\n");
    std::fs::write(repo.root.join("asset.asset"), &target).unwrap();
    std::fs::write(repo.root.join("unrelated.glb"), vec![7u8; 4 * 1024 * 1024]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "staged\n").unwrap();
    git(&repo.root, &["add", "notes.txt"]).unwrap();
    std::fs::write(repo.root.join("notes.txt"), "unstaged\n").unwrap();
    let before = index_entries(&repo.root).unwrap();
    let job = scoped(&repo, commit.clone(), "asset.asset", PrepareMode::Files);
    assert_eq!(job.snapshot.files.len(), 2);
    assert!(!job.dependency_files.contains_key("unrelated.glb"));
    assert!(job.schemas.is_empty());
    assert!(job.catalog_ready);
    assert!(execute(
        &repo.root,
        &job.id,
        "files.delete",
        json!({"path":"notes.txt"})
    )
    .unwrap_err()
    .contains("scope"));
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(
        state_bytes(&dir, &job.snapshot.files["asset.asset"]).unwrap(),
        target.as_bytes()
    );
    execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"asset.asset","version":"source","commit":commit}),
    )
    .unwrap();
    let preview = preview(&repo.root, &job.id).unwrap();
    assert!(preview.ready_to_apply, "{:?}", preview.issues);
    execute(
        &repo.root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":preview.plan_hash}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.root.join("asset.asset")).unwrap(),
        yaml(123, 5, "source")
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("notes.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(index_entries(&repo.root).unwrap(), before);
    assert_eq!(
        std::fs::metadata(repo.root.join("unrelated.glb"))
            .unwrap()
            .len(),
        4 * 1024 * 1024
    );
    assert!(execute(
        &repo.root,
        &job.id,
        "files.delete",
        json!({"path":"notes.txt"})
    )
    .is_err());
}

#[test]
fn scoped_clean_and_dirty_reverse_references_remain_frozen() {
    for dirty in [false, true] {
        let repo = repo_with_external_prefab_reference(EXTERNAL_PARENT_GUID);
        if dirty {
            // This addition is absent from Git and must be retained as raw evidence.
            std::fs::write(
                repo.root.join("dirty.asset"),
                external_scene_reference(EXTERNAL_PARENT_GUID, 3),
            )
            .unwrap();
        }
        let commit = repo.commit("Assets/Parent.prefab", externally_referenced_prefab(false));
        let job = scoped(
            &repo,
            commit.clone(),
            "Assets/Parent.prefab",
            PrepareMode::Files,
        );
        let dir = job_dir(&repo.root, &job.id).unwrap();
        if dirty {
            assert!(git_blob_oid(job.dependency_files["dirty.asset"].as_ref().unwrap()).is_none());
        }
        assert!(job
            .dependency_files
            .values()
            .flatten()
            .any(|s| git_blob_oid(s).is_some()));
        execute(
            &repo.root,
            &job.id,
            "files.take",
            json!({"path":"Assets/Parent.prefab","version":"source","commit":commit}),
        )
        .unwrap();
        let first = preview(&repo.root, &job.id).unwrap();
        assert!(
            first
                .issues
                .iter()
                .any(|v| v["code"] == "removed_object_dependency"),
            "{:?}",
            first.issues
        );
        // Rewriting live referrers cannot change a preview's frozen dependency proof.
        for (path, state) in &job.dependency_files {
            if path.ends_with(".unity") || path.ends_with(".asset") {
                let expected = state_bytes(&dir, state).unwrap();
                std::fs::write(repo.root.join(path), yaml(1, 2, "later")).unwrap();
                assert_eq!(state_bytes(&dir, state).unwrap(), expected);
            }
        }
        let second = preview(&repo.root, &job.id).unwrap();
        assert_eq!(first.plan_hash, second.plan_hash);
        assert!(check_target(&repo.root, &dir, &job)
            .unwrap_err()
            .contains("stale"));
    }
}

#[test]
fn scoped_guid_removal_checks_unchanged_git_asset() {
    let repo = Repo::new();
    let guid = "1234567890abcdef1234567890abcdef";
    std::fs::write(
        repo.root.join("observer.asset"),
        external_scene_reference(guid, 11400000),
    )
    .unwrap();
    std::fs::write(
        repo.root.join("asset.asset.meta"),
        format!("guid: {guid}\n"),
    )
    .unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "references"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let commit = repo.commit(
        "asset.asset.meta",
        "guid: 99999999999999999999999999999999\n",
    );
    let job = scoped(
        &repo,
        commit.clone(),
        "asset.asset.meta",
        PrepareMode::Files,
    );
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
            .any(|v| v["code"] == "missing_guid_dependency" && v["path"] == "observer.asset"),
        "{:?}",
        plan.issues
    );
}

#[test]
fn shared_blobs_are_reused_and_stale_verification_does_not_write() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let first = scoped(&repo, commit.clone(), "asset.asset", PrepareMode::Files);
    let folder = journal_root(&repo.root).unwrap().join("blobs-v2");
    let count = std::fs::read_dir(&folder).unwrap().count();
    let second = scoped(&repo, commit, "asset.asset", PrepareMode::Files);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), count);
    assert_eq!(first.snapshot.files, second.snapshot.files);
    assert!(!job_dir(&repo.root, &second.id)
        .unwrap()
        .join("blobs")
        .exists());
    std::fs::write(repo.root.join("asset.asset"), yaml(999, 5, "later")).unwrap();
    assert!(check_target(
        &repo.root,
        &job_dir(&repo.root, &second.id).unwrap(),
        &second
    )
    .unwrap_err()
    .contains("stale"));
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), count);
}

#[test]
fn lazy_catalog_and_summary_are_available_without_full_payload() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let job = scoped(&repo, commit, "asset.asset", PrepareMode::Structural);
    assert!(!job.catalog_ready);
    assert!(job.schemas.is_empty());
    assert!(job.deltas.iter().all(|d| d.changes.is_empty()));
    let summary = execute(&repo.root, &job.id, "get", json!({})).unwrap();
    assert!(summary["snapshot"].get("index_records").is_none());
    assert!(summary["snapshot"].get("files").is_none());
    assert!(serde_json::to_vec(&summary).unwrap().len() < 4096);
    let page = execute(&repo.root, &job.id, "snapshot", json!({"limit":1})).unwrap();
    assert_eq!(page["files"].as_object().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    execute(&repo.root, &job.id, "new_plan", json!({})).unwrap();
    assert!(!load(&repo.root, &job.id).unwrap().catalog_ready);
    let changes = execute(&repo.root, &job.id, "changes", json!({})).unwrap();
    assert!(!changes["changes"].as_array().unwrap().is_empty());
    let ready = load(&repo.root, &job.id).unwrap();
    assert!(ready.catalog_ready);
    assert_eq!(ready.revision, 1);
    execute(&repo.root, &job.id, "include", json!({"all":true})).unwrap();
    assert!(preview(&repo.root, &job.id).unwrap().ready_to_apply);
}

#[test]
fn file_scope_rejects_empty_ambiguous_and_outside_paths() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    for paths in [
        None,
        Some(vec![]),
        Some(vec!["../escape.asset".into()]),
        Some(vec![".".into()]),
        Some(vec!["*.asset".into()]),
    ] {
        assert!(prepare(
            &repo.root,
            &PrepareRequest {
                sources: vec![SourceSelection {
                    commits: vec![commit.clone()],
                    ..Default::default()
                }],
                paths,
                mode: PrepareMode::Files,
                ..Default::default()
            }
        )
        .is_err());
    }
}

#[test]
fn scoped_alias_evidence_uses_dirty_script_overlay() {
    let repo = Repo::new();
    let guid = "11112222333344445555666677778888";
    let script =
        "using UnityEngine; public class Example : ScriptableObject { public int legacyValue; }";
    let renamed="using UnityEngine; using UnityEngine.Serialization; public class Example : ScriptableObject { [FormerlySerializedAs(\"legacyValue\")] public int currentValue; }";
    let asset = |field: &str, value: u32| {
        format!("--- !u!114 &11400000\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n  {field}: {value}\n")
    };
    std::fs::write(repo.root.join("Example.cs"), script).unwrap();
    std::fs::write(repo.root.join("Example.cs.meta"), format!("guid: {guid}\n")).unwrap();
    std::fs::write(repo.root.join("asset.asset"), asset("legacyValue", 10)).unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "schema baseline"]).unwrap();
    git(&repo.source, &["merge", "--ff-only", "main"]).unwrap();
    let commit = repo.commit("asset.asset", asset("legacyValue", 20));
    std::fs::write(repo.root.join("Example.cs"), renamed).unwrap();
    std::fs::write(repo.root.join("asset.asset"), asset("currentValue", 10)).unwrap();
    let job = scoped(&repo, commit, "asset.asset", PrepareMode::Structural);
    assert!(job.schemas.is_empty());
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
        asset("currentValue", 20)
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("Example.cs")).unwrap(),
        renamed
    );
}

#[test]
fn custom_filter_clean_dependency_keeps_real_worktree_bytes() {
    let repo = Repo::new();
    git(
        &repo.root,
        &[
            "config",
            "filter.merge-fixture.clean",
            "printf filtered-placeholder",
        ],
    )
    .unwrap();
    std::fs::write(
        repo.root.join(".gitattributes"),
        "observer.asset filter=merge-fixture\n",
    )
    .unwrap();
    let content = external_scene_reference(EXTERNAL_PARENT_GUID, 3);
    std::fs::write(repo.root.join("observer.asset"), &content).unwrap();
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "filtered dependency"]).unwrap();
    assert!(
        snapshot::names(&repo.root, &["diff", "--name-only", "-z", "HEAD", "--"])
            .unwrap()
            .is_empty()
    );
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let job = scoped(&repo, commit, "asset.asset", PrepareMode::Files);
    let state = job.dependency_files["observer.asset"].as_ref().unwrap();
    assert!(git_blob_oid(state).is_none());
    let dir = job_dir(&repo.root, &job.id).unwrap();
    assert_eq!(read_blob(&dir, state).unwrap(), content.as_bytes());
    // Changing a filter while Git still considers a file clean cannot bypass stale checks.
    git(
        &repo.root,
        &[
            "config",
            "filter.merge-fixture.clean",
            "printf another-placeholder",
        ],
    )
    .unwrap();
    assert!(check_target(&repo.root, &dir, &job)
        .unwrap_err()
        .contains("configuration"));
}

#[test]
fn legacy_journal_still_reads_its_local_blobs() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let job = repo.job(vec![commit]);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let mut legacy = job.clone();
    legacy.id = uuid::Uuid::new_v4().to_string();
    legacy.version = 1;
    let old = job_dir(&repo.root, &legacy.id).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    for state in job.snapshot.files.values().flatten().chain(
        job.deltas
            .iter()
            .flat_map(|d| d.base.iter().chain(&d.source)),
    ) {
        assert_eq!(
            store_blob(&old, &read_blob(&dir, state).unwrap(), &state.mode).unwrap(),
            *state
        );
    }
    let mut value = serde_json::to_value(&legacy).unwrap();
    for field in [
        "paths",
        "mode",
        "dependency_files",
        "catalog_ready",
        "prepare_metrics",
    ] {
        value.as_object_mut().unwrap().remove(field);
    }
    atomic_json(&old.join("job.json"), &value).unwrap();
    let loaded = load(&repo.root, &legacy.id).unwrap();
    assert!(loaded.catalog_ready);
    assert_eq!(loaded.version, 1);
    let view = execute(
        &repo.root,
        &legacy.id,
        "inspect_asset",
        json!({"path":"asset.asset"}),
    )
    .unwrap();
    assert_eq!(view["kind"], "unity_yaml");
    execute(&repo.root, &legacy.id, "include", json!({"all":true})).unwrap();
    assert!(preview(&repo.root, &legacy.id).unwrap().ready_to_apply);
}

#[test]
fn absent_source_version_cannot_implicitly_delete_a_target_only_path() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    std::fs::write(repo.root.join("local-only.txt"), "keep local file").unwrap();
    let job = scoped(&repo, commit.clone(), "local-only.txt", PrepareMode::Files);
    let error = execute(
        &repo.root,
        &job.id,
        "files.take",
        json!({"path":"local-only.txt","version":"source","commit":commit}),
    )
    .unwrap_err();
    assert!(error.contains("No source file version"), "{error}");
    assert!(load(&repo.root, &job.id)
        .unwrap()
        .selection
        .files
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(repo.root.join("local-only.txt")).unwrap(),
        "keep local file"
    );
    execute(
        &repo.root,
        &job.id,
        "files.delete",
        json!({"path":"local-only.txt"}),
    )
    .unwrap();
    assert!(preview(&repo.root, &job.id).unwrap().ready_to_apply);
}

#[test]
fn scoped_commit_does_not_reuse_validation_of_excluded_dirty_code() {
    let repo = Repo::new();
    std::fs::write(repo.root.join("Example.cs"), "public class Example {}\n").unwrap();
    git(&repo.root, &["add", "Example.cs"]).unwrap();
    git(&repo.root, &["commit", "-m", "baseline code"]).unwrap();
    std::fs::write(
        repo.root.join("Example.cs"),
        "public class Example { public int Changed; }\n",
    )
    .unwrap();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let mut job = scoped(&repo, commit, "asset.asset", PrepareMode::Files);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    job.applied_files.insert(
        "asset.asset".into(),
        job.deltas
            .iter()
            .find(|d| d.path == "asset.asset")
            .unwrap()
            .source
            .clone(),
    );
    let error =
        validate_commit_scope(&repo.root, &dir, &job, &job.applied_files, true).unwrap_err();
    assert!(error.contains("needs_commit_validation"), "{error}");
}

#[test]
fn parallel_publication_never_exposes_partial_shared_blobs() {
    use rayon::prelude::*;
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let job = scoped(&repo, commit, "asset.asset", PrepareMode::Files);
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let bytes = vec![42u8; 1024 * 1024];
    parallel::pool().install(|| {
        (0..128).into_par_iter().for_each(|_| {
            let state = store_blob(&dir, &bytes, "100644").unwrap();
            assert_eq!(read_blob(&dir, &state).unwrap(), bytes);
        })
    });
}

#[test]
#[ignore = "manual performance baseline; prints measurements without timing assertions"]
fn prepare_performance_baseline() {
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("Assets")).unwrap();
    for i in 0..1200 {
        std::fs::write(
            repo.root.join(format!("Assets/asset-{i}.asset")),
            yaml(i, 5, "baseline"),
        )
        .unwrap();
        std::fs::write(
            repo.root.join(format!("Assets/asset-{i}.asset.meta")),
            format!("guid: {i:032x}\n"),
        )
        .unwrap();
    }
    git(&repo.root, &["add", "."]).unwrap();
    git(&repo.root, &["commit", "-m", "benchmark assets"]).unwrap();
    std::fs::write(
        repo.root.join("large-unrelated.glb"),
        vec![7u8; 32 * 1024 * 1024],
    )
    .unwrap();
    let commit = repo.commit("asset.asset", yaml(123, 5, "source"));
    for (label, paths, mode) in [
        (
            "scoped-first",
            Some(vec!["asset.asset".into()]),
            PrepareMode::Files,
        ),
        (
            "scoped-repeat",
            Some(vec!["asset.asset".into()]),
            PrepareMode::Files,
        ),
        ("full-first", None, PrepareMode::Structural),
        ("full-repeat", None, PrepareMode::Structural),
    ] {
        let start = Instant::now();
        let job = prepare(
            &repo.root,
            &PrepareRequest {
                sources: vec![SourceSelection {
                    commits: vec![commit.clone()],
                    ..Default::default()
                }],
                paths,
                mode,
                ..Default::default()
            },
        )
        .unwrap();
        let prepare_ms = start.elapsed().as_millis();
        let folder = journal_root(&repo.root).unwrap().join("blobs-v2");
        let size: u64 = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        std::println!(
            "MERGE_BENCH {}",
            json!({"label":label,"prepare_ms":prepare_ms,"metrics":job.prepare_metrics,"shared_blob_bytes":size,"response_bytes":serde_json::to_vec(&summary(&job)).unwrap().len()})
        );
    }
}
