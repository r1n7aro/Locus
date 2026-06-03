use serde::Deserialize;

use super::{make_exec, ToolDef, ToolResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginExportToolArgs {
    id: String,
    name: String,
    version: String,
    file_path: String,
    #[serde(default)]
    skill_package_ids: Vec<String>,
    #[serde(default)]
    view_ids: Vec<String>,
    #[serde(default)]
    project_dependencies: Vec<crate::plugin::LocusPluginProjectDependency>,
    audit_summary: String,
    structure_plan: String,
    user_approval: String,
}

fn require_detail(label: &str, value: &str) -> Result<(), ToolResult> {
    if value.trim().chars().count() < 20 {
        return Err(ToolResult {
            output: format!(
                "plugin_export requires a detailed {}. Run the /create-plugin audit and present the structure plan before exporting.",
                label
            ),
            is_error: true,
        });
    }
    Ok(())
}

fn parse_args(args: serde_json::Value) -> Result<PluginExportToolArgs, ToolResult> {
    let parsed =
        serde_json::from_value::<PluginExportToolArgs>(args).map_err(|error| ToolResult {
            output: format!("Error parsing plugin_export arguments: {}", error),
            is_error: true,
        })?;
    if parsed.skill_package_ids.is_empty() && parsed.view_ids.is_empty() {
        return Err(ToolResult {
            output: "plugin_export requires at least one Skill package id or View id.".to_string(),
            is_error: true,
        });
    }
    require_detail("auditSummary", &parsed.audit_summary)?;
    require_detail("structurePlan", &parsed.structure_plan)?;
    require_detail("userApproval", &parsed.user_approval)?;
    Ok(parsed)
}

pub(super) fn plugin_export() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_EXPORT);
    ToolDef {
        name: "plugin_export".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match parse_args(args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let request = crate::commands::PluginExportRequest {
                    id: parsed.id,
                    name: parsed.name,
                    version: parsed.version,
                    file_path: parsed.file_path,
                    skill_package_ids: parsed.skill_package_ids,
                    view_ids: parsed.view_ids,
                    project_dependencies: parsed.project_dependencies,
                };
                let working_dir = ctx.working_dir.unwrap_or_default();
                match crate::commands::export_plugin_archive_sync(&working_dir, request) {
                    Ok(result) => match serde_json::to_string_pretty(&result) {
                        Ok(output) => ToolResult {
                            output,
                            is_error: false,
                        },
                        Err(error) => ToolResult {
                            output: format!("Failed to serialize plugin_export result: {}", error),
                            is_error: true,
                        },
                    },
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolExecutionContext, ToolRegistry};

    #[test]
    fn plugin_export_tool_is_skill_loaded() {
        let registry = ToolRegistry::with_builtins();
        assert_eq!(
            registry.default_load_mode("plugin_export"),
            crate::tool::ToolLoadMode::Skill
        );
    }

    #[tokio::test]
    async fn plugin_export_tool_requires_audit_structure_and_approval() {
        let tool = plugin_export();
        let result = (tool.execute)(
            serde_json::json!({
                "id": "com.example.demo",
                "name": "Demo",
                "version": "0.1.0",
                "filePath": "demo.zip",
                "skillPackageIds": [],
                "viewIds": ["demo-view"],
                "projectDependencies": [],
                "auditSummary": "short",
                "structurePlan": "short",
                "userApproval": "short"
            }),
            ToolExecutionContext::default(),
        )
        .await;
        assert!(result.is_error);
        assert!(result.output.contains("auditSummary"));
    }
}
