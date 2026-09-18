# Selective Unity integration

For a known set of files, pass exact repository-relative `paths`. For complete file replacement, use `mode="files"`; this avoids building field diffs or C# schemas:

```python
job = await locus.merges.prepare(
    workspace_ref=workspace_ref,
    sources=[{"commits": [source_oid]}],
    paths=["Assets/Scenes/Blockout.unity"],
    mode="files",
)
plan = job.plan()
await plan.files.take("Assets/Scenes/Blockout.unity", version="source", commit=source_oid)
preview = await plan.preview()
```

`paths` adds the asset/meta pair to the available scope but does not select either automatically. It accepts files, not directories or globs. Scoped plans cannot write outside these paths. `mode="structural"` (default) supports the usual field/object operations; its catalog is computed when `changes` or structural selections first require it. `mode="files"` supports whole-file operations and still checks missing GUIDs and references to removed objects. Omit `paths` to retain the full-target snapshot behavior.

Scoped preparation stores raw bytes for write targets and dirty dependency inputs. Other dependency inputs refer to pinned immutable Git blobs; preview never substitutes later worktree contents. Private `refs/locus/merge-jobs/` refs keep those versions reachable. New jobs share a repository-level blob store; v1 jobs remain readable from their original local blobs.

`prepare` and `get` return a compact job descriptor. `job.snapshot` contains HEAD, branch, epoch and counts; use `await job.snapshot_page(kind="target", offset=0, limit=100)` or `kind="dependencies"` for paginated frozen identities. `job.prepare_metrics` reports phase durations, file counts and worker count. Preparation uses a shared pool (up to 32 workers, configurable with `LOCUS_MERGE_WORKERS=1..32`) and bounded file-read memory. Long preparation should run as an asynchronous Python tool workflow; cancellation of the waiter does not release a worker's actual coordination lease early.

`locus.merges` is a trusted structural write API backed by Locus's own Rust Unity parser/merger. It never invokes UnityYAMLMerge or asks the model to edit YAML. Use `readonly=false` for a mutating Python workflow. Nested SDK writes borrow the Python run's real checkout lease; do not falsify `readonly` to avoid coordination. The lease expires with the Python process. Cross-checkout contention returns a retryable busy error instead of waiting in a lock cycle.

```python
job = await locus.merges.prepare(
    sources=[{"branch_ref": "feature", "commits": [first_oid, third_oid]}],
    destination={"kind": "checkout", "workspace_ref": workspace_ref},
)
catalog = await job.changes(limit=1000)
print(catalog.changes)  # Includes clean AND conflicting source changes.
plan = await job.new_plan(default="keep_target")
await plan.include(change_ids=chosen_ids)
await plan.exclude(change_ids=excluded_ids)
await plan.defer(change_ids=deferred_ids)
await plan.resolve(conflicting_change_id, side="source")
preview = await plan.preview()
print(preview.issues, preview.files, preview.plan_hash)
if preview.ready_to_apply:
    applied = await plan.apply(expected_plan_hash=preview.plan_hash)
    validated = await plan.validate(level="unity")
```

Nothing is included by default. `include(all=True)` is an explicit broad selection. Source lists preserve the supplied order; `{"range":"base..tip"}` expands in topological order. Non-contiguous commits use each selected commit's parent delta and do not absorb omitted intermediate commits. A merge commit requires `mainline={commit_oid: 1}` (one-based parent).

The destination includes its original HEAD, complete index records, disk changes, and relevant untracked files. `apply` modifies the planned working files and preserves HEAD/index. Planning-time local edits are supported; edits after snapshotting return `stale` and require a new job. `job.plan()` resumes saved decisions; `new_plan()` clears them.

`validate(level="static")` checks a preview. **Unity validation without `paths` runs after apply in the chosen destination**, waits for compile/domain reload, imports applied assets and their Unity dependencies, and checks missing scripts, object references, and managed-reference types, including ScriptedImporter metadata. Selected code/import-setting changes expand the asset validation scope; existing dirty code still compiles as the destination schema. Importer side effects invalidate validation and require review/replanning. Unity 6.5 (`6000.5.x`) is required. Asset/code commits require successful Unity validation. Dirty scenes/Prefab stages and Play Mode block disk apply until saved/exited. Exact commit-candidate validation with `paths` is described below.

Destinations can name either participant checkout. A new branch can inherit the base's dirty HEAD/index/worktree layers:

```python
destination = {"kind": "new_branch", "name": "integration/chosen",
               "base_workspace_ref": workspace_ref,
               "location": "new_worktree"}  # or current_checkout
```

Optional `path` names the new worktree directory. The returned job carries the actual destination WorkspaceRef. Changing direction means preparing a new job.

Typed operations all feed the same preview:

Inspect complete frozen assets before choosing unchanged objects or fields:

```python
view = await job.inspect_asset("Assets/graph.asset", version="target", limit=200)
print(view.objects, view.fields, view.references)
field = await job.inspect_asset("Assets/graph.asset", version="source", commit=source_oid,
    object_id="11400000",
    property_path="/MonoBehaviour/references/RefIds/@rid=101/data/health")
```

