// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import type { CellComponent } from "tabulator-tables";
import { csvCellEditor, csvCellInputValue, focusCsvCellInput } from "../components/csv/csvCellEditor";
import { editCsvCells, parseCsvDocument, serializeCsvDocument } from "../document/csv/csvDocument";
import { csvColumnLabel, projectCsvColumns, projectCsvRowIndices, retainCsvProjectedColumns } from "../document/csv/csvGridProjection";
import { applyCsvNormalizedTextEdit } from "../document/csv/csvSourceEditing";
import { defaultCsvView, reconcileCsvView } from "../document/csv/csvView";

describe("CSV worksheet projection", () => {
  it("shows a blank worksheet without creating CSV records", () => {
    const document = parseCsvDocument("");
    const view = reconcileCsvView(defaultCsvView(), document);
    const columns = projectCsvColumns(document, view);
    expect(columns).toHaveLength(26);
    expect(projectCsvRowIndices(document, view, columns.length)).toHaveLength(100);
    expect([0, 25, 26, 51, 52, 701].map(csvColumnLabel)).toEqual(["A", "Z", "AA", "AZ", "BA", "ZZ"]);
    expect(serializeCsvDocument(document)).toBe("");
    expect(editCsvCells(document, [{ row: 45, column: 12, value: "" }])).toBe("");
  });
  it("materializes only up to an edited blank cell and preserves existing raw data", () => {
    const original = '\ufeffid,name\r\n001,"name"\r\n';
    const text = editCsvCells(parseCsvDocument(original), [{ row: 5, column: 4, value: "新内容" }]);
    expect(text.startsWith(original)).toBe(true);
    const document = parseCsvDocument(text);
    expect(document.records).toHaveLength(6);
    expect(document.records[5]!.fields.map((field) => field.value)).toEqual(["", "", "", "", "新内容"]);
  });
  it("retains a resized blank column when data is entered into that column", () => {
    const document = parseCsvDocument("id,name\n001,test\n");
    const view = reconcileCsvView(defaultCsvView(), document);
    const projected = projectCsvColumns(document, view);
    projected[7]!.width = 220;
    const configured = retainCsvProjectedColumns(view, projected);
    const changed = parseCsvDocument(editCsvCells(document, [{ row: 0, column: 7, value: "note" }]));
    const rebound = reconcileCsvView(configured, changed);
    expect(rebound.columns.blank_7).toMatchObject({ width: 220, sourceIndex: 7, header: "note" });
    expect(rebound.columnOrder[7]).toBe("blank_7");
  });
  it("keeps the first CSV row visible and blank rows after sorted/filtered data", () => {
    const document = parseCsvDocument("id,name\n2,second\n1,first\n3,third\n");
    const view = reconcileCsvView(defaultCsvView(), document);
    const id = view.columnOrder[0]!;
    const sorted = { ...view, sort: [{ columnId: id, direction: "asc" as const }] };
    expect(projectCsvRowIndices(document, sorted, 26).slice(0, 6)).toEqual([0, 2, 1, 3, 4, 5]);
    expect(projectCsvRowIndices(document, { ...sorted, filters: [{ columnId: id, value: "2" }] }, 26).slice(0, 3)).toEqual([0, 1, 4]);
  });
  it("bounds blank projection for tall data", () => {
    const document = parseCsvDocument(Array.from({ length: 30000 }, () => "a,b").join("\n"));
    const columns = projectCsvColumns(document, reconcileCsvView(defaultCsvView(), document));
    expect(columns.length).toBeLessThanOrEqual(16);
    expect(columns.length * projectCsvRowIndices(document, defaultCsvView(), columns.length).length).toBeLessThanOrEqual(500000);
  });
  it("ignores distant blank columns saved by older scrolling behavior", () => {
    const document = parseCsvDocument("id,name\n1,test\n");
    const view = reconcileCsvView(defaultCsvView(), document);
    view.columns.distant = { sourceIndex: 9999, header: "", width: 140 };
    view.columnOrder.push("distant");
    expect(projectCsvColumns(document, view)).toHaveLength(26);
    expect(view.columns.distant.sourceIndex).toBe(9999);
  });
  it("adds only a fixed margin outside actual data", () => {
    const document = parseCsvDocument(Array.from({ length: 150 }, () => Array(30).fill("value").join(",")).join("\n"));
    const columns = projectCsvColumns(document, reconcileCsvView(defaultCsvView(), document));
    expect(columns).toHaveLength(38);
    expect(projectCsvRowIndices(document, defaultCsvView(), columns.length)).toHaveLength(170);
  });
});

