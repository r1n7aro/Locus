use super::*;
use crate::unity_serialized_property::property_tree::PropertyTreePath;

#[tokio::test]
async fn prefab_batch_reuses_phase_snapshots_but_respects_apply_and_revert_barriers() {
    let root = prefab_fixture();
    let before = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    let write = |value: Value| json!({"target":before["target"],"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"],"value":value});
    let mut writes = (0..128).map(|n| write(json!(n))).collect::<Vec<_>>();
    writes.push(write(json!({"action":"applyToSource","level":1})));
    writes.push(write(json!(999)));
    writes.push(write(json!({"action":"revert"})));
    let result = crate::unity_assets::execute(
        root.path(),
        json!({"action":"apply_properties","backend":"yaml","writes":writes,"profile":true}),
    )
    .await
    .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["value"] == 127 && r["saved"] == true));
    assert!(result["profile"]["effectiveBuilds"].as_u64().unwrap() < 10);
    assert_eq!(result["profile"]["treeBuilds"], 2);
    assert_eq!(result["profile"]["overrideValidationPasses"], 2);
    assert!(
        std::fs::read_to_string(root.path().join("Assets/Base.prefab"))
            .unwrap()
            .contains("amount: 127")
    );
}

#[tokio::test]
async fn source_type_evidence_changes_invalidate_prefab_reads_before_writes() {
    let root = prefab_fixture();
    let script = "using UnityEngine; public class Data : MonoBehaviour { public int amount; }";
    std::fs::write(root.path().join("Assets/Data.cs"), script).unwrap();
    std::fs::write(
        root.path().join("Assets/Data.cs.meta"),
        "guid: cccccccccccccccccccccccccccccccc\n",
    )
    .unwrap();
    std::fs::write(root.path().join("Assets/Base.prefab"),"--- !u!114 &10\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  amount: 7\n").unwrap();
    let before = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    assert!(before["dependencies"]["Assets/Data.cs"].is_string());
    let make_write = |value: Value| json!({"target":before["target"],"value":value,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]});
    let invalid=crate::unity_assets::execute(root.path(),json!({"action":"apply_properties","writes":[make_write(json!({"kind":"int64","value":"9223372036854775807"})),make_write(json!(1))]})).await.unwrap_err();
    assert!(invalid.contains("range"), "{invalid}");
    std::fs::write(
        root.path().join("Assets/Data.cs"),
        script.replace("int amount", "long amount"),
    )
    .unwrap();
    let bytes = std::fs::read(root.path().join("Assets/Variant.prefab")).unwrap();
    assert!(advanced_write(root.path(), &before, json!(9))
        .await
        .unwrap_err()
        .contains("stale_dependency"));
    assert_eq!(
        std::fs::read(root.path().join("Assets/Variant.prefab")).unwrap(),
        bytes
    );
}

#[cfg(windows)]
#[tokio::test]
async fn semantic_transaction_rejects_path_aliases_before_journaling() {
    let root = prefab_fixture();
    std::fs::write(root.path().join("Assets/Variant.prefab.meta"),"guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n").unwrap();
    let mut lower=prefab_request();lower["target"]["path"]=json!("Assets/variant.prefab");
    let lower=crate::unity_assets::execute(root.path(),lower).await.unwrap();
    assert!(lower["dependencies"]["Assets/variant.prefab.meta"].is_string());
    let before = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    let write = json!({"target":before["target"],"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"],"value":3});
    let mut alias = write.clone();
    alias["target"]["path"] = json!("Assets/variant.prefab");
    assert!(crate::unity_assets::execute(
        root.path(),
        json!({"action":"apply_properties","writes":[write,alias]})
    )
    .await
    .unwrap_err()
    .contains("path_alias"));
    assert!(!root.path().join("Library/Locus/AssetApi").exists());
}

