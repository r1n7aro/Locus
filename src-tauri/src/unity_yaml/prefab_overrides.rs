//! Prefab-instance property modifications (`m_Modifications`) captured for
//! the asset index, plus helpers for the property-path shapes Unity writes.
//!
//! Verified against Unity 6.5 output:
//!
//! ```yaml
//! --- !u!1001 &1369411486
//! PrefabInstance:
//!   m_Modification:
//!     m_Modifications:
//!     - target: {fileID: 4954498630159034338, guid: d82f41f6…, type: 3}
//!       propertyPath: tuning.steps.Array.data[0].max          # plain field
//!       value: 33
//!     - target: {fileID: 4954498630159034338, guid: d82f41f6…, type: 3}
//!       propertyPath: main                                    # the field now
//!       value: 7542395272935243781                            # holds this rid
//!     - target: …
//!       propertyPath: 'managedReferences[7542395272935243781]' # instance type
//!       value: Assembly-CSharp Probe.Combat.Chain             # (`-2` / empty
//!     - target: …                                             #  value = null)
//!       propertyPath: managedReferences[7542395272935243781].power
//!       value: 41
//! ```
//!
//! `target` always names an object of the *immediate* source prefab. Objects
//! nested deeper (variants, nested prefabs) are addressed by the fileID Unity
//! derives as `(prefabInstanceFileID ^ sourceFileID) & i64::MAX`, usually with
//! no document of their own — see [`nested_file_id`].

use crate::asset_db::types::Guid;

use super::managed_refs::ManagedTypeIdentity;

/// One `m_Modifications` entry of a PrefabInstance document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPrefabOverride {
    /// fileID of the PrefabInstance document holding the modification.
    pub instance_file_id: i64,
    pub target_guid: Guid,
    pub target_file_id: i64,
    pub property_path: String,
    /// Decoded `value:` scalar (first line only for wrapped strings).
    pub value: String,
    /// 1-based line of `propertyPath:`.
    pub line: u32,
}

/// Property-path heads of built-in Transform / RectTransform / GameObject
/// state. Every prefab instance carries them, so they are not indexed.
const BUILTIN_HEADS: &[&str] = &[
    "m_LocalPosition",
    "m_LocalRotation",
    "m_LocalScale",
    "m_LocalEulerAnglesHint",
    "m_RootOrder",
    "m_AnchorMin",
    "m_AnchorMax",
    "m_AnchoredPosition",
    "m_SizeDelta",
    "m_Pivot",
    "m_Name",
    "m_IsActive",
    "m_ConstrainProportionsScale",
    "m_TagString",
    "m_Layer",
    "m_StaticEditorFlags",
];

/// First property-path segment: `tuning.range.max` → `tuning`,
/// `ranges.Array.data[0].max` → `ranges`, `managedReferences[7].x` →
/// `managedReferences`.
pub fn property_path_head(path: &str) -> &str {
    let end = path.find(['.', '[']).unwrap_or(path.len());
    &path[..end]
}

/// Whether a modification is indexed (custom data, managed references).
pub fn is_indexed_override_path(path: &str) -> bool {
    let head = property_path_head(path);
    !head.is_empty() && !BUILTIN_HEADS.contains(&head)
}

/// `managedReferences[R]` → `(R, "")`; `managedReferences[R].a.b` → `(R, "a.b")`.
pub fn parse_managed_reference_path(path: &str) -> Option<(i64, &str)> {
    let rest = path.strip_prefix("managedReferences[")?;
    let close = rest.find(']')?;
    let rid = rest[..close].trim().parse::<i64>().ok()?;
    let tail = &rest[close + 1..];
    if tail.is_empty() {
        return Some((rid, ""));
    }
    Some((rid, tail.strip_prefix('.')?))
}

/// Value of a `managedReferences[R]` modification: `"<asm> <ns>.<class>"`
/// (Unity's `managedReferenceFullTypename`). `None` for a null reference
/// (empty value).
pub fn parse_managed_type_value(value: &str) -> Option<ManagedTypeIdentity> {
    let value = value.trim();
    let (assembly, full_name) = value.split_once(' ')?;
    let full_name = full_name.trim();
    if assembly.is_empty() || full_name.is_empty() {
        return None;
    }
    // Generic argument lists may contain dots; split the namespace off the
    // part before them.
    let base_end = full_name.find('[').unwrap_or(full_name.len());
    let (namespace, class_name) = match full_name[..base_end].rfind('.') {
        Some(dot) => (&full_name[..dot], &full_name[dot + 1..]),
        None => ("", full_name),
    };
    Some(ManagedTypeIdentity {
        class_name: class_name.to_string(),
        namespace: namespace.to_string(),
        assembly: assembly.to_string(),
    })
}

/// FileID Unity assigns, inside an asset holding PrefabInstance
/// `instance_file_id`, to the source prefab's object `source_file_id`. The
/// operation is its own inverse, so it also maps back to the source.
pub fn nested_file_id(instance_file_id: i64, source_file_id: i64) -> i64 {
    (instance_file_id ^ source_file_id) & i64::MAX
}

/// One step of a serialized property path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathToken {
    Field(String),
    /// `Array.data[N]` element.
    Element(usize),
    /// `Array.size`.
    Size,
}

/// Tokenize a Unity property path: `a.list.Array.data[2].b` →
/// `[Field a, Field list, Element 2, Field b]`. Managed-reference prefixes
/// are not handled here (strip them with [`parse_managed_reference_path`]).
pub fn tokenize_property_path(path: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        if part == "Array" {
            match parts.peek().copied() {
                Some("size") => {
                    parts.next();
                    tokens.push(PathToken::Size);
                    continue;
                }
                Some(next) if next.starts_with("data[") => {
                    parts.next();
                    let index = next
                        .trim_start_matches("data[")
                        .trim_end_matches(']')
                        .parse()
                        .unwrap_or(0);
                    tokens.push(PathToken::Element(index));
                    continue;
                }
                _ => {}
            }
        }
        if !part.is_empty() {
            tokens.push(PathToken::Field(part.to_string()));
        }
    }
    tokens
}

/// Read `{fileID: N, guid: G, …}` of a `- target:` line.
pub(super) fn parse_target(text: &str) -> Option<(Guid, i64)> {
    let file_id = super::tokenizer::extract_internal_file_id(text)?;
    let guid_text = super::tokenizer::extract_value(text, "guid:")?;
    let guid = crate::asset_db::types::parse_guid_hex(guid_text.trim().trim_end_matches(','))?;
    Some((guid, file_id))
}
