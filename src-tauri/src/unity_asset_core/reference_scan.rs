//! Reference-only lexical scan for Unity YAML that the structural parser
//! cannot load: assets beyond its node/byte budgets, or syntax outside its
//! dialect. It builds no syntax tree; cost is a linear pass driven by a SIMD
//! search for the literal `guid`, and memory grows only with the references.
//!
//! Soundness: the parser reports a reference for every mapping whose keys are
//! `fileID` plus optional `guid`/`type` (see `validation::pptr`). Unity writes
//! those as flow mappings `{fileID: N, guid: G, type: T}`, possibly wrapped
//! across lines, which this scan recognizes exactly. Any other `guid` key
//! (block or quoted keys, extra members) is kept as `unrecognized` with its
//! value, because the scan cannot pair it with a fileID. Text that cannot be a
//! key is skipped: a `guid` after other words on its line (YAML keys start a
//! line or follow `{`, `[` or `,`, so string continuation lines are covered),
//! one inside a quoted scalar, and one whose value is not a 32-hex token (such
//! as Wwise's dashed GUIDs), since it can never equal a project asset GUID.

use std::sync::LazyLock;

use regex::bytes::Regex;

/// A `guid` key outside a recognized reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnrecognizedGuid {
    pub offset: usize,
    /// The lowercase GUID written on the key's line; `None` when the value is
    /// not on that line, so it could be any GUID.
    pub guid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedReference {
    /// The enclosing document's fileID, or `$root` for a header-less file.
    pub host_object_id: String,
    pub file_id: String,
    /// As written (callers normalize case).
    pub guid: String,
    /// Byte offset of the reference's opening `{`.
    pub offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceScan {
    /// The root `guid:` of a header-less (`.meta`) file.
    pub own_guid: Option<String>,
    pub references: Vec<ScannedReference>,
    /// GUID keys that may reference an asset in a form the scan cannot pair
    /// with a fileID; callers treat their GUIDs as possibly referenced.
    pub unrecognized: Vec<UnrecognizedGuid>,
}

impl ReferenceScan {
    /// Every reference is recognized.
    pub fn complete(&self) -> bool {
        self.unrecognized.is_empty()
    }
}

static GUID: LazyLock<Regex> = LazyLock::new(|| Regex::new("guid").expect("literal regex"));
static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^---").expect("header regex"));

fn ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}
fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Document fileIDs by header position, in file order.
fn headers(bytes: &[u8]) -> Vec<(usize, String)> {
    HEADER
        .find_iter(bytes)
        .map(|m| {
            let end = bytes[m.start()..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| m.start() + i);
            let line = String::from_utf8_lossy(&bytes[m.start()..end]);
            let id = line
                .split_whitespace()
                .find_map(|part| part.strip_prefix('&'))
                .unwrap_or("$root")
                .to_owned();
            (m.start(), id)
        })
        .collect()
}

/// `{fileID: N, guid: ` ending at the `guid` at `at`: the `{` and fileID.
fn pptr_prefix(bytes: &[u8], at: usize) -> Option<(usize, String)> {
    let mut i = at;
    let back_ws = |i: &mut usize| {
        while *i > 0 && ws(bytes[*i - 1]) {
            *i -= 1;
        }
    };
    back_ws(&mut i);
    if i == 0 || bytes[i - 1] != b',' {
        return None;
    }
    i -= 1;
    back_ws(&mut i);
    let digits_end = i;
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == digits_end {
        return None;
    }
    if i > 0 && bytes[i - 1] == b'-' {
        i -= 1;
    }
    let file_id = String::from_utf8_lossy(&bytes[i..digits_end]).into_owned();
    back_ws(&mut i);
    if i == 0 || bytes[i - 1] != b':' {
        return None;
    }
    i -= 1;
    back_ws(&mut i);
    if i < 6 || &bytes[i - 6..i] != b"fileID" {
        return None;
    }
    i -= 6;
    back_ws(&mut i);
    if i == 0 || bytes[i - 1] != b'{' {
        return None;
    }
    Some((i - 1, file_id))
}

