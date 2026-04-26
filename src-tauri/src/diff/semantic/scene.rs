use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;

use crate::asset_db::types::{guid_to_hex, Guid, PrefabInstanceIR, PropertyOverride};
use crate::unity_yaml::{
    build_go_tree, extract_prefab_instance_irs, extract_stripped_mappings, get_components_for_go,
    parse_yaml_docs, UnityYamlFile, YamlDoc,
};

use super::component_inference::{
    infer_component_from_override_groups, inference_for_known_class_id,
};
use super::inspector::{
    apply_field_label_enhancements, build_doc_panel_pair, build_doc_panel_pair_readonly,
    build_gameobject_header_panel, build_inspector_fields, build_inspector_fields_pair,
    collect_hierarchy_entries, component_sort_key, count_changed_leaf_fields,
};
use super::parse::{reference_display, split_property_path};
use super::script::{
    doc_script_guid, load_script_semantic_info, load_side_text_file_strict,
    lookup_script_semantic_info, ScriptInfoCache,
};
use super::{
    build_doc_label_map, build_doc_label_map_readonly, doc_type_label, doc_type_label_no_io,
    doc_type_label_readonly, emit_diff_progress, resolve_script_class_name,
    resolve_script_class_name_readonly, unity_class_name, HierarchyEntry, ParsedFieldLine,
    SemanticBuildEnv,
};
use crate::diff::content::{unity_asset_kind, BatchBlobReader};
use crate::diff::context::{DiffBuildContext, SideContext, SideFileSource};
use crate::diff::profiler::{DiffPhase, DiffProfiler};
use crate::diff::types::*;
use crate::error::AppError;

// ── Scene/prefab types ──

#[derive(Debug, Clone)]
pub(crate) struct SceneTargetBuild {
    pub(crate) id: String,
    pub(crate) file_id: i64,
    pub(crate) change_kind: String,
    pub(crate) component_changes: usize,
    pub(crate) field_changes: usize,
    pub(crate) changed_inspector: SemanticTargetInspector,
    pub(crate) full_inspector: SemanticTargetInspector,
}

// ── Source prefab cache for prefab instance semantic diff ──

/// How a [`SourcePrefabInfo`] was constructed. The semantic-diff layer uses
/// this to choose between strict YAML-doc lookups and the more permissive
/// model-importer fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourcePrefabBacking {
    /// Built from a parsed `.prefab`/`.unity` YAML document. `docs` and
    /// `doc_by_file_id` are populated and may be indexed directly.
    YamlPrefab,
    /// Built from a Unity `ModelImporter` `.meta` file (e.g. for `.fbx` source
    /// assets). `docs` is empty by design — only the `*_by_file_id` lookup
    /// tables are meaningful, plus [`SourcePrefabInfo::order_index_by_file_id`]
    /// for stable panel ordering.
    ModelImporterMeta,
}

/// Parsed source prefab data cached for reuse across multiple prefab instance targets.
#[allow(dead_code)]
pub(crate) struct SourcePrefabInfo {
    pub(crate) docs: Vec<YamlDoc>,
    pub(crate) lines: Vec<String>,
    /// fileID → index into `docs`. Only populated when
    /// `backing_kind == SourcePrefabBacking::YamlPrefab`; for model-meta
    /// backed entries this map is intentionally empty (the docs vec is empty
    /// too, so any stale index would be unsafe).
    pub(crate) doc_by_file_id: HashMap<i64, usize>,
    /// fileID → hierarchy path string (e.g. "Root/Child/Grandchild")
    pub(crate) hierarchy_path_by_file_id: HashMap<i64, String>,
    /// component fileID → owning GameObject fileID
    pub(crate) owner_go_by_component: HashMap<i64, i64>,
    /// fileID → component display label (e.g. "Transform", "MyScript")
    pub(crate) component_label_by_file_id: HashMap<i64, String>,
    /// fileID → class_id
    pub(crate) class_id_by_file_id: HashMap<i64, i32>,
    /// fileID → stable panel sort key. For YAML-backed prefabs this mirrors
    /// the doc order; for model-meta backed prefabs it reflects the order
    /// entries appear in the `.meta` file. Kept separate from
    /// `doc_by_file_id` so the latter never carries fake indices.
    pub(crate) order_index_by_file_id: HashMap<i64, usize>,
    /// Which side this prefab was loaded from (true = new, false = old).
    pub(crate) loaded_from_new_side: bool,
    /// How this `SourcePrefabInfo` was constructed.
    pub(crate) backing_kind: SourcePrefabBacking,
}

/// Specific reason a [`SourcePrefabInfo`] could not be produced for a given
/// GUID. This used to be collapsed into `Option::None`, which left UI panels
/// reporting the misleading blanket message "source prefab was not loaded"
/// regardless of whether the GUID failed to resolve, the blob was unreadable,
/// the YAML failed to parse, or the parser simply found no usable entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourcePrefabLoadError {
    /// Neither `resolve_script_guid_path` nor `resolve_guid_path` returned a
    /// path for this GUID on either side. The asset database does not know
    /// about it (typical when the `.meta` file was added or removed in the
    /// commit being inspected).
    GuidUnresolved,
    /// The GUID resolved to a path, but neither the snapshot blob nor the
    /// workspace fallback could read the file content.
    BlobMissing { tried_path: String },
    /// File content was loaded but YAML parsing failed (or the document had
    /// no usable root key, e.g. neither `ModelImporter` nor a YAML prefab
    /// header).
    ParseFailed { tried_path: String },
    /// The file parsed but `parse_model_importer_meta` returned no entries.
    /// `detail` carries the underlying sub-cause (YAML invalid, no
    /// `ModelImporter` root, no recycle tables, or tables present but empty)
    /// so the panel layer can give an actionable hint instead of a blanket
    /// "meta has no entries".
    EmptyMeta { tried_path: String, detail: String },
}

impl SourcePrefabLoadError {
    pub(crate) fn describe(&self) -> String {
        match self {
            SourcePrefabLoadError::GuidUnresolved => {
                "source prefab GUID could not be resolved to an asset path on either side"
                    .to_string()
            }
            SourcePrefabLoadError::BlobMissing { tried_path } => format!(
                "source prefab content was unreadable (snapshot blob missing and workspace fallback failed) at {}",
                tried_path
            ),
            SourcePrefabLoadError::ParseFailed { tried_path } => format!(
                "source prefab content failed to parse at {}",
                tried_path
            ),
            SourcePrefabLoadError::EmptyMeta { tried_path, detail } => format!(
                "model importer meta at {} contained no recycleID entries: {}",
                tried_path, detail
            ),
        }
    }
}

/// Cache keyed by GUID hex string → parsed source prefab info (None if load failed).
///
/// `errors` runs in parallel with `entries`: when `entries[guid] == None`, the
/// matching entry in `errors` (if present) carries the specific failure reason
/// so the panel layer can produce an actionable diagnostic instead of the
/// blanket "source prefab was not loaded".
#[derive(Default)]
pub(crate) struct SourcePrefabCache {
    pub(crate) entries: HashMap<String, Option<SourcePrefabInfo>>,
    pub(crate) errors: HashMap<String, SourcePrefabLoadError>,
}

#[derive(Debug, Clone)]
pub(crate) struct PrefabLocalTargetInfo {
    pub(crate) class_id: i32,
    pub(crate) panel_title: String,
    pub(crate) component_label: String,
    pub(crate) local_doc_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PrefabInstanceLocalInfo {
    pub(crate) targets: HashMap<i64, PrefabLocalTargetInfo>,
}

/// Shared parsed scene/prefab metadata reused by asset preview, diff, and merge.
///
/// This is the common "semantic parsing" layer for one side of a Unity
/// scene/prefab file: hierarchy entries, cross-doc labels, prefab-instance IR,
/// and local stripped-target metadata. Higher-level features can build their
/// own inspectors on top of this without re-running the same parsing passes.
#[derive(Debug, Clone, Default)]
pub(crate) struct SceneSemanticSideData {
    pub(crate) entries: HashMap<i64, HierarchyEntry>,
    pub(crate) labels: HashMap<i64, String>,
    pub(crate) prefab_irs: HashMap<i64, PrefabInstanceIR>,
    pub(crate) prefab_local_infos: HashMap<i64, PrefabInstanceLocalInfo>,
}

pub(crate) fn scene_root_docs<'a>(docs: &'a [YamlDoc]) -> HashMap<i64, &'a YamlDoc> {
    docs.iter()
        .filter(|doc| (doc.class_id == 1 && !doc.is_stripped) || doc.class_id == 1001)
        .map(|doc| (doc.file_id, doc))
        .collect()
}

fn object_path_map(entries: &HashMap<i64, HierarchyEntry>) -> HashMap<i64, String> {
    entries
        .iter()
        .map(|(file_id, entry)| (*file_id, entry.path.clone()))
        .collect()
}

pub(crate) fn build_scene_side_data(
    docs: &[YamlDoc],
    lines: &[String],
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> SceneSemanticSideData {
    let root_docs = scene_root_docs(docs);
    let entries = collect_hierarchy_entries(&build_go_tree(docs), &root_docs);
    build_scene_side_data_from_entries(docs, lines, entries, side_ctx, env)
}

pub(crate) fn build_scene_side_data_readonly(
    docs: &[YamlDoc],
    lines: &[String],
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> SceneSemanticSideData {
    let root_docs = scene_root_docs(docs);
    let entries = collect_hierarchy_entries(&build_go_tree(docs), &root_docs);
    build_scene_side_data_from_entries_readonly(docs, lines, entries, side_ctx, script_cache)
}

pub(crate) fn build_scene_side_data_from_entries(
    docs: &[YamlDoc],
    lines: &[String],
    entries: HashMap<i64, HierarchyEntry>,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> SceneSemanticSideData {
    let labels = build_doc_label_map(docs, &object_path_map(&entries), lines, side_ctx, env);
    let lines_ref = lines.iter().map(|line| line.as_str()).collect::<Vec<_>>();
    let prefab_irs = extract_prefab_instance_irs(docs, &lines_ref)
        .into_iter()
        .map(|ir| (ir.local_file_id, ir))
        .collect::<HashMap<_, _>>();
    let prefab_local_infos = build_prefab_instance_local_infos(docs, lines, &lines_ref, side_ctx);

    SceneSemanticSideData {
        entries,
        labels,
        prefab_irs,
        prefab_local_infos,
    }
}

pub(crate) fn build_scene_side_data_from_entries_readonly(
    docs: &[YamlDoc],
    lines: &[String],
    entries: HashMap<i64, HierarchyEntry>,
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> SceneSemanticSideData {
    let labels = build_doc_label_map_readonly(
        docs,
        &object_path_map(&entries),
        lines,
        side_ctx,
        script_cache,
    );
    let lines_ref = lines.iter().map(|line| line.as_str()).collect::<Vec<_>>();
    let prefab_irs = extract_prefab_instance_irs(docs, &lines_ref)
        .into_iter()
        .map(|ir| (ir.local_file_id, ir))
        .collect::<HashMap<_, _>>();
    let prefab_local_infos = build_prefab_instance_local_infos(docs, lines, &lines_ref, side_ctx);

    SceneSemanticSideData {
        entries,
        labels,
        prefab_irs,
        prefab_local_infos,
    }
}

fn prefab_instance_subtitle(
    ir: Option<&PrefabInstanceIR>,
    primary_ctx: &SideContext,
    fallback_ctx: Option<&SideContext>,
) -> Option<String> {
    let source_path_hint = ir.and_then(|ir| {
        let guid = &ir.source_prefab_guid;
        primary_ctx
            .resolve_script_guid_path(guid)
            .or_else(|| fallback_ctx.and_then(|ctx| ctx.resolve_script_guid_path(guid)))
            .or_else(|| primary_ctx.resolve_guid_path(guid))
            .or_else(|| fallback_ctx.and_then(|ctx| ctx.resolve_guid_path(guid)))
    });

    match source_path_hint {
        Some(path) => Some(format!("Prefab Instance \u{00b7} {}", path)),
        None => {
            let guid_hex = ir.map(|entry| guid_to_hex(&entry.source_prefab_guid));
            match guid_hex {
                Some(hex) => Some(format!("Prefab Instance \u{00b7} {}", hex)),
                None => Some("Prefab Instance".into()),
            }
        }
    }
}

// ── Shared read-only context for parallel scene target building ──

struct SceneBuildShared<'a> {
    old_object_docs: &'a HashMap<i64, &'a YamlDoc>,
    new_object_docs: &'a HashMap<i64, &'a YamlDoc>,
    old_all_docs: &'a [YamlDoc],
    new_all_docs: &'a [YamlDoc],
    old_entries: &'a HashMap<i64, HierarchyEntry>,
    new_entries: &'a HashMap<i64, HierarchyEntry>,
    old_lines: &'a [String],
    new_lines: &'a [String],
    old_labels: &'a HashMap<i64, String>,
    new_labels: &'a HashMap<i64, String>,
    ctx: &'a DiffBuildContext<'a>,
    old_prefab_irs: &'a HashMap<i64, PrefabInstanceIR>,
    new_prefab_irs: &'a HashMap<i64, PrefabInstanceIR>,
    old_prefab_local_infos: &'a HashMap<i64, PrefabInstanceLocalInfo>,
    new_prefab_local_infos: &'a HashMap<i64, PrefabInstanceLocalInfo>,
    source_prefab_cache: &'a SourcePrefabCache,
    old_component_index: &'a HashMap<i64, Vec<usize>>,
    new_component_index: &'a HashMap<i64, Vec<usize>>,
}

/// Worker-local mutable state for parallel scene target building.
/// Fields are currently unused because warmup covers all I/O, but retained
/// for future fallback support.
#[allow(dead_code)]
struct SceneWorkerEnv {
    cwd: String,
    batch_reader: Option<BatchBlobReader>,
    miss_count: u64,
}

// ── Scene component collection ──

/// Pre-built index: go_file_id → Vec<doc index> for O(1) component lookup.
pub(crate) fn build_component_index(docs: &[YamlDoc]) -> HashMap<i64, Vec<usize>> {
    let mut idx: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, doc) in docs.iter().enumerate() {
        if let Some(go_id) = doc.m_game_object_id {
            idx.entry(go_id).or_default().push(i);
        }
        if doc.class_id == 1 || doc.class_id == 1001 {
            idx.entry(doc.file_id).or_default().push(i);
        }
    }
    for v in idx.values_mut() {
        v.sort();
        v.dedup();
    }
    idx
}

pub(crate) fn collect_scene_components<'a>(
    docs: &'a [YamlDoc],
    lines: &[String],
    go_file_id: i64,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
    component_index: Option<&HashMap<i64, Vec<usize>>>,
) -> Vec<(String, String, Option<String>, &'a YamlDoc)> {
    let mut counters: HashMap<String, usize> = HashMap::new();
    let mut entries = Vec::new();

    let fallback;
    let index_slice = if let Some(idx) = component_index {
        idx.get(&go_file_id).map(|v| v.as_slice()).unwrap_or(&[])
    } else {
        fallback = get_components_for_go(docs, go_file_id);
        fallback.as_slice()
    };
    for &index in index_slice {
        let Some(doc) = docs.get(index) else {
            continue;
        };
        if doc.class_id == 1 || doc.class_id == 1001 || doc.m_game_object_id != Some(go_file_id) {
            continue;
        }
        let script_class = if doc.class_id == 114 {
            resolve_script_class_name(doc, lines, side_ctx, env)
        } else {
            None
        };
        let base_title = script_class
            .clone()
            .unwrap_or_else(|| doc_type_label(doc, lines, side_ctx, env));
        let base_key = if doc.class_id == 114 {
            let guid_key = doc_script_guid(doc, lines)
                .map(|guid| guid_to_hex(&guid))
                .or_else(|| script_class.clone())
                .unwrap_or_else(|| "mono".into());
            format!("{}:{}", doc.class_id, guid_key)
        } else {
            format!("{}", doc.class_id)
        };
        let count = counters.entry(base_key.clone()).or_insert(0usize);
        let key = format!("{}:{}", base_key, *count);
        let title = if *count == 0 {
            base_title
        } else {
            format!("{} ({})", base_title, *count + 1)
        };
        *count += 1;
        entries.push((key, title, script_class, doc));
    }

    entries.sort_by_key(|(_, _, _, doc)| (component_sort_key(doc.class_id), doc.line_start));
    entries
}

/// Like `collect_scene_components` but uses readonly script cache (no &mut SemanticBuildEnv).
pub(crate) fn collect_scene_components_readonly<'a>(
    docs: &'a [YamlDoc],
    lines: &[String],
    go_file_id: i64,
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
    component_index: &HashMap<i64, Vec<usize>>,
) -> Vec<(String, String, Option<String>, &'a YamlDoc)> {
    let mut counters: HashMap<String, usize> = HashMap::new();
    let mut entries = Vec::new();

    let index_slice = component_index
        .get(&go_file_id)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    for &index in index_slice {
        let Some(doc) = docs.get(index) else {
            continue;
        };
        if doc.class_id == 1 || doc.class_id == 1001 || doc.m_game_object_id != Some(go_file_id) {
            continue;
        }
        let script_class = if doc.class_id == 114 {
            resolve_script_class_name_readonly(doc, lines, side_ctx, script_cache)
        } else {
            None
        };
        let base_title = script_class
            .clone()
            .unwrap_or_else(|| doc_type_label_readonly(doc, lines, side_ctx, script_cache));
        let base_key = if doc.class_id == 114 {
            let guid_key = doc_script_guid(doc, lines)
                .map(|guid| guid_to_hex(&guid))
                .or_else(|| script_class.clone())
                .unwrap_or_else(|| "mono".into());
            format!("{}:{}", doc.class_id, guid_key)
        } else {
            format!("{}", doc.class_id)
        };
        let count = counters.entry(base_key.clone()).or_insert(0usize);
        let key = format!("{}:{}", base_key, *count);
        let title = if *count == 0 {
            base_title
        } else {
            format!("{} ({})", base_title, *count + 1)
        };
        *count += 1;
        entries.push((key, title, script_class, doc));
    }

    entries.sort_by_key(|(_, _, _, doc)| (component_sort_key(doc.class_id), doc.line_start));
    entries
}

