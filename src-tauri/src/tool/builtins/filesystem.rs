use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};
use crate::eol::{apply_line_ending, normalize_lf, resolve_preferred_line_ending};
use tauri::Manager;
use tokio::io::AsyncWriteExt;

fn knowledge_registry_for_context(
    ctx: &crate::tool::ToolExecutionContext,
) -> Option<crate::knowledge_source_registry::KnowledgeSourceRegistry> {
    let working_dir = ctx.working_dir.as_deref()?;
    let app_knowledge_dir = ctx
        .app_handle
        .as_ref()
        .and_then(|handle| handle.try_state::<crate::commands::AppKnowledgeDir>())
        .and_then(|state| state.0.as_ref().as_ref().cloned());
    Some(
        crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
            working_dir,
            app_knowledge_dir.as_ref(),
        ),
    )
}

fn knowledge_l1_summary_for_read(
    ctx: &crate::tool::ToolExecutionContext,
    file_path: &str,
    content: &str,
) -> Option<String> {
    let working_dir = ctx.working_dir.as_deref()?;
    let registry = knowledge_registry_for_context(ctx)?;
    let target = registry.classify_path_string(file_path)?;

    if let Ok(document) =
        crate::knowledge_store::read_document_from_file(target.physical_path.as_path())
    {
        if let Some(summary) = document.summary.map(|value| value.trim().to_string()) {
            if !summary.is_empty() {
                return Some(summary);
            }
        }
    }

    if let Some(summary) = super::read_outline::markdown_section_text(content, "L1") {
        return Some(summary);
    }

    if target.doc_type != crate::knowledge_store::KnowledgeType::Skill {
        return None;
    }

    let app_knowledge_dir = ctx
        .app_handle
        .as_ref()
        .and_then(|handle| handle.try_state::<crate::commands::AppKnowledgeDir>())
        .and_then(|state| state.0.as_ref().as_ref().cloned());
    crate::commands::execute_knowledge_read_request(
        working_dir,
        app_knowledge_dir.as_ref(),
        crate::knowledge_store::KnowledgeReadRequest {
            kind: crate::knowledge_store::KnowledgeTargetKind::Document,
            path: target.logical_path,
            doc_type: Some(target.doc_type),
            part: Some("summary".to_string()),
            include_history: false,
        },
    )
    .ok()?
    .document?
    .document
    .summary
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
}

fn format_generated_knowledge_frontmatter(
    resolved: &crate::knowledge_source_registry::ResolvedKnowledgePath,
    prepared: &crate::knowledge_store::PreparedGenericKnowledgeWrite,
) -> String {
    format!(
        "\nKnowledge document registered\n  path: {}\n  physicalPath: {}\n  contentStartLine: {}\nGenerated frontmatter:\n---\n{}---",
        resolved.display_path,
        resolved.physical_path.to_string_lossy().replace('\\', "/"),
        prepared.content_start_line,
        prepared.frontmatter
    )
}

fn has_complete_frontmatter(content: &str) -> bool {
    let normalized = normalize_lf(content);
    let content = normalized.trim_start_matches('\u{feff}');
    content.starts_with("---\n") && content[4..].contains("\n---\n")
}

fn prepare_missing_knowledge_frontmatter(
    ctx: &crate::tool::ToolExecutionContext,
    target: Option<&crate::knowledge_source_registry::ResolvedKnowledgePath>,
    content: &str,
) -> Result<Option<crate::knowledge_store::PreparedGenericKnowledgeWrite>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    if target.kind != crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
        || has_complete_frontmatter(content)
    {
        return Ok(None);
    }
    let working_dir = ctx
        .working_dir
        .as_deref()
        .ok_or_else(|| "Knowledge frontmatter generation requires a workspace".to_string())?;
    crate::knowledge_store::prepare_generic_knowledge_write(
        working_dir,
        target.doc_type,
        &target.logical_path,
        content,
    )
    .map(Some)
}

async fn sync_written_knowledge(
    ctx: &crate::tool::ToolExecutionContext,
    target: Option<&crate::knowledge_source_registry::ResolvedKnowledgePath>,
) -> Option<String> {
    let target = target?;
    let working_dir = ctx.working_dir.as_deref()?;
    let app_handle = ctx.app_handle.as_ref()?;
    let Some(state) =
        app_handle.try_state::<std::sync::Arc<crate::knowledge_index::KnowledgeIndexState>>()
    else {
        return Some("Knowledge index: filesystem watcher pending".to_string());
    };

    let result = if target.kind
        == crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
    {
        crate::commands::sync_visible_document_for_path(
            app_handle,
            working_dir,
            state.inner().clone(),
            target.doc_type,
            &target.logical_path,
        )
        .await
    } else {
        crate::commands::reconcile_and_emit_knowledge_changed(
            app_handle,
            working_dir,
            state.inner().clone(),
            "generic_file_tool",
        )
        .await
    };

    match result {
        Ok(()) => {
            if target.kind
                == crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
            {
                crate::commands::emit_knowledge_changed(
                    app_handle,
                    working_dir,
                    "generic_file_tool",
                );
            }
            Some("Knowledge index: updated".to_string())
        }
        Err(error) => Some(format!(
            "Knowledge index: immediate update failed; filesystem watcher will retry ({error})"
        )),
    }
}

// ─── read ───────────────────────────────────────────────────────────────────

