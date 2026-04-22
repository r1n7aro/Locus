use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::asset_db::AssetDbState;
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::workspace::Workspace;
use crate::UndoManagerHandle;

pub use crate::diff::compute_hunks;
pub use crate::diff::types::*;

#[tauri::command]
pub async fn diff_single_file(
    app_handle: AppHandle,
    config: State<'_, Arc<AppConfig>>,
    workspace: State<'_, Arc<Workspace>>,
    undo_mgr: State<'_, UndoManagerHandle>,
    ref_graph_state: State<'_, AssetDbState>,
    binary_cache: State<'_, Arc<crate::binary_cache::BinaryCache>>,
    request: FileDiffRequest,
) -> AppResult<FileDiffPayload> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "diff.no_workspace",
            "No working directory set",
        ));
    }

    crate::diff::service::build_file_diff_payload(
        &app_handle,
        &cwd,
        &request,
        &undo_mgr,
        &ref_graph_state,
        &binary_cache,
        config.debug_enabled(),
    )
    .await
}

#[tauri::command]
pub async fn diff_semantic_target(
    request: SemanticTargetRequest,
) -> AppResult<SemanticTargetInspector> {
    let session =
        crate::diff::service::get_semantic_session(&request.diff_key).ok_or_else(|| {
            AppError::new(
                "diff.semantic_missing",
                "Semantic diff session was not found. Reload the file diff and try again.",
            )
        })?;

    let inspector = if request.include_unchanged {
        session.full_inspectors.get(&request.target_id)
    } else {
        session.changed_inspectors.get(&request.target_id)
    }
    .cloned()
    .ok_or_else(|| {
        let detail = crate::diff::service::semantic_target_lookup_detail(
            &session,
            &request.target_id,
            request.include_unchanged,
        );
        eprintln!(
            "[diff] semantic target lookup failed: diff_key='{}', target_id='{}', {}",
            request.diff_key, request.target_id, detail
        );
        AppError::new(
            "diff.semantic_target_missing",
            format!("Semantic target '{}' was not found", request.target_id),
        )
        .detail(detail)
    })?;

    Ok(inspector)
}

#[tauri::command]
pub async fn diff_text_for_large(
    workspace: State<'_, Arc<Workspace>>,
    undo_mgr: State<'_, UndoManagerHandle>,
    request: FileDiffRequest,
) -> AppResult<TextDiffResult> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "diff.no_workspace",
            "No working directory set",
        ));
    }
    crate::diff::service::compute_text_diff_on_demand(&cwd, &request, &undo_mgr).await
}

#[tauri::command]
pub async fn diff_strings(
    old_text: String,
    new_text: String,
    context_lines: Option<usize>,
) -> AppResult<Vec<DiffHunk>> {
    Ok(compute_hunks(
        &old_text,
        &new_text,
        context_lines.unwrap_or(3),
    ))
}
