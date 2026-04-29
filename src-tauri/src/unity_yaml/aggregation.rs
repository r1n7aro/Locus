use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{
    guid_to_hex, BulkPropertyOverride, Guid, KeyOverride, OverrideSummary, PrefabInstanceIR,
    PrefabSourceRef, PropertyOverride, RendererOverrideSummary, TransformOverrideSummary,
};

use super::parser::{
    build_hierarchy_path_map, find_hierarchy_node_by_path, format_go_annotations,
    round_decimal_str, HierarchyNode, YamlDoc,
};
use super::prefab::{extract_prefab_instance_irs, extract_stripped_mappings};

pub struct SourcePrefabContext {
    pub tree: Vec<HierarchyNode>,
    pub docs: Vec<YamlDoc>,
}

const TRANSFORM_PROPS: &[&str] = &[
    "m_LocalPosition.",
    "m_LocalRotation.",
    "m_LocalScale.",
    "m_LocalEulerAnglesHint.",
    "m_RootOrder",
];

pub const DEFAULT_HIERARCHY_MAX_NODES: usize = 1000;

const RENDERER_PROPS: &[&str] = &[
    "m_Materials.",
    "m_Enabled",
    "m_CastShadows",
    "m_ReceiveShadows",
    "m_LightProbeUsage",
    "m_ReflectionProbeUsage",
];

const KEY_PROPS: &[&str] = &[
    "m_Name",
    "m_IsActive",
    "m_Enabled",
    "m_Layer",
    "m_TagString",
    "m_StaticEditorFlags",
];

pub fn summarize_prefab_instance(
    ir: &PrefabInstanceIR,
    stripped_count: usize,
    child_prefab_names: Vec<String>,
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
) -> OverrideSummary {
    let source_path = guid_resolver(&ir.source_prefab_guid);

    let mut by_target: HashMap<PrefabSourceRef, Vec<&PropertyOverride>> = HashMap::new();
    for ov in &ir.property_overrides {
        by_target.entry(ov.target.clone()).or_default().push(ov);
    }

    let mut transform_overrides = Vec::new();
    let mut bulk_overrides_map: HashMap<(String, String), Vec<i64>> = HashMap::new();
    let mut renderer_overrides_map: HashMap<PrefabSourceRef, Vec<(String, String)>> =
        HashMap::new();
    let mut key_overrides = Vec::new();

    for (target, overrides) in &by_target {
        let mut pos: [Option<String>; 3] = [None, None, None];
        let mut rot: [Option<String>; 4] = [None, None, None, None];
        let mut scale: [Option<String>; 3] = [None, None, None];
        let mut euler: [Option<String>; 3] = [None, None, None];
        let mut has_transform = false;

        for ov in overrides {
            let pp = &ov.property_path;

            if TRANSFORM_PROPS.iter().any(|p| pp.starts_with(p)) {
                has_transform = true;
                let val = ov.value.clone();
                match pp.as_str() {
                    "m_LocalPosition.x" => pos[0] = val,
                    "m_LocalPosition.y" => pos[1] = val,
                    "m_LocalPosition.z" => pos[2] = val,
                    "m_LocalRotation.x" => rot[0] = val,
                    "m_LocalRotation.y" => rot[1] = val,
                    "m_LocalRotation.z" => rot[2] = val,
                    "m_LocalRotation.w" => rot[3] = val,
                    "m_LocalScale.x" => scale[0] = val,
                    "m_LocalScale.y" => scale[1] = val,
                    "m_LocalScale.z" => scale[2] = val,
                    "m_LocalEulerAnglesHint.x" => euler[0] = val,
                    "m_LocalEulerAnglesHint.y" => euler[1] = val,
                    "m_LocalEulerAnglesHint.z" => euler[2] = val,
                    _ => {} // m_RootOrder etc
                }
                continue;
            }

            if let Some(v) = &ov.value {
                let key = (pp.clone(), v.clone());
                bulk_overrides_map
                    .entry(key)
                    .or_default()
                    .push(target.source_file_id);
                if KEY_PROPS.iter().any(|k| pp.starts_with(k)) {
                    key_overrides.push(KeyOverride {
                        target: target.clone(),
                        label: None,
                        property_path: pp.clone(),
                        value: Some(v.clone()),
                        object_ref_desc: None,
                    });
                }
            }

            if RENDERER_PROPS.iter().any(|p| pp.starts_with(p)) {
                let val = ov.value.clone().unwrap_or_default();
                renderer_overrides_map
                    .entry(target.clone())
                    .or_default()
                    .push((pp.clone(), val));
                continue;
            }
        }

        if has_transform {
            let has_pos = pos.iter().any(|v| v.is_some());
            let has_rot = rot.iter().any(|v| v.is_some());
            let has_scale = scale.iter().any(|v| v.is_some());
            let has_euler = euler.iter().any(|v| v.is_some());
            transform_overrides.push(TransformOverrideSummary {
                target: target.clone(),
                label: None,
                position: if has_pos { Some(pos) } else { None },
                rotation: if has_rot { Some(rot) } else { None },
                scale: if has_scale { Some(scale) } else { None },
                euler_hint: if has_euler { Some(euler) } else { None },
            });
        }
    }

    let mut bulk_overrides = Vec::new();
    let mut bulk_keys: HashSet<String> = HashSet::new();
    for ((prop, val), targets) in &bulk_overrides_map {
        if targets.len() >= 3 {
            bulk_overrides.push(BulkPropertyOverride {
                property_path: prop.clone(),
                value: val.clone(),
                target_count: targets.len(),
                target_source_file_ids: targets.clone(),
            });
            bulk_keys.insert(prop.clone());
        }
    }
    key_overrides.retain(|k| !bulk_keys.contains(&k.property_path));

    let mut key_seen: HashSet<(String, String)> = HashSet::new();
    key_overrides.retain(|k| {
        let key = (k.property_path.clone(), k.value.clone().unwrap_or_default());
        key_seen.insert(key)
    });

    let renderer_overrides: Vec<RendererOverrideSummary> = renderer_overrides_map
        .into_iter()
        .map(|(target, ovs)| RendererOverrideSummary {
            target,
            label: None,
            overrides: ovs,
        })
        .collect();

    OverrideSummary {
        instance_name: ir.instance_name.clone().unwrap_or_else(|| "?".to_string()),
        source_prefab_guid: ir.source_prefab_guid,
        source_prefab_path: source_path,
        total_override_count: ir.property_overrides.len(),
        stripped_ref_count: stripped_count,
        removed_component_count: ir.removed_components.len(),
        transform_overrides,
        bulk_overrides,
        renderer_overrides,
        key_overrides,
        child_prefab_names,
        detail_file_id: ir.local_file_id,
    }
}

