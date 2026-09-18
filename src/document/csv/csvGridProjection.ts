import { CSV_MAX_CELLS, type CsvDocument } from "./csvDocument";
import type { CsvColumnView, CsvView } from "./csvView";
import { csvMergeAxisGroups, orderCsvMergeColumns } from "./csvMerges";

export interface CsvProjectedColumn extends CsvColumnView { id: string; virtual: boolean }

export function csvColumnLabel(index: number): string {
  let label = "";
  for (let number = index + 1; number > 0; number = Math.floor((number - 1) / 26)) {
    label = String.fromCharCode(65 + (number - 1) % 26) + label;
  }
  return label;
}

/** Keep a finite editing margin. Scrolling and old view metadata never grow it. */
export function projectCsvColumns(document: CsvDocument, view: CsvView): CsvProjectedColumn[] {
  const mergeRows = Math.max(0, ...(view.merges ?? []).map((merge) => merge.rows[1] + 1));
  const mergeColumns = Math.max(0, ...(view.merges ?? []).map((merge) => merge.columns[1] + 1));
  const maximum = Math.max(document.columnCount, mergeColumns, Math.floor(CSV_MAX_CELLS / Math.max(document.records.length, mergeRows, 100)));
  const count = Math.min(10000, maximum, Math.max(26, document.columnCount + 8, mergeColumns));
  const columns = view.columnOrder.map((id) => ({ id, ...view.columns[id]!, virtual: false }))
    .filter((column) => column.sourceIndex < count);
  const indices = new Set(columns.map((column) => column.sourceIndex));
  const naturalOrder = columns.every((column, index) => !index || column.sourceIndex > columns[index - 1]!.sourceIndex);
  for (let index = 0; index < count; index++) if (!indices.has(index)) {
    columns.push({ id: `blank_${index}`, sourceIndex: index, header: "", width: 110, virtual: true });
  }
  if (naturalOrder) columns.sort((left, right) => left.sourceIndex - right.sourceIndex);
  return orderCsvMergeColumns(columns, view.merges);
}

export function retainCsvProjectedColumns(view: CsvView, projected: CsvProjectedColumn[]): CsvView {
  const ids = projected.map((column) => column.id);
  return { ...view, columns: { ...view.columns, ...Object.fromEntries(projected.map(({ id, virtual: _, ...column }) => [id, column])) },
    columnOrder: [...ids, ...view.columnOrder.filter((id) => !ids.includes(id))] };
}

export function projectCsvRowIndices(document: CsvDocument, view: CsvView, columnCount: number): number[] {
  if (view.merges?.length) return projectMergedRows(document, view, columnCount).filter((row) => !view.rowDimensions?.[row]?.hidden);
  const headers = Array.from({ length: Math.min(view.headerRows, document.records.length) }, (_, index) => index);
  const records = document.records.map((_, index) => index).slice(headers.length).filter((index) =>
    (view.filters ?? []).every((filter) => {
      const column = view.columns[filter.columnId]?.sourceIndex;
      return column == null || (document.records[index]!.fields[column]?.value ?? "").toLocaleLowerCase().includes(filter.value.toLocaleLowerCase());
    }));
  if (view.sort?.length) records.sort((left, right) => {
    for (const sort of view.sort ?? []) {
      const column = view.columns[sort.columnId]?.sourceIndex;
      if (column == null) continue;
      const comparison = (document.records[left]!.fields[column]?.value ?? "")
        .localeCompare(document.records[right]!.fields[column]?.value ?? "");
      if (comparison) return sort.direction === "desc" ? -comparison : comparison;
    }
    return left - right;
  });
  const maximum = Math.floor(CSV_MAX_CELLS / Math.max(columnCount, 1));
  const extent = Math.min(maximum, Math.max(document.records.length + 20, 100));
  return [...headers, ...records, ...Array.from({ length: Math.max(0, extent - document.records.length) }, (_, index) => document.records.length + index)].filter((row) => !view.rowDimensions?.[row]?.hidden);
}

function projectMergedRows(document: CsvDocument, view: CsvView, columnCount: number): number[] {
  const groups = csvMergeAxisGroups(view.merges!, "rows");
  const maximum = Math.floor(CSV_MAX_CELLS / Math.max(columnCount, 1));
  const mergeExtent = Math.max(0, ...view.merges!.map((merge) => merge.rows[1] + 1));
  const extent = Math.min(maximum, Math.max(document.records.length + 20, mergeExtent, 100));
  const blocks: number[][] = [], pinned: number[] = [], blanks: number[] = [];
  let groupIndex = 0;
  for (let index = 0; index < extent;) {
    while (groups[groupIndex] && groups[groupIndex]![1] < index) groupIndex++;
    const end = groups[groupIndex]?.[0] === index ? Math.min(extent - 1, groups[groupIndex]![1]) : index;
    const block = Array.from({ length: end - index + 1 }, (_, offset) => index + offset);
    if (index < view.headerRows) pinned.push(...block);
    else if (index >= document.records.length) blanks.push(...block);
    else if (block.some((row) => (view.filters ?? []).every((filter) => {
      const column = view.columns[filter.columnId]?.sourceIndex;
      return column == null || (document.records[row]?.fields[column]?.value ?? "").toLocaleLowerCase().includes(filter.value.toLocaleLowerCase());
    }))) blocks.push(block);
    index = end + 1;
  }
  if (view.sort?.length) blocks.sort((left, right) => {
    for (const sort of view.sort!) {
      const column = view.columns[sort.columnId]?.sourceIndex;
      if (column == null) continue;
      const comparison = (document.records[left[0]!] ?.fields[column]?.value ?? "")
        .localeCompare(document.records[right[0]!] ?.fields[column]?.value ?? "");
      if (comparison) return sort.direction === "desc" ? -comparison : comparison;
    }
    return left[0]! - right[0]!;
  });
  return [...pinned, ...blocks.flat(), ...blanks];
}
