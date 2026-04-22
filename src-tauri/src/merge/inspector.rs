use std::collections::{BTreeSet, HashMap, HashSet};

use crate::asset_db::types::{guid_to_hex, PrefabInstanceIR};
use crate::asset_db::AssetDbState;
use crate::diff::content::BatchBlobReader;
use crate::diff::context::{GuidResolver, SideContext, SideFileSource, SourceMode};
use crate::diff::profiler::DiffProfiler;
use crate::diff::semantic::component_inference::{
    infer_component_from_override_groups, inference_for_known_class_id,
};
use crate::diff::semantic::inspector::{apply_field_label_enhancements, HIDDEN_ASSET_FIELDS};
use crate::diff::semantic::scene::{
    build_override_field_map, build_scene_side_data, collect_scene_components,
    load_source_prefab_info, prefab_instance_is_unchanged, prefab_panel_meta, prefab_panel_title,
    scene_root_docs, PrefabInstanceLocalInfo, SourcePrefabInfo,
};
use crate::diff::semantic::script::{
    doc_script_guid, load_script_semantic_info, resolve_all_field_types, ScriptSemanticInfo,
};
use crate::diff::semantic::{
    build_doc_label_map, doc_type_label, resolve_script_class_name, unity_class_name,
    HierarchyEntry, SemanticBuildEnv,
};
use crate::diff::types::{
    InspectorComponentInference, InspectorPanelKind, SemanticBadgeCounts, SemanticTreeNode,
    UnityAssetKind,
};
use crate::error::{AppError, AppResult};
use crate::unity_yaml::YamlDoc;

use super::three_way::{
    build_merge_fields, change_kind_for_status, collect_conflict_field_ids,
    count_merge_leaf_states, doc_merge_status, match_asset_docs_three_way,
};
use super::types::*;

struct MergeSideContexts<'a> {
    base: SideContext<'a>,
    ours: SideContext<'a>,
    theirs: SideContext<'a>,
}

struct BuiltTarget {
    target_id: String,
    summary: MergeTargetSummary,
    inspector: MergeTargetInspector,
    locator: MergeTargetLocator,
    conflict_ids: Vec<String>,
    parent_file_id: Option<i64>,
    label: String,
    path: String,
    object_kind: String,
    changed_component_panels: usize,
    conflict_count: usize,
    auto_count: usize,
}

pub(crate) struct SessionBuildOutput {
    pub(crate) summary: MergeSummary,
    pub(crate) tree: Vec<SemanticTreeNode>,
    pub(crate) targets: Vec<MergeTargetSummary>,
    pub(crate) inspectors: HashMap<String, MergeTargetInspector>,
    pub(crate) target_locators: HashMap<String, MergeTargetLocator>,
    pub(crate) conflict_field_ids: HashSet<String>,
}

fn merge_side_contexts<'a>(ref_graph_state: &'a AssetDbState) -> MergeSideContexts<'a> {
    let build_side = |stage| SideContext {
        guid_resolver: GuidResolver::Workspace(ref_graph_state),
        script_guid_resolver: GuidResolver::Workspace(ref_graph_state),
        source_mode: SourceMode::Snapshot,
        file_source: SideFileSource::GitStage(stage),
    };

    MergeSideContexts {
        base: build_side(1),
        ours: build_side(2),
        theirs: build_side(3),
    }
}

fn scene_doc_key(file_id: i64) -> String {
    format!("sceneDoc:{}", file_id)
}

fn asset_doc_key(match_key: &str) -> String {
    format!("assetDoc:{}", match_key)
}

fn preferred_script_info<'a>(
    base: &'a Option<ScriptSemanticInfo>,
    ours: &'a Option<ScriptSemanticInfo>,
    theirs: &'a Option<ScriptSemanticInfo>,
) -> Option<&'a ScriptSemanticInfo> {
    ours.as_ref().or(theirs.as_ref()).or(base.as_ref())
}

fn hidden_roots_for_panel(
    panel_kind: &InspectorPanelKind,
    script_infos: &[&Option<ScriptSemanticInfo>],
) -> HashSet<String> {
    let mut hidden_roots = HashSet::new();
    if matches!(
        panel_kind,
        InspectorPanelKind::AssetRoot
            | InspectorPanelKind::SubObject
            | InspectorPanelKind::Component
    ) {
        hidden_roots.extend(HIDDEN_ASSET_FIELDS.iter().map(|value| value.to_string()));
        for info in script_infos.iter().filter_map(|info| info.as_ref()) {
            for (alias, field) in &info.field_aliases {
                if field.hidden {
                    hidden_roots.insert(alias.clone());
                    hidden_roots.insert(field.canonical_name.clone());
                }
            }
        }
    }
    hidden_roots
}

fn filter_relevant_paths(
    base_map: &HashMap<String, crate::diff::semantic::ParsedFieldLine>,
    ours_map: &HashMap<String, crate::diff::semantic::ParsedFieldLine>,
    theirs_map: &HashMap<String, crate::diff::semantic::ParsedFieldLine>,
    hidden_roots: &HashSet<String>,
) -> Vec<String> {
    let mut all_paths: Vec<String> = base_map
        .keys()
        .chain(ours_map.keys())
        .chain(theirs_map.keys())
        .cloned()
        .collect();
    all_paths.sort_unstable();
    all_paths.dedup();

    if hidden_roots.is_empty() {
        return all_paths;
    }

    all_paths
        .into_iter()
        .filter(|path| {
            let root_field = path.split('.').next().unwrap_or(path);
            let root_field = root_field.split('[').next().unwrap_or(root_field);
            !hidden_roots.contains(root_field)
        })
        .collect()
}

fn merge_component_meta(
    panel_kind: &InspectorPanelKind,
    class_id: Option<i32>,
    script_class: Option<&str>,
) -> (Option<String>, Option<String>) {
    let component_type = class_id.map(|id| {
        if id == 114 {
            script_class.unwrap_or("MonoBehaviour").to_string()
        } else {
            unity_class_name(id).to_string()
        }
    });
    let component_source = Some(
        match panel_kind {
            InspectorPanelKind::Component => {
                if class_id == Some(114) {
                    "script"
                } else {
                    "builtin"
                }
            }
            InspectorPanelKind::GameObjectHeader => "gameObjectHeader",
            InspectorPanelKind::AssetRoot => "assetRoot",
            InspectorPanelKind::SubObject => "subObject",
        }
        .to_string(),
    );
    (component_type, component_source)
}

fn apply_prefab_merge_component_inference(
    source_file_id: i64,
    base_overrides: &[&crate::asset_db::types::PropertyOverride],
    ours_overrides: &[&crate::asset_db::types::PropertyOverride],
    theirs_overrides: &[&crate::asset_db::types::PropertyOverride],
    panel_kind: &mut InspectorPanelKind,
    class_id: Option<i32>,
    component_type: &mut Option<String>,
    component_source: &mut Option<String>,
) -> Option<InspectorComponentInference> {
    if component_source.as_deref() == Some("modelImporterMetaHeuristic") {
        return class_id.map(|cid| {
            inference_for_known_class_id(
                cid,
                "legacyShortFileId",
                vec![format!("fileID:{}", source_file_id)],
            )
            .to_inspector_inference()
        });
    }

    if class_id.is_some() || component_type.is_some() {
        return None;
    }

    let inferred =
        infer_component_from_override_groups(&[base_overrides, ours_overrides, theirs_overrides])?;
    *panel_kind = if inferred.component_type == "GameObject" {
        InspectorPanelKind::GameObjectHeader
    } else {
        InspectorPanelKind::Component
    };
    *component_type = Some(inferred.component_type.clone());
    *component_source = Some("inferred".into());
    Some(inferred.to_inspector_inference())
}

