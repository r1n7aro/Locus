//! Every type a C# source file declares — not just the file-stem primary type.
//!
//! `parse_cs_script` answers Unity's `MonoBehaviour` / `ScriptableObject`
//! binding question (one type per file, named after the file). Managed
//! references (`[SerializeReference]`) are different: any `[Serializable]`
//! class can be a serialized instance, files routinely declare several, and
//! nested types serialize as `Outer/Inner`. This module enumerates all
//! class / struct / interface declarations with the data needed to match a
//! serialized `{class, ns, asm}` identity and walk inheritance:
//! nesting path, namespace, generic arity, the full base list,
//! `[MovedFrom]` legacy identity and the serialized fields.

use tree_sitter::{Node, Parser};

use super::parser::{collect_modifiers, collect_serialized_fields, unquote_csharp_string};
use super::ScriptFieldMeta;

/// A type reference as written in a base list, generic arguments dropped:
/// `global::Game.Base<int>` → `segments = ["Game", "Base"], arity = 1`.
/// The qualifier may be a namespace or an outer type — syntax alone cannot
/// tell, so resolvers try both readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsTypeRef {
    pub segments: Vec<String>,
    pub arity: usize,
}

impl CsTypeRef {
    pub fn simple_name(&self) -> &str {
        self.segments.last().map(String::as_str).unwrap_or("")
    }
}

/// `[MovedFrom(autoUpdateAPI, sourceNamespace, sourceAssembly, sourceClassName)]`.
/// `None` fields mean "unchanged" — the current value applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovedFromInfo {
    pub source_namespace: Option<String>,
    pub source_assembly: Option<String>,
    pub source_class_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredTypeKind {
    Class,
    Struct,
    Interface,
}

#[derive(Debug, Clone)]
pub struct DeclaredCsType {
    /// Nesting chain, outermost first (`["Outer", "Inner"]`).
    pub nested_path: Vec<String>,
    /// Dotted namespace; empty for the global namespace.
    pub namespace: String,
    pub kind: DeclaredTypeKind,
    /// Generic arity of the innermost type (`Box<T>` → 1).
    pub arity: usize,
    /// Generic type-parameter names, in order (`Box<TKey, TValue>`).
    pub type_params: Vec<String>,
    pub is_abstract: bool,
    /// Carries `[Serializable]` — required for Unity to serialize a plain
    /// class / struct as a field value.
    pub is_serializable: bool,
    /// Every base-list entry: base class and interfaces, in source order.
    pub bases: Vec<CsTypeRef>,
    pub moved_from: Option<MovedFromInfo>,
    /// Serialized fields declared directly on this type (not inherited).
    pub serialized_fields: Vec<ScriptFieldMeta>,
    /// 1-based line of the declaration.
    pub line: u32,
}

impl DeclaredCsType {
    pub fn name(&self) -> &str {
        self.nested_path.last().map(String::as_str).unwrap_or("")
    }

    /// Unity's serialized class string without generic arguments:
    /// `Outer/Inner`, generic arity shown as `` `N``.
    pub fn serialized_class_path(&self) -> String {
        let mut path = self.nested_path.join("/");
        if self.arity > 0 {
            path.push_str(&format!("`{}", self.arity));
        }
        path
    }