pub(super) fn read() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::READ);
    ToolDef {
        name: "read".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        };
                    }
                };

                let outline = match args.get("outline") {
                    None => false,
                    Some(value) => match value.as_bool() {
                        Some(value) => value,
                        None => {
                            return ToolResult {
                                output: "Parameter 'outline' must be a boolean".to_string(),
                                is_error: true,
                            };
                        }
                    },
                };

                let offset = args
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1)
                    .max(1) as usize;
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

                let path = std::path::Path::new(&file_path);

                let metadata = match tokio::fs::metadata(&file_path).await {
                    Ok(m) => m,
                    Err(_) => {
                        let hint = if let Some(parent) = path.parent() {
                            if let Some(base) = path.file_name().and_then(|n| n.to_str()) {
                                let base_lower = base.to_lowercase();
                                match tokio::fs::read_dir(parent).await {
                                    Ok(mut entries) => {
                                        let mut suggestions = Vec::new();
                                        while let Ok(Some(entry)) = entries.next_entry().await {
                                            let name =
                                                entry.file_name().to_string_lossy().to_string();
                                            let name_lower = name.to_lowercase();
                                            if name_lower.contains(&base_lower)
                                                || base_lower.contains(&name_lower)
                                            {
                                                suggestions
                                                    .push(parent.join(&name).display().to_string());
                                                if suggestions.len() >= 3 {
                                                    break;
                                                }
                                            }
                                        }
                                        if suggestions.is_empty() {
                                            String::new()
                                        } else {
                                            format!("\n\nDid you mean:\n{}", suggestions.join("\n"))
                                        }
                                    }
                                    Err(_) => String::new(),
                                }
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                        return ToolResult {
                            output: format!("File not found: {}{}", file_path, hint),
                            is_error: true,
                        };
                    }
                };

                if metadata.is_dir() {
                    ToolResult {
                        output: format!(
                            "Cannot read directory '{}': the read tool only reads files. Use the list tool for directories.",
                            file_path
                        ),
                        is_error: true,
                    }
                } else {
                    let outline_kind = if outline {
                        match super::read_outline::ReadOutlineKind::from_path(&file_path) {
                            Some(kind) => Some(kind),
                            None => {
                                return ToolResult {
                                    output: format!(
                                        "Outline mode does not support '{}'. Supported file types: C# (.cs) and Markdown (.md).",
                                        file_path
                                    ),
                                    is_error: true,
                                };
                            }
                        }
                    } else {
                        None
                    };

                    if ctx.should_redirect_unity_asset_read(&file_path) {
                        return ToolResult {
                            output: format!(
                                "Direct raw reads are disabled for Unity YAML asset '{}'. Use `unity_yaml_read` with an asset-qualified Property Tree `path`, then continue with returned child paths; use `unity_yaml_search` to locate a precise path.",
                                file_path
                            ),
                            is_error: true,
                        };
                    }

                    if is_binary_extension(&file_path) {
                        return ToolResult {
                            output: format!("Cannot read binary file: {}", file_path),
                            is_error: true,
                        };
                    }

                    match tokio::fs::read_to_string(&file_path).await {
                        Ok(content) => {
                            let normalized_content = normalize_lf(&content);
                            if let Some(kind) = outline_kind {
                                let l1_summary = knowledge_l1_summary_for_read(
                                    &ctx,
                                    &file_path,
                                    &normalized_content,
                                );
                                return match super::read_outline::render_outline(
                                    &file_path,
                                    &normalized_content,
                                    kind,
                                    l1_summary.as_deref(),
                                ) {
                                    Ok(output) => ToolResult {
                                        output,
                                        is_error: false,
                                    },
                                    Err(error) => ToolResult {
                                        output: error,
                                        is_error: true,
                                    },
                                };
                            }
                            let lines: Vec<&str> = normalized_content.lines().collect();
                            let total = lines.len();

                            if total < offset && !(total == 0 && offset == 1) {
                                return ToolResult {
                                    output: format!(
                                        "Offset {} is out of range (file has {} lines)",
                                        offset, total
                                    ),
                                    is_error: true,
                                };
                            }

                            let start = (offset - 1).min(total);
                            let end = (start + limit).min(total);
                            let selected = &lines[start..end];

                            let max_bytes: usize = 50 * 1024;
                            let mut bytes = 0;
                            let mut truncated_by_bytes = false;
                            let mut result_lines = Vec::new();

                            for (index, line) in selected.iter().enumerate() {
                                let display = if line.len() > 2000 {
                                    format!(
                                        "{}... (line truncated to 2000 chars)",
                                        truncate_utf8_prefix(line, 2000)
                                    )
                                } else {
                                    line.to_string()
                                };
                                // Absolute line numbers let agents feed precise positions to
                                // code tools. Keep the prefix compact because this output is
                                // added to model context.
                                let line_str = format!("{}\t{}", start + index + 1, display);
                                bytes += line_str.len() + 1;
                                if bytes > max_bytes {
                                    truncated_by_bytes = true;
                                    break;
                                }
                                result_lines.push(line_str);
                            }

                            let last_read_line = start + result_lines.len();
                            let has_more = end < total || truncated_by_bytes;

                            let mut output = format!("<content>\n{}", result_lines.join("\n"));

                            if truncated_by_bytes {
                                output.push_str(&format!(
                                    "\n\n(Output capped at 50KB. Showing lines {}-{}. Use offset={} to continue.)",
                                    offset, last_read_line, last_read_line + 1
                                ));
                            } else if has_more {
                                output.push_str(&format!(
                                    "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
                                    offset,
                                    last_read_line,
                                    total,
                                    last_read_line + 1
                                ));
                            } else {
                                output.push_str(&format!(
                                    "\n\n(End of file — {} lines total)",
                                    total
                                ));
                            }
                            output.push_str("\n</content>");

                            ToolResult {
                                output,
                                is_error: false,
                            }
                        }
                        Err(e) => ToolResult {
                            output: format!("Failed to read file '{}': {}", file_path, e),
                            is_error: true,
                        },
                    }
                }
            })
        }),
    }
}

pub(crate) fn is_binary_extension(filepath: &str) -> bool {
    let binary_exts = [
        ".zip", ".tar", ".gz", ".exe", ".dll", ".so", ".class", ".jar", ".7z", ".bin", ".wasm",
        ".pyc", ".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".mp3", ".mov",
    ];
    let lower = filepath.to_lowercase();
    binary_exts.iter().any(|ext| lower.ends_with(ext))
}

async fn append_unity_csharp_status(
    output: String,
    working_dir: Option<&str>,
    file_path: &str,
) -> String {
    let Some(project) = working_dir else {
        return output;
    };
    match crate::unity_hotreload::coordinator::format_pending_edit_status(project, file_path).await
    {
        Some(status) if !status.trim().is_empty() => format!("{output}\n\n{status}"),
        _ => output,
    }
}