fn build_merge_doc_panel(
    doc_key: &str,
    panel_kind: InspectorPanelKind,
    title: String,
    script_class: Option<String>,
    base_doc: Option<&YamlDoc>,
    ours_doc: Option<&YamlDoc>,
    theirs_doc: Option<&YamlDoc>,
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    base_labels: &HashMap<i64, String>,
    ours_labels: &HashMap<i64, String>,
    theirs_labels: &HashMap<i64, String>,
    contexts: &MergeSideContexts<'_>,
    env: &mut SemanticBuildEnv<'_>,
    class_id: Option<i32>,
) -> (Option<MergePanel>, usize, usize, Vec<String>) {
    let base_script_info =
        base_doc.and_then(|doc| load_script_semantic_info(doc, base_lines, &contexts.base, env));
    let ours_script_info =
        ours_doc.and_then(|doc| load_script_semantic_info(doc, ours_lines, &contexts.ours, env));
    let theirs_script_info = theirs_doc
        .and_then(|doc| load_script_semantic_info(doc, theirs_lines, &contexts.theirs, env));

    let mut base_map = base_doc
        .map(|doc| parse_doc_field_map(doc, base_lines, &contexts.base, base_labels))
        .unwrap_or_default();
    let mut ours_map = ours_doc
        .map(|doc| parse_doc_field_map(doc, ours_lines, &contexts.ours, ours_labels))
        .unwrap_or_default();
    let mut theirs_map = theirs_doc
        .map(|doc| parse_doc_field_map(doc, theirs_lines, &contexts.theirs, theirs_labels))
        .unwrap_or_default();

    apply_field_label_enhancements(&mut base_map, base_script_info.as_ref());
    apply_field_label_enhancements(&mut ours_map, ours_script_info.as_ref());
    apply_field_label_enhancements(&mut theirs_map, theirs_script_info.as_ref());

    let hidden_roots = hidden_roots_for_panel(
        &panel_kind,
        &[&base_script_info, &ours_script_info, &theirs_script_info],
    );
    let relevant_paths = filter_relevant_paths(&base_map, &ours_map, &theirs_map, &hidden_roots);

    let script_info =
        preferred_script_info(&base_script_info, &ours_script_info, &theirs_script_info);
    let hint_script_path = ours_doc
        .and_then(|doc| {
            doc_script_guid(doc, ours_lines)
                .and_then(|guid| contexts.ours.resolve_script_guid_path(&guid))
        })
        .or_else(|| {
            theirs_doc.and_then(|doc| {
                doc_script_guid(doc, theirs_lines)
                    .and_then(|guid| contexts.theirs.resolve_script_guid_path(&guid))
            })
        })
        .or_else(|| {
            base_doc.and_then(|doc| {
                doc_script_guid(doc, base_lines)
                    .and_then(|guid| contexts.base.resolve_script_guid_path(&guid))
            })
        });

    let field_type_map = resolve_all_field_types(
        base_map
            .keys()
            .chain(ours_map.keys())
            .chain(theirs_map.keys()),
        script_info,
        hint_script_path.as_deref(),
        &contexts.ours,
        env,
    );

    let fields = build_merge_fields(
        doc_key,
        relevant_paths,
        &base_map,
        &ours_map,
        &theirs_map,
        &field_type_map,
    );
    let (conflict_count, auto_count) = count_merge_leaf_states(&fields);
    let status = doc_merge_status(
        base_doc.is_some(),
        ours_doc.is_some(),
        theirs_doc.is_some(),
        conflict_count,
        auto_count,
    );

    let mut conflict_ids = Vec::new();
    collect_conflict_field_ids(&fields, &mut conflict_ids);

    let panel = if fields.is_empty() && status == DocMergeStatus::Unchanged {
        None
    } else {
        let panel_title = match class_id {
            Some(114) => script_class.clone().unwrap_or(title),
            _ => title,
        };
        let (component_type, component_source) =
            merge_component_meta(&panel_kind, class_id, script_class.as_deref());
        Some(MergePanel {
            panel_kind,
            title: panel_title,
            script_class,
            component_type,
            component_source,
            component_inference: None,
            merge_status: status,
            fields,
        })
    };

    (panel, conflict_count, auto_count, conflict_ids)
}

fn fixed_header_leaf(
    doc_key: &str,
    property_path: &str,
    label: &str,
    base: Option<String>,
    ours: Option<String>,
    theirs: Option<String>,
) -> Option<MergeField> {
    let (merge_state, auto_choice, result) =
        super::three_way::auto_merge_field(base.as_deref(), ours.as_deref(), theirs.as_deref());
    if merge_state == MergeState::Unchanged {
        return None;
    }
    Some(MergeField {
        id: format!("{}|{}", doc_key, property_path),
        property_path: property_path.to_string(),
        label: label.to_string(),
        value_type: "string".to_string(),
        base,
        ours,
        theirs,
        result,
        merge_state,
        auto_choice,
        manual_choice: None,
        children: Vec::new(),
        field_type: None,
        reference_base: None,
        reference_ours: None,
        reference_theirs: None,
    })
}

fn build_gameobject_header_merge_panel(
    file_id: i64,
    base_doc: Option<&YamlDoc>,
    ours_doc: Option<&YamlDoc>,
    theirs_doc: Option<&YamlDoc>,
) -> (Option<MergePanel>, usize, usize, Vec<String>) {
    let doc_key = scene_doc_key(file_id);
    let boolish =
        |value: Option<i32>| value.map(|v| if v != 0 { "true" } else { "false" }.to_string());
    let num = |value: Option<i64>| value.map(|v| v.to_string());
    let num_i32 = |value: Option<i32>| value.map(|v| v.to_string());

    let mut fields = Vec::new();
    if let Some(field) = fixed_header_leaf(
        &doc_key,
        "m_Name",
        "Name",
        base_doc.and_then(|doc| doc.m_name.clone()),
        ours_doc.and_then(|doc| doc.m_name.clone()),
        theirs_doc.and_then(|doc| doc.m_name.clone()),
    ) {
        fields.push(field);
    }
    if let Some(field) = fixed_header_leaf(
        &doc_key,
        "m_TagString",
        "Tag",
        base_doc.and_then(|doc| doc.m_tag_string.clone()),
        ours_doc.and_then(|doc| doc.m_tag_string.clone()),
        theirs_doc.and_then(|doc| doc.m_tag_string.clone()),
    ) {
        fields.push(field);
    }
    if let Some(field) = fixed_header_leaf(
        &doc_key,
        "m_Layer",
        "Layer",
        base_doc.and_then(|doc| num_i32(doc.m_layer)),
        ours_doc.and_then(|doc| num_i32(doc.m_layer)),
        theirs_doc.and_then(|doc| num_i32(doc.m_layer)),
    ) {
        fields.push(field);
    }
    if let Some(field) = fixed_header_leaf(
        &doc_key,
        "m_IsActive",
        "Active",
        base_doc.and_then(|doc| boolish(doc.m_is_active)),
        ours_doc.and_then(|doc| boolish(doc.m_is_active)),
        theirs_doc.and_then(|doc| boolish(doc.m_is_active)),
    ) {
        fields.push(field);
    }
    if let Some(field) = fixed_header_leaf(
        &doc_key,
        "m_StaticEditorFlags",
        "Static",
        base_doc.and_then(|doc| num(doc.m_static_editor_flags)),
        ours_doc.and_then(|doc| num(doc.m_static_editor_flags)),
        theirs_doc.and_then(|doc| num(doc.m_static_editor_flags)),
    ) {
        fields.push(field);
    }

    let (conflict_count, auto_count) = count_merge_leaf_states(&fields);
    let status = doc_merge_status(
        base_doc.is_some(),
        ours_doc.is_some(),
        theirs_doc.is_some(),
        conflict_count,
        auto_count,
    );
    let mut conflict_ids = Vec::new();
    collect_conflict_field_ids(&fields, &mut conflict_ids);

    let panel = if fields.is_empty() && status == DocMergeStatus::Unchanged {
        None
    } else {
        Some(MergePanel {
            panel_kind: InspectorPanelKind::GameObjectHeader,
            title: "GameObject".to_string(),
            script_class: None,
            component_type: Some("GameObject".to_string()),
            component_source: Some("gameObjectHeader".to_string()),
            component_inference: None,
            merge_status: status,
            fields,
        })
    };

    (panel, conflict_count, auto_count, conflict_ids)
}

