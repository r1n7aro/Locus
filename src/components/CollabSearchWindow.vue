<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emitTo } from "@tauri-apps/api/event";
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  Search,
  SlidersHorizontal,
  X,
} from "lucide";
import type { GitHistorySearchResult } from "../types";
import { gitHistorySearch } from "../services/git";
import {
  COLLAB_SEARCH_SELECT_EVENT,
  type CollabSearchSelectionPayload,
} from "../services/collabSearchWindow";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../composables/resizeObserver";
import { normalizeAppError } from "../services/errors";
import { locale, t } from "../i18n";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
} from "./icons/unityAssetIcons";
import LucideIcon from "./icons/LucideIcon.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";

type DatePickerTarget = "from" | "to";
type ResultColumnKey = "kind" | "message" | "ref" | "author" | "date" | "files";
type ResizableResultColumn = Exclude<ResultColumnKey, "files">;
type CalendarWeekdayKey = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

interface CalendarDay {
  key: string;
  value: string;
  day: number;
  currentMonth: boolean;
  today: boolean;
  selected: boolean;
}

const RESULT_COLUMN_STORAGE_KEY = "locus.collabSearch.resultColumns.v4";
const RESULT_COLUMN_KEYS: ResultColumnKey[] = ["kind", "message", "ref", "author", "date", "files"];
const RESULT_AUTO_FIT_COLUMNS: ResultColumnKey[] = ["message", "ref", "author", "date"];
const RESULT_ROW_HEIGHT = 38;
const RESULT_ROW_BUFFER = 12;
const RESULT_COLUMN_DEFAULT_WIDTHS: Record<ResultColumnKey, number> = {
  kind: 58,
  message: 300,
  ref: 86,
  author: 110,
  date: 96,
  files: 360,
};
const RESULT_COLUMN_MIN_WIDTHS: Record<ResultColumnKey, number> = {
  kind: 58,
  message: 96,
  ref: 72,
  author: 84,
  date: 86,
  files: 180,
};
const RESULT_COLUMN_MAX_WIDTHS: Record<ResultColumnKey, number> = {
  kind: 82,
  message: 760,
  ref: 180,
  author: 260,
  date: 180,
  files: 960,
};
const CALENDAR_WEEKDAY_KEYS: CalendarWeekdayKey[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

const appWindow = getCurrentWindow();
const query = ref("");
const useRegex = ref(false);
const author = ref("");
const dateFrom = ref("");
const dateTo = ref("");
const loading = ref(false);
const searched = ref(false);
const error = ref("");
const results = ref<GitHistorySearchResult[]>([]);
const truncated = ref(false);
const selectingHash = ref<string | null>(null);
const queryInputRef = ref<HTMLInputElement | null>(null);
const filterRef = ref<HTMLElement | null>(null);
const resultScrollRef = ref<HTMLElement | null>(null);
const resultTableRef = ref<HTMLElement | null>(null);
const resultHeaderRef = ref<HTMLElement | null>(null);
const filterOpen = ref(false);
const activeDatePicker = ref<DatePickerTarget | null>(null);
const calendarMonth = ref(startOfMonthDate(new Date()));
const resultColumnWidths = ref<Record<ResultColumnKey, number>>(loadStoredResultColumnWidths());
const activeResizeColumn = ref<ResizableResultColumn | null>(null);
const resultScrollTop = ref(0);
const resultViewportHeight = ref(0);
const resultHeaderHeight = ref(30);

let columnResizeMoveHandler: ((event: MouseEvent) => void) | null = null;
let columnResizeUpHandler: ((event: MouseEvent) => void) | null = null;
let releaseColumnResizeSelectionLock: (() => void) | null = null;
let previousBodyCursor = "";
let measureContext: CanvasRenderingContext2D | null = null;
let pendingColumnResizeFrame = 0;
let pendingColumnResizeWidths: Record<ResultColumnKey, number> | null = null;
let resultScrollFrame = 0;
let resultResizeObserver: ResizeObserverHandle | null = null;

const canSearch = computed(() =>
  !!query.value.trim()
  || !!author.value.trim()
  || !!dateFrom.value
  || !!dateTo.value,
);
const activeFilterCount = computed(() =>
  [
    author.value.trim(),
    dateFrom.value,
    dateTo.value,
    useRegex.value ? "regex" : "",
  ].filter(Boolean).length,
);

const resultCountLabel = computed(() => {
  if (loading.value) return t("common.loading");
  return t("collab.search.resultCount", results.value.length);
});

const showSearchStatus = computed(() => searched.value || loading.value || truncated.value);

const resultGridStyle = computed<Record<string, string>>(() => {
  const widths = resultColumnWidths.value;
  return {
    "--collab-search-grid-columns": resultGridColumns(widths),
    "--collab-search-grid-min-width": `${resultGridMinWidth(widths)}px`,
  };
});

const virtualResultRows = computed(() => {
  const totalRows = results.value.length;
  if (totalRows === 0) {
    return {
      rows: [] as Array<{ result: GitHistorySearchResult; top: number }>,
      totalHeight: 0,
    };
  }

  if (resultViewportHeight.value <= 0) {
    return {
      rows: results.value.map((result, index) => ({
        result,
        top: index * RESULT_ROW_HEIGHT,
      })),
      totalHeight: totalRows * RESULT_ROW_HEIGHT,
    };
  }

  const bodyScrollTop = Math.max(0, resultScrollTop.value - resultHeaderHeight.value);
  const visibleStart = Math.max(0, Math.floor(bodyScrollTop / RESULT_ROW_HEIGHT));
  const visibleEnd = Math.min(
    totalRows - 1,
    Math.ceil((bodyScrollTop + resultViewportHeight.value) / RESULT_ROW_HEIGHT) - 1,
  );
  const start = Math.max(0, visibleStart - RESULT_ROW_BUFFER);
  const end = Math.max(start, Math.min(totalRows - 1, visibleEnd + RESULT_ROW_BUFFER));
  return {
    rows: results.value.slice(start, end + 1).map((result, index) => ({
      result,
      top: (start + index) * RESULT_ROW_HEIGHT,
    })),
    totalHeight: totalRows * RESULT_ROW_HEIGHT,
  };
});

const virtualResultSpacerStyle = computed(() => ({
  height: `${virtualResultRows.value.totalHeight}px`,
}));

const weekdayLabels = computed(() =>
  CALENDAR_WEEKDAY_KEYS.map((day) => t(`collab.search.weekday.${day}`)),
);

const selectedDateValue = computed(() => {
  if (activeDatePicker.value === "from") return dateFrom.value;
  if (activeDatePicker.value === "to") return dateTo.value;
  return "";
});

const calendarMonthLabel = computed(() => {
  const date = calendarMonth.value;
  const month = locale.value === "zh"
    ? String(date.getMonth() + 1)
    : new Intl.DateTimeFormat("en-US", { month: "long" }).format(date);
  return t("collab.search.monthYear", date.getFullYear(), month);
});

const calendarDays = computed<CalendarDay[]>(() => {
  const month = calendarMonth.value;
  const firstDay = new Date(month.getFullYear(), month.getMonth(), 1);
  const mondayOffset = (firstDay.getDay() + 6) % 7;
  const gridStart = new Date(firstDay);
  gridStart.setDate(firstDay.getDate() - mondayOffset);
  const todayValue = formatDateValue(new Date());
  const selected = selectedDateValue.value;

  return Array.from({ length: 42 }, (_, index) => {
    const date = new Date(gridStart);
    date.setDate(gridStart.getDate() + index);
    const value = formatDateValue(date);
    return {
      key: value,
      value,
      day: date.getDate(),
      currentMonth: date.getMonth() === month.getMonth(),
      today: value === todayValue,
      selected: value === selected,
    };
  });
});

function parseDateValue(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year
    || date.getMonth() !== month - 1
    || date.getDate() !== day
  ) {
    return null;
  }
  return date;
}