// Kept out of routine CI: an actual service/transaction workload, not a timing
// assertion. Run with --ignored --nocapture to compare changes on one machine.
#[tokio::test]
#[ignore]
async fn prefab_batch_service_benchmark() {
    for count in [128, 1000] {
        let root = prefab_fixture();
        let mut base = String::from("--- !u!114 &10\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  amount: 7\n");
        let mut script=String::from("using UnityEngine; public class Data : MonoBehaviour { public int amount;");
        for index in 0..32 {
            base.push_str(&format!("  field{index}: {index}\n"));
            script.push_str(&format!(" public int field{index};"));
        }
        script.push('}');
        std::fs::write(root.path().join("Assets/Data.cs"),script).unwrap();
        std::fs::write(root.path().join("Assets/Base.prefab"), base).unwrap();
        let mut variant =
            std::fs::read_to_string(root.path().join("Assets/Variant.prefab")).unwrap();
        let first = variant.clone();
        for index in 1..8 {
            variant.push_str(&first.replace("&100", &format!("&{}", 100 + index)));
        }
        std::fs::write(root.path().join("Assets/Variant.prefab"), variant).unwrap();
        let before = crate::unity_assets::execute(root.path(), prefab_request())
            .await
            .unwrap();
        let writes=(0..count).map(|i|json!({"target":{"kind":"asset","path":"Assets/Variant.prefab","targetFileId":(10^(100+i%8)).to_string(),"propertyPath":format!("field{}",(i/8)%32)},"value":i,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]})).collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let result = crate::unity_assets::execute(
            root.path(),
            json!({"action":"apply_properties","backend":"yaml","writes":writes,"profile":true}),
        )
        .await
        .unwrap();
        std::println!(
            "PROPERTY_BENCH {}",
            json!({"writes":count,"objects":8,"fieldsPerObject":33,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"profile":result["profile"]})
        );
        assert_eq!(result["results"].as_array().unwrap().len(), count as usize);
        assert_eq!(result["results"][count as usize - 1]["value"], count - 1);
    }
}

#[tokio::test]
#[ignore]
async fn prefab_typed_batch_service_benchmark() {
    let root = prefab_fixture();
    for index in 0..128 {
        std::fs::write(
            root.path().join(format!("Assets/Unrelated{index}.cs")),
            format!("public class Unrelated{index} {{ public int value; }}"),
        )
        .unwrap();
        std::fs::write(
            root.path().join(format!("Assets/Unrelated{index}.cs.meta")),
            format!("guid: {index:032x}\n"),
        )
        .unwrap();
    }
    let mut script =
        String::from("using UnityEngine; public class Data : MonoBehaviour { public int amount;");
    let mut base=String::from("--- !u!114 &10\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  amount: 7\n");
    for index in 0..32 {
        script.push_str(&format!(" public int field{index};"));
        base.push_str(&format!("  field{index}: {index}\n"));
    }
    script.push('}');
    std::fs::write(root.path().join("Assets/Data.cs"), script).unwrap();
    std::fs::write(
        root.path().join("Assets/Data.cs.meta"),
        "guid: cccccccccccccccccccccccccccccccc\n",
    )
    .unwrap();
    std::fs::write(root.path().join("Assets/Base.prefab"), base).unwrap();
    let first = std::fs::read_to_string(root.path().join("Assets/Variant.prefab")).unwrap();
    let variant = (0..8)
        .map(|i| first.replace("&100", &format!("&{}", 100 + i)))
        .collect::<String>();
    std::fs::write(root.path().join("Assets/Variant.prefab"), variant).unwrap();
    let before = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    let writes=(0..1000).map(|i|json!({"target":{"kind":"asset","path":"Assets/Variant.prefab","targetFileId":(10^(100+i%8)).to_string(),"propertyPath":format!("field{}",(i/8)%32)},"value":i,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]})).collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let result = crate::unity_assets::execute(
        root.path(),
        json!({"action":"apply_properties","writes":writes,"profile":true}),
    )
    .await
    .unwrap();
    std::println!(
        "PROPERTY_BENCH {}",
        json!({"typed":true,"sourceFiles":129,"writes":1000,"objects":8,"fieldsPerObject":33,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"profile":result["profile"]})
    );
    assert_eq!(result["results"][999]["value"], 999);
    assert_eq!(result["profile"]["overrideValidationPasses"], 1);
}

