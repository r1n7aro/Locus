<script lang="ts">
import {
  cacheWorkspaceAssetPreviewFrame,
  previewWorkspaceAsset,
  previewWorkspaceAssetTarget,
  previewWorkspaceAssetThumbnail,
  readWorkspaceAssetPreviewFrameCache,
  renderWorkspaceAssetPreviewFrame,
} from "../../services/asset";

type WorkspaceAssetThumbnail = Awaited<ReturnType<typeof previewWorkspaceAssetThumbnail>>;
type WorkspaceAssetPreviewFrame = Awaited<ReturnType<typeof renderWorkspaceAssetPreviewFrame>>;

const workspacePreviewPayloadCache = new Map<string, {
  payload?: Awaited<ReturnType<typeof previewWorkspaceAsset>>;
  promise?: ReturnType<typeof previewWorkspaceAsset>;
}>();

const workspaceAssetThumbnailCache = new Map<string, {
  thumbnail?: WorkspaceAssetThumbnail;
  promise?: ReturnType<typeof previewWorkspaceAssetThumbnail>;
}>();

const workspaceAssetPreviewFrameCache = new Map<string, {
  frame?: WorkspaceAssetPreviewFrame | null;
  promise?: ReturnType<typeof readWorkspaceAssetPreviewFrameCache>;
}>();

const workspacePreviewTargetCache = new Map<string, {
  inspector?: Awaited<ReturnType<typeof previewWorkspaceAssetTarget>>;
  promise?: ReturnType<typeof previewWorkspaceAssetTarget>;
}>();

const unityObjectPreviewExpandedStateCache = new Map<string, boolean>();
const UNITY_OBJECT_PREVIEW_EXPANDED_STATE_CACHE_LIMIT = 2000;