fn load_prefab_local_script_info(
    source_file_id: i64,
    base_local_info: Option<&PrefabInstanceLocalInfo>,
    ours_local_info: Option<&PrefabInstanceLocalInfo>,
    theirs_local_info: Option<&PrefabInstanceLocalInfo>,
    base_docs: &[YamlDoc],
    ours_docs: &[YamlDoc],
    theirs_docs: &[YamlDoc],
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    contexts: &MergeSideContexts<'_>,
    env: &mut SemanticBuildEnv<'_>,
) -> (Option<ScriptSemanticInfo>, Option<String>) {
    if let Some(target) = ours_local_info.and_then(|info| info.targets.get(&source_file_id)) {
        if let Some(doc) = ours_docs.get(target.local_doc_index) {
            let hint_script_path = doc_script_guid(doc, ours_lines)
                .and_then(|guid| contexts.ours.resolve_script_guid_path(&guid));
            return (
                load_script_semantic_info(doc, ours_lines, &contexts.ours, env),
                hint_script_path,
            );
        }
    }

    if let Some(target) = theirs_local_info.and_then(|info| info.targets.get(&source_file_id)) {
        if let Some(doc) = theirs_docs.get(target.local_doc_index) {
            let hint_script_path = doc_script_guid(doc, theirs_lines)
                .and_then(|guid| contexts.theirs.resolve_script_guid_path(&guid));
            return (
                load_script_semantic_info(doc, theirs_lines, &contexts.theirs, env),
                hint_script_path,
            );
        }
    }

    if let Some(target) = base_local_info.and_then(|info| info.targets.get(&source_file_id)) {
        if let Some(doc) = base_docs.get(target.local_doc_index) {
            let hint_script_path = doc_script_guid(doc, base_lines)
                .and_then(|guid| contexts.base.resolve_script_guid_path(&guid));
            return (
                load_script_semantic_info(doc, base_lines, &contexts.base, env),
                hint_script_path,
            );
        }
    }

    (None, None)
}

fn build_removed_prefab_component_merge_panel(
    file_id: i64,
    base_ir: Option<&PrefabInstanceIR>,
    ours_ir: Option<&PrefabInstanceIR>,
    theirs_ir: Option<&PrefabInstanceIR>,
    source_info: Option<&SourcePrefabInfo>,
    base_local_info: Option<&PrefabInstanceLocalInfo>,
    ours_local_info: Option<&PrefabInstanceLocalInfo>,
    theirs_local_info: Option<&PrefabInstanceLocalInfo>,
) -> (Option<MergePanel>, usize, usize, Vec<String>) {
    let base_removed: HashSet<i64> = base_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();
    let ours_removed: HashSet<i64> = ours_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();
    let theirs_removed: HashSet<i64> = theirs_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();

    if base_removed.is_empty() && ours_removed.is_empty() && theirs_removed.is_empty() {
        return (None, 0, 0, Vec::new());
    }

    let all_removed: BTreeSet<i64> = base_removed
        .iter()
        .chain(ours_removed.iter())
        .chain(theirs_removed.iter())
        .copied()
        .collect();

    let doc_key = format!("{}:removed", scene_doc_key(file_id));
    let mut fields = Vec::new();
    for fid in all_removed {
        let base = base_removed.contains(&fid).then(|| "Removed".to_string());
        let ours = ours_removed.contains(&fid).then(|| "Removed".to_string());
        let theirs = theirs_removed.contains(&fid).then(|| "Removed".to_string());
        let (merge_state, auto_choice, result) =
            super::three_way::auto_merge_field(base.as_deref(), ours.as_deref(), theirs.as_deref());
        if merge_state == MergeState::Unchanged {
            continue;
        }

        fields.push(MergeField {
            id: format!("{}|removedComponent:{}", doc_key, fid),
            property_path: format!("removedComponent:{}", fid),
            label: resolved_prefab_panel_title(
                file_id,
                fid,
                source_info,
                base_local_info,
                ours_local_info,
                theirs_local_info,
            ),
            value_type: "string".into(),
            base,
            ours,
            theirs,
            result,
            merge_state,
            auto_choice,
            manual_choice: None,
            children: Vec::new(),
            field_type: None,
            reference_base: None,
            reference_ours: None,
            reference_theirs: None,
        });
    }

    let (conflict_count, auto_count) = count_merge_leaf_states(&fields);
    let status = doc_merge_status(
        base_ir.is_some(),
        ours_ir.is_some(),
        theirs_ir.is_some(),
        conflict_count,
        auto_count,
    );
    let mut conflict_ids = Vec::new();
    collect_conflict_field_ids(&fields, &mut conflict_ids);

    let panel = if fields.is_empty() && status == DocMergeStatus::Unchanged {
        None
    } else {
        Some(MergePanel {
            panel_kind: InspectorPanelKind::SubObject,
            title: "Removed Components".into(),
            script_class: None,
            component_type: None,
            component_source: Some("subObject".into()),
            component_inference: None,
            merge_status: status,
            fields,
        })
    };

    (panel, conflict_count, auto_count, conflict_ids)
}

fn resolved_prefab_panel_title(
    prefab_instance_file_id: i64,
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    base_local_info: Option<&PrefabInstanceLocalInfo>,
    ours_local_info: Option<&PrefabInstanceLocalInfo>,
    theirs_local_info: Option<&PrefabInstanceLocalInfo>,
) -> String {
    let title = prefab_panel_title(
        source_file_id,
        source_info,
        ours_local_info.or(theirs_local_info),
        base_local_info,
    );
    if title.starts_with("Component (fileID:") {
        let base_has_target = base_local_info
            .and_then(|info| info.targets.get(&source_file_id))
            .is_some();
        let ours_has_target = ours_local_info
            .and_then(|info| info.targets.get(&source_file_id))
            .is_some();
        let theirs_has_target = theirs_local_info
            .and_then(|info| info.targets.get(&source_file_id))
            .is_some();
        eprintln!(
            "[merge/prefab] failed to resolve component title: prefab_instance_file_id={} source_file_id={} source_prefab_loaded={} local_target(base/ours/theirs)={}/{}/{}",
            prefab_instance_file_id,
            source_file_id,
            source_info.is_some(),
            base_has_target,
            ours_has_target,
            theirs_has_target
        );
    }
    title
}

