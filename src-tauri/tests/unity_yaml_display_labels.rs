use locus_lib::asset_db::types::{guid_to_hex, parse_guid_hex, Guid};
use locus_lib::asset_db::AssetDb;
use locus_lib::unity_yaml::{
    build_internal_id_map, extract_prefab_instance_irs, extract_stripped_mappings,
    format_doc_display_label, format_prefab_instance_detail, parse_yaml_docs,
    resolve_references_in_lines,
};

#[test]
fn mono_behaviour_semantic_names_label_documents_and_internal_refs() {
    let yaml = br#"--- !u!114 &100
MonoBehaviour:
  m_Name: New Group
  _parent: {fileID: 200}
  _tracks:
  - {fileID: 200}
--- !u!114 &200
MonoBehaviour:
  m_Name: HitTrack
"#;
    let docs = parse_yaml_docs(yaml);

    assert_eq!(
        format_doc_display_label(&docs[0]),
        "New Group (MonoBehaviour)"
    );
    assert_eq!(
        format_doc_display_label(&docs[1]),
        "HitTrack (MonoBehaviour)"
    );

    let internal_map = build_internal_id_map(&docs);
    assert_eq!(
        internal_map.get(&100).map(String::as_str),
        Some("New Group (MonoBehaviour)")
    );
    assert_eq!(
        internal_map.get(&200).map(String::as_str),
        Some("HitTrack (MonoBehaviour)")
    );

    let lines: Vec<&str> = std::str::from_utf8(yaml).unwrap().lines().collect();
    let resolved = resolve_references_in_lines(
        &lines,
        docs[0].line_start + 2,
        docs[0].line_end,
        &|_, _| None,
        &|file_id| internal_map.get(&file_id).cloned(),
    );
    assert!(resolved.contains("_parent: {HitTrack (MonoBehaviour)}"));
    assert!(resolved.contains("- {HitTrack (MonoBehaviour)}"));
}

#[test]
fn external_references_use_file_id_to_distinguish_importer_subassets() {
    let yaml = br#"--- !u!114 &100
MonoBehaviour:
  idle: {fileID: 7400000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  dash: {fileID: 7400001, guid: aabbccdd11223344aabbccdd11223344,
    type: 3}
"#;
    let lines: Vec<&str> = std::str::from_utf8(yaml).unwrap().lines().collect();
    let resolved = resolve_references_in_lines(
        &lines,
        2,
        lines.len(),
        &|guid, file_id| {
            if guid != "aabbccdd11223344aabbccdd11223344" {
                return None;
            }
            match file_id {
                Some(7_400_000) => Some("Assets/Models/Hero.fbx/Idle".to_string()),
                Some(7_400_001) => Some("Assets/Models/Hero.fbx/Light_DashAttack.1".to_string()),
                _ => None,
            }
        },
        &|_| None,
    );

    assert!(resolved.contains("idle: {Assets/Models/Hero.fbx/Idle}"));
    assert!(resolved.contains("dash: {Assets/Models/Hero.fbx/Light_DashAttack.1}"));
}

#[test]
fn mono_behaviour_display_label_falls_back_without_semantic_name() {
    let docs = parse_yaml_docs(
        br#"--- !u!114 &100
MonoBehaviour:
  m_Name:
"#,
    );

    assert_eq!(format_doc_display_label(&docs[0]), "MonoBehaviour");
}

#[test]
fn asset_db_resolves_model_importer_subassets_by_guid_and_file_id() {
    let project = tempfile::tempdir().unwrap();
    let assets_dir = project.path().join("Assets/Models");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(assets_dir.join("Hero.fbx"), b"placeholder model").unwrap();
    std::fs::write(
        assets_dir.join("Hero.fbx.meta"),
        br#"fileFormatVersion: 2
guid: aabbccdd11223344aabbccdd11223344
ModelImporter:
  internalIDToNameTable:
  - first:
      74: 7400000
    second: Idle
  - first:
      74: 7400001
    second: Light_DashAttack.1
"#,
    )
    .unwrap();

    let mut db = AssetDb::open(project.path()).unwrap();
    db.full_scan(|_| {}).unwrap();
    let guid = parse_guid_hex("aabbccdd11223344aabbccdd11223344").unwrap();
    let resolved = db
        .batch_resolve_asset_objects(&[(guid, 7_400_000), (guid, 7_400_001)])
        .unwrap();

    let idle = resolved.get(&(guid, 7_400_000)).unwrap();
    assert_eq!(idle.path, "Assets/Models/Hero.fbx");
    assert_eq!(idle.name, "Idle");
    assert_eq!(idle.type_name, "AnimationClip");
    assert!(idle.is_sub_asset);

    let dash = resolved.get(&(guid, 7_400_001)).unwrap();
    assert_eq!(dash.name, "Light_DashAttack.1");
    assert_eq!(dash.type_name, "AnimationClip");
    assert!(dash.is_sub_asset);
}

#[test]
fn prefab_override_object_references_resolve_subassets() {
    let yaml = br#"--- !u!1001 &9000
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Animation
      value:
      objectReference: {fileID: 7400001, guid: ccccddddccccddddccccddddccccdddd, type: 3}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
    let docs = parse_yaml_docs(yaml);
    let lines: Vec<&str> = std::str::from_utf8(yaml).unwrap().lines().collect();
    let instances = extract_prefab_instance_irs(&docs, &lines);
    let stripped = extract_stripped_mappings(&docs, &lines);
    let guid_resolver = |_: &Guid| -> Option<String> { None };
    let object_resolver = |guid: &Guid, file_id: i64| -> Option<String> {
        (guid_to_hex(guid) == "ccccddddccccddddccccddddccccdddd" && file_id == 7_400_001)
            .then(|| "Assets/Models/Hero.fbx/Light_DashAttack.1".to_string())
    };

    let detail = format_prefab_instance_detail(
        &instances[0],
        &guid_resolver,
        &object_resolver,
        None,
        &stripped,
    );

    assert!(detail.contains("m_Animation"));
    assert!(detail.contains("{Assets/Models/Hero.fbx/Light_DashAttack.1}"));
}