pub fn format_override_summary(summary: &OverrideSummary) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "=== PrefabInstance: {} ===\n",
        summary.instance_name
    ));
    if let Some(ref path) = summary.source_prefab_path {
        out.push_str(&format!("Source: {}\n", path));
    } else {
        out.push_str(&format!(
            "Source GUID: {}\n",
            guid_to_hex(&summary.source_prefab_guid)
        ));
    }

    out.push_str(&format!(
        "Overrides: {} properties | {} stripped refs | {} removed components\n",
        summary.total_override_count, summary.stripped_ref_count, summary.removed_component_count,
    ));

    if !summary.child_prefab_names.is_empty() {
        out.push_str(&format!(
            "Child prefabs: {}\n",
            summary.child_prefab_names.join(", ")
        ));
    }

    if !summary.bulk_overrides.is_empty() {
        out.push_str("\n[Bulk overrides]\n");
        for b in &summary.bulk_overrides {
            out.push_str(&format!(
                "  {} = {} (×{} targets)\n",
                b.property_path, b.value, b.target_count
            ));
        }
    }

    if !summary.transform_overrides.is_empty() {
        out.push_str(&format!(
            "\n[Transform overrides] ({} objects)\n",
            summary.transform_overrides.len()
        ));
        let show = summary.transform_overrides.len().min(5);
        for (idx, ts) in summary.transform_overrides[..show].iter().enumerate() {
            let fallback = format!("target #{}", idx + 1);
            let label = ts.label.as_deref().unwrap_or(&fallback);
            let mut parts = Vec::new();
            if ts.position.is_some() {
                parts.push("pos");
            }
            if ts.rotation.is_some() {
                parts.push("rot");
            }
            if ts.scale.is_some() {
                parts.push("scale");
            }
            if ts.euler_hint.is_some() {
                parts.push("euler");
            }
            out.push_str(&format!("  {} → {}\n", label, parts.join("+")));
        }
        if summary.transform_overrides.len() > show {
            out.push_str(&format!(
                "  ... and {} more\n",
                summary.transform_overrides.len() - show
            ));
        }
    }

    if !summary.renderer_overrides.is_empty() {
        out.push_str(&format!(
            "\n[Renderer/Material overrides] ({} objects)\n",
            summary.renderer_overrides.len()
        ));
        for (idx, r) in summary.renderer_overrides.iter().enumerate() {
            let fallback = format!("target #{}", idx + 1);
            let label = r.label.as_deref().unwrap_or(&fallback);
            let props: Vec<&str> = r.overrides.iter().map(|(p, _)| p.as_str()).collect();
            out.push_str(&format!("  {} → {}\n", label, props.join(", ")));
        }
    }

    if !summary.key_overrides.is_empty() {
        out.push_str("\n[Key property overrides]\n");
        for k in &summary.key_overrides {
            let val = k.value.as_deref().unwrap_or("(ref)");
            out.push_str(&format!("  {} = {}\n", k.property_path, val));
        }
    }

    out
}

pub fn format_prefab_file_summary(
    docs: &[YamlDoc],
    lines: &[&str],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
) -> String {
    let irs = extract_prefab_instance_irs(docs, lines);
    let stripped = extract_stripped_mappings(docs, lines);

    if irs.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    for ir in &irs {
        let stripped_count = stripped
            .iter()
            .filter(|s| s.prefab_instance_id == ir.local_file_id)
            .count();

        let child_names: Vec<String> = irs
            .iter()
            .filter(|child| {
                child.transform_parent == Some(ir.local_file_id) || {
                    child.transform_parent.map_or(false, |tp| {
                        stripped.iter().any(|s| {
                            s.local_file_id == tp
                                && s.prefab_instance_id == ir.local_file_id
                                && (s.class_id == 4 || s.class_id == 224)
                        })
                    })
                }
            })
            .filter(|child| child.local_file_id != ir.local_file_id)
            .filter_map(|child| child.instance_name.clone())
            .collect();

        let summary = summarize_prefab_instance(ir, stripped_count, child_names, guid_resolver);
        out.push_str(&format_override_summary(&summary));
        out.push('\n');
    }

    out
}

