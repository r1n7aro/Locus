use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{
    guid_to_hex, PrefabInstanceIR, PrefabSourceRef, PropertyOverride, RemovedComponent,
};
use crate::diff::semantic::parse::{leading_spaces, split_key_value};
use crate::diff::types::SemanticLayout;
use crate::error::{AppError, AppResult};
use crate::unity_yaml::{extract_prefab_instance_irs, YamlDoc};

use super::three_way::match_asset_docs_three_way;
use super::types::*;

#[derive(Debug, Clone)]
struct RawFieldEntry {
    line_index: usize,
    prefix: String,
    raw_value: String,
    patchable: bool,
}

#[derive(Debug, Clone)]
struct DocPlan<'a> {
    doc_key: String,
    base_doc: Option<&'a YamlDoc>,
    ours_doc: Option<&'a YamlDoc>,
    theirs_doc: Option<&'a YamlDoc>,
    sort_group: usize,
    sort_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AssembledMerge {
    ResolvedText(String),
    DeleteFile,
}

fn doc_slice_bounds(doc: &YamlDoc, line_count: usize) -> Option<(usize, usize)> {
    let start = doc.line_start.min(line_count);
    let end = doc.line_end.min(line_count);
    if start >= end {
        None
    } else {
        Some((start, end))
    }
}

fn extract_doc_text(lines: &[String], doc: &YamlDoc) -> String {
    doc_slice_bounds(doc, lines.len())
        .map(|(start, end)| lines[start..end].join("\n"))
        .unwrap_or_default()
}

fn first_doc_start(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|line| line.starts_with("---"))
        .unwrap_or(0)
}

fn parse_raw_field_entries(doc: &YamlDoc, lines: &[String]) -> HashMap<String, RawFieldEntry> {
    let Some((start, end)) = doc_slice_bounds(doc, lines.len()) else {
        return HashMap::new();
    };

    let mut fields = HashMap::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut list_counters: HashMap<String, usize> = HashMap::new();

    for (relative_idx, raw_line) in lines[start..end].iter().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('%') {
            continue;
        }
        if relative_idx == 0 && trimmed.starts_with("---") {
            continue;
        }
        if relative_idx == 1 && !raw_line.starts_with(' ') && trimmed.ends_with(':') {
            continue;
        }

        let indent = leading_spaces(raw_line);
        let is_list_item = trimmed.starts_with("- ");

        while let Some((last_indent, _)) = stack.last() {
            if is_list_item {
                let should_pop = if *last_indent > indent {
                    true
                } else if *last_indent == indent {
                    stack
                        .last()
                        .map(|(_, path)| path.ends_with(']'))
                        .unwrap_or(false)
                } else {
                    false
                };
                if should_pop {
                    stack.pop();
                } else {
                    break;
                }
            } else if *last_indent >= indent {
                stack.pop();
            } else {
                break;
            }
        }

        if let Some(rest) = trimmed.strip_prefix("- ") {
            let Some((_, parent_path)) = stack.last() else {
                continue;
            };
            let index = list_counters.entry(parent_path.clone()).or_insert(0usize);
            let item_path = format!("{}[{}]", parent_path, *index);
            *index += 1;

            let rest = rest.trim();
            if rest.is_empty() {
                stack.push((indent, item_path));
                continue;
            }

            if !rest.starts_with('{') {
                if let Some((key, value)) = split_key_value(rest) {
                    let child_path = format!("{}.{}", item_path, key);
                    if let Some(colon_pos) = raw_line.find(':') {
                        fields.insert(
                            child_path.clone(),
                            RawFieldEntry {
                                line_index: relative_idx,
                                prefix: raw_line[..=colon_pos].to_string(),
                                raw_value: raw_line[colon_pos + 1..].trim_start().to_string(),
                                patchable: !value.is_empty(),
                            },
                        );
                    }
                    stack.push((indent, item_path));
                    if value.is_empty() {
                        stack.push((indent + 1, child_path));
                    }
                    continue;
                }
            }

            fields.insert(
                item_path,
                RawFieldEntry {
                    line_index: relative_idx,
                    prefix: format!("{}-", " ".repeat(indent)),
                    raw_value: rest.to_string(),
                    patchable: false,
                },
            );
            continue;
        }

        if let Some((key, value)) = split_key_value(trimmed) {
            let path = match stack.last() {
                Some((_, parent)) => format!("{}.{}", parent, key),
                None => key.clone(),
            };
            if let Some(colon_pos) = raw_line.find(':') {
                fields.insert(
                    path.clone(),
                    RawFieldEntry {
                        line_index: relative_idx,
                        prefix: raw_line[..=colon_pos].to_string(),
                        raw_value: raw_line[colon_pos + 1..].trim_start().to_string(),
                        patchable: !value.is_empty(),
                    },
                );
            }
            if value.is_empty() {
                stack.push((indent, path));
            }
        }
    }

    fields
}

