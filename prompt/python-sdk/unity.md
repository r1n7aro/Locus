# Unity lifecycle and blocked-editor workflows

Inspect process, connection, readiness, and crash state:

```python
status = await locus.get_unity_editor_status(project=project)
print({
    "process": status.process_state,
    "pid": status.process_id,
    "connected": status.connected,
    "ready": status.ready,
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

`ready` becomes false while a modal dialog blocks the main thread, even when the status channel still responds. `blocking_reason`, `blocking_dialog`, `blocking_dialog_recoverable`, `main_thread`, and `safety` describe observable state and available capabilities. The model chooses whether to resolve a dialog, wait, or restart the editor.

Resolve a native Unity modal dialog while the managed main thread is blocked:

```python
dialog = await locus.get_unity_dialog(project=project)
if dialog:
    choice = dialog.choices[0]
    result = await locus.choose_unity_dialog(
        project=project,
        dialog_id=dialog.dialog_id,
        choice_id=choice.id,
    )
    print(result)
```

For a detached `unity_execute` result, pass its execution id to `await locus.wait_unity_execution(...)`.
