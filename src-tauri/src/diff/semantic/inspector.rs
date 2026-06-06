use super::list_matching::stable_list_match;
use super::list_matching::{build_list_items, ListMatchPair, ParsedFieldLineIR};
use super::material::{parse_shader_properties, restructure_material_fields};
use super::parse::*;
use super::script::{
    doc_script_guid, load_script_semantic_info, load_side_text_file, resolve_all_field_types,
    ScriptSemanticInfo,
};
use super::{unity_class_name, FieldTreeNode, HierarchyEntry, ParsedFieldLine, SemanticBuildEnv};
use crate::diff::context::SideContext;
use crate::diff::types::*;
use crate::unity_yaml::{build_hierarchy_path_map, HierarchyNode, YamlDoc};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};

pub(crate) fn field_label_for_path(path: &str, script_info: Option<&ScriptSemanticInfo>) -> String {
    let segments = split_property_path(path);
    let Some(last_segment) = segments.last() else {
        return path.to_string();
    };
    if last_segment.starts_with('[') {
        return last_segment.clone();
    }

    if segments.len() == 1 {
        if let Some(info) = script_info {
            if let Some(field) = info.field_aliases.get(last_segment) {
                return field.display_label.clone();
            }
        }
    }

    if script_info.is_some() {
        prettify_field_label(last_segment)
    } else {
        prettify_builtin_field_label(last_segment)
    }
}

pub(crate) fn apply_field_label_enhancements(
    field_map: &mut HashMap<String, ParsedFieldLine>,
    script_info: Option<&ScriptSemanticInfo>,
) {
    for (path, entry) in field_map.iter_mut() {
        entry.label = field_label_for_path(path, script_info);
    }
}

/// Compare two references by semantic identity (guid + fileID), ignoring display path.
/// This is necessary because source isolation means old-side (snapshot) won't resolve
/// GUID paths while new-side (workspace) will, causing false "modified" results.
fn references_semantically_equal(
    a: Option<&InspectorReference>,
    b: Option<&InspectorReference>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.guid == b.guid && a.file_id == b.file_id,
        (None, None) => true,
        _ => false,
    }
}

fn path_change_kind(old: Option<&ParsedFieldLine>, new: Option<&ParsedFieldLine>) -> &'static str {
    match (old, new) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        (Some(old), Some(new)) => {
            let values_equal = old.value == new.value
                || (old.reference.is_some()
                    && new.reference.is_some()
                    && references_semantically_equal(
                        old.reference.as_ref(),
                        new.reference.as_ref(),
                    ));
            let refs_equal =
                references_semantically_equal(old.reference.as_ref(), new.reference.as_ref());
            if values_equal && refs_equal {
                "unchanged"
            } else {
                "modified"
            }
        }
        (None, None) => "unchanged",
    }
}

fn infer_value_type(
    old: Option<&ParsedFieldLine>,
    new: Option<&ParsedFieldLine>,
    has_children: bool,
) -> String {
    if has_children {
        return "group".into();
    }
    if old.and_then(|entry| entry.reference.as_ref()).is_some()
        || new.and_then(|entry| entry.reference.as_ref()).is_some()
    {
        return "reference".into();
    }
    let value = new
        .and_then(|entry| entry.value.as_ref())
        .or_else(|| old.and_then(|entry| entry.value.as_ref()));
    match value {
        Some(value)
            if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") =>
        {
            "bool".into()
        }
        Some(value) if value.parse::<f64>().is_ok() => "number".into(),
        Some(value) if value.starts_with('[') && value.ends_with(']') => "array".into(),
        _ => "string".into(),
    }
}

fn insert_field_node(
    root: &mut FieldTreeNode,
    path: &str,
    old_map: &HashMap<String, ParsedFieldLine>,
    new_map: &HashMap<String, ParsedFieldLine>,
) {
    let segments = split_property_path(path);
    let mut cursor = root;
    let mut current_segments = Vec::new();

    for segment in segments {
        current_segments.push(segment.clone());
        let current_path = join_property_segments(&current_segments);
        cursor = cursor
            .children
            .entry(segment.clone())
            .or_insert_with(|| FieldTreeNode {
                label: segment.clone(),
                path: current_path.clone(),
                old_entry: old_map.get(&current_path).cloned(),
                new_entry: new_map.get(&current_path).cloned(),
                children: IndexMap::new(),
            });
        if cursor.old_entry.is_none() {
            cursor.old_entry = old_map.get(&current_path).cloned();
        }
        if cursor.new_entry.is_none() {
            cursor.new_entry = new_map.get(&current_path).cloned();
        }
    }
}

fn build_field_from_node(
    node: FieldTreeNode,
    include_unchanged: bool,
    script_info: Option<&ScriptSemanticInfo>,
    field_type_map: &HashMap<String, String>,
) -> Option<InspectorField> {
    build_field_from_node_ref(&node, include_unchanged, script_info, field_type_map)
}