pub fn format_prefab_instance_detail(
    ir: &PrefabInstanceIR,
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
    source_ctx: Option<&SourcePrefabContext>,
    stripped_mappings: &[crate::asset_db::types::StrippedMapping],
) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "=== Detail: {} ===\n",
        ir.instance_name.as_deref().unwrap_or("?")
    ));

    if let Some(path) = guid_resolver(&ir.source_prefab_guid) {
        out.push_str(&format!("Source: {}\n", path));
    }

    let mut override_source_ids: HashSet<i64> = ir
        .property_overrides
        .iter()
        .map(|ov| ov.target.source_file_id)
        .collect();
    for rc in &ir.removed_components {
        override_source_ids.insert(rc.target.source_file_id);
    }

    if let Some(ctx) = source_ctx {
        let mut component_to_go: HashMap<i64, i64> = HashMap::new();
        for doc in &ctx.docs {
            if let Some(go_id) = doc.m_game_object_id {
                if go_id != 0 {
                    component_to_go.insert(doc.file_id, go_id);
                }
            }
        }

        let mut modified_go_ids: HashSet<i64> = HashSet::new();
        for &src_id in &override_source_ids {
            if ctx
                .docs
                .iter()
                .any(|d| d.file_id == src_id && d.class_id == 1)
            {
                modified_go_ids.insert(src_id);
            } else if let Some(&go_id) = component_to_go.get(&src_id) {
                modified_go_ids.insert(go_id);
            }
        }

        if !ctx.tree.is_empty() {
            out.push_str("\n── Hierarchy ──\n\n");
            format_annotated_hierarchy(&mut out, &ctx.tree, &modified_go_ids, 0);
        }
    }

    let mut by_target: HashMap<PrefabSourceRef, Vec<&PropertyOverride>> = HashMap::new();
    for ov in &ir.property_overrides {
        by_target.entry(ov.target.clone()).or_default().push(ov);
    }

    let target_labels: HashMap<i64, String> = if let Some(ctx) = source_ctx {
        build_target_labels(&ctx.docs)
    } else {
        HashMap::new()
    };

    let mut targets: Vec<_> = by_target.keys().cloned().collect();
    targets.sort_by_key(|t| t.source_file_id);

    out.push_str("\n── Overrides ──\n");
    for (idx, target) in targets.iter().enumerate() {
        let ovs = &by_target[target];
        let label = target_labels
            .get(&target.source_file_id)
            .cloned()
            .unwrap_or_else(|| format!("target #{}", idx + 1));
        out.push_str(&format!("\n--- {} ---\n", label));

        let formatted = merge_override_vector_components(ovs, guid_resolver);
        out.push_str(&formatted);
    }

    // Removed components
    if !ir.removed_components.is_empty() {
        out.push_str(&format!(
            "\n--- Removed components ({}) ---\n",
            ir.removed_components.len()
        ));
        for (idx, rc) in ir.removed_components.iter().enumerate() {
            let label = target_labels
                .get(&rc.target.source_file_id)
                .cloned()
                .unwrap_or_else(|| format!("target #{}", idx + 1));
            out.push_str(&format!("  {}\n", label));
        }
    }

    let mappings: Vec<_> = stripped_mappings
        .iter()
        .filter(|mapping| mapping.prefab_instance_id == ir.local_file_id)
        .collect();
    if !mappings.is_empty() {
        out.push_str(&format!("\n--- Stripped refs ({}) ---\n", mappings.len()));
        for mapping in mappings {
            let source_label = target_labels
                .get(&mapping.source.source_file_id)
                .cloned()
                .unwrap_or_else(|| mapping.type_name.clone());
            out.push_str(&format!("  {} -> {}\n", mapping.type_name, source_label));
        }
    }

    out
}

fn build_target_labels(docs: &[YamlDoc]) -> HashMap<i64, String> {
    let go_names: HashMap<i64, &str> = docs
        .iter()
        .filter(|d| d.class_id == 1)
        .filter_map(|d| d.m_name.as_deref().map(|n| (d.file_id, n)))
        .collect();

    let mut labels = HashMap::new();
    for doc in docs {
        let label = if doc.class_id == 1 {
            format!("GO:{}", doc.m_name.as_deref().unwrap_or("?"))
        } else if let Some(go_id) = doc.m_game_object_id {
            let go_name = go_names.get(&go_id).copied().unwrap_or("?");
            format!("{}/{}", go_name, doc.type_name)
        } else {
            doc.type_name.clone()
        };
        labels.insert(doc.file_id, label);
    }
    labels
}

fn format_annotated_hierarchy(
    out: &mut String,
    nodes: &[HierarchyNode],
    modified_go_ids: &HashSet<i64>,
    depth: usize,
) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        let marker = if modified_go_ids.contains(&node.file_id) {
            " [modified]"
        } else {
            ""
        };
        out.push_str(&format!("{}{}{}\n", indent, node.name, marker));
        format_annotated_hierarchy(out, &node.children, modified_go_ids, depth + 1);
    }
}

fn merge_override_vector_components(
    ovs: &[&PropertyOverride],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < ovs.len() {
        let ov = ovs[i];
        if let Some(base) = ov.property_path.strip_suffix(".x") {
            let mut components: Vec<(&str, &str)> = vec![("x", ov.value.as_deref().unwrap_or("0"))];
            let mut j = i + 1;
            let expected = [".y", ".z", ".w"];
            let mut ei = 0;
            while j < ovs.len() && ei < expected.len() {
                let expected_path = format!("{}{}", base, expected[ei]);
                if ovs[j].property_path == expected_path {
                    components.push((
                        &expected[ei][1..], // "y", "z", "w"
                        ovs[j].value.as_deref().unwrap_or("0"),
                    ));
                    j += 1;
                    ei += 1;
                } else {
                    break;
                }
            }
            if components.len() >= 2 {
                let parts: Vec<String> = components
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, round_decimal_str(v)))
                    .collect();
                out.push_str(&format!("  {} = {{{}}}\n", base, parts.join(", ")));
                i = j;
                continue;
            }
        }
        if let Some(base) = ov.property_path.strip_suffix(".r") {
            let mut components: Vec<(&str, &str)> = vec![("r", ov.value.as_deref().unwrap_or("0"))];
            let mut j = i + 1;
            let expected = [".g", ".b", ".a"];
            let mut ei = 0;
            while j < ovs.len() && ei < expected.len() {
                let expected_path = format!("{}{}", base, expected[ei]);
                if ovs[j].property_path == expected_path {
                    components.push((&expected[ei][1..], ovs[j].value.as_deref().unwrap_or("0")));
                    j += 1;
                    ei += 1;
                } else {
                    break;
                }
            }
            if components.len() >= 2 {
                let parts: Vec<String> = components
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, round_decimal_str(v)))
                    .collect();
                out.push_str(&format!("  {} = {{{}}}\n", base, parts.join(", ")));
                i = j;
                continue;
            }
        }
        let val = ov.value.as_deref().unwrap_or("");
        let formatted_val = round_decimal_str(val);
        if let Some(ref obj) = ov.object_ref {
            if obj.guid != [0u8; 16] {
                let obj_path = guid_resolver(&obj.guid).unwrap_or_else(|| guid_to_hex(&obj.guid));
                out.push_str(&format!(
                    "  {} = {} → {{{}}}\n",
                    ov.property_path, formatted_val, obj_path
                ));
            } else {
                out.push_str(&format!("  {} = {}\n", ov.property_path, formatted_val));
            }
        } else {
            out.push_str(&format!("  {} = {}\n", ov.property_path, formatted_val));
        }
        i += 1;
    }
    out
}

