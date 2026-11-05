use super::*;
use tauri::Manager;

fn action_error(error: impl ToString) -> AppError {
    AppError::new("workspace.file_action_failed", error.to_string())
}

pub(super) fn sibling_file(source: &Path, name: &str) -> Result<PathBuf, AppError> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
    {
        return Err(action_error("Invalid file name"));
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(action_error("Invalid file name"));
    }
    Ok(source.with_file_name(name))
}

pub(super) fn companion_files(path: &Path) -> Result<Vec<PathBuf>, AppError> {
    let csv = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"));
    let suffixes: &[&str] = if csv {
        &["", ".meta", ".view", ".view.meta"]
    } else {
        &["", ".meta"]
    };
    let mut paths = Vec::new();
    for suffix in suffixes {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let entry = PathBuf::from(name);
        match std::fs::symlink_metadata(&entry) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                paths.push(entry)
            }
            Ok(_) => return Err(action_error("File operations require regular files")),
            Err(error) if !suffix.is_empty() && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(action_error(error)),
        }
    }
    Ok(paths)
}

fn relocate_files(source: &Path, target: &Path) -> Result<Vec<(PathBuf, PathBuf)>, AppError> {
    if cfg!(windows) && source != target && path_key(source) == path_key(target) {
        let staging = source.with_file_name(format!(".locus-rename-{}", uuid::Uuid::new_v4()));
        let moved = relocate_files(source, &staging)?;
        // Use the recorded companions: the temporary filename has no CSV extension.
        let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (from, staged) in &moved {
            let suffix = from.to_string_lossy()[source.to_string_lossy().len()..].to_string();
            let mut name = target.as_os_str().to_os_string();
            name.push(suffix);
            let to = PathBuf::from(name);
            if let Err(error) = std::fs::rename(staged, &to) {
                for (staged, to) in completed.iter().rev() {
                    std::fs::rename(to, staged).map_err(action_error)?;
                }
                rollback_files(&moved)?;
                return Err(action_error(error));
            }
            completed.push((staged.clone(), to));
        }
        return Ok(moved
            .into_iter()
            .zip(completed)
            .map(|((from, _), (_, to))| (from, to))
            .collect());
    }
    let files = companion_files(source)?;
    let moves: Vec<_> = files
        .into_iter()
        .map(|from| {
            let suffix = from.as_os_str().to_string_lossy()
                [source.as_os_str().to_string_lossy().len()..]
                .to_string();
            let mut name = target.as_os_str().to_os_string();
            name.push(suffix);
            (from, PathBuf::from(name))
        })
        .collect();
    for (_, to) in &moves {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(action_error(format!(
                "Target already exists: {}",
                to.display()
            )));
        }
    }
    let mut completed = Vec::new();
    for (from, to) in moves {
        if let Err(error) = std::fs::rename(&from, &to) {
            rollback_files(&completed)?;
            return Err(action_error(error));
        }
        completed.push((from, to));
    }
    Ok(completed)
}

fn rollback_files(moves: &[(PathBuf, PathBuf)]) -> Result<(), AppError> {
    for (from, to) in moves.iter().rev() {
        std::fs::rename(to, from).map_err(action_error)?;
    }
    Ok(())
}

