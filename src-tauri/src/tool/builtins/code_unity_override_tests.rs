//! `[SerializeReference]` data reached through prefab instances: type
//! replacements and `managedReferences[rid].field` overrides in scenes,
//! variants and nested prefabs. Synthetic fixtures mirror the YAML Unity 6.5
//! writes (verified with a throwaway project).

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::code_unity::run_code_usages;
use super::code_unity_managed::{
    check_managed_reference_types, resolve_rid_type, RidType, SharedAssetDb,
};
use crate::asset_db::types::parse_guid_hex;
use crate::asset_db::AssetDb;
use crate::unity_yaml::managed_refs::ManagedTypeIdentity;

const HOLDER: &str = "a0a0a0a0a0a0a0a0a0a0a0a0a0a00101";
const BASE: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b00001";
const VARIANT: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b00002";
const OUTER: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b00003";
const SCENE: &str = "c0c0c0c0c0c0c0c0c0c0c0c0c0c00001";

const GO: i64 = 1383303035007636456;
const HOLDER_ID: i64 = 4954498630159034338;
const VARIANT_PI: i64 = 1675151794310873844;
const VARIANT_HOLDER: i64 = 6052485984431105302;
const OUTER_PI: i64 = 4212884190871731329;
const OUTER_HOLDER: i64 = 9130698156442213219;

const RID_BURN: i64 = 7542395272935243776;
const RID_FREEZE: i64 = 7542395272935243777;
const RID_VARIANT_FREEZE: i64 = 7542395272935243780;
const RID_CHAIN: i64 = 7542395272935243781;
const RID_NEXT: i64 = 7542395272935243783;

fn write_asset(root: &Path, rel: &str, content: &str, guid: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    std::fs::write(
        root.join(format!("{rel}.meta")),
        format!("fileFormatVersion: 2\nguid: {guid}\n"),
    )
    .unwrap();
}

fn modification(target_guid: &str, target_file_id: i64, path: &str, value: &str) -> String {
    let path = if path.ends_with(']') {
        format!("'{path}'")
    } else {
        path.to_string()
    };
    format!(
        "    - target: {{fileID: {target_file_id}, guid: {target_guid}, type: 3}}\n      propertyPath: {path}\n      value: {value}\n      objectReference: {{fileID: 0}}\n"
    )
}

fn prefab_instance(file_id: i64, name: &str, source: &str, mods: &[String]) -> String {
    format!(
        "--- !u!1001 &{file_id}\nPrefabInstance:\n  serializedVersion: 2\n  m_Modification:\n    serializedVersion: 3\n    m_TransformParent: {{fileID: 0}}\n    m_Modifications:\n{}{}    m_RemovedComponents: []\n  m_SourcePrefab: {{fileID: 100100000, guid: {source}, type: 3}}\n",
        modification(source, GO, "m_Name", name),
        mods.concat()
    )
}