fn collect_field_choices(
    fields: &[MergeField],
    resolutions: &HashMap<String, FieldResolution>,
    doc_choices: &mut HashMap<String, HashMap<String, MergeSide>>,
) -> AppResult<()> {
    for field in fields {
        if field.children.is_empty() {
            let Some((raw_doc_key, _)) = field.id.split_once('|') else {
                return Err(AppError::new(
                    "merge.invalid_field_id",
                    format!("Invalid merge field id '{}'", field.id),
                ));
            };
            let doc_key = raw_doc_key;
            let chosen = resolutions
                .get(&field.id)
                .map(|resolution| resolution.side)
                .or(field.auto_choice)
                .or_else(|| {
                    if field.merge_state == MergeState::Unchanged {
                        Some(MergeSide::Base)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    AppError::new(
                        "merge.unresolved_field",
                        format!("Field '{}' is still unresolved", field.id),
                    )
                })?;

            doc_choices
                .entry(doc_key.to_string())
                .or_default()
                .insert(field.property_path.clone(), chosen);
        } else {
            collect_field_choices(&field.children, resolutions, doc_choices)?;
        }
    }
    Ok(())
}

fn doc_choice_groups_for_plan(
    doc_key: &str,
    all_choices: &HashMap<String, HashMap<String, MergeSide>>,
) -> HashMap<String, HashMap<String, MergeSide>> {
    let nested_prefix = format!("{doc_key}:");
    all_choices
        .iter()
        .filter(|(raw_key, _)| raw_key.as_str() == doc_key || raw_key.starts_with(&nested_prefix))
        .map(|(raw_key, choices)| (raw_key.clone(), choices.clone()))
        .collect()
}

fn collect_unique_sides(
    choice_groups: &HashMap<String, HashMap<String, MergeSide>>,
) -> Vec<MergeSide> {
    let mut unique = Vec::new();
    for side in choice_groups
        .values()
        .flat_map(|choices| choices.values().copied())
    {
        if !unique.contains(&side) {
            unique.push(side);
        }
    }
    unique
}

fn extract_single_prefab_instance_ir(
    doc: &YamlDoc,
    lines: &[String],
) -> AppResult<PrefabInstanceIR> {
    let line_refs = lines.iter().map(|line| line.as_str()).collect::<Vec<_>>();
    extract_prefab_instance_irs(std::slice::from_ref(doc), &line_refs)
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new(
                "merge.invalid_prefab_doc",
                format!(
                    "PrefabInstance doc '{}' could not be converted into an override IR",
                    doc.file_id
                ),
            )
        })
}

fn prefab_override_key(entry: &PropertyOverride) -> (i64, String) {
    (entry.target.source_file_id, entry.property_path.clone())
}

fn removed_component_key(entry: &RemovedComponent) -> i64 {
    entry.target.source_file_id
}

fn build_prefab_override_map(
    ir: Option<&PrefabInstanceIR>,
) -> HashMap<(i64, String), PropertyOverride> {
    ir.map(|doc| {
        doc.property_overrides
            .iter()
            .cloned()
            .map(|entry| (prefab_override_key(&entry), entry))
            .collect()
    })
    .unwrap_or_default()
}

fn build_removed_component_map(ir: Option<&PrefabInstanceIR>) -> HashMap<i64, RemovedComponent> {
    ir.map(|doc| {
        doc.removed_components
            .iter()
            .cloned()
            .map(|entry| (removed_component_key(&entry), entry))
            .collect()
    })
    .unwrap_or_default()
}

fn ordered_prefab_override_keys(
    base_ir: &PrefabInstanceIR,
    ours_ir: Option<&PrefabInstanceIR>,
    theirs_ir: Option<&PrefabInstanceIR>,
) -> Vec<(i64, String)> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    for ir in [Some(base_ir), ours_ir, theirs_ir].into_iter().flatten() {
        for entry in &ir.property_overrides {
            let key = prefab_override_key(entry);
            if seen.insert(key.clone()) {
                ordered.push(key);
            }
        }
    }
    ordered
}

fn ordered_removed_component_ids(
    base_ir: &PrefabInstanceIR,
    ours_ir: Option<&PrefabInstanceIR>,
    theirs_ir: Option<&PrefabInstanceIR>,
) -> Vec<i64> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    for ir in [Some(base_ir), ours_ir, theirs_ir].into_iter().flatten() {
        for entry in &ir.removed_components {
            let key = removed_component_key(entry);
            if seen.insert(key) {
                ordered.push(key);
            }
        }
    }
    ordered
}

fn format_prefab_source_ref(source: Option<&PrefabSourceRef>) -> String {
    match source {
        Some(source) => format!(
            "{{fileID: {}, guid: {}, type: {}}}",
            source.source_file_id,
            guid_to_hex(&source.guid),
            source.type_id
        ),
        None => "{fileID: 0}".to_string(),
    }
}

