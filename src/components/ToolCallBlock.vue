
<script setup lang="ts">
import { ref, computed, nextTick, watch } from "vue";
import { PanelTopOpen } from "lucide";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import ToolCallCollection from "./ToolCallCollection.vue";
import ToolResultImages from "./ToolResultImages.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import hljs, { langFromPath } from "../hljs";
import { diffStrings } from "../services/diff";
import { t } from "../i18n";
import { resolveToolBlockOverride } from "./tool-block-overrides/toolBlockOverrides";
import { buildToolCallArgsSummary } from "./toolCallSummary";
import { persistedOutputDisplay } from "./toolPersistedOutput";
import { agentGraphToolReopen } from "../services/agentGraphTool";
import { normalizeViewError, viewRun } from "../services/view";
import { useNotificationStore } from "../stores/notification";
import { useProjectStore } from "../stores/project";
import { traceToolBlockLayoutChange } from "../services/layoutDiagnostics";
import { resolveViewToolOpenId } from "./viewToolCallActions";

import type { ToolCallDisplay, FileDiffPayload } from "../types";

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

function isSubagentToolName(name: string) {
  return name === "explore" || name === "task";
}

function shouldAutoExpandSubagentTool(toolCall: ToolCallDisplay) {
  return isSubagentToolName(toolCall.name) && toolCall.status === "running";
}

const expanded = ref(shouldAutoExpandSubagentTool(props.toolCall));
const openingGraphView = ref(false);
const openingViewTool = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);
const outputPre = ref<HTMLPreElement | null>(null);
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();

watch(
  () => [props.toolCall.output, props.toolCall.nestedToolCalls?.length],
  () => {
    if (outputPre.value && props.toolCall.status === "running") {
      nextTick(() => {
        if (outputPre.value) {
          outputPre.value.scrollTop = outputPre.value.scrollHeight;
        }
      });
    }
  }
);

const isSubagentTool = computed(() => {
  const name = props.toolCall.name;
  return isSubagentToolName(name);
});

const waitingLabel = computed(() => (
  isSubagentTool.value ? t("tool.subagentWaiting") : t("tool.waiting")
));

const unityBackgroundHookSolved = computed(() =>
  projectStore.unityConnectionStatus?.backgroundHook?.patched === true,
);
const showRecompileHint = computed(() =>
  props.toolCall.name === "unity_recompile"
  && props.toolCall.status === "running"
  && !unityBackgroundHookSolved.value,
);
const toolBlockOverride = computed(() => resolveToolBlockOverride(props.toolCall.name));
const toolLayoutKey = computed(() => props.toolCall.id || props.toolCall.name);
const toolLayoutToolCallIds = computed(() => props.toolCall.id);
const toolLayoutStatuses = computed(() => `${props.toolCall.id}:${props.toolCall.status}`);

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

function emitToolViewportAnchorStart(anchor: HTMLElement) {
  emit("toolViewportAnchorStart", anchor);
}

function emitToolViewportAnchorEnd(anchor: HTMLElement) {
  emit("toolViewportAnchorEnd", anchor);
}

function traceBlockLayout(reason: string, detail: Record<string, unknown> = {}) {
  const root = rootRef.value;
  const scrollElement = root?.closest<HTMLElement>(".chat-transcript-scroll") ?? null;
  const contentElement = scrollElement?.querySelector<HTMLElement>(".chat-transcript-content") ?? null;
  traceToolBlockLayoutChange({
    scope: "tool-block",
    reason,
    scrollElement,
    contentElement,
    detail: {
      toolCallId: props.toolCall.id,
      toolName: props.toolCall.name,
      status: props.toolCall.status,
      expanded: expanded.value,
      collapseEnabled: props.collapseEnabled,
      ...detail,
    },
  });
}

