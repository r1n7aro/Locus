---
tools:
  - view_list
  - view_create
  - view_reload
  - view_run
  - view_compile_script
  - view_call_script
  - view_property_read
  - view_property_discover
  - view_property_write
  - view_property_apply
---

# View

## L1
Use when the user asks to build or edit a Locus View (视图/面板): a Vue UI panel, inspector, table, board, or graph editor for the connected Unity project. Ignore Unity project View classes, folders, assets, and camera/runtime views.

## Instructions

Use this workflow when the user asks for a Locus View or a frontend-built Unity editor interface.

1. Resolve the target View.
   - Use `view_list` first when a matching View may already exist. To change an existing View, reuse its `packageRoot` and continue from step 3.
   - For a new View, choose the template:
     - `blank` for custom panels and early prototypes.
     - `inspector-form` for field-oriented Unity asset, GameObject, Component, Material, or ScriptableObject editing.
     - `canvas-board` for draggable custom blocks.
     - `field-blocks` for block-based Unity `SerializedProperty` editors.
     - `node-graph` for graph editors with nodes, parameters, links, ports, layout, and scripted write-back.
     - `link-board` for source-to-target mapping interfaces.
     - `serialized-table` for table-based aggregation and editing of `SerializedProperty` data.

2. Create the package with `view_create`.
   - Keep `id` lowercase kebab-case and the display name concise. Pick `icon` from the tool enum, or omit it for the template default.
   - Set `temporary: true` for one-off display Views. They are written under the app temp directory, stay out of `Locus/View` and `view_list`, and the requested id gains a unique suffix — use the returned id everywhere afterwards.
   - `displayPath` only changes the user-visible View tree path. `packageName` picks the workspace folder under `Locus/View` and defaults to the Unity project name.
   - Unity-capable templates (`inspector-form`, `field-blocks`, `node-graph`, `serialized-table`) need a connected Unity editor when the View runs.

3. Edit only inside the returned `packageRoot`.
   - Manifest `view.json` owns `id`, `name`, `template`, `apiVersion`, entry/style paths, scripts, capabilities, and requirements. Keep every path package-relative with forward slashes; never write absolute Unity project paths.
   - Main UI: `src/App.vue`. Entry: `src/main.ts`. Styles: `src/style.css` (template styles extend a shared base block — keep it and append rules). Optional Unity script: `unity/ViewApi.cs`. Additional modules (for example a `src/store.ts` for shared state) may be created under `packageRoot` as needed.
   - Code shared across Views in the same workspace lives in the workspace `src/` and is imported as `@locus/project-view`.

4. Resolve API details through the stable View contract, in this order:
   - `runtime-api.md` in this skill package: the quick reference for `@locus/view-runtime` services (Unity property editing, drawers, drag/drop, graph/canvas, session, LLM, storage, fs, logs) and `@locus/components` components. Load it with `knowledge_read` path `skill/view/runtime-api.md`.
   - Exported runtime sources under this skill package's `app/view-runtime/src/`: exact component props, graph/canvas data shapes, and release behavior.
   - Locus application sources: only present in a development checkout. Installed releases do not ship them, so never depend on them from package code or instructions.

   Typical starting imports:

```ts
import { view, unity, property, onEditorUpdate } from "@locus/view-runtime";
import { UnitySerializedPropertyTree, GraphView } from "@locus/components";
```

5. Use the right runtime path for Unity data.
   - `SerializedProperty` editing from package code: `property.fromPath("asset/<assetPath>/property/<propertyPath>")` — also `selection/…`, `guid/<assetGuid>/…`, `scene/…`, and `prefab/…` path forms — then `tree.drawDefaultEditor()`, `tree.require(path).draw()`, `property.write(target, value)`, or batched `property.apply([...])`.
   - Unknown property paths: `view_property_discover` before hardcoding any path. Agent-side spot checks and one-off fixes: `view_property_read`, `view_property_write`, `view_property_apply`.
   - Custom property rendering: `propertyDrawer.registerValue/registerField/registerAttribute/registerPropertyPath/register`, passed as `propertyDrawers` into `UnitySerializedPropertyTree`, `UnityPropertyDraw`, or `UnityObjectPreview`. Whole-object rendering: `unityObjectDrawer.register(...)`, passed as `objectDrawers` into `UnityObjectPreview`.
   - Custom Unity logic: declare the C# file in `view.json` `scripts[]`, then call it with `view.callScript` from package code or `view_compile_script` + `view_call_script` from the agent.
   - Selection-driven panels: `onEditorUpdate(handler)`. Unity selection and inspectors: `unity.select(...)`, `unity.inspect(...)`, `unity.selectAsset(...)`, `unity.selectSceneObject(...)`.
   - Locus <-> Unity drag and drop: `useUnityReferenceDrag`, `useUnityAssetDropTarget`, `UnityReferenceChip`, `UnityDropZone`.
   - LLM-assisted semantic editors: `view.session` and `view.llm`.

6. Keep the UI aligned with Locus / Unity Editor tool style.
   - Prefer panels, split panes, inspectors, tables, trees, toolbars, and workspaces.
   - Keep controls compact, neutral, and useful for long editing sessions. Use existing tokens for surfaces, borders, text, hover states, and accent color.
   - Use state badges only for strong statuses such as running, error, modified, enabled, or disabled.
   - Avoid marketing-style hero areas, decorative gradients, heavy shadows, oversized cards, colorful chip clusters, and continuous animation.

7. Validate, debug, and report.
   - Run `view_reload` after edits: it validates the manifest and refreshes an open host. Use `view_run` to open or focus the View host.
   - If the View fails to load or misbehaves, read `debug.md` with `knowledge_read` path `skill/view/debug.md`. That read activates the debugging tools (`view_capture`, `view_snapshot`, `view_action`, `view_wait`, `view_console_read`, `view_debug_eval`); plain file reads do not. Check `view_console_read` first for frontend runtime errors.
   - When the user-facing reply should reference the finished View, put a standalone line in this exact format: `view:<view-id>`, using the id returned by `view_create`, `view_list`, or `view_reload`. The Locus frontend renders that line as a View reference block with an Open View button.
   - Report the View id, `packageRoot`, template used, files changed, reload or run result, and the standalone `view:<view-id>` reference line.
