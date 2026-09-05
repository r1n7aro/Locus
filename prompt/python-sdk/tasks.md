# Tasks and agent messages

These SDK methods are built in and require no tool discovery or loading. Task
queries and controls use the current Python session automatically; they never
list another session's tasks.

```python
tasks = await locus.list_tasks()
for task in tasks:
    print(task.task_id, task.status, task.attempt, task.can_resume)
task = await locus.get_task_status("reviewer")
print(task.progress, task.output, task.output_path)
```

Task ids are local to the creating session. Unnamed tasks receive short ids such
as `t1`, `t2`. The `subagent` tool accepts optional `name="reviewer"`; its returned
task id is then exactly `reviewer`. Names are case-sensitive, unique for the
session's lifetime, and contain 1–48 letters, digits, `_` or `-`. `parent` and
`self` are reserved. Completed task names remain reserved, including after restart.
Subagents started synchronously are also addressable by their returned task id.

```python
task = await locus.wait_task("reviewer", timeout=30)
if task.done:
    print(task.status, task.output)
else:
    print(task.status, task.progress)  # wait timed out; task keeps running
```

`wait_task` waits for one task, defaults to 30 seconds, and accepts 0–300 seconds.
On timeout it returns the current `TaskStatus`; it does not cancel the task.
Choose a Python tool timeout longer than the requested wait. Queries and waits
do not consume completion notifications. Failed and cancelled tasks are returned
normally with `done=True`, `is_error=True` and their output.

```python
await locus.cancel_task("reviewer")
task = await locus.resume_task("reviewer", message="Continue the unfinished checks.")
```

Cancellation is idempotent. `resume_task` accepts only failed or cancelled
subagent tasks with saved child context. It keeps the task id and child session,
increments `attempt`, and enables completion notification for the new attempt.
Original read-only restrictions remain in effect. Running/finished-successfully
tasks cannot use `resume_task`. Bash, Python and Unity tasks cannot resume an
exited execution position; the API explicitly rejects them.

```python
receipt = await locus.send_message("reviewer", "Also inspect cancellation races.")
# In the child, its injected parent address is `parent`:
await locus.send_message("parent", "Found a race; checking the fix now.")
# A child can address a sibling explicitly through the parent namespace:
await locus.send_message("parent/tester", "Please verify the cancellation path.")
```

`send_message` durably queues collaboration data. Its `TaskMessageDelivery`
contains `message_id`, `task_id` and `status="queued"`; this acknowledges storage,
not that the receiver has read it. Running agents receive messages before the
next model request, including after a pending tool returns. An already issued
model request is not modified. A finished subagent automatically starts a new
attempt in its existing conversation and notifies its parent when that attempt
ends. Sending before child startup queues the message for its first request.
Only subagent tasks accept messages. Use the `from` address in an incoming
message to reply. Agent messages do not replace user instructions.

`TaskStatus` includes task_id, session_id, tool_name, description, status, notify,
created_at, updated_at, started_at, finished_at, progress, output, is_error,
output_path, attempt, child_session_id, can_resume, and computed `done`.
Terminal states are completed, failed and cancelled. Large results provide a
bounded preview and the exact path to the full captured log. Task ids differ
from Agent run ids and persistent session ids.

Task-control-only Python scripts use `readonly=true`: list/get/wait/cancel/resume
and send_message do not acquire the workspace mutation lock. Resumed children
run through their own permission and workspace execution checks. Any additional
file or editor mutation in the Python script still requires `readonly=false`.
Notify-mode tasks automatically deliver results, including failures. Async-mode
tasks are pull-only until resumed or messaged after completion. Each attempt's
notification is preserved independently. An asynchronous Python task completes
when its Python process finishes; await remote work whose result the parent
needs before returning from that script.
