use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::validation::{path_child, pptr, scalar_text};
use super::{
    parse, validate, Asset, CoreError, Diagnostic, Document, Entry, Node, NodeKind, Severity, Span,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    AddField,
    RemoveField,
    ModifyField,
    AddObject,
    RemoveObject,
    ModifyObject,
    ReplaceSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Clean,
    Conflict,
    AlreadyApplied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueSummary {
    pub kind: String,
    pub hash: String,
    /// Bounded preview, not a round-trip serialization API.
    pub raw: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub id: String,
    pub object_id: String,
    /// RFC 6901 pointer. Stable identity selectors replace array indices where
    /// the Unity format provides an identity (e.g. `@rid=101`).
    pub property_path: String,
    pub kind: ChangeKind,
    pub status: ChangeStatus,
    pub reason: Option<String>,
    pub base: Option<ValueSummary>,
    pub target: Option<ValueSummary>,
    pub source: Option<ValueSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeCatalog {
    pub parser_version: u32,
    pub base_hash: String,
    pub target_hash: String,
    pub source_hash: String,
    pub changes: Vec<Change>,
    pub diagnostics: Vec<Diagnostic>,
    pub aliases: Vec<FieldAlias>,
}

/// A rename proven by the caller's immutable C# schema snapshots. Only a
/// same-parent rename into an existing target field is accepted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldAlias {
    pub object_id: String,
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolution {
    /// Apply the source delta only if the three-way rule proves it unambiguous.
    Include,
    Source,
    Target,
    Base,
    /// Delete an object or complete block-mapping entry. Flow entry deletion
    /// requires selecting the enclosing mapping to keep delimiters explicit.
    Delete,
    /// JSON scalar/array/map; strings are safely YAML-quoted, never raw YAML.
    Set {
        value: serde_json::Value,
    },
    Exclude,
    Defer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub change_id: String,
    pub resolution: Resolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub change_id: String,
    pub object_id: String,
    pub property_path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeOutput {
    pub bytes: Vec<u8>,
    pub conflicts: Vec<Conflict>,
    pub diagnostics: Vec<Diagnostic>,
    pub ready: bool,
    pub applied_change_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum RegionKind {
    Value,
    Entry,
    Document,
}

#[derive(Debug, Clone)]
struct Region {
    span: Span,
    kind: RegionKind,
    indent: usize,
    key: Option<String>,
}

#[derive(Clone, Copy)]
struct Point<'a> {
    node: &'a Node,
    entry: Option<&'a Entry>,
    parent: Option<&'a Node>,
}

impl Point<'_> {
    fn region(self) -> Region {
        let entry = self.entry.filter(|_| self.parent.is_some_and(|p| !p.flow));
        Region {
            span: entry.map(|e| e.span).unwrap_or(self.node.span),
            kind: if entry.is_some() {
                RegionKind::Entry
            } else {
                RegionKind::Value
            },
            indent: self.parent.map(|p| p.indent).unwrap_or(self.node.indent),
            key: self.entry.map(|e| e.key.clone()),
        }
    }
}

#[derive(Debug, Clone)]
struct StoredChange {
    base: Option<Region>,
    target: Option<Region>,
    source: Option<Region>,
    insertion: Option<(usize, usize)>,
}

#[derive(Debug, Clone)]
struct Patch {
    span: Span,
    bytes: Vec<u8>,
    id: String,
    object_id: String,
    path: String,
}

/// An immutable snapshot triple. Creating/revising a plan never edits its input.
#[derive(Debug, Clone)]
pub struct MergeSession {
    base: Arc<Asset>,
    target: Arc<Asset>,
    source: Arc<Asset>,
    catalog: ChangeCatalog,
    stored: Vec<StoredChange>,
    original_base: Option<Arc<Asset>>,
    original_source: Option<Arc<Asset>>,
}

pub fn prepare_merge(base: &[u8], target: &[u8], source: &[u8]) -> Result<MergeSession, CoreError> {
    let mut snapshots = Vec::with_capacity(3);
    MergeSession::from_shared(
        parse_snapshot(base, &mut snapshots)?,
        parse_snapshot(target, &mut snapshots)?,
        parse_snapshot(source, &mut snapshots)?,
    )
}

/// Keep each distinct byte snapshot once for this merge, even when its AST is
/// too large for the bounded process cache. Byte equality preserves spelling,
/// comments and missing/empty distinctions; semantic equality is insufficient.
fn parse_snapshot(bytes: &[u8], snapshots: &mut Vec<Arc<Asset>>) -> Result<Arc<Asset>, CoreError> {
    if let Some(asset) = snapshots.iter().find(|asset| asset.bytes.as_ref() == bytes) {
        return Ok(Arc::clone(asset));
    }
    let asset = super::parser::parse_shared(bytes)?;
    snapshots.push(Arc::clone(&asset));
    Ok(asset)
}

#[cfg(test)]
mod snapshot_sharing_tests {
    use super::*;

    #[test]
    fn local_snapshot_reuses_uncached_ast_only_for_identical_raw_bytes() {
        let bytes = b"--- !u!114 &11400000\r\nMonoBehaviour:\r\n  health: 100 # original\r\n";
        // Bypass the global cache deliberately: this verifies local ownership,
        // including the path used by ASTs exceeding the cache byte budget.
        let uncached = Arc::new(parse(bytes).unwrap());
        let mut snapshots = vec![Arc::clone(&uncached)];
        let copied_bytes = bytes.to_vec();
        let reused = parse_snapshot(&copied_bytes, &mut snapshots).unwrap();
        assert!(Arc::ptr_eq(&uncached, &reused));
        assert_eq!(snapshots.len(), 1);
        let other_spelling = b"--- !u!114 &11400000\nMonoBehaviour:\n  health: 100 # other\n";
        let distinct = parse_snapshot(other_spelling, &mut snapshots).unwrap();
        assert!(!Arc::ptr_eq(&uncached, &distinct));
        assert_eq!(snapshots.len(), 2);
        assert_eq!(uncached.bytes.as_ref(), bytes);
        assert_eq!(distinct.bytes.as_ref(), other_spelling);
    }

    #[test]
    fn same_byte_pairs_share_arcs_and_render_preserves_the_exact_target() {
        let a = b"--- !u!114 &11400000\nMonoBehaviour:\n  health: 100 # a\n";
        let b = b"--- !u!114 &11400000\nMonoBehaviour:\n  health: 101 # b\n";
        for (base, target, source) in [(a.as_slice(), a.as_slice(), b.as_slice()), (a.as_slice(), b.as_slice(), a.as_slice()), (a.as_slice(), b.as_slice(), b.as_slice()), (a.as_slice(), a.as_slice(), a.as_slice())] {
            let session = prepare_merge(base, target, source).unwrap();
            assert_eq!(Arc::ptr_eq(&session.base, &session.target), base == target);
            assert_eq!(Arc::ptr_eq(&session.base, &session.source), base == source);
            assert_eq!(Arc::ptr_eq(&session.target, &session.source), target == source);
            assert_eq!(session.render(&[]).unwrap().bytes, target);
        }
    }

    #[test]
    fn alias_normalization_shares_originals_and_matching_normalized_target() {
        let old = b"--- !u!114 &11400000\r\nMonoBehaviour:\r\n  health: 100 # preserved\r\n";
        let target = b"--- !u!114 &11400000\r\nMonoBehaviour:\r\n  hitPoints: 100 # preserved\r\n";
        let session = prepare_merge_with_aliases(old, target, old, &[FieldAlias {
            object_id: "11400000".into(), old_path: "/MonoBehaviour/health".into(), new_path: "/MonoBehaviour/hitPoints".into(),
        }]).unwrap();
        assert!(Arc::ptr_eq(session.original_base.as_ref().unwrap(), session.original_source.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&session.base, &session.target));
        assert!(Arc::ptr_eq(&session.target, &session.source));
        assert_eq!(session.render(&[]).unwrap().bytes, target);
        assert_eq!(session.render(&[MergeSession::file_decision(Resolution::Source)]).unwrap().bytes, old);
    }
}

pub fn prepare_merge_with_aliases(
    base: &[u8],
    target: &[u8],
    source: &[u8],
    aliases: &[FieldAlias],
) -> Result<MergeSession, CoreError> {
    if aliases.is_empty() {
        return prepare_merge(base, target, source);
    }
    let mut snapshots = Vec::with_capacity(5);
    let base_asset = parse_snapshot(base, &mut snapshots)?;
    let target_asset = parse_snapshot(target, &mut snapshots)?;
    let source_asset = parse_snapshot(source, &mut snapshots)?;
    let mut seen = HashSet::new();
    for alias in aliases {
        if !seen.insert((alias.object_id.clone(), alias.new_path.clone())) {
            return Err(CoreError::new(
                "ambiguous_alias",
                "multiple aliases map to the same target field",
                0,
            ));
        }
        let (old_parent, old_key) = split_pointer(&alias.old_path)?;
        let (new_parent, new_key) = split_pointer(&alias.new_path)?;
        if old_parent != new_parent || old_key == new_key {
            return Err(CoreError::new(
                "alias_scope",
                "field aliases must rename one key within the same parent mapping",
                0,
            ));
        }
        if point_at(&target_asset, &alias.object_id, &alias.new_path)?.is_none()
            || point_at(&target_asset, &alias.object_id, &alias.old_path)?.is_some()
        {
            return Err(CoreError::new(
                "alias_target",
                "target must contain only the proven new field name",
                0,
            ));
        }
    }
    let normalized_base = rename_snapshot_keys(&base_asset, aliases)?;
    let normalized_source = rename_snapshot_keys(&source_asset, aliases)?;
    let mut session = MergeSession::from_shared(
        parse_snapshot(&normalized_base, &mut snapshots)?,
        target_asset,
        parse_snapshot(&normalized_source, &mut snapshots)?,
    )?;
    session.catalog.base_hash = base_asset.content_hash.clone();
    session.catalog.source_hash = source_asset.content_hash.clone();
    session.catalog.aliases = aliases.to_vec();
    session.original_base = Some(base_asset);
    session.original_source = Some(source_asset);
    Ok(session)
}

impl MergeSession {
    pub fn new(base: Asset, target: Asset, source: Asset) -> Result<Self, CoreError> {
        Self::from_shared(Arc::new(base), Arc::new(target), Arc::new(source))
    }

    pub fn from_shared(
        base: Arc<Asset>,
        target: Arc<Asset>,
        source: Arc<Asset>,
    ) -> Result<Self, CoreError> {
        for asset in [&base, &target, &source] {
            if !asset.is_writable() {
                return Err(CoreError::new("invalid_asset", "duplicate keys/object IDs make structural writes ambiguous; inspect diagnostics or explicitly select a whole file",0));
            }
        }
        let mut catalog = ChangeCatalog {
            parser_version: super::PARSER_VERSION,
            base_hash: base.content_hash.clone(),
            target_hash: target.content_hash.clone(),
            source_hash: source.content_hash.clone(),
            changes: Vec::new(),
            diagnostics: Vec::new(),
            aliases: Vec::new(),
        };
        // Snapshot diagnostics are informational here; only NEW output errors
        // block rendering, so pre-existing missing external/broken references
        // cannot be silently erased or misclassified as a merge regression.
        for (name, asset) in [("base", &base), ("target", &target), ("source", &source)] {
            for mut diagnostic in validate(asset) {
                diagnostic.message = format!("{name}: {}", diagnostic.message);
                catalog.diagnostics.push(diagnostic);
            }
        }
        let mut builder = Builder {
            base: &base,
            target: &target,
            source: &source,
            catalog: &mut catalog,
            stored: Vec::new(),
        };
        let mut seen = HashSet::new();
        for doc in base.documents.iter().chain(source.documents.iter()) {
            if !seen.insert(doc.object_id.clone()) {
                continue;
            }
            builder.document(&doc.object_id)?;
        }
        let stored = builder.stored;
        Ok(Self {
            base,
            target,
            source,
            catalog,
            stored,
            original_base: None,
            original_source: None,
        })
    }

    pub fn catalog(&self) -> &ChangeCatalog {
        &self.catalog
    }
    pub fn base_asset(&self) -> &Asset {
        &self.base
    }
    pub fn target_asset(&self) -> &Asset {
        &self.target
    }
    pub fn source_asset(&self) -> &Asset {
        &self.source
    }

    pub fn decisions_for_object(&self, object_id: &str, resolution: Resolution) -> Vec<Decision> {
        self.catalog
            .changes
            .iter()
            .filter(|c| c.object_id == object_id)
            .map(|c| Decision {
                change_id: c.id.clone(),
                resolution: resolution.clone(),
            })
            .collect()
    }

    /// A full-object choice, including fields unchanged between base/source.
    /// It conflicts with any simultaneous fine-grained choice for that object.
    pub fn object_decision(object_id: &str, resolution: Resolution) -> Decision {
        Decision {
            change_id: format!("$object:{object_id}"),
            resolution,
        }
    }

    /// Whole-file choice is explicit and conflicts with selected inner changes.
    pub fn file_decision(resolution: Resolution) -> Decision {
        Decision {
            change_id: "$file".into(),
            resolution,
        }
    }

    /// Address a target field even when it has no incoming source delta.
    pub fn field_decision(
        object_id: &str,
        property_path: &str,
        resolution: Resolution,
    ) -> Decision {
        Decision {
            change_id: format!(
                "$field:{}",
                serde_json::to_string(&(object_id, property_path)).expect("string tuple JSON")
            ),
            resolution,
        }
    }

    pub fn render(&self, decisions: &[Decision]) -> Result<MergeOutput, CoreError> {
        let mut conflicts = Vec::new();
        let mut patches = Vec::new();
        let mut unique = HashSet::new();
        let lookup: HashMap<_, _> = self
            .catalog
            .changes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();
        let active = |r: &Resolution| !matches!(r, Resolution::Exclude | Resolution::Defer);
        let whole = decisions
            .iter()
            .find(|d| d.change_id == "$file" && active(&d.resolution));
        if let Some(whole) = whole {
            if decisions
                .iter()
                .any(|d| d.change_id != "$file" && active(&d.resolution))
            {
                return Err(CoreError::new(
                    "overlapping_scope",
                    "whole-file choice overlaps an object/field choice",
                    0,
                ));
            }
            let bytes = match whole.resolution {
                Resolution::Source => self
                    .original_source
                    .as_ref()
                    .unwrap_or(&self.source)
                    .bytes
                    .to_vec(),
                Resolution::Target => self.target.bytes.to_vec(),
                Resolution::Base => self
                    .original_base
                    .as_ref()
                    .unwrap_or(&self.base)
                    .bytes
                    .to_vec(),
                _ => {
                    return Err(CoreError::new(
                        "whole_file_resolution",
                        "whole-file choice requires source, target, or base",
                        0,
                    ))
                }
            };
            return self.finish(bytes, Vec::new(), vec!["$file".into()]);
        }
        let object_scopes: HashSet<_> = decisions
            .iter()
            .filter(|d| active(&d.resolution))
            .filter_map(|d| d.change_id.strip_prefix("$object:"))
            .collect();
        for decision in decisions {
            if !unique.insert(decision.change_id.as_str()) {
                return Err(CoreError::new(
                    "duplicate_decision",
                    format!("duplicate decision {}", decision.change_id),
                    0,
                ));
            }
            if decision.change_id == "$file" {
                continue;
            }
            if let Some(encoded) = decision.change_id.strip_prefix("$field:") {
                let (object_id, path): (String, String) = serde_json::from_str(encoded)
                    .map_err(|e| CoreError::new("field_selector", e.to_string(), 0))?;
                if !active(&decision.resolution) {
                    continue;
                }
                if object_scopes.contains(object_id.as_str()) {
                    return Err(CoreError::new(
                        "overlapping_scope",
                        "whole-object choice overlaps an explicit field choice",
                        0,
                    ));
                }
                if matches!(decision.resolution, Resolution::Target) {
                    continue;
                }
                patches.push(self.explicit_field_patch(&object_id, &path, decision)?);
                continue;
            }
            if let Some(object_id) = decision.change_id.strip_prefix("$object:") {
                if !active(&decision.resolution) {
                    continue;
                }
                let chosen = match decision.resolution {
                    Resolution::Source => {
                        let asset = self.original_source.as_ref().unwrap_or(&self.source);
                        asset.document(object_id).map(|d| (asset, d))
                    }
                    Resolution::Target => {
                        self.target.document(object_id).map(|d| (&self.target, d))
                    }
                    Resolution::Base => {
                        let asset = self.original_base.as_ref().unwrap_or(&self.base);
                        asset.document(object_id).map(|d| (asset, d))
                    }
                    Resolution::Delete => None,
                    _ => {
                        return Err(CoreError::new(
                            "object_resolution",
                            "whole-object choice requires source, target, base, or delete",
                            0,
                        ))
                    }
                };
                let target = self.target.document(object_id);
                if target.is_none() && chosen.is_none() {
                    return Err(CoreError::new(
                        "unknown_object",
                        format!("no object {object_id} in the chosen snapshot or target"),
                        0,
                    ));
                }
                patches.push(Patch {
                    span: target
                        .map(|d| d.span)
                        .unwrap_or(Span::new(self.target.bytes.len(), self.target.bytes.len())),
                    bytes: chosen
                        .map(|(a, d)| d.span.bytes(&a.bytes).to_vec())
                        .unwrap_or_default(),
                    id: decision.change_id.clone(),
                    object_id: object_id.into(),
                    path: String::new(),
                });
                continue;
            }
            let index = *lookup.get(decision.change_id.as_str()).ok_or_else(|| {
                CoreError::new(
                    "unknown_change",
                    format!("unknown change {}", decision.change_id),
                    0,
                )
            })?;
            let change = &self.catalog.changes[index];
            if !active(&decision.resolution) {
                continue;
            }
            if object_scopes.contains(change.object_id.as_str()) {
                return Err(CoreError::new(
                    "overlapping_scope",
                    format!(
                        "object {} choice overlaps field {}",
                        change.object_id, change.property_path
                    ),
                    0,
                ));
            }
            if matches!(decision.resolution, Resolution::Target) {
                continue;
            }
            if matches!(decision.resolution, Resolution::Include)
                && change.status == ChangeStatus::Conflict
            {
                conflicts.push(Conflict { change_id:change.id.clone(),object_id:change.object_id.clone(),property_path:change.property_path.clone(),code:"concurrent_change".into(),message:change.reason.clone().unwrap_or_else(|| "target and source changed differently; explicitly choose source, target, base, or a typed value".into()) });
                continue;
            }
            if matches!(decision.resolution, Resolution::Include)
                && change.status == ChangeStatus::AlreadyApplied
            {
                continue;
            }
            let stored = &self.stored[index];
            let chosen = match &decision.resolution {
                Resolution::Source | Resolution::Include => {
                    stored.source.as_ref().map(|r| (&self.source, r))
                }
                Resolution::Base => stored.base.as_ref().map(|r| (&self.base, r)),
                Resolution::Set { .. } | Resolution::Delete => None,
                _ => continue,
            };
            let span = stored
                .target
                .as_ref()
                .map(|r| r.span)
                .or_else(|| stored.insertion.map(|(p, _)| Span::new(p, p)));
            let Some(span) = span else {
                continue;
            };
            let indent = stored
                .target
                .as_ref()
                .map(|r| r.indent)
                .or_else(|| stored.insertion.map(|(_, i)| i))
                .unwrap_or(0);
            if matches!(decision.resolution, Resolution::Delete)
                && stored
                    .target
                    .as_ref()
                    .is_some_and(|r| matches!(r.kind, RegionKind::Value))
            {
                return Err(CoreError::new("delete_scope","delete a complete block field/object or explicitly select the enclosing flow mapping",span.start));
            }
            let bytes = if let Resolution::Set { value } = &decision.resolution {
                let region = stored
                    .target
                    .as_ref()
                    .or(stored.source.as_ref())
                    .or(stored.base.as_ref())
                    .ok_or_else(|| CoreError::new("set_region", "missing field region", 0))?;
                if matches!(region.kind, RegionKind::Document) {
                    return Err(CoreError::new(
                        "typed_object",
                        "set operates on fields; use object selection for an entire Unity document",
                        0,
                    ));
                }
                let rendered = json_yaml(value)?;
                if matches!(region.kind, RegionKind::Entry) {
                    let key = region
                        .key
                        .as_ref()
                        .ok_or_else(|| CoreError::new("set_key", "typed field has no key", 0))?;
                    format!(
                        "{}{}: {}{}",
                        " ".repeat(indent),
                        yaml_key(key),
                        rendered,
                        self.target.newline
                    )
                    .into_bytes()
                } else {
                    rendered.into_bytes()
                }
            } else if let Some((asset, region)) = chosen {
                let bytes = region.span.bytes(&asset.bytes);
                if matches!(region.kind, RegionKind::Entry) {
                    reindent(bytes, region.indent, indent, self.target.newline)
                } else {
                    bytes.to_vec()
                }
            } else {
                Vec::new()
            };
            patches.push(Patch {
                span,
                bytes,
                id: change.id.clone(),
                object_id: change.object_id.clone(),
                path: change.property_path.clone(),
            });
        }
        // Ascending stable order means independent additions at the same end
        // position retain source order. Nonzero overlapping spans never win by
        // accidental call order.
        patches.sort_by_key(|p| (p.span.start, p.span.end));
        let mut end = 0;
        for patch in &patches {
            if patch.span.start < end {
                conflicts.push(Conflict { change_id:patch.id.clone(),object_id:patch.object_id.clone(),property_path:patch.path.clone(),code:"overlapping_patch".into(),message:"coarse and fine operations overlap; choose one scope or explicitly expand it".into() });
            }
            end = end.max(patch.span.end);
        }
        if !conflicts.is_empty() {
            return Ok(MergeOutput {
                bytes: self.target.bytes.to_vec(),
                conflicts,
                diagnostics: Vec::new(),
                ready: false,
                applied_change_ids: Vec::new(),
            });
        }
        let mut bytes = Vec::with_capacity(self.target.bytes.len());
        let mut cursor = 0;
        let mut ids = Vec::new();
        for patch in patches {
            bytes.extend_from_slice(&self.target.bytes[cursor..patch.span.start]);
            // Appending fields/documents to files without a terminal newline
            // must not concatenate the previous token and the new key/header.
            if patch.span.start == patch.span.end
                && !patch.bytes.is_empty()
                && !bytes.is_empty()
                && !bytes.ends_with(b"\n")
            {
                bytes.extend_from_slice(self.target.newline.as_bytes());
            }
            bytes.extend_from_slice(&patch.bytes);
            cursor = patch.span.end;
            ids.push(patch.id);
        }
        bytes.extend_from_slice(&self.target.bytes[cursor..]);
        self.finish(bytes, conflicts, ids)
    }

    fn explicit_field_patch(
        &self,
        object_id: &str,
        path: &str,
        decision: &Decision,
    ) -> Result<Patch, CoreError> {
        if path.is_empty() {
            return Err(CoreError::new(
                "field_scope",
                "use object_decision for an entire Unity object",
                0,
            ));
        }
        let target = point_at(&self.target, object_id, path)?;
        let (parent_path, key) = split_pointer(path)?;
        let parent = point_at(&self.target, object_id, &parent_path)?;
        let region = if let Some(point) = target {
            point.region()
        } else {
            let parent = parent.ok_or_else(|| {
                CoreError::new(
                    "missing_parent",
                    "field insertion requires an existing target parent mapping",
                    0,
                )
            })?;
            if parent.node.flow || parent.node.entries().is_none() {
                return Err(CoreError::new(
                    "insert_scope",
                    "insert requires a block mapping; set the complete flow mapping instead",
                    parent.node.span.start,
                ));
            }
            Region {
                span: Span::new(parent.node.span.end, parent.node.span.end),
                kind: RegionKind::Entry,
                indent: parent.node.indent,
                key: Some(key.clone()),
            }
        };
        let bytes = match &decision.resolution {
            Resolution::Set { value } => {
                let rendered = json_yaml(value)?;
                if matches!(region.kind, RegionKind::Entry) {
                    let first_prefix = if region.span.start < region.span.end {
                        self.target.bytes[region.span.start..region.span.end]
                            .iter()
                            .take_while(|b| **b == b' ')
                            .count()
                    } else {
                        region.indent
                    };
                    format!(
                        "{}{}: {}{}",
                        " ".repeat(first_prefix),
                        yaml_key(&key),
                        rendered,
                        self.target.newline
                    )
                    .into_bytes()
                } else {
                    rendered.into_bytes()
                }
            }
            Resolution::Delete => {
                if target.is_none() {
                    return Err(CoreError::new(
                        "unknown_field",
                        "cannot delete a missing target field",
                        0,
                    ));
                }
                if !matches!(region.kind, RegionKind::Entry) {
                    return Err(CoreError::new(
                        "delete_scope",
                        "delete a complete block entry or set the enclosing flow mapping",
                        region.span.start,
                    ));
                }
                Vec::new()
            }
            Resolution::Source | Resolution::Base => {
                let asset = if matches!(decision.resolution, Resolution::Source) {
                    &self.source
                } else {
                    &self.base
                };
                let chosen = point_at(asset, object_id, path)?.ok_or_else(|| {
                    CoreError::new(
                        "missing_side_field",
                        "chosen snapshot has no such field; use explicit delete if intended",
                        0,
                    )
                })?;
                let chosen_region = chosen.region();
                if matches!(region.kind, RegionKind::Entry)
                    != matches!(chosen_region.kind, RegionKind::Entry)
                {
                    return Err(CoreError::new(
                        "field_representation",
                        "field representation differs; choose its parent or provide a typed value",
                        region.span.start,
                    ));
                }
                if matches!(region.kind, RegionKind::Entry) {
                    reindent(
                        chosen_region.span.bytes(&asset.bytes),
                        chosen_region.indent,
                        region.indent,
                        self.target.newline,
                    )
                } else {
                    chosen_region.span.bytes(&asset.bytes).to_vec()
                }
            }
            _ => {
                return Err(CoreError::new(
                    "field_resolution",
                    "explicit fields require set, delete, source, target, or base",
                    0,
                ))
            }
        };
        Ok(Patch {
            span: region.span,
            bytes,
            id: decision.change_id.clone(),
            object_id: object_id.into(),
            path: path.into(),
        })
    }

    fn finish(
        &self,
        mut bytes: Vec<u8>,
        mut conflicts: Vec<Conflict>,
        applied_change_ids: Vec<String>,
    ) -> Result<MergeOutput, CoreError> {
        if self.target.documents.is_empty() && bytes.starts_with(b"--- !u!") {
            let source = if self.source.documents.is_empty() {
                &self.base
            } else {
                &self.source
            };
            if let Some(first) = source.documents.first() {
                if first.span.start > 0 {
                    let mut prefixed = source.bytes[..first.span.start].to_vec();
                    prefixed.extend_from_slice(&bytes);
                    bytes = prefixed;
                }
            }
        }
        if !applied_change_ids.is_empty()
            && std::str::from_utf8(&bytes).is_ok_and(|s| {
                s.lines().all(|line| {
                    let s = line.trim_start_matches('\u{feff}').trim();
                    s.is_empty() || s.starts_with('%') || s.starts_with('#')
                })
            })
        {
            bytes.clear();
        }
        let parsed = if bytes.is_empty() {
            Asset::absent()
        } else {
            parse(&bytes)?
        };
        let diagnostics = validate(&parsed);
        let baseline: HashSet<_> = validate(&self.target)
            .into_iter()
            .map(|d| (d.code, d.object_id, d.property_path, d.message))
            .collect();
        for d in &diagnostics {
            if d.severity == Severity::Error
                && !baseline.contains(&(
                    d.code.clone(),
                    d.object_id.clone(),
                    d.property_path.clone(),
                    d.message.clone(),
                ))
            {
                conflicts.push(Conflict {
                    change_id: String::new(),
                    object_id: d.object_id.clone().unwrap_or_default(),
                    property_path: d.property_path.clone().unwrap_or_default(),
                    code: d.code.clone(),
                    message: d.message.clone(),
                });
            }
        }
        let ready = conflicts.is_empty();
        // Never expose an invalid candidate as ready-to-write bytes. Callers can
        // inspect conflicts and add the missing identity/reference operations.
        Ok(MergeOutput {
            bytes: if ready {
                bytes
            } else {
                self.target.bytes.to_vec()
            },
            conflicts,
            diagnostics,
            ready,
            applied_change_ids: if ready {
                applied_change_ids
            } else {
                Vec::new()
            },
        })
    }
}

struct Builder<'a, 'b> {
    base: &'a Asset,
    target: &'a Asset,
    source: &'a Asset,
    catalog: &'b mut ChangeCatalog,
    stored: Vec<StoredChange>,
}

impl Builder<'_, '_> {
    fn document(&mut self, id: &str) -> Result<(), CoreError> {
        let base = self.base.document(id);
        let target = self.target.document(id);
        let source = self.source.document(id);
        if docs_equal(base, source) {
            return Ok(());
        }
        if let (Some(b), Some(t), Some(s)) = (base, target, source) {
            if b.class_id == s.class_id
                && b.class_id == t.class_id
                && b.stripped == s.stripped
                && b.stripped == t.stripped
            {
                return self.node(
                    id,
                    "",
                    Some(Point {
                        node: &b.root,
                        entry: None,
                        parent: None,
                    }),
                    Some(Point {
                        node: &t.root,
                        entry: None,
                        parent: None,
                    }),
                    Some(Point {
                        node: &s.root,
                        entry: None,
                        parent: None,
                    }),
                    None,
                );
            }
        }
        let kind = if base.is_none() {
            ChangeKind::AddObject
        } else if source.is_none() {
            ChangeKind::RemoveObject
        } else {
            ChangeKind::ModifyObject
        };
        let status = if base.is_none() && target.is_some() && source.is_some() {
            ChangeStatus::Conflict
        } else if docs_equal(target, source) {
            ChangeStatus::AlreadyApplied
        } else if docs_equal(target, base) {
            ChangeStatus::Clean
        } else {
            ChangeStatus::Conflict
        };
        let id_hash = change_id(id, "", base.map(|d| &d.root), source.map(|d| &d.root));
        self.catalog.changes.push(Change { id:id_hash,object_id:id.into(),property_path:String::new(),kind,status,reason:Some("object addition/deletion, class identity, or stripped state requires an object-level decision".into()),base:base.map(|d|summary(self.base,&d.root)),target:target.map(|d|summary(self.target,&d.root)),source:source.map(|d|summary(self.source,&d.root)) });
        let region = |d: &Document| Region {
            span: d.span,
            kind: RegionKind::Document,
            indent: 0,
            key: None,
        };
        self.stored.push(StoredChange {
            base: base.map(region),
            target: target.map(region),
            source: source.map(region),
            insertion: Some((self.target.bytes.len(), 0)),
        });
        Ok(())
    }

    fn node(
        &mut self,
        object_id: &str,
        path: &str,
        base: Option<Point<'_>>,
        target: Option<Point<'_>>,
        source: Option<Point<'_>>,
        insertion: Option<(usize, usize)>,
    ) -> Result<(), CoreError> {
        if points_equal(base, source) {
            return Ok(());
        }
        let mut reason = None;
        let mut identity_collision = false;
        if let (Some(b), Some(t), Some(s)) = (base, target, source) {
            let same_type = b
                .node
                .get("type")
                .zip(s.node.get("type"))
                .zip(t.node.get("type"))
                .map(|((b, s), t)| b.fingerprint == s.fingerprint && b.fingerprint == t.fingerprint)
                .unwrap_or(true);
            let managed_entry = path.contains("/RefIds/@rid=");
            if managed_entry && !same_type {
                reason=Some("managed-reference type identity changed; type and data require a joint explicit choice".into());
            } else if pptr(b.node) || pptr(s.node) || pptr(t.node) {
                reason = Some("PPtr GUID/fileID/type form one reference identity".into());
            } else if let (NodeKind::Mapping(bm), NodeKind::Mapping(tm), NodeKind::Mapping(sm)) =
                (&b.node.kind, &t.node.kind, &s.node.kind)
            {
                let same_styles = b.node.flow == t.node.flow && b.node.flow == s.node.flow;
                let same_keys = |x: &[Entry], y: &[Entry]| {
                    x.len() == y.len() && x.iter().all(|e| y.iter().any(|f| f.key == e.key))
                };
                // Block mappings can insert/delete complete entry spans. Flow
                // mapping structural edits remain atomic to preserve delimiters.
                if same_styles && (!b.node.flow || (same_keys(bm, sm) && same_keys(bm, tm))) {
                    let bmap: HashMap<_, _> = bm.iter().map(|e| (e.key.as_str(), e)).collect();
                    let tmap: HashMap<_, _> = tm.iter().map(|e| (e.key.as_str(), e)).collect();
                    let smap: HashMap<_, _> = sm.iter().map(|e| (e.key.as_str(), e)).collect();
                    let mut seen = HashSet::new();
                    for entry in bm.iter().chain(sm.iter()) {
                        if !seen.insert(entry.key.as_str()) {
                            continue;
                        }
                        let bchild = entry_point(bmap.get(entry.key.as_str()), b.node);
                        let tchild = entry_point(tmap.get(entry.key.as_str()), t.node);
                        let schild = entry_point(smap.get(entry.key.as_str()), s.node);
                        self.node(
                            object_id,
                            &path_child(path, &entry.key),
                            bchild,
                            tchild,
                            schild,
                            Some((t.node.span.end, t.node.indent)),
                        )?;
                    }
                    return Ok(());
                }
                reason = Some(
                    "mapping representation or flow key set changed; choose the complete mapping"
                        .into(),
                );
            } else if let (NodeKind::Sequence(bi), NodeKind::Sequence(ti), NodeKind::Sequence(si)) =
                (&b.node.kind, &t.node.kind, &s.node.kind)
            {
                let bk = sequence_keys(self.base, b.node, path);
                let tk = sequence_keys(self.target, t.node, path);
                let sk = sequence_keys(self.source, s.node, path);
                if let (Some(bk), Some(tk), Some(sk)) = (bk, tk, sk) {
                    if path.ends_with("/references/RefIds") {
                        let base_ids: HashSet<_> = bk.iter().collect();
                        let target_new: HashSet<_> =
                            tk.iter().filter(|id| !base_ids.contains(id)).collect();
                        identity_collision = sk
                            .iter()
                            .any(|id| !base_ids.contains(id) && target_new.contains(id));
                    }
                    if bk == tk && bk == sk {
                        for (i, key) in bk.iter().enumerate() {
                            self.node(
                                object_id,
                                &path_child(path, key),
                                Some(Point {
                                    node: &bi[i].value,
                                    entry: None,
                                    parent: Some(b.node),
                                }),
                                Some(Point {
                                    node: &ti[i].value,
                                    entry: None,
                                    parent: Some(t.node),
                                }),
                                Some(Point {
                                    node: &si[i].value,
                                    entry: None,
                                    parent: Some(s.node),
                                }),
                                None,
                            )?;
                        }
                        return Ok(());
                    }
                    reason=Some(if identity_collision{"both snapshots independently introduced the same managed-reference rid; explicitly choose or remap the complete identity closure"}else{"identity sequence membership/order changed; explicit whole-sequence resolution preserves order and graph closure"}.into());
                } else {
                    reason=Some("sequence has no proven unique Unity identity; positional pairing would be ambiguous".into());
                }
            }
        }
        let status = if identity_collision {
            ChangeStatus::Conflict
        } else if points_equal(target, source) {
            ChangeStatus::AlreadyApplied
        } else if points_equal(target, base) {
            ChangeStatus::Clean
        } else {
            ChangeStatus::Conflict
        };
        let kind = if base.is_none() {
            ChangeKind::AddField
        } else if source.is_none() {
            ChangeKind::RemoveField
        } else if source.is_some_and(|p| matches!(p.node.kind, NodeKind::Sequence(_))) {
            ChangeKind::ReplaceSequence
        } else {
            ChangeKind::ModifyField
        };
        let id = change_id(
            object_id,
            path,
            base.map(|p| p.node),
            source.map(|p| p.node),
        );
        self.catalog.changes.push(Change {
            id,
            object_id: object_id.into(),
            property_path: path.into(),
            kind,
            status,
            reason,
            base: base.map(|p| summary(self.base, p.node)),
            target: target.map(|p| summary(self.target, p.node)),
            source: source.map(|p| summary(self.source, p.node)),
        });
        self.stored.push(StoredChange {
            base: base.map(Point::region),
            target: target.map(Point::region),
            source: source.map(Point::region),
            insertion,
        });
        Ok(())
    }
}

fn decode_pointer_token(token: &str) -> Result<String, CoreError> {
    let mut out = String::new();
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => {
                    return Err(CoreError::new(
                        "property_path",
                        "invalid JSON pointer escape",
                        0,
                    ))
                }
            }
        } else {
            out.push(c)
        }
    }
    Ok(out)
}

