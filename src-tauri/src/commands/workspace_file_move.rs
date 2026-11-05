use super::*;
use super::explorer_file_actions::companion_files;
use tauri::Manager;

fn move_error(message: impl ToString) -> AppError {
    AppError::new("workspace.file_move_failed", message.to_string())
}

pub(super) fn configuration_path(relative: &str) -> bool {
    let path = relative.to_ascii_lowercase();
    path == ".git" || path.starts_with(".git/") || path == ".locus" || path.starts_with(".locus/")
        || path == "locus" || path == "locus/knowledge"
}

/// Resolve both existing endpoints without following links outside this checkout.
/// `name` renames the entry inside `directory`; otherwise it keeps its name.
fn move_paths(root: &Path, source: &str, directory: &str, name: Option<&str>) -> Result<(PathBuf, PathBuf), AppError> {
    let source_relative = super::super::workspace::normalize_workspace_sub_path(source)?;
    let directory_relative = super::super::workspace::normalize_workspace_sub_path(directory)?;
    if source_relative.is_empty() { return Err(move_error("Cannot move the workspace root")); }
    let root = dunce::canonicalize(root).map_err(move_error)?;
    let source = root.join(&source_relative);
    let directory = root.join(&directory_relative);
    for path in [&source, &directory] {
        let resolved = dunce::canonicalize(path).map_err(move_error)?;
        if !resolved.starts_with(&root) { return Err(move_error("Move must stay within the workspace")); }
        if std::fs::symlink_metadata(path).map_err(move_error)?.file_type().is_symlink() {
            return Err(move_error("Linked files and folders cannot be moved"));
        }
    }
    if !directory.is_dir() { return Err(move_error("The destination is not a folder")); }
    let source = dunce::canonicalize(source).map_err(move_error)?;
    let directory = dunce::canonicalize(directory).map_err(move_error)?;
    if directory.starts_with(&source) { return Err(move_error("Cannot move a folder into itself")); }
    if configuration_path(&source_relative) {
        return Err(move_error("This workspace configuration path cannot be moved"));
    }
    let destination = directory_relative.to_ascii_lowercase();
    if destination == ".git" || destination.starts_with(".git/") || destination == ".locus" || destination.starts_with(".locus/") {
        return Err(move_error("Files cannot be moved into workspace configuration folders"));
    }
    let target = match name {
        Some(name) => super::explorer_file_actions::sibling_file(&directory.join("entry"), name)?,
        None => directory.join(source.file_name().ok_or_else(|| move_error("Invalid source name"))?),
    };
    let target_name = target.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    let target_relative = if directory_relative.is_empty() { target_name.into_owned() } else { format!("{directory_relative}/{target_name}") };
    if configuration_path(&target_relative) {
        return Err(move_error("This name is reserved for workspace configuration"));
    }
    Ok((source, target))
}

pub(super) fn relocate_entry(source: &Path, target: &Path) -> Result<Vec<(PathBuf, PathBuf)>, AppError> {
    let paths = if source.is_file() { companion_files(source)? } else {
        if !source.is_dir() { return Err(move_error("Only files and folders can be moved")); }
        let mut paths = vec![source.to_path_buf()];
        let sidecar = PathBuf::from(format!("{}.meta", source.to_string_lossy()));
        match std::fs::symlink_metadata(&sidecar) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => paths.push(sidecar),
            Ok(_) => return Err(move_error("Invalid folder metadata file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(move_error(error)),
        }
        paths
    };
    let moves: Vec<_> = paths.into_iter().map(|from| {
        let suffix = from.to_string_lossy()[source.to_string_lossy().len()..].to_string();
        let to = PathBuf::from(format!("{}{suffix}", target.to_string_lossy()));
        (from, to)
    }).collect();
    for (_, to) in &moves {
        if std::fs::symlink_metadata(to).is_ok() { return Err(move_error(format!("Target already exists: {}", to.display()))); }
    }
    let mut completed = Vec::new();
    for (from, to) in moves {
        if let Err(error) = rename_without_replace(&from, &to) {
            rollback_moves(&completed)?;
            return Err(move_error(error));
        }
        completed.push((from, to));
    }
    Ok(completed)
}

pub(super) fn rename_without_replace(from: &Path, to: &Path) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // No REPLACE_EXISTING: preserve a destination created after preflight too.
        unsafe { MoveFileExW(windows::core::PCWSTR(from.as_ptr()), windows::core::PCWSTR(to.as_ptr()), MOVEFILE_WRITE_THROUGH) }.map_err(move_error)
    }
    #[cfg(not(windows))]
    { std::fs::rename(from, to).map_err(move_error) }
}

