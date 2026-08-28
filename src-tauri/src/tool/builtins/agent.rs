use serde::Serialize;
use tauri::Manager;

use super::{make_exec, ToolDef, ToolResult};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentReloadItem {
    id: String,
    name: String,
    description: String,
    is_default: bool,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentReloadOutput {
    user_agent_root: String,
    default_agent_id: String,
    count: usize,
    agents: Vec<AgentReloadItem>,
}

pub(super) fn agent_reload() -> ToolDef {
    let execute = make_exec(|_args, ctx| {
        Box::pin(async move {
            let Some(execution) = ctx.execution else {
                return ToolResult {
                    output: "Tool 'agent_reload' requires a checkout-scoped ToolExecutionContext."
                        .to_string(),
                    is_error: true,
                };
            };
            let Some(app_handle) = ctx.app_handle else {
                return ToolResult {
                    output: "agent_reload requires an application context".to_string(),
                    is_error: true,
                };
            };
            let app_agent_dir = app_handle.state::<crate::AppAgentDir>();
            let Some(bundled_root) = app_agent_dir.0.as_ref() else {
                return ToolResult {
                    output: "Locus could not resolve its installed Agent directory".to_string(),
                    is_error: true,
                };
            };
            let user_agent_root = crate::agent::definition::user_agent_dir(bundled_root);
            if let Err(error) = std::fs::create_dir_all(&user_agent_root) {
                return ToolResult {
                    output: format!(
                        "Failed to create writable user Agent directory '{}': {}",
                        user_agent_root.display(),
                        error
                    ),
                    is_error: true,
                };
            }
            let definitions =
                app_handle.state::<std::sync::Arc<
                    crate::workspace_definition_registry::WorkspaceDefinitionRegistry,
                >>();
            if let Err(error) = definitions.invalidate_app_base() {
                return ToolResult {
                    output: format!("Failed to invalidate Agent definitions: {error}"),
                    is_error: true,
                };
            }
            if let Err(error) = definitions.invalidate_checkout(&execution.checkout_id) {
                return ToolResult {
                    output: format!("Failed to invalidate checkout Agent definitions: {error}"),
                    is_error: true,
                };
            }
            let process_registry = app_handle.state::<crate::AgentDefRegistryState>();
            crate::commands::reload_agent_registry(&process_registry, &app_agent_dir, "").await;
            let snapshot = match definitions.snapshot(execution.workspace.as_ref()).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return ToolResult {
                        output: format!("Failed to reload checkout Agent definitions: {error}"),
                        is_error: true,
                    };
                }
            };
            let default_agent_id = snapshot.default_id().to_string();
            let mut agents = snapshot
                .list_all()
                .into_iter()
                .filter(|def| !crate::agent::definition::is_hidden_legacy_agent_id(&def.id))
                .map(|def| AgentReloadItem {
                    id: def.id.clone(),
                    name: def.name.clone(),
                    description: def.description.clone(),
                    is_default: def.id == default_agent_id,
                    source: def.source.clone(),
                })
                .collect::<Vec<_>>();
            agents.sort_by(|a, b| {
                b.is_default
                    .cmp(&a.is_default)
                    .then(a.name.cmp(&b.name))
                    .then(a.id.cmp(&b.id))
            });
            crate::commands::emit_agents_changed(&app_handle);
            crate::commands::emit_agents_changed_for_workspace(
                &app_handle,
                execution.workspace.as_ref(),
            );

            let output = AgentReloadOutput {
                user_agent_root: user_agent_root.to_string_lossy().replace('\\', "/"),
                default_agent_id,
                count: agents.len(),
                agents,
            };
            match serde_json::to_string_pretty(&output) {
                Ok(output) => ToolResult {
                    output,
                    is_error: false,
                },
                Err(error) => ToolResult {
                    output: format!("Failed to serialize refreshed Agent index: {}", error),
                    is_error: true,
                },
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::AGENT_RELOAD);
    ToolDef {
        name: "agent_reload".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
}

#[cfg(test)]
mod tests {
    use super::agent_reload;

    #[tokio::test]
    async fn agent_reload_requires_checkout_execution_scope() {
        let tool = agent_reload();
        let result = (tool.execute)(
            serde_json::json!({}),
            crate::tool::ToolExecutionContext::default(),
        )
        .await;

        assert!(result.is_error);
        assert!(result
            .output
            .contains("checkout-scoped ToolExecutionContext"));
    }
}
