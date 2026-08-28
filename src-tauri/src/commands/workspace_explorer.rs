use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::AppError;
use crate::session::models::{
    ProjectExplorerMutationResult, ProjectExplorerOperation, ProjectExplorerPresetSummary,
    ProjectExplorerSnapshot,
};
use crate::workspace_service::{ProjectId, ProjectRegistry};

pub const PROJECT_EXPLORER_CHANGED_EVENT: &str = "project-explorer-changed";

const MAX_MOUNT_ENTRIES: usize = 5_000;
const MAX_TEXT_PREVIEW_BYTES: u64 = 1024 * 1024;
const MAX_MEDIA_PREVIEW_BYTES: u64 = 50 * 1024 * 1024;

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
    pub data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_relative_path: Option<String>,
}

fn resolve_project(
    registry: &ProjectRegistry,
    project_id: String,
    operation: &'static str,
) -> Result<(ProjectId, PathBuf), AppError> {
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
    roots.sort_by(|left, right| {
        left.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&right.to_string_lossy().to_ascii_lowercase())
    });
    let has_manifest = |root: &Path| {
        root.join("Locus")
            .join("workspace-trees")
            .join("index.json")
            .is_file()
    };
    let root = roots
        .iter()
        .find(|root| has_manifest(root) && root.join(".git").is_dir())
        .or_else(|| roots.iter().find(|root| has_manifest(root)))
        .or_else(|| roots.iter().find(|root| root.join(".git").is_dir()))
        .or_else(|| roots.first())
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                "workspace.project_root_unavailable",
                "The project does not have an available working directory.",
            )
            .detail(project_id.to_string())
            .operation(operation)
        })?;
    Ok((project_id, root))
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

#[tauri::command]
pub async fn project_explorer_snapshot(
    project_id: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerSnapshot, AppError> {
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerSnapshot",
    )?;
    crate::workspace_tree::snapshot(&root, project_id.as_str())
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerApplyOperations",
    )?;
    let result = crate::workspace_tree::apply_operations(
        &root,
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerListPresets",
    )?;
    crate::workspace_tree::list_presets(&root, project_id.as_str())
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerCreatePreset",
    )?;
    let snapshot = crate::workspace_tree::create_preset(
        &root,
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerSwitchPreset",
    )?;
    let snapshot = crate::workspace_tree::switch_preset(&root, project_id.as_str(), &preset_id)
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerRenamePreset",
    )?;
    let snapshot =
        crate::workspace_tree::rename_preset(&root, project_id.as_str(), &preset_id, &name)
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerDeletePreset",
    )?;
    let snapshot = crate::workspace_tree::delete_preset(&root, project_id.as_str(), &preset_id)
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
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerListMount",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&root, project_id.as_str())
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
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

fn is_text_extension(extension: &str) -> bool {
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
            | "tsv"
            | "asmdef"
            | "asmref"
    )
}

fn unity_workspace_binding(
    registry: &ProjectRegistry,
    project_id: &ProjectId,
    canonical: &Path,
) -> Option<(String, u64, String)> {
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
                relative,
            ));
        }
    }
    None
}

#[tauri::command]
pub async fn project_explorer_preview_file(
    project_id: String,
    path: String,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<ProjectExplorerFilePreview, AppError> {
    let (project_id, root) = resolve_project(
        registry.inner().as_ref(),
        project_id,
        "projectExplorerPreviewFile",
    )?;
    let snapshot = crate::workspace_tree::snapshot(&root, project_id.as_str())
        .map_err(|error| explorer_error(error, "projectExplorerPreviewFile"))?;
    let canonical = ensure_preview_path_authorized(&snapshot, Path::new(&path))?;
    let metadata = std::fs::metadata(&canonical).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_metadata_failed",
            "The selected file could not be inspected.",
        )
        .detail(error.to_string())
    })?;
    let size = metadata.len();
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
    let extension = canonical
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if let Some((checkout_id, workspace_generation, relative_path)) =
        unity_workspace_binding(registry.inner().as_ref(), &project_id, &canonical)
    {
        return Ok(ProjectExplorerFilePreview {
            path: canonical.to_string_lossy().into_owned(),
            name,
            extension,
            size,
            kind: "unity".to_string(),
            mime_type: "application/x-unity-asset".to_string(),
            text: None,
            data_url: None,
            total_lines: None,
            truncated: false,
            checkout_id: Some(checkout_id),
            workspace_generation: Some(workspace_generation),
            workspace_relative_path: Some(relative_path),
        });
    }

    let mime_type = mime_for_extension(&extension).to_string();
    let media_kind = if mime_type.starts_with("image/") {
        Some("image")
    } else if mime_type == "application/pdf" {
        Some("pdf")
    } else if mime_type.starts_with("audio/") {
        Some("audio")
    } else if mime_type.starts_with("video/") {
        Some("video")
    } else {
        None
    };
    if let Some(kind) = media_kind {
        if size <= MAX_MEDIA_PREVIEW_BYTES {
            let bytes = std::fs::read(&canonical).map_err(|error| {
                AppError::new(
                    "workspace.explorer_file_read_failed",
                    "The selected file could not be read.",
                )
                .detail(error.to_string())
            })?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            return Ok(ProjectExplorerFilePreview {
                path: canonical.to_string_lossy().into_owned(),
                name,
                extension,
                size,
                kind: kind.to_string(),
                mime_type: mime_type.clone(),
                text: None,
                data_url: Some(format!("data:{mime_type};base64,{encoded}")),
                total_lines: None,
                truncated: false,
                checkout_id: None,
                workspace_generation: None,
                workspace_relative_path: None,
            });
        }
    }

    let mut file = std::fs::File::open(&canonical).map_err(|error| {
        AppError::new(
            "workspace.explorer_file_read_failed",
            "The selected file could not be read.",
        )
        .detail(error.to_string())
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_TEXT_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::new(
                "workspace.explorer_file_read_failed",
                "The selected file could not be read.",
            )
            .detail(error.to_string())
        })?;
    let truncated = bytes.len() as u64 > MAX_TEXT_PREVIEW_BYTES;
    if truncated {
        bytes.truncate(MAX_TEXT_PREVIEW_BYTES as usize);
    }
    let looks_text = is_text_extension(&extension) || !bytes.contains(&0);
    if looks_text {
        if let Ok(text) = String::from_utf8(bytes) {
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
                data_url: None,
                total_lines: Some(total_lines),
                truncated,
                checkout_id: None,
                workspace_generation: None,
                workspace_relative_path: None,
            });
        }
    }
    Ok(ProjectExplorerFilePreview {
        path: canonical.to_string_lossy().into_owned(),
        name,
        extension,
        size,
        kind: "binary".to_string(),
        mime_type,
        text: None,
        data_url: None,
        total_lines: None,
        truncated: size > MAX_MEDIA_PREVIEW_BYTES,
        checkout_id: None,
        workspace_generation: None,
        workspace_relative_path: None,
    })
}
