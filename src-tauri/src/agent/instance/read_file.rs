use crate::session::models::ImageData;
use crate::tool::ToolResult;

use super::{AgentInstance, ExecutedToolResult, LazyToolRenderer, ToolRunOutcome};

const MAX_READ_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

impl AgentInstance {
    pub(super) async fn execute_read(
        &self,
        app_handle: &tauri::AppHandle,
        args: &serde_json::Value,
        tool_call_id: &str,
        run_id: &str,
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

        let outline = match args.get("outline") {
            None => false,
            Some(value) => match value.as_bool() {
                Some(value) => value,
                None => {
                    return ExecutedToolResult::from_tool_result(ToolResult {
                        output: "Parameter 'outline' must be a boolean".to_string(),
                        is_error: true,
                    });
                }
            },
        };
        if outline && !Self::is_read_outline_path(file_path) {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: format!(
                    "Outline mode does not support '{}'. Supported file types: C# (.cs) and Markdown (.md).",
                    file_path
                ),
                is_error: true,
            });
        }

        if !Self::is_read_image_path(file_path) {
            let tool_context = self
                .build_tool_execution_context(app_handle, "read", args)
                .await;
            let mut result = self
                .await_tool_result(self.tool_registry.execute_with_context(
                    "read",
                    args,
                    tool_context,
                ))
                .await;
            self.enrich_registered_knowledge_read(
                app_handle,
                file_path,
                tool_call_id,
                run_id,
                &mut result,
            )
            .await;
            return result;
        }

        let metadata = match tokio::fs::metadata(file_path).await {
            Ok(metadata) => metadata,
            Err(_) => {
                let tool_context = self
                    .build_tool_execution_context(app_handle, "read", args)
                    .await;
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
            let tool_context = self
                .build_tool_execution_context(app_handle, "read", args)
                .await;
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

    fn is_read_outline_path(file_path: &str) -> bool {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|value| value.to_str());
        ext.is_some_and(|value| {
            value.eq_ignore_ascii_case("cs") || value.eq_ignore_ascii_case("md")
        })
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

    async fn enrich_registered_knowledge_read(
        &self,
        app_handle: &tauri::AppHandle,
        file_path: &str,
        tool_call_id: &str,
        run_id: &str,
        result: &mut ExecutedToolResult,
    ) {
        if result.outcome != ToolRunOutcome::Done {
            return;
        }

        let registry = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
        );
        let Some(target) = registry.classify_path_string(file_path) else {
            return;
        };
        if target.doc_type != crate::knowledge_store::KnowledgeType::Skill {
            return;
        }

        let response = crate::commands::execute_knowledge_read_request(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
            crate::knowledge_store::KnowledgeReadRequest {
                kind: crate::knowledge_store::KnowledgeTargetKind::Document,
                path: target.logical_path.clone(),
                doc_type: Some(target.doc_type),
                part: Some("full".to_string()),
                include_history: false,
            },
        );
        let Ok(response) = response else {
            return;
        };
        let skill_runtime_context = response.document.as_ref().and_then(|document| {
            crate::skill_runtime_context::for_knowledge_document(
                &document.document,
                crate::skill_runtime_context::SkillRuntimeContextTrigger::Read,
            )
        });
        let tool_names = response
            .document
            .as_ref()
            .map(|document| document.document.tools.clone())
            .unwrap_or_default();
        let activated_tools = self.activate_document_skill_tool_names(&tool_names);
        let mut referenced_tools = Vec::new();
        for name in &tool_names {
            let Some(canonical) = self.canonical_tool_name(name) else {
                continue;
            };
            if Self::is_meta_tool(&canonical) || referenced_tools.contains(&canonical) {
                continue;
            }
            referenced_tools.push(canonical);
        }

        match self
            .compile_skill_package_unity_scripts_for_knowledge_read(
                app_handle,
                tool_call_id,
                run_id,
                &target.logical_path,
            )
            .await
        {
            Ok(Some(note)) => {
                result.output.push_str("\n\n");
                result.output.push_str(&note);
            }
            Ok(None) => {}
            Err(error) => {
                result.output.push_str(&format!(
                    "\n\nLocus Skill runtime: Unity C# scripts are not ready.\n{error}"
                ));
            }
        }

        if let Some(runtime_context) = skill_runtime_context {
            result.output.push_str("\n\n");
            result.output.push_str(&runtime_context);
        }

        if !activated_tools.is_empty() {
            result
                .output
                .push_str("\n\nLoaded Skill document tools for the next step: ");
            result.output.push_str(&activated_tools.join(", "));
        }
        match self.cached_lazy_tool_renderer() {
            LazyToolRenderer::AnthropicNative => {
                crate::llm::tool_references::append_tool_reference_marker(
                    &mut result.output,
                    &referenced_tools,
                );
            }
            LazyToolRenderer::CodexNative if !referenced_tools.is_empty() => {
                result.output.push_str(&format!(
                    "\n\nThe Skill tools above are deferred. Call `{}` with a matching query to load their schemas before use: {}",
                    super::CODEX_TOOL_SEARCH_TOOL_NAME,
                    referenced_tools.join(", ")
                ));
            }
            _ => {}
        }
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
    fn read_outline_path_accepts_only_csharp_and_markdown() {
        assert!(AgentInstance::is_read_outline_path("Assets/Player.CS"));
        assert!(AgentInstance::is_read_outline_path("Docs/guide.md"));
        assert!(!AgentInstance::is_read_outline_path("Docs/guide.txt"));
        assert!(!AgentInstance::is_read_outline_path("Assets/hero.png"));
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
