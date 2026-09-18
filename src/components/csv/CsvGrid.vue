<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { TabulatorFull as Tabulator, type CellComponent, type ColumnDefinition, type RowComponent } from "tabulator-tables";
import "tabulator-tables/dist/css/tabulator.css";
import { t } from "../../i18n";
import { csvDocumentChangedRows, parseCsvClipboard, serializeCsvClipboard,
  type CsvCellEdit, type CsvDocument } from "../../document/csv/csvDocument";
import type { CsvView } from "../../document/csv/csvView";
import { applyCsvCellStyle, resolveCsvCellStyle } from "../../document/csv/csvStyles";
import { applyCsvExcelText, fitCsvExcelText } from "../../document/csv/csvExcelStyles";
import { csvColumnLabel, projectCsvColumns, projectCsvRowIndices, retainCsvProjectedColumns, type CsvProjectedColumn } from "../../document/csv/csvGridProjection";
import { csvCellEditor, editCsvCellText, focusCsvCellInput, seedCsvCellInput, type CsvCellEditPoint } from "./csvCellEditor";
import { csvMergeAt, csvMergeEdits, orderCsvMergeColumns, type CsvMerge } from "../../document/csv/csvMerges";
import { createCsvMergeLayer, projectCsvMerges, type CsvDisplayedMerge } from "./csvMergeLayer";
import { createCsvContentSizer } from "./csvContentSizing";
import { observeCsvGridResize } from "./csvGridResize";
import { CsvHorizontalRenderer } from "./csvHorizontalRenderer";
import { installCsvRangeSelection, scrollCsvColumnIntoView } from "./csvRangeSelection";

const props = withDefaults(defineProps<{ document: CsvDocument; view: CsvView; readOnly?: boolean; active?: boolean }>(), { active: true });
const emit = defineEmits<{
  edit: [edits: CsvCellEdit[]]; viewChange: [view: CsvView];
  selectionChange: [selection: { row: number; column: number }];
  contextMenu: [event: MouseEvent]; error: [error: unknown];
  undo: []; redo: []; save: []; find: [];
  zoomChange: [zoom: number];
}>();
export interface CsvGridSnapshot { row: number; column: number; endRow?: number; endColumn?: number; scrollTop: number; scrollLeft: number; zoom?: number }
type Position = { row: number; column: number };
const host = ref<HTMLElement | null>(null);
const zoom = ref(1);
let contentSizer: ReturnType<typeof createCsvContentSizer> | null = null;
let zoomFrame: number | null = null;
let requestedZoom = 1;
let columnAnchor: number | null = null;
let table: Tabulator | null = null;
let updating = false;
let built = false;
let disposed = false;
let refreshRequested = false;
let redrawRequested = false;
let viewportResize: ReturnType<typeof observeCsvGridResize> | null = null;
let layoutKey = "";
let dataKey = "";
let lastDocument: CsvDocument | null = null;
let selection: Position = { row: 0, column: 0 };
let keyboardAnchor: Position | null = null;
let pendingSelection: Position | null = null;
let pendingSnapshot: CsvGridSnapshot | null = null;
let projected: CsvProjectedColumn[] = [];
let rowIndices: number[] = [];
let displayedMerges: CsvDisplayedMerge[] = [];
let mergeLayer: ReturnType<typeof createCsvMergeLayer> | null = null;
let normalizingRange = false;
let editPoint: CsvCellEditPoint | undefined;
const rowHeights = new Map<number, number>();

function displayedBounds() {
  const range = table?.getRanges()[0];
  if (!range) return null;
  const visible = projected.filter((column) => !column.hidden);
  const fields = new Set(range.getColumns().map((column) => column.getField()));
  let left = visible.findIndex((column) => fields.has(column.id));
  let right = visible.length - 1;
  while (right >= 0 && !fields.has(visible[right]!.id)) right--;
  let top = range.getTopEdge(), bottom = range.getBottomEdge();
  if (left < 0 || right < 0) return null;
  let changed = true;
  while (changed) {
    changed = false;
    for (const merge of displayedMerges) if (merge.rows[0] <= bottom && merge.rows[1] >= top && merge.columns[0] <= right && merge.columns[1] >= left) {
      const next: [number, number, number, number] = [Math.min(top, merge.rows[0]), Math.max(bottom, merge.rows[1]), Math.min(left, merge.columns[0]), Math.max(right, merge.columns[1])];
      if (next[0] !== top || next[1] !== bottom || next[2] !== left || next[3] !== right) {
        [top, bottom, left, right] = next; changed = true;
      }
    }
  }
  return { top, bottom, left, right, visible };
}

