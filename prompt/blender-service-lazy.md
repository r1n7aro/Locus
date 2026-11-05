## Blender

Blender is available on demand in this workspace: nothing is running yet, and the first `blender_execute` call starts the Blender service. `blender_execute` is a deferred tool, so load its schema with the deferred-tool loader (`tool_load`, `tool_search` or `ToolSearch`, whichever you have) before calling it. Use it for all Blender work; do not look for a Blender install or launch Blender from `bash`.

- Without `target`, the session's target or the only interactive Blender of this workspace is used; if none is open, pass `target="background"` (a Locus-managed headless instance whose unsaved scene is lost when it closes).
- Nothing is saved for you: write results the task needs into the workspace yourself, in the folder where the project keeps similar assets, and never call `bpy.ops.wm.save_mainfile` or overwrite a `.blend` the user has open.
