use super::filesystem;
use super::{make_exec, ToolDef, ToolExecutionContext, ToolResult};
use crate::eol::{apply_line_ending, resolve_preferred_line_ending};
use crate::knowledge_source_registry::ResolvedKnowledgePath;
use crate::tool::apply_patch::{from_arguments, update_content, Change};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

struct PreparedChange {
    path: String,
    original: Option<String>,
    content: Option<String>,
    knowledge: Option<ResolvedKnowledgePath>,
    registration: Option<String>,
}

pub(super) fn apply_patch() -> ToolDef {
    let prompt =
        crate::prompt::parse_tool_prompt(include_str!("../../../../tools/apply_patch.json"));
    ToolDef {
        name: "apply_patch".into(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let changes = match prepare(&args, &ctx).await {
                    Ok(changes) => changes,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Patch verification failed: {error}. No files were changed."
                            ),
                            is_error: true,
                        }
                    }
                };
                let mut output = Vec::new();
                for change in changes {
                    if ctx.cancel_rx.as_ref().is_some_and(|rx| *rx.borrow()) {
                        return failure(output, "Patch cancelled".into());
                    }
                    if let Err(error) = commit(&change).await {
                        return failure(output, error);
                    }
                    let kind = match (&change.original, &change.content) {
                        (None, _) => "A",
                        (_, None) => "D",
                        _ => "M",
                    };
                    output.push(format!("{kind} {}", change.path));
                    if let Some(project) = ctx.working_dir.as_deref() {
                        crate::unity_hotreload::coordinator::note_cs_written(
                            project,
                            &change.path,
                            change.original.clone().unwrap_or_default(),
                        )
                        .await;
                        crate::workspace::note_unity_test_source_written(project, &change.path);
                    }
                    if let Some(registration) = change.registration {
                        output.push(registration);
                    }
                    if let Some(status) = filesystem::sync_written_knowledge(
                        &ctx,
                        change.knowledge.as_ref(),
                        if kind == "D" { "delete" } else { "content" },
                    )
                    .await
                    {
                        output.push(status);
                    }
                    let status = filesystem::append_unity_csharp_status(
                        String::new(),
                        ctx.working_dir.as_deref(),
                        &change.path,
                    )
                    .await;
                    if !status.trim().is_empty() {
                        output.push(status);
                    }
                }
                ToolResult {
                    output: format!(
                        "Success. Updated the following files:\n{}",
                        output.join("\n")
                    ),
                    is_error: false,
                }
            })
        }),
    }
}

fn failure(output: Vec<String>, error: String) -> ToolResult {
    let committed = if output.is_empty() {
        "No file changes were committed.".into()
    } else {
        format!("Earlier changes remain applied:\n{}", output.join("\n"))
    };
    ToolResult {
        output: format!("{error}\n{committed}"),
        is_error: true,
    }
}

// Resolve aliases before checking duplicate targets. Missing paths resolve via
// their nearest existing ancestor, so a symlinked parent cannot hide aliases.
fn path_key(path: &str) -> Result<String, String> {
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let mut lexical = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            _ => lexical.push(part.as_os_str()),
        }
    }
    let mut parent = lexical.as_path();
    let mut suffix = Vec::new();
    while !parent.exists() {
        if let Some(name) = parent.file_name() {
            suffix.push(name.to_os_string());
        }
        parent = parent.parent().ok_or("Path has no existing ancestor")?;
    }
    let mut resolved = dunce::canonicalize(parent).map_err(|error| error.to_string())?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    let key = resolved.to_string_lossy().into_owned();
    Ok(if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    })
}

fn knowledge_target(
    ctx: &ToolExecutionContext,
    path: &str,
    exists: bool,
) -> Result<Option<ResolvedKnowledgePath>, String> {
    let target = filesystem::knowledge_registry_for_context(ctx)
        .and_then(|registry| registry.classify_path_string(path));
    if let Some(error) = filesystem::knowledge_scope_error(ctx, target.as_ref(), "apply_patch") {
        return Err(error.output);
    }
    if let Some(target) = target.as_ref() {
        if !target.mutability.is_writable() {
            return Err(format!(
                "Knowledge source is {}: {}",
                target.mutability.label(),
                target.display_path
            ));
        }
        if exists
            || target.kind
                != crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
        {
            if let Some(document) =
                filesystem::load_knowledge_policy_document(ctx, target, !exists)?
            {
                if document.read_only || !crate::knowledge_store::document_allows_ai_edit(&document)
                {
                    return Err(format!(
                        "Knowledge document cannot be edited: {}",
                        target.display_path
                    ));
                }
            }
        }
    }
    Ok(target)
}

