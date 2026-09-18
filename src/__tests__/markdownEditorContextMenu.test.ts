// @vitest-environment jsdom
import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseMarkdownEditor from "../components/ui/BaseMarkdownEditor.vue";
import type { MarkdownEditorSelection } from "../components/ui/markdown-editor/markdownEditorSelection";

vi.mock("../i18n", () => ({ t: (key: string) => key }));
const notice = vi.hoisted(() => vi.fn());
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: notice }) }));

let app: App | null = null;
const copy = vi.fn();
const paste = vi.fn();
beforeEach(() => {
  copy.mockReset().mockResolvedValue(undefined);
  paste.mockReset().mockResolvedValue("pasted\r\ntext");
  notice.mockClear();
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: copy, readText: paste } });
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
});
afterEach(() => { app?.unmount(); app = null; document.body.replaceChildren(); });

async function mount(disabled = false, transactionModel = false, initialContent = "# Boss\nalpha\nbeta\ngamma") {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const editor = ref<{ getEditorView(): EditorView } | null>(null);
  const contentKey = ref("one");
  const model = ref(initialContent);
  const viewMode = ref<"rendered" | "native">("rendered");
  const save = vi.fn();
  const changes = vi.fn();
  const quote = vi.fn<(selection: MarkdownEditorSelection) => void>();
  app = createApp({ setup: () => () => h(BaseMarkdownEditor, {
    ref: editor, modelValue: model.value, contentKey: contentKey.value,
    disabled, transactionModel, canQuoteSelection: true, viewMode: viewMode.value,
    "onUpdate:modelValue": (value: string) => { model.value = value; },
    onDocumentChange: changes, onQuoteSelection: quote, onShortcutSave: save,
  }) });
  app.mount(root);
  await nextTick();
  const view = editor.value!.getEditorView();
  return { view, contentKey, model, quote, changes, viewMode, save };
}
async function open(view: EditorView, target: Element = view.contentDOM) {
  target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 }));
  await nextTick();
  await nextTick();
}
function button(action: string) {
  return document.querySelector<HTMLButtonElement>(`[data-editor-action="${action}"]`)!;
}
async function click(action: string) {
  button(action).click();
  await Promise.resolve();
  await nextTick();
}

