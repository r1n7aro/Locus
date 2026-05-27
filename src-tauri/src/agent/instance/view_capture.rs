use crate::session::models::ImageData;
use crate::tool::ToolResult;

use super::{AgentInstance, ExecutedToolResult};

impl AgentInstance {
    pub(super) async fn execute_view_capture(
        &self,
        app_handle: &tauri::AppHandle,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        if !self.has_selected_working_dir() {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "view_capture requires a selected Unity project working directory."
                    .to_string(),
                is_error: true,
            });
        }

        let view_id = match args
            .get("viewId")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => value.to_string(),
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: viewId".to_string(),
                    is_error: true,
                });
            }
        };

        if let Err(error) = crate::view::read_view_sync(&self.working_dir, &view_id) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: error,
                is_error: true,
            });
        }

        let capture = match crate::view::capture_view_window(app_handle, &view_id).await {
            Ok(value) => value,
            Err(error) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: error,
                    is_error: true,
                });
            }
        };

        use base64::Engine as _;
        let image = ImageData {
            data: base64::engine::general_purpose::STANDARD.encode(&capture.bytes),
            mime_type: capture.mime_type.clone(),
        };
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "status": "captured",
            "viewId": capture.view_id,
            "windowLabel": capture.window_label,
            "format": capture.format,
            "mimeType": capture.mime_type,
            "width": capture.width,
            "height": capture.height,
            "byteSize": capture.byte_size,
            "image": "attached"
        }))
        .unwrap_or_else(|_| "View screenshot captured. PNG image attached.".to_string());

        ExecutedToolResult::from_tool_result(ToolResult {
            output,
            is_error: false,
        })
        .with_images(vec![image])
    }
}
