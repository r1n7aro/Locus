use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use crate::error::AppError;
use crate::unity_bridge;

pub type CanvasSpecStore = Arc<tokio::sync::Mutex<HashMap<String, String>>>;

#[tauri::command]
pub async fn canvas_set_spec(
    spec_id: String,
    spec: String,
    store: State<'_, CanvasSpecStore>,
) -> Result<(), AppError> {
    store.lock().await.insert(spec_id, spec);
    Ok(())
}

#[tauri::command]
pub async fn canvas_get_spec(
    spec_id: String,
    store: State<'_, CanvasSpecStore>,
) -> Result<String, AppError> {
    store
        .lock()
        .await
        .get(&spec_id)
        .cloned()
        .ok_or_else(|| format!("Canvas spec not found: {}", spec_id).into())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldUpdateParams {
    pub mode: String,
    pub game_object_path: Option<String>,
    pub component_type: Option<String>,
    pub property_path: Option<String>,
    pub code: Option<String>,
}

#[tauri::command]
pub async fn canvas_update_field(
    project_path: String,
    scene_path: Option<String>,
    update: FieldUpdateParams,
    value: serde_json::Value,
    value_type: String,
) -> Result<serde_json::Value, AppError> {
    match update.mode.as_str() {
        "serialized" => {
            let go_path = update
                .game_object_path
                .ok_or(AppError::from("Missing gameObjectPath"))?;
            let comp_type = update
                .component_type
                .ok_or(AppError::from("Missing componentType"))?;
            let prop_path = update
                .property_path
                .ok_or(AppError::from("Missing propertyPath"))?;

            let msg = serde_json::json!({
                "scenePath": scene_path.unwrap_or_default(),
                "gameObjectPath": go_path,
                "componentType": comp_type,
                "propertyPath": prop_path,
                "value": value,
                "valueType": value_type,
            });

            let resp = unity_bridge::send_message(
                &project_path,
                "set_serialized_data",
                &serde_json::to_string(&msg).map_err(|e| e.to_string())?,
            )
            .await?;

            if resp.ok {
                Ok(serde_json::json!({ "ok": true }))
            } else {
                Ok(serde_json::json!({
                    "ok": false,
                    "error": resp.error.unwrap_or_else(|| "unknown error".to_string())
                }))
            }
        }
        "code" => {
            let code = update
                .code
                .ok_or(AppError::from("Missing code for code mode"))?;

            let value_decl = generate_csharp_value_decl(&value, &value_type);
            let full_code = format!("{}\n{}", value_decl, code);

            match unity_bridge::unity_execute_code(&project_path, &full_code).await {
                Ok(output) => Ok(serde_json::json!({ "ok": true, "output": output })),
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": e })),
            }
        }
        other => Err(format!("Unknown update mode: {}", other).into()),
    }
}

fn generate_csharp_value_decl(value: &serde_json::Value, value_type: &str) -> String {
    match value_type {
        "int" => format!("var VALUE = {};", value.as_i64().unwrap_or(0)),
        "float" => {
            let f = value.as_f64().unwrap_or(0.0);
            format!("var VALUE = {}f;", f)
        }
        "bool" => format!(
            "var VALUE = {};",
            if value.as_bool().unwrap_or(false) {
                "true"
            } else {
                "false"
            }
        ),
        "string" => {
            let s = value.as_str().unwrap_or("");
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("var VALUE = \"{}\";", escaped)
        }
        "enum" => format!("var VALUE = {};", value.as_i64().unwrap_or(0)),
        "vector2" => {
            let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            format!("var VALUE = new Vector2({}f, {}f);", x, y)
        }
        "vector3" => {
            let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = value.get("z").and_then(|v| v.as_f64()).unwrap_or(0.0);
            format!("var VALUE = new Vector3({}f, {}f, {}f);", x, y, z)
        }
        "vector4" => {
            let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = value.get("z").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = value.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
            format!("var VALUE = new Vector4({}f, {}f, {}f, {}f);", x, y, z, w)
        }
        "color" => {
            let r = value.get("r").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let g = value.get("g").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let b = value.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let a = value.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0);
            format!("var VALUE = new Color({}f, {}f, {}f, {}f);", r, g, b, a)
        }
        _ => format!(
            "object VALUE = \"{}\";",
            value
                .as_str()
                .unwrap_or("")
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshQuery {
    pub id: String,
    pub game_object_path: String,
    pub component_type: String,
    pub property_path: String,
}

#[tauri::command]
pub async fn canvas_refresh(
    project_path: String,
    scene_path: Option<String>,
    queries: Vec<RefreshQuery>,
) -> Result<serde_json::Value, AppError> {
    let query_array: Vec<serde_json::Value> = queries
        .iter()
        .map(|q| {
            serde_json::json!({
                "id": q.id,
                "gameObjectPath": q.game_object_path,
                "componentType": q.component_type,
                "propertyPath": q.property_path,
            })
        })
        .collect();

    let msg = serde_json::json!({
        "scenePath": scene_path.unwrap_or_default(),
        "queries": query_array,
    });

    let resp = unity_bridge::send_message(
        &project_path,
        "get_serialized_data",
        &serde_json::to_string(&msg).map_err(|e| e.to_string())?,
    )
    .await?;

    if resp.ok {
        let results: serde_json::Value = resp
            .message
            .as_deref()
            .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::Null))
            .unwrap_or(serde_json::Value::Null);
        Ok(results)
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "get_serialized_data failed".to_string())
            .into())
    }
}

const CANVAS_DIR: &str = "Locus/Canvases";

fn canvas_dir(project_path: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::path::Path::new(project_path).join(CANVAS_DIR);
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create canvas directory: {}", e))?;
    }
    Ok(dir)
}

fn sanitize_canvas_name(name: &str) -> String {
    name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

#[tauri::command]
pub async fn canvas_save(
    project_path: String,
    name: String,
    data: String,
) -> Result<String, AppError> {
    let dir = canvas_dir(&project_path)?;
    let file_name = format!("{}.canvas.json", sanitize_canvas_name(&name));
    let file_path = dir.join(&file_name);
    std::fs::write(&file_path, &data).map_err(|e| format!("Failed to save canvas: {}", e))?;
    Ok(file_path.display().to_string())
}

#[tauri::command]
pub async fn canvas_load(project_path: String, name: String) -> Result<String, AppError> {
    let dir = canvas_dir(&project_path)?;
    let file_name = format!("{}.canvas.json", sanitize_canvas_name(&name));
    let file_path = dir.join(&file_name);
    std::fs::read_to_string(&file_path).map_err(|e| format!("Failed to load canvas: {}", e).into())
}

#[tauri::command]
pub async fn canvas_list(project_path: String) -> Result<Vec<String>, AppError> {
    let dir = canvas_dir(&project_path)?;
    let mut names = Vec::new();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("Failed to read canvas directory: {}", e))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".canvas.json") {
            names.push(name.trim_end_matches(".canvas.json").to_string());
        }
    }
    names.sort();
    Ok(names)
}

#[tauri::command]
pub async fn canvas_delete(project_path: String, name: String) -> Result<(), AppError> {
    let dir = canvas_dir(&project_path)?;
    let file_name = format!("{}.canvas.json", sanitize_canvas_name(&name));
    let file_path = dir.join(&file_name);
    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| format!("Failed to delete canvas: {}", e))?;
    }
    Ok(())
}
