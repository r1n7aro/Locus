use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::models::{
    ProjectExplorerItemRef, ProjectExplorerItemState, ProjectExplorerMutationResult,
    ProjectExplorerNode, ProjectExplorerOperation, ProjectExplorerPresetSummary,
    ProjectExplorerSnapshot,
};

const WORKSPACE_TREE_SCHEMA_VERSION: u32 = 3;
const PREVIOUS_WORKSPACE_TREE_SCHEMA_VERSION: u32 = 2;
const LEGACY_WORKSPACE_TREE_SCHEMA_VERSION: u32 = 1;
const WORKSPACE_TREE_DIR: &str = "workspace-trees";
const WORKSPACE_TREE_INDEX: &str = "index.json";
const DEFAULT_PRESET_ID: &str = "default";
const DEFAULT_PRESET_NAME: &str = "Default";
const WORKSPACE_TREE_MIGRATION_ID: &str = "workspace-tree-migration-v3";
const DEFAULT_HIDDEN_SYSTEM_RESOURCE_ID: &str = "archived";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTreeIndex {
    schema_version: u32,
    active_preset_id: String,
    preset_order: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTreePresetFile {
    schema_version: u32,
    preset_id: String,
    name: String,
    project_id: String,
    revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_operation_id: Option<String>,
    #[serde(default)]
    nodes: Vec<ProjectExplorerNode>,
    #[serde(default)]
    item_states: Vec<ProjectExplorerItemState>,
}

fn layout_locks() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn layout_lock(root: &Path) -> Result<Arc<Mutex<()>>, String> {
    let mut locks = layout_locks()
        .lock()
        .map_err(|error| format!("Workspace tree lock registry is unavailable: {error}"))?;
    Ok(Arc::clone(
        locks
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    ))
}

fn tree_dir(root: &Path) -> PathBuf {
    root.join("Locus").join(WORKSPACE_TREE_DIR)
}

fn index_path(root: &Path) -> PathBuf {
    tree_dir(root).join(WORKSPACE_TREE_INDEX)
}

fn preset_path(root: &Path, preset_id: &str) -> PathBuf {
    tree_dir(root).join(format!("{preset_id}.json"))
}

fn validate_preset_id(preset_id: &str) -> Result<&str, String> {
    let preset_id = preset_id.trim();
    if preset_id.is_empty()
        || !preset_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Workspace tree preset id contains unsupported characters".to_string());
    }
    Ok(preset_id)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Failed to serialize workspace tree: {error}"))?;
    crate::config::atomic_write_config(path, &bytes)
}

fn default_preset(project_id: &str) -> WorkspaceTreePresetFile {
    WorkspaceTreePresetFile {
        schema_version: WORKSPACE_TREE_SCHEMA_VERSION,
        preset_id: DEFAULT_PRESET_ID.to_string(),
        name: DEFAULT_PRESET_NAME.to_string(),
        project_id: project_id.to_string(),
        revision: 0,
        last_operation_id: None,
        nodes: Vec::new(),
        item_states: Vec::new(),
    }
}

fn ensure_layout(root: &Path, project_id: &str) -> Result<WorkspaceTreeIndex, String> {
    let directory = tree_dir(root);
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Failed to create workspace tree directory '{}': {error}",
            directory.display()
        )
    })?;
    let path = index_path(root);
    if path.is_file() {
        return read_index(root);
    }
    let preset = default_preset(project_id);
    write_json(&preset_path(root, DEFAULT_PRESET_ID), &preset)?;
    let index = WorkspaceTreeIndex {
        schema_version: WORKSPACE_TREE_SCHEMA_VERSION,
        active_preset_id: DEFAULT_PRESET_ID.to_string(),
        preset_order: vec![DEFAULT_PRESET_ID.to_string()],
    };
    write_json(&path, &index)?;
    Ok(index)
}

fn read_index(root: &Path) -> Result<WorkspaceTreeIndex, String> {
    let path = index_path(root);
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "Failed to read workspace tree index '{}': {error}",
            path.display()
        )
    })?;
    let mut index = serde_json::from_str::<WorkspaceTreeIndex>(&raw).map_err(|error| {
        format!(
            "Failed to parse workspace tree index '{}': {error}",
            path.display()
        )
    })?;
    match index.schema_version {
        WORKSPACE_TREE_SCHEMA_VERSION => {}
        LEGACY_WORKSPACE_TREE_SCHEMA_VERSION | PREVIOUS_WORKSPACE_TREE_SCHEMA_VERSION => {
            index.schema_version = WORKSPACE_TREE_SCHEMA_VERSION;
            write_json(&path, &index)?;
        }
        version => {
            return Err(format!(
                "Unsupported workspace tree index schema version: {version}"
            ));
        }
    }
    validate_preset_id(&index.active_preset_id)?;
    Ok(index)
}

fn migrate_preset(preset: &mut WorkspaceTreePresetFile) -> Result<bool, String> {
    match preset.schema_version {
        WORKSPACE_TREE_SCHEMA_VERSION => return Ok(false),
        LEGACY_WORKSPACE_TREE_SCHEMA_VERSION | PREVIOUS_WORKSPACE_TREE_SCHEMA_VERSION => {}
        version => {
            return Err(format!(
                "Unsupported workspace tree preset schema version: {version}"
            ));
        }
    }

    // Knowledge removal is represented by the absence of a placement. Sessions
    // use their archive state. Visibility remains a property of Locus system nodes.
    if preset.schema_version == LEGACY_WORKSPACE_TREE_SCHEMA_VERSION {
        preset
            .nodes
            .retain(|node| !(node.hidden && node.resource_kind.as_deref() == Some("knowledge")));
        for node in &mut preset.nodes {
            if node.resource_kind.as_deref() != Some("system") {
                node.hidden = false;
            }
        }
        let parents = preset
            .nodes
            .iter()
            .map(|node| node.parent_node_id.clone())
            .collect::<HashSet<_>>();
        for parent in parents {
            normalize_siblings(&mut preset.nodes, parent.as_deref());
        }
    }
    // v1/v2 presets have no item decoration state. Persist the explicit empty
    // collection once; conversation records and their export schema are untouched.
    preset.item_states.clear();
    preset.schema_version = WORKSPACE_TREE_SCHEMA_VERSION;
    preset.revision = preset
        .revision
        .checked_add(1)
        .ok_or_else(|| "Workspace tree revision is exhausted".to_string())?;
    preset.last_operation_id = Some(WORKSPACE_TREE_MIGRATION_ID.to_string());
    Ok(true)
}

fn read_preset(
    root: &Path,
    project_id: &str,
    preset_id: &str,
) -> Result<WorkspaceTreePresetFile, String> {
    let preset_id = validate_preset_id(preset_id)?;
    let path = preset_path(root, preset_id);
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "Failed to read workspace tree preset '{}': {error}",
            path.display()
        )
    })?;
    let mut preset = serde_json::from_str::<WorkspaceTreePresetFile>(&raw).map_err(|error| {
        format!(
            "Failed to parse workspace tree preset '{}': {error}",
            path.display()
        )
    })?;
    let migrated = migrate_preset(&mut preset)?;
    if preset.preset_id != preset_id {
        return Err(format!(
            "Workspace tree preset id '{}' does not match its file name '{}'",
            preset.preset_id, preset_id
        ));
    }
    if preset.project_id.trim().is_empty() {
        preset.project_id = project_id.to_string();
    }
    if preset.project_id != project_id {
        return Err(format!(
            "Workspace tree preset belongs to project '{}', expected '{}'",
            preset.project_id, project_id
        ));
    }
    validate_nodes(project_id, &preset.nodes)?;
    if migrated {
        write_json(&path, &preset)?;
    }
    Ok(preset)
}

fn preset_summary(
    root: &Path,
    preset: &WorkspaceTreePresetFile,
    active_preset_id: &str,
) -> ProjectExplorerPresetSummary {
    ProjectExplorerPresetSummary {
        preset_id: preset.preset_id.clone(),
        name: preset.name.clone(),
        revision: preset.revision,
        active: preset.preset_id == active_preset_id,
        file_path: preset_path(root, &preset.preset_id)
            .to_string_lossy()
            .into_owned(),
    }
}