/// Borrow-based version of `build_field_from_node` that doesn't consume the tree.
/// Allows the same tree to be traversed twice (once for changed, once for full).
fn build_field_from_node_ref(
    node: &FieldTreeNode,
    include_unchanged: bool,
    script_info: Option<&ScriptSemanticInfo>,
    field_type_map: &HashMap<String, String>,
) -> Option<InspectorField> {
    let mut children = Vec::new();
    for (_, child) in &node.children {
        if let Some(field) =
            build_field_from_node_ref(child, include_unchanged, script_info, field_type_map)
        {
            children.push(field);
        }
    }

    let direct_kind = path_change_kind(node.old_entry.as_ref(), node.new_entry.as_ref());
    let change_kind = if direct_kind != "unchanged" {
        direct_kind.to_string()
    } else if children.is_empty() {
        "unchanged".to_string()
    } else if children
        .iter()
        .all(|child| child.change_kind == "unchanged")
    {
        "unchanged".to_string()
    } else if children.iter().all(|child| child.change_kind == "added") {
        "added".to_string()
    } else if children.iter().all(|child| child.change_kind == "removed") {
        "removed".to_string()
    } else {
        "modified".to_string()
    };

    if !include_unchanged && change_kind == "unchanged" {
        return None;
    }

    let old_entry = node.old_entry.as_ref();
    let new_entry = node.new_entry.as_ref();
    let label = new_entry
        .map(|entry| entry.label.clone())
        .or_else(|| old_entry.map(|entry| entry.label.clone()))
        .unwrap_or_else(|| node.label.clone());

    // Look up C# declared type: prefer pre-resolved map (supports nested), fallback to top-level
    let field_type = field_type_map.get(&node.path).cloned().or_else(|| {
        script_info.and_then(|info| {
            let segments = split_property_path(&node.path);
            if segments.len() == 1 {
                info.field_aliases
                    .get(&segments[0])
                    .and_then(|enh| enh.field_type.clone())
            } else {
                None
            }
        })
    });

    Some(InspectorField {
        id: format!("{}:{}", node.path, change_kind),
        label,
        property_path: node.path.clone(),
        value_type: infer_value_type(old_entry, new_entry, !children.is_empty()),
        change_kind,
        before: old_entry.and_then(|entry| entry.value.clone()),
        after: new_entry.and_then(|entry| entry.value.clone()),
        children,
        reference: new_entry
            .and_then(|entry| entry.reference.clone())
            .or_else(|| old_entry.and_then(|entry| entry.reference.clone())),
        field_type,
    })
}

/// Fields hidden in asset inspector (matching Unity Inspector behavior).
pub(crate) const HIDDEN_ASSET_FIELDS: &[&str] = &[
    "m_ObjectHideFlags",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "m_EditorClassIdentifier",
    "m_Script",
];

pub(crate) fn build_inspector_fields(
    old_map: &HashMap<String, ParsedFieldLine>,
    new_map: &HashMap<String, ParsedFieldLine>,
    include_unchanged: bool,
    hidden_roots: HashSet<String>,
    script_info: Option<&ScriptSemanticInfo>,
    field_type_map: &HashMap<String, String>,
) -> Vec<InspectorField> {
    let mut all_paths: Vec<String> = old_map.keys().chain(new_map.keys()).cloned().collect();
    all_paths.sort_unstable();
    all_paths.dedup();
    // Build the full visible tree first, then filter unchanged nodes only after
    // stable list matching. Filtering by raw index before remapping causes
    // shifted list items to lose unchanged sibling fields, which turns clean
    // insertions into noisy added/removed groups in changed-only view.
    let relevant: Vec<String> = if hidden_roots.is_empty() {
        all_paths
    } else {
        all_paths
            .into_iter()
            .filter(|path| {
                let root_field = path.split('.').next().unwrap_or(path);
                let root_field = root_field.split('[').next().unwrap_or(root_field);
                !hidden_roots.contains(root_field)
            })
            .collect()
    };

    let mut root = FieldTreeNode::default();
    for path in &relevant {
        insert_field_node(&mut root, path, old_map, new_map);
    }

    // Phase 3: Apply stable list matching to array parent nodes
    apply_stable_list_matching(&mut root, old_map, new_map);

    let mut result = Vec::new();
    for (_, child) in root.children {
        if let Some(field) =
            build_field_from_node(child, include_unchanged, script_info, field_type_map)
        {
            result.push(field);
        }
    }
    result
}

/// Build both changed and full field lists from a single tree construction + list matching pass.
/// The tree is built once and traversed twice, saving ~40% of field construction work.
pub(crate) fn build_inspector_fields_pair(
    old_map: &HashMap<String, ParsedFieldLine>,
    new_map: &HashMap<String, ParsedFieldLine>,
    hidden_roots: HashSet<String>,
    script_info: Option<&ScriptSemanticInfo>,
    field_type_map: &HashMap<String, String>,
) -> (Vec<InspectorField>, Vec<InspectorField>) {
    let mut all_paths: Vec<String> = old_map.keys().chain(new_map.keys()).cloned().collect();
    all_paths.sort_unstable();
    all_paths.dedup();

    let relevant: Vec<String> = if hidden_roots.is_empty() {
        all_paths
    } else {
        all_paths
            .into_iter()
            .filter(|path| {
                let root_field = path.split('.').next().unwrap_or(path);
                let root_field = root_field.split('[').next().unwrap_or(root_field);
                !hidden_roots.contains(root_field)
            })
            .collect()
    };

    let mut root = FieldTreeNode::default();
    for path in &relevant {
        insert_field_node(&mut root, path, old_map, new_map);
    }

    apply_stable_list_matching(&mut root, old_map, new_map);

    // Traverse the tree twice: once for changed-only, once for full
    let mut changed = Vec::new();
    let mut full = Vec::new();
    for (_, child) in &root.children {
        if let Some(field) = build_field_from_node_ref(child, false, script_info, field_type_map) {
            changed.push(field);
        }
        if let Some(field) = build_field_from_node_ref(child, true, script_info, field_type_map) {
            full.push(field);
        }
    }
    (changed, full)
}

