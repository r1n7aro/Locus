import {
  startLocusNativeFileDrag,
  setUnityEmbedDragPassthrough,
  startUnityEmbedAssetDrag,
  startUnityNativeAssetFileDrag,
  type LocusFileDropRef,
} from "../services/unity";
import type { AssetRefAttachment } from "../types";

const POINTER_DRAG_THRESHOLD_PX = 4;
const DRAG_PASSTHROUGH_RESET_MS = 12000;
const NATIVE_FILE_DRAG_RESTORE_MS = 12000;

let passthroughResetTimer: number | null = null;

function scheduleDragPassthroughReset() {
  if (passthroughResetTimer !== null) {
    window.clearTimeout(passthroughResetTimer);
  }
  passthroughResetTimer = window.setTimeout(() => {
    passthroughResetTimer = null;
    void setUnityEmbedDragPassthrough(false);
  }, DRAG_PASSTHROUGH_RESET_MS);
}

async function beginUnityReferencePointerDrag(refs: AssetRefAttachment[]) {
  try {
    await startUnityEmbedAssetDrag(refs);
    await setUnityEmbedDragPassthrough(true);
    scheduleDragPassthroughReset();
  } catch (error) {
    console.warn("[Locus] Failed to start Unity reference drag", error);
    void setUnityEmbedDragPassthrough(false);
  }
}

async function beginNativeAssetFileDrag(refs: AssetRefAttachment[]) {
  const shouldResetPassthrough = isUnityEmbedWindow();
  try {
    if (shouldResetPassthrough) {
      await setUnityEmbedDragPassthrough(true);
    }
    await startUnityEmbedAssetDrag(refs).catch((error) => {
      console.warn("[Locus] Failed to arm Unity asset drag", error);
    });
    await startUnityNativeAssetFileDrag(refs);
  } catch (error) {
    console.warn("[Locus] Failed to start native asset file drag", error);
  } finally {
    if (shouldResetPassthrough) {
      void setUnityEmbedDragPassthrough(false);
    }
  }
}

async function beginNativeFileDrag(files: LocusFileDropRef[]) {
  const shouldResetPassthrough = isUnityEmbedWindow();
  try {
    if (shouldResetPassthrough) {
      await setUnityEmbedDragPassthrough(true);
    }
    await startLocusNativeFileDrag(files);
  } catch (error) {
    console.warn("[Locus] Failed to start native file drag", error);
  } finally {
    if (shouldResetPassthrough) {
      void setUnityEmbedDragPassthrough(false);
    }
  }
}

export function startUnityReferenceHtmlDrag(event: DragEvent, refs: AssetRefAttachment[]) {
  if (refs.length === 0) return;
  event.preventDefault();
  event.stopPropagation();

  if (shouldStartNativeAssetFileDrag(refs)) {
    return;
  }

  void beginUnityReferencePointerDrag(refs);
}

export function startLocusFileHtmlDrag(event: DragEvent, files: LocusFileDropRef[]) {
  if (files.length === 0) return;
  event.preventDefault();
  event.stopPropagation();
}

function isUnityEmbedWindow(): boolean {
  return window.location.pathname === "/unity-embed";
}

function canStartNativeAssetFileDrag(refs: AssetRefAttachment[]): boolean {
  return refs.every((ref) => ref.kind === "asset");
}

function shouldStartNativeAssetFileDrag(refs: AssetRefAttachment[]): boolean {
  return !isUnityEmbedWindow() && canStartNativeAssetFileDrag(refs);
}

function suppressHtmlDraggable(event: PointerEvent): (() => void) | null {
  const target = event.target;
  if (!(target instanceof Element)) return null;
  const draggable = target.closest('[draggable="true"]') as HTMLElement | null;
  if (!draggable) return null;

  const previous = draggable.getAttribute("draggable");
  draggable.setAttribute("draggable", "false");
  return () => {
    if (previous === null) {
      draggable.removeAttribute("draggable");
    } else {
      draggable.setAttribute("draggable", previous);
    }
  };
}

