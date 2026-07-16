//! Marker protocol for native lazy tool loading (Anthropic `tool_reference`).
//!
//! When the Anthropic native renderer is active, tool results that activate
//! deferred tools (`tool_load`, `knowledge_read` on a Skill document) carry a
//! single trailing marker line naming the activated tools. The marker lives in
//! the persisted tool-message content, so it replays byte-stable across
//! requests and sessions:
//!
//! - the Anthropic serializer strips the line and emits real
//!   `tool_reference` content blocks (filtered against the tools declared in
//!   the current request, so a deleted tool can never produce a
//!   `Tool reference not found` 400);
//! - session restore re-derives the deferred declaration set by scanning
//!   history for markers;
//! - every other backend sees one short inert text line (fallback tolerated).

pub const TOOL_REFERENCE_MARKER_OPEN: &str = "<locus-tool-references>";
pub const TOOL_REFERENCE_MARKER_CLOSE: &str = "</locus-tool-references>";

/// Appends the marker line for `tool_names` to `output`. Names are joined
/// verbatim; callers pass canonical tool names. No-op for an empty list.
pub fn append_tool_reference_marker(output: &mut String, tool_names: &[String]) {
    if tool_names.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(TOOL_REFERENCE_MARKER_OPEN);
    output.push_str(&tool_names.join(", "));
    output.push_str(TOOL_REFERENCE_MARKER_CLOSE);
}

/// Splits tool-result text into (text without marker lines, referenced tool
/// names). Marker lines are recognized anywhere in the content — output that
/// merely mentions the marker inline (not as a full line) is left untouched.
pub fn split_tool_reference_marker(content: &str) -> (String, Vec<String>) {
    if !content.contains(TOOL_REFERENCE_MARKER_OPEN) {
        return (content.to_string(), Vec::new());
    }

    let mut names: Vec<String> = Vec::new();
    let mut kept_lines: Vec<&str> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let parsed = trimmed
            .strip_prefix(TOOL_REFERENCE_MARKER_OPEN)
            .and_then(|rest| rest.strip_suffix(TOOL_REFERENCE_MARKER_CLOSE));
        if let Some(list) = parsed {
            for name in list.split(',') {
                let name = name.trim();
                if !name.is_empty() && !names.iter().any(|existing| existing == name) {
                    names.push(name.to_string());
                }
            }
        } else {
            kept_lines.push(line);
        }
    }

    if names.is_empty() {
        return (content.to_string(), Vec::new());
    }

    let mut text = kept_lines.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    (text, names)
}

/// Referenced tool names in `content`, if any. Used by session restore.
pub fn parse_tool_reference_marker(content: &str) -> Vec<String> {
    split_tool_reference_marker(content).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_parse_roundtrip() {
        let mut output = "loaded skill".to_string();
        append_tool_reference_marker(
            &mut output,
            &["pdf_export".to_string(), "sheet".to_string()],
        );
        assert_eq!(
            output,
            "loaded skill\n\n<locus-tool-references>pdf_export, sheet</locus-tool-references>"
        );

        let (text, names) = split_tool_reference_marker(&output);
        assert_eq!(text, "loaded skill");
        assert_eq!(names, vec!["pdf_export".to_string(), "sheet".to_string()]);
    }

    #[test]
    fn append_skips_empty_list() {
        let mut output = "unchanged".to_string();
        append_tool_reference_marker(&mut output, &[]);
        assert_eq!(output, "unchanged");
    }

    #[test]
    fn split_returns_content_verbatim_without_marker() {
        let (text, names) = split_tool_reference_marker("plain output\nwith lines");
        assert_eq!(text, "plain output\nwith lines");
        assert!(names.is_empty());
    }

    #[test]
    fn split_ignores_inline_mention_and_dedupes() {
        let content = format!(
            "sample mentions {TOOL_REFERENCE_MARKER_OPEN} inline\n{TOOL_REFERENCE_MARKER_OPEN}a, b, a{TOOL_REFERENCE_MARKER_CLOSE}"
        );
        let (text, names) = split_tool_reference_marker(&content);
        assert_eq!(text, format!("sample mentions {TOOL_REFERENCE_MARKER_OPEN} inline"));
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn split_collects_markers_from_multiple_lines() {
        let content = format!(
            "first\n{TOOL_REFERENCE_MARKER_OPEN}a{TOOL_REFERENCE_MARKER_CLOSE}\nsecond\n{TOOL_REFERENCE_MARKER_OPEN}b{TOOL_REFERENCE_MARKER_CLOSE}"
        );
        let (text, names) = split_tool_reference_marker(&content);
        assert_eq!(text, "first\nsecond");
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }
}