/// A workspace reference authorizes workspace files; externally mounted files
/// are authorized against the active project preset instead.
#[tauri::command]
pub async fn explorer_file_action(
    project_id: String,
    path: String,
    new_name: Option<String>,
    workspace_ref: Option<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<Option<String>, AppError> {
    let (project_id, root, layout) =
        resolve_project_layout(registry.inner().as_ref(), project_id, "explorerFileAction")?;
    if let (Some(name), Some(reference)) = (new_name.as_deref(), workspace_ref.as_ref()) {
        let directory = {
            let scope = workspace_file_scope(reference, registry.inner().as_ref())?;
            let directory = super::super::workspace::normalize_workspace_sub_path(&path).ok()
                .filter(|relative| scope.runtime().root().join(relative).is_dir());
            if directory.is_some() && scope.runtime().project_id() != &project_id {
                return Err(action_error("Workspace does not belong to the project"));
            }
            directory
        };
        // Folder renames share the move flow: sidecars, knowledge folders and layout references.
        if let Some(relative) = directory {
            let next = super::workspace_file_move::rename_workspace_entry(reference.clone(), &relative, name, registry, app_handle).await?;
            return Ok(Some(next));
        }
    }
    if let (None, Some(reference)) = (new_name.as_deref(), workspace_ref.as_ref()) {
        let scope = workspace_file_scope(reference, registry.inner().as_ref())?;
        if let Some(directory) = workspace_directory(scope.runtime().root(), &path)? {
            if scope.runtime().project_id() != &project_id {
                return Err(action_error("Workspace does not belong to the project"));
            }
            delete_workspace_directory(&project_id, &layout, reference, scope, directory, registry, app_handle).await?;
            return Ok(None);
        }
    }
    let snapshot =
        crate::workspace_tree::snapshot(&layout, project_id.as_str()).map_err(action_error)?;
    let (scope, source) = if let Some(workspace_ref) = workspace_ref.as_ref() {
        let (scope, scoped_project, source) =
            resolve_workspace_file(workspace_ref, &path, registry.inner().as_ref())?;
        if scoped_project != project_id {
            return Err(action_error("Workspace does not belong to the project"));
        }
        (Some(scope), source)
    } else {
        (
            None,
            ensure_preview_path_authorized(&snapshot, Path::new(&path))?,
        )
    };
    let action_root = scope
        .as_ref()
        .map(|scope| scope.runtime().root())
        .unwrap_or(&root);
    let app_knowledge_dir: State<'_, crate::commands::knowledge::AppKnowledgeDir> =
        app_handle.state();
    let sources = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
        &action_root.to_string_lossy(),
        app_knowledge_dir.0.as_ref().as_ref(),
    );
    if let Some(knowledge) = sources.classify_path(&source) {
        use crate::knowledge_source_registry::KnowledgeSourceKind;
        if knowledge.kind != KnowledgeSourceKind::WorkspaceKnowledge
            || !knowledge.mutability.is_writable()
        {
            return Err(action_error(
                "This knowledge source is managed or read-only",
            ));
        }
        let reference = workspace_ref.ok_or_else(|| {
            action_error("A workspace reference is required for knowledge documents")
        })?;
        if let Some(name) = new_name.as_deref() {
            sibling_file(&source, name)?;
            let new_path = Path::new(&knowledge.logical_path)
                .with_file_name(name.trim())
                .to_string_lossy()
                .replace('\\', "/");
            let result = crate::commands::knowledge_move(
                reference,
                crate::knowledge_store::KnowledgeMoveRequest {
                    kind: crate::knowledge_store::KnowledgeTargetKind::Document,
                    doc_type: Some(knowledge.doc_type),
                    path: knowledge.logical_path,
                    new_path,
                },
                app_handle,
                registry,
            )
            .await?;
            let next = crate::knowledge_store::document_path(
                &action_root.to_string_lossy(),
                result.doc_type,
                result.result_path.as_deref().unwrap_or(&result.path),
            )
            .map_err(action_error)?;
            return Ok(Some(next.to_string_lossy().into_owned()));
        }
        crate::commands::knowledge_delete(
            reference,
            crate::knowledge_store::KnowledgeDeleteRequest {
                kind: crate::knowledge_store::KnowledgeTargetKind::Document,
                doc_type: Some(knowledge.doc_type),
                path: knowledge.logical_path,
            },
            app_handle,
            registry,
        )
        .await?;
        return Ok(None);
    }
    // Keep Markdown saves out of the rename/reference-update transaction.
    let _document_guard = super::super::view_documents::DOCUMENT_WRITE_LOCK.lock().map_err(action_error)?;
    // The common CSV lock also excludes concurrent CSV companion saves.
    let _guard = CSV_FILE_WRITE_LOCK.lock().map_err(action_error)?;
    let target = new_name
        .as_deref()
        .map(|name| sibling_file(&source, name))
        .transpose()?;
    if target.as_deref() == Some(source.as_path()) {
        return Ok(Some(source.to_string_lossy().into_owned()));
    }
    // Stage a deletion beside the source so a failed preset update can restore it.
    let staged = target.clone().unwrap_or_else(|| {
        source.with_file_name(format!(".locus-delete-{}", uuid::Uuid::new_v4()))
    });
    let image_rename = target.is_some() && source.extension().is_some_and(|ext| {
        matches!(ext.to_string_lossy().to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg")
    });
    let mut reference_updates = if image_rename {
        crate::markdown_attachments::image_reference_updates(action_root, &source, target.as_deref().unwrap()).map_err(action_error)?
    } else { Vec::new() };
    if image_rename {
        reference_updates.extend(super::super::markdown_documents::recovery_image_updates(
            &app_handle, action_root, workspace_ref.as_ref().map(|r| r.checkout_id.as_str()), &source, target.as_deref().unwrap(),
        )?);
    }
    let moved = relocate_files(&source, &staged)?;
    if let Err(error) = crate::markdown_attachments::apply_updates(&reference_updates) {
        rollback_files(&moved)?;
        return Err(action_error(error));
    }
    let updated = match crate::workspace_tree::relocate_file_references(
        &layout,
        project_id.as_str(),
        &source,
        target.as_deref(),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            crate::markdown_attachments::rollback_updates(&reference_updates).map_err(action_error)?;
            rollback_files(&moved)?;
            return Err(action_error(error));
        }
    };
    if target.is_none() {
        for (_, path) in &moved {
            std::fs::remove_file(path).map_err(action_error)?;
        }
    }
    crate::workspace_file_order::relocate(action_root, &source, target.as_deref()).map_err(action_error)?;
    emit_file_action(
        &app_handle,
        project_id.as_str(),
        action_root,
        &source,
        target.as_deref(),
    )?;
    if image_rename {
        app_handle.emit("markdown-image-renamed", serde_json::json!({
            "root": action_root, "oldPath": source, "newPath": target,
            "checkoutId": workspace_ref.as_ref().map(|r| &r.checkout_id),
            "workspaceGeneration": workspace_ref.as_ref().and_then(|r| r.expected_generation),
            "materializationEpoch": workspace_ref.as_ref().and_then(|r| r.expected_materialization_epoch),
        })).map_err(action_error)?;
    }
    drop(scope);
    emit_changed(
        &app_handle,
        &updated,
        format!("file-action:{}", uuid::Uuid::new_v4()),
    )?;
    Ok(target.map(|path| path.to_string_lossy().into_owned()))
}