/// "WoodenChair (1)" → "WoodenChair", "WoodenChair" → "WoodenChair"
pub(super) fn normalize_instance_name(name: &str) -> &str {
    if let Some(paren_start) = name.rfind(" (") {
        let rest = &name[paren_start + 2..];
        if rest.ends_with(')') {
            let digits = &rest[..rest.len() - 1];
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                return &name[..paren_start];
            }
        }
    }
    if let Some(under_idx) = name.rfind('_') {
        let digits = &name[under_idx + 1..];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return &name[..under_idx];
        }
    }
    name
}

#[cfg(test)]
const MESH_BLACKLIST_PREFIXES: &[&str] = &[
    "m_PolyMesh.",
    "normals.Array.",
    "vertices.Array.",
    "uv.Array.",
    "triangles.",
    "m_MeshFormatVersion",
    "m_Mesh.",
    "tangents.Array.",
    "colors.Array.",
    "m_BakedConvexCollisionMesh",
    "m_BakedTriangleCollisionMesh",
    "m_CompressedMesh.",
    "m_LocalAABB.",
    "m_ShapeVertices.",
    "m_BakedLightmapTag",
];

#[cfg(test)]
pub(super) fn is_mesh_data_property(prop: &str) -> bool {
    MESH_BLACKLIST_PREFIXES
        .iter()
        .any(|prefix| prop.starts_with(prefix))
}

struct GroupedHierarchyNodes<'a> {
    representative: &'a HierarchyNode,
    members: Vec<&'a HierarchyNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HierarchySummaryOptions {
    pub max_depth: Option<usize>,
    pub max_nodes: Option<usize>,
    pub query: Option<String>,
    pub component_filters: Vec<String>,
    pub path_prefix: Option<String>,
}

impl HierarchySummaryOptions {
    fn effective_max_nodes(&self) -> usize {
        self.max_nodes.unwrap_or(DEFAULT_HIERARCHY_MAX_NODES)
    }

    fn has_search_filters(&self) -> bool {
        self.query
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || self
                .component_filters
                .iter()
                .any(|value| !value.trim().is_empty())
    }

    fn has_any_option(&self) -> bool {
        self.max_depth.is_some()
            || self.max_nodes.is_some()
            || self
                .path_prefix
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            || self.has_search_filters()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HierarchySearchOptions {
    pub query: Option<String>,
    pub component_filters: Vec<String>,
    pub match_fields: Vec<String>,
    pub path_prefix: Option<String>,
    pub limit: Option<usize>,
}

impl HierarchySearchOptions {
    pub fn has_search_filters(&self) -> bool {
        self.query
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || self
                .component_filters
                .iter()
                .any(|value| !value.trim().is_empty())
    }
}

struct PreparedHierarchyFilters {
    query: Option<QueryMatcher>,
    component_filters: Vec<String>,
    match_fields: SearchMatchFields,
}

enum QueryMatcher {
    Plain(String),
    Regex(regex::Regex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchMatchFields {
    path: bool,
    name: bool,
    component: bool,
    annotation: bool,
    tag: bool,
    layer: bool,
    prefab_source: bool,
    field_name: bool,
    field_value: bool,
    explicit_labels: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SearchableSerializedFields {
    names: Vec<String>,
    values: Vec<String>,
}

impl Default for SearchMatchFields {
    fn default() -> Self {
        Self {
            path: true,
            name: true,
            component: true,
            annotation: true,
            tag: true,
            layer: true,
            prefab_source: true,
            field_name: false,
            field_value: false,
            explicit_labels: Vec::new(),
        }
    }
}

impl PreparedHierarchyFilters {
    fn from_options(options: &HierarchySummaryOptions) -> Self {
        Self::from_parts(
            options.query.as_deref(),
            options.component_filters.iter().map(String::as_str),
            std::iter::empty::<&str>(),
        )
    }

    fn from_search_options(options: &HierarchySearchOptions) -> Self {
        Self::from_parts(
            options.query.as_deref(),
            options.component_filters.iter().map(String::as_str),
            options.match_fields.iter().map(String::as_str),
        )
    }

    fn from_parts<'a>(
        query: Option<&str>,
        component_filters: impl Iterator<Item = &'a str>,
        match_fields: impl Iterator<Item = &'a str>,
    ) -> Self {
        let query = query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if let Some(pattern) = value.strip_prefix("re:").filter(|value| !value.is_empty()) {
                    regex::RegexBuilder::new(pattern)
                        .case_insensitive(true)
                        .build()
                        .map(QueryMatcher::Regex)
                        .unwrap_or_else(|_| QueryMatcher::Plain(value.to_ascii_lowercase()))
                } else {
                    QueryMatcher::Plain(value.to_ascii_lowercase())
                }
            });
        let component_filters = component_filters
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect();
        let match_fields = SearchMatchFields::from_values(match_fields);
        Self {
            query,
            component_filters,
            match_fields,
        }
    }
}

impl SearchMatchFields {
    fn from_values<'a>(values: impl Iterator<Item = &'a str>) -> Self {
        let mut parsed = SearchMatchFields::default();
        let mut saw_value = false;
        for value in values {
            for part in value.split([',', '|']) {
                let normalized = part.trim().to_ascii_lowercase().replace('-', "_");
                if normalized.is_empty() {
                    continue;
                }
                if !saw_value {
                    parsed = SearchMatchFields {
                        path: false,
                        name: false,
                        component: false,
                        annotation: false,
                        tag: false,
                        layer: false,
                        prefab_source: false,
                        field_name: false,
                        field_value: false,
                        explicit_labels: Vec::new(),
                    };
                    saw_value = true;
                }
                parsed.enable(&normalized);
            }
        }
        parsed
    }

    fn enable(&mut self, normalized: &str) {
        match normalized {
            "default" => {
                self.path = true;
                self.name = true;
                self.component = true;
                self.annotation = true;
                self.tag = true;
                self.layer = true;
                self.prefab_source = true;
                self.push_label("default");
            }
            "all" => {
                self.path = true;
                self.name = true;
                self.component = true;
                self.annotation = true;
                self.tag = true;
                self.layer = true;
                self.prefab_source = true;
                self.field_name = true;
                self.field_value = true;
                self.push_label("all");
            }
            "path" => {
                self.path = true;
                self.push_label("path");
            }
            "name" => {
                self.name = true;
                self.push_label("name");
            }
            "component" | "components" => {
                self.component = true;
                self.push_label("component");
            }
            "annotation" | "annotations" | "state" => {
                self.annotation = true;
                self.push_label("annotation");
            }
            "tag" => {
                self.tag = true;
                self.push_label("tag");
            }
            "layer" => {
                self.layer = true;
                self.push_label("layer");
            }
            "prefab" | "prefab_source" | "source_prefab" => {
                self.prefab_source = true;
                self.push_label("prefab_source");
            }
            "field" | "fields" => {
                self.field_name = true;
                self.field_value = true;
                self.push_label("field_name");
                self.push_label("field_value");
            }
            "field_name" | "field_names" | "property_name" | "property_path" => {
                self.field_name = true;
                self.push_label("field_name");
            }
            "field_value" | "field_values" | "property_value" => {
                self.field_value = true;
                self.push_label("field_value");
            }
            _ => {}
        }
    }

    fn push_label(&mut self, label: &str) {
        if !self
            .explicit_labels
            .iter()
            .any(|existing| existing == label)
        {
            self.explicit_labels.push(label.to_string());
        }
    }

    fn needs_serialized_fields(&self) -> bool {
        self.field_name || self.field_value
    }
}

#[derive(Default)]
struct HierarchyWriteState {
    printed_nodes: usize,
    hidden_by_max_nodes: usize,
}

fn component_signature(components: &[String]) -> String {
    let mut sorted = components.to_vec();
    sorted.sort();
    sorted.join(",")
}

fn format_component_suffix(node: &HierarchyNode) -> String {
    if node.components.is_empty() {
        String::new()
    } else {
        format!(" ({})", node.components.join(", "))
    }
}

fn node_structure_signature(node: &HierarchyNode, cache: &mut HashMap<i64, String>) -> String {
    if let Some(existing) = cache.get(&node.file_id) {
        return existing.clone();
    }

    let child_signatures: Vec<String> = node
        .children
        .iter()
        .map(|child| node_structure_signature(child, cache))
        .collect();

    let signature = format!(
        "name:{name}|components:{components}|annotations:{annotations}|children:[{children}]",
        name = normalize_instance_name(&node.name),
        components = component_signature(&node.components),
        annotations = format_go_annotations(node),
        children = child_signatures.join("||"),
    );
    cache.insert(node.file_id, signature.clone());
    signature
}

fn build_structure_signature_map(roots: &[HierarchyNode]) -> HashMap<i64, String> {
    let mut cache = HashMap::new();
    for root in roots {
        let _ = node_structure_signature(root, &mut cache);
    }
    cache
}

fn group_children_by_structure<'a>(
    children: &'a [HierarchyNode],
    signature_map: &HashMap<i64, String>,
) -> Vec<GroupedHierarchyNodes<'a>> {
    let mut groups: Vec<GroupedHierarchyNodes<'a>> = Vec::new();
    let mut group_map: HashMap<&str, usize> = HashMap::new();

    for child in children {
        let signature = signature_map
            .get(&child.file_id)
            .map(String::as_str)
            .unwrap_or("");
        if let Some(&idx) = group_map.get(signature) {
            groups[idx].members.push(child);
        } else {
            let idx = groups.len();
            group_map.insert(signature, idx);
            groups.push(GroupedHierarchyNodes {
                representative: child,
                members: vec![child],
            });
        }
    }

    groups
}

