//! Explicit native layouts used by structural authoring. Missing links must not
//! evade validation simply because a template omitted them.
use crate::unity_asset_core::{self as core, semantic::SemanticAsset};
use serde_json::Value;

pub(super) fn validate_templates(model: &SemanticAsset, added: &[String]) -> Result<(), String> {
    for object in &model.snapshot.objects {
        let data = model.resolve(&object.object_id, "", false)?.value.unwrap();
        if added.contains(&object.object_id) {
            let mut required = vec![
                "m_ObjectHideFlags",
                "m_CorrespondingSourceObject",
                "m_PrefabInstance",
                "m_PrefabAsset",
            ];
            match object.root_type.as_str() {
                "GameObject" => {
                    if data["serializedVersion"] != 6 {
                        return Err("property.template_gameobject_version_6_required".into());
                    }
                    required.extend([
                        "m_Component",
                        "m_Layer",
                        "m_Name",
                        "m_TagString",
                        "m_Icon",
                        "m_NavMeshLayer",
                        "m_StaticEditorFlags",
                        "m_IsActive",
                    ]);
                }
                "Transform" | "RectTransform" => {
                    if data["serializedVersion"] != 2 {
                        return Err("property.template_transform_version_2_required".into());
                    }
                    required.extend([
                        "m_GameObject",
                        "m_LocalRotation",
                        "m_LocalPosition",
                        "m_LocalScale",
                        "m_Children",
                        "m_Father",
                    ]);
                    if object.root_type == "RectTransform" {
                        required.extend([
                            "m_AnchorMin",
                            "m_AnchorMax",
                            "m_AnchoredPosition",
                            "m_SizeDelta",
                            "m_Pivot",
                        ]);
                    }
                }
                "MonoBehaviour" => required.extend([
                    "m_GameObject",
                    "m_Enabled",
                    "m_EditorHideFlags",
                    "m_Script",
                    "m_Name",
                    "m_EditorClassIdentifier",
                ]),
                _ => return Err("property.unsupported_object_template_class".into()),
            }
            for key in required {
                if data.get(key).is_none() {
                    return Err(format!(
                        "property.explicit_data_required: {}.{key}",
                        object.root_type
                    ));
                }
            }
        }
        if object.root_type == "GameObject" {
            let components = data["m_Component"]
                .as_array()
                .ok_or("property.gameobject_components_required")?;
            let mut transforms = 0;
            for item in components {
                let id = core::authoring::decimal(&item["component"]["fileID"])
                    .ok_or("property.invalid_component_link")?;
                let child = model.object(&id)?;
                if matches!(child.root_type.as_str(), "GameObject" | "PrefabInstance") {
                    return Err("property.component_type_required".into());
                }
                let child_data = model.resolve(&id, "", false)?.value.unwrap();
                if core::authoring::decimal(&child_data["m_GameObject"]["fileID"]).as_deref()
                    != Some(&object.object_id)
                {
                    return Err("property.component_owner_required".into());
                }
                if matches!(child.root_type.as_str(), "Transform" | "RectTransform") {
                    transforms += 1;
                }
            }
            if transforms != 1 {
                return Err("property.gameobject_requires_one_transform".into());
            }
        }
        if matches!(object.root_type.as_str(), "Transform" | "RectTransform") {
            let owner = core::authoring::decimal(&data["m_GameObject"]["fileID"])
                .ok_or("property.transform_owner_required")?;
            if model.object(&owner)?.root_type != "GameObject" {
                return Err("property.transform_gameobject_required".into());
            }
            let father = core::authoring::decimal(&data["m_Father"]["fileID"])
                .ok_or("property.transform_parent_required")?;
            if father != "0"
                && !matches!(
                    model.object(&father)?.root_type.as_str(),
                    "Transform" | "RectTransform"
                )
            {
                return Err("property.transform_parent_type".into());
            }
            for child in data["m_Children"]
                .as_array()
                .ok_or("property.transform_children_required")?
            {
                let id = core::authoring::decimal(&child["fileID"])
                    .ok_or("property.invalid_child_link")?;
                if !matches!(
                    model.object(&id)?.root_type.as_str(),
                    "Transform" | "RectTransform"
                ) {
                    return Err("property.transform_child_type".into());
                }
            }
        }
    }
    Ok(())
}