async fn prepare(
    args: &serde_json::Value,
    ctx: &ToolExecutionContext,
) -> Result<Vec<PreparedChange>, String> {
    let files = from_arguments(args)?;
    let mut targets = HashSet::new();
    let mut prepared = Vec::new();
    for file in files {
        for (raw, _) in file.targets() {
            let path = filesystem::resolve_context_path(ctx, raw);
            if !targets.insert(path_key(&path)?) {
                return Err(format!("Multiple operations target {path}"));
            }
            if tokio::fs::symlink_metadata(&path)
                .await
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(format!(
                    "Patch targets must be regular files, not symbolic links: {path}"
                ));
            }
        }
        let path = filesystem::resolve_context_path(ctx, &file.path);
        let original = if matches!(file.change, Change::Add(_)) {
            if tokio::fs::try_exists(&path)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err(format!("Path already exists: {path}"));
            }
            None
        } else {
            Some(
                tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|error| format!("Failed to read {path}: {error}"))?,
            )
        };
        let knowledge = knowledge_target(ctx, &path, original.is_some())?;
        let content = match file.change {
            Change::Add(content) => Some(content),
            Change::Delete => None,
            Change::Update { move_to, chunks } => {
                let content = update_content(original.as_deref().unwrap(), &chunks)
                    .map_err(|error| format!("{path}: {error}"))?;
                if let Some(destination) = move_to {
                    let destination = filesystem::resolve_context_path(ctx, &destination);
                    if tokio::fs::try_exists(&destination)
                        .await
                        .map_err(|error| error.to_string())?
                    {
                        return Err(format!("Move destination already exists: {destination}"));
                    }
                    let target = knowledge_target(ctx, &destination, false)?;
                    prepared.push(prepare_write(
                        ctx,
                        destination,
                        None,
                        Some(content),
                        target,
                    )?);
                    None
                } else {
                    Some(content)
                }
            }
        };
        prepared.push(prepare_write(ctx, path, original, content, knowledge)?);
    }
    Ok(prepared)
}

fn prepare_write(
    ctx: &ToolExecutionContext,
    path: String,
    original: Option<String>,
    content: Option<String>,
    knowledge: Option<ResolvedKnowledgePath>,
) -> Result<PreparedChange, String> {
    let mut registration = None;
    let content = if let Some(content) = content {
        let prepared =
            filesystem::prepare_missing_knowledge_frontmatter(ctx, knowledge.as_ref(), &content)?;
        if let (Some(target), Some(prepared)) = (knowledge.as_ref(), prepared.as_ref()) {
            registration = Some(filesystem::format_generated_knowledge_frontmatter(
                target, prepared,
            ));
        }
        let content = prepared
            .as_ref()
            .map(|value| value.content.as_str())
            .unwrap_or(&content);
        let ending = resolve_preferred_line_ending(
            ctx.working_dir.as_deref().map(Path::new),
            Path::new(&path),
            original.as_deref(),
        );
        Some(apply_line_ending(content, ending))
    } else {
        None
    };
    Ok(PreparedChange {
        path,
        original,
        content,
        knowledge,
        registration,
    })
}

async fn commit(change: &PreparedChange) -> Result<(), String> {
    match (&change.original, &change.content) {
        (Some(original), Some(content)) => {
            filesystem::replace_file_atomically(&change.path, content.as_bytes(), Some(original))
                .await
        }
        (None, Some(content)) => {
            if let Some(parent) = Path::new(&change.path).parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            filesystem::create_new_file(&change.path, content.as_bytes())
                .await
                .map_err(|error| format!("Failed to create {}: {error}", change.path))
        }
        (Some(original), None) => {
            filesystem::ensure_edit_base_is_current(&change.path, original).await?;
            tokio::fs::remove_file(&change.path)
                .await
                .map_err(|error| format!("Failed to delete {}: {error}", change.path))
        }
        (None, None) => unreachable!(),
    }
}

#[cfg(test)]
#[path = "filesystem_patch_tests.rs"]
mod tests;
