import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";
import type {
  AssetRefAttachment,
  PluginStatus,
  UnityConnectionStatus,
  UnityLaunchMode,
  UnityPluginInstallPlan,
} from "../types";
import { WORKSPACE_EVENT_NAME, type RoutedWorkspaceEvent, type WorkspaceRef } from "./project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

export type UnityServiceReadinessPhase =
  | "starting"
  | "connected"
  | "ready"
  | "reloading"
  | "degraded"
  | "stopped";

export interface UnityServiceReadinessSnapshot {
  phase: UnityServiceReadinessPhase;
  revision: number;
  detail?: string | null;
}

export interface UnityCheckoutConnectionStatus {
  checkoutId: string;
  workspaceGeneration: number;
  connected: boolean;
  ready: boolean;
  serviceStatus?: string | null;
  readiness?: UnityServiceReadinessSnapshot | null;
}

export interface AssetSearchResult {
  name: string;
  guid: string;
  fileID?: string;
  path: string;
  type: string;
}

export function checkUnityConnection(workspaceRef: WorkspaceRef): Promise<boolean> {
  return ipcInvoke<boolean>("check_unity_connection", { workspaceRef });
}

export function checkUnityConnectionStatus(
  workspaceRef: WorkspaceRef,
): Promise<UnityCheckoutConnectionStatus> {
  return ipcInvoke<UnityCheckoutConnectionStatus>("check_unity_connection_status", {
    workspaceRef,
  });
}

export function checkUnityPlugin(workspaceRef: WorkspaceRef): Promise<PluginStatus> {
  return ipcInvoke<PluginStatus>("check_unity_plugin", { workspaceRef });
}

export function checkUnityPluginInstallPlan(
  workspaceRef: WorkspaceRef,
): Promise<UnityPluginInstallPlan> {
  return ipcInvoke<UnityPluginInstallPlan>("check_unity_plugin_install_plan", { workspaceRef });
}

export interface InstallUnityPluginOptions {
  forceCloseUnity?: boolean;
}

export function installUnityPlugin(
  workspaceRef: WorkspaceRef,
  options: InstallUnityPluginOptions = {},
): Promise<string> {
  return ipcInvoke<string>("install_unity_plugin", {
    workspaceRef,
    forceCloseUnity: options.forceCloseUnity ?? false,
  });
}

export interface UnityLaunchResult {
  editorPath: string;
  projectPath: string;
  projectVersion: string;
  processId: number;
  mode: UnityLaunchMode;
}

export function launchUnityProject(workspaceRef: WorkspaceRef): Promise<UnityLaunchResult> {
  return ipcInvoke<UnityLaunchResult>("launch_unity_project", { workspaceRef });
}

export function closeHeadlessUnityProject(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke<void>("close_headless_unity_project", { workspaceRef });
}

export interface WorkspaceUnityStatusHandlers {
  onConnection?: (connected: boolean) => void;
  onConnectionDetail?: (status: UnityConnectionStatus) => void;
  onPluginStatus?: (status: PluginStatus) => void;
}

export function subscribeWorkspaceUnityStatus(
  workspaceRef: WorkspaceRef,
  handlers: WorkspaceUnityStatusHandlers,
): Promise<RuntimeUnsubscribe> {
  const scopedRef = { ...workspaceRef };
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent>(WORKSPACE_EVENT_NAME, (event) => {
    if (event.checkoutId !== scopedRef.checkoutId) return;
    if (
      scopedRef.expectedGeneration != null
      && event.workspaceGeneration !== scopedRef.expectedGeneration
    ) return;
    if (event.eventName === "unity-connection-status") {
      handlers.onConnection?.(event.payload as boolean);
    } else if (event.eventName === "unity-connection-status-detail") {
      handlers.onConnectionDetail?.(event.payload as UnityConnectionStatus);
    } else if (event.eventName === "unity-plugin-status") {
      handlers.onPluginStatus?.(event.payload as PluginStatus);
    }
  });
}

export interface SelectUnityAssetOptions {
  focusProjectWindow?: boolean;
}