function setExpanded(nextExpanded: boolean, preserveViewport = false) {
  if (expanded.value === nextExpanded) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (preserveViewport && anchor) emitToolViewportAnchorStart(anchor);
  traceBlockLayout("toolBlockExpandedChanged", {
    previousExpanded: expanded.value,
    nextExpanded,
    preserveViewport,
  });
  expanded.value = nextExpanded;

  if (preserveViewport && anchor) {
    nextTick(() => {
      runOnNextFrame(() => emitToolViewportAnchorEnd(anchor));
    });
  }
}

function toggleExpanded() {
  setExpanded(!expanded.value, true);
}

function expandFromBlockClick(event: MouseEvent) {
  if (expanded.value) return;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest("button, a, input, textarea, select, [role='button'], .tool-call-detail, .ui-select-text")) {
    return;
  }
  setExpanded(true, true);
}

watch(
  () => [props.toolCall.id, props.toolCall.name, props.toolCall.status] as const,
  ([nextId, _nextName, nextStatus], [previousId, _previousName, previousStatus]) => {
    if (nextId !== previousId) {
      setExpanded(shouldAutoExpandSubagentTool(props.toolCall), false);
      return;
    }
    if (!isSubagentTool.value) return;
    if (previousStatus === "running" && nextStatus !== "running") {
      setExpanded(false, true);
    } else if (previousStatus !== "running" && nextStatus === "running") {
      setExpanded(true, true);
    }
  },
);

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case "running": return "spinner";
    case "done": return "check";
    case "error": return "error";
    case "interrupted": return "error";
  }
});

const displayName = computed(() => {
  if (props.toolCall.name === "task") {
    try {
      const args = JSON.parse(props.toolCall.arguments);
      return args.subagent_type || "task";
    } catch {
      return "task";
    }
  }
  return props.toolCall.name;
});

const isEditTool = computed(() => props.toolCall.name === "edit");
const showGraphViewOpenButton = computed(() =>
  props.toolCall.name === "graph_view" && props.toolCall.status !== "running",
);
const viewToolOpenId = computed(() =>
  resolveViewToolOpenId({
    name: props.toolCall.name,
    arguments: props.toolCall.arguments,
    output: props.toolCall.output,
    status: props.toolCall.status,
  }),
);
const showViewOpenButton = computed(() => viewToolOpenId.value.length > 0);
const GRAPH_VIEW_HIDDEN_ARG_KEYS = new Set(["description"]);

interface EditDiffItem {
  oldStr: string;
  newStr: string;
  replaceAll: boolean;
  startLine: number;
}
interface EditDiffResult {
  filePath: string;
  items: EditDiffItem[];
}
function parseEditStartLines(output: string | undefined): number[] {
  if (!output) return [];
  const m = output.match(/\[lines:([0-9,]+)\]/);
  if (!m) return [];
  return m[1].split(",").map(Number);
}

const editDiffData = computed((): EditDiffResult | null => {
  if (!isEditTool.value) return null;
  try {
    const args = JSON.parse(props.toolCall.arguments);
    const filePath = args.filePath || args.file_path || args.path || "";
    const startLines = parseEditStartLines(props.toolCall.output);
    const items: EditDiffItem[] = [];
    if (Array.isArray(args.edits)) {
      for (let i = 0; i < args.edits.length; i++) {
        const edit = args.edits[i];
        items.push({
          oldStr: edit.oldString || edit.old_string || "",
          newStr: edit.newString || edit.new_string || "",
          replaceAll: edit.replaceAll || edit.replace_all || false,
          startLine: startLines[i] || 0,
        });
      }
    } else {
      const oldStr = args.oldString || args.old_string || "";
      const newStr = args.newString || args.new_string || "";
      if (oldStr || newStr) {
        items.push({
          oldStr,
          newStr,
          replaceAll: args.replaceAll || args.replace_all || false,
          startLine: startLines[0] || 0,
        });
      }
    }
    if (items.length === 0) return null;
    return { filePath, items };
  } catch {
    return null;
  }
});

