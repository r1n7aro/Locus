use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{parse_guid_hex, ExtractedRef, Guid};

use super::tokenizer::{
    count_braces, extract_field_name, extract_internal_file_id, extract_plain_value, extract_value,
    find_closing_brace, parse_doc_header_full,
};

struct DocMeta {
    file_id: i64,
    class_id: i32,
    type_name: String,
    m_name: Option<String>,
    m_game_object_id: Option<i64>,
    m_father_id: Option<i64>,
    is_stripped: bool,
    m_prefab_instance_id: Option<i64>,
}

struct RawRef {
    dst_guid: Guid,
    dst_file_id: Option<i64>,
    class_id_hint: Option<i32>,
    field_hint: Option<String>,
    src_doc_file_id: i64,
}

pub fn extract_refs(content: &[u8]) -> Vec<ExtractedRef> {
    extract_refs_with_resolver(content, None)
}

pub fn extract_refs_with_resolver(
    content: &[u8],
    guid_to_path: Option<&HashMap<Guid, String>>,
) -> Vec<ExtractedRef> {
    let text = String::from_utf8_lossy(content);
    // Stream `text.lines()` directly into phase1_parse instead of collecting
    // into a Vec<&str>. Typical Unity scene/prefab files are 10k-30k lines,
    // so the old `collect()` was allocating ~250-720 KiB of fat-pointer
    // headers per file just to make a single sequential pass. Across 3.6k
    // yaml files in the rayon par_iter that's gigabytes of allocator
    // traffic and L2 cache pressure for zero algorithmic benefit.
    let (docs, raw_refs) = phase1_parse(text.lines());
    phase2_build_paths(&docs, raw_refs, guid_to_path)
}

fn phase1_parse<'a, I: IntoIterator<Item = &'a str>>(lines: I) -> (Vec<DocMeta>, Vec<RawRef>) {
    let mut docs: Vec<DocMeta> = Vec::new();
    let mut raw_refs: Vec<RawRef> = Vec::new();
    let mut seen: HashSet<([u8; 16], Option<i64>, Option<String>, i64)> = HashSet::new();

    let mut cur_class_id: Option<i32> = None;
    let mut cur_file_id: i64 = 0;
    let mut cur_type_name: Option<String> = None;
    let mut cur_m_name: Option<String> = None;
    let mut cur_m_go_id: Option<i64> = None;
    let mut cur_m_father: Option<i64> = None;
    let mut cur_is_stripped: bool = false;
    let mut cur_m_prefab_instance_id: Option<i64> = None;
    let mut first_key = false;
    let mut last_field: Option<String> = None;
    let mut awaiting_name_value: bool = false;

    let mut pending_line: Option<String> = None;
    let mut pending_braces: i32 = 0;

    macro_rules! flush_doc {
        () => {
            if cur_file_id != 0 {
                if let Some(cid) = cur_class_id {
                    docs.push(DocMeta {
                        file_id: cur_file_id,
                        class_id: cid,
                        type_name: cur_type_name.take().unwrap_or_default(),
                        m_name: cur_m_name.take(),
                        m_game_object_id: cur_m_go_id.take(),
                        m_father_id: cur_m_father.take(),
                        is_stripped: cur_is_stripped,
                        m_prefab_instance_id: cur_m_prefab_instance_id.take(),
                    });
                }
            }
        };
    }

    for line in lines {
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
                process_flow_line(
                    ct,
                    cur_class_id,
                    cur_is_stripped,
                    &last_field,
                    cur_file_id,
                    &mut cur_m_go_id,
                    &mut cur_m_father,
                    &mut cur_m_prefab_instance_id,
                    &mut raw_refs,
                    &mut seen,
                );
                pending_braces = 0;
            }
            continue;
        }

        if trimmed.starts_with("---") {
            flush_doc!();
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
            cur_m_prefab_instance_id = None;
            awaiting_name_value = false;
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

        if let Some(f) = extract_field_name(trimmed) {
            // m_Name（GameObject classID=1）
            if f == "m_Name" && cur_class_id == Some(1) {
                if let Some(val) = extract_plain_value(trimmed, "m_Name:") {
                    cur_m_name = Some(val);
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
            last_field = Some(f);
        }

        if trimmed.contains('{') {
            let balance = count_braces(trimmed);
            if balance > 0 {
                pending_line = Some(trimmed.to_string());
                pending_braces = balance;
                continue;
            }
            process_flow_line(
                trimmed,
                cur_class_id,
                cur_is_stripped,
                &last_field,
                cur_file_id,
                &mut cur_m_go_id,
                &mut cur_m_father,
                &mut cur_m_prefab_instance_id,
                &mut raw_refs,
                &mut seen,
            );
        }
    }

    flush_doc!();
    (docs, raw_refs)
}

fn process_flow_line(
    line: &str,
    class_id: Option<i32>,
    is_stripped: bool,
    last_field: &Option<String>,
    doc_file_id: i64,
    m_go_id: &mut Option<i64>,
    m_father: &mut Option<i64>,
    m_prefab_instance_id: &mut Option<i64>,
    raw_refs: &mut Vec<RawRef>,
    seen: &mut HashSet<([u8; 16], Option<i64>, Option<String>, i64)>,
) {
    if let Some(field) = last_field {
        if (field == "m_GameObject" || field == "m_Father" || field == "m_PrefabInstance")
            && line.contains("fileID:")
            && !line.contains("guid:")
        {
            if let Some(fid) = extract_internal_file_id(line) {
                if fid != 0 {
                    match field.as_str() {
                        "m_GameObject" => *m_go_id = Some(fid),
                        "m_Father" => *m_father = Some(fid),
                        "m_PrefabInstance" if is_stripped => *m_prefab_instance_id = Some(fid),
                        _ => {}
                    }
                }
            }
        }
    }

    if line.contains("guid:") {
        extract_flow_maps_raw(line, class_id, last_field, doc_file_id, raw_refs, seen);
    }
}

fn phase2_build_paths(
    docs: &[DocMeta],
    raw_refs: Vec<RawRef>,
    guid_to_path: Option<&HashMap<Guid, String>>,
) -> Vec<ExtractedRef> {
    let doc_map: HashMap<i64, &DocMeta> = docs.iter().map(|d| (d.file_id, d)).collect();

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
        .filter_map(|d| d.m_prefab_instance_id.map(|pi| (d.file_id, pi)))
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
    raw: &RawRef,
    doc_map: &HashMap<i64, &DocMeta>,
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

fn extract_flow_maps_raw(
    line: &str,
    class_id: Option<i32>,
    last_field: &Option<String>,
    doc_file_id: i64,
    refs: &mut Vec<RawRef>,
    seen: &mut HashSet<([u8; 16], Option<i64>, Option<String>, i64)>,
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
) -> Option<RawRef> {
    let guid_str = extract_value(block, "guid:")?;
    let guid_str = guid_str.trim().trim_end_matches(',');
    let dst_guid = parse_guid_hex(guid_str)?;

    if dst_guid == [0u8; 16] {
        return None;
    }

    let dst_file_id = extract_value(block, "fileID:")
        .and_then(|v| v.trim().trim_end_matches(',').parse::<i64>().ok());

    Some(RawRef {
        dst_guid,
        dst_file_id,
        class_id_hint: class_id,
        field_hint: last_field.clone(),
        src_doc_file_id: doc_file_id,
    })
}
