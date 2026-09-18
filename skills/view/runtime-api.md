# View Runtime API Quick Reference

Use this when building a Locus View package. Prefer `@locus/view-runtime` for services, Unity editing, drawers, drag/drop, graph/canvas helpers, session, LLM, storage, and logs. Import visual components from `@locus/components`.

## Imports

- `vue`: the native Vue runtime. New entries export a component; legacy `createApp(...).mount(...)` retains plugin/provide/component registrations in the View subtree.
- `@locus/view-runtime`: main service and helper SDK.
- `@locus/components`: native component module.
- `@locus/frontend`: exports `locus`, the shared Workbench, UI and View SDK.
- `pinia`: the native Pinia runtime; the View inherits the host instance.
- `node:fs/promises`, `fs/promises`: Promise-based filesystem APIs.
- `node:fs`, `fs`: filesystem APIs with `promises` and common callback forms.
- `node:path`, `path`: common path helpers.
- Relative package modules: `.ts`, `.vue`, `.js`, `.css`, plus extension-qualified files that the runtime compiler can execute.
- Shared View package modules: `@locus/project-view` and `@project-view`.

## Main SDK Values

`@locus/view-runtime` currently exposes these runtime values:

- `view`: `manifest`, `summary`, `reload`, `callScript`, `assets.search`, `logs.read/latest/open`, `session`, `llm`, `storage`, `fs`, `path`, `unity`, `files`, `undo`, `propertyDrawer`, `unityObjectDrawer`, `objectReferencePicker`, `openLog`, `onUpdate`.
- `session`: `create`, `show`, `display`, `load`, `activeRun`, `events`, `queueInput`, `chat`, `send`, `wait`, `onEvent`, `fork`, `forkFromMessage`, `list`, `listArchived`, `rename`, `archive`, `unarchive`, `delete`, `undo`, `rollback`.
- `llm`: `call`.
- `storage`: `get`, `set`, `remove`.
- `fs`: `readFile`, `writeFile`, `appendFile`, `mkdir`, `readdir`, `stat`, `lstat`, `access`, `unlink`, `rm`, `rename`, `copyFile`, `constants`.
- `path`: `join`, `resolve`, `normalize`, `dirname`, `basename`, `extname`, `relative`, `parse`, `format`, `isAbsolute`, `sep`, `delimiter`, `posix`, `win32`.
- `unity`: `callScript`, `checkConnection`, `connectionStatus`, `normalizeReference`, `sceneObjectTarget`, `selectAsset`, `inspectAsset`, `openAssetInspector`, `selectSceneObject`, `inspectSceneObject`, `openSceneObjectInspector`, `select`, `inspect`, `drag.start/arm/commitDrop/onDrop/onState`, `onDrop`, `onDragState`, `objectDrawer`, `objectReferencePicker`.
- `files`: `drag.start/arm/onDrop/onState`, `onDrop`, `onDragState`.
- `undo`: `state`, `record`, `undo`, `redo`, `clear`, `handleKeydown`, `isRunning`.
- `property`: `parsePath`, `objectTarget`, `write`, `apply`, `readTree`, `fromPath`, `readProperty`, `property`.
- `propertyDrawer`: `library`, `projectLibrary`, `register`, `registerValue`, `registerField`, `registerAttribute`, `registerPropertyPath`, `define`, `normalize`, `createLibrary`.
- `unityObjectDrawer`: `library`, `projectLibrary`, `register`, `define`, `normalize`, `createLibrary`, `resolve`.
- `objectReferencePicker`: `roots`, `searchQuery`, `filterResults`, `isResult`, `typeHint`, `typeKey`, `typeRule`, `normalizePath`, `extension`.
- Helpers: `defineView`, `useViewContext`, `useViewState(initial, stableKey?)`, `useViewScript`, `onEditorUpdate`, `useUnityReferenceDrag`, `useUnityAssetDropTarget`, `useLocusFileDrag`, `useLocusFileDropTarget`.
- Graph helpers: `GraphViewController`, `defineGraphView`, `layoutGraphDocument`.
- Serialized table helpers: `resolveSerializedTableSources`, `serializedTableSourcesFromAssets`, `normalizeSerializedTableSource`, `dedupeSerializedTableSources` (feed `SerializedTableView` from manual sources plus scripted providers).

Legacy `window.locus.view` and `window.locus.unity` resolve through an instance-local compatibility proxy. They do not overwrite the native window's globals. New code imports services and `locus` explicitly.

