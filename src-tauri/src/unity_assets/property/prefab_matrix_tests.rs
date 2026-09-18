//! Real Unity fixture -> production YAML service -> real Unity reload.
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("Assets")).unwrap();
    fs::write(root.path().join("Assets/Config.cs"),"using System; using System.Collections.Generic; using UnityEngine; public class Config:MonoBehaviour { [Serializable] public class Item { public bool flag; public float weight; public string label; public List<int> values; } public int amount; public List<Item> empty; public List<Item> items; }").unwrap();
    fs::write(
        root.path().join("Assets/Config.cs.meta"),
        "guid: cccccccccccccccccccccccccccccccc\n",
    )
    .unwrap();
    fs::write(root.path().join("Assets/Base.prefab"),"--- !u!114 &10\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  amount: 7\n  empty: []\n  items:\n  - flag: 1\n    weight: 1\n    label: 001\n    values: [1, 2]\n").unwrap();
    fs::write(
        root.path().join("Assets/Base.prefab.meta"),
        "guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .unwrap();
    let record = |p: &str, v: &str| json!({"target":{"fileID":10,"guid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","type":3},"propertyPath":p,"value":v,"objectReference":{"fileID":0}});
    fs::write(root.path().join("Assets/Variant.prefab"),format!("--- !u!1001 &100\nPrefabInstance:\n  m_SourcePrefab: {{fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}}\n  m_Modification:\n    m_Modifications: {}\n",json!([record("empty.Array.size","1"),record("empty.Array.data[0].label","007"),record("empty.Array.data[0].weight","1"),record("empty.Array.data[0].values.Array.size","1"),record("empty.Array.data[0].values.Array.data[0]","9"),record("items.Array.size","2")]))).unwrap();
    root
}
fn target(property: &str) -> Value {
    json!({"kind":"asset","path":"Assets/Variant.prefab","targetFileId":"110","propertyPath":property})
}
async fn read(root: &Path, property: &str) -> Value {
    crate::unity_assets::execute(
        root,
        json!({"action":"read_property","target":target(property)}),
    )
    .await
    .unwrap()
}
async fn apply(root: &Path, before: &Value, ops: Vec<(&str, Value)>) -> Result<Value, String> {
    let writes=ops.into_iter().map(|(p,v)|json!({"target":target(p),"value":v,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]})).collect::<Vec<_>>();
    crate::unity_assets::execute(
        root,
        json!({"action":"apply_properties","writes":writes,"resultMode":"summary","profile":true}),
    )
    .await
}

#[tokio::test]
async fn prefab_empty_and_inherited_array_leaf_types_are_preserved() {
    let root = fixture();
    for (p, expected) in [
        ("empty.Array.data[0].flag", json!(false)),
        ("empty.Array.data[0].weight", json!(1.0)),
        ("empty.Array.data[0].label", json!("007")),
        ("empty.Array.data[0].values.Array.data[0]", json!(9)),
        ("items.Array.data[1].flag", json!(true)),
        ("items.Array.data[1].weight", json!(1.0)),
        ("items.Array.data[1].label", json!("001")),
    ] {
        assert_eq!(read(root.path(), p).await["value"], expected, "{p}");
    }
}