fn count_hierarchy_nodes(nodes: &[HierarchyNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_hierarchy_nodes(&node.children))
        .sum()
}

fn count_group_nodes(group: &GroupedHierarchyNodes<'_>) -> usize {
    group
        .members
        .iter()
        .map(|node| 1 + count_hierarchy_nodes(&node.children))
        .sum()
}

fn select_path_prefix_roots(roots: &[HierarchyNode], path_prefix: &str) -> Vec<HierarchyNode> {
    if path_prefix
        .split('/')
        .map(str::trim)
        .all(|value| value.is_empty())
    {
        return roots.to_vec();
    }

    find_hierarchy_node_by_path(roots, path_prefix)
        .cloned()
        .into_iter()
        .collect()
}

fn filter_hierarchy_nodes(
    nodes: &[HierarchyNode],
    filters: &PreparedHierarchyFilters,
    prefab_source_paths: &HashMap<i64, String>,
    path_map: &HashMap<i64, String>,
    serialized_fields: Option<&HashMap<i64, SearchableSerializedFields>>,
) -> Vec<HierarchyNode> {
    nodes
        .iter()
        .filter_map(|node| {
            filter_hierarchy_node(
                node,
                filters,
                prefab_source_paths,
                path_map,
                serialized_fields,
            )
        })
        .collect()
}

fn filter_hierarchy_node(
    node: &HierarchyNode,
    filters: &PreparedHierarchyFilters,
    prefab_source_paths: &HashMap<i64, String>,
    path_map: &HashMap<i64, String>,
    serialized_fields: Option<&HashMap<i64, SearchableSerializedFields>>,
) -> Option<HierarchyNode> {
    let path = path_map
        .get(&node.file_id)
        .map(String::as_str)
        .unwrap_or(node.name.as_str());
    let children = filter_hierarchy_nodes(
        &node.children,
        filters,
        prefab_source_paths,
        path_map,
        serialized_fields,
    );
    if node_matches_filters(node, path, filters, prefab_source_paths, serialized_fields)
        || !children.is_empty()
    {
        let mut cloned = node.clone();
        cloned.children = children;
        Some(cloned)
    } else {
        None
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle_lower)
}

fn query_matches(value: &str, matcher: &QueryMatcher) -> bool {
    match matcher {
        QueryMatcher::Plain(needle_lower) => contains_ignore_ascii_case(value, needle_lower),
        QueryMatcher::Regex(regex) => regex.is_match(value),
    }
}