Some visual components are still available from `@locus/view-runtime` for compatibility. New View code should import them from `@locus/components`.

## Filesystem

Filesystem calls run through the Locus desktop bridge. Absolute paths are used directly. Relative paths resolve from the current Unity project root, matching the normal Node idea of a working directory.

```ts
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const shaderPath = path.join("Assets", "Shaders", "MyShader.shader");
const source = await readFile(shaderPath, "utf8");
await writeFile(shaderPath, source.replace("_Color", "_Tint"), "utf8");
```

`readFile(path, "utf8")` returns a string. `readFile(path)` returns a `Uint8Array` with `toString("utf8")` support. `readdir(path, { withFileTypes: true })` returns Dirent-like objects with `isFile()`, `isDirectory()`, and `isSymbolicLink()`.

## Property Paths

Use `property` for normal Unity `SerializedProperty` work:

```ts
const tree = await property.fromPath("asset/Assets/Data/Config.asset/property/m_Name");
const name = await property.readProperty("selection/property/m_Name");
await property.write("guid/<asset-guid>/property/m_Name", "Player");
await property.apply([
  { target: { kind: "asset", path: "Assets/Data/Config.asset", propertyPath: "m_Name" }, value: "Player" },
]);
```

Common string path forms:

- `selection/property/<propertyPath>`
- `asset/<assetPath>/property/<propertyPath>`
- `guid/<assetGuid>/property/<propertyPath>`
- `scene/<scenePath>/object/<objectPath>/property/<propertyPath>`
- `scene/<scenePath>/object/<objectPath>/component/<type>/<index>/property/<propertyPath>`
- `prefab/<prefabPath>/object/<objectPath>/component/<type>/<index>/property/<propertyPath>`

Bound property objects expose `write`, `preview`, `undo`, `redo`, `draw`, and `drawDefaultEditor`. Bound trees expose `root`, `properties`, `get`, `require`, `refresh`, `writeProperty`, `writeCommit`, `apply`, `undo`, `redo`, `drawDefaultEditor`, and `drawPropertyEditor`.

Bound trees also expose `loadChildren(property)` for truncated nodes. Array continuations use `arrayOffset` and a bounded page size, including arrays larger than 1024 elements. Resolved targets retain `globalObjectId`; preserve it when deriving another property path. `Long` and `UnsignedLong` values use exact decimal strings. Write responses provide authoritative `beforeSnapshot` and opaque `restoreState` data for history; do not reconstruct undo values from display text or truncated children. Native property editors embedded in a View share its editing workspace and undo history.

## Expanded Helper Exports

These are also available from `@locus/view-runtime` for custom renderers and advanced editors:

- Property tree: `InspectorProperty`, `PropertyTree`, `createPropertyTree`, `createInspectorPropertyTreeBinding`, `resolveInspectorDrawer`, `resolveManagedReferenceTypeOption`, `searchManagedReferenceTypeOptions`, `defineInspectorPropertyDrawers`, `createInspectorPropertyDrawerLibrary`, `publicInspectorPropertyDrawerLibrary`, `projectInspectorPropertyDrawerLibrary`, `normalizeInspectorPropertyDrawers`, `registerInspectorPropertyDrawer`, `registerInspectorValueDrawer`, `registerInspectorFieldDrawer`, `registerInspectorAttributeDrawer`, `registerInspectorPropertyPathDrawer`, `propertyTreeService`.
- Unity property binding: `UnityBoundProperty`, `UnityBoundPropertyTree`, `createUnityPropertyRuntime`, `unityBoundPropertySnapshots`.
- Unity value formatting: `normalizeUnityPropertyType`, `isUnityIntegerPropertyType`, `isUnityNumberPropertyType`, `isUnityVectorPropertyType`, `isUnityQuaternionPropertyType`, `unityVectorKeysForType`, `normalizeUnityOptions`, `unitySerializedValueToEditText`, `tryParseUnitySerializedEditValue`, `parseUnitySerializedEditValue`, `constrainUnityNumberValue`, `formatUnityNumberValue`, `formatUnityEnumValue`, `unityEnumIndexValue`, `unityEnumNumericValue`, `parseUnityVectorValue`, `formatUnityVectorValue`, `parseUnityQuaternionEulerValue`, `formatUnityQuaternionEulerValue`, `parseUnityColorValue`, `formatUnityColorValue`, `unityColorTextToRgbHex`, `applyUnityRgbHexToColorText`.
- Unity property target paths: `parseUnityPropertyPath`, `resolveUnityPropertyTarget`, `unityPropertyObjectTarget`, `unityPropertyTargetWithPath`, `unityPropertyTargetKey`.
- Object drawers: `defineUnityObjectDrawers`, `createUnityObjectDrawerLibrary`, `publicUnityObjectDrawerLibrary`, `projectUnityObjectDrawerLibrary`, `normalizeUnityObjectDrawers`, `resolveUnityObjectDrawer`, `registerUnityObjectDrawer`, `unityObjectDrawerService`.
- Object reference picker: `UNITY_OBJECT_REFERENCE_SEARCH_ROOTS`, `normalizeUnityObjectReferenceType`, `unityObjectReferenceTypeKey`, `unityObjectReferenceTypeHint`, `getUnityObjectReferenceTypeRule`, `unityObjectReferenceSearchQuery`, `normalizeUnityObjectReferencePath`, `unityObjectReferenceDisplayParts`, `unityObjectReferenceAssetKey`, `unityObjectReferenceValueForSearchResult`, `unityObjectReferenceExtension`, `isUnityObjectReferenceSearchResult`, `filterUnityObjectReferenceSearchResults`.

