use crate::session::models::ImageData;
use crate::tool::ToolResult;

use super::{AgentInstance, ExecutedToolResult};

impl AgentInstance {
    pub(super) async fn execute_unity_capture_viewport(
        &self,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        if !self.has_selected_working_dir() {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output:
                    "unity_capture_viewport requires a selected Unity project working directory."
                        .to_string(),
                is_error: true,
            });
        }

        let requested_status = match args
            .get("request_editor_status")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(status) => status,
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: request_editor_status".to_string(),
                    is_error: true,
                });
            }
        };

        if requested_status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
            || !crate::unity_bridge::is_known_editor_status(requested_status)
        {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
                    requested_status
                ),
                is_error: true,
            });
        }

        let target = match args
            .get("target")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(target @ ("game" | "scene" | "editor_window")) => target,
            Some(other) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!(
                        "Invalid target: '{}'. Allowed values: game, scene, editor_window.",
                        other
                    ),
                    is_error: true,
                });
            }
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: target".to_string(),
                    is_error: true,
                });
            }
        };

        let (connected, actual_status, _) =
            crate::unity_bridge::query_unity_status(&self.working_dir).await;
        if !connected {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "Unity Editor not connected".to_string(),
                is_error: true,
            });
        }
        if actual_status != requested_status {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Unity Editor status is \"{}\". `unity_capture_viewport` requires \"{}\".",
                    actual_status, requested_status
                ),
                is_error: true,
            });
        }

        let window_title = args
            .get("window_title")
            .or_else(|| args.get("windowTitle"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let capture =
            match crate::unity_bridge::capture_viewport(&self.working_dir, target, window_title)
                .await
            {
                Ok(capture) => capture,
                Err(error) => {
                    return ExecutedToolResult::from_tool_result(ToolResult {
                        output: error,
                        is_error: true,
                    });
                }
            };

        let image_bytes = match tokio::fs::read(&capture.path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!(
                        "Failed to read Unity viewport screenshot '{}': {}",
                        capture.path, error
                    ),
                    is_error: true,
                });
            }
        };

        use base64::Engine as _;
        let mime_type = if capture.mime_type.trim().is_empty() {
            "image/png".to_string()
        } else {
            capture.mime_type.clone()
        };
        let image = ImageData {
            data: base64::engine::general_purpose::STANDARD.encode(image_bytes),
            mime_type: mime_type.clone(),
        };
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "status": "captured",
            "target": capture.target,
            "title": capture.title,
            "format": "png",
            "mime_type": mime_type,
            "width": capture.width,
            "height": capture.height,
            "original_width": capture.original_width,
            "original_height": capture.original_height,
            "path": capture.path,
            "image": "attached"
        }))
        .unwrap_or_else(|_| "Unity viewport screenshot captured. PNG image attached.".to_string());

        ExecutedToolResult::from_tool_result(ToolResult {
            output,
            is_error: false,
        })
        .with_images(vec![image])
    }
}