function formatDateValue(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function startOfMonthDate(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), 1);
}

function clampColumnWidth(column: ResultColumnKey, width: number): number {
  return Math.min(
    RESULT_COLUMN_MAX_WIDTHS[column],
    Math.max(RESULT_COLUMN_MIN_WIDTHS[column], Math.round(width)),
  );
}

function resultGridColumns(widths: Record<ResultColumnKey, number>): string {
  return [
    `${widths.kind}px`,
    `${widths.message}px`,
    `${widths.ref}px`,
    `${widths.author}px`,
    `${widths.date}px`,
    `minmax(${widths.files}px, 1fr)`,
  ].join(" ");
}

function resultGridMinWidth(widths: Record<ResultColumnKey, number>): number {
  const columnWidth = RESULT_COLUMN_KEYS.reduce((total, column) => total + widths[column], 0);
  const columnGapWidth = (RESULT_COLUMN_KEYS.length - 1) * 10;
  const horizontalPaddingWidth = 28;
  return columnWidth + columnGapWidth + horizontalPaddingWidth;
}

function applyResultGridSizing(widths: Record<ResultColumnKey, number>) {
  const table = resultTableRef.value;
  if (!table) return;
  table.style.setProperty("--collab-search-grid-columns", resultGridColumns(widths));
  table.style.setProperty("--collab-search-grid-min-width", `${resultGridMinWidth(widths)}px`);
}

function flushScheduledResultGridSizing() {
  pendingColumnResizeFrame = 0;
  if (!pendingColumnResizeWidths) return;
  applyResultGridSizing(pendingColumnResizeWidths);
}

function scheduleResultGridSizing(widths: Record<ResultColumnKey, number>) {
  pendingColumnResizeWidths = widths;
  if (pendingColumnResizeFrame) return;
  if (typeof requestAnimationFrame !== "function") {
    flushScheduledResultGridSizing();
    return;
  }
  pendingColumnResizeFrame = requestAnimationFrame(flushScheduledResultGridSizing);
}

function cancelScheduledResultGridSizing() {
  if (pendingColumnResizeFrame && typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(pendingColumnResizeFrame);
  }
  pendingColumnResizeFrame = 0;
  pendingColumnResizeWidths = null;
}

function commitResultColumnWidths(widths: Record<ResultColumnKey, number>) {
  cancelScheduledResultGridSizing();
  applyResultGridSizing(widths);
  resultColumnWidths.value = widths;
  persistResultColumnWidths();
}

