<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import { getDebugMode } from "../../services/permissions";
import { normalizeAppError } from "../../services/errors";
import { useNotificationStore } from "../../stores/notification";
import {
  clearDebugConsole,
  getDebugConsoleSnapshot,
  initDebugConsole,
  refreshDebugConsole,
  saveDebugConsoleLogExport,
  subscribeDebugConsole,
} from "../../services/debugConsole";
import type { DebugConsoleEntry } from "../../types";

type LevelFilter = "all" | "trace" | "debug" | "info" | "warn" | "error";
type SourceFilter = "all" | "backend" | "frontend";
type ResizableConsoleColumn = "timeWidth" | "sourceWidth" | "moduleWidth";
type ConsoleColumnWidths = Record<ResizableConsoleColumn, number>;
interface HighlightSegment {
  text: string;
  hit: boolean;
}

const CONSOLE_COLUMN_STORAGE_KEY = "locus.settings.console.columns.v1";
const CONSOLE_MESSAGE_MIN_WIDTH = 320;
const CONSOLE_MESSAGE_MAX_HEIGHT = 132;
const CONSOLE_MESSAGE_PREVIEW_LIMIT = 4_000;
const DEFAULT_COLUMN_WIDTHS: ConsoleColumnWidths = {
  timeWidth: 88,
  sourceWidth: 72,
  moduleWidth: 220,
};
const MIN_COLUMN_WIDTHS: ConsoleColumnWidths = {
  timeWidth: 78,
  sourceWidth: 58,
  moduleWidth: 140,
};
const MAX_COLUMN_WIDTHS: ConsoleColumnWidths = {
  timeWidth: 160,
  sourceWidth: 120,
  moduleWidth: 460,
};

const notificationStore = useNotificationStore();
const entries = ref<DebugConsoleEntry[]>(getDebugConsoleSnapshot());
const debugEnabled = ref(false);
const autoScroll = ref(true);
const isExporting = ref(false);
const levelFilter = ref<LevelFilter>("all");
const sourceFilter = ref<SourceFilter>("all");
const searchQuery = ref("");
const listRef = ref<HTMLElement | null>(null);
const expandedEntryIds = ref<Set<string>>(new Set());
const activeResizeColumn = ref<ResizableConsoleColumn | null>(null);
const columnWidths = ref<ConsoleColumnWidths>(loadStoredColumnWidths());

const levelOptions = computed(() => [
  { value: "all", label: t("settings.console.level.all") },
  { value: "trace", label: t("settings.console.level.trace") },
  { value: "debug", label: t("settings.console.level.debug") },
  { value: "info", label: t("settings.console.level.info") },
  { value: "warn", label: t("settings.console.level.warn") },
  { value: "error", label: t("settings.console.level.error") },
]);

const sourceOptions = computed(() => [
  { value: "all", label: t("settings.console.source.all") },
  { value: "backend", label: t("settings.console.source.backend") },
  { value: "frontend", label: t("settings.console.source.frontend") },
]);

const highlightPattern = computed<RegExp | null>(() => {
  const query = searchQuery.value.trim();
  if (!query) return null;
  return new RegExp(escapeRegExp(query), "gi");
});

const filteredEntries = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  return entries.value
    .map((entry, index) => ({ entry, index }))
    .filter(({ entry }) => {
      if (levelFilter.value !== "all" && entry.level !== levelFilter.value) return false;
      if (sourceFilter.value !== "all" && entry.source !== sourceFilter.value) return false;
      if (!query) return true;
      const visibleSource = formatSource(entry.source).toLowerCase();
      return (
        entry.module.toLowerCase().includes(query) ||
        entry.message.toLowerCase().includes(query) ||
        entry.level.toLowerCase().includes(query) ||
        entry.source.toLowerCase().includes(query) ||
        visibleSource.includes(query)
      );
    })
    .sort((left, right) =>
      right.entry.timestampMs - left.entry.timestampMs || right.index - left.index,
    )
    .map(({ entry }) => entry);
});

const statusLabel = computed(() =>
  debugEnabled.value
    ? t("settings.console.status.debugOn")
    : t("settings.console.status.debugOff"),
);

const countLabel = computed(() =>
  t("settings.console.count", String(filteredEntries.value.length)),
);