fn split_pointer(path: &str) -> Result<(String, String), CoreError> {
    if !path.starts_with('/') {
        return Err(CoreError::new(
            "property_path",
            "property path must be an RFC 6901 pointer",
            0,
        ));
    }
    let (parent, key) = path.rsplit_once('/').expect("leading slash");
    Ok((parent.into(), decode_pointer_token(key)?))
}

fn point_at<'a>(
    asset: &'a Asset,
    object_id: &str,
    path: &str,
) -> Result<Option<Point<'a>>, CoreError> {
    let Some(doc) = asset.document(object_id) else {
        return Ok(None);
    };
    let mut point = Point {
        node: &doc.root,
        entry: None,
        parent: None,
    };
    if path.is_empty() {
        return Ok(Some(point));
    }
    if !path.starts_with('/') {
        return Err(CoreError::new(
            "property_path",
            "property path must be an RFC 6901 pointer",
            0,
        ));
    }
    let mut walked = String::new();
    for raw in path[1..].split('/') {
        let key = decode_pointer_token(raw)?;
        match &point.node.kind {
            NodeKind::Mapping(entries) => {
                let Some(entry) = entries.iter().find(|e| e.key == key) else {
                    return Ok(None);
                };
                point = Point {
                    node: &entry.value,
                    entry: Some(entry),
                    parent: Some(point.node),
                };
            }
            NodeKind::Sequence(items) => {
                let index = if key.starts_with('@') {
                    sequence_keys(asset, point.node, &walked)
                        .and_then(|keys| keys.iter().position(|k| *k == key))
                } else {
                    key.parse::<usize>().ok()
                };
                let Some(item) = index.and_then(|i| items.get(i)) else {
                    return Ok(None);
                };
                point = Point {
                    node: &item.value,
                    entry: None,
                    parent: Some(point.node),
                };
            }
            _ => return Ok(None),
        }
        walked = path_child(&walked, &key);
    }
    Ok(Some(point))
}

