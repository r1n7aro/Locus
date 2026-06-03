import {
  defineComponent,
  computed,
  h,
  markRaw,
  nextTick,
  onMounted,
  onBeforeUnmount,
  onErrorCaptured,
  reactive,
  readonly,
  ref,
  shallowRef,
  type Component,
  type ComputedRef,
  type PropType,
} from "vue";
import * as VueRuntime from "vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import {
  CanvasView,
  type CanvasClipboardEvent,
  type CanvasContextMenuEvent,
  type CanvasEditBehavior,
  type CanvasItem,
  type CanvasItemMoveEvent,
  type CanvasPoint,
  type CanvasSelectionEvent,
  type CanvasViewExpose,
  type CanvasViewport,
} from "../canvas";
import UnityBoolField from "../unity/UnityBoolField.vue";
import UnityColorField from "../unity/UnityColorField.vue";
import UnityEnumField from "../unity/UnityEnumField.vue";
import UnityFlagsField from "../unity/UnityFlagsField.vue";
import UnityLayerMaskField from "../unity/UnityLayerMaskField.vue";
import UnityNumberField from "../unity/UnityNumberField.vue";
import UnityObjectReferenceField from "../unity/UnityObjectReferenceField.vue";
import UnityPropertyDraw from "../unity/UnityPropertyDraw.vue";
import UnityPropertyEditor from "../unity/UnityPropertyEditor.vue";
import UnitySerializedPropertyTree from "../unity/UnitySerializedPropertyTree.vue";
import UnityVectorField from "../unity/UnityVectorField.vue";
import UnityObjectPreview from "../unity-preview/UnityObjectPreview.vue";
import {
  applyUnityRgbHexToColorText,
  formatUnityColorValue,
  formatUnityVectorValue,
  isUnityIntegerPropertyType,
  isUnityNumberPropertyType,
  isUnityVectorPropertyType,
  normalizeUnityOptions,
  normalizeUnityPropertyType,
  parseUnityColorValue,
  parseUnitySerializedEditValue,
  parseUnityVectorValue,
  tryParseUnitySerializedEditValue,
  unityColorTextToRgbHex,
  unityEnumIndexValue,
  unityEnumNumericValue,
  unitySerializedValueToEditText,
  unityVectorKeysForType,
} from "../unity/unitySerializedValue";
import type {
  ViewBindingApplyRequest,
  ViewBindingApplyResult,
  ViewBindingApplyWrite,
  ViewBindingDiscoverRequest,
  ViewBindingDiscoverResult,
  ViewBindingReadRequest,
  ViewBindingReadResult,
  ViewBindingWriteRequest,
  ViewBindingWriteResult,
  ViewCallScriptResult,
  ViewFrontendLogEntry,
  ViewLlmCallRequest,
  ViewLlmCallResult,
  ViewPackageDetail,
  ViewPackageFile,
  ViewSessionChatRequest,
  ViewSessionChatResult,
  ViewSessionCreateRequest,
  ViewSessionQueueInputRequest,
  ViewSessionQueueInputResult,
  ViewSessionWaitRequest,
  ViewSessionWaitResult,
  ViewSerializedPropertySnapshot,
  ViewRuntimeUpdateEvent,
} from "../../services/view";
import type {
  UnitySerializedPropertyApplyRequest,
  UnitySerializedPropertyApplyResult,
  UnitySerializedPropertyDiscoverRequest,
  UnitySerializedPropertyDiscoverResult,
  UnitySerializedPropertyReadRequest,
  UnitySerializedPropertyReadResult,
  UnitySerializedPropertyWriteRequest,
  UnitySerializedPropertyWriteResult,
} from "../../services/unitySerializedProperty";
import {
  checkUnityConnectionStatus,
  commitUnityEmbedAssetDrop,
  openUnityAssetInspector,
  openUnitySceneObjectInspector,
  selectUnityAsset,
  selectUnitySceneObject,
  subscribeLocusFileDragState,
  subscribeLocusFileDrop,
  subscribeUnityEmbedAssetDragState,
  subscribeUnityEmbedAssetDrop,
  type LocusFileDragStatePayload,
  type LocusFileDropPayload,
  type LocusFileDropRef,
  type UnityEmbedAssetDragStatePayload,
  type UnityEmbedAssetDropPayload,
} from "../../services/unity";
import {
  armLocusFilePointerDrag,
  armUnityReferencePointerDrag,
  startLocusFileHtmlDrag,
  startUnityReferenceHtmlDrag,
} from "../../composables/useUnityReferenceDragSource";
import { useUnityAssetDropTarget as useUnityAssetDropTargetBase } from "../../composables/useUnityAssetDropTarget";
import {
  sanitizeCssForPreview,
  viewPackageRelPath,
  viewFileContent,
} from "./viewPackageFiles";
import type { ViewSfcCompileResult } from "./viewCompiler";
import {
  GraphView,
  GraphViewController,
  defineGraphView,
  layoutGraphDocument,
  type GraphConnectionValidation,
  type GraphController,
  type GraphData,
  type GraphEndpoint,
  type GraphLayoutOptions,
  type GraphLink,
  type GraphNode,
  type GraphParameter,
  type GraphParameterOption,
  type GraphParameterType,
  type GraphPort,
  type GraphPortDirection,
} from "../graph";
import type {
  AssetRefAttachment,
  AssetRefKind,
  AssetSearchResult,
  SessionDetail,
  SessionEventRecord,
  SessionRunSummary,
  StreamEvent,
} from "../../types";
import * as PropertyTreeService from "../../services/propertyTree";
import * as UnityObjectDrawerService from "../../services/unityObjectDrawer";
import * as UnityObjectReferencePickerService from "../../services/unityObjectReferencePicker";
import { markStartupPhase } from "../../services/startupPerf";

export type {
  ViewLlmCallRequest,
  ViewLlmCallResult,
  ViewSessionChatRequest,
  ViewSessionChatResult,
  ViewSessionCreateRequest,
  ViewSessionQueueInputRequest,
  ViewSessionQueueInputResult,
  ViewSessionWaitRequest,
  ViewSessionWaitResult,
  ViewBindingDiscoverMatch,
  ViewBindingDiscoverRequest,
  ViewBindingDiscoverResult,
} from "../../services/view";

export type {
  TransformResult,
  ViewCompileDiagnostic,
} from "./viewCompiler";
export { CanvasView } from "../canvas";
export {
  GraphView,
  GraphViewController,
  defineGraphView,
  layoutGraphDocument,
} from "../graph";

export type {
  CanvasClipboardEvent,
  CanvasContextMenuEvent,
  CanvasEditBehavior,
  CanvasItem,
  CanvasItemMoveEvent,
  CanvasPoint,
  CanvasSelectionEvent,
  CanvasViewExpose,
  CanvasViewport,
  GraphConnectionValidation,
  GraphController,
  GraphData,
  GraphEndpoint,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphParameter,
  GraphParameterOption,
  GraphParameterType,
  GraphPort,
  GraphPortDirection,
  ViewFrontendLogEntry,
};

type ModuleExports = Record<string, unknown>;
type ViewRuntimeUnsubscribe = () => void;
type CompileViewSfc = (source: string, fileName?: string) => ViewSfcCompileResult;
type TransformModuleSource = (source: string, fileName?: string) => string;

const PROJECT_VIEW_IMPORT_PREFIXES = [
  "@locus/project-view",
  "@project-view",
];

