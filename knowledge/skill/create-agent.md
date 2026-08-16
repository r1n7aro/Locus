---
id: kd_skill_create_agent
injectMode: none
summary: Use only when the user explicitly invokes /create-agent to create or edit a persistent Locus Agent. Ignore ordinary requests to delegate work or change the current Agent.
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-agent
argumentHint: <agent-id>
tools:
  - agent_reload
  - read
  - write
  - edit
  - list
---

# Create Agent

## Instructions

Command arguments: `<agent-id>` identifies the Agent to create or edit. Use a short lowercase id containing letters, digits, hyphens, or underscores. Ask for the id and purpose only when they are missing.

1. Call `agent_reload` before editing. Use its `userAgentRoot` exactly; this directory is owned by the user and remains untouched by Locus updates.
2. Inspect `<userAgentRoot>/<agent-id>/` when it exists. Preserve useful files and confirm the requested changes. Reject ids already owned by another Agent unless the user explicitly asked to edit that Agent. Keep `dev`, `explorer`, `doc`, `wiki`, `git`, `knowledge`, and `runtime_debugger` reserved.
3. Create `<userAgentRoot>/<agent-id>/config.json` and `system.md`. Add `env.md`, `rule/`, `rule_config.json`, `injection_config.json`, or `tools/` only when the workflow needs them.
4. Write `config.json` with this schema:

```json
{
  "name": "Display Name",
  "description": "One concise sentence describing the Agent's job.",
  "tools": ["read", "grep", "list"],
  "sub_agents": [],
  "default": false,
  "default_effort": "medium",
  "model_recommendation": "large"
}
```

   - Choose only tools required by the workflow. Use real Locus tool names already visible in the current tool surface.
   - Keep `default` false so Unity remains the default Agent.
   - Use `default_effort` from `none`, `low`, `medium`, `high`, `xhigh`, or `max`.
   - Use `model_recommendation` as `small` or `large`. The model selector remembers the exact model and reasoning effort chosen for each Agent.
5. Write `system.md` as the Agent's focused role, execution rules, validation requirements, and output contract. Keep general Locus behavior out of the file and avoid copying Unity wholesale.
6. To change default injection availability for this Agent, add `injection_config.json`. Keys are the exact ids shown by the Agent page, including `env`, `extra_workdirs`, `knowledge_context`, `lazy_tool_names`, and `knowledge_rule::<type>::<path>`:

```json
{
  "knowledge_rule::memory::测试设计原则.md": { "enabled": false }
}
```

   The Agent page stores later per-workspace choices separately, so these remain portable Agent defaults.
7. To replace Unity-oriented wording in a shared tool, add `<userAgentRoot>/<agent-id>/tools/<tool-name>.json`. Override the top-level tool description and only the parameter descriptions that need different wording:

```json
{
  "description": "List files and directories for this Agent's workflow.",
  "parameters": {
    "properties": {
      "path": { "description": "Directory to inspect." }
    }
  }
}
```

   Use the real tool name as the file name, such as `list.json` or `edit.json`. Locus applies `description` fields only; the original parameter types, required fields, defaults, enums, and validation structure remain intact. Unknown parameter paths are ignored.
8. Read back every created or edited file. Confirm valid JSON, a non-empty name and description, a non-empty `system.md`, and paths contained by `<userAgentRoot>/<agent-id>/`.
9. Call `agent_reload` again. Finish only when the result contains the expected id, display name, and `source: "user"` (or `source: "appUser"` for an explicit overlay). Report the Agent id, physical directory, default effort, tools, default-disabled injections, description overrides, and successful index refresh.
