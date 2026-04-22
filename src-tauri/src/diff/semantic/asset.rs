use std::collections::{HashMap, HashSet};

use crate::asset_db::types::Guid;
use crate::unity_yaml::{parse_yaml_docs, YamlDoc};

use super::super::service::SemanticSession;
use super::inspector::{build_doc_panel_pair, count_changed_leaf_fields};
use super::parse::parse_doc_field_map;
use super::script::{load_script_semantic_info, ScriptInfoCache};
use super::{
    build_doc_label_map, doc_type_label, emit_diff_progress, resolve_script_class_name,
    unity_class_name, SemanticBuildEnv,
};
use crate::diff::content::{unity_asset_kind, BatchBlobReader};
use crate::diff::context::{DiffBuildContext, SideContext, SideFileSource};
use crate::diff::profiler::{DiffPhase, DiffProfiler};
use crate::diff::types::*;

// ── Asset target types ──

#[derive(Debug, Clone)]
pub(crate) struct AssetTargetBuild {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) path: String,
    pub(crate) change_kind: String,
    pub(crate) script_class: Option<String>,
    pub(crate) field_changes: usize,
    pub(crate) changed_inspector: SemanticTargetInspector,
    pub(crate) full_inspector: SemanticTargetInspector,
    pub(crate) order: usize,
}

/// Composite key for matching YAML docs across old/new sides.
/// Used instead of pure fileID matching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DocMatchKey {
    class_id: i32,
    script_guid: Option<Guid>,
    m_name: Option<String>,
    is_main_object: bool,
}

impl DocMatchKey {
    fn from_doc(doc: &YamlDoc) -> Self {
        Self {
            class_id: doc.class_id,
            script_guid: doc.m_script_guid,
            m_name: doc.m_name.clone(),
            is_main_object: doc.doc_index == 0,
        }
    }
}

// ── Asset doc matching ──

/// Match old/new YAML docs using composite DocMatchKey instead of pure fileID.
/// Returns pairs of (old_doc, new_doc) where at least one side is Some.
pub(crate) fn match_asset_docs<'a>(
    old_docs: &'a [YamlDoc],
    new_docs: &'a [YamlDoc],
) -> Vec<(Option<&'a YamlDoc>, Option<&'a YamlDoc>)> {
    // Group by DocMatchKey
    let mut old_by_key: HashMap<DocMatchKey, Vec<&'a YamlDoc>> = HashMap::new();
    for doc in old_docs {
        old_by_key
            .entry(DocMatchKey::from_doc(doc))
            .or_default()
            .push(doc);
    }
    let mut new_by_key: HashMap<DocMatchKey, Vec<&'a YamlDoc>> = HashMap::new();
    for doc in new_docs {
        new_by_key
            .entry(DocMatchKey::from_doc(doc))
            .or_default()
            .push(doc);
    }

    let mut all_keys: Vec<DocMatchKey> = Vec::new();
    let mut seen = HashSet::new();
    // Prefer new-side ordering (main object first)
    for doc in new_docs {
        let key = DocMatchKey::from_doc(doc);
        if seen.insert(key.clone()) {
            all_keys.push(key);
        }
    }
    for doc in old_docs {
        let key = DocMatchKey::from_doc(doc);
        if seen.insert(key.clone()) {
            all_keys.push(key);
        }
    }

    let mut pairs = Vec::new();
    let mut old_matched: HashSet<i64> = HashSet::new();
    let mut new_matched: HashSet<i64> = HashSet::new();

    for key in &all_keys {
        let old_group = old_by_key.get(key).cloned().unwrap_or_default();
        let new_group = new_by_key.get(key).cloned().unwrap_or_default();
        let pair_count = old_group.len().min(new_group.len());

        // Pair by doc_index order
        for i in 0..pair_count {
            old_matched.insert(old_group[i].file_id);
            new_matched.insert(new_group[i].file_id);
            pairs.push((Some(old_group[i]), Some(new_group[i])));
        }
        // Excess old → removed
        for doc in &old_group[pair_count..] {
            old_matched.insert(doc.file_id);
            pairs.push((Some(*doc), None));
        }
        // Excess new → added
        for doc in &new_group[pair_count..] {
            new_matched.insert(doc.file_id);
            pairs.push((None, Some(*doc)));
        }
    }

    pairs
}

