//! End-to-end coverage for `[SerializeReference]` support in
//! `unity_code_usages` and the serialized-type check behind
//! `code_diagnostics`, on a temp project indexed by a real AssetDb full scan.

use std::path::Path;
use std::sync::{Arc, Mutex};

use super::code_unity::run_code_usages;
use super::code_unity_managed::{
    check_managed_reference_types, format_managed_type_section, registry_data_fields,
    rid_backlinks, SharedAssetDb,
};
use crate::asset_db::AssetDb;

const HOST_GUID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaa0001";

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

/// Abstract base + interface + subclasses (one renamed via [MovedFrom], two
/// in a multi-class file whose name matches neither) inside the
/// `Game.Combat` asmdef, and an `Assembly-CSharp` ScriptableObject host whose
/// asset holds current, legacy, assembly-stranded, missing and external
/// managed references.
fn fixture_project() -> (tempfile::TempDir, SharedAssetDb) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(root.join("Assets")).unwrap();

    write_asset(
        &root,
        "Assets/Scripts/Game/Game.Combat.asmdef",
        "{ \"name\": \"Game.Combat\" }",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbb0001",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/TimedEffect.cs",
        r#"using System;
using UnityEngine.Serialization;
namespace Game.Combat
{
    [Serializable]
    public abstract class TimedEffect
    {
        public int startTime;
        [FormerlySerializedAs("endAt")] public int endTime;
    }
}
"#,
        "cccccccccccccccccccccccccccc0001",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/IReaction.cs",
        "namespace Game.Combat { public interface IReaction { } }\n",
        "cccccccccccccccccccccccccccc0002",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/TurnEffect.cs",
        "using System;\nnamespace Game.Combat\n{\n    [Serializable]\n    public sealed class TurnEffect : TimedEffect, IReaction { public int turnMode; }\n}\n",
        "cccccccccccccccccccccccccccc0003",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/Legacy/PulseEffect.cs",
        "using UnityEngine.Scripting.APIUpdating;\nnamespace Game.Combat\n{\n    [MovedFrom(false, \"Old.Events\", null, \"OldPulseEffect\")]\n    public sealed class PulseEffect : TimedEffect { }\n}\n",
        "cccccccccccccccccccccccccccc0004",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/Hits.cs",
        "namespace Game.Combat\n{\n    public class HitEvent : TimedEffect { }\n    public sealed class KnockEvent : HitEvent { }\n}\n",
        "cccccccccccccccccccccccccccc0005",
    );
    write_asset(
        &root,
        "Assets/Scripts/Game/Unused.cs",
        "namespace Game.Combat { public sealed class Unused { } }\n",
        "cccccccccccccccccccccccccccc0006",
    );
    write_asset(
        &root,
        "Assets/Scripts/Host/EffectSequence.cs",
        "using System.Collections.Generic;\nusing UnityEngine;\nusing Game.Combat;\npublic class EffectSequence : ScriptableObject\n{\n    [SerializeReference] public List<TimedEffect> events;\n}\n",
        HOST_GUID,
    );
    write_asset(
        &root,
        "Assets/Actions/Combo.asset",
        &format!(
            r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!114 &11400000
MonoBehaviour:
  m_ObjectHideFlags: 0
  m_GameObject: {{fileID: 0}}
  m_Enabled: 1
  m_Script: {{fileID: 11500000, guid: {HOST_GUID}, type: 3}}
  m_Name: Combo
  events:
  - rid: 1
  - rid: 2
  - rid: 3
  - rid: 4
  - rid: 5
  - rid: 6
  references:
    version: 2
    RefIds:
    - rid: 1
      type: {{class: TurnEffect, ns: Game.Combat, asm: Game.Combat}}
      data:
        startTime: 3
        endAt: 9
        turnMode: 1
    - rid: 2
      type: {{class: OldPulseEffect, ns: Old.Events, asm: Game.Combat}}
      data:
        startTime: 0
    - rid: 3
      type: {{class: KnockEvent, ns: Game.Combat, asm: Game.Combat}}
      data:
        startTime: 5
        endTime: 7
    - rid: 4
      type: {{class: TurnEffect, ns: Game.Combat, asm: Assembly-CSharp}}
      data:
        turnMode: 2
    - rid: 5
      type: {{class: GhostEvent, ns: Game.Combat, asm: Game.Combat}}
      data: {{}}
    - rid: 6
      type: {{class: Cube, ns: UnityEngine.ProBuilder.Shapes, asm: Unity.ProBuilder}}
      data:
        size: 1
"#
        ),
        "dddddddddddddddddddddddddddd0001",
    );

    let mut db = AssetDb::open(&root).expect("open asset db");
    db.full_scan(|_| {}).expect("scan");
    (temp, Arc::new(Mutex::new(Some(db))))
}