async fn create_new_file(file_path: &str, content: &[u8]) -> Result<(), std::io::Error> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file_path)
        .await?;
    if let Err(error) = file.write_all(content).await {
        drop(file);
        let _ = tokio::fs::remove_file(file_path).await;
        return Err(error);
    }
    file.flush().await
}

async fn ensure_edit_base_is_current(file_path: &str, expected: &str) -> Result<(), String> {
    let current = tokio::fs::read_to_string(file_path)
        .await
        .map_err(|error| format!("Failed to re-read file '{}': {}", file_path, error))?;
    if current == expected {
        return Ok(());
    }
    eprintln!(
        "[FilesystemEdit] conflict path={} expected_bytes={} current_bytes={}",
        file_path,
        expected.len(),
        current.len()
    );
    Err(format!(
        "Edit conflict: '{}' changed after this edit read it. No content was written. Read the current file and retry the replacement.",
        file_path
    ))
}

async fn replace_file_atomically(
    file_path: &str,
    content: &[u8],
    expected_base: Option<&str>,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(file_path);
    let parent = path
        .parent()
        .ok_or_else(|| format!("File path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("File path has no file name: {}", path.display()))?
        .to_string_lossy();
    let temp_path = parent.join(format!(".{}.locus-{}.tmp", file_name, uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, content)
        .await
        .map_err(|error| format!("Failed to write temporary edit file: {}", error))?;

    if let Ok(metadata) = tokio::fs::metadata(&path).await {
        if let Err(error) = tokio::fs::set_permissions(&temp_path, metadata.permissions()).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(format!(
                "Failed to preserve permissions for '{}': {}",
                file_path, error
            ));
        }
    }

    if let Some(expected) = expected_base {
        if let Err(error) = ensure_edit_base_is_current(file_path, expected).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(error);
        }
    }

    #[cfg(target_os = "windows")]
    let replace_result = {
        let temp_path_for_replace = temp_path.clone();
        let target_path_for_replace = path.clone();
        tokio::task::spawn_blocking(move || {
            use std::os::windows::ffi::OsStrExt;
            use windows::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            use windows_core::PCWSTR;

            let temp_wide = temp_path_for_replace
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let target_wide = target_path_for_replace
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            unsafe {
                MoveFileExW(
                    PCWSTR(temp_wide.as_ptr()),
                    PCWSTR(target_wide.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("Atomic edit replace task failed: {}", error))?
    };

    #[cfg(not(target_os = "windows"))]
    let replace_result = tokio::fs::rename(&temp_path, &path)
        .await
        .map_err(|error| error.to_string());

    if let Err(error) = replace_result {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(format!(
            "Failed to atomically replace '{}': {}",
            file_path, error
        ));
    }
    Ok(())
}

// ─── write ──────────────────────────────────────────────────────────────────

pub(super) fn write() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::WRITE);
    ToolDef {
        name: "write".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        };
                    }
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: content".to_string(),
                            is_error: true,
                        };
                    }
                };

                match tokio::fs::metadata(&file_path).await {
                    Ok(metadata) => {
                        let target_kind = if metadata.is_dir() {
                            "directory"
                        } else {
                            "file"
                        };
                        return ToolResult {
                            output: format!(
                                "Path already exists: {} ({})\nUse the write tool only for new files. Use edit for existing files.",
                                file_path, target_kind
                            ),
                            is_error: true,
                        };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return ToolResult {
                            output: format!("Failed to access path '{}': {}", file_path, error),
                            is_error: true,
                        };
                    }
                }

                let knowledge_target = knowledge_registry_for_context(&ctx)
                    .and_then(|registry| registry.classify_path_string(&file_path));
                if let Some(target) = knowledge_target.as_ref() {
                    if !target.mutability.is_writable() {
                        return ToolResult {
                            output: format!(
                                "Knowledge source is {} and cannot be written: {}",
                                target.mutability.label(),
                                target.display_path
                            ),
                            is_error: true,
                        };
                    }
                }

                let prepared_knowledge = match (
                    ctx.working_dir.as_deref(),
                    knowledge_target.as_ref(),
                ) {
                    (Some(working_dir), Some(target))
                        if target.kind
                            == crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge =>
                    {
                        let parent_path = std::path::Path::new(&target.logical_path)
                            .parent()
                            .map(|path| path.to_string_lossy().replace('\\', "/"))
                            .filter(|path| !path.is_empty() && path != ".");
                        if let Err(error) =
                            crate::commands::ensure_parent_directory_allows_create(
                                working_dir,
                                target.doc_type,
                                parent_path.as_deref(),
                                crate::knowledge_store::KnowledgeTargetKind::Document,
                            )
                        {
                            return ToolResult {
                                output: error,
                                is_error: true,
                            };
                        }
                        match crate::knowledge_store::prepare_generic_knowledge_write(
                            working_dir,
                            target.doc_type,
                            &target.logical_path,
                            &content,
                        ) {
                            Ok(prepared) => Some(prepared),
                            Err(error) => {
                                return ToolResult {
                                    output: format!(
                                        "Failed to generate knowledge frontmatter for '{}': {}",
                                        target.display_path, error
                                    ),
                                    is_error: true,
                                };
                            }
                        }
                    }
                    _ => None,
                };
                let content_to_write = prepared_knowledge
                    .as_ref()
                    .map(|prepared| prepared.content.as_str())
                    .unwrap_or(content.as_str());

                if let Some(parent) = std::path::Path::new(&file_path).parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return ToolResult {
                            output: format!("Failed to create directory: {}", e),
                            is_error: true,
                        };
                    }
                }

                match create_new_file(&file_path, content_to_write.as_bytes()).await {
                    Ok(()) => {
                        // Hot reload tracks the pre-edit baseline of every
                        // touched .cs source; a brand-new file's baseline is
                        // empty (all of it is "new types").
                        if let Some(project) = ctx.working_dir.as_deref() {
                            crate::unity_hotreload::coordinator::note_cs_written(
                                project,
                                &file_path,
                                String::new(),
                            )
                            .await;
                            crate::workspace::note_unity_test_source_written(project, &file_path);
                        }
                        let mut base_output = format!("Created {}", file_path);
                        if let (Some(target), Some(prepared)) =
                            (knowledge_target.as_ref(), prepared_knowledge.as_ref())
                        {
                            base_output.push_str(&format_generated_knowledge_frontmatter(
                                target, prepared,
                            ));
                        }
                        if let Some(sync_status) =
                            sync_written_knowledge(&ctx, knowledge_target.as_ref()).await
                        {
                            base_output.push('\n');
                            base_output.push_str(&sync_status);
                        }
                        let output = append_unity_csharp_status(
                            base_output,
                            ctx.working_dir.as_deref(),
                            &file_path,
                        )
                        .await;
                        ToolResult {
                            output,
                            is_error: false,
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => ToolResult {
                        output: format!(
                            "Path already exists: {} (file)\nUse the write tool only for new files. Use edit for existing files.",
                            file_path
                        ),
                        is_error: true,
                    },
                    Err(error) => ToolResult {
                        output: format!("Failed to write file '{}': {}", file_path, error),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── edit ───────────────────────────────────────────────────────────────────

struct EditOp {
    old_string: String,
    new_string: String,
    replace_all: bool,
}

#[derive(Clone)]
struct PlannedReplacement {
    edit_index: usize,
    start: usize,
    end: usize,
}

struct ReplacePlan {
    ranges: Vec<(usize, usize)>,
    match_offset: usize,
}

pub(super) fn edit() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::EDIT);
    ToolDef {
        name: "edit".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        };
                    }
                };

                let metadata = match tokio::fs::metadata(&file_path).await {
                    Ok(m) => m,
                    Err(_) => {
                        return ToolResult {
                            output: format!("File not found: {}", file_path),
                            is_error: true,
                        };
                    }
                };
                if metadata.is_dir() {
                    return ToolResult {
                        output: format!("Path is a directory: {}", file_path),
                        is_error: true,
                    };
                }

                let knowledge_target = knowledge_registry_for_context(&ctx)
                    .and_then(|registry| registry.classify_path_string(&file_path));
                if let Some(target) = knowledge_target.as_ref() {
                    if !target.mutability.is_writable() {
                        return ToolResult {
                            output: format!(
                                "Knowledge source is {} and cannot be edited: {}",
                                target.mutability.label(),
                                target.display_path
                            ),
                            is_error: true,
                        };
                    }
                    if target.kind
                        == crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
                    {
                        if let Some(working_dir) = ctx.working_dir.as_deref() {
                            if let Ok(document) = crate::knowledge_store::load_document_by_path(
                                working_dir,
                                target.doc_type,
                                &target.logical_path,
                            ) {
                                if document.read_only {
                                    return ToolResult {
                                        output: format!(
                                            "Knowledge document is read-only and cannot be edited: {}",
                                            target.display_path
                                        ),
                                        is_error: true,
                                    };
                                }
                                if !crate::knowledge_store::document_allows_ai_edit(&document) {
                                    return ToolResult {
                                        output: format!(
                                            "AI editing is disabled for knowledge document: {}",
                                            target.display_path
                                        ),
                                        is_error: true,
                                    };
                                }
                            }
                        }
                    }
                }

                let content = match tokio::fs::read_to_string(&file_path).await {
                    Ok(c) => c,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Failed to read file '{}': {}", file_path, e),
                            is_error: true,
                        };
                    }
                };
                let file_eol = resolve_preferred_line_ending(
                    ctx.working_dir.as_deref().map(std::path::Path::new),
                    std::path::Path::new(&file_path),
                    Some(&content),
                );

                // Public calls use `edits`; persisted calls may still carry the
                // former top-level single-edit shape. The scheduler can also
                // coalesce multiple same-file calls into this batch shape.
                let ops: Vec<EditOp> =
                    if let Some(edits_arr) = args.get("edits").and_then(|v| v.as_array()) {
                        let mut ops = Vec::with_capacity(edits_arr.len());
                        for (i, edit) in edits_arr.iter().enumerate() {
                            let old_s = match edit.get("oldString").and_then(|v| v.as_str()) {
                                Some(s) => s.to_string(),
                                None => {
                                    return ToolResult {
                                        output: format!(
                                            "edits[{}]: missing required field 'oldString'",
                                            i
                                        ),
                                        is_error: true,
                                    };
                                }
                            };
                            let new_s = match edit.get("newString").and_then(|v| v.as_str()) {
                                Some(s) => s.to_string(),
                                None => {
                                    return ToolResult {
                                        output: format!(
                                            "edits[{}]: missing required field 'newString'",
                                            i
                                        ),
                                        is_error: true,
                                    };
                                }
                            };
                            let repl_all = edit
                                .get("replaceAll")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            ops.push(EditOp {
                                old_string: normalize_lf(&old_s),
                                new_string: normalize_lf(&new_s),
                                replace_all: repl_all,
                            });
                        }
                        if ops.is_empty() {
                            return ToolResult {
                                output: "edits array is empty".to_string(),
                                is_error: true,
                            };
                        }
                        ops
                    } else {
                        let old_string = match args.get("oldString").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => {
                                return ToolResult {
                                    output: "Missing required parameter: oldString".to_string(),
                                    is_error: true,
                                }
                            }
                        };
                        let new_string = match args.get("newString").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => {
                                return ToolResult {
                                    output: "Missing required parameter: newString".to_string(),
                                    is_error: true,
                                }
                            }
                        };
                        let replace_all = args
                            .get("replaceAll")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        vec![EditOp {
                            old_string: normalize_lf(&old_string),
                            new_string: normalize_lf(&new_string),
                            replace_all,
                        }]
                    };

                let original_content = normalize_lf(&content);
                let mut start_lines: Vec<usize> = Vec::new();
                let mut planned_replacements: Vec<PlannedReplacement> = Vec::new();
                let mut full_replacement: Option<String> = None;

                for (i, op) in ops.iter().enumerate() {
                    if op.old_string == op.new_string {
                        return ToolResult {
                            output: format!(
                                "Edit {}: oldString and newString are identical, no changes to apply.",
                                i + 1
                            ),
                            is_error: true,
                        };
                    }

                    if op.old_string.is_empty() {
                        if ops.len() > 1 {
                            return ToolResult {
                                output: format!(
                                    "Edit {} of {} failed: an empty oldString replaces the entire file and cannot be combined with other edits. Batch edits must be independent; no changes were applied.",
                                    i + 1,
                                    ops.len()
                                ),
                                is_error: true,
                            };
                        }
                        full_replacement = Some(op.new_string.clone());
                        start_lines.push(1);
                        continue;
                    }

                    match plan_replace(&original_content, &op.old_string, op.replace_all) {
                        Ok(plan) => {
                            let line_no =
                                line_number_at_offset(&original_content, plan.match_offset);
                            start_lines.push(line_no);
                            planned_replacements.extend(plan.ranges.into_iter().map(
                                |(start, end)| PlannedReplacement {
                                    edit_index: i,
                                    start,
                                    end,
                                },
                            ));
                        }
                        Err(e) => {
                            return ToolResult {
                                output: if ops.len() > 1 {
                                    format!(
                                        "Edit {} of {} failed against the original file: {} Batch edits must be independent and cannot depend on another edit's output. No changes were applied.",
                                        i + 1,
                                        ops.len(),
                                        e
                                    )
                                } else {
                                    e
                                },
                                is_error: true,
                            };
                        }
                    }
                }

                let current_content = match full_replacement {
                    Some(content) => content,
                    None => match apply_planned_replacements(
                        &original_content,
                        planned_replacements,
                        &ops,
                    ) {
                        Ok(content) => content,
                        Err(error) => {
                            return ToolResult {
                                output: error,
                                is_error: true,
                            };
                        }
                    },
                };
                let applied_count = ops.len();

                let prepared_knowledge = match prepare_missing_knowledge_frontmatter(
                    &ctx,
                    knowledge_target.as_ref(),
                    &current_content,
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Failed to generate knowledge frontmatter for '{}': {}",
                                file_path, error
                            ),
                            is_error: true,
                        };
                    }
                };
                let final_content = prepared_knowledge
                    .as_ref()
                    .map(|prepared| prepared.content.as_str())
                    .unwrap_or(current_content.as_str());
                let rewritten = apply_line_ending(final_content, file_eol);
                match replace_file_atomically(&file_path, rewritten.as_bytes(), Some(&content))
                    .await
                {
                    Ok(()) => {
                        // Baseline for hot reload = the file as it was when
                        // the loaded assemblies were compiled (first edit's
                        // pre-content wins inside the coordinator).
                        if let Some(project) = ctx.working_dir.as_deref() {
                            crate::unity_hotreload::coordinator::note_cs_written(
                                project,
                                &file_path,
                                content.clone(),
                            )
                            .await;
                            crate::workspace::note_unity_test_source_written(project, &file_path);
                        }
                        let lines_info = if !start_lines.is_empty() {
                            let nums: Vec<String> =
                                start_lines.iter().map(|n| n.to_string()).collect();
                            format!(" [lines:{}]", nums.join(","))
                        } else {
                            String::new()
                        };
                        let mut output = if applied_count > 1 {
                            format!(
                                "Edited {} ({} edits applied){}",
                                file_path, applied_count, lines_info
                            )
                        } else {
                            format!("Edited {}{}", file_path, lines_info)
                        };
                        if let (Some(target), Some(prepared)) =
                            (knowledge_target.as_ref(), prepared_knowledge.as_ref())
                        {
                            output.push_str(&format_generated_knowledge_frontmatter(
                                target, prepared,
                            ));
                        }
                        if let Some(sync_status) =
                            sync_written_knowledge(&ctx, knowledge_target.as_ref()).await
                        {
                            output.push('\n');
                            output.push_str(&sync_status);
                        }
                        let output = append_unity_csharp_status(
                            output,
                            ctx.working_dir.as_deref(),
                            &file_path,
                        )
                        .await;
                        ToolResult {
                            output,
                            is_error: false,
                        }
                    }
                    Err(e) => ToolResult {
                        output: format!("Failed to write file '{}': {}", file_path, e),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

fn plan_replace(content: &str, old_string: &str, replace_all: bool) -> Result<ReplacePlan, String> {
    fn plan_matched(
        content: &str,
        matched: &str,
        replace_all: bool,
    ) -> Result<ReplacePlan, String> {
        let positions: Vec<usize> = content.match_indices(matched).map(|(pos, _)| pos).collect();
        if positions.is_empty() {
            return Err("Internal error: fuzzy match could not be located in content.".to_string());
        }
        if !replace_all && positions.len() > 1 {
            let display_limit = 20;
            let mut line_numbers: Vec<String> = positions
                .iter()
                .take(display_limit)
                .map(|pos| line_number_at_offset(content, *pos).to_string())
                .collect();
            if positions.len() > display_limit {
                line_numbers.push("...".to_string());
            }
            Err(format!(
                    "Found multiple matches for oldString at lines: {}. Provide more surrounding context to make it unique.",
                    line_numbers.join(", ")
                ))
        } else {
            let selected = if replace_all {
                positions
            } else {
                vec![positions[0]]
            };
            let match_offset = selected[0];
            Ok(ReplacePlan {
                ranges: selected
                    .into_iter()
                    .map(|start| (start, start + matched.len()))
                    .collect(),
                match_offset,
            })
        }
    }

    if content.contains(old_string) {
        return plan_matched(content, old_string, replace_all);
    }

    if let Some(matched) = line_trimmed_match(content, old_string) {
        return plan_matched(content, &matched, replace_all);
    }

    if let Some(matched) = whitespace_normalized_match(content, old_string) {
        return plan_matched(content, &matched, replace_all);
    }

    let trimmed = old_string.trim();
    if trimmed != old_string && content.contains(trimmed) {
        return plan_matched(content, trimmed, replace_all);
    }

    Err(
        "Could not find oldString in the file. It must match exactly, including whitespace, indentation, and line endings."
            .to_string(),
    )
}

fn apply_planned_replacements(
    content: &str,
    mut replacements: Vec<PlannedReplacement>,
    ops: &[EditOp],
) -> Result<String, String> {
    replacements.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then(left.end.cmp(&right.end))
            .then(left.edit_index.cmp(&right.edit_index))
    });

    for pair in replacements.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if current.start < previous.end {
            return Err(format!(
                "Edits {} and {} target overlapping ranges in the original file near lines {} and {}. Batch edits must be independent; no changes were applied.",
                previous.edit_index + 1,
                current.edit_index + 1,
                line_number_at_offset(content, previous.start),
                line_number_at_offset(content, current.start)
            ));
        }
    }

    let mut result = String::with_capacity(content.len());
    let mut cursor = 0;
    for replacement in replacements {
        result.push_str(&content[cursor..replacement.start]);
        result.push_str(&ops[replacement.edit_index].new_string);
        cursor = replacement.end;
    }
    result.push_str(&content[cursor..]);
    Ok(result)
}

