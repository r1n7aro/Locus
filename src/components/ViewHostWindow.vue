<script setup lang="ts">
import {
  computed,
  markRaw,
  nextTick,
  onMounted,
  onUnmounted,
  ref,
  shallowRef,
  watch,
  type Component,
  type ComponentPublicInstance,
} from "vue";
import {
  cursorPosition,
  getCurrentWindow,
  Window as TauriWindow,
  type Window as TauriWindowHandle,
} from "@tauri-apps/api/window";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n";
import { searchWorkspaceAssets } from "../services/asset";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import { getLastEffort, getLastModel, getModelDefaults } from "../services/model";
import { markStartupPhase } from "../services/startupPerf";
import {
  chat as launchSessionChat,
  createSession as createLocusSession,
  getSessionActiveRun,
  listSessionEvents,
  loadSession as loadLocusSession,
  queueChatInput,
  saveActiveSessionSelection,
} from "../services/session";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import {
  checkUnityConnectionStatus,
  startLocusDragPreview,
  stopLocusDragPreview,
} from "../services/unity";
import {
  applyUnitySerializedProperties,
  discoverUnitySerializedProperties,
  readUnitySerializedProperty,
  writeUnitySerializedProperty,
} from "../services/unitySerializedProperty";
import {
  viewAppendFrontendLog,
  viewAutomationRespond,
  viewCallScript,
  viewContentHide,
  viewContentMount,
  viewDetachTab,
  viewFsAccess,
  viewFsAppendFile,
  viewFsCopyFile,
  viewFsLstat,
  viewFsMkdir,
  viewFsReadFile,
  viewFsReaddir,
  viewFsRename,
  viewFsRm,
  viewFsStat,
  viewFsUnlink,
  viewFsWriteFile,
  viewHostRevealed,
  viewHostPoolPrepare,
  viewHostPoolReady,
  viewHostIdFromLocation,
  isViewHostPoolWindowLocation,
  viewOpenFrontendLog,
  viewRead,
  viewReadFrontendLog,
  viewRequiresUnityConnection,
  viewSetTabHost,
  viewStorageGet,
  viewStorageRemove,
  viewStorageSet,
  type ViewFrontendLogEntry,
  type ViewFrontendLogLevel,
  type ViewAutomationRequest,
  type ViewContentMountRequest,
  type ViewLlmCallRequest,
  type ViewLlmCallResult,
  type ViewPackageDetail,
  type ViewPackageSummary,
  type ViewSessionChatRequest,
  type ViewSessionChatResult,
  type ViewSessionCreateRequest,
  type ViewSessionQueueInputRequest,
  type ViewSessionWaitRequest,
  type ViewSessionWaitResult,
  type ViewSessionWaitStatus,
  type ViewRuntimeUpdateEvent,
} from "../services/view";
import type {
  AppErrorPayload,
  ChatMessage,
  SessionDetail,
  SessionEventRecord,
  SessionRunSummary,
  StreamEvent,
} from "../types";
import { createViewRuntimeComponent } from "./view/viewRuntime";

const CONSOLE_LOG_LEVELS: ViewFrontendLogLevel[] = ["debug", "log", "info", "warn", "error"];
const AUTOMATION_REQUEST_TTL_MS = 120_000;
const VIEW_HOST_TABS_MERGE_EVENT = "view-host-tabs-merge";
const VIEW_HOST_TABS_MERGE_DONE_EVENT = "view-host-tabs-merge-done";
const VIEW_HOST_TABS_SELECT_EVENT = "view-host-tabs-select";
const VIEW_HOST_TABS_DROP_TARGET_EVENT = "view-host-tabs-drop-target";
const VIEW_HOST_WINDOW_LABEL_PREFIX = "view-";
const VIEW_HOST_TAB_DROP_HEIGHT_PX = 40;
const VIEW_HOST_TAB_DRAG_THRESHOLD_PX = 8;
const VIEW_HOST_TAB_DRAG_FRAME_MS = 16;
const VIEW_HOST_DETACH_OFFSET_X = 96;
const VIEW_HOST_DETACH_OFFSET_Y = 18;
const UNITY_EMBED_WINDOW_LABEL_PREFIX = "unity-embed-";
const VIEW_CONTENT_WINDOW_LABEL_PREFIX = "view-content-";
const VIEW_CONTENT_SYNC_FRAME_MS = 16;
const VIEW_HOST_CONTENT_DEBUG = false;

const props = withDefaults(defineProps<{
  embedded?: boolean;
}>(), {
  embedded: false,
});

interface ViewHostTab {
  id: string;
  title: string;
  packageRoot: string;
  icon?: string | null;
}

interface ViewHostTabsMergePayload {
  sourceLabel: string;
  viewIds: string[];
  activeViewId: string;
}

interface ViewHostTabsMergeDonePayload {
  targetLabel: string;
  viewIds: string[];
  activeViewId: string;
}

interface ViewHostTabsSelectPayload {
  viewId: string;
  targetLabel?: string;
  allowPoolClaim?: boolean;
}

interface ViewHostTabsDropTargetPayload {
  sourceLabel: string;
  active: boolean;
}

interface ViewHostDragState {
  pointerId: number;
  tabId: string;
  originX: number;
  originY: number;
  cursorX: number;
  cursorY: number;
  dragging: boolean;
  raf: number | null;
}

interface ViewRuntimeRecord {
  viewId: string;
  detail: ViewPackageDetail | null;
  component: Component | null;
  loading: boolean;
  error: string;
  latestFrontendLog: ViewFrontendLogEntry | null;
  loadPromise: Promise<void> | null;
  reloadQueued: boolean;
  geometrySyncQueued: boolean;
  stale: boolean;
}

interface ViewContentMountGeometry {
  viewId: string;
  hostLabel: string;
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
}

interface AutomationPoint {
  x: number;
  y: number;
}

let appWindow: TauriWindowHandle | null = null;
if (hasTauriWindowRuntime()) {
  try {
    appWindow = getCurrentWindow();
  } catch {
    appWindow = null;
  }
}

const initialViewId = viewHostIdFromLocation();
const isViewHostPoolWindow = isViewHostPoolWindowLocation();
const currentWindowLabel = appWindow?.label ?? "";
const activeViewId = ref(initialViewId);
const tabs = ref<ViewHostTab[]>(initialViewId
  ? [{ id: initialViewId, title: initialViewId, packageRoot: "" }]
  : []);
const runtimeRecords = ref<ViewRuntimeRecord[]>(initialViewId
  ? [{
      viewId: initialViewId,
      detail: null,
      component: null,
      loading: false,
      error: "",
      latestFrontendLog: null,
      loadPromise: null,
      reloadQueued: false,
      geometrySyncQueued: false,
      stale: true,
    }]
  : []);
const isMaximized = ref(false);
const alwaysOnTop = ref(false);
const tabDragState = ref<ViewHostDragState | null>(null);
const tabDropTargetLabel = ref("");
const externalTabDropActive = ref(false);
const embeddedLogbarSlot = shallowRef<HTMLElement | null>(null);
const viewHostBodyRef = ref<HTMLElement | null>(null);
const runtimeFrameRefs = new Map<string, HTMLElement>();
let unsubscribeReload: RuntimeUnsubscribe | null = null;
let restoreConsoleLogCapture: (() => void) | null = null;
let unlistenTabMerge: UnlistenFn | null = null;
let unlistenTabSelect: UnlistenFn | null = null;
let unlistenTabDropTarget: UnlistenFn | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let suppressNextTabClickId = "";
let suppressNextTabClickTimer: ReturnType<typeof setTimeout> | null = null;
let statusbarObserver: MutationObserver | null = null;
let embeddedLogbarSyncTimer: ReturnType<typeof setTimeout> | null = null;
let viewContentSyncTimer: ReturnType<typeof setTimeout> | null = null;
let viewContentResizeObserver: ResizeObserver | null = null;
let unlistenWindowResize: UnlistenFn | null = null;
let unlistenWindowMove: UnlistenFn | null = null;
let unsubscribeAutomation: RuntimeUnsubscribe | null = null;
let automationElementSeq = 0;
let lastEmittedTabDropTargetLabel = "";
let externalTabDropSourceLabel = "";
let nativeTabDragPreviewActive = false;
let hostWindowRevealStarted = false;
let poolPreparePromise: Promise<void> | null = null;
let lastViewContentMountGeometry: ViewContentMountGeometry | null = null;
const handledAutomationRequests = new Map<string, number>();

const RUNTIME_STATUSBAR_SELECTOR = [
  "[data-locus-view-statusbar]",
  "footer.table-statusbar",
  ".table-statusbar",
  "footer.view-statusbar",
  "footer.view-status-bar",
  "footer.statusbar",
  "footer.status-bar",
].join(", ");