fn load_presets(
    root: &Path,
    project_id: &str,
    index: &WorkspaceTreeIndex,
) -> Result<Vec<WorkspaceTreePresetFile>, String> {
    let mut ids = index.preset_order.clone();
    let directory = tree_dir(root);
    for entry in std::fs::read_dir(&directory).map_err(|error| {
        format!(
            "Failed to list workspace tree presets '{}': {error}",
            directory.display()
        )
    })? {
        let entry =
            entry.map_err(|error| format!("Failed to inspect workspace tree preset: {error}"))?;
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(WORKSPACE_TREE_INDEX)
            || path.extension().and_then(|extension| extension.to_str()) != Some("json")
        {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if validate_preset_id(id).is_ok() && !ids.iter().any(|candidate| candidate == id) {
            ids.push(id.to_string());
        }
    }
    ids.into_iter()
        .filter(|id| preset_path(root, id).is_file())
        .map(|id| read_preset(root, project_id, &id))
        .collect()
}

fn snapshot_from_parts(
    root: &Path,
    project_id: &str,
    index: &WorkspaceTreeIndex,
    preset: WorkspaceTreePresetFile,
    presets: &[WorkspaceTreePresetFile],
) -> ProjectExplorerSnapshot {
    ProjectExplorerSnapshot {
        project_id: project_id.to_string(),
        preset_id: preset.preset_id.clone(),
        preset_name: preset.name.clone(),
        manifest_path: preset_path(root, &preset.preset_id)
            .to_string_lossy()
            .into_owned(),
        revision: preset.revision,
        nodes: preset.nodes,
        item_states: preset.item_states,
        presets: presets
            .iter()
            .map(|candidate| preset_summary(root, candidate, &index.active_preset_id))
            .collect(),
    }
}

fn active_snapshot_unlocked(
    root: &Path,
    project_id: &str,
) -> Result<ProjectExplorerSnapshot, String> {
    let index = ensure_layout(root, project_id)?;
    let presets = load_presets(root, project_id, &index)?;
    let preset = presets
        .iter()
        .find(|preset| preset.preset_id == index.active_preset_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Active workspace tree preset '{}' is unavailable",
                index.active_preset_id
            )
        })?;
    Ok(snapshot_from_parts(
        root, project_id, &index, preset, &presets,
    ))
}

pub fn snapshot(root: &Path, project_id: &str) -> Result<ProjectExplorerSnapshot, String> {
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    active_snapshot_unlocked(root, project_id)
}

pub fn list_presets(
    root: &Path,
    project_id: &str,
) -> Result<Vec<ProjectExplorerPresetSummary>, String> {
    Ok(snapshot(root, project_id)?.presets)
}

/// Preserve placements and per-entry pins in every preset after a file operation.
/// No schema change: only existing paths and names are updated.
pub fn relocate_file_references(
    root: &Path,
    project_id: &str,
    source: &Path,
    target: Option<&Path>,
) -> Result<ProjectExplorerSnapshot, String> {
    fn key(path: &Path) -> String {
        let value = path.to_string_lossy().replace('\\', "/");
        if cfg!(windows) { value.to_lowercase() } else { value }
    }
    let lock = layout_lock(root)?;
    let _guard = lock.lock().map_err(|error| error.to_string())?;
    let index = ensure_layout(root, project_id)?;
    let originals = load_presets(root, project_id, &index)?;
    let source_key = key(source);
    let mut updates = Vec::new();
    for original in &originals {
        let mut preset = original.clone();
        let mut removed = HashSet::new();
        let mut changed = false;
        for node in &mut preset.nodes {
            let Some(path) = node.source_path.clone() else { continue };
            if key(Path::new(&path)) == source_key {
                changed = true;
                if let Some(target) = target {
                    node.source_path = Some(target.to_string_lossy().into_owned());
                    node.folder_name = target.file_name().map(|name| name.to_string_lossy().into_owned());
                } else {
                    removed.insert(node.node_id.clone());
                }
            } else {
                for state in preset.item_states.iter_mut().filter(|state| state.node_id == node.node_id) {
                    let Some(relative) = &state.relative_path else { continue };
                    if key(&Path::new(&path).join(relative)) != source_key { continue; }
                    changed = true;
                    if let Some(target) = target {
                        // Rename stays in the same directory, so the mount root is unchanged.
                        state.relative_path = Some(Path::new(relative).with_file_name(target.file_name().unwrap())
                            .to_string_lossy().replace('\\', "/"));
                    } else {
                        state.pinned = false;
                        state.highlighted = false;
                    }
                }
            }
        }
        if !changed { continue; }
        preset.nodes.retain(|node| !removed.contains(&node.node_id));
        preset.item_states.retain(|state| !removed.contains(&state.node_id) && (state.pinned || state.highlighted));
        preset.revision = preset.revision.checked_add(1).ok_or("Workspace tree revision is exhausted")?;
        preset.last_operation_id = None;
        updates.push(preset);
    }
    for preset in &updates {
        if let Err(error) = write_json(&preset_path(root, &preset.preset_id), preset) {
            let mut rollback_errors = Vec::new();
            for original in &originals {
                if updates.iter().any(|item| item.preset_id == original.preset_id) {
                    if let Err(error) = write_json(&preset_path(root, &original.preset_id), original) {
                        rollback_errors.push(error);
                    }
                }
            }
            return Err(format!("{error}; rollback: {}", rollback_errors.join("; ")));
        }
    }
    active_snapshot_unlocked(root, project_id)
}

fn validate_nodes(project_id: &str, nodes: &[ProjectExplorerNode]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for node in nodes {
        if node.node_id.trim().is_empty() || !ids.insert(node.node_id.as_str()) {
            return Err(format!(
                "Workspace tree contains an empty or duplicate node id: '{}'",
                node.node_id
            ));
        }
        if node.project_id != project_id {
            return Err(format!(
                "Workspace tree node '{}' belongs to another project",
                node.node_id
            ));
        }
        if !matches!(node.node_kind.as_str(), "folder" | "resource") {
            return Err(format!(
                "Workspace tree node '{}' has unsupported kind '{}'",
                node.node_id, node.node_kind
            ));
        }
        if node.hidden && node.resource_kind.as_deref() != Some("system") {
            return Err(format!(
                "Only Locus system resources can be hidden: '{}'",
                node.node_id
            ));
        }
    }
    let by_id = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    for node in nodes {
        if let Some(parent_id) = node.parent_node_id.as_deref() {
            let parent = by_id.get(parent_id).ok_or_else(|| {
                format!(
                    "Workspace tree node '{}' references missing parent '{}'",
                    node.node_id, parent_id
                )
            })?;
            if !parent_accepts_node(parent, node) {
                return Err(format!(
                    "Workspace tree node '{}' references an incompatible parent",
                    node.node_id
                ));
            }
        }
        let mut cursor = node.parent_node_id.as_deref();
        let mut visited = HashSet::new();
        while let Some(parent_id) = cursor {
            if parent_id == node.node_id || !visited.insert(parent_id) {
                return Err(format!(
                    "Workspace tree contains a cycle at '{}'",
                    node.node_id
                ));
            }
            cursor = by_id
                .get(parent_id)
                .and_then(|parent| parent.parent_node_id.as_deref());
        }
    }
    Ok(())
}

fn parent_accepts_node(parent: &ProjectExplorerNode, child: &ProjectExplorerNode) -> bool {
    parent.node_kind == "folder"
        || (parent.node_kind == "resource"
            && parent.resource_kind.as_deref() == Some("session")
            && child.node_kind == "resource"
            && child.resource_kind.as_deref() == Some("session"))
}

