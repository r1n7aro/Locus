use std::collections::{HashMap, HashSet};

use crate::asset_db::types::{
    guid_to_hex, BulkPropertyOverride, Guid, KeyOverride, OverrideSummary, PrefabInstanceIR,
    PrefabSourceRef, PropertyOverride, RendererOverrideSummary, TransformOverrideSummary,
};

use super::parser::{format_go_annotations, round_decimal_str, HierarchyNode, YamlDoc};
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
        for ts in &summary.transform_overrides[..show] {
            let fallback = format!("fileID:{}", ts.target.source_file_id);
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
        for r in &summary.renderer_overrides {
            let fallback = format!("fileID:{}", r.target.source_file_id);
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
) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "=== Detail: {} (fileID: {}) ===\n",
        ir.instance_name.as_deref().unwrap_or("?"),
        ir.local_file_id
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
    for target in &targets {
        let ovs = &by_target[target];
        let label = target_labels
            .get(&target.source_file_id)
            .map(|l| format!(" ({})", l))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n--- target fileID:{}{} ---\n",
            target.source_file_id, label,
        ));

        let formatted = merge_override_vector_components(ovs, guid_resolver);
        out.push_str(&formatted);
    }

    // Removed components
    if !ir.removed_components.is_empty() {
        out.push_str(&format!(
            "\n--- Removed components ({}) ---\n",
            ir.removed_components.len()
        ));
        for rc in &ir.removed_components {
            let label = target_labels
                .get(&rc.target.source_file_id)
                .map(|l| format!(" ({})", l))
                .unwrap_or_default();
            out.push_str(&format!("  fileID:{}{}\n", rc.target.source_file_id, label,));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverrideClass {
    Clean,
    TransformOnly,
    LightCustom,
    Heavy,
}

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

const CLEAN_PROPS: &[&str] = &["m_Name", "m_IsActive"];

pub(super) fn is_mesh_data_property(prop: &str) -> bool {
    MESH_BLACKLIST_PREFIXES
        .iter()
        .any(|prefix| prop.starts_with(prefix))
}

fn is_transform_prop(prop: &str) -> bool {
    TRANSFORM_PROPS.iter().any(|p| prop.starts_with(p))
}

fn is_clean_prop(prop: &str) -> bool {
    CLEAN_PROPS.iter().any(|p| prop == *p)
}

fn classify_instance_overrides(ir: &PrefabInstanceIR) -> (OverrideClass, usize, bool) {
    let total = ir.property_overrides.len();
    let mut mesh_count = 0usize;
    let mut transform_count = 0usize;
    let mut clean_count = 0usize;
    let mut has_scale = false;

    for ov in &ir.property_overrides {
        let pp = &ov.property_path;
        if is_mesh_data_property(pp) {
            mesh_count += 1;
        } else if is_transform_prop(pp) {
            transform_count += 1;
            if pp.starts_with("m_LocalScale.") {
                has_scale = true;
            }
        } else if is_clean_prop(pp) {
            clean_count += 1;
        }
    }

    let non_trivial = total.saturating_sub(transform_count + clean_count + mesh_count);

    let class = if !ir.removed_components.is_empty() || mesh_count > 0 || total >= 30 {
        OverrideClass::Heavy
    } else if non_trivial == 0 {
        if transform_count == 0 {
            OverrideClass::Clean
        } else {
            OverrideClass::TransformOnly
        }
    } else {
        OverrideClass::LightCustom
    };

    (class, mesh_count, has_scale)
}

struct PrefabSourceStats {
    source_path: Option<String>,
    #[allow(dead_code)]
    source_guid: Guid,
    instance_count: usize,
    transform_only_count: usize,
    clean_count: usize,
    #[allow(dead_code)]
    has_scale_count: usize,
    light_custom_count: usize,
    heavy_count: usize,
    total_override_max: usize,
    mesh_override_total: usize,
    has_removed_components: bool,
}

fn aggregate_prefab_sources(
    irs: &[PrefabInstanceIR],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
) -> Vec<PrefabSourceStats> {
    let mut order: Vec<Guid> = Vec::new();
    let mut by_source: HashMap<Guid, Vec<&PrefabInstanceIR>> = HashMap::new();
    for ir in irs {
        let entry = by_source.entry(ir.source_prefab_guid).or_default();
        if entry.is_empty() {
            order.push(ir.source_prefab_guid);
        }
        entry.push(ir);
    }

    let mut result: Vec<PrefabSourceStats> = order
        .into_iter()
        .map(|guid| {
            let instances = &by_source[&guid];
            let source_path = guid_resolver(&guid);
            let mut transform_only = 0usize;
            let mut clean = 0usize;
            let mut has_scale_cnt = 0usize;
            let mut light_custom = 0usize;
            let mut heavy = 0usize;
            let mut max_overrides = 0usize;
            let mut mesh_total = 0usize;
            let mut has_removed = false;

            for ir in instances {
                let (class, mesh_count, scale) = classify_instance_overrides(ir);
                match class {
                    OverrideClass::Clean => clean += 1,
                    OverrideClass::TransformOnly => transform_only += 1,
                    OverrideClass::LightCustom => light_custom += 1,
                    OverrideClass::Heavy => heavy += 1,
                }
                if scale {
                    has_scale_cnt += 1;
                }
                if ir.property_overrides.len() > max_overrides {
                    max_overrides = ir.property_overrides.len();
                }
                mesh_total += mesh_count;
                if !ir.removed_components.is_empty() {
                    has_removed = true;
                }
            }

            PrefabSourceStats {
                source_path,
                source_guid: guid,
                instance_count: instances.len(),
                transform_only_count: transform_only,
                clean_count: clean,
                has_scale_count: has_scale_cnt,
                light_custom_count: light_custom,
                heavy_count: heavy,
                total_override_max: max_overrides,
                mesh_override_total: mesh_total,
                has_removed_components: has_removed,
            }
        })
        .collect();

    result.sort_by(|a, b| {
        let a_notable = (a.heavy_count > 0 || a.mesh_override_total > 0) as u8;
        let b_notable = (b.heavy_count > 0 || b.mesh_override_total > 0) as u8;
        b_notable
            .cmp(&a_notable)
            .then(b.total_override_max.cmp(&a.total_override_max))
    });

    result
}

struct GroupedHierarchyNodes<'a> {
    representative: &'a HierarchyNode,
    members: Vec<&'a HierarchyNode>,
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

fn format_node_label(node: &HierarchyNode, collapsed: bool) -> String {
    let name = if collapsed {
        normalize_instance_name(&node.name)
    } else {
        node.name.as_str()
    };
    format!(
        "{}{}{}",
        name,
        format_component_suffix(node),
        format_go_annotations(node)
    )
}

fn format_instance_sample(members: &[&HierarchyNode]) -> String {
    const SAMPLE_LIMIT: usize = 5;

    let sample: Vec<&str> = members
        .iter()
        .take(SAMPLE_LIMIT)
        .map(|node| node.name.as_str())
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
    depth: usize,
    signature_map: &HashMap<i64, String>,
) {
    for group in group_children_by_structure(nodes, signature_map) {
        let indent = "  ".repeat(depth);
        let representative = group.representative;

        if group.members.len() > 1 {
            out.push_str(&indent);
            out.push_str(&format_node_label(representative, true));
            out.push_str(&format!(" ×{}\n", group.members.len()));

            out.push_str(&indent);
            out.push_str("  Instances: ");
            out.push_str(&format_instance_sample(&group.members));
            out.push('\n');

            if !representative.children.is_empty() {
                out.push_str(&indent);
                out.push_str("  Shared subtree:\n");
                write_grouped_nodes(out, &representative.children, depth + 2, signature_map);
            }
            continue;
        }

        out.push_str(&indent);
        out.push_str(&format_node_label(representative, false));
        out.push('\n');

        if !representative.children.is_empty() {
            write_grouped_nodes(out, &representative.children, depth + 1, signature_map);
        }
    }
}

pub fn format_hierarchy_summary(roots: &[HierarchyNode]) -> String {
    let mut out = String::new();
    let signature_map = build_structure_signature_map(roots);
    write_grouped_nodes(&mut out, roots, 0, &signature_map);
    out
}

fn short_prefab_name(path: &str) -> &str {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let filename = filename.rsplit('\\').next().unwrap_or(filename);
    filename.strip_suffix(".prefab").unwrap_or(filename)
}

pub fn format_scene_summary(
    roots: &[HierarchyNode],
    docs: &[YamlDoc],
    lines: &[&str],
    guid_resolver: &dyn Fn(&Guid) -> Option<String>,
    file_path: &str,
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

    // ── B. Hierarchy ──
    out.push_str("\n── Hierarchy ──\n\n");
    out.push_str(&format_hierarchy_summary(roots));

    // ── C. Notable Prefab Overrides ──
    if !irs.is_empty() {
        let aggregated = aggregate_prefab_sources(&irs, guid_resolver);

        let notable: Vec<&PrefabSourceStats> = aggregated
            .iter()
            .filter(|s| s.heavy_count > 0 || s.light_custom_count > 0 || s.has_removed_components)
            .collect();

        if !notable.is_empty() {
            out.push_str("\n── Notable Prefab Overrides ──\n\n");
            for stats in &notable {
                let name = stats
                    .source_path
                    .as_deref()
                    .map(short_prefab_name)
                    .unwrap_or("unknown");

                let mut desc = Vec::new();

                if stats.mesh_override_total > 0 {
                    desc.push(format!(
                        "{} mesh-data overrides ⚠",
                        stats.mesh_override_total
                    ));
                }

                if stats.heavy_count > 0 {
                    if stats.instance_count == 1 {
                        desc.push(format!("{} overrides", stats.total_override_max));
                    } else {
                        desc.push(format!(
                            "{} heavy instance(s), max {} overrides",
                            stats.heavy_count, stats.total_override_max
                        ));
                    }
                }

                if stats.light_custom_count > 0 {
                    desc.push(format!("{} custom-edited", stats.light_custom_count));
                }

                if stats.has_removed_components {
                    desc.push("removed components".to_string());
                }

                if stats.instance_count > 1 {
                    out.push_str(&format!(
                        "- {} ×{}: {}\n",
                        name,
                        stats.instance_count,
                        desc.join(", ")
                    ));
                } else {
                    out.push_str(&format!("- {}: {}\n", name, desc.join(", ")));
                }
            }
        }

        // ── D. Suppressed ──
        let transform_only_total: usize = aggregated.iter().map(|s| s.transform_only_count).sum();
        let clean_total: usize = aggregated.iter().map(|s| s.clean_count).sum();

        out.push_str("\n── Suppressed ──\n\n");
        if transform_only_total > 0 {
            out.push_str(&format!(
                "- {} transform-only prefab instances\n",
                transform_only_total
            ));
        }
        if clean_total > 0 {
            out.push_str(&format!(
                "- {} clean (name/active-only) prefab instances\n",
                clean_total
            ));
        }
        out.push_str("- Per-instance transform details\n");
        out.push_str("- Per-instance source path repeats\n");
        if aggregated.iter().any(|s| s.mesh_override_total > 0) {
            out.push_str("- Raw mesh/poly/normals override entries\n");
        }

        // ── E. Drill-down hints ──
        out.push_str("\nDrill down with object_path:\n");
        out.push_str("- \"InstanceName\" → structured override detail for a PrefabInstance\n");
        out.push_str("- \"Parent/Child\" → GameObject components\n");
        out.push_str(
            "- Use exact names from \"Instances\" lines when a folded group has duplicates\n",
        );
    }

    out
}
