use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::session::models::{Citation, CitationKind};

#[derive(Debug, Clone)]
struct PendingAnnotation {
    output_index: usize,
    content_index: usize,
    annotation_index: usize,
    annotation: Value,
}

/// Collects Responses API text-part offsets and annotation events while text
/// streams. Final output items remain authoritative; event annotations cover
/// compatible endpoints that omit `output` from their terminal response.
#[derive(Debug, Default)]
pub struct CitationCollector {
    part_offsets: HashMap<(usize, usize), u32>,
    pending: Vec<PendingAnnotation>,
}

impl CitationCollector {
    pub fn observe_text_delta(&mut self, event: &Value, current_text: &str) {
        let Some(output_index) = json_usize(event.get("output_index")) else {
            return;
        };
        let content_index = json_usize(event.get("content_index")).unwrap_or(0);
        self.part_offsets
            .entry((output_index, content_index))
            .or_insert_with(|| utf16_len(current_text));
    }

    pub fn observe_annotation_event(&mut self, event: &Value) {
        let Some(annotation) = event.get("annotation").cloned() else {
            return;
        };
        self.push_pending(
            json_usize(event.get("output_index")).unwrap_or(0),
            json_usize(event.get("content_index")).unwrap_or(0),
            json_usize(event.get("annotation_index")).unwrap_or(self.pending.len()),
            annotation,
        );
    }

    pub fn observe_content_part(&mut self, event: &Value) {
        let Some(part) = event.get("part") else {
            return;
        };
        if part.get("type").and_then(Value::as_str) != Some("output_text") {
            return;
        }
        let output_index = json_usize(event.get("output_index")).unwrap_or(0);
        let content_index = json_usize(event.get("content_index")).unwrap_or(0);
        if let Some(annotations) = part.get("annotations").and_then(Value::as_array) {
            for (annotation_index, annotation) in annotations.iter().enumerate() {
                self.push_pending(
                    output_index,
                    content_index,
                    annotation_index,
                    annotation.clone(),
                );
            }
        }
    }

