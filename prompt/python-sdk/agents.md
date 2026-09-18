# Agent workflows

List and select an installed agent:

```python
agents = await locus.list_agents()
agent = await locus.get_agent("unity")
run = await agent.prompt(
    "Inspect the current compile errors",
    workspace_ref=workspace_ref,
)
result = await run.wait()
print(result.text)
```

Run a one-shot prompt:

```python
result = await agent.run(
    "Run EditMode tests and explain failures",
    workspace_ref=workspace_ref,
    model="mock/tool",
)
print(result.text)
```

## Workspace rules for installed Agents

Use the installed Agent object to manage rules without opening the Agent page:

```python
agent = await locus.get_agent("unity")
rules = await agent.list_rules()
print([(rule.key, rule.source, rule.enabled) for rule in rules])

rule = await agent.save_rule(
    "project-conventions.md",
    "# Project conventions\nUse the project's existing naming and formatting conventions.",
)
print(await agent.read_rule(rule.key))
await agent.set_rule_enabled(rule.key, False)
await agent.set_rule_enabled(rule.key, True)
```

- All four methods default to the checkout identity injected into the Python process. Pass `workspace_ref=workspace_ref` or `worktree=handle` to select a specific registered checkout. Missing or stale checkout references fail; the active UI workspace is never used as a fallback.
- `list_rules()` includes disabled rules and returns `AgentRule` objects with `key`, `file_name`, `title`, `enabled`, `order`, `source`, `read_only`, `updated_at`, `plugin_id`, and `plugin_scope`. Use the exact `key` for reading or toggling a rule, including an inherited app rule.
- `save_rule(file_name, content)` creates or replaces a Markdown rule in `<workspace>/Locus/agent/<agent-id>/rule/`. Omitted `.md` is appended. A new rule is enabled; updating an existing rule preserves its effective enabled state and order. Saving an inherited rule creates a workspace override.
- `set_rule_enabled(key, enabled)` stores the workspace choice in `rule_config.json`; it never edits installed app/user defaults or another workspace. Re-enable with `True`. Unknown keys fail. Plugin rule enablement remains controlled by plugin state.
- Changes refresh the existing Agent page and apply on the next model request, including the current run's next tool round. Already-sent requests and conversation messages stay as history.
- Use `readonly=False` when the Python tool saves or toggles rules. Listing and reading can use `readonly=True`. These methods manage installed Agents, including the built-in `unity` Agent; Python-only inline Agents are rejected.

The same operations are available on `Client`: `list_agent_rules(agent_id, ...)`, `read_agent_rule(agent_id, key, ...)`, `save_agent_rule(agent_id, file_name, content, ...)`, and `set_agent_rule_enabled(agent_id, key, enabled, ...)`.

## In-memory Agents

Define an in-memory agent for the current Python process:

```python
reviewer = locus.define_agent(
    "local-reviewer",
    name="Local reviewer",
    system_prompt="Review Unity changes and cite concrete evidence.",
    tools=["read", "grep", "unity_get_console_log"],
)
result = await reviewer.run("Review the current checkout", workspace_ref=workspace_ref)
print(result.text)
```

An in-memory definition is sent with each prompt and does not modify the repository's agent files.
