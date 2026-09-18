import type { CsvDocument } from "./csvDocument";
import type { CsvColumnView, CsvView } from "./csvView";
import { csvExcelBorder, csvExcelFillImage, serializeCsvExcelStyle, validateCsvExcelStyle, type CsvExcelStyle } from "./csvExcelStyles";

export type CsvBorderEdge = "top" | "right" | "bottom" | "left";
export interface CsvBorderStyle { color?: string; width?: number; style?: "solid" | "dashed" | "dotted" | "double" | "none"; edges?: CsvBorderEdge[] }
export interface CsvCellStyle { font?: string; size?: number; bold?: boolean; color?: string; background?: string; border?: CsvBorderStyle; excel?: CsvExcelStyle }
export interface CsvStyleCondition { columnId: string; op: "eq" | "ne" | "contains" | "not_contains" | "empty" | "not_empty" | "gt" | "gte" | "lt" | "lte" | "num_eq" | "num_ne"; value?: string | number }
export interface CsvStyleRule { id: string; rows?: [number, number]; columns?: string[]; when?: CsvStyleCondition; style: CsvCellStyle }

const hasOwn = (value: object, key: PropertyKey) => Object.prototype.hasOwnProperty.call(value, key);
const colors: Record<string, string> = {
  text: "var(--text-color)", secondary: "var(--text-secondary)", accent: "var(--accent-color)",
  success: "var(--status-good-fg)", warning: "var(--status-warn-fg)", error: "var(--status-danger-fg)",
  surface: "var(--panel-bg)", subtle: "var(--sidebar-bg)", border: "var(--border-color)",
  "accent-soft": "var(--accent-soft)", "success-soft": "var(--status-good-bg)",
  "warning-soft": "var(--status-warn-bg)", "error-soft": "var(--status-danger-bg)", transparent: "transparent",
};
const edges: CsvBorderEdge[] = ["top", "right", "bottom", "left"];
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const fields = (v: Record<string, unknown>, keys: string[]) => Object.keys(v).every((key) => keys.includes(key));
const integer = (v: unknown, min: number, max: number) => typeof v === "number" && Number.isSafeInteger(v) && v >= min && v <= max;
const color = (v: unknown): v is string => typeof v === "string" && (/^#(?:[\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$/i.test(v) || v === "default" || hasOwn(colors, v));
const optional = (v: unknown, check: (v: unknown) => boolean) => v === undefined || check(v);
function numeric(value: string | number): number {
  const text = String(value).trim();
  return /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text) ? Number(text) : NaN;
}

