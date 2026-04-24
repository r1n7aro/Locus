<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import { t } from "../../i18n";
import type {
  GitCommitInfo,
  GitGraphRef,
  GitHeadState,
  GitHistorySelection,
  GitHistoryTarget,
  GitStashEntry,
} from "../../types";
import { clampHistoryGraphColumnWidth, layoutHistoryGraph } from "./graph/layout";
import { normalizeHistoryGraph } from "./graph/normalize";
import type {
  HistoryGraphAuxLayout,
  HistoryGraphColumnWidthOverrides,
  HistoryGraphDisplayRef,
  HistoryGraphResizableColumn,
  HistoryGraphRowLayout,
} from "./graph/types";
import { collapseDisplayRefsForRail } from "./graph/refs";
import {
  HISTORY_GRAPH_AUX_RADIUS as AUX_R,
  HISTORY_GRAPH_DOT_RADIUS as DOT_R,
  HISTORY_GRAPH_HEAD_RING_RADIUS as HEAD_RING_R,
  HISTORY_GRAPH_SELECTED_HEAD_RING_RADIUS as SELECTED_HEAD_RING_R,
  historyGraphRefConnectorRunout,
} from "./graph/geometry";
import type {
  GitGraphPublicApi,
  GitGraphScrollOptions,
  GitGraphSelectionTarget,
  GitGraphSelectOptions,
} from "./gitGraphSelection";

const VIRTUAL_ROW_BUFFER = 12;
const GRAPH_COLUMN_STORAGE_KEY = "locus.collab.gitGraph.columns.v1";

const props = defineProps<{
  commits: GitCommitInfo[];
  graphRefs: GitGraphRef[];
  headState: GitHeadState;
  selectedHistory: GitHistorySelection | null;
  loading: boolean;
  loadingMore: boolean;
  hasMoreCommits: boolean;
  currentBranch: string;
  currentAuthor: string;
  stashes: GitStashEntry[];
  workspaceChangeCount: number;
  isMerging?: boolean;
  operationBadge?: string;
}>();

const emit = defineEmits<{
  (e: "selectCommit", hash: string | null): void;
  (e: "loadMore"): void;
  (e: "historyContextmenu", event: MouseEvent, target: GitHistoryTarget): void;
}>();

const headerRef = ref<HTMLElement | null>(null);
const scrollRef = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);
const headerHeight = ref(0);
const columnWidthOverrides = ref<HistoryGraphColumnWidthOverrides>(readStoredColumnWidths());
const activeResizeColumn = ref<HistoryGraphResizableColumn | null>(null);
let scrollFrame = 0;
let resizeObserver: ResizeObserver | null = null;
let columnResizeMoveHandler: ((event: MouseEvent) => void) | null = null;
let columnResizeUpHandler: (() => void) | null = null;
let releaseColumnResizeSelectionLock: (() => void) | null = null;

function readStoredColumnWidths(): HistoryGraphColumnWidthOverrides {
  try {
    const raw = localStorage.getItem(GRAPH_COLUMN_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown> | null;
    if (!parsed || typeof parsed !== "object") return {};

    const next: HistoryGraphColumnWidthOverrides = {};
    for (const column of ["refsWidth", "messageWidth", "metaWidth"] as const) {
      const value = parsed[column];
      if (typeof value === "number") {
        next[column] = clampHistoryGraphColumnWidth(column, value);
      }
    }
    if (next.metaWidth === undefined && typeof parsed.authorWidth === "number") {
      next.metaWidth = clampHistoryGraphColumnWidth("metaWidth", parsed.authorWidth + 96);
    }
    return next;
  } catch {
    return {};
  }
}

function persistColumnWidths(overrides: HistoryGraphColumnWidthOverrides) {
  try {
    if (Object.keys(overrides).length === 0) {
      localStorage.removeItem(GRAPH_COLUMN_STORAGE_KEY);
      return;
    }
    localStorage.setItem(GRAPH_COLUMN_STORAGE_KEY, JSON.stringify(overrides));
  } catch {
    // ignore storage failures
  }
}

function syncViewport() {
  const el = scrollRef.value;
  scrollTop.value = el?.scrollTop ?? 0;
  viewportHeight.value = el?.clientHeight ?? 0;
  headerHeight.value = headerRef.value?.offsetHeight ?? 0;
}

function maybeLoadMore() {
  const el = scrollRef.value;
  if (!el || !props.hasMoreCommits || props.loadingMore) return;
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 100) {
    emit("loadMore");
  }
}

