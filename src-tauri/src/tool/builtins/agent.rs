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

            let working_dir = ctx.working_dir.unwrap_or_default();
            let registry = app_handle.state::<crate::AgentDefRegistryState>();
            crate::commands::reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;

            let snapshot = registry.snapshot().await;
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