function perfNowMs(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function elapsedMs(startedAt: number): number {
  return Math.round(perfNowMs() - startedAt);
}

const hostScriptStartedAt = perfNowMs();
let hostMountedAt = 0;

function createRuntimeRecord(viewId: string): ViewRuntimeRecord {
  return {
    viewId,
    detail: null,
    component: null,
    loading: false,
    error: "",
    latestFrontendLog: null,
    loadPromise: null,
    reloadQueued: false,
    geometrySyncQueued: false,
    stale: true,
  };
}

function findRuntimeRecord(viewId: string): ViewRuntimeRecord | null {
  return runtimeRecords.value.find((record) => record.viewId === viewId) ?? null;
}

function ensureRuntimeRecord(viewId: string): ViewRuntimeRecord {
  const existing = findRuntimeRecord(viewId);
  if (existing) return existing;
  const record = createRuntimeRecord(viewId);
  runtimeRecords.value = [...runtimeRecords.value, record];
  return record;
}

function removeRuntimeRecord(viewId: string) {
  runtimeRecords.value = runtimeRecords.value.filter((record) => record.viewId !== viewId);
  runtimeFrameRefs.delete(viewId);
}

function runtimeLabelForRecord(record: ViewRuntimeRecord): string {
  return record.detail?.manifest.name
    || record.detail?.summary.name
    || tabs.value.find((tab) => tab.id === record.viewId)?.title
    || record.viewId
    || t("view.host.untitled");
}

function activeRuntimeFrame(): HTMLElement | null {
  return activeViewId.value ? runtimeFrameRefs.get(activeViewId.value) ?? null : null;
}

function setRuntimeFrameRef(viewId: string, value: Element | ComponentPublicInstance | null) {
  let element: HTMLElement | null = null;
  if (value instanceof HTMLElement) {
    element = value;
  } else if (value && !(value instanceof Element) && value.$el instanceof HTMLElement) {
    element = value.$el;
  }
  if (element) {
    runtimeFrameRefs.set(viewId, element);
  } else {
    runtimeFrameRefs.delete(viewId);
  }
  if (viewId === activeViewId.value) scheduleEmbeddedLogbarSync();
}

const activeRuntimeRecord = computed(() => findRuntimeRecord(activeViewId.value));
const visibleRuntimeRecords = computed(() => {
  const visibleIds = new Set(tabs.value.map((tab) => tab.id));
  return runtimeRecords.value.filter((record) => visibleIds.has(record.viewId));
});
const detail = computed(() => activeRuntimeRecord.value?.detail ?? null);
const runtimeComponent = computed(() => activeRuntimeRecord.value?.component ?? null);
const loading = computed(() => activeRuntimeRecord.value?.loading ?? false);
const error = computed(() => activeRuntimeRecord.value?.error ?? "");
const latestFrontendLog = computed(() => activeRuntimeRecord.value?.latestFrontendLog ?? null);
const usePersistentViewContentPool = computed(() => !props.embedded && !!appWindow);
const manifest = computed(() => detail.value?.manifest ?? null);
const activeTab = computed(() => tabs.value.find((tab) => tab.id === activeViewId.value) ?? null);
const windowTitle = computed(() =>
  manifest.value?.name || detail.value?.summary.name || activeTab.value?.title || activeViewId.value || t("view.host.title"),
);
const latestFrontendLogLevel = computed(() => latestFrontendLog.value?.level ?? "log");
const latestFrontendLogText = computed(() => {
  const entry = latestFrontendLog.value;
  if (!entry) return "No frontend log";
  const message = firstLogLine(entry.message);
  return message ? `${entry.level.toUpperCase()} ${message}` : `${entry.level.toUpperCase()} empty message`;
});

function viewHostContentLog(event: string, detail: Record<string, unknown> = {}) {
  if (!VIEW_HOST_CONTENT_DEBUG) return;
  const now = perfNowMs();
  const payload = {
    hostLabel: currentWindowLabel,
    activeViewId: activeViewId.value,
    tabs: tabs.value.map((tab) => tab.id).join(","),
    embedded: props.embedded,
    poolWindow: isViewHostPoolWindow,
    persistentPool: usePersistentViewContentPool.value,
    sinceScriptStartMs: Math.round(now - hostScriptStartedAt),
    sinceHostMountedMs: hostMountedAt > 0 ? Math.round(now - hostMountedAt) : "",
    ...detail,
  };
  console.info(`[view-host:content] ${event}`, payload);
}

async function revealViewHostWindow(reason: string) {
  if (props.embedded || !appWindow || hostWindowRevealStarted) return;
  hostWindowRevealStarted = true;
  const revealStartedAt = perfNowMs();
  viewHostContentLog("host-reveal-start", { reason });
  try {
    await appWindow.show();
    await appWindow.setFocus().catch((focusError) => {
      viewHostContentLog("host-reveal-focus-failed", {
        reason,
        message: normalizeAppError(focusError).message,
      });
    });
    if (currentWindowLabel) {
      await viewHostRevealed(currentWindowLabel).catch((ownerError) => {
        viewHostContentLog("host-reveal-owner-sync-failed", {
          reason,
          message: normalizeAppError(ownerError).message,
        });
      });
    }
    viewHostContentLog("host-reveal-done", {
      reason,
      elapsedMs: elapsedMs(revealStartedAt),
    });
  } catch (revealError) {
    hostWindowRevealStarted = false;
    viewHostContentLog("host-reveal-error", {
      reason,
      elapsedMs: elapsedMs(revealStartedAt),
      message: normalizeAppError(revealError).message,
    });
  }
}

function prepareViewHostPool(reason: string) {
  if (props.embedded || isViewHostPoolWindow || !appWindow) return;
  if (poolPreparePromise) return;
  const prepareStartedAt = perfNowMs();
  viewHostContentLog("pool-prepare-start", { reason });
  poolPreparePromise = viewHostPoolPrepare()
    .then((result) => {
      viewHostContentLog("pool-prepare-done", {
        reason,
        label: result.windowLabel,
        elapsedMs: elapsedMs(prepareStartedAt),
      });
    })
    .catch((prepareError) => {
      viewHostContentLog("pool-prepare-error", {
        reason,
        elapsedMs: elapsedMs(prepareStartedAt),
        message: normalizeAppError(prepareError).message,
      });
    })
    .finally(() => {
      poolPreparePromise = null;
    });
}

function firstLogLine(message: string) {
  return String(message || "").split(/\r?\n/, 1)[0]?.trim() ?? "";
}

async function syncMaximizedState() {
  if (!appWindow) return;
  try {
    isMaximized.value = await appWindow.isMaximized();
  } catch {
    isMaximized.value = false;
  }
}

async function syncAlwaysOnTopState() {
  if (!appWindow) return;
  try {
    alwaysOnTop.value = await appWindow.isAlwaysOnTop();
  } catch {
    alwaysOnTop.value = false;
  }
}

async function minimizeWindow() {
  await appWindow?.minimize().catch(() => undefined);
}

async function toggleAlwaysOnTop() {
  if (!appWindow) return;
  alwaysOnTop.value = !alwaysOnTop.value;
  try {
    await appWindow.setAlwaysOnTop(alwaysOnTop.value);
  } catch (topError) {
    alwaysOnTop.value = !alwaysOnTop.value;
    console.error("[view-host] Failed to set always on top", topError);
  }
}

async function toggleMaximizeWindow() {
  if (!appWindow) return;
  await appWindow.toggleMaximize().catch(() => undefined);
  await syncMaximizedState();
}

async function closeWindow() {
  if (appWindow) {
    try {
      await appWindow.close();
      return;
    } catch {
      await appWindow.destroy().catch(() => undefined);
      return;
    }
  }
  window.close();
}

function installViewConsoleLogCapture(activeViewId: string) {
  if (!activeViewId) return () => {};

  const consoleForCapture = console as unknown as Record<
    ViewFrontendLogLevel,
    (...args: unknown[]) => void
  >;
  const originals = new Map<ViewFrontendLogLevel, (...args: unknown[]) => void>();

  const appendLog = (level: ViewFrontendLogLevel, args: unknown[]) => {
    const message = formatConsoleArgs(args);
    const record = ensureRuntimeRecord(activeViewId);
    record.latestFrontendLog = {
      time: Date.now(),
      level,
      message,
    };
    void viewAppendFrontendLog({ viewId: activeViewId, level, message }).catch(() => undefined);
  };

  for (const level of CONSOLE_LOG_LEVELS) {
    const original = consoleForCapture[level]?.bind(console) ?? (() => undefined);
    originals.set(level, original);
    consoleForCapture[level] = (...args: unknown[]) => {
      original(...args);
      appendLog(level, args);
    };
  }

  const handleWindowError = (event: ErrorEvent) => {
    appendLog("error", [formatErrorEvent(event)]);
  };
  const handleUnhandledRejection = (event: PromiseRejectionEvent) => {
    appendLog("error", ["Unhandled promise rejection", event.reason]);
  };

  window.addEventListener("error", handleWindowError);
  window.addEventListener("unhandledrejection", handleUnhandledRejection);

  return () => {
    for (const [level, original] of originals) {
      consoleForCapture[level] = original;
    }
    window.removeEventListener("error", handleWindowError);
    window.removeEventListener("unhandledrejection", handleUnhandledRejection);
  };
}

function formatConsoleArgs(args: unknown[]) {
  return args.map((arg) => formatConsoleValue(arg)).join(" ");
}

function formatConsoleValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack || value.message;
  if (value === undefined) return "undefined";
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "symbol") return value.toString();
  if (typeof value === "function") return `[Function ${value.name || "anonymous"}]`;
  if (value === null || typeof value !== "object") return String(value);

  const seen = new WeakSet<object>();
  try {
    return JSON.stringify(value, (_key, nestedValue: unknown) => {
      if (typeof nestedValue === "bigint") return nestedValue.toString();
      if (typeof nestedValue === "function") {
        return `[Function ${nestedValue.name || "anonymous"}]`;
      }
      if (nestedValue && typeof nestedValue === "object") {
        if (seen.has(nestedValue)) return "[Circular]";
        seen.add(nestedValue);
      }
      return nestedValue;
    });
  } catch {
    return String(value);
  }
}

function formatErrorEvent(event: ErrorEvent) {
  const location = [event.filename, event.lineno, event.colno].filter(Boolean).join(":");
  const stack = event.error instanceof Error ? event.error.stack : "";
  return [event.message || "Uncaught error", location, stack].filter(Boolean).join("\n");
}

async function refreshLatestFrontendLog() {
  const viewId = activeViewId.value;
  if (!viewId) return;
  const record = ensureRuntimeRecord(viewId);
  try {
    const entries = await viewReadFrontendLog({ viewId, limit: 1 });
    record.latestFrontendLog = entries[entries.length - 1] ?? null;
  } catch {
    record.latestFrontendLog = null;
  }
}

async function openFrontendLog() {
  const viewId = activeViewId.value;
  if (!viewId) return;
  try {
    await viewOpenFrontendLog(viewId);
  } catch (openError) {
    console.error("[view-host] Failed to open frontend log", openError);
  }
}

function tabFromDetail(next: ViewPackageDetail): ViewHostTab {
  return {
    id: next.manifest.id,
    title: next.manifest.name || next.summary.name || next.manifest.id,
    packageRoot: next.summary.packageRoot,
    icon: next.manifest.icon ?? next.summary.icon ?? null,
  };
}

function tabFromSummary(summary: ViewPackageSummary): ViewHostTab {
  return {
    id: summary.id,
    title: summary.name || summary.id,
    packageRoot: summary.packageRoot,
    icon: summary.icon ?? null,
  };
}

function normalizeTabIds(ids: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const id of ids) {
    const value = String(id || "").trim();
    if (!value || seen.has(value)) continue;
    seen.add(value);
    normalized.push(value);
  }
  return normalized;
}

function upsertTab(tab: ViewHostTab) {
  const index = tabs.value.findIndex((item) => item.id === tab.id);
  if (index >= 0) {
    tabs.value = tabs.value.map((item, itemIndex) => itemIndex === index ? { ...item, ...tab } : item);
    return;
  }
  tabs.value = [...tabs.value, tab];
}

function removeTab(tabId: string, options: { releaseContent?: boolean } = {}): ViewHostTab | null {
  const index = tabs.value.findIndex((tab) => tab.id === tabId);
  if (index < 0) return null;
  const removed = tabs.value[index];
  const nextTabs = tabs.value.filter((tab) => tab.id !== tabId);
  viewHostContentLog("remove-tab", {
    tabId,
    releaseContent: options.releaseContent !== false,
    nextTabs: nextTabs.map((tab) => tab.id).join(","),
  });
  tabs.value = nextTabs;
  removeRuntimeRecord(tabId);
  if (usePersistentViewContentPool.value && options.releaseContent !== false) {
    viewHostContentLog("hide-from-remove-tab", { tabId });
    void viewContentHide(tabId);
  }
  if (activeViewId.value === tabId) {
    const nextActive = nextTabs[Math.min(index, nextTabs.length - 1)]?.id ?? "";
    activeViewId.value = nextActive;
    clearEmbeddedLogbarSlot();
    refreshConsoleLogCapture();
    if (nextActive) {
      const record = ensureRuntimeRecord(nextActive);
      if (record.component && !record.stale && !record.error) {
        void nextTick().then(installStatusbarObserver);
      } else {
        scheduleLoadView(0, nextActive);
      }
      void refreshLatestFrontendLog();
    }
  }
  return removed;
}

async function resolveViewTab(id: string): Promise<ViewHostTab> {
  if (detail.value?.manifest.id === id) return tabFromDetail(detail.value);
  const existing = tabs.value.find((tab) => tab.id === id);
  try {
    const next = await viewRead(id);
    return tabFromDetail(next);
  } catch {
    return existing ?? { id, title: id, packageRoot: "" };
  }
}

async function registerCurrentTabHost() {
  const canRegisterHost = currentWindowLabel
    && !currentWindowLabel.startsWith(VIEW_CONTENT_WINDOW_LABEL_PREFIX)
    && (currentWindowLabel.startsWith(VIEW_HOST_WINDOW_LABEL_PREFIX)
      || (props.embedded && currentWindowLabel.startsWith(UNITY_EMBED_WINDOW_LABEL_PREFIX)));
  if (!canRegisterHost) return;
  const viewIds = normalizeTabIds(tabs.value.map((tab) => tab.id));
  if (viewIds.length === 0) return;
  const registerStartedAt = perfNowMs();
  markStartupPhase("registerHost_start", {
    hostLabel: currentWindowLabel,
    viewIds: viewIds.join(","),
  });
  try {
    await viewSetTabHost({ hostLabel: currentWindowLabel, viewIds });
    markStartupPhase("registerHost_done", {
      hostLabel: currentWindowLabel,
      viewIds: viewIds.join(","),
      elapsedMs: elapsedMs(registerStartedAt),
    });
  } catch (hostError) {
    markStartupPhase("registerHost_error", {
      hostLabel: currentWindowLabel,
      viewIds: viewIds.join(","),
      elapsedMs: elapsedMs(registerStartedAt),
      message: normalizeAppError(hostError).message,
    });
    console.warn("[view-host] Failed to register tab host", hostError);
  }
}

function refreshConsoleLogCapture() {
  restoreConsoleLogCapture?.();
  restoreConsoleLogCapture = activeViewId.value
    ? installViewConsoleLogCapture(activeViewId.value)
    : null;
}

async function setActiveViewTab(viewId: string, options: { loadNow?: boolean } = {}) {
  const normalized = String(viewId || "").trim();
  if (!normalized) return;
  const previousActiveViewId = activeViewId.value;
  if (!tabs.value.some((tab) => tab.id === normalized)) {
    upsertTab(await resolveViewTab(normalized));
  }
  const record = ensureRuntimeRecord(normalized);
  const changed = activeViewId.value !== normalized;
  viewHostContentLog("set-active", {
    viewId: normalized,
    previousActiveViewId,
    changed,
    loadNow: !!options.loadNow,
    recordHasDetail: !!record.detail,
    recordHasComponent: !!record.component,
    recordStale: record.stale,
  });
  if (changed) {
    activeViewId.value = normalized;
    clearEmbeddedLogbarSlot();
    refreshConsoleLogCapture();
  }
  void refreshLatestFrontendLog();
  await registerCurrentTabHost();
  if (options.loadNow) {
    const inFlightLoad = record.loadPromise;
    if (inFlightLoad) await inFlightLoad.catch(() => undefined);
    await loadView(normalized);
  } else if (record.component && !record.stale && !record.error) {
    void nextTick().then(installStatusbarObserver);
  } else if (changed) {
    viewHostContentLog("set-active-schedule-load", { viewId: normalized });
    scheduleLoadView(0, normalized);
  }
}

