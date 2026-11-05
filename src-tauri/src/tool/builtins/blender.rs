//! `blender_execute` registry entry. The agent loop and the MCP server call
//! `crate::blender::tool` directly so results can carry images; this entry
//! serves the other callers (text only, images listed as omitted).

use std::sync::Arc;

use super::{ToolDef, ToolResult};

pub(super) fn blender_execute() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::BLENDER_EXECUTE);
    ToolDef {
        name: "blender_execute".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Code may export files into the workspace.
        mutates_workspace: true,
        execute: Arc::new(|args, ctx| {
            Box::pin(async move {
                let Some(execution) = ctx.execution.clone() else {
                    return ToolResult {
                        output: "Tool 'blender_execute' requires a checkout-scoped workspace.".to_string(),
                        is_error: true,
                    };
                };
                let session_id = ctx.process_owner.as_ref().and_then(|owner| owner.session_id.clone());
                let output = ctx.output.clone();
                let run = crate::blender::tool::run_in_execution(
                    &execution,
                    session_id.as_deref(),
                    &args,
                    move |delta| {
                        if let Some(output) = output.as_ref() {
                            output(delta);
                        }
                    },
                    ctx.cancel_rx.clone(),
                )
                .await;
                let mut text = run.output;
                if !run.images.is_empty() {
                    text.push_str(&format!(
                        "\n({} image(s) not shown: this caller cannot return images)",
                        run.images.len()
                    ));
                }
                ToolResult { output: text, is_error: run.is_error }
            })
        }),
    }
}
