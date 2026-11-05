//! Plain `[Serializable]` class / struct values for `unity_code_usages`.
//!
//! YAML stores no type identity for a plain serialized field — `tuning:` is
//! just a nested mapping — so usages are found from the code side:
//!
//! 1. **Routes.** Every serialized field whose declared type carries the
//!    target `T` (`T`, `T[]`, `List<T>`, or a project generic such as
//!    `Slot<T>` through its type parameters) is a carrier. Carriers inside
//!    other plain `[Serializable]` types are followed outward until they reach
//!    a root: a MonoBehaviour / ScriptableObject script (and every script
//!    deriving from it), or a `[SerializeReference]` type (and its subclasses).
//!    Each route is a field path such as `tuning.steps[].max`.
//! 2. **Values.** Script roots are found through assets referencing the
//!    script GUID; `[SerializeReference]` roots through the `managed_refs`
//!    registry. The route is then walked as a YAML key path inside the
//!    component / registry `data:` block.
//! 3. **Overrides.** Prefab-instance modifications whose property path
//!    follows a route (`tuning.steps.Array.data[0].max`), on the found prefab
//!    components or — through Unity's XOR fileID rule — on their instances in
//!    variants and nested prefabs; plus `managedReferences[R].…` paths whose
//!    effective type is a `[SerializeReference]` root.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::code_unity_managed::{
    override_rid_type, ManagedSnapshot, RidType, SharedAssetDb, TypeCatalog, TypeKey,
};
use crate::asset_db::types::{AssetKind, Guid};
use crate::asset_db::AssetDb;
use crate::unity_csharp::{parse_field_type, DeclaredTypeKind, FieldTypeShape};
use crate::unity_yaml::managed_refs::{managed_class_key, ManagedTypeIdentity};
use crate::unity_yaml::prefab_overrides::{
    nested_file_id, parse_managed_reference_path, tokenize_property_path, PathToken,
};

/// Plain-type nesting followed outward from the target.
const MAX_ROUTE_DEPTH: usize = 6;
/// Generic substitution depth (`Slot<Pair<T>>`).
const MAX_GENERIC_DEPTH: usize = 4;
/// Routes kept per query; real projects stay far below.
const MAX_ROUTES: usize = 256;
/// Prefab nesting followed when collecting overrides of a found component.
const MAX_OVERRIDE_NESTING: usize = 4;

