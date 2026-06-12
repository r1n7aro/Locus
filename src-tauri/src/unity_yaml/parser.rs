use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{parse_guid_hex, Guid};

use super::references;
use super::tokenizer::{
    count_braces, extract_field_name, extract_field_name_ref, extract_internal_file_id,
    extract_plain_value, extract_value, find_closing_brace, parse_doc_header_full,
};

#[derive(Debug, Clone)]
pub struct YamlDoc {
    pub file_id: i64,
    pub class_id: i32,
    pub type_name: String,
    pub line_start: usize,
    pub line_end: usize,
    pub m_name: Option<String>,
    pub m_game_object_id: Option<i64>,
    pub m_father_id: Option<i64>,
    pub is_stripped: bool,
    pub source_prefab_guid: Option<Guid>,
    pub transform_parent_id: Option<i64>,
    pub prefab_instance_id: Option<i64>,
    pub m_layer: Option<i32>,
    pub m_tag_string: Option<String>,
    pub m_static_editor_flags: Option<i64>,
    pub m_is_active: Option<i32>,
    pub m_enabled: Option<bool>,
    /// For Transform / RectTransform docs: sibling order for root objects.
    pub transform_root_order: Option<i32>,
    /// For Transform / RectTransform docs: ordered child transform fileIDs.
    pub transform_children: Vec<i64>,
    /// For MonoBehaviour (class_id==114): GUID from m_Script field.
    pub m_script_guid: Option<Guid>,
    /// Parse order index within the YAML file (0-based).
    pub doc_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct HierarchyNode {
    pub name: String,
    pub file_id: i64,
    pub components: Vec<String>,
    pub children: Vec<HierarchyNode>,
    pub tag: Option<String>,
    pub layer: Option<i32>,
    pub is_static: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformWorldInfo {
    pub position: [f64; 3],
    pub rotation_euler: [f64; 3],
    pub scale: [f64; 3],
}

#[derive(Debug, Clone, Copy)]
struct LocalTransformData {
    position: [f64; 3],
    rotation: [f64; 4],
    scale: [f64; 3],
}

impl Default for LocalTransformData {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TransformWorldState {
    matrix: [[f64; 4]; 4],
    rotation: [f64; 4],
}

pub fn parse_yaml_docs(content: &[u8]) -> Vec<YamlDoc> {
    parse_yaml_docs_impl(content, false).0
}

/// Single-pass variant that additionally captures every guid-bearing flow map
/// as a raw reference candidate. The ref-graph pipeline needs both the
/// documents and the references for each yaml asset; producing them from one
/// scan halves the parse cost compared to running `parse_yaml_docs` and a
/// standalone reference extractor back to back (and keeps the two from ever
/// disagreeing about document boundaries or field context).
pub fn parse_yaml_docs_with_refs(content: &[u8]) -> (Vec<YamlDoc>, Vec<references::RawYamlRef>) {
    parse_yaml_docs_impl(content, true)
}

fn parse_yaml_docs_impl(
    content: &[u8],
    collect_refs: bool,
) -> (Vec<YamlDoc>, Vec<references::RawYamlRef>) {
    let text = String::from_utf8_lossy(content);

    let mut docs: Vec<YamlDoc> = Vec::new();
    let mut raw_refs: Vec<references::RawYamlRef> = Vec::new();
    let mut raw_seen: references::RawRefSeen = HashSet::new();
    let mut cur_class_id: Option<i32> = None;
    let mut cur_file_id: i64 = 0;
    let mut cur_type_name: Option<String> = None;
    let mut cur_m_name: Option<String> = None;
    let mut cur_m_go_id: Option<i64> = None;
    let mut cur_m_father: Option<i64> = None;
    let mut cur_line_start: usize = 0;
    let mut first_key = false;
    let mut last_field: Option<String> = None;
    let mut pending_line: Option<String> = None;
    let mut pending_braces: i32 = 0;
    let mut cur_is_stripped: bool = false;
    let mut cur_source_prefab_guid: Option<Guid> = None;
    let mut cur_transform_parent_id: Option<i64> = None;
    let mut cur_prefab_instance_id: Option<i64> = None;
    let mut awaiting_name_value: bool = false;
    let mut cur_m_layer: Option<i32> = None;
    let mut cur_m_tag_string: Option<String> = None;
    let mut cur_m_static_editor_flags: Option<i64> = None;
    let mut cur_m_is_active: Option<i32> = None;
    let mut cur_m_enabled: Option<bool> = None;
    let mut cur_transform_root_order: Option<i32> = None;
    let mut cur_transform_children: Vec<i64> = Vec::new();
    let mut cur_m_script_guid: Option<Guid> = None;

    fn extract_prefab_meta_from_flow(
        ct: &str,
        field: &str,
        class_id: Option<i32>,
        is_stripped: bool,
        m_go_id: &mut Option<i64>,
        m_father: &mut Option<i64>,
        transform_parent_id: &mut Option<i64>,
        prefab_instance_id: &mut Option<i64>,
        source_prefab_guid: &mut Option<Guid>,
    ) {
        let has_file_id = ct.contains("fileID:");
        let has_guid = ct.contains("guid:");

        if has_file_id && !has_guid {
            if let Some(fid) = extract_internal_file_id(ct) {
                match field {
                    "m_GameObject" => {
                        if fid != 0 {
                            *m_go_id = Some(fid);
                        }
                    }
                    "m_Father" => {
                        if fid != 0 {
                            *m_father = Some(fid);
                        }
                    }
                    "m_TransformParent" if class_id == Some(1001) => {
                        *transform_parent_id = Some(fid);
                    }
                    "m_PrefabInstance" if is_stripped => {
                        if fid != 0 {
                            *prefab_instance_id = Some(fid);
                        }
                    }
                    _ => {}
                }
            }
        }

        if has_guid && field == "m_SourcePrefab" && class_id == Some(1001) {
            if let Some(guid_str) = extract_value(ct, "guid:") {
                let hex = guid_str.trim().trim_end_matches(',');
                if hex.len() >= 32 {
                    *source_prefab_guid = parse_guid_hex(&hex[..32]);
                }
            }
        }
    }

    macro_rules! flush_doc {
        ($line_end:expr) => {
            if cur_file_id != 0 {
                if let Some(cid) = cur_class_id {
                    docs.push(YamlDoc {
                        file_id: cur_file_id,
                        class_id: cid,
                        type_name: cur_type_name.take().unwrap_or_default(),
                        line_start: cur_line_start,
                        line_end: $line_end,
                        m_name: cur_m_name.take(),
                        m_game_object_id: cur_m_go_id.take(),
                        m_father_id: cur_m_father.take(),
                        is_stripped: cur_is_stripped,
                        source_prefab_guid: cur_source_prefab_guid.take(),
                        transform_parent_id: cur_transform_parent_id.take(),
                        prefab_instance_id: cur_prefab_instance_id.take(),
                        m_layer: cur_m_layer.take(),
                        m_tag_string: cur_m_tag_string.take(),
                        m_static_editor_flags: cur_m_static_editor_flags.take(),
                        m_is_active: cur_m_is_active.take(),
                        m_enabled: cur_m_enabled.take(),
                        transform_root_order: cur_transform_root_order.take(),
                        transform_children: std::mem::take(&mut cur_transform_children),
                        m_script_guid: cur_m_script_guid.take(),
                        doc_index: docs.len(),
                    });
                }
            }
        };
    }

    // Stream the lines instead of collecting a Vec<&str>: scene files run to
    // tens of thousands of lines and this loop is the hottest part of the
    // full scan's yaml phase.
    let mut line_count = 0usize;
    for (i, line) in text.lines().enumerate() {
        line_count = i + 1;
        let trimmed = line.trim();

        if let Some(ref mut buf) = pending_line {
            buf.push(' ');
            buf.push_str(trimmed);
            pending_braces += count_braces(trimmed);
            if pending_braces <= 0 {
                let complete = pending_line.take().unwrap();
                let ct = complete.trim();
                if let Some(f) = extract_field_name(ct) {
                    last_field = Some(f);
                }
                if let Some(ref field) = last_field {
                    extract_prefab_meta_from_flow(
                        ct,
                        field,
                        cur_class_id,
                        cur_is_stripped,
                        &mut cur_m_go_id,
                        &mut cur_m_father,
                        &mut cur_transform_parent_id,
                        &mut cur_prefab_instance_id,
                        &mut cur_source_prefab_guid,
                    );
                    // Extract m_Script GUID from multi-line flow
                    if cur_class_id == Some(114) && field == "m_Script" && ct.contains("guid:") {
                        if let Some(start) = ct.find("guid:") {
                            let after = &ct[start + 5..];
                            let after = after.trim_start();
                            let end = after
                                .find(|c: char| c == ',' || c == '}')
                                .unwrap_or(after.len());
                            let guid_str = after[..end].trim().trim_end_matches(',');
                            if let Some(guid) = parse_guid_hex(guid_str) {
                                if guid != [0u8; 16] {
                                    cur_m_script_guid = Some(guid);
                                }
                            }
                        }
                    }
                }
                if collect_refs && ct.contains("guid:") {
                    references::extract_flow_maps_raw(
                        ct,
                        cur_class_id,
                        &last_field,
                        cur_file_id,
                        &mut raw_refs,
                        &mut raw_seen,
                    );
                }
                pending_braces = 0;
            }
            continue;
        }

        if trimmed.starts_with("---") {
            flush_doc!(i);
            if let Some((cid, fid)) = parse_doc_header_full(trimmed) {
                cur_class_id = Some(cid);
                cur_file_id = fid;
            } else {
                cur_class_id = None;
                cur_file_id = 0;
            }
            cur_type_name = None;
            cur_m_name = None;
            cur_m_go_id = None;
            cur_m_father = None;
            cur_is_stripped = trimmed.contains(" stripped");
            cur_source_prefab_guid = None;
            cur_transform_parent_id = None;
            cur_prefab_instance_id = None;
            awaiting_name_value = false;
            cur_m_layer = None;
            cur_m_tag_string = None;
            cur_m_static_editor_flags = None;
            cur_m_is_active = None;
            cur_m_enabled = None;
            cur_transform_root_order = None;
            cur_transform_children.clear();
            cur_line_start = i;
            first_key = true;
            last_field = None;
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('%') {
            continue;
        }

        if first_key && !line.starts_with(' ') && !line.starts_with('\t') {
            if let Some(pos) = trimmed.find(':') {
                let key = &trimmed[..pos];
                if !key.is_empty() {
                    cur_type_name = Some(key.to_string());
                }
            }
            first_key = false;
        }

        if let Some(f) = extract_field_name_ref(trimmed) {
            if f == "m_Name" {
                if let Some(val) = extract_plain_value(trimmed, "m_Name:") {
                    cur_m_name = Some(val);
                }
            }
            if cur_class_id == Some(1) {
                if f == "m_Layer" {
                    if let Some(val) = extract_plain_value(trimmed, "m_Layer:") {
                        if let Ok(n) = val.trim().parse::<i32>() {
                            cur_m_layer = Some(n);
                        }
                    }
                } else if f == "m_TagString" {
                    if let Some(val) = extract_plain_value(trimmed, "m_TagString:") {
                        cur_m_tag_string = Some(val);
                    }
                } else if f == "m_StaticEditorFlags" {
                    if let Some(val) = extract_plain_value(trimmed, "m_StaticEditorFlags:") {
                        if let Ok(n) = val.trim().parse::<i64>() {
                            cur_m_static_editor_flags = Some(n);
                        }
                    }
                } else if f == "m_IsActive" {
                    if let Some(val) = extract_plain_value(trimmed, "m_IsActive:") {
                        if let Ok(n) = val.trim().parse::<i32>() {
                            cur_m_is_active = Some(n);
                        }
                    }
                }
            }
            if f == "m_Enabled" {
                if let Some(val) = extract_plain_value(trimmed, "m_Enabled:") {
                    if let Ok(n) = val.trim().parse::<i32>() {
                        cur_m_enabled = Some(n != 0);
                    }
                }
            }
            if cur_class_id == Some(4) || cur_class_id == Some(224) {
                if f == "m_RootOrder" {
                    if let Some(val) = extract_plain_value(trimmed, "m_RootOrder:") {
                        if let Ok(n) = val.trim().parse::<i32>() {
                            cur_transform_root_order = Some(n);
                        }
                    }
                } else if f == "m_Children" {
                    cur_transform_children.clear();
                }
            }
            if cur_class_id == Some(1001) {
                if f == "propertyPath" {
                    if let Some(val) = extract_plain_value(trimmed, "propertyPath:") {
                        awaiting_name_value = val.trim() == "m_Name";
                    }
                } else if f == "value" && awaiting_name_value {
                    if cur_m_name.is_none() {
                        if let Some(val) = extract_plain_value(trimmed, "value:") {
                            if !val.is_empty() {
                                cur_m_name = Some(val);
                            }
                        }
                    }
                    awaiting_name_value = false;
                } else if f != "value" {
                    awaiting_name_value = false;
                }
            }
            // Extract m_Script GUID for MonoBehaviour docs
            if cur_class_id == Some(114) && f == "m_Script" {
                if trimmed.contains("guid:") {
                    if let Some(start) = trimmed.find("guid:") {
                        let after = &trimmed[start + 5..];
                        let after = after.trim_start();
                        let end = after
                            .find(|c: char| c == ',' || c == '}')
                            .unwrap_or(after.len());
                        let guid_str = after[..end].trim().trim_end_matches(',');
                        if let Some(guid) = parse_guid_hex(guid_str) {
                            if guid != [0u8; 16] {
                                cur_m_script_guid = Some(guid);
                            }
                        }
                    }
                }
            }
            last_field = Some(f.to_string());
        } else if (cur_class_id == Some(4) || cur_class_id == Some(224))
            && last_field.as_deref() == Some("m_Children")
            && trimmed.starts_with('-')
        {
            if let Some(child_transform_id) = extract_internal_file_id(trimmed) {
                if child_transform_id != 0 {
                    cur_transform_children.push(child_transform_id);
                }
            }
        }

        if trimmed.contains('{') {
            let balance = count_braces(trimmed);
            if balance > 0 {
                let mut buf = String::with_capacity(256);
                buf.push_str(trimmed);
                pending_line = Some(buf);
                pending_braces = balance;
                continue;
            }
            if let Some(ref field) = last_field {
                extract_prefab_meta_from_flow(
                    trimmed,
                    field,
                    cur_class_id,
                    cur_is_stripped,
                    &mut cur_m_go_id,
                    &mut cur_m_father,
                    &mut cur_transform_parent_id,
                    &mut cur_prefab_instance_id,
                    &mut cur_source_prefab_guid,
                );
            }
            if collect_refs && trimmed.contains("guid:") {
                references::extract_flow_maps_raw(
                    trimmed,
                    cur_class_id,
                    &last_field,
                    cur_file_id,
                    &mut raw_refs,
                    &mut raw_seen,
                );
            }
        }
    }

    flush_doc!(line_count);
    (docs, raw_refs)
}

pub fn build_world_transform_map(
    docs: &[YamlDoc],
    lines: &[&str],
) -> HashMap<i64, TransformWorldInfo> {
    let mut local_by_transform: HashMap<i64, LocalTransformData> = HashMap::new();
    let mut parent_by_transform: HashMap<i64, i64> = HashMap::new();

    for doc in docs {
        if doc.class_id != 4 && doc.class_id != 224 {
            continue;
        }

        local_by_transform.insert(doc.file_id, extract_local_transform_data(lines, doc));
        if let Some(parent_id) = doc.m_father_id.filter(|fid| *fid != 0) {
            parent_by_transform.insert(doc.file_id, parent_id);
        }
    }

    let mut state_cache: HashMap<i64, TransformWorldState> = HashMap::new();
    let mut result: HashMap<i64, TransformWorldInfo> = HashMap::new();

    for &transform_id in local_by_transform.keys() {
        let mut visiting = HashSet::new();
        if let Some(state) = compute_world_transform_state(
            transform_id,
            &local_by_transform,
            &parent_by_transform,
            &mut state_cache,
            &mut visiting,
        ) {
            result.insert(
                transform_id,
                TransformWorldInfo {
                    position: [
                        sanitize_negative_zero(state.matrix[0][3]),
                        sanitize_negative_zero(state.matrix[1][3]),
                        sanitize_negative_zero(state.matrix[2][3]),
                    ],
                    rotation_euler: quaternion_to_euler_degrees(state.rotation),
                    scale: extract_lossy_scale(state.matrix),
                },
            );
        }
    }

    result
}

pub fn build_go_tree(docs: &[YamlDoc]) -> Vec<HierarchyNode> {
    let mut node_names: HashMap<i64, &str> = docs
        .iter()
        .filter(|d| d.class_id == 1 && !d.is_stripped)
        .filter_map(|d| d.m_name.as_deref().map(|n| (d.file_id, n)))
        .collect();

    let mut go_to_transform: HashMap<i64, i64> = HashMap::new();
    let mut transform_father: HashMap<i64, i64> = HashMap::new();
    let mut transform_to_go: HashMap<i64, i64> = HashMap::new();
    let mut transform_root_order: HashMap<i64, i32> = HashMap::new();
    let mut transform_children: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut go_components: HashMap<i64, Vec<String>> = HashMap::new();
    let mut node_doc_index: HashMap<i64, usize> = docs
        .iter()
        .filter(|doc| doc.class_id == 1 && !doc.is_stripped)
        .map(|doc| (doc.file_id, doc.doc_index))
        .collect();
    let mut stripped_owner_by_transform: HashMap<i64, i64> = HashMap::new();

    for doc in docs {
        if doc.class_id == 1001 {
            node_doc_index.insert(doc.file_id, doc.doc_index);
        }
        if doc.class_id == 4 || doc.class_id == 224 {
            if let Some(go_id) = doc.m_game_object_id {
                if go_id != 0 && !doc.is_stripped {
                    go_to_transform.insert(go_id, doc.file_id);
                    transform_to_go.insert(doc.file_id, go_id);
                }
            }
            if let Some(father) = doc.m_father_id {
                transform_father.insert(doc.file_id, father);
            }
            if let Some(order) = doc.transform_root_order {
                transform_root_order.insert(doc.file_id, order);
            }
            if !doc.transform_children.is_empty() {
                transform_children.insert(doc.file_id, doc.transform_children.clone());
            }
            if doc.is_stripped {
                if let Some(pi) = doc.prefab_instance_id {
                    stripped_owner_by_transform.insert(doc.file_id, pi);
                }
            }
        }
        if !doc.is_stripped {
            if let Some(go_id) = doc.m_game_object_id {
                if go_id != 0
                    && doc.class_id != 4
                    && doc.class_id != 224
                    && doc.type_name != "CanvasRenderer"
                {
                    go_components
                        .entry(go_id)
                        .or_default()
                        .push(doc.type_name.clone());
                }
            }
        }
    }

    let mut all_node_ids: Vec<i64> = docs
        .iter()
        .filter(|d| d.class_id == 1 && !d.is_stripped)
        .map(|d| d.file_id)
        .collect();

    let stripped_transform_to_prefab: HashMap<i64, i64> = docs
        .iter()
        .filter(|d| d.is_stripped && (d.class_id == 4 || d.class_id == 224))
        .filter_map(|d| d.prefab_instance_id.map(|pi| (d.file_id, pi)))
        .collect();
    let prefab_root_transform: HashMap<i64, i64> = docs
        .iter()
        .filter(|doc| doc.is_stripped && (doc.class_id == 4 || doc.class_id == 224))
        .filter_map(|doc| {
            let prefab_instance_id = doc.prefab_instance_id?;
            let same_prefab_parent = doc
                .m_father_id
                .and_then(|father| stripped_owner_by_transform.get(&father).copied())
                == Some(prefab_instance_id);
            if same_prefab_parent {
                None
            } else {
                Some((prefab_instance_id, doc.file_id))
            }
        })
        .collect();

    let mut parent_map: HashMap<i64, i64> = HashMap::new();
    for &go_id in &all_node_ids {
        if let Some(&transform_id) = go_to_transform.get(&go_id) {
            if let Some(&father_transform) = transform_father.get(&transform_id) {
                if father_transform != 0 {
                    if let Some(&parent_go) = transform_to_go.get(&father_transform) {
                        parent_map.insert(go_id, parent_go);
                    } else if let Some(&parent_pi) =
                        stripped_transform_to_prefab.get(&father_transform)
                    {
                        parent_map.insert(go_id, parent_pi);
                    }
                }
            }
        }
    }

    for doc in docs {
        if doc.class_id == 1001 {
            let name = doc.m_name.as_deref().unwrap_or("?");
            node_names.insert(doc.file_id, name);
            all_node_ids.push(doc.file_id);

            if let Some(parent_transform) = doc.transform_parent_id {
                if parent_transform != 0 {
                    if let Some(&parent_pi) = stripped_transform_to_prefab.get(&parent_transform) {
                        parent_map.insert(doc.file_id, parent_pi);
                    } else if let Some(&parent_go) = transform_to_go.get(&parent_transform) {
                        parent_map.insert(doc.file_id, parent_go);
                    }
                }
            }
        }
    }

    let mut go_props: HashMap<i64, (Option<String>, Option<i32>, Option<i64>, Option<i32>)> =
        HashMap::new();
    for doc in docs {
        if doc.class_id == 1 && !doc.is_stripped {
            go_props.insert(
                doc.file_id,
                (
                    doc.m_tag_string.clone(),
                    doc.m_layer,
                    doc.m_static_editor_flags,
                    doc.m_is_active,
                ),
            );
        }
    }

    let mut children_map: HashMap<i64, Vec<i64>> = HashMap::new();
    for (&child, &parent) in &parent_map {
        children_map.entry(parent).or_default().push(child);
    }

    fn compare_root_nodes(
        left: i64,
        right: i64,
        go_to_transform: &HashMap<i64, i64>,
        prefab_root_transform: &HashMap<i64, i64>,
        transform_root_order: &HashMap<i64, i32>,
        node_doc_index: &HashMap<i64, usize>,
    ) -> Ordering {
        let left_root_order = go_to_transform
            .get(&left)
            .and_then(|transform_id| transform_root_order.get(transform_id))
            .copied()
            .or_else(|| {
                prefab_root_transform
                    .get(&left)
                    .and_then(|transform_id| transform_root_order.get(transform_id))
                    .copied()
            });
        let right_root_order = go_to_transform
            .get(&right)
            .and_then(|transform_id| transform_root_order.get(transform_id))
            .copied()
            .or_else(|| {
                prefab_root_transform
                    .get(&right)
                    .and_then(|transform_id| transform_root_order.get(transform_id))
                    .copied()
            });

        match (left_root_order, right_root_order) {
            (Some(a), Some(b)) => a
                .cmp(&b)
                .then_with(|| {
                    node_doc_index
                        .get(&left)
                        .copied()
                        .unwrap_or(usize::MAX)
                        .cmp(&node_doc_index.get(&right).copied().unwrap_or(usize::MAX))
                })
                .then_with(|| left.cmp(&right)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => node_doc_index
                .get(&left)
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&node_doc_index.get(&right).copied().unwrap_or(usize::MAX))
                .then_with(|| left.cmp(&right)),
        }
    }

    fn ordered_children_for_parent(
        parent_node_id: i64,
        parent_map: &HashMap<i64, i64>,
        children_map: &HashMap<i64, Vec<i64>>,
        go_to_transform: &HashMap<i64, i64>,
        transform_to_go: &HashMap<i64, i64>,
        stripped_transform_to_prefab: &HashMap<i64, i64>,
        prefab_root_transform: &HashMap<i64, i64>,
        transform_children: &HashMap<i64, Vec<i64>>,
        node_doc_index: &HashMap<i64, usize>,
    ) -> Vec<i64> {
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        let ordering_transform = go_to_transform
            .get(&parent_node_id)
            .copied()
            .or_else(|| prefab_root_transform.get(&parent_node_id).copied());

        if let Some(parent_transform_id) = ordering_transform {
            if let Some(child_transform_ids) = transform_children.get(&parent_transform_id) {
                for child_transform_id in child_transform_ids {
                    let child_node_id =
                        transform_to_go
                            .get(child_transform_id)
                            .copied()
                            .or_else(|| {
                                stripped_transform_to_prefab
                                    .get(child_transform_id)
                                    .copied()
                            });
                    let Some(child_node_id) = child_node_id else {
                        continue;
                    };
                    if child_node_id == parent_node_id {
                        continue;
                    }
                    if parent_map.get(&child_node_id) == Some(&parent_node_id)
                        && seen.insert(child_node_id)
                    {
                        ordered.push(child_node_id);
                    }
                }
            }
        }

        let mut fallback = children_map
            .get(&parent_node_id)
            .cloned()
            .unwrap_or_default();
        fallback.retain(|child_node_id| !seen.contains(child_node_id));
        fallback.sort_by_key(|child_node_id| {
            (
                node_doc_index
                    .get(child_node_id)
                    .copied()
                    .unwrap_or(usize::MAX),
                *child_node_id,
            )
        });
        ordered.extend(fallback);
        ordered
    }

    let mut roots: Vec<i64> = all_node_ids
        .iter()
        .filter(|id| !parent_map.contains_key(id))
        .copied()
        .collect();
    roots.sort_by(|left, right| {
        compare_root_nodes(
            *left,
            *right,
            &go_to_transform,
            &prefab_root_transform,
            &transform_root_order,
            &node_doc_index,
        )
    });

    fn build_node(
        node_id: i64,
        node_names: &HashMap<i64, &str>,
        parent_map: &HashMap<i64, i64>,
        children_map: &HashMap<i64, Vec<i64>>,
        go_to_transform: &HashMap<i64, i64>,
        transform_to_go: &HashMap<i64, i64>,
        stripped_transform_to_prefab: &HashMap<i64, i64>,
        prefab_root_transform: &HashMap<i64, i64>,
        transform_children: &HashMap<i64, Vec<i64>>,
        node_doc_index: &HashMap<i64, usize>,
        go_components: &HashMap<i64, Vec<String>>,
        go_props: &HashMap<i64, (Option<String>, Option<i32>, Option<i64>, Option<i32>)>,
    ) -> HierarchyNode {
        let name = node_names.get(&node_id).copied().unwrap_or("?").to_string();
        let child_ids = ordered_children_for_parent(
            node_id,
            parent_map,
            children_map,
            go_to_transform,
            transform_to_go,
            stripped_transform_to_prefab,
            prefab_root_transform,
            transform_children,
            node_doc_index,
        );
        let children = child_ids
            .iter()
            .map(|&child_id| {
                build_node(
                    child_id,
                    node_names,
                    parent_map,
                    children_map,
                    go_to_transform,
                    transform_to_go,
                    stripped_transform_to_prefab,
                    prefab_root_transform,
                    transform_children,
                    node_doc_index,
                    go_components,
                    go_props,
                )
            })
            .collect();
        let (tag, layer, static_flags, is_active) = go_props
            .get(&node_id)
            .cloned()
            .unwrap_or((None, None, None, None));
        HierarchyNode {
            name,
            file_id: node_id,
            components: go_components.get(&node_id).cloned().unwrap_or_default(),
            children,
            tag: tag.filter(|t| t != "Untagged" && !t.is_empty()),
            layer: layer.filter(|&l| l != 0),
            is_static: static_flags.map_or(false, |f| f != 0),
            is_active: is_active.map_or(true, |a| a != 0),
        }
    }

    roots
        .into_iter()
        .map(|id| {
            build_node(
                id,
                &node_names,
                &parent_map,
                &children_map,
                &go_to_transform,
                &transform_to_go,
                &stripped_transform_to_prefab,
                &prefab_root_transform,
                &transform_children,
                &node_doc_index,
                &go_components,
                &go_props,
            )
        })
        .collect()
}

fn extract_local_transform_data(lines: &[&str], doc: &YamlDoc) -> LocalTransformData {
    let mut data = LocalTransformData::default();
    let mut i = (doc.line_start + 2).min(doc.line_end);

    while i < doc.line_end {
        if let Some((position, consumed)) =
            parse_vector_field(lines, i, doc.line_end, "m_LocalPosition", ["x", "y", "z"])
        {
            data.position = position;
            i += consumed;
            continue;
        }

        if let Some((rotation, consumed)) = parse_vector_field(
            lines,
            i,
            doc.line_end,
            "m_LocalRotation",
            ["x", "y", "z", "w"],
        ) {
            data.rotation = rotation;
            i += consumed;
            continue;
        }

        if let Some((scale, consumed)) =
            parse_vector_field(lines, i, doc.line_end, "m_LocalScale", ["x", "y", "z"])
        {
            data.scale = scale;
            i += consumed;
            continue;
        }

        i += 1;
    }

    data
}

fn parse_vector_field<const N: usize>(
    lines: &[&str],
    pos: usize,
    end: usize,
    field: &str,
    components: [&str; N],
) -> Option<([f64; N], usize)> {
    let line = *lines.get(pos)?;
    let trimmed = line.trim();
    let prefix = format!("{}:", field);
    if !trimmed.starts_with(&prefix) {
        return None;
    }

    let rest = trimmed[prefix.len()..].trim();
    if rest.starts_with('{') {
        return parse_flow_vector(rest, &components).map(|values| (values, 1));
    }
    if !rest.is_empty() {
        return None;
    }

    parse_block_vector(lines, pos, end, &components)
}

fn parse_flow_vector<const N: usize>(text: &str, components: &[&str; N]) -> Option<[f64; N]> {
    let open = text.find('{')?;
    let close = text.rfind('}')?;
    let inner = &text[open + 1..close];

    let mut values = [0.0; N];
    let mut found = [false; N];

    for part in inner.split(',') {
        let part = part.trim();
        let colon = part.find(':')?;
        let key = part[..colon].trim();
        let value = part[colon + 1..].trim().trim_end_matches(',');
        let idx = components.iter().position(|component| *component == key)?;
        values[idx] = value.parse::<f64>().ok()?;
        found[idx] = true;
    }

    found.iter().all(|value| *value).then_some(values)
}

fn parse_block_vector<const N: usize>(
    lines: &[&str],
    pos: usize,
    end: usize,
    components: &[&str; N],
) -> Option<([f64; N], usize)> {
    let line = *lines.get(pos)?;
    let parent_indent = line.len() - line.trim_start().len();

    let mut values = [0.0; N];
    let mut found = [false; N];
    let mut consumed = 1usize;
    let mut child_indent: Option<usize> = None;
    let mut idx = pos + 1;

    while idx < end {
        let child = lines[idx];
        let trimmed = child.trim();
        if trimmed.is_empty() {
            break;
        }

        let indent = child.len() - child.trim_start().len();
        if indent <= parent_indent {
            break;
        }

        let expected_indent = *child_indent.get_or_insert(indent);
        if indent != expected_indent {
            break;
        }

        let colon = trimmed.find(':')?;
        let key = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim().trim_end_matches(',');
        let component_idx = components.iter().position(|component| *component == key)?;
        values[component_idx] = value.parse::<f64>().ok()?;
        found[component_idx] = true;
        consumed += 1;
        idx += 1;
    }

    found
        .iter()
        .all(|value| *value)
        .then_some((values, consumed))
}

fn compute_world_transform_state(
    transform_id: i64,
    local_by_transform: &HashMap<i64, LocalTransformData>,
    parent_by_transform: &HashMap<i64, i64>,
    cache: &mut HashMap<i64, TransformWorldState>,
    visiting: &mut HashSet<i64>,
) -> Option<TransformWorldState> {
    if let Some(state) = cache.get(&transform_id).copied() {
        return Some(state);
    }
    if !visiting.insert(transform_id) {
        return None;
    }

    let local = local_by_transform
        .get(&transform_id)
        .copied()
        .unwrap_or_default();
    let local_rotation = normalize_quaternion(local.rotation);
    let local_matrix = compose_trs_matrix(local.position, local_rotation, local.scale);

    let state = if let Some(parent_id) = parent_by_transform.get(&transform_id).copied() {
        if let Some(parent_state) = compute_world_transform_state(
            parent_id,
            local_by_transform,
            parent_by_transform,
            cache,
            visiting,
        ) {
            TransformWorldState {
                matrix: multiply_matrices(parent_state.matrix, local_matrix),
                rotation: normalize_quaternion(multiply_quaternions(
                    parent_state.rotation,
                    local_rotation,
                )),
            }
        } else {
            TransformWorldState {
                matrix: local_matrix,
                rotation: local_rotation,
            }
        }
    } else {
        TransformWorldState {
            matrix: local_matrix,
            rotation: local_rotation,
        }
    };

    visiting.remove(&transform_id);
    cache.insert(transform_id, state);
    Some(state)
}

fn compose_trs_matrix(position: [f64; 3], rotation: [f64; 4], scale: [f64; 3]) -> [[f64; 4]; 4] {
    let [x, y, z, w] = normalize_quaternion(rotation);
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let wx = w * x;
    let wy = w * y;
    let wz = w * z;

    let rotation_matrix = [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
    ];

    [
        [
            rotation_matrix[0][0] * scale[0],
            rotation_matrix[0][1] * scale[1],
            rotation_matrix[0][2] * scale[2],
            position[0],
        ],
        [
            rotation_matrix[1][0] * scale[0],
            rotation_matrix[1][1] * scale[1],
            rotation_matrix[1][2] * scale[2],
            position[1],
        ],
        [
            rotation_matrix[2][0] * scale[0],
            rotation_matrix[2][1] * scale[1],
            rotation_matrix[2][2] * scale[2],
            position[2],
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn multiply_matrices(left: [[f64; 4]; 4], right: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for row in 0..4 {
        for col in 0..4 {
            for idx in 0..4 {
                result[row][col] += left[row][idx] * right[idx][col];
            }
        }
    }
    result
}

fn multiply_quaternions(left: [f64; 4], right: [f64; 4]) -> [f64; 4] {
    let [lx, ly, lz, lw] = left;
    let [rx, ry, rz, rw] = right;
    [
        lw * rx + lx * rw + ly * rz - lz * ry,
        lw * ry - lx * rz + ly * rw + lz * rx,
        lw * rz + lx * ry - ly * rx + lz * rw,
        lw * rw - lx * rx - ly * ry - lz * rz,
    ]
}

fn normalize_quaternion(rotation: [f64; 4]) -> [f64; 4] {
    let magnitude = (rotation[0] * rotation[0]
        + rotation[1] * rotation[1]
        + rotation[2] * rotation[2]
        + rotation[3] * rotation[3])
        .sqrt();

    if magnitude <= f64::EPSILON {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [
            rotation[0] / magnitude,
            rotation[1] / magnitude,
            rotation[2] / magnitude,
            rotation[3] / magnitude,
        ]
    }
}

fn extract_lossy_scale(matrix: [[f64; 4]; 4]) -> [f64; 3] {
    let mut scale = [
        (matrix[0][0] * matrix[0][0] + matrix[1][0] * matrix[1][0] + matrix[2][0] * matrix[2][0])
            .sqrt(),
        (matrix[0][1] * matrix[0][1] + matrix[1][1] * matrix[1][1] + matrix[2][1] * matrix[2][1])
            .sqrt(),
        (matrix[0][2] * matrix[0][2] + matrix[1][2] * matrix[1][2] + matrix[2][2] * matrix[2][2])
            .sqrt(),
    ];

    let determinant = matrix[0][0] * (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
        - matrix[0][1] * (matrix[1][0] * matrix[2][2] - matrix[1][2] * matrix[2][0])
        + matrix[0][2] * (matrix[1][0] * matrix[2][1] - matrix[1][1] * matrix[2][0]);
    if determinant < 0.0 {
        scale[0] = -scale[0];
    }

    scale.map(sanitize_negative_zero)
}

fn quaternion_to_euler_degrees(rotation: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = normalize_quaternion(rotation);

    let sinr_cosp = 2.0 * (w * x + y * z);
    let cosr_cosp = 1.0 - 2.0 * (x * x + y * y);
    let roll = sinr_cosp.atan2(cosr_cosp);

    let sinp = 2.0 * (w * y - z * x);
    let pitch = if sinp.abs() >= 1.0 {
        sinp.signum() * std::f64::consts::FRAC_PI_2
    } else {
        sinp.asin()
    };

    let siny_cosp = 2.0 * (w * z + x * y);
    let cosy_cosp = 1.0 - 2.0 * (y * y + z * z);
    let yaw = siny_cosp.atan2(cosy_cosp);

    let primary = normalize_unity_euler([roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees()]);
    let alternate = normalize_unity_euler([
        roll.to_degrees() + 180.0,
        180.0 - pitch.to_degrees(),
        yaw.to_degrees() + 180.0,
    ]);

    if unity_euler_score(alternate) < unity_euler_score(primary) {
        alternate
    } else {
        primary
    }
}

fn sanitize_negative_zero(value: f64) -> f64 {
    if value.abs() < 0.000_000_5 {
        0.0
    } else {
        value
    }
}

fn normalize_unity_euler(angles: [f64; 3]) -> [f64; 3] {
    angles.map(|angle| {
        let mut normalized = angle % 360.0;
        if normalized < 0.0 {
            normalized += 360.0;
        }
        if (360.0 - normalized).abs() < 0.000_000_5 {
            normalized = 0.0;
        }
        sanitize_negative_zero(normalized)
    })
}

fn unity_euler_score(angles: [f64; 3]) -> f64 {
    angles
        .into_iter()
        .map(|angle| angle.min(360.0 - angle).abs())
        .sum()
}

fn layer_name(layer: i32) -> &'static str {
    match layer {
        0 => "Default",
        1 => "TransparentFX",
        2 => "Ignore Raycast",
        3 => "Layer3",
        4 => "Water",
        5 => "UI",
        6 => "Layer6",
        7 => "Layer7",
        _ => "",
    }
}

pub(super) fn format_go_annotations(node: &HierarchyNode) -> String {
    let mut parts = Vec::new();
    if node.is_static {
        parts.push("Static".to_string());
    }
    if !node.is_active {
        parts.push("Inactive".to_string());
    }
    if let Some(ref tag) = node.tag {
        parts.push(format!("Tag:{}", tag));
    }
    if let Some(layer) = node.layer {
        let lname = layer_name(layer);
        if lname.is_empty() {
            parts.push(format!("Layer:{}", layer));
        } else {
            parts.push(format!("Layer:{}", lname));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("  [{}]", parts.join(", "))
    }
}

pub fn format_hierarchy_tree(roots: &[HierarchyNode]) -> String {
    let mut out = String::new();
    for (i, root) in roots.iter().enumerate() {
        format_node_root(&mut out, root);
        if i < roots.len() - 1 {
            out.push('\n');
        }
    }
    out
}

fn format_node_root(out: &mut String, node: &HierarchyNode) {
    out.push_str(&node.name);
    out.push_str(&format_go_annotations(node));
    out.push('\n');
    for (i, child) in node.children.iter().enumerate() {
        format_node(out, child, "", i == node.children.len() - 1);
    }
}

fn format_node(out: &mut String, node: &HierarchyNode, prefix: &str, is_last: bool) {
    out.push_str(prefix);
    out.push_str(if is_last { "└── " } else { "├── " });
    out.push_str(&node.name);
    out.push_str(&format_go_annotations(node));
    out.push('\n');

    let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });

    for (i, child) in node.children.iter().enumerate() {
        format_node(out, child, &child_prefix, i == node.children.len() - 1);
    }
}

fn parse_path_segment(segment: &str) -> (&str, Option<usize>) {
    if let Some(base) = segment.strip_suffix(']').and_then(|value| {
        let bracket = value.rfind('[')?;
        let ordinal = value[bracket + 1..].parse::<usize>().ok()?;
        if ordinal == 0 || bracket == 0 {
            return None;
        }
        Some((&value[..bracket], ordinal))
    }) {
        return (base.0, Some(base.1));
    }

    (segment, None)
}

fn find_node_in_siblings<'a>(
    siblings: &'a [HierarchyNode],
    segment: &str,
) -> Option<&'a HierarchyNode> {
    let (name, ordinal) = parse_path_segment(segment);
    if let Some(ordinal) = ordinal {
        if let Some(node) = siblings
            .iter()
            .filter(|node| node.name == name)
            .nth(ordinal.saturating_sub(1))
        {
            return Some(node);
        }
    }

    siblings.iter().find(|node| node.name == segment)
}

pub fn find_hierarchy_node_by_path<'a>(
    roots: &'a [HierarchyNode],
    path: &str,
) -> Option<&'a HierarchyNode> {
    let parts: Vec<&str> = path
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }

    let root = find_node_in_siblings(roots, parts[0])?;
    let mut current = root;

    for &part in &parts[1..] {
        current = find_node_in_siblings(&current.children, part)?;
    }

    Some(current)
}

pub fn find_go_by_path(roots: &[HierarchyNode], path: &str) -> Option<i64> {
    let current = find_hierarchy_node_by_path(roots, path)?;
    Some(current.file_id)
}

pub fn build_hierarchy_path_map(roots: &[HierarchyNode]) -> HashMap<i64, String> {
    fn collect_siblings(
        nodes: &[HierarchyNode],
        parent_path: &str,
        paths: &mut HashMap<i64, String>,
    ) {
        let mut totals: HashMap<&str, usize> = HashMap::new();
        for node in nodes {
            *totals.entry(node.name.as_str()).or_insert(0) += 1;
        }

        let mut ordinals: HashMap<&str, usize> = HashMap::new();
        for node in nodes {
            let total = totals.get(node.name.as_str()).copied().unwrap_or(0);
            let segment = if total > 1 {
                let ordinal = ordinals.entry(node.name.as_str()).or_insert(0);
                *ordinal += 1;
                format!("{}[{}]", node.name, *ordinal)
            } else {
                node.name.clone()
            };
            let path = if parent_path.is_empty() {
                segment
            } else {
                format!("{}/{}", parent_path, segment)
            };
            paths.insert(node.file_id, path.clone());
            collect_siblings(&node.children, &path, paths);
        }
    }

    let mut paths = HashMap::new();
    collect_siblings(roots, "", &mut paths);
    paths
}

pub fn get_components_for_go(docs: &[YamlDoc], go_file_id: i64) -> Vec<usize> {
    let mut result = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        if doc.m_game_object_id == Some(go_file_id) {
            result.push(i);
        }
        if doc.file_id == go_file_id && doc.class_id == 1 {
            result.push(i);
        }
        if doc.file_id == go_file_id && doc.class_id == 1001 {
            result.push(i);
        }
    }
    result.sort();
    result.dedup();
    result
}

