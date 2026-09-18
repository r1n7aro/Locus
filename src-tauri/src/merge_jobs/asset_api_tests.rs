use super::*;
use tempfile::TempDir;

const ASSET_PATH: &str = "Assets/Fixture.asset";
fn yaml(amount: u32) -> String {
    format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  amount: {amount}\n  numbers: [10, 20]\n  large: 9007199254740993\n  row: {{x: 1, y: 2}}\n")
}

fn setup() -> (TempDir, MergeJob) {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("Assets")).unwrap();
    git(root, &["init", "-b", "main"]).unwrap();
    git(root, &["config", "user.email", "asset-api@example.invalid"]).unwrap();
    git(root, &["config", "user.name", "Asset API Test"]).unwrap();
    git(root, &["config", "core.autocrlf", "false"]).unwrap();
    std::fs::write(root.join(ASSET_PATH), yaml(1)).unwrap();
    git(root, &["add", "."]).unwrap();
    git(root, &["commit", "-m", "base"]).unwrap();
    git(root, &["switch", "-c", "source"]).unwrap();
    std::fs::write(root.join(ASSET_PATH), yaml(2)).unwrap();
    git(root, &["add", "."]).unwrap();
    git(root, &["commit", "-m", "source amount"]).unwrap();
    let commit = git_text(root, &["rev-parse", "HEAD"]).unwrap();
    git(root, &["switch", "main"]).unwrap();
    let job = prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![commit],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    (dir, job)
}

fn operations() -> Value {
    json!([
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/amount","value":9},
        {"op":"array_insert","object_id":"11400000","property_path":"/MonoBehaviour/numbers","index":1,"value":15},
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/large","value":{"kind":"int64","value":"9223372036854775807"}}
    ])
}

#[test]
fn shared_asset_api_stages_frozen_merge_values_and_materializes_direct_core_result() {
    let (repo, job) = setup();
    let root = repo.path();
    let original = std::fs::read(root.join(ASSET_PATH)).unwrap();
    let snapshot = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let params = json!({"path":ASSET_PATH,"operations":operations(),"expected_revision":snapshot["revision"],"persist":"plan"});
    let preview_result = execute(root, &job.id, "assets.preview", params.clone()).unwrap();
    assert_eq!(preview_result["applied"], false);
    assert_eq!(preview_result["persisted"], false);
    assert_eq!(load(root, &job.id).unwrap().revision, job.revision);
    assert_eq!(std::fs::read(root.join(ASSET_PATH)).unwrap(), original);
    let applied = execute(root, &job.id, "assets.apply", params).unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["persisted"], false);
    assert_eq!(applied["destination"], "merge_plan");
    assert_eq!(applied["operations_count"], 3);
    assert_eq!(std::fs::read(root.join(ASSET_PATH)).unwrap(), original);
    let current = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let direct = crate::unity_asset_core::edit(
        &original,
        &serde_json::from_value::<Vec<crate::unity_asset_core::AssetOperation>>(operations())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        current["objects"],
        serde_json::to_value(&direct.snapshot.objects).unwrap()
    );
    let plan = preview(root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(std::fs::read(root.join(ASSET_PATH)).unwrap(), direct.bytes);
}

#[test]
fn asset_edit_rejects_stale_revision_failed_batches_and_overlapping_plan_scopes() {
    let (repo, job) = setup();
    let root = repo.path();
    let snapshot = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let old_revision = snapshot["revision"].clone();
    execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"operations":operations(),"expected_revision":old_revision,"persist":"plan"})).unwrap();
    assert!(execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"operations":operations(),"expected_revision":old_revision,"persist":"plan"})).unwrap_err().contains("stale_revision"));
    let before = load(root, &job.id).unwrap();
    let snapshot = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let bad = json!([
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/amount","value":17},
        {"op":"array_remove","object_id":"11400000","property_path":"/MonoBehaviour/numbers","index":999}
    ]);
    assert!(execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"operations":bad,"expected_revision":snapshot["revision"],"persist":"plan"})).is_err());
    assert_eq!(
        serde_json::to_value(load(root, &job.id).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    execute(root, &job.id, "fields.set", json!({"path":ASSET_PATH,"object_id":"11400000","property_path":"/MonoBehaviour/row","value":{"x":4,"y":5}})).unwrap();
    let snapshot = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let error = execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"expected_revision":snapshot["revision"],"persist":"plan","operations":[
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/row/x","value":8}
    ]})).unwrap_err();
    assert!(error.contains("asset_plan_conflict"), "{error}");
    assert_eq!(
        std::fs::read_to_string(root.join(ASSET_PATH)).unwrap(),
        yaml(1)
    );
}

#[test]
fn merge_preview_accepts_optional_revision_and_rejects_disk_persistence() {
    let (repo, job) = setup();
    let root = repo.path();
    let result = execute(
        root,
        &job.id,
        "assets.preview",
        json!({"path":ASSET_PATH,"operations":operations(),"persist":"plan"}),
    )
    .unwrap();
    assert_eq!(result["persisted"], false);
    assert!(execute(
        root,
        &job.id,
        "assets.apply",
        json!({"path":ASSET_PATH,"operations":operations(),"persist":"plan"})
    )
    .unwrap_err()
    .contains("expected_revision"));
    assert!(execute(
        root,
        &job.id,
        "assets.preview",
        json!({"path":ASSET_PATH,"operations":operations(),"persist":"disk"})
    )
    .unwrap_err()
    .contains("persist='plan'"));
}