function normalizedPreviewCacheKey(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function normalizedPreviewStateKey(value: string | undefined): string {
  return (value ?? "").trim();
}

function readUnityObjectPreviewExpandedState(stateKey: string | undefined): boolean | null {
  const key = normalizedPreviewStateKey(stateKey);
  if (!key || !unityObjectPreviewExpandedStateCache.has(key)) return null;
  const expanded = unityObjectPreviewExpandedStateCache.get(key)!;
  unityObjectPreviewExpandedStateCache.delete(key);
  unityObjectPreviewExpandedStateCache.set(key, expanded);
  return expanded;
}

function rememberUnityObjectPreviewExpandedState(stateKey: string | undefined, expanded: boolean) {
  const key = normalizedPreviewStateKey(stateKey);
  if (!key) return;
  if (unityObjectPreviewExpandedStateCache.has(key)) {
    unityObjectPreviewExpandedStateCache.delete(key);
  }
  unityObjectPreviewExpandedStateCache.set(key, expanded);
  while (unityObjectPreviewExpandedStateCache.size > UNITY_OBJECT_PREVIEW_EXPANDED_STATE_CACHE_LIMIT) {
    const oldestKey = unityObjectPreviewExpandedStateCache.keys().next().value;
    if (!oldestKey) break;
    unityObjectPreviewExpandedStateCache.delete(oldestKey);
  }
}

function loadWorkspaceAssetPreviewCached(path: string): ReturnType<typeof previewWorkspaceAsset> {
  const key = normalizedPreviewCacheKey(path);
  const cached = workspacePreviewPayloadCache.get(key);
  if (cached?.payload) return Promise.resolve(cached.payload);
  if (cached?.promise) return cached.promise;

  const promise = previewWorkspaceAsset(key)
    .then((payload) => {
      workspacePreviewPayloadCache.set(key, { payload });
      return payload;
    })
    .catch((error) => {
      workspacePreviewPayloadCache.delete(key);
      throw error;
    });
  workspacePreviewPayloadCache.set(key, { promise });
  return promise;
}

function loadWorkspaceAssetThumbnailCached(
  path: string,
): ReturnType<typeof previewWorkspaceAssetThumbnail> {
  const key = normalizedPreviewCacheKey(path);
  const cached = workspaceAssetThumbnailCache.get(key);
  if (cached?.thumbnail) return Promise.resolve(cached.thumbnail);
  if (cached?.promise) return cached.promise;

  const promise = previewWorkspaceAssetThumbnail(key)
    .then((thumbnail) => {
      workspaceAssetThumbnailCache.set(key, { thumbnail });
      return thumbnail;
    })
    .catch((error) => {
      workspaceAssetThumbnailCache.delete(key);
      throw error;
    });
  workspaceAssetThumbnailCache.set(key, { promise });
  return promise;
}

function loadWorkspaceAssetPreviewFrameCacheCached(
  path: string,
): ReturnType<typeof readWorkspaceAssetPreviewFrameCache> {
  const key = normalizedPreviewCacheKey(path);
  const cached = workspaceAssetPreviewFrameCache.get(key);
  if (cached && "frame" in cached) return Promise.resolve(cached.frame ?? null);
  if (cached?.promise) return cached.promise;

  const promise = readWorkspaceAssetPreviewFrameCache(key)
    .then((frame) => {
      workspaceAssetPreviewFrameCache.set(key, { frame });
      return frame;
    })
    .catch((error) => {
      workspaceAssetPreviewFrameCache.delete(key);
      throw error;
    });
  workspaceAssetPreviewFrameCache.set(key, { promise });
  return promise;
}

function rememberWorkspaceAssetPreviewFrameCache(path: string, frame: WorkspaceAssetPreviewFrame) {
  workspaceAssetPreviewFrameCache.set(normalizedPreviewCacheKey(path), { frame });
}

function loadWorkspaceAssetTargetCached(
  previewKey: string,
  targetId: string,
): ReturnType<typeof previewWorkspaceAssetTarget> {
  const key = `${previewKey}:${targetId}`;
  const cached = workspacePreviewTargetCache.get(key);
  if (cached?.inspector) return Promise.resolve(cached.inspector);
  if (cached?.promise) return cached.promise;

  const promise = previewWorkspaceAssetTarget(previewKey, targetId)
    .then((inspector) => {
      workspacePreviewTargetCache.set(key, { inspector });
      return inspector;
    })
    .catch((error) => {
      workspacePreviewTargetCache.delete(key);
      throw error;
    });
  workspacePreviewTargetCache.set(key, { promise });
  return promise;
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import type {
  AssetBinaryMeta,
  AssetPreviewPayload,
  SemanticDisplayMode,
  SemanticTargetInspector,
} from "../../types";
import {
  createInspectorPropertyTreeBinding,
  createPropertyTree,
  type InspectorPropertyDrawerInput,
  type InspectorPropertyCommit,
  type InspectorPropertyTreeBinding,
} from "../../services/propertyTree";
import {
  resolveUnityObjectDrawer,
  type UnityObjectDrawerContext,
  type UnityObjectDrawerInput,
} from "../../services/unityObjectDrawer";
import type { UnitySerializedPropertyCommitEvent } from "../unity/unitySerializedValue";
import { t } from "../../i18n";
import { isUnityConnectionError, normalizeAppError } from "../../services/errors";
import {
  applyUnitySerializedProperties,
  readUnitySerializedProperty,
  type UnitySerializedPropertyTarget,
} from "../../services/unitySerializedProperty";
import BinaryPreviewHost from "../diff/BinaryPreviewHost.vue";
import UnityHierarchyPane from "../diff/UnityHierarchyPane.vue";
import UnityInspectorPane from "../diff/UnityInspectorPane.vue";
import {
  hasEditableUnityPropertySnapshot,
  isUnityExternalSourceAssetPath,
  normalizeUnityObjectPreviewModel,
  type UnityObjectPreviewInput,
  type UnityObjectPreviewLevel,
  type UnityObjectPreviewModel,
  type UnityObjectPreviewSourceState,
  type UnityObjectPropertyTreeInput,
} from "./unityObjectPreview";
import UnityObjectEditorPanel from "./UnityObjectEditorPanel.vue";
import UnityObjectIdentity from "./UnityObjectIdentity.vue";

const props = withDefaults(defineProps<{
  model: UnityObjectPreviewInput | UnityObjectPreviewModel;
  level?: UnityObjectPreviewLevel;
  loading?: boolean;
  error?: string;
  selected?: boolean;
  disabled?: boolean;
  readonly?: boolean;
  draggable?: boolean;
  diffKey?: string;
  displayMode?: SemanticDisplayMode;
  targetLoading?: boolean;
  includeUnchanged?: boolean;
  propertyDrawers?: InspectorPropertyDrawerInput;
  objectDrawers?: UnityObjectDrawerInput;
  disableObjectDrawer?: boolean;
  autoLoadPreview?: boolean;
  previewStateKey?: string;
}>(), {
  level: "inline",
  loading: false,
  error: "",
  selected: false,
  disabled: false,
  readonly: false,
  draggable: true,
  diffKey: "",
  displayMode: "optimized",
  targetLoading: false,
  includeUnchanged: true,
  propertyDrawers: undefined,
  objectDrawers: undefined,
  disableObjectDrawer: false,
  autoLoadPreview: true,
  previewStateKey: "",
});

const emit = defineEmits<{
  select: [model: UnityObjectPreviewModel];
  commit: [event: UnitySerializedPropertyCommitEvent];
  preview: [event: UnitySerializedPropertyCommitEvent];
  blocked: [model: UnityObjectPreviewModel];
  "source-change": [state: UnityObjectPreviewSourceState];
}>();

const objectModel = computed(() => normalizeUnityObjectPreviewModel(props.model));
const autoPreviewPayload = ref<AssetPreviewPayload | null>(null);
const autoPreviewLoading = ref(false);
const autoPreviewError = ref("");
const autoThumbnail = ref<WorkspaceAssetThumbnail | null>(null);
const autoThumbnailLoading = ref(false);
const interactiveFrame = ref<WorkspaceAssetPreviewFrame | null>(null);
const interactiveFrameLoading = ref(false);
const interactiveFrameError = ref("");
const interactiveYaw = ref(25);
const interactivePitch = ref(-12);
const interactiveDistance = ref(1.15);
const interactivePanX = ref(0);
const interactivePanY = ref(0);
const interactivePanZ = ref(0);
const interactiveDragging = ref(false);
const activeTargetId = ref<string | null>(null);
const inspectorCollapsed = ref(false);
const targetCache = ref<Map<string, SemanticTargetInspector>>(new Map());
const autoTargetLoading = ref(false);
const autoTargetError = ref("");
const livePropertyTree = ref<UnityObjectPropertyTreeInput | null>(null);
const livePropertyLoading = ref(false);
const livePropertyError = ref("");
let autoPreviewRun = 0;
let autoThumbnailRun = 0;
let interactiveFrameRun = 0;
let interactiveFrameInFlight = false;
let interactiveFrameQueued = false;
let interactiveFrameTimer: number | null = null;
let interactiveFrameCacheTimer: number | null = null;
let lastInteractiveFrameAt = 0;
let interactivePointerId: number | null = null;
let interactivePointerX = 0;
let interactivePointerY = 0;
let interactiveKeyboardFrame: number | null = null;
let interactiveKeyboardLastAt = 0;
let autoTargetRun = 0;
let livePropertyRun = 0;
let editorWriteOrder = 0;
let editorWriteVersion = 0;
let editorWriteFlushTimer: number | null = null;
let editorWriteInFlight = false;
let editorWriteRefreshTargetKey = "";
let lastEditorWriteFlushAt = 0;
let disposed = false;

interface PendingEditorPropertyWrite {
  targetKey: string;
  refreshTargetKey: string;
  bindingId: string;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode: "commit" | "preview";
  order: number;
}

const pendingEditorWrites = new Map<string, PendingEditorPropertyWrite>();

const INTERACTIVE_FRAME_MIN_INTERVAL_MS = 90;
const INTERACTIVE_FRAME_CACHE_DELAY_MS = 450;
const INTERACTIVE_KEY_MOVE_SPEED = 0.95;
const EDITOR_WRITE_MIN_INTERVAL_MS = 90;
const interactiveMoveKeys = new Set<string>();
const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const UNITY_SCENE_OBJECT_PATH_RE = /^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i;

function unitySerializedTargetKey(target: UnitySerializedPropertyTarget | null | undefined): string {
  if (!target) return "";
  return [
    target.kind,
    target.path ?? "",
    target.scenePath ?? "",
    target.objectPath ?? "",
    target.objectFileId ?? "",
    target.targetFileId ?? "",
    target.componentType ?? "",
    target.componentIndex ?? "",
  ].join("|");
}

function unitySerializedTargetWithProperty(
  target: UnitySerializedPropertyTarget,
  propertyPath: string,
): UnitySerializedPropertyTarget {
  return {
    ...target,
    propertyPath,
  };
}

const previewPayload = computed(() => objectModel.value.previewPayload ?? autoPreviewPayload.value);
const structuredPayload = computed(() => (
  previewPayload.value?.kind === "structured" ? previewPayload.value : null
));
const binaryPreviewPayload = computed(() => (
  previewPayload.value?.kind === "binaryPreview" ? previewPayload.value : null
));
const binaryInfoPayload = computed(() => (
  previewPayload.value?.kind === "binaryInfo" ? previewPayload.value : null
));
const textPayload = computed(() => (
  previewPayload.value?.kind === "text" ? previewPayload.value : null
));
const inspector = computed(() => (
  objectModel.value.inspector
  ?? (activeTargetId.value ? targetCache.value.get(activeTargetId.value) ?? null : null)
));
const effectiveDiffKey = computed(() =>
  props.diffKey || `unity-object:${objectModel.value.ref.path || objectModel.value.title}`,
);
const compactPreview = computed(() => props.level === "thumbnail");
const autoPreviewPath = computed(() => objectModel.value.ref.path.trim().replace(/\\/g, "/"));
const liveSerializedTarget = computed<UnitySerializedPropertyTarget | null>(() => {
  const model = objectModel.value;
  const path = model.ref.path.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
  if (!path) return null;
  const objectFileId = Number.isFinite(model.ref.fileId) && model.ref.fileId !== 0
    ? model.ref.fileId
    : undefined;

  if (model.ref.kind === "sceneObject") {
    const match = path.match(UNITY_SCENE_OBJECT_PATH_RE);
    if (!match) return { kind: "gameObject", objectPath: path, objectFileId };
    return {
      kind: "gameObject",
      scenePath: match[1],
      objectPath: match[2].replace(/^\/+|\/+$/g, ""),
      objectFileId,
    };
  }

  if (model.ref.kind !== "asset" && model.ref.kind !== "subObject") return null;
  if (!UNITY_ASSET_REF_ROOT_RE.test(path)) return null;
  if (isUnityExternalSourceAssetPath(path)) return null;
  return { kind: "asset", path };
});
const liveSerializedTargetKey = computed(() => {
  return unitySerializedTargetKey(liveSerializedTarget.value);
});
const canAutoLoadPreview = computed(() => (
  props.autoLoadPreview
  && (props.level === "thumbnail" || props.level === "inspector")
  && !objectModel.value.previewPayload
  && !objectModel.value.inspector
  && objectModel.value.ref.kind === "asset"
  && /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i.test(autoPreviewPath.value)
));
const canAutoLoadThumbnail = computed(() => (
  props.autoLoadPreview
  && props.level === "thumbnail"
  && !objectModel.value.previewPayload
  && objectModel.value.ref.kind === "asset"
  && autoPreviewPath.value.toLowerCase().endsWith(".prefab")
  && /^(?:Assets|Packages)(?:\/|$)/i.test(autoPreviewPath.value)
));
const canAutoLoadInteractivePreview = computed(() => canAutoLoadThumbnail.value);
const canAutoLoadLiveProperties = computed(() => (
  props.autoLoadPreview
  && (props.level === "editor" || props.level === "inspector")
  && !!liveSerializedTarget.value
));
const effectivePropertyTree = computed<UnityObjectPropertyTreeInput | null>(() =>
  livePropertyTree.value ?? objectModel.value.propertyTree ?? null,
);
const liveEditorModel = computed<UnityObjectPreviewInput>(() => {
  const propertyTree = effectivePropertyTree.value;
  const editable = hasEditableUnityPropertySnapshot(propertyTree);
  return {
    ref: objectModel.value.ref,
    title: objectModel.value.title,
    subtitle: objectModel.value.subtitle,
    iconKind: objectModel.value.iconKind,
    previewPayload: objectModel.value.previewPayload,
    inspector: objectModel.value.inspector,
    propertyTree,
    writable: editable,
    readonlyReason: livePropertyErrorState.value
      || (livePropertyLoading.value ? "Loading properties..." : objectModel.value.readonlyReason),
    capabilities: {
      ...objectModel.value.capabilities,
      inspect: true,
      edit: editable,
    },
  };
});
const editorPropertyTreeBinding = computed<InspectorPropertyTreeBinding>(() => {
  const propertyTree = effectivePropertyTree.value;
  const targetId = liveSerializedTargetKey.value || objectModel.value.ref.path || objectModel.value.title;
  return createInspectorPropertyTreeBinding({
    id: `unity-object:${targetId}`,
    targetId,
    snapshots: propertyTree,
    loading: livePropertyLoading.value,
    error: livePropertyErrorState.value,
    disabled: props.disabled || livePropertyLoading.value,
    readonly: props.readonly,
    editable: hasEditableUnityPropertySnapshot(propertyTree),
  });
});
const previewSourceState = computed<UnityObjectPreviewSourceState>(() => {
  if (
    livePropertyLoading.value ||
    (canAutoLoadLiveProperties.value && !livePropertyTree.value && !livePropertyError.value)
  ) return "loading";
  if (livePropertyTree.value) return "live";
  return "disk";
});
const interactivePreviewUrl = computed(() =>
  interactiveFrame.value?.url || autoThumbnail.value?.url || "",
);
const loadingState = computed(() => props.loading || autoPreviewLoading.value);
const errorSource = computed(() => props.error || autoPreviewError.value);
const errorState = computed(() => previewErrorDisplayMessage(errorSource.value));
const errorStateRequiresUnity = computed(() => isUnityConnectionError(errorSource.value));
const objectDrawerContext = computed<UnityObjectDrawerContext>(() => ({
  level: props.level,
  selected: props.selected,
  disabled: props.disabled,
  readonly: props.readonly,
  draggable: props.draggable,
  loading: loadingState.value,
  error: errorState.value,
}));
const objectDrawerComponent = computed(() => props.disableObjectDrawer
  ? null
  : resolveUnityObjectDrawer(objectModel.value, objectDrawerContext.value, props.objectDrawers));
const thumbnailLoadingState = computed(() => (
  (loadingState.value && !previewPayload.value && canAutoLoadPreview.value)
  || autoThumbnailLoading.value
  || (interactiveFrameLoading.value && !interactivePreviewUrl.value)
));
const targetLoadingState = computed(() => props.targetLoading || autoTargetLoading.value);
const targetErrorState = computed(() => previewErrorDisplayMessage(autoTargetError.value));
const targetErrorRequiresUnity = computed(() => isUnityConnectionError(autoTargetError.value));
const livePropertyErrorState = computed(() => previewErrorDisplayMessage(livePropertyError.value));
const livePropertyErrorRequiresUnity = computed(() => isUnityConnectionError(livePropertyError.value));
const canRenderBinaryThumbnail = computed(() => {
  const payload = binaryPreviewPayload.value;
  if (!payload) return false;
  return payload.preview.kind === "image"
    || payload.preview.kind === "psd"
    || payload.preview.kind === "model";
});
const shouldRenderBinaryPreviewHost = computed(() => {
  const payload = binaryPreviewPayload.value;
  if (!payload) return false;
  if (!compactPreview.value) return true;
  return canRenderBinaryThumbnail.value;
});
const hasRealThumbnail = computed(() => (
  props.level === "thumbnail"
  && (!!interactivePreviewUrl.value || canRenderBinaryThumbnail.value)
));
const usesCompactSummary = computed(() => compactPreview.value && !hasRealThumbnail.value);
const previewClass = computed(() => [
  `level-${props.level}`,
  {
    "compact-summary": usesCompactSummary.value,
    "inspector-collapsed": props.level === "inspector" && inspectorCollapsed.value,
  },
]);
const compactBinaryMeta = computed<AssetBinaryMeta | null>(() =>
  binaryPreviewPayload.value?.meta ?? binaryInfoPayload.value?.meta ?? null,
);
const compactBinaryKindLabel = computed(() => {
  const previewKind = binaryPreviewPayload.value?.preview.kind;
  if (previewKind === "model") return "3D model";
  if (previewKind === "image") return "Image";
  if (previewKind === "psd") return "PSD";
  return compactBinaryMeta.value?.ext?.replace(/^\./, "").toUpperCase() || "Binary";
});
const compactTextLines = computed(() =>
  (textPayload.value?.snippet ?? "").split(/\r?\n/g).slice(0, 6),
);
const structuredSummaryTargets = computed(() =>
  structuredPayload.value?.targets.slice(0, 4) ?? [],
);
const structuredSelectableTargetIds = computed(() => {
  const payload = structuredPayload.value;
  if (!payload) return [];
  return [...new Set(payload.tree
    .filter((node) => node.hasInspector)
    .map((node) => node.id))];
});
const showStructuredTargetSelector = computed(() => structuredSelectableTargetIds.value.length > 1);
const structuredSummaryMeta = computed(() => {
  const payload = structuredPayload.value;
  if (!payload) return [];
  const inspectable = payload.tree.filter((node) => node.hasInspector).length;
  const roots = payload.tree.filter((node) => !node.parentId).length;
  return [
    `${payload.tree.length} nodes`,
    `${inspectable} inspectable`,
    `${roots} roots`,
  ];
});
const defaultStructuredTargetId = computed(() => {
  const payload = structuredPayload.value;
  if (!payload) return null;

  const knownIds = new Set(payload.tree.map((node) => node.id));
  if (objectModel.value.ref.path.toLowerCase().endsWith(".prefab")) {
    const root = payload.tree.find((node) =>
      node.hasInspector && (!node.parentId || !knownIds.has(node.parentId)),
    );
    if (root) return root.id;
  }

  return payload.targets.find((target) =>
    payload.tree.some((node) => node.id === target.id && node.hasInspector),
  )?.id ?? payload.tree.find((node) => node.hasInspector)?.id ?? null;
});

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function shortGuid(guid: string | undefined): string {
  if (!guid) return "";
  return guid.length > 10 ? `${guid.slice(0, 8)}...` : guid;
}

function compactPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 3) return path;
  return `${parts[0]}/.../${parts.slice(-2).join("/")}`;
}