/// guid_resolver: guid hex string → Option<asset path>
pub fn format_doc_state_lines(doc: &YamlDoc) -> String {
    let mut out = String::new();
    if let Some(enabled) = doc.m_enabled {
        out.push_str(&format!(
            "  Enabled: {}\n",
            if enabled { "true" } else { "false" }
        ));
    }
    out
}

pub fn resolve_references_in_lines(
    lines: &[&str],
    start: usize,
    end: usize,
    guid_resolver: &dyn Fn(&str) -> Option<String>,
    internal_resolver: &dyn Fn(i64) -> Option<String>,
) -> String {
    resolve_references_in_lines_skipping_fields(
        lines,
        start,
        end,
        guid_resolver,
        internal_resolver,
        &[],
    )
}

pub fn resolve_references_in_lines_skipping_fields(
    lines: &[&str],
    start: usize,
    end: usize,
    guid_resolver: &dyn Fn(&str) -> Option<String>,
    internal_resolver: &dyn Fn(i64) -> Option<String>,
    skipped_fields: &[&str],
) -> String {
    let end = end.min(lines.len());
    let mut out = String::new();
    let mut i = start;

    let mut pending_line: Option<String> = None;
    let mut pending_braces: i32 = 0;

    while i < end {
        let line = lines[i];

        if let Some(ref mut buf) = pending_line {
            buf.push(' ');
            buf.push_str(line.trim());
            pending_braces += count_braces(line);
            if pending_braces <= 0 {
                let complete = pending_line.take().unwrap();
                out.push_str(&resolve_line_refs(
                    &complete,
                    guid_resolver,
                    internal_resolver,
                ));
                out.push('\n');
                pending_braces = 0;
            }
            i += 1;
            continue;
        }

        if should_skip_field(line, skipped_fields) {
            i += 1;
            continue;
        }

        if let Some((merged, consumed)) = try_merge_vector_lines(lines, i, end) {
            out.push_str(&merged);
            out.push('\n');
            i += consumed;
            continue;
        }
        if line.contains('{') && (line.contains("guid:") || line.contains("fileID:")) {
            let balance = count_braces(line);
            if balance > 0 {
                pending_line = Some(line.to_string());
                pending_braces = balance;
                i += 1;
                continue;
            }
            out.push_str(&resolve_line_refs(line, guid_resolver, internal_resolver));
        } else {
            out.push_str(&format_decimal_line(line));
        }
        out.push('\n');
        i += 1;
    }

    if let Some(buf) = pending_line {
        out.push_str(&buf);
        out.push('\n');
    }

    out
}