    pub fn observe_output_item(&mut self, event: &Value) {
        let output_index = json_usize(event.get("output_index")).unwrap_or(0);
        let Some(content) = event
            .get("item")
            .and_then(|item| item.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for (content_index, part) in content.iter().enumerate() {
            if part.get("type").and_then(Value::as_str) != Some("output_text") {
                continue;
            }
            if let Some(annotations) = part.get("annotations").and_then(Value::as_array) {
                for (annotation_index, annotation) in annotations.iter().enumerate() {
                    self.push_pending(
                        output_index,
                        content_index,
                        annotation_index,
                        annotation.clone(),
                    );
                }
            }
        }
    }

    pub fn collect(&self, response_items: &[Value], full_text: &str) -> Vec<Citation> {
        let mut collected = Vec::new();
        let mut sequential_offset = 0u32;

        for (output_index, item) in response_items.iter().enumerate() {
            let Some(content) = item.get("content").and_then(Value::as_array) else {
                continue;
            };
            for (content_index, part) in content.iter().enumerate() {
                if part.get("type").and_then(Value::as_str) != Some("output_text") {
                    continue;
                }
                let base = self
                    .part_offsets
                    .get(&(output_index, content_index))
                    .copied()
                    .unwrap_or(sequential_offset);
                if let Some(annotations) = part.get("annotations").and_then(Value::as_array) {
                    for annotation in annotations {
                        if let Some(citation) = parse_annotation(annotation, base, full_text) {
                            collected.push(citation);
                        }
                    }
                }
                sequential_offset = sequential_offset.saturating_add(
                    part.get("text")
                        .and_then(Value::as_str)
                        .map(utf16_len)
                        .unwrap_or(0),
                );
            }
        }

        for pending in &self.pending {
            let base = self
                .part_offsets
                .get(&(pending.output_index, pending.content_index))
                .copied()
                .unwrap_or(0);
            if let Some(mut citation) = parse_annotation(&pending.annotation, base, full_text) {
                if citation.id.is_empty() {
                    citation.id = format!(
                        "citation-{}-{}-{}",
                        pending.output_index, pending.content_index, pending.annotation_index
                    );
                }
                collected.push(citation);
            }
        }

        normalize_citations(collected, full_text)
    }

    fn push_pending(
        &mut self,
        output_index: usize,
        content_index: usize,
        annotation_index: usize,
        annotation: Value,
    ) {
        self.pending.push(PendingAnnotation {
            output_index,
            content_index,
            annotation_index,
            annotation,
        });
    }
}

fn parse_annotation(annotation: &Value, base: u32, full_text: &str) -> Option<Citation> {
    let annotation_type = annotation.get("type").and_then(Value::as_str).unwrap_or("");
    let url = json_string(annotation, &["url", "uri"]);
    let file_id = json_string(annotation, &["file_id", "fileId"]);
    let filename = json_string(annotation, &["filename", "file_name", "fileName"]);
    let kind = match annotation_type {
        "url_citation" => CitationKind::Url,
        "file_citation" => CitationKind::File,
        "container_file_citation" => CitationKind::ContainerFile,
        _ if url.is_some() => CitationKind::Url,
        _ if file_id.is_some() => CitationKind::File,
        _ => CitationKind::Reference,
    };

    let local_start = json_u32(annotation.get("start_index"))
        .or_else(|| json_u32(annotation.get("start")))
        .or_else(|| json_u32(annotation.get("index")));
    let local_end = json_u32(annotation.get("end_index"))
        .or_else(|| json_u32(annotation.get("end")))
        .or(local_start);
    let start_index = local_start.map(|value| base.saturating_add(value));
    let end_index = local_end.map(|value| base.saturating_add(value));

    let mut reference_ids = Vec::new();
    for key in ["reference_id", "referenceId", "ref_id", "refId"] {
        if let Some(value) = annotation.get(key).and_then(Value::as_str) {
            push_unique(&mut reference_ids, value);
        }
    }
    if let Some(values) = annotation.get("reference_ids").and_then(Value::as_array) {
        for value in values.iter().filter_map(Value::as_str) {
            push_unique(&mut reference_ids, value);
        }
    }
    if let (Some(start), Some(end)) = (start_index, end_index) {
        if let Some(slice) = utf16_slice(full_text, start, end) {
            for reference_id in citation_reference_ids(slice) {
                push_unique(&mut reference_ids, &reference_id);
            }
        }
    }

    let declares_reference = annotation_type.contains("citation")
        || matches!(annotation_type, "reference" | "source_reference");
    if kind == CitationKind::Reference && reference_ids.is_empty() && !declares_reference {
        return None;
    }

    Some(Citation {
        id: String::new(),
        kind,
        start_index,
        end_index,
        url,
        title: json_string(annotation, &["title", "name"]),
        file_id,
        filename,
        reference_ids,
    })
}

fn normalize_citations(mut citations: Vec<Citation>, full_text: &str) -> Vec<Citation> {
    citations.sort_by_key(|citation| {
        (
            citation.start_index.unwrap_or(u32::MAX),
            citation.end_index.unwrap_or(u32::MAX),
        )
    });
    let mut seen = HashSet::new();
    citations.retain(|citation| {
        let key = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            citation.kind,
            citation.start_index,
            citation.end_index,
            citation.url,
            citation.file_id,
            citation.filename,
            citation.reference_ids
        );
        seen.insert(key)
    });
    for (index, citation) in citations.iter_mut().enumerate() {
        citation.id = format!("citation-{}", index + 1);
        if citation.reference_ids.is_empty() {
            if let (Some(start), Some(end)) = (citation.start_index, citation.end_index) {
                if let Some(slice) = utf16_slice(full_text, start, end) {
                    citation.reference_ids = citation_reference_ids(slice);
                }
            }
        }
    }
    citations
}

fn citation_reference_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    collect_marker_ids(text, '\u{e200}', '\u{e202}', '\u{e201}', &mut ids);
    collect_marker_ids(text, '\u{fffd}', '\u{fffd}', '\u{fffd}', &mut ids);
    ids
}