export interface ViewRuntimeApi {
  callScript(scriptName: string, method: string, args?: unknown): Promise<ViewCallScriptResult>;
  bindingRead(request: Omit<ViewBindingReadRequest, "viewId">): Promise<ViewBindingReadResult>;
  bindingDiscover(
    request: Omit<ViewBindingDiscoverRequest, "viewId">,
  ): Promise<ViewBindingDiscoverResult>;
  bindingWrite(request: Omit<ViewBindingWriteRequest, "viewId">): Promise<ViewBindingWriteResult>;
  bindingApply(request: Omit<ViewBindingApplyRequest, "viewId">): Promise<ViewBindingApplyResult>;
  unityPropertyRead(request: UnitySerializedPropertyReadRequest): Promise<UnitySerializedPropertyReadResult>;
  unityPropertyDiscover(request: UnitySerializedPropertyDiscoverRequest): Promise<UnitySerializedPropertyDiscoverResult>;
  unityPropertyWrite(request: UnitySerializedPropertyWriteRequest): Promise<UnitySerializedPropertyWriteResult>;
  unityPropertyApply(request: UnitySerializedPropertyApplyRequest): Promise<UnitySerializedPropertyApplyResult>;
  searchAssets(query: string, roots?: string[], limit?: number): Promise<AssetSearchResult[]>;
  createSession(request?: ViewSessionCreateRequest): Promise<string>;
  showSession(sessionId: string): Promise<void>;
  loadSession(sessionId: string): Promise<SessionDetail>;
  getSessionActiveRun(sessionId: string): Promise<SessionRunSummary | null>;
  listSessionEvents(sessionId: string, afterSeq?: number | null, limit?: number | null): Promise<SessionEventRecord[]>;
  queueSessionInput(request: ViewSessionQueueInputRequest): Promise<ViewSessionQueueInputResult>;
  sendSessionMessage(request: ViewSessionChatRequest): Promise<ViewSessionChatResult>;
  waitSession(request: ViewSessionWaitRequest): Promise<ViewSessionWaitResult>;
  callLlm(request: ViewLlmCallRequest): Promise<ViewLlmCallResult>;
  onSessionEvent(handler: (event: StreamEvent) => void): Promise<ViewRuntimeUnsubscribe>;
  readFrontendLog(limit?: number): Promise<ViewFrontendLogEntry[]>;
  openFrontendLog(): Promise<void>;
  storageGet(key: string): Promise<unknown | null>;
  storageSet(key: string, value: unknown): Promise<void>;
  storageRemove(key: string): Promise<void>;
  onUpdate(handler: (event: ViewRuntimeUpdateEvent) => void): Promise<ViewRuntimeUnsubscribe>;
  reload(): Promise<void>;
}

export interface ViewRuntimeComponentOptions {
  detail: ViewPackageDetail;
  api: ViewRuntimeApi;
}

interface RuntimeContext {
  detail: ViewPackageDetail;
  api: ViewRuntimeApi;
  styles: string[];
  compileViewSfc: CompileViewSfc;
  transformModuleSource: TransformModuleSource;
  entryComponent?: Component;
  importModule: (specifier: string, importer?: string) => ModuleExports;
}

export interface ViewRuntimeUndoState {
  canUndo: boolean;
  canRedo: boolean;
  running: boolean;
  lastAction: string;
}

export interface ViewRuntimeUndoEntry {
  id?: string;
  label?: string;
  undo: () => unknown | Promise<unknown>;
  redo: () => unknown | Promise<unknown>;
}

export type ViewSerializedPropertySnapshotInput = Partial<ViewSerializedPropertySnapshot> & {
  propertyPath: string;
  value?: unknown;
  children?: ViewSerializedPropertySnapshotInput[];
};

export interface ViewRuntimeBindingWriteOptions {
  undoable?: boolean;
  label?: string;
  beforeSnapshot?: ViewSerializedPropertySnapshotInput | null;
  onApplied?: (result: ViewBindingWriteResult) => void | Promise<void>;
}

export interface ViewRuntimeBindingApplyOptions {
  undoable?: boolean;
  label?: string;
}

export interface ViewSerializedPropertyCommitInput {
  propertyPath: string;
  value: unknown;
  property?: ViewSerializedPropertySnapshotInput | null;
  snapshot?: ViewSerializedPropertySnapshotInput | null;
}

export type ViewGraphPortDirection = GraphPortDirection;
export type ViewCanvasClipboardEvent = CanvasClipboardEvent;
export type ViewCanvasContextMenuEvent = CanvasContextMenuEvent;
export type ViewCanvasEditBehavior = CanvasEditBehavior;
export type ViewCanvasItem = CanvasItem;
export type ViewCanvasItemMoveEvent = CanvasItemMoveEvent;
export type ViewCanvasPoint = CanvasPoint;
export type ViewCanvasSelectionEvent = CanvasSelectionEvent;
export type ViewCanvasViewport = CanvasViewport;
export type ViewCanvasViewExpose = CanvasViewExpose;
export type ViewGraphParameterType = GraphParameterType;
export type ViewGraphPort = GraphPort;
export type ViewGraphParameterOption = GraphParameterOption;
export type ViewGraphParameter = GraphParameter;
export type ViewGraphNode = GraphNode;
export type ViewGraphEndpoint = GraphEndpoint;
export type ViewGraphConnection = GraphLink;
export type ViewGraphData = GraphData;
export type ViewGraphConnectionValidation = GraphConnectionValidation;
export type ViewGraphController = GraphController;

export type ViewUnityReferenceInput =
  | string
  | AssetRefAttachment
  | {
    path: string;
    kind?: AssetRefKind;
    name?: string;
    typeLabel?: string;
    source?: "unity" | "manual";
  };

export interface ViewUnitySceneObjectTarget {
  scenePath: string;
  objectPath: string;
}

export interface ViewUnityReferenceDragBinding {
  refs: ComputedRef<AssetRefAttachment[]>;
  draggable: ComputedRef<boolean>;
  attrs: ComputedRef<Record<string, unknown>>;
  dragstart: (event: DragEvent) => void;
  pointerdown: (event: PointerEvent) => void;
}

export interface ViewUnityAssetDropTargetOptions {
  enabled?: () => boolean;
  onDrop?: (refs: AssetRefAttachment[], payload: UnityEmbedAssetDropPayload) => void;
}

export interface ViewLocusFileDropTargetOptions {
  enabled?: () => boolean;
  onDrop?: (files: LocusFileDropRef[], payload: LocusFileDropPayload) => void;
}

type ViewRuntimeUnsubscribeMaybeAsync = (() => void) | null;
type ViewUnityReferenceSource =
  | ViewUnityReferenceInput
  | ViewUnityReferenceInput[]
  | (() => ViewUnityReferenceInput | ViewUnityReferenceInput[] | null | undefined)
  | null
  | undefined;
type ViewLocusFileSource =
  | LocusFileDropRef
  | LocusFileDropRef[]
  | (() => LocusFileDropRef | LocusFileDropRef[] | null | undefined)
  | null
  | undefined;

const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const UNITY_SCENE_OBJECT_PATH_RE = /^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i;

function normalizeRuntimePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function basenameWithoutExtension(path: string): string {
  const normalized = normalizeRuntimePath(path);
  const name = normalized.split("/").filter(Boolean).pop() || normalized;
  const dotIdx = name.lastIndexOf(".");
  return dotIdx > 0 ? name.slice(0, dotIdx) : name;
}

function inferUnityRefKind(path: string, fallback?: AssetRefKind): AssetRefKind {
  if (UNITY_SCENE_OBJECT_PATH_RE.test(normalizeRuntimePath(path))) return "sceneObject";
  return fallback ?? "asset";
}

function normalizeUnityReference(input: ViewUnityReferenceInput | null | undefined): AssetRefAttachment | null {
  if (!input) return null;
  const raw = typeof input === "string"
    ? { path: input }
    : input;
  const path = normalizeRuntimePath(raw.path ?? "");
  if (!path) return null;
  const kind = inferUnityRefKind(path, raw.kind);
  return {
    kind,
    path,
    name: raw.name?.trim() || basenameWithoutExtension(path),
    typeLabel: raw.typeLabel?.trim() || undefined,
    source: raw.source ?? "manual",
  };
}

function normalizeUnityReferences(source: ViewUnityReferenceSource): AssetRefAttachment[] {
  const value = typeof source === "function" ? source() : source;
  const items = Array.isArray(value) ? value : value ? [value] : [];
  return items
    .map((item) => normalizeUnityReference(item))
    .filter((item): item is AssetRefAttachment => !!item);
}

function normalizeLocusFiles(source: ViewLocusFileSource): LocusFileDropRef[] {
  const value = typeof source === "function" ? source() : source;
  const items = Array.isArray(value) ? value : value ? [value] : [];
  return items
    .map((file) => ({
      ...file,
      path: normalizeRuntimePath(file.path ?? ""),
      name: file.name?.trim() || basenameWithoutExtension(file.path ?? ""),
      source: file.source ?? "view",
    }))
    .filter((file) => !!file.path);
}

function sceneObjectTargetFromReference(input: ViewUnityReferenceInput | AssetRefAttachment): ViewUnitySceneObjectTarget | null {
  const ref = normalizeUnityReference(input);
  if (!ref || ref.kind !== "sceneObject") return null;
  const match = ref.path.match(UNITY_SCENE_OBJECT_PATH_RE);
  if (!match) return null;
  return {
    scenePath: normalizeRuntimePath(match[1]),
    objectPath: normalizeRuntimePath(match[2]).replace(/^\/+|\/+$/g, ""),
  };
}

