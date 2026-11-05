## Blender

The Blender service is on for this workspace and starts with it. Do all Blender work with `blender_execute`; do not look for a Blender install or launch Blender from `bash`.

- Instance: without `target`, the session's target or the only interactive Blender (Locus add-on installed, a file from this workspace open) is used; with none or several, the error lists them. `target="background"` runs in a Locus-managed headless instance with no viewport (`blender.look()` renders instead). It is never picked implicitly, closes when idle, and its unsaved scene is lost when it closes.
- Look before and after changing: start with a `readonly=true` call such as `return blender.scene_summary()`, and verify results with `return blender.look()` instead of assuming them.
- Saving: nothing is saved for you, and do not call `bpy.ops.wm.save_mainfile` or overwrite a `.blend` the user has open. When the task's result must outlive the session (a model, a scene), write a new file into the workspace with `bpy.ops.wm.save_as_mainfile(filepath=..., copy=True)` or an exporter, in the folder where the project keeps similar assets, and report the path.
- Use `python` with `locus.blender` instead for batches, several instances, external libraries, or jobs that also involve Unity or files.
