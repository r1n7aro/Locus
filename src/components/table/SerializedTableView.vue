<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import UnityPropertyEditor from "../unity/UnityPropertyEditor.vue";
import UnitySerializedPropertyTree from "../unity/UnitySerializedPropertyTree.vue";
import type { UnitySerializedPropertySnapshot } from "../unity/unitySerializedValue";
import type {
  SerializedTableCell,
  SerializedTableColumnConfig,
  SerializedTableCommitEvent,
  SerializedTableProgress,
  SerializedTableRow,
} from "./serializedTable";

interface ColumnResizeState {
  key: string;
  pointerId: number;
  startX: number;
  startWidth: number;
  minWidth: number;
}

const props = withDefaults(defineProps<{
  columns: SerializedTableColumnConfig[];
  rows: SerializedTableRow[];
  loading?: boolean;
  status?: string;
  error?: string;
  progress?: SerializedTableProgress | null;
  savingCellKey?: string;
  sourceCount?: number;
  columnWidths?: Record<string, number>;
}>(), {
  loading: false,
  status: "",
  error: "",
  progress: null,
  savingCellKey: "",
  sourceCount: undefined,
  columnWidths: () => ({}),
});

const emit = defineEmits<{
  (event: "commit", payload: SerializedTableCommitEvent): void;
  (event: "update:columnWidths", widths: Record<string, number>): void;
}>();

const ASSET_COLUMN_WIDTH = 220;
const STATUS_COLUMN_WIDTH = 160;
const MIN_DATA_COLUMN_WIDTH = 88;
const MIN_EDGE_COLUMN_WIDTH = 120;
const MAX_DATA_COLUMN_WIDTH = 220;
const MAX_RESIZED_COLUMN_WIDTH = 640;
const ASSET_COLUMN_KEY = "asset";
const STATUS_COLUMN_KEY = "status";
const RESIZING_BODY_CLASS = "locus-serialized-table-resizing";

const scrollerRef = ref<HTMLElement | null>(null);
const tableRef = ref<HTMLTableElement | null>(null);
const columnWidthOverrides = ref<Record<string, number>>({ ...props.columnWidths });
const resizingColumnKey = ref("");

let layoutCheckFrame = 0;
let lastLayoutProblemSignature = "";
let resizeState: ColumnResizeState | null = null;

watch(() => props.columnWidths, (next) => {
  columnWidthOverrides.value = { ...(next ?? {}) };
  scheduleLayoutCheck();
}, { deep: true });

const progressPercent = computed(() => `${Math.round(clampProgress(props.progress?.progress ?? 0) * 100)}%`);
const emptyStateText = computed(() => {
  const progress = props.progress;
  if (progress?.active) {
    return [progress.title, progress.info].filter((part) => part.trim()).join(" · ");
  }
  return props.error || props.status || "No rows";
});
const assetColumnWidth = computed(() =>
  widthFromOverride(ASSET_COLUMN_KEY, ASSET_COLUMN_WIDTH, MIN_EDGE_COLUMN_WIDTH),
);
const statusColumnWidth = computed(() =>
  widthFromOverride(STATUS_COLUMN_KEY, STATUS_COLUMN_WIDTH, MIN_EDGE_COLUMN_WIDTH),
);
const columnWidths = computed(() => props.columns.map((column) =>
  widthFromOverride(columnKey(column), autoColumnWidth(column), MIN_DATA_COLUMN_WIDTH),
));
const tablePixelWidth = computed(() =>
  assetColumnWidth.value
  + statusColumnWidth.value
  + columnWidths.value.reduce((total, width) => total + width, 0),
);
const tableLayoutStyle = computed(() => ({
  width: `max(100%, ${tablePixelWidth.value}px)`,
  minWidth: `${tablePixelWidth.value}px`,
}));

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function columnDisplayName(column: SerializedTableColumnConfig): string {
  return column.label || column.propertyPath || column.id;
}

function columnKey(column: SerializedTableColumnConfig): string {
  return `column:${column.id || column.propertyPath || columnDisplayName(column)}`;
}

function clampColumnWidth(value: number, minWidth: number): number {
  if (!Number.isFinite(value)) return minWidth;
  return Math.min(MAX_RESIZED_COLUMN_WIDTH, Math.max(minWidth, Math.round(value)));
}

