/// <reference types="vite/client" />
import { CellValueType, extractPureTextFromCell, LifecycleStages, LocaleType, mergeLocales, Univer, type ICellData, type IWorkbookData } from "@univerjs/core";
import { FUniver } from "@univerjs/core/facade";
import { UniverRenderEnginePlugin } from "@univerjs/engine-render";
import { UniverUIPlugin } from "@univerjs/ui";
import { UniverDocsPlugin } from "@univerjs/docs";
import { UniverDocsUIPlugin } from "@univerjs/docs-ui";
import { AddWorksheetMergeMutation, RemoveWorksheetMergeMutation, UniverSheetsPlugin } from "@univerjs/sheets";
import { UniverSheetsUIPlugin } from "@univerjs/sheets-ui";
import { UniverSheetsNumfmtPlugin } from "@univerjs/sheets-numfmt";
import { UniverSheetsNumfmtUIPlugin } from "@univerjs/sheets-numfmt-ui";
import { UniverFormulaEnginePlugin } from "@univerjs/engine-formula";
import { UniverSheetsFormulaPlugin } from "@univerjs/sheets-formula";
import { UniverSheetsFormulaUIPlugin } from "@univerjs/sheets-formula-ui";
import DesignZh from "@univerjs/design/locale/zh-CN";
import UIZh from "@univerjs/ui/locale/zh-CN";
import DocsUIZh from "@univerjs/docs-ui/locale/zh-CN";
import SheetsZh from "@univerjs/sheets/locale/zh-CN";
import SheetsUIZh from "@univerjs/sheets-ui/locale/zh-CN";
import NumfmtUIZh from "@univerjs/sheets-numfmt-ui/locale/zh-CN";
import FormulaUIZh from "@univerjs/sheets-formula-ui/locale/zh-CN";
import "@univerjs/ui/facade";
import "@univerjs/docs-ui/facade";
import "@univerjs/sheets/facade";
import "@univerjs/sheets-ui/facade";
import "@univerjs/sheets-numfmt/facade";
import "@univerjs/sheets-formula/facade";
import "@univerjs/design/lib/index.css";
import "@univerjs/ui/lib/index.css";
import "@univerjs/docs-ui/lib/index.css";
import "@univerjs/sheets-ui/lib/index.css";
import "@univerjs/sheets-numfmt-ui/lib/index.css";
import "@univerjs/sheets-formula-ui/lib/index.css";
import "./style.css";
import { applyCsvCellEdits, parseCsvDocument, serializeCsvDocument, type CsvDocument } from "../../src/document/csv/csvDocument";