    /// `Namespace.Outer.Inner` for display.
    pub fn display_name(&self) -> String {
        let nested = self.nested_path.join(".");
        if self.namespace.is_empty() {
            nested
        } else {
            format!("{}.{}", self.namespace, nested)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CsFileTypes {
    pub types: Vec<DeclaredCsType>,
    /// Namespaces imported by plain `using X.Y;` directives (no aliases,
    /// no `using static`), anywhere in the file.
    pub usings: Vec<String>,
}

const TYPE_DECL_KINDS: &[(&str, DeclaredTypeKind)] = &[
    ("class_declaration", DeclaredTypeKind::Class),
    ("struct_declaration", DeclaredTypeKind::Struct),
    ("interface_declaration", DeclaredTypeKind::Interface),
];

/// Enumerate every class / struct / interface declared in `content`,
/// including nested types. Never fails: unparsable regions are skipped.
pub fn parse_cs_declared_types(content: &str) -> CsFileTypes {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .is_err()
    {
        return CsFileTypes::default();
    }
    let Some(tree) = parser.parse(content, None) else {
        return CsFileTypes::default();
    };
    let source = content.as_bytes();
    let mut out = CsFileTypes::default();
    walk_scope(source, tree.root_node(), "", &mut out);
    out.usings.sort();
    out.usings.dedup();
    out
}

fn walk_scope(source: &[u8], node: Node, namespace: &str, out: &mut CsFileTypes) {
    let mut file_scoped_ns: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let current_ns = file_scoped_ns.as_deref().unwrap_or(namespace);
        let kind = child.kind();
        match kind {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(compact_whitespace)
                    .unwrap_or_default();
                let new_ns = match (current_ns.is_empty(), name.is_empty()) {
                    (_, true) => current_ns.to_string(),
                    (true, false) => name,
                    (false, false) => format!("{current_ns}.{name}"),
                };
                if kind == "file_scoped_namespace_declaration" {
                    file_scoped_ns = Some(new_ns);
                } else {
                    walk_scope(source, child, &new_ns, out);
                }
            }
            "using_directive" => {
                if let Some(using) = plain_using_namespace(source, child) {
                    out.usings.push(using);
                }
            }
            _ => {
                if let Some((_, decl_kind)) = TYPE_DECL_KINDS.iter().find(|(k, _)| *k == kind) {
                    collect_type(source, child, *decl_kind, current_ns, &[], out);
                } else {
                    // `preproc_if`, `declaration_list`, `global_statement`, …
                    // tree-sitter does no real preprocessing, so declarations
                    // behind `#if` are still visible here.
                    walk_scope(source, child, current_ns, out);
                }
            }
        }
    }
}

fn collect_type(
    source: &[u8],
    node: Node,
    kind: DeclaredTypeKind,
    namespace: &str,
    outer: &[String],
    out: &mut CsFileTypes,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(name) = name_node.utf8_text(source).map(str::to_string) else {
        return;
    };
    let mut nested_path = outer.to_vec();
    nested_path.push(name);

    let mut type_params: Vec<String> = Vec::new();
    let mut bases = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_parameter_list" => {
                let mut tp = child.walk();
                type_params = child
                    .named_children(&mut tp)
                    .filter(|n| n.kind() == "type_parameter")
                    .filter_map(|n| {
                        let text = n.utf8_text(source).ok()?;
                        // `in T` / `out T` / `[Attr] T`: the name is last.
                        text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .filter(|part| !part.is_empty())
                            .next_back()
                            .map(str::to_string)
                    })
                    .collect();
            }
            "base_list" => bases = collect_bases(source, child),
            _ => {}
        }
    }
    let arity = type_params.len();

    let modifiers = collect_modifiers(source, node);
    let body = node.child_by_field_name("body");
    let mut serialized_fields = Vec::new();
    if let Some(body) = body {
        collect_serialized_fields(source, body, &mut serialized_fields);
    }

    out.types.push(DeclaredCsType {
        nested_path: nested_path.clone(),
        namespace: namespace.to_string(),
        kind,
        arity,
        type_params,
        is_abstract: modifiers.iter().any(|m| m == "abstract"),
        is_serializable: has_attribute(source, node, "Serializable"),
        bases,
        moved_from: moved_from_attribute(source, node),
        serialized_fields,
        // The name, not the node start: attribute lists belong to the node.
        line: name_node.start_position().row as u32 + 1,
    });

    if let Some(body) = body {
        collect_nested(source, body, namespace, &nested_path, out);
    }
}

/// Nested declarations live directly in the body (or behind `#if` blocks).
fn collect_nested(
    source: &[u8],
    node: Node,
    namespace: &str,
    outer: &[String],
    out: &mut CsFileTypes,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if let Some((_, decl_kind)) = TYPE_DECL_KINDS.iter().find(|(k, _)| *k == kind) {
            collect_type(source, child, *decl_kind, namespace, outer, out);
        } else if kind.starts_with("preproc_") {
            collect_nested(source, child, namespace, outer, out);
        }
    }
}

fn collect_bases(source: &[u8], base_list: Node) -> Vec<CsTypeRef> {
    let mut out = Vec::new();
    let mut cursor = base_list.walk();
    for child in base_list.named_children(&mut cursor) {
        let type_node = match child.kind() {
            "argument_list" | "comment" => continue,
            // `class Foo(int x) : Base(x)` — the base type is its first child.
            "primary_constructor_base_type" => match child.named_child(0) {
                Some(inner) => inner,
                None => continue,
            },
            _ => child,
        };
        if let Ok(text) = type_node.utf8_text(source) {
            if let Some(type_ref) = parse_type_ref(text) {
                out.push(type_ref);
            }
        }
    }
    out
}