## Component Module

`@locus/components` exposes:

- `BaseButton`, `BaseCheckbox`, `BaseDropdown`, `BaseSegmented`, `BaseSwitch`.
- `CanvasView`, `GraphView`, `LinkBoard`, `SerializedTableView`.
- `UnityBoolField`, `UnityBoundsField`, `UnityColorField`, `UnityColorHdrField`, `UnityCurveField`, `UnityEnumField`, `UnityFlagsField`, `UnityGradientField`, `UnityLayerMaskField`, `UnityNumberField`, `UnityObjectReferenceField`, `UnityPropertyDraw`, `UnityPropertyEditor`, `UnitySerializedPropertyTree`, `UnityVectorField`.
- `UnityObjectPreview`, `UnityReferenceChip`, `UnityDropZone`.

`SerializedTableView` renders rows of Unity serialized cells with resizable columns, a progress/status bar, and per-cell editors. Props: `columns`, `rows`, `loading`, `status`, `error`, `progress`, `savingCellKey`, `sourceCount`, `columnWidths` (v-model). Events: `commit` (`SerializedTableCommitEvent`), `update:columnWidths`. The caller supplies data loading, commit handling and optional persistence.

`LinkBoard` renders source-to-target connections. Props: `sources`, `targets`, `modelValue` (v-model), optional `sourceTitle`, `targetTitle`, `readonly`, `multiple`. Endpoints are `{ id, label, disabled? }`; connections are `{ source, target }`. It handles selection, connection lines, resize and unlinking; the caller owns data and persistence. `components.md` contains composition examples for this and the existing editors. `locus.views.components()` lists the available native components.

`UnityCurveField` and `UnityGradientField` render AnimationCurve / Gradient previews; when given `editable` plus a `bindingTarget` (the property tree passes both automatically), clicking them opens the floating Locus value editor window, which owns its own preview/commit write-back and broadcasts `locus-value-editor:committed` on apply.

## Plugin Drawer Packages

Locus plugins can ship `drawers/<drawer-id>/` packages that extend the in-app Inspector/property rendering (chat property fences, the Locus Inspector window, diff panes, and Views) in every Locus window — unlike per-View `propertyDrawers` props, these apply app-wide.

- Manifest: `drawer.json` with `{ "id", "entry" }`; `entry` defaults to `src/index.ts`. Declared in `locus.plugin.json` under `components.drawers` (or discovered from the `drawers/` directory).
- The entry runs once per window against `@locus/drawer-runtime`, a deliberately small runtime: `meta` (`pluginId`, `pluginName`, `drawerId`), `components` (same map as `@locus/components`), `propertyDrawer` (`register`, `registerValue`, `registerField`, `registerAttribute`, `registerPropertyPath`, `define`), `unityObjectDrawer` (`register`, `registerExtension`, `define`). No fs/session/llm surface.
- Allowed imports: `vue`, `@locus/components`, `@locus/drawer-runtime`, and relative `.ts`/`.js`/`.vue`/`.css`/`.json` files inside the package.
- Resolution priority: explicit `propertyDrawers` props > View/project registrations (`propertyDrawer.register*` from View code) > plugin drawer packages > built-in editors. A drawer that throws is isolated per property row and falls back to a raw value display.
- Limits: at most 64 files and 512 KB per file per package; registrations are removed automatically when the plugin is disabled or uninstalled.

