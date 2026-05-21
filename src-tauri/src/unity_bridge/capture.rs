use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureViewportRequest<'a> {
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_title: Option<&'a str>,
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
    pub mime_type: String,
}

pub async fn capture_viewport(
    project_path: &str,
    target: &str,
    window_title: Option<&str>,
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
