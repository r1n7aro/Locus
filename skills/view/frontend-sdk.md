---
title: Locus Frontend TypeScript SDK
tools:
  - execute_typescript
---

# Locus Frontend SDK

Use `execute_typescript` for all View authoring, live frontend operations and debugging. Its `code` argument is a TypeScript async function body; `locus` is provided. `import { locus } from "@locus/frontend"` is also supported. Return JSON-compatible data. Each execution has a bounded lifetime; do not create persistent listeners or background loops in tool code.

The SDK also runs inside native View packages. There, import `locus` from `@locus/frontend`; it is bound to the View editor's checkout and lifecycle. View code uses the current Locus Vue application, native components, Pinia, theme and scoped services. Do not mount another WebView, install a second Pinia, replace native window globals or load Vue from a CDN.

## Workbench and application

```typescript
return locus.workbench.tabs();
// Each tab has editorId, paneId, title, kind, active and checkoutId.
```

```typescript
await locus.workbench.activate("editor-id");
return await locus.ui.snapshot({ maxElements: 80 });
```

`locus.workbench.close(editorId)` follows native close/unsaved-change behavior. `locus.logs.read(limit?)` returns native debug-console entries. `locus.ui` operates on the current Locus window. Use an existing `windowLabel` tool argument to target a detached Workbench window.

## View packages

```typescript
return await locus.views.list();
```

```typescript
const created = await locus.views.create({
  fileName: "asset-editor.vue",
  component: `<script setup lang="ts">
import { ref } from "vue";
import { BaseButton } from "@locus/components";
const count = ref(0);
</script>
<template><main class="panel"><BaseButton @click="count++">Count {{ count }}</BaseButton></main></template>
<style scoped>.panel { padding: 12px; color: var(--text-color); }</style>`,
});
const panel = await locus.views.open(created.manifest.id);
return { id: created.manifest.id, packageRoot: created.summary.packageRoot, entry: created.manifest.entry };
```

This creates one file, `asset-editor.vue`, inside an isolated View directory. No `view.json`, entry script, separate CSS or shared workspace scaffold is generated. File names use lowercase kebab-case. The id and default display name are the file name without `.vue`; `id` is optional with `fileName`. Supplying `id` alone with `component` creates `<id>.vue`. If both are supplied, they must match.

Metadata is optional. To customize it, put this JSON custom block **at the beginning of the Vue file**, before script/template/style blocks:

```vue
<view>
{ "name": "Asset Editor", "icon": "InspectionPanel", "unity": true }
</view>
```

The supported fields are `name`, `icon`, `displayPath`, `unity` and `scripts` (the existing `{ name, path, entryType }` declarations for optional files under `unity/`). All fields and the entire block may be omitted. `unity` controls both Unity capability and the connection requirement; its default is false, or true when scripts are declared. Locus derives schema, API version, version, entry and styles. Do not repeat them in the source. Renaming or moving a View in the tree updates only the relevant inline metadata and preserves the component code.

`create()` also accepts `packageName`, `name`, `icon`, `displayPath`, `unity`, `directories` and `temporary`. Explicit author options override the inline metadata. Temporary Views get a unique file name/id and remain outside `Locus/View` and `list()`; always use the returned id/entry. Runtime logs and storage remain managed under the View's internal `.locus/` directory and are excluded from exports.

To initialize a directory before writing the implementation, omit `component`:

```typescript
const created = await locus.views.create({ fileName: "asset-editor.vue", directories: ["src/components", "unity"] });
return { packageRoot: created.summary.packageRoot, entry: created.manifest.entry };
```

This writes an empty Vue component and creates the requested package-relative directories. No example page, independent stylesheet, C# implementation or workspace library is copied. Supplying `template` returns an error; the template catalog is removed.

`await locus.views.components()` lists native reusable components. Import them from `@locus/components`; see `components.md` for composition examples. `read(viewId)` returns the derived or legacy manifest and source files. `instances()` lists mounted instances in this window and the execution's checkout. Existing `view.json` packages remain readable, and ZIP import/export and plugin packaging continue to work. Extra source modules go under `src/`.

```typescript
const panel = await locus.views.open("asset-editor");
await panel.wait({ condition: "runtimeReady" });
return await panel.snapshot();
```

`open(viewId)` opens and activates a permanent Workbench tab in the addressed window and checkout. If that View already has a tab, it activates that tab without creating a duplicate; a preview tab becomes permanent. Existing tabs remain open. The returned handle stays bound to the selected tab instance.

