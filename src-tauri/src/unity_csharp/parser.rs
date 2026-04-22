use tree_sitter::{Node, Parser};

/// Metadata extracted from a C# script file.
#[derive(Debug, Clone)]
pub struct ScriptMetadata {
    pub class_name: String,
    pub base_type: Option<String>,
    pub namespace: Option<String>,
    pub serialized_fields: Vec<ScriptFieldMeta>,
}

/// Why a `.cs` source has no extractable [`ScriptMetadata`].
///
/// `Parsed` carries the metadata when extraction succeeded; the other variants
/// let the caller distinguish "the parser worked exactly as designed for an
/// expected file shape" (no warning needed) from "we should have got a class
/// here and didn't" (worth surfacing).
#[derive(Debug, Clone)]
pub enum ScriptParseStatus {
    /// A top-level class/struct/interface was extracted.
    Parsed(ScriptMetadata),
    /// Source contains no top-level declarations of any kind — empty
    /// namespace, only `using` directives, or entirely commented out.
    EmptySource,
    /// Source contains only `enum` (and/or `delegate`) declarations.
    /// Unity does not bind enums by file name, so ref_graph deliberately
    /// skips them.
    OnlyNonClassTypes,
    /// Source contains a class/struct/interface that the parser could not
    /// extract — usually a tree-sitter recovery error around an unusual
    /// preprocessor / generic / collection-initializer shape. Worth logging.
    Unparseable,
}

/// Metadata for a single serialized field in a C# script.
#[derive(Debug, Clone)]
pub struct ScriptFieldMeta {
    pub name: String,
    pub field_type: String,
    pub former_names: Vec<String>,
    pub hidden: bool,
    pub serialize_field: bool,
}

/// Parse a C# script source string and extract Unity-relevant metadata.
///
/// `expected_name` is the file stem (without `.cs`). When provided, a
/// top-level type whose name matches the file stem **case-sensitively** is
/// preferred over any other top-level type — this matches Unity's contract
/// that a `MonoBehaviour` / `ScriptableObject` script must declare a public
/// class with the same name as its containing file. The case-sensitive
/// match is intentional: although Windows file systems are case-insensitive,
/// Unity's class binding is not, so a `Foo.cs` file containing
/// `class foo : MonoBehaviour` is not a valid binding even on Windows. Pass
/// `None` only when the source has no associated path (e.g. tests, in-memory
/// snippets); in that case the parser falls back to "first public top-level
/// type", and finally to "first top-level type at all".
///
/// All `partial` declarations of the chosen type are merged so that fields
/// split across files / fragments in the same source string are reported
/// together.
///
/// Returns `None` only when the source contains no parseable class /
/// struct / record / interface declaration. Files that are entirely
/// commented out, behind `#if false`, or contain only enums / delegates /
/// extension stubs return `None`.
pub fn parse_cs_script(content: &str, expected_name: Option<&str>) -> Option<ScriptMetadata> {
    match parse_cs_script_status(content, expected_name) {
        ScriptParseStatus::Parsed(meta) => Some(meta),
        _ => None,
    }
}

/// Parse a C# script source and report extraction status, distinguishing
/// "no metadata, by design" (empty / enum-only files) from
/// "no metadata, parser failure" (real classes the parser tripped on).
///
/// Used by the ref_graph watcher to suppress noisy warnings on the first
/// two cases while still flagging the third.
pub fn parse_cs_script_status(content: &str, expected_name: Option<&str>) -> ScriptParseStatus {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .is_err()
    {
        return ScriptParseStatus::Unparseable;
    }
    let Some(tree) = parser.parse(content, None) else {
        return ScriptParseStatus::Unparseable;
    };
    let source = content.as_bytes();

    let mut top_level: Vec<TopLevelType> = Vec::new();
    collect_top_level(source, tree.root_node(), None, &mut top_level);

    let Some(chosen_name) = pick_primary_name(&top_level, expected_name) else {
        return classify_no_class_reason(tree.root_node());
    };
    let Some(primary_index) = top_level.iter().position(|t| t.name == chosen_name) else {
        return ScriptParseStatus::Unparseable;
    };
    let primary = &top_level[primary_index];

    // Merge fields from all `partial` declarations of the same type. We
    // require name + namespace + tree-sitter node kind to all match the
    // primary, so a pathological mix like `partial class Foo` and
    // `partial struct Foo` (rejected by the C# compiler but still parsed by
    // tree-sitter) doesn't get its bodies stitched together.
    let primary_kind = primary.kind;
    let primary_namespace = primary.namespace.clone();
    let matches_primary = |t: &&TopLevelType| {
        t.name == chosen_name && t.namespace == primary_namespace && t.kind == primary_kind
    };

    let mut serialized_fields = Vec::new();
    for t in top_level.iter().filter(matches_primary) {
        if let Some(body) = t.body {
            collect_serialized_fields(source, body, &mut serialized_fields);
        }
    }

    // The base class can be declared on any of the partial fragments —
    // C# only requires that no two fragments specify *different* bases.
    // The reviewer's example: `partial class Boss { ... }` (no base) +
    // `partial class Boss : MonoBehaviour { ... }` must report
    // `MonoBehaviour`, not `None`, regardless of which fragment was
    // selected as primary.
    let merged_base = top_level
        .iter()
        .filter(matches_primary)
        .find_map(|t| t.base.clone());

    ScriptParseStatus::Parsed(ScriptMetadata {
        class_name: primary.name.clone(),
        base_type: merged_base,
        namespace: primary.namespace.clone(),
        serialized_fields,
    })
}

