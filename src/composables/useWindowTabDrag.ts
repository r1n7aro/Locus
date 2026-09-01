import { ref, type Ref } from "vue";
import { invokeLocusRuntime } from "../services/locusRuntime";
import { emit, emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Window as TauriWindow,
  type Window as TauriWindowHandle,
} from "@tauri-apps/api/window";
import { currentWorkbenchCursorPosition } from "../services/workbenchWindow";

export const WINDOW_TAB_DROP_TARGET_EVENT = "locus-window-tab-drop-target";
export const WINDOW_TAB_NATIVE_MIME = "application/x-locus-window-tab";

const WINDOW_TAB_DRAG_STATE_EVENT = "locus-window-tab-drag-state";
const NATIVE_DRAG_POINT_EVENT = "shared-workbench:drag-point";

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

interface WindowTabDragStatePayload extends WindowTabDropTargetPayload {}

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
  begin: (item: WindowTabDragItem, event: DragEvent) => void;
  markDropped: () => void;
  finish: (tabId: string, event: DragEvent) => Promise<void>;
  startListening: () => Promise<void>;
  dispose: () => void;
}

interface NativeWindowBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
}

export function useWindowTabDrag(options: WindowTabDragOptions): WindowTabDragController {
  const draggingTabId = ref("");
  const dropTargetLabel = ref("");
  const externalDropActive = ref(false);
  let activeItem: WindowTabDragItem | null = null;
  let localDropCommitted = false;
  let externalSourceLabel = "";
  let targetStateEmitted = false;
  let nativeBounds: NativeWindowBounds | null = null;
  let disposed = false;
  const releases: UnlistenFn[] = [];

  async function refreshNativeBounds(): Promise<void> {
    if (!options.appWindow) return;
    const [position, size, scale] = await Promise.all([
      options.appWindow.outerPosition(),
      options.appWindow.outerSize(),
      options.appWindow.scaleFactor(),
    ]);
    nativeBounds = {
      x: position.x,
      y: position.y,
      width: size.width,
      height: size.height,
      scale,
    };
  }

  function pointInsideTabBand(point: WindowTabScreenPoint): boolean {
    const bounds = nativeBounds;
    if (!bounds) return false;
    const bandHeight = (options.tabBandHeight ?? 40) * bounds.scale;
    return point.x >= bounds.x
      && point.x <= bounds.x + bounds.width
      && point.y >= bounds.y
      && point.y <= Math.min(bounds.y + bounds.height, bounds.y + bandHeight);
  }

  function emitTargetState(active: boolean): void {
    if (!externalSourceLabel || targetStateEmitted === active) return;
    targetStateEmitted = active;
    externalDropActive.value = active;
    void emitTo<WindowTabDropTargetPayload>(
      externalSourceLabel,
      WINDOW_TAB_DROP_TARGET_EVENT,
      {
        family: options.family,
        sourceLabel: options.windowLabel,
        active,
      },
    ).catch(() => undefined);
  }

  function clearExternalSource(sourceLabel = externalSourceLabel): void {
    if (sourceLabel && externalSourceLabel !== sourceLabel) return;
    emitTargetState(false);
    externalSourceLabel = "";
    targetStateEmitted = false;
    externalDropActive.value = false;
  }

  function applyDragState(payload: WindowTabDragStatePayload): void {
    if (
      payload.family !== options.family
      || !payload.sourceLabel
      || payload.sourceLabel === options.windowLabel
    ) return;
    if (!payload.active) {
      clearExternalSource(payload.sourceLabel);
      return;
    }
    if (externalSourceLabel && externalSourceLabel !== payload.sourceLabel) clearExternalSource();
    externalSourceLabel = payload.sourceLabel;
  }

  function applyNativePoint(point: WindowTabScreenPoint): void {
    if (!externalSourceLabel) return;
    emitTargetState(pointInsideTabBand(point));
  }

  function applyDropTarget(payload: WindowTabDropTargetPayload): void {
    if (
      payload.family !== options.family
      || !payload.sourceLabel
      || !options.acceptsWindowLabel(payload.sourceLabel)
      || payload.sourceLabel === options.windowLabel
    ) return;
    dropTargetLabel.value = payload.active ? payload.sourceLabel : "";
  }

  async function windowAtPoint(point: WindowTabScreenPoint): Promise<string> {
    const windows = await TauriWindow.getAll().catch(() => []);
    for (const candidate of windows) {
      if (!options.acceptsWindowLabel(candidate.label)) continue;
      try {
        const [visible, minimized, position, size] = await Promise.all([
          candidate.isVisible(),
          candidate.isMinimized(),
          candidate.outerPosition(),
          candidate.outerSize(),
        ]);
        if (!visible || minimized) continue;
        if (
          point.x >= position.x
          && point.x <= position.x + size.width
          && point.y >= position.y
          && point.y <= position.y + size.height
        ) return candidate.label;
      } catch {
        // The window can close while the release point is being resolved.
      }
    }
    return "";
  }

  function publishDragState(active: boolean): void {
    void emit<WindowTabDragStatePayload>(WINDOW_TAB_DRAG_STATE_EVENT, {
      family: options.family,
      sourceLabel: options.windowLabel,
      active,
    }).catch(() => undefined);
  }

  function begin(item: WindowTabDragItem, event: DragEvent): void {
    if (!options.appWindow || disposed || !event.dataTransfer) return;
    activeItem = item;
    localDropCommitted = false;
    draggingTabId.value = item.id;
    dropTargetLabel.value = "";
    event.dataTransfer.setData(WINDOW_TAB_NATIVE_MIME, JSON.stringify({
      family: options.family,
      sourceLabel: options.windowLabel,
      tabId: item.id,
    }));
    void Promise.resolve(options.prepare?.()).catch((error) => {
      console.warn("[window-tab-drag] failed to prepare detached host", error);
    });
    void invokeLocusRuntime("start_shared_workbench_drag_tracking").catch(() => undefined);
    publishDragState(true);
  }

  function markDropped(): void {
    localDropCommitted = true;
    dropTargetLabel.value = "";
  }

  async function finish(tabId: string, _event: DragEvent): Promise<void> {
    const item = activeItem;
    if (!item || item.id !== tabId) return;
    const targetLabel = dropTargetLabel.value;
    activeItem = null;
    draggingTabId.value = "";
    dropTargetLabel.value = "";
    void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
    publishDragState(false);
    const dropped = localDropCommitted;
    localDropCommitted = false;
    if (dropped) return;

    const point = await currentWorkbenchCursorPosition().catch(() => null);
    if (!point) {
      return;
    }
    const releaseWindowLabel = await windowAtPoint(point);
    if (
      targetLabel
      && targetLabel !== options.windowLabel
      && (!releaseWindowLabel || releaseWindowLabel === targetLabel)
    ) {
      await item.transfer(targetLabel);
      return;
    }
    if (releaseWindowLabel) return;
    if (item.canDetach()) await item.detach(point);
  }

  async function startListening(): Promise<void> {
    if (!options.appWindow || releases.length > 0 || disposed) return;
    await refreshNativeBounds();
    releases.push(
      await listen<WindowTabDragStatePayload>(WINDOW_TAB_DRAG_STATE_EVENT, (event) => {
        applyDragState(event.payload);
      }),
      await listen<WindowTabScreenPoint>(NATIVE_DRAG_POINT_EVENT, (event) => {
        applyNativePoint(event.payload);
      }),
      await options.appWindow.listen<WindowTabDropTargetPayload>(
        WINDOW_TAB_DROP_TARGET_EVENT,
        (event) => applyDropTarget(event.payload),
      ),
      await options.appWindow.onMoved(() => void refreshNativeBounds()),
      await options.appWindow.onResized(() => void refreshNativeBounds()),
    );
  }

  function dispose(): void {
    disposed = true;
    if (activeItem) {
      void invokeLocusRuntime("stop_shared_workbench_drag_tracking").catch(() => undefined);
      publishDragState(false);
    }
    activeItem = null;
    draggingTabId.value = "";
    dropTargetLabel.value = "";
    clearExternalSource();
    for (const release of releases.splice(0)) release();
  }

  return {
    draggingTabId,
    dropTargetLabel,
    externalDropActive,
    begin,
    markDropped,
    finish,
    startListening,
    dispose,
  };
}