describe("Markdown editor context menu", () => {
  it("retains reversed multiline selection while focusing the menu and emits its immutable snapshot", async () => {
    const { view, quote } = await mount();
    view.dispatch({ selection: EditorSelection.single(18, 7) });
    await open(view);
    expect(document.activeElement?.textContent).toBe("editor.quoteSelection");
    document.querySelector<HTMLButtonElement>(".markdown-editor-context-menu button")!.click();
    expect(quote).toHaveBeenCalledTimes(1);
    expect(quote.mock.calls[0]![0].ranges[0]).toMatchObject({ text: "alpha\nbeta\n", startLine: 2, endLine: 3 });
    expect(view.state.selection.main.from).toBe(7);
  });

  it("cuts through CodeMirror transactions, supports undo/redo, and pastes at the captured selection", async () => {
    const { view, changes } = await mount(false, true);
    view.dispatch({ selection: { anchor: 7, head: 12 } });
    await open(view);
    await click("cut");
    expect(copy).toHaveBeenCalledWith("alpha");
    expect(view.state.doc.toString()).toBe("# Boss\n\nbeta\ngamma");
    expect(changes).toHaveBeenCalledTimes(1);
    await open(view); await click("undo");
    expect(view.state.doc.toString()).toContain("alpha");
    await open(view); await click("redo");
    expect(view.state.doc.toString()).not.toContain("alpha");
    await open(view); await click("paste");
    expect(view.state.doc.toString()).toContain("pasted\ntext");
  });

  it("does not delete text when clipboard write fails", async () => {
    const { view } = await mount();
    copy.mockRejectedValueOnce(new Error("denied"));
    view.dispatch({ selection: { anchor: 7, head: 12 } });
    await open(view); await click("cut");
    expect(view.state.doc.toString()).toContain("alpha");
    expect(notice).toHaveBeenCalledWith("error", expect.stringContaining("denied"));
  });

  it("keeps multiple editor selections instead of replacing them with the browser's main range", async () => {
    const { view, quote } = await mount();
    view.focus();
    view.dispatch({ selection: EditorSelection.create([
      EditorSelection.range(7, 12), EditorSelection.range(18, 23),
    ]) });
    await open(view);
    document.querySelector<HTMLButtonElement>(".markdown-editor-context-menu button")!.click();
    expect(quote.mock.calls[0]![0].ranges.map((range) => range.text)).toEqual(["alpha", "gamma"]);
  });

  it("does not paste into a new document after asynchronous clipboard access", async () => {
    const { view, contentKey, model } = await mount();
    let resolvePaste!: (text: string) => void;
    paste.mockImplementationOnce(() => new Promise<string>((resolve) => { resolvePaste = resolve; }));
    await open(view); await click("paste");
    contentKey.value = "two"; model.value = "different document";
    await nextTick();
    resolvePaste("wrong text");
    await Promise.resolve(); await nextTick();
    expect(view.state.doc.toString()).toBe("different document");
  });

  it("passes rich clipboard data through the editor's existing paste pipeline", async () => {
    const { view } = await mount();
    Object.defineProperty(navigator.clipboard, "read", { configurable: true, value: vi.fn(async () => [{
      types: ["text/plain", "text/html"],
      getType: async (type: string) => ({ text: async () => type === "text/html" ? "<strong>bold</strong>" : "bold" }),
    }]) });
    // jsdom has no clipboard constructors; provide the same event data contract.
    vi.stubGlobal("DataTransfer", class {
      values = new Map<string, string>();
      setData(type: string, value: string) { this.values.set(type, value); }
      getData(type: string) { return this.values.get(type) ?? ""; }
    });
    vi.stubGlobal("ClipboardEvent", class extends Event {
      clipboardData: DataTransfer;
      constructor(type: string, options: ClipboardEventInit) { super(type, options); this.clipboardData = options.clipboardData!; }
    });
    try {
      view.dispatch({ selection: { anchor: 7, head: 12 } });
      await open(view); await click("paste");
      for (let i = 0; i < 8; i++) await nextTick();
      expect(view.state.doc.toString()).toContain("**bold**");
    } finally { vi.unstubAllGlobals(); }
  });

  it("offers copy and quoting in read-only documents while disabling mutations", async () => {
    const { view } = await mount(true);
    const text = view.contentDOM.querySelectorAll(".cm-line")[1]!.firstChild!;
    const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, 5);
    document.getSelection()!.removeAllRanges(); document.getSelection()!.addRange(range);
    await open(view);
    expect(button("cut").disabled).toBe(true);
    expect(button("paste").disabled).toBe(true);
    expect(button("delete").disabled).toBe(true);
    expect(button("copy").disabled).toBe(false);
    await click("copy");
    expect(copy).toHaveBeenCalledWith("alpha");
    await open(view); await click("selectAll");
    await open(view); await click("copy");
    expect(copy).toHaveBeenLastCalledWith(view.state.doc.toString());
  });

  it("supports keyboard menu navigation, select all, delete and Escape", async () => {
    const { view } = await mount();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true }));
    await nextTick(); await nextTick();
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(document.activeElement).toBe(button("selectAll"));
    await click("selectAll");
    expect(view.state.selection.main.to).toBe(view.state.doc.length);
    await open(view); await click("delete");
    expect(view.state.doc.length).toBe(0);
    await open(view);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    expect(document.querySelector(".markdown-editor-context-menu")).toBeNull();
  });
});

const table = "| Name | Notes |\n| :--- | ---: |\n| Hero | old |\n| Other | keep |";
const tableDocument = `before\n\n${table}\n\nafter`;
function tableCell(view: EditorView, row: number, column: number) {
  return view.contentDOM.querySelectorAll(".cm-live-table-row")[row]!.querySelectorAll(".cm-live-table-cell")[column]!;
}
function tableButton(action: string) {
  return document.querySelector<HTMLButtonElement>(`[data-table-action="${action}"]`)!;
}

