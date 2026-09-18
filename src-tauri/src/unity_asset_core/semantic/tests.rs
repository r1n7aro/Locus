use super::*;

pub(super) fn fixture() -> Vec<u8> {
    b"--- !u!114 &11400000\nMonoBehaviour:\n  amount: 7\n  wide: 9223372036854775807\n  unsigned: 18446744073709551615\n  values: [10, 20]\n  node: {rid: 9007199254740993}\n  alias: {rid: 9007199254740993}\n  reference: {fileID: 0}\n  references:\n    version: 2\n    RefIds:\n    - rid: 9007199254740993\n      type: {class: Node, ns: Test, asm: Assembly-CSharp}\n      data:\n        amount: 23\n        next: {rid: 9007199254740993}\n".to_vec()
}

#[test]
fn semantic_alias_and_cycle_resolve_to_the_same_stable_registry_field() {
    let model = SemanticAsset::new(super::super::inspect(&fixture()).unwrap()).unwrap();
    for path in ["node.amount", "alias.amount", "node.next.next.amount"] {
        let resolved = model.resolve("11400000", path, false).unwrap();
        assert_eq!(resolved.value, Some(&json!(23)));
        assert_eq!(
            resolved.pointer,
            "/MonoBehaviour/references/RefIds/@rid=9007199254740993/data/amount"
        );
        assert_eq!(
            resolved.serialized_path,
            "references.RefIds.Array.data[0].data.amount"
        );
    }
}

#[test]
fn semantic_graph_write_preserves_identity_and_changes_all_aliases() {
    let bytes = fixture();
    let model = SemanticAsset::new(super::super::inspect(&bytes).unwrap()).unwrap();
    let operation = model
        .lower_write("11400000", "node.next.amount", &json!(91))
        .unwrap();
    let edited = super::super::edit(&bytes, &[operation]).unwrap();
    let result = SemanticAsset::new(edited.snapshot).unwrap();
    assert_eq!(
        result
            .resolve("11400000", "alias.amount", false)
            .unwrap()
            .value,
        Some(&json!(91))
    );
    assert_eq!(
        result
            .resolve("11400000", "node.next", false)
            .unwrap()
            .value,
        Some(&json!({"rid":"9007199254740993"}))
    );
}

#[test]
fn semantic_registry_identity_is_host_scoped() {
    let first = String::from_utf8(fixture()).unwrap();
    let second = first
        .replace("&11400000", "&11400001")
        .replace("amount: 23", "amount: 99");
    let model =
        SemanticAsset::new(super::super::inspect(format!("{first}{second}").as_bytes()).unwrap())
            .unwrap();
    assert_eq!(
        model
            .resolve("11400000", "node.amount", false)
            .unwrap()
            .value,
        Some(&json!(23))
    );
    assert_eq!(
        model
            .resolve("11400001", "node.amount", false)
            .unwrap()
            .value,
        Some(&json!(99))
    );
}

#[test]
fn semantic_exact_integers_survive_yaml_projection_and_string_writes() {
    let model = SemanticAsset::new(super::super::inspect(&fixture()).unwrap()).unwrap();
    let yaml = model.yaml_value("11400000").unwrap();
    assert_eq!(yaml["wide"].as_i64(), Some(i64::MAX));
    assert_eq!(yaml["unsigned"].as_u64(), Some(u64::MAX));
    let operation = model
        .lower_write("11400000", "wide", &json!("9223372036854775806"))
        .unwrap();
    let encoded = serde_json::to_value(operation).unwrap();
    assert_eq!(
        encoded["value"],
        json!({"kind":"int64","value":"9223372036854775806"})
    );
}

#[test]
fn semantic_paths_reject_ambiguous_indexes_and_atomic_identity_members() {
    let model = SemanticAsset::new(super::super::inspect(&fixture()).unwrap()).unwrap();
    for path in [
        "values.Array.data[-1]",
        "values.Array.data[01]",
        "values.Array.size",
        "node..amount",
        "reference.fileID",
        "wide.value",
    ] {
        assert!(model.resolve("11400000", path, false).is_err(), "{path}");
    }
    assert_eq!(
        model
            .resolve("11400000", "values.Array.data[1]", false)
            .unwrap()
            .value,
        Some(&json!(20))
    );
}

#[test]
fn semantic_commands_require_explicit_creation_data_and_do_not_create_types() {
    let model = SemanticAsset::new(super::super::inspect(&fixture()).unwrap()).unwrap();
    assert!(model
        .lower_write("11400000", "values", &json!({"action":"insert","index":0}))
        .unwrap_err()
        .contains("explicit_value"));
    assert!(model
        .lower_write(
            "11400000",
            "node",
            &json!({"action":"setType","typeName":"Test.Other"})
        )
        .unwrap_err()
        .contains("unsupported_command"));
    let operation = model
        .lower_write(
            "11400000",
            "values",
            &json!({"action":"insert","index":1,"value":8}),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(operation).unwrap()["op"],
        "array_insert"
    );
}

