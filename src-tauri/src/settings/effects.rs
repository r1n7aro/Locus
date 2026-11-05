//! Async effects of settings that need the app handle or live workspaces.
//! Pure in-process mirrors live in `mirrors.rs`; settings not listed here
//! only need to be persisted.

use tauri::Manager;

use super::apply::WriteContext;
use super::definitions as keys;
use super::store::SettingChange;
use crate::workspace_service::ServiceKind;

pub(crate) async fn apply(context: &WriteContext, change: &SettingChange) -> Result<(), String> {
    let app = &context.app;
    let enabled = change.new.as_bool().unwrap_or(false);
    let previous = change.old.as_bool().unwrap_or(false);
    let key = change.key;

    if key == keys::SYSTEM_DEBUG.name() {
        crate::commands::apply_debug_mode(app, enabled).await
    } else if key == keys::AGENT_SESSION_UNDO.name() {
        crate::commands::publish_session_undo_enabled(app, enabled);
        Ok(())
    } else if key == keys::PROVIDERS_CLAUDE_CODE_ENABLED.name() {
        crate::commands::publish_claude_code_cli_enabled(app, enabled)
    } else if key == keys::UNITY_ENABLED.name() {
        crate::commands::apply_service_switch(app, ServiceKind::Unity, enabled).await
    } else if key == keys::BLENDER_ENABLED.name() {
        crate::commands::apply_service_switch(app, ServiceKind::Blender, enabled).await
    } else if key == keys::UNITY_EMBED_ENABLED.name() {
        crate::commands::apply_unity_embed_enabled(app, enabled, previous).await
    } else if key == keys::UNITY_CONNECTION_NATIVE_BRIDGE.name() {
        crate::commands::apply_unity_native_bridge(app, enabled, previous).await
    } else if key == keys::UNITY_CONNECTION_BACKGROUND_HOOK.name() {
        crate::commands::apply_unity_background_hook(
            app,
            enabled,
            previous,
            context.reporting_root.as_deref(),
        )
        .await
    } else if key == keys::UNITY_CONNECTION_STATE_PROBE.name() {
        crate::commands::apply_unity_state_probe(app, enabled).await
    } else if key == keys::UNITY_EXTERNAL_EDITOR_AUTO_SELECT.name() {
        crate::commands::apply_unity_external_editor_default(
            app,
            enabled,
            context.workspace_checkout.clone(),
        )
        .await
    } else if key == keys::UNITY_CODE_ANALYSIS_CSHARP_LSP.name() {
        let warm_target = context.workspace_root.clone().filter(|_| enabled);
        crate::csharp_lsp::set_enabled(enabled, warm_target).await;
        Ok(())
    } else if key == keys::UNITY_COMPILE_SIDECAR.name() {
        crate::csharp_compile::set_enabled(enabled).await;
        if enabled {
            crate::csharp_compile::warm_up_in_background();
        }
        Ok(())
    } else if key == keys::UNITY_CODE_ANALYSIS_UNITY_ANALYZERS.name() {
        // The analyzer set is wired into the language server workspace at
        // startup (Directory.Build.props), so it takes effect on a restart.
        if crate::csharp_lsp::is_enabled() {
            if let Some(root) = context.workspace_root.clone() {
                tokio::spawn(async move {
                    let _ = crate::csharp_lsp::restart(&root).await;
                });
            }
        }
        Ok(())
    } else if key.starts_with("workspace.resources.") || key.starts_with("blender.resources.") {
        if let Some(policy) =
            app.try_state::<std::sync::Arc<crate::resource_policy::ResourcePolicyStore>>()
        {
            policy.republish_from_config();
        }
        Ok(())
    } else {
        Ok(())
    }
}

/// Effects of project settings on the open checkout the write targeted.
/// The file is already written; these keep the live workspace in step.
pub(crate) fn apply_project(context: &WriteContext, changes: &[SettingChange]) {
    for change in changes.iter().filter(|change| change.old != change.new) {
        if change.key == keys::PROJECT_KNOWLEDGE_ENABLED.name() {
            apply_project_knowledge_enabled(context, change.new.as_bool().unwrap_or(true));
        }
    }
}

/// Same as saving the workspace setup dialog: disabling stops the knowledge
/// watchers, and every window of the checkout learns the new state.
fn apply_project_knowledge_enabled(context: &WriteContext, enabled: bool) {
    use crate::workspace_service::event::{emit_for_workspace_scope, WorkspaceEventScope};
    let Some(checkout_id) = context.workspace_checkout.as_ref() else {
        return;
    };
    let Some(registry) = context
        .app
        .try_state::<std::sync::Arc<crate::workspace_service::ProjectRegistry>>()
    else {
        return;
    };
    let Some(runtime) = registry.runtime(checkout_id) else {
        return;
    };
    if !enabled {
        runtime.core().stop_knowledge_watchers();
    }
    emit_for_workspace_scope(
        &context.app,
        &WorkspaceEventScope::for_runtime(&runtime),
        "workspace-setup-changed",
        serde_json::json!({ "knowledgeEnabled": enabled }),
    );
}

/// Effects that run once per batch for settings a module applies as a
/// whole (restarting a listener, re-reading a group). They cannot fail the
/// write; problems surface in the module's own status.
pub(crate) async fn apply_groups(app: &tauri::AppHandle, changes: &[SettingChange]) {
    let moved = changes
        .iter()
        .filter(|change| change.old != change.new)
        .map(|change| change.key)
        .collect::<Vec<_>>();
    let touched = |prefix: &str| moved.iter().any(|key| key.starts_with(prefix));

    if touched("network.proxy.") {
        crate::network::apply_proxy_settings();
    }
    if touched("permissions.mode") || touched("permissions.tools") {
        crate::commands::publish_tool_permissions(app).await;
    }
    if let Some(change) = changes
        .iter()
        .find(|change| change.key == keys::RUNTIMES_GIT_PATH.name() && change.old != change.new)
    {
        crate::commands::apply_git_path_setting(change.new.as_str().unwrap_or_default());
    }
    if touched("providers.claudeCode.") {
        crate::llm::claude_code_cli::ssh::apply_settings_change(&moved);
    }
    if touched("mcpServer.") {
        crate::mcp::server::reconcile(app.clone()).await;
    }
    if touched("remoteAccess.") {
        crate::remote_access::reconcile(app.clone()).await;
    }
}
