use crate::session::models::ImageData;
use crate::tool::ToolResult;

use super::{AgentInstance, ExecutedToolResult};

const MAX_READ_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

impl AgentInstance {
    pub(super) async fn execute_read(
        &self,
        app_handle: &tauri::AppHandle,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        let file_path = match args.get("filePath").and_then(|value| value.as_str()) {
            Some(path) => path.trim(),
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: "Missing required parameter: filePath".to_string(),
                    is_error: true,
                });
            }
        };

        if !Self::is_read_image_path(file_path) {
            let tool_context = self.build_tool_execution_context(app_handle, "read").await;
            return self
                .await_tool_result(self.tool_registry.execute_with_context(
                    "read",
                    args,
                    tool_context,
                ))
                .await;
        }

        let metadata = match tokio::fs::metadata(file_path).await {
            Ok(metadata) => metadata,
            Err(_) => {
                let tool_context = self.build_tool_execution_context(app_handle, "read").await;
                return self
                    .await_tool_result(self.tool_registry.execute_with_context(
                        "read",
                        args,
                        tool_context,
                    ))
                    .await;
            }
        };

        if metadata.is_dir() {
            let tool_context = self.build_tool_execution_context(app_handle, "read").await;
            return self
                .await_tool_result(self.tool_registry.execute_with_context(
                    "read",
                    args,
                    tool_context,
                ))
                .await;
        }

        if metadata.len() > MAX_READ_IMAGE_BYTES {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Image file is too large to attach: {} ({} bytes, max {} bytes)",
                    file_path,
                    metadata.len(),
                    MAX_READ_IMAGE_BYTES
                ),
                is_error: true,
            });
        }

        let image_bytes = match tokio::fs::read(file_path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!("Failed to read image file '{}': {}", file_path, error),
                    is_error: true,
                });
            }
        };

        let mime_type = match Self::detect_read_image_mime(&image_bytes) {
            Some(mime_type) => mime_type,
            None => {
                return ExecutedToolResult::from_tool_result(ToolResult {
                    output: format!(
                        "File extension looks like an image, but content is not a supported PNG, JPEG, GIF, or WebP image: {}",
                        file_path
                    ),
                    is_error: true,
                });
            }
        };

        use base64::Engine as _;
        let image = ImageData {
            data: base64::engine::general_purpose::STANDARD.encode(&image_bytes),
            mime_type: mime_type.to_string(),
        };
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "status": "read",
            "file_path": file_path,
            "mime_type": mime_type,
            "byte_size": image_bytes.len(),
            "image": "attached"
        }))
        .unwrap_or_else(|_| "Image file read. Image attached.".to_string());

        ExecutedToolResult::from_tool_result(ToolResult {
            output,
            is_error: false,
        })
        .with_images(vec![image])
    }

    fn is_read_image_path(file_path: &str) -> bool {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());

        matches!(
            ext.as_deref(),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp")
        )
    }

    fn detect_read_image_mime(bytes: &[u8]) -> Option<&'static str> {
        if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return Some("image/png");
        }
        if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
            return Some("image/jpeg");
        }
        if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
            return Some("image/gif");
        }
        if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return Some("image/webp");
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_image_path_accepts_common_web_images() {
        assert!(AgentInstance::is_read_image_path("Assets/hero.PNG"));
        assert!(AgentInstance::is_read_image_path("Assets/hero.jpeg"));
        assert!(AgentInstance::is_read_image_path("Assets/hero.webp"));
        assert!(!AgentInstance::is_read_image_path("Assets/hero.svg"));
        assert!(!AgentInstance::is_read_image_path("Assets/hero.psd"));
    }

    #[test]
    fn detect_read_image_mime_uses_magic_bytes() {
        assert_eq!(
            AgentInstance::detect_read_image_mime(b"\x89PNG\r\n\x1A\nrest"),
            Some("image/png")
        );
        assert_eq!(
            AgentInstance::detect_read_image_mime(b"\xFF\xD8\xFFrest"),
            Some("image/jpeg")
        );
        assert_eq!(
            AgentInstance::detect_read_image_mime(b"GIF89arest"),
            Some("image/gif")
        );
        assert_eq!(
            AgentInstance::detect_read_image_mime(b"RIFFxxxxWEBPrest"),
            Some("image/webp")
        );
        assert_eq!(AgentInstance::detect_read_image_mime(b"not image"), None);
    }
}