describe("Markdown table context menu", () => {
  it.each([
    ["row-before", table.replace("| Hero", "|  |  |\n| Hero")],
    ["row-after", table.replace("| Other", "|  |  |\n| Other")],
    ["row-delete", table.replace("| Hero | old |\n", "")],
    ["column-before", "| Name |  | Notes |\n| :--- | --- | ---: |\n| Hero |  | old |\n| Other |  | keep |"],
    ["column-after", "| Name | Notes |  |\n| :--- | ---: | --- |\n| Hero | old |  |\n| Other | keep |  |"],
    ["column-delete", "| Name |\n| :--- |\n| Hero |\n| Other |"],
    ["table-delete", ""],
  ])("runs %s on the clicked cell, saves the Markdown, and supports undo/redo", async (action, expectedTable) => {
    const { view, model, changes, viewMode, save } = await mount(false, true, tableDocument);
    view.focus();
    view.dispatch({ selection: { anchor: 0 } });
    await open(view, tableCell(view, 1, 1));
    expect(document.querySelectorAll("[data-table-action]")).toHaveLength(7);
    expect(view.state.selection.main.head).toBe(0);
    tableButton(action!).click();
    await nextTick();
    const expected = `before\n\n${expectedTable}\n\nafter`;
    expect(view.state.doc.toString()).toBe(expected);
    expect(changes).toHaveBeenCalledTimes(1);
    expect(view.hasFocus).toBe(true);
    expect(document.querySelector(".markdown-editor-context-menu")).toBeNull();
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true, cancelable: true }));
    await nextTick();
    expect(save).toHaveBeenCalledTimes(1);
    expect(model.value).toBe(expected);
    viewMode.value = "native"; await nextTick();
    expect(view.contentDOM.querySelector(".cm-live-table-row")).toBeNull();
    viewMode.value = "rendered"; await nextTick();
    expect(!!view.contentDOM.querySelector(".cm-live-table-row")).toBe(!!expectedTable);
    await open(view); await click("undo");
    expect(view.state.doc.toString()).toBe(tableDocument);
    await open(view); await click("redo");
    expect(view.state.doc.toString()).toBe(expected);
  });

  it("targets the right-clicked table even when the caret is in another table", async () => {
    const doc = `${table}\n\n${table.replace("Hero", "Second")}\n\nend`;
    const { view, model } = await mount(false, false, doc);
    view.dispatch({ selection: { anchor: doc.indexOf("Hero") } });
    await open(view, tableCell(view, 4, 1));
    tableButton("table-delete").click(); await nextTick();
    expect(model.value).toBe(`${table}\n\n\n\nend`);
    expect(view.contentDOM.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
  });

  it.each(["| Hero |", "|Hero||"])("targets an empty cell in %s without editing it when opening the menu", async (row) => {
    const doc = tableDocument.replace("| Hero | old |", row);
    const { view, model } = await mount(false, false, doc);
    const cell = tableCell(view, 1, 1);
    cell.dispatchEvent(new MouseEvent("mousedown", { button: 2, buttons: 2, bubbles: true, cancelable: true }));
    await open(view, cell);
    expect(model.value).toBe(doc);
    expect(view.state.doc.toString()).toBe(doc);
    tableButton("column-delete").click(); await nextTick();
    expect(model.value).toBe("before\n\n| Name |\n| :--- |\n| Hero |\n| Other |\n\nafter");
  });

  it("resolves the cell when right-clicking inline links", async () => {
    const doc = tableDocument.replace("old", "[link](https://example.com)");
    const { view } = await mount(false, false, doc);
    await open(view, tableCell(view, 1, 1).querySelector(".cm-live-link")!);
    tableButton("row-delete").click(); await nextTick();
    expect(view.state.doc.toString()).toBe(tableDocument.replace("| Hero | old |\n", ""));
  });

  it("offers keyboard access for the active cell and restores focus on Escape", async () => {
    const { view } = await mount(false, false, tableDocument);
    view.dispatch({ selection: { anchor: tableDocument.indexOf("old") } });
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true }));
    await nextTick(); await nextTick();
    expect(document.activeElement).toBe(tableButton("row-before"));
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(tableButton("row-after"));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await nextTick();
    expect(document.querySelector(".markdown-editor-context-menu")).toBeNull();
    expect(view.hasFocus).toBe(true);
    expect(view.state.doc.toString()).toBe(tableDocument);
  });

  it("hides table actions outside cells even when the caret remains inside the table", async () => {
    const { view } = await mount(false, false, tableDocument);
    view.dispatch({ selection: { anchor: tableDocument.indexOf("old") } });
    await open(view, view.contentDOM.querySelector(".cm-line")!);
    expect(tableButton("table-delete")).toBeNull();
  });

  it("hides table mutations in read-only documents", async () => {
    const { view } = await mount(true, false, tableDocument);
    await open(view, tableCell(view, 1, 1));
    expect(tableButton("table-delete")).toBeNull();
    expect(view.state.doc.toString()).toBe(tableDocument);
  });

  it("does not apply an old menu action after switching documents", async () => {
    const { view, model, contentKey } = await mount(false, false, tableDocument);
    await open(view, tableCell(view, 1, 1));
    const remove = tableButton("table-delete");
    contentKey.value = "two"; model.value = table;
    await nextTick();
    remove.click(); await nextTick();
    expect(view.state.doc.toString()).toBe(table);
  });
});
