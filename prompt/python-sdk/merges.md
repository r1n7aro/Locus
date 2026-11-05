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

Preparation stores raw bytes only for writable targets (paths the selected commits change, the `paths` scope, and for a full snapshot the destination's dirty and untracked paths) and for dirty dependency inputs. Every other Unity input refers to its pinned immutable HEAD blob; preview never substitutes later worktree contents. A plan operation that writes such a file (for example `files.delete` or a field edit) first checks it still matches that blob, otherwise it fails with `stale`. Private `refs/locus/merge-jobs/` refs keep those versions reachable.

A job's journal lasts only as long as the job. `plan.commit` and `plan.abort` remove it and return `removed: true`; afterwards the job ID answers that it no longer exists (abort again is a no-op). An applied or staged job stays available for `validate`, `stage` and `commit` until its destination moves on (HEAD, branch or an applied file changes) or after a week unused; a prepared job expires after three days unused; a job mid-transaction or needing recovery is kept. Each prepare and each job end removes finished jobs, the stored file versions only they used, and aged caches. Jobs from earlier Locus versions are not read; prepare a new job.

`prepare` and `get` return a compact job descriptor. `job.snapshot` contains HEAD, branch, epoch and counts; use `await job.snapshot_page(kind="target", offset=0, limit=100)` or `kind="dependencies"` for paginated frozen identities. `job.prepare_metrics` reports phase durations, file counts and worker count. Preparation uses a shared pool (up to 32 workers, configurable with `LOCUS_MERGE_WORKERS=1..32`) and bounded file-read memory. Long preparation should run as an asynchronous Python tool workflow; cancellation of the waiter does not release a worker's actual coordination lease early.

`locus.merges` is a trusted structural write API backed by Locus's own Rust Unity parser/merger. It never invokes UnityYAMLMerge or asks the model to edit YAML. Use `readonly=false` for a mutating Python workflow. Nested SDK writes borrow the Python run's real checkout lease; do not falsify `readonly` to avoid coordination. The lease expires with the Python process. Cross-checkout contention returns a retryable busy error instead of waiting in a lock cycle.

```python
job = await locus.merges.prepare(
    sources=[{"branch_ref": "feature", "commits": [first_oid, third_oid]}],
    destination={"kind": "checkout", "workspace_ref": workspace_ref},
)
summary = await job.change_summary()  # Per file: statuses, kinds, destination_changed.
conflicts = await job.all_changes(status="conflict")  # Only what needs a decision; each has a reason.
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

Read the catalog on the server's terms: a large scene can hold tens of thousands of clean changes. `change_summary()` lists every file with its `changes`, `statuses` and `kinds` counts, its `commits`, and `destination_changed` (false when the destination still holds the base, so including the file's changes takes the source file), plus total `statuses`. `all_changes(status=..., paths=[...])` and `changes(status=..., paths=[...], offset, limit)` filter on the server by status (a string or list) and exact file paths; `total` counts the matches and `catalog_total` the whole catalog. Select clean changes with `include(all=True)` or `include(path=...)` instead of reading them.

Change statuses: `clean` means `include` applies without a decision; `already_applied` means the destination already has it; `conflict` always carries a `reason` naming the disagreement and the sides that resolve it; `explicit_choice` marks file-mode changes. Resolve conflicts by change ID (`resolve(change_id, side=...)`) or with a typed field operation. Preview issues carry `change_id`/`change_ids` (and `overlaps` for overlapping operations) so each one maps back to the catalog; a validation finding that names an object (for example `missing_owner_component`) lists that object's changes in `related_change_ids`.

An object deletion is one decision. When the source deletes an object and that deletion conflicts with a destination edit, the rest of the same deletion (its GameObject, components and child Transforms, and the references in `m_Component`, `m_Children` and `m_Roots`) is `conflict` too; resolve them all with one side, `target` to keep the object attached or `source` to remove it entirely. To keep the source's edit of an object the destination deleted, call `plan.objects.restore(path, object_id=...)` (the conflict's `reason` names it). It brings back the deleted part of the hierarchy the object belongs to: its GameObject with every component, its children with their components and prefab instances, up to the nearest object the destination kept, plus the element that lists it there (`m_Component`, `m_Children`, the scene's `m_Roots` or a prefab instance's added objects). `result.restored` lists the `objects`, the `resolved_change_ids` and the `attached` elements; everything renders with that commit and is validated as one result. `resolve(..., side='target')` keeps it deleted. A field operation bound to a commit that overlaps no selected change also renders together with that commit's changes.

A file the destination never changed takes the source file byte for byte when all of its changes are included. Added objects from either side take their place in ascending fileID order, the order Unity itself writes, so an Editor save does not reorder a merged scene; in a destination that is not fileID-ordered, source additions follow their nearest source neighbour.

Code and plain text merge by lines. `include` applies a three-way line merge onto the destination's own lines, keeping their exact bytes and line endings; lines taken from the source follow the destination's CRLF/LF convention, and a working file that differs from its base only by line endings counts as unchanged. A text change whose destination also changed other lines is `clean` with `merge="line_merge"`. As in Git, edits that overlap or touch (adjacent lines, insertions at one boundary) are `conflict` with `hunks` (1-based line ranges and previews). `inspect_asset(path, version="result")` returns the destination with diff3 conflict markers (`<<<<<<< destination`, `||||||| base`, `=======`, `>>>>>>> source`); write the resolved file into the plan with `plan.files.set_text(path, merged_text)`. It is stored in the job, bound to the plan hash, applied and aborted with everything else, and supersedes include/resolve for that path, so a manual merge needs no destination edit and no new prepare. `eol="target"` (default) converts LF to a CRLF destination; `eol="exact"` keeps the content as given. Content with conflict markers, Unity YAML and binary files are rejected. Deletion/addition disagreements (one side deleted, the other modified; both added) are conflicts resolved with a side.

Nothing is included by default. `include(all=True)` is an explicit broad selection. Source lists preserve the supplied order; `{"range":"base..tip"}` expands in topological order. Non-contiguous commits use each selected commit's parent delta and do not absorb omitted intermediate commits. A merge commit requires `mainline={commit_oid: 1}` (one-based parent).

Selected commits that change the same file apply in order, like a sequence of cherry-picks. Each later commit is cataloged against the file as the earlier ones leave it when included in full (clean changes applied, conflicts kept at the destination), so a commit editing what an earlier selected commit added or changed is `clean`, not a conflict. If the plan's decisions for those earlier commits produce a different file, preview reports `earlier_commit_decisions` with the affected `change_ids` and `earlier_commits`: include the earlier commits' changes for that file or choose a complete version with `files.take`.

To integrate a branch's net result (a squash), use `{"branch_ref": "origin/feature", "squash": True}` (the same as `{"range": "HEAD..origin/feature", "squash": True}`). The job then holds a single delta from the merge base to the tip, so states that intermediate commits created and later reverted produce no changes or conflicts; `commit` is the tip OID and `version="base"` is the merge base. `squash` accepts a two-dot range or `branch_ref`, not a commit list, and rejects ambiguous merge bases. No temporary commit or manual path list is needed.

The destination includes its original HEAD, complete index records, disk changes, and relevant untracked files. `apply` modifies the planned working files and preserves HEAD/index. Planning-time local edits are supported; edits after snapshotting make `apply` fail with `stale` before anything is written. Then `job = await job.replan()` prepares a fresh snapshot from the job's exact source commits and bases (a moved source branch adds nothing) and carries over every decision whose files have unchanged destination bytes. `job.replan_report` lists the `dropped` decisions with their `reason`, the `changed_paths` and the `carried` counts; review them, preview again and apply with the new `plan_hash`. Replan accepts a prepared or aborted job on the same branch and removes the previous job once the new one is saved. `job.plan()` resumes saved decisions; `new_plan()` clears them.

`validate(level="static")` checks a preview. **Unity validation without `paths` runs after apply in the chosen destination**, waits for compile/domain reload, imports applied assets and their Unity dependencies, and checks missing scripts, object references, and managed-reference types, including ScriptedImporter metadata. Selected code/import-setting changes expand the asset validation scope; existing dirty code still compiles as the destination schema. Importer side effects invalidate validation and require review/replanning. Unity 6.5 (`6000.5.x`) is required. Asset/code commits require successful Unity validation. Dirty scenes/Prefab stages and Play Mode block disk apply until saved/exited. Exact commit-candidate validation with `paths` is described below.

A plan of existing Unity text assets is written through the connected Editor's disk transaction; the Editor reads the merged bytes from copies staged in the project's `Library`, so scene size is not limited by the bridge's request size. A plan that also contains code, text, `.meta` or new files is written directly, and the connected Editor then reimports the written assets and reloads affected open scenes and the Prefab Stage from disk, so it never keeps (and later saves) the previous in-memory version. The apply result reports this as `editor_sync` (`reloaded.imported`, `reloaded.reloaded_scenes`, `reloaded.stale`, `scripts.scripts_changed`) and lists anything requiring attention in `warnings`: assets that gained unsaved Editor edits are reported as stale and never overwritten. Changed scripts compile on the Editor's next refresh; run `unity_recompile` before relying on new types.

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

- `plan.files.include/exclude(path, commit=...)` select source deltas for a file. `files.take(path, version={"side":"source","commit":oid})` chooses one complete version. `files.set_text(path, content)` supplies merged code/plain text (see above). `files.delete(path)` and `files.move(path,destination)` are explicit file operations; asset/meta dependencies are checked. `files.clear(path)` removes a whole-file choice before using interior operations.
- FBX and other binary/opaque files **require** `files.take` with an explicit full version (`source`, `target`, or `base`); multiple source variants require the exact selected commit. `include` cannot implicitly choose binary bytes.
- `plan.objects.include/exclude(path, object_id="...")` select deltas for one object. `objects.take(..., side="source", commit=oid)` takes the entire object, including fields unchanged by the commit. `objects.add/delete` perform structural object operations. `objects.move(path,object_id=transform_file_id,parent_id=new_parent_file_id,position=...)` reparents a Transform/RectTransform and updates both parent child lists atomically. Use `parent_id="0"` for a root.
- `plan.fields.take(path,object_id="...",property_path="/MonoBehaviour/health",side="source",commit=oid)` selects one field version. `fields.set(...,value=120)` supplies typed JSON; strings are values, never raw YAML. `fields.delete(...)` removes a field. Object selectors, identity path segments, and inspection reference IDs are strings. To set a numeric `fileID` or `rid`, pass a Python integer, for example `value=int("9007199254740993")` or `value={"fileID": int(reference["file_id"])}`. Python-to-Rust JSON preserves signed 64-bit integers exactly; a string value stays quoted and is rejected as an invalid numeric reference. Host-scoped SerializeReference IDs and graph dependencies are validated.

File mode changes appear as separate `file_mode` decisions and require `files.take` for a complete version. Excluding a mode decision while including content retains the destination mode, including on Windows where executable bits are recorded in Git rather than the filesystem. Malformed or unknown selection parameters are rejected; they never broaden a field/object selector to an entire file. Select by `change_ids` or by path/object/property/commit filters; do not combine these two forms. When `files.take` supplies both `commit` and `version.commit`, they must match exactly.

Identity sequences merge per element: Prefab overrides (`m_Modifications`, addressed `@override=<target>:<propertyPath>`), objects added to a prefab instance (`m_AddedGameObjects`/`m_AddedComponents`, `@added=<fileID>`), removed instance objects (`m_RemovedComponents`/`m_RemovedGameObjects`, `@removed=<guid>:<fileID>`), managed references (`RefIds`, `@rid=`), components (`@component=`), children (`@child=`) and scene roots (`SceneRoots/m_Roots`, `@root=`). Additions, deletions and edits of different elements from either side combine, also when the base list was Unity's empty `[]`; a value both sides changed differently conflicts at that element's field, and one side deleting an element the other modified conflicts at the element. Source additions enter after their nearest source neighbor. Identity spelling is decoded, so a quoted and a plain `propertyPath` address one element. Override, added/removed-object and managed-reference order carries no meaning; when the source reorders significant elements (components, children, scene roots), when the destination or source list is a flow/empty list, or when elements have no Unity identity (plain value lists, `[SerializeReference]` lists of `{rid: N}`), the whole sequence remains one `replace_sequence` choice whose `reason` names the next step: append-only on both sides means `fields.set` of the combined list; element edits on both sides mean `resolve` one side, then `fields.take` the other side's `<path>/<index>` elements.

Both sides adding objects to the same prefab instance write the same stripped proxy document (its fileID is derived from the instance and the source object); identical proxies count as already applied. Two *different* objects added with one fileID remain an object conflict (`both sides added an object with this fileID`) and only one side can be kept. A single Unity Editor session hands out the same next scene fileID when the same base scene is edited twice (for example after switching branches inside one open Editor); separate sessions do not. A managed reference deleted by one side and edited by the other conflicts at its `RefIds/@rid=` element; the reason lists the fields that point to it, which a restore must also take from the source.

Element paths work with every field operation on block sequences: `fields.delete` removes an element, `fields.take(side="source"|"base")` copies an element's exact bytes (inserting it in that snapshot's order when the destination lacks it), and `fields.set` inserts or replaces one, where an identity-addressed value must carry that identity. The first field of an element shares its `- ` line, and values inside flow collections are not deleted alone; set the element or the flow value instead.

Field operations run after catalog decisions, on their result. A field operation strictly inside a selected change refines it. For example, keep the source's override list but drop one element with `resolve(sequence_change_id, side="source")` and then `fields.delete(path, object_id=..., property_path=element_path)`; the copied elements keep their exact source bytes. An operation on the same node as, or enclosing, a selected change is reported as `overlapping_selection` with `overlaps` naming the change IDs or operations.

`fields.set` and `plan.assets` edits write minimally: unchanged scalars, comments and elements keep their bytes, and identity elements are matched by identity. A directly addressed string field is written double-quoted. Scalars changed inside a container you set keep their plain spelling when the new text has the same YAML type (a number stays a number, text stays text); other text is double-quoted, and a scalar that used `\uXXXX` escapes keeps that spelling. Shapes a patch cannot express rewrite only the enclosing entry, element or flow value. [Asset editing](assets.md) describes `plan.assets` (`array_insert`, `array_remove`, ...) on the frozen merge result.

File/object choices overlapping finer operations return a conflict. Unknown syntax, ambiguous sequence identity, missing dependencies, incompatible alias/type changes, and unsupported transformations require a revised explicit plan; the engine does not silently include dependencies.

Removing objects from a Prefab or other asset checks every frozen asset for references to them (`removed_object_dependency` names the exact referrer). Scene objects cannot be referenced from other assets, so removals inside `.unity` files need no such proof. Only assets that spell an affected GUID are examined further. An asset beyond the parser's size/node budgets or dialect is checked by a lexical reference scan: its verdict is exact, and its issues add `line` with an approximate `property_path` (`property_path_approximate=True`). Git LFS-stored text assets are read from the local LFS cache. `external_object_dependency_coverage` (object removal, with `guids` and `deleted_from`) or `dependency_coverage` (GUID removal, with `guids`) remains only when such an asset names an affected GUID in a `guid` key outside a recognizable `{fileID, guid}` reference, or its LFS object is not cached (fetch it). Text that cannot be a key is ignored: string content (including continuation lines of wrapped strings and JSON kept in strings), and `guid` values that are not GUIDs, such as Wwise's dashed IDs. Resolve it by excluding those removals or taking a Unity text-format version of the referrer; do not exclude unrelated source changes or write files outside the plan to clear an issue. Direct, unambiguous `FormerlySerializedAs` aliases are inferred only from matching snapshot schemas with the same declared field type; conditional/partial/inherited type inference is not treated as complete.

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

Paths with pre-existing index or working changes require explicit `include_local_changes=True` for stage/commit; with that flag, explicitly named frozen local paths can also be included. Other staged paths remain untouched. A commit subset is checked again against HEAD: missing asset/meta/script dependencies block it. Unity evidence applies only to the tested source/asset state; a narrower commit with a different schema or asset context returns `needs_commit_validation` until the required paths are included or that narrower result is validated in another checkout. Selective integration produces a single-parent commit and retains its source/selection manifest, so excluded source history is not marked merged. Successful apply/commit retries are idempotent: a repeated `apply` returns `idempotent=True` and the applied `files` only, so keep the first result for `editor_sync`/`warnings`. `abort()` conditionally restores only this job's writes, preserving any later Editor/user edits; it does not reset staging (including an interrupted staging transaction) or rewrite commits. Restoring an applied Unity asset, scene, Prefab or `.meta` requires the destination Editor to be closed, since the Editor may hold unsaved changes to it; code and plain-text writes are restored with the Editor open.

Git LFS file choices materialize the selected complete object from the local LFS cache and verify its size/SHA-256. A missing object does not fail preview: every affected file reports an `lfs_object_missing` issue with `path`, `version`, `commit`, `oid`, `size` and `fetch_command`. Issues from the same commit share one `git lfs fetch` command covering all of that commit's missing paths; run each distinct command once (it only fills the LFS cache and leaves the working tree untouched), then preview again. Pointer text is never written over an asset.

`prepare` and `get` accept `worktree=handle` as an alternative to `workspace_ref`. Use `locus.worktrees.acquire(commit=source_oid, ...)` for live inspection of an exact source revision, then pass the returned `.worktree` to Unity calls. Use `job.workspace_ref` for destination calls after preparation, especially when it creates a new worktree. Live analysis does not replace frozen inspection, stale-plan checks or Unity validation. Exact candidate validation may need another Editor slot; close the source analysis Editor first when capacity is full.