fn with_db<T>(
    asset_db: &SharedAssetDb,
    f: impl FnOnce(&AssetDb) -> Result<T, String>,
) -> Result<T, String> {
    let guard = asset_db
        .lock()
        .map_err(|error| format!("Failed to lock AssetDb: {error}"))?;
    match guard.as_ref() {
        Some(db) => f(db),
        None => Err("AssetDb not initialized".to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum PathStep {
    Field(String),
    /// One element of an array / `List<>`.
    Element,
}

fn render_route(steps: &[PathStep]) -> String {
    let mut out = String::new();
    for step in steps {
        match step {
            PathStep::Field(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(name);
            }
            PathStep::Element => out.push_str("[]"),
        }
    }
    out
}

#[derive(Debug, Clone)]
pub(super) struct ScriptRoute {
    pub script_guid: Guid,
    pub class_name: String,
    pub path: Vec<PathStep>,
    /// Display name of the plain type the route ends at.
    pub target: String,
}

#[derive(Debug, Clone)]
pub(super) struct ManagedRoute {
    pub identity: ManagedTypeIdentity,
    pub path: Vec<PathStep>,
    pub target: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PlainPlan {
    /// Plain `[Serializable]` types declared in the queried file.
    pub targets: Vec<String>,
    /// Declared classes / structs without `[Serializable]` (only reported
    /// when the file declares no MonoBehaviour / ScriptableObject).
    pub not_serializable: Vec<String>,
    pub script_routes: Vec<ScriptRoute>,
    pub managed_routes: Vec<ManagedRoute>,
    /// Field path the member search appends (`member` + former names).
    pub member_names: Vec<String>,
    pub member: Option<String>,
}

impl PlainPlan {
    pub(super) fn route_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .script_routes
            .iter()
            .map(|r| format!("{}.{}", r.class_name, render_route(&r.path)))
            .chain(
                self.managed_routes
                    .iter()
                    .map(|r| format!("{}.{}", r.identity.display_name(), render_route(&r.path))),
            )
            .collect();
        labels.sort();
        labels.dedup();
        labels
    }
}

/// A carrier field: `host.field…` reaches the target through `path`.
#[derive(Debug, Clone)]
struct Carrier {
    host: TypeKey,
    path: Vec<PathStep>,
}

/// Bound generic parameter: its argument shape and the context it was
/// written in.
type Bindings = HashMap<String, (FieldTypeShape, TypeKey)>;

/// Discover the routes from roots to the plain types declared in `file`.
pub(super) fn plan_plain_routes(
    asset_db: &SharedAssetDb,
    catalog: &mut TypeCatalog,
    snapshot: &ManagedSnapshot,
    file: &str,
    member: Option<&str>,
) -> Result<PlainPlan, String> {
    let mut plan = PlainPlan::default();
    let file_keys = catalog.types_in_file(file);
    let mut targets: Vec<TypeKey> = Vec::new();
    let mut has_unity_object = false;
    for key in file_keys {
        let decl = catalog.decl(key);
        if decl.kind == DeclaredTypeKind::Interface || decl.arity > 0 {
            continue;
        }
        if catalog.is_unity_object(key) {
            has_unity_object = true;
            continue;
        }
        if catalog.decl(key).is_serializable {
            plan.targets.push(catalog.display(key));
            targets.push(key);
        } else {
            plan.not_serializable.push(catalog.display(key));
        }
    }
    if has_unity_object {
        plan.not_serializable.clear();
    }
    if let Some(member) = member {
        let mut names = vec![member.to_string()];
        for key in &targets {
            for field in &catalog.decl(*key).serialized_fields {
                if field.name == member {
                    names.extend(field.former_names.iter().cloned());
                }
            }
        }
        names.sort();
        names.dedup();
        plan.member_names = names;
        plan.member = Some(member.to_string());
    }
    if targets.is_empty() {
        return Ok(plan);
    }

    let project_assemblies = catalog.project_assemblies();
    let mut carrier_memo: HashMap<TypeKey, Vec<Carrier>> = HashMap::new();
    let mut identity_types: Option<Vec<(ManagedTypeIdentity, TypeKey)>> = None;
    let mut seen_routes: HashSet<(String, Vec<PathStep>)> = HashSet::new();

    for target in targets {
        let target_name = catalog.display(target);
        // (host, path from host to target, hosts on the chain)
        let mut queue: Vec<(TypeKey, Vec<PathStep>, Vec<TypeKey>)> =
            vec![(target, Vec::new(), vec![target])];
        while let Some((host, path, chain)) = queue.pop() {
            if plan.script_routes.len() + plan.managed_routes.len() >= MAX_ROUTES {
                break;
            }
            if !path.is_empty() {
                if catalog.is_unity_object(host) {
                    for (guid, class_name) in script_roots(asset_db, catalog, host)? {
                        let key = (format!("s:{}", hex(&guid)), path.clone());
                        if seen_routes.insert(key) {
                            plan.script_routes.push(ScriptRoute {
                                script_guid: guid,
                                class_name,
                                path: path.clone(),
                                target: target_name.clone(),
                            });
                        }
                    }
                    continue;
                }
                let managed = identity_types.get_or_insert_with(|| {
                    resolve_project_identities(catalog, snapshot, &project_assemblies)
                });
                for (identity, concrete) in managed.clone() {
                    if concrete == host
                        || catalog
                            .derives_from(concrete, &HashSet::from([host]))
                            .is_some()
                    {
                        let key = (format!("m:{}", identity.display_name()), path.clone());
                        if seen_routes.insert(key) {
                            plan.managed_routes.push(ManagedRoute {
                                identity: identity.clone(),
                                path: path.clone(),
                                target: target_name.clone(),
                            });
                        }
                    }
                }
            }
            if chain.len() > MAX_ROUTE_DEPTH {
                continue;
            }
            if host != target && !catalog.decl(host).is_serializable {
                continue;
            }
            let carriers = match carrier_memo.get(&host) {
                Some(found) => found.clone(),
                None => {
                    let found = find_carriers(catalog, host);
                    carrier_memo.insert(host, found.clone());
                    found
                }
            };
            for carrier in carriers {
                if chain.contains(&carrier.host) {
                    continue;
                }
                let mut full = carrier.path.clone();
                full.extend(path.iter().cloned());
                let mut next_chain = chain.clone();
                next_chain.push(carrier.host);
                queue.push((carrier.host, full, next_chain));
            }
        }
    }
    Ok(plan)
}

fn hex(guid: &Guid) -> String {
    crate::asset_db::types::guid_to_hex(guid)
}

/// Project-assembly identities (registry + override types) with their
/// resolved declarations.
fn resolve_project_identities(
    catalog: &mut TypeCatalog,
    snapshot: &ManagedSnapshot,
    project_assemblies: &HashSet<String>,
) -> Vec<(ManagedTypeIdentity, TypeKey)> {
    use super::code_unity_managed::Resolution;
    let mut out = Vec::new();
    for stat in snapshot.identity_stats() {
        if !project_assemblies.contains(&stat.identity.assembly) {
            continue;
        }
        match catalog.resolve_identity(&stat.identity) {
            Resolution::Exact(key)
            | Resolution::Legacy(key)
            | Resolution::AssemblyMismatch(key) => out.push((stat.identity.clone(), key)),
            Resolution::Missing => {}
        }
    }
    out
}

/// The script of `host` plus every script deriving from it (verified by
/// walking the derived file's bases), as `(guid, class name)`.
fn script_roots(
    asset_db: &SharedAssetDb,
    catalog: &mut TypeCatalog,
    host: TypeKey,
) -> Result<Vec<(Guid, String)>, String> {
    let mut out = Vec::new();
    if let Some(guid) = catalog.script_guid_of(host) {
        out.push((guid, catalog.decl(host).name().to_string()));
    }
    let decl = catalog.decl(host);
    let short = decl.name().to_ascii_lowercase();
    let full = if decl.namespace.is_empty() {
        short.clone()
    } else {
        format!("{}.{}", decl.namespace.to_ascii_lowercase(), short)
    };
    let derived = with_db(asset_db, |db| db.find_scripts_inheriting(&[short, full]))?;
    for (guid, path) in derived {
        if out.iter().any(|(g, _)| *g == guid) {
            continue;
        }
        let stem = Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(key) = catalog.types_in_file(&path).into_iter().find(|key| {
            catalog.decl(*key).nested_path.len() == 1 && catalog.decl(*key).name() == stem
        }) else {
            continue;
        };
        if catalog.derives_from(key, &HashSet::from([host])).is_some() {
            out.push((guid, stem));
        }
    }
    Ok(out)
}

/// Serialized fields (anywhere in the project) whose declared type carries
/// `target`, with the path from the declaring type to the target value.
fn find_carriers(catalog: &mut TypeCatalog, target: TypeKey) -> Vec<Carrier> {
    let name = catalog.decl(target).name().to_string();
    let mut paths = catalog.files_mentioning(&name);
    paths.push(catalog.file_path(target).to_string());
    paths.sort();
    paths.dedup();
    let mut out = Vec::new();
    for path in paths {
        for host in catalog.types_in_file(&path) {
            if catalog.decl(host).kind == DeclaredTypeKind::Interface {
                continue;
            }
            let fields = catalog.decl(host).serialized_fields.clone();
            for field in fields {
                if field.serialize_reference || !field.field_type.contains(name.as_str()) {
                    continue;
                }
                let Some(shape) = parse_field_type(&field.field_type) else {
                    continue;
                };
                for rest in match_shape(catalog, host, &shape, target, &Bindings::new(), 0) {
                    let mut path = vec![PathStep::Field(field.name.clone())];
                    path.extend(rest);
                    out.push(Carrier { host, path });
                }
            }
        }
    }
    out
}

/// Relative paths through which a value of `shape` (written in `ctx`)
/// contains `target`.
fn match_shape(
    catalog: &mut TypeCatalog,
    ctx: TypeKey,
    shape: &FieldTypeShape,
    target: TypeKey,
    bindings: &Bindings,
    depth: usize,
) -> Vec<Vec<PathStep>> {
    let prefixed = |paths: Vec<Vec<PathStep>>| -> Vec<Vec<PathStep>> {
        paths
            .into_iter()
            .map(|rest| {
                let mut path = vec![PathStep::Element];
                path.extend(rest);
                path
            })
            .collect()
    };
    if let Some(element) = shape.element() {
        return prefixed(match_shape(catalog, ctx, &element, target, bindings, depth));
    }
    if shape.is_list() {
        return prefixed(match_shape(
            catalog,
            ctx,
            &shape.args[0],
            target,
            bindings,
            depth,
        ));
    }
    if shape.name.segments.len() == 1 && shape.args.is_empty() {
        if let Some((bound, bound_ctx)) = bindings.get(&shape.name.segments[0]) {
            let (bound, bound_ctx) = (bound.clone(), *bound_ctx);
            return match_shape(catalog, bound_ctx, &bound, target, &Bindings::new(), depth);
        }
    }
    let resolved = catalog.resolve_type_ref(ctx, &shape.name);
    if resolved.contains(&target) {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    if depth >= MAX_GENERIC_DEPTH || shape.args.is_empty() {
        return out;
    }
    for generic in resolved {
        let decl = catalog.decl(generic).clone();
        if !decl.is_serializable || decl.type_params.len() != shape.args.len() {
            continue;
        }
        let new_bindings: Bindings = decl
            .type_params
            .iter()
            .cloned()
            .zip(shape.args.iter().map(|arg| (arg.clone(), ctx)))
            .collect();
        for field in &decl.serialized_fields {
            if field.serialize_reference {
                continue;
            }
            let Some(inner) = parse_field_type(&field.field_type) else {
                continue;
            };
            for rest in match_shape(catalog, generic, &inner, target, &new_bindings, depth + 1) {
                let mut path = vec![PathStep::Field(field.name.clone())];
                path.extend(rest);
                out.push(path);
            }
        }
    }
    out
}

// ─── YAML key-path walking ──────────────────────────────────────────────────

/// One YAML line with list items normalized: `  - min: 1` has its key at
/// column 4, like the `max:` line of the same item below it.
#[derive(Debug, Clone)]
struct YamlLine<'a> {
    key_indent: usize,
    item: bool,
    key: Option<&'a str>,
    value: &'a str,
    blank: bool,
}

fn normalize<'a>(lines: &[&'a str]) -> Vec<YamlLine<'a>> {
    lines
        .iter()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return YamlLine {
                    key_indent: usize::MAX,
                    item: false,
                    key: None,
                    value: "",
                    blank: true,
                };
            }
            let indent = line.len() - line.trim_start().len();
            let (item, content, key_indent) = match trimmed.strip_prefix('-') {
                Some(rest) if rest.is_empty() || rest.starts_with(' ') => {
                    (true, rest.trim_start(), indent + 2)
                }
                _ => (false, trimmed, indent),
            };
            let (key, value) = match content.find(':') {
                Some(colon)
                    if !content[..colon].is_empty()
                        && content[..colon]
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_') =>
                {
                    (Some(&content[..colon]), content[colon + 1..].trim())
                }
                _ => (None, content),
            };
            YamlLine {
                key_indent,
                item,
                key,
                value,
                blank: false,
            }
        })
        .collect()
}

