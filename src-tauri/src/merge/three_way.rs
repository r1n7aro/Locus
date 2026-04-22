use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::asset_db::types::{guid_to_hex, Guid};
use crate::diff::semantic::parse::prettify_builtin_field_label;
use crate::diff::semantic::parse::{join_property_segments, split_property_path};
use crate::diff::semantic::ParsedFieldLine;
use crate::diff::types::InspectorReference;
use crate::unity_yaml::YamlDoc;

use super::types::*;

#[derive(Debug, Default, Clone)]
pub(crate) struct FieldTreeNode3 {
    pub(crate) label: String,
    pub(crate) path: String,
    pub(crate) base_entry: Option<ParsedFieldLine>,
    pub(crate) ours_entry: Option<ParsedFieldLine>,
    pub(crate) theirs_entry: Option<ParsedFieldLine>,
    pub(crate) children: IndexMap<String, FieldTreeNode3>,
}

pub(crate) fn auto_merge_field(
    base: Option<&str>,
    ours: Option<&str>,
    theirs: Option<&str>,
) -> (MergeState, Option<MergeSide>, Option<String>) {
    let b = base.unwrap_or("");
    let o = ours.unwrap_or("");
    let t = theirs.unwrap_or("");

    if b == o && b == t {
        (
            MergeState::Unchanged,
            Some(MergeSide::Base),
            Some(b.to_string()),
        )
    } else if b == o && b != t {
        (
            MergeState::Auto,
            Some(MergeSide::Theirs),
            Some(t.to_string()),
        )
    } else if b != o && b == t {
        (MergeState::Auto, Some(MergeSide::Ours), Some(o.to_string()))
    } else if o == t {
        (MergeState::Auto, Some(MergeSide::Ours), Some(o.to_string()))
    } else {
        (MergeState::Conflict, None, None)
    }
}

pub(crate) fn references_equal(
    a: Option<&InspectorReference>,
    b: Option<&InspectorReference>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.guid == b.guid && a.file_id == b.file_id,
        _ => false,
    }
}

fn infer_value_type(val: Option<&str>) -> String {
    match val {
        None => "string".to_string(),
        Some(v) => {
            if v == "1"
                || v == "0"
                || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("false")
            {
                "bool".to_string()
            } else if v.parse::<f64>().is_ok() {
                "number".to_string()
            } else {
                "string".to_string()
            }
        }
    }
}

fn insert_merge_field_node(
    root: &mut FieldTreeNode3,
    path: &str,
    base_map: &HashMap<String, ParsedFieldLine>,
    ours_map: &HashMap<String, ParsedFieldLine>,
    theirs_map: &HashMap<String, ParsedFieldLine>,
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
            .or_insert_with(|| FieldTreeNode3 {
                label: segment.clone(),
                path: current_path.clone(),
                base_entry: base_map.get(&current_path).cloned(),
                ours_entry: ours_map.get(&current_path).cloned(),
                theirs_entry: theirs_map.get(&current_path).cloned(),
                children: IndexMap::new(),
            });
        if cursor.base_entry.is_none() {
            cursor.base_entry = base_map.get(&current_path).cloned();
        }
        if cursor.ours_entry.is_none() {
            cursor.ours_entry = ours_map.get(&current_path).cloned();
        }
        if cursor.theirs_entry.is_none() {
            cursor.theirs_entry = theirs_map.get(&current_path).cloned();
        }
    }
}

