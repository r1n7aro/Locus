use super::*;
use crate::unity_asset_core::prefab::PrefabGraph;

#[test]
fn stripped_index_is_authoritative_and_duplicate_aliases_are_rejected() {
    let mut graph = graph();
    let bytes = String::from_utf8(graph.files["variant"].original.clone())
        .unwrap()
        .replace("&100", "&10");
    let stripped="--- !u!114 &77 stripped\nMonoBehaviour:\n  m_CorrespondingSourceObject: {fileID: 10, guid: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA, type: 3}\n  m_PrefabInstance: {fileID: 10}\n  m_PrefabAsset: {fileID: 0}\n";
    graph
        .files
        .insert("variant".into(), asset(&(bytes.clone() + stripped)));
    assert_eq!(
        graph.effective("variant").unwrap()["77"].object.data["amount"],
        7
    );
    graph.files.insert(
        "variant".into(),
        asset(&(bytes + stripped + &stripped.replace("&77", "&78"))),
    );
    assert!(graph
        .effective("variant")
        .unwrap_err()
        .contains("ambiguous_stripped_identity"));
}

#[test]
fn override_batch_is_atomic_validates_overwritten_values_and_coalesces_keys() {
    use crate::unity_asset_core::prefab::OverrideEdit;
    let mut graph = graph();
    let id = (10 ^ 100 ^ 200).to_string();
    let layer = graph.effective("scene").unwrap()[&id].layers[0].clone();
    graph
        .override_value(&layer, "amount", Some(json!(19)))
        .unwrap();
    let before = graph.files["scene"].render(false).unwrap();
    let edits = vec![
        OverrideEdit {
            layer: layer.clone(),
            property: "label".into(),
            value: Some(json!("edited")),
        },
        OverrideEdit {
            layer: layer.clone(),
            property: "amount".into(),
            value: Some(json!({"invalid":true})),
        },
        OverrideEdit {
            layer: layer.clone(),
            property: "amount".into(),
            value: Some(json!(20)),
        },
    ];
    assert!(graph.override_values(&edits).is_err());
    assert_eq!(graph.files["scene"].render(false).unwrap(), before);
    let edits = (0..1000)
        .map(|n| OverrideEdit {
            layer: layer.clone(),
            property: "amount".into(),
            value: Some(json!(n)),
        })
        .collect::<Vec<_>>();
    graph.override_values(&edits).unwrap();
    assert_eq!(
        graph.effective("scene").unwrap()[&id].object.data["amount"],
        999
    );
    assert_eq!(
        graph.files["scene"].objects["200"].data["m_Modification"]["m_Modifications"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unquoted_unity_override_strings_keep_lexical_identity() {
    for value in [
        "007",
        "1e3",
        "null",
        "true",
        "00123456789012345678901234567890",
        "",
    ] {
        let mut graph = graph();
        let source = graph.files["variant"].original.clone();
        let source=String::from_utf8(source).unwrap().replace("m_Modifications: []",&format!("m_Modifications:\n    - target: {{fileID: 10, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}}\n      propertyPath: label\n      value: {value}\n      objectReference: {{fileID: 0}}"));
        graph.files.insert("variant".into(), asset(&source));
        assert_eq!(
            graph.effective("variant").unwrap()[&(10 ^ 100).to_string()]
                .object
                .data["label"],
            value
        );
    }
}

#[test]
fn prefab_guids_are_case_insensitive_for_source_overrides_and_revert() {
    let mut graph = graph();
    let file = graph.files.get_mut("variant").unwrap();
    file.objects.get_mut("100").unwrap().data["m_SourcePrefab"]["guid"] =
        json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let id = (10 ^ 100).to_string();
    let layer = graph.effective("variant").unwrap()[&id].layers[0].clone();
    graph
        .override_value(&layer, "amount", Some(json!(91)))
        .unwrap();
    graph
        .files
        .get_mut("variant")
        .unwrap()
        .objects
        .get_mut("100")
        .unwrap()
        .data["m_Modification"]["m_Modifications"][0]["target"]["guid"] =
        json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(
        graph.effective("variant").unwrap()[&id].object.data["amount"],
        91
    );
    graph.override_value(&layer, "amount", None).unwrap();
    assert_eq!(
        graph.effective("variant").unwrap()[&id].object.data["amount"],
        7
    );
}

#[test]
fn small_integer_source_promotes_exact_override_before_apply() {
    let mut graph = graph();
    let id = (10 ^ 100).to_string();
    let layer = graph.effective("variant").unwrap()[&id].layers[0].clone();
    let value = json!({"kind":"int64","value":"9223372036854775807"});
    graph
        .override_value(&layer, "amount", Some(value.clone()))
        .unwrap();
    let actual = graph.effective("variant").unwrap()[&id].object.data["amount"].clone();
    assert_eq!(actual, value);
    graph
        .files
        .get_mut("base")
        .unwrap()
        .set("10", "amount", actual)
        .unwrap();
    graph.files["base"].render(true).unwrap();
}
fn asset(text: &str) -> AuthoringAsset {
    AuthoringAsset::new(text.as_bytes(), Default::default()).unwrap()
}
fn graph() -> PrefabGraph {
    let base = asset("--- !u!114 &10\nMonoBehaviour:\n  amount: 7\n  label: old\n");
    fn instance(id: &str, guid: &str) -> AuthoringAsset {
        asset(&format!("--- !u!1001 &{id}\nPrefabInstance:\n  m_SourcePrefab: {{fileID: 100100000, guid: {guid}, type: 3}}\n  m_Modification:\n    m_Modifications: []\n    m_RemovedComponents: []\n    m_RemovedGameObjects: []\n"))
    }
    PrefabGraph {
        array_templates: Default::default(),
        files: BTreeMap::from([
            ("base".into(), base),
            (
                "variant".into(),
                instance("100", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            (
                "scene".into(),
                instance("200", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            ),
        ]),
        guids: BTreeMap::from([
            ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(), "base".into()),
            ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(), "variant".into()),
        ]),
    }
}

#[test]
fn prefab_array_sizes_precede_elements_and_nested_sizes() {
    let mut g = graph();
    g.files
        .get_mut("base")
        .unwrap()
        .objects
        .get_mut("10")
        .unwrap()
        .data["items"] = json!([{"label":"first","values":[1,2]}]);
    let target = json!({"fileID":"10","guid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","type":3});
    // Native override ordering is not an evaluation order; size can follow elements.
    g.files
        .get_mut("variant")
        .unwrap()
        .objects
        .get_mut("100")
        .unwrap()
        .data["m_Modification"]["m_Modifications"] = json!([
        {"target":target,"propertyPath":"items.Array.data[1].values.Array.data[2]","value":"9"},
        {"target":target,"propertyPath":"items.Array.data[1].label","value":"007"},
        {"target":target,"propertyPath":"items.Array.data[1].values.Array.size","value":"3"},
        {"target":target,"propertyPath":"items.Array.size","value":"2"}
    ]);
    let projected = g.effective("scene").unwrap();
    assert_eq!(
        projected[&(10 ^ 100 ^ 200).to_string()].object.data["items"],
        json!([
            {"label":"first","values":[1,2]}, {"label":"007","values":[1,2,9]}
        ])
    );
}

#[test]
fn prefab_added_components_and_children_project_owned_links() {
    let mut g = graph();
    g.files.insert("base".into(),asset("--- !u!1 &10\nGameObject:\n  m_Component: [{component: {fileID: 11}}]\n--- !u!4 &11\nTransform:\n  m_GameObject: {fileID: 10}\n  m_Father: {fileID: 0}\n  m_Children: []\n"));
    let file = g.files.get_mut("variant").unwrap();
    for (id, class, root, data) in [
        (
            "70",
            "114",
            "MonoBehaviour",
            json!({"m_GameObject":{"fileID":"110"},"amount":5}),
        ),
        (
            "71",
            "1",
            "GameObject",
            json!({"m_Component":[{"component":{"fileID":"72"}}]}),
        ),
        (
            "72",
            "4",
            "Transform",
            json!({"m_GameObject":{"fileID":"71"},"m_Father":{"fileID":"111"},"m_Children":[]}),
        ),
    ] {
        file.objects.insert(
            id.into(),
            ObjectData {
                id: id.into(),
                class_id: class.into(),
                root_type: root.into(),
                data,
                stripped: false,
            },
        );
    }
    let m = &mut file.objects.get_mut("100").unwrap().data["m_Modification"];
    m["m_AddedComponents"] = json!([{"targetCorrespondingSourceObject":{"fileID":"10","guid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","type":3},"insertIndex":-1,"addedObject":{"fileID":"70"}}]);
    m["m_AddedGameObjects"] = json!([{"targetCorrespondingSourceObject":{"fileID":"11","guid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","type":3},"insertIndex":-1,"addedObject":{"fileID":"72"}}]);
    let projected = g.effective("scene").unwrap();
    assert_eq!(
        projected[&(10 ^ 100 ^ 200).to_string()].object.data["m_Component"][1]["component"]
            ["fileID"],
        (70 ^ 200).to_string()
    );
    assert_eq!(
        projected[&(11 ^ 100 ^ 200).to_string()].object.data["m_Children"][0]["fileID"],
        (72 ^ 200).to_string()
    );
}

#[test]
fn prefab_managed_aliases_use_registry_wire_identity_and_host_scope() {
    let mut g = graph();
    g.files.insert("base".into(), managed());
    let id = (10 ^ 100).to_string();
    let effective = g.effective("variant").unwrap();
    let data = &effective[&id].object.data;
    assert_eq!(
        crate::unity_asset_core::prefab::wire_path(data, "alias.next.amount").unwrap(),
        "managedReferences[1].amount"
    );
    g.override_value(
        &effective[&id].layers[0],
        "managedReferences[1].amount",
        Some(json!(88)),
    )
    .unwrap();
    let effective = g.effective("scene").unwrap();
    assert_eq!(
        crate::unity_asset_core::prefab::value_at(
            &effective[&(10 ^ 100 ^ 200).to_string()].object.data,
            "alias.next.next.amount"
        )
        .unwrap(),
        &json!(88)
    );
    assert!(
        crate::unity_asset_core::prefab::value_at(data, "managedReferences[99].amount").is_err()
    );
}

#[test]
fn prefab_array_replacement_drops_stale_elements_but_preserves_siblings() {
    let mut g = graph();
    g.files
        .get_mut("base")
        .unwrap()
        .objects
        .get_mut("10")
        .unwrap()
        .data["values"] = json!([1, 2, 3]);
    let layer = g.effective("variant").unwrap()["110"].layers[0].clone();
    g.override_value(&layer, "values.Array.data[2]", Some(json!(99)))
        .unwrap();
    g.override_value(&layer, "amount", Some(json!(17))).unwrap();
    g.override_array(&layer, "values", &json!([8])).unwrap();
    let e = g.effective("variant").unwrap();
    assert_eq!(e["110"].object.data["values"], json!([8]));
    assert_eq!(e["110"].object.data["amount"], 17);
    let bytes = g.files["variant"].render(false).unwrap();
    assert!(!String::from_utf8(bytes.clone())
        .unwrap()
        .contains("data[2]"));
    assert!(g
        .override_array(&layer, "values", &json!([{"rid":"1"}]))
        .is_err());
    assert_eq!(g.files["variant"].render(false).unwrap(), bytes);
    g.revert_subtree(&layer, "values").unwrap();
    assert_eq!(
        g.effective("variant").unwrap()["110"].object.data["values"],
        json!([1, 2, 3])
    );
    assert_eq!(
        g.effective("variant").unwrap()["110"].object.data["amount"],
        17
    );
}

#[test]
fn prefab_added_links_reject_wrong_scope_and_missing_object() {
    let mut g = graph();
    let original = g.files["variant"].objects["100"].clone();
    for record in [
        json!({"targetCorrespondingSourceObject":{"fileID":"10","guid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"insertIndex":-1,"addedObject":{"fileID":"77"}}),
        json!({"targetCorrespondingSourceObject":{"fileID":"10","guid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"insertIndex":-1,"addedObject":{"fileID":"77"}}),
    ] {
        g.files
            .get_mut("variant")
            .unwrap()
            .objects
            .get_mut("100")
            .unwrap()
            .data["m_Modification"]["m_AddedComponents"] = json!([record]);
        assert!(g.effective("variant").is_err());
        g.files
            .get_mut("variant")
            .unwrap()
            .objects
            .insert("100".into(), original.clone());
    }
}

#[test]
fn prefab_shrunken_source_keeps_dormant_array_overrides_without_breaking_reads() {
    let mut g = graph();
    g.files
        .get_mut("base")
        .unwrap()
        .objects
        .get_mut("10")
        .unwrap()
        .data["values"] = json!([1, 2, 3]);
    let layer = g.effective("variant").unwrap()["110"].layers[0].clone();
    g.override_value(&layer, "values.Array.data[2]", Some(json!(99)))
        .unwrap();
    g.files
        .get_mut("base")
        .unwrap()
        .objects
        .get_mut("10")
        .unwrap()
        .data["values"] = json!([1]);
    assert_eq!(
        g.effective("variant").unwrap()["110"].object.data["values"],
        json!([1])
    );
    g.files
        .get_mut("base")
        .unwrap()
        .objects
        .get_mut("10")
        .unwrap()
        .data["values"] = json!([1, 2, 3]);
    assert_eq!(
        g.effective("variant").unwrap()["110"].object.data["values"],
        json!([1, 2, 99])
    );
}

#[test]
fn prefab_scene_roots_project_instance_handles_to_root_transforms() {
    let mut g = graph();
    g.files.insert("base".into(),asset("--- !u!1 &10\nGameObject:\n  m_Component: [{component: {fileID: 11}}]\n--- !u!4 &11\nTransform:\n  m_GameObject: {fileID: 10}\n  m_Father: {fileID: 0}\n  m_Children: []\n"));
    g.files.get_mut("scene").unwrap().objects.insert(
        "999".into(),
        ObjectData {
            id: "999".into(),
            class_id: "1660057539".into(),
            root_type: "SceneRoots".into(),
            data: json!({"m_Roots":[{"fileID":"200"}]}),
            stripped: false,
        },
    );
    assert_eq!(
        g.effective("scene").unwrap()["999"].object.data["m_Roots"][0]["fileID"],
        (11 ^ 100 ^ 200).to_string()
    );
}
#[test]
fn prefab_three_levels_propagate_and_same_value_override_is_not_revert() {
    let mut graph = graph();
    let id = (10 ^ 100 ^ 200).to_string();
    let object = graph.effective("scene").unwrap()[&id].clone();
    assert_eq!(object.layers.len(), 2);
    graph
        .override_value(&object.layers[0], "amount", Some(json!(7)))
        .unwrap();
    graph
        .files
        .get_mut("base")
        .unwrap()
        .set("10", "amount", json!(9))
        .unwrap();
    assert_eq!(
        graph.effective("scene").unwrap()[&id].object.data["amount"],
        7
    );
    graph
        .override_value(&object.layers[0], "amount", None)
        .unwrap();
    assert_eq!(
        graph.effective("scene").unwrap()[&id].object.data["amount"],
        9
    );
    graph
        .override_value(&object.layers[1], "amount", Some(json!(11)))
        .unwrap();
    assert_eq!(
        graph.effective("scene").unwrap()[&id].object.data["amount"],
        11
    );
}
#[test]
fn repeated_nested_instances_have_independent_identity() {
    let mut graph = graph();
    let file = graph.files.get_mut("scene").unwrap();
    let mut other = file.objects["200"].clone();
    other.id = "300".into();
    file.objects.insert(other.id.clone(), other);
    let id = (10 ^ 100 ^ 200).to_string();
    let sibling = (10 ^ 100 ^ 300).to_string();
    let layer = graph.effective("scene").unwrap()[&id].layers[0].clone();
    graph
        .override_value(&layer, "amount", Some(json!(99)))
        .unwrap();
    let result = graph.effective("scene").unwrap();
    assert_eq!(result[&id].object.data["amount"], 99);
    assert_eq!(result[&sibling].object.data["amount"], 7);
}
#[test]
fn prefab_cycles_and_ambiguous_identity_fail() {
    let mut graph = graph();
    graph
        .guids
        .insert("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(), "scene".into());
    assert!(graph.effective("scene").unwrap_err().contains("cycle"));
}

#[test]
fn prefab_integer_envelopes_and_invalid_added_topology_are_explicit() {
    let mut graph = graph();
    let id = (10 ^ 100 ^ 200).to_string();
    graph
        .files
        .get_mut("base")
        .unwrap()
        .set(
            "10",
            "amount",
            json!({"kind":"int64","value":"9223372036854775806"}),
        )
        .unwrap();
    let layer = graph.effective("scene").unwrap()[&id].layers[0].clone();
    graph
        .override_value(
            &layer,
            "amount",
            Some(json!({"kind":"int64","value":"9223372036854775807"})),
        )
        .unwrap();
    assert_eq!(
        graph.effective("scene").unwrap()[&id].object.data["amount"]["value"],
        "9223372036854775807"
    );
    graph
        .files
        .get_mut("scene")
        .unwrap()
        .objects
        .get_mut("200")
        .unwrap()
        .data["m_Modification"]["m_AddedComponents"] = json!([{}]);
    assert!(graph
        .effective("scene")
        .unwrap_err()
        .contains("added_target_scope"));
}
fn managed() -> AuthoringAsset {
    asset("--- !u!114 &10\nMonoBehaviour:\n  node: {rid: -2}\n  alias: {rid: 1}\n  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: Node, ns: Test, asm: Assembly-CSharp}\n      data:\n        amount: 7\n        next: {rid: 1}\n")
}
fn template() -> Value {
    json!({"rootRid":"1","entries":[{"rid":"1","type":{"class":"Node","ns":"Test","asm":"Assembly-CSharp"},"data":{"amount":23,"next":{"rid":"2"}}},{"rid":"2","type":{"class":"Node","ns":"Test","asm":"Assembly-CSharp"},"data":{"amount":42,"next":{"rid":"1"}}}]})
}
#[test]
fn creation_remaps_cycles_preserves_aliases_and_survives_reload() {
    let mut file = managed();
    let result = file.create_managed("10", "node", &template()).unwrap();
    assert_ne!(result["rid"], "1");
    let file = AuthoringAsset::new(&file.render(true).unwrap(), Default::default()).unwrap();
    let model = file.semantic().unwrap();
    assert_eq!(
        model
            .resolve("10", "node.next.next.amount", false)
            .unwrap()
            .value,
        Some(&json!(23))
    );
    assert_eq!(
        model.resolve("10", "alias.amount", false).unwrap().value,
        Some(&json!(7))
    );
}
#[test]
fn replacement_does_not_destroy_old_shared_object() {
    let mut file = managed();
    file.set("10", "node", json!({"rid":"1"})).unwrap();
    file.create_managed("10", "node", &template()).unwrap();
    assert_eq!(
        file.semantic()
            .unwrap()
            .resolve("10", "alias.next.amount", false)
            .unwrap()
            .value,
        Some(&json!(7))
    );
}
#[test]
fn creation_rejects_duplicate_labels_external_edges_and_incomplete_data() {
    for template in [
        json!({"rootRid":"1","entries":[template()["entries"][0].clone(),template()["entries"][0].clone()]}),
        json!({"rootRid":"1","entries":[template()["entries"][0].clone()]}),
        json!({"rootRid":"1","entries":[{"rid":"1","type":{"class":"Node","ns":"","asm":"X"}}]}),
    ] {
        assert!(managed().create_managed("10", "node", &template).is_err());
    }
}

#[test]
fn bulk_scalar_requests_share_one_patch_but_every_input_is_validated() {
    let bytes = b"--- !u!114 &1\nMonoBehaviour:\n  amount: 0\n";
    let mut ops = (0..1000)
        .map(|n| AssetOperation::Set {
            object_id: "1".into(),
            property_path: "/MonoBehaviour/amount".into(),
            value: json!(n),
        })
        .collect::<Vec<_>>();
    let output = edit(bytes, &ops).unwrap();
    assert_eq!(output.applied_operations, 1000);
    assert_eq!(
        SemanticAsset::new(output.snapshot)
            .unwrap()
            .resolve("1", "amount", false)
            .unwrap()
            .value,
        Some(&json!(999))
    );
    if let AssetOperation::Set { value, .. } = &mut ops[0] {
        *value = json!({"invalid":true});
    }
    assert!(edit(bytes, &ops).is_err());
}

#[test]
fn unity_null_registry_sentinel_and_opaque_hash_survive_creation() {
    let mut file=asset("--- !u!114 &1\nMonoBehaviour:\n  node: {rid: -2}\n  hash: {serializedVersion: 2, Hash: 01000000020000000300000004000000}\n  references:\n    version: 2\n    RefIds:\n    - rid: -2\n      type: {class: , ns: , asm: }\n");
    file.create_managed("1", "node", &template()).unwrap();
    let rendered = String::from_utf8(file.render(true).unwrap()).unwrap();
    assert!(
        rendered.contains("hash: {serializedVersion: 2, Hash: 01000000020000000300000004000000}")
    );
    assert_eq!(
        file.semantic()
            .unwrap()
            .resolve("1", "node.next.amount", false)
            .unwrap()
            .value,
        Some(&json!(42))
    );
}
#[test]
fn topology_changes_validate_only_final_graph_and_preserve_other_documents() {
    let mut file=asset("--- !u!1 &1\nGameObject:\n  m_Component: []\n--- !u!114 &2\nMonoBehaviour:\n  note: 'untouched'\n");
    file.edit_objects(
        vec![ObjectData {
            id: "3".into(),
            class_id: "114".into(),
            root_type: "MonoBehaviour".into(),
            stripped: false,
            data: json!({"m_GameObject":{"fileID":"1"},"amount":5}),
        }],
        &[],
    )
    .unwrap();
    assert!(file.render(true).is_err());
    file.set("1", "m_Component", json!([{"component":{"fileID":"3"}}]))
        .unwrap();
    let bytes = file.render(true).unwrap();
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains("  note: 'untouched'"));
    file.edit_objects(vec![], &["3".into()]).unwrap();
    assert!(file.render(true).is_err());
    file.set("1", "m_Component", json!([])).unwrap();
    file.render(true).unwrap();
}