function loadStoredResultColumnWidths(): Record<ResultColumnKey, number> {
  try {
    const stored = localStorage.getItem(RESULT_COLUMN_STORAGE_KEY);
    if (!stored) return { ...RESULT_COLUMN_DEFAULT_WIDTHS };
    const parsed = JSON.parse(stored) as Partial<Record<ResultColumnKey, number>>;
    return RESULT_COLUMN_KEYS.reduce((widths, column) => {
      const value = parsed[column];
      widths[column] = clampColumnWidth(
        column,
        typeof value === "number" ? value : RESULT_COLUMN_DEFAULT_WIDTHS[column],
      );
      return widths;
    }, {} as Record<ResultColumnKey, number>);
  } catch {
    return { ...RESULT_COLUMN_DEFAULT_WIDTHS };
  }
}

function persistResultColumnWidths() {
  try {
    localStorage.setItem(RESULT_COLUMN_STORAGE_KEY, JSON.stringify(resultColumnWidths.value));
  } catch {
    // ignore unavailable storage
  }
}

function dateStartTimestamp(value: string): number | null {
  if (!value) return null;
  const date = new Date(`${value}T00:00:00`);
  if (!Number.isFinite(date.getTime())) return null;
  return Math.floor(date.getTime() / 1000);
}

function dateEndTimestamp(value: string): number | null {
  if (!value) return null;
  const date = new Date(`${value}T00:00:00`);
  if (!Number.isFinite(date.getTime())) return null;
  return Math.floor((date.getTime() + 24 * 60 * 60 * 1000 - 1) / 1000);
}

function formatDate(timestamp: number): string {
  if (!timestamp) return "";
  const date = new Date(timestamp * 1000);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function kindLabel(kind: GitHistorySearchResult["kind"]): string {
  return kind === "stash" ? "stash" : "commit";
}

function kindClass(kind: GitHistorySearchResult["kind"]): string {
  return kind === "stash" ? "is-stash" : "is-commit";
}

function resultRef(result: GitHistorySearchResult): string {
  if (result.kind === "stash") return result.refName || result.shortHash;
  return result.shortHash;
}

function fileStatusLabel(status: string): string {
  return status || "M";
}

function fileStatusClass(status: string): string {
  switch (status) {
    case "A":
    case "?":
      return "status-added";
    case "D":
      return "status-deleted";
    case "R":
      return "status-renamed";
    case "M":
    default:
      return "status-modified";
  }
}

function fileAssetIconClass(file: GitHistorySearchResult["files"][number]): string[] {
  return [
    "collab-search-file-icon",
    unityAssetIconClassForPath(file.path, { isFolder: false }),
    "is-git-status-icon",
    fileStatusClass(file.status),
  ];
}

function fileDisplay(file: GitHistorySearchResult["files"][number]): string {
  return `${fileStatusLabel(file.status)} ${file.path}`;
}

function resultFilesTitle(result: GitHistorySearchResult): string {
  return result.files.map(fileDisplay).join("\n");
}

function rowTitle(result: GitHistorySearchResult): string {
  return `${kindLabel(result.kind)} ${resultRef(result)} ${result.message}`;
}

function resultColumnLabel(column: ResultColumnKey): string {
  return t(`collab.search.column.${column}`);
}

function resultColumnFont(column: ResultColumnKey): string {
  const styles = getComputedStyle(document.documentElement);
  const uiFont = styles.getPropertyValue("--font-ui").trim() || "system-ui, sans-serif";
  const monoFont = styles.getPropertyValue("--font-mono-identifier").trim() || "monospace";
  if (column === "message") return `600 13px ${uiFont}`;
  if (column === "ref") return `600 12px ${monoFont}`;
  return `12px ${uiFont}`;
}

function measureTextWidth(text: string, font: string): number {
  if (typeof document === "undefined") return text.length * 8;
  if (!measureContext) {
    measureContext = document.createElement("canvas").getContext("2d");
  }
  if (!measureContext) return text.length * 8;
  measureContext.font = font;
  return Math.ceil(measureContext.measureText(text || "").width);
}

function resultColumnValues(
  column: ResultColumnKey,
  searchResults: GitHistorySearchResult[],
): string[] {
  switch (column) {
    case "message":
      return searchResults.map((result) => result.message);
    case "ref":
      return searchResults.map(resultRef);
    case "author":
      return searchResults.map((result) => result.author);
    case "date":
      return searchResults.map((result) => formatDate(result.date));
    default:
      return [];
  }
}

function autoFitResultColumnWidths(searchResults: GitHistorySearchResult[]) {
  if (searchResults.length === 0) return;
  const nextWidths = {
    ...resultColumnWidths.value,
    kind: RESULT_COLUMN_DEFAULT_WIDTHS.kind,
  };

  RESULT_AUTO_FIT_COLUMNS.forEach((column) => {
    const values = [resultColumnLabel(column), ...resultColumnValues(column, searchResults)];
    const font = resultColumnFont(column);
    const contentWidth = Math.max(...values.map((value) => measureTextWidth(value, font)));
    const paddingWidth = column === "message" ? 28 : 24;
    nextWidths[column] = clampColumnWidth(column, contentWidth + paddingWidth);
  });

  resultColumnWidths.value = nextWidths;
}

function resultColumnResizeTitle(column: ResizableResultColumn): string {
  return t("collab.search.resizeColumn", resultColumnLabel(column));
}

function isResizableResultColumn(column: ResultColumnKey): column is ResizableResultColumn {
  return column !== "files";
}

function nextResultColumn(column: ResizableResultColumn): ResultColumnKey {
  return RESULT_COLUMN_KEYS[RESULT_COLUMN_KEYS.indexOf(column) + 1];
}

function pointerDeltaToPreferredDelta(delta: number): number {
  return delta;
}

function adjacentResultColumnWidths(
  column: ResizableResultColumn,
  nextColumn: ResultColumnKey,
  startColumnWidth: number,
  startNextColumnWidth: number,
  preferredDelta: number,
): Record<ResultColumnKey, number> {
  const minDelta = Math.max(
    RESULT_COLUMN_MIN_WIDTHS[column] - startColumnWidth,
    startNextColumnWidth - RESULT_COLUMN_MAX_WIDTHS[nextColumn],
  );
  const maxDelta = Math.min(
    RESULT_COLUMN_MAX_WIDTHS[column] - startColumnWidth,
    startNextColumnWidth - RESULT_COLUMN_MIN_WIDTHS[nextColumn],
  );
  const appliedDelta = Math.min(maxDelta, Math.max(minDelta, preferredDelta));
  return {
    ...resultColumnWidths.value,
    [column]: clampColumnWidth(column, startColumnWidth + appliedDelta),
    [nextColumn]: clampColumnWidth(nextColumn, startNextColumnWidth - appliedDelta),
  };
}

function nudgeResultColumnWidth(column: ResizableResultColumn, delta: number) {
  const nextColumn = nextResultColumn(column);
  commitResultColumnWidths(
    adjacentResultColumnWidths(
      column,
      nextColumn,
      resultColumnWidths.value[column],
      resultColumnWidths.value[nextColumn],
      delta,
    ),
  );
}

function stopResultColumnResize() {
  if (columnResizeMoveHandler) {
    document.removeEventListener("mousemove", columnResizeMoveHandler);
    columnResizeMoveHandler = null;
  }
  if (columnResizeUpHandler) {
    document.removeEventListener("mouseup", columnResizeUpHandler);
    columnResizeUpHandler = null;
  }
  if (activeResizeColumn.value) {
    document.body.style.cursor = previousBodyCursor;
  }
  releaseColumnResizeSelectionLock?.();
  releaseColumnResizeSelectionLock = null;
  activeResizeColumn.value = null;
  cancelScheduledResultGridSizing();
}

function onResultColumnResizeStart(event: MouseEvent, column: ResizableResultColumn) {
  event.preventDefault();
  event.stopPropagation();
  stopResultColumnResize();

  activeResizeColumn.value = column;
  const nextColumn = nextResultColumn(column);
  const startX = event.clientX;
  const startColumnWidth = resultColumnWidths.value[column];
  const startNextColumnWidth = resultColumnWidths.value[nextColumn];
  let latestWidths = resultColumnWidths.value;
  previousBodyCursor = document.body.style.cursor;
  document.body.style.cursor = "col-resize";
  releaseColumnResizeSelectionLock?.();
  releaseColumnResizeSelectionLock = acquireSelectionLock();

  columnResizeMoveHandler = (nextEvent: MouseEvent) => {
    if (activeResizeColumn.value !== column) return;
    latestWidths = adjacentResultColumnWidths(
      column,
      nextColumn,
      startColumnWidth,
      startNextColumnWidth,
      pointerDeltaToPreferredDelta(nextEvent.clientX - startX),
    );
    scheduleResultGridSizing(latestWidths);
  };
  columnResizeUpHandler = () => {
    commitResultColumnWidths(latestWidths);
    stopResultColumnResize();
  };

  document.addEventListener("mousemove", columnResizeMoveHandler);
  document.addEventListener("mouseup", columnResizeUpHandler);
}

function onResultColumnResizeKeydown(event: KeyboardEvent, column: ResizableResultColumn) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  event.stopPropagation();
  nudgeResultColumnWidth(column, event.key === "ArrowRight" ? 12 : -12);
}

