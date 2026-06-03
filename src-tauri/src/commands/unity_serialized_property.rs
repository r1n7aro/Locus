use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::unity_serialized_property::{
    UnitySerializedPropertyApplyRequest, UnitySerializedPropertyDiscoverRequest,
    UnitySerializedPropertyReadRequest, UnitySerializedPropertyWriteRequest,
};
use crate::view::{
    ViewBindingApplyResult, ViewBindingDiscoverResult, ViewBindingReadResult,
    ViewBindingWriteResult,
};
use crate::workspace::Workspace;

#[tauri::command]
pub async fn unity_serialized_property_read(
    request: UnitySerializedPropertyReadRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingReadResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::read(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_discover(
    request: UnitySerializedPropertyDiscoverRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingDiscoverResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::discover(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_write(
    request: UnitySerializedPropertyWriteRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingWriteResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::write(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_apply(
    request: UnitySerializedPropertyApplyRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<ViewBindingApplyResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::apply(&working_dir, request)
        .await
        .map_err(Into::into)
}
