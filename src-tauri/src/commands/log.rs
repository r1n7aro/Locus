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