function previewErrorDisplayMessage(error: string): string {
  if (!error) return "";
  return isUnityConnectionError(error) ? t("asset.preview.unityConnectionRequired") : error;
}

function toggleInspectorCollapsed() {
  inspectorCollapsed.value = !inspectorCollapsed.value;
  rememberUnityObjectPreviewExpandedState(props.previewStateKey, !inspectorCollapsed.value);
}

function handlePreviewRootClick(event: MouseEvent) {
  if (props.level === "inspector") {
    event.stopPropagation();
  }
}

async function loadStructuredTarget(previewKey: string, targetId: string) {
  const cached = targetCache.value.get(targetId);
  activeTargetId.value = targetId;
  if (cached) {
    autoTargetLoading.value = false;
    autoTargetError.value = "";
    return cached;
  }

  const run = ++autoTargetRun;
  autoTargetLoading.value = true;
  autoTargetError.value = "";
  try {
    const nextInspector = await loadWorkspaceAssetTargetCached(previewKey, targetId);
    if (run !== autoTargetRun || structuredPayload.value?.previewKey !== previewKey) return null;
    const nextCache = new Map(targetCache.value);
    nextCache.set(targetId, nextInspector);
    targetCache.value = nextCache;
    activeTargetId.value = targetId;
    return nextInspector;
  } catch (error) {
    if (run !== autoTargetRun) return null;
    autoTargetError.value = normalizeAppError(error).message;
    return null;
  } finally {
    if (run === autoTargetRun) {
      autoTargetLoading.value = false;
    }
  }
}