`locus.views.get(viewIdOrInstanceId)` returns a handle for an open native panel. When the same View has multiple instances, use the instanceId from `instances()`. Handles expose activate(), reload(), snapshot(), wait(), locator(), getByRole(), getByText(), getById(), logs(limit?) and capture().

## UI actions

```typescript
const panel = locus.views.get("asset-editor");
await panel.locator('input[name="filter"]').fill("Material");
await panel.getByRole("button", "Refresh").click();
await panel.wait({ condition: "textPresent", text: "Material" });
return await panel.snapshot({ maxElements: 100 });
```

Locators support click(), doubleClick(), hover(), focus(), fill(value), type(text), press(key), check(checked?), select(value), scroll(deltaY, deltaX?) and dragTo(target). `getById(id)` uses an automation element id from a snapshot. Targets for dragTo/action can contain selector, text, role, name or id.

`wait()` supports runtimeReady, selectorVisible, selectorHidden, textPresent, textAbsent and noConsoleError, with timeoutMs and pollIntervalMs. Snapshots include status, frame, focus and actionable elements. Use `panel` operations to keep interaction inside a View; use `locus.ui` to operate the native Locus interface itself.

## Screenshots and logs

```typescript
const panel = locus.views.get("asset-editor");
await panel.capture();
return await panel.logs(20);
```

`panel.capture()` activates the panel and captures its bounds from the existing WebView. `locus.ui.capture()` captures the current Locus window. The tool attaches PNG images; do not print base64. At most four screenshots may be attached per execution.

## Editing and hot updates in shipped Locus

```typescript
const packageInfo = await locus.views.read("asset-editor");
const path = packageInfo.summary.packageRoot + "/" + packageInfo.manifest.entry;
const source = await locus.fs.read(path);
// Inspect the actual source before changing it; write the intended new source.
await locus.fs.write(path, source.replace("old label", "new label"));
await locus.views.get("asset-editor").reload();
return await locus.views.get("asset-editor").snapshot();
```

The application includes the compilation Worker. No Vite dev server, Node installation or Locus rebuild is needed. Changes made by the native file editor or external writers are also observed. CSS-only edits preserve the actual component DOM. Component/script edits rebuild the affected View; `useViewState(initial, stableKey)` retains explicit editor state across hot updates. Compilation errors leave the previous usable View in place.

Use scoped component CSS and the native tokens. Global selectors in package styles are bounded to the package's View root. Keep heavy work out of the frontend thread. Register persistent View work through `useViewContext().onDispose()` or the SDK's managed subscriptions; tool executions are temporary.

## Unity

`locus.assets` provides one API for serialized asset edits through either Rust YAML or Unity Editor live. Backend contexts retain the View's checkout and lifecycle; selecting a backend does not change another context. YAML is the default and never starts Unity. With a connected Editor, YAML edits still run in Rust and use one coordinated Editor batch to replace/import files; dirty target assets are rejected and unrelated dirty state is preserved. Live requires an already connected, ready Editor and saves successful edits to disk.

```typescript
const assets = locus.assets.backend("yaml"); // Or "live", with identical calls.
const path = "Assets/Config.asset";
const snapshot = await assets.read(path);
const operations = [{
  op: "set" as const,
  object_id: "11400000",
  property_path: "/MonoBehaviour/speed",
  value: 12.5,
}];
await assets.preview(path, operations, { expected_revision: snapshot.revision });
return await assets.apply(path, operations, { expected_revision: snapshot.revision });
```

Snapshots contain `revision`, `objects` and `diagnostics`. Object entries contain exact string `object_id`, `class_id`, `root_type` and flat `fields`; each field has `property_path`, `kind`, and `value`. Use the returned root-inclusive RFC 6901 pointers, including any stable `@rid=...` path selectors. `read(path,{object_id?,property_path?})` filters the snapshot. `discover(path,{query?,object_id?,property_path?,offset?,limit?})` returns `revision`, `matches`, `total`, `truncated`, and `next_offset`.

