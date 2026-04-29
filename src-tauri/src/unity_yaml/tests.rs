use super::aggregation::{is_mesh_data_property, normalize_instance_name};
use super::prefab::{extract_prefab_instance_irs, extract_stripped_mappings};
use super::references::extract_refs;
use super::tokenizer::parse_doc_header_full;
use super::{
    build_go_tree, build_hierarchy_path_map, build_internal_id_map, build_world_transform_map,
    find_go_by_path, format_doc_state_lines, format_hierarchy_search_results,
    format_hierarchy_summary, format_hierarchy_summary_with_options, format_hierarchy_tree,
    format_override_summary, format_prefab_instance_detail, get_components_for_go, parse_yaml_docs,
    resolve_references_in_lines, resolve_references_in_lines_skipping_fields,
    summarize_prefab_instance, HierarchyNode, HierarchySearchOptions, HierarchySummaryOptions,
    DEFAULT_HIERARCHY_MAX_NODES,
};
use crate::asset_db::types::guid_to_hex;
use crate::asset_db::types::Guid;

fn assert_vec3_close(actual: [f64; 3], expected: [f64; 3]) {
    for idx in 0..3 {
        assert!(
            (actual[idx] - expected[idx]).abs() < 0.01,
            "component {} mismatch: actual={}, expected={}",
            idx,
            actual[idx],
            expected[idx]
        );
    }
}

#[test]
fn test_flow_map_basic() {
    let content = b"--- !u!1 &100000\nGameObject:\n  m_Component:\n  - component: {fileID: 11400000, guid: abcdef0123456789abcdef0123456789, type: 3}\n";
    let refs = extract_refs(content);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        guid_to_hex(&refs[0].dst_guid),
        "abcdef0123456789abcdef0123456789"
    );
    assert_eq!(refs[0].dst_file_id, Some(11400000));
}

#[test]
fn test_mono_script_ref() {
    let content = b"--- !u!114 &114000000\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}\n";
    let refs = extract_refs(content);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].field_hint.as_deref(), Some("m_Script"));
}

#[test]
fn test_skip_zero_guid() {
    let content = b"  m_Script: {fileID: 0, guid: 00000000000000000000000000000000, type: 0}\n";
    let refs = extract_refs(content);
    assert!(refs.is_empty());
}

#[test]
fn test_multiple_array_refs() {
    let yaml = b"  m_Materials:\n  - {fileID: 2100000, guid: aaaa0000bbbb1111cccc2222dddd3333, type: 2}\n  - {fileID: 2100000, guid: 11112222333344445555666677778888, type: 2}\n";
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 2);
}

#[test]
fn test_doc_header_parsing() {
    assert_eq!(
        parse_doc_header_full("--- !u!114 &11400000"),
        Some((114, 11400000))
    );
    assert_eq!(parse_doc_header_full("--- !u!1 &100000"), Some((1, 100000)));
    assert_eq!(
        parse_doc_header_full("--- !u!1001 &7856341"),
        Some((1001, 7856341))
    );
    assert_eq!(parse_doc_header_full("---"), None);
}

#[test]
fn test_multiline_flow_map() {
    let yaml = b"--- !u!114 &1234\nMonoBehaviour:\n  _playerPrefab: {fileID: 5557640735889932260, guid: 062b3805bf6784e4d9c599ee60eaa002,\n    type: 3}\n";
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        guid_to_hex(&refs[0].dst_guid),
        "062b3805bf6784e4d9c599ee60eaa002"
    );
    assert_eq!(refs[0].dst_file_id, Some(5557640735889932260));
    assert_eq!(refs[0].field_hint.as_deref(), Some("_playerPrefab"));
}

#[test]
fn test_consecutive_multiline_flow_maps() {
    let yaml = b"--- !u!114 &1234\nMonoBehaviour:\n  _playerPrefab: {fileID: 5557640735889932260, guid: 062b3805bf6784e4d9c599ee60eaa002,\n    type: 3}\n  _player2Prefab: {fileID: 5557640735889932260, guid: 062b3805bf6784e4d9c599ee60eaa002,\n    type: 3}\n";
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 2);
}

#[test]
fn test_multiline_target_ref() {
    let yaml = b"--- !u!1001 &9999\nPrefabInstance:\n  m_Modification:\n    m_Modifications:\n    - target: {fileID: 58162620821929115, guid: 062b3805bf6784e4d9c599ee60eaa002,\n        type: 3}\n      propertyPath: m_LocalEulerAnglesHint.x\n      value: -28.01\n      objectReference: {fileID: 0}\n";
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        guid_to_hex(&refs[0].dst_guid),
        "062b3805bf6784e4d9c599ee60eaa002"
    );
    assert_eq!(refs[0].dst_file_id, Some(58162620821929115));
}

#[test]
fn test_mixed_single_and_multiline() {
    let yaml = b"--- !u!114 &5678\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}\n  _inputReader: {fileID: 11400000, guid: 945ec0365077176418488737deed54be, type: 2}\n  _playerPrefab: {fileID: 5557640735889932260, guid: 062b3805bf6784e4d9c599ee60eaa002,\n    type: 3}\n  _playerTransformAnchor: {fileID: 11400000, guid: 35fc4039342b6ba458d0d4429e89ee74,\n    type: 2}\n";
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 4);
}

#[test]
fn test_ref_path_basic_hierarchy() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Player
  m_Component:
  - component: {fileID: 200}
--- !u!4 &150
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].ref_path.as_deref(),
        Some("Player/MonoBehaviour/m_Script")
    );
}

#[test]
fn test_ref_path_nested_hierarchy() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Root
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!1 &200
GameObject:
  m_Name: Child
