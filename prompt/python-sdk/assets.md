# Unity asset editing

`locus.assets` provides the same snapshot, field paths and explicit operations for Rust YAML editing and a connected Unity Editor. Use `readonly=false` for Python workflows that apply changes; nested writes reuse the Python execution's checkout lease. A Locus process must be running, but the YAML backend needs no Unity Editor process and never starts one.

```python
import locus

assets = locus.assets.backend("yaml", workspace_ref=workspace_ref)
# Or: locus.assets.backend("live", worktree=worktree)
snapshot = await assets.read("Assets/Config.asset")
print(snapshot.revision, snapshot.objects)

operations = [{
    "op": "set",
    "object_id": "11400000",
    "property_path": "/MonoBehaviour/speed",
    "value": 12.5,
}]
preview = await assets.preview("Assets/Config.asset", operations,
                               expected_revision=snapshot.revision)
applied = await assets.apply("Assets/Config.asset", operations,
                            expected_revision=snapshot.revision)
print(applied.applied, applied.persisted, applied.snapshot["revision"])
```

`backend()` returns a new context bound to one checkout and backend. Switching from `yaml` to `live` leaves the read/edit calls unchanged. There is no automatic fallback. YAML reads serialized disk state and performs edits in Rust. With no Editor, it writes directly without launching one. With an Editor connected, it coordinates one batch replacement/import on the Editor's main thread and rejects dirty target assets while preserving unrelated dirty state. Live uses the already connected, ready Editor and persists successful edits to disk. `capabilities()` reports backend operation and extension support.

Snapshots have `revision`, `objects`, and `diagnostics`. Each object has exact string `object_id`, `class_id`, `root_type`, and a flat `fields` list. Fields have `property_path`, serialized `kind`, and `value`; container fields and their descendants are both available. Paths are root-inclusive RFC 6901 pointers, such as `/MonoBehaviour/speed` or `/MonoBehaviour/items/0/name`. Escape literal `~` and `/` as `~0` and `~1`. Use returned paths directly, including stable `@rid=...` selectors for managed-reference data. Serialized shape does not replace a C# type schema; Unity stores booleans and enums as integers.

`read(path, object_id=..., property_path=...)` filters a snapshot. `discover(path, query=..., object_id=..., property_path=..., offset=0, limit=100)` returns `revision`, `matches`, `total`, `truncated`, and `next_offset`. Each match includes its `object_id`, `property_path`, `kind`, and `value`.

All operations require an exact string `object_id` and a `property_path`:

| `op` | Additional fields | Behavior |
| --- | --- | --- |
| `set` | `value` | Replace the addressed field value |
| `array_insert` | `index`, `value` | Insert an explicit value before an index |
| `array_remove` | `index` | Remove one item |
| `array_move` | `index`, `to_index` | Move an item to its final destination index |
| `array_resize` | `size`, optional `value` | Truncate, or grow with an explicit fill value |

Array growth requires `value`; it never duplicates the last item implicitly. Operations are ordered. Unknown fields, unsupported targets and invalid operations fail rather than being ignored. `value={"action":"resize",...}` is ordinary data, not an implicit array command.

IDs remain strings. Reference maps use `{"fileID":"11400000","guid":"...","type":2}` or `{"rid":"101"}`. Scalar integers outside JavaScript's exact range use `{"kind":"int64","value":"9007199254740993"}`. `locus.asset_integer(decimal_or_int)` and `assets.integer(...)` construct signed 64-bit values. Unsigned 64-bit values use `locus.asset_unsigned_integer(...)` or `assets.unsigned_integer(...)`, producing `{"kind":"uint64","value":"18446744073709551615"}` for the maximum. Reading and resubmitting either tag preserves its exact value; tags require a canonical decimal string. Object IDs and reference IDs remain restricted to signed 64-bit range. Python automatically encodes large signed `int` values and numeric reference IDs without rounding; use the unsigned helper for larger positive values. Strings otherwise remain strings. Non-finite floats, unsafe floating-point integers and out-of-range values are rejected.

Large changes use one batch request:

```python
entries = []
for path in paths:
    snapshot = await assets.read(path)
    entries.append({"path": path, "expected_revision": snapshot.revision,
                    "operations": build_operations(snapshot)})

preview = await assets.preview_batch(entries)
result = await assets.apply_batch(entries)
print(result.applied, result.persisted, len(result.results))
```

Every apply requires the revision returned by a current read/discovery. The preview's proposed revision describes the result, so use the original input revision when applying that preview. Every batch is validated before mutation and committed as one transaction with rollback; filesystem observers may see individual files being replaced. Preview returns `applied=false,persisted=false`; successful apply returns `applied=true,persisted=true`. Each result includes `snapshot`, `operations_count`, and `diagnostics`. A stale revision fails before mutation. Do not blindly replay a timed-out write: re-read the assets to determine whether it completed, especially for array operations. An interrupted YAML transaction can be recovered with `await assets.recover(transaction_id)` using its recorded transaction ID; recovery checks recorded file hashes and refuses to overwrite later edits.

Merge plans reuse these operations through `job.assets` and `plan.assets`. Their destination is explicitly `merge_plan`, and persistence is `plan`:

```python
snapshot = await plan.assets.read("Assets/Config.asset")
await plan.assets.preview("Assets/Config.asset", operations,
                          expected_revision=snapshot.revision, persist="plan")
selected = await plan.assets.apply("Assets/Config.asset", operations,
                                   expected_revision=snapshot.revision, persist="plan")
preview = await plan.preview()
await plan.apply(expected_plan_hash=preview.plan_hash)
```

Merge asset apply updates the saved selections and returns `applied=true,persisted=false`; the outer `plan.apply()` writes project files. The asset snapshot is the current frozen merge result, so unresolved conflicts must be resolved first. Read and preview leave the plan unchanged. See the merges help for dependency checks, Unity validation and commit controls.

The common contract edits serialized assets supported by both backends; inspect `capabilities().supported_extensions` before targeting another file type. Binary/imported assets, arbitrary runtime objects, C# type creation and Prefab inheritance projection are not implied by this API. Unsupported operations return an error. Static YAML validation checks serialized structure and references; actual Unity import and behavior still require Editor validation. Existing `locus.unity.property` APIs in TypeScript are legacy Editor-specific inspector APIs with a different path/value contract.
