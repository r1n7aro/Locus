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
    let folder = journal_root(&repo.root).unwrap().join(BLOB_STORE);
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

/// A job written by an earlier journal version is not read; the next
/// prepare removes it with the stores only earlier versions used.
#[test]
fn earlier_journal_versions_are_rejected_and_removed() {
    let repo = Repo::new();
    let commit = repo.commit("asset.asset", yaml(111, 5, "source"));
    let journal = journal_root(&repo.root).unwrap();
    let old = uuid::Uuid::new_v4().to_string();
    let recovering = uuid::Uuid::new_v4().to_string();
    for (id, state) in [(&old, "prepared"), (&recovering, "recovery_required")] {
        std::fs::create_dir_all(journal.join(id)).unwrap();
        std::fs::write(journal.join(id).join("job.json"), format!(r#"{{"version":2,"state":"{state}","deltas":[]}}"#)).unwrap();
    }
    for store in ["blobs-v2", "git-blobs-v2", "reference-cache-1-objects-v2"] {
        std::fs::create_dir_all(journal.join(store)).unwrap();
        std::fs::write(journal.join(store).join("entry"), b"x").unwrap();
    }
    let error = load(&repo.root, &old).unwrap_err();
    assert!(error.contains("earlier Locus version"), "{error}");
    repo.job(vec![commit]);
    assert!(!journal.join(&old).exists());
    assert!(!journal.join("reference-cache-1-objects-v2").exists());
    // A job an earlier version left mid-recovery keeps its data.
    assert!(journal.join(&recovering).join("job.json").is_file());
    assert!(journal.join("blobs-v2").exists());
    std::fs::remove_dir_all(journal.join(&recovering)).unwrap();
    repo.job(vec![repo.commit("notes.txt", "next\n")]);
    assert!(!journal.join("blobs-v2").exists() && !journal.join("git-blobs-v2").exists());
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
        let folder = journal_root(&repo.root).unwrap().join(BLOB_STORE);
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

/// Prefixes the plain `m_Name` values at the given line offsets.
fn renamed(text: &str, offsets: &[usize], prefix: &str) -> String {
    let mut out = String::with_capacity(text.len() + offsets.len() * prefix.len());
    let mut cursor = 0;
    for &at in offsets {
        out.push_str(&text[cursor..at]);
        out.push_str(prefix);
        cursor = at;
    }
    out.push_str(&text[cursor..]);
    out
}

/// What one merge of a real asset costs the agent, end to end. Set
/// LOCUS_MERGE_BENCH_FILES to `;`-separated Unity YAML paths (read only; the
/// bytes are copied into a temporary repository). The destination renames the
/// first object, the source renames a middle and the last one, so the plan is
/// clean and every phase does real work on the whole file.
#[test]
#[ignore = "manual benchmark over real assets; prints MERGE_REAL_BENCH lines"]
fn real_asset_merge_benchmark() {
    let Ok(list) = std::env::var("LOCUS_MERGE_BENCH_FILES") else {
        eprintln!("LOCUS_MERGE_BENCH_FILES is not set");
        return;
    };
    for file in list.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let text = String::from_utf8(std::fs::read(file).unwrap()).unwrap();
        let marker = "\n  m_Name: ";
        // Plain names get the prefix in front; Unity's double-quoted
        // (`\u`-escaped) names get it inside the quotes.
        let names: Vec<usize> = text
            .match_indices(marker)
            .map(|(at, _)| at + marker.len())
            .filter_map(|at| match text[at..].chars().next() {
                Some('"') => Some(at + 1),
                Some(c) if c.is_ascii_alphanumeric() => Some(at),
                _ => None,
            })
            .collect();
        assert!(names.len() >= 3, "{file}: needs three m_Name values");
        let target = renamed(&text, &names[..1], "Target");
        let source = renamed(&text, &[names[names.len() / 2], names[names.len() - 1]], "Source");
        let name = Path::new(file).file_name().unwrap().to_string_lossy().into_owned();
        let path = format!("Assets/{name}");
        let repo = repo_with(&[(&path, &text)]);
        let commit = repo.commit_files(&[(&path, Some(&source))]);
        std::fs::write(repo.root.join(&path), &target).unwrap();

        let timed = |f: &mut dyn FnMut()| {
            let start = Instant::now();
            f();
            start.elapsed().as_millis()
        };
        let mut session = None;
        let core_prepare = timed(&mut || {
            session = Some(crate::unity_asset_core::prepare_merge(text.as_bytes(), target.as_bytes(), source.as_bytes()).unwrap())
        });
        let session = session.unwrap();
        let decisions: Vec<_> = session
            .catalog()
            .changes
            .iter()
            .map(|c| crate::unity_asset_core::Decision { change_id: c.id.clone(), resolution: crate::unity_asset_core::Resolution::Include })
            .collect();
        let mut rendered = None;
        let core_render = timed(&mut || rendered = Some(session.render(&decisions).unwrap()));
        let rendered = rendered.unwrap();
        assert!(rendered.ready, "{file}: {:?}", rendered.conflicts);

        let mut job = None;
        let prepare_ms = timed(&mut || {
            job = Some(
                prepare(
                    &repo.root,
                    &PrepareRequest {
                        sources: vec![SourceSelection { commits: vec![commit.clone()], ..Default::default() }],
                        paths: Some(vec![path.clone()]),
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
        });
        let job = job.unwrap();
        let mut bytes = 0usize;
        let mut call = |action: &str, params: Value| {
            let start = Instant::now();
            let value = execute(&repo.root, &job.id, action, params).unwrap();
            bytes += value.to_string().len();
            (start.elapsed().as_millis(), value)
        };
        let (changes_ms, changes) = call("changes", json!({"limit":1000}));
        let (include_ms, _) = call("include", json!({"path":path}));
        let (preview_ms, preview) = call("preview", json!({}));
        let (repeat_preview_ms, _) = call("preview", json!({}));
        assert_eq!(preview["ready_to_apply"], true, "{file}: {}", preview["issues"]);
        let (apply_ms, _) = call("apply", json!({"expected_plan_hash":preview["plan_hash"]}));
        let merged = std::fs::read_to_string(repo.root.join(&path)).unwrap();
        assert!(merged == renamed(&source, &names[..1], "Target"), "{file}: merged bytes differ from the expected union");
        std::println!(
            "MERGE_REAL_BENCH {}",
            json!({"file":name,"bytes":text.len(),"documents":text.matches("\n--- !u!").count() + 1,
                "changes":changes["total"],"core_prepare_ms":core_prepare,"core_render_ms":core_render,
                "prepare_ms":prepare_ms,"prepare_phases":job.prepare_metrics,"changes_ms":changes_ms,"include_ms":include_ms,
                "preview_ms":preview_ms,"repeat_preview_ms":repeat_preview_ms,"apply_ms":apply_ms,"response_bytes":bytes})
        );
    }
}

/// Journal cost of one large merge: a full snapshot of a project with many
/// clean Unity files and a scene whose every object changed. Prints the cost
/// of each agent call and the bytes a plan change writes.
#[test]
#[ignore = "manual journal benchmark; prints JOURNAL_BENCH"]
fn journal_cost_of_a_large_catalog() {
    let objects: i64 = std::env::var("LOCUS_JOURNAL_BENCH_OBJECTS").ok().and_then(|v| v.parse().ok()).unwrap_or(12_000);
    let clean_files = 4_000;
    let scene = |suffix: &str| {
        let mut text = String::from("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n");
        for id in 1..=objects {
            text.push_str(&format!("--- !u!1 &{id}\nGameObject:\n  m_Name: Object{id}{suffix}\n  m_IsActive: 1\n"));
        }
        text
    };
    let mut files: Vec<(String, String)> = (0..clean_files)
        .map(|i| (format!("Assets/Clean/Asset{i}.asset.meta"), format!("fileFormatVersion: 2\nguid: {:032x}\n", i + 1)))
        .collect();
    files.push(("Assets/Level.unity".into(), scene("")));
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let repo = repo_with(&borrowed);
    let commit = repo.commit_files(&[("Assets/Level.unity", Some(&scene("Renamed")))]);
    let start = Instant::now();
    let job = repo.job(vec![commit]);
    let prepare_ms = start.elapsed().as_millis();
    let dir = job_dir(&repo.root, &job.id).unwrap();
    let sizes = |dir: &Path| -> BTreeMap<String, u64> {
        std::fs::read_dir(dir).unwrap().flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len())).collect()
    };
    let blobs = std::fs::read_dir(journal_root(&repo.root).unwrap().join(BLOB_STORE)).unwrap().count();
    let mut calls = serde_json::Map::new();
    let mut time = |label: &str, action: &str, params: Value| {
        let start = Instant::now();
        let value = execute(&repo.root, &job.id, action, params).unwrap();
        calls.insert(label.into(), json!({"ms": start.elapsed().as_millis(), "response_bytes": value.to_string().len()}));
    };
    time("change_summary", "change_summary", json!({}));
    time("conflicts", "changes", json!({"status":"conflict"}));
    time("include_all", "include", json!({"all":true}));
    time("exclude_one_file", "exclude", json!({"path":"Assets/Level.unity"}));
    time("include_again", "include", json!({"path":"Assets/Level.unity"}));
    time("preview", "preview", json!({}));
    time("repeat_preview", "preview", json!({}));
    let load_start = Instant::now();
    let loaded = load(&repo.root, &job.id).unwrap();
    let cached_load_ms = load_start.elapsed().as_micros() as f64 / 1000.0;
    // The single-file journal every call used to parse, and every plan change to rewrite.
    let whole = serde_json::to_vec(&(&loaded, &*loaded.deltas, &*loaded.snapshot.files, &*loaded.snapshot.index_records,
        &*loaded.snapshot.index_flags, &*loaded.dependency_files, &loaded.selection.decisions)).unwrap();
    let old_start = Instant::now();
    let parsed: Value = serde_json::from_slice(&whole).unwrap();
    let old_parse_ms = old_start.elapsed().as_millis();
    let old_start = Instant::now();
    atomic_bytes(&dir.join("old-format.tmp"), &serde_json::to_vec(&parsed).unwrap()).unwrap();
    let old_write_ms = old_start.elapsed().as_millis();
    std::fs::remove_file(dir.join("old-format.tmp")).unwrap();
    std::println!(
        "JOURNAL_BENCH {}",
        json!({"objects":objects,"clean_files":clean_files,"changes":job.deltas.iter().map(|d| d.changes.len()).sum::<usize>(),
            "prepare_ms":prepare_ms,"prepare_phases":job.prepare_metrics["phases"],"snapshot_targets":job.snapshot.files.len(),
            "dependency_inputs":job.dependency_files.len(),"blob_files":blobs,"journal_files":sizes(&dir),
            "calls":calls,"cached_load_ms":cached_load_ms,
            "single_file_journal":{"bytes":whole.len(),"parse_ms":old_parse_ms,"write_ms":old_write_ms}})
    );
}