function toggleFilterDropdown() {
  filterOpen.value = !filterOpen.value;
  if (!filterOpen.value) {
    closeDatePicker();
  }
}

function closeFilterDropdown() {
  filterOpen.value = false;
  closeDatePicker();
}

function clearFilterFields() {
  author.value = "";
  dateFrom.value = "";
  dateTo.value = "";
  useRegex.value = false;
  closeDatePicker();
}

function dateButtonLabel(value: string): string {
  return value ? value.replace(/-/g, "/") : t("collab.search.datePlaceholder");
}

function openDatePicker(target: DatePickerTarget) {
  activeDatePicker.value = target;
  const selectedDate = parseDateValue(target === "from" ? dateFrom.value : dateTo.value);
  const fallbackDate = parseDateValue(target === "from" ? dateTo.value : dateFrom.value);
  calendarMonth.value = startOfMonthDate(selectedDate ?? fallbackDate ?? new Date());
}

function toggleDatePicker(target: DatePickerTarget) {
  if (activeDatePicker.value === target) {
    closeDatePicker();
    return;
  }
  openDatePicker(target);
}

function closeDatePicker() {
  activeDatePicker.value = null;
}

function shiftCalendarMonth(delta: number) {
  const current = calendarMonth.value;
  calendarMonth.value = new Date(current.getFullYear(), current.getMonth() + delta, 1);
}

