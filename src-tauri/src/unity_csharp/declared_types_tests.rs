//! Coverage for `parse_cs_declared_types` and the asmdef/asmref assembly
//! resolver.

use std::collections::HashMap;
use std::path::Path;

use super::assembly::{predefined_assembly, AssemblyResolver, AssemblySource};
use super::{parse_cs_declared_types, parse_type_ref, CsTypeRef, DeclaredTypeKind};

fn names(content: &str) -> Vec<(String, String)> {
    parse_cs_declared_types(content)
        .types
        .iter()
        .map(|t| (t.namespace.clone(), t.serialized_class_path()))
        .collect()
}

#[test]
fn enumerates_all_types_including_nested_and_generic() {
    let source = r#"
using System;
using UnityEngine;
using Alias = Game.Other;
using static System.Math;

namespace Game.Combat
{
    [Serializable]
    public abstract class TimedEffect : IActEvent
    {
        public int startTime;
        [SerializeField] private int endTime;

        [Serializable]
        public sealed class Nested : TimedEffect { public float weight; }
    }

    public interface IActEvent { }

    [Serializable]
    public class Box<T, U> : TimedEffect where T : class { }

    struct Pair { public int a; }
}

namespace Game
{
    namespace Deep
    {
        class Inner { }
    }
}

class GlobalType { }
"#;
    let parsed = parse_cs_declared_types(source);
    assert_eq!(
        names(source),
        vec![
            ("Game.Combat".to_string(), "TimedEffect".to_string()),
            ("Game.Combat".to_string(), "TimedEffect/Nested".to_string()),
            ("Game.Combat".to_string(), "IActEvent".to_string()),
            ("Game.Combat".to_string(), "Box`2".to_string()),
            ("Game.Combat".to_string(), "Pair".to_string()),
            ("Game.Deep".to_string(), "Inner".to_string()),
            (String::new(), "GlobalType".to_string()),
        ]
    );
    assert_eq!(
        parsed.usings,
        vec!["System".to_string(), "UnityEngine".to_string()]
    );

    let base = &parsed.types[0];
    assert!(base.is_abstract);
    assert_eq!(base.kind, DeclaredTypeKind::Class);
    assert_eq!(base.line, 10);
    assert_eq!(
        base.bases,
        vec![CsTypeRef {
            segments: vec!["IActEvent".into()],
            arity: 0
        }]
    );
    let fields: Vec<&str> = base
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, vec!["startTime", "endTime"]);

    let nested = &parsed.types[1];
    assert_eq!(nested.nested_path, vec!["TimedEffect", "Nested"]);
    assert_eq!(nested.display_name(), "Game.Combat.TimedEffect.Nested");
    assert_eq!(nested.serialized_fields.len(), 1);

    assert_eq!(parsed.types[2].kind, DeclaredTypeKind::Interface);
    assert_eq!(parsed.types[3].arity, 2);
    assert_eq!(parsed.types[4].kind, DeclaredTypeKind::Struct);
}

#[test]
fn file_scoped_namespace_and_preprocessor_blocks() {
    let source = r#"
namespace Game.Fx;

#if UNITY_EDITOR
public class EditorOnlyEvent : Base.Event { }
#endif

public class Burn : global::Game.Fx.Effect<int> { }
"#;
    let parsed = parse_cs_declared_types(source);
    let found: Vec<(String, String)> = parsed
        .types
        .iter()
        .map(|t| (t.namespace.clone(), t.name().to_string()))
        .collect();
    assert_eq!(
        found,
        vec![
            ("Game.Fx".to_string(), "EditorOnlyEvent".to_string()),
            ("Game.Fx".to_string(), "Burn".to_string()),
        ]
    );
    assert_eq!(
        parsed.types[0].bases[0].segments,
        vec!["Base".to_string(), "Event".to_string()]
    );
    assert_eq!(
        parsed.types[1].bases[0],
        CsTypeRef {
            segments: vec!["Game".into(), "Fx".into(), "Effect".into()],
            arity: 1
        }
    );
}