/// Detect array parent nodes in the tree and apply stable list matching.
/// An array parent is a FieldTreeNode whose children are all `[N]` keyed.
fn apply_stable_list_matching(
    node: &mut FieldTreeNode,
    _old_map: &HashMap<String, ParsedFieldLine>,
    _new_map: &HashMap<String, ParsedFieldLine>,
) {
    for (_, child) in node.children.iter_mut() {
        apply_stable_list_matching(child, _old_map, _new_map);
    }

    if node.children.len() <= 1 || !node.children.keys().all(|key| is_array_key(key)) {
        return;
    }

    let child_entries = sorted_array_children(&node.children);
    let old_sources = build_side_list_items(&child_entries, FieldSide::Old);
    let new_sources = build_side_list_items(&child_entries, FieldSide::New);
    if old_sources.is_empty() || new_sources.is_empty() {
        return;
    }

    let old_items = build_list_items(
        &old_sources
            .iter()
            .map(|(_, value, reference, fields)| (value.clone(), reference.clone(), fields.clone()))
            .collect::<Vec<_>>(),
    );
    let new_items = build_list_items(
        &new_sources
            .iter()
            .map(|(_, value, reference, fields)| (value.clone(), reference.clone(), fields.clone()))
            .collect::<Vec<_>>(),
    );
    if !old_items
        .iter()
        .chain(new_items.iter())
        .any(|item| !item.match_key.is_empty())
    {
        return;
    }

    let match_pairs = stable_list_match(&old_items, &new_items);
    let old_children = std::mem::take(&mut node.children);
    let mut remapped_children = IndexMap::new();

    for (pair_idx, pair) in match_pairs.iter().enumerate() {
        let key = format!("[{}]", pair_idx);
        let path = if node.path.is_empty() {
            key.clone()
        } else {
            format!("{}{}", node.path, key)
        };
        let label = semantic_list_label(pair, pair_idx);
        let old_source = pair
            .old_index
            .and_then(|index| old_sources.get(index))
            .and_then(|(source_key, _, _, _)| old_children.get(source_key));
        let new_source = pair
            .new_index
            .and_then(|index| new_sources.get(index))
            .and_then(|(source_key, _, _, _)| old_children.get(source_key));
        let merged = merge_list_item_nodes(old_source, new_source, &path, &label);
        let merged = collapse_single_child_item(merged);
        remapped_children.insert(key, merged);
    }

    // Unmatched old → removed
    // Unmatched new → added

    if !remapped_children.is_empty() {
        node.children = remapped_children;
    }
}

#[derive(Clone, Copy)]
enum FieldSide {
    Old,
    New,
}

fn is_array_key(key: &str) -> bool {
    key.starts_with('[') && key.ends_with(']')
}

fn array_key_index(key: &str) -> Option<usize> {
    key.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

fn sorted_array_children(
    children: &IndexMap<String, FieldTreeNode>,
) -> Vec<(&str, &FieldTreeNode)> {
    let mut entries: Vec<(&str, &FieldTreeNode)> = children
        .iter()
        .map(|(key, child)| (key.as_str(), child))
        .collect();
    entries.sort_by_key(|(key, _)| array_key_index(key).unwrap_or(usize::MAX));
    entries
}

fn build_side_list_items(
    children: &[(&str, &FieldTreeNode)],
    side: FieldSide,
) -> Vec<(
    String,
    Option<String>,
    Option<InspectorReference>,
    IndexMap<String, ParsedFieldLineIR>,
)> {
    let mut items = Vec::new();
    for (key, child) in children {
        if !node_has_side_data(child, side) {
            continue;
        }
        items.push((
            (*key).to_string(),
            node_entry_for_side(child, side).and_then(|entry| entry.value.clone()),
            node_entry_for_side(child, side).and_then(|entry| entry.reference.clone()),
            collect_relative_item_fields(child, side),
        ));
    }
    items
}

fn node_entry_for_side<'a>(
    node: &'a FieldTreeNode,
    side: FieldSide,
) -> Option<&'a ParsedFieldLine> {
    match side {
        FieldSide::Old => node.old_entry.as_ref(),
        FieldSide::New => node.new_entry.as_ref(),
    }
}

fn node_has_side_data(node: &FieldTreeNode, side: FieldSide) -> bool {
    node_entry_for_side(node, side).is_some()
        || node
            .children
            .values()
            .any(|child| node_has_side_data(child, side))
}

fn collect_relative_item_fields(
    node: &FieldTreeNode,
    side: FieldSide,
) -> IndexMap<String, ParsedFieldLineIR> {
    let mut fields = IndexMap::new();
    collect_relative_item_fields_inner(node, node, side, &mut fields);
    fields
}