fn scene_target_change_kind(old_exists: bool, new_exists: bool, has_changes: bool) -> String {
    match (old_exists, new_exists, has_changes) {
        (false, true, _) => "added".into(),
        (true, false, _) => "removed".into(),
        (_, _, true) => "modified".into(),
        _ => "unchanged".into(),
    }
}

/// Fast check: are two prefab instance IRs semantically identical?
/// O(overrides) comparison, much cheaper than building full inspector panels.
pub(crate) fn prefab_instance_is_unchanged(
    old_ir: Option<&PrefabInstanceIR>,
    new_ir: Option<&PrefabInstanceIR>,
) -> bool {
    match (old_ir, new_ir) {
        (Some(old), Some(new)) => {
            old.property_overrides.len() == new.property_overrides.len()
                && old.removed_components.len() == new.removed_components.len()
                && old
                    .property_overrides
                    .iter()
                    .zip(&new.property_overrides)
                    .all(|(a, b)| {
                        a.target.source_file_id == b.target.source_file_id
                            && a.property_path == b.property_path
                            && a.value == b.value
                            && a.object_ref.as_ref().map(|r| (r.source_file_id, r.guid))
                                == b.object_ref.as_ref().map(|r| (r.source_file_id, r.guid))
                    })
                && old
                    .removed_components
                    .iter()
                    .zip(&new.removed_components)
                    .all(|(a, b)| a.target.source_file_id == b.target.source_file_id)
        }
        _ => false, // added/removed — not unchanged
    }
}

pub(crate) fn build_prefab_instance_local_infos(
    docs: &[YamlDoc],
    _lines: &[String],
    lines_ref: &[&str],
    side_ctx: &SideContext,
) -> HashMap<i64, PrefabInstanceLocalInfo> {
    let doc_meta_by_file_id: HashMap<i64, (usize, &YamlDoc)> = docs
        .iter()
        .enumerate()
        .map(|(idx, doc)| (doc.file_id, (idx, doc)))
        .collect();
    let mut infos: HashMap<i64, PrefabInstanceLocalInfo> = HashMap::new();

    for mapping in extract_stripped_mappings(docs, lines_ref) {
        let Some((doc_idx, doc)) = doc_meta_by_file_id.get(&mapping.local_file_id).copied() else {
            continue;
        };
        let component_label = doc_type_label_no_io(doc, side_ctx);
        let panel_title = if mapping.class_id == 1 {
            doc.m_name
                .clone()
                .unwrap_or_else(|| component_label.clone())
        } else {
            component_label.clone()
        };

        infos
            .entry(mapping.prefab_instance_id)
            .or_default()
            .targets
            .insert(
                mapping.source.source_file_id,
                PrefabLocalTargetInfo {
                    class_id: mapping.class_id,
                    panel_title,
                    component_label,
                    local_doc_index: doc_idx,
                },
            );
    }

    infos
}

// ── Prefab instance semantic diff ──

/// Normalize Unity's serialized array property path notation.
/// `m_Materials.Array.data[0]` → `m_Materials[0]`
fn normalize_override_path(path: &str) -> String {
    path.replace(".Array.data[", "[")
        .replace(".Array.size", ".size")
}

/// Load source prefab content for warmup. Avoids expensive LFS smudge by falling
/// back to workspace file when git returns an LFS pointer. Source prefabs are text
/// YAML files that typically exist in the workspace even for snapshot diffs.
fn try_load_prefab_content(
    guid: &Guid,
    side_ctx: &SideContext,
    cwd: &str,
    batch_reader: Option<&mut BatchBlobReader>,
) -> Option<String> {
    use crate::diff::content::{git_show_file_sync, parse_lfs_pointer};

    let asset_path = side_ctx
        .resolve_script_guid_path(guid)
        .or_else(|| side_ctx.resolve_guid_path(guid))?;

    match &side_ctx.file_source {
        SideFileSource::Workspace => {
            let full_path = Path::new(cwd).join(&asset_path);
            std::fs::read_to_string(full_path).ok()
        }
        SideFileSource::GitRef(reference) => {
            let ref_spec = format!("{}:{}", reference, asset_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| git_show_file_sync(cwd, &ref_spec));
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    // LFS pointer — fall back to workspace file (fast, no smudge)
                    let full_path = Path::new(cwd).join(&asset_path);
                    std::fs::read_to_string(full_path).ok()
                }
                other => other,
            }
        }
        SideFileSource::GitIndex => {
            let ref_spec = format!(":{}", asset_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| git_show_file_sync(cwd, &ref_spec));
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    let full_path = Path::new(cwd).join(&asset_path);
                    std::fs::read_to_string(full_path).ok()
                }
                other => other,
            }
        }
        SideFileSource::GitStage(n) => {
            let ref_spec = format!(":{}:{}", n, asset_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| git_show_file_sync(cwd, &ref_spec));
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    let full_path = Path::new(cwd).join(&asset_path);
                    std::fs::read_to_string(full_path).ok()
                }
                other => other,
            }
        }
    }
}

