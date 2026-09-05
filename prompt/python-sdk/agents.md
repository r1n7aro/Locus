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
