use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub const PARSER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn bytes<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.start..self.end]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub span: Span,
    pub object_id: Option<String>,
    pub property_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreError {
    pub code: String,
    pub message: String,
    pub offset: usize,
}

impl CoreError {
    pub(crate) fn new(code: &str, message: impl Into<String>, offset: usize) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            offset,
        }
    }
}
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}: {}", self.code, self.offset, self.message)
    }
}
impl std::error::Error for CoreError {}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024 * 1024,
            max_nodes: 2_000_000,
            max_depth: 128,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarStyle {
    Empty,
    Null,
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
    Folded,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub key_span: Span,
    /// Entire entry for block syntax; key/value only for flow syntax.
    pub span: Span,
    pub value: Node,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub span: Span,
    pub value: Node,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Scalar(ScalarStyle),
    Mapping(Vec<Entry>),
    Sequence(Vec<Item>),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub span: Span,
    pub kind: NodeKind,
    pub flow: bool,
    pub indent: usize,
    /// Semantic structure hash. Scalars intentionally retain their lexical type.
    pub fingerprint: [u8; 32],
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match &self.kind {
            NodeKind::Mapping(entries) => entries.iter().find(|e| e.key == key).map(|e| &e.value),
            _ => None,
        }
    }
    pub fn entries(&self) -> Option<&[Entry]> {
        match &self.kind {
            NodeKind::Mapping(v) => Some(v),
            _ => None,
        }
    }
    pub fn items(&self) -> Option<&[Item]> {
        match &self.kind {
            NodeKind::Sequence(v) => Some(v),
            _ => None,
        }
    }
    pub fn scalar<'a>(&self, asset: &'a Asset) -> Option<&'a str> {
        match self.kind {
            NodeKind::Scalar(_) => Some(asset.text(self.span)),
            _ => None,
        }
    }
    pub fn is_scalar(&self) -> bool {
        matches!(self.kind, NodeKind::Scalar(_))
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    /// A decimal string, never converted through a floating point representation.
    pub object_id: String,
    pub class_id: Option<String>,
    pub stripped: bool,
    pub header: Span,
    pub span: Span,
    pub root: Node,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub bytes: Arc<[u8]>,
    pub documents: Vec<Document>,
    pub diagnostics: Vec<Diagnostic>,
    pub content_hash: String,
    pub newline: &'static str,
    document_index: HashMap<String, usize>,
    node_count: usize,
}