/// Walk the syntax tree and decide why no class/struct/interface was extracted.
///
/// - If the tree contains *any* `class_declaration` / `struct_declaration` /
///   `interface_declaration` (even one tree-sitter recovered with errors),
///   `pick_primary_name` rejected it for a non-trivial reason — flag as
///   `Unparseable` so the watcher logs it.
/// - Otherwise, if the tree contains an `enum_declaration` or
///   `delegate_declaration` (the two top-level kinds Unity doesn't bind to a
///   .cs file by name), report `OnlyNonClassTypes`.
/// - Otherwise the file is genuinely empty / commented out / only `using`s /
///   only an empty namespace body — report `EmptySource`.
fn classify_no_class_reason(root: Node) -> ScriptParseStatus {
    let mut has_class_like = false;
    let mut has_non_class_type = false;
    walk_classify(root, &mut has_class_like, &mut has_non_class_type);
    if has_class_like {
        ScriptParseStatus::Unparseable
    } else if has_non_class_type {
        ScriptParseStatus::OnlyNonClassTypes
    } else {
        ScriptParseStatus::EmptySource
    }
}

fn walk_classify(node: Node, has_class_like: &mut bool, has_non_class_type: &mut bool) {
    let kind = node.kind();
    if TYPE_DECL_KINDS.contains(&kind) {
        *has_class_like = true;
        return;
    }
    if matches!(kind, "enum_declaration" | "delegate_declaration") {
        *has_non_class_type = true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_classify(child, has_class_like, has_non_class_type);
        if *has_class_like {
            return;
        }
    }
}

#[derive(Debug)]
struct TopLevelType<'tree> {
    namespace: Option<String>,
    name: String,
    base: Option<String>,
    body: Option<Node<'tree>>,
    is_public: bool,
    /// The tree-sitter node kind (`class_declaration`, `struct_declaration`,
    /// `interface_declaration`). Used to keep `partial` merging from mixing
    /// different declaration kinds with the same name.
    kind: &'static str,
}

// `record_declaration` / `record_struct_declaration` are intentionally NOT
// listed here. Unity does not serialize records, and positional records
// (`public record Foo(int X);`) have no `body` field — picking one as the
// primary type would shadow a real `MonoBehaviour` in the same file with
// zero serialized fields.
const TYPE_DECL_KINDS: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "interface_declaration",
];

fn collect_top_level<'tree>(
    source: &[u8],
    node: Node<'tree>,
    current_ns: Option<String>,
    out: &mut Vec<TopLevelType<'tree>>,
) {
    let mut file_scoped_ns: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        let inherited_ns = file_scoped_ns.clone().or_else(|| current_ns.clone());

        if kind == "namespace_declaration" || kind == "file_scoped_namespace_declaration" {
            let name_text = child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());
            let new_ns = match (inherited_ns.as_deref(), name_text.as_deref()) {
                (Some(parent), Some(c)) => Some(format!("{}.{}", parent, c)),
                (None, Some(c)) => Some(c.to_string()),
                (parent, None) => parent.map(|s| s.to_string()),
            };
            if kind == "file_scoped_namespace_declaration" {
                file_scoped_ns = new_ns;
            } else {
                collect_top_level(source, child, new_ns, out);
            }
            continue;
        }

        if TYPE_DECL_KINDS.contains(&kind) {
            if let Some(t) = build_top_level_type(source, child, inherited_ns.clone()) {
                out.push(t);
            }
            // Don't recurse into the type body — we only want top-level types,
            // not nested ones.
            continue;
        }

        // Recurse into anything else: `preproc_if` (so `#if UNITY_EDITOR`
        // wrapped declarations are visible), `declaration_list`,
        // `global_statement`, etc. Tree-sitter-c-sharp does not perform real
        // preprocessing, so the children of `preproc_if` are still parsed.
        collect_top_level(source, child, inherited_ns, out);
    }
}

