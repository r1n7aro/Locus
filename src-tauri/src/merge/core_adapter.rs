//! Compatibility DTOs for Collab, driven exclusively by the lossless asset core.
//! Field visibility and Inspector labels never determine the merge write set.
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::diff::content::unity_asset_kind;
use crate::diff::types::{
    InspectorPanelKind, SemanticBadgeCounts, SemanticLayout, SemanticTreeNode,
};
use crate::error::{AppError, AppResult};
use crate::process_util::command;
use crate::unity_asset_core::{self as core, ChangeStatus, Decision, Resolution};

use super::patch::AssembledMerge;
use super::types::*;

fn core_error(error: core::CoreError) -> AppError {
    AppError::new(format!("merge.core.{}", error.code), error.message)
        .detail(format!("byte {}", error.offset))
}

pub(crate) fn checked_path(cwd: &str, path: &str) -> AppResult<PathBuf> {
    let relative = Path::new(path);
    if path.is_empty()
        || path.contains('\0')
        || path.contains(':')
        || relative.components().any(|c|matches!(c,Component::Normal(value) if value.to_string_lossy().eq_ignore_ascii_case(".git")))
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::new(
            "merge.invalid_path",
            "Merge file path must stay within the selected checkout",
        ));
    }
    let root = dunce::canonicalize(cwd)
        .map_err(|e| AppError::new("merge.workspace_path", e.to_string()))?;
    let joined = root.join(relative);
    let mut existing = joined.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            AppError::new("merge.invalid_path", "Merge path has no existing ancestor")
        })?;
    }
    let resolved = dunce::canonicalize(existing)
        .map_err(|e| AppError::new("merge.workspace_path", e.to_string()))?;
    if !resolved.starts_with(&root) {
        return Err(AppError::new(
            "merge.invalid_path",
            "Merge path resolves outside the selected checkout",
        ));
    }
    Ok(joined)
}

fn absent_oid(oid: &str) -> bool {
    oid.is_empty() || oid.bytes().all(|b| b == b'0')
}