/// A real folder inside the checkout, or `None` when `path` names something else
/// so the regular file flow reports its own errors.
fn workspace_directory(root: &Path, path: &str) -> Result<Option<PathBuf>, AppError> {
    let relative = super::super::workspace::normalize_workspace_sub_path(path)?;
    if relative.is_empty() {
        return Err(action_error("Cannot delete the workspace root"));
    }
    let candidate = root.join(&relative);
    let Ok(metadata) = std::fs::symlink_metadata(&candidate) else { return Ok(None) };
    if metadata.file_type().is_symlink() {
        return if candidate.is_dir() { Err(action_error("Linked folders cannot be deleted")) } else { Ok(None) };
    }
    if !metadata.is_dir() {
        return Ok(None);
    }
    let root = dunce::canonicalize(root).map_err(action_error)?;
    let directory = dunce::canonicalize(&candidate).map_err(action_error)?;
    if directory == root || !directory.starts_with(&root) {
        return Err(action_error("Folders outside the workspace cannot be deleted"));
    }
    let protected = relative.to_ascii_lowercase();
    if [".git", ".locus", "locus", "locus/knowledge"].contains(&protected.as_str())
        || protected.starts_with(".git/")
        || protected.starts_with(".locus/")
    {
        return Err(action_error("This workspace configuration folder cannot be deleted"));
    }
    Ok(Some(directory))
}

/// Deletes a folder: knowledge folders go through the knowledge store; other
/// folders are staged with their Unity `.meta` beside them, unlinked from the
/// layout, then removed.
async fn delete_workspace_directory(
    project_id: &ProjectId,
    layout_root: &Path,
    reference: &WorkspaceRef,
    scope: ResolvedWorkspaceScope,
    source: PathBuf,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let action_root = scope.runtime().root().to_path_buf();
    let app_knowledge_dir: State<'_, crate::commands::knowledge::AppKnowledgeDir> = app_handle.state();
    let sources = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
        &action_root.to_string_lossy(),
        app_knowledge_dir.0.as_ref().as_ref(),
    );
    // Source roots (knowledge types, skill packages, mounts) are deleted only
    // through their own management flows, never as part of a parent folder.
    if sources.sources().iter().any(|knowledge| {
        crate::knowledge_source_registry::path_is_within(&knowledge.physical_root, &source)
    }) {
        return Err(action_error("This folder contains a knowledge source and cannot be deleted here"));
    }
    if let Some(knowledge) = sources.classify_path(&source) {
        use crate::knowledge_source_registry::KnowledgeSourceKind;
        if knowledge.kind != KnowledgeSourceKind::WorkspaceKnowledge
            || !knowledge.mutability.is_writable()
            || knowledge.logical_path.is_empty()
        {
            return Err(action_error("This knowledge source is managed or read-only"));
        }
        let registry_handle = Arc::clone(registry.inner());
        crate::commands::knowledge_delete(
            reference.clone(),
            crate::knowledge_store::KnowledgeDeleteRequest {
                kind: crate::knowledge_store::KnowledgeTargetKind::Directory,
                doc_type: Some(knowledge.doc_type),
                path: knowledge.logical_path,
            },
            app_handle.clone(),
            registry,
        )
        .await?;
        return reconcile_knowledge_file_action(
            registry_handle.as_ref(),
            &app_handle,
            project_id,
            &action_root.to_string_lossy(),
            &source,
            None,
        );
    }
    let _document_guard = super::super::view_documents::DOCUMENT_WRITE_LOCK.lock().map_err(action_error)?;
    let _guard = CSV_FILE_WRITE_LOCK.lock().map_err(action_error)?;
    let staged = source.with_file_name(format!(".locus-delete-{}", uuid::Uuid::new_v4()));
    // Renaming first fails while files inside are locked, before anything is lost.
    let moved = super::workspace_file_move::relocate_entry(&source, &staged)?;
    let updated = match crate::workspace_tree::relocate_file_references(layout_root, project_id.as_str(), &source, None) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            super::workspace_file_move::rollback_moves(&moved)?;
            return Err(action_error(error));
        }
    };
    for (index, (_, path)) in moved.iter().enumerate() {
        let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
        if let Err(error) = removed {
            // Put back whatever is left of the folder so it stays visible for a retry.
            if index == 0 {
                if let Err(rollback) = super::workspace_file_move::rollback_moves(&moved) {
                    tracing::warn!(error = %rollback.message, "folder delete rollback failed");
                }
            }
            return Err(action_error(error));
        }
    }
    crate::workspace_file_order::relocate(&action_root, &source, None).map_err(action_error)?;
    app_handle
        .emit(
            "explorer-file-action",
            serde_json::json!({
                "projectId": project_id.as_str(), "root": action_root, "path": source,
                "newPath": null, "isDirectory": true,
            }),
        )
        .map_err(action_error)?;
    drop(scope);
    emit_changed(&app_handle, &updated, format!("file-action:{}", uuid::Uuid::new_v4()))
}