function isUnityAssetReference(ref: AssetRefAttachment): boolean {
  return ref.kind === "asset" && UNITY_ASSET_REF_ROOT_RE.test(normalizeRuntimePath(ref.path));
}

async function selectUnityReference(input: ViewUnityReferenceInput, options: { focusProjectWindow?: boolean } = {}) {
  const sceneObject = sceneObjectTargetFromReference(input);
  if (sceneObject) {
    await selectUnitySceneObject(sceneObject.scenePath, sceneObject.objectPath);
    return;
  }
  const ref = normalizeUnityReference(input);
  if (!ref || !isUnityAssetReference(ref)) return;
  await selectUnityAsset(ref.path, options);
}

async function inspectUnityReference(input: ViewUnityReferenceInput) {
  const sceneObject = sceneObjectTargetFromReference(input);
  if (sceneObject) {
    await openUnitySceneObjectInspector(sceneObject.scenePath, sceneObject.objectPath);
    return;
  }
  const ref = normalizeUnityReference(input);
  if (!ref || !isUnityAssetReference(ref)) return;
  await openUnityAssetInspector(ref.path);
}

function useUnityReferenceDrag(source: ViewUnityReferenceSource): ViewUnityReferenceDragBinding {
  const refs = computed(() => normalizeUnityReferences(source));
  const draggable = computed(() => refs.value.length > 0);

  function dragstart(event: DragEvent) {
    startUnityReferenceHtmlDrag(event, refs.value);
  }

  function pointerdown(event: PointerEvent) {
    armUnityReferencePointerDrag(event, refs.value);
  }

  const attrs = computed<Record<string, unknown>>(() => ({
    draggable: draggable.value,
    onDragstart: dragstart,
    onPointerdown: pointerdown,
  }));

  return {
    refs,
    draggable,
    attrs,
    dragstart,
    pointerdown,
  };
}

function useLocusFileDrag(source: ViewLocusFileSource) {
  const files = computed(() => normalizeLocusFiles(source));
  const draggable = computed(() => files.value.length > 0);

  function dragstart(event: DragEvent) {
    startLocusFileHtmlDrag(event, files.value);
  }

  function pointerdown(event: PointerEvent) {
    armLocusFilePointerDrag(event, files.value);
  }

  const attrs = computed<Record<string, unknown>>(() => ({
    draggable: draggable.value,
    onDragstart: dragstart,
    onPointerdown: pointerdown,
  }));

  return {
    files,
    draggable,
    attrs,
    dragstart,
    pointerdown,
  };
}

function useUnityAssetDropTargetRuntime(options: ViewUnityAssetDropTargetOptions = {}) {
  const refs = ref<AssetRefAttachment[]>([]);
  const active = ref(false);
  const base = useUnityAssetDropTargetBase({
    enabled: options.enabled,
    warnPrefix: "[view-runtime]",
  });
  let releaseDrop: ViewRuntimeUnsubscribeMaybeAsync = null;
  let releaseState: ViewRuntimeUnsubscribeMaybeAsync = null;
  let disposed = false;

  const isEnabled = () => options.enabled?.() ?? true;

  onMounted(() => {
    disposed = false;
    subscribeUnityEmbedAssetDrop((payload) => {
      if (!isEnabled()) return;
      const nextRefs = Array.isArray(payload.refs) ? payload.refs : [];
      refs.value = nextRefs;
      active.value = false;
      options.onDrop?.(nextRefs, payload);
    }).then((release) => {
      if (disposed) release();
      else releaseDrop = release;
    }).catch((error) => console.warn("[view-runtime] Unity asset drop subscription failed", error));

    subscribeUnityEmbedAssetDragState((payload: UnityEmbedAssetDragStatePayload) => {
      if (!isEnabled()) return;
      const nextRefs = Array.isArray(payload.refs) ? payload.refs : [];
      refs.value = nextRefs;
      active.value = !!payload.hasRefs && nextRefs.length > 0;
    }).then((release) => {
      if (disposed) release();
      else releaseState = release;
    }).catch((error) => console.warn("[view-runtime] Unity asset drag subscription failed", error));
  });

  onBeforeUnmount(() => {
    disposed = true;
    releaseDrop?.();
    releaseDrop = null;
    releaseState?.();
    releaseState = null;
    active.value = false;
    refs.value = [];
  });

  function dragenter(event: DragEvent) {
    if (!isEnabled()) return;
    active.value = true;
    base.handleUnityAssetDrag(event);
  }

  function dragover(event: DragEvent) {
    if (!isEnabled()) return;
    active.value = true;
    base.handleUnityAssetDrag(event);
  }

  function dragleave() {
    active.value = base.hasUnityAssetDragState();
  }

  function drop(event: DragEvent) {
    if (!isEnabled()) return;
    active.value = false;
    base.handleUnityAssetDrop(event);
  }

  const attrs = computed<Record<string, unknown>>(() => ({
    onDragenter: dragenter,
    onDragover: dragover,
    onDragleave: dragleave,
    onDrop: drop,
  }));

  return {
    refs,
    active,
    attrs,
    dragenter,
    dragover,
    dragleave,
    drop,
  };
}

function useLocusFileDropTargetRuntime(options: ViewLocusFileDropTargetOptions = {}) {
  const files = ref<LocusFileDropRef[]>([]);
  const dragState = ref<LocusFileDragStatePayload | null>(null);
  const active = computed(() => !!dragState.value?.active);
  let releaseDrop: ViewRuntimeUnsubscribeMaybeAsync = null;
  let releaseState: ViewRuntimeUnsubscribeMaybeAsync = null;
  let disposed = false;
  const isEnabled = () => options.enabled?.() ?? true;

  onMounted(() => {
    disposed = false;
    subscribeLocusFileDrop((payload) => {
      if (!isEnabled()) return;
      const nextFiles = Array.isArray(payload.files) ? payload.files : [];
      files.value = nextFiles;
      options.onDrop?.(nextFiles, payload);
    }).then((release) => {
      if (disposed) release();
      else releaseDrop = release;
    }).catch((error) => console.warn("[view-runtime] file drop subscription failed", error));

    subscribeLocusFileDragState((payload) => {
      if (!isEnabled()) return;
      dragState.value = payload;
    }).then((release) => {
      if (disposed) release();
      else releaseState = release;
    }).catch((error) => console.warn("[view-runtime] file drag subscription failed", error));
  });

  onBeforeUnmount(() => {
    disposed = true;
    releaseDrop?.();
    releaseDrop = null;
    releaseState?.();
    releaseState = null;
    files.value = [];
    dragState.value = null;
  });

  return {
    files,
    dragState,
    active,
  };
}

const UnityReferenceChip = defineComponent({
  name: "UnityReferenceChip",
  props: {
    reference: {
      type: [String, Object] as unknown as PropType<ViewUnityReferenceInput>,
      default: null,
    },
    path: {
      type: String,
      default: "",
    },
    kind: {
      type: String as PropType<AssetRefKind>,
      default: undefined,
    },
    name: {
      type: String,
      default: "",
    },
    inspectOnMeta: {
      type: Boolean,
      default: true,
    },
  },
  setup(props, { slots }) {
    const refItem = computed(() => normalizeUnityReference(
      props.reference ?? {
        path: props.path,
        kind: props.kind,
        name: props.name,
      },
    ));
    const drag = useUnityReferenceDrag(() => refItem.value ? [refItem.value] : []);
    const label = computed(() => props.name || refItem.value?.name || basenameWithoutExtension(props.path));

    async function click(event: MouseEvent) {
      const current = refItem.value;
      if (!current) return;
      if (props.inspectOnMeta && (event.ctrlKey || event.metaKey)) {
        await inspectUnityReference(current);
        return;
      }
      await selectUnityReference(current);
    }

    return () => h("button", {
      type: "button",
      class: "locus-unity-reference-chip",
      title: refItem.value?.path ?? props.path,
      draggable: drag.draggable.value,
      onClick: click,
      onDragstart: drag.dragstart,
      onPointerdown: drag.pointerdown,
    }, slots.default?.({ reference: refItem.value }) ?? label.value);
  },
});

const UnityDropZone = defineComponent({
  name: "UnityDropZone",
  emits: ["drop"],
  setup(_props, { emit, slots }) {
    const target = useUnityAssetDropTargetRuntime({
      onDrop: (refs, payload) => emit("drop", refs, payload),
    });
    return () => h("div", {
      class: [
        "locus-unity-drop-zone",
        target.active.value ? "is-active" : "",
      ],
      onDragenter: target.dragenter,
      onDragover: target.dragover,
      onDragleave: target.dragleave,
      onDrop: target.drop,
    }, slots.default?.({
      active: target.active.value,
      refs: target.refs.value,
    }));
  },
});