export function selectUnityAsset(
  workspaceRef: WorkspaceRef,
  assetPath: string,
  options: SelectUnityAssetOptions = {},
): Promise<void> {
  const focusProjectWindow = options.focusProjectWindow ?? true;
  return ipcInvoke("select_unity_asset", { workspaceRef, assetPath, focusProjectWindow });
}

export function openUnityAssetInspector(
  workspaceRef: WorkspaceRef,
  assetPath: string,
): Promise<void> {
  return ipcInvoke("open_unity_asset_inspector", { workspaceRef, assetPath });
}

export function selectUnitySceneObject(
  workspaceRef: WorkspaceRef,
  scenePath: string,
  objectPath: string,
): Promise<void> {
  return ipcInvoke("select_unity_scene_object", { workspaceRef, scenePath, objectPath });
}

export function validateUnitySceneObject(
  workspaceRef: WorkspaceRef,
  scenePath: string,
  objectPath: string,
): Promise<void> {
  return ipcInvoke("validate_unity_scene_object", { workspaceRef, scenePath, objectPath });
}

export function openUnitySceneObjectInspector(
  workspaceRef: WorkspaceRef,
  scenePath: string,
  objectPath: string,
): Promise<void> {
  return ipcInvoke("open_unity_scene_object_inspector", { workspaceRef, scenePath, objectPath });
}

export type UnityEmbeddedFrontendTarget = "session" | "view";

export interface UnityEmbeddedFrontendWindowRequest {
  windowId?: string | null;
  targetKind: UnityEmbeddedFrontendTarget;
  targetId?: string | null;
  title?: string | null;
}

export interface UnityEmbeddedFrontendWindowResult {
  windowId: string;
  windowLabel: string;
  targetKind: UnityEmbeddedFrontendTarget;
  targetId: string;
  title: string;
  hostUrl: string;
}

export function currentUnityEmbedWindowId(): string | null {
  try {
    return new URLSearchParams(window.location.search).get("windowId");
  } catch {
    return null;
  }
}

export function currentUnityEmbedWorkspaceRef(): WorkspaceRef | null {
  try {
    const params = new URLSearchParams(window.location.search);
    const checkoutId = params.get("checkoutId")?.trim() ?? "";
    const rawGeneration = params.get("workspaceGeneration");
    if (!checkoutId || rawGeneration == null || !/^\d+$/.test(rawGeneration)) return null;
    const expectedGeneration = Number(rawGeneration);
    if (!Number.isSafeInteger(expectedGeneration) || expectedGeneration < 0) return null;
    return { checkoutId, expectedGeneration };
  } catch {
    return null;
  }
}

function requireUnityEmbedWorkspaceRef(explicit?: WorkspaceRef | null): WorkspaceRef {
  const workspaceRef = explicit
    ?? currentUnityEmbedWorkspaceRef()
    ?? useWorkspaceContextStore().focusedWorkspaceRef;
  if (!workspaceRef) throw new Error("Unity embed requires an explicit checkout workspace.");
  return workspaceRef;
}

export function getUnityEmbedEnabled(): Promise<boolean> {
  return ipcInvoke<boolean>("get_unity_embed_enabled");
}

export function setUnityEmbedEnabled(value: boolean, workspaceRef?: WorkspaceRef | null): Promise<boolean> {
  return ipcInvoke<boolean>("set_unity_embed_enabled", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    value,
  });
}

export interface UnityTestToolsWorkspaceStatus {
  enabled: boolean;
  packageInstalled: boolean;
  packageVersion: string | null;
  packageSupported: boolean;
  available: boolean;
}

export function getUnityTestToolsWorkspaceStatus(
  workspaceRef: WorkspaceRef,
): Promise<UnityTestToolsWorkspaceStatus> {
  return ipcInvoke<UnityTestToolsWorkspaceStatus>("get_unity_test_tools_workspace_status", {
    workspaceRef,
  });
}

export function setUnityTestToolsWorkspaceEnabled(
  workspaceRef: WorkspaceRef,
  value: boolean,
): Promise<UnityTestToolsWorkspaceStatus> {
  return ipcInvoke<UnityTestToolsWorkspaceStatus>("set_unity_test_tools_workspace_enabled", {
    workspaceRef,
    value,
  });
}