fn fixture() -> (tempfile::TempDir, SharedAssetDb, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("Assets")).unwrap();

    let script = |rel: &str, body: &str, guid: &str| {
        write_asset(
            &root,
            rel,
            &format!(
                "using System;\nusing System.Collections.Generic;\nusing UnityEngine;\n{body}\n"
            ),
            guid,
        )
    };
    script(
        "Assets/Scripts/Fx/Effect.cs",
        "namespace Game.Fx { [Serializable] public abstract class Effect { public int power; } }",
        "d0d0d0d0d0d0d0d0d0d0d0d0d0d00001",
    );
    script(
        "Assets/Scripts/Fx/Burn.cs",
        "namespace Game.Fx { [Serializable] public class Burn : Effect { public float duration; } }",
        "d0d0d0d0d0d0d0d0d0d0d0d0d0d00002",
    );
    script(
        "Assets/Scripts/Fx/Freeze.cs",
        "namespace Game.Fx { [Serializable] public class Freeze : Effect { public int stacks; } }",
        "d0d0d0d0d0d0d0d0d0d0d0d0d0d00003",
    );
    script(
        "Assets/Scripts/Fx/Chain.cs",
        "namespace Game.Fx { [Serializable] public class Chain : Effect { [SerializeReference] public Effect next; } }",
        "d0d0d0d0d0d0d0d0d0d0d0d0d0d00004",
    );
    script(
        "Assets/Scripts/Holder.cs",
        "using Game.Fx;\npublic class Holder : MonoBehaviour\n{\n    [SerializeReference] public Effect main;\n    [SerializeReference] public List<Effect> extra;\n}",
        HOLDER,
    );

    write_asset(
        &root,
        "Assets/Prefabs/Base.prefab",
        &format!(
            "%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &{GO}
GameObject:
  m_Component:
  - component: {{fileID: 4631236571115629623}}
  - component: {{fileID: {HOLDER_ID}}}
  m_Name: Holder
--- !u!4 &4631236571115629623
Transform:
  m_GameObject: {{fileID: {GO}}}
  m_Father: {{fileID: 0}}
--- !u!114 &{HOLDER_ID}
MonoBehaviour:
  m_GameObject: {{fileID: {GO}}}
  m_Script: {{fileID: 11500000, guid: {HOLDER}, type: 3}}
  m_Name:
  main:
    rid: {RID_BURN}
  extra:
  - rid: {RID_FREEZE}
  references:
    version: 2
    RefIds:
    - rid: {RID_BURN}
      type: {{class: Burn, ns: Game.Fx, asm: Assembly-CSharp}}
      data:
        power: 1
        duration: 2
    - rid: {RID_FREEZE}
      type: {{class: Freeze, ns: Game.Fx, asm: Assembly-CSharp}}
      data:
        power: 3
        stacks: 4
"
        ),
        BASE,
    );

    // Variant: replaces `main` with a new Freeze instance.
    write_asset(
        &root,
        "Assets/Prefabs/Variant.prefab",
        &format!(
            "%YAML 1.1\n{}",
            prefab_instance(
                VARIANT_PI,
                "Variant",
                BASE,
                &[
                    modification(BASE, HOLDER_ID, "main", &RID_VARIANT_FREEZE.to_string()),
                    modification(
                        BASE,
                        HOLDER_ID,
                        &format!("managedReferences[{RID_VARIANT_FREEZE}]"),
                        "Assembly-CSharp Game.Fx.Freeze",
                    ),
                    modification(
                        BASE,
                        HOLDER_ID,
                        &format!("managedReferences[{RID_VARIANT_FREEZE}].power"),
                        "11",
                    ),
                    modification(
                        BASE,
                        HOLDER_ID,
                        &format!("managedReferences[{RID_VARIANT_FREEZE}].stacks"),
                        "12",
                    ),
                ],
            )
        ),
        VARIANT,
    );

    // Outer prefab nesting Base, overriding the nested Burn's power.
    write_asset(
        &root,
        "Assets/Prefabs/Outer.prefab",
        &format!(
            "%YAML 1.1\n--- !u!1 &10\nGameObject:\n  m_Name: Outer\n{}",
            prefab_instance(
                OUTER_PI,
                "Holder",
                BASE,
                &[modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_BURN}].power"),
                    "21",
                )],
            )
        ),
        OUTER,
    );

    let scene = [
        prefab_instance(
            1369411486,
            "FieldOverride",
            BASE,
            &[modification(
                BASE,
                HOLDER_ID,
                &format!("managedReferences[{RID_BURN}].power"),
                "31",
            )],
        ),
        prefab_instance(
            29641039,
            "TypeOverride",
            BASE,
            &[
                modification(BASE, HOLDER_ID, "main", &RID_CHAIN.to_string()),
                modification(BASE, HOLDER_ID, "extra.Array.data[0]", "-2"),
                modification(BASE, HOLDER_ID, "managedReferences[-2]", ""),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_CHAIN}]"),
                    "Assembly-CSharp Game.Fx.Chain",
                ),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_NEXT}]"),
                    "Assembly-CSharp Game.Fx.Freeze",
                ),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_CHAIN}].next"),
                    &RID_NEXT.to_string(),
                ),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_CHAIN}].power"),
                    "41",
                ),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_NEXT}].stacks"),
                    "43",
                ),
            ],
        ),
        prefab_instance(
            999063860,
            "VariantInstance",
            VARIANT,
            &[modification(
                VARIANT,
                VARIANT_HOLDER,
                &format!("managedReferences[{RID_VARIANT_FREEZE}].stacks"),
                "51",
            )],
        ),
        prefab_instance(
            2073138431,
            "OuterInstance",
            OUTER,
            &[modification(
                OUTER,
                OUTER_HOLDER,
                &format!("managedReferences[{RID_BURN}].power"),
                "61",
            )],
        ),
        prefab_instance(
            55,
            "Ghosted",
            BASE,
            &[modification(
                BASE,
                HOLDER_ID,
                "managedReferences[900]",
                "Assembly-CSharp Game.Fx.Ghost",
            )],
        ),
    ]
    .concat();
    write_asset(
        &root,
        "Assets/Scenes/Arena.unity",
        &format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n{scene}"),
        SCENE,
    );

    let mut db = AssetDb::open(&root).expect("open asset db");
    db.full_scan(|_| {}).expect("scan");
    let root_text = root.to_string_lossy().to_string();
    (temp, Arc::new(Mutex::new(Some(db))), root_text)
}