fn validate_parent_for_node(
    nodes: &[ProjectExplorerNode],
    parent_node_id: Option<&str>,
    node_kind: &str,
    resource_kind: Option<&str>,
    allow_session_parent: bool,
) -> Result<(), String> {
    let Some(parent_node_id) = parent_node_id else {
        return Ok(());
    };
    let parent = nodes
        .iter()
        .find(|node| node.node_id == parent_node_id)
        .ok_or_else(|| format!("Workspace tree parent does not exist: {parent_node_id}"))?;
    let candidate = ProjectExplorerNode {
        node_id: String::new(),
        project_id: parent.project_id.clone(),
        node_kind: node_kind.to_string(),
        parent_node_id: None,
        resource_kind: resource_kind.map(str::to_string),
        resource_id: None,
        folder_name: None,
        hidden: false,
        source_path: None,
        source_kind: None,
        position: 0,
    };
    if parent.node_kind != "folder"
        && !(allow_session_parent && parent_accepts_node(parent, &candidate))
    {
        return Err(if allow_session_parent {
            "Workspace tree parent must be a folder, or a session for session children".to_string()
        } else {
            "Workspace tree parent must be a folder".to_string()
        });
    }
    Ok(())
}

fn normalize_siblings(nodes: &mut [ProjectExplorerNode], parent_node_id: Option<&str>) {
    let mut indexes = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent_node_id.as_deref() == parent_node_id)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indexes.sort_by(|left, right| {
        nodes[*left]
            .position
            .cmp(&nodes[*right].position)
            .then_with(|| nodes[*left].node_id.cmp(&nodes[*right].node_id))
    });
    for (position, index) in indexes.into_iter().enumerate() {
        nodes[index].position = position as i64;
    }
}

fn move_node(
    nodes: &mut Vec<ProjectExplorerNode>,
    node_id: &str,
    parent_node_id: Option<&str>,
    position: i64,
) -> Result<(), String> {
    move_node_with_policy(nodes, node_id, parent_node_id, position, false)
}

fn move_node_with_policy(
    nodes: &mut Vec<ProjectExplorerNode>,
    node_id: &str,
    parent_node_id: Option<&str>,
    position: i64,
    allow_session_parent: bool,
) -> Result<(), String> {
    let index = nodes
        .iter()
        .position(|node| node.node_id == node_id)
        .ok_or_else(|| format!("Workspace tree node does not exist: {node_id}"))?;
    validate_parent_for_node(
        nodes,
        parent_node_id,
        &nodes[index].node_kind,
        nodes[index].resource_kind.as_deref(),
        allow_session_parent,
    )?;
    if parent_node_id == Some(node_id) {
        return Err("Workspace tree node cannot contain itself".to_string());
    }
    let by_id = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut cursor = parent_node_id;
    while let Some(parent_id) = cursor {
        if parent_id == node_id {
            return Err("Workspace tree node cannot move into its descendant".to_string());
        }
        cursor = by_id
            .get(parent_id)
            .and_then(|parent| parent.parent_node_id.as_deref());
    }
    let old_parent = nodes[index].parent_node_id.clone();
    nodes[index].parent_node_id = parent_node_id.map(str::to_string);
    nodes[index].position = i64::MAX / 2;
    normalize_siblings(nodes, old_parent.as_deref());
    let mut siblings = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.node_id != node_id && node.parent_node_id.as_deref() == parent_node_id
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    siblings.sort_by_key(|sibling| nodes[*sibling].position);
    let insertion = position.clamp(0, siblings.len() as i64) as usize;
    siblings.insert(insertion, index);
    for (next_position, sibling) in siblings.into_iter().enumerate() {
        nodes[sibling].position = next_position as i64;
    }
    Ok(())
}

fn delete_folder(nodes: &mut Vec<ProjectExplorerNode>, node_id: &str) -> Result<(), String> {
    let index = nodes
        .iter()
        .position(|node| node.node_id == node_id && node.node_kind == "folder")
        .ok_or_else(|| format!("Workspace tree folder does not exist: {node_id}"))?;
    let parent = nodes[index].parent_node_id.clone();
    let insertion = nodes
        .iter()
        .filter(|node| node.parent_node_id == parent)
        .filter(|node| node.position < nodes[index].position)
        .count();
    let mut children = nodes
        .iter_mut()
        .filter(|node| node.parent_node_id.as_deref() == Some(node_id))
        .collect::<Vec<_>>();
    children.sort_by_key(|node| node.position);
    for (offset, child) in children.into_iter().enumerate() {
        child.parent_node_id = parent.clone();
        child.position = (insertion + offset) as i64;
    }
    nodes.remove(index);
    normalize_siblings(nodes, parent.as_deref());
    Ok(())
}

fn normalized_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn mount_path_node(
    project_id: &str,
    nodes: &mut Vec<ProjectExplorerNode>,
    node_id: Option<&str>,
    parent_node_id: Option<&str>,
    path: &str,
    source_kind: Option<&str>,
    name: Option<&str>,
    position: i64,
) -> Result<(), String> {
    validate_parent_for_node(nodes, parent_node_id, "folder", None, false)?;
    let source = dunce::canonicalize(Path::new(path.trim())).map_err(|error| {
        format!(
            "Workspace tree mount path is unavailable '{}': {error}",
            path.trim()
        )
    })?;
    let metadata = std::fs::metadata(&source).map_err(|error| {
        format!(
            "Failed to inspect workspace tree mount '{}': {error}",
            source.display()
        )
    })?;
    let source_text = source.to_string_lossy().into_owned();
    let source_key = normalized_path_key(&source);
    let existing = nodes.iter().position(|node| {
        node.source_path
            .as_deref()
            .map(Path::new)
            .map(normalized_path_key)
            .as_deref()
            == Some(source_key.as_str())
    });
    let node_id = existing
        .map(|index| nodes[index].node_id.clone())
        .or_else(|| {
            node_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("mount:{}", Uuid::new_v4()));
    let display_name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| source_text.clone());
    let kind = source_kind
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("local")
        .to_string();
    if let Some(index) = existing {
        nodes[index].source_kind = Some(kind);
        nodes[index].folder_name = Some(display_name);
    } else {
        let resource_kind = if metadata.is_dir() {
            "local_directory"
        } else {
            "local_file"
        };
        nodes.push(ProjectExplorerNode {
            node_id: node_id.clone(),
            project_id: project_id.to_string(),
            node_kind: if metadata.is_dir() {
                "folder"
            } else {
                "resource"
            }
            .to_string(),
            parent_node_id: parent_node_id.map(str::to_string),
            resource_kind: Some(resource_kind.to_string()),
            resource_id: Some(format!(
                "path:{}",
                blake3::hash(source_key.as_bytes()).to_hex()
            )),
            folder_name: Some(display_name),
            hidden: false,
            source_path: Some(source_text),
            source_kind: Some(kind),
            position: i64::MAX / 2,
        });
    }
    move_node(nodes, &node_id, parent_node_id, position)
}

