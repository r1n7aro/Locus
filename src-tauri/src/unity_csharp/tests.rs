use super::parser::parse_cs_script;

#[test]
fn parses_scriptable_object_with_attributes() {
    let content = r#"
using UnityEngine;
using UnityEngine.Serialization;
using System.Collections.Generic;

namespace MyGame.Data
{
    public class EnemyConfig : ScriptableObject
    {
        public string enemyName;
        public int health;
        [HideInInspector]
        public float _internalValue;
        [SerializeField]
        private List<int> _actions;
        [FormerlySerializedAs("oldSpeed")]
        public float moveSpeed;
    }
}
"#;
    let meta = parse_cs_script(content, Some("EnemyConfig")).unwrap();
    assert_eq!(meta.class_name, "EnemyConfig");
    assert_eq!(meta.base_type.as_deref(), Some("ScriptableObject"));
    assert_eq!(meta.namespace.as_deref(), Some("MyGame.Data"));

    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&"enemyName"));
    assert!(names.contains(&"health"));
    assert!(names.contains(&"_internalValue"));
    assert!(names.contains(&"_actions"));
    assert!(names.contains(&"moveSpeed"));

    let internal_val = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "_internalValue")
        .unwrap();
    assert!(internal_val.hidden);

    let speed = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "moveSpeed")
        .unwrap();
    assert_eq!(speed.former_names, vec!["oldSpeed"]);
}

#[test]
fn parses_monobehaviour_excluding_methods_and_properties() {
    let content = r#"
public class PlayerController : MonoBehaviour
{
    public float speed = 5f;
    public int Score { get; set; }   // auto-property: NOT serialized by Unity
    public void Update() { }
    public PlayerController() { }
}
"#;
    let meta = parse_cs_script(content, Some("PlayerController")).unwrap();
    assert_eq!(meta.class_name, "PlayerController");
    assert_eq!(meta.base_type.as_deref(), Some("MonoBehaviour"));
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["speed"]);
}

#[test]
fn parses_field_targeted_serialize_field_auto_property() {
    let content = r#"
using UnityEngine;
public class Hero : MonoBehaviour
{
    [field: SerializeField] public int Hp { get; private set; }
    [field: SerializeField, HideInInspector] public string Name { get; set; }
    public int NotSerialized { get; set; }
}
"#;
    let meta = parse_cs_script(content, Some("Hero")).unwrap();
    let hp = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "Hp")
        .expect("[field: SerializeField] auto-property should be picked up");
    assert_eq!(hp.field_type, "int");

    let name = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "Name")
        .unwrap();
    assert!(name.hidden);

    assert!(meta
        .serialized_fields
        .iter()
        .all(|f| f.name != "NotSerialized"));
}

#[test]
fn excludes_static_const_and_non_serialized_fields() {
    let content = r#"
using System;
using UnityEngine;
public class Conf : ScriptableObject
{
    public const int MaxHp = 100;
    public static int Counter;
    [NonSerialized] public int runtimeOnly;
    public int normalField;
}
"#;
    let meta = parse_cs_script(content, Some("Conf")).unwrap();
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["normalField"]);
}

#[test]
fn picks_type_matching_file_stem_over_first_in_source() {
    let content = r#"
namespace Demo
{
    internal class Helper { public int aux; }

    public class MainConfig : ScriptableObject
    {
        public int value;
    }
}
"#;
    // File-stem hint must override "first in source order".
    let meta = parse_cs_script(content, Some("MainConfig")).unwrap();
    assert_eq!(meta.class_name, "MainConfig");
    assert_eq!(meta.base_type.as_deref(), Some("ScriptableObject"));
    assert_eq!(meta.namespace.as_deref(), Some("Demo"));
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["value"]);
}

#[test]
fn falls_back_to_first_public_when_no_filename_hint_matches() {
    let content = r#"
namespace Demo
{
    internal class Helper { public int aux; }

    public class MainConfig : ScriptableObject
    {
        public int value;
    }
}
"#;
    // File stem doesn't match either type → first public type wins.
    let meta = parse_cs_script(content, Some("Unrelated")).unwrap();
    assert_eq!(meta.class_name, "MainConfig");
}

#[test]
fn handles_file_scoped_namespace() {
    let content = r#"
namespace Game.Combat;

public class FireConfig : CombatConfig
{
    public float damage;
}
"#;
    let meta = parse_cs_script(content, Some("FireConfig")).unwrap();
    assert_eq!(meta.class_name, "FireConfig");
    assert_eq!(meta.base_type.as_deref(), Some("CombatConfig"));
    assert_eq!(meta.namespace.as_deref(), Some("Game.Combat"));
}