// ── Asset target building ──

fn build_asset_target(
    file_id: i64,
    old_doc: Option<&YamlDoc>,
    new_doc: Option<&YamlDoc>,
    old_lines: &[String],
    new_lines: &[String],
    old_labels: &HashMap<i64, String>,
    new_labels: &HashMap<i64, String>,
    ctx: &DiffBuildContext,
    env: &mut SemanticBuildEnv,
) -> Option<AssetTargetBuild> {
    let doc = new_doc.or(old_doc)?;
    let title = if new_doc.is_some() {
        doc_type_label(doc, new_lines, &ctx.new, env)
    } else {
        doc_type_label(doc, old_lines, &ctx.old, env)
    };

    // Resolve script class for MonoBehaviour docs using the matching script snapshot.
    let script_class = if doc.class_id == 114 {
        if let Some(nd) = new_doc {
            resolve_script_class_name(nd, new_lines, &ctx.new, env)
        } else if let Some(od) = old_doc {
            resolve_script_class_name(od, old_lines, &ctx.old, env)
        } else {
            None
        }
    } else {
        None
    };

    let label = match &doc.m_name {
        Some(name) if !name.is_empty() => {
            if let Some(ref cls) = script_class {
                format!("{} ({})", name, cls)
            } else {
                format!("{} {}", title, name)
            }
        }
        _ => format!("{} (fileID:{})", title, doc.file_id),
    };
    let target_id = format!("doc:{}", file_id);
    let subtitle = Some(format!("fileID:{}", file_id));

    let asset_class_id = Some(doc.class_id);

    let (changed_panel, full_panel) = build_doc_panel_pair(
        InspectorPanelKind::AssetRoot,
        title,
        script_class.clone(),
        old_doc,
        new_doc,
        old_lines,
        new_lines,
        old_labels,
        new_labels,
        &ctx.old,
        &ctx.new,
        env,
        asset_class_id,
    );
    let changed_panels: Vec<_> = changed_panel.into_iter().collect();
    let full_panels: Vec<_> = full_panel.into_iter().collect();

    let field_changes = count_changed_leaf_fields(
        &changed_panels
            .iter()
            .flat_map(|panel| panel.fields.clone())
            .collect::<Vec<_>>(),
    );
    let change_kind = match (old_doc, new_doc) {
        (None, Some(_)) => "added".to_string(),
        (Some(_), None) => "removed".to_string(),
        _ if field_changes > 0 => "modified".to_string(),
        _ => "unchanged".to_string(),
    };

    if change_kind == "unchanged" {
        return None;
    }

    Some(AssetTargetBuild {
        id: target_id.clone(),
        label: label.clone(),
        subtitle: subtitle.clone(),
        path: label.clone(),
        change_kind,
        script_class,
        field_changes,
        changed_inspector: SemanticTargetInspector {
            target_id: target_id.clone(),
            title: label.clone(),
            subtitle: subtitle.clone(),
            path: label.clone(),
            panels: changed_panels,
        },
        full_inspector: SemanticTargetInspector {
            target_id,
            title: label.clone(),
            subtitle,
            path: label,
            panels: full_panels,
        },
        order: doc.line_start,
    })
}

// ── Asset kind inference ──

pub(crate) fn infer_asset_kind_with_script_metadata(
    path: &str,
    old_docs: &[YamlDoc],
    new_docs: &[YamlDoc],
    old_lines: &[String],
    new_lines: &[String],
    ctx: &DiffBuildContext,
    env: &mut SemanticBuildEnv,
) -> UnityAssetKind {
    let fallback = unity_asset_kind(path);
    if !matches!(fallback, UnityAssetKind::GenericYaml) {
        return fallback;
    }

    let new_main = new_docs
        .iter()
        .find(|doc| doc.doc_index == 0 && doc.class_id == 114);
    let old_main = old_docs
        .iter()
        .find(|doc| doc.doc_index == 0 && doc.class_id == 114);

    let script_info = if let Some(doc) = new_main {
        load_script_semantic_info(doc, new_lines, &ctx.new, env)
    } else if let Some(doc) = old_main {
        load_script_semantic_info(doc, old_lines, &ctx.old, env)
    } else {
        None
    };

    if script_info
        .as_ref()
        .and_then(|info| info.base_type.as_deref())
        == Some("ScriptableObject")
    {
        UnityAssetKind::ScriptableObject
    } else {
        fallback
    }
}

