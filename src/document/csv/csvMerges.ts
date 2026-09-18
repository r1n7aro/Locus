import { CSV_MAX_CELLS, type CsvCellEdit } from "./csvDocument";
import type { CsvView } from "./csvView";

/** Inclusive, zero-based source coordinates, including the header record. */
export interface CsvMerge { rows: [number, number]; columns: [number, number] }
export interface CsvPosition { row: number; column: number }

export function intersectsCsvMerge(a: CsvMerge, b: CsvMerge): boolean {
  return a.rows[0] <= b.rows[1] && b.rows[0] <= a.rows[1]
    && a.columns[0] <= b.columns[1] && b.columns[0] <= a.columns[1];
}
export function containsCsvPosition(merge: CsvMerge, position: CsvPosition): boolean {
  return position.row >= merge.rows[0] && position.row <= merge.rows[1]
    && position.column >= merge.columns[0] && position.column <= merge.columns[1];
}
export function validateCsvMerges(value: unknown): value is CsvMerge[] {
  if (!Array.isArray(value) || value.length > 10000) return false;
  let area = 0, rows = 0, columns = 0;
  for (const merge of value) {
    if (!merge || typeof merge !== "object" || Object.keys(merge).some((key) => !["rows", "columns"].includes(key))) return false;
    for (const [key, maximum] of [["rows", 499999], ["columns", 9999]] as const) {
      const range = merge[key];
      if (!Array.isArray(range) || range.length !== 2 || !range.every((v) => Number.isSafeInteger(v) && v >= 0 && v <= maximum)
        || range[0] > range[1]) return false;
    }
    const size = (merge.rows[1] - merge.rows[0] + 1) * (merge.columns[1] - merge.columns[0] + 1);
    if (size < 2 || (area += size) > CSV_MAX_CELLS) return false;
    rows = Math.max(rows, merge.rows[1] + 1); columns = Math.max(columns, merge.columns[1] + 1);
  }
  if (rows * columns > CSV_MAX_CELLS) return false;
  // Sweep source rows; no expansion into individual cells for large rectangles.
  const ordered = [...value].sort((a, b) => a.rows[0] - b.rows[0]);
  let active: CsvMerge[] = [];
  for (const merge of ordered) {
    active = active.filter((other) => other.rows[1] >= merge.rows[0]);
    if (active.some((other) => intersectsCsvMerge(other, merge))) return false;
    active.push(merge);
  }
  return true;
}
export function migrateCsvViewToV3(view: CsvView): CsvView { return view.schema === "locus.csv-view.v4" ? view : { ...view, schema: "locus.csv-view.v3" }; }
export function serializeCsvMerges(merges: CsvMerge[] = []): string {
  return merges.length ? `merges:\n${merges.map(({ rows, columns }) => `  - ${JSON.stringify({ rows, columns })}\n`).join("")}` : "";
}
export function csvMergeAt(merges: readonly CsvMerge[] | undefined, position: CsvPosition): CsvMerge | undefined {
  return merges?.find((merge) => containsCsvPosition(merge, position));
}
export function csvMergeAnchor(merges: readonly CsvMerge[] | undefined, position: CsvPosition): CsvPosition {
  const merge = csvMergeAt(merges, position);
  return merge ? { row: merge.rows[0], column: merge.columns[0] } : position;
}
export function expandCsvMergeSelection(merges: readonly CsvMerge[], range: CsvMerge): CsvMerge {
  let result = range, changed = true;
  while (changed) {
    changed = false;
    for (const merge of merges) if (intersectsCsvMerge(result, merge)) {
      const rows: [number, number] = [Math.min(result.rows[0], merge.rows[0]), Math.max(result.rows[1], merge.rows[1])];
      const columns: [number, number] = [Math.min(result.columns[0], merge.columns[0]), Math.max(result.columns[1], merge.columns[1])];
      if (rows[0] !== result.rows[0] || rows[1] !== result.rows[1] || columns[0] !== result.columns[0] || columns[1] !== result.columns[1]) {
        result = { rows, columns }; changed = true;
      }
    }
  }
  return result;
}
export function mergeCsvSelection(view: CsvView, range: CsvMerge): CsvView {
  const expanded = expandCsvMergeSelection(view.merges ?? [], range);
  const merges = [...(view.merges ?? []).filter((merge) => !intersectsCsvMerge(merge, expanded)), expanded];
  if (!validateCsvMerges(merges)) throw new Error("csv.invalidMerge");
  return { ...migrateCsvViewToV3(view), merges };
}
export function unmergeCsvSelection(view: CsvView, rows: number[], columns: number[]): CsvView {
  return { ...view, merges: (view.merges ?? []).filter((merge) => !rows.some((row) => row >= merge.rows[0] && row <= merge.rows[1])
    || !columns.some((column) => column >= merge.columns[0] && column <= merge.columns[1])) };
}

/** Inserts inside a merge expand it; deleting its anchor exposes the new top-left value. */
export function adjustCsvMerges(merges: CsvMerge[] | undefined, axis: "rows" | "columns", insertion: number | null, removed: number[] = []): CsvMerge[] | undefined {
  const deleted = [...new Set(removed)].sort((a, b) => a - b);
  return merges?.flatMap((merge) => {
    let [start, end] = merge[axis];
    if (insertion !== null) { if (insertion <= start) start++; if (insertion <= end) end++; }
    else {
      const before = deleted.filter((index) => index < start).length;
      const inside = deleted.filter((index) => index >= start && index <= end).length;
      if (inside === end - start + 1) return [];
      start -= before; end -= before + inside;
    }
    const next = { ...merge, [axis]: [start, end] } as CsvMerge;
    return next.rows[0] === next.rows[1] && next.columns[0] === next.columns[1] ? [] : [next];
  });
}

/** Visible edits never clear or overwrite values covered by a merge. */
export function csvMergeEdits(merges: readonly CsvMerge[], edits: CsvCellEdit[], fill = false): CsvCellEdit[] {
  const result = new Map<string, CsvCellEdit>();
  for (const edit of edits) {
    const anchor = csvMergeAnchor(merges, edit);
    if (!fill && (anchor.row !== edit.row || anchor.column !== edit.column)) continue;
    const key = `${anchor.row}:${anchor.column}`;
    if (!result.has(key)) result.set(key, { ...anchor, value: edit.value });
  }
  return [...result.values()];
}

/** Overlapping row/column spans form indivisible display groups. */
export function csvMergeAxisGroups(merges: readonly CsvMerge[], axis: "rows" | "columns"): [number, number][] {
  const spans = merges.map((merge) => merge[axis]).filter(([start, end]) => end > start).sort((a, b) => a[0] - b[0]);
  const groups: [number, number][] = [];
  for (const [start, end] of spans) {
    const last = groups[groups.length - 1];
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else groups.push([start, end]);
  }
  return groups;
}
export function orderCsvMergeColumns<T extends { sourceIndex: number }>(columns: T[], merges: readonly CsvMerge[] = []): T[] {
  const groups = csvMergeAxisGroups(merges, "columns");
  if (!groups.length) return columns;
  const emitted = new Set<number>();
  return columns.flatMap((column) => {
    const group = groups.findIndex(([start, end]) => column.sourceIndex >= start && column.sourceIndex <= end);
    if (group < 0) return [column];
    if (emitted.has(group)) return [];
    emitted.add(group);
    const [start, end] = groups[group]!;
    return columns.filter((item) => item.sourceIndex >= start && item.sourceIndex <= end).sort((a, b) => a.sourceIndex - b.sourceIndex);
  });
}