function widthFromOverride(key: string, fallback: number, minWidth: number): number {
  const stored = columnWidthOverrides.value[key];
  return typeof stored === "number"
    ? clampColumnWidth(stored, minWidth)
    : clampColumnWidth(fallback, minWidth);
}

function estimateLabelWidth(label: string): number {
  const contentWidth = Array.from(label || "Property").reduce((total, char) => {
    return total + (char.charCodeAt(0) > 255 ? 12 : 7);
  }, 0);
  return Math.ceil(contentWidth + 24);
}

function widthForCell(cell: SerializedTableCell): number {
  if (usesPropertyTree(cell)) return MAX_DATA_COLUMN_WIDTH;

  const valueType = String(cell.valueType || cell.type || "");
  if (valueType === "Boolean") return 72;
  if (/^(Integer|Float|Double)$/.test(valueType)) return 78;
  if (/^(Vector2|Vector3|Vector4|Quaternion|Rect|Bounds)$/.test(valueType)) return 160;
  if (valueType === "Color" || valueType === "LayerMask") return 132;
  if (valueType === "ObjectReference") return 180;
  if (valueType === "Enum") return 120;
  if (valueType === "String") return 140;
  return 112;
}

function autoColumnWidth(column: SerializedTableColumnConfig): number {
  const labelWidth = estimateLabelWidth(columnDisplayName(column));
  const cellWidth = props.rows.reduce((maxWidth, row) => {
    const cell = row.cells.find((item) => item.columnId === column.id);
    return cell ? Math.max(maxWidth, widthForCell(cell)) : maxWidth;
  }, 0);
  return Math.min(
    MAX_DATA_COLUMN_WIDTH,
    Math.max(MIN_DATA_COLUMN_WIDTH, labelWidth, cellWidth),
  );
}

function resizeHandleLabel(label: string): string {
  return `Resize ${label}`;
}

