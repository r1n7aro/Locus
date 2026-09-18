// @vitest-environment jsdom
import { redo, undo } from "@codemirror/commands";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { editMarkdownTable } from "../components/ui/markdown-editor/markdownTableEditing";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  markdownEditorBaseExtensions, markdownEditorLanguageExtension, markdownEditorModeExtension,
} from "../components/ui/markdown-editor/codeMirrorMarkdownExtensions";

let view: EditorView;
let mode: Compartment;
const source = "intro\n\n| Name | Notes |\n| :--- | ---: |\n| Hero | old |\n| Other | keep |\n\nend";

beforeEach(() => {
  for (const method of ["getClientRects", "getBoundingClientRect"]) {
    if (!(method in Range.prototype)) Object.defineProperty(Range.prototype, method, {
      configurable: true, value: () => method === "getClientRects" ? [] : { left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 },
    });
  }
});
afterEach(() => {
  view?.destroy();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function mount(doc = source, readOnly = false) {
  mode = new Compartment();
  const parent = document.createElement("div");
  document.body.append(parent);
  view = new EditorView({ parent, state: EditorState.create({ doc, extensions: [
    ...markdownEditorBaseExtensions(() => undefined), markdownEditorLanguageExtension("markdown"),
    mode.of(markdownEditorModeExtension("rendered", "markdown")), EditorState.readOnly.of(readOnly),
    EditorView.editable.of(!readOnly),
  ] }) });
  return view;
}

function select(text: string, offset = 0) {
  view.focus();
  view.dispatch({ selection: { anchor: view.state.doc.toString().indexOf(text) + offset } });
}

function key(key: string, shiftKey = false) {
  const event = new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true });
  view.contentDOM.dispatchEvent(event);
  return event;
}

function paste(text: string) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { getData: (type: string) => type === "text/plain" ? text : "" } });
  view.contentDOM.dispatchEvent(event);
}