function onScroll() {
  if (scrollFrame) return;
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = 0;
    syncViewport();
    maybeLoadMore();
  });
}

function refreshViewport() {
  syncViewport();
  maybeLoadMore();
}

function clampScrollTopValue(value: number): number {
  const el = scrollRef.value;
  if (!el) return 0;
  const maxTop = Math.max(0, el.scrollHeight - el.clientHeight);
  return Math.max(0, Math.min(maxTop, value));
}

function reconnectObservers() {
  resizeObserver?.disconnect();
  if (!resizeObserver) return;
  if (scrollRef.value) resizeObserver.observe(scrollRef.value);
  if (headerRef.value) resizeObserver.observe(headerRef.value);
}

watch([scrollRef, headerRef], ([nextScrollRef], [previousScrollRef]) => {
  previousScrollRef?.removeEventListener("scroll", onScroll);
  nextScrollRef?.addEventListener("scroll", onScroll, { passive: true });
  reconnectObservers();
  void nextTick(refreshViewport);
}, { flush: "post" });

onMounted(() => {
  if (typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => {
      refreshViewport();
    });
    reconnectObservers();
  } else {
    window.addEventListener("resize", refreshViewport);
  }
  refreshViewport();
});

onUnmounted(() => {
  stopColumnResize(false);
  scrollRef.value?.removeEventListener("scroll", onScroll);
  if (scrollFrame) {
    cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
  }
  resizeObserver?.disconnect();
  resizeObserver = null;
  window.removeEventListener("resize", refreshViewport);
});

const selectedHash = computed(() =>
  props.selectedHistory?.kind === "commit" || props.selectedHistory?.kind === "stash"
    ? props.selectedHistory.hash
    : null,
);

const scene = computed(() =>
  normalizeHistoryGraph({
    commits: props.commits,
    stashes: props.stashes,
    refs: props.graphRefs,
    headState: props.headState,
    selectedHistory: props.selectedHistory,
    workspaceChangeCount: props.workspaceChangeCount,
  }),
);

const layout = computed(() => layoutHistoryGraph(scene.value, columnWidthOverrides.value));
const workspaceSelected = computed(() => scene.value.selectionKind === "workspace");
watch(() => layout.value.rows.length, () => {
  void nextTick(refreshViewport);
});
const rowRefAvailableWidth = computed(() => Math.max(48, layout.value.rails.refsWidth - 18));

const contentTopInset = computed(() => layout.value.rows[0]?.top ?? 0);
const virtualWindow = computed(() => {
  const rows = layout.value.rows;
  const totalRows = rows.length;
  if (totalRows === 0) {
    return {
      start: 0,
      end: -1,
      rows: [] as HistoryGraphRowLayout[],
      topSpacer: 0,
      bottomSpacer: 0,
    };
  }

  if (viewportHeight.value <= 0) {
    return {
      start: 0,
      end: totalRows - 1,
      rows,
      topSpacer: 0,
      bottomSpacer: 0,
    };
  }

  const rowHeight = layout.value.rails.rowHeight;
  const bodyScrollTop = Math.max(0, scrollTop.value - headerHeight.value);
  const bodyViewportHeight = Math.max(rowHeight, viewportHeight.value - headerHeight.value);
  const visibleStart = Math.max(
    0,
    Math.floor((bodyScrollTop - contentTopInset.value) / rowHeight),
  );
  const visibleEnd = Math.max(
    0,
    Math.min(
      totalRows - 1,
      Math.ceil((bodyScrollTop + bodyViewportHeight - contentTopInset.value) / rowHeight) - 1,
    ),
  );
  const start = Math.max(0, visibleStart - VIRTUAL_ROW_BUFFER);
  const end = Math.max(start, Math.min(totalRows - 1, visibleEnd + VIRTUAL_ROW_BUFFER));

  return {
    start,
    end,
    rows: rows.slice(start, end + 1),
    topSpacer: start * rowHeight,
    bottomSpacer: Math.max(0, (totalRows - end - 1) * rowHeight),
  };
});
const visibleCommits = computed(() =>
  layout.value.commits.filter(commit =>
    commit.rowIndex >= virtualWindow.value.start
    && commit.rowIndex <= virtualWindow.value.end,
  ),
);
const visibleAuxNodes = computed(() =>
  layout.value.auxNodes.filter(node =>
    node.rowIndex >= virtualWindow.value.start
    && node.rowIndex <= virtualWindow.value.end,
  ),
);
const visibleEdges = computed(() =>
  layout.value.edges.filter(edge =>
    edge.startRowIndex <= virtualWindow.value.end
    && edge.endRowIndex >= virtualWindow.value.start,
  ),
);
const visibleRowEntries = computed(() =>
  virtualWindow.value.rows.map(row => ({
    row,
    refDisplay: collapseDisplayRefsForRail(row.refs, rowRefAvailableWidth.value),
  })),
);