function handleStructuredTargetSelect(targetId: string) {
  const payload = structuredPayload.value;
  if (!payload) return;
  void loadStructuredTarget(payload.previewKey, targetId);
}

async function loadLivePropertyTree(force = false, options: { background?: boolean } = {}) {
  const target = liveSerializedTarget.value;
  if (!canAutoLoadLiveProperties.value || !target) {
    livePropertyTree.value = null;
    livePropertyLoading.value = false;
    livePropertyError.value = "";
    return null;
  }
  if (!force && livePropertyTree.value) return livePropertyTree.value;

  const run = ++livePropertyRun;
  const background = options.background === true && !!livePropertyTree.value;
  const writeVersion = editorWriteVersion;
  if (!background) livePropertyLoading.value = true;
  livePropertyError.value = "";
  try {
    const result = await readUnitySerializedProperty({
      target,
      maxDepth: 5,
      maxArrayItems: 128,
    });
    if (run !== livePropertyRun || liveSerializedTargetKey.value === "") return null;
    if (background && writeVersion !== editorWriteVersion) return livePropertyTree.value;
    livePropertyTree.value = Array.isArray(result.properties) && result.properties.length
      ? result.properties
      : result;
    return result;
  } catch (error) {
    if (run !== livePropertyRun) return null;
    if (background && writeVersion !== editorWriteVersion) return livePropertyTree.value;
    if (!background) livePropertyTree.value = null;
    livePropertyError.value = normalizeAppError(error).message;
    return null;
  } finally {
    if (run === livePropertyRun && !background) {
      livePropertyLoading.value = false;
    }
  }
}

function toUnityCommitEvent(commit: InspectorPropertyCommit): UnitySerializedPropertyCommitEvent {
  const target = commitRootTarget(commit);
  return {
    propertyPath: commit.propertyPath,
    value: commit.value,
    property: commit.snapshot as UnitySerializedPropertyCommitEvent["property"],
    target,
  };
}

function propertyTreeSnapshots(): UnityObjectPropertyTreeInput[] {
  const source = effectivePropertyTree.value;
  if (!source) return [];
  return Array.isArray(source) ? source : [source];
}

function snapshotBindingTarget(snapshot: unknown): UnitySerializedPropertyTarget | null {
  if (!snapshot || typeof snapshot !== "object") return null;
  const source = snapshot as {
    bindingTarget?: UnitySerializedPropertyTarget | null;
    target?: UnitySerializedPropertyTarget | null;
  };
  return source.bindingTarget ?? source.target ?? null;
}

function snapshotTargetKey(snapshot: unknown): string {
  return unitySerializedTargetKey(snapshotBindingTarget(snapshot));
}

function commitRootTarget(commit: InspectorPropertyCommit): UnitySerializedPropertyTarget | null {
  return snapshotBindingTarget(commit.property.root.snapshot)
    ?? snapshotBindingTarget(commit.snapshot);
}

function commitFromUnityEvent(event: UnitySerializedPropertyCommitEvent): InspectorPropertyCommit | null {
  const eventTargetKey = unitySerializedTargetKey(event.target);
  const matchedSnapshot = eventTargetKey
    ? propertyTreeSnapshots().find((snapshot) => snapshotTargetKey(snapshot) === eventTargetKey)
    : null;
  const tree = createPropertyTree(matchedSnapshot ?? effectivePropertyTree.value ?? event.property, {
    id: editorPropertyTreeBinding.value.id,
    targetId: editorPropertyTreeBinding.value.targetId,
    disabled: editorPropertyTreeBinding.value.disabled,
    readonly: editorPropertyTreeBinding.value.readonly,
  });
  const property = tree.getProperty(event.propertyPath)
    ?? tree.getProperty(event.property.propertyPath)
    ?? tree.rootProperty;
  return property ? property.createCommit(event.value) : null;
}

function editorPropertyWriteKey(targetKey: string, propertyPath: string): string {
  return `${targetKey}\u0000${propertyPath}`;
}

function clearEditorWriteFlushFrame() {
  if (editorWriteFlushTimer === null) return;
  window.clearTimeout(editorWriteFlushTimer);
  editorWriteFlushTimer = null;
}

function scheduleEditorWriteFlush(immediate = false) {
  if (editorWriteFlushTimer !== null || editorWriteInFlight || disposed) return;
  const elapsed = Date.now() - lastEditorWriteFlushAt;
  const wait = immediate ? 0 : Math.max(0, EDITOR_WRITE_MIN_INTERVAL_MS - elapsed);
  editorWriteFlushTimer = window.setTimeout(() => {
    editorWriteFlushTimer = null;
    void flushEditorWrites();
  }, wait);
}

function queueEditorWriteRefresh(writes: PendingEditorPropertyWrite[]) {
  if (disposed) return;
  const currentTargetKey = liveSerializedTargetKey.value;
  if (!currentTargetKey) return;
  if (writes.some((write) => write.refreshTargetKey === currentTargetKey)) {
    editorWriteRefreshTargetKey = currentTargetKey;
  }
}

function refreshAfterEditorWrites() {
  if (
    disposed ||
    editorWriteInFlight ||
    pendingEditorWrites.size > 0 ||
    !editorWriteRefreshTargetKey
  ) return;

  const targetKey = editorWriteRefreshTargetKey;
  editorWriteRefreshTargetKey = "";
  if (targetKey !== liveSerializedTargetKey.value) return;
  void loadLivePropertyTree(true, { background: true });
}

async function flushEditorWrites() {
  clearEditorWriteFlushFrame();
  if (editorWriteInFlight) return;

  lastEditorWriteFlushAt = Date.now();
  const writes = Array.from(pendingEditorWrites.values())
    .sort((left, right) => left.order - right.order);
  pendingEditorWrites.clear();
  if (writes.length === 0) {
    refreshAfterEditorWrites();
    return;
  }

  editorWriteInFlight = true;
  if (!disposed) livePropertyError.value = "";
  try {
    const result = await applyUnitySerializedProperties({
      writes: writes.map((write) => ({
        bindingId: write.bindingId,
        target: write.target,
        value: write.value,
        writeMode: write.writeMode,
      })),
    });
    const successfulWrites = writes.filter((write, index) =>
      write.writeMode !== "preview" && result.results[index]?.ok === true
    );
    if (successfulWrites.length > 0) queueEditorWriteRefresh(successfulWrites);
    const failed = result.results.find((item) => !item.ok);
    if (!result.ok || failed) {
      throw new Error(failed?.message || result.message || "Failed to apply serialized property writes.");
    }
  } catch (error) {
    if (!disposed) {
      const message = normalizeAppError(error).message;
      if (writes.some((write) => write.writeMode !== "preview")) {
        livePropertyError.value = message;
        emit("blocked", objectModel.value);
      } else {
        console.warn("[UnityObjectPreview] preview write failed:", message);
      }
    }
  } finally {
    editorWriteInFlight = false;
    if (pendingEditorWrites.size > 0) {
      if (disposed) {
        void flushEditorWrites();
      } else {
        scheduleEditorWriteFlush();
      }
    } else {
      refreshAfterEditorWrites();
    }
  }
}

