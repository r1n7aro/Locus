# Worktrees and Unity project slots

`locus.worktrees` manages sibling checkouts of the current logical Unity project and Git repository. The source defaults to the Python tool's injected checkout; pass `workspace_ref=` to choose another source. Calls leave the Python process's working directory and the Agent session binding unchanged.

```python
worktrees = await locus.worktrees.list()
for wt in worktrees:
    print(wt.checkout_id, wt.root, wt.branch, wt.lifecycle, wt.assignment_id)

a = await locus.worktrees.create(
    destination="F:/UnityWorktrees/feature-a",  # Absolute, unused directory; parent must exist.
    branch="feature-a", start_ref="HEAD", include_dirty=False,
)
# Explicitly resolve/reopen an existing active managed/imported checkout:
a = await locus.worktrees.get(a.checkout_id)

external_paths = await locus.worktrees.discover()
# An external Git worktree must be imported before use:
# external = await locus.worktrees.import_worktree(external_paths[0])
```

`include_dirty=True` creates a private snapshot of the source's local state; the source HEAD/index/files remain intact. It cannot be combined with an unrelated `start_ref`. `list()` includes idle, removed and quarantined records; only active assignments can run tools. It does not start Editor/services. Use `get()` to explicitly register an active checkout after a Locus restart. Failed/interrupted operations can be inspected via `operations()`.

The returned `Worktree` is accepted as `worktree=` by `call_tool`, `ToolInfo.call`, `list_tools`, `get_tool`, `get_workspace`, all Unity lifecycle/dialog/execution-wait APIs, and `merges.prepare/get`. Existing `workspace_ref=` remains supported. Do not supply both selectors. A bare checkout ID must first be resolved with `worktrees.get(id)`.

For managed worktrees, call Unity tools directly. Locus reuses an existing Editor, preferring the user's interactive instance. If none is running, it prepares the plugin and starts a managed headless Editor. The same process is retained across Agent turns and released after the configured idle timeout (one hour by default); active tasks, tests and unsaved Editor content prevent automatic shutdown. Do not start or close Editors at each turn. Interactive Editors remain under user control. A cold first import may need a longer tool timeout.

```python
slot = await locus.worktrees.acquire(
    pool_root="F:/UnityPool", commit=source_oid, max_slots=2,
    assignment_id="merge-source-analysis",  # Optional idempotent assignment key.
)
b = slot.worktree
print(slot.reused, slot.preserved_library, b.materialization_epoch)

import asyncio
results = await asyncio.gather(*(
    locus.call_tool("unity_execute", {
        "code": "print(UnityEngine.Application.dataPath);",
        "request_editor_status": "editing", "readonly": True,
    }, worktree=wt, timeout=900)
    for wt in (a, b)
))
for wt, result in zip((a, b), results):
    result.raise_for_error()
    print(wt.checkout_id, result.output)
```

`max_slots` bounds physical slots in this repository/project pool. It does not increase the app's Editor concurrency setting. An exact Unity Editor version match is required to reuse a private Library. There is no writable Library sharing between active checkouts. Ensure uses per-checkout lifecycle coordination; independent checkouts can start and execute concurrently, subject to existing service/Editor budgets.

To finish an assignment, first close its Editor, preserve or commit source changes as appropriate, then release it. Source/index dirtiness, live runtime leases, open Editors, stale assignment IDs or epochs reject recycling. Removing a worktree never force-discards local work or deletes its branch.

```python
closed = await locus.close_unity_editor(worktree=b, timeout=60)
print(closed.closed_process_ids, closed.forced_process_ids)
await locus.worktrees.release(b)  # Returns the slot; keeps its private Library.
await locus.close_unity_editor(worktree=a)
await locus.worktrees.remove(a)   # Clean, closed, unassigned managed worktrees only.
```

Close/restart use the same project process ownership checks and normal-close/force fallback as existing lifecycle APIs. Dialogs can be queried/chosen with the same `worktree=`; `force=True` explicitly requests forced closure. Status and detached execution waits also accept the same selector. Explicit `project=` paths remain supported for compatibility.

Worktree handles carry checkout identity, runtime generation and durable materialization epoch. After pool reassignment, old handles are rejected. Use the new handle returned by `acquire`; do not refresh an old task's epoch to continue it on unrelated content. The source checkout cannot remove/release itself during its SDK request.

Use `readonly=False` for workflows that create/remove/assign checkouts, start/close Editors, write files or apply merges. Read-only cross-editor analysis needs no workspace write gate. When session file undo is disabled, opaque Python/bash calls retain the relaxed existing policy; known-path writes, Unity execution barriers and merge transactions keep their established coordination. Where an outer writable Python call already owns a gate, nested SDK writes borrow it; sibling contention returns a retryable busy error instead of a circular wait.

For merge analysis, materialize the selected source commit in a pool slot, inspect both Editors, then plan against frozen snapshots:

```python
job = await locus.merges.prepare(worktree=a, sources=[{"commits": [source_oid]}])
frozen = await job.inspect_asset("Assets/Example.asset", version="source", commit=source_oid)
plan = await job.new_plan(default="keep_target")
# Select explicit fields/objects/files using the frozen catalog; see merges.md.
# After preview and apply, analyze the destination with the job's actual reference:
# result = await locus.call_tool("unity_execute", arguments, workspace_ref=job.workspace_ref)
# await plan.validate(level="unity")
```

Live Editor observations are evidence for selecting a merge; they do not replace frozen `inspect_asset` data, the stale-plan check, or validation. For multiple noncontiguous commits, one current Editor is only one revision; acquire the relevant exact revision to inspect each selected variant. `validate(level="unity", paths=[...])` uses another pool checkout for the exact commit candidate and may need additional Editor capacity. Finish/release the source analysis slot before candidate validation if capacity is full.
