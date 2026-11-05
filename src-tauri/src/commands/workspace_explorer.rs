use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::text_preview::decode_text_preview;
use crate::binary_cache::BinaryCache;
use crate::error::AppError;
use crate::session::models::{
    ProjectExplorerMutationResult, ProjectExplorerOperation, ProjectExplorerPresetSummary,
    ProjectExplorerSnapshot,
};
use crate::workspace_service::{ProjectId, ProjectRegistry, ResolvedWorkspaceScope, WorkspaceRef};

pub const PROJECT_EXPLORER_CHANGED_EVENT: &str = "project-explorer-changed";

const MAX_MOUNT_ENTRIES: usize = 5_000;
const MAX_TEXT_PREVIEW_BYTES: u64 = 1024 * 1024;
const MAX_CSV_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
use crate::csv_document::CSV_FILE_WRITE_LOCK;

#[path = "csv_document.rs"]
mod csv_document;
pub use csv_document::*;

#[path = "explorer_file_actions.rs"]
mod explorer_file_actions;
pub use explorer_file_actions::*;

#[path = "workspace_file_move.rs"]
mod workspace_file_move;
pub use workspace_file_move::*;

#[path = "workspace_file_create.rs"]
mod workspace_file_create;
pub use workspace_file_create::*;

#[path = "workspace_file_import.rs"]
mod workspace_file_import;
pub use workspace_file_import::*;

#[path = "file_history.rs"]
mod file_history;
pub use file_history::*;

