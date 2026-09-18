use crate::session::models::ImageData;
use crate::tool::ToolResult;
use super::{AgentInstance, ExecutedToolResult};

impl AgentInstance {
    pub(super) async fn execute_frontend_typescript(&self, app: &tauri::AppHandle, args: &serde_json::Value) -> ExecutedToolResult {
        let code = args.get("code").and_then(|value| value.as_str()).unwrap_or("");
        let label = args.get("windowLabel").and_then(|value| value.as_str());
        let timeout = args.get("timeoutMs").and_then(|value| value.as_u64()).unwrap_or(30_000);
        match crate::view::request_frontend_execution(app, &self.working_dir, code, label, timeout).await {
            Ok(mut result) => {
                let images = result.get("images").and_then(|value| value.as_array()).map(|values| values.iter().filter_map(|value| {
                    Some(ImageData { data: value.get("data")?.as_str()?.to_string(), mime_type: value.get("mimeType")?.as_str()?.to_string() })
                }).collect::<Vec<_>>()).unwrap_or_default();
                if let Some(object) = result.as_object_mut() { object.insert("images".to_string(), serde_json::json!({ "attached": images.len() })); }
                ExecutedToolResult::from_tool_result(ToolResult { output: result.to_string(), is_error: false }).with_images(images)
            },
            Err(error) => ExecutedToolResult::from_tool_result(ToolResult { output: error, is_error: true }),
        }
    }
}
