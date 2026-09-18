# Tool discovery and calls

Discover tools from the same registry used by Locus agents:

```python
tools = await locus.list_tools()
for item in tools:
    print(item.name, item.source, item.mutates_workspace)

definition = await locus.get_tool("unity_get_console_log")
print(definition.input_schema)
```

Call a tool:

```python
result = await locus.call_tool(
    "unity_get_console_log",
    {"limit": 100},
    timeout=30,
    workspace_ref=workspace_ref,
)
if result.is_error:
    raise RuntimeError(result.output)
print(result.output)
```

Pass the injected `workspace_ref` to checkout-scoped tools. Built-in, MCP, and Skill tools share the same result model. Tool calls that require human interaction or agent-loop-only context are reported as unavailable.

Inside the Python tool, omitting the selector uses its injected checkout identity. For another checkout, pass `worktree=wt` to discovery and calls, where `wt` is returned by `locus.worktrees.create/get/acquire` (`acquire` returns `.worktree`). Independent worktree calls can use `asyncio.gather`; they do not change the Agent's working directory. `workspace_ref=job.workspace_ref` also accepts the dictionary returned by merge jobs. Read [worktrees.md](worktrees.md) for the complete dual-Editor workflow.
