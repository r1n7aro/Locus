use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::models::{
    ProjectExplorerMutationResult, ProjectExplorerNode, ProjectExplorerOperation,
    ProjectExplorerPresetSummary, ProjectExplorerSnapshot,
};

const WORKSPACE_TREE_SCHEMA_VERSION: u32 = 2;
const LEGACY_WORKSPACE_TREE_SCHEMA_VERSION: u32 = 1;
const WORKSPACE_TREE_DIR: &str = "workspace-trees";
const WORKSPACE_TREE_INDEX: &str = "index.json";
const DEFAULT_PRESET_ID: &str = "default";
const DEFAULT_PRESET_NAME: &str = "Default";
const WORKSPACE_TREE_V2_MIGRATION_ID: &str = "workspace-tree-migration-v2";
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
        LEGACY_WORKSPACE_TREE_SCHEMA_VERSION => {
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

fn migrate_preset_to_v2(preset: &mut WorkspaceTreePresetFile) -> Result<bool, String> {
    match preset.schema_version {
        WORKSPACE_TREE_SCHEMA_VERSION => return Ok(false),
        LEGACY_WORKSPACE_TREE_SCHEMA_VERSION => {}
        version => {
            return Err(format!(
                "Unsupported workspace tree preset schema version: {version}"
            ));
        }
    }

    // Knowledge removal is represented by the absence of a placement. Sessions
    // use their archive state. Visibility remains a property of Locus system nodes.
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
    preset.schema_version = WORKSPACE_TREE_SCHEMA_VERSION;
    preset.revision = preset
        .revision
        .checked_add(1)
        .ok_or_else(|| "Workspace tree revision is exhausted".to_string())?;
    preset.last_operation_id = Some(WORKSPACE_TREE_V2_MIGRATION_ID.to_string());
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
    let migrated = migrate_preset_to_v2(&mut preset)?;
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
            if !matches!(resource_kind.as_str(), "session" | "knowledge" | "system") {
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
        apply_operation(project_id, &mut preset.nodes, operation)?;
    }
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
            Some(WORKSPACE_TREE_V2_MIGRATION_ID)
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
