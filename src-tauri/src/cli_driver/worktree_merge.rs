//! End-to-end acceptance suites. Fixture writes are confined to owned linked
//! worktrees and uniquely named Unity asset folders; source index/HEAD stay put.
use super::*;
use crate::workspace_service::worktrees::{self, CreateWorktreeRequest};

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = crate::process_util::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn check(condition: bool, detail: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(detail.to_string())
    }
}

pub(super) async fn run_worktrees(
    app: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let source = resolve_project_path(config.project_path.as_deref(), app).await?;
    let source_root = Path::new(&source);
    let before_index = git(source_root, &["ls-files", "--stage", "-z"])?;
    let before_head = git(source_root, &["rev-parse", "HEAD"])?;
    let before_diff = git(source_root, &["diff", "--binary", "HEAD", "--"])?;
    let mut fixture_base = String::from_utf8(before_head.clone())
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();
    let excluded_package = std::env::var("LOCUS_WORKTREE_TEST_EXCLUDE_PATH").ok();
    if let Some(path) = &excluded_package {
        check(
            !path.is_empty()
                && !Path::new(path).is_absolute()
                && !path.contains([':', '\\'])
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && !path.split('/').any(|c| c.eq_ignore_ascii_case(".git")),
            "Unsafe test-only exclusion path",
        )?;
        let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let index = temp.path().join("index");
        indexed_git(source_root, &index, &["read-tree", &fixture_base], None)?;
        indexed_git(
            source_root,
            &index,
            &[
                "rm",
                "-r",
                "--cached",
                "--ignore-unmatch",
                "--",
                path,
                &format!("{path}.meta"),
            ],
            None,
        )?;
        fixture_base = finish_commit(
            source_root,
            &index,
            &fixture_base,
            "Locus acceptance baseline: exclude unavailable optional tooling package",
        )?;
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    let directory_base = std::env::var_os("LOCUS_WORKTREE_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            source_root
                .parent()
                .unwrap_or(source_root)
                .join(".locus-driver-worktrees")
        });
    check(
        directory_base.is_absolute(),
        "Worktree test root must be absolute",
    )?;
    let directory = directory_base.join(&token);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    sink.emit("suite_start", json!({"suite":"worktrees", "source":source, "artifacts":directory,"fixtureBase":fixture_base,"excludedOptionalPackage":excluded_package}));
    let version_override = std::env::var("LOCUS_WORKTREE_TEST_UNITY_VERSION").ok();
    if let Some(version) = &version_override {
        check(
            version.starts_with("6000.5.") && !version.contains(['\r', '\n']),
            "Test override must identify a Unity 6.5 Editor",
        )?;
    }
    let mut roots = Vec::new();
    let mut records = Vec::new();
    for suffix in ["a", "b"] {
        let request = CreateWorktreeRequest {
            source_root: roots.first().cloned().unwrap_or_else(|| source.clone()),
            destination: directory.join(suffix).to_string_lossy().into_owned(),
            branch: format!("codex/locus-driver-{token}-{suffix}"),
            start_ref: if roots.is_empty() {
                Some(fixture_base.clone())
            } else {
                None
            },
            include_dirty: suffix == "b",
        };
        let record = tokio::task::spawn_blocking(move || worktrees::create(&request))
            .await
            .map_err(|e| e.to_string())??;
        if let Some(version) = &version_override {
            std::fs::write(
                Path::new(&record.root).join("ProjectSettings/ProjectVersion.txt"),
                format!("m_EditorVersion: {version}\n"),
            )
            .map_err(|e| e.to_string())?;
        }
        if suffix == "a" {
            std::fs::write(
                Path::new(&record.root).join(".locus-worktree-driver-isolation.txt"),
                "seed-local\n",
            )
            .map_err(|e| e.to_string())?;
        }
        sink.emit("worktree_created", &record);
        roots.push(record.root.clone());
        records.push(record);
    }
    check(
        records[0].project_id == records[1].project_id,
        "Sibling worktrees changed logical project identity",
    )?;
    check(
        records[0].checkout_id != records[1].checkout_id,
        "Sibling worktrees share physical identity",
    )?;
    let marker = ".locus-worktree-driver-isolation.txt";
    check(
        std::fs::read(Path::new(&roots[1]).join(marker)).map_err(|e| e.to_string())?
            == b"seed-local\n",
        "Local source changes were not transferred to the sibling worktree",
    )?;
    std::fs::write(Path::new(&roots[0]).join(marker), "checkout-a\n").map_err(|e| e.to_string())?;
    check(
        std::fs::read(Path::new(&roots[1]).join(marker)).map_err(|e| e.to_string())?
            == b"seed-local\n",
        "Writes leaked into sibling checkout",
    )?;
    std::fs::write(
        directory.join("worktrees.json"),
        serde_json::to_vec_pretty(&records).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut nested = config.clone();
    nested.project_path = Some(roots[0].clone());
    nested.workspace_paths = vec![roots[1].clone()];
    nested.suites = vec![CliDriverSuite::Workspace];
    nested.install_plugin = true;
    run_workspace_suite(app, &nested, sink, cancel).await?;
    check(
        before_index == git(source_root, &["ls-files", "--stage", "-z"])?,
        "Source staged records changed",
    )?;
    check(
        before_head == git(source_root, &["rev-parse", "HEAD"])?,
        "Source HEAD changed",
    )?;
    check(
        before_diff == git(source_root, &["diff", "--binary", "HEAD", "--"])?,
        "Source dirty tracked bytes changed",
    )?;
    sink.emit(
        "suite_result",
        json!({"suite":"worktrees", "passed":6,"failed":0,"checkouts":records,"artifacts":directory,
        "sourceIndexPreserved":true,"sourceHeadPreserved":true,"sourceDirtyPreserved":true}),
    );
    Ok(())
}

pub(super) async fn run_asset_merge(
    app: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let (_idle_cancel_sender, mut idle_cancel) = watch::channel(false);
    super::wait_for_unity_editor_idle(project, config, sink, &mut idle_cancel).await?;
    use crate::merge_jobs::{self, PrepareRequest, SourceSelection};
    use crate::unity_asset_core::{self, Decision, Resolution};
    let root = Path::new(project);
    let token = uuid::Uuid::new_v4().simple().to_string();
    let folder = format!("Assets/LocusMergeDriver-{token}");
    sink.emit(
        "suite_start",
        json!({"suite":"asset-merge","project":project,"fixture":folder}),
    );
    let output = execute_capture(
        project,
        &format!(
            "print(Locus.MergeTesting.LocusMergeFixtureApi.Generate({}));",
            json!(folder)
        ),
    )
    .await?;
    let generated = capture_json(&output)?;
    let version = generated["unityVersion"]
        .as_str()
        .ok_or("Fixture omitted Unity version")?;
    check(
        version.starts_with("6000.5."),
        "Asset merge acceptance requires Unity 6.5",
    )?;
    let snapshots = Path::new(
        generated["snapshots"]
            .as_str()
            .ok_or("Fixture omitted snapshot directory")?,
    );
    let read = |name: &str| std::fs::read(snapshots.join(name)).map_err(|e| e.to_string());
    let graph = format!("{folder}/Graph.asset");
    let prefab = format!("{folder}/Parent.prefab");
    let mut prefab_ids = Vec::new();
    for name in [
        "prefab-base.yaml",
        "prefab-target.yaml",
        "prefab-source.yaml",
    ] {
        let parsed = unity_asset_core::parse(&read(name)?).map_err(|e| e.to_string())?;
        let mut ids: Vec<_> = parsed
            .documents
            .iter()
            .map(|d| d.object_id.clone())
            .collect();
        ids.sort();
        prefab_ids.push(ids);
    }
    check(
        prefab_ids[0] == prefab_ids[1] && prefab_ids[1] == prefab_ids[2],
        "Prefab fixture did not retain stable Unity file IDs across source/target saves",
    )?;
    let binary = format!("{folder}/Opaque.bytes");
    let skipped = format!("{folder}/Skipped.txt");
    let unrelated = format!("{folder}/Local.txt");
    std::fs::write(root.join(&binary), [0, 1, 0, 255]).map_err(|e| e.to_string())?;
    std::fs::write(root.join(&unrelated), "staged-local\n").map_err(|e| e.to_string())?;
    // Materialize the two assets' real Unity .meta files before freezing the
    // source snapshot, independent of the Editor's background refresh timing.
    execute_capture(project, "UnityEditor.AssetDatabase.Refresh(UnityEditor.ImportAssetOptions.ForceSynchronousImport); print(\"MERGE_FIXTURE_IMPORTED\");").await?;
    // Only our fixture enters the real index. It deliberately remains staged
    // while the destination has additional unstaged edits to the same asset.
    git(root, &["add", "--", &graph, &unrelated])?;
    let baseline = snapshot_commit(
        root,
        &[
            &folder,
            "Packages/com.farlocus.locus",
            "ProjectSettings/ProjectVersion.txt",
        ],
        "Locus merge driver fixture base",
    )?;
    let c1 = patch_commit(
        root,
        &baseline,
        &[
            (graph.clone(), read("graph-source.yaml")?),
            (binary.clone(), vec![0, 2, 0, 255]),
        ],
        "selected graph and binary",
    )?;
    let c2 = patch_commit(
        root,
        &c1,
        &[(skipped.clone(), b"must not be integrated\n".to_vec())],
        "intentionally omitted commit",
    )?;
    let c3 = patch_commit(
        root,
        &c2,
        &[(prefab.clone(), read("prefab-source.yaml")?)],
        "selected prefab",
    )?;
    let conflict_commit = patch_commit(
        root,
        &baseline,
        &[(graph.clone(), read("graph-conflict.yaml")?)],
        "conflicting managed reference",
    )?;
    git(
        root,
        &[
            "update-ref",
            &format!("refs/locus/merge-driver/{token}"),
            &c3,
        ],
    )?;
    std::fs::write(root.join(&graph), read("graph-target.yaml")?).map_err(|e| e.to_string())?;
    std::fs::write(root.join(&prefab), read("prefab-target.yaml")?).map_err(|e| e.to_string())?;
    std::fs::write(root.join(&binary), [0, 3, 0, 255]).map_err(|e| e.to_string())?;
    std::fs::write(root.join(&unrelated), "unstaged-local\n").map_err(|e| e.to_string())?;
    let index_before = git(root, &["ls-files", "--stage", "-z"])?;
    let head_before = git(root, &["rev-parse", "HEAD"])?;
    let request = PrepareRequest {
        sources: vec![SourceSelection {
            commits: vec![c1.clone(), c3.clone()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let job = merge_jobs::prepare(root, &request)?;
    let changes = merge_jobs::execute(root, &job.id, "changes", json!({"limit":10000}))?;
    let changes = changes["changes"]
        .as_array()
        .ok_or("Missing change catalog")?;
    let ids: Vec<_> = changes.iter().filter_map(|c| c["id"].as_str()).collect();
    check(!ids.is_empty(), "No source changes discovered")?;
    check(
        changes.iter().all(|c| c["path"].as_str() != Some(&skipped)),
        "Noncontiguous selection included an omitted commit",
    )?;
    merge_jobs::execute(root, &job.id, "include", json!({"change_ids":ids}))?;
    let incoming = changes
        .iter()
        .find(|c| {
            c["path"].as_str() == Some(&graph)
                && c["property_path"]
                    .as_str()
                    .is_some_and(|p| p.ends_with("/incomingValue"))
        })
        .ok_or("Graph catalog must expose incomingValue as a field operation")?;
    merge_jobs::execute(
        root,
        &job.id,
        "exclude",
        json!({"change_ids":[incoming["id"]]}),
    )?;
    let unresolved = merge_jobs::preview(root, &job.id)?;
    check(
        !unresolved.ready_to_apply,
        "Binary integration did not require an explicit version choice",
    )?;
    let binary_ids: Vec<_> = changes
        .iter()
        .filter(|c| c["path"].as_str() == Some(&binary))
        .filter_map(|c| c["id"].as_str())
        .collect();
    merge_jobs::execute(root, &job.id, "exclude", json!({"change_ids":binary_ids}))?;
    merge_jobs::execute(
        root,
        &job.id,
        "files.take",
        json!({"path":binary,"version":"source","commit":c1}),
    )?;
    let preview = merge_jobs::preview(root, &job.id)?;
    if !preview.ready_to_apply {
        return Err(format!(
            "Selective graph/prefab plan blocked: {}",
            json!(preview.issues)
        ));
    }
    let apply = json!({"expected_plan_hash":preview.plan_hash,"index_policy":"preserve"});
    merge_jobs::execute(root, &job.id, "apply", apply.clone())?;
    let merged_graph = std::fs::read(root.join(&graph)).map_err(|e| e.to_string())?;
    let merged_prefab = std::fs::read(root.join(&prefab)).map_err(|e| e.to_string())?;
    merge_jobs::execute(root, &job.id, "apply", apply)?;
    check(
        std::fs::read(root.join(&graph)).map_err(|e| e.to_string())? == merged_graph,
        "Retry applied a graph change twice",
    )?;
    check(
        std::fs::read(root.join(&prefab)).map_err(|e| e.to_string())? == merged_prefab,
        "Retry applied a prefab change twice",
    )?;
    check(
        index_before == git(root, &["ls-files", "--stage", "-z"])?,
        "Merge apply changed existing staged records",
    )?;
    check(
        head_before == git(root, &["rev-parse", "HEAD"])?,
        "Merge apply committed or changed HEAD",
    )?;
    check(
        std::fs::read(root.join(&unrelated)).map_err(|e| e.to_string())? == b"unstaged-local\n",
        "Unrelated dirty edit was overwritten",
    )?;
    check(
        !root.join(&skipped).exists(),
        "Skipped commit reached the working tree",
    )?;
    check(
        std::fs::read(root.join(&binary)).map_err(|e| e.to_string())? == [0, 2, 0, 255],
        "Binary side selection changed bytes",
    )?;
    let validation = merge_jobs::validate_unity(root, &job.id).await?;
    sink.emit("merge_unity_validation", &validation);
    let inspected = inspect_fixture(project, &folder).await?;
    validate_fixture(&inspected, 75)?;

    // Identical managed-reference fields conflict, while independently edited
    // fields and the shared/cyclic object identities remain intact after resolve.
    let conflict = unity_asset_core::prepare_merge(
        &read("graph-base.yaml")?,
        &read("graph-target.yaml")?,
        &read("graph-conflict.yaml")?,
    )
    .map_err(|e| e.to_string())?;
    check(
        conflict
            .catalog()
            .changes
            .iter()
            .any(|c| matches!(c.status, unity_asset_core::ChangeStatus::Conflict)),
        "Same-field changes were not classified as conflicts",
    )?;
    let include: Vec<_> = conflict
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: Resolution::Include,
        })
        .collect();
    check(
        !conflict.render(&include).map_err(|e| e.to_string())?.ready,
        "Unresolved managed-reference conflict was writable",
    )?;
    let resolved: Vec<_> = conflict
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: if matches!(c.status, unity_asset_core::ChangeStatus::Conflict) {
                Resolution::Source
            } else {
                Resolution::Include
            },
        })
        .collect();
    let resolved = conflict.render(&resolved).map_err(|e| e.to_string())?;
    check(
        resolved.ready,
        "Explicit source resolution did not resolve graph",
    )?;
    // Reimport the result in its original host, then restore the first plan's
    // exact bytes. Unity is the semantic oracle for alias/cycle identity.
    std::fs::write(root.join(&graph), &resolved.bytes).map_err(|e| e.to_string())?;
    let conflict_inspected = inspect_fixture(project, &folder).await?;
    check(
        conflict_inspected["health"] == 100
            && conflict_inspected["sharedIdentity"] == true
            && conflict_inspected["cycleIdentity"] == true,
        "Resolved graph failed Unity round-trip",
    )?;
    std::fs::write(root.join(&graph), &merged_graph).map_err(|e| e.to_string())?;
    validate_fixture(&inspect_fixture(project, &folder).await?, 75)?;

    let type_merge = unity_asset_core::prepare_merge(
        &read("graph-base.yaml")?,
        &read("graph-type-target.yaml")?,
        &read("graph-type-change.yaml")?,
    )
    .map_err(|e| e.to_string())?;
    check(
        type_merge.catalog().changes.iter().any(|c| {
            matches!(c.status, unity_asset_core::ChangeStatus::Conflict)
                && c.property_path.contains("103")
        }),
        "Same-rid type change and old-type field edit must conflict",
    )?;
    let type_decisions: Vec<_> = type_merge
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: if matches!(c.status, unity_asset_core::ChangeStatus::Conflict) {
                Resolution::Source
            } else {
                Resolution::Include
            },
        })
        .collect();
    let type_result = type_merge
        .render(&type_decisions)
        .map_err(|e| e.to_string())?;
    check(
        type_result.ready,
        "Explicit managed-reference type selection did not produce a valid graph",
    )?;
    std::fs::write(root.join(&graph), &type_result.bytes).map_err(|e| e.to_string())?;
    let type_report = inspect_fixture(project, &folder).await?;
    check(
        type_report["alternateType"] == "MergeWeightedAction"
            && type_report["sharedIdentity"] == true
            && type_report["cycleIdentity"] == true
            && type_report["missingTypes"] == false,
        "Unity failed to deserialize a resolved same-rid type change",
    )?;
    let reorder = unity_asset_core::prepare_merge(
        &read("graph-type-change.yaml")?,
        &read("graph-reorder-target.yaml")?,
        &read("graph-reordered.yaml")?,
    )
    .map_err(|e| e.to_string())?;
    let reorder_include: Vec<_> = reorder
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: Resolution::Include,
        })
        .collect();
    check(
        !reorder
            .render(&reorder_include)
            .map_err(|e| e.to_string())?
            .ready,
        "Competing reorders with duplicate shared references must conflict",
    )?;
    let reorder_decisions: Vec<_> = reorder
        .catalog()
        .changes
        .iter()
        .map(|c| Decision {
            change_id: c.id.clone(),
            resolution: Resolution::Source,
        })
        .collect();
    let reorder_result = reorder
        .render(&reorder_decisions)
        .map_err(|e| e.to_string())?;
    check(
        reorder_result.ready,
        "Explicit list ordering did not resolve the conflict",
    )?;
    std::fs::write(root.join(&graph), &reorder_result.bytes).map_err(|e| e.to_string())?;
    let reorder_report = inspect_fixture(project, &folder).await?;
    check(
        reorder_report["childOrder"] == "weighted|shared|shared"
            && reorder_report["sharedIdentity"] == true
            && reorder_report["cycleIdentity"] == true,
        "Unity did not preserve the selected list order and alias graph",
    )?;
    std::fs::write(root.join(&graph), &merged_graph).map_err(|e| e.to_string())?;
    validate_fixture(&inspect_fixture(project, &folder).await?, 75)?;

    let stale = merge_jobs::prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![conflict_commit],
                ..Default::default()
            }],
            ..Default::default()
        },
    )?;
    merge_jobs::execute(
        root,
        &stale.id,
        "files.take",
        json!({"path":graph,"version":"source"}),
    )?;
    let stale_preview = merge_jobs::preview(root, &stale.id)?;
    let mut concurrent = merged_graph.clone();
    concurrent.push(b'\n');
    std::fs::write(root.join(&graph), &concurrent).map_err(|e| e.to_string())?;
    let stale_result = merge_jobs::execute(
        root,
        &stale.id,
        "apply",
        json!({"expected_plan_hash":stale_preview.plan_hash,"index_policy":"preserve"}),
    );
    check(
        stale_result.is_err(),
        "Post-plan local edit was overwritten",
    )?;
    check(
        std::fs::read(root.join(&graph)).map_err(|e| e.to_string())? == concurrent,
        "Rejected stale plan changed target bytes",
    )?;
    std::fs::write(root.join(&graph), &merged_graph).map_err(|e| e.to_string())?;
    let corpus = corpus_roundtrip(root)?;
    let schema = run_schema_alias_acceptance(project, &folder, &token).await?;
    let sdk = super::merge_sdk_acceptance::run(app, project, config, sink).await?;
    let commit_scope = super::merge_sdk_commit_acceptance::run(app, project, config, sink).await?;
    let report = json!({"suite":"asset-merge","passed":29,"failed":0,"unityVersion":version,"jobId":job.id,"fixture":folder,
        "selectedCommits":[c1,c3],"omittedCommit":c2,"unity":inspected,"typeChange":type_report,"listReorder":reorder_report,"corpus":corpus,"schemaMigration":schema,"sdk":sdk,"commitScope":commit_scope,"snapshots":snapshots});
    std::fs::write(
        snapshots.join("acceptance.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    sink.emit("suite_result", report);
    Ok(())
}

async fn run_schema_alias_acceptance(
    project: &str,
    folder: &str,
    token: &str,
) -> Result<Value, String> {
    use crate::merge_jobs::{self, PrepareRequest, SourceSelection};
    let root = Path::new(project);
    let class = format!("LocusMergeSchemaProbe_{token}");
    let script = format!("{folder}/{class}.cs");
    let asset = format!("{folder}/Schema.asset");
    let old_code=format!("using UnityEngine;\npublic sealed class {class} : ScriptableObject {{ public int legacyValue = 10; public int localValue = 1; }}\n");
    std::fs::write(root.join(&script), &old_code).map_err(|e| e.to_string())?;
    unity_bridge::recompile_and_wait(project).await?;
    execute_capture(project,&format!(r#"
UnityEditor.AssetDatabase.Refresh(UnityEditor.ImportAssetOptions.ForceSynchronousImport);
var script = UnityEditor.AssetDatabase.LoadAssetAtPath<UnityEditor.MonoScript>({script});
if (script == null || script.GetClass() == null) throw new System.Exception("Migration fixture script did not compile");
var asset = UnityEngine.ScriptableObject.CreateInstance(script.GetClass());
UnityEditor.AssetDatabase.CreateAsset(asset, {asset});
UnityEditor.AssetDatabase.SaveAssets();
print("SCHEMA_BASE_CREATED");
"#,script=json!(script),asset=json!(asset))).await?;
    let base = snapshot_commit(root, &[folder], "Old serialized field schema")?;
    execute_capture(
        project,
        &format!(
            r#"
var asset = UnityEditor.AssetDatabase.LoadMainAssetAtPath({asset});
var serialized = new UnityEditor.SerializedObject(asset);
serialized.FindProperty("legacyValue").intValue = 20;
serialized.ApplyModifiedPropertiesWithoutUndo();
UnityEditor.EditorUtility.SetDirty(asset); UnityEditor.AssetDatabase.SaveAssets();
print("SCHEMA_SOURCE_CREATED");
"#,
            asset = json!(asset)
        ),
    )
    .await?;
    let source = patch_commit(
        root,
        &base,
        &[(
            asset.clone(),
            std::fs::read(root.join(&asset)).map_err(|e| e.to_string())?,
        )],
        "Source changes the former field",
    )?;
    execute_capture(
        project,
        &format!(
            r#"
var asset = UnityEditor.AssetDatabase.LoadMainAssetAtPath({asset});
var serialized = new UnityEditor.SerializedObject(asset);
serialized.FindProperty("legacyValue").intValue = 10;
serialized.FindProperty("localValue").intValue = 99;
serialized.ApplyModifiedPropertiesWithoutUndo();
UnityEditor.EditorUtility.SetDirty(asset); UnityEditor.AssetDatabase.SaveAssets();
print("SCHEMA_TARGET_CREATED");
"#,
            asset = json!(asset)
        ),
    )
    .await?;
    let new_code=format!("using UnityEngine;\nusing UnityEngine.Serialization;\npublic sealed class {class} : ScriptableObject {{ [FormerlySerializedAs(\"legacyValue\")] public int currentValue = 10; public int localValue = 1; }}\n");
    std::fs::write(root.join(&script), &new_code).map_err(|e| e.to_string())?;
    unity_bridge::recompile_and_wait(project).await?;
    execute_capture(project,&format!(r#"
UnityEditor.AssetDatabase.ImportAsset({asset}, UnityEditor.ImportAssetOptions.ForceUpdate | UnityEditor.ImportAssetOptions.ForceSynchronousImport);
var asset = UnityEditor.AssetDatabase.LoadMainAssetAtPath({asset});
var serialized = new UnityEditor.SerializedObject(asset);
if (serialized.FindProperty("currentValue").intValue != 10 || serialized.FindProperty("localValue").intValue != 99) throw new System.Exception("Unity schema migration lost the baseline");
UnityEditor.EditorUtility.SetDirty(asset); UnityEditor.AssetDatabase.SaveAssets();
UnityEditor.AssetDatabase.ForceReserializeAssets(new [] {{ {asset} }});
print("SCHEMA_TARGET_RENAMED");
"#,asset=json!(asset))).await?;
    let index_before = git(root, &["ls-files", "--stage", "-z"])?;
    let job = merge_jobs::prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![source.clone()],
                ..Default::default()
            }],
            ..Default::default()
        },
    )?;
    merge_jobs::execute(root, &job.id, "include", json!({"path":asset,"all":true}))?;
    let preview = merge_jobs::preview(root, &job.id)?;
    if !preview.ready_to_apply {
        return Err(format!(
            "FormerlySerializedAs plan failed: {}",
            json!(preview.issues)
        ));
    }
    merge_jobs::execute(
        root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":preview.plan_hash,"index_policy":"preserve"}),
    )?;
    merge_jobs::validate_unity(root, &job.id).await?;
    let result=execute_capture(project,&format!(r#"
var asset = UnityEditor.AssetDatabase.LoadMainAssetAtPath({asset});
var serialized = new UnityEditor.SerializedObject(asset);
if (serialized.FindProperty("currentValue").intValue != 20 || serialized.FindProperty("localValue").intValue != 99 || serialized.FindProperty("legacyValue") != null) throw new System.Exception("FormerlySerializedAs merge did not preserve source value and local edits");
print("SCHEMA_MERGE_VERIFIED");
"#,asset=json!(asset))).await?;
    check(
        result.contains("SCHEMA_MERGE_VERIFIED"),
        "Unity did not verify the merged serialized schema",
    )?;
    check(
        std::fs::read_to_string(root.join(&script)).map_err(|e| e.to_string())? == new_code,
        "Merge overwrote target's uncommitted schema change",
    )?;
    check(
        git(root, &["ls-files", "--stage", "-z"])? == index_before,
        "Schema merge changed the existing index",
    )?;
    Ok(
        json!({"jobId":job.id,"commit":source,"script":script,"asset":asset,"oldField":"legacyValue","newField":"currentValue","mergedValue":20,"localValue":99,"unityVerified":true}),
    )
}

fn capture_json(text: &str) -> Result<Value, String> {
    for line in text.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
            if value.is_object() {
                return Ok(value);
            }
        }
    }
    let start = text
        .find('{')
        .ok_or_else(|| format!("Unity returned no JSON: {}", clip(text, 500)))?;
    let end = text.rfind('}').ok_or("Unity returned incomplete JSON")?;
    serde_json::from_str(&text[start..=end])
        .map_err(|e| format!("Unity report parse failed: {e}; {}", clip(text, 1000)))
}

async fn inspect_fixture(project: &str, folder: &str) -> Result<Value, String> {
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

fn validate_fixture(report: &Value, health: i64) -> Result<(), String> {
    check(
        report["localValue"] == 99 && report["incomingValue"] == 20,
        "Agent's include/exclude decisions changed after import",
    )?;
    check(
        report["health"] == health && report["speed"].as_f64() == Some(2.5),
        "Independent SerializeReference fields did not merge",
    )?;
    check(
        report["alternateLabel"] == "source alternate",
        "Polymorphic list member edit was lost",
    )?;
    check(
        report["sharedIdentity"] == true
            && report["cycleIdentity"] == true
            && report["nullPreserved"] == true
            && report["missingTypes"] == false
            && report["sharedId"] == "9007199254740993",
        "Managed reference identities did not survive Unity deserialization",
    )?;
    check(
        report["x"].as_f64() == Some(3.0)
            && report["y"].as_f64() == Some(4.0)
            && report["prefabReferences"] == true,
        "Nested prefab or flow-field merge did not survive import",
    )
}

fn indexed_git(
    root: &Path,
    index: &Path,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut cmd = crate::process_util::command("git");
    cmd.arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "Locus CLI Driver")
        .env("GIT_AUTHOR_EMAIL", "driver@locus.local")
        .env("GIT_COMMITTER_NAME", "Locus CLI Driver")
        .env("GIT_COMMITTER_EMAIL", "driver@locus.local")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or("Git stdin unavailable")?
            .write_all(input)
            .map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn snapshot_commit(root: &Path, paths: &[&str], message: &str) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = dir.path().join("index");
    let parent =
        String::from_utf8(git(root, &["rev-parse", "HEAD"])?).map_err(|e| e.to_string())?;
    indexed_git(root, &index, &["read-tree", parent.trim()], None)?;
    let mut args = vec!["add", "-A", "--"];
    args.extend_from_slice(paths);
    indexed_git(root, &index, &args, None)?;
    finish_commit(root, &index, parent.trim(), message)
}

fn patch_commit(
    root: &Path,
    parent: &str,
    replacements: &[(String, Vec<u8>)],
    message: &str,
) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index = dir.path().join("index");
    indexed_git(root, &index, &["read-tree", parent], None)?;
    for (path, bytes) in replacements {
        let oid = String::from_utf8(indexed_git(
            root,
            &index,
            &["hash-object", "-w", "--stdin"],
            Some(bytes),
        )?)
        .map_err(|e| e.to_string())?;
        indexed_git(
            root,
            &index,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "100644",
                oid.trim(),
                path,
            ],
            None,
        )?;
    }
    finish_commit(root, &index, parent, message)
}