fn collect_relative_item_fields_inner(
    root: &FieldTreeNode,
    node: &FieldTreeNode,
    side: FieldSide,
    out: &mut IndexMap<String, ParsedFieldLineIR>,
) {
    for child in node.children.values() {
        if !node_has_side_data(child, side) {
            continue;
        }
        if let Some(entry) = node_entry_for_side(child, side) {
            if entry.value.is_some() || entry.reference.is_some() {
                let relative_path = child
                    .path
                    .strip_prefix(&root.path)
                    .unwrap_or(&child.path)
                    .trim_start_matches('.')
                    .to_string();
                out.insert(
                    relative_path,
                    ParsedFieldLineIR {
                        label: entry.label.clone(),
                        value: entry.value.clone(),
                        reference: entry.reference.clone(),
                    },
                );
            }
        }
        collect_relative_item_fields_inner(root, child, side, out);
    }
}

fn semantic_list_label(pair: &ListMatchPair, index: usize) -> String {
    let default_label = format!("[{}]", index);
    match pair.display_label.trim() {
        "" | "(object)" | "(empty)" => default_label,
        other => other.to_string(),
    }
}

fn merge_list_item_nodes(
    old_node: Option<&FieldTreeNode>,
    new_node: Option<&FieldTreeNode>,
    path: &str,
    label: &str,
) -> FieldTreeNode {
    let mut merged = FieldTreeNode {
        label: label.to_string(),
        path: path.to_string(),
        old_entry: old_node.and_then(|node| {
            node.old_entry.as_ref().map(|entry| ParsedFieldLine {
                label: label.to_string(),
                value: entry.value.clone(),
                reference: entry.reference.clone(),
            })
        }),
        new_entry: new_node.and_then(|node| {
            node.new_entry.as_ref().map(|entry| ParsedFieldLine {
                label: label.to_string(),
                value: entry.value.clone(),
                reference: entry.reference.clone(),
            })
        }),
        children: IndexMap::new(),
    };

    for key in merge_child_keys(old_node, new_node) {
        let old_child = old_node.and_then(|node| node.children.get(&key));
        let new_child = new_node.and_then(|node| node.children.get(&key));
        if old_child.is_none() && new_child.is_none() {
            continue;
        }
        let child_label = new_child
            .map(|child| child.label.clone())
            .or_else(|| old_child.map(|child| child.label.clone()))
            .unwrap_or_else(|| key.clone());
        let child_path = if key.starts_with('[') {
            format!("{}{}", path, key)
        } else if path.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", path, key)
        };
        let child_node = merge_list_item_nodes(old_child, new_child, &child_path, &child_label);
        merged.children.insert(key, child_node);
    }

    merged
}

/// Collapse a list item node that has exactly one child and no own value.
/// This flattens structures like:
///   Base Color (group)
///     └── _BaseColor: {r: 1, ...}
/// into:
///   Base Color: {r: 1, ...}
fn collapse_single_child_item(mut node: FieldTreeNode) -> FieldTreeNode {
    if node.children.len() != 1 {
        return node;
    }
    // Only collapse when the parent has no own value (it's a pure group node).
    let parent_has_value = node
        .old_entry
        .as_ref()
        .map(|e| e.value.is_some())
        .unwrap_or(false)
        || node
            .new_entry
            .as_ref()
            .map(|e| e.value.is_some())
            .unwrap_or(false);
    if parent_has_value {
        return node;
    }

    let (_, child) = node.children.into_iter().next().unwrap();
    // Don't collapse if the child itself has sub-children (would lose structure).
    if !child.children.is_empty() {
        node.children = IndexMap::new();
        node.children.insert(child.label.clone(), child);
        return node;
    }

    // Promote child's value/reference into the parent, keeping the parent's label.
    node.old_entry = child.old_entry.map(|mut e| {
        e.label = node.label.clone();
        e
    });
    node.new_entry = child.new_entry.map(|mut e| {
        e.label = node.label.clone();
        e
    });
    node.children = IndexMap::new();
    node
}

