//! Plain `[Serializable]` values in `unity_code_usages`: routes through
//! nested types, arrays, lists and project generics; derived scripts;
//! `[SerializeReference]` hosts; and prefab overrides along those routes.
//! All fixtures are synthetic.

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::code_unity::run_code_usages;
use super::code_unity_managed::SharedAssetDb;
use super::code_unity_plain::{
    path_follows_route, values_in_document, values_in_registry_entry, PathStep,
};
use crate::asset_db::AssetDb;
use crate::unity_yaml::prefab_overrides::tokenize_property_path;

const HOLDER: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e10001";
const BOSS: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e10002";
const SEQUENCE: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e10003";
const BASE: &str = "f1f1f1f1f1f1f1f1f1f1f1f1f1f10001";
const VARIANT: &str = "f1f1f1f1f1f1f1f1f1f1f1f1f1f10002";

const GO: i64 = 1383303035007636456;
const HOLDER_ID: i64 = 4954498630159034338;
const VARIANT_PI: i64 = 1675151794310873844;
const VARIANT_HOLDER: i64 = 6052485984431105302;
const RID_BURN: i64 = 7542395272935243776;

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
    format!(
        "    - target: {{fileID: {target_file_id}, guid: {target_guid}, type: 3}}\n      propertyPath: {path}\n      value: {value}\n      objectReference: {{fileID: 0}}\n"
    )
}

fn prefab_instance(file_id: i64, name: &str, source: &str, mods: &[String]) -> String {
    format!(
        "--- !u!1001 &{file_id}\nPrefabInstance:\n  m_Modification:\n    m_TransformParent: {{fileID: 0}}\n    m_Modifications:\n{}{}    m_RemovedComponents: []\n  m_SourcePrefab: {{fileID: 100100000, guid: {source}, type: 3}}\n",
        modification(source, GO, "m_Name", name),
        mods.concat()
    )
}