fn build_prefab_instance_merge_panels(
    file_id: i64,
    base_ir: Option<&PrefabInstanceIR>,
    ours_ir: Option<&PrefabInstanceIR>,
    theirs_ir: Option<&PrefabInstanceIR>,
    source_info: Option<&SourcePrefabInfo>,
    base_local_info: Option<&PrefabInstanceLocalInfo>,
    ours_local_info: Option<&PrefabInstanceLocalInfo>,
    theirs_local_info: Option<&PrefabInstanceLocalInfo>,
    base_docs: &[YamlDoc],
    ours_docs: &[YamlDoc],
    theirs_docs: &[YamlDoc],
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    base_labels: &HashMap<i64, String>,
    ours_labels: &HashMap<i64, String>,
    theirs_labels: &HashMap<i64, String>,
    contexts: &MergeSideContexts<'_>,
    env: &mut SemanticBuildEnv<'_>,
) -> (Vec<MergePanel>, usize, usize, Vec<String>) {
    let mut base_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut ours_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut theirs_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut target_ids = BTreeSet::new();

    if let Some(ir) = base_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            base_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }
    if let Some(ir) = ours_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            ours_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }
    if let Some(ir) = theirs_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            theirs_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }

    let mut panels = Vec::new();
    let mut total_conflicts = 0usize;
    let mut total_autos = 0usize;
    let mut conflict_ids = Vec::new();

    for src_fid in target_ids {
        let base_overrides = base_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let ours_overrides = ours_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let theirs_overrides = theirs_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let mut base_map = build_override_field_map(base_overrides, &contexts.base, base_labels);
        let mut ours_map = build_override_field_map(ours_overrides, &contexts.ours, ours_labels);
        let mut theirs_map =
            build_override_field_map(theirs_overrides, &contexts.theirs, theirs_labels);
        if base_map.is_empty() && ours_map.is_empty() && theirs_map.is_empty() {
            continue;
        }

        // Three-way merge inspectors don't yet plumb a typed
        // `SourcePrefabLoadError` through their cache (the merge cache is a
        // flat `HashMap<_, Option<_>>`), so pass `None` for the error slot.
        // The semantic-diff path is the one that surfaces the precise
        // diagnostic to the inspector UI.
        let (mut panel_kind, class_id, mut component_type, mut component_source, _) =
            prefab_panel_meta(
                src_fid,
                source_info,
                None,
                ours_local_info.or(theirs_local_info),
                base_local_info,
            );
        let component_inference = apply_prefab_merge_component_inference(
            src_fid,
            base_overrides,
            ours_overrides,
            theirs_overrides,
            &mut panel_kind,
            class_id,
            &mut component_type,
            &mut component_source,
        );
        let panel_title = resolved_prefab_panel_title(
            file_id,
            src_fid,
            source_info,
            base_local_info,
            ours_local_info,
            theirs_local_info,
        );
        let script_class = (class_id == Some(114)).then(|| {
            component_type
                .clone()
                .unwrap_or_else(|| "MonoBehaviour".into())
        });
        let (script_info, hint_script_path) = if class_id == Some(114) {
            load_prefab_local_script_info(
                src_fid,
                base_local_info,
                ours_local_info,
                theirs_local_info,
                base_docs,
                ours_docs,
                theirs_docs,
                base_lines,
                ours_lines,
                theirs_lines,
                contexts,
                env,
            )
        } else {
            (None, None)
        };

        apply_field_label_enhancements(&mut base_map, script_info.as_ref());
        apply_field_label_enhancements(&mut ours_map, script_info.as_ref());
        apply_field_label_enhancements(&mut theirs_map, script_info.as_ref());

        let fields = build_merge_fields(
            &format!("{}:{}", scene_doc_key(file_id), src_fid),
            filter_relevant_paths(&base_map, &ours_map, &theirs_map, &HashSet::new()),
            &base_map,
            &ours_map,
            &theirs_map,
            &resolve_all_field_types(
                base_map
                    .keys()
                    .chain(ours_map.keys())
                    .chain(theirs_map.keys()),
                script_info.as_ref(),
                hint_script_path.as_deref(),
                &contexts.ours,
                env,
            ),
        );

        let (panel_conflicts, panel_autos) = count_merge_leaf_states(&fields);
        let status = doc_merge_status(
            !base_overrides.is_empty(),
            !ours_overrides.is_empty(),
            !theirs_overrides.is_empty(),
            panel_conflicts,
            panel_autos,
        );
        let mut panel_conflict_ids = Vec::new();
        collect_conflict_field_ids(&fields, &mut panel_conflict_ids);

        if !(fields.is_empty() && status == DocMergeStatus::Unchanged) {
            panels.push(MergePanel {
                panel_kind,
                title: panel_title,
                script_class,
                component_type,
                component_source,
                component_inference,
                merge_status: status,
                fields,
            });
        }

        total_conflicts += panel_conflicts;
        total_autos += panel_autos;
        conflict_ids.extend(panel_conflict_ids);
    }

    let (removed_panel, removed_conflicts, removed_autos, removed_conflict_ids) =
        build_removed_prefab_component_merge_panel(
            file_id,
            base_ir,
            ours_ir,
            theirs_ir,
            source_info,
            base_local_info,
            ours_local_info,
            theirs_local_info,
        );
    if let Some(panel) = removed_panel {
        panels.push(panel);
    }
    total_conflicts += removed_conflicts;
    total_autos += removed_autos;
    conflict_ids.extend(removed_conflict_ids);

    (panels, total_conflicts, total_autos, conflict_ids)
}