#[tokio::test]
async fn prefab_array_phase_batches_and_rejects_invalid_overwritten_values() {
    let root = fixture();
    let before = read(root.path(), "amount").await;
    let item = json!({"flag":true,"weight":2.5,"label":"009","values":[3,4]});
    let mut ops = vec![("empty", json!({"action":"insert","index":1,"value":item}))];
    ops.extend((0..1000).map(|n| ("empty.Array.data[1].values.Array.data[1]", json!(n))));
    let result = apply(root.path(), &before, ops).await.unwrap();
    assert_eq!(result["profile"]["overrideValidationPasses"], 1);
    assert_eq!(
        read(root.path(), "empty.Array.data[1].values.Array.data[1]").await["value"],
        999
    );
    let bytes = fs::read(root.path().join("Assets/Variant.prefab")).unwrap();
    assert!(apply(root.path(), &before, vec![("amount", json!(99))])
        .await
        .is_err());
    let before = read(root.path(), "amount").await;
    let error = apply(
        root.path(),
        &before,
        vec![
            ("empty", json!({"action":"insert","index":2,"value":item})),
            ("empty.Array.data[2].weight", json!("bad")),
            ("empty.Array.data[2].weight", json!(3)),
        ],
    )
    .await
    .unwrap_err();
    assert!(error.contains("mismatch"), "{error}");
    assert_eq!(
        fs::read(root.path().join("Assets/Variant.prefab")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn prefab_unknown_empty_element_schema_is_rejected_without_mutation() {
    let root = fixture();
    fs::write(root.path().join("Assets/Config.cs"),"using UnityEngine; public partial class Config:MonoBehaviour { public int amount; public Unknown[] empty; }").unwrap();
    let bytes = fs::read(root.path().join("Assets/Variant.prefab")).unwrap();
    let error = crate::unity_assets::execute(
        root.path(),
        json!({"action":"read_property","target":target("amount")}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("template_required"), "{error}");
    assert_eq!(
        fs::read(root.path().join("Assets/Variant.prefab")).unwrap(),
        bytes
    );
}

fn launch(editor: &str, root: &Path, method: &str) -> bool {
    let mut command = std::process::Command::new(editor);
    command
        .args(["-batchmode", "-nographics", "-projectPath"])
        .arg(root)
        .args([
            "-executeMethod",
            &format!("PropertyPrefabMatrix.PropertyPrefabMatrix.{method}"),
            "-logFile",
        ])
        .arg(root.join(format!("{method}.log")));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().unwrap();
    std::println!(
        "PROPERTY_PREFAB_EDITOR {}",
        json!({"root":root,"pid":child.id(),"method":method})
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.success();
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("owned Unity timed out: {}", root.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

#[tokio::test]
#[ignore = "requires LOCUS_PROPERTY_UNITY_EDITOR; retains an isolated Unity project"]
async fn complex_prefab_unity_roundtrip() {
    let editor =
        std::env::var("LOCUS_PROPERTY_UNITY_EDITOR").expect("set LOCUS_PROPERTY_UNITY_EDITOR");
    let root = std::path::PathBuf::from("E:/LocusTemp")
        .join(format!("property-prefab-{}", uuid::Uuid::new_v4().simple()));
    for dir in ["Assets/Editor", "Packages", "ProjectSettings"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/tests/unity-property");
    fs::copy(
        fixtures.join("PrefabMatrixComponent.cs"),
        root.join("Assets/PrefabMatrixComponent.cs"),
    )
    .unwrap();
    fs::copy(
        fixtures.join("PropertyPrefabMatrix.cs"),
        root.join("Assets/Editor/PropertyPrefabMatrix.cs"),
    )
    .unwrap();
    fs::write(
        root.join("Packages/manifest.json"),
        r#"{"dependencies":{"com.unity.modules.jsonserialize":"1.0.0"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("ProjectSettings/ProjectVersion.txt"),
        "m_EditorVersion: 2022.3.47f1\n",
    )
    .unwrap();
    assert!(launch(&editor, &root, "Seed"), "{}", root.display());
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("prefab-matrix.json")).unwrap()).unwrap();
    let mut reports = vec![];
    for case in manifest["cases"].as_array().unwrap() {
        let run = async {
            for check in case["checks"].as_array().unwrap() {
                let read = crate::unity_assets::execute(&root, json!({"action":"read_property","target":check["target"]})).await?;
                let expected: Value = serde_json::from_str(check["expectedJson"].as_str().unwrap()).unwrap();
                if read["value"] != expected { return Err(format!("read mismatch: {} != {expected}", read["value"])); }
            }
            let mut writes = vec![];
            for write in case["writes"].as_array().unwrap() {
                let mut probe = write["target"].clone();
                // Dependencies/revision describe the pre-batch file, including targets
                // that will only exist after an earlier structural request.
                probe["propertyPath"] = json!("amount");
                let before = crate::unity_assets::execute(&root, json!({"action":"read_property","target":probe})).await?;
                let value: Value = serde_json::from_str(write["valueJson"].as_str().unwrap()).unwrap();
                writes.push(json!({"target":write["target"],"value":value,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]}));
            }
            crate::unity_assets::execute(&root, json!({"action":"apply_properties","writes":writes,"resultMode":"summary","profile":true})).await
        }.await;
        let report = json!({"name":case["name"],"result":run});
        std::println!("PROPERTY_PREFAB_CASE {report}");
        reports.push(report);
    }
    fs::write(
        root.join("service-results.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    let verified = launch(&editor, &root, "Verify");
    let native = fs::read_to_string(root.join("prefab-matrix-results.json")).unwrap_or_default();
    std::println!("PROPERTY_PREFAB_NATIVE {native}");
    assert!(
        reports.iter().all(|r| r["result"].get("Ok").is_some()),
        "{reports:?}; {}",
        root.display()
    );
    assert!(verified, "{native}; {}", root.display());
}
