use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use regex::{Regex, RegexBuilder};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::view::{
    UnityPropertyTreeSubassetEntry, UnitySerializedPropertySnapshot, UnitySerializedPropertyTarget,
};

pub const AGENT_PROPERTY_TREE_DEFAULT_DEPTH: usize = 2;
pub const AGENT_PROPERTY_TREE_MAX_DEPTH: usize = 4;
pub const AGENT_PROPERTY_TREE_ARRAY_LIMIT: usize = 4;
pub const AGENT_PROPERTY_TREE_AUTO_EXPAND_CHAR_LIMIT: usize = 4_000;
pub const AGENT_PROPERTY_TREE_COMPLETE_MAX_DEPTH: usize = 16;
pub const AGENT_PROPERTY_TREE_COMPLETE_MAX_ARRAY_ITEMS: usize = 1_024;
pub const AGENT_PROPERTY_TREE_SUBASSET_PREVIEW_LIMIT: usize = 32;

const UNITY_ASSET_EXTENSIONS: &[&str] = &[
    "overridecontroller",
    "shadergraph",
    "controller",
    "playable",
    "prefab",
    "unity",
    "asset",
    "anim",
    "mask",
    "mat",
];

const ROOT_METADATA_FIELDS: &[&str] = &[
    "serializedVersion",
    "m_ObjectHideFlags",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "m_GameObject",
    "m_Component",
    "m_Children",
    "m_Father",
    "m_EditorHideFlags",
    "m_EditorClassIdentifier",
    "m_Script",
    "m_Name",
    "references",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyTreePath {
    pub asset_path: String,
    pub absolute_asset_path: PathBuf,
    pub segments: Vec<String>,
}

impl PropertyTreePath {
    pub fn parse(working_dir: &str, input: &str) -> Result<Self, String> {
        let normalized = input.trim().replace('\\', "/");
        if normalized.is_empty() {
            return Err("Property Tree path is required".to_string());
        }

        let boundaries = asset_boundaries(&normalized);
        if boundaries.is_empty() {
            return Err(format!(
                "Property Tree path must contain a supported Unity asset extension: {}",
                input
            ));
        }

        let root = PathBuf::from(working_dir.trim());
        let mut chosen: Option<(usize, PathBuf)> = None;
        for boundary in boundaries.iter().copied().rev() {
            let candidate = &normalized[..boundary];
            let absolute = resolve_asset_path(&root, candidate);
            if absolute.is_file() {
                chosen = Some((boundary, absolute));
                break;
            }
        }
        let (boundary, absolute_asset_path) = chosen.unwrap_or_else(|| {
            let boundary = *boundaries
                .last()
                .expect("asset boundaries were checked as non-empty");
            let candidate = &normalized[..boundary];
            (boundary, resolve_asset_path(&root, candidate))
        });

        let raw_asset_path = &normalized[..boundary];
        let asset_path = normalize_asset_display_path(&root, raw_asset_path, &absolute_asset_path);
        let suffix = normalized[boundary..].trim_start_matches('/');
        let segments = if suffix.is_empty() {
            Vec::new()
        } else {
            suffix
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(decode_path_segment)
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(Self {
            asset_path,
            absolute_asset_path,
            segments,
        })
    }

    pub fn full_path(&self) -> String {
        append_segments(&self.asset_path, self.segments.iter().map(String::as_str))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PropertyTreeSearchOptions {
    pub query: String,
    pub match_fields: Vec<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyTreeSearchMatch {
    pub path: String,
    pub name: String,
    pub display_name: String,
    pub property_type: String,
    pub display_value: String,
    pub evidence: PropertyTreeSearchMatchEvidence,
}

#[derive(Debug, Clone, Default)]
pub struct LivePropertyTreeSearchResult {
    pub matches: Vec<PropertyTreeSearchMatch>,
    pub traversal_truncated: bool,
    pub scanned_objects: i32,
    pub scanned_properties: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyTreeSearchMatchEvidence {
    pub path: bool,
    pub field_name: bool,
    pub field_value: bool,
    pub property_type: bool,
}

impl PropertyTreeSearchMatchEvidence {
    fn any(&self) -> bool {
        self.path || self.field_name || self.field_value || self.property_type
    }

    fn has_non_path_match(&self) -> bool {
        self.field_name || self.field_value || self.property_type
    }
}

#[derive(Debug, Clone)]
pub struct CompletePropertyTreeRead {
    pub snapshot: UnitySerializedPropertySnapshot,
    pub output: String,
    pub max_depth: usize,
    pub max_array_items: usize,
}

#[derive(Debug)]
struct CompleteProjectionBudget {
    char_limit: usize,
    chars_used: usize,
    max_depth: usize,
    max_array_items: usize,
}

impl CompleteProjectionBudget {
    fn new(char_limit: usize) -> Self {
        Self {
            char_limit,
            chars_used: 0,
            max_depth: 0,
            max_array_items: 0,
        }
    }

    fn record_node(&mut self, node: &UnitySerializedPropertySnapshot, tree_depth: usize) -> bool {
        let prefix_chars = if tree_depth == 0 {
            0
        } else {
            (tree_depth - 1).saturating_mul(3).saturating_add(3)
        };
        let line_chars = prefix_chars
            .saturating_add(format_node(node, tree_depth == 0).chars().count())
            .saturating_add(1);
        if self.chars_used.saturating_add(line_chars) > self.char_limit {
            return false;
        }

        self.chars_used += line_chars;
        self.max_depth = self.max_depth.max(tree_depth);
        if node.is_array {
            self.max_array_items = self
                .max_array_items
                .max(AGENT_PROPERTY_TREE_ARRAY_LIMIT.min(node.array_size.max(0) as usize));
        }
        true
    }
}

#[derive(Debug, Clone, Default)]
struct ScriptFieldSchema {
    name: String,
    field_type: String,
    former_names: Vec<String>,
    serialize_reference: bool,
}

#[derive(Debug, Clone, Default)]
struct ScriptSchema {
    type_name: String,
    type_full_name: String,
    fields: Vec<ScriptFieldSchema>,
}

#[derive(Debug, Clone)]
struct DocumentDescriptor {
    file_id: i64,
    display_name: String,
    type_name: String,
    type_full_name: String,
    schema: Option<ScriptSchema>,
}

#[derive(Debug, Clone)]
struct YamlPropertyDocument {
    root: UnitySerializedPropertySnapshot,
    managed_references: HashMap<i64, UnitySerializedPropertySnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PropertyInstanceId {
    UnityObject(i64),
    ManagedReference {
        owner_file_id: i64,
        reference_id: i64,
    },
}

/// Disk-backed implementation of the same Snapshot protocol used by the
/// live LocusBridge PropertyTree endpoint.  Documents remain addressable by
/// local file id internally; callers only see stable semantic paths.
#[derive(Debug, Clone)]
pub struct YamlPropertyTree {
    asset_path: String,
    root_owner_file_id: i64,
    root: UnitySerializedPropertySnapshot,
    documents: HashMap<i64, YamlPropertyDocument>,
    canonical_paths: HashMap<PropertyInstanceId, String>,
}

#[derive(Clone)]
struct CanonicalSeedCacheEntry {
    modified: Option<SystemTime>,
    len: u64,
    tree: Option<Arc<YamlPropertyTree>>,
}

fn canonical_seed_cache() -> &'static Mutex<HashMap<PathBuf, CanonicalSeedCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CanonicalSeedCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn cached_yaml_property_tree(path: &Path) -> Option<Arc<YamlPropertyTree>> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok();
    let len = metadata.len();
    let cache = canonical_seed_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.get(path).and_then(|entry| {
        (entry.modified == modified && entry.len == len)
            .then(|| entry.tree.clone())
            .flatten()
    })
}

pub(crate) fn cache_yaml_property_tree(path: &Path, tree: Arc<YamlPropertyTree>) {
    const MAX_CACHED_PROPERTY_TREES: usize = 8;
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    let modified = metadata.modified().ok();
    let len = metadata.len();
    let mut cache = canonical_seed_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= 64 && !cache.contains_key(path) {
        cache.clear();
    }
    let replacing_cached_tree = cache.get(path).is_some_and(|entry| entry.tree.is_some());
    if !replacing_cached_tree
        && cache.values().filter(|entry| entry.tree.is_some()).count() >= MAX_CACHED_PROPERTY_TREES
    {
        if let Some(entry) = cache.values_mut().find(|entry| entry.tree.is_some()) {
            entry.tree = None;
        }
    }
    cache.insert(
        path.to_path_buf(),
        CanonicalSeedCacheEntry {
            modified,
            len,
            tree: Some(tree),
        },
    );
}

/// Read the same Property Tree from the connected Unity Editor.  This keeps
/// unsaved values visible and uses the exact Snapshot/Target contract that
/// powers the frontend inspector and write/apply operations.
pub async fn read_live_property_tree(
    working_dir: &str,
    path: &PropertyTreePath,
    requested_depth: usize,
) -> Result<UnitySerializedPropertySnapshot, String> {
    read_live_property_tree_with_limits(
        working_dir,
        path,
        requested_depth.min(AGENT_PROPERTY_TREE_MAX_DEPTH),
        AGENT_PROPERTY_TREE_ARRAY_LIMIT,
    )
    .await
}

pub async fn read_live_property_tree_with_limits(
    working_dir: &str,
    path: &PropertyTreePath,
    requested_depth: usize,
    requested_array_items: usize,
) -> Result<UnitySerializedPropertySnapshot, String> {
    // Live reads intentionally start from Editor state alone.  Reading the
    // YAML file here would make an available Unity connection observe stale
    // saved values before it ever asks the Editor for unsaved state.
    let canonical_seed = HashMap::new();
    read_live_property_tree_with_limits_and_canonical_seed(
        working_dir,
        path,
        requested_depth,
        requested_array_items,
        &canonical_seed,
    )
    .await
}

/// Search the shared Property Tree from the connected Editor. Scene roots use
/// the bridge's filtered hierarchy discovery so a large scene does not need to
/// cross the pipe in full; other targets search a bounded live projection.
pub async fn search_live_property_tree(
    working_dir: &str,
    path: &PropertyTreePath,
    options: &PropertyTreeSearchOptions,
) -> Result<LivePropertyTreeSearchResult, String> {
    // Validate regex syntax consistently before entering either live path.
    SearchMatcher::new(&options.query)?;
    let requested_limit = options.limit.clamp(1, 1000);
    let fetch_limit = requested_limit.saturating_add(1);
    let lower = path.asset_path.to_ascii_lowercase();
    if lower.ends_with(".unity") && path.segments.is_empty() {
        let result = super::discover(
            working_dir,
            super::UnitySerializedPropertyDiscoverRequest {
                binding_id: None,
                target: UnitySerializedPropertyTarget {
                    kind: "asset".to_string(),
                    path: Some(path.asset_path.clone()),
                    ..Default::default()
                },
                query: Some(options.query.clone()),
                field_name: None,
                field_type: None,
                match_fields: Some(options.match_fields.clone()),
                max_depth: Some(32),
                max_results: Some(fetch_limit as i32),
                include_all: Some(false),
                shallow_path_matches: Some(true),
            },
        )
        .await?;
        let traversal_truncated = result.truncated;
        let scanned_objects = result.scanned_objects;
        let scanned_properties = result.scanned_properties;
        let matches = result
            .matches
            .into_iter()
            .map(|item| PropertyTreeSearchMatch {
                path: if item.semantic_path.is_empty() {
                    path.asset_path.clone()
                } else {
                    item.semantic_path
                },
                name: item.name,
                display_name: item.display_name,
                property_type: if item.field_type_full_name.is_empty() {
                    item.property_type
                } else {
                    item.field_type_full_name
                },
                display_value: item.display_value,
                evidence: PropertyTreeSearchMatchEvidence {
                    path: item.matched_path,
                    field_name: item.matched_field_name,
                    field_value: item.matched_field_value,
                    property_type: item.matched_type,
                },
            })
            .collect();
        return Ok(LivePropertyTreeSearchResult {
            matches: normalize_search_matches(matches, options)?,
            traversal_truncated,
            scanned_objects,
            scanned_properties,
        });
    }

    let scope_snapshot =
        read_live_property_tree_with_limits(working_dir, path, 1, AGENT_PROPERTY_TREE_ARRAY_LIMIT)
            .await?;
    let target = scope_snapshot.binding_target.clone().ok_or_else(|| {
        format!(
            "Property Tree scope '{}' has no live target",
            path.full_path()
        )
    })?;
    let scope_property_path = target.property_path.clone().unwrap_or_default();
    let result = super::discover(
        working_dir,
        super::UnitySerializedPropertyDiscoverRequest {
            binding_id: None,
            target,
            query: Some(options.query.clone()),
            field_name: None,
            field_type: None,
            match_fields: Some(options.match_fields.clone()),
            max_depth: Some(32),
            max_results: Some(fetch_limit.saturating_add(64).min(1001) as i32),
            include_all: Some(false),
            shallow_path_matches: Some(true),
        },
    )
    .await?;

    let direct_names = scope_snapshot
        .children
        .iter()
        .filter_map(|child| {
            let relative =
                relative_serialized_property_segments(&scope_property_path, &child.property_path)?;
            (relative.len() == 1).then(|| (relative[0].clone(), child.name.clone()))
        })
        .collect::<HashMap<_, _>>();
    let traversal_truncated = result.truncated;
    let scanned_objects = result.scanned_objects;
    let scanned_properties = result.scanned_properties;
    let matches = result
        .matches
        .into_iter()
        .filter_map(|item| {
            if !item.semantic_path.trim().is_empty() {
                return Some(PropertyTreeSearchMatch {
                    path: item.semantic_path,
                    name: item.name,
                    display_name: item.display_name,
                    property_type: if item.field_type_full_name.is_empty() {
                        item.property_type
                    } else {
                        item.field_type_full_name
                    },
                    display_value: item.display_value,
                    evidence: PropertyTreeSearchMatchEvidence {
                        path: item.matched_path,
                        field_name: item.matched_field_name,
                        field_value: item.matched_field_value,
                        property_type: item.matched_type,
                    },
                });
            }
            let mut relative =
                relative_serialized_property_segments(&scope_property_path, &item.property_path)?;
            if relative.is_empty() {
                return None;
            }
            if let Some(name) = direct_names.get(&relative[0]) {
                relative[0] = name.clone();
            }
            Some(PropertyTreeSearchMatch {
                path: append_segments(&path.full_path(), relative.iter().map(String::as_str)),
                name: item.name,
                display_name: item.display_name,
                property_type: if item.field_type_full_name.is_empty() {
                    item.property_type
                } else {
                    item.field_type_full_name
                },
                display_value: item.display_value,
                evidence: PropertyTreeSearchMatchEvidence {
                    path: item.matched_path,
                    field_name: item.matched_field_name,
                    field_value: item.matched_field_value,
                    property_type: item.matched_type,
                },
            })
        })
        .collect();
    Ok(LivePropertyTreeSearchResult {
        matches: normalize_search_matches(matches, options)?,
        traversal_truncated,
        scanned_objects,
        scanned_properties,
    })
}

fn normalize_search_matches(
    matches: Vec<PropertyTreeSearchMatch>,
    options: &PropertyTreeSearchOptions,
) -> Result<Vec<PropertyTreeSearchMatch>, String> {
    let matcher = SearchMatcher::new(&options.query)?;
    let fields = normalize_match_fields(&options.match_fields);
    let mut path_roots = Vec::<String>::new();
    let mut normalized = Vec::with_capacity(matches.len());
    for mut item in matches {
        if !item.evidence.any() {
            item.evidence = PropertyTreeSearchMatchEvidence {
                path: fields.contains("path") && matcher.is_match(&item.path),
                field_name: fields.contains("field_name")
                    && (matcher.is_match(&item.name) || matcher.is_match(&item.display_name)),
                field_value: fields.contains("field_value")
                    && matcher.is_match(&item.display_value),
                property_type: fields.contains("type") && matcher.is_match(&item.property_type),
            };
        }
        if item.evidence.path {
            let inherited = path_roots
                .iter()
                .any(|root| semantic_path_is_descendant_of(&item.path, root));
            if inherited {
                item.evidence.path = false;
            } else {
                path_roots.push(item.path.clone());
            }
        }
        if item.evidence.any() {
            normalized.push(item);
        }
    }
    Ok(normalized)
}

fn semantic_path_is_descendant_of(path: &str, ancestor: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes().get(ancestor.len()) == Some(&b'/')
}

pub fn search_property_tree_snapshot(
    snapshot: &UnitySerializedPropertySnapshot,
    scope: &str,
    options: &PropertyTreeSearchOptions,
) -> Result<Vec<PropertyTreeSearchMatch>, String> {
    fn visit(
        node: &UnitySerializedPropertySnapshot,
        path: &str,
        is_scope_root: bool,
        matcher: &SearchMatcher,
        fields: &HashSet<String>,
        limit: usize,
        path_ancestor_matched: bool,
        matches: &mut Vec<PropertyTreeSearchMatch>,
    ) {
        if matches.len() >= limit {
            return;
        }
        let mut evidence = if is_scope_root {
            PropertyTreeSearchMatchEvidence::default()
        } else {
            property_match_evidence(node, path, matcher, fields)
        };
        if path_ancestor_matched {
            evidence.path = false;
        }
        let descendant_path_matched = path_ancestor_matched || evidence.path;
        if evidence.any() {
            matches.push(PropertyTreeSearchMatch {
                path: path.to_string(),
                name: node.name.clone(),
                display_name: node.display_name.clone(),
                property_type: display_type(node),
                display_value: property_search_display_value(node),
                evidence: evidence.clone(),
            });
        }
        if descendant_path_matched && !evidence.has_non_path_match() && fields.len() == 1 {
            return;
        }
        for child in &node.children {
            if matches.len() >= limit {
                return;
            }
            let child_path = if child.semantic_path.is_empty() {
                append_path_segment(path, &child.name)
            } else {
                child.semantic_path.clone()
            };
            visit(
                child,
                &child_path,
                false,
                matcher,
                fields,
                limit,
                descendant_path_matched,
                matches,
            );
        }
    }

    let matcher = SearchMatcher::new(&options.query)?;
    let fields = normalize_match_fields(&options.match_fields);
    let limit = options.limit.clamp(1, 1000).saturating_add(1);
    let mut matches = Vec::new();
    visit(
        snapshot,
        scope,
        true,
        &matcher,
        &fields,
        limit,
        false,
        &mut matches,
    );
    Ok(matches)
}

pub async fn read_live_property_tree_with_limits_and_canonical_seed(
    working_dir: &str,
    path: &PropertyTreePath,
    requested_depth: usize,
    requested_array_items: usize,
    canonical_seed: &HashMap<String, String>,
) -> Result<UnitySerializedPropertySnapshot, String> {
    let depth = requested_depth.min(AGENT_PROPERTY_TREE_COMPLETE_MAX_DEPTH);
    let array_limit = requested_array_items
        .max(1)
        .min(AGENT_PROPERTY_TREE_COMPLETE_MAX_ARRAY_ITEMS);
    let root_target = UnitySerializedPropertyTarget {
        kind: "asset".to_string(),
        path: Some(path.asset_path.clone()),
        ..Default::default()
    };
    let asset_result = read_live_property_tree_from_target(
        working_dir,
        path,
        depth,
        array_limit,
        root_target,
        0,
        canonical_seed,
    )
    .await;
    if asset_result.is_ok() || path.segments.is_empty() || !is_hierarchical_asset(path) {
        return asset_result;
    }

    // A SceneAsset cannot expose its loaded GameObjects through SerializedObject,
    // and a prefab asset root does not expose nested GameObjects as serialized
    // children. Resolve the longest leading suffix as a hierarchy path, then
    // continue through the same component/property snapshots.
    let asset_error = asset_result.expect_err("checked error above");
    for consumed_segments in (1..=path.segments.len()).rev() {
        let object_path = path.segments[..consumed_segments].join("/");
        let root_target = UnitySerializedPropertyTarget {
            kind: "gameobject".to_string(),
            path: Some(path.asset_path.clone()),
            scene_path: path
                .asset_path
                .to_ascii_lowercase()
                .ends_with(".unity")
                .then(|| path.asset_path.clone()),
            object_path: Some(object_path),
            ..Default::default()
        };
        if let Ok(snapshot) = read_live_property_tree_from_target(
            working_dir,
            path,
            depth,
            array_limit,
            root_target,
            consumed_segments,
            canonical_seed,
        )
        .await
        {
            return Ok(snapshot);
        }
    }
    Err(asset_error)
}

fn is_hierarchical_asset(path: &PropertyTreePath) -> bool {
    let lower = path.asset_path.to_ascii_lowercase();
    lower.ends_with(".unity") || lower.ends_with(".prefab")
}

async fn read_live_property_tree_from_target(
    working_dir: &str,
    path: &PropertyTreePath,
    depth: usize,
    array_limit: usize,
    root_target: UnitySerializedPropertyTarget,
    consumed_segments: usize,
    canonical_seed: &HashMap<String, String>,
) -> Result<UnitySerializedPropertySnapshot, String> {
    let initial_depth = if path.segments.len() == consumed_segments {
        depth
    } else {
        1
    };
    let mut node =
        read_live_target(working_dir, root_target.clone(), initial_depth, array_limit).await?;
    let mut active_target = root_target;
    let mut active_subassets = node.subassets.clone();
    let mut current_path = append_segments(
        &path.asset_path,
        path.segments[..consumed_segments]
            .iter()
            .map(String::as_str),
    );
    let mut canonical = canonical_seed.clone();
    if let Some(identity) = snapshot_identity(&node).or_else(|| target_identity(&active_target)) {
        canonical.insert(identity, current_path.clone());
    }

    for segment in &path.segments[consumed_segments..] {
        if let Some(reference_target) = node.reference_target.clone() {
            if let Some(identity) = target_identity(&reference_target) {
                canonical.entry(identity).or_insert(current_path.clone());
            }
            active_target = reference_target;
            node = read_live_target(working_dir, active_target.clone(), 1, array_limit).await?;
            active_subassets = node.subassets.clone();
        } else if node.children.is_empty() && node.has_children {
            if let Some(target) = node.binding_target.clone() {
                active_target = target;
                node = read_live_target(working_dir, active_target.clone(), 1, array_limit).await?;
                active_subassets = node.subassets.clone();
            }
        }

        if let Some(entry) = active_subassets
            .iter()
            .find(|entry| entry.segment == *segment)
            .cloned()
        {
            active_subassets = entry.children.clone();
            active_target = entry.target;
            current_path = append_path_segment(&current_path, segment);
            node = read_live_target(working_dir, active_target.clone(), 1, array_limit).await?;
            node.subassets = active_subassets.clone();
            if let Some(identity) =
                snapshot_identity(&node).or_else(|| target_identity(&active_target))
            {
                canonical.entry(identity).or_insert(current_path.clone());
            }
            continue;
        }

        if node.is_array {
            if let Ok(index) = segment.parse::<usize>() {
                let mut target = node.binding_target.clone().ok_or_else(|| {
                    format!("Array '{}' has no Property Tree target", current_path)
                })?;
                target.property_path =
                    Some(array_element_property_path(&node.property_path, index));
                active_target = target;
                node = read_live_target(working_dir, active_target.clone(), 1, array_limit).await?;
                active_subassets.clear();
                current_path = append_path_segment(&current_path, segment);
                continue;
            }
        }

        let mut names = live_child_names(&node);
        names.extend(active_subassets.iter().map(|entry| entry.segment.clone()));
        let child_index = names
            .iter()
            .position(|name| name == segment)
            .or_else(|| {
                node.children.iter().position(|child| {
                    child.name == *segment
                        || child.display_name == *segment
                        || property_leaf_name(&child.property_path) == segment
                })
            })
            .ok_or_else(|| {
                format!(
                    "Property '{}' was not found below '{}'. Available children: {}",
                    encode_path_segment(segment),
                    current_path,
                    if names.is_empty() {
                        "<none>".to_string()
                    } else {
                        names
                            .iter()
                            .take(12)
                            .map(|name| encode_path_segment(name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
            })?;
        node = node.children[child_index].clone();
        if let Some(target) = node.binding_target.clone() {
            active_target = target;
        }
        active_subassets.clear();
        current_path = append_path_segment(&current_path, segment);
    }

    if let Some(target) = node.binding_target.clone() {
        active_target = target;
        node = read_live_target(working_dir, active_target.clone(), depth, array_limit).await?;
        if active_target
            .property_path
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            node.subassets = active_subassets;
        }
    } else if path.segments.len() == consumed_segments && initial_depth != depth {
        node = read_live_target(working_dir, active_target.clone(), depth, array_limit).await?;
        node.subassets = active_subassets;
    }

    project_live_node(
        working_dir,
        node,
        path.full_path(),
        depth,
        array_limit,
        &mut canonical,
    )
    .await
}

async fn read_live_target(
    working_dir: &str,
    target: UnitySerializedPropertyTarget,
    depth: usize,
    array_limit: usize,
) -> Result<UnitySerializedPropertySnapshot, String> {
    let result = super::read(
        working_dir,
        super::UnitySerializedPropertyReadRequest {
            binding_id: None,
            target,
            max_depth: Some(depth.min(AGENT_PROPERTY_TREE_COMPLETE_MAX_DEPTH) as i32),
            max_array_items: Some(
                array_limit
                    .max(1)
                    .min(AGENT_PROPERTY_TREE_COMPLETE_MAX_ARRAY_ITEMS) as i32,
            ),
            auto_expand_char_limit: Some(AGENT_PROPERTY_TREE_AUTO_EXPAND_CHAR_LIMIT as i32),
        },
    )
    .await?;
    if result.property.node_kind.trim().is_empty() {
        return Err(
            "Connected Unity plugin predates the shared Property Tree snapshot protocol"
                .to_string(),
        );
    }
    Ok(result.property)
}

fn project_live_node<'a>(
    working_dir: &'a str,
    source: UnitySerializedPropertySnapshot,
    semantic_path: String,
    depth: usize,
    array_limit: usize,
    canonical: &'a mut HashMap<String, String>,
) -> Pin<Box<dyn Future<Output = Result<UnitySerializedPropertySnapshot, String>> + Send + 'a>> {
    Box::pin(async move {
        let mut projected = source;
        projected.semantic_path = semantic_path.clone();
        projected.canonical_path.clear();

        let mut content_children = projected.children.clone();
        let mut content_has_children = projected.has_children;
        let mut content_is_array = projected.is_array;
        let mut content_array_size = projected.array_size;
        let mut content_is_object_root = projected.property_path.is_empty();
        if content_is_object_root {
            if let Some(component_target) = projected
                .binding_target
                .as_ref()
                .filter(|target| target.kind.eq_ignore_ascii_case("component"))
            {
                if let Some(identity) = target_identity(component_target) {
                    if let Some(first_path) = canonical.get(&identity) {
                        if first_path != &semantic_path {
                            projected.node_kind = "reference".to_string();
                            projected.canonical_path = first_path.clone();
                            projected.children.clear();
                            projected.visible_child_count = 0;
                            projected.children_truncated = false;
                            projected.has_children = false;
                            return Ok(projected);
                        }
                    }
                    canonical.entry(identity).or_insert(semantic_path.clone());
                }
            }
        }
        if projected.is_managed_reference && projected.managed_reference_id > 0 {
            let owner = projected
                .binding_target
                .as_ref()
                .and_then(target_identity)
                .unwrap_or_else(|| "selection#0".to_string());
            let identity = format!("{}#managed:{}", owner, projected.managed_reference_id);
            if let Some(first_path) = canonical.get(&identity) {
                if first_path != &semantic_path {
                    projected.canonical_path = first_path.clone();
                    projected.children.clear();
                    projected.visible_child_count = 0;
                    projected.children_truncated = false;
                    projected.has_children = false;
                    return Ok(projected);
                }
            }
            canonical.insert(identity, semantic_path.clone());
            projected.node_kind = "reference".to_string();
        }
        if let Some(reference_target) = projected.reference_target.clone() {
            if let Some(identity) = target_identity(&reference_target) {
                if let Some(first_path) = canonical.get(&identity) {
                    if first_path != &semantic_path {
                        projected.canonical_path = first_path.clone();
                        projected.children.clear();
                        projected.visible_child_count = 0;
                        projected.children_truncated = false;
                        projected.has_children = false;
                        return Ok(projected);
                    }
                }
                canonical.insert(identity, semantic_path.clone());
            }

            if depth > 0 {
                let referenced =
                    read_live_target(working_dir, reference_target, depth, array_limit).await?;
                projected.node_kind = "reference".to_string();
                projected.property_type = referenced.property_type.clone();
                projected.value_type = referenced.value_type.clone();
                projected.field_type_full_name = referenced.field_type_full_name.clone();
                projected.field_type_assembly = referenced.field_type_assembly.clone();
                projected.subassets = referenced.subassets;
                projected.display_sections = referenced.display_sections;
                content_children = referenced.children;
                content_has_children = referenced.has_children;
                content_is_array = referenced.is_array;
                content_array_size = referenced.array_size;
                content_is_object_root = referenced.property_path.is_empty();
            }
        }

        // Unity exposes vectors, quaternions, colors, bounds, curves, and
        // gradients as properties with visible implementation children.  The
        // shared Property Tree treats those values as semantic atoms: their
        // snapshot already carries the complete editable value and a compact
        // display string, while expanding x/y/z or keyframe internals only
        // adds latency and makes them look like arrays to the agent.
        if is_compact_unity_value(&projected) {
            content_children.clear();
            content_has_children = false;
            content_is_array = false;
            content_array_size = -1;
            content_is_object_root = false;
            projected.children_truncated = false;
        }

        projected.has_children = content_has_children || !content_children.is_empty();
        projected.is_array = content_is_array;
        projected.array_size = content_array_size;
        projected.children.clear();
        if depth == 0 {
            projected.children_truncated = projected.has_children;
            projected.visible_child_count = 0;
            return Ok(projected);
        }

        content_children
            .retain(|child| !(content_is_object_root && is_root_metadata_name(&child.name)));
        let names = live_child_names_from_children(&projected, &content_children);
        let total = content_children.len();
        let limit = if projected.is_array {
            total.min(array_limit)
        } else {
            total
        };
        for (index, mut child) in content_children.into_iter().take(limit).enumerate() {
            let name = names
                .get(index)
                .cloned()
                .unwrap_or_else(|| child.name.clone());
            child.name = name.clone();
            let child_path = append_path_segment(&semantic_path, &name);
            projected.children.push(
                project_live_node(
                    working_dir,
                    child,
                    child_path,
                    depth - 1,
                    array_limit,
                    canonical,
                )
                .await?,
            );
        }
        projected.visible_child_count = projected.children.len() as i32;
        projected.children_truncated = projected.children_truncated || limit < total;
        Ok(projected)
    })
}

fn live_child_names(parent: &UnitySerializedPropertySnapshot) -> Vec<String> {
    live_child_names_from_children(parent, &parent.children)
}

fn live_child_names_from_children(
    parent: &UnitySerializedPropertySnapshot,
    children: &[UnitySerializedPropertySnapshot],
) -> Vec<String> {
    let bases = children
        .iter()
        .map(|child| {
            if parent.is_array {
                return property_leaf_name(&child.property_path).to_string();
            }
            if !child.name.is_empty() {
                return child.name.clone();
            }
            if !child.property_path.is_empty() {
                return property_leaf_name(&child.property_path).to_string();
            }
            if !child.display_name.is_empty() {
                return child.display_name.clone();
            }
            if child
                .binding_target
                .as_ref()
                .is_some_and(|target| target.kind.eq_ignore_ascii_case("gameobject"))
            {
                return "GameObject".to_string();
            }
            child
                .binding_target
                .as_ref()
                .and_then(|target| target.component_type.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(short_type_name)
                .unwrap_or_else(|| "Property".to_string())
        })
        .collect::<Vec<_>>();
    let mut totals = HashMap::<String, usize>::new();
    for base in &bases {
        *totals.entry(base.to_ascii_lowercase()).or_default() += 1;
    }
    let mut seen = HashMap::<String, usize>::new();
    bases
        .into_iter()
        .map(|base| {
            let key = base.to_ascii_lowercase();
            if totals.get(&key).copied().unwrap_or_default() <= 1 {
                return base;
            }
            let ordinal = seen.entry(key).or_default();
            *ordinal += 1;
            if *ordinal == 1 {
                base
            } else {
                format!("{}[{}]", base, ordinal)
            }
        })
        .collect()
}

fn snapshot_identity(snapshot: &UnitySerializedPropertySnapshot) -> Option<String> {
    snapshot.binding_target.as_ref().and_then(target_identity)
}

fn target_identity(target: &UnitySerializedPropertyTarget) -> Option<String> {
    let path = target
        .path
        .as_deref()
        .or(target.scene_path.as_deref())
        .unwrap_or_default()
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let file_id = target.target_file_id.unwrap_or_default();
    if path.is_empty() && file_id == 0 {
        return None;
    }
    Some(format!("{}#{}", path, file_id))
}

impl YamlPropertyTree {
    pub fn parse(
        asset_path: &str,
        text: &str,
        project_root: Option<&Path>,
        guid_paths: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let docs = crate::unity_yaml::parse_yaml_docs_str(text);
        if docs.is_empty() {
            return Err(format!("No Unity YAML documents found in '{}'", asset_path));
        }
        let lines: Vec<&str> = text.lines().collect();

        let mut descriptors = HashMap::new();
        for doc in &docs {
            let schema = load_script_schema(doc, project_root, guid_paths);
            let type_name = schema
                .as_ref()
                .map(|schema| schema.type_name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| doc.type_name.clone());
            let type_full_name = schema
                .as_ref()
                .map(|schema| schema.type_full_name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| type_name.clone());
            let display_name = doc
                .m_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| type_name.clone());
            descriptors.insert(
                doc.file_id,
                DocumentDescriptor {
                    file_id: doc.file_id,
                    display_name,
                    type_name,
                    type_full_name,
                    schema,
                },
            );
        }

        let mut documents = HashMap::new();
        for doc in &docs {
            let start = (doc.line_start + 1).min(lines.len());
            let end = doc.line_end.min(lines.len());
            let body = lines[start..end].join("\n");
            let parsed: YamlValue = serde_yaml::from_str(&body).map_err(|error| {
                format!(
                    "Failed to parse Unity YAML document {} in '{}': {}",
                    doc.doc_index, asset_path, error
                )
            })?;
            let descriptor = descriptors
                .get(&doc.file_id)
                .expect("descriptor is built for every YAML document");
            let value = unwrap_document_value(&parsed, &doc.type_name);
            let target = document_target(asset_path, descriptor, "");
            let mut root = UnitySerializedPropertySnapshot {
                property_path: String::new(),
                node_kind: "object".to_string(),
                binding_target: Some(target),
                display_name: descriptor.display_name.clone(),
                name: descriptor.display_name.clone(),
                property_type: "Object".to_string(),
                value_type: "Object".to_string(),
                field_type_full_name: descriptor.type_full_name.clone(),
                value: serde_json::Value::String(descriptor.display_name.clone()),
                display_value: descriptor.display_name.clone(),
                editable: false,
                array_size: -1,
                ..Default::default()
            };
            root.children = build_children(
                value,
                "",
                descriptor.schema.as_ref(),
                &target_without_property(asset_path, descriptor),
                &descriptors,
                guid_paths,
                0,
            );
            root.visible_child_count = root.children.len() as i32;
            root.has_children = !root.children.is_empty();
            let managed_references = extract_managed_reference_registry(&root);
            documents.insert(
                doc.file_id,
                YamlPropertyDocument {
                    root,
                    managed_references,
                },
            );
        }

        let referenced = collect_internal_document_references(&documents);
        let main_file_id = docs
            .iter()
            .find(|doc| doc.class_id == 114 && doc.file_id == 11_400_000)
            .or_else(|| docs.iter().find(|doc| !referenced.contains(&doc.file_id)))
            .or_else(|| docs.first())
            .map(|doc| doc.file_id)
            .unwrap_or_default();

        let hierarchy_root = build_hierarchy_property_tree_root(asset_path, &docs, &descriptors);
        let (root_owner_file_id, root) = if let Some(root) = hierarchy_root {
            (0, root)
        } else {
            let root = build_asset_property_tree_root(
                asset_path,
                main_file_id,
                &docs,
                &descriptors,
                &documents,
            )?;
            (main_file_id, root)
        };

        let mut tree = Self {
            asset_path: asset_path.trim_end_matches('/').to_string(),
            root_owner_file_id,
            root,
            documents,
            canonical_paths: HashMap::new(),
        };
        tree.canonical_paths = tree.compute_canonical_paths();
        Ok(tree)
    }

    pub fn read(
        &self,
        path: &PropertyTreePath,
        requested_depth: usize,
    ) -> Result<UnitySerializedPropertySnapshot, String> {
        if !self.asset_path.eq_ignore_ascii_case(&path.asset_path) {
            return Err(format!(
                "Property Tree asset mismatch: parsed '{}', requested '{}'",
                self.asset_path, path.asset_path
            ));
        }
        let depth = requested_depth.min(AGENT_PROPERTY_TREE_MAX_DEPTH);
        let cursor = self.resolve(path)?;
        let scoped_subassets = cursor.subassets;
        let mut canonical = cursor.canonical;
        if cursor.owner_file_id != 0 {
            canonical
                .entry(PropertyInstanceId::UnityObject(cursor.owner_file_id))
                .or_insert_with(|| cursor.owner_path.clone());
        }
        let mut snapshot = self.project_node(
            cursor.node,
            cursor.owner_file_id,
            &path.full_path(),
            depth,
            &mut canonical,
        );
        snapshot.subassets = scoped_subassets.to_vec();
        Ok(snapshot)
    }

    pub fn read_complete_within_budget(
        &self,
        path: &PropertyTreePath,
        char_limit: usize,
    ) -> Result<Option<CompletePropertyTreeRead>, String> {
        if char_limit == 0 {
            return Ok(None);
        }
        if !self.asset_path.eq_ignore_ascii_case(&path.asset_path) {
            return Err(format!(
                "Property Tree asset mismatch: parsed '{}', requested '{}'",
                self.asset_path, path.asset_path
            ));
        }

        let cursor = self.resolve(path)?;
        let scoped_subassets = cursor.subassets;
        let mut canonical = cursor.canonical;
        if cursor.owner_file_id != 0 {
            canonical
                .entry(PropertyInstanceId::UnityObject(cursor.owner_file_id))
                .or_insert_with(|| cursor.owner_path.clone());
        }
        let mut budget = CompleteProjectionBudget::new(char_limit);
        let Some(mut snapshot) = self.project_complete_node(
            cursor.node,
            cursor.owner_file_id,
            &path.full_path(),
            0,
            &mut canonical,
            &mut budget,
        ) else {
            return Ok(None);
        };
        snapshot.subassets = scoped_subassets.to_vec();
        let output = format_property_tree(&snapshot);
        if output.chars().count() > char_limit {
            return Ok(None);
        }
        Ok(Some(CompletePropertyTreeRead {
            snapshot,
            output,
            max_depth: budget.max_depth,
            max_array_items: budget.max_array_items,
        }))
    }

    pub fn search(
        &self,
        path: &PropertyTreePath,
        options: &PropertyTreeSearchOptions,
    ) -> Result<Vec<PropertyTreeSearchMatch>, String> {
        let cursor = self.resolve(path)?;
        let matcher = SearchMatcher::new(&options.query)?;
        let fields = normalize_match_fields(&options.match_fields);
        let limit = options.limit.clamp(1, 1000).saturating_add(1);
        if !cursor.subassets.is_empty() {
            return Ok(self.search_scope_with_subassets(
                cursor.node,
                cursor.owner_file_id,
                &path.full_path(),
                cursor.subassets,
                &matcher,
                &fields,
                limit,
            ));
        }
        let mut canonical = HashMap::new();
        if cursor.owner_file_id != 0 {
            canonical
                .entry(PropertyInstanceId::UnityObject(cursor.owner_file_id))
                .or_insert_with(|| cursor.owner_path.clone());
        }
        let mut matches = Vec::new();
        self.search_node(
            cursor.node,
            cursor.owner_file_id,
            &path.full_path(),
            true,
            &matcher,
            &fields,
            limit,
            false,
            &mut canonical,
            &mut matches,
        );
        Ok(matches)
    }

    fn search_scope_with_subassets(
        &self,
        root: &UnitySerializedPropertySnapshot,
        root_owner_file_id: i64,
        scope_path: &str,
        subassets: &[UnityPropertyTreeSubassetEntry],
        matcher: &SearchMatcher,
        fields: &HashSet<String>,
        limit: usize,
    ) -> Vec<PropertyTreeSearchMatch> {
        let mut matches = Vec::new();
        let mut path_matched_subassets = HashSet::new();

        // Directory objects come first so a type/name query such as
        // `EntityTrack` yields the short alias even when the main object also
        // has a matching ObjectReference field.
        self.search_subasset_directory_entries(
            subassets,
            scope_path,
            matcher,
            fields,
            limit,
            false,
            &mut path_matched_subassets,
            &mut matches,
        );

        // Root catalog search scans each serialized object exactly once. All
        // Unity object ids are pre-seeded so ObjectReference fields remain
        // searchable leaves and do not recursively duplicate another object.
        let mut canonical = self
            .documents
            .keys()
            .map(|file_id| {
                (
                    PropertyInstanceId::UnityObject(*file_id),
                    self.asset_path.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        self.search_node(
            root,
            root_owner_file_id,
            scope_path,
            true,
            matcher,
            fields,
            limit,
            false,
            &mut canonical,
            &mut matches,
        );

        self.search_subasset_documents(
            subassets,
            scope_path,
            matcher,
            fields,
            limit,
            &path_matched_subassets,
            &mut canonical,
            &mut matches,
        );
        matches
    }

    #[allow(clippy::too_many_arguments)]
    fn search_subasset_directory_entries(
        &self,
        entries: &[UnityPropertyTreeSubassetEntry],
        parent_path: &str,
        matcher: &SearchMatcher,
        fields: &HashSet<String>,
        limit: usize,
        path_ancestor_matched: bool,
        path_matched_subassets: &mut HashSet<String>,
        matches: &mut Vec<PropertyTreeSearchMatch>,
    ) {
        for entry in entries {
            if matches.len() >= limit {
                return;
            }
            let entry_path = append_path_segment(parent_path, &entry.segment);
            let entry_node = UnitySerializedPropertySnapshot {
                name: entry.segment.clone(),
                display_name: entry.display_name.clone(),
                property_type: entry.property_type.clone(),
                value_type: "Object".to_string(),
                field_type_full_name: entry.type_full_name.clone(),
                ..Default::default()
            };
            let mut evidence = property_match_evidence(&entry_node, &entry_path, matcher, fields);
            let own_path_matched = evidence.path;
            if path_ancestor_matched {
                evidence.path = false;
            }
            let descendant_path_matched = path_ancestor_matched || own_path_matched;
            if descendant_path_matched {
                path_matched_subassets.insert(entry_path.clone());
            }
            if evidence.any() {
                matches.push(PropertyTreeSearchMatch {
                    path: entry_path.clone(),
                    name: entry.segment.clone(),
                    display_name: entry.display_name.clone(),
                    property_type: if entry.type_full_name.is_empty() {
                        entry.property_type.clone()
                    } else {
                        entry.type_full_name.clone()
                    },
                    display_value: entry.display_name.clone(),
                    evidence,
                });
            }
            self.search_subasset_directory_entries(
                &entry.children,
                &entry_path,
                matcher,
                fields,
                limit,
                descendant_path_matched,
                path_matched_subassets,
                matches,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn search_subasset_documents(
        &self,
        entries: &[UnityPropertyTreeSubassetEntry],
        parent_path: &str,
        matcher: &SearchMatcher,
        fields: &HashSet<String>,
        limit: usize,
        path_matched_subassets: &HashSet<String>,
        canonical: &mut HashMap<PropertyInstanceId, String>,
        matches: &mut Vec<PropertyTreeSearchMatch>,
    ) {
        for entry in entries {
            if matches.len() >= limit {
                return;
            }
            let Some(file_id) = entry.target.target_file_id else {
                continue;
            };
            let Some(document) = self.documents.get(&file_id) else {
                continue;
            };
            let entry_path = append_path_segment(parent_path, &entry.segment);
            for child in &document.root.children {
                if matches.len() >= limit {
                    return;
                }
                if is_root_metadata_child(&document.root, child) {
                    continue;
                }
                let child_path = append_path_segment(&entry_path, &child.name);
                self.search_node(
                    child,
                    file_id,
                    &child_path,
                    false,
                    matcher,
                    fields,
                    limit,
                    path_matched_subassets.contains(&entry_path),
                    canonical,
                    matches,
                );
            }
            self.search_subasset_documents(
                &entry.children,
                &entry_path,
                matcher,
                fields,
                limit,
                path_matched_subassets,
                canonical,
                matches,
            );
        }
    }

    fn resolve<'a>(&'a self, path: &PropertyTreePath) -> Result<ResolvedCursor<'a>, String> {
        let mut node = &self.root;
        let mut owner_file_id = self.root_owner_file_id;
        let mut owner_path = self.asset_path.clone();
        let mut current_path = self.asset_path.clone();
        let mut canonical = self.canonical_paths.clone();
        let mut subassets = self.root.subassets.as_slice();

        for segment in &path.segments {
            if let Some(target_id) = internal_target_id(node, &self.documents) {
                canonical
                    .entry(PropertyInstanceId::UnityObject(target_id))
                    .or_insert(current_path.clone());
                let target = self
                    .documents
                    .get(&target_id)
                    .ok_or_else(|| format!("Referenced document {} is unavailable", target_id))?;
                node = &target.root;
                owner_file_id = target_id;
                owner_path = current_path.clone();
                subassets = &[];
            } else if let Some(reference_id) = managed_reference_id(node) {
                let identity = PropertyInstanceId::ManagedReference {
                    owner_file_id,
                    reference_id,
                };
                canonical.entry(identity).or_insert(current_path.clone());
                let target = self
                    .managed_reference(owner_file_id, reference_id)
                    .ok_or_else(|| {
                        format!(
                            "Managed reference {} is unavailable below '{}'",
                            reference_id, current_path
                        )
                    })?;
                node = target;
                subassets = &[];
            }

            if let Some(entry) = subassets.iter().find(|entry| entry.segment == *segment) {
                let target_id = entry.target.target_file_id.ok_or_else(|| {
                    format!(
                        "Subasset '{}' below '{}' has no local file id",
                        encode_path_segment(segment),
                        current_path
                    )
                })?;
                current_path = append_path_segment(&current_path, segment);
                canonical
                    .entry(PropertyInstanceId::UnityObject(target_id))
                    .or_insert(current_path.clone());
                let target = self.documents.get(&target_id).ok_or_else(|| {
                    format!(
                        "Subasset '{}' below '{}' is unavailable",
                        encode_path_segment(segment),
                        self.asset_path
                    )
                })?;
                node = &target.root;
                owner_file_id = target_id;
                owner_path = current_path.clone();
                subassets = entry.children.as_slice();
                continue;
            }

            let Some(child) = node.children.iter().find(|child| {
                child.name == *segment
                    || child.display_name == *segment
                    || property_leaf_name(&child.property_path) == segment
            }) else {
                let mut available = node
                    .children
                    .iter()
                    .filter(|child| !is_root_metadata_child(node, child))
                    .take(12)
                    .map(|child| encode_path_segment(&child.name))
                    .collect::<Vec<_>>();
                available.extend(
                    subassets
                        .iter()
                        .take(12usize.saturating_sub(available.len()))
                        .map(|entry| encode_path_segment(&entry.segment)),
                );
                return Err(format!(
                    "Property '{}' was not found below '{}'. Available children: {}",
                    encode_path_segment(segment),
                    current_path,
                    if available.is_empty() {
                        "<none>".to_string()
                    } else {
                        available.join(", ")
                    }
                ));
            };
            node = child;
            subassets = &[];
            current_path = append_path_segment(&current_path, segment);
        }

        Ok(ResolvedCursor {
            node,
            owner_file_id,
            owner_path,
            canonical,
            subassets,
        })
    }

    fn project_node(
        &self,
        source: &UnitySerializedPropertySnapshot,
        owner_file_id: i64,
        semantic_path: &str,
        depth: usize,
        canonical: &mut HashMap<PropertyInstanceId, String>,
    ) -> UnitySerializedPropertySnapshot {
        let mut projected = source.clone();
        projected.semantic_path = semantic_path.to_string();
        projected.canonical_path.clear();

        let mut content = source;
        let mut child_owner_file_id = owner_file_id;
        if let Some(target_id) = internal_target_id(source, &self.documents) {
            let identity = PropertyInstanceId::UnityObject(target_id);
            if let Some(first_path) = canonical.get(&identity) {
                if first_path != semantic_path {
                    projected.canonical_path = first_path.clone();
                    projected.children.clear();
                    projected.visible_child_count = 0;
                    projected.children_truncated = false;
                    projected.has_children = false;
                    return projected;
                }
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.documents.get(&target_id) {
                content = &target.root;
                child_owner_file_id = target_id;
                projected.node_kind = "reference".to_string();
                projected.property_type = content.property_type.clone();
                projected.value_type = content.value_type.clone();
                projected.field_type_full_name = content.field_type_full_name.clone();
                projected.field_type_assembly = content.field_type_assembly.clone();
                if projected.display_value.is_empty() {
                    projected.display_value = content.display_value.clone();
                }
            }
        } else if let Some(reference_id) = managed_reference_id(source) {
            let identity = PropertyInstanceId::ManagedReference {
                owner_file_id,
                reference_id,
            };
            if let Some(first_path) = canonical.get(&identity) {
                if first_path != semantic_path {
                    projected.canonical_path = first_path.clone();
                    projected.children.clear();
                    projected.visible_child_count = 0;
                    projected.children_truncated = false;
                    projected.has_children = false;
                    return projected;
                }
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.managed_reference(owner_file_id, reference_id) {
                content = target;
                projected.node_kind = "reference".to_string();
                projected.property_type = "ManagedReference".to_string();
                projected.value_type = "ManagedReference".to_string();
                if !target.field_type_full_name.is_empty() {
                    projected.field_type_full_name = target.field_type_full_name.clone();
                }
            }
        }

        let visible_children = content
            .children
            .iter()
            .filter(|child| !is_root_metadata_child(content, child))
            .collect::<Vec<_>>();
        let total = visible_children.len();
        projected.has_children = total > 0 || content.has_children;
        projected.visible_child_count = total as i32;
        projected.children.clear();

        if depth == 0 {
            projected.visible_child_count = 0;
            projected.children_truncated = projected.has_children;
            return projected;
        }

        let limit = if content.is_array {
            AGENT_PROPERTY_TREE_ARRAY_LIMIT.min(total)
        } else {
            total
        };
        for child in visible_children.into_iter().take(limit) {
            let child_path = append_path_segment(semantic_path, &child.name);
            projected.children.push(self.project_node(
                child,
                child_owner_file_id,
                &child_path,
                depth - 1,
                canonical,
            ));
        }
        projected.visible_child_count = projected.children.len() as i32;
        projected.children_truncated = content.children_truncated || limit < total;
        projected
    }

    fn project_complete_node(
        &self,
        source: &UnitySerializedPropertySnapshot,
        owner_file_id: i64,
        semantic_path: &str,
        tree_depth: usize,
        canonical: &mut HashMap<PropertyInstanceId, String>,
        budget: &mut CompleteProjectionBudget,
    ) -> Option<UnitySerializedPropertySnapshot> {
        let mut projected = source.clone();
        projected.semantic_path = semantic_path.to_string();
        projected.canonical_path.clear();

        let mut content = source;
        let mut child_owner_file_id = owner_file_id;
        if let Some(target_id) = internal_target_id(source, &self.documents) {
            let identity = PropertyInstanceId::UnityObject(target_id);
            if let Some(first_path) = canonical.get(&identity) {
                if first_path != semantic_path {
                    projected.canonical_path = first_path.clone();
                    projected.children.clear();
                    projected.visible_child_count = 0;
                    projected.children_truncated = false;
                    projected.has_children = false;
                    return budget
                        .record_node(&projected, tree_depth)
                        .then_some(projected);
                }
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.documents.get(&target_id) {
                content = &target.root;
                child_owner_file_id = target_id;
                projected.node_kind = "reference".to_string();
                projected.property_type = content.property_type.clone();
                projected.value_type = content.value_type.clone();
                projected.field_type_full_name = content.field_type_full_name.clone();
                projected.field_type_assembly = content.field_type_assembly.clone();
                if projected.display_value.is_empty() {
                    projected.display_value = content.display_value.clone();
                }
            }
        } else if let Some(reference_id) = managed_reference_id(source) {
            let identity = PropertyInstanceId::ManagedReference {
                owner_file_id,
                reference_id,
            };
            if let Some(first_path) = canonical.get(&identity) {
                if first_path != semantic_path {
                    projected.canonical_path = first_path.clone();
                    projected.children.clear();
                    projected.visible_child_count = 0;
                    projected.children_truncated = false;
                    projected.has_children = false;
                    return budget
                        .record_node(&projected, tree_depth)
                        .then_some(projected);
                }
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.managed_reference(owner_file_id, reference_id) {
                content = target;
                projected.node_kind = "reference".to_string();
                projected.property_type = "ManagedReference".to_string();
                projected.value_type = "ManagedReference".to_string();
                if !target.field_type_full_name.is_empty() {
                    projected.field_type_full_name = target.field_type_full_name.clone();
                }
            }
        }

        if content.children_truncated {
            return None;
        }
        let visible_children = content
            .children
            .iter()
            .filter(|child| !is_root_metadata_child(content, child))
            .collect::<Vec<_>>();
        let total = visible_children.len();
        if tree_depth >= AGENT_PROPERTY_TREE_COMPLETE_MAX_DEPTH && total > 0 {
            return None;
        }

        projected.has_children = total > 0 || content.has_children;
        projected.is_array = content.is_array;
        projected.array_size = content.array_size;
        projected.visible_child_count = 0;
        projected.children.clear();
        projected.children_truncated = false;
        if !budget.record_node(&projected, tree_depth) {
            return None;
        }

        let limit = if content.is_array {
            AGENT_PROPERTY_TREE_ARRAY_LIMIT.min(total)
        } else {
            total
        };
        for child in visible_children.into_iter().take(limit) {
            let child_path = append_path_segment(semantic_path, &child.name);
            projected.children.push(self.project_complete_node(
                child,
                child_owner_file_id,
                &child_path,
                tree_depth + 1,
                canonical,
                budget,
            )?);
        }
        projected.visible_child_count = projected.children.len() as i32;
        projected.children_truncated = content.children_truncated || limit < total;
        Some(projected)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_node(
        &self,
        source: &UnitySerializedPropertySnapshot,
        owner_file_id: i64,
        semantic_path: &str,
        is_scope_root: bool,
        matcher: &SearchMatcher,
        fields: &HashSet<String>,
        limit: usize,
        path_ancestor_matched: bool,
        canonical: &mut HashMap<PropertyInstanceId, String>,
        matches: &mut Vec<PropertyTreeSearchMatch>,
    ) {
        if matches.len() >= limit {
            return;
        }

        let mut evidence = if is_scope_root {
            PropertyTreeSearchMatchEvidence::default()
        } else {
            property_match_evidence(source, semantic_path, matcher, fields)
        };
        if path_ancestor_matched {
            evidence.path = false;
        }
        let descendant_path_matched = path_ancestor_matched || evidence.path;
        if evidence.any() {
            matches.push(PropertyTreeSearchMatch {
                path: semantic_path.to_string(),
                name: source.name.clone(),
                display_name: source.display_name.clone(),
                property_type: display_type(source),
                display_value: property_search_display_value(source),
                evidence: evidence.clone(),
            });
            if matches.len() >= limit {
                return;
            }
        }

        if descendant_path_matched && !evidence.has_non_path_match() && fields.len() == 1 {
            return;
        }

        let mut content = source;
        let mut child_owner_file_id = owner_file_id;
        if let Some(target_id) = internal_target_id(source, &self.documents) {
            let identity = PropertyInstanceId::UnityObject(target_id);
            if canonical.contains_key(&identity) {
                return;
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.documents.get(&target_id) {
                content = &target.root;
                child_owner_file_id = target_id;
            }
        } else if let Some(reference_id) = managed_reference_id(source) {
            let identity = PropertyInstanceId::ManagedReference {
                owner_file_id,
                reference_id,
            };
            if canonical.contains_key(&identity) {
                return;
            }
            canonical.insert(identity, semantic_path.to_string());
            if let Some(target) = self.managed_reference(owner_file_id, reference_id) {
                content = target;
            }
        }

        for child in &content.children {
            if is_root_metadata_child(content, child) {
                continue;
            }
            let child_path = append_path_segment(semantic_path, &child.name);
            self.search_node(
                child,
                child_owner_file_id,
                &child_path,
                false,
                matcher,
                fields,
                limit,
                descendant_path_matched,
                canonical,
                matches,
            );
            if matches.len() >= limit {
                return;
            }
        }
    }

    fn managed_reference(
        &self,
        owner_file_id: i64,
        reference_id: i64,
    ) -> Option<&UnitySerializedPropertySnapshot> {
        self.documents
            .get(&owner_file_id)?
            .managed_references
            .get(&reference_id)
    }

    fn compute_canonical_paths(&self) -> HashMap<PropertyInstanceId, String> {
        let mut canonical = HashMap::new();
        if self.root_owner_file_id != 0 {
            canonical.insert(
                PropertyInstanceId::UnityObject(self.root_owner_file_id),
                self.asset_path.clone(),
            );
        }
        self.collect_canonical_paths(
            &self.root,
            self.root_owner_file_id,
            &self.asset_path,
            0,
            &mut canonical,
        );
        canonical
    }

    fn collect_canonical_paths(
        &self,
        source: &UnitySerializedPropertySnapshot,
        owner_file_id: i64,
        semantic_path: &str,
        recursion_depth: usize,
        canonical: &mut HashMap<PropertyInstanceId, String>,
    ) {
        if recursion_depth >= 1024 {
            return;
        }

        let mut content = source;
        let mut child_owner_file_id = owner_file_id;
        if let Some(target_id) = internal_target_id(source, &self.documents) {
            let identity = PropertyInstanceId::UnityObject(target_id);
            if canonical.contains_key(&identity) {
                return;
            }
            canonical.insert(identity, semantic_path.to_string());
            let Some(target) = self.documents.get(&target_id) else {
                return;
            };
            content = &target.root;
            child_owner_file_id = target_id;
        } else if let Some(reference_id) = managed_reference_id(source) {
            let identity = PropertyInstanceId::ManagedReference {
                owner_file_id,
                reference_id,
            };
            if canonical.contains_key(&identity) {
                return;
            }
            canonical.insert(identity, semantic_path.to_string());
            let Some(target) = self.managed_reference(owner_file_id, reference_id) else {
                return;
            };
            content = target;
        }

        for child in &content.children {
            if is_root_metadata_child(content, child) {
                continue;
            }
            let child_path = append_path_segment(semantic_path, &child.name);
            self.collect_canonical_paths(
                child,
                child_owner_file_id,
                &child_path,
                recursion_depth + 1,
                canonical,
            );
        }
    }
}

struct ResolvedCursor<'a> {
    node: &'a UnitySerializedPropertySnapshot,
    owner_file_id: i64,
    owner_path: String,
    canonical: HashMap<PropertyInstanceId, String>,
    subassets: &'a [UnityPropertyTreeSubassetEntry],
}

enum SearchMatcher {
    All,
    Substring(String),
    Regex(Regex),
}

impl SearchMatcher {
    fn new(query: &str) -> Result<Self, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Self::All);
        }
        if let Some(pattern) = query.strip_prefix("re:") {
            let regex = RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|error| format!("Invalid Property Tree search regex: {}", error))?;
            return Ok(Self::Regex(regex));
        }
        Ok(Self::Substring(query.to_ascii_lowercase()))
    }

    fn is_match(&self, value: &str) -> bool {
        match self {
            Self::All => true,
            Self::Substring(query) => value.to_ascii_lowercase().contains(query),
            Self::Regex(regex) => regex.is_match(value),
        }
    }
}

pub fn format_property_tree(snapshot: &UnitySerializedPropertySnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format_node(snapshot, true));
    out.push('\n');
    format_children(snapshot, "", &mut out);
    format_subassets(snapshot, &mut out);
    format_display_sections(snapshot, &mut out);
    out
}

fn format_subassets(snapshot: &UnitySerializedPropertySnapshot, out: &mut String) {
    if snapshot.subassets.is_empty() {
        return;
    }

    let total = count_subasset_entries(&snapshot.subassets);
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str("--- Subassets [");
    out.push_str(&total.to_string());
    out.push_str("] ---\n");
    let mut visible = 0;
    for entry in &snapshot.subassets {
        if visible >= AGENT_PROPERTY_TREE_SUBASSET_PREVIEW_LIMIT {
            break;
        }
        if entry.segment.trim().is_empty() {
            continue;
        }
        out.push_str("  ");
        format_subasset_label(entry, out);
        out.push('\n');
        visible += 1;
        format_subasset_children(entry, "  ", &mut visible, out);
    }
    if visible < total {
        out.push_str("  … +");
        out.push_str(&(total - visible).to_string());
        out.push('\n');
    }
}

fn format_subasset_children(
    parent: &UnityPropertyTreeSubassetEntry,
    prefix: &str,
    visible: &mut usize,
    out: &mut String,
) {
    for (index, child) in parent.children.iter().enumerate() {
        if *visible >= AGENT_PROPERTY_TREE_SUBASSET_PREVIEW_LIMIT {
            return;
        }
        if child.segment.trim().is_empty() {
            continue;
        }
        let last = index + 1 == parent.children.len();
        out.push_str(prefix);
        out.push_str(if last { "└─ " } else { "├─ " });
        format_subasset_label(child, out);
        out.push('\n');
        *visible += 1;
        let next_prefix = format!("{}{}", prefix, if last { "   " } else { "│  " });
        format_subasset_children(child, &next_prefix, visible, out);
    }
}

fn format_subasset_label(entry: &UnityPropertyTreeSubassetEntry, out: &mut String) {
    out.push_str(&encode_path_segment(&entry.segment));
    let property_type = short_type_name(if entry.type_full_name.trim().is_empty() {
        &entry.property_type
    } else {
        &entry.type_full_name
    });
    if !property_type.is_empty() {
        out.push_str(" (");
        out.push_str(&property_type);
        out.push(')');
    }
}

fn count_subasset_entries(entries: &[UnityPropertyTreeSubassetEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| !entry.segment.trim().is_empty())
        .map(|entry| 1 + count_subasset_entries(&entry.children))
        .sum()
}

fn format_display_sections(snapshot: &UnitySerializedPropertySnapshot, out: &mut String) {
    let sections = snapshot
        .display_sections
        .iter()
        .filter(|section| {
            !section.title.trim().is_empty()
                && section.lines.iter().any(|line| !line.trim().is_empty())
        })
        .collect::<Vec<_>>();
    if sections.is_empty() {
        return;
    }

    for section in sections {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("--- ");
        out.push_str(section.title.trim());
        out.push_str(" ---\n");
        for line in section.lines.iter().filter(|line| !line.trim().is_empty()) {
            out.push_str("  ");
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
}

pub fn format_complete_property_tree_within_budget(
    snapshot: &UnitySerializedPropertySnapshot,
    char_limit: usize,
) -> Option<String> {
    if !property_tree_snapshot_is_complete(snapshot) {
        return None;
    }
    let output = format_property_tree(snapshot);
    (output.chars().count() <= char_limit).then_some(output)
}

pub fn property_tree_snapshot_is_complete(node: &UnitySerializedPropertySnapshot) -> bool {
    (!node.children_truncated || node.is_array)
        && node.children.iter().all(property_tree_snapshot_is_complete)
}

pub fn format_property_tree_search_results(
    scope: &str,
    matches: &[PropertyTreeSearchMatch],
    limit: usize,
) -> String {
    if matches.is_empty() {
        return format!("No matching properties under '{}'.", scope);
    }
    let limit = limit.clamp(1, 1000);
    let visible = matches.iter().take(limit).collect::<Vec<_>>();
    let mut out = String::new();
    let path_matches = visible
        .iter()
        .copied()
        .filter(|item| item.evidence.path)
        .collect::<Vec<_>>();
    if !path_matches.is_empty() {
        out.push_str("Path matches:\n");
        for item in path_matches {
            format_property_tree_search_match_line(&mut out, item);
        }
    }

    let field_matches = visible
        .iter()
        .copied()
        .filter(|item| item.evidence.field_name || item.evidence.field_value)
        .collect::<Vec<_>>();
    if !field_matches.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Field matches:\n");
        for item in field_matches {
            format_property_tree_search_match_line(&mut out, item);
            let field_name = property_tree_search_field_name(item);
            if item.evidence.field_name {
                out.push_str("  Matched field_name: ");
                out.push_str(&field_name);
                out.push('\n');
            }
            if item.evidence.field_value {
                out.push_str("  Matched field_value: ");
                out.push_str(&field_name);
                out.push_str(" = ");
                if item.display_value.is_empty() {
                    out.push_str("<empty>");
                } else {
                    out.push_str(&compact_scalar(&item.display_value));
                }
                out.push('\n');
            }
        }
    }

    let type_matches = visible
        .iter()
        .copied()
        .filter(|item| item.evidence.property_type)
        .collect::<Vec<_>>();
    if !type_matches.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Type matches:\n");
        for item in type_matches {
            format_property_tree_search_match_line(&mut out, item);
            out.push_str("  Matched type: ");
            out.push_str(&item.property_type);
            out.push('\n');
        }
    }

    if out.is_empty() {
        return format!("No matching properties under '{}'.", scope);
    }
    if matches.len() > limit {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&format!(
            "… [search limit {} reached; narrow path, query, or match_fields]\n",
            limit
        ));
    }
    out
}

fn format_property_tree_search_match_line(out: &mut String, item: &PropertyTreeSearchMatch) {
    out.push_str(&item.path);
    let short_type = short_type_name(&item.property_type);
    let game_object_summary = short_type == "GameObject"
        && (item.display_value.trim_start().starts_with('(')
            || item.display_value.trim_start().starts_with('['));
    if game_object_summary {
        out.push(' ');
        out.push_str(&compact_semantic_summary(&item.display_value));
    } else if !item.display_value.is_empty() {
        out.push_str(": ");
        out.push_str(&compact_scalar(&item.display_value));
    }
    if !item.property_type.is_empty() && !game_object_summary {
        out.push_str(" (");
        out.push_str(&short_type);
        out.push(')');
    }
    out.push('\n');
}

fn property_tree_search_field_name(item: &PropertyTreeSearchMatch) -> String {
    if !item.display_name.trim().is_empty() {
        item.display_name.trim().to_string()
    } else if !item.name.trim().is_empty() {
        item.name.trim().to_string()
    } else {
        item.path
            .rsplit('/')
            .next()
            .unwrap_or(item.path.as_str())
            .to_string()
    }
}

fn format_children(node: &UnitySerializedPropertySnapshot, prefix: &str, out: &mut String) {
    if is_compact_unity_value(node) {
        return;
    }
    let omitted_array_items = if node.is_array && node.children_truncated {
        (node.array_size - node.children.len() as i32).max(0)
    } else {
        0
    };
    let has_array_omission = node.is_array && node.children_truncated;
    for (index, child) in node.children.iter().enumerate() {
        let last = index + 1 == node.children.len() && !has_array_omission;
        out.push_str(prefix);
        out.push_str(if last { "└─ " } else { "├─ " });
        out.push_str(&format_node(child, false));
        out.push('\n');
        let mut next_prefix = prefix.to_string();
        next_prefix.push_str(if last { "   " } else { "│  " });
        format_children(child, &next_prefix, out);
    }
    if has_array_omission {
        out.push_str(prefix);
        out.push_str("└─ …");
        if omitted_array_items > 0 {
            out.push_str(&format!(" +{}", omitted_array_items));
        }
        out.push('\n');
    }
}

fn format_node(node: &UnitySerializedPropertySnapshot, root: bool) -> String {
    let mut out = if root {
        if node.semantic_path.is_empty() {
            node.display_name.clone()
        } else {
            node.semantic_path.clone()
        }
    } else if node.name.is_empty() {
        node.display_name.clone()
    } else {
        encode_path_segment(&node.name)
    };

    if !node.canonical_path.is_empty() {
        out.push_str(" → ");
        out.push_str(&node.canonical_path);
        return out;
    }

    if node.is_array {
        out.push_str(&format!(" [{}]", node.array_size.max(0)));
        let ty = display_type(node);
        if !ty.is_empty() && ty != "Array" && ty != "Generic" {
            out.push_str(" (");
            out.push_str(&short_type_name(&ty));
            out.push(')');
        }
    } else if is_compact_unity_value(node) {
        if !node.display_value.is_empty() {
            out.push_str(": ");
            out.push_str(&compact_scalar(&node.display_value));
        }
    } else if node.node_kind == "hierarchy" {
        if !node.display_value.trim().is_empty() {
            out.push(' ');
            out.push_str(&compact_semantic_summary(&node.display_value));
        } else {
            out.push_str(" (GameObject)");
        }
    } else if node.has_children || !node.children.is_empty() || node.node_kind == "object" {
        let ty = display_type(node);
        if !ty.is_empty() && ty != "Object" && ty != "Generic" {
            out.push_str(" (");
            out.push_str(&short_type_name(&ty));
            out.push(')');
        }
    } else if !node.display_value.is_empty() {
        out.push_str(": ");
        out.push_str(&compact_scalar(&node.display_value));
    }

    if node.children_truncated && !node.is_array {
        out.push_str(" …");
    }
    out
}

fn build_children(
    value: &YamlValue,
    parent_property_path: &str,
    schema: Option<&ScriptSchema>,
    owner_target: &UnitySerializedPropertyTarget,
    descriptors: &HashMap<i64, DocumentDescriptor>,
    guid_paths: &HashMap<String, String>,
    recursion_depth: usize,
) -> Vec<UnitySerializedPropertySnapshot> {
    if recursion_depth >= 512 {
        return Vec::new();
    }
    match untag_yaml(value) {
        YamlValue::Mapping(mapping) => ordered_mapping_entries(mapping, schema)
            .into_iter()
            .map(|(name, serialized_name, value, field)| {
                let property_path = join_property_path(parent_property_path, &serialized_name);
                build_snapshot(
                    &name,
                    value,
                    &property_path,
                    field.as_ref(),
                    owner_target,
                    descriptors,
                    guid_paths,
                    recursion_depth + 1,
                    false,
                )
            })
            .collect(),
        YamlValue::Sequence(sequence) => sequence
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let property_path = array_element_property_path(parent_property_path, index);
                build_snapshot(
                    &index.to_string(),
                    value,
                    &property_path,
                    None,
                    owner_target,
                    descriptors,
                    guid_paths,
                    recursion_depth + 1,
                    true,
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_snapshot(
    name: &str,
    value: &YamlValue,
    property_path: &str,
    field: Option<&ScriptFieldSchema>,
    owner_target: &UnitySerializedPropertyTarget,
    descriptors: &HashMap<i64, DocumentDescriptor>,
    guid_paths: &HashMap<String, String>,
    recursion_depth: usize,
    is_array_item: bool,
) -> UnitySerializedPropertySnapshot {
    let value = untag_yaml(value);
    if let Some(reference_id) = parse_managed_reference_stub(value) {
        let field_type = field
            .map(|field| field.field_type.clone())
            .unwrap_or_default();
        return UnitySerializedPropertySnapshot {
            property_path: property_path.to_string(),
            node_kind: if is_array_item { "item" } else { "reference" }.to_string(),
            binding_target: Some(with_property_path(owner_target, property_path)),
            display_name: name.to_string(),
            name: name.to_string(),
            property_type: "ManagedReference".to_string(),
            value_type: "ManagedReference".to_string(),
            field_type_full_name: field_type,
            value: serde_json::json!({ "rid": reference_id }),
            display_value: if reference_id <= 0 {
                "null".to_string()
            } else {
                format!("rid:{}", reference_id)
            },
            editable: false,
            has_children: reference_id > 0,
            array_size: -1,
            is_managed_reference: true,
            managed_reference_id: reference_id,
            managed_reference_field_typename: field
                .filter(|field| field.serialize_reference)
                .map(|field| field.field_type.clone())
                .unwrap_or_default(),
            ..Default::default()
        };
    }
    if let Some(reference) = parse_yaml_reference(value) {
        let internal = reference.guid.is_none()
            && reference.file_id != 0
            && descriptors.contains_key(&reference.file_id);
        let target_descriptor = descriptors.get(&reference.file_id);
        let reference_target = internal.then(|| {
            let descriptor = target_descriptor.expect("checked above");
            document_target(
                &owner_target.path.clone().unwrap_or_default(),
                descriptor,
                "",
            )
        });
        let display_value = if reference.file_id == 0 {
            "null".to_string()
        } else if let Some(descriptor) = target_descriptor {
            descriptor.display_name.clone()
        } else if let Some(guid) = reference.guid.as_deref() {
            let resolved = guid_paths
                .get(&guid.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| format!("guid:{}", guid));
            if reference.file_id == 0 {
                resolved
            } else {
                format!("{}#{}", resolved, reference.file_id)
            }
        } else {
            format!("fileID:{}", reference.file_id)
        };
        let field_type = field
            .map(|field| field.field_type.clone())
            .or_else(|| target_descriptor.map(|descriptor| descriptor.type_full_name.clone()))
            .unwrap_or_default();
        return UnitySerializedPropertySnapshot {
            property_path: property_path.to_string(),
            node_kind: if is_array_item { "item" } else { "reference" }.to_string(),
            binding_target: Some(with_property_path(owner_target, property_path)),
            reference_target,
            display_name: name.to_string(),
            name: name.to_string(),
            property_type: "ObjectReference".to_string(),
            value_type: "ObjectReference".to_string(),
            field_type_full_name: field_type.clone(),
            value: serde_json::Value::String(display_value.clone()),
            display_value,
            editable: false,
            has_children: internal,
            array_size: -1,
            reference_type_full_name: field_type,
            ..Default::default()
        };
    }

    let field_type = field
        .map(|field| field.field_type.clone())
        .unwrap_or_default();
    if let Some((property_type, display_value, json_value)) =
        compact_yaml_unity_value(name, value, &field_type)
    {
        return UnitySerializedPropertySnapshot {
            property_path: property_path.to_string(),
            node_kind: if is_array_item { "item" } else { "property" }.to_string(),
            binding_target: Some(with_property_path(owner_target, property_path)),
            display_name: name.to_string(),
            name: name.to_string(),
            property_type: property_type.clone(),
            value_type: property_type,
            field_type_full_name: field_type,
            value: json_value,
            display_value,
            editable: false,
            has_children: false,
            array_size: -1,
            ..Default::default()
        };
    }

    let (property_type, display_value, json_value, is_array, children) = match value {
        YamlValue::Mapping(_) => {
            let children = build_children(
                value,
                property_path,
                None,
                owner_target,
                descriptors,
                guid_paths,
                recursion_depth,
            );
            (
                "Generic".to_string(),
                String::new(),
                serde_json::Value::Null,
                false,
                children,
            )
        }
        YamlValue::Sequence(sequence) => {
            let children = build_children(
                value,
                property_path,
                None,
                owner_target,
                descriptors,
                guid_paths,
                recursion_depth,
            );
            (
                "Generic".to_string(),
                String::new(),
                serde_json::Value::Null,
                true,
                if sequence.is_empty() {
                    Vec::new()
                } else {
                    children
                },
            )
        }
        YamlValue::Bool(value) => (
            "Boolean".to_string(),
            value.to_string(),
            serde_json::Value::Bool(*value),
            false,
            Vec::new(),
        ),
        YamlValue::Number(value) => {
            if let Some(integer) = value.as_i64() {
                (
                    "Integer".to_string(),
                    integer.to_string(),
                    serde_json::Value::Number(integer.into()),
                    false,
                    Vec::new(),
                )
            } else {
                let float = value.as_f64().unwrap_or_default();
                (
                    "Float".to_string(),
                    format_float(float),
                    serde_json::Number::from_f64(float)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null),
                    false,
                    Vec::new(),
                )
            }
        }
        YamlValue::String(value) => (
            "String".to_string(),
            value.clone(),
            serde_json::Value::String(value.clone()),
            false,
            Vec::new(),
        ),
        YamlValue::Null => (
            "String".to_string(),
            "null".to_string(),
            serde_json::Value::Null,
            false,
            Vec::new(),
        ),
        YamlValue::Tagged(_) => unreachable!("untag_yaml removes tags"),
    };

    let child_count = children.len() as i32;
    UnitySerializedPropertySnapshot {
        property_path: property_path.to_string(),
        node_kind: if is_array {
            "array"
        } else if is_array_item {
            "item"
        } else {
            "property"
        }
        .to_string(),
        binding_target: Some(with_property_path(owner_target, property_path)),
        display_name: name.to_string(),
        name: name.to_string(),
        property_type: property_type.clone(),
        value_type: property_type,
        field_type_full_name: field_type,
        value: json_value,
        display_value,
        editable: false,
        has_children: child_count > 0,
        is_array,
        array_size: if is_array { child_count } else { -1 },
        visible_child_count: child_count,
        children,
        ..Default::default()
    }
}

fn compact_yaml_unity_value(
    name: &str,
    value: &YamlValue,
    field_type: &str,
) -> Option<(String, String, serde_json::Value)> {
    let mapping = match value {
        YamlValue::Mapping(mapping) => mapping,
        _ => return None,
    };
    let field_kind = short_type_name(field_type);
    let lower_name = name.to_ascii_lowercase();

    let explicit_kind = match field_kind.as_str() {
        "Vector2" | "Vector3" | "Vector4" | "Vector2Int" | "Vector3Int" | "Quaternion"
        | "Color" | "Rect" | "RectInt" | "Bounds" | "BoundsInt" => Some(field_kind.as_str()),
        _ => None,
    };

    if explicit_kind == Some("Bounds") || explicit_kind == Some("BoundsInt") {
        let (first_name, first_keys, second_name, second_keys) =
            if explicit_kind == Some("BoundsInt") {
                ("position", ["x", "y", "z"], "size", ["x", "y", "z"])
            } else {
                ("center", ["x", "y", "z"], "extents", ["x", "y", "z"])
            };
        let first = mapping_value_any(
            mapping,
            &[first_name, &format!("m_{}", capitalize_ascii(first_name))],
        )?;
        let second = mapping_value_any(
            mapping,
            &[
                second_name,
                if second_name == "extents" {
                    "m_Extent"
                } else {
                    "m_Size"
                },
            ],
        )?;
        let (_, first_display, first_json) =
            compact_yaml_components(first, &first_keys, "Vector3")?;
        let (_, second_display, second_json) =
            compact_yaml_components(second, &second_keys, "Vector3")?;
        let mut object = serde_json::Map::new();
        object.insert(first_name.to_string(), first_json);
        object.insert(second_name.to_string(), second_json);
        return Some((
            explicit_kind.unwrap().to_string(),
            format!(
                "{{{}: {}, {}: {}}}",
                first_name, first_display, second_name, second_display
            ),
            serde_json::Value::Object(object),
        ));
    }

    let inferred_kind = explicit_kind.or_else(|| {
        if mapping_has_exact_keys(mapping, &["r", "g", "b", "a"]) {
            return Some("Color");
        }
        if mapping_has_exact_keys(mapping, &["x", "y", "width", "height"]) {
            return Some("Rect");
        }
        if mapping_has_exact_keys(mapping, &["x", "y", "z", "w"])
            && (lower_name.contains("rotation") || lower_name.contains("quaternion"))
        {
            return Some("Quaternion");
        }
        let vector_hint = [
            "position",
            "scale",
            "euler",
            "direction",
            "offset",
            "center",
            "extent",
            "size",
            "velocity",
            "normal",
            "point",
            "vector",
            "axis",
            "anchor",
            "pivot",
        ]
        .iter()
        .any(|hint| lower_name.contains(hint));
        if vector_hint && mapping_has_exact_keys(mapping, &["x", "y", "z"]) {
            return Some("Vector3");
        }
        if vector_hint && mapping_has_exact_keys(mapping, &["x", "y"]) {
            return Some("Vector2");
        }
        None
    })?;

    let keys: &[&str] = match inferred_kind {
        "Vector2" | "Vector2Int" => &["x", "y"],
        "Vector3" | "Vector3Int" => &["x", "y", "z"],
        "Vector4" | "Quaternion" => &["x", "y", "z", "w"],
        "Color" => &["r", "g", "b", "a"],
        "Rect" | "RectInt" => &["x", "y", "width", "height"],
        _ => return None,
    };
    compact_yaml_components(value, keys, inferred_kind)
}

fn compact_yaml_components(
    value: &YamlValue,
    keys: &[&str],
    property_type: &str,
) -> Option<(String, String, serde_json::Value)> {
    let mapping = match value {
        YamlValue::Mapping(mapping) => mapping,
        _ => return None,
    };
    if !mapping_has_exact_keys(mapping, keys) {
        return None;
    }

    let mut display_parts = Vec::with_capacity(keys.len());
    let mut object = serde_json::Map::new();
    for key in keys {
        let value = mapping.get(YamlValue::String((*key).to_string()))?;
        let (json, display) = compact_yaml_scalar(value)?;
        object.insert((*key).to_string(), json);
        display_parts.push(format!("{}: {}", key, display));
    }
    Some((
        property_type.to_string(),
        format!("{{{}}}", display_parts.join(", ")),
        serde_json::Value::Object(object),
    ))
}

fn compact_yaml_scalar(value: &YamlValue) -> Option<(serde_json::Value, String)> {
    match untag_yaml(value) {
        YamlValue::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Some((
                    serde_json::Value::Number(integer.into()),
                    integer.to_string(),
                ))
            } else {
                let float = number.as_f64()?;
                Some((
                    serde_json::Number::from_f64(float)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null),
                    format_float(float),
                ))
            }
        }
        _ => None,
    }
}

fn mapping_has_exact_keys(mapping: &Mapping, keys: &[&str]) -> bool {
    mapping.len() == keys.len()
        && keys
            .iter()
            .all(|key| mapping.contains_key(YamlValue::String((*key).to_string())))
}

fn mapping_value_any<'a>(mapping: &'a Mapping, keys: &[&str]) -> Option<&'a YamlValue> {
    keys.iter()
        .find_map(|key| mapping.get(YamlValue::String((*key).to_string())))
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn ordered_mapping_entries<'a>(
    mapping: &'a Mapping,
    schema: Option<&ScriptSchema>,
) -> Vec<(String, String, &'a YamlValue, Option<ScriptFieldSchema>)> {
    mapping
        .iter()
        .map(|(key, value)| {
            let serialized_name = yaml_key_string(key);
            let field = schema.and_then(|schema| {
                schema
                    .fields
                    .iter()
                    .find(|field| {
                        field.name == serialized_name
                            || field.former_names.iter().any(|old| old == &serialized_name)
                    })
                    .cloned()
            });
            let semantic_name = field
                .as_ref()
                .map(|field| field.name.clone())
                .unwrap_or_else(|| serialized_name.clone());
            (semantic_name, serialized_name, value, field)
        })
        .collect()
}

fn load_script_schema(
    doc: &crate::unity_yaml::YamlDoc,
    project_root: Option<&Path>,
    guid_paths: &HashMap<String, String>,
) -> Option<ScriptSchema> {
    let guid = doc
        .m_script_guid
        .as_ref()
        .map(crate::asset_db::types::guid_to_hex)?
        .to_ascii_lowercase();
    let script_path = guid_paths.get(&guid)?;
    let absolute = if Path::new(script_path).is_absolute() {
        PathBuf::from(script_path)
    } else {
        project_root?.join(script_path)
    };
    let source = std::fs::read_to_string(&absolute).ok()?;
    let expected_name = absolute.file_stem().and_then(|stem| stem.to_str());
    let metadata = crate::unity_csharp::parse_cs_script(&source, expected_name)?;
    let type_full_name = metadata
        .namespace
        .as_deref()
        .map(|namespace| format!("{}.{}", namespace, metadata.class_name))
        .unwrap_or_else(|| metadata.class_name.clone());
    Some(ScriptSchema {
        type_name: metadata.class_name,
        type_full_name,
        fields: metadata
            .serialized_fields
            .into_iter()
            .map(|field| ScriptFieldSchema {
                name: field.name,
                field_type: field.field_type,
                former_names: field.former_names,
                serialize_reference: field.serialize_reference,
            })
            .collect(),
    })
}

#[derive(Debug)]
struct YamlReference {
    file_id: i64,
    guid: Option<String>,
}

fn parse_yaml_reference(value: &YamlValue) -> Option<YamlReference> {
    let mapping = value.as_mapping()?;
    if mapping.is_empty()
        || mapping
            .keys()
            .any(|key| !matches!(key.as_str(), Some("fileID") | Some("guid") | Some("type")))
    {
        return None;
    }
    let file_id = yaml_mapping_get(mapping, "fileID").and_then(yaml_i64)?;
    let guid = yaml_mapping_get(mapping, "guid")
        .and_then(YamlValue::as_str)
        .map(str::trim)
        .filter(|guid| !guid.is_empty())
        .map(|guid| guid.to_ascii_lowercase());
    Some(YamlReference { file_id, guid })
}

fn parse_managed_reference_stub(value: &YamlValue) -> Option<i64> {
    let mapping = value.as_mapping()?;
    if mapping.len() != 1 {
        return None;
    }
    yaml_mapping_get(mapping, "rid").and_then(yaml_i64)
}

fn managed_reference_id(node: &UnitySerializedPropertySnapshot) -> Option<i64> {
    (node.is_managed_reference && node.managed_reference_id > 0)
        .then_some(node.managed_reference_id)
}

fn extract_managed_reference_registry(
    root: &UnitySerializedPropertySnapshot,
) -> HashMap<i64, UnitySerializedPropertySnapshot> {
    let Some(references) = root
        .children
        .iter()
        .find(|child| child.name == "references")
    else {
        return HashMap::new();
    };
    let Some(entries) = references
        .children
        .iter()
        .find(|child| child.name == "RefIds")
    else {
        return HashMap::new();
    };

    let mut registry = HashMap::new();
    for entry in &entries.children {
        let reference_id = entry
            .children
            .iter()
            .find(|child| child.name == "rid")
            .and_then(|child| child.value.as_i64())
            .unwrap_or_default();
        if reference_id <= 0 {
            continue;
        }
        let Some(data) = entry.children.iter().find(|child| child.name == "data") else {
            continue;
        };
        let mut snapshot = data.clone();
        snapshot.name = format!("rid:{}", reference_id);
        snapshot.display_name = snapshot.name.clone();
        snapshot.node_kind = "object".to_string();
        snapshot.property_type = "ManagedReference".to_string();
        snapshot.value_type = "ManagedReference".to_string();
        snapshot.is_managed_reference = false;
        snapshot.managed_reference_id = 0;
        if let Some(type_node) = entry.children.iter().find(|child| child.name == "type") {
            let class_name = type_node
                .children
                .iter()
                .find(|child| child.name == "class")
                .map(|child| child.display_value.trim())
                .unwrap_or_default();
            let namespace = type_node
                .children
                .iter()
                .find(|child| child.name == "ns")
                .map(|child| child.display_value.trim())
                .unwrap_or_default();
            snapshot.field_type_full_name = if namespace.is_empty() {
                class_name.to_string()
            } else if class_name.is_empty() {
                namespace.to_string()
            } else {
                format!("{}.{}", namespace, class_name)
            };
            snapshot.field_type_assembly = type_node
                .children
                .iter()
                .find(|child| child.name == "asm")
                .map(|child| child.display_value.clone())
                .unwrap_or_default();
        }
        registry.insert(reference_id, snapshot);
    }
    registry
}

fn unwrap_document_value<'a>(value: &'a YamlValue, type_name: &str) -> &'a YamlValue {
    let Some(mapping) = value.as_mapping() else {
        return value;
    };
    if let Some(value) = mapping.get(YamlValue::String(type_name.to_string())) {
        return value;
    }
    if mapping.len() == 1 {
        if let Some((_, value)) = mapping.iter().next() {
            return value;
        }
    }
    value
}

fn untag_yaml(value: &YamlValue) -> &YamlValue {
    match value {
        YamlValue::Tagged(tagged) => untag_yaml(&tagged.value),
        _ => value,
    }
}

fn build_hierarchy_property_tree_root(
    asset_path: &str,
    docs: &[crate::unity_yaml::YamlDoc],
    descriptors: &HashMap<i64, DocumentDescriptor>,
) -> Option<UnitySerializedPropertySnapshot> {
    let normalized = asset_path.trim_end_matches('/');
    let lower = normalized.to_ascii_lowercase();
    let is_prefab = lower.ends_with(".prefab");
    if !is_prefab && !lower.ends_with(".unity") {
        return None;
    }

    let hierarchy = crate::unity_yaml::build_go_tree(docs);
    if hierarchy.is_empty() {
        return None;
    }

    let mut component_docs = HashMap::<i64, Vec<&crate::unity_yaml::YamlDoc>>::new();
    for doc in docs {
        if doc.is_stripped || doc.class_id == 1 {
            continue;
        }
        if let Some(game_object_id) = doc.m_game_object_id.filter(|file_id| *file_id != 0) {
            component_docs.entry(game_object_id).or_default().push(doc);
        }
    }
    for entries in component_docs.values_mut() {
        entries.sort_by_key(|doc| doc.doc_index);
    }

    let mut hierarchy_children = hierarchy
        .iter()
        .map(|node| {
            build_hierarchy_game_object_snapshot(normalized, node, descriptors, &component_docs)
        })
        .collect::<Vec<_>>();
    make_sibling_names_unique(&mut hierarchy_children);

    // A prefab path already identifies its root GameObject, so its own
    // components and direct child objects live immediately below the asset.
    // Scene assets retain their top-level GameObject names.
    let children = if is_prefab && hierarchy_children.len() == 1 {
        hierarchy_children.remove(0).children
    } else {
        hierarchy_children
    };
    let target = UnitySerializedPropertyTarget {
        kind: "asset".to_string(),
        path: Some(normalized.to_string()),
        ..Default::default()
    };
    Some(UnitySerializedPropertySnapshot {
        property_path: String::new(),
        node_kind: "asset".to_string(),
        binding_target: Some(target),
        display_name: normalized.to_string(),
        name: normalized.to_string(),
        property_type: if is_prefab { "Prefab" } else { "Scene" }.to_string(),
        value_type: "Object".to_string(),
        field_type_full_name: if is_prefab {
            "UnityEngine.GameObject"
        } else {
            "UnityEngine.SceneManagement.Scene"
        }
        .to_string(),
        value: serde_json::Value::String(normalized.to_string()),
        display_value: normalized.to_string(),
        editable: false,
        has_children: !children.is_empty(),
        array_size: -1,
        visible_child_count: children.len() as i32,
        children,
        ..Default::default()
    })
}

fn build_asset_property_tree_root(
    asset_path: &str,
    main_file_id: i64,
    docs: &[crate::unity_yaml::YamlDoc],
    descriptors: &HashMap<i64, DocumentDescriptor>,
    documents: &HashMap<i64, YamlPropertyDocument>,
) -> Result<UnitySerializedPropertySnapshot, String> {
    let mut root = documents
        .get(&main_file_id)
        .ok_or_else(|| "Main Property Tree document is unavailable".to_string())?
        .root
        .clone();
    make_sibling_names_unique(&mut root.children);
    root.subassets = build_property_tree_subasset_entries(
        asset_path,
        main_file_id,
        docs,
        descriptors,
        documents,
        &root.children,
    );
    root.has_children = !root.children.is_empty();
    root.visible_child_count = root.children.len() as i32;
    Ok(root)
}

fn build_property_tree_subasset_entries(
    asset_path: &str,
    main_file_id: i64,
    docs: &[crate::unity_yaml::YamlDoc],
    descriptors: &HashMap<i64, DocumentDescriptor>,
    documents: &HashMap<i64, YamlPropertyDocument>,
    root_children: &[UnitySerializedPropertySnapshot],
) -> Vec<UnityPropertyTreeSubassetEntry> {
    let mut used = root_children
        .iter()
        .filter(|child| !is_root_metadata_name(&child.name))
        .map(|child| child.name.clone())
        .collect::<HashSet<_>>();
    if root_children.iter().any(|child| child.name == "m_Script") {
        used.insert("Script".to_string());
    }
    let mut next_ordinals = HashMap::<String, usize>::new();
    let candidate_order = docs
        .iter()
        .filter(|doc| doc.file_id != main_file_id && descriptors.contains_key(&doc.file_id))
        .map(|doc| doc.file_id)
        .collect::<Vec<_>>();
    let candidate_ids = candidate_order.iter().copied().collect::<HashSet<_>>();
    let mut claimed = HashSet::from([main_file_id]);
    let mut entries = Vec::new();
    append_owned_subasset_entries(
        asset_path,
        main_file_id,
        descriptors,
        documents,
        &candidate_ids,
        &mut claimed,
        &mut used,
        &mut next_ordinals,
        &mut entries,
    );

    // Documents without a serialized owner remain stable roots in source
    // order. Their own serialized references can still claim descendants.
    for file_id in candidate_order {
        if !claimed.insert(file_id) {
            continue;
        }
        let Some(entry) = build_owned_subasset_entry(
            asset_path,
            file_id,
            descriptors,
            documents,
            &candidate_ids,
            &mut claimed,
            &mut used,
            &mut next_ordinals,
        ) else {
            continue;
        };
        entries.push(entry);
    }
    entries
}

#[allow(clippy::too_many_arguments)]
fn append_owned_subasset_entries(
    asset_path: &str,
    owner_file_id: i64,
    descriptors: &HashMap<i64, DocumentDescriptor>,
    documents: &HashMap<i64, YamlPropertyDocument>,
    candidate_ids: &HashSet<i64>,
    claimed: &mut HashSet<i64>,
    used: &mut HashSet<String>,
    next_ordinals: &mut HashMap<String, usize>,
    destination: &mut Vec<UnityPropertyTreeSubassetEntry>,
) {
    for file_id in serialized_local_reference_order(owner_file_id, documents, candidate_ids) {
        if !claimed.insert(file_id) {
            continue;
        }
        let Some(entry) = build_owned_subasset_entry(
            asset_path,
            file_id,
            descriptors,
            documents,
            candidate_ids,
            claimed,
            used,
            next_ordinals,
        ) else {
            continue;
        };
        destination.push(entry);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_owned_subasset_entry(
    asset_path: &str,
    file_id: i64,
    descriptors: &HashMap<i64, DocumentDescriptor>,
    documents: &HashMap<i64, YamlPropertyDocument>,
    candidate_ids: &HashSet<i64>,
    claimed: &mut HashSet<i64>,
    used: &mut HashSet<String>,
    next_ordinals: &mut HashMap<String, usize>,
) -> Option<UnityPropertyTreeSubassetEntry> {
    let descriptor = descriptors.get(&file_id)?;
    let base = if descriptor.display_name.trim().is_empty() {
        descriptor.type_name.trim()
    } else {
        descriptor.display_name.trim()
    };
    let segment = unique_subasset_segment(
        if base.is_empty() { "Subasset" } else { base },
        used,
        next_ordinals,
    );

    let mut child_used = documents
        .get(&file_id)
        .map(|document| serialized_root_segments(&document.root))
        .unwrap_or_default();
    let mut child_ordinals = HashMap::new();
    let mut children = Vec::new();
    append_owned_subasset_entries(
        asset_path,
        file_id,
        descriptors,
        documents,
        candidate_ids,
        claimed,
        &mut child_used,
        &mut child_ordinals,
        &mut children,
    );

    Some(UnityPropertyTreeSubassetEntry {
        segment,
        display_name: descriptor.display_name.clone(),
        property_type: descriptor.type_name.clone(),
        type_full_name: descriptor.type_full_name.clone(),
        target: target_without_property(asset_path, descriptor),
        children,
    })
}

fn unique_subasset_segment(
    base: &str,
    used: &mut HashSet<String>,
    next_ordinals: &mut HashMap<String, usize>,
) -> String {
    let mut ordinal = next_ordinals.get(base).copied().unwrap_or(1).max(1);
    let mut segment = if ordinal == 1 {
        base.to_string()
    } else {
        format!("{}[{}]", base, ordinal)
    };
    while used.contains(&segment) {
        ordinal += 1;
        segment = format!("{}[{}]", base, ordinal);
    }
    used.insert(segment.clone());
    next_ordinals.insert(base.to_string(), ordinal + 1);
    segment
}

fn serialized_root_segments(root: &UnitySerializedPropertySnapshot) -> HashSet<String> {
    let mut used = root
        .children
        .iter()
        .filter(|child| !is_root_metadata_child(root, child))
        .flat_map(|child| [child.name.clone(), child.display_name.clone()])
        .filter(|name| !name.trim().is_empty())
        .collect::<HashSet<_>>();
    if root.children.iter().any(|child| child.name == "m_Script") {
        used.insert("Script".to_string());
    }
    used
}

fn serialized_local_reference_order(
    owner_file_id: i64,
    documents: &HashMap<i64, YamlPropertyDocument>,
    candidate_ids: &HashSet<i64>,
) -> Vec<i64> {
    fn visit(
        node: &UnitySerializedPropertySnapshot,
        document: &YamlPropertyDocument,
        candidate_ids: &HashSet<i64>,
        root: bool,
        managed_seen: &mut HashSet<i64>,
        out: &mut Vec<i64>,
    ) {
        if let Some(file_id) = node
            .reference_target
            .as_ref()
            .and_then(|target| target.target_file_id)
            .filter(|file_id| candidate_ids.contains(file_id))
        {
            out.push(file_id);
            return;
        }
        if let Some(reference_id) = managed_reference_id(node).filter(|id| *id > 0) {
            if managed_seen.insert(reference_id) {
                if let Some(managed) = document.managed_references.get(&reference_id) {
                    visit(managed, document, candidate_ids, false, managed_seen, out);
                }
            }
            return;
        }
        for child in &node.children {
            if root && is_root_metadata_child(node, child) {
                continue;
            }
            visit(child, document, candidate_ids, false, managed_seen, out);
        }
    }

    let Some(document) = documents.get(&owner_file_id) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut managed_seen = HashSet::new();
    visit(
        &document.root,
        document,
        candidate_ids,
        true,
        &mut managed_seen,
        &mut out,
    );
    out
}

fn build_hierarchy_game_object_snapshot(
    asset_path: &str,
    node: &crate::unity_yaml::HierarchyNode,
    descriptors: &HashMap<i64, DocumentDescriptor>,
    component_docs: &HashMap<i64, Vec<&crate::unity_yaml::YamlDoc>>,
) -> UnitySerializedPropertySnapshot {
    let mut children = Vec::new();
    if let Some(descriptor) = descriptors.get(&node.file_id) {
        let document_name = if descriptor.type_name == "GameObject" {
            "GameObject"
        } else {
            descriptor.type_name.as_str()
        };
        children.push(build_hierarchy_document_reference(
            document_name,
            asset_path,
            descriptor,
        ));
    }
    if let Some(components) = component_docs.get(&node.file_id) {
        for component in components {
            if let Some(descriptor) = descriptors.get(&component.file_id) {
                children.push(build_hierarchy_document_reference(
                    &short_type_name(&descriptor.type_name),
                    asset_path,
                    descriptor,
                ));
            }
        }
    }
    children.extend(node.children.iter().map(|child| {
        build_hierarchy_game_object_snapshot(asset_path, child, descriptors, component_docs)
    }));
    make_sibling_names_unique(&mut children);

    let target = UnitySerializedPropertyTarget {
        kind: "gameobject".to_string(),
        path: Some(asset_path.to_string()),
        object_file_id: Some(node.file_id),
        target_file_id: Some(node.file_id),
        target_type_full_name: Some("UnityEngine.GameObject".to_string()),
        target_type_name: Some("GameObject".to_string()),
        ..Default::default()
    };
    UnitySerializedPropertySnapshot {
        property_path: String::new(),
        node_kind: "object".to_string(),
        binding_target: Some(target),
        display_name: node.name.clone(),
        name: node.name.clone(),
        property_type: "GameObject".to_string(),
        value_type: "Object".to_string(),
        field_type_full_name: "UnityEngine.GameObject".to_string(),
        value: serde_json::Value::String(node.name.clone()),
        display_value: node.name.clone(),
        editable: false,
        has_children: !children.is_empty(),
        array_size: -1,
        visible_child_count: children.len() as i32,
        children,
        ..Default::default()
    }
}

fn build_hierarchy_document_reference(
    name: &str,
    asset_path: &str,
    descriptor: &DocumentDescriptor,
) -> UnitySerializedPropertySnapshot {
    let target = document_target(asset_path, descriptor, "");
    UnitySerializedPropertySnapshot {
        property_path: String::new(),
        node_kind: "reference".to_string(),
        binding_target: Some(target.clone()),
        reference_target: Some(target),
        display_name: name.to_string(),
        name: name.to_string(),
        property_type: descriptor.type_name.clone(),
        value_type: "ObjectReference".to_string(),
        field_type_full_name: descriptor.type_full_name.clone(),
        value: serde_json::Value::String(descriptor.display_name.clone()),
        display_value: descriptor.display_name.clone(),
        editable: false,
        has_children: true,
        array_size: -1,
        ..Default::default()
    }
}

fn make_sibling_names_unique(children: &mut [UnitySerializedPropertySnapshot]) {
    let mut used = HashSet::<String>::new();
    let mut next_ordinal = HashMap::<String, usize>::new();
    for child in children {
        let base = child.name.clone();
        let mut ordinal = next_ordinal.get(&base).copied().unwrap_or(1);
        let mut candidate = if ordinal == 1 {
            base.clone()
        } else {
            format!("{}[{}]", base, ordinal)
        };
        while used.contains(&candidate) {
            ordinal += 1;
            candidate = format!("{}[{}]", base, ordinal);
        }
        next_ordinal.insert(base, ordinal + 1);
        used.insert(candidate.clone());
        child.name = candidate.clone();
        child.display_name = candidate;
    }
}

fn collect_internal_document_references(
    documents: &HashMap<i64, YamlPropertyDocument>,
) -> HashSet<i64> {
    fn visit(node: &UnitySerializedPropertySnapshot, out: &mut HashSet<i64>) {
        if let Some(target_id) = node
            .reference_target
            .as_ref()
            .and_then(|target| target.target_file_id)
        {
            out.insert(target_id);
        }
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut out = HashSet::new();
    for document in documents.values() {
        visit(&document.root, &mut out);
    }
    out
}

fn internal_target_id(
    node: &UnitySerializedPropertySnapshot,
    documents: &HashMap<i64, YamlPropertyDocument>,
) -> Option<i64> {
    node.reference_target
        .as_ref()
        .and_then(|target| target.target_file_id)
        .filter(|file_id| documents.contains_key(file_id))
}

fn document_target(
    asset_path: &str,
    descriptor: &DocumentDescriptor,
    property_path: &str,
) -> UnitySerializedPropertyTarget {
    UnitySerializedPropertyTarget {
        kind: "asset".to_string(),
        path: Some(asset_path.to_string()),
        target_file_id: Some(descriptor.file_id),
        target_type_full_name: Some(descriptor.type_full_name.clone()),
        target_type_name: Some(descriptor.type_name.clone()),
        property_path: (!property_path.is_empty()).then(|| property_path.to_string()),
        ..Default::default()
    }
}

fn target_without_property(
    asset_path: &str,
    descriptor: &DocumentDescriptor,
) -> UnitySerializedPropertyTarget {
    document_target(asset_path, descriptor, "")
}

fn with_property_path(
    target: &UnitySerializedPropertyTarget,
    property_path: &str,
) -> UnitySerializedPropertyTarget {
    let mut target = target.clone();
    target.property_path = Some(property_path.to_string());
    target
}

fn property_match_evidence(
    node: &UnitySerializedPropertySnapshot,
    path: &str,
    matcher: &SearchMatcher,
    fields: &HashSet<String>,
) -> PropertyTreeSearchMatchEvidence {
    let field_value = property_search_display_value(node);
    PropertyTreeSearchMatchEvidence {
        path: fields.contains("path") && matcher.is_match(path),
        field_name: fields.contains("field_name")
            && (matcher.is_match(&node.name) || matcher.is_match(&node.display_name)),
        field_value: fields.contains("field_value") && matcher.is_match(&field_value),
        property_type: fields.contains("type")
            && (matcher.is_match(&node.property_type)
                || matcher.is_match(&node.value_type)
                || matcher.is_match(&node.field_type_full_name)),
    }
}

fn property_search_display_value(node: &UnitySerializedPropertySnapshot) -> String {
    if node.display_value.is_empty() {
        node.value.to_string()
    } else {
        node.display_value.clone()
    }
}

fn normalize_match_fields(input: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for value in input {
        for field in value.split([',', '|']) {
            match field.trim().to_ascii_lowercase().as_str() {
                "name" | "field_name" => {
                    out.insert("field_name".to_string());
                }
                "value" | "field_value" => {
                    out.insert("field_value".to_string());
                }
                "type" => {
                    out.insert("type".to_string());
                }
                "path" => {
                    out.insert("path".to_string());
                }
                "all" => {
                    out.extend(
                        ["path", "field_name", "field_value", "type"]
                            .into_iter()
                            .map(str::to_string),
                    );
                }
                _ => {}
            }
        }
    }
    if out.is_empty() {
        out.extend(
            ["path", "field_name", "field_value", "type"]
                .into_iter()
                .map(str::to_string),
        );
    }
    out
}

fn is_root_metadata_child(
    parent: &UnitySerializedPropertySnapshot,
    child: &UnitySerializedPropertySnapshot,
) -> bool {
    parent.property_path.is_empty() && is_root_metadata_name(&child.name)
}

fn is_root_metadata_name(name: &str) -> bool {
    ROOT_METADATA_FIELDS.iter().any(|field| name == *field)
}

fn display_type(node: &UnitySerializedPropertySnapshot) -> String {
    if !node.field_type_full_name.is_empty() {
        node.field_type_full_name.clone()
    } else if !node.value_type.is_empty() {
        node.value_type.clone()
    } else {
        node.property_type.clone()
    }
}

fn is_compact_unity_value(node: &UnitySerializedPropertySnapshot) -> bool {
    if node.display_value.trim().is_empty() {
        return false;
    }
    let property_type = short_type_name(&node.property_type);
    let value_type = short_type_name(&node.value_type);
    let field_type = short_type_name(&node.field_type_full_name);
    [
        property_type.as_str(),
        value_type.as_str(),
        field_type.as_str(),
    ]
    .iter()
    .any(|value| {
        matches!(
            *value,
            "Vector2"
                | "Vector3"
                | "Vector4"
                | "Vector2Int"
                | "Vector3Int"
                | "Quaternion"
                | "Color"
                | "Rect"
                | "RectInt"
                | "Bounds"
                | "BoundsInt"
                | "AnimationCurve"
                | "Gradient"
                | "Hash128"
        )
    })
}

fn short_type_name(value: &str) -> String {
    let without_assembly = value.split(',').next().unwrap_or(value).trim();
    without_assembly
        .rsplit('.')
        .next()
        .unwrap_or(without_assembly)
        .to_string()
}

fn compact_scalar(value: &str) -> String {
    const LIMIT: usize = 160;
    let normalized = value.replace(['\r', '\n', '\t'], " ");
    let mut chars = normalized.chars();
    let prefix: String = chars.by_ref().take(LIMIT).collect();
    if chars.next().is_some() {
        format!("{}…", prefix)
    } else if normalized.contains(' ') || normalized.is_empty() {
        format!("{:?}", normalized)
    } else {
        normalized
    }
}

fn compact_semantic_summary(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_float(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{:.1}", value)
    } else {
        value.to_string()
    }
}

fn yaml_mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn yaml_i64(value: &YamlValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn yaml_key_string(value: &YamlValue) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| {
        serde_yaml::to_string(value)
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

fn join_property_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", parent, name)
    }
}

fn array_element_property_path(parent: &str, index: usize) -> String {
    format!("{}.Array.data[{}]", parent, index)
}

fn serialized_property_path_segments(path: &str) -> Vec<String> {
    let normalized = path.replace(".Array.data[", "[");
    let mut segments = Vec::new();
    for part in normalized.split('.') {
        let mut remaining = part;
        loop {
            let Some(open) = remaining.find('[') else {
                if !remaining.is_empty() {
                    segments.push(remaining.to_string());
                }
                break;
            };
            if open > 0 {
                segments.push(remaining[..open].to_string());
            }
            let Some(close_offset) = remaining[open + 1..].find(']') else {
                if open + 1 < remaining.len() {
                    segments.push(remaining[open + 1..].to_string());
                }
                break;
            };
            let close = open + 1 + close_offset;
            segments.push(remaining[open + 1..close].to_string());
            remaining = &remaining[close + 1..];
            if remaining.is_empty() {
                break;
            }
        }
    }
    segments
}

fn relative_serialized_property_segments(base: &str, full: &str) -> Option<Vec<String>> {
    let base = serialized_property_path_segments(base);
    let full = serialized_property_path_segments(full);
    if full.len() < base.len()
        || !full
            .iter()
            .zip(base.iter())
            .all(|(left, right)| left == right)
    {
        return None;
    }
    Some(full.into_iter().skip(base.len()).collect())
}

fn property_leaf_name(path: &str) -> &str {
    if let Some(index) = path.rfind(".Array.data[") {
        return path[index + ".Array.data[".len()..]
            .strip_suffix(']')
            .unwrap_or(&path[index + ".Array.data[".len()..]);
    }
    path.rsplit('.').next().unwrap_or(path)
}

fn asset_boundaries(path: &str) -> Vec<usize> {
    let lower = path.to_ascii_lowercase();
    let mut boundaries = Vec::new();
    for extension in UNITY_ASSET_EXTENSIONS {
        let marker = format!(".{}", extension);
        let mut offset = 0;
        while let Some(index) = lower[offset..].find(&marker) {
            let end = offset + index + marker.len();
            if end == lower.len() || lower.as_bytes().get(end) == Some(&b'/') {
                boundaries.push(end);
            }
            offset = end;
            if offset >= lower.len() {
                break;
            }
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

fn resolve_asset_path(root: &Path, candidate: &str) -> PathBuf {
    let path = PathBuf::from(candidate);
    if path.is_absolute() || root.as_os_str().is_empty() {
        path
    } else {
        root.join(path)
    }
}

fn normalize_asset_display_path(root: &Path, raw: &str, absolute: &Path) -> String {
    if !root.as_os_str().is_empty() {
        if let Ok(relative) = absolute.strip_prefix(root) {
            return relative.to_string_lossy().replace('\\', "/");
        }
    }
    let normalized = raw.trim_start_matches("./").replace('\\', "/");
    for marker in ["/Assets/", "/Packages/"] {
        if let Some(index) = normalized.find(marker) {
            return normalized[index + 1..].to_string();
        }
    }
    normalized
}

fn decode_path_segment(segment: &str) -> Result<String, String> {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            Some(other) => {
                return Err(format!(
                    "Invalid Property Tree path escape '~{}' in '{}'",
                    other, segment
                ))
            }
            None => {
                return Err(format!(
                    "Incomplete Property Tree path escape in '{}'",
                    segment
                ))
            }
        }
    }
    Ok(out)
}

fn encode_path_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn append_path_segment(path: &str, segment: &str) -> String {
    format!(
        "{}/{}",
        path.trim_end_matches('/'),
        encode_path_segment(segment)
    )
}

fn append_segments<'a>(path: &str, segments: impl Iterator<Item = &'a str>) -> String {
    segments.fold(path.trim_end_matches('/').to_string(), |path, segment| {
        append_path_segment(&path, segment)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMELINE_YAML: &str = r#"--- !u!114 &11400000
MonoBehaviour:
  m_ObjectHideFlags: 0
  m_Name: LightNormalAttack1
  hitTrack: {fileID: 11400001}
  bakedRootMotion:
  - 0
  - 1
  - 2
  - 3
  - 4
--- !u!114 &11400001
MonoBehaviour:
  m_Name: HitTrack
  _parent: {fileID: 11400000}
  clips:
  - {fileID: 11400002}
  - {fileID: 11400003}
--- !u!114 &11400002
MonoBehaviour:
  m_Name: HitBoxClipA
  _parent: {fileID: 11400001}
  kind: AttackWindow
  damage: 12
--- !u!114 &11400003
MonoBehaviour:
  m_Name: HitBoxClipB
  _parent: {fileID: 11400001}
  kind: Recovery
  damage: 5
"#;

    fn parse_tree() -> YamlPropertyTree {
        YamlPropertyTree::parse(
            "Assets/Actions/LightNormalAttack1.asset",
            TIMELINE_YAML,
            None,
            &HashMap::new(),
        )
        .expect("parse timeline")
    }

    fn path(value: &str) -> PropertyTreePath {
        PropertyTreePath::parse("", value).expect("parse path")
    }

    #[test]
    fn live_property_names_prefer_the_property_over_its_inherited_object_target() {
        let parent = UnitySerializedPropertySnapshot {
            name: "GameObject".to_string(),
            ..Default::default()
        };
        let game_object_target = UnitySerializedPropertyTarget {
            kind: "gameobject".to_string(),
            object_path: Some("Root".to_string()),
            ..Default::default()
        };
        let children = vec![
            UnitySerializedPropertySnapshot {
                property_path: "m_Name".to_string(),
                name: "Name".to_string(),
                display_name: "Name".to_string(),
                binding_target: Some(game_object_target.clone()),
                ..Default::default()
            },
            UnitySerializedPropertySnapshot {
                property_path: "m_IsActive".to_string(),
                name: "Active".to_string(),
                display_name: "Active".to_string(),
                binding_target: Some(game_object_target),
                ..Default::default()
            },
        ];

        assert_eq!(
            live_child_names_from_children(&parent, &children),
            vec!["Name".to_string(), "Active".to_string()]
        );
    }

    #[test]
    fn compact_unity_values_render_inline_without_implementation_children() {
        let mut root = UnitySerializedPropertySnapshot {
            semantic_path: "Assets/Scenes/Arena.unity/Hero/Transform".to_string(),
            name: "Transform".to_string(),
            node_kind: "object".to_string(),
            property_type: "Transform".to_string(),
            has_children: true,
            ..Default::default()
        };
        root.children.push(UnitySerializedPropertySnapshot {
            property_path: "m_LocalPosition".to_string(),
            name: "Local Position".to_string(),
            display_name: "Local Position".to_string(),
            property_type: "Vector3".to_string(),
            value_type: "Vector3".to_string(),
            display_value: "{x: 1, y: 2, z: 3}".to_string(),
            has_children: true,
            children: vec![UnitySerializedPropertySnapshot {
                name: "x".to_string(),
                display_value: "1".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });

        let output = format_property_tree(&root);
        assert!(output.contains("Local Position: {x: 1, y: 2, z: 3}"));
        assert!(!output
            .lines()
            .any(|line| line.trim_start().starts_with("└─ x")));
    }

    #[test]
    fn read_only_display_sections_render_after_the_addressable_tree() {
        let mut root = UnitySerializedPropertySnapshot {
            semantic_path: "Assets/Scenes/Arena.unity/Hero".to_string(),
            name: "Hero".to_string(),
            node_kind: "object".to_string(),
            property_type: "GameObject".to_string(),
            has_children: true,
            display_sections: vec![crate::view::UnityPropertyTreeDisplaySection {
                title: "Transform".to_string(),
                lines: vec!["World Position: {x: 1, y: 2, z: 3}".to_string()],
            }],
            ..Default::default()
        };
        root.children.push(UnitySerializedPropertySnapshot {
            name: "GameObject".to_string(),
            node_kind: "object".to_string(),
            property_type: "GameObject".to_string(),
            has_children: true,
            children: vec![UnitySerializedPropertySnapshot {
                name: "Name".to_string(),
                display_value: "Hero".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });

        let output = format_property_tree(&root);
        assert!(output.contains("└─ GameObject (GameObject)\n"));
        assert!(output.contains("--- Transform ---\n  World Position: {x: 1, y: 2, z: 3}"));
        assert!(!output.contains("├─ Transform\n"));
    }

    #[test]
    fn live_scene_hierarchy_is_visible_and_searches_with_the_same_paths() {
        let target_dummy_path = "Assets/Scenes/Arena.unity/ECS Prototype/TargetDummy";
        let target_dummy = UnitySerializedPropertySnapshot {
            semantic_path: target_dummy_path.to_string(),
            node_kind: "hierarchy".to_string(),
            name: "TargetDummy".to_string(),
            display_name: "TargetDummy".to_string(),
            property_type: "GameObject".to_string(),
            field_type_full_name: "UnityEngine.GameObject".to_string(),
            display_value: "(MeshFilter, MeshRenderer) [Layer:Friend]".to_string(),
            ..Default::default()
        };
        let ecs_path = "Assets/Scenes/Arena.unity/ECS Prototype";
        let ecs = UnitySerializedPropertySnapshot {
            semantic_path: ecs_path.to_string(),
            node_kind: "hierarchy".to_string(),
            name: "ECS Prototype".to_string(),
            display_name: "ECS Prototype".to_string(),
            property_type: "GameObject".to_string(),
            field_type_full_name: "UnityEngine.GameObject".to_string(),
            display_value: "(EloraEcsPrototypeBridge) [Layer:Ground]".to_string(),
            has_children: true,
            visible_child_count: 1,
            children: vec![target_dummy],
            ..Default::default()
        };
        let root = UnitySerializedPropertySnapshot {
            semantic_path: "Assets/Scenes/Arena.unity".to_string(),
            node_kind: "scene".to_string(),
            name: "Arena".to_string(),
            display_name: "Arena".to_string(),
            property_type: "Scene".to_string(),
            field_type_full_name: "UnityEngine.SceneManagement.Scene".to_string(),
            has_children: true,
            visible_child_count: 1,
            children: vec![ecs],
            ..Default::default()
        };

        let output = format_property_tree(&root);
        assert!(output.contains("└─ ECS Prototype (EloraEcsPrototypeBridge) [Layer:Ground]"));
        assert!(output.contains("└─ TargetDummy (MeshFilter, MeshRenderer) [Layer:Friend]"));

        let matches = search_property_tree_snapshot(
            &root,
            "Assets/Scenes/Arena.unity",
            &PropertyTreeSearchOptions {
                query: "TargetDummy".to_string(),
                match_fields: vec!["all".to_string()],
                limit: 20,
            },
        )
        .expect("search live scene snapshot");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, target_dummy_path);
    }

    #[test]
    fn path_search_keeps_the_shallowest_match_and_preserves_field_evidence() {
        let kalan_path = "Assets/Scenes/Arena.unity/ECS Prototype/KalanECSPrototype";
        let target_dummy_path = format!("{}/TargetDummy", kalan_path);
        let target_name_path = format!("{}/targetName", kalan_path);
        let root = UnitySerializedPropertySnapshot {
            semantic_path: "Assets/Scenes/Arena.unity".to_string(),
            node_kind: "scene".to_string(),
            name: "Arena".to_string(),
            display_name: "Arena".to_string(),
            property_type: "Scene".to_string(),
            has_children: true,
            children: vec![UnitySerializedPropertySnapshot {
                semantic_path: kalan_path.to_string(),
                node_kind: "hierarchy".to_string(),
                name: "KalanECSPrototype".to_string(),
                display_name: "KalanECSPrototype".to_string(),
                property_type: "GameObject".to_string(),
                display_value: "(EloraEcsPrototypeBridge) [Layer:Ground]".to_string(),
                has_children: true,
                children: vec![
                    UnitySerializedPropertySnapshot {
                        semantic_path: target_dummy_path.clone(),
                        node_kind: "hierarchy".to_string(),
                        name: "TargetDummy".to_string(),
                        display_name: "TargetDummy".to_string(),
                        property_type: "GameObject".to_string(),
                        ..Default::default()
                    },
                    UnitySerializedPropertySnapshot {
                        semantic_path: target_name_path.clone(),
                        node_kind: "property".to_string(),
                        name: "targetName".to_string(),
                        display_name: "Target Name".to_string(),
                        property_type: "String".to_string(),
                        display_value: "KalanECSPrototype".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let options = PropertyTreeSearchOptions {
            query: "KalanECSPrototype".to_string(),
            match_fields: vec!["all".to_string()],
            limit: 20,
        };
        let matches = search_property_tree_snapshot(&root, "Assets/Scenes/Arena.unity", &options)
            .expect("search shallow path and fields");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, kalan_path);
        assert!(matches[0].evidence.path);
        assert!(matches[0].evidence.field_name);
        assert_eq!(matches[1].path, target_name_path);
        assert!(!matches[1].evidence.path);
        assert!(matches[1].evidence.field_value);
        assert!(!matches.iter().any(|item| item.path == target_dummy_path));

        let output = format_property_tree_search_results(
            "Assets/Scenes/Arena.unity",
            &matches,
            options.limit,
        );
        assert!(output.contains("Path matches:\n"));
        assert!(output.contains("Field matches:\n"));
        assert!(output.contains("Matched field_name: KalanECSPrototype"));
        assert!(output.contains("Matched field_value: Target Name = KalanECSPrototype"));
        assert!(!output.contains(&target_dummy_path));
    }

    #[test]
    fn field_search_limit_is_applied_after_path_deduplication() {
        let make_field = |name: &str| UnitySerializedPropertySnapshot {
            semantic_path: format!("Assets/Data.asset/{}", name),
            name: name.to_string(),
            display_name: name.to_string(),
            property_type: "String".to_string(),
            display_value: format!("Hit {}", name),
            ..Default::default()
        };
        let root = UnitySerializedPropertySnapshot {
            semantic_path: "Assets/Data.asset".to_string(),
            children: vec![
                make_field("first"),
                make_field("second"),
                make_field("third"),
            ],
            ..Default::default()
        };
        let options = PropertyTreeSearchOptions {
            query: "Hit".to_string(),
            match_fields: vec!["field_value".to_string()],
            limit: 2,
        };
        let matches = search_property_tree_snapshot(&root, "Assets/Data.asset", &options)
            .expect("search limited fields");
        assert_eq!(matches.len(), 3, "one extra match signals truncation");

        let output =
            format_property_tree_search_results("Assets/Data.asset", &matches, options.limit);
        assert!(output.contains("Assets/Data.asset/first"));
        assert!(output.contains("Assets/Data.asset/second"));
        assert!(!output.contains("Assets/Data.asset/third"));
        assert!(output.contains("search limit 2 reached"));
    }

    #[test]
    fn parses_asset_qualified_paths_without_dollar_root() {
        let parsed = path("Assets/Actions/LightNormalAttack1.asset/hitTrack/clips/1");
        assert_eq!(parsed.asset_path, "Assets/Actions/LightNormalAttack1.asset");
        assert_eq!(parsed.segments, ["hitTrack", "clips", "1"]);
        assert_eq!(
            parsed.full_path(),
            "Assets/Actions/LightNormalAttack1.asset/hitTrack/clips/1"
        );
    }

    #[test]
    fn read_expands_internal_documents_at_their_first_serialized_field() {
        let tree = parse_tree();
        let snapshot = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset"),
                AGENT_PROPERTY_TREE_DEFAULT_DEPTH,
            )
            .expect("read root");
        let hit_track = snapshot
            .children
            .iter()
            .find(|child| child.name == "hitTrack")
            .expect("hitTrack child");
        assert_eq!(
            hit_track.semantic_path,
            "Assets/Actions/LightNormalAttack1.asset/hitTrack"
        );
        assert!(hit_track.children.iter().any(|child| child.name == "clips"));
        assert!(!snapshot.children.iter().any(|child| child.name == "m_Name"));
    }

    #[test]
    fn subassets_render_in_a_separate_addressable_directory() {
        let yaml = r#"--- !u!114 &1
MonoBehaviour:
  m_Name: Primary
  value: 1
--- !u!114 &2
MonoBehaviour:
  m_Name: Detached
  value: 2
"#;
        let tree =
            YamlPropertyTree::parse("Assets/MultipleRoots.asset", yaml, None, &HashMap::new())
                .expect("parse independent roots");
        let root = tree
            .read(&path("Assets/MultipleRoots.asset"), 2)
            .expect("read roots");
        assert!(root.children.iter().any(|child| child.name == "value"));
        assert!(!root.children.iter().any(|child| child.name == "Detached"));
        assert_eq!(root.subassets.len(), 1);
        assert_eq!(root.subassets[0].segment, "Detached");

        let output = format_property_tree(&root);
        assert!(output.contains("--- Subassets [1] ---\n  Detached (MonoBehaviour)"));

        let detached = tree
            .read(&path("Assets/MultipleRoots.asset/Detached/value"), 1)
            .expect("read detached root field");
        assert_eq!(detached.display_value, "2");

        let matches = tree
            .search(
                &path("Assets/MultipleRoots.asset"),
                &PropertyTreeSearchOptions {
                    query: "Detached".to_string(),
                    match_fields: vec!["all".to_string()],
                    limit: 10,
                },
            )
            .expect("search subasset directory");
        assert_eq!(matches[0].path, "Assets/MultipleRoots.asset/Detached");
        tree.read(&path(&matches[0].path), 1)
            .expect("read subasset search path");
    }

    #[test]
    fn reachable_timeline_documents_are_also_listed_as_subassets() {
        let tree = parse_tree();
        let root = tree
            .read(&path("Assets/Actions/LightNormalAttack1.asset"), 1)
            .expect("read timeline root");
        assert_eq!(
            root.subassets
                .iter()
                .map(|entry| entry.segment.as_str())
                .collect::<Vec<_>>(),
            vec!["HitTrack"]
        );
        assert_eq!(
            root.subassets[0]
                .children
                .iter()
                .map(|entry| entry.segment.as_str())
                .collect::<Vec<_>>(),
            vec!["HitBoxClipA", "HitBoxClipB"]
        );
        let output = format_property_tree(&root);
        assert!(output.contains(
            "--- Subassets [3] ---\n  HitTrack (MonoBehaviour)\n  ├─ HitBoxClipA (MonoBehaviour)\n  └─ HitBoxClipB (MonoBehaviour)"
        ));

        let matches = tree
            .search(
                &path("Assets/Actions/LightNormalAttack1.asset"),
                &PropertyTreeSearchOptions {
                    query: "HitBoxClip".to_string(),
                    match_fields: vec!["field_name,type".to_string()],
                    limit: 10,
                },
            )
            .expect("search timeline subassets");
        assert_eq!(
            matches
                .iter()
                .take(2)
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Assets/Actions/LightNormalAttack1.asset/HitTrack/HitBoxClipA",
                "Assets/Actions/LightNormalAttack1.asset/HitTrack/HitBoxClipB"
            ]
        );
        let kind = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset/HitTrack/HitBoxClipA/kind"),
                1,
            )
            .expect("read catalog alias");
        assert_eq!(kind.display_value, "AttackWindow");
    }

    #[test]
    fn first_serialized_reference_claims_shared_subasset_without_field_name_rules() {
        let yaml = r#"--- !u!114 &11400000
MonoBehaviour:
  m_Name: Main
  first: {fileID: 2}
  second: {fileID: 3}
--- !u!114 &2
MonoBehaviour:
  m_Name: First
  arbitraryForwardSlot: {fileID: 4}
--- !u!114 &3
MonoBehaviour:
  m_Name: Second
  anotherReference: {fileID: 4}
--- !u!114 &4
MonoBehaviour:
  m_Name: Shared
  arbitraryBackSlot: {fileID: 2}
  value: 7
"#;
        let tree = YamlPropertyTree::parse("Assets/Shared.asset", yaml, None, &HashMap::new())
            .expect("parse shared graph");
        let root = tree
            .read(&path("Assets/Shared.asset"), 1)
            .expect("read shared graph");

        assert_eq!(
            root.subassets
                .iter()
                .map(|entry| entry.segment.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
        assert_eq!(root.subassets[0].children[0].segment, "Shared");
        assert!(root.subassets[1].children.is_empty());

        let value = tree
            .read(&path("Assets/Shared.asset/First/Shared/value"), 1)
            .expect("read nested shared subasset path");
        assert_eq!(value.display_value, "7");
    }

    #[test]
    fn arrays_are_hard_limited_but_later_items_remain_addressable() {
        let tree = parse_tree();
        let root = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset"),
                AGENT_PROPERTY_TREE_MAX_DEPTH,
            )
            .expect("read root");
        let motion = root
            .children
            .iter()
            .find(|child| child.name == "bakedRootMotion")
            .expect("motion array");
        assert_eq!(motion.array_size, 5);
        assert_eq!(motion.children.len(), AGENT_PROPERTY_TREE_ARRAY_LIMIT);
        assert!(motion.children_truncated);

        let fifth = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset/bakedRootMotion/4"),
                2,
            )
            .expect("read fifth item");
        assert_eq!(fifth.display_value, "4");
    }

    #[test]
    fn array_omission_is_rendered_as_the_final_tree_child() {
        let tree = parse_tree();
        let snapshot = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset/bakedRootMotion"),
                1,
            )
            .expect("read array preview");
        let output = format_property_tree(&snapshot);
        let lines = output.lines().collect::<Vec<_>>();

        assert_eq!(
            lines.first().copied(),
            Some("Assets/Actions/LightNormalAttack1.asset/bakedRootMotion [5]")
        );
        assert_eq!(lines.last().copied(), Some("└─ … +1"));
        assert!(lines.iter().any(|line| line.starts_with("├─ 3")));
    }

    #[test]
    fn short_tree_auto_expansion_is_complete_and_prunes_at_the_character_budget() {
        let tree = parse_tree();
        let asset_path = path("Assets/Actions/LightNormalAttack1.asset");
        let complete = tree
            .read_complete_within_budget(&asset_path, AGENT_PROPERTY_TREE_AUTO_EXPAND_CHAR_LIMIT)
            .expect("read complete candidate")
            .expect("timeline fits auto-expand budget");

        assert_eq!(complete.max_array_items, AGENT_PROPERTY_TREE_ARRAY_LIMIT);
        assert!(complete.output.contains("└─ … +1"));
        assert!(!complete.output.contains("4: 4"));
        assert!(format_complete_property_tree_within_budget(
            &complete.snapshot,
            AGENT_PROPERTY_TREE_AUTO_EXPAND_CHAR_LIMIT,
        )
        .is_some());

        let exact_chars = complete.output.chars().count();
        assert!(tree
            .read_complete_within_budget(&asset_path, exact_chars)
            .expect("exact budget")
            .is_some());
        assert!(tree
            .read_complete_within_budget(&asset_path, exact_chars - 1)
            .expect("pruned budget")
            .is_none());
    }

    #[test]
    fn compact_outline_prints_the_full_path_once_without_metadata_headers() {
        let tree = parse_tree();
        let snapshot = tree
            .read(
                &path("Assets/Actions/LightNormalAttack1.asset"),
                AGENT_PROPERTY_TREE_DEFAULT_DEPTH,
            )
            .expect("read root");
        let output = format_property_tree(&snapshot);
        assert_eq!(
            output
                .matches("Assets/Actions/LightNormalAttack1.asset")
                .count(),
            1
        );
        assert!(!output.contains("UNITY_YAML_OUTLINE"));
        assert!(!output.contains("revision:"));
        assert!(output.contains("├─ hitTrack"));
        assert!(output.contains("bakedRootMotion [5]"));
    }

    #[test]
    fn zero_depth_returns_only_the_selected_node_boundary() {
        let tree = parse_tree();
        let snapshot = tree
            .read(&path("Assets/Actions/LightNormalAttack1.asset/hitTrack"), 0)
            .expect("read boundary");
        assert!(snapshot.children.is_empty());
        assert_eq!(snapshot.visible_child_count, 0);
        assert!(snapshot.children_truncated);
    }

    #[test]
    fn search_paths_round_trip_into_read() {
        let tree = parse_tree();
        let scope = path("Assets/Actions/LightNormalAttack1.asset/hitTrack");
        let matches = tree
            .search(
                &scope,
                &PropertyTreeSearchOptions {
                    query: "AttackWindow".to_string(),
                    match_fields: vec!["field_value".to_string()],
                    limit: 50,
                },
            )
            .expect("search");
        assert_eq!(matches.len(), 1);
        let match_path = path(&matches[0].path);
        let snapshot = tree.read(&match_path, 2).expect("read match path");
        assert_eq!(snapshot.display_value, "AttackWindow");
    }

    #[test]
    fn cycles_point_to_the_first_canonical_path() {
        let yaml = r#"--- !u!114 &1
MonoBehaviour:
  m_Name: Root
  child: {fileID: 2}
--- !u!114 &2
MonoBehaviour:
  m_Name: Child
  parent: {fileID: 1}
"#;
        let tree = YamlPropertyTree::parse("Assets/Cycle.asset", yaml, None, &HashMap::new())
            .expect("parse cycle");
        let snapshot = tree
            .read(&path("Assets/Cycle.asset"), 4)
            .expect("read cycle");
        let parent = &snapshot.children[0].children[0];
        assert_eq!(parent.canonical_path, "Assets/Cycle.asset");
        assert!(parent.children.is_empty());
    }

    #[test]
    fn direct_reads_keep_the_asset_global_first_definition_path() {
        let yaml = r#"--- !u!114 &1
MonoBehaviour:
  m_Name: Root
  deep:
    holder:
      value: {fileID: 2}
  repeated: {fileID: 2}
--- !u!114 &2
MonoBehaviour:
  m_Name: Shared
  amount: 8
"#;
        let tree = YamlPropertyTree::parse("Assets/Shared.asset", yaml, None, &HashMap::new())
            .expect("parse shared object");
        let repeated = tree
            .read(&path("Assets/Shared.asset/repeated"), 4)
            .expect("read repeated reference directly");
        assert_eq!(
            repeated.canonical_path,
            "Assets/Shared.asset/deep/holder/value"
        );
        assert!(repeated.children.is_empty());

        let canonical = tree
            .read(&path("Assets/Shared.asset/deep/holder/value"), 1)
            .expect("read canonical reference directly");
        assert!(canonical.canonical_path.is_empty());
        assert!(canonical
            .children
            .iter()
            .any(|child| child.name == "amount"));
    }

    #[test]
    fn managed_references_expand_at_the_serialized_field_path() {
        let yaml = r#"--- !u!114 &1
MonoBehaviour:
  m_Name: Root
  action:
    rid: 42
  references:
    version: 2
    RefIds:
    - rid: 42
      type: {class: HitAction, ns: Game.Combat, asm: Assembly-CSharp}
      data:
        damage: 18
        window: Attack
"#;
        let tree = YamlPropertyTree::parse("Assets/Managed.asset", yaml, None, &HashMap::new())
            .expect("parse managed reference");
        let snapshot = tree
            .read(&path("Assets/Managed.asset/action"), 2)
            .expect("read managed reference");
        assert_eq!(snapshot.field_type_full_name, "Game.Combat.HitAction");
        assert!(snapshot.children.iter().any(|child| child.name == "damage"));
        assert_eq!(snapshot.semantic_path, "Assets/Managed.asset/action");
    }

    const PREFAB_YAML: &str = r#"--- !u!1 &1
GameObject:
  m_Component:
  - component: {fileID: 2}
  - component: {fileID: 3}
  m_Name: Hero
--- !u!4 &2
Transform:
  m_GameObject: {fileID: 1}
  m_Father: {fileID: 0}
  m_Children:
  - {fileID: 5}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalPosition: {x: 1, y: 2, z: 3}
  m_LocalScale: {x: 1, y: 1, z: 1}
--- !u!114 &3
MonoBehaviour:
  m_GameObject: {fileID: 1}
  m_Name: Combat
  data:
    damage: 24
--- !u!1 &4
GameObject:
  m_Component:
  - component: {fileID: 5}
  m_Name: Child
--- !u!4 &5
Transform:
  m_GameObject: {fileID: 4}
  m_Father: {fileID: 2}
  m_Children: []
"#;

    #[test]
    fn prefab_root_flattens_components_and_keeps_child_gameobjects() {
        let tree =
            YamlPropertyTree::parse("Assets/Hero.prefab", PREFAB_YAML, None, &HashMap::new())
                .expect("parse prefab");
        let snapshot = tree
            .read(&path("Assets/Hero.prefab"), 2)
            .expect("read prefab root");
        let names = snapshot
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"GameObject"));
        assert!(names.contains(&"Transform"));
        assert!(names.contains(&"MonoBehaviour"));
        assert!(names.contains(&"Child"));

        let damage_path = path("Assets/Hero.prefab/MonoBehaviour/data/damage");
        let damage = tree.read(&damage_path, 2).expect("read component field");
        assert_eq!(damage.display_value, "24");
    }

    #[test]
    fn native_transform_vectors_are_inline_at_the_depth_boundary() {
        let tree =
            YamlPropertyTree::parse("Assets/Hero.prefab", PREFAB_YAML, None, &HashMap::new())
                .expect("parse prefab");
        let snapshot = tree
            .read(&path("Assets/Hero.prefab"), 2)
            .expect("read prefab root");
        let output = format_property_tree(&snapshot);

        assert!(output.contains("m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}"));
        assert!(output.contains("m_LocalPosition: {x: 1, y: 2, z: 3}"));
        assert!(output.contains("m_LocalScale: {x: 1, y: 1, z: 1}"));
        assert!(!output.contains("m_LocalPosition …"));
    }

    #[test]
    fn scene_search_returns_asset_qualified_readable_paths() {
        let tree =
            YamlPropertyTree::parse("Assets/Arena.unity", PREFAB_YAML, None, &HashMap::new())
                .expect("parse scene");
        let scope = path("Assets/Arena.unity/Hero");
        let matches = tree
            .search(
                &scope,
                &PropertyTreeSearchOptions {
                    query: "damage".to_string(),
                    match_fields: vec!["field_name".to_string()],
                    limit: 50,
                },
            )
            .expect("search scene");
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            "Assets/Arena.unity/Hero/MonoBehaviour/data/damage"
        );
        let read_back = tree
            .read(&path(&matches[0].path), 1)
            .expect("read search result");
        assert_eq!(read_back.display_value, "24");
    }

    #[test]
    fn prefab_root_searches_component_fields_with_readable_paths() {
        let tree =
            YamlPropertyTree::parse("Assets/Hero.prefab", PREFAB_YAML, None, &HashMap::new())
                .expect("parse prefab");
        let matches = tree
            .search(
                &path("Assets/Hero.prefab"),
                &PropertyTreeSearchOptions {
                    query: "damage".to_string(),
                    match_fields: vec!["field_name".to_string()],
                    limit: 50,
                },
            )
            .expect("search prefab root");

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            "Assets/Hero.prefab/MonoBehaviour/data/damage"
        );
        let read_back = tree
            .read(&path(&matches[0].path), 1)
            .expect("read prefab search result");
        assert_eq!(read_back.display_value, "24");
    }

    #[test]
    fn scene_root_searches_component_fields_and_component_types() {
        let tree =
            YamlPropertyTree::parse("Assets/Arena.unity", PREFAB_YAML, None, &HashMap::new())
                .expect("parse scene");
        let field_matches = tree
            .search(
                &path("Assets/Arena.unity"),
                &PropertyTreeSearchOptions {
                    query: "damage".to_string(),
                    match_fields: vec!["field_name".to_string()],
                    limit: 50,
                },
            )
            .expect("search scene root field");
        assert_eq!(field_matches.len(), 1);
        assert_eq!(
            field_matches[0].path,
            "Assets/Arena.unity/Hero/MonoBehaviour/data/damage"
        );

        let type_matches = tree
            .search(
                &path("Assets/Arena.unity"),
                &PropertyTreeSearchOptions {
                    query: "MonoBehaviour".to_string(),
                    match_fields: vec!["type".to_string()],
                    limit: 50,
                },
            )
            .expect("search scene root component type");
        assert!(type_matches
            .iter()
            .any(|item| item.path == "Assets/Arena.unity/Hero/MonoBehaviour"));
    }
}