fn node_layer_search_text(layer: i32) -> String {
    match layer {
        0 => "Default 0".to_string(),
        1 => "TransparentFX 1".to_string(),
        2 => "Ignore Raycast 2".to_string(),
        3 => "Layer3 3".to_string(),
        4 => "Water 4".to_string(),
        5 => "UI 5".to_string(),
        6 => "Layer6 6".to_string(),
        7 => "Layer7 7".to_string(),
        _ => layer.to_string(),
    }
}

fn serialized_field_matches(
    file_id: i64,
    filters: &PreparedHierarchyFilters,
    query: &QueryMatcher,
    serialized_fields: Option<&HashMap<i64, SearchableSerializedFields>>,
) -> bool {
    if !filters.match_fields.needs_serialized_fields() {
        return false;
    }

    let Some(fields) = serialized_fields.and_then(|fields| fields.get(&file_id)) else {
        return false;
    };

    (filters.match_fields.field_name && fields.names.iter().any(|name| query_matches(name, query)))
        || (filters.match_fields.field_value
            && fields
                .values
                .iter()
                .any(|value| query_matches(value, query)))
}

fn node_matches_filters(
    node: &HierarchyNode,
    path: &str,
    filters: &PreparedHierarchyFilters,
    prefab_source_paths: &HashMap<i64, String>,
    serialized_fields: Option<&HashMap<i64, SearchableSerializedFields>>,
) -> bool {
    let query_matches = if let Some(query) = &filters.query {
        (filters.match_fields.path && query_matches(path, query))
            || (filters.match_fields.name && query_matches(&node.name, query))
            || (filters.match_fields.component
                && node
                    .components
                    .iter()
                    .any(|component| query_matches(component, query)))
            || (filters.match_fields.annotation
                && query_matches(&format_go_annotations(node), query))
            || (filters.match_fields.tag
                && node
                    .tag
                    .as_deref()
                    .is_some_and(|tag| query_matches(tag, query)))
            || (filters.match_fields.layer
                && node
                    .layer
                    .is_some_and(|layer| query_matches(&node_layer_search_text(layer), query)))
            || (filters.match_fields.prefab_source
                && prefab_source_paths
                    .get(&node.file_id)
                    .is_some_and(|path| query_matches(path, query)))
            || serialized_field_matches(node.file_id, filters, query, serialized_fields)
    } else {
        true
    };

    if !query_matches {
        return false;
    }

    if filters.component_filters.is_empty() {
        return true;
    }

    node.components.iter().any(|component| {
        filters
            .component_filters
            .iter()
            .any(|filter| contains_ignore_ascii_case(component, filter))
    })
}

fn apply_hierarchy_options(
    roots: &[HierarchyNode],
    options: &HierarchySummaryOptions,
    prefab_source_paths: &HashMap<i64, String>,
) -> Vec<HierarchyNode> {
    let scoped_roots = options
        .path_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|path| select_path_prefix_roots(roots, path))
        .unwrap_or_else(|| roots.to_vec());

    if !options.has_search_filters() {
        return scoped_roots;
    }

    let path_map = build_hierarchy_path_map(roots);
    let filters = PreparedHierarchyFilters::from_options(options);
    filter_hierarchy_nodes(
        &scoped_roots,
        &filters,
        prefab_source_paths,
        &path_map,
        None,
    )
}

fn option_summary_line(options: &HierarchySummaryOptions) -> Option<String> {
    if !options.has_any_option() {
        return None;
    }

    let mut parts = Vec::new();
    if let Some(path_prefix) = options
        .path_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("path_prefix=\"{}\"", path_prefix));
    }
    if let Some(query) = options
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("query=\"{}\"", query));
    }

    let component_filters: Vec<&str> = options
        .component_filters
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if !component_filters.is_empty() {
        parts.push(format!(
            "component_filter=\"{}\"",
            component_filters.join(",")
        ));
    }

    if let Some(max_depth) = options.max_depth {
        parts.push(format!("max_depth={}", max_depth));
    }
    if let Some(max_nodes) = options.max_nodes {
        parts.push(format!("max_nodes={}", max_nodes));
    }

    Some(format!("Hierarchy filters: {}", parts.join(", ")))
}

fn path_needs_hint(node: &HierarchyNode, path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment != node.name)
}

fn format_node_label(node: &HierarchyNode, collapsed: bool, path: Option<&str>) -> String {
    let name = if collapsed {
        normalize_instance_name(&node.name)
    } else {
        node.name.as_str()
    };
    let mut base = format!(
        "{}{}{}",
        name,
        format_component_suffix(node),
        format_go_annotations(node)
    );
    if !collapsed {
        if let Some(path) = path.filter(|path| path_needs_hint(node, path)) {
            base.push_str(&format!("  {{object_path: {}}}", path));
        }
    }
    base
}

fn format_instance_sample(members: &[&HierarchyNode], path_map: &HashMap<i64, String>) -> String {
    const SAMPLE_LIMIT: usize = 5;

    let sample: Vec<String> = members
        .iter()
        .take(SAMPLE_LIMIT)
        .map(|node| {
            path_map
                .get(&node.file_id)
                .cloned()
                .unwrap_or_else(|| node.name.clone())
        })
        .collect();

    if members.len() <= SAMPLE_LIMIT {
        sample.join(", ")
    } else {
        format!(
            "{}, ... +{}",
            sample.join(", "),
            members.len() - SAMPLE_LIMIT
        )
    }
}

