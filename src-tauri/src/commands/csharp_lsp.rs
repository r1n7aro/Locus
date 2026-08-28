use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::workspace_service::{ProjectRegistry, ResolvedWorkspaceScope, WorkspaceRef};

fn resolve_scope(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<ResolvedWorkspaceScope, AppError> {
    super::session::resolve_workspace_scope(workspace_registry, workspace_ref, operation)
}

fn scope_root(scope: &ResolvedWorkspaceScope) -> String {
    scope.runtime().root().to_string_lossy().to_string()
}

async fn scope_lsp_status(
    scope: &ResolvedWorkspaceScope,
) -> crate::csharp_lsp::CsharpLspStatusPayload {
    crate::csharp_lsp::status_for_checkout(
        scope.runtime().checkout_id(),
        Some(scope.runtime().generation()),
    )
    .await
}

async fn scope_compile_status(
    scope: &ResolvedWorkspaceScope,
) -> crate::csharp_compile::CsharpCompileStatusPayload {
    crate::csharp_compile::status_for_project(&scope_root(scope)).await
}

#[tauri::command]
pub async fn csharp_lsp_get_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "csharp_lsp_get_status",
    )?;
    Ok(scope_lsp_status(&scope).await)
}

#[tauri::command]
pub async fn csharp_lsp_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "csharp_lsp_set_enabled",
    )?;
    config
        .set_csharp_lsp_enabled(value)
        .map_err(|error| AppError::new("csharp_lsp.persist_failed", error))?;

    let warm_target = value.then(|| scope_root(&scope));
    crate::csharp_lsp::set_enabled(value, warm_target).await;
    Ok(scope_lsp_status(&scope).await)
}

#[tauri::command]
pub async fn unity_sidecar_compiler_get_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_sidecar_compiler_get_status",
    )?;
    Ok(crate::csharp_compile::refresh_status_for_project(&scope_root(&scope)).await)
}

#[tauri::command]
pub async fn unity_sidecar_compiler_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_sidecar_compiler_set_enabled",
    )?;
    config
        .set_unity_sidecar_compiler_enabled(value)
        .map_err(|error| AppError::new("csharp_compile.persist_failed", error))?;

    crate::csharp_compile::set_enabled(value).await;
    if value {
        crate::csharp_compile::warm_up_in_background();
    }
    Ok(scope_compile_status(&scope).await)
}

#[tauri::command]
pub async fn unity_non_public_access_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_non_public_access_set_enabled",
    )?;
    config
        .set_unity_non_public_access_enabled(value)
        .map_err(|error| AppError::new("csharp_compile.persist_failed", error))?;

    crate::csharp_compile::set_non_public_access_enabled(value);
    Ok(scope_compile_status(&scope).await)
}

#[tauri::command]
pub async fn unity_in_process_compile_fallback_get_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.unity_in_process_compile_fallback_enabled())
}

/// Toggle the in-Unity Roslyn fallback used when the sidecar is on but a
/// compile is unavailable. Off = pure-sidecar (no in-process compile runs);
/// on = graceful fallback. Useful for A-B testing the sidecar in isolation.
#[tauri::command]
pub async fn unity_in_process_compile_fallback_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_in_process_compile_fallback_set_enabled",
    )?;
    config
        .set_unity_in_process_compile_fallback_enabled(value)
        .map_err(|error| AppError::new("csharp_compile.persist_failed", error))?;

    crate::csharp_compile::set_in_process_fallback(value);
    Ok(scope_compile_status(&scope).await)
}

#[tauri::command]
pub async fn unity_hot_reload_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_set_enabled",
    )?;
    config
        .set_unity_hot_reload_enabled(value)
        .map_err(|error| AppError::new("unity_hotreload.persist_failed", error))?;

    crate::unity_hotreload::set_enabled(value);
    Ok(scope_compile_status(&scope).await)
}

/// Experimental (Phase B, default off): toggle whether the Unity plugin may
/// force-JIT a synthetic caller stub to evaluate a method's inline risk. Persists
/// to config and updates the live module flag delivered in each hot-patch payload.
#[tauri::command]
pub async fn unity_inline_force_evaluate_set_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_compile::CsharpCompileStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_inline_force_evaluate_set_enabled",
    )?;
    config
        .set_unity_inline_force_evaluate_enabled(value)
        .map_err(|error| AppError::new("unity_hotreload.persist_failed", error))?;

    crate::unity_hotreload::set_inline_force_evaluate_enabled(value);
    Ok(scope_compile_status(&scope).await)
}