fn text_preview_limit(path: &Path) -> u64 {
    if path.extension().and_then(|value| value.to_str()).unwrap_or("").eq_ignore_ascii_case("csv") {
        MAX_CSV_DOCUMENT_BYTES
    } else {
        MAX_TEXT_PREVIEW_BYTES
    }
}
const MAX_MEDIA_PREVIEW_BYTES: u64 = 50 * 1024 * 1024;
/// Models are parsed in the webview; keep the limit aligned with the Unity
/// asset preview (`diff::content::within_binary_threshold`).
const MAX_MODEL_PREVIEW_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerChangedEvent {
    pub project_id: String,
    pub revision: i64,
    pub operation_id: String,
    pub preset_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerMountEntry {
    pub node_id: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerMountListing {
    pub node_id: String,
    pub root_path: String,
    pub entries: Vec<ProjectExplorerMountEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerFileRevision {
    pub exists: bool,
    pub size: u64,
    pub modified_at_nanos: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExplorerFilePreview {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size: u64,
    pub kind: String,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Binary cache URL of the image, PDF, audio, video or model the viewer
    /// fetches on demand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    pub truncated: bool,
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_relative_path: Option<String>,
    pub revision: ProjectExplorerFileRevision,
}

fn file_revision_from_metadata(metadata: &std::fs::Metadata) -> ProjectExplorerFileRevision {
    let size = metadata.len();
    let modified_at_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_default();
    ProjectExplorerFileRevision {
        exists: true,
        size,
        key: format!("{size}:{modified_at_nanos}"),
        modified_at_nanos,
    }
}

fn file_revision(path: &Path) -> Result<ProjectExplorerFileRevision, AppError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return Ok(ProjectExplorerFileRevision {
                exists: false,
                size: 0,
                modified_at_nanos: String::new(),
                key: "missing".to_string(),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProjectExplorerFileRevision {
                exists: false,
                size: 0,
                modified_at_nanos: String::new(),
                key: "missing".to_string(),
            });
        }
        Err(error) => {
            return Err(AppError::new(
                "workspace.explorer_file_metadata_failed",
                "The selected file could not be inspected.",
            )
            .detail(error.to_string()));
        }
    };
    Ok(file_revision_from_metadata(&metadata))
}

/// Existing checkout roots of a project, main checkouts (with a `.git`
/// directory) first.
fn project_checkouts(
    registry: &ProjectRegistry,
    project_id: String,
    operation: &'static str,
) -> Result<(ProjectId, Vec<PathBuf>), AppError> {
    let project_id = ProjectId::new(project_id).map_err(|error| {
        AppError::new(
            "workspace.project_identity_invalid",
            "The project identity is invalid.",
        )
        .detail(error.to_string())
        .operation(operation)
    })?;
    let project = registry.project(&project_id).ok_or_else(|| {
        AppError::new(
            "workspace.project_unavailable",
            "The project context is unavailable.",
        )
        .detail(project_id.to_string())
        .operation(operation)
    })?;
    let mut roots = project
        .checkout_sources()
        .map_err(|error| {
            AppError::new(
                "workspace.project_checkout_catalog_unavailable",
                "The project checkout catalog is unavailable.",
            )
            .detail(error)
            .operation(operation)
        })?
        .into_iter()
        .map(|checkout| checkout.root)
        .filter(|root| root.is_dir())
        .collect::<Vec<_>>();
    roots.sort_by_cached_key(|root| {
        (
            !root.join(".git").is_dir(),
            root.to_string_lossy().to_ascii_lowercase(),
        )
    });
    Ok((project_id, roots))
}

fn main_checkout(
    project_id: &ProjectId,
    roots: &[PathBuf],
    operation: &'static str,
) -> Result<PathBuf, AppError> {
    roots.first().cloned().ok_or_else(|| {
        AppError::new(
            "workspace.project_root_unavailable",
            "The project does not have an available working directory.",
        )
        .detail(project_id.to_string())
        .operation(operation)
    })
}

fn resolve_project(
    registry: &ProjectRegistry,
    project_id: String,
    operation: &'static str,
) -> Result<(ProjectId, PathBuf), AppError> {
    let (project_id, roots) = project_checkouts(registry, project_id, operation)?;
    let root = main_checkout(&project_id, &roots, operation)?;
    Ok((project_id, root))
}

/// The project's workspace tree directory in the user data directory, shared
/// by all of its checkouts. A tree an older version kept in a checkout
/// (`Locus/workspace-trees`) is copied over on first use.
fn layout_for_checkouts(
    project_id: &ProjectId,
    checkouts: &[PathBuf],
    operation: &'static str,
) -> Result<PathBuf, AppError> {
    let data_dir =
        crate::runtime_paths::active_data_dir().map_err(|error| explorer_error(error, operation))?;
    let layout = crate::workspace_tree::layout_dir(data_dir, project_id.as_str());
    crate::workspace_tree::adopt_legacy_layout(&layout, checkouts)
        .map_err(|error| explorer_error(error, operation))?;
    Ok(layout)
}

fn resolve_layout(
    registry: &ProjectRegistry,
    project_id: String,
    operation: &'static str,
) -> Result<(ProjectId, PathBuf), AppError> {
    let (project_id, roots) = project_checkouts(registry, project_id, operation)?;
    let layout = layout_for_checkouts(&project_id, &roots, operation)?;
    Ok((project_id, layout))
}

/// The main checkout root together with the user-level layout directory.
fn resolve_project_layout(
    registry: &ProjectRegistry,
    project_id: String,
    operation: &'static str,
) -> Result<(ProjectId, PathBuf, PathBuf), AppError> {
    let (project_id, roots) = project_checkouts(registry, project_id, operation)?;
    let root = main_checkout(&project_id, &roots, operation)?;
    let layout = layout_for_checkouts(&project_id, &roots, operation)?;
    Ok((project_id, root, layout))
}

fn explorer_error(error: String, operation: &'static str) -> AppError {
    if error.starts_with("project_explorer_revision_conflict:") {
        AppError::new(
            "workspace.explorer_revision_conflict",
            "The workspace tree changed in another window or process.",
        )
        .detail(error)
        .operation(operation)
        .retryable(true)
    } else {
        AppError::new(
            "workspace.explorer_operation_failed",
            "The workspace tree operation failed.",
        )
        .detail(error)
        .operation(operation)
    }
}

fn emit_changed(
    app_handle: &AppHandle,
    snapshot: &ProjectExplorerSnapshot,
    operation_id: String,
) -> Result<(), AppError> {
    app_handle
        .emit(
            PROJECT_EXPLORER_CHANGED_EVENT,
            ProjectExplorerChangedEvent {
                project_id: snapshot.project_id.clone(),
                revision: snapshot.revision,
                operation_id,
                preset_id: snapshot.preset_id.clone(),
            },
        )
        .map_err(|error| {
            AppError::new(
                "workspace.explorer_event_failed",
                "The workspace tree update could not be published.",
            )
            .detail(error.to_string())
            .operation("projectExplorerEmitChanged")
        })
}

/// Removes deleted sessions from the project's shared workspace tree.
pub(crate) fn remove_deleted_session_placements(
    app_handle: &AppHandle,
    registry: &ProjectRegistry,
    project_id: String,
    session_ids: &[String],
) -> Result<(), AppError> {
    const OPERATION: &str = "removeDeletedSessionPlacements";
    let (project_id, layout) = resolve_layout(registry, project_id, OPERATION)?;
    let operation_id = uuid::Uuid::new_v4().to_string();
    let snapshot = crate::workspace_tree::remove_session_placements(
        &layout,
        project_id.as_str(),
        &operation_id,
        session_ids,
    )
    .map_err(|error| explorer_error(error, OPERATION))?;
    if let Some(snapshot) = snapshot {
        emit_changed(app_handle, &snapshot, operation_id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn project_explorer_snapshot(
    project_id: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerSnapshot",
    )?;
    crate::workspace_tree::snapshot(&layout, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerSnapshot"))
}

#[tauri::command]
pub async fn project_explorer_apply_operations(
    project_id: String,
    expected_revision: i64,
    operation_id: String,
    operations: Vec<ProjectExplorerOperation>,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerMutationResult, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerApplyOperations",
    )?;
    let result = crate::workspace_tree::apply_operations(
        &layout,
        project_id.as_str(),
        expected_revision,
        &operation_id,
        &operations,
    )
    .map_err(|error| explorer_error(error, "projectExplorerApplyOperations"))?;
    emit_changed(&app_handle, &result.snapshot, result.operation_id.clone())?;
    Ok(result)
}

#[tauri::command]
pub async fn project_explorer_list_presets(
    project_id: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<ProjectExplorerPresetSummary>, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerListPresets",
    )?;
    crate::workspace_tree::list_presets(&layout, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerListPresets"))
}

#[tauri::command]
pub async fn project_explorer_create_preset(
    project_id: String,
    name: String,
    source_preset_id: Option<String>,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerCreatePreset",
    )?;
    let snapshot = crate::workspace_tree::create_preset(
        &layout,
        project_id.as_str(),
        &name,
        source_preset_id.as_deref(),
    )
    .map_err(|error| explorer_error(error, "projectExplorerCreatePreset"))?;
    emit_changed(
        &app_handle,
        &snapshot,
        format!("preset-create:{}", snapshot.preset_id),
    )?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn project_explorer_switch_preset(
    project_id: String,
    preset_id: String,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerSwitchPreset",
    )?;
    let snapshot = crate::workspace_tree::switch_preset(&layout, project_id.as_str(), &preset_id)
        .map_err(|error| explorer_error(error, "projectExplorerSwitchPreset"))?;
    emit_changed(
        &app_handle,
        &snapshot,
        format!("preset-switch:{}", snapshot.preset_id),
    )?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn project_explorer_rename_preset(
    project_id: String,
    preset_id: String,
    name: String,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerRenamePreset",
    )?;
    let snapshot =
        crate::workspace_tree::rename_preset(&layout, project_id.as_str(), &preset_id, &name)
            .map_err(|error| explorer_error(error, "projectExplorerRenamePreset"))?;
    emit_changed(&app_handle, &snapshot, format!("preset-rename:{preset_id}"))?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn project_explorer_delete_preset(
    project_id: String,
    preset_id: String,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerDeletePreset",
    )?;
    let snapshot = crate::workspace_tree::delete_preset(&layout, project_id.as_str(), &preset_id)
        .map_err(|error| explorer_error(error, "projectExplorerDeletePreset"))?;
    emit_changed(&app_handle, &snapshot, format!("preset-delete:{preset_id}"))?;
    Ok(snapshot)
}

fn canonical_mounted_root(
    snapshot: &ProjectExplorerSnapshot,
    node_id: &str,
) -> Result<PathBuf, AppError> {
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .ok_or_else(|| {
            AppError::new(
                "workspace.explorer_mount_missing",
                "The mounted directory is unavailable.",
            )
            .detail(node_id.to_string())
        })?;
    let source = node.source_path.as_deref().ok_or_else(|| {
        AppError::new(
            "workspace.explorer_mount_path_missing",
            "The workspace tree node does not contain a local path.",
        )
        .detail(node_id.to_string())
    })?;
    let canonical = dunce::canonicalize(source).map_err(|error| {
        AppError::new(
            "workspace.explorer_mount_unavailable",
            "The mounted directory is unavailable.",
        )
        .detail(format!("{source}: {error}"))
    })?;
    if !canonical.is_dir() {
        return Err(AppError::new(
            "workspace.explorer_mount_not_directory",
            "The mounted path is not a directory.",
        )
        .detail(canonical.to_string_lossy().into_owned()));
    }
    Ok(canonical)
}

#[tauri::command]
pub async fn project_explorer_list_mount(
    project_id: String,
    node_id: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerMountListing, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerListMount",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&layout, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerListMount"))?;
    let mounted_root = canonical_mounted_root(&snapshot, &node_id)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in walkdir::WalkDir::new(&mounted_root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .skip(1)
    {
        let Ok(entry) = entry else {
            continue;
        };
        if entries.len() >= MAX_MOUNT_ENTRIES {
            truncated = true;
            break;
        }
        let relative = entry.path().strip_prefix(&mounted_root).map_err(|error| {
            AppError::new(
                "workspace.explorer_mount_scope_invalid",
                "A mounted directory entry escaped its root.",
            )
            .detail(error.to_string())
        })?;
        entries.push(ProjectExplorerMountEntry {
            node_id: node_id.clone(),
            relative_path: relative.to_string_lossy().replace('\\', "/"),
            absolute_path: entry.path().to_string_lossy().into_owned(),
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: entry.file_type().is_dir(),
            depth: entry.depth().saturating_sub(1),
        });
    }
    Ok(ProjectExplorerMountListing {
        node_id,
        root_path: mounted_root.to_string_lossy().into_owned(),
        entries,
        truncated,
    })
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn ensure_preview_path_authorized(
    snapshot: &ProjectExplorerSnapshot,
    requested: &Path,
) -> Result<PathBuf, AppError> {
    let canonical = dunce::canonicalize(requested).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_unavailable",
            "The selected file is unavailable.",
        )
        .detail(format!("{}: {error}", requested.display()))
    })?;
    if !canonical.is_file() {
        return Err(AppError::new(
            "workspace.explorer_preview_requires_file",
            "The selected workspace tree entry is not a file.",
        )
        .detail(canonical.to_string_lossy().into_owned()));
    }
    let requested_key = path_key(&canonical);
    let authorized = snapshot.nodes.iter().any(|node| {
        let Some(source_path) = node.source_path.as_deref() else {
            return false;
        };
        let Ok(source) = dunce::canonicalize(source_path) else {
            return false;
        };
        if source.is_file() {
            path_key(&source) == requested_key
        } else {
            canonical.starts_with(&source)
        }
    });
    if !authorized {
        return Err(AppError::new(
            "workspace.explorer_preview_scope_denied",
            "The file is outside the active workspace tree preset.",
        )
        .detail(canonical.to_string_lossy().into_owned()));
    }
    Ok(canonical)
}