/// Parse type text such as `global::Game.Outer<int>.Base<T, U>` into
/// segments (generic arguments dropped) plus the innermost arity.
pub fn parse_type_ref(text: &str) -> Option<CsTypeRef> {
    let text = compact_whitespace(text);
    let text = match text.find("::") {
        Some(index) => text[index + 2..].to_string(),
        None => text,
    };
    let text = text.trim_end_matches('?');

    let mut segments = Vec::new();
    let mut arities = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut commas_at_depth_one = 0usize;
    let mut saw_args = false;
    for ch in text.chars() {
        match ch {
            '<' => {
                depth += 1;
                if depth == 1 {
                    saw_args = true;
                    commas_at_depth_one = 0;
                }
            }
            '>' => depth -= 1,
            ',' if depth == 1 => commas_at_depth_one += 1,
            '.' if depth == 0 => {
                segments.push(std::mem::take(&mut current));
                arities.push(if saw_args { commas_at_depth_one + 1 } else { 0 });
                saw_args = false;
            }
            c if depth == 0 => current.push(c),
            _ => {}
        }
    }
    segments.push(current);
    arities.push(if saw_args { commas_at_depth_one + 1 } else { 0 });

    let segments: Vec<String> = segments.into_iter().map(|s| s.trim().to_string()).collect();
    if segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(CsTypeRef {
        segments,
        arity: arities.last().copied().unwrap_or(0),
    })
}

fn compact_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn plain_using_namespace(source: &[u8], node: Node) -> Option<String> {
    // `using A = B;` (alias) and `using static X;` are not namespace imports.
    if node.child_by_field_name("name").is_some() {
        return None;
    }
    let mut cursor = node.walk();
    let mut namespace = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "static" => return None,
            "identifier" | "qualified_name" | "alias_qualified_name" => {
                namespace = child.utf8_text(source).ok().map(compact_whitespace);
            }
            _ => {}
        }
    }
    let namespace = namespace?;
    Some(match namespace.find("::") {
        Some(index) => namespace[index + 2..].to_string(),
        None => namespace,
    })
}

/// `[MovedFrom(...)]` on the type declaration. Supports both constructors —
/// `(bool autoUpdateAPI, string sourceNamespace = null, string sourceAssembly
/// = null, string sourceClassName = null)` and `(string sourceNamespace)` —
/// with positional or named arguments.
fn moved_from_attribute(source: &[u8], node: Node) -> Option<MovedFromInfo> {
    let mut cursor = node.walk();
    for list in node.children(&mut cursor) {
        if list.kind() != "attribute_list" {
            continue;
        }
        let mut attr_cursor = list.walk();
        for attr in list.children(&mut attr_cursor) {
            if attr.kind() != "attribute" {
                continue;
            }
            let name = attr
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or("");
            let short = name.rsplit('.').next().unwrap_or(name);
            if short.strip_suffix("Attribute").unwrap_or(short) != "MovedFrom" {
                continue;
            }
            return Some(parse_moved_from_args(source, attr));
        }
    }
    None
}

enum AttrValue {
    Str(String),
    Null,
    Bool,
    Other,
}

fn attr_value(source: &[u8], expr: Node) -> AttrValue {
    match expr.kind() {
        "string_literal" | "verbatim_string_literal" => expr
            .utf8_text(source)
            .map(|text| AttrValue::Str(unquote_csharp_string(text)))
            .unwrap_or(AttrValue::Other),
        "null_literal" => AttrValue::Null,
        "boolean_literal" => AttrValue::Bool,
        _ => AttrValue::Other,
    }
}

fn parse_moved_from_args(source: &[u8], attr: Node) -> MovedFromInfo {
    let mut info = MovedFromInfo::default();
    let mut cursor = attr.walk();
    let Some(args) = attr
        .children(&mut cursor)
        .find(|c| c.kind() == "attribute_argument_list")
    else {
        return info;
    };

    let mut positional: Vec<AttrValue> = Vec::new();
    let mut arg_cursor = args.walk();
    for arg in args.named_children(&mut arg_cursor) {
        if arg.kind() != "attribute_argument" {
            continue;
        }
        let name_node = arg.child_by_field_name("name");
        let mut value_cursor = arg.walk();
        let Some(expr) = arg
            .named_children(&mut value_cursor)
            .find(|child| Some(child.id()) != name_node.map(|n| n.id()))
        else {
            continue;
        };
        let value = attr_value(source, expr);
        match name_node.and_then(|n| n.utf8_text(source).ok()) {
            Some(name) => assign_moved_from(&mut info, name, value),
            None => positional.push(value),
        }
    }

    // The single-string overload is `MovedFrom(string sourceNamespace)`.
    let names: &[&str] = match positional.first() {
        Some(AttrValue::Str(_)) => &["sourceNamespace"],
        _ => &[
            "autoUpdateAPI",
            "sourceNamespace",
            "sourceAssembly",
            "sourceClassName",
        ],
    };
    for (value, name) in positional.into_iter().zip(names.iter()) {
        assign_moved_from(&mut info, name, value);
    }
    info
}

