# Locus Python SDK topics

The `python` tool already injects `locus`, the checkout path as `project`, and the checkout-pinned `workspace_ref`. Its code is an async body, so call SDK coroutines with top-level `await`.

Load one topic when needed:

- `agents`: list, select, define, prompt, and run agents with checkout pinning
- `sessions`: continue sessions, inspect messages and runs, stream events, cancel, and answer questions
- `tools`: discover and invoke Locus built-in, MCP, and Skill tools
- `tasks`: built-in `list_tasks()`, `get_task_status(id)`, `wait_task(id)`, `cancel_task(id)`, `resume_task(id)`, and `send_message(id, text)`; short task ids/names are scoped to the current session
- `unity`: editor lifecycle, crash/readiness signals, modal dialogs, and detached execution
- `callbacks`: expose local Python functions as typed tools for a temporary agent

Prefer the injected `workspace_ref` for Agent runs so a workflow cannot silently move to another checkout generation.