fn ensure_revision_path_authorized(
    snapshot: &ProjectExplorerSnapshot,
    requested: &Path,
) -> Result<PathBuf, AppError> {
    if requested.exists() {
        return ensure_preview_path_authorized(snapshot, requested);
    }
    if !requested.is_absolute() {
        return Err(AppError::new(
            "workspace.explorer_preview_scope_denied",
            "The file is outside the active workspace tree preset.",
        )
        .detail(requested.to_string_lossy().into_owned()));
    }
    let candidate = requested
        .parent()
        .and_then(|parent| dunce::canonicalize(parent).ok())
        .and_then(|parent| requested.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| requested.to_path_buf());
    let candidate_key = path_key(&candidate);
    let authorized = snapshot.nodes.iter().any(|node| {
        let Some(source_path) = node.source_path.as_deref() else {
            return false;
        };
        let source_path = Path::new(source_path);
        if path_key(source_path) == candidate_key {
            return true;
        }
        let Ok(source) = dunce::canonicalize(source_path) else {
            return false;
        };
        source.is_dir() && candidate.starts_with(source)
    });
    if !authorized {
        return Err(AppError::new(
            "workspace.explorer_preview_scope_denied",
            "The file is outside the active workspace tree preset.",
        )
        .detail(candidate.to_string_lossy().into_owned()));
    }
    Ok(candidate)
}