fn serialize_prefab_override(entry: &PropertyOverride, indent: usize) -> Vec<String> {
    let item_indent = " ".repeat(indent);
    let child_indent = " ".repeat(indent + 2);
    vec![
        format!(
            "{}- target: {}",
            item_indent,
            format_prefab_source_ref(Some(&entry.target))
        ),
        format!("{}propertyPath: {}", child_indent, entry.property_path),
        match entry.value.as_deref() {
            Some(value) if !value.is_empty() => format!("{}value: {}", child_indent, value),
            _ => format!("{}value:", child_indent),
        },
        format!(
            "{}objectReference: {}",
            child_indent,
            format_prefab_source_ref(entry.object_ref.as_ref())
        ),
    ]
}

fn serialize_removed_component(entry: &RemovedComponent, indent: usize) -> String {
    format!(
        "{}- {}",
        " ".repeat(indent),
        format_prefab_source_ref(Some(&entry.target))
    )
}

fn try_find_prefab_list_block(
    doc_lines: &[String],
    field_name: &str,
) -> Option<(usize, usize, usize)> {
    let mut start = None;
    let mut indent = 0usize;
    let field_prefix = format!("{field_name}:");

    for (idx, line) in doc_lines.iter().enumerate() {
        let trimmed = line.trim();
        if start.is_none() {
            if trimmed.starts_with(&field_prefix) {
                start = Some(idx);
                indent = leading_spaces(line);
            }
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let line_indent = leading_spaces(line);
        if line_indent <= indent && !trimmed.starts_with('-') {
            return Some((start.unwrap_or(0), idx, indent));
        }
    }

    start.map(|block_start| (block_start, doc_lines.len(), indent))
}

fn find_prefab_list_block(
    doc_lines: &[String],
    field_name: &str,
) -> AppResult<(usize, usize, usize)> {
    try_find_prefab_list_block(doc_lines, field_name).ok_or_else(|| {
        AppError::new(
            "merge.invalid_prefab_doc",
            format!("PrefabInstance doc is missing the {field_name} block"),
        )
    })
}

enum PrefabChoiceGroup {
    Overrides(i64),
    RemovedComponents,
}

fn parse_prefab_choice_group(
    plan_doc_key: &str,
    raw_doc_key: &str,
) -> AppResult<PrefabChoiceGroup> {
    let suffix = raw_doc_key
        .strip_prefix(plan_doc_key)
        .and_then(|rest| rest.strip_prefix(':'))
        .ok_or_else(|| {
            AppError::new(
                "merge.unsupported_structural_merge",
                format!(
                    "Prefab override group '{}' cannot be applied semantically",
                    raw_doc_key
                ),
            )
        })?;
    if suffix == "removed" {
        return Ok(PrefabChoiceGroup::RemovedComponents);
    }
    suffix
        .parse::<i64>()
        .map(PrefabChoiceGroup::Overrides)
        .map_err(|_| {
            AppError::new(
                "merge.unsupported_structural_merge",
                format!(
                    "Prefab override group '{}' is missing a stable source file id",
                    raw_doc_key
                ),
            )
        })
}

fn parse_removed_component_property_path(property_path: &str) -> AppResult<i64> {
    property_path
        .strip_prefix("removedComponent:")
        .ok_or_else(|| {
            AppError::new(
                "merge.unsupported_structural_merge",
                format!("Unsupported removed component field '{}'", property_path),
            )
        })?
        .parse::<i64>()
        .map_err(|_| {
            AppError::new(
                "merge.unsupported_structural_merge",
                format!(
                    "Removed component field '{}' is missing a stable source file id",
                    property_path
                ),
            )
        })
}

fn patch_prefab_instance_doc(
    plan_doc_key: &str,
    base_doc: &YamlDoc,
    base_lines: &[String],
    ours_doc: Option<&YamlDoc>,
    ours_lines: &[String],
    theirs_doc: Option<&YamlDoc>,
    theirs_lines: &[String],
    choice_groups: &HashMap<String, HashMap<String, MergeSide>>,
) -> AppResult<String> {
    let Some((start, end)) = doc_slice_bounds(base_doc, base_lines.len()) else {
        return Err(AppError::new(
            "merge.invalid_doc_bounds",
            "Invalid prefab base document bounds",
        ));
    };

    let base_ir = extract_single_prefab_instance_ir(base_doc, base_lines)?;
    let ours_ir = ours_doc
        .map(|doc| extract_single_prefab_instance_ir(doc, ours_lines))
        .transpose()?;
    let theirs_ir = theirs_doc
        .map(|doc| extract_single_prefab_instance_ir(doc, theirs_lines))
        .transpose()?;

    let base_map = build_prefab_override_map(Some(&base_ir));
    let ours_map = build_prefab_override_map(ours_ir.as_ref());
    let theirs_map = build_prefab_override_map(theirs_ir.as_ref());
    let mut resolved_map = base_map.clone();
    let base_removed_map = build_removed_component_map(Some(&base_ir));
    let ours_removed_map = build_removed_component_map(ours_ir.as_ref());
    let theirs_removed_map = build_removed_component_map(theirs_ir.as_ref());
    let mut resolved_removed_map = base_removed_map.clone();

    for (raw_doc_key, choices) in choice_groups {
        if raw_doc_key == plan_doc_key {
            return Err(AppError::new(
                "merge.unsupported_structural_merge",
                format!(
                    "PrefabInstance doc '{}' contains non-override semantic fields",
                    plan_doc_key
                ),
            ));
        }

        match parse_prefab_choice_group(plan_doc_key, raw_doc_key)? {
            PrefabChoiceGroup::Overrides(target_file_id) => {
                for (property_path, side) in choices {
                    let key = (target_file_id, property_path.clone());
                    let replacement = match side {
                        MergeSide::Base => base_map.get(&key).cloned(),
                        MergeSide::Ours => ours_map.get(&key).cloned(),
                        MergeSide::Theirs => theirs_map.get(&key).cloned(),
                    };

                    match replacement {
                        Some(entry) => {
                            resolved_map.insert(key, entry);
                        }
                        None => {
                            resolved_map.remove(&key);
                        }
                    }
                }
            }
            PrefabChoiceGroup::RemovedComponents => {
                for (property_path, side) in choices {
                    let key = parse_removed_component_property_path(property_path)?;
                    let replacement = match side {
                        MergeSide::Base => base_removed_map.get(&key).cloned(),
                        MergeSide::Ours => ours_removed_map.get(&key).cloned(),
                        MergeSide::Theirs => theirs_removed_map.get(&key).cloned(),
                    };

                    match replacement {
                        Some(entry) => {
                            resolved_removed_map.insert(key, entry);
                        }
                        None => {
                            resolved_removed_map.remove(&key);
                        }
                    }
                }
            }
        }
    }

    let ordered_keys = ordered_prefab_override_keys(&base_ir, ours_ir.as_ref(), theirs_ir.as_ref());
    let mut resolved_overrides = Vec::new();
    let mut emitted = HashSet::new();
    for key in ordered_keys {
        if let Some(entry) = resolved_map.get(&key) {
            resolved_overrides.push(entry.clone());
            emitted.insert(key);
        }
    }
    let mut remaining_override_keys = resolved_map
        .keys()
        .filter(|key| !emitted.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    remaining_override_keys.sort();
    for key in remaining_override_keys {
        if let Some(entry) = resolved_map.get(&key) {
            resolved_overrides.push(entry.clone());
            emitted.insert(key);
        }
    }

    let ordered_removed_ids =
        ordered_removed_component_ids(&base_ir, ours_ir.as_ref(), theirs_ir.as_ref());
    let mut resolved_removed_components = Vec::new();
    let mut emitted_removed = HashSet::new();
    for key in ordered_removed_ids {
        if let Some(entry) = resolved_removed_map.get(&key) {
            resolved_removed_components.push(entry.clone());
            emitted_removed.insert(key);
        }
    }
    let mut remaining_removed_ids = resolved_removed_map
        .keys()
        .filter(|key| !emitted_removed.contains(*key))
        .copied()
        .collect::<Vec<_>>();
    remaining_removed_ids.sort_unstable();
    for key in remaining_removed_ids {
        if let Some(entry) = resolved_removed_map.get(&key) {
            resolved_removed_components.push(entry.clone());
            emitted_removed.insert(key);
        }
    }

    let mut doc_lines = base_lines[start..end].to_vec();
    let (mods_start, mods_end, mods_indent) =
        find_prefab_list_block(&doc_lines, "m_Modifications")?;
    let removed_block = try_find_prefab_list_block(&doc_lines, "m_RemovedComponents");
    let removed_indent = removed_block
        .map(|(_, _, indent)| indent)
        .unwrap_or(mods_indent);
    let mods_replacement = if resolved_overrides.is_empty() {
        vec![format!("{}m_Modifications: []", " ".repeat(mods_indent))]
    } else {
        let mut lines = vec![format!("{}m_Modifications:", " ".repeat(mods_indent))];
        for entry in &resolved_overrides {
            lines.extend(serialize_prefab_override(entry, mods_indent));
        }
        lines
    };
    let removed_replacement = if resolved_removed_components.is_empty() {
        vec![format!(
            "{}m_RemovedComponents: []",
            " ".repeat(removed_indent)
        )]
    } else {
        let mut lines = vec![format!(
            "{}m_RemovedComponents:",
            " ".repeat(removed_indent)
        )];
        for entry in &resolved_removed_components {
            lines.push(serialize_removed_component(entry, removed_indent));
        }
        lines
    };

    let mut replacements = vec![(mods_start, mods_end, mods_replacement)];
    if let Some((removed_start, removed_end, _)) = removed_block {
        replacements.push((removed_start, removed_end, removed_replacement));
    } else if !resolved_removed_components.is_empty() {
        replacements.push((mods_end, mods_end, removed_replacement));
    }
    replacements.sort_by(|a, b| b.0.cmp(&a.0));
    for (start, end, replacement) in replacements {
        doc_lines.splice(start..end, replacement);
    }
    Ok(doc_lines.join("\n"))
}

fn build_doc_plans<'a>(session: &'a MergeSemanticSession) -> Vec<DocPlan<'a>> {
    match session.layout {
        SemanticLayout::SceneHierarchyInspector => {
            let mut file_ids: Vec<i64> = session
                .base_docs
                .iter()
                .chain(session.ours_docs.iter())
                .chain(session.theirs_docs.iter())
                .map(|doc| doc.file_id)
                .collect();
            file_ids.sort_unstable();
            file_ids.dedup();

            let base_by_id: HashMap<i64, &YamlDoc> = session
                .base_docs
                .iter()
                .map(|doc| (doc.file_id, doc))
                .collect();
            let ours_by_id: HashMap<i64, &YamlDoc> = session
                .ours_docs
                .iter()
                .map(|doc| (doc.file_id, doc))
                .collect();
            let theirs_by_id: HashMap<i64, &YamlDoc> = session
                .theirs_docs
                .iter()
                .map(|doc| (doc.file_id, doc))
                .collect();

            let mut plans = Vec::new();
            for file_id in file_ids {
                let base_doc = base_by_id.get(&file_id).copied();
                let ours_doc = ours_by_id.get(&file_id).copied();
                let theirs_doc = theirs_by_id.get(&file_id).copied();
                let sort_group = if base_doc.is_some() { 0 } else { 1 };
                let sort_order = base_doc
                    .map(|doc| doc.line_start)
                    .or_else(|| ours_doc.map(|doc| doc.line_start))
                    .or_else(|| theirs_doc.map(|doc| doc.line_start))
                    .unwrap_or(usize::MAX);
                plans.push(DocPlan {
                    doc_key: format!("sceneDoc:{}", file_id),
                    base_doc,
                    ours_doc,
                    theirs_doc,
                    sort_group,
                    sort_order,
                });
            }
            plans.sort_by_key(|plan| (plan.sort_group, plan.sort_order));
            plans
        }
        _ => {
            let mut plans = Vec::new();
            for matched_doc in match_asset_docs_three_way(
                &session.base_docs,
                &session.ours_docs,
                &session.theirs_docs,
            ) {
                plans.push(DocPlan {
                    doc_key: format!("assetDoc:{}", matched_doc.key),
                    base_doc: matched_doc.base_doc,
                    ours_doc: matched_doc.ours_doc,
                    theirs_doc: matched_doc.theirs_doc,
                    sort_group: matched_doc.sort_group,
                    sort_order: matched_doc.sort_order,
                });
            }
            plans.sort_by_key(|plan| (plan.sort_group, plan.sort_order));
            plans
        }
    }
}