export function openUnityEmbeddedFrontendWindow(
  workspaceRef: WorkspaceRef,
  request: UnityEmbeddedFrontendWindowRequest,
): Promise<UnityEmbeddedFrontendWindowResult> {
  return ipcInvoke<UnityEmbeddedFrontendWindowResult>("unity_embed_open_frontend_window", {
    workspaceRef,
    request,
  });
}

export function openUnityEmbeddedSessionWindow(workspaceRef: WorkspaceRef, request: {
  sessionId?: string | null;
  title?: string | null;
} = {}): Promise<UnityEmbeddedFrontendWindowResult> {
  const sessionId = request.sessionId?.trim() || null;
  return openUnityEmbeddedFrontendWindow(workspaceRef, {
    targetKind: "session",
    targetId: sessionId,
    title: request.title?.trim()
      ? `Locus Session - ${request.title.trim()}${sessionId ? ` (${sessionId.slice(0, 8)})` : ""}`
      : sessionId
        ? `Locus Session (${sessionId.slice(0, 8)})`
        : "Locus Session",
  });
}

export function setUnityEmbedMouseActivationSuppressed(
  suppressed: boolean,
  workspaceRef?: WorkspaceRef | null,
): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_set_mouse_activation_suppressed", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    windowId: currentUnityEmbedWindowId(),
    suppressed,
  });
}

export function activateUnityEmbedForInput(workspaceRef?: WorkspaceRef | null): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_activate_for_input", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    windowId: currentUnityEmbedWindowId(),
  });
}

export function setUnityEmbedDragPassthrough(
  active: boolean,
  workspaceRef?: WorkspaceRef | null,
): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_set_drag_passthrough", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    windowId: currentUnityEmbedWindowId(),
    active,
  });
}

export function commitUnityEmbedAssetDrop(workspaceRef?: WorkspaceRef | null): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_commit_asset_drop", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
  });
}

export function startUnityEmbedAssetDrag(
  refs: AssetRefAttachment[],
  workspaceRef?: WorkspaceRef | null,
): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri" || refs.length === 0) return Promise.resolve();
  return runtime.invoke("unity_embed_start_asset_drag", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    request: {
      refs,
    },
  });
}

export function cancelUnityEmbedAssetDrag(workspaceRef?: WorkspaceRef | null): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("unity_embed_cancel_asset_drag", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
  });
}

export function startUnityNativeAssetFileDrag(
  refs: AssetRefAttachment[],
  workspaceRef?: WorkspaceRef | null,
): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri" || refs.length === 0) return Promise.resolve();
  return runtime.invoke("unity_embed_start_native_asset_file_drag", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    request: {
      refs,
    },
  });
}

export function startLocusNativeFileDrag(
  files: LocusFileDropRef[],
  workspaceRef?: WorkspaceRef | null,
): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri" || files.length === 0) return Promise.resolve();
  return runtime.invoke("locus_start_native_file_drag", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    request: { files },
  });
}

export interface LocusDragPreviewAnchor {
  x: number;
  y: number;
}

interface LocusDragPreviewColor {
  red: number;
  green: number;
  blue: number;
}

interface LocusDragPreviewAppearance {
  width: number;
  height: number;
  pointerOffsetX: number;
  pointerOffsetY: number;
  scaleFactor: number;
  backgroundColor: LocusDragPreviewColor;
  borderColor: LocusDragPreviewColor;
  textColor: LocusDragPreviewColor;
  iconColor: LocusDragPreviewColor;
}

function clampDragPreviewColor(value: string | undefined, scale = 1): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return 0;
  return Math.min(255, Math.max(0, Math.round(numeric * scale)));
}