--- !u!4 &201
Transform:
  m_GameObject: {fileID: 200}
  m_Father: {fileID: 101}
--- !u!114 &300
MonoBehaviour:
  m_GameObject: {fileID: 200}
  _prefabRef: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].ref_path.as_deref(),
        Some("Root/Child/MonoBehaviour/_prefabRef")
    );
}

#[test]
fn test_ref_path_scriptable_object() {
    let yaml = br#"--- !u!114 &11400000
MonoBehaviour:
  m_Name: GameSettings
  someRef: {fileID: 2100000, guid: aabbccdd11223344aabbccdd11223344, type: 2}
"#;
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].ref_path.as_deref(), Some("MonoBehaviour/someRef"));
}

#[test]
fn test_ref_path_deep_hierarchy() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: GrandParent
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!1 &20
GameObject:
  m_Name: Parent
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_Father: {fileID: 11}
--- !u!1 &30
GameObject:
  m_Name: Child
--- !u!4 &31
Transform:
  m_GameObject: {fileID: 30}
  m_Father: {fileID: 21}
--- !u!114 &40
MonoBehaviour:
  m_GameObject: {fileID: 30}
  targetAsset: {fileID: 2100000, guid: aabbccdd11223344aabbccdd11223344, type: 2}
"#;
    let refs = extract_refs(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].ref_path.as_deref(),
        Some("GrandParent/Parent/Child/MonoBehaviour/targetAsset")
    );
}

#[test]
fn test_parse_yaml_docs() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Player
--- !u!4 &150
Transform:
  m_GameObject: {fileID: 100}
  m_Children:
  - {fileID: 151}
  m_Father: {fileID: 0}
  m_RootOrder: 3
--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    assert_eq!(docs.len(), 3);
    assert_eq!(docs[0].class_id, 1);
    assert_eq!(docs[0].m_name.as_deref(), Some("Player"));
    assert_eq!(docs[1].class_id, 4);
    assert_eq!(docs[1].m_game_object_id, Some(100));
    assert_eq!(docs[1].transform_children, vec![151]);
    assert_eq!(docs[1].transform_root_order, Some(3));
    assert_eq!(docs[2].class_id, 114);
    assert_eq!(docs[2].m_enabled, Some(true));
    assert_eq!(docs[2].type_name, "MonoBehaviour");
}

#[test]
fn test_format_doc_state_lines_outputs_enabled_state() {
    let yaml = br#"--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 0
"#;
    let docs = parse_yaml_docs(yaml);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].m_enabled, Some(false));
    assert_eq!(format_doc_state_lines(&docs[0]), "  Enabled: false\n");
}

#[test]
fn test_resolve_references_skips_raw_enabled_field_when_requested() {
    let yaml = br#"--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 1
  m_Name: PlayerLogic
"#;
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();
    let docs = parse_yaml_docs(yaml);
    let guid_resolver = |_hex: &str| -> Option<String> { None };
    let internal_resolver = |_fid: i64| -> Option<String> { None };

    let rendered = resolve_references_in_lines_skipping_fields(
        &lines,
        docs[0].line_start + 2,
        docs[0].line_end,
        &guid_resolver,
        &internal_resolver,
        &["m_Enabled"],
    );

    assert!(!rendered.contains("m_Enabled"));
    assert!(rendered.contains("m_Name: PlayerLogic"));
}

#[test]
fn test_build_go_tree_and_format() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!1 &20
GameObject:
  m_Name: Child1
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_Father: {fileID: 11}
--- !u!1 &30
GameObject:
  m_Name: Child2
--- !u!4 &31
Transform:
  m_GameObject: {fileID: 30}
  m_Father: {fileID: 11}
--- !u!1 &40
GameObject:
  m_Name: GrandChild
--- !u!4 &41
Transform:
  m_GameObject: {fileID: 40}
  m_Father: {fileID: 21}
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].name, "Root");
    assert_eq!(tree[0].children.len(), 2);

    let formatted = format_hierarchy_tree(&tree);
    assert!(formatted.contains("Root"));
    assert!(formatted.contains("Child1"));
    assert!(formatted.contains("Child2"));
    assert!(formatted.contains("GrandChild"));
}

#[test]
fn test_build_go_tree_uses_root_order_and_children_order() {
    let yaml = br#"--- !u!1 &30
GameObject:
  m_Name: Third
--- !u!4 &31
Transform:
  m_GameObject: {fileID: 30}
  m_Father: {fileID: 0}
  m_RootOrder: 2
--- !u!1 &10
GameObject:
  m_Name: Parent
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Children:
  - {fileID: 41}
  - {fileID: 21}
  m_Father: {fileID: 0}
  m_RootOrder: 0
--- !u!1 &40
GameObject:
  m_Name: ChildB
--- !u!4 &41
Transform:
  m_GameObject: {fileID: 40}
  m_Father: {fileID: 11}
--- !u!1 &20
GameObject:
  m_Name: ChildA
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_Father: {fileID: 11}
--- !u!1 &50
GameObject:
  m_Name: Second
--- !u!4 &51
Transform:
  m_GameObject: {fileID: 50}
  m_Father: {fileID: 0}
  m_RootOrder: 1
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);

    let root_names: Vec<&str> = tree.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(root_names, vec!["Parent", "Second", "Third"]);
    let child_names: Vec<&str> = tree[0]
        .children
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(child_names, vec!["ChildB", "ChildA"]);
}