function setDateFilterValue(target: DatePickerTarget, value: string) {
  if (target === "from") {
    dateFrom.value = value;
    if (dateTo.value && value > dateTo.value) {
      dateTo.value = value;
    }
    return;
  }
  dateTo.value = value;
  if (dateFrom.value && value < dateFrom.value) {
    dateFrom.value = value;
  }
}

function selectCalendarDate(value: string) {
  if (!activeDatePicker.value) return;
  setDateFilterValue(activeDatePicker.value, value);
  closeDatePicker();
}

function clearDateFilter(target: DatePickerTarget) {
  if (target === "from") {
    dateFrom.value = "";
  } else {
    dateTo.value = "";
  }
  closeDatePicker();
}

function clearActiveDateFilter() {
  if (!activeDatePicker.value) return;
  clearDateFilter(activeDatePicker.value);
}

function onDocumentPointerDown(event: PointerEvent) {
  if (!filterOpen.value) return;
  if (filterRef.value?.contains(event.target as Node)) return;
  closeFilterDropdown();
}

function syncResultViewport() {
  const scroll = resultScrollRef.value;
  resultScrollTop.value = scroll?.scrollTop ?? 0;
  resultViewportHeight.value = scroll?.clientHeight ?? 0;
  resultHeaderHeight.value = resultHeaderRef.value?.offsetHeight ?? 30;
}

function onResultScroll() {
  if (resultScrollFrame) return;
  resultScrollFrame = requestAnimationFrame(() => {
    resultScrollFrame = 0;
    syncResultViewport();
  });
}

function reconnectResultResizeObserver() {
  resultResizeObserver?.disconnect();
  if (!resultResizeObserver) return;
  if (resultScrollRef.value) resultResizeObserver.observe(resultScrollRef.value);
  if (resultHeaderRef.value) resultResizeObserver.observe(resultHeaderRef.value);
}

function resetResultScrollTop() {
  if (resultScrollRef.value) {
    resultScrollRef.value.scrollTop = 0;
  }
  resultScrollTop.value = 0;
}

async function runSearch() {
  if (loading.value || !canSearch.value) return;
  closeFilterDropdown();
  loading.value = true;
  searched.value = true;
  error.value = "";
  try {
    const response = await gitHistorySearch({
      query: query.value,
      useRegex: useRegex.value,
      author: author.value,
      dateFrom: dateStartTimestamp(dateFrom.value),
      dateTo: dateEndTimestamp(dateTo.value),
    });
    results.value = response.results;
    resetResultScrollTop();
    autoFitResultColumnWidths(response.results);
    void nextTick(syncResultViewport);
    truncated.value = response.truncated;
    if (!response.isRepo) {
      error.value = t("collab.notVcs");
    }
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
    results.value = [];
    truncated.value = false;
  } finally {
    loading.value = false;
  }
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through
  }
  await appWindow.destroy().catch(() => {});
}

async function selectResult(result: GitHistorySearchResult) {
  if (selectingHash.value) return;
  selectingHash.value = result.hash;
  const payload: CollabSearchSelectionPayload = {
    kind: result.kind,
    hash: result.hash,
  };
  try {
    await emitTo("main", COLLAB_SEARCH_SELECT_EVENT, payload);
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  } finally {
    selectingHash.value = null;
  }
}

onMounted(() => {
  queryInputRef.value?.focus();
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  if (typeof ResizeObserver !== "undefined") {
    resultResizeObserver = createAnimationFrameResizeObserver(syncResultViewport);
    reconnectResultResizeObserver();
  } else {
    window.addEventListener("resize", syncResultViewport);
  }
  syncResultViewport();
});

onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  stopResultColumnResize();
  if (resultScrollFrame) {
    cancelAnimationFrame(resultScrollFrame);
    resultScrollFrame = 0;
  }
  resultResizeObserver?.disconnect();
  resultResizeObserver = null;
  window.removeEventListener("resize", syncResultViewport);
});

watch([resultScrollRef, resultHeaderRef], () => {
  reconnectResultResizeObserver();
  void nextTick(syncResultViewport);
}, { flush: "post" });

watch(results, () => {
  void nextTick(syncResultViewport);
}, { flush: "post" });
</script>

