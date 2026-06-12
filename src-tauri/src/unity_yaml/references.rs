use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{parse_guid_hex, ExtractedRef, Guid};

use super::parser::YamlDoc;
use super::tokenizer::{extract_value, find_closing_brace};

/// A guid-bearing flow map captured during the single YAML parse pass, before
/// per-document hierarchy paths are resolved. Produced by
/// `parser::parse_yaml_docs_with_refs` alongside the `YamlDoc` list so callers
/// never pay a second line scan just to build reference edges.
#[derive(Debug, Clone)]
pub struct RawYamlRef {
    pub dst_guid: Guid,
    pub dst_file_id: Option<i64>,
    pub class_id_hint: Option<i32>,
    pub field_hint: Option<String>,
    pub src_doc_file_id: i64,
}

/// File-scoped dedupe set threaded through `extract_flow_maps_raw`: two flow
/// maps in the same document with the same target and field collapse to one
/// raw ref, matching the old two-pass extractor.
pub(super) type RawRefSeen = HashSet<(Guid, Option<i64>, Option<String>, i64)>;

pub fn extract_refs(content: &[u8]) -> Vec<ExtractedRef> {
    extract_refs_with_resolver(content, None)
}

pub fn extract_refs_with_resolver(
    content: &[u8],
    guid_to_path: Option<&HashMap<Guid, String>>,
) -> Vec<ExtractedRef> {
    let (docs, raw_refs) = super::parser::parse_yaml_docs_with_refs(content);
    build_refs_from_docs(&docs, raw_refs, guid_to_path)
}

/// Resolve raw refs into `ExtractedRef`s with human-readable `ref_path`
/// hierarchy strings. `docs` must come from the same parse that produced
/// `raw_refs` (see `parse_yaml_docs_with_refs`); callers that already need
/// the docs for other indexing reuse them here instead of re-parsing.
pub fn build_refs_from_docs(
    docs: &[YamlDoc],
    raw_refs: Vec<RawYamlRef>,
    guid_to_path: Option<&HashMap<Guid, String>>,
) -> Vec<ExtractedRef> {
    let doc_map: HashMap<i64, &YamlDoc> = docs.iter().map(|d| (d.file_id, d)).collect();

    // GameObject fileID → m_Name
    let go_names: HashMap<i64, &str> = docs
        .iter()
        .filter(|d| d.class_id == 1)
        .filter_map(|d| d.m_name.as_deref().map(|n| (d.file_id, n)))
        .collect();

    let mut go_to_transform: HashMap<i64, i64> = HashMap::new();
    let mut transform_father: HashMap<i64, i64> = HashMap::new();
    let mut transform_to_go: HashMap<i64, i64> = HashMap::new();

    for doc in docs {
        if doc.class_id == 4 || doc.class_id == 224 {
            if let Some(go_id) = doc.m_game_object_id {
                if go_id != 0 {
                    go_to_transform.insert(go_id, doc.file_id);
                    transform_to_go.insert(doc.file_id, go_id);
                }
            }
            if let Some(father) = doc.m_father_id {
                transform_father.insert(doc.file_id, father);
            }
        }
    }

    let script_class_names: HashMap<i64, String> = if let Some(g2p) = guid_to_path {
        raw_refs
            .iter()
            .filter(|r| r.field_hint.as_deref() == Some("m_Script"))
            .filter_map(|r| {
                let path = g2p.get(&r.dst_guid)?;
                let class_name = std::path::Path::new(path)
                    .file_stem()?
                    .to_str()?
                    .to_string();
                Some((r.src_doc_file_id, class_name))
            })
            .collect()
    } else {
        HashMap::new()
    };

    let pi_names: HashMap<i64, &str> = docs
        .iter()
        .filter(|d| d.class_id == 1001)
        .filter_map(|d| d.m_name.as_deref().map(|n| (d.file_id, n)))
        .collect();

    let stripped_to_pi: HashMap<i64, i64> = docs
        .iter()
        .filter(|d| d.is_stripped)
        .filter_map(|d| d.prefab_instance_id.map(|pi| (d.file_id, pi)))
        .collect();

    raw_refs
        .into_iter()
        .map(|raw| {
            let ref_path = build_single_path(
                &raw,
                &doc_map,
                &go_names,
                &go_to_transform,
                &transform_father,
                &transform_to_go,
                &script_class_names,
                &pi_names,
                &stripped_to_pi,
            );
            ExtractedRef {
                src_file_id: Some(raw.src_doc_file_id),
                dst_guid: raw.dst_guid,
                dst_file_id: raw.dst_file_id,
                class_id_hint: raw.class_id_hint,
                field_hint: raw.field_hint,
                ref_path,
            }
        })
        .collect()
}

