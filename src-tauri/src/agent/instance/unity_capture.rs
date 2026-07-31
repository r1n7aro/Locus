use crate::session::models::ImageData;
use crate::tool::ToolResult;

use super::{AgentInstance, ExecutedToolResult};

const DEFAULT_CAPTURE_MAX_LONG_EDGE: u32 = 1280;
const MAX_CAPTURE_MAX_LONG_EDGE: u32 = 8192;

fn parse_capture_max_long_edge(args: &serde_json::Value) -> Result<u32, String> {
    let Some(value) = args
        .get("max_long_edge")
        .or_else(|| args.get("maxLongEdge"))
    else {
        return Ok(DEFAULT_CAPTURE_MAX_LONG_EDGE);
    };

    let max_long_edge = value.as_u64().and_then(|value| u32::try_from(value).ok());
    match max_long_edge {
        Some(value) if value <= MAX_CAPTURE_MAX_LONG_EDGE => Ok(value),
        _ => Err(format!(
            "Invalid max_long_edge. Expected an integer from 0 to {}.",
            MAX_CAPTURE_MAX_LONG_EDGE
        )),
    }
}

impl AgentInstance {
    pub(crate) async fn execute_unity_capture_viewport(
        working_dir: &str,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        if !Self::has_selected_working_dir_value(working_dir) {
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
            crate::unity_bridge::query_unity_status(working_dir).await;
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

        let max_long_edge = match parse_capture_max_long_edge(args) {
            Ok(value) => value,
            Err(error) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: error,
                    is_error: true,
                });
            }
        };

        let capture = match crate::unity_bridge::capture_viewport(
            working_dir,
            target,
            window_title,
            max_long_edge,
        )
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
        let source_width = capture.effective_source_width();
        let source_height = capture.effective_source_height();
        let output_width = capture.effective_output_width();
        let output_height = capture.effective_output_height();
        let applied_max_long_edge = capture.max_long_edge.unwrap_or(max_long_edge);
        let capture_area = capture
            .capture_area
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(if target == "editor_window" {
                "window"
            } else {
                "viewport"
            });
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "status": "captured",
            "target": capture.target,
            "title": capture.title,
            "format": "png",
            "mime_type": mime_type,
            "width": output_width,
            "height": output_height,
            "original_width": source_width,
            "original_height": source_height,
            "source_width": source_width,
            "source_height": source_height,
            "output_width": output_width,
            "output_height": output_height,
            "max_long_edge": applied_max_long_edge,
            "pixels_per_point": capture.pixels_per_point,
            "capture_area": capture_area,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_max_long_edge_defaults_to_1280() {
        assert_eq!(
            parse_capture_max_long_edge(&serde_json::json!({})).unwrap(),
            DEFAULT_CAPTURE_MAX_LONG_EDGE
        );
    }

    #[test]
    fn capture_max_long_edge_accepts_zero_and_camel_case() {
        assert_eq!(
            parse_capture_max_long_edge(&serde_json::json!({ "max_long_edge": 0 })).unwrap(),
            0
        );
        assert_eq!(
            parse_capture_max_long_edge(&serde_json::json!({ "maxLongEdge": 2048 })).unwrap(),
            2048
        );
    }

    #[test]
    fn capture_max_long_edge_rejects_invalid_values() {
        for value in [
            serde_json::json!(-1),
            serde_json::json!(8193),
            serde_json::json!(1.5),
        ] {
            assert!(parse_capture_max_long_edge(&serde_json::json!({
                "max_long_edge": value
            }))
            .is_err());
        }
    }
}
