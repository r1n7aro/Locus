<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import hljs from "../../hljs";
import { t } from "../../i18n";
import { persistedOutputDisplay } from "../toolPersistedOutput";
import ToolResultImages from "../ToolResultImages.vue";

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

const expanded = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);

function setExpanded(value: boolean) {
  if (expanded.value === value) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (anchor) emit("toolViewportAnchorStart", anchor);
  expanded.value = value;
  if (anchor) {
    nextTick(() => {
      const done = () => emit("toolViewportAnchorEnd", anchor);
      if (typeof requestAnimationFrame === "function") requestAnimationFrame(done);
      else setTimeout(done, 16);
    });
  }
}

function expandFromBlockClick(event: MouseEvent) {
  if (expanded.value) return;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest("button, a, input, textarea, select, [role='button'], .tool-call-detail, .ui-select-text")) {
    return;
  }
  setExpanded(true);
}

function escapeHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function highlightPython(code: string): string {
  try {
    return hljs.highlight(code, { language: "python" }).value;
  } catch {
    return escapeHtml(code);
  }
}

const args = computed<Record<string, unknown>>(() => {
  try {
    const parsed = JSON.parse(props.toolCall.arguments);
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
});

const code = computed(() => (typeof args.value.code === "string" ? args.value.code : ""));
const target = computed(() => {
  const value = args.value.target;
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
});

const outputDisplay = computed(() => {
  const output = props.toolCall.output;
  return output ? persistedOutputDisplay(output) : { kind: "normal" as const, text: "" };
});
const isDeletedOutput = computed(() => outputDisplay.value.kind === "deleted");
const rawOutput = computed(() => (isDeletedOutput.value ? "" : outputDisplay.value.text));

/** `[Blender 4.2.1 · PID 1234 · hero.blend · interactive]` header line. */
const instanceLine = computed(() => {
  const first = rawOutput.value.split("\n", 1)[0] ?? "";
  const match = /^\[Blender (.+)\]$/.exec(first.trim());
  return match ? match[1] : "";
});
const bodyOutput = computed(() =>
  instanceLine.value ? rawOutput.value.slice(rawOutput.value.indexOf("\n") + 1) : rawOutput.value,
);

const codeSummary = computed(() => {
  const line = code.value
    .split("\n")
    .map((part) => part.trim())
    .find((part) => part && !part.startsWith("#")) ?? "";
  return line.length <= 60 ? line : `${line.slice(0, 57)}...`;
});
const headerSummary = computed(() => [target.value, codeSummary.value].filter(Boolean).join(" · "));
const images = computed(() => props.toolCall.images ?? []);
const running = computed(() => props.toolCall.status === "running");
const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    default: return "error";
  }
});
const liveStatus = computed(() => {
  const live = running.value ? props.toolCall.progress : null;
  return live ? [live.title, live.info].filter((part) => part && part.trim()).join(" · ") : "";
});
</script>

<template>
  <div
    ref="rootRef"
    class="blender-tool-call-block"
    :class="[toolCall.status, { 'is-expanded': expanded, 'is-framed': expanded || images.length > 0 }]"
    @click="expandFromBlockClick"
  >
    <button
      ref="headerRef"
      type="button"
      class="tool-call-header ui-select-none"
      :aria-expanded="expanded"
      @click.stop="setExpanded(!expanded)"
    >
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="running" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">{{ toolCall.name }}</span>
      <span v-if="headerSummary" class="tool-call-summary">{{ headerSummary }}</span>
      <span v-if="instanceLine && !running" class="tool-call-instance">{{ instanceLine }}</span>
      <span v-if="liveStatus" class="tool-call-inline-status">{{ liveStatus }}</span>
    </button>

    <ToolResultImages v-if="images.length > 0 && !expanded" class="blender-images" :images="images" />

    <div v-if="expanded" class="tool-call-detail">
      <div class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.args") }}</div>
        <div v-if="target" class="tool-call-meta">{{ t("blender.tool.target") }}: {{ target }}</div>
        <pre v-if="code" class="tool-call-pre ui-select-text hljs" v-html="highlightPython(code)"></pre>
        <pre v-else class="tool-call-pre ui-select-text">{{ toolCall.arguments }}</pre>
      </div>
      <div v-if="toolCall.output !== undefined || running" class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.output") }}</div>
        <div v-if="instanceLine" class="tool-call-meta">{{ instanceLine }}</div>
        <div v-if="isDeletedOutput" class="tool-call-meta">{{ t("tool.persistedOutputDeleted") }}</div>
        <pre
          v-else-if="bodyOutput"
          class="tool-call-pre ui-select-text"
          :class="{ 'error-output': toolCall.status === 'error', 'streaming-output': running }"
        >{{ bodyOutput }}</pre>
        <pre v-else-if="!running" class="tool-call-pre ui-select-text">{{ t("tool.noOutput") }}</pre>
        <ToolResultImages v-if="images.length > 0" :images="images" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.blender-tool-call-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  border: 1px solid transparent;
  border-radius: 6px;
  overflow: hidden;
  font-size: 13px;
  transition: background 0.18s ease, border-color 0.18s ease;
}

.blender-tool-call-block.is-framed {
  padding: 4px 6px 6px;
  border-color: color-mix(in srgb, #e87d0d 22%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--msg-assistant-bg) 18%);
}

.blender-tool-call-block:not(.is-expanded) {
  cursor: pointer;
}

.tool-call-header {
  appearance: none;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  width: 100%;
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

.tool-call-icon.spinner { color: var(--accent-color); }
.tool-call-icon.check { color: var(--text-secondary); }
.tool-call-icon.error { color: var(--status-danger-fg); }

.tool-call-status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.6;
}

.spinner-anim {
  width: 10px;
  height: 10px;
  border: 1.5px solid color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: blender-tool-spin 0.8s linear infinite;
  display: inline-block;
}

@keyframes blender-tool-spin {
  to { transform: rotate(360deg); }
}

.tool-call-name {
  font-family: var(--font-ui-label);
  color: var(--text-secondary);
  flex-shrink: 0;
}

.tool-call-summary,
.tool-call-instance {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-call-instance {
  flex-shrink: 1;
  opacity: 0.72;
}

.tool-call-inline-status {
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
  opacity: 0.72;
}

.blender-images {
  align-self: stretch;
  margin: 4px 0 0 20px;
}

.tool-call-detail {
  align-self: stretch;
  margin-top: 6px;
  padding: 6px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
}

.tool-call-section { margin-bottom: 6px; }
.tool-call-section:last-child { margin-bottom: 0; }

.tool-call-section-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

.tool-call-meta {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
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

.streaming-output { max-height: 220px; overflow-y: auto; }
.error-output { color: var(--status-danger-fg); }
</style>
