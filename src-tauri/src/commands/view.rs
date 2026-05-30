use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::view::{
    append_view_frontend_log_sync, call_view_script, compile_view_script,
    complete_view_automation_request, create_view_folder_sync, create_view_sync_with_scope,
    delete_view_entry_sync, destroy_view_content_window, detach_view_tab_window, emit_view_reload,
    emit_view_tree_changed, ensure_view_host_pool_window, export_view_package_sync,
    hide_view_content_window, import_view_package_sync, list_view_tree_sync, list_views_sync,
    mark_view_host_pool_ready, mark_view_host_revealed, mount_view_content_window,
    move_view_entry_sync, open_view_frontend_log_sync, open_view_unity_embed_window,
    open_view_window, parse_view_create_request, read_view_frontend_log_sync, read_view_sync,
    reload_view_sync, rename_view_entry_sync, supported_view_templates,
    view_binding_apply as view_binding_apply_impl,
    view_binding_discover as view_binding_discover_impl,
    view_binding_read as view_binding_read_impl, view_binding_write as view_binding_write_impl,
    view_storage_get_sync, view_storage_remove_sync, view_storage_set_sync, ViewAutomationStore,
    ViewBindingApplyRequest, ViewBindingApplyResult, ViewBindingDiscoverRequest,
    ViewBindingDiscoverResult, ViewBindingReadRequest, ViewBindingReadResult,
    ViewBindingWriteRequest, ViewBindingWriteResult, ViewCallScriptRequest, ViewCallScriptResult,
    ViewCompileScriptRequest, ViewCompileScriptResult, ViewContentMountRequest,
    ViewCreateFolderRequest, ViewDeleteEntryRequest, ViewDetachTabRequest,
    ViewExportPackageRequest, ViewFolderSummary, ViewFrontendLogEntry, ViewFrontendLogReadRequest,
    ViewFrontendLogRequest, ViewImportPackageRequest, ViewMoveEntryRequest, ViewPackageDetail,
    ViewPackageImportResult, ViewPackageSummary, ViewRenameEntryRequest, ViewRunResult,
    ViewSetTabHostRequest, ViewStorageGetRequest, ViewStorageRemoveRequest, ViewStorageSetRequest,
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
pub async fn view_rename_entry(
    request: ViewRenameEntryRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let snapshot = rename_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
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
pub async fn view_export_package(
    request: ViewExportPackageRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    export_view_package_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_import_package(
    request: ViewImportPackageRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewPackageImportResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let result = import_view_package_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed(&app_handle);
    Ok(result)
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
    config: State<'_, Arc<crate::config::AppConfig>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    open_view_window(
        &app_handle,
        &working_dir,
        &view_id,
        config.view_windows_above_main_enabled(),
        config.view_open_in_existing_window_enabled(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn view_run_in_unity(
    view_id: String,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    open_view_unity_embed_window(&app_handle, &working_dir, &view_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_set_tab_host(request: ViewSetTabHostRequest) -> Result<(), AppError> {
    crate::view::set_view_tab_host_sync(request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_detach_tab(
    request: ViewDetachTabRequest,
    workspace: State<'_, Arc<Workspace>>,
    config: State<'_, Arc<crate::config::AppConfig>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    detach_view_tab_window(
        &app_handle,
        &working_dir,
        request,
        config.view_windows_above_main_enabled(),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn view_host_pool_prepare(
    config: State<'_, Arc<crate::config::AppConfig>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    ensure_view_host_pool_window(&app_handle, config.view_windows_above_main_enabled())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_host_pool_ready(
    host_label: String,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    mark_view_host_pool_ready(&app_handle, &host_label).map_err(Into::into)
}

#[tauri::command]
pub async fn view_host_revealed(
    host_label: String,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    mark_view_host_revealed(&app_handle, &working_dir, &host_label)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_content_mount(
    request: ViewContentMountRequest,
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    mount_view_content_window(&app_handle, &working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_content_hide(view_id: String, app_handle: AppHandle) -> Result<(), AppError> {
    hide_view_content_window(&app_handle, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_content_destroy(view_id: String, app_handle: AppHandle) -> Result<(), AppError> {
    destroy_view_content_window(&app_handle, &view_id).map_err(Into::into)
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
pub async fn view_read_frontend_log(
    request: ViewFrontendLogReadRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<ViewFrontendLogEntry>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    read_view_frontend_log_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_open_frontend_log(
    view_id: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    open_view_frontend_log_sync(&working_dir, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_get(
    request: ViewStorageGetRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Option<serde_json::Value>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_storage_get_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_set(
    request: ViewStorageSetRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_storage_set_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_remove(
    request: ViewStorageRemoveRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<(), AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_storage_remove_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_automation_respond(
    request_id: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
    store: State<'_, Arc<ViewAutomationStore>>,
) -> Result<(), AppError> {
    if complete_view_automation_request(store.inner().as_ref(), request_id, ok, result, error) {
        Ok(())
    } else {
        Err(AppError::from(
            "View automation request is no longer pending",
        ))
    }
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
pub async fn view_binding_discover(
    request: ViewBindingDiscoverRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingDiscoverResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    view_binding_discover_impl(&working_dir, request)
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