fn assign_moved_from(info: &mut MovedFromInfo, name: &str, value: AttrValue) {
    let text = match value {
        AttrValue::Str(text) if !text.is_empty() => Some(text),
        AttrValue::Str(_) | AttrValue::Null | AttrValue::Bool | AttrValue::Other => None,
    };
    match name {
        "sourceNamespace" => info.source_namespace = text,
        "sourceAssembly" => info.source_assembly = text,
        "sourceClassName" => info.source_class_name = text,
        _ => {}
    }
}

/// Whether the declaration carries `[Name]` / `[NameAttribute]` (any
/// namespace qualification).
fn has_attribute(source: &[u8], node: Node, name: &str) -> bool {
    let mut cursor = node.walk();
    for list in node.children(&mut cursor) {
        if list.kind() != "attribute_list" {
            continue;
        }
        let mut attr_cursor = list.walk();
        for attr in list.children(&mut attr_cursor) {
            if attr.kind() != "attribute" {
                continue;
            }
            let text = attr
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or("");
            let short = text.rsplit(['.', ':']).next().unwrap_or(text);
            if short.strip_suffix("Attribute").unwrap_or(short) == name {
                return true;
            }
        }
    }
    false
}

/// Structure of a field's declared type: `List<Game.Range>[]` →
/// `{ name: List/1, args: [{ name: Game.Range }], array_rank: 1 }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldTypeShape {
    /// Qualified name segments with `arity` = number of generic arguments.
    pub name: CsTypeRef,
    pub args: Vec<FieldTypeShape>,
    /// Number of trailing `[]` (multi-dimensional `[,]` counts once).
    pub array_rank: usize,
}

impl FieldTypeShape {
    /// The same type without its outermost array rank.
    pub fn element(&self) -> Option<FieldTypeShape> {
        (self.array_rank > 0).then(|| FieldTypeShape {
            name: self.name.clone(),
            args: self.args.clone(),
            array_rank: self.array_rank - 1,
        })
    }

    /// `List<T>` (unqualified or `System.Collections.Generic.List`).
    pub fn is_list(&self) -> bool {
        self.array_rank == 0
            && self.args.len() == 1
            && self.name.simple_name() == "List"
            && (self.name.segments.len() == 1
                || self.name.segments[..self.name.segments.len() - 1].join(".")
                    == "System.Collections.Generic")
    }
}

/// Parse a C# field type (`T`, `T[]`, `List<T>`, `global::A.B<C, D<E>>[]`,
/// `int?`). Tuples, pointers and function pointers return `None`.
pub fn parse_field_type(text: &str) -> Option<FieldTypeShape> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let compact = match compact.find("::") {
        Some(index) => compact[index + 2..].to_string(),
        None => compact,
    };
    let chars: Vec<char> = compact.chars().collect();
    let mut pos = 0usize;
    let shape = parse_shape(&chars, &mut pos)?;
    (pos == chars.len()).then_some(shape)
}

fn parse_shape(chars: &[char], pos: &mut usize) -> Option<FieldTypeShape> {
    let mut segments = Vec::new();
    let mut args = Vec::new();
    loop {
        let start = *pos;
        while *pos < chars.len() && (chars[*pos].is_alphanumeric() || chars[*pos] == '_') {
            *pos += 1;
        }
        if *pos == start {
            return None;
        }
        segments.push(chars[start..*pos].iter().collect::<String>());
        // Generic arguments attach to the segment they follow; only the
        // innermost segment's arguments describe the field's own type.
        args.clear();
        if *pos < chars.len() && chars[*pos] == '<' {
            *pos += 1;
            loop {
                args.push(parse_shape(chars, pos)?);
                match chars.get(*pos) {
                    Some(',') => *pos += 1,
                    Some('>') => {
                        *pos += 1;
                        break;
                    }
                    _ => return None,
                }
            }
        }
        if *pos < chars.len() && chars[*pos] == '.' {
            *pos += 1;
            continue;
        }
        break;
    }
    let mut array_rank = 0usize;
    loop {
        match chars.get(*pos) {
            Some('?') => *pos += 1,
            Some('[') => {
                let close = chars[*pos..].iter().position(|c| *c == ']')? + *pos;
                if chars[*pos + 1..close].iter().any(|c| *c != ',') {
                    return None;
                }
                array_rank += 1;
                *pos = close + 1;
            }
            _ => break,
        }
    }
    let arity = args.len();
    Some(FieldTypeShape {
        name: CsTypeRef { segments, arity },
        args,
        array_rank,
    })
}
