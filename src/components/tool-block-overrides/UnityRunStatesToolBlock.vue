<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
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

const infoExpanded = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function setExpanded(value: boolean) {
  if (infoExpanded.value === value) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (anchor) emit("toolViewportAnchorStart", anchor);
  infoExpanded.value = value;

  if (anchor) {
    nextTick(() => {
      runOnNextFrame(() => emit("toolViewportAnchorEnd", anchor));
    });
  }
}

function toggleExpanded() {
  setExpanded(!infoExpanded.value);
}

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

const hasPrints = computed(() => (runtimePreview.value?.printText.trim().length ?? 0) > 0);
const showRuntimePrintText = computed(() => props.toolCall.status === "running" && hasPrints.value);

const runtimeProgressSummary = computed(() => {
  const runtime = runtimePreview.value;
  if (!runtime) return "";
  const parts: string[] = [];
  if (runtime.currentState) {
    parts.push(runtime.currentState);
  }
  if (runtime.isFinal && runtime.printCount > 0) {
    parts.push(t("tool.unityRunStates.printCount", runtime.printCount));
  }
  return parts.join(" · ");
});

const runtimePromptText = computed(() => runtimePreview.value?.promptText.trim() ?? "");

const printFallback = computed(() =>
  props.toolCall.status === "running"
    ? t("tool.unityRunStates.waitingPrints")
    : t("tool.unityRunStates.noPrints"),
);

const showFinalSections = computed(() => props.toolCall.status !== "running");
const hasInfoDetail = computed(() => showFinalSections.value);
const headerSummary = computed(() => runtimeProgressSummary.value);
const showRuntimeProgressLine = computed(() => props.toolCall.status === "running" && Boolean(runtimePreview.value));
const isFramed = computed(() => infoExpanded.value || showRuntimeProgressLine.value);
const showRuntimePromptText = computed(() => props.toolCall.status === "running" && Boolean(runtimePromptText.value));
const showRuntimePrintFallback = computed(() => showRuntimeProgressLine.value && !showRuntimePrintText.value);
</script>

<template>
  <div ref="rootRef" class="unity-tool-call-block unity-run-tool-block" :class="[toolCall.status, { 'is-expanded': infoExpanded, 'is-framed': isFramed }]">
    <button
      ref="headerRef"
      type="button"
      class="tool-call-header ui-select-none"
      :aria-expanded="infoExpanded && hasInfoDetail"
      @click="toggleExpanded"
    >
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="toolCall.status === 'running'" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">{{ toolCall.name }}</span>
      <span v-if="headerSummary" class="tool-call-summary">{{ headerSummary }}</span>
    </button>

    <div v-if="showRuntimeProgressLine" class="tool-call-progress-line" aria-live="polite">
      <div class="unity-run-progress">
        <div v-if="showRuntimePromptText" class="unity-run-prompt-text ui-select-text">{{ runtimePromptText }}</div>
        <pre v-if="showRuntimePrintText" class="unity-run-print-text ui-select-text">{{ runtimePreview?.printText ?? "" }}</pre>
        <div v-else-if="showRuntimePrintFallback" class="unity-run-empty">{{ printFallback }}</div>
      </div>
    </div>

    <div v-if="infoExpanded && hasInfoDetail" class="tool-call-detail">
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
          />
          <pre v-else class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error' }">{{ displayOutput }}</pre>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.unity-tool-call-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  margin: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  overflow: visible;
  font-size: 13px;
  transition: background 0.18s ease, border-color 0.18s ease, border-radius 0.18s ease, padding 0.18s ease;
}

.unity-tool-call-block.is-framed {
  width: 100%;
  padding: 4px 6px 6px;
  border: 1px solid color-mix(in srgb, #8b7cf6 46%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--msg-assistant-bg) 18%);
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
  margin-top: 6px;
  padding: 6px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
}

.tool-call-progress-line {
  align-self: stretch;
  margin-top: 4px;
  padding: 5px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
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

.unity-run-progress {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 2px 2px 1px;
  background: transparent;
}

.unity-run-prompt-text {
  min-width: 0;
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.unity-run-print-text {
  max-height: 260px;
}

.unity-run-progress .unity-run-print-text {
  padding: 0;
  border-radius: 0;
  background: transparent;
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

.unity-run-progress .unity-run-empty {
  min-height: 0;
  padding: 0;
}

.error-output {
  color: var(--status-danger-fg);
}
</style>