fn rename_snapshot_keys(asset: &Asset, aliases: &[FieldAlias]) -> Result<Vec<u8>, CoreError> {
    let mut patches = Vec::new();
    for alias in aliases {
        let old = point_at(asset, &alias.object_id, &alias.old_path)?;
        let new = point_at(asset, &alias.object_id, &alias.new_path)?;
        if old.is_some() && new.is_some() {
            return Err(CoreError::new(
                "ambiguous_alias",
                "snapshot contains both the old and new field",
                0,
            ));
        }
        if let Some(old) = old {
            let entry = old.entry.ok_or_else(|| {
                CoreError::new(
                    "alias_scope",
                    "only mapping field keys can be renamed",
                    old.node.span.start,
                )
            })?;
            let (_, new_key) = split_pointer(&alias.new_path)?;
            patches.push((entry.key_span, yaml_key(&new_key).into_bytes()));
        }
    }
    patches.sort_by_key(|(span, _)| span.start);
    let mut bytes = Vec::new();
    let mut cursor = 0;
    for (span, replacement) in patches {
        if span.start < cursor {
            return Err(CoreError::new(
                "ambiguous_alias",
                "multiple aliases modify the same key",
                span.start,
            ));
        }
        bytes.extend_from_slice(&asset.bytes[cursor..span.start]);
        bytes.extend_from_slice(&replacement);
        cursor = span.end;
    }
    bytes.extend_from_slice(&asset.bytes[cursor..]);
    Ok(bytes)
}

