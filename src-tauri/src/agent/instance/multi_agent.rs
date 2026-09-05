use super::*;

#[cfg(test)]
#[path = "multi_agent_tests.rs"]
mod tests;

const PROACTIVE_DELEGATION_GUIDANCE: &str = "If at any point you can parallelize work by delegating tasks to another agent (no matter if you are root or subagent), you should do so using collaboration tools if it could save time or improve quality.";
const EXPLICIT_DELEGATION_GUIDANCE: &str = "Multi-Agent is disabled. Any earlier instruction encouraging proactive delegation no longer applies. Do not spawn or invoke subagents unless the user explicitly requests subagents, delegation, or parallel agent work. Requests for depth, thoroughness, research, or detailed analysis alone do not authorize delegation. This restriction applies to every route, including the subagent tool, Python SDK agent.prompt(), agent.run(), locus.prompt_agent(), session-start APIs, and task resumes or messages that start another agent run. Do not work around it using Python, shell commands, or other tools.";

impl AgentInstance {
    pub fn set_multi_agent_enabled(&mut self, enabled: bool) {
        self.multi_agent_enabled = enabled;
    }

    pub(super) fn explicit_delegation_guidance(&self) -> Option<&'static str> {
        (!self.multi_agent_enabled).then_some(EXPLICIT_DELEGATION_GUIDANCE)
    }

    /// Append the session policy after agent-specific description overrides.
    pub(super) fn apply_multi_agent_guidance(&self, name: &str, tool: &mut serde_json::Value) {
        let guidance = match name {
            "subagent" if self.multi_agent_enabled => PROACTIVE_DELEGATION_GUIDANCE,
            "python" if !self.multi_agent_enabled => EXPLICIT_DELEGATION_GUIDANCE,
            _ => return,
        };
        if let Some(description) = tool["function"]["description"].as_str() {
            if !description.contains(guidance) {
                tool["function"]["description"] =
                    serde_json::Value::String(format!("{description}\n\n{guidance}"));
            }
        }
    }

    pub(super) fn multi_agent_disabled_result(&self) -> ExecutedToolResult {
        ExecutedToolResult::from_tool_result(ToolResult {
            output: "The subagent tool is disabled for this session. Complete the task with your other tools.".to_string(),
            is_error: true,
        })
    }
}