// ── Asset semantic session ──

pub(crate) fn build_asset_semantic_session(
    old_content: &str,
    new_content: &str,
    path: &str,
    ctx: &DiffBuildContext,
    cwd: &str,
    app_handle: &tauri::AppHandle,
    profiler: &mut DiffProfiler,
) -> Option<SemanticSession> {
    // Emit ParseYaml phase BEFORE parsing so UI shows progress during the work
    emit_diff_progress(app_handle, profiler, DiffPhase::ParseYaml, None);
    let old_docs = parse_yaml_docs(old_content.as_bytes());
    let new_docs = parse_yaml_docs(new_content.as_bytes());
    profiler.set_doc_counts(old_docs.len(), new_docs.len());
    profiler.record(DiffPhase::ParseYaml);
    eprintln!(
        "[diff/asset] old_docs={}, new_docs={}, old_len={}, new_len={}",
        old_docs.len(),
        new_docs.len(),
        old_content.len(),
        new_content.len()
    );
    if old_docs.is_empty() && new_docs.is_empty() {
        eprintln!("[diff/asset] Both doc lists empty, returning None");
        return None;
    }

    let mut lap = profiler.elapsed_ms();

    let old_lines = old_content
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let new_lines = new_content
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
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

    let old_labels =
        build_doc_label_map(&old_docs, &HashMap::new(), &old_lines, &ctx.old, &mut env);
    let new_labels =
        build_doc_label_map(&new_docs, &HashMap::new(), &new_lines, &ctx.new, &mut env);

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.labels", now - lap);
    lap = now;

    // Phase 1: Use DocMatchKey-based composite pairing instead of pure fileID.
    let doc_pairs = match_asset_docs(&old_docs, &new_docs);
    eprintln!("[diff/asset] doc_pairs={}", doc_pairs.len());

    let mut targets = Vec::new();
    for (old_doc, new_doc) in &doc_pairs {
        let file_id = new_doc.or(*old_doc).map(|doc| doc.file_id).unwrap_or(0);
        if let Some(target) = build_asset_target(
            file_id,
            *old_doc,
            *new_doc,
            &old_lines,
            &new_lines,
            &old_labels,
            &new_labels,
            ctx,
            &mut env,
        ) {
            targets.push(target);
        }
    }

    let now = env.profiler.elapsed_ms();
    env.profiler.record_sub_phase("sem.targets", now - lap);
    let _ = lap;

    eprintln!("[diff/asset] targets built: {}", targets.len());
    if targets.is_empty() {
        eprintln!("[diff/asset] All targets filtered out, returning None");
        return None;
    }
    targets.sort_by_key(|target| target.order);

    if env.script_cache.walkdir_ms > 0 {
        env.profiler
            .record_walkdir_call(env.script_cache.walkdir_ms);
    }
    env.emit_phase(DiffPhase::BuildSemantic);

    Some(SemanticSession {
        layout: SemanticLayout::AssetInspector,
        asset_kind: infer_asset_kind_with_script_metadata(
            path, &old_docs, &new_docs, &old_lines, &new_lines, ctx, &mut env,
        ),
        summary: SemanticSummary {
            changed_targets: targets.len(),
            changed_objects: targets.len(),
            changed_components: 0,
            changed_fields: targets.iter().map(|target| target.field_changes).sum(),
        },
        default_target_id: targets.first().map(|target| target.id.clone()),
        script_class_name: targets
            .first()
            .and_then(|target| target.script_class.clone()),
        tree: Vec::new(),
        targets: targets
            .iter()
            .map(|target| SemanticTargetSummary {
                id: target.id.clone(),
                label: target.label.clone(),
                subtitle: target.subtitle.clone(),
                path: target.path.clone(),
                change_kind: target.change_kind.clone(),
                has_inspector: true,
                target_kind: None,
                script_class: target.script_class.clone(),
                is_main_object: None,
                source_mode: None,
            })
            .collect(),
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