function computedDragPreviewColor(value: string, fallback: LocusDragPreviewColor): LocusDragPreviewColor {
  const rgb = value.match(/rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
  if (rgb) {
    return {
      red: clampDragPreviewColor(rgb[1]),
      green: clampDragPreviewColor(rgb[2]),
      blue: clampDragPreviewColor(rgb[3]),
    };
  }
  const srgb = value.match(/color\(srgb\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)/i);
  if (srgb) {
    return {
      red: clampDragPreviewColor(srgb[1], 255),
      green: clampDragPreviewColor(srgb[2], 255),
      blue: clampDragPreviewColor(srgb[3], 255),
    };
  }
  return fallback;
}

function currentDragPreviewAppearance(
  anchor: LocusDragPreviewAnchor,
  sourceDocument?: Document,
): LocusDragPreviewAppearance | null {
  if (typeof document === "undefined" || typeof window === "undefined") return null;
  const previewDocument = sourceDocument ?? document;
  const sourceWindow = previewDocument.defaultView ?? window;
  const preview = previewDocument.querySelector<HTMLElement>("[data-internal-drag-preview]");
  if (!preview) return null;
  const bounds = preview.getBoundingClientRect();
  const style = sourceWindow.getComputedStyle(preview);
  const icon = preview.querySelector<HTMLElement>(".internal-drag-overlay-icon");
  const iconStyle = icon ? sourceWindow.getComputedStyle(icon) : style;
  return {
    width: Math.max(1, bounds.width),
    height: Math.max(1, bounds.height),
    pointerOffsetX: Math.min(bounds.width, Math.max(0, anchor.x)),
    pointerOffsetY: Math.min(bounds.height, Math.max(0, anchor.y)),
    scaleFactor: Math.min(4, Math.max(0.5, sourceWindow.devicePixelRatio || 1)),
    backgroundColor: computedDragPreviewColor(style.backgroundColor, { red: 31, green: 36, blue: 44 }),
    borderColor: computedDragPreviewColor(style.borderColor, { red: 74, green: 84, blue: 98 }),
    textColor: computedDragPreviewColor(style.color, { red: 238, green: 242, blue: 248 }),
    iconColor: computedDragPreviewColor(iconStyle.color, { red: 168, green: 178, blue: 190 }),
  };
}

export function startLocusDragPreview(
  label: string,
  anchor?: LocusDragPreviewAnchor | null,
  sourceDocument?: Document,
): Promise<void> {
  const runtime = getLocusRuntime();
  const normalized = label.trim();
  if (runtime.kind !== "tauri" || !normalized) return Promise.resolve();
  return runtime.invoke("locus_start_drag_preview", {
    label: normalized,
    preview: anchor ? currentDragPreviewAppearance(anchor, sourceDocument) : null,
  });
}

export function stopLocusDragPreview(): Promise<void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve();
  return runtime.invoke("locus_stop_drag_preview");
}

export interface UnityEmbedAssetDropPayload {
  refs: AssetRefAttachment[];
  workspaceRef?: WorkspaceRef;
}

export interface UnityEmbedTextDropEntry {
  text: string;
  title?: string;
  source?: string;
  level?: string;
}

export interface UnityEmbedTextDropPayload {
  workspaceRef: WorkspaceRef;
  text: string;
  entries?: UnityEmbedTextDropEntry[];
  title?: string;
  source?: string;
}

export interface UnityConsoleTextPayload {
  text: string;
  entries?: UnityEmbedTextDropEntry[];
  title?: string;
  source?: string;
}

const UNITY_CONSOLE_ERROR_LEVEL_MARKERS = ["error", "assert", "exception", "fatal"] as const;

export function isUnityConsoleErrorLevel(level: string | null | undefined): boolean {
  const normalized = level?.trim().toLowerCase() ?? "";
  return UNITY_CONSOLE_ERROR_LEVEL_MARKERS.some((marker) => normalized.includes(marker));
}

export function filterUnityConsoleErrorPayload(
  payload: UnityConsoleTextPayload,
): UnityConsoleTextPayload {
  return {
    ...payload,
    text: "",
    entries: (payload.entries ?? []).filter((entry) => isUnityConsoleErrorLevel(entry.level)),
  };
}

export interface LocusFileDropRef {
  path: string;
  name?: string;
  typeLabel?: string;
  isDir: boolean;
  source?: string;
}

export interface LocusFileDropPayload {
  files: LocusFileDropRef[];
  x?: number;
  y?: number;
}