/// `(line index, rendered path, value)` for every value at `steps` inside
/// lines `[start, end)` whose keys sit at `indent`.
fn walk(
    lines: &[YamlLine],
    start: usize,
    end: usize,
    indent: usize,
    steps: &[PathStep],
    rendered: &str,
    out: &mut Vec<(usize, String, String)>,
) {
    let Some((step, rest)) = steps.split_first() else {
        return;
    };
    let block_end = |from: usize, key_indent: usize| -> usize {
        (from + 1..end)
            .find(|j| !lines[*j].blank && lines[*j].key_indent <= key_indent)
            .unwrap_or(end)
    };
    match step {
        PathStep::Field(name) => {
            for i in start..end {
                let line = &lines[i];
                if line.blank || line.key_indent != indent || line.key != Some(name.as_str()) {
                    continue;
                }
                let path = if rendered.is_empty() {
                    name.clone()
                } else {
                    format!("{rendered}.{name}")
                };
                if rest.is_empty() {
                    out.push((i, path, line.value.to_string()));
                } else {
                    walk(
                        lines,
                        i + 1,
                        block_end(i, indent),
                        indent + 2,
                        rest,
                        &path,
                        out,
                    );
                }
            }
        }
        PathStep::Element => {
            let mut index = 0usize;
            let mut i = start;
            while i < end {
                let line = &lines[i];
                if line.blank || !line.item || line.key_indent != indent {
                    i += 1;
                    continue;
                }
                let item_end = (i + 1..end)
                    .find(|j| {
                        let other = &lines[*j];
                        !other.blank
                            && (other.key_indent < indent
                                || (other.item && other.key_indent == indent))
                    })
                    .unwrap_or(end);
                let path = format!("{rendered}[{index}]");
                if rest.is_empty() {
                    let value = match line.key {
                        Some(key) => format!("{key}: {}", line.value),
                        None => line.value.to_string(),
                    };
                    out.push((i, path, value));
                } else {
                    walk(lines, i, item_end, indent, rest, &path, out);
                }
                index += 1;
                i = item_end;
            }
        }
    }
}