fn mime_for_extension(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "glb" => "model/gltf-binary",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

/// 3D model formats the webview viewer can render.
fn is_model_preview_extension(extension: &str) -> bool {
    matches!(extension, "glb" | "fbx")
}

pub(super) fn is_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "json"
            | "jsonc"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "less"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "vue"
            | "svelte"
            | "rs"
            | "cs"
            | "cpp"
            | "c"
            | "h"
            | "hpp"
            | "java"
            | "kt"
            | "kts"
            | "py"
            | "rb"
            | "php"
            | "go"
            | "swift"
            | "sh"
            | "ps1"
            | "bat"
            | "cmd"
            | "sql"
            | "graphql"
            | "gql"
            | "ini"
            | "cfg"
            | "conf"
            | "log"
            | "csv"
            | "view"
            | "tsv"
            | "asmdef"
            | "asmref"
            | "shader"
            | "compute"
            | "cginc"
            | "hlsl"
            | "glsl"
            | "uss"
            | "uxml"
    )
}

fn unity_workspace_binding(
    registry: &ProjectRegistry,
    project_id: &ProjectId,
    canonical: &Path,
) -> Option<(String, u64, u64, String)> {
    let project = registry.project(project_id)?;
    let mut runtimes = project.runtimes();
    runtimes.sort_by(|left, right| left.checkout_id().cmp(right.checkout_id()));
    for runtime in runtimes {
        let Ok(relative) = canonical.strip_prefix(runtime.root()) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.starts_with("Assets/")
            || relative.starts_with("Packages/")
            || relative.starts_with("ProjectSettings/")
        {
            return Some((
                runtime.checkout_id().to_string(),
                runtime.generation(),
                runtime.materialization_epoch(),
                relative,
            ));
        }
    }
    None
}