const LOCUS_COMPONENTS = {
  BaseButton,
  BaseCheckbox,
  BaseDropdown,
  BaseSegmented,
  BaseSwitch,
  CanvasView,
  GraphView,
  UnityBoolField,
  UnityColorField,
  UnityEnumField,
  UnityFlagsField,
  UnityLayerMaskField,
  UnityNumberField,
  UnityObjectReferenceField,
  UnityPropertyDraw,
  UnityPropertyEditor,
  UnitySerializedPropertyTree,
  UnityObjectPreview,
  UnityReferenceChip,
  UnityDropZone,
  UnityVectorField,
};

type ViewRuntimeModuleApi = ReturnType<typeof createViewRuntimeApiUncached>;

const runtimeApiCache = new WeakMap<ViewRuntimeApi, ViewRuntimeModuleApi>();
const MAX_RUNTIME_API_REQUEST_LOGS = 48;
let undoEntrySequence = 0;

function perfNowMs(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function elapsedMs(startedAt: number): number {
  return Math.round(perfNowMs() - startedAt);
}

function runtimeErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function afterNextFrame(task: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => task());
    return;
  }
  setTimeout(task, 0);
}

function createViewUndoService() {
  const undoStack: Required<ViewRuntimeUndoEntry>[] = [];
  const redoStack: Required<ViewRuntimeUndoEntry>[] = [];
  const state = reactive<ViewRuntimeUndoState>({
    canUndo: false,
    canRedo: false,
    running: false,
    lastAction: "",
  });

  function refreshState() {
    state.canUndo = undoStack.length > 0;
    state.canRedo = redoStack.length > 0;
  }

  function normalizeEntry(entry: ViewRuntimeUndoEntry): Required<ViewRuntimeUndoEntry> {
    undoEntrySequence += 1;
    return {
      id: entry.id || `view-undo-${undoEntrySequence}`,
      label: entry.label || "View change",
      undo: entry.undo,
      redo: entry.redo,
    };
  }

  function record(entry: ViewRuntimeUndoEntry) {
    undoStack.push(normalizeEntry(entry));
    redoStack.length = 0;
    refreshState();
  }

  async function run(direction: "undo" | "redo") {
    if (state.running) return;
    const source = direction === "undo" ? undoStack : redoStack;
    const target = direction === "undo" ? redoStack : undoStack;
    const entry = source.pop();
    if (!entry) {
      refreshState();
      return;
    }

    state.running = true;
    state.lastAction = entry.label;
    refreshState();
    try {
      await (direction === "undo" ? entry.undo() : entry.redo());
      target.push(entry);
    } catch (error) {
      source.push(entry);
      throw error;
    } finally {
      state.running = false;
      refreshState();
    }
  }

  function clear() {
    undoStack.length = 0;
    redoStack.length = 0;
    state.lastAction = "";
    refreshState();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.altKey) return;
    if (!event.ctrlKey && !event.metaKey) return;
    const key = event.key.toLowerCase();
    const wantsUndo = key === "z" && !event.shiftKey;
    const wantsRedo = key === "y" || (key === "z" && event.shiftKey);
    if (wantsUndo && state.canUndo) {
      event.preventDefault();
      event.stopPropagation();
      void run("undo").catch((error) => console.error("[view-runtime] Undo failed", error));
    } else if (wantsRedo && state.canRedo) {
      event.preventDefault();
      event.stopPropagation();
      void run("redo").catch((error) => console.error("[view-runtime] Redo failed", error));
    }
  }

  refreshState();

  return {
    state: readonly(state),
    record,
    undo: () => run("undo"),
    redo: () => run("redo"),
    clear,
    handleKeydown,
    isRunning: () => state.running,
  };
}

function snapshotRestoreValue(snapshot: ViewSerializedPropertySnapshotInput) {
  return {
    action: "restoreSnapshot",
    snapshot,
  };
}

function snapshotHistoryKey(snapshot: ViewSerializedPropertySnapshotInput | null | undefined): string {
  if (!snapshot) return "";
  try {
    return JSON.stringify({
      propertyPath: snapshot.propertyPath,
      value: snapshot.value,
      displayValue: snapshot.displayValue,
      arraySize: snapshot.arraySize,
      managedReferenceFullTypename: snapshot.managedReferenceFullTypename,
      children: snapshot.children,
    });
  } catch {
    return `${snapshot.propertyPath}|${snapshot.displayValue}`;
  }
}

function bindingWriteKey(request: Omit<ViewBindingWriteRequest, "viewId">): string {
  try {
    return JSON.stringify({
      bindingId: request.bindingId ?? "",
      target: request.target ?? null,
    });
  } catch {
    return `${request.bindingId ?? ""}`;
  }
}

function createViewBindingRuntime(api: ViewRuntimeApi, undo: ReturnType<typeof createViewUndoService>) {
  const writeQueues = new Map<string, Promise<unknown>>();

  function queueWrite<T>(key: string, job: () => Promise<T>): Promise<T> {
    const previous = writeQueues.get(key) ?? Promise.resolve();
    const run = previous.catch(() => undefined).then(job);
    let tracked: Promise<unknown>;
    tracked = run.finally(() => {
      if (writeQueues.get(key) === tracked) writeQueues.delete(key);
    });
    writeQueues.set(key, tracked);
    return run;
  }

  async function readBeforeSnapshot(
    request: Omit<ViewBindingWriteRequest, "viewId">,
    options: ViewRuntimeBindingWriteOptions,
  ): Promise<ViewSerializedPropertySnapshotInput | null> {
    if (options.beforeSnapshot) return options.beforeSnapshot;
    const result = await api.bindingRead({
      bindingId: request.bindingId,
      target: request.target,
    });
    return result;
  }

  function recordWriteUndo(
    request: Omit<ViewBindingWriteRequest, "viewId">,
    before: ViewSerializedPropertySnapshotInput,
    after: ViewBindingWriteResult,
    options: ViewRuntimeBindingWriteOptions,
  ) {
    if (snapshotHistoryKey(before) === snapshotHistoryKey(after)) return;
    const target = after.target ?? request.target ?? null;
    const bindingId = after.bindingId ?? request.bindingId ?? null;
    const label = options.label || after.displayName || after.propertyPath || "View property";
    const undoRequest = { bindingId, target, value: snapshotRestoreValue(before) };
    const redoRequest = { bindingId, target, value: snapshotRestoreValue(after) };
    undo.record({
      label,
      undo: () => write(undoRequest, { undoable: false, onApplied: options.onApplied }),
      redo: () => write(redoRequest, { undoable: false, onApplied: options.onApplied }),
    });
  }

  async function writeNow(
    request: Omit<ViewBindingWriteRequest, "viewId">,
    options: ViewRuntimeBindingWriteOptions = {},
  ): Promise<ViewBindingWriteResult> {
    const shouldRecord = options.undoable !== false && !undo.isRunning();
    const before = shouldRecord ? await readBeforeSnapshot(request, options) : null;
    const result = await api.bindingWrite(request);
    await options.onApplied?.(result);
    if (shouldRecord && before) {
      recordWriteUndo(request, before, result, options);
    }
    return result;
  }

  function write(
    request: Omit<ViewBindingWriteRequest, "viewId">,
    options: ViewRuntimeBindingWriteOptions = {},
  ): Promise<ViewBindingWriteResult> {
    return queueWrite(bindingWriteKey(request), () => writeNow(request, options));
  }

  async function writeProperty(
    request: Omit<ViewBindingReadRequest, "viewId">,
    commit: ViewSerializedPropertyCommitInput,
    options: ViewRuntimeBindingWriteOptions = {},
  ): Promise<ViewBindingWriteResult> {
    const baseTarget = request.target
      ? { ...request.target, propertyPath: commit.propertyPath }
      : (await api.bindingRead(request)).target;
    const target = baseTarget
      ? { ...baseTarget, propertyPath: commit.propertyPath }
      : null;
    return write({
      bindingId: request.bindingId,
      target,
      value: commit.value,
    }, {
      ...options,
      beforeSnapshot: options.beforeSnapshot ?? commit.property ?? commit.snapshot ?? null,
    });
  }

  async function apply(
    request: Omit<ViewBindingApplyRequest, "viewId">,
    options: ViewRuntimeBindingApplyOptions = {},
  ): Promise<ViewBindingApplyResult> {
    const shouldRecord = options.undoable !== false && !undo.isRunning();
    const before = shouldRecord
      ? await Promise.all(request.writes.map((writeRequest) =>
        api.bindingRead({
          bindingId: writeRequest.bindingId,
          target: writeRequest.target,
        }).catch(() => null),
      ))
      : [];
    const result = await api.bindingApply(request);
    if (shouldRecord) {
      const undoWrites: ViewBindingApplyWrite[] = result.results
        .map((after, index): ViewBindingApplyWrite | null => before[index] && after.target
          ? {
            bindingId: after.bindingId,
            target: after.target,
            value: snapshotRestoreValue(before[index]!),
          }
          : null)
        .filter((item): item is ViewBindingApplyWrite => !!item);
      const redoWrites: ViewBindingApplyWrite[] = result.results
        .filter((after) => after.target)
        .map((after) => ({
          bindingId: after.bindingId,
          target: after.target,
          value: snapshotRestoreValue(after),
        }));
      if (undoWrites.length && redoWrites.length) {
        undo.record({
          label: options.label || "View bindings",
          undo: () => apply({ writes: undoWrites }, { undoable: false }),
          redo: () => apply({ writes: redoWrites }, { undoable: false }),
        });
      }
    }
    return result;
  }

  return {
    read: (request: Omit<ViewBindingReadRequest, "viewId">) => api.bindingRead(request),
    discover: (request: Omit<ViewBindingDiscoverRequest, "viewId">) =>
      api.bindingDiscover(request),
    write,
    writeProperty,
    apply,
  };
}

