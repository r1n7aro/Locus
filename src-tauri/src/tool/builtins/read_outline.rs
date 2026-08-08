use pulldown_cmark::{Event, HeadingLevel, Options, Parser as MarkdownParser, Tag, TagEnd};
use tree_sitter::{Node, Parser as CSharpParser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadOutlineKind {
    CSharp,
    Markdown,
}

impl ReadOutlineKind {
    pub(super) fn from_path(file_path: &str) -> Option<Self> {
        let extension = std::path::Path::new(file_path)
            .extension()
            .and_then(|value| value.to_str())?;
        if extension.eq_ignore_ascii_case("cs") {
            Some(Self::CSharp)
        } else if extension.eq_ignore_ascii_case("md") {
            Some(Self::Markdown)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownHeading {
    level: u8,
    title: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CSharpSymbol {
    depth: usize,
    kind: &'static str,
    label: String,
    start_line: usize,
    end_line: usize,
}

pub(super) fn render_outline(
    file_path: &str,
    content: &str,
    kind: ReadOutlineKind,
    knowledge_l1_summary: Option<&str>,
) -> Result<String, String> {
    let mut output = match kind {
        ReadOutlineKind::CSharp => render_csharp_outline(file_path, content)?,
        ReadOutlineKind::Markdown => render_markdown_outline(file_path, content),
    };

    if let Some(summary) = knowledge_l1_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        output.push_str("\n\nL1 summary (knowledge):\n");
        for line in summary.lines() {
            output.push_str("  ");
            output.push_str(line);
            output.push('\n');
        }
        output.pop();
    }

    output.push_str("\n</outline>");
    Ok(output)
}

pub(super) fn markdown_section_text(content: &str, title: &str) -> Option<String> {
    let headings = markdown_headings(content);
    let heading = headings
        .iter()
        .find(|heading| heading.title.trim().eq_ignore_ascii_case(title.trim()))?;
    if heading.end_line <= heading.start_line {
        return None;
    }

    let lines = content.lines().collect::<Vec<_>>();
    let start = heading.start_line.min(lines.len());
    let end = heading.end_line.min(lines.len());
    let text = lines[start..end].join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn render_markdown_outline(file_path: &str, content: &str) -> String {
    let total_lines = content.lines().count();
    let headings = markdown_headings(content);
    let mut output = format!(
        "<outline>\nFile: {file_path}\nFormat: Markdown\nNotice: outline only; original file content omitted."
    );

    if headings.is_empty() {
        output.push_str(&format!(
            "\n\n(No Markdown sections found; file has {total_lines} lines.)"
        ));
        return output;
    }

    output.push_str("\n\nSections:\n");
    for heading in headings {
        let indent = "  ".repeat(heading.level.saturating_sub(1) as usize);
        let marker = "#".repeat(heading.level as usize);
        output.push_str(&format!(
            "{indent}{marker} {} {}\n",
            heading.title,
            format_line_range(heading.start_line, heading.end_line)
        ));
    }
    output.pop();
    output
}

fn markdown_headings(content: &str) -> Vec<MarkdownHeading> {
    let total_lines = content.lines().count();
    let line_starts = line_starts(content);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut headings = Vec::new();
    let mut current: Option<(u8, usize, String)> = None;

    for (event, range) in MarkdownParser::new_ext(content, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some((
                    heading_level(level),
                    byte_offset_line(&line_starts, range.start),
                    String::new(),
                ));
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => {
                if let Some((_, _, title)) = current.as_mut() {
                    title.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, title)) = current.as_mut() {
                    title.push(' ');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start_line, title)) = current.take() {
                    let title = collapse_whitespace(&title);
                    headings.push(MarkdownHeading {
                        level,
                        title: if title.is_empty() {
                            "(untitled)".to_string()
                        } else {
                            title
                        },
                        start_line,
                        end_line: total_lines,
                    });
                }
            }
            _ => {}
        }
    }

    for index in 0..headings.len() {
        let current_level = headings[index].level;
        if let Some(next) = headings[index + 1..]
            .iter()
            .find(|candidate| candidate.level <= current_level)
        {
            headings[index].end_line = next.start_line.saturating_sub(1);
        }
    }
    headings
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn byte_offset_line(line_starts: &[usize], offset: usize) -> usize {
    line_starts.partition_point(|start| *start <= offset).max(1)
}

fn render_csharp_outline(file_path: &str, content: &str) -> Result<String, String> {
    let mut parser = CSharpParser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .map_err(|error| format!("Failed to initialize C# outline parser: {error}"))?;
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| "Failed to parse C# file for outline".to_string())?;
    let mut symbols = Vec::new();
    collect_csharp_children(
        content.as_bytes(),
        tree.root_node(),
        0,
        content.lines().count(),
        &mut symbols,
    );

    let mut output = format!(
        "<outline>\nFile: {file_path}\nFormat: C#\nNotice: outline only; original file content and implementations omitted."
    );
    if tree.root_node().has_error() {
        output.push_str("\nParser note: syntax errors were recovered; the outline may be partial.");
    }

    if symbols.is_empty() {
        output.push_str("\n\n(No C# symbols found.)");
        return Ok(output);
    }

    output.push_str("\n\nSymbols:\n");
    for symbol in symbols {
        let indent = "  ".repeat(symbol.depth);
        output.push_str(&format!(
            "{indent}{} {} {}\n",
            symbol.kind,
            symbol.label,
            format_line_range(symbol.start_line, symbol.end_line)
        ));
    }
    output.pop();
    Ok(output)
}

fn collect_csharp_children(
    source: &[u8],
    node: Node<'_>,
    depth: usize,
    total_lines: usize,
    output: &mut Vec<CSharpSymbol>,
) {
    let mut active_depth = depth;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "file_scoped_namespace_declaration" {
            if let Some(name) = field_text(source, child, "name") {
                output.push(CSharpSymbol {
                    depth,
                    kind: "namespace",
                    label: name,
                    start_line: node_start_line(child),
                    end_line: total_lines.max(node_start_line(child)),
                });
                active_depth = depth + 1;
            }
            continue;
        }
        collect_csharp_node(source, child, active_depth, total_lines, output);
    }
}

fn collect_csharp_node(
    source: &[u8],
    node: Node<'_>,
    depth: usize,
    total_lines: usize,
    output: &mut Vec<CSharpSymbol>,
) {
    match node.kind() {
        "namespace_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                push_symbol(output, depth, "namespace", name, node);
                if let Some(body) = node.child_by_field_name("body") {
                    collect_csharp_children(source, body, depth + 1, total_lines, output);
                }
            }
        }
        "class_declaration"
        | "struct_declaration"
        | "interface_declaration"
        | "record_declaration"
        | "enum_declaration" => {
            let kind = match node.kind() {
                "class_declaration" => "class",
                "struct_declaration" => "struct",
                "interface_declaration" => "interface",
                "record_declaration" => "record",
                "enum_declaration" => "enum",
                _ => unreachable!(),
            };
            if let Some(mut name) = field_text(source, node, "name") {
                if let Some(type_parameters) = field_text(source, node, "type_parameters")
                    .or_else(|| direct_named_child_text(source, node, "type_parameter_list"))
                {
                    name.push_str(&type_parameters);
                }
                push_symbol(output, depth, kind, name, node);
                if let Some(body) = node.child_by_field_name("body") {
                    collect_csharp_children(source, body, depth + 1, total_lines, output);
                }
            }
        }
        "delegate_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                let returns = field_text(source, node, "type").unwrap_or_else(|| "?".to_string());
                let type_parameters =
                    field_text(source, node, "type_parameters").unwrap_or_default();
                let parameters =
                    parameter_list_text(source, node.child_by_field_name("parameters"));
                push_symbol(
                    output,
                    depth,
                    "delegate",
                    format!("{name}{type_parameters}{parameters}: {returns}"),
                    node,
                );
            }
        }
        "field_declaration" | "event_field_declaration" => {
            collect_field_symbols(source, node, depth, output);
        }
        "property_declaration" | "event_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                let name = explicit_member_name(source, node, &name);
                let value_type =
                    field_text(source, node, "type").unwrap_or_else(|| "?".to_string());
                let kind = if node.kind() == "property_declaration" {
                    "property"
                } else {
                    "event"
                };
                push_symbol(output, depth, kind, format!("{name}: {value_type}"), node);
            }
        }
        "method_declaration" | "local_function_statement" => {
            if let Some(name) = field_text(source, node, "name") {
                let name = explicit_member_name(source, node, &name);
                let returns = field_text(source, node, "returns")
                    .or_else(|| field_text(source, node, "type"))
                    .unwrap_or_else(|| "?".to_string());
                let type_parameters =
                    field_text(source, node, "type_parameters").unwrap_or_default();
                let parameters =
                    parameter_list_text(source, node.child_by_field_name("parameters"));
                push_symbol(
                    output,
                    depth,
                    if node.kind() == "method_declaration" {
                        "method"
                    } else {
                        "local function"
                    },
                    format!("{name}{type_parameters}{parameters}: {returns}"),
                    node,
                );
            }
        }
        "constructor_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                let parameters =
                    parameter_list_text(source, node.child_by_field_name("parameters"));
                push_symbol(
                    output,
                    depth,
                    "constructor",
                    format!("{name}{parameters}"),
                    node,
                );
            }
        }
        "destructor_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                push_symbol(output, depth, "destructor", format!("~{name}()"), node);
            }
        }
        "operator_declaration" => {
            let operator = field_text(source, node, "operator").unwrap_or_else(|| "?".to_string());
            let returns = field_text(source, node, "type").unwrap_or_else(|| "?".to_string());
            let parameters = parameter_list_text(source, node.child_by_field_name("parameters"));
            push_symbol(
                output,
                depth,
                "operator",
                format!("operator {operator}{parameters}: {returns}"),
                node,
            );
        }
        "conversion_operator_declaration" => {
            let target_type = field_text(source, node, "type").unwrap_or_else(|| "?".to_string());
            let parameters = parameter_list_text(source, node.child_by_field_name("parameters"));
            push_symbol(
                output,
                depth,
                "conversion operator",
                format!("operator {target_type}{parameters}"),
                node,
            );
        }
        "indexer_declaration" => {
            let value_type = field_text(source, node, "type").unwrap_or_else(|| "?".to_string());
            let parameters = parameter_list_text(source, node.child_by_field_name("parameters"));
            push_symbol(
                output,
                depth,
                "indexer",
                format!("this{parameters}: {value_type}"),
                node,
            );
        }
        "enum_member_declaration" => {
            if let Some(name) = field_text(source, node, "name") {
                push_symbol(output, depth, "enum member", name, node);
            }
        }
        // Member bodies are deliberately not traversed. Unknown wrappers such
        // as preprocessor branches and global statements may still contain
        // declarations, so descend through those at the current depth.
        "block" | "arrow_expression_clause" | "accessor_list" | "accessor_declaration" => {}
        _ => collect_csharp_children(source, node, depth, total_lines, output),
    }
}