fn build_merge_field(
    node: &FieldTreeNode3,
    doc_key: &str,
    field_type_map: &HashMap<String, String>,
    include_unchanged: bool,
) -> Option<MergeField> {
    let field_id = format!("{}|{}", doc_key, node.path);
    let label = node
        .ours_entry
        .as_ref()
        .map(|entry| entry.label.clone())
        .or_else(|| node.theirs_entry.as_ref().map(|entry| entry.label.clone()))
        .or_else(|| node.base_entry.as_ref().map(|entry| entry.label.clone()))
        .unwrap_or_else(|| prettify_builtin_field_label(&node.label));

    if node.children.is_empty() {
        let base_val = node.base_entry.as_ref().and_then(|e| e.value.as_deref());
        let ours_val = node.ours_entry.as_ref().and_then(|e| e.value.as_deref());
        let theirs_val = node.theirs_entry.as_ref().and_then(|e| e.value.as_deref());

        let base_ref = node.base_entry.as_ref().and_then(|e| e.reference.as_ref());
        let ours_ref = node.ours_entry.as_ref().and_then(|e| e.reference.as_ref());
        let theirs_ref = node
            .theirs_entry
            .as_ref()
            .and_then(|e| e.reference.as_ref());

        let (merge_state, auto_choice, result) =
            if base_ref.is_some() || ours_ref.is_some() || theirs_ref.is_some() {
                let b_eq_o = references_equal(base_ref, ours_ref) && base_val == ours_val;
                let b_eq_t = references_equal(base_ref, theirs_ref) && base_val == theirs_val;
                let o_eq_t = references_equal(ours_ref, theirs_ref) && ours_val == theirs_val;

                if b_eq_o && b_eq_t {
                    (
                        MergeState::Unchanged,
                        Some(MergeSide::Base),
                        base_val.map(|s| s.to_string()),
                    )
                } else if b_eq_o && !b_eq_t {
                    (
                        MergeState::Auto,
                        Some(MergeSide::Theirs),
                        theirs_val.map(|s| s.to_string()),
                    )
                } else if !b_eq_o && b_eq_t {
                    (
                        MergeState::Auto,
                        Some(MergeSide::Ours),
                        ours_val.map(|s| s.to_string()),
                    )
                } else if o_eq_t {
                    (
                        MergeState::Auto,
                        Some(MergeSide::Ours),
                        ours_val.map(|s| s.to_string()),
                    )
                } else {
                    (MergeState::Conflict, None, None)
                }
            } else {
                auto_merge_field(base_val, ours_val, theirs_val)
            };

        if !include_unchanged && merge_state == MergeState::Unchanged {
            return None;
        }

        let value_type = if base_ref.is_some() || ours_ref.is_some() || theirs_ref.is_some() {
            "reference".to_string()
        } else {
            infer_value_type(base_val.or(ours_val).or(theirs_val))
        };

        return Some(MergeField {
            id: field_id,
            property_path: node.path.clone(),
            label,
            value_type,
            base: base_val.map(|s| s.to_string()),
            ours: ours_val.map(|s| s.to_string()),
            theirs: theirs_val.map(|s| s.to_string()),
            result,
            merge_state,
            auto_choice,
            manual_choice: None,
            children: Vec::new(),
            field_type: field_type_map.get(&node.path).cloned(),
            reference_base: base_ref.cloned(),
            reference_ours: ours_ref.cloned(),
            reference_theirs: theirs_ref.cloned(),
        });
    }

    let mut children = Vec::new();
    for child in node.children.values() {
        if let Some(field) = build_merge_field(child, doc_key, field_type_map, include_unchanged) {
            children.push(field);
        }
    }

    let direct_has_value =
        node.base_entry.is_some() || node.ours_entry.is_some() || node.theirs_entry.is_some();
    if children.is_empty() && (!include_unchanged || !direct_has_value) {
        return None;
    }

    let has_conflict = children
        .iter()
        .any(|c| c.merge_state == MergeState::Conflict);
    let has_auto = children.iter().any(|c| c.merge_state == MergeState::Auto);
    let merge_state = if has_conflict {
        MergeState::Conflict
    } else if has_auto {
        MergeState::Auto
    } else {
        MergeState::Unchanged
    };

    if !include_unchanged && merge_state == MergeState::Unchanged {
        return None;
    }

    Some(MergeField {
        id: field_id,
        property_path: node.path.clone(),
        label,
        value_type: "group".to_string(),
        base: None,
        ours: None,
        theirs: None,
        result: None,
        merge_state,
        auto_choice: None,
        manual_choice: None,
        children,
        field_type: None,
        reference_base: None,
        reference_ours: None,
        reference_theirs: None,
    })
}

pub(crate) fn build_merge_fields(
    doc_key: &str,
    paths: Vec<String>,
    base_map: &HashMap<String, ParsedFieldLine>,
    ours_map: &HashMap<String, ParsedFieldLine>,
    theirs_map: &HashMap<String, ParsedFieldLine>,
    field_type_map: &HashMap<String, String>,
) -> Vec<MergeField> {
    let mut root = FieldTreeNode3::default();
    for path in paths {
        insert_merge_field_node(&mut root, &path, base_map, ours_map, theirs_map);
    }

    let mut fields = Vec::new();
    for child in root.children.values() {
        if let Some(field) = build_merge_field(child, doc_key, field_type_map, false) {
            fields.push(field);
        }
    }
    fields
}

pub(crate) fn count_merge_leaf_states(fields: &[MergeField]) -> (usize, usize) {
    let mut conflicts = 0usize;
    let mut autos = 0usize;
    for field in fields {
        if field.children.is_empty() {
            match field.merge_state {
                MergeState::Conflict => conflicts += 1,
                MergeState::Auto => autos += 1,
                MergeState::Unchanged => {}
            }
        } else {
            let (child_conflicts, child_autos) = count_merge_leaf_states(&field.children);
            conflicts += child_conflicts;
            autos += child_autos;
        }
    }
    (conflicts, autos)
}

