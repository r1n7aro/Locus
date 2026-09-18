import type { CsvDocument } from "../../document/csv/csvDocument";
import { applyCsvCellStyle, resolveCsvCellStyle } from "../../document/csv/csvStyles";
import { applyCsvExcelText } from "../../document/csv/csvExcelStyles";
import type { CsvView } from "../../document/csv/csvView";

/** Measure source text, including cells outside Tabulator's virtual viewport. */
export function createCsvContentSizer(host: HTMLElement) {
  const probe = document.createElement("div");
  probe.className = "csv-cell-measure";
  probe.setAttribute("aria-hidden", "true");
  const text = document.createElement("span");
  probe.appendChild(text);
  host.appendChild(probe);
  const cache = new Map<string, number>();

  return {
    measure(document: CsvDocument, view: CsvView, row: number, columnId: string, value: string, width?: number): number {
      const style = resolveCsvCellStyle(document, view, row, columnId);
      if (width !== undefined && !Number.isFinite(width) && style.excel?.alignment?.wrapText) width = view.columns[columnId]?.width ?? width;
      if (width !== undefined && style.excel?.alignment && !style.excel.alignment.wrapText) width = Infinity;
      const key = JSON.stringify([value, style, width]);
      const cached = cache.get(key);
      if (cached !== undefined) return cached;
      applyCsvCellStyle(probe, style);
      // Measurements use unscaled CSS pixels; zoom never changes saved widths.
      probe.style.width = width === undefined || !Number.isFinite(width) ? "max-content" : `${width}px`;
      applyCsvExcelText(text, value || " ", style.excel, Number.isFinite(width));
      if (width === undefined) text.style.whiteSpace = "pre";
      const rotation = style.excel?.alignment?.textRotation;
      const result = Math.ceil(width === undefined ? probe.offsetWidth : rotation && rotation !== 255 ? Math.max(probe.offsetHeight, text.getBoundingClientRect().height + 10) : probe.offsetHeight);
      if (cache.size >= 5000) cache.clear();
      cache.set(key, result);
      return result;
    },
    clear() { cache.clear(); },
    destroy() { probe.remove(); cache.clear(); },
  };
}
