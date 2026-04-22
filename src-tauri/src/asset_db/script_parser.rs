//! Ref-graph specific layer over the neutral C# parser.
//!
//! Wraps `crate::unity_csharp::parse_cs_script` with the on-disk snapshot
//! capture (mtime/size/content hash) and the indexed-metadata cache that
//! `ref_graph::watcher` and `ref_graph::mod` need to walk inheritance chains
//! across files.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;

use crate::unity_csharp::{parse_cs_script_status, ScriptParseStatus};

use super::scanner;
use super::types::{hash128, Guid};

pub use crate::unity_csharp::{ScriptFieldMeta, ScriptMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptNoMetadataReason {
    /// File contains no top-level declarations of any kind (empty namespace,
    /// `using`-only, fully commented out). Indexed silently.
    Empty,
    /// File contains only `enum` / `delegate` declarations. Unity does not
    /// bind these to a `.cs` file by name, so ref_graph indexes silently.
    OnlyNonClassTypes,
    /// File appears to contain a real class/struct/interface that the
    /// parser tripped on. Worth surfacing as a warning.
    Unparseable,
}

#[derive(Debug, Clone)]
pub struct ScriptFileSnapshot {
    /// `None` when the source file has no parseable top-level type
    /// (entirely commented out, behind `#if false`, only contains enums or
    /// extension stubs, etc.). The watcher still indexes these as scripts —
    /// just with no class-level metadata to put into the search index.
    pub metadata: Option<ScriptMetadata>,
    /// Set when `metadata` is `None`, explaining whether the absence is
    /// design-expected (empty / enum-only) or a real parser failure.
    pub no_metadata_reason: Option<ScriptNoMetadataReason>,
    pub content_hash: [u8; 16],
    pub mtime_ns: u64,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct IndexedScriptMetadata {
    pub class_name: String,
    pub class_name_lower: String,
    pub namespace_lower: String,
    pub full_name_lower: String,
    pub type_search_lower: String,
    pub inheritance_search_lower: String,
    pub inherits_scriptable_object: bool,
    pub content_hash: [u8; 16],
    pub mtime_ns: u64,
    pub size: u64,
}

fn dedupe_type_terms_lower<'a>(terms: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for term in terms {
        let normalized = term.trim().to_ascii_lowercase();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        out.push(normalized);
    }
    out
}