## Agent Tools

The View skill grants the single `execute_typescript` TypeScript tool. It exposes the shared native frontend SDK; see `frontend-sdk.md`.

Debugging uses the same SDK and tool as authoring. `debug.md` contains the inspection workflow.

## Usage Guidance

### Property backend and deferred writes

Select the backend once in View code, using the shared SDK. Controls and bound
trees inherit that selection; do not add mode selectors to individual controls.
Existing `property` calls continue to use Unity's main-thread API.

```ts
import { locus } from "@locus/frontend";

const properties = locus.unity.property.backend("yaml"); // "live" = Unity API (default)
const target = { kind: "asset", path: "Assets/Data.asset", targetFileId: "11400000" };
const tree = await properties.readTree(target);
// tree.drawDefaultEditor() reuses the existing Property Tree controls.
// Numeric drag preview stays local; committing a field persists its YAML.

const before = await properties.read({ target });
const batch = properties.batch();
batch.enqueue({ target: { ...before.target, propertyPath: "amount" }, value: 42,
  expectedRevision: before.revision });
batch.enqueue({ target: { ...before.target, propertyPath: "note" }, value: "updated",
  expectedRevision: before.revision });
await batch.flush();
await tree.refresh();
```

- `backend()` returns an independent instance; it does not switch existing trees
  or batches. Requests and controls have no backend parameter. `read`, `write`,
  `apply`, `readTree` and `batch` retain the instance's workspace and lifetime.
- YAML reads project persisted serialized values into Property Tree snapshots.
  They do not reproduce Editor-only attributes, enum choices, specialized
  Editor floating Curve/Gradient editors or all synthetic headers. Text Prefab
  effective values are resolved recursively through the shared dependency graph.
  Reference slots are displayed read-only; existing managed-reference children can
  be read and written through logical paths such as `node.next.amount`. Shared/cyclic
  objects retain host-scoped identity. Explicit API writes accept serialized
  `{fileID, guid, type}` / `{rid}` values. `property.discover({target,query})` returns
  effective matches with exact `target` IDs, including virtual Prefab objects.
  Raw serialized discovery remains available on `locus.assets.backend("yaml")`.
- YAML targets require an `Assets/` path and exact persisted `targetFileId` for
  multi-object assets. Use the returned `bindingTarget`; selection, runtime IDs,
  and hierarchy/component locators require `live`.
- YAML direct writes require `expectedRevision` from a YAML read. A bound tree
  manages its own revision. Prefab reads also return `dependencies` and `prefabLayers`;
  direct writes must include `expectedDependencies: read.dependencies`. Trees track
  source revisions automatically. Stale data is rejected; refresh before retrying.
  `writeMode` remains `commit/preview` for the live API and is independent of backend.
  YAML direct writes support `commit`; bound numeric previews do not issue IPC.
- Enqueueing performs no IO. `flush()` submits one `apply_properties` request; Rust
  resolves the logical paths against the evolving graph and compiles ordered
  operations into one existing asset transaction. The frontend does not parse YAML.
  While Unity is connected, that transaction performs one main-thread preflight
  and coordinated import pass. It does not use `SerializedProperty` setters for
  YAML editing. With Unity closed, it writes offline without starting Unity.
  This is not a guarantee that importing/OnValidate has no main-thread cost.
- A batch supports up to 256 files / 10,000 edits. Repeated flush calls share the
  in-flight promise. A failed batch cannot replay automatically: inspect its outcome,
  `clear()`, reread and rebuild. `live` apply may report partial success; it is not
  the same rollback contract as the YAML transaction.
