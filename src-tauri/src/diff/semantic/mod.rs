pub mod asset;
pub(crate) mod component_inference;
pub mod inspector;
pub mod list_matching;
pub mod material;
pub(crate) mod model_meta;
pub mod parse;
pub mod scene;
pub mod script;
pub(crate) mod unity_builtin;

use std::collections::HashMap;

use indexmap::IndexMap;
use tauri::AppHandle;

use crate::asset_db::types::{parse_guid_hex, Guid};
use crate::unity_yaml::YamlDoc;

use super::content::unity_asset_kind;
use super::profiler::{DiffPhase, DiffProfiler};
use super::types::*;

use self::script::{ScriptInfoCache, ScriptSemanticInfo};
pub(crate) use self::unity_builtin::unity_class_name;
use super::content::BatchBlobReader;
use super::context::SideContext;

// ── Shared types ──

#[derive(Debug, Clone)]
pub(crate) struct HierarchyEntry {
    pub(crate) file_id: i64,
    pub(crate) parent_id: Option<i64>,
    pub(crate) label: String,
    pub(crate) path: String,
    pub(crate) object_kind: String,
    pub(crate) order: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFieldLine {
    pub(crate) label: String,
    pub(crate) value: Option<String>,
    pub(crate) reference: Option<InspectorReference>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct FieldTreeNode {
    pub(crate) label: String,
    pub(crate) path: String,
    pub(crate) old_entry: Option<ParsedFieldLine>,
    pub(crate) new_entry: Option<ParsedFieldLine>,
    pub(crate) children: IndexMap<String, FieldTreeNode>,
}

// ── Semantic build environment ──

pub(crate) struct SemanticBuildEnv<'a> {
    pub(crate) app_handle: Option<AppHandle>,
    pub(crate) cwd: &'a str,
    pub(crate) profiler: &'a mut DiffProfiler,
    pub(crate) batch_reader: Option<BatchBlobReader>,
    pub(crate) script_cache: ScriptInfoCache,
}

impl SemanticBuildEnv<'_> {
    pub(crate) fn emit_phase(&mut self, phase: DiffPhase) {
        self.profiler.record(phase);
        if let Some(ref handle) = self.app_handle {
            emit_diff_progress(handle, self.profiler, phase, None);
        }
    }
}

// ── Shared semantic helpers ──

pub(crate) fn emit_diff_progress(
    app_handle: &AppHandle,
    profiler: &DiffProfiler,
    phase: DiffPhase,
    error: Option<String>,
) {
    use tauri::Emitter;
    let _ = app_handle.emit("diff-progress", profiler.progress_event(phase, error));
}

pub(crate) fn extract_script_guid(doc: &YamlDoc, lines: &[String]) -> Option<Guid> {
    for idx in doc.line_start.min(lines.len())..doc.line_end.min(lines.len()) {
        let line = lines[idx].trim();
        if line.starts_with("m_Script:") || line.contains(" m_Script:") {
            let guid = parse::extract_flow_value(line, "guid:")?;
            return parse_guid_hex(guid.trim_end_matches(','));
        }
    }
    None
}