fn project_root(temp: &tempfile::TempDir) -> String {
    temp.path().join("project").to_string_lossy().to_string()
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

fn line_with<'a>(output: &'a str, needle: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line containing {needle:?} in:\n{output}"))
}

#[test]
fn base_class_query_lists_subclass_legacy_transitive_and_stranded_instances() {
    let (temp, db) = fixture_project();
    let root = project_root(&temp);
    let output = usages(&db, &root, "Assets/Scripts/Game/TimedEffect.cs", None);

    assert!(output.contains("No asset references script 'TimedEffect' by GUID"));
    assert!(output.contains("4 [SerializeReference] instances of 4 serialized types in 1 asset"));
    assert!(output.contains("in assembly 'Game.Combat'"));

    let turn = line_with(&output, "Game.Combat.TurnEffect (rid 1)");
    assert!(turn.contains(" in EffectSequence 'Combo'"), "{turn}");
    assert!(turn.contains("via events[0]"), "{turn}");
    assert!(
        turn.contains("subclass/implementer of Game.Combat.TimedEffect"),
        "{turn}"
    );
    assert!(!turn.contains("BROKEN"), "{turn}");

    let legacy = line_with(&output, "Old.Events.OldPulseEffect (rid 2)");
    assert!(
        legacy.contains("legacy name, resolves via [MovedFrom] on Game.Combat.PulseEffect"),
        "{legacy}"
    );

    // Declared in a multi-class file named after neither type, two levels down.
    let knock = line_with(&output, "Game.Combat.KnockEvent (rid 3)");
    assert!(knock.contains("via events[2]"), "{knock}");

    let stranded = line_with(&output, "Game.Combat.TurnEffect (rid 4)");
    assert!(
        stranded.contains("BROKEN: serialized asm 'Assembly-CSharp' but Game.Combat.TurnEffect compiles into 'Game.Combat'"),
        "{stranded}"
    );

    assert!(!output.contains("GhostEvent"));
    assert!(!output.contains("ProBuilder"));
}

#[test]
fn concrete_and_interface_queries_match_their_own_instances() {
    let (temp, db) = fixture_project();
    let root = project_root(&temp);

    let turn = usages(&db, &root, "Assets/Scripts/Game/TurnEffect.cs", None);
    assert!(turn.contains("2 [SerializeReference] instances"));
    assert!(!line_with(&turn, "(rid 1)").contains("subclass"));
    assert!(line_with(&turn, "(rid 4)").contains("BROKEN"));
    assert!(!turn.contains("(rid 3)"));

    let reaction = usages(&db, &root, "Assets/Scripts/Game/IReaction.cs", None);
    assert!(reaction.contains("2 [SerializeReference] instances"));
    assert!(
        line_with(&reaction, "(rid 1)").contains("subclass/implementer of Game.Combat.IReaction")
    );
    assert!(!reaction.contains("(rid 3)"));

    let renamed = usages(
        &db,
        &root,
        "Assets/Scripts/Game/Legacy/PulseEffect.cs",
        None,
    );
    assert!(line_with(&renamed, "(rid 2)").contains("legacy name"));
}

#[test]
fn member_query_reads_registry_data_including_former_names() {
    let (temp, db) = fixture_project();
    let root = project_root(&temp);
    let output = usages(
        &db,
        &root,
        "Assets/Scripts/Game/TimedEffect.cs",
        Some("endTime"),
    );

    let former = line_with(&output, "endAt: 9");
    assert!(former.contains("[SerializeReference field]"), "{former}");
    assert!(
        former.contains("Game.Combat.TurnEffect (rid 1)"),
        "{former}"
    );
    assert!(
        former.contains("[FormerlySerializedAs \"endAt\"]"),
        "{former}"
    );
    let current = line_with(&output, "endTime: 7");
    assert!(
        current.contains("Game.Combat.KnockEvent (rid 3)"),
        "{current}"
    );
    assert!(output.starts_with("2 serialized usages of 'TimedEffect.endTime'"));
}

