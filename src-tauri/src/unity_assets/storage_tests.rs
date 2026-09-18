use super::*;
use tempfile::TempDir;

fn yaml(value: u32) -> Vec<u8> {
    format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  value: {value}\n  values: [1, 2]\n").into_bytes()
}

fn project() -> TempDir {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("Assets")).unwrap();
    fs::write(root.path().join("Assets/A.asset"), yaml(1)).unwrap();
    fs::write(root.path().join("Assets/B.asset"), yaml(2)).unwrap();
    root
}

fn entry(path: &str, before: &[u8], value: u32) -> Value {
    json!({"path":path,"expected_revision":blake3::hash(before).to_hex().to_string(),"operations":[
        {"op":"set","object_id":"11400000","property_path":"/MonoBehaviour/value","value":value}
    ]})
}

#[test]
fn no_op_transaction_preserves_target_modified_time(){
    let project=project();let path=project.path().join("Assets/A.asset");
    fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(std::time::UNIX_EPOCH+std::time::Duration::from_secs(1_600_000_000)).unwrap();
    let before=fs::read(&path).unwrap();let modified=fs::metadata(&path).unwrap().modified().unwrap();
    let mut request=entry("Assets/A.asset",&before,1);request["action"]=json!("apply");
    assert_eq!(execute(project.path(),&request).unwrap()["persisted"],true);
    assert_eq!(fs::read(&path).unwrap(),before);assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(),modified);
}

#[test]
fn uppercase_guid_cannot_bypass_exact_external_file_id_validation() {
    let project = project();
    let root = project.path();
    let guid = "abcdef0123456789abcdef0123456789";
    fs::write(
        root.join("Assets/B.asset.meta"),
        format!("fileFormatVersion: 2\nguid: {guid}\n"),
    )
    .unwrap();
    let before = b"%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  reference: {fileID: 0}\n";
    fs::write(root.join("Assets/A.asset"), before).unwrap();
    let request = |id: &str| {
        json!({"action":"apply","path":"Assets/A.asset",
        "expected_revision":blake3::hash(before).to_hex().to_string(), "operations":[{
            "op":"set","object_id":"11400000","property_path":"/MonoBehaviour/reference",
            "value":{"fileID":id,"guid":guid.to_ascii_uppercase(),"type":2}
        }]})
    };
    let error = execute(root, &request("9223372036854775807")).unwrap_err();
    assert!(
        error.contains("assets.missing_reference") && error.contains("fileID"),
        "{error}"
    );
    assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), before);
    assert_eq!(
        execute(root, &request("11400000")).unwrap()["persisted"],
        true
    );
}

#[test]
fn interrupted_journal_recovery_retains_later_external_writes_and_can_resume() {
    let project = project();
    let root = project.path();
    let id = uuid::Uuid::new_v4().to_string();
    let transaction = directory(root).unwrap().join(&id);
    fs::create_dir(&transaction).unwrap();
    let mut journal = Journal {
        id: id.clone(),
        state: "applying".into(),
        before: BTreeMap::new(),
        after: BTreeMap::new(),
    };
    for (path, before, after) in [
        ("Assets/A.asset", yaml(1), yaml(11)),
        ("Assets/B.asset", yaml(2), yaml(22)),
    ] {
        journal.before.insert(
            path.into(),
            io::store_blob(&transaction, &before, "100644").unwrap(),
        );
        journal.after.insert(
            path.into(),
            io::store_blob(&transaction, &after, "100644").unwrap(),
        );
        fs::write(root.join(path), after).unwrap();
    }
    persist_journal(&transaction, &journal).unwrap();
    let external = yaml(99);
    fs::write(root.join("Assets/B.asset"), &external).unwrap();
    let error = execute(root, &json!({"action":"recover","transaction_id":id})).unwrap_err();
    assert!(error.contains("assets.recovery_required"), "{error}");
    assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), yaml(1));
    assert_eq!(fs::read(root.join("Assets/B.asset")).unwrap(), external);
    let persisted: Journal =
        serde_json::from_slice(&fs::read(transaction.join("journal.json")).unwrap()).unwrap();
    assert_eq!(persisted.state, "recovery_required");
    let mut pending = entry("Assets/A.asset", &yaml(1), 33);
    pending["action"] = json!("apply");
    assert!(execute(root, &pending)
        .unwrap_err()
        .contains("assets.recovery_required"));
    assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), yaml(1));
    // The caller explicitly reconciles its later change, then retries recovery.
    fs::write(root.join("Assets/B.asset"), yaml(2)).unwrap();
    let recovered = execute(root, &json!({"action":"recover","transaction_id":id})).unwrap();
    assert_eq!(recovered["state"], "rolled_back");
    assert_eq!(
        execute(root, &json!({"action":"recover","transaction_id":id})).unwrap()["state"],
        "rolled_back"
    );
    assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), yaml(1));
    assert_eq!(fs::read(root.join("Assets/B.asset")).unwrap(), yaml(2));
}

