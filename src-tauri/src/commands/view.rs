use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::error::AppError;
use crate::view::{
    append_view_frontend_log_sync, call_view_script, compile_view_script,
    complete_view_automation_request, create_view_folder_sync, create_view_sync_with_scope,
    delete_view_entry_sync, emit_view_reload_for_scope,
    emit_view_tree_changed_for_scope, export_view_package_sync, import_view_package_sync, list_view_tree_sync, list_views_sync, move_view_entry_sync, open_view_frontend_log_sync,
    open_view_unity_embed_window, parse_view_create_request, read_view_frontend_log_sync,
    read_view_sync, reload_view_sync, rename_view_entry_sync, set_view_tab_host_scoped_sync,
    view_fs_access as view_fs_access_impl,
    view_fs_append_file as view_fs_append_file_impl, view_fs_copy_file as view_fs_copy_file_impl,
    view_fs_lstat as view_fs_lstat_impl, view_fs_mkdir as view_fs_mkdir_impl,
    view_fs_read_file as view_fs_read_file_impl, view_fs_readdir as view_fs_readdir_impl,
    view_fs_rename as view_fs_rename_impl, view_fs_rm as view_fs_rm_impl,
    view_fs_stat as view_fs_stat_impl, view_fs_unlink as view_fs_unlink_impl,
    view_fs_write_file as view_fs_write_file_impl, view_storage_get_sync, view_storage_remove_sync,
    view_storage_set_sync, ViewAutomationStore, ViewCallScriptRequest, ViewCallScriptResult,
    ViewCompileScriptRequest, ViewCompileScriptResult, ViewCreateFolderRequest, ViewDeleteEntryRequest, ViewExportPackageRequest, ViewFolderSummary,
    ViewFrontendLogEntry, ViewFrontendLogReadRequest, ViewFrontendLogRequest,
    ViewFsCopyFileRequest, ViewFsMkdirRequest, ViewFsPathRequest, ViewFsReadFileRequest,
    ViewFsReadFileResult, ViewFsReaddirRequest, ViewFsReaddirResult, ViewFsRenameRequest,
    ViewFsRmRequest, ViewFsStatResult, ViewFsWriteFileRequest, ViewImportPackageRequest,
    ViewMoveEntryRequest, ViewPackageDetail, ViewPackageImportResult, ViewPackageSummary,
    ViewRenameEntryRequest, ViewRunResult, ViewSetTabHostRequest, ViewStorageGetRequest,
    ViewStorageRemoveRequest, ViewStorageSetRequest, ViewTreeSnapshot,
};
use crate::workspace_service::{
    ProjectRegistry as InnerProjectRegistry, ResolvedWorkspaceScope, WorkspaceRef,
    WorkspaceResolveError,
};

type ProjectRegistry = Arc<InnerProjectRegistry>;

fn view_workspace_resolve_error(error: WorkspaceResolveError) -> AppError {
    match error {
        error @ WorkspaceResolveError::StaleMaterialization { .. } => AppError::new(
            "workspace.materialization_stale",
            "The checkout assignment changed. Reopen the checkout before continuing.",
        ).detail(error.to_string()),
        WorkspaceResolveError::RegistryUnavailable { detail } => AppError::new(
            "workspace.registry_unavailable",
            "The workspace registry is unavailable.",
        )
        .detail(detail),
        WorkspaceResolveError::CheckoutUnavailable { checkout_id } => AppError::new(
            "workspace.checkout_unavailable",
            "The requested checkout is unavailable.",
        )
        .detail(checkout_id.to_string()),
        WorkspaceResolveError::StaleGeneration {
            checkout_id,
            expected_generation,
            actual_generation,
        } => AppError::new(
            "workspace.generation_stale",
            "The workspace runtime changed before the View request was handled.",
        )
        .detail(format!(
            "checkout={checkout_id}, expected={expected_generation}, actual={actual_generation}"
        )),
    }
}

fn resolve_view_workspace_scope(
    registry: &InnerProjectRegistry,
    workspace_ref: &WorkspaceRef,
) -> Result<ResolvedWorkspaceScope, AppError> {
    registry
        .resolve_workspace_ref(workspace_ref)
        .map_err(view_workspace_resolve_error)
}

fn view_scope_root(scope: &ResolvedWorkspaceScope) -> String {
    scope.runtime().root().to_string_lossy().to_string()
}

#[tauri::command]
pub async fn view_list(
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<Vec<ViewPackageSummary>, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    list_views_sync(&working_dir).map_err(Into::into)
}

#[tauri::command]
pub async fn view_tree(
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewTreeSnapshot, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    list_view_tree_sync(&working_dir).map_err(Into::into)
}

#[tauri::command]
pub async fn view_create(
    request: serde_json::Value,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewPackageDetail, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let (request, temporary) = parse_view_create_request(request).map_err(AppError::from)?;
    let detail =
        create_view_sync_with_scope(&working_dir, request, temporary).map_err(AppError::from)?;
    emit_view_reload_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
        &detail.summary,
    );
    Ok(detail)
}

#[tauri::command]
pub async fn view_create_folder(
    request: ViewCreateFolderRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewFolderSummary, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let folder = create_view_folder_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
    );
    Ok(folder)
}