fn holder_component(script: &str, extra: &str) -> String {
    format!(
        "--- !u!1 &{GO}
GameObject:
  m_Component:
  - component: {{fileID: {HOLDER_ID}}}
  m_Name: Holder
--- !u!114 &{HOLDER_ID}
MonoBehaviour:
  m_GameObject: {{fileID: {GO}}}
  m_Script: {{fileID: 11500000, guid: {script}, type: 3}}
  m_Name:
  tuning:
    range:
      min: 0
      max: 1
    steps:
    - min: 1
      max: 2
  ranges:
  - min: 5
    max: 6
{extra}"
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
        "Assets/Scripts/Data/Range.cs",
        "namespace Game.Data { [Serializable] public struct Range { public float min; [UnityEngine.Serialization.FormerlySerializedAs(\"top\")] public float max; } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10001",
    );
    script(
        "Assets/Scripts/Data/Tuning.cs",
        "namespace Game.Data { [Serializable] public class Tuning { public Range range; public List<Range> steps = new List<Range>(); } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10002",
    );
    script(
        "Assets/Scripts/Data/Slot.cs",
        "namespace Game.Data { [Serializable] public class Slot<T> { public T value; } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10003",
    );
    script(
        "Assets/Scripts/Data/Unused.cs",
        "namespace Game.Data { [Serializable] public class Unused { public int x; } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10004",
    );
    script(
        "Assets/Scripts/Data/Plain.cs",
        "namespace Game.Data { public class Plain { public int x; } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10005",
    );
    script(
        "Assets/Scripts/Fx/Effect.cs",
        "using Game.Data;\nnamespace Game.Fx { [Serializable] public abstract class Effect { public Range window; } [Serializable] public class Burn : Effect { public int heat; } }",
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a10006",
    );
    script(
        "Assets/Scripts/Holder.cs",
        "using Game.Data;\nusing Game.Fx;\npublic class Holder : MonoBehaviour\n{\n    public Tuning tuning;\n    public Range[] ranges;\n    public Slot<Range> slot;\n    [SerializeReference] public Effect main;\n}",
        HOLDER,
    );
    script(
        "Assets/Scripts/BossHolder.cs",
        "public class BossHolder : Holder { public int phase; }",
        BOSS,
    );
    script(
        "Assets/Scripts/Sequence.cs",
        "using Game.Data;\npublic class Sequence : ScriptableObject { public Tuning tuning; }",
        SEQUENCE,
    );

    write_asset(
        &root,
        "Assets/Prefabs/Base.prefab",
        &format!(
            "%YAML 1.1\n{}",
            holder_component(
                HOLDER,
                &format!(
                    "  slot:
    value:
      min: 8
      max: 9
  main:
    rid: {RID_BURN}
  references:
    version: 2
    RefIds:
    - rid: {RID_BURN}
      type: {{class: Burn, ns: Game.Fx, asm: Assembly-CSharp}}
      data:
        window:
          min: 3
          max: 4
        heat: 2
"
                )
            )
        ),
        BASE,
    );
    write_asset(
        &root,
        "Assets/Prefabs/Boss.prefab",
        &format!("%YAML 1.1\n{}", holder_component(BOSS, "  phase: 2\n")),
        "f1f1f1f1f1f1f1f1f1f1f1f1f1f10003",
    );
    write_asset(
        &root,
        "Assets/Data/Sequence.asset",
        &format!(
            "%YAML 1.1\n--- !u!114 &11400000\nMonoBehaviour:\n  m_GameObject: {{fileID: 0}}\n  m_Script: {{fileID: 11500000, guid: {SEQUENCE}, type: 3}}\n  m_Name: Opening\n  tuning:\n    range:\n      min: 73\n      max: 74\n    steps: []\n"
        ),
        "f1f1f1f1f1f1f1f1f1f1f1f1f1f10004",
    );
    write_asset(
        &root,
        "Assets/Prefabs/Variant.prefab",
        &format!(
            "%YAML 1.1\n{}",
            prefab_instance(
                VARIANT_PI,
                "Variant",
                BASE,
                &[modification(BASE, HOLDER_ID, "tuning.range.max", "3")],
            )
        ),
        VARIANT,
    );
    let scene = [
        prefab_instance(
            1369411486,
            "FieldOverride",
            BASE,
            &[
                modification(BASE, HOLDER_ID, "tuning.steps.Array.data[0].max", "33"),
                modification(BASE, HOLDER_ID, "ranges.Array.data[0].max", "34"),
                modification(BASE, HOLDER_ID, "slot.value.min", "35"),
                modification(
                    BASE,
                    HOLDER_ID,
                    &format!("managedReferences[{RID_BURN}].window.min"),
                    "36",
                ),
                modification(BASE, HOLDER_ID, "tuning.range.max", "37"),
                modification(BASE, HOLDER_ID, "tuning.steps.Array.size", "2"),
                modification(BASE, HOLDER_ID, "phaseless", "1"),
            ],
        ),
        prefab_instance(
            999063860,
            "VariantInstance",
            VARIANT,
            &[modification(
                VARIANT,
                VARIANT_HOLDER,
                "tuning.range.min",
                "52",
            )],
        ),
    ]
    .concat();
    write_asset(
        &root,
        "Assets/Scenes/Arena.unity",
        &format!("%YAML 1.1\n{scene}"),
        "f1f1f1f1f1f1f1f1f1f1f1f1f1f10005",
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

fn count(output: &str, needle: &str) -> usize {
    output.lines().filter(|line| line.contains(needle)).count()
}

#[test]
fn struct_values_are_found_through_nesting_arrays_generics_subclasses_and_references() {
    let (_temp, db, root) = fixture();
    let output = usages(&db, &root, "Assets/Scripts/Data/Range.cs", None);

    assert!(
        output.contains("No asset references script 'Range' by GUID"),
        "{output}"
    );
    assert!(
        output.contains("17 [Serializable] values of Game.Data.Range"),
        "{output}"
    );
    for route in [
        "Holder.tuning.range",
        "Holder.tuning.steps[]",
        "Holder.ranges[]",
        "Holder.slot.value",
        "BossHolder.tuning.range",
        "Sequence.tuning.range",
        "Game.Fx.Burn.window",
    ] {
        assert!(
            output.contains(route),
            "missing route {route} in:\n{output}"
        );
    }

    // Base prefab: four field paths plus the reference instance's field.
    assert_eq!(
        count(&output, "[Serializable value] Holder."),
        4,
        "{output}"
    );
    assert!(
        output.contains("Holder.tuning.steps[0]: min: 1 on GameObject 'Holder'"),
        "{output}"
    );
    assert!(
        output.contains("Holder.slot.value: {…} on GameObject 'Holder'"),
        "{output}"
    );
    assert!(
        output.contains("[Serializable value] Game.Fx.Burn (rid 7542395272935243776).window"),
        "{output}"
    );
    // Derived script carries the base class fields.
    assert_eq!(
        count(&output, "[Serializable value] BossHolder."),
        3,
        "{output}"
    );
    assert!(
        output.contains("Sequence.tuning.range: {…} in 'Opening'"),
        "{output}"
    );

    // Overrides on the prefab component, through the variant's nested
    // fileID, and inside the reference instance.
    assert!(output.contains("[Serializable override] tuning.steps.Array.data[0].max: 33 — Holder prefab instance 'FieldOverride' of Assets/Prefabs/Base.prefab"), "{output}");
    assert!(
        output.contains(
            "[Serializable override] tuning.range.max: 3 — Holder prefab instance 'Variant'"
        ),
        "{output}"
    );
    assert!(output.contains("[Serializable override] tuning.range.min: 52 — Holder prefab instance 'VariantInstance' of Assets/Prefabs/Variant.prefab"), "{output}");
    assert!(output.contains("[Serializable override] managedReferences[7542395272935243776].window.min: 36 — Game.Fx.Burn"), "{output}");
    assert!(!output.contains("phaseless"), "{output}");
}

#[test]
fn member_query_lists_member_values_and_matching_overrides_with_former_names() {
    let (_temp, db, root) = fixture();
    let output = usages(&db, &root, "Assets/Scripts/Data/Range.cs", Some("max"));
    assert!(
        output.starts_with("13 serialized usages of 'Range.max'"),
        "{output}"
    );
    assert!(
        output.contains("[Serializable field] Holder.tuning.range.max: 1 on GameObject 'Holder'"),
        "{output}"
    );
    assert!(
        output
            .contains("[Serializable field] Game.Fx.Burn (rid 7542395272935243776).window.max: 4"),
        "{output}"
    );
    assert_eq!(count(&output, "[Serializable override]"), 4, "{output}");
    assert!(
        !output.contains("max: 52") && !output.contains("min: 35"),
        "{output}"
    );
    assert!(!output.contains("Array.size"), "{output}");
}

#[test]
fn outer_type_query_reports_its_own_routes() {
    let (_temp, db, root) = fixture();
    let output = usages(&db, &root, "Assets/Scripts/Data/Tuning.cs", None);
    assert!(output.contains("8 [Serializable] values of Game.Data.Tuning in 5 assets via 3 field paths (BossHolder.tuning, Holder.tuning, Sequence.tuning)"), "{output}");
}

#[test]
fn unused_and_non_serializable_types_explain_the_empty_result() {
    let (_temp, db, root) = fixture();
    let unused = usages(&db, &root, "Assets/Scripts/Data/Unused.cs", None);
    assert!(unused.starts_with("No serialized usages"), "{unused}");
    assert!(unused.contains("No serialized field of a MonoBehaviour / ScriptableObject / [SerializeReference] type holds Game.Data.Unused"), "{unused}");

    let plain = usages(&db, &root, "Assets/Scripts/Data/Plain.cs", None);
    assert!(
        plain.contains("Game.Data.Plain lack [Serializable]"),
        "{plain}"
    );
}

#[test]
fn yaml_walker_follows_fields_items_and_registry_data() {
    let doc = "--- !u!114 &1
MonoBehaviour:
  tuning:
    range:
      min: 0
      max: 1
    steps:
    - min: 1
      max: 2
    - min: 3
      max: 4
  ranges: []
  other:
    range:
      min: 9
";
    let lines: Vec<&str> = doc.lines().collect();
    let found = values_in_document(
        &lines,
        0,
        lines.len(),
        &[
            PathStep::Field("tuning".into()),
            PathStep::Field("steps".into()),
            PathStep::Element,
            PathStep::Field("max".into()),
        ],
    );
    assert_eq!(
        found,
        vec![
            (9, "tuning.steps[0].max".to_string(), "2".to_string()),
            (11, "tuning.steps[1].max".to_string(), "4".to_string()),
        ]
    );
    assert!(values_in_document(
        &lines,
        0,
        lines.len(),
        &[PathStep::Field("ranges".into()), PathStep::Element]
    )
    .is_empty());

    let registry = "    - rid: 7
      type: {class: Burn, ns: Game.Fx,
        asm: Assembly-CSharp}
      data:
        window:
          min: 3
          max: 4
    - rid: 8
      type: {class: Burn, ns: Game.Fx, asm: Assembly-CSharp}
      data:
        window:
          min: 5
";
    let lines: Vec<&str> = registry.lines().collect();
    let steps = [
        PathStep::Field("window".into()),
        PathStep::Field("min".into()),
    ];
    assert_eq!(
        values_in_registry_entry(&lines, 1, &steps),
        vec![(6, "window.min".to_string(), "3".to_string())]
    );
    assert_eq!(
        values_in_registry_entry(&lines, 8, &steps),
        vec![(12, "window.min".to_string(), "5".to_string())]
    );
}

#[test]
fn override_paths_match_routes_with_any_element_index() {
    let route = [
        PathStep::Field("tuning".into()),
        PathStep::Field("steps".into()),
        PathStep::Element,
    ];
    assert!(path_follows_route(
        &tokenize_property_path("tuning.steps.Array.data[3].max"),
        &route,
        true
    ));
    assert!(path_follows_route(
        &tokenize_property_path("tuning.steps.Array.size"),
        &route,
        true
    ));
    // A resize adds/removes values but touches no member.
    let mut member_route = route.to_vec();
    member_route.push(PathStep::Field("max".into()));
    assert!(!path_follows_route(
        &tokenize_property_path("tuning.steps.Array.size"),
        &member_route,
        false
    ));
    assert!(path_follows_route(
        &tokenize_property_path("tuning.steps.Array.data[0].max"),
        &member_route,
        false
    ));
    assert!(!path_follows_route(
        &tokenize_property_path("tuning.range.max"),
        &route,
        true
    ));
    assert!(!path_follows_route(
        &tokenize_property_path("tuning"),
        &route,
        true
    ));
}