const sharedRailStyle = computed(() => ({
  "--graph-refs-width": `${layout.value.rails.refsWidth}px`,
  "--graph-graph-width": `${layout.value.rails.graphWidth}px`,
  "--graph-message-width": `${layout.value.rails.messageWidth}px`,
  "--graph-meta-width": `${layout.value.rails.metaWidth}px`,
  "--graph-row-height": `${layout.value.rails.rowHeight}px`,
  "--graph-body-width": `${layout.value.rails.bodyMinWidth}px`,
  "--graph-ref-popup-width": `${rowRefAvailableWidth.value}px`,
  minWidth: `${layout.value.rails.bodyMinWidth}px`,
}));

const tableStyle = computed(() => ({
  ...sharedRailStyle.value,
  minHeight: `${layout.value.contentHeight}px`,
}));

const svgStyle = computed(() => ({
  left: `${layout.value.rails.refsWidth}px`,
  width: `${layout.value.rails.graphWidth}px`,
  height: `${layout.value.contentHeight}px`,
}));

function setColumnWidth(column: HistoryGraphResizableColumn, width: number) {
  columnWidthOverrides.value = {
    ...columnWidthOverrides.value,
    [column]: clampHistoryGraphColumnWidth(column, width),
  };
}

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
    persistColumnWidths(columnWidthOverrides.value);
  }
}

