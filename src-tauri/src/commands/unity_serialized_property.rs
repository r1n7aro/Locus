use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::unity_serialized_property::{
    UnitySerializedPropertyApplyRequest, UnitySerializedPropertyDiscoverRequest,
    UnitySerializedPropertyReadRequest, UnitySerializedPropertyWriteRequest,
};
use crate::view::{
    UnitySerializedPropertyApplyResult, UnitySerializedPropertyDiscoverResult,
    UnitySerializedPropertyReadResult, UnitySerializedPropertyWriteResult,
};
use crate::workspace::Workspace;

#[tauri::command]
pub async fn unity_serialized_property_read(
    request: UnitySerializedPropertyReadRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<UnitySerializedPropertyReadResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::read(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_discover(
    request: UnitySerializedPropertyDiscoverRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<UnitySerializedPropertyDiscoverResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::discover(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_write(
    request: UnitySerializedPropertyWriteRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<UnitySerializedPropertyWriteResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::write(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_apply(
    request: UnitySerializedPropertyApplyRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<UnitySerializedPropertyApplyResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    crate::unity_serialized_property::apply(&working_dir, request)
        .await
        .map_err(Into::into)
}