#[test]
fn test_find_go_by_path() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!1 &20
GameObject:
  m_Name: Child
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_Father: {fileID: 11}
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);

    assert_eq!(find_go_by_path(&tree, "Root"), Some(10));
    assert_eq!(find_go_by_path(&tree, "Root/Child"), Some(20));
    assert_eq!(find_go_by_path(&tree, "Root/NonExistent"), None);
    assert_eq!(find_go_by_path(&tree, "NonExistent"), None);
}

#[test]
fn test_find_go_by_path_accepts_sibling_ordinal_paths() {
    let roots = vec![HierarchyNode {
        name: "Root".to_string(),
        file_id: 1,
        is_active: true,
        children: vec![
            HierarchyNode {
                name: "Enemy".to_string(),
                file_id: 2,
                is_active: true,
                ..Default::default()
            },
            HierarchyNode {
                name: "Enemy".to_string(),
                file_id: 3,
                is_active: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let paths = build_hierarchy_path_map(&roots);
    assert_eq!(paths.get(&2).map(String::as_str), Some("Root/Enemy[1]"));
    assert_eq!(paths.get(&3).map(String::as_str), Some("Root/Enemy[2]"));
    assert_eq!(find_go_by_path(&roots, "Root/Enemy[1]"), Some(2));
    assert_eq!(find_go_by_path(&roots, "Root/Enemy[2]"), Some(3));
}

#[test]
fn test_get_components_for_go() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Player
--- !u!4 &150
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let components = get_components_for_go(&docs, 100);
    assert_eq!(components.len(), 3);
}

#[test]
fn test_build_world_transform_map_accumulates_position_and_scale() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 1, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0.70710678, w: 0.70710678}
  m_LocalScale: {x: 2, y: 2, z: 2}
  m_Father: {fileID: 0}
--- !u!1 &20
GameObject:
  m_Name: Child
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_LocalPosition: {x: 1, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 0.5, y: 1, z: 2}
  m_Father: {fileID: 11}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let world_map = build_world_transform_map(&docs, &lines);
    let child = world_map.get(&21).copied().unwrap();

    assert_vec3_close(child.position, [1.0, 2.0, 0.0]);
    assert_vec3_close(child.scale, [1.0, 2.0, 4.0]);
}

#[test]
fn test_build_world_transform_map_accumulates_rotation() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0.70710678, z: 0, w: 0.70710678}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let world_map = build_world_transform_map(&docs, &lines);
    let root = world_map.get(&11).copied().unwrap();

    assert_vec3_close(root.rotation_euler, [0.0, 90.0, 0.0]);
}

#[test]
fn test_resolve_references() {
    let yaml = br#"--- !u!114 &200
MonoBehaviour:
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  someRef: {fileID: 100}
  nullRef: {fileID: 0}
"#;
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let guid_resolver = |hex: &str| -> Option<String> {
        if hex == "aabbccdd11223344aabbccdd11223344" {
            Some("Assets/Scripts/PlayerController.cs".to_string())
        } else {
            None
        }
    };

    let internal_resolver = |fid: i64| -> Option<String> {
        if fid == 100 {
            Some("GO:Player".to_string())
        } else {
            None
        }
    };

    let resolved = resolve_references_in_lines(&lines, 2, 5, &guid_resolver, &internal_resolver);
    assert!(resolved.contains("Assets/Scripts/PlayerController.cs"));
    assert!(resolved.contains("GO:Player"));
    assert!(resolved.contains("{none}"));
}

#[test]
fn test_resolve_references_multiline_flow_map() {
    let yaml = br#"--- !u!114 &200
MonoBehaviour:
  singleLine: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  multiLine: {fileID: 4041797215495687746, guid: aabbccdd11223344aabbccdd11223344,
    type: 3}
  afterMulti: {fileID: 100}
"#;
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let guid_resolver = |hex: &str| -> Option<String> {
        if hex == "aabbccdd11223344aabbccdd11223344" {
            Some("Assets/Animations/Idle.anim".to_string())
        } else {
            None
        }
    };

    let internal_resolver = |fid: i64| -> Option<String> {
        if fid == 100 {
            Some("GO:Player".to_string())
        } else {
            None
        }
    };

    let resolved = resolve_references_in_lines(&lines, 2, 7, &guid_resolver, &internal_resolver);
    assert!(
        resolved.contains("Assets/Animations/Idle.anim"),
        "single-line ref should resolve"
    );
    let multiline_count = resolved.matches("Assets/Animations/Idle.anim").count();
    assert_eq!(
        multiline_count, 2,
        "both single-line and multi-line refs should resolve, got:\n{}",
        resolved
    );
    assert!(
        resolved.contains("GO:Player"),
        "ref after multi-line should resolve"
    );
}

#[test]
fn test_internal_id_map() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Player
--- !u!4 &150
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!114 &200
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let map = build_internal_id_map(&docs);

    assert_eq!(map.get(&100).unwrap(), "GO:Player");
    assert_eq!(map.get(&150).unwrap(), "Player.Transform");
    assert_eq!(map.get(&200).unwrap(), "Player.MonoBehaviour");
}

