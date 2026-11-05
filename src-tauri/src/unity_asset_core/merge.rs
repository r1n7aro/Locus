use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::identity::{decoded_scalar, identity_fingerprint, ListIdentity};
use super::validation::{path_child, pptr};
use super::{
    parse, validate, Asset, CoreError, Diagnostic, Document, Entry, Item, Node, NodeKind, Severity,
    Span,
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
    /// Complete block-mapping entry lines.
    Entry,
    /// Complete block-sequence element lines, including its `- ` marker.
    Item,
    Document,
}

impl RegionKind {
    /// Whole-line regions that can be deleted, inserted or reindented.
    fn lines(self) -> bool {
        matches!(self, RegionKind::Entry | RegionKind::Item)
    }
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
    item: Option<&'a Item>,
    parent: Option<&'a Node>,
}

impl<'a> Point<'a> {
    fn root(node: &'a Node) -> Self {
        Point { node, entry: None, item: None, parent: None }
    }
    fn in_sequence(item: &'a Item, parent: &'a Node) -> Self {
        Point { node: &item.value, entry: None, item: Some(item), parent: Some(parent) }
    }
    fn region(self) -> Region {
        let block_parent = self.parent.is_some_and(|p| !p.flow);
        let entry = self.entry.filter(|_| block_parent);
        let item = self.item.filter(|_| block_parent);
        Region {
            span: entry
                .map(|e| e.span)
                .or(item.map(|i| i.span))
                .unwrap_or(self.node.span),
            kind: if entry.is_some() {
                RegionKind::Entry
            } else if item.is_some() {
                RegionKind::Item
            } else {
                RegionKind::Value
            },
            indent: self.parent.map(|p| p.indent).unwrap_or(self.node.indent),
            key: self.entry.map(|e| e.key.clone()),
        }
    }
}

/// An entry/element region that starts after other content on its line, such
/// as the first field of a `- ` sequence element, cannot be removed alone.
pub(super) fn starts_line(bytes: &[u8], position: usize) -> bool {
    position == 0 || bytes[position - 1] == b'\n'
}