- For profiling, use `yaml.apply({writes, profile:true})`. The optional response
  `profile` contains `prepareMs`, `commitMs`, `projectionMs`, `totalMs`,
  `effectiveBuilds`, `treeBuilds`, `overrideValidationPasses`, `materializedCompilePasses`, `readProjections`
  and byte-changing `changedFiles`. When connected, `profile.editor` separates
  Unity `preflightMs`, `writeMs`, `importMs` and `changedFiles`; offline it is null.
  Timings exclude frontend IPC serialization. No UI or global setting is required.
  For bulk scripts that do not need per-write snapshots, use
  `yaml.apply({writes, resultMode:"summary", profile:true})`. This returns
  `{ok,message,writesApplied,transactionId,assets:[{path,revision,dependencies}],profile?}`
  and skips Tree/beforeSnapshot projection while retaining identical validation and
  transaction guards. Use each asset's returned revision/dependencies for later writes.
  The default full response and `batch().flush()` are unchanged; live rejects summary.
  Materialized writes and inherited scalar runs share phase snapshots and validation; commands
  such as Revert/Apply/creation are ordering barriers. Always retain every returned
  dependency, including source `.cs` and `.cs.meta` evidence.
- YAML array commands support `resize`, `insert`, `delete`, `move`; growing and
  inserting need an explicit fill `value`. Whole-array values are also accepted.
  These commands also support inherited arrays and nested inline object lists.
  Existing size overrides on empty source arrays require a source-proven element
  schema. The batch compiles ordered edits into final size/leaf overrides.
  Unity's implicit element construction/type creation and restore commands are
  not emulated. Use `live` for those Inspector interactions.
- YAML authoring commands are values sent through the same `write/apply/batch` API:
  `{action:"revert"}` removes the current property override subtree, including array sizes/elements;
  `{action:"applyToSource",level:1}` applies to the next source layer and clears
  crossed overrides. `prefabLayers.length` applies to the materialized base.
  Whole-array Apply is supported when it needs no non-null local object/rid remapping.
  For a nested array whose containing element does not exist at the destination,
  apply an outer array that exists there. Existing inherited managed leaves use
  host-scoped registry identity, including aliases such as `node.next.amount`.
  `{action:"createManaged",template:{rootRid,entries:[{rid,type:{class,ns,asm},data}]}}`
  creates/replaces a materialized host slot using complete explicit data. Template
  rid labels are remapped, cycles preserved and old aliases retained. Missing,
  ambiguous, non-Serializable, abstract, generic or incompatible types are rejected.
  Use the exact Unity registry identity: nested classes use `Outer/Inner`, not `Outer.Inner`
  or CLR `Outer+Inner`. Property writes require source or supported built-in type evidence;
  partial, conditional or unavailable schemas fail with `property.schema_unverified`.
  The raw asset API remains a structural API with diagnostics, not an equivalent type guarantee.
  `{action:"editObjects",add:[{id,classId,rootType,data}],remove:[id],updates:[{objectId,propertyPath,value}]}`
  edits materialized object topology as one validated candidate. IDs must be exact
  strings; supported templates are GameObject, Transform, RectTransform and MonoBehaviour.
- Curves/gradients accept the standard live value payload in programmatic YAML
  writes. Their default YAML Tree previews remain read-only because the floating
  editor owns a live writer. Existing Prefab added/removed components and children
  are projected with ownership and instance identity, so their fields can be edited.
  Authoring topology on inherited objects, inherited managed creation/array structure,
  cross-layer reference Apply and nonfinite curve tangents remain unsupported.
  No implicit fallback, constructor or business-callback execution is provided;
  implement view-specific validation/derived values in the View.
- Bulk operations already expressed as raw asset operations can use
  `const batch = locus.assets.backend("yaml").batch()`, then
  `batch.enqueue(path, operations, {expected_revision})` and `await batch.flush()`.
  This bypasses logical Property Tree resolution. Keep draft UI state local and
  flush on explicit Apply, input commit or the end of a bulk job.

Start with `view`, `unity`, `property`, and components from `@locus/components`. Reach for the expanded helper exports only when custom rendering or value parsing requires them.

`session` covers the full session lifecycle: alongside `create`/`chat`/`wait`, it exposes `fork`/`forkFromMessage`, `list`/`listArchived`, `rename`, `archive`/`unarchive`, `delete`, and conversation-history `undo`/`rollback`. These operate on any session id in the current workspace — there is no per-View ownership scoping, so a View can manage sessions it did not create. `delete`, `undo`, and `rollback` are destructive and irreversible; confirm intent (and prefer `archive` for cleanup) before calling.

`useViewContext()` provides the editor-bound workspace, active ref, lifetime signal and onDispose(). `useViewState(initial, stableKey)` retains editor state across source updates. Hot-updating CSS preserves the component instance; script/template updates remount the affected View.