/// `: G[, type: T]}` after the `guid` key ending at `at`: the GUID text.
fn pptr_suffix(bytes: &[u8], mut i: usize) -> Option<String> {
    let skip = |i: &mut usize| {
        while *i < bytes.len() && ws(bytes[*i]) {
            *i += 1;
        }
    };
    skip(&mut i);
    if bytes.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    skip(&mut i);
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
        i += 1;
    }
    if i - start != 32 {
        return None;
    }
    let guid = String::from_utf8_lossy(&bytes[start..i]).into_owned();
    skip(&mut i);
    if bytes.get(i) == Some(&b',') {
        i += 1;
        skip(&mut i);
        if bytes.get(i..i + 4) != Some(b"type") {
            return None;
        }
        i += 4;
        skip(&mut i);
        if bytes.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        skip(&mut i);
        if bytes.get(i) == Some(&b'-') {
            i += 1;
        }
        let digits = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            return None;
        }
        skip(&mut i);
    }
    (bytes.get(i) == Some(&b'}')).then_some(guid)
}

/// Whether the `guid` at `at` is spelled as a mapping key: not part of a
/// longer identifier and followed by `:`. `quoted` reports `"guid"`/`'guid'`.
fn guid_key(bytes: &[u8], at: usize) -> Option<bool> {
    let before = at.checked_sub(1).map(|i| bytes[i]);
    let quote = before.filter(|b| matches!(b, b'"' | b'\''));
    if quote.is_none() && before.is_some_and(ident) {
        return None;
    }
    let mut i = at + 4;
    if let Some(quote) = quote {
        if bytes.get(i) != Some(&quote) {
            return None;
        }
        i += 1;
    } else if bytes.get(i).copied().is_some_and(ident) {
        return None;
    }
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    (bytes.get(i) == Some(&b':')).then_some(quote.is_some())
}

/// Whether `at` lies inside a quoted scalar that opened earlier on its line,
/// such as Unity's `m_Name: 'X (Missing Prefab with guid: ...)'`. A quote
/// opens a scalar only where a token starts (line start or after `:`, `-`,
/// `[`, `{`, `,`); an apostrophe inside a plain word does not. A scalar
/// continued from a previous line is not detected, which only makes the
/// caller more conservative.
fn inside_quoted_scalar(bytes: &[u8], at: usize) -> bool {
    let start = bytes[..at].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut quote: Option<u8> = None;
    let mut previous = None;
    let mut i = start;
    while i < at {
        let b = bytes[i];
        match quote {
            Some(b'\'') if b == b'\'' => {
                if bytes.get(i + 1) == Some(&b'\'') && i + 1 < at {
                    i += 1;
                } else {
                    quote = None;
                }
            }
            Some(b'"') if b == b'\\' => i += 1,
            Some(b'"') if b == b'"' => quote = None,
            Some(_) => {}
            None if matches!(b, b'\'' | b'"')
                && matches!(previous, None | Some(b':' | b'-' | b'[' | b'{' | b',')) =>
            {
                quote = Some(b);
            }
            None => {}
        }
        if quote.is_none() && !matches!(b, b' ' | b'\t') {
            previous = Some(b);
        }
        i += 1;
    }
    quote.is_some()
}

/// Whether a key may start at `start`: at the beginning of its line (after
/// indentation and `- ` sequence markers) or after `{`, `[` or `,`. A word
/// after other text on its line, as in a continued string, is never a key.
fn key_position(bytes: &[u8], start: usize) -> bool {
    let line = bytes[..start].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let prefix = &bytes[line..start];
    let end = prefix.iter().rposition(|b| !matches!(b, b' ' | b'\t')).map_or(0, |i| i + 1);
    let prefix = &prefix[..end];
    match prefix.last() {
        None | Some(b'{' | b'[' | b',') => true,
        Some(b'-') => prefix.iter().all(|b| matches!(b, b' ' | b'\t' | b'-')),
        _ => false,
    }
}

/// The value of the `guid` key whose `:` follows (after spaces) at `at`:
/// `Some(Some(guid))` for a 32-hex token, `Some(None)` when the value is not
/// on this line, and `None` for any other token, which cannot be a GUID.
fn guid_value(bytes: &[u8], mut at: usize) -> Option<Option<String>> {
    let spaces = |i: &mut usize| {
        while matches!(bytes.get(*i), Some(b' ' | b'\t')) {
            *i += 1;
        }
    };
    spaces(&mut at);
    at += 1; // The `:` that `guid_key` found.
    spaces(&mut at);
    if matches!(bytes.get(at), None | Some(b'\r' | b'\n' | b'#')) {
        return Some(None);
    }
    let quote = bytes.get(at).copied().filter(|b| matches!(b, b'\'' | b'"'));
    if quote.is_some() {
        at += 1;
    }
    let start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_hexdigit) {
        at += 1;
    }
    let delimited = match bytes.get(at) {
        None => true,
        Some(b) => matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']' | b'#') || Some(*b) == quote,
    };
    (at - start == 32 && delimited)
        .then(|| Some(String::from_utf8_lossy(&bytes[start..at]).to_ascii_lowercase()))
}