fn exact_keys<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .as_object()
        .filter(|map| map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)))
        .ok_or_else(|| "invalid serialized native value layout".into())
}
fn finite(value: &Value) -> Result<(), String> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
        .map(|_| ())
        .ok_or_else(|| "finite float32 required".into())
}
fn vector(value: &Value, keys: &[&str]) -> Result<(), String> {
    for v in exact_keys(value, keys)?.values() {
        finite(v)?;
    }
    Ok(())
}
/// Known complex values are still source-type checked before reaching here.
pub(super) fn check_value(name: &str, value: &Value) -> Result<bool, String> {
    match name.rsplit('.').next().unwrap_or(name) {
        "AnimationCurve" => {
            exact_keys(
                value,
                &[
                    "serializedVersion",
                    "m_Curve",
                    "m_PreInfinity",
                    "m_PostInfinity",
                    "m_RotationOrder",
                ],
            )?;
            if value["serializedVersion"] != 2 {
                return Err("unsupported curve version".into());
            }
            for key in value["m_Curve"].as_array().ok_or("curve keys required")? {
                exact_keys(
                    key,
                    &[
                        "serializedVersion",
                        "time",
                        "value",
                        "inSlope",
                        "outSlope",
                        "tangentMode",
                        "weightedMode",
                        "inWeight",
                        "outWeight",
                    ],
                )?;
                if key["serializedVersion"] != 3 {
                    return Err("unsupported curve key version".into());
                }
                for name in [
                    "time",
                    "value",
                    "inSlope",
                    "outSlope",
                    "inWeight",
                    "outWeight",
                ] {
                    finite(&key[name])?;
                }
                if !key["weightedMode"].as_u64().is_some_and(|v| v <= 3)
                    || !key["tangentMode"].is_i64()
                {
                    return Err("invalid curve mode".into());
                }
            }
            for name in ["m_PreInfinity", "m_PostInfinity"] {
                if !value[name].as_u64().is_some_and(|v| v <= 2) {
                    return Err("invalid curve infinity mode".into());
                }
            }
            if !value["m_RotationOrder"].is_i64() {
                return Err("invalid curve rotation order".into());
            }
        }
        "Gradient" => {
            if value["serializedVersion"] != 2 {
                return Err("unsupported gradient version".into());
            }
            for name in ["m_NumColorKeys", "m_NumAlphaKeys"] {
                if !value[name].as_u64().is_some_and(|v| (2..=8).contains(&v)) {
                    return Err("invalid gradient key count".into());
                }
            }
            for i in 0..8 {
                vector(&value[format!("key{i}")], &["r", "g", "b", "a"])?;
                for prefix in ["ctime", "atime"] {
                    if !value[format!("{prefix}{i}")]
                        .as_u64()
                        .is_some_and(|v| v <= 65535)
                    {
                        return Err("invalid gradient time".into());
                    }
                }
            }
            if !value["m_Mode"].as_u64().is_some_and(|v| v <= 1) {
                return Err("invalid gradient mode".into());
            }
        }
        "Bounds" => {
            exact_keys(value, &["m_Center", "m_Extent"])?;
            vector(&value["m_Center"], &["x", "y", "z"])?;
            vector(&value["m_Extent"], &["x", "y", "z"])?;
        }
        "Hash128" => {
            exact_keys(value, &["Hash"])?;
            if !value["Hash"]
                .as_str()
                .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("invalid Hash128".into());
            }
        }
        "__GameObjectComponents" => {
            for item in value.as_array().ok_or("component list required")? {
                exact_keys(item, &["component"])?;
                let r = &item["component"];
                if r.get("fileID").is_none() {
                    return Err("component reference required".into());
                }
                core::decode_asset_value(r).map_err(|e| e.to_string())?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