fn build_scene_target(
    file_id: i64,
    base_root_docs: &HashMap<i64, &YamlDoc>,
    ours_root_docs: &HashMap<i64, &YamlDoc>,
    theirs_root_docs: &HashMap<i64, &YamlDoc>,
    base_entries: &HashMap<i64, HierarchyEntry>,
    ours_entries: &HashMap<i64, HierarchyEntry>,
    theirs_entries: &HashMap<i64, HierarchyEntry>,
    base_docs: &[YamlDoc],
    ours_docs: &[YamlDoc],
    theirs_docs: &[YamlDoc],
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    base_labels: &HashMap<i64, String>,
    ours_labels: &HashMap<i64, String>,
    theirs_labels: &HashMap<i64, String>,
    contexts: &MergeSideContexts<'_>,
    env: &mut SemanticBuildEnv<'_>,
    base_prefab_irs: &HashMap<i64, PrefabInstanceIR>,
    ours_prefab_irs: &HashMap<i64, PrefabInstanceIR>,
    theirs_prefab_irs: &HashMap<i64, PrefabInstanceIR>,
    base_prefab_local_infos: &HashMap<i64, PrefabInstanceLocalInfo>,
    ours_prefab_local_infos: &HashMap<i64, PrefabInstanceLocalInfo>,
    theirs_prefab_local_infos: &HashMap<i64, PrefabInstanceLocalInfo>,
    source_prefab_cache: &mut HashMap<String, Option<SourcePrefabInfo>>,
) -> Option<BuiltTarget> {
    let base_doc = base_root_docs.get(&file_id).copied();
    let ours_doc = ours_root_docs.get(&file_id).copied();
    let theirs_doc = theirs_root_docs.get(&file_id).copied();
    if base_doc.is_none() && ours_doc.is_none() && theirs_doc.is_none() {
        return None;
    }

    let entry = base_entries
        .get(&file_id)
        .or_else(|| ours_entries.get(&file_id))
        .or_else(|| theirs_entries.get(&file_id));
    let label = entry
        .map(|entry| entry.label.clone())
        .or_else(|| {
            base_doc
                .or(ours_doc)
                .or(theirs_doc)
                .and_then(|doc| doc.m_name.clone())
        })
        .unwrap_or_else(|| format!("Object {}", file_id));
    let path = entry
        .map(|entry| entry.path.clone())
        .unwrap_or_else(|| label.clone());
    let parent_file_id = entry.and_then(|entry| entry.parent_id);
    let object_kind = entry
        .map(|entry| entry.object_kind.clone())
        .unwrap_or_else(|| "gameObject".to_string());

    let target_id = format!("go:{}", file_id);
    let mut panels = Vec::new();
    let mut conflict_ids = Vec::new();
    let mut conflict_count = 0usize;
    let mut auto_count = 0usize;
    let mut changed_component_panels = 0usize;

    if object_kind == "prefabInstance" {
        let base_ir = base_prefab_irs.get(&file_id);
        let ours_ir = ours_prefab_irs.get(&file_id);
        let theirs_ir = theirs_prefab_irs.get(&file_id);
        if prefab_instance_is_unchanged(base_ir, ours_ir)
            && prefab_instance_is_unchanged(base_ir, theirs_ir)
        {
            return None;
        }

        let base_local_info = base_prefab_local_infos.get(&file_id);
        let ours_local_info = ours_prefab_local_infos.get(&file_id);
        let theirs_local_info = theirs_prefab_local_infos.get(&file_id);
        let source_guid = ours_ir
            .map(|ir| &ir.source_prefab_guid)
            .or_else(|| theirs_ir.map(|ir| &ir.source_prefab_guid))
            .or_else(|| base_ir.map(|ir| &ir.source_prefab_guid));
        let source_info_key = source_guid.map(guid_to_hex);
        if let Some(guid) = source_guid {
            let guid_hex = guid_to_hex(guid);
            if !source_prefab_cache.contains_key(&guid_hex) {
                // The semantic-diff loader now returns a typed `Result`
                // (`SourcePrefabLoadError`) so the inspector can render a
                // precise reason. Three-way merge inspectors don't yet
                // surface that diagnostic, so collapse to `Option` here and
                // try each side in order.
                let info = load_source_prefab_info(guid, &guid_hex, &contexts.ours, env, true)
                    .ok()
                    .or_else(|| {
                        load_source_prefab_info(guid, &guid_hex, &contexts.theirs, env, true).ok()
                    })
                    .or_else(|| {
                        load_source_prefab_info(guid, &guid_hex, &contexts.base, env, false).ok()
                    });
                source_prefab_cache.insert(guid_hex, info);
            }
        }
        let source_info = source_info_key
            .as_ref()
            .and_then(|key| source_prefab_cache.get(key))
            .and_then(|opt| opt.as_ref());

        let (prefab_panels, panel_conflicts, panel_autos, ids) = build_prefab_instance_merge_panels(
            file_id,
            base_ir,
            ours_ir,
            theirs_ir,
            source_info,
            base_local_info,
            ours_local_info,
            theirs_local_info,
            base_docs,
            ours_docs,
            theirs_docs,
            base_lines,
            ours_lines,
            theirs_lines,
            base_labels,
            ours_labels,
            theirs_labels,
            contexts,
            env,
        );
        changed_component_panels += prefab_panels
            .iter()
            .filter(|panel| {
                matches!(
                    panel.panel_kind,
                    InspectorPanelKind::Component | InspectorPanelKind::SubObject
                ) && panel.merge_status != DocMergeStatus::Unchanged
            })
            .count();
        panels.extend(prefab_panels);
        conflict_ids.extend(ids);
        conflict_count += panel_conflicts;
        auto_count += panel_autos;
    } else {
        if base_doc.map(|doc| doc.class_id) == Some(1)
            || ours_doc.map(|doc| doc.class_id) == Some(1)
            || theirs_doc.map(|doc| doc.class_id) == Some(1)
        {
            let (panel, panel_conflicts, panel_autos, ids) =
                build_gameobject_header_merge_panel(file_id, base_doc, ours_doc, theirs_doc);
            if let Some(panel) = panel {
                panels.push(panel);
            }
            conflict_ids.extend(ids);
            conflict_count += panel_conflicts;
            auto_count += panel_autos;
        }

        let base_components =
            collect_scene_components(base_docs, base_lines, file_id, &contexts.base, env, None);
        let ours_components =
            collect_scene_components(ours_docs, ours_lines, file_id, &contexts.ours, env, None);
        let theirs_components = collect_scene_components(
            theirs_docs,
            theirs_lines,
            file_id,
            &contexts.theirs,
            env,
            None,
        );

        let mut base_by_key = HashMap::new();
        for (key, title, script_class, doc) in base_components {
            base_by_key.insert(key, (title, script_class, doc));
        }
        let mut ours_by_key = HashMap::new();
        for (key, title, script_class, doc) in ours_components {
            ours_by_key.insert(key, (title, script_class, doc));
        }
        let mut theirs_by_key = HashMap::new();
        for (key, title, script_class, doc) in theirs_components {
            theirs_by_key.insert(key, (title, script_class, doc));
        }

        let mut component_keys: Vec<String> = base_by_key
            .keys()
            .chain(ours_by_key.keys())
            .chain(theirs_by_key.keys())
            .cloned()
            .collect();
        component_keys.sort_unstable();
        component_keys.dedup();

        for component_key in component_keys {
            let base_component = base_by_key.get(&component_key);
            let ours_component = ours_by_key.get(&component_key);
            let theirs_component = theirs_by_key.get(&component_key);
            let title = ours_component
                .map(|(title, _, _)| title.clone())
                .or_else(|| theirs_component.map(|(title, _, _)| title.clone()))
                .or_else(|| base_component.map(|(title, _, _)| title.clone()))
                .unwrap_or_else(|| component_key.clone());
            let script_class = ours_component
                .and_then(|(_, script_class, _)| script_class.clone())
                .or_else(|| theirs_component.and_then(|(_, script_class, _)| script_class.clone()))
                .or_else(|| base_component.and_then(|(_, script_class, _)| script_class.clone()));
            let chosen_doc = ours_component
                .map(|(_, _, doc)| *doc)
                .or_else(|| theirs_component.map(|(_, _, doc)| *doc))
                .or_else(|| base_component.map(|(_, _, doc)| *doc));
            let Some(chosen_doc) = chosen_doc else {
                continue;
            };
            let doc_key = scene_doc_key(chosen_doc.file_id);
            let (panel, panel_conflicts, panel_autos, ids) = build_merge_doc_panel(
                &doc_key,
                InspectorPanelKind::Component,
                title,
                script_class,
                base_component.map(|(_, _, doc)| *doc),
                ours_component.map(|(_, _, doc)| *doc),
                theirs_component.map(|(_, _, doc)| *doc),
                base_lines,
                ours_lines,
                theirs_lines,
                base_labels,
                ours_labels,
                theirs_labels,
                contexts,
                env,
                Some(chosen_doc.class_id),
            );
            if let Some(panel) = panel {
                if !panel.fields.is_empty() || panel.merge_status != DocMergeStatus::Unchanged {
                    changed_component_panels += 1;
                }
                panels.push(panel);
            }
            conflict_ids.extend(ids);
            conflict_count += panel_conflicts;
            auto_count += panel_autos;
        }

        if panels.is_empty() {
            let chosen_doc = ours_doc.or(theirs_doc).or(base_doc)?;
            let doc_key = scene_doc_key(chosen_doc.file_id);
            let title = doc_type_label(chosen_doc, ours_lines, &contexts.ours, env);
            let (panel, panel_conflicts, panel_autos, ids) = build_merge_doc_panel(
                &doc_key,
                InspectorPanelKind::SubObject,
                title,
                None,
                base_doc,
                ours_doc,
                theirs_doc,
                base_lines,
                ours_lines,
                theirs_lines,
                base_labels,
                ours_labels,
                theirs_labels,
                contexts,
                env,
                Some(chosen_doc.class_id),
            );
            if let Some(panel) = panel {
                panels.push(panel);
            }
            conflict_ids.extend(ids);
            conflict_count += panel_conflicts;
            auto_count += panel_autos;
        }
    }

    let status = doc_merge_status(
        base_doc.is_some(),
        ours_doc.is_some(),
        theirs_doc.is_some(),
        conflict_count,
        auto_count,
    );
    if status == DocMergeStatus::Unchanged && panels.is_empty() {
        return None;
    }

    Some(BuiltTarget {
        target_id: target_id.clone(),
        summary: MergeTargetSummary {
            id: target_id.clone(),
            label: label.clone(),
            path: path.clone(),
            merge_status: status,
            conflict_count,
            auto_resolved_count: auto_count,
        },
        inspector: MergeTargetInspector {
            target_id,
            title: label.clone(),
            path: path.clone(),
            panels,
        },
        locator: MergeTargetLocator::SceneTarget { file_id },
        conflict_ids,
        parent_file_id,
        label,
        path,
        object_kind,
        changed_component_panels,
        conflict_count,
        auto_count,
    })
}