pub fn scan_references(bytes: &[u8]) -> ReferenceScan {
    let headers = headers(bytes);
    let host = |at: usize| match headers.partition_point(|(start, _)| *start <= at) {
        0 => "$root".to_owned(),
        n => headers[n - 1].1.clone(),
    };
    let mut scan = ReferenceScan::default();
    for found in GUID.find_iter(bytes) {
        let at = found.start();
        let Some(quoted) = guid_key(bytes, at) else {
            continue;
        };
        if !quoted {
            if let (Some((brace, file_id)), Some(guid)) =
                (pptr_prefix(bytes, at), pptr_suffix(bytes, at + 4))
            {
                scan.references.push(ScannedReference {
                    host_object_id: host(brace),
                    file_id,
                    guid,
                    offset: brace,
                });
                continue;
            }
            let line_start = at == 0 || bytes[at - 1] == b'\n';
            if line_start && headers.first().is_none_or(|(start, _)| *start > at) && scan.own_guid.is_none() {
                let colon = at + 4 + bytes[at + 4..].iter().position(|b| *b == b':').unwrap_or(0);
                let end = bytes[colon..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| colon + i);
                let value = String::from_utf8_lossy(&bytes[colon + 1..end]).trim().to_owned();
                if !value.is_empty() {
                    scan.own_guid = Some(value);
                    continue;
                }
            }
        }
        // A quoted key starts at its quote. Words later on a line (string
        // continuation lines) and quoted text such as a missing prefab's
        // generated name or JSON kept in a string are never keys.
        let key = if quoted { at - 1 } else { at };
        if !key_position(bytes, key) || inside_quoted_scalar(bytes, key) {
            continue;
        }
        if let Some(guid) = guid_value(bytes, at + 4 + usize::from(quoted)) {
            scan.unrecognized.push(UnrecognizedGuid { offset: at, guid });
        }
    }
    scan
}

