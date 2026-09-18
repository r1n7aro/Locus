import { format, is_date } from "ssf";

export interface CsvExcelFont { name: string; size: number; bold: boolean; italic: boolean; strike: boolean; underline: string; color: string; vertAlign: string }
export interface CsvExcelFill { patternType: string; fgColor: string; bgColor: string }
export interface CsvExcelSide { style: string; color: string }
export interface CsvExcelAlignment { horizontal: string; vertical: string; wrapText: boolean; shrinkToFit: boolean; textRotation: number; indent: number; readingOrder: number }
export interface CsvExcelStyle {
  font?: CsvExcelFont; fill?: CsvExcelFill;
  border?: Partial<Record<"top" | "right" | "bottom" | "left", CsvExcelSide>>;
  alignment?: CsvExcelAlignment; numberFormat?: string;
}

export function serializeCsvExcelStyle(style: CsvExcelStyle): CsvExcelStyle {
  const f = style.font, fill = style.fill, a = style.alignment;
  return {
    ...(f ? { font: { name: f.name, size: f.size, bold: f.bold, italic: f.italic, strike: f.strike, underline: f.underline, color: f.color, vertAlign: f.vertAlign } } : {}),
    ...(fill ? { fill: { patternType: fill.patternType, fgColor: fill.fgColor, bgColor: fill.bgColor } } : {}),
    ...(style.border ? { border: Object.fromEntries(Object.entries(style.border).sort(([a], [b]) => a.localeCompare(b)).map(([edge, side]) => [edge, { style: side.style, color: side.color }])) } : {}),
    ...(a ? { alignment: { horizontal: a.horizontal, vertical: a.vertical, wrapText: a.wrapText, shrinkToFit: a.shrinkToFit, textRotation: a.textRotation, indent: a.indent, readingOrder: a.readingOrder } } : {}),
    ...(style.numberFormat !== undefined ? { numberFormat: style.numberFormat } : {}),
  };
}
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const keys = (v: Record<string, unknown>, fields: string[]) => Object.keys(v).every((k) => fields.includes(k));
const integer = (v: unknown, min: number, max: number) => typeof v === "number" && Number.isInteger(v) && v >= min && v <= max;
const patterns = ["none", "solid", "darkDown", "darkGray", "darkGrid", "darkHorizontal", "darkTrellis", "darkUp", "darkVertical", "gray0625", "gray125", "lightDown", "lightGray", "lightGrid", "lightHorizontal", "lightTrellis", "lightUp", "lightVertical", "mediumGray"];
const borderStyles = ["none", "thin", "medium", "thick", "hair", "dashed", "dotted", "double", "dashDot", "dashDotDot", "mediumDashed", "mediumDashDot", "mediumDashDotDot", "slantDashDot"];

export function validateCsvExcelStyle(v: unknown, color: (value: unknown) => boolean): v is CsvExcelStyle {
  if (!object(v) || !Object.keys(v).length || !keys(v, ["font", "fill", "border", "alignment", "numberFormat"])) return false;
  if (v.font !== undefined) {
    const f = v.font;
    if (!object(f) || !keys(f, ["name", "size", "bold", "italic", "strike", "underline", "color", "vertAlign"])
      || typeof f.name !== "string" || !f.name.trim() || new TextEncoder().encode(f.name).length > 256 || /[\u0000-\u001f\u007f-\u009f;:{}\\(),<>"']/.test(f.name)
      || typeof f.size !== "number" || !Number.isFinite(f.size) || f.size < 1 || f.size > 409
      || ![f.bold, f.italic, f.strike].every((b) => typeof b === "boolean") || !color(f.color)
      || !["none", "single", "double", "singleAccounting", "doubleAccounting"].includes(String(f.underline))
      || !["baseline", "superscript", "subscript"].includes(String(f.vertAlign))) return false;
  }
  if (v.fill !== undefined && (!object(v.fill) || !keys(v.fill, ["patternType", "fgColor", "bgColor"])
    || !patterns.includes(String(v.fill.patternType)) || !color(v.fill.fgColor) || !color(v.fill.bgColor))) return false;
  if (v.border !== undefined && (!object(v.border) || !keys(v.border, ["top", "right", "bottom", "left"])
    || !Object.values(v.border).every((s) => object(s) && keys(s, ["style", "color"]) && borderStyles.includes(String(s.style)) && color(s.color)))) return false;
  if (v.alignment !== undefined) {
    const a = v.alignment;
    if (!object(a) || !keys(a, ["horizontal", "vertical", "wrapText", "shrinkToFit", "textRotation", "indent", "readingOrder"])
      || !["general", "left", "center", "right", "justify", "distributed"].includes(String(a.horizontal))
      || !["top", "center", "bottom", "justify", "distributed"].includes(String(a.vertical))
      || typeof a.wrapText !== "boolean" || typeof a.shrinkToFit !== "boolean"
      || !(integer(a.textRotation, 0, 180) || a.textRotation === 255) || !integer(a.indent, 0, 255) || !integer(a.readingOrder, 0, 2)) return false;
  }
  return v.numberFormat === undefined || typeof v.numberFormat === "string" && v.numberFormat.length > 0
    && new TextEncoder().encode(v.numberFormat).length <= 512 && !/[\u0000-\u001f\u007f-\u009f]/.test(v.numberFormat);
}

export function csvExcelBorder(side: CsvExcelSide, color: (value: string, fallback: string) => string): string {
  const kind = side.style;
  const width = kind === "none" ? 0 : kind === "thick" || kind === "double" ? 3 : kind.startsWith("medium") ? 2 : kind === "hair" ? 0.5 : 1;
  const line = kind === "none" ? "none" : kind === "double" ? "double" : kind.toLowerCase().includes("dash") ? "dashed" : kind === "dotted" ? "dotted" : "solid";
  return `${width}px ${line} ${color(side.color, "var(--border-color)")}`;
}

/** Number formats affect display only. General / text preserve identifiers such as 001. */
export function formatCsvExcelValue(value: string, numberFormat?: string): string {
  if (!numberFormat || numberFormat === "General" || numberFormat === "@" || !value.trim()) return value;
  let input: number | Date | string = value;
  if (/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value.trim())) {
    const number = Number(value);
    if (!Number.isFinite(number)) return value;
    input = number;
  } else if (is_date(numberFormat) && /^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?)?$/.test(value)) {
    const date = new Date(`${value.replace(" ", "T")}${value.length === 10 ? "T00:00:00" : ""}`);
    if (!Number.isNaN(date.getTime())) input = date;
  }
  try { return format(numberFormat, input); } catch { return value; }
}

