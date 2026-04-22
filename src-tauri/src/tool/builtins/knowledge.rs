use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolResult};

fn intercepted_tool(name: &str, prompt: &str) -> ToolDef {
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
        execute,
    }
}

pub(super) fn knowledge_list_tool() -> ToolDef {
    intercepted_tool("knowledge_list", crate::prompt::tools::KNOWLEDGE_LIST)
}

pub(super) fn knowledge_query_tool() -> ToolDef {
    intercepted_tool("knowledge_query", crate::prompt::tools::KNOWLEDGE_QUERY)
}

pub(super) fn knowledge_read_tool() -> ToolDef {
    intercepted_tool("knowledge_read", crate::prompt::tools::KNOWLEDGE_READ)
}

pub(super) fn knowledge_create_tool() -> ToolDef {
    intercepted_tool("knowledge_create", crate::prompt::tools::KNOWLEDGE_CREATE)
}

pub(super) fn knowledge_delete_tool() -> ToolDef {
    intercepted_tool("knowledge_delete", crate::prompt::tools::KNOWLEDGE_DELETE)
}

pub(super) fn knowledge_move_tool() -> ToolDef {
    intercepted_tool("knowledge_move", crate::prompt::tools::KNOWLEDGE_MOVE)
}

pub(super) fn knowledge_edit_tool() -> ToolDef {
    intercepted_tool("knowledge_edit", crate::prompt::tools::KNOWLEDGE_EDIT)
}
