use tauri::AppHandle;

use crate::commands::StreamEvent;
use crate::tool::ToolResult;

use super::{emit_stream, emit_tool_progress, AgentInstance, ExecutedToolResult};

impl AgentInstance {
    /// `blender_execute` in the agent loop: streams Blender's output into
    /// the tool block and returns images with the result.
    pub(super) async fn execute_blender_execute(
        &self,
        app_handle: &AppHandle,
        tool_call_id: &str,
        args: &serde_json::Value,
        run_id: &str,
    ) -> ExecutedToolResult {
        if self.is_cancel_requested() {
            return Self::interrupted_tool_result();
        }
        let Some(execution) = self.execution_context.clone() else {
            return ExecutedToolResult::from_tool_result(ToolResult {
                output: "Tool 'blender_execute' requires a checkout-scoped workspace.".to_string(),
                is_error: true,
            });
        };
        emit_tool_progress(
            app_handle,
            run_id,
            &self.session_id,
            tool_call_id,
            "Running in Blender",
            "",
            None,
            "running",
        );
        let output_handle = app_handle.clone();
        let output_run_id = run_id.to_string();
        let output_session_id = self.session_id.clone();
        let output_tool_call_id = tool_call_id.to_string();
        let run = crate::blender::tool::run_in_execution(
            &execution,
            Some(&self.session_id),
            args,
            move |delta| {
                emit_stream(
                    &output_handle,
                    &output_run_id,
                    StreamEvent::ToolCallDelta {
                        session_id: output_session_id.clone(),
                        tool_call_id: output_tool_call_id.clone(),
                        delta,
                    },
                );
            },
            Some(self.cancel_waiter()),
        )
        .await;
        if self.is_cancel_requested() && run.is_error {
            let mut result = Self::interrupted_tool_result();
            result.output = format!("{}\n{}", crate::session::history::INTERRUPTED_TOOL_RESULT, run.output);
            return result;
        }
        ExecutedToolResult::from_tool_result(ToolResult {
            output: run.output,
            is_error: run.is_error,
        })
        .with_images(run.images)
    }
}