#[test]
fn unused_type_reports_coverage_instead_of_a_bare_negative() {
    let (temp, db) = fixture_project();
    let root = project_root(&temp);
    let output = usages(&db, &root, "Assets/Scripts/Game/Unused.cs", None);
    assert!(output.starts_with("No serialized usages of 'Assets/Scripts/Game/Unused.cs'"));
    assert!(output.contains("1 declared type (Game.Combat.Unused) in assembly 'Game.Combat'"));
    assert!(
        output.contains("Game.Combat.Unused lack [Serializable]"),
        "{output}"
    );
    assert!(
        output.contains("Not covered: binary-serialized assets"),
        "{output}"
    );

    let member = usages(&db, &root, "Assets/Scripts/Game/Unused.cs", Some("nothing"));
    assert!(
        member.contains("Not covered: binary-serialized assets"),
        "{member}"
    );
}

#[test]
fn serialized_type_check_reports_missing_and_stranded_types() {
    let (temp, db) = fixture_project();
    let root = temp.path().join("project");

    let report = check_managed_reference_types(&db, &root, None)
        .unwrap()
        .expect("asset db present");
    assert_eq!(report.checked_types, 5);
    assert_eq!(report.skipped_external_types, 1);
    let mut issues: Vec<String> = report
        .issues
        .iter()
        .map(|issue| {
            format!(
                "{}@{}",
                issue.identity.display_name(),
                issue.identity.assembly
            )
        })
        .collect();
    issues.sort();
    assert_eq!(
        issues,
        vec![
            "Game.Combat.GhostEvent@Game.Combat".to_string(),
            "Game.Combat.TurnEffect@Assembly-CSharp".to_string(),
        ]
    );
    let ghost = report
        .issues
        .iter()
        .find(|issue| issue.identity.class_name == "GhostEvent")
        .unwrap();
    assert!(ghost.moved_to.is_none());
    assert_eq!(
        ghost.samples,
        vec![("Assets/Actions/Combo.asset".to_string(), 39)]
    );
    let stranded = report
        .issues
        .iter()
        .find(|issue| issue.identity.assembly == "Assembly-CSharp")
        .unwrap();
    assert_eq!(
        stranded.moved_to,
        Some((
            "Game.Combat.TurnEffect".to_string(),
            "Game.Combat".to_string()
        ))
    );

    let text = format_managed_type_section(&report, 10);
    assert!(text.contains("SerializeReference types: 2 unresolved of 5 checked"));
    assert!(text.contains("no such type in code"));
    assert!(text.contains("now compiles into 'Game.Combat', not 'Assembly-CSharp'"));
    assert!(text.contains("Assets/Actions/Combo.asset:39"));

    // File scope: the host's assembly (Assembly-CSharp) only sees the stranded
    // TurnEffect; TurnEffect.cs sees its own assembly plus identities naming it.
    let host =
        check_managed_reference_types(&db, &root, Some("Assets/Scripts/Host/EffectSequence.cs"))
            .unwrap()
            .unwrap();
    assert_eq!(host.issues.len(), 1);
    assert_eq!(host.issues[0].identity.assembly, "Assembly-CSharp");

    let turn = check_managed_reference_types(&db, &root, Some("assets/scripts/game/turneffect.cs"))
        .unwrap()
        .unwrap();
    assert_eq!(turn.checked_types, 5);
    assert_eq!(turn.issues.len(), 2);
}