const LOCUS_COMPONENT_MODULE = {
  ...LOCUS_COMPONENTS,
  ...PropertyTreeService,
  ...UnityObjectDrawerService,
  applyUnityRgbHexToColorText,
  formatUnityColorValue,
  formatUnityVectorValue,
  isUnityIntegerPropertyType,
  isUnityNumberPropertyType,
  isUnityVectorPropertyType,
  normalizeUnityOptions,
  normalizeUnityPropertyType,
  parseUnityColorValue,
  parseUnitySerializedEditValue,
  parseUnityVectorValue,
  tryParseUnitySerializedEditValue,
  unityColorTextToRgbHex,
  unityEnumIndexValue,
  unityEnumNumericValue,
  unitySerializedValueToEditText,
  unityVectorKeysForType,
};

function createVueModule(context: RuntimeContext): ModuleExports {
  const createAppShim = (component: Component) => {
    context.entryComponent = component;
    const app: {
      mount: () => undefined;
      use: () => unknown;
      component: () => unknown;
      provide: () => unknown;
    } = {
      mount: () => undefined,
      use: () => app,
      component: () => app,
      provide: () => app,
    };
    return app;
  };

  return {
    ...VueRuntime,
    createApp: createAppShim,
  };
}

function fileByPath(detail: ViewPackageDetail, relPath: string): ViewPackageFile | null {
  return detail.files.find((file) => file.relPath === relPath) ?? null;
}

function normalizeRelPath(value: string): string {
  const parts: string[] = [];
  for (const part of value.replace(/\\/g, "/").split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      parts.pop();
      continue;
    }
    parts.push(part);
  }
  return parts.join("/");
}

function viewWorkspaceRelPath(detail: ViewPackageDetail): string {
  const viewRoot = normalizeRelPath(detail.summary.packageRelPath || detail.manifest.id);
  const parts = viewRoot.split("/").filter(Boolean);
  parts.pop();
  return parts.join("/");
}

function viewWorkspaceSourceRelPath(detail: ViewPackageDetail, relPath = "index"): string {
  const workspace = viewWorkspaceRelPath(detail);
  return normalizeRelPath(workspace ? `${workspace}/src/${relPath}` : `src/${relPath}`);
}

function resolveModulePath(detail: ViewPackageDetail, specifier: string, importer = viewPackageRelPath(detail, "src/App.vue")): string {
  const projectSharedPath = resolveProjectSharedModulePath(detail, specifier);
  if (projectSharedPath) return projectSharedPath;
  if (!specifier.startsWith(".")) return viewPackageRelPath(detail, specifier);
  const base = importer.includes("/") ? importer.slice(0, importer.lastIndexOf("/") + 1) : "";
  const normalized = normalizeRelPath(`${base}${specifier}`);
  const hasExtension = /\.[a-z0-9]+$/i.test(normalized);
  if (hasExtension) return normalized;
  return normalized;
}

function resolveProjectSharedModulePath(detail: ViewPackageDetail, specifier: string): string | null {
  for (const prefix of PROJECT_VIEW_IMPORT_PREFIXES) {
    if (specifier === prefix) return viewWorkspaceSourceRelPath(detail, "index");
    if (specifier.startsWith(`${prefix}/`)) {
      const childPath = normalizeRelPath(specifier.slice(prefix.length + 1));
      return childPath ? viewWorkspaceSourceRelPath(detail, childPath) : viewWorkspaceSourceRelPath(detail, "index");
    }
  }
  return null;
}

function resolveFile(detail: ViewPackageDetail, specifier: string, importer?: string): ViewPackageFile | null {
  const base = resolveModulePath(detail, specifier, importer);
  const candidates = /\.[a-z0-9]+$/i.test(base)
    ? [base]
    : [`${base}.ts`, `${base}.vue`, `${base}.js`, `${base}.css`, `${base}/index.ts`];

  for (const candidate of candidates) {
    const file = fileByPath(detail, candidate);
    if (file) return file;
  }
  return null;
}

function createViewRuntimeApi(detail: ViewPackageDetail, api: ViewRuntimeApi): ViewRuntimeModuleApi {
  const cached = runtimeApiCache.get(api);
  if (cached) return cached;
  const runtime = createViewRuntimeApiUncached(detail, api);
  runtimeApiCache.set(api, runtime);
  return runtime;
}