/// Content for a field or element that the target does not contain yet.
enum Insertion<'a> {
    Value(&'a serde_json::Value),
    Copy(&'a Asset, Region),
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
    /// Fills an empty scalar on its existing line rather than adding lines.
    inline: bool,
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
        let anchors = document_anchors(&target, &source);
        let mut builder = Builder {
            base: &base,
            target: &target,
            source: &source,
            catalog: &mut catalog,
            stored: Vec::new(),
            anchors,
        };
        let mut seen = HashSet::new();
        for doc in base.documents.iter().chain(source.documents.iter()) {
            if !seen.insert(doc.object_id.clone()) {
                continue;
            }
            builder.document(&doc.object_id)?;
        }
        let stored = builder.stored;
        super::ownership::link_deletions(&base, &mut catalog.changes);
        super::ownership::owner_hints(&source, &mut catalog.changes);
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
                patches.extend(self.explicit_field_patch(&object_id, &path, decision)?);
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
                let insertion = || {
                    let at = chosen.map_or(self.target.bytes.len(), |(asset, _)| {
                        document_anchor(&self.target, asset, object_id)
                    });
                    Span::new(at, at)
                };
                patches.push(Patch {
                    span: target.map(|d| d.span).unwrap_or_else(insertion),
                    bytes: chosen
                        .map(|(a, d)| d.span.bytes(&a.bytes).to_vec())
                        .unwrap_or_default(),
                    id: decision.change_id.clone(),
                    object_id: object_id.into(),
                    path: String::new(),
                    inline: false,
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
            if matches!(decision.resolution, Resolution::Delete) {
                if let Some(region) = stored.target.as_ref() {
                    self.check_deletable(region)?;
                }
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
                if stored.target.is_some() {
                    if let Some(point) = point_at(&self.target, &change.object_id, &change.property_path)? {
                        for (span, bytes, inline) in self.set_value(point, value, &change.property_path)? {
                            patches.push(Patch {
                                span,
                                bytes,
                                id: change.id.clone(),
                                object_id: change.object_id.clone(),
                                path: change.property_path.clone(),
                                inline,
                            });
                        }
                        continue;
                    }
                }
                match region.kind {
                    RegionKind::Entry => {
                        let key = region
                            .key
                            .as_ref()
                            .ok_or_else(|| CoreError::new("set_key", "typed field has no key", 0))?;
                        format!(
                            "{}{}: {}{}",
                            " ".repeat(indent),
                            yaml_key(key),
                            json_yaml_unity(value, true)?,
                            self.target.newline
                        )
                        .into_bytes()
                    }
                    RegionKind::Item => super::set_patch::Patcher::new(&self.target)
                        .render_item(value, indent, None)?
                        .into_bytes(),
                    _ => json_yaml_unity(value, false)?.into_bytes(),
                }
            } else if let Some((asset, region)) = chosen {
                if region.kind.lines() {
                    transplant(asset, region, &self.target.bytes, span.start, indent, self.target.newline)
                } else {
                    with_newline(region.span.bytes(&asset.bytes), self.target.newline)
                }
            } else {
                Vec::new()
            };
            if bytes.is_empty()
                && stored.target.as_ref().is_some_and(|region| {
                    matches!(region.kind, RegionKind::Entry | RegionKind::Item)
                        && !starts_line(&self.target.bytes, region.span.start)
                })
            {
                conflicts.push(Conflict {
                    change_id: change.id.clone(),
                    object_id: change.object_id.clone(),
                    property_path: change.property_path.clone(),
                    code: "delete_scope".into(),
                    message: "this removes the first field of a sequence element, which shares its '- ' line; take or set the whole element instead".into(),
                });
                continue;
            }
            patches.push(Patch {
                span,
                bytes,
                id: change.id.clone(),
                object_id: change.object_id.clone(),
                path: change.property_path.clone(),
                inline: false,
            });
        }
        // Ascending stable order means independent additions at the same end
        // position retain source order. Nonzero overlapping spans never win by
        // accidental call order.
        // Whole documents inserted at one offset of a fileID-ordered target
        // follow fileID order as well; everything else keeps push order.
        let ordered = file_id_order(&self.target).is_some();
        patches.sort_by_key(|p| {
            let document = ordered && p.span.start == p.span.end && p.path.is_empty() && p.bytes.starts_with(b"--- !u!");
            (p.span.start, p.span.end, document.then(|| p.object_id.parse::<i64>().ok()).flatten())
        });
        // Several elements added to one empty flow sequence each remove its
        // `[]`; that removal happens once.
        patches.dedup_by(|a, b| a.span.start == b.span.start && a.span.end == b.span.end && a.span.start < a.span.end && a.bytes == b.bytes);
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
            // Inline fills (a value after an empty `key:`) stay on their line.
            if patch.span.start == patch.span.end
                && !patch.bytes.is_empty()
                && !patch.inline
                && !bytes.is_empty()
                && !bytes.ends_with(b"\n")
            {
                bytes.extend_from_slice(self.target.newline.as_bytes());
            }
            bytes.extend_from_slice(&patch.bytes);
            cursor = patch.span.end;
            if !ids.contains(&patch.id) {
                ids.push(patch.id);
            }
        }
        bytes.extend_from_slice(&self.target.bytes[cursor..]);
        self.finish(bytes, conflicts, ids)
    }

    /// Address a target field even when the source catalog has no delta for it.
    /// Sequence elements are whole-line regions: they can be deleted, copied
    /// from a snapshot, or inserted at their identity/index position.
    fn explicit_field_patch(
        &self,
        object_id: &str,
        path: &str,
        decision: &Decision,
    ) -> Result<Vec<Patch>, CoreError> {
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
        let patch = |span: Span, bytes: Vec<u8>, inline: bool| Patch {
            span,
            bytes,
            id: decision.change_id.clone(),
            object_id: object_id.into(),
            path: path.into(),
            inline,
        };
        let missing_parent = || {
            CoreError::new(
                "missing_parent",
                "field insertion requires an existing target parent mapping or block sequence",
                0,
            )
        };
        match &decision.resolution {
            Resolution::Set { value } => {
                if let Some(point) = target {
                    return Ok(self
                        .set_value(point, value, path)?
                        .into_iter()
                        .map(|(span, bytes, inline)| patch(span, bytes, inline))
                        .collect());
                }
                let parent = parent.ok_or_else(missing_parent)?;
                let parts =
                    self.insertion(object_id, parent, &parent_path, &key, Insertion::Value(value))?;
                Ok(parts.into_iter().map(|(span, bytes)| patch(span, bytes, false)).collect())
            }
            Resolution::Delete => {
                let point = target.ok_or_else(|| {
                    CoreError::new("unknown_field", "cannot delete a missing target field", 0)
                })?;
                let region = point.region();
                self.check_deletable(&region)?;
                Ok(vec![patch(region.span, Vec::new(), false)])
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
                let Some(point) = target else {
                    let parent = parent.ok_or_else(missing_parent)?;
                    let parts = self.insertion(
                        object_id,
                        parent,
                        &parent_path,
                        &key,
                        Insertion::Copy(asset, chosen_region),
                    )?;
                    return Ok(parts.into_iter().map(|(span, bytes)| patch(span, bytes, false)).collect());
                };
                let region = point.region();
                if region.kind.lines() != chosen_region.kind.lines() {
                    return Err(CoreError::new(
                        "field_representation",
                        "field representation differs; choose its parent or provide a typed value",
                        region.span.start,
                    ));
                }
                let bytes = if region.kind.lines() {
                    transplant(
                        asset,
                        &chosen_region,
                        &self.target.bytes,
                        region.span.start,
                        region.indent,
                        self.target.newline,
                    )
                } else {
                    with_newline(chosen_region.span.bytes(&asset.bytes), self.target.newline)
                };
                Ok(vec![patch(region.span, bytes, false)])
            }
            _ => Err(CoreError::new(
                "field_resolution",
                "explicit fields require set, delete, source, target, or base",
                0,
            )),
        }
    }

    /// Minimal typed write of an existing target node.
    fn set_value(
        &self,
        point: Point<'_>,
        value: &serde_json::Value,
        path: &str,
    ) -> Result<Vec<(Span, Vec<u8>, bool)>, CoreError> {
        use super::set_patch::{Holder, Patcher};
        let parent_flow = point.parent.is_some_and(|p| p.flow);
        let (holder, flow) = if parent_flow {
            (Holder::Inline, true)
        } else if let Some(entry) = point.entry {
            (Holder::Entry(entry), false)
        } else if let Some(item) = point.item {
            (Holder::Item(item), false)
        } else {
            (Holder::Inline, point.node.flow)
        };
        let identity = match (point.parent, point.entry) {
            (Some(parent), Some(entry)) => super::identity::identity_scalar(parent, &entry.key),
            _ => false,
        };
        let mut patcher = Patcher::new(&self.target);
        // The addressed scalar itself is written as typed (strings quoted);
        // scalars changed inside an addressed container keep their style.
        patcher.set(point.node, holder, value, path, flow, identity, true)?;
        Ok(patcher.patches)
    }

    fn check_deletable(&self, region: &Region) -> Result<(), CoreError> {
        match region.kind {
            RegionKind::Entry | RegionKind::Item
                if starts_line(&self.target.bytes, region.span.start) =>
            {
                Ok(())
            }
            RegionKind::Entry => Err(CoreError::new(
                "delete_scope",
                "this is the first field of a sequence element and shares its '- ' line; delete or replace the element itself (its parent path) instead",
                region.span.start,
            )),
            RegionKind::Item => Err(CoreError::new(
                "delete_scope",
                "this is the first element of a compact nested sequence and shares the enclosing '- ' line; set the enclosing element (its parent path) instead",
                region.span.start,
            )),
            RegionKind::Document => Err(CoreError::new(
                "delete_scope",
                "use objects.delete for a complete Unity object",
                region.span.start,
            )),
            _ => Err(CoreError::new(
                "delete_scope",
                "values inside a flow collection ({...} or [...]) cannot be deleted individually; set the enclosing flow value instead",
                region.span.start,
            )),
        }
    }

    /// Patches that add a field/element absent from the target: one insertion,
    /// or for an empty flow sequence (`key: []`) the removal of `[]` plus the
    /// element as the first item of a block sequence.
    fn insertion(
        &self,
        object_id: &str,
        parent: Point<'_>,
        parent_path: &str,
        key: &str,
        source: Insertion<'_>,
    ) -> Result<Vec<(Span, Vec<u8>)>, CoreError> {
        let newline = self.target.newline;
        let representation = || {
            CoreError::new(
                "field_representation",
                "field representation differs; choose its parent or provide a typed value",
                parent.node.span.start,
            )
        };
        match &parent.node.kind {
            NodeKind::Mapping(_) if !parent.node.flow => {
                let indent = parent.node.indent;
                let bytes = match source {
                    Insertion::Value(value) => format!(
                        "{}{}: {}{}",
                        " ".repeat(indent),
                        yaml_key(key),
                        json_yaml_unity(value, true)?,
                        newline
                    )
                    .into_bytes(),
                    Insertion::Copy(asset, region) => {
                        if !matches!(region.kind, RegionKind::Entry) {
                            return Err(representation());
                        }
                        transplant(asset, &region, &self.target.bytes, parent.node.span.end, indent, newline)
                    }
                };
                let at = parent.node.span.end;
                Ok(vec![(Span::new(at, at), bytes)])
            }
            NodeKind::Sequence(items) if parent.node.flow && items.is_empty() => {
                self.empty_flow_insertion(parent, parent_path, key, source)
            }
            NodeKind::Sequence(items) if !parent.node.flow => {
                let indent = parent.node.indent;
                let end = items
                    .last()
                    .map(|i| i.span.end)
                    .unwrap_or(parent.node.span.end);
                let at = if key.starts_with('@') {
                    let target_keys = sequence_keys(&self.target, parent.node, parent_path)
                        .ok_or_else(|| {
                            CoreError::new(
                                "insert_scope",
                                "the target sequence has no stable element identity; address elements by index",
                                parent.node.span.start,
                            )
                        })?;
                    let target_index: HashMap<String, usize> = target_keys
                        .into_iter()
                        .enumerate()
                        .map(|(i, k)| (k, i))
                        .collect();
                    match &source {
                        Insertion::Copy(asset, _) => {
                            let chosen_keys = point_at(asset, object_id, parent_path)?
                                .and_then(|p| sequence_keys(asset, p.node, parent_path))
                                .unwrap_or_default();
                            match chosen_keys.iter().position(|k| k == key) {
                                Some(position) => {
                                    sequence_insertion(
                                        &chosen_keys,
                                        position,
                                        &target_index,
                                        items,
                                        parent.node,
                                    )
                                    .0
                                }
                                None => end,
                            }
                        }
                        Insertion::Value(value) => {
                            let identity = super::identity::json_sequence_keys(
                                parent_path,
                                std::slice::from_ref(*value),
                            );
                            if identity
                                .as_deref()
                                .and_then(|keys| keys.first())
                                .map(String::as_str)
                                != Some(key)
                            {
                                return Err(CoreError::new(
                                    "identity_mismatch",
                                    "the value's own identity (target/propertyPath, rid, or fileID) does not match the addressed element",
                                    parent.node.span.start,
                                ));
                            }
                            end
                        }
                    }
                } else {
                    let index: usize = key.parse().map_err(|_| {
                        CoreError::new(
                            "property_path",
                            "sequence elements are addressed by index or identity selector",
                            0,
                        )
                    })?;
                    if index > items.len() {
                        return Err(CoreError::new(
                            "array_index",
                            format!(
                                "index {index} is out of bounds for array length {} (insertion at length is allowed)",
                                items.len()
                            ),
                            parent.node.span.start,
                        ));
                    }
                    items.get(index).map(|i| i.span.start).unwrap_or(end)
                };
                if at < self.target.bytes.len() && !starts_line(&self.target.bytes, at) {
                    return Err(CoreError::new(
                        "insert_scope",
                        "this position is the first element of a compact nested sequence, which shares the enclosing '- ' line; set the enclosing element (its parent path) instead",
                        at,
                    ));
                }
                let bytes = match source {
                    Insertion::Value(value) => super::set_patch::Patcher::new(&self.target)
                        .render_item(value, indent, items.first().map(|i| &i.value))?
                        .into_bytes(),
                    Insertion::Copy(asset, region) => {
                        if !matches!(region.kind, RegionKind::Item) {
                            return Err(representation());
                        }
                        transplant(asset, &region, &self.target.bytes, at, indent, newline)
                    }
                };
                Ok(vec![(Span::new(at, at), bytes)])
            }
            _ => Err(CoreError::new(
                "insert_scope",
                "insertion requires a block mapping or block sequence parent; set the complete flow value instead",
                parent.node.span.start,
            )),
        }
    }

    /// `key: []` closing its line becomes `key:` followed by a block item at
    /// the key's indent, as Unity writes a non-empty list.
    fn empty_flow_insertion(
        &self,
        parent: Point<'_>,
        parent_path: &str,
        key: &str,
        source: Insertion<'_>,
    ) -> Result<Vec<(Span, Vec<u8>)>, CoreError> {
        let bytes = &self.target.bytes;
        let (start, end) = (parent.node.span.start, parent.node.span.end);
        let unsupported = || {
            CoreError::new(
                "insert_scope",
                "insertion requires a block mapping or block sequence parent; set the complete flow value instead",
                start,
            )
        };
        let line_end = bytes[end..].iter().position(|b| *b == b'\n').map(|n| end + n).ok_or_else(unsupported)?;
        if !bytes[end..line_end].iter().all(|b| matches!(b, b' ' | b'\r')) {
            return Err(unsupported());
        }
        let mut open = start;
        while open > 0 && bytes[open - 1] == b' ' {
            open -= 1;
        }
        if open == 0 || bytes[open - 1] != b':' {
            return Err(unsupported());
        }
        let line_start = bytes[..open].iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        let indent = bytes[line_start..].iter().take_while(|b| **b == b' ').count();
        let at = line_end + 1;
        let item = match source {
            Insertion::Value(value) => {
                if key.starts_with('@') {
                    let identity = super::identity::json_sequence_keys(parent_path, std::slice::from_ref(value));
                    if identity.as_deref().and_then(|keys| keys.first()).map(String::as_str) != Some(key) {
                        return Err(CoreError::new(
                            "identity_mismatch",
                            "the value's own identity (target/propertyPath, rid, or fileID) does not match the addressed element",
                            start,
                        ));
                    }
                }
                super::set_patch::Patcher::new(&self.target).render_item(value, indent, None)?.into_bytes()
            }
            Insertion::Copy(asset, region) => {
                if !matches!(region.kind, RegionKind::Item) {
                    return Err(CoreError::new(
                        "field_representation",
                        "field representation differs; choose its parent or provide a typed value",
                        start,
                    ));
                }
                transplant(asset, &region, bytes, at.min(bytes.len()), indent, self.target.newline)
            }
        };
        Ok(vec![(Span::new(open, end), Vec::new()), (Span::new(at, at), item)])
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
    /// Insertion offsets of source documents the target lacks.
    anchors: HashMap<String, usize>,
}

/// The target's (fileID, start offset) pairs when its documents are in Unity's
/// own serialization order, ascending signed fileID, which Unity writes for
/// scenes and prefabs. `None` for any other order or a non-numeric fileID.
fn file_id_order(asset: &Asset) -> Option<Vec<(i64, usize)>> {
    let mut order: Vec<(i64, usize)> = Vec::with_capacity(asset.documents.len());
    for document in &asset.documents {
        let id: i64 = document.object_id.parse().ok()?;
        if order.last().is_some_and(|(previous, _)| *previous >= id) {
            return None;
        }
        order.push((id, document.span.start));
    }
    Some(order)
}

/// Where a document absent from the target goes. In a target that follows
/// Unity's fileID order the document takes its fileID position, so the merged
/// file is what Unity itself would write and an Editor save does not reorder
/// it, even when both sides added documents. Otherwise it follows `previous`,
/// its nearest preceding `order` document that the target has, or precedes the
/// target's first document.
fn insertion_point(target: &Asset, ordered: Option<&[(i64, usize)]>, id: &str, previous: Option<usize>) -> usize {
    let end = target.documents.last().map_or(target.bytes.len(), |document| document.span.end);
    if let (Some(ordered), Ok(id)) = (ordered, id.parse::<i64>()) {
        let index = ordered.partition_point(|(existing, _)| *existing < id);
        return ordered.get(index).map_or(end, |(_, start)| *start);
    }
    previous.unwrap_or_else(|| target.documents.first().map_or(target.bytes.len(), |document| document.span.start))
}

/// Insertion offsets of every source document absent from `target`.
/// Additions sharing an offset are ordered by `render`.
fn document_anchors(target: &Asset, source: &Asset) -> HashMap<String, usize> {
    let ordered = file_id_order(target);
    let mut previous = None;
    let mut anchors = HashMap::new();
    for document in &source.documents {
        match target.document(&document.object_id) {
            Some(existing) => previous = Some(existing.span.end),
            None => {
                let at = insertion_point(target, ordered.as_deref(), &document.object_id, previous);
                anchors.insert(document.object_id.clone(), at);
            }
        }
    }
    anchors
}

/// The offset `document_anchors` assigns to one document of `order`.
fn document_anchor(target: &Asset, order: &Asset, id: &str) -> usize {
    let previous = order
        .documents
        .iter()
        .position(|d| d.object_id == id)
        .and_then(|index| {
            order.documents[..index]
                .iter()
                .rev()
                .find_map(|d| target.document(&d.object_id).map(|existing| existing.span.end))
        });
    insertion_point(target, file_id_order(target).as_deref(), id, previous)
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
                    Some(Point::root(&b.root)),
                    Some(Point::root(&t.root)),
                    Some(Point::root(&s.root)),
                    None,
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
        // A stripped document is the proxy of an object inside a prefab
        // instance; Unity derives its fileID from the instance and source
        // object, so identical proxies added on both sides are one object.
        let same_proxy = docs_equal(target, source) && target.is_some_and(|d| d.stripped);
        let status = if base.is_none() && target.is_some() && source.is_some() && !same_proxy {
            ChangeStatus::Conflict
        } else if docs_equal(target, source) {
            ChangeStatus::AlreadyApplied
        } else if docs_equal(target, base) {
            ChangeStatus::Clean
        } else {
            ChangeStatus::Conflict
        };
        let reason = match (base.is_some(), target.is_some(), source.is_some()) {
            (false, true, true) if !same_proxy => "both sides added an object with this fileID; choose one with resolve side='source' or 'target'",
            (true, false, true) => "the destination deleted this object while the source modified it; resolve side='source' to restore it or 'target' to keep it deleted",
            (true, true, false) => "the source deleted this object while the destination modified it; resolve side='source' to delete it or 'target' to keep it",
            (false, _, true) => "the source added this object",
            (true, _, false) => "the source deleted this object",
            _ => "the object's class or stripped state changed; choose the complete object",
        };
        let id_hash = change_id(id, "", base.map(|d| &d.root), source.map(|d| &d.root));
        self.catalog.changes.push(Change { id:id_hash,object_id:id.into(),property_path:String::new(),kind,status,reason:Some(reason.into()),base:base.map(|d|summary(self.base,&d.root)),target:target.map(|d|summary(self.target,&d.root)),source:source.map(|d|summary(self.source,&d.root)) });
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
            insertion: Some((
                self.anchors.get(id).copied().unwrap_or(self.target.bytes.len()),
                0,
            )),
        });
        Ok(())
    }

    /// Identity-keyed three-way merge of block sequences. Each element is an
    /// independent change: additions, deletions and element edits from either
    /// side combine, and only same-element disagreements conflict.
    #[allow(clippy::too_many_arguments)]
    fn keyed_sequence(
        &mut self,
        object_id: &str,
        path: &str,
        (b, bk): (&Node, &[String]),
        (t, tk): (&Node, &[String]),
        (s, sk): (&Node, &[String]),
    ) -> Result<(), CoreError> {
        let (Some(bi), Some(ti), Some(si)) = (b.items(), t.items(), s.items()) else {
            return Ok(());
        };
        let index = |keys: &[String]| -> HashMap<String, usize> {
            keys.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect()
        };
        let (bmap, tmap, smap) = (index(bk), index(tk), index(sk));
        // A managed reference is only used through the fields that point to it;
        // name them so a restore can bring the pointer along.
        let registry = path.ends_with("/references/RefIds");
        let pointers = |asset: &Asset, key: &str| -> String {
            let Some(rid) = key.strip_prefix("@rid=").filter(|_| registry) else {
                return String::new();
            };
            let found = rid_pointers(asset, object_id, rid);
            if found.is_empty() {
                String::new()
            } else {
                format!(" (pointed to by {})", found.join(", "))
            }
        };
        for (position, key) in sk.iter().enumerate() {
            let base = bmap.get(key).map(|&i| Point::in_sequence(&bi[i], b));
            let target = tmap.get(key).map(|&i| Point::in_sequence(&ti[i], t));
            let source = Some(Point::in_sequence(&si[position], s));
            let insertion = target
                .is_none()
                .then(|| sequence_insertion(sk, position, &tmap, ti, t));
            let reason = match (base.is_some(), target.is_some()) {
                (true, false) if registry => Some(format!(
                    "the destination deleted this managed reference while the source modified it; to restore it, resolve side='source' and also take the source fields that point to it{}; 'target' keeps it deleted",
                    pointers(self.source, key)
                )),
                (true, false) => Some("the destination deleted this element while the source modified it; resolve side='source' to restore the source element or 'target' to keep it deleted".into()),
                (false, true) => Some("both sides added this element with different content; resolve side='source' or 'target', or set it explicitly".into()),
                _ => None,
            };
            self.node(object_id, &path_child(path, key), base, target, source, insertion, reason)?;
        }
        for (position, key) in bk.iter().enumerate() {
            if smap.contains_key(key) {
                continue;
            }
            let base = Some(Point::in_sequence(&bi[position], b));
            let target = tmap.get(key).map(|&i| Point::in_sequence(&ti[i], t));
            let reason = if registry {
                format!(
                    "the source deleted this managed reference while the destination modified it{}; resolve side='source' to delete it or 'target' to keep the destination version",
                    pointers(self.target, key)
                )
            } else {
                "the source deleted this element while the destination modified it; resolve side='source' to delete it or 'target' to keep the destination element".into()
            };
            self.node(object_id, &path_child(path, key), base, target, None, None, Some(reason))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        object_id: &str,
        path: &str,
        base: Option<Point<'_>>,
        target: Option<Point<'_>>,
        source: Option<Point<'_>>,
        insertion: Option<(usize, usize)>,
        context_reason: Option<String>,
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
                            None,
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
                                Some(Point::in_sequence(&bi[i], b.node)),
                                Some(Point::in_sequence(&ti[i], t.node)),
                                Some(Point::in_sequence(&si[i], s.node)),
                                None,
                                None,
                            )?;
                        }
                        return Ok(());
                    }
                    let unordered = ListIdentity::of(path).is_some_and(ListIdentity::unordered);
                    let reordered = !unordered && relative_order_changed(&bk, &sk);
                    // Elements are patched into the destination's block list. An
                    // empty base (Unity writes `[]`) only contributes no elements.
                    let block = (!b.node.flow || bi.is_empty()) && !t.node.flow && !s.node.flow;
                    if !identity_collision && !reordered && block {
                        return self.keyed_sequence(
                            object_id,
                            path,
                            (b.node, &bk),
                            (t.node, &tk),
                            (s.node, &sk),
                        );
                    }
                    reason = Some(if identity_collision {
                        "both snapshots independently introduced the same managed-reference rid; explicitly choose or remap the complete identity closure"
                    } else if reordered {
                        "the source reordered identity elements whose order is significant; choose the complete sequence (resolve side='source' or 'target')"
                    } else {
                        "identity sequence membership changed in a flow/empty sequence; choose the complete sequence (resolve side='source' or 'target')"
                    }
                    .into());
                } else {
                    reason = Some(identityless_reason(b.node, t.node, s.node).into());
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
        if reason.is_none() && status == ChangeStatus::Conflict {
            reason = Some(context_reason.unwrap_or_else(|| {
                "the destination and the source changed this value differently; resolve side='source', 'target' or 'base', or set a typed value".into()
            }));
        }
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
    let mut point = Point::root(&doc.root);
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
                    item: None,
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
                point = Point::in_sequence(item, point.node);
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
        item: None,
        parent: Some(parent),
    })
}