/// `blobs` receives the bytes of previewable media and models. Callers without
/// a viewer (saves) pass `None` and get the information card instead.
fn build_file_preview(
    registry: &ProjectRegistry,
    project_id: &ProjectId,
    canonical: &Path,
    blobs: Option<&BinaryCache>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let metadata = std::fs::metadata(canonical).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_metadata_failed",
            "The selected file could not be inspected.",
        )
        .detail(error.to_string())
    })?;
    let size = metadata.len();
    let revision = file_revision_from_metadata(&metadata);
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
    let extension = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // PDFs open in the reader inside Unity folders too: Unity imports them as
    // a plain DefaultAsset with nothing to inspect.
    if !is_text_extension(&extension) && extension != "pdf" {
        if let Some((checkout_id, workspace_generation, materialization_epoch, relative_path)) =
            unity_workspace_binding(registry, project_id, canonical)
        {
            return Ok(ProjectExplorerFilePreview {
                path: canonical.to_string_lossy().into_owned(),
                name,
                extension,
                size,
                kind: "unity".to_string(),
                mime_type: "application/x-unity-asset".to_string(),
                text: None,
                content_hash: None,
                blob_url: None,
                total_lines: None,
                truncated: false,
                editable: false,
                checkout_id: Some(checkout_id),
                workspace_generation: Some(workspace_generation),
                materialization_epoch: Some(materialization_epoch),
                workspace_relative_path: Some(relative_path),
                revision: revision.clone(),
            });
        }
    }

    let mime_type = mime_for_extension(&extension).to_string();
    // The viewer element fetches the bytes from the binary cache by URL; they
    // never travel through the IPC reply.
    let blob_kind = if is_model_preview_extension(&extension) {
        Some(("model", MAX_MODEL_PREVIEW_BYTES))
    } else if mime_type.starts_with("image/") {
        Some(("image", MAX_MEDIA_PREVIEW_BYTES))
    } else if mime_type == "application/pdf" {
        Some(("pdf", MAX_MEDIA_PREVIEW_BYTES))
    } else if mime_type.starts_with("audio/") {
        Some(("audio", MAX_MEDIA_PREVIEW_BYTES))
    } else if mime_type.starts_with("video/") {
        Some(("video", MAX_MEDIA_PREVIEW_BYTES))
    } else {
        None
    };
    if let (Some(cache), Some((kind, limit))) = (blobs, blob_kind) {
        if size <= limit {
            let bytes = std::fs::read(canonical).map_err(|error| {
                AppError::new(
                    "workspace.explorer_file_read_failed",
                    "The selected file could not be read.",
                )
                .detail(error.to_string())
            })?;
            let blob_id = cache.insert(bytes, mime_type.clone());
            return Ok(ProjectExplorerFilePreview {
                path: canonical.to_string_lossy().into_owned(),
                name,
                extension,
                size,
                kind: kind.to_string(),
                mime_type,
                text: None,
                content_hash: None,
                blob_url: Some(format!("{}{blob_id}", locus_remote_protocol::BLOB_URL_PREFIX)),
                total_lines: None,
                truncated: false,
                editable: false,
                checkout_id: None,
                workspace_generation: None,
                materialization_epoch: None,
                workspace_relative_path: None,
                revision,
            });
        }
    }

    let mut file = std::fs::File::open(canonical).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_read_failed",
            "The selected file could not be read.",
        )
        .detail(error.to_string())
    })?;
    let mut bytes = Vec::new();
    let preview_limit = text_preview_limit(canonical);
    file.by_ref()
        .take(preview_limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::new(
                "workspace.explorer_file_read_failed",
                "The selected file could not be read.",
            )
            .detail(error.to_string())
        })?;
    let truncated = bytes.len() as u64 > preview_limit;
    if truncated {
        bytes.truncate(preview_limit as usize);
    }
    let content_hash = if truncated {
        None
    } else {
        Some(blake3::hash(&bytes).to_hex().to_string())
    };
    if let Some(decoded) = decode_text_preview(&bytes, truncated) {
        let text = decoded.text;
        let total_lines = text.lines().count().max(1);
        return Ok(ProjectExplorerFilePreview {
            path: canonical.to_string_lossy().into_owned(),
            name,
            extension,
            size,
            kind: "text".to_string(),
            mime_type: if mime_type == "application/octet-stream" {
                "text/plain".to_string()
            } else {
                mime_type
            },
            text: Some(text),
            content_hash,
            blob_url: None,
            total_lines: Some(total_lines),
            truncated,
            editable: !truncated && decoded.utf8,
            checkout_id: None,
            workspace_generation: None,
            workspace_relative_path: None,
            materialization_epoch: None,
            revision: revision.clone(),
        });
    }
    Ok(ProjectExplorerFilePreview {
        path: canonical.to_string_lossy().into_owned(),
        name,
        extension,
        size,
        kind: "binary".to_string(),
        mime_type,
        text: None,
        content_hash: None,
        blob_url: None,
        total_lines: None,
        truncated: size > MAX_MEDIA_PREVIEW_BYTES,
        editable: false,
        checkout_id: None,
        workspace_generation: None,
        materialization_epoch: None,
        workspace_relative_path: None,
        revision,
    })
}

#[tauri::command]
pub async fn project_explorer_preview_file(
    project_id: String,
    path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
    binary_cache: State<'_, Arc<BinaryCache>>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerPreviewFile",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&layout, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerPreviewFile"))?;
    let canonical = ensure_preview_path_authorized(&snapshot, Path::new(&path))?;
    build_file_preview(registry.inner().as_ref(), &project_id, &canonical, Some(binary_cache.inner()))
}

#[tauri::command]
pub async fn project_explorer_file_revision(
    project_id: String,
    path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerFileRevision, AppError> {
    let (project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerFileRevision",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&layout, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerFileRevision"))?;
    let candidate = ensure_revision_path_authorized(&snapshot, Path::new(&path))?;
    file_revision(&candidate)
}