function startColumnResize(event: PointerEvent, key: string, currentWidth: number, minWidth: number) {
  resizeState = {
    key,
    pointerId: event.pointerId,
    startX: event.clientX,
    startWidth: currentWidth,
    minWidth,
  };
  resizingColumnKey.value = key;
  try {
    (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  } catch {
    // Synthetic View automation pointer events do not always register as active browser pointers.
  }
  document.body.classList.add(RESIZING_BODY_CLASS);
  window.addEventListener("pointermove", updateColumnResize);
  window.addEventListener("pointerup", finishColumnResize);
  window.addEventListener("pointercancel", cancelColumnResize);
}

function updateColumnResize(event: PointerEvent) {
  if (!resizeState || event.pointerId !== resizeState.pointerId) return;
  const nextWidth = clampColumnWidth(
    resizeState.startWidth + event.clientX - resizeState.startX,
    resizeState.minWidth,
  );
  columnWidthOverrides.value = {
    ...columnWidthOverrides.value,
    [resizeState.key]: nextWidth,
  };
  scheduleLayoutCheck();
}

function stopColumnResize(save: boolean) {
  if (save && resizeState) emit("update:columnWidths", { ...columnWidthOverrides.value });
  resizeState = null;
  resizingColumnKey.value = "";
  document.body.classList.remove(RESIZING_BODY_CLASS);
  window.removeEventListener("pointermove", updateColumnResize);
  window.removeEventListener("pointerup", finishColumnResize);
  window.removeEventListener("pointercancel", cancelColumnResize);
}

function finishColumnResize() {
  stopColumnResize(true);
}

function cancelColumnResize() {
  stopColumnResize(false);
}

function resetColumnWidth(key: string) {
  if (!(key in columnWidthOverrides.value)) return;
  const nextWidths = { ...columnWidthOverrides.value };
  delete nextWidths[key];
  columnWidthOverrides.value = nextWidths;
  emit("update:columnWidths", { ...nextWidths });
  scheduleLayoutCheck();
}

function scheduleLayoutCheck() {
  if (layoutCheckFrame) window.cancelAnimationFrame(layoutCheckFrame);
  layoutCheckFrame = window.requestAnimationFrame(() => {
    layoutCheckFrame = 0;
    void nextTick().then(checkTableLayout);
  });
}

function checkTableLayout() {
  const table = tableRef.value;
  const scroller = scrollerRef.value;
  if (!table || !scroller) return;

  const problems: string[] = [];
  const headers = Array.from(table.querySelectorAll<HTMLTableCellElement>("thead th"));
  let previousRight = Number.NEGATIVE_INFINITY;

  headers.forEach((header, index) => {
    const rect = header.getBoundingClientRect();
    if (rect.left < previousRight - 0.5) {
      problems.push(`header ${index + 1} overlaps previous column`);
    }
    if (rect.width < 40) {
      problems.push(`header ${index + 1} width ${Math.round(rect.width)}px`);
    }
    previousRight = Math.max(previousRight, rect.right);
  });

  const tableWidth = table.getBoundingClientRect().width;
  const expectedWidth = tablePixelWidth.value;
  if (tableWidth + 1 < expectedWidth) {
    problems.push(`table compressed to ${Math.round(tableWidth)}px below ${expectedWidth}px`);
  }
  if (expectedWidth > scroller.clientWidth && scroller.scrollWidth <= scroller.clientWidth + 1) {
    problems.push("horizontal overflow is not scrollable");
  }

  const signature = problems.join("|");
  if (!signature) {
    lastLayoutProblemSignature = "";
    return;
  }
  if (signature === lastLayoutProblemSignature) return;
  lastLayoutProblemSignature = signature;
  console.error("[serialized-table] Layout check failed", {
    problems,
    columns: props.columns.length,
    expectedWidth,
    tableWidth: Math.round(tableWidth),
    viewportWidth: scroller.clientWidth,
  });
}

function handleTableWheel(event: WheelEvent) {
  const scroller = scrollerRef.value;
  if (!scroller) return;

  const canScrollY = scroller.scrollHeight > scroller.clientHeight + 1;
  const canScrollX = scroller.scrollWidth > scroller.clientWidth + 1;
  if (!canScrollY && !canScrollX) return;

  const preferHorizontal = event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY);
  if (preferHorizontal && canScrollX) {
    const before = scroller.scrollLeft;
    scroller.scrollLeft += event.deltaX || event.deltaY;
    if (scroller.scrollLeft !== before) event.preventDefault();
    return;
  }

  if (canScrollY) {
    const before = scroller.scrollTop;
    scroller.scrollTop += event.deltaY;
    if (scroller.scrollTop !== before) event.preventDefault();
    return;
  }

  if (canScrollX) {
    const before = scroller.scrollLeft;
    scroller.scrollLeft += event.deltaX || event.deltaY;
    if (scroller.scrollLeft !== before) event.preventDefault();
  }
}

function usesPropertyTree(cell: SerializedTableCell): boolean {
  return !!cell.isArray || !!cell.isManagedReference || (Array.isArray(cell.children) && cell.children.length > 0);
}

function treeSnapshot(cell: SerializedTableCell): UnitySerializedPropertySnapshot {
  return cell as unknown as UnitySerializedPropertySnapshot;
}

function cellKey(row: SerializedTableRow, cell: SerializedTableCell): string {
  return `${row.id}:${cell.columnId}`;
}

function statusForCell(row: SerializedTableRow, cell: SerializedTableCell): string {
  if (props.savingCellKey === cellKey(row, cell)) return "Saving";
  return cell.message || (cell.editable ? "Editable" : "Read only");
}

function commitCell(row: SerializedTableRow, cell: SerializedTableCell, value: unknown, propertyPath = "") {
  emit("commit", {
    row,
    cell,
    propertyPath: propertyPath || cell.propertyPath,
    value,
  });
}

function commitCellProperty(
  row: SerializedTableRow,
  cell: SerializedTableCell,
  event: { propertyPath: string; value: unknown },
) {
  commitCell(row, cell, event.value, event.propertyPath);
}

watch(() => props.columns, scheduleLayoutCheck, { deep: true });
watch(() => props.rows, scheduleLayoutCheck, { deep: true });

onMounted(() => {
  window.addEventListener("resize", scheduleLayoutCheck);
  scheduleLayoutCheck();
});