/// The next step for a list whose elements carry no Unity identity, by what
/// the two sides did to it.
fn identityless_reason(base: &Node, target: &Node, source: &Node) -> &'static str {
    let prints = |node: &Node| {
        node.items()
            .map(|items| items.iter().map(|item| item.value.fingerprint).collect::<Vec<_>>())
    };
    match (prints(base), prints(target), prints(source)) {
        (Some(b), Some(t), Some(s)) if t.len() > b.len() && s.len() > b.len() && t.starts_with(&b) && s.starts_with(&b) => {
            "both sides appended to a list whose elements have no Unity identity, so the combined order is ambiguous; keep one side with resolve, or fields.set this path to the combined list (the destination's elements, then the source's new ones)"
        }
        (Some(b), Some(t), Some(s)) if b.len() == t.len() && b.len() == s.len() => {
            "both sides edited elements of a list whose elements have no Unity identity; resolve one side, then fields.take the other side's changed element paths (this path followed by /<index>)"
        }
        _ => "the list's elements have no Unity identity, so pairing them by position would be ambiguous; choose a side with resolve, or fields.set the complete list",
    }
}

/// Paths in one object that hold a `{rid: N}` pointer to a managed reference.
fn rid_pointers(asset: &Asset, object_id: &str, rid: &str) -> Vec<String> {
    fn walk(asset: &Asset, node: &Node, path: &str, rid: &str, found: &mut Vec<String>) {
        if found.len() >= 5 {
            return;
        }
        if super::identity::managed_reference(node) {
            if node.get("rid").and_then(|n| decoded_scalar(asset, n)).as_deref() == Some(rid) {
                found.push(path.to_string());
            }
            return;
        }
        match &node.kind {
            NodeKind::Mapping(entries) => {
                for entry in entries {
                    walk(asset, &entry.value, &path_child(path, &entry.key), rid, found);
                }
            }
            NodeKind::Sequence(items) => {
                let keys = sequence_keys(asset, node, path);
                for (index, item) in items.iter().enumerate() {
                    let key = keys.as_ref().map(|k| k[index].clone()).unwrap_or_else(|| index.to_string());
                    walk(asset, &item.value, &path_child(path, &key), rid, found);
                }
            }
            NodeKind::Scalar(_) => {}
        }
    }
    let mut found = Vec::new();
    if let Some(doc) = asset.document(object_id) {
        walk(asset, &doc.root, "", rid, &mut found);
    }
    found
}

