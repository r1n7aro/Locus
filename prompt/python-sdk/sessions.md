# Sessions and runs

Continue an existing session:

```python
session = await locus.get_session("session-id")
run = await session.prompt(
    "Continue from the previous result",
    workspace_ref=workspace_ref,
)
result = await run.wait()
print(result.text)
```

Inspect recent sessions and messages:

```python
for summary in await locus.list_sessions(limit=20):
    print(summary.id, summary.title, summary.agent_id)

session = await locus.get_session("session-id")
for message in session.messages:
    print(message.role, message.content)
```

List only sessions that currently have an active run:

```python
for summary in await locus.list_running_sessions():
    print(summary.id, summary.title, summary.runtime_status)
```

Insert a message into another running session's next model iteration:

```python
target = next(
    session
    for session in await locus.list_running_sessions()
    if not session.is_current
)
delivery = await target.send_message("Please verify the failing test before you finish.")
print(delivery.target_session_id, delivery.target_run_id)
```

Locus derives the source session from the current Python tool invocation. The
received user-role message includes the source session title and ID. The target
must still be running and accepting inserted input. This call changes another
session, so use `readonly=false` in the Python tool.

Observe or control a run:

```python
async for event in run.event_stream():
    print(event.event_type, event.payload)

status = await run.status()
await run.cancel()
await run.answer("question-id", "approved")
```

Pass `workspace_ref=workspace_ref` when starting or continuing a run to pin it to the injected checkout generation.
