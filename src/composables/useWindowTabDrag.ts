import { ref, type Ref } from "vue";
import { emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import {
  cursorPosition,
  Window as TauriWindow,
  type Window as TauriWindowHandle,
} from "@tauri-apps/api/window";
import type { InternalDragFinishResult } from "./useInternalDrag";
import { startLocusDragPreview, stopLocusDragPreview } from "../services/unity";

export const WINDOW_TAB_DROP_TARGET_EVENT = "locus-window-tab-drop-target";

const WINDOW_TAB_DRAG_FRAME_MS = 16;

export interface WindowTabScreenPoint {
  x: number;
  y: number;
}

export interface WindowTabDragItem {
  id: string;
  title: string;
  canDetach: () => boolean;
  transfer: (targetWindowLabel: string) => void | Promise<void>;
  detach: (point: WindowTabScreenPoint) => void | Promise<void>;
}

export interface WindowTabDropTargetPayload {
  family: string;
  sourceLabel: string;
  active: boolean;
}

export interface WindowTabDragOptions {
  family: string;
  appWindow: TauriWindowHandle | null;
  windowLabel: string;
  acceptsWindowLabel: (label: string) => boolean;
  tabBandHeight?: number;
  prepare?: () => void | Promise<void>;
}

export interface WindowTabDragController {
  draggingTabId: Ref<string>;
  dropTargetLabel: Ref<string>;
  externalDropActive: Ref<boolean>;
  begin: (item: WindowTabDragItem) => void;
  finish: (tabId: string, result: InternalDragFinishResult) => Promise<void>;
  startListening: () => Promise<void>;
  dispose: () => void;
}

/**
 * Bridges the app-level pointer drag controller to native window coordinates.
 * The shared layer decides where a gesture ends; each tab supplies its own
 * transfer and detach implementation.
 */
export function useWindowTabDrag(options: WindowTabDragOptions): WindowTabDragController {
  const draggingTabId = ref("");
  const dropTargetLabel = ref("");
  const externalDropActive = ref(false);
  let activeItem: WindowTabDragItem | null = null;
  let frameTimer: ReturnType<typeof setTimeout> | null = null;
  let unlistenDropTarget: UnlistenFn | null = null;
  let lastEmittedTargetLabel = "";
  let externalSourceLabel = "";
  let nativePreviewActive = false;
  let disposed = false;

  function emitTargetState(targetLabel: string, active: boolean): void {
    if (!options.windowLabel || !targetLabel || targetLabel === options.windowLabel) return;
    void emitTo<WindowTabDropTargetPayload>(targetLabel, WINDOW_TAB_DROP_TARGET_EVENT, {
      family: options.family,
      sourceLabel: options.windowLabel,
      active,
    }).catch((error) => {
      console.warn("[window-tab-drag] failed to update drop target", error);
    });
  }

  function setDropTargetLabel(nextLabel: string): void {
    const normalized = nextLabel || "";
    if (normalized === dropTargetLabel.value) return;
    if (lastEmittedTargetLabel) emitTargetState(lastEmittedTargetLabel, false);
    dropTargetLabel.value = normalized;
    lastEmittedTargetLabel = normalized;
    if (normalized) emitTargetState(normalized, true);
  }

  async function findDropTargetAt(point: WindowTabScreenPoint): Promise<string> {
    if (!options.windowLabel) return "";
    let windows: TauriWindowHandle[] = [];
    try {
      windows = await TauriWindow.getAll();
    } catch {
      return "";
    }
    const tabBandHeight = options.tabBandHeight ?? 40;
    for (const candidate of windows) {
      if (
        candidate.label === options.windowLabel
        || !options.acceptsWindowLabel(candidate.label)
      ) continue;
      try {
        const [position, size] = await Promise.all([
          candidate.outerPosition(),
          candidate.outerSize(),
        ]);
        const withinX = point.x >= position.x && point.x <= position.x + size.width;
        const withinY = point.y >= position.y && point.y <= position.y + size.height;
        const withinTabBand = point.y >= position.y
          && point.y <= position.y + tabBandHeight;
        if (withinX && withinY && withinTabBand) return candidate.label;
      } catch {
        // A window can disappear between enumeration and geometry lookup.
      }
    }
    return "";
  }

  async function isCurrentTabBandAt(point: WindowTabScreenPoint): Promise<boolean> {
    if (!options.appWindow) return false;
    try {
      const [position, size] = await Promise.all([
        options.appWindow.outerPosition(),
        options.appWindow.outerSize(),
      ]);
      const tabBandHeight = options.tabBandHeight ?? 40;
      return point.x >= position.x
        && point.x <= position.x + size.width
        && point.y >= position.y
        && point.y <= position.y + Math.min(tabBandHeight, size.height);
    } catch {
      return false;
    }
  }

  async function isCurrentWindowAt(point: WindowTabScreenPoint): Promise<boolean> {
    if (!options.appWindow) return false;
    try {
      const [position, size] = await Promise.all([
        options.appWindow.outerPosition(),
        options.appWindow.outerSize(),
      ]);
      return point.x >= position.x
        && point.x <= position.x + size.width
        && point.y >= position.y
        && point.y <= position.y + size.height;
    } catch {
      return false;
    }
  }

  function startNativePreview(label: string): void {
    if (nativePreviewActive) return;
    nativePreviewActive = true;
    void startLocusDragPreview(label).catch((error) => {
      nativePreviewActive = false;
      console.warn("[window-tab-drag] failed to start native preview", error);
    });
  }

  function stopNativePreview(): void {
    if (!nativePreviewActive) return;
    nativePreviewActive = false;
    void stopLocusDragPreview().catch((error) => {
      console.warn("[window-tab-drag] failed to stop native preview", error);
    });
  }

  function scheduleFrame(): void {
    if (disposed || !activeItem || frameTimer !== null) return;
    frameTimer = setTimeout(() => {
      frameTimer = null;
      void updateFrame();
    }, WINDOW_TAB_DRAG_FRAME_MS);
  }

  async function updateFrame(): Promise<void> {
    const item = activeItem;
    if (!item || disposed) return;
    try {
      const cursor = await cursorPosition();
      const insideCurrentWindow = await isCurrentWindowAt(cursor);
      if (activeItem !== item || disposed) return;
      if (insideCurrentWindow) stopNativePreview();
      else startNativePreview(item.title);
      const nextTargetLabel = await findDropTargetAt(cursor);
      if (activeItem === item && !disposed) setDropTargetLabel(nextTargetLabel);
    } catch (error) {
      console.warn("[window-tab-drag] failed to inspect native windows", error);
    }
    scheduleFrame();
  }

  function begin(item: WindowTabDragItem): void {
    if (!options.appWindow || disposed) return;
    activeItem = item;
    draggingTabId.value = item.id;
    void Promise.resolve(options.prepare?.()).catch((error) => {
      console.warn("[window-tab-drag] failed to prepare detached host", error);
    });
    scheduleFrame();
  }

  async function finish(tabId: string, result: InternalDragFinishResult): Promise<void> {
    const item = activeItem;
    if (!item || item.id !== tabId) return;
    activeItem = null;
    draggingTabId.value = "";
    if (frameTimer !== null) clearTimeout(frameTimer);
    frameTimer = null;
    stopNativePreview();

    let targetLabel = dropTargetLabel.value;
    setDropTargetLabel("");
    if (result.dropped || result.reason !== "drop") return;

    let releasePoint: WindowTabScreenPoint;
    try {
      releasePoint = await cursorPosition();
      targetLabel = await findDropTargetAt(releasePoint) || targetLabel;
    } catch {
      return;
    }
    if (targetLabel) {
      await item.transfer(targetLabel);
      return;
    }
    if (await isCurrentTabBandAt(releasePoint)) return;
    if (item.canDetach()) await item.detach(releasePoint);
  }

  function applyExternalDropTarget(payload: WindowTabDropTargetPayload): void {
    if (
      payload.family !== options.family
      || !payload.sourceLabel
      || payload.sourceLabel === options.windowLabel
    ) return;
    if (payload.active) {
      externalSourceLabel = payload.sourceLabel;
      externalDropActive.value = true;
      return;
    }
    if (!externalSourceLabel || externalSourceLabel === payload.sourceLabel) {
      externalSourceLabel = "";
      externalDropActive.value = false;
    }
  }

  async function startListening(): Promise<void> {
    if (!options.appWindow || unlistenDropTarget || disposed) return;
    unlistenDropTarget = await options.appWindow.listen<WindowTabDropTargetPayload>(
      WINDOW_TAB_DROP_TARGET_EVENT,
      (event) => applyExternalDropTarget(event.payload),
    );
  }

  function dispose(): void {
    disposed = true;
    activeItem = null;
    draggingTabId.value = "";
    if (frameTimer !== null) clearTimeout(frameTimer);
    frameTimer = null;
    stopNativePreview();
    setDropTargetLabel("");
    externalSourceLabel = "";
    externalDropActive.value = false;
    unlistenDropTarget?.();
    unlistenDropTarget = null;
  }

  return {
    draggingTabId,
    dropTargetLabel,
    externalDropActive,
    begin,
    finish,
    startListening,
    dispose,
  };
}