// This is a disposable validation host, not a production editor replacement.
// The production CSV parser is deliberately reused, so lexical preservation is
// measured against Locus rather than against a new export implementation.
let univer: Univer;
let api: FUniver;
let workbook: ReturnType<FUniver["createWorkbook"]>;
let source = "";
let documentModel: CsvDocument;
let serial = 0;
const events: unknown[] = [];
const afterPaint = async () => { await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
const status = (value: string) => { document.querySelector("#status")!.textContent = value; };
const sheet = () => workbook.getActiveSheet();
const cellText = (cell: ICellData | null) => cell?.p?.body
  ? extractPureTextFromCell(cell).replaceAll("\r\n", "\n").replaceAll("\r", "\n")
  : String(cell?.v ?? "");

async function mount(snapshot: Partial<IWorkbookData>) {
  univer?.dispose();
  document.querySelector("#grid")!.replaceChildren();
  const start = performance.now();
  univer = new Univer({ locale: LocaleType.ZH_CN, darkMode: true,
    locales: { [LocaleType.ZH_CN]: mergeLocales(DesignZh, UIZh, DocsUIZh, SheetsZh, SheetsUIZh, NumfmtUIZh, FormulaUIZh) } });
  univer.registerPlugin(UniverRenderEnginePlugin);
  univer.registerPlugin(UniverFormulaEnginePlugin);
  univer.registerPlugin(UniverUIPlugin, { container: "grid", header: false, toolbar: false, footer: false });
  univer.registerPlugin(UniverDocsPlugin);
  univer.registerPlugin(UniverDocsUIPlugin);
  univer.registerPlugin(UniverSheetsPlugin);
  univer.registerPlugin(UniverSheetsUIPlugin, { formulaBar: false, footer: false });
  univer.registerPlugin(UniverSheetsNumfmtPlugin);
  univer.registerPlugin(UniverSheetsNumfmtUIPlugin);
  // In 0.25.1 this OSS plugin supplies the normal cell editor too, even with
  // formulaBar:false. Omitting it renders a grid with no keyboard editor.
  univer.registerPlugin(UniverSheetsFormulaPlugin);
  univer.registerPlugin(UniverSheetsFormulaUIPlugin);
  api = FUniver.newAPI(univer);
  api.addEvent(api.Event.CommandExecuted, (event) => {
    events.push({ id: event.id, parameterKeys: Object.keys(event.params ?? {}) });
    if (events.length > 100) events.shift();
  });
  workbook = api.createWorkbook(snapshot);
  const deadline = performance.now() + 10000;
  while (api.getCurrentLifecycleStage() < LifecycleStages.Rendered || !document.querySelector("#grid canvas")) {
    if (performance.now() > deadline) throw new Error("Univer did not reach its rendered lifecycle stage.");
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  await afterPaint();
  status(`${sheet().getMaxRows()} 行 × ${sheet().getMaxColumns()} 列`);
  return { mountMs: performance.now() - start, canvases: document.querySelectorAll("canvas").length };
}

async function load(text: string, options: { frozen?: number; rows?: number; columns?: number } = {}) {
  source = text;
  const start = performance.now();
  const parsed = documentModel = parseCsvDocument(source);
  const cellData: Record<number, Record<number, ICellData>> = {};
  parsed.records.forEach((record, row) => {
    cellData[row] = {};
    record.fields.forEach((field, column) => { cellData[row]![column] = { v: field.value, t: CellValueType.STRING, s: "plain" }; });
  });
  const rowCount = Math.max(options.rows ?? 100, parsed.records.length + 20);
  const columnCount = Math.max(options.columns ?? 26, parsed.columnCount + 8);
  // 0.25.1's input/paste paths read cell.s, not the composed worksheet default.
  // Explicit styles on the finite editing surface prevent numeric/formula
  // inference in empty cells. This is an adapter requirement being measured.
  for (let row = 0; row < rowCount; row++) {
    cellData[row] ??= {};
    for (let column = 0; column < columnCount; column++) cellData[row]![column] ??= { s: "plain" };
  }
  const frozen = options.frozen ?? 2;
  const projectionMs = performance.now() - start;
  const result = await mount({ id: `csv-probe-${++serial}`, name: "CSV 验证", sheetOrder: ["sheet"],
    styles: { plain: { n: { pattern: "@" }, fs: 13 } },
    sheets: { sheet: { id: "sheet", name: "CSV", rowCount,
      columnCount, defaultColumnWidth: 140, defaultRowHeight: 28,
      freeze: { xSplit: frozen, ySplit: 0, startColumn: frozen, startRow: -1 },
      cellData, defaultStyle: { n: { pattern: "@" }, fs: 13 } } } });
  return { ...result, projectionMs, totalMs: performance.now() - start, cells: parsed.records.reduce((n, row) => n + row.fields.length, 0) };
}

const probe = {
  load,
  async loadSnapshot(snapshot: Partial<IWorkbookData>, text: string) { source = text; documentModel = parseCsvDocument(text); return mount(snapshot); },
  async sample(rows = 100, columns = 20) {
    return load(Array.from({ length: rows }, (_, r) => Array.from({ length: columns }, (_, c) => `${r}:${c}`).join(",")).join("\r\n"), { columns: columns + 8 });
  },
  async benchmarkEdit(row: number, column: number, value: string) {
    const start = performance.now();
    const result = applyCsvCellEdits(documentModel, source, [{ row, column, value }]);
    source = result.text; documentModel = result.document;
    sheet().getRange(row,column).setValue({ v:value,t:CellValueType.STRING,f:null,s:"plain" });
    await afterPaint();
    if (sheet().getRange(row,column).getValue() !== value) throw new Error("CSV incremental update did not reach Univer.");
    return {totalMs:performance.now()-start,value};
  },
  get api() { return api; }, get workbook() { return workbook; }, get sheet() { return sheet(); },
  get events() { return events; }, get source() { return source; },
  cells(a1: string) { return sheet().getRange(a1).getCellDatas(); },
  text(a1: string) { return cellText(sheet().getRange(a1).getCellData()); },
  values(a1: string) { return sheet().getRange(a1).getValues(); },
  selection() { return sheet().getSelection()?.getActiveRange()?.getRange() ?? sheet().getActiveRange()?.getRange(); },
  async select(a1: string) { sheet().setActiveRange(sheet().getRange(a1)); await afterPaint(); return this.selection(); },
  async freeze(count: number) { sheet().setFrozenColumns(count); await afterPaint(); return workbook.save().sheets.sheet!.freeze; },
  async scroll(row: number, column: number) { sheet().scrollToCell(row, column, 0); await afterPaint(); return sheet().getVisibleRange(); },
  async literal(a1: string, text: string) { sheet().getRange(a1).setValue({ v: text, t: CellValueType.STRING, f: null, s: { n: { pattern: "@" } } }); await afterPaint(); },
  async merge(a1: string) { sheet().getRange(a1).merge(); await afterPaint(); return workbook.save().sheets.sheet!.mergeData; },
  async unmerge(a1: string) { sheet().getRange(a1).breakApart(); await afterPaint(); },
  async mergeView(a1: string, remove = false) {
    // The CSV .view model owns merge semantics/history. The exported mutation
    // changes only geometry, whereas the user-facing merge command clears data.
    const result = await api.executeCommand(remove ? RemoveWorksheetMergeMutation.id : AddWorksheetMergeMutation.id, {
      unitId: workbook.getId(), subUnitId: sheet().getSheetId(), ranges: [sheet().getRange(a1).getRange()],
    });
    await afterPaint(); return result;
  },
  async zoom(ratio: number) { sheet().zoom(ratio); await afterPaint(); return sheet().getZoom(); },
  async recreate() {
    const snapshot = workbook.save();
    const selection = this.selection();
    const scroll = sheet().getScrollState();
    const result = await mount(snapshot);
    if (selection) sheet().setActiveRange(sheet().getRange(selection));
    if (scroll) sheet().scrollToCell(scroll.sheetViewStartRow, scroll.sheetViewStartColumn, 0);
    await afterPaint();
    return { ...result, selection: this.selection(), scroll: sheet().getScrollState() };
  },
  // Diff only a known edited source cell. This demonstrates the integration
  // boundary; merging must never be exported as deletions of covered CSV cells.
  commitCell(row: number, column: number) {
    const parsed = parseCsvDocument(source);
    const value = cellText(sheet().getRange(row, column).getCellData());
    source = applyCsvCellEdits(parsed, source, [{ row, column, value: String(value ?? "") }]).text;
    return source;
  },
  unchangedRoundTrip() { return serializeCsvDocument(parseCsvDocument(source)); },
  snapshot() { return workbook.save(); },
  metrics() { return { elements: document.querySelectorAll("*").length, canvases: document.querySelectorAll("canvas").length,
    width: document.querySelector("#grid")!.getBoundingClientRect().width, height: document.querySelector("#grid")!.getBoundingClientRect().height,
    heap: (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? null }; },
};
Object.assign(window, { probe });
if (!new URLSearchParams(location.search).has("benchmark")) await probe.sample();
Object.assign(window, { probeReady: true });
