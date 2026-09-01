import { ref, type Ref } from "vue";
import { invokeLocusRuntime } from "../services/locusRuntime";
import { listen } from "@tauri-apps/api/event";
import { Window as TauriWindow, type Window as TauriWindowHandle } from "@tauri-apps/api/window";
import type { WorkbenchWindowDropIntent } from "../types/workbench";
import { startLocusDragPreview, stopLocusDragPreview } from "../services/unity";
import {
  isWorkbenchWindowLabel,
  recordWorkbenchWindowMetric,
  type WorkbenchWindowScreenPoint,
} from "../services/workbenchWindow";

const WORKBENCH_NATIVE_DRAG_POINT_EVENT = "shared-workbench:drag-point";

export interface WorkbenchWindowTabDragItem {
  id: string;
  title: string;
  sourceWindowId: string;
  sourcePaneId: string;
  anchor: { x: number; y: number };
  move: (intent: WorkbenchWindowDropIntent) => void | Promise<void>;
  transfer: (intent: WorkbenchWindowDropIntent) => void | Promise<void>;
  detach: (
    point: WorkbenchWindowScreenPoint,
    anchor: { x: number; y: number },
  ) => void | Promise<void>;
}

export interface WorkbenchWindowTabDragController {
  dropTarget: Ref<WorkbenchWindowDropIntent | null>;
  externalize: (
    item: WorkbenchWindowTabDragItem,
    previewAnchor: { x: number; y: number },
  ) => void;
  dispose: () => void;
}

interface ControllerState {
  dropTarget: Ref<WorkbenchWindowDropIntent | null>;
  disposed: boolean;
  windowLabel: string;
  ownerWindow: Window;
  resolveClientPoint?: (x: number, y: number) => WorkbenchWindowDropIntent | null;
}

interface NativeWorkbenchDragSession {
  item: WorkbenchWindowTabDragItem;
  source: ControllerState;
  startedAt: number;
  cancelled: boolean;
  dropped: boolean;
  observedPressed: boolean;
  finishing: boolean;
  dragOverCount: number;
  dragOverTotalMs: number;
  dragOverMaxMs: number;
  removeEscapeListener: () => void;
}

interface NativeWorkbenchDragPoint extends WorkbenchWindowScreenPoint {
  leftButtonPressed?: boolean;
  targetWindowLabel?: string | null;
}

const controllerStates = new Set<ControllerState>();
let activeSession: NativeWorkbenchDragSession | null = null;
let nativePointListener: Promise<() => void> | null = null;
let pendingNativePoint: NativeWorkbenchDragPoint | null = null;
let nativePointFrame = 0;

function clearAllTargets(): void {
  for (const state of controllerStates) state.dropTarget.value = null;
}

function clearSession(session: NativeWorkbenchDragSession): void {
  if (activeSession === session) activeSession = null;
  session.removeEscapeListener();
  clearAllTargets();
}

function controllerTargetAt(
  point: WorkbenchWindowScreenPoint,
  source: ControllerState,
  includeSource: boolean,
  targetWindowLabel?: string | null,
): { state: ControllerState; intent: WorkbenchWindowDropIntent | null } | null {
  const candidates = targetWindowLabel
    ? [...controllerStates].filter((candidate) => candidate.windowLabel === targetWindowLabel)
    : [...controllerStates].reverse();
  for (const candidate of candidates) {
    if ((!includeSource && candidate === source) || candidate.disposed || !candidate.resolveClientPoint) continue;
    const scale = candidate.ownerWindow.devicePixelRatio || 1;
    const left = candidate.ownerWindow.screenX * scale;
    const top = candidate.ownerWindow.screenY * scale;
    const right = left + candidate.ownerWindow.outerWidth * scale;
    const bottom = top + candidate.ownerWindow.outerHeight * scale;
    if (point.x < left || point.x > right || point.y < top || point.y > bottom) continue;
    return {
      state: candidate,
      intent: candidate.resolveClientPoint(
        point.x / scale - candidate.ownerWindow.screenX,
        point.y / scale - candidate.ownerWindow.screenY,
      ),
    };
  }
  return null;
}

async function finishExternalizedDrag(
  session: NativeWorkbenchDragSession,
  point: NativeWorkbenchDragPoint,
): Promise<void> {
  if (session.finishing || activeSession !== session) return;
  session.finishing = true;
  const target = controllerTargetAt(
    point,
    session.source,
    true,
    point.targetWindowLabel,
  );
  session.dropped = !!target?.intent;
  void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
  void stopLocusDragPreview().catch(() => undefined);
  recordDragSummary(session);
  const targetWindowLabel = point.targetWindowLabel;
  const releaseWindow = target || targetWindowLabel ? null : await workbenchWindowAt(point);
  clearSession(session);
  if (target?.intent) {
    if (target.intent.windowId === session.item.sourceWindowId) {
      await session.item.move(target.intent);
    } else {
      await session.item.transfer(target.intent);
    }
    return;
  }
  if (targetWindowLabel || releaseWindow) return;
  await session.item.detach(point, session.item.anchor);
}