fn should_skip_field(line: &str, skipped_fields: &[&str]) -> bool {
    if skipped_fields.is_empty() {
        return false;
    }

    extract_field_name_ref(line.trim())
        .map(|field| skipped_fields.iter().any(|skip| *skip == field))
        .unwrap_or(false)
}

fn try_merge_vector_lines(lines: &[&str], pos: usize, end: usize) -> Option<(String, usize)> {
    let parent_line = lines[pos].trim_end();
    let colon_idx = parent_line.find(':')?;
    let after_colon = parent_line[colon_idx + 1..].trim();
    if !after_colon.is_empty() {
        return None;
    }
    if pos + 1 >= end {
        return None;
    }
    let parent_indent = parent_line.len() - parent_line.trim_start().len();
    let child_line = lines[pos + 1];
    let child_indent = child_line.len() - child_line.trim_start().len();
    if child_indent <= parent_indent {
        return None;
    }

    let mut fields: Vec<(&str, &str)> = Vec::new();
    let mut j = pos + 1;
    while j < end {
        let l = lines[j];
        let l_indent = l.len() - l.trim_start().len();
        if l_indent != child_indent {
            break;
        }
        let trimmed = l.trim();
        if let Some(ci) = trimmed.find(':') {
            let key = trimmed[..ci].trim();
            let val = trimmed[ci + 1..].trim();
            fields.push((key, val));
        } else {
            break;
        }
        j += 1;
    }

    let is_vec3 =
        fields.len() == 3 && fields[0].0 == "x" && fields[1].0 == "y" && fields[2].0 == "z";
    let is_vec4 = fields.len() == 4
        && fields[0].0 == "x"
        && fields[1].0 == "y"
        && fields[2].0 == "z"
        && fields[3].0 == "w";
    let is_color = fields.len() == 4
        && fields[0].0 == "r"
        && fields[1].0 == "g"
        && fields[2].0 == "b"
        && fields[3].0 == "a";
    let is_vec2 = fields.len() == 2 && fields[0].0 == "x" && fields[1].0 == "y";

    if !is_vec3 && !is_vec4 && !is_color && !is_vec2 {
        return None;
    }

    let prefix = &parent_line[..parent_line.len()];
    let parts: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}: {}", k, round_decimal_str(v)))
        .collect();
    let merged = format!("{} {{{}}}", prefix, parts.join(", "));

    Some((merged, fields.len() + 1))
}