Versions are `target` (the frozen dirty destination), `source`, `base` (the selected commit's parent), and `result` (the current preview, including readiness/issues). Source/base queries require an exact selected commit when several are present and can inspect files unchanged by that commit. Objects/fields include fields without source deltas. Field paths use the merger's own stable identity selectors; sequences with only positional addressing explicitly report `stable_identity_path=False`. IDs remain strings. `scalar_text` preserves lexical spelling and `scalar_style` identifies quoting; long scalars report truncation, with `scalar_limit` adjustable up to 65,536 bytes. Fields paginate with `offset`/`next_offset`; references independently paginate with `reference_offset`/`next_reference_offset` and use paths in the immutable inspected snapshot. `object_id` filters both; `property_path` filters the field subtree. Binary files report their kind and require full-version selection.

- `plan.files.include/exclude(path, commit=...)` select source deltas for a file. `files.take(path, version={"side":"source","commit":oid})` chooses one complete version. `files.delete(path)` and `files.move(path,destination)` are explicit file operations; asset/meta dependencies are checked. `files.clear(path)` removes a whole-file choice before using interior operations.
- FBX and other binary/opaque files **require** `files.take` with an explicit full version (`source`, `target`, or `base`); multiple source variants require the exact selected commit. `include` cannot implicitly choose binary bytes.
- `plan.objects.include/exclude(path, object_id="...")` select deltas for one object. `objects.take(..., side="source", commit=oid)` takes the entire object, including fields unchanged by the commit. `objects.add/delete` perform structural object operations. `objects.move(path,object_id=transform_file_id,parent_id=new_parent_file_id,position=...)` reparents a Transform/RectTransform and updates both parent child lists atomically. Use `parent_id="0"` for a root.
- `plan.fields.take(path,object_id="...",property_path="/MonoBehaviour/health",side="source",commit=oid)` selects one field version. `fields.set(...,value=120)` supplies typed JSON; strings are values, never raw YAML. `fields.delete(...)` removes a field. Object selectors, identity path segments, and inspection reference IDs are strings. To set a numeric `fileID` or `rid`, pass a Python integer, for example `value=int("9007199254740993")` or `value={"fileID": int(reference["file_id"])}`. Python-to-Rust JSON preserves signed 64-bit integers exactly; a string value stays quoted and is rejected as an invalid numeric reference. Host-scoped SerializeReference IDs and graph dependencies are validated.

File mode changes appear as separate `file_mode` decisions and require `files.take` for a complete version. Excluding a mode decision while including content retains the destination mode, including on Windows where executable bits are recorded in Git rather than the filesystem. Malformed or unknown selection parameters are rejected; they never broaden a field/object selector to an entire file. Select by `change_ids` or by path/object/property/commit filters; do not combine these two forms. When `files.take` supplies both `commit` and `version.commit`, they must match exactly.

File/object choices overlapping finer operations return a conflict. Unknown syntax, ambiguous sequence identity, missing dependencies, incompatible alias/type changes, and unsupported transformations require a revised explicit plan; the engine does not silently include dependencies. Direct, unambiguous `FormerlySerializedAs` aliases are inferred only from matching snapshot schemas with the same declared field type; conditional/partial/inherited type inference is not treated as complete.

Staging and committing are separate, explicitly scoped operations:

```python
await plan.stage(paths=selected_paths, include_local_changes=False)
await plan.commit(paths=selected_paths, message="Integrate selected changes",
                  include_local_changes=False)
```

To commit a subset while leaving unrelated dirty assets/code in the destination, validate the exact commit tree first:

```python
checked = await plan.validate(level="unity", paths=selected_paths,
                              include_local_changes=False)
await plan.commit(paths=selected_paths, message="Integrate reviewed subset",
                  include_local_changes=False)
```

This constructs HEAD plus only those paths in a reusable isolated Unity checkout. Validation is bound to the candidate Git tree, plan hash, parent, paths, and local-change consent. `commit` reconstructs and checks that exact tree before updating the destination ref; other staged/working changes stay in place. The verification Editor checks candidate assets and compilation without importing the destination's excluded dirty code. Its temporary Editor validation harness is separately owned and removed after checking importer/shutdown side effects.

Isolated validation can certify an exact CRLF-to-LF save only when the before/after byte fingerprints, immutable candidate blob, and Git attribute checks agree. Its result records `source_eol_normalizations` and source snapshot hashes. Changed content, added or ignored source files, missing files, binary/LFS data, and arbitrary filters receive no exception. Closing cleanup can refresh only these proven paths through a shadow index while preserving index records/flags, HEAD/tree, and all working bytes; a dirty or polluted pool slot remains unavailable for reuse.

Paths with pre-existing index or working changes require explicit `include_local_changes=True` for stage/commit; with that flag, explicitly named frozen local paths can also be included. Other staged paths remain untouched. A commit subset is checked again against HEAD: missing asset/meta/script dependencies block it. Unity evidence applies only to the tested source/asset state; a narrower commit with a different schema or asset context returns `needs_commit_validation` until the required paths are included or that narrower result is validated in another checkout. Selective integration produces a single-parent commit and retains its source/selection manifest, so excluded source history is not marked merged. Successful apply/commit retries are idempotent. `abort()` conditionally restores only this job's writes, preserving any later Editor/user edits; it does not reset staging (including an interrupted staging transaction) or rewrite commits.

Git LFS file choices materialize the selected complete object from the local LFS cache and verify its size/SHA-256. Missing objects require fetching the selected commit's LFS objects before retrying preview. Pointer text is never written over an asset.

`prepare` and `get` accept `worktree=handle` as an alternative to `workspace_ref`. Use `locus.worktrees.acquire(commit=source_oid, ...)` for live inspection of an exact source revision, then pass the returned `.worktree` to Unity calls. Use `job.workspace_ref` for destination calls after preparation, especially when it creates a new worktree. Live analysis does not replace frozen inspection, stale-plan checks or Unity validation. Exact candidate validation may need another Editor slot; close the source analysis Editor first when capacity is full.
