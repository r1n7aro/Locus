# Composing a View

Build a View by importing native components from `@locus/components`. `await locus.views.components()` lists the available names. Use component props, slots and events to supply the View's data and behavior. The components own their controls, interaction and theme; the View owns data loading, saving and domain rules.

| Need | Reuse |
| --- | --- |
| Actions, choices and flags | `BaseButton`, `BaseDropdown`, `BaseSegmented`, `BaseCheckbox`, `BaseSwitch` |
| Movable custom blocks and field boards | `CanvasView` with an `item` slot |
| Nodes, ports, connections and graph layout | `GraphView` and `defineGraphView` |
| Source-to-target mappings | `LinkBoard` |
| Serialized property tables | `SerializedTableView` |
| A Unity object or inspector | `UnityObjectPreview`, `UnitySerializedPropertyTree`, or a bound tree's `drawDefaultEditor()` |
| Individual properties and custom drawers | `UnityPropertyDraw`, `UnityPropertyEditor`, `UnityNumberField`, `UnityObjectReferenceField` and the other Unity field components |
| Unity reference drag/drop | `UnityReferenceChip`, `UnityDropZone` |

Keep the View source small. Compose existing controls before writing another canvas, table, property editor or button style. Use `BaseCheckbox` for configuration flags and `BaseSwitch` for immediate state changes. Put saving, status, toolbars and additional panels in the View only when that tool needs them.

## Link mappings

```vue
<script setup lang="ts">
import { ref } from "vue";
import { LinkBoard, BaseButton } from "@locus/components";
import { view } from "@locus/view-runtime";
const sources = [{ id: "color", label: "Color" }, { id: "normal", label: "Normal" }];
const targets = [{ id: "base-map", label: "Base Map" }, { id: "normal-map", label: "Normal Map" }];
const links = ref<Array<{ source: string; target: string }>>([]);
async function save() { await view.storage.set("mapping", links.value); }
</script>
<template>
  <LinkBoard v-model="links" :sources="sources" :targets="targets" />
  <BaseButton @click="save">Save</BaseButton>
</template>
```

Endpoint shape: `{ id, label, disabled? }`. Connection shape: `{ source, target }`. Select a source, then a target to connect; select the same pair again to unlink. With no selected source, clicking a linked target unlinks it. Escape cancels the pending source. Native button keyboard behavior supports Enter/Space.

Props: `sources`, `targets`, `modelValue`, optional `sourceTitle`, `targetTitle`, `readonly`, `multiple`. Default mapping is one-to-one; `multiple` allows several connections per endpoint. The `update:modelValue` event returns a new array and does not modify input data. `source` and `target` slots receive `{ item, linked }`; the source slot also receives `selected`. Parent data changes and element resizing update the connection lines. No storage key, data provider or business rule is built in.

## Freeform boards

```vue
<script setup lang="ts">
import { ref } from "vue";
import { CanvasView } from "@locus/components";
const blocks = ref([{ id: "details", x: 40, y: 40, width: 240, height: 120, title: "Details" }]);
const selected = ref<string[]>([]);
</script>
<template>
  <CanvasView :items="blocks" v-model:selected-item-ids="selected">
    <template #item="{ item }"><section>{{ item.title }}</section></template>
  </CanvasView>
</template>
```

`CanvasView` supplies pan/zoom, moving, selection and hit testing. Its `editBehavior` prop controls permissions; `copySelection`, `pasteSelection`, `deleteSelection` and `contextMenu` events let the View implement its document rules. Compose `UnityPropertyDraw` or other native controls in the item slot for field boards. Give the canvas a container with a defined height. Read `app/view-runtime/src/components/canvas/canvasTypes.ts` for event shapes and the exposed `fitContent()` API.

## Node graphs

```ts
import { GraphView } from "@locus/components";
import { defineGraphView, view } from "@locus/view-runtime";
import type { GraphData } from "@locus/view-runtime";
const graph = defineGraphView({
  loadGraph: async (): Promise<GraphData> => (await view.storage.get("graph") as GraphData | null) ?? { schema: "locus.graph.v1", nodes: [], connections: [] },
  saveGraph: async (data: GraphData) => view.storage.set("graph", data),
});
// Render <GraphView :controller="graph" />.
```

Controllers can provide `createNode`, `validateConnection`, `applyGraph` and `onGraphChange`. Graph interaction and layout remain in the component. Use the actual `GraphData`/`GraphNode`/`GraphPort` types in `app/view-runtime/src/components/graph/graphTypes.ts` rather than inventing another graph format.

## Unity properties and tables

Use `property.fromPath(...)` to bind a Unity object or property. A bound tree's `drawDefaultEditor()` and an individual property's `draw()` already connect the editor to write-back; no wrapper C# script is required. `UnitySerializedPropertyTree` and `UnityPropertyDraw` also accept custom property drawers. `UnityObjectPreview` takes a `model` and can show an object, its identity, preview and inspector.

`SerializedTableView` takes `columns`, `rows`, `loading`, `error`, `status`, `progress`, `savingCellKey` and `columnWidths`. Handle `commit` to write and refresh a cell; bind `columnWidths` with `v-model` if the View should retain column sizing. Use `resolveSerializedTableSources` and the source helpers from `@locus/view-runtime` when aggregating asset sources. Data adapters can use the shared asset/property APIs; UI rendering does not require a generated C# script or a particular source provider.

For exact props and data shapes, read the relevant exported component source under `app/view-runtime/src/components/`. More services and field components are described in `runtime-api.md`.
