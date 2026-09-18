use super::*;
use std::fs;

const GUID: &str = "cccccccccccccccccccccccccccccccc";
fn fixture(script: &str, fields: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("Assets")).unwrap();
    fs::write(root.path().join("Assets/Config.cs"), script).unwrap();
    fs::write(
        root.path().join("Assets/Config.cs.meta"),
        format!("guid: {GUID}\n"),
    )
    .unwrap();
    fs::write(root.path().join("Assets/Data.asset"), format!("--- !u!114 &10\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {GUID}, type: 3}}\n{fields}")).unwrap();
    root
}
fn target(property: &str) -> Value {
    json!({"kind":"asset","path":"Assets/Data.asset","targetFileId":"10","propertyPath":property})
}
async fn read(root: &Path, property: &str) -> Value {
    crate::unity_assets::execute(
        root,
        json!({"action":"read_property","target":target(property)}),
    )
    .await
    .unwrap()
}
fn write(before: &Value, property: &str, value: Value, deps: bool) -> Value {
    let mut result =
        json!({"target":target(property),"value":value,"expectedRevision":before["revision"]});
    if deps {
        result["expectedDependencies"] = before
            .get("dependencies")
            .cloned()
            .unwrap_or_else(|| json!({"Assets/Data.asset":before["revision"]}));
    }
    result
}
async fn apply(root: &Path, writes: Vec<Value>) -> Result<Value, String> {
    crate::unity_assets::execute(
        root,
        json!({"action":"apply_properties","writes":writes,"profile":true}),
    )
    .await
}
fn template(class: &str) -> Value {
    json!({"action":"createManaged","template":{"rootRid":"1","entries":[{"rid":"1","type":{"class":class,"ns":"","asm":"Assembly-CSharp"},"data":{"amount":11}}]}})
}
const NESTED: &str = "using System; using UnityEngine; public class Config : ScriptableObject { [SerializeReference] public Node node; [Serializable] public class Node { public int amount; } }";

#[tokio::test]
async fn review_nested_managed_wire_name_is_accepted_and_noncanonical_names_rejected() {
    for class in ["Config/Node", "Config.Node", "Config+Node"] {
        let root = fixture(NESTED, "  node: {rid: -2}\n");
        let before = read(root.path(), "node").await;
        let original = fs::read(root.path().join("Assets/Data.asset")).unwrap();
        let result = apply(
            root.path(),
            vec![write(&before, "node", template(class), false)],
        )
        .await;
        if class == "Config/Node" {
            assert!(result.is_ok(), "canonical nested type rejected: {result:?}");
            assert_eq!(read(root.path(), "node.amount").await["value"], 11);
        } else {
            assert!(
                result.is_err(),
                "invalid wire identity was committed: {class}"
            );
            assert_eq!(
                fs::read(root.path().join("Assets/Data.asset")).unwrap(),
                original
            );
        }
    }
}

#[tokio::test]
async fn review_property_writes_require_type_evidence_before_any_mutation() {
    for deps in [false, true] {
        for script in [
            "using UnityEngine; public partial class Config : ScriptableObject { public int amount; }",
            "using UnityEngine; public class Config : ScriptableObject {\n#if UNITY_EDITOR\npublic int amount;\n#endif\n}",
        ] {
            let root = fixture(script, "  amount: 7\n");
            let before = read(root.path(), "amount").await;
            let original = fs::read(root.path().join("Assets/Data.asset")).unwrap();
            let result = apply(root.path(), vec![write(&before, "amount", json!(2.5), deps)]).await;
            assert!(result.is_err(), "unverified int accepted fractional input");
            assert_eq!(fs::read(root.path().join("Assets/Data.asset")).unwrap(), original);
        }
    }
}

