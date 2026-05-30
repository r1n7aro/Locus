pub(super) fn table_config_ts() -> String {
    r#"export interface SerializedTableColumnConfig {
  id: string;
  label: string;
  propertyPath: string;
}

export interface AssetSearchResult {
  path: string;
  name: string;
  root?: string;
  kind: string;
  typeLabel?: string;
  matchScore?: number;
  source: string;
}

export interface SerializedTablePropertyOverride {
  columnId: string;
  propertyPath: string;
}

export interface SerializedTableSourceConfig {
  id: string;
  label: string;
  assetPath: string;
  guid?: string;
  sourceKind?: "asset" | "component";
  componentType?: string;
  componentIndex?: number;
  propertyOverrides?: SerializedTablePropertyOverride[];
}

export interface SerializedTableSourceProviderContext {
  searchAssets(query: string, roots?: string[], limit?: number): Promise<AssetSearchResult[]>;
  fromAssetResults(
    results: AssetSearchResult[],
    defaults?: Partial<SerializedTableSourceConfig>,
  ): SerializedTableSourceConfig[];
}

export interface SerializedTableOptions {
  maxRows?: number;
}

export type SerializedTableSourceProvider =
  | SerializedTableSourceConfig[]
  | ((context: SerializedTableSourceProviderContext) =>
      SerializedTableSourceConfig[] | Promise<SerializedTableSourceConfig[]>)
  | {
      id: string;
      label?: string;
      resolve: (context: SerializedTableSourceProviderContext) =>
        SerializedTableSourceConfig[] | Promise<SerializedTableSourceConfig[]>;
    };

export const tableColumns: SerializedTableColumnConfig[] = [
  { id: "name", label: "Name", propertyPath: "m_Name" },
];

export const tableSources: SerializedTableSourceConfig[] = [];

export const tableOptions: SerializedTableOptions = {
  maxRows: 1000,
};

export const tableSourceProviders: SerializedTableSourceProvider[] = [
  // {
  //   id: "entity-prefabs",
  //   label: "Entity Prefabs",
  //   resolve: async ({ searchAssets, fromAssetResults }) =>
  //     fromAssetResults(await searchAssets("t:prefab component:Entity", ["Assets"], 1000), {
  //       sourceKind: "component",
  //       componentType: "Entity",
  //     }),
  // },
  // {
  //   id: "idata-assets",
  //   label: "IData Assets",
  //   resolve: async ({ searchAssets, fromAssetResults }) =>
  //     fromAssetResults(await searchAssets("t:scriptableObject inherits:IData", ["Assets"], 1000), {
  //       sourceKind: "asset",
  //     }),
  // },
];
"#
    .to_string()
}