onBeforeUnmount(() => {
  if (layoutCheckFrame) window.cancelAnimationFrame(layoutCheckFrame);
  stopColumnResize(false);
  window.removeEventListener("resize", scheduleLayoutCheck);
});
</script>

<template>
  <div class="locus-serialized-table">
    <div ref="scrollerRef" class="locus-serialized-table-scroller" @wheel.capture="handleTableWheel">
      <table ref="tableRef" :style="tableLayoutStyle">
        <colgroup>
          <col :style="{ width: assetColumnWidth + 'px' }" />
          <col
            v-for="(column, index) in columns"
            :key="column.id"
            :style="{ width: columnWidths[index] + 'px' }"
          />
          <col :style="{ width: statusColumnWidth + 'px' }" />
        </colgroup>
        <thead>
          <tr>
            <th class="asset-col resizable-col" :class="{ resizing: resizingColumnKey === ASSET_COLUMN_KEY }">
              <span class="column-label">Asset</span>
              <button
                type="button"
                class="column-resize-handle"
                :title="resizeHandleLabel('Asset')"
                @pointerdown.prevent.stop="startColumnResize($event, ASSET_COLUMN_KEY, assetColumnWidth, MIN_EDGE_COLUMN_WIDTH)"
                @dblclick.prevent.stop="resetColumnWidth(ASSET_COLUMN_KEY)"
              ></button>
            </th>
            <th
              v-for="(column, index) in columns"
              :key="column.id"
              class="value-col resizable-col"
              :class="{ resizing: resizingColumnKey === columnKey(column) }"
              :title="columnDisplayName(column)"
            >
              <span class="column-label">{{ columnDisplayName(column) }}</span>
              <button
                type="button"
                class="column-resize-handle"
                :title="resizeHandleLabel(columnDisplayName(column))"
                @pointerdown.prevent.stop="startColumnResize($event, columnKey(column), columnWidths[index], MIN_DATA_COLUMN_WIDTH)"
                @dblclick.prevent.stop="resetColumnWidth(columnKey(column))"
              ></button>
            </th>
            <th class="status-col resizable-col" :class="{ resizing: resizingColumnKey === STATUS_COLUMN_KEY }">
              <span class="column-label">Status</span>
              <button
                type="button"
                class="column-resize-handle"
                :title="resizeHandleLabel('Status')"
                @pointerdown.prevent.stop="startColumnResize($event, STATUS_COLUMN_KEY, statusColumnWidth, MIN_EDGE_COLUMN_WIDTH)"
                @dblclick.prevent.stop="resetColumnWidth(STATUS_COLUMN_KEY)"
              ></button>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in rows" :key="row.id">
            <td class="asset-cell">
              <slot name="asset" :row="row">
                <div class="asset-cell-content">
                  <span>{{ row.label }}</span>
                  <small>{{ row.assetPath }}</small>
                </div>
              </slot>
            </td>
            <td v-for="cell in row.cells" :key="cell.columnId" class="value-cell" :class="{ error: !cell.ok }">
              <UnitySerializedPropertyTree
                v-if="usesPropertyTree(cell)"
                :property="treeSnapshot(cell)"
                :disabled="!!savingCellKey"
                @commit="commitCellProperty(row, cell, $event)"
              />
              <UnityPropertyEditor
                v-else
                :model-value="cell.value"
                :property-type="cell.valueType || cell.type"
                :display-value="cell.displayValue"
                :editable="cell.editable"
                :disabled="!!savingCellKey"
                :enum-options="cell.enumOptions"
                :is-flags-enum="cell.isFlagsEnum"
                :enum-value-index="cell.enumValueIndex"
                :enum-value-flag="cell.enumValueFlag"
                :reference-type-full-name="cell.referenceTypeFullName"
                :reference-type-assembly="cell.referenceTypeAssembly"
                :title="cell.propertyPath"
                @commit="commitCell(row, cell, $event)"
              />
            </td>
            <td class="status-cell">
              <div class="status-cell-content">
                <span>{{ row.message || row.status }}</span>
                <small v-for="cell in row.cells.filter((item) => !item.ok)" :key="cell.columnId">
                  {{ cell.label }}: {{ cell.message }}
                </small>
                <small v-if="row.cells.every((item) => item.ok)">
                  {{ row.cells[0] ? statusForCell(row, row.cells[0]) : "" }}
                </small>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="!rows.length" class="locus-serialized-table-empty">
        {{ emptyStateText }}
      </div>
    </div>
    <footer class="locus-serialized-table-statusbar" :class="{ error: !!error, loading: !!progress?.active }">
      <div v-if="progress?.active" class="progress-status" aria-live="polite">
        <div class="progress-row">
          <span class="progress-title">{{ progress.title }}</span>
          <span v-if="progress.info" class="progress-info">{{ progress.info }}</span>
          <span class="progress-percent">{{ progressPercent }}</span>
        </div>
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill" :style="{ width: progressPercent }"></div>
        </div>
      </div>
      <span v-else>{{ error || status }}</span>
      <span>
        {{ rows.length }} rows ·
        <template v-if="sourceCount !== undefined">{{ sourceCount }} sources · </template>{{ columns.length }} columns
      </span>
    </footer>
  </div>
