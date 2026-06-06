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

## Summary
Create and maintain Locus Views: desktop-style Unity editor interfaces rendered in the dedicated View host.

## Instructions

Use this workflow when the user asks for a Locus View or frontend-built Unity editor interface.

1. Choose the template:
   - `blank` for custom panels and early prototypes.
   - `inspector-form` for field-oriented Unity asset, GameObject, Component, Material, or ScriptableObject editing.
   - `canvas-board` for draggable custom blocks.
   - `field-blocks` for block-based Unity `SerializedProperty` editors.
   - `node-graph` for graph editors with nodes, parameters, links, ports, layout, and scripted write-back.
   - `link-board` for source-to-target mapping interfaces.
   - `serialized-table` for table-based aggregation and editing of `SerializedProperty` data.

2. Create or locate the package.
   - Use `view_list` first when a matching View may already exist.
   - Use `view_create` for a new package.
   - Set `temporary: true` for one-off display Views that should stay out of `Locus/View`.
   - Keep `id` lowercase kebab-case.
   - Use concise display names.
   - Choose `icon` from the Locus icon library when creating a package. Common choices: `View`, `InspectionPanel`, `TableProperties`, `Network`, `Link2`, `Workflow`, `Kanban`, `Grid2X2`, `Layers`, `Package`, `Box`, `Braces`, `FileCode2`, `Puzzle`, `ScanSearch`.

3. Edit only inside the returned `packageRoot`.
   - Manifest: `view.json` owns `id`, `name`, `template`, `apiVersion`, entry/style paths, scripts, capabilities, and requirements.
   - Main UI: `src/App.vue`
   - Entry: `src/main.ts`
   - Shared state: `src/store.ts`
   - Styles: `src/style.css`
   - Optional Unity script: `unity/ViewApi.cs`
   - Additional package files and modules may be created under `packageRoot` as needed.

4. Use the public View Runtime SDK from package frontend code.

```ts
import {
  view,
  GraphViewController,
  defineGraphView,
  unity,
  useViewScript,
  onEditorUpdate,
  useUnityReferenceDrag,
  useUnityAssetDropTarget,
  property,
  propertyDrawer,
  unityObjectDrawer,
} from "@locus/view-runtime";
import {
  GraphView,
  CanvasView,
  UnityReferenceChip,
  UnityDropZone,
  UnityObjectPreview,
  UnityPropertyDraw,
  UnityPropertyEditor,
  UnitySerializedPropertyTree,
} from "@locus/components";
```

5. Resolve API details through the stable View contract.
   - Read `runtime-api.md` first for the common `@locus/view-runtime`, `@locus/components`, Unity property, drawer, drag/drop, session, LLM, storage, log, and tool APIs.
   - Reading Locus implementation files in a development checkout is normal when the public contract is insufficient for diagnosis.
   - Installed releases expose the created View Package source under `packageRoot`, the bundled View skill files, exported View Runtime sources under this skill package's `app/view-runtime/`, and the `view_*` tools. The Locus application source tree is not part of the selected Unity workspace.
   - Prefer the exported files under `app/view-runtime/src/` for runtime APIs, component props, graph/canvas data shapes, and release behavior before searching private app implementation files.
   - Treat `@locus/view-runtime` as the View SDK for services, Unity editing, drawers, drag/drop, graph/canvas helpers, session, LLM, storage, and logs.
   - Treat `@locus/components` as the component module for Base controls, Canvas/Graph views, Unity property editors, object previews, and drag/drop display components.

6. Use the right runtime path for Unity data.
   - Unity `SerializedProperty` editing: `property.fromPath("asset/Assets/Data/Config.asset/property/m_Name")`, `property.fromPath("guid/<asset-guid>/property/m_Name")`, `property.readProperty(...)`, `property.write(...)`, or `property.apply(...)`.
   - Property tree rendering: prefer `tree.drawDefaultEditor()`, `tree.root?.draw()`, or `tree.require(path).draw()`; pass `tree.snapshots` or a property snapshot into `UnitySerializedPropertyTree` when using the component directly.
   - Direct field commits: call `property.write(target, value)` with a string path or a `{ kind, path, propertyPath }` target.
   - Custom property rendering: `propertyDrawer.registerValue/registerField/registerAttribute/registerPropertyPath/register(...)`; pass `propertyDrawers` into `UnitySerializedPropertyTree`, `UnityPropertyDraw`, or `UnityObjectPreview`.
   - Custom Unity object or asset rendering: `unityObjectDrawer.register(...)`; pass `objectDrawers` into `UnityObjectPreview` when the override should be local to a View.
   - Unknown property paths: `view_property_discover` before hardcoding paths.
   - Custom Unity logic: `view.callScript` from package code or `view_compile_script` / `view_call_script` from the agent.
   - Selection-driven panels: `onEditorUpdate(handler)`.
   - Unity selection and inspectors: `unity.select(...)`, `unity.inspect(...)`, `unity.selectAsset(...)`, `unity.selectSceneObject(...)`.
   - Locus <-> Unity drag and drop: `useUnityReferenceDrag`, `useUnityAssetDropTarget`, `UnityReferenceChip`, and `UnityDropZone`.
   - LLM-assisted semantic editors: `view.session` and `view.llm`.

7. Keep the UI aligned with Locus / Unity Editor tool style.
   - Prefer panels, split panes, inspectors, tables, trees, toolbars, and workspaces.
   - Keep controls compact, neutral, and useful for long editing sessions.
   - Use existing tokens for surfaces, borders, text, hover states, and accent color.
   - Use state badges only for strong statuses such as running, error, modified, enabled, or disabled.
   - Avoid marketing-style hero areas, decorative gradients, heavy shadows, oversized cards, colorful chip clusters, and continuous animation.

8. Validate package paths and reload.
   - Use package-relative paths with forward slashes.
   - Do not write absolute Unity project paths into `view.json`.
   - Use `view_reload` after edits.
   - Use `view_run` to open the View host.
   - When a user-facing reply should reference the finished View, put a standalone line in this exact format: `view:<view-id>`. Use the package `id` from `view_create`, `view_list`, or `view_reload`. The Locus frontend renders that line as a View reference block with an Open View button.

For visual inspection, DOM interaction, frontend log reading, or live debugging of an open View host, read `debug.md`. That document loads the View debugging tools only when needed.

Report the result with the View id, package root, template used, files changed, reload or run result, and the standalone `view:<view-id>` reference line.
