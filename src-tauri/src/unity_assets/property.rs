//! One scoped service for View and SDK logical property operations. Agent's
//! disk tree uses the same semantic model and YamlPropertyTree projection.
use super::*;
use crate::unity_serialized_property::property_tree::YamlPropertyTree;
use crate::view::UnitySerializedPropertyTarget;
use std::collections::BTreeMap;
pub(super) mod advanced;

fn target(request: &Value) -> Result<UnitySerializedPropertyTarget, String> {
    let target: UnitySerializedPropertyTarget =
        serde_json::from_value(request["target"].clone())
            .map_err(|e| format!("property.invalid_target: {e}"))?;
    if target
        .global_object_id
        .as_deref()
        .is_some_and(|v| !v.is_empty())
        || target.object_path.as_deref().is_some_and(|v| !v.is_empty())
        || target
            .component_type
            .as_deref()
            .is_some_and(|v| !v.is_empty())
        || target.component_index.is_some()
    {
        return Err(
            "property.unsupported_target: use persisted path/fileID, not Editor locators".into(),
        );
    }
    if !matches!(
        target.kind.as_str(),
        "asset" | "component" | "gameObject" | "gameobject" | "scene"
    ) {
        return Err("property.unsupported_target: requires a persisted asset".into());
    }
    path(&target)?;
    Ok(target)
}

fn path(target: &UnitySerializedPropertyTarget) -> Result<&str, String> {
    target
        .path
        .as_deref()
        .filter(|path| !path.is_empty())
        .or(target.scene_path.as_deref())
        .filter(|path| path.starts_with("Assets/"))
        .ok_or_else(|| "property.invalid_target: explicit Assets/ path required".into())
}

fn object_id(
    tree: &YamlPropertyTree,
    target: &UnitySerializedPropertyTarget,
) -> Result<i64, String> {
    if let Some(id) = target.target_file_id.or(target.object_file_id) {
        tree.semantic().object(&id.to_string())?;
        return Ok(id);
    }
    let objects = &tree.semantic().snapshot.objects;
    if target.kind == "asset" && objects.len() == 1 {
        return objects[0]
            .object_id
            .parse::<i64>()
            .map_err(|e| e.to_string());
    }
    Err("property.ambiguous_target: targetFileId is required for multi-object assets".into())
}

fn read_result(tree: &YamlPropertyTree, request: &Value) -> Result<Value, String> {
    let mut target = target(request)?;
    let id = object_id(tree, &target)?;
    let property_path = target.property_path.clone().unwrap_or_default();
    let depth = request
        .get("maxDepth")
        .and_then(Value::as_i64)
        .unwrap_or(4)
        .clamp(0, 16) as usize;
    let limit = request
        .get("maxArrayItems")
        .and_then(Value::as_i64)
        .unwrap_or(128)
        .clamp(1, 1024) as usize;
    let offset = request
        .get("arrayOffset")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as usize;
    let snapshot = tree.read_target(id, &property_path, depth, limit, offset)?;
    target.target_file_id = Some(id);
    target.kind = "asset".into();
    target.path = Some(path(&target)?.into());
    target.scene_path = None;
    let mut result = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    result["target"] = serde_json::to_value(target).map_err(|e| e.to_string())?;
    result["ok"] = json!(true);
    result["message"] = json!("");
    result["backend"] = json!("yaml");
    result["revision"] = json!(tree.semantic().snapshot.revision);
    result["diagnostics"] = json!(tree.semantic().snapshot.diagnostics);
    if let Some(id) = request.get("bindingId") {
        result["bindingId"] = id.clone();
    }
    Ok(result)
}

/// One semantic compiler and transaction path, independent of request shape.
pub(super) async fn execute(root: &Path, request: Value) -> Result<Value, String> {
    if request
        .get("backend")
        .and_then(Value::as_str)
        .unwrap_or("yaml")
        != "yaml"
    {
        return Err("property.unsupported_backend: logical disk operations require yaml".into());
    }
    if request
        .get("resultMode")
        .is_some_and(|mode| !matches!(mode.as_str(), Some("full" | "summary")))
    {
        return Err("property.invalid_result_mode".into());
    }
    if request["action"] == "apply_properties"
        && request["writes"].as_array().is_some_and(Vec::is_empty)
    {
        if request["resultMode"] == "summary" {
            return Ok(
                json!({"ok":true,"message":"","writesApplied":0,"assets":[],"transactionId":null}),
            );
        }
        return Ok(json!({"ok":true,"message":"","results":[]}));
    }
    advanced::execute(root, request).await
}
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod prefab_matrix_tests;