fn merge_child_keys(
    old_node: Option<&FieldTreeNode>,
    new_node: Option<&FieldTreeNode>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    if let Some(node) = new_node {
        for key in node.children.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    if let Some(node) = old_node {
        for key in node.children.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    if keys.iter().all(|key| is_array_key(key)) {
        keys.sort_by_key(|key| array_key_index(key).unwrap_or(usize::MAX));
    }
    keys
}

pub(crate) fn count_changed_leaf_fields(fields: &[InspectorField]) -> usize {
    let mut total = 0usize;
    for field in fields {
        if field.children.is_empty() {
            if field.change_kind != "unchanged" {
                total += 1;
            }
        } else {
            total += count_changed_leaf_fields(&field.children);
        }
    }
    total
}

/// Load the shader referenced by a material and parse its Properties block.
/// Returns shader properties for ordering/typing, or empty vec if shader can't be resolved.
fn load_material_shader_properties(
    new_map: &HashMap<String, ParsedFieldLine>,
    old_map: &HashMap<String, ParsedFieldLine>,
    new_ctx: &SideContext,
    old_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> Vec<super::material::ShaderProperty> {
    // Find the m_Shader reference in the field map
    let shader_ref = new_map
        .get("m_Shader")
        .and_then(|f| f.reference.as_ref())
        .or_else(|| old_map.get("m_Shader").and_then(|f| f.reference.as_ref()));

    let Some(reference) = shader_ref else {
        return Vec::new();
    };

    // Try to get the shader file path
    let shader_path = reference.path.as_deref().or_else(|| {
        reference
            .guid
            .as_deref()
            .and_then(|_| reference.path.as_deref())
    });
    let Some(shader_path) = shader_path else {
        return Vec::new();
    };

    // Only process .shader files (not ShaderGraph etc.)
    if !shader_path.ends_with(".shader") {
        return Vec::new();
    }

    // Load shader content (prefer new side)
    let content = load_side_text_file(
        env.cwd,
        shader_path,
        new_ctx,
        env.batch_reader.as_mut(),
        env.profiler,
    )
    .or_else(|| {
        load_side_text_file(
            env.cwd,
            shader_path,
            old_ctx,
            env.batch_reader.as_mut(),
            env.profiler,
        )
    });

    match content {
        Some(source) => parse_shader_properties(&source),
        None => Vec::new(),
    }
}

fn component_resolve_reason(
    panel_kind: &InspectorPanelKind,
    class_id: Option<i32>,
    script_class: Option<&str>,
) -> Option<String> {
    if !matches!(panel_kind, InspectorPanelKind::Component) {
        return None;
    }

    match class_id {
        Some(114) if script_class.is_none() => {
            Some("MonoBehaviour 脚本类名解析失败，已回退显示为 MonoBehaviour".into())
        }
        None => Some("组件缺少 classId，无法解析实际类型名称".into()),
        _ => None,
    }
}

/// Build both changed and full panels in a single pass, sharing all expensive
/// parsing work (parse_doc_field_map, resolve_all_field_types, script loading).
/// Returns (changed_panel, full_panel).
pub(crate) fn build_doc_panel_pair(
    panel_kind: InspectorPanelKind,
    title: String,
    script_class: Option<String>,
    old_doc: Option<&YamlDoc>,
    new_doc: Option<&YamlDoc>,
    old_lines: &[String],
    new_lines: &[String],
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    old_ctx: &SideContext,
    new_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
    class_id: Option<i32>,
) -> (Option<InspectorPanel>, Option<InspectorPanel>) {
    let old_script_info =
        old_doc.and_then(|doc| load_script_semantic_info(doc, old_lines, old_ctx, env));
    let new_script_info =
        new_doc.and_then(|doc| load_script_semantic_info(doc, new_lines, new_ctx, env));

    let mut old_map = old_doc
        .map(|doc| parse_doc_field_map(doc, old_lines, old_ctx, old_labels))
        .unwrap_or_default();
    let mut new_map = new_doc
        .map(|doc| parse_doc_field_map(doc, new_lines, new_ctx, new_labels))
        .unwrap_or_default();
    apply_field_label_enhancements(&mut old_map, old_script_info.as_ref());
    apply_field_label_enhancements(&mut new_map, new_script_info.as_ref());

    let mut hidden_roots = HashSet::new();
    if matches!(
        panel_kind,
        InspectorPanelKind::AssetRoot
            | InspectorPanelKind::SubObject
            | InspectorPanelKind::Component
    ) {
        hidden_roots.extend(HIDDEN_ASSET_FIELDS.iter().map(|value| value.to_string()));
        for info in [&old_script_info, &new_script_info].into_iter().flatten() {
            for (alias, field) in &info.field_aliases {
                if field.hidden {
                    hidden_roots.insert(alias.clone());
                    hidden_roots.insert(field.canonical_name.clone());
                }
            }
        }
    }
    let script_info = new_script_info.as_ref().or(old_script_info.as_ref());

    let hint_script_path = new_doc
        .and_then(|doc| {
            doc_script_guid(doc, new_lines).and_then(|guid| new_ctx.resolve_script_guid_path(&guid))
        })
        .or_else(|| {
            old_doc.and_then(|doc| {
                doc_script_guid(doc, old_lines)
                    .and_then(|guid| old_ctx.resolve_script_guid_path(&guid))
            })
        });
    let all_paths = old_map.keys().chain(new_map.keys());
    let resolve_ctx = new_ctx;
    let field_type_map = resolve_all_field_types(
        all_paths,
        script_info,
        hint_script_path.as_deref(),
        resolve_ctx,
        env,
    );

    let (mut changed_fields, mut full_fields) = build_inspector_fields_pair(
        &old_map,
        &new_map,
        hidden_roots,
        script_info,
        &field_type_map,
    );

    if class_id == Some(21) {
        let shader_props =
            load_material_shader_properties(&new_map, &old_map, new_ctx, old_ctx, env);
        restructure_material_fields(&mut changed_fields, &shader_props);
        restructure_material_fields(&mut full_fields, &shader_props);
    }

    let change_kind = match (old_doc, new_doc) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        _ if changed_fields.is_empty() => "unchanged",
        _ => "modified",
    };

    let component_resolve_reason =
        component_resolve_reason(&panel_kind, class_id, script_class.as_deref());

    let make_panel =
        |fields: Vec<InspectorField>, include_unchanged: bool| -> Option<InspectorPanel> {
            if !include_unchanged && change_kind == "unchanged" {
                return None;
            }
            let component_type = class_id.map(|id| {
                if id == 114 {
                    script_class
                        .clone()
                        .unwrap_or_else(|| "MonoBehaviour".into())
                } else {
                    unity_class_name(id).to_string()
                }
            });
            let component_source = Some(
                match panel_kind {
                    InspectorPanelKind::Component => {
                        if class_id == Some(114) {
                            "script"
                        } else {
                            "builtin"
                        }
                    }
                    InspectorPanelKind::GameObjectHeader => "gameObjectHeader",
                    InspectorPanelKind::AssetRoot => "assetRoot",
                    InspectorPanelKind::SubObject => "subObject",
                }
                .into(),
            );
            Some(InspectorPanel {
                panel_kind: panel_kind.clone(),
                title: title.clone(),
                script_class: script_class.clone(),
                change_kind: change_kind.to_string(),
                added: change_kind == "added",
                removed: change_kind == "removed",
                component_type,
                component_class_id: class_id,
                component_source,
                component_resolve_reason: component_resolve_reason.clone(),
                component_inference: None,
                fields,
            })
        };

    let changed = make_panel(changed_fields, false);
    let full = make_panel(full_fields, true);
    (changed, full)
}

/// Like `build_doc_panel_pair` but uses an immutable `ScriptInfoCache` instead of `&mut SemanticBuildEnv`.
/// Used in the parallel phase where no mutable state is available.
pub(crate) fn build_doc_panel_pair_readonly(
    panel_kind: InspectorPanelKind,
    title: String,
    script_class: Option<String>,
    old_doc: Option<&YamlDoc>,
    new_doc: Option<&YamlDoc>,
    old_lines: &[String],
    new_lines: &[String],
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    old_ctx: &SideContext,
    new_ctx: &SideContext,
    script_cache: &super::script::ScriptInfoCache,
    class_id: Option<i32>,
) -> (Option<InspectorPanel>, Option<InspectorPanel>) {
    use super::script::{
        doc_script_guid, lookup_script_semantic_info, resolve_all_field_types_readonly,
    };

    let old_script_info =
        old_doc.and_then(|doc| lookup_script_semantic_info(doc, old_lines, old_ctx, script_cache));
    let new_script_info =
        new_doc.and_then(|doc| lookup_script_semantic_info(doc, new_lines, new_ctx, script_cache));

    let mut old_map = old_doc
        .map(|doc| parse_doc_field_map(doc, old_lines, old_ctx, old_labels))
        .unwrap_or_default();
    let mut new_map = new_doc
        .map(|doc| parse_doc_field_map(doc, new_lines, new_ctx, new_labels))
        .unwrap_or_default();
    apply_field_label_enhancements(&mut old_map, old_script_info.as_ref());
    apply_field_label_enhancements(&mut new_map, new_script_info.as_ref());

    let mut hidden_roots = HashSet::new();
    if matches!(
        panel_kind,
        InspectorPanelKind::AssetRoot
            | InspectorPanelKind::SubObject
            | InspectorPanelKind::Component
    ) {
        hidden_roots.extend(HIDDEN_ASSET_FIELDS.iter().map(|value| value.to_string()));
        for info in [&old_script_info, &new_script_info].into_iter().flatten() {
            for (alias, field) in &info.field_aliases {
                if field.hidden {
                    hidden_roots.insert(alias.clone());
                    hidden_roots.insert(field.canonical_name.clone());
                }
            }
        }
    }
    let script_info = new_script_info.as_ref().or(old_script_info.as_ref());

    let hint_script_path = new_doc
        .and_then(|doc| {
            doc_script_guid(doc, new_lines).and_then(|guid| new_ctx.resolve_script_guid_path(&guid))
        })
        .or_else(|| {
            old_doc.and_then(|doc| {
                doc_script_guid(doc, old_lines)
                    .and_then(|guid| old_ctx.resolve_script_guid_path(&guid))
            })
        });
    let all_paths = old_map.keys().chain(new_map.keys());
    let field_type_map = resolve_all_field_types_readonly(
        all_paths,
        script_info,
        hint_script_path.as_deref(),
        script_cache,
    );

    let (changed_fields, full_fields) = build_inspector_fields_pair(
        &old_map,
        &new_map,
        hidden_roots,
        script_info,
        &field_type_map,
    );

    // Note: material shader property loading skipped in readonly mode (requires I/O).
    // Pre-warming phase should have loaded these if needed.

    let change_kind = match (old_doc, new_doc) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        _ if changed_fields.is_empty() => "unchanged",
        _ => "modified",
    };

    let component_resolve_reason =
        component_resolve_reason(&panel_kind, class_id, script_class.as_deref());

    let make_panel =
        |fields: Vec<InspectorField>, include_unchanged: bool| -> Option<InspectorPanel> {
            if !include_unchanged && change_kind == "unchanged" {
                return None;
            }
            let component_type = class_id.map(|id| {
                if id == 114 {
                    script_class
                        .clone()
                        .unwrap_or_else(|| "MonoBehaviour".into())
                } else {
                    unity_class_name(id).to_string()
                }
            });
            let component_source = Some(
                match panel_kind {
                    InspectorPanelKind::Component => {
                        if class_id == Some(114) {
                            "script"
                        } else {
                            "builtin"
                        }
                    }
                    InspectorPanelKind::GameObjectHeader => "gameObjectHeader",
                    InspectorPanelKind::AssetRoot => "assetRoot",
                    InspectorPanelKind::SubObject => "subObject",
                }
                .into(),
            );
            Some(InspectorPanel {
                panel_kind: panel_kind.clone(),
                title: title.clone(),
                script_class: script_class.clone(),
                change_kind: change_kind.to_string(),
                added: change_kind == "added",
                removed: change_kind == "removed",
                component_type,
                component_class_id: class_id,
                component_source,
                component_resolve_reason: component_resolve_reason.clone(),
                component_inference: None,
                fields,
            })
        };

    let changed = make_panel(changed_fields, false);
    let full = make_panel(full_fields, true);
    (changed, full)
}

#[allow(dead_code)] // Used in tests; production code uses build_doc_panel_pair
pub(crate) fn build_doc_panel(
    panel_kind: InspectorPanelKind,
    title: String,
    script_class: Option<String>,
    old_doc: Option<&YamlDoc>,
    new_doc: Option<&YamlDoc>,
    old_lines: &[String],
    new_lines: &[String],
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    old_ctx: &SideContext,
    new_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
    include_unchanged: bool,
    class_id: Option<i32>,
) -> Option<InspectorPanel> {
    let old_script_info =
        old_doc.and_then(|doc| load_script_semantic_info(doc, old_lines, old_ctx, env));
    let new_script_info =
        new_doc.and_then(|doc| load_script_semantic_info(doc, new_lines, new_ctx, env));

    let mut old_map = old_doc
        .map(|doc| parse_doc_field_map(doc, old_lines, old_ctx, old_labels))
        .unwrap_or_default();
    let mut new_map = new_doc
        .map(|doc| parse_doc_field_map(doc, new_lines, new_ctx, new_labels))
        .unwrap_or_default();
    apply_field_label_enhancements(&mut old_map, old_script_info.as_ref());
    apply_field_label_enhancements(&mut new_map, new_script_info.as_ref());

    let mut hidden_roots = HashSet::new();
    if matches!(
        panel_kind,
        InspectorPanelKind::AssetRoot
            | InspectorPanelKind::SubObject
            | InspectorPanelKind::Component
    ) {
        hidden_roots.extend(HIDDEN_ASSET_FIELDS.iter().map(|value| value.to_string()));
        for info in [&old_script_info, &new_script_info].into_iter().flatten() {
            for (alias, field) in &info.field_aliases {
                if field.hidden {
                    hidden_roots.insert(alias.clone());
                    hidden_roots.insert(field.canonical_name.clone());
                }
            }
        }
    }
    let script_info = new_script_info.as_ref().or(old_script_info.as_ref());

    // Pre-resolve field types for all paths (including nested) via the C# type chain
    let hint_script_path = new_doc
        .and_then(|doc| {
            doc_script_guid(doc, new_lines).and_then(|guid| new_ctx.resolve_script_guid_path(&guid))
        })
        .or_else(|| {
            old_doc.and_then(|doc| {
                doc_script_guid(doc, old_lines)
                    .and_then(|guid| old_ctx.resolve_script_guid_path(&guid))
            })
        });
    let all_paths = old_map.keys().chain(new_map.keys());
    let resolve_ctx = new_ctx;
    let field_type_map = resolve_all_field_types(
        all_paths,
        script_info,
        hint_script_path.as_deref(),
        resolve_ctx,
        env,
    );

    let mut fields = build_inspector_fields(
        &old_map,
        &new_map,
        include_unchanged,
        hidden_roots,
        script_info,
        &field_type_map,
    );

    // Material-specific: flatten saved properties and order by shader definition
    if class_id == Some(21) {
        let shader_props =
            load_material_shader_properties(&new_map, &old_map, new_ctx, old_ctx, env);
        restructure_material_fields(&mut fields, &shader_props);
    }

    let change_kind = match (old_doc, new_doc) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        _ if fields.is_empty() => "unchanged",
        _ => "modified",
    };

    if !include_unchanged && change_kind == "unchanged" {
        return None;
    }

    let component_type = class_id.map(|id| {
        if id == 114 {
            script_class
                .clone()
                .unwrap_or_else(|| "MonoBehaviour".into())
        } else {
            unity_class_name(id).to_string()
        }
    });
    let component_source = Some(
        match panel_kind {
            InspectorPanelKind::Component => {
                if class_id == Some(114) {
                    "script"
                } else {
                    "builtin"
                }
            }
            InspectorPanelKind::GameObjectHeader => "gameObjectHeader",
            InspectorPanelKind::AssetRoot => "assetRoot",
            InspectorPanelKind::SubObject => "subObject",
        }
        .into(),
    );
    let component_resolve_reason =
        component_resolve_reason(&panel_kind, class_id, script_class.as_deref());

    Some(InspectorPanel {
        panel_kind,
        title,
        script_class,
        change_kind: change_kind.to_string(),
        added: change_kind == "added",
        removed: change_kind == "removed",
        component_type,
        component_class_id: class_id,
        component_source,
        component_resolve_reason,
        component_inference: None,
        fields,
    })
}

