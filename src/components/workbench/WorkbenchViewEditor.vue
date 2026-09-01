<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { Window as TauriWindowHandle } from "@tauri-apps/api/window";
import type { WorkspaceRef } from "../../services/project";
import { viewContentHide, viewContentMount } from "../../services/view";
import { normalizeAppError } from "../../services/errors";

const props = defineProps<{
  viewId: string;
  workspaceRef: WorkspaceRef | null;
  active: boolean;
  nativeWindow: TauriWindowHandle | null;
  ownerWindow: Window;
}>();

const root = ref<HTMLElement | null>(null);
const error = ref("");
let resizeObserver: ResizeObserver | null = null;
let unlistenMoved: (() => void) | null = null;
let unlistenResized: (() => void) | null = null;
let syncTimer = 0;
let syncPromise: Promise<void> | null = null;
let syncQueued = false;
let relinquished = false;
let lastGeometry = "";

async function hide(): Promise<void> {
  if (!props.workspaceRef || !props.viewId) return;
  await viewContentHide(props.workspaceRef, props.viewId).catch(() => undefined);
}

async function mount(): Promise<void> {
  if (syncPromise) {
    syncQueued = true;
    return syncPromise;
  }
  syncPromise = (async () => {
    if (relinquished) return;
    const element = root.value;
    const appWindow = props.nativeWindow;
    const workspaceRef = props.workspaceRef;
    if (!props.active || !element || !appWindow || !workspaceRef) {
      await hide();
      return;
    }
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      await hide();
      return;
    }
    const [position, scaleFactor] = await Promise.all([
      appWindow.outerPosition(),
      appWindow.scaleFactor().catch(() => props.ownerWindow.devicePixelRatio || 1),
    ]);
    const scale = Number.isFinite(scaleFactor) && scaleFactor > 0 ? scaleFactor : 1;
    const request = {
      viewId: props.viewId,
      hostLabel: appWindow.label,
      x: Math.round(position.x + rect.left * scale),
      y: Math.round(position.y + rect.top * scale),
      width: Math.max(1, Math.round(rect.width * scale)),
      height: Math.max(1, Math.round(rect.height * scale)),
      visible: true,
    };
    const geometry = JSON.stringify(request);
    if (geometry === lastGeometry) return;
    await viewContentMount(workspaceRef, request);
    lastGeometry = geometry;
    error.value = "";
  })().catch((mountError) => {
    error.value = normalizeAppError(mountError).message;
    throw mountError;
  }).finally(() => {
    syncPromise = null;
    if (syncQueued) {
      syncQueued = false;
      scheduleSync();
    }
  });
  return syncPromise;
}

function scheduleSync(): void {
  if (syncTimer) props.ownerWindow.clearTimeout(syncTimer);
  syncTimer = props.ownerWindow.setTimeout(() => {
    syncTimer = 0;
    void mount().catch(() => undefined);
  }, 16);
}

async function ensureMounted(): Promise<void> {
  relinquished = false;
  lastGeometry = "";
  await nextTick();
  await mount();
}

function relinquish(): void {
  relinquished = true;
}

defineExpose({ ensureMounted, relinquish });

watch(
  () => [props.active, props.viewId, props.workspaceRef?.checkoutId, props.workspaceRef?.expectedGeneration],
  () => {
    lastGeometry = "";
    scheduleSync();
  },
);

onMounted(async () => {
  if (root.value && typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(scheduleSync);
    resizeObserver.observe(root.value);
  }
  props.ownerWindow.addEventListener("resize", scheduleSync);
  if (props.nativeWindow) {
    unlistenMoved = await props.nativeWindow.onMoved(scheduleSync);
    unlistenResized = await props.nativeWindow.onResized(scheduleSync);
  }
  scheduleSync();
});

onUnmounted(() => {
  if (syncTimer) props.ownerWindow.clearTimeout(syncTimer);
  syncTimer = 0;
  resizeObserver?.disconnect();
  resizeObserver = null;
  props.ownerWindow.removeEventListener("resize", scheduleSync);
  unlistenMoved?.();
  unlistenMoved = null;
  unlistenResized?.();
  unlistenResized = null;
  if (!relinquished) void hide();
});
</script>

<template>
  <div ref="root" class="workbench-view-editor">
    <div v-if="error" class="workbench-view-editor-error">{{ error }}</div>
  </div>
</template>

<style scoped>
.workbench-view-editor {
  position: relative;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--panel-bg);
}

.workbench-view-editor-error {
  display: grid;
  width: 100%;
  height: 100%;
  place-items: center;
  padding: 16px;
  color: var(--status-error-fg);
  font-size: 12px;
  text-align: center;
}
</style>