<template>
  <div class="collab-search-window-root">
    <div class="collab-search-titlebar">
      <div class="collab-search-title">{{ t("collab.search.title") }}</div>
      <button
        class="collab-search-close"
        type="button"
        :aria-label="t('common.close')"
        :title="t('common.close')"
        @click="void closeWindow()"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <form class="collab-search-toolbar" @submit.prevent="runSearch" @keydown.esc="closeFilterDropdown">
      <label class="collab-search-field collab-search-query">
        <input
          ref="queryInputRef"
          v-model="query"
          type="text"
          :aria-label="t('collab.search.file')"
          :placeholder="t('collab.search.filePlaceholder')"
        />
      </label>

      <div class="collab-search-actions">
        <div ref="filterRef" class="collab-search-filter">
          <button
            type="button"
            class="collab-search-filter-trigger"
            :class="{ active: filterOpen || activeFilterCount > 0 }"
            :aria-expanded="filterOpen"
            @click.stop="toggleFilterDropdown"
          >
            <LucideIcon :icon="SlidersHorizontal" :size="14" />
            <span>{{ t("collab.search.filter") }}</span>
            <span v-if="activeFilterCount > 0" class="collab-search-filter-count">{{ activeFilterCount }}</span>
          </button>

          <div v-if="filterOpen" class="collab-search-filter-menu">
            <label class="collab-search-field">
              <span>{{ t("collab.search.author") }}</span>
              <input
                v-model="author"
                type="text"
                :placeholder="t('collab.search.authorPlaceholder')"
              />
            </label>

            <div class="collab-search-option-row">
              <BaseCheckbox
                v-model="useRegex"
                :aria-label="t('collab.search.regex')"
              />
              <span>{{ t("collab.search.regex") }}</span>
            </div>

            <div class="collab-search-filter-dates">
              <div class="collab-search-field collab-search-date-field">
                <span>{{ t("collab.search.from") }}</span>
                <button
                  type="button"
                  class="collab-search-date-button"
                  :class="{ active: activeDatePicker === 'from', empty: !dateFrom }"
                  :aria-expanded="activeDatePicker === 'from'"
                  @click="toggleDatePicker('from')"
                >
                  <span>{{ dateButtonLabel(dateFrom) }}</span>
                  <LucideIcon :icon="CalendarDays" :size="13" />
                </button>
              </div>

              <div class="collab-search-field collab-search-date-field">
                <span>{{ t("collab.search.to") }}</span>
                <button
                  type="button"
                  class="collab-search-date-button"
                  :class="{ active: activeDatePicker === 'to', empty: !dateTo }"
                  :aria-expanded="activeDatePicker === 'to'"
                  @click="toggleDatePicker('to')"
                >
                  <span>{{ dateButtonLabel(dateTo) }}</span>
                  <LucideIcon :icon="CalendarDays" :size="13" />
                </button>
              </div>

              <div
                v-if="activeDatePicker"
                class="collab-search-date-popover"
                :class="{ 'align-end': activeDatePicker === 'to' }"
              >
                <div class="collab-search-calendar-header">
                  <button
                    type="button"
                    class="collab-search-calendar-nav"
                    :aria-label="t('collab.search.previousMonth')"
                    :title="t('collab.search.previousMonth')"
                    @click="shiftCalendarMonth(-1)"
                  >
                    <LucideIcon :icon="ChevronLeft" :size="14" />
                  </button>
                  <div class="collab-search-calendar-month">{{ calendarMonthLabel }}</div>
                  <button
                    type="button"
                    class="collab-search-calendar-nav"
                    :aria-label="t('collab.search.nextMonth')"
                    :title="t('collab.search.nextMonth')"
                    @click="shiftCalendarMonth(1)"
                  >
                    <LucideIcon :icon="ChevronRight" :size="14" />
                  </button>
                </div>

                <div class="collab-search-weekdays">
                  <span v-for="day in weekdayLabels" :key="day">{{ day }}</span>
                </div>

                <div class="collab-search-calendar-grid">
                  <button
                    v-for="day in calendarDays"
                    :key="day.key"
                    type="button"
                    class="collab-search-calendar-day"
                    :class="{
                      outside: !day.currentMonth,
                      today: day.today,
                      selected: day.selected,
                    }"
                    @click="selectCalendarDate(day.value)"
                  >
                    {{ day.day }}
                  </button>
                </div>

                <div class="collab-search-calendar-footer">
                  <BaseButton type="button" @click="clearActiveDateFilter">
                    {{ t("collab.search.clear") }}
                  </BaseButton>
                  <BaseButton type="button" @click="closeDatePicker">
                    {{ t("common.done") }}
                  </BaseButton>
                </div>
              </div>
            </div>

            <div class="collab-search-filter-footer">
              <BaseButton type="button" :disabled="activeFilterCount === 0" @click="clearFilterFields">
                {{ t("collab.search.clear") }}
              </BaseButton>
              <BaseButton type="button" @click="closeFilterDropdown">
                {{ t("common.done") }}
              </BaseButton>
            </div>
          </div>
        </div>

        <BaseButton type="submit" variant="primary" :disabled="loading || !canSearch">
          <LucideIcon :icon="Search" :size="14" />
          {{ loading ? t("common.loading") : t("collab.search.action") }}
        </BaseButton>
      </div>
    </form>

    <div v-if="showSearchStatus" class="collab-search-status">
      <span>{{ resultCountLabel }}</span>
      <span v-if="truncated" class="collab-search-truncated">
        {{ t("collab.search.truncated") }}
      </span>
    </div>

    <div ref="resultScrollRef" class="collab-search-body" @scroll="onResultScroll">
      <div v-if="error" class="collab-search-error">{{ error }}</div>
      <div v-else-if="!searched" class="collab-search-empty">
        {{ t("collab.search.emptyIdle") }}
      </div>
      <div v-else-if="loading" class="collab-search-empty">
        {{ t("common.loading") }}
      </div>
      <div v-else-if="results.length === 0" class="collab-search-empty">
        {{ t("collab.search.noResults") }}
      </div>
      <div v-else ref="resultTableRef" class="collab-search-results" :style="resultGridStyle">
        <div ref="resultHeaderRef" class="collab-search-result-header">
          <div
            v-for="column in RESULT_COLUMN_KEYS"
            :key="column"
            class="collab-search-result-header-cell"
            :class="{ resizable: isResizableResultColumn(column) }"
          >
            <span>{{ resultColumnLabel(column) }}</span>
            <button
              v-if="isResizableResultColumn(column)"
              type="button"
              class="collab-search-column-resize-handle"
              :class="{ dragging: activeResizeColumn === column }"
              :aria-label="resultColumnResizeTitle(column)"
              :title="resultColumnResizeTitle(column)"
              @click.stop
              @mousedown="onResultColumnResizeStart($event, column)"
              @keydown="onResultColumnResizeKeydown($event, column)"
            ></button>
          </div>
        </div>
        <div class="collab-search-result-spacer" :style="virtualResultSpacerStyle">
          <button
            v-for="{ result, top } in virtualResultRows.rows"
            :key="`${result.kind}:${result.hash}`"
            type="button"
            class="collab-search-result"
            :class="{ selecting: selectingHash === result.hash }"
            :style="{ transform: `translateY(${top}px)` }"
            :title="rowTitle(result)"
            @click="void selectResult(result)"
          >
            <span class="collab-search-kind" :class="kindClass(result.kind)">{{ kindLabel(result.kind) }}</span>
            <span class="collab-search-message">{{ result.message }}</span>
            <span class="collab-search-ref">{{ resultRef(result) }}</span>
            <span class="collab-search-author">{{ result.author }}</span>
            <span class="collab-search-date-text">{{ formatDate(result.date) }}</span>
            <span class="collab-search-files" :title="resultFilesTitle(result)">
              <span
                v-for="file in result.files.slice(0, 2)"
                :key="`${result.hash}:${file.status}:${file.path}`"
                class="collab-search-file"
              >
                <LucideIcon
                  :icon="unityAssetIconNodeForPath(file.path, { isFolder: false })"
                  :class="fileAssetIconClass(file)"
                  :size="13"
                />
                <span class="collab-search-file-path">{{ file.path }}</span>
              </span>
              <span v-if="result.files.length > 2" class="collab-search-file-more">
                {{ t("collab.search.moreFiles", result.files.length - 2) }}
              </span>
            </span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.collab-search-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.collab-search-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.collab-search-title {
  min-width: 0;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.collab-search-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.collab-search-close:hover {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
}

.collab-search-toolbar {
  flex-shrink: 0;
  display: grid;
  grid-template-columns: minmax(220px, 1fr) auto;
  gap: 10px;
  align-items: end;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.collab-search-field {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.collab-search-field span {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.collab-search-field input {
  width: 100%;
  min-height: 30px;
  box-sizing: border-box;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color));
  color: var(--text-color);
  font-size: 12px;
  outline: none;
}

.collab-search-field input:focus {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 16%, transparent);
}

.collab-search-date-button {
  width: 100%;
  min-height: 30px;
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color));
  color: var(--text-color);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  outline: none;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease, box-shadow 0.15s ease;
}

