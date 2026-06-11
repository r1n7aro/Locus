use tauri::State;

use crate::error::AppError;

#[tauri::command]
pub async fn csharp_lsp_get_status() -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError>
{
    Ok(crate::csharp_lsp::status().await)
}

#[tauri::command]
pub async fn csharp_lsp_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError> {
    config
        .set_csharp_lsp_enabled(value)
        .map_err(|error| AppError::new("csharp_lsp.persist_failed", error))?;

    let cwd = workspace.path.read().await.clone();
    let warm_target = (!cwd.trim().is_empty()).then_some(cwd);
    crate::csharp_lsp::set_enabled(value, warm_target).await;
    Ok(crate::csharp_lsp::status().await)
}

#[tauri::command]
pub async fn csharp_lsp_restart(
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.trim().is_empty() {
        return Err(AppError::new(
            "csharp_lsp.no_workspace",
            "No workspace selected",
        ));
    }
    crate::csharp_lsp::restart(&cwd)
        .await
        .map_err(|error| AppError::new("csharp_lsp.restart_failed", error))?;
    Ok(crate::csharp_lsp::status().await)
}