function commitEditorPropertyTree(
  commit: InspectorPropertyCommit,
  writeMode: PendingEditorPropertyWrite["writeMode"] = "commit",
) {
  const target = commitRootTarget(commit) ?? liveSerializedTarget.value;
  if (!target) {
    emit("commit", toUnityCommitEvent(commit));
    return;
  }

  const propertyPath = commit.propertyPath.trim();
  const targetKey = unitySerializedTargetKey(target);
  const refreshTargetKey = liveSerializedTargetKey.value;
  if (!propertyPath || !targetKey || !refreshTargetKey) {
    emit("blocked", objectModel.value);
    return;
  }

  editorWriteVersion += 1;
  pendingEditorWrites.set(editorPropertyWriteKey(targetKey, propertyPath), {
    targetKey,
    refreshTargetKey,
    bindingId: editorPropertyTreeBinding.value.id,
    target: unitySerializedTargetWithProperty(target, propertyPath),
    value: commit.value,
    writeMode,
    order: ++editorWriteOrder,
  });
  scheduleEditorWriteFlush(writeMode === "commit");
}

function handleEditorCommit(event: UnitySerializedPropertyCommitEvent) {
  const commit = commitFromUnityEvent(event);
  if (!commit) {
    emit("blocked", objectModel.value);
    return;
  }
  void commitEditorPropertyTree(commit, "commit");
}

function handleEditorPreview(event: UnitySerializedPropertyCommitEvent) {
  const commit = commitFromUnityEvent(event);
  if (!commit) return;
  void commitEditorPropertyTree(commit, "preview");
}

function handleThumbnailImageError() {
  if (interactiveFrame.value) {
    interactiveFrame.value = null;
    return;
  }
  autoThumbnail.value = null;
}

function clearInteractiveFrameTimer() {
  if (interactiveFrameTimer === null) return;
  window.clearTimeout(interactiveFrameTimer);
  interactiveFrameTimer = null;
}

function clearInteractiveFrameCacheTimer() {
  if (interactiveFrameCacheTimer === null) return;
  window.clearTimeout(interactiveFrameCacheTimer);
  interactiveFrameCacheTimer = null;
}

function resetInteractivePreviewState() {
  clearInteractiveFrameTimer();
  clearInteractiveFrameCacheTimer();
  stopInteractiveKeyboardMoveLoop();
  removeInteractiveKeyboardListeners();
  interactiveFrame.value = null;
  interactiveFrameLoading.value = false;
  interactiveFrameError.value = "";
  interactiveYaw.value = 25;
  interactivePitch.value = -12;
  interactiveDistance.value = 1.15;
  interactivePanX.value = 0;
  interactivePanY.value = 0;
  interactivePanZ.value = 0;
  interactiveDragging.value = false;
  interactiveFrameInFlight = false;
  interactiveFrameQueued = false;
  interactiveMoveKeys.clear();
  interactivePointerId = null;
  lastInteractiveFrameAt = 0;
}

function scheduleInteractiveFrameRender(immediate = false) {
  if (!canAutoLoadInteractivePreview.value || !autoPreviewPath.value) return;
  if (interactiveFrameTimer !== null) return;

  const elapsed = Date.now() - lastInteractiveFrameAt;
  const wait = immediate ? 0 : Math.max(0, INTERACTIVE_FRAME_MIN_INTERVAL_MS - elapsed);
  interactiveFrameTimer = window.setTimeout(() => {
    interactiveFrameTimer = null;
    void renderInteractiveFrame();
  }, wait);
}

async function renderInteractiveFrame() {
  if (!canAutoLoadInteractivePreview.value || !autoPreviewPath.value) return;
  if (interactiveFrameInFlight) {
    interactiveFrameQueued = true;
    return;
  }

  const run = interactiveFrameRun;
  const path = autoPreviewPath.value;
  const request = {
    width: 360,
    height: 240,
    yaw: interactiveYaw.value,
    pitch: interactivePitch.value,
    distance: interactiveDistance.value,
    panX: interactivePanX.value,
    panY: interactivePanY.value,
    panZ: interactivePanZ.value,
  };

  interactiveFrameInFlight = true;
  interactiveFrameLoading.value = true;
  interactiveFrameError.value = "";
  try {
    const frame = await renderWorkspaceAssetPreviewFrame(path, request);
    if (run !== interactiveFrameRun || path !== autoPreviewPath.value) return;
    await decodeInteractiveFrame(frame.url);
    if (run !== interactiveFrameRun || path !== autoPreviewPath.value) return;
    interactiveFrame.value = frame;
    scheduleInteractiveFrameCache(frame);
  } catch (error) {
    if (run !== interactiveFrameRun) return;
    interactiveFrameError.value = normalizeAppError(error).message;
  } finally {
    if (run === interactiveFrameRun) {
      interactiveFrameInFlight = false;
      interactiveFrameLoading.value = false;
      lastInteractiveFrameAt = Date.now();
      if (interactiveFrameQueued) {
        interactiveFrameQueued = false;
        scheduleInteractiveFrameRender(false);
      }
    }
  }
}

function decodeInteractiveFrame(url: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.decoding = "async";
    image.onload = () => {
      resolve();
    };
    image.onerror = () => {
      reject(new Error("Interactive preview frame failed to decode."));
    };
    image.src = url;
  });
}

function applyInteractiveFrameState(frame: WorkspaceAssetPreviewFrame) {
  interactiveYaw.value = frame.yaw ?? interactiveYaw.value;
  interactivePitch.value = frame.pitch ?? interactivePitch.value;
  interactiveDistance.value = frame.distance ?? interactiveDistance.value;
  interactivePanX.value = frame.panX ?? interactivePanX.value;
  interactivePanY.value = frame.panY ?? interactivePanY.value;
  interactivePanZ.value = frame.panZ ?? interactivePanZ.value;
}

function scheduleInteractiveFrameCache(frame: WorkspaceAssetPreviewFrame) {
  if (!canAutoLoadInteractivePreview.value || !autoPreviewPath.value) return;
  clearInteractiveFrameCacheTimer();
  interactiveFrameCacheTimer = window.setTimeout(() => {
    interactiveFrameCacheTimer = null;
    void cacheInteractiveFrame(frame);
  }, INTERACTIVE_FRAME_CACHE_DELAY_MS);
}

async function cacheInteractiveFrame(frame: WorkspaceAssetPreviewFrame) {
  const path = autoPreviewPath.value;
  if (!path || !canAutoLoadInteractivePreview.value) return;
  try {
    await cacheWorkspaceAssetPreviewFrame(path, {
      ...frame,
      yaw: frame.yaw ?? interactiveYaw.value,
      pitch: frame.pitch ?? interactivePitch.value,
      distance: frame.distance ?? interactiveDistance.value,
      panX: frame.panX ?? interactivePanX.value,
      panY: frame.panY ?? interactivePanY.value,
      panZ: frame.panZ ?? interactivePanZ.value,
    });
    rememberWorkspaceAssetPreviewFrameCache(path, frame);
  } catch (error) {
    console.warn("[UnityObjectPreview] failed to cache preview frame:", error);
  }
}

function handleInteractivePreviewPointerDown(event: PointerEvent) {
  if (!canAutoLoadInteractivePreview.value) return;
  if (!event.isPrimary || event.button !== 0 || (event.buttons & 1) !== 1) return;
  interactiveDragging.value = true;
  interactivePointerId = event.pointerId;
  interactivePointerX = event.clientX;
  interactivePointerY = event.clientY;
  addInteractiveKeyboardListeners();
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  event.preventDefault();
}

function handleInteractivePreviewPointerMove(event: PointerEvent) {
  if (!interactiveDragging.value || interactivePointerId !== event.pointerId) return;
  if ((event.buttons & 1) !== 1) {
    stopInteractivePreviewDrag(event);
    return;
  }

  const dx = event.clientX - interactivePointerX;
  const dy = event.clientY - interactivePointerY;
  interactivePointerX = event.clientX;
  interactivePointerY = event.clientY;
  interactiveYaw.value = (interactiveYaw.value - dx * 0.45) % 360;
  interactivePitch.value = Math.max(-70, Math.min(70, interactivePitch.value + dy * 0.35));
  scheduleInteractiveFrameRender(false);
  event.preventDefault();
}

function handleInteractivePreviewPointerUp(event: PointerEvent) {
  if (interactivePointerId !== event.pointerId) return;
  stopInteractivePreviewDrag(event);
}

