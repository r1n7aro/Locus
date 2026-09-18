import { parseDocument, stringify } from "yaml";
import type { CsvDocument } from "./csvDocument";
import { reconcileCsvStyles, serializeCsvStyleRules, validateCsvStyleRules, type CsvStyleRule } from "./csvStyles";
import { serializeCsvMerges, validateCsvMerges, type CsvMerge } from "./csvMerges";

export interface CsvColumnView { sourceIndex: number; header: string; width: number; hidden?: boolean }
export interface CsvView {
  schema: "locus.csv-view.v1" | "locus.csv-view.v2" | "locus.csv-view.v3" | "locus.csv-view.v4";
  headerRows: number;
  rowHeight: number;
  wrapText: boolean;
  frozenColumns: number;
  columns: Record<string, CsvColumnView>;
  columnOrder: string[];
  sort?: { columnId: string; direction: "asc" | "desc" }[];
  filters?: { columnId: string; value: string }[];
  styles?: CsvStyleRule[];
  merges?: CsvMerge[];
  rowDimensions?: Record<string, { height: number; hidden: boolean }>;
}

export function defaultCsvView(): CsvView {
  return { schema: "locus.csv-view.v1", headerRows: 1, rowHeight: 28, wrapText: false,
    frozenColumns: 0, columns: {}, columnOrder: [] };
}

function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
function integer(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum && value <= maximum;
}
function fields(value: Record<string, unknown>, allowed: string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}

export function parseCsvView(source: string): CsvView {
  if (source.length > 1024 * 1024) throw new Error("csv.invalidView");
  const parsed = parseDocument(source, { version: "1.2", uniqueKeys: true, stringKeys: true,
    resolveKnownTags: false, merge: false });
  if (parsed.errors.length || parsed.warnings.length) throw new Error("csv.invalidView");
  const value: unknown = parsed.toJS({ maxAliasCount: 0 });
  if (!object(value) || !["locus.csv-view.v1", "locus.csv-view.v2", "locus.csv-view.v3", "locus.csv-view.v4"].includes(String(value.schema))) throw new Error("csv.unsupportedView");
  if (!fields(value, ["schema", "headerRows", "rowHeight", "wrapText", "frozenColumns", "columns", "columnOrder", "sort", "filters", "styles", "merges", "rowDimensions"])
    || !integer(value.headerRows, 0, 1) || !integer(value.rowHeight, 20, 120)
    || typeof value.wrapText !== "boolean" || !integer(value.frozenColumns, 0, 10000)
    || !object(value.columns) || !Array.isArray(value.columnOrder)) throw new Error("csv.invalidView");
  const keys = Object.keys(value.columns);
  const indices = new Set<number>();
  for (const [id, column] of Object.entries(value.columns)) {
    if (!/^[a-zA-Z][a-zA-Z0-9_-]{0,100}$/.test(id) || ["constructor", "prototype", "__proto__"].includes(id)
      || !object(column) || !fields(column, ["sourceIndex", "header", "width", "hidden"])
      || !integer(column.sourceIndex, 0, 9999) || indices.has(column.sourceIndex)
      || typeof column.header !== "string" || !integer(column.width, 48, 2000)
      || (column.hidden !== undefined && typeof column.hidden !== "boolean")) throw new Error("csv.invalidView");
    indices.add(column.sourceIndex);
  }
  if (keys.length > 10000 || value.columnOrder.length !== keys.length
    || new Set(value.columnOrder).size !== keys.length
    || !value.columnOrder.every((id) => typeof id === "string" && keys.includes(id))) throw new Error("csv.invalidView");
  if (value.sort !== undefined && (!Array.isArray(value.sort) || !value.sort.every((entry) =>
    object(entry) && fields(entry, ["columnId", "direction"]) && keys.includes(String(entry.columnId))
      && ["asc", "desc"].includes(String(entry.direction))))) throw new Error("csv.invalidView");
  if (value.filters !== undefined && (!Array.isArray(value.filters) || !value.filters.every((entry) =>
    object(entry) && fields(entry, ["columnId", "value"]) && keys.includes(String(entry.columnId))
      && typeof entry.value === "string"))) throw new Error("csv.invalidView");
  if (value.styles !== undefined && (!validateCsvStyleRules(value.styles, value.columns as unknown as Record<string, CsvColumnView>)
    || value.styles.length > 0 && value.schema === "locus.csv-view.v1")) throw new Error("csv.invalidView");
  if (value.merges !== undefined && (!validateCsvMerges(value.merges)
    || value.merges.length > 0 && !["locus.csv-view.v3", "locus.csv-view.v4"].includes(String(value.schema)))) throw new Error("csv.invalidView");
  if ((value.styles as CsvStyleRule[] | undefined)?.some((r) => r.style.excel || r.when?.op.startsWith("num_")) && value.schema !== "locus.csv-view.v4") throw new Error("csv.invalidView");
  if (value.rowDimensions !== undefined && (!object(value.rowDimensions) || Object.keys(value.rowDimensions).length > 10000
    || Object.keys(value.rowDimensions).length > 0 && value.schema !== "locus.csv-view.v4"
    || !Object.entries(value.rowDimensions).every(([row, dim]) => /^(0|[1-9]\d*)$/.test(row) && integer(Number(row), 0, 499999)
      && object(dim) && fields(dim, ["height", "hidden"]) && typeof dim.height === "number" && Number.isFinite(dim.height)
      && dim.height >= 1 && dim.height <= 409 && typeof dim.hidden === "boolean"))) throw new Error("csv.invalidView");
  return value as unknown as CsvView;
}

