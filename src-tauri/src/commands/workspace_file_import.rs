//! Copies files and folders dropped from the operating system into a checkout
//! folder. Each entry travels with its Unity `.meta` (and CSV view files). It
//! is copied to a hidden staging name beside its destination first and only
//! then takes the final name, so a failed copy never leaves a partial entry
//! under the dropped name, and a replaced entry is restored on failure.
use std::collections::HashSet;

use super::*;
use tauri::Manager;

fn import_error(message: impl ToString) -> AppError {
    AppError::new("workspace.file_import_failed", message.to_string())
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileImportResult {
    /// Workspace-relative paths of the dropped entries now in the folder.
    imported: Vec<String>,
    /// Workspace-relative destinations that already exist. When any are
    /// reported, nothing was copied.
    conflicts: Vec<String>,
}

/// A dropped entry and its companions, each paired with its destination. The
/// entry itself is always first.
#[derive(Debug)]
struct ImportEntry {
    target: PathBuf,
    relative: String,
    files: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug)]
struct ImportPlan {
    entries: Vec<ImportEntry>,
    conflicts: Vec<String>,
}

fn relative_path(root: &Path, path: &Path) -> Result<String, AppError> {
    Ok(path.strip_prefix(root).map_err(import_error)?.to_string_lossy().replace('\\', "/"))
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// The companion suffix of `path` (`""`, `.meta`, `.view`, ...), which is
/// always built by appending to `base`.
fn suffix_of(path: &Path, base: &Path) -> String {
    path.to_string_lossy()[base.to_string_lossy().len()..].to_string()
}

fn companions(source: &Path, is_dir: bool) -> Result<Vec<PathBuf>, AppError> {
    if !is_dir {
        return super::explorer_file_actions::companion_files(source);
    }
    let mut paths = vec![source.to_path_buf()];
    let sidecar = suffixed(source, ".meta");
    match std::fs::symlink_metadata(&sidecar) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => paths.push(sidecar),
        Ok(_) => return Err(import_error("Invalid folder metadata file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(import_error(error)),
    }
    Ok(paths)
}

/// Resolves the destination of every dropped path and reports existing
/// destinations without touching the file system.
fn import_plan(root: &Path, sources: &[String], directory: &str) -> Result<ImportPlan, AppError> {
    let root = dunce::canonicalize(root).map_err(import_error)?;
    let requested = super::super::workspace::normalize_workspace_sub_path(directory)?;
    let directory = dunce::canonicalize(root.join(requested)).map_err(import_error)?;
    if !directory.is_dir() || !directory.starts_with(&root) {
        return Err(import_error("The destination must be a folder within this workspace"));
    }
    let destination = relative_path(&root, &directory)?.to_ascii_lowercase();
    if destination == ".git" || destination.starts_with(".git/")
        || destination == ".locus" || destination.starts_with(".locus/") {
        return Err(import_error("Files cannot be added to workspace configuration folders"));
    }

    let mut dropped: Vec<(PathBuf, bool, Vec<PathBuf>)> = Vec::new();
    let mut seen = HashSet::new();
    for raw in sources {
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(import_error(format!("Not an absolute path: {raw}")));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(import_error)?;
        if metadata.file_type().is_symlink() {
            return Err(import_error(format!("Linked files and folders cannot be added: {}", path.display())));
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(import_error(format!("Only files and folders can be added: {}", path.display())));
        }
        let path = dunce::canonicalize(&path).map_err(import_error)?;
        if seen.insert(path_key(&path)) {
            let files = companions(&path, metadata.is_dir())?;
            dropped.push((path, metadata.is_dir(), files));
        }
    }
    // A `.meta` or CSV view file dropped together with its entry travels as
    // that entry's companion instead of a separate item.
    let carried: HashSet<String> = dropped.iter()
        .flat_map(|(_, _, files)| files.iter().skip(1).map(|file| path_key(file)))
        .collect();
    dropped.retain(|(path, _, _)| !carried.contains(&path_key(path)));

    let mut entries = Vec::new();
    let mut conflicts = Vec::new();
    let mut targets = HashSet::new();
    for (source, is_dir, files) in dropped {
        let name = source.file_name().ok_or_else(|| import_error("Invalid source name"))?
            .to_string_lossy().into_owned();
        if is_dir && crate::knowledge_source_registry::path_is_within(&directory, &source) {
            return Err(import_error(format!("Cannot copy {name} into itself")));
        }
        let target = super::explorer_file_actions::sibling_file(&directory.join("entry"), &name)?;
        if path_key(&target) == path_key(&source) {
            return Err(import_error(format!("{name} is already in this folder")));
        }
        if crate::knowledge_source_registry::path_is_within(&source, &target) {
            return Err(import_error(format!("Cannot replace {name} with an item inside it")));
        }
        let relative = relative_path(&root, &target)?;
        if super::workspace_file_move::configuration_path(&relative) {
            return Err(import_error("This name is reserved for workspace configuration"));
        }
        if !targets.insert(path_key(&target)) {
            return Err(import_error(format!("More than one dropped item is named {name}")));
        }
        let files: Vec<_> = files.into_iter()
            .map(|from| {
                let to = suffixed(&target, &suffix_of(&from, &source));
                (from, to)
            })
            .collect();
        for (_, to) in &files {
            if std::fs::symlink_metadata(to).is_ok() {
                conflicts.push(relative_path(&root, to)?);
            }
        }
        entries.push(ImportEntry { target, relative, files });
    }
    Ok(ImportPlan { entries, conflicts })
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for child in std::fs::read_dir(from)? {
        let child = child?;
        let kind = child.file_type()?;
        let target = to.join(child.file_name());
        if kind.is_symlink() {
            return Err(std::io::Error::other(format!(
                "Linked files and folders cannot be added: {}",
                child.path().display()
            )));
        } else if kind.is_dir() {
            copy_tree(&child.path(), &target)?;
        } else {
            std::fs::copy(child.path(), &target)?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn discard(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        if let Err(error) = remove_path(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %path.display(), %error, "failed to remove a temporary import entry");
            }
        }
    }
}

/// Copies every file of the entry to a hidden staging name beside its
/// destination. Returns `(staged, destination)` pairs.
fn stage_entry(entry: &ImportEntry) -> Result<Vec<(PathBuf, PathBuf)>, AppError> {
    let base = entry.target.with_file_name(format!(".locus-import-{}", uuid::Uuid::new_v4()));
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in &entry.files {
        let path = suffixed(&base, &suffix_of(to, &entry.target));
        let copied = if from.is_dir() {
            copy_tree(from, &path)
        } else {
            std::fs::copy(from, &path).map(|_| ())
        };
        if let Err(error) = copied {
            discard(staged.into_iter().map(|(staged, _)| staged).chain([path]));
            return Err(import_error(error));
        }
        staged.push((path, to.clone()));
    }
    Ok(staged)
}

/// Moves staged files onto their destinations. With `overwrite`, existing
/// destinations are set aside first and restored if any step fails. Returns
/// the set-aside paths, which the caller removes.
fn commit_entry(staged: &[(PathBuf, PathBuf)], overwrite: bool) -> Result<Vec<PathBuf>, AppError> {
    use super::workspace_file_move::rename_without_replace;
    let rename = |from: &Path, to: &Path| rename_without_replace(from, to).map_err(|error| import_error(error.message));
    let restore = |replaced: &[(PathBuf, PathBuf)]| {
        for (destination, aside) in replaced.iter().rev() {
            if let Err(error) = rename(aside, destination) {
                tracing::warn!(path = %destination.display(), error = %error.message, "failed to restore a replaced entry");
            }
        }
    };
    let Some((_, target)) = staged.first() else {
        return Ok(Vec::new());
    };
    let mut replaced: Vec<(PathBuf, PathBuf)> = Vec::new();
    if overwrite {
        let base = target.with_file_name(format!(".locus-replace-{}", uuid::Uuid::new_v4()));
        for (_, destination) in staged {
            if std::fs::symlink_metadata(destination).is_err() {
                continue;
            }
            let aside = suffixed(&base, &suffix_of(destination, target));
            if let Err(error) = rename(destination, &aside) {
                restore(&replaced);
                return Err(error);
            }
            replaced.push((destination.clone(), aside));
        }
    }
    let mut placed: Vec<(&Path, &Path)> = Vec::new();
    for (path, destination) in staged {
        if let Err(error) = rename(path, destination) {
            for (path, destination) in placed.iter().rev() {
                if let Err(error) = rename(destination, path) {
                    tracing::warn!(path = %destination.display(), error = %error.message, "failed to undo an imported entry");
                }
            }
            restore(&replaced);
            return Err(error);
        }
        placed.push((path.as_path(), destination.as_path()));
    }
    Ok(replaced.into_iter().map(|(_, aside)| aside).collect())
}

/// Imports entries in order. Returns the destinations written before any
/// failure, together with that failure.
fn import_entries(entries: &[ImportEntry], overwrite: bool) -> (Vec<PathBuf>, Option<AppError>) {
    let mut written = Vec::new();
    for entry in entries {
        let staged = match stage_entry(entry) {
            Ok(staged) => staged,
            Err(error) => return (written, Some(error)),
        };
        let committed = (|| {
            // Keep Markdown and CSV saves out of the replacement.
            let _document_guard = super::super::view_documents::DOCUMENT_WRITE_LOCK.lock().map_err(import_error)?;
            let _guard = CSV_FILE_WRITE_LOCK.lock().map_err(import_error)?;
            commit_entry(&staged, overwrite)
        })();
        match committed {
            Ok(replaced) => {
                discard(replaced);
                written.extend(staged.into_iter().map(|(_, destination)| destination));
            }
            Err(error) => {
                discard(staged.into_iter().map(|(staged, _)| staged));
                return (written, Some(error));
            }
        }
    }
    (written, None)
}

#[derive(Clone, Serialize)]
struct ImportedFileChange {
    #[serde(flatten)]
    change: crate::workspace_changes::WorkspaceChangeEvent,
    /// A copied folder arrives as one event, so a loaded subtree is re-read.
    rescan: bool,
}

fn emit_imported(app_handle: &AppHandle, scope: &ResolvedWorkspaceScope, root: &Path, written: &[PathBuf]) {
    use crate::workspace_changes::{hub_for_workspace, WorkspaceChangeKind, WorkspaceChangeSource};
    use crate::workspace_service::event::{emit_for_workspace_scope, WorkspaceEventScope};
    let hub = hub_for_workspace(root);
    let event_scope = WorkspaceEventScope::for_runtime(scope.runtime());
    for path in written {
        let Ok(relative) = relative_path(root, path) else { continue };
        if let Some(change) = hub.observe(&relative, WorkspaceChangeKind::Upsert, WorkspaceChangeSource::LocusWrite) {
            emit_for_workspace_scope(app_handle, &event_scope, "workspace-file-changed",
                ImportedFileChange { change, rescan: path.is_dir() });
        }
    }
}

/// Copies files and folders from outside the checkout into `target_directory`.
/// Without `overwrite`, existing destinations are reported as conflicts and
/// nothing is copied; with it, they are replaced.
#[tauri::command]
pub async fn workspace_import_entries(
    workspace_ref: WorkspaceRef,
    source_paths: Vec<String>,
    target_directory: String,
    overwrite: bool,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<WorkspaceFileImportResult, AppError> {
    let scope = registry.resolve_workspace_ref(&workspace_ref).map_err(import_error)?;
    let root = dunce::canonicalize(scope.runtime().root()).map_err(import_error)?;
    let plan = import_plan(&root, &source_paths, &target_directory)?;
    let app_knowledge: State<'_, crate::commands::knowledge::AppKnowledgeDir> = app_handle.state();
    let sources = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
        &root.to_string_lossy(),
        app_knowledge.0.as_ref().as_ref(),
    );
    for entry in &plan.entries {
        if sources.sources().iter().any(|knowledge| {
            crate::knowledge_source_registry::path_is_within(&knowledge.physical_root, &entry.target)
        }) {
            return Err(import_error("A folder that holds a knowledge source cannot be replaced"));
        }
        if let Some(knowledge) = sources.classify_path(&entry.target) {
            if knowledge.kind != crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
                || !knowledge.mutability.is_writable() {
                return Err(import_error("This knowledge source is read-only"));
            }
        }
    }
    if !overwrite && !plan.conflicts.is_empty() {
        return Ok(WorkspaceFileImportResult { imported: Vec::new(), conflicts: plan.conflicts });
    }
    let imported: Vec<String> = plan.entries.iter().map(|entry| entry.relative.clone()).collect();
    let entries = plan.entries;
    let (written, error) = tauri::async_runtime::spawn_blocking(move || import_entries(&entries, overwrite))
        .await
        .map_err(import_error)?;
    emit_imported(&app_handle, &scope, &root, &written);
    if let Some(error) = error {
        return Err(error);
    }
    Ok(WorkspaceFileImportResult { imported, conflicts: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn arg(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn import(root: &Path, sources: &[String], directory: &str, overwrite: bool) -> Result<ImportPlan, AppError> {
        let plan = import_plan(root, sources, directory)?;
        if !overwrite && !plan.conflicts.is_empty() {
            return Ok(plan);
        }
        let (_, error) = import_entries(&plan.entries, overwrite);
        match error {
            Some(error) => Err(error),
            None => Ok(plan),
        }
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(directory).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn copies_files_and_folders_with_unity_metadata() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Assets")).unwrap();
        std::fs::write(outside.path().join("hero.png"), "png").unwrap();
        std::fs::write(outside.path().join("hero.png.meta"), "guid: png").unwrap();
        std::fs::write(outside.path().join("table.csv"), "id\n").unwrap();
        std::fs::write(outside.path().join("table.csv.view"), "view").unwrap();
        std::fs::create_dir_all(outside.path().join("Prefabs/Nested")).unwrap();
        std::fs::write(outside.path().join("Prefabs/Nested/a.prefab"), "prefab").unwrap();
        std::fs::write(outside.path().join("Prefabs/Nested/a.prefab.meta"), "guid: a").unwrap();
        std::fs::write(outside.path().join("Prefabs.meta"), "guid: folder").unwrap();
        // The dropped `.meta` travels with its asset instead of being copied twice.
        let sources = ["hero.png", "hero.png.meta", "table.csv", "Prefabs"]
            .map(|name| arg(&outside.path().join(name)));
        let plan = import(root.path(), &sources, "Assets", false).unwrap();
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.entries.iter().map(|entry| entry.relative.as_str()).collect::<Vec<_>>(),
            ["Assets/hero.png", "Assets/table.csv", "Assets/Prefabs"]);
        let assets = root.path().join("Assets");
        assert_eq!(names(&assets), ["Prefabs", "Prefabs.meta", "hero.png", "hero.png.meta", "table.csv", "table.csv.view"]);
        assert_eq!(text(&assets.join("hero.png.meta")), "guid: png");
        assert_eq!(text(&assets.join("Prefabs/Nested/a.prefab.meta")), "guid: a");
        assert_eq!(text(&assets.join("Prefabs.meta")), "guid: folder");
        // Copying leaves the originals in place.
        assert_eq!(text(&outside.path().join("hero.png")), "png");
        assert!(outside.path().join("Prefabs/Nested/a.prefab").exists());
    }

    #[test]
    fn reports_conflicts_and_copies_nothing_until_overwrite() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), "new a").unwrap();
        std::fs::write(outside.path().join("b.txt"), "new b").unwrap();
        std::fs::write(outside.path().join("b.txt.meta"), "new meta").unwrap();
        std::fs::write(root.path().join("b.txt.meta"), "old meta").unwrap();
        let sources = [arg(&outside.path().join("a.txt")), arg(&outside.path().join("b.txt"))];
        let plan = import(root.path(), &sources, ".", false).unwrap();
        assert_eq!(plan.conflicts, ["b.txt.meta"]);
        assert_eq!(names(root.path()), ["b.txt.meta"]);

        import(root.path(), &sources, ".", true).unwrap();
        assert_eq!(names(root.path()), ["a.txt", "b.txt", "b.txt.meta"]);
        assert_eq!(text(&root.path().join("b.txt.meta")), "new meta");
    }

    #[test]
    fn overwrite_replaces_whole_folders_and_keeps_unmatched_metadata() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("Data")).unwrap();
        std::fs::write(outside.path().join("Data/new.txt"), "new").unwrap();
        std::fs::create_dir_all(root.path().join("Data")).unwrap();
        std::fs::write(root.path().join("Data/old.txt"), "old").unwrap();
        std::fs::write(root.path().join("Data.meta"), "guid: kept").unwrap();
        let sources = [arg(&outside.path().join("Data"))];
        assert_eq!(import(root.path(), &sources, ".", false).unwrap().conflicts, ["Data"]);
        import(root.path(), &sources, ".", true).unwrap();
        assert_eq!(names(&root.path().join("Data")), ["new.txt"]);
        // The source had no `.meta`, so the existing GUID stays.
        assert_eq!(text(&root.path().join("Data.meta")), "guid: kept");
        assert_eq!(names(root.path()), ["Data", "Data.meta"]);
    }

    #[test]
    fn failed_commit_restores_replaced_entries() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), "new").unwrap();
        std::fs::write(outside.path().join("a.txt.meta"), "new meta").unwrap();
        std::fs::write(root.path().join("a.txt"), "old").unwrap();
        std::fs::write(root.path().join("a.txt.meta"), "old meta").unwrap();
        let plan = import_plan(root.path(), &[arg(&outside.path().join("a.txt"))], ".").unwrap();
        let staged = stage_entry(&plan.entries[0]).unwrap();
        // Lose the staged companion so the second rename fails after the first succeeded.
        std::fs::remove_file(&staged[1].0).unwrap();
        assert!(commit_entry(&staged, true).is_err());
        assert_eq!(text(&root.path().join("a.txt")), "old");
        assert_eq!(text(&root.path().join("a.txt.meta")), "old meta");
        discard(staged.into_iter().map(|(staged, _)| staged));
        assert_eq!(names(root.path()), ["a.txt", "a.txt.meta"]);
    }

    // Only the Windows rename refuses to replace an existing destination.
    #[cfg(windows)]
    #[test]
    fn commit_without_overwrite_keeps_entries_created_after_planning() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), "new").unwrap();
        let plan = import_plan(root.path(), &[arg(&outside.path().join("a.txt"))], ".").unwrap();
        assert!(plan.conflicts.is_empty());
        std::fs::write(root.path().join("a.txt"), "created meanwhile").unwrap();
        let (written, error) = import_entries(&plan.entries, false);
        assert!(written.is_empty() && error.is_some());
        assert_eq!(text(&root.path().join("a.txt")), "created meanwhile");
        assert_eq!(names(root.path()), ["a.txt"]);
    }

    #[test]
    fn rejects_invalid_destinations_and_sources() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("a.txt"), "a").unwrap();
        let file = [arg(&outside.path().join("a.txt"))];
        for directory in ["../", "missing", ".git", ".locus/state"] {
            std::fs::create_dir_all(root.path().join(".git")).unwrap();
            std::fs::create_dir_all(root.path().join(".locus/state")).unwrap();
            assert!(import_plan(root.path(), &file, directory).is_err(), "{directory}");
        }
        assert!(import_plan(root.path(), &["a.txt".to_string()], ".").is_err());
        assert!(import_plan(root.path(), &[arg(&outside.path().join("missing.txt"))], ".").is_err());

        // A folder into itself, an entry onto itself, and a folder replaced by its own child.
        std::fs::create_dir_all(root.path().join("Docs/Docs")).unwrap();
        assert!(import_plan(root.path(), &[arg(&root.path().join("Docs"))], "Docs/Docs").is_err());
        assert!(import_plan(root.path(), &[arg(&root.path().join("Docs"))], ".").is_err());
        assert!(import_plan(root.path(), &[arg(&root.path().join("Docs/Docs"))], ".").is_err());

        // Reserved names and two dropped items with the same name.
        std::fs::create_dir_all(outside.path().join("Locus")).unwrap();
        assert!(import_plan(root.path(), &[arg(&outside.path().join("Locus"))], ".").is_err());
        std::fs::create_dir_all(outside.path().join("other")).unwrap();
        std::fs::write(outside.path().join("other/a.txt"), "other").unwrap();
        let twins = [arg(&outside.path().join("a.txt")), arg(&outside.path().join("other/a.txt"))];
        assert!(import_plan(root.path(), &twins, ".").is_err());
        assert_eq!(names(root.path()), [".git", ".locus", "Docs"]);
    }
}