fn finish_commit(root: &Path, index: &Path, parent: &str, message: &str) -> Result<String, String> {
    let tree = String::from_utf8(indexed_git(root, index, &["write-tree"], None)?)
        .map_err(|e| e.to_string())?;
    String::from_utf8(indexed_git(
        root,
        index,
        &["commit-tree", tree.trim(), "-p", parent, "-m", message],
        None,
    )?)
    .map(|v| v.trim().to_string())
    .map_err(|e| e.to_string())
}

fn corpus_roundtrip(root: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let mut files = 0;
    let mut bytes = 0u64;
    let mut errors = Vec::new();
    for entry in walkdir::WalkDir::new(root.join("Assets"))
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        if !matches!(ext, "prefab" | "unity" | "asset" | "mat" | "meta") {
            continue;
        }
        let raw = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        if raw.len() > 8 * 1024 * 1024 || (!raw.starts_with(b"%YAML") && ext != "meta") {
            continue;
        }
        match crate::unity_asset_core::prepare_merge(&raw, &raw, &raw)
            .and_then(|session| session.render(&[]))
        {
            Ok(result) if result.ready && result.bytes == raw => {}
            Ok(result) => errors.push(format!(
                "{}: no-op changed bytes or returned conflicts: {:?}",
                entry.path().display(),
                result.conflicts
            )),
            Err(error) => errors.push(format!("{}: {error}", entry.path().display())),
        }
        files += 1;
        bytes += raw.len() as u64;
        if files >= 256 {
            break;
        }
    }
    check(files > 0, "No real Unity assets were examined")?;
    if !errors.is_empty() {
        return Err(format!(
            "Real-project parser round-trip failures: {}",
            errors.join("\n")
        ));
    }
    Ok(
        json!({"files":files,"bytes":bytes,"elapsedMs":started.elapsed().as_millis(),"byteExact":true}),
    )
}