#[tauri::command]
pub async fn unity_hot_reload_selftest_run(
    app: tauri::AppHandle,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_selftest_run",
    )
    .await?;
    let cwd = ready.root_text();
    let event_scope = ready.checkout_event_scope();
    crate::unity_hotreload::selftest::run_scoped(app, cwd, event_scope)
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
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<serde_json::Value, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_access_probe_run",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_hotreload::coordinator::access_probe_run(&cwd)
        .await
        .map_err(|error| AppError::new("unity_hotreload.access_probe_failed", error))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotReloadPreflight {
    /// Whether a Unity editor answered the probe.
    pub connected: bool,
    /// "debug" | "release" when readable; `None` when the editor is
    /// unreachable or the value could not be parsed.
    pub code_optimization: Option<String>,
    /// Whether entering Play Mode reloads the domain (`Some(true)` = Unity's
    /// default reload, `Some(false)` = DisableDomainReload); `None` when the
    /// editor is unreachable or the plugin predates the toggle.
    pub domain_reload_on_play: Option<bool>,
}

/// Enable-time check the toggle UI runs before turning hot reload on: report
/// the connected editor's Code Optimization so the UI can warn (and offer to
/// auto-switch) when it is Release. Independent of the `unity_hot_reload`
/// feature flag. Never errors on a missing editor — the UI treats "can't tell"
/// as "go ahead", and the execution-time probe still gates real hot reloads.
#[tauri::command]
pub async fn unity_hot_reload_preflight(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<HotReloadPreflight, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_preflight",
    )
    .await?;
    let cwd = ready.root_text();
    let (connected, code_optimization, domain_reload_on_play) =
        crate::unity_hotreload::coordinator::detect_hot_reload_editor_settings(&cwd).await;
    Ok(HotReloadPreflight {
        connected,
        code_optimization,
        domain_reload_on_play,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOptimizationResult {
    pub code_optimization: String,
}

/// Switch the connected editor's Code Optimization to Debug (the auto-fix the
/// user confirms in the enable-time prompt). Triggers a Unity script recompile.
#[tauri::command]
pub async fn unity_hot_reload_set_code_optimization_debug(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<CodeOptimizationResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_set_code_optimization_debug",
    )
    .await?;
    let cwd = ready.root_text();
    let code_optimization = crate::unity_hotreload::coordinator::set_code_optimization_debug(&cwd)
        .await
        .map_err(|error| AppError::new("unity_hotreload.set_code_optimization_failed", error))?;
    Ok(CodeOptimizationResult { code_optimization })
}

/// Switch the connected editor's Code Optimization to an explicit level
/// ("debug" | "release"), driven by the hot-reload popover dropdown. Triggers a
/// Unity script recompile, exactly like flipping the Editor's status-bar icon.
#[tauri::command]
pub async fn unity_hot_reload_set_code_optimization(
    level: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<CodeOptimizationResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_set_code_optimization",
    )
    .await?;
    let cwd = ready.root_text();
    let code_optimization =
        crate::unity_hotreload::coordinator::set_code_optimization(&cwd, &level)
            .await
            .map_err(|error| {
                AppError::new("unity_hotreload.set_code_optimization_failed", error)
            })?;
    Ok(CodeOptimizationResult { code_optimization })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayModeReloadResult {
    pub domain_reload_on_play: bool,
}

/// Set whether entering Play Mode reloads the domain, driven by the manual
/// hot-reload popover toggle (EditorSettings.enterPlayModeOptions /
/// DisableDomainReload). Unlike the Code Optimization switch this does NOT
/// trigger a Unity recompile. Returns the resulting effective value.
#[tauri::command]
pub async fn unity_hot_reload_set_play_mode_reload(
    domain_reload: bool,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<PlayModeReloadResult, AppError> {
    let ready = super::workspace::resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_hot_reload_set_play_mode_reload",
    )
    .await?;
    let cwd = ready.root_text();
    let domain_reload_on_play =
        crate::unity_hotreload::coordinator::set_play_mode_reload(&cwd, domain_reload)
            .await
            .map_err(|error| AppError::new("unity_hotreload.set_play_mode_reload_failed", error))?;
    Ok(PlayModeReloadResult {
        domain_reload_on_play,
    })
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
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::config::CodeAnalysisToolsConfig, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "code_analysis_tools_set_config",
    )?;
    let previous = config.code_analysis_tools();
    config
        .set_code_analysis_tools(value)
        .map_err(|error| AppError::new("code_analysis.persist_failed", error))?;
    crate::code_tools::set(value);

    // The analyzer set is wired into the language server workspace at startup
    // (Directory.Build.props), so flipping it only takes effect after a
    // server restart. Do that in the background when one is running.
    if previous.unity_analyzers != value.unity_analyzers && crate::csharp_lsp::is_enabled() {
        let cwd = scope_root(&scope);
        tokio::spawn(async move {
            let _ = crate::csharp_lsp::restart(&cwd).await;
        });
    }
    Ok(config.code_analysis_tools())
}

#[tauri::command]
pub async fn csharp_lsp_restart(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::csharp_lsp::CsharpLspStatusPayload, AppError> {
    let scope = resolve_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "csharp_lsp_restart",
    )?;
    let cwd = scope_root(&scope);
    crate::csharp_lsp::restart(&cwd)
        .await
        .map_err(|error| AppError::new("csharp_lsp.restart_failed", error))?;
    Ok(scope_lsp_status(&scope).await)
}
