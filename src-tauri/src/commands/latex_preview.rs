//! LaTeX preview commands: preview panes open and close previews, report
//! their visibility (builds run only while visible) and request rebuilds;
//! the settings page and the file preview inspect the toolchain.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::latex_preview::manager::{self, OpenResult, PreviewSnapshot};
use crate::latex_preview::toolchain::{self, Toolchain};
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

fn validate_path(path: &str) -> Result<PathBuf, AppError> {
    let candidate = PathBuf::from(path.trim());
    let is_tex = candidate
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tex"));
    if !candidate.is_absolute() || !is_tex {
        return Err(AppError::new(
            "latex.invalid_path",
            "LaTeX preview needs the absolute path of a .tex file.",
        )
        .detail(path.to_string()));
    }
    dunce::canonicalize(&candidate).map_err(|error| {
        AppError::new("latex.file_missing", "The .tex file does not exist.")
            .detail(format!("{}: {error}", candidate.display()))
    })
}

fn preview_closed(error: String) -> AppError {
    AppError::new("latex.preview_closed", "The LaTeX preview was closed.").detail(error)
}

/// Opens a preview session for `path`. Within a workspace the session is
/// keyed by the checkout and watches stay inside it; files opened from
/// outside a workspace use their own folder.
#[tauri::command]
pub async fn latex_preview_open(
    workspace_ref: Option<WorkspaceRef>,
    path: String,
    visible: bool,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: tauri::AppHandle,
    webview: tauri::Webview,
) -> Result<OpenResult, AppError> {
    let path = validate_path(&path)?;
    let (checkout_id, owner_dir) = match workspace_ref {
        Some(workspace_ref) => {
            let resolved = registry
                .inner()
                .resolve_workspace_ref(&workspace_ref)
                .map_err(super::workspace_service::workspace_resolve_error)?;
            let runtime = resolved.runtime();
            (runtime.checkout_id().to_string(), runtime.root().to_path_buf())
        }
        None => (
            String::new(),
            path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone()),
        ),
    };
    manager::manager()
        .open(&app_handle, webview.label(), &checkout_id, &owner_dir, &path, visible)
        .map_err(|error| {
            AppError::new("latex.preview_open_failed", "Failed to open the LaTeX preview.")
                .detail(error)
        })
}

#[tauri::command]
pub fn latex_preview_set_visible(
    subscription_id: String,
    visible: bool,
) -> Result<PreviewSnapshot, AppError> {
    manager::manager()
        .set_visible(&subscription_id, visible)
        .map_err(preview_closed)
}

#[tauri::command]
pub fn latex_preview_rebuild(subscription_id: String) -> Result<(), AppError> {
    manager::manager()
        .rebuild(&subscription_id)
        .map_err(preview_closed)
}

#[tauri::command]
pub fn latex_preview_close(subscription_id: String) {
    manager::manager().close(&subscription_id);
}

#[tauri::command]
pub async fn latex_toolchain_status(refresh: bool) -> Result<Toolchain, AppError> {
    Ok(toolchain::get(refresh).await)
}