export function armUnityReferencePointerDrag(event: PointerEvent, refs: AssetRefAttachment[]) {
  if (refs.length === 0 || event.button !== 0) return;

  const useNativeFileDrag = shouldStartNativeAssetFileDrag(refs);
  const restoreHtmlDraggable = useNativeFileDrag ? suppressHtmlDraggable(event) : null;
  const startX = event.clientX;
  const startY = event.clientY;
  let started = false;
  let restored = false;

  const restoreHtmlDraggableOnce = () => {
    if (restored) return;
    restored = true;
    restoreHtmlDraggable?.();
  };

  const cleanup = (restoreDraggable = true) => {
    window.removeEventListener("pointermove", handlePointerMove, true);
    window.removeEventListener("pointerup", handlePointerEnd, true);
    window.removeEventListener("pointercancel", handlePointerEnd, true);
    if (restoreDraggable) {
      restoreHtmlDraggableOnce();
    }
  };

  const handlePointerMove = (moveEvent: PointerEvent) => {
    if (started) return;
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    if (Math.hypot(dx, dy) < POINTER_DRAG_THRESHOLD_PX) return;

    started = true;
    moveEvent.preventDefault();
    moveEvent.stopPropagation();
    cleanup(false);
    const drag = useNativeFileDrag
      ? beginNativeAssetFileDrag(refs)
      : beginUnityReferencePointerDrag(refs);
    const restoreTimer = restoreHtmlDraggable
      ? window.setTimeout(restoreHtmlDraggableOnce, NATIVE_FILE_DRAG_RESTORE_MS)
      : null;
    void drag.finally(() => {
      if (restoreTimer !== null) {
        window.clearTimeout(restoreTimer);
      }
      restoreHtmlDraggableOnce();
    });
  };

  const handlePointerEnd = () => {
    cleanup();
  };

  window.addEventListener("pointermove", handlePointerMove, true);
  window.addEventListener("pointerup", handlePointerEnd, true);
  window.addEventListener("pointercancel", handlePointerEnd, true);
}

export function armLocusFilePointerDrag(event: PointerEvent, files: LocusFileDropRef[]) {
  if (files.length === 0 || event.button !== 0) return;

  const restoreHtmlDraggable = suppressHtmlDraggable(event);
  const startX = event.clientX;
  const startY = event.clientY;
  let started = false;
  let restored = false;

  const restoreHtmlDraggableOnce = () => {
    if (restored) return;
    restored = true;
    restoreHtmlDraggable?.();
  };

  const cleanup = (restoreDraggable = true) => {
    window.removeEventListener("pointermove", handlePointerMove, true);
    window.removeEventListener("pointerup", handlePointerEnd, true);
    window.removeEventListener("pointercancel", handlePointerEnd, true);
    if (restoreDraggable) {
      restoreHtmlDraggableOnce();
    }
  };

  const handlePointerMove = (moveEvent: PointerEvent) => {
    if (started) return;
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    if (Math.hypot(dx, dy) < POINTER_DRAG_THRESHOLD_PX) return;

    started = true;
    moveEvent.preventDefault();
    moveEvent.stopPropagation();
    cleanup(false);
    const drag = beginNativeFileDrag(files);
    const restoreTimer = restoreHtmlDraggable
      ? window.setTimeout(restoreHtmlDraggableOnce, NATIVE_FILE_DRAG_RESTORE_MS)
      : null;
    void drag.finally(() => {
      if (restoreTimer !== null) {
        window.clearTimeout(restoreTimer);
      }
      restoreHtmlDraggableOnce();
    });
  };

  const handlePointerEnd = () => {
    cleanup();
  };

  window.addEventListener("pointermove", handlePointerMove, true);
  window.addEventListener("pointerup", handlePointerEnd, true);
  window.addEventListener("pointercancel", handlePointerEnd, true);
}
