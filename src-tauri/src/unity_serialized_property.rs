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
    let schema = load_schema_for_target(working_dir, &request.target).await;
    let schema_mode = if schema.is_some() { "dynamic" } else { "full" };
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "maxDepth": request.max_depth.unwrap_or_default(),
        "maxArrayItems": request.max_array_items.unwrap_or_default(),
        "schemaMode": schema_mode,
    });
    let raw = crate::unity_bridge::view_binding_read(working_dir, &payload).await?;
    let mut result: UnitySerializedPropertyReadResult = serde_json::from_str(&raw)
        .map_err(|error| format!("Invalid unity_serialized_property_read response: {}", error))?;
    if let Some(schema) = schema {
        schema.enrich_read_result(&mut result);
    }
    Ok(result)
}

pub async fn discover(
    working_dir: &str,
    request: UnitySerializedPropertyDiscoverRequest,
) -> Result<UnitySerializedPropertyDiscoverResult, String> {
    validate_object_target(&request.target)?;
    let schema = load_schema_for_target(working_dir, &request.target).await;
    let filters = crate::unity_serialized_schema::DiscoverFilters {
        query: request.query.clone(),
        field_name: request.field_name.clone(),
        field_type: request.field_type.clone(),
        max_results: request.max_results,
    };
    let include_all = schema.is_some() && !discover_has_filters(&filters);
    let schema_mode = if include_all { "dynamic" } else { "full" };
    let unity_max_results = if include_all {
        request.max_results.unwrap_or_default().max(5000)
    } else {
        request.max_results.unwrap_or_default()
    };
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "query": if include_all { String::new() } else { request.query.unwrap_or_default() },
        "fieldName": if include_all { String::new() } else { request.field_name.unwrap_or_default() },
        "fieldType": if include_all { String::new() } else { request.field_type.unwrap_or_default() },
        "maxDepth": request.max_depth.unwrap_or_default(),
        "maxResults": unity_max_results,
        "includeAll": include_all,
        "schemaMode": schema_mode,
    });
    let raw = crate::unity_bridge::view_binding_discover(working_dir, &payload).await?;
    let mut result: UnitySerializedPropertyDiscoverResult =
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "Invalid unity_serialized_property_discover response: {}",
                error
            )
        })?;
    if include_all {
        if let Some(schema) = schema {
            schema.enrich_discover_result(&mut result, &filters);
        }
    }
    Ok(result)
}

pub async fn write(
    working_dir: &str,
    request: UnitySerializedPropertyWriteRequest,
) -> Result<UnitySerializedPropertyWriteResult, String> {
    validate_property_target(&request.target)?;
    let schema = load_schema_for_target(working_dir, &request.target).await;
    let schema_mode = if schema.is_some() { "dynamic" } else { "full" };
    let value_json = serde_json::to_string(&request.value)
        .map_err(|error| format!("Failed to serialize serialized property value: {}", error))?;
    let payload = serde_json::json!({
        "bindingId": request.binding_id,
        "target": request.target,
        "valueJson": value_json,
        "mode": normalize_write_mode(request.write_mode.as_deref())?,
        "schemaMode": schema_mode,
    });
    let raw = crate::unity_bridge::view_binding_write(working_dir, &payload).await?;
    let mut result: UnitySerializedPropertyWriteResult =
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "Invalid unity_serialized_property_write response: {}",
                error
            )
        })?;
    if let Some(schema) = schema {
        if schema.can_enrich_target(&result.read.target) {
            schema.enrich_read_result(&mut result.read);
        }
    }
    Ok(result)
}

pub async fn apply(
    working_dir: &str,
    request: UnitySerializedPropertyApplyRequest,
) -> Result<UnitySerializedPropertyApplyResult, String> {
    for write in &request.writes {
        validate_property_target(&write.target)?;
    }
    let schema = if request.writes.iter().any(|write| {
        !crate::unity_serialized_schema::target_is_static_schema_excluded(&write.target)
    }) {
        crate::unity_serialized_schema::try_load_current_schema(working_dir).await
    } else {
        None
    };
    let mut writes = Vec::with_capacity(request.writes.len());
    for write in request.writes {
        let value_json = serde_json::to_string(&write.value)
            .map_err(|error| format!("Failed to serialize serialized property value: {}", error))?;
        let schema_mode = if schema
            .as_ref()
            .map(|schema| schema.can_enrich_target(&write.target))
            .unwrap_or(false)
        {
            "dynamic"
        } else {
            "full"
        };
        writes.push(serde_json::json!({
            "bindingId": write.binding_id,
            "target": write.target,
            "valueJson": value_json,
            "mode": normalize_write_mode(write.write_mode.as_deref())?,
            "schemaMode": schema_mode,
        }));
    }
    let payload = serde_json::json!({ "writes": writes });
    let raw = crate::unity_bridge::view_binding_apply(working_dir, &payload).await?;
    let mut result: UnitySerializedPropertyApplyResult =
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "Invalid unity_serialized_property_apply response: {}",
                error
            )
        })?;
    if let Some(schema) = schema {
        for item in &mut result.results {
            if schema.can_enrich_target(&item.read.target) {
                schema.enrich_read_result(&mut item.read);
            }
        }
    }
    Ok(result)
}

async fn load_schema_for_target(
    working_dir: &str,
    target: &UnitySerializedPropertyTarget,
) -> Option<std::sync::Arc<crate::unity_serialized_schema::SerializedSchemaIndex>> {
    if crate::unity_serialized_schema::target_is_static_schema_excluded(target) {
        return None;
    }
    let schema = crate::unity_serialized_schema::try_load_current_schema(working_dir).await;
    schema.filter(|schema| schema.can_enrich_target(target))
}

fn discover_has_filters(filters: &crate::unity_serialized_schema::DiscoverFilters) -> bool {
    filters
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        || filters
            .field_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        || filters
            .field_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
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
