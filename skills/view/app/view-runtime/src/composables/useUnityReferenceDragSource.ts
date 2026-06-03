import {
  cancelUnityEmbedAssetDrag,
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
const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;

let passthroughResetTimer: number | null = null;

interface UnityReferenceDragWarmup {
  promise: Promise<boolean>;
  committed: boolean;
}

function normalizeUnityReferencePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function isUnityAssetRefPath(path: string): boolean {
  return UNITY_ASSET_REF_ROOT_RE.test(normalizeUnityReferencePath(path));
}

function isUnityDragWarmupRef(ref: AssetRefAttachment): boolean {
  if (ref.kind === "sceneObject") return true;
  if (ref.kind !== "asset") return false;
  return isUnityAssetRefPath(ref.path);
}

function shouldWarmupUnityDrag(refs: AssetRefAttachment[]): boolean {
  return refs.length > 0 && refs.every(isUnityDragWarmupRef);
}

function scheduleDragPassthroughReset() {
  if (passthroughResetTimer !== null) {
    window.clearTimeout(passthroughResetTimer);
  }
  passthroughResetTimer = window.setTimeout(() => {
    passthroughResetTimer = null;
    void setUnityEmbedDragPassthrough(false);
  }, DRAG_PASSTHROUGH_RESET_MS);
}

function startUnityAssetDragWarmup(refs: AssetRefAttachment[]): UnityReferenceDragWarmup | null {
  if (!shouldWarmupUnityDrag(refs)) return null;

  const promise = startUnityEmbedAssetDrag(refs)
    .then(() => true)
    .catch((error) => {
      console.warn("[Locus] Failed to arm Unity asset drag", error);
      return false;
    });

  return {
    promise,
    committed: false,
  };
}

function cancelUnityAssetDragWarmup(warmup: UnityReferenceDragWarmup | null) {
  if (!warmup || warmup.committed) return;

  void warmup.promise.then((armed) => {
    if (!armed || warmup.committed) return;
    void cancelUnityEmbedAssetDrag().catch((error) => {
      console.warn("[Locus] Failed to cancel Unity reference drag", error);
    });
  });
}

async function beginUnityReferencePointerDrag(
  refs: AssetRefAttachment[],
  warmup?: UnityReferenceDragWarmup | null,
) {
  const activeWarmup = warmup ?? startUnityAssetDragWarmup(refs);
  if (!activeWarmup) return;
  activeWarmup.committed = true;

  const armPromise = activeWarmup.promise;
  const passthroughPromise = setUnityEmbedDragPassthrough(true)
    .then(() => true)
    .catch((error) => {
      console.warn("[Locus] Failed to enable Unity drag passthrough", error);
      return false;
    });

  const [armed, passthroughEnabled] = await Promise.all([armPromise, passthroughPromise]);
  if (armed && passthroughEnabled) {
    scheduleDragPassthroughReset();
    return;
  }

  if (armed) {
    void cancelUnityEmbedAssetDrag().catch((error) => {
      console.warn("[Locus] Failed to cancel Unity reference drag", error);
    });
  }
  void setUnityEmbedDragPassthrough(false);
}

async function beginNativeAssetFileDrag(
  refs: AssetRefAttachment[],
  warmup?: UnityReferenceDragWarmup | null,
) {
  const shouldResetPassthrough = isUnityEmbedWindow();
  const activeWarmup = warmup ?? startUnityAssetDragWarmup(refs);
  if (activeWarmup) {
    activeWarmup.committed = true;
  }

  const armPromise = activeWarmup?.promise ?? Promise.resolve(false);
  let armed = false;
  try {
    if (shouldResetPassthrough) {
      await setUnityEmbedDragPassthrough(true);
    }
    const nativeDragPromise = startUnityNativeAssetFileDrag(refs);
    const [armResult, nativeDragResult] = await Promise.allSettled([armPromise, nativeDragPromise]);
    armed = armResult.status === "fulfilled" && armResult.value;
    if (nativeDragResult.status === "rejected") {
      throw nativeDragResult.reason;
    }
  } catch (error) {
    console.warn("[Locus] Failed to start native asset file drag", error);
  } finally {
    if (armed) {
      void cancelUnityEmbedAssetDrag().catch((error) => {
        console.warn("[Locus] Failed to cancel Unity reference drag", error);
      });
    }
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
  return refs.every((ref) => ref.kind === "asset" && isUnityAssetRefPath(ref.path));
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
  const warmup = shouldWarmupUnityDrag(refs) ? startUnityAssetDragWarmup(refs) : null;
  const shouldSuppressHtmlDrag = useNativeFileDrag || (isUnityEmbedWindow() && !!warmup);
  const restoreHtmlDraggable = shouldSuppressHtmlDrag
    ? suppressHtmlDraggable(event)
    : null;
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
      ? beginNativeAssetFileDrag(refs, warmup)
      : beginUnityReferencePointerDrag(refs, warmup);
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
    if (!started) {
      cancelUnityAssetDragWarmup(warmup);
    }
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
