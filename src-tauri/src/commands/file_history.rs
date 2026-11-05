use super::*;
use serde::Deserialize;
use std::process::Output;
use std::time::Duration;

fn history_error(error: impl ToString) -> AppError {
    AppError::new("workspace.file_history_failed", error.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryVersion {
    pub commit: String,
    pub author: String,
    pub email: String,
    pub modified_at: String,
    pub subject: String,
    pub path: String,
    pub blob: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistory {
    pub versions: Vec<FileHistoryVersion>,
    pub current_modified_at: String,
    pub current_hash: String,
    pub is_repository: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryPreview {
    pub file: ProjectExplorerFilePreview,
    pub asset: Option<super::super::asset::AssetPreviewPayload>,
    pub content_hash: String,
    pub csv_view: Option<String>,
}

struct HistoryTarget {
    path: PathBuf,
    project_id: ProjectId,
    scope: Option<ResolvedWorkspaceScope>,
}

fn resolve_target(
    registry: &ProjectRegistry,
    project_id: String,
    path: &str,
    workspace_ref: Option<&WorkspaceRef>,
) -> Result<HistoryTarget, AppError> {
    if let Some(reference) = workspace_ref {
        let (scope, resolved_project, canonical) =
            resolve_workspace_file(reference, path, registry)?;
        if resolved_project.as_str() != project_id {
            return Err(history_error("Workspace does not belong to the project"));
        }
        Ok(HistoryTarget {
            path: dunce::canonicalize(canonical).map_err(history_error)?,
            project_id: resolved_project,
            scope: Some(scope),
        })
    } else {
        let (project_id, layout) = resolve_layout(registry, project_id, "fileHistory")?;
        let snapshot =
            crate::workspace_tree::snapshot(&layout, project_id.as_str()).map_err(history_error)?;
        let path = ensure_preview_path_authorized(&snapshot, Path::new(path))?;
        Ok(HistoryTarget {
            path,
            project_id,
            scope: None,
        })
    }
}

async fn git_output(root: &Path, args: &[&str]) -> Result<Output, AppError> {
    let mut command = crate::process_util::async_command("git");
    command
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.quotepath=false",
        ])
        .args(args)
        .current_dir(root)
        .kill_on_drop(true)
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_OPTIONAL_LOCKS", "0");
    tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .map_err(|_| history_error("Git history request timed out"))?
        .map_err(history_error)
}

async fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, AppError> {
    let output = git_output(root, args).await?;
    if !output.status.success() {
        return Err(history_error(
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(output.stdout)
}

async fn repository(path: &Path) -> Result<Option<(PathBuf, String)>, AppError> {
    let output = git_output(
        path.parent()
            .ok_or_else(|| history_error("Invalid file path"))?,
        &["rev-parse", "--show-toplevel"],
    )
    .await?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
            return Ok(None);
        }
        return Err(history_error(
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    let root = dunce::canonicalize(String::from_utf8_lossy(&output.stdout).trim())
        .map_err(history_error)?;
    let relative = path
        .strip_prefix(&root)
        .map_err(history_error)?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(Some((root, relative)))
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|ch| ch.is_ascii_hexdigit())
}

// NUL-separated metadata and raw paths preserve Unicode, spaces, tabs and
// newlines in filenames. Pure renames and mode-only changes have equal blobs.
fn parse_history(bytes: &[u8]) -> Result<Vec<FileHistoryVersion>, AppError> {
    let text = std::str::from_utf8(bytes).map_err(history_error)?;
    let fields: Vec<_> = text.split('\0').collect();
    let mut cursor = 0;
    let mut commit: Option<FileHistoryVersion> = None;
    let mut versions = Vec::new();
    while cursor < fields.len() {
        let field = fields[cursor].trim_start_matches('\n');
        cursor += 1;
        if is_oid(field) && cursor + 4 <= fields.len() {
            commit = Some(FileHistoryVersion {
                commit: field.to_string(),
                author: fields[cursor].to_string(),
                email: fields[cursor + 1].to_string(),
                modified_at: fields[cursor + 2].to_string(),
                subject: fields[cursor + 3].to_string(),
                path: String::new(),
                blob: String::new(),
            });
            cursor += 4;
        } else if field.starts_with(':') {
            let raw: Vec<_> = field.split_whitespace().collect();
            if raw.len() != 5 || cursor >= fields.len() {
                return Err(history_error("Invalid Git history record"));
            }
            let mut path = fields[cursor];
            cursor += 1;
            if raw[4].starts_with(['R', 'C']) {
                path = fields
                    .get(cursor)
                    .ok_or_else(|| history_error("Invalid Git rename record"))?;
                cursor += 1;
            }
            if raw[2] == raw[3] || !raw[1].starts_with("100") || raw[3].bytes().all(|ch| ch == b'0')
            {
                continue;
            }
            if let Some(mut version) = commit.clone() {
                version.path = path.to_string();
                version.blob = raw[3].to_string();
                versions.push(version);
            }
        }
    }
    Ok(versions)
}

async fn versions(root: &Path, path: &str) -> Result<Vec<FileHistoryVersion>, AppError> {
    // An unborn repository has no committed versions yet.
    if !git_output(root, &["rev-parse", "--verify", "HEAD"])
        .await?
        .status
        .success()
    {
        return Ok(Vec::new());
    }
    let bytes = git(
        root,
        &[
            "log",
            "--follow",
            "--raw",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "-z",
            "--format=%x00%H%x00%cn%x00%ce%x00%cI%x00%s%x00",
            "HEAD",
            "--",
            path,
        ],
    )
    .await?;
    parse_history(&bytes)
}

async fn selected_version(
    target: &HistoryTarget,
    commit: &str,
) -> Result<(PathBuf, FileHistoryVersion), AppError> {
    if !is_oid(commit) {
        return Err(history_error("Invalid commit ID"));
    }
    let (root, path) = repository(&target.path)
        .await?
        .ok_or_else(|| history_error("This file is not in a Git repository"))?;
    let version = versions(&root, &path)
        .await?
        .into_iter()
        .find(|version| version.commit == commit)
        .ok_or_else(|| {
            history_error("This version is no longer in the file history. Refresh and try again.")
        })?;
    Ok((root, version))
}

const MAX_VERSION_BYTES: u64 = 256 * 1024 * 1024;

async fn read_blob(root: &Path, oid: &str) -> Result<Vec<u8>, AppError> {
    let size = git(root, &["cat-file", "-s", oid]).await?;
    let size: u64 = String::from_utf8_lossy(&size)
        .trim()
        .parse()
        .map_err(history_error)?;
    if size > MAX_VERSION_BYTES {
        return Err(history_error("This file version exceeds the 256 MB limit"));
    }
    git(root, &["cat-file", "blob", oid]).await
}

async fn read_version(root: &Path, version: &FileHistoryVersion) -> Result<Vec<u8>, AppError> {
    let bytes = read_blob(root, &version.blob).await?;
    if let Ok(pointer) = std::str::from_utf8(&bytes) {
        if crate::diff::content::parse_lfs_pointer(pointer).is_some() {
            let size = pointer
                .lines()
                .find_map(|line| line.strip_prefix("size "))
                .and_then(|size| size.parse::<u64>().ok())
                .ok_or_else(|| history_error("Invalid Git LFS pointer"))?;
            if size > MAX_VERSION_BYTES {
                return Err(history_error("This file version exceeds the 256 MB limit"));
            }
            let spec = format!("{}:{}", version.commit, version.path);
            let resolved = git(root, &["cat-file", "--filters", &spec]).await?;
            // Never overwrite a binary working file with an unresolved LFS pointer.
            if resolved.len() as u64 != size || resolved == bytes {
                return Err(history_error(
                    "The Git LFS content for this version is unavailable",
                ));
            }
            return Ok(resolved);
        }
    }
    Ok(bytes)
}

fn read_current(path: &Path) -> Result<Vec<u8>, AppError> {
    if std::fs::metadata(path).map_err(history_error)?.len() > MAX_VERSION_BYTES {
        return Err(history_error("This file exceeds the 256 MB limit"));
    }
    std::fs::read(path).map_err(history_error)
}

#[tauri::command]
pub async fn file_history(
    project_id: String,
    path: String,
    workspace_ref: Option<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<FileHistory, AppError> {
    let target = resolve_target(registry.inner(), project_id, &path, workspace_ref.as_ref())?;
    let current = read_current(&target.path)?;
    let modified = file_revision(&target.path)?.modified_at_nanos;
    let repo = repository(&target.path).await?;
    let versions = if let Some((root, path)) = &repo {
        versions(root, path).await?
    } else {
        Vec::new()
    };
    Ok(FileHistory {
        versions,
        current_modified_at: modified,
        current_hash: blake3::hash(&current).to_hex().to_string(),
        is_repository: repo.is_some(),
    })
}

#[tauri::command]
pub async fn file_history_preview(
    project_id: String,
    path: String,
    commit: Option<String>,
    workspace_ref: Option<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
    binary_cache: State<'_, Arc<crate::binary_cache::BinaryCache>>,
) -> Result<FileHistoryPreview, AppError> {
    let target = resolve_target(registry.inner(), project_id, &path, workspace_ref.as_ref())?;
    let (bytes, csv_view) = if let Some(commit) = commit {
        let (root, version) = selected_version(&target, &commit).await?;
        let bytes = read_version(&root, &version).await?;
        let view = if target
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        {
            let spec = format!("{}:{}.view", version.commit, version.path);
            match git(&root, &["rev-parse", "--verify", &spec]).await {
                Ok(oid) => read_blob(&root, String::from_utf8_lossy(&oid).trim())
                    .await
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok()),
                Err(_) => None,
            }
        } else {
            None
        };
        (bytes, view)
    } else {
        let view = if target
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        {
            std::fs::read_to_string(target.path.with_extension("csv.view")).ok()
        } else {
            None
        };
        (read_current(&target.path)?, view)
    };
    let hash = blake3::hash(&bytes).to_hex().to_string();
    // Reuse type-specific parsers against an isolated snapshot, never by
    // replacing the workspace file while browsing history.
    let temp = tempfile::tempdir().map_err(history_error)?;
    let snapshot = temp.path().join(
        target
            .path
            .file_name()
            .ok_or_else(|| history_error("Invalid filename"))?,
    );
    std::fs::write(&snapshot, &bytes).map_err(history_error)?;
    let asset = super::super::asset::preview_file_history_snapshot(
        &snapshot,
        target.scope.as_ref(),
        binary_cache.inner(),
    );
    // A snapshot the asset viewer renders needs no second copy of its bytes.
    let viewer_cache = asset.is_none().then(|| binary_cache.inner().as_ref());
    let mut file = build_file_preview(registry.inner(), &target.project_id, &snapshot, viewer_cache)?;
    file.path = target.path.to_string_lossy().into_owned();
    file.editable = false;
    Ok(FileHistoryPreview {
        file,
        asset,
        content_hash: hash,
        csv_view,
    })
}

#[tauri::command]
pub async fn file_history_restore(
    project_id: String,
    path: String,
    commit: String,
    expected_content_hash: String,
    workspace_ref: Option<WorkspaceRef>,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let target = resolve_target(registry.inner(), project_id, &path, workspace_ref.as_ref())?;
    let (root, version) = selected_version(&target, &commit).await?;
    let bytes = read_version(&root, &version).await?;
    let _document_guard = super::super::view_documents::DOCUMENT_WRITE_LOCK
        .lock()
        .map_err(history_error)?;
    let _guard = CSV_FILE_WRITE_LOCK.lock().map_err(history_error)?;
    let canonical = dunce::canonicalize(&target.path).map_err(history_error)?;
    if canonical != target.path {
        return Err(history_error("The file path changed. Reopen its history."));
    }
    // Keep managed and read-only knowledge sources protected, as in ordinary file actions.
    if let Some(scope) = target.scope.as_ref() {
        use tauri::Manager;
        let dir: State<'_, crate::commands::knowledge::AppKnowledgeDir> = app_handle.state();
        let sources = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
            &scope.runtime().root().to_string_lossy(),
            dir.0.as_ref().as_ref(),
        );
        if let Some(source) = sources.classify_path(&canonical) {
            if source.kind
                != crate::knowledge_source_registry::KnowledgeSourceKind::WorkspaceKnowledge
                || !source.mutability.is_writable()
            {
                return Err(history_error(
                    "This knowledge source is managed or read-only",
                ));
            }
        }
    }
    restore_bytes(&canonical, &bytes, &expected_content_hash)?;
    if let Some(scope) = target.scope.as_ref() {
        use crate::workspace_changes::{
            hub_for_workspace, WorkspaceChangeKind, WorkspaceChangeSource,
        };
        let workspace_root = scope.runtime().root();
        let root = dunce::canonicalize(workspace_root).map_err(history_error)?;
        if let Ok(relative) = canonical.strip_prefix(&root) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if let Some(event) = hub_for_workspace(workspace_root).observe(
                &relative,
                WorkspaceChangeKind::Upsert,
                WorkspaceChangeSource::LocusWrite,
            ) {
                crate::workspace_service::event::emit_for_workspace_scope(
                    &app_handle,
                    &crate::workspace_service::event::WorkspaceEventScope::for_runtime(
                        scope.runtime(),
                    ),
                    "workspace-file-changed",
                    event,
                );
            }
        }
    }
    Ok(())
}

fn restore_bytes(path: &Path, bytes: &[u8], expected_hash: &str) -> Result<(), AppError> {
    let current = read_current(path)?;
    if blake3::hash(&current).to_hex().as_str() != expected_hash {
        return Err(AppError::new(
            "workspace.explorer_file_changed",
            "The file changed on disk. Refresh its history before restoring.",
        ));
    }
    let permissions = std::fs::metadata(path)
        .map_err(history_error)?
        .permissions();
    if permissions.readonly() {
        return Err(history_error("This file is read-only"));
    }
    crate::config::atomic_write_config(path, bytes).map_err(history_error)?;
    std::fs::set_permissions(path, permissions).map_err(history_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn init() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init", "-b", "main"]).await.unwrap();
        git(temp.path(), &["config", "user.name", "History Tester"])
            .await
            .unwrap();
        git(
            temp.path(),
            &["config", "user.email", "history@example.invalid"],
        )
        .await
        .unwrap();
        git(temp.path(), &["config", "core.autocrlf", "false"])
            .await
            .unwrap();
        temp
    }
    async fn commit(root: &Path, subject: &str) {
        git(root, &["add", "--all"]).await.unwrap();
        git(root, &["commit", "--allow-empty", "-m", subject])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn file_history_follows_renames_and_only_lists_content_changes() {
        let temp = init().await;
        let root = temp.path();
        let old = "旧 名 [1].md";
        let new = "new name.md";
        std::fs::write(root.join(old), "first\n").unwrap();
        commit(root, "first version").await;
        std::fs::write(root.join("unrelated.txt"), "other").unwrap();
        commit(root, "unrelated change").await;
        commit(root, "empty commit").await;
        git(root, &["mv", "--", old, new]).await.unwrap();
        commit(root, "rename only").await;
        std::fs::write(root.join(new), "second\n").unwrap();
        commit(root, "second version").await;
        std::fs::write(root.join(new), "first\n").unwrap();
        commit(root, "revert contents").await;
        let list = versions(root, new).await.unwrap();
        assert_eq!(
            list.iter().map(|v| v.subject.as_str()).collect::<Vec<_>>(),
            vec!["revert contents", "second version", "first version"]
        );
        assert_eq!(list[2].path, old);
        assert_eq!(list[0].author, "History Tester");
        assert_eq!(list[0].email, "history@example.invalid");
        assert_eq!(list[0].blob, list[2].blob); // Reverts are real changes, not globally deduplicated.
        assert_eq!(read_version(root, &list[2]).await.unwrap(), b"first\n");
    }

    #[tokio::test]
    async fn file_history_restores_binary_without_touching_head_index_or_other_files() {
        let temp = init().await;
        let root = temp.path();
        let path = root.join("image.png");
        let old = b"\x89PNG\0old\xff";
        std::fs::write(&path, old).unwrap();
        std::fs::write(root.join("other.txt"), "other").unwrap();
        commit(root, "image version").await;
        let head = git(root, &["rev-parse", "HEAD"]).await.unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        std::fs::write(&path, b"current").unwrap();
        let history = versions(root, "image.png").await.unwrap();
        let bytes = read_version(root, &history[0]).await.unwrap();
        assert!(restore_bytes(&path, &bytes, "stale").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"current");
        restore_bytes(&path, &bytes, blake3::hash(b"current").to_hex().as_str()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(git(root, &["rev-parse", "HEAD"]).await.unwrap(), head);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(root.join("other.txt")).unwrap(),
            "other"
        );
    }

    #[tokio::test]
    async fn file_history_handles_untracked_and_unborn_repositories() {
        let temp = init().await;
        assert!(versions(temp.path(), "not committed.txt")
            .await
            .unwrap()
            .is_empty());
        commit(temp.path(), "empty").await;
        assert!(versions(temp.path(), "not committed.txt")
            .await
            .unwrap()
            .is_empty());
        let plain = tempfile::tempdir().unwrap();
        std::fs::write(plain.path().join("note.md"), "text").unwrap();
        assert!(repository(&plain.path().join("note.md"))
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn file_history_skips_mode_only_changes_and_preserves_unusual_paths() {
        let commit = "a".repeat(40);
        let before = "b".repeat(40);
        let after = "c".repeat(40);
        let raw = format!("\0{commit}\0Name\0mail\02026-10-09T12:00:00+08:00\0subject\0\n:100644 100755 {before} {before} M\0mode.txt\0\n:100644 100644 {before} {after} M\0tab\tand\nline.txt\0");
        let list = parse_history(raw.as_bytes()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, "tab\tand\nline.txt");
    }
}