fn write_grouped_nodes(
    out: &mut String,
    nodes: &[HierarchyNode],
    prefix: &str,
    top_level: bool,
    logical_depth: usize,
    signature_map: &HashMap<i64, String>,
    options: &HierarchySummaryOptions,
    path_map: &HashMap<i64, String>,
    state: &mut HierarchyWriteState,
) {
    let groups = group_children_by_structure(nodes, signature_map);
    for (idx, group) in groups.iter().enumerate() {
        if state.printed_nodes >= options.effective_max_nodes() {
            state.hidden_by_max_nodes += count_group_nodes(group);
            continue;
        }

        let is_last = idx + 1 == groups.len();
        let line_prefix = tree_line_prefix(prefix, is_last, top_level);
        let child_prefix = tree_child_prefix(prefix, is_last, top_level);
        let metadata_prefix = format!("{}  ", child_prefix);
        let representative = group.representative;
        state.printed_nodes += 1;

        if group.members.len() > 1 {
            out.push_str(&line_prefix);
            out.push_str(&format_node_label(representative, true, None));
            out.push_str(&format!(" ×{}\n", group.members.len()));

            out.push_str(&metadata_prefix);
            out.push_str("Instances: ");
            out.push_str(&format_instance_sample(&group.members, path_map));
            out.push('\n');

            if !representative.children.is_empty() {
                if options
                    .max_depth
                    .is_some_and(|max_depth| logical_depth >= max_depth)
                {
                    let hidden: usize = group
                        .members
                        .iter()
                        .map(|node| count_hierarchy_nodes(&node.children))
                        .sum();
                    out.push_str(&metadata_prefix);
                    out.push_str(&format!(
                        "... ({} child nodes hidden by max_depth)\n",
                        hidden
                    ));
                } else {
                    out.push_str(&metadata_prefix);
                    out.push_str("Shared subtree:\n");
                    let shared_prefix = format!("{}  ", child_prefix);
                    write_grouped_nodes(
                        out,
                        &representative.children,
                        &shared_prefix,
                        false,
                        logical_depth + 1,
                        signature_map,
                        options,
                        path_map,
                        state,
                    );
                }
            }
            continue;
        }

        out.push_str(&line_prefix);
        out.push_str(&format_node_label(
            representative,
            false,
            path_map.get(&representative.file_id).map(String::as_str),
        ));
        out.push('\n');

        if !representative.children.is_empty() {
            if options
                .max_depth
                .is_some_and(|max_depth| logical_depth >= max_depth)
            {
                out.push_str(&metadata_prefix);
                out.push_str(&format!(
                    "... ({} child nodes hidden by max_depth)\n",
                    count_hierarchy_nodes(&representative.children)
                ));
            } else {
                write_grouped_nodes(
                    out,
                    &representative.children,
                    &child_prefix,
                    false,
                    logical_depth + 1,
                    signature_map,
                    options,
                    path_map,
                    state,
                );
            }
        }
    }
}

fn tree_line_prefix(prefix: &str, is_last: bool, top_level: bool) -> String {
    if top_level {
        String::new()
    } else if is_last {
        format!("{}└─ ", prefix)
    } else {
        format!("{}├─ ", prefix)
    }
}

fn tree_child_prefix(prefix: &str, is_last: bool, top_level: bool) -> String {
    if top_level {
        String::new()
    } else if is_last {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    }
}

pub fn format_hierarchy_summary(roots: &[HierarchyNode]) -> String {
    format_hierarchy_summary_with_options(roots, &HierarchySummaryOptions::default())
}

pub fn format_hierarchy_summary_with_options(
    roots: &[HierarchyNode],
    options: &HierarchySummaryOptions,
) -> String {
    let prefab_source_paths = HashMap::new();
    format_hierarchy_summary_with_metadata(roots, options, &prefab_source_paths)
}

fn format_hierarchy_summary_with_metadata(
    roots: &[HierarchyNode],
    options: &HierarchySummaryOptions,
    prefab_source_paths: &HashMap<i64, String>,
) -> String {
    let mut out = String::new();
    let scoped_roots = apply_hierarchy_options(roots, options, prefab_source_paths);
    if scoped_roots.is_empty() {
        out.push_str("No hierarchy nodes matched filters.\n");
        return out;
    }

    let path_map = build_hierarchy_path_map(roots);
    let signature_map = build_structure_signature_map(&scoped_roots);
    let mut state = HierarchyWriteState::default();
    write_grouped_nodes(
        &mut out,
        &scoped_roots,
        "",
        true,
        1,
        &signature_map,
        options,
        &path_map,
        &mut state,
    );
    if state.hidden_by_max_nodes > 0 {
        out.push_str(&format!(
            "... ({} hierarchy nodes hidden by max_nodes)\n",
            state.hidden_by_max_nodes
        ));
    }
    out
}

fn search_options_summary_line(options: &HierarchySearchOptions) -> String {
    let mut parts = Vec::new();
    if let Some(path_prefix) = options
        .path_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("path_prefix=\"{}\"", path_prefix));
    }
    if let Some(query) = options
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("query=\"{}\"", query));
    }
    let component_filters: Vec<&str> = options
        .component_filters
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if !component_filters.is_empty() {
        parts.push(format!(
            "component_filter=\"{}\"",
            component_filters.join(",")
        ));
    }
    if !options.match_fields.is_empty() {
        let match_fields: Vec<&str> = options
            .match_fields
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        if !match_fields.is_empty() {
            parts.push(format!("match_fields=\"{}\"", match_fields.join(",")));
        }
    }
    if let Some(limit) = options.limit {
        parts.push(format!("limit={}", limit));
    }

    if parts.is_empty() {
        "Search filters: none".to_string()
    } else {
        format!("Search filters: {}", parts.join(", "))
    }
}

fn collect_matching_nodes<'a>(
    nodes: &'a [HierarchyNode],
    filters: &PreparedHierarchyFilters,
    prefab_source_paths: &HashMap<i64, String>,
    path_map: &HashMap<i64, String>,
    serialized_fields: Option<&HashMap<i64, SearchableSerializedFields>>,
    out: &mut Vec<&'a HierarchyNode>,
) {
    for node in nodes {
        let path = path_map
            .get(&node.file_id)
            .map(String::as_str)
            .unwrap_or(node.name.as_str());
        if node_matches_filters(node, path, filters, prefab_source_paths, serialized_fields) {
            out.push(node);
        }
        collect_matching_nodes(
            &node.children,
            filters,
            prefab_source_paths,
            path_map,
            serialized_fields,
            out,
        );
    }
}

fn format_search_result_node(
    out: &mut String,
    node: &HierarchyNode,
    path_map: &HashMap<i64, String>,
) {
    let path = path_map
        .get(&node.file_id)
        .map(String::as_str)
        .unwrap_or(node.name.as_str());
    out.push_str("- ");
    out.push_str(path);
    out.push_str(&format_component_suffix(node));
    out.push_str(&format_go_annotations(node));
    out.push('\n');
}

