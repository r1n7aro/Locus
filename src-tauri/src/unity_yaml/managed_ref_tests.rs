//! Coverage for `[SerializeReference]` registry capture
//! (`parse_yaml_docs_for_index` → `managed_refs`) and the class-name helpers.

use super::managed_refs::{
    managed_class_key, managed_class_segments, parse_managed_type_flow, ManagedClassSegment,
    RawManagedRef,
};
use super::parse_yaml_docs_for_index;

fn captured(yaml: &str) -> Vec<RawManagedRef> {
    parse_yaml_docs_for_index(yaml.as_bytes()).managed_refs
}

#[test]
fn captures_v2_registry_entries_with_host_and_entry_line() {
    let yaml = "%YAML 1.1
--- !u!114 &11400000
MonoBehaviour:
  m_Script: {fileID: 11500000, guid: 1111111111111111111111111111aaaa, type: 3}
  m_Name: ComboStrike
  effects:
  - rid: 1000
  - rid: 1001
  references:
    version: 2
    RefIds:
    - rid: 1000
      type: {class: HitEvent, ns: Game.Combat, asm: Game.Combat}
      data:
        startTime: 0
        child: {rid: 1001}
    - rid: 1001
      type: {class: TurnEffect, ns: Game.Combat, asm: Game.Combat}
      data:
        turnMode: 1
";
    let refs = captured(yaml);
    assert_eq!(
        refs,
        vec![
            RawManagedRef {
                host_file_id: 11400000,
                rid: 1000,
                class_name: "HitEvent".into(),
                namespace: "Game.Combat".into(),
                assembly: "Game.Combat".into(),
                line: 12,
            },
            RawManagedRef {
                host_file_id: 11400000,
                rid: 1001,
                class_name: "TurnEffect".into(),
                namespace: "Game.Combat".into(),
                assembly: "Game.Combat".into(),
                line: 17,
            },
        ]
    );
}

#[test]
fn captures_v1_registry_and_skips_terminus_sentinel() {
    let yaml = "%YAML 1.1
--- !u!114 &200
MonoBehaviour:
  m_Script: {fileID: 11500000, guid: 1111111111111111111111111111aaaa, type: 3}
  effect:
    id: 0
  references:
    version: 1
    00000000:
      type: {class: Burn, ns: Fx, asm: Assembly-CSharp}
      data:
        damage: 3
    00000001:
      type: {class: Terminus, ns: UnityEngine.DMAT, asm: FAKE_ASM}
";
    let refs = captured(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].rid, 0);
    assert_eq!(refs[0].class_name, "Burn");
    assert_eq!(refs[0].namespace, "Fx");
    assert_eq!(refs[0].assembly, "Assembly-CSharp");
    assert_eq!(refs[0].host_file_id, 200);
    assert_eq!(refs[0].line, 9);
}

#[test]
fn skips_null_entries_and_keeps_scanning() {
    let yaml = "--- !u!114 &1
MonoBehaviour:
  slot: {rid: -2}
  references:
    version: 2
    RefIds:
    - rid: -2
      type: {class: , ns: , asm: }
    - rid: 5
      type: {class: Live, ns: , asm: Assembly-CSharp}
";
    let refs = captured(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].rid, 5);
    assert_eq!(refs[0].class_name, "Live");
    assert_eq!(refs[0].namespace, "");
}

#[test]
fn joins_wrapped_type_flow_maps() {
    // Unity wraps long flow maps onto an indented continuation line.
    let yaml = "--- !u!114 &7
MonoBehaviour:
  references:
    version: 2
    RefIds:
    - rid: 42
      type: {class: GeneratedKnockbackResponseEffect, ns: Studio.Gameplay.Combat.Very.Long,
        asm: Studio.Gameplay}
      data:
        startTime: 0
";
    let refs = captured(yaml);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].class_name, "GeneratedKnockbackResponseEffect");
    assert_eq!(refs[0].namespace, "Studio.Gameplay.Combat.Very.Long");
    assert_eq!(refs[0].assembly, "Studio.Gameplay");
    assert_eq!(refs[0].line, 6);
}