#[test]
fn reads_moved_from_positional_named_and_namespace_only_forms() {
    let source = r#"
using UnityEngine.Scripting.APIUpdating;
namespace Game
{
    [MovedFrom(false, "Legacy.Events", "Legacy.Asm", "OldPulseEffect")]
    public sealed class PulseEffect { }

    [MovedFrom(true, sourceClassName: "OldName")]
    public sealed class Renamed { }

    [UnityEngine.Scripting.APIUpdating.MovedFromAttribute("Old.Namespace")]
    public sealed class Moved { }

    [MovedFrom(false, null, "Old.Asm")]
    public sealed class AsmOnly { }

    public sealed class Plain { }
}
"#;
    let parsed = parse_cs_declared_types(source);
    let moved = |name: &str| {
        parsed
            .types
            .iter()
            .find(|t| t.name() == name)
            .unwrap()
            .moved_from
            .clone()
    };

    let rate = moved("PulseEffect").expect("PulseEffect moved");
    assert_eq!(rate.source_namespace.as_deref(), Some("Legacy.Events"));
    assert_eq!(rate.source_assembly.as_deref(), Some("Legacy.Asm"));
    assert_eq!(rate.source_class_name.as_deref(), Some("OldPulseEffect"));

    let renamed = moved("Renamed").expect("Renamed moved");
    assert_eq!(renamed.source_namespace, None);
    assert_eq!(renamed.source_class_name.as_deref(), Some("OldName"));

    let ns_only = moved("Moved").expect("Moved moved");
    assert_eq!(ns_only.source_namespace.as_deref(), Some("Old.Namespace"));
    assert_eq!(ns_only.source_class_name, None);

    let asm_only = moved("AsmOnly").expect("AsmOnly moved");
    assert_eq!(asm_only.source_namespace, None);
    assert_eq!(asm_only.source_assembly.as_deref(), Some("Old.Asm"));

    assert!(moved("Plain").is_none());
}

#[test]
fn keeps_former_names_on_serialized_fields() {
    let source = r#"
[System.Serializable]
public class Knockback
{
    [UnityEngine.Serialization.FormerlySerializedAs("force")]
    public float strength;
    [field: UnityEngine.SerializeField] public int Hits { get; private set; }
    public static int ignored;
    [System.NonSerialized] public int alsoIgnored;
}
"#;
    let parsed = parse_cs_declared_types(source);
    let fields = &parsed.types[0].serialized_fields;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "strength");
    assert_eq!(fields[0].former_names, vec!["force".to_string()]);
    assert_eq!(fields[1].name, "Hits");
}

#[test]
fn parses_type_reference_text() {
    assert_eq!(
        parse_type_ref("global::Game.Outer<int>.Base<T, Dictionary<K, V>>"),
        Some(CsTypeRef {
            segments: vec!["Game".into(), "Outer".into(), "Base".into()],
            arity: 2
        })
    );
    assert_eq!(
        parse_type_ref("Foo?"),
        Some(CsTypeRef {
            segments: vec!["Foo".into()],
            arity: 0
        })
    );
    assert_eq!(parse_type_ref(""), None);
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn resolves_assemblies_from_asmdef_asmref_and_predefined_rules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Assets/Scripts/Game/Game.asmdef",
        "\u{feff}{ \"name\": \"Game.Runtime\", \"references\": [] }",
    );
    write(root, "Assets/Scripts/Game/Sub/Deep/Hit.cs", "");
    write(
        root,
        "Assets/Scripts/Ext/Ext.asmref",
        "{ \"reference\": \"GUID:ABCDEFABCDEFABCDEFABCDEFABCDEF01\" }",
    );
    write(root, "Assets/Scripts/Ext/Extra.cs", "");
    write(
        root,
        "Assets/Scripts/ByName/ByName.asmref",
        "{ \"reference\": \"Other.Named\" }",
    );
    write(root, "Assets/Scripts/ByName/Named.cs", "");
    write(
        root,
        "Packages/com.studio.tools/Runtime/Tools.asmdef",
        "{ \"name\": \"Studio.Tools\" }",
    );
    write(root, "Packages/com.studio.tools/Runtime/Tool.cs", "");
    write(root, "Packages/com.studio.loose/Loose.cs", "");

    let guids: HashMap<String, String> = HashMap::from([(
        "abcdefabcdefabcdefabcdefabcdef01".to_string(),
        "Assets/Scripts/Game/Game.asmdef".to_string(),
    )]);
    let mut resolver = AssemblyResolver::new(root, |guid| guids.get(guid).cloned());

    let hit = resolver
        .assembly_for("Assets/Scripts/Game/Sub/Deep/Hit.cs")
        .unwrap();
    assert_eq!(hit.name, "Game.Runtime");
    assert_eq!(
        hit.source,
        AssemblySource::Asmdef("Assets/Scripts/Game/Game.asmdef".to_string())
    );

    let ext = resolver
        .assembly_for("Assets/Scripts/Ext/Extra.cs")
        .unwrap();
    assert_eq!(ext.name, "Game.Runtime");
    assert_eq!(
        ext.source,
        AssemblySource::Asmref("Assets/Scripts/Ext/Ext.asmref".to_string())
    );

    assert_eq!(
        resolver
            .assembly_for("Assets/Scripts/ByName/Named.cs")
            .unwrap()
            .name,
        "Other.Named"
    );
    assert_eq!(
        resolver
            .assembly_for("Packages/com.studio.tools/Runtime/Tool.cs")
            .unwrap()
            .name,
        "Studio.Tools"
    );
    assert_eq!(
        resolver.assembly_for("Packages/com.studio.loose/Loose.cs"),
        None
    );
    assert_eq!(resolver.assembly_for("Library/PackageCache/x/Y.cs"), None);

    let predefined = |path: &str| {
        let mut resolver = AssemblyResolver::new(root, |_| None);
        let found = resolver.assembly_for(path).unwrap();
        assert_eq!(found.source, AssemblySource::Predefined);
        found.name
    };
    assert_eq!(predefined("Assets/Foo.cs"), "Assembly-CSharp");
    assert_eq!(
        predefined("Assets/Tools/Editor/T.cs"),
        "Assembly-CSharp-Editor"
    );
    assert_eq!(
        predefined("Assets/Plugins/P.cs"),
        "Assembly-CSharp-firstpass"
    );
    assert_eq!(
        predefined("Assets/Standard Assets/Editor/E.cs"),
        "Assembly-CSharp-Editor-firstpass"
    );
    // `Plugins` only counts as a special folder at the top of Assets.
    assert_eq!(predefined("Assets/Game/Plugins/P.cs"), "Assembly-CSharp");
}