fn docs_equal(a: Option<&Document>, b: Option<&Document>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.class_id == b.class_id
                && a.stripped == b.stripped
                && a.root.fingerprint == b.root.fingerprint
        }
        _ => false,
    }
}
fn points_equal(a: Option<Point<'_>>, b: Option<Point<'_>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.node.fingerprint == b.node.fingerprint,
        _ => false,
    }
}
fn entry_point<'a>(entry: Option<&&'a Entry>, parent: &'a Node) -> Option<Point<'a>> {
    entry.map(|entry| Point {
        node: &entry.value,
        entry: Some(*entry),
        parent: Some(parent),
    })
}
fn change_id(object_id: &str, path: &str, base: Option<&Node>, source: Option<&Node>) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(object_id.as_bytes());
    hash.update(&[0]);
    hash.update(path.as_bytes());
    hash.update(&[0]);
    for node in [base, source] {
        if let Some(n) = node {
            hash.update(&[1]);
            hash.update(&n.fingerprint);
        } else {
            hash.update(&[0]);
        }
    }
    hash.finalize().to_hex().to_string()
}
fn summary(asset: &Asset, node: &Node) -> ValueSummary {
    let text = asset.text(node.span);
    let mut end = text.len().min(1200);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let kind = match &node.kind {
        NodeKind::Scalar(style) => format!("scalar:{style:?}"),
        NodeKind::Mapping(_) => "mapping".into(),
        NodeKind::Sequence(_) => "sequence".into(),
    };
    ValueSummary {
        kind,
        hash: blake3::Hash::from_bytes(node.fingerprint)
            .to_hex()
            .to_string(),
        raw: text[..end].into(),
        truncated: end < text.len(),
    }
}