function onColumnResizeStart(event: MouseEvent, column: HistoryGraphResizableColumn) {
  event.preventDefault();
  event.stopPropagation();
  stopColumnResize(false);

  activeResizeColumn.value = column;
  const startX = event.clientX;
  const startWidth = layout.value.rails[column];

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

function findSelectionRow(target: GitGraphSelectionTarget): HistoryGraphRowLayout | null {
  if (target.kind === "workspace") {
    return layout.value.auxNodes.find(node => node.kind === "workspace") ?? null;
  }
  if (target.kind === "stash") {
    return layout.value.auxNodes.find(node => node.kind === "stash" && node.hash === target.hash) ?? null;
  }
  return layout.value.commits.find(row => row.commit.hash === target.hash) ?? null;
}

function isSelectionActive(target: GitGraphSelectionTarget): boolean {
  if (target.kind === "workspace") {
    return workspaceSelected.value;
  }
  return selectedHash.value === target.hash;
}

function targetSelectionHash(target: GitGraphSelectionTarget): string | null {
  return target.kind === "workspace" ? null : target.hash;
}

function scrollToSelectionRow(row: HistoryGraphRowLayout, options: GitGraphScrollOptions = {}): boolean {
  const el = scrollRef.value;
  if (!el) return false;

  const bodyViewportHeight = Math.max(
    layout.value.rails.rowHeight,
    el.clientHeight - headerHeight.value,
  );
  const targetTop = headerHeight.value + row.top + row.height / 2 - bodyViewportHeight / 2;
  const top = clampScrollTopValue(targetTop);
  el.scrollTo({
    top,
    behavior: options.behavior ?? "auto",
  });
  scrollTop.value = top;
  return true;
}

async function scrollToHistory(
  target: GitGraphSelectionTarget,
  options: GitGraphScrollOptions = {},
): Promise<boolean> {
  await nextTick();
  refreshViewport();
  const row = findSelectionRow(target);
  if (!row) return false;
  return scrollToSelectionRow(row, options);
}

async function selectHistory(
  target: GitGraphSelectionTarget,
  options: GitGraphSelectOptions = {},
): Promise<boolean> {
  const shouldClear = options.toggle === true && isSelectionActive(target);
  emit("selectCommit", shouldClear ? null : targetSelectionHash(target));
  if (shouldClear || options.scroll === false) return false;
  return scrollToHistory(target, options);
}

defineExpose<GitGraphPublicApi>({
  selectHistory,
  scrollToHistory,
});

function formatDate(ts: number): string {
  const now = Date.now() / 1000;
  const diff = now - ts;
  if (diff < 60) return t("time.justNow");
  if (diff < 3600) return t("time.minutesAgo", String(Math.floor(diff / 60)));
  if (diff < 86400) return t("time.hoursAgo", String(Math.floor(diff / 3600)));
  if (diff < 604800) return t("time.daysAgo", String(Math.floor(diff / 86400)));
  if (diff < 2592000) return t("time.weeksAgo", String(Math.floor(diff / 604800)));
  const d = new Date(ts * 1000);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

function refStyle(ref: HistoryGraphDisplayRef, row: HistoryGraphRowLayout) {
  if (ref.kind !== "branch" && ref.kind !== "remote" && ref.kind !== "tag") {
    return undefined;
  }

  const tone = ref.kind === "remote"
    ? { background: "18", border: "40" }
    : ref.kind === "tag"
      ? { background: "20", border: "48" }
      : { background: "22", border: "55" };

  return {
    "--graph-ref-color": row.color,
    "--graph-ref-bg": `${row.color}${tone.background}`,
    "--graph-ref-border": `${row.color}${tone.border}`,
  };
}

function refTitle(ref: HistoryGraphDisplayRef): string {
  return ref.title ?? ref.text;
}

function hiddenRefsTitle(refs: HistoryGraphDisplayRef[]): string {
  return refs.map(refTitle).join(", ");
}

function remoteMarkerVariant(marker: NonNullable<HistoryGraphDisplayRef["sourceMarkers"]>[number]): number {
  return marker.variant ?? 0;
}

function rowTitle(row: HistoryGraphRowLayout): string {
  return row.kind === "commit" ? row.commit.message : row.label;
}

function rowMetaPrimary(row: HistoryGraphRowLayout): string {
  if (row.kind === "workspace") return props.currentAuthor;
  if (row.kind === "stash") {
    return row.stash?.author ?? "";
  }
  return row.kind === "commit" ? row.commit.author : "";
}

function rowMetaSecondary(row: HistoryGraphRowLayout): string {
  if (row.kind === "workspace") return "";
  if (row.kind === "stash" && row.stash) return formatDate(row.stash.date);
  return row.kind === "commit" ? formatDate(row.commit.date) : "";
}

function rowMetaText(row: HistoryGraphRowLayout): string {
  const primary = rowMetaPrimary(row).trim();
  const secondary = rowMetaSecondary(row).trim();
  if (primary && secondary) return `${primary} / ${secondary}`;
  return primary || secondary;
}

function onClickCommit(hash: string) {
  emit("selectCommit", selectedHash.value === hash ? null : hash);
}

function onClickWorkspace() {
  emit("selectCommit", null);
}

function onClickAux(node: HistoryGraphAuxLayout) {
  if (node.kind === "workspace") {
    onClickWorkspace();
    return;
  }
  emit("selectCommit", selectedHash.value === node.hash ? null : node.hash);
}

function onRowClick(row: HistoryGraphRowLayout) {
  if (row.kind === "commit") {
    onClickCommit(row.commit.hash);
    return;
  }
  onClickAux(row);
}

function onCommitContextMenu(event: MouseEvent, row: Extract<HistoryGraphRowLayout, { kind: "commit" }>) {
  emit("historyContextmenu", event, { kind: "commit", commit: row.commit });
}

function onAuxContextMenu(event: MouseEvent, node: HistoryGraphAuxLayout) {
  if (node.kind !== "stash" || !node.stash) return;
  emit("historyContextmenu", event, { kind: "stash", stash: node.stash });
}

function onRowContextMenu(event: MouseEvent, row: HistoryGraphRowLayout) {
  if (row.kind === "commit") {
    onCommitContextMenu(event, row);
    return;
  }
  onAuxContextMenu(event, row);
}

function showRowConnector(row: HistoryGraphRowLayout): boolean {
  return row.refs.length > 0;
}

function rowRefConnectorStyle(row: HistoryGraphRowLayout) {
  return {
    "--graph-connector-color": row.color,
    "--graph-connector-runout": `${historyGraphRefConnectorRunout(row)}px`,
  };
}
</script>

<template>
  <div class="graph-panel">
    <div v-if="loading && layout.rows.length === 0" class="graph-empty">
      <div class="empty-text">Loading...</div>
    </div>
    <div v-else class="graph-table-shell">
      <div ref="scrollRef" class="graph-scroll">
        <div ref="headerRef" class="graph-header-row" :style="sharedRailStyle">
          <div class="graph-header-cell graph-header-cell-resizable">
            <span class="graph-header-cell-label">BRANCH / TAG</span>
            <span
              class="graph-column-handle"
              :class="{ dragging: activeResizeColumn === 'refsWidth' }"
              @mousedown="onColumnResizeStart($event, 'refsWidth')"
            />
          </div>
          <div class="graph-header-cell">GRAPH</div>
          <div class="graph-header-cell graph-header-cell-resizable">
            <span class="graph-header-cell-label">COMMIT MESSAGE</span>
            <span
              class="graph-column-handle"
              :class="{ dragging: activeResizeColumn === 'messageWidth' }"
              @mousedown="onColumnResizeStart($event, 'messageWidth')"
            />
          </div>
          <div class="graph-header-cell graph-header-cell-resizable">
            <span class="graph-header-cell-label">AUTHOR / DATE</span>
            <span
              class="graph-column-handle"
              :class="{ dragging: activeResizeColumn === 'metaWidth' }"
              @mousedown="onColumnResizeStart($event, 'metaWidth')"
            />
          </div>
        </div>

        <div class="graph-table" :style="tableStyle">
          <svg
            class="graph-svg"
            :style="svgStyle"
            :width="layout.rails.graphWidth"
            :height="layout.contentHeight"
            aria-hidden="true"
          >
            <path
              v-for="edge in visibleEdges"
              :key="edge.id"
              :d="edge.path"
              :stroke="edge.color"
              stroke-width="2"
              fill="none"
              stroke-linecap="round"
              :opacity="edge.opacity ?? 1"
              :stroke-dasharray="edge.dashed ? '4,3' : undefined"
            />

            <template v-for="aux in visibleAuxNodes" :key="'node_' + aux.id">
              <circle
                v-if="aux.kind === 'workspace'"
                :cx="aux.x"
                :cy="aux.y"
                :r="workspaceSelected ? AUX_R + 1.2 : AUX_R"
                fill="none"
                :stroke="aux.color"
                :stroke-width="workspaceSelected ? 2 : 1.75"
                stroke-dasharray="3,2"
              />
              <rect
                v-else
                :x="aux.x - AUX_R"
                :y="aux.y - AUX_R"
                :width="AUX_R * 2"
                :height="AUX_R * 2"
                :fill="aux.color"
                :stroke="aux.selected ? 'var(--active-bg)' : 'none'"
                :stroke-width="aux.selected ? 1.5 : 0"
                rx="0"
              />
            </template>

            <template v-for="row in visibleCommits" :key="'dot_' + row.commit.hash">
              <circle
                v-if="row.selected"
                :cx="row.x"
                :cy="row.y"
                :r="row.isHead ? SELECTED_HEAD_RING_R : DOT_R"
                fill="none"
                stroke="var(--active-bg)"
                :stroke-width="1.5"
              />
              <circle
                v-if="row.isHead"
                :cx="row.x"
                :cy="row.y"
                :r="HEAD_RING_R"
                fill="none"
                :stroke="row.color"
                :stroke-width="1.75"
                opacity="0.95"
              />
              <circle
                :cx="row.x"
                :cy="row.y"
                :r="DOT_R"
                :fill="row.color"
              />
            </template>
          </svg>

          <div class="graph-rows">
            <div v-if="virtualWindow.topSpacer > 0" class="graph-virtual-spacer" :style="{ height: `${virtualWindow.topSpacer}px` }" />
            <button
              v-for="entry in visibleRowEntries"
              :key="entry.row.id"
              type="button"
              class="graph-row"
              :class="[
                `graph-row-${entry.row.kind}`,
                {
                  selected: entry.row.selected,
                  head: entry.row.kind === 'commit' && entry.row.isHead,
                  unanchored: entry.row.kind !== 'commit' && entry.row.unanchored,
                },
              ]"
              :title="rowTitle(entry.row)"
              @click="onRowClick(entry.row)"
              @contextmenu.prevent.stop="onRowContextMenu($event, entry.row)"
            >
              <div class="graph-row-refs">
                <div
                  v-if="entry.row.refs.length > 0"
                  class="graph-row-ref-badges"
                  :class="{ 'is-collapsed': entry.refDisplay.isCollapsed }"
                >
                  <div class="graph-row-ref-badges-summary">
                    <span
                      v-for="ref in entry.refDisplay.visibleRefs"
                      :key="ref.key"
                      class="ref-badge"
                      :class="[
                        'ref-' + ref.kind,
                        { 'ref-badge-plain': ref.kind === 'head' || ref.kind === 'stash' },
                      ]"
                      :style="refStyle(ref, entry.row)"
                      :title="refTitle(ref)"
                    >
                      <span v-if="ref.sourceMarkers?.length" class="ref-badge-markers" aria-hidden="true">
                        <span
                          v-for="marker in ref.sourceMarkers"
                          :key="`${ref.key}:${marker.key}`"
                          class="ref-badge-marker"
                          :class="`ref-badge-marker-${marker.kind}`"
                          :title="marker.title"
                        >
                          <svg v-if="marker.kind === 'local'" viewBox="0 0 16 16" fill="currentColor">
                            <path d="M11.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zm-2.25.75a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6A1.5 1.5 0 0 0 4.5 10v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A3 3 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25zM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zM3.5 3.25a.75.75 0 1 1 1.5 0 .75.75 0 0 1-1.5 0z"/>
                          </svg>
                          <svg v-else viewBox="0 0 16 16" fill="currentColor">
                            <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zM1.5 8a6.5 6.5 0 1 1 13 0 6.5 6.5 0 0 1-13 0z"/>
                            <path d="M8 1.5c-1.38 0-2.74 1.9-3.27 4.5h6.54C10.74 3.4 9.38 1.5 8 1.5zM4.55 7.5C4.52 7.66 4.5 7.83 4.5 8s.02.34.05.5h6.9c.03-.16.05-.33.05-.5s-.02-.34-.05-.5h-6.9zM4.73 10c.53 2.6 1.89 4.5 3.27 4.5s2.74-1.9 3.27-4.5H4.73z"/>
                            <circle
                              v-if="remoteMarkerVariant(marker) % 6 === 1"
                              cx="12.15"
                              cy="11.95"
                              r="1.35"
                            />
                            <rect
                              v-else-if="remoteMarkerVariant(marker) % 6 === 2"
                              x="10.85"
                              y="10.65"
                              width="2.6"
                              height="2.6"
                              rx="0.45"
                            />
                            <path
                              v-else-if="remoteMarkerVariant(marker) % 6 === 3"
                              d="M12.15 9.85L14.05 11.75L12.15 13.65L10.25 11.75Z"
                            />
                            <path
                              v-else-if="remoteMarkerVariant(marker) % 6 === 4"
                              d="M12.15 9.9L14.1 13.2H10.2Z"
                            />
                            <rect
                              v-else-if="remoteMarkerVariant(marker) % 6 === 5"
                              x="11.45"
                              y="9.75"
                              width="1.4"
                              height="4"
                              rx="0.4"
                            />
                          </svg>
                        </span>
                      </span>
                      <span class="ref-badge-text">{{ ref.text }}</span>
                    </span>
                    <span
                      v-if="entry.refDisplay.hiddenCount > 0"
                      class="ref-badge ref-badge-overflow"
                      :title="hiddenRefsTitle(entry.refDisplay.hiddenRefs)"
                    >
                      +{{ entry.refDisplay.hiddenCount }}
                    </span>
                  </div>
                  <div
                    v-if="entry.refDisplay.hiddenCount > 0"
                    class="graph-row-ref-badges-expanded"
                    :title="hiddenRefsTitle(entry.refDisplay.hiddenRefs)"
                  >
                    <span
                      v-for="ref in entry.row.refs"
                      :key="`expanded:${ref.key}`"
                      class="ref-badge"
                      :class="[
                        'ref-' + ref.kind,
                        { 'ref-badge-plain': ref.kind === 'head' || ref.kind === 'stash' },
                      ]"
                      :style="refStyle(ref, entry.row)"
                      :title="refTitle(ref)"
                    >
                      <span v-if="ref.sourceMarkers?.length" class="ref-badge-markers" aria-hidden="true">
                        <span
                          v-for="marker in ref.sourceMarkers"
                          :key="`expanded:${ref.key}:${marker.key}`"
                          class="ref-badge-marker"
                          :class="`ref-badge-marker-${marker.kind}`"
                          :title="marker.title"
                        >
                          <svg v-if="marker.kind === 'local'" viewBox="0 0 16 16" fill="currentColor">
                            <path d="M11.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zm-2.25.75a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6A1.5 1.5 0 0 0 4.5 10v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A3 3 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25zM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zM3.5 3.25a.75.75 0 1 1 1.5 0 .75.75 0 0 1-1.5 0z"/>
                          </svg>
                          <svg v-else viewBox="0 0 16 16" fill="currentColor">
                            <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zM1.5 8a6.5 6.5 0 1 1 13 0 6.5 6.5 0 0 1-13 0z"/>
                            <path d="M8 1.5c-1.38 0-2.74 1.9-3.27 4.5h6.54C10.74 3.4 9.38 1.5 8 1.5zM4.55 7.5C4.52 7.66 4.5 7.83 4.5 8s.02.34.05.5h6.9c.03-.16.05-.33.05-.5s-.02-.34-.05-.5h-6.9zM4.73 10c.53 2.6 1.89 4.5 3.27 4.5s2.74-1.9 3.27-4.5H4.73z"/>
                            <circle
                              v-if="remoteMarkerVariant(marker) % 6 === 1"
                              cx="12.15"
                              cy="11.95"
                              r="1.35"
                            />
                            <rect
                              v-else-if="remoteMarkerVariant(marker) % 6 === 2"
                              x="10.85"
                              y="10.65"
                              width="2.6"
                              height="2.6"
                              rx="0.45"
                            />
                            <path
                              v-else-if="remoteMarkerVariant(marker) % 6 === 3"
                              d="M12.15 9.85L14.05 11.75L12.15 13.65L10.25 11.75Z"
                            />
                            <path
                              v-else-if="remoteMarkerVariant(marker) % 6 === 4"
                              d="M12.15 9.9L14.1 13.2H10.2Z"
                            />
                            <rect
                              v-else-if="remoteMarkerVariant(marker) % 6 === 5"
                              x="11.45"
                              y="9.75"
                              width="1.4"
                              height="4"
                              rx="0.4"
                            />
                          </svg>
                        </span>
                      </span>
                      <span class="ref-badge-text">{{ ref.text }}</span>
                    </span>
                  </div>
                </div>
                <span
                  v-if="showRowConnector(entry.row)"
                  class="graph-row-ref-connector"
                  :style="rowRefConnectorStyle(entry.row)"
                />
              </div>

              <div class="graph-row-track" />

              <div class="graph-row-message">
                <span class="graph-row-title">{{ rowTitle(entry.row) }}</span>
                <span
                  v-if="entry.row.kind === 'workspace'"
                  class="workspace-inline-badge"
                >+{{ entry.row.workspaceChangeCount ?? 0 }}</span>
              </div>

              <div class="graph-row-meta">
                <span v-if="rowMetaText(entry.row)" class="graph-row-meta-text">{{ rowMetaText(entry.row) }}</span>
              </div>
            </button>
            <div
              v-if="virtualWindow.bottomSpacer > 0"
              class="graph-virtual-spacer"
              :style="{ height: `${virtualWindow.bottomSpacer}px` }"
            />
          </div>
        </div>
        <div v-if="loadingMore" class="load-more-indicator">Loading...</div>
      </div>
    </div>
  </div>
</template>
