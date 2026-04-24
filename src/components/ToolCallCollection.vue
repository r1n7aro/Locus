<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { summarizeToolCallBatch } from "../composables/toolCallBatches";
import { t } from "../i18n";
import { logToolCollapseTrace } from "../services/toolCollapseTrace";

import type { ToolCallDisplay } from "../types";

const props = withDefaults(defineProps<{
  toolCalls: ToolCallDisplay[];
  allowCollapse?: boolean;
  collapseEnabled?: boolean;
}>(), {
  allowCollapse: true,
  collapseEnabled: true,
});
const emit = defineEmits<{
  (e: "collapseFinished"): void;
  (e: "viewportAnchorStart", anchor: HTMLElement): void;
  (e: "viewportAnchorEnd", anchor: HTMLElement): void;
}>();

const { state: displaySettings } = useDisplaySettings();
const expanded = ref(false);
const panelVisible = ref(false);
const summaryRef = ref<HTMLElement | null>(null);
const panelTransitionCleanup = new WeakMap<HTMLElement, () => void>();

const PANEL_TRANSITION = [
  "height 320ms cubic-bezier(0.2, 0, 0, 1)",
  "opacity 220ms ease",
  "transform 320ms cubic-bezier(0.2, 0, 0, 1)",
].join(", ");
const PANEL_TRANSITION_TIMEOUT_MS = 380;

const batchState = computed(() =>
  summarizeToolCallBatch(
    props.toolCalls,
    displaySettings.compactToolCalls && props.allowCollapse && props.collapseEnabled,
  ),
);

const batchSummary = computed(() => {
  const { total, errorCount, interruptedCount } = batchState.value;
  if (errorCount > 0 && interruptedCount > 0) {
    return t("tool.batch.summaryWithIssues", total, errorCount, interruptedCount);
  }
  if (errorCount > 0) {
    return t("tool.batch.summaryWithErrors", total, errorCount);
  }
  if (interruptedCount > 0) {
    return t("tool.batch.summaryWithInterrupted", total, interruptedCount);
  }
  return t("tool.batch.summary", total);
});

const toggleLabel = computed(() =>
  expanded.value ? t("tool.batch.collapse") : t("tool.batch.expand"),
);

const summaryOpen = computed(() =>
  batchState.value.canCollapse && (expanded.value || panelVisible.value),
);

function traceCollection(event: string, detail?: Record<string, unknown>) {
  logToolCollapseTrace("tool-collection", event, {
    firstToolCallId: props.toolCalls[0]?.id ?? "",
    total: props.toolCalls.length,
    allowCollapse: props.allowCollapse,
    collapseEnabled: props.collapseEnabled,
    canCollapse: batchState.value.canCollapse,
    expanded: expanded.value,
    panelVisible: panelVisible.value,
    ...detail,
  });
}

function clearPanelTransitionListener(element: HTMLElement) {
  const cleanup = panelTransitionCleanup.get(element);
  cleanup?.();
  panelTransitionCleanup.delete(element);
}

function preparePanelTransition(element: HTMLElement) {
  clearPanelTransitionListener(element);
  element.style.overflow = "hidden";
  element.style.transformOrigin = "top center";
  element.style.willChange = "height, opacity, transform";
  element.style.transition = PANEL_TRANSITION;
}

function resetPanelTransition(element: HTMLElement) {
  clearPanelTransitionListener(element);
  element.style.height = "";
  element.style.opacity = "";
  element.style.transform = "";
  element.style.overflow = "";
  element.style.transformOrigin = "";
  element.style.willChange = "";
  element.style.transition = "";
}

function queuePanelTransition(element: HTMLElement, done: () => void) {
  let finished = false;

  const complete = () => {
    if (finished) return;
    finished = true;
    clearPanelTransitionListener(element);
    done();
  };

  const onTransitionEnd = (event: Event) => {
    const transitionEvent = event as TransitionEvent;
    if (transitionEvent.target !== element || transitionEvent.propertyName !== "height") return;
    complete();
  };

  const timeoutId = setTimeout(complete, PANEL_TRANSITION_TIMEOUT_MS);
  element.addEventListener("transitionend", onTransitionEnd);
  panelTransitionCleanup.set(element, () => {
    clearTimeout(timeoutId);
    element.removeEventListener("transitionend", onTransitionEnd);
  });
}

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function emitViewportAnchorEnd() {
  const anchor = summaryRef.value;
  if (anchor) emit("viewportAnchorEnd", anchor);
}

function toggleExpanded() {
  const anchor = summaryRef.value;
  if (anchor) emit("viewportAnchorStart", anchor);
  expanded.value = !expanded.value;
}

function onPanelEnter(element: Element, done: () => void) {
  const panel = element as HTMLElement;
  traceCollection("panelEnter", {
    heightBefore: panel.scrollHeight,
  });
  panelVisible.value = true;
  preparePanelTransition(panel);
  panel.style.height = "0px";
  panel.style.opacity = "0";
  panel.style.transform = "translateY(-4px) scaleY(0.97)";
  void panel.offsetHeight;
  queuePanelTransition(panel, done);
  runOnNextFrame(() => {
    panel.style.height = `${panel.scrollHeight}px`;
    panel.style.opacity = "1";
    panel.style.transform = "translateY(0) scaleY(1)";
  });
}