const consoleGridStyle = computed(() => ({
  "--console-time-width": `${columnWidths.value.timeWidth}px`,
  "--console-source-width": `${columnWidths.value.sourceWidth}px`,
  "--console-module-width": `${columnWidths.value.moduleWidth}px`,
  "--console-message-min-width": `${CONSOLE_MESSAGE_MIN_WIDTH}px`,
  "--console-message-max-height": `${CONSOLE_MESSAGE_MAX_HEIGHT}px`,
}));

function clampColumnWidth(column: ResizableConsoleColumn, value: number): number {
  const min = MIN_COLUMN_WIDTHS[column];
  const max = MAX_COLUMN_WIDTHS[column];
  return Math.min(max, Math.max(min, Math.round(value)));
}

function loadStoredColumnWidths(): ConsoleColumnWidths {
  try {
    const raw = localStorage.getItem(CONSOLE_COLUMN_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_COLUMN_WIDTHS };
    const parsed = JSON.parse(raw) as Record<string, unknown> | null;
    if (!parsed || typeof parsed !== "object") return { ...DEFAULT_COLUMN_WIDTHS };

    return {
      timeWidth: typeof parsed.timeWidth === "number"
        ? clampColumnWidth("timeWidth", parsed.timeWidth)
        : DEFAULT_COLUMN_WIDTHS.timeWidth,
      sourceWidth: typeof parsed.sourceWidth === "number"
        ? clampColumnWidth("sourceWidth", parsed.sourceWidth)
        : DEFAULT_COLUMN_WIDTHS.sourceWidth,
      moduleWidth: typeof parsed.moduleWidth === "number"
        ? clampColumnWidth("moduleWidth", parsed.moduleWidth)
        : DEFAULT_COLUMN_WIDTHS.moduleWidth,
    };
  } catch {
    return { ...DEFAULT_COLUMN_WIDTHS };
  }
}

function persistColumnWidths() {
  try {
    localStorage.setItem(CONSOLE_COLUMN_STORAGE_KEY, JSON.stringify(columnWidths.value));
  } catch {
    // ignore persistence failures
  }
}

function setColumnWidth(column: ResizableConsoleColumn, width: number) {
  columnWidths.value = {
    ...columnWidths.value,
    [column]: clampColumnWidth(column, width),
  };
}

function nudgeColumnWidth(column: ResizableConsoleColumn, delta: number) {
  setColumnWidth(column, columnWidths.value[column] + delta);
  persistColumnWidths();
}

function formatSource(source: DebugConsoleEntry["source"]): string {
  return source === "backend"
    ? t("settings.console.source.backend")
    : t("settings.console.source.frontend");
}