pub(super) fn app_vue(_name: &str) -> String {
    r####"<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { UnityPropertyEditor, UnitySerializedPropertyTree } from "@locus/components";
import { view } from "@locus/view-runtime";
import {
  tableColumns as defaultColumns,
  tableOptions as defaultTableOptions,
  tableSources as defaultSources,
  tableSourceProviders as defaultSourceProviders,
} from "./tableConfig";
import type {
  AssetSearchResult,
  SerializedTableColumnConfig,
  SerializedTableSourceConfig,
  SerializedTableSourceProvider,
} from "./tableConfig";

interface SerializedCell {
  columnId: string;
  label: string;
  propertyPath: string;
  name: string;
  type: string;
  valueType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  value: unknown;
  displayValue: string;
  editable: boolean;
  hasChildren: boolean;
  isArray: boolean;
  arraySize: number;
  ok: boolean;
  message: string;
  isFlagsEnum: boolean;
  enumValueIndex: number;
  enumValueFlag: number;
  enumOptions: Array<{
    label: string;
    value: string;
    name: string;
    index: number;
    numericValue: number;
  }>;
  children: SerializedCell[];
  isManagedReference: boolean;
  managedReferenceFullTypename: string;
  managedReferenceFieldTypename: string;
  managedReferenceDisplayName: string;
  managedReferenceTypes: Array<{
    label: string;
    value: string;
    fullName: string;
    assembly: string;
  }>;
}

interface SerializedPropertyCommitEvent {
  propertyPath: string;
  value: unknown;
}

interface SerializedTableRow {
  id: string;
  label: string;
  assetPath: string;
  sourceKind: string;
  typeName: string;
  status: string;
  message: string;
  cells: SerializedCell[];
}

interface SerializedTableResponse {
  ok: boolean;
  message: string;
  rows: SerializedTableRow[];
}

interface SerializedTableWriteResponse {
  ok: boolean;
  message: string;
  cell: SerializedCell;
}

interface TableLoadProgress {
  active: boolean;
  title: string;
  info: string;
  progress: number;
}

interface TableColumnResizeState {
  key: string;
  pointerId: number;
  startX: number;
  startWidth: number;
  minWidth: number;
}

const sourceProviders: SerializedTableSourceProvider[] = Array.isArray(defaultSourceProviders)
  ? defaultSourceProviders
  : [];
const storageKey = "serialized-table.config";
const readMaxRows = Math.max(1, Math.min(defaultTableOptions?.maxRows ?? 1000, 5000));
const TABLE_ASSET_COLUMN_WIDTH = 220;
const TABLE_STATUS_COLUMN_WIDTH = 160;
const TABLE_MIN_DATA_COLUMN_WIDTH = 88;
const TABLE_MIN_EDGE_COLUMN_WIDTH = 120;
const TABLE_MAX_DATA_COLUMN_WIDTH = 220;
const TABLE_MAX_RESIZED_COLUMN_WIDTH = 640;
const TABLE_ASSET_COLUMN_KEY = "asset";
const TABLE_STATUS_COLUMN_KEY = "status";
const sourceRows = ref<SerializedTableSourceConfig[]>(cloneConfig(defaultSources));
const providerRows = ref<SerializedTableSourceConfig[]>([]);
const resolvedSourceRows = ref<SerializedTableSourceConfig[]>([]);
const columns = ref<SerializedTableColumnConfig[]>(cloneConfig(defaultColumns));
const columnWidthOverrides = ref<Record<string, number>>({});
const tableRows = ref<SerializedTableRow[]>([]);
const tableScrollerRef = ref<HTMLElement | null>(null);
const tableRef = ref<HTMLTableElement | null>(null);
const loading = ref(false);
const savingCellKey = ref("");
const statusText = ref("Loading table");
const errorText = ref("");
const loadProgress = ref<TableLoadProgress>({
  active: false,
  title: "",
  info: "",
  progress: 0,
});

const sourceCount = computed(() => resolvedSourceRows.value.length || sourceRows.value.length + providerRows.value.length);
const progressPercent = computed(() => `${Math.round(clampProgress(loadProgress.value.progress) * 100)}%`);
const progressWidth = computed(() => `${Math.round(clampProgress(loadProgress.value.progress) * 100)}%`);
const emptyStateText = computed(() => {
  if (!loadProgress.value.active) return statusText.value;
  return [loadProgress.value.title, loadProgress.value.info].filter((part) => part.trim()).join(" · ");
});
const assetColumnWidth = computed(() =>
  columnWidthFromOverride(TABLE_ASSET_COLUMN_KEY, TABLE_ASSET_COLUMN_WIDTH, TABLE_MIN_EDGE_COLUMN_WIDTH),
);
const statusColumnWidth = computed(() =>
  columnWidthFromOverride(TABLE_STATUS_COLUMN_KEY, TABLE_STATUS_COLUMN_WIDTH, TABLE_MIN_EDGE_COLUMN_WIDTH),
);
const columnWidths = computed(() => columns.value.map((column) =>
  columnWidthFromOverride(tableColumnKey(column), autoColumnWidth(column), TABLE_MIN_DATA_COLUMN_WIDTH),
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

let layoutCheckFrame = 0;
let lastLayoutProblemSignature = "";
let resizeState: TableColumnResizeState | null = null;
const resizingColumnKey = ref("");

function cloneConfig<T>(value: T): T {
  return JSON.parse(JSON.stringify(value));
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function setStatus(message: string) {
  statusText.value = message;
}

function setProgress(title: string, info: string, progress: number) {
  loadProgress.value = {
    active: true,
    title,
    info,
    progress: clampProgress(progress),
  };
  setStatus([title, info].filter((part) => part.trim()).join(" · "));
}

function clearProgress() {
  loadProgress.value = {
    active: false,
    title: "",
    info: "",
    progress: 0,
  };
}

function columnDisplayName(column: SerializedTableColumnConfig): string {
  return column.label || column.propertyPath || column.id;
}

function tableColumnKey(column: SerializedTableColumnConfig): string {
  return `column:${column.id || column.propertyPath || columnDisplayName(column)}`;
}

function clampColumnWidth(value: number, minWidth: number): number {
  if (!Number.isFinite(value)) return minWidth;
  return Math.min(TABLE_MAX_RESIZED_COLUMN_WIDTH, Math.max(minWidth, Math.round(value)));
}

function columnWidthFromOverride(key: string, fallback: number, minWidth: number): number {
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

function widthForCell(cell: SerializedCell): number {
  if (usesPropertyTree(cell)) return TABLE_MAX_DATA_COLUMN_WIDTH;

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
  const cellWidth = tableRows.value.reduce((maxWidth, row) => {
    const cell = row.cells.find((item) => item.columnId === column.id);
    return cell ? Math.max(maxWidth, widthForCell(cell)) : maxWidth;
  }, 0);
  return Math.min(
    TABLE_MAX_DATA_COLUMN_WIDTH,
    Math.max(TABLE_MIN_DATA_COLUMN_WIDTH, labelWidth, cellWidth),
  );
}

function normalizeStoredColumnWidths(value: unknown): Record<string, number> {
  if (!isPlainRecord(value)) return {};
  const result: Record<string, number> = {};
  for (const [key, width] of Object.entries(value)) {
    if (typeof width !== "number" || !Number.isFinite(width)) continue;
    const minWidth = key === TABLE_ASSET_COLUMN_KEY || key === TABLE_STATUS_COLUMN_KEY
      ? TABLE_MIN_EDGE_COLUMN_WIDTH
      : TABLE_MIN_DATA_COLUMN_WIDTH;
    result[key] = clampColumnWidth(width, minWidth);
  }
  return result;
}

async function readStoredConfigObject(): Promise<Record<string, unknown>> {
  try {
    const parsed = await view.storage.get(storageKey);
    return isPlainRecord(parsed) ? parsed : {};
  } catch (error) {
    console.error("[serialized-table] Stored config read failed", error);
    return {};
  }
}

async function loadStoredConfig() {
  const stored = await readStoredConfigObject();
  columnWidthOverrides.value = normalizeStoredColumnWidths(stored.columnWidths);
  scheduleTableLayoutCheck();
}

function persistColumnWidths() {
  void (async () => {
    const stored = await readStoredConfigObject();
    await view.storage.set(storageKey, {
      ...stored,
      columnWidths: columnWidthOverrides.value,
    });
  })().catch((error) => {
    console.error("[serialized-table] Stored config write failed", error);
  });
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
  document.body.classList.add("serialized-table-resizing");
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
  scheduleTableLayoutCheck();
}

function stopColumnResize(save: boolean) {
  if (save && resizeState) persistColumnWidths();
  resizeState = null;
  resizingColumnKey.value = "";
  document.body.classList.remove("serialized-table-resizing");
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
  persistColumnWidths();
  scheduleTableLayoutCheck();
}

function scheduleTableLayoutCheck() {
  if (layoutCheckFrame) window.cancelAnimationFrame(layoutCheckFrame);
  layoutCheckFrame = window.requestAnimationFrame(() => {
    layoutCheckFrame = 0;
    void nextTick().then(checkTableLayout);
  });
}

function checkTableLayout() {
  const table = tableRef.value;
  const scroller = tableScrollerRef.value;
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
    columns: columns.value.length,
    expectedWidth,
    tableWidth: Math.round(tableWidth),
    viewportWidth: scroller.clientWidth,
  });
}

function handleTableWheel(event: WheelEvent) {
  const scroller = tableScrollerRef.value;
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

function normalizeId(value: string, fallback: string): string {
  const normalized = value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return normalized || fallback;
}

function assetNameFromPath(path: string): string {
  const name = path.split("/").pop() || path;
  return name.replace(/\.[^.]+$/, "");
}

function normalizeSourceRow(
  source: SerializedTableSourceConfig,
  fallbackId: string,
): SerializedTableSourceConfig {
  const assetPath = source.assetPath || "";
  return {
    ...source,
    id: source.id || normalizeId(assetPath || fallbackId, fallbackId),
    label: source.label || (assetPath ? assetNameFromPath(assetPath) : fallbackId),
    sourceKind: source.sourceKind || "asset",
    componentIndex: source.componentIndex ?? 0,
  };
}

function sourceKey(source: SerializedTableSourceConfig): string {
  return [
    source.id,
    source.guid || "",
    source.assetPath || "",
    source.sourceKind || "asset",
    source.componentType || "",
    String(source.componentIndex ?? 0),
  ].join("|");
}

function dedupeSources(sources: SerializedTableSourceConfig[]): SerializedTableSourceConfig[] {
  const seen = new Set<string>();
  const rows: SerializedTableSourceConfig[] = [];
  for (const source of sources) {
    const key = sourceKey(source);
    if (seen.has(key)) continue;
    seen.add(key);
    rows.push(source);
  }
  return rows;
}

function sourceRowsFromAssets(
  results: AssetSearchResult[],
  defaults: Partial<SerializedTableSourceConfig> = {},
): SerializedTableSourceConfig[] {
  return results.map((result, index) => normalizeSourceRow({
    ...defaults,
    id: defaults.id
      ? `${defaults.id}-${index + 1}`
      : normalizeId(result.path.replace(/\.[^.]+$/, ""), `asset-${index + 1}`),
    label: defaults.label || result.name.replace(/\.[^.]+$/, ""),
    assetPath: result.path,
    componentIndex: defaults.componentIndex ?? 0,
  }, `asset-${index + 1}`));
}

function sourceProviderLabel(provider: SerializedTableSourceProvider, index: number): string {
  if (Array.isArray(provider)) return `Provider ${index + 1}`;
  if (typeof provider === "function") return provider.name || `Provider ${index + 1}`;
  return provider.label || provider.id || `Provider ${index + 1}`;
}

function normalizeProviderRows(
  rows: SerializedTableSourceConfig[],
  provider: SerializedTableSourceProvider,
  index: number,
): SerializedTableSourceConfig[] {
  const providerId = Array.isArray(provider)
    ? `provider-${index + 1}`
    : typeof provider === "function"
      ? normalizeId(provider.name || "", `provider-${index + 1}`)
      : normalizeId(provider.id || provider.label || "", `provider-${index + 1}`);
  return rows.map((row, rowIndex) => {
    const normalized = normalizeSourceRow(row, `${providerId}-${rowIndex + 1}`);
    return {
      ...normalized,
      id: normalized.id.startsWith(`${providerId}-`) ? normalized.id : `${providerId}-${normalized.id}`,
    };
  });
}

async function resolveSourceProviders(): Promise<SerializedTableSourceConfig[]> {
  const rows: SerializedTableSourceConfig[] = [];
  const errors: string[] = [];
  const context = {
    searchAssets: (query: string, roots = ["Assets", "Packages"], limit = 1000) =>
      view.assets.search(query, roots, limit) as Promise<AssetSearchResult[]>,
    fromAssetResults: sourceRowsFromAssets,
  };

  for (let index = 0; index < sourceProviders.length; index += 1) {
    const provider = sourceProviders[index];
    const label = sourceProviderLabel(provider, index);
    setProgress("Sources", `${label} (${index + 1}/${sourceProviders.length})`, 0.18 + (index / sourceProviders.length) * 0.22);
    try {
      const nextRows = Array.isArray(provider)
        ? provider
        : typeof provider === "function"
          ? await provider(context)
          : await provider.resolve(context);
      if (Array.isArray(nextRows)) {
        rows.push(...normalizeProviderRows(nextRows, provider, index));
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`[serialized-table] Source provider failed: ${label}`, error);
      errors.push(`${label}: ${message}`);
    }
  }

  providerRows.value = dedupeSources(rows);
  if (errors.length) errorText.value = errors.join("\n");
  return providerRows.value;
}

async function resolveSourcesForRead(): Promise<SerializedTableSourceConfig[]> {
  const scriptedRows = await resolveSourceProviders();
  const manualRows = sourceRows.value.map((source, index) => normalizeSourceRow(source, `row-${index + 1}`));
  const rows = dedupeSources([...manualRows, ...scriptedRows]);
  resolvedSourceRows.value = rows;
  return rows;
}

async function refresh() {
  loading.value = true;
  errorText.value = "";
  try {
    setProgress("Sources", "Resolving table rows", 0.12);
    const sources = await resolveSourcesForRead();
    if (!sources.length || !columns.value.length) {
      tableRows.value = [];
      setStatus("No configured sources or columns");
      return;
    }
    setProgress("Request", `${sources.length} sources, ${columns.value.length} columns`, 0.42);
    setProgress("Unity", "Preparing C# reader", 0.64);
    const response = await view.callScript("SerializedTableApi", "Read", {
      sources,
      columns: columns.value,
      maxRows: readMaxRows,
    }) as SerializedTableResponse;
    setProgress("Table", "Rendering rows", 0.9);
    tableRows.value = Array.isArray(response.rows) ? response.rows : [];
    setStatus(response.message || "Ready");
  } catch (error) {
    errorText.value = error instanceof Error ? error.message : String(error);
    console.error("[serialized-table] Read failed", error);
    setStatus("Read failed");
  } finally {
    loading.value = false;
    clearProgress();
    scheduleTableLayoutCheck();
  }
}

function sourceForRow(rowId: string): SerializedTableSourceConfig | null {
  return resolvedSourceRows.value.find((source) => source.id === rowId) ?? null;
}

function columnForCell(cell: SerializedCell): SerializedTableColumnConfig | null {
  return columns.value.find((column) => column.id === cell.columnId) ?? null;
}

function cellKey(row: SerializedTableRow, cell: SerializedCell): string {
  return `${row.id}:${cell.columnId}`;
}

async function commitCell(row: SerializedTableRow, cell: SerializedCell, value: unknown, propertyPath = "") {
  const source = sourceForRow(row.id);
  const column = columnForCell(cell);
  if (!source || !column || !cell.editable || savingCellKey.value) return;
  const key = cellKey(row, cell);
  savingCellKey.value = key;
  errorText.value = "";
  try {
    setProgress("Write", "Saving serialized property", 0.5);
    const result = await view.callScript("SerializedTableApi", "Write", {
      source,
      column,
      propertyPath: propertyPath || cell.propertyPath,
      valueJson: JSON.stringify(value),
    }) as SerializedTableWriteResponse;
    const nextCell = result.cell;
    const rowIndex = tableRows.value.findIndex((item) => item.id === row.id);
    if (rowIndex >= 0 && nextCell) {
      const cellIndex = tableRows.value[rowIndex].cells.findIndex((item) => item.columnId === cell.columnId);
        if (cellIndex >= 0) tableRows.value[rowIndex].cells[cellIndex] = nextCell;
    }
    setStatus(result.message || "Saved");
  } catch (error) {
    errorText.value = error instanceof Error ? error.message : String(error);
    console.error("[serialized-table] Write failed", error);
    setStatus("Write failed");
  } finally {
    savingCellKey.value = "";
    clearProgress();
  }
}

async function commitCellProperty(row: SerializedTableRow, cell: SerializedCell, event: SerializedPropertyCommitEvent) {
  await commitCell(row, cell, event.value, event.propertyPath);
}

function usesPropertyTree(cell: SerializedCell): boolean {
  return !!cell.isArray || !!cell.isManagedReference || (Array.isArray(cell.children) && cell.children.length > 0);
}

function statusForCell(row: SerializedTableRow, cell: SerializedCell): string {
  if (savingCellKey.value === cellKey(row, cell)) return "Saving";
  return cell.message || (cell.editable ? "Editable" : "Read only");
}

watch(columns, scheduleTableLayoutCheck, { deep: true });
watch(tableRows, scheduleTableLayoutCheck, { deep: true });

onMounted(() => {
  window.addEventListener("resize", scheduleTableLayoutCheck);
  void loadStoredConfig();
  void refresh();
  scheduleTableLayoutCheck();
});

onBeforeUnmount(() => {
  if (layoutCheckFrame) window.cancelAnimationFrame(layoutCheckFrame);
  stopColumnResize(false);
  window.removeEventListener("resize", scheduleTableLayoutCheck);
});
</script>

<template>
  <main class="view-shell serialized-table-view" data-locus-template="serialized-table">
    <header class="view-toolbar">
      <button type="button" :disabled="loading" @click="refresh">
        {{ loading ? "Reading" : "Refresh" }}
      </button>
    </header>

    <section class="table-workspace">
      <section class="table-pane">
        <div ref="tableScrollerRef" class="serialized-table" @wheel.capture="handleTableWheel">
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
                <th class="asset-col resizable-col" :class="{ resizing: resizingColumnKey === TABLE_ASSET_COLUMN_KEY }">
                  <span class="column-label">Asset</span>
                  <button
                    type="button"
                    class="column-resize-handle"
                    :title="resizeHandleLabel('Asset')"
                    @pointerdown.prevent.stop="startColumnResize($event, TABLE_ASSET_COLUMN_KEY, assetColumnWidth, TABLE_MIN_EDGE_COLUMN_WIDTH)"
                    @dblclick.prevent.stop="resetColumnWidth(TABLE_ASSET_COLUMN_KEY)"
                  ></button>
                </th>
                <th
                  v-for="(column, index) in columns"
                  :key="column.id"
                  class="value-col resizable-col"
                  :class="{ resizing: resizingColumnKey === tableColumnKey(column) }"
                  :title="columnDisplayName(column)"
                >
                  <span class="column-label">{{ columnDisplayName(column) }}</span>
                  <button
                    type="button"
                    class="column-resize-handle"
                    :title="resizeHandleLabel(columnDisplayName(column))"
                    @pointerdown.prevent.stop="startColumnResize($event, tableColumnKey(column), columnWidths[index], TABLE_MIN_DATA_COLUMN_WIDTH)"
                    @dblclick.prevent.stop="resetColumnWidth(tableColumnKey(column))"
                  ></button>
                </th>
                <th class="status-col resizable-col" :class="{ resizing: resizingColumnKey === TABLE_STATUS_COLUMN_KEY }">
                  <span class="column-label">Status</span>
                  <button
                    type="button"
                    class="column-resize-handle"
                    :title="resizeHandleLabel('Status')"
                    @pointerdown.prevent.stop="startColumnResize($event, TABLE_STATUS_COLUMN_KEY, statusColumnWidth, TABLE_MIN_EDGE_COLUMN_WIDTH)"
                    @dblclick.prevent.stop="resetColumnWidth(TABLE_STATUS_COLUMN_KEY)"
                  ></button>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in tableRows" :key="row.id">
                <td class="asset-cell">
                  <div class="asset-cell-content">
                    <span>{{ row.label }}</span>
                    <small>{{ row.assetPath }}</small>
                  </div>
                </td>
                <td v-for="cell in row.cells" :key="cell.columnId" class="value-cell" :class="{ error: !cell.ok }">
                  <UnitySerializedPropertyTree
                    v-if="usesPropertyTree(cell)"
                    :property="cell"
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
          <div v-if="!tableRows.length" class="empty-state">
            {{ emptyStateText }}
          </div>
        </div>
      </section>
    </section>
    <footer class="table-statusbar" :class="{ error: !!errorText, loading: loadProgress.active }">
      <div v-if="loadProgress.active" class="table-progress-status" aria-live="polite">
        <div class="table-progress-row">
          <span class="table-progress-title">{{ loadProgress.title }}</span>
          <span v-if="loadProgress.info" class="table-progress-info">{{ loadProgress.info }}</span>
          <span class="table-progress-percent">{{ progressPercent }}</span>
        </div>
        <div class="table-progress-track" aria-hidden="true">
          <div class="table-progress-fill" :style="{ width: progressWidth }"></div>
        </div>
      </div>
      <span v-else>{{ errorText || statusText }}</span>
      <span>{{ tableRows.length }} rows · {{ sourceCount }} sources · {{ columns.length }} columns</span>
    </footer>
  </main>
</template>
"####
    .to_string()
}

pub(super) fn style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.view-shell {
  width: 100vw;
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 0;
  overflow: hidden;
  background: var(--bg-color);
}

.view-toolbar {
  flex-shrink: 0;
  min-height: 44px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

th {
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

button,
input,
select {
  min-height: 28px;
  min-width: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
}

button {
  padding: 0 10px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  cursor: pointer;
}

button:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

button:disabled {
  cursor: default;
  opacity: 0.55;
}

input,
select {
  padding: 0 8px;
}

input:focus,
select:focus {
  outline: none;
  border-color: var(--accent-color);
}

input[type="number"] {
  appearance: textfield;
  -moz-appearance: textfield;
}

input[type="number"]::-webkit-inner-spin-button,
input[type="number"]::-webkit-outer-spin-button {
  margin: 0;
  -webkit-appearance: none;
}

.table-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: flex;
  overflow: hidden;
}

.asset-cell span,
.asset-cell small,
.status-cell span,
.status-cell small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.asset-cell span {
  font-weight: 600;
}

.asset-cell small,
.status-cell small {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.table-pane {
  flex: 1;
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.serialized-table {
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
  text-align: left;
  overflow: hidden;
}

th:last-child,
td:last-child {
  border-right: 0;
}

td {
  overflow: hidden;
}

.asset-col {
  width: 220px;
}

.status-col {
  width: 160px;
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

.column-resize-handle:hover {
  border-color: transparent;
  background: transparent;
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

body.serialized-table-resizing,
body.serialized-table-resizing * {
  cursor: col-resize !important;
  user-select: none !important;
}

tr:hover td {
  background: var(--hover-bg);
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

.value-cell input:not([type="checkbox"]),
.value-cell select,
.value-cell .unity-property-tree,
.value-cell .unity-property-editor {
  max-width: 100%;
  min-width: 0;
  width: 100%;
  font-size: 12px;
}

.value-cell input[type="checkbox"],
.value-cell .unity-bool-field {
  width: 14px;
  height: 14px;
  min-height: 14px;
  padding: 0;
}

.value-cell.error input {
  border-color: var(--status-danger-border);
}

.empty-state {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.table-statusbar {
  min-height: 22px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 8px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 64%, black 36%);
  background: color-mix(in srgb, var(--bg-color) 72%, black 28%);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.table-statusbar span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.table-progress-status {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 3px;
}

.table-progress-row {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, auto) minmax(0, 1fr) auto;
  align-items: baseline;
  gap: 8px;
}

.table-progress-title {
  color: var(--text-color);
  font-weight: 600;
}

.table-progress-info {
  color: var(--text-secondary);
}

.table-progress-percent {
  color: var(--text-secondary);
  font-size: 11px;
}

.table-progress-track {
  height: 3px;
  overflow: hidden;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.table-progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent-color);
  transition: width 0.16s ease;
}

.table-statusbar.error {
  color: var(--status-danger-fg);
}
"#
    .to_string()
}

pub(super) fn view_api_cs() -> String {
    r##"using System;
using System.Collections.Generic;
using System.Globalization;
using UnityEditor;
using UnityEngine;

public static class SerializedTableApi
{
    [Serializable]
    public sealed class ReadArgs
    {
        public SerializedTableSourceConfig[] sources;
        public SerializedTableColumnConfig[] columns;
        public int maxRows;
    }

    [Serializable]
    public sealed class WriteArgs
    {
        public SerializedTableSourceConfig source;
        public SerializedTableColumnConfig column;
        public string propertyPath;
        public string valueJson;
    }

    [Serializable]
    public sealed class SerializedTableSourceConfig
    {
        public string id;
        public string label;
        public string assetPath;
        public string guid;
        public string sourceKind;
        public string componentType;
        public int componentIndex;
        public SerializedTablePropertyOverride[] propertyOverrides;
    }

    [Serializable]
    public sealed class SerializedTablePropertyOverride
    {
        public string columnId;
        public string propertyPath;
    }

    [Serializable]
    public sealed class SerializedTableColumnConfig
    {
        public string id;
        public string label;
        public string propertyPath;
    }

    private sealed class SerializedTableRow
    {
        public string id;
        public string label;
        public string assetPath;
        public string sourceKind;
        public string typeName;
        public string status;
        public string message;
        public SerializedCell[] cells;
    }

    private sealed class SerializedCell
    {
        public string columnId;
        public string label;
        public string propertyPath;
        public string name;
        public string type;
        public string valueType;
        public string fieldTypeFullName;
        public string fieldTypeAssembly;
        public object value;
        public string displayValue;
        public bool editable;
        public bool hasChildren;
        public bool isArray;
        public int arraySize;
        public bool ok;
        public string message;
        public bool isFlagsEnum;
        public int enumValueIndex;
        public long enumValueFlag;
        public Locus.LocusBridge.SerializedEnumOption[] enumOptions;
        public Locus.LocusBridge.SerializedPropertySnapshot[] children;
        public bool isManagedReference;
        public string managedReferenceFullTypename;
        public string managedReferenceFieldTypename;
        public string managedReferenceDisplayName;
        public Locus.LocusBridge.SerializedManagedReferenceTypeOption[] managedReferenceTypes;
    }

    private sealed class SerializedTableResponse
    {
        public bool ok;
        public string message;
        public SerializedTableRow[] rows;
    }

    private sealed class SerializedTableWriteResponse
    {
        public bool ok;
        public string message;
        public SerializedCell cell;
    }

    private sealed class ResolvedSource
    {
        public UnityEngine.Object obj;
        public string assetPath;
        public string sourceKind;
        public string typeName;
    }

    public static object Read(ReadArgs args)
    {
        args = args ?? new ReadArgs();
        SerializedTableSourceConfig[] sources = args.sources ?? new SerializedTableSourceConfig[0];
        SerializedTableColumnConfig[] columns = args.columns ?? new SerializedTableColumnConfig[0];
        int maxRows = args.maxRows > 0 ? Math.Min(args.maxRows, 5000) : 1000;
        var rows = new List<SerializedTableRow>();

        for (int i = 0; i < sources.Length && rows.Count < maxRows; i++)
            rows.Add(ReadSourceRow(sources[i], columns));

        return new SerializedTableResponse
        {
            ok = true,
            message = rows.Count == 0 ? "No configured assets" : "Ready",
            rows = rows.ToArray()
        };
    }

    public static object Write(WriteArgs args)
    {
        if (args == null)
            throw new Exception("Write arguments are required");
        if (args.source == null)
            throw new Exception("Source row is required");
        if (args.column == null)
            throw new Exception("Column is required");

        ResolvedSource source = ResolveSource(args.source);
        string rootPropertyPath = ResolvePropertyPath(args.source, args.column);
        string propertyPath = !string.IsNullOrWhiteSpace(args.propertyPath)
            ? args.propertyPath
            : rootPropertyPath;
        var serialized = new SerializedObject(source.obj);
        serialized.Update();
        SerializedProperty prop = serialized.FindProperty(propertyPath);
        if (prop == null)
            throw new Exception("SerializedProperty not found: " + propertyPath);
        if (!Locus.LocusBridge.IsSerializedPropertyWritable(prop))
            throw new Exception("SerializedProperty is read only: " + propertyPath);

        Locus.LocusBridge.SetSerializedPropertyValue(prop, args.valueJson);
        ApplySerializedChanges(serialized, source.obj);

        SerializedProperty updated = serialized.FindProperty(rootPropertyPath);
        return new SerializedTableWriteResponse
        {
            ok = true,
            message = "Saved",
            cell = BuildCell(args.source, args.column, updated != null ? updated : prop)
        };
    }

    private static SerializedTableRow ReadSourceRow(
        SerializedTableSourceConfig sourceConfig,
        SerializedTableColumnConfig[] columns)
    {
        if (sourceConfig == null)
            return ErrorRow(null, columns, "Source row is empty");

        try
        {
            ResolvedSource source = ResolveSource(sourceConfig);
            var serialized = new SerializedObject(source.obj);
            serialized.Update();
            var cells = new SerializedCell[columns.Length];
            for (int i = 0; i < columns.Length; i++)
            {
                SerializedTableColumnConfig column = columns[i];
                string propertyPath = ResolvePropertyPath(sourceConfig, column);
                SerializedProperty prop = string.IsNullOrWhiteSpace(propertyPath)
                    ? null
                    : serialized.FindProperty(propertyPath);
                cells[i] = prop == null
                    ? ErrorCell(column, propertyPath, "SerializedProperty not found")
                    : BuildCell(sourceConfig, column, prop);
            }

            return new SerializedTableRow
            {
                id = SafeId(sourceConfig),
                label = SafeLabel(sourceConfig),
                assetPath = source.assetPath,
                sourceKind = source.sourceKind,
                typeName = source.typeName,
                status = "ok",
                message = "Ready",
                cells = cells
            };
        }
        catch (Exception ex)
        {
            return ErrorRow(sourceConfig, columns, ex.Message);
        }
    }

    private static SerializedTableRow ErrorRow(
        SerializedTableSourceConfig sourceConfig,
        SerializedTableColumnConfig[] columns,
        string message)
    {
        var cells = new SerializedCell[columns.Length];
        for (int i = 0; i < columns.Length; i++)
            cells[i] = ErrorCell(columns[i], ResolvePropertyPath(sourceConfig, columns[i]), message);

        return new SerializedTableRow
        {
            id = SafeId(sourceConfig),
            label = SafeLabel(sourceConfig),
            assetPath = sourceConfig != null ? sourceConfig.assetPath ?? "" : "",
            sourceKind = sourceConfig != null ? sourceConfig.sourceKind ?? "" : "",
            typeName = "",
            status = "error",
            message = message,
            cells = cells
        };
    }

    private static SerializedCell BuildCell(
        SerializedTableSourceConfig source,
        SerializedTableColumnConfig column,
        SerializedProperty prop)
    {
        Locus.LocusBridge.SerializedPropertySnapshot snapshot =
            Locus.LocusBridge.SnapshotSerializedProperty(prop, 3, 32);
        bool writable = Locus.LocusBridge.IsSerializedPropertyWritable(prop);
        return new SerializedCell
        {
            columnId = SafeColumnId(column),
            label = SafeColumnLabel(column),
            propertyPath = snapshot.propertyPath,
            name = snapshot.name,
            type = snapshot.type,
            valueType = snapshot.valueType,
            fieldTypeFullName = snapshot.fieldTypeFullName,
            fieldTypeAssembly = snapshot.fieldTypeAssembly,
            value = snapshot.value,
            displayValue = snapshot.displayValue,
            editable = writable,
            hasChildren = snapshot.hasChildren,
            isArray = snapshot.isArray,
            arraySize = snapshot.arraySize,
            ok = true,
            message = writable ? "Editable" : "Read only",
            isFlagsEnum = snapshot.isFlagsEnum,
            enumValueIndex = snapshot.enumValueIndex,
            enumValueFlag = snapshot.enumValueFlag,
            enumOptions = snapshot.enumOptions,
            children = snapshot.children,
            isManagedReference = snapshot.isManagedReference,
            managedReferenceFullTypename = snapshot.managedReferenceFullTypename,
            managedReferenceFieldTypename = snapshot.managedReferenceFieldTypename,
            managedReferenceDisplayName = snapshot.managedReferenceDisplayName,
            managedReferenceTypes = snapshot.managedReferenceTypes
        };
    }

    private static SerializedCell ErrorCell(
        SerializedTableColumnConfig column,
        string propertyPath,
        string message)
    {
        return new SerializedCell
        {
            columnId = SafeColumnId(column),
            label = SafeColumnLabel(column),
            propertyPath = propertyPath ?? "",
            name = "",
            type = "Error",
            valueType = "Error",
            fieldTypeFullName = "",
            fieldTypeAssembly = "",
            value = null,
            displayValue = "",
            editable = false,
            hasChildren = false,
            isArray = false,
            arraySize = -1,
            ok = false,
            message = message,
            isFlagsEnum = false,
            enumValueIndex = -1,
            enumValueFlag = 0,
            enumOptions = new Locus.LocusBridge.SerializedEnumOption[0],
            children = new Locus.LocusBridge.SerializedPropertySnapshot[0],
            isManagedReference = false,
            managedReferenceFullTypename = "",
            managedReferenceFieldTypename = "",
            managedReferenceDisplayName = "",
            managedReferenceTypes = new Locus.LocusBridge.SerializedManagedReferenceTypeOption[0]
        };
    }

    private static ResolvedSource ResolveSource(SerializedTableSourceConfig source)
    {
        string assetPath = ResolveAssetPath(source);
        if (string.IsNullOrWhiteSpace(assetPath))
            throw new Exception("Asset path is required");

        UnityEngine.Object asset = AssetDatabase.LoadMainAssetAtPath(assetPath);
        if (asset == null)
            throw new Exception("Asset not found: " + assetPath);

        string kind = string.IsNullOrWhiteSpace(source.sourceKind) ? "asset" : source.sourceKind.Trim();
        UnityEngine.Object obj = asset;
        if (string.Equals(kind, "component", StringComparison.OrdinalIgnoreCase))
            obj = ResolveComponent(asset, source);

        Type type = obj.GetType();
        return new ResolvedSource
        {
            obj = obj,
            assetPath = assetPath,
            sourceKind = kind,
            typeName = type.FullName ?? type.Name
        };
    }

    private static UnityEngine.Object ResolveComponent(
        UnityEngine.Object asset,
        SerializedTableSourceConfig source)
    {
        GameObject go = asset as GameObject;
        if (go == null)
            throw new Exception("Component source requires a prefab or GameObject asset");

        Component[] components = go.GetComponents<Component>();
        string componentType = source.componentType ?? "";
        int targetIndex = source.componentIndex < 0 ? 0 : source.componentIndex;
        int matchIndex = 0;
        for (int i = 0; i < components.Length; i++)
        {
            Component component = components[i];
            if (component == null)
                continue;
            Type type = component.GetType();
            if (!Locus.LocusBridge.TypeMatches(type, componentType))
                continue;
            if (matchIndex == targetIndex)
                return component;
            matchIndex++;
        }
        throw new Exception("Component not found: " + componentType);
    }

    private static string ResolveAssetPath(SerializedTableSourceConfig source)
    {
        if (source == null)
            return "";
        if (!string.IsNullOrWhiteSpace(source.guid))
        {
            string guidPath = AssetDatabase.GUIDToAssetPath(source.guid.Trim());
            if (!string.IsNullOrWhiteSpace(guidPath))
                return guidPath;
        }
        return source.assetPath ?? "";
    }

    private static string ResolvePropertyPath(
        SerializedTableSourceConfig source,
        SerializedTableColumnConfig column)
    {
        if (column == null)
            return "";
        if (source != null && source.propertyOverrides != null)
        {
            for (int i = 0; i < source.propertyOverrides.Length; i++)
            {
                SerializedTablePropertyOverride item = source.propertyOverrides[i];
                if (item != null &&
                    string.Equals(item.columnId, column.id, StringComparison.Ordinal) &&
                    !string.IsNullOrWhiteSpace(item.propertyPath))
                    return item.propertyPath;
            }
        }
        return column.propertyPath ?? "";
    }

    private static string SafeId(SerializedTableSourceConfig source)
    {
        if (source == null || string.IsNullOrWhiteSpace(source.id))
            return "source";
        return source.id;
    }

    private static string SafeLabel(SerializedTableSourceConfig source)
    {
        if (source == null)
            return "Source";
        if (!string.IsNullOrWhiteSpace(source.label))
            return source.label;
        if (!string.IsNullOrWhiteSpace(source.assetPath))
            return System.IO.Path.GetFileName(source.assetPath);
        return SafeId(source);
    }

    private static string SafeColumnId(SerializedTableColumnConfig column)
    {
        if (column == null || string.IsNullOrWhiteSpace(column.id))
            return "property";
        return column.id;
    }

    private static string SafeColumnLabel(SerializedTableColumnConfig column)
    {
        if (column == null)
            return "Property";
        if (!string.IsNullOrWhiteSpace(column.label))
            return column.label;
        return column.propertyPath ?? SafeColumnId(column);
    }

    private static void MarkObjectDirty(UnityEngine.Object obj)
    {
        if (obj == null)
            return;
        EditorUtility.SetDirty(obj);
        AssetDatabase.SaveAssetIfDirty(obj);
    }

    private static bool ApplySerializedChanges(SerializedObject serialized, UnityEngine.Object obj)
    {
        int undoGroup = Undo.GetCurrentGroup();
        Undo.SetCurrentGroupName("Locus Serialized Table");
        bool changed = serialized.ApplyModifiedProperties();
        if (changed)
        {
            RecordPrefabModifications(obj);
            MarkObjectDirty(obj);
            Undo.CollapseUndoOperations(undoGroup);
        }
        serialized.Update();
        return changed;
    }

    private static void RecordPrefabModifications(UnityEngine.Object obj)
    {
        if (obj == null)
            return;

        try
        {
            Component component = obj as Component;
            GameObject go = obj as GameObject;
            if (go == null && component != null)
                go = component.gameObject;
            if (go != null && PrefabUtility.GetNearestPrefabInstanceRoot(go) != null)
                PrefabUtility.RecordPrefabInstancePropertyModifications(obj);
        }
        catch
        {
        }
    }
}
"##
    .to_string()
}
