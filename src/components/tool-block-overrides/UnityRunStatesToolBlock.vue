<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { t } from "../../i18n";
import UnityRunStatesPreview from "../tool-previews/UnityRunStatesPreview.vue";
import UnityRunStatesOutputPreview from "../tool-previews/UnityRunStatesOutputPreview.vue";
import {
  buildUnityRunStatesRuntimePreview,
  parseUnityRunStatesArguments,
  parseUnityRunStatesOutput,
} from "../../composables/unityRunStatesPreview";

import type { ToolCallDisplay } from "../../types";

const props = withDefaults(defineProps<{
  toolCall: ToolCallDisplay;
  collapseEnabled?: boolean;
}>(), {
  collapseEnabled: true,
});

const emit = defineEmits<{
  (e: "toolViewportAnchorStart", anchor: HTMLElement): void;
  (e: "toolViewportAnchorEnd", anchor: HTMLElement): void;
}>();

const expanded = ref(props.toolCall.status === "running");
const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);
let collapseTimer: ReturnType<typeof setTimeout> | null = null;

function clearCollapseTimer() {
  if (collapseTimer === null) return;
  clearTimeout(collapseTimer);
  collapseTimer = null;
}

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function setExpanded(value: boolean) {
  if (expanded.value === value) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (anchor) emit("toolViewportAnchorStart", anchor);
  expanded.value = value;

  if (anchor) {
    nextTick(() => {
      runOnNextFrame(() => emit("toolViewportAnchorEnd", anchor));
    });
  }
}

function toggleExpanded() {
  clearCollapseTimer();
  setExpanded(!expanded.value);
}

watch(
  () => props.toolCall.status,
  (status, previousStatus) => {
    clearCollapseTimer();
    if (status === "running") {
      setExpanded(true);
      return;
    }

    if (previousStatus === "running" || expanded.value) {
      collapseTimer = setTimeout(() => {
        setExpanded(false);
        collapseTimer = null;
      }, 1400);
    }
  },
);

onBeforeUnmount(clearCollapseTimer);

function unwrapPersistedOutput(output: string): string {
  const match = output.match(/^<persisted-output>\n?([\s\S]*?)\n?<\/persisted-output>\s*$/);
  return match ? match[1].trim() : output;
}

const displayOutput = computed(() => {
  const output = props.toolCall.output;
  return output ? unwrapPersistedOutput(output) : "";
});

const argsPreview = computed(() => parseUnityRunStatesArguments(props.toolCall.arguments));

const outputPreview = computed(() => {
  if (!displayOutput.value) return null;
  return parseUnityRunStatesOutput(displayOutput.value);
});

const runtimePreview = computed(() =>
  buildUnityRunStatesRuntimePreview(
    props.toolCall.arguments,
    displayOutput.value,
    props.toolCall.status,
  ),
);

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    case "error": return "error";
    case "interrupted": return "error";
  }
});

const promptSummary = computed(() =>
  runtimePreview.value?.promptText.replace(/\s+/g, " ").trim() ?? "",
);

const headerSummary = computed(() => {
  const runtime = runtimePreview.value;
  const parts: string[] = [];
  if (runtime?.currentState) {
    parts.push(`${t("tool.unityRunStates.currentState")} ${runtime.currentState}`);
  }
  if (promptSummary.value) {
    parts.push(`${t("tool.unityRunStates.userPrompt")} ${promptSummary.value}`);
  }
  if (runtime?.isFinal && runtime.printCount > 0 && !expanded.value) {
    parts.push(t("tool.unityRunStates.printCount", runtime.printCount));
  }
  return parts.join(" · ");
});

const hasPrints = computed(() => (runtimePreview.value?.printText.trim().length ?? 0) > 0);

const printFallback = computed(() =>
  props.toolCall.status === "running"
    ? t("tool.unityRunStates.waitingPrints")
    : t("tool.unityRunStates.noPrints"),
);

const showFinalSections = computed(() => props.toolCall.status !== "running");
</script>

<template>
  <div ref="rootRef" class="tool-call-block unity-run-tool-block" :class="[toolCall.status, { 'is-expanded': expanded }]">
    <button ref="headerRef" type="button" class="tool-call-header ui-select-none" @click="toggleExpanded">
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="toolCall.status === 'running'" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">{{ toolCall.name }}</span>
      <span v-if="headerSummary" class="tool-call-summary">{{ headerSummary }}</span>
    </button>

    <div v-if="expanded" class="tool-call-detail">
      <div v-if="runtimePreview" class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.unityRunStates.printLabel") }}</div>
        <pre v-if="hasPrints" class="unity-run-print-text ui-select-text">{{ runtimePreview.printText }}</pre>
        <div v-else class="unity-run-empty">{{ printFallback }}</div>
      </div>

      <template v-if="showFinalSections">
        <div class="tool-call-section">
          <div class="tool-call-section-label">{{ t("tool.section.args") }}</div>
          <UnityRunStatesPreview v-if="argsPreview" :preview="argsPreview" />
          <pre v-else class="tool-call-pre ui-select-text">{{ toolCall.arguments }}</pre>
        </div>

        <div v-if="toolCall.output !== undefined" class="tool-call-section">
          <div class="tool-call-section-label">{{ t("tool.section.output") }}</div>
          <UnityRunStatesOutputPreview
            v-if="outputPreview"
            :preview="outputPreview"
            hide-prints
          />
          <pre v-else class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error' }">{{ displayOutput }}</pre>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.tool-call-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  margin: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  overflow: visible;
  font-size: 13px;
}

.tool-call-block.is-expanded {
  width: 100%;
}

.tool-call-header {
  appearance: none;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  width: 100%;
  max-width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 1px 4px;
  border-radius: 4px;
  cursor: pointer;
  user-select: none;
  min-height: 22px;
  text-align: left;
  transition: color 0.12s ease, background 0.12s ease;
}

.tool-call-header:hover {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
}

.tool-call-header:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 36%, transparent);
  outline-offset: 1px;
}

.tool-call-icon {
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.tool-call-icon.spinner {
  color: var(--accent-color);
}

.tool-call-icon.check {
  color: var(--text-secondary);
}

.tool-call-icon.error {
  color: var(--status-danger-fg);
}

.tool-call-status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.7;
}

.tool-call-icon.check .tool-call-status-dot {
  opacity: 0.46;
}

.tool-call-icon.error .tool-call-status-dot {
  width: 6px;
  height: 6px;
  opacity: 0.78;
}

.spinner-anim {
  width: 10px;
  height: 10px;
  border: 1.5px solid color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: tool-spin 0.8s linear infinite;
  display: inline-block;
}

@keyframes tool-spin {
  to { transform: rotate(360deg); }
}

.tool-call-name {
  font-weight: 600;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
  font-size: 12px;
  flex-shrink: 0;
}

.tool-call-summary {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-call-detail {
  align-self: stretch;
  margin-top: 4px;
  padding: 6px 0 0 26px;
}

.tool-call-section {
  margin-bottom: 6px;
}

.tool-call-section:last-child {
  margin-bottom: 0;
}

.tool-call-section-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

.tool-call-pre,
.unity-run-print-text {
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.4;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

.unity-run-print-text {
  max-height: 260px;
}

.unity-run-empty {
  display: flex;
  align-items: center;
  min-height: 28px;
  padding: 0 2px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.error-output {
  color: var(--status-danger-fg);
}
</style>
