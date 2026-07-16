use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolResult};

/// Agent-facing reload: reconciles live MCP connections against the config
/// file and returns a per-server report (status, error + stderr on failure,
/// wire tool names on success). Unlike the intercepted skill tools this one
/// executes directly — the manager is global state, no agent context needed.
pub(super) fn mcp_reload_tool() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            let reports = crate::mcp::manager::reconcile().await;
            ToolResult {
                output: crate::mcp::manager::format_reports(&reports),
                is_error: false,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::MCP_RELOAD);
    ToolDef {
        name: "mcp_reload".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
}