fn workspace_file_scope(
    workspace_ref: &WorkspaceRef,
    registry: &ProjectRegistry,
) -> Result<ResolvedWorkspaceScope, AppError> {
    registry
        .resolve_workspace_ref(workspace_ref)
        .map_err(|error| {
            AppError::new(
                "workspace.file_scope_unavailable",
                "The requested workspace is unavailable.",
            )
            .detail(error.to_string())
        })
}

fn resolve_workspace_file(
    workspace_ref: &WorkspaceRef,
    path: &str,
    registry: &ProjectRegistry,
) -> Result<(ResolvedWorkspaceScope, ProjectId, PathBuf), AppError> {
    let resolved = workspace_file_scope(workspace_ref, registry)?;
    let runtime = resolved.runtime();
    let project_id = runtime.project_id().clone();
    let (canonical, _) = super::asset::resolve_workspace_path(runtime.root(), path)?;
    Ok((resolved, project_id, canonical))
}

/// Canonical form of an absolute path that may not exist yet.
fn absolute_file_candidate(requested: &Path) -> Result<PathBuf, AppError> {
    if requested.exists() {
        return dunce::canonicalize(requested).map_err(|error| {
            AppError::new(
                "workspace.explorer_file_metadata_failed",
                "The selected file could not be inspected.",
            )
            .detail(error.to_string())
        });
    }
    Ok(requested
        .parent()
        .and_then(|parent| dunce::canonicalize(parent).ok())
        .and_then(|parent| requested.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| requested.to_path_buf()))
}

/// Existing regular file addressed by an absolute path, wherever it lives.
fn absolute_existing_file(requested: &Path, display: &str) -> Result<PathBuf, AppError> {
    let canonical = dunce::canonicalize(requested).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new("asset.preview.not_found", format!("File not found: {display}"))
        } else {
            AppError::new(
                "workspace.explorer_file_metadata_failed",
                "The selected file could not be inspected.",
            )
            .detail(error.to_string())
        }
    })?;
    if !canonical.is_file() {
        return Err(AppError::new(
            "asset.preview.not_file",
            format!("Path is not a regular file: {display}"),
        ));
    }
    Ok(canonical)
}

/// Editor tabs open any local file by absolute path, like chat file
/// references do. Only content commands (preview, revision, text and CSV
/// layout writes) use this; rename, move, history and CSV relocation stay
/// limited to the workspace through `resolve_workspace_file`.
fn resolve_openable_workspace_file(
    workspace_ref: &WorkspaceRef,
    path: &str,
    registry: &ProjectRegistry,
) -> Result<(ResolvedWorkspaceScope, ProjectId, PathBuf), AppError> {
    let requested = Path::new(path.trim());
    if !requested.is_absolute() {
        return resolve_workspace_file(workspace_ref, path, registry);
    }
    let resolved = workspace_file_scope(workspace_ref, registry)?;
    let project_id = resolved.runtime().project_id().clone();
    let canonical = absolute_existing_file(requested, path)?;
    Ok((resolved, project_id, canonical))
}

fn resolve_openable_workspace_file_candidate(
    workspace_ref: &WorkspaceRef,
    path: &str,
    registry: &ProjectRegistry,
) -> Result<(ResolvedWorkspaceScope, PathBuf), AppError> {
    let requested = Path::new(path.trim());
    if !requested.is_absolute() {
        return resolve_workspace_file_candidate(workspace_ref, path, registry);
    }
    let resolved = workspace_file_scope(workspace_ref, registry)?;
    Ok((resolved, absolute_file_candidate(requested)?))
}

fn resolve_workspace_file_candidate(
    workspace_ref: &WorkspaceRef,
    path: &str,
    registry: &ProjectRegistry,
) -> Result<(ResolvedWorkspaceScope, PathBuf), AppError> {
    let resolved = workspace_file_scope(workspace_ref, registry)?;
    let requested = Path::new(path.trim());
    if requested.is_absolute() {
        let candidate = absolute_file_candidate(requested)?;
        let canonical_root = dunce::canonicalize(resolved.runtime().root())
            .unwrap_or_else(|_| resolved.runtime().root().to_path_buf());
        if !candidate.starts_with(&canonical_root) {
            return Err(AppError::new(
                "asset.preview.invalid_path",
                format!("Path is not within the workspace: {path}"),
            ));
        }
        return Ok((resolved, candidate));
    }
    let relative = super::workspace::normalize_workspace_sub_path(path).map_err(|_| {
        AppError::new(
            "asset.preview.invalid_path",
            format!("Path is not within the workspace: {path}"),
        )
    })?;
    if relative.is_empty() {
        return Err(AppError::new(
            "asset.preview.invalid_path",
            "Asset preview path cannot be empty.",
        ));
    }
    let candidate = resolved.runtime().root().join(&relative);
    if candidate.exists()
        && !super::workspace::workspace_entry_target_allowed(
            resolved.runtime().root(),
            &relative,
            &candidate,
        )
    {
        return Err(AppError::new(
            "asset.preview.invalid_path",
            format!("Path is not within the workspace: {path}"),
        ));
    }
    Ok((resolved, candidate))
}