fn apply_operation(
    project_id: &str,
    nodes: &mut Vec<ProjectExplorerNode>,
    operation: &ProjectExplorerOperation,
) -> Result<(), String> {
    match operation {
        ProjectExplorerOperation::SetItemState { .. }
        | ProjectExplorerOperation::MovePinnedItems { .. } => {
            Err("Item state must be applied to its workspace tree preset".to_string())
        }
        ProjectExplorerOperation::CreateFolder {
            node_id,
            parent_node_id,
            name,
            position,
        } => {
            validate_parent_for_node(nodes, parent_node_id.as_deref(), "folder", None, false)?;
            let name = name.trim();
            if name.is_empty() {
                return Err("Workspace tree folder name cannot be empty".to_string());
            }
            let node_id = node_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("folder:{}", Uuid::new_v4()));
            if nodes.iter().any(|node| node.node_id == node_id) {
                return Err(format!("Workspace tree node already exists: {node_id}"));
            }
            nodes.push(ProjectExplorerNode {
                node_id: node_id.clone(),
                project_id: project_id.to_string(),
                node_kind: "folder".to_string(),
                parent_node_id: parent_node_id.clone(),
                resource_kind: None,
                resource_id: None,
                folder_name: Some(name.to_string()),
                hidden: false,
                source_path: None,
                source_kind: None,
                position: i64::MAX / 2,
            });
            move_node(nodes, &node_id, parent_node_id.as_deref(), *position)
        }
        ProjectExplorerOperation::RenameFolder { node_id, name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err("Workspace tree folder name cannot be empty".to_string());
            }
            let folder = nodes
                .iter_mut()
                .find(|node| node.node_id == *node_id && node.node_kind == "folder")
                .ok_or_else(|| format!("Workspace tree folder does not exist: {node_id}"))?;
            folder.folder_name = Some(name.to_string());
            Ok(())
        }
        ProjectExplorerOperation::DeleteFolder { node_id } => delete_folder(nodes, node_id),
        ProjectExplorerOperation::MoveNode {
            node_id,
            parent_node_id,
            position,
        } => move_node(nodes, node_id, parent_node_id.as_deref(), *position),
        ProjectExplorerOperation::PlaceResource {
            node_id,
            resource_kind,
            resource_id,
            source_kind,
            parent_node_id,
            position,
        } => {
            if !matches!(
                resource_kind.as_str(),
                "session" | "knowledge" | "system" | "view"
            ) {
                return Err(format!(
                    "Unsupported workspace tree resource kind: {resource_kind}"
                ));
            }
            validate_parent_for_node(
                nodes,
                parent_node_id.as_deref(),
                "resource",
                Some(resource_kind),
                true,
            )?;
            let existing_node_id = nodes
                .iter()
                .find(|node| {
                    node.resource_kind.as_deref() == Some(resource_kind)
                        && node.resource_id.as_deref() == Some(resource_id)
                })
                .map(|node| node.node_id.clone());
            let restores_existing = existing_node_id.is_some();
            let node_id = existing_node_id
                .or_else(|| {
                    node_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("resource:{resource_kind}:{}", Uuid::new_v4()));
            if let Some(existing) = nodes.iter().find(|node| node.node_id == node_id) {
                if existing.resource_kind.as_deref() != Some(resource_kind)
                    || existing.resource_id.as_deref() != Some(resource_id)
                {
                    return Err(format!("Workspace tree node already exists: {node_id}"));
                }
            } else {
                nodes.push(ProjectExplorerNode {
                    node_id: node_id.clone(),
                    project_id: project_id.to_string(),
                    node_kind: "resource".to_string(),
                    parent_node_id: parent_node_id.clone(),
                    resource_kind: Some(resource_kind.clone()),
                    resource_id: Some(resource_id.clone()),
                    folder_name: None,
                    hidden: resource_kind == "system"
                        && resource_id == DEFAULT_HIDDEN_SYSTEM_RESOURCE_ID,
                    source_path: None,
                    source_kind: source_kind.clone(),
                    position: i64::MAX / 2,
                });
            }
            if restores_existing {
                let node = nodes
                    .iter_mut()
                    .find(|node| node.node_id == node_id)
                    .expect("existing workspace tree resource disappeared");
                if source_kind.is_some() {
                    node.source_kind = source_kind.clone();
                }
                // Explicit placement restores a system resource that was hidden earlier.
                node.hidden = false;
            }
            move_node_with_policy(nodes, &node_id, parent_node_id.as_deref(), *position, true)
        }
        ProjectExplorerOperation::RemoveResourcePlacement {
            resource_kind,
            resource_id,
        } => {
            if resource_kind != "knowledge" {
                return Err(
                    "Only knowledge documents can be removed from the workspace".to_string()
                );
            }
            nodes.retain(|node| {
                node.resource_kind.as_deref() != Some(resource_kind)
                    || node.resource_id.as_deref() != Some(resource_id)
            });
            Ok(())
        }
        ProjectExplorerOperation::MountPath {
            node_id,
            parent_node_id,
            path,
            source_kind,
            name,
            position,
        } => mount_path_node(
            project_id,
            nodes,
            node_id.as_deref(),
            parent_node_id.as_deref(),
            path,
            source_kind.as_deref(),
            name.as_deref(),
            *position,
        ),
        ProjectExplorerOperation::SetNodeHidden { node_id, hidden } => {
            let node_index = nodes
                .iter()
                .position(|node| node.node_id == *node_id)
                .ok_or_else(|| format!("Workspace tree node does not exist: {node_id}"))?;
            if nodes[node_index].resource_kind.as_deref() != Some("system") {
                return Err(format!(
                    "Only Locus system resources can be hidden: '{node_id}'"
                ));
            }
            if !*hidden {
                move_node(nodes, node_id, None, 0)?;
            }
            nodes[node_index].hidden = *hidden;
            Ok(())
        }
        ProjectExplorerOperation::RemoveNode { node_id } => {
            let is_folder = nodes
                .iter()
                .find(|node| node.node_id == *node_id)
                .map(|node| node.node_kind == "folder")
                .ok_or_else(|| format!("Workspace tree node does not exist: {node_id}"))?;
            if is_folder {
                delete_folder(nodes, node_id)
            } else {
                nodes.retain(|node| node.node_id != *node_id);
                Ok(())
            }
        }
    }
}

fn set_item_state(
    preset: &mut WorkspaceTreePresetFile,
    node_id: &str,
    relative_path: Option<&str>,
    pinned: Option<bool>,
    highlighted: Option<bool>,
) -> Result<(), String> {
    let node = preset
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .ok_or_else(|| "Workspace tree node is unavailable".to_string())?;
    if node.resource_kind.as_deref() == Some("system") {
        return Err("System entries cannot be pinned or highlighted".to_string());
    }
    let relative_path = relative_path
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    if let Some(path) = &relative_path {
        if node.node_kind != "folder"
            || node.source_path.is_none()
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || path.contains(['\\', ':'])
        {
            return Err("Item path must be relative to a mounted folder".to_string());
        }
    }
    let index = preset
        .item_states
        .iter()
        .position(|state| state.node_id == node_id && state.relative_path == relative_path);
    let mut state = index
        .map(|index| preset.item_states[index].clone())
        .unwrap_or(ProjectExplorerItemState {
            node_id: node_id.to_string(),
            relative_path,
            pinned: false,
            highlighted: false,
        });
    if let Some(value) = pinned {
        state.pinned = value;
    }
    if let Some(value) = highlighted {
        state.highlighted = value;
    }
    if let Some(index) = index {
        preset.item_states.remove(index);
    }
    if state.pinned || state.highlighted {
        preset
            .item_states
            .insert(index.unwrap_or(preset.item_states.len()), state);
    }
    Ok(())
}

fn move_pinned_items(
    states: &mut Vec<ProjectExplorerItemState>,
    items: &[ProjectExplorerItemRef],
    before: Option<&ProjectExplorerItemRef>,
) -> Result<(), String> {
    let matches = |state: &ProjectExplorerItemState, item: &ProjectExplorerItemRef| {
        state.node_id == item.node_id
            && state.relative_path.as_deref().unwrap_or_default()
                == item.relative_path.as_deref().unwrap_or_default()
    };
    if items.iter().any(|item| {
        !states
            .iter()
            .any(|state| state.pinned && matches(state, item))
    }) {
        return Err("Only pinned workspace items can be reordered".to_string());
    }
    if let Some(anchor) = before {
        if !states
            .iter()
            .any(|state| state.pinned && matches(state, anchor))
        {
            return Err("Pinned insertion target is unavailable".to_string());
        }
        if states
            .iter()
            .any(|state| matches(state, anchor) && items.iter().any(|item| matches(state, item)))
        {
            return Ok(());
        }
    }
    // Move existing state records so their stars, identities and original tree
    // locations stay intact. The saved state order already defines the pin order.
    let moving = states
        .iter()
        .filter(|state| items.iter().any(|item| matches(state, item)))
        .cloned()
        .collect::<Vec<_>>();
    states.retain(|state| !items.iter().any(|item| matches(state, item)));
    let index = before
        .and_then(|anchor| states.iter().position(|state| matches(state, anchor)))
        .unwrap_or(states.len());
    states.splice(index..index, moving);
    Ok(())
}