/** Pattern fills use CSS primitives; Excel's pattern geometry is approximated. */
export function csvExcelFillImage(fill: CsvExcelFill, color: (value: string, fallback: string) => string): string {
  if (["none", "solid"].includes(fill.patternType)) return "";
  const fg = color(fill.fgColor, "var(--text-color)");
  const p = fill.patternType;
  if (/Gray|gray/.test(p)) return `radial-gradient(${fg} ${p === "gray0625" ? "0.5px" : "1px"}, transparent 1px)`;
  const angle = /Horizontal/.test(p) ? 0 : /Vertical/.test(p) ? 90 : /Down/.test(p) ? 135 : 45;
  const width = p.startsWith("dark") ? 2 : 1;
  const line = (deg: number) => `repeating-linear-gradient(${deg}deg, ${fg} 0 ${width}px, transparent ${width}px 6px)`;
  return /Grid|Trellis/.test(p) ? `${line(angle)}, ${line(angle + 90)}` : line(angle);
}

export function applyCsvExcelText(text: HTMLElement, value: string, excel: CsvExcelStyle | undefined, wrap: boolean): void {
  text.textContent = formatCsvExcelValue(value, excel?.numberFormat);
  const a = excel?.alignment;
  // Global text tokens apply to spans too. Cell-specific fonts must inherit through this inner element.
  text.style.fontFamily = "inherit";
  text.style.fontWeight = "inherit";
  text.style.fontStyle = "inherit";
  text.style.whiteSpace = (a?.wrapText ?? wrap) ? "pre-wrap" : "pre";
  text.style.display = a || excel?.font?.vertAlign !== undefined ? "inline-block" : "";
  text.style.width = a && !a.shrinkToFit && !a.textRotation ? "100%" : "";
  text.style.writingMode = a?.textRotation === 255 ? "vertical-rl" : "";
  text.style.textOrientation = a?.textRotation === 255 ? "upright" : "";
  const rotation = a?.textRotation && a.textRotation !== 255 ? a.textRotation > 90 ? a.textRotation - 90 : -a.textRotation : 0;
  text.style.transform = rotation ? `rotate(${rotation}deg)` : "";
  text.style.transformOrigin = "center";
  const vertical = excel?.font?.vertAlign;
  text.style.fontSize = vertical === "superscript" || vertical === "subscript" ? "75%" : "inherit";
  text.style.verticalAlign = vertical === "superscript" ? "super" : vertical === "subscript" ? "sub" : "";
  const font = excel?.font;
  text.style.textDecorationLine = font ? [font.underline !== "none" ? "underline" : "", font.strike ? "line-through" : ""].filter(Boolean).join(" ") || "none" : "";
  text.style.textDecorationStyle = font?.underline.startsWith("double") ? "double" : "";
}

export function fitCsvExcelText(text: HTMLElement, excel: CsvExcelStyle | undefined): void {
  if (!excel?.alignment?.shrinkToFit || excel.alignment.wrapText || !text.parentElement) return;
  const parent = text.parentElement;
  const css = getComputedStyle(parent);
  const available = parent.clientWidth - parseFloat(css.paddingLeft || "0") - parseFloat(css.paddingRight || "0");
  if (available > 0 && text.scrollWidth > available) text.style.transform += ` scaleX(${available / text.scrollWidth})`;
}
