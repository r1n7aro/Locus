use crate::asset_db::types::{
    parse_guid_hex, PrefabInstanceIR, PrefabSourceRef, PropertyOverride, RemovedComponent,
    StrippedMapping,
};

use super::parser::YamlDoc;
use super::tokenizer::{
    closes_single_quoted, count_braces, extract_field_name, extract_internal_file_id,
    extract_plain_value, extract_value, is_unclosed_single_quoted, trimmed_value_after,
};

pub fn extract_prefab_instance_irs(docs: &[YamlDoc], lines: &[&str]) -> Vec<PrefabInstanceIR> {
    use rayon::prelude::*;
    // par_iter preserves input order per rayon's ParallelIterator::collect guarantee.
    docs.par_iter()
        .filter(|d| d.class_id == 1001 && !d.is_stripped)
        .filter_map(|doc| parse_single_prefab_instance_ir(doc, lines))
        .collect()
}

pub fn extract_stripped_mappings(docs: &[YamlDoc], lines: &[&str]) -> Vec<StrippedMapping> {
    docs.iter()
        .filter(|d| d.is_stripped)
        .filter_map(|doc| {
            let pi_id = doc.prefab_instance_id?;
            let source = extract_corresponding_source(lines, doc.line_start, doc.line_end)?;
            Some(StrippedMapping {
                local_file_id: doc.file_id,
                class_id: doc.class_id,
                type_name: doc.type_name.clone(),
                source,
                prefab_instance_id: pi_id,
            })
        })
        .collect()
}

#[derive(PartialEq, Clone, Copy)]
enum PrefabModSection {
    None,
    Modifications,
    RemovedComponents,
    RemovedGameObjects,
    AddedGameObjects,
    AddedComponents,
}

