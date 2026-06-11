use serde::{Deserialize, Serialize};

use crate::view::{
    UnitySerializedPropertyApplyResult, UnitySerializedPropertyDiscoverResult,
    UnitySerializedPropertyReadResult, UnitySerializedPropertyTarget,
    UnitySerializedPropertyWriteResult,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitySerializedPropertyReadRequest {
    #[serde(default)]
    pub binding_id: Option<String>,
    pub target: UnitySerializedPropertyTarget,
    #[serde(default)]
    pub max_depth: Option<i32>,
    #[serde(default)]
    pub max_array_items: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitySerializedPropertyDiscoverRequest {
    #[serde(default)]
    pub binding_id: Option<String>,
    pub target: UnitySerializedPropertyTarget,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub field_name: Option<String>,
    #[serde(default)]
    pub field_type: Option<String>,
    #[serde(default)]
    pub max_depth: Option<i32>,
    #[serde(default)]
    pub max_results: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitySerializedPropertyWriteRequest {
    #[serde(default)]
    pub binding_id: Option<String>,
    pub target: UnitySerializedPropertyTarget,
    pub value: serde_json::Value,
    #[serde(default)]
    pub write_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitySerializedPropertyApplyWrite {
    #[serde(default)]
    pub binding_id: Option<String>,
    pub target: UnitySerializedPropertyTarget,
    pub value: serde_json::Value,
    #[serde(default)]
    pub write_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitySerializedPropertyApplyRequest {
    pub writes: Vec<UnitySerializedPropertyApplyWrite>,
}

pub async fn read(
    working_dir: &str,
    request: UnitySerializedPropertyReadRequest,
) -> Result<UnitySerializedPropertyReadResult, String> {
    validate_object_target(&request.target)?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "maxDepth": request.max_depth.unwrap_or_default(),
        "maxArrayItems": request.max_array_items.unwrap_or_default(),
    });
    let raw = crate::unity_bridge::view_binding_read(working_dir, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid unity_serialized_property_read response: {}", error))
}

pub async fn discover(
    working_dir: &str,
    request: UnitySerializedPropertyDiscoverRequest,
) -> Result<UnitySerializedPropertyDiscoverResult, String> {
    validate_object_target(&request.target)?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "query": request.query.unwrap_or_default(),
        "fieldName": request.field_name.unwrap_or_default(),
        "fieldType": request.field_type.unwrap_or_default(),
        "maxDepth": request.max_depth.unwrap_or_default(),
        "maxResults": request.max_results.unwrap_or_default(),
    });
    let raw = crate::unity_bridge::view_binding_discover(working_dir, &payload).await?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Invalid unity_serialized_property_discover response: {}",
            error
        )
    })
}

pub async fn write(
    working_dir: &str,
    request: UnitySerializedPropertyWriteRequest,
) -> Result<UnitySerializedPropertyWriteResult, String> {
    validate_property_target(&request.target)?;
    let value_json = serde_json::to_string(&request.value)
        .map_err(|error| format!("Failed to serialize serialized property value: {}", error))?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "valueJson": value_json,
        "mode": normalize_write_mode(request.write_mode.as_deref())?,
    });
    let raw = crate::unity_bridge::view_binding_write(working_dir, &payload).await?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Invalid unity_serialized_property_write response: {}",
            error
        )
    })
}

pub async fn apply(
    working_dir: &str,
    request: UnitySerializedPropertyApplyRequest,
) -> Result<UnitySerializedPropertyApplyResult, String> {
    for write in &request.writes {
        validate_property_target(&write.target)?;
    }
    let mut writes = Vec::with_capacity(request.writes.len());
    for write in request.writes {
        let value_json = serde_json::to_string(&write.value)
            .map_err(|error| format!("Failed to serialize serialized property value: {}", error))?;
        writes.push(serde_json::json!({
            "bindingId": write.binding_id,
            "target": write.target,
            "valueJson": value_json,
            "mode": normalize_write_mode(write.write_mode.as_deref())?,
        }));
    }
    let payload = serde_json::json!({ "writes": writes });
    let raw = crate::unity_bridge::view_binding_apply(working_dir, &payload).await?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Invalid unity_serialized_property_apply response: {}",
            error
        )
    })
}

fn normalize_write_mode(mode: Option<&str>) -> Result<&'static str, String> {
    match mode
        .unwrap_or("commit")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "commit" => Ok("commit"),
        "preview" => Ok("preview"),
        other => Err(format!(
            "Unsupported Unity serialized property write mode: {}",
            other
        )),
    }
}

fn validate_property_target(target: &UnitySerializedPropertyTarget) -> Result<(), String> {
    validate_object_target(target)?;
    if target
        .property_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err("Unity serialized property target propertyPath is required.".to_string());
    }
    Ok(())
}

fn validate_object_target(target: &UnitySerializedPropertyTarget) -> Result<(), String> {
    let kind = target.kind.trim();
    if kind.is_empty() {
        return Err("Unity serialized property target kind cannot be empty.".to_string());
    }
    // Asset targets are meaningless without a locator; reject early instead of
    // round-tripping to the Unity bridge. Other kinds (e.g. selection-relative
    // component targets) may legitimately omit every path field.
    if kind.eq_ignore_ascii_case("asset") {
        let has_locator = [target.guid.as_deref(), target.path.as_deref()]
            .into_iter()
            .flatten()
            .any(|value| !value.trim().is_empty());
        if !has_locator {
            return Err(
                "Unity serialized property asset target requires a path or guid.".to_string(),
            );
        }
    }
    if matches!(target.component_index, Some(index) if index < 0) {
        return Err(
            "Unity serialized property target componentIndex cannot be negative.".to_string(),
        );
    }
    for path in [
        target.guid.as_deref(),
        target.path.as_deref(),
        target.scene_path.as_deref(),
        target.object_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if path.contains('\0') {
            return Err(
                "Unity serialized property target path contains an invalid character.".to_string(),
            );
        }
    }
    Ok(())
}