pub(crate) fn normalize_namespace_lower(namespace: Option<&str>) -> String {
    namespace
        .map(|ns| ns.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) fn compose_full_name_lower(namespace_lower: &str, class_name_lower: &str) -> String {
    if namespace_lower.is_empty() {
        class_name_lower.to_string()
    } else {
        format!("{}.{}", namespace_lower, class_name_lower)
    }
}

pub fn read_script_file_snapshot(path: &Path) -> Option<ScriptFileSnapshot> {
    let content = std::fs::read(path).ok()?;
    let content_text = String::from_utf8_lossy(&content);
    let expected_name = path.file_stem().and_then(|stem| stem.to_str());
    let (metadata, no_metadata_reason) = match parse_cs_script_status(&content_text, expected_name)
    {
        ScriptParseStatus::Parsed(meta) => (Some(meta), None),
        ScriptParseStatus::EmptySource => (None, Some(ScriptNoMetadataReason::Empty)),
        ScriptParseStatus::OnlyNonClassTypes => {
            (None, Some(ScriptNoMetadataReason::OnlyNonClassTypes))
        }
        ScriptParseStatus::Unparseable => (None, Some(ScriptNoMetadataReason::Unparseable)),
    };
    let file_meta = std::fs::metadata(path).ok();
    Some(ScriptFileSnapshot {
        metadata,
        no_metadata_reason,
        content_hash: hash128(&content),
        mtime_ns: file_meta.as_ref().map(scanner::get_mtime_ns).unwrap_or(0),
        size: file_meta.map(|m| m.len()).unwrap_or(content.len() as u64),
    })
}

/// Build the indexed metadata used by the search/lookup layer. Returns
/// `None` when the snapshot has no parseable script metadata — the caller
/// should still record the file's hash/mtime/size from the snapshot fields
/// but skip indexing it by class name.
pub fn build_indexed_script_metadata(
    snapshot: &ScriptFileSnapshot,
    inherited_base_search_lower: Option<&str>,
) -> Option<IndexedScriptMetadata> {
    let metadata = snapshot.metadata.as_ref()?;
    let class_name = metadata.class_name.clone();
    let class_name_lower = class_name.to_ascii_lowercase();
    let namespace_lower = normalize_namespace_lower(metadata.namespace.as_deref());
    let full_name_lower = compose_full_name_lower(&namespace_lower, &class_name_lower);
    let mut self_terms = vec![class_name_lower.clone()];
    self_terms.push(full_name_lower.clone());
    let mut inheritance_terms = Vec::new();

    if let Some(base_search) = inherited_base_search_lower {
        inheritance_terms.extend(dedupe_type_terms_lower(base_search.split_whitespace()));
    } else if let Some(base_type) = metadata.base_type.as_deref() {
        inheritance_terms.extend(dedupe_type_terms_lower(std::iter::once(base_type)));
    }

    let type_search_lower = dedupe_type_terms_lower(
        self_terms
            .iter()
            .map(String::as_str)
            .chain(inheritance_terms.iter().map(String::as_str)),
    )
    .join(" ");
    let inheritance_search_lower =
        dedupe_type_terms_lower(inheritance_terms.iter().map(String::as_str)).join(" ");
    let inherits_scriptable_object = type_search_lower
        .split_whitespace()
        .any(|term| term == "scriptableobject");

    Some(IndexedScriptMetadata {
        class_name,
        class_name_lower,
        namespace_lower,
        full_name_lower,
        type_search_lower,
        inheritance_search_lower,
        inherits_scriptable_object,
        content_hash: snapshot.content_hash,
        mtime_ns: snapshot.mtime_ns,
        size: snapshot.size,
    })
}

pub fn build_script_metadata_index(
    project_root: &Path,
    path_to_guid: &HashMap<String, Guid>,
) -> HashMap<Guid, IndexedScriptMetadata> {
    // Phase 1: parallel IO + parse. Reading 4k+ .cs files serially was the
    // dominant cost of a full scan (~16s out of ~20s on a real project);
    // par_iter brings it in line with the meta/yaml phases. The merge step
    // afterwards stays serial because HashMap insertion order matters for
    // `class_to_guid` (first writer wins on duplicate class names — same
    // semantics as the previous serial loop, which was already
    // non-deterministic since `path_to_guid` is itself a HashMap).
    let script_entries: Vec<(&String, &Guid)> = path_to_guid
        .iter()
        .filter(|(rel_path, _)| {
            Path::new(rel_path)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("cs"))
                .unwrap_or(false)
        })
        .collect();

    let raw_snapshots: Vec<(Guid, ScriptFileSnapshot)> = script_entries
        .par_iter()
        .filter_map(|(rel_path, guid)| {
            let abs_path = project_root.join(rel_path.as_str());
            read_script_file_snapshot(&abs_path).map(|snap| (**guid, snap))
        })
        .collect();

    let mut snapshots: HashMap<Guid, ScriptFileSnapshot> =
        HashMap::with_capacity(raw_snapshots.len());
    let mut class_to_guids: HashMap<String, Vec<Guid>> = HashMap::new();
    let mut full_name_to_guids: HashMap<String, Vec<Guid>> = HashMap::new();
    for (guid, snapshot) in raw_snapshots {
        if let Some(metadata) = snapshot.metadata.as_ref() {
            let class_name_lower = metadata.class_name.to_ascii_lowercase();
            class_to_guids
                .entry(class_name_lower.clone())
                .or_default()
                .push(guid);
            let namespace_lower = normalize_namespace_lower(metadata.namespace.as_deref());
            let full_name_lower = compose_full_name_lower(&namespace_lower, &class_name_lower);
            full_name_to_guids
                .entry(full_name_lower)
                .or_default()
                .push(guid);
        }
        snapshots.insert(guid, snapshot);
    }

    fn unique_guid(index: &HashMap<String, Vec<Guid>>, key: &str) -> Option<Guid> {
        let guids = index.get(key)?;
        if guids.len() == 1 {
            Some(guids[0])
        } else {
            None
        }
    }

    fn resolve_base_guid(
        metadata: &ScriptMetadata,
        class_to_guids: &HashMap<String, Vec<Guid>>,
        full_name_to_guids: &HashMap<String, Vec<Guid>>,
    ) -> Option<Guid> {
        let base_type_lower = metadata.base_type.as_deref()?.trim().to_ascii_lowercase();
        if base_type_lower.is_empty() {
            return None;
        }

        let namespace_lower = normalize_namespace_lower(metadata.namespace.as_deref());
        if !namespace_lower.is_empty() {
            let same_namespace_key = compose_full_name_lower(&namespace_lower, &base_type_lower);
            if let Some(guid) = unique_guid(full_name_to_guids, &same_namespace_key) {
                return Some(guid);
            }
        }

        unique_guid(class_to_guids, &base_type_lower)
    }

    fn resolve_for_guid(
        guid: Guid,
        snapshots: &HashMap<Guid, ScriptFileSnapshot>,
        class_to_guids: &HashMap<String, Vec<Guid>>,
        full_name_to_guids: &HashMap<String, Vec<Guid>>,
        cache: &mut HashMap<Guid, IndexedScriptMetadata>,
        visiting: &mut HashSet<Guid>,
    ) -> Option<IndexedScriptMetadata> {
        if let Some(cached) = cache.get(&guid) {
            return Some(cached.clone());
        }

        let snapshot = snapshots.get(&guid)?;
        let metadata = snapshot.metadata.as_ref()?;
        let inherited_base_search = resolve_base_guid(metadata, class_to_guids, full_name_to_guids)
            .and_then(|base_guid| {
                if !visiting.insert(base_guid) {
                    return None;
                }
                let resolved = resolve_for_guid(
                    base_guid,
                    snapshots,
                    class_to_guids,
                    full_name_to_guids,
                    cache,
                    visiting,
                );
                visiting.remove(&base_guid);
                resolved.map(|meta| meta.type_search_lower)
            });

        let indexed = build_indexed_script_metadata(snapshot, inherited_base_search.as_deref())?;
        cache.insert(guid, indexed.clone());
        Some(indexed)
    }

    let mut cache = HashMap::new();
    for guid in snapshots.keys().copied().collect::<Vec<_>>() {
        let mut visiting = HashSet::from([guid]);
        let _ = resolve_for_guid(
            guid,
            &snapshots,
            &class_to_guids,
            &full_name_to_guids,
            &mut cache,
            &mut visiting,
        );
    }

    cache
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_indexed_script_metadata_keeps_base_chain() {
        let snapshot = ScriptFileSnapshot {
            metadata: Some(ScriptMetadata {
                class_name: "FireConfig".to_string(),
                base_type: Some("CombatConfig".to_string()),
                namespace: Some("Game.Combat".to_string()),
                serialized_fields: Vec::new(),
            }),
            no_metadata_reason: None,
            content_hash: [1u8; 16],
            mtime_ns: 10,
            size: 20,
        };

        let indexed = build_indexed_script_metadata(
            &snapshot,
            Some("combatconfig game.combat.combatconfig baseconfig scriptableobject"),
        )
        .expect("snapshot has metadata");

        assert_eq!(indexed.class_name, "FireConfig");
        assert_eq!(indexed.namespace_lower, "game.combat");
        assert_eq!(indexed.full_name_lower, "game.combat.fireconfig");
        assert_eq!(
            indexed.type_search_lower,
            "fireconfig game.combat.fireconfig combatconfig game.combat.combatconfig baseconfig scriptableobject"
        );
        assert_eq!(
            indexed.inheritance_search_lower,
            "combatconfig game.combat.combatconfig baseconfig scriptableobject"
        );
        assert!(indexed.inherits_scriptable_object);
    }

    #[test]
    fn build_indexed_script_metadata_returns_none_when_metadata_missing() {
        let snapshot = ScriptFileSnapshot {
            metadata: None,
            no_metadata_reason: Some(ScriptNoMetadataReason::Empty),
            content_hash: [0u8; 16],
            mtime_ns: 1,
            size: 2,
        };
        assert!(build_indexed_script_metadata(&snapshot, None).is_none());
    }
}
