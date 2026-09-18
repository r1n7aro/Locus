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

Write all model-facing prompts in English, including the soul, rules, environment instructions, and tool-description overrides. Preserve exact identifiers, paths, code, and user-provided names. Use the language of the user's latest prompt for questions, progress updates, and the final report unless the user explicitly requests another language. Apply the same communication policy to the Agent being created; English prompt files do not imply English user-facing replies.

1. Choose the storage scope before writing, then call `agent_reload` to resolve the physical roots and inspect existing Agents.
   - Default new Agents to the current Project. Requests mentioning project-level Agents, team collaboration, or version control use `projectAgentRoot`, which is `<current-checkout>/Locus/agent/`. This is the checkout bound to the current session, including a worktree, not another project selected in the UI.
   - Use `userAgentRoot` only when the user explicitly requests an installation-level Agent shared across projects. It is the user-owned `user-agents/` directory beside the installed resources; it is outside the project repository.
   - Set `<agentRoot>` to the returned root for the chosen scope and use absolute paths under it for every file operation. Never substitute the process working directory or the Locus installation directory for the project root. If `projectAgentRoot` cannot be resolved, ask for the target project; do not fall back to `userAgentRoot`.
   - When editing an existing Agent, honor the requested scope. If neither the request nor the existing files identify it unambiguously, ask which scope to edit before writing.
2. Inspect `<agentRoot>/<agent-id>/` when it exists. Preserve useful files and apply the requested changes. Reject ids already owned by another Agent unless the user explicitly asked to edit or create a project copy of that Agent. Keep `unity`, `simple`, `dev`, `explorer`, `doc`, `wiki`, `git`, `knowledge`, and `runtime_debugger` reserved for new Agents.
3. Create `<agentRoot>/<agent-id>/config.json`, `soul.md`, and the focused Markdown rules the workflow needs under `rule/`, with their defaults in `rule_config.json`. Add `env.md`, `injection_config.json`, or `tools/` only when needed.
   - A project Agent must be usable from the repository on a teammate's machine: keep its definition, prompts, rules, and tool-description overrides together under `<projectAgentRoot>/<agent-id>/`, without depending on a same-id Agent in the local installation or machine-specific absolute paths in its content. When explicitly copying an existing Agent into the project, include all required files; leave the original files intact.
   - Use the current Unity and Simple structure as the reference: Unity has a short role prompt in `system.md` with detailed injected rules; Simple has a compact `soul.md` and essential tools. Match that restraint and include only behavior relevant to the new Agent, without copying Unity-specific constraints into unrelated workflows.
   - `soul.md` takes precedence over the legacy `system.md`. New Agents use `soul.md`. For an existing Agent, edit its effective prompt, or deliberately move its role into `soul.md` while preserving required behavior in rules; do not maintain competing prompt copies.
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
   - Keep `default` false to preserve the current default Agent.
   - Use `default_effort` from `none`, `low`, `medium`, `high`, `xhigh`, or `max`.
   - Use `model_recommendation` as `small` or `large`. The model selector remembers the exact model and reasoning effort chosen for each Agent.
5. Keep the soul concise and put most behavioral instructions in injected rules.
   - The soul defines the Agent's identity, purpose, scope, and core working stance in one short paragraph or a few sentences. Keep tool procedures, checklists, validation steps, detailed output requirements, and environment data out of it.
   - Put execution and collaboration requirements, domain constraints, validation criteria, and communication requirements in focused `rule/*.md` files. Use one coherent topic per rule with a short English heading. Preserve useful constraints when extracting an existing long prompt, remove duplication, and avoid restating the same instructions in both the soul and rules.
   - Create only rules that change how this Agent works. Keep them concise; do not split every sentence into a file or add empty placeholders. Use conditional wording for requirements that apply only to particular tasks.
   - Include an English communication rule, such as `rule/output_principles.md`, that tells the Agent to match the language of the user's latest prompt unless another language is explicitly requested. Apply it to questions, progress updates, and final reports. Lead with the outcome, summarize relevant verification and material limitations, keep detail proportional to the task, and use no emoji.
   - Store rules as Markdown files directly inside `rule/`. Locus injects enabled rules automatically; Markdown links in the soul are not a substitute for rule injection. Add entries to `rule_config.json` keyed by the exact file name, with `enabled` and `order`:

```json
{
  "execution.md": { "enabled": true, "order": 0 },
  "output_principles.md": { "enabled": true, "order": 1 }
}
```

   Use only the file names actually created. Give new rules a deliberate order and enable required behavior, including the communication rule. When editing, preserve existing enabled states and ordering unless the requested changes require otherwise. Rules that should remain independently configurable belong here, not in the soul.
6. To change other injection defaults for this Agent, add `injection_config.json`; use `rule_config.json` for the Agent's own `rule/*.md` files. Injection keys are the exact ids shown by the Agent page, including `env`, `extra_workdirs`, `knowledge_context`, `lazy_tool_names`, and `knowledge_rule::<type>::<path>`:

```json
{
  "knowledge_rule::memory::test-design-principles.md": { "enabled": false }
}
```

   The Agent page stores later per-workspace choices separately, so these remain portable Agent defaults.
7. To replace Unity-oriented wording in a shared tool, add `<agentRoot>/<agent-id>/tools/<tool-name>.json`. Override the top-level tool description and only the parameter descriptions that need different wording:

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
8. Read back every created or edited file. Confirm valid JSON, a non-empty name and description, a concise non-empty effective soul, and paths contained by `<agentRoot>/<agent-id>/`. Check that detailed behavior lives in rules, rule config entries match real files, required rules are enabled, model-facing instructions are in English, and the communication rule follows the user's prompt language. For a project Agent, verify all required files are in the project directory; an installation copy alone does not satisfy project scope.
9. Call `agent_reload` again. Finish only when the result contains the expected id and display name, the selected root is unchanged, and the source matches the scope:
   - Project: `source: "project"`, or `source: "both"` when an explicitly requested project copy or overlay also has an installed definition. `source: "user"` and `source: "appUser"` do not validate a project Agent.
   - Installation: `source: "user"`, or `source: "appUser"` for an explicit overlay. A same-id project overlay reports `source: "both"`; inspect it before claiming the installation changes are effective in this checkout.
   Report concisely in the language of the user's latest prompt unless another language was requested. Include the Agent id, scope, physical directory, soul and rule organization, default effort, tools, any disabled rules or injections, description overrides, and successful index refresh. For project scope, identify `Locus/agent/<agent-id>/` as the directory to include in version control for team use.