pub(crate) fn sequence_keys(asset: &Asset, node: &Node, path: &str) -> Option<Vec<String>> {
    let items = node.items()?;
    let mut keys = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    for item in items {
        let key = if path.ends_with("/references/RefIds") {
            let rid = item.value.get("rid").and_then(|n| scalar_text(asset, n))?;
            if rid.parse::<i64>().is_err() {
                return None;
            }
            format!("@rid={rid}")
        } else if path.ends_with("/m_Modifications") {
            let target = item.value.get("target")?;
            if !pptr(target) {
                return None;
            }
            let property = item
                .value
                .get("propertyPath")
                .and_then(|n| scalar_text(asset, n))?;
            format!(
                "@override={}:{}",
                blake3::Hash::from_bytes(target.fingerprint).to_hex(),
                property
            )
        } else if path.ends_with("/m_Component") {
            let component = item.value.get("component")?;
            if !pptr(component) {
                return None;
            }
            format!(
                "@component={}",
                component
                    .get("fileID")
                    .and_then(|n| scalar_text(asset, n))?
            )
        } else if path.ends_with("/m_Children") {
            if !pptr(&item.value) {
                return None;
            }
            format!(
                "@child={}",
                item.value
                    .get("fileID")
                    .and_then(|n| scalar_text(asset, n))?
            )
        } else {
            return None;
        };
        if !seen.insert(key.clone()) {
            return None;
        }
        keys.push(key);
    }
    Some(keys)
}