pub fn apply_operations(
    root: &Path,
    project_id: &str,
    expected_revision: i64,
    operation_id: &str,
    operations: &[ProjectExplorerOperation],
) -> Result<ProjectExplorerMutationResult, String> {
    let operation_id = operation_id.trim();
    if operation_id.is_empty() {
        return Err("Workspace tree operation identity is required".to_string());
    }
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    let index = ensure_layout(root, project_id)?;
    let mut preset = read_preset(root, project_id, &index.active_preset_id)?;
    if preset.last_operation_id.as_deref() == Some(operation_id) {
        let snapshot = active_snapshot_unlocked(root, project_id)?;
        return Ok(ProjectExplorerMutationResult {
            operation_id: operation_id.to_string(),
            snapshot,
        });
    }
    if preset.revision != expected_revision {
        return Err(format!(
            "project_explorer_revision_conflict:{expected_revision}:{}",
            preset.revision
        ));
    }
    for operation in operations {
        if let ProjectExplorerOperation::SetItemState {
            node_id,
            relative_path,
            pinned,
            highlighted,
        } = operation
        {
            set_item_state(
                &mut preset,
                node_id,
                relative_path.as_deref(),
                *pinned,
                *highlighted,
            )?;
        } else if let ProjectExplorerOperation::MovePinnedItems { items, before } = operation {
            move_pinned_items(&mut preset.item_states, items, before.as_ref())?;
        } else {
            apply_operation(project_id, &mut preset.nodes, operation)?;
        }
    }
    preset.item_states.retain(|state| {
        preset
            .nodes
            .iter()
            .any(|node| node.node_id == state.node_id)
    });
    validate_nodes(project_id, &preset.nodes)?;
    preset.revision = preset
        .revision
        .checked_add(1)
        .ok_or_else(|| "Workspace tree revision is exhausted".to_string())?;
    preset.last_operation_id = Some(operation_id.to_string());
    write_json(&preset_path(root, &preset.preset_id), &preset)?;
    let snapshot = active_snapshot_unlocked(root, project_id)?;
    Ok(ProjectExplorerMutationResult {
        operation_id: operation_id.to_string(),
        snapshot,
    })
}

pub fn create_preset(
    root: &Path,
    project_id: &str,
    name: &str,
    source_preset_id: Option<&str>,
) -> Result<ProjectExplorerSnapshot, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workspace tree preset name cannot be empty".to_string());
    }
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    let mut index = ensure_layout(root, project_id)?;
    let source_id = source_preset_id.unwrap_or(&index.active_preset_id);
    let source = read_preset(root, project_id, source_id)?;
    let preset_id = format!("preset-{}", Uuid::new_v4().simple());
    let preset = WorkspaceTreePresetFile {
        schema_version: WORKSPACE_TREE_SCHEMA_VERSION,
        preset_id: preset_id.clone(),
        name: name.to_string(),
        project_id: project_id.to_string(),
        revision: 0,
        last_operation_id: None,
        nodes: source.nodes,
        item_states: source.item_states,
    };
    write_json(&preset_path(root, &preset_id), &preset)?;
    index.preset_order.push(preset_id.clone());
    index.active_preset_id = preset_id;
    write_json(&index_path(root), &index)?;
    active_snapshot_unlocked(root, project_id)
}

pub fn switch_preset(
    root: &Path,
    project_id: &str,
    preset_id: &str,
) -> Result<ProjectExplorerSnapshot, String> {
    let preset_id = validate_preset_id(preset_id)?.to_string();
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    let mut index = ensure_layout(root, project_id)?;
    read_preset(root, project_id, &preset_id)?;
    index.active_preset_id = preset_id.clone();
    if !index.preset_order.iter().any(|id| id == &preset_id) {
        index.preset_order.push(preset_id);
    }
    write_json(&index_path(root), &index)?;
    active_snapshot_unlocked(root, project_id)
}

pub fn rename_preset(
    root: &Path,
    project_id: &str,
    preset_id: &str,
    name: &str,
) -> Result<ProjectExplorerSnapshot, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workspace tree preset name cannot be empty".to_string());
    }
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    ensure_layout(root, project_id)?;
    let mut preset = read_preset(root, project_id, preset_id)?;
    preset.name = name.to_string();
    preset.revision = preset
        .revision
        .checked_add(1)
        .ok_or_else(|| "Workspace tree revision is exhausted".to_string())?;
    preset.last_operation_id = None;
    write_json(&preset_path(root, &preset.preset_id), &preset)?;
    active_snapshot_unlocked(root, project_id)
}