fn read_blob(cwd: &str, oid: &str) -> AppResult<Arc<core::Asset>> {
    if absent_oid(oid) {
        return Ok(Arc::new(core::Asset::absent()));
    }
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::new(
            "merge.invalid_oid",
            "Merge snapshot must use a full immutable blob OID",
        ));
    }
    let output = command("git")
        .args(["cat-file", "blob", oid])
        .current_dir(cwd)
        .output()
        .map_err(|e| AppError::new("merge.read_blob", e.to_string()))?;
    if !output.status.success() {
        return Err(AppError::new(
            "merge.read_blob",
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    core::parse_shared(&output.stdout).map_err(core_error)
}

pub(crate) fn build(cwd: &str, path: &str, oids: [&str; 3]) -> AppResult<MergeSemanticSession> {
    let full_path = checked_path(cwd, path)?;
    if matches!(
        Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("fbx" | "png" | "jpg" | "jpeg" | "tga" | "psd" | "exr" | "wav" | "mp3" | "dll")
    ) {
        return Err(AppError::new(
            "merge.opaque_asset",
            "This asset requires an explicit whole-file version choice",
        ));
    }
    let base = read_blob(cwd, oids[0])?;
    let ours = read_blob(cwd, oids[1])?;
    let theirs = read_blob(cwd, oids[2])?;
    if base.documents.is_empty() && ours.documents.is_empty() && theirs.documents.is_empty() {
        return Err(AppError::new(
            "merge.empty_content",
            "All three file snapshots are absent",
        ));
    }
    let engine = Arc::new(core::MergeSession::from_shared(base, ours, theirs).map_err(core_error)?);
    from_core(
        cwd,
        path,
        oids,
        engine,
        super::session::hash_workspace_bytes(std::fs::read(full_path).ok().as_deref()),
    )
}

pub(crate) fn from_core(
    cwd: &str,
    path: &str,
    oids: [&str; 3],
    engine: Arc<core::MergeSession>,
    workspace_hash: u64,
) -> AppResult<MergeSemanticSession> {
    let asset_kind = unity_asset_kind(path);
    let hierarchy = matches!(
        asset_kind,
        crate::diff::types::UnityAssetKind::Scene | crate::diff::types::UnityAssetKind::Prefab
    );
    let layout = if hierarchy {
        SemanticLayout::SceneHierarchyInspector
    } else {
        SemanticLayout::AssetInspector
    };
    let mut summary = MergeSummary::default();
    let mut targets = Vec::new();
    let mut tree = Vec::new();
    let mut conflict_field_ids = HashSet::new();
    let mut grouped: HashMap<String, Vec<&core::Change>> = HashMap::new();
    let mut object_order = Vec::new();
    for change in &engine.catalog().changes {
        let id = if hierarchy {
            object_group_id(&engine, &change.object_id)
        } else {
            change.object_id.clone()
        };
        grouped
            .entry(id.clone())
            .or_insert_with(|| {
                object_order.push(id);
                Vec::new()
            })
            .push(change);
    }
    for object_id in &object_order {
        let changes = &grouped[object_id];
        let conflicts = changes
            .iter()
            .filter(|c| c.status == ChangeStatus::Conflict)
            .count();
        let auto = changes.len() - conflicts;
        conflict_field_ids.extend(
            changes
                .iter()
                .filter(|c| c.status == ChangeStatus::Conflict)
                .map(|c| c.id.clone()),
        );
        let (label, _) = object_label(&engine, object_id);
        let status = if conflicts > 0 {
            DocMergeStatus::HasConflicts
        } else {
            DocMergeStatus::AutoResolved
        };
        targets.push(MergeTargetSummary {
            id: format!("core:{object_id}"),
            label,
            path: object_id.clone(),
            merge_status: status,
            conflict_count: conflicts,
            auto_resolved_count: auto,
        });
        summary.total_targets += 1;
        summary.total_conflicts += conflicts;
        summary.total_auto_resolved += auto;
        if conflicts > 0 {
            summary.conflicting_targets += 1
        } else {
            summary.auto_resolved_targets += 1
        }
    }
    // Include unchanged ancestors solely for navigation. Identity and parentage
    // come from each immutable snapshot's fileIDs, never object-name matching.
    let mut visible = HashSet::new();
    let mut tree_order = Vec::new();
    let mut parents: HashMap<String, String> = HashMap::new();
    for id in &object_order {
        let mut cursor = id.clone();
        let mut chain = HashSet::new();
        loop {
            if !chain.insert(cursor.clone()) {
                break;
            }
            if visible.insert(cursor.clone()) {
                tree_order.push(cursor.clone());
            }
            if !hierarchy {
                break;
            }
            let Some(parent) = object_parent_id(&engine, &cursor) else {
                break;
            };
            if chain.contains(&parent) {
                break;
            }
            parents.insert(cursor, parent.clone());
            cursor = parent;
        }
    }
    let mut visited = HashSet::new();
    for id in &tree_order {
        let mut cursor = id.clone();
        let mut chain = HashSet::new();
        while !visited.contains(&cursor) {
            if !chain.insert(cursor.clone()) {
                parents.remove(&cursor);
                break;
            }
            let Some(parent) = parents.get(&cursor).cloned() else {
                break;
            };
            cursor = parent;
        }
        visited.extend(chain);
    }
    for object_id in tree_order {
        let (label, kind) = object_label(&engine, &object_id);
        let changes = grouped.get(&object_id);
        tree.push(SemanticTreeNode {
            id: format!("core:{object_id}"),
            parent_id: parents.get(&object_id).map(|id| format!("core:{id}")),
            label,
            object_kind: kind,
            change_kind: if changes.is_some() {
                "modified"
            } else {
                "unchanged"
            }
            .into(),
            path: object_id.clone(),
            child_ids: Vec::new(),
            badge_counts: SemanticBadgeCounts {
                modified: changes.map(Vec::len).unwrap_or(0),
                ..Default::default()
            },
            has_inspector: changes.is_some(),
        });
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for node in &tree {
        if let Some(parent) = &node.parent_id {
            children
                .entry(parent.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
    for node in &mut tree {
        node.child_ids = children.remove(&node.id).unwrap_or_default();
    }
    Ok(MergeSemanticSession {
        layout,
        asset_kind,
        summary,
        tree,
        targets,
        inspectors: HashMap::new(),
        target_locators: HashMap::new(),
        conflict_field_ids,
        base_docs: Vec::new(),
        ours_docs: Vec::new(),
        theirs_docs: Vec::new(),
        base_lines: Vec::new(),
        ours_lines: Vec::new(),
        theirs_lines: Vec::new(),
        workspace_hash,
        core_session: Some(engine),
        snapshot_oids: Some(oids.map(str::to_owned)),
        file_path: path.into(),
        workspace_root: cwd.into(),
    })
}

fn preferred_document<'a>(
    engine: &'a core::MergeSession,
    id: &str,
) -> Option<(&'a core::Asset, &'a core::Document)> {
    engine
        .target_asset()
        .document(id)
        .map(|d| (engine.target_asset(), d))
        .or_else(|| {
            engine
                .source_asset()
                .document(id)
                .map(|d| (engine.source_asset(), d))
        })
        .or_else(|| {
            engine
                .base_asset()
                .document(id)
                .map(|d| (engine.base_asset(), d))
        })
}
fn object_body(doc: &core::Document) -> Option<&core::Node> {
    doc.root.entries()?.first().map(|e| &e.value)
}
fn reference_id<'a>(asset: &'a core::Asset, node: Option<&core::Node>) -> Option<&'a str> {
    let node = node?;
    if node
        .get("guid")
        .and_then(|n| n.scalar(asset))
        .is_some_and(|s| s.bytes().any(|b| b != b'0'))
    {
        return None;
    }
    node.get("fileID")
        .and_then(|n| n.scalar(asset))
        .filter(|id| *id != "0" && id.parse::<i64>().is_ok())
}
fn object_group_id(engine: &core::MergeSession, id: &str) -> String {
    if let Some((asset, doc)) = preferred_document(engine, id) {
        if let Some(owner) =
            object_body(doc).and_then(|body| reference_id(asset, body.get("m_GameObject")))
        {
            if preferred_document(engine, owner)
                .is_some_and(|(_, d)| d.class_id.as_deref() == Some("1"))
            {
                return owner.into();
            }
        }
    }
    id.into()
}
fn object_parent_id(engine: &core::MergeSession, id: &str) -> Option<String> {
    let (asset, doc) = preferred_document(engine, id)?;
    let body = object_body(doc)?;
    let parent_transform = if doc.class_id.as_deref() == Some("1") {
        let components = body.get("m_Component")?.items()?;
        let transform = components
            .iter()
            .filter_map(|i| reference_id(asset, i.value.get("component")))
            .find_map(|id| {
                preferred_document(engine, id)
                    .filter(|(_, d)| matches!(d.class_id.as_deref(), Some("4" | "224")))
            })?;
        reference_id(transform.0, object_body(transform.1)?.get("m_Father"))?
    } else if doc.class_id.as_deref() == Some("1001") {
        reference_id(asset, body.get("m_Modification")?.get("m_TransformParent"))?
    } else {
        return None;
    };
    let (parent_asset, parent_doc) = preferred_document(engine, parent_transform)?;
    let owner = reference_id(parent_asset, object_body(parent_doc)?.get("m_GameObject"))?;
    if owner == id || preferred_document(engine, owner).is_none() {
        None
    } else {
        Some(owner.into())
    }
}

fn object_label(engine: &core::MergeSession, object_id: &str) -> (String, String) {
    let document = engine
        .target_asset()
        .document(object_id)
        .map(|d| (engine.target_asset(), d))
        .or_else(|| {
            engine
                .source_asset()
                .document(object_id)
                .map(|d| (engine.source_asset(), d))
        })
        .or_else(|| {
            engine
                .base_asset()
                .document(object_id)
                .map(|d| (engine.base_asset(), d))
        });
    let Some((asset, doc)) = document else {
        return (object_id.into(), "Object".into());
    };
    let root = doc.root.entries().and_then(|e| e.first());
    let kind = root
        .map(|e| e.key.clone())
        .unwrap_or_else(|| "Object".into());
    let name = root
        .and_then(|e| e.value.get("m_Name"))
        .and_then(|n| n.scalar(asset))
        .filter(|s| !s.is_empty());
    (
        name.map(|name| format!("{name} ({object_id})"))
            .unwrap_or_else(|| format!("{kind} ({object_id})")),
        kind,
    )
}

pub(crate) fn materialize(
    session: &MergeSemanticSession,
    target_id: &str,
) -> AppResult<MergeTargetInspector> {
    let engine = session.core_session.as_ref().ok_or_else(|| {
        AppError::new(
            "merge.core_required",
            "Reopen this merge session with the current asset engine",
        )
    })?;
    let target_id_raw = target_id
        .strip_prefix("core:")
        .ok_or_else(|| AppError::new("merge.unknown_target", "Unknown merge object selector"))?;
    let target = session
        .targets
        .iter()
        .find(|t| t.id == target_id)
        .ok_or_else(|| {
            AppError::new(
                "merge.unknown_target",
                "Object is not part of this merge catalog",
            )
        })?;
    let hierarchy = matches!(session.layout, SemanticLayout::SceneHierarchyInspector);
    let mut grouped: HashMap<&str, Vec<&core::Change>> = HashMap::new();
    let mut order = Vec::new();
    for change in &engine.catalog().changes {
        let group = if hierarchy {
            object_group_id(engine, &change.object_id)
        } else {
            change.object_id.clone()
        };
        if group != target_id_raw {
            continue;
        }
        grouped
            .entry(&change.object_id)
            .or_insert_with(|| {
                order.push(change.object_id.as_str());
                Vec::new()
            })
            .push(change);
    }
    order.sort_by_key(|id| if *id == target_id_raw { 0 } else { 1 });
    let panels = order
        .into_iter()
        .map(|id| {
            let (label, kind) = object_label(engine, id);
            let changes = &grouped[id];
            let conflict = changes.iter().any(|c| c.status == ChangeStatus::Conflict);
            let panel_kind = if preferred_document(engine, id)
                .is_some_and(|(_, d)| d.class_id.as_deref() == Some("1"))
            {
                InspectorPanelKind::GameObjectHeader
            } else if hierarchy {
                InspectorPanelKind::Component
            } else {
                InspectorPanelKind::AssetRoot
            };
            MergePanel {
                panel_kind,
                title: label,
                script_class: None,
                component_type: Some(kind),
                component_source: None,
                component_inference: None,
                merge_status: if conflict {
                    DocMergeStatus::HasConflicts
                } else {
                    DocMergeStatus::AutoResolved
                },
                fields: changes.iter().map(|change| change_field(change)).collect(),
            }
        })
        .collect();
    Ok(MergeTargetInspector {
        target_id: target_id.into(),
        title: target.label.clone(),
        path: target.path.clone(),
        panels,
    })
}

fn change_field(change: &core::Change) -> MergeField {
    let raw = |value: &Option<core::ValueSummary>| value.as_ref().map(|v| v.raw.clone());
    let conflict = change.status == ChangeStatus::Conflict;
    let auto_choice = if conflict {
        None
    } else if change.status == ChangeStatus::AlreadyApplied {
        Some(MergeSide::Ours)
    } else {
        Some(MergeSide::Theirs)
    };
    let result = if conflict {
        raw(&change.target)
    } else {
        raw(&change.source)
    };
    MergeField {
        id: change.id.clone(),
        property_path: change.property_path.clone(),
        label: if change.property_path.is_empty() {
            "Object".into()
        } else {
            change.property_path.clone()
        },
        value_type: change
            .source
            .as_ref()
            .or(change.target.as_ref())
            .or(change.base.as_ref())
            .map(|v| v.kind.clone())
            .unwrap_or_else(|| "absent".into()),
        base: raw(&change.base),
        ours: raw(&change.target),
        theirs: raw(&change.source),
        result,
        merge_state: if conflict {
            MergeState::Conflict
        } else {
            MergeState::Auto
        },
        auto_choice,
        manual_choice: None,
        children: Vec::new(),
        field_type: None,
        reference_base: None,
        reference_ours: None,
        reference_theirs: None,
    }
}

pub(crate) fn assemble(
    session: &MergeSemanticSession,
    resolutions: &HashMap<String, FieldResolution>,
) -> AppResult<AssembledMerge> {
    let engine = session.core_session.as_ref().ok_or_else(|| {
        AppError::new(
            "merge.core_required",
            "Reopen this merge session with the current asset engine",
        )
    })?;
    for id in resolutions.keys() {
        if !engine.catalog().changes.iter().any(|c| &c.id == id) {
            return Err(AppError::new(
                "merge.unknown_resolution",
                "Resolution does not belong to this immutable merge catalog",
            ));
        }
    }
    // The human Collab operation explicitly resolves a complete Git conflict.
    // Agent selective integration uses merge_jobs and defaults to keep_target.
    let decisions = engine
        .catalog()
        .changes
        .iter()
        .map(|change| Decision {
            change_id: change.id.clone(),
            resolution: match resolutions.get(&change.id).map(|r| r.side) {
                Some(MergeSide::Base) => Resolution::Base,
                Some(MergeSide::Ours) => Resolution::Target,
                Some(MergeSide::Theirs) => Resolution::Source,
                None => Resolution::Include,
            },
        })
        .collect::<Vec<_>>();
    let output = engine.render(&decisions).map_err(core_error)?;
    if !output.ready {
        return Err(AppError::new(
            "merge.unresolved_conflicts",
            "Resolve the remaining field or reference dependencies before applying",
        )
        .detail(serde_json::to_string(&output.conflicts).unwrap_or_default()));
    }
    if output.bytes.is_empty() {
        Ok(AssembledMerge::DeleteFile)
    } else {
        String::from_utf8(output.bytes)
            .map(AssembledMerge::ResolvedText)
            .map_err(|e| AppError::new("merge.invalid_utf8", e.to_string()))
    }
}

/// Verify the file still has the same three unmerged index stages. OIDs in the
/// session are content identities, not a promise that the index stayed fixed.
pub(crate) fn verify_snapshot(
    session: &MergeSemanticSession,
    cwd: &str,
    path: &str,
) -> AppResult<()> {
    checked_path(cwd, path)?;
    if session.workspace_root != cwd || session.file_path != path {
        return Err(AppError::new(
            "merge.scope_mismatch",
            "Merge session belongs to another checkout or file",
        ));
    }
    let Some(expected) = &session.snapshot_oids else {
        return Err(AppError::new(
            "merge.core_required",
            "Reopen the merge session",
        ));
    };
    let output = command("git")
        .args(["ls-files", "--unmerged", "-z", "--", path])
        .current_dir(cwd)
        .output()
        .map_err(|e| AppError::new("merge.index_read", e.to_string()))?;
    if !output.status.success() {
        return Err(AppError::new(
            "merge.index_read",
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    let mut actual = [String::new(), String::new(), String::new()];
    for record in output.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let Some(tab) = record.iter().position(|b| *b == b'\t') else {
            continue;
        };
        let header = std::str::from_utf8(&record[..tab])
            .map_err(|e| AppError::new("merge.index_read", e.to_string()))?;
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() == 3 {
            if let Ok(stage) = fields[2].parse::<usize>() {
                if (1..=3).contains(&stage) {
                    actual[stage - 1] = fields[1].into();
                }
            }
        }
    }
    if (0..3).any(|i| {
        if absent_oid(&expected[i]) {
            !actual[i].is_empty()
        } else {
            expected[i] != actual[i]
        }
    }) {
        return Err(AppError::new(
            "merge.snapshot_stale",
            "Git conflict stages changed; reopen the merge session",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn yaml(body: &str) -> Vec<u8> {
        format!("%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n{body}").into_bytes()
    }
    #[test]
    fn adapter_exposes_hidden_fields_and_uses_the_core_conflict_identity() {
        let base = yaml("  m_ObjectHideFlags: 0\n  nested: {x: 0, y: 0}\n");
        let target = yaml("  m_ObjectHideFlags: 1\n  nested: {x: 3, y: 0}\n");
        let source = yaml("  m_ObjectHideFlags: 2\n  nested: {x: 0, y: 4}\n");
        let engine = Arc::new(core::prepare_merge(&base, &target, &source).unwrap());
        let session = from_core("test", "Assets/Test.asset", ["", "", ""], engine, 0).unwrap();
        assert_eq!(session.summary.total_conflicts, 1);
        assert_eq!(session.summary.total_auto_resolved, 1);
        let inspector = materialize(&session, "core:11400000").unwrap();
        assert!(inspector.panels[0]
            .fields
            .iter()
            .any(|f| f.property_path.ends_with("m_ObjectHideFlags")));
        assert!(assemble(&session, &HashMap::new()).is_err());
        let id = session.conflict_field_ids.iter().next().unwrap().clone();
        let resolutions = HashMap::from([(
            id,
            FieldResolution {
                side: MergeSide::Ours,
            },
        )]);
        let AssembledMerge::ResolvedText(result) = assemble(&session, &resolutions).unwrap() else {
            panic!("expected text")
        };
        assert!(result.contains("m_ObjectHideFlags: 1"));
        assert!(result.contains("nested: {x: 3, y: 4}"));
        assert!(inspector.panels[0]
            .fields
            .iter()
            .all(|f| f.reference_base.is_none()
                && f.reference_ours.is_none()
                && f.reference_theirs.is_none()));
    }

    #[test]
    fn reads_requested_blob_oids_without_reading_mutable_index_stages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_str().unwrap();
        assert!(command("git")
            .args(["init", "--quiet"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        let store = |name: &str, bytes: &[u8]| {
            std::fs::write(temp.path().join(name), bytes).unwrap();
            let output = command("git")
                .args(["hash-object", "-w", "--", name])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        let base = store("base.yml", &yaml("  number: 1\n"));
        let left = store("left.yml", &yaml("  number: 2\n"));
        let right = store("right.yml", &yaml("  number: 3\n"));
        std::fs::create_dir(temp.path().join("Assets")).unwrap();
        std::fs::write(
            temp.path().join("Assets/Test.asset"),
            yaml("  unrelatedWorkspace: 9\n"),
        )
        .unwrap();
        let session = build(root, "Assets/Test.asset", [&base, &left, &right]).unwrap();
        assert_eq!(session.summary.total_conflicts, 1);
        let inspector = materialize(&session, "core:11400000").unwrap();
        let field = &inspector.panels[0].fields[0];
        assert_eq!(field.base.as_deref(), Some("1"));
        assert_eq!(field.ours.as_deref(), Some("2"));
        assert_eq!(field.theirs.as_deref(), Some("3"));
        assert!(verify_snapshot(&session, root, "Assets/Test.asset").is_err());
        assert!(checked_path(root, "../elsewhere.asset").is_err());
        assert!(read_blob(root, "HEAD").is_err());
    }

    #[test]
    fn scene_navigation_groups_components_by_file_id_and_keeps_unchanged_ancestors() {
        let base=b"--- !u!1 &1\nGameObject:\n  m_Name: Parent\n  m_Component:\n  - component: {fileID: 2}\n--- !u!4 &2\nTransform:\n  m_GameObject: {fileID: 1}\n  m_Father: {fileID: 0}\n  m_Children:\n  - {fileID: 4}\n--- !u!1 &3\nGameObject:\n  m_Name: Child\n  m_Component:\n  - component: {fileID: 4}\n--- !u!4 &4\nTransform:\n  m_GameObject: {fileID: 3}\n  m_Father: {fileID: 2}\n  m_Children: []\n  m_LocalPosition: {x: 0, y: 0, z: 0}\n";
        let target = String::from_utf8(base.to_vec())
            .unwrap()
            .replace("x: 0", "x: 3");
        let source = String::from_utf8(base.to_vec())
            .unwrap()
            .replace("y: 0", "y: 4");
        let engine =
            Arc::new(core::prepare_merge(base, target.as_bytes(), source.as_bytes()).unwrap());
        let session = from_core("test", "Assets/Test.prefab", ["", "", ""], engine, 0).unwrap();
        assert!(matches!(
            session.layout,
            SemanticLayout::SceneHierarchyInspector
        ));
        assert_eq!(session.targets[0].id, "core:3");
        assert_eq!(
            session
                .tree
                .iter()
                .find(|n| n.id == "core:3")
                .unwrap()
                .parent_id
                .as_deref(),
            Some("core:1")
        );
        assert!(
            !session
                .tree
                .iter()
                .find(|n| n.id == "core:1")
                .unwrap()
                .has_inspector
        );
        let inspector = materialize(&session, "core:3").unwrap();
        assert_eq!(inspector.panels.len(), 1);
        assert!(matches!(
            inspector.panels[0].panel_kind,
            InspectorPanelKind::Component
        ));
        let AssembledMerge::ResolvedText(result) = assemble(&session, &HashMap::new()).unwrap()
        else {
            panic!("expected text")
        };
        assert!(result.contains("{x: 3, y: 4, z: 0}"));
    }
}
