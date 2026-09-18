//! Lossless, snapshot-local Unity YAML parsing and selective three-way merging.
//!
//! This module deliberately has no Editor, filesystem, Git, or legacy YAML parser
//! dependencies. Unselected changes always retain the target bytes. Ambiguous
//! sequence identity and unsupported syntax are errors, never guessed repairs.

mod edit;
mod merge;
mod parser;
mod packed;
mod validation;
pub mod semantic;
pub mod authoring;
pub mod prefab;
pub(crate) mod property_values;

pub(crate) use merge::sequence_keys;
pub use packed::{packed_array_value, PackedElement};

pub use edit::{
    decode_asset_value, edit, edit_with_hints, inspect, inspect_with_hints, AssetField, AssetObject,
    AssetOperation, AssetSnapshot, EditOutput, ScalarHint, ScalarHints, validate_set_values,
};

pub use merge::{
    prepare_merge, prepare_merge_with_aliases, Change, ChangeCatalog, ChangeKind, ChangeStatus,
    Conflict, Decision, FieldAlias, MergeOutput, MergeSession, Resolution, ValueSummary,
};
pub use parser::{
    parse, parse_shared, Asset, CoreError, Diagnostic, Document, Entry, Item, Limits, Node,
    NodeKind, ScalarStyle, Severity, Span, PARSER_VERSION,
};
pub use validation::{validate, AssetReference, ReferenceKind};

#[cfg(test)]
mod tests;