fn collect_field_symbols(
    source: &[u8],
    node: Node<'_>,
    depth: usize,
    output: &mut Vec<CSharpSymbol>,
) {
    let Some(declaration) = direct_named_child(node, "variable_declaration") else {
        return;
    };
    let value_type = field_text(source, declaration, "type").unwrap_or_else(|| "?".to_string());
    let kind = if node.kind() == "event_field_declaration" {
        "event"
    } else {
        "field"
    };
    let mut cursor = declaration.walk();
    for declarator in declaration.named_children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        if let Some(name) = field_text(source, declarator, "name") {
            push_symbol(
                output,
                depth,
                kind,
                format!("{name}: {value_type}"),
                declarator,
            );
        }
    }
}

fn parameter_list_text(source: &[u8], node: Option<Node<'_>>) -> String {
    let Some(node) = node else {
        return "()".to_string();
    };
    let (open, close) = if node.kind() == "bracketed_parameter_list" {
        ('[', ']')
    } else {
        ('(', ')')
    };
    let mut parameters = Vec::new();
    let mut cursor = node.walk();
    for parameter in node.named_children(&mut cursor) {
        if !matches!(parameter.kind(), "parameter" | "implicit_parameter") {
            continue;
        }
        let name = field_text(source, parameter, "name").unwrap_or_else(|| "?".to_string());
        let value_type = field_text(source, parameter, "type");
        let mut modifiers = Vec::new();
        let mut parameter_cursor = parameter.walk();
        for child in parameter.named_children(&mut parameter_cursor) {
            if child.kind() == "modifier" {
                if let Some(value) = node_text(source, child) {
                    modifiers.push(value);
                }
            }
        }
        let mut label = String::new();
        if !modifiers.is_empty() {
            label.push_str(&modifiers.join(" "));
            label.push(' ');
        }
        if let Some(value_type) = value_type {
            label.push_str(&value_type);
            label.push(' ');
        }
        label.push_str(&name);
        parameters.push(label);
    }
    format!("{open}{}{close}", parameters.join(", "))
}