function stopInteractivePreviewDrag(event?: PointerEvent) {
  interactiveDragging.value = false;
  const pointerId = interactivePointerId;
  interactivePointerId = null;
  interactiveMoveKeys.clear();
  stopInteractiveKeyboardMoveLoop();
  removeInteractiveKeyboardListeners();
  if (event && pointerId !== null) {
    (event.currentTarget as HTMLElement | null)?.releasePointerCapture?.(pointerId);
  }
}

function interactiveMoveKey(key: string): string {
  const normalized = key.trim().toLowerCase();
  return /^[wasdqe]$/.test(normalized) ? normalized : "";
}

function addInteractiveKeyboardListeners() {
  window.addEventListener("keydown", handleInteractivePreviewKeyDown);
  window.addEventListener("keyup", handleInteractivePreviewKeyUp);
}

function removeInteractiveKeyboardListeners() {
  window.removeEventListener("keydown", handleInteractivePreviewKeyDown);
  window.removeEventListener("keyup", handleInteractivePreviewKeyUp);
}

function handleInteractivePreviewKeyDown(event: KeyboardEvent) {
  if (!interactiveDragging.value) return;
  const key = interactiveMoveKey(event.key);
  if (!key) return;
  interactiveMoveKeys.add(key);
  startInteractiveKeyboardMoveLoop();
  event.preventDefault();
}

function handleInteractivePreviewKeyUp(event: KeyboardEvent) {
  const key = interactiveMoveKey(event.key);
  if (!key) return;
  interactiveMoveKeys.delete(key);
  if (!interactiveMoveKeys.size) {
    stopInteractiveKeyboardMoveLoop();
  }
  event.preventDefault();
}

function startInteractiveKeyboardMoveLoop() {
  if (interactiveKeyboardFrame !== null) return;
  interactiveKeyboardLastAt = performance.now();
  const tick = (now: number) => {
    interactiveKeyboardFrame = null;
    if (!interactiveDragging.value || !interactiveMoveKeys.size) return;

    const dt = Math.min(0.12, Math.max(0, (now - interactiveKeyboardLastAt) / 1000));
    interactiveKeyboardLastAt = now;
    const step = INTERACTIVE_KEY_MOVE_SPEED * dt;
    const horizontal = (interactiveMoveKeys.has("d") ? 1 : 0) - (interactiveMoveKeys.has("a") ? 1 : 0);
    const forward = (interactiveMoveKeys.has("w") ? 1 : 0) - (interactiveMoveKeys.has("s") ? 1 : 0);
    const vertical = (interactiveMoveKeys.has("e") ? 1 : 0) - (interactiveMoveKeys.has("q") ? 1 : 0);

    if (horizontal || forward || vertical) {
      interactivePanX.value = Math.max(-8, Math.min(8, interactivePanX.value + horizontal * step));
      interactivePanY.value = Math.max(-8, Math.min(8, interactivePanY.value + vertical * step));
      interactivePanZ.value = Math.max(-8, Math.min(8, interactivePanZ.value + forward * step));
      scheduleInteractiveFrameRender(false);
    }

    interactiveKeyboardFrame = window.requestAnimationFrame(tick);
  };
  interactiveKeyboardFrame = window.requestAnimationFrame(tick);
}

function stopInteractiveKeyboardMoveLoop() {
  if (interactiveKeyboardFrame === null) return;
  window.cancelAnimationFrame(interactiveKeyboardFrame);
  interactiveKeyboardFrame = null;
}

function handleInteractivePreviewWheel(event: WheelEvent) {
  if (!canAutoLoadInteractivePreview.value) return;
  const nextDistance = interactiveDistance.value + Math.sign(event.deltaY) * 0.08;
  interactiveDistance.value = Math.max(0.75, Math.min(2.8, nextDistance));
  scheduleInteractiveFrameRender(false);
  event.preventDefault();
}

watch(
  previewSourceState,
  (state) => {
    emit("source-change", state);
  },
  { immediate: true },
);

watch(
  () => [canAutoLoadPreview.value, autoPreviewPath.value] as const,
  async ([canLoad, path]) => {
    const run = ++autoPreviewRun;
    autoPreviewError.value = "";
    autoPreviewPayload.value = null;
    autoPreviewLoading.value = false;
    activeTargetId.value = null;
    targetCache.value = new Map();
    autoTargetLoading.value = false;
    autoTargetError.value = "";
    if (!canLoad || !path) return;

    autoPreviewLoading.value = true;
    try {
      const payload = await loadWorkspaceAssetPreviewCached(path);
      if (run !== autoPreviewRun) return;
      autoPreviewPayload.value = payload;
    } catch (error) {
      if (run !== autoPreviewRun) return;
      autoPreviewError.value = normalizeAppError(error).message;
    } finally {
      if (run === autoPreviewRun) {
        autoPreviewLoading.value = false;
      }
    }
  },
  { immediate: true },
);

watch(
  () => [canAutoLoadThumbnail.value, autoPreviewPath.value] as const,
  async ([canLoad, path]) => {
    const run = ++autoThumbnailRun;
    autoThumbnail.value = null;
    autoThumbnailLoading.value = false;
    if (!canLoad || !path) return;

    autoThumbnailLoading.value = true;
    try {
      const thumbnail = await loadWorkspaceAssetThumbnailCached(path);
      if (run !== autoThumbnailRun) return;
      autoThumbnail.value = thumbnail;
    } catch {
      if (run !== autoThumbnailRun) return;
    } finally {
      if (run === autoThumbnailRun) {
        autoThumbnailLoading.value = false;
      }
    }
  },
  { immediate: true },
);

watch(
  () => [canAutoLoadInteractivePreview.value, autoPreviewPath.value] as const,
  async ([canLoad, path]) => {
    const run = ++interactiveFrameRun;
    resetInteractivePreviewState();
    if (!canLoad || !path) return;
    try {
      const frame = await loadWorkspaceAssetPreviewFrameCacheCached(path);
      if (run !== interactiveFrameRun || path !== autoPreviewPath.value || !frame) return;
      await decodeInteractiveFrame(frame.url);
      if (run !== interactiveFrameRun || path !== autoPreviewPath.value) return;
      applyInteractiveFrameState(frame);
      interactiveFrame.value = frame;
    } catch {
      if (run !== interactiveFrameRun) return;
    }
  },
  { immediate: true },
);

watch(
  () => [canAutoLoadLiveProperties.value, liveSerializedTargetKey.value] as const,
  ([canLoad]) => {
    livePropertyRun++;
    livePropertyTree.value = null;
    livePropertyLoading.value = false;
    livePropertyError.value = "";
    if (canLoad) void loadLivePropertyTree(true);
  },
  { immediate: true },
);

watch(
  () => structuredPayload.value?.previewKey ?? "",
  () => {
    activeTargetId.value = null;
    targetCache.value = new Map();
    autoTargetLoading.value = false;
    autoTargetError.value = "";
  },
);

watch(
  () => [props.previewStateKey, props.level] as const,
  ([previewStateKey, level]) => {
    if (level !== "inspector") return;
    const expanded = readUnityObjectPreviewExpandedState(previewStateKey);
    if (expanded === null) return;
    inspectorCollapsed.value = !expanded;
  },
  { immediate: true },
);

watch(
  () => [props.level, structuredPayload.value?.previewKey ?? "", defaultStructuredTargetId.value ?? ""] as const,
  ([level, previewKey, targetId]) => {
    if (level !== "inspector" || !previewKey || !targetId || inspector.value) return;
    void loadStructuredTarget(previewKey, targetId);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  disposed = true;
  clearEditorWriteFlushFrame();
  if (pendingEditorWrites.size > 0) {
    void flushEditorWrites();
  }
  if (interactiveFrame.value) {
    void cacheInteractiveFrame(interactiveFrame.value);
  }
  interactiveFrameRun++;
  livePropertyRun++;
  resetInteractivePreviewState();
});
</script>

