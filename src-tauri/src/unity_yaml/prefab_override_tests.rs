//! Coverage for PrefabInstance `m_Modifications` capture
//! (`parse_yaml_docs_for_index` → `prefab_overrides`) and the property-path
//! helpers. Shapes mirror Unity 6.5 output for synthetic scripts.

use super::managed_refs::ManagedTypeIdentity;
use super::parse_yaml_docs_for_index;
use super::prefab_overrides::{
    is_indexed_override_path, nested_file_id, parse_managed_reference_path,
    parse_managed_type_value, property_path_head, tokenize_property_path, PathToken,
    RawPrefabOverride,
};
use crate::asset_db::types::parse_guid_hex;

const BASE_GUID: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f9";

fn scene() -> String {
    format!(
        "%YAML 1.1
--- !u!1001 &1369411486
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    serializedVersion: 3
    m_TransformParent: {{fileID: 0}}
    m_Modifications:
    - target: {{fileID: 1383303035007636456, guid: {BASE_GUID}, type: 3}}
      propertyPath: m_Name
      value: TypeOverride
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4954498630159034338, guid: {BASE_GUID},
        type: 3}}
      propertyPath: main
      value: 7542395272935243781
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4954498630159034338, guid: {BASE_GUID}, type: 3}}
      propertyPath: 'managedReferences[7542395272935243781]'
      value: Assembly-CSharp Probe.Combat.Chain
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4954498630159034338, guid: {BASE_GUID}, type: 3}}
      propertyPath: 'managedReferences[-2]'
      value:
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4954498630159034338, guid: {BASE_GUID}, type: 3}}
      propertyPath: tuning.steps.Array.data[0].max
      value: 33
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4954498630159034338, guid: {BASE_GUID}, type: 3}}
      propertyPath: icon
      value:
      objectReference: {{fileID: 2800000, guid: {BASE_GUID}, type: 3}}
    m_RemovedComponents: []
    m_RemovedGameObjects: []
    m_AddedGameObjects: []
    m_AddedComponents: []
  m_SourcePrefab: {{fileID: 100100000, guid: {BASE_GUID}, type: 3}}
--- !u!114 &77
MonoBehaviour:
  data:
  - target: {{fileID: 1, guid: {BASE_GUID}, type: 3}}
    propertyPath: notAnOverride
    value: 1
"
    )
}

#[test]
fn captures_modifications_with_wrapped_targets_and_quoted_paths() {
    let parsed = parse_yaml_docs_for_index(scene().as_bytes());
    let base = parse_guid_hex(BASE_GUID).unwrap();
    let paths: Vec<(&str, &str, i64)> = parsed
        .prefab_overrides
        .iter()
        .map(|o| (o.property_path.as_str(), o.value.as_str(), o.target_file_id))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("m_Name", "TypeOverride", 1383303035007636456),
            ("main", "7542395272935243781", 4954498630159034338),
            (
                "managedReferences[7542395272935243781]",
                "Assembly-CSharp Probe.Combat.Chain",
                4954498630159034338
            ),
            ("managedReferences[-2]", "", 4954498630159034338),
            ("tuning.steps.Array.data[0].max", "33", 4954498630159034338),
            ("icon", "", 4954498630159034338),
        ]
    );
    let main = &parsed.prefab_overrides[1];
    assert_eq!(
        main,
        &RawPrefabOverride {
            instance_file_id: 1369411486,
            target_guid: base,
            target_file_id: 4954498630159034338,
            property_path: "main".into(),
            value: "7542395272935243781".into(),
            line: 16,
        }
    );
    assert!(parsed
        .prefab_overrides
        .iter()
        .all(|o| o.instance_file_id == 1369411486));
}

#[test]
fn managed_reference_paths_and_type_values() {
    assert_eq!(
        parse_managed_reference_path("managedReferences[7542395272935243781]"),
        Some((7542395272935243781, ""))
    );
    assert_eq!(
        parse_managed_reference_path("managedReferences[-2]"),
        Some((-2, ""))
    );
    assert_eq!(
        parse_managed_reference_path("managedReferences[12].next"),
        Some((12, "next"))
    );
    assert_eq!(
        parse_managed_reference_path("managedReferences[12].list.Array.data[3].x"),
        Some((12, "list.Array.data[3].x"))
    );
    assert_eq!(parse_managed_reference_path("tuning.range"), None);
    assert_eq!(parse_managed_reference_path("managedReferences[x]"), None);

    let identity = |c: &str, n: &str, a: &str| ManagedTypeIdentity {
        class_name: c.into(),
        namespace: n.into(),
        assembly: a.into(),
    };
    assert_eq!(
        parse_managed_type_value("Assembly-CSharp Probe.Combat.Chain"),
        Some(identity("Chain", "Probe.Combat", "Assembly-CSharp"))
    );
    assert_eq!(
        parse_managed_type_value("Game.Runtime Game.Outer/Inner"),
        Some(identity("Outer/Inner", "Game", "Game.Runtime"))
    );
    assert_eq!(
        parse_managed_type_value("Asm Game.Box`1[[System.Int32, mscorlib]]"),
        Some(identity("Box`1[[System.Int32, mscorlib]]", "Game", "Asm"))
    );
    assert_eq!(
        parse_managed_type_value("Assembly-CSharp GlobalType"),
        Some(identity("GlobalType", "", "Assembly-CSharp"))
    );
    assert_eq!(parse_managed_type_value(""), None);
    assert_eq!(parse_managed_type_value("OnlyOneWord"), None);
}

#[test]
fn nested_file_ids_follow_unity_xor_rule_both_ways() {
    // Values observed in Unity 6.5 for a variant and a nested prefab.
    assert_eq!(
        nested_file_id(1675151794310873844, 4954498630159034338),
        6052485984431105302
    );
    assert_eq!(
        nested_file_id(4212884190871731329, 4954498630159034338),
        9130698156442213219
    );
    assert_eq!(
        nested_file_id(4212884190871731329, 9130698156442213219),
        4954498630159034338
    );
}

#[test]
fn property_path_tokens_heads_and_index_filter() {
    assert_eq!(
        tokenize_property_path("tuning.steps.Array.data[12].max"),
        vec![
            PathToken::Field("tuning".into()),
            PathToken::Field("steps".into()),
            PathToken::Element(12),
            PathToken::Field("max".into()),
        ]
    );
    assert_eq!(
        tokenize_property_path("extra.Array.size"),
        vec![PathToken::Field("extra".into()), PathToken::Size]
    );
    assert_eq!(property_path_head("tuning.range.max"), "tuning");
    assert_eq!(property_path_head("ranges.Array.data[0].max"), "ranges");
    assert_eq!(
        property_path_head("managedReferences[7].x"),
        "managedReferences"
    );
    assert!(is_indexed_override_path("tuning.range.max"));
    assert!(is_indexed_override_path("managedReferences[7]"));
    assert!(is_indexed_override_path("m_Speed"));
    assert!(!is_indexed_override_path("m_LocalPosition.x"));
    assert!(!is_indexed_override_path("m_Name"));
}
