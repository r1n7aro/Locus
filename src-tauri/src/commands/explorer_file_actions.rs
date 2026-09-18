use super::*;
use tauri::Manager;

fn action_error(error: impl ToString) -> AppError {
    AppError::new("workspace.file_action_failed", error.to_string())
}

fn sibling_file(source: &Path, name: &str) -> Result<PathBuf, AppError> {
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

fn companion_files(path: &Path) -> Result<Vec<PathBuf>, AppError> {
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
    let (project_id, root) =
        resolve_project(registry.inner().as_ref(), project_id, "explorerFileAction")?;
    let snapshot =
        crate::workspace_tree::snapshot(&root, project_id.as_str()).map_err(action_error)?;
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
    let moved = relocate_files(&source, &staged)?;
    let updated = match crate::workspace_tree::relocate_file_references(
        &root,
        project_id.as_str(),
        &source,
        target.as_deref(),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            rollback_files(&moved)?;
            return Err(action_error(error));
        }
    };
    if target.is_none() {
        for (_, path) in &moved {
            std::fs::remove_file(path).map_err(action_error)?;
        }
    }
    emit_file_action(
        &app_handle,
        project_id.as_str(),
        action_root,
        &source,
        target.as_deref(),
    )?;
    drop(scope);
    emit_changed(
        &app_handle,
        &updated,
        format!("file-action:{}", uuid::Uuid::new_v4()),
    )?;
    Ok(target.map(|path| path.to_string_lossy().into_owned()))
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
    let (_, root) = resolve_project(registry, project_id.to_string(), "knowledgeFileAction")?;
    let snapshot =
        crate::workspace_tree::relocate_file_references(&root, project_id.as_str(), source, target)
            .map_err(action_error)?;
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