function applyNativeDragPoint(point: NativeWorkbenchDragPoint): void {
  const session = activeSession;
  if (!session || session.cancelled) return;
  clearAllTargets();
  const decisionStartedAt = performance.now();
  const target = controllerTargetAt(point, session.source, true, point.targetWindowLabel);
  const decisionMs = performance.now() - decisionStartedAt;
  session.dragOverCount += 1;
  session.dragOverTotalMs += decisionMs;
  session.dragOverMaxMs = Math.max(session.dragOverMaxMs, decisionMs);
  if (target?.intent) target.state.dropTarget.value = target.intent;
  if (point.leftButtonPressed === true) session.observedPressed = true;
  else if (point.leftButtonPressed === false && session.observedPressed) {
    void finishExternalizedDrag(session, point);
  }
}

function ensureNativePointListener(): void {
  if (nativePointListener) return;
  nativePointListener = listen<NativeWorkbenchDragPoint>(
    WORKBENCH_NATIVE_DRAG_POINT_EVENT,
    (event) => {
      pendingNativePoint = event.payload;
      if (nativePointFrame) return;
      nativePointFrame = window.requestAnimationFrame(() => {
        nativePointFrame = 0;
        const point = pendingNativePoint;
        pendingNativePoint = null;
        if (point) applyNativeDragPoint(point);
      });
    },
  );
}

async function workbenchWindowAt(
  point: WorkbenchWindowScreenPoint,
): Promise<TauriWindowHandle | null> {
  const windows = await TauriWindow.getAll().catch(() => []);
  const candidates = await Promise.all(windows
    .filter((candidate) => isWorkbenchWindowLabel(candidate.label))
    .map(async (candidate) => {
      try {
        const [visible, minimized, position, size] = await Promise.all([
          candidate.isVisible(),
          candidate.isMinimized(),
          candidate.outerPosition(),
          candidate.outerSize(),
        ]);
        if (!visible || minimized) return null;
        if (
          point.x < position.x
          || point.x > position.x + size.width
          || point.y < position.y
          || point.y > position.y + size.height
        ) return null;
        return candidate;
      } catch {
        return null;
      }
    }));
  const matches = candidates.filter(
    (candidate): candidate is TauriWindowHandle => candidate !== null,
  );
  if (matches.length <= 1) return matches[0] ?? null;
  for (const candidate of matches) {
    if (await candidate.isFocused().catch(() => false)) return candidate;
  }
  return matches[matches.length - 1] ?? null;
}

function recordDragSummary(session: NativeWorkbenchDragSession): void {
  const durationMs = Math.max(1, performance.now() - session.startedAt);
  recordWorkbenchWindowMetric("externalized-drag-summary", {
    startedAt: Date.now() - durationMs,
    detail: {
      dragOverCount: session.dragOverCount,
      averageDecisionMs: session.dragOverCount > 0
        ? Math.round((session.dragOverTotalMs / session.dragOverCount) * 100) / 100
        : 0,
      maxDecisionMs: Math.round(session.dragOverMaxMs * 100) / 100,
      durationMs: Math.round(durationMs * 100) / 100,
      dropped: session.dropped,
      cancelled: session.cancelled,
    },
  });
}

export function useWorkbenchWindowTabDrag(options: {
  windowLabel: string;
  ownerWindow?: Window;
  resolveClientPoint?: (x: number, y: number) => WorkbenchWindowDropIntent | null;
}): WorkbenchWindowTabDragController {
  const ownerWindow = options.ownerWindow ?? window;
  const state: ControllerState = {
    dropTarget: ref<WorkbenchWindowDropIntent | null>(null),
    disposed: false,
    windowLabel: options.windowLabel,
    ownerWindow,
    resolveClientPoint: options.resolveClientPoint,
  };
  controllerStates.add(state);

  function externalize(
    item: WorkbenchWindowTabDragItem,
    previewAnchor: { x: number; y: number },
  ): void {
    if (state.disposed) return;
    if (activeSession) {
      void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
      void stopLocusDragPreview().catch(() => undefined);
      clearSession(activeSession);
    }
    const handleEscape = (keyboardEvent: KeyboardEvent) => {
      const session = activeSession;
      if (keyboardEvent.key !== "Escape" || session?.item !== item) return;
      session.cancelled = true;
      void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
      void stopLocusDragPreview().catch(() => undefined);
      clearSession(session);
    };
    ownerWindow.addEventListener("keydown", handleEscape, true);
    activeSession = {
      item,
      source: state,
      startedAt: performance.now(),
      cancelled: false,
      dropped: false,
      observedPressed: true,
      finishing: false,
      dragOverCount: 0,
      dragOverTotalMs: 0,
      dragOverMaxMs: 0,
      removeEscapeListener: () => ownerWindow.removeEventListener("keydown", handleEscape, true),
    };
    ensureNativePointListener();
    void startLocusDragPreview(item.title, previewAnchor, ownerWindow.document)
      .catch(() => undefined);
    const session = activeSession;
    void invokeLocusRuntime("start_shared_workbench_drag_tracking").catch(() => {
      if (activeSession !== session) return;
      void stopLocusDragPreview().catch(() => undefined);
      clearSession(session);
    });
    recordWorkbenchWindowMetric("internal-drag-externalized", {
      startedAt: Date.now(),
      detail: {
        editorId: item.id,
        sourceWindowId: item.sourceWindowId,
        sourcePaneId: item.sourcePaneId,
      },
    });
  }

  function dispose(): void {
    state.disposed = true;
    controllerStates.delete(state);
    if (activeSession?.source === state) {
      void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
      void stopLocusDragPreview().catch(() => undefined);
      clearSession(activeSession);
    }
    state.dropTarget.value = null;
  }

  return {
    dropTarget: state.dropTarget,
    externalize,
    dispose,
  };
}
