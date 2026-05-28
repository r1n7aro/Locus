<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, watch, nextTick, type CSSProperties } from "vue";
import FileDiffViewer from "./FileDiffViewer.vue";
import { clampFloatingPosition } from "../ui/floatingPosition";
import {
  DIFF_POPOVER_WIDTH_PX,
  estimateDiffPopoverHeight,
} from "./fileDiffPopoverLayout";
import type { FileDiffPayload } from "../../types";

const props = defineProps<{
  payload: FileDiffPayload;
  anchor: HTMLElement;
}>();

const popoverRef = ref<HTMLElement | null>(null);
const positionStyle = ref<CSSProperties>({ top: "0px", left: "0px" });
const sizeStyle = computed<CSSProperties>(() => ({
  "--diff-popover-width": `${DIFF_POPOVER_WIDTH_PX}px`,
  "--diff-popover-height": `${estimateDiffPopoverHeight(props.payload)}px`,
} as CSSProperties));

const emit = defineEmits<{
  close: [];
  enter: [];
  leave: [];
  open: [];
}>();

function updatePosition() {
  if (!props.anchor || !popoverRef.value) return;
  const rect = props.anchor.getBoundingClientRect();
  const popRect = popoverRef.value.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const margin = 8;
  const gap = 4;

  const belowTop = rect.bottom + gap;
  const aboveTop = rect.top - popRect.height - gap;
  const belowSpace = vh - belowTop - margin;
  const aboveSpace = rect.top - gap - margin;
  let top = belowSpace >= popRect.height || belowSpace >= aboveSpace ? belowTop : aboveTop;
  const left = rect.left;

  const clamped = clampFloatingPosition(
    { x: left, y: top },
    { width: popRect.width, height: popRect.height },
    { width: vw, height: vh },
    margin,
  );

  positionStyle.value = { top: `${clamped.y}px`, left: `${clamped.x}px` };
}

// Close on scroll in any ancestor
let scrollParents: Element[] = [];

function findScrollParents(el: Element | null): Element[] {
  const parents: Element[] = [];
  let current = el?.parentElement;
  while (current) {
    const overflow = getComputedStyle(current).overflowY;
    if (overflow === "auto" || overflow === "scroll") {
      parents.push(current);
    }
    current = current.parentElement;
  }
  return parents;
}

function onScroll() {
  emit("close");
}

function onWindowResize() {
  updatePosition();
}

onMounted(() => {
  nextTick(updatePosition);
  scrollParents = findScrollParents(props.anchor);
  scrollParents.forEach((p) => p.addEventListener("scroll", onScroll, { passive: true }));
  window.addEventListener("resize", onWindowResize, { passive: true });
});

onUnmounted(() => {
  scrollParents.forEach((p) => p.removeEventListener("scroll", onScroll));
  window.removeEventListener("resize", onWindowResize);
});

watch(() => props.anchor, () => nextTick(updatePosition));
watch(() => props.payload, () => nextTick(updatePosition));
</script>

<template>
  <Teleport to="body">
    <div
      ref="popoverRef"
      class="diff-popover"
      :style="[positionStyle, sizeStyle]"
      @mouseenter="emit('enter')"
      @mouseleave="emit('leave')"
    >
      <div class="popover-summary">
        <span v-for="(line, i) in payload.previewSummary" :key="i" class="summary-line">
          {{ line }}
        </span>
      </div>
      <div class="popover-body">
        <FileDiffViewer :payload="payload" mode="unified" :compact="true" />
      </div>
      <button type="button" class="popover-hint" @click.stop="emit('open')">Click to see full diff</button>
    </div>
  </Teleport>
</template>

<style scoped>
.diff-popover {
  position: fixed;
  z-index: 150;
  box-sizing: border-box;
  width: min(var(--diff-popover-width, 760px), calc(100vw - 16px));
  min-height: min(360px, calc(100vh - 16px));
  height: min(var(--diff-popover-height, 520px), calc(100vh - 16px));
  max-height: calc(100vh - 16px);
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.popover-summary {
  padding: 6px 10px;
  font-size: 11px;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-color);
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.popover-body {
  flex: 1;
  overflow: auto;
  min-height: 0;
}
.popover-hint {
  padding: 4px 10px;
  border: none;
  font-size: 10px;
  color: var(--text-secondary);
  text-align: center;
  border-top: 1px solid var(--border-color);
  background: transparent;
  opacity: 0.6;
  cursor: pointer;
}
.popover-hint:hover,
.popover-hint:focus-visible {
  opacity: 1;
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}
</style>