#[test]
fn keeps_quoted_generic_and_nested_class_names() {
    let yaml = "--- !u!114 &9
MonoBehaviour:
  references:
    version: 2
    RefIds:
    - rid: 1
      type: {class: 'Box`1[[System.Int32, mscorlib]]', ns: Game, asm: Assembly-CSharp}
    - rid: 2
      type: {class: Config/Node, ns: Demo, asm: Assembly-CSharp}
";
    let refs = captured(yaml);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].class_name, "Box`1[[System.Int32, mscorlib]]");
    assert_eq!(refs[0].namespace, "Game");
    assert_eq!(refs[1].class_name, "Config/Node");
}

#[test]
fn ignores_user_fields_named_references_and_host_rid_lists() {
    // A user list field called `references` and host-side `- rid:` links at
    // doc level must not be mistaken for registry entries; neither must a
    // non-MonoBehaviour document.
    let yaml = "--- !u!114 &1
MonoBehaviour:
  references:
  - {fileID: 0}
  items:
  - rid: 3
  type: {class: NotAnEntry, ns: X, asm: Y}
--- !u!1 &2
GameObject:
  references:
    version: 2
    RefIds:
    - rid: 1
      type: {class: Wrong, ns: X, asm: Y}
";
    assert!(captured(yaml).is_empty());
}

#[test]
fn registry_ends_at_next_document_and_tracks_each_host() {
    let yaml = "--- !u!114 &10
MonoBehaviour:
  references:
    version: 2
    RefIds:
    - rid: 1
      type: {class: A, ns: N, asm: Asm}
--- !u!114 &20
MonoBehaviour:
  m_Name: Second
  references:
    version: 2
    RefIds:
    - rid: 1
      type: {class: B, ns: N, asm: Asm}
";
    let refs = captured(yaml);
    assert_eq!(refs.len(), 2);
    assert_eq!(
        (refs[0].host_file_id, refs[0].class_name.as_str()),
        (10, "A")
    );
    assert_eq!(
        (refs[1].host_file_id, refs[1].class_name.as_str()),
        (20, "B")
    );
}

#[test]
fn non_index_parse_entry_points_are_unchanged() {
    let yaml = b"--- !u!114 &10\nMonoBehaviour:\n  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: A, ns: N, asm: Asm}\n";
    let docs = super::parse_yaml_docs(yaml);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].file_id, 10);
    let (docs, _refs, bindings) = super::parse_yaml_docs_with_refs_and_bindings(yaml);
    assert_eq!(docs.len(), 1);
    assert!(bindings.is_empty());
}

#[test]
fn parses_type_flow_values() {
    let identity =
        parse_managed_type_flow(" {class: 'Box`1[[System.Int32, mscorlib]]', ns: Game, asm: Asm}")
            .expect("flow map");
    assert_eq!(identity.class_name, "Box`1[[System.Int32, mscorlib]]");
    assert_eq!(identity.namespace, "Game");
    assert_eq!(identity.assembly, "Asm");
    assert_eq!(
        identity.display_name(),
        "Game.Box`1[[System.Int32, mscorlib]]"
    );

    // Incomplete (still wrapped) maps are rejected so the caller retries.
    assert!(parse_managed_type_flow(" {class: A, ns: N,").is_none());
    assert!(parse_managed_type_flow(" {ns: N, asm: A}").is_none());
}

#[test]
fn decomposes_nested_and_generic_class_names() {
    assert_eq!(
        managed_class_segments("Outer/Box`2[[System.Int32, mscorlib],[Game.Item, Asm]]"),
        vec![
            ManagedClassSegment {
                name: "Outer".into(),
                arity: 0
            },
            ManagedClassSegment {
                name: "Box".into(),
                arity: 2
            },
        ]
    );
    assert_eq!(managed_class_segments("Outer+Inner").len(), 2);
    assert_eq!(
        managed_class_key("Outer/Box`1[[System.Int32, mscorlib]]"),
        "box"
    );
    assert_eq!(managed_class_key("FaceTargetEffect"), "facetargeteffect");
    assert_eq!(managed_class_key(""), "");
}