fn reindent(bytes: &[u8], old: usize, new: usize, newline: &str) -> Vec<u8> {
    if old == new {
        return bytes.to_vec();
    }
    let text = std::str::from_utf8(bytes).expect("validated UTF-8");
    let mut out = String::new();
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\r', '\n']);
        let spaces = content.bytes().take_while(|b| *b == b' ').count();
        // First sequence-map entry may start immediately after '- '. Its span
        // excludes the dash and must retain that positioning.
        if i == 0 && spaces < old {
            out.push_str(content);
        } else {
            out.push_str(&" ".repeat(new + spaces.saturating_sub(old)));
            out.push_str(&content[spaces..]);
        }
        if line.ends_with('\n') {
            out.push_str(newline);
        }
    }
    out.into_bytes()
}

fn yaml_key(key: &str) -> String {
    if key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        key.into()
    } else {
        serde_json::to_string(key).expect("string JSON")
    }
}
pub(crate) fn json_yaml(value: &serde_json::Value) -> Result<String, CoreError> {
    // JSON is a safe subset of flow YAML for Unity's structural fields. Reject
    // numbers outside JSON's finite representation in the caller/JSON decoder.
    match value {
        serde_json::Value::Null => Ok("null".into()),
        serde_json::Value::Bool(b) => Ok(if *b { "1" } else { "0" }.into()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::String(s) => {
            serde_json::to_string(s).map_err(|e| CoreError::new("typed_value", e.to_string(), 0))
        }
        serde_json::Value::Array(items) => Ok(format!(
            "[{}]",
            items
                .iter()
                .map(json_yaml)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        serde_json::Value::Object(map) => {
            if map.len() == 1 {
                if let Some(value) = map.get("$locus_packed_array") { return super::packed::marker(value); }
            }
            let sorted: BTreeMap<_, _> = map.iter().collect();
            let reference = map.contains_key("fileID")
                && map.keys().all(|key| matches!(key.as_str(), "fileID" | "guid" | "type"));
            Ok(format!(
                "{{{}}}",
                sorted
                    .into_iter()
                    .map(|(k, v)| {
                        let rendered = if reference && k == "guid" {
                            let guid = v.as_str().filter(|guid| guid.len() == 32 && guid.bytes().all(|byte| byte.is_ascii_hexdigit()))
                                .ok_or_else(|| CoreError::new("invalid_guid", "reference GUID must contain exactly 32 hexadecimal characters", 0))?;
                            guid.to_string()
                        } else { json_yaml(v)? };
                        Ok(format!("{}: {}", yaml_key(k), rendered))
                    })
                    .collect::<Result<Vec<_>, CoreError>>()?
                    .join(", ")
            ))
        }
    }
}