pub(crate) fn field_from_scalar(
    property_path: &str,
    label: &str,
    before: Option<String>,
    after: Option<String>,
    include_unchanged: bool,
) -> Option<InspectorField> {
    let change_kind = match (&before, &after) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        (Some(before), Some(after)) if before == after => "unchanged",
        (Some(_), Some(_)) => "modified",
        (None, None) => "unchanged",
    };

    if !include_unchanged && change_kind == "unchanged" {
        return None;
    }

    Some(InspectorField {
        id: format!("{}:{}", property_path, change_kind),
        label: label.to_string(),
        property_path: property_path.to_string(),
        value_type: "string".into(),
        change_kind: change_kind.to_string(),
        before,
        after,
        children: Vec::new(),
        reference: None,
        field_type: None,
    })
}

pub(crate) fn build_gameobject_header_panel(
    old_doc: Option<&YamlDoc>,
    new_doc: Option<&YamlDoc>,
    include_unchanged: bool,
) -> Option<InspectorPanel> {
    let mut fields = Vec::new();
    let field_specs = [
        (
            "m_Name",
            "Name",
            old_doc.and_then(|doc| doc.m_name.clone()),
            new_doc.and_then(|doc| doc.m_name.clone()),
        ),
        (
            "m_TagString",
            "Tag",
            old_doc.and_then(|doc| doc.m_tag_string.clone()),
            new_doc.and_then(|doc| doc.m_tag_string.clone()),
        ),
        (
            "m_Layer",
            "Layer",
            old_doc.and_then(|doc| doc.m_layer.map(|value| value.to_string())),
            new_doc.and_then(|doc| doc.m_layer.map(|value| value.to_string())),
        ),
        (
            "m_IsActive",
            "Active",
            old_doc.and_then(|doc| {
                doc.m_is_active
                    .map(|value| if value != 0 { "true" } else { "false" }.to_string())
            }),
            new_doc.and_then(|doc| {
                doc.m_is_active
                    .map(|value| if value != 0 { "true" } else { "false" }.to_string())
            }),
        ),
        (
            "m_StaticEditorFlags",
            "Static",
            old_doc.and_then(|doc| {
                doc.m_static_editor_flags
                    .map(|value| if value != 0 { "true" } else { "false" }.to_string())
            }),
            new_doc.and_then(|doc| {
                doc.m_static_editor_flags
                    .map(|value| if value != 0 { "true" } else { "false" }.to_string())
            }),
        ),
    ];

    for (property_path, label, before, after) in field_specs {
        if let Some(field) =
            field_from_scalar(property_path, label, before, after, include_unchanged)
        {
            fields.push(field);
        }
    }

    // Use per-field change_kind to detect a real modification, NOT
    // `fields.is_empty()`. With `include_unchanged == true` (used by the
    // workspace asset preview's "two same doc" trick) the field list is
    // populated with all-`unchanged` entries even though nothing changed —
    // the old `fields.is_empty()` check would mis-classify the panel as
    // "modified" and paint every GameObject header in the asset preview red.
    let change_kind = match (old_doc, new_doc) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        _ if fields.iter().all(|f| f.change_kind == "unchanged") => "unchanged",
        _ => "modified",
    };

    if !include_unchanged && change_kind == "unchanged" {
        return None;
    }

    Some(InspectorPanel {
        panel_kind: InspectorPanelKind::GameObjectHeader,
        title: "GameObject".into(),
        script_class: None,
        change_kind: change_kind.to_string(),
        added: change_kind == "added",
        removed: change_kind == "removed",
        component_type: Some("GameObject".into()),
        component_class_id: Some(1),
        component_source: Some("gameObjectHeader".into()),
        component_resolve_reason: None,
        component_inference: None,
        fields,
    })
}

