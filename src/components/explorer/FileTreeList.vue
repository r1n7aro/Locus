<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../composables/resizeObserver";

interface FileTreeListItem {
  key: string;
}

const props = withDefaults(defineProps<{
  items: FileTreeListItem[];
  rowHeight?: number;
  overscan?: number;
}>(), {
  rowHeight: 30,
  overscan: 8,
});

const emit = defineEmits<{
  (e: "visibleRangeChange", payload: { start: number; end: number }): void;
}>();

const scrollRef = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);

let resizeObserver: ResizeObserverHandle | null = null;
let scrollFrame = 0;

function updateViewportMetrics() {
  const element = scrollRef.value;
  scrollTop.value = element?.scrollTop ?? 0;
  viewportHeight.value = element?.clientHeight ?? 0;
}

function scheduleViewportMetrics() {
  if (scrollFrame) return;
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = 0;
    updateViewportMetrics();
  });
}

const virtualWindow = computed(() => {
  const rowHeight = Math.max(1, props.rowHeight);
  const total = props.items.length;
  if (total === 0) {
    return {
      start: 0,
      end: 0,
      topSpacer: 0,
      bottomSpacer: 0,
      items: [] as FileTreeListItem[],
    };
  }

  const visibleCount = Math.max(1, Math.ceil(viewportHeight.value / rowHeight));
  const start = Math.max(0, Math.floor(scrollTop.value / rowHeight) - props.overscan);
  const end = Math.min(total, start + visibleCount + props.overscan * 2);

  return {
    start,
    end,
    topSpacer: start * rowHeight,
    bottomSpacer: Math.max(0, (total - end) * rowHeight),
    items: props.items.slice(start, end),
  };
});

watch(
  () => [virtualWindow.value.start, virtualWindow.value.end, props.items.length],
  () => {
    emit("visibleRangeChange", {
      start: virtualWindow.value.start,
      end: Math.max(virtualWindow.value.start, virtualWindow.value.end - 1),
    });
  },
  { immediate: true },
);

watch(
  () => props.items.length,
  () => {
    scheduleViewportMetrics();
  },
);

/**
 * Scroll a row into view. "auto" only scrolls when the row is outside the
 * viewport; "center" always re-centers (used for explicit reveal actions).
 */
function scrollToIndex(index: number, options?: { align?: "auto" | "center" }) {
  const element = scrollRef.value;
  if (!element || !props.items.length) return;
  const rowHeight = Math.max(1, props.rowHeight);
  const clamped = Math.max(0, Math.min(index, props.items.length - 1));
  const top = clamped * rowHeight;
  const bottom = top + rowHeight;
  const viewTop = element.scrollTop;
  const viewBottom = viewTop + element.clientHeight;
  const align = options?.align ?? "auto";
  if (align === "auto" && top >= viewTop && bottom <= viewBottom) return;
  const nextTop =
    align === "center"
      ? top - Math.max(0, (element.clientHeight - rowHeight) / 2)
      : top < viewTop
        ? top
        : bottom - element.clientHeight;
  element.scrollTop = Math.max(0, nextTop);
  updateViewportMetrics();
}

defineExpose({ scrollToIndex });

onMounted(() => {
  updateViewportMetrics();
  if (scrollRef.value && typeof ResizeObserver !== "undefined") {
    resizeObserver = createAnimationFrameResizeObserver(updateViewportMetrics);
    resizeObserver?.observe(scrollRef.value);
  }
});

onUnmounted(() => {
  if (scrollFrame) {
    cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
  }
  resizeObserver?.disconnect();
  resizeObserver = null;
});
</script>

<template>
  <div ref="scrollRef" class="file-tree-list" @scroll="scheduleViewportMetrics">
    <slot v-if="!items.length" name="empty"></slot>
    <template v-else>
      <div
        v-if="virtualWindow.topSpacer > 0"
        class="file-tree-list-spacer"
        :style="{ height: `${virtualWindow.topSpacer}px` }"
      ></div>
      <template v-for="(item, localIndex) in virtualWindow.items" :key="item.key">
        <slot
          name="item"
          :item="item"
          :index="virtualWindow.start + localIndex"
        ></slot>
      </template>
      <div
        v-if="virtualWindow.bottomSpacer > 0"
        class="file-tree-list-spacer"
        :style="{ height: `${virtualWindow.bottomSpacer}px` }"
      ></div>
    </template>
  </div>
</template>

<style scoped>
.file-tree-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  overflow-anchor: none;
}

.file-tree-list-spacer {
  width: 100%;
  pointer-events: none;
}
</style>