#[tokio::test]
async fn review_native_templates_require_a_complete_owned_transform() {
    let root = fixture(
        "using UnityEngine; public class Config : ScriptableObject { public int amount; }",
        "  amount: 7\n",
    );
    let before = read(root.path(), "amount").await;
    let original = fs::read(root.path().join("Assets/Data.asset")).unwrap();
    for data in [
        json!({"m_Name":"Invalid"}),
        json!({"serializedVersion":6,"m_Name":"Invalid","m_Component":[],"m_IsActive":1}),
    ] {
        let command = json!({"action":"editObjects","add":[{"id":"20","classId":"1","rootType":"GameObject","data":data}]});
        assert!(
            apply(root.path(), vec![write(&before, "amount", command, false)])
                .await
                .is_err(),
            "incomplete GameObject was persisted"
        );
        assert_eq!(
            fs::read(root.path().join("Assets/Data.asset")).unwrap(),
            original
        );
    }
}

#[tokio::test]
async fn review_native_script_template_must_match_component_or_asset_ownership() {
    let root = fixture(
        "using UnityEngine; public class Config : MonoBehaviour { public int amount; }",
        "  amount: 7\n",
    );
    let before = read(root.path(), "amount").await;
    let data = json!({"m_ObjectHideFlags":0,"m_CorrespondingSourceObject":{"fileID":"0"},"m_PrefabInstance":{"fileID":"0"},"m_PrefabAsset":{"fileID":"0"},"m_GameObject":{"fileID":"0"},"m_Enabled":1,"m_EditorHideFlags":0,"m_Script":{"fileID":"11500000","guid":GUID,"type":3},"m_Name":"InvalidAsset","m_EditorClassIdentifier":"","amount":8});
    let command = json!({"action":"editObjects","add":[{"id":"20","classId":"114","rootType":"MonoBehaviour","data":data}]});
    let result = apply(root.path(), vec![write(&before, "amount", command, true)]).await;
    assert!(
        result.is_err(),
        "MonoBehaviour script was accepted as a ScriptableObject asset"
    );
    assert_eq!(
        read(root.path(), "amount").await["revision"],
        before["revision"]
    );
}

#[tokio::test]
async fn review_nested_list_insert_response_has_the_same_types_as_a_fresh_read() {
    for deps in [false, true] {
        let root = fixture("using System; using System.Collections.Generic; using UnityEngine; [Serializable] public class Item { public bool flag; public float weight; public string label; } public class Config : ScriptableObject { public List<Item> items; }", "  items:\n  - flag: 1\n    weight: 1\n    label: first\n");
        let before = read(root.path(), "items").await;
        let inserted =
            json!({"action":"insert","index":1,"value":{"flag":false,"weight":2,"label":"007"}});
        let result = apply(root.path(), vec![write(&before, "items", inserted, deps)])
            .await
            .unwrap();
        let fresh = read(root.path(), "items").await;
        for name in ["flag", "weight", "label"] {
            let find = |tree: &Value| {
                tree["children"][1]["children"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["name"] == name)
                    .map(|n| (n["valueType"].clone(), n["value"].clone()))
                    .unwrap()
            };
            assert_eq!(
                find(&result["results"][0]),
                find(&fresh),
                "{name}, dependencies={deps}"
            );
        }
    }
}

#[tokio::test]
async fn review_materialized_batches_rebuild_once_per_phase_and_validate_overwritten_inputs() {
    let root = fixture(
        "using UnityEngine; public class Config : ScriptableObject { public int amount; }",
        "  amount: 7\n",
    );
    let before = read(root.path(), "amount").await;
    let result = apply(
        root.path(),
        (0..128)
            .map(|n| write(&before, "amount", json!(n), true))
            .collect(),
    )
    .await
    .unwrap();
    assert_eq!(result["results"][127]["value"], 127);
    assert!(
        result["profile"]["effectiveBuilds"].as_u64().unwrap() <= 2,
        "{:?}",
        result["profile"]
    );
    let before = read(root.path(), "amount").await;
    let original = fs::read(root.path().join("Assets/Data.asset")).unwrap();
    assert!(apply(
        root.path(),
        vec![
            write(&before, "amount", json!(2.5), true),
            write(&before, "amount", json!(1), true)
        ]
    )
    .await
    .is_err());
    assert_eq!(
        fs::read(root.path().join("Assets/Data.asset")).unwrap(),
        original
    );
}