</template>

<style scoped>
.locus-serialized-table {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.locus-serialized-table-scroller {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable both-edges;
  touch-action: pan-x pan-y;
}

table {
  border-collapse: collapse;
  table-layout: fixed;
}

thead {
  position: sticky;
  top: 0;
  z-index: 1;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
}

th,
td {
  min-width: 0;
  padding: 6px 8px;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 58%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 66%, transparent);
  font-size: 12px;
  vertical-align: middle;
}

th {
  position: relative;
  overflow: hidden;
  text-align: left;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

th:last-child,
td:last-child {
  border-right: 0;
}

td {
  overflow: hidden;
}

tr:hover td {
  background: var(--hover-bg);
}

.column-label {
  display: block;
  min-width: 0;
  padding-right: 8px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.column-resize-handle {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  z-index: 2;
  width: 9px;
  min-width: 9px;
  min-height: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  cursor: col-resize;
  touch-action: none;
  user-select: none;
}

.column-resize-handle::before {
  content: "";
  position: absolute;
  top: 6px;
  right: 4px;
  bottom: 6px;
  width: 1px;
  background: color-mix(in srgb, var(--border-strong) 72%, transparent);
}

.column-resize-handle:hover::before,
.column-resize-handle:focus-visible::before,
.resizable-col.resizing .column-resize-handle::before {
  right: 3px;
  width: 2px;
  background: var(--accent-color);
}

.column-resize-handle:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -2px;
}

.asset-cell,
.status-cell {
  vertical-align: middle;
}

.asset-cell-content,
.status-cell-content {
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
}

.asset-cell-content span,
.asset-cell-content small,
.status-cell-content span,
.status-cell-content small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.asset-cell-content span {
  font-weight: 600;
}

.asset-cell-content small,
.status-cell-content small {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.value-cell :deep(input:not([type="checkbox"])),
.value-cell :deep(select),
.value-cell :deep(.unity-property-tree),
.value-cell :deep(.unity-property-editor) {
  max-width: 100%;
  min-width: 0;
  width: 100%;
  font-size: 12px;
}

.value-cell :deep(input[type="checkbox"]),
.value-cell :deep(.unity-bool-field) {
  width: 14px;
  height: 14px;
  min-height: 14px;
  padding: 0;
}

.value-cell.error :deep(input) {
  border-color: var(--status-danger-border);
}

.locus-serialized-table-empty {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.locus-serialized-table-statusbar {
  min-height: 22px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 8px;
  border-top: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.locus-serialized-table-statusbar > span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.locus-serialized-table-statusbar.error {
  color: var(--status-danger-fg);
}

.progress-status {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 3px;
}

.progress-row {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, auto) minmax(0, 1fr) auto;
  align-items: baseline;
  gap: 8px;
}

.progress-title {
  color: var(--text-color);
  font-weight: 600;
}

.progress-info,
.progress-percent {
  color: var(--text-secondary);
}

.progress-percent {
  font-size: 11px;
}

.progress-track {
  height: 3px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent-color);
  transition: width 0.16s ease;
}
</style>

<style>
body.locus-serialized-table-resizing,
body.locus-serialized-table-resizing * {
  cursor: col-resize !important;
  user-select: none !important;
}
</style>