/// Values at `steps` inside a component / ScriptableObject document whose
/// header line is `doc_start` (fields at indent 2).
pub(super) fn values_in_document(
    raw_lines: &[&str],
    doc_start: usize,
    doc_end: usize,
    steps: &[PathStep],
) -> Vec<(u32, String, String)> {
    let end = doc_end.min(raw_lines.len());
    let start = (doc_start + 1).min(end);
    let lines = normalize(&raw_lines[start..end]);
    let mut out = Vec::new();
    walk(&lines, 0, lines.len(), 2, steps, "", &mut out);
    out.into_iter()
        .map(|(i, path, value)| ((start + i) as u32 + 1, path, value))
        .collect()
}

/// Values at `steps` inside the `data:` block of the registry entry starting
/// at 1-based `entry_line`.
pub(super) fn values_in_registry_entry(
    raw_lines: &[&str],
    entry_line: u32,
    steps: &[PathStep],
) -> Vec<(u32, String, String)> {
    let entry = entry_line.saturating_sub(1) as usize;
    if entry >= raw_lines.len() {
        return Vec::new();
    }
    let lines = normalize(&raw_lines[entry..]);
    // `- rid: N` normalizes to key indent of its sibling keys (`type:`,
    // `data:`); the entry ends at the next item or anything shallower.
    let entry_indent = lines[0].key_indent;
    let entry_end = (1..lines.len())
        .find(|j| {
            let line = &lines[*j];
            !line.blank
                && (line.key_indent < entry_indent
                    || (line.item && line.key_indent == entry_indent))
        })
        .unwrap_or(lines.len());
    let Some(data) = (1..entry_end)
        .find(|j| lines[*j].key_indent == entry_indent && lines[*j].key == Some("data"))
    else {
        return Vec::new();
    };
    let data_end = (data + 1..entry_end)
        .find(|j| !lines[*j].blank && lines[*j].key_indent <= entry_indent)
        .unwrap_or(entry_end);
    let mut out = Vec::new();
    walk(
        &lines,
        data + 1,
        data_end,
        entry_indent + 2,
        steps,
        "",
        &mut out,
    );
    out.into_iter()
        .map(|(i, path, value)| ((entry + i) as u32 + 1, path, value))
        .collect()
}