fn prefab_fixture() -> tempfile::TempDir {
    let root = fixture();
    std::fs::write(
        root.path().join("Assets/Base.prefab"),
        "--- !u!114 &10\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  amount: 7\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("Assets/Base.prefab.meta"),
        "guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .unwrap();
    std::fs::write(root.path().join("Assets/Variant.prefab"),"--- !u!1001 &100\nPrefabInstance:\n  m_SourcePrefab: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}\n  m_Modification:\n    m_Modifications: []\n    m_RemovedComponents: []\n    m_RemovedGameObjects: []\n").unwrap();
    root
}
fn prefab_request() -> Value {
    json!({"action":"read_property","backend":"yaml","target":{"kind":"asset","path":"Assets/Variant.prefab","targetFileId":"110","propertyPath":"amount"}})
}
async fn advanced_write(root: &Path, read: &Value, value: Value) -> Result<Value, String> {
    crate::unity_assets::execute(root,json!({"action":"apply_properties","backend":"yaml","writes":[{"target":read["target"],"expectedRevision":read["revision"],"expectedDependencies":read["dependencies"],"value":value}]})).await
}
#[tokio::test]
async fn prefab_service_override_revert_apply_and_dependency_cas() {
    let root = prefab_fixture();
    let read = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    assert_eq!(read["value"], 7);
    assert_eq!(read["prefabLayers"].as_array().unwrap().len(), 1);
    let result = advanced_write(root.path(), &read, json!(31)).await.unwrap();
    let read = &result["results"][0];
    assert_eq!(read["value"], 31);
    assert_eq!(read["prefabOverride"], true);
    let result = advanced_write(root.path(), read, json!({"action":"revert"}))
        .await
        .unwrap();
    let read = &result["results"][0];
    assert_eq!(read["value"], 7);
    assert_eq!(read["prefabOverride"], false);
    let result = advanced_write(root.path(), read, json!(43)).await.unwrap();
    let result = advanced_write(
        root.path(),
        &result["results"][0],
        json!({"action":"applyToSource","level":1}),
    )
    .await
    .unwrap();
    assert_eq!(result["results"][0]["value"], 43);
    assert_eq!(result["results"][0]["prefabOverride"], false);
    let base = std::fs::read_to_string(root.path().join("Assets/Base.prefab")).unwrap();
    assert!(base.contains("amount: 43"));
    std::fs::write(
        root.path().join("Assets/Base.prefab"),
        base.replace("43", "44"),
    )
    .unwrap();
    let before = std::fs::read(root.path().join("Assets/Variant.prefab")).unwrap();
    assert!(
        advanced_write(root.path(), &result["results"][0], json!(55))
            .await
            .unwrap_err()
            .contains("stale_dependency")
    );
    assert_eq!(
        before,
        std::fs::read(root.path().join("Assets/Variant.prefab")).unwrap()
    );
}

#[tokio::test]
async fn discovery_returns_inherited_virtual_targets_and_agent_cache_tracks_sources() {
    let root = prefab_fixture();
    let mut request = prefab_request();
    request["action"] = json!("discover_property");
    request["target"]
        .as_object_mut()
        .unwrap()
        .remove("targetFileId");
    request["target"]["propertyPath"] = json!("");
    request["query"] = json!("amount");
    let found = crate::unity_assets::execute(root.path(), request)
        .await
        .unwrap();
    assert_eq!(found["matches"][0]["target"]["targetFileId"], "110");
    let full = root.path().join("Assets/Variant.prefab");
    let text = std::fs::read_to_string(&full).unwrap();
    let tree = YamlPropertyTree::parse(
        "Assets/Variant.prefab",
        &text,
        Some(root.path()),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        tree.read_target(110, "amount", 1, 10, 0).unwrap().value,
        json!(7)
    );
    crate::unity_serialized_property::property_tree::cache_yaml_property_tree(
        &full,
        std::sync::Arc::new(tree),
    );
    assert!(
        crate::unity_serialized_property::property_tree::cached_yaml_property_tree(&full).is_some()
    );
    std::fs::write(
        root.path().join("Assets/Base.prefab"),
        "--- !u!114 &10\nMonoBehaviour:\n  amount: 9\n",
    )
    .unwrap();
    assert!(
        crate::unity_serialized_property::property_tree::cached_yaml_property_tree(&full).is_none()
    );
}

#[tokio::test]
async fn nested_virtual_override_target_is_validated_in_storage() {
    let root = prefab_fixture();
    std::fs::write(
        root.path().join("Assets/Variant.prefab.meta"),
        "guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(root.path().join("Assets/Variant.prefab"))
        .unwrap()
        .replace("&100", "&200")
        .replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
    std::fs::write(root.path().join("Assets/Outer.prefab"), text).unwrap();
    let mut request = prefab_request();
    request["target"]["path"] = json!("Assets/Outer.prefab");
    request["target"]["targetFileId"] = json!((10 ^ 100 ^ 200).to_string());
    let before = crate::unity_assets::execute(root.path(), request)
        .await
        .unwrap();
    let after = advanced_write(root.path(), &before, json!(99))
        .await
        .unwrap();
    assert_eq!(after["results"][0]["value"], 99);
    let applied = advanced_write(
        root.path(),
        &after["results"][0],
        json!({"action":"applyToSource","level":2}),
    )
    .await
    .unwrap();
    assert_eq!(applied["results"][0]["value"], 99);
}

#[tokio::test]
async fn failed_late_semantic_write_preserves_all_source_and_target_bytes() {
    let root = prefab_fixture();
    let before = crate::unity_assets::execute(root.path(), prefab_request())
        .await
        .unwrap();
    let files = ["Assets/Base.prefab", "Assets/Variant.prefab"]
        .map(|p| (p, std::fs::read(root.path().join(p)).unwrap()));
    let valid = json!({"target":before["target"],"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"],"value":99});
    let mut invalid = valid.clone();
    invalid["value"] = json!({"arbitrary":"object"});
    let error = crate::unity_assets::execute(
        root.path(),
        json!({"action":"apply_properties","backend":"yaml","writes":[valid,invalid]}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("required") || error.contains("mismatch"));
    for (path, bytes) in files {
        assert_eq!(std::fs::read(root.path().join(path)).unwrap(), bytes);
    }
}
#[tokio::test]
async fn creation_service_requires_assignable_complete_source_and_assembly() {
    let root = fixture();
    std::fs::write(root.path().join("Assets/Data.cs"),"using System; using UnityEngine; public class Data : ScriptableObject { [SerializeReference] public Node node; [SerializeReference] public Node alias; } [Serializable] public class Node { public int amount; [SerializeReference] public Node next; } [Serializable] public class Other : Node { public bool enabled; }").unwrap();
    std::fs::write(
        root.path().join("Assets/Data.cs.meta"),
        "guid: cccccccccccccccccccccccccccccccc\n",
    )
    .unwrap();
    std::fs::write(root.path().join("Assets/Data.asset"),"--- !u!114 &11400000\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n  node: {rid: -2}\n  alias: {rid: -2}\n").unwrap();
    let before = read(root.path(), "node").await;
    let template = json!({"rootRid":"1","entries":[{"rid":"1","type":{"class":"Other","ns":"","asm":"Assembly-CSharp"},"data":{"amount":19,"next":{"rid":"1"},"enabled":true}}]});
    for bad in [
        json!({"class":"Missing","ns":"","asm":"Assembly-CSharp"}),
        json!({"class":"Other","ns":"","asm":"Wrong"}),
        json!({"class":"Data","ns":"","asm":"Assembly-CSharp"}),
    ] {
        let mut invalid = template.clone();
        invalid["entries"][0]["type"] = bad;
        assert!(advanced_write(
            root.path(),
            &before,
            json!({"action":"createManaged","template":invalid})
        )
        .await
        .is_err());
    }
    let result = advanced_write(
        root.path(),
        &before,
        json!({"action":"createManaged","template":template}),
    )
    .await
    .unwrap();
    assert_eq!(result["results"][0]["saved"], true);
    assert_eq!(read(root.path(), "node.next.amount").await["value"], 19);
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Assets")).unwrap();
    std::fs::write(root.path().join("Assets/Data.cs"),"using UnityEngine; using System; using System.Collections.Generic; public class Data : ScriptableObject { public int amount; public long wide; public List<int> values; [SerializeReference] public Test.Node node; [SerializeReference] public Test.Node alias; } namespace Test { [Serializable] public class Node { public int amount; [SerializeReference] public Node next; } }").unwrap();
    std::fs::write(root.path().join("Assets/Data.cs.meta"),"guid: cccccccccccccccccccccccccccccccc\n").unwrap();
    std::fs::write(root.path().join("Assets/Data.asset"), b"--- !u!114 &11400000\nMonoBehaviour:\n  amount: 7\n  wide: 9223372036854775807\n  values: [1, 2, 3]\n  node: {rid: 9007199254740993}\n  alias: {rid: 9007199254740993}\n  references:\n    version: 2\n    RefIds:\n    - rid: 9007199254740993\n      type: {class: Node, ns: Test, asm: Assembly-CSharp}\n      data:\n        amount: 23\n        next: {rid: 9007199254740993}\n").unwrap();
    let path=root.path().join("Assets/Data.asset");
    let bytes=std::fs::read_to_string(&path).unwrap().replace("MonoBehaviour:\n", "MonoBehaviour:\n  m_Script: {fileID: 11500000, guid: cccccccccccccccccccccccccccccccc, type: 3}\n");
    std::fs::write(path,bytes).unwrap();
    root
}
fn request(action: &str, path: &str) -> Value {
    json!({"action":action,"backend":"yaml","target":{"kind":"asset","path":"Assets/Data.asset","targetFileId":"11400000","propertyPath":path}})
}
async fn read(root: &Path, path: &str) -> Value {
    crate::unity_assets::execute(root, request("read_property", path))
        .await
        .unwrap()
}

#[tokio::test]
async fn property_service_and_agent_tree_share_graph_values_and_exact_ids() {
    let root = fixture();
    let view = read(root.path(), "node.next.amount").await;
    let text = std::fs::read_to_string(root.path().join("Assets/Data.asset")).unwrap();
    let tree =
        YamlPropertyTree::parse("Assets/Data.asset", &text, None, &Default::default()).unwrap();
    let path = PropertyTreePath::parse(
        root.path().to_str().unwrap(),
        "Assets/Data.asset/node/next/amount",
    )
    .unwrap();
    let agent = tree.read(&path, 2).unwrap();
    assert_eq!(view["value"], agent.value);
    assert_eq!(view["target"]["targetFileId"], "11400000");
    assert_eq!(
        read(root.path(), "wide").await["value"],
        "9223372036854775807"
    );
    let graph = read(root.path(), "node").await;
    assert_eq!(graph["value"]["rid"], "9007199254740993");
    assert_eq!(graph["children"][0]["propertyPath"], "node.amount");
    assert!(graph["children"][1]["canonicalPath"]
        .as_str()
        .is_some_and(|path| !path.is_empty()));
}

#[tokio::test]
async fn property_service_writes_graph_aliases_in_one_transaction_and_rejects_stale_versions() {
    let root = fixture();
    let before = read(root.path(), "node.amount").await;
    let write = json!({"target":before["target"],"value":91,"expectedRevision":before["revision"]});
    let input = json!({"action":"apply_properties","backend":"yaml","writes":[write]});
    let after = crate::unity_assets::execute(root.path(), input.clone())
        .await
        .unwrap();
    assert_eq!(after["results"][0]["value"], 91);
    assert_eq!(after["results"][0]["beforeSnapshot"]["value"], 23);
    assert_eq!(read(root.path(), "alias.next.amount").await["value"], 91);
    assert!(crate::unity_assets::execute(root.path(), input)
        .await
        .unwrap_err()
        .contains("stale_revision"));
}

#[tokio::test]
async fn property_service_pages_arrays_and_keeps_removed_targets_successful() {
    let root = fixture();
    let before = read(root.path(), "values").await;
    let mut page = request("read_property", "values");
    page["arrayOffset"] = json!(2);
    page["maxArrayItems"] = json!(1);
    let result = crate::unity_assets::execute(root.path(), page)
        .await
        .unwrap();
    assert_eq!(
        result["children"][0]["propertyPath"],
        "values.Array.data[2]"
    );
    assert_eq!(result["arraySize"], 3);
    assert_eq!(result["childrenTruncated"], false);
    let mut child = before["target"].clone();
    child["propertyPath"] = json!("values.Array.data[2]");
    let result=crate::unity_assets::execute(root.path(),json!({"action":"apply_properties","backend":"yaml","writes":[
        {"target":child,"value":9,"expectedRevision":before["revision"]},
        {"target":before["target"],"value":{"action":"resize","size":1},"expectedRevision":before["revision"]}
    ]})).await.unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["results"][0]["saved"], true);
    assert!(result["results"][0]["message"]
        .as_str()
        .unwrap()
        .contains("removed"));
}

#[tokio::test]
async fn property_service_rejects_editor_identity_and_unsupported_creation_before_mutation() {
    let root = fixture();
    let before = read(root.path(), "node").await;
    let mut invalid = request("read_property", "node");
    invalid["target"]["globalObjectId"] = json!("scene-instance");
    assert!(crate::unity_assets::execute(root.path(), invalid)
        .await
        .unwrap_err()
        .contains("unsupported_target"));
    let error=crate::unity_assets::execute(root.path(),json!({"action":"apply_properties","backend":"yaml","writes":[
        {"target":before["target"],"value":{"action":"setType","typeName":"Other"},"expectedRevision":before["revision"]}
    ]})).await.unwrap_err();
    assert!(error.contains("unsupported_command"));
    assert_eq!(
        read(root.path(), "node").await["revision"],
        before["revision"]
    );
}

#[tokio::test]
async fn property_service_and_agent_apply_the_same_source_proven_scalar_types() {
    let root = fixture();
    let guid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    std::fs::write(root.path().join("Assets/Typed.cs"),"using UnityEngine; public class Typed : ScriptableObject { public bool enabled; public float speed; public string text; }").unwrap();
    std::fs::write(
        root.path().join("Assets/Typed.cs.meta"),
        format!("fileFormatVersion: 2\nguid: {guid}\n"),
    )
    .unwrap();
    let text=format!("--- !u!114 &11400000\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {guid}, type: 3}}\n  enabled: 1\n  speed: 1\n  text: 123\n");
    std::fs::write(root.path().join("Assets/Data.asset"), &text).unwrap();
    let tree = YamlPropertyTree::parse(
        "Assets/Data.asset",
        &text,
        Some(root.path()),
        &std::collections::HashMap::from([(guid.into(), "Assets/Typed.cs".into())]),
    )
    .unwrap();
    for (field, kind, value) in [
        ("enabled", "Boolean", json!(true)),
        ("speed", "Float", json!(1.0)),
        ("text", "String", json!("123")),
    ] {
        let view = read(root.path(), field).await;
        let path = PropertyTreePath::parse(
            root.path().to_str().unwrap(),
            &format!("Assets/Data.asset/{field}"),
        )
        .unwrap();
        let agent = tree.read(&path, 1).unwrap();
        assert_eq!(view["valueType"], kind);
        assert_eq!(view["value"], value);
        assert_eq!(view["value"], agent.value);
        assert_eq!(view["valueType"], agent.value_type);
    }
}