describe("Markdown table cell editing", () => {
  it("inserts above a header while preserving all existing cells and alignment", () => {
    mount(); select("Name");
    expect(editMarkdownTable(view, "row-before")).toBe(true);
    expect(view.state.doc.toString()).toBe(source.replace("| Name | Notes |\n| :--- | ---: |", "|  |  |\n| :--- | ---: |\n| Name | Notes |"));
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(4);
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(source);
  });

  it("promotes the next row after deleting the header without dropping its formatting", () => {
    const doc = source.replace("Hero", "**Hero**").replace("old", "a\\|b<br>`code`");
    mount(doc); select("Name");
    editMarkdownTable(view, "row-delete");
    expect(view.state.doc.toString()).toBe("intro\n\n| **Hero** | a\\|b<br>`code` |\n| :--- | ---: |\n| Other | keep |\n\nend");
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(doc);
  });

  it.each(["row-delete", "column-delete", "table-delete"] as const)("removes a final single-cell table through %s and can restore it", (action) => {
    const doc = "| Header |\n| --- |";
    mount(doc); select("Header");
    expect(editMarkdownTable(view, action)).toBe(true);
    expect(view.state.doc.length).toBe(0);
    expect(view.state.selection.main.head).toBe(0);
    expect(view.dom.querySelector(".cm-live-table-row")).toBeNull();
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(doc);
    expect(redo(view)).toBe(true);
    expect(view.state.doc.length).toBe(0);
  });

  it("undoes table deletion independently of the preceding text edit", () => {
    mount(); select("old", 3);
    view.dispatch({ ...view.state.replaceSelection(" changed"), userEvent: "input.type" });
    editMarkdownTable(view, "table-delete");
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(source.replace("old", "old changed"));
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(source);
  });

  it("refuses to delete tables in a read-only state", () => {
    mount(source, true); select("old");
    expect(editMarkdownTable(view, "table-delete")).toBe(false);
    expect(view.state.doc.toString()).toBe(source);
  });

  it("keeps native editable text and inline formatting inside stable cell wrappers", () => {
    mount(source.replace("Hero", "**Hero**").replace("old", "[link](https://example.com) and `code`"));
    const row = view.dom.querySelectorAll(".cm-live-table-row")[1];
    expect(row.querySelectorAll(".cm-live-table-cell")).toHaveLength(2);
    expect(row.querySelector(".cm-live-table-cell .cm-live-strong")).not.toBeNull();
    const cell = row.querySelector(".cm-live-table-cell")!;
    expect(cell.closest("[contenteditable]")).toBe(view.contentDOM);
    select("Hero", 2);
    view.dispatch({ changes: { from: view.state.selection.main.head, insert: "中文" }, userEvent: "input" });
    expect(view.state.doc.toString()).toContain("**He中文ro**");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
    expect(view.contentDOM.textContent).not.toContain("| :--- | ---: |");
  });

  it("uses the document undo history and retains it through source/live mode changes", () => {
    mount();
    select("old", 3);
    view.dispatch({ changes: { from: view.state.selection.main.head, insert: " changed" }, userEvent: "input" });
    view.dispatch({ effects: mode.reconfigure(markdownEditorModeExtension("native", "markdown")) });
    expect(view.dom.querySelector(".cm-live-table-row")).toBeNull();
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(source);
    view.dispatch({ effects: mode.reconfigure(markdownEditorModeExtension("rendered", "markdown")) });
    expect(redo(view)).toBe(true);
    expect(view.state.doc.toString()).toContain("| Hero | old changed |");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
  });

  it("navigates with Tab, Shift-Tab and Enter without exposing delimiters", () => {
    mount();
    select("Name");
    key("Tab");
    expect(view.state.selection.main.head).toBe(source.indexOf("Notes"));
    key("Tab");
    expect(view.state.selection.main.head).toBe(source.indexOf("Hero"));
    key("Tab", true);
    expect(view.state.selection.main.head).toBe(source.indexOf("Notes"));
    key("Enter");
    expect(view.state.selection.main.head).toBe(source.indexOf("old"));
    expect(view.state.doc.toString()).toBe(source);
  });

  it("adds an undoable row from the final cell and lets an empty cell receive text", () => {
    mount();
    select("keep");
    key("Tab");
    const anchor = view.state.selection.main.head;
    view.dispatch({ changes: { from: anchor, insert: "新行" }, userEvent: "input" });
    expect(view.state.doc.toString()).toContain("| Other | keep |\n| 新行 |  |\n\nend");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(4);
    expect(undo(view)).toBe(true);
  });

  it("materializes a missing trailing cell without changing other rows", () => {
    mount(source.replace("| Hero | old |", "| Hero |"));
    select("Hero");
    key("Tab");
    const anchor = view.state.selection.main.head;
    view.dispatch({ changes: { from: anchor, insert: "补齐" }, userEvent: "input" });
    expect(view.state.doc.toString()).toBe(source.replace("old", "补齐"));
  });

  it.each(["|  | old |", "||old|"])("edits a blank cell in %s", (row) => {
    mount(source.replace("| Hero | old |", row));
    const cell = view.dom.querySelectorAll(".cm-live-table-row")[1].querySelector(".cm-live-table-empty-cell")!;
    if (row === "||old|") cell.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    else {
      expect(cell.closest("[contenteditable]")).toBe(view.contentDOM);
      select("|  | old |", 2);
    }
    const anchor = view.state.selection.main.head;
    view.dispatch({ changes: { from: anchor, insert: "值" }, userEvent: "input" });
    expect(view.dom.querySelectorAll(".cm-live-table-row")[1].querySelector(".cm-live-table-cell")?.textContent).toBe("值");
    expect(view.state.doc.toString()).toContain("old");
  });

  it("protects table boundaries from Backspace, Delete, Home and End", () => {
    mount();
    select("Hero");
    expect(key("Backspace").defaultPrevented).toBe(true);
    expect(view.state.doc.toString()).toBe(source);
    key("End");
    expect(view.state.selection.main.head).toBe(source.indexOf("Hero") + 4);
    expect(key("Delete").defaultPrevented).toBe(true);
    key("Home");
    expect(view.state.selection.main.head).toBe(source.indexOf("Hero"));
    key("ArrowLeft");
    expect(view.state.selection.main.head).toBe(source.indexOf("Notes") + 5);
    key("ArrowRight");
    expect(view.state.selection.main.head).toBe(source.indexOf("Hero"));
    expect(view.state.doc.toString()).toBe(source);
  });

  it("moves vertically within the same column when layout measurements are unavailable", () => {
    mount();
    select("old");
    key("ArrowUp");
    expect(view.state.selection.main.head).toBe(source.indexOf("Notes") + 5);
    key("ArrowDown");
    expect(view.state.selection.main.head).toBe(source.indexOf("old"));
    key("ArrowDown");
    expect(view.state.selection.main.head).toBe(source.indexOf("keep"));
  });

  it("can leave a table at the end of a document to continue writing", () => {
    mount(source.slice(0, -5));
    select("keep", 4);
    key("ArrowRight");
    expect(view.state.doc.toString()).toBe(source.slice(0, -5) + "\n\n");
    expect(view.state.selection.main.head).toBe(view.state.doc.length);
  });

  it("pastes pipes and newlines inside the cell without changing columns or rows", () => {
    mount();
    select("old", 3);
    paste("甲|乙\n下一行");
    expect(view.state.doc.toString()).toContain("| Hero | old甲\\|乙<br>下一行 |");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
    const cell = view.dom.querySelectorAll(".cm-live-table-row")[1].querySelectorAll(".cm-live-table-cell")[1];
    expect(cell.textContent).toContain("old甲|乙");
    expect(cell.querySelector("br")).not.toBeNull();
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(source);
  });

  it("keeps pipe escaping valid after an existing backslash", () => {
    mount(source.replace("old", "old\\"));
    select("old\\", 4);
    paste("|");
    expect(view.state.doc.toString()).toContain("old\\\\\\|");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
  });

  it("inserts and deletes a cell line break as one Markdown token", () => {
    mount();
    select("old", 3);
    key("Enter", true);
    expect(view.state.doc.toString()).toContain("old<br>");
    key("Backspace");
    expect(view.state.doc.toString()).toBe(source);
  });

  it("preserves inline code containing a literal br tag", () => {
    mount(source.replace("old", "`<br>`"));
    const cell = view.dom.querySelectorAll(".cm-live-table-row")[1].querySelectorAll(".cm-live-table-cell")[1];
    expect(cell.textContent).toBe("<br>");
    expect(cell.querySelector("br")).toBeNull();
  });

  it("maps cell edits after external changes ahead of the table", () => {
    mount();
    view.dispatch({ changes: { from: 0, insert: "external\n" }, annotations: [Transaction.remote.of(true), Transaction.addToHistory.of(false)] });
    select("old", 3);
    paste("|new");
    expect(view.state.doc.toString()).toBe(`external\n${source.replace("old", "old\\|new")}`);
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe(`external\n${source}`);
  });

  it("keeps read-only cells unchanged", () => {
    mount(source.replace("| Hero | old |", "||old|"), true);
    const empty = view.dom.querySelector(".cm-live-table-empty-cell")!;
    empty.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    view.dispatch({ selection: { anchor: view.state.doc.toString().indexOf("old") } });
    key("Tab");
    paste("|new\nline");
    expect(view.state.doc.toString()).toBe(source.replace("| Hero | old |", "||old|"));
  });
});