fn build_asset_target(
    match_key: &str,
    base_doc: Option<&YamlDoc>,
    ours_doc: Option<&YamlDoc>,
    theirs_doc: Option<&YamlDoc>,
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    base_labels: &HashMap<i64, String>,
    ours_labels: &HashMap<i64, String>,
    theirs_labels: &HashMap<i64, String>,
    contexts: &MergeSideContexts<'_>,
    env: &mut SemanticBuildEnv<'_>,
) -> Option<BuiltTarget> {
    let chosen_doc = ours_doc.or(theirs_doc).or(base_doc)?;
    let script_class = if chosen_doc.class_id == 114 {
        ours_doc
            .and_then(|doc| resolve_script_class_name(doc, ours_lines, &contexts.ours, env))
            .or_else(|| {
                theirs_doc.and_then(|doc| {
                    resolve_script_class_name(doc, theirs_lines, &contexts.theirs, env)
                })
            })
            .or_else(|| {
                base_doc
                    .and_then(|doc| resolve_script_class_name(doc, base_lines, &contexts.base, env))
            })
    } else {
        None
    };
    let label = chosen_doc
        .m_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| doc_type_label(chosen_doc, ours_lines, &contexts.ours, env));
    let target_id = format!("doc:{}", match_key);
    let doc_key = asset_doc_key(match_key);

    let (panel, conflict_count, auto_count, conflict_ids) = build_merge_doc_panel(
        &doc_key,
        InspectorPanelKind::AssetRoot,
        doc_type_label(chosen_doc, ours_lines, &contexts.ours, env),
        script_class,
        base_doc,
        ours_doc,
        theirs_doc,
        base_lines,
        ours_lines,
        theirs_lines,
        base_labels,
        ours_labels,
        theirs_labels,
        contexts,
        env,
        Some(chosen_doc.class_id),
    );
    let status = doc_merge_status(
        base_doc.is_some(),
        ours_doc.is_some(),
        theirs_doc.is_some(),
        conflict_count,
        auto_count,
    );
    if status == DocMergeStatus::Unchanged && panel.is_none() {
        return None;
    }

    Some(BuiltTarget {
        target_id: target_id.clone(),
        summary: MergeTargetSummary {
            id: target_id.clone(),
            label: label.clone(),
            path: label.clone(),
            merge_status: status,
            conflict_count,
            auto_resolved_count: auto_count,
        },
        inspector: MergeTargetInspector {
            target_id,
            title: label.clone(),
            path: label.clone(),
            panels: panel.into_iter().collect(),
        },
        locator: MergeTargetLocator::AssetTarget {
            match_key: match_key.to_string(),
        },
        conflict_ids,
        parent_file_id: None,
        label: label.clone(),
        path: label,
        object_kind: "assetDoc".to_string(),
        changed_component_panels: 1,
        conflict_count,
        auto_count,
    })
}

fn scene_badges(
    status: DocMergeStatus,
    conflict_count: usize,
    auto_count: usize,
    component_panels: usize,
) -> SemanticBadgeCounts {
    let mut badges = SemanticBadgeCounts::default();
    match status {
        DocMergeStatus::AddedOurs | DocMergeStatus::AddedTheirs => badges.added = 1,
        DocMergeStatus::RemovedOurs | DocMergeStatus::RemovedTheirs => badges.removed = 1,
        DocMergeStatus::HasConflicts | DocMergeStatus::AutoResolved => {
            badges.modified = conflict_count + auto_count;
        }
        DocMergeStatus::Unchanged => {}
    }
    badges.components_changed = component_panels;
    badges
}

pub(crate) fn build_merge_session_targets(
    cwd: &str,
    asset_kind: &UnityAssetKind,
    base_docs: &[YamlDoc],
    ours_docs: &[YamlDoc],
    theirs_docs: &[YamlDoc],
    base_lines: &[String],
    ours_lines: &[String],
    theirs_lines: &[String],
    ref_graph_state: &AssetDbState,
    profiler: &mut DiffProfiler,
) -> AppResult<SessionBuildOutput> {
    let contexts = merge_side_contexts(ref_graph_state);
    let mut env = SemanticBuildEnv {
        app_handle: None,
        cwd,
        profiler,
        batch_reader: BatchBlobReader::new(cwd),
        script_cache: Default::default(),
    };

    let mut built_targets = Vec::new();

    match asset_kind {
        UnityAssetKind::Scene | UnityAssetKind::Prefab => {
            let base_root_docs = scene_root_docs(base_docs);
            let ours_root_docs = scene_root_docs(ours_docs);
            let theirs_root_docs = scene_root_docs(theirs_docs);

            let base_side = build_scene_side_data(base_docs, base_lines, &contexts.base, &mut env);
            let ours_side = build_scene_side_data(ours_docs, ours_lines, &contexts.ours, &mut env);
            let theirs_side =
                build_scene_side_data(theirs_docs, theirs_lines, &contexts.theirs, &mut env);
            let mut source_prefab_cache: HashMap<String, Option<SourcePrefabInfo>> = HashMap::new();

            let mut target_ids: Vec<i64> = base_side
                .entries
                .keys()
                .chain(ours_side.entries.keys())
                .chain(theirs_side.entries.keys())
                .copied()
                .collect();
            target_ids.sort_unstable();
            target_ids.dedup();
            target_ids.sort_by_key(|file_id| {
                base_side
                    .entries
                    .get(file_id)
                    .map(|entry| entry.order)
                    .or_else(|| ours_side.entries.get(file_id).map(|entry| entry.order))
                    .or_else(|| theirs_side.entries.get(file_id).map(|entry| entry.order))
                    .unwrap_or(usize::MAX)
            });

            for file_id in target_ids {
                if let Some(target) = build_scene_target(
                    file_id,
                    &base_root_docs,
                    &ours_root_docs,
                    &theirs_root_docs,
                    &base_side.entries,
                    &ours_side.entries,
                    &theirs_side.entries,
                    base_docs,
                    ours_docs,
                    theirs_docs,
                    base_lines,
                    ours_lines,
                    theirs_lines,
                    &base_side.labels,
                    &ours_side.labels,
                    &theirs_side.labels,
                    &contexts,
                    &mut env,
                    &base_side.prefab_irs,
                    &ours_side.prefab_irs,
                    &theirs_side.prefab_irs,
                    &base_side.prefab_local_infos,
                    &ours_side.prefab_local_infos,
                    &theirs_side.prefab_local_infos,
                    &mut source_prefab_cache,
                ) {
                    built_targets.push(target);
                }
            }
        }
        _ => {
            let base_labels = build_doc_label_map(
                base_docs,
                &HashMap::new(),
                base_lines,
                &contexts.base,
                &mut env,
            );
            let ours_labels = build_doc_label_map(
                ours_docs,
                &HashMap::new(),
                ours_lines,
                &contexts.ours,
                &mut env,
            );
            let theirs_labels = build_doc_label_map(
                theirs_docs,
                &HashMap::new(),
                theirs_lines,
                &contexts.theirs,
                &mut env,
            );

            for matched_doc in match_asset_docs_three_way(base_docs, ours_docs, theirs_docs) {
                if let Some(target) = build_asset_target(
                    &matched_doc.key,
                    matched_doc.base_doc,
                    matched_doc.ours_doc,
                    matched_doc.theirs_doc,
                    base_lines,
                    ours_lines,
                    theirs_lines,
                    &base_labels,
                    &ours_labels,
                    &theirs_labels,
                    &contexts,
                    &mut env,
                ) {
                    built_targets.push(target);
                }
            }
        }
    }

    let mut summary = MergeSummary::default();
    let mut conflict_field_ids = HashSet::new();
    let inspectors = HashMap::new();
    let mut target_locators = HashMap::new();
    let targets: Vec<MergeTargetSummary> = built_targets
        .iter()
        .map(|target| target.summary.clone())
        .collect();

    for target in &built_targets {
        summary.total_conflicts += target.conflict_count;
        summary.total_auto_resolved += target.auto_count;
        match target.summary.merge_status {
            DocMergeStatus::HasConflicts => summary.conflicting_targets += 1,
            DocMergeStatus::AutoResolved
            | DocMergeStatus::AddedOurs
            | DocMergeStatus::AddedTheirs
            | DocMergeStatus::RemovedOurs
            | DocMergeStatus::RemovedTheirs => summary.auto_resolved_targets += 1,
            DocMergeStatus::Unchanged => {}
        }
        for field_id in &target.conflict_ids {
            conflict_field_ids.insert(field_id.clone());
        }
        target_locators.insert(target.target_id.clone(), target.locator.clone());
    }
    summary.total_targets = targets.len();

    let target_id_set: HashSet<String> = built_targets
        .iter()
        .map(|target| target.target_id.clone())
        .collect();
    let mut child_ids_by_target: HashMap<String, Vec<String>> = HashMap::new();
    for target in &built_targets {
        if let Some(parent_file_id) = target.parent_file_id {
            let parent_target_id = format!("go:{}", parent_file_id);
            if target_id_set.contains(&parent_target_id) {
                child_ids_by_target
                    .entry(parent_target_id)
                    .or_default()
                    .push(target.target_id.clone());
            }
        }
    }

    let tree = built_targets
        .into_iter()
        .map(|target| SemanticTreeNode {
            id: target.target_id.clone(),
            parent_id: target
                .parent_file_id
                .map(|parent_file_id| format!("go:{}", parent_file_id))
                .filter(|parent_id| target_id_set.contains(parent_id)),
            label: target.label,
            object_kind: target.object_kind,
            change_kind: change_kind_for_status(target.summary.merge_status),
            path: target.path,
            child_ids: child_ids_by_target
                .remove(&target.target_id)
                .unwrap_or_default(),
            badge_counts: scene_badges(
                target.summary.merge_status,
                target.conflict_count,
                target.auto_count,
                target.changed_component_panels,
            ),
            has_inspector: true,
        })
        .collect();

    Ok(SessionBuildOutput {
        summary,
        tree,
        targets,
        inspectors,
        target_locators,
        conflict_field_ids,
    })
}