// Compute diff payloads for each edit item using backend diff_strings
const editDiffPayloads = ref<Map<number, FileDiffPayload>>(new Map());

watch(editDiffData, async (data) => {
  editDiffPayloads.value = new Map();
  if (!data) return;
  for (let i = 0; i < data.items.length; i++) {
    const item = data.items[i];
    try {
      const hunks = await diffStrings(item.oldStr, item.newStr, 3);
      const additions = hunks.reduce((sum, h) => sum + h.lines.filter(l => l.kind === "add").length, 0);
      const deletions = hunks.reduce((sum, h) => sum + h.lines.filter(l => l.kind === "delete").length, 0);
      const payload: FileDiffPayload = {
        key: `edit-${i}`,
        filePath: data.filePath,
        status: "M",
        language: langFromPath(data.filePath) || undefined,
        isBinary: false,
        isLarge: false,
        contentState: { type: 'normal' },
        stats: { additions, deletions, changedHunks: hunks.length },
        previewSummary: [`+${additions} -${deletions}`],
        text: { hunks },
      };
      editDiffPayloads.value.set(i, payload);
    } catch {
      // Fall through to old rendering if diff fails
    }
  }
}, { immediate: true });

/** Syntax-highlight diff content and return HTML with line numbers.
 *  startLine: 1-based line number in the source file, 0 means start from 1
 */
function highlightDiffCode(code: string, filePath: string, startLine: number): string {
  if (!code) return "";
  const lang = filePath ? langFromPath(filePath) : null;
  let highlighted: string;
  if (lang) {
    try {
      highlighted = hljs.highlight(code, { language: lang }).value;
    } catch {
      highlighted = escapeHtml(code);
    }
  } else {
    highlighted = escapeHtml(code);
  }
  const base = startLine > 0 ? startLine : 1;
  const lines = highlighted.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line, i) =>
    `<div class="edit-diff-line"><span class="edit-diff-ln">${base + i}</span><span class="edit-diff-line-content">${line || " "}</span></div>`
  ).join("");
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

const parsedArgs = computed(() => {
  try {
    const args = JSON.parse(props.toolCall.arguments);
    if (typeof args !== "object" || args === null) return [];
    const isTask = props.toolCall.name === "task";
    const isEdit = props.toolCall.name === "edit";
    const editDiffKeys = ["oldString", "old_string", "newString", "new_string", "edits"];
    return Object.entries(args)
      .filter(([key]) => !isTask || key === "prompt")
      .filter(([key]) => !isEdit || !editDiffKeys.includes(key))
      .filter(([key]) => props.toolCall.name !== "graph_view" || !GRAPH_VIEW_HIDDEN_ARG_KEYS.has(key))
      .map(([key, value]) => ({
        key,
        value,
        isLong: typeof value === "string" && (value as string).length > 80,
        isMultiline: typeof value === "string" && (value as string).includes("\n"),
      }));
  } catch {
    return [];
  }
});

const rawArgsFallback = computed(() => {
  if (parsedArgs.value.length > 0) return "";
  return props.toolCall.arguments;
});

function formatValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  if (value === null) return "null";
  return JSON.stringify(value, null, 2);
}

function prettifyKey(key: string): string {
  return key
    .replace(/_/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .toLowerCase();
}

const argsSummary = computed(() =>
  buildToolCallArgsSummary(props.toolCall.name, props.toolCall.arguments),
);

async function reopenGraphView() {
  if (openingGraphView.value) return;
  openingGraphView.value = true;
  try {
    await agentGraphToolReopen({
      toolCallId: props.toolCall.id,
      arguments: props.toolCall.arguments,
      output: props.toolCall.output,
    });
  } catch {
    // ipcInvoke reports the error through the notification store.
  } finally {
    openingGraphView.value = false;
  }
}

async function openViewTool() {
  if (openingViewTool.value) return;
  const viewId = viewToolOpenId.value;
  if (!viewId) return;
  openingViewTool.value = true;
  try {
    await viewRun(viewId);
  } catch (error) {
    const err = normalizeViewError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "viewRunFromToolCall",
      replaceOperation: true,
    });
  } finally {
    openingViewTool.value = false;
  }
}