export function validateCsvStyleRules(value: unknown, columns: Record<string, CsvColumnView>): value is CsvStyleRule[] {
  if (!Array.isArray(value) || value.length > 10000) return false;
  const ids = new Set<string>();
  return value.every((rule) => {
    if (!object(rule) || !fields(rule, ["id", "rows", "columns", "when", "style"]) || typeof rule.id !== "string"
      || !rule.id.trim() || new TextEncoder().encode(rule.id).length > 128 || /[\u0000-\u001f\u007f-\u009f]/.test(rule.id) || ids.has(rule.id)) return false;
    ids.add(rule.id);
    if (!optional(rule.rows, (rows) => Array.isArray(rows) && rows.length === 2 && integer(rows[0], 0, 499999) && integer(rows[1], rows[0], 499999))
      || !optional(rule.columns, (ids) => Array.isArray(ids) && ids.length > 0 && new Set(ids).size === ids.length && ids.every((id) => typeof id === "string" && hasOwn(columns, id)))) return false;
    if (rule.when !== undefined) {
      const when = rule.when;
      if (!object(when) || !fields(when, ["columnId", "op", "value"]) || typeof when.columnId !== "string" || !hasOwn(columns, when.columnId)) return false;
      if (["empty", "not_empty"].includes(String(when.op))) { if (when.value !== undefined) return false; }
      else if (["eq", "ne", "contains", "not_contains"].includes(String(when.op))) { if (typeof when.value !== "string" && !(typeof when.value === "number" && Number.isFinite(when.value))) return false; }
      else if (["gt", "gte", "lt", "lte", "num_eq", "num_ne"].includes(String(when.op))) { if ((typeof when.value !== "string" && typeof when.value !== "number") || !Number.isFinite(numeric(when.value))) return false; }
      else return false;
    }
    const style = rule.style;
    if (!object(style) || !Object.keys(style).length || !fields(style, ["font", "size", "bold", "color", "background", "border", "excel"])
      || !optional(style.font, (v) => typeof v === "string" && !!v.trim() && new TextEncoder().encode(v).length <= 256 && !/[\u0000-\u001f\u007f-\u009f;:{}\\(),<>"']/.test(v))
      || !optional(style.size, (v) => integer(v, 8, 72)) || !optional(style.bold, (v) => typeof v === "boolean")
      || !optional(style.color, color) || !optional(style.background, color)
      || !optional(style.excel, (v) => validateCsvExcelStyle(v, color))) return false;
    if (style.border !== undefined) {
      const border = style.border;
      if (!object(border) || !fields(border, ["color", "width", "style", "edges"])
        || !optional(border.color, color) || !optional(border.width, (v) => integer(v, 0, 4))
        || !optional(border.style, (v) => ["solid", "dashed", "dotted", "double", "none"].includes(String(v)))
        || !optional(border.edges, (v) => Array.isArray(v) && v.length > 0 && new Set(v).size === v.length && v.every((item) => edges.includes(item)))) return false;
    }
    return true;
  });
}

/** Explicit, idempotent migration; loading a v1 file does not write it. */
export function migrateCsvViewToV2(view: CsvView): CsvView { return view.schema !== "locus.csv-view.v1" ? view : { ...view, schema: "locus.csv-view.v2" }; }

/** Fixed key order matches the Rust writer; one compact mapping per rule. */
export function serializeCsvStyleRules(rules: CsvStyleRule[]): string {
  if (!rules.length) return "";
  const rows = rules.map((rule) => {
    const style = rule.style, border = style.border;
    return { id: rule.id, ...(rule.rows ? { rows: rule.rows } : {}), ...(rule.columns ? { columns: rule.columns } : {}),
      ...(rule.when ? { when: { columnId: rule.when.columnId, op: rule.when.op, ...(rule.when.value !== undefined ? { value: rule.when.value } : {}) } } : {}),
      style: { ...(style.font !== undefined ? { font: style.font } : {}), ...(style.size !== undefined ? { size: style.size } : {}),
        ...(style.bold !== undefined ? { bold: style.bold } : {}), ...(style.color !== undefined ? { color: style.color } : {}),
        ...(style.background !== undefined ? { background: style.background } : {}), ...(border ? { border: {
          ...(border.color !== undefined ? { color: border.color } : {}), ...(border.width !== undefined ? { width: border.width } : {}),
          ...(border.style !== undefined ? { style: border.style } : {}), ...(border.edges ? { edges: border.edges } : {}),
        } } : {}), ...(style.excel ? { excel: serializeCsvExcelStyle(style.excel) } : {}) } };
  });
  return `styles:\n${rows.map((rule) => `  - ${JSON.stringify(rule).replace(/\u0085/g, "\\u0085").replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029")}\n`).join("")}`;
}

export function reconcileCsvStyles(rules: CsvStyleRule[] | undefined, columns: Record<string, CsvColumnView>): CsvStyleRule[] | undefined {
  return rules?.flatMap((rule) => {
    if (rule.when && !hasOwn(columns, rule.when.columnId)) return [];
    const ids = rule.columns?.filter((id) => hasOwn(columns, id));
    if (ids && !ids.length) return [];
    return [{ ...rule, ...(ids ? { columns: ids } : {}) }];
  });
}

/** Follow source-record edits without expanding ranges into individual cells. */
export function adjustCsvStyleRows(rules: CsvStyleRule[] | undefined, insertion: number | null, removed: number[] = []): CsvStyleRule[] | undefined {
  const deleted = [...new Set(removed)].sort((a, b) => a - b);
  return rules?.flatMap((rule) => {
    if (!rule.rows) return [rule];
    let [start, end] = rule.rows;
    if (insertion !== null) { if (insertion <= start) start++; if (insertion <= end) end++; }
    else {
      const before = deleted.filter((row) => row < start).length;
      const inside = deleted.filter((row) => row >= start && row <= end).length;
      if (inside === end - start + 1) return [];
      start -= before; end -= before + inside;
    }
    return [{ ...rule, rows: [start, end] as [number, number] }];
  });
}

function matches(value: string, when: CsvStyleCondition): boolean {
  const expected = String(when.value ?? "");
  switch (when.op) {
    case "empty": return !value.trim();
    case "not_empty": return !!value.trim();
    case "eq": return value === expected;
    case "ne": return value !== expected;
    case "contains": return value.includes(expected);
    case "not_contains": return !value.includes(expected);
    default: {
      if (!value.trim()) return false;
      const a = numeric(value), b = numeric(expected);
      if (!Number.isFinite(a) || !Number.isFinite(b)) return false;
      return when.op === "num_eq" ? a === b : when.op === "num_ne" ? a !== b : when.op === "gt" ? a > b : when.op === "gte" ? a >= b : when.op === "lt" ? a < b : a <= b;
    }
  }
}

export function adjustCsvRowDimensions(dimensions: CsvView["rowDimensions"], insertion: number | null, removed: number[] = []): CsvView["rowDimensions"] {
  if (!dimensions) return dimensions;
  const deleted = [...new Set(removed)].sort((a, b) => a - b);
  return Object.fromEntries(Object.entries(dimensions).flatMap(([key, value]) => {
    const row = Number(key);
    if (insertion !== null) return [[String(row >= insertion ? row + 1 : row), value]];
    if (deleted.includes(row)) return [];
    return [[String(row - deleted.filter((n) => n < row).length), value]];
  }));
}

export interface ResolvedCsvCellStyle extends Omit<CsvCellStyle, "border"> { borders?: Partial<Record<CsvBorderEdge, string>> }
function cssColor(value: string, fallback: string) { return value === "default" ? fallback : colors[value] ?? value; }

/** Later matching rules override specified properties; borders merge per edge. */
export function resolveCsvCellStyle(document: CsvDocument, view: CsvView, row: number, columnId: string): ResolvedCsvCellStyle {
  const result: ResolvedCsvCellStyle = {};
  for (const rule of view.styles ?? []) {
    if (rule.rows && (row < rule.rows[0] || row > rule.rows[1]) || rule.columns && !rule.columns.includes(columnId)) continue;
    if (rule.when) {
      const sourceIndex = view.columns[rule.when.columnId]?.sourceIndex;
      if (sourceIndex === undefined || !document.records[row] || !matches(document.records[row]?.fields[sourceIndex]?.value ?? "", rule.when)) continue;
    }
    const { border, excel, ...style } = rule.style;
    Object.assign(result, style);
    if (style.background !== undefined && result.excel?.fill) result.excel = { ...result.excel, fill: undefined };
    if (border) {
      result.borders ??= {};
      const value = `${border.width ?? 1}px ${border.style ?? "solid"} ${cssColor(border.color ?? "border", "var(--border-color)")}`;
      for (const edge of border.edges ?? edges) result.borders[edge] = value;
    }
    if (excel) {
      result.excel = { ...result.excel, ...excel };
      if (excel.font) Object.assign(result, { font: excel.font.name, size: excel.font.size * 96 / 72, bold: excel.font.bold, color: excel.font.color });
      if (excel.fill) result.background = excel.fill.patternType === "none" ? "default" : excel.fill.patternType === "solid" ? excel.fill.fgColor : excel.fill.bgColor;
      if (excel.border) {
        result.borders ??= {};
        for (const edge of edges) {
          const side = excel.border[edge];
          result.borders[edge] = !side || side.style === "none"
            ? edge === "right" || edge === "bottom" ? "1px solid var(--border-color)" : "0px none var(--border-color)"
            : csvExcelBorder(side, cssColor);
        }
      }
    }
  }
  return result;
}

export function applyCsvCellStyle(element: HTMLElement, value: ResolvedCsvCellStyle): void {
  const hasStyle = Object.keys(value).length > 0;
  if (!hasStyle && !element.dataset.csvStyled) return;
  if (hasStyle) element.dataset.csvStyled = "true"; else delete element.dataset.csvStyled;
  const style = element.style;
  const fonts: Record<string, string> = { ui: "var(--font-ui)", sans: "var(--font-stack-sans)", mono: "var(--font-stack-mono)" };
  style.fontFamily = value.font ? fonts[value.font] ?? `${JSON.stringify(value.font)}, var(--font-ui)` : "";
  style.fontSize = value.size === undefined ? "" : `${value.size}px`;
  style.fontWeight = value.bold === undefined ? "" : value.bold ? "700" : "400";
  style.fontStyle = value.excel?.font ? value.excel.font.italic ? "italic" : "normal" : "";
  const font = value.excel?.font;
  style.textDecorationLine = font ? [font.underline !== "none" ? "underline" : "", font.strike ? "line-through" : ""].filter(Boolean).join(" ") || "none" : "";
  style.textDecorationStyle = font?.underline.startsWith("double") ? "double" : "";
  const alignment = value.excel?.alignment;
  style.textAlign = alignment ? ["general", "distributed"].includes(alignment.horizontal) ? alignment.horizontal === "general" ? "start" : "justify" : alignment.horizontal : "";
  style.direction = alignment?.readingOrder ? alignment.readingOrder === 2 ? "rtl" : "ltr" : "";
  style.textIndent = alignment?.indent ? `${alignment.indent}ch` : "";
  style.display = alignment ? "inline-flex" : "";
  style.alignItems = alignment ? ({ top: "flex-start", center: "center", bottom: "flex-end" }[alignment.vertical] ?? "center") : "";
  style.justifyContent = alignment ? ({ left: "flex-start", center: "center", right: "flex-end" }[alignment.horizontal] ?? "flex-start") : "";
  style.backgroundImage = value.excel?.fill ? csvExcelFillImage(value.excel.fill, cssColor) : "";
  style.backgroundSize = value.excel?.fill && /Gray|gray/.test(value.excel.fill.patternType) ? "4px 4px" : "";
  for (const [key, color, fallback] of [["--csv-cell-color", value.color, "var(--text-color)"], ["--csv-cell-bg", value.background, "var(--panel-bg)"]] as const) {
    if (color === undefined) style.removeProperty(key); else style.setProperty(key, cssColor(color, fallback));
  }
  // Tabulator recycles cells: clear borders and every optional property first.
  for (const edge of edges) { style.removeProperty(`border-${edge}`); for (const part of ["width", "style", "color"]) style.removeProperty(`border-${edge}-${part}`); if (value.borders?.[edge]) style.setProperty(`border-${edge}`, value.borders[edge]!); }
}
