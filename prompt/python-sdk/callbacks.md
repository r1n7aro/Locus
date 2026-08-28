# Python callback tools

Expose typed local functions to an in-memory agent with the `@locus.tool` decorator:

```python
@locus.tool(description="Look up one build record")
def get_build(build_id: str, include_logs: bool = False) -> dict:
    return {"id": build_id, "status": "passed", "include_logs": include_logs}

agent = locus.define_agent(
    "build-reader",
    system_prompt="Answer from the build tool result.",
    tools=[get_build],
)
result = await agent.run(
    "What happened to build 42?",
    workspace_ref=workspace_ref,
)
print(result.text)
```

Parameter schemas are derived from Python annotations. Callback tools live only for this Python process. Keep callbacks deterministic and return JSON-serializable values.