#[tauri::command]
pub async fn workspace_file_preview(
    file_path: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
    binary_cache: State<'_, Arc<BinaryCache>>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let (_scope, project_id, canonical) =
        resolve_openable_workspace_file(&workspace_ref, &file_path, registry.inner().as_ref())?;
    build_file_preview(registry.inner().as_ref(), &project_id, &canonical, Some(binary_cache.inner()))
}

/// Releases the model bytes a preview put in the binary cache once its viewer
/// no longer needs them. Unknown or foreign URLs are ignored.
#[tauri::command]
pub async fn release_preview_blob(
    blob_url: String,
    binary_cache: State<'_, Arc<BinaryCache>>,
) -> Result<bool, AppError> {
    Ok(blob_url
        .strip_prefix(locus_remote_protocol::BLOB_URL_PREFIX)
        .is_some_and(|id| binary_cache.release(id)))
}

#[tauri::command]
pub async fn workspace_file_revision(
    file_path: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerFileRevision, AppError> {
    let (_scope, candidate) = resolve_openable_workspace_file_candidate(
        &workspace_ref,
        &file_path,
        registry.inner().as_ref(),
    )?;
    file_revision(&candidate)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextFileWriteValidationError {
    CurrentFileNotEditable,
    CurrentFileChanged,
    EditedFileTooLarge,
}

fn validate_text_file_write(
    current: &[u8],
    content: &str,
    expected_content_hash: &str,
) -> Result<(), TextFileWriteValidationError> {
    validate_text_file_write_with_limit(current, content, expected_content_hash, MAX_TEXT_PREVIEW_BYTES)
}

fn validate_text_file_write_with_limit(
    current: &[u8], content: &str, expected_content_hash: &str, limit: u64,
) -> Result<(), TextFileWriteValidationError> {
    if current.len() as u64 > limit
        || !decode_text_preview(current, false).is_some_and(|decoded| decoded.utf8)
    {
        return Err(TextFileWriteValidationError::CurrentFileNotEditable);
    }
    let expected_content_hash = expected_content_hash.trim();
    let current_hash = blake3::hash(current).to_hex().to_string();
    if expected_content_hash.is_empty() || expected_content_hash != current_hash {
        return Err(TextFileWriteValidationError::CurrentFileChanged);
    }
    if content.len() as u64 > limit || content.as_bytes().contains(&0) {
        return Err(TextFileWriteValidationError::EditedFileTooLarge);
    }
    Ok(())
}

fn write_text_file(
    registry: &ProjectRegistry,
    project_id: &ProjectId,
    canonical: &Path,
    content: &str,
    expected_content_hash: &str,
    operation: &'static str,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let _document_guard = if canonical.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")) {
        Some(super::view_documents::DOCUMENT_WRITE_LOCK.lock()
            .map_err(|error| AppError::new("document.write_lock", error.to_string()))?)
    } else { None };
    let csv_path = canonical.to_string_lossy().to_ascii_lowercase();
    let _csv_guard = if csv_path.ends_with(".csv") || csv_path.ends_with(".csv.view") {
        Some(CSV_FILE_WRITE_LOCK.lock().map_err(|error| AppError::new("csv.write_lock", error.to_string()))?)
    } else { None };
    let extension = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if unity_workspace_binding(registry, project_id, canonical).is_some()
        && !is_text_extension(&extension)
    {
        return Err(AppError::new(
            "workspace.explorer_file_uses_unity_inspector",
            "This Unity asset is edited through Locus Inspector.",
        )
        .detail(canonical.to_string_lossy().into_owned())
        .operation(operation));
    }

    let current = std::fs::read(canonical).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_read_failed",
            "The selected file could not be read.",
        )
        .detail(error.to_string())
        .operation(operation)
    })?;
    if let Err(validation_error) =
        validate_text_file_write_with_limit(&current, content, expected_content_hash, text_preview_limit(canonical))
    {
        let (code, message) = match validation_error {
            TextFileWriteValidationError::CurrentFileNotEditable => (
                "workspace.explorer_file_not_editable",
                "This file cannot be edited as UTF-8 text.",
            ),
            TextFileWriteValidationError::CurrentFileChanged => (
                "workspace.explorer_file_changed",
                "The file changed on disk after this editor loaded it.",
            ),
            TextFileWriteValidationError::EditedFileTooLarge => (
                "workspace.explorer_file_too_large",
                "The edited file exceeds the text editor limit.",
            ),
        };
        return Err(AppError::new(code, message)
            .detail(canonical.to_string_lossy().into_owned())
            .operation(operation));
    }
    crate::config::atomic_write_config(canonical, content.as_bytes()).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_write_failed",
            "The edited file could not be saved.",
        )
        .detail(error)
        .operation(operation)
    })?;

    build_file_preview(registry, project_id, canonical, None)
}