fn merge_target_not_found_error(session: &MergeSemanticSession, target_id: &str) -> AppError {
    let available = session
        .targets
        .iter()
        .map(|target| target.id.clone())
        .collect::<Vec<_>>();
    AppError::new(
        "merge.target_not_found",
        format!("Merge semantic target '{}' was not found", target_id),
    )
    .detail(format!("available targets: {}", available.join(", ")))
}

fn build_scene_target_from_session<'a>(
    session: &MergeSemanticSession,
    file_id: i64,
    contexts: &MergeSideContexts<'a>,
    env: &mut SemanticBuildEnv<'a>,
) -> Option<BuiltTarget> {
    let base_root_docs = scene_root_docs(&session.base_docs);
    let ours_root_docs = scene_root_docs(&session.ours_docs);
    let theirs_root_docs = scene_root_docs(&session.theirs_docs);

    let base_side =
        build_scene_side_data(&session.base_docs, &session.base_lines, &contexts.base, env);
    let ours_side =
        build_scene_side_data(&session.ours_docs, &session.ours_lines, &contexts.ours, env);
    let theirs_side = build_scene_side_data(
        &session.theirs_docs,
        &session.theirs_lines,
        &contexts.theirs,
        env,
    );
    let mut source_prefab_cache: HashMap<String, Option<SourcePrefabInfo>> = HashMap::new();

    build_scene_target(
        file_id,
        &base_root_docs,
        &ours_root_docs,
        &theirs_root_docs,
        &base_side.entries,
        &ours_side.entries,
        &theirs_side.entries,
        &session.base_docs,
        &session.ours_docs,
        &session.theirs_docs,
        &session.base_lines,
        &session.ours_lines,
        &session.theirs_lines,
        &base_side.labels,
        &ours_side.labels,
        &theirs_side.labels,
        contexts,
        env,
        &base_side.prefab_irs,
        &ours_side.prefab_irs,
        &theirs_side.prefab_irs,
        &base_side.prefab_local_infos,
        &ours_side.prefab_local_infos,
        &theirs_side.prefab_local_infos,
        &mut source_prefab_cache,
    )
}

fn build_asset_target_from_session<'a>(
    session: &MergeSemanticSession,
    match_key: &str,
    contexts: &MergeSideContexts<'a>,
    env: &mut SemanticBuildEnv<'a>,
) -> Option<BuiltTarget> {
    let base_labels = build_doc_label_map(
        &session.base_docs,
        &HashMap::new(),
        &session.base_lines,
        &contexts.base,
        env,
    );
    let ours_labels = build_doc_label_map(
        &session.ours_docs,
        &HashMap::new(),
        &session.ours_lines,
        &contexts.ours,
        env,
    );
    let theirs_labels = build_doc_label_map(
        &session.theirs_docs,
        &HashMap::new(),
        &session.theirs_lines,
        &contexts.theirs,
        env,
    );

    let matched_doc =
        match_asset_docs_three_way(&session.base_docs, &session.ours_docs, &session.theirs_docs)
            .into_iter()
            .find(|doc| doc.key == match_key)?;

    build_asset_target(
        match_key,
        matched_doc.base_doc,
        matched_doc.ours_doc,
        matched_doc.theirs_doc,
        &session.base_lines,
        &session.ours_lines,
        &session.theirs_lines,
        &base_labels,
        &ours_labels,
        &theirs_labels,
        contexts,
        env,
    )
}

fn rebuild_merge_target_from_locator(
    cwd: &str,
    session: &MergeSemanticSession,
    target_id: &str,
    ref_graph_state: &AssetDbState,
) -> AppResult<BuiltTarget> {
    let locator = session
        .target_locators
        .get(target_id)
        .cloned()
        .ok_or_else(|| merge_target_not_found_error(session, target_id))?;

    let contexts = merge_side_contexts(ref_graph_state);
    let mut profiler = DiffProfiler::new(format!("merge-target:{}", target_id), true, false);
    let mut env = SemanticBuildEnv {
        app_handle: None,
        cwd,
        profiler: &mut profiler,
        batch_reader: BatchBlobReader::new(cwd),
        script_cache: Default::default(),
    };

    let built = match locator {
        MergeTargetLocator::SceneTarget { file_id } => {
            build_scene_target_from_session(session, file_id, &contexts, &mut env)
        }
        MergeTargetLocator::AssetTarget { match_key } => {
            build_asset_target_from_session(session, &match_key, &contexts, &mut env)
        }
    };

    built.ok_or_else(|| {
        AppError::new(
            "merge.target_rebuild_failed",
            format!("Failed to rebuild merge semantic target '{}'", target_id),
        )
    })
}

pub(crate) fn materialize_merge_target(
    session: &mut MergeSemanticSession,
    target_id: &str,
    cwd: &str,
    ref_graph_state: &AssetDbState,
) -> AppResult<MergeTargetInspector> {
    if let Some(inspector) = session.inspectors.get(target_id) {
        return Ok(inspector.clone());
    }

    let built = rebuild_merge_target_from_locator(cwd, session, target_id, ref_graph_state)?;
    let inspector = built.inspector;
    session
        .inspectors
        .insert(target_id.to_string(), inspector.clone());
    Ok(inspector)
}

pub(crate) fn materialize_all_merge_targets(
    session: &mut MergeSemanticSession,
    cwd: &str,
    ref_graph_state: &AssetDbState,
) -> AppResult<()> {
    let target_ids = session
        .targets
        .iter()
        .map(|target| target.id.clone())
        .collect::<Vec<_>>();
    for target_id in target_ids {
        materialize_merge_target(session, &target_id, cwd, ref_graph_state)?;
    }
    Ok(())
}

