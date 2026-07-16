<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { t } from "../../i18n";
import { persistedOutputDisplay } from "../toolPersistedOutput";

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
    nextTick(() => runOnNextFrame(() => emit("toolViewportAnchorEnd", anchor)));
  }
}

function toggleExpanded() {
  setExpanded(!infoExpanded.value);
}

function expandFromBlockClick(event: MouseEvent) {
  if (infoExpanded.value || !hasInfoDetail.value) return;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest("button, a, input, textarea, select, [role='button'], .tool-call-detail")) return;
  setExpanded(true);
}

const outputDisplay = computed(() => {
  const output = props.toolCall.output;
  return output ? persistedOutputDisplay(output) : { kind: "normal" as const, text: "" };
});

const displayOutput = computed(() => outputDisplay.value.text);
const isDeletedOutput = computed(() => outputDisplay.value.kind === "deleted");
const deletedOutputPath = computed(() => outputDisplay.value.path || "");
const toolProgress = computed(() => props.toolCall.status === "running" ? props.toolCall.progress : null);

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    case "error": return "error";
    case "interrupted": return "error";
  }
});

const progressText = computed(() => {
  const progress = toolProgress.value;
  if (!progress) return "";
  return [progress.title, progress.info].filter((part) => part.trim()).join(" · ");
});

const progressWidth = computed(() => {
  const value = toolProgress.value?.progress;
  if (typeof value !== "number" || !Number.isFinite(value)) return "0%";
  return `${Math.round(Math.min(1, Math.max(0, value)) * 100)}%`;
});

const summaryLine = computed(() => {
  const match = displayOutput.value.match(/^summary:\s*(.+)$/m);
  return match?.[1] ?? progressText.value;
});

const hasInfoDetail = computed(() => props.toolCall.status !== "running" || Boolean(displayOutput.value) || isDeletedOutput.value);
const showProgressLine = computed(() => props.toolCall.status === "running" && Boolean(toolProgress.value));
const isFramed = computed(() => infoExpanded.value || showProgressLine.value);
</script>

<template>
  <div
    ref="rootRef"
    class="unity-tool-call-block unity-test-tool-block"
    :class="[toolCall.status, { 'is-expanded': infoExpanded, 'is-framed': isFramed }]"
    @click="expandFromBlockClick"
  >
    <button
      ref="headerRef"
      type="button"
      class="tool-call-header ui-select-none"
      :aria-expanded="infoExpanded && hasInfoDetail"
      @click.stop="toggleExpanded"
    >
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="toolCall.status === 'running'" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">{{ toolCall.name }}</span>
      <span v-if="summaryLine" class="tool-call-summary">{{ summaryLine }}</span>
    </button>

    <div v-if="showProgressLine" class="tool-call-progress-line" aria-live="polite">
      <div class="unity-test-progress">
        <div class="unity-test-progress-row">
          <span class="unity-test-progress-title">{{ toolProgress?.title || "Unity tests" }}</span>
          <span class="unity-test-progress-info">{{ toolProgress?.info || "" }}</span>
        </div>
        <div class="unity-test-progress-track" aria-hidden="true">
          <div class="unity-test-progress-fill" :style="{ width: progressWidth }"></div>
        </div>
      </div>
    </div>

    <div v-if="infoExpanded && hasInfoDetail" class="tool-call-detail">
      <div class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.args") }}</div>
        <pre class="tool-call-pre ui-select-text">{{ toolCall.arguments }}</pre>
      </div>
      <div v-if="toolCall.output !== undefined" class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.output") }}</div>
        <div v-if="isDeletedOutput" class="tool-output-deleted">
          <div class="tool-output-deleted-title">{{ t("tool.persistedOutputDeleted") }}</div>
          <code v-if="deletedOutputPath" class="tool-output-deleted-path">
            {{ t("tool.persistedOutputDeletedPath", deletedOutputPath) }}
          </code>
        </div>
        <pre v-else class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error' }">{{ displayOutput || t("tool.noOutput") }}</pre>
      </div>
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
}

.unity-tool-call-block.is-framed {
  width: 100%;
  padding: 4px 6px 6px;
  border: 1px solid color-mix(in srgb, #2f9e44 42%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--msg-assistant-bg) 16%);
}

.unity-tool-call-block:not(.is-expanded) {
  cursor: pointer;
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
  min-height: 22px;
  text-align: left;
}

.tool-call-header:hover {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
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
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-call-progress-line {
  align-self: stretch;
  margin-top: 4px;
  padding: 5px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
}

.unity-test-progress {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 2px 2px 1px;
  background: transparent;
}

.unity-test-progress-row {
  display: grid;
  grid-template-columns: minmax(0, auto) minmax(0, 1fr);
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  font-size: 12px;
  line-height: 1.4;
}

.unity-test-progress-title {
  color: var(--text-color);
  font-weight: 600;
  white-space: nowrap;
}

.unity-test-progress-info {
  min-width: 0;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-test-progress-track {
  height: 4px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.unity-test-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: #2f9e44;
  transition: width 0.16s ease;
}

.tool-call-detail {
  align-self: stretch;
  margin-top: 6px;
  padding: 6px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
}

.tool-call-section {
  margin-bottom: 6px;
}

.tool-call-section-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

.tool-call-pre {
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
}

.tool-output-deleted {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-size: 12px;
}

.tool-output-deleted-title {
  color: var(--text-color);
  font-weight: 600;
}

.tool-output-deleted-path {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.error-output {
  color: var(--status-danger-fg);
}
</style>
