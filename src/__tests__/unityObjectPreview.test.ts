import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { defineComponent } from "vue";
import {
  createUnityObjectDrawerLibrary,
  defineUnityObjectDrawers,
  hasEditableUnityPropertySnapshot,
  isUnityCodeSourceAssetPath,
  isUnityExternalSourceAssetPath,
  normalizeUnityObjectPreviewModel,
  resolveUnityObjectDrawer,
  type UnityObjectDrawerContext,
  unityObjectPreviewAssetRef,
  type UnityObjectPreviewInput,
} from "../components/unity-preview";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unityObjectPreview", () => {
  it("keeps external model source assets previewable and read-only", () => {
    const model = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Assets/Characters/Hero.fbx",
      name: "Hero",
    });

    expect(model.title).toBe("Hero");
    expect(model.iconKind).toBe("model");
    expect(model.capabilities.preview).toBe(true);
    expect(model.capabilities.edit).toBe(false);
    expect(model.editState).toBe("externalSource");
    expect(model.readonlyReason).toBe("External source asset");
    expect(isUnityExternalSourceAssetPath(model.ref.path)).toBe(true);
  });

  it("classifies script and shader sources as code source assets", () => {
    expect(isUnityCodeSourceAssetPath("Assets/Scripts/Effects/FireLight.cs")).toBe(true);
    expect(isUnityCodeSourceAssetPath("Assets/Shaders/Water.shader")).toBe(true);
    expect(isUnityCodeSourceAssetPath("Assets/Shaders/Blur.compute")).toBe(true);
    expect(isUnityCodeSourceAssetPath("Assets/Shaders/Common.cginc")).toBe(true);
    expect(isUnityCodeSourceAssetPath("Assets/Prefabs/Enemy.prefab")).toBe(false);
    expect(isUnityCodeSourceAssetPath("Assets/Data/Notes.txt")).toBe(false);
  });

  it("uses asset file names with extensions for default titles", () => {
    const scriptModel = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Assets/Scripts/AttackConfigSO.cs",
    });
    const configModel = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Assets/Data/BasicAttack.asset",
    });

    expect(scriptModel.title).toBe("AttackConfigSO.cs");
    expect(configModel.title).toBe("BasicAttack.asset");
  });

  it("allows importer property trees to opt into editing", () => {
    const input: UnityObjectPreviewInput = {
      ref: {
        kind: "importer",
        path: "Assets/Characters/Hero.fbx",
        name: "Hero Importer",
      },
      propertyTree: {
        propertyPath: "importer",
        displayName: "Importer",
        valueType: "Generic",
        value: null,
        editable: false,
        children: [
          {
            propertyPath: "importer.importMaterials",
            displayName: "Import Materials",
            valueType: "Boolean",
            value: true,
            editable: true,
          },
        ],
      },
    };

    const model = normalizeUnityObjectPreviewModel(input);

    expect(model.ref.kind).toBe("importer");
    expect(model.capabilities.drag).toBe(false);
    expect(model.capabilities.edit).toBe(true);
    expect(model.editState).toBe("editable");
    expect(model.readonlyReason).toBeUndefined();
  });

  it("detects editable descendants without requiring the root to be editable", () => {
    expect(hasEditableUnityPropertySnapshot({
      propertyPath: "root",
      valueType: "Generic",
      value: null,
      editable: false,
      children: [
        {
          propertyPath: "root.name",
          valueType: "String",
          value: "Player",
          editable: true,
        },
      ],
    })).toBe(true);

    expect(hasEditableUnityPropertySnapshot({
      propertyPath: "root",
      valueType: "Generic",
      value: null,
      editable: false,
      children: [
        {
          propertyPath: "root.name",
          valueType: "String",
          value: "Player",
          editable: false,
        },
      ],
    })).toBe(false);
  });

  it("normalizes scene object refs and exposes Unity drag refs", () => {
    const model = normalizeUnityObjectPreviewModel({
      kind: "sceneObject",
      path: "Assets/Scenes/Main.unity/Root/Player",
      name: "Player",
    });

    expect(model.ref.kind).toBe("sceneObject");
    expect(model.iconKind).toBe("gameobject");
    expect(model.capabilities.select).toBe(true);
    expect(model.capabilities.drag).toBe(true);
    expect(unityObjectPreviewAssetRef(model)).toEqual({
      kind: "sceneObject",
      path: "Assets/Scenes/Main.unity/Root/Player",
      name: "Player",
      typeLabel: undefined,
      source: "manual",
    });
  });

  it("keeps guid-only refs selectable without requiring editable property data", () => {
    const model = normalizeUnityObjectPreviewModel({
      kind: "asset",
      guid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    });

    expect(model.ref.path).toBe("");
    expect(model.ref.guid).toBe("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    expect(model.title).toBe("aaaaaaaa");
    expect(model.subtitle).toBe("GUID");
    expect(model.capabilities.select).toBe(true);
    expect(model.capabilities.drag).toBe(false);
    expect(model.capabilities.edit).toBe(false);
    expect(model.propertyTree).toBeNull();
  });

  it("keeps package assets read-only unless the caller provides mutability", () => {
    const readonlyModel = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Packages/com.vendor.tool/Runtime/Config.asset",
      propertyTree: {
        propertyPath: "config.enabled",
        valueType: "Boolean",
        value: true,
        editable: true,
      },
    });
    const writableModel = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Packages/com.vendor.tool/Runtime/Config.asset",
      writable: true,
      propertyTree: readonlyModel.propertyTree,
    });

    expect(readonlyModel.capabilities.edit).toBe(false);
    expect(readonlyModel.readonlyReason).toBe("Package asset");
    expect(writableModel.capabilities.edit).toBe(true);
  });

  it("resolves Unity object drawers by combined object and level matchers", () => {
    const PrefabThumbnailDrawer = defineComponent({ name: "PrefabThumbnailDrawer" });
    const RowDrawer = defineComponent({ name: "RowDrawer" });
    const model = normalizeUnityObjectPreviewModel({
      kind: "asset",
      path: "Assets/Prefabs/Enemy.prefab",
    });
    const baseContext: UnityObjectDrawerContext = {
      level: "thumbnail",
      selected: false,
      disabled: false,
      readonly: false,
      draggable: true,
      loading: false,
      error: "",
    };
    const library = createUnityObjectDrawerLibrary(defineUnityObjectDrawers([
      {
        extension: ".prefab",
        level: "thumbnail",
        drawer: PrefabThumbnailDrawer,
      },
      {
        level: "row",
        drawer: RowDrawer,
      },
    ]));

    expect(resolveUnityObjectDrawer(model, baseContext, library)).toBe(PrefabThumbnailDrawer);
    expect(resolveUnityObjectDrawer(model, { ...baseContext, level: "row" }, library)).toBe(RowDrawer);
    expect(resolveUnityObjectDrawer(model, { ...baseContext, level: "inspector" }, library)).toBeNull();
  });

  it("wires the public components to the shared preview and editor surfaces", () => {
    const preview = read("src/components/unity-preview/UnityObjectPreview.vue");
    const editor = read("src/components/unity-preview/UnityObjectEditorPanel.vue");
    const serializedTree = read("src/components/unity/UnitySerializedPropertyTree.vue");
    const numberField = read("src/components/unity/UnityNumberField.vue");
    const identity = read("src/components/unity-preview/UnityObjectIdentity.vue");
    const binaryHost = read("src/components/diff/BinaryPreviewHost.vue");
    const fbxPreview = read("src/components/diff/FbxBinaryPreview.vue");
    const assetService = read("src/services/asset.ts");
    const assetCommands = read("src-tauri/src/commands/asset.rs");
    const serializedService = read("src/services/unitySerializedProperty.ts");
    const objectDrawerService = read("src/services/unityObjectDrawer.ts");
    const propertyTree = read("src/services/propertyTree.ts");
    const serializedValue = read("src/components/unity/unitySerializedValue.ts");
    const serializedCommands = read("src-tauri/src/commands/unity_serialized_property.rs");
    const serializedRust = read("src-tauri/src/unity_serialized_property.rs");
    const serializedProperties = read("locus_unity/Editor/LocusBridge.SerializedProperties.cs");
    const viewBindings = read("locus_unity/Editor/LocusBridge.ViewBindings.cs");
    const unityBridge = read("locus_unity/Editor/LocusBridge.AssetThumbnail.cs");
    const chat = read("src/components/ChatView.vue");
    const viewRuntime = read("src/components/view/viewRuntime.ts");

    expect(preview).toContain("BinaryPreviewHost");
    expect(preview).toContain("UnityInspectorPane");
    expect(preview).toContain("UnityObjectEditorPanel");
    expect(preview).toContain("readUnitySerializedProperty");
    expect(preview).toContain("applyUnitySerializedProperties");
    expect(preview).toContain("pendingEditorWrites");
    expect(preview).toContain("editorPropertyWriteKey");
    expect(preview).toContain("unitySerializedTargetKey");
    expect(preview).toContain("commitRootTarget");
    expect(preview).toContain("refreshTargetKey");
    expect(preview).toContain("EDITOR_WRITE_MIN_INTERVAL_MS");
    expect(preview).toContain("window.setTimeout");
    expect(preview).toContain('writeMode: "commit" | "preview"');
    expect(preview).toContain("handleEditorPreview");
    expect(preview).toContain("isUnityConnectionError");
    expect(preview).toContain("asset.preview.unityConnectionRequired");
    expect(preview).toContain("unity-object-preview-error.neutral");
    expect(preview).toContain("livePropertyTree");
    expect(preview).toContain("liveSerializedTarget");
    expect(preview).toContain("AssetTextViewer");
    expect(preview).toContain("prefersCodeSourcePreview");
    expect(preview).toContain("isUnityCodeSourceAssetPath(autoPreviewPath.value)");
    expect(preview).toContain("(!prefersCodeSourcePreview.value || codeSourcePreviewUnavailable.value)");
    expect(preview).toContain("target.objectFileId ??");
    expect(preview).toContain("target.targetFileId ??");
    expect(preview).toContain("objectFileId,");
    expect(preview).toContain("createInspectorPropertyTreeBinding");
    expect(preview).toContain("editorPropertyTreeBinding");
    expect(preview).toContain("commitEditorPropertyTree");
    expect(preview).toContain("handleEditorCommit");
    expect(preview).toContain("previewSourceState");
    expect(preview).toContain('emit("source-change", state)');
    expect(preview).toContain("livePropertyTree.value ?? objectModel.value.propertyTree ?? null");
    expect(preview).not.toContain("&& !objectModel.value.propertyTree");
    expect(preview).toContain("resolveUnityObjectDrawer");
    expect(preview).toContain("objectDrawers");
    expect(preview).toContain("disableObjectDrawer");
    expect(preview).toContain("objectDrawerComponent");
    expect(preview).toContain("disable-object-drawer");
    expect(preview).toContain("maxDepth: 5");
    expect(preview).toContain("maxArrayItems: 128");
    expect(preview).toContain("previewWorkspaceAsset");
    expect(preview).toContain("previewWorkspaceAssetThumbnail");
    expect(preview).toContain("renderWorkspaceAssetPreviewFrame");
    expect(preview).toContain("readWorkspaceAssetPreviewFrameCache");
    expect(preview).toContain("cacheWorkspaceAssetPreviewFrame");
    expect(preview).toContain("loadWorkspaceAssetThumbnailCached");
    expect(preview).toContain("loadWorkspaceAssetPreviewFrameCacheCached");
    expect(preview).toContain("interactiveFrameInFlight");
    expect(preview).toContain("interactiveFrameQueued");
    expect(preview).toContain("interactivePanX");
    expect(preview).toContain("interactivePanY");
    expect(preview).toContain("interactivePanZ");
    expect(preview).toContain("INTERACTIVE_FRAME_MIN_INTERVAL_MS");
    expect(preview).toContain("INTERACTIVE_FRAME_CACHE_DELAY_MS");
    expect(preview).toContain("handleInteractivePreviewKeyDown");
    expect(preview).toContain("handleInteractivePreviewKeyUp");
    expect(preview).toContain("interactiveMoveKeys.has(\"w\")");
    expect(preview).toContain("interactiveMoveKeys.has(\"q\")");
    expect(preview).toContain("scheduleInteractiveFrameCache(frame)");
    expect(preview).toContain("decodeInteractiveFrame(frame.url)");
    expect(preview).toContain("handleInteractivePreviewPointerMove");
    expect(preview).toContain("handleInteractivePreviewWheel");
    expect(preview).toContain("event.button !== 0");
    expect(preview).toContain("(event.buttons & 1) !== 1");
    expect(preview).toContain("stopInteractivePreviewDrag(event)");
    expect(preview).toContain("interactiveYaw.value - dx");
    expect(preview).toContain("interactivePitch.value + dy");
    expect(preview).not.toContain(".unity-object-interactive-preview.enabled");
    expect(preview).not.toContain("scheduleInteractiveFrameRender(true)");
    expect(preview).toContain("autoLoadPreview");
    expect(preview).toContain("compact-summary");
    expect(preview).toContain("inspectorCollapsed");
    expect(preview).toContain("inspector-collapsed");
    expect(preview).toContain("toggleInspectorCollapsed");
    expect(preview).toContain("handlePreviewRootClick");
    expect(preview).toContain('@click="handlePreviewRootClick"');
    expect(preview).toContain("unity-object-inspector-header");
    expect(preview).toContain("unity-object-inspector-fold");
    expect(preview).toContain("structuredSelectableTargetIds");
    expect(preview).toContain("showStructuredTargetSelector");
    expect(preview).toContain('v-if="showStructuredTargetSelector"');
    expect(preview).toContain("canRenderBinaryThumbnail");
    expect(preview).toContain("Loading thumbnail...");
    expect(preview).toContain("unity-object-thumbnail-header");
    expect(preview).toContain("unity-object-thumbnail-binary");
    expect(preview).toContain("unity-object-thumbnail-row-fallback");
    expect(preview).toContain(":property-tree=\"editorPropertyTreeBinding\"");
    expect(preview).toContain(":show-header=\"false\"");
    expect(preview.match(/<UnityObjectEditorPanel[\s\S]*?@commit="handleEditorCommit"/g)?.length).toBeGreaterThanOrEqual(2);
    expect(preview.match(/class="unity-object-thumbnail-header"[\s\S]*?:show-path="true"/g)?.length).toBe(2);
    expect(preview.match(/class="unity-object-thumbnail-header"[\s\S]*?:draggable="objectModel\.capabilities\.drag"/g)?.length).toBe(2);
    expect(preview.match(/class="unity-object-thumbnail-header"[\s\S]*?:highlightable="objectModel\.capabilities\.drag"/g)?.length).toBe(2);
    expect(preview.match(/class="unity-object-thumbnail-row-fallback"[\s\S]*?:draggable="objectModel\.capabilities\.drag"/g)?.length).toBe(1);
    expect(preview.match(/class="unity-object-thumbnail-row-fallback"[\s\S]*?:highlightable="objectModel\.capabilities\.drag"/g)?.length).toBe(1);
    expect(preview).toContain("structuredSummaryTargets");
    expect(preview).not.toContain(':show-edit-state="level === \'row\'"');
    expect(preview).not.toContain(':show-edit-state="true"');
    expect(identity).toContain("highlightable?: boolean");
    expect(identity).toContain("highlightable: false");
    expect(identity).toContain(".unity-object-identity.highlightable:hover");
    expect(binaryHost).toContain("preview.kind === 'model'");
    expect(binaryHost).toContain(":compact=\"compact\"");
    expect(fbxPreview).toContain("compact?: boolean");
    expect(fbxPreview).toContain("v-if=\"!compact\"");
    expect(assetService).toContain("previewWorkspaceAssetThumbnail");
    expect(assetService).toContain("renderWorkspaceAssetPreviewFrame");
    expect(assetService).toContain("readWorkspaceAssetPreviewFrameCache");
    expect(assetService).toContain("cacheWorkspaceAssetPreviewFrame");
    expect(assetService).toContain("panX");
    expect(assetCommands).toContain("preview_workspace_asset_thumbnail");
    expect(assetCommands).toContain("render_workspace_asset_preview_frame");
    expect(assetCommands).toContain("read_workspace_asset_preview_frame_cache");
    expect(assetCommands).toContain("cache_workspace_asset_preview_frame");
    expect(assetCommands).toContain("asset-preview-cache");
    expect(assetCommands).toContain("asset_thumbnail(&cwd, &asset_rel_path, 192)");
    expect(assetCommands).toContain("asset_preview_render");
    expect(assetCommands).toContain("pan_x");
    expect(serializedService).toContain("unity_serialized_property_read");
    expect(serializedService).toContain("unity_serialized_property_discover");
    expect(serializedService).toContain("unity_serialized_property_write");
    expect(serializedService).toContain("unity_serialized_property_apply");
    expect(serializedService).toContain("properties?: UnitySerializedPropertySnapshot[]");
    expect(serializedCommands).toContain("pub async fn unity_serialized_property_read");
    expect(serializedCommands).toContain("pub async fn unity_serialized_property_apply");
    expect(serializedRust).toContain("validate_object_target");
    expect(serializedRust).toContain("validate_property_target");
    expect(serializedRust).toContain("maxArrayItems");
    expect(serializedRust).toContain("write_mode");
    expect(serializedRust).toContain("normalize_write_mode");
    expect(propertyTree).toContain("normalizePropertyAttributes");
    expect(propertyTree).toContain("registerInspectorValueDrawer");
    expect(propertyTree).toContain("registerInspectorAttributeDrawer");
    expect(propertyTree).toContain("propertyDrawers");
    expect(propertyTree).toContain("drawer: Component");
    expect(objectDrawerService).toContain("publicUnityObjectDrawerLibrary");
    expect(objectDrawerService).toContain("registerUnityObjectDrawer");
    expect(objectDrawerService).toContain("unityObjectDrawerMatches");
    expect(objectDrawerService).toContain("refKind");
    expect(objectDrawerService).toContain("extension");
    expect(viewRuntime).toContain("UnityObjectPreview");
    expect(viewRuntime).toContain("unityObjectDrawer");
    expect(propertyTree).toContain("referenceTypeFullName");
    expect(propertyTree).toContain("bindingTarget?: InspectorPropertyTargetSnapshot");
    expect(propertyTree).toContain("hasRange");
    expect(propertyTree).toContain("tooltip");
    expect(serializedValue).toContain("UnitySerializedPropertyAttributeInfo");
    expect(serializedValue).toContain("bindingTarget?: UnitySerializedPropertyTargetSnapshot");
    expect(serializedValue).toContain("objectFileId?: number | null");
    expect(serializedValue).toContain("targetFileId?: number | null");
    expect(serializedProperties).toContain("SnapshotSerializedObject");
    expect(serializedProperties).toContain("SerializedPropertyBindingTarget");
    expect(serializedProperties).toContain("SerializedPropertyAttributeInfo");
    expect(serializedProperties).toContain("RangeAttribute");
    expect(serializedProperties).toContain("TooltipAttribute");
    expect(serializedProperties).toContain("referenceTypeFullName");
    expect(viewBindings).toContain("ValidateViewBindingObjectTarget");
    expect(viewBindings).toContain("SnapshotSerializedObject");
    expect(viewBindings).toContain("SnapshotViewBindingObjectProperties");
    expect(viewBindings).toContain("ViewBindingComponentTarget");
    expect(viewBindings).toContain("ViewBindingComponentEnabledPropertyPath");
    expect(viewBindings).toContain("ViewBindingGameObjectActivePropertyPath");
    expect(viewBindings).toContain("BuildViewBindingSyntheticHeaderPropertySnapshot");
    expect(viewBindings).toContain("WriteViewBindingSyntheticHeaderProperty");
    expect(viewBindings).toContain("TrySetViewBindingComponentEnabledState");
    expect(viewBindings).toContain("go.SetActive(value)");
    expect(viewBindings).toContain("ResolvePrefabAssetGameObjectTarget");
    expect(viewBindings).toContain("EditorUtility.SetDirty(prefabRoot)");
    expect(viewBindings).toContain("\\\"properties\\\":");
    expect(viewBindings).toContain("IsViewBindingPreviewMode");
    expect(viewBindings).toContain("CanPreviewWriteSerializedProperty");
    expect(viewBindings).toContain("TrySetDirectPreviewPathValue");
    expect(viewBindings).not.toContain("PrefabUtility.SavePrefabAsset");
    expect(viewBindings).not.toContain("AssetDatabase.SaveAssets");
    expect(viewBindings).not.toContain("AssetDatabase.SaveAssetIfDirty");
    expect(viewBindings).not.toContain("QueueViewBindingAssetSave");
    expect(unityBridge).toContain("AssetPreview.GetAssetPreview");
    expect(unityBridge).toContain("AssetPreviewRenderSession");
    expect(unityBridge).toContain("AssetPreviewRenderSessions");
    expect(unityBridge).toContain("preview.AddSingleGO(instance)");
    expect(unityBridge).toContain("GetAssetPreviewRenderSession(assetPath).Render(request)");
    expect(unityBridge).toContain("EncodeToPNG()");
    expect(unityBridge).toContain("mimeType = \"image/png\"");
    expect(unityBridge).toContain("request.panX");
    expect(unityBridge).toContain("request.panY");
    expect(unityBridge).toContain("request.panZ");
    expect(unityBridge).toContain("Asset thumbnail unavailable");
    expect(chat).toContain("function isInsidePassiveMarkdownUnityPreview");
    expect(chat).toContain("if (isInsidePassiveMarkdownUnityPreview(target)) return;");
    expect(chat).toContain("[data-md-unity-passive='true']");
    expect(editor).toContain("propertyTree?: InspectorPropertyTreeBindingInput");
    expect(editor).toContain("showHeader?: boolean");
    expect(editor).toContain("createInspectorPropertyTreeBinding");
    expect(editor).toContain('v-if="showHeader"');
    expect(editor).toContain(":source=\"sourceForProperty(property)\"");
    expect(editor).toContain("unity-component-header");
    expect(editor).toContain("unity-component-fold-button");
    expect(editor).toContain("unity-component-enable");
    expect(editor).toContain("componentEnableProperty");
    expect(editor).toContain("commitComponentEnable");
    expect(editor).toContain("m_Enabled");
    expect(editor).toContain("m_IsActive");
    expect(editor).toContain("hide-root-object-header");
    expect(editor).toContain("UnitySerializedPropertyTree");
    expect(editor).not.toContain("objectModel.value.editState");
    expect(editor).not.toContain("show-edit-state");
    expect(serializedTree).toContain("hideRootObjectHeader?: boolean");
    expect(serializedTree).toContain("hideObjectHeader");
    expect(serializedTree).toContain("hide-root-header");
    expect(serializedTree).toContain("startNumberLabelDrag");
    expect(serializedTree).toContain("property-name-drag");
    expect(serializedTree).toContain('preview: [event: UnitySerializedPropertyCommitEvent]');
    expect(serializedTree).toContain('writeMode,');
    expect(numberField).toContain('type="range"');
    expect(numberField).toContain('emit("commit", parsed)');
    expect(numberField).toContain('emit("preview", parsed)');
    expect(numberField).toContain("constrainUnityNumberValue");
    expect(numberField).toContain("usesRange");
    expect(identity).toContain("armUnityReferencePointerDrag");
    expect(identity).toContain("resolveRefGraphGuid");
    expect(identity).toContain("resolveRefGraphPath");
    expect(identity).toContain("unityObjectPreviewAssetRef");
  });

  it("routes markdown Unity refs through the public preview component without replacing delegated actions", () => {
    const renderer = read("src/components/MarkdownRenderer.vue");
    const inject = read("src/composables/markdownInject.ts");

    expect(inject).toContain('data-md-unity-object-preview="true"');
    expect(inject).toContain("md-unity-object-ref");
    expect(renderer).toContain("import UnityObjectPreview from \"./unity-preview/UnityObjectPreview.vue\"");
    expect(renderer).toContain("injectUnityObjectFenceRefs");
    expect(renderer).toContain("isMarkdownUnityObjectFenceLanguage(normalizedLang)");
    expect(renderer).toContain("function mountMarkdownUnityObjectPreviews()");
    expect(renderer).toContain("[data-md-unity-object-preview='true']");
    expect(renderer).toContain("data-md-unity-passive");
    expect(renderer).toContain("host.removeAttribute(\"draggable\")");
    expect(renderer).toContain("isInsidePassiveMarkdownUnityPreview");
    expect(renderer).toContain("draggable: false");
    expect(renderer).toContain("autoLoadPreview: true");
    expect(renderer).toContain("unityPreviewStateScope?: string | null");
    expect(renderer).toContain("function markdownUnityObjectPreviewStateKey");
    expect(renderer).toContain("previewStateKey: markdownUnityObjectPreviewStateKey(host, index, model, level)");
    expect(renderer).toContain("host.replaceChildren();");
    expect(renderer).toContain(".md-unity-object-ref[data-md-unity-level=\"row\"]");
    expect(renderer).toContain("target.closest(\".md-unity-asset-ref, .md-file-ref[data-asset-path]\")");
  });

  it("remembers markdown inspector expansion by preview state key", () => {
    const preview = read("src/components/unity-preview/UnityObjectPreview.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(preview).toContain("unityObjectPreviewExpandedStateCache");
    expect(preview).toContain("previewStateKey?: string");
    expect(preview).toContain("readUnityObjectPreviewExpandedState(previewStateKey)");
    expect(preview).toContain("rememberUnityObjectPreviewExpandedState(props.previewStateKey, !inspectorCollapsed.value)");
    expect(preview).toContain(":preview-state-key=\"previewStateKey\"");
    expect(transcript).toContain("function markdownUnityPreviewStateScope");
    expect(transcript).toContain(":unity-preview-state-scope=\"markdownUnityPreviewStateScope(segment)\"");
  });
});