#[test]
fn merge_asset_validation_uses_frozen_script_types_after_working_source_changes() {
    let (repo, original_job) = setup();
    let root = repo.path();
    let guid = "aabbccdd00112233445566778899aabb";
    std::fs::write(root.join("Assets/Config.cs"), "using UnityEngine; public class Config : ScriptableObject { public int amount; public int[] numbers; public long large; public Row row; [System.Serializable] public struct Row { public int x; public int y; } }").unwrap();
    std::fs::write(
        root.join("Assets/Config.cs.meta"),
        format!("guid: {guid}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join(ASSET_PATH),
        yaml(1).replace(
            "MonoBehaviour:\n",
            &format!("MonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n"),
        ),
    )
    .unwrap();
    let job = prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![original_job.deltas[0].commit.clone()],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(
        root.join("Assets/Config.cs"),
        "using UnityEngine; public class Config : ScriptableObject { public float amount; }",
    )
    .unwrap();
    let error = execute(
        root,
        &job.id,
        "assets.preview",
        json!({"path":ASSET_PATH,"persist":"plan","operations":[
            {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/amount","value":1.5}
        ]}),
    )
    .unwrap_err();
    assert!(error.contains("schema_type_mismatch"), "{error}");
    assert!(error.contains("exact integer"), "{error}");
}

#[test]
fn merge_packed_array_index_edits_persist_compact_bytes_in_the_frozen_plan() {
    let (repo, original_job) = setup();
    let root = repo.path();
    let guid = "aabbccdd00112233445566778899aabb";
    std::fs::write(
        root.join("Assets/Config.cs"),
        "using UnityEngine; public class Config : ScriptableObject { public int[] numbers; }",
    )
    .unwrap();
    std::fs::write(
        root.join("Assets/Config.cs.meta"),
        format!("guid: {guid}\n"),
    )
    .unwrap();
    let before = yaml(1)
        .replace(
            "MonoBehaviour:\n",
            &format!("MonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n"),
        )
        .replace("numbers: [10, 20]", "numbers: 010000000200000003000000");
    std::fs::write(root.join(ASSET_PATH), &before).unwrap();
    let job = prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![original_job.deltas[0].commit.clone()],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let snapshot = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let result = execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"persist":"plan","expected_revision":snapshot["revision"],"operations":[
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/numbers/0","value":9},
        {"op":"array_remove","object_id":"11400000","property_path":"/MonoBehaviour/numbers","index":1}
    ]})).unwrap();
    assert_eq!(result["persisted"], false);
    assert_eq!(
        std::fs::read_to_string(root.join(ASSET_PATH)).unwrap(),
        before
    );
    let plan = preview(root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    let bytes = state_bytes(
        &job_dir(Path::new(&job.root), &job.id).unwrap(),
        &plan.files[ASSET_PATH],
    )
    .unwrap();
    assert!(String::from_utf8(bytes.clone())
        .unwrap()
        .contains("numbers: 0900000003000000\n"));
    execute(
        root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert_eq!(std::fs::read(root.join(ASSET_PATH)).unwrap(), bytes);
}

#[test]
fn merge_uint64_whole_array_read_modify_write_preserves_the_unsigned_maximum() {
    let (repo, original_job) = setup();
    let root = repo.path();
    let guid = "aabbccdd00112233445566778899aabb";
    std::fs::write(
        root.join("Assets/Config.cs"),
        "using UnityEngine; public class Config : ScriptableObject { public ulong[] numbers; }",
    )
    .unwrap();
    std::fs::write(
        root.join("Assets/Config.cs.meta"),
        format!("guid: {guid}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join(ASSET_PATH),
        yaml(1)
            .replace(
                "MonoBehaviour:\n",
                &format!(
                    "MonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n"
                ),
            )
            .replace(
                "numbers: [10, 20]",
                "numbers: 0100000000000000ffffffffffffffff",
            ),
    )
    .unwrap();
    let job = prepare(
        root,
        &PrepareRequest {
            sources: vec![SourceSelection {
                commits: vec![original_job.deltas[0].commit.clone()],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let read = execute(root, &job.id, "assets.read", json!({"path":ASSET_PATH})).unwrap();
    let mut values = read["objects"][0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["property_path"] == "/MonoBehaviour/numbers")
        .unwrap()["value"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        values[1],
        json!({"kind":"uint64","value":"18446744073709551615"})
    );
    values.reverse();
    execute(root, &job.id, "assets.apply", json!({"path":ASSET_PATH,"expected_revision":read["revision"],"persist":"plan","operations":[
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/numbers","value":values}
    ]})).unwrap();
    let plan = preview(root, &job.id).unwrap();
    assert!(plan.ready_to_apply, "{:?}", plan.issues);
    execute(
        root,
        &job.id,
        "apply",
        json!({"expected_plan_hash":plan.plan_hash}),
    )
    .unwrap();
    assert!(std::fs::read_to_string(root.join(ASSET_PATH))
        .unwrap()
        .contains("numbers: ffffffffffffffff0100000000000000"));
}
