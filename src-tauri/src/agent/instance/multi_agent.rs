use super::*;

#[cfg(test)]
#[path = "multi_agent_tests.rs"]
mod tests;

/// Collaboration tool offered with `subagent` while Multi-Agent is enabled.
/// Unlike `subagent` it stays available to depth-capped children.
pub(super) const SEND_MESSAGE_TOOL: &str = "send_message";

/// Tools that exist only while the session has Multi-Agent enabled.
pub(super) fn requires_multi_agent(name: &str) -> bool {
    matches!(name, "subagent" | SEND_MESSAGE_TOOL)
}

pub(super) const PROACTIVE_DELEGATION_GUIDANCE: &str = "If at any point you can parallelize work by delegating tasks to another agent (no matter if you are root or subagent), you should do so using collaboration tools if it could save time or improve quality.";
const EXPLICIT_DELEGATION_GUIDANCE: &str = "Multi-Agent is disabled. Any earlier instruction encouraging proactive delegation no longer applies. Do not spawn or invoke subagents unless the user explicitly requests subagents, delegation, or parallel agent work. Requests for depth, thoroughness, research, or detailed analysis alone do not authorize delegation. This restriction applies to every route, including the subagent tool, Python SDK agent.prompt(), agent.run(), locus.prompt_agent(), session-start APIs, and task resumes or messages that start another agent run. Do not work around it using Python, shell commands, or other tools.";

impl AgentInstance {
    pub fn set_multi_agent_enabled(&mut self, enabled: bool) {
        self.multi_agent_enabled = enabled;
    }

    pub(super) fn explicit_delegation_guidance(&self) -> Option<String> {
        if !self.multi_agent_enabled {
            return Some(EXPLICIT_DELEGATION_GUIDANCE.to_string());
        }
        // The native Agent tool carries Claude's own description; the Locus
        // routing and proactive-delegation policy live here instead.
        self.claude_delegation().map(|_| {
            format!(
                "{} {PROACTIVE_DELEGATION_GUIDANCE}",
                super::claude_native_agents::CLAUDE_UNIFIED_GUIDANCE
            )
        })
    }

    /// Delegation policy for all routes lives in the system prompt. Only the
    /// dedicated subagent tool needs additional proactive usage guidance.
    pub(super) fn apply_multi_agent_guidance(&self, name: &str, tool: &mut serde_json::Value) {
        let guidance = match name {
            "subagent" if self.multi_agent_enabled => PROACTIVE_DELEGATION_GUIDANCE,
            _ => return,
        };
        if let Some(description) = tool["function"]["description"].as_str() {
            if !description.contains(guidance) {
                tool["function"]["description"] =
                    serde_json::Value::String(format!("{description}\n\n{guidance}"));
            }
        }
    }

    /// Queues a message for another agent in this tree. Delivery happens
    /// between the recipient's tool calls through the async-task outbox.
    pub(super) fn execute_send_message(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> ExecutedToolResult {
        let result = if !self.multi_agent_enabled {
            Err(
                "The send_message tool is disabled for this session because Multi-Agent is off."
                    .to_string(),
            )
        } else {
            self.send_agent_message(app_handle, args)
        };
        ExecutedToolResult::from_tool_result(match result {
            Ok(output) => ToolResult {
                output,
                is_error: false,
            },
            Err(output) => ToolResult {
                output: format!("Error: {output}"),
                is_error: true,
            },
        })
    }

    fn send_agent_message(
        &self,
        app_handle: &AppHandle,
        args: &serde_json::Value,
    ) -> Result<String, String> {
        let to = args
            .get("to")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|to| !to.is_empty())
            .ok_or("send_message requires a non-empty 'to' agent id")?;
        let message = args
            .get("message")
            .and_then(serde_json::Value::as_str)
            .ok_or("send_message requires a 'message' string")?;
        let manager = app_handle
            .try_state::<Arc<crate::async_tasks::AsyncTaskManager>>()
            .ok_or("Agent messaging is unavailable in this runtime")?
            .inner()
            .clone();
        let receipt = manager
            .send_agent_message(app_handle, &self.session_id, to, message)
            .map_err(|error| {
                let reachable = manager.reachable_agent_ids(&self.session_id);
                if reachable.is_empty() {
                    format!("{error} No other agent is reachable from this session yet.")
                } else {
                    format!("{error} Reachable agent ids: {}.", reachable.join(", "))
                }
            })?;
        eprintln!(
            "[Agent {}] queued agent message session={} to={} message_id={}",
            self.id,
            self.session_id,
            to,
            receipt["messageId"].as_str().unwrap_or_default()
        );
        Ok(serde_json::json!({
            "status": "queued",
            "to": to,
            "messageId": receipt["messageId"],
            "delivery": "The recipient reads it before its next model request; replies arrive as agent messages.",
        })
        .to_string())
    }

    pub(super) fn multi_agent_disabled_result(&self) -> ExecutedToolResult {
        ExecutedToolResult::from_tool_result(ToolResult {
            output: "The subagent tool is disabled for this session. Complete the task with your other tools.".to_string(),
            is_error: true,
        })
    }
}