function editor(original: string) {
  const element = document.createElement("div"); element.tabIndex = 0; document.body.appendChild(element);
  const text = document.createElement("span"); text.className = "csv-cell-text"; text.textContent = original;
  element.appendChild(text);
  const success = vi.fn(), cancel = vi.fn(), navigate = vi.fn();
  const cell = { getValue: () => original, getElement: () => element } as unknown as CellComponent;
  let rendered = () => {};
  if (typeof csvCellEditor !== "function") throw new Error("Expected editor function");
  const input = csvCellEditor(cell, (callback) => { rendered = callback; }, success, cancel, { newline: "\r\n", navigate }) as HTMLElement;
  element.replaceChildren(input); rendered();
  return { input, text, success, cancel, navigate, element };
}
describe("CSV cell editing", () => {
  it("edits the existing text element with a collapsed caret and no input control", () => {
    const { input, text, element } = editor("之前的文字");
    expect(input).toBe(text);
    expect(input.getAttribute("contenteditable")).toBe("plaintext-only");
    expect(element.querySelector("textarea, input")).toBeNull();
    expect(document.activeElement).toBe(input);
    expect(document.getSelection()?.isCollapsed).toBe(true);
    element.remove();
  });
  it("places the caret at the clicked text position", () => {
    const { input, element } = editor("之前的文字");
    const range = document.createRange(); range.setStart(input.firstChild!, 2); range.collapse(true);
    const hit = vi.fn(() => range);
    Object.defineProperty(document, "caretRangeFromPoint", { configurable: true, value: hit });
    focusCsvCellInput(input, { x: 100, y: 80 });
    expect(hit).toHaveBeenCalledWith(100, 80);
    expect(document.getSelection()?.anchorOffset).toBe(2);
    Reflect.deleteProperty(document, "caretRangeFromPoint"); element.remove();
  });
  it("does not rewrite multiline data merely by opening and leaving the editor", () => {
    const { input, success, cancel, element } = editor("one\r\ntwo");
    input.blur(); expect(cancel).toHaveBeenCalled(); expect(success).not.toHaveBeenCalled(); element.remove();
  });
  it("preserves existing trailing newlines and omits the native caret placeholder", () => {
    const { input, success, cancel, element } = editor("one\r\n\r\n");
    expect(csvCellInputValue(input)).toBe("one\n\n");
    input.blur(); expect(cancel).toHaveBeenCalled(); expect(success).not.toHaveBeenCalled();
    input.replaceChildren(document.createTextNode("changed\n"), document.createTextNode("\n"));
    expect(csvCellInputValue(input)).toBe("changed\n");
    input.textContent = "changed\n\n";
    expect(csvCellInputValue(input)).toBe("changed\n\n");
    element.remove();
  });
  it("commits and navigates on Enter and preserves untouched embedded CRLF", () => {
    const { input, success, navigate, element } = editor("one\r\ntwo");
    input.textContent = "one\ntwo changed";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(success).toHaveBeenCalledWith("one\r\ntwo changed");
    expect(navigate).toHaveBeenCalledWith(expect.anything(), 0, 1); element.remove();
  });
  it("supports Shift+Tab, Escape and Alt+Enter without a second input border", async () => {
    const first = editor("before"); first.input.textContent = "after";
    first.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true }));
    expect(first.navigate).toHaveBeenCalledWith(expect.anything(), -1, 0); first.element.remove();
    const second = editor("before"); second.input.textContent = "discard";
    second.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(second.success).not.toHaveBeenCalled(); expect(second.cancel).toHaveBeenCalled(); await Promise.resolve(); second.element.remove();
    const third = editor("value");
    const insert = vi.fn(); Object.defineProperty(document, "execCommand", { configurable: true, value: insert });
    third.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", altKey: true }));
    expect(insert).toHaveBeenCalledWith("insertText", false, "\n");
    expect(third.success).not.toHaveBeenCalled(); Reflect.deleteProperty(document, "execCommand"); third.element.remove();
  });
  it("waits until IME composition has ended before committing Enter", () => {
    const { input, success, element } = editor("");
    input.dispatchEvent(new CompositionEvent("compositionstart")); input.textContent = "中文";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true }));
    expect(success).not.toHaveBeenCalled();
    input.dispatchEvent(new CompositionEvent("compositionend")); input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(success).toHaveBeenCalledWith("中文"); element.remove();
  });
  it("applies source changes without normalizing other records", () => {
    expect(applyCsvNormalizedTextEdit('id,note\r\n001,"a\r\nb"\r\n', 'id,note\n001,"a\nb changed"\n', "\r\n"))
      .toBe('id,note\r\n001,"a\r\nb changed"\r\n');
  });
  it("commits composed text if focus leaves before compositionend", () => {
    const { input, success, element } = editor("");
    input.dispatchEvent(new CompositionEvent("compositionstart"));
    input.textContent = "中文输入";
    input.blur();
    expect(success).not.toHaveBeenCalled();
    input.dispatchEvent(new CompositionEvent("compositionend"));
    expect(success).toHaveBeenCalledExactlyOnceWith("中文输入");
    element.remove();
  });
});
