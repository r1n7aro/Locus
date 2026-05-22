---
id: kd_skill_builtin_view
type: skill
path: builtin/view.md
title: View Package
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /view
argumentHint: <view requirement>
createdAt: 1779350400000
updatedAt: 1779350400000
---

# View Package

## Summary
Create, edit, reload, and open Locus View Packages for Unity editor interfaces through the dedicated View tools.

## Content
## Instructions

Use this workflow when the user asks for a Locus View, View Package, or frontend-built Unity editor interface.

1. Choose the template:
   - `blank` for custom panels and early prototypes.
   - `inspector-form` for field-oriented Unity asset, GameObject, Component, Material, or ScriptableObject editing.
   - `node-graph` for built-in graph editors with nodes, parameters, node links, and port links.
   - `link-board` for source-to-target mapping interfaces.

2. Load View tools with `tool_load`:
   - `view_list`
   - `view_create`
   - `view_reload`
   - `view_run`
   - `view_compile_script`
   - `view_call_script`
   - `view_binding_read`
   - `view_binding_write`
   - `view_binding_apply`

3. Create or locate the package.
   - Use `view_list` first when a matching View may already exist.
   - Use `view_create` for a new package.
   - Keep `id` lowercase kebab-case.
   - Use concise display names.
   - Choose `icon` from the Locus icon library when creating a package. Common choices: `View`, `InspectionPanel`, `TableProperties`, `Network`, `Link2`, `Workflow`, `Kanban`, `Grid2X2`, `Layers`, `Package`, `Box`, `Braces`, `FileCode2`, `Puzzle`, `ScanSearch`.

4. Edit only inside the returned `packageRoot`.
   - Use normal file tools to read and edit package files.
   - Main UI: `src/App.vue`
   - Entry: `src/main.ts`
   - Shared state: `src/store.ts`
   - Styles: `src/style.css`
   - Bindings: `bindings.json`
   - Optional Unity script: `unity/ViewApi.cs`

5. Use the public View Runtime SDK from package frontend code.
   - Import runtime helpers from `@locus/view-runtime` inside `src/App.vue`, `src/main.ts`, `src/store.ts`, or package-local modules.
   - Available public helpers:

```ts
import {
  view,
  GraphView,
  GraphViewController,
  defineGraphView,
  useViewScript,
  onEditorUpdate,
  useUnityBinding,
} from "@locus/view-runtime";
```

   - For `node-graph`, import `GraphView`, extend `GraphViewController`, and override graph lifecycle methods. The built-in runtime owns graph styling, pan/zoom, node dragging, selection, parameter controls, node links, port links, and automatic layout. Nodes can omit `x` / `y`; the shared graph component will place them.

```vue
<script setup lang="ts">
import { GraphView, GraphViewController, defineGraphView } from "@locus/view-runtime";

class MaterialGraphView extends GraphViewController {
  loadGraph() {
    return {
      layout: { auto: "missing", direction: "right" },
      nodes: [
        {
          id: "texture",
          title: "Texture",
          outputs: [{ id: "color", label: "Color", type: "Color" }],
          parameters: [{ id: "path", label: "Path", type: "string", value: "_BaseMap" }],
        },
        {
          id: "output",
          title: "Output",
          inputs: [{ id: "base", label: "Base", type: "Color" }],
        },
      ],
      connections: [
        { from: { nodeId: "texture", portId: "color" }, to: { nodeId: "output", portId: "base" } },
      ],
    };
  }

  saveGraph(graph) {
    console.info("Graph saved", graph);
  }
}

const graphView = defineGraphView(new MaterialGraphView());
</script>

<template>
  <GraphView :controller="graphView" title="Material Graph" />
</template>
```

   - `view.callScript(scriptName, method, args?)` invokes a `view.json` `scripts[]` entry and returns the script method result:

```ts
const model = await view.callScript("InspectorViewApi", "Read", {});
await view.callScript("InspectorViewApi", "Apply", { value: nextValue });
```

   - `useViewScript(scriptName)` creates a small script client with `call(method, args?)`:

```ts
const inspectorApi = useViewScript("InspectorViewApi");
const model = await inspectorApi.call("Read", {});
```

   - `onEditorUpdate(handler)` subscribes to Unity editor updates and returns an unsubscribe function. Use it for selection-driven panels:

```ts
import { onMounted, onUnmounted } from "vue";

let unsubscribeEditorUpdate: (() => void) | null = null;

onMounted(async () => {
  unsubscribeEditorUpdate = await onEditorUpdate((event) => {
    const selection = event.selection;
    const selectionKey = selection.instanceId
      ? `${selection.path}|${selection.instanceId}|${selection.type}`
      : "";
    void refreshForSelection(selectionKey);
  });
});

onUnmounted(() => {
  unsubscribeEditorUpdate?.();
  unsubscribeEditorUpdate = null;
});
```

   - Editor update event schema:

```ts
type ViewRuntimeUpdateEvent = {
  sequence: number;
  timeSinceStartup: number;
  isPlaying: boolean;
  isPaused: boolean;
  activeScenePath: string;
  selection: {
    kind: string;
    name: string;
    type: string;
    path: string;
    instanceId: number;
  };
};
```

   - `useUnityBinding(bindingIdOrRequest)` creates a binding client with `{ value, status, error, read, write }`.

```ts
const colorBinding = useUnityBinding("base-color");
await colorBinding.read();
colorBinding.value.value = "#d9dde5";
await colorBinding.write();
```

   - Concrete dynamic binding targets can be passed from TypeScript when the target depends on current UI state:

```ts
const nameBinding = useUnityBinding({
  target: {
    kind: "asset",
    path: selectedAssetPath.value,
    propertyPath: "m_Name",
  },
});
```

   - Use `view_binding_read`, `view_binding_write`, `view_binding_apply`, `view_compile_script`, and `view_call_script` tools for agent-side validation and scripted Unity work. Use `@locus/view-runtime` for package frontend code.
   - Treat Locus implementation files such as `src/components/view/viewRuntime.ts`, `src-tauri/src/view.rs`, and built `dist` assets as internal implementation details. Create View packages from the public SDK, View tools, and package files.

6. Keep the UI aligned with Locus desktop tool style.
   - Use panel, list, toolbar, and workspace structures.
   - Keep text short and operational.
   - Use state badges only for strong statuses such as running, error, or modified.

7. Choose the data write-back model before designing controls.
   - For direct `SerializedProperty` fields and straightforward JSON-compatible values, update through `view_binding_write` or `view_binding_apply` as the user edits, usually on change, blur, or a short debounce.
   - Show lightweight pending, saved, and error states near the edited control when useful.
   - Avoid manual Refresh, Save, or Apply controls for simple direct bindings.
   - Use explicit Refresh, Save, or Apply controls only for complex serialization, batch transforms, generated assets, scripted write-back, high-risk Unity operations, or flows that need diff, impact summary, or confirmation.
   - Treat `view_reload` as a package development reload after package files change, not as the normal way for end users to persist simple View edits.

8. Validate package paths and reload.
   - Use package-relative paths with forward slashes.
   - Do not write absolute Unity project paths into `view.json`.
   - Use `view_reload` after edits.
   - Use `view_run` to open the View host.
   - Use `view_compile_script` and `view_call_script` when the package has a manifest `scripts[]` entry.
   - Use `view_binding_read`, `view_binding_write`, and `view_binding_apply` for SerializedProperty bindings.

9. Report the result with:
   - View id and package root.
   - Template used.
   - Files changed.
   - Reload or run result.
