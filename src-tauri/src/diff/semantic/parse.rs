use std::collections::HashMap;

use crate::asset_db::types::{guid_to_hex, parse_guid_hex};
use crate::unity_yaml::YamlDoc;

use super::ParsedFieldLine;
use crate::diff::context::SideContext;
use crate::diff::types::InspectorReference;

pub(crate) fn leading_spaces(line: &str) -> usize {
    line.as_bytes().iter().take_while(|b| **b == b' ').count()
}

pub(crate) fn split_key_value(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), line[colon + 1..].trim().to_string()))
}

pub(crate) fn strip_quotes(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].replace("\\\"", "\"")
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn extract_flow_value(block: &str, key: &str) -> Option<String> {
    let start = block.find(key)?;
    let after = &block[start + key.len()..];
    let after = after.trim_start();
    let end = after
        .find(|c: char| c == ',' || c == '}')
        .unwrap_or(after.len());
    let value = after[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub(crate) fn parse_reference(
    raw_value: &str,
    side_ctx: &SideContext,
    local_labels: &HashMap<i64, String>,
) -> Option<InspectorReference> {
    if !raw_value.contains("fileID:") {
        return None;
    }

    let file_id = extract_flow_value(raw_value, "fileID:")
        .and_then(|value| value.trim_end_matches(',').parse::<i64>().ok());
    let guid = extract_flow_value(raw_value, "guid:")
        .and_then(|value| parse_guid_hex(value.trim().trim_end_matches(',')))
        .filter(|guid| *guid != [0u8; 16]);
    let path = guid
        .as_ref()
        .and_then(|guid| side_ctx.resolve_guid_path(guid));
    let local_path = if guid.is_none() {
        file_id.and_then(|file_id| local_labels.get(&file_id).cloned())
    } else {
        None
    };

    if guid.is_none() && file_id.is_none() {
        return None;
    }

    // Diagnostic hint when GUID exists but path resolution failed
    let resolve_hint = if guid.is_some() && path.is_none() {
        Some("not_in_asset_db: GUID not found in project asset database (.meta files)".into())
    } else {
        None
    };
    // Mark as stale when resolved via current workspace RefGraph for a snapshot side
    let stale = path.is_some() && side_ctx.is_snapshot();

    Some(InspectorReference {
        guid: guid.as_ref().map(guid_to_hex),
        path: path.or(local_path),
        file_id,
        resolve_hint,
        stale,
    })
}

pub(crate) fn reference_display(reference: &InspectorReference) -> String {
    match (&reference.path, &reference.guid, reference.file_id) {
        (Some(path), _, Some(file_id)) => format!("{} (fileID:{})", path, file_id),
        (Some(path), _, None) => path.clone(),
        (None, Some(guid), Some(file_id)) => format!("{} (fileID:{})", guid, file_id),
        (None, Some(guid), None) => guid.clone(),
        (None, None, Some(file_id)) => format!("fileID:{}", file_id),
        _ => "(none)".into(),
    }
}

pub(crate) fn parse_doc_field_map(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    local_labels: &HashMap<i64, String>,
) -> HashMap<String, ParsedFieldLine> {
    let mut fields = HashMap::new();
    let start = doc.line_start.min(lines.len());
    let end = doc.line_end.min(lines.len());
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

        // For list items (starting with "- "), only pop items with indent STRICTLY
        // greater than current indent. In Unity YAML, list items sit at the same
        // indent as their parent key:
        //   _actions:          (indent=2, pushed to stack)
        //   - {fileID: ...}    (indent=2, child of _actions)
        // Without this, the parent would be popped before the list item can use it.
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
            let item_label = format!("[{}]", *index);
            *index += 1;

            let item_path = format!("{}{}", parent_path, item_label);
            fields.entry(item_path.clone()).or_insert(ParsedFieldLine {
                label: item_label.clone(),
                value: None,
                reference: None,
            });

            let rest = rest.trim();
            if rest.is_empty() {
                stack.push((indent, item_path));
                continue;
            }

            // Skip split_key_value for flow maps like {fileID: 11400000, guid: ...}
            // These should be treated as plain reference values, not key:value pairs.
            if !rest.starts_with('{') {
                if let Some((key, value)) = split_key_value(rest) {
                    let child_path = format!("{}.{}", item_path, key);
                    let reference = parse_reference(&value, side_ctx, local_labels);
                    let display_value = if value.is_empty() {
                        None
                    } else if let Some(reference) = reference.as_ref() {
                        Some(reference_display(reference))
                    } else {
                        Some(strip_quotes(&value))
                    };
                    fields.insert(
                        child_path.clone(),
                        ParsedFieldLine {
                            label: key,
                            value: display_value,
                            reference,
                        },
                    );
                    stack.push((indent, item_path));
                    if value.is_empty() {
                        stack.push((indent + 1, child_path));
                    }
                    continue;
                }
            }

            let reference = parse_reference(rest, side_ctx, local_labels);
            let display_value = if let Some(reference) = reference.as_ref() {
                Some(reference_display(reference))
            } else {
                Some(strip_quotes(rest))
            };
            fields.insert(
                item_path.clone(),
                ParsedFieldLine {
                    label: item_label,
                    value: display_value,
                    reference,
                },
            );
            continue;
        }

        if let Some((key, value)) = split_key_value(trimmed) {
            let path = match stack.last() {
                Some((_, parent)) => format!("{}.{}", parent, key),
                None => key.clone(),
            };
            let reference = parse_reference(&value, side_ctx, local_labels);
            let display_value = if value.is_empty() {
                None
            } else if let Some(reference) = reference.as_ref() {
                Some(reference_display(reference))
            } else {
                Some(strip_quotes(&value))
            };
            fields.insert(
                path.clone(),
                ParsedFieldLine {
                    label: key,
                    value: display_value,
                    reference,
                },
            );
            if value.is_empty() {
                stack.push((indent, path));
            }
        }
    }

    fields
}