#[test]
fn test_parse_prefab_instance_scene() {
    let yaml = br#"--- !u!29 &1
OcclusionCullingSettings:
  m_ObjectHideFlags: 0
--- !u!114 &670213351 stripped
MonoBehaviour:
  m_CorrespondingSourceObject: {fileID: 8980297398607076176, guid: ccad748453924ff4092fe3e5b978d8e5, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
  m_GameObject: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: DialogueManager
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: ccad748453924ff4092fe3e5b978d8e5, type: 3}
--- !u!1001 &9001
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: 11223344aabbccdd11223344aabbccdd, type: 3}
      propertyPath: m_Name
      value: GameManager
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: 11223344aabbccdd11223344aabbccdd, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);

    assert_eq!(docs.len(), 4);

    // stripped MonoBehaviour
    let stripped = &docs[1];
    assert_eq!(stripped.class_id, 114);
    assert!(stripped.is_stripped);
    assert_eq!(stripped.prefab_instance_id, Some(9000));

    // PrefabInstance 9000
    let pi0 = &docs[2];
    assert_eq!(pi0.class_id, 1001);
    assert_eq!(pi0.file_id, 9000);
    assert_eq!(pi0.m_name.as_deref(), Some("DialogueManager"));
    assert_eq!(pi0.transform_parent_id, Some(0));
    assert!(pi0.source_prefab_guid.is_some());

    // PrefabInstance 9001
    let pi1 = &docs[3];
    assert_eq!(pi1.class_id, 1001);
    assert_eq!(pi1.m_name.as_deref(), Some("GameManager"));
}

#[test]
fn test_build_go_tree_with_prefab_instances() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: CutsceneManager
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
--- !u!1001 &9001
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: 11223344aabbccdd11223344aabbccdd, type: 3}
      propertyPath: m_Name
      value: DialogueManager
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: 11223344aabbccdd11223344aabbccdd, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);

    assert_eq!(tree.len(), 2);
    let names: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"CutsceneManager"));
    assert!(names.contains(&"DialogueManager"));

    let formatted = format_hierarchy_tree(&tree);
    assert!(formatted.contains("CutsceneManager"));
    assert!(formatted.contains("DialogueManager"));
}

#[test]
fn test_mixed_go_and_prefab_instance() {
    let yaml = br#"--- !u!1 &10
GameObject:
  m_Name: SceneRoot
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 11}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: ChildPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);

    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].name, "SceneRoot");
    assert_eq!(tree[0].children.len(), 1);
    assert_eq!(tree[0].children[0].name, "ChildPrefab");
}

#[test]
fn test_gameobject_parented_to_stripped_transform_attaches_to_prefab_instance() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: ParentPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
--- !u!4 &600 stripped
Transform:
  m_PrefabInstance: {fileID: 9000}
  m_GameObject: {fileID: 0}
  m_Father: {fileID: 0}
--- !u!1 &10
GameObject:
  m_Name: NestedChild
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 600}
"#;
    let docs = parse_yaml_docs(yaml);
    let tree = build_go_tree(&docs);

    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].name, "ParentPrefab");
    assert_eq!(tree[0].children.len(), 1);
    assert_eq!(tree[0].children[0].name, "NestedChild");
}

#[test]
fn test_get_components_for_prefab_instance() {
    let yaml = br#"--- !u!114 &500 stripped
MonoBehaviour:
  m_PrefabInstance: {fileID: 9000}
  m_GameObject: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: MyPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let components = get_components_for_go(&docs, 9000);
    assert_eq!(components.len(), 1);
}

#[test]
fn test_internal_id_map_with_prefab() {
    let yaml = br#"--- !u!114 &500 stripped
MonoBehaviour:
  m_PrefabInstance: {fileID: 9000}
  m_GameObject: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: MyPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let map = build_internal_id_map(&docs);

    assert_eq!(map.get(&9000).unwrap(), "Prefab:MyPrefab");
    assert_eq!(map.get(&500).unwrap(), "MyPrefab.MonoBehaviour (stripped)");
}

#[test]
fn test_extract_prefab_instance_ir_basic() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: MyInstance
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.x
      value: 1.5
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.y
      value: 2.0
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.z
      value: -0.5
      objectReference: {fileID: 0}
    - target: {fileID: 300, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 9
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let irs = extract_prefab_instance_irs(&docs, &lines);
    assert_eq!(irs.len(), 1);

    let ir = &irs[0];
    assert_eq!(ir.local_file_id, 9000);
    assert_eq!(ir.instance_name.as_deref(), Some("MyInstance"));
    assert_eq!(ir.property_overrides.len(), 5);
    assert_eq!(
        guid_to_hex(&ir.source_prefab_guid),
        "aabbccdd11223344aabbccdd11223344"
    );

    let name_ov = ir
        .property_overrides
        .iter()
        .find(|o| o.property_path == "m_Name")
        .unwrap();
    assert_eq!(name_ov.value.as_deref(), Some("MyInstance"));

    let pos_x = ir
        .property_overrides
        .iter()
        .find(|o| o.property_path == "m_LocalPosition.x")
        .unwrap();
    assert_eq!(pos_x.value.as_deref(), Some("1.5"));
    assert_eq!(pos_x.target.source_file_id, 200);
}

#[test]
fn test_extract_prefab_instance_ir_multiline_target() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 58162620821929115, guid: 062b3805bf6784e4d9c599ee60eaa002,
        type: 3}
      propertyPath: m_LocalEulerAnglesHint.x
      value: -28.01
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: 062b3805bf6784e4d9c599ee60eaa002, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let irs = extract_prefab_instance_irs(&docs, &lines);
    assert_eq!(irs.len(), 1);
    assert_eq!(irs[0].property_overrides.len(), 1);

    let ov = &irs[0].property_overrides[0];
    assert_eq!(ov.property_path, "m_LocalEulerAnglesHint.x");
    assert_eq!(ov.value.as_deref(), Some("-28.01"));
    assert_eq!(ov.target.source_file_id, 58162620821929115);
    assert_eq!(
        guid_to_hex(&ov.target.guid),
        "062b3805bf6784e4d9c599ee60eaa002"
    );
}