/// Build SourcePrefabInfo from already-loaded content. Pure computation, no I/O.
fn load_source_prefab_info_from_content(
    content: String,
    side_ctx: &SideContext,
    loaded_from_new_side: bool,
) -> Option<SourcePrefabInfo> {
    let docs = parse_yaml_docs(content.as_bytes());
    if docs.is_empty() {
        return None;
    }
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    let mut doc_by_file_id = HashMap::new();
    let mut order_index_by_file_id = HashMap::new();
    let mut class_id_by_file_id = HashMap::new();
    let mut owner_go_by_component = HashMap::new();
    let mut component_label_by_file_id = HashMap::new();

    for (idx, doc) in docs.iter().enumerate() {
        doc_by_file_id.insert(doc.file_id, idx);
        order_index_by_file_id.insert(doc.file_id, idx);
        class_id_by_file_id.insert(doc.file_id, doc.class_id);
        if let Some(go_id) = doc.m_game_object_id {
            if doc.class_id != 1 && doc.class_id != 1001 {
                owner_go_by_component.insert(doc.file_id, go_id);
            }
        }
        let label = if doc.class_id == 114 {
            doc.m_script_guid
                .and_then(|sg| side_ctx.resolve_script_guid_path(&sg))
                .and_then(|p| {
                    Path::new(&p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "MonoBehaviour".into())
        } else {
            unity_class_name(doc.class_id).to_string()
        };
        component_label_by_file_id.insert(doc.file_id, label);
    }

    let go_docs: HashMap<i64, &YamlDoc> = docs
        .iter()
        .filter(|d| d.class_id == 1 && !d.is_stripped)
        .map(|d| (d.file_id, d))
        .collect();
    let tree = build_go_tree(&docs);
    let entries = collect_hierarchy_entries(&tree, &go_docs);
    let mut hierarchy_path_by_file_id: HashMap<i64, String> =
        entries.into_iter().map(|(fid, e)| (fid, e.path)).collect();

    for (comp_fid, go_fid) in &owner_go_by_component {
        if let Some(go_path) = hierarchy_path_by_file_id.get(go_fid) {
            let comp_label = component_label_by_file_id
                .get(comp_fid)
                .cloned()
                .unwrap_or_else(|| "Component".into());
            let full_path = format!("{}/{}", go_path, comp_label);
            hierarchy_path_by_file_id.insert(*comp_fid, full_path);
        }
    }

    Some(SourcePrefabInfo {
        docs,
        lines,
        doc_by_file_id,
        hierarchy_path_by_file_id,
        owner_go_by_component,
        component_label_by_file_id,
        class_id_by_file_id,
        order_index_by_file_id,
        loaded_from_new_side,
        backing_kind: SourcePrefabBacking::YamlPrefab,
    })
}

/// Build a [`SourcePrefabInfo`] from a Unity ModelImporter `.meta` file.
///
/// FBX/OBJ/etc. source assets are opaque binary that we cannot parse, but
/// Unity's importer persists a `fileID → (classID, name)` table in the sidecar
/// `.meta`. This synthesizes a partial `SourcePrefabInfo` from that table so
/// that prefab-instance panels referencing model-derived components still get
/// real titles like `Transform`/`MeshRenderer` instead of the generic
/// `Component (fileID:...)` fallback.
///
/// `docs`/`lines`/`doc_by_file_id`/`owner_go_by_component` are intentionally
/// empty — the meta has no concept of YAML doc indices and downstream code
/// only reaches them via `Option`/`get()` lookups, which gracefully return
/// `None` for model-meta backed entries.
fn load_source_prefab_info_from_model_meta(
    content: &str,
    loaded_from_new_side: bool,
) -> Option<SourcePrefabInfo> {
    use super::model_meta::parse_model_importer_meta;
    let entries = parse_model_importer_meta(content);
    if entries.is_empty() {
        return None;
    }

    let mut class_id_by_file_id: HashMap<i64, i32> = HashMap::new();
    let mut component_label_by_file_id: HashMap<i64, String> = HashMap::new();
    let mut hierarchy_path_by_file_id: HashMap<i64, String> = HashMap::new();
    let mut order_index_by_file_id: HashMap<i64, usize> = HashMap::new();

    for entry in entries {
        order_index_by_file_id.insert(entry.file_id, entry.order_index);

        let class_label = entry.class_id.map(|cid| unity_class_name(cid).to_string());

        if let Some(cid) = entry.class_id {
            class_id_by_file_id.insert(entry.file_id, cid);
        }
        if let Some(ref label) = class_label {
            component_label_by_file_id.insert(entry.file_id, label.clone());
        }

        // Build a display path: GameObjects use the node name verbatim;
        // components nested under that node show "<node>/<ComponentType>".
        // When either piece is missing we fall back to whatever we have so
        // the panel title at least carries useful information.
        let path = match (
            entry.class_id,
            class_label.as_deref(),
            entry.node_name.as_str(),
        ) {
            (Some(1), _, name) if !name.is_empty() => name.to_string(),
            (Some(_), Some(label), name) if !name.is_empty() => format!("{}/{}", name, label),
            (_, Some(label), _) => label.to_string(),
            (_, None, name) if !name.is_empty() => name.to_string(),
            _ => continue,
        };
        hierarchy_path_by_file_id.insert(entry.file_id, path);
    }

    if class_id_by_file_id.is_empty() && hierarchy_path_by_file_id.is_empty() {
        return None;
    }

    Some(SourcePrefabInfo {
        docs: Vec::new(),
        lines: Vec::new(),
        doc_by_file_id: HashMap::new(),
        hierarchy_path_by_file_id,
        owner_go_by_component: HashMap::new(),
        component_label_by_file_id,
        class_id_by_file_id,
        order_index_by_file_id,
        loaded_from_new_side,
        backing_kind: SourcePrefabBacking::ModelImporterMeta,
    })
}

/// Returns true when `asset_path` points at a Unity ModelImporter asset
/// (FBX/OBJ/DAE/...). Such assets cannot be parsed as YAML — instead the
/// loader should read `{asset_path}.meta` and route the content through
/// [`load_source_prefab_info_from_model_meta`].
pub(crate) fn is_model_importer_asset(asset_path: &str) -> bool {
    let ext = Path::new(asset_path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("fbx" | "obj" | "dae" | "blend" | "3ds" | "max")
    )
}

/// Dispatch source-asset content to the right parser based on whether the
/// originating `asset_path` is a YAML prefab or a ModelImporter asset.
fn build_source_info_from_loaded_content(
    asset_path: &str,
    content: String,
    side_ctx: &SideContext,
    loaded_from_new_side: bool,
) -> Option<SourcePrefabInfo> {
    if is_model_importer_asset(asset_path) {
        load_source_prefab_info_from_model_meta(&content, loaded_from_new_side)
    } else {
        load_source_prefab_info_from_content(content, side_ctx, loaded_from_new_side)
    }
}

pub(crate) fn load_source_prefab_info(
    guid: &Guid,
    guid_hex: &str,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
    is_new_side: bool,
) -> Result<SourcePrefabInfo, SourcePrefabLoadError> {
    let asset_path = side_ctx
        .resolve_script_guid_path(guid)
        .or_else(|| side_ctx.resolve_guid_path(guid));
    let Some(asset_path) = asset_path else {
        eprintln!(
            "[semantic/prefab] failed to resolve source prefab path: guid={} file_source={:?}",
            guid_hex, side_ctx.file_source
        );
        return Err(SourcePrefabLoadError::GuidUnresolved);
    };

    // For ModelImporter assets (FBX/OBJ/...) we read the sidecar `.meta`
    // instead of the binary asset itself.
    let load_target = if is_model_importer_asset(&asset_path) {
        format!("{}.meta", asset_path)
    } else {
        asset_path.clone()
    };

    let strict_content = load_side_text_file_strict(
        env.cwd,
        &load_target,
        side_ctx,
        env.batch_reader.as_mut(),
        env.profiler,
    );

    let content = if let Some(content) = strict_content {
        content
    } else {
        let workspace_path = Path::new(env.cwd).join(&load_target);
        match std::fs::read_to_string(&workspace_path) {
            Ok(content) => {
                eprintln!(
                    "[semantic/prefab] snapshot blob missing for guid={} path={} file_source={:?}; using workspace fallback {}",
                    guid_hex,
                    load_target,
                    side_ctx.file_source,
                    workspace_path.display()
                );
                content
            }
            Err(err) => {
                eprintln!(
                    "[semantic/prefab] failed to load source prefab guid={} path={} file_source={:?}: snapshot blob missing and workspace fallback failed: {}",
                    guid_hex,
                    load_target,
                    side_ctx.file_source,
                    err
                );
                return Err(SourcePrefabLoadError::BlobMissing {
                    tried_path: load_target,
                });
            }
        }
    };

    classify_loaded_content(&asset_path, &load_target, content, side_ctx, is_new_side)
        .map_err(|err| {
            eprintln!(
                "[semantic/prefab] failed to parse source prefab content: guid={} asset_path={} loaded_from={} file_source={:?} err={:?}",
                guid_hex, asset_path, load_target, side_ctx.file_source, err
            );
            err
        })
}

/// Pick the more diagnostically useful of two load failures. Ranking, lowest
/// first: `GuidUnresolved` < `BlobMissing` < `ParseFailed` < `EmptyMeta`.
/// The intuition is that the later stages indicate the file *was* found and
/// partially processed, so the failure tells the user something concrete about
/// the file rather than about the lookup table.
fn pick_more_specific_error(
    a: SourcePrefabLoadError,
    b: SourcePrefabLoadError,
) -> SourcePrefabLoadError {
    fn rank(e: &SourcePrefabLoadError) -> u8 {
        match e {
            SourcePrefabLoadError::GuidUnresolved => 0,
            SourcePrefabLoadError::BlobMissing { .. } => 1,
            SourcePrefabLoadError::ParseFailed { .. } => 2,
            SourcePrefabLoadError::EmptyMeta { .. } => 3,
        }
    }
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

/// Wrap [`build_source_info_from_loaded_content`] so that the `None` outcome is
/// split into the two failure modes the cache layer cares about: parse failure
/// vs. successfully-parsed-but-empty meta. Both used to collapse to `None`,
/// which made the panel-layer diagnostic useless.
fn classify_loaded_content(
    asset_path: &str,
    tried_path: &str,
    content: String,
    side_ctx: &SideContext,
    is_new_side: bool,
) -> Result<SourcePrefabInfo, SourcePrefabLoadError> {
    if is_model_importer_asset(asset_path) {
        // Use the detailed parser so we can split "couldn't parse YAML" /
        // "no ModelImporter root" / "no recycle tables" / "tables empty" into
        // distinct sub-causes that the inspector UI can render.
        use super::model_meta::parse_model_importer_meta_detailed;
        let parsed = parse_model_importer_meta_detailed(&content);
        if parsed.entries.is_empty() {
            let detail = parsed
                .failure
                .as_ref()
                .map(|f| f.describe())
                .unwrap_or_else(|| "no entries and no diagnostic from parser".to_string());
            return Err(SourcePrefabLoadError::EmptyMeta {
                tried_path: tried_path.to_string(),
                detail,
            });
        }
        match build_source_info_from_loaded_content(asset_path, content, side_ctx, is_new_side) {
            Some(info) => Ok(info),
            None => Err(SourcePrefabLoadError::EmptyMeta {
                tried_path: tried_path.to_string(),
                detail: "model importer dispatcher returned None despite non-empty parser entries"
                    .to_string(),
            }),
        }
    } else {
        match build_source_info_from_loaded_content(asset_path, content, side_ctx, is_new_side) {
            Some(info) => Ok(info),
            None => Err(SourcePrefabLoadError::ParseFailed {
                tried_path: tried_path.to_string(),
            }),
        }
    }
}

/// Build a ParsedFieldLine map from a list of PropertyOverrides targeting a specific
/// source_file_id. This produces the same format that `parse_doc_field_map` outputs,
/// allowing reuse of `build_inspector_fields`.
pub(crate) fn build_override_field_map(
    overrides: &[&crate::asset_db::types::PropertyOverride],
    side_ctx: &SideContext,
    local_labels: &HashMap<i64, String>,
) -> HashMap<String, ParsedFieldLine> {
    let mut map = HashMap::new();
    for ovr in overrides {
        let norm_path = normalize_override_path(&ovr.property_path);
        let label = {
            let segments = split_property_path(&norm_path);
            segments
                .last()
                .cloned()
                .unwrap_or_else(|| norm_path.clone())
        };

        let (value, reference) = if let Some(ref obj_ref) = ovr.object_ref {
            // This override sets an object reference
            if obj_ref.guid == [0u8; 16] && obj_ref.source_file_id == 0 {
                // Null reference
                (Some("None".into()), None)
            } else {
                let guid_hex = guid_to_hex(&obj_ref.guid);
                let path = if obj_ref.guid != [0u8; 16] {
                    side_ctx.resolve_guid_path(&obj_ref.guid)
                } else {
                    None
                };
                let local_path = if obj_ref.guid == [0u8; 16] {
                    local_labels.get(&obj_ref.source_file_id).cloned()
                } else {
                    None
                };
                let has_guid = obj_ref.guid != [0u8; 16];
                let resolve_hint = if has_guid && path.is_none() {
                    Some(
                        "not_in_asset_db: GUID not found in project asset database (.meta files)"
                            .into(),
                    )
                } else {
                    None
                };
                let stale = path.is_some() && side_ctx.is_snapshot();
                let insp_ref = InspectorReference {
                    guid: if has_guid {
                        Some(guid_hex.clone())
                    } else {
                        None
                    },
                    path: path.clone().or(local_path.clone()),
                    file_id: Some(obj_ref.source_file_id),
                    resolve_hint,
                    stale,
                };
                let display = reference_display(&insp_ref);
                (Some(display), Some(insp_ref))
            }
        } else {
            (ovr.value.clone(), None)
        };

        map.insert(
            norm_path,
            ParsedFieldLine {
                label,
                value,
                reference,
            },
        );
    }
    map
}

fn prefab_local_target<'a>(
    source_file_id: i64,
    primary_local_info: Option<&'a PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&'a PrefabInstanceLocalInfo>,
) -> Option<&'a PrefabLocalTargetInfo> {
    primary_local_info
        .and_then(|info| info.targets.get(&source_file_id))
        .or_else(|| secondary_local_info.and_then(|info| info.targets.get(&source_file_id)))
}

/// Determine panel title for a source target in a prefab instance.
pub(crate) fn prefab_panel_title(
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    primary_local_info: Option<&PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&PrefabInstanceLocalInfo>,
) -> String {
    if let Some(info) = source_info {
        if let Some(path) = info.hierarchy_path_by_file_id.get(&source_file_id) {
            return path.clone();
        }
        if let Some(label) = info.component_label_by_file_id.get(&source_file_id) {
            return label.clone();
        }
        // Use actual class_id from source prefab for fallback label
        if let Some(&class_id) = info.class_id_by_file_id.get(&source_file_id) {
            return unity_class_name(class_id).to_string();
        }
        // Model-importer meta loaded but didn't pin down a class id — try the
        // legacy short-fileID heuristic. This only catches old `.fbx` layouts;
        // 64-bit recycleIDs return None and continue to the generic fallback.
        if info.backing_kind == SourcePrefabBacking::ModelImporterMeta {
            if let Some(label) = heuristic_component_label_from_file_id(source_file_id) {
                return format!("{} (fileID:{})", label, source_file_id);
            }
        }
    }
    if let Some(target) =
        prefab_local_target(source_file_id, primary_local_info, secondary_local_info)
    {
        return target.panel_title.clone();
    }
    // Unresolved: generic label (don't use PrefabSourceRef.type_id, it's the reference type 2/3)
    format!("Component (fileID:{})", source_file_id)
}

/// Heuristic class-name lookup for legacy short Unity fileIDs of the form
/// `classID * 100000 + N`. Used only as a last-resort label for ModelImporter
/// sources where `.meta` parsing succeeded but did not pin down a class id.
/// Returns `None` for modern 64-bit recycleIDs — those have no derivable class
/// without the `internalIDToNameTable` entry.
fn heuristic_component_label_from_file_id(file_id: i64) -> Option<&'static str> {
    let cid = super::model_meta::legacy_class_id_from_short_file_id(file_id)?;
    Some(unity_class_name(cid))
}

fn prefab_panel_order_index(
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    primary_local_info: Option<&PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&PrefabInstanceLocalInfo>,
) -> usize {
    source_info
        .and_then(|info| info.order_index_by_file_id.get(&source_file_id).copied())
        .or_else(|| {
            prefab_local_target(source_file_id, primary_local_info, secondary_local_info)
                .map(|target| target.local_doc_index)
        })
        .unwrap_or(usize::MAX)
}

fn prefab_panel_sort_key(
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    primary_local_info: Option<&PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&PrefabInstanceLocalInfo>,
) -> (i32, i32, usize, i64) {
    let local_target =
        prefab_local_target(source_file_id, primary_local_info, secondary_local_info);
    let class_id = source_info
        .and_then(|info| info.class_id_by_file_id.get(&source_file_id).copied())
        .or_else(|| local_target.map(|target| target.class_id));
    let panel_rank = match class_id {
        Some(1) => 0,
        Some(_) => 1,
        None => 2,
    };
    let component_rank = class_id.map(component_sort_key).unwrap_or(i32::MAX);
    let order_index = prefab_panel_order_index(
        source_file_id,
        source_info,
        primary_local_info,
        secondary_local_info,
    );

    (panel_rank, component_rank, order_index, source_file_id)
}

fn prefab_panel_titles_for_targets(
    source_file_ids: &[i64],
    source_info: Option<&SourcePrefabInfo>,
    primary_local_info: Option<&PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&PrefabInstanceLocalInfo>,
) -> HashMap<i64, String> {
    let mut raw_titles = Vec::with_capacity(source_file_ids.len());
    let mut counts: HashMap<String, usize> = HashMap::new();

    for source_file_id in source_file_ids {
        let title = prefab_panel_title(
            *source_file_id,
            source_info,
            primary_local_info,
            secondary_local_info,
        );
        *counts.entry(title.clone()).or_insert(0) += 1;
        raw_titles.push((*source_file_id, title));
    }

    raw_titles
        .into_iter()
        .map(|(source_file_id, title)| {
            let disambiguated = if counts.get(&title).copied().unwrap_or(0) > 1 {
                format!("{} [fileID:{}]", title, source_file_id)
            } else {
                title
            };
            (source_file_id, disambiguated)
        })
        .collect()
}

fn prefab_component_resolve_reason(
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    source_load_error: Option<&SourcePrefabLoadError>,
    local_target: Option<&PrefabLocalTargetInfo>,
    class_id: Option<i32>,
    script_class: Option<&str>,
) -> Option<String> {
    // Helper: render the source-side state. When `source_info` is missing we
    // prefer the precise typed error from the cache (e.g. "blob missing at
    // X.fbx.meta", "model importer meta at X.fbx.meta contained no recycleID
    // entries") instead of the historical "source prefab was not loaded"
    // catch-all.
    let source_unloaded_state = || -> String {
        match source_load_error {
            Some(err) => err.describe(),
            None => "source prefab was not loaded".to_string(),
        }
    };
    match class_id {
        Some(1) => None,
        Some(114) if script_class.is_some() => None,
        Some(114) => {
            let source_state = match source_info {
                None => source_unloaded_state(),
                Some(info) if info.backing_kind == SourcePrefabBacking::ModelImporterMeta => {
                    // MonoBehaviour from a model importer is impossible —
                    // ModelImporter never emits user scripts. Note the
                    // mismatch so the UI can flag it.
                    "source is a model importer (.fbx etc.) which cannot define MonoBehaviour scripts".to_string()
                }
                Some(info) if info.doc_by_file_id.contains_key(&source_file_id) => {
                    "source prefab entry was found but the script class name could not be resolved"
                        .to_string()
                }
                Some(_) => "source prefab did not contain this source fileID".to_string(),
            };
            let local_state = if local_target.is_some() {
                "local stripped mapping exists but does not include a script class name"
            } else {
                "local stripped mapping is also missing"
            };
            Some(format!(
                "{}; {}; fell back to MonoBehaviour (source_file_id={})",
                source_state, local_state, source_file_id
            ))
        }
        Some(_) => None,
        None => {
            let source_state = match source_info {
                None => source_unloaded_state(),
                Some(info) if info.backing_kind == SourcePrefabBacking::ModelImporterMeta => {
                    if info.order_index_by_file_id.contains_key(&source_file_id) {
                        "model importer meta was loaded and contains this fileID, but no classId could be determined".to_string()
                    } else {
                        "model importer meta was loaded but does not contain this source fileID"
                            .to_string()
                    }
                }
                Some(info) if info.doc_by_file_id.contains_key(&source_file_id) => {
                    "source prefab entry was found but classId is missing".to_string()
                }
                Some(_) => "source prefab did not contain this source fileID".to_string(),
            };
            let local_state = if local_target.is_some() {
                "local stripped mapping exists but does not provide enough metadata"
            } else {
                "local stripped mapping is also missing"
            };
            Some(format!(
                "{}; {}; failed to resolve the concrete component name (source_file_id={})",
                source_state, local_state, source_file_id
            ))
        }
    }
}

