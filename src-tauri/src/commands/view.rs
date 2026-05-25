use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::view::{
    append_view_frontend_log_sync, call_view_script, compile_view_script, create_view_folder_sync,
    create_view_sync_with_scope, delete_view_entry_sync, emit_view_reload, emit_view_tree_changed,
    list_view_tree_sync, list_views_sync, move_view_entry_sync, open_view_window,
    parse_view_create_request, read_view_sync, reload_view_sync, supported_view_templates,
    view_binding_apply as view_binding_apply_impl, view_binding_read as view_binding_read_impl,
    view_binding_write as view_binding_write_impl, ViewBindingApplyRequest, ViewBindingApplyResult,
    ViewBindingReadRequest, ViewBindingReadResult, ViewBindingWriteRequest, ViewBindingWriteResult,
    ViewCallScriptRequest, ViewCallScriptResult, ViewCompileScriptRequest, ViewCompileScriptResult,
    ViewCreateFolderRequest, ViewDeleteEntryRequest, ViewFolderSummary, ViewFrontendLogRequest,
    ViewMoveEntryRequest, ViewPackageDetail, ViewPackageSummary, ViewRunResult,
    ViewTemplateSummary, ViewTreeSnapshot,
};
use crate::workspace::Workspace;

#[tauri::command]
pub async fn view_templates() -> Result<Vec<ViewTemplateSummary>, AppError> {
    Ok(supported_view_templates())
}

#[tauri::command]
pub async fn view_list(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<ViewPackageSummary>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    list_views_sync(&working_dir).map_err(Into::into)
}

#[tauri::command]
pub async fn view_tree(workspace: State<'_, Arc<Workspace>>) -> Result<ViewTreeSnapshot, AppError> {
    let working_dir = workspace.path.read().await.clone();
    list_view_tree_sync(&working_dir).map_err(Into::into)
}

#[tauri::command]
pub async fn view_create(
    request: serde_json::Value,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewPackageDetail, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let (request, temporary) = parse_view_create_request(request).map_err(AppError::from)?;
    let detail =
        create_view_sync_with_scope(&working_dir, request, temporary).map_err(AppError::from)?;
    emit_view_reload(&app_handle, &detail.summary);
    Ok(detail)
}

#[tauri::command]
pub async fn view_create_folder(
    request: ViewCreateFolderRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewFolderSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let folder = create_view_folder_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed(&app_handle);
    Ok(folder)
}

#[tauri::command]
pub async fn view_delete_entry(
    request: ViewDeleteEntryRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let snapshot = delete_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed(&app_handle);
    Ok(snapshot)
}

#[tauri::command]
pub async fn view_move_entry(
    request: ViewMoveEntryRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let snapshot = move_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed(&app_handle);
    Ok(snapshot)
}

#[tauri::command]
pub async fn view_read(
    view_id: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewPackageDetail, AppError> {
    let working_dir = workspace.path.read().await.clone();
    read_view_sync(&working_dir, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_reload(
    view_id: String,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewPackageSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let summary = reload_view_sync(&working_dir, &view_id).map_err(AppError::from)?;
    emit_view_reload(&app_handle, &summary);
    Ok(summary)
}

#[tauri::command]
pub async fn view_run(
    view_id: String,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    open_view_window(&app_handle, &working_dir, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_compile_script(
    request: ViewCompileScriptRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewCompileScriptResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    compile_view_script(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_call_script(
    request: ViewCallScriptRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewCallScriptResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    call_view_script(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_append_frontend_log(
    request: ViewFrontendLogRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    append_view_frontend_log_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_binding_read(
    request: ViewBindingReadRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingReadResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_binding_read_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_binding_write(
    request: ViewBindingWriteRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingWriteResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_binding_write_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_binding_apply(
    request: ViewBindingApplyRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingApplyResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_binding_apply_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}
