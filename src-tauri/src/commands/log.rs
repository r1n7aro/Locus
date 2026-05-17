use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::logging::{AppLogEntry, AppLogStore};

const DEFAULT_LOG_FETCH_LIMIT: usize = 2_000;

#[tauri::command]
pub async fn get_log_entries(
    limit: Option<usize>,
    logs: State<'_, Arc<AppLogStore>>,
) -> Result<Vec<AppLogEntry>, AppError> {
    let limit = limit
        .unwrap_or(DEFAULT_LOG_FETCH_LIMIT)
        .clamp(1, DEFAULT_LOG_FETCH_LIMIT);
    Ok(logs.snapshot(limit))
}

#[tauri::command]
pub async fn clear_log_entries(logs: State<'_, Arc<AppLogStore>>) -> Result<(), AppError> {
    logs.clear();
    Ok(())
}

#[tauri::command]
pub async fn save_log_export(file_path: String, content: String) -> Result<String, AppError> {
    let trimmed = file_path.trim();
    if trimmed.is_empty() {
        return Err(
            AppError::new("log.export.empty_path", "Log export path is empty")
                .operation("saveLogExport"),
        );
    }

    let mut path = std::path::PathBuf::from(trimmed);
    let has_log_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("log"))
        .unwrap_or(false);
    if !has_log_extension {
        path.set_extension("log");
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                "log.export.create_dir_failed",
                "Failed to create log export directory",
            )
            .detail(error.to_string())
            .operation("saveLogExport")
        })?;
    }

    std::fs::write(&path, content.as_bytes()).map_err(|error| {
        AppError::new("log.export.write_failed", "Failed to write log export")
            .detail(error.to_string())
            .operation("saveLogExport")
    })?;

    eprintln!("[Locus] exported console log to {}", path.display());
    Ok(path.to_string_lossy().to_string())
}