/// Whether the elements two snapshots share appear in a different relative order.
pub(super) fn relative_order_changed(a: &[String], b: &[String]) -> bool {
    let in_a: HashSet<&String> = a.iter().collect();
    let in_b: HashSet<&String> = b.iter().collect();
    let left: Vec<&String> = a.iter().filter(|k| in_b.contains(k)).collect();
    let right: Vec<&String> = b.iter().filter(|k| in_a.contains(k)).collect();
    left != right
}

/// Where a chosen element enters a target block sequence: after the nearest
/// preceding element (in the chosen order) that the target also contains,
/// otherwise before the nearest following one, otherwise at the end.
pub(super) fn sequence_insertion(
    chosen_keys: &[String],
    position: usize,
    target_index: &HashMap<String, usize>,
    target_items: &[Item],
    target: &Node,
) -> (usize, usize) {
    let at = chosen_keys[..position]
        .iter()
        .rev()
        .find_map(|key| target_index.get(key))
        .map(|&i| target_items[i].span.end)
        .or_else(|| {
            chosen_keys[position + 1..]
                .iter()
                .find_map(|key| target_index.get(key))
                .map(|&i| target_items[i].span.start)
        })
        .unwrap_or_else(|| target_items.last().map(|i| i.span.end).unwrap_or(target.span.end));
    (at, target.indent)
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

/// Stable element identities. Identity text is decoded, so a quoted and a
/// plain spelling of the same propertyPath/rid/fileID address one element.
pub(crate) fn sequence_keys(asset: &Asset, node: &Node, path: &str) -> Option<Vec<String>> {
    let kind = ListIdentity::of(path)?;
    let items = node.items()?;
    let mut keys = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    let text = |node: Option<&Node>| node.and_then(|n| decoded_scalar(asset, n));
    let file_id = |node: Option<&Node>| node.filter(|n| pptr(n)).and_then(|n| text(n.get("fileID")));
    for item in items {
        let key = match kind {
            ListIdentity::Rid => {
                let rid = text(item.value.get("rid"))?;
                if rid.parse::<i64>().is_err() {
                    return None;
                }
                format!("@rid={rid}")
            }
            ListIdentity::Override => {
                let target = item.value.get("target")?;
                if !pptr(target) {
                    return None;
                }
                let property = text(item.value.get("propertyPath"))?;
                format!(
                    "@override={}:{}",
                    blake3::Hash::from_bytes(identity_fingerprint(asset, target)).to_hex(),
                    property
                )
            }
            ListIdentity::Added => format!("@added={}", file_id(item.value.get("addedObject"))?),
            ListIdentity::Removed => {
                let guid = match item.value.get("guid") {
                    Some(guid) => text(Some(guid))?,
                    None => String::new(),
                };
                format!("@removed={guid}:{}", file_id(Some(&item.value))?)
            }
            ListIdentity::Component => format!("@component={}", file_id(item.value.get("component"))?),
            ListIdentity::Child => format!("@child={}", file_id(Some(&item.value))?),
            ListIdentity::Root => format!("@root={}", file_id(Some(&item.value))?),
        };
        if !seen.insert(key.clone()) {
            return None;
        }
        keys.push(key);
    }
    Some(keys)
}

/// Convert copied bytes to the destination's line ending. Git blobs are LF
/// while Windows working files may be CRLF; mixed endings are never produced.
fn with_newline(bytes: &[u8], newline: &str) -> Vec<u8> {
    let has_crlf = bytes.windows(2).any(|w| w == b"\r\n");
    let has_lf = bytes
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\n' && (i == 0 || bytes[i - 1] != b'\r'));
    if (newline == "\n" && !has_crlf) || (newline == "\r\n" && !has_lf) {
        return bytes.to_vec();
    }
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    if newline == "\n" {
        text.into_bytes()
    } else {
        text.replace('\n', "\r\n").into_bytes()
    }
}

/// Whole-line region bytes from `asset`, placed at `at` in `target`. A region
/// that starts after `- ` on its line (first field of an element, or first
/// element of a compact nested sequence) has no leading indentation of its
/// own; it gains `indent` when placed on a fresh line, and a line-start region
/// loses its indentation when placed after an existing `- `.
fn transplant(
    asset: &Asset,
    region: &Region,
    target: &[u8],
    at: usize,
    indent: usize,
    newline: &str,
) -> Vec<u8> {
    let mut bytes = reindent(region.span.bytes(&asset.bytes), region.indent, indent, newline);
    // Appending at an end without a final newline first writes one.
    let fresh_line = at >= target.len() || starts_line(target, at);
    match (starts_line(&asset.bytes, region.span.start), fresh_line) {
        (false, true) => {
            let mut padded = " ".repeat(indent).into_bytes();
            padded.append(&mut bytes);
            padded
        }
        (true, false) => {
            let spaces = bytes.iter().take_while(|b| **b == b' ').count();
            bytes.drain(..spaces);
            bytes
        }
        _ => bytes,
    }
}

fn reindent(bytes: &[u8], old: usize, new: usize, newline: &str) -> Vec<u8> {
    if old == new {
        return with_newline(bytes, newline);
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

pub(super) fn yaml_key(key: &str) -> String {
    if key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        key.into()
    } else {
        serde_json::to_string(key).expect("string JSON")
    }
}
pub(crate) fn json_yaml(value: &serde_json::Value) -> Result<String, CoreError> {
    render_json(value, None)
}

/// Unity-style rendering for merge writes: unambiguous text stays plain (as
/// Unity writes it) and every other string is double-quoted. `block_context`
/// is true for an entry/element value; nested values are flow context.
pub(crate) fn json_yaml_unity(
    value: &serde_json::Value,
    block_context: bool,
) -> Result<String, CoreError> {
    render_json(value, Some(!block_context))
}

/// `plain_flow`: `None` quotes every string; `Some(flow)` writes safe text plain.
fn render_json(value: &serde_json::Value, plain_flow: Option<bool>) -> Result<String, CoreError> {
    // JSON is a safe subset of flow YAML for Unity's structural fields. Reject
    // numbers outside JSON's finite representation in the caller/JSON decoder.
    let nested = plain_flow.map(|_| true);
    match value {
        serde_json::Value::Null => Ok("null".into()),
        serde_json::Value::Bool(b) => Ok(if *b { "1" } else { "0" }.into()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::String(s) => match plain_flow {
            Some(flow) => Ok(super::identity::new_string(s, flow)),
            None => serde_json::to_string(s)
                .map_err(|e| CoreError::new("typed_value", e.to_string(), 0)),
        },
        serde_json::Value::Array(items) => Ok(format!(
            "[{}]",
            items
                .iter()
                .map(|item| render_json(item, nested))
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
                        } else { render_json(v, nested)? };
                        Ok(format!("{}: {}", yaml_key(k), rendered))
                    })
                    .collect::<Result<Vec<_>, CoreError>>()?
                    .join(", ")
            ))
        }
    }
}
