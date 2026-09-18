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

List the current checkout's sessions. `archived=False` selects unarchived
sessions; `archived=True` selects archived sessions:

```python
for summary in await locus.list_sessions(archived=False, limit=20):
    print(summary.id, summary.title, summary.agent_id)

archived = await locus.list_sessions(archived=True, limit=20)
```

`list_sessions`, `list_running_sessions`, `search_sessions`, and `read_session`
accept `workspace_ref=workspace_ref` or `worktree=handle`. Omitted selectors use
the checkout identity injected into the Python process, even if the UI changes
workspaces. Without any checkout identity, listing retains the legacy unbound
session list; search and paged reading require an explicit selector.

Search titles and persisted message text, thinking, tool arguments and results:

```python
hits = await locus.search_sessions("shader", archived=False, limit=10)
for hit in hits.matches:
    print(hit.session_id, hit.session_title, hit.message_id, hit.field, hit.excerpt)

# Restrict the search to one session; use archived=True for an archived session.
hits = await locus.search_sessions("compile", session_id="session-id", archived=True)

# Continue the same search when more matches are needed.
if hits.has_more:
    hits = await locus.search_sessions(
        "compile", session_id="session-id", archived=True, cursor=hits.next_cursor,
    )
```

Search is a literal substring match, ASCII case-insensitive; Chinese text is
matched directly. `%` and `_` are ordinary characters. `limit` is 1–100
(default 20). Each match contains a bounded excerpt, and `field` is `title`,
`content`, `thinking`, or `tool_calls`. Title matches have no message ID.
Search does not open separately persisted large-output files or image content.
Search pages have a scan budget as well as a result limit. **An empty page with
`has_more=True` is not the end of the search.** Pass `cursor=hits.next_cursor`
with the same query, archive state and session/workspace selection to continue.
`has_more=False` means all selected history has been scanned. `scanned_messages`
and `scanned_bytes` describe the work performed by this call.

Sessions are visited by recent activity, title first, then messages from newest
to oldest. Each call scans at most 512 message rows, with an 8 MiB / 100 ms target
budget checked between text fields; one large field or storage wait may exceed
the target. Only the current row and bounded excerpts are held in memory. New
messages appended after the initial call are excluded; restart the search to
include them. Existing text edits, archive changes and deletions remain live.
The opaque cursor records progress, so subsequent pages skip scanned fields
instead of rescanning and sorting the whole history. There is no full-text index;
finishing a search with no matches still requires scanning the selected text.

Read recent content first, then request older pages only when needed:

```python
page = await locus.read_session("session-id", limit=30)
for message in page.messages:
    print(message.role, message.content)

if page.has_more_history:
    page = await locus.read_session(
        "session-id", before_row_id=page.oldest_message_row_id, limit=30,
    )
    for message in page.messages:
        print(message.role, message.content)
```

`read_session` supports both archive states and rejects session IDs outside the
selected checkout. The first page is the latest; subsequent pages move backward,
with messages in chronological order inside each page. `limit` is 1–1000
(default 50), a target size that can expand to keep a tool call and its results
together. Use `has_more_history` to decide whether to continue, even if a page
has no visible messages. The row cursor is exclusive and remains usable when
new messages arrive. Additional message fields, such as `toolCalls` and
`thinkingContent`, are available in `message.raw`.

`get_session()` and `SessionSummary.load()` still load the full history for
compatibility and continuation workflows. Prefer `read_session()` for inspection.

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