function createViewRuntimeApiUncached(detail: ViewPackageDetail, api: ViewRuntimeApi) {
  const undo = createViewUndoService();
  const binding = createViewBindingRuntime(api, undo);
  const serializedProperty = {
    read: api.unityPropertyRead,
    discover: api.unityPropertyDiscover,
    write: api.unityPropertyWrite,
    apply: api.unityPropertyApply,
  };
  const propertyDrawer = {
    library: PropertyTreeService.publicInspectorPropertyDrawerLibrary,
    projectLibrary: PropertyTreeService.projectInspectorPropertyDrawerLibrary,
    register: PropertyTreeService.registerInspectorPropertyDrawer,
    registerValue: PropertyTreeService.registerInspectorValueDrawer,
    registerField: PropertyTreeService.registerInspectorFieldDrawer,
    registerAttribute: PropertyTreeService.registerInspectorAttributeDrawer,
    registerPropertyPath: PropertyTreeService.registerInspectorPropertyPathDrawer,
    define: PropertyTreeService.defineInspectorPropertyDrawers,
    normalize: PropertyTreeService.normalizeInspectorPropertyDrawers,
    createLibrary: PropertyTreeService.createInspectorPropertyDrawerLibrary,
  };
  const unityObjectDrawer = {
    library: UnityObjectDrawerService.publicUnityObjectDrawerLibrary,
    projectLibrary: UnityObjectDrawerService.projectUnityObjectDrawerLibrary,
    register: UnityObjectDrawerService.registerUnityObjectDrawer,
    define: UnityObjectDrawerService.defineUnityObjectDrawers,
    normalize: UnityObjectDrawerService.defineUnityObjectDrawers,
    createLibrary: UnityObjectDrawerService.createUnityObjectDrawerLibrary,
    resolve: UnityObjectDrawerService.resolveUnityObjectDrawer,
  };
  const objectReferencePicker = {
    roots: UnityObjectReferencePickerService.UNITY_OBJECT_REFERENCE_SEARCH_ROOTS,
    searchQuery: UnityObjectReferencePickerService.unityObjectReferenceSearchQuery,
    filterResults: UnityObjectReferencePickerService.filterUnityObjectReferenceSearchResults,
    isResult: UnityObjectReferencePickerService.isUnityObjectReferenceSearchResult,
    typeHint: UnityObjectReferencePickerService.unityObjectReferenceTypeHint,
    typeKey: UnityObjectReferencePickerService.unityObjectReferenceTypeKey,
    typeRule: UnityObjectReferencePickerService.getUnityObjectReferenceTypeRule,
    normalizePath: UnityObjectReferencePickerService.normalizeUnityObjectReferencePath,
    extension: UnityObjectReferencePickerService.unityObjectReferenceExtension,
  };
  const session = {
    create: (request?: string | ViewSessionCreateRequest) =>
      api.createSession(typeof request === "string" ? { title: request } : request),
    show: (sessionId: string) => api.showSession(sessionId),
    display: (sessionId: string) => api.showSession(sessionId),
    load: (sessionId: string) => api.loadSession(sessionId),
    activeRun: (sessionId: string) => api.getSessionActiveRun(sessionId),
    events: (sessionId: string, afterSeq?: number | null, limit?: number | null) =>
      api.listSessionEvents(sessionId, afterSeq, limit),
    queueInput: (request: ViewSessionQueueInputRequest) => api.queueSessionInput(request),
    chat: (request: ViewSessionChatRequest) => api.sendSessionMessage(request),
    send: (request: ViewSessionChatRequest) => api.sendSessionMessage(request),
    wait: (request: string | ViewSessionWaitRequest, runId?: string | null) =>
      api.waitSession(typeof request === "string" ? { sessionId: request, runId } : request),
    onEvent: (handler: (event: StreamEvent) => void) => api.onSessionEvent(handler),
  };

  const llm = {
    call: (request: ViewLlmCallRequest) => api.callLlm(request),
  };

  const storage = {
    get: (key: string) => api.storageGet(key),
    set: (key: string, value: unknown) => api.storageSet(key, value),
    remove: (key: string) => api.storageRemove(key),
  };

  const unity = {
    callScript: async (scriptName: string, method: string, args?: unknown) => {
      const response = await api.callScript(scriptName, method, args);
      return response.result;
    },
    checkConnection: checkUnityConnectionStatus,
    connectionStatus: checkUnityConnectionStatus,
    normalizeReference: normalizeUnityReference,
    sceneObjectTarget: sceneObjectTargetFromReference,
    selectAsset: (path: string, options?: { focusProjectWindow?: boolean }) =>
      selectUnityAsset(path, options),
    inspectAsset: (path: string) => openUnityAssetInspector(path),
    openAssetInspector: (path: string) => openUnityAssetInspector(path),
    selectSceneObject: (scenePath: string, objectPath: string) =>
      selectUnitySceneObject(scenePath, objectPath),
    inspectSceneObject: (scenePath: string, objectPath: string) =>
      openUnitySceneObjectInspector(scenePath, objectPath),
    openSceneObjectInspector: (scenePath: string, objectPath: string) =>
      openUnitySceneObjectInspector(scenePath, objectPath),
    select: selectUnityReference,
    inspect: inspectUnityReference,
    drag: {
      start: (event: DragEvent, refs: ViewUnityReferenceInput | ViewUnityReferenceInput[]) =>
        startUnityReferenceHtmlDrag(event, normalizeUnityReferences(refs)),
      arm: (event: PointerEvent, refs: ViewUnityReferenceInput | ViewUnityReferenceInput[]) =>
        armUnityReferencePointerDrag(event, normalizeUnityReferences(refs)),
      commitDrop: commitUnityEmbedAssetDrop,
      onDrop: subscribeUnityEmbedAssetDrop,
      onState: subscribeUnityEmbedAssetDragState,
    },
    onDrop: subscribeUnityEmbedAssetDrop,
    onDragState: subscribeUnityEmbedAssetDragState,
    serializedProperty,
    objectDrawer: unityObjectDrawer,
    objectReferencePicker,
  };

  const files = {
    drag: {
      start: (event: DragEvent, refs: LocusFileDropRef | LocusFileDropRef[]) =>
        startLocusFileHtmlDrag(event, normalizeLocusFiles(refs)),
      arm: (event: PointerEvent, refs: LocusFileDropRef | LocusFileDropRef[]) =>
        armLocusFilePointerDrag(event, normalizeLocusFiles(refs)),
      onDrop: subscribeLocusFileDrop,
      onState: subscribeLocusFileDragState,
    },
    onDrop: subscribeLocusFileDrop,
    onDragState: subscribeLocusFileDragState,
  };

  const view = {
    manifest: readonly(detail.manifest),
    summary: readonly(detail.summary),
    reload: api.reload,
    callScript: async (scriptName: string, method: string, args?: unknown) => {
      const response = await api.callScript(scriptName, method, args);
      return response.result;
    },
    binding,
    assets: {
      search: (query: string, roots = ["Assets", "Packages"], limit?: number) =>
        api.searchAssets(query, roots, limit),
    },
    logs: {
      read: (limit?: number) => api.readFrontendLog(limit),
      latest: async () => {
        const entries = await api.readFrontendLog(1);
        return entries[entries.length - 1] ?? null;
      },
      open: () => api.openFrontendLog(),
    },
    session,
    llm,
    storage,
    unity,
    files,
    undo,
    propertyDrawer,
    unityObjectDrawer,
    objectReferencePicker,
    serializedProperty,
    unitySerializedProperty: serializedProperty,
    openLog: () => api.openFrontendLog(),
    onUpdate: (handler: (event: ViewRuntimeUpdateEvent) => void) => api.onUpdate(handler),
    readBinding: (bindingId: string, target?: ViewBindingReadRequest["target"]) =>
      binding.read({ bindingId, target }),
    discoverBinding: (request: Omit<ViewBindingDiscoverRequest, "viewId">) =>
      binding.discover(request),
    writeBinding: (bindingId: string, value: unknown, target?: ViewBindingWriteRequest["target"]) =>
      binding.write({ bindingId, value, target }),
    applyBindings: (writes: ViewBindingApplyRequest["writes"]) => binding.apply({ writes }),
  };

  return {
    view,
    session,
    llm,
    storage,
    unity,
    files,
    undo,
    propertyDrawer,
    unityObjectDrawer,
    serializedProperty,
    unitySerializedProperty: serializedProperty,
    defineView: <T>(value: T) => value,
    defineGraphView,
    CanvasView,
    GraphView,
    GraphViewController,
    UnityPropertyDraw,
    UnityPropertyEditor,
    UnitySerializedPropertyTree,
    UnityObjectPreview,
    UnityReferenceChip,
    UnityDropZone,
    layoutGraphDocument,
    ...PropertyTreeService,
    ...UnityObjectDrawerService,
    ...UnityObjectReferencePickerService,
    onEditorUpdate: (handler: (event: ViewRuntimeUpdateEvent) => void) => view.onUpdate(handler),
    useUnityReferenceDrag,
    useUnityAssetDropTarget: useUnityAssetDropTargetRuntime,
    useLocusFileDrag,
    useLocusFileDropTarget: useLocusFileDropTargetRuntime,
    useViewState: <T extends object>(initial: T) => reactive(initial),
    useViewScript: (scriptName: string) => ({
      call: (method: string, args?: unknown) => view.callScript(scriptName, method, args),
    }),
    useUnityBinding: (bindingIdOrRequest: string | Omit<ViewBindingReadRequest, "viewId">) => {
      const value = ref<unknown>(null);
      const property = ref<ViewBindingReadResult | null>(null);
      const status = ref("idle");
      const error = ref("");
      const requestForBinding = (): Omit<ViewBindingReadRequest, "viewId"> =>
        typeof bindingIdOrRequest === "string"
          ? { bindingId: bindingIdOrRequest }
          : bindingIdOrRequest;
      const applyResultToState = (result: ViewBindingReadResult) => {
        property.value = result;
        value.value = result.value;
      };
      const read = async () => {
        status.value = "reading";
        error.value = "";
        try {
          const result = await binding.read(requestForBinding());
          applyResultToState(result);
          status.value = "ready";
          return result;
        } catch (readError) {
          status.value = "error";
          error.value = readError instanceof Error ? readError.message : String(readError);
          throw readError;
        }
      };
      const write = async (nextValue = value.value) => {
        status.value = "writing";
        error.value = "";
        try {
          const result = await binding.write({
            ...requestForBinding(),
            value: nextValue,
          }, {
            onApplied: applyResultToState,
          });
          status.value = "ready";
          return result;
        } catch (writeError) {
          status.value = "error";
          error.value = writeError instanceof Error ? writeError.message : String(writeError);
          throw writeError;
        }
      };
      const writeProperty = async (commit: ViewSerializedPropertyCommitInput) => {
        status.value = "writing";
        error.value = "";
        try {
          const result = await binding.writeProperty(requestForBinding(), commit, {
            onApplied: async () => {
              const refreshed = await binding.read(requestForBinding());
              applyResultToState(refreshed);
            },
          });
          status.value = "ready";
          return result;
        } catch (writeError) {
          status.value = "error";
          error.value = writeError instanceof Error ? writeError.message : String(writeError);
          throw writeError;
        }
      };
      return { value, property, status, error, read, write, writeProperty, undo: view.undo.undo, redo: view.undo.redo };
    },
  };
}