#[tauri::command]
pub async fn project_explorer_write_file(
    project_id: String,
    path: String,
    content: String,
    expected_content_hash: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let (resolved_project_id, layout) = resolve_layout(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerWriteFile",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&layout, resolved_project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerWriteFile"))?;
    let canonical = ensure_preview_path_authorized(&snapshot, Path::new(&path))?;
    write_text_file(
        registry.inner().as_ref(),
        &resolved_project_id,
        &canonical,
        &content,
        &expected_content_hash,
        "projectExplorerWriteFile",
    )
}

#[tauri::command]
pub async fn workspace_file_write(
    file_path: String,
    content: String,
    expected_content_hash: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let (_scope, project_id, canonical) =
        resolve_openable_workspace_file(&workspace_ref, &file_path, registry.inner().as_ref())?;
    write_text_file(
        registry.inner().as_ref(),
        &project_id,
        &canonical,
        &content,
        &expected_content_hash,
        "workspaceFileWrite",
    )
}

#[cfg(test)]
#[path = "workspace_text_preview_tests.rs"]
mod text_preview_tests;

#[cfg(test)]
mod file_write_tests {
    use super::{
        file_revision, is_text_extension, validate_text_file_write, TextFileWriteValidationError,
        MAX_TEXT_PREVIEW_BYTES,
    };

    #[test]
    fn source_extensions_are_editable_while_structured_unity_assets_use_inspector() {
        assert!(is_text_extension("cs"));
        assert!(is_text_extension("shader"));
        assert!(is_text_extension("asmdef"));
        assert!(!is_text_extension("prefab"));
        assert!(!is_text_extension("unity"));
        assert!(!is_text_extension("asset"));
    }

    #[test]
    fn text_writes_require_the_loaded_content_hash() {
        let current = b"const value = 1;\n";
        let hash = blake3::hash(current).to_hex().to_string();
        assert_eq!(
            validate_text_file_write(current, "const value = 2;\n", &hash),
            Ok(())
        );
        assert_eq!(
            validate_text_file_write(current, "const value = 2;\n", "stale"),
            Err(TextFileWriteValidationError::CurrentFileChanged)
        );
    }

    #[test]
    fn text_writes_reject_binary_and_oversized_content() {
        let binary = b"text\0binary";
        let binary_hash = blake3::hash(binary).to_hex().to_string();
        assert_eq!(
            validate_text_file_write(binary, "next", &binary_hash),
            Err(TextFileWriteValidationError::CurrentFileNotEditable)
        );

        let current = b"small";
        let hash = blake3::hash(current).to_hex().to_string();
        let oversized = "x".repeat(MAX_TEXT_PREVIEW_BYTES as usize + 1);
        assert_eq!(
            validate_text_file_write(current, &oversized, &hash),
            Err(TextFileWriteValidationError::EditedFileTooLarge)
        );
    }

    #[test]
    fn file_revision_changes_without_reading_preview_content() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("notes.md");
        let missing = file_revision(&path).expect("missing revision");
        assert!(!missing.exists);
        assert_eq!(missing.key, "missing");

        std::fs::write(&path, "version 4").expect("write version 4");
        let before = file_revision(&path).expect("before revision");
        std::thread::sleep(std::time::Duration::from_millis(2));
        std::fs::write(&path, "version 5 with more content").expect("write version 5");
        let after = file_revision(&path).expect("after revision");

        assert!(before.exists);
        assert!(after.exists);
        assert_ne!(before.key, after.key);
        assert_ne!(before.size, after.size);
    }
}

#[cfg(test)]
mod openable_file_tests {
    use super::{absolute_existing_file, absolute_file_candidate};

    #[test]
    fn absolute_files_outside_any_workspace_are_openable() {
        let external = tempfile::tempdir().expect("temp dir");
        let path = external.path().join("diagonal_cut.py");
        std::fs::write(&path, "print('cut')\n").expect("write file");
        let display = path.to_string_lossy().replace('\\', "/");

        let resolved = absolute_existing_file(std::path::Path::new(&display), &display)
            .expect("external file resolves");

        assert_eq!(resolved, dunce::canonicalize(&path).unwrap());
    }

    #[test]
    fn absolute_open_rejects_missing_files_and_directories() {
        let external = tempfile::tempdir().expect("temp dir");
        let missing = external.path().join("missing.py");
        let missing_error = absolute_existing_file(&missing, "missing.py").unwrap_err();
        assert_eq!(missing_error.code, "asset.preview.not_found");

        let directory_error = absolute_existing_file(external.path(), "dir").unwrap_err();
        assert_eq!(directory_error.code, "asset.preview.not_file");
    }

    #[test]
    fn absolute_revision_candidate_keeps_missing_file_name() {
        let external = tempfile::tempdir().expect("temp dir");
        let missing = external.path().join("removed.py");

        let candidate = absolute_file_candidate(&missing).expect("candidate");

        assert_eq!(
            candidate,
            dunce::canonicalize(external.path()).unwrap().join("removed.py")
        );
    }
}