function suppressNextTabClick(tabId: string) {
  suppressNextTabClickId = tabId;
  if (suppressNextTabClickTimer) clearTimeout(suppressNextTabClickTimer);
  suppressNextTabClickTimer = setTimeout(() => {
    if (suppressNextTabClickId === tabId) suppressNextTabClickId = "";
    suppressNextTabClickTimer = null;
  }, 250);
}

function onTabClick(event: MouseEvent, tabId: string) {
  if (suppressNextTabClickId === tabId) {
    event.preventDefault();
    suppressNextTabClickId = "";
    if (suppressNextTabClickTimer) {
      clearTimeout(suppressNextTabClickTimer);
      suppressNextTabClickTimer = null;
    }
    return;
  }
  void setActiveViewTab(tabId);
}

async function applyMergedViewTabs(payload: ViewHostTabsMergePayload) {
  const incomingIds = normalizeTabIds(payload.viewIds);
  if (incomingIds.length === 0) return;
  const nextIds = normalizeTabIds([...tabs.value.map((tab) => tab.id), ...incomingIds]);
  const nextTabs = await Promise.all(nextIds.map((id) => resolveViewTab(id)));
  tabs.value = nextTabs;
  await setActiveViewTab(
    payload.activeViewId || incomingIds[incomingIds.length - 1] || nextIds[0] || activeViewId.value,
    { loadNow: true },
  );
  await registerCurrentTabHost();
  if (payload.sourceLabel && currentWindowLabel) {
    void emitTo<ViewHostTabsMergeDonePayload>(
      payload.sourceLabel,
      VIEW_HOST_TABS_MERGE_DONE_EVENT,
      {
        targetLabel: currentWindowLabel,
        viewIds: incomingIds,
        activeViewId: activeViewId.value,
      },
    ).catch((mergeDoneError) => {
      console.warn("[view-host] Failed to confirm View tab merge", mergeDoneError);
    });
  }
}

async function selectHostedViewTab(payload: ViewHostTabsSelectPayload) {
  const claimStartedAt = perfNowMs();
  const targetLabel = String(payload.targetLabel || "").trim();
  if (targetLabel && currentWindowLabel && targetLabel !== currentWindowLabel) {
    viewHostContentLog("claim-ignored-target", {
      viewId: payload.viewId,
      targetLabel,
      elapsedMs: elapsedMs(claimStartedAt),
    });
    return;
  }
  const hasHostedTab = tabs.value.some((tab) => tab.id === payload.viewId);
  const canClaimPoolTab = isViewHostPoolWindow
    && tabs.value.length === 0
    && payload.allowPoolClaim === true;
  viewHostContentLog("claim-start", {
    viewId: payload.viewId,
    targetLabel,
    allowPoolClaim: payload.allowPoolClaim === true,
    hasTab: hasHostedTab,
    canClaimPoolTab,
  });
  if (!hasHostedTab && !canClaimPoolTab) {
    await registerCurrentTabHost();
    viewHostContentLog("claim-ignored", {
      viewId: payload.viewId,
      elapsedMs: elapsedMs(claimStartedAt),
    });
    return;
  }
  await setActiveViewTab(payload.viewId, { loadNow: true });
  viewHostContentLog("claim-mounted", {
    viewId: payload.viewId,
    elapsedMs: elapsedMs(claimStartedAt),
  });
  await revealViewHostWindow("claim-mounted");
}

async function findTabDropTargetAt(point: { x: number; y: number }): Promise<string> {
  if (!currentWindowLabel) return "";
  let windows: TauriWindowHandle[] = [];
  try {
    windows = await TauriWindow.getAll();
  } catch {
    return "";
  }

  for (const candidate of windows) {
    if (
      candidate.label === currentWindowLabel
      || !candidate.label.startsWith(VIEW_HOST_WINDOW_LABEL_PREFIX)
      || candidate.label.startsWith(VIEW_CONTENT_WINDOW_LABEL_PREFIX)
    ) {
      continue;
    }
    try {
      const [position, size] = await Promise.all([
        candidate.outerPosition(),
        candidate.outerSize(),
      ]);
      const withinX = point.x >= position.x && point.x <= position.x + size.width;
      const withinY = point.y >= position.y && point.y <= position.y + size.height;
      const withinDropBand = point.y >= position.y && point.y <= position.y + VIEW_HOST_TAB_DROP_HEIGHT_PX;
      if (withinX && withinY && withinDropBand) return candidate.label;
    } catch {
      continue;
    }
  }
  return "";
}

function emitTabDropTargetState(targetLabel: string, active: boolean) {
  if (!currentWindowLabel || !targetLabel || targetLabel === currentWindowLabel) return;
  void emitTo<ViewHostTabsDropTargetPayload>(targetLabel, VIEW_HOST_TABS_DROP_TARGET_EVENT, {
    sourceLabel: currentWindowLabel,
    active,
  }).catch((dropTargetError) => {
    console.warn("[view-host] Failed to update View tab drop target", dropTargetError);
  });
}

function setTabDropTargetLabel(nextLabel: string) {
  const normalized = nextLabel || "";
  if (normalized === tabDropTargetLabel.value) return;
  if (lastEmittedTabDropTargetLabel) {
    emitTabDropTargetState(lastEmittedTabDropTargetLabel, false);
  }
  tabDropTargetLabel.value = normalized;
  lastEmittedTabDropTargetLabel = normalized;
  if (normalized) {
    emitTabDropTargetState(normalized, true);
  }
}

async function isCurrentTabBandAt(point: { x: number; y: number }): Promise<boolean> {
  if (!appWindow) return false;
  try {
    const [position, size] = await Promise.all([
      appWindow.outerPosition(),
      appWindow.outerSize(),
    ]);
    const withinX = point.x >= position.x && point.x <= position.x + size.width;
    const withinY = point.y >= position.y && point.y <= position.y + size.height;
    const withinTabBand = point.y >= position.y && point.y <= position.y + VIEW_HOST_TAB_DROP_HEIGHT_PX;
    return withinX && withinY && withinTabBand;
  } catch {
    return false;
  }
}

async function updateTabDropTarget(point: { x: number; y: number }) {
  setTabDropTargetLabel(await findTabDropTargetAt(point));
}

function createTabMergeDoneWaiter(
  targetLabel: string,
  viewIds: string[],
): { ready: Promise<void>; done: Promise<void> } {
  if (!appWindow || !targetLabel || viewIds.length === 0) {
    return { ready: Promise.resolve(), done: Promise.resolve() };
  }
  const expectedIds = new Set(viewIds);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let resolveDone: () => void = () => undefined;
  let unlisten: UnlistenFn | null = null;
  let doneSettled = false;
  const done = new Promise<void>((resolve) => {
    resolveDone = resolve;
    timer = setTimeout(resolve, 1000);
  });
  const ready = appWindow.listen<ViewHostTabsMergeDonePayload>(
    VIEW_HOST_TABS_MERGE_DONE_EVENT,
    (event) => {
      const payload = event.payload;
      if (payload.targetLabel !== targetLabel) return;
      const receivedIds = new Set(normalizeTabIds(payload.viewIds));
      for (const viewId of expectedIds) {
        if (!receivedIds.has(viewId)) return;
      }
      resolveDone();
    },
  ).then((nextUnlisten) => {
    if (doneSettled) {
      nextUnlisten();
      return;
    }
    unlisten = nextUnlisten;
  }).catch(() => {
    resolveDone();
  });
  return {
    ready,
    done: done.finally(() => {
      doneSettled = true;
      if (timer) clearTimeout(timer);
      unlisten?.();
    }),
  };
}

function tabDragPreviewLabel(tabId: string): string {
  const tab = tabs.value.find((item) => item.id === tabId);
  return (tab?.title || tabId || t("view.host.title")).trim();
}

function startTabDragPreview(tabId: string) {
  if (nativeTabDragPreviewActive) return;
  nativeTabDragPreviewActive = true;
  void startLocusDragPreview(tabDragPreviewLabel(tabId)).catch((previewError) => {
    nativeTabDragPreviewActive = false;
    console.warn("[view-host] Failed to start View tab drag preview", previewError);
  });
}

function stopTabDragPreview() {
  if (!nativeTabDragPreviewActive) return;
  nativeTabDragPreviewActive = false;
  void stopLocusDragPreview().catch((previewError) => {
    console.warn("[view-host] Failed to stop View tab drag preview", previewError);
  });
}

function scheduleTabDragFrame() {
  const state = tabDragState.value;
  if (!state || state.raf !== null) return;
  state.raf = window.setTimeout(() => {
    const current = tabDragState.value;
    if (!current) return;
    current.raf = null;
    void updateDraggedTabFrame();
  }, VIEW_HOST_TAB_DRAG_FRAME_MS);
}

async function updateDraggedTabFrame() {
  const state = tabDragState.value;
  if (!state || !state.dragging) return;
  try {
    const cursor = await cursorPosition();
    state.cursorX = cursor.x;
    state.cursorY = cursor.y;
    await updateTabDropTarget(cursor);
  } catch (dragError) {
    console.warn("[view-host] Failed to inspect View tab drag", dragError);
  }
  if (tabDragState.value?.dragging) scheduleTabDragFrame();
}

function onTabDragPointerMove(event: PointerEvent) {
  const state = tabDragState.value;
  if (!state || event.pointerId !== state.pointerId) return;
  const distance = Math.hypot(event.screenX - state.originX, event.screenY - state.originY);
  if (!state.dragging && distance >= VIEW_HOST_TAB_DRAG_THRESHOLD_PX) {
    state.dragging = true;
    startTabDragPreview(state.tabId);
    prepareViewHostPool("tab-drag");
    scheduleTabDragFrame();
  }
  if (state.dragging) event.preventDefault();
}

async function mergeTabIntoWindow(tabId: string, targetLabel: string) {
  if (!targetLabel || targetLabel === currentWindowLabel) return;
  const tab = tabs.value.find((item) => item.id === tabId);
  if (!tab) return;
  viewHostContentLog("merge-start", { tabId, targetLabel });
  const mergeDone = createTabMergeDoneWaiter(targetLabel, [tab.id]);
  await mergeDone.ready;
  await emitTo<ViewHostTabsMergePayload>(targetLabel, VIEW_HOST_TABS_MERGE_EVENT, {
    sourceLabel: currentWindowLabel,
    viewIds: [tab.id],
    activeViewId: tab.id,
  });
  await viewSetTabHost({
    hostLabel: targetLabel,
    viewIds: [tab.id],
    keepExistingForHost: true,
  }).catch((hostError) => {
    console.warn("[view-host] Failed to pre-register merged View tab", hostError);
  });
  removeTab(tab.id, { releaseContent: false });
  viewHostContentLog("merge-source-removed", { tabId, targetLabel });
  if (tabs.value.length > 0) {
    await registerCurrentTabHost();
  }
  const targetWindow = await TauriWindow.getByLabel(targetLabel).catch(() => null);
  await targetWindow?.setFocus().catch(() => undefined);
  if (tabs.value.length === 0) {
    await mergeDone.done;
    viewHostContentLog("merge-close-empty-source", { tabId, targetLabel });
    await appWindow?.close().catch(() => appWindow?.destroy().catch(() => undefined));
  }
}

async function detachTab(tabId: string, point: { x: number; y: number }) {
  if (tabs.value.length <= 1) return;
  const tab = tabs.value.find((item) => item.id === tabId);
  if (!tab) return;
  viewHostContentLog("detach-start", {
    tabId,
    releaseX: point.x,
    releaseY: point.y,
    tabCount: tabs.value.length,
  });
  removeTab(tab.id, { releaseContent: false });
  viewHostContentLog("detach-source-removed", { tabId });
  await registerCurrentTabHost();
  try {
    const result = await viewDetachTab({
      viewId: tab.id,
      sourceHostLabel: currentWindowLabel,
      x: Math.round(point.x - VIEW_HOST_DETACH_OFFSET_X),
      y: Math.round(point.y - VIEW_HOST_DETACH_OFFSET_Y),
    });
    viewHostContentLog("detach-command-done", {
      tabId,
      targetLabel: result.windowLabel,
      hostUrl: result.hostUrl,
    });
  } catch (detachError) {
    upsertTab(tab);
    await setActiveViewTab(tab.id, { loadNow: true });
    console.error("[view-host] Failed to detach View tab", detachError);
  }
}

