# Blender (`locus.blender`)

`bpy` exists only inside a Blender process. From the `python` tool, send code to a running Blender with `locus.blender`; the code runs on Blender's main thread with `bpy`, `mathutils`, `bmesh`, `locus`, `project` and the helper object `blender` pre-imported.

Choose the path by the job:

| Job | Use |
|---|---|
| One step that reads or changes the scene and the result should be seen | the `blender_execute` tool |
| Batches, program-side decisions, flows across Unity/files/sessions, several instances, external libraries | `python` + `@locus.blender.function` |
| Code assembled at run time | `python` + `await locus.blender.execute(code)` |
| An existing script in the workspace | `python` + `await locus.blender.execute_file(path)` |

## Functions

```python
bl = locus.blender
import numpy as np

@bl.function
def vertex_positions(name):
    mesh = bpy.data.objects[name].data
    co = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
    mesh.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)

points = await vertex_positions("Cube")          # numpy array back in Locus
other = await vertex_positions.on(inst)("Cube")  # another instance
```

- Only plain `def`; `async def`, generators and lambdas are refused at definition.
- Arguments bind locally (`inspect.signature().bind`; defaults evaluate in Locus), then travel to Blender. `*args`, `**kwargs`, keyword-only parameters work.
- Modules the function uses (`np` above) are imported inside Blender under the same name; a module Blender lacks raises there. Other `@bl.function` functions it calls travel along. Any other outer variable (constants, plain functions, objects) is an error naming it: pass it as a parameter.
- `bpy`, `mathutils`, `bmesh`, `blender` need no import. Locals are fresh on every call.
- The source is sent once per instance; later calls send only its hash and the arguments.
- `@bl.function(readonly=True)` skips the undo step.

## execute / execute_file

```python
result = await bl.execute(
    "obj = bpy.data.objects[name]\nobj.location.z += dz\nreturn tuple(obj.location)",
    args={"name": "Cube", "dz": 0.5},   # values become variables
    target=None, namespace="fresh", readonly=False, timeout=120,
)
print(result.value, result.instance.pid, result.warnings, result.waited_ms, result.duration_ms)
await bl.execute_file("tools/cleanup.py", args={"threshold": 0.01})
mod = await bl.import_module("tools/blend_utils.py")   # each call is one round trip
await mod.rename_all("old_", "new_")
```

Same semantics as `blender_execute`: `return` ends the code, top-level `await` works (it freezes Blender's UI while waiting, so keep long waits on the Locus side). Images in the value are not attached automatically; call `locus.attach_image(img)`.

Printed output appears in the `python` result after each call (prefixed with `[Blender <pid>]` once several instances are used). Different instances run in parallel with `asyncio.gather`; calls to one instance queue in order.

## Values

Both directions, for functions, `args` and return values:

- `None`, bool, int, float (`nan`/`inf` kept), str unchanged; lists, tuples, sets and dicts keep their type (non-string keys included).
- `bytes` and numpy arrays travel as binary (dtype and shape kept; object arrays refused). Without numpy in Locus, arrays arrive as `locus.blender.Array` (`.tolist()`). Large data uses shared memory automatically.
- Blender data blocks return as `locus.blender.Ref` (type, name, library; nested structs add `path`). Passing a `Ref` back resolves to the live object; a deleted or renamed target raises.
- Vectors, Eulers, Quaternions, Colors return as tuples; matrices as tuples of rows. Collections such as `mesh.vertices` return lists of `Ref` (more than 10000 items raise: use `foreach_get`).
- Images are `locus.Image` (`data`, `mime_type`, `width`, `height`, `meta`, `save(path)`).
- Anything else raises with its location, e.g. `args['mesh'][2]`.

Keep names, not bpy objects, across calls: undo or reloading a file invalidates references.

## Errors

- `locus.blender.BlenderError`: an exception inside Blender (`type`, `message`, `traceback`, `instance`).
- `locus.blender.NotExecuted`: Blender never started the code; retry is safe.
- `locus.blender.TimedOut`: the code started and did not finish; Locus asked Blender to cancel and the scene state is uncertain. C-level work such as a render cannot be interrupted.
- `locus.blender.TargetUnavailable`: no matching instance; `candidates` lists running ones.

Calling `locus.blender` from code that already runs in that same Blender raises: call `bpy` or `blender.<helper>` directly.

## Targets and instances

Without `target`, calls use the session target, else the only interactive Blender of this workspace (one whose open file lies in the checkout or an attached directory, that Locus launched for it, or that a session targets) that no other session targets. With none or several, `TargetUnavailable` lists them. A background instance is never chosen implicitly.

Parallel sessions each work in their own instance: `await bl.launch(file="scenes/a.blend")` returns once the file is open, then `await bl.set_target(file="scenes/a.blend")`. An instance another session targets is used only with an explicit `target`, and results warn about it.

```python
for inst in await bl.instances():            # scope="all" includes other workspaces
    print(inst.key, inst.pid, inst.version_string, inst.file, inst.mode, inst.is_target)
await bl.set_target(pid=1234)                # or set_target(inst), file="art/hero.blend", background=True, version="4.2"
await bl.clear_target()
inst = await bl.launch("art/hero.blend")     # interactive; stays open after Locus exits
bg = await bl.background(version="4.2")      # this checkout's `-b --factory-startup` instance
await bl.close(bg)                           # background instances only
await bl.reset_namespace()                   # clear namespace="session" variables
```

`target` accepts an `Instance`, its key, a PID, an open .blend path, `"background"` or `"background:<version>"`. Session targets live in memory and clear when Locus restarts. Background instances belong to the checkout; at most 2 run at once (Settings > Blender) and idle ones close after 10 minutes. A running call keeps its instance alive.

## Versions and add-on

```python
await bl.installations()                 # paths, versions, add-on status
await bl.workspace_version()             # pinned or inferred version and `.blend` version counts
await bl.set_workspace_version("4.2")    # pin (None clears)
await bl.file_info("art/hero.blend")     # saved version and which installations can open it
await bl.install_addon()                 # default installation; all=True for every one
await bl.reload_hosts()                  # running instances load the current host when idle
```

Launches pick the installation in this order: explicit `installation`/`version`, the file's saved version, the workspace version, the default installation, the newest supported (4.2+). A 5.x file needs Blender 4.5+; opening a newer file in an older Blender needs `allow_downgrade=True`. Results carry the instance version; check `blender.api("bpy.types.X")` before using API that changed between 4.2 and 5.x. Never save unless the user asked.

## Helpers

Same names and arguments as `blender.<helper>` inside Blender, plus `target=`:

```python
img = await bl.look(mode="angles", views=["front", "iso"], shading="material", max_long_edge=1024)
locus.attach_image(img[0])                 # at most 4 images per call
print(await bl.scene_summary(query="Light"))
print(await bl.object_detail("Cube"))
print(await bl.file_report(path="art/prop.blend"))   # background instance opens the file first
print(await bl.api("bpy.types.Object.modifiers"))
```

`look` modes: `viewport` (interactive only), `camera`, `angles` (up to 6 directions), `frames`; shading `solid`, `material`, `rendered`, `wireframe`. It restores view, shading, frame and render settings and never adds an undo step.

`locus.attach_image(image, label=None)` takes a `locus.Image`, a file path or bytes (PNG, JPEG, WebP up to 20 MiB) and adds it to the current `python` result; in background tasks the images are saved as files and the result lists their paths.

## Undo

Non-readonly calls in an interactive instance add undo steps named `Locus: ...`; all calls of one `python` tool call to one instance merge into a single step pushed when the tool call ends. A `python` call with `readonly=true` makes every Blender call readonly.