fn collect_marker_ids(
    text: &str,
    open: char,
    separator: char,
    close: char,
    output: &mut Vec<String>,
) {
    let prefix = format!("{open}cite{separator}");
    let mut remainder = text;
    while let Some(start) = remainder.find(&prefix) {
        let after_prefix = &remainder[start + prefix.len()..];
        let Some(end) = after_prefix.find(close) else {
            break;
        };
        for value in after_prefix[..end].split(separator) {
            let value = value.trim();
            if value.starts_with("turn") {
                push_unique(output, value);
            }
        }
        remainder = &after_prefix[end + close.len_utf8()..];
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn json_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_usize(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn json_u32(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn utf16_len(value: &str) -> u32 {
    u32::try_from(value.encode_utf16().count()).unwrap_or(u32::MAX)
}

fn utf16_slice(value: &str, start: u32, end: u32) -> Option<&str> {
    if end < start {
        return None;
    }
    let mut utf16_offset = 0u32;
    let mut byte_start = None;
    let mut byte_end = None;
    for (byte_index, character) in value.char_indices() {
        if utf16_offset == start {
            byte_start = Some(byte_index);
        }
        if utf16_offset == end {
            byte_end = Some(byte_index);
            break;
        }
        utf16_offset = utf16_offset.saturating_add(character.len_utf16() as u32);
        if utf16_offset > start && byte_start.is_none() {
            return None;
        }
        if utf16_offset > end {
            return None;
        }
    }
    if utf16_offset == start && byte_start.is_none() {
        byte_start = Some(value.len());
    }
    if utf16_offset == end && byte_end.is_none() {
        byte_end = Some(value.len());
    }
    Some(&value[byte_start?..byte_end?])
}

#[cfg(test)]
mod tests {
    use super::CitationCollector;
    use crate::session::models::CitationKind;

    #[test]
    fn collects_url_and_file_citations_from_final_output_items() {
        let text = "资料\u{e200}cite\u{e202}turn1view0\u{e201} 与文件";
        let marker_start = "资料".encode_utf16().count();
        let marker_end = marker_start
            + "\u{e200}cite\u{e202}turn1view0\u{e201}"
                .encode_utf16()
                .count();
        let items = vec![serde_json::json!({
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": [
                    {
                        "type": "url_citation",
                        "start_index": marker_start,
                        "end_index": marker_end,
                        "url": "https://example.com/source",
                        "title": "Example"
                    },
                    {
                        "type": "file_citation",
                        "index": text.encode_utf16().count(),
                        "file_id": "file-1",
                        "filename": "notes.pdf"
                    }
                ]
            }]
        })];

        let citations = CitationCollector::default().collect(&items, text);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].kind, CitationKind::Url);
        assert_eq!(citations[0].reference_ids, vec!["turn1view0"]);
        assert_eq!(citations[1].kind, CitationKind::File);
        assert_eq!(citations[1].filename.as_deref(), Some("notes.pdf"));
    }

    #[test]
    fn annotation_events_use_streamed_text_part_offsets() {
        let mut collector = CitationCollector::default();
        collector.observe_text_delta(
            &serde_json::json!({"output_index": 1, "content_index": 0}),
            "前文",
        );
        collector.observe_annotation_event(&serde_json::json!({
            "output_index": 1,
            "content_index": 0,
            "annotation_index": 0,
            "annotation": {
                "type": "url_citation",
                "start_index": 0,
                "end_index": 2,
                "url": "https://example.com"
            }
        }));

        let citations = collector.collect(&[], "前文来源");
        assert_eq!(citations[0].start_index, Some(2));
        assert_eq!(citations[0].end_index, Some(4));
    }

    #[test]
    fn ignores_non_citation_text_annotations() {
        let items = vec![serde_json::json!({
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "annotated text",
                "annotations": [{
                    "type": "text_annotation",
                    "start_index": 0,
                    "end_index": 9,
                    "label": "emphasis"
                }]
            }]
        })];

        let citations = CitationCollector::default().collect(&items, "annotated text");
        assert!(citations.is_empty());
    }
}
