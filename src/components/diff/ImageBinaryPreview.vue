<script setup lang="ts">
import { ref, computed, onBeforeUnmount } from "vue";
import { refetchDiffByKey } from "../../services/diff";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import type { BinaryPreview } from "../../types";

const props = defineProps<{
  preview: BinaryPreview;
  compact?: boolean;
  diffKey: string;
  mode?: "diff" | "neutral";
}>();

const activeSide = ref<"before" | "after">(props.preview.after ? "after" : "before");
const loadError = ref(false);
const scale = ref(1);
const panX = ref(0);
const panY = ref(0);
let dragging = false;
let lastX = 0;
let lastY = 0;
let releaseSelectionLock: (() => void) | null = null;

const activeRef = computed(() =>
  activeSide.value === "before" ? props.preview.before : props.preview.after,
);

const hasBoth = computed(() => !!props.preview.before && !!props.preview.after);
const statusLabel = computed(() => {
  if (props.mode === "neutral") return null;
  if (hasBoth.value) return null;
  return props.preview.after ? "Added" : "Deleted";
});

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

async function onImgError() {
  if (!loadError.value) {
    loadError.value = true;
    await refetchDiffByKey(props.diffKey);
  }
}

function onWheel(e: WheelEvent) {
  if (props.compact) return;
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  scale.value = Math.max(0.1, Math.min(10, scale.value * factor));
}

function onMouseDown(e: MouseEvent) {
  if (props.compact) return;
  dragging = true;
  lastX = e.clientX;
  lastY = e.clientY;
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
}

function onMouseMove(e: MouseEvent) {
  if (!dragging) return;
  panX.value += e.clientX - lastX;
  panY.value += e.clientY - lastY;
  lastX = e.clientX;
  lastY = e.clientY;
}

function onMouseUp() {
  dragging = false;
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

function resetView() {
  scale.value = 1;
  panX.value = 0;
  panY.value = 0;
}

onBeforeUnmount(() => {
  releaseSelectionLock?.();
  releaseSelectionLock = null;
});
</script>

<template>
  <div class="image-preview" :class="{ compact }">
    <!-- Controls -->
    <div v-if="!compact" class="preview-controls">
      <div v-if="hasBoth" class="side-toggle">
        <button :class="{ active: activeSide === 'before' }" @click="activeSide = 'before'">Before</button>
        <button :class="{ active: activeSide === 'after' }" @click="activeSide = 'after'">After</button>
      </div>
      <span v-if="statusLabel" class="status-badge">{{ statusLabel }}</span>
      <span v-if="activeRef" class="size-label">{{ formatSize(activeRef.byteSize) }}</span>
      <button class="reset-btn" @click="resetView" title="Reset zoom">1:1</button>
    </div>

    <!-- Image display -->
    <div
      v-if="activeRef && !loadError"
      class="image-container"
      @wheel="onWheel"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @mouseup="onMouseUp"
      @mouseleave="onMouseUp"
      @dblclick="resetView"
    >
      <img
        :src="activeRef.url"
        :style="{
          transform: compact ? 'none' : `translate(${panX}px, ${panY}px) scale(${scale})`,
        }"
        draggable="false"
        @error="onImgError"
      />
    </div>
    <div v-else class="preview-fallback">Binary file, no preview available</div>
  </div>
</template>

<style scoped>
.image-preview {
  display: flex;
  flex-direction: column;
  width: 100%;
}
.image-preview.compact {
  max-height: 200px;
}
.preview-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}
.side-toggle {
  display: flex;
  gap: 0;
}
.side-toggle button {
  padding: 2px 10px;
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
}
.side-toggle button:first-child {
  border-radius: 4px 0 0 4px;
}
.side-toggle button:last-child {
  border-radius: 0 4px 4px 0;
  border-left: none;
}
.side-toggle button.active {
  background: var(--accent);
  color: var(--text-on-accent, #fff);
  border-color: var(--accent);
}
.status-badge {
  padding: 1px 6px;
  border-radius: 3px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font-size: 11px;
}
.size-label {
  color: var(--text-secondary);
  margin-left: auto;
}
.reset-btn {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
}
.image-container {
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 120px;
  background: var(--bg-primary);
  cursor: grab;
}
.image-container:active {
  cursor: grabbing;
}
.compact .image-container {
  cursor: default;
  max-height: 200px;
}
.image-container img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  transform-origin: center center;
  user-select: none;
}
.compact .image-container img {
  max-height: 200px;
}
.preview-fallback {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
}
</style>