use crate::diff::semantic::parse::parse_doc_field_map;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::context::{GuidResolver, SideFileSource, SourceMode};
    use crate::diff::semantic::scene::{build_prefab_instance_local_infos, SourcePrefabBacking};
    use crate::diff::semantic::script::ScriptInfoCache;
    use crate::unity_yaml::extract_prefab_instance_irs;

    fn snapshot_side() -> SideContext<'static> {
        SideContext {
            guid_resolver: GuidResolver::None,
            script_guid_resolver: GuidResolver::None,
            source_mode: SourceMode::Snapshot,
            file_source: SideFileSource::Workspace,
        }
    }

    fn test_env<'a>(profiler: &'a mut DiffProfiler) -> SemanticBuildEnv<'a> {
        SemanticBuildEnv {
            app_handle: None,
            cwd: ".",
            profiler,
            batch_reader: None,
            script_cache: ScriptInfoCache::default(),
        }
    }

    #[test]
    fn prefab_instance_merge_panels_use_semantic_targets_instead_of_raw_modification_rows() {
        let base_content = r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!4 &400 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 8679921383154817045, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
  m_GameObject: {fileID: 0}
--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 8679921383154817045, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_LocalPosition.x
      value: 0
      objectReference: {fileID: 0}
    - target: {fileID: 8679921383154817045, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_LocalRotation.w
      value: 1
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
"#;
        let ours_content = base_content.replace("value: 0", "value: 1");
        let theirs_content = base_content.replace("value: 0", "value: 2");

        let base_docs = crate::unity_yaml::parse_yaml_docs(base_content.as_bytes());
        let ours_docs = crate::unity_yaml::parse_yaml_docs(ours_content.as_bytes());
        let theirs_docs = crate::unity_yaml::parse_yaml_docs(theirs_content.as_bytes());
        let base_lines = base_content.lines().map(str::to_string).collect::<Vec<_>>();
        let ours_lines = ours_content.lines().map(str::to_string).collect::<Vec<_>>();
        let theirs_lines = theirs_content
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let base_lines_ref = base_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let ours_lines_ref = ours_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let theirs_lines_ref = theirs_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();

        let base_ir = extract_prefab_instance_irs(&base_docs, &base_lines_ref)
            .into_iter()
            .next()
            .expect("base prefab ir");
        let ours_ir = extract_prefab_instance_irs(&ours_docs, &ours_lines_ref)
            .into_iter()
            .next()
            .expect("ours prefab ir");
        let theirs_ir = extract_prefab_instance_irs(&theirs_docs, &theirs_lines_ref)
            .into_iter()
            .next()
            .expect("theirs prefab ir");

        let base_ctx = snapshot_side();
        let ours_ctx = snapshot_side();
        let theirs_ctx = snapshot_side();
        let contexts = MergeSideContexts {
            base: base_ctx,
            ours: ours_ctx,
            theirs: theirs_ctx,
        };

        let base_local_infos = build_prefab_instance_local_infos(
            &base_docs,
            &base_lines,
            &base_lines_ref,
            &contexts.base,
        );
        let ours_local_infos = build_prefab_instance_local_infos(
            &ours_docs,
            &ours_lines,
            &ours_lines_ref,
            &contexts.ours,
        );
        let theirs_local_infos = build_prefab_instance_local_infos(
            &theirs_docs,
            &theirs_lines,
            &theirs_lines_ref,
            &contexts.theirs,
        );

        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(&mut profiler);
        let (panels, conflict_count, _, _) = build_prefab_instance_merge_panels(
            base_ir.local_file_id,
            Some(&base_ir),
            Some(&ours_ir),
            Some(&theirs_ir),
            None,
            base_local_infos.get(&base_ir.local_file_id),
            ours_local_infos.get(&ours_ir.local_file_id),
            theirs_local_infos.get(&theirs_ir.local_file_id),
            &base_docs,
            &ours_docs,
            &theirs_docs,
            &base_lines,
            &ours_lines,
            &theirs_lines,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &contexts,
            &mut env,
        );

        assert_eq!(conflict_count, 1);
        let panel = panels
            .iter()
            .find(|panel| matches!(panel.panel_kind, InspectorPanelKind::Component))
            .expect("semantic transform panel");
        assert_eq!(panel.title, "Transform");
        assert_eq!(panel.merge_status, DocMergeStatus::HasConflicts);
        assert!(
            panel
                .fields
                .iter()
                .all(|field| !field.property_path.starts_with("m_Modifications")),
            "prefab merge should expose semantic override paths, not raw m_Modifications rows"
        );

        let position_field = panel
            .fields
            .iter()
            .find(|field| field.property_path == "m_LocalPosition")
            .expect("local position group");
        assert_eq!(position_field.label, "Local Position");
        assert_eq!(
            position_field.children[0].property_path,
            "m_LocalPosition.x"
        );
        assert_eq!(position_field.children[0].label, "X");
    }

    #[test]
    fn prefab_instance_merge_panels_infer_renderer_from_override_paths() {
        let renderer_fid: i64 = -7511558181221131132;
        let base_content = format!(
            r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1001 &9000
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {{fileID: 0}}
    m_Modifications:
    - target: {{fileID: {fid}, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}}
      propertyPath: m_Materials.Array.data[0]
      value: 0
      objectReference: {{fileID: 0}}
  m_SourcePrefab: {{fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}}
"#,
            fid = renderer_fid
        );
        let ours_content = base_content.replace("value: 0", "value: 1");
        let theirs_content = base_content.replace("value: 0", "value: 1");

        let base_docs = crate::unity_yaml::parse_yaml_docs(base_content.as_bytes());
        let ours_docs = crate::unity_yaml::parse_yaml_docs(ours_content.as_bytes());
        let theirs_docs = crate::unity_yaml::parse_yaml_docs(theirs_content.as_bytes());
        let base_lines = base_content.lines().map(str::to_string).collect::<Vec<_>>();
        let ours_lines = ours_content.lines().map(str::to_string).collect::<Vec<_>>();
        let theirs_lines = theirs_content
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let base_lines_ref = base_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let ours_lines_ref = ours_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let theirs_lines_ref = theirs_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();

        let base_ir = extract_prefab_instance_irs(&base_docs, &base_lines_ref)
            .into_iter()
            .next()
            .expect("base prefab ir");
        let ours_ir = extract_prefab_instance_irs(&ours_docs, &ours_lines_ref)
            .into_iter()
            .next()
            .expect("ours prefab ir");
        let theirs_ir = extract_prefab_instance_irs(&theirs_docs, &theirs_lines_ref)
            .into_iter()
            .next()
            .expect("theirs prefab ir");

        let contexts = MergeSideContexts {
            base: snapshot_side(),
            ours: snapshot_side(),
            theirs: snapshot_side(),
        };

        let mut order = HashMap::new();
        order.insert(renderer_fid, 0_usize);
        let source_info = SourcePrefabInfo {
            docs: Vec::new(),
            lines: Vec::new(),
            doc_by_file_id: HashMap::new(),
            hierarchy_path_by_file_id: HashMap::new(),
            owner_go_by_component: HashMap::new(),
            component_label_by_file_id: HashMap::new(),
            class_id_by_file_id: HashMap::new(),
            order_index_by_file_id: order,
            loaded_from_new_side: true,
            backing_kind: SourcePrefabBacking::ModelImporterMeta,
        };

        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(&mut profiler);
        let (panels, _, _, _) = build_prefab_instance_merge_panels(
            base_ir.local_file_id,
            Some(&base_ir),
            Some(&ours_ir),
            Some(&theirs_ir),
            Some(&source_info),
            None,
            None,
            None,
            &base_docs,
            &ours_docs,
            &theirs_docs,
            &base_lines,
            &ours_lines,
            &theirs_lines,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &contexts,
            &mut env,
        );

        let panel = panels
            .iter()
            .find(|panel| matches!(panel.panel_kind, InspectorPanelKind::Component))
            .expect("inferred renderer panel");
        assert_eq!(panel.component_type.as_deref(), Some("Renderer"));
        assert_eq!(panel.component_source.as_deref(), Some("inferred"));
        assert_eq!(
            panel
                .component_inference
                .as_ref()
                .map(|meta| meta.reason_code.as_str()),
            Some("propertyPathBuiltinFamily")
        );
    }
}
