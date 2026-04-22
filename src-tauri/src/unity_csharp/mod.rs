//! Neutral C# script parsing layer for Unity assets.
//!
//! This module owns the low-level C# source parsing used to extract
//! Unity-relevant metadata from `.cs` files: the primary type's name,
//! its base type, namespace, and the list of serialized fields.
//!
//! It used to live under `crate::ref_graph::script_parser` as a regex-based
//! parser, but multiple consumers (`ref_graph`, `diff::semantic`) need it
//! independently and the regex approach was brittle on real-world C# (nested
//! types, multi-class files, multi-line attributes, `[field: SerializeField]`
//! auto-properties, etc.). It now wraps `tree-sitter-c-sharp` to get a real
//! AST.
//!
//! It deliberately does not own:
//! - File I/O, mtime/size capture, content hashing — that's `ref_graph`'s
//!   `script_parser` snapshot layer
//! - Cross-script base-type resolution — that's `diff::semantic::script` /
//!   `ref_graph` who follow inheritance chains across files
//! - Display labels / UI semantics — that's `diff::semantic`
//!
//! ## Known limitations
//!
//! - **Short type names only.** `class_name` and `base_type` are stored as
//!   the rightmost segment, with namespace and generic arguments stripped.
//!   Two distinct types `Foo.Bar.Baz` and `Qux.Baz` collide on `Baz` at the
//!   ref-graph level. This matches Unity's own GUID-by-class-name binding
//!   model and the `class_to_guid` index in `ref_graph::script_parser`.
//! - **Base class vs interface heuristic.** C# allows
//!   `class Foo : SomeBase, IBar` and the syntax tree cannot tell `SomeBase`
//!   from `IBar` apart from the convention that base classes come first.
//!   We additionally skip apparent interfaces (names matching `I` followed
//!   by an uppercase letter) so that
//!   `class Hero : IDamageable, MonoBehaviour` resolves the base to
//!   `MonoBehaviour`. This is a heuristic and will be wrong for types like
//!   `Item` (an interface that doesn't follow the convention) or `ItemBase`
//!   (a class that happens to start with `I`).
//! - **`[FormerlySerializedAs]` argument decoding is naive.** See the
//!   `unquote_csharp_string` TODO in `parser.rs`.

mod parser;
#[cfg(test)]
mod tests;

pub use parser::{
    parse_cs_script, parse_cs_script_status, ScriptFieldMeta, ScriptMetadata, ScriptParseStatus,
};