function installLegacyWindowApi(runtime: ReturnType<typeof createViewRuntimeApi>) {
  if (typeof window === "undefined") return;
  const target = window as typeof window & {
    locus?: Record<string, unknown>;
  };
  target.locus = {
    ...(target.locus ?? {}),
    view: runtime.view,
    unity: runtime.unity,
  };
}

function createModuleLoader(context: RuntimeContext) {
  const cache = new Map<string, ModuleExports>();

  function load(specifier: string, importer = viewPackageRelPath(context.detail, "src/App.vue")): ModuleExports {
    if (specifier === "vue") return createVueModule(context);
    if (specifier === "@locus/view-runtime") return createViewRuntimeApi(context.detail, context.api);
    if (specifier === "@locus/components") return LOCUS_COMPONENT_MODULE;

    const file = resolveFile(context.detail, specifier, importer);
    if (!file) {
      throw new Error(`View module not found: ${specifier}`);
    }
    if (cache.has(file.relPath)) return cache.get(file.relPath)!;

    if (file.relPath.endsWith(".css")) {
      context.styles.push(file.content);
      const exports = {};
      cache.set(file.relPath, exports);
      return exports;
    }

    if (file.relPath.endsWith(".vue")) {
      const exports = {
        default: buildSfcComponent(context, file.content, file.relPath),
      };
      cache.set(file.relPath, exports);
      return exports;
    }

    const module = { exports: {} as ModuleExports };
    cache.set(file.relPath, module.exports);
    const code = context.transformModuleSource(file.content, file.relPath);
    const execute = new Function("__import", "exports", "module", "__vue", "__runtime", code);
    execute(
      (childSpecifier: string) => load(childSpecifier, file.relPath),
      module.exports,
      module,
      createVueModule(context),
      createViewRuntimeApi(context.detail, context.api),
    );
    cache.set(file.relPath, module.exports);
    return module.exports;
  }

  return load;
}

function buildSfcComponent(context: RuntimeContext, source: string, relPath: string): Component {
  const compiled = context.compileViewSfc(source, relPath);
  const importModule = context.importModule;
  context.styles.push(...compiled.styles);

  const module = { exports: {} as ModuleExports };
  const execute = new Function("__import", "exports", "module", "__vue", "__runtime", compiled.code);
  execute(
    (specifier: string) => importModule(specifier, relPath),
    module.exports,
    module,
    createVueModule(context),
    createViewRuntimeApi(context.detail, context.api),
  );
  const options = (module.exports.default ?? {}) as Record<string, unknown>;
  return defineComponent({
    ...options,
    components: {
      ...LOCUS_COMPONENTS,
      ...((options.components as Record<string, Component> | undefined) ?? {}),
    },
  });
}

function viewRuntimeStyleText(detail: ViewPackageDetail, styles: string[]): string {
  return [
    viewRuntimeBaseCss(),
    sanitizeCssForPreview(viewFileContent(detail, detail.manifest.style)),
    ...styles.map(sanitizeCssForPreview),
  ].join("\n\n");
}

function useViewRuntimeStyles(detail: ViewPackageDetail): (styles: string[]) => void {
  const styleEl = document.createElement("style");
  styleEl.dataset.locusViewRuntimeStyle = detail.manifest.id;
  const applyStyles = (styles: string[]) => {
    styleEl.textContent = viewRuntimeStyleText(detail, styles);
  };
  applyStyles([]);
  document.head.appendChild(styleEl);
  onBeforeUnmount(() => {
    styleEl.remove();
  });
  return applyStyles;
}

function viewRuntimeBaseCss(): string {
  return `body {
  background: var(--bg-color);
  color: var(--text-color);
}

.locus-view-runtime-root {
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: auto;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

.locus-view-runtime-root input[type="number"] {
  appearance: textfield;
  -moz-appearance: textfield;
}

.locus-view-runtime-root input[type="number"]::-webkit-inner-spin-button,
.locus-view-runtime-root input[type="number"]::-webkit-outer-spin-button {
  margin: 0;
  -webkit-appearance: none;
}

.locus-unity-reference-chip {
  max-width: 100%;
  min-height: 24px;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  line-height: 1.3;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: default;
}

.locus-unity-reference-chip:hover,
.locus-unity-reference-chip:focus-visible {
  border-color: var(--border-strong);
  background: var(--hover-bg);
  outline: none;
}

.locus-unity-drop-zone {
  min-height: 32px;
  border: 1px dashed var(--border-color);
  border-radius: 6px;
}

.locus-unity-drop-zone.is-active {
  border-color: var(--accent-color);
  background: var(--accent-soft);
}

.view-runtime-error {
  margin: 12px;
  padding: 8px 10px;
  border: 1px solid var(--status-danger-border);
  border-radius: 6px;
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.45;
}

.view-runtime-loading {
  margin: 12px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}`;
}

function createInstrumentedRuntimeApi(detail: ViewPackageDetail, api: ViewRuntimeApi): ViewRuntimeApi {
  const viewId = detail.manifest.id;
  let requestSequence = 0;
  let dataRequestSequence = 0;

  function measureRequest<T>(
    request: string,
    task: () => Promise<T>,
    requestDetail: Record<string, unknown> = {},
    kind: "data" | "subscription" | "log" | "control" = "data",
  ): Promise<T> {
    requestSequence += 1;
    const callId = requestSequence;
    const startedAt = perfNowMs();
    const phasePrefix = kind === "data" ? "runtimeDataRequest" : "runtimeApiRequest";
    let dataCallId: number | undefined;
    if (kind === "data") {
      dataRequestSequence += 1;
      dataCallId = dataRequestSequence;
    }
    const shouldLog = callId <= MAX_RUNTIME_API_REQUEST_LOGS
      || (kind === "data" && (dataCallId ?? 0) <= MAX_RUNTIME_API_REQUEST_LOGS);
    const detailBase = {
      viewId,
      request,
      callId,
      dataCallId,
      kind,
      ...requestDetail,
    };
    if (dataCallId === 1) {
      markStartupPhase("runtimeFirstDataRequest", detailBase);
    }
    if (shouldLog) {
      markStartupPhase(`${phasePrefix}_start`, detailBase);
    }
    return Promise.resolve()
      .then(task)
      .then((result) => {
        if (shouldLog) {
          markStartupPhase(`${phasePrefix}_done`, {
            ...detailBase,
            elapsedMs: elapsedMs(startedAt),
          });
        }
        return result;
      }, (error) => {
        if (shouldLog) {
          markStartupPhase(`${phasePrefix}_error`, {
            ...detailBase,
            elapsedMs: elapsedMs(startedAt),
            message: runtimeErrorMessage(error),
          });
        }
        throw error;
      });
  }

  return {
    callScript: (scriptName, method, args) =>
      measureRequest("callScript", () => api.callScript(scriptName, method, args), {
        scriptName,
        method,
      }),
    bindingRead: (request) =>
      measureRequest("bindingRead", () => api.bindingRead(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        propertyPath: request.target?.propertyPath ?? "",
      }),
    bindingDiscover: (request) =>
      measureRequest("bindingDiscover", () => api.bindingDiscover(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        query: request.query ?? "",
      }),
    bindingWrite: (request) =>
      measureRequest("bindingWrite", () => api.bindingWrite(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        propertyPath: request.target?.propertyPath ?? "",
      }),
    bindingApply: (request) =>
      measureRequest("bindingApply", () => api.bindingApply(request), {
        writeCount: request.writes.length,
      }),
    unityPropertyRead: (request) =>
      measureRequest("unityPropertyRead", () => api.unityPropertyRead(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        propertyPath: request.target?.propertyPath ?? "",
      }),
    unityPropertyDiscover: (request) =>
      measureRequest("unityPropertyDiscover", () => api.unityPropertyDiscover(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        query: request.query ?? "",
      }),
    unityPropertyWrite: (request) =>
      measureRequest("unityPropertyWrite", () => api.unityPropertyWrite(request), {
        bindingId: request.bindingId ?? "",
        targetKind: request.target?.kind ?? "",
        propertyPath: request.target?.propertyPath ?? "",
      }),
    unityPropertyApply: (request) =>
      measureRequest("unityPropertyApply", () => api.unityPropertyApply(request), {
        writeCount: request.writes.length,
      }),
    searchAssets: (query, roots, limit) =>
      measureRequest("searchAssets", () => api.searchAssets(query, roots, limit), {
        query,
        roots: roots?.join(",") ?? "",
        limit: limit ?? "",
      }),
    createSession: (request) =>
      measureRequest("createSession", () => api.createSession(request), {
        sessionType: request?.sessionType ?? "",
        agentId: request?.agentId ?? "",
      }),
    showSession: (sessionId) =>
      measureRequest("showSession", () => api.showSession(sessionId), { sessionId }),
    loadSession: (sessionId) =>
      measureRequest("loadSession", () => api.loadSession(sessionId), { sessionId }),
    getSessionActiveRun: (sessionId) =>
      measureRequest("getSessionActiveRun", () => api.getSessionActiveRun(sessionId), { sessionId }),
    listSessionEvents: (sessionId, afterSeq, limit) =>
      measureRequest("listSessionEvents", () => api.listSessionEvents(sessionId, afterSeq, limit), {
        sessionId,
        afterSeq: afterSeq ?? "",
        limit: limit ?? "",
      }),
    queueSessionInput: (request) =>
      measureRequest("queueSessionInput", () => api.queueSessionInput(request), {
        sessionId: request.sessionId,
        runId: request.runId,
        mergeGroupId: request.mergeGroupId,
      }),
    sendSessionMessage: (request) =>
      measureRequest("sendSessionMessage", () => api.sendSessionMessage(request), {
        sessionId: request.sessionId ?? "",
        sessionType: request.sessionType ?? "",
        textLength: request.text?.length ?? 0,
        wait: !!request.wait,
      }),
    waitSession: (request) =>
      measureRequest("waitSession", () => api.waitSession(request), {
        sessionId: request.sessionId,
        runId: request.runId ?? "",
        timeoutMs: request.timeoutMs ?? "",
      }),
    callLlm: (request) =>
      measureRequest("callLlm", () => api.callLlm(request), {
        sessionId: request.sessionId ?? "",
        promptLength: request.prompt?.length ?? 0,
        wait: !!request.wait,
      }),
    onSessionEvent: (handler) =>
      measureRequest("onSessionEvent", () => api.onSessionEvent(handler), {}, "subscription"),
    readFrontendLog: (limit) =>
      measureRequest("readFrontendLog", () => api.readFrontendLog(limit), { limit: limit ?? "" }, "log"),
    openFrontendLog: () =>
      measureRequest("openFrontendLog", () => api.openFrontendLog(), {}, "log"),
    storageGet: (key) =>
      measureRequest("storageGet", () => api.storageGet(key), { key }),
    storageSet: (key, value) =>
      measureRequest("storageSet", () => api.storageSet(key, value), {
        key,
        valueType: Array.isArray(value) ? "array" : typeof value,
      }),
    storageRemove: (key) =>
      measureRequest("storageRemove", () => api.storageRemove(key), { key }),
    onUpdate: (handler) =>
      measureRequest("onUpdate", () => api.onUpdate(handler), {}, "subscription"),
    reload: () =>
      measureRequest("reload", () => api.reload(), {}, "control"),
  };
}

