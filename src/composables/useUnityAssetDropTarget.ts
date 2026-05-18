import { onMounted, onUnmounted, ref } from "vue";
import {
  commitUnityEmbedAssetDrop,
  subscribeUnityEmbedAssetDragState,
  type UnityEmbedAssetDragStatePayload,
} from "../services/unity";
import type { AssetRefAttachment } from "../types";

const UNITY_ASSET_DRAG_STATE_TTL_MS = 1200;

interface UnityAssetDropTargetOptions {
  enabled?: () => boolean;
  warnPrefix?: string;
}

export function useUnityAssetDropTarget(options: UnityAssetDropTargetOptions = {}) {
  const unityAssetDragRefs = ref<AssetRefAttachment[]>([]);

  let releaseUnityAssetDragState: (() => void) | null = null;
  let unityAssetDragStateSubscriptionDisposed = false;
  let unityAssetDragStateClearTimer = 0;
  let assetDropCommitInFlight = false;
  let assetDropCommitErrorLogged = false;
  let assetDragStateSubscriptionErrorLogged = false;

  const isEnabled = () => options.enabled?.() ?? true;
  const warnPrefix = options.warnPrefix ?? "[Locus]";

  function clearUnityAssetDragStateTimer() {
    if (!unityAssetDragStateClearTimer) return;
    window.clearTimeout(unityAssetDragStateClearTimer);
    unityAssetDragStateClearTimer = 0;
  }

  function clearUnityAssetDragState() {
    clearUnityAssetDragStateTimer();
    unityAssetDragRefs.value = [];
  }

  function hasUnityAssetDragState(): boolean {
    return unityAssetDragRefs.value.length > 0;
  }

  function scheduleUnityAssetDragStateExpiry() {
    clearUnityAssetDragStateTimer();
    unityAssetDragStateClearTimer = window.setTimeout(() => {
      unityAssetDragStateClearTimer = 0;
      unityAssetDragRefs.value = [];
    }, UNITY_ASSET_DRAG_STATE_TTL_MS);
  }

  function handleUnityAssetDragState(payload: UnityEmbedAssetDragStatePayload) {
    if (!isEnabled()) return;
    const refs = Array.isArray(payload.refs) ? payload.refs : [];
    if (!payload.hasRefs || refs.length === 0) {
      clearUnityAssetDragState();
      return;
    }

    unityAssetDragRefs.value = refs;
    scheduleUnityAssetDragStateExpiry();
  }

  function isUnityExternalFileDrag(event: DragEvent): boolean {
    const types = event.dataTransfer ? Array.from(event.dataTransfer.types) : [];
    return types.includes("Files");
  }

  function acceptUnityAssetDrag(event: DragEvent): boolean {
    if (!isEnabled()) return false;
    if (!isUnityExternalFileDrag(event) && !hasUnityAssetDragState()) return false;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "copy";
    }
    return true;
  }

  function handleUnityAssetDrag(event: DragEvent) {
    acceptUnityAssetDrag(event);
  }

  function handleUnityAssetDrop(event: DragEvent) {
    if (!acceptUnityAssetDrag(event) || assetDropCommitInFlight) return;
    assetDropCommitInFlight = true;
    commitUnityEmbedAssetDrop()
      .then(() => {
        clearUnityAssetDragState();
      })
      .catch((error: unknown) => {
        if (assetDropCommitErrorLogged) return;
        assetDropCommitErrorLogged = true;
        console.warn(`${warnPrefix} failed to commit Unity asset drop:`, error);
      })
      .finally(() => {
        assetDropCommitInFlight = false;
      });
  }

  onMounted(() => {
    if (!isEnabled()) return;
    unityAssetDragStateSubscriptionDisposed = false;
    subscribeUnityEmbedAssetDragState(handleUnityAssetDragState)
      .then((release) => {
        if (unityAssetDragStateSubscriptionDisposed) {
          release();
          return;
        }
        releaseUnityAssetDragState = release;
      })
      .catch((error: unknown) => {
        if (assetDragStateSubscriptionErrorLogged) return;
        assetDragStateSubscriptionErrorLogged = true;
        console.warn(`${warnPrefix} Unity asset drag state subscription failed:`, error);
      });
  });

  onUnmounted(() => {
    unityAssetDragStateSubscriptionDisposed = true;
    releaseUnityAssetDragState?.();
    releaseUnityAssetDragState = null;
    clearUnityAssetDragState();
  });

  return {
    handleUnityAssetDrag,
    handleUnityAssetDrop,
    hasUnityAssetDragState,
  };
}