#[tokio::test]
async fn review_creation_dependencies_exclude_unrelated_sources_and_are_reusable() {
    let root = fixture("using System; using UnityEngine; public class Config : ScriptableObject { [SerializeReference] public Node node; } [Serializable] public class Node { public int amount; }", "  node: {rid: -2}\n");
    for n in 0..2050 {
        fs::write(
            root.path().join(format!("Assets/Unused{n}.cs")),
            format!("public class Unused{n} {{ public int amount; }}"),
        )
        .unwrap();
        fs::write(
            root.path().join(format!("Assets/Unused{n}.cs.meta")),
            format!("guid: {n:032x}\n"),
        )
        .unwrap();
    }
    let before = read(root.path(), "node").await;
    let result = apply(
        root.path(),
        vec![write(&before, "node", template("Node"), false)],
    )
    .await
    .unwrap();
    let after = &result["results"][0];
    assert!(
        after["dependencies"].as_object().unwrap().len() < 10,
        "unrelated source files captured: {}",
        after["dependencies"].as_object().unwrap().len()
    );
    fs::write(
        root.path().join("Assets/Unused0.cs"),
        "public class Unused0 { public string changed; }",
    )
    .unwrap();
    let result = apply(
        root.path(),
        vec![write(after, "node.amount", json!(12), true)],
    )
    .await
    .unwrap();
    assert_eq!(result["results"][0]["value"], 12);
}

#[tokio::test]
async fn review_materialized_ordering_handles_nested_insert_move_and_aliases() {
    let root=fixture("using System; using System.Collections.Generic; using UnityEngine; public class Config : ScriptableObject { public List<Item> items; [SerializeReference] public Node node; [SerializeReference] public Node alias; } [Serializable] public class Item { public bool flag; public float weight; } [Serializable] public class Node { public int amount; [SerializeReference] public Node next; }",
        "  items:\n  - flag: 1\n    weight: 1\n  node: {rid: 1}\n  alias: {rid: 2}\n  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: Node, ns: , asm: Assembly-CSharp}\n      data:\n        amount: 1\n        next: {rid: -2}\n    - rid: 2\n      type: {class: Node, ns: , asm: Assembly-CSharp}\n      data:\n        amount: 2\n        next: {rid: -2}\n");
    let before = read(root.path(), "items").await;
    let requests = [
        (
            "items",
            json!({"action":"insert","index":1,"value":{"flag":false,"weight":2}}),
        ),
        ("items.Array.data[1].weight", json!(2.5)),
        ("items", json!({"action":"move","index":1,"toIndex":0})),
        ("items.Array.data[0].flag", json!(true)),
        ("node", json!({"rid":"2"})),
        ("node.amount", json!(91)),
    ];
    let result = apply(
        root.path(),
        requests
            .into_iter()
            .map(|(p, v)| write(&before, p, v, true))
            .collect(),
    )
    .await
    .unwrap();
    assert_eq!(read(root.path(), "alias.amount").await["value"], 91);
    assert_eq!(
        read(root.path(), "items.Array.data[0].weight").await["value"],
        2.5
    );
    assert_eq!(
        read(root.path(), "items.Array.data[0].flag").await["value"],
        true
    );
    assert_eq!(result["profile"]["materializedCompilePasses"], 1);
}