#[test]
fn all_asset_preflight_prevents_early_writes_for_late_stale_or_invalid_operations() {
    let project = project();
    let root = project.path();
    let good = entry("Assets/A.asset", &yaml(1), 11);
    let mut stale = entry("Assets/B.asset", &yaml(2), 22);
    stale["expected_revision"] = json!("outdated");
    let mut invalid = entry("Assets/B.asset", &yaml(2), 22);
    invalid["operations"] = json!([
        {"op":"array_remove","object_id":"11400000","property_path":"/MonoBehaviour/values","index":20}
    ]);
    for bad in [stale, invalid] {
        assert!(execute(root, &json!({"action":"apply_batch","entries":[good,bad]})).is_err());
        assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), yaml(1));
        assert_eq!(fs::read(root.join("Assets/B.asset")).unwrap(), yaml(2));
        assert!(fs::read_dir(directory(root).unwrap())
            .unwrap()
            .all(|entry| !entry.unwrap().file_type().unwrap().is_dir()));
    }
}

#[test]
fn committed_batch_has_durable_before_after_blobs_and_cannot_be_reverted_by_recovery() {
    let project = project();
    let root = project.path();
    let result = execute(
        root,
        &json!({"action":"apply_batch","entries":[
            entry("Assets/A.asset", &yaml(1), 11), entry("Assets/B.asset", &yaml(2), 22)
        ]}),
    )
    .unwrap();
    assert_eq!(result["applied"], true);
    assert_eq!(result["persisted"], true);
    let id = result["transaction_id"].as_str().unwrap();
    let transaction = directory(root).unwrap().join(id);
    let journal: Journal =
        serde_json::from_slice(&fs::read(transaction.join("journal.json")).unwrap()).unwrap();
    assert_eq!(journal.state, "committed");
    for (path, before, after) in [
        ("Assets/A.asset", yaml(1), yaml(11)),
        ("Assets/B.asset", yaml(2), yaml(22)),
    ] {
        assert_eq!(
            io::read_blob(&transaction, &journal.before[path]).unwrap(),
            before
        );
        assert_eq!(
            io::read_blob(&transaction, &journal.after[path]).unwrap(),
            after
        );
        assert_eq!(fs::read(root.join(path)).unwrap(), after);
    }
    assert!(
        execute(root, &json!({"action":"recover","transaction_id":id}))
            .unwrap_err()
            .contains("assets.already_committed")
    );
    assert_eq!(fs::read(root.join("Assets/A.asset")).unwrap(), yaml(11));
    assert_eq!(fs::read(root.join("Assets/B.asset")).unwrap(), yaml(22));
}

