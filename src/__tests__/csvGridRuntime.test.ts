// @vitest-environment jsdom
import { createApp, h, nextTick, shallowRef, type App } from "vue";
import { TabulatorFull as Tabulator } from "tabulator-tables";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import CsvGrid from "../components/csv/CsvGrid.vue";
import { applyCsvCellEdits, parseCsvDocument } from "../document/csv/csvDocument";
import { defaultCsvView, reconcileCsvView, type CsvView } from "../document/csv/csvView";
import { mergeCsvSelection } from "../document/csv/csvMerges";

const apps: App[] = [];
const errors: unknown[] = [];
const onError = (event: ErrorEvent) => { errors.push(event.error); event.preventDefault(); };
const settle = async () => { await nextTick(); await new Promise((resolve) => setTimeout(resolve, 30)); };

beforeEach(() => {
  // Supply layout metrics while keeping the real Tabulator renderers, frozen
  // columns, range selection, scroll listeners, and Vue lifecycle under test.
  vi.spyOn(HTMLElement.prototype, "offsetParent", "get").mockImplementation(function (this: HTMLElement) {
    return this.isConnected ? this.parentElement : null;
  });
  for (const property of ["clientWidth", "offsetWidth"] as const) {
    vi.spyOn(HTMLElement.prototype, property, "get").mockImplementation(function (this: HTMLElement) {
      return Number.parseFloat(this.style.width) || 700;
    });
  }
  for (const property of ["clientHeight", "offsetHeight"] as const) {
    vi.spyOn(HTMLElement.prototype, property, "get").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("tabulator-row") || this.classList.contains("tabulator-cell") ? 28 : 400;
    });
  }
  vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("tabulator-row") ? (Number(this.querySelector(".tabulator-row-header")?.textContent) - 1) * 28 : 0;
  });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  window.addEventListener("error", onError);
});
afterEach(async () => {
  for (const app of apps.splice(0)) app.unmount();
  await settle();
  window.removeEventListener("error", onError);
  document.body.innerHTML = "";
  vi.restoreAllMocks(); vi.unstubAllGlobals(); errors.length = 0;
});

async function mount(text = "id,name\n001,first\n002,second\n") {
  const source = shallowRef(text);
  const documentRef = shallowRef(parseCsvDocument(text));
  const view = shallowRef(reconcileCsvView(defaultCsvView(), documentRef.value));
  const active = shallowRef(true);
  const grid = shallowRef<InstanceType<typeof CsvGrid> | null>(null);
  const host = document.createElement("div"); document.body.appendChild(host);
  const edit = (edits: Parameters<typeof applyCsvCellEdits>[2]) => {
    const result = applyCsvCellEdits(documentRef.value, source.value, edits);
    source.value = result.text; documentRef.value = result.document;
    view.value = reconcileCsvView(view.value, result.document);
  };
  const app = createApp({ setup: () => () => h(CsvGrid, { ref: grid, document: documentRef.value, view: view.value,
    active: active.value, onEdit: edit, onViewChange: (next: CsvView) => { view.value = next; }, onError: (error: unknown) => errors.push(error) }) });
  apps.push(app); app.mount(host);
  await vi.waitFor(() => expect(Tabulator.findTable(".csv-grid")[0]?.getDataCount()).toBe(Math.max(100, documentRef.value.records.length + 20)));
  await settle();
  const table = Tabulator.findTable(".csv-grid")[0]!;
  const scroller = host.querySelector<HTMLElement>(".tabulator-tableholder")!;
  Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 2800 });
  Object.defineProperty(scroller, "scrollWidth", { configurable: true, value: 4000 });
  return { app, host, grid, table, scroller, documentRef, view, active, edit, source };
}