function getFilePath(): string {
  try {
    const args = JSON.parse(props.toolCall.arguments);
    return args.filePath || args.file_path || args.path || "";
  } catch {
    return "";
  }
}

const outputDisplay = computed(() => {
  const output = props.toolCall.output;
  return output ? persistedOutputDisplay(output) : { kind: "normal" as const, text: "" };
});

const displayOutput = computed(() => outputDisplay.value.text);
const isDeletedOutput = computed(() => outputDisplay.value.kind === "deleted");
const deletedOutputPath = computed(() => outputDisplay.value.path || "");
const toolResultImages = computed(() => props.toolCall.images ?? []);
const hasToolResultImages = computed(() => toolResultImages.value.length > 0);

const highlightedOutput = computed(() => {
  const output = props.toolCall.output;
  if (!output) return null;
  if (outputDisplay.value.kind !== "normal") return null;
  const name = props.toolCall.name;
  if (name !== "read" && name !== "write" && name !== "edit") return null;
  const filePath = getFilePath();
  if (!filePath) return null;
  const lang = langFromPath(filePath);
  if (!lang) return null;
  try {
    let code = displayOutput.value;
    const contentMatch = code.match(/^<content>\n?([\s\S]*?)\n?<\/content>\s*$/);
    if (contentMatch) {
      code = contentMatch[1];
    }
    return hljs.highlight(code, { language: lang }).value;
  } catch {
    return null;
  }
});


</script>