fn patch_base_doc(
    base_doc: &YamlDoc,
    base_lines: &[String],
    ours_doc: Option<&YamlDoc>,
    ours_lines: &[String],
    theirs_doc: Option<&YamlDoc>,
    theirs_lines: &[String],
    choices: &HashMap<String, MergeSide>,
) -> AppResult<String> {
    let Some((start, end)) = doc_slice_bounds(base_doc, base_lines.len()) else {
        return Err(AppError::new(
            "merge.invalid_doc_bounds",
            "Invalid base document bounds",
        ));
    };
    let mut doc_lines = base_lines[start..end].to_vec();
    let base_raw = parse_raw_field_entries(base_doc, base_lines);
    let ours_raw = ours_doc
        .map(|doc| parse_raw_field_entries(doc, ours_lines))
        .unwrap_or_default();
    let theirs_raw = theirs_doc
        .map(|doc| parse_raw_field_entries(doc, theirs_lines))
        .unwrap_or_default();

    for (path, side) in choices {
        let base_entry = base_raw.get(path).ok_or_else(|| {
            AppError::new(
                "merge.unsupported_field_patch",
                format!("Field '{}' cannot be patched semantically", path),
            )
        })?;
        if !base_entry.patchable {
            return Err(AppError::new(
                "merge.unsupported_field_patch",
                format!("Field '{}' requires text-mode resolution", path),
            ));
        }
        let replacement = match side {
            MergeSide::Base => continue,
            MergeSide::Ours => ours_raw.get(path),
            MergeSide::Theirs => theirs_raw.get(path),
        }
        .ok_or_else(|| {
            AppError::new(
                "merge.unsupported_structural_merge",
                format!("Field '{}' is not available on the selected side", path),
            )
        })?;
        if !replacement.patchable {
            return Err(AppError::new(
                "merge.unsupported_structural_merge",
                format!("Field '{}' requires whole-side or text resolution", path),
            ));
        }
        let line = if replacement.raw_value.is_empty() {
            base_entry.prefix.clone()
        } else {
            format!("{} {}", base_entry.prefix, replacement.raw_value)
        };
        if let Some(slot) = doc_lines.get_mut(base_entry.line_index) {
            *slot = line;
        }
    }

    Ok(doc_lines.join("\n"))
}

