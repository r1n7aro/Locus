use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::diff::types::InspectorReference;

/// Intermediate representation for a single list item.
#[derive(Debug, Clone)]
pub struct ListItemIR {
    pub index: usize,
    pub kind: ListItemKind,
    pub match_key: String,
    pub display_label: String,
    /// Ordered descendant fields within this item.
    pub fields: IndexMap<String, ParsedFieldLineIR>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListItemKind {
    /// `{fileID: N, guid: ...}` style object reference.
    ObjectReference,
    /// A plain scalar value (string, number, bool).
    Scalar,
    /// Has sub-fields (nested object).
    CompoundObject,
}

/// Mirror of ParsedFieldLine for list item fields.
#[derive(Debug, Clone)]
pub struct ParsedFieldLineIR {
    pub label: String,
    pub value: Option<String>,
    pub reference: Option<InspectorReference>,
}

/// Result of matching a pair of list items.
#[derive(Debug, Clone)]
pub struct ListMatchPair {
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub display_label: String,
}

/// Build ListItemIR entries from a parent field's children.
pub fn build_list_items(
    items: &[(
        Option<String>,
        Option<InspectorReference>,
        IndexMap<String, ParsedFieldLineIR>,
    )],
) -> Vec<ListItemIR> {
    items
        .iter()
        .enumerate()
        .map(|(index, (value, reference, child_fields))| {
            let (kind, match_key, display_label) = classify_item(value, reference, child_fields);
            ListItemIR {
                index,
                kind,
                match_key,
                display_label,
                fields: child_fields.clone(),
            }
        })
        .collect()
}

/// Classify a single list item and compute its semantic match key + display label.
fn classify_item(
    value: &Option<String>,
    reference: &Option<InspectorReference>,
    child_fields: &IndexMap<String, ParsedFieldLineIR>,
) -> (ListItemKind, String, String) {
    // Priority 1: direct object reference item.
    if let Some(reference) = reference {
        let key = reference_signature(reference);
        let label = reference_display_short(reference);
        return (ListItemKind::ObjectReference, key, label);
    }

    // Priority 2: scalar item.
    if child_fields.is_empty() {
        if let Some(value) = value {
            return (
                ListItemKind::Scalar,
                format!("val:{}", value),
                truncate_display(value),
            );
        }
        return (ListItemKind::Scalar, String::new(), "(empty)".into());
    }

    // Priority 3: common transition/state-machine key pair.
    if let (Some(from), Some(to)) = (child_fields.get("FromState"), child_fields.get("ToState")) {
        if let (Some(from_key), Some(to_key)) = (field_match_value(from), field_match_value(to)) {
            let display = format!(
                "{} -> {}",
                field_display_label(from),
                field_display_label(to)
            );
            return (
                ListItemKind::CompoundObject,
                format!("from:{}|to:{}", from_key, to_key),
                display,
            );
        }
    }

    // Priority 4: well-known semantic keys.
    let key_candidates = [
        "m_Name",
        "name",
        "id",
        "guid",
        "key",
        "propertyPath",
        "Condition",
        "FromState",
        "ToState",
    ];
    for candidate in &key_candidates {
        if let Some(field) = child_fields.get(*candidate) {
            if let Some(key) = field_match_value(field) {
                return (
                    ListItemKind::CompoundObject,
                    key,
                    field_display_label(field),
                );
            }
        }
    }

    // Priority 5: single named child — common for Unity material property maps
    // (m_Colors, m_Floats, m_TexEnvs, etc.) where the child field name IS the
    // identity (e.g. _BaseColor, _MainTex) and the value is the data.
    if child_fields.len() == 1 {
        let (path, field) = child_fields.iter().next().unwrap();
        return (
            ListItemKind::CompoundObject,
            format!("name:{}", path),
            field.label.clone(),
        );
    }

    // Fallback: semantic hash of descendant values/references.
    let hash_key = child_fields
        .iter()
        .map(|(path, field)| format!("{}={}", path, field_match_value(field).unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(";");
    (ListItemKind::CompoundObject, hash_key, "(object)".into())
}

fn reference_signature(reference: &InspectorReference) -> String {
    format!(
        "ref:{}:{}",
        reference.guid.as_deref().unwrap_or(""),
        reference
            .file_id
            .map(|id| id.to_string())
            .unwrap_or_default()
    )
}

fn reference_display(reference: &InspectorReference) -> String {
    if let Some(path) = &reference.path {
        if let Some(file_id) = reference.file_id {
            return format!("{} (fileID:{})", path, file_id);
        }
        return path.clone();
    }
    if let Some(guid) = &reference.guid {
        if let Some(file_id) = reference.file_id {
            return format!("{} (fileID:{})", guid, file_id);
        }
        return guid.clone();
    }
    if let Some(file_id) = reference.file_id {
        return format!("fileID:{}", file_id);
    }
    "(none)".into()
}

fn field_match_value(field: &ParsedFieldLineIR) -> Option<String> {
    field
        .reference
        .as_ref()
        .map(reference_signature)
        .or_else(|| field.value.as_ref().map(|value| format!("val:{}", value)))
}

fn field_display_label(field: &ParsedFieldLineIR) -> String {
    if let Some(reference) = &field.reference {
        return reference_display_short(reference);
    }
    field
        .value
        .as_ref()
        .map(|value| truncate_display(value))
        .unwrap_or_else(|| "(object)".into())
}

/// Short display name for references — just the asset name without path/extension/fileID.
fn reference_display_short(reference: &InspectorReference) -> String {
    if let Some(path) = &reference.path {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let name_no_ext = file_name
            .rsplit_once('.')
            .map(|(name, _)| name)
            .unwrap_or(file_name);
        if !name_no_ext.is_empty() {
            return name_no_ext.to_string();
        }
    }
    if let Some(guid) = &reference.guid {
        if guid.len() > 8 {
            return format!("{}...", &guid[..8]);
        }
        return guid.clone();
    }
    if let Some(file_id) = reference.file_id {
        return format!("fileID:{}", file_id);
    }
    "(none)".into()
}

fn truncate_display(value: &str) -> String {
    if value.chars().count() > 30 {
        let prefix = value.chars().take(27).collect::<String>();
        format!("{prefix}...")
    } else {
        value.to_string()
    }
}

const MAX_LIST_ITEMS_FOR_STABLE_MATCH: usize = 500;

/// Perform stable matching between old and new list items.
pub fn stable_list_match(old_items: &[ListItemIR], new_items: &[ListItemIR]) -> Vec<ListMatchPair> {
    if old_items.len() > MAX_LIST_ITEMS_FOR_STABLE_MATCH
        || new_items.len() > MAX_LIST_ITEMS_FOR_STABLE_MATCH
    {
        return index_fallback_match(old_items, new_items);
    }

    let mut old_by_key: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, item) in old_items.iter().enumerate() {
        old_by_key.entry(&item.match_key).or_default().push(i);
    }

    let mut new_by_key: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, item) in new_items.iter().enumerate() {
        new_by_key.entry(&item.match_key).or_default().push(i);
    }

    let mut old_matched = vec![false; old_items.len()];
    let mut new_matched = vec![false; new_items.len()];
    let mut pairs = Vec::new();
    let mut seen_keys = Vec::new();
    let mut seen_set = HashSet::new();

    for item in new_items {
        if seen_set.insert(item.match_key.as_str()) {
            seen_keys.push(item.match_key.as_str());
        }
    }
    for item in old_items {
        if seen_set.insert(item.match_key.as_str()) {
            seen_keys.push(item.match_key.as_str());
        }
    }

    for key in seen_keys {
        let old_indices = old_by_key.get(key).cloned().unwrap_or_default();
        let new_indices = new_by_key.get(key).cloned().unwrap_or_default();
        let pair_count = old_indices.len().min(new_indices.len());

        for i in 0..pair_count {
            let old_index = old_indices[i];
            let new_index = new_indices[i];
            old_matched[old_index] = true;
            new_matched[new_index] = true;
            pairs.push(ListMatchPair {
                old_index: Some(old_index),
                new_index: Some(new_index),
                display_label: new_items[new_index].display_label.clone(),
            });
        }

        for &old_index in &old_indices[pair_count..] {
            old_matched[old_index] = true;
            pairs.push(ListMatchPair {
                old_index: Some(old_index),
                new_index: None,
                display_label: old_items[old_index].display_label.clone(),
            });
        }

        for &new_index in &new_indices[pair_count..] {
            new_matched[new_index] = true;
            pairs.push(ListMatchPair {
                old_index: None,
                new_index: Some(new_index),
                display_label: new_items[new_index].display_label.clone(),
            });
        }
    }

    pairs.sort_by_key(|pair| match (pair.new_index, pair.old_index) {
        (Some(new_index), _) => (new_index * 2, 0),
        (None, Some(old_index)) => (old_index * 2 + 1, 1),
        _ => (usize::MAX, 2),
    });

    pairs
}

fn index_fallback_match(old_items: &[ListItemIR], new_items: &[ListItemIR]) -> Vec<ListMatchPair> {
    let max_len = old_items.len().max(new_items.len());
    let mut pairs = Vec::with_capacity(max_len);

    for index in 0..max_len {
        let old_index = (index < old_items.len()).then_some(index);
        let new_index = (index < new_items.len()).then_some(index);
        let display_label = new_index
            .map(|i| new_items[i].display_label.clone())
            .or_else(|| old_index.map(|i| old_items[i].display_label.clone()))
            .unwrap_or_else(|| format!("[{}]", index));
        pairs.push(ListMatchPair {
            old_index,
            new_index,
            display_label,
        });
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::truncate_display;

    #[test]
    fn truncate_display_handles_unicode_boundaries() {
        assert_eq!(
            truncate_display(&"项".repeat(31)),
            format!("{}...", "项".repeat(27))
        );
    }
}