#[allow(unused_assignments)]
fn parse_single_prefab_instance_ir(doc: &YamlDoc, lines: &[&str]) -> Option<PrefabInstanceIR> {
    if doc.class_id != 1001 {
        return None;
    }

    let source_guid = doc.source_prefab_guid?;

    let source_file_id = extract_source_prefab_file_id(lines, doc.line_start, doc.line_end);

    let mut property_overrides = Vec::new();
    let mut removed_components = Vec::new();
    let mut removed_game_objects = Vec::new();
    let mut added_game_object_count = 0usize;
    let mut added_component_count = 0usize;
    let mut instance_name = doc.m_name.clone();

    let mut section = PrefabModSection::None;
    let mut in_modification_entry = false;

    let mut cur_target: Option<PrefabSourceRef> = None;
    let mut cur_property_path: Option<String> = None;
    let mut cur_value: Option<String> = None;
    let mut cur_object_ref: Option<PrefabSourceRef> = None;

    let mut pending_line: Option<String> = None;
    let mut pending_braces: i32 = 0;

    macro_rules! flush_modification {
        () => {
            if let Some(target) = cur_target.take() {
                if let Some(ref pp) = cur_property_path {
                    if pp == "m_Name" {
                        if let Some(ref v) = cur_value {
                            if instance_name.is_none() && !v.is_empty() {
                                instance_name = Some(v.clone());
                            }
                        }
                    }
                    property_overrides.push(PropertyOverride {
                        target,
                        property_path: cur_property_path.take().unwrap(),
                        value: cur_value.take(),
                        object_ref: cur_object_ref.take(),
                    });
                } else {
                    cur_property_path = None;
                    cur_value = None;
                    cur_object_ref = None;
                }
            } else {
                cur_property_path = None;
                cur_value = None;
                cur_object_ref = None;
            }
        };
    }

    let end_line = doc.line_end.min(lines.len());
    let mut i = doc.line_start;
    while i < end_line {
        let line = lines[i];
        let trimmed = line.trim();
        i += 1;

        if let Some(ref mut buf) = pending_line {
            buf.push(' ');
            buf.push_str(trimmed);
            pending_braces += count_braces(trimmed);
            if pending_braces <= 0 {
                let complete = pending_line.take().unwrap();
                process_modification_flow_line(&complete, &mut cur_target, &mut cur_object_ref);
                pending_braces = 0;
            }
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        let section_header = match trimmed {
            "m_Modifications:" => Some(PrefabModSection::Modifications),
            "m_RemovedComponents:" => Some(PrefabModSection::RemovedComponents),
            "m_RemovedGameObjects:" => Some(PrefabModSection::RemovedGameObjects),
            "m_AddedGameObjects:" => Some(PrefabModSection::AddedGameObjects),
            "m_AddedComponents:" => Some(PrefabModSection::AddedComponents),
            _ => None,
        };
        if let Some(next_section) = section_header {
            if in_modification_entry {
                flush_modification!();
                in_modification_entry = false;
            }
            section = next_section;
            continue;
        }

        if indent <= 4
            && !trimmed.starts_with('-')
            && trimmed.contains(':')
            && section != PrefabModSection::None
        {
            if in_modification_entry {
                flush_modification!();
                in_modification_entry = false;
            }
            section = PrefabModSection::None;
        }

        match section {
            PrefabModSection::Modifications => {
                if trimmed.starts_with("- target:") {
                    if in_modification_entry {
                        flush_modification!();
                    }
                    in_modification_entry = true;
                    cur_target = None;
                    cur_property_path = None;
                    cur_value = None;
                    cur_object_ref = None;

                    if trimmed.contains('{') {
                        let balance = count_braces(trimmed);
                        if balance > 0 {
                            pending_line = Some(trimmed.to_string());
                            pending_braces = balance;
                        } else {
                            cur_target = parse_source_ref_from_flow(trimmed);
                        }
                    }
                    continue;
                }

                if in_modification_entry {
                    // propertyPath / value / objectReference
                    if let Some(f) = extract_field_name(trimmed) {
                        match f.as_str() {
                            "propertyPath" => {
                                cur_property_path = extract_plain_value(trimmed, "propertyPath:");
                            }
                            "value" => {
                                let raw = trimmed_value_after(trimmed, "value:");
                                if is_unclosed_single_quoted(raw) {
                                    // Multi-line single-quoted scalar (text
                                    // overrides commonly span lines); join
                                    // the continuation lines instead of
                                    // storing a half scalar.
                                    let (joined, consumed) =
                                        take_multiline_single_quoted(raw, lines, i, end_line);
                                    cur_value = Some(joined);
                                    i += consumed;
                                } else {
                                    cur_value = extract_plain_value(trimmed, "value:");
                                }
                            }
                            "objectReference" => {
                                if trimmed.contains('{') {
                                    let balance = count_braces(trimmed);
                                    if balance > 0 {
                                        pending_line = Some(trimmed.to_string());
                                        pending_braces = balance;
                                    } else {
                                        cur_object_ref = parse_source_ref_from_flow(trimmed);
                                    }
                                }
                            }
                            "target" => {
                                // continuation target (shouldn't happen normally, but handle)
                                if trimmed.contains('{') {
                                    let balance = count_braces(trimmed);
                                    if balance > 0 {
                                        pending_line = Some(trimmed.to_string());
                                        pending_braces = balance;
                                    } else {
                                        cur_target = parse_source_ref_from_flow(trimmed);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            PrefabModSection::RemovedComponents => {
                if trimmed.starts_with("- ") && trimmed.contains('{') {
                    if let Some(src) = parse_source_ref_from_flow(trimmed) {
                        removed_components.push(RemovedComponent { target: src });
                    }
                }
            }
            PrefabModSection::RemovedGameObjects => {
                if trimmed.starts_with("- ") && trimmed.contains('{') {
                    if let Some(src) = parse_source_ref_from_flow(trimmed) {
                        removed_game_objects.push(RemovedComponent { target: src });
                    }
                }
            }
            PrefabModSection::AddedGameObjects => {
                // Entries are small maps (`- targetCorrespondingSourceObject:
                // ...`); count entry heads only.
                if trimmed.starts_with("- ") {
                    added_game_object_count += 1;
                }
            }
            PrefabModSection::AddedComponents => {
                if trimmed.starts_with("- ") {
                    added_component_count += 1;
                }
            }
            PrefabModSection::None => {}
        }
    }

    if in_modification_entry {
        flush_modification!();
    }

    Some(PrefabInstanceIR {
        local_file_id: doc.file_id,
        source_prefab_guid: source_guid,
        source_prefab_file_id: source_file_id,
        transform_parent: doc.transform_parent_id,
        instance_name,
        property_overrides,
        removed_components,
        removed_game_objects,
        added_game_object_count,
        added_component_count,
        line_start: doc.line_start,
        line_end: doc.line_end,
    })
}

/// Join a multi-line single-quoted scalar starting at `first_raw` (the `'…`
/// remainder of the `value:` line, missing its closing quote). Returns the
/// unescaped joined text and how many continuation lines were consumed.
fn take_multiline_single_quoted(
    first_raw: &str,
    lines: &[&str],
    next: usize,
    end: usize,
) -> (String, usize) {
    let mut out = first_raw[1..].replace("''", "'");
    let end = end.min(lines.len());
    let mut j = next;
    while j < end && j - next < 200 {
        let cont = lines[j].trim_start();
        if closes_single_quoted(cont) {
            // Content runs up to the first unescaped quote.
            let bytes = cont.as_bytes();
            let mut cut = cont.len();
            let mut k = 0;
            while k < bytes.len() {
                if bytes[k] == b'\'' {
                    if bytes.get(k + 1) == Some(&b'\'') {
                        k += 2;
                        continue;
                    }
                    cut = k;
                    break;
                }
                k += 1;
            }
            out.push('\n');
            out.push_str(&cont[..cut].replace("''", "'"));
            return (out, j + 1 - next);
        }
        out.push('\n');
        out.push_str(&cont.replace("''", "'"));
        j += 1;
    }
    (out, j - next)
}

pub(super) fn parse_source_ref_from_flow(line: &str) -> Option<PrefabSourceRef> {
    let guid_str = extract_value(line, "guid:")?;
    let guid = parse_guid_hex(guid_str.trim().trim_end_matches(','))?;
    if guid == [0u8; 16] {
        return None;
    }
    let file_id = extract_value(line, "fileID:")
        .and_then(|v| v.trim().trim_end_matches(',').parse::<i64>().ok())
        .unwrap_or(0);
    let type_id = extract_value(line, "type:")
        .and_then(|v| {
            v.trim()
                .trim_end_matches(',')
                .trim_end_matches('}')
                .parse::<i32>()
                .ok()
        })
        .unwrap_or(0);
    Some(PrefabSourceRef {
        guid,
        source_file_id: file_id,
        type_id,
    })
}

fn process_modification_flow_line(
    complete_line: &str,
    cur_target: &mut Option<PrefabSourceRef>,
    cur_object_ref: &mut Option<PrefabSourceRef>,
) {
    let trimmed = complete_line.trim();
    if trimmed.contains("target:") || (cur_target.is_none() && trimmed.contains("guid:")) {
        *cur_target = parse_source_ref_from_flow(trimmed);
    } else if trimmed.contains("objectReference:") {
        *cur_object_ref = parse_source_ref_from_flow(trimmed);
    }
}

fn extract_corresponding_source(
    lines: &[&str],
    start: usize,
    end: usize,
) -> Option<PrefabSourceRef> {
    let mut pending_line: Option<String> = None;
    let mut pending_braces: i32 = 0;
    let mut found_field = false;

    for i in start..end.min(lines.len()) {
        let trimmed = lines[i].trim();

        if let Some(ref mut buf) = pending_line {
            buf.push(' ');
            buf.push_str(trimmed);
            pending_braces += count_braces(trimmed);
            if pending_braces <= 0 {
                let complete = pending_line.take().unwrap();
                return parse_source_ref_from_flow(&complete);
            }
            continue;
        }

        if trimmed.contains("m_CorrespondingSourceObject:") {
            found_field = true;
            if trimmed.contains('{') {
                let balance = count_braces(trimmed);
                if balance > 0 {
                    pending_line = Some(trimmed.to_string());
                    pending_braces = balance;
                } else {
                    return parse_source_ref_from_flow(trimmed);
                }
            }
        } else if found_field {
            break;
        }
    }
    None
}

fn extract_source_prefab_file_id(lines: &[&str], start: usize, end: usize) -> i64 {
    for i in start..end.min(lines.len()) {
        let trimmed = lines[i].trim();
        if trimmed.contains("m_SourcePrefab:") && trimmed.contains("fileID:") {
            if let Some(fid) = extract_internal_file_id(trimmed) {
                return fid;
            }
        }
    }
    0
}
