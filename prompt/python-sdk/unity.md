# Unity lifecycle and blocked-editor workflows

Managed worktrees acquire their Editor automatically when a Unity command needs it. Locus reuses the same headless process across turns and releases it after the idle TTL. Existing interactive Editors take priority and stay user-managed; `ensure_unity_editor(mode="headless")` also reuses them and reports their actual mode. Routine work does not need per-turn ensure/restart/close calls. Use the explicit lifecycle APIs only for requested recovery or shutdown.

Inspect process, connection, readiness, and crash state:

```python
status = await locus.get_unity_editor_status(project=project)
print({
    "process": status.process_state,
    "pid": status.process_id,
    "connected": status.connected,
    "ready": status.ready,
    "safe_mode": status.safe_mode,
    "editor_log": status.editor_log_path,
    "main_thread_blocked": status.main_thread_blocked,
    "dialog_recoverable": status.blocking_dialog_recoverable,
    "dialog": status.blocking_dialog,
    "crashed": status.is_crashed,
    "phase": status.semantic_phase,
})
```

Ensure a usable editor, or restart it explicitly:

```python
ready = await locus.ensure_unity_editor(
    project=project,
    mode="interactive",  # interactive | headless
    wait_until="ready",  # process | connected | ready
    timeout=300,
)

restarted = await locus.restart_unity_editor(
    project=project,
    mode="headless",
    wait_until="ready",
    timeout=300,
    force=False,  # request a normal close before force-closing on timeout
)
print(restarted.closed_process_ids, restarted.forced_process_ids)
```

`ensure_unity_editor` and `restart_unity_editor` stop waiting when a native dialog requires a choice, including startup prompts such as **Recovering Scene Backups** before the managed bridge connects. They raise `LocusRpcError` with `code=unity_modal_dialog_blocked`, the title, message, `dialog_id`, and all labeled `choice_id` values, just like a blocked `unity_recompile`. Choose by the dialog's meaning with `choose_unity_dialog`; no option is selected automatically.

For `request_state=editor_starting`, the editor has already launched. After choosing, call `ensure_unity_editor` with the same `project`, `mode`, and `wait_until` to continue waiting for that process; do not restart again. Further dialogs are reported the same way. For `request_state=editor_closing`, normal shutdown paused before force-close and no replacement has launched; inspect status after choosing, then continue the original operation if still needed. `force=True` explicitly bypasses normal close. `wait_until="process"` only waits for process creation.

`ready` becomes false while a modal dialog blocks the main thread, even when the status channel still responds. `blocking_reason`, `blocking_dialog`, `blocking_dialog_recoverable`, `main_thread`, and `safety` describe observable state and available capabilities. The model chooses whether to resolve a dialog, wait, or restart the editor.

`safe_mode` is detected outside the Unity managed domain. In Safe Mode, call `unity_get_console_log` with `level="error"`, repair the referenced source files with file tools, and poll status. If Unity does not consume the external file change promptly, call `restart_unity_editor(...)` and wait for `ready`. After an abnormal exit, inspect `editor_log_path` before restarting Unity.

Resolve a native Unity modal dialog while the managed main thread is blocked:

```python
dialog = await locus.get_unity_dialog(project=project)
if dialog:
    print(dialog.title, dialog.message)
    print([(choice.id, choice.label) for choice in dialog.choices])
```

After selecting a returned `choice_id` based on the dialog's meaning:

```python
if dialog:
    result = await locus.choose_unity_dialog(
        project=project,
        dialog_id=dialog.dialog_id,
        choice_id=choice_id,
    )
    print(result)
```

`choose_unity_dialog(...)` returns after the selected dialog has closed or been replaced, so the next Unity call cannot race with the original modal window. If the user already handled the dialog, the call returns `invoked=False` with `status="dialog_not_found"`; this is a normal result and does not raise an RPC error.

For a detached `unity_execute` result, pass its execution id to `await locus.wait_unity_execution(...)`.

All lifecycle, dialog and detached-execution-wait calls also accept `worktree=handle` or `workspace_ref=reference` in place of `project=`. `close_unity_editor(worktree=handle, timeout=60, force=False)` closes that checkout without reopening it and reports closed/forced PIDs plus final status. Worktree handles preserve the materialization epoch and are rejected after pool reuse. Read [worktrees.md](worktrees.md) for parallel Editor analysis and pool release.
