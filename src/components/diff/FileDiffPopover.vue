<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, nextTick } from "vue";
import FileDiffViewer from "./FileDiffViewer.vue";
import type { FileDiffPayload } from "../../types";

const props = defineProps<{
  payload: FileDiffPayload;
  anchor: HTMLElement;
}>();

const popoverRef = ref<HTMLElement | null>(null);
const style = ref({ top: "0px", left: "0px" });

function updatePosition() {
  if (!props.anchor || !popoverRef.value) return;
  const rect = props.anchor.getBoundingClientRect();
  const popRect = popoverRef.value.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  let top = rect.bottom + 4;
  let left = rect.left;

  // Flip vertically if no room below
  if (top + popRect.height > vh && rect.top - popRect.height - 4 > 0) {
    top = rect.top - popRect.height - 4;
  }
  // Clamp horizontally
  if (left + popRect.width > vw) {
    left = vw - popRect.width - 8;
  }
  if (left < 8) left = 8;

  style.value = { top: `${top}px`, left: `${left}px` };
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

const emit = defineEmits<{ close: [] }>();

function onScroll() {
  emit("close");
}

onMounted(() => {
  nextTick(updatePosition);
  scrollParents = findScrollParents(props.anchor);
  scrollParents.forEach((p) => p.addEventListener("scroll", onScroll, { passive: true }));
});

onUnmounted(() => {
  scrollParents.forEach((p) => p.removeEventListener("scroll", onScroll));
});

watch(() => props.anchor, () => nextTick(updatePosition));
</script>

<template>
  <Teleport to="body">
    <div
      ref="popoverRef"
      class="diff-popover"
      :style="style"
    >
      <div class="popover-summary">
        <span v-for="(line, i) in payload.previewSummary" :key="i" class="summary-line">
          {{ line }}
        </span>
      </div>
      <div class="popover-body">
        <FileDiffViewer :payload="payload" mode="unified" :compact="true" />
      </div>
      <div class="popover-hint">Click to see full diff</div>
    </div>
  </Teleport>
</template>

<style scoped>
.diff-popover {
  position: fixed;
  z-index: 150;
  width: 480px;
  max-height: 260px;
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
  font-size: 10px;
  color: var(--text-secondary);
  text-align: center;
  border-top: 1px solid var(--border-color);
  opacity: 0.6;
}
</style>