#[tauri::command]
pub async fn view_delete_entry(
    request: ViewDeleteEntryRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let snapshot = delete_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn view_rename_entry(
    request: ViewRenameEntryRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let snapshot = rename_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn view_move_entry(
    request: ViewMoveEntryRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewTreeSnapshot, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let snapshot = move_view_entry_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn view_export_package(
    request: ViewExportPackageRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<String, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    export_view_package_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_import_package(
    request: ViewImportPackageRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewPackageImportResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let result = import_view_package_sync(&working_dir, request).map_err(AppError::from)?;
    emit_view_tree_changed_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
    );
    Ok(result)
}

#[tauri::command]
pub async fn view_read(
    view_id: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewPackageDetail, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    read_view_sync(&working_dir, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_reload(
    view_id: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewPackageSummary, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    let summary = reload_view_sync(&working_dir, &view_id).map_err(AppError::from)?;
    emit_view_reload_for_scope(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
        &summary,
    );
    Ok(summary)
}

#[tauri::command]
pub async fn view_run(
    view_id: String,
    window_label: Option<String>,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<ViewRunResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    if let Some(label) = window_label.as_deref() {
        if (label != "main" && !label.starts_with("workbench-")) || app_handle.get_webview_window(label).is_none() {
            return Err(AppError::from(format!("Workbench window is unavailable: {label}")));
        }
    }
    crate::view::open_view_in_workbench_on_window(&app_handle, &working_dir, &view_id, Some(window_label.as_deref().unwrap_or(window.label())))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_run_in_unity(
    view_id: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
    app_handle: AppHandle,
) -> Result<ViewRunResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    open_view_unity_embed_window(&app_handle, &working_dir, &view_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_set_tab_host(
    request: ViewSetTabHostRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let _scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let scope_key = format!(
        "{}@{}",
        workspace_ref.checkout_id,
        workspace_ref.expected_generation.unwrap_or_default()
    );
    set_view_tab_host_scoped_sync(request, &scope_key).map_err(Into::into)
}

#[tauri::command]
pub async fn view_compile_script(
    request: ViewCompileScriptRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewCompileScriptResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    compile_view_script(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_call_script(
    request: ViewCallScriptRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewCallScriptResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    call_view_script(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_append_frontend_log(
    request: ViewFrontendLogRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    append_view_frontend_log_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_read_frontend_log(
    request: ViewFrontendLogReadRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<Vec<ViewFrontendLogEntry>, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    read_view_frontend_log_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_open_frontend_log(
    view_id: String,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    open_view_frontend_log_sync(&working_dir, &view_id).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_get(
    request: ViewStorageGetRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<Option<serde_json::Value>, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_storage_get_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_set(
    request: ViewStorageSetRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_storage_set_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_storage_remove(
    request: ViewStorageRemoveRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_storage_remove_sync(&working_dir, request).map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_read_file(
    request: ViewFsReadFileRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewFsReadFileResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_read_file_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_write_file(
    request: ViewFsWriteFileRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_write_file_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_append_file(
    request: ViewFsWriteFileRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_append_file_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_mkdir(
    request: ViewFsMkdirRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_mkdir_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_readdir(
    request: ViewFsReaddirRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewFsReaddirResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_readdir_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_stat(
    request: ViewFsPathRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewFsStatResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_stat_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_lstat(
    request: ViewFsPathRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<ViewFsStatResult, AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_lstat_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_access(
    request: ViewFsPathRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_access_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_unlink(
    request: ViewFsPathRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_unlink_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_rm(
    request: ViewFsRmRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_rm_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_rename(
    request: ViewFsRenameRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_rename_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_fs_copy_file(
    request: ViewFsCopyFileRequest,
    workspace_ref: WorkspaceRef,
    registry: State<'_, ProjectRegistry>,
) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    let working_dir = view_scope_root(&scope);
    view_fs_copy_file_impl(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn view_automation_respond(
    request_id: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
    store: State<'_, Arc<ViewAutomationStore>>,
) -> Result<bool, AppError> {
    // A late or duplicate reply is an expired transport message, not a UI error.
    Ok(complete_view_automation_request(store.inner().as_ref(), request_id, ok, result, error))
}

#[tauri::command]
pub async fn view_watch(workspace_ref: WorkspaceRef, view_id: String, token: String, host_label: String, registry: State<'_, ProjectRegistry>, app_handle: AppHandle) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    crate::view::register_native_view_host(&app_handle, &view_scope_root(&scope), &workspace_ref, &view_id, &token, &host_label).map_err(Into::into)
}

#[tauri::command]
pub async fn view_unwatch(workspace_ref: WorkspaceRef, view_id: String, token: String, host_label: String) -> Result<(), AppError> {
    // Release a lease even if its checkout has already been retired.
    crate::view::release_native_view_host(&workspace_ref, &view_id, &token, &host_label);
    Ok(())
}

#[tauri::command]
pub async fn view_append_frontend_logs(workspace_ref: WorkspaceRef, requests: Vec<ViewFrontendLogRequest>, registry: State<'_, ProjectRegistry>) -> Result<(), AppError> {
    let scope = resolve_view_workspace_scope(&registry, &workspace_ref)?;
    crate::view::append_view_frontend_logs_sync(&view_scope_root(&scope), requests).map_err(Into::into)
}

#[tauri::command]
pub async fn frontend_capture(window_label: String, clip: Option<serde_json::Value>, app_handle: AppHandle) -> Result<serde_json::Value, AppError> {
    crate::view::capture_frontend_window(&app_handle, &window_label, clip).await.map_err(Into::into)
}