<template>
  <component
    :is="objectDrawerComponent"
    v-if="objectDrawerComponent"
    :model="objectModel"
    :level="level"
    :context="objectDrawerContext"
    :selected="selected"
    :disabled="disabled"
    :readonly="readonly"
    :draggable="draggable"
    :loading="loadingState"
    :error="errorState"
    @select="emit('select', $event)"
    @preview="handleEditorPreview"
    @commit="handleEditorCommit"
    @blocked="emit('blocked', $event)"
  >
    <UnityObjectPreview
      :model="model"
      :level="level"
      :loading="loading"
      :error="error"
      :selected="selected"
      :disabled="disabled"
      :readonly="readonly"
      :draggable="draggable"
      :diff-key="diffKey"
      :display-mode="displayMode"
      :target-loading="targetLoading"
      :include-unchanged="includeUnchanged"
      :property-drawers="propertyDrawers"
      :object-drawers="objectDrawers"
      :auto-load-preview="autoLoadPreview"
      :preview-state-key="previewStateKey"
      disable-object-drawer
      @select="emit('select', $event)"
      @preview="handleEditorPreview"
      @commit="handleEditorCommit"
      @blocked="emit('blocked', $event)"
      @source-change="emit('source-change', $event)"
    />
  </component>

  <UnityObjectIdentity
    v-else-if="level === 'inline' || level === 'row'"
    :model="objectModel"
    :mode="level === 'row' ? 'row' : 'inline'"
    :selected="selected"
    :disabled="disabled"
    :interactive="objectModel.capabilities.select"
    :draggable="draggable"
    :show-path="level === 'row'"
    @select="emit('select', $event)"
  />

  <UnityObjectEditorPanel
    v-else-if="level === 'editor'"
    :model="liveEditorModel"
    :property-tree="editorPropertyTreeBinding"
    :disabled="disabled || livePropertyLoading"
    :readonly="readonly"
    :property-drawers="propertyDrawers"
    @preview="handleEditorPreview"
    @commit="handleEditorCommit"
    @blocked="emit('blocked', $event)"
  />

  <section v-else class="unity-object-preview" :class="previewClass" @click="handlePreviewRootClick">
    <div v-if="level === 'inspector'" class="unity-object-inspector-header">
      <button
        type="button"
        class="unity-object-inspector-fold"
        :class="{ collapsed: inspectorCollapsed }"
        :aria-expanded="!inspectorCollapsed"
        @click.stop="toggleInspectorCollapsed"
      >
        <span aria-hidden="true">▶</span>
      </button>
      <UnityObjectIdentity
        :model="objectModel"
        mode="row"
        :draggable="objectModel.capabilities.drag"
        :highlightable="objectModel.capabilities.drag"
        :show-path="true"
      />
    </div>

    <div
      v-if="level === 'inspector' && inspectorCollapsed"
      class="unity-object-inspector-collapsed-body"
    />

    <template v-else-if="level === 'inspector' && (effectivePropertyTree || livePropertyLoading || livePropertyError)">
      <UnityObjectEditorPanel
        v-if="effectivePropertyTree || livePropertyLoading"
        :model="liveEditorModel"
        :property-tree="editorPropertyTreeBinding"
        :disabled="disabled || livePropertyLoading"
        :readonly="readonly"
        :show-header="false"
        :property-drawers="propertyDrawers"
        @preview="handleEditorPreview"
        @commit="handleEditorCommit"
        @blocked="emit('blocked', $event)"
      />
      <div
        v-else
        class="unity-object-preview-state"
        :class="{ error: !livePropertyErrorRequiresUnity }"
      >
        {{ livePropertyErrorState }}
      </div>
    </template>

    <template v-else-if="level === 'thumbnail'">
      <div v-if="thumbnailLoadingState" class="unity-object-preview-state">Loading thumbnail...</div>
      <div v-else-if="interactivePreviewUrl" class="unity-object-thumbnail-image">
        <div class="unity-object-thumbnail-header">
          <UnityObjectIdentity
            :model="objectModel"
            mode="row"
            :draggable="objectModel.capabilities.drag"
            :highlightable="objectModel.capabilities.drag"
            :show-path="true"
          />
        </div>
        <div
          class="unity-object-interactive-preview"
          :class="{
            enabled: canAutoLoadInteractivePreview,
            dragging: interactiveDragging,
            loading: interactiveFrameLoading,
          }"
          @pointerdown="handleInteractivePreviewPointerDown"
          @pointermove="handleInteractivePreviewPointerMove"
          @pointerup="handleInteractivePreviewPointerUp"
          @pointercancel="handleInteractivePreviewPointerUp"
          @wheel="handleInteractivePreviewWheel"
          @click.stop.prevent
        >
          <img
            :src="interactivePreviewUrl"
            :alt="objectModel.title"
            draggable="false"
            @error="handleThumbnailImageError"
          >
        </div>
      </div>
      <div
        v-else-if="binaryPreviewPayload && canRenderBinaryThumbnail"
        class="unity-object-thumbnail-binary"
      >
        <div class="unity-object-thumbnail-header">
          <UnityObjectIdentity
            :model="objectModel"
            mode="row"
            :draggable="objectModel.capabilities.drag"
            :highlightable="objectModel.capabilities.drag"
            :show-path="true"
          />
        </div>
        <div class="unity-object-thumbnail-preview-body">
          <BinaryPreviewHost
            :preview="binaryPreviewPayload.preview"
            :compact="true"
            :diff-key="effectiveDiffKey"
            mode="neutral"
            :asset-meta="binaryPreviewPayload.meta"
            :unity-texture-meta="binaryPreviewPayload.meta.unityTexture"
          />
        </div>
      </div>
      <div v-else class="unity-object-thumbnail-row-fallback">
        <UnityObjectIdentity
          :model="objectModel"
          mode="row"
          :draggable="objectModel.capabilities.drag"
          :highlightable="objectModel.capabilities.drag"
          :show-path="true"
        />
      </div>
    </template>

    <div v-else-if="loadingState && !previewPayload" class="unity-object-preview-state">Loading preview...</div>
    <div v-else-if="errorState && !previewPayload" class="unity-object-preview-summary">
      <UnityObjectIdentity :model="objectModel" mode="row" :draggable="false" />
      <div
        class="unity-object-preview-error"
        :class="{ neutral: errorStateRequiresUnity }"
      >
        {{ errorState }}
      </div>
    </div>

    <template v-else-if="binaryPreviewPayload && shouldRenderBinaryPreviewHost">
      <BinaryPreviewHost
        :preview="binaryPreviewPayload.preview"
        :compact="compactPreview"
        :diff-key="effectiveDiffKey"
        mode="neutral"
        :asset-meta="binaryPreviewPayload.meta"
        :unity-texture-meta="binaryPreviewPayload.meta.unityTexture"
      />
    </template>

    <template v-else-if="binaryPreviewPayload && compactBinaryMeta">
      <div class="unity-object-preview-summary">
        <UnityObjectIdentity :model="objectModel" mode="row" :draggable="false" />
        <div class="unity-object-preview-meta">
          <span>{{ compactBinaryKindLabel }}</span>
          <span>{{ formatBytes(compactBinaryMeta.size) }}</span>
          <span v-if="compactBinaryMeta.guid" :title="compactBinaryMeta.guid">
            {{ shortGuid(compactBinaryMeta.guid) }}
          </span>
        </div>
        <div class="unity-object-preview-path" :title="compactBinaryMeta.path">
          {{ compactPath(compactBinaryMeta.path) }}
        </div>
      </div>
    </template>

    <template v-else-if="structuredPayload">
      <div v-if="level === 'inspector'" class="unity-object-structured-inspector">
        <div v-if="showStructuredTargetSelector" class="unity-object-structured-tree">
          <UnityHierarchyPane
            :nodes="structuredPayload.tree"
            :selected-id="activeTargetId"
            :show-collapse-all="true"
            :auto-collapse-when-overflow="structuredPayload.layout === 'sceneHierarchyInspector'"
            hide-title
            @select="handleStructuredTargetSelect"
          />
        </div>
        <div
          class="unity-object-structured-detail"
          :class="{ 'full-width': !showStructuredTargetSelector }"
        >
          <UnityInspectorPane
            v-if="inspector || targetLoadingState"
            :inspector="inspector"
            :loading="targetLoadingState"
            :include-unchanged="includeUnchanged"
            :display-mode="displayMode"
            header-layout="unity"
            hide-toolbar
          />
          <div
            v-else-if="targetErrorState"
            class="unity-object-preview-state"
            :class="{ error: !targetErrorRequiresUnity }"
          >
            {{ targetErrorState }}
          </div>
          <div v-else class="unity-object-preview-state">Select an object</div>
        </div>
      </div>
      <div v-else class="unity-object-preview-summary">
        <UnityObjectIdentity :model="objectModel" mode="row" :draggable="false" />
        <div class="unity-object-preview-meta">
          <span v-for="item in structuredSummaryMeta" :key="item">{{ item }}</span>
        </div>
        <div v-if="structuredSummaryTargets.length" class="unity-object-targets">
          <span
            v-for="target in structuredSummaryTargets"
            :key="target.id"
            class="unity-object-target"
            :title="target.subtitle || target.title"
          >
            {{ target.title }}
          </span>
        </div>
      </div>
    </template>

    <template v-else-if="inspector">
      <UnityInspectorPane
        v-if="level === 'inspector'"
        :inspector="inspector"
        :loading="targetLoadingState"
        :include-unchanged="includeUnchanged"
        :display-mode="displayMode"
        header-layout="unity"
        hide-toolbar
      />
      <div v-else class="unity-object-preview-placeholder">
        <UnityObjectIdentity
          :model="objectModel"
          mode="row"
          :draggable="false"
        />
      </div>
    </template>

    <template v-else-if="textPayload">
      <pre v-if="level === 'inspector'" class="unity-object-text-preview">{{ textPayload.snippet }}</pre>
      <div v-else class="unity-object-text-summary">
        <UnityObjectIdentity :model="objectModel" mode="row" :draggable="false" />
        <pre class="unity-object-text-lines"><span
          v-for="(line, index) in compactTextLines"
          :key="index"
        >{{ line || " " }}