<template>
  <component
    :is="toolBlockOverride"
    v-if="toolBlockOverride"
    :tool-call="toolCall"
    :collapse-enabled="collapseEnabled"
    data-tool-layout-kind="block"
    :data-tool-layout-key="toolLayoutKey"
    :data-tool-layout-tool-call-ids="toolLayoutToolCallIds"
    :data-tool-layout-statuses="toolLayoutStatuses"
    :data-tool-layout-collapse-enabled="String(collapseEnabled)"
    :data-tool-layout-expanded="String(expanded)"
    @tool-viewport-anchor-start="emitToolViewportAnchorStart"
    @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
  />
  <div
    v-else
    ref="rootRef"
    class="tool-call-block"
    :class="[toolCall.status, { 'is-expanded': expanded, 'is-recompile-attention': showRecompileHint }]"
    data-tool-layout-kind="block"
    :data-tool-layout-key="toolLayoutKey"
    :data-tool-layout-tool-call-ids="toolLayoutToolCallIds"
    :data-tool-layout-statuses="toolLayoutStatuses"
    :data-tool-layout-collapse-enabled="String(collapseEnabled)"
    :data-tool-layout-expanded="String(expanded)"
    @click="expandFromBlockClick"
  >
    <div class="tool-call-header-row">
      <button ref="headerRef" type="button" class="tool-call-header ui-select-none" @click.stop="toggleExpanded">
        <span class="tool-call-icon" :class="statusIcon">
          <span v-if="toolCall.status === 'running'" class="spinner-anim"></span>
          <span v-else class="tool-call-status-dot"></span>
        </span>
        <span class="tool-call-name">{{ displayName }}</span>
        <span v-if="argsSummary" class="tool-call-summary">{{ argsSummary }}</span>
      </button>
      <button
        v-if="showGraphViewOpenButton"
        type="button"
        class="tool-call-action-button"
        :title="t('tool.graphView.open')"
        :aria-label="t('tool.graphView.open')"
        :disabled="openingGraphView"
        @click.stop="reopenGraphView"
      >
        <LucideIcon :icon="PanelTopOpen" :size="13" />
        <span>{{ t("tool.graphView.open") }}</span>
      </button>
      <button
        v-if="showViewOpenButton"
        type="button"
        class="tool-call-action-button"
        :title="t('tool.view.open')"
        :aria-label="t('tool.view.open')"
        :disabled="openingViewTool"
        @click.stop="openViewTool"
      >
        <LucideIcon :icon="PanelTopOpen" :size="13" />
        <span>{{ t("tool.view.open") }}</span>
      </button>
    </div>
    <div v-if="showRecompileHint" class="recompile-hint">
      <div class="recompile-hint-main">{{ t("tool.recompile.hint") }}</div>
      <div class="recompile-hint-sub">{{ t("tool.recompile.sub") }}</div>
    </div>
    <div v-if="expanded" class="tool-call-detail">
      <div class="tool-call-section">
        <div class="tool-call-section-label">{{ t("tool.section.args") }}</div>
        <template v-if="isEditTool && editDiffData">
          <div v-if="parsedArgs.length > 0" class="tool-args-table" style="margin-bottom: 6px;">
            <div v-for="arg in parsedArgs" :key="arg.key" class="tool-arg-row" :class="{ 'arg-block': arg.isMultiline || arg.isLong }">
              <span class="tool-arg-key">{{ prettifyKey(arg.key) }}</span>
              <pre v-if="arg.isMultiline" class="tool-arg-value-block">{{ formatValue(arg.value) }}</pre>
              <span v-else class="tool-arg-value" :class="{ 'value-bool': typeof arg.value === 'boolean', 'value-num': typeof arg.value === 'number' }">{{ formatValue(arg.value) }}</span>
            </div>
          </div>
          <template v-for="(item, idx) in editDiffData.items" :key="idx">
            <FileDiffViewer
              v-if="editDiffPayloads.get(idx)"
              :payload="editDiffPayloads.get(idx)!"
              :style="idx > 0 ? 'margin-top: 6px' : ''"
            />
            <div v-else class="edit-diff-container" :style="idx > 0 ? 'margin-top: 6px' : ''">
              <div class="edit-diff-panel edit-diff-old">
                <div class="edit-diff-panel-header edit-diff-header-old">
                  <span class="edit-diff-indicator">&#x2212;</span>
                  <span>{{ t("tool.diff.old") }}{{ editDiffData.items.length > 1 ? ` #${idx + 1}` : '' }}</span>
                </div>
                <pre class="edit-diff-code hljs" v-html="highlightDiffCode(item.oldStr, editDiffData.filePath, item.startLine)"></pre>
              </div>
              <div class="edit-diff-panel edit-diff-new">
                <div class="edit-diff-panel-header edit-diff-header-new">
                  <span class="edit-diff-indicator">&#x2b;</span>
                  <span>{{ t("tool.diff.new") }}{{ editDiffData.items.length > 1 ? ` #${idx + 1}` : '' }}</span>
                </div>
                <pre class="edit-diff-code hljs" v-html="highlightDiffCode(item.newStr, editDiffData.filePath, item.startLine)"></pre>
              </div>
            </div>
          </template>
        </template>
        <template v-else-if="isSubagentTool && parsedArgs.length === 1 && parsedArgs[0].key === 'prompt'">
          <pre class="tool-arg-prompt-direct ui-select-text">{{ formatValue(parsedArgs[0].value) }}</pre>
        </template>
        <div v-else-if="parsedArgs.length > 0" class="tool-args-table">
          <div v-for="arg in parsedArgs" :key="arg.key" class="tool-arg-row" :class="{ 'arg-block': arg.isMultiline || arg.isLong }">
            <span class="tool-arg-key">{{ prettifyKey(arg.key) }}</span>
              <pre v-if="arg.isMultiline" class="tool-arg-value-block ui-select-text">{{ formatValue(arg.value) }}</pre>
            <span v-else class="tool-arg-value" :class="{ 'value-bool': typeof arg.value === 'boolean', 'value-num': typeof arg.value === 'number' }">{{ formatValue(arg.value) }}</span>
          </div>
        </div>
        <pre v-else-if="rawArgsFallback" class="tool-call-pre ui-select-text">{{ rawArgsFallback }}</pre>
      </div>
      <div v-if="toolCall.output !== undefined || toolCall.status === 'running'" class="tool-call-section">
        <div class="tool-call-section-label">
          {{ t("tool.section.output") }}
          <span v-if="toolCall.status === 'running' && toolCall.output" class="output-streaming-indicator"></span>
        </div>
        <template v-if="toolCall.output || hasToolResultImages || (isSubagentTool && toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0)">
          <div v-if="isSubagentTool && toolCall.status !== 'error'" class="subagent-output ui-select-text" :class="{ 'streaming-output': toolCall.status === 'running' }" ref="outputPre">
            <div v-if="toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0" class="nested-tool-calls">
              <ToolCallCollection
                :tool-calls="toolCall.nestedToolCalls"
                :collapse-enabled="collapseEnabled"
                @viewport-anchor-start="emitToolViewportAnchorStart"
                @viewport-anchor-end="emitToolViewportAnchorEnd"
              >
                <template #default="{ toolCall: nestedToolCall }">
                  <ToolCallBlock
                    :tool-call="nestedToolCall"
                    :collapse-enabled="collapseEnabled"
                    @tool-viewport-anchor-start="emitToolViewportAnchorStart"
                    @tool-viewport-anchor-end="emitToolViewportAnchorEnd"
                  />
                </template>
              </ToolCallCollection>
            </div>
            <div v-if="toolCall.output && isDeletedOutput" class="tool-output-deleted">
              <div class="tool-output-deleted-title">{{ t("tool.persistedOutputDeleted") }}</div>
              <code v-if="deletedOutputPath" class="tool-output-deleted-path">
                {{ t("tool.persistedOutputDeletedPath", deletedOutputPath) }}
              </code>
            </div>
            <MarkdownRenderer v-else-if="toolCall.output" :content="displayOutput" />
          </div>
          <div v-else-if="toolCall.output && isDeletedOutput" class="tool-output-deleted">
            <div class="tool-output-deleted-title">{{ t("tool.persistedOutputDeleted") }}</div>
            <code v-if="deletedOutputPath" class="tool-output-deleted-path">
              {{ t("tool.persistedOutputDeletedPath", deletedOutputPath) }}
            </code>
          </div>
          <pre v-else-if="toolCall.output && highlightedOutput" class="tool-call-pre ui-select-text hljs" :class="{ 'error-output': toolCall.status === 'error', 'streaming-output': toolCall.status === 'running' }" ref="outputPre" v-html="highlightedOutput"></pre>
          <pre v-else-if="toolCall.output" class="tool-call-pre ui-select-text" :class="{ 'error-output': toolCall.status === 'error', 'streaming-output': toolCall.status === 'running' }" ref="outputPre">{{ displayOutput }}</pre>
          <ToolResultImages v-if="hasToolResultImages" :images="toolResultImages" />
        </template>
        <template v-else>
          <div v-if="toolCall.status === 'running'" class="tool-call-waiting">
            <span class="waiting-dots"></span>
            <span class="waiting-text">{{ waitingLabel }}</span>
          </div>
          <pre v-else class="tool-call-pre ui-select-text">{{ t("tool.noOutput") }}</pre>
        </template>
      </div>
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

