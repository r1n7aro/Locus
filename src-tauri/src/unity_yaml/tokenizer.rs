/// Count `{` minus `}` outside quoted scalars. Quote awareness matters:
/// a string value like `m_Text: 'HP {'` must NOT look brace-open, or the
/// parser's multi-line flow-map joiner would swallow every following line
/// (including `---` document headers) until a spare `}` shows up — silently
/// dropping the rest of the file. A quote left unclosed at end of line (the
/// start of a multi-line quoted scalar) keeps its braces ignored, which is
/// exactly the safe reading.
pub(super) fn count_braces(s: &str) -> i32 {
    let mut balance = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                // `''` is the single-quote escape; consume both and stay in.
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
        } else if in_double {
            match b {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'"' => in_double = false,
                _ => {}
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'{' => balance += 1,
                b'}' => balance -= 1,
                _ => {}
            }
        }
        i += 1;
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

/// The raw (still-quoted, undecoded) value text after `key` on this line;
/// empty when the key is absent.
pub(super) fn trimmed_value_after<'a>(line: &'a str, key: &str) -> &'a str {
    match line.find(key) {
        Some(start) => line[start + key.len()..].trim(),
        None => "",
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
    // Single-quoted scalar — Unity's emitter quotes in this style whenever a
    // string needs quoting at all (`m_Name: '[Managers] '`, `m_Name: '>'`,
    // trailing spaces, leading `-`/`:`), so this is the common quoted form in
    // real projects. The only escape in single-quoted YAML is `''` → `'`;
    // backslashes are literal, so skip the escape loop below.
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        let inner = &s[1..s.len() - 1];
        if inner.contains("''") {
            return inner.replace("''", "'");
        }
        return inner.to_string();
    }
    let inner = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    };
    if !inner.contains('\\') {
        return inner.to_string();
    }
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                // YAML double-quoted escapes: `\u` takes 4 hex digits, `\U`
                // takes 8 (astral chars — emoji GameObject names land here).
                // A high surrogate from `\u` pairs with a following `\uDC00..`
                // escape the way .NET emitters write astral chars.
                Some(u_kind @ ('u' | 'U')) => {
                    let want = if u_kind == 'U' { 8 } else { 4 };
                    let hex: String = chars.by_ref().take(want).collect();
                    let decoded = (hex.len() == want)
                        .then(|| u32::from_str_radix(&hex, 16).ok())
                        .flatten();
                    match decoded {
                        Some(code) if (0xD800..0xDC00).contains(&code) => {
                            // High surrogate: try to pair with a `\uXXXX` low
                            // surrogate right behind it.
                            let mut paired = None;
                            if chars.peek() == Some(&'\\') {
                                let mut ahead = chars.clone();
                                ahead.next(); // '\\'
                                if matches!(ahead.next(), Some('u' | 'U')) {
                                    let low_hex: String = ahead.by_ref().take(4).collect();
                                    if low_hex.len() == 4 {
                                        if let Ok(low) = u32::from_str_radix(&low_hex, 16) {
                                            if (0xDC00..0xE000).contains(&low) {
                                                let combined = 0x10000
                                                    + ((code - 0xD800) << 10)
                                                    + (low - 0xDC00);
                                                if let Some(ch) = char::from_u32(combined) {
                                                    paired = Some((ch, ahead));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some((ch, ahead)) = paired {
                                result.push(ch);
                                chars = ahead;
                            } else {
                                result.push('\\');
                                result.push(u_kind);
                                result.push_str(&hex);
                            }
                        }
                        Some(code) => {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                            } else {
                                result.push('\\');
                                result.push(u_kind);
                                result.push_str(&hex);
                            }
                        }
                        None => {
                            result.push('\\');
                            result.push(u_kind);
                            result.push_str(&hex);
                        }
                    }
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

/// True when `value` starts a single-quoted scalar whose closing quote is not
/// on this line (Unity writes multi-line strings — prefab `value:` overrides,
/// long names — as single-quoted scalars spilling across lines). `''` escapes
/// are not closers.
pub(super) fn is_unclosed_single_quoted(value: &str) -> bool {
    let Some(inner) = value.strip_prefix('\'') else {
        return false;
    };
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return false;
        }
        i += 1;
    }
    true
}

/// True when this continuation line of a multi-line single-quoted scalar
/// contains the closing quote (an odd trailing unescaped `'`).
pub(super) fn closes_single_quoted(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
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
    let mut in_single = false;
    let mut in_double = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
        } else if in_double {
            match b {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'"' => in_double = false,
                _ => {}
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
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
        i += 1;
    }
    None
}
