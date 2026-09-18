import type { Tabulator } from "tabulator-tables";
import type { CsvDocument } from "../../document/csv/csvDocument";
import type { CsvView } from "../../document/csv/csvView";
import type { CsvMerge } from "../../document/csv/csvMerges";
import type { CsvProjectedColumn } from "../../document/csv/csvGridProjection";
import { applyCsvCellStyle, resolveCsvCellStyle } from "../../document/csv/csvStyles";
import { applyCsvExcelText, fitCsvExcelText } from "../../document/csv/csvExcelStyles";
import type { CsvCellEditPoint } from "./csvCellEditor";

export interface CsvDisplayedMerge {
  source: CsvMerge;
  rows: [number, number];
  columns: [number, number];
}
export function projectCsvMerges(merges: readonly CsvMerge[], rows: number[], columns: CsvProjectedColumn[]): CsvDisplayedMerge[] {
  const rowMap = new Map(rows.map((row, index) => [row, index]));
  return merges.flatMap((source) => {
    const visibleRows = rows.filter((row) => row >= source.rows[0] && row <= source.rows[1]);
    const first = visibleRows.length ? rowMap.get(visibleRows[0]!) : undefined;
    const last = visibleRows.length ? rowMap.get(visibleRows[visibleRows.length - 1]!) : undefined;
    const visible = columns.flatMap((column, index) => column.sourceIndex >= source.columns[0] && column.sourceIndex <= source.columns[1] ? [index] : []);
    if (first === undefined || last === undefined || !visible.length) return [];
    return [{ source, rows: [first, last], columns: [visible[0]!, visible[visible.length - 1]!] } as CsvDisplayedMerge];
  });
}

interface MergeLayerState {
  table: Tabulator; document: CsvDocument; view: CsvView; zoom: number;
  rows: number[]; columns: CsvProjectedColumn[]; merges: CsvDisplayedMerge[];
  rowHeight: (row: number) => number;
  selected: (merge: CsvDisplayedMerge) => boolean;
}

/** A clipped visual layer; underlying grid cells still handle pointer selection.
 * Neither scrolling nor offscreen anchors materialize additional Tabulator cells.
 */
export function createCsvMergeLayer(host: HTMLElement, state: () => MergeLayerState) {
  const layer = document.createElement("div");
  layer.className = "csv-merge-layer";
  layer.setAttribute("aria-hidden", "true");
  host.appendChild(layer);
  let frame: number | null = null;
  let prefix: number[] | null = null;
  let boxes: Array<{ merge: CsvMerge; element: HTMLElement; left: number; top: number; right: number; bottom: number }> = [];
  let input: HTMLElement | null = null;
  function render() {
    frame = null;
    if (input) return;
    const current = state();
    const scroller = host.querySelector<HTMLElement>(".tabulator-tableholder");
    layer.replaceChildren(); boxes = [];
    if (!scroller || !current.merges.length) return;
    const bounds = scroller.getBoundingClientRect();
    Object.assign(layer.style, { left: `${scroller.offsetLeft}px`, top: `${scroller.offsetTop}px`,
      width: `${scroller.clientWidth}px`, height: `${scroller.clientHeight}px` });
    const rendered = current.table.getRows("visible");
    const first = rendered.find((row) => row.getElement().isConnected);
    if (!first) return;
    const firstIndex = current.rows.indexOf(Number(first.getData()._row));
    if (firstIndex < 0) return;
    if (!prefix) {
      prefix = [0];
      for (const row of current.rows) prefix.push(prefix[prefix.length - 1]! + current.rowHeight(row));
    }
    const origin = first.getElement().getBoundingClientRect().top - bounds.top - prefix[firstIndex]!;
    const widths = [40 * current.zoom];
    for (const column of current.columns) widths.push(widths[widths.length - 1]! + column.width * current.zoom);
    const frozen = Math.min(current.view.frozenColumns, current.columns.length);
    const boundary = widths[frozen]!;
    for (const projected of current.merges) {
      const { source, rows, columns } = projected;
      const top = origin + prefix[rows[0]]!, height = prefix[rows[1] + 1]! - prefix[rows[0]]!;
      if (top >= scroller.clientHeight || top + height <= 0) continue;
      const width = widths[columns[1] + 1]! - widths[columns[0]]!;
      const columnId = current.view.columnOrder.find((id) => current.view.columns[id]!.sourceIndex === source.columns[0]);
      const style = columnId ? resolveCsvCellStyle(current.document, current.view, source.rows[0], columnId) : {};
      for (const fixed of [false, true]) {
        if (fixed ? columns[0] >= frozen : columns[1] < frozen) continue;
        const left = widths[columns[0]]! - (fixed ? 0 : scroller.scrollLeft);
        const clipLeft = Math.max(left, fixed ? widths[0]! : boundary);
        const clipRight = Math.min(left + width, fixed ? boundary : scroller.clientWidth);
        if (clipRight <= clipLeft) continue;
        const cell = document.createElement("div");
        cell.className = "csv-merged-cell";
        cell.dataset.csvMerge = `${source.rows[0]}:${source.columns[0]}`;
        cell.classList.toggle("csv-merged-selected", current.selected(projected));
        applyCsvCellStyle(cell, { ...style, ...(style.size === undefined ? {} : { size: style.size * current.zoom }) });
        Object.assign(cell.style, { left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px`,
          clipPath: `inset(0 ${left + width - clipRight}px 0 ${clipLeft - left}px)` });
        const text = document.createElement("span");
        text.className = "csv-cell-text";
        applyCsvExcelText(text, current.document.records[source.rows[0]]?.fields[source.columns[0]]?.value ?? "", style.excel, current.view.wrapText);
        cell.appendChild(text); layer.appendChild(cell);
        fitCsvExcelText(text, style.excel);
        boxes.push({ merge: source, element: cell, left: bounds.left + clipLeft, right: bounds.left + clipRight,
          top: bounds.top + Math.max(0, top), bottom: bounds.top + Math.min(scroller.clientHeight, top + height) });
      }
    }
  }
  function schedule() { if (frame === null) frame = requestAnimationFrame(render); }
  return {
    schedule,
    invalidate() { prefix = null; schedule(); },
    edit(merge: CsvMerge, point?: CsvCellEditPoint): HTMLElement | null {
      if (frame !== null) { cancelAnimationFrame(frame); frame = null; }
      if (!boxes.some((box) => box.merge === merge)) render();
      const parts = boxes.filter((box) => box.merge === merge);
      if (!parts.length) return null;
      // Keep the full text width and original cell surface even when clipped by
      // the viewport. Prefer the anchor's frozen portion when a merge spans it.
      const cell = parts.find((part) => point && point.x >= part.left && point.x <= part.right
        && point.y >= part.top && point.y <= part.bottom)?.element ?? parts[parts.length - 1]!.element;
      input = cell.querySelector<HTMLElement>(".csv-cell-text");
      if (!input) return null;
      cell.classList.add("csv-merged-editing");
      layer.removeAttribute("aria-hidden");
      return input;
    },
    finishEditing() {
      if (input) {
        input.classList.remove("csv-cell-input");
        for (const name of ["contenteditable", "tabindex", "role", "aria-multiline"]) input.removeAttribute(name);
        input.parentElement?.classList.remove("csv-merged-editing");
      }
      input = null; layer.setAttribute("aria-hidden", "true"); schedule();
    },
    destroy() { if (frame !== null) cancelAnimationFrame(frame); layer.remove(); },
  };
}
