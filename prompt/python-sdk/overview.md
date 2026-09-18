# Locus Python SDK

The `python` tool already injects `locus`, the checkout path as `project`, and the checkout-pinned `workspace_ref`. Its code is an async body, so call SDK coroutines with top-level `await`.

Run Python by passing `code` and `readonly` to the `python` tool. `readonly=true` requires the entire script and every nested SDK/tool call to observe state only; file writes, rule changes and Editor mutations require `readonly=false`. Each call starts a fresh Python process in the current checkout. The optional `timeout` is in milliseconds (default 120000, maximum 1800000).

Read any topic file directly from this directory as needed:

- [agents.md](agents.md): discover and run Agents; read, save and enable workspace rules
- [sessions.md](sessions.md): discover sessions, search/read history, send messages and continue runs
- [tools.md](tools.md): discover and call built-in, MCP and Skill tools
- [tasks.md](tasks.md): background execution, task status, waiting, cancellation and subagent messages
- [unity.md](unity.md): Editor lifecycle, readiness, crashes, dialogs and detached execution
- [worktrees.md](worktrees.md): sibling checkouts, private Unity project pool slots and parallel Editors
- [assets.md](assets.md): snapshot-bound asset inspection and bulk editing across YAML and the live Editor
- [csv.md](csv.md): edit CSV tables with the same worksheet and style objects as openpyxl, the standard Python Excel-editing library; use `await locus.csv.load_workbook(path)`, normal openpyxl edits, then `await wb.save()`
- [merges.md](merges.md): selective integration, preview/apply, validation and commits
- [callbacks.md](callbacks.md): expose local Python functions as typed Agent tools

Prefer the injected `workspace_ref` for Agent runs so a workflow cannot silently move to another checkout generation.

Pass `worktree=handle` to tool discovery/calls, Unity lifecycle APIs (including `close_unity_editor`), or merge preparation; the Agent working directory stays fixed. Omitted tool/workspace selectors use the injected checkout identity.
