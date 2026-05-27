---
title: View Runtime Debugging
tools:
  - view_capture
  - view_snapshot
  - view_action
  - view_wait
  - view_console_read
  - view_debug_eval
---

# View Runtime Debugging

Use these tools after a View has been opened with `view_run`.

## Tools

- `view_capture`: capture the View host as an attached PNG image.
- `view_snapshot`: inspect runtime state, viewport, focus, and visible actionable DOM elements.
- `view_action`: click, type, press keys, set values, scroll, focus, hover, check, uncheck, or drag inside the View host.
- `view_wait`: wait for runtime readiness, selector visibility, text, console health, or a JavaScript expression.
- `view_console_read`: read captured frontend console logs from `.locus/logs/frontend.log`.
- `view_debug_eval`: evaluate focused JavaScript against the View root for debugging.

## Workflow

1. Use `view_run` for the target View.
2. Use `view_wait` with `condition: "runtimeReady"`.
3. Use `view_snapshot` to find element ids, selectors, text, roles, and screen coordinates.
4. Use `view_capture` when visual inspection matters.
5. Use `view_action` for interaction and `view_wait` after async UI changes.
6. Use `view_console_read` and `view_debug_eval` to diagnose frontend runtime failures.

Prefer element ids from `view_snapshot` for `view_action` targets. Use selectors or text only when ids are unavailable or the UI has changed after reload.