/// Whether a Unity property path follows `steps` (`Element` matches any
/// `Array.data[N]`). With `count_resizes`, an `Array.size` change of a routed
/// sequence counts too — it adds or removes values, but touches no member.
pub(super) fn path_follows_route(
    tokens: &[PathToken],
    steps: &[PathStep],
    count_resizes: bool,
) -> bool {
    let resize = count_resizes && matches!(tokens.last(), Some(PathToken::Size));
    if tokens.len() < steps.len() && !resize {
        return false;
    }
    for (index, step) in steps.iter().enumerate() {
        match (tokens.get(index), step) {
            (Some(PathToken::Field(a)), PathStep::Field(b)) if a == b => {}
            (Some(PathToken::Element(_)), PathStep::Element) => {}
            (Some(PathToken::Size), PathStep::Element) if resize => return true,
            _ => return false,
        }
    }
    true
}

// ─── value search ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub(super) struct PlainReport {
    /// `(asset path, line, description)` per value / override.
    pub entries: Vec<(String, u32, String)>,
    pub asset_count: usize,
    /// Plain types that produced at least one entry, sorted.
    pub targets: Vec<String>,
}

struct LoadedAsset {
    lines: Vec<String>,
    docs: Vec<crate::unity_yaml::YamlDoc>,
}

