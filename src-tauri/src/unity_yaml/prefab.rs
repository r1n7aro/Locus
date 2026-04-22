use crate::asset_db::types::{
    parse_guid_hex, PrefabInstanceIR, PrefabSourceRef, PropertyOverride, RemovedComponent,
    StrippedMapping,
};

use super::parser::YamlDoc;
use super::tokenizer::{
    count_braces, extract_field_name, extract_internal_file_id, extract_plain_value, extract_value,
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

#[allow(unused_assignments)]
fn parse_single_prefab_instance_ir(doc: &YamlDoc, lines: &[&str]) -> Option<PrefabInstanceIR> {
    if doc.class_id != 1001 {
        return None;
    }

    let source_guid = doc.source_prefab_guid?;

    let source_file_id = extract_source_prefab_file_id(lines, doc.line_start, doc.line_end);

    let mut property_overrides = Vec::new();
    let mut removed_components = Vec::new();
    let mut instance_name = doc.m_name.clone();

    let mut in_modifications = false;
    let mut in_removed = false;
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

    for i in doc.line_start..doc.line_end.min(lines.len()) {
        let line = lines[i];
        let trimmed = line.trim();

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

        if trimmed == "m_Modifications:" {
            in_modifications = true;
            in_removed = false;
            in_modification_entry = false;
            continue;
        }
        if trimmed == "m_RemovedComponents:" {
            if in_modification_entry {
                flush_modification!();
                in_modification_entry = false;
            }
            in_modifications = false;
            in_removed = true;
            continue;
        }

        if indent <= 4
            && !trimmed.starts_with('-')
            && trimmed.contains(':')
            && !trimmed.starts_with("m_Modifications")
            && !trimmed.starts_with("m_RemovedComponents")
        {
            if in_modifications || in_removed {
                if in_modification_entry {
                    flush_modification!();
                    in_modification_entry = false;
                }
                in_modifications = false;
                in_removed = false;
            }
        }

        if in_modifications {
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
                            cur_value = extract_plain_value(trimmed, "value:");
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

        if in_removed {
            if trimmed.starts_with("- ") && trimmed.contains('{') {
                if let Some(src) = parse_source_ref_from_flow(trimmed) {
                    removed_components.push(RemovedComponent { target: src });
                }
            }
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
        line_start: doc.line_start,
        line_end: doc.line_end,
    })
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