async function finishTabDrag(event?: PointerEvent) {
  const state = tabDragState.value;
  if (!state) return;
  if (event && event.pointerId !== state.pointerId) return;
  if (state.raf !== null) clearTimeout(state.raf);
  tabDragState.value = null;
  window.removeEventListener("pointermove", onTabDragPointerMove);
  window.removeEventListener("pointerup", onTabDragPointerUp);
  window.removeEventListener("pointercancel", onTabDragPointerCancel);

  const wasDragging = state.dragging;
  if (wasDragging) {
    stopTabDragPreview();
  }
  if (!wasDragging) {
    setTabDropTargetLabel("");
    return;
  }
  suppressNextTabClick(state.tabId);
  let targetLabel = tabDropTargetLabel.value;
  let releasePoint = { x: state.cursorX, y: state.cursorY };
  try {
    const cursor = await cursorPosition();
    releasePoint = { x: cursor.x, y: cursor.y };
    targetLabel = await findTabDropTargetAt(cursor) || targetLabel;
  } catch {
    targetLabel = targetLabel || "";
  }
  setTabDropTargetLabel("");
  if (targetLabel) {
    await mergeTabIntoWindow(state.tabId, targetLabel);
    return;
  }
  if (await isCurrentTabBandAt(releasePoint)) {
    return;
  }
  await detachTab(state.tabId, releasePoint);
}

function applyExternalTabDropTarget(payload: ViewHostTabsDropTargetPayload) {
  if (!payload.sourceLabel || payload.sourceLabel === currentWindowLabel) return;
  if (payload.active) {
    externalTabDropSourceLabel = payload.sourceLabel;
    externalTabDropActive.value = true;
    return;
  }
  if (!externalTabDropSourceLabel || externalTabDropSourceLabel === payload.sourceLabel) {
    externalTabDropSourceLabel = "";
    externalTabDropActive.value = false;
  }
}

function onTabDragPointerUp(event: PointerEvent) {
  void finishTabDrag(event);
}

function onTabDragPointerCancel(event: PointerEvent) {
  void finishTabDrag(event);
}

async function startTabDrag(event: PointerEvent, tabId: string) {
  if (!appWindow || event.button !== 0 || event.detail > 1) return;
  event.preventDefault();
  if (tabId !== activeViewId.value) {
    viewHostContentLog("drag-activate-request", {
      tabId,
      previousActiveViewId: activeViewId.value,
    });
    void setActiveViewTab(tabId)
      .then(() => {
        viewHostContentLog("drag-activate-done", {
          tabId,
          stillHosted: tabs.value.some((tab) => tab.id === tabId),
        });
      })
      .catch((activateError) => {
        viewHostContentLog("drag-activate-error", {
          tabId,
          message: normalizeAppError(activateError).message,
        });
      });
  }
  let cursor: { x: number; y: number };
  try {
    cursor = await cursorPosition();
  } catch (dragError) {
    console.warn("[view-host] Failed to start View tab drag", dragError);
    return;
  }
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  tabDragState.value = {
    pointerId: event.pointerId,
    tabId,
    originX: event.screenX,
    originY: event.screenY,
    cursorX: cursor.x,
    cursorY: cursor.y,
    dragging: false,
    raf: null,
  };
  window.addEventListener("pointermove", onTabDragPointerMove);
  window.addEventListener("pointerup", onTabDragPointerUp);
  window.addEventListener("pointercancel", onTabDragPointerCancel);
}

function automationRoot(): HTMLElement {
  return activeRuntimeFrame() ?? document.body;
}

function automationVisible(element: Element): boolean {
  const style = window.getComputedStyle(element);
  if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
    return false;
  }
  const rect = element.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}

function automationText(value: string, limit = 240): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
}

function automationElementName(element: Element): string {
  const input = element as HTMLInputElement;
  return automationText(
    element.getAttribute("aria-label")
      || element.getAttribute("title")
      || element.getAttribute("alt")
      || input.placeholder
      || input.value
      || (element as HTMLElement).innerText
      || element.textContent
      || "",
  );
}

function ensureAutomationElementId(element: Element): string {
  const html = element as HTMLElement;
  if (!html.dataset.locusAutomationId) {
    automationElementSeq += 1;
    html.dataset.locusAutomationId = `view-el-${automationElementSeq}`;
  }
  return html.dataset.locusAutomationId;
}