describe("CSV grid with real Tabulator", () => {
  it("keeps the frozen selection visible outside the horizontal virtual window", async () => {
    const warning = vi.spyOn(console, "warn");
    const { grid, table, scroller, view, host } = await mount("a,b,c,d,e,f\n0,1,2,3,4,5\n6,7,8,9,10,11\n");
    view.value = { ...view.value, frozenColumns: 3 }; await settle();
    await grid.value!.applySnapshot({ row: 1, column: 0, endRow: 2, endColumn: 18, scrollLeft: 1400, scrollTop: 0 });
    scroller.dispatchEvent(new Event("scroll")); await settle();
    const fixed = host.querySelector<HTMLElement>('.csv-range-fragment[data-frozen="true"]')!;
    const scrolling = host.querySelector<HTMLElement>('.csv-range-fragment[data-frozen="false"]')!;
    const headerWidth = table.getColumns()[0]!.getWidth();
    const frozenWidth = view.value.columnOrder.slice(0, 3).reduce((width, id) => width + table.getColumn(id).getWidth(), 0);
    expect(fixed.style.display).toBe("block");
    expect(Number.parseFloat(fixed.style.left)).toBe(headerWidth);
    expect(Number.parseFloat(fixed.style.width)).toBe(frozenWidth);
    expect(Number.parseFloat(scrolling.style.left)).toBe(headerWidth + frozenWidth - scroller.scrollLeft);
    expect(grid.value!.selectedColumns()).toEqual(Array.from({ length: 19 }, (_, index) => index));
    expect(grid.value!.selectedRows()).toEqual([1, 2]);
    const copied = vi.fn();
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(copy, "clipboardData", { value: { setData: copied } });
    scroller.dispatchEvent(copy);
    expect(copied).toHaveBeenCalledWith("text/plain", ["0\t1\t2\t3\t4\t5" + "\t".repeat(13), "6\t7\t8\t9\t10\t11" + "\t".repeat(13)].join("\n"));
    // A selection entirely in the frozen prefix must survive a far-right scroll.
    await grid.value!.applySnapshot({ row: 1, column: 0, endColumn: 1, scrollLeft: 1400, scrollTop: 0 });
    await settle();
    expect(fixed.style.display).toBe("block");
    expect(scrolling.style.display).toBe("none");
    expect(warning).not.toHaveBeenCalled();
    expect(errors).toEqual([]);
  });

  it("reveals keyboard destinations beyond the whole frozen prefix and preserves Shift selection", async () => {
    const { grid, scroller, view, table } = await mount("a,b,c,d,e,f\n0,1,2,3,4,5\n");
    view.value = { ...view.value, frozenColumns: 2 }; await settle();
    await grid.value!.applySnapshot({ row: 1, column: 1, scrollLeft: 1000, scrollTop: 0 });
    scroller.dispatchEvent(new Event("scroll")); await settle();
    const key = async (key: string, shiftKey = false) => {
      scroller.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true })); await settle();
    };
    await key("ArrowLeft");
    expect(scroller.scrollLeft).toBe(1000); // Frozen navigation leaves the scroll position alone.
    await key("ArrowRight", true);
    await key("ArrowRight", true);
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 0, endRow: 1, endColumn: 2 });
    expect(scroller.scrollLeft).toBe(0); // C was underneath frozen A/B.
    expect(table.getRanges()[0]!.getColumns().map((column) => column.getField())).toEqual(view.value.columnOrder.slice(0, 3));
    expect(errors).toEqual([]);
  });

  it("keeps frozen columns on the left after hiding, reordering and zooming", async () => {
    const warning = vi.spyOn(console, "warn");
    const { grid, table, view } = await mount("a,b,c,d\n0,1,2,3\n");
    const hidden = view.value.columnOrder[0]!;
    view.value = { ...view.value, frozenColumns: 2, columns: { ...view.value.columns, [hidden]: { ...view.value.columns[hidden]!, hidden: true } } };
    await settle();
    for (const zoom of [1, 1.5]) {
      grid.value!.setZoom(zoom); await settle(); await settle();
      const visible = table.getColumns().filter((column) => column.isVisible());
      expect(visible.slice(0, 3).every((column) => column.getDefinition().frozen)).toBe(true);
      expect(visible[2]!.getElement().classList.contains("tabulator-frozen-left")).toBe(true);
      expect(visible[2]!.getElement().classList.contains("tabulator-frozen-right")).toBe(false);
    }
    view.value = { ...view.value, columnOrder: [hidden, ...view.value.columnOrder.slice(1).reverse()] }; await settle();
    expect(table.getColumns().filter((column) => column.isVisible())[2]!.getElement().classList.contains("tabulator-frozen-left")).toBe(true);
    view.value = { ...view.value, frozenColumns: 0 }; await settle();
    expect(table.getColumns().filter((column) => column.isVisible() && column.getDefinition().frozen)).toHaveLength(1);
    expect(warning).not.toHaveBeenCalled();
    expect(errors).toEqual([]);
  });

  it("ends a range drag in the current owner document after moving into a floating window", async () => {
    const { host, grid, table, view } = await mount("a,b,c,d\n0,1,2,3\n4,5,6,7\n8,9,10,11\n");
    view.value = { ...view.value, frozenColumns: 2 }; await settle();
    const frame = document.createElement("iframe"); document.body.appendChild(frame);
    frame.contentDocument!.body.appendChild(host);
    const cell = (row: number, column: number) => table.getRow(row).getCell(view.value.columnOrder[column]!).getElement();
    cell(1, 0).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, buttons: 1 }));
    cell(2, 3).dispatchEvent(new MouseEvent("mousemove", { bubbles: true, buttons: 1 }));
    frame.contentDocument!.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    cell(3, 0).dispatchEvent(new MouseEvent("mousemove", { bubbles: true, buttons: 0 }));
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 0, endRow: 2, endColumn: 3 });
    document.body.appendChild(host); frame.remove();
    expect(errors).toEqual([]);
  });

  it("keeps editing when clicking and dragging inside the active cell text", async () => {
    const { table, host, view, source } = await mount();
    const row = table.getRow(1);
    if (!row) throw new Error("Missing row");
    const cell = row.getCell(view.value.columnOrder[1]!);
    if (!cell) throw new Error("Missing cell");
    cell.edit();
    const input = host.querySelector<HTMLElement>(".csv-cell-input")!;
    const cancel = vi.fn(); table.on("cellEditCancelled", cancel);
    for (const type of ["mousedown", "mousemove", "mouseup", "click", "dblclick"]) {
      const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 });
      input.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
      expect(document.activeElement === input, `${type} keeps text focus`).toBe(true);
      expect(host.querySelector(".csv-cell-input") === input, `${type} keeps the editor`).toBe(true);
    }
    expect(input.textContent).toBe("first");
    expect(cancel).not.toHaveBeenCalled();
    input.textContent = "fiXrst";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();
    expect(source.value).toBe("id,name\n001,fiXrst\n002,second\n");
    expect(errors).toEqual([]);
  });
  it("renders merged rectangles, edits the anchor and navigates past covered cells", async () => {
    const { view, grid, host, source, table } = await mount();
    view.value = mergeCsvSelection(view.value, { rows: [1, 2], columns: [0, 1] });
    await settle();
    await grid.value!.applySnapshot({ row: 2, column: 1, scrollTop: 0, scrollLeft: 0 });
    await settle();
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 0, endRow: 2, endColumn: 1 });
    expect(host.querySelector(".csv-merged-cell")?.textContent).toBe("001");
    const row = table.getRow(2);
    expect(row && row.getCell(view.value.columnOrder[1]!).getElement().textContent).toBe("");
    host.firstElementChild!.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true }));
    const input = host.querySelector<HTMLElement>(".csv-merged-cell .csv-cell-input")!;
    expect(input).not.toBeNull(); expect(input.textContent).toBe("001");
    expect(host.querySelector("textarea, input")).toBeNull();
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", keyCode: 37, bubbles: true }));
    await settle();
    expect(document.activeElement === input).toBe(true);
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 0, endRow: 2, endColumn: 1 });
    input.textContent = "merged";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await settle();
    expect(source.value).toBe("id,name\nmerged,first\n002,second\n");
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 2 });
    expect(errors).toEqual([]);
  });
  it("copies only visible merged content and clears/fills anchors while retaining covered source values", async () => {
    const { view, grid, host, source } = await mount();
    view.value = mergeCsvSelection(view.value, { rows: [1, 2], columns: [0, 1] }); await settle();
    await grid.value!.applySnapshot({ row: 1, column: 0, scrollTop: 0, scrollLeft: 0 });
    const copied = vi.fn();
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(copy, "clipboardData", { value: { setData: copied } });
    host.firstElementChild!.dispatchEvent(copy);
    expect(copied).toHaveBeenCalledWith("text/plain", "001\t\n\t");
    grid.value!.clearSelection(); await settle();
    expect(source.value).toBe("id,name\n,first\n002,second\n");
    const paste = (value: string) => {
      const event = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: { getData: () => value } });
      host.firstElementChild!.dispatchEvent(event);
    };
    paste("filled"); await settle();
    expect(source.value).toBe("id,name\nfilled,first\n002,second\n");
    paste("a\tb\nc\td"); await settle();
    expect(source.value).toBe("id,name\nfilled,first\n002,second\n");
    expect(errors).toHaveLength(1); expect(String(errors[0])).toContain("csv.pasteMergedRange"); errors.length = 0;
    view.value = { ...view.value, merges: [] }; await settle();
    expect(host.querySelector(".csv-merged-cell")).toBeNull();
    expect(host.textContent).toContain("second");
  });
  it("edits a hidden source anchor through its visible merged area and clips at frozen columns", async () => {
    const { view, grid, host, source } = await mount();
    view.value = { ...mergeCsvSelection(view.value, { rows: [1, 2], columns: [0, 1] }), frozenColumns: 1 }; await settle();
    expect(host.querySelectorAll(".csv-merged-cell")).toHaveLength(2);
    const id = view.value.columnOrder[0]!;
    view.value = { ...view.value, columns: { ...view.value.columns, [id]: { ...view.value.columns[id]!, hidden: true } } }; await settle();
    await grid.value!.applySnapshot({ row: 1, column: 1, scrollTop: 0, scrollLeft: 0 }); await settle();
    expect(host.querySelector(".csv-merged-cell")?.textContent).toBe("001");
    host.firstElementChild!.dispatchEvent(new KeyboardEvent("keydown", { key: "x", bubbles: true }));
    const input = host.querySelector<HTMLElement>(".csv-merged-cell .csv-cell-input")!;
    input.blur(); await settle();
    expect(source.value).toBe("id,name\nx,first\n002,second\n");
    expect(errors).toEqual([]);
  });
  it("patches one changed row after reparsing 100,000 cells without rebuilding the grid", async () => {
    const text = Array.from({ length: 5000 }, (_, row) => Array.from({ length: 20 }, (_, column) =>
      `${row}-${column}`).join(",")).join("\n");
    const { table, grid, documentRef, view, host } = await mount(text);
    const replace = vi.spyOn(table, "replaceData"), columns = vi.spyOn(table, "setColumns"), update = vi.spyOn(table, "updateData");
    const before = grid.value!.getSnapshot();
    const cells = host.querySelectorAll(".tabulator-cell").length;
    documentRef.value = parseCsvDocument(text.replace("4000-12", "agent-updated"));
    view.value = reconcileCsvView(view.value, documentRef.value);
    await settle();
    expect(replace).not.toHaveBeenCalled(); expect(columns).not.toHaveBeenCalled();
    expect(update).toHaveBeenCalledTimes(1);
    expect(update.mock.calls[0]![0]).toHaveLength(1);
    const row = table.getRow(4000);
    expect(row && row.getData()[view.value.columnOrder[12]!]).toBe("agent-updated");
    expect(grid.value!.getSnapshot()).toEqual(before);
    expect(host.querySelectorAll(".tabulator-cell").length).toBe(cells);
    expect(cells).toBeLessThan(2000); expect(errors).toEqual([]);
  });

  it("does not render again when only CSV quoting and line endings change", async () => {
    const { table, documentRef, view } = await mount();
    const replace = vi.spyOn(table, "replaceData"), update = vi.spyOn(table, "updateData");
    documentRef.value = parseCsvDocument('"id",name\r\n"001",first\r\n002,"second"\r\n');
    view.value = reconcileCsvView(view.value, documentRef.value);
    await settle();
    expect(replace).not.toHaveBeenCalled(); expect(update).not.toHaveBeenCalled(); expect(errors).toEqual([]);
  });

  it("selects a whole column on header click and extends with Shift after reordering", async () => {
    const { table, grid, view, host } = await mount();
    const ids = view.value.columnOrder;
    view.value = { ...view.value, columnOrder: [...ids].reverse() };
    await settle();
    table.getColumn(ids[1]!).getElement().click();
    expect(grid.value!.selectedColumns()).toEqual([1]);
    expect(grid.value!.selectedRows()).toHaveLength(100);
    table.getColumn(ids[0]!).getElement().dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    expect(grid.value!.selectedColumns()).toEqual([1, 0]);
    const copied = vi.fn();
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(copy, "clipboardData", { value: { setData: copied } });
    host.firstElementChild!.dispatchEvent(copy);
    expect(copied.mock.calls[0]![1]).toMatch(/^name\tid\nfirst\t001\nsecond\t002/);
    expect(errors).toEqual([]);
  });

  it("zooms only on Ctrl-wheel, preserves selection and saves resized widths in logical pixels", async () => {
    const { host, table, grid, view, scroller, source } = await mount();
    await grid.value!.applySnapshot({ row: 1, column: 1, endRow: 2, endColumn: 1, scrollTop: 56, scrollLeft: 200 });
    const plain = new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true });
    scroller.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
    expect(grid.value!.getSnapshot().zoom).toBe(1);
    const wheel = new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true });
    scroller.dispatchEvent(wheel);
    await settle(); await settle();
    expect(wheel.defaultPrevented).toBe(true);
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 1, endRow: 2, endColumn: 1, zoom: 1.1 });
    const id = view.value.columnOrder[1]!;
    expect(table.getColumn(id).getWidth()).toBeCloseTo(154);
    expect(view.value.columns[id]!.width).toBe(140);
    expect(source.value).toBe("id,name\n001,first\n002,second\n");
    grid.value!.setZoom(10); await settle();
    expect(grid.value!.getSnapshot().zoom).toBe(2);
    grid.value!.setZoom(0.01); await settle();
    expect(grid.value!.getSnapshot().zoom).toBe(0.5);
    grid.value!.setZoom(1); await settle();
    expect(host.firstElementChild?.getAttribute("style")).toContain("--csv-zoom: 1");
    expect(errors).toEqual([]);
  });

  it("fits columns from offscreen source rows without creating their cells, and enables adaptive row heights", async () => {
    // Simulate text layout only for the sizing probe; keep real virtual rendering.
    const metrics = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("csv-cell-measure")) return (this.textContent?.length ?? 0) * 10 + 15;
      return Number.parseFloat(this.style.width) || 700;
    });
    const { grid, view, table, source } = await mount("name,value\nshort,x\n" + Array.from({ length: 80 }, (_, i) => `r${i},${i === 79 ? "long offscreen content" : "x"}\n`).join(""));
    const id = view.value.columnOrder[1]!;
    const row = table.getRow(81);
    if (!row) throw new Error("Missing offscreen row");
    const getCell = vi.spyOn(row, "getCell");
    grid.value!.autoFitColumns([1]); await settle();
    expect(view.value.columns[id]!.width).toBe(235);
    expect(getCell).not.toHaveBeenCalled();
    grid.value!.autoFitRows(); await settle();
    expect(view.value).toMatchObject({ wrapText: true, rowHeight: 20 });
    expect(source.value).toContain("long offscreen content");
    expect(errors).toEqual([]);
    metrics.mockRestore();
  });

  it("cleans up when the tab closes before Tabulator finishes mounting", async () => {
    const host = document.createElement("div"); document.body.appendChild(host);
    const doc = parseCsvDocument("a,b\n1,2\n");
    const app = createApp({ render: () => h(CsvGrid, { document: doc, view: reconcileCsvView(defaultCsvView(), doc) }) });
    app.mount(host);
    const element = host.firstElementChild as HTMLElement;
    app.unmount();
    await settle();
    expect(element.classList.contains("tabulator")).toBe(false);
    expect(errors).toEqual([]);
  });
  it("rebuilds scrolled frozen columns without combining new columns with old cells", async () => {
    const { grid, table, scroller, view } = await mount();
    await grid.value!.applySnapshot({ row: 1, column: 1, endRow: 2, endColumn: 3, scrollLeft: 1500, scrollTop: 56 });
    scroller.dispatchEvent(new Event("scroll"));
    const column = view.value.columnOrder[0]!;
    for (const change of [
      (v: CsvView) => ({ ...v, frozenColumns: 1 }),
      (v: CsvView) => ({ ...v, wrapText: true }),
      (v: CsvView) => ({ ...v, columns: { ...v.columns, [column]: { ...v.columns[column]!, hidden: true } } }),
      (v: CsvView) => ({ ...v, columnOrder: [...v.columnOrder].reverse() }),
    ]) {
      view.value = change(view.value); await settle();
      expect(errors).toEqual([]);
      expect(table.getDataCount()).toBe(100);
      expect(grid.value!.getSnapshot()).toMatchObject({ row: 1, column: 1, endRow: 2, endColumn: 3, scrollLeft: 1500, scrollTop: 56 });
    }
  });

  it("patches repeated cell edits and skips full redraw on width-only view changes", async () => {
    const { table, edit, view } = await mount();
    const replace = vi.spyOn(table, "replaceData"), redraw = vi.spyOn(table, "redraw"), columns = vi.spyOn(table, "setColumns");
    const patch = vi.spyOn(table, "updateData");
    const bounds = vi.spyOn(table.getRanges()[0]!, "setBounds");
    for (let i = 0; i < 5; i++) { edit([{ row: 1, column: 1, value: `edit ${i}` }]); await settle(); }
    const id = view.value.columnOrder[1]!;
    view.value = { ...view.value, columns: { ...view.value.columns, [id]: { ...view.value.columns[id]!, width: 220 } } };
    await settle();
    expect(patch).toHaveBeenCalledTimes(5);
    expect(replace).not.toHaveBeenCalled(); expect(columns).not.toHaveBeenCalled(); expect(redraw).not.toHaveBeenCalled();
    expect(bounds).not.toHaveBeenCalled();
    const row = table.getRow(1);
    expect(row && row.getData()[id]).toBe("edit 4");
    expect(errors).toEqual([]);
  });

  it("captures a large selection without visiting each selected cell", async () => {
    const { table, grid } = await mount();
    await grid.value!.applySnapshot({ row: 0, column: 0, endRow: 90, endColumn: 20, scrollLeft: 0, scrollTop: 0 });
    const range = table.getRanges()[0]!;
    const cells = vi.spyOn(range, "getStructuredCells");
    const bounds = vi.spyOn(range, "getBounds");
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 0, column: 0, endRow: 90, endColumn: 20 });
    expect(grid.value!.selectedRows()).toHaveLength(91);
    expect(grid.value!.selectedColumns()).toHaveLength(21);
    expect(cells).not.toHaveBeenCalled(); expect(bounds).not.toHaveBeenCalled();
    expect(errors).toEqual([]);
  });

  it("copies, fills and clears a range from CSV data without creating its cells", async () => {
    const { host, table, grid, source } = await mount();
    await grid.value!.applySnapshot({ row: 1, column: 0, endRow: 2, endColumn: 1, scrollLeft: 0, scrollTop: 0 });
    const cells = vi.spyOn(table.getRanges()[0]!, "getStructuredCells");
    const copied = vi.fn();
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(copy, "clipboardData", { value: { setData: copied } });
    host.firstElementChild!.dispatchEvent(copy);
    expect(copied).toHaveBeenCalledWith("text/plain", "001\tfirst\n002\tsecond");
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "filled" } });
    host.firstElementChild!.dispatchEvent(paste); await settle();
    expect(source.value).toBe("id,name\nfilled,filled\nfilled,filled\n");
    grid.value!.clearSelection(); await settle();
    expect(source.value).toBe("id,name\n,\n,\n");
    expect(cells).not.toHaveBeenCalled(); expect(errors).toEqual([]);
  });

  it("bounds scrolling, keyboard navigation and stale snapshots to a finite blank margin", async () => {
    const { grid, table, scroller, source } = await mount();
    const replace = vi.spyOn(table, "replaceData"), columns = vi.spyOn(table, "setColumns");
    for (let i = 0; i < 10; i++) {
      scroller.scrollTop = 2400; scroller.scrollLeft = 3000;
      scroller.dispatchEvent(new Event("scroll"));
    }
    await settle();
    await grid.value!.applySnapshot({ row: 400000, column: 9999, scrollTop: 2400, scrollLeft: 3000 });
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 99, column: 25 });
    for (const key of ["ArrowRight", "ArrowDown", "Tab", "Enter"]) {
      scroller.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })); await settle();
    }
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 99, column: 25 });
    expect(table.getDataCount()).toBe(100); expect(table.getColumns()).toHaveLength(27);
    expect(columns).not.toHaveBeenCalled(); expect(replace).not.toHaveBeenCalled();
    expect(source.value).toBe("id,name\n001,first\n002,second\n");
    expect(errors).toEqual([]);
  });

  it("extends the finite sheet when pasting real values across its boundary", async () => {
    const { grid, host, table, documentRef } = await mount();
    await grid.value!.applySnapshot({ row: 99, column: 25, scrollTop: 0, scrollLeft: 0 });
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "a\tb\nc\td" } });
    host.firstElementChild!.dispatchEvent(paste); await settle();
    expect(documentRef.value.records).toHaveLength(101);
    expect(documentRef.value.columnCount).toBe(27);
    expect(documentRef.value.records[100]!.fields[26]!.value).toBe("d");
    expect(table.getDataCount()).toBe(121);
    expect(table.getColumns()).toHaveLength(36); // 27 data + 8 blank + row header
    expect(errors).toEqual([]);
  });

  it("keeps an in-progress edit through a deferred column rebuild and then navigates", async () => {
    const { table, host, grid, view, source } = await mount();
    const row = table.getRow(1);
    if (!row) throw new Error("Missing row");
    const cell = row.getCell(view.value.columnOrder[1]!);
    if (!cell) throw new Error("Missing cell");
    cell.edit();
    const input = host.querySelector<HTMLElement>(".csv-cell-input")!;
    input.textContent = "continued edit";
    view.value = { ...view.value, wrapText: true };
    await settle();
    expect(host.querySelector(".csv-cell-input")).toBe(input);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();
    expect(source.value).toContain("001,continued edit");
    expect(grid.value!.getSnapshot()).toMatchObject({ row: 2, column: 1 });
    expect(table.getColumn(view.value.columnOrder[1]!).getDefinition().variableHeight).toBe(true);
    expect(errors).toEqual([]);
  });

  it("defers updates while inactive and stops a queued refresh after unmount", async () => {
    const state = await mount();
    const replace = vi.spyOn(state.table, "replaceData");
    state.active.value = false; await settle();
    state.view.value = { ...state.view.value, frozenColumns: 2 };
    state.edit([{ row: 1, column: 1, value: "hidden edit" }]); await settle();
    expect(replace).not.toHaveBeenCalled();
    state.active.value = true; await settle();
    expect(replace).toHaveBeenCalledTimes(1); expect(errors).toEqual([]);
    const pending = state.grid.value!.refresh();
    state.app.unmount(); apps.splice(apps.indexOf(state.app), 1);
    await pending;
    expect(errors).toEqual([]);
  });
});