fn emit_file_action(
    app: &AppHandle,
    project_id: &str,
    root: &Path,
    source: &Path,
    target: Option<&Path>,
) -> Result<(), AppError> {
    app.emit(
        "explorer-file-action",
        serde_json::json!({
            "projectId": project_id, "root": root, "path": source, "newPath": target,
        }),
    )
    .map_err(action_error)
}

/// Knowledge mutations share the same path reconciliation as ordinary files.
pub(crate) fn reconcile_knowledge_file_action(
    registry: &ProjectRegistry,
    app: &AppHandle,
    project_id: &ProjectId,
    working_dir: &str,
    source: &Path,
    target: Option<&Path>,
) -> Result<(), AppError> {
    let (_, layout) = resolve_layout(registry, project_id.to_string(), "knowledgeFileAction")?;
    let snapshot =
        crate::workspace_tree::relocate_file_references(&layout, project_id.as_str(), source, target)
            .map_err(action_error)?;
    crate::workspace_file_order::relocate(Path::new(working_dir), source, target).map_err(action_error)?;
    emit_file_action(
        app,
        project_id.as_str(),
        Path::new(working_dir),
        source,
        target,
    )?;
    emit_changed(
        app,
        &snapshot,
        format!("knowledge-file-action:{}", uuid::Uuid::new_v4()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_traversal_and_preserves_files_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.md");
        let target = dir.path().join("b.md");
        std::fs::write(&source, "source").unwrap();
        std::fs::write(&target, "target").unwrap();
        for name in ["../b.md", "..\\b.md", ".", "a:b", "a?"] {
            assert!(sibling_file(&source, name).is_err());
        }
        assert!(relocate_files(&source, &target).is_err());
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "source");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "target");
    }
    #[test]
    fn folder_delete_targets_only_ordinary_workspace_folders() {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["docs/sub", ".git/objects", "Locus/knowledge/design", "Locus/workspace-trees"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        std::fs::write(dir.path().join("docs/a.md"), "a").unwrap();
        let expected = dunce::canonicalize(dir.path().join("docs/sub")).unwrap();
        assert_eq!(workspace_directory(dir.path(), "docs/sub").unwrap(), Some(expected));
        assert_eq!(workspace_directory(dir.path(), "docs/a.md").unwrap(), None);
        assert_eq!(workspace_directory(dir.path(), "docs/missing").unwrap(), None);
        // Layouts moved to the data dir; a leftover checkout copy is ordinary content.
        let leftover = dunce::canonicalize(dir.path().join("Locus/workspace-trees")).unwrap();
        assert_eq!(workspace_directory(dir.path(), "Locus/workspace-trees").unwrap(), Some(leftover));
        for protected in ["", ".", ".git", ".git/objects", "Locus", "locus/knowledge", "../x"] {
            assert!(workspace_directory(dir.path(), protected).is_err(), "{protected}");
        }
    }
    #[test]
    fn csv_rename_and_rollback_preserve_companions() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.csv");
        let target = dir.path().join("b.csv");
        for suffix in ["", ".meta", ".view", ".view.meta"] {
            std::fs::write(dir.path().join(format!("a.csv{suffix}")), suffix).unwrap();
        }
        let moved = relocate_files(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(companion_files(&target).unwrap().len(), 4);
        rollback_files(&moved).unwrap();
        assert_eq!(companion_files(&source).unwrap().len(), 4);
        assert!(!target.exists());
    }
}