pub(crate) fn resolve_script_class_name(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> Option<String> {
    script::load_script_semantic_info(doc, lines, side_ctx, env)
        .map(|info| info.class_name)
        .or_else(|| {
            let guid = script::doc_script_guid(doc, lines)?;
            let path = side_ctx.resolve_script_guid_path(&guid)?;
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
        })
}

pub(crate) fn doc_type_label(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> String {
    if doc.class_id == 114 {
        if let Some(script) = resolve_script_class_name(doc, lines, side_ctx, env) {
            return script;
        }
    }
    if !doc.type_name.is_empty() {
        doc.type_name.clone()
    } else {
        unity_class_name(doc.class_id).to_string()
    }
}

pub(crate) fn build_doc_label_map(
    docs: &[YamlDoc],
    object_paths: &HashMap<i64, String>,
    lines: &[String],
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> HashMap<i64, String> {
    let mut labels = HashMap::new();
    for doc in docs {
        let label = if let Some(path) = object_paths.get(&doc.file_id) {
            path.clone()
        } else if let Some(go_id) = doc.m_game_object_id {
            let owner = object_paths
                .get(&go_id)
                .cloned()
                .unwrap_or_else(|| format!("GameObject {}", go_id));
            format!("{}/{}", owner, doc_type_label(doc, lines, side_ctx, env))
        } else if let Some(name) = &doc.m_name {
            format!("{}/{}", doc_type_label(doc, lines, side_ctx, env), name)
        } else {
            format!(
                "{} (fileID:{})",
                doc_type_label(doc, lines, side_ctx, env),
                doc.file_id
            )
        };
        labels.insert(doc.file_id, label);
    }
    labels
}

// ── No-I/O variant for prefab local info building ──

/// Like `doc_type_label` but never triggers script file I/O.
/// Uses GUID → path resolution (in-memory) and file_stem extraction only.
pub(crate) fn doc_type_label_no_io(doc: &YamlDoc, side_ctx: &SideContext) -> String {
    if doc.class_id == 114 {
        if let Some(name) = doc
            .m_script_guid
            .and_then(|sg| side_ctx.resolve_script_guid_path(&sg))
            .and_then(|p| {
                std::path::Path::new(&p)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
        {
            return name;
        }
    }
    if !doc.type_name.is_empty() {
        doc.type_name.clone()
    } else {
        unity_class_name(doc.class_id).to_string()
    }
}

// ── Readonly variants for parallel scene diff ──

/// Like `resolve_script_class_name` but reads from a pre-warmed, immutable `ScriptInfoCache`.
/// Returns None on cache miss instead of loading from disk.
pub(crate) fn resolve_script_class_name_readonly(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> Option<String> {
    script::lookup_script_semantic_info(doc, lines, side_ctx, script_cache)
        .map(|info| info.class_name.clone())
        .or_else(|| {
            let guid = script::doc_script_guid(doc, lines)?;
            let path = side_ctx.resolve_script_guid_path(&guid)?;
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
        })
}

/// Like `doc_type_label` but uses readonly script cache.
pub(crate) fn doc_type_label_readonly(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> String {
    if doc.class_id == 114 {
        if let Some(script) = resolve_script_class_name_readonly(doc, lines, side_ctx, script_cache)
        {
            return script;
        }
    }
    if !doc.type_name.is_empty() {
        doc.type_name.clone()
    } else {
        unity_class_name(doc.class_id).to_string()
    }
}

/// Like `build_doc_label_map` but uses readonly script cache. No I/O.
pub(crate) fn build_doc_label_map_readonly(
    docs: &[YamlDoc],
    object_paths: &HashMap<i64, String>,
    lines: &[String],
    side_ctx: &SideContext,
    script_cache: &ScriptInfoCache,
) -> HashMap<i64, String> {
    let mut labels = HashMap::new();
    for doc in docs {
        let label = if let Some(path) = object_paths.get(&doc.file_id) {
            path.clone()
        } else if let Some(go_id) = doc.m_game_object_id {
            let owner = object_paths
                .get(&go_id)
                .cloned()
                .unwrap_or_else(|| format!("GameObject {}", go_id));
            format!(
                "{}/{}",
                owner,
                doc_type_label_readonly(doc, lines, side_ctx, script_cache)
            )
        } else if let Some(name) = &doc.m_name {
            format!(
                "{}/{}",
                doc_type_label_readonly(doc, lines, side_ctx, script_cache),
                name
            )
        } else {
            format!(
                "{} (fileID:{})",
                doc_type_label_readonly(doc, lines, side_ctx, script_cache),
                doc.file_id
            )
        };
        labels.insert(doc.file_id, label);
    }
    labels
}

// ── Semantic session dispatcher ──

pub(crate) fn build_semantic_session(
    path: &str,
    old_content: &str,
    new_content: &str,
    ctx: &super::context::DiffBuildContext,
    cwd: &str,
    app_handle: &AppHandle,
    profiler: &mut DiffProfiler,
) -> Option<super::service::SemanticSession> {
    match unity_asset_kind(path) {
        UnityAssetKind::Scene | UnityAssetKind::Prefab => scene::build_scene_semantic_session(
            old_content,
            new_content,
            path,
            ctx,
            cwd,
            app_handle,
            profiler,
        ),
        _ => asset::build_asset_semantic_session(
            old_content,
            new_content,
            path,
            ctx,
            cwd,
            app_handle,
            profiler,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_db::{AssetDb, AssetDbState};
    use crate::diff::context::{
        DiffBuildContext, GuidResolver, SideContext, SideFileSource, SourceMode,
    };
    use crate::diff::profiler::DiffProfiler;
    use crate::diff::types::*;
    use crate::unity_yaml::parse_yaml_docs;

    use super::asset::infer_asset_kind_with_script_metadata;
    use super::inspector::{build_doc_panel, field_label_for_path};
    use super::parse::{parse_doc_field_map, parse_reference};
    use super::scene::{build_scene_tree, SceneTargetBuild};
    use super::script::{ScriptFieldEnhancement, ScriptInfoCache, ScriptSemanticInfo};

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    fn snapshot_side() -> SideContext<'static> {
        SideContext {
            guid_resolver: GuidResolver::None,
            script_guid_resolver: GuidResolver::None,
            source_mode: SourceMode::Snapshot,
            file_source: SideFileSource::Workspace,
        }
    }

    fn temp_project_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("locus-diff-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets")).expect("create temp Assets dir");
        root
    }

    fn write_text_file(root: &Path, relative_path: &str, content: &str) {
        let full_path = root.join(relative_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(full_path, content).expect("write file");
    }

    fn build_ref_graph_state(root: &Path) -> AssetDbState {
        let mut graph = AssetDb::open(root).expect("open ref graph");
        graph.full_scan(|_| {}).expect("scan ref graph");
        AssetDbState(Arc::new(Mutex::new(Some(graph))))
    }

    fn test_env<'a>(cwd: &'a str, profiler: &'a mut DiffProfiler) -> SemanticBuildEnv<'a> {
        SemanticBuildEnv {
            app_handle: None,
            cwd,
            profiler,
            batch_reader: None,
            script_cache: ScriptInfoCache::default(),
        }
    }

    fn transition_asset(transitions: &str) -> String {
        format!(
            "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_ObjectHideFlags: 0\n  m_CorrespondingSourceObject: {{fileID: 0}}\n  m_PrefabInstance: {{fileID: 0}}\n  m_PrefabAsset: {{fileID: 0}}\n  m_GameObject: {{fileID: 0}}\n  m_Enabled: 1\n  m_EditorHideFlags: 0\n  m_Script: {{fileID: 11500000, guid: 872cbaa965d1f6e4e98365d74e2060df, type: 3}}\n  m_Name: TransitionTable\n  m_EditorClassIdentifier: \n  _transitions:\n{}",
            transitions
        )
    }

    fn flatten_fields<'a>(fields: &'a [InspectorField], out: &mut Vec<(&'a str, &'a str)>) {
        for field in fields {
            out.push((field.property_path.as_str(), field.change_kind.as_str()));
            flatten_fields(&field.children, out);
        }
    }

    fn flatten_field_labels<'a>(fields: &'a [InspectorField], out: &mut Vec<(&'a str, &'a str)>) {
        for field in fields {
            out.push((field.property_path.as_str(), field.label.as_str()));
            flatten_field_labels(&field.children, out);
        }
    }

    #[test]
    fn build_scene_tree_marks_ancestor_nodes_as_noninspectable_and_unchanged() {
        let mut old_entries = HashMap::new();
        let mut new_entries = HashMap::new();

        new_entries.insert(
            1,
            HierarchyEntry {
                file_id: 1,
                parent_id: None,
                label: "prefab".into(),
                path: "prefab".into(),
                object_kind: "gameObject".into(),
                order: 0,
            },
        );
        new_entries.insert(
            2,
            HierarchyEntry {
                file_id: 2,
                parent_id: Some(1),
                label: "child".into(),
                path: "prefab/child".into(),
                object_kind: "gameObject".into(),
                order: 1,
            },
        );

        old_entries.clone_from(&new_entries);

        let targets = vec![SceneTargetBuild {
            id: "go:2".into(),
            file_id: 2,
            change_kind: "modified".into(),
            component_changes: 1,
            field_changes: 2,
            changed_inspector: SemanticTargetInspector {
                target_id: "go:2".into(),
                title: "child".into(),
                subtitle: Some("GameObject".into()),
                path: "prefab/child".into(),
                panels: Vec::new(),
            },
            full_inspector: SemanticTargetInspector {
                target_id: "go:2".into(),
                title: "child".into(),
                subtitle: Some("GameObject".into()),
                path: "prefab/child".into(),
                panels: Vec::new(),
            },
        }];

        let tree = build_scene_tree(&targets, &old_entries, &new_entries);
        let parent = tree
            .iter()
            .find(|node| node.id == "go:1")
            .expect("parent node");
        let child = tree
            .iter()
            .find(|node| node.id == "go:2")
            .expect("child node");

        assert_eq!(parent.change_kind, "unchanged");
        assert!(!parent.has_inspector);
        assert_eq!(parent.badge_counts.modified, 1);
        assert_eq!(parent.badge_counts.components_changed, 1);

        assert_eq!(child.change_kind, "modified");
        assert!(child.has_inspector);
    }

    #[test]
    fn builtin_field_labels_drop_unity_m_prefix() {
        assert_eq!(
            field_label_for_path("m_StaticShadowCaster", None),
            "Static Shadow Caster"
        );
        assert_eq!(field_label_for_path("m_IncludeLayers.m_Bits", None), "Bits");
    }

    #[test]
    fn script_field_labels_keep_script_semantic_aliases() {
        let mut field_aliases = HashMap::new();
        field_aliases.insert(
            "m_MoveSpeed".to_string(),
            ScriptFieldEnhancement {
                canonical_name: "m_MoveSpeed".to_string(),
                display_label: "Move Speed".to_string(),
                hidden: false,
                field_type: Some("float".to_string()),
            },
        );
        let info = ScriptSemanticInfo {
            class_name: "EnemyConfig".to_string(),
            base_type: Some("ScriptableObject".to_string()),
            field_aliases,
        };

        assert_eq!(
            field_label_for_path("m_MoveSpeed", Some(&info)),
            "Move Speed"
        );
        assert_eq!(
            field_label_for_path("nested.m_Value", Some(&info)),
            "M Value"
        );
    }

    #[test]
    fn parse_doc_field_map_keeps_list_siblings_flat() {
        let content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    ToState: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n",
        );
        let docs = parse_yaml_docs(content.as_bytes());
        let lines = content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let field_map = parse_doc_field_map(&docs[0], &lines, &snapshot_side(), &HashMap::new());

        assert!(field_map.contains_key("_transitions[0].FromState"));
        assert!(field_map.contains_key("_transitions[0].Conditions[0].Condition"));
        assert!(field_map.contains_key("_transitions[1].FromState"));
        assert!(field_map.contains_key("_transitions[1].ToState"));
        assert!(!field_map
            .keys()
            .any(|path| path.contains("_transitions[0][0]")));
    }

    #[test]
    fn external_guid_reference_does_not_fall_back_to_local_file_id_label() {
        let mut local_labels = HashMap::new();
        local_labels.insert(11400000, "MonoBehaviour".to_string());

        let reference = parse_reference(
            "{fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}",
            &snapshot_side(),
            &local_labels,
        )
        .expect("reference");

        assert_eq!(
            reference.guid.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(reference.file_id, Some(11400000));
        assert_eq!(reference.path, None);
    }

    #[test]
    fn script_metadata_applies_former_names_and_hide_in_inspector() {
        let root = temp_project_root();
        write_text_file(
            &root,
            "Assets/EnemyConfig.cs",
            r#"
using UnityEngine;
using UnityEngine.Serialization;

public class EnemyConfig : ScriptableObject
{
    [FormerlySerializedAs("oldSpeed")]
    public float moveSpeed;

    [HideInInspector]
    public float _hiddenValue;
}
"#,
        );
        write_text_file(
            &root,
            "Assets/EnemyConfig.cs.meta",
            "fileFormatVersion: 2\nguid: aabbccddaabbccddaabbccddaabbccdd\n",
        );

        let ref_graph_state = build_ref_graph_state(&root);
        let side = SideContext {
            guid_resolver: GuidResolver::None,
            script_guid_resolver: GuidResolver::Workspace(&ref_graph_state),
            source_mode: SourceMode::Workspace,
            file_source: SideFileSource::Workspace,
        };

        let content = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_ObjectHideFlags: 0\n  m_CorrespondingSourceObject: {fileID: 0}\n  m_PrefabInstance: {fileID: 0}\n  m_PrefabAsset: {fileID: 0}\n  m_GameObject: {fileID: 0}\n  m_Enabled: 1\n  m_EditorHideFlags: 0\n  m_Script: {fileID: 11500000, guid: aabbccddaabbccddaabbccddaabbccdd, type: 3}\n  m_Name: EnemyConfig\n  m_EditorClassIdentifier: \n  oldSpeed: 5\n  _hiddenValue: 99\n";
        let docs = parse_yaml_docs(content.as_bytes());
        let lines = content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(root.to_str().expect("temp root"), &mut profiler);
        let panel = build_doc_panel(
            InspectorPanelKind::AssetRoot,
            "EnemyConfig".into(),
            Some("EnemyConfig".into()),
            Some(&docs[0]),
            Some(&docs[0]),
            &lines,
            &lines,
            &HashMap::new(),
            &HashMap::new(),
            &side,
            &side,
            &mut env,
            true,
            Some(114),
        )
        .expect("panel");

        let mut labels = Vec::new();
        flatten_field_labels(&panel.fields, &mut labels);

        assert!(labels
            .iter()
            .any(|(path, label)| *path == "oldSpeed" && *label == "Move Speed"));
        assert!(!labels
            .iter()
            .any(|(path, _)| path.starts_with("_hiddenValue")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scriptable_object_asset_kind_follows_inheritance_chain() {
        let root = temp_project_root();
        write_text_file(
            &root,
            "Assets/BaseConfig.cs",
            r#"
using UnityEngine;

public class BaseConfig : ScriptableObject
{
}
"#,
        );
        write_text_file(
            &root,
            "Assets/EnemyConfig.cs",
            r#"
public class EnemyConfig : BaseConfig
{
    public float speed;
}
"#,
        );
        write_text_file(
            &root,
            "Assets/EnemyConfig.cs.meta",
            "fileFormatVersion: 2\nguid: aabbccddaabbccddaabbccddaabbccdd\n",
        );

        let ref_graph_state = build_ref_graph_state(&root);
        let ctx = DiffBuildContext {
            source: DiffSource::GitUnstaged,
            old: SideContext {
                guid_resolver: GuidResolver::None,
                script_guid_resolver: GuidResolver::Workspace(&ref_graph_state),
                source_mode: SourceMode::Workspace,
                file_source: SideFileSource::Workspace,
            },
            new: SideContext {
                guid_resolver: GuidResolver::Workspace(&ref_graph_state),
                script_guid_resolver: GuidResolver::Workspace(&ref_graph_state),
                source_mode: SourceMode::Workspace,
                file_source: SideFileSource::Workspace,
            },
        };

        let content = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_ObjectHideFlags: 0\n  m_CorrespondingSourceObject: {fileID: 0}\n  m_PrefabInstance: {fileID: 0}\n  m_PrefabAsset: {fileID: 0}\n  m_GameObject: {fileID: 0}\n  m_Enabled: 1\n  m_EditorHideFlags: 0\n  m_Script: {fileID: 11500000, guid: aabbccddaabbccddaabbccddaabbccdd, type: 3}\n  m_Name: EnemyConfig\n  m_EditorClassIdentifier: \n  speed: 5\n";
        let docs = parse_yaml_docs(content.as_bytes());
        let lines = content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(root.to_str().expect("temp root"), &mut profiler);
        let asset_kind = infer_asset_kind_with_script_metadata(
            "Assets/EnemyConfig.asset",
            &docs,
            &docs,
            &lines,
            &lines,
            &ctx,
            &mut env,
        );

        assert!(matches!(asset_kind, UnityAssetKind::ScriptableObject));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compound_list_insertion_keeps_following_item_unchanged() {
        let old_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    ToState: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: ffffffffffffffffffffffffffffffff, type: 2}\n      Operator: 1\n",
        );
        let new_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: 11111111111111111111111111111111, type: 2}\n    ToState: {fileID: 11400000, guid: 22222222222222222222222222222222, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: 33333333333333333333333333333333, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    ToState: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: ffffffffffffffffffffffffffffffff, type: 2}\n      Operator: 1\n",
        );
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
        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(".", &mut profiler);
        let panel = build_doc_panel(
            InspectorPanelKind::AssetRoot,
            "TransitionTable".into(),
            None,
            Some(&old_docs[0]),
            Some(&new_docs[0]),
            &old_lines,
            &new_lines,
            &HashMap::new(),
            &HashMap::new(),
            &snapshot_side(),
            &snapshot_side(),
            &mut env,
            true,
            None,
        )
        .expect("panel");

        let mut flattened = Vec::new();
        flatten_fields(&panel.fields, &mut flattened);

        assert!(flattened
            .iter()
            .any(|(path, kind)| *path == "_transitions[1]" && *kind == "added"));
        assert!(flattened
            .iter()
            .any(|(path, kind)| *path == "_transitions[2]" && *kind == "unchanged"));
        assert!(flattened
            .iter()
            .any(|(path, kind)| *path == "_transitions[2].ToState" && *kind == "unchanged"));
        assert!(!flattened
            .iter()
            .any(|(path, _)| path.contains("_transitions[0][0]")));
    }

    #[test]
    fn compound_list_insertion_stays_clean_in_changed_only_view() {
        let old_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    ToState: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: ffffffffffffffffffffffffffffffff, type: 2}\n      Operator: 1\n",
        );
        let new_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: 11111111111111111111111111111111, type: 2}\n    ToState: {fileID: 11400000, guid: 22222222222222222222222222222222, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: 33333333333333333333333333333333, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    ToState: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: ffffffffffffffffffffffffffffffff, type: 2}\n      Operator: 1\n",
        );
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
        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(".", &mut profiler);
        let panel = build_doc_panel(
            InspectorPanelKind::AssetRoot,
            "TransitionTable".into(),
            None,
            Some(&old_docs[0]),
            Some(&new_docs[0]),
            &old_lines,
            &new_lines,
            &HashMap::new(),
            &HashMap::new(),
            &snapshot_side(),
            &snapshot_side(),
            &mut env,
            false,
            None,
        )
        .expect("panel");

        let mut flattened = Vec::new();
        flatten_fields(&panel.fields, &mut flattened);

        assert!(flattened
            .iter()
            .any(|(path, kind)| *path == "_transitions[1]" && *kind == "added"));
        assert!(!flattened
            .iter()
            .any(|(path, _)| path.starts_with("_transitions[2]")));
    }

    #[test]
    fn repeated_from_state_insertion_uses_from_and_to_to_keep_following_item_unchanged() {
        let old_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n      Operator: 1\n",
        );
        let new_content = transition_asset(
            "  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: ffffffffffffffffffffffffffffffff, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: 11111111111111111111111111111111, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 2}\n    Conditions:\n    - ExpectedResult: 0\n      Condition: {fileID: 11400000, guid: cccccccccccccccccccccccccccccccc, type: 2}\n      Operator: 0\n  - FromState: {fileID: 11400000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 2}\n    ToState: {fileID: 11400000, guid: dddddddddddddddddddddddddddddddd, type: 2}\n    Conditions:\n    - ExpectedResult: 1\n      Condition: {fileID: 11400000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 2}\n      Operator: 1\n",
        );
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
        let mut profiler = DiffProfiler::new("test".into(), false, false);
        let mut env = test_env(".", &mut profiler);
        let panel = build_doc_panel(
            InspectorPanelKind::AssetRoot,
            "TransitionTable".into(),
            None,
            Some(&old_docs[0]),
            Some(&new_docs[0]),
            &old_lines,
            &new_lines,
            &HashMap::new(),
            &HashMap::new(),
            &snapshot_side(),
            &snapshot_side(),
            &mut env,
            false,
            None,
        )
        .expect("panel");

        let mut flattened = Vec::new();
        flatten_fields(&panel.fields, &mut flattened);

        assert!(flattened
            .iter()
            .any(|(path, kind)| *path == "_transitions[0]" && *kind == "added"));
        assert!(!flattened
            .iter()
            .any(|(path, _)| path.starts_with("_transitions[1]")));
        assert!(!flattened
            .iter()
            .any(|(path, _)| path.starts_with("_transitions[2]")));
        assert!(!flattened
            .iter()
            .any(|(path, _)| path.starts_with("_transitions[3]")));
    }
}