.collab-search-date-button span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: inherit;
  font-size: 12px;
  font-weight: 500;
}

.collab-search-date-button.empty {
  color: var(--text-secondary);
}

.collab-search-date-button:hover,
.collab-search-date-button.active,
.collab-search-date-button:focus-visible {
  border-color: var(--accent-color);
  color: var(--text-color);
}

.collab-search-date-button:focus-visible,
.collab-search-date-button.active {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 16%, transparent);
}

.collab-search-actions {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.collab-search-filter {
  position: relative;
}

.collab-search-filter-trigger {
  min-height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.collab-search-filter-trigger:hover,
.collab-search-filter-trigger.active {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.collab-search-filter-count {
  min-width: 16px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0 4px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent-color);
  font-size: 10px;
  line-height: 16px;
}

.collab-search-filter-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 20;
  width: min(420px, calc(100vw - 28px));
  padding: 12px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--panel-bg);
  box-shadow: 0 10px 28px color-mix(in srgb, var(--bg-color) 46%, transparent);
}

.collab-search-option-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 28px;
  margin-top: 10px;
  color: var(--text-color);
  font-size: 12px;
}

.collab-search-option-row span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.collab-search-filter-dates {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 10px;
  margin-top: 10px;
}

.collab-search-date-popover {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 30;
  width: 244px;
  box-sizing: border-box;
  padding: 10px;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--panel-bg);
  box-shadow: 0 10px 28px color-mix(in srgb, var(--bg-color) 46%, transparent);
}

.collab-search-date-popover.align-end {
  right: 0;
  left: auto;
}

.collab-search-calendar-header {
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr) 28px;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
}

.collab-search-calendar-month {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  text-align: center;
}

.collab-search-calendar-nav {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.collab-search-calendar-nav:hover,
.collab-search-calendar-nav:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.collab-search-weekdays,
.collab-search-calendar-grid {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  gap: 3px;
}

.collab-search-weekdays {
  margin-bottom: 4px;
}

.collab-search-weekdays span {
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 600;
}

.collab-search-calendar-day {
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font-size: 11px;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
}

.collab-search-calendar-day.outside {
  color: color-mix(in srgb, var(--text-secondary) 58%, transparent);
}

.collab-search-calendar-day.today {
  border-color: var(--accent-border);
}

.collab-search-calendar-day:hover,
.collab-search-calendar-day:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  outline: none;
}

.collab-search-calendar-day.selected {
  background: var(--accent-color);
  border-color: var(--accent-color);
  color: #fff;
}

.collab-search-calendar-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color);
}

.collab-search-filter-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}