fn build_top_level_type<'tree>(
    source: &[u8],
    node: Node<'tree>,
    namespace: Option<String>,
) -> Option<TopLevelType<'tree>> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())?
        .to_string();
    let base = extract_first_base_type(source, node);
    let body = node.child_by_field_name("body");
    let modifiers = collect_modifiers(source, node);
    let is_public = modifiers.iter().any(|m| m == "public");
    let kind = TYPE_DECL_KINDS
        .iter()
        .copied()
        .find(|k| *k == node.kind())?;

    Some(TopLevelType {
        namespace,
        name,
        base,
        body,
        is_public,
        kind,
    })
}

fn pick_primary_name(items: &[TopLevelType], expected_name: Option<&str>) -> Option<String> {
    if items.is_empty() {
        return None;
    }

    // Priority 1: file-stem match. This matches Unity's binding contract
    // (public MonoBehaviour / ScriptableObject must live in a same-named .cs
    // file). Without this, multi-class files index the wrong GUID → wrong
    // class_name in the ref graph.
    if let Some(expected) = expected_name {
        if let Some(t) = items.iter().find(|t| t.name == expected) {
            return Some(t.name.clone());
        }
    }

    // Priority 2: first public top-level type.
    if let Some(t) = items.iter().find(|t| t.is_public) {
        return Some(t.name.clone());
    }

    // Fallback: first top-level type at all.
    Some(items[0].name.clone())
}

fn extract_first_base_type(source: &[u8], node: Node) -> Option<String> {
    let mut cursor = node.walk();
    let base_list = node
        .children(&mut cursor)
        .find(|c| c.kind() == "base_list")?;
    let mut c2 = base_list.walk();
    let mut first_non_interface: Option<String> = None;
    let mut first_any: Option<String> = None;
    for child in base_list.named_children(&mut c2) {
        // Comments are tree-sitter "extras" and appear as named children of
        // any node they syntactically fall inside, including base_list. Skip
        // them so a `: Foo /* old base */, IBar` doesn't return the comment
        // text as a base type. We also restrict to the kinds tree-sitter-
        // c-sharp actually emits for base list entries — anything else
        // (whitespace nodes, error recovery sentinels) is silently ignored.
        match child.kind() {
            "identifier"
            | "generic_name"
            | "qualified_name"
            | "predefined_type"
            | "alias_qualified_name" => {}
            _ => continue,
        }
        let Ok(text) = child.utf8_text(source) else {
            continue;
        };
        let normalized = normalize_type_name(text);
        if normalized.is_empty() {
            continue;
        }
        if first_any.is_none() {
            first_any = Some(normalized.clone());
        }
        if !looks_like_interface(&normalized) && first_non_interface.is_none() {
            first_non_interface = Some(normalized);
        }
    }
    // C# requires the base class to be the first item in the base list, but
    // tree-sitter has no way to distinguish a class from an interface at the
    // syntax level. We use the conventional `I` + uppercase prefix as a
    // heuristic to skip apparent interfaces. This isn't perfect (e.g.
    // `class Foo : Item` where `Item` is actually an interface, or
    // `class Foo : ItemBase` where `ItemBase` happens to start with `I`),
    // but it gets the common Unity case right and matches the regex parser's
    // historical behavior.
    first_non_interface.or(first_any)
}

fn looks_like_interface(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('I')) && matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
}

fn normalize_type_name(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("global::").unwrap_or(s);
    let s = s.split('<').next().unwrap_or(s);
    let s = s.rsplit('.').next().unwrap_or(s);
    s.trim().trim_end_matches('?').to_string()
}