.tool-call-block:not(.is-expanded) {
  cursor: pointer;
}

.tool-call-block.is-recompile-attention {
  align-items: stretch;
  padding: 4px 6px 6px;
  border: 1px solid var(--status-warn-border);
  border-left-width: 3px;
  border-left-color: var(--status-warn-fg);
  border-radius: 4px;
  background: color-mix(in srgb, var(--status-warn-bg) 82%, var(--panel-bg) 18%);
  overflow: hidden;
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

.tool-call-block.is-recompile-attention .tool-call-icon.spinner {
  color: var(--status-warn-fg);
}

.tool-call-header-row {
  width: 100%;
  max-width: 100%;
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
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

.tool-call-action-button {
  appearance: none;
  min-height: 24px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease, opacity 0.12s ease;
}

.tool-call-action-button:hover:not(:disabled),
.tool-call-action-button:focus-visible {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.tool-call-action-button:disabled {
  cursor: wait;
  opacity: 0.58;
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

.tool-args-table {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
}

.tool-arg-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  line-height: 1.5;
  font-size: 12px;
}

.tool-arg-row.arg-block {
  flex-direction: column;
  gap: 2px;
}

.tool-arg-key {
  color: var(--text-secondary);
  font-size: 11px;
  flex-shrink: 0;
  min-width: 60px;
  font-weight: 500;
}

.tool-arg-value {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-color);
  word-break: break-word;
  min-width: 0;
}

.tool-arg-value.value-bool {
  color: #d2a8ff;
}

.tool-arg-value.value-num {
  color: #79c0ff;
}

.tool-arg-value-block {
  font-family: var(--font-mono-block);
  font-size: 12px;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  padding: 4px 6px;
  border-radius: 4px;
  background: rgba(0, 0, 0, 0.15);
  line-height: 1.4;
}

.tool-arg-prompt-direct {
  font-family: var(--font-mono-block);
  font-size: 12px;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  line-height: 1.5;
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

.subagent-output {
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  overflow-y: auto;
  max-height: 600px;
  scrollbar-gutter: stable;
}

.nested-tool-calls {
  margin-bottom: 4px;
}

.nested-tool-calls :deep(.tool-call-collection) {
  gap: 2px;
}

.nested-tool-calls :deep(.tool-call-collection-list) {
  gap: 2px;
}

.nested-tool-calls :deep(.tool-call-block) {
  border-color: rgba(128, 128, 128, 0.2);
  font-size: 12px;
}

.nested-tool-calls :deep(.tool-call-header) {
  gap: 5px;
  min-height: 18px;
  padding: 0 4px;
  border-radius: 3px;
}

.nested-tool-calls :deep(.tool-call-icon) {
  width: 12px;
  height: 12px;
}

.nested-tool-calls :deep(.tool-call-status-dot) {
  width: 4px;
  height: 4px;
}

.nested-tool-calls :deep(.spinner-anim) {
  width: 8px;
  height: 8px;
  border-width: 1px;
}

.nested-tool-calls :deep(.tool-call-name) {
  font-size: 11px;
}

.nested-tool-calls :deep(.tool-call-summary) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-status) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-detail) {
  margin-top: 2px;
  padding: 3px 0 0 18px;
}

.nested-tool-calls :deep(.tool-call-batch-summary) {
  min-height: 20px;
  padding: 1px 6px 1px 15px;
  border-color: transparent;
  border-radius: 4px;
}

.nested-tool-calls :deep(.tool-call-batch-summary:hover),
.nested-tool-calls :deep(.tool-call-batch-summary:focus-visible) {
  border-color: rgba(128, 128, 128, 0.2);
}

.nested-tool-calls :deep(.tool-call-batch-summary.open) {
  border-color: rgba(128, 128, 128, 0.24);
  border-radius: 6px 6px 0 0;
}

.nested-tool-calls :deep(.tool-call-batch-summary.open.closing) {
  border-color: transparent;
  border-radius: 6px;
}

.nested-tool-calls :deep(.tool-call-batch-chevron) {
  left: 3px;
  width: 10px;
  height: 10px;
}

.nested-tool-calls :deep(.tool-call-batch-chevron svg) {
  width: 9px;
  height: 9px;
}

.nested-tool-calls :deep(.tool-call-batch-title) {
  font-size: 11px;
}

.nested-tool-calls :deep(.tool-call-batch-meta) {
  font-size: 10px;
}

.nested-tool-calls :deep(.tool-call-collection-list.with-summary.open) {
  padding: 4px;
  border-color: rgba(128, 128, 128, 0.24);
  border-radius: 0 0 6px 6px;
}

.error-output {
  color: #f85149;
}

.streaming-output {
  max-height: 300px;
  overflow-y: auto;
  scrollbar-gutter: stable;
  border-left: 2px solid #4a9eff;
}

.output-streaming-indicator {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #4a9eff;
  margin-left: 6px;
  vertical-align: middle;
  animation: output-pulse 1s ease-in-out infinite;
}

@keyframes output-pulse {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}

.tool-call-waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  font-size: 12px;
  color: var(--text-secondary);
  scrollbar-gutter: stable;
}