pub(crate) fn assemble_resolved_yaml(
    session: &MergeSemanticSession,
    resolutions: &HashMap<String, FieldResolution>,
) -> AppResult<AssembledMerge> {
    let missing_conflicts: Vec<String> = session
        .conflict_field_ids
        .iter()
        .filter(|field_id| !resolutions.contains_key(field_id.as_str()))
        .cloned()
        .collect();
    if !missing_conflicts.is_empty() {
        return Err(AppError::new(
            "merge.unresolved_conflicts",
            "Semantic merge still has unresolved fields",
        )
        .detail(missing_conflicts.join(", ")));
    }

    let inspectors = session.inspectors.values().collect::<Vec<_>>();
    if inspectors.is_empty() {
        return Err(AppError::new(
            "merge.no_inspectors",
            "No merge inspectors are available for semantic apply",
        ));
    }

    let mut doc_choices: HashMap<String, HashMap<String, MergeSide>> = HashMap::new();
    for inspector in inspectors {
        for panel in &inspector.panels {
            collect_field_choices(&panel.fields, resolutions, &mut doc_choices)?;
        }
    }

    let header_lines = {
        let preferred = if !session.base_lines.is_empty() {
            &session.base_lines
        } else if !session.ours_lines.is_empty() {
            &session.ours_lines
        } else {
            &session.theirs_lines
        };
        preferred[..first_doc_start(preferred)].to_vec()
    };

    let mut output_docs = Vec::new();
    for plan in build_doc_plans(session) {
        let choice_groups = doc_choice_groups_for_plan(&plan.doc_key, &doc_choices);
        let direct_choices = choice_groups
            .get(&plan.doc_key)
            .cloned()
            .unwrap_or_default();
        let unique_sides = collect_unique_sides(&choice_groups);

        let result_doc = if choice_groups.is_empty() {
            plan.base_doc
                .map(|doc| extract_doc_text(&session.base_lines, doc))
                .or_else(|| {
                    plan.ours_doc
                        .map(|doc| extract_doc_text(&session.ours_lines, doc))
                })
                .or_else(|| {
                    plan.theirs_doc
                        .map(|doc| extract_doc_text(&session.theirs_lines, doc))
                })
        } else if unique_sides.len() == 1 {
            match unique_sides.first().copied().unwrap_or(MergeSide::Base) {
                MergeSide::Base => plan
                    .base_doc
                    .map(|doc| extract_doc_text(&session.base_lines, doc)),
                MergeSide::Ours => plan
                    .ours_doc
                    .map(|doc| extract_doc_text(&session.ours_lines, doc)),
                MergeSide::Theirs => plan
                    .theirs_doc
                    .map(|doc| extract_doc_text(&session.theirs_lines, doc)),
            }
        } else {
            let Some(base_doc) = plan.base_doc else {
                return Err(AppError::new(
                    "merge.unsupported_structural_merge",
                    format!(
                        "Doc '{}' has mixed selections without a base doc",
                        plan.doc_key
                    ),
                ));
            };
            let patched = if base_doc.class_id == 1001 {
                patch_prefab_instance_doc(
                    &plan.doc_key,
                    base_doc,
                    &session.base_lines,
                    plan.ours_doc,
                    &session.ours_lines,
                    plan.theirs_doc,
                    &session.theirs_lines,
                    &choice_groups,
                )?
            } else {
                if choice_groups.keys().any(|raw_key| raw_key != &plan.doc_key) {
                    return Err(AppError::new(
                        "merge.unsupported_structural_merge",
                        format!(
                            "Doc '{}' contains nested semantic groups that require text-mode resolution",
                            plan.doc_key
                        ),
                    ));
                }
                patch_base_doc(
                    base_doc,
                    &session.base_lines,
                    plan.ours_doc,
                    &session.ours_lines,
                    plan.theirs_doc,
                    &session.theirs_lines,
                    &direct_choices,
                )?
            };
            Some(patched)
        };

        if let Some(doc_text) = result_doc {
            if !doc_text.trim().is_empty() {
                output_docs.push(doc_text);
            }
        }
    }

    if output_docs.is_empty() {
        return Ok(AssembledMerge::DeleteFile);
    }

    let mut final_lines = header_lines;
    if !final_lines.is_empty() {
        final_lines.push(String::new());
    }
    for doc in &output_docs {
        final_lines.extend(doc.lines().map(|l| l.to_string()));
    }

    Ok(AssembledMerge::ResolvedText(final_lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::diff::types::{InspectorPanelKind, SemanticLayout, UnityAssetKind};
    use crate::merge::types::{
        DocMergeStatus, FieldResolution, MergeField, MergePanel, MergeSemanticSession, MergeSide,
        MergeState, MergeSummary, MergeTargetInspector,
    };
    use crate::unity_yaml::{extract_prefab_instance_irs, parse_yaml_docs};

    use super::{assemble_resolved_yaml, match_asset_docs_three_way, AssembledMerge};

    fn make_leaf(
        id: &str,
        property_path: &str,
        merge_state: MergeState,
        base: Option<&str>,
        ours: Option<&str>,
        theirs: Option<&str>,
    ) -> MergeField {
        MergeField {
            id: id.to_string(),
            property_path: property_path.to_string(),
            label: property_path.to_string(),
            value_type: "string".into(),
            base: base.map(str::to_string),
            ours: ours.map(str::to_string),
            theirs: theirs.map(str::to_string),
            result: None,
            merge_state,
            auto_choice: None,
            manual_choice: None,
            children: Vec::new(),
            field_type: None,
            reference_base: None,
            reference_ours: None,
            reference_theirs: None,
        }
    }

    fn make_session(
        layout: SemanticLayout,
        asset_kind: UnityAssetKind,
        base_content: &str,
        ours_content: &str,
        theirs_content: &str,
        fields: Vec<MergeField>,
        conflict_ids: &[&str],
    ) -> MergeSemanticSession {
        let base_docs = parse_yaml_docs(base_content.as_bytes());
        let ours_docs = parse_yaml_docs(ours_content.as_bytes());
        let theirs_docs = parse_yaml_docs(theirs_content.as_bytes());
        let inspector = MergeTargetInspector {
            target_id: "target".into(),
            title: "target".into(),
            path: "target".into(),
            panels: vec![MergePanel {
                panel_kind: InspectorPanelKind::Component,
                title: "panel".into(),
                script_class: None,
                component_type: None,
                component_source: None,
                component_inference: None,
                merge_status: DocMergeStatus::HasConflicts,
                fields,
            }],
        };

        MergeSemanticSession {
            layout,
            asset_kind,
            summary: MergeSummary::default(),
            tree: Vec::new(),
            targets: Vec::new(),
            inspectors: HashMap::from([(inspector.target_id.clone(), inspector)]),
            target_locators: HashMap::new(),
            conflict_field_ids: conflict_ids
                .iter()
                .map(|field_id| field_id.to_string())
                .collect::<HashSet<_>>(),
            base_docs,
            ours_docs,
            theirs_docs,
            base_lines: base_content.lines().map(str::to_string).collect(),
            ours_lines: ours_content.lines().map(str::to_string).collect(),
            theirs_lines: theirs_content.lines().map(str::to_string).collect(),
            workspace_hash: 0,
        }
    }

    fn prefab_override_value_map(text: &str) -> HashMap<(i64, String), Option<String>> {
        let docs = parse_yaml_docs(text.as_bytes());
        let lines = text.lines().collect::<Vec<_>>();
        extract_prefab_instance_irs(&docs, &lines)
            .into_iter()
            .next()
            .expect("prefab instance ir")
            .property_overrides
            .into_iter()
            .map(|entry| {
                (
                    (entry.target.source_file_id, entry.property_path),
                    entry.value.clone(),
                )
            })
            .collect()
    }

    fn prefab_removed_component_ids(text: &str) -> Vec<i64> {
        let docs = parse_yaml_docs(text.as_bytes());
        let lines = text.lines().collect::<Vec<_>>();
        let mut ids = extract_prefab_instance_irs(&docs, &lines)
            .into_iter()
            .next()
            .expect("prefab instance ir")
            .removed_components
            .into_iter()
            .map(|entry| entry.target.source_file_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn prefab_override_apply_keeps_source_file_id_scope() {
        let base_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 1
      objectReference: {fileID: 0}
    - target: {fileID: 300, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Layer
      value: 2
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
        let ours_content = base_content
            .replace("value: 1", "value: 11")
            .replace("value: 2", "value: 12");
        let theirs_content = base_content
            .replace("value: 1", "value: 21")
            .replace("value: 2", "value: 22");

        let field_a = "sceneDoc:9000:200|m_Layer";
        let field_b = "sceneDoc:9000:300|m_Layer";
        let session = make_session(
            SemanticLayout::SceneHierarchyInspector,
            UnityAssetKind::Prefab,
            base_content,
            ours_content.as_str(),
            theirs_content.as_str(),
            vec![
                make_leaf(
                    field_a,
                    "m_Layer",
                    MergeState::Conflict,
                    Some("1"),
                    Some("11"),
                    Some("21"),
                ),
                make_leaf(
                    field_b,
                    "m_Layer",
                    MergeState::Conflict,
                    Some("2"),
                    Some("12"),
                    Some("22"),
                ),
            ],
            &[field_a, field_b],
        );

        let resolutions = HashMap::from([
            (
                field_a.to_string(),
                FieldResolution {
                    side: MergeSide::Ours,
                },
            ),
            (
                field_b.to_string(),
                FieldResolution {
                    side: MergeSide::Theirs,
                },
            ),
        ]);
        let assembled = assemble_resolved_yaml(&session, &resolutions).expect("assembled merge");
        let AssembledMerge::ResolvedText(text) = assembled else {
            panic!("expected resolved text");
        };

        let values = prefab_override_value_map(&text);
        assert_eq!(
            values.get(&(200, "m_Layer".to_string())),
            Some(&Some("11".into()))
        );
        assert_eq!(
            values.get(&(300, "m_Layer".to_string())),
            Some(&Some("22".into()))
        );
    }

    #[test]
    fn prefab_override_apply_can_add_new_override_rows() {
        let base_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: BaseName
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
        let ours_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_Name
      value: OursName
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
      propertyPath: m_IsActive
      value: 0
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
        let theirs_content = base_content.replace("BaseName", "TheirsName");

        let name_field = "sceneDoc:9000:200|m_Name";
        let active_field = "sceneDoc:9000:200|m_IsActive";
        let session = make_session(
            SemanticLayout::SceneHierarchyInspector,
            UnityAssetKind::Prefab,
            base_content,
            ours_content,
            theirs_content.as_str(),
            vec![
                make_leaf(
                    name_field,
                    "m_Name",
                    MergeState::Conflict,
                    Some("BaseName"),
                    Some("OursName"),
                    Some("TheirsName"),
                ),
                MergeField {
                    auto_choice: Some(MergeSide::Ours),
                    merge_state: MergeState::Auto,
                    ..make_leaf(
                        active_field,
                        "m_IsActive",
                        MergeState::Auto,
                        None,
                        Some("0"),
                        None,
                    )
                },
            ],
            &[name_field],
        );

        let resolutions = HashMap::from([(
            name_field.to_string(),
            FieldResolution {
                side: MergeSide::Theirs,
            },
        )]);
        let assembled = assemble_resolved_yaml(&session, &resolutions).expect("assembled merge");
        let AssembledMerge::ResolvedText(text) = assembled else {
            panic!("expected resolved text");
        };

        let values = prefab_override_value_map(&text);
        assert_eq!(
            values.get(&(200, "m_Name".to_string())),
            Some(&Some("TheirsName".into()))
        );
        assert_eq!(
            values.get(&(200, "m_IsActive".to_string())),
            Some(&Some("0".into()))
        );
    }

    #[test]
    fn delete_side_assembly_returns_delete_file() {
        let base_content = r#"--- !u!1 &1
GameObject:
  m_Name: Deleted
"#;
        let base_docs = parse_yaml_docs(base_content.as_bytes());
        let empty_docs = Vec::new();
        let field_id = format!(
            "assetDoc:{}|m_Name",
            match_asset_docs_three_way(&base_docs, &empty_docs, &empty_docs)
                .into_iter()
                .next()
                .expect("matched asset doc")
                .key
        );
        let session = make_session(
            SemanticLayout::AssetInspector,
            UnityAssetKind::GenericYaml,
            base_content,
            "",
            "",
            vec![make_leaf(
                &field_id,
                "m_Name",
                MergeState::Conflict,
                Some("Deleted"),
                None,
                None,
            )],
            &[field_id.as_str()],
        );
        let resolutions = HashMap::from([(
            field_id.clone(),
            FieldResolution {
                side: MergeSide::Ours,
            },
        )]);

        let assembled = assemble_resolved_yaml(&session, &resolutions).expect("assembled delete");
        assert_eq!(assembled, AssembledMerge::DeleteFile);
    }

    #[test]
    fn prefab_removed_components_can_merge_across_sides() {
        let base_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications: []
    m_RemovedComponents:
    - {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
        let ours_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications: []
    m_RemovedComponents:
    - {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
    - {fileID: 300, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;
        let theirs_content = r#"--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications: []
    m_RemovedComponents:
    - {fileID: 200, guid: aabbccdd11223344aabbccdd11223344, type: 3}
    - {fileID: 400, guid: aabbccdd11223344aabbccdd11223344, type: 3}
  m_SourcePrefab: {fileID: 100100000, guid: aabbccdd11223344aabbccdd11223344, type: 3}
"#;

        let ours_field = "sceneDoc:9000:removed|removedComponent:300";
        let theirs_field = "sceneDoc:9000:removed|removedComponent:400";
        let session = make_session(
            SemanticLayout::SceneHierarchyInspector,
            UnityAssetKind::Prefab,
            base_content,
            ours_content,
            theirs_content,
            vec![
                make_leaf(
                    ours_field,
                    "removedComponent:300",
                    MergeState::Conflict,
                    None,
                    Some("Removed"),
                    None,
                ),
                make_leaf(
                    theirs_field,
                    "removedComponent:400",
                    MergeState::Conflict,
                    None,
                    None,
                    Some("Removed"),
                ),
            ],
            &[ours_field, theirs_field],
        );

        let resolutions = HashMap::from([
            (
                ours_field.to_string(),
                FieldResolution {
                    side: MergeSide::Ours,
                },
            ),
            (
                theirs_field.to_string(),
                FieldResolution {
                    side: MergeSide::Theirs,
                },
            ),
        ]);

        let assembled = assemble_resolved_yaml(&session, &resolutions).expect("assembled merge");
        let AssembledMerge::ResolvedText(text) = assembled else {
            panic!("expected resolved text");
        };

        assert_eq!(prefab_removed_component_ids(&text), vec![200, 300, 400]);
    }
}