fn line_number_at_offset(content: &str, offset: usize) -> usize {
    content[..offset].matches('\n').count() + 1
}

fn line_trimmed_match(content: &str, find: &str) -> Option<String> {
    let line_sep = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let orig: Vec<&str> = content.lines().collect();
    let mut search: Vec<&str> = find.lines().collect();
    if search.last().map(|l| l.is_empty()).unwrap_or(false) {
        search.pop();
    }
    if search.is_empty() {
        return None;
    }
    if search.len() > orig.len() {
        return None;
    }
    for i in 0..=orig.len().saturating_sub(search.len()) {
        if search.iter().enumerate().all(|(j, l)| {
            orig.get(i + j)
                .map(|o| o.trim() == l.trim())
                .unwrap_or(false)
        }) {
            let matched: Vec<&str> = orig[i..i + search.len()].to_vec();
            return Some(matched.join(line_sep));
        }
    }
    None
}

fn whitespace_normalized_match(content: &str, find: &str) -> Option<String> {
    let normalize = |t: &str| -> String { t.split_whitespace().collect::<Vec<&str>>().join(" ") };
    let nf = normalize(find);

    for line in content.lines() {
        if normalize(line) == nf {
            return Some(line.to_string());
        }
    }

    let line_sep = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let fl: Vec<&str> = find.lines().collect();
    if fl.len() <= 1 {
        return None;
    }
    let cl: Vec<&str> = content.lines().collect();
    if fl.len() > cl.len() {
        return None;
    }
    for i in 0..=cl.len().saturating_sub(fl.len()) {
        let block = cl[i..i + fl.len()].join(line_sep);
        if normalize(&block) == nf {
            return Some(block);
        }
    }
    None
}