function selectedCells(): CellComponent[][] { return table?.getRanges()[0]?.getStructuredCells() ?? []; }
function selectedRows(): number[] {
  const bounds = displayedBounds();
  return bounds ? rowIndices.slice(bounds.top, bounds.bottom + 1) : [];
}
function selectedColumns(): number[] {
  const bounds = displayedBounds();
  return bounds ? bounds.visible.slice(bounds.left, bounds.right + 1).map((column) => column.sourceIndex) : [];
}
function selectionBounds(): { start: Position; end: Position } | null {
  const range = displayedBounds();
  if (!range) return null;
  // Tabulator 6.5's getBounds also enumerates all cells (and returns internal
  // Cells, despite its public type). Read row edges and column identities only.
  const columns = selectedColumns();
  const row = rowIndices[range.top], endRow = rowIndices[range.bottom];
  const column = columns[0], endColumn = columns[columns.length - 1];
  return row != null && endRow != null && column != null && endColumn != null
    ? { start: { row, column }, end: { row: endRow, column: endColumn } } : null;
}
function mergeSelection(): CsvMerge | null {
  const rows = selectedRows(), columns = selectedColumns();
  if (!rows.length || !columns.length || !rows.every((row, index) => !index || row === rows[index - 1]! + 1)
    || !columns.every((column, index) => !index || column > columns[index - 1]!)) return null;
  const range: CsvMerge = { rows: [rows[0]!, rows[rows.length - 1]!], columns: [columns[0]!, columns[columns.length - 1]!] };
  if (projected.some((column) => !column.hidden && column.sourceIndex >= range.columns[0] && column.sourceIndex <= range.columns[1]
    && !columns.includes(column.sourceIndex))) return null;
  for (const merge of props.view.merges ?? []) if (rows.some((row) => row >= merge.rows[0] && row <= merge.rows[1])
    && columns.some((column) => column >= merge.columns[0] && column <= merge.columns[1])) {
    range.columns = [Math.min(range.columns[0], merge.columns[0]), Math.max(range.columns[1], merge.columns[1])];
  }
  return range.rows[0] === range.rows[1] && range.columns[0] === range.columns[1] ? null : range;
}
function visibleAnchor(position: Position): Position {
  const merge = csvMergeAt(props.view.merges, position);
  if (!merge) return position;
  const column = projected.find((column) => !column.hidden && column.sourceIndex >= merge.columns[0] && column.sourceIndex <= merge.columns[1]);
  return { row: merge.rows[0], column: column?.sourceIndex ?? position.column };
}
function normalizeRange(): void {
  if (!displayedMerges.length || normalizingRange || updating) return;
  const bounds = selectionBounds();
  if (!bounds) return;
  normalizingRange = true;
  try { setRange(bounds.start, bounds.end); } finally { normalizingRange = false; }
  mergeLayer?.schedule();
}
function reportError(error: unknown): void { if (!disposed) emit("error", error); }
function isActive(): boolean { return props.active !== false; }
function isEditing(): boolean { return !!host.value?.querySelector(".csv-cell-input"); }
function resumeAfterEditing(): void {
  // Let cellEdited update the CSV document before applying any deferred view.
  queueMicrotask(() => { if (refreshRequested) void refresh(); viewportResize?.resume(); });
}
function location(cell: CellComponent): Position {
  return { row: Number(cell.getRow().getData()._row), column: projected.find((column) => column.id === cell.getField())?.sourceIndex ?? 0 };
}
function cellAt(position: Position): CellComponent | null {
  const column = projected.find((column) => column.sourceIndex === position.column && !column.hidden);
  const row = table?.getRow(position.row);
  return row && column ? row.getCell(column.id) || null : null;
}
function announce(position: Position): void { selection = visibleAnchor(position); emit("selectionChange", selection); }
function selectColumn(column: number, extend = false): void {
  if (!rowIndices.length) return;
  columnAnchor = extend ? columnAnchor ?? selection.column : column;
  keyboardAnchor = null;
  setRange({ row: rowIndices[0]!, column: columnAnchor }, { row: rowIndices[rowIndices.length - 1]!, column });
  announce({ row: rowIndices[0]!, column });
  focus();
}
function setRange(start: Position, end = start): void {
  const first = cellAt(start), last = cellAt(end);
  if (!first || !last || !table) return;
  const range = table.getRanges()[0];
  if (range) range.setBounds(first, last);
  else { table.addRange(first, last); table.getRanges()[0]?.setBounds(first, last); }
}
async function focusCell(position: Position, anchor?: Position): Promise<void> {
  const currentTable = table;
  if (!currentTable || disposed) return;
  position = visibleAnchor(position);
  const cell = cellAt(position);
  if (!cell) return;
  await cell.getRow().scrollTo("nearest", false);
  if (disposed || table !== currentTable || updating) return;
  if (scrollCsvColumnIntoView(currentTable, cell.getColumn())) {
    // Wait for horizontal virtualization before focusing the destination cell.
    await new Promise<void>((resolve) => host.value!.ownerDocument.defaultView!.requestAnimationFrame(() => resolve()));
  }
  if (disposed || table !== currentTable || updating) return;
  setRange(anchor ?? position, position);
  normalizeRange();
  announce(position);
  cell.getElement().focus({ preventScroll: true });
}
function columns(): ColumnDefinition[] {
  let visibleIndex = 0;
  const variableHeight = true;
  return projected.map((column) => {
    const ordinal = visibleIndex;
    if (!column.hidden) visibleIndex++;
    return {
      field: column.id, title: csvColumnLabel(ordinal), width: column.width * zoom.value, minWidth: 48 * zoom.value,
      // Hidden columns must retain prefix membership: Tabulator switches to
      // right-frozen mode after the first non-frozen definition, even if hidden.
      visible: !column.hidden, frozen: ordinal < props.view.frozenColumns,
      // Body handles are recreated for every rendered cell on every scroll or
      // redraw. Column resizing belongs to the header, as in a worksheet.
      resizable: "header", headerSort: false, headerHozAlign: "center",
      headerClick: (event) => selectColumn(column.sourceIndex, event instanceof MouseEvent && event.shiftKey),
      editor: csvCellEditor, editable: (cell) => !props.readOnly && !csvMergeAt(props.view.merges, location(cell)),
      editorParams: () => ({ newline: props.document.newline, point: editPoint, navigate: (cell: CellComponent, x: number, y: number) => void navigate(location(cell), x, y).catch(reportError) }),
      formatter: (cell, _params, onRendered) => {
        const element = cell.getElement();
        const style = props.view.styles?.length ? resolveCsvCellStyle(props.document, props.view, Number(cell.getRow().getData()._row), cell.getField()) : {};
        if (props.view.styles?.length || element.dataset.csvStyled) {
          applyCsvCellStyle(element, { ...style, ...(style.size === undefined ? {} : { size: style.size * zoom.value }) });
        }
        const merge = csvMergeAt(props.view.merges, location(cell));
        const span = document.createElement("span"); span.textContent = merge ? "" : String(cell.getValue() ?? "");
        span.className = "csv-cell-text";
        if (merge) {
          element.setAttribute("aria-label", props.document.records[merge.rows[0]]?.fields[merge.columns[0]]?.value ?? "");
          element.setAttribute("aria-rowspan", String(merge.rows[1] - merge.rows[0] + 1));
          element.setAttribute("aria-colspan", String(merge.columns[1] - merge.columns[0] + 1));
        } else for (const attribute of ["aria-label", "aria-rowspan", "aria-colspan"]) element.removeAttribute(attribute);
        applyCsvExcelText(span, merge ? "" : String(cell.getValue() ?? ""), style.excel, props.view.wrapText);
        if (style.excel?.alignment?.shrinkToFit) onRendered(() => fitCsvExcelText(span, style.excel));
        return span;
      },
      variableHeight,
    };
  });
}
function sizeRow(row: RowComponent): void {
  const index = Number(row.getData()._row);
  const element = row.getElement();
  element.style.setProperty("--csv-content-height", `${rowHeight(index)}px`);
  if (props.view.rowDimensions?.[index]) element.dataset.csvFixedHeight = "true";
  else delete element.dataset.csvFixedHeight;
  row.normalizeHeight();
}
function rowHeight(index: number): number {
  const cached = rowHeights.get(index);
  if (cached !== undefined) return cached;
  const fields = props.document.records[index]?.fields;
  const dimension = props.view.rowDimensions?.[index];
  if (dimension) {
    const height = dimension.height * 96 / 72 * zoom.value;
    rowHeights.set(index, height);
    return height;
  }
  let height = props.view.rowHeight;
  if (fields && contentSizer) for (const column of projected) {
    if (column.hidden) continue;
    const merge = csvMergeAt(props.view.merges, { row: index, column: column.sourceIndex });
    if (merge && (index !== merge.rows[0] || column.sourceIndex !== visibleAnchor({ row: index, column: column.sourceIndex }).column)) continue;
    const value = fields[merge?.columns[0] ?? column.sourceIndex]?.value ?? "";
    if (!value || !props.view.wrapText && !/[\r\n]/.test(value) && !props.view.styles?.length) continue;
    // Even horizontally offscreen columns contribute to the row's height.
    const width = merge ? projected.filter((item) => !item.hidden && item.sourceIndex >= merge.columns[0] && item.sourceIndex <= merge.columns[1])
      .reduce((sum, item) => sum + item.width, 0) : column.width;
    const measured = contentSizer.measure(props.document, props.view, index, column.id, value, props.view.wrapText ? width : Infinity);
    height = Math.max(height, merge ? measured - (merge.rows[1] - merge.rows[0]) * props.view.rowHeight : measured);
  }
  const result = Math.ceil(height * zoom.value);
  rowHeights.set(index, result);
  return result;
}
function autoFitColumns(indices = selectedColumns()): void {
  if (!contentSizer || !indices.length) return;
  const targets = new Set(indices);
  const next = retainCsvProjectedColumns(props.view, projected);
  for (const column of projected) {
    if (column.hidden || !targets.has(column.sourceIndex)) continue;
    let width = 48;
    for (const row of rowIndices) {
      const value = props.document.records[row]?.fields[column.sourceIndex]?.value;
      if (!value) continue;
      width = Math.max(width, contentSizer.measure(props.document, props.view, row, column.id, value));
      if (width >= 2000) break;
    }
    next.columns[column.id] = { ...next.columns[column.id]!, width: Math.min(2000, width) };
  }
  emit("viewChange", next);
}
function autoFitRows(): void {
  // Existing wrap/minimum-height settings keep this behavior compatible with .view files and the SDK.
  emit("viewChange", { ...props.view, wrapText: true, rowHeight: 20 });
}
function setZoom(value: number): void {
  if (!Number.isFinite(value)) return;
  const next = Math.max(0.5, Math.min(2, Math.round(value * 100) / 100));
  requestedZoom = next;
  if (next === zoom.value) return;
  const snapshot = pendingSnapshot ?? getSnapshot();
  const ratio = next / zoom.value;
  pendingSnapshot = { ...snapshot, zoom: next, scrollTop: snapshot.scrollTop * ratio, scrollLeft: snapshot.scrollLeft * ratio };
  zoom.value = next;
  emit("zoomChange", next);
  void refresh();
}
function onWheel(event: WheelEvent): void {
  if (!event.ctrlKey && !event.metaKey) return;
  event.preventDefault(); event.stopImmediatePropagation();
  const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1);
  requestedZoom = Math.max(0.5, Math.min(2, requestedZoom - delta * 0.001));
  if (zoomFrame === null) zoomFrame = requestAnimationFrame(() => {
    zoomFrame = null; setZoom(requestedZoom);
  });
}
function onDoubleClick(event: MouseEvent): void {
  if (!isInput(event.target) && (event.target as HTMLElement).closest(".tabulator-cell:not(.tabulator-row-header)")) {
    event.preventDefault(); event.stopImmediatePropagation(); beginEdit(undefined, { x: event.clientX, y: event.clientY }); return;
  }
  const handle = (event.target as HTMLElement).closest(".tabulator-col-resize-handle");
  if (!handle) return;
  const id = handle.previousElementSibling?.getAttribute("tabulator-field");
  const column = projected.find((item) => item.id === id);
  if (!column) return;
  event.preventDefault(); event.stopImmediatePropagation();
  autoFitColumns([column.sourceIndex]);
}
function projectedLayoutKey(): string {
  return JSON.stringify([projected.map((column) => [column.id, column.sourceIndex, column.hidden]),
    props.view.frozenColumns, props.view.wrapText, zoom.value, props.view.merges]);
}
function rows(document = props.document) {
  return rowIndices.map((index) => {
    const data: Record<string, string | number> = { _row: index };
    for (const column of projected) data[column.id] = document.records[index]?.fields[column.sourceIndex]?.value ?? "";
    return data;
  });
}
function sameRows(left: readonly number[], right: readonly number[]): boolean {
  return left.length === right.length && left.every((row, index) => row === right[index]);
}
function documentRowPatches(previous: CsvDocument, current: CsvDocument): Record<string, string | number>[] | null {
  const patches: Record<string, string | number>[] = [];
  const trackedRows = csvDocumentChangedRows(previous, current);
  if (trackedRows) {
    for (const index of trackedRows) {
      if (!table?.getRow(index)) continue;
      const data: Record<string, string | number> = { _row: index };
      for (const column of projected) data[column.id] = current.records[index]?.fields[column.sourceIndex]?.value ?? "";
      patches.push(data);
    }
    return patches;
  }
  const maximumPatches = Math.max(32, Math.ceil(rowIndices.length / 4));
  for (const index of rowIndices) {
    const before = previous.records[index]?.fields, after = current.records[index]?.fields;
    // A disk/Worker parse creates new field arrays even for unchanged rows.
    // Compare displayed values (not quoting/line endings) before patching the grid.
    if (before === after || projected.every((column) =>
      (before?.[column.sourceIndex]?.value ?? "") === (after?.[column.sourceIndex]?.value ?? ""))) continue;
    if (patches.length >= maximumPatches) return null;
    const data: Record<string, string | number> = { _row: index };
    for (const column of projected) data[column.id] = current.records[index]?.fields[column.sourceIndex]?.value ?? "";
    patches.push(data);
  }
  return patches;
}
function getSnapshot(): CsvGridSnapshot {
  const scroller = host.value?.querySelector<HTMLElement>(".tabulator-tableholder");
  // Reading the endpoints must not materialize every cell in a large selection.
  const bounds = selectionBounds();
  const start = bounds?.start ?? selection;
  const end = bounds?.end ?? selection;
  return { ...start, endRow: end.row, endColumn: end.column, zoom: zoom.value,
    scrollTop: scroller?.scrollTop ?? 0, scrollLeft: scroller?.scrollLeft ?? 0 };
}
async function applySnapshot(snapshot: CsvGridSnapshot): Promise<void> {
  if (disposed) return;
  if (snapshot.zoom !== undefined) setZoom(snapshot.zoom);
  pendingSnapshot = snapshot;
  await refresh();
}
function restoreSnapshot(snapshot: CsvGridSnapshot): void {
  const visible = projected.filter((column) => !column.hidden);
  if (!rowIndices.length || !visible.length) return;
  const clamp = (row: number, column: number): Position => ({
    row: rowIndices.includes(row) ? row : rowIndices[Math.max(0, Math.min(rowIndices.length - 1, Math.trunc(row) || 0))]!,
    column: (visible.find((item) => item.sourceIndex === column)
      ?? visible.find((item) => item.sourceIndex >= column) ?? visible[visible.length - 1]!).sourceIndex,
  });
  announce(clamp(snapshot.row, snapshot.column));
  setRange(selection, clamp(snapshot.endRow ?? snapshot.row, snapshot.endColumn ?? snapshot.column));
  const scroller = host.value?.querySelector<HTMLElement>(".tabulator-tableholder");
  if (scroller) { scroller.scrollTop = snapshot.scrollTop; scroller.scrollLeft = snapshot.scrollLeft; }
}
async function refresh(): Promise<void> {
  if (!table || !built || disposed) return;
  if (!isActive() || isEditing()) { refreshRequested = true; return; }
  if (updating) { refreshRequested = true; return; }
  refreshRequested = false;
  updating = true;
  try {
    const requestedSnapshot = pendingSnapshot;
    const snapshot = requestedSnapshot ?? getSnapshot(); pendingSnapshot = null;
    await nextTick();
    if (!table || disposed) return;
    if (isEditing()) { refreshRequested = true; return; }
    const currentTable = table;
    const currentDocument = props.document;
    const previousRows = rowIndices;
    projected = projectCsvColumns(props.document, props.view);
    rowHeights.clear();
    const nextLayout = projectedLayoutKey();
    const nextData = JSON.stringify([props.view.sort, props.view.filters, props.view.headerRows, props.view.rowHeight, props.view.styles, props.view.merges, props.view.rowDimensions]);
    const canReuseRows = !!lastDocument && lastDocument.records.length === props.document.records.length
      && !props.view.sort?.length && !props.view.filters?.length && nextData === dataKey;
    const nextRows = canReuseRows ? previousRows
      : projectCsvRowIndices(props.document, props.view, projected.length);
    const changedLayout = nextLayout !== layoutKey;
    rowIndices = nextRows;
    displayedMerges = projectCsvMerges(props.view.merges ?? [], rowIndices, projected.filter((column) => !column.hidden));
    const canPatchRows = !changedLayout && !!lastDocument && nextData === dataKey && sameRows(previousRows, nextRows);
    const patches = canPatchRows && lastDocument !== currentDocument
      ? documentRowPatches(lastDocument!, currentDocument) : null;
    const replaceRows = changedLayout || lastDocument !== currentDocument || nextData !== dataKey;
    const data = replaceRows && !patches ? rows(currentDocument) : null;
    const definitions = changedLayout ? columns() : null;
    if (changedLayout) {
      // setColumns resets scroll synchronously. With horizontal virtualization,
      // that scroll event otherwise pairs new frozen columns with old row cells.
      // Clear the old rows (and the renderer's visible-row cache) first.
      await currentTable.setData([]);
      if (disposed || table !== currentTable) return;
      currentTable.setColumns(definitions!);
      const rowHeader = currentTable.getColumns()[0];
      if (rowHeader?.getElement().classList.contains("tabulator-row-header")) rowHeader.setWidth(40 * zoom.value);
      layoutKey = nextLayout;
    }
    let widthsChanged = false;
    for (const item of projected) {
      const column = table.getColumn(item.id);
      if (column && Math.abs(column.getWidth() - item.width * zoom.value) > 0.5) {
        column.setWidth(item.width * zoom.value); widthsChanged = true;
      }
    }
    if (canPatchRows && patches) {
      if (patches.length) await currentTable.updateData(patches);
      if (props.view.styles?.some((rule) => rule.when)) {
        for (const patch of patches) { const row = currentTable.getRow(Number(patch._row)); if (row) row.reformat(); }
      }
      lastDocument = currentDocument;
    } else if (data) {
      await currentTable.replaceData(data); lastDocument = currentDocument;
    }
    if (disposed || table !== currentTable) return;
    if (widthsChanged && props.view.wrapText && !data) for (const row of currentTable.getRows()) row.reformat();
    const restoreSelection = !!requestedSnapshot || !!data || changedLayout;
    if (redrawRequested) { redrawRequested = false; currentTable.redraw(); }
    dataKey = nextData;
    // Incremental edits and width changes keep their existing range. Reapplying
    // bounds makes Tabulator revisit every row, even when nothing moved.
    if (restoreSelection) restoreSnapshot(snapshot);
    mergeLayer?.invalidate();
  } catch (error) { reportError(error); }
  finally {
    updating = false;
    normalizeRange();
    viewportResize?.resume();
    if (disposed) { refreshRequested = false; pendingSelection = null; }
    else if (refreshRequested && isActive() && !isEditing()) { await refresh(); }
    else if (pendingSelection) { const target = pendingSelection; pendingSelection = null; await focusCell(target, keyboardAnchor ?? undefined).catch(reportError); }
  }
}
async function navigate(from: Position, x: number, y: number, extend = false): Promise<void> {
  const visible = projected.filter((column) => !column.hidden);
  if (!visible.length || !rowIndices.length) return;
  const merge = csvMergeAt(props.view.merges, from);
  const displayed = merge && displayedMerges.find((item) => item.source === merge);
  const columnIndex = Math.max(0, Math.min(visible.length - 1, (displayed ? displayed.columns[x > 0 ? 1 : 0] : visible.findIndex((column) => column.sourceIndex === from.column)) + x));
  const rowIndex = Math.max(0, Math.min(rowIndices.length - 1, (displayed ? displayed.rows[y > 0 ? 1 : 0] : rowIndices.indexOf(from.row)) + y));
  const target = { row: rowIndices[rowIndex]!, column: visible[columnIndex]!.sourceIndex };
  keyboardAnchor = extend ? keyboardAnchor ?? from : null;
  pendingSelection = target;
  await nextTick();
  if (!updating && lastDocument === props.document && cellAt(target)) {
    pendingSelection = null;
    await focusCell(target, keyboardAnchor ?? undefined);
  } else await refresh();
}
function clearSelection(): void {
  if (props.readOnly) return;
  const columns = selectedColumns();
  emit("edit", csvMergeEdits(props.view.merges ?? [], selectedRows().flatMap((row) => columns.map((column) => ({ row, column, value: "" }))), true));
}
function clipboardText(): string {
  const columns = selectedColumns();
  return serializeCsvClipboard(selectedRows().map((row) => columns.map((column) => {
    const merge = csvMergeAt(props.view.merges, { row, column });
    if (!merge) return props.document.records[row]?.fields[column]?.value ?? "";
    const anchor = visibleAnchor({ row, column });
    return row === anchor.row && column === anchor.column ? props.document.records[merge.rows[0]]?.fields[merge.columns[0]]?.value ?? "" : "";
  })));
}
function pasteText(text: string): void {
  if (props.readOnly || !table) return;
  try {
    const incoming = parseCsvClipboard(text), first = selectionBounds()?.start;
    if (!first) return;
    const activeColumns = projected.filter((column) => !column.hidden);
    const rowOffset = rowIndices.indexOf(first.row), columnOffset = activeColumns.findIndex((column) => column.sourceIndex === first.column);
    const lastRow = rowIndices.reduce((maximum, row) => Math.max(maximum, row + 1), props.document.records.length);
    const lastColumn = Math.max(props.document.columnCount, ...projected.map((column) => column.sourceIndex + 1));
    const edits: CsvCellEdit[] = [];
    const fillSelection = incoming.length === 1 && incoming[0]?.length === 1;
    if (fillSelection) {
      const columns = selectedColumns();
      for (const row of selectedRows()) for (const column of columns) edits.push({ row, column, value: incoming[0]![0]! });
    }
    else for (const [y, row] of incoming.entries()) for (const [x, value] of row.entries()) edits.push({
      row: rowIndices[rowOffset + y] ?? lastRow + rowOffset + y - rowIndices.length,
      column: activeColumns[columnOffset + x]?.sourceIndex ?? lastColumn + columnOffset + x - activeColumns.length, value });
    if (!fillSelection && edits.some((edit) => {
      const merge = csvMergeAt(props.view.merges, edit);
      return merge && (merge.rows[0] !== edit.row || merge.columns[0] !== edit.column) && edit.value !== "";
    })) throw new Error("csv.pasteMergedRange");
    emit("edit", csvMergeEdits(props.view.merges ?? [], edits, fillSelection));
  } catch (error) { emit("error", error); }
}
async function clipboardAction(action: "copy" | "cut" | "paste"): Promise<void> {
  try {
    if (action === "paste") pasteText(await navigator.clipboard.readText());
    else { await navigator.clipboard.writeText(clipboardText()); if (action === "cut") clearSelection(); }
  } catch (error) { emit("error", error); }
}
function isInput(target: EventTarget | null): boolean { return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLElement && (target.isContentEditable || !!target.closest(".csv-cell-input")); }
function focus(): void { host.value?.querySelector<HTMLElement>(".tabulator-tableholder")?.focus({ preventScroll: true }); }
function onClipboard(event: ClipboardEvent): void {
  if (!table || isInput(event.target) || !selectionBounds()) return;
  event.preventDefault(); event.stopImmediatePropagation();
  if (event.type === "paste") pasteText(event.clipboardData?.getData("text/plain") ?? "");
  else { event.clipboardData?.setData("text/plain", clipboardText()); if (event.type === "cut") clearSelection(); }
}
function beginEdit(seed?: string, point?: CsvCellEditPoint): void {
  if (props.readOnly) return;
  const merge = csvMergeAt(props.view.merges, selection);
  if (merge && mergeLayer) {
    const position = visibleAnchor(selection);
    const finish = () => { mergeLayer?.finishEditing(); resumeAfterEditing(); };
    const text = mergeLayer.edit(merge, point);
    if (!text) return;
    const input = editCsvCellText(text, props.document.records[merge.rows[0]]?.fields[merge.columns[0]]?.value ?? "", props.document.newline,
      (value) => { finish(); emit("edit", [{ row: merge.rows[0], column: merge.columns[0], value }]); }, finish,
      (x, y) => void navigate(position, x, y).catch(reportError), focus);
    input.setAttribute("aria-label", `${csvColumnLabel(merge.columns[0])}${merge.rows[0] + 1}`);
    if (seed !== undefined) seedCsvCellInput(input, seed);
    else focusCsvCellInput(input, point);
    return;
  }
  const cell = cellAt(selection) ?? cellAt(selectionBounds()?.start ?? selection);
  if (!cell) return;
  editPoint = point;
  try { cell.edit(); } finally { editPoint = undefined; }
  const input = cell.getElement().querySelector<HTMLElement>(".csv-cell-input");
  if (input && seed !== undefined) seedCsvCellInput(input, seed);
}
function onKeydown(event: KeyboardEvent): void {
  const modifier = event.ctrlKey || event.metaKey;
  if (modifier && event.key.toLowerCase() === "s") { event.preventDefault(); event.stopPropagation(); emit("save"); return; }
  if (isInput(event.target)) return;
  const stop = () => { event.preventDefault(); event.stopImmediatePropagation(); };
  if (modifier && ["z", "y"].includes(event.key.toLowerCase())) { stop(); event.shiftKey || event.key.toLowerCase() === "y" ? emit("redo") : emit("undo"); }
  else if (modifier && event.key.toLowerCase() === "f") { stop(); emit("find"); }
  else if (modifier && event.key.toLowerCase() === "a") {
    stop(); const visible = projected.filter((column) => !column.hidden);
    const last = [...visible].reverse().find((column) => column.sourceIndex < props.document.columnCount) ?? visible[0];
    if (visible[0] && last) setRange({ row: rowIndices[0] ?? 0, column: visible[0].sourceIndex },
      { row: [...rowIndices].reverse().find((row) => row < props.document.records.length) ?? 0, column: last.sourceIndex });
  }
  else if (event.key === "Delete" || event.key === "Backspace") { stop(); clearSelection(); }
  else if (event.key === "F2") { stop(); beginEdit(); }
  else if (!modifier && (event.key === "Enter" || event.key === "Tab")) { stop(); void navigate(selection, event.key === "Tab" ? (event.shiftKey ? -1 : 1) : 0, event.key === "Enter" ? (event.shiftKey ? -1 : 1) : 0).catch(reportError); }
  else if (!modifier && event.key.startsWith("Arrow")) {
    stop(); void navigate(selection, event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0,
      event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0, event.shiftKey).catch(reportError);
  } else if (!modifier && !event.altKey && (event.keyCode === 229 || event.key === "Process")) beginEdit("");
  else if (!modifier && !event.altKey && !event.isComposing && event.key.length === 1) { stop(); beginEdit(event.key); }
  else if (event.key === "ContextMenu" || event.key === "F10" && event.shiftKey) {
    stop(); const rect = cellAt(selection)?.getElement().getBoundingClientRect();
    emit("contextMenu", new MouseEvent("contextmenu", { clientX: rect?.left ?? 0, clientY: rect?.bottom ?? 0 }));
  }
}
function onContextMenu(event: MouseEvent): void {
  event.preventDefault(); event.stopPropagation();
  const element = (event.target as HTMLElement).closest<HTMLElement>(".tabulator-cell");
  const field = element?.getAttribute("tabulator-field");
  if (element && field) {
    const row = table?.getRows("visible").find((entry) => entry.getElement() === element.closest(".tabulator-row"));
    const cell = row?.getCell(field);
    if (cell) {
      const position = location(cell);
      if (!selectedRows().includes(position.row) || !selectedColumns().includes(position.column)) setRange(position);
      announce(position);
    }
  } else if (element?.classList.contains("tabulator-row-header")) {
    const row = table?.getRows("visible").find((entry) => entry.getElement() === element.closest(".tabulator-row"));
    const visible = projected.filter((column) => !column.hidden);
    if (row && visible.length) {
      const index = Number(row.getData()._row);
      if (!selectedRows().includes(index)) setRange({ row: index, column: visible[0]!.sourceIndex }, { row: index, column: visible[visible.length - 1]!.sourceIndex });
      announce({ row: index, column: visible[0]!.sourceIndex });
    }
  } else {
    const id = (event.target as HTMLElement).closest(".tabulator-col")?.getAttribute("tabulator-field");
    const column = projected.find((column) => column.id === id);
    if (column && rowIndices.length) {
      if (!selectedColumns().includes(column.sourceIndex)) setRange({ row: rowIndices[0]!, column: column.sourceIndex },
        { row: rowIndices[rowIndices.length - 1]!, column: column.sourceIndex });
      announce({ row: rowIndices[0]!, column: column.sourceIndex });
    }
  }
  emit("contextMenu", event);
}
onMounted(() => {
  if (!host.value) return;
  projected = projectCsvColumns(props.document, props.view);
  layoutKey = projectedLayoutKey();
  const instance = table = new Tabulator(host.value, {
    height: "100%", layout: "fitData", index: "_row", data: [], columns: columns(),
    selectableRange: 1, selectableRangeRows: true, selectableRangeColumns: true,
    editTriggerEvent: "dblclick", movableColumns: true, clipboard: false,
    rowHeader: { width: 40, frozen: true, resizable: false, headerSort: false, hozAlign: "center",
      // Tabulator supports a formatter function here; its RowHeader type only declares strings.
      formatter: ((cell: CellComponent) => String(Number(cell.getRow().getData()._row) + 1)) as unknown as string },
    columnDefaults: { vertAlign: "middle" }, rowFormatter: sizeRow, renderHorizontal: CsvHorizontalRenderer,
    renderVerticalBuffer: 160, placeholder: "", autoResize: false,
  });
  installCsvRangeSelection(instance);
  table.on("tableBuilt", () => {
    // Tabulator starts building in an uncancellable constructor timer. If Vue
    // unmounts first, destroy after that build so it cannot recreate listeners.
    if (disposed) { instance.destroy(); return; }
    contentSizer = createCsvContentSizer(host.value!);
    mergeLayer = createCsvMergeLayer(host.value!, () => ({ table: instance, document: props.document, view: props.view, zoom: zoom.value,
      rows: rowIndices, columns: projected.filter((column) => !column.hidden), merges: displayedMerges, rowHeight,
      selected: (merge) => { const bounds = displayedBounds(); return !!bounds && bounds.top <= merge.rows[1] && bounds.bottom >= merge.rows[0]
        && bounds.left <= merge.columns[1] && bounds.right >= merge.columns[0]; } }));
    built = true;
    viewportResize = observeCsvGridResize(host.value!, () => { redrawRequested = true; void refresh(); },
      () => !disposed && built && isActive() && !updating && !isEditing());
    void refresh();
  });
  table.on("cellEdited", (cell) => {
    if (!updating && !props.readOnly) emit("edit", [{ ...location(cell), value: String(cell.getValue() ?? "") }]);
    resumeAfterEditing();
  });
  table.on("cellEditCancelled", resumeAfterEditing);
  table.on("cellClick", (_, cell) => { columnAnchor = null; keyboardAnchor = null; announce(location(cell)); normalizeRange(); });
  table.on("rangeChanged", () => { if (!updating && !keyboardAnchor) { const bounds = selectionBounds(); if (bounds) announce(bounds.start); } mergeLayer?.schedule(); });
  table.on("renderComplete", () => mergeLayer?.schedule());
  table.on("scrollVertical", () => mergeLayer?.schedule());
  table.on("scrollHorizontal", () => mergeLayer?.schedule());
  table.on("columnResized", (column) => {
    if (updating) return;
    const id = column.getField(), next = retainCsvProjectedColumns(props.view, projected);
    if (next.columns[id]) emit("viewChange", { ...next, columns: { ...next.columns,
      [id]: { ...next.columns[id]!, width: Math.max(48, Math.min(2000, Math.round(column.getWidth() / zoom.value))) } } });
  });
  table.on("columnMoved", (_, moved) => {
    if (updating) return;
    const next = retainCsvProjectedColumns(props.view, projected);
    const order = moved.map((column) => column.getField()).filter((id) => id in next.columns);
    const grouped = orderCsvMergeColumns([...order, ...next.columnOrder.filter((id) => !order.includes(id))].map((id) => ({ id, ...next.columns[id]! })), next.merges);
    emit("viewChange", { ...next, columnOrder: grouped.map((column) => column.id) });
    if (grouped.some((column, index) => column.id !== order[index])) { layoutKey = ""; void refresh(); }
  });
  for (const name of ["copy", "cut", "paste"] as const) host.value.addEventListener(name, onClipboard, true);
  host.value.addEventListener("keydown", onKeydown, true);
  host.value.addEventListener("contextmenu", onContextMenu);
  host.value.addEventListener("wheel", onWheel, { passive: false, capture: true });
  host.value.addEventListener("dblclick", onDoubleClick, true);
  host.value.addEventListener("mouseup", normalizeRange);
});
watch(() => [props.document, props.view, props.readOnly], () => void refresh());
watch(() => props.active, (active) => {
  if (active) { void refresh(); viewportResize?.request(); }
  else viewportResize?.pause();
});
onBeforeUnmount(() => {
  disposed = true;
  viewportResize?.disconnect(); viewportResize = null;
  for (const name of ["copy", "cut", "paste"] as const) host.value?.removeEventListener(name, onClipboard, true);
  host.value?.removeEventListener("keydown", onKeydown, true);
  host.value?.removeEventListener("contextmenu", onContextMenu);
  host.value?.removeEventListener("wheel", onWheel, true);
  host.value?.removeEventListener("dblclick", onDoubleClick, true);
  host.value?.removeEventListener("mouseup", normalizeRange);
  if (zoomFrame !== null) cancelAnimationFrame(zoomFrame);
  contentSizer?.destroy(); contentSizer = null;
  mergeLayer?.destroy(); mergeLayer = null;
  if (built) table?.destroy();
  table = null;
});
defineExpose({ getSnapshot, applySnapshot, selectedRows, selectedColumns, selectedCells, mergeSelection, refresh, clipboardAction, clearSelection, focus, autoFitColumns, autoFitRows, setZoom });
</script>