export interface UnitySendToLocusPayload {
  assetRefs?: AssetRefAttachment[];
  files?: LocusFileDropRef[];
}

export interface UnitySendToLocusEventPayload extends UnitySendToLocusPayload {
  workspaceRef: WorkspaceRef;
}

export interface LocusFileDragStatePayload {
  phase: "enter" | "over" | "drop" | "leave";
  active: boolean;
  fileCount: number;
  tabEligible: boolean;
  x: number;
  y: number;
}

export interface UnityEmbedAssetDragStatePayload {
  hasRefs: boolean;
  refs: AssetRefAttachment[];
}

export function subscribeUnitySendToLocus(
  handler: (payload: UnitySendToLocusEventPayload) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<UnitySendToLocusPayload>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName !== "unity-send-to-locus") return;
      handler({
        ...event.payload,
        workspaceRef: {
          checkoutId: event.checkoutId,
          expectedGeneration: event.workspaceGeneration,
        },
      });
    },
  );
}

export function subscribeUnityEmbedAssetDrop(
  handler: (payload: UnityEmbedAssetDropPayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<UnityEmbedAssetDropPayload>("unity-embed-asset-drop", handler);
}

export function subscribeUnityEmbedTextDrop(
  handler: (payload: UnityEmbedTextDropPayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<UnityEmbedTextDropPayload>("unity-embed-text-drop", handler);
}

export function getUnityConsoleText(
  workspaceRef: WorkspaceRef,
): Promise<UnityConsoleTextPayload> {
  return ipcInvoke<UnityConsoleTextPayload>("get_unity_console_text", { workspaceRef });
}

export function subscribeLocusFileDrop(
  handler: (payload: LocusFileDropPayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<LocusFileDropPayload>("locus-file-drop", handler);
}

export function subscribeLocusFileDragState(
  handler: (payload: LocusFileDragStatePayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<LocusFileDragStatePayload>("locus-file-drag-state", handler);
}

export function subscribeUnityEmbedAssetDragState(
  handler: (payload: UnityEmbedAssetDragStatePayload) => void,
): Promise<() => void> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(() => {});
  return runtime.subscribe<UnityEmbedAssetDragStatePayload>("unity-embed-asset-drag-state", handler);
}

export interface UnityEmbedFocusDebugSnapshot {
  ok: boolean;
  reason: string;
  foregroundHwnd: number;
  foregroundTitle: string;
  inputFocusHwnd?: number;
  inputFocusTitle?: string;
  overlayHwnd: number;
  overlayTitle: string;
  overlayVisible: boolean;
  overlayForeground: boolean;
  overlayInputFocused?: boolean;
  overlayChildWindow: boolean;
  overlayParentHwnd: number;
  overlayNoActivate: boolean;
  activationGuardEnabled: boolean;
  mouseActivateHookInstalled: boolean;
  mouseActivateHookedHwndCount: number;
  mouseActivateBlockCount: number;
  mouseActivationSuppressed: boolean;
  parentHwnd: number;
  parentTitle: string;
  parentVisible: boolean;
  parentForeground: boolean;
}

export function getUnityEmbedFocusDebugSnapshot(
  workspaceRef?: WorkspaceRef | null,
): Promise<UnityEmbedFocusDebugSnapshot | null> {
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri") return Promise.resolve(null);
  return runtime.invoke<UnityEmbedFocusDebugSnapshot>("unity_embed_focus_debug_snapshot", {
    workspaceRef: requireUnityEmbedWorkspaceRef(workspaceRef),
    windowId: currentUnityEmbedWindowId(),
  });
}

export type UnitySceneObjectErrorKind = "sceneNotLoaded" | "objectMissing" | "unknown";

export function classifyUnitySceneObjectError(error: unknown): UnitySceneObjectErrorKind {
  const message = typeof error === "object" && error !== null && "message" in error
    ? String((error as { message?: unknown }).message ?? "")
    : String(error ?? "");

  if (/scene is not loaded/i.test(message)) return "sceneNotLoaded";
  if (/gameobject was not found/i.test(message)) return "objectMissing";
  return "unknown";
}

export function searchAssets(query: string): Promise<AssetSearchResult[]> {
  return ipcInvoke<AssetSearchResult[]>("search_assets", { query });
}

export function sendUnityLog(workspaceRef: WorkspaceRef, message: string): Promise<void> {
  return ipcInvoke("send_unity_log", { workspaceRef, message });
}

export function focusedWorkspaceRef(): WorkspaceRef {
  const workspaceRef = useWorkspaceContextStore().focusedWorkspaceRef;
  if (!workspaceRef) throw new Error("A focused checkout is required");
  return workspaceRef;
}

export function openFileExternal(filePath: string): Promise<void>;
export function openFileExternal(filePath: string, workspaceRef: WorkspaceRef): Promise<void>;
export function openFileExternal(workspaceRef: WorkspaceRef, filePath: string): Promise<void>;
export function openFileExternal(
  workspaceRefOrPath: WorkspaceRef | string,
  scopedPathOrRef?: string | WorkspaceRef,
): Promise<void> {
  const workspaceRef = typeof workspaceRefOrPath === "string"
    ? (typeof scopedPathOrRef === "object" ? scopedPathOrRef : focusedWorkspaceRef())
    : workspaceRefOrPath;
  const filePath = typeof workspaceRefOrPath === "string"
    ? workspaceRefOrPath
    : typeof scopedPathOrRef === "string" ? scopedPathOrRef : "";
  return ipcInvoke("open_file_external", { workspaceRef, filePath });
}

export function showInFolder(filePath: string): Promise<void>;
export function showInFolder(filePath: string, workspaceRef: WorkspaceRef): Promise<void>;
export function showInFolder(workspaceRef: WorkspaceRef, filePath: string): Promise<void>;
export function showInFolder(
  workspaceRefOrPath: WorkspaceRef | string,
  scopedPathOrRef?: string | WorkspaceRef,
): Promise<void> {
  const workspaceRef = typeof workspaceRefOrPath === "string"
    ? (typeof scopedPathOrRef === "object" ? scopedPathOrRef : focusedWorkspaceRef())
    : workspaceRefOrPath;
  const filePath = typeof workspaceRefOrPath === "string"
    ? workspaceRefOrPath
    : typeof scopedPathOrRef === "string" ? scopedPathOrRef : "";
  return ipcInvoke("reveal_workspace_file", { workspaceRef, filePath });
}

export interface WorkspaceFilePreview {
  displayPath: string;
  exists: boolean;
  kind: "text" | "binary" | "not_found";
  language?: string;
  snippet?: string;
  truncated: boolean;
  isUnityAsset: boolean;
  preferredAction: "editor" | "unity" | "external";
  fileSize?: number;
  snippetStartLine: number;
  previewSuppressed?: "largeFile" | string;
}

export function previewWorkspaceFile(filePath: string, line?: number, full?: boolean): Promise<WorkspaceFilePreview>;
export function previewWorkspaceFile(workspaceRef: WorkspaceRef, filePath: string, line?: number, full?: boolean): Promise<WorkspaceFilePreview>;
export function previewWorkspaceFile(
  workspaceRefOrPath: WorkspaceRef | string,
  scopedPathOrLine?: string | number,
  scopedLineOrFull?: number | boolean,
  scopedFull = false,
): Promise<WorkspaceFilePreview> {
  const legacy = typeof workspaceRefOrPath === "string";
  const workspaceRef = legacy ? focusedWorkspaceRef() : workspaceRefOrPath;
  const filePath = legacy ? workspaceRefOrPath : String(scopedPathOrLine ?? "");
  const line = legacy
    ? (typeof scopedPathOrLine === "number" ? scopedPathOrLine : undefined)
    : (typeof scopedLineOrFull === "number" ? scopedLineOrFull : undefined);
  const full = legacy
    ? (typeof scopedLineOrFull === "boolean" ? scopedLineOrFull : false)
    : scopedFull;
  return ipcInvoke<WorkspaceFilePreview>("preview_workspace_file", { workspaceRef, filePath, line, full });
}