pub fn delete_preset(
    root: &Path,
    project_id: &str,
    preset_id: &str,
) -> Result<ProjectExplorerSnapshot, String> {
    let preset_id = validate_preset_id(preset_id)?.to_string();
    let lock = layout_lock(root)?;
    let _guard = lock
        .lock()
        .map_err(|error| format!("Workspace tree is unavailable: {error}"))?;
    let mut index = ensure_layout(root, project_id)?;
    let presets = load_presets(root, project_id, &index)?;
    if presets.len() <= 1 {
        return Err("A workspace must keep at least one tree preset".to_string());
    }
    if !presets.iter().any(|preset| preset.preset_id == preset_id) {
        return Err(format!("Workspace tree preset does not exist: {preset_id}"));
    }
    index.preset_order.retain(|id| id != &preset_id);
    if index.active_preset_id == preset_id {
        index.active_preset_id = index
            .preset_order
            .first()
            .cloned()
            .or_else(|| {
                presets
                    .iter()
                    .find(|preset| preset.preset_id != preset_id)
                    .map(|preset| preset.preset_id.clone())
            })
            .ok_or_else(|| "Workspace tree has no remaining preset".to_string())?;
    }
    write_json(&index_path(root), &index)?;
    std::fs::remove_file(preset_path(root, &preset_id)).map_err(|error| {
        format!("Failed to delete workspace tree preset '{preset_id}': {error}")
    })?;
    active_snapshot_unlocked(root, project_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_independent_text_files() {
        let temp = tempfile::tempdir().unwrap();
        let first = snapshot(temp.path(), "project-a").unwrap();
        assert_eq!(first.preset_id, DEFAULT_PRESET_ID);
        assert!(Path::new(&first.manifest_path).is_file());

        let second = create_preset(temp.path(), "project-a", "Review", None).unwrap();
        assert_ne!(second.preset_id, first.preset_id);
        assert!(Path::new(&second.manifest_path).is_file());
        assert_eq!(second.presets.len(), 2);

        let switched = switch_preset(temp.path(), "project-a", DEFAULT_PRESET_ID).unwrap();
        assert_eq!(switched.preset_id, DEFAULT_PRESET_ID);
    }

    #[test]
    fn view_placements_round_trip_move_and_remove() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("view.vue");
        std::fs::write(&source, "<template>View</template>").unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-view",
            &[
                ProjectExplorerOperation::CreateFolder {
                    node_id: Some("views-folder".to_string()),
                    parent_node_id: None,
                    name: "Tools".to_string(),
                    position: 0,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "view".to_string(),
                    resource_id: "test-view".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 1,
                },
            ],
        )
        .unwrap();
        let view_node_id = placed
            .snapshot
            .nodes
            .iter()
            .find(|node| node.resource_kind.as_deref() == Some("view"))
            .unwrap()
            .node_id
            .clone();
        let moved = apply_operations(
            temp.path(),
            "project-a",
            placed.snapshot.revision,
            "move-view",
            &[ProjectExplorerOperation::PlaceResource {
                node_id: None,
                resource_kind: "view".to_string(),
                resource_id: "test-view".to_string(),
                source_kind: None,
                parent_node_id: Some("views-folder".to_string()),
                position: 0,
            }],
        )
        .unwrap();
        let restored = snapshot(temp.path(), "project-a").unwrap();
        let views = restored
            .nodes
            .iter()
            .filter(|node| node.resource_kind.as_deref() == Some("view"))
            .collect::<Vec<_>>();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].node_id, view_node_id);
        assert_eq!(views[0].parent_node_id.as_deref(), Some("views-folder"));
        let removed = apply_operations(
            temp.path(),
            "project-a",
            moved.snapshot.revision,
            "remove-view",
            &[ProjectExplorerOperation::RemoveNode {
                node_id: view_node_id,
            }],
        )
        .unwrap();
        assert!(!removed
            .snapshot
            .nodes
            .iter()
            .any(|node| node.resource_kind.as_deref() == Some("view")));
        assert_eq!(
            std::fs::read_to_string(source).unwrap(),
            "<template>View</template>"
        );
    }

    #[test]
    fn mounted_paths_round_trip_and_reject_hidden_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("notes");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("readme.md"), "hello").unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let mounted = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "mount-1",
            &[ProjectExplorerOperation::MountPath {
                node_id: Some("mount-notes".to_string()),
                parent_node_id: None,
                path: source.to_string_lossy().into_owned(),
                source_kind: Some("knowledge".to_string()),
                name: Some("Notes".to_string()),
                position: 0,
            }],
        )
        .unwrap();
        let node = mounted.snapshot.nodes.first().unwrap();
        assert_eq!(node.source_kind.as_deref(), Some("knowledge"));
        assert_eq!(node.node_kind, "folder");

        let error = apply_operations(
            temp.path(),
            "project-a",
            mounted.snapshot.revision,
            "hide-1",
            &[ProjectExplorerOperation::SetNodeHidden {
                node_id: node.node_id.clone(),
                hidden: true,
            }],
        )
        .unwrap_err();
        assert!(error.contains("Only Locus system resources can be hidden"));
        assert!(!snapshot(temp.path(), "project-a").unwrap().nodes[0].hidden);
    }

    #[test]
    fn v1_presets_migrate_hidden_state_to_resource_specific_semantics() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree_dir(temp.path())).unwrap();
        write_json(
            &index_path(temp.path()),
            &WorkspaceTreeIndex {
                schema_version: LEGACY_WORKSPACE_TREE_SCHEMA_VERSION,
                active_preset_id: DEFAULT_PRESET_ID.to_string(),
                preset_order: vec![DEFAULT_PRESET_ID.to_string()],
            },
        )
        .unwrap();
        let resource = |node_id: &str, resource_kind: &str, resource_id: &str, position: i64| {
            ProjectExplorerNode {
                node_id: node_id.to_string(),
                project_id: "project-a".to_string(),
                node_kind: "resource".to_string(),
                parent_node_id: None,
                resource_kind: Some(resource_kind.to_string()),
                resource_id: Some(resource_id.to_string()),
                folder_name: None,
                hidden: true,
                source_path: None,
                source_kind: None,
                position,
            }
        };
        write_json(
            &preset_path(temp.path(), DEFAULT_PRESET_ID),
            &WorkspaceTreePresetFile {
                schema_version: LEGACY_WORKSPACE_TREE_SCHEMA_VERSION,
                preset_id: DEFAULT_PRESET_ID.to_string(),
                name: DEFAULT_PRESET_NAME.to_string(),
                project_id: "project-a".to_string(),
                revision: 7,
                last_operation_id: Some("legacy-operation".to_string()),
                item_states: Vec::new(),
                nodes: vec![
                    resource(
                        "knowledge-user-preference",
                        "knowledge",
                        "kd_builtin_memory_user_preference",
                        0,
                    ),
                    resource("session-a", "session", "session-a", 1),
                    resource("collaboration", "system", "collaboration", 2),
                ],
            },
        )
        .unwrap();

        let migrated = snapshot(temp.path(), "project-a").unwrap();
        assert_eq!(migrated.revision, 8);
        assert_eq!(migrated.nodes.len(), 2);
        assert!(migrated.nodes.iter().all(|node| {
            node.resource_id.as_deref() != Some("kd_builtin_memory_user_preference")
        }));
        assert!(
            !migrated
                .nodes
                .iter()
                .find(|node| node.resource_kind.as_deref() == Some("session"))
                .unwrap()
                .hidden
        );
        assert!(
            migrated
                .nodes
                .iter()
                .find(|node| node.resource_id.as_deref() == Some("collaboration"))
                .unwrap()
                .hidden
        );

        let persisted_index = read_index(temp.path()).unwrap();
        let persisted_preset = read_preset(temp.path(), "project-a", DEFAULT_PRESET_ID).unwrap();
        assert_eq!(
            persisted_index.schema_version,
            WORKSPACE_TREE_SCHEMA_VERSION
        );
        assert_eq!(
            persisted_preset.schema_version,
            WORKSPACE_TREE_SCHEMA_VERSION
        );
        assert_eq!(
            persisted_preset.last_operation_id.as_deref(),
            Some(WORKSPACE_TREE_MIGRATION_ID)
        );
        assert_eq!(
            snapshot(temp.path(), "project-a").unwrap().revision,
            migrated.revision
        );
    }

    #[test]
    fn shown_system_resources_return_to_the_workspace_root_head() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-collaboration",
            &[
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "session".to_string(),
                    resource_id: "session-a".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 0,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "system".to_string(),
                    resource_id: "collaboration".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 1,
                },
            ],
        )
        .unwrap();
        let node_id = placed
            .snapshot
            .nodes
            .iter()
            .find(|node| node.resource_id.as_deref() == Some("collaboration"))
            .map(|node| node.node_id.clone())
            .unwrap();
        let hidden = apply_operations(
            temp.path(),
            "project-a",
            placed.snapshot.revision,
            "hide-collaboration",
            &[ProjectExplorerOperation::SetNodeHidden {
                node_id: node_id.clone(),
                hidden: true,
            }],
        )
        .unwrap();

        assert!(
            hidden
                .snapshot
                .nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .unwrap()
                .hidden
        );

        let shown = apply_operations(
            temp.path(),
            "project-a",
            hidden.snapshot.revision,
            "show-collaboration",
            &[ProjectExplorerOperation::SetNodeHidden {
                node_id: node_id.clone(),
                hidden: false,
            }],
        )
        .unwrap();
        let mut roots = shown
            .snapshot
            .nodes
            .iter()
            .filter(|node| node.parent_node_id.is_none())
            .collect::<Vec<_>>();
        roots.sort_by_key(|node| node.position);

        assert_eq!(roots[0].node_id, node_id);
        assert!(!roots[0].hidden);
        assert_eq!(roots[0].position, 0);
        assert_eq!(roots[1].resource_id.as_deref(), Some("session-a"));
    }

    #[test]
    fn sessions_can_contain_sessions_and_reject_other_resource_children() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let nested = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-nested-sessions",
            &[
                ProjectExplorerOperation::PlaceResource {
                    node_id: Some("session-node:parent".to_string()),
                    resource_kind: "session".to_string(),
                    resource_id: "parent-session".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 0,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: Some("session-node:child".to_string()),
                    resource_kind: "session".to_string(),
                    resource_id: "child-session".to_string(),
                    source_kind: None,
                    parent_node_id: Some("session-node:parent".to_string()),
                    position: 0,
                },
            ],
        )
        .unwrap();
        let child = nested
            .snapshot
            .nodes
            .iter()
            .find(|node| node.resource_id.as_deref() == Some("child-session"))
            .unwrap();
        assert_eq!(child.parent_node_id.as_deref(), Some("session-node:parent"));
        assert_eq!(
            snapshot(temp.path(), "project-a")
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.resource_id.as_deref() == Some("child-session"))
                .and_then(|node| node.parent_node_id.as_deref()),
            Some("session-node:parent")
        );

        let manual_nesting = apply_operations(
            temp.path(),
            "project-a",
            nested.snapshot.revision,
            "manually-nest-parent-under-child",
            &[ProjectExplorerOperation::MoveNode {
                node_id: "session-node:parent".to_string(),
                parent_node_id: Some("session-node:child".to_string()),
                position: 0,
            }],
        )
        .unwrap_err();
        assert!(manual_nesting.contains("parent must be a folder"));

        let incompatible = apply_operations(
            temp.path(),
            "project-a",
            nested.snapshot.revision,
            "place-knowledge-under-session",
            &[ProjectExplorerOperation::PlaceResource {
                node_id: Some("knowledge-node".to_string()),
                resource_kind: "knowledge".to_string(),
                resource_id: "knowledge-a".to_string(),
                source_kind: Some("knowledge".to_string()),
                parent_node_id: Some("session-node:parent".to_string()),
                position: 1,
            }],
        )
        .unwrap_err();
        assert!(incompatible.contains("session for session children"));

        let cycle = apply_operations(
            temp.path(),
            "project-a",
            nested.snapshot.revision,
            "automatically-nest-parent-under-child",
            &[ProjectExplorerOperation::PlaceResource {
                node_id: None,
                resource_kind: "session".to_string(),
                resource_id: "parent-session".to_string(),
                source_kind: None,
                parent_node_id: Some("session-node:child".to_string()),
                position: 0,
            }],
        )
        .unwrap_err();
        assert!(cycle.contains("cannot move into its descendant"));

        let extracted = apply_operations(
            temp.path(),
            "project-a",
            nested.snapshot.revision,
            "move-child-out",
            &[ProjectExplorerOperation::MoveNode {
                node_id: "session-node:child".to_string(),
                parent_node_id: None,
                position: 1,
            }],
        )
        .unwrap();
        assert!(extracted
            .snapshot
            .nodes
            .iter()
            .find(|node| node.node_id == "session-node:child")
            .unwrap()
            .parent_node_id
            .is_none());
    }

    #[test]
    fn archived_system_resource_starts_hidden_and_can_be_shown() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-archived",
            &[ProjectExplorerOperation::PlaceResource {
                node_id: None,
                resource_kind: "system".to_string(),
                resource_id: "archived".to_string(),
                source_kind: None,
                parent_node_id: None,
                position: 0,
            }],
        )
        .unwrap();
        let archived = placed
            .snapshot
            .nodes
            .iter()
            .find(|node| node.resource_id.as_deref() == Some("archived"))
            .unwrap();
        assert!(archived.hidden);
        let archived_node_id = archived.node_id.clone();

        let shown = apply_operations(
            temp.path(),
            "project-a",
            placed.snapshot.revision,
            "show-archived",
            &[ProjectExplorerOperation::SetNodeHidden {
                node_id: archived_node_id.clone(),
                hidden: false,
            }],
        )
        .unwrap();

        assert!(
            !shown
                .snapshot
                .nodes
                .iter()
                .find(|node| node.node_id == archived_node_id)
                .unwrap()
                .hidden
        );
    }

    #[test]
    fn knowledge_removal_drops_the_placement_and_allows_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let placement = ProjectExplorerOperation::PlaceResource {
            node_id: None,
            resource_kind: "knowledge".to_string(),
            resource_id: "kd_builtin_memory_user_preference".to_string(),
            source_kind: Some("knowledge".to_string()),
            parent_node_id: None,
            position: 0,
        };
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-user-preference",
            std::slice::from_ref(&placement),
        )
        .unwrap();
        let removed = apply_operations(
            temp.path(),
            "project-a",
            placed.snapshot.revision,
            "remove-user-preference",
            &[ProjectExplorerOperation::RemoveResourcePlacement {
                resource_kind: "knowledge".to_string(),
                resource_id: "kd_builtin_memory_user_preference".to_string(),
            }],
        )
        .unwrap();
        assert!(removed.snapshot.nodes.is_empty());

        let replaced = apply_operations(
            temp.path(),
            "project-a",
            removed.snapshot.revision,
            "replace-user-preference",
            &[placement],
        )
        .unwrap();
        assert_eq!(replaced.snapshot.nodes.len(), 1);
        assert!(!replaced.snapshot.nodes[0].hidden);
    }

    #[test]
    fn pinned_reordering_persists_without_moving_nodes_or_clearing_stars() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let mut operations = Vec::new();
        for (position, name) in ["a", "b", "c"].iter().enumerate() {
            operations.push(ProjectExplorerOperation::CreateFolder {
                node_id: Some(name.to_string()),
                name: name.to_string(),
                parent_node_id: None,
                position: position as i64,
            });
            operations.push(ProjectExplorerOperation::SetItemState {
                node_id: name.to_string(),
                relative_path: None,
                pinned: Some(true),
                highlighted: Some(true),
            });
        }
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "seed-pins",
            &operations,
        )
        .unwrap()
        .snapshot;
        let reference = |name: &str| ProjectExplorerItemRef {
            node_id: name.into(),
            relative_path: None,
        };
        let reordered = apply_operations(
            temp.path(),
            "project-a",
            placed.revision,
            "reorder-pins",
            &[ProjectExplorerOperation::MovePinnedItems {
                items: vec![reference("b"), reference("c")],
                before: Some(reference("a")),
            }],
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            reordered
                .item_states
                .iter()
                .map(|state| state.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        assert!(reordered
            .item_states
            .iter()
            .all(|state| state.highlighted && state.pinned));
        assert_eq!(reordered.nodes, placed.nodes);
        assert_eq!(snapshot(temp.path(), "project-a").unwrap(), reordered);
        let tail = apply_operations(
            temp.path(),
            "project-a",
            reordered.revision,
            "pin-tail",
            &[ProjectExplorerOperation::MovePinnedItems {
                items: vec![reference("b")],
                before: None,
            }],
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            tail.item_states
                .iter()
                .map(|state| state.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
        let invalid = apply_operations(
            temp.path(),
            "project-a",
            tail.revision,
            "invalid-pin",
            &[ProjectExplorerOperation::MovePinnedItems {
                items: vec![reference("a")],
                before: Some(reference("missing")),
            }],
        );
        assert!(invalid.is_err());
        assert_eq!(snapshot(temp.path(), "project-a").unwrap(), tail);
    }

    #[test]
    fn pinned_reordering_distinguishes_mounted_paths_and_ignores_self_drops() {
        let reference = |path: &str| ProjectExplorerItemRef {
            node_id: "mount".into(),
            relative_path: Some(path.into()),
        };
        let mut states = ["first.md", "second.md", "unstarred.md"]
            .iter()
            .map(|path| ProjectExplorerItemState {
                node_id: "mount".into(),
                relative_path: Some(path.to_string()),
                pinned: *path != "unstarred.md",
                highlighted: true,
            })
            .collect::<Vec<_>>();
        move_pinned_items(
            &mut states,
            &[reference("second.md")],
            Some(&reference("first.md")),
        )
        .unwrap();
        assert_eq!(states[0].relative_path.as_deref(), Some("second.md"));
        let original = states.clone();
        move_pinned_items(
            &mut states,
            &[reference("first.md"), reference("second.md")],
            Some(&reference("first.md")),
        )
        .unwrap();
        assert_eq!(states, original);
        assert!(move_pinned_items(&mut states, &[reference("unstarred.md")], None).is_err());
        assert_eq!(states, original);
    }

    #[test]
    fn v2_presets_migrate_item_states_once_without_changing_layout() {
        let temp = tempfile::tempdir().unwrap();
        snapshot(temp.path(), "project-a").unwrap();
        let path = preset_path(temp.path(), DEFAULT_PRESET_ID);
        let mut previous: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        previous["schemaVersion"] = serde_json::json!(2);
        previous["revision"] = serde_json::json!(9);
        previous.as_object_mut().unwrap().remove("itemStates");
        write_json(&path, &previous).unwrap();
        let migrated = snapshot(temp.path(), "project-a").unwrap();
        assert_eq!(migrated.revision, 10);
        assert!(migrated.item_states.is_empty());
        assert_eq!(migrated, snapshot(temp.path(), "project-a").unwrap());
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(saved["schemaVersion"], 3);
        assert_eq!(saved["itemStates"], serde_json::json!([]));
    }

    #[test]
    fn item_states_persist_independently_and_copy_with_presets() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        assert!(initial.item_states.is_empty());
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place",
            &[
                ProjectExplorerOperation::CreateFolder {
                    node_id: Some("folder".into()),
                    parent_node_id: None,
                    name: "Notes".into(),
                    position: 0,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: Some("session".into()),
                    resource_kind: "session".into(),
                    resource_id: "session-a".into(),
                    source_kind: None,
                    parent_node_id: Some("folder".into()),
                    position: 0,
                },
                ProjectExplorerOperation::SetItemState {
                    node_id: "session".into(),
                    relative_path: None,
                    pinned: Some(true),
                    highlighted: Some(true),
                },
            ],
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            placed
                .nodes
                .iter()
                .find(|node| node.node_id == "session")
                .unwrap()
                .parent_node_id
                .as_deref(),
            Some("folder")
        );
        assert_eq!(snapshot(temp.path(), "project-a").unwrap(), placed);
        let copied = create_preset(temp.path(), "project-a", "Copy", None).unwrap();
        assert_eq!(copied.item_states, placed.item_states);
        let unpinned = apply_operations(
            temp.path(),
            "project-a",
            copied.revision,
            "unpin",
            &[ProjectExplorerOperation::SetItemState {
                node_id: "session".into(),
                relative_path: None,
                pinned: Some(false),
                highlighted: None,
            }],
        )
        .unwrap()
        .snapshot;
        assert!(!unpinned.item_states[0].pinned);
        assert!(unpinned.item_states[0].highlighted);
        assert_eq!(unpinned.nodes, placed.nodes);
        assert_eq!(
            apply_operations(temp.path(), "project-a", copied.revision, "unpin", &[])
                .unwrap()
                .snapshot,
            unpinned
        );
        let restored = switch_preset(temp.path(), "project-a", DEFAULT_PRESET_ID).unwrap();
        assert_eq!(restored.item_states, placed.item_states);
        let removed = apply_operations(
            temp.path(),
            "project-a",
            restored.revision,
            "remove",
            &[ProjectExplorerOperation::RemoveNode {
                node_id: "session".into(),
            }],
        )
        .unwrap()
        .snapshot;
        assert!(removed.item_states.is_empty());
    }

    #[test]
    fn file_actions_preserve_all_preset_placements_and_child_states() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let source = root.join("before.md");
        let target = root.join("after.md");
        std::fs::write(&source, "document").unwrap();
        let initial = snapshot(&root, "project-a").unwrap();
        let placed = apply_operations(&root, "project-a", initial.revision, "mount-file", &[
            ProjectExplorerOperation::MountPath { node_id: Some("file".into()), parent_node_id: None,
                path: source.to_string_lossy().into_owned(), source_kind: None, name: None, position: 0 },
            ProjectExplorerOperation::MountPath { node_id: Some("directory".into()), parent_node_id: None,
                path: root.to_string_lossy().into_owned(), source_kind: None, name: None, position: 1 },
            ProjectExplorerOperation::SetItemState { node_id: "file".into(), relative_path: None, pinned: Some(true), highlighted: Some(true) },
            ProjectExplorerOperation::SetItemState { node_id: "directory".into(), relative_path: Some("before.md".into()), pinned: Some(true), highlighted: Some(true) },
        ]).unwrap().snapshot;
        let copy = create_preset(&root, "project-a", "Other", None).unwrap();
        std::fs::rename(&source, &target).unwrap();
        let renamed = relocate_file_references(&root, "project-a", &source, Some(&target)).unwrap();
        for preset_id in [copy.preset_id.as_str(), placed.preset_id.as_str()] {
            let current = switch_preset(&root, "project-a", preset_id).unwrap();
            assert_eq!(current.nodes.iter().find(|node| node.node_id == "file").unwrap().source_path.as_deref(), target.to_str());
            assert!(current.item_states.iter().all(|state| state.pinned && state.highlighted));
            assert_eq!(current.item_states.iter().find(|state| state.node_id == "directory").unwrap().relative_path.as_deref(), Some("after.md"));
        }
        assert_eq!(renamed.item_states.len(), 2);
        let deleted = relocate_file_references(&root, "project-a", &target, None).unwrap();
        assert!(deleted.nodes.iter().all(|node| node.node_id != "file"));
        assert!(deleted.item_states.is_empty());
        let other = switch_preset(&root, "project-a", &copy.preset_id).unwrap();
        assert!(other.item_states.is_empty());
        assert_eq!(other.nodes.len(), 1);
    }

    #[test]
    fn mounted_child_states_do_not_mark_the_parent_or_escape_the_mount() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let mounted = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "mount",
            &[
                ProjectExplorerOperation::MountPath {
                    node_id: Some("mount".into()),
                    parent_node_id: None,
                    path: temp.path().to_string_lossy().into_owned(),
                    source_kind: None,
                    name: None,
                    position: 0,
                },
                ProjectExplorerOperation::SetItemState {
                    node_id: "mount".into(),
                    relative_path: Some("notes/design.md".into()),
                    pinned: Some(true),
                    highlighted: Some(true),
                },
            ],
        )
        .unwrap()
        .snapshot;
        assert_eq!(mounted.item_states.len(), 1);
        assert_eq!(
            mounted.item_states[0].relative_path.as_deref(),
            Some("notes/design.md")
        );
        assert_eq!(snapshot(temp.path(), "project-a").unwrap(), mounted);
        let error = apply_operations(
            temp.path(),
            "project-a",
            mounted.revision,
            "bad-path",
            &[
                ProjectExplorerOperation::SetItemState {
                    node_id: "mount".into(),
                    relative_path: None,
                    pinned: Some(true),
                    highlighted: None,
                },
                ProjectExplorerOperation::SetItemState {
                    node_id: "mount".into(),
                    relative_path: Some("../outside.md".into()),
                    pinned: Some(true),
                    highlighted: None,
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("relative to a mounted folder"));
        assert_eq!(snapshot(temp.path(), "project-a").unwrap(), mounted);
        let cleared = apply_operations(
            temp.path(),
            "project-a",
            mounted.revision,
            "clear",
            &[ProjectExplorerOperation::SetItemState {
                node_id: "mount".into(),
                relative_path: Some("notes/design.md".into()),
                pinned: Some(false),
                highlighted: Some(false),
            }],
        )
        .unwrap()
        .snapshot;
        assert!(cleared.item_states.is_empty());
    }

    #[test]
    fn system_resources_share_root_order_and_persist_tail_moves() {
        let temp = tempfile::tempdir().unwrap();
        let initial = snapshot(temp.path(), "project-a").unwrap();
        let placed = apply_operations(
            temp.path(),
            "project-a",
            initial.revision,
            "place-system-resources",
            &[
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "system".to_string(),
                    resource_id: "newSession".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 0,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "system".to_string(),
                    resource_id: "knowledge".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 1,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "session".to_string(),
                    resource_id: "session-a".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 2,
                },
                ProjectExplorerOperation::PlaceResource {
                    node_id: None,
                    resource_kind: "system".to_string(),
                    resource_id: "collaboration".to_string(),
                    source_kind: None,
                    parent_node_id: None,
                    position: 3,
                },
            ],
        )
        .unwrap();
        let knowledge_node_id = placed
            .snapshot
            .nodes
            .iter()
            .find(|node| {
                node.resource_kind.as_deref() == Some("system")
                    && node.resource_id.as_deref() == Some("knowledge")
            })
            .map(|node| node.node_id.clone())
            .unwrap();
        let moved = apply_operations(
            temp.path(),
            "project-a",
            placed.snapshot.revision,
            "move-knowledge-to-tail",
            &[ProjectExplorerOperation::MoveNode {
                node_id: knowledge_node_id,
                parent_node_id: None,
                position: placed.snapshot.nodes.len() as i64,
            }],
        )
        .unwrap();
        let mut roots = moved
            .snapshot
            .nodes
            .iter()
            .filter(|node| node.parent_node_id.is_none())
            .collect::<Vec<_>>();
        roots.sort_by_key(|node| node.position);
        assert_eq!(
            roots
                .iter()
                .map(|node| {
                    format!(
                        "{}:{}",
                        node.resource_kind.as_deref().unwrap_or_default(),
                        node.resource_id.as_deref().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                "system:newSession",
                "session:session-a",
                "system:collaboration",
                "system:knowledge",
            ]
        );
        let reloaded = snapshot(temp.path(), "project-a").unwrap();
        assert_eq!(reloaded.nodes, moved.snapshot.nodes);
    }
}