impl Asset {
    /// An absent Git file, explicitly represented by the caller. Parsing an
    /// existing empty file still fails, preserving absent/invalid distinction.
    pub fn absent() -> Self {
        Self {
            bytes: Arc::from([]),
            documents: Vec::new(),
            diagnostics: Vec::new(),
            content_hash: blake3::hash(&[]).to_hex().to_string(),
            newline: "\n",
            document_index: HashMap::new(),
            node_count: 0,
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, CoreError> {
        parse(bytes)
    }
    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self, CoreError> {
        Parser::new(bytes, limits)?.asset()
    }
    pub fn text(&self, span: Span) -> &str {
        // UTF-8 is checked once, before any spans are built.
        std::str::from_utf8(span.bytes(&self.bytes)).expect("parser spans remain UTF-8 boundaries")
    }
    pub fn document(&self, object_id: &str) -> Option<&Document> {
        self.document_index
            .get(object_id)
            .and_then(|index| self.documents.get(*index))
    }
    pub fn is_writable(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Asset, CoreError> {
    Asset::parse_with_limits(bytes, Limits::default())
}

const PARSE_CACHE_BYTE_BUDGET: usize = 128 * 1024 * 1024;
#[derive(Default)]
struct ParseCache {
    entries: HashMap<String, (Arc<Asset>, usize)>,
    order: VecDeque<String>,
    bytes: usize,
}
static PARSE_CACHE: OnceLock<Mutex<ParseCache>> = OnceLock::new();

/// Shared immutable syntax only; GUID/schema resolution remains snapshot-local.
pub fn parse_shared(bytes: &[u8]) -> Result<Arc<Asset>, CoreError> {
    let hash = blake3::hash(bytes).to_hex().to_string();
    let cache = PARSE_CACHE.get_or_init(|| Mutex::new(ParseCache::default()));
    if let Ok(mut guard) = cache.lock() {
        if let Some((asset, _)) = guard.entries.get(&hash) {
            let asset = Arc::clone(asset);
            guard.order.retain(|key| key != &hash);
            guard.order.push_back(hash.clone());
            return Ok(asset);
        }
    }
    let asset = Arc::new(parse(bytes)?);
    // Bound retained syntax allocations as well as raw bytes. Active sessions
    // retain independent Arc leases and cannot be invalidated by eviction.
    let cost = bytes
        .len()
        .saturating_mul(2)
        .saturating_add(asset.node_count.saturating_mul(256));
    if cost <= PARSE_CACHE_BYTE_BUDGET {
        if let Ok(mut guard) = cache.lock() {
            if let Some((cached, _)) = guard.entries.get(&hash) {
                return Ok(Arc::clone(cached));
            }
            while guard.bytes.saturating_add(cost) > PARSE_CACHE_BYTE_BUDGET
                || guard.entries.len() >= 4096
            {
                let Some(key) = guard.order.pop_front() else {
                    break;
                };
                if let Some((_, bytes)) = guard.entries.remove(&key) {
                    guard.bytes = guard.bytes.saturating_sub(bytes);
                }
            }
            guard.bytes += cost;
            guard.order.push_back(hash.clone());
            guard.entries.insert(hash, (Arc::clone(&asset), cost));
        }
    }
    Ok(asset)
}

#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
    full_end: usize,
    indent: usize,
}

struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    lines: Vec<Line>,
    limits: Limits,
    nodes: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8], limits: Limits) -> Result<Self, CoreError> {
        if bytes.len() > limits.max_bytes {
            return Err(CoreError::new(
                "size_limit",
                "asset exceeds parser byte budget",
                0,
            ));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| {
            CoreError::new(
                "invalid_utf8",
                "opaque/non-UTF-8 asset requires whole-file selection",
                e.valid_up_to(),
            )
        })?;
        if bytes.contains(&0) {
            return Err(CoreError::new("binary_asset", "NUL byte in YAML input", 0));
        }
        let mut lines = Vec::new();
        let mut start = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            3
        } else {
            0
        };
        while start < bytes.len() {
            let newline = bytes[start..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| i + start);
            let full_end = newline.map(|i| i + 1).unwrap_or(bytes.len());
            let mut end = newline.unwrap_or(bytes.len());
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            let indent = bytes[start..end].iter().take_while(|b| **b == b' ').count();
            if start + indent < end && bytes[start + indent] == b'\t' {
                return Err(CoreError::new(
                    "tab_indentation",
                    "tab indentation is outside the supported Unity dialect",
                    start + indent,
                ));
            }
            lines.push(Line {
                start,
                end,
                full_end,
                indent,
            });
            start = full_end;
        }
        Ok(Self {
            bytes,
            text,
            lines,
            limits,
            nodes: 0,
            diagnostics: Vec::new(),
        })
    }

    fn error(&self, code: &str, message: impl Into<String>, offset: usize) -> CoreError {
        CoreError::new(code, message, offset)
    }
    fn line_text(&self, i: usize) -> &'a str {
        let l = self.lines[i];
        &self.text[l.start + l.indent..l.end]
    }
    fn trivia(&self, i: usize) -> bool {
        let s = self.line_text(i);
        s.is_empty() || s.starts_with('#')
    }
    fn skip(&self, mut i: usize, end: usize) -> usize {
        while i < end && self.trivia(i) {
            i += 1;
        }
        i
    }
    fn sequence_line(&self, i: usize) -> bool {
        let s = self.line_text(i);
        s == "-" || s.starts_with("- ")
    }
    fn diagnostic(&mut self, code: &str, message: String, span: Span) {
        self.diagnostics.push(Diagnostic {
            code: code.into(),
            message,
            span,
            severity: Severity::Error,
            object_id: None,
            property_path: None,
        });
    }

    fn asset(mut self) -> Result<Asset, CoreError> {
        let mut docs = Vec::new();
        let mut ids = HashSet::new();
        let mut i = self.skip(0, self.lines.len());
        while i < self.lines.len() && self.line_text(i).starts_with('%') {
            i = self.skip(i + 1, self.lines.len());
        }
        while i < self.lines.len() {
            let start = i;
            let mut object_id = "$root".to_owned();
            let mut class_id = None;
            let mut stripped = false;
            let mut header = Span::new(self.lines[i].start, self.lines[i].start);
            if self.line_text(i).starts_with("---") {
                let h = self.line_text(i);
                header.end = self.lines[i].full_end;
                if h != "---" {
                    let parts: Vec<_> = h.split_whitespace().collect();
                    if !(parts.len() == 3 || (parts.len() == 4 && parts[3] == "stripped"))
                        || parts[0] != "---"
                        || !parts[1].starts_with("!u!")
                        || !parts[2].starts_with('&')
                    {
                        return Err(self.error(
                            "document_header",
                            "expected a Unity !u! class tag and fileID anchor",
                            self.lines[i].start,
                        ));
                    }
                    let cid = &parts[1][3..];
                    let fid = &parts[2][1..];
                    if cid.parse::<i32>().is_err() || fid.parse::<i64>().is_err() {
                        return Err(self.error(
                            "document_id",
                            "classID/fileID is outside its signed integer range",
                            self.lines[i].start,
                        ));
                    }
                    class_id = Some(cid.to_owned());
                    object_id = fid.to_owned();
                    stripped = parts.len() == 4;
                }
                i = self.skip(i + 1, self.lines.len());
            }
            if !ids.insert(object_id.clone()) {
                self.diagnostic(
                    "duplicate_object_id",
                    format!("duplicate document fileID {object_id}"),
                    header,
                );
            }
            let mut end = i;
            while end < self.lines.len()
                && !(self.lines[end].indent == 0
                    && (self.line_text(end).starts_with("---") || self.line_text(end) == "..."))
            {
                end += 1;
            }
            if i >= end {
                return Err(self.error(
                    "empty_document",
                    "Unity asset document has no root",
                    header.end,
                ));
            }
            let indent = self.lines[i].indent;
            let (root, consumed) = self.block(i, end, indent, 0)?;
            if self.skip(consumed, end) != end {
                return Err(self.error(
                    "unparsed_content",
                    "content remains after the root node",
                    self.lines[consumed].start,
                ));
            }
            if !matches!(root.kind, NodeKind::Mapping(_)) {
                return Err(self.error(
                    "document_root",
                    "Unity asset root must be a mapping",
                    root.span.start,
                ));
            }
            let doc_end = if end < self.lines.len() {
                self.lines[end].start
            } else {
                self.bytes.len()
            };
            docs.push(Document {
                object_id,
                class_id,
                stripped,
                header,
                span: Span::new(self.lines[start].start, doc_end),
                root,
            });
            i = end;
            if i < self.lines.len() && self.line_text(i) == "..." {
                i = self.skip(i + 1, self.lines.len());
            }
        }
        if docs.is_empty() {
            return Err(self.error("empty_asset", "no Unity YAML documents", 0));
        }
        let document_index = docs
            .iter()
            .enumerate()
            .map(|(index, doc)| (doc.object_id.clone(), index))
            .collect();
        Ok(Asset {
            bytes: Arc::from(self.bytes),
            documents: docs,
            document_index,
            node_count: self.nodes,
            diagnostics: self.diagnostics,
            content_hash: blake3::hash(self.bytes).to_hex().to_string(),
            newline: if self.bytes.windows(2).any(|w| w == b"\r\n") {
                "\r\n"
            } else {
                "\n"
            },
        })
    }

    fn make(
        &mut self,
        span: Span,
        kind: NodeKind,
        flow: bool,
        indent: usize,
        depth: usize,
    ) -> Result<Node, CoreError> {
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err(self.error("node_limit", "asset exceeds parser node budget", span.start));
        }
        if depth > self.limits.max_depth {
            return Err(self.error(
                "depth_limit",
                "asset exceeds parser nesting budget",
                span.start,
            ));
        }
        let mut hash = blake3::Hasher::new();
        match &kind {
            NodeKind::Scalar(style) => {
                hash.update(&[0, *style as u8]);
                hash.update(span.bytes(self.bytes));
            }
            NodeKind::Mapping(entries) => {
                hash.update(&[1]);
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by(|a, b| a.key.cmp(&b.key));
                for e in sorted {
                    hash.update(&(e.key.len() as u64).to_le_bytes());
                    hash.update(e.key.as_bytes());
                    hash.update(&e.value.fingerprint);
                }
            }
            NodeKind::Sequence(items) => {
                hash.update(&[2]);
                for item in items {
                    hash.update(&item.value.fingerprint);
                }
            }
        }
        Ok(Node {
            span,
            kind,
            flow,
            indent,
            fingerprint: *hash.finalize().as_bytes(),
        })
    }

    fn block(
        &mut self,
        i: usize,
        end: usize,
        indent: usize,
        depth: usize,
    ) -> Result<(Node, usize), CoreError> {
        if depth > self.limits.max_depth {
            return Err(self.error(
                "depth_limit",
                "asset exceeds parser nesting budget",
                self.lines[i].start,
            ));
        }
        if self.sequence_line(i) {
            self.sequence(i, end, indent, depth + 1)
        } else {
            self.mapping(i, end, indent, None, depth + 1)
        }
    }

    fn mapping(
        &mut self,
        mut i: usize,
        end: usize,
        indent: usize,
        mut first: Option<usize>,
        depth: usize,
    ) -> Result<(Node, usize), CoreError> {
        let start = first.unwrap_or(self.lines[i].start + indent);
        let mut entries = Vec::new();
        let mut keys = HashSet::new();
        let mut node_end = start;
        loop {
            i = self.skip(i, end);
            if i >= end {
                break;
            }
            let l = self.lines[i];
            let key_start = if let Some(pos) = first.take() {
                pos
            } else {
                if l.indent != indent || self.sequence_line(i) {
                    break;
                }
                l.start + indent
            };
            let colon = find_colon(self.bytes, key_start, l.end).ok_or_else(|| {
                self.error(
                    "mapping_key",
                    "expected a mapping key followed by ':'",
                    key_start,
                )
            })?;
            let key_end = trim_end(self.bytes, key_start, colon);
            let key_span = Span::new(key_start, key_end);
            let key = decode_key(&self.text[key_start..key_end])
                .map_err(|m| self.error("mapping_key", m, key_start))?;
            if key == "<<" {
                return Err(self.error(
                    "yaml_merge_key",
                    "YAML aliases/merge keys are outside the Unity dialect",
                    key_start,
                ));
            }
            if !keys.insert(key.clone()) {
                self.diagnostic(
                    "duplicate_key",
                    format!("duplicate mapping key {key}"),
                    key_span,
                );
            }
            let value_start = skip_spaces(self.bytes, colon + 1, l.end);
            let (value, next) = self.block_value(i, end, value_start, indent, depth, true)?;
            let entry_start = if key_start == l.start + l.indent {
                l.start
            } else {
                key_start
            };
            let entry_end = self.lines[next.saturating_sub(1)].full_end;
            node_end = entry_end;
            entries.push(Entry {
                key,
                key_span,
                span: Span::new(entry_start, entry_end),
                value,
            });
            i = next;
        }
        if entries.is_empty() {
            return Err(self.error("empty_mapping", "expected a mapping entry", start));
        }
        Ok((
            self.make(
                Span::new(start, node_end),
                NodeKind::Mapping(entries),
                false,
                indent,
                depth,
            )?,
            i,
        ))
    }

    fn sequence(
        &mut self,
        mut i: usize,
        end: usize,
        indent: usize,
        depth: usize,
    ) -> Result<(Node, usize), CoreError> {
        let start = self.lines[i].start + indent;
        let mut items = Vec::new();
        let mut node_end = start;
        loop {
            i = self.skip(i, end);
            if i >= end || self.lines[i].indent != indent || !self.sequence_line(i) {
                break;
            }
            let l = self.lines[i];
            let value_start = skip_spaces(self.bytes, l.start + indent + 1, l.end);
            let mapping_colon = find_colon(self.bytes, value_start, l.end);
            let (value, next) = if mapping_colon.is_some()
                && !matches!(
                    self.bytes.get(value_start),
                    Some(b'{') | Some(b'[') | Some(b'\'') | Some(b'"')
                ) {
                self.mapping(i, end, indent + 2, Some(value_start), depth + 1)?
            } else {
                self.block_value(i, end, value_start, indent, depth + 1, false)?
            };
            node_end = self.lines[next.saturating_sub(1)].full_end;
            items.push(Item {
                span: Span::new(l.start, node_end),
                value,
            });
            i = next;
        }
        Ok((
            self.make(
                Span::new(start, node_end),
                NodeKind::Sequence(items),
                false,
                indent,
                depth,
            )?,
            i,
        ))
    }

    fn block_value(
        &mut self,
        i: usize,
        end: usize,
        value_start: usize,
        indent: usize,
        depth: usize,
        allow_indentless_sequence: bool,
    ) -> Result<(Node, usize), CoreError> {
        let l = self.lines[i];
        let value_end = comment_end(self.bytes, value_start, l.end);
        if value_start >= value_end {
            let next = self.skip(i + 1, end);
            if next < end
                && (self.lines[next].indent > indent
                    || (allow_indentless_sequence
                        && self.lines[next].indent == indent
                        && self.sequence_line(next)))
            {
                return self.block(next, end, self.lines[next].indent, depth + 1);
            }
            return Ok((
                self.make(
                    Span::new(value_start, value_start),
                    NodeKind::Scalar(ScalarStyle::Empty),
                    false,
                    indent,
                    depth,
                )?,
                i + 1,
            ));
        }
        if matches!(self.bytes[value_start], b'|' | b'>') {
            let marker = &self.text[value_start..value_end];
            if !marker[1..]
                .bytes()
                .all(|c| c == b'+' || c == b'-' || (b'1'..=b'9').contains(&c))
            {
                return Err(self.error(
                    "block_scalar",
                    "invalid block scalar chomping/indent indicator",
                    value_start,
                ));
            }
            let mut next = i + 1;
            while next < end && (self.trivia(next) || self.lines[next].indent > indent) {
                next += 1;
            }
            let scalar_end = if next > i + 1 {
                self.lines[next - 1].full_end
            } else {
                l.full_end
            };
            let style = if self.bytes[value_start] == b'|' {
                ScalarStyle::Literal
            } else {
                ScalarStyle::Folded
            };
            return Ok((
                self.make(
                    Span::new(value_start, scalar_end),
                    NodeKind::Scalar(style),
                    false,
                    indent,
                    depth,
                )?,
                next,
            ));
        }
        if matches!(self.bytes[value_start], b'{' | b'[' | b'\'' | b'"') {
            let max = if end < self.lines.len() {
                self.lines[end].start
            } else {
                self.bytes.len()
            };
            let mut pos = value_start;
            let node = self.flow_value(&mut pos, max, indent, depth + 1)?;
            let mut last = i;
            while last + 1 < end && pos > self.lines[last].full_end {
                last += 1;
            }
            if pos == self.lines[last].full_end && last + 1 < end {
                last += 1;
            }
            let rest_start = skip_spaces(self.bytes, pos, self.lines[last].end);
            if rest_start < self.lines[last].end && self.bytes[rest_start] != b'#' {
                return Err(self.error(
                    "trailing_content",
                    "unexpected content after a quoted/flow value",
                    rest_start,
                ));
            }
            return Ok((node, last + 1));
        }
        if matches!(self.bytes[value_start], b'&' | b'*' | b'!') {
            return Err(self.error(
                "unsupported_alias_tag",
                "only Unity document tags/anchors are supported",
                value_start,
            ));
        }
        // Unity emits wrapped plain strings and packed hexadecimal arrays.
        let mut next = i + 1;
        while next < end && !self.trivia(next) && self.lines[next].indent > indent {
            if find_colon(
                self.bytes,
                self.lines[next].start + self.lines[next].indent,
                self.lines[next].end,
            )
            .is_some()
            {
                return Err(self.error(
                    "plain_continuation",
                    "mapping content cannot continue an unquoted scalar",
                    self.lines[next].start,
                ));
            }
            next += 1;
        }
        let scalar_end = if next > i + 1 {
            self.lines[next - 1].end
        } else {
            value_end
        };
        let style = scalar_style(&self.text[value_start..scalar_end]);
        Ok((
            self.make(
                Span::new(value_start, scalar_end),
                NodeKind::Scalar(style),
                false,
                indent,
                depth,
            )?,
            next,
        ))
    }

    fn flow_value(
        &mut self,
        pos: &mut usize,
        end: usize,
        indent: usize,
        depth: usize,
    ) -> Result<Node, CoreError> {
        if depth > self.limits.max_depth {
            return Err(self.error("depth_limit", "asset exceeds parser nesting budget", *pos));
        }
        flow_ws(self.bytes, pos, end);
        let start = *pos;
        if start >= end {
            return Err(self.error("unterminated_flow", "unexpected end of value", start));
        }
        match self.bytes[start] {
            b'{' => {
                *pos += 1;
                let mut entries = Vec::new();
                let mut keys = HashSet::new();
                loop {
                    flow_ws(self.bytes, pos, end);
                    if *pos >= end {
                        return Err(self.error("unterminated_flow", "missing '}'", start));
                    }
                    if self.bytes[*pos] == b'}' {
                        *pos += 1;
                        break;
                    }
                    let key_start = *pos;
                    if matches!(self.bytes[*pos], b'\'' | b'"') {
                        self.quoted(pos, end)?;
                    } else {
                        while *pos < end
                            && !matches!(self.bytes[*pos], b':' | b',' | b'}' | b'\n' | b'\r')
                        {
                            *pos += 1;
                        }
                    }
                    let key_end = trim_end(self.bytes, key_start, *pos);
                    flow_ws(self.bytes, pos, end);
                    if *pos >= end || self.bytes[*pos] != b':' {
                        return Err(self.error(
                            "flow_key",
                            "expected ':' after flow mapping key",
                            *pos,
                        ));
                    }
                    let key = decode_key(&self.text[key_start..key_end])
                        .map_err(|m| self.error("flow_key", m, key_start))?;
                    if key == "<<" {
                        return Err(self.error(
                            "yaml_merge_key",
                            "YAML merge keys are unsupported",
                            key_start,
                        ));
                    }
                    if !keys.insert(key.clone()) {
                        self.diagnostic(
                            "duplicate_key",
                            format!("duplicate mapping key {key}"),
                            Span::new(key_start, key_end),
                        );
                    }
                    *pos += 1;
                    flow_ws(self.bytes, pos, end);
                    let value = if *pos < end && matches!(self.bytes[*pos], b',' | b'}') {
                        self.make(
                            Span::new(*pos, *pos),
                            NodeKind::Scalar(ScalarStyle::Empty),
                            true,
                            indent,
                            depth,
                        )?
                    } else {
                        self.flow_value(pos, end, indent, depth + 1)?
                    };
                    entries.push(Entry {
                        key,
                        key_span: Span::new(key_start, key_end),
                        span: Span::new(key_start, *pos),
                        value,
                    });
                    flow_ws(self.bytes, pos, end);
                    if *pos >= end {
                        return Err(self.error("unterminated_flow", "missing '}'", start));
                    }
                    match self.bytes[*pos] {
                        b',' => *pos += 1,
                        b'}' => {
                            *pos += 1;
                            break;
                        }
                        _ => return Err(self.error("flow_separator", "expected ',' or '}'", *pos)),
                    }
                }
                self.make(
                    Span::new(start, *pos),
                    NodeKind::Mapping(entries),
                    true,
                    indent,
                    depth,
                )
            }
            b'[' => {
                *pos += 1;
                let mut items = Vec::new();
                loop {
                    flow_ws(self.bytes, pos, end);
                    if *pos >= end {
                        return Err(self.error("unterminated_flow", "missing ']'", start));
                    }
                    if self.bytes[*pos] == b']' {
                        *pos += 1;
                        break;
                    }
                    let value = self.flow_value(pos, end, indent, depth + 1)?;
                    items.push(Item {
                        span: value.span,
                        value,
                    });
                    flow_ws(self.bytes, pos, end);
                    if *pos >= end {
                        return Err(self.error("unterminated_flow", "missing ']'", start));
                    }
                    match self.bytes[*pos] {
                        b',' => *pos += 1,
                        b']' => {
                            *pos += 1;
                            break;
                        }
                        _ => return Err(self.error("flow_separator", "expected ',' or ']'", *pos)),
                    }
                }
                self.make(
                    Span::new(start, *pos),
                    NodeKind::Sequence(items),
                    true,
                    indent,
                    depth,
                )
            }
            b'\'' | b'"' => {
                let style = if self.bytes[start] == b'\'' {
                    ScalarStyle::SingleQuoted
                } else {
                    ScalarStyle::DoubleQuoted
                };
                self.quoted(pos, end)?;
                self.make(
                    Span::new(start, *pos),
                    NodeKind::Scalar(style),
                    true,
                    indent,
                    depth,
                )
            }
            b'&' | b'*' | b'!' => Err(self.error(
                "unsupported_alias_tag",
                "YAML aliases/custom tags are unsupported",
                start,
            )),
            _ => {
                while *pos < end && !matches!(self.bytes[*pos], b',' | b'}' | b']') {
                    if matches!(self.bytes[*pos], b'{' | b'[')
                        || (self.bytes[*pos] == b':'
                            && self
                                .bytes
                                .get(*pos + 1)
                                .is_some_and(u8::is_ascii_whitespace))
                    {
                        return Err(self.error("flow_plain_scalar","unquoted flow scalar contains a collection delimiter or mapping separator",*pos));
                    }
                    if self.bytes[*pos] == b'#'
                        && (*pos == start || self.bytes[*pos - 1].is_ascii_whitespace())
                    {
                        break;
                    }
                    *pos += 1;
                }
                let scalar_end = trim_end(self.bytes, start, *pos);
                if scalar_end == start {
                    return Err(self.error(
                        "empty_flow_value",
                        "missing flow sequence value",
                        start,
                    ));
                }
                self.make(
                    Span::new(start, scalar_end),
                    NodeKind::Scalar(scalar_style(&self.text[start..scalar_end])),
                    true,
                    indent,
                    depth,
                )
            }
        }
    }

    fn quoted(&self, pos: &mut usize, end: usize) -> Result<(), CoreError> {
        let start = *pos;
        let quote = self.bytes[*pos];
        *pos += 1;
        while *pos < end {
            let b = self.bytes[*pos];
            *pos += 1;
            if quote == b'"' && b == b'\\' {
                if *pos >= end {
                    break;
                }
                let escape = self.bytes[*pos];
                if !b"0abtnvfre /\\\"N_LPuxU\r\n".contains(&escape) {
                    return Err(self.error(
                        "quoted_escape",
                        "invalid YAML double-quoted escape",
                        *pos,
                    ));
                }
                *pos += 1;
                let digits = match escape {
                    b'x' => 2,
                    b'u' => 4,
                    b'U' => 8,
                    _ => 0,
                };
                if digits > 0 {
                    if *pos + digits > end
                        || !self.bytes[*pos..*pos + digits]
                            .iter()
                            .all(u8::is_ascii_hexdigit)
                    {
                        return Err(self.error("quoted_escape", "invalid Unicode escape", *pos));
                    }
                    *pos += digits;
                }
            } else if b == quote {
                if quote == b'\'' && *pos < end && self.bytes[*pos] == b'\'' {
                    *pos += 1;
                    continue;
                }
                return Ok(());
            }
        }
        Err(self.error("unterminated_quote", "quoted scalar is not closed", start))
    }
}

