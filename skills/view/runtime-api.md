# View Runtime API Quick Reference

Use this when building a Locus View package. Prefer `@locus/view-runtime` for services, Unity editing, drawers, drag/drop, graph/canvas helpers, session, LLM, storage, and logs. Import visual components from `@locus/components`.

## Imports

- `vue`: Vue runtime APIs are available; `createApp(...).mount(...)` is captured by the View host.
- `@locus/view-runtime`: main service and helper SDK.
- `@locus/components`: component module.
- `node:fs/promises`, `fs/promises`: Promise-based filesystem APIs.
- `node:fs`, `fs`: filesystem APIs with `promises` and common callback forms.
- `node:path`, `path`: common path helpers.
- Relative package modules: `.ts`, `.vue`, `.js`, `.css`, plus extension-qualified files that the runtime compiler can execute.
- Shared View package modules: `@locus/project-view` and `@project-view`.

## Main SDK Values

`@locus/view-runtime` currently exposes these runtime values:

- `view`: `manifest`, `summary`, `reload`, `callScript`, `assets.search`, `logs.read/latest/open`, `session`, `llm`, `storage`, `fs`, `path`, `unity`, `files`, `undo`, `propertyDrawer`, `unityObjectDrawer`, `objectReferencePicker`, `openLog`, `onUpdate`.
- `session`: `create`, `show`, `display`, `load`, `activeRun`, `events`, `queueInput`, `chat`, `send`, `wait`, `onEvent`.
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
- Helpers: `defineView`, `useViewState`, `useViewScript`, `onEditorUpdate`, `useUnityReferenceDrag`, `useUnityAssetDropTarget`, `useLocusFileDrag`, `useLocusFileDropTarget`.
- Graph helpers: `GraphViewController`, `defineGraphView`, `layoutGraphDocument`.

Legacy globals are still installed as `window.locus.view` and `window.locus.unity`.

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
- `CanvasView`, `GraphView`.
- `UnityBoolField`, `UnityColorField`, `UnityEnumField`, `UnityFlagsField`, `UnityLayerMaskField`, `UnityNumberField`, `UnityObjectReferenceField`, `UnityPropertyDraw`, `UnityPropertyEditor`, `UnitySerializedPropertyTree`, `UnityVectorField`.
- `UnityObjectPreview`, `UnityReferenceChip`, `UnityDropZone`.

## Agent Tools

The View skill grants these normal tools: `view_list`, `view_create`, `view_reload`, `view_run`, `view_compile_script`, `view_call_script`, `view_property_read`, `view_property_discover`, `view_property_write`, `view_property_apply`.

Debug-only tools live in `debug.md`: `view_capture`, `view_snapshot`, `view_action`, `view_wait`, `view_console_read`, `view_debug_eval`.

## Design Notes

The API shape is mostly sound: high-frequency work uses compact namespaces (`view`, `unity`, `property`, `session`), Unity writes flow through Unity property paths, and generated packages can build useful interfaces without a local bundler. The broad helper spread gives agents enough power for custom drawers and inspectors, so generated Views should start with `view`, `unity`, `property`, and components from `@locus/components`, then reach for expanded helpers only when custom rendering or value parsing is required.
