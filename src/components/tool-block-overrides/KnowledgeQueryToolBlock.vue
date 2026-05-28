<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { t } from "../../i18n";
import { buildToolCallArgsSummary } from "../toolCallSummary";
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
    nextTick(() => {
      runOnNextFrame(() => emit("toolViewportAnchorEnd", anchor));
    });
  }
}

function toggleExpanded() {
  setExpanded(!infoExpanded.value);
}

function expandFromBlockClick(event: MouseEvent) {
  if (infoExpanded.value || !hasInfoDetail.value) return;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest("button, a, input, textarea, select, [role='button'], .tool-call-detail, .ui-select-text")) {
    return;
  }
  setExpanded(true);
}

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

const outputDisplay = computed(() => {
  const output = props.toolCall.output;
  return output ? persistedOutputDisplay(output) : { kind: "normal" as const, text: "" };
});

const displayOutput = computed(() => outputDisplay.value.text);
const isDeletedOutput = computed(() => outputDisplay.value.kind === "deleted");
const deletedOutputPath = computed(() => outputDisplay.value.path || "");
const progress = computed(() => props.toolCall.status === "running" ? props.toolCall.progress : null);
const progressRatio = computed(() =>
  typeof progress.value?.progress === "number" ? clampProgress(progress.value.progress) : null,
);
const progressPercent = computed(() =>
  progressRatio.value === null ? "" : `${Math.round(progressRatio.value * 100)}%`,
);
const progressWidth = computed(() =>
  progressRatio.value === null ? "0%" : `${Math.round(progressRatio.value * 100)}%`,
);
const headerSummary = computed(() => {
  if (props.toolCall.status === "running" && progress.value) {
    return [progress.value.title, progress.value.info].filter((part) => part.trim()).join(" - ");
  }
  return buildToolCallArgsSummary(props.toolCall.name, props.toolCall.arguments);
});

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    case "error": return "error";
    case "interrupted": return "error";
  }
});

const showProgressLine = computed(() => props.toolCall.status === "running" && Boolean(progress.value));
const showRuntimeOnly = computed(() => props.toolCall.status === "running");
const hasInfoDetail = computed(() => !showRuntimeOnly.value || Boolean(displayOutput.value) || isDeletedOutput.value);
const isFramed = computed(() => infoExpanded.value || showProgressLine.value);
</script>

<template>
  <div
    ref="rootRef"
    class="knowledge-query-tool-block"
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
      <span v-if="headerSummary" class="tool-call-summary">{{ headerSummary }}</span>
    </button>

    <div v-if="showProgressLine && progress" class="tool-call-progress-line" aria-live="polite">
      <div class="knowledge-query-progress">
        <div class="knowledge-query-progress-row">
          <span class="knowledge-query-progress-title">{{ progress.title }}</span>
          <span v-if="progress.info" class="knowledge-query-progress-info">{{ progress.info }}</span>
          <span v-if="progressPercent" class="knowledge-query-progress-percent">{{ progressPercent }}</span>
        </div>
        <div v-if="progressRatio !== null" class="knowledge-query-progress-track" aria-hidden="true">
          <div class="knowledge-query-progress-fill" :style="{ width: progressWidth }"></div>
        </div>
      </div>
    </div>

    <div v-if="infoExpanded && hasInfoDetail" class="tool-call-detail">
      <template v-if="!showRuntimeOnly">
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
          <pre v-else-if="displayOutput" class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error' }">{{ displayOutput }}</pre>
          <pre v-else class="tool-call-pre ui-select-text">{{ t("tool.noOutput") }}</pre>
        </div>
      </template>

      <div v-else-if="displayOutput" class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.output") }}</div>
        <pre class="tool-call-pre ui-select-text streaming-output">{{ displayOutput }}</pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.knowledge-query-tool-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  margin: 0;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  overflow: hidden;
  font-size: 13px;
  transition: background 0.18s ease, border-color 0.18s ease, border-radius 0.18s ease, padding 0.18s ease;
}

.knowledge-query-tool-block.is-framed {
  width: 100%;
  padding: 4px 6px 6px;
  border-color: var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--msg-assistant-bg) 18%);
}

.knowledge-query-tool-block:not(.is-expanded) {
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

.tool-call-progress-line {
  align-self: stretch;
  margin-top: 4px;
  padding: 5px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
}

.knowledge-query-progress {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 2px 2px 1px;
  background: transparent;
}

.knowledge-query-progress-row {
  display: grid;
  grid-template-columns: minmax(0, auto) minmax(0, 1fr) auto;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  font-size: 12px;
  line-height: 1.4;
}

.knowledge-query-progress-title {
  min-width: 0;
  color: var(--text-color);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.knowledge-query-progress-info {
  min-width: 0;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.knowledge-query-progress-percent {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.knowledge-query-progress-track {
  height: 4px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.knowledge-query-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent-color);
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
  overflow-y: auto;
  scrollbar-gutter: stable;
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

.streaming-output {
  max-height: 220px;
}

.error-output {
  color: var(--status-danger-fg);
}
</style>
