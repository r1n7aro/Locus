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
pub async fn unity_sidecar_compiler_get_status(
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    Ok(crate::csharp_compile::refresh_status().await)
}

#[tauri::command]
pub async fn unity_sidecar_compiler_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    config
        .set_unity_sidecar_compiler_enabled(value)
        .map_err(|error| AppError::new("csharp_compile.persist_failed", error))?;

    crate::csharp_compile::set_enabled(value).await;
    if value {
        crate::csharp_compile::warm_up_in_background();
    }
    Ok(crate::csharp_compile::status().await)
}

#[tauri::command]
pub async fn unity_hot_reload_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    config
        .set_unity_hot_reload_enabled(value)
        .map_err(|error| AppError::new("unity_hotreload.persist_failed", error))?;

    crate::unity_hotreload::set_enabled(value);
    Ok(crate::csharp_compile::status().await)
}

#[tauri::command]
pub async fn unity_hot_reload_selftest_run(
    app: tauri::AppHandle,
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_hotreload::selftest::run(app, cwd)
        .await
        .map_err(|error| AppError::new("unity_hotreload.selftest_failed", error))
}

/// C0 diagnostic: run (or return the cached) runtime access-capability probe
/// against the connected Unity editor and return the full matrix JSON
/// (`{cached, domainGeneration, caps, matrix}`). Needs the sidecar compiler
/// and a connected editor with a current plugin; independent of the
/// `unity_hot_reload` feature flag so it can be used to qualify an editor
/// before enabling hot reload.
#[tauri::command]
pub async fn unity_hot_reload_access_probe_run(
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<serde_json::Value, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.trim().is_empty() {
        return Err(AppError::new(
            "unity_hotreload.no_workspace",
            "No workspace selected",
        ));
    }
    crate::unity_hotreload::coordinator::access_probe_run(&cwd)
        .await
        .map_err(|error| AppError::new("unity_hotreload.access_probe_failed", error))
}

#[tauri::command]
pub async fn code_analysis_tools_get_config(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::config::CodeAnalysisToolsConfig, AppError> {
    Ok(config.code_analysis_tools())
}

#[tauri::command]
pub async fn code_analysis_tools_set_config(
    value: crate::config::CodeAnalysisToolsConfig,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<crate::config::CodeAnalysisToolsConfig, AppError> {
    let previous = config.code_analysis_tools();
    config
        .set_code_analysis_tools(value)
        .map_err(|error| AppError::new("code_analysis.persist_failed", error))?;
    crate::code_tools::set(value);

    // The analyzer set is wired into the language server workspace at startup
    // (Directory.Build.props), so flipping it only takes effect after a
    // server restart. Do that in the background when one is running.
    if previous.unity_analyzers != value.unity_analyzers && crate::csharp_lsp::is_enabled() {
        let cwd = workspace.path.read().await.clone();
        if !cwd.trim().is_empty() {
            tokio::spawn(async move {
                let _ = crate::csharp_lsp::restart(&cwd).await;
            });
        }
    }
    Ok(config.code_analysis_tools())
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
