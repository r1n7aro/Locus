import { createApp, h, nextTick, shallowRef, type App } from "vue";
import { TabulatorFull as Tabulator } from "tabulator-tables";
import CsvGrid from "../../src/components/csv/CsvGrid.vue";
import { applyCsvCellEdits, parseCsvDocument, type CsvCellEdit, type CsvDocument } from "../../src/document/csv/csvDocument";
import { defaultCsvView, reconcileCsvView, type CsvView } from "../../src/document/csv/csvView";
import "./style.css";

let app: App | null = null;
let source = "";
const documentRef = shallowRef<CsvDocument>();
const view = shallowRef<CsvView>();
const grid = shallowRef<InstanceType<typeof CsvGrid> | null>(null);
const errors: unknown[] = [];
const afterPaint = async () => { await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
const table = () => Tabulator.findTable(".csv-grid")[0]!;
const scroller = () => document.querySelector<HTMLElement>(".tabulator-tableholder")!;
function edit(edits: CsvCellEdit[]) {
  const result = applyCsvCellEdits(documentRef.value!, source, edits);
  source = result.text;
  documentRef.value = result.document;
  view.value = reconcileCsvView(view.value!, result.document);
}
async function load(text: string, frozen = 2) {
  app?.unmount();
  document.querySelector("#grid")!.replaceChildren();
  errors.length = 0;
  const start = performance.now();
  source = text;
  documentRef.value = parseCsvDocument(source);
  view.value = reconcileCsvView({ ...defaultCsvView(), frozenColumns: frozen }, documentRef.value);
  const projectionMs = performance.now() - start;
  app = createApp({ setup: () => () => h(CsvGrid, { ref: grid,
    document: documentRef.value!, view: view.value!, active: true,
    onEdit: edit, onViewChange: (next: CsvView) => { view.value = next; }, onError: (error: unknown) => errors.push(error),
  }) });
  app.mount(document.querySelector("#grid")!);
  const deadline = performance.now() + 30000;
  while (!Tabulator.findTable(".csv-grid")[0]?.getDataCount() || !document.querySelector(".tabulator-row .tabulator-cell")) {
    if (errors.length) throw errors[0];
    if (performance.now() > deadline) throw new Error("CsvGrid did not render its initial rows.");
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  await afterPaint();
  if (errors.length) throw errors[0];
  document.querySelector("#status")!.textContent = `${documentRef.value.records.length} 行 × ${documentRef.value.columnCount} 列`;
  return { totalMs: performance.now() - start, projectionMs,
    cells: documentRef.value.records.reduce((sum, row) => sum + row.fields.length, 0) };
}
const probe = {
  load,
  async setView(patch: Partial<CsvView>) {
    view.value = { ...view.value!, ...patch };
    await nextTick();
    await grid.value!.refresh();
    await afterPaint();
  },
  async sample(rows: number, columns: number) {
    return load(Array.from({ length: rows }, (_, r) => Array.from({ length: columns }, (_, c) => `${r}:${c}`).join(",")).join("\r\n"));
  },
  async scroll(row: number, column: number) {
    scroller().scrollTop = row * 28;
    scroller().scrollLeft = Math.max(0, column - 2) * 140;
    await afterPaint();
    return { top: scroller().scrollTop, left: scroller().scrollLeft };
  },
  async benchmarkEdit(row: number, column: number, value: string) {
    const start = performance.now();
    edit([{ row, column, value }]);
    await nextTick();
    await afterPaint();
    const cell = table().getRow(row);
    if (!cell || cell.getData()[view.value!.columnOrder[column]!] !== value) throw new Error("CSV incremental update did not reach Tabulator.");
    if (errors.length) throw errors[0];
    return { totalMs: performance.now() - start, value };
  },
  get source() { return source; }, get grid() { return grid.value; }, get table() { return table(); },
  get view() { return view.value; },
  metrics() { const box=document.querySelector("#grid")!.getBoundingClientRect();return {
    elements: document.querySelectorAll("*").length, cells: document.querySelectorAll(".tabulator-cell").length,
    canvases: document.querySelectorAll("canvas").length, width: box.width, height: box.height,
  }; },
};
Object.assign(window, { probe });
if (!new URLSearchParams(location.search).has("benchmark")) await probe.sample(100, 20);
Object.assign(window, { probeReady: true });