#[test]
fn exact_declaration_in_multi_class_file_beats_same_name_in_another_assembly() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    write_asset(
        &root,
        "Assets/A/AsmA.asmdef",
        "{ \"name\": \"AsmA\" }",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeee0001",
    );
    // Primary (indexed) type of Multi.cs is `Bar`, so only the declaration
    // scan finds `Foo` here.
    write_asset(
        &root,
        "Assets/A/Multi.cs",
        "namespace Game { public class Bar { } public class Foo { } }\n",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeee0002",
    );
    write_asset(
        &root,
        "Assets/B/AsmB.asmdef",
        "{ \"name\": \"AsmB\" }",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeee0003",
    );
    write_asset(
        &root,
        "Assets/B/Foo.cs",
        "namespace Game { public class Foo { } }\n",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeee0004",
    );
    write_asset(
        &root,
        "Assets/Data/Holder.asset",
        "%YAML 1.1\n--- !u!114 &11400000\nMonoBehaviour:\n  m_Name: Holder\n  item:\n    rid: 1\n  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: Foo, ns: Game, asm: AsmA}\n      data:\n        x: 1\n",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeee0005",
    );
    let mut db = AssetDb::open(&root).unwrap();
    db.full_scan(|_| {}).unwrap();
    let db: SharedAssetDb = Arc::new(Mutex::new(Some(db)));

    let report = check_managed_reference_types(&db, &root, None)
        .unwrap()
        .unwrap();
    assert_eq!(report.checked_types, 1);
    assert!(report.issues.is_empty(), "{:?}", report.issues);

    // The AsmB `Foo.cs` is a different type: no instances belong to it.
    let root_text = root.to_string_lossy().to_string();
    let b = usages(&db, &root_text, "Assets/B/Foo.cs", None);
    assert!(b.starts_with("No serialized usages"), "{b}");
    let a = usages(&db, &root_text, "Assets/A/Multi.cs", None);
    let line = line_with(&a, "Game.Foo (rid 1)");
    assert!(line.contains("via item"), "{line}");
    assert!(!line.contains("BROKEN"), "{line}");
}

#[test]
fn serialized_type_check_is_quiet_without_index_and_when_all_resolve() {
    let empty: SharedAssetDb = Arc::new(Mutex::new(None));
    assert!(check_managed_reference_types(&empty, Path::new("."), None)
        .unwrap()
        .is_none());

    let report = super::code_unity_managed::ManagedTypeReport {
        checked_types: 3,
        checked_instances: 7,
        ..Default::default()
    };
    assert_eq!(
        format_managed_type_section(&report, 10),
        "\n\nSerializeReference types: all 3 serialized project types resolve (7 instances)."
    );
}

#[test]
fn backlinks_cover_list_block_flow_and_nested_registry_links() {
    let doc = "--- !u!114 &11400000
MonoBehaviour:
  m_Script: {fileID: 11500000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaa0001, type: 3}
  events:
  - rid: 10
  - rid: 11
  root:
    rid: 12
  single: {rid: 13}
  groups:
  - name: a
    effect: {rid: 14}
  references:
    version: 2
    RefIds:
    - rid: 10
      type: {class: A, ns: N, asm: X}
      data:
        child: {rid: 15}
        children:
        - rid: 16
";
    let lines: Vec<&str> = doc.lines().collect();
    let links = rid_backlinks(&lines);
    let path = |rid: i64| links.get(&rid).map(|p| p.join(" | ")).unwrap_or_default();
    assert_eq!(path(10), "events[0]");
    assert_eq!(path(11), "events[1]");
    assert_eq!(path(12), "root");
    assert_eq!(path(13), "single");
    assert_eq!(path(14), "groups[0].effect");
    assert_eq!(path(15), "managedReferences[10].child");
    assert_eq!(path(16), "managedReferences[10].children[0]");
}

#[test]
fn registry_data_fields_stay_inside_one_entry() {
    let text = "  references:
    version: 2
    RefIds:
    - rid: 1
      type: {class: Long, ns: Very.Long.Namespace,
        asm: Asm}
      data:
        amount: 1
        nested:
          amount: 99
        curve:
          serializedVersion: 2
    - rid: 2
      type: {class: Other, ns: N, asm: Asm}
      data:
        amount: 2
";
    let lines: Vec<&str> = text.lines().collect();
    let names = vec!["amount".to_string(), "curve".to_string()];
    assert_eq!(
        registry_data_fields(&lines, 4, &names),
        vec![
            (8, "amount".to_string(), "1".to_string()),
            (11, "curve".to_string(), "{…}".to_string()),
        ]
    );
    assert_eq!(
        registry_data_fields(&lines, 13, &names),
        vec![(16, "amount".to_string(), "2".to_string())]
    );
}
