//! CSV view persistence shared by the desktop editor and the Python SDK.
//! Cell data stays in the CSV; this service only writes its adjacent `.view`.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::AppError;

mod data;
mod excel;
mod merges;
mod patch;
mod styles;
mod view;
pub(crate) use patch::CsvViewPatch;
pub(crate) use view::{parse_view, CsvViewConfig};

pub(crate) static CSV_FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());
const CSV_VIEW_LIMIT: usize = 1024 * 1024;
const CSV_DATA_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvViewFile {
    pub text: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CsvViewSnapshot {
    pub file_path: String,
    pub revision: String,
    pub row_count: usize,
    pub column_count: usize,
    pub delimiter: String,
    pub view: CsvViewConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

pub(crate) fn view_path(csv: &Path) -> Result<PathBuf, AppError> {
    if !csv
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
    {
        return Err(AppError::new(
            "csv.invalid_path",
            "A CSV document is required.",
        ));
    }
    let mut name = csv.as_os_str().to_os_string();
    name.push(".view");
    let path = PathBuf::from(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(AppError::new(
                "csv.invalid_view_path",
                "The CSV view must be a regular adjacent file.",
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(AppError::new("csv.view_read_failed", error.to_string()));
        }
        _ => {}
    }
    Ok(path)
}

fn read_bytes(path: &Path, limit: usize, code: &str) -> Result<Vec<u8>, AppError> {
    let file = std::fs::File::open(path).map_err(|error| AppError::new(code, error.to_string()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::new(code, error.to_string()))?;
    if bytes.len() > limit {
        return Err(AppError::new(
            "csv.too_large",
            format!("File exceeds its {limit}-byte limit."),
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_view_file(csv: &Path) -> Result<CsvViewFile, AppError> {
    let path = view_path(csv)?;
    if !path
        .try_exists()
        .map_err(|error| AppError::new("csv.view_read_failed", error.to_string()))?
    {
        return Ok(CsvViewFile {
            text: None,
            content_hash: None,
        });
    }
    let bytes = read_bytes(&path, CSV_VIEW_LIMIT, "csv.view_read_failed")?;
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let text = String::from_utf8(bytes)
        .map_err(|error| AppError::new("csv.invalid_view_encoding", error.to_string()))?;
    Ok(CsvViewFile {
        text: Some(text),
        content_hash: Some(content_hash),
    })
}

fn write_view_unlocked(
    csv: &Path,
    content: &str,
    expected_view_hash: Option<&str>,
    expected_csv_hash: &str,
) -> Result<CsvViewFile, AppError> {
    let next_view = parse_view(content)?;
    let data = read_bytes(csv, CSV_DATA_LIMIT, "csv.read_failed")?;
    if expected_csv_hash.is_empty() || blake3::hash(&data).to_hex().as_str() != expected_csv_hash {
        return Err(AppError::new(
            "csv.file_changed",
            "The CSV changed on disk. Read it again before saving the view.",
        ));
    }
    let current = read_view_file(csv)?;
    if current.content_hash.as_deref() != expected_view_hash {
        return Err(AppError::new(
            "csv.view_changed",
            "The CSV view changed on disk. Read it again before saving.",
        ));
    }
    if current
        .text
        .as_deref()
        .is_some_and(|text| parse_view(text).is_ok_and(|view| view.schema > next_view.schema))
    {
        return Err(AppError::new(
            "csv.schema_downgrade",
            "A CSV view cannot be overwritten with an older schema.",
        ));
    }
    if current.text.as_deref() == Some(content) {
        return Ok(current);
    }
    crate::config::atomic_write_config(&view_path(csv)?, content.as_bytes())
        .map_err(|error| AppError::new("csv.view_write_failed", error))?;
    // Return the committed version, not a later external writer's contents.
    Ok(CsvViewFile {
        text: Some(content.to_owned()),
        content_hash: Some(blake3::hash(content.as_bytes()).to_hex().to_string()),
    })
}

pub(crate) fn write_view_file(
    csv: &Path,
    content: &str,
    expected_view_hash: Option<&str>,
    expected_csv_hash: &str,
) -> Result<CsvViewFile, AppError> {
    let _guard = CSV_FILE_WRITE_LOCK
        .lock()
        .map_err(|error| AppError::new("csv.write_lock", error.to_string()))?;
    write_view_unlocked(csv, content, expected_view_hash, expected_csv_hash)
}

fn revision(csv: &Path, csv_hash: &str, view_hash: Option<&str>) -> Result<String, AppError> {
    let canonical = dunce::canonicalize(csv)
        .map_err(|error| AppError::new("csv.read_failed", error.to_string()))?;
    let key = canonical.to_string_lossy().replace('\\', "/");
    let key = if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    };
    let encoded = serde_json::to_vec(&(key, csv_hash, view_hash)).unwrap();
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

fn snapshot(
    csv: &Path,
    shape: &data::CsvShape,
    csv_hash: &str,
    stored: &CsvViewFile,
    view: CsvViewConfig,
) -> Result<CsvViewSnapshot, AppError> {
    Ok(CsvViewSnapshot {
        file_path: csv.to_string_lossy().replace('\\', "/"),
        revision: revision(csv, csv_hash, stored.content_hash.as_deref())?,
        row_count: shape.row_count,
        column_count: shape.column_count,
        delimiter: (shape.delimiter as char).to_string(),
        view,
        content: None,
    })
}

fn read_state(
    csv: &Path,
) -> Result<(data::CsvShape, String, CsvViewFile, CsvViewConfig), AppError> {
    let bytes = read_bytes(csv, CSV_DATA_LIMIT, "csv.read_failed")?;
    let csv_hash = blake3::hash(&bytes).to_hex().to_string();
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| AppError::new("csv.invalid_encoding", error.to_string()))?;
    let shape = data::parse_shape(text)?;
    let stored = read_view_file(csv)?;
    let mut view = match &stored.text {
        Some(text) => parse_view(text)?,
        None => CsvViewConfig::default(),
    };
    if stored.text.is_none() && shape.row_count == 0 {
        view.header_rows = 0;
    }
    view.reconcile(&shape);
    Ok((shape, csv_hash, stored, view))
}

pub(crate) fn read_view(csv: &Path) -> Result<CsvViewSnapshot, AppError> {
    let _guard = CSV_FILE_WRITE_LOCK
        .lock()
        .map_err(|error| AppError::new("csv.write_lock", error.to_string()))?;
    let (shape, csv_hash, stored, view) = read_state(csv)?;
    snapshot(csv, &shape, &csv_hash, &stored, view)
}

pub(crate) fn read_workbook(csv: &Path) -> Result<CsvViewSnapshot, AppError> {
    let _guard = CSV_FILE_WRITE_LOCK
        .lock()
        .map_err(|e| AppError::new("csv.write_lock", e.to_string()))?;
    let (shape, csv_hash, stored, view) = read_state(csv)?;
    let bytes = read_bytes(csv, CSV_DATA_LIMIT, "csv.read_failed")?;
    if blake3::hash(&bytes).to_hex().as_str() != csv_hash {
        return Err(AppError::new(
            "csv.file_changed",
            "CSV changed while loading the worksheet.",
        ));
    }
    let mut result = snapshot(csv, &shape, &csv_hash, &stored, view)?;
    result.content = Some(
        String::from_utf8(bytes)
            .map_err(|e| AppError::new("csv.invalid_encoding", e.to_string()))?,
    );
    Ok(result)
}

pub(crate) fn patch_view(
    csv: &Path,
    expected_revision: &str,
    patch: CsvViewPatch,
) -> Result<CsvViewSnapshot, AppError> {
    save_workbook(csv, expected_revision, patch, None)
}

/// Validate the whole request before writing. A failed CSV write restores the companion.
/// The revision guards both files; callers never need to manage a file lock.
pub(crate) fn save_workbook(
    csv: &Path,
    expected_revision: &str,
    patch: CsvViewPatch,
    content: Option<String>,
) -> Result<CsvViewSnapshot, AppError> {
    let _guard = CSV_FILE_WRITE_LOCK
        .lock()
        .map_err(|error| AppError::new("csv.write_lock", error.to_string()))?;
    let (shape, csv_hash, stored, mut view) = read_state(csv)?;
    if expected_revision.is_empty()
        || revision(csv, &csv_hash, stored.content_hash.as_deref())? != expected_revision
    {
        return Err(AppError::new(
            "csv.revision_changed",
            "The CSV or its view changed. Read the view again before applying the patch.",
        ));
    }
    let previous = view.clone();
    patch.apply(&mut view, &shape)?;
    let next_shape = if let Some(text) = content.as_ref() {
        if text.len() > CSV_DATA_LIMIT {
            return Err(AppError::new("csv.too_large", "CSV exceeds 16 MiB."));
        }
        Some(data::parse_shape(text)?)
    } else {
        None
    };
    let next_hash = content
        .as_ref()
        .map(|text| blake3::hash(text.as_bytes()).to_hex().to_string())
        .unwrap_or_else(|| csv_hash.clone());
    if let Some(next_shape) = next_shape.as_ref() {
        view.reconcile(next_shape);
    }
    if view == previous && next_hash == csv_hash {
        return snapshot(csv, &shape, &csv_hash, &stored, view);
    }
    let serialized = view.serialize()?;
    let written = if view != previous {
        write_view_unlocked(csv, &serialized, stored.content_hash.as_deref(), &csv_hash)?
    } else {
        stored.clone()
    };
    if next_hash != csv_hash {
        let save = (|| {
            let bytes = read_bytes(csv, CSV_DATA_LIMIT, "csv.read_failed")?;
            if blake3::hash(&bytes).to_hex().as_str() != csv_hash
                || read_view_file(csv)?.content_hash != written.content_hash
            {
                return Err(AppError::new(
                    "csv.revision_changed",
                    "CSV or layout changed while saving.",
                ));
            }
            crate::config::atomic_write_config(csv, content.as_ref().unwrap().as_bytes())
                .map_err(|e| AppError::new("csv.write_failed", e))
        })();
        if let Err(error) = save {
            if written.content_hash != stored.content_hash
                && read_view_file(csv)?.content_hash == written.content_hash
            {
                let rollback = match stored.text.as_ref() {
                    Some(text) => {
                        crate::config::atomic_write_config(&view_path(csv)?, text.as_bytes())
                    }
                    None => std::fs::remove_file(view_path(csv)?).map_err(|e| e.to_string()),
                };
                rollback.map_err(|e| {
                    AppError::new(
                        "csv.rollback_failed",
                        format!("{error}; restoring layout failed: {e}"),
                    )
                })?;
            }
            return Err(error);
        }
    }
    snapshot(
        csv,
        next_shape.as_ref().unwrap_or(&shape),
        &next_hash,
        &written,
        view,
    )
}

/// Resolve policy through the CSV owner, never through the `.view` suffix.
pub(crate) fn ensure_write_allowed(
    root: &Path,
    csv: &Path,
    app_knowledge_dir: Option<&PathBuf>,
    ai: bool,
) -> Result<(), AppError> {
    use crate::knowledge_source_registry::{KnowledgeSourceKind, KnowledgeSourceRegistry};
    if std::fs::metadata(csv)
        .map_err(|error| AppError::new("csv.read_failed", error.to_string()))?
        .permissions()
        .readonly()
    {
        return Err(AppError::new(
            "csv.read_only",
            "The CSV document is read-only.",
        ));
    }
    let working_dir = root.to_string_lossy();
    let registry = KnowledgeSourceRegistry::build(&working_dir, app_knowledge_dir);
    let Some(target) = registry.classify_path(csv) else {
        return Ok(());
    };
    if !target.mutability.is_writable() {
        return Err(AppError::new(
            "csv.read_only",
            format!(
                "Knowledge source is {}: {}",
                target.mutability.label(),
                target.display_path
            ),
        ));
    }
    if target.kind == KnowledgeSourceKind::WorkspaceKnowledge {
        let document = crate::knowledge_store::inspect_workspace_document_policy(
            &working_dir,
            target.doc_type,
            &target.logical_path,
        )
        .map_err(|error| AppError::new("csv.policy_failed", error))?;
        if document.read_only || (ai && !crate::knowledge_store::document_allows_ai_edit(&document))
        {
            return Err(AppError::new(
                "csv.read_only",
                "Editing is disabled for this knowledge document.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
