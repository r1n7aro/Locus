import {
  cancelUnityEmbedAssetDrag,
  startLocusNativeFileDrag,
  setUnityEmbedDragPassthrough,
  startUnityEmbedAssetDrag,
  startUnityNativeAssetFileDrag,
  type LocusFileDropRef,
} from "../services/unity";
import type { AssetRefAttachment } from "../types";

// Crossing the threshold mid-click dispatches a native OS drag that outlives
// the gesture, so both gates below must hold before a drag starts:
// - distance, well above aim jitter while clicking;
// - hold time, because a click-and-go gesture (release while flicking the
//   mouse away) can travel a long distance yet always releases quickly.
const POINTER_DRAG_THRESHOLD_PX = 12;
const POINTER_DRAG_MIN_HOLD_MS = 120;
const DRAG_PASSTHROUGH_RESET_MS = 12000;
const NATIVE_FILE_DRAG_RESTORE_MS = 12000;
const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;

let passthroughResetTimer: number | null = null;

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

function startUnityAssetDragWarmup(refs: AssetRefAttachment[]): Promise<boolean> | null {
  if (!shouldWarmupUnityDrag(refs)) return null;

  return startUnityEmbedAssetDrag(refs)
    .then(() => true)
    .catch((error) => {
      console.warn("[Locus] Failed to arm Unity asset drag", error);
      return false;
    });
}

async function beginUnityReferencePointerDrag(refs: AssetRefAttachment[]) {
  const armPromise = startUnityAssetDragWarmup(refs);
  if (!armPromise) return;

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

async function beginNativeAssetFileDrag(refs: AssetRefAttachment[]) {
  const shouldResetPassthrough = isUnityEmbedWindow();
  const armPromise = startUnityAssetDragWarmup(refs) ?? Promise.resolve(false);
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

interface ArmedPointerDragSession {
  pointerId: number;
  disarm: () => void;
}

// Only one pointer-drag gesture can be armed at a time. Chat chips arm twice
// for the same pointerdown (the hydrated identity component and the markdown
// host both observe it); without this guard each arm later dispatches its own
// native drag, and the duplicate OS drag loop enters after the button is
// already up, leaving the drag ghost stuck to the cursor.
let armedPointerDragSession: ArmedPointerDragSession | null = null;

function isPointerDragArmed(event: PointerEvent): boolean {
  return armedPointerDragSession?.pointerId === event.pointerId;
}

interface PointerDragArmOptions {
  suppressHtmlDrag: boolean;
  onDragStart: () => Promise<unknown>;
}

function armPointerDrag(event: PointerEvent, options: PointerDragArmOptions) {
  armedPointerDragSession?.disarm();

  const restoreHtmlDraggable = options.suppressHtmlDrag
    ? suppressHtmlDraggable(event)
    : null;
  const pointerId = event.pointerId;
  const startX = event.clientX;
  const startY = event.clientY;
  const armedAt = performance.now();
  let started = false;
  let restored = false;

  const restoreHtmlDraggableOnce = () => {
    if (restored) return;
    restored = true;
    restoreHtmlDraggable?.();
  };

  const cleanup = (restoreDraggable = true) => {
    if (armedPointerDragSession === session) {
      armedPointerDragSession = null;
    }
    window.removeEventListener("pointermove", handlePointerMove, true);
    window.removeEventListener("pointerup", handlePointerEnd, true);
    window.removeEventListener("pointercancel", handlePointerEnd, true);
    if (restoreDraggable) {
      restoreHtmlDraggableOnce();
    }
  };

  const handlePointerMove = (moveEvent: PointerEvent) => {
    if (started || moveEvent.pointerId !== pointerId) return;
    // The primary button is no longer down, so the pointerup never reached
    // this listener (or raced ahead of the move). Starting now would enter
    // the OS drag loop with the button already released, where no release
    // transition can ever end it.
    if ((moveEvent.buttons & 1) === 0) {
      cleanup();
      return;
    }
    const dx = moveEvent.clientX - startX;
    const dy = moveEvent.clientY - startY;
    if (Math.hypot(dx, dy) < POINTER_DRAG_THRESHOLD_PX) return;
    // Not a disarm: keep waiting — a held pointer becomes a drag on the next
    // move once the hold gate passes, while a click releases first.
    if (performance.now() - armedAt < POINTER_DRAG_MIN_HOLD_MS) return;

    started = true;
    moveEvent.preventDefault();
    moveEvent.stopPropagation();
    cleanup(false);
    const restoreTimer = restoreHtmlDraggable
      ? window.setTimeout(restoreHtmlDraggableOnce, NATIVE_FILE_DRAG_RESTORE_MS)
      : null;
    void options.onDragStart().finally(() => {
      if (restoreTimer !== null) {
        window.clearTimeout(restoreTimer);
      }
      restoreHtmlDraggableOnce();
    });
  };

  const handlePointerEnd = (endEvent: PointerEvent) => {
    if (endEvent.pointerId !== pointerId) return;
    cleanup();
  };

  const session: ArmedPointerDragSession = { pointerId, disarm: cleanup };
  armedPointerDragSession = session;
  window.addEventListener("pointermove", handlePointerMove, true);
  window.addEventListener("pointerup", handlePointerEnd, true);
  window.addEventListener("pointercancel", handlePointerEnd, true);
}

export function armUnityReferencePointerDrag(event: PointerEvent, refs: AssetRefAttachment[]) {
  if (refs.length === 0 || event.button !== 0) return;
  // Keep the first arm of this gesture; a second arm for the same pointer
  // must not create another drag dispatch.
  if (isPointerDragArmed(event)) return;

  const useNativeFileDrag = shouldStartNativeAssetFileDrag(refs);
  // The Unity drag warmup is created lazily by the begin* paths once the drag
  // threshold passes: warming up on pointerdown would flash the native drag
  // preview on plain clicks (and orphan it when Unity is disconnected).
  armPointerDrag(event, {
    suppressHtmlDrag: useNativeFileDrag || (isUnityEmbedWindow() && shouldWarmupUnityDrag(refs)),
    onDragStart: () => useNativeFileDrag
      ? beginNativeAssetFileDrag(refs)
      : beginUnityReferencePointerDrag(refs),
  });
}

export function armLocusFilePointerDrag(event: PointerEvent, files: LocusFileDropRef[]) {
  if (files.length === 0 || event.button !== 0) return;
  if (isPointerDragArmed(event)) return;

  armPointerDrag(event, {
    suppressHtmlDrag: true,
    onDragStart: () => beginNativeFileDrag(files),
  });
}