/// 1-based line and an approximate field path (keys only, without sequence
/// indices) of the reference at `offset`, for reporting.
pub fn reference_location(bytes: &[u8], offset: usize) -> (usize, String) {
    let offset = offset.min(bytes.len());
    let line = 1 + bytes[..offset].iter().filter(|b| **b == b'\n').count();
    let line_end = bytes[offset..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| offset + i);
    let doc_start = HEADER
        .find_iter(&bytes[..offset])
        .last()
        .map_or(0, |m| {
            bytes[m.start()..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| m.start() + i + 1)
        });
    let mut stack: Vec<(usize, String)> = Vec::new();
    for raw in bytes[doc_start..line_end].split(|b| *b == b'\n') {
        let text = String::from_utf8_lossy(raw);
        let text = text.trim_end_matches('\r');
        let mut column = text.len() - text.trim_start_matches(' ').len();
        let mut rest = &text[column..];
        if rest.is_empty() || rest.starts_with(['#', '%']) {
            continue;
        }
        while let Some(after) = rest.strip_prefix("- ") {
            column += 2;
            rest = after;
        }
        let stop = rest.find(['{', '[', '"', '\'']).unwrap_or(rest.len());
        let Some(colon) = rest[..stop].find(": ").or_else(|| rest[..stop].strip_suffix(':').map(str::len)) else {
            continue;
        };
        while stack.last().is_some_and(|(c, _)| *c >= column) {
            stack.pop();
        }
        stack.push((column, rest[..colon].trim().to_owned()));
    }
    let path = stack
        .iter()
        .map(|(_, key)| format!("/{}", key.replace('~', "~0").replace('/', "~1")))
        .collect();
    (line, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: &str = "0123456789abcdef0123456789ABCDEF";

    #[test]
    fn recognizes_unity_flow_references_including_wrapped_and_hosts() {
        let text = format!(
            "%YAML 1.1\n--- !u!1 &10\nGameObject:\n  m_Component:\n  - component: {{fileID: 11}}\n\
             --- !u!114 &11\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {G}, type: 3}}\n\
             \x20 m_Target: {{fileID: -42,\n    guid: {G},\n    type: 3}}\n  m_Other: {{fileID: 7, guid: {G}}}\n"
        );
        let scan = scan_references(text.as_bytes());
        assert!(scan.complete());
        assert_eq!(scan.own_guid, None);
        let found: Vec<_> = scan.references.iter().map(|r| (r.host_object_id.as_str(), r.file_id.as_str())).collect();
        assert_eq!(found, [("11", "11500000"), ("11", "-42"), ("11", "7")]);
        let (line, path) = reference_location(text.as_bytes(), scan.references[1].offset);
        assert_eq!((line, path.as_str()), (9, "/MonoBehaviour/m_Target"));
    }

    #[test]
    fn meta_own_guid_is_not_an_unrecognized_key() {
        let text = format!("fileFormatVersion: 2\nguid: {G}\nPluginImporter:\n  platformData:\n  - first:\n      : Any\n");
        let scan = scan_references(text.as_bytes());
        assert!(scan.complete());
        assert_eq!(scan.own_guid.as_deref(), Some(G));
        assert!(scan.references.is_empty());
    }

    #[test]
    fn unrecognized_guid_keys_keep_their_values() {
        let lower = Some(G.to_ascii_lowercase());
        for (text, expected) in [
            // Forms the parser could read as a reference without a recognizable fileID pairing.
            (format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref:\n    fileID: 3\n    guid: {G}\n"), Some(lower.clone())),
            (format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref: {{fileID: 3, guid: {G}, type: 3, extra: 1}}\n"), Some(lower.clone())),
            (format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref: {{fileID: 3, \"guid\": '{G}'}}\n"), Some(lower.clone())),
            (format!("fileFormatVersion: 2\nguid: {G}\nguid: {G}\n"), Some(lower.clone())),
            // A value below its key could be any GUID.
            (format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref:\n    guid:\n      {G}\n"), Some(None)),
            // Values that can never equal a project asset GUID.
            ("--- !u!114 &1\nMonoBehaviour:\n  m_Ref: {fileID: 3, guid: 0}\n".to_owned(), None),
            ("--- !u!114 &1\nMonoBehaviour:\n  wwise:\n    guid: 00FFFCE6-9E61-4205-8AE9-DEDCAD2C0E03\n".to_owned(), None),
            (format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref:\n    guid: {G}0\n"), None),
        ] {
            let scan = scan_references(text.as_bytes());
            let found: Vec<_> = scan.unrecognized.iter().map(|u| u.guid.clone()).collect();
            assert_eq!(found, expected.into_iter().collect::<Vec<_>>(), "{text}");
        }
    }

    #[test]
    fn multi_line_strings_only_keep_a_guid_key_starting_a_continuation_line() {
        // Mid-line text of a continued string can never be a key.
        let mid = format!(
            "--- !u!114 &1\nMonoBehaviour:\n  m_Text: 'a long description that wraps and then\n    names the guid: {G} of a prefab\n\n'\n\
             \x20 m_Json: '{{\"obj\":{{\"fileID\":2800000,\"guid\":\"{G}\",\"type\":3}}}}'\n"
        );
        assert!(scan_references(mid.as_bytes()).complete(), "{mid}");
        // A continuation line that starts with `guid:` is indistinguishable from
        // a key without the full string state, so it stays conservative.
        let start = format!("--- !u!114 &1\nMonoBehaviour:\n  m_Text: 'a long description\n    guid: {G} more'\n");
        let scan = scan_references(start.as_bytes());
        assert_eq!(scan.unrecognized.len(), 1);
        assert_eq!(scan.unrecognized[0].guid.as_deref(), Some(G.to_ascii_lowercase().as_str()));
    }

    #[test]
    fn non_key_mentions_are_ignored() {
        let text = format!(
            "--- !u!114 &1\nMonoBehaviour:\n  m_AssetGUID: {G}\n  m_guid: x\n  guidance: y\n  m_Name: guides\n\
             \x20 m_Name: 'House (Missing Prefab with guid: {G})'\n  m_Text: \"a \\\"q\\\" guid: {G}\"\n\
             \x20 m_Note: 'it''s guid: {G}'\n"
        );
        let scan = scan_references(text.as_bytes());
        assert!(scan.complete(), "{text}");
        assert!(scan.references.is_empty());
        // An apostrophe inside a plain word opens no string: the key counts.
        let plain = format!("--- !u!114 &1\nMonoBehaviour:\n  m_Ref: {{name: Bob's, guid: {G}}}\n");
        assert!(!scan_references(plain.as_bytes()).complete());
    }
}