/// Determine panel kind and component metadata for a source target.
/// Returns (panel_kind, class_id, component_type, component_source, component_resolve_reason).
pub(crate) fn prefab_panel_meta(
    source_file_id: i64,
    source_info: Option<&SourcePrefabInfo>,
    source_load_error: Option<&SourcePrefabLoadError>,
    primary_local_info: Option<&PrefabInstanceLocalInfo>,
    secondary_local_info: Option<&PrefabInstanceLocalInfo>,
) -> (
    InspectorPanelKind,
    Option<i32>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    // Get actual class_id from source prefab docs. Do NOT use PrefabSourceRef.type_id
    // as a fallback — it contains the Unity reference type (2=component, 3=external),
    // not the class_id.
    let source_class_id =
        source_info.and_then(|info| info.class_id_by_file_id.get(&source_file_id).copied());
    let local_target =
        prefab_local_target(source_file_id, primary_local_info, secondary_local_info);
    let class_id = source_class_id.or_else(|| local_target.map(|target| target.class_id));

    let source_kind_for_builtin = |info: Option<&SourcePrefabInfo>| -> Option<String> {
        info.and_then(|i| match i.backing_kind {
            SourcePrefabBacking::ModelImporterMeta => Some("modelImporterMeta".into()),
            SourcePrefabBacking::YamlPrefab => None,
        })
    };

    match class_id {
        Some(1) => {
            let component_source =
                source_kind_for_builtin(source_info).unwrap_or_else(|| "gameObjectHeader".into());
            (
                InspectorPanelKind::GameObjectHeader,
                Some(1),
                Some("GameObject".into()),
                Some(component_source),
                None,
            )
        }
        Some(114) => {
            let script_class = source_info
                .and_then(|info| info.component_label_by_file_id.get(&source_file_id))
                .cloned()
                .or_else(|| local_target.map(|target| target.component_label.clone()));
            let comp_type = script_class
                .clone()
                .unwrap_or_else(|| "MonoBehaviour".into());
            let resolve_reason = prefab_component_resolve_reason(
                source_file_id,
                source_info,
                source_load_error,
                local_target,
                Some(114),
                script_class.as_deref(),
            );
            (
                InspectorPanelKind::Component,
                Some(114),
                Some(comp_type),
                Some("script".into()),
                resolve_reason,
            )
        }
        Some(cid) if cid > 0 => {
            let comp_type = unity_class_name(cid).to_string();
            let component_source =
                source_kind_for_builtin(source_info).unwrap_or_else(|| "builtin".into());
            (
                InspectorPanelKind::Component,
                Some(cid),
                Some(comp_type),
                Some(component_source),
                None,
            )
        }
        _ => {
            // Source prefab not loaded, fileID not found, or model meta loaded
            // without an explicit class id. As a last resort try the legacy
            // short-fileID heuristic (only effective when source_info is a
            // model importer meta and the fileID matches the legacy
            // classID*100000+N layout — modern 64-bit recycleIDs return None).
            let model_meta_loaded = source_info
                .map(|i| i.backing_kind == SourcePrefabBacking::ModelImporterMeta)
                .unwrap_or(false);
            let heuristic_cid = if model_meta_loaded {
                super::model_meta::legacy_class_id_from_short_file_id(source_file_id)
            } else {
                None
            };
            let component_source = if heuristic_cid.is_some() {
                "modelImporterMetaHeuristic".to_string()
            } else if model_meta_loaded {
                "modelImporterMeta".to_string()
            } else {
                "builtin".to_string()
            };
            let comp_type = heuristic_cid.map(|cid| unity_class_name(cid).to_string());
            let panel_kind = match heuristic_cid {
                Some(1) => InspectorPanelKind::GameObjectHeader,
                _ => InspectorPanelKind::Component,
            };
            (
                panel_kind,
                heuristic_cid,
                comp_type,
                Some(component_source),
                prefab_component_resolve_reason(
                    source_file_id,
                    source_info,
                    source_load_error,
                    local_target,
                    heuristic_cid,
                    None,
                ),
            )
        }
    }
}

fn apply_prefab_override_component_inference(
    source_file_id: i64,
    old_overrides: &[&PropertyOverride],
    new_overrides: &[&PropertyOverride],
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

    let inferred = infer_component_from_override_groups(&[old_overrides, new_overrides])?;
    *panel_kind = if inferred.component_type == "GameObject" {
        InspectorPanelKind::GameObjectHeader
    } else {
        InspectorPanelKind::Component
    };
    *component_type = Some(inferred.component_type.clone());
    *component_source = Some("inferred".into());
    Some(inferred.to_inspector_inference())
}

/// Build semantic panels for a prefab instance, replacing the old generic "Prefab Overrides" panel.
/// Groups overrides by target source_file_id, creating one panel per source object.
fn build_prefab_instance_panels(
    old_ir: Option<&PrefabInstanceIR>,
    new_ir: Option<&PrefabInstanceIR>,
    source_info: Option<&SourcePrefabInfo>,
    source_load_error: Option<&SourcePrefabLoadError>,
    old_local_info: Option<&PrefabInstanceLocalInfo>,
    new_local_info: Option<&PrefabInstanceLocalInfo>,
    old_docs: &[YamlDoc],
    new_docs: &[YamlDoc],
    old_lines: &[String],
    new_lines: &[String],
    old_ctx: &SideContext,
    new_ctx: &SideContext,
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    env: &mut SemanticBuildEnv,
    include_unchanged: bool,
) -> Vec<InspectorPanel> {
    // Pre-group overrides by target source_file_id: O(O) once instead of O(U·O)
    let mut old_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut new_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut target_ids: BTreeSet<i64> = BTreeSet::new();

    if let Some(ir) = old_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            old_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }
    if let Some(ir) = new_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            new_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }

    let mut panels = Vec::new();
    let mut sorted_target_ids: Vec<_> = target_ids.iter().copied().collect();
    sorted_target_ids.sort_by_key(|source_file_id| {
        prefab_panel_sort_key(*source_file_id, source_info, new_local_info, old_local_info)
    });
    let panel_titles = prefab_panel_titles_for_targets(
        &sorted_target_ids,
        source_info,
        new_local_info,
        old_local_info,
    );

    // Build one panel per source target
    for src_fid in sorted_target_ids {
        let old_overrides = old_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let new_overrides = new_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let old_map = build_override_field_map(&old_overrides, old_ctx, old_labels);
        let new_map = build_override_field_map(&new_overrides, new_ctx, new_labels);

        if old_map.is_empty() && new_map.is_empty() {
            continue;
        }

        let title = panel_titles.get(&src_fid).cloned().unwrap_or_else(|| {
            prefab_panel_title(src_fid, source_info, new_local_info, old_local_info)
        });
        let (
            mut panel_kind,
            class_id,
            mut component_type,
            mut component_source,
            component_resolve_reason,
        ) = prefab_panel_meta(
            src_fid,
            source_info,
            source_load_error,
            new_local_info,
            old_local_info,
        );
        let component_inference = apply_prefab_override_component_inference(
            src_fid,
            old_overrides,
            new_overrides,
            &mut panel_kind,
            class_id,
            &mut component_type,
            &mut component_source,
        );

        // Load script info for MonoBehaviour targets from source prefab
        let script_info = if class_id == Some(114) {
            source_info
                .and_then(|info| {
                    let doc_idx = info.doc_by_file_id.get(&src_fid)?;
                    let doc = info.docs.get(*doc_idx)?;
                    load_script_semantic_info(doc, &info.lines, new_ctx, env)
                })
                .or_else(|| {
                    let target = new_local_info?.targets.get(&src_fid)?;
                    let doc = new_docs.get(target.local_doc_index)?;
                    load_script_semantic_info(doc, new_lines, new_ctx, env)
                })
                .or_else(|| {
                    let target = old_local_info?.targets.get(&src_fid)?;
                    let doc = old_docs.get(target.local_doc_index)?;
                    load_script_semantic_info(doc, old_lines, old_ctx, env)
                })
        } else {
            None
        };

        // Apply label enhancements
        let mut old_map = old_map;
        let mut new_map = new_map;
        let is_script = script_info.is_some();
        apply_field_label_enhancements(&mut old_map, script_info.as_ref());
        apply_field_label_enhancements(&mut new_map, script_info.as_ref());

        // If no script info and this is a builtin component, labels already use
        // prettify_builtin_field_label via field_label_for_path(path, None)

        let hidden_roots = HashSet::new();
        let empty_field_types = HashMap::new();
        let fields = build_inspector_fields(
            &old_map,
            &new_map,
            include_unchanged,
            hidden_roots,
            script_info.as_ref(),
            &empty_field_types,
        );

        let change_kind = if old_overrides.is_empty() && !new_overrides.is_empty() {
            "added"
        } else if !old_overrides.is_empty() && new_overrides.is_empty() {
            "removed"
        } else if fields.is_empty() {
            "unchanged"
        } else if fields.iter().all(|f| f.change_kind == "unchanged") {
            "unchanged"
        } else {
            "modified"
        };

        if !include_unchanged && change_kind == "unchanged" {
            continue;
        }

        let script_class = if is_script {
            component_type.clone()
        } else {
            None
        };

        panels.push(InspectorPanel {
            panel_kind,
            title,
            script_class,
            change_kind: change_kind.to_string(),
            added: change_kind == "added",
            removed: change_kind == "removed",
            component_type,
            component_class_id: class_id,
            component_source,
            component_resolve_reason,
            component_inference,
            fields,
        });
    }

    // Build Removed Components panel
    let old_removed: HashSet<i64> = old_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();
    let new_removed: HashSet<i64> = new_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();

    if !old_removed.is_empty() || !new_removed.is_empty() {
        let all_removed: BTreeSet<i64> = old_removed
            .iter()
            .chain(new_removed.iter())
            .copied()
            .collect();
        let mut fields = Vec::new();
        for fid in &all_removed {
            let in_old = old_removed.contains(fid);
            let in_new = new_removed.contains(fid);
            let change_kind = match (in_old, in_new) {
                (false, true) => "added",
                (true, false) => "removed",
                _ => "unchanged",
            };

            if !include_unchanged && change_kind == "unchanged" {
                continue;
            }

            let label = prefab_panel_title(*fid, source_info, new_local_info, old_local_info);
            fields.push(InspectorField {
                id: format!("removed:{}:{}", fid, change_kind),
                label,
                property_path: format!("removedComponent:{}", fid),
                value_type: "string".into(),
                change_kind: change_kind.to_string(),
                before: if in_old { Some("Removed".into()) } else { None },
                after: if in_new { Some("Removed".into()) } else { None },
                children: Vec::new(),
                reference: None,
                field_type: None,
            });
        }

        if !fields.is_empty() {
            let panel_change = if fields.iter().all(|f| f.change_kind == "unchanged") {
                "unchanged"
            } else if fields.iter().all(|f| f.change_kind == "added") {
                "added"
            } else if fields.iter().all(|f| f.change_kind == "removed") {
                "removed"
            } else {
                "modified"
            };

            if include_unchanged || panel_change != "unchanged" {
                panels.push(InspectorPanel {
                    panel_kind: InspectorPanelKind::SubObject,
                    title: "Removed Components".into(),
                    script_class: None,
                    change_kind: panel_change.to_string(),
                    added: panel_change == "added",
                    removed: panel_change == "removed",
                    component_type: None,
                    component_class_id: None,
                    component_source: Some("subObject".into()),
                    component_resolve_reason: None,
                    component_inference: None,
                    fields,
                });
            }
        }
    }

    panels
}

/// Build both changed and full prefab instance panels in a single pass (readonly).
/// Shares override grouping, script info lookup, and field label work.
fn build_prefab_instance_panels_pair(
    old_ir: Option<&PrefabInstanceIR>,
    new_ir: Option<&PrefabInstanceIR>,
    source_info: Option<&SourcePrefabInfo>,
    source_load_error: Option<&SourcePrefabLoadError>,
    old_local_info: Option<&PrefabInstanceLocalInfo>,
    new_local_info: Option<&PrefabInstanceLocalInfo>,
    old_docs: &[YamlDoc],
    new_docs: &[YamlDoc],
    old_lines: &[String],
    new_lines: &[String],
    old_ctx: &SideContext,
    new_ctx: &SideContext,
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    script_cache: &ScriptInfoCache,
) -> (Vec<InspectorPanel>, Vec<InspectorPanel>) {
    // Pre-group overrides by target source_file_id (once, shared for both views)
    let mut old_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut new_grouped: HashMap<i64, Vec<&crate::asset_db::types::PropertyOverride>> =
        HashMap::new();
    let mut target_ids: BTreeSet<i64> = BTreeSet::new();

    if let Some(ir) = old_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            old_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }
    if let Some(ir) = new_ir {
        for ovr in &ir.property_overrides {
            target_ids.insert(ovr.target.source_file_id);
            new_grouped
                .entry(ovr.target.source_file_id)
                .or_default()
                .push(ovr);
        }
    }

    let mut changed_panels = Vec::new();
    let mut full_panels = Vec::new();
    let mut sorted_target_ids: Vec<_> = target_ids.iter().copied().collect();
    sorted_target_ids.sort_by_key(|source_file_id| {
        prefab_panel_sort_key(*source_file_id, source_info, new_local_info, old_local_info)
    });
    let panel_titles = prefab_panel_titles_for_targets(
        &sorted_target_ids,
        source_info,
        new_local_info,
        old_local_info,
    );

    for src_fid in sorted_target_ids {
        let old_overrides = old_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let new_overrides = new_grouped
            .get(&src_fid)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let old_map = build_override_field_map(old_overrides, old_ctx, old_labels);
        let new_map = build_override_field_map(new_overrides, new_ctx, new_labels);

        if old_map.is_empty() && new_map.is_empty() {
            continue;
        }

        let title = panel_titles.get(&src_fid).cloned().unwrap_or_else(|| {
            prefab_panel_title(src_fid, source_info, new_local_info, old_local_info)
        });
        let (
            mut panel_kind,
            class_id,
            mut component_type,
            mut component_source,
            component_resolve_reason,
        ) = prefab_panel_meta(
            src_fid,
            source_info,
            source_load_error,
            new_local_info,
            old_local_info,
        );
        let component_inference = apply_prefab_override_component_inference(
            src_fid,
            old_overrides,
            new_overrides,
            &mut panel_kind,
            class_id,
            &mut component_type,
            &mut component_source,
        );

        // Load script info for MonoBehaviour targets (readonly)
        let script_info = if class_id == Some(114) {
            source_info
                .and_then(|info| {
                    let doc_idx = info.doc_by_file_id.get(&src_fid)?;
                    let doc = info.docs.get(*doc_idx)?;
                    lookup_script_semantic_info(doc, &info.lines, new_ctx, script_cache)
                })
                .or_else(|| {
                    let target = new_local_info?.targets.get(&src_fid)?;
                    let doc = new_docs.get(target.local_doc_index)?;
                    lookup_script_semantic_info(doc, new_lines, new_ctx, script_cache)
                })
                .or_else(|| {
                    let target = old_local_info?.targets.get(&src_fid)?;
                    let doc = old_docs.get(target.local_doc_index)?;
                    lookup_script_semantic_info(doc, old_lines, old_ctx, script_cache)
                })
        } else {
            None
        };

        let mut old_map = old_map;
        let mut new_map = new_map;
        let is_script = script_info.is_some();
        apply_field_label_enhancements(&mut old_map, script_info.as_ref());
        apply_field_label_enhancements(&mut new_map, script_info.as_ref());

        let hidden_roots = HashSet::new();
        let empty_field_types = HashMap::new();

        let (changed_fields, full_fields) = build_inspector_fields_pair(
            &old_map,
            &new_map,
            hidden_roots,
            script_info.as_ref(),
            &empty_field_types,
        );

        let change_kind = if old_overrides.is_empty() && !new_overrides.is_empty() {
            "added"
        } else if !old_overrides.is_empty() && new_overrides.is_empty() {
            "removed"
        } else if changed_fields.is_empty() {
            "unchanged"
        } else if changed_fields.iter().all(|f| f.change_kind == "unchanged") {
            "unchanged"
        } else {
            "modified"
        };

        let script_class = if is_script {
            component_type.clone()
        } else {
            None
        };

        // Changed panel
        if change_kind != "unchanged" {
            changed_panels.push(InspectorPanel {
                panel_kind: panel_kind.clone(),
                title: title.clone(),
                script_class: script_class.clone(),
                change_kind: change_kind.to_string(),
                added: change_kind == "added",
                removed: change_kind == "removed",
                component_type: component_type.clone(),
                component_class_id: class_id,
                component_source: component_source.clone(),
                component_resolve_reason: component_resolve_reason.clone(),
                component_inference: component_inference.clone(),
                fields: changed_fields,
            });
        }

        // Full panel (always included)
        full_panels.push(InspectorPanel {
            panel_kind,
            title,
            script_class,
            change_kind: change_kind.to_string(),
            added: change_kind == "added",
            removed: change_kind == "removed",
            component_type,
            component_class_id: class_id,
            component_source,
            component_resolve_reason,
            component_inference,
            fields: full_fields,
        });
    }

    // Build Removed Components panels (shared logic for both views)
    let old_removed: HashSet<i64> = old_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();
    let new_removed: HashSet<i64> = new_ir
        .map(|ir| {
            ir.removed_components
                .iter()
                .map(|rc| rc.target.source_file_id)
                .collect()
        })
        .unwrap_or_default();

    if !old_removed.is_empty() || !new_removed.is_empty() {
        let all_removed: BTreeSet<i64> = old_removed
            .iter()
            .chain(new_removed.iter())
            .copied()
            .collect();

        let mut changed_fields = Vec::new();
        let mut full_fields = Vec::new();

        for fid in &all_removed {
            let in_old = old_removed.contains(fid);
            let in_new = new_removed.contains(fid);
            let change_kind = match (in_old, in_new) {
                (false, true) => "added",
                (true, false) => "removed",
                _ => "unchanged",
            };

            let label = prefab_panel_title(*fid, source_info, new_local_info, old_local_info);
            let field = InspectorField {
                id: format!("removed:{}:{}", fid, change_kind),
                label,
                property_path: format!("removedComponent:{}", fid),
                value_type: "string".into(),
                change_kind: change_kind.to_string(),
                before: if in_old { Some("Removed".into()) } else { None },
                after: if in_new { Some("Removed".into()) } else { None },
                children: Vec::new(),
                reference: None,
                field_type: None,
            };

            full_fields.push(field.clone());
            if change_kind != "unchanged" {
                changed_fields.push(field);
            }
        }

        if !full_fields.is_empty() {
            let full_change = if full_fields.iter().all(|f| f.change_kind == "unchanged") {
                "unchanged"
            } else if full_fields.iter().all(|f| f.change_kind == "added") {
                "added"
            } else if full_fields.iter().all(|f| f.change_kind == "removed") {
                "removed"
            } else {
                "modified"
            };

            full_panels.push(InspectorPanel {
                panel_kind: InspectorPanelKind::SubObject,
                title: "Removed Components".into(),
                script_class: None,
                change_kind: full_change.to_string(),
                added: full_change == "added",
                removed: full_change == "removed",
                component_type: None,
                component_class_id: None,
                component_source: Some("subObject".into()),
                component_resolve_reason: None,
                component_inference: None,
                fields: full_fields,
            });

            if !changed_fields.is_empty() {
                let changed_change = if changed_fields.iter().all(|f| f.change_kind == "added") {
                    "added"
                } else if changed_fields.iter().all(|f| f.change_kind == "removed") {
                    "removed"
                } else {
                    "modified"
                };

                changed_panels.push(InspectorPanel {
                    panel_kind: InspectorPanelKind::SubObject,
                    title: "Removed Components".into(),
                    script_class: None,
                    change_kind: changed_change.to_string(),
                    added: changed_change == "added",
                    removed: changed_change == "removed",
                    component_type: None,
                    component_class_id: None,
                    component_source: Some("subObject".into()),
                    component_resolve_reason: None,
                    component_inference: None,
                    fields: changed_fields,
                });
            }
        }
    }

    (changed_panels, full_panels)
}