pub(crate) fn collect_conflict_field_ids(fields: &[MergeField], out: &mut Vec<String>) {
    for field in fields {
        if field.children.is_empty() {
            if field.merge_state == MergeState::Conflict {
                out.push(field.id.clone());
            }
        } else {
            collect_conflict_field_ids(&field.children, out);
        }
    }
}

pub(crate) fn doc_merge_status(
    base_exists: bool,
    ours_exists: bool,
    theirs_exists: bool,
    conflict_count: usize,
    auto_count: usize,
) -> DocMergeStatus {
    match (base_exists, ours_exists, theirs_exists) {
        (true, true, true) => {
            if conflict_count > 0 {
                DocMergeStatus::HasConflicts
            } else if auto_count > 0 {
                DocMergeStatus::AutoResolved
            } else {
                DocMergeStatus::Unchanged
            }
        }
        (true, true, false) => DocMergeStatus::RemovedTheirs,
        (true, false, true) => DocMergeStatus::RemovedOurs,
        (true, false, false) => DocMergeStatus::AutoResolved,
        (false, true, false) => DocMergeStatus::AddedOurs,
        (false, false, true) => DocMergeStatus::AddedTheirs,
        (false, true, true) => {
            if conflict_count > 0 {
                DocMergeStatus::HasConflicts
            } else {
                DocMergeStatus::AutoResolved
            }
        }
        (false, false, false) => DocMergeStatus::Unchanged,
    }
}

pub(crate) fn change_kind_for_status(status: DocMergeStatus) -> String {
    match status {
        DocMergeStatus::HasConflicts => "conflict",
        DocMergeStatus::AutoResolved => "autoResolved",
        DocMergeStatus::AddedOurs => "oursOnly",
        DocMergeStatus::AddedTheirs => "theirsOnly",
        DocMergeStatus::RemovedOurs | DocMergeStatus::RemovedTheirs => "removed",
        DocMergeStatus::Unchanged => "unchanged",
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AssetDocMatchGroupKey {
    class_id: i32,
    script_guid: Option<Guid>,
    m_name: Option<String>,
}

impl AssetDocMatchGroupKey {
    fn from_doc(doc: &YamlDoc) -> Self {
        Self {
            class_id: doc.class_id,
            script_guid: doc.m_script_guid,
            m_name: doc.m_name.clone(),
        }
    }
}

fn asset_doc_match_group_label(key: &AssetDocMatchGroupKey) -> String {
    let script_part = key
        .script_guid
        .map(|guid| guid_to_hex(&guid))
        .unwrap_or_default();
    let name_part = key.m_name.clone().unwrap_or_default();
    format!("{}:{}:{}", key.class_id, script_part, name_part)
}

#[derive(Debug, Clone)]
pub(crate) struct MatchedAssetDoc<'a> {
    pub(crate) key: String,
    pub(crate) base_doc: Option<&'a YamlDoc>,
    pub(crate) ours_doc: Option<&'a YamlDoc>,
    pub(crate) theirs_doc: Option<&'a YamlDoc>,
    pub(crate) sort_group: usize,
    pub(crate) sort_order: usize,
}

fn group_asset_docs<'a>(docs: &'a [YamlDoc]) -> HashMap<AssetDocMatchGroupKey, Vec<&'a YamlDoc>> {
    let mut grouped: HashMap<AssetDocMatchGroupKey, Vec<&'a YamlDoc>> = HashMap::new();
    for doc in docs {
        grouped
            .entry(AssetDocMatchGroupKey::from_doc(doc))
            .or_default()
            .push(doc);
    }
    for group in grouped.values_mut() {
        group.sort_by_key(|doc| doc.doc_index);
    }
    grouped
}