fn build_serialized_field_search_index(
    docs: &[YamlDoc],
    lines: &[&str],
) -> HashMap<i64, SearchableSerializedFields> {
    let mut index: HashMap<i64, SearchableSerializedFields> = HashMap::new();
    for doc in docs {
        let go_file_id = if doc.class_id == 1 {
            Some(doc.file_id)
        } else {
            doc.m_game_object_id
        };
        let Some(go_file_id) = go_file_id else {
            continue;
        };

        let entry = index.entry(go_file_id).or_default();
        collect_doc_serialized_search_fields(doc, lines, entry);
    }
    index
}

fn collect_doc_serialized_search_fields(
    doc: &YamlDoc,
    lines: &[&str],
    entry: &mut SearchableSerializedFields,
) {
    let start = (doc.line_start + 2).min(doc.line_end);
    for line in lines.iter().take(doc.line_end).skip(start) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("---") || trimmed.starts_with('%') {
            continue;
        }

        if let Some((name, value)) = parse_serialized_search_field_line(trimmed) {
            if let Some(name) = name.filter(|value| !value.is_empty()) {
                entry.names.push(name.to_string());
            }
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                entry.values.push(value.to_string());
            }
        }
    }
}

fn parse_serialized_search_field_line(line: &str) -> Option<(Option<&str>, Option<&str>)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let list_value = line.strip_prefix("- ").map(str::trim);
    let candidate = list_value.unwrap_or(line);
    if let Some(colon) = candidate.find(':') {
        let name = candidate[..colon].trim();
        let value = candidate[colon + 1..].trim();
        let value = value
            .trim_matches(',')
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        return Some((Some(name), Some(value)));
    }

    list_value.map(|value| (None, Some(value)))
}

pub fn format_hierarchy_search_results(
    roots: &[HierarchyNode],
    docs: &[YamlDoc],
    lines: &[&str],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
    file_path: &str,
    options: &HierarchySearchOptions,
) -> String {
    let irs = if docs.iter().any(|d| d.class_id == 1001 && !d.is_stripped) {
        extract_prefab_instance_irs(docs, lines)
    } else {
        Vec::new()
    };
    let prefab_source_paths: HashMap<i64, String> = irs
        .iter()
        .filter_map(|ir| guid_resolver(&ir.source_prefab_guid).map(|path| (ir.local_file_id, path)))
        .collect();
    let scoped_roots = options
        .path_prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|path| select_path_prefix_roots(roots, path))
        .unwrap_or_else(|| roots.to_vec());
    let path_map = build_hierarchy_path_map(roots);
    let filters = PreparedHierarchyFilters::from_search_options(options);
    let serialized_fields =
        if filters.query.is_some() && filters.match_fields.needs_serialized_fields() {
            Some(build_serialized_field_search_index(docs, lines))
        } else {
            None
        };
    let mut matches = Vec::new();
    collect_matching_nodes(
        &scoped_roots,
        &filters,
        &prefab_source_paths,
        &path_map,
        serialized_fields.as_ref(),
        &mut matches,
    );

    let mut out = String::new();
    out.push_str(&format!("Search: {}\n", file_path));
    out.push_str(&search_options_summary_line(options));
    out.push('\n');
    out.push_str(&format!("Total matches: {}\n", matches.len()));

    if matches.is_empty() {
        out.push_str("\nNo hierarchy nodes matched filters.\n");
        return out;
    }

    let limit = options.limit.unwrap_or(50);
    let shown = matches.len().min(limit);
    out.push_str(&format!("Showing: 1-{}\n\n", shown));
    for node in matches.iter().take(limit) {
        format_search_result_node(&mut out, node, &path_map);
    }
    if matches.len() > shown {
        out.push_str(&format!(
            "\n... ({} more matches hidden by limit)\n",
            matches.len() - shown
        ));
    }
    out
}

pub fn format_scene_summary(
    roots: &[HierarchyNode],
    docs: &[YamlDoc],
    lines: &[&str],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
    file_path: &str,
) -> String {
    format_scene_summary_with_options(
        roots,
        docs,
        lines,
        guid_resolver,
        file_path,
        &HierarchySummaryOptions::default(),
    )
}

pub fn format_scene_summary_with_options(
    roots: &[HierarchyNode],
    docs: &[YamlDoc],
    lines: &[&str],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
    file_path: &str,
    options: &HierarchySummaryOptions,
) -> String {
    let mut out = String::new();
    let has_prefab_instances = docs.iter().any(|d| d.class_id == 1001 && !d.is_stripped);
    let irs = if has_prefab_instances {
        extract_prefab_instance_irs(docs, lines)
    } else {
        Vec::new()
    };

    // ── A. Scene Summary ──
    let unique_sources: HashSet<Guid> = irs.iter().map(|ir| ir.source_prefab_guid).collect();
    let file_kind = if file_path.to_ascii_lowercase().ends_with(".prefab") {
        "Prefab"
    } else {
        "Scene"
    };
    out.push_str(&format!("{}: {}\n", file_kind, file_path));
    out.push_str(&format!("Top-level objects: {}\n", roots.len()));
    if !irs.is_empty() {
        out.push_str(&format!(
            "Unique prefab sources: {}\n",
            unique_sources.len()
        ));
        out.push_str(&format!("Total prefab instances: {}\n", irs.len()));
    }
    if let Some(summary_line) = option_summary_line(options) {
        out.push_str(&summary_line);
        out.push('\n');
    }

    // ── B. Hierarchy ──
    out.push_str("\n── Hierarchy ──\n\n");
    let prefab_source_paths: HashMap<i64, String> = irs
        .iter()
        .filter_map(|ir| guid_resolver(&ir.source_prefab_guid).map(|path| (ir.local_file_id, path)))
        .collect();
    out.push_str(&format_hierarchy_summary_with_metadata(
        roots,
        options,
        &prefab_source_paths,
    ));

    if !irs.is_empty() {
        out.push_str("\nDrill down with object_path:\n");
        out.push_str("- Use paths from the hierarchy or Instances lines for object_path\n");
        out.push_str("- PrefabInstance targets return structured override detail\n");
    }
    if irs.is_empty() {
        out.push_str("\nDrill down with object_path:\n");
        out.push_str("- Use paths from the hierarchy for object_path\n");
    }

    out
}