#[test]
fn handles_multiple_declarators_in_one_field() {
    let content = r#"
public class Foo : MonoBehaviour
{
    public int a, b, c;
}
"#;
    let meta = parse_cs_script(content, Some("Foo")).unwrap();
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["a", "b", "c"]);
}

#[test]
fn handles_generic_base_with_where_clause() {
    let content = r#"
public class Bag<T> : ItemContainer<T> where T : Item
{
    public int capacity;
}
"#;
    let meta = parse_cs_script(content, Some("Bag")).unwrap();
    assert_eq!(meta.class_name, "Bag");
    assert_eq!(meta.base_type.as_deref(), Some("ItemContainer"));
}

#[test]
fn skips_apparent_interface_in_base_list() {
    let content = r#"
public class Hero : IDamageable, MonoBehaviour
{
    public int hp;
}
"#;
    let meta = parse_cs_script(content, Some("Hero")).unwrap();
    // IDamageable should be skipped (looks like an interface), MonoBehaviour wins.
    assert_eq!(meta.base_type.as_deref(), Some("MonoBehaviour"));
}

#[test]
fn merges_partial_class_field_fragments() {
    let content = r#"
public partial class Boss : MonoBehaviour
{
    public int hp;
}

public partial class Boss
{
    public float speed;
    [SerializeField] private string codename;
}
"#;
    let meta = parse_cs_script(content, Some("Boss")).unwrap();
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(names.contains(&"hp"));
    assert!(names.contains(&"speed"));
    assert!(names.contains(&"codename"));
}

#[test]
fn finds_class_inside_preprocessor_conditional() {
    let content = r#"
#if UNITY_EDITOR
using UnityEngine;
namespace Tools
{
    public class EditorWidget : ScriptableObject
    {
        public int value;
    }
}
#endif
"#;
    let meta = parse_cs_script(content, Some("EditorWidget")).unwrap();
    assert_eq!(meta.class_name, "EditorWidget");
    assert_eq!(meta.base_type.as_deref(), Some("ScriptableObject"));
    assert_eq!(meta.namespace.as_deref(), Some("Tools"));
}

#[test]
fn returns_none_when_only_commented_or_empty() {
    let content = r#"
// public class Disabled : MonoBehaviour { public int hp; }

/*
public class AlsoDisabled : MonoBehaviour { public int hp; }
*/
"#;
    assert!(parse_cs_script(content, Some("anything")).is_none());
}

#[test]
fn property_records_former_names_via_field_target_attribute() {
    let content = r#"
using UnityEngine;
using UnityEngine.Serialization;
public class Hero : MonoBehaviour
{
    [field: SerializeField, FormerlySerializedAs("oldHp")]
    public int Hp { get; private set; }
}
"#;
    let meta = parse_cs_script(content, Some("Hero")).unwrap();
    let hp = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "Hp")
        .expect("auto-property should be picked up under [field: SerializeField]");
    assert_eq!(hp.former_names, vec!["oldHp"]);
    assert_eq!(hp.field_type, "int");
}

#[test]
fn verbatim_former_name_collapses_doubled_quotes() {
    // C# verbatim strings escape `"` as `""`. Decoding `@"a""b"` must yield
    // `a"b`, not `a""b`. Narrow case in practice but used to be a silent
    // corruption bug.
    let content = r#"
using UnityEngine;
using UnityEngine.Serialization;
public class Quoted : MonoBehaviour
{
    [FormerlySerializedAs(@"a""b")]
    public int counter;
}
"#;
    let meta = parse_cs_script(content, Some("Quoted")).unwrap();
    let counter = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "counter")
        .unwrap();
    assert_eq!(counter.former_names, vec!["a\"b"]);
}

#[test]
fn positional_record_does_not_shadow_real_monobehaviour() {
    // Positional records have no body and aren't Unity-serializable.
    // The parser must not pick the record as the primary type and must
    // surface the real MonoBehaviour even when the record appears first.
    let content = r#"
public record Coords(int X, int Y);

public class World : MonoBehaviour
{
    public int seed;
}
"#;
    let meta = parse_cs_script(content, Some("World")).unwrap();
    assert_eq!(meta.class_name, "World");
    assert_eq!(meta.base_type.as_deref(), Some("MonoBehaviour"));
    let names: Vec<&str> = meta
        .serialized_fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["seed"]);
}