pub(super) fn round_decimal_str(s: &str) -> String {
    if let Ok(f) = s.parse::<f64>() {
        if s.contains('.') {
            format!("{:.2}", f)
        } else {
            s.to_string()
        }
    } else {
        s.to_string()
    }
}

fn format_decimal_line(line: &str) -> String {
    let trimmed = line.trim();
    if let Some(colon_idx) = trimmed.find(':') {
        let val = trimmed[colon_idx + 1..].trim();
        if !val.is_empty() && val.contains('.') {
            if let Ok(f) = val.parse::<f64>() {
                let indent = &line[..line.len() - trimmed.len()];
                let key = &trimmed[..colon_idx];
                return format!("{}{}: {:.2}", indent, key, f);
            }
        }
    }
    line.to_string()
}

fn resolve_line_refs(
    line: &str,
    guid_resolver: &dyn Fn(&str) -> Option<String>,
    internal_resolver: &dyn Fn(i64) -> Option<String>,
) -> String {
    let bytes = line.as_bytes();
    let mut result = String::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_closing_brace(bytes, i) {
                let block = &line[i..=end];
                let resolved = resolve_single_ref(block, guid_resolver, internal_resolver);
                result.push_str(&resolved);
                i = end + 1;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

fn resolve_single_ref(
    block: &str,
    guid_resolver: &dyn Fn(&str) -> Option<String>,
    internal_resolver: &dyn Fn(i64) -> Option<String>,
) -> String {
    let has_guid = block.contains("guid:");
    let has_file_id = block.contains("fileID:");

    if has_guid {
        if let Some(guid_str) = extract_value(block, "guid:") {
            let guid_hex = guid_str.trim().trim_end_matches(',');
            if let Some(path) = guid_resolver(guid_hex) {
                return format!("{{{}}}", path);
            }
            return format!("{{guid:{}}}", guid_hex);
        }
        "{unresolved external ref}".to_string()
    } else if has_file_id {
        if let Some(fid_str) = extract_value(block, "fileID:") {
            let fid_str = fid_str.trim().trim_end_matches(',');
            if let Ok(fid) = fid_str.parse::<i64>() {
                if fid == 0 {
                    return "{none}".to_string();
                }
                if let Some(desc) = internal_resolver(fid) {
                    return format!("{{{}}}", desc);
                }
            }
        }
        "{unresolved internal ref}".to_string()
    } else {
        block.to_string()
    }
}

pub fn build_internal_id_map(docs: &[YamlDoc]) -> HashMap<i64, String> {
    let tree = build_go_tree(docs);
    let go_paths = build_hierarchy_path_map(&tree);

    let go_names: HashMap<i64, &str> = docs
        .iter()
        .filter(|d| d.class_id == 1 || d.class_id == 1001)
        .filter_map(|d| d.m_name.as_deref().map(|n| (d.file_id, n)))
        .collect();

    let mut map: HashMap<i64, String> = HashMap::new();

    for doc in docs {
        let desc = if doc.class_id == 1 {
            let path = go_paths
                .get(&doc.file_id)
                .cloned()
                .unwrap_or_else(|| doc.m_name.as_deref().unwrap_or("?").to_string());
            format!("GO:{}", path)
        } else if doc.class_id == 1001 {
            let path = go_paths
                .get(&doc.file_id)
                .cloned()
                .unwrap_or_else(|| doc.m_name.as_deref().unwrap_or("?").to_string());
            format!("Prefab:{}", path)
        } else if let Some(go_id) = doc.m_game_object_id {
            let go_path = go_paths
                .get(&go_id)
                .cloned()
                .unwrap_or_else(|| go_names.get(&go_id).copied().unwrap_or("?").to_string());
            format!("{}.{}", go_path, doc.type_name)
        } else if doc.is_stripped {
            if let Some(pi_id) = doc.prefab_instance_id {
                let pi_path = go_paths
                    .get(&pi_id)
                    .cloned()
                    .unwrap_or_else(|| go_names.get(&pi_id).copied().unwrap_or("?").to_string());
                format!("{}.{} (stripped)", pi_path, doc.type_name)
            } else {
                format!("{} (stripped)", doc.type_name)
            }
        } else {
            doc.type_name.clone()
        };
        map.insert(doc.file_id, desc);
    }

    map
}

pub fn collect_guids_from_lines(lines: &[&str], start: usize, end: usize) -> Vec<Guid> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for i in start..end.min(lines.len()) {
        let line = lines[i];
        if !line.contains("guid:") {
            continue;
        }
        let mut pos = 0;
        while let Some(idx) = line[pos..].find("guid:") {
            let after = pos + idx + 5; // skip "guid:"
            let rest = line[after..].trim_start();
            if rest.len() >= 32 {
                let hex = &rest[..32];
                if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    if let Some(guid) = parse_guid_hex(hex) {
                        if guid != [0u8; 16] && seen.insert(guid) {
                            result.push(guid);
                        }
                    }
                }
            }
            pos = after;
        }
    }

    result
}

pub fn collect_guids_from_ranges(lines: &[&str], ranges: &[(usize, usize)]) -> Vec<Guid> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for &(start, end) in ranges {
        for guid in collect_guids_from_lines(lines, start, end) {
            if seen.insert(guid) {
                result.push(guid);
            }
        }
    }

    result
}

pub fn is_hierarchical_file(ext: &str) -> bool {
    matches!(ext.to_lowercase().as_str(), "unity" | "prefab")
}
