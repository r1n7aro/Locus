pub(super) fn count_braces(s: &str) -> i32 {
    let mut balance = 0i32;
    for b in s.bytes() {
        match b {
            b'{' => balance += 1,
            b'}' => balance -= 1,
            _ => {}
        }
    }
    balance
}

pub(super) fn parse_doc_header_full(line: &str) -> Option<(i32, i64)> {
    let rest = line.strip_prefix("---")?.trim_start();
    let rest = rest.strip_prefix("!u!")?;
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let class_id = rest[..end].parse::<i32>().ok()?;

    let rest = rest[end..].trim_start();
    let rest = rest.strip_prefix('&')?;
    let digits = if let Some(stripped) = rest.strip_prefix('-') {
        let end = stripped
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(stripped.len());
        if end == 0 {
            return None;
        }
        1 + end
    } else {
        rest.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len())
    };
    if digits == 0 {
        return None;
    }
    let file_id = rest[..digits].parse::<i64>().ok()?;

    Some((class_id, file_id))
}

pub(super) fn extract_field_name(trimmed: &str) -> Option<String> {
    extract_field_name_ref(trimmed).map(|s| s.to_string())
}

/// Like `extract_field_name` but returns a borrowed slice, avoiding allocation.
/// Only usable when the source `trimmed` outlives the return value.
pub(super) fn extract_field_name_ref(trimmed: &str) -> Option<&str> {
    let s = if trimmed.starts_with("- ") {
        trimmed[2..].trim_start()
    } else {
        trimmed
    };

    let colon = s.find(':')?;
    let key = &s[..colon];
    if !key.is_empty()
        && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Some(key)
    } else {
        None
    }
}

pub(super) fn extract_plain_value(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)?;
    let after = &line[start + key.len()..];
    let value = after.trim();
    if value.is_empty() {
        None
    } else {
        Some(decode_yaml_string(value))
    }
}

fn decode_yaml_string(s: &str) -> String {
    let inner = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    };
    if !inner.contains('\\') {
        return inner.to_string();
    }
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('u') | Some('U') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() == 4 {
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                                continue;
                            }
                        }
                    }
                    result.push('\\');
                    result.push('u');
                    result.push_str(&hex);
                }
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub(super) fn extract_internal_file_id(line: &str) -> Option<i64> {
    let fid_str = extract_value(line, "fileID:")?;
    fid_str.trim().trim_end_matches(',').parse::<i64>().ok()
}

pub(super) fn extract_value<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let start = block.find(key)?;
    let after_key = start + key.len();
    let rest = &block[after_key..];
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| c == ',' || c == '}')
        .unwrap_or(rest.len());
    let val = rest[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

pub(super) fn find_closing_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0;
    for i in start..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
