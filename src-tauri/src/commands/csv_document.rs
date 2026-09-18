use super::*;
#[cfg(test)]
use crate::csv_document::view_path as csv_view_path;
use crate::csv_document::{read_view_file as read_csv_view_file, CsvViewFile};

fn resolve_csv_file(
    file_path: &str,
    workspace_ref: Option<&WorkspaceRef>,
    project_id: Option<String>,
    registry: &ProjectRegistry,
) -> Result<(Option<ResolvedWorkspaceScope>, PathBuf, PathBuf), AppError> {
    let (scope, path, root) = if let Some(reference) = workspace_ref {
        let (scope, _, path) = resolve_workspace_file(reference, file_path, registry)?;
        {
            let root = scope.runtime().root().to_path_buf();
            (Some(scope), path, root)
        }
    } else {
        let (id, root) = resolve_project(registry, project_id.unwrap_or_default(), "csvView")?;
        let tree = crate::workspace_tree::snapshot(&root, id.as_str())
            .map_err(|error| explorer_error(error, "csvView"))?;
        (
            None,
            ensure_preview_path_authorized(&tree, Path::new(file_path))?,
            PathBuf::from(root),
        )
    };
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("csv")
    {
        return Err(AppError::new(
            "csv.invalid_path",
            "A CSV document is required.",
        ));
    }
    Ok((scope, path, root))
}

#[tauri::command]
pub async fn csv_view_read(
    file_path: String,
    workspace_ref: Option<WorkspaceRef>,
    project_id: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<CsvViewFile, AppError> {
    let (_scope, csv, _root) = resolve_csv_file(
        &file_path,
        workspace_ref.as_ref(),
        project_id,
        registry.inner().as_ref(),
    )?;
    read_csv_view_file(&csv)
}

#[tauri::command]
pub async fn csv_view_write(
    file_path: String,
    workspace_ref: Option<WorkspaceRef>,
    project_id: Option<String>,
    content: String,
    expected_content_hash: Option<String>,
    expected_csv_hash: String,
    registry: State<'_, Arc<ProjectRegistry>>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
) -> Result<CsvViewFile, AppError> {
    let (_scope, csv, root) = resolve_csv_file(
        &file_path,
        workspace_ref.as_ref(),
        project_id,
        registry.inner().as_ref(),
    )?;
    crate::csv_document::ensure_write_allowed(
        &root,
        &csv,
        app_knowledge_dir.0.as_ref().as_ref(),
        false,
    )?;
    crate::csv_document::write_view_file(
        &csv,
        &content,
        expected_content_hash.as_deref(),
        &expected_csv_hash,
    )
}

fn csv_companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn copy_new_file(source: &Path, target: &Path) -> Result<(), String> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(target.parent().ok_or("Missing target directory")?)
            .map_err(|error| error.to_string())?;
    let mut input = std::fs::File::open(source).map_err(|error| error.to_string())?;
    std::io::copy(&mut input, &mut temporary).map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist_noclobber(target)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn relocate_csv_files(source: &Path, target: &Path, copy: bool) -> Result<(), String> {
    for suffix in ["", ".view", ".meta", ".view.meta"] {
        if csv_companion(target, suffix).exists() {
            return Err(format!(
                "Target or companion already exists: {}{suffix}",
                target.display()
            ));
        }
    }
    let suffixes: &[&str] = if copy {
        &["", ".view"]
    } else {
        &["", ".meta", ".view", ".view.meta"]
    };
    let mut files = Vec::new();
    for suffix in suffixes {
        let from = csv_companion(source, suffix);
        let to = csv_companion(target, suffix);
        if to.exists() {
            return Err(format!("Target already exists: {}", to.display()));
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&from) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("CSV companions must be regular files".to_string());
            }
            files.push((from, to));
        }
    }
    let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in files {
        let result = if copy {
            copy_new_file(&from, &to)
        } else {
            std::fs::rename(&from, &to).map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (old, new) in completed.into_iter().rev() {
                let undo = if copy {
                    std::fs::remove_file(&new)
                } else {
                    std::fs::rename(&new, &old)
                };
                if let Err(error) = undo {
                    rollback_errors.push(error.to_string());
                }
            }
            return Err(format!(
                "CSV file operation failed: {error}. Rollback errors: {}",
                rollback_errors.join("; ")
            ));
        }
        completed.push((from, to));
    }
    Ok(())
}