fn explicit_member_name(source: &[u8], node: Node<'_>, name: &str) -> String {
    direct_named_child_text(source, node, "explicit_interface_specifier")
        .map(|prefix| format!("{prefix}{name}"))
        .unwrap_or_else(|| name.to_string())
}

fn push_symbol(
    output: &mut Vec<CSharpSymbol>,
    depth: usize,
    kind: &'static str,
    label: String,
    node: Node<'_>,
) {
    output.push(CSharpSymbol {
        depth,
        kind,
        label: collapse_whitespace(&label),
        start_line: node_start_line(node),
        end_line: node_end_line(node),
    });
}

fn field_text(source: &[u8], node: Node<'_>, field: &str) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|child| node_text(source, child))
}

fn direct_named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let result = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == kind);
    result
}

fn direct_named_child_text(source: &[u8], node: Node<'_>, kind: &str) -> Option<String> {
    direct_named_child(node, kind).and_then(|child| node_text(source, child))
}

fn node_text(source: &[u8], node: Node<'_>) -> Option<String> {
    node.utf8_text(source).ok().map(collapse_whitespace)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn node_start_line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn node_end_line(node: Node<'_>) -> usize {
    node.end_position().row + 1
}

fn format_line_range(start_line: usize, end_line: usize) -> String {
    if start_line == end_line {
        format!("[line {start_line}]")
    } else {
        format!("[lines {start_line}-{end_line}]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_outline_reports_hierarchical_section_ranges() {
        let content = "# Guide\nIntro\n\n## Setup\nText\n\n### Windows\nText\n\n## Usage\nDone\n";
        let output = render_outline("guide.md", content, ReadOutlineKind::Markdown, None)
            .expect("render markdown outline");

        assert!(output.contains("# Guide [lines 1-11]"), "{output}");
        assert!(output.contains("  ## Setup [lines 4-9]"), "{output}");
        assert!(output.contains("    ### Windows [lines 7-9]"), "{output}");
        assert!(output.contains("  ## Usage [lines 10-11]"), "{output}");
        assert!(output.contains("outline only; original file content omitted"));
    }

    #[test]
    fn markdown_outline_ignores_headings_inside_code_fences_and_supports_setext() {
        let content = "Title\n=====\n\n```md\n# Hidden\n```\n\n## Visible\nText\n";
        let output = render_outline("guide.md", content, ReadOutlineKind::Markdown, None)
            .expect("render markdown outline");

        assert!(output.contains("# Title [lines 1-9]"), "{output}");
        assert!(output.contains("  ## Visible [lines 8-9]"), "{output}");
        assert!(!output.contains("Hidden"), "{output}");
    }

    #[test]
    fn markdown_section_text_extracts_only_the_requested_section() {
        let content = "# Skill\n\n## L1\nUse this skill.\n\n## Instructions\nDo work.\n";
        assert_eq!(
            markdown_section_text(content, "L1").as_deref(),
            Some("Use this skill.")
        );
    }

    #[test]
    fn csharp_outline_lists_types_members_and_signatures_without_bodies() {
        let content = r#"namespace Game
{
    public class Player<T>
    {
        private int health = 100, armor = 5;
        public string Name { get; set; }

        public Player(int health) { this.health = health; }

        public void Move(ref float speed)
        {
            speed += 1;
        }

        public struct State
        {
            public bool Alive;
        }
    }
}
"#;
        let output = render_outline("Player.cs", content, ReadOutlineKind::CSharp, None)
            .expect("render C# outline");

        assert!(output.contains("namespace Game [lines 1-20]"), "{output}");
        assert!(
            output.contains("  class Player<T> [lines 3-19]"),
            "{output}"
        );
        assert!(
            output.contains("    field health: int [line 5]"),
            "{output}"
        );
        assert!(output.contains("    field armor: int [line 5]"), "{output}");
        assert!(
            output.contains("    property Name: string [line 6]"),
            "{output}"
        );
        assert!(
            output.contains("    constructor Player(int health) [line 8]"),
            "{output}"
        );
        assert!(
            output.contains("    method Move(ref float speed): void [lines 10-13]"),
            "{output}"
        );
        assert!(
            output.contains("    struct State [lines 15-18]"),
            "{output}"
        );
        assert!(!output.contains("speed += 1"), "{output}");
        assert!(output.contains("implementations omitted"));
    }

    #[test]
    fn outline_appends_knowledge_l1_summary() {
        let output = render_outline(
            "skill.md",
            "# Skill\n",
            ReadOutlineKind::Markdown,
            Some("Use for focused audits.\nKeep results concise."),
        )
        .expect("render outline");

        assert!(output.contains(
            "L1 summary (knowledge):\n  Use for focused audits.\n  Keep results concise."
        ));
    }
}