// ─── list ───────────────────────────────────────────────────────────────────

pub(super) fn list() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::LIST);
    ToolDef {
        name: "list".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let root_path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string());
                let root_path = match root_path {
                    Some(path) => path,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: path".to_string(),
                            is_error: true,
                        };
                    }
                };

                let max_depth = args
                    .get("depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2)
                    .min(5) as usize;

                let max_items = args
                    .get("max_items")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(500)
                    .min(1000) as usize;

                let max_total = args
                    .get("max_total")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1000)
                    .min(5000) as usize;

                let include_files = args
                    .get("include_files")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let root = std::path::PathBuf::from(&root_path);
                if !root.is_dir() {
                    return ToolResult {
                        output: format!("Directory not found: {}", root_path),
                        is_error: true,
                    };
                }

                let mut output = String::new();
                let mut total_count: usize = 0;
                list_dir_recursive(
                    &root,
                    &root,
                    0,
                    max_depth,
                    max_items,
                    max_total,
                    include_files,
                    &mut total_count,
                    &mut output,
                );

                if output.is_empty() {
                    output = "(empty directory)".to_string();
                }

                ToolResult {
                    output,
                    is_error: false,
                }
            })
        }),
    }
}

fn list_dir_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    current_depth: usize,
    max_depth: usize,
    max_items: usize,
    max_total: usize,
    include_files: bool,
    total_count: &mut usize,
    output: &mut String,
) {
    if *total_count >= max_total {
        return;
    }

    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    let mut dirs: Vec<std::fs::DirEntry> = Vec::new();
    let mut files: Vec<std::fs::DirEntry> = Vec::new();
    for entry in entries {
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if super::should_skip_generated_root_entry(base, &entry.path()) {
                    continue;
                }
                dirs.push(entry);
            } else if include_files {
                let name = entry.file_name();
                if !name.to_string_lossy().ends_with(".meta") {
                    files.push(entry);
                }
            }
        }
    }

    let indent = "  ".repeat(current_depth);
    let total = dirs.len() + files.len();
    let mut shown = 0;

    for entry in &dirs {
        if shown >= max_items || *total_count >= max_total {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("{}{}/", indent, name));
        shown += 1;
        *total_count += 1;

        if current_depth + 1 < max_depth {
            list_dir_recursive(
                base,
                &entry.path(),
                current_depth + 1,
                max_depth,
                max_items,
                max_total,
                include_files,
                total_count,
                output,
            );
        }
    }

    for entry in &files {
        if shown >= max_items || *total_count >= max_total {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("{}{}", indent, name));
        shown += 1;
        *total_count += 1;
    }

    let actually_shown = shown;
    if total > actually_shown {
        if !output.is_empty() {
            output.push('\n');
        }
        if *total_count >= max_total {
            output.push_str(&format!(
                "{}... (total limit reached, {} more in this dir)",
                indent,
                total - actually_shown
            ));
        } else {
            output.push_str(&format!(
                "{}... and {} more",
                indent,
                total - actually_shown
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{edit, list, read, write};
    use crate::process_util::command;
    use crate::tool::ToolExecutionContext;
    use serde_json::json;
    use tempfile::tempdir;

    fn git(cwd: &std::path::Path, args: &[&str]) {
        let output = command("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn write_creates_new_file_when_path_does_not_exist() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("Assets/Scripts/NewFile.cs");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "public class NewFile {}\n"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("Created"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read created file"),
            "public class NewFile {}\n"
        );
    }

    #[test]
    fn write_rejects_existing_file_paths() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("existing.txt");
        std::fs::write(&target, "before").expect("seed existing file");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "after"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("Path already exists"));
        assert!(result.output.contains("Use edit for existing files"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            "before"
        );
    }

    #[test]
    fn write_rejects_existing_directory_paths() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("existing-dir");
        std::fs::create_dir_all(&target).expect("create existing dir");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "after"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("Path already exists"));
        assert!(result.output.contains("(directory)"));
    }

    #[test]
    fn concurrent_write_calls_create_same_path_exactly_once() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("race.txt");
        let target_str = target.to_string_lossy().to_string();

        let (first, second) = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                let first_tool = write();
                let second_tool = write();
                tokio::join!(
                    (first_tool.execute)(
                        json!({
                            "filePath": target_str,
                            "content": "first payload\n"
                        }),
                        ToolExecutionContext::default(),
                    ),
                    (second_tool.execute)(
                        json!({
                            "filePath": target.to_string_lossy().to_string(),
                            "content": "second payload\n"
                        }),
                        ToolExecutionContext::default(),
                    )
                )
            });

        assert_ne!(first.is_error, second.is_error);
        let content = std::fs::read_to_string(&target).expect("read winner");
        assert!(content == "first payload\n" || content == "second payload\n");
        let loser = if first.is_error { &first } else { &second };
        assert!(loser.output.contains("Path already exists"));
    }

    #[test]
    fn edit_base_check_rejects_external_change() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("external-change.txt");
        std::fs::write(&target, "current\n").expect("seed file");

        let result = tokio::runtime::Runtime::new().expect("runtime").block_on(
            super::ensure_edit_base_is_current(&target.to_string_lossy(), "stale\n"),
        );

        let error = result.expect_err("stale edit must conflict");
        assert!(error.contains("Edit conflict"));
        assert_eq!(
            std::fs::read_to_string(target).expect("read unchanged file"),
            "current\n"
        );
    }

    #[test]
    fn atomic_replace_leaves_complete_file_and_no_temp_artifact() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("atomic.txt");
        std::fs::write(&target, "before\n").expect("seed file");

        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(super::replace_file_atomically(
                &target.to_string_lossy(),
                b"after\n",
                None,
            ))
            .expect("atomic replace");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read replaced file"),
            "after\n"
        );
        let leftovers = std::fs::read_dir(root.path())
            .expect("list temp dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".locus-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn write_generates_and_reports_workspace_knowledge_frontmatter() {
        let root = tempdir().expect("temp dir");
        let target = root
            .path()
            .join("Locus/knowledge/design/gameplay/new-loop.md");
        let context = ToolExecutionContext {
            working_dir: Some(root.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "content": "# New Loop\n\n## Content\nThis heading remains ordinary Markdown."
                    }),
                    context,
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Generated frontmatter:\n---\n"));
        assert!(result.output.contains("bodyFormat: markdown"));
        assert!(result.output.contains("contentStartLine:"));
        let raw = std::fs::read_to_string(target).expect("read knowledge file");
        assert!(raw.starts_with("---\n"));
        assert!(
            raw.ends_with("# New Loop\n\n## Content\nThis heading remains ordinary Markdown.\n")
        );
    }

    #[test]
    fn edit_adds_and_reports_frontmatter_for_unregistered_plain_knowledge_file() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("Locus/knowledge/memory/context.md");
        std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
        std::fs::write(&target, "Durable preference\n").expect("seed plain markdown");
        let context = ToolExecutionContext {
            working_dir: Some(root.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "Durable preference",
                        "newString": "Updated durable preference"
                    }),
                    context,
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Generated frontmatter:\n---\n"));
        let raw = std::fs::read_to_string(target).expect("read knowledge file");
        assert!(raw.starts_with("---\n"));
        assert!(raw.ends_with("Updated durable preference\n"));
    }

    #[test]
    fn list_skips_generated_root_directories_by_default() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path().join("Assets/Scripts")).expect("create scripts");
        std::fs::create_dir_all(root.path().join("Library")).expect("create library");
        std::fs::create_dir_all(root.path().join("BuildPlayer")).expect("create build output");

        std::fs::write(
            root.path().join("Assets/Scripts/PlayerController.cs"),
            "public class PlayerController : MonoBehaviour {}",
        )
        .expect("write gameplay script");
        std::fs::write(root.path().join("Library/cache.db"), "cached").expect("write cache");
        std::fs::write(root.path().join("BuildPlayer/game.exe"), "binary").expect("write build");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (list().execute)(
                    json!({
                        "path": root.path().to_string_lossy().to_string(),
                        "depth": 3,
                        "include_files": true
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("Assets/"));
        assert!(result.output.contains("PlayerController.cs"));
        assert!(!result.output.contains("Library/"));
        assert!(!result.output.contains("BuildPlayer/"));
    }

    #[test]
    fn list_can_browse_explicit_generated_directory_roots() {
        let root = tempdir().expect("temp dir");
        std::fs::write(root.path().join("cache.db"), "cached").expect("write cache");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (list().execute)(
                    json!({
                        "path": root.path().to_string_lossy().to_string(),
                        "depth": 2,
                        "include_files": true
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("cache.db"));
    }

    #[test]
    fn read_normalizes_crlf_content_to_lf_output() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("crlf.txt");
        std::fs::write(&target, "alpha\r\nbeta\r\n").expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (read().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "offset": 1,
                        "limit": 20
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("<content>\n1\talpha\n2\tbeta"));
        assert!(!result.output.starts_with("<content>\n "));
        assert!(!result.output.contains('\r'));
    }

    #[test]
    fn read_unity_asset_redirect_suggests_unity_execute_script() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("walk.anim");
        std::fs::write(&target, "%YAML 1.1\n").expect("seed Unity asset");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (read().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string()
                    }),
                    ToolExecutionContext {
                        unity_connected: Some(true),
                        ..ToolExecutionContext::default()
                    },
                )
                .await
            });

        assert!(result.is_error);
        assert!(
            result.output.contains(
                "use `unity_execute` to load and inspect the asset with a Unity Editor C# script"
            ),
            "{}",
            result.output
        );
    }

    #[test]
    fn read_outline_returns_markdown_ranges_and_injects_knowledge_l1() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("Locus/knowledge/skill/audit.md");
        std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
        std::fs::write(
            &target,
            "# Audit\n\n## L1\nUse for focused audits.\n\n## Instructions\nInspect the project.\n",
        )
        .expect("seed knowledge markdown");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (read().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "outline": true,
                        "offset": 999,
                        "limit": 1
                    }),
                    ToolExecutionContext {
                        working_dir: Some(root.path().to_string_lossy().to_string()),
                        ..ToolExecutionContext::default()
                    },
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert!(
            result.output.contains("# Audit [lines 1-7]"),
            "{}",
            result.output
        );
        assert!(
            result.output.contains("## L1 [lines 3-5]"),
            "{}",
            result.output
        );
        assert!(
            result
                .output
                .contains("L1 summary (knowledge):\n  Use for focused audits."),
            "{}",
            result.output
        );
        assert!(
            !result.output.contains("Inspect the project."),
            "{}",
            result.output
        );
    }

    #[test]
    fn read_outline_rejects_unsupported_file_types() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("notes.txt");
        std::fs::write(&target, "notes\n").expect("seed text file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (read().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "outline": true
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result
            .output
            .contains("Supported file types: C# (.cs) and Markdown (.md)"));
    }

    #[test]
    fn edit_accepts_lf_old_string_for_crlf_file_and_preserves_crlf() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("player.cs");
        std::fs::write(
            &target,
            "class Player\r\n{\r\n    void Fire()\r\n    {\r\n        Shoot();\r\n    }\r\n}\r\n",
        )
        .expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "    void Fire()\n    {\n        Shoot();\n    }\n",
                        "newString": "    void Fire()\n    {\n        Shoot();\n        Reload();\n    }\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            "class Player\r\n{\r\n    void Fire()\r\n    {\r\n        Shoot();\r\n        Reload();\r\n    }\r\n}\r\n"
        );
    }

    #[test]
    fn edit_batch_applies_non_overlapping_replacements_from_original_snapshot() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("batch.txt");
        std::fs::write(&target, "alpha middle omega\n").expect("seed batch file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "edits": [
                            { "oldString": "omega", "newString": "O" },
                            { "oldString": "alpha", "newString": "ALPHA-LONG" }
                        ]
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            "ALPHA-LONG middle O\n"
        );
    }

    #[test]
    fn edit_batch_rejects_followup_that_only_matches_prior_output() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("batched.txt");
        let original = "before\n";
        std::fs::write(&target, original).expect("seed file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "edits": [
                            { "oldString": "before", "newString": "alpha beta" },
                            { "oldString": "beta", "newString": "BETA" }
                        ]
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result
            .output
            .contains("Edit 2 of 2 failed against the original file"));
        assert!(result
            .output
            .contains("cannot depend on another edit's output"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            original
        );
    }

    #[test]
    fn edit_batch_rejects_overlapping_original_ranges() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("overlap.txt");
        let original = "alpha beta gamma\n";
        std::fs::write(&target, original).expect("seed file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "edits": [
                            { "oldString": "alpha beta", "newString": "first" },
                            { "oldString": "beta gamma", "newString": "second" }
                        ]
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result
            .output
            .contains("Edits 1 and 2 target overlapping ranges"));
        assert!(result.output.contains("no changes were applied"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            original
        );
    }

    #[test]
    fn edit_batch_rejects_full_replacement_combined_with_other_edits() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("full-replacement.txt");
        let original = "before\n";
        std::fs::write(&target, original).expect("seed file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "edits": [
                            { "oldString": "", "newString": "alpha beta\n" },
                            { "oldString": "before", "newString": "after" }
                        ]
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("replaces the entire file"));
        assert!(result
            .output
            .contains("cannot be combined with other edits"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            original
        );
    }

    #[test]
    fn edit_reports_line_numbers_for_multiple_old_string_matches() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("player.cs");
        let original = "class Player\n{\n    return;\n    Tick();\n    return;\n}\n";
        std::fs::write(&target, original).expect("seed repeated file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "    return;",
                        "newString": "    Stop();",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result
            .output
            .contains("Found multiple matches for oldString"));
        assert!(result.output.contains("at lines: 3, 5"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            original
        );
    }

    #[test]
    fn edit_reports_missing_old_string_when_search_block_is_longer_than_file() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("short.txt");
        let original = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        let old_string = (1..=15)
            .map(|index| format!("missing line {}", index))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&target, original).expect("seed short file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": old_string,
                        "newString": "replacement",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("Could not find oldString"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            original
        );
    }

    #[test]
    fn edit_normalizes_mixed_eol_file_to_preferred_style() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("mixed.txt");
        std::fs::write(&target, "alpha\r\nbeta\ngamma\r\n").expect("seed mixed eol file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "alpha\nbeta\ngamma\n",
                        "newString": "alpha\nbeta\ndelta\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            "alpha\r\nbeta\r\ndelta\r\n"
        );
    }

    #[test]
    fn edit_prefers_repo_eol_rule_over_current_file_style() {
        let root = tempdir().expect("temp dir");
        git(root.path(), &["init", "-b", "main"]);
        git(root.path(), &["config", "user.name", "Test User"]);
        git(root.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(root.path().join(".gitattributes"), "*.txt text eol=lf\n")
            .expect("write attributes");

        let target = root.path().join("notes.txt");
        std::fs::write(&target, "alpha\r\nbeta\r\n").expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "alpha\nbeta\n",
                        "newString": "alpha\nbeta\ngamma\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext {
                        working_dir: Some(root.path().to_string_lossy().to_string()),
                        ..ToolExecutionContext::default()
                    },
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read(&target).expect("read edited bytes"),
            b"alpha\nbeta\ngamma\n"
        );
    }
}