fn collect_serialized_fields(source: &[u8], body: Node, out: &mut Vec<ScriptFieldMeta>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "field_declaration" => parse_field_declaration(source, child, out),
            "property_declaration" => parse_property_declaration(source, child, out),
            _ => {}
        }
    }
}

#[derive(Default, Debug)]
struct AttrInfo {
    /// `[SerializeField]` directly on a field declaration.
    has_plain_serialize_field: bool,
    /// `[field: SerializeField]` on a property declaration — targets the
    /// compiler-generated backing field rather than the property itself.
    has_field_target_serialize_field: bool,
    has_hide_in_inspector: bool,
    has_non_serialized: bool,
    former_names: Vec<String>,
}

fn collect_attribute_info(source: &[u8], node: Node) -> AttrInfo {
    let mut info = AttrInfo::default();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "attribute_list" {
            continue;
        }

        // Check for an attribute target specifier (e.g. `[field: ...]`).
        let mut tgt_cursor = child.walk();
        let target = child
            .children(&mut tgt_cursor)
            .find(|n| n.kind() == "attribute_target_specifier")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim_end_matches(':').trim().to_string());
        let is_field_target = matches!(target.as_deref(), Some("field"));

        let mut attr_cursor = child.walk();
        for attr in child.children(&mut attr_cursor) {
            if attr.kind() != "attribute" {
                continue;
            }
            let name = attr
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .unwrap_or("");
            let short = name.rsplit('.').next().unwrap_or(name);

            match short {
                "SerializeField" => {
                    if is_field_target {
                        info.has_field_target_serialize_field = true;
                    } else {
                        info.has_plain_serialize_field = true;
                    }
                }
                "HideInInspector" => info.has_hide_in_inspector = true,
                "NonSerialized" => info.has_non_serialized = true,
                "FormerlySerializedAs" => {
                    if let Some(arg) = first_string_argument(source, attr) {
                        info.former_names.push(arg);
                    }
                }
                _ => {}
            }
        }
    }
    info
}

/// Extract the first string-literal argument of an attribute, stripping
/// quotes. Returns `None` for attributes whose argument is not a literal —
/// `[FormerlySerializedAs(nameof(SomeField))]`,
/// `[FormerlySerializedAs($"prefix_{x}")]`, or any expression that resolves
/// to a string at compile time. We could resolve `nameof(...)` syntactically,
/// but the rest are out of reach without a real C# evaluator, so we degrade
/// to "no recorded former name" rather than risking a wrong rename mapping.
fn first_string_argument(source: &[u8], attr: Node) -> Option<String> {
    let mut cursor = attr.walk();
    let args = attr
        .children(&mut cursor)
        .find(|c| c.kind() == "attribute_argument_list")?;
    let mut found: Option<String> = None;
    walk_for_string_literal(source, args, &mut found);
    found
}

fn walk_for_string_literal(source: &[u8], node: Node, out: &mut Option<String>) {
    if out.is_some() {
        return;
    }
    let kind = node.kind();
    if kind == "string_literal" || kind == "verbatim_string_literal" || kind == "raw_string_literal"
    {
        if let Ok(text) = node.utf8_text(source) {
            *out = Some(unquote_csharp_string(text));
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_string_literal(source, child, out);
        if out.is_some() {
            return;
        }
    }
}

// Decode a C# string literal to its contained text. This is intentionally
// scoped to what `[FormerlySerializedAs]` arguments need in practice — old
// field names are almost always plain identifiers, occasionally wrapped in a
// verbatim string. We handle:
//
// - Verbatim `@"foo""bar"`: strip `@"` and the trailing quote, then collapse
//   doubled `""` to a single `"` per the C# verbatim escape rule.
// - Plain `"foo\"bar"`: strip the surrounding quotes and decode the common
//   single-character escapes (`\"`, `\\`, `\n`, `\r`, `\t`, `\0`). Other
//   escapes (`\uFFFF`, `\xFF`, `\U00010000`) are passed through unchanged.
//
// We do NOT handle:
// - Raw string literals (`"""..."""`, C# 11+).
// - Interpolated strings (`$"..."`).
//
// If field-rename tracking starts misfiring on more exotic forms, replace
// this with a real C# string-literal decoder.
fn unquote_csharp_string(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('@').and_then(|r| r.strip_prefix('"')) {
        let inner = rest.strip_suffix('"').unwrap_or(rest);
        return inner.replace("\"\"", "\"");
    }
    let inner = s.strip_prefix('"').unwrap_or(s);
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    decode_csharp_simple_escapes(inner)
}

fn decode_csharp_simple_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            // Pass through anything else (`\uFFFF`, `\xFF`, unknown) verbatim;
            // we explicitly do not try to decode hex / unicode escapes here.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn collect_modifiers(source: &[u8], node: Node) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            if let Ok(text) = child.utf8_text(source) {
                out.push(text.to_string());
            }
        }
    }
    out
}