pub(crate) fn match_asset_docs_three_way<'a>(
    base_docs: &'a [YamlDoc],
    ours_docs: &'a [YamlDoc],
    theirs_docs: &'a [YamlDoc],
) -> Vec<MatchedAssetDoc<'a>> {
    let base_grouped = group_asset_docs(base_docs);
    let ours_grouped = group_asset_docs(ours_docs);
    let theirs_grouped = group_asset_docs(theirs_docs);

    let mut all_group_keys = Vec::new();
    let mut seen = HashSet::new();
    for docs in [base_docs, ours_docs, theirs_docs] {
        for doc in docs {
            let key = AssetDocMatchGroupKey::from_doc(doc);
            if seen.insert(key.clone()) {
                all_group_keys.push(key);
            }
        }
    }

    let mut matched = Vec::new();
    for group_key in all_group_keys {
        let group_label = asset_doc_match_group_label(&group_key);
        let base_group = base_grouped.get(&group_key).cloned().unwrap_or_default();
        let ours_group = ours_grouped.get(&group_key).cloned().unwrap_or_default();
        let theirs_group = theirs_grouped.get(&group_key).cloned().unwrap_or_default();
        let slot_count = base_group
            .len()
            .max(ours_group.len())
            .max(theirs_group.len());

        for slot in 0..slot_count {
            let base_doc = base_group.get(slot).copied();
            let ours_doc = ours_group.get(slot).copied();
            let theirs_doc = theirs_group.get(slot).copied();
            let sort_group = if base_doc.is_some() { 0 } else { 1 };
            let sort_order = base_doc
                .map(|doc| doc.line_start)
                .or_else(|| ours_doc.map(|doc| doc.line_start + 100_000))
                .or_else(|| theirs_doc.map(|doc| doc.line_start + 200_000))
                .unwrap_or(usize::MAX);
            matched.push(MatchedAssetDoc {
                key: format!("{group_label}:{slot}"),
                base_doc,
                ours_doc,
                theirs_doc,
                sort_group,
                sort_order,
            });
        }
    }

    matched.sort_by_key(|doc| (doc.sort_group, doc.sort_order));
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_merge_unchanged() {
        let (state, side, result) = auto_merge_field(Some("10"), Some("10"), Some("10"));
        assert_eq!(state, MergeState::Unchanged);
        assert_eq!(side, Some(MergeSide::Base));
        assert_eq!(result, Some("10".to_string()));
    }

    #[test]
    fn test_auto_merge_ours_only() {
        let (state, side, result) = auto_merge_field(Some("10"), Some("20"), Some("10"));
        assert_eq!(state, MergeState::Auto);
        assert_eq!(side, Some(MergeSide::Ours));
        assert_eq!(result, Some("20".to_string()));
    }

    #[test]
    fn test_auto_merge_theirs_only() {
        let (state, side, result) = auto_merge_field(Some("10"), Some("10"), Some("20"));
        assert_eq!(state, MergeState::Auto);
        assert_eq!(side, Some(MergeSide::Theirs));
        assert_eq!(result, Some("20".to_string()));
    }

    #[test]
    fn test_auto_merge_both_same() {
        let (state, side, result) = auto_merge_field(Some("10"), Some("20"), Some("20"));
        assert_eq!(state, MergeState::Auto);
        assert_eq!(side, Some(MergeSide::Ours));
        assert_eq!(result, Some("20".to_string()));
    }

    #[test]
    fn test_auto_merge_conflict() {
        let (state, side, result) = auto_merge_field(Some("10"), Some("20"), Some("30"));
        assert_eq!(state, MergeState::Conflict);
        assert_eq!(side, None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_doc_merge_status_added() {
        assert_eq!(
            doc_merge_status(false, true, false, 0, 1),
            DocMergeStatus::AddedOurs
        );
    }

    fn make_yaml_doc(
        file_id: i64,
        class_id: i32,
        m_name: Option<&str>,
        doc_index: usize,
        line_start: usize,
    ) -> YamlDoc {
        YamlDoc {
            file_id,
            class_id,
            type_name: "MonoBehaviour".into(),
            line_start,
            line_end: line_start + 5,
            m_name: m_name.map(str::to_string),
            m_game_object_id: None,
            m_father_id: None,
            is_stripped: false,
            source_prefab_guid: None,
            transform_parent_id: None,
            prefab_instance_id: None,
            m_layer: None,
            m_tag_string: None,
            m_static_editor_flags: None,
            m_is_active: None,
            m_enabled: None,
            transform_root_order: None,
            transform_children: Vec::new(),
            m_script_guid: None,
            doc_index,
        }
    }

    #[test]
    fn match_asset_docs_three_way_ignores_unrelated_doc_index_shift() {
        let base_docs = vec![make_yaml_doc(10, 114, Some("Foo"), 0, 10)];
        let ours_docs = vec![
            make_yaml_doc(1, 1, Some("Root"), 0, 0),
            make_yaml_doc(20, 114, Some("Foo"), 1, 20),
        ];
        let theirs_docs = vec![make_yaml_doc(30, 114, Some("Foo"), 0, 30)];

        let matched = match_asset_docs_three_way(&base_docs, &ours_docs, &theirs_docs);
        let foo = matched
            .iter()
            .find(|entry| entry.base_doc.map(|doc| doc.file_id) == Some(10))
            .expect("matched Foo doc");

        assert_eq!(foo.ours_doc.map(|doc| doc.file_id), Some(20));
        assert_eq!(foo.theirs_doc.map(|doc| doc.file_id), Some(30));
    }
}