export function createViewRuntimeComponent(options: ViewRuntimeComponentOptions): Component {
  const instrumentedApi = createInstrumentedRuntimeApi(options.detail, options.api);
  const runtime = createViewRuntimeApi(options.detail, instrumentedApi);
  installLegacyWindowApi(runtime);

  return markRaw(
    defineComponent({
      name: "LocusViewRuntimeRoot",
      setup() {
        const runtimeError = ref("");
        const loading = ref(true);
        const appComponent = shallowRef<Component | null>(null);
        const applyStyles = useViewRuntimeStyles(options.detail);
        let disposed = false;

        onBeforeUnmount(() => {
          disposed = true;
        });

        const prepare = async () => {
          const viewId = options.detail.manifest.id;
          const prepareStartedAt = perfNowMs();
          markStartupPhase("runtimePrepare_start", {
            viewId,
            entry: options.detail.manifest.entry,
            fileCount: options.detail.files.length,
          });
          try {
            const compilerImportStartedAt = perfNowMs();
            markStartupPhase("runtimeCompilerImport_start", { viewId });
            const { compileViewSfc, transformModuleSource } = await import("./viewSfcCompiler");
            markStartupPhase("runtimeCompilerImport_done", {
              viewId,
              elapsedMs: elapsedMs(compilerImportStartedAt),
            });
            const styles: string[] = [];
            const context: RuntimeContext = {
              detail: options.detail,
              api: instrumentedApi,
              styles,
              compileViewSfc,
              transformModuleSource,
              importModule: () => {
                throw new Error("View module loader is not ready.");
              },
            };
            const moduleLoaderStartedAt = perfNowMs();
            context.importModule = createModuleLoader(context);
            markStartupPhase("runtimeModuleLoader_ready", {
              viewId,
              elapsedMs: elapsedMs(moduleLoaderStartedAt),
            });
            const appPath = viewPackageRelPath(options.detail, "src/App.vue");
            const entryPath = viewPackageRelPath(options.detail, options.detail.manifest.entry);
            const entryImportStartedAt = perfNowMs();
            markStartupPhase("runtimeEntryImport_start", { viewId, entryPath });
            const entryExports = context.importModule(entryPath, appPath);
            markStartupPhase("runtimeEntryImport_done", {
              viewId,
              entryPath,
              elapsedMs: elapsedMs(entryImportStartedAt),
              styleCount: styles.length,
            });
            const componentResolveStartedAt = perfNowMs();
            const entryComponent = context.entryComponent
              ?? ((entryExports.default as Component | undefined) || undefined);
            const appFile = fileByPath(options.detail, appPath);
            const resolvedComponent = entryComponent
              ?? (appFile
                ? buildSfcComponent(context, appFile.content, appFile.relPath)
                : defineComponent({
                    setup: () => () => h("main", { class: "view-preview-empty" }, options.detail.manifest.name),
                  }));
            markStartupPhase("runtimeComponentResolve_done", {
              viewId,
              elapsedMs: elapsedMs(componentResolveStartedAt),
              usedEntryComponent: !!entryComponent,
              hasAppFile: !!appFile,
            });

            if (disposed) {
              markStartupPhase("runtimePrepare_aborted", {
                viewId,
                elapsedMs: elapsedMs(prepareStartedAt),
              });
              return;
            }
            const styleApplyStartedAt = perfNowMs();
            applyStyles(styles);
            markStartupPhase("runtimeStylesApply_done", {
              viewId,
              elapsedMs: elapsedMs(styleApplyStartedAt),
              styleCount: styles.length,
            });
            appComponent.value = markRaw(resolvedComponent);
            const firstInteractiveStartedAt = perfNowMs();
            markStartupPhase("runtimePrepare_done", {
              viewId,
              elapsedMs: elapsedMs(prepareStartedAt),
              styleCount: styles.length,
            });
            void nextTick().then(() => {
              afterNextFrame(() => {
                if (disposed) return;
                markStartupPhase("viewFirstInteractive", {
                  viewId,
                  elapsedMs: elapsedMs(prepareStartedAt),
                  sinceRuntimePrepareDoneMs: elapsedMs(firstInteractiveStartedAt),
                });
              });
            });
          } catch (prepareError) {
            if (disposed) return;
            runtimeError.value = prepareError instanceof Error
              ? prepareError.message
              : String(prepareError);
            markStartupPhase("runtimePrepare_error", {
              viewId,
              elapsedMs: elapsedMs(prepareStartedAt),
              message: runtimeError.value,
            });
            console.error("[view-runtime]", prepareError);
          } finally {
            if (!disposed) {
              loading.value = false;
            }
          }
        };

        void prepare();

        onErrorCaptured((capturedError) => {
          runtimeError.value = capturedError instanceof Error
            ? capturedError.message
            : String(capturedError);
          console.error("[view-runtime]", capturedError);
          return false;
        });
        return () => h("div", {
          class: "locus-view-runtime-root",
          onKeydownCapture: runtime.view.undo.handleKeydown,
        }, [
          runtimeError.value
            ? h("div", { class: "view-runtime-error" }, runtimeError.value)
            : appComponent.value
              ? h(appComponent.value)
              : h("div", { class: "view-runtime-loading" }, loading.value ? "Loading view..." : ""),
        ]);
      },
    }),
  );
}
