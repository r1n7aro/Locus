// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { defaultCsvView, parseCsvView, reconcileCsvView, serializeCsvView, type CsvView } from "../document/csv/csvView";
import { parseCsvDocument } from "../document/csv/csvDocument";
import { adjustCsvRowDimensions, applyCsvCellStyle, migrateCsvViewToV2, resolveCsvCellStyle } from "../document/csv/csvStyles";
import { applyCsvExcelText, formatCsvExcelValue, type CsvExcelStyle } from "../document/csv/csvExcelStyles";
import { migrateCsvViewToV3 } from "../document/csv/csvMerges";
import { projectCsvRowIndices } from "../document/csv/csvGridProjection";

const excel: CsvExcelStyle = {
  font: { name: "Arial", size: 12.5, bold: true, italic: true, strike: true, underline: "double", color: "#123456", vertAlign: "baseline" },
  fill: { patternType: "solid", fgColor: "#FFEEDD", bgColor: "transparent" },
  border: { bottom: { style: "double", color: "#FF0000" }, left: { style: "thin", color: "#000000" } },
  alignment: { horizontal: "center", vertical: "center", wrapText: true, shrinkToFit: false, textRotation: 0, indent: 1, readingOrder: 0 },
  numberFormat: "0.0%",
};
function fixture() {
  const document = parseCsvDocument("id,value\n001,0.125\n002,1000\n");
  const view: CsvView = { ...reconcileCsvView(defaultCsvView(), document), schema: "locus.csv-view.v4",
    styles: [{ id: "excel", rows: [1, 1], style: { excel } }], rowDimensions: { "1": { height: 32.5, hidden: false }, "2": { height: 21, hidden: true } } };
  return { document, view };
}
describe("CSV openpyxl display", () => {
  it("round-trips extended styles and dimensions without downgrading v4", () => {
    const { view } = fixture();
    expect(parseCsvView(serializeCsvView(view))).toEqual(view);
    expect(serializeCsvView(parseCsvView(serializeCsvView(view)))).toBe(serializeCsvView(view));
    expect(migrateCsvViewToV2(view).schema).toBe("locus.csv-view.v4");
    expect(migrateCsvViewToV3(view).schema).toBe("locus.csv-view.v4");
    expect(() => parseCsvView(serializeCsvView({ ...view, schema: "locus.csv-view.v3" }))).toThrow();
  });
  it("applies real font, alignment, fill and independent border properties and clears recycled cells", () => {
    const { document, view } = fixture();
    const cell = globalThis.document.createElement("div");
    const style = resolveCsvCellStyle(document, view, 1, view.columnOrder[1]!);
    applyCsvCellStyle(cell, style);
    expect(cell.style.fontSize).toBe(`${12.5 * 96 / 72}px`);
    expect(cell.style.fontStyle).toBe("italic");
    expect(cell.style.textDecorationLine).toBe("underline line-through");
    expect(cell.style.textDecorationStyle).toBe("double");
    expect(cell.style.borderBottomStyle).toBe("double");
    expect(cell.style.borderLeftWidth).toBe("1px");
    expect(cell.style.textAlign).toBe("center");
    expect(cell.style.alignItems).toBe("center");
    const text = globalThis.document.createElement("span");
    applyCsvExcelText(text, "0.125", style.excel, false);
    expect(text.textContent).toBe("12.5%");
    expect(text.style.whiteSpace).toBe("pre-wrap");
    applyCsvCellStyle(cell, {});
    for (const key of ["fontSize", "fontStyle", "textDecorationLine", "textDecorationStyle", "textAlign", "alignItems", "borderBottomStyle", "backgroundImage"] as const) expect(cell.style[key]).toBe("");
  });
  it("formats numeric text only when explicitly requested and leaves source identifiers intact", () => {
    for (const [value, fmt, output] of [["001", "General", "001"], ["001", "@", "001"], ["1234.5", "#,##0.00", "1,234.50"],
      ["0.125", "0.0%", "12.5%"], ["1234.5", "$#,##0.00", "$1,234.50"], ["45292", "yyyy-mm-dd", "2024-01-01"],
      ["2024-01-01", "yyyy/mm/dd", "2024/01/01"], ["words", "0.00", "words"]]) expect(formatCsvExcelValue(value!, fmt!)).toBe(output);
  });
  it("preserves sparse row dimensions across edits and hides the intended source row", () => {
    const { document, view } = fixture();
    expect(projectCsvRowIndices(document, view, 2)).not.toContain(2);
    expect(projectCsvRowIndices(document, view, 2)).toContain(1);
    expect(adjustCsvRowDimensions(view.rowDimensions, 1)).toEqual({ "2": view.rowDimensions!["1"], "3": view.rowDimensions!["2"] });
    expect(adjustCsvRowDimensions(view.rowDimensions, null, [1])).toEqual({ "1": view.rowDimensions!["2"] });
  });
  it("rejects malformed rich styles and arbitrary CSS", () => {
    const { view } = fixture();
    for (const value of [{ font: { ...excel.font!, size: NaN } }, { fill: { ...excel.fill!, fgColor: "url(x)" } },
      { alignment: { ...excel.alignment!, textRotation: 190 } }, { border: { diagonal: { style: "thin", color: "#000" } } },
      { numberFormat: "a\nb" }, { protection: true }]) {
      expect(() => parseCsvView(serializeCsvView({ ...view, styles: [{ id: "invalid", style: { excel: value as CsvExcelStyle } }] }))).toThrow();
    }
  });
});