function automationSelector(element: Element): string {
  if (element.id) return `#${CSS.escape(element.id)}`;
  const parts: string[] = [];
  let current: Element | null = element;
  const root = automationRoot();
  while (current && current !== root && current !== document.body) {
    const parent: Element | null = current.parentElement;
    const tag = current.tagName.toLowerCase();
    if (!parent) {
      parts.unshift(tag);
      break;
    }
    const currentTagName = current.tagName;
    const siblings = Array.from(parent.children).filter((item) => item.tagName === currentTagName);
    const index = siblings.indexOf(current) + 1;
    parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${index})` : tag);
    current = parent;
  }
  return parts.join(" > ");
}

function automationElementSnapshot(element: Element) {
  const rect = element.getBoundingClientRect();
  const input = element as HTMLInputElement;
  return {
    id: ensureAutomationElementId(element),
    tag: element.tagName.toLowerCase(),
    role: element.getAttribute("role") || "",
    name: automationElementName(element),
    text: automationText((element as HTMLElement).innerText || element.textContent || ""),
    selector: automationSelector(element),
    rect: {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    },
    visible: automationVisible(element),
    disabled: !!(input as { disabled?: boolean }).disabled || element.getAttribute("aria-disabled") === "true",
    checked: typeof input.checked === "boolean" ? input.checked : undefined,
    value: "value" in input ? input.value : undefined,
  };
}

function collectAutomationElements(payload: Record<string, unknown>) {
  const root = typeof payload.selector === "string" && payload.selector.trim()
    ? automationRoot().querySelector(payload.selector)
    : automationRoot();
  if (!root) throw new Error(`Selector not found: ${String(payload.selector)}`);

  const maxElements = Math.max(1, Math.min(Number(payload.maxElements ?? 120), 500));
  const includeHidden = payload.includeHidden === true;
  const selector = [
    "button",
    "a[href]",
    "input",
    "textarea",
    "select",
    "summary",
    "label",
    "h1",
    "h2",
    "h3",
    "h4",
    "[role]",
    "[tabindex]",
    "[contenteditable='true']",
    "[data-locus-action]",
    "[data-node-id]",
    "[data-canvas-item-id]",
    "[data-locus-status]",
    ".status",
    ".error",
    ".warning",
    ".empty",
  ].join(", ");
  const elements = Array.from(root.querySelectorAll(selector))
    .filter((element) => includeHidden || automationVisible(element))
    .slice(0, maxElements)
    .map(automationElementSnapshot);
  return { root, elements };
}

function automationSnapshot(payload: Record<string, unknown> = {}) {
  const root = automationRoot();
  const { elements } = collectAutomationElements(payload);
  const active = document.activeElement instanceof Element
    ? automationElementSnapshot(document.activeElement)
    : null;
  const rect = root.getBoundingClientRect();
  return {
    ok: true,
    viewId: activeViewId.value,
    status: {
      loading: loading.value,
      error: error.value,
      manifest: manifest.value
        ? {
            id: manifest.value.id,
            name: manifest.value.name,
            template: manifest.value.template,
            version: manifest.value.version,
          }
        : null,
      latestFrontendLog: latestFrontendLog.value,
    },
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: Math.round(window.scrollX),
      scrollY: Math.round(window.scrollY),
    },
    frame: {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    },
    focus: active,
    elements,
  };
}

function targetFromPayload(payload: Record<string, unknown>): Element {
  const target = (payload.target && typeof payload.target === "object"
    ? payload.target
    : payload) as Record<string, unknown>;
  const root = automationRoot();

  const elementId = typeof target.elementId === "string" ? target.elementId.trim() : "";
  if (elementId) {
    const found = root.querySelector(`[data-locus-automation-id="${CSS.escape(elementId)}"]`);
    if (found) return found;
    throw new Error(`Element id not found: ${elementId}`);
  }

  const selector = typeof target.selector === "string" ? target.selector.trim() : "";
  if (selector) {
    const found = root.querySelector(selector);
    if (found) return found;
    throw new Error(`Selector not found: ${selector}`);
  }

  const x = Number(target.x);
  const y = Number(target.y);
  if (Number.isFinite(x) && Number.isFinite(y)) {
    const found = document.elementFromPoint(x, y);
    if (found) return found;
    throw new Error(`No element at point ${x},${y}`);
  }

  const text = typeof target.text === "string" ? target.text.trim().toLowerCase() : "";
  const name = typeof target.name === "string" ? target.name.trim().toLowerCase() : "";
  const role = typeof target.role === "string" ? target.role.trim().toLowerCase() : "";
  if (text || name || role) {
    const candidates = Array.from(root.querySelectorAll("*"))
      .filter((element) => automationVisible(element));
    const found = candidates.find((element) => {
      const snapshot = automationElementSnapshot(element);
      const candidateName = snapshot.name.toLowerCase();
      const candidateText = snapshot.text.toLowerCase();
      const candidateRole = snapshot.role.toLowerCase();
      return (!text || candidateText.includes(text))
        && (!name || candidateName.includes(name))
        && (!role || candidateRole === role);
    });
    if (found) return found;
  }

  throw new Error("View action target is required.");
}

function automationPointForElement(element: Element): AutomationPoint {
  const rect = element.getBoundingClientRect();
  return {
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

function automationPointFromLocator(
  locator: Record<string, unknown> | null | undefined,
  fallback: Element,
): AutomationPoint {
  const x = Number(locator?.x);
  const y = Number(locator?.y);
  if (Number.isFinite(x) && Number.isFinite(y)) {
    return { x, y };
  }
  return automationPointForElement(fallback);
}

function dispatchMouseEventAt(
  target: Element | Window,
  type: string,
  point: AutomationPoint,
  button = 0,
  buttons = 0,
) {
  target.dispatchEvent(new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: point.x,
    clientY: point.y,
    button,
    buttons,
    view: window,
  }));
}

function dispatchPointerEventAt(
  target: Element | Window,
  type: string,
  point: AutomationPoint,
  button = 0,
  buttons = 0,
) {
  if (typeof PointerEvent !== "function") return;
  target.dispatchEvent(new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: point.x,
    clientY: point.y,
    button,
    buttons,
    pointerId: 1,
    pointerType: "mouse",
    isPrimary: true,
    view: window,
  }));
}

function dispatchMouseSequence(element: Element, type: "click" | "doubleClick" | "hover") {
  const point = automationPointForElement(element);
  if (type === "hover") {
    dispatchPointerEventAt(element, "pointerover", point);
    dispatchPointerEventAt(element, "pointerenter", point);
    dispatchPointerEventAt(element, "pointermove", point);
    dispatchMouseEventAt(element, "mouseover", point);
    dispatchMouseEventAt(element, "mouseenter", point);
    dispatchMouseEventAt(element, "mousemove", point);
    return;
  }
  dispatchPointerEventAt(element, "pointerdown", point, 0, 1);
  dispatchMouseEventAt(element, "mousedown", point, 0, 1);
  dispatchPointerEventAt(element, "pointerup", point, 0, 0);
  dispatchMouseEventAt(element, "mouseup", point, 0, 0);
  (element as HTMLElement).click?.();
  if (type === "doubleClick") {
    dispatchMouseEventAt(element, "dblclick", point);
  }
}

function interpolateAutomationPoint(from: AutomationPoint, to: AutomationPoint, ratio: number): AutomationPoint {
  return {
    x: from.x + (to.x - from.x) * ratio,
    y: from.y + (to.y - from.y) * ratio,
  };
}

function dispatchDragSequence(
  source: Element,
  destination: Element,
  from: AutomationPoint,
  to: AutomationPoint,
) {
  const distance = Math.hypot(to.x - from.x, to.y - from.y);
  const steps = Math.max(2, Math.min(12, Math.ceil(distance / 80)));

  dispatchPointerEventAt(source, "pointerover", from, 0, 1);
  dispatchMouseEventAt(source, "mouseover", from, 0, 1);
  dispatchPointerEventAt(source, "pointerdown", from, 0, 1);
  dispatchMouseEventAt(source, "mousedown", from, 0, 1);

  for (let step = 1; step <= steps; step += 1) {
    const point = interpolateAutomationPoint(from, to, step / steps);
    const mouseTarget = document.elementFromPoint(point.x, point.y) || destination;
    dispatchPointerEventAt(source, "pointermove", point, 0, 1);
    dispatchMouseEventAt(mouseTarget, "mousemove", point, 0, 1);
  }

  dispatchPointerEventAt(source, "pointerup", to, 0, 0);
  dispatchMouseEventAt(destination, "mouseup", to, 0, 0);
}

function setElementValue(element: Element, value: unknown, append = false) {
  const next = String(value ?? "");
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    element.focus();
    element.value = append ? `${element.value}${next}` : next;
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return;
  }
  if (element instanceof HTMLSelectElement) {
    element.focus();
    element.value = next;
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return;
  }
  if ((element as HTMLElement).isContentEditable) {
    (element as HTMLElement).focus();
    element.textContent = append ? `${element.textContent ?? ""}${next}` : next;
    element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: next }));
    return;
  }
  throw new Error("Target does not accept text input.");
}

function performAutomationAction(payload: Record<string, unknown>) {
  const action = String(payload.action || "").trim();
  if (!action) throw new Error("Action is required.");
  const element = action === "scroll" && !payload.target ? automationRoot() : targetFromPayload(payload);
  const before = automationElementSnapshot(element);

  if (action === "click" || action === "doubleClick" || action === "hover") {
    dispatchMouseSequence(element, action);
  } else if (action === "focus") {
    (element as HTMLElement).focus();
  } else if (action === "type") {
    setElementValue(element, payload.text, true);
  } else if (action === "setValue" || action === "selectOption") {
    setElementValue(element, payload.value);
  } else if (action === "check" || action === "uncheck") {
    if (!(element instanceof HTMLInputElement) || !["checkbox", "radio"].includes(element.type)) {
      throw new Error("Target is not a checkbox or radio input.");
    }
    element.checked = action === "check";
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
  } else if (action === "press") {
    const key = String(payload.key || payload.text || "").trim();
    if (!key) throw new Error("Key is required for press.");
    (element as HTMLElement).focus();
    element.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    if (key === "Enter" && element instanceof HTMLButtonElement) element.click();
    element.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
  } else if (action === "scroll") {
    const deltaX = Number(payload.deltaX ?? 0);
    const deltaY = Number(payload.deltaY ?? 0);
    if (element === automationRoot()) {
      (element as HTMLElement).scrollBy({ left: deltaX, top: deltaY, behavior: "auto" });
    } else {
      (element as HTMLElement).scrollBy({ left: deltaX, top: deltaY, behavior: "auto" });
    }
  } else if (action === "drag") {
    const toPayload = payload.to && typeof payload.to === "object" ? payload.to as Record<string, unknown> : {};
    const fromPayload = payload.target && typeof payload.target === "object"
      ? payload.target as Record<string, unknown>
      : payload;
    const target = targetFromPayload({ target: toPayload });
    dispatchDragSequence(
      element,
      target,
      automationPointFromLocator(fromPayload, element),
      automationPointFromLocator(toPayload, target),
    );
  } else {
    throw new Error(`Unsupported View action: ${action}`);
  }

  return {
    ok: true,
    action,
    before,
    after: automationElementSnapshot(element),
  };
}

function serializeAutomationValue(value: unknown, depth = 0, seen = new WeakSet<object>()): unknown {
  if (value == null || typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return value;
  }
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "function") return `[Function ${(value as Function).name || "anonymous"}]`;
  if (value instanceof Element) return automationElementSnapshot(value);
  if (value instanceof Error) return { name: value.name, message: value.message, stack: value.stack };
  if (typeof value !== "object") return String(value);
  if (seen.has(value)) return "[Circular]";
  if (depth > 4) return "[MaxDepth]";
  seen.add(value);
  if (Array.isArray(value)) {
    return value.slice(0, 100).map((item) => serializeAutomationValue(item, depth + 1, seen));
  }
  const out: Record<string, unknown> = {};
  for (const [key, nested] of Object.entries(value as Record<string, unknown>).slice(0, 100)) {
    out[key] = serializeAutomationValue(nested, depth + 1, seen);
  }
  return out;
}

async function evaluateAutomationExpression(payload: Record<string, unknown>) {
  const source = String(payload.expression || "").trim();
  if (!source) throw new Error("Expression is required.");
  const root = automationRoot();
  const args = payload.args;
  let fn: Function;
  try {
    fn = new Function("root", "detail", "args", `"use strict"; return (${source});`);
  } catch {
    fn = new Function("root", "detail", "args", `"use strict"; ${source}`);
  }
  const value = await fn(root, detail.value, args);
  return {
    ok: true,
    value: serializeAutomationValue(value),
  };
}

async function waitForAutomationCondition(payload: Record<string, unknown>) {
  const condition = String(payload.condition || "runtimeReady").trim();
  const timeoutMs = Math.max(0, Math.min(Number(payload.timeoutMs ?? 5000), 60000));
  const pollIntervalMs = Math.max(50, Math.min(Number(payload.pollIntervalMs ?? 100), 2000));
  const startedAt = Date.now();
  let lastError = "";

  const check = async () => {
    try {
      if (condition === "runtimeReady") return !!runtimeComponent.value && !loading.value && !error.value;
      if (condition === "selectorVisible") {
        const selector = String(payload.selector || "");
        const element = selector ? automationRoot().querySelector(selector) : null;
        return !!element && automationVisible(element);
      }
      if (condition === "selectorHidden") {
        const selector = String(payload.selector || "");
        const element = selector ? automationRoot().querySelector(selector) : null;
        return !element || !automationVisible(element);
      }
      if (condition === "textPresent") {
        return automationRoot().innerText.includes(String(payload.text || ""));
      }
      if (condition === "textAbsent") {
        return !automationRoot().innerText.includes(String(payload.text || ""));
      }
      if (condition === "noConsoleError") {
        return latestFrontendLog.value?.level !== "error";
      }
      if (condition === "expression") {
        const result = await evaluateAutomationExpression(payload);
        return !!result.value;
      }
      throw new Error(`Unsupported wait condition: ${condition}`);
    } catch (conditionError) {
      lastError = conditionError instanceof Error ? conditionError.message : String(conditionError);
      return false;
    }
  };

  while (Date.now() - startedAt <= timeoutMs) {
    if (await check()) {
      return {
        ok: true,
        condition,
        elapsedMs: Date.now() - startedAt,
      };
    }
    await new Promise((resolve) => window.setTimeout(resolve, pollIntervalMs));
  }
  throw new Error(lastError || `Wait condition timed out: ${condition}`);
}

function shouldHandleAutomationRequest(requestId: string): boolean {
  const now = Date.now();
  for (const [seenRequestId, seenAt] of handledAutomationRequests) {
    if (now - seenAt > AUTOMATION_REQUEST_TTL_MS) {
      handledAutomationRequests.delete(seenRequestId);
    }
  }
  if (handledAutomationRequests.has(requestId)) return false;
  handledAutomationRequests.set(requestId, now);
  return true;
}

async function handleAutomationRequest(request: ViewAutomationRequest) {
  if (usePersistentViewContentPool.value) return;
  if (request.viewId !== activeViewId.value) {
    if (!tabs.value.some((tab) => tab.id === request.viewId)) return;
    await setActiveViewTab(request.viewId, { loadNow: true });
  }
  if (!shouldHandleAutomationRequest(request.requestId)) return;
  const payload = request.payload || {};
  try {
    const result = request.kind === "snapshot"
      ? automationSnapshot(payload)
      : request.kind === "action"
        ? performAutomationAction(payload)
        : request.kind === "wait"
          ? await waitForAutomationCondition(payload)
          : request.kind === "debugEval"
            ? await evaluateAutomationExpression(payload)
            : (() => {
                throw new Error(`Unsupported View automation kind: ${request.kind}`);
              })();
    await viewAutomationRespond(request.requestId, true, result);
  } catch (automationError) {
    const message = automationError instanceof Error
      ? automationError.message
      : String(automationError);
    await viewAutomationRespond(request.requestId, false, null, message);
  }
}

function findRuntimeStatusbar(): HTMLElement | null {
  const frame = activeRuntimeFrame();
  if (!frame) return null;
  return frame.querySelector<HTMLElement>(RUNTIME_STATUSBAR_SELECTOR);
}

function clearEmbeddedLogbarSlot() {
  embeddedLogbarSlot.value?.remove();
  embeddedLogbarSlot.value = null;
}

function ensureEmbeddedLogbarSlot(statusbar: HTMLElement | null) {
  if (!statusbar) {
    clearEmbeddedLogbarSlot();
    return;
  }

  let slot = embeddedLogbarSlot.value;
  if (!slot || slot.parentElement !== statusbar) {
    slot?.remove();
    slot = document.createElement("span");
    slot.className = "view-host-logbar-slot";
    slot.dataset.locusHostLogbarSlot = "true";
    embeddedLogbarSlot.value = slot;
  }

  const statusbarChildren = Array.from(statusbar.children).filter((child) => child !== slot);
  const rightStatusItem = statusbarChildren.length > 1
    ? statusbarChildren[statusbarChildren.length - 1]
    : null;
  if (rightStatusItem) {
    if (slot.parentElement !== statusbar || slot.nextElementSibling !== rightStatusItem) {
      statusbar.insertBefore(slot, rightStatusItem);
    }
  } else if (slot.parentElement !== statusbar) {
    statusbar.appendChild(slot);
  }
}

function syncEmbeddedLogbarSlot() {
  ensureEmbeddedLogbarSlot(findRuntimeStatusbar());
}

function scheduleEmbeddedLogbarSync() {
  if (embeddedLogbarSyncTimer) return;
  embeddedLogbarSyncTimer = setTimeout(() => {
    embeddedLogbarSyncTimer = null;
    syncEmbeddedLogbarSlot();
  }, 0);
}

function installStatusbarObserver() {
  statusbarObserver?.disconnect();
  statusbarObserver = null;
  clearEmbeddedLogbarSlot();

  const frame = activeRuntimeFrame();
  if (!frame) return;

  statusbarObserver = new MutationObserver(scheduleEmbeddedLogbarSync);
  statusbarObserver.observe(frame, { childList: true, subtree: true });
  syncEmbeddedLogbarSlot();
}

function nonEmptyString(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed ? trimmed : null;
}

function defaultViewSessionTitle(requestTitle?: string | null): string {
  return nonEmptyString(requestTitle)
    ?? nonEmptyString(windowTitle.value)
    ?? "View Session";
}

async function resolveViewModel(model?: string | null): Promise<string | null> {
  const explicit = nonEmptyString(model);
  if (explicit) return explicit;

  const [defaultsResult, lastModelResult] = await Promise.allSettled([
    getModelDefaults(),
    getLastModel(),
  ]);
  const defaultModel = defaultsResult.status === "fulfilled"
    ? nonEmptyString(defaultsResult.value.mainModel)
    : null;
  const lastModel = lastModelResult.status === "fulfilled"
    ? nonEmptyString(lastModelResult.value)
    : null;
  return defaultModel ?? lastModel;
}

async function resolveViewEffort(effort?: string | null): Promise<string | null> {
  const explicit = nonEmptyString(effort);
  if (explicit) return explicit;
  try {
    return nonEmptyString(await getLastEffort());
  } catch {
    return null;
  }
}

function waitRequestFromChat(
  launch: { sessionId: string; runId: string },
  wait: ViewSessionChatRequest["wait"],
): ViewSessionWaitRequest | null {
  if (wait === false || wait == null) return null;
  if (wait === true) {
    return { sessionId: launch.sessionId, runId: launch.runId };
  }
  return {
    ...wait,
    sessionId: wait.sessionId || launch.sessionId,
    runId: wait.runId || launch.runId,
  };
}

function terminalStatusFromStreamEvent(
  event: StreamEvent,
  sessionId: string,
  runId?: string | null,
): { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null {
  if (event.sessionId !== sessionId) return null;
  if (runId && event.runId !== runId) return null;
  if (event.type === "done") return { status: "done", error: null };
  if (event.type === "cancelled") return { status: "cancelled", error: null };
  if (event.type === "error") return { status: "error", error: event.error };
  return null;
}

function terminalStatusFromRecord(
  record: SessionEventRecord,
  sessionId: string,
  runId?: string | null,
): { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null {
  if (record.sessionId !== sessionId) return null;
  if (runId && record.runId !== runId) return null;
  const payload = record.payload as { type?: unknown; error?: unknown };
  if (payload.type === "done") return { status: "done", error: null };
  if (payload.type === "cancelled") return { status: "cancelled", error: null };
  if (payload.type === "error") {
    return { status: "error", error: normalizeAppError(payload.error) };
  }
  return null;
}

function assistantTextFromMessage(message: ChatMessage | null): string {
  if (!message) return "";
  if (message.content) return message.content;
  return (message.renderParts ?? [])
    .filter((part) => part.kind === "text")
    .map((part) => part.content)
    .join("");
}

function latestAssistantMessage(detail: SessionDetail): ChatMessage | null {
  for (let index = detail.messages.length - 1; index >= 0; index -= 1) {
    const message = detail.messages[index];
    if (message.role === "assistant") return message;
  }
  return null;
}

function finalTextFromEvents(events: SessionEventRecord[], runId?: string | null): string {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const record = events[index];
    if (runId && record.runId !== runId) continue;
    const payload = record.payload as { type?: unknown; fullText?: unknown };
    if (payload.type === "done" && typeof payload.fullText === "string") {
      return payload.fullText;
    }
  }
  return "";
}

async function createRuntimeSession(request: ViewSessionCreateRequest = {}): Promise<string> {
  return createLocusSession({
    title: defaultViewSessionTitle(request.title),
    parentSessionId: request.parentSessionId ?? null,
    sessionType: request.sessionType ?? "view",
    agentId: request.agentId ?? null,
  });
}

async function showRuntimeSession(sessionId: string): Promise<void> {
  const normalized = nonEmptyString(sessionId);
  if (!normalized) throw new Error("Session id is required.");
  await saveActiveSessionSelection(normalized);
}

async function finalizeRuntimeSessionWait(
  sessionId: string,
  runId: string | null,
  status: ViewSessionWaitStatus,
  events: SessionEventRecord[],
  includeEvents: boolean,
  activeRun: SessionRunSummary | null,
  error: AppErrorPayload | null,
): Promise<ViewSessionWaitResult> {
  const detail = await loadLocusSession(sessionId);
  const message = latestAssistantMessage(detail);
  const finalText = finalTextFromEvents(events, runId) || assistantTextFromMessage(message);
  return {
    sessionId,
    runId,
    status,
    detail,
    activeRun,
    events: includeEvents ? events : [],
    message,
    finalText,
    error,
  };
}

async function waitRuntimeSession(request: ViewSessionWaitRequest): Promise<ViewSessionWaitResult> {
  const sessionId = nonEmptyString(request.sessionId);
  if (!sessionId) throw new Error("Session id is required.");

  const timeoutMs = Math.max(0, request.timeoutMs ?? 120_000);
  const pollIntervalMs = Math.max(100, request.pollIntervalMs ?? 500);
  const includeEvents = request.includeEvents !== false;
  const returnOnWaitingInput = request.returnOnWaitingInput !== false;
  const events: SessionEventRecord[] = [];
  let afterSeq = Math.max(0, request.afterSeq ?? 0);
  let targetRunId = nonEmptyString(request.runId);
  let terminal: { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null = null;
  let activeRun: SessionRunSummary | null = null;

  const appendNewEvents = async () => {
    const batch = await listSessionEvents(sessionId, afterSeq, 2_000);
    if (batch.length === 0) return;
    events.push(...batch);
    afterSeq = Math.max(afterSeq, ...batch.map((event) => event.seq));
    if (!targetRunId) {
      targetRunId = nonEmptyString(batch[batch.length - 1]?.runId);
    }
    for (const record of batch) {
      terminal = terminalStatusFromRecord(record, sessionId, targetRunId) ?? terminal;
    }
  };

  const unsubscribe = await getLocusRuntime().subscribe<StreamEvent>("stream-event", (event) => {
    terminal = terminalStatusFromStreamEvent(event, sessionId, targetRunId) ?? terminal;
  });

  const startedAt = Date.now();
  try {
    if (!targetRunId) {
      activeRun = await getSessionActiveRun(sessionId);
      targetRunId = nonEmptyString(activeRun?.runId);
    }
    await appendNewEvents();
    while (!terminal && Date.now() - startedAt <= timeoutMs) {
      activeRun = await getSessionActiveRun(sessionId);
      if (!targetRunId) targetRunId = nonEmptyString(activeRun?.runId);
      if (activeRun?.status === "waiting_input" && returnOnWaitingInput) {
        return finalizeRuntimeSessionWait(
          sessionId,
          targetRunId,
          "waiting_input",
          events,
          includeEvents,
          activeRun,
          null,
        );
      }
      await new Promise((resolve) => window.setTimeout(resolve, pollIntervalMs));
      await appendNewEvents();
    }
  } finally {
    unsubscribe();
  }

  const terminalResult = terminal as { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null;
  if (terminalResult) {
    return finalizeRuntimeSessionWait(
      sessionId,
      targetRunId,
      terminalResult.status,
      events,
      includeEvents,
      activeRun,
      terminalResult.error ?? null,
    );
  }

  return finalizeRuntimeSessionWait(
    sessionId,
    targetRunId,
    "timeout",
    events,
    includeEvents,
    activeRun,
    null,
  );
}

async function sendRuntimeSessionMessage(
  request: ViewSessionChatRequest,
): Promise<ViewSessionChatResult> {
  const text = request.text ?? "";
  if (!text.trim()) throw new Error("Session message text is required.");
  const model = await resolveViewModel(request.model);
  if (!model) throw new Error("No model configured for View LLM calls.");
  const effort = await resolveViewEffort(request.effort);
  const launch = await launchSessionChat({
    sessionId: request.sessionId ?? null,
    text,
    sessionTitle: request.sessionTitle ?? request.title ?? defaultViewSessionTitle(null),
    agentId: request.agentId ?? null,
    model,
    effort,
    images: request.images ?? null,
    assetRefs: request.assetRefs ?? null,
    sessionType: request.sessionType ?? "view",
    mode: request.mode ?? null,
    userIntent: request.userIntent ?? null,
    subagentModels: request.subagentModels ?? null,
    knowledgeMode: request.knowledgeMode ?? null,
  });

  if (request.show) {
    await showRuntimeSession(launch.sessionId);
  }

  const waitRequest = waitRequestFromChat(launch, request.wait);
  const result = waitRequest ? await waitRuntimeSession(waitRequest) : null;
  return { ...launch, result };
}

async function callRuntimeLlm(request: ViewLlmCallRequest): Promise<ViewLlmCallResult> {
  const launch = await sendRuntimeSessionMessage({
    ...request,
    text: request.prompt,
    wait: request.wait ?? {
      sessionId: request.sessionId ?? "",
      timeoutMs: request.timeoutMs ?? undefined,
    },
  });

  if (!launch.result) {
    return {
      sessionId: launch.sessionId,
      runId: launch.runId,
      status: "running",
      text: "",
      detail: null,
      events: [],
      message: null,
      error: null,
    };
  }

  return {
    sessionId: launch.sessionId,
    runId: launch.runId,
    status: launch.result.status,
    text: launch.result.finalText,
    detail: launch.result.detail,
    events: launch.result.events,
    message: launch.result.message ?? null,
    error: launch.result.error ?? null,
  };
}

function viewContentMountGeometryFromRequest(
  request: ViewContentMountRequest,
): ViewContentMountGeometry {
  return {
    viewId: request.viewId,
    hostLabel: request.hostLabel,
    x: request.x,
    y: request.y,
    width: request.width,
    height: request.height,
    visible: request.visible !== false,
  };
}

function viewContentMountGeometryMatches(
  left: ViewContentMountGeometry | null,
  right: ViewContentMountRequest,
): boolean {
  if (!left) return false;
  const next = viewContentMountGeometryFromRequest(right);
  return left.viewId === next.viewId
    && left.hostLabel === next.hostLabel
    && left.x === next.x
    && left.y === next.y
    && left.width === next.width
    && left.height === next.height
    && left.visible === next.visible;
}

async function buildViewContentMountRequest(
  viewId: string,
  visible: boolean,
): Promise<ViewContentMountRequest | null> {
  const geometryStartedAt = perfNowMs();
  if (!appWindow || !viewHostBodyRef.value || !currentWindowLabel) {
    viewHostContentLog("mount-geometry-missing", {
      viewId,
      hasWindow: !!appWindow,
      hasBody: !!viewHostBodyRef.value,
      hasLabel: !!currentWindowLabel,
    });
    return null;
  }
  const rect = viewHostBodyRef.value.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) {
    viewHostContentLog("mount-geometry-empty", {
      viewId,
      width: rect.width,
      height: rect.height,
    });
    return null;
  }
  const [position, scaleFactor] = await Promise.all([
    appWindow.outerPosition(),
    appWindow.scaleFactor().catch(() => window.devicePixelRatio || 1),
  ]);
  const scale = Number.isFinite(scaleFactor) && scaleFactor > 0 ? scaleFactor : 1;
  viewHostContentLog("mount-geometry-ready", {
    viewId,
    elapsedMs: elapsedMs(geometryStartedAt),
    left: rect.left,
    top: rect.top,
    width: rect.width,
    height: rect.height,
    scale,
  });
  return {
    viewId,
    hostLabel: currentWindowLabel,
    x: Math.round(position.x + rect.left * scale),
    y: Math.round(position.y + rect.top * scale),
    width: Math.max(1, Math.round(rect.width * scale)),
    height: Math.max(1, Math.round(rect.height * scale)),
    visible,
  };
}

async function mountViewContentFromPool(
  targetViewId = activeViewId.value,
  options: { force?: boolean; updateGeometryOnly?: boolean } = {},
): Promise<void> {
  const viewId = String(targetViewId || "").trim();
  if (!viewId) {
    const record = ensureRuntimeRecord("");
    record.error = t("view.host.missingId");
    return;
  }

  const record = ensureRuntimeRecord(viewId);
  if (!options.updateGeometryOnly) {
    viewHostContentLog("mount-request", {
      viewId,
      force: !!options.force,
      updateGeometryOnly: !!options.updateGeometryOnly,
      recordHasDetail: !!record.detail,
      recordStale: record.stale,
      recordLoading: !!record.loadPromise,
    });
  }
  if (record.loadPromise) {
    if (options.updateGeometryOnly) record.geometrySyncQueued = true;
    else record.reloadQueued = true;
    if (!options.updateGeometryOnly) {
      viewHostContentLog("mount-deferred", {
        viewId,
        reloadQueued: record.reloadQueued,
        geometrySyncQueued: record.geometrySyncQueued,
      });
    }
    return record.loadPromise;
  }
  if (options.force) record.stale = true;

  record.reloadQueued = false;
  record.loadPromise = (async () => {
    const loadStartedAt = perfNowMs();
    const visible = viewId === activeViewId.value;
    if (!options.updateGeometryOnly) {
      record.loading = !record.detail;
      record.error = "";
      viewHostContentLog("mount-flow-start", {
        viewId,
        hasDetail: !!record.detail,
        stale: record.stale,
        visible,
      });
      markStartupPhase("load-start", { viewId, mode: "content-pool" });
    }

    try {
      if (!options.updateGeometryOnly && (record.stale || !record.detail)) {
        const viewReadStartedAt = perfNowMs();
        markStartupPhase("viewRead_start", { viewId, mode: "content-pool" });
        const next = await viewRead(viewId);
        markStartupPhase("viewRead_done", {
          viewId,
          mode: "content-pool",
          elapsedMs: elapsedMs(viewReadStartedAt),
          fileCount: next.files.length,
          packageRoot: next.summary.packageRoot,
        });
        record.detail = next;
        record.stale = false;
        upsertTab(tabFromDetail(next));
        void registerCurrentTabHost();
      }

      if (!visible) {
        if (!options.updateGeometryOnly) {
          viewHostContentLog("hide-inactive-content", {
            viewId,
            activeViewId: activeViewId.value,
          });
        }
        await viewContentHide(viewId);
        return;
      }

      const mountRequest = await buildViewContentMountRequest(viewId, true);
      if (!mountRequest) return;
      if (options.updateGeometryOnly
        && viewContentMountGeometryMatches(lastViewContentMountGeometry, mountRequest)
      ) {
        viewHostContentLog("mount-geometry-unchanged", {
          viewId,
          hostLabel: mountRequest.hostLabel,
          width: mountRequest.width,
          height: mountRequest.height,
        });
        return;
      }
      const mountStartedAt = perfNowMs();
      if (!options.updateGeometryOnly) {
        viewHostContentLog("mount-ipc-start", {
          viewId,
          hostLabel: mountRequest.hostLabel,
          x: mountRequest.x,
          y: mountRequest.y,
          width: mountRequest.width,
          height: mountRequest.height,
        });
        markStartupPhase("viewContentMount_start", { viewId, hostLabel: mountRequest.hostLabel });
      }
      const mountResult = await viewContentMount(mountRequest);
      lastViewContentMountGeometry = viewContentMountGeometryFromRequest(mountRequest);
      if (!options.updateGeometryOnly) {
        viewHostContentLog("mount-ipc-done", {
          viewId,
          contentLabel: mountResult.windowLabel,
          packageRoot: mountResult.packageRoot,
          elapsedMs: elapsedMs(mountStartedAt),
          sinceMountFlowStartMs: elapsedMs(loadStartedAt),
        });
      }
      const inactiveTabs = tabs.value
        .map((tab) => tab.id)
        .filter((tabId) => tabId !== viewId);
      if (inactiveTabs.length > 0 && !options.updateGeometryOnly) {
        viewHostContentLog("hide-inactive-tabs-after-mount", {
          viewId,
          inactiveTabs: inactiveTabs.join(","),
        });
      }
      void hidePersistentViewContentTabs(inactiveTabs);
      if (!options.updateGeometryOnly) {
        markStartupPhase("viewContentMount_done", {
          viewId,
          hostLabel: mountRequest.hostLabel,
          elapsedMs: elapsedMs(mountStartedAt),
        });
      }
    } catch (loadError) {
      const message = normalizeAppError(loadError).message;
      if (!options.updateGeometryOnly) {
        record.error = message;
        markStartupPhase("load-error", {
          viewId,
          mode: "content-pool",
          elapsedMs: elapsedMs(loadStartedAt),
          message,
        });
        console.error("[view-host] Failed to mount persistent View content", loadError);
      }
    } finally {
      record.loading = false;
      if (!options.updateGeometryOnly) {
        markStartupPhase("load-finish", {
          viewId,
          mode: "content-pool",
          elapsedMs: elapsedMs(loadStartedAt),
          error: record.error || undefined,
        });
      }
      record.loadPromise = null;
      const shouldSyncGeometry = record.geometrySyncQueued;
      record.geometrySyncQueued = false;
      if (record.reloadQueued) scheduleLoadView(0, record.viewId, true);
      else if (shouldSyncGeometry && record.viewId === activeViewId.value) scheduleViewContentSync(0);
    }
  })();

  return record.loadPromise;
}

function scheduleViewContentSync(delay = VIEW_CONTENT_SYNC_FRAME_MS) {
  if (!usePersistentViewContentPool.value) return;
  if (viewContentSyncTimer) clearTimeout(viewContentSyncTimer);
  viewContentSyncTimer = setTimeout(() => {
    viewContentSyncTimer = null;
    const viewId = activeViewId.value;
    if (!viewId) return;
    void mountViewContentFromPool(viewId, { updateGeometryOnly: true });
  }, delay);
}

async function hidePersistentViewContentTabs(viewIds: string[]) {
  const normalized = normalizeTabIds(viewIds);
  if (normalized.length > 0) {
    viewHostContentLog("hide-batch", { viewIds: normalized.join(",") });
  }
  await Promise.allSettled(
    normalized.map((viewId) => viewContentHide(viewId)),
  );
}

async function loadView(
  targetViewId = activeViewId.value,
  options: { force?: boolean } = {},
): Promise<void> {
  if (usePersistentViewContentPool.value) {
    return mountViewContentFromPool(targetViewId, options);
  }

  const viewId = String(targetViewId || "").trim();
  if (!viewId) {
    const record = ensureRuntimeRecord("");
    record.error = t("view.host.missingId");
    return;
  }

  const record = ensureRuntimeRecord(viewId);
  if (record.loadPromise) {
    record.reloadQueued = true;
    return record.loadPromise;
  }
  if (options.force) record.stale = true;
  if (!record.stale && record.component && !record.error) return;

  record.reloadQueued = false;
  record.loadPromise = (async () => {
    const loadStartedAt = perfNowMs();
    record.loading = true;
    record.error = "";
    markStartupPhase("load-start", { viewId });
    try {
      const viewReadStartedAt = perfNowMs();
      markStartupPhase("viewRead_start", { viewId });
      const next = await viewRead(viewId);
      markStartupPhase("viewRead_done", {
        viewId,
        elapsedMs: elapsedMs(viewReadStartedAt),
        fileCount: next.files.length,
        packageRoot: next.summary.packageRoot,
      });
      record.detail = next;
      record.stale = false;
      upsertTab(tabFromDetail(next));
      void registerCurrentTabHost();
      if (viewRequiresUnityConnection(next.manifest)) {
        const unityStatusStartedAt = perfNowMs();
        markStartupPhase("unityStatus_start", { viewId });
        const status = await checkUnityConnectionStatus();
        markStartupPhase("unityStatus_done", {
          viewId,
          elapsedMs: elapsedMs(unityStatusStartedAt),
          connected: status.connected,
        });
        if (!status.connected) {
          record.error = t("view.host.unityConnectionRequired");
          record.component = null;
          return;
        }
      }
      const runtimeComponentStartedAt = perfNowMs();
      markStartupPhase("runtimeComponent_create_start", { viewId });
      record.component = markRaw(
        createViewRuntimeComponent({
          detail: next,
          api: {
            callScript: (scriptName, method, args) =>
              viewCallScript({ viewId: next.manifest.id, scriptName, method, args }),
            unityPropertyRead: readUnitySerializedProperty,
            unityPropertyDiscover: discoverUnitySerializedProperties,
            unityPropertyWrite: writeUnitySerializedProperty,
            unityPropertyApply: applyUnitySerializedProperties,
            searchAssets: (query, roots, limit) =>
              searchWorkspaceAssets(query, roots?.length ? roots : ["Assets", "Packages"], limit),
            createSession: (request) => createRuntimeSession(request),
            showSession: (sessionId) => showRuntimeSession(sessionId),
            loadSession: (sessionId) => loadLocusSession(sessionId),
            getSessionActiveRun: (sessionId) => getSessionActiveRun(sessionId),
            listSessionEvents: (sessionId, afterSeq, limit) =>
              listSessionEvents(sessionId, afterSeq, limit),
            queueSessionInput: (request: ViewSessionQueueInputRequest) => queueChatInput(request),
            sendSessionMessage: (request) => sendRuntimeSessionMessage(request),
            waitSession: (request) => waitRuntimeSession(request),
            callLlm: (request) => callRuntimeLlm(request),
            onSessionEvent: (handler) =>
              getLocusRuntime().subscribe<StreamEvent>("stream-event", handler),
            readFrontendLog: (limit) => viewReadFrontendLog({ viewId: next.manifest.id, limit }),
            openFrontendLog: () => viewOpenFrontendLog(next.manifest.id),
            storageGet: (key) => viewStorageGet({ viewId: next.manifest.id, key }),
            storageSet: (key, value) => viewStorageSet({ viewId: next.manifest.id, key, value }),
            storageRemove: (key) => viewStorageRemove({ viewId: next.manifest.id, key }),
            fsReadFile: (path, encoding) => viewFsReadFile({ path, encoding }),
            fsWriteFile: (path, data, encoding) => viewFsWriteFile({ path, data, encoding }),
            fsAppendFile: (path, data, encoding) => viewFsAppendFile({ path, data, encoding }),
            fsMkdir: (path, options) => viewFsMkdir({ path, recursive: options?.recursive }),
            fsReaddir: (path, options) => viewFsReaddir({ path, withFileTypes: options?.withFileTypes }),
            fsStat: (path) => viewFsStat({ path }),
            fsLstat: (path) => viewFsLstat({ path }),
            fsAccess: (path) => viewFsAccess({ path }),
            fsUnlink: (path) => viewFsUnlink({ path }),
            fsRm: (path, options) =>
              viewFsRm({ path, recursive: options?.recursive, force: options?.force }),
            fsRename: (oldPath, newPath) => viewFsRename({ oldPath, newPath }),
            fsCopyFile: (src, dest) => viewFsCopyFile({ src, dest }),
            onUpdate: (handler) =>
              getLocusRuntime().subscribe<ViewRuntimeUpdateEvent>("unity-editor-update", handler),
            reload: () => loadView(record.viewId, { force: true }),
          },
        }),
      );
      markStartupPhase("runtimeComponent_create_done", {
        viewId,
        elapsedMs: elapsedMs(runtimeComponentStartedAt),
      });
    } catch (loadError) {
      record.error = normalizeAppError(loadError).message;
      markStartupPhase("load-error", {
        viewId,
        elapsedMs: elapsedMs(loadStartedAt),
        message: record.error,
      });
      console.error("[view-host]", loadError);
      record.component = null;
    } finally {
      record.loading = false;
      markStartupPhase("load-finish", {
        viewId,
        elapsedMs: elapsedMs(loadStartedAt),
        error: record.error || undefined,
      });
      record.loadPromise = null;
      if (record.reloadQueued) scheduleLoadView(0, record.viewId, true);
    }
  })();

  return record.loadPromise;
}

function scheduleLoadView(delay = 120, targetViewId = activeViewId.value, force = false) {
  const viewId = String(targetViewId || "").trim();
  if (viewId && force) ensureRuntimeRecord(viewId).stale = true;
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void loadView(viewId || activeViewId.value, { force });
  }, delay);
}

watch(runtimeComponent, () => {
  void nextTick().then(installStatusbarObserver);
});

function installViewContentPoolObservers() {
  if (!usePersistentViewContentPool.value) return;
  viewContentResizeObserver?.disconnect();
  viewContentResizeObserver = null;
  if (viewHostBodyRef.value && typeof ResizeObserver !== "undefined") {
    viewContentResizeObserver = new ResizeObserver(() => scheduleViewContentSync());
    viewContentResizeObserver.observe(viewHostBodyRef.value);
  }
  window.addEventListener("resize", onViewContentViewportChanged);
  if (appWindow) {
    void appWindow.onResized(() => {
      scheduleViewContentSync();
    }).then((unlisten) => {
      unlistenWindowResize = unlisten;
    }).catch(() => undefined);
    const movableWindow = appWindow as TauriWindowHandle & {
      onMoved?: (handler: () => void) => Promise<UnlistenFn>;
    };
    const movedListener = movableWindow.onMoved?.(() => {
      scheduleViewContentSync();
    });
    if (movedListener) {
      void movedListener.then((unlisten) => {
        unlistenWindowMove = unlisten;
      }).catch(() => undefined);
    }
  }
}

function onViewContentViewportChanged() {
  scheduleViewContentSync();
}

onMounted(async () => {
  hostMountedAt = perfNowMs();
  viewHostContentLog("host-mounted", { initialViewId });
  void syncMaximizedState();
  void syncAlwaysOnTopState();
  refreshConsoleLogCapture();
  void refreshLatestFrontendLog();
  installViewContentPoolObservers();
  prepareViewHostPool("host-mounted");
  const listenerSetup = (async () => {
    const listenerStartedAt = perfNowMs();
    viewHostContentLog("listener-setup-start");
    if (appWindow) {
      unlistenTabMerge = await appWindow.listen<ViewHostTabsMergePayload>(
        VIEW_HOST_TABS_MERGE_EVENT,
        (event) => {
          void applyMergedViewTabs(event.payload);
        },
      );
      unlistenTabSelect = await appWindow.listen<ViewHostTabsSelectPayload>(
        VIEW_HOST_TABS_SELECT_EVENT,
        (event) => {
          void selectHostedViewTab(event.payload);
        },
      );
      unlistenTabDropTarget = await appWindow.listen<ViewHostTabsDropTargetPayload>(
        VIEW_HOST_TABS_DROP_TARGET_EVENT,
        (event) => {
          applyExternalTabDropTarget(event.payload);
        },
      );
    }
    unsubscribeAutomation = await getLocusRuntime().subscribe<ViewAutomationRequest>(
      "view-automation-request",
      (payload) => {
        void handleAutomationRequest(payload);
      },
    );
    unsubscribeReload = await getLocusRuntime().subscribe<ViewPackageSummary>(
      "view-package-reloaded",
      (payload) => {
        if (!tabs.value.some((tab) => tab.id === payload.id)) return;
        upsertTab(tabFromSummary(payload));
        const record = findRuntimeRecord(payload.id);
        if (record) record.stale = true;
        void registerCurrentTabHost();
        if (payload.id === activeViewId.value) scheduleLoadView(120, payload.id, true);
      },
    );
    viewHostContentLog("listener-setup-done", { elapsedMs: elapsedMs(listenerStartedAt) });
    if (isViewHostPoolWindow && currentWindowLabel) {
      const readyStartedAt = perfNowMs();
      viewHostContentLog("pool-ready-start");
      try {
        await viewHostPoolReady(currentWindowLabel);
        viewHostContentLog("pool-ready-done", { elapsedMs: elapsedMs(readyStartedAt) });
      } catch (readyError) {
        viewHostContentLog("pool-ready-error", {
          elapsedMs: elapsedMs(readyStartedAt),
          message: normalizeAppError(readyError).message,
        });
      }
    }
  })();
  const initialLoad = (async () => {
    const initialLoadStartedAt = perfNowMs();
    if (isViewHostPoolWindow && !activeViewId.value) {
      viewHostContentLog("initial-load-skip-pool");
      return;
    }
    viewHostContentLog("initial-load-start", { viewId: activeViewId.value });
    try {
      await loadView();
      viewHostContentLog("initial-load-done", {
        viewId: activeViewId.value,
        elapsedMs: elapsedMs(initialLoadStartedAt),
      });
    } catch (loadError) {
      viewHostContentLog("initial-load-error", {
        viewId: activeViewId.value,
        elapsedMs: elapsedMs(initialLoadStartedAt),
        message: normalizeAppError(loadError).message,
      });
    } finally {
      await revealViewHostWindow("initial-load-settled");
    }
  })();
  await Promise.all([listenerSetup, initialLoad]);
  await nextTick();
  installStatusbarObserver();
  viewHostContentLog("host-ready", { viewId: activeViewId.value });
  prepareViewHostPool("host-ready");
});

onUnmounted(() => {
  const state = tabDragState.value;
  if (state && state.raf !== null) clearTimeout(state.raf);
  stopTabDragPreview();
  setTabDropTargetLabel("");
  tabDragState.value = null;
  externalTabDropActive.value = false;
  externalTabDropSourceLabel = "";
  window.removeEventListener("pointermove", onTabDragPointerMove);
  window.removeEventListener("pointerup", onTabDragPointerUp);
  window.removeEventListener("pointercancel", onTabDragPointerCancel);
  if (suppressNextTabClickTimer) clearTimeout(suppressNextTabClickTimer);
  suppressNextTabClickTimer = null;
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = null;
  if (embeddedLogbarSyncTimer) clearTimeout(embeddedLogbarSyncTimer);
  embeddedLogbarSyncTimer = null;
  if (viewContentSyncTimer) clearTimeout(viewContentSyncTimer);
  viewContentSyncTimer = null;
  viewContentResizeObserver?.disconnect();
  viewContentResizeObserver = null;
  window.removeEventListener("resize", onViewContentViewportChanged);
  unlistenWindowResize?.();
  unlistenWindowResize = null;
  unlistenWindowMove?.();
  unlistenWindowMove = null;
  if (usePersistentViewContentPool.value) {
    viewHostContentLog("unmount-hide-tabs", {
      viewIds: tabs.value.map((tab) => tab.id).join(","),
    });
    void hidePersistentViewContentTabs(tabs.value.map((tab) => tab.id));
  }
  statusbarObserver?.disconnect();
  statusbarObserver = null;
  clearEmbeddedLogbarSlot();
  runtimeFrameRefs.clear();
  unsubscribeReload?.();
  unsubscribeReload = null;
  unsubscribeAutomation?.();
  unsubscribeAutomation = null;
  unlistenTabMerge?.();
  unlistenTabMerge = null;
  unlistenTabSelect?.();
  unlistenTabSelect = null;
  unlistenTabDropTarget?.();
  unlistenTabDropTarget = null;
  handledAutomationRequests.clear();
  restoreConsoleLogCapture?.();
  restoreConsoleLogCapture = null;
});
</script>

<template>
  <main
    class="view-host-window"
    :class="{
      'is-embedded': props.embedded,
      'is-tab-dragging': tabDragState?.dragging,
      'is-tab-drop-target': !!tabDropTargetLabel,
      'is-external-tab-drop-target': externalTabDropActive,
    }"
  >
    <header
      v-if="!props.embedded"
      class="view-host-titlebar"
      data-tauri-drag-region
      @dblclick="toggleMaximizeWindow"
    >
      <div
        class="view-host-tabs"
        data-tauri-drag-region
        role="tablist"
        @dblclick.stop="toggleMaximizeWindow"
      >
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          class="view-host-tab"
          :class="{
            active: tab.id === activeViewId,
            dragging: tabDragState?.dragging && tabDragState.tabId === tab.id,
          }"
          :title="tab.title"
          role="tab"
          :aria-selected="tab.id === activeViewId"
          @pointerdown="startTabDrag($event, tab.id)"
          @click="onTabClick($event, tab.id)"
        >
          <span class="view-host-tab-title">{{ tab.title }}</span>
        </button>
        <span v-if="tabs.length === 0" class="view-host-title-main">{{ windowTitle }}</span>
      </div>
      <div class="view-host-window-controls" data-window-no-drag @dblclick.stop>
        <button
          type="button"
          class="view-host-win-ctrl-btn"
          :class="{ 'view-host-win-pinned': alwaysOnTop }"
          :title="alwaysOnTop ? t('app.pin.unpin') : t('app.pin.pin')"
          @click="toggleAlwaysOnTop"
        >
          <svg
            viewBox="0 0 16 16"
            width="12"
            height="12"
            fill="currentColor"
            :style="{ transform: alwaysOnTop ? 'rotate(0deg)' : 'rotate(45deg)' }"
            aria-hidden="true"
          >
            <path d="M9.828 1.282a.75.75 0 0 1 .955.073l3.862 3.862a.75.75 0 0 1-.564 1.272h-.862L11.2 8.507a2.25 2.25 0 0 1-.039 2.994l-.56.56a.75.75 0 0 1-1.06 0L7.05 9.57l-3.72 3.72a.75.75 0 1 1-1.06-1.06l3.72-3.72L3.5 6.02a.75.75 0 0 1 0-1.06l.56-.56a2.25 2.25 0 0 1 2.994-.04L9.07 2.342V1.48a.75.75 0 0 1 .758-.198z"/>
          </svg>
        </button>
        <button
          type="button"
          class="view-host-win-ctrl-btn"
          :title="t('app.win.minimize')"
          @click="minimizeWindow"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1" y="5.5" width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          type="button"
          class="view-host-win-ctrl-btn"
          :title="t('app.win.maximize')"
          @click="toggleMaximizeWindow"
        >
          <svg v-if="!isMaximized" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
          <svg v-else viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1" />
            <rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1" />
          </svg>
        </button>
        <button
          type="button"
          class="view-host-win-ctrl-btn view-host-win-close"
          :title="t('app.win.close')"
          @click="closeWindow"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </header>

    <section ref="viewHostBodyRef" class="view-host-body">
      <div
        class="view-runtime-cache"
        :class="{ 'is-suspended': !!error || (loading && !detail) }"
      >
        <div
          v-for="record in visibleRuntimeRecords"
          v-show="record.viewId === activeViewId && record.component"
          :key="record.viewId"
          :ref="(element) => setRuntimeFrameRef(record.viewId, element)"
          class="view-runtime-frame"
          :aria-label="runtimeLabelForRecord(record)"
        >
          <component v-if="record.component" :is="record.component" />
        </div>
      </div>
      <div v-if="error" class="view-host-state view-host-state-error">{{ error }}</div>
      <div v-else-if="loading && !detail" class="view-host-state">{{ t("common.loading") }}</div>
    </section>
    <Teleport v-if="!usePersistentViewContentPool && embeddedLogbarSlot" :to="embeddedLogbarSlot">
      <button
        type="button"
        class="view-host-logbar-inline"
        :class="`level-${latestFrontendLogLevel}`"
        title="Double-click to open frontend.log"
        @dblclick.stop="openFrontendLog"
      >
        <span class="view-host-logbar-inline-label">Log</span>
        <span class="view-host-logbar-inline-message">{{ latestFrontendLogText }}</span>
      </button>
    </Teleport>
    <footer
      v-else-if="!usePersistentViewContentPool"
      class="view-host-logbar"
      :class="`level-${latestFrontendLogLevel}`"
      title="Double-click to open frontend.log"
      @dblclick="openFrontendLog"
    >
      <span class="view-host-logbar-label">Log</span>
      <span class="view-host-logbar-message">{{ latestFrontendLogText }}</span>
    </footer>
  </main>
</template>

<style scoped>
.view-host-window {
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
  box-sizing: border-box;
}

.view-host-window.is-embedded {
  border: 0;
}

.view-host-titlebar {
  -webkit-app-region: drag;
  height: 32px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 0 0 8px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.view-host-tabs {
  -webkit-app-region: drag;
  align-self: stretch;
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: flex-end;
  gap: 2px;
  overflow: hidden;
}

.view-host-tab {
  -webkit-app-region: no-drag;
  max-width: 220px;
  min-width: 96px;
  height: 29px;
  min-height: 29px;
  display: inline-flex;
  align-items: center;
  padding: 0 12px;
  border: 1px solid var(--border-color);
  border-bottom-color: transparent;
  border-radius: 5px 5px 0 0;
  background: color-mix(in srgb, var(--panel-bg) 64%, var(--sidebar-bg) 36%);
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  font-weight: 550;
  text-align: left;
  cursor: grab;
  user-select: none;
  transition: background 0.1s ease, border-color 0.1s ease, color 0.1s ease;
}

.view-host-tab:hover,
.view-host-tab:focus-visible {
  border-color: var(--border-strong);
  border-bottom-color: transparent;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--sidebar-bg) 22%);
  color: var(--text-color);
  outline: none;
}

.view-host-tab.active {
  height: 31px;
  min-height: 31px;
  border-color: var(--border-strong);
  border-bottom-color: var(--bg-color);
  background: var(--bg-color);
  color: var(--text-color);
  box-shadow: inset 0 2px 0 var(--accent-color);
  font-weight: 650;
}

.view-host-tab-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.view-host-title-main {
  min-width: 0;
  overflow: hidden;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.view-host-window.is-tab-dragging {
  cursor: grabbing;
}

.view-host-window.is-tab-dragging .view-host-tab {
  cursor: grabbing;
}

.view-host-tab.dragging {
  opacity: 0.72;
  border-color: var(--accent-color);
  color: var(--text-color);
  transform: translateY(1px);
}

.view-host-window.is-tab-drop-target .view-host-titlebar {
  box-shadow: inset 0 2px 0 var(--accent-color);
}

.view-host-window.is-external-tab-drop-target .view-host-titlebar {
  background: color-mix(in srgb, var(--sidebar-bg) 84%, var(--accent-soft) 16%);
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 48%, transparent),
    inset 0 2px 0 var(--accent-color);
}

.view-host-window.is-external-tab-drop-target .view-host-tabs::after {
  content: "";
  align-self: stretch;
  width: 32px;
  margin-left: 4px;
  border-left: 1px solid var(--accent-color);
  opacity: 0.8;
}

.view-host-window-controls {
  -webkit-app-region: no-drag;
  height: 100%;
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

.view-host-win-ctrl-btn {
  width: 42px;
  height: 100%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.view-host-win-ctrl-btn:hover,
.view-host-win-ctrl-btn:focus-visible {
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.view-host-win-ctrl-btn.view-host-win-pinned {
  color: var(--accent-color);
}

.view-host-win-close:hover,
.view-host-win-close:focus-visible {
  background: #e81123;
  color: #fff;
}

.view-host-body {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  position: relative;
  overflow: hidden;
}

.view-host-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.view-host-state-error {
  padding: 16px;
  color: var(--status-danger-fg);
  text-align: center;
}

.view-host-logbar {
  height: 24px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  color: var(--text-secondary);
  font-size: 11px;
  user-select: none;
}

.view-host-logbar.level-warn {
  color: var(--status-warn-fg);
}

.view-host-logbar.level-error {
  color: var(--status-danger-fg);
}

.view-host-logbar-label {
  flex-shrink: 0;
  font-weight: 650;
}

.view-host-logbar-message {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:global(.view-host-logbar-slot) {
  min-width: 0;
  flex: 1 1 auto;
  display: inline-flex;
  justify-content: flex-end;
  margin-left: auto;
}

:global(.view-host-logbar-inline) {
  max-width: min(46vw, 640px);
  min-width: 0;
  height: 18px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  line-height: 1;
  text-align: left;
  cursor: default;
}

:global(.view-host-logbar-inline.level-warn) {
  color: var(--status-warn-fg);
}

:global(.view-host-logbar-inline.level-error) {
  color: var(--status-danger-fg);
}

:global(.view-host-logbar-inline:hover),
:global(.view-host-logbar-inline:focus-visible) {
  color: var(--text-color);
  outline: none;
}

:global(.view-host-logbar-inline span) {
  min-width: initial;
  overflow: visible;
  text-overflow: clip;
  white-space: nowrap;
}

:global(.view-host-logbar-inline-label) {
  flex: 0 0 auto;
  font-weight: 650;
}

:global(.view-host-logbar-inline-message) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.view-runtime-frame {
  flex: 1;
  width: 100%;
  height: 100%;
  min-height: 0;
  display: flex;
  overflow: hidden;
  background: var(--bg-color);
}

.view-runtime-cache {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.view-runtime-cache.is-suspended {
  position: absolute;
  inset: 0;
  visibility: hidden;
  pointer-events: none;
}
</style>