fn usages(db: &SharedAssetDb, root: &str, file: &str, member: Option<&str>) -> String {
    let result = run_code_usages(Arc::clone(db), root, file, member, 100);
    assert!(
        !result.is_error,
        "unity_code_usages failed: {}",
        result.output
    );
    result.output
}

fn lines_with<'a>(output: &'a str, needle: &str) -> Vec<&'a str> {
    output
        .lines()
        .filter(|line| line.contains(needle))
        .collect()
}

fn identity(class: &str) -> RidType {
    RidType::Type(ManagedTypeIdentity {
        class_name: class.into(),
        namespace: "Game.Fx".into(),
        assembly: "Assembly-CSharp".into(),
    })
}

#[test]
fn rid_types_resolve_through_variants_nested_prefabs_and_registries() {
    let (_temp, db, _root) = fixture();
    let guard = db.lock().unwrap();
    let db = guard.as_ref().unwrap();
    let base = parse_guid_hex(BASE).unwrap();
    let variant = parse_guid_hex(VARIANT).unwrap();
    let outer = parse_guid_hex(OUTER).unwrap();

    assert_eq!(
        resolve_rid_type(db, &base, HOLDER_ID, RID_BURN).unwrap(),
        Some(identity("Burn"))
    );
    // Introduced by the variant's own type replacement.
    assert_eq!(
        resolve_rid_type(db, &variant, VARIANT_HOLDER, RID_VARIANT_FREEZE).unwrap(),
        Some(identity("Freeze"))
    );
    // Nested object without a document: XOR fileID back to Base's registry.
    assert_eq!(
        resolve_rid_type(db, &outer, OUTER_HOLDER, RID_FREEZE).unwrap(),
        Some(identity("Freeze"))
    );
    assert_eq!(
        resolve_rid_type(db, &base, HOLDER_ID, -2).unwrap(),
        Some(RidType::Null)
    );
    assert_eq!(resolve_rid_type(db, &base, HOLDER_ID, 12345).unwrap(), None);
    assert_eq!(resolve_rid_type(db, &outer, 777, RID_BURN).unwrap(), None);
}

#[test]
fn type_replacements_count_as_instances_with_owning_field() {
    let (_temp, db, root) = fixture();
    let output = usages(&db, &root, "Assets/Scripts/Fx/Freeze.cs", None);

    assert!(
        output.contains("3 [SerializeReference] instances"),
        "{output}"
    );
    let registry = lines_with(
        &output,
        "[SerializeReference] Game.Fx.Freeze (rid 7542395272935243777)",
    );
    assert_eq!(registry.len(), 1, "{output}");
    assert!(registry[0].contains("via extra[0]"), "{}", registry[0]);

    let variant = lines_with(
        &output,
        "[SerializeReference override] Game.Fx.Freeze (rid 7542395272935243780)",
    );
    assert_eq!(variant.len(), 1, "{output}");
    assert!(
        variant[0].contains("prefab instance 'Variant' of Assets/Prefabs/Base.prefab sets this instance type via main"),
        "{}",
        variant[0]
    );

    let nested = lines_with(&output, "(rid 7542395272935243783)");
    assert_eq!(nested.len(), 1, "{output}");
    assert!(
        nested[0].contains("prefab instance 'TypeOverride'")
            && nested[0].contains("via managedReferences[7542395272935243781].next"),
        "{}",
        nested[0]
    );
    // Null replacement and the unrelated missing type are not instances.
    assert!(!output.contains("rid -2"));
    assert!(!output.contains("Ghost"));
}

