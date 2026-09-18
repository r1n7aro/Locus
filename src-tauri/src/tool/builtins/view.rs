use super::{make_exec, ToolDef, ToolResult};

pub(super) fn execute_typescript() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::EXECUTE_TYPESCRIPT);
    ToolDef {
        name: "execute_typescript".to_string(), description: prompt.description, parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| Box::pin(async move {
            let Some(app) = ctx.app_handle else { return ToolResult { output: "Locus desktop runtime is required".to_string(), is_error: true }; };
            let Some(root) = ctx.working_dir else { return ToolResult { output: "A checkout is required".to_string(), is_error: true }; };
            let code = args.get("code").and_then(|value| value.as_str()).unwrap_or("");
            let label = args.get("windowLabel").and_then(|value| value.as_str());
            let timeout = args.get("timeoutMs").and_then(|value| value.as_u64()).unwrap_or(30_000);
            match crate::view::request_frontend_execution(&app, &root, code, label, timeout).await {
                Ok(value) => ToolResult { output: value.to_string(), is_error: false },
                Err(error) => ToolResult { output: error, is_error: true },
            }
        })),
    }
}