pub(crate) fn build_workspace_scene_target_inspector(
    scene: &UnityYamlFile,
    side_data: &SceneSemanticSideData,
    target_id: &str,
    file_id: i64,
    side_ctx: &SideContext,
    cwd: &str,
) -> Result<SemanticTargetInspector, AppError> {
    let entry = side_data.entries.get(&file_id).ok_or_else(|| {
        AppError::new(
            "asset.preview.target_not_found",
            format!("GameObject not found in session: {}", target_id),
        )
    })?;

    let object_docs = scene_root_docs(&scene.docs);
    let object_doc = object_docs.get(&file_id).copied();
    let subtitle = if entry.object_kind == "prefabInstance" {
        prefab_instance_subtitle(side_data.prefab_irs.get(&file_id), side_ctx, None)
    } else {
        Some(entry.path.clone())
    };

    let mut profiler = DiffProfiler::new(
        format!("workspace-semantic-target:{}", target_id),
        true,
        false,
    );
    let mut env = SemanticBuildEnv {
        app_handle: None,
        cwd,
        profiler: &mut profiler,
        batch_reader: None,
        script_cache: ScriptInfoCache::default(),
    };

    let panels = if entry.object_kind == "gameObject" {
        let mut panels = Vec::new();
        if let Some(panel) = build_gameobject_header_panel(object_doc, object_doc, true) {
            panels.push(panel);
        }

        let components = object_doc
            .map(|doc| {
                collect_scene_components(
                    &scene.docs,
                    &scene.lines,
                    doc.file_id,
                    side_ctx,
                    &mut env,
                    Some(&scene.component_index),
                )
            })
            .unwrap_or_default();

        for (_key, title, script_class, comp_doc) in components {
            let (_changed, full) = build_doc_panel_pair(
                InspectorPanelKind::Component,
                title,
                script_class,
                Some(comp_doc),
                Some(comp_doc),
                &scene.lines,
                &scene.lines,
                &side_data.labels,
                &side_data.labels,
                side_ctx,
                side_ctx,
                &mut env,
                Some(comp_doc.class_id),
            );
            if let Some(panel) = full {
                panels.push(panel);
            }
        }

        panels
    } else {
        let prefab_ir = side_data.prefab_irs.get(&file_id);
        let local_info = side_data.prefab_local_infos.get(&file_id);
        let mut source_prefab_cache = SourcePrefabCache::default();

        if let Some(ir) = prefab_ir {
            let guid_hex = guid_to_hex(&ir.source_prefab_guid);
            if !source_prefab_cache.entries.contains_key(&guid_hex) {
                match load_source_prefab_info(
                    &ir.source_prefab_guid,
                    &guid_hex,
                    side_ctx,
                    &mut env,
                    true,
                ) {
                    Ok(info) => {
                        source_prefab_cache.entries.insert(guid_hex, Some(info));
                    }
                    Err(err) => {
                        source_prefab_cache.errors.insert(guid_hex.clone(), err);
                        source_prefab_cache.entries.insert(guid_hex, None);
                    }
                }
            }
        }

        let source_info_key = prefab_ir.map(|ir| guid_to_hex(&ir.source_prefab_guid));
        let source_info = source_info_key
            .as_ref()
            .and_then(|key| source_prefab_cache.entries.get(key))
            .and_then(|value| value.as_ref());
        let source_load_error = source_info_key
            .as_ref()
            .and_then(|key| source_prefab_cache.errors.get(key));

        build_prefab_instance_panels(
            prefab_ir,
            prefab_ir,
            source_info,
            source_load_error,
            local_info,
            local_info,
            &scene.docs,
            &scene.docs,
            &scene.lines,
            &scene.lines,
            side_ctx,
            side_ctx,
            &side_data.labels,
            &side_data.labels,
            &mut env,
            true,
        )
    };

    Ok(SemanticTargetInspector {
        target_id: target_id.to_string(),
        title: entry.label.clone(),
        subtitle,
        path: entry.path.clone(),
        panels,
    })
}

/// Parallel-safe version of `build_scene_target` that uses readonly shared state.
fn build_scene_target_parallel(
    file_id: i64,
    shared: &SceneBuildShared,
    script_cache: &ScriptInfoCache,
    _worker: &mut SceneWorkerEnv,
) -> Option<SceneTargetBuild> {
    let old_doc = shared.old_object_docs.get(&file_id).copied();
    let new_doc = shared.new_object_docs.get(&file_id).copied();
    if old_doc.is_none() && new_doc.is_none() {
        return None;
    }

    let entry = shared
        .new_entries
        .get(&file_id)
        .or_else(|| shared.old_entries.get(&file_id))?;
    let target_id = if entry.object_kind == "prefabInstance" {
        format!("pi:{}", file_id)
    } else {
        format!("go:{}", file_id)
    };

    let mut changed_panels = Vec::new();
    let mut full_panels = Vec::new();
    let subtitle = if entry.object_kind == "prefabInstance" {
        prefab_instance_subtitle(
            shared
                .new_prefab_irs
                .get(&file_id)
                .or_else(|| shared.old_prefab_irs.get(&file_id)),
            &shared.ctx.new,
            Some(&shared.ctx.old),
        )
    } else {
        Some("GameObject".into())
    };

    if entry.object_kind == "gameObject" {
        if let Some(panel) = build_gameobject_header_panel(old_doc, new_doc, false) {
            changed_panels.push(panel);
        }
        if let Some(panel) = build_gameobject_header_panel(old_doc, new_doc, true) {
            full_panels.push(panel);
        }

        let old_components = old_doc
            .map(|doc| {
                collect_scene_components_readonly(
                    shared.old_all_docs,
                    shared.old_lines,
                    doc.file_id,
                    &shared.ctx.old,
                    script_cache,
                    shared.old_component_index,
                )
            })
            .unwrap_or_default();
        let new_components = new_doc
            .map(|doc| {
                collect_scene_components_readonly(
                    shared.new_all_docs,
                    shared.new_lines,
                    doc.file_id,
                    &shared.ctx.new,
                    script_cache,
                    shared.new_component_index,
                )
            })
            .unwrap_or_default();
        let mut old_map = HashMap::new();
        let mut new_map = HashMap::new();
        for (key, title, script_class, doc) in old_components {
            old_map.insert(key, (title, script_class, doc));
        }
        for (key, title, script_class, doc) in new_components {
            new_map.insert(key, (title, script_class, doc));
        }

        let keys: BTreeSet<String> = old_map.keys().chain(new_map.keys()).cloned().collect();
        for key in keys {
            let old_component = old_map.get(&key);
            let new_component = new_map.get(&key);
            let title = new_component
                .map(|(title, _, _)| title.clone())
                .or_else(|| old_component.map(|(title, _, _)| title.clone()))
                .unwrap_or_else(|| "Component".into());
            let script_class = new_component
                .and_then(|(_, script_class, _)| script_class.clone())
                .or_else(|| old_component.and_then(|(_, script_class, _)| script_class.clone()));
            let comp_class_id = new_component
                .map(|(_, _, doc)| doc.class_id)
                .or_else(|| old_component.map(|(_, _, doc)| doc.class_id));

            let (changed, full) = build_doc_panel_pair_readonly(
                InspectorPanelKind::Component,
                title,
                script_class,
                old_component.map(|(_, _, doc)| *doc),
                new_component.map(|(_, _, doc)| *doc),
                shared.old_lines,
                shared.new_lines,
                shared.old_labels,
                shared.new_labels,
                &shared.ctx.old,
                &shared.ctx.new,
                script_cache,
                comp_class_id,
            );
            if let Some(panel) = changed {
                changed_panels.push(panel);
            }
            if let Some(panel) = full {
                full_panels.push(panel);
            }
        }
    } else {
        // Prefab instance: build semantic panels from PrefabInstanceIR
        let old_ir = shared.old_prefab_irs.get(&file_id);
        let new_ir = shared.new_prefab_irs.get(&file_id);

        // Fast skip: if overrides are identical, skip expensive panel building
        if prefab_instance_is_unchanged(old_ir, new_ir) {
            return None;
        }

        let old_local_info = shared.old_prefab_local_infos.get(&file_id);
        let new_local_info = shared.new_prefab_local_infos.get(&file_id);

        let source_guid = new_ir
            .map(|ir| &ir.source_prefab_guid)
            .or_else(|| old_ir.map(|ir| &ir.source_prefab_guid));
        let source_info_key = source_guid.map(guid_to_hex);
        let source_info = source_info_key
            .as_ref()
            .and_then(|key| shared.source_prefab_cache.entries.get(key))
            .and_then(|opt| opt.as_ref());
        let source_load_error = source_info_key
            .as_ref()
            .and_then(|key| shared.source_prefab_cache.errors.get(key));

        let (cp, fp) = build_prefab_instance_panels_pair(
            old_ir,
            new_ir,
            source_info,
            source_load_error,
            old_local_info,
            new_local_info,
            shared.old_all_docs,
            shared.new_all_docs,
            shared.old_lines,
            shared.new_lines,
            &shared.ctx.old,
            &shared.ctx.new,
            shared.old_labels,
            shared.new_labels,
            script_cache,
        );
        changed_panels = cp;
        full_panels = fp;
    }

    let changed_inspector = SemanticTargetInspector {
        target_id: target_id.clone(),
        title: entry.label.clone(),
        subtitle: subtitle.clone(),
        path: entry.path.clone(),
        panels: changed_panels,
    };
    let full_inspector = SemanticTargetInspector {
        target_id: target_id.clone(),
        title: entry.label.clone(),
        subtitle,
        path: entry.path.clone(),
        panels: full_panels,
    };

    let field_changes = count_changed_leaf_fields(
        &changed_inspector
            .panels
            .iter()
            .flat_map(|panel| panel.fields.clone())
            .collect::<Vec<_>>(),
    );
    let component_changes = changed_inspector
        .panels
        .iter()
        .filter(|panel| {
            matches!(
                panel.panel_kind,
                InspectorPanelKind::Component | InspectorPanelKind::SubObject
            )
        })
        .count();
    let change_kind = scene_target_change_kind(
        old_doc.is_some(),
        new_doc.is_some(),
        field_changes > 0 || component_changes > 0,
    );

    if change_kind == "unchanged" {
        return None;
    }

    Some(SceneTargetBuild {
        id: target_id,
        file_id,
        change_kind,
        component_changes,
        field_changes,
        changed_inspector,
        full_inspector,
    })
}

