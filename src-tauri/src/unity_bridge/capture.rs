use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureViewportRequest<'a> {
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_title: Option<&'a str>,
    max_long_edge: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityViewportCapture {
    pub target: String,
    pub title: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    #[serde(default)]
    pub source_width: Option<u32>,
    #[serde(default)]
    pub source_height: Option<u32>,
    #[serde(default)]
    pub output_width: Option<u32>,
    #[serde(default)]
    pub output_height: Option<u32>,
    #[serde(default)]
    pub max_long_edge: Option<u32>,
    #[serde(default)]
    pub pixels_per_point: Option<f32>,
    #[serde(default)]
    pub capture_area: Option<String>,
    pub mime_type: String,
}

impl UnityViewportCapture {
    pub fn effective_source_width(&self) -> u32 {
        self.source_width
            .filter(|value| *value > 0)
            .unwrap_or(self.original_width)
    }

    pub fn effective_source_height(&self) -> u32 {
        self.source_height
            .filter(|value| *value > 0)
            .unwrap_or(self.original_height)
    }

    pub fn effective_output_width(&self) -> u32 {
        self.output_width
            .filter(|value| *value > 0)
            .unwrap_or(self.width)
    }

    pub fn effective_output_height(&self) -> u32 {
        self.output_height
            .filter(|value| *value > 0)
            .unwrap_or(self.height)
    }
}

pub async fn capture_viewport(
    project_path: &str,
    target: &str,
    window_title: Option<&str>,
    max_long_edge: u32,
) -> Result<UnityViewportCapture, String> {
    let normalized_target = target.trim();
    if !matches!(normalized_target, "game" | "scene" | "editor_window") {
        return Err(format!(
            "Invalid target: '{}'. Allowed values: game, scene, editor_window.",
            target
        ));
    }

    let op_lock = super::project_unity_op_lock(project_path).await;
    let _guard = op_lock.lock().await;
    let payload = serde_json::to_string(&CaptureViewportRequest {
        target: normalized_target,
        window_title: window_title
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        max_long_edge,
    })
    .map_err(|e| e.to_string())?;
    let resp = super::send_message(project_path, "capture_viewport", &payload).await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "capture_viewport failed".to_string()));
    }
    let message = resp
        .message
        .ok_or_else(|| "capture_viewport returned an empty response".to_string())?;
    serde_json::from_str::<UnityViewportCapture>(&message)
        .map_err(|e| format!("Failed to parse capture_viewport response: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_camel_case_output_limit() {
        let payload = serde_json::to_value(CaptureViewportRequest {
            target: "game",
            window_title: None,
            max_long_edge: 2048,
        })
        .expect("serialize capture request");

        assert_eq!(payload["target"], "game");
        assert_eq!(payload["maxLongEdge"], 2048);
        assert!(payload.get("windowTitle").is_none());
    }

    #[test]
    fn legacy_response_dimensions_remain_compatible() {
        let capture: UnityViewportCapture = serde_json::from_value(serde_json::json!({
            "target": "scene",
            "title": "Scene",
            "path": "scene.png",
            "width": 1280,
            "height": 720,
            "originalWidth": 2560,
            "originalHeight": 1440,
            "mimeType": "image/png"
        }))
        .expect("parse legacy capture response");

        assert_eq!(capture.effective_source_width(), 2560);
        assert_eq!(capture.effective_source_height(), 1440);
        assert_eq!(capture.effective_output_width(), 1280);
        assert_eq!(capture.effective_output_height(), 720);
        assert_eq!(capture.max_long_edge, None);
    }
}