Operations share `object_id` and `property_path`: `set` takes `value`; `array_insert` takes `index,value`; `array_remove` takes `index`; `array_move` takes `index,to_index`; `array_resize` takes `size` and requires a fill `value` when growing. Arrays never implicitly duplicate an item. Values are data, not embedded action commands. Reference maps use string IDs (`{fileID:"11400000",guid:"...",type:2}` or `{rid:"101"}`). Exact scalar integers use `assets.integer("9007199254740993")` or a `bigint`; the SDK encodes `{kind:"int64",value:"..."}`. Unsafe JavaScript numbers are rejected, including nested reference IDs.

For many files, `preview_batch(entries)` and `apply_batch(entries)` take `[{path,expected_revision,operations}]` in one backend transaction. Every apply entry requires its original snapshot revision; a preview's result revision cannot substitute for the input revision. Preview returns `applied:false,persisted:false`; a successful apply returns both as `true` with the resulting snapshot and operation count. Batch responses contain `results`. Stale revisions fail before mutation. The backend does not change automatically. `assets.capabilities()` lists supported operations/extensions. Binary/imported assets, runtime-only objects, arbitrary C# type creation and Prefab inherited-value projection are not implied by the common API.

Batch transactions validate first and roll back failures; multiple filesystem replacements are not a single instantaneous filesystem operation. `await assets.recover(transaction_id)` recovers an interrupted YAML transaction from its journal and refuses to overwrite later edits.

For unsigned 64-bit scalar values, use `assets.unsignedInteger("18446744073709551615")`, which returns `{kind:"uint64",value:"..."}`. Both integer tags can be resubmitted directly from snapshots, including inside whole-array `set` values. `assets.integer(...)` and all object/fileID/rid identities retain signed 64-bit limits.

Cancellation prevents new SDK requests after disposal. It does not roll back an already dispatched transaction: after a timeout, read current state before retrying array edits. `locus.unity.property` defaults to the Editor API; View authors can select an independent YAML instance once with `locus.unity.property.backend("yaml")`. Its trees and batches inherit that backend. Target IDs accept exact decimal strings and reject unsafe numeric IDs.

```typescript
const properties = locus.unity.property.backend("yaml"); // "live" selects Unity's main-thread API
const current = await properties.read({ target: {
  kind: "asset", path: "Assets/Data.asset", targetFileId: "11400000", propertyPath: "amount",
} });
const batch = properties.batch();
batch.enqueue({ target: current.target, value: 42, expectedRevision: current.revision });
// Enqueue more edits; no disk writes or Unity calls until flush.
await batch.flush();
```

`readTree(target)` returns the existing bound Property Tree and keeps YAML numeric
drag previews local. YAML operates on serialized fields, with explicit path/fileID
identity and revision checks. Text Prefab effective values, scalar Override/Revert,
cross-layer Apply, explicit managed templates and materialized topology templates
are available through the YAML API. `discover()` returns exact inherited targets;
carry `read.dependencies` as `expectedDependencies` on inherited writes. Runtime
selection and rich Editor metadata still require `live`. See
`runtime-api.md` → Property backend and deferred writes for capability boundaries.
For large raw asset jobs, `assets.batch().enqueue(path, operations, {expected_revision})`
and `flush()` avoid property conversion. Both batch APIs preserve operation order,
share concurrent flushes and block automatic retries after an uncertain outcome.

Logical property paths, including existing managed-reference children such as
`node.next.amount`, are resolved in the shared Rust semantic layer. The same
normalized values and reference identities feed Agent YAML trees and View trees.
When a batch reassigns a reference or moves an array item, later writes resolve
against the updated graph. No frontend YAML projection or silent live fallback is used.

```typescript
const yaml = locus.unity.property.backend("yaml");
const found = await yaml.discover({ target: { kind: "asset", path: "Assets/Variant.prefab" }, query: "amount" });
const before = await yaml.read({ target: found.matches[0]!.target! });
await yaml.write({ target: before.target, value: { action: "revert" },
  expectedRevision: before.revision, expectedDependencies: before.dependencies });
```

```typescript
return await locus.views.callScript({ viewId: "asset-editor", scriptName: "InspectorViewApi", method: "Read", args: {} });
```

`locus.views.compileScript({ viewId, scriptName })` explicitly compiles a manifest-declared Unity script. `locus.unity.property.read/discover/write/apply` use the same typed request structures as the native frontend's serialized-property service, bound to the execution's checkout.

The authoritative TypeScript implementation is included in the runtime source export at `app/view-runtime/src/services/frontendSdk.ts`; native View APIs and components remain available through `@locus/view-runtime` and `@locus/components`.