function formatTime(timestampMs: number): string {
  const date = new Date(timestampMs);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${hours}:${minutes}:${seconds}`;
}

function defaultExportFileName(): string {
  const date = new Date();
  const year = String(date.getFullYear());
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `locus-console-${year}${month}${day}-${hours}${minutes}${seconds}.log`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function highlightSegments(text: string): HighlightSegment[] {
  const pattern = highlightPattern.value;
  if (!pattern || !text) return [{ text, hit: false }];

  const result: HighlightSegment[] = [];
  let lastIndex = 0;
  pattern.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      result.push({ text: text.slice(lastIndex, match.index), hit: false });
    }
    result.push({ text: match[0], hit: true });
    lastIndex = match.index + match[0].length;
    if (match[0].length === 0) pattern.lastIndex++;
  }

  if (lastIndex < text.length) {
    result.push({ text: text.slice(lastIndex), hit: false });
  }
  return result.length > 0 ? result : [{ text, hit: false }];
}

function scrollToLatest() {
  if (!listRef.value) return;
  listRef.value.scrollTop = 0;
}

function isMessageLong(entry: DebugConsoleEntry): boolean {
  return entry.message.length > CONSOLE_MESSAGE_PREVIEW_LIMIT;
}

function isMessageExpanded(entryId: string): boolean {
  return expandedEntryIds.value.has(entryId);
}

function displayMessage(entry: DebugConsoleEntry): string {
  if (!isMessageLong(entry) || isMessageExpanded(entry.id)) {
    return entry.message;
  }
  return entry.message.slice(0, CONSOLE_MESSAGE_PREVIEW_LIMIT);
}

function hiddenMessageChars(entry: DebugConsoleEntry): string {
  return String(Math.max(0, entry.message.length - CONSOLE_MESSAGE_PREVIEW_LIMIT));
}

function toggleMessageExpanded(entryId: string) {
  const next = new Set(expandedEntryIds.value);
  if (next.has(entryId)) {
    next.delete(entryId);
  } else {
    next.add(entryId);
  }
  expandedEntryIds.value = next;
}

let columnResizeMoveHandler: ((event: MouseEvent) => void) | null = null;
let columnResizeUpHandler: (() => void) | null = null;
let releaseColumnResizeSelectionLock: (() => void) | null = null;

function stopColumnResize(shouldPersist = true) {
  activeResizeColumn.value = null;
  if (columnResizeMoveHandler) {
    document.removeEventListener("mousemove", columnResizeMoveHandler);
    columnResizeMoveHandler = null;
  }
  if (columnResizeUpHandler) {
    document.removeEventListener("mouseup", columnResizeUpHandler);
    columnResizeUpHandler = null;
  }
  document.body.style.cursor = "";
  releaseColumnResizeSelectionLock?.();
  releaseColumnResizeSelectionLock = null;
  if (shouldPersist) {
    persistColumnWidths();
  }
}

function onColumnResizeStart(event: MouseEvent, column: ResizableConsoleColumn) {
  event.preventDefault();
  event.stopPropagation();
  stopColumnResize(false);

  activeResizeColumn.value = column;
  const startX = event.clientX;
  const startWidth = columnWidths.value[column];

  columnResizeMoveHandler = (nextEvent: MouseEvent) => {
    if (activeResizeColumn.value !== column) return;
    setColumnWidth(column, startWidth + nextEvent.clientX - startX);
  };

  columnResizeUpHandler = () => {
    stopColumnResize(true);
  };

  document.addEventListener("mousemove", columnResizeMoveHandler);
  document.addEventListener("mouseup", columnResizeUpHandler);
  document.body.style.cursor = "col-resize";
  releaseColumnResizeSelectionLock?.();
  releaseColumnResizeSelectionLock = acquireSelectionLock();
}

async function syncDebugMode() {
  try {
    debugEnabled.value = await getDebugMode();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", normalized.message, {
      code: normalized.code,
      operation: "loadDebugMode",
    });
  }
}

async function refreshAll() {
  try {
    await initDebugConsole();
    await refreshDebugConsole();
    entries.value = getDebugConsoleSnapshot();
    await syncDebugMode();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("settings.console.refreshFailed", normalized.message), {
      code: normalized.code,
      operation: "refreshDebugConsole",
    });
  }
}

async function clearAll() {
  try {
    await clearDebugConsole();
    entries.value = getDebugConsoleSnapshot();
    expandedEntryIds.value = new Set();
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("settings.console.clearFailed", normalized.message), {
      code: normalized.code,
      operation: "clearDebugConsole",
    });
  }
}

async function exportLogs() {
  const snapshot = entries.value.slice();
  if (snapshot.length === 0 || isExporting.value) return;

  try {
    isExporting.value = true;
    const filePath = await save({
      defaultPath: defaultExportFileName(),
      filters: [{ name: "Log", extensions: ["log"] }],
    });
    if (!filePath) return;

    const savedPath = await saveDebugConsoleLogExport(filePath, snapshot);
    notificationStore.addNotice("success", t("settings.console.exported", savedPath), {
      operation: "exportDebugConsole",
      skipConsoleLog: true,
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("settings.console.exportFailed", normalized.message), {
      code: normalized.code,
      operation: "exportDebugConsole",
    });
  } finally {
    isExporting.value = false;
  }
}

let unsubscribe: (() => void) | null = null;

onMounted(async () => {
  unsubscribe = subscribeDebugConsole(() => {
    entries.value = getDebugConsoleSnapshot();
  });
  await refreshAll();
});

onUnmounted(() => {
  unsubscribe?.();
  unsubscribe = null;
  stopColumnResize(false);
});

watch(
  () => [filteredEntries.value[0]?.id ?? "", filteredEntries.value.length],
  async () => {
    if (!autoScroll.value) return;
    await nextTick();
    scrollToLatest();
  },
);
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.console.title") }}</div>
    <p class="section-desc">{{ t("settings.console.desc") }}</p>

    <div class="console-panel">
      <div class="console-toolbar">
        <BaseSegmented
          :model-value="levelFilter"
          :options="levelOptions"
          size="sm"
          @update:model-value="levelFilter = $event as LevelFilter"
        />
        <BaseSegmented
          :model-value="sourceFilter"
          :options="sourceOptions"
          size="sm"
          @update:model-value="sourceFilter = $event as SourceFilter"
        />
        <label class="console-toggle">
          <BaseSwitch
            :model-value="autoScroll"
            :aria-label="t('settings.console.autoscroll')"
            @update:model-value="autoScroll = $event"
          />
          <span>{{ t("settings.console.autoscroll") }}</span>
        </label>
        <input
          v-model="searchQuery"
          class="console-search"
          type="text"
          :placeholder="t('settings.console.searchPlaceholder')"
        />
        <BaseButton class="console-action" size="sm" @click="refreshAll">
          {{ t("common.refresh") }}
        </BaseButton>
        <BaseButton
          class="console-action"
          size="sm"
          :disabled="entries.length === 0 || isExporting"
          @click="exportLogs"
        >
          {{ isExporting ? t("settings.console.exporting") : t("settings.console.export") }}
        </BaseButton>
        <BaseButton class="console-action" size="sm" @click="clearAll">
          {{ t("settings.console.clear") }}
        </BaseButton>
      </div>

      <div class="console-meta">
        <span>{{ countLabel }}</span>
        <span>{{ statusLabel }}</span>
      </div>

      <div ref="listRef" class="console-list">
        <div class="console-grid" :style="consoleGridStyle">
          <div class="console-header">
            <div class="console-header-cell console-header-cell-resizable">
              <span class="console-header-label">{{ t("settings.console.column.time") }}</span>
              <button
                type="button"
                class="console-column-handle"
                :class="{ dragging: activeResizeColumn === 'timeWidth' }"
                :aria-label="t('settings.console.column.time')"
                :title="t('settings.console.column.time')"
                @mousedown="onColumnResizeStart($event, 'timeWidth')"
                @keydown.left.prevent="nudgeColumnWidth('timeWidth', -12)"
                @keydown.right.prevent="nudgeColumnWidth('timeWidth', 12)"
              />
            </div>
            <div class="console-header-cell console-header-cell-resizable">
              <span class="console-header-label">{{ t("settings.console.column.source") }}</span>
              <button
                type="button"
                class="console-column-handle"
                :class="{ dragging: activeResizeColumn === 'sourceWidth' }"
                :aria-label="t('settings.console.column.source')"
                :title="t('settings.console.column.source')"
                @mousedown="onColumnResizeStart($event, 'sourceWidth')"
                @keydown.left.prevent="nudgeColumnWidth('sourceWidth', -12)"
                @keydown.right.prevent="nudgeColumnWidth('sourceWidth', 12)"
              />
            </div>
            <div class="console-header-cell console-header-cell-resizable">
              <span class="console-header-label">{{ t("settings.console.column.module") }}</span>
              <button
                type="button"
                class="console-column-handle"
                :class="{ dragging: activeResizeColumn === 'moduleWidth' }"
                :aria-label="t('settings.console.column.module')"
                :title="t('settings.console.column.module')"
                @mousedown="onColumnResizeStart($event, 'moduleWidth')"
                @keydown.left.prevent="nudgeColumnWidth('moduleWidth', -12)"
                @keydown.right.prevent="nudgeColumnWidth('moduleWidth', 12)"
              />
            </div>
            <div class="console-header-cell">
              <span class="console-header-label">{{ t("settings.console.column.message") }}</span>
            </div>
          </div>

          <div v-if="filteredEntries.length === 0" class="console-empty">
            {{ t("settings.console.empty") }}
          </div>
          <div
            v-for="entry in filteredEntries"
            :key="entry.id"
            class="console-row"
            :class="`level-${entry.level}`"
          >
            <span class="console-time">{{ formatTime(entry.timestampMs) }}</span>
            <span class="console-source">
              <template
                v-for="(segment, segmentIndex) in highlightSegments(formatSource(entry.source))"
                :key="segmentIndex"
              >
                <mark v-if="segment.hit" class="console-search-hit">{{ segment.text }}</mark>
                <template v-else>{{ segment.text }}</template>
              </template>
            </span>
            <span class="console-module" :title="entry.module">
              <template
                v-for="(segment, segmentIndex) in highlightSegments(entry.module)"
                :key="segmentIndex"
              >
                <mark v-if="segment.hit" class="console-search-hit">{{ segment.text }}</mark>
                <template v-else>{{ segment.text }}</template>
              </template>
            </span>
            <div class="console-message-cell">
              <pre class="console-message"><template
                v-for="(segment, segmentIndex) in highlightSegments(displayMessage(entry))"
                :key="segmentIndex"
              ><mark v-if="segment.hit" class="console-search-hit">{{ segment.text }}</mark><template v-else>{{ segment.text }}</template></template></pre>
              <div v-if="isMessageLong(entry)" class="console-message-meta">
                <span v-if="!isMessageExpanded(entry.id)">
                  {{ t("settings.console.hiddenChars", hiddenMessageChars(entry)) }}
                </span>
                <button
                  type="button"
                  class="console-message-toggle"
                  @click="toggleMessageExpanded(entry.id)"
                >
                  {{
                    isMessageExpanded(entry.id)
                      ? t("settings.console.collapseMessage")
                      : t("settings.console.expandMessage")
                  }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.console-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 520px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
  padding: 14px 16px;
}

.console-toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.console-toggle {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
  color: var(--text-secondary);
  font-size: 12px;
}

.console-search {
  min-width: 220px;
  flex: 1;
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 12px;
  outline: none;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.console-search:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.console-action {
  font-size: 11px;
}

.console-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 11px;
  color: var(--text-secondary);
}

.console-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--bg-color) 28%);
  scrollbar-gutter: stable;
}

.console-grid {
  min-width: calc(
    var(--console-time-width)
    + var(--console-source-width)
    + var(--console-module-width)
    + var(--console-message-min-width)
  );
}

.console-header,
.console-row {
  display: grid;
  grid-template-columns:
    var(--console-time-width)
    var(--console-source-width)
    var(--console-module-width)
    minmax(var(--console-message-min-width), 1fr);
  gap: 10px;
  align-items: start;
  padding: 8px 12px;
  font-family: var(--font-mono-editor);
  font-size: 12px;
  line-height: 1.45;
}

.console-header {
  position: sticky;
  top: 0;
  z-index: 2;
  align-items: center;
  padding-top: 7px;
  padding-bottom: 7px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.console-header-cell {
  min-width: 0;
  color: var(--text-secondary);
}

.console-header-cell-resizable {
  position: relative;
  padding-right: 10px;
}

.console-header-label {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.console-column-handle {
  position: absolute;
  top: -5px;
  right: -6px;
  bottom: -5px;
  width: 12px;
  border: none;
  padding: 0;
  background: transparent;
  cursor: col-resize;
}

.console-column-handle::before {
  content: "";
  position: absolute;
  top: 5px;
  bottom: 5px;
  left: 5px;
  width: 2px;
  border-radius: 999px;
  background: transparent;
  transition: background 0.15s ease;
}

.console-column-handle:hover::before,
.console-column-handle.dragging::before {
  background: color-mix(in srgb, var(--accent-color) 46%, transparent);
}

.console-column-handle:focus-visible {
  outline: none;
}

.console-column-handle:focus-visible::before {
  background: color-mix(in srgb, var(--accent-color) 60%, transparent);
}

.console-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 220px;
  color: var(--text-secondary);
  font-size: 12px;
}

.console-row {
  position: relative;
  border-bottom: 1px solid var(--border-color);
  background: transparent;
  transition: background 0.12s ease;
}

.console-row:hover {
  background: var(--hover-bg);
}

.console-row:last-child {
  border-bottom: none;
}

.console-row.level-trace,
.console-row.level-debug {
  color: var(--text-secondary);
}

.console-row.level-warn {
  background: color-mix(in srgb, var(--status-warn-bg) 26%, transparent);
  box-shadow: inset 2px 0 var(--status-warn-border);
}

.console-row.level-error {
  background: color-mix(in srgb, var(--status-danger-bg) 30%, transparent);
  box-shadow: inset 2px 0 var(--status-danger-border);
}

.console-time,
.console-source,
.console-module {
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-variant-numeric: tabular-nums;
}

.console-module {
  color: var(--text-color);
}

.console-message-cell {
  min-width: 0;
}

.console-message {
  margin: 0;
  min-width: 0;
  max-height: var(--console-message-max-height);
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  word-break: break-word;
  color: var(--text-color);
  font: inherit;
  tab-size: 2;
}

.console-search-hit {
  border-radius: 2px;
  background: color-mix(in srgb, var(--accent-color) 38%, transparent);
  color: inherit;
  padding: 0 2px;
}

.console-message-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
}

.console-message-toggle {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color);
  font: inherit;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.console-message-toggle:hover {
  color: var(--text-color);
}

.console-message-toggle:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 2px;
  border-radius: 3px;
}
</style>
