//! Remote compaction v2 retention, matching Codex's default 64K approximate
//! token budget with compaction_image_budget enabled. JSON wrappers and base64
//! length do not count as user-message text.
use base64::Engine;
use serde_json::Value;

fn tokens(text: &str) -> usize {
    text.len().saturating_add(3) / 4
}

fn image_tokens(part: &Value) -> usize {
    if part["detail"] == "original" {
        let dimensions = part["image_url"].as_str().and_then(|url| {
            let (header, payload) = url.split_once(',')?;
            let header = header.to_ascii_lowercase();
            if !header.starts_with("data:image/") || !header.ends_with(";base64") {
                return None;
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()?;
            imagesize::blob_size(&bytes).ok()
        });
        if let Some(size) = dimensions {
            return size
                .width
                .div_ceil(32)
                .saturating_mul(size.height.div_ceil(32))
                .min(10_000);
        }
    }
    // Codex RESIZED_IMAGE_BYTES_ESTIMATE = 7,373; ceil(bytes / 4).
    1_844
}

fn part_tokens(part: &Value) -> usize {
    match part["type"].as_str() {
        Some("input_text" | "output_text") => tokens(part["text"].as_str().unwrap_or_default()),
        Some("input_image") => image_tokens(part),
        _ => 0,
    }
}

fn item_tokens(item: &Value) -> usize {
    if item["type"] == "agent_message" {
        return tokens(&item.to_string());
    }
    item["content"]
        .as_array()
        .map_or(0, |parts| parts.iter().map(part_tokens).sum())
        .max(1)
}

fn retained_source(item: &Value) -> bool {
    if item["type"] == "agent_message" {
        let text = item["content"][0]["text"].as_str().unwrap_or_default();
        let author = item["author"].as_str().unwrap_or_default();
        let recipient = item["recipient"].as_str().unwrap_or_default();
        let descendant_progress = author
            .strip_prefix(recipient)
            .is_some_and(|s| s.starts_with('/'))
            && text.starts_with("Message Type: MESSAGE\n");
        return !descendant_progress
            && !text.starts_with("Message Type: FINAL_ANSWER\n")
            && item_tokens(item) <= 10_000;
    }
    // Base developer/system instructions are rebuilt with every prompt. The
    // upstream retain_client_developer_messages feature defaults to disabled.
    item["role"] == "user"
}

fn attached_notice(item: &Value) -> bool {
    item["role"] == "developer"
        && item["content"].as_array().is_some_and(|parts| {
            parts.len() == 1
                && parts[0]["text"].as_str().is_some_and(|text| {
                    text.starts_with("<image_resize_notice>")
                        && text.ends_with("</image_resize_notice>")
                })
        })
}

// Codex's middle truncation retains both ends and appends a token-count marker.
// The marker is diagnostic overhead, outside its approximate content budget.
fn truncate_text(text: &str, budget: usize) -> String {
    let bytes = budget.saturating_mul(4);
    if text.len() <= bytes {
        return text.to_string();
    }
    let mut left = bytes / 2;
    let mut right = text.len().saturating_sub(bytes - left);
    while !text.is_char_boundary(left) {
        left -= 1;
    }
    while !text.is_char_boundary(right) {
        right += 1;
    }
    format!(
        "{}…{} tokens truncated…{}",
        &text[..left],
        text.len().saturating_sub(bytes).div_ceil(4),
        &text[right..]
    )
}

fn image_open(part: &Value) -> bool {
    part["text"].as_str().is_some_and(|text| {
        (text.starts_with("<image") || text.starts_with("<local_image")) && text.ends_with('>')
    })
}
fn image_close(part: &Value) -> bool {
    matches!(part["text"].as_str(), Some("</image>" | "</local_image>"))
}

fn truncate_item(mut item: Value, budget: usize) -> Option<Value> {
    let parts = item["content"].as_array_mut()?;
    let has_images = parts.iter().any(|part| part["type"] == "input_image");
    let mut remaining = budget;
    let mut retained = Vec::new();
    if has_images {
        // At image boundaries, prefer later content and keep image labels atomic.
        while !parts.is_empty() {
            let last = parts.len() - 1;
            let image_index = if parts[last]["type"] == "input_image" {
                Some(last)
            } else if last > 0
                && image_close(&parts[last])
                && parts[last - 1]["type"] == "input_image"
            {
                Some(last - 1)
            } else {
                None
            };
            if let Some(index) = image_index {
                let start = index - usize::from(index > 0 && image_open(&parts[index - 1]));
                let cost: usize = parts[start..].iter().map(part_tokens).sum();
                let fits = cost <= remaining;
                remaining = if fits { remaining - cost } else { 0 };
                for part in parts.drain(start..).rev() {
                    if fits {
                        retained.push(part);
                    }
                }
            } else {
                let part = parts.pop().unwrap();
                retain_part(part, &mut remaining, &mut retained);
            }
        }
        retained.reverse();
    } else {
        for part in std::mem::take(parts) {
            retain_part(part, &mut remaining, &mut retained);
        }
    }
    if retained.is_empty() {
        return None;
    }
    item["content"] = Value::Array(retained);
    Some(item)
}

fn retain_part(mut part: Value, remaining: &mut usize, retained: &mut Vec<Value>) {
    if matches!(part["type"].as_str(), Some("input_text" | "output_text")) {
        if *remaining == 0 {
            return;
        }
        let text = part["text"].as_str().unwrap_or_default();
        let count = tokens(text);
        if count > *remaining {
            part["text"] = Value::String(truncate_text(text, *remaining));
        }
        *remaining = remaining.saturating_sub(count);
        if part["text"].as_str().is_some_and(|s| !s.is_empty()) {
            retained.push(part);
        }
    } else {
        retained.push(part);
    }
}

pub(super) fn retain(input: Vec<Value>, max_tokens: usize) -> Vec<Value> {
    let mut items = input.into_iter().peekable();
    let mut groups = Vec::new();
    while let Some(source) = items.next() {
        let notice = if items.peek().is_some_and(attached_notice) {
            items.next()
        } else {
            None
        };
        if retained_source(&source) {
            groups.push((source, notice));
        }
    }
    let mut remaining = max_tokens;
    let mut result = Vec::new();
    for (source, notice) in groups.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        let notice_cost = notice.as_ref().map_or(0, item_tokens);
        let cost = item_tokens(&source).saturating_add(notice_cost);
        if cost <= remaining {
            if let Some(notice) = notice {
                result.push(notice);
            }
            result.push(source);
            remaining -= cost;
        } else if remaining > notice_cost {
            if let Some(source) = truncate_item(source, remaining - notice_cost) {
                if let Some(notice) = notice {
                    result.push(notice);
                }
                result.push(source);
            }
            // The boundary consumes the window, even when an image cannot fit.
            remaining = 0;
        } else {
            remaining = 0;
        }
    }
    result.reverse();
    result
}