#[tokio::test]
async fn actual_unity2022_thirteen_primitive_array_matrix_previews_and_persists_offline() {
    // Exact Unity-produced bytes from unity-sample-project attempt 4. All work
    // below happens in a disposable copy and never contacts an Editor.
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("Assets")).unwrap();
    let original = include_bytes!("fixtures/unity2022_primitive_arrays.asset");
    fs::write(root.path().join("Assets/Numeric.asset"), original).unwrap();
    fs::write(
        root.path()
            .join("Assets/LocusAssetApiPrimitiveArraysFixture.cs"),
        include_str!(
            "../../../locus_unity/Runtime/MergeTesting/LocusAssetApiPrimitiveArraysFixture.cs"
        ),
    )
    .unwrap();
    fs::write(
        root.path()
            .join("Assets/LocusAssetApiPrimitiveArraysFixture.cs.meta"),
        include_str!(
            "../../../locus_unity/Runtime/MergeTesting/LocusAssetApiPrimitiveArraysFixture.cs.meta"
        ),
    )
    .unwrap();
    let expected = BTreeMap::from([
        ("bytes", json!([1, 2, 250])),
        ("signedBytes", json!([-100, 0, 100])),
        ("shorts", json!([-30000, 0, 30000])),
        ("unsignedShorts", json!([0, 1000, 65000])),
        ("integers", json!([-12, 0, 512])),
        (
            "unsignedIntegers",
            json!([1, 2147483648_u64, 4000000000_u64]),
        ),
        (
            "longs",
            json!([{"kind":"int64","value":"-9007199254740993"},1,{"kind":"int64","value":"9007199254740993"}]),
        ),
        (
            "unsignedLongs",
            json!([1,{"kind":"int64","value":"9007199254740993"},{"kind":"uint64","value":"18446744073709551615"}]),
        ),
        ("floats", json!([0.125, -2.5, 7.75])),
        ("doubles", json!([0.125, -2.5, 7.75])),
        ("booleans", json!([1, 0, 1])),
        ("characters", json!([65, 20013, 122])),
        ("modes", json!([-2, 0, 7])),
    ]);
    let field = |snapshot: &Value, name: &str| -> Value {
        snapshot["objects"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["property_path"] == format!("/MonoBehaviour/{name}"))
            .unwrap_or_else(|| panic!("missing {name}"))
            .get("value")
            .unwrap()
            .clone()
    };
    let initial = super::super::execute(
        root.path(),
        json!({"action":"read","backend":"yaml","path":"Assets/Numeric.asset"}),
    )
    .await
    .unwrap();
    let mut operations = Vec::new();
    for (name, values) in &expected {
        assert_eq!(field(&initial, name), *values, "initial {name}");
        let path = format!("/MonoBehaviour/{name}");
        let first = values[0].clone();
        let last = values[2].clone();
        operations.extend([
            json!({"op":"array_insert","object_id":"11400000","property_path":path,"index":1,"value":first}),
            json!({"op":"array_move","object_id":"11400000","property_path":path,"index":0,"to_index":3}),
            json!({"op":"array_remove","object_id":"11400000","property_path":path,"index":1}),
            json!({"op":"array_resize","object_id":"11400000","property_path":path,"size":5,"value":first}),
            json!({"op":"set","object_id":"11400000","property_path":format!("{path}/0"),"value":last}),
        ]);
    }
    for (name, value) in [
        ("booleans", json!([2])),
        ("characters", json!([65536])),
        ("modes", json!([2147483648_u64])),
    ] {
        let bad = json!({"action":"apply","backend":"yaml","path":"Assets/Numeric.asset","expected_revision":initial["revision"],"operations":[
            {"op":"set","object_id":"11400000","property_path":format!("/MonoBehaviour/{name}"),"value":value}
        ]});
        assert!(super::super::execute(root.path(), bad)
            .await
            .unwrap_err()
            .contains("schema_type_mismatch"));
        assert_eq!(
            fs::read(root.path().join("Assets/Numeric.asset")).unwrap(),
            original
        );
    }
    let mut request = json!({"action":"preview","backend":"yaml","path":"Assets/Numeric.asset","expected_revision":initial["revision"],"operations":operations});
    let preview = super::super::execute(root.path(), request.clone())
        .await
        .unwrap();
    assert_eq!(preview["persisted"], false);
    assert_eq!(
        fs::read(root.path().join("Assets/Numeric.asset")).unwrap(),
        original
    );
    for (name, values) in &expected {
        assert_eq!(
            field(&preview["snapshot"], name),
            json!([values[2], values[2], values[0], values[0], values[0]]),
            "preview {name}"
        );
    }
    request["action"] = json!("apply");
    let applied = super::super::execute(root.path(), request).await.unwrap();
    assert_eq!(applied["persisted"], true);
    let final_read = super::super::execute(
        root.path(),
        json!({"action":"read","backend":"yaml","path":"Assets/Numeric.asset"}),
    )
    .await
    .unwrap();
    assert_eq!(applied["snapshot"]["objects"], final_read["objects"]);
    for (name, values) in &expected {
        assert_eq!(
            field(&final_read, name),
            json!([values[2], values[2], values[0], values[0], values[0]]),
            "persisted {name}"
        );
    }
    let text = fs::read_to_string(root.path().join("Assets/Numeric.asset")).unwrap();
    assert!(text.contains("booleans: 0101010101"));
    assert!(text.contains("characters: 7a007a00410041004100"));
    assert!(text.contains("modes: 0700000007000000fefffffffefffffffeffffff"));
    assert!(text.contains("floats: ["));
    assert!(text.contains("doubles: ["));
}
