<script setup lang="ts">
import { computed, markRaw, nextTick, onMounted, onUnmounted, ref, shallowRef, watch, type Component } from "vue";
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
import { checkUnityConnectionStatus } from "../services/unity";
import {
  viewAppendFrontendLog,
  viewAutomationRespond,
  viewBindingApply,
  viewBindingDiscover,
  viewBindingRead,
  viewBindingWrite,
  viewCallScript,
  viewDetachTab,
  viewHostIdFromLocation,
  viewOpenFrontendLog,
  viewRead,
  viewReadFrontendLog,
  viewRequiresUnityConnection,
  viewSetTabHost,
  type ViewFrontendLogEntry,
  type ViewFrontendLogLevel,
  type ViewAutomationRequest,
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
const VIEW_HOST_TABS_SELECT_EVENT = "view-host-tabs-select";
const VIEW_HOST_WINDOW_LABEL_PREFIX = "view-";
const VIEW_HOST_TAB_DROP_HEIGHT_PX = 40;
const VIEW_HOST_TAB_DRAG_THRESHOLD_PX = 8;
const VIEW_HOST_TAB_DRAG_FRAME_MS = 16;
const VIEW_HOST_DETACH_OFFSET_X = 96;
const VIEW_HOST_DETACH_OFFSET_Y = 18;

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

interface ViewHostTabsSelectPayload {
  viewId: string;
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
const currentWindowLabel = appWindow?.label ?? "";
const activeViewId = ref(initialViewId);
const tabs = ref<ViewHostTab[]>(initialViewId
  ? [{ id: initialViewId, title: initialViewId, packageRoot: "" }]
  : []);
const detail = ref<ViewPackageDetail | null>(null);
const runtimeComponent = shallowRef<Component | null>(null);
const loading = ref(false);
const error = ref("");
const isMaximized = ref(false);
const alwaysOnTop = ref(false);
const tabDragState = ref<ViewHostDragState | null>(null);
const tabDropTargetLabel = ref("");
const latestFrontendLog = ref<ViewFrontendLogEntry | null>(null);
const runtimeFrameRef = ref<HTMLElement | null>(null);
const embeddedLogbarSlot = shallowRef<HTMLElement | null>(null);
let unsubscribeReload: RuntimeUnsubscribe | null = null;
let restoreConsoleLogCapture: (() => void) | null = null;
let unlistenTabMerge: UnlistenFn | null = null;
let unlistenTabSelect: UnlistenFn | null = null;
let loadViewPromise: Promise<void> | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let reloadQueued = false;
let suppressNextTabClickId = "";
let suppressNextTabClickTimer: ReturnType<typeof setTimeout> | null = null;
let statusbarObserver: MutationObserver | null = null;
let embeddedLogbarSyncTimer: ReturnType<typeof setTimeout> | null = null;
let unsubscribeAutomation: RuntimeUnsubscribe | null = null;
let automationElementSeq = 0;
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

const manifest = computed(() => detail.value?.manifest ?? null);
const activeTab = computed(() => tabs.value.find((tab) => tab.id === activeViewId.value) ?? null);
const windowTitle = computed(() =>
  manifest.value?.name || detail.value?.summary.name || activeTab.value?.title || activeViewId.value || t("view.host.title"),
);
const runtimeLabel = computed(() => manifest.value?.name || activeViewId.value || t("view.host.untitled"));
const latestFrontendLogLevel = computed(() => latestFrontendLog.value?.level ?? "log");
const latestFrontendLogText = computed(() => {
  const entry = latestFrontendLog.value;
  if (!entry) return "No frontend log";
  const message = firstLogLine(entry.message);
  return message ? `${entry.level.toUpperCase()} ${message}` : `${entry.level.toUpperCase()} empty message`;
});

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
    latestFrontendLog.value = {
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
  try {
    const entries = await viewReadFrontendLog({ viewId, limit: 1 });
    latestFrontendLog.value = entries[entries.length - 1] ?? null;
  } catch {
    latestFrontendLog.value = null;
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

function removeTab(tabId: string): ViewHostTab | null {
  const index = tabs.value.findIndex((tab) => tab.id === tabId);
  if (index < 0) return null;
  const removed = tabs.value[index];
  const nextTabs = tabs.value.filter((tab) => tab.id !== tabId);
  tabs.value = nextTabs;
  if (activeViewId.value === tabId) {
    const nextActive = nextTabs[Math.min(index, nextTabs.length - 1)]?.id ?? "";
    activeViewId.value = nextActive;
    error.value = "";
    runtimeComponent.value = null;
    latestFrontendLog.value = null;
    clearEmbeddedLogbarSlot();
    refreshConsoleLogCapture();
    if (nextActive) {
      scheduleLoadView(0);
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
  if (!currentWindowLabel || !currentWindowLabel.startsWith(VIEW_HOST_WINDOW_LABEL_PREFIX)) return;
  const viewIds = normalizeTabIds(tabs.value.map((tab) => tab.id));
  if (viewIds.length === 0) return;
  try {
    await viewSetTabHost({ hostLabel: currentWindowLabel, viewIds });
  } catch (hostError) {
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
  if (!tabs.value.some((tab) => tab.id === normalized)) {
    upsertTab(await resolveViewTab(normalized));
  }
  const changed = activeViewId.value !== normalized;
  if (changed) {
    activeViewId.value = normalized;
    error.value = "";
    runtimeComponent.value = null;
    latestFrontendLog.value = null;
    clearEmbeddedLogbarSlot();
    refreshConsoleLogCapture();
  }
  void refreshLatestFrontendLog();
  await registerCurrentTabHost();
  if (options.loadNow) {
    const inFlightLoad = loadViewPromise;
    if (inFlightLoad) await inFlightLoad.catch(() => undefined);
    await loadView();
  } else if (changed) {
    scheduleLoadView(0);
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
  await setActiveViewTab(payload.activeViewId || incomingIds[incomingIds.length - 1] || nextIds[0] || activeViewId.value);
  await registerCurrentTabHost();
}

async function selectHostedViewTab(payload: ViewHostTabsSelectPayload) {
  await setActiveViewTab(payload.viewId, { loadNow: true });
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
  tabDropTargetLabel.value = await findTabDropTargetAt(point);
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
    scheduleTabDragFrame();
  }
  if (state.dragging) event.preventDefault();
}

async function mergeTabIntoWindow(tabId: string, targetLabel: string) {
  if (!targetLabel || targetLabel === currentWindowLabel) return;
  const tab = tabs.value.find((item) => item.id === tabId);
  if (!tab) return;
  await emitTo<ViewHostTabsMergePayload>(targetLabel, VIEW_HOST_TABS_MERGE_EVENT, {
    sourceLabel: currentWindowLabel,
    viewIds: [tab.id],
    activeViewId: tab.id,
  });
  removeTab(tab.id);
  if (tabs.value.length > 0) {
    await registerCurrentTabHost();
  }
  const targetWindow = await TauriWindow.getByLabel(targetLabel).catch(() => null);
  await targetWindow?.setFocus().catch(() => undefined);
  if (tabs.value.length === 0) {
    await appWindow?.close().catch(() => appWindow?.destroy().catch(() => undefined));
  }
}

async function detachTab(tabId: string, point: { x: number; y: number }) {
  if (tabs.value.length <= 1) return;
  const tab = tabs.value.find((item) => item.id === tabId);
  if (!tab) return;
  removeTab(tab.id);
  await registerCurrentTabHost();
  try {
    await viewDetachTab({
      viewId: tab.id,
      sourceHostLabel: currentWindowLabel,
      x: Math.max(0, Math.round(point.x - VIEW_HOST_DETACH_OFFSET_X)),
      y: Math.max(0, Math.round(point.y - VIEW_HOST_DETACH_OFFSET_Y)),
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
  if (!wasDragging) {
    tabDropTargetLabel.value = "";
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
  tabDropTargetLabel.value = "";
  if (targetLabel) {
    await mergeTabIntoWindow(state.tabId, targetLabel);
    return;
  }
  if (await isCurrentTabBandAt(releasePoint)) {
    return;
  }
  await detachTab(state.tabId, releasePoint);
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
    void setActiveViewTab(tabId);
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
  return runtimeFrameRef.value ?? document.body;
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
  const frame = runtimeFrameRef.value;
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

  const frame = runtimeFrameRef.value;
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

async function loadView() {
  if (loadViewPromise) {
    reloadQueued = true;
    return loadViewPromise;
  }

  const viewId = activeViewId.value;
  if (!viewId) {
    error.value = t("view.host.missingId");
    return;
  }

  reloadQueued = false;
  loadViewPromise = (async () => {
    loading.value = true;
    error.value = "";
    try {
      const next = await viewRead(viewId);
      if (activeViewId.value !== viewId) return;
      detail.value = next;
      upsertTab(tabFromDetail(next));
      void registerCurrentTabHost();
      if (viewRequiresUnityConnection(next.manifest)) {
        const status = await checkUnityConnectionStatus();
        if (activeViewId.value !== viewId) return;
        if (!status.connected) {
          error.value = t("view.host.unityConnectionRequired");
          runtimeComponent.value = null;
          return;
        }
      }
      runtimeComponent.value = markRaw(
        createViewRuntimeComponent({
          detail: next,
          api: {
            callScript: (scriptName, method, args) =>
              viewCallScript({ viewId: next.manifest.id, scriptName, method, args }),
            bindingRead: (request) => viewBindingRead({ viewId: next.manifest.id, ...request }),
            bindingDiscover: (request) =>
              viewBindingDiscover({ viewId: next.manifest.id, ...request }),
            bindingWrite: (request) => viewBindingWrite({ viewId: next.manifest.id, ...request }),
            bindingApply: (request) => viewBindingApply({ viewId: next.manifest.id, ...request }),
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
            onUpdate: (handler) =>
              getLocusRuntime().subscribe<ViewRuntimeUpdateEvent>("unity-editor-update", handler),
            reload: loadView,
          },
        }),
      );
    } catch (loadError) {
      error.value = normalizeAppError(loadError).message;
      console.error("[view-host]", loadError);
      runtimeComponent.value = null;
    } finally {
      loading.value = false;
      loadViewPromise = null;
      if (reloadQueued) scheduleLoadView(0);
    }
  })();

  return loadViewPromise;
}

function scheduleLoadView(delay = 120) {
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void loadView();
  }, delay);
}

watch(runtimeComponent, () => {
  void nextTick().then(installStatusbarObserver);
});

onMounted(async () => {
  void syncMaximizedState();
  refreshConsoleLogCapture();
  void refreshLatestFrontendLog();
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
      void registerCurrentTabHost();
      if (payload.id === activeViewId.value) scheduleLoadView();
    },
  );
  await loadView();
  await nextTick();
  installStatusbarObserver();
});

onUnmounted(() => {
  if (tabDragState.value?.raf !== null) clearTimeout(tabDragState.value.raf);
  tabDragState.value = null;
  window.removeEventListener("pointermove", onTabDragPointerMove);
  window.removeEventListener("pointerup", onTabDragPointerUp);
  window.removeEventListener("pointercancel", onTabDragPointerCancel);
  if (suppressNextTabClickTimer) clearTimeout(suppressNextTabClickTimer);
  suppressNextTabClickTimer = null;
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = null;
  if (embeddedLogbarSyncTimer) clearTimeout(embeddedLogbarSyncTimer);
  embeddedLogbarSyncTimer = null;
  statusbarObserver?.disconnect();
  statusbarObserver = null;
  clearEmbeddedLogbarSlot();
  unsubscribeReload?.();
  unsubscribeReload = null;
  unsubscribeAutomation?.();
  unsubscribeAutomation = null;
  unlistenTabMerge?.();
  unlistenTabMerge = null;
  unlistenTabSelect?.();
  unlistenTabSelect = null;
  handledAutomationRequests.clear();
  restoreConsoleLogCapture?.();
  restoreConsoleLogCapture = null;
});
</script>

<template>
  <main
    class="view-host-window"
    :class="{ 'is-tab-dragging': tabDragState?.dragging, 'is-tab-drop-target': !!tabDropTargetLabel }"
  >
    <header class="view-host-titlebar" data-tauri-drag-region @dblclick="toggleMaximizeWindow">
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
          :class="{ active: tab.id === activeViewId }"
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

    <section class="view-host-body">
      <div v-if="error" class="view-host-state view-host-state-error">{{ error }}</div>
      <div v-else-if="loading && !detail" class="view-host-state">{{ t("common.loading") }}</div>
      <div
        v-else-if="runtimeComponent"
        ref="runtimeFrameRef"
        class="view-runtime-frame"
        :aria-label="runtimeLabel"
      >
        <component :is="runtimeComponent" />
      </div>
    </section>
    <Teleport v-if="embeddedLogbarSlot" :to="embeddedLogbarSlot">
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
      v-else
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

.view-host-window.is-tab-drop-target .view-host-titlebar {
  box-shadow: inset 0 2px 0 var(--accent-color);
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
</style>