<template><div ref="host" class="csv-grid" :style="{ '--csv-row-height': `${view.rowHeight * zoom}px`, '--csv-zoom': zoom }" :aria-label="t('csv.table')" /></template>

<style>
.csv-grid.tabulator { flex: 1; min-width: 0; min-height: 0; border: 0; background: var(--panel-bg); color: var(--text-color); font: calc(13px * var(--csv-zoom, 1))/1.45 var(--font-ui); user-select: none; }
.csv-grid.tabulator .tabulator-header, .csv-grid.tabulator .tabulator-header .tabulator-col { background: var(--sidebar-bg); color: var(--text-secondary); border-color: var(--border-color); font-weight: normal; }
.csv-grid.tabulator .tabulator-header .tabulator-col .tabulator-col-content { padding: calc(4px * var(--csv-zoom, 1)) calc(7px * var(--csv-zoom, 1)); }
.csv-grid.tabulator .tabulator-tableholder .tabulator-table { background: var(--panel-bg); color: var(--text-color); }
.csv-grid .tabulator-row, .csv-grid .tabulator-row.tabulator-row-even { background: var(--panel-bg); color: var(--text-color); min-height: var(--csv-content-height, var(--csv-row-height, 28px)); }
.csv-grid .tabulator-row .tabulator-cell, .csv-grid .tabulator-row .tabulator-cell.tabulator-editing { border: 0; border-right: 1px solid var(--border-color); border-bottom: 1px solid var(--border-color); padding: calc(4px * var(--csv-zoom, 1)) calc(7px * var(--csv-zoom, 1)); min-height: var(--csv-content-height, var(--csv-row-height, 28px)); color: var(--csv-cell-color, inherit); background: var(--csv-cell-bg, var(--panel-bg)); overflow-wrap: anywhere; }
.csv-grid .tabulator-row .tabulator-cell.tabulator-row-header { background: var(--sidebar-bg); color: var(--text-secondary); font-size: calc(12px * var(--csv-zoom, 1)); }
.csv-grid .tabulator-row[data-csv-fixed-height] .tabulator-cell { max-height: var(--csv-content-height); overflow: hidden; }
.csv-grid .csv-cell-measure { position: fixed; left: -100000px; top: 0; visibility: hidden; pointer-events: none; display: inline-block; box-sizing: border-box; height: auto; padding: 4px 7px; border-right: 1px solid transparent; border-bottom: 1px solid transparent; font: 13px/1.45 var(--font-ui); overflow-wrap: anywhere; }
/* Above frozen cells (11) and the merge layer (12); pointer events stay disabled. */
.csv-grid .tabulator-tableholder .tabulator-range-overlay { z-index: 13; }
.csv-grid .tabulator-range-overlay .tabulator-range { border-color: var(--accent-color); }
.csv-grid .tabulator-range-overlay .tabulator-range-cell-active { border-color: var(--accent-color); }
.csv-grid .tabulator-row .tabulator-cell.tabulator-range-selected:not(.tabulator-range-only-cell-selected):not(.tabulator-row-header) { background: color-mix(in srgb, var(--accent-soft) 55%, var(--csv-cell-bg, var(--panel-bg))); color: var(--csv-cell-color, var(--text-color)); }
.csv-grid .tabulator-row .tabulator-cell.tabulator-range-selected { background: color-mix(in srgb, var(--accent-soft) 55%, var(--csv-cell-bg, var(--panel-bg))); }
.csv-grid .tabulator-row .tabulator-cell.tabulator-range-selected:not(.tabulator-row-header) { border-right-color: color-mix(in srgb, var(--accent-color) 38%, var(--border-strong) 62%); border-bottom-color: color-mix(in srgb, var(--accent-color) 38%, var(--border-strong) 62%); }
.csv-grid .csv-cell-input { min-width: 1px; min-height: 1.45em; max-height: 100%; overflow: auto; outline: none; user-select: text; cursor: text; }
.csv-grid .tabulator-frozen.tabulator-frozen-left { border-right-color: var(--border-strong); }
.csv-grid .tabulator-frozen.tabulator-frozen-right { border-left-color: var(--border-strong); }
.csv-grid .csv-merge-layer { position: absolute; overflow: hidden; pointer-events: none; z-index: 12; }
.csv-grid .csv-merged-cell { position: absolute; box-sizing: border-box; display: flex; align-items: center; overflow: hidden; padding: calc(4px * var(--csv-zoom, 1)) calc(7px * var(--csv-zoom, 1)); border-right: 1px solid var(--border-color); border-bottom: 1px solid var(--border-color); color: var(--csv-cell-color, var(--text-color)); background: var(--csv-cell-bg, var(--panel-bg)); overflow-wrap: anywhere; }
.csv-grid .csv-merged-cell > span { min-width: 0; width: 100%; }
.csv-grid .csv-merged-selected { background: color-mix(in srgb, var(--accent-soft) 55%, var(--csv-cell-bg, var(--panel-bg))); outline: 1px solid var(--accent-color); outline-offset: -1px; }
.csv-grid .csv-merged-editing { pointer-events: auto; }
.csv-grid .tabulator-range-highlight, .csv-grid .tabulator-range-selected.tabulator-row-header { background: var(--accent-soft) !important; color: var(--text-color) !important; }
</style>