pub(crate) fn split_property_path(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                current.push('[');
                while let Some(next) = chars.next() {
                    current.push(next);
                    if next == ']' {
                        break;
                    }
                }
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

pub(crate) fn join_property_segments(segments: &[String]) -> String {
    let mut path = String::new();
    for segment in segments {
        if segment.starts_with('[') {
            path.push_str(segment);
        } else if path.is_empty() {
            path.push_str(segment);
        } else {
            path.push('.');
            path.push_str(segment);
        }
    }
    path
}

pub(crate) fn prettify_field_label(raw: &str) -> String {
    if raw.is_empty() || raw.starts_with('[') {
        return raw.to_string();
    }

    let trimmed = raw.trim_start_matches('_');
    if trimmed.is_empty() {
        return raw.to_string();
    }

    let mut words = Vec::new();
    let mut current = String::new();
    let chars = trimmed.chars().collect::<Vec<_>>();

    for (index, ch) in chars.iter().copied().enumerate() {
        let prev = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();

        if ch == '_' || ch == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        let boundary = match prev {
            Some(prev) if prev != '_' && prev != '-' => {
                (prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
                    || (prev.is_ascii_uppercase()
                        && ch.is_ascii_uppercase()
                        && next
                            .map(|value| value.is_ascii_lowercase())
                            .unwrap_or(false))
                    || (prev.is_ascii_alphabetic() && ch.is_ascii_digit())
                    || (prev.is_ascii_digit() && ch.is_ascii_alphabetic())
            }
            _ => false,
        };

        if boundary && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }

    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return raw.to_string();
    }

    words
        .into_iter()
        .map(|word| {
            if word.chars().all(|ch| !ch.is_ascii_lowercase()) {
                word
            } else {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => {
                        let mut result = first.to_uppercase().to_string();
                        result.push_str(chars.as_str());
                        result
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn prettify_builtin_field_label(raw: &str) -> String {
    let trimmed = raw
        .strip_prefix("m_")
        .filter(|value| !value.is_empty())
        .unwrap_or(raw);
    prettify_field_label(trimmed)
}