pub(crate) fn collect_hierarchy_entries(
    roots: &[HierarchyNode],
    docs_by_id: &HashMap<i64, &YamlDoc>,
) -> HashMap<i64, HierarchyEntry> {
    fn walk(
        node: &HierarchyNode,
        parent_id: Option<i64>,
        prefix: Option<&str>,
        docs_by_id: &HashMap<i64, &YamlDoc>,
        path_by_file_id: &HashMap<i64, String>,
        order: &mut usize,
        out: &mut HashMap<i64, HierarchyEntry>,
    ) {
        let fallback_path = match prefix {
            Some(prefix) if !prefix.is_empty() => format!("{}/{}", prefix, node.name),
            _ => node.name.clone(),
        };
        let path = path_by_file_id
            .get(&node.file_id)
            .cloned()
            .unwrap_or(fallback_path);
        let object_kind = match docs_by_id.get(&node.file_id).map(|doc| doc.class_id) {
            Some(1001) => "prefabInstance",
            _ => "gameObject",
        };
        out.insert(
            node.file_id,
            HierarchyEntry {
                file_id: node.file_id,
                parent_id,
                label: node.name.clone(),
                path: path.clone(),
                object_kind: object_kind.to_string(),
                order: *order,
            },
        );
        *order += 1;
        for child in &node.children {
            walk(
                child,
                Some(node.file_id),
                Some(&path),
                docs_by_id,
                path_by_file_id,
                order,
                out,
            );
        }
    }

    let mut out = HashMap::new();
    let path_by_file_id = build_hierarchy_path_map(roots);
    let mut order = 0usize;
    for root in roots {
        walk(
            root,
            None,
            None,
            docs_by_id,
            &path_by_file_id,
            &mut order,
            &mut out,
        );
    }
    out
}

pub(crate) fn component_sort_key(class_id: i32) -> i32 {
    match class_id {
        4 | 224 => 0,
        _ => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_hierarchy_entries_uses_unity_yaml_ordinal_paths() {
        let roots = vec![HierarchyNode {
            name: "Root".to_string(),
            file_id: 1,
            children: vec![
                HierarchyNode {
                    name: "Enemy".to_string(),
                    file_id: 2,
                    ..Default::default()
                },
                HierarchyNode {
                    name: "Enemy".to_string(),
                    file_id: 3,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        let docs_by_id = HashMap::new();

        let entries = collect_hierarchy_entries(&roots, &docs_by_id);

        assert_eq!(
            entries.get(&2).map(|entry| entry.path.as_str()),
            Some("Root/Enemy[1]")
        );
        assert_eq!(
            entries.get(&3).map(|entry| entry.path.as_str()),
            Some("Root/Enemy[2]")
        );
    }
}
