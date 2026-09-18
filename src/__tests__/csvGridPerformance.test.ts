// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createApp, h, nextTick, shallowRef, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import CsvGrid from "../components/csv/CsvGrid.vue";
import { applyCsvCellEdits, parseCsvDocument } from "../document/csv/csvDocument";
import { defaultCsvView, reconcileCsvView } from "../document/csv/csvView";

const calls = vi.hoisted(() => ({ replaceData: [] as unknown[][], updateData: [] as unknown[][] }));

vi.mock("tabulator-tables", () => {
  class FakeTabulator {
    private data: Array<Record<string, unknown>> = [];

    constructor(element: HTMLElement) { element.classList.add("tabulator"); }
    on(name: string, callback: (...args: unknown[]) => void) {
      if (name === "tableBuilt") queueMicrotask(callback);
    }
    getRanges() { return []; }
    setColumns() {}
    getColumn() { return false; }
    replaceData(data: Array<Record<string, unknown>>) {
      this.data = data;
      calls.replaceData.push(data);
      return Promise.resolve();
    }
    updateData(data: Array<Record<string, unknown>>) {
      calls.updateData.push(data);
      for (const patch of data) {
        const row = this.data.find((entry) => entry._row === patch._row);
        if (row) Object.assign(row, patch);
      }
      return Promise.resolve();
    }
    getRow(index: number) { return this.data.some((row) => row._row === index) ? {} : false; }
    redraw() {}
    destroy() {}
  }
  return { TabulatorFull: FakeTabulator };
});

const apps: App[] = [];
async function flush(): Promise<void> {
  for (let index = 0; index < 12; index++) { await Promise.resolve(); await nextTick(); }
}

afterEach(() => {
  for (const app of apps.splice(0)) app.unmount();
  calls.replaceData.length = 0;
  calls.updateData.length = 0;
  document.body.innerHTML = "";
});

describe("CSV grid incremental rendering", () => {
  it("updates only the edited row instead of replacing the worksheet", async () => {
    const source = "id,name\n001,first\n002,second\n";
    const documentRef = shallowRef(parseCsvDocument(source));
    const viewRef = shallowRef(reconcileCsvView(defaultCsvView(), documentRef.value));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({ setup: () => () => h(CsvGrid, {
      document: documentRef.value, view: viewRef.value,
    }) });
    apps.push(app);
    app.mount(host);
    await flush();
    expect(calls.replaceData).toHaveLength(1);

    const result = applyCsvCellEdits(documentRef.value, source, [{ row: 1, column: 1, value: "changed" }]);
    documentRef.value = result.document;
    viewRef.value = reconcileCsvView(viewRef.value, result.document);
    await flush();

    expect(calls.replaceData).toHaveLength(1);
    expect(calls.updateData).toHaveLength(1);
    expect(calls.updateData[0]).toEqual([expect.objectContaining({ _row: 1, [viewRef.value.columnOrder[1]!]: "changed" })]);
  });

  it("keeps selected-cell dividers visible with existing theme tokens", () => {
    const source = readFileSync(resolve(process.cwd(), "src/components/csv/CsvGrid.vue"), "utf8");
    expect(source).toContain(".tabulator-cell.tabulator-range-selected:not(.tabulator-row-header)");
    expect(source).toContain("color-mix(in srgb, var(--accent-color) 38%, var(--border-strong) 62%)");
  });
});