fn build_scene_target(
    file_id: i64,
    old_object_docs: &HashMap<i64, &YamlDoc>,
    new_object_docs: &HashMap<i64, &YamlDoc>,
    old_all_docs: &[YamlDoc],
    new_all_docs: &[YamlDoc],
    old_entries: &HashMap<i64, HierarchyEntry>,
    new_entries: &HashMap<i64, HierarchyEntry>,
    old_lines: &[String],
    new_lines: &[String],
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    ctx: &DiffBuildContext,
    env: &mut SemanticBuildEnv,
    old_prefab_irs: &HashMap<i64, PrefabInstanceIR>,
    new_prefab_irs: &HashMap<i64, PrefabInstanceIR>,
    old_prefab_local_infos: &HashMap<i64, PrefabInstanceLocalInfo>,
    new_prefab_local_infos: &HashMap<i64, PrefabInstanceLocalInfo>,
    source_prefab_cache: &mut SourcePrefabCache,
    old_component_index: &HashMap<i64, Vec<usize>>,
    new_component_index: &HashMap<i64, Vec<usize>>,
) -> Option<SceneTargetBuild> {
    let old_doc = old_object_docs.get(&file_id).copied();
    let new_doc = new_object_docs.get(&file_id).copied();
    if old_doc.is_none() && new_doc.is_none() {
        return None;
    }

    let entry = new_entries
        .get(&file_id)
        .or_else(|| old_entries.get(&file_id))?;
    let target_id = if entry.object_kind == "prefabInstance" {
        format!("pi:{}", file_id)
    } else {
        format!("go:{}", file_id)
    };

    let mut changed_panels = Vec::new();
    let mut full_panels = Vec::new();
    let subtitle = if entry.object_kind == "prefabInstance" {
        prefab_instance_subtitle(
            new_prefab_irs
                .get(&file_id)
                .or_else(|| old_prefab_irs.get(&file_id)),
            &ctx.new,
            Some(&ctx.old),
        )
    } else {
        Some("GameObject".into())
    };

    if entry.object_kind == "gameObject" {
        if let Some(panel) = build_gameobject_header_panel(old_doc, new_doc, false) {
            changed_panels.push(panel);
        }
        if let Some(panel) = build_gameobject_header_panel(old_doc, new_doc, true) {
            full_panels.push(panel);
        }

        let old_components = old_doc
            .map(|doc| {
                collect_scene_components(
                    old_all_docs,
                    old_lines,
                    doc.file_id,
                    &ctx.old,
                    env,
                    Some(old_component_index),
                )
            })
            .unwrap_or_default();
        let new_components = new_doc
            .map(|doc| {
                collect_scene_components(
                    new_all_docs,
                    new_lines,
                    doc.file_id,
                    &ctx.new,
                    env,
                    Some(new_component_index),
                )
            })
            .unwrap_or_default();
        let mut old_map = HashMap::new();
        let mut new_map = HashMap::new();
        for (key, title, script_class, doc) in old_components {
            old_map.insert(key, (title, script_class, doc));
        }
        for (key, title, script_class, doc) in new_components {
            new_map.insert(key, (title, script_class, doc));
        }

        let keys: BTreeSet<String> = old_map.keys().chain(new_map.keys()).cloned().collect();
        for key in keys {
            let old_component = old_map.get(&key);
            let new_component = new_map.get(&key);
            let title = new_component
                .map(|(title, _, _)| title.clone())
                .or_else(|| old_component.map(|(title, _, _)| title.clone()))
                .unwrap_or_else(|| "Component".into());
            let script_class = new_component
                .and_then(|(_, script_class, _)| script_class.clone())
                .or_else(|| old_component.and_then(|(_, script_class, _)| script_class.clone()));
            let comp_class_id = new_component
                .map(|(_, _, doc)| doc.class_id)
                .or_else(|| old_component.map(|(_, _, doc)| doc.class_id));

            let (changed, full) = build_doc_panel_pair(
                InspectorPanelKind::Component,
                title,
                script_class,
                old_component.map(|(_, _, doc)| *doc),
                new_component.map(|(_, _, doc)| *doc),
                old_lines,
                new_lines,
                old_labels,
                new_labels,
                &ctx.old,
                &ctx.new,
                env,
                comp_class_id,
            );
            if let Some(panel) = changed {
                changed_panels.push(panel);
            }
            if let Some(panel) = full {
                full_panels.push(panel);
            }
        }
    } else {
        // Prefab instance: build semantic panels from PrefabInstanceIR
        let old_ir = old_prefab_irs.get(&file_id);
        let new_ir = new_prefab_irs.get(&file_id);

        // Fast skip: if overrides are identical, no need to build panels
        if prefab_instance_is_unchanged(old_ir, new_ir) {
            return None;
        }

        let old_local_info = old_prefab_local_infos.get(&file_id);
        let new_local_info = new_prefab_local_infos.get(&file_id);

        // Resolve source prefab (prefer new side, fallback to old)
        let source_guid = new_ir
            .map(|ir| &ir.source_prefab_guid)
            .or_else(|| old_ir.map(|ir| &ir.source_prefab_guid));

        let source_info_key = source_guid.map(guid_to_hex);
        if let Some(guid) = source_guid {
            // Ensure source prefab is loaded/cached; try new side first, then old.
            // We only cache the OLD-side error if the NEW-side attempt also
            // failed (the old-side error is the "more interesting" one because
            // the new-side attempt comes first and tends to fail with the
            // generic "GUID not in workspace yet" mode).
            let guid_hex = guid_to_hex(guid);
            if !source_prefab_cache.entries.contains_key(&guid_hex) {
                let result = match load_source_prefab_info(guid, &guid_hex, &ctx.new, env, true) {
                    Ok(info) => Ok(info),
                    Err(new_err) => {
                        match load_source_prefab_info(guid, &guid_hex, &ctx.old, env, false) {
                            Ok(info) => Ok(info),
                            Err(old_err) => Err(pick_more_specific_error(new_err, old_err)),
                        }
                    }
                };
                match result {
                    Ok(info) => {
                        source_prefab_cache.entries.insert(guid_hex, Some(info));
                    }
                    Err(err) => {
                        source_prefab_cache.errors.insert(guid_hex.clone(), err);
                        source_prefab_cache.entries.insert(guid_hex, None);
                    }
                }
            }
        }
        let source_info = source_info_key
            .as_ref()
            .and_then(|key| source_prefab_cache.entries.get(key))
            .and_then(|opt| opt.as_ref());
        let source_load_error = source_info_key
            .as_ref()
            .and_then(|key| source_prefab_cache.errors.get(key));

        changed_panels = build_prefab_instance_panels(
            old_ir,
            new_ir,
            source_info,
            source_load_error,
            old_local_info,
            new_local_info,
            old_all_docs,
            new_all_docs,
            old_lines,
            new_lines,
            &ctx.old,
            &ctx.new,
            old_labels,
            new_labels,
            env,
            false,
        );
        full_panels = build_prefab_instance_panels(
            old_ir,
            new_ir,
            source_info,
            source_load_error,
            old_local_info,
            new_local_info,
            old_all_docs,
            new_all_docs,
            old_lines,
            new_lines,
            &ctx.old,
            &ctx.new,
            old_labels,
            new_labels,
            env,
            true,
        );
    }

    let changed_inspector = SemanticTargetInspector {
        target_id: target_id.clone(),
        title: entry.label.clone(),
        subtitle: subtitle.clone(),
        path: entry.path.clone(),
        panels: changed_panels,
    };
    let full_inspector = SemanticTargetInspector {
        target_id: target_id.clone(),
        title: entry.label.clone(),
        subtitle,
        path: entry.path.clone(),
        panels: full_panels,
    };

    let field_changes = count_changed_leaf_fields(
        &changed_inspector
            .panels
            .iter()
            .flat_map(|panel| panel.fields.clone())
            .collect::<Vec<_>>(),
    );
    let component_changes = changed_inspector
        .panels
        .iter()
        .filter(|panel| {
            matches!(
                panel.panel_kind,
                InspectorPanelKind::Component | InspectorPanelKind::SubObject
            )
        })
        .count();
    let change_kind = scene_target_change_kind(
        old_doc.is_some(),
        new_doc.is_some(),
        field_changes > 0 || component_changes > 0,
    );

    if change_kind == "unchanged" {
        return None;
    }

    Some(SceneTargetBuild {
        id: target_id,
        file_id,
        change_kind,
        component_changes,
        field_changes,
        changed_inspector,
        full_inspector,
    })
}

pub(crate) fn node_id_from_entry(entry: &HierarchyEntry) -> String {
    // Both forms are keyed by file_id, which is the only stable identifier.
    // `entry.path` is built from GameObject names and Unity allows duplicate
    // sibling names, so `go:<path>` is NOT unique and produced colliding
    // SemanticTreeNode ids in scenes with same-named siblings.
    if entry.object_kind == "prefabInstance" {
        format!("pi:{}", entry.file_id)
    } else {
        format!("go:{}", entry.file_id)
    }
}

pub(crate) fn build_scene_tree(
    targets: &[SceneTargetBuild],
    old_entries: &HashMap<i64, HierarchyEntry>,
    new_entries: &HashMap<i64, HierarchyEntry>,
) -> Vec<SemanticTreeNode> {
    let target_map: HashMap<i64, &SceneTargetBuild> = targets
        .iter()
        .map(|target| (target.file_id, target))
        .collect();
    let mut include_ids: HashSet<i64> = target_map.keys().copied().collect();

    for file_id in target_map.keys() {
        let mut cursor = Some(*file_id);
        while let Some(current) = cursor {
            let entry = new_entries
                .get(&current)
                .or_else(|| old_entries.get(&current));
            let Some(entry) = entry else {
                break;
            };
            include_ids.insert(entry.file_id);
            cursor = entry.parent_id;
        }
    }

    let mut chosen_entries = HashMap::new();
    for file_id in &include_ids {
        if let Some(entry) = new_entries
            .get(file_id)
            .or_else(|| old_entries.get(file_id))
        {
            chosen_entries.insert(*file_id, entry.clone());
        }
    }

    let mut children_map: HashMap<Option<i64>, Vec<HierarchyEntry>> = HashMap::new();
    for entry in chosen_entries.values() {
        children_map
            .entry(entry.parent_id)
            .or_default()
            .push(entry.clone());
    }
    for children in children_map.values_mut() {
        children.sort_by_key(|entry| entry.order);
    }

    fn accumulate_badges(
        file_id: i64,
        children_map: &HashMap<Option<i64>, Vec<HierarchyEntry>>,
        target_map: &HashMap<i64, &SceneTargetBuild>,
        badges: &mut HashMap<i64, SemanticBadgeCounts>,
    ) -> SemanticBadgeCounts {
        let mut badge = SemanticBadgeCounts::default();
        if let Some(target) = target_map.get(&file_id) {
            match target.change_kind.as_str() {
                "added" => badge.added += 1,
                "removed" => badge.removed += 1,
                "modified" => badge.modified += 1,
                _ => {}
            }
            badge.components_changed += target.component_changes;
        }
        if let Some(children) = children_map.get(&Some(file_id)) {
            for child in children {
                let child_badge =
                    accumulate_badges(child.file_id, children_map, target_map, badges);
                badge.added += child_badge.added;
                badge.removed += child_badge.removed;
                badge.modified += child_badge.modified;
                badge.components_changed += child_badge.components_changed;
            }
        }
        badges.insert(file_id, badge.clone());
        badge
    }

    let mut badges = HashMap::new();
    if let Some(roots) = children_map.get(&None) {
        for root in roots {
            accumulate_badges(root.file_id, &children_map, &target_map, &mut badges);
        }
    }

    fn walk(
        parent_id: Option<i64>,
        children_map: &HashMap<Option<i64>, Vec<HierarchyEntry>>,
        target_map: &HashMap<i64, &SceneTargetBuild>,
        badges: &HashMap<i64, SemanticBadgeCounts>,
        out: &mut Vec<SemanticTreeNode>,
    ) {
        let Some(children) = children_map.get(&parent_id) else {
            return;
        };
        for child in children {
            let id = node_id_from_entry(child);
            let change_kind = target_map
                .get(&child.file_id)
                .map(|target| target.change_kind.clone())
                .unwrap_or_else(|| "unchanged".into());
            let child_ids = children_map
                .get(&Some(child.file_id))
                .map(|items| items.iter().map(node_id_from_entry).collect())
                .unwrap_or_default();

            out.push(SemanticTreeNode {
                id: id.clone(),
                parent_id: parent_id.and_then(|pid| {
                    children_map
                        .values()
                        .flat_map(|items| items.iter())
                        .find(|entry| entry.file_id == pid)
                        .map(node_id_from_entry)
                }),
                label: child.label.clone(),
                object_kind: child.object_kind.clone(),
                change_kind,
                path: child.path.clone(),
                child_ids,
                badge_counts: badges.get(&child.file_id).cloned().unwrap_or_default(),
                has_inspector: target_map.contains_key(&child.file_id),
            });
            walk(Some(child.file_id), children_map, target_map, badges, out);
        }
    }

    let mut out = Vec::new();
    walk(None, &children_map, &target_map, &badges, &mut out);
    out
}