fn scalar_style(s: &str) -> ScalarStyle {
    if s.is_empty() {
        ScalarStyle::Empty
    } else if matches!(s, "null" | "Null" | "NULL" | "~") {
        ScalarStyle::Null
    } else {
        ScalarStyle::Plain
    }
}

fn skip_spaces(bytes: &[u8], mut pos: usize, end: usize) -> usize {
    while pos < end && matches!(bytes[pos], b' ' | b'\t') {
        pos += 1;
    }
    pos
}
fn trim_end(bytes: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}
fn flow_ws(bytes: &[u8], pos: &mut usize, end: usize) {
    loop {
        while *pos < end && bytes[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if *pos < end && bytes[*pos] == b'#' {
            while *pos < end && bytes[*pos] != b'\n' {
                *pos += 1;
            }
        } else {
            break;
        }
    }
}

fn find_colon(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut quote = 0;
    let mut nesting = 0i32;
    let mut i = start;
    while i < end {
        let b = bytes[i];
        if quote != 0 {
            if quote == b'"' && b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                if quote == b'\'' && i + 1 < end && bytes[i + 1] == quote {
                    i += 2;
                    continue;
                }
                quote = 0;
            }
        } else {
            match b {
                b'\'' | b'"' => quote = b,
                b'{' | b'[' => nesting += 1,
                b'}' | b']' => nesting -= 1,
                b':' if nesting == 0 && (i + 1 == end || bytes[i + 1].is_ascii_whitespace()) => {
                    return Some(i)
                }
                b'#' if i == start || bytes[i - 1].is_ascii_whitespace() => break,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn comment_end(bytes: &[u8], start: usize, end: usize) -> usize {
    // Flow/quoted parsers own their comments and delimiters.
    if matches!(
        bytes.get(start),
        Some(b'{') | Some(b'[') | Some(b'\'') | Some(b'"')
    ) {
        return trim_end(bytes, start, end);
    }
    let stop = (start..end)
        .find(|i| bytes[*i] == b'#' && (*i == start || bytes[*i - 1].is_ascii_whitespace()))
        .unwrap_or(end);
    trim_end(bytes, start, stop)
}

fn decode_key(raw: &str) -> Result<String, &'static str> {
    if raw.is_empty() {
        return Err("empty mapping key");
    }
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw).map_err(|_| "unsupported quoted key escape");
    }
    if raw.starts_with('\'') {
        if !raw.ends_with('\'') || raw.len() < 2 {
            return Err("unterminated quoted key");
        }
        return Ok(raw[1..raw.len() - 1].replace("''", "'"));
    }
    if raw.starts_with(['?', '[', '{', '!', '&', '*']) {
        return Err("complex mapping keys are outside the Unity dialect");
    }
    Ok(raw.to_owned())
}
