---
tools:
  - view_list
  - view_create
  - view_reload
  - view_run
  - view_compile_script
  - view_call_script
  - view_binding_read
  - view_binding_discover
  - view_binding_write
  - view_binding_apply
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
   - Main UI: `src/App.vue`
   - Entry: `src/main.ts`
   - Shared state: `src/store.ts`
   - Styles: `src/style.css`
   - Bindings: `bindings.json`
   - Optional Unity script: `unity/ViewApi.cs`

4. Use the public View Runtime SDK from package frontend code.

```ts
import {
  view,
  GraphView,
  GraphViewController,
  CanvasView,
  defineGraphView,
  useViewScript,
  onEditorUpdate,
  useUnityBinding,
} from "@locus/view-runtime";
```

5. Use the right runtime path for Unity data.
   - Direct `SerializedProperty` fields: `view.binding.read/write/apply` or `useUnityBinding`.
   - Unknown property paths: `view_binding_discover` before hardcoding paths.
   - Custom Unity logic: `view.callScript` from package code or `view_compile_script` / `view_call_script` from the agent.
   - Selection-driven panels: `onEditorUpdate(handler)`.
   - LLM-assisted semantic editors: `view.session` and `view.llm`.

6. Keep the UI aligned with Locus / Unity Editor tool style.
   - Prefer panels, split panes, inspectors, tables, trees, toolbars, and workspaces.
   - Keep controls compact, neutral, and useful for long editing sessions.
   - Use existing tokens for surfaces, borders, text, hover states, and accent color.
   - Use state badges only for strong statuses such as running, error, modified, enabled, or disabled.
   - Avoid marketing-style hero areas, decorative gradients, heavy shadows, oversized cards, colorful chip clusters, and continuous animation.

7. Validate package paths and reload.
   - Use package-relative paths with forward slashes.
   - Do not write absolute Unity project paths into `view.json`.
   - Use `view_reload` after edits.
   - Use `view_run` to open the View host.

For visual inspection, DOM interaction, frontend log reading, or live debugging of an open View host, read `skill/view/debug.md`. That document loads the View debugging tools only when needed.

Report the result with the View id, package root, template used, files changed, and reload or run result.
