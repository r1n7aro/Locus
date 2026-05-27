import {
  defineComponent,
  h,
  markRaw,
  onBeforeUnmount,
  onErrorCaptured,
  reactive,
  readonly,
  ref,
  shallowRef,
  type Component,
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
  ViewRuntimeUpdateEvent,
} from "../../services/view";
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
import type { AssetSearchResult, SessionDetail, SessionEventRecord, SessionRunSummary, StreamEvent } from "../../types";
import * as PropertyTreeService from "../../services/propertyTree";

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
  UnityVectorField,
};

type ViewRuntimeModuleApi = ReturnType<typeof createViewRuntimeApiUncached>;

const runtimeApiCache = new WeakMap<ViewRuntimeApi, ViewRuntimeModuleApi>();
let undoEntrySequence = 0;

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
      const undoWrites = result.results
        .map((after, index) => before[index] && after.target
          ? {
            bindingId: after.bindingId,
            target: after.target,
            value: snapshotRestoreValue(before[index]!),
          }
          : null)
        .filter((item): item is ViewBindingApplyRequest["writes"][number] => !!item);
      const redoWrites = result.results
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
  const propertyDraw = {
    library: PropertyTreeService.publicInspectorPropertyDrawLibrary,
    projectLibrary: PropertyTreeService.projectInspectorPropertyDrawLibrary,
    register: PropertyTreeService.registerInspectorPropertyDrawComponent,
    createLibrary: PropertyTreeService.createInspectorPropertyDrawLibrary,
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
    undo,
    propertyDraw,
    drawLibrary: propertyDraw.library,
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
    undo,
    propertyDraw,
    defineView: <T>(value: T) => value,
    defineGraphView,
    CanvasView,
    GraphView,
    GraphViewController,
    UnityPropertyDraw,
    UnityPropertyEditor,
    UnitySerializedPropertyTree,
    layoutGraphDocument,
    ...PropertyTreeService,
    onEditorUpdate: (handler: (event: ViewRuntimeUpdateEvent) => void) => view.onUpdate(handler),
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
    unity: {
      callScript: runtime.view.callScript,
    },
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

export function createViewRuntimeComponent(options: ViewRuntimeComponentOptions): Component {
  const runtime = createViewRuntimeApi(options.detail, options.api);
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
          try {
            const { compileViewSfc, transformModuleSource } = await import("./viewSfcCompiler");
            const styles: string[] = [];
            const context: RuntimeContext = {
              detail: options.detail,
              api: options.api,
              styles,
              compileViewSfc,
              transformModuleSource,
              importModule: () => {
                throw new Error("View module loader is not ready.");
              },
            };
            context.importModule = createModuleLoader(context);
            const appPath = viewPackageRelPath(options.detail, "src/App.vue");
            const entryPath = viewPackageRelPath(options.detail, options.detail.manifest.entry);
            const entryExports = context.importModule(entryPath, appPath);
            const entryComponent = context.entryComponent
              ?? ((entryExports.default as Component | undefined) || undefined);
            const appFile = fileByPath(options.detail, appPath);
            const resolvedComponent = entryComponent
              ?? (appFile
                ? buildSfcComponent(context, appFile.content, appFile.relPath)
                : defineComponent({
                    setup: () => () => h("main", { class: "view-preview-empty" }, options.detail.manifest.name),
                  }));

            if (disposed) return;
            applyStyles(styles);
            appComponent.value = markRaw(resolvedComponent);
          } catch (prepareError) {
            if (disposed) return;
            runtimeError.value = prepareError instanceof Error
              ? prepareError.message
              : String(prepareError);
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
