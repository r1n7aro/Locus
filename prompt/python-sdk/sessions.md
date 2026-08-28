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

Observe or control a run:

```python
async for event in run.event_stream():
    print(event.event_type, event.payload)

status = await run.status()
await run.cancel()
await run.answer("question-id", "approved")
```

Pass `workspace_ref=workspace_ref` when starting or continuing a run to pin it to the injected checkout generation.