#[test]
fn predefined_rule_table() {
    assert_eq!(predefined_assembly(&[]), "Assembly-CSharp");
    assert_eq!(
        predefined_assembly(&["plugins", "editor"]),
        "Assembly-CSharp-Editor-firstpass"
    );
    assert_eq!(
        predefined_assembly(&["Pro Standard Assets"]),
        "Assembly-CSharp-firstpass"
    );
}

#[test]
fn records_type_parameters_and_serializable_attribute() {
    let source = r#"
namespace Game
{
    [System.Serializable] public class Slot<in TKey, out TValue> { public TValue value; }
    [SerializableAttribute] public struct Pair { public int a; }
    public class NotMarked { public int a; }
}
"#;
    let parsed = parse_cs_declared_types(source);
    let slot = &parsed.types[0];
    assert_eq!(
        slot.type_params,
        vec!["TKey".to_string(), "TValue".to_string()]
    );
    assert_eq!(slot.arity, 2);
    assert!(slot.is_serializable);
    assert!(parsed.types[1].is_serializable);
    assert!(!parsed.types[2].is_serializable);
    assert!(parsed.types[2].type_params.is_empty());
}

#[test]
fn parses_field_type_shapes() {
    use super::{parse_field_type, FieldTypeShape};
    let simple = |segments: &[&str]| FieldTypeShape {
        name: CsTypeRef {
            segments: segments.iter().map(|s| s.to_string()).collect(),
            arity: 0,
        },
        args: Vec::new(),
        array_rank: 0,
    };

    assert_eq!(parse_field_type("Range"), Some(simple(&["Range"])));
    let array = parse_field_type("Game.Data.Range[]").unwrap();
    assert_eq!(array.array_rank, 1);
    assert_eq!(array.element(), Some(simple(&["Game", "Data", "Range"])));

    let list = parse_field_type("List< Range >").unwrap();
    assert!(list.is_list());
    assert_eq!(list.args, vec![simple(&["Range"])]);
    assert!(parse_field_type("System.Collections.Generic.List<int>")
        .unwrap()
        .is_list());
    assert!(!parse_field_type("Game.List<int>").unwrap().is_list());

    let nested = parse_field_type("global::Game.Slot<Pair<int, Range>>[][]").unwrap();
    assert_eq!(nested.array_rank, 2);
    assert_eq!(
        nested.name.segments,
        vec!["Game".to_string(), "Slot".to_string()]
    );
    assert_eq!(nested.name.arity, 1);
    assert_eq!(nested.args[0].name.arity, 2);
    assert_eq!(nested.args[0].args[1], simple(&["Range"]));

    assert_eq!(parse_field_type("int?").unwrap().array_rank, 0);
    assert_eq!(parse_field_type("float[,]").unwrap().array_rank, 1);
    assert_eq!(parse_field_type("(int, int)"), None);
    assert_eq!(parse_field_type("List<int"), None);
}