export function serializeCsvView(view: CsvView): string {
  const columns = Object.fromEntries(Object.keys(view.columns).sort().map((id) => {
    const column = view.columns[id]!;
    return [id, { sourceIndex: column.sourceIndex, header: column.header, width: column.width,
      ...(column.hidden ? { hidden: true } : {}) }];
  }));
  return stringify({ schema: view.schema, headerRows: view.headerRows, rowHeight: view.rowHeight,
    wrapText: view.wrapText, frozenColumns: view.frozenColumns, columns, columnOrder: view.columnOrder,
    ...(view.sort?.length ? { sort: view.sort } : {}), ...(view.filters?.length ? { filters: view.filters } : {}),
  }, { indent: 2, lineWidth: 0, defaultStringType: "QUOTE_DOUBLE", defaultKeyType: "PLAIN" }) + serializeCsvStyleRules(view.styles ?? []) + serializeCsvMerges(view.merges)
    + (Object.keys(view.rowDimensions ?? {}).length ? `rowDimensions:\n${Object.entries(view.rowDimensions!).sort((a, b) => Number(a[0]) - Number(b[0])).map(([row, dim]) => `  "${row}": ${JSON.stringify({ height: dim.height, hidden: dim.hidden })}\n`).join("")}` : "");
}

export function reconcileCsvView(previous: CsvView, document: CsvDocument): CsvView {
  const count = Math.max(1, document.columnCount, ...Object.values(previous.columns).map((column) => column.sourceIndex + 1));
  const headers = Array.from({ length: count }, (_, index) => previous.headerRows
    ? document.records[0]?.fields[index]?.value ?? "" : "");
  const available = new Set(Object.keys(previous.columns));
  const columns: Record<string, CsvColumnView> = {};
  const keys: string[] = [];
  for (const [index, header] of headers.entries()) {
    const matching = [...available].filter((id) => previous.columns[id]!.header === header);
    const id = matching.find((id) => previous.columns[id]!.sourceIndex === index)
      ?? (header && headers.filter((value) => value === header).length === 1 && matching.length === 1 ? matching[0] : undefined)
      ?? [...available].find((id) => previous.columns[id]!.sourceIndex === index &&
        (!previous.columns[id]!.header || !headers.includes(previous.columns[id]!.header)));
    const key = id ?? `c_${crypto.randomUUID().replace(/-/g, "")}`;
    if (id) available.delete(id);
    columns[key] = { ...(id ? previous.columns[id]! : { width: 140 }), sourceIndex: index, header };
    keys.push(key);
  }
  const columnOrder = previous.columnOrder.filter((id) => id in columns);
  for (const id of keys) if (!columnOrder.includes(id)) columnOrder.push(id);
  if (previous.columnOrder.every((id, index) => !index || previous.columns[id]!.sourceIndex > previous.columns[previous.columnOrder[index - 1]!]!.sourceIndex)) {
    columnOrder.sort((left, right) => columns[left]!.sourceIndex - columns[right]!.sourceIndex);
  }
  return { ...previous, columns, columnOrder, frozenColumns: Math.min(previous.frozenColumns, count),
    sort: previous.sort?.filter((entry) => entry.columnId in columns),
    filters: previous.filters?.filter((entry) => entry.columnId in columns),
    ...(previous.styles ? { styles: reconcileCsvStyles(previous.styles, columns) } : {}) };
}
