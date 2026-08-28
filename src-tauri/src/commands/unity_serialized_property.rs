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
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

#[tauri::command]
pub async fn unity_serialized_property_read(
    request: UnitySerializedPropertyReadRequest,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnitySerializedPropertyReadResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_serialized_property_read",
    )
    .await?;
    let working_dir = ready.root_text();
    crate::unity_serialized_property::read(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_discover(
    request: UnitySerializedPropertyDiscoverRequest,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnitySerializedPropertyDiscoverResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_serialized_property_discover",
    )
    .await?;
    let working_dir = ready.root_text();
    crate::unity_serialized_property::discover(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_write(
    request: UnitySerializedPropertyWriteRequest,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnitySerializedPropertyWriteResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_serialized_property_write",
    )
    .await?;
    let working_dir = ready.root_text();
    crate::unity_serialized_property::write(&working_dir, request)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unity_serialized_property_apply(
    request: UnitySerializedPropertyApplyRequest,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnitySerializedPropertyApplyResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_serialized_property_apply",
    )
    .await?;
    let working_dir = ready.root_text();
    crate::unity_serialized_property::apply(&working_dir, request)
        .await
        .map_err(Into::into)
}