#[tauri::command]
pub async fn csv_file_relocate(
    file_path: String,
    target_path: String,
    copy: bool,
    expected_content_hash: String,
    workspace_ref: Option<WorkspaceRef>,
    project_id: Option<String>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let (_scope, source, _root) = resolve_csv_file(
        &file_path,
        workspace_ref.as_ref(),
        project_id,
        registry.inner().as_ref(),
    )?;
    let requested = Path::new(target_path.trim());
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        source.parent().unwrap().join(requested)
    };
    if candidate
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("csv"))
    {
        return Err(AppError::new(
            "csv.invalid_path",
            "Target must be a CSV file.",
        ));
    }
    let parent = candidate
        .parent()
        .and_then(|path| dunce::canonicalize(path).ok())
        .ok_or_else(|| AppError::new("csv.invalid_path", "The target directory does not exist."))?;
    let target = parent.join(
        candidate
            .file_name()
            .ok_or_else(|| AppError::new("csv.invalid_path", "Missing file name."))?,
    );
    if let Some(reference) = workspace_ref.as_ref() {
        resolve_workspace_file_candidate(
            reference,
            &target.to_string_lossy(),
            registry.inner().as_ref(),
        )?;
    } else if parent != source.parent().unwrap() {
        return Err(AppError::new(
            "csv.invalid_path",
            "Mounted CSV files can only be copied or renamed within their directory.",
        ));
    }
    let _guard = CSV_FILE_WRITE_LOCK
        .lock()
        .map_err(|error| AppError::new("csv.write_lock", error.to_string()))?;
    let bytes = std::fs::read(&source)
        .map_err(|error| AppError::new("csv.read_failed", error.to_string()))?;
    if expected_content_hash.is_empty()
        || blake3::hash(&bytes).to_hex().as_str() != expected_content_hash
    {
        return Err(AppError::new(
            "csv.file_changed",
            "The CSV changed on disk. Reload before moving or copying.",
        ));
    }
    relocate_csv_files(&source, &target, copy)
        .map_err(|error| AppError::new("csv.relocate_failed", error))?;
    if let Some(scope) = _scope.as_ref() {
        if let Ok(root) = dunce::canonicalize(scope.runtime().root()) {
            if let Ok(relative) = target.strip_prefix(root) {
                return Ok(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(target.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_view_is_optional_and_does_not_touch_unity_meta() {
        let root = tempfile::tempdir().unwrap();
        let csv = root.path().join("items.csv");
        let unity_meta = root.path().join("items.csv.meta");
        std::fs::write(&csv, "id,name\n001,test\n").unwrap();
        std::fs::write(&unity_meta, "guid: preserved\n").unwrap();
        let view = csv_view_path(&csv).unwrap();
        assert_eq!(view, root.path().join("items.csv.view"));
        assert!(read_csv_view_file(&csv).unwrap().text.is_none());
        std::fs::write(&view, "schema: locus.csv-view.v1\n").unwrap();
        let loaded = read_csv_view_file(&csv).unwrap();
        assert!(loaded.content_hash.is_some());
        assert_eq!(
            std::fs::read_to_string(unity_meta).unwrap(),
            "guid: preserved\n"
        );
    }

    #[test]
    fn rename_preserves_unity_guids_and_copy_creates_new_asset_identity() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.csv");
        for (suffix, content) in [
            ("", "id\n001\n"),
            (".view", "schema: locus.csv-view.v1\n"),
            (".meta", "guid: original\n"),
            (".view.meta", "guid: view\n"),
        ] {
            std::fs::write(csv_companion(&source, suffix), content).unwrap();
        }
        let copied = root.path().join("copy.csv");
        relocate_csv_files(&source, &copied, true).unwrap();
        assert!(csv_companion(&copied, ".view").is_file());
        assert!(!csv_companion(&copied, ".meta").exists());
        let renamed = root.path().join("renamed.csv");
        relocate_csv_files(&source, &renamed, false).unwrap();
        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(csv_companion(&renamed, ".meta")).unwrap(),
            "guid: original\n"
        );
        assert!(relocate_csv_files(&renamed, &copied, false).is_err());
        assert!(renamed.is_file());
    }

    #[test]
    fn view_schema_rejects_duplicate_keys_and_dangling_columns() {
        let valid = "schema: locus.csv-view.v1\nheaderRows: 1\nrowHeight: 28\nwrapText: false\nfrozenColumns: 0\ncolumns:\n  c1:\n    sourceIndex: 0\n    header: id\n    width: 120\ncolumnOrder: [c1]\n";
        assert!(crate::csv_document::parse_view(valid).is_ok());
        assert!(crate::csv_document::parse_view(&format!("{valid}rowHeight: 40\n")).is_err());
        assert!(crate::csv_document::parse_view(&valid.replace("[c1]", "[missing]")).is_err());
        assert!(crate::csv_document::parse_view(&valid.replace("width: 120", "width: 0")).is_err());
    }
}