pub(crate) fn build_scene_semantic_session(
    old_content: &str,
    new_content: &str,
    path: &str,
    ctx: &DiffBuildContext,
    cwd: &str,
    app_handle: &tauri::AppHandle,
    profiler: &mut DiffProfiler,
) -> Option<crate::diff::service::SemanticSession> {
    // Emit ParseYaml phase BEFORE parsing so UI shows progress during the work
    emit_diff_progress(app_handle, profiler, DiffPhase::ParseYaml, None);
    let (old_docs, new_docs) = rayon::join(
        || parse_yaml_docs(old_content.as_bytes()),
        || parse_yaml_docs(new_content.as_bytes()),
    );
    profiler.set_doc_counts(old_docs.len(), new_docs.len());
    profiler.record(DiffPhase::ParseYaml);
    if old_docs.is_empty() && new_docs.is_empty() {
        return None;
    }

    let mut lap = profiler.elapsed_ms();

    let (old_lines, new_lines) = rayon::join(
        || {
            old_content
                .lines()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        },
        || {
            new_content
                .lines()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        },
    );

    let old_object_docs: HashMap<i64, &YamlDoc> = old_docs
        .iter()
        .filter(|doc| doc.class_id == 1 || doc.class_id == 1001)
        .map(|doc| (doc.file_id, doc))
        .collect();
    let new_object_docs: HashMap<i64, &YamlDoc> = new_docs
        .iter()
        .filter(|doc| doc.class_id == 1 || doc.class_id == 1001)
        .map(|doc| (doc.file_id, doc))
        .collect();

    let (old_entries, new_entries) = rayon::join(
        || collect_hierarchy_entries(&build_go_tree(&old_docs), &old_object_docs),
        || collect_hierarchy_entries(&build_go_tree(&new_docs), &new_object_docs),
    );

    let now = profiler.elapsed_ms();
    profiler.record_sub_phase("sem.hierarchy", now - lap);
    lap = now;

    let batch_reader = if matches!(
        ctx.old.file_source,
        SideFileSource::GitRef(_) | SideFileSource::GitIndex | SideFileSource::GitStage(_)
    ) || matches!(
        ctx.new.file_source,
        SideFileSource::GitRef(_) | SideFileSource::GitIndex | SideFileSource::GitStage(_)
    ) {
        BatchBlobReader::new(cwd)
    } else {
        None
    };
    let mut env = SemanticBuildEnv {
        app_handle: Some(app_handle.clone()),
        cwd,
        profiler,
        batch_reader,
        script_cache: ScriptInfoCache::default(),
    };
    // Emit BuildSemantic BEFORE the actual build so UI shows progress during the work
    emit_diff_progress(app_handle, env.profiler, DiffPhase::BuildSemantic, None);

    // Pre-warm scene script cache so labels can use readonly lookup
    {
        for doc in old_docs.iter().filter(|d| d.class_id == 114) {
            let _ = load_script_semantic_info(doc, &old_lines, &ctx.old, &mut env);
        }
        for doc in new_docs.iter().filter(|d| d.class_id == 114) {
            let _ = load_script_semantic_info(doc, &new_lines, &ctx.new, &mut env);
        }
    }

    let now = env.profiler.elapsed_ms();
    env.profiler
        .record_sub_phase("sem.sceneScriptWarm", now - lap);
    lap = now;

    let (old_side, new_side) = rayon::join(
        || {
            build_scene_side_data_from_entries_readonly(
                &old_docs,
                &old_lines,
                old_entries.clone(),
                &ctx.old,
                &env.script_cache,
            )
        },
        || {
            build_scene_side_data_from_entries_readonly(
                &new_docs,
                &new_lines,
                new_entries.clone(),
                &ctx.new,
                &env.script_cache,
            )
        },
    );

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.labels", now - lap);
    lap = now;
    let mut source_prefab_cache = SourcePrefabCache::default();

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.prefabIR", now - lap);
    lap = now;

    // Pre-build component indexes: O(D) once instead of O(D) per target
    let old_component_index = build_component_index(&old_docs);
    let new_component_index = build_component_index(&new_docs);

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.compIndex", now - lap);

    // ── Phase: Source prefab warmup (parallel I/O + parse) ──
    // Each rayon worker gets its own BatchBlobReader (git cat-file process) to avoid
    // serializing LFS smudge operations which dominate prefab load time.
    let mut lap = now;
    {
        let mut unique_guids: Vec<(Guid, String)> = Vec::new();
        let mut seen: HashSet<[u8; 16]> = HashSet::new();
        for ir in old_side
            .prefab_irs
            .values()
            .chain(new_side.prefab_irs.values())
        {
            if seen.insert(ir.source_prefab_guid) {
                let guid_hex = guid_to_hex(&ir.source_prefab_guid);
                if !source_prefab_cache.entries.contains_key(&guid_hex) {
                    unique_guids.push((ir.source_prefab_guid, guid_hex));
                }
            }
        }

        let cwd_ref = cwd;
        let needs_batch = matches!(
            ctx.old.file_source,
            SideFileSource::GitRef(_) | SideFileSource::GitIndex | SideFileSource::GitStage(_)
        ) || matches!(
            ctx.new.file_source,
            SideFileSource::GitRef(_) | SideFileSource::GitIndex | SideFileSource::GitStage(_)
        );

        // Thread-local BatchBlobReader pool: one git process per OS thread.
        use std::cell::RefCell;
        thread_local! {
            static THREAD_BR: RefCell<Option<BatchBlobReader>> = RefCell::new(None);
        }

        let loaded: Vec<(String, Result<SourcePrefabInfo, SourcePrefabLoadError>)> = unique_guids
            .par_iter()
            .map(|(guid, guid_hex)| {
                // Returns (asset_path, load_target, content) on success, or a
                // typed error so the cache can later surface a precise reason
                // to the inspector layer.
                //
                // Read order — git first, then workspace fallback. This
                // matches the on-demand `load_source_prefab_info` path so
                // warmup and slow path agree on which version of the file
                // they consult.
                let load_content = |side_ctx: &SideContext| -> Result<
                        (String, String, String),
                        SourcePrefabLoadError,
                    > {
                        use crate::diff::content::{git_show_file_sync, parse_lfs_pointer};

                        let Some(asset_path) = side_ctx
                            .resolve_script_guid_path(guid)
                            .or_else(|| side_ctx.resolve_guid_path(guid))
                        else {
                            return Err(SourcePrefabLoadError::GuidUnresolved);
                        };

                        let is_model = is_model_importer_asset(&asset_path);
                        let load_target = if is_model {
                            format!("{}.meta", asset_path)
                        } else {
                            asset_path.clone()
                        };

                        let read_workspace = || -> Option<String> {
                            let full_path = Path::new(cwd_ref).join(&load_target);
                            std::fs::read_to_string(full_path).ok()
                        };

                        let read_git = || -> Option<String> {
                            let ref_spec = match &side_ctx.file_source {
                                SideFileSource::Workspace => return None,
                                SideFileSource::GitRef(reference) => {
                                    format!("{}:{}", reference, load_target)
                                }
                                SideFileSource::GitIndex => format!(":{}", load_target),
                                SideFileSource::GitStage(n) => {
                                    format!(":{}:{}", n, load_target)
                                }
                            };
                            let raw = if needs_batch {
                                THREAD_BR.with(|cell| {
                                    let mut opt = cell.borrow_mut();
                                    if opt.is_none() {
                                        *opt = BatchBlobReader::new(cwd_ref);
                                    }
                                    opt.as_mut().and_then(|br| br.read_blob(&ref_spec))
                                })
                            } else {
                                git_show_file_sync(cwd_ref, &ref_spec)
                            };
                            match raw {
                                Some(content) if parse_lfs_pointer(&content).is_some() => None,
                                other => other,
                            }
                        };

                        let content = if matches!(side_ctx.file_source, SideFileSource::Workspace) {
                            read_workspace()
                        } else {
                            read_git().or_else(read_workspace)
                        };

                        match content {
                            Some(c) => Ok((asset_path, load_target, c)),
                            None => Err(SourcePrefabLoadError::BlobMissing {
                                tried_path: load_target,
                            }),
                        }
                    };

                // Try load+classify on the new side first; if either step
                // fails, try the old side. The previous code only retried
                // when `load_content` itself errored — that meant a successful
                // load whose classify reported an empty meta would silently
                // shadow a populated copy on the old side. `attempt` makes
                // load+classify atomic so old-side recovery is reachable.
                let attempt = |side_ctx: &SideContext,
                               is_new_side: bool|
                 -> Result<SourcePrefabInfo, SourcePrefabLoadError> {
                    let (asset_path, load_target, content) = load_content(side_ctx)?;
                    classify_loaded_content(
                        &asset_path,
                        &load_target,
                        content,
                        side_ctx,
                        is_new_side,
                    )
                };

                let result = match attempt(&ctx.new, true) {
                    Ok(info) => Ok(info),
                    Err(new_err) => match attempt(&ctx.old, false) {
                        Ok(info) => Ok(info),
                        Err(old_err) => Err(pick_more_specific_error(new_err, old_err)),
                    },
                };

                (guid_hex.clone(), result)
            })
            .collect();

        for (guid_hex, result) in loaded {
            match result {
                Ok(info) => {
                    source_prefab_cache.entries.insert(guid_hex, Some(info));
                }
                Err(err) => {
                    source_prefab_cache.errors.insert(guid_hex.clone(), err);
                    source_prefab_cache.entries.insert(guid_hex, None);
                }
            }
        }
    }

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.prefabWarm", now - lap);
    lap = now;

    // ── Phase: Source prefab script warmup ──
    // Scene scripts already warmed before labels phase; now warm source prefab scripts.
    {
        for entry in source_prefab_cache.entries.values() {
            if let Some(info) = entry {
                let primary_ctx = if info.loaded_from_new_side {
                    &ctx.new
                } else {
                    &ctx.old
                };
                let fallback_ctx = if info.loaded_from_new_side {
                    &ctx.old
                } else {
                    &ctx.new
                };
                for doc in info.docs.iter().filter(|d| d.class_id == 114) {
                    let _ = load_script_semantic_info(doc, &info.lines, primary_ctx, &mut env)
                        .or_else(|| {
                            load_script_semantic_info(doc, &info.lines, fallback_ctx, &mut env)
                        });
                }
            }
        }
    }

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.scriptWarm", now - lap);
    lap = now;

    let file_ids: BTreeSet<i64> = old_side
        .entries
        .keys()
        .chain(new_side.entries.keys())
        .copied()
        .collect();
    let total_file_ids = file_ids.len();

    let use_parallel = std::env::var("LOCUS_DIFF_SCENE_SEQUENTIAL").is_err();

    let targets: Vec<SceneTargetBuild> = if use_parallel {
        let shared = SceneBuildShared {
            old_object_docs: &old_object_docs,
            new_object_docs: &new_object_docs,
            old_all_docs: &old_docs,
            new_all_docs: &new_docs,
            old_entries: &old_side.entries,
            new_entries: &new_side.entries,
            old_lines: &old_lines,
            new_lines: &new_lines,
            old_labels: &old_side.labels,
            new_labels: &new_side.labels,
            ctx,
            old_prefab_irs: &old_side.prefab_irs,
            new_prefab_irs: &new_side.prefab_irs,
            old_prefab_local_infos: &old_side.prefab_local_infos,
            new_prefab_local_infos: &new_side.prefab_local_infos,
            source_prefab_cache: &source_prefab_cache,
            old_component_index: &old_component_index,
            new_component_index: &new_component_index,
        };

        let file_ids_vec: Vec<i64> = file_ids.into_iter().collect();
        let cwd_owned = cwd.to_string();
        let miss_count = std::sync::atomic::AtomicU64::new(0);

        let mut results: Vec<SceneTargetBuild> = file_ids_vec
            .par_iter()
            .filter_map(|&fid| {
                let mut worker = SceneWorkerEnv {
                    cwd: cwd_owned.clone(),
                    batch_reader: None, // warmup should cover all I/O
                    miss_count: 0,
                };
                let result =
                    build_scene_target_parallel(fid, &shared, &env.script_cache, &mut worker);
                if worker.miss_count > 0 {
                    miss_count.fetch_add(worker.miss_count, std::sync::atomic::Ordering::Relaxed);
                }
                result
            })
            .collect();

        // Restore stable order (BTreeSet iteration order = sorted by file_id)
        results.sort_by_key(|t| t.file_id);

        let total_miss = miss_count.load(std::sync::atomic::Ordering::Relaxed);
        if total_miss > 0 {
            eprintln!("[diff/scene] parallel miss count: {}", total_miss);
        }

        results
    } else {
        // Sequential fallback (LOCUS_DIFF_SCENE_SEQUENTIAL=1)
        let mut targets = Vec::new();
        for file_id in file_ids {
            if let Some(target) = build_scene_target(
                file_id,
                &old_object_docs,
                &new_object_docs,
                &old_docs,
                &new_docs,
                &old_side.entries,
                &new_side.entries,
                &old_lines,
                &new_lines,
                &old_side.labels,
                &new_side.labels,
                ctx,
                &mut env,
                &old_side.prefab_irs,
                &new_side.prefab_irs,
                &old_side.prefab_local_infos,
                &new_side.prefab_local_infos,
                &mut source_prefab_cache,
                &old_component_index,
                &new_component_index,
            ) {
                targets.push(target);
            }
        }
        targets
    };

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.targets", now - lap);
    eprintln!(
        "[diff/scene] targets loop: iterated={}, produced={}, parallel={}, elapsed={}ms",
        total_file_ids,
        targets.len(),
        use_parallel,
        now - lap
    );
    lap = now;

    if targets.is_empty() {
        return None;
    }

    let summary = SemanticSummary {
        changed_targets: targets.len(),
        changed_objects: targets.len(),
        changed_components: targets.iter().map(|target| target.component_changes).sum(),
        changed_fields: targets.iter().map(|target| target.field_changes).sum(),
    };
    let tree = build_scene_tree(&targets, &old_side.entries, &new_side.entries);
    let default_target_id = tree
        .iter()
        .find(|node| targets.iter().any(|target| target.id == node.id))
        .map(|node| node.id.clone())
        .or_else(|| targets.first().map(|target| target.id.clone()));

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.tree", now - lap);
    let _ = lap;

    if env.script_cache.walkdir_ms > 0 {
        env.profiler
            .record_walkdir_call(env.script_cache.walkdir_ms);
    }
    env.emit_phase(DiffPhase::BuildSemantic);

    Some(crate::diff::service::SemanticSession {
        layout: SemanticLayout::SceneHierarchyInspector,
        asset_kind: unity_asset_kind(path),
        summary,
        default_target_id,
        script_class_name: None,
        tree,
        targets: Vec::new(),
        changed_inspectors: targets
            .iter()
            .map(|target| (target.id.clone(), target.changed_inspector.clone()))
            .collect(),
        full_inspectors: targets
            .iter()
            .map(|target| (target.id.clone(), target.full_inspector.clone()))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::context::{GuidResolver, SideContext, SideFileSource, SourceMode};
    use crate::diff::profiler::DiffProfiler;
    use crate::diff::types::InspectorPanelKind;

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
    fn workspace_scene_target_inspector_uses_shared_prefab_instance_panels() {
        let content = r#"%YAML 1.1
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

        let scene = UnityYamlFile::parse(content.as_bytes());
        let side = snapshot_side();
        let side_data = build_scene_side_data_readonly(
            &scene.docs,
            &scene.lines,
            &side,
            &ScriptInfoCache::default(),
        );

        let inspector =
            build_workspace_scene_target_inspector(&scene, &side_data, "pi:9000", 9000, &side, ".")
                .expect("workspace prefab inspector");

        assert!(
            inspector
                .panels
                .iter()
                .any(|panel| matches!(panel.panel_kind, InspectorPanelKind::Component)
                    && panel.title == "Transform"),
            "workspace inspector should reuse semantic prefab-instance panels instead of header-only fallback"
        );
        assert!(
            inspector
                .panels
                .iter()
                .all(|panel| !matches!(panel.panel_kind, InspectorPanelKind::GameObjectHeader)),
            "prefab instance inspector should not fall back to a synthetic GameObject header"
        );
    }

    #[test]
    fn prefab_instance_panels_use_stripped_transform_metadata_when_source_prefab_is_unavailable() {
        let old_content = r#"%YAML 1.1
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
        let new_content = old_content.replace("value: 0", "value: 1");

        let old_docs = parse_yaml_docs(old_content.as_bytes());
        let new_docs = parse_yaml_docs(new_content.as_bytes());
        let old_lines = old_content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let new_lines = new_content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let old_lines_ref = old_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let new_lines_ref = new_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let side = snapshot_side();

        let old_ir = extract_prefab_instance_irs(&old_docs, &old_lines_ref)
            .into_iter()
            .next()
            .expect("old prefab ir");
        let new_ir = extract_prefab_instance_irs(&new_docs, &new_lines_ref)
            .into_iter()
            .next()
            .expect("new prefab ir");

        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(&mut profiler);
        let old_local_infos =
            build_prefab_instance_local_infos(&old_docs, &old_lines, &old_lines_ref, &side);
        let new_local_infos =
            build_prefab_instance_local_infos(&new_docs, &new_lines, &new_lines_ref, &side);

        let panels = build_prefab_instance_panels(
            Some(&old_ir),
            Some(&new_ir),
            None,
            None,
            old_local_infos.get(&old_ir.local_file_id),
            new_local_infos.get(&new_ir.local_file_id),
            &old_docs,
            &new_docs,
            &old_lines,
            &new_lines,
            &side,
            &side,
            &HashMap::new(),
            &HashMap::new(),
            &mut env,
            false,
        );

        let panel = panels
            .iter()
            .find(|panel| matches!(panel.panel_kind, InspectorPanelKind::Component))
            .expect("transform panel");
        assert_eq!(panel.title, "Transform");
        assert_eq!(panel.component_type.as_deref(), Some("Transform"));
        assert_eq!(panel.component_class_id, Some(4));
    }

    #[test]
    fn prefab_instance_panels_put_gameobject_header_before_components() {
        let old_content = r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &100 stripped
GameObject:
  m_CorrespondingSourceObject: {fileID: 200, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
--- !u!4 &400 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 100, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
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
    - target: {fileID: 100, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_LocalPosition.x
      value: 0
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_Layer
      value: 0
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
"#;
        let new_content = r#"%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &100 stripped
GameObject:
  m_CorrespondingSourceObject: {fileID: 200, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
  m_PrefabInstance: {fileID: 9000}
  m_PrefabAsset: {fileID: 0}
--- !u!4 &400 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 100, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
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
    - target: {fileID: 100, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_LocalPosition.x
      value: 1
      objectReference: {fileID: 0}
    - target: {fileID: 200, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
      propertyPath: m_Layer
      value: 1
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
"#;

        let old_docs = parse_yaml_docs(old_content.as_bytes());
        let new_docs = parse_yaml_docs(new_content.as_bytes());
        let old_lines = old_content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let new_lines = new_content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let old_lines_ref = old_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let new_lines_ref = new_lines
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>();
        let side = snapshot_side();

        let old_ir = extract_prefab_instance_irs(&old_docs, &old_lines_ref)
            .into_iter()
            .next()
            .expect("old prefab ir");
        let new_ir = extract_prefab_instance_irs(&new_docs, &new_lines_ref)
            .into_iter()
            .next()
            .expect("new prefab ir");

        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(&mut profiler);
        let old_local_infos =
            build_prefab_instance_local_infos(&old_docs, &old_lines, &old_lines_ref, &side);
        let new_local_infos =
            build_prefab_instance_local_infos(&new_docs, &new_lines, &new_lines_ref, &side);

        let panels = build_prefab_instance_panels(
            Some(&old_ir),
            Some(&new_ir),
            None,
            None,
            old_local_infos.get(&old_ir.local_file_id),
            new_local_infos.get(&new_ir.local_file_id),
            &old_docs,
            &new_docs,
            &old_lines,
            &new_lines,
            &side,
            &side,
            &HashMap::new(),
            &HashMap::new(),
            &mut env,
            false,
        );

        assert_eq!(panels.len(), 2);
        assert!(matches!(
            panels.first().map(|panel| &panel.panel_kind),
            Some(InspectorPanelKind::GameObjectHeader)
        ));
        assert_eq!(panels[1].component_type.as_deref(), Some("Transform"));
    }

    // ── Helpers for the model-importer (.fbx-backed) tests below ──

    /// Builds a synthetic scene `.prefab` whose only PrefabInstance points at
    /// a model importer source. The instance has overrides on three
    /// `source_file_id`s — typically a GameObject + Transform + a third
    /// component such as MeshRenderer — but emits NO stripped component
    /// stubs, mirroring the real failure mode where local fallback also misses.
    fn build_model_instance_prefab_yaml(target_ids: &[i64]) -> String {
        let mut s = String::from(
            "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!1001 &9000\nPrefabInstance:\n  m_ObjectHideFlags: 0\n  serializedVersion: 2\n  m_Modification:\n    m_TransformParent: {fileID: 0}\n    m_Modifications:\n",
        );
        for (i, &fid) in target_ids.iter().enumerate() {
            s.push_str(&format!(
                "    - target: {{fileID: {}, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}}\n      propertyPath: m_LocalPosition.x\n      value: {}\n      objectReference: {{fileID: 0}}\n",
                fid, i
            ));
        }
        s.push_str("  m_SourcePrefab: {fileID: 100100000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}\n");
        s
    }

    fn build_model_instance_prefab_yaml_with_paths(targets: &[(i64, &str)]) -> String {
        let mut s = String::from(
            "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!1001 &9000\nPrefabInstance:\n  m_ObjectHideFlags: 0\n  serializedVersion: 2\n  m_Modification:\n    m_TransformParent: {fileID: 0}\n    m_Modifications:\n",
        );
        for (i, (fid, path)) in targets.iter().enumerate() {
            s.push_str(&format!(
                "    - target: {{fileID: {}, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}}\n      propertyPath: {}\n      value: {}\n      objectReference: {{fileID: 0}}\n",
                fid, path, i
            ));
        }
        s.push_str("  m_SourcePrefab: {fileID: 100100000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}\n");
        s
    }

    fn build_panels_with_source_info(
        prefab_yaml: &str,
        source_info: Option<&SourcePrefabInfo>,
    ) -> Vec<InspectorPanel> {
        build_panels_with_source_info_and_error(prefab_yaml, source_info, None)
    }

    fn build_panels_with_source_info_and_error(
        prefab_yaml: &str,
        source_info: Option<&SourcePrefabInfo>,
        source_load_error: Option<&SourcePrefabLoadError>,
    ) -> Vec<InspectorPanel> {
        let docs = parse_yaml_docs(prefab_yaml.as_bytes());
        let lines = prefab_yaml
            .lines()
            .map(|l| l.to_string())
            .collect::<Vec<_>>();
        let lines_ref = lines.iter().map(|l| l.as_str()).collect::<Vec<_>>();
        let side = snapshot_side();
        let ir = extract_prefab_instance_irs(&docs, &lines_ref)
            .into_iter()
            .next()
            .expect("prefab ir");
        let local_infos = build_prefab_instance_local_infos(&docs, &lines, &lines_ref, &side);

        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(&mut profiler);
        // Use the same content for both sides — value differences don't
        // matter; we only care about how panels resolve titles/types.
        build_prefab_instance_panels(
            Some(&ir),
            Some(&ir),
            source_info,
            source_load_error,
            local_infos.get(&ir.local_file_id),
            local_infos.get(&ir.local_file_id),
            &docs,
            &docs,
            &lines,
            &lines,
            &side,
            &side,
            &HashMap::new(),
            &HashMap::new(),
            &mut env,
            true, // include unchanged so single-side fixtures still produce panels
        )
    }

    #[test]
    fn prefab_instance_panels_disambiguate_repeated_source_component_labels() {
        let first_rect: i64 = 881729730001825605;
        let second_rect: i64 = 2658705397569961675;
        let prefab = build_model_instance_prefab_yaml_with_paths(&[
            (first_rect, "m_AnchorMax.y"),
            (second_rect, "m_AnchorMin.y"),
        ]);

        let mut component_label_by_file_id = HashMap::new();
        component_label_by_file_id.insert(first_rect, "RectTransform".to_string());
        component_label_by_file_id.insert(second_rect, "RectTransform".to_string());

        let mut class_id_by_file_id = HashMap::new();
        class_id_by_file_id.insert(first_rect, 224);
        class_id_by_file_id.insert(second_rect, 224);

        let mut order_index_by_file_id = HashMap::new();
        order_index_by_file_id.insert(first_rect, 0);
        order_index_by_file_id.insert(second_rect, 1);

        let info = SourcePrefabInfo {
            docs: Vec::new(),
            lines: Vec::new(),
            doc_by_file_id: HashMap::new(),
            hierarchy_path_by_file_id: HashMap::new(),
            owner_go_by_component: HashMap::new(),
            component_label_by_file_id,
            class_id_by_file_id,
            order_index_by_file_id,
            loaded_from_new_side: true,
            backing_kind: SourcePrefabBacking::YamlPrefab,
        };

        let panels = build_panels_with_source_info(&prefab, Some(&info));

        assert_eq!(panels.len(), 2);
        assert_eq!(panels[0].title, "RectTransform [fileID:881729730001825605]");
        assert_eq!(
            panels[1].title,
            "RectTransform [fileID:2658705397569961675]"
        );
        assert!(panels
            .iter()
            .all(|panel| panel.component_type.as_deref() == Some("RectTransform")));
    }

    #[test]
    fn prefab_instance_panels_resolve_modern_recycle_id_from_model_meta() {
        // Real-world failure value: a 64-bit recycleID that the legacy
        // /100000 heuristic cannot crack.
        let go_fid: i64 = 919132149155446097;
        let xform_fid: i64 = 919132149155446098;
        let renderer_fid: i64 = 919132149155446099;

        let meta = format!(
            r#"
fileFormatVersion: 2
guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ModelImporter:
  serializedVersion: 22
  internalIDToNameTable:
  - first:
      1: {go}
    second: //RootNode
  - first:
      4: {xf}
    second: //RootNode
  - first:
      23: {rn}
    second: //RootNode
"#,
            go = go_fid,
            xf = xform_fid,
            rn = renderer_fid
        );

        let info = load_source_prefab_info_from_model_meta(&meta, true)
            .expect("model meta should yield SourcePrefabInfo");
        assert_eq!(info.backing_kind, SourcePrefabBacking::ModelImporterMeta);
        assert!(info.docs.is_empty());
        assert!(
            info.doc_by_file_id.is_empty(),
            "doc_by_file_id must stay empty for model-meta backing"
        );
        assert_eq!(
            info.class_id_by_file_id.get(&renderer_fid).copied(),
            Some(23)
        );
        assert!(info.order_index_by_file_id.contains_key(&renderer_fid));

        let prefab = build_model_instance_prefab_yaml(&[go_fid, xform_fid, renderer_fid]);
        let panels = build_panels_with_source_info(&prefab, Some(&info));

        assert_eq!(
            panels.len(),
            3,
            "expected one panel per overridden source fileID"
        );

        let go_panel = panels
            .iter()
            .find(|p| matches!(p.panel_kind, InspectorPanelKind::GameObjectHeader))
            .expect("game object header");
        assert_eq!(go_panel.component_class_id, Some(1));
        assert_eq!(
            go_panel.component_source.as_deref(),
            Some("modelImporterMeta")
        );

        let xform_panel = panels
            .iter()
            .find(|p| p.component_class_id == Some(4))
            .expect("transform panel");
        assert_eq!(xform_panel.component_type.as_deref(), Some("Transform"));
        assert_eq!(
            xform_panel.component_source.as_deref(),
            Some("modelImporterMeta")
        );
        assert!(xform_panel.component_resolve_reason.is_none());
        // Title should be the synthesized hierarchy path, NOT
        // "Component (fileID:...)".
        assert!(
            !xform_panel.title.starts_with("Component (fileID"),
            "transform title regressed to generic fallback: {}",
            xform_panel.title
        );

        let renderer_panel = panels
            .iter()
            .find(|p| p.component_class_id == Some(23))
            .expect("mesh renderer panel");
        assert_eq!(
            renderer_panel.component_type.as_deref(),
            Some("MeshRenderer")
        );
        assert!(
            !renderer_panel.title.starts_with("Component (fileID"),
            "mesh renderer title regressed to generic fallback: {}",
            renderer_panel.title
        );
    }

    #[test]
    fn prefab_instance_panels_resolve_legacy_short_file_id_from_model_meta() {
        let meta = r#"
fileFormatVersion: 2
guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ModelImporter:
  serializedVersion: 22
  fileIDToRecycleName:
    100000: //RootNode
    400000: //RootNode
    2300000: //RootNode
"#;
        let info = load_source_prefab_info_from_model_meta(meta, true)
            .expect("legacy meta should yield SourcePrefabInfo");
        assert_eq!(info.backing_kind, SourcePrefabBacking::ModelImporterMeta);
        assert_eq!(info.class_id_by_file_id.get(&100000).copied(), Some(1));
        assert_eq!(info.class_id_by_file_id.get(&400000).copied(), Some(4));
        assert_eq!(info.class_id_by_file_id.get(&2300000).copied(), Some(23));

        let prefab = build_model_instance_prefab_yaml(&[100000, 400000, 2300000]);
        let panels = build_panels_with_source_info(&prefab, Some(&info));
        assert_eq!(panels.len(), 3);
        assert!(panels
            .iter()
            .any(|p| matches!(p.panel_kind, InspectorPanelKind::GameObjectHeader)));
        assert!(panels
            .iter()
            .any(|p| p.component_class_id == Some(4)
                && p.component_type.as_deref() == Some("Transform")));
        assert!(panels.iter().any(|p| p.component_class_id == Some(23)
            && p.component_type.as_deref() == Some("MeshRenderer")));
    }

    #[test]
    fn prefab_instance_panels_use_heuristic_when_meta_lacks_class_id() {
        // Synthesize a SourcePrefabInfo that has the fileID present but no
        // class_id — this is the "meta loaded but classID unknown" branch.
        // We pick a legacy short fileID (2300000) so the heuristic can fire;
        // the panel should be tagged `modelImporterMetaHeuristic`.
        let mut order = HashMap::new();
        order.insert(2300000_i64, 0_usize);
        let info = SourcePrefabInfo {
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

        let prefab = build_model_instance_prefab_yaml(&[2300000]);
        let panels = build_panels_with_source_info(&prefab, Some(&info));
        assert_eq!(panels.len(), 1);
        let panel = &panels[0];
        assert_eq!(panel.component_class_id, Some(23));
        assert_eq!(panel.component_type.as_deref(), Some("MeshRenderer"));
        assert_eq!(
            panel.component_source.as_deref(),
            Some("modelImporterMetaHeuristic")
        );
        assert!(
            !panel.title.contains('?'),
            "heuristic marker should live in component_inference, not title: {}",
            panel.title
        );
        assert_eq!(
            panel
                .component_inference
                .as_ref()
                .map(|meta| meta.reason_code.as_str()),
            Some("legacyShortFileId")
        );
    }

    #[test]
    fn prefab_instance_panels_infer_renderer_when_model_meta_lacks_class_id() {
        let renderer_fid: i64 = -7511558181221131132;
        let mut order = HashMap::new();
        order.insert(renderer_fid, 0_usize);
        let info = SourcePrefabInfo {
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

        let prefab = build_model_instance_prefab_yaml_with_paths(&[
            (renderer_fid, "m_Materials.Array.data[0]"),
            (renderer_fid, "m_CastShadows"),
        ]);
        let panels = build_panels_with_source_info(&prefab, Some(&info));
        assert_eq!(panels.len(), 1);
        let panel = &panels[0];
        assert!(panel.component_class_id.is_none());
        assert_eq!(panel.component_type.as_deref(), Some("Renderer"));
        assert_eq!(panel.component_source.as_deref(), Some("inferred"));
        let inference = panel.component_inference.as_ref().expect("inference");
        assert_eq!(inference.reason_code, "propertyPathBuiltinFamily");
        assert_eq!(inference.inferred_class_id, Some(25));
        assert!(inference
            .evidence
            .iter()
            .any(|path| path == "m_Materials.Array.data[0]"));
    }

    #[test]
    fn prefab_instance_panels_report_modern_recycle_id_when_meta_missing_class() {
        // 64-bit recycleID, no entry in class_id_by_file_id, no heuristic
        // possible — make sure we surface the model-importer-meta diagnostic
        // and DO NOT pretend to know the class.
        let huge_fid: i64 = 919132149155446097;
        let mut order = HashMap::new();
        order.insert(huge_fid, 0_usize);
        let info = SourcePrefabInfo {
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

        let prefab = build_model_instance_prefab_yaml(&[huge_fid]);
        let panels = build_panels_with_source_info(&prefab, Some(&info));
        assert_eq!(panels.len(), 1);
        let panel = &panels[0];
        assert!(panel.component_class_id.is_none());
        assert_eq!(panel.component_type.as_deref(), Some("Transform"));
        assert_eq!(panel.component_source.as_deref(), Some("inferred"));
        assert_eq!(
            panel
                .component_inference
                .as_ref()
                .map(|meta| meta.reason_code.as_str()),
            Some("propertyPathUniqueBuiltinComponent")
        );
        let reason = panel
            .component_resolve_reason
            .as_deref()
            .unwrap_or_default();
        assert!(
            reason.contains("model importer meta"),
            "expected model importer meta diagnostic, got: {}",
            reason
        );
    }

    #[test]
    fn is_model_importer_asset_recognizes_common_extensions() {
        assert!(is_model_importer_asset("Assets/Foo/Bar.fbx"));
        assert!(is_model_importer_asset("Assets/Foo/Bar.FBX"));
        assert!(is_model_importer_asset("Assets/Foo/Bar.obj"));
        assert!(is_model_importer_asset("Assets/Foo/Bar.dae"));
        assert!(is_model_importer_asset("Assets/Foo/Bar.blend"));
        assert!(!is_model_importer_asset("Assets/Foo/Bar.prefab"));
        assert!(!is_model_importer_asset("Assets/Foo/Bar.unity"));
        assert!(!is_model_importer_asset("Assets/Foo/Bar.mat"));
    }
}