fn parse_field_declaration(source: &[u8], node: Node, out: &mut Vec<ScriptFieldMeta>) {
    let attrs = collect_attribute_info(source, node);
    if attrs.has_non_serialized {
        return;
    }

    let modifiers = collect_modifiers(source, node);
    let is_public = modifiers.iter().any(|m| m == "public");
    let is_static = modifiers.iter().any(|m| m == "static");
    let is_const = modifiers.iter().any(|m| m == "const");
    if is_static || is_const {
        return;
    }
    if !is_public && !attrs.has_plain_serialize_field {
        return;
    }

    // field_declaration → variable_declaration → variable_declarator+
    let mut cursor = node.walk();
    let Some(var_decl) = node
        .children(&mut cursor)
        .find(|c| c.kind() == "variable_declaration")
    else {
        return;
    };

    let field_type = var_decl
        .child_by_field_name("type")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut decl_cursor = var_decl.walk();
    for declarator in var_decl.children(&mut decl_cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let name = declarator
            .child_by_field_name("name")
            .or_else(|| declarator.named_child(0))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.to_string());
        let Some(name) = name else {
            continue;
        };
        out.push(ScriptFieldMeta {
            name,
            field_type: field_type.clone(),
            former_names: attrs.former_names.clone(),
            hidden: attrs.has_hide_in_inspector,
            serialize_field: true,
        });
    }
}

/// Returns `true` if every accessor of the property is an auto-accessor
/// (no `{ ... }` body and no `=> expr` arrow body). Expression-bodied
/// properties (`public int Hp => 42;`) have no accessor list at all and
/// also return `false` here — they have no compiler-generated backing field.
fn is_auto_property(node: Node) -> bool {
    let mut cursor = node.walk();
    let Some(accessor_list) = node
        .children(&mut cursor)
        .find(|c| c.kind() == "accessor_list")
    else {
        // Expression-bodied property: `public int Hp => _hp;` — no backing
        // field exists, so `[field: SerializeField]` would be invalid.
        return false;
    };

    let mut acc_cursor = accessor_list.walk();
    let mut saw_accessor = false;
    for accessor in accessor_list.children(&mut acc_cursor) {
        if accessor.kind() != "accessor_declaration" {
            continue;
        }
        saw_accessor = true;
        // An auto-accessor's only meaningful child is the `get` / `set` /
        // `init` keyword followed by `;`. If we see a block or an arrow
        // expression as a child, it's not an auto-accessor.
        let mut child_cursor = accessor.walk();
        for child in accessor.children(&mut child_cursor) {
            if matches!(child.kind(), "block" | "arrow_expression_clause") {
                return false;
            }
        }
    }
    saw_accessor
}

fn parse_property_declaration(source: &[u8], node: Node, out: &mut Vec<ScriptFieldMeta>) {
    // Only properties tagged with `[field: SerializeField]` participate in
    // Unity serialization.
    let attrs = collect_attribute_info(source, node);
    if !attrs.has_field_target_serialize_field || attrs.has_non_serialized {
        return;
    }

    // `[field: SerializeField]` is only meaningful on auto-properties — the
    // attribute targets the compiler-generated backing field, which only
    // exists when every accessor is auto (`get;` / `set;` with no body).
    // The C# compiler rejects `[field: ...]` on properties with bodies, but
    // tree-sitter still parses them; if we don't filter we'd emit a phantom
    // ScriptFieldMeta that no Unity YAML will ever reference.
    if !is_auto_property(node) {
        return;
    }

    let modifiers = collect_modifiers(source, node);
    if modifiers.iter().any(|m| m == "static" || m == "const") {
        return;
    }

    let field_type = node
        .child_by_field_name("type")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .map(|s| s.to_string())
    else {
        return;
    };

    out.push(ScriptFieldMeta {
        name,
        field_type,
        former_names: attrs.former_names,
        hidden: attrs.has_hide_in_inspector,
        serialize_field: true,
    });
}