#[test]
fn test_extract_stripped_mappings() {
    let yaml = br#"--- !u!114 &500 stripped
MonoBehaviour:
  m_CorrespondingSourceObject: {fileID: 8980297398607076176, guid: ccad748453924ff4092fe3e5b978d8e5, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
--- !u!4 &600 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 1234567890123456789, guid: ccad748453924ff4092fe3e5b978d8e5, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications: []
  m_SourcePrefab: {fileID: 100100000, guid: ccad748453924ff4092fe3e5b978d8e5, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();

    let mappings = extract_stripped_mappings(&docs, &lines);
    assert_eq!(mappings.len(), 2);

    let mono = mappings.iter().find(|m| m.class_id == 114).unwrap();
    assert_eq!(mono.local_file_id, 500);
    assert_eq!(mono.source.source_file_id, 8980297398607076176);
    assert_eq!(mono.prefab_instance_id, 9000);

    let tf = mappings.iter().find(|m| m.class_id == 4).unwrap();
    assert_eq!(tf.local_file_id, 600);
    assert_eq!(tf.source.source_file_id, 1234567890123456789);
}

#[test]
fn test_summarize_prefab_instance() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: TestPrefab
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.x
      value: 1.0
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.y
      value: 2.0
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_LocalPosition.z
      value: 3.0
      objectReference: {fileID: 0}
    - target: {fileID: 300, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 9
      objectReference: {fileID: 0}
    - target: {fileID: 400, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 9
      objectReference: {fileID: 0}
    - target: {fileID: 500, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 9
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();
    let irs = extract_prefab_instance_irs(&docs, &lines);
    let ir = &irs[0];

    let no_resolve = |_: &Guid| -> Option<String> { None };
    let summary = summarize_prefab_instance(ir, 0, vec![], &no_resolve);

    assert_eq!(summary.instance_name, "TestPrefab");
    assert_eq!(summary.total_override_count, 7);

    assert_eq!(summary.transform_overrides.len(), 1);
    let ts = &summary.transform_overrides[0];
    assert!(ts.position.is_some());

    assert_eq!(summary.bulk_overrides.len(), 1);
    assert_eq!(summary.bulk_overrides[0].property_path, "m_Layer");
    assert_eq!(summary.bulk_overrides[0].value, "9");
    assert_eq!(summary.bulk_overrides[0].target_count, 3);
}

#[test]
fn test_format_override_summary() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: FormattedPrefab
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();
    let irs = extract_prefab_instance_irs(&docs, &lines);
    let ir = &irs[0];

    let resolver = |_: &Guid| -> Option<String> { Some("Assets/Prefabs/Test.prefab".to_string()) };
    let summary = summarize_prefab_instance(ir, 2, vec!["Child1".to_string()], &resolver);
    let formatted = format_override_summary(&summary);

    assert!(formatted.contains("=== PrefabInstance: FormattedPrefab ==="));
    assert!(formatted.contains("Source: Assets/Prefabs/Test.prefab"));
    assert!(formatted.contains("1 properties"));
    assert!(formatted.contains("2 stripped refs"));
    assert!(formatted.contains("Child prefabs: Child1"));
}

#[test]
fn test_format_prefab_instance_detail() {
    let yaml = br#"--- !u!4 &600 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 999, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  m_PrefabInstance: {fileID: 9000}
--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 100, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: DetailTest
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_IsActive
      value: 0
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();
    let irs = extract_prefab_instance_irs(&docs, &lines);
    let ir = &irs[0];

    let resolver = |_: &Guid| -> Option<String> { None };
    let stripped = extract_stripped_mappings(&docs, &lines);
    let detail = format_prefab_instance_detail(ir, &resolver, None, &stripped);

    assert!(detail.contains("=== Detail: DetailTest"));
    assert!(detail.contains("m_Name = DetailTest"));
    assert!(detail.contains("m_IsActive = 0"));
    assert!(
        !detail.contains("fileID"),
        "detail leaked fileID:\n{}",
        detail
    );
}

#[test]
fn test_normalize_instance_name() {
    assert_eq!(normalize_instance_name("WoodenChair"), "WoodenChair");
    assert_eq!(normalize_instance_name("WoodenChair (1)"), "WoodenChair");
    assert_eq!(normalize_instance_name("WoodenChair (23)"), "WoodenChair");
    assert_eq!(normalize_instance_name("Torch (0)"), "Torch");
    assert_eq!(normalize_instance_name("LevelRoom_12"), "LevelRoom");
    // Edge cases: should NOT strip
    assert_eq!(normalize_instance_name("Name (abc)"), "Name (abc)");
    assert_eq!(normalize_instance_name("Name ()"), "Name ()");
    assert_eq!(normalize_instance_name("Name (1) extra"), "Name (1) extra");
    assert_eq!(normalize_instance_name("Name_suffix"), "Name_suffix");
    assert_eq!(normalize_instance_name(""), "");
}

#[test]
fn test_hierarchy_summary_groups_repeated_with_instances_and_shared_subtree() {
    let roots = vec![
        HierarchyNode {
            name: "LevelRoom".to_string(),
            file_id: 1,
            components: vec![
                "LevelRoom".to_string(),
                "Rigidbody2D".to_string(),
                "BoxCollider2D".to_string(),
            ],
            is_active: true,
            children: vec![HierarchyNode {
                name: "Mechanism".to_string(),
                file_id: 10,
                is_active: true,
                children: vec![
                    HierarchyNode {
                        name: "Pit".to_string(),
                        file_id: 100,
                        is_active: true,
                        ..Default::default()
                    },
                    HierarchyNode {
                        name: "Pit (1)".to_string(),
                        file_id: 101,
                        is_active: true,
                        ..Default::default()
                    },
                    HierarchyNode {
                        name: "WindBallTriggerTeleport".to_string(),
                        file_id: 102,
                        is_active: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
        HierarchyNode {
            name: "LevelRoom (1)".to_string(),
            file_id: 2,
            components: vec![
                "LevelRoom".to_string(),
                "Rigidbody2D".to_string(),
                "BoxCollider2D".to_string(),
            ],
            is_active: true,
            children: vec![HierarchyNode {
                name: "Mechanism".to_string(),
                file_id: 20,
                is_active: true,
                children: vec![
                    HierarchyNode {
                        name: "Pit".to_string(),
                        file_id: 200,
                        is_active: true,
                        ..Default::default()
                    },
                    HierarchyNode {
                        name: "Pit (1)".to_string(),
                        file_id: 201,
                        is_active: true,
                        ..Default::default()
                    },
                    HierarchyNode {
                        name: "WindBallTriggerTeleport".to_string(),
                        file_id: 202,
                        is_active: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let summary = format_hierarchy_summary(&roots);
    assert!(
        summary.contains("LevelRoom (LevelRoom, Rigidbody2D, BoxCollider2D) ×2"),
        "Should group structurally identical rooms: {}",
        summary
    );
    assert!(
        summary.contains("Instances: LevelRoom, LevelRoom (1)"),
        "Should keep exact instance names for drill-down: {}",
        summary
    );
    assert!(
        summary.contains("Shared subtree:"),
        "Should expose one representative subtree: {}",
        summary
    );
    assert!(
        summary.contains("WindBallTriggerTeleport"),
        "Should keep important child names visible after folding: {}",
        summary
    );
}

#[test]
fn test_hierarchy_summary_keeps_different_subtrees_unfolded() {
    let roots = vec![
        HierarchyNode {
            name: "LevelRoom".to_string(),
            file_id: 1,
            components: vec!["LevelRoom".to_string()],
            is_active: true,
            children: vec![HierarchyNode {
                name: "Mechanism".to_string(),
                file_id: 10,
                is_active: true,
                children: vec![HierarchyNode {
                    name: "WindBallTriggerTeleport".to_string(),
                    file_id: 100,
                    is_active: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
        HierarchyNode {
            name: "LevelRoom (1)".to_string(),
            file_id: 2,
            components: vec!["LevelRoom".to_string()],
            is_active: true,
            children: vec![HierarchyNode {
                name: "Mechanism".to_string(),
                file_id: 20,
                is_active: true,
                children: vec![HierarchyNode {
                    name: "Block".to_string(),
                    file_id: 200,
                    is_active: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    let summary = format_hierarchy_summary(&roots);
    assert!(
        !summary.contains("LevelRoom (LevelRoom) ×2"),
        "Different subtrees must not fold together: {}",
        summary
    );
    assert!(
        summary.contains("LevelRoom (LevelRoom)") && summary.contains("LevelRoom (1) (LevelRoom)"),
        "Each distinct room should stay visible: {}",
        summary
    );
}

#[test]
fn test_hierarchy_summary_filters_by_query_with_ancestor_context() {
    let roots = vec![HierarchyNode {
        name: "Environment".to_string(),
        file_id: 1,
        is_active: true,
        children: vec![
            HierarchyNode {
                name: "Road".to_string(),
                file_id: 2,
                is_active: true,
                children: vec![HierarchyNode {
                    name: "StreetLight_A".to_string(),
                    file_id: 3,
                    components: vec!["Light".to_string()],
                    is_active: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            HierarchyNode {
                name: "Building".to_string(),
                file_id: 4,
                is_active: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let summary = format_hierarchy_summary_with_options(
        &roots,
        &HierarchySummaryOptions {
            query: Some("streetlight".to_string()),
            ..Default::default()
        },
    );

    assert!(
        summary.contains("Environment"),
        "ancestor should remain: {}",
        summary
    );
    assert!(
        summary.contains("Road"),
        "parent should remain: {}",
        summary
    );
    assert!(
        summary.contains("StreetLight_A (Light)"),
        "matching node should remain: {}",
        summary
    );
    assert!(
        !summary.contains("Building"),
        "unmatched sibling should be filtered out: {}",
        summary
    );

    let regex_summary = format_hierarchy_summary_with_options(
        &roots,
        &HierarchySummaryOptions {
            query: Some("re:StreetLight_[A-Z]".to_string()),
            ..Default::default()
        },
    );
    assert!(
        regex_summary.contains("StreetLight_A (Light)"),
        "regex query should match hierarchy nodes: {}",
        regex_summary
    );
}

#[test]
fn test_hierarchy_summary_limits_depth_and_nodes() {
    let roots = vec![
        HierarchyNode {
            name: "RootA".to_string(),
            file_id: 1,
            is_active: true,
            children: vec![HierarchyNode {
                name: "ChildA".to_string(),
                file_id: 2,
                is_active: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        HierarchyNode {
            name: "RootB".to_string(),
            file_id: 3,
            is_active: true,
            ..Default::default()
        },
    ];

    let depth_summary = format_hierarchy_summary_with_options(
        &roots,
        &HierarchySummaryOptions {
            max_depth: Some(1),
            ..Default::default()
        },
    );
    assert!(
        depth_summary.contains("RootA"),
        "root should remain: {}",
        depth_summary
    );
    assert!(
        !depth_summary.contains("ChildA\n"),
        "child should be folded by max_depth: {}",
        depth_summary
    );
    assert!(
        depth_summary.contains("hidden by max_depth"),
        "depth fold count should be shown: {}",
        depth_summary
    );

    let node_summary = format_hierarchy_summary_with_options(
        &roots,
        &HierarchySummaryOptions {
            max_nodes: Some(1),
            ..Default::default()
        },
    );
    assert!(
        node_summary.contains("RootA"),
        "first node should remain: {}",
        node_summary
    );
    assert!(
        !node_summary.contains("RootB"),
        "second root should be hidden by max_nodes: {}",
        node_summary
    );
    assert!(
        node_summary.contains("hidden by max_nodes"),
        "node limit count should be shown: {}",
        node_summary
    );
}

#[test]
fn test_hierarchy_summary_defaults_to_1000_nodes() {
    let roots: Vec<HierarchyNode> = (0..=DEFAULT_HIERARCHY_MAX_NODES)
        .map(|idx| HierarchyNode {
            name: format!("Root{}", idx),
            file_id: idx as i64 + 1,
            is_active: true,
            ..Default::default()
        })
        .collect();

    let summary =
        format_hierarchy_summary_with_options(&roots, &HierarchySummaryOptions::default());

    assert!(
        summary.contains("Root999"),
        "the 1000th root should be printed by default: {}",
        summary
    );
    assert!(
        !summary.contains("Root1000"),
        "nodes beyond the default max_nodes should be hidden: {}",
        summary
    );
    assert!(
        summary.contains("1 hierarchy nodes hidden by max_nodes"),
        "hidden count should reflect the default max_nodes limit: {}",
        summary
    );
}

#[test]
fn test_hierarchy_summary_uses_tree_guides_without_dropping_labels() {
    let roots = vec![HierarchyNode {
        name: "E0002美术".to_string(),
        file_id: 1,
        is_active: true,
        children: vec![HierarchyNode {
            name: "场景模型".to_string(),
            file_id: 2,
            is_active: true,
            children: vec![
                HierarchyNode {
                    name: "路灯".to_string(),
                    file_id: 3,
                    components: vec!["Light".to_string()],
                    is_active: true,
                    ..Default::default()
                },
                HierarchyNode {
                    name: "破碎路灯".to_string(),
                    file_id: 4,
                    tag: Some("Environment".to_string()),
                    layer: Some(0),
                    is_active: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }];

    let summary =
        format_hierarchy_summary_with_options(&roots, &HierarchySummaryOptions::default());

    assert!(
        summary.contains("E0002美术\n└─ 场景模型"),
        "root child should use a tree guide: {}",
        summary
    );
    assert!(
        summary.contains("   ├─ 路灯 (Light)"),
        "component labels should remain after tree prefix: {}",
        summary
    );
    assert!(
        summary.contains("   └─ 破碎路灯  [Tag:Environment, Layer:Default]"),
        "tag/layer annotations should remain after tree prefix: {}",
        summary
    );
}

#[test]
fn test_hierarchy_summary_filters_by_component_and_path_prefix() {
    let roots = vec![HierarchyNode {
        name: "CanvasRoot".to_string(),
        file_id: 1,
        is_active: true,
        children: vec![
            HierarchyNode {
                name: "BattleUI".to_string(),
                file_id: 2,
                components: vec!["Canvas".to_string()],
                is_active: true,
                children: vec![HierarchyNode {
                    name: "HpText".to_string(),
                    file_id: 3,
                    components: vec!["TextMeshProUGUI".to_string()],
                    is_active: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            HierarchyNode {
                name: "MenuUI".to_string(),
                file_id: 4,
                components: vec!["Canvas".to_string()],
                is_active: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let summary = format_hierarchy_summary_with_options(
        &roots,
        &HierarchySummaryOptions {
            path_prefix: Some("CanvasRoot/BattleUI".to_string()),
            component_filters: vec!["TextMeshPro".to_string()],
            ..Default::default()
        },
    );

    assert!(
        summary.contains("BattleUI"),
        "path prefix root should remain: {}",
        summary
    );
    assert!(
        summary.contains("HpText (TextMeshProUGUI)"),
        "component match should remain: {}",
        summary
    );
    assert!(
        !summary.contains("MenuUI"),
        "sibling outside path_prefix should be excluded: {}",
        summary
    );
}

#[test]
fn test_hierarchy_search_uses_ordinal_paths_without_file_ids() {
    let roots = vec![HierarchyNode {
        name: "Root".to_string(),
        file_id: 1,
        is_active: true,
        children: vec![
            HierarchyNode {
                name: "Enemy".to_string(),
                file_id: 2,
                components: vec!["NavMeshAgent".to_string()],
                tag: Some("Enemy".to_string()),
                layer: Some(5),
                is_active: true,
                ..Default::default()
            },
            HierarchyNode {
                name: "Enemy".to_string(),
                file_id: 3,
                components: vec!["Animator".to_string()],
                is_active: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let docs = Vec::new();
    let lines = Vec::new();
    let resolver = |_: &Guid| -> Option<String> { None };

    let results = format_hierarchy_search_results(
        &roots,
        &docs,
        &lines,
        &resolver,
        "Assets/Scenes/Test.unity",
        &HierarchySearchOptions {
            query: Some("Enemy".to_string()),
            ..Default::default()
        },
    );

    assert!(results.contains("Root/Enemy[1]"));
    assert!(results.contains("Root/Enemy[2]"));
    assert!(
        results.contains("- Root/Enemy[1] (NavMeshAgent)  [Tag:Enemy, Layer:UI]"),
        "search should show components/tag/layer inline:\n{}",
        results
    );
    assert!(
        !results.contains("Components:"),
        "search should avoid separate component lines:\n{}",
        results
    );
    assert!(
        !results.contains("State:"),
        "search should avoid separate state lines:\n{}",
        results
    );
    assert!(
        !results.contains("fileID"),
        "search leaked fileID:\n{}",
        results
    );
}

#[test]
fn test_hierarchy_search_can_match_serialized_field_names_and_values() {
    let yaml = br#"--- !u!1 &100
GameObject:
  m_Name: Root
  m_Component:
  - component: {fileID: 101}
  - component: {fileID: 114}
  m_Layer: 0
  m_TagString: Untagged
  m_IsActive: 1
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Children: []
  m_Father: {fileID: 0}
--- !u!114 &114
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 1
  storyNode: E0002_TEST_TEACHER
  nested:
    displayName: Teacher
"#;
    let docs = parse_yaml_docs(yaml);
    let text = String::from_utf8_lossy(yaml);
    let lines: Vec<&str> = text.lines().collect();
    let roots = build_go_tree(&docs);
    let resolver = |_: &Guid| -> Option<String> { None };

    let default_results = format_hierarchy_search_results(
        &roots,
        &docs,
        &lines,
        &resolver,
        "Assets/Scenes/Test.unity",
        &HierarchySearchOptions {
            query: Some("E0002_TEST_TEACHER".to_string()),
            ..Default::default()
        },
    );
    assert!(
        default_results.contains("No hierarchy nodes matched filters."),
        "serialized fields should not match by default:\n{}",
        default_results
    );

    let value_results = format_hierarchy_search_results(
        &roots,
        &docs,
        &lines,
        &resolver,
        "Assets/Scenes/Test.unity",
        &HierarchySearchOptions {
            query: Some("E0002_TEST_TEACHER".to_string()),
            match_fields: vec!["field_value".to_string()],
            ..Default::default()
        },
    );
    assert!(
        value_results.contains("- Root"),
        "field_value should match serialized values:\n{}",
        value_results
    );

    let name_results = format_hierarchy_search_results(
        &roots,
        &docs,
        &lines,
        &resolver,
        "Assets/Scenes/Test.unity",
        &HierarchySearchOptions {
            query: Some("storyNode".to_string()),
            match_fields: vec!["field_name".to_string()],
            ..Default::default()
        },
    );
    assert!(
        name_results.contains("- Root"),
        "field_name should match serialized names:\n{}",
        name_results
    );
}

#[test]
fn test_mesh_blacklist_detection() {
    assert!(is_mesh_data_property("m_PolyMesh.normals.Array.data[0]"));
    assert!(is_mesh_data_property("normals.Array.data[5]"));
    assert!(is_mesh_data_property("vertices.Array.data[100]"));
    assert!(!is_mesh_data_property("m_LocalPosition.x"));
    assert!(!is_mesh_data_property("m_Name"));
}

/// Pin `UnityYamlFile::component_index` to
/// `crate::diff::semantic::scene::build_component_index`. The two
/// implementations exist in parallel because scene.rs builds the index
/// from a borrowed `&[YamlDoc]` slice in the middle of a hot diff path
/// where threading a full `UnityYamlFile` would cause lifetime churn.
/// They must produce identical output for any input — if you change one
/// and not the other this test fails.
///
/// The fixture exercises every branch of the algorithm:
/// - GameObject doc (`class_id == 1`) — must self-insert under its own
///   fileID
/// - PrefabInstance doc (`class_id == 1001`) — same self-insert rule
/// - Component docs with `m_GameObject` set (Transform `class_id 4`,
///   MonoBehaviour `class_id 114`, MeshRenderer `class_id 23`) — must
///   insert under the parent GO's fileID
/// - A second GameObject so the index has multiple keys to compare
#[test]
fn component_index_matches_scene_impl() {
    use super::index::UnityYamlFile;

    let content = br#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &100
GameObject:
  m_ObjectHideFlags: 0
  m_Name: Player
  m_TagString: Untagged
  m_Layer: 0
  m_IsActive: 1
--- !u!4 &200
Transform:
  m_GameObject: {fileID: 100}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_Father: {fileID: 0}
--- !u!114 &300
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
--- !u!1 &400
GameObject:
  m_ObjectHideFlags: 0
  m_Name: Camera
  m_Layer: 0
  m_IsActive: 1
--- !u!23 &500
MeshRenderer:
  m_GameObject: {fileID: 400}
  m_Enabled: 1
--- !u!1001 &600
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    serializedVersion: 3
    m_TransformParent: {fileID: 0}
    m_Modifications: []
    m_RemovedComponents: []
  m_SourcePrefab: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
"#;

    let file = UnityYamlFile::parse(content);
    let scene_idx = crate::diff::semantic::scene::build_component_index(&file.docs);

    assert_eq!(
        file.component_index, scene_idx,
        "UnityYamlFile::component_index drifted from \
         diff::semantic::scene::build_component_index. Update both \
         implementations together."
    );

    // Sanity assertions on the *expected* shape — guards against the
    // case where both impls are identically wrong.
    assert!(
        file.component_index.contains_key(&100),
        "GameObject 100 (Player) should appear in component_index"
    );
    assert!(
        file.component_index.contains_key(&400),
        "GameObject 400 (Camera) should appear in component_index"
    );
    assert!(
        file.component_index.contains_key(&600),
        "PrefabInstance 600 should self-insert under its own fileID"
    );
    // GameObject 100 owns 3 docs: itself + Transform + MonoBehaviour.
    assert_eq!(file.component_index.get(&100).map(Vec::len), Some(3));
    // GameObject 400 owns 2 docs: itself + MeshRenderer.
    assert_eq!(file.component_index.get(&400).map(Vec::len), Some(2));
    // PrefabInstance 600 owns 1 doc: itself.
    assert_eq!(file.component_index.get(&600).map(Vec::len), Some(1));
}