#[test]
fn semantic_batch_reassignment_changes_later_logical_write_addresses() {
    let text=String::from_utf8(fixture()).unwrap().replace("  node:","  other: {rid: 42}\n  node:")
        + "    - rid: 42\n      type: {class: Node, ns: Test, asm: Assembly-CSharp}\n      data:\n        amount: 41\n        next: {rid: -2}\n";
    let model = SemanticAsset::new(super::super::inspect(text.as_bytes()).unwrap()).unwrap();
    let operations = model
        .lower_writes(&[
            ("11400000".into(), "node".into(), json!({"rid":"42"})),
            ("11400000".into(), "node.amount".into(), json!(99)),
        ])
        .unwrap();
    assert!(operations[1].property_path().contains("@rid=42/"));
    let after = SemanticAsset::new(
        super::super::edit(text.as_bytes(), &operations)
            .unwrap()
            .snapshot,
    )
    .unwrap();
    assert_eq!(
        after
            .resolve("11400000", "alias.amount", false)
            .unwrap()
            .value,
        Some(&json!(23))
    );
    assert_eq!(
        after
            .resolve("11400000", "other.amount", false)
            .unwrap()
            .value,
        Some(&json!(99))
    );
}

#[test]
fn semantic_batch_insert_then_child_write_uses_the_staged_array() {
    let model = SemanticAsset::new(super::super::inspect(&fixture()).unwrap()).unwrap();
    let operations = model
        .lower_writes(&[
            (
                "11400000".into(),
                "values".into(),
                json!({"action":"insert","index":2,"value":0}),
            ),
            ("11400000".into(), "values.Array.data[2]".into(), json!(91)),
        ])
        .unwrap();
    let after = SemanticAsset::new(
        super::super::edit(&fixture(), &operations)
            .unwrap()
            .snapshot,
    )
    .unwrap();
    assert_eq!(
        after.resolve("11400000", "values", false).unwrap().value,
        Some(&json!([10, 20, 91]))
    );
}

#[test]
fn semantic_type_evidence_restores_boolean_float_and_string_presentation() {
    let bytes = b"--- !u!114 &11400000\nMonoBehaviour:\n  enabled: 1\n  speed: 1\n  text: 123\n";
    let hints = std::collections::BTreeMap::from([(
        "11400000".into(),
        std::collections::BTreeMap::from([
            (
                "/MonoBehaviour/enabled".into(),
                super::super::ScalarHint::Boolean,
            ),
            (
                "/MonoBehaviour/speed".into(),
                super::super::ScalarHint::Float,
            ),
            (
                "/MonoBehaviour/text".into(),
                super::super::ScalarHint::String,
            ),
        ]),
    )]);
    let model =
        SemanticAsset::new(super::super::inspect_with_hints(bytes, &hints).unwrap()).unwrap();
    let yaml = model.yaml_value("11400000").unwrap();
    assert_eq!(yaml["enabled"].as_bool(), Some(true));
    assert_eq!(yaml["speed"].as_f64(), Some(1.0));
    assert!(yaml["speed"].as_i64().is_none());
    assert_eq!(yaml["text"].as_str(), Some("123"));
    assert_eq!(model.snapshot.objects[0].fields[0].value, json!(1));
}

#[test]
fn semantic_new_array_elements_keep_declared_type_evidence_before_reread() {
    let bytes = b"--- !u!114 &11400000\nMonoBehaviour:\n  values: [1, 2]\n";
    let hints = std::collections::BTreeMap::from([(
        "11400000".into(),
        std::collections::BTreeMap::from([
            (
                "/MonoBehaviour/values".into(),
                super::super::ScalarHint::PackedArray(super::super::PackedElement::F32),
            ),
            (
                "/MonoBehaviour/values/0".into(),
                super::super::ScalarHint::Float,
            ),
            (
                "/MonoBehaviour/values/1".into(),
                super::super::ScalarHint::Float,
            ),
        ]),
    )]);
    let operation = AssetOperation::ArrayResize {
        object_id: "11400000".into(),
        property_path: "/MonoBehaviour/values".into(),
        size: 4,
        value: Some(json!(1)),
    };
    let output = super::super::edit_with_hints(bytes, &[operation], &hints).unwrap();
    for field in output.snapshot.objects[0]
        .fields
        .iter()
        .filter(|field| field.property_path.starts_with("/MonoBehaviour/values/"))
    {
        assert_eq!(field.type_hint.as_deref(), Some("Float"));
    }
    let projected = SemanticAsset::new(output.snapshot)
        .unwrap()
        .yaml_value("11400000")
        .unwrap();
    assert!(projected["values"][3].as_i64().is_none());
    assert_eq!(projected["values"][3].as_f64(), Some(1.0));
}