.waiting-dots {
  display: inline-flex;
  gap: 3px;
}

.waiting-dots::before,
.waiting-dots::after {
  content: "";
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--text-secondary);
  animation: dot-bounce 1.4s infinite ease-in-out both;
}

.waiting-dots::before {
  animation-delay: 0s;
}

.waiting-dots::after {
  animation-delay: 0.32s;
}

@keyframes dot-bounce {
  0%, 80%, 100% { opacity: 0.2; transform: scale(0.8); }
  40% { opacity: 1; transform: scale(1); }
}

.waiting-text {
  font-style: italic;
}

.recompile-hint {
  align-self: stretch;
  margin-top: 4px;
  padding: 6px 2px 0 20px;
  border-top: 1px solid color-mix(in srgb, var(--status-warn-border) 72%, transparent);
}

.recompile-hint-main {
  font-size: 13px;
  font-weight: 600;
  color: var(--status-warn-fg);
}

.recompile-hint-sub {
  font-size: 11px;
  color: color-mix(in srgb, var(--status-warn-fg) 48%, var(--text-secondary));
  margin-top: 2px;
}

.edit-diff-container {
  display: flex;
  flex-direction: column;
  gap: 0;
  border-radius: 6px;
  overflow: hidden;
  border: 1px solid var(--border-color);
  background: var(--hover-bg);
}