fn build_single_path(
    raw: &RawYamlRef,
    doc_map: &HashMap<i64, &YamlDoc>,
    go_names: &HashMap<i64, &str>,
    go_to_transform: &HashMap<i64, i64>,
    transform_father: &HashMap<i64, i64>,
    transform_to_go: &HashMap<i64, i64>,
    script_class_names: &HashMap<i64, String>,
    pi_names: &HashMap<i64, &str>,
    stripped_to_pi: &HashMap<i64, i64>,
) -> Option<String> {
    let doc = doc_map.get(&raw.src_doc_file_id)?;
    let field = raw.field_hint.as_deref().unwrap_or("?");
    let resolved_name: Option<&str> = if doc.type_name == "MonoBehaviour" {
        script_class_names
            .get(&raw.src_doc_file_id)
            .map(|s| s.as_str())
    } else {
        None
    };
    let type_name = resolved_name.unwrap_or(if doc.type_name.is_empty() {
        "?"
    } else {
        &doc.type_name
    });

    let pi_name_for_doc = if doc.class_id == 1001 {
        pi_names.get(&doc.file_id).copied()
    } else if doc.is_stripped {
        stripped_to_pi
            .get(&doc.file_id)
            .and_then(|pi_id| pi_names.get(pi_id).copied())
    } else {
        None
    };

    let hierarchy = if let Some(go_id) = doc.m_game_object_id {
        let mut h = get_go_hierarchy(
            go_id,
            go_names,
            go_to_transform,
            transform_father,
            transform_to_go,
        );
        if h.is_empty() {
            if let Some(pi_name) = pi_name_for_doc {
                h.insert(0, pi_name.to_string());
            }
        }
        h
    } else if doc.class_id == 1 {
        let mut h = get_go_hierarchy(
            doc.file_id,
            go_names,
            go_to_transform,
            transform_father,
            transform_to_go,
        );
        if h.is_empty() {
            if let Some(pi_name) = pi_name_for_doc {
                h.insert(0, pi_name.to_string());
            }
        }
        h
    } else if doc.class_id == 1001 {
        if let Some(pi_name) = pi_name_for_doc {
            vec![pi_name.to_string()]
        } else {
            Vec::new()
        }
    } else if doc.is_stripped {
        if let Some(pi_name) = pi_name_for_doc {
            vec![pi_name.to_string()]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut parts: Vec<&str> = hierarchy.iter().map(|s| s.as_str()).collect();
    parts.push(type_name);
    parts.push(field);
    Some(parts.join("/"))
}

fn get_go_hierarchy(
    go_id: i64,
    go_names: &HashMap<i64, &str>,
    go_to_transform: &HashMap<i64, i64>,
    transform_father: &HashMap<i64, i64>,
    transform_to_go: &HashMap<i64, i64>,
) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = go_id;
    let mut visited = HashSet::new();

    loop {
        if !visited.insert(current) {
            break;
        }

        if let Some(name) = go_names.get(&current).copied() {
            parts.push(name.to_string());
        }

        let tid = match go_to_transform.get(&current) {
            Some(t) => *t,
            None => break,
        };
        let fid = match transform_father.get(&tid) {
            Some(f) => *f,
            None => break,
        };
        let parent_go = match transform_to_go.get(&fid) {
            Some(g) => *g,
            None => break,
        };
        current = parent_go;
    }

    parts.reverse();
    parts
}

/// Scan one complete (possibly brace-joined) line for `{... guid: ...}` flow
/// maps and append a raw ref per map. Called from the parser's main loop so
/// docs and refs come out of a single pass over the file.
pub(super) fn extract_flow_maps_raw(
    line: &str,
    class_id: Option<i32>,
    last_field: &Option<String>,
    doc_file_id: i64,
    refs: &mut Vec<RawYamlRef>,
    seen: &mut RawRefSeen,
) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = find_closing_brace(bytes, i) {
                let block = &line[i..=end];
                if let Some(raw) = parse_flow_map_raw(block, class_id, last_field, doc_file_id) {
                    let key = (
                        raw.dst_guid,
                        raw.dst_file_id,
                        raw.field_hint.clone(),
                        raw.src_doc_file_id,
                    );
                    if seen.insert(key) {
                        refs.push(raw);
                    }
                }
                i = end + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

fn parse_flow_map_raw(
    block: &str,
    class_id: Option<i32>,
    last_field: &Option<String>,
    doc_file_id: i64,
) -> Option<RawYamlRef> {
    let guid_str = extract_value(block, "guid:")?;
    let guid_str = guid_str.trim().trim_end_matches(',');
    let dst_guid = parse_guid_hex(guid_str)?;

    if dst_guid == [0u8; 16] {
        return None;
    }

    let dst_file_id = extract_value(block, "fileID:")
        .and_then(|v| v.trim().trim_end_matches(',').parse::<i64>().ok());

    Some(RawYamlRef {
        dst_guid,
        dst_file_id,
        class_id_hint: class_id,
        field_hint: last_field.clone(),
        src_doc_file_id: doc_file_id,
    })
}