.collab-search-status {
  min-height: 32px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 14px;
  border-bottom: 1px solid var(--border-color);
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 12px;
}

.collab-search-truncated {
  color: var(--status-warn-fg);
}

.collab-search-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--bg-color);
}

.collab-search-empty,
.collab-search-error {
  padding: 24px 16px;
  color: var(--text-secondary);
  font-size: 13px;
}

.collab-search-error {
  color: var(--status-danger-fg);
}

.collab-search-results {
  --collab-search-result-row-height: 38px;
  position: relative;
  display: flex;
  flex-direction: column;
  min-width: var(--collab-search-grid-min-width);
}

.collab-search-column-resize-handle {
  position: absolute;
  top: 0;
  right: -6px;
  bottom: 0;
  width: 12px;
  padding: 0;
  border: 0;
  background: transparent;
  cursor: col-resize;
  z-index: 2;
}

.collab-search-column-resize-handle::before {
  content: "";
  position: absolute;
  top: 6px;
  bottom: 6px;
  left: 5px;
  width: 1px;
  background: var(--border-color);
  opacity: 0.72;
  transition: background 0.12s ease, opacity 0.12s ease, width 0.12s ease;
}

.collab-search-column-resize-handle:hover::before,
.collab-search-column-resize-handle:focus-visible::before,
.collab-search-column-resize-handle.dragging::before {
  left: 4px;
  width: 3px;
  background: var(--accent-color);
  opacity: 0.82;
}

.collab-search-column-resize-handle:focus-visible {
  outline: none;
}

.collab-search-result-header {
  box-sizing: border-box;
  width: 100%;
  min-width: var(--collab-search-grid-min-width);
  min-height: 30px;
  display: grid;
  grid-template-columns: var(--collab-search-grid-columns);
  gap: 10px;
  align-items: center;
  padding: 0 14px;
  border-bottom: 1px solid var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--sidebar-bg) 18%);
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0;
  text-transform: uppercase;
  position: sticky;
  top: 0;
  z-index: 2;
}

.collab-search-result-header span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.collab-search-result-header-cell {
  position: relative;
  min-width: 0;
  height: 100%;
  display: flex;
  align-items: center;
}

.collab-search-result-header-cell.resizable {
  padding-right: 8px;
}

.collab-search-result-spacer {
  position: relative;
  flex: 0 0 auto;
  width: 100%;
  min-width: var(--collab-search-grid-min-width);
}

.collab-search-result {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  box-sizing: border-box;
  width: 100%;
  min-width: var(--collab-search-grid-min-width);
  height: var(--collab-search-result-row-height);
  min-height: var(--collab-search-result-row-height);
  display: grid;
  grid-template-columns: var(--collab-search-grid-columns);
  gap: 10px;
  align-items: center;
  padding: 0 14px;
  border: none;
  border-bottom: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  text-align: left;
  cursor: pointer;
  contain: layout paint style;
  transition: background 0.12s ease;
}

.collab-search-result:hover,
.collab-search-result:focus-visible {
  background: var(--hover-bg);
  outline: none;
}

.collab-search-result.selecting {
  background: color-mix(in srgb, var(--active-bg) 70%, transparent);
}

.collab-search-files,
.collab-search-file {
  min-width: 0;
  display: flex;
  align-items: center;
}

.collab-search-kind {
  box-sizing: border-box;
  flex-shrink: 0;
  width: 52px;
  max-width: 100%;
  padding: 1px 5px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  color: var(--text-secondary);
  font-size: 9px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  text-transform: uppercase;
  overflow: hidden;
  text-overflow: clip;
  white-space: nowrap;
}

.collab-search-kind.is-commit {
  border-color: color-mix(in srgb, #29b7d6 42%, var(--border-color) 58%);
  background: color-mix(in srgb, #29b7d6 13%, transparent);
  color: #29b7d6;
}

.collab-search-kind.is-stash {
  border-color: color-mix(in srgb, #af7aa1 48%, var(--border-color) 52%);
  background: color-mix(in srgb, #af7aa1 14%, transparent);
  color: #af7aa1;
}

.collab-search-message,
.collab-search-ref,
.collab-search-author,
.collab-search-date-text,
.collab-search-files,
.collab-search-file,
.collab-search-file-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.collab-search-message {
  font-size: 13px;
  font-weight: 600;
}

.collab-search-author,
.collab-search-date-text,
.collab-search-files {
  color: var(--text-secondary);
  font-size: 12px;
}

.collab-search-ref {
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
  font-size: 12px;
}

.collab-search-files {
  gap: 10px;
}

.collab-search-file {
  gap: 5px;
  max-width: 100%;
  color: var(--text-secondary);
  font-size: 11px;
}

.collab-search-file-icon.is-git-status-icon.status-modified {
  color: var(--git-status-modified);
}

.collab-search-file-icon.is-git-status-icon.status-added {
  color: var(--git-status-added);
}

.collab-search-file-icon.is-git-status-icon.status-deleted {
  color: var(--git-status-deleted);
}

.collab-search-file-icon.is-git-status-icon.status-renamed {
  color: var(--git-status-renamed);
}

.collab-search-file-icon {
  width: 13px;
  height: 13px;
  flex: 0 0 13px;
}

.collab-search-file-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.collab-search-file-more {
  color: var(--text-secondary);
  font-size: 11px;
}

</style>