impl LoadedAsset {
    fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        Some(Self {
            docs: crate::unity_yaml::parse_yaml_docs_str(&text),
            lines: text.lines().map(str::to_string).collect(),
        })
    }

    fn go_name(&self, go: Option<i64>) -> Option<String> {
        let go = go?;
        self.docs
            .iter()
            .find(|d| d.file_id == go && d.class_id == 1)
            .and_then(|d| d.m_name.clone())
    }

    fn instance_name(&self, instance: i64) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.file_id == instance && d.class_id == 1001)
            .and_then(|d| d.m_name.clone())
    }
}

fn preview(value: &str) -> String {
    let value = if value.is_empty() { "{…}" } else { value };
    if value.chars().count() > 60 {
        value.chars().take(60).collect::<String>() + "…"
    } else {
        value.to_string()
    }
}

/// Locate serialized values (and, in member mode, the member's values) along
/// every route.
pub(super) fn search_plain_values(
    asset_db: &SharedAssetDb,
    root: &Path,
    plan: &PlainPlan,
    snapshot: &ManagedSnapshot,
) -> Result<PlainReport, String> {
    let mut report = PlainReport::default();
    if plan.script_routes.is_empty() && plan.managed_routes.is_empty() {
        return Ok(report);
    }
    let member_steps: Vec<Vec<PathStep>> = if plan.member.is_some() {
        plan.member_names
            .iter()
            .map(|name| vec![PathStep::Field(name.clone())])
            .collect()
    } else {
        vec![Vec::new()]
    };
    let extend = |route: &[PathStep], suffix: &[PathStep]| -> Vec<PathStep> {
        let mut full = route.to_vec();
        full.extend(suffix.iter().cloned());
        full
    };

    let mut assets: HashMap<String, Option<LoadedAsset>> = HashMap::new();
    let mut touched: HashSet<String> = HashSet::new();
    let label = |plain: bool| if plain { "value" } else { "field" };
    let is_member = plan.member.is_some();

    // Script roots: components / ScriptableObjects attached by GUID.
    let mut by_script: HashMap<Guid, Vec<&ScriptRoute>> = HashMap::new();
    for route in &plan.script_routes {
        by_script.entry(route.script_guid).or_default().push(route);
    }
    // Prefab components found, for override search: (prefab guid, path, fileID, routes).
    let mut components: Vec<(Guid, String, i64, Vec<&ScriptRoute>)> = Vec::new();
    for (script_guid, routes) in &by_script {
        let referencing = with_db(asset_db, |db| {
            let mut found = Vec::new();
            let mut seen = HashSet::new();
            for edge in db.get_direct_refs(script_guid)? {
                if !seen.insert(edge.src_guid) {
                    continue;
                }
                if let Some((path, kind)) = db.resolve_path_and_kind_by_guid(&edge.src_guid)? {
                    if matches!(
                        kind,
                        AssetKind::Scene
                            | AssetKind::Prefab
                            | AssetKind::GenericAsset
                            | AssetKind::Controller
                            | AssetKind::OtherYaml
                    ) {
                        found.push((edge.src_guid, path));
                    }
                }
            }
            Ok(found)
        })?;
        for (asset_guid, path) in referencing {
            let loaded = assets
                .entry(path.clone())
                .or_insert_with(|| LoadedAsset::load(&root.join(&path)));
            let Some(asset) = loaded.as_ref() else {
                continue;
            };
            let raw: Vec<&str> = asset.lines.iter().map(String::as_str).collect();
            for doc in asset
                .docs
                .iter()
                .filter(|d| d.m_script_guid.as_ref() == Some(script_guid))
            {
                let host = match asset.go_name(doc.m_game_object_id) {
                    Some(go) => format!(" on GameObject '{go}'"),
                    None => match doc.m_name.as_deref() {
                        Some(name) if !name.is_empty() => format!(" in '{name}'"),
                        _ => String::new(),
                    },
                };
                for route in routes {
                    for suffix in &member_steps {
                        let steps = extend(&route.path, suffix);
                        for (line, rendered, value) in
                            values_in_document(&raw, doc.line_start, doc.line_end, &steps)
                        {
                            touched.insert(path.clone());
                            report.entries.push((
                                path.clone(),
                                line,
                                format!(
                                    "[Serializable {}] {}.{rendered}: {}{host} — {}",
                                    label(!is_member),
                                    route.class_name,
                                    preview(&value),
                                    route.target
                                ),
                            ));
                        }
                    }
                }
                if path.to_ascii_lowercase().ends_with(".prefab") && !doc.is_stripped {
                    components.push((asset_guid, path.clone(), doc.file_id, routes.clone()));
                }
            }
        }
    }

    // Prefab-instance overrides of routed fields on those components,
    // following instances into variants / nested prefabs.
    for (prefab_guid, prefab_path, component, routes) in components {
        let mut queue = vec![(prefab_guid, prefab_path, component, 0usize)];
        let mut visited: HashSet<(Guid, i64)> = HashSet::new();
        while let Some((guid, source_path, file_id, depth)) = queue.pop() {
            if !visited.insert((guid, file_id)) {
                continue;
            }
            let (rows, instances) = with_db(asset_db, |db| {
                Ok((
                    db.get_prefab_overrides_for_target(&guid, file_id)?,
                    if depth < MAX_OVERRIDE_NESTING {
                        db.get_prefab_instances_of(&guid)?
                    } else {
                        Vec::new()
                    },
                ))
            })?;
            for row in rows {
                if row.property_path.starts_with("managedReferences[") {
                    continue;
                }
                let tokens = tokenize_property_path(&row.property_path);
                let matched = routes.iter().find(|route| {
                    member_steps.iter().any(|suffix| {
                        path_follows_route(&tokens, &extend(&route.path, suffix), !is_member)
                    })
                });
                let Some(route) = matched else {
                    continue;
                };
                let instance = assets
                    .entry(row.path.clone())
                    .or_insert_with(|| LoadedAsset::load(&root.join(&row.path)))
                    .as_ref()
                    .and_then(|asset| asset.instance_name(row.instance_file_id))
                    .map(|name| format!("prefab instance '{name}'"))
                    .unwrap_or_else(|| "prefab instance".to_string());
                touched.insert(row.path.clone());
                report.entries.push((
                    row.path.clone(),
                    row.line,
                    format!(
                        "[Serializable override] {}: {} — {} {instance} of {source_path} — {}",
                        row.property_path,
                        preview(&row.value),
                        route.class_name,
                        route.target
                    ),
                ));
            }
            for (instance_guid, instance_path, instance_id) in instances {
                if instance_path.to_ascii_lowercase().ends_with(".prefab") {
                    queue.push((
                        instance_guid,
                        instance_path,
                        nested_file_id(instance_id, file_id),
                        depth + 1,
                    ));
                }
            }
        }
    }

    // `[SerializeReference]` roots: registry data blocks.
    if !plan.managed_routes.is_empty() {
        let keys: Vec<String> = plan
            .managed_routes
            .iter()
            .map(|route| managed_class_key(&route.identity.class_name))
            .collect();
        let hits = with_db(asset_db, |db| db.get_managed_refs_by_class_keys(&keys))?;
        for hit in hits {
            let identity = ManagedTypeIdentity {
                class_name: hit.class_name.clone(),
                namespace: hit.namespace.clone(),
                assembly: hit.assembly.clone(),
            };
            let routes: Vec<&ManagedRoute> = plan
                .managed_routes
                .iter()
                .filter(|route| route.identity == identity)
                .collect();
            if routes.is_empty() {
                continue;
            }
            let loaded = assets
                .entry(hit.path.clone())
                .or_insert_with(|| LoadedAsset::load(&root.join(&hit.path)));
            let Some(asset) = loaded.as_ref() else {
                continue;
            };
            let raw: Vec<&str> = asset.lines.iter().map(String::as_str).collect();
            for route in routes {
                for suffix in &member_steps {
                    let steps = extend(&route.path, suffix);
                    for (line, rendered, value) in values_in_registry_entry(&raw, hit.line, &steps)
                    {
                        touched.insert(hit.path.clone());
                        report.entries.push((
                            hit.path.clone(),
                            line,
                            format!(
                                "[Serializable {}] {} (rid {}).{rendered}: {} — {}",
                                label(!is_member),
                                identity.display_name(),
                                hit.rid,
                                preview(&value),
                                route.target
                            ),
                        ));
                    }
                }
            }
        }

        // `managedReferences[R].<route>` modifications on prefab instances.
        let candidates: Vec<_> = snapshot
            .managed_override_rows()
            .iter()
            .filter_map(|row| {
                let (rid, tail) = parse_managed_reference_path(&row.property_path)?;
                if tail.is_empty() {
                    return None;
                }
                let tokens = tokenize_property_path(tail);
                let routes: Vec<&ManagedRoute> = plan
                    .managed_routes
                    .iter()
                    .filter(|route| {
                        member_steps.iter().any(|suffix| {
                            path_follows_route(&tokens, &extend(&route.path, suffix), !is_member)
                        })
                    })
                    .collect();
                (!routes.is_empty()).then(|| (row.clone(), rid, routes))
            })
            .collect();
        for (row, rid, routes) in candidates {
            let resolved = with_db(asset_db, |db| override_rid_type(db, &row, rid))?;
            let Some(RidType::Type(identity)) = resolved else {
                continue;
            };
            let Some(route) = routes.iter().find(|route| route.identity == identity) else {
                continue;
            };
            touched.insert(row.path.clone());
            report.entries.push((
                row.path.clone(),
                row.line,
                format!(
                    "[Serializable override] {}: {} — {} (rid {rid}) — {}",
                    row.property_path,
                    preview(&row.value),
                    identity.display_name(),
                    route.target
                ),
            ));
        }
    }

    report
        .entries
        .sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    report.entries.dedup();
    report.asset_count = touched.len();
    let mut targets: Vec<String> = plan
        .targets
        .iter()
        .filter(|target| {
            let suffix = format!("— {target}");
            report
                .entries
                .iter()
                .any(|(_, _, desc)| desc.ends_with(&suffix))
        })
        .cloned()
        .collect();
    targets.sort();
    report.targets = targets;
    Ok(report)
}