#[tokio::test]
async fn review_summary_receipt_skips_trees_and_retains_transaction_guards() {
    let root = fixture(
        "using UnityEngine; public class Config : ScriptableObject { public int amount; }",
        "  amount: 7\n",
    );
    let before = read(root.path(), "amount").await;
    let request = json!({"action":"apply_properties","resultMode":"summary","profile":true,"writes":(0..128).map(|n|write(&before,"amount",json!(n),true)).collect::<Vec<_>>()});
    let result = crate::unity_assets::execute(root.path(), request.clone())
        .await
        .unwrap();
    assert_eq!(result["writesApplied"], 128);
    assert_eq!(result["assets"].as_array().unwrap().len(), 1);
    assert!(result.get("results").is_none());
    assert_eq!(result["profile"]["treeBuilds"], 0);
    assert_eq!(result["profile"]["readProjections"], 0);
    let fresh = read(root.path(), "amount").await;
    assert_eq!(fresh["value"], 127);
    assert_eq!(fresh["revision"], result["assets"][0]["revision"]);
    assert_eq!(fresh["dependencies"], result["assets"][0]["dependencies"]);
    assert!(crate::unity_assets::execute(root.path(), request)
        .await
        .unwrap_err()
        .contains("stale_revision"));
    let before_bytes = fs::read(root.path().join("Assets/Data.asset")).unwrap();
    let invalid = json!({"action":"apply_properties","resultMode":"summary","writes":[write(&fresh,"amount",json!(1),true),write(&fresh,"amount",json!(2.5),true)]});
    assert!(crate::unity_assets::execute(root.path(), invalid)
        .await
        .is_err());
    assert_eq!(
        fs::read(root.path().join("Assets/Data.asset")).unwrap(),
        before_bytes
    );
}

#[tokio::test]
async fn review_interleaved_materialized_files_compile_once_each() {
    let root = fixture(
        "using UnityEngine; public class Config : ScriptableObject { public int amount; }",
        "  amount: 7\n",
    );
    fs::copy(
        root.path().join("Assets/Data.asset"),
        root.path().join("Assets/Other.asset"),
    )
    .unwrap();
    let before = read(root.path(), "amount").await;
    let other=crate::unity_assets::execute(root.path(),json!({"action":"read_property","target":{"kind":"asset","path":"Assets/Other.asset","targetFileId":"10","propertyPath":"amount"}})).await.unwrap();
    let writes=(0..128).map(|n|{let source=if n%2==0 {&before}else{&other};json!({"target":source["target"],"value":n,"expectedRevision":source["revision"],"expectedDependencies":source["dependencies"]})}).collect::<Vec<_>>();
    let result = apply(root.path(), writes).await.unwrap();
    assert_eq!(result["profile"]["materializedCompilePasses"], 2);
    assert_eq!(result["profile"]["effectiveBuilds"], 4);
    assert_eq!(result["results"][126]["value"], 126);
    assert_eq!(result["results"][127]["value"], 127);
}

#[tokio::test]
#[ignore = "scale matrix; run explicitly with --ignored --nocapture"]
async fn review_service_performance_matrix() {
    // Replay the review's exact 15,940-byte fixture/request shape. Keep this
    // separate from the fully typed scale matrix below for a fair before/after.
    for summary in [false, true] {
        let mut fields = String::from("  amount: 7\n  items:\n  - flag: 1\n    label: first\n");
        for n in 0..1000 {
            fields.push_str(&format!("  extra{n}: {n}\n"));
        }
        let root=fixture("using System; using System.Collections.Generic; using UnityEngine; [Serializable] public class Item { public bool flag; public string label; } public class Config : ScriptableObject { public int amount; public List<Item> items; }",&fields);
        let bytes = fs::read(root.path().join("Assets/Data.asset"))
            .unwrap()
            .len();
        assert_eq!(bytes, 15940);
        let before = read(root.path(), "amount").await;
        let writes=(0..128).map(|n|json!({"target":target("amount"),"value":n,"expectedRevision":before["revision"],"expectedDependencies":{"Assets/Data.asset":before["revision"]}})).collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let result=crate::unity_assets::execute(root.path(),json!({"action":"apply_properties","writes":writes,"profile":true,"resultMode":if summary{"summary"}else{"full"}})).await.unwrap();
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        std::println!(
            "PROPERTY_BASELINE_REPLAY {}",
            json!({"assetBytes":bytes,"writes":128,"summary":summary,"elapsedMs":elapsed,"profile":result["profile"]})
        );
        assert_eq!(read(root.path(), "amount").await["value"], 127);
    }
    for (fields, count, distinct) in [
        (1, 1000, false),
        (1000, 128, false),
        (1000, 1000, true),
        (1000, 10000, true),
        (10000, 1000, true),
    ] {
        for summary in [false, true] {
            let mut script =
                String::from("using UnityEngine; public class Config : ScriptableObject {");
            let mut body = String::new();
            for n in 0..fields {
                script.push_str(&format!(" public int field{n};"));
                body.push_str(&format!("  field{n}: {n}\n"));
            }
            script.push('}');
            let root = fixture(&script, &body);
            let before = read(root.path(), "field0").await;
            let writes = (0..count)
                .map(|n| {
                    write(
                        &before,
                        &format!("field{}", if distinct { n % fields } else { 0 }),
                        json!(n + fields),
                        true,
                    )
                })
                .collect::<Vec<_>>();
            let started = std::time::Instant::now();
            let result=crate::unity_assets::execute(root.path(),json!({"action":"apply_properties","writes":writes,"profile":true,"resultMode":if summary{"summary"}else{"full"}})).await.unwrap();
            let response_bytes = serde_json::to_vec(&result).unwrap().len();
            std::println!(
                "PROPERTY_SCALE {}",
                json!({"fields":fields,"writes":count,"distinct":distinct,"summary":summary,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"profile":result["profile"],"responseBytes":response_bytes})
            );
            assert_eq!(
                read(
                    root.path(),
                    &format!("field{}", if distinct { (count - 1) % fields } else { 0 })
                )
                .await["value"],
                count - 1 + fields
            );
            assert_eq!(result["profile"]["materializedCompilePasses"], 1);
            assert_eq!(
                result["profile"]["effectiveBuilds"],
                if summary { 1 } else { 2 }
            );
        }
    }
}