#[test]
fn merges_partial_class_base_type_from_other_fragment() {
    // Real Unity case: one partial fragment declares the base, another
    // declares fields. The merged metadata must report `MonoBehaviour` as
    // the base regardless of which fragment was selected as primary.
    let content = r#"
public partial class Boss
{
    public int hp;
}

public partial class Boss : MonoBehaviour
{
    public float speed;
}
"#;
    let meta = parse_cs_script(content, Some("Boss")).unwrap();
    assert_eq!(meta.class_name, "Boss");
    assert_eq!(meta.base_type.as_deref(), Some("MonoBehaviour"));
}

#[test]
fn ignores_comments_inside_base_list() {
    // Comments are tree-sitter "extras" and appear as named children of any
    // surrounding node. Without filtering, the comment text would be picked
    // up as a base type.
    let content = r#"
public class Hero : MonoBehaviour /* legacy: was Behaviour */
{
    public int hp;
}
"#;
    let meta = parse_cs_script(content, Some("Hero")).unwrap();
    assert_eq!(meta.base_type.as_deref(), Some("MonoBehaviour"));
}

#[test]
fn rejects_field_serialize_field_on_non_auto_property() {
    // `[field: SerializeField]` on a property with an explicit body is
    // invalid C#, but tree-sitter still parses it. The parser must not
    // emit a phantom field for it.
    let content = r#"
using UnityEngine;
public class Hero : MonoBehaviour
{
    private int _hp;
    [field: SerializeField] public int Hp { get { return _hp; } set { _hp = value; } }
    [field: SerializeField] public int Computed => 42;
}
"#;
    let meta = parse_cs_script(content, Some("Hero")).unwrap();
    assert!(
        meta.serialized_fields.iter().all(|f| f.name != "Hp"),
        "explicit-body property must not yield a phantom field"
    );
    assert!(
        meta.serialized_fields.iter().all(|f| f.name != "Computed"),
        "expression-bodied property must not yield a phantom field"
    );
}

#[test]
fn decodes_escaped_quote_in_non_verbatim_former_name() {
    let content = r#"
using UnityEngine;
using UnityEngine.Serialization;
public class Quoted : MonoBehaviour
{
    [FormerlySerializedAs("foo\"bar")]
    public int counter;
}
"#;
    let meta = parse_cs_script(content, Some("Quoted")).unwrap();
    let counter = meta
        .serialized_fields
        .iter()
        .find(|f| f.name == "counter")
        .unwrap();
    assert_eq!(counter.former_names, vec!["foo\"bar"]);
}

#[test]
fn returns_none_when_only_enum_present() {
    // Enums aren't tracked as primary types — files containing only enums
    // must return None so the watcher can index them as no-metadata scripts.
    let content = r#"
namespace Demo
{
    public enum BuildFlag { Mono, IL2CPP }
}
"#;
    assert!(parse_cs_script(content, Some("BuildFlag")).is_none());
}

#[test]
fn finds_class_with_preproc_if_inside_collection_initializer() {
    // Repro for a real top-level class whose Dictionary field initializer interleaves
    // `#if UNITY_2022_1_OR_NEWER` blocks between entries. Tree-sitter's
    // recovery on this shape is not exercised by the existing top-level
    // `#if UNITY_EDITOR` test, and the watcher logged "no parseable C# type"
    // for the real file. We require the class name to come back so the
    // ref_graph index gets the right script_class_name.
    let content = r#"
using System.Collections.Generic;
namespace ExampleProject.HotReload.Editor {
    internal static class ExampleHotReloadSuggestions {
        public static Dictionary<int, string> suggestionMap = new Dictionary<int, string> {
            { 1, "first" },
#if UNITY_2022_1_OR_NEWER
            { 2, "twenty-two" },
#endif
            { 3, "third" },
#if UNITY_2020_1_OR_NEWER
            { 4, "twenty" },
#endif
        };
    }
}
"#;
    let meta = parse_cs_script(content, Some("ExampleHotReloadSuggestions"))
        .expect("class with preproc_if inside collection initializer must parse");
    assert_eq!(meta.class_name, "ExampleHotReloadSuggestions");
    assert_eq!(
        meta.namespace.as_deref(),
        Some("ExampleProject.HotReload.Editor")
    );
}
