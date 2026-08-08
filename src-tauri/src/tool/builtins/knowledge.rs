use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolResult};

fn intercepted_tool(name: &str, prompt: &str, mutates_workspace: bool) -> ToolDef {
    let execute: ToolExecuteFn = Arc::new({
        let name = name.to_string();
        move |_args, _ctx| {
            let name = name.clone();
            Box::pin(async move {
                ToolResult {
                    output: format!("Error: {} tool should be intercepted by agent loop", name),
                    is_error: true,
                }
            })
        }
    });

    let prompt = crate::prompt::parse_tool_prompt(prompt);
    ToolDef {
        name: name.to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace,
        execute,
    }
}

pub(super) fn knowledge_query_tool() -> ToolDef {
    intercepted_tool(
        "knowledge_query",
        crate::prompt::tools::KNOWLEDGE_QUERY,
        false,
    )
}