</span></pre>
        <div class="unity-object-preview-meta">
          <span>{{ textPayload.totalLines }} lines</span>
          <span v-if="textPayload.truncated">truncated</span>
          <span v-if="textPayload.language">{{ textPayload.language }}</span>
        </div>
      </div>
    </template>

    <template v-else-if="binaryInfoPayload">
      <div class="unity-object-preview-summary">
        <UnityObjectIdentity :model="objectModel" mode="row" :draggable="false" />
        <div class="unity-object-preview-meta">
          <span>{{ binaryInfoPayload.meta.ext || "file" }}</span>
          <span>{{ formatBytes(binaryInfoPayload.meta.size) }}</span>
          <span v-if="binaryInfoPayload.meta.guid" :title="binaryInfoPayload.meta.guid">
            {{ shortGuid(binaryInfoPayload.meta.guid) }}
          </span>
        </div>
        <div class="unity-object-preview-path" :title="binaryInfoPayload.meta.path">
          {{ compactPath(binaryInfoPayload.meta.path) }}
        </div>
      </div>
    </template>

    <div v-else class="unity-object-preview-summary">
      <UnityObjectIdentity
        :model="objectModel"
        mode="row"
        :draggable="false"
      />
      <div class="unity-object-preview-meta">
        <span v-if="objectModel.subtitle">{{ objectModel.subtitle }}</span>
        <span v-if="objectModel.readonlyReason">{{ objectModel.readonlyReason }}</span>
      </div>
      <div v-if="objectModel.ref.path" class="unity-object-preview-path" :title="objectModel.ref.path">
        {{ compactPath(objectModel.ref.path) }}
      </div>
    </div>
  </section>
</template>

<style scoped>
.unity-object-preview {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
}

.unity-object-preview.level-thumbnail {
  width: 100%;
  min-height: 96px;
  aspect-ratio: 16 / 9;
}

.unity-object-preview.level-thumbnail.compact-summary {
  min-height: 0;
  aspect-ratio: auto;
}

.unity-object-preview.level-inspector {
  height: 100%;
}

.unity-object-preview.level-inspector.inspector-collapsed {
  height: auto;
  min-height: 0;
}

.unity-object-inspector-header {
  flex: 0 0 auto;
  min-width: 0;
  display: flex;
  align-items: stretch;
  overflow: hidden;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
}

.unity-object-preview.inspector-collapsed .unity-object-inspector-header {
  border-bottom: 0;
}

.unity-object-inspector-fold {
  flex: 0 0 28px;
  width: 28px;
  min-height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  cursor: pointer;
}

.unity-object-inspector-fold:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.unity-object-inspector-fold:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.unity-object-inspector-fold span {
  display: inline-block;
  font-size: 10px;
  transform: rotate(90deg);
  transition: transform 120ms ease;
}

.unity-object-inspector-fold.collapsed span {
  transform: rotate(0deg);
}

.unity-object-inspector-header :deep(.unity-object-identity.mode-row) {
  min-height: 30px;
  padding: 4px 8px 4px 6px;
}

.unity-object-inspector-collapsed-body {
  display: none;
}

.unity-object-thumbnail-image,
.unity-object-thumbnail-binary {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.unity-object-thumbnail-header {
  flex: 0 0 auto;
  min-width: 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
}

.unity-object-thumbnail-header :deep(.unity-object-identity.mode-row) {
  min-height: 26px;
  padding: 3px 8px;
}

.unity-object-interactive-preview {
  flex: 1 1 0;
  min-height: 0;
  width: 100%;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  overflow: hidden;
  touch-action: none;
  user-select: none;
}

.unity-object-interactive-preview.dragging {
  cursor: grabbing;
}

.unity-object-interactive-preview.loading {
  opacity: 0.92;
}

.unity-object-interactive-preview img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  pointer-events: none;
}

.unity-object-thumbnail-preview-body {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.unity-object-thumbnail-row-fallback {
  flex: 0 0 auto;
  min-width: 0;
}

.unity-object-preview-state,
.unity-object-preview-placeholder {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.unity-object-preview-state.error {
  color: var(--status-danger-fg);
}

.unity-object-preview-summary,
.unity-object-text-summary {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 7px;
  padding: 10px 12px;
}

.unity-object-preview.level-thumbnail.compact-summary .unity-object-preview-summary,
.unity-object-preview.level-thumbnail.compact-summary .unity-object-text-summary {
  flex: 0 0 auto;
}

.unity-object-preview-meta {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding-left: 28px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.35;
}

.unity-object-preview-meta span {
  min-width: 0;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-object-preview-path {
  min-width: 0;
  padding-left: 28px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.unity-object-preview-error {
  padding-left: 28px;
  color: var(--status-danger-fg);
  font-size: 11px;
  line-height: 1.4;
}

.unity-object-preview-error.neutral {
  color: var(--text-secondary);
}

.unity-object-targets {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding-left: 28px;
}

.unity-object-target {
  max-width: 140px;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.35;
}

.unity-object-structured-inspector {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.unity-object-structured-tree {
  width: 34%;
  min-width: 180px;
  max-width: 280px;
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.unity-object-structured-detail {
  flex: 1;
  min-width: 0;
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.unity-object-structured-detail.full-width {
  width: 100%;
}

.unity-object-text-preview {
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 12px;
  overflow: auto;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--bg-color) 14%);
  color: var(--text-color);
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.45;
}

.unity-object-text-lines {
  max-height: 112px;
  margin: 0;
  padding: 6px 8px;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 5px;
  background: color-mix(in srgb, var(--panel-bg) 80%, var(--bg-color) 20%);
  color: var(--text-secondary);
  font-family: var(--font-mono-block);
  font-size: 11px;
  line-height: 1.45;
}

.unity-object-text-lines span {
  display: block;
}
</style>