function onPanelAfterEnter(element: Element) {
  traceCollection("panelAfterEnter", {
    heightAfter: (element as HTMLElement).scrollHeight,
  });
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

function onPanelEnterCancelled(element: Element) {
  traceCollection("panelEnterCancelled");
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

function onPanelLeave(element: Element, done: () => void) {
  const panel = element as HTMLElement;
  traceCollection("panelLeave", {
    heightBefore: panel.scrollHeight,
  });
  panelVisible.value = true;
  preparePanelTransition(panel);
  panel.style.height = `${panel.scrollHeight}px`;
  panel.style.opacity = "1";
  panel.style.transform = "translateY(0) scaleY(1)";
  void panel.offsetHeight;
  queuePanelTransition(panel, done);
  runOnNextFrame(() => {
    panel.style.height = "0px";
    panel.style.opacity = "0";
    panel.style.transform = "translateY(-4px) scaleY(0.97)";
  });
}

function onPanelAfterLeave(element: Element) {
  traceCollection("panelAfterLeave");
  panelVisible.value = false;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
  emit("collapseFinished");
}

function onPanelLeaveCancelled(element: Element) {
  traceCollection("panelLeaveCancelled");
  panelVisible.value = true;
  resetPanelTransition(element as HTMLElement);
  emitViewportAnchorEnd();
}

watch(
  () => ({
    firstId: props.toolCalls[0]?.id ?? "",
    total: props.toolCalls.length,
    allowCollapse: props.allowCollapse,
    collapseEnabled: props.collapseEnabled,
    canCollapse: batchState.value.canCollapse,
    summaryOpen: summaryOpen.value,
  }),
  (next, prev) => {
    traceCollection("stateChanged", {
      previous: prev ?? null,
      next,
    });
  },
  { immediate: true },
);

watch(expanded, (value, previousValue) => {
  traceCollection("expandedChanged", {
    previous: previousValue,
    next: value,
  });
});

watch(
  () => ({
    firstId: props.toolCalls[0]?.id ?? "",
    canCollapse: batchState.value.canCollapse,
  }),
  (next, prev) => {
    traceCollection("collapseResetCheck", {
      previous: prev ?? null,
      next,
    });
    if (!prev || next.firstId !== prev.firstId || (!prev.canCollapse && next.canCollapse)) {
      expanded.value = false;
    }
  },
  { immediate: true },
);
</script>

<template>
  <div
    class="tool-call-collection"
    :class="{
      'is-collapsible': batchState.canCollapse,
      'is-expanded': batchState.canCollapse && summaryOpen,
    }"
  >
    <button
      v-if="batchState.canCollapse"
      ref="summaryRef"
      type="button"
      class="tool-call-batch-summary ui-select-none"
      :class="{ open: summaryOpen }"
      :title="toggleLabel"
      :aria-label="toggleLabel"
      :aria-expanded="expanded"
      @click="toggleExpanded"
    >
      <span class="tool-call-batch-chevron" :class="{ open: summaryOpen }" aria-hidden="true">
        <svg viewBox="0 0 12 12" width="10" height="10">
          <path
            d="M4 2.5L8 6 4 9.5"
            fill="none"
            stroke="currentColor"
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="1.5"
          />
        </svg>
      </span>
      <span class="tool-call-batch-title">{{ batchSummary }}</span>
    </button>

    <Transition
      :css="false"
      @enter="onPanelEnter"
      @after-enter="onPanelAfterEnter"
      @enter-cancelled="onPanelEnterCancelled"
      @leave="onPanelLeave"
      @after-leave="onPanelAfterLeave"
      @leave-cancelled="onPanelLeaveCancelled"
    >
      <div
        v-if="!batchState.canCollapse || expanded"
        class="tool-call-collection-panel"
      >
        <div
          class="tool-call-collection-list"
          :class="{
            'with-summary': batchState.canCollapse,
            open: batchState.canCollapse && summaryOpen,
          }"
        >
          <template v-for="toolCall in toolCalls" :key="toolCall.id">
            <slot :tool-call="toolCall" />
          </template>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.tool-call-collection {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-call-collection.is-collapsible {
  margin-left: -8px;
}

.tool-call-collection.is-expanded {
  gap: 0;
}

.tool-call-collection-panel {
  min-width: 0;
}

.tool-call-batch-summary {
  appearance: none;
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 30px;
  padding: 5px 10px 5px 23px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}

.tool-call-batch-summary:hover,
.tool-call-batch-summary:focus-visible {
  background: color-mix(in srgb, var(--hover-bg) 72%, var(--msg-assistant-bg));
  border-color: color-mix(in srgb, var(--accent-color) 18%, var(--border-color));
}

.tool-call-batch-summary.open {
  border-color: color-mix(in srgb, var(--accent-color) 26%, var(--border-color));
  border-bottom-color: color-mix(in srgb, var(--border-color) 76%, transparent);
  border-radius: 8px 8px 0 0;
  background: color-mix(in srgb, var(--msg-assistant-bg) 76%, var(--panel-bg) 24%);
}

.tool-call-batch-summary:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 44%, transparent);
  outline-offset: 1px;
}

.tool-call-batch-title {
  min-width: 0;
  line-height: 1.45;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.tool-call-batch-meta {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-secondary);
}

.tool-call-batch-chevron {
  position: absolute;
  left: 6px;
  top: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  color: var(--text-secondary);
  transform: translateY(-50%);
  transition: transform 0.18s ease, color 0.15s;
}

.tool-call-batch-summary:hover .tool-call-batch-chevron,
.tool-call-batch-summary:focus-visible .tool-call-batch-chevron,
.tool-call-batch-summary.open .tool-call-batch-chevron {
  color: var(--text-color);
}

.tool-call-batch-chevron svg {
  display: block;
  overflow: visible;
}

.tool-call-batch-chevron.open {
  transform: translateY(-50%) rotate(90deg);
}

.tool-call-collection-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-call-collection-list.with-summary {
  margin-top: 0;
}

.tool-call-collection-list.with-summary.open {
  padding: 8px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-top: none;
  border-radius: 0 0 8px 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--msg-assistant-bg) 18%);
}
</style>