.edit-diff-panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}

.edit-diff-panel-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  user-select: none;
  flex-shrink: 0;
}

.edit-diff-header-old {
  background: rgba(248, 81, 73, 0.15);
  color: #f85149;
  border-bottom: 1px solid rgba(248, 81, 73, 0.15);
}

.edit-diff-header-new {
  background: rgba(63, 185, 80, 0.15);
  color: #3fb950;
  border-bottom: 1px solid rgba(63, 185, 80, 0.15);
}

.edit-diff-indicator {
  font-size: 14px;
  font-weight: 700;
  line-height: 1;
}

.edit-diff-code {
  font-family: var(--font-mono-block);
  font-size: 13px;
  line-height: 1.6;
  padding: 12px 0;
  margin: 0;
  white-space: pre;
  overflow-x: auto;
  flex: 1;
  min-height: 0;
}

.edit-diff-code :deep(.edit-diff-line) {
  display: block;
}

.edit-diff-code :deep(.edit-diff-ln) {
  display: inline-block;
  width: 3.5em;
  padding-right: 12px;
  text-align: right;
  color: var(--line-number-color, #6e7681);
  user-select: none;
  opacity: 0.6;
  font-size: 12px;
  font-family: inherit;
}

.edit-diff-code :deep(.edit-diff-line-content) {
  padding-left: 4px;
}

.edit-diff-old .edit-diff-code {
  border-left: 3px solid rgba(248, 81, 73, 0.6);
}

.edit-diff-new .edit-diff-code {
  border-left: 3px solid rgba(63, 185, 80, 0.6);
}

.edit-diff-old {
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}
</style>