#[test]
fn base_type_query_includes_override_only_subclass_instances() {
    let (_temp, db, root) = fixture();
    let output = usages(&db, &root, "Assets/Scripts/Fx/Effect.cs", None);
    assert!(
        output.contains("5 [SerializeReference] instances of 3 serialized types"),
        "{output}"
    );
    let chain = lines_with(&output, "Game.Fx.Chain (rid 7542395272935243781)");
    assert_eq!(chain.len(), 1, "{output}");
    assert!(
        chain[0].contains("subclass/implementer of Game.Fx.Effect"),
        "{}",
        chain[0]
    );
    assert!(chain[0].contains("via main"), "{}", chain[0]);
}

#[test]
fn member_query_follows_overrides_to_their_effective_types() {
    let (_temp, db, root) = fixture();
    let power = usages(&db, &root, "Assets/Scripts/Fx/Effect.cs", Some("power"));
    assert!(
        power.starts_with("7 serialized usages of 'Effect.power'"),
        "{power}"
    );
    for (needle, class) in [
        (
            "managedReferences[7542395272935243780].power: 11",
            "Game.Fx.Freeze",
        ),
        (
            "managedReferences[7542395272935243776].power: 21",
            "Game.Fx.Burn",
        ),
        (
            "managedReferences[7542395272935243776].power: 31",
            "Game.Fx.Burn",
        ),
        (
            "managedReferences[7542395272935243781].power: 41",
            "Game.Fx.Chain",
        ),
        (
            "managedReferences[7542395272935243776].power: 61",
            "Game.Fx.Burn",
        ),
    ] {
        let found = lines_with(&power, needle);
        assert_eq!(found.len(), 1, "{needle} in:\n{power}");
        assert!(found[0].contains(class), "{}", found[0]);
    }
    assert_eq!(
        lines_with(&power, "[SerializeReference field] power:").len(),
        2
    );
    assert!(lines_with(&power, "power: 61")[0]
        .contains("prefab instance 'OuterInstance' of Assets/Prefabs/Outer.prefab"));

    let stacks = usages(&db, &root, "Assets/Scripts/Fx/Freeze.cs", Some("stacks"));
    assert!(
        stacks.starts_with("4 serialized usages of 'Freeze.stacks'"),
        "{stacks}"
    );
    assert_eq!(lines_with(&stacks, "stacks: 51").len(), 1, "{stacks}");
    assert_eq!(lines_with(&stacks, "stacks: 43").len(), 1, "{stacks}");
    assert_eq!(lines_with(&stacks, "stacks: 12").len(), 1, "{stacks}");

    // Burn's file does not declare `stacks`, and no Burn instance has it.
    let burn = usages(&db, &root, "Assets/Scripts/Fx/Burn.cs", Some("stacks"));
    assert!(burn.starts_with("No serialized usages"), "{burn}");
}

#[test]
fn type_check_reports_override_only_missing_types_with_samples() {
    let (temp, db, _root) = fixture();
    let root = temp.path().join("project");
    let report = check_managed_reference_types(&db, &root, None)
        .unwrap()
        .unwrap();
    let issues: Vec<String> = report
        .issues
        .iter()
        .map(|issue| issue.identity.display_name())
        .collect();
    assert_eq!(issues, vec!["Game.Fx.Ghost".to_string()]);
    let ghost = &report.issues[0];
    assert_eq!(ghost.instance_count, 1);
    assert_eq!(ghost.samples.len(), 1);
    assert_eq!(ghost.samples[0].0, "Assets/Scenes/Arena.unity");
    // Burn, Freeze and Chain (registry + overrides) all resolve.
    assert_eq!(report.checked_types, 4);
}