pub(super) fn rollback_moves(moves: &[(PathBuf, PathBuf)]) -> Result<(), AppError> {
    for (from, to) in moves.iter().rev() { rename_without_replace(to, from)?; }
    Ok(())
}

/// A case-only rename on a case-insensitive file system sees its own source as
/// an existing target, so it steps through a staging name.
fn relocate_entry_renaming_case(source: &Path, target: &Path) -> Result<Vec<(PathBuf, PathBuf)>, AppError> {
    if source == target || path_key(source) != path_key(target) { return relocate_entry(source, target); }
    let staging = source.with_file_name(format!(".locus-rename-{}", uuid::Uuid::new_v4()));
    let staged = relocate_entry(source, &staging)?;
    match relocate_entry(&staging, target) {
        // Both steps list the entry first and then its sidecars in the same order.
        Ok(moved) => Ok(staged.into_iter().zip(moved).map(|((from, _), (_, to))| (from, to)).collect()),
        Err(error) => { rollback_moves(&staged)?; Err(error) }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileMoveResult {
    project_id: String,
    root: String,
    path: String,
    new_path: String,
    is_directory: bool,
    workspace_ref: WorkspaceRef,
}

#[tauri::command]
pub async fn workspace_move_entry(
    workspace_ref: WorkspaceRef,
    source_path: String,
    target_directory: String,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<WorkspaceFileMoveResult, AppError> {
    relocate_workspace_entry(workspace_ref, &source_path, &target_directory, None, registry, app_handle).await
}

/// Renames a checkout file or folder in place through the move flow, so folders
/// keep their Unity `.meta`, knowledge identity, layout references and order.
/// Returns the new absolute path.
pub(super) async fn rename_workspace_entry(
    workspace_ref: WorkspaceRef,
    source_path: &str,
    name: &str,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let source = super::super::workspace::normalize_workspace_sub_path(source_path)?;
    let parent = source.rsplit_once('/').map_or(".", |(parent, _)| parent).to_string();
    let result = relocate_workspace_entry(workspace_ref, &source, &parent, Some(name), registry, app_handle).await?;
    Ok(result.new_path)
}

async fn relocate_workspace_entry(
    workspace_ref: WorkspaceRef,
    source_path: &str,
    target_directory: &str,
    name: Option<&str>,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<WorkspaceFileMoveResult, AppError> {
    let registry_handle = Arc::clone(registry.inner());
    let scope = registry.resolve_workspace_ref(&workspace_ref).map_err(move_error)?;
    let runtime = scope.runtime();
    let root = runtime.root();
    let project_id = runtime.project_id().clone();
    let (source, target) = move_paths(root, source_path, target_directory, name)?;
    let result = WorkspaceFileMoveResult {
        project_id: project_id.to_string(), root: root.to_string_lossy().into_owned(),
        path: source.to_string_lossy().into_owned(), new_path: target.to_string_lossy().into_owned(),
        is_directory: source.is_dir(), workspace_ref: workspace_ref.clone(),
    };
    if source == target { return Ok(result); }
    let app_knowledge: State<'_, crate::commands::knowledge::AppKnowledgeDir> = app_handle.state();
    let sources = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(&root.to_string_lossy(), app_knowledge.0.as_ref().as_ref());
    // Source roots (knowledge types, skill packages, mounts) move only through
    // their own management flows, never as part of a parent folder.
    if result.is_directory && sources.sources().iter().any(|knowledge| {
        crate::knowledge_source_registry::path_is_within(&knowledge.physical_root, &source)
    }) {
        return Err(move_error("This folder contains a knowledge source and cannot be moved or renamed"));
    }
    let from_knowledge = sources.classify_path(&source);
    let to_knowledge = sources.classify_path(&target);
    if from_knowledge.is_some() || to_knowledge.is_some() {
        let (Some(from), Some(to)) = (from_knowledge, to_knowledge) else {
            return Err(move_error("Knowledge entries must remain in their knowledge category"));
        };
        use crate::knowledge_source_registry::KnowledgeSourceKind;
        if from.kind != KnowledgeSourceKind::WorkspaceKnowledge || to.kind != KnowledgeSourceKind::WorkspaceKnowledge
            || !from.mutability.is_writable() || !to.mutability.is_writable() || from.doc_type != to.doc_type
            || from.logical_path.is_empty() || to.logical_path.is_empty() {
            return Err(move_error("This knowledge entry cannot be moved to that folder"));
        }
        crate::commands::knowledge_move(workspace_ref, crate::knowledge_store::KnowledgeMoveRequest {
            kind: if result.is_directory { crate::knowledge_store::KnowledgeTargetKind::Directory } else { crate::knowledge_store::KnowledgeTargetKind::Document },
            doc_type: Some(from.doc_type), path: from.logical_path, new_path: to.logical_path,
        }, app_handle.clone(), registry).await?;
        if result.is_directory {
            super::explorer_file_actions::reconcile_knowledge_file_action(registry_handle.as_ref(), &app_handle,
                &project_id, &result.root, &source, Some(&target))?;
        }
    } else {
        let (_, layout_root) = resolve_layout(registry.inner().as_ref(), project_id.to_string(), "workspaceMoveEntry")?;
        // Keep Markdown and CSV saves out of the move/reference-update transaction.
        let _document_guard = super::super::view_documents::DOCUMENT_WRITE_LOCK.lock().map_err(move_error)?;
        let _guard = CSV_FILE_WRITE_LOCK.lock().map_err(move_error)?;
        let moved = relocate_entry_renaming_case(&source, &target)?;
        let updated = match crate::workspace_tree::relocate_file_references(&layout_root, project_id.as_str(), &source, Some(&target)) {
            Ok(snapshot) => snapshot,
            Err(error) => { rollback_moves(&moved)?; return Err(move_error(error)); }
        };
        emit_changed(&app_handle, &updated, format!("file-move:{}", uuid::Uuid::new_v4()))?;
        for (from, to) in moved.iter().skip(1) {
            app_handle.emit("explorer-file-action", WorkspaceFileMoveResult {
                path: from.to_string_lossy().into_owned(), new_path: to.to_string_lossy().into_owned(),
                is_directory: false, ..result.clone()
            }).map_err(move_error)?;
        }
    }
    crate::workspace_file_order::relocate(root, &source, Some(&target)).map_err(move_error)?;
    app_handle.emit("explorer-file-action", result.clone()).map_err(move_error)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_move_preserves_csv_companions_and_conflicts() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();
        for suffix in ["", ".meta", ".view", ".view.meta"] {
            std::fs::write(root.path().join(format!("a.csv{suffix}")), suffix).unwrap();
        }
        let (source, target) = move_paths(root.path(), "a.csv", "target", None).unwrap();
        for suffix in [".meta", ".view", ".view.meta"] {
            let occupied = target.with_file_name(format!("a.csv{suffix}"));
            std::fs::write(&occupied, "occupied").unwrap();
            assert!(relocate_entry(&source, &target).is_err());
            for original in ["", ".meta", ".view", ".view.meta"] {
                assert_eq!(std::fs::read_to_string(source.with_file_name(format!("a.csv{original}"))).unwrap(), original);
            }
            assert!(!target.exists());
            assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "occupied");
            std::fs::remove_file(occupied).unwrap();
        }
        let moves = relocate_entry(&source, &target).unwrap();
        assert_eq!(moves.len(), 4);
        for suffix in ["", ".meta", ".view", ".view.meta"] {
            let name = format!("a.csv{suffix}");
            assert!(!source.with_file_name(&name).exists());
            assert_eq!(std::fs::read_to_string(target.with_file_name(&name)).unwrap(), suffix);
        }
        rollback_moves(&moves).unwrap();
        for suffix in ["", ".meta", ".view", ".view.meta"] {
            let name = format!("a.csv{suffix}");
            assert!(!target.with_file_name(&name).exists());
            assert_eq!(std::fs::read_to_string(source.with_file_name(&name)).unwrap(), suffix);
        }
    }
    #[test]
    fn folder_move_preserves_children_and_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("source/nested")).unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();
        std::fs::write(root.path().join("source/nested/a.txt"), "content").unwrap();
        std::fs::write(root.path().join("source.meta"), "guid: abc").unwrap();
        let (source, target) = move_paths(root.path(), "source", "target", None).unwrap();
        let moves = relocate_entry(&source, &target).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("nested/a.txt")).unwrap(), "content");
        assert_eq!(std::fs::read_to_string(target.with_file_name("source.meta")).unwrap(), "guid: abc");
        rollback_moves(&moves).unwrap();
        assert!(source.join("nested/a.txt").exists());
    }
    #[test]
    fn rejects_root_escape_self_descendant_and_existing_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("a/b")).unwrap();
        std::fs::create_dir_all(root.path().join("target/a")).unwrap();
        for (source, target) in [(".", "a"), ("../a", "."), ("a", "../"), ("a", "a/b"), ("a", "a")] {
            assert!(move_paths(root.path(), source, target, None).is_err());
        }
        let (source, target) = move_paths(root.path(), "a", "target", None).unwrap();
        assert!(relocate_entry(&source, &target).is_err());
        assert!(source.join("b").exists());
        assert!(target.is_dir());
    }
    #[test]
    fn folder_rename_stays_in_parent_and_rejects_invalid_or_reserved_names() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("docs/old/nested")).unwrap();
        std::fs::create_dir(root.path().join("top")).unwrap();
        std::fs::write(root.path().join("docs/old/nested/a.md"), "content").unwrap();
        std::fs::write(root.path().join("docs/old.meta"), "guid: abc").unwrap();
        let (source, target) = move_paths(root.path(), "docs/old", "docs", Some(" new ")).unwrap();
        assert_eq!(target, source.with_file_name("new"));
        for name in ["", "..", "../escape", "a/b", "a\\b", "bad?", "trailing."] {
            assert!(move_paths(root.path(), "docs/old", "docs", Some(name)).is_err(), "{name}");
        }
        for name in [".git", ".locus", "Locus"] {
            assert!(move_paths(root.path(), "top", ".", Some(name)).is_err(), "{name}");
        }
        let moves = relocate_entry_renaming_case(&source, &target).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("nested/a.md")).unwrap(), "content");
        assert_eq!(std::fs::read_to_string(target.with_file_name("new.meta")).unwrap(), "guid: abc");
        assert!(!source.exists());
        rollback_moves(&moves).unwrap();
        assert!(source.join("nested/a.md").exists());
        assert!(source.with_file_name("old.meta").exists());
    }
    #[test]
    fn folder_rename_changes_only_letter_case() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Docs/nested")).unwrap();
        std::fs::write(root.path().join("Docs/nested/a.md"), "content").unwrap();
        std::fs::write(root.path().join("Docs.meta"), "guid: abc").unwrap();
        let (source, target) = move_paths(root.path(), "Docs", ".", Some("docs")).unwrap();
        let moves = relocate_entry_renaming_case(&source, &target).unwrap();
        assert_eq!(moves.iter().map(|(from, _)| from.clone()).collect::<Vec<_>>(), vec![source.clone(), source.with_file_name("Docs.meta")]);
        let mut names: Vec<_> = std::fs::read_dir(root.path()).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["docs", "docs.meta"]);
        assert_eq!(std::fs::read_to_string(target.join("nested/a.md")).unwrap(), "content");
    }
}
