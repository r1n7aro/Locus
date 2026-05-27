use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WindowEvent};

use crate::error::AppError;
use crate::session::models::ImageData;

pub const AGENT_GRAPH_TOOL_ROUTE: &str = "/agent-graph";
const AGENT_GRAPH_TOOL_LABEL_PREFIX: &str = "agent-graph-";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolOption {
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolRequest {
    pub request_id: String,
    pub tool_call_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub editable: bool,
    pub graph: serde_json::Value,
    #[serde(default)]
    pub options: Vec<AgentGraphToolOption>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub return_image: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolPayload {
    pub request_id: String,
    pub tool_call_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub editable: bool,
    pub graph: serde_json::Value,
    #[serde(default)]
    pub options: Vec<AgentGraphToolOption>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub return_image: bool,
}

impl From<&AgentGraphToolRequest> for AgentGraphToolPayload {
    fn from(request: &AgentGraphToolRequest) -> Self {
        Self {
            request_id: request.request_id.clone(),
            tool_call_id: request.tool_call_id.clone(),
            title: request.title.clone(),
            description: request.description.clone(),
            editable: request.editable,
            graph: request.graph.clone(),
            options: request.options.clone(),
            return_image: request.return_image,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolSelectedOption {
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolSubmitRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<AgentGraphToolSelectedOption>,
    pub graph: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageData>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolSubmitResult {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolOpenResult {
    pub request_id: String,
    pub window_label: String,
    pub host_url: String,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphToolReopenRequest {
    pub tool_call_id: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug)]
pub enum AgentGraphToolAnswer {
    Submitted(AgentGraphToolSubmitRequest),
    Cancelled,
}

pub struct AgentGraphToolEntry {
    pub request: AgentGraphToolRequest,
    pub tx: Option<tokio::sync::oneshot::Sender<AgentGraphToolAnswer>>,
}

pub type AgentGraphToolStore = Arc<tokio::sync::Mutex<HashMap<String, AgentGraphToolEntry>>>;

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn agent_graph_tool_window_label(request_id: &str) -> String {
    format!("{}{}", AGENT_GRAPH_TOOL_LABEL_PREFIX, request_id)
}

pub fn agent_graph_tool_request_id_from_label(label: &str) -> Option<String> {
    label
        .strip_prefix(AGENT_GRAPH_TOOL_LABEL_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn agent_graph_tool_request_from_args(
    args: &serde_json::Value,
    tool_call_id: &str,
) -> Result<AgentGraphToolRequest, String> {
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Graph")
        .to_string();
    let editable = args
        .get("editable")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let graph = normalize_graph_arg(args)?;
    let options = normalize_graph_tool_options(args.get("options"))?;
    let return_image = args
        .get("returnImage")
        .or_else(|| args.get("return_image"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    Ok(AgentGraphToolRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        tool_call_id: tool_call_id.to_string(),
        title,
        description: None,
        editable,
        graph,
        options,
        return_image,
    })
}

fn agent_graph_tool_snapshot_request_id(request: &AgentGraphToolReopenRequest) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    request.tool_call_id.hash(&mut hasher);
    request.arguments.hash(&mut hasher);
    request.output.hash(&mut hasher);
    format!("snapshot-{:016x}", hasher.finish())
}

fn submitted_graph_from_output(output: Option<&str>) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(output?).ok()?;
    parsed
        .get("graph")
        .filter(|value| value.is_object())
        .cloned()
}

fn request_id_from_output(output: Option<&str>) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(output?).ok()?;
    parsed
        .get("requestId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn agent_graph_tool_reopen_request_from_record(
    request: &AgentGraphToolReopenRequest,
) -> Result<AgentGraphToolRequest, String> {
    let mut args = serde_json::from_str::<serde_json::Value>(&request.arguments)
        .map_err(|error| format!("Error parsing graph_view arguments: {}", error))?;
    let Some(args_object) = args.as_object_mut() else {
        return Err("graph_view arguments must be an object.".to_string());
    };

    if let Some(graph) = submitted_graph_from_output(request.output.as_deref()) {
        args_object.insert("graph".to_string(), graph);
        args_object.remove("nodes");
        args_object.remove("links");
        args_object.remove("connections");
    }
    args_object.insert("editable".to_string(), serde_json::Value::Bool(false));
    args_object.remove("options");

    let mut graph_request = agent_graph_tool_request_from_args(&args, &request.tool_call_id)?;
    graph_request.request_id = agent_graph_tool_snapshot_request_id(request);
    graph_request.editable = false;
    graph_request.options = Vec::new();
    graph_request.return_image = false;
    Ok(graph_request)
}

fn normalize_graph_arg(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(graph) = args.get("graph").filter(|value| value.is_object()) {
        let mut graph = graph.clone();
        if let Some(layout) = args.get("layout") {
            graph["layout"] = layout.clone();
        }
        validate_graph_value(&graph)?;
        return Ok(graph);
    }

    let nodes = args
        .get("nodes")
        .ok_or_else(|| "Missing required parameter: nodes".to_string())?;
    if !nodes.is_array() {
        return Err("Parameter 'nodes' must be an array.".to_string());
    }
    let links = args
        .get("links")
        .or_else(|| args.get("connections"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    if !links.is_array() {
        return Err("Parameter 'links' must be an array.".to_string());
    }

    let mut graph = serde_json::json!({
        "schema": "locus.graph.v1",
        "nodes": nodes.clone(),
        "links": links,
    });
    if let Some(layout) = args.get("layout") {
        graph["layout"] = layout.clone();
    }
    validate_graph_value(&graph)?;
    Ok(graph)
}

fn validate_graph_value(graph: &serde_json::Value) -> Result<(), String> {
    let Some(nodes) = graph.get("nodes").and_then(|value| value.as_array()) else {
        return Err("Graph requires a 'nodes' array.".to_string());
    };
    if nodes.is_empty() {
        return Err("Graph requires at least one node.".to_string());
    }
    for (index, node) in nodes.iter().enumerate() {
        let id = node
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        if id.is_empty() {
            return Err(format!("Graph node at index {} is missing an id.", index));
        }
    }

    let links = graph
        .get("links")
        .or_else(|| graph.get("connections"))
        .or_else(|| graph.get("edges"));
    if let Some(links) = links {
        let Some(links) = links.as_array() else {
            return Err("Graph links/connections must be an array.".to_string());
        };
        for (index, link) in links.iter().enumerate() {
            let from = link
                .get("from")
                .and_then(|value| value.get("nodeId"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or("");
            let to = link
                .get("to")
                .and_then(|value| value.get("nodeId"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or("");
            if from.is_empty() || to.is_empty() {
                return Err(format!(
                    "Graph link at index {} requires from.nodeId and to.nodeId.",
                    index
                ));
            }
        }
    }

    Ok(())
}

fn validate_submit_images(images: Option<&Vec<ImageData>>) -> Result<(), String> {
    let Some(images) = images else {
        return Ok(());
    };
    if images.len() > 1 {
        return Err("Graph submission supports at most one layout image.".to_string());
    }
    for image in images {
        if image.data.trim().is_empty() {
            return Err("Graph layout image data is empty.".to_string());
        }
        if image.data.len() > 16 * 1024 * 1024 {
            return Err("Graph layout image is too large.".to_string());
        }
        if image.mime_type != "image/png"
            && image.mime_type != "image/jpeg"
            && image.mime_type != "image/webp"
        {
            return Err("Graph layout image must be PNG, JPEG, or WebP.".to_string());
        }
    }
    Ok(())
}

fn normalize_graph_tool_options(
    value: Option<&serde_json::Value>,
) -> Result<Vec<AgentGraphToolOption>, String> {
    let Some(value) = value else {
        return Ok(default_graph_tool_options());
    };
    let Some(items) = value.as_array() else {
        return Err("Parameter 'options' must be an array.".to_string());
    };
    if items.len() > 3 {
        return Err("Parameter 'options' supports at most 3 items.".to_string());
    }
    let mut options = Vec::new();
    for item in items {
        let label = item
            .get("label")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Each option requires a non-empty label.".to_string())?
            .to_string();
        let description = item
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let value = item
            .get("value")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        options.push(AgentGraphToolOption {
            label,
            description,
            value,
        });
    }
    if options.is_empty() {
        Ok(default_graph_tool_options())
    } else {
        Ok(options)
    }
}

fn default_graph_tool_options() -> Vec<AgentGraphToolOption> {
    vec![AgentGraphToolOption {
        label: "Confirm".to_string(),
        description: "Use the current graph.".to_string(),
        value: Some("confirm".to_string()),
    }]
}

pub async fn insert_agent_graph_tool_request(
    store: &AgentGraphToolStore,
    request: AgentGraphToolRequest,
    tx: Option<tokio::sync::oneshot::Sender<AgentGraphToolAnswer>>,
) {
    let mut guard = store.lock().await;
    guard.insert(
        request.request_id.clone(),
        AgentGraphToolEntry { request, tx },
    );
}

pub async fn remove_agent_graph_tool_request(
    store: &AgentGraphToolStore,
    request_id: &str,
) -> Option<AgentGraphToolEntry> {
    let mut guard = store.lock().await;
    guard.remove(request_id)
}

pub async fn cancel_agent_graph_tool_request_by_id(
    store: &AgentGraphToolStore,
    request_id: &str,
) -> bool {
    match remove_agent_graph_tool_request(store, request_id).await {
        Some(mut entry) => {
            if let Some(tx) = entry.tx.take() {
                let _ = tx.send(AgentGraphToolAnswer::Cancelled);
            }
            true
        }
        None => false,
    }
}

pub fn open_agent_graph_tool_window(
    app_handle: &AppHandle,
    request: &AgentGraphToolRequest,
) -> Result<AgentGraphToolOpenResult, String> {
    let label = agent_graph_tool_window_label(&request.request_id);
    let host_url = format!("{}?id={}", AGENT_GRAPH_TOOL_ROUTE, request.request_id);

    if let Some(window) = app_handle.get_webview_window(&label) {
        window
            .set_focus()
            .map_err(|error| format!("Failed to focus Graph window: {}", error))?;
    } else {
        tauri::WebviewWindowBuilder::new(
            app_handle,
            &label,
            WebviewUrl::App(host_url.clone().into()),
        )
        .title(format!("{} - Locus Graph", request.title))
        .inner_size(1180.0, 760.0)
        .min_inner_size(760.0, 520.0)
        .decorations(false)
        .resizable(true)
        .visible(true)
        .disable_drag_drop_handler()
        .build()
        .map_err(|error| format!("Failed to open Graph window: {}", error))?;
    }

    Ok(AgentGraphToolOpenResult {
        request_id: request.request_id.clone(),
        window_label: label,
        host_url,
        editable: request.editable,
    })
}

pub fn close_agent_graph_tool_window(app_handle: &AppHandle, request_id: &str) {
    let label = agent_graph_tool_window_label(request_id);
    if let Some(window) = app_handle.get_webview_window(&label) {
        let _ = window.destroy().or_else(|_| window.close());
    }
}

pub fn handle_agent_graph_tool_window_event(window: &tauri::Window, event: &WindowEvent) {
    if !matches!(
        event,
        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
    ) {
        return;
    }
    let Some(request_id) = agent_graph_tool_request_id_from_label(window.label()) else {
        return;
    };
    let app_handle = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let store: tauri::State<'_, AgentGraphToolStore> = app_handle.state();
        let _ = cancel_agent_graph_tool_request_by_id(store.inner(), &request_id).await;
    });
}

#[tauri::command]
pub async fn agent_graph_tool_request(
    request_id: String,
    store: State<'_, AgentGraphToolStore>,
) -> Result<AgentGraphToolPayload, AppError> {
    let guard = store.lock().await;
    let Some(entry) = guard.get(&request_id) else {
        return Err(format!("Graph request '{}' not found.", request_id).into());
    };
    Ok(AgentGraphToolPayload::from(&entry.request))
}

#[tauri::command]
pub async fn agent_graph_tool_submit(
    request: AgentGraphToolSubmitRequest,
    store: State<'_, AgentGraphToolStore>,
) -> Result<AgentGraphToolSubmitResult, AppError> {
    validate_graph_value(&request.graph).map_err(AppError::from)?;
    validate_submit_images(request.images.as_ref()).map_err(AppError::from)?;
    let Some(mut entry) = remove_agent_graph_tool_request(store.inner(), &request.request_id).await
    else {
        return Err(format!("Graph request '{}' not found.", request.request_id).into());
    };
    if let Some(tx) = entry.tx.take() {
        tx.send(AgentGraphToolAnswer::Submitted(request))
            .map_err(|_| AppError::from("Graph receiver dropped"))?;
    }
    Ok(AgentGraphToolSubmitResult {
        status: "submitted".to_string(),
    })
}

#[tauri::command]
pub async fn agent_graph_tool_cancel(
    request_id: String,
    store: State<'_, AgentGraphToolStore>,
) -> Result<AgentGraphToolSubmitResult, AppError> {
    let cancelled = cancel_agent_graph_tool_request_by_id(store.inner(), &request_id).await;
    Ok(AgentGraphToolSubmitResult {
        status: if cancelled { "cancelled" } else { "missing" }.to_string(),
    })
}

#[tauri::command]
pub async fn agent_graph_tool_reopen(
    request: AgentGraphToolReopenRequest,
    store: State<'_, AgentGraphToolStore>,
    app_handle: AppHandle,
) -> Result<AgentGraphToolOpenResult, AppError> {
    if let Some(existing_request_id) = request_id_from_output(request.output.as_deref()) {
        let existing_request = {
            let guard = store.lock().await;
            guard
                .get(&existing_request_id)
                .map(|entry| entry.request.clone())
        };
        if let Some(existing_request) = existing_request {
            return open_agent_graph_tool_window(&app_handle, &existing_request)
                .map_err(AppError::from);
        }
    }

    let graph_request =
        agent_graph_tool_reopen_request_from_record(&request).map_err(AppError::from)?;
    insert_agent_graph_tool_request(store.inner(), graph_request.clone(), None).await;
    match open_agent_graph_tool_window(&app_handle, &graph_request) {
        Ok(result) => Ok(result),
        Err(error) => {
            let _ = remove_agent_graph_tool_request(store.inner(), &graph_request.request_id).await;
            Err(AppError::from(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_tool_accepts_top_level_nodes_and_links() {
        let args = serde_json::json!({
            "title": "Shader Graph",
            "description": "Ignored graph description",
            "editable": true,
            "nodes": [
                { "id": "texture", "title": "Texture" },
                { "id": "color", "title": "Color" }
            ],
            "links": [
                { "from": { "nodeId": "texture" }, "to": { "nodeId": "color" } }
            ],
            "layout": { "auto": "missing", "direction": "right" },
            "options": [
                { "label": "Apply", "description": "Rewrite shader", "value": "apply" }
            ]
        });

        let request = agent_graph_tool_request_from_args(&args, "tool-1").unwrap();

        assert!(request.editable);
        assert_eq!(request.title, "Shader Graph");
        assert_eq!(request.description, None);
        assert_eq!(request.graph["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(request.graph["links"].as_array().unwrap().len(), 1);
        assert_eq!(request.graph["layout"]["direction"], "right");
        assert_eq!(request.options[0].value.as_deref(), Some("apply"));
        assert!(!request.return_image);
    }

    #[test]
    fn graph_tool_accepts_manual_layout_image_request() {
        let args = serde_json::json!({
            "editable": true,
            "returnImage": true,
            "nodes": [
                { "id": "a", "x": 20, "y": 30 },
                { "id": "b", "x": 280, "y": 30 }
            ],
            "links": [
                { "from": { "nodeId": "a" }, "to": { "nodeId": "b" } }
            ],
            "layout": { "mode": "manual", "auto": "off" }
        });

        let request = agent_graph_tool_request_from_args(&args, "tool-1").unwrap();

        assert!(request.return_image);
        assert_eq!(request.graph["layout"]["mode"], "manual");
    }

    #[test]
    fn graph_tool_rejects_missing_node_ids() {
        let args = serde_json::json!({
            "nodes": [{ "title": "Missing id" }]
        });

        let error = agent_graph_tool_request_from_args(&args, "tool-1").unwrap_err();

        assert!(error.contains("missing an id"));
    }

    #[test]
    fn graph_tool_limits_options_to_three() {
        let args = serde_json::json!({
            "nodes": [{ "id": "a" }],
            "options": [
                { "label": "A", "description": "" },
                { "label": "B", "description": "" },
                { "label": "C", "description": "" },
                { "label": "D", "description": "" }
            ]
        });

        let error = agent_graph_tool_request_from_args(&args, "tool-1").unwrap_err();

        assert!(error.contains("at most 3"));
    }

    #[test]
    fn graph_tool_validates_submit_images() {
        let image = ImageData {
            data: "abc".to_string(),
            mime_type: "image/png".to_string(),
        };

        assert!(validate_submit_images(Some(&vec![image])).is_ok());

        let unsupported = ImageData {
            data: "abc".to_string(),
            mime_type: "image/svg+xml".to_string(),
        };
        let error = validate_submit_images(Some(&vec![unsupported])).unwrap_err();

        assert!(error.contains("PNG"));
    }

    #[test]
    fn graph_tool_reopen_forces_readonly_and_uses_submitted_graph() {
        let request = AgentGraphToolReopenRequest {
            tool_call_id: "tool-1".to_string(),
            arguments: serde_json::json!({
                "title": "Editable Graph",
                "editable": true,
                "nodes": [{ "id": "original" }],
                "links": [],
                "options": [
                    { "label": "Apply", "description": "Apply graph", "value": "apply" }
                ]
            })
            .to_string(),
            output: Some(
                serde_json::json!({
                    "status": "submitted",
                    "requestId": "request-1",
                    "graph": {
                        "schema": "locus.graph.v1",
                        "nodes": [{ "id": "submitted" }],
                        "links": []
                    }
                })
                .to_string(),
            ),
        };

        let reopened = agent_graph_tool_reopen_request_from_record(&request).unwrap();

        assert!(!reopened.editable);
        assert!(reopened.request_id.starts_with("snapshot-"));
        assert!(reopened.options.is_empty());
        assert_eq!(reopened.graph["nodes"][0]["id"], "submitted");
    }

    #[test]
    fn graph_tool_reopen_falls_back_to_original_graph_args() {
        let request = AgentGraphToolReopenRequest {
            tool_call_id: "tool-2".to_string(),
            arguments: serde_json::json!({
                "title": "Readonly Graph",
                "editable": false,
                "nodes": [{ "id": "node-a" }],
                "links": []
            })
            .to_string(),
            output: Some("Graph editing was cancelled before confirmation.".to_string()),
        };

        let reopened = agent_graph_tool_reopen_request_from_record(&request).unwrap();

        assert!(!reopened.editable);
        assert_eq!(reopened.graph["nodes"][0]["id"], "node-a");
    }
}