// Real Editor fixture -> closed Editor -> production Property API and storage ->
// real Editor reload and SerializedObject comparison. No test-only write bypass.
#[tokio::test]
#[ignore = "requires LOCUS_PROPERTY_UNITY_EDITOR; creates a retained isolated Unity project"]
async fn review_unity_service_roundtrip() {
    let editor =
        std::env::var("LOCUS_PROPERTY_UNITY_EDITOR").expect("set LOCUS_PROPERTY_UNITY_EDITOR");
    let root = std::path::PathBuf::from("E:/LocusTemp").join(format!(
        "property-service-{}",
        uuid::Uuid::new_v4().simple()
    ));
    for directory in ["Assets/Editor", "Packages", "ProjectSettings"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/tests/unity-property");
    for file in ["ServiceData.cs", "ServiceUnverified.cs"] {
        fs::copy(fixtures.join(file), root.join("Assets").join(file)).unwrap();
    }
    fs::copy(
        fixtures.join("PropertyServiceReview.cs"),
        root.join("Assets/Editor/PropertyServiceReview.cs"),
    )
    .unwrap();
    fs::write(
        root.join("Packages/manifest.json"),
        r#"{"dependencies":{"com.unity.modules.jsonserialize":"1.0.0"}}"#,
    )
    .unwrap();
    // The Editor writes its own exact version during the seed import.
    fs::write(
        root.join("ProjectSettings/ProjectVersion.txt"),
        "m_EditorVersion: 2022.3.47f1\n",
    )
    .unwrap();
    let launch = |method: &str| {
        let mut command = std::process::Command::new(&editor);
        command
            .args(["-batchmode", "-nographics", "-projectPath"])
            .arg(&root)
            .args(["-executeMethod", method, "-logFile"])
            .arg(root.join(if method.ends_with("Seed") {
                "seed.log"
            } else {
                "verify.log"
            }));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().unwrap();
        std::println!(
            "PROPERTY_SERVICE_EDITOR {}",
            json!({"root":root,"pid":child.id(),"method":method})
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status.success();
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("owned Unity test timed out: {}", root.display());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    };
    assert!(
        launch("PropertyServiceReview.PropertyServiceReview.Seed"),
        "{}",
        root.display()
    );
    let read_asset = |path: &str, property: &str| json!({"action":"read_property","target":{"kind":"asset","path":path,"propertyPath":property}});
    let before = crate::unity_assets::execute(&root, read_asset("Assets/Data.asset", "node"))
        .await
        .unwrap();
    let make = |property: &str, value: Value| {
        let mut t = before["target"].clone();
        t["propertyPath"] = json!(property);
        json!({"target":t,"value":value,"expectedRevision":before["revision"],"expectedDependencies":before["dependencies"]})
    };
    let mut writes = vec![
        make(
            "node",
            json!({"action":"createManaged","template":{"rootRid":"1","entries":[{"rid":"1","type":{"class":"ServiceData/Node","ns":"PropertyServiceReview","asm":"Assembly-CSharp"},"data":{"amount":11,"next":{"rid":"1"}}}]}}),
        ),
        make("node.next.amount", json!(73)),
        make(
            "groups.Array.data[0].items",
            json!({"action":"insert","index":1,"value":{"flag":false,"weight":2.5,"label":"007"}}),
        ),
        make(
            "groups.Array.data[0].items.Array.data[1].flag",
            json!(false),
        ),
    ];
    writes.extend((0..500).map(|n| make("amount", json!(n))));
    let result = apply(&root, writes).await;
    fs::write(
        root.join("service-response.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    let unverified =
        crate::unity_assets::execute(&root, read_asset("Assets/Unverified.asset", "amount"))
            .await
            .unwrap();
    let rejected=apply(&root,vec![json!({"target":unverified["target"],"value":2.5,"expectedRevision":unverified["revision"],"expectedDependencies":unverified["dependencies"]})]).await;
    fs::write(
        root.join("unverified-response.json"),
        serde_json::to_vec_pretty(&rejected).unwrap(),
    )
    .unwrap();
    // Use actual Unity-generated native templates, remapping all ownership links
    // and committing the parent/child update in the same production transaction.
    let bytes = fs::read(root.join("Assets/Structure.prefab")).unwrap();
    let model = crate::unity_asset_core::authoring::AuthoringAsset::new(&bytes, Default::default())
        .unwrap();
    let parent_go = model.objects.values().find(|o| o.class_id == "1").unwrap();
    let parent_transform = model.objects.values().find(|o| o.class_id == "4").unwrap();
    let mut added_go = parent_go.clone();
    added_go.id = "901".into();
    added_go.data["m_Name"] = json!("AddedByYaml");
    added_go.data["m_Component"] = json!([{"component":{"fileID":"902"}}]);
    let mut added_transform = parent_transform.clone();
    added_transform.id = "902".into();
    added_transform.data["m_GameObject"] = json!({"fileID":"901"});
    added_transform.data["m_Father"] = json!({"fileID":parent_transform.id});
    added_transform.data["m_Children"] = json!([]);
    let native_target = json!({"kind":"asset","path":"Assets/Structure.prefab","targetFileId":parent_go.id,"propertyPath":"m_Name"});
    let native_before = crate::unity_assets::execute(
        &root,
        json!({"action":"read_property","target":native_target}),
    )
    .await
    .unwrap();
    let topology=apply(&root,vec![json!({"target":native_target,"expectedRevision":native_before["revision"],"expectedDependencies":native_before["dependencies"],"value":{"action":"editObjects","add":[added_go,added_transform],"updates":[{"objectId":parent_transform.id,"propertyPath":"m_Children","value":[{"fileID":"902"}]}]}})]).await;
    fs::write(
        root.join("topology-response.json"),
        serde_json::to_vec_pretty(&topology).unwrap(),
    )
    .unwrap();
    let reloaded = launch("PropertyServiceReview.PropertyServiceReview.Verify");
    let native = fs::read_to_string(root.join("service-review-results.json")).unwrap_or_default();
    std::println!("PROPERTY_SERVICE_NATIVE {native}");
    assert!(
        result.is_ok(),
        "service failed: {result:?}; {}",
        root.display()
    );
    assert!(
        rejected.is_err(),
        "unverified write was accepted; {}",
        root.display()
    );
    assert!(
        topology.is_ok(),
        "native templates failed: {topology:?}; {}",
        root.display()
    );
    assert!(reloaded, "{native}; {}", root.display());
}
