// @vitest-environment jsdom
import { undo } from "@codemirror/commands";
import { EditorState, EditorSelection, ChangeSet } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { markdownEditorBaseExtensions, markdownEditorLanguageExtension, markdownEditorModeExtension } from "../components/ui/markdown-editor/codeMirrorMarkdownExtensions";
import { setMarkdownHeading, toggleMarkdownFormat } from "../components/ui/markdown-editor/markdownVisualCommands";
import { markdownLinkTarget, mapMarkdownEditTarget, serializeMarkdownEdit } from "../components/ui/markdown-editor/markdownEditTarget";
import { htmlToEditorMarkdown } from "../components/ui/markdown-editor/markdownRichPaste";
import { editMarkdownTable, parseClipboardTable, pasteMarkdownTable } from "../components/ui/markdown-editor/markdownTableEditing";

let view: EditorView;
beforeEach(() => {
  if (!Range.prototype.getClientRects) Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  if (!Range.prototype.getBoundingClientRect) Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
});
afterEach(() => { view?.destroy(); document.body.replaceChildren(); vi.restoreAllMocks(); });
function mount(doc: string, readonly = false) {
  const parent = document.createElement("div"); document.body.append(parent);
  view = new EditorView({ parent, state: EditorState.create({ doc, extensions: [
    ...markdownEditorBaseExtensions(() => undefined), markdownEditorLanguageExtension("markdown"),
    markdownEditorModeExtension("rendered", "markdown"), EditorState.readOnly.of(readonly),
  ] }) });
  return view;
}
function select(text: string, offset = 0) {
  view.focus(); view.dispatch({ selection: { anchor: view.state.doc.toString().indexOf(text) + offset } });
}
function key(key: string, shiftKey = false) {
  view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }));
}
function paste(html: string, text = "") {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { getData: (type: string) => type === "text/html" ? html : text } });
  view.contentDOM.dispatchEvent(event);
}

describe("persistent visual Markdown editing", () => {
  it("preserves multiple cursors when moving them out of hidden formatting markers", () => {
    mount("**a** and **b**");
    view.dispatch({ selection: EditorSelection.create([EditorSelection.cursor(0), EditorSelection.cursor(10)], 1) });
    expect(view.state.selection.ranges.map((range) => range.head)).toEqual([2, 12]);
    expect(view.state.selection.mainIndex).toBe(1);
  });
  it.each([["bold", "**"], ["italic", "*"], ["strike", "~~"], ["code", "`"]] as const)("toggles %s using document history", (format, marker) => {
    mount("before hello after");
    view.dispatch({ selection: { anchor: 7, head: 12 } });
    toggleMarkdownFormat(view, format);
    expect(view.state.doc.toString()).toBe(`before ${marker}hello${marker} after`);
    expect(view.contentDOM.textContent).toBe("before hello after");
    toggleMarkdownFormat(view, format);
    expect(view.state.doc.toString()).toBe("before hello after");
    expect(undo(view)).toBe(true);
  });

  it("changes heading level without re-creating the editor", () => {
    const editor = mount("## 标题\n\n正文"); const dom = editor.dom;
    select("标题"); setMarkdownHeading(view, 3);
    expect(view.state.doc.toString()).toBe("### 标题\n\n正文");
    expect(view.dom).toBe(dom);
    key("Home");
    expect(view.state.selection.main.head).toBe(4);
    key("Backspace");
    expect(view.state.doc.toString()).toBe("标题\n\n正文");
  });

  it("turns text typed directly after an ATX marker into a heading", () => {
    mount("");
    for (const text of ["#", "#", "1", "1", "2"]) {
      view.dispatch({
        ...view.state.replaceSelection(text),
        userEvent: "input.type",
      });
    }

    expect(view.state.doc.toString()).toBe("## 112");
    expect(view.state.selection.main.head).toBe(6);
    expect(view.dom.querySelector(".cm-live-heading-2")).not.toBeNull();
    expect(view.contentDOM.textContent).toContain("112");
    expect(view.contentDOM.textContent).not.toContain("##");
  });

  it("keeps direct ATX heading input natural during IME composition", () => {
    mount("##");
    view.dispatch({ selection: { anchor: 2 } });
    view.dispatch({
      changes: { from: 2, insert: "标题" },
      selection: { anchor: 4 },
      userEvent: "input.type.compose",
    });

    expect(view.state.doc.toString()).toBe("## 标题");
    expect(view.state.selection.main.head).toBe(5);
    expect(view.dom.querySelector(".cm-live-heading-2")).not.toBeNull();
  });

  it("renders underline-style headings and changes them through the same heading command", () => {
    mount("Title\n=====\n\nbody"); select("Title");
    expect(view.dom.querySelector(".cm-live-heading-1")).not.toBeNull();
    expect(view.contentDOM.textContent).not.toContain("=====");
    setMarkdownHeading(view, 2);
    expect(view.state.doc.toString()).toBe("## Title\n\nbody");
  });

  it("removes formatting cleanly at an inline boundary or when clearing its text", () => {
    mount("plain **bold** end"); select("bold"); key("Backspace");
    expect(view.state.doc.toString()).toBe("plain bold end");
    undo(view);
    view.dispatch({ selection: { anchor: 8, head: 12 } }); key("Delete");
    expect(view.state.doc.toString()).toBe("plain  end");
  });

  it("removes empty formatting when the final visible character is deleted", () => {
    mount("before **字** after"); select("字", 1); key("Backspace");
    expect(view.state.doc.toString()).toBe("before  after");
  });

  it("nests and unnests blockquotes using Tab", () => {
    mount("> quote"); select("quote", 2); key("Tab");
    expect(view.state.doc.toString()).toBe("> > quote");
    key("Tab", true); expect(view.state.doc.toString()).toBe("> quote");
  });

  it("keeps task checkboxes rendered across Enter, indentation and outdent", () => {
    mount("- [x] task"); select("task", 4); key("Enter");
    expect(view.state.doc.toString()).toBe("- [x] task\n- [ ] ");
    expect(view.dom.querySelectorAll(".cm-live-task-checkbox")).toHaveLength(2);
    view.dispatch({ changes: { from: view.state.selection.main.head, insert: "child" } });
    key("Tab"); expect(view.state.doc.toString()).toContain("\n  - [ ] child");
    key("Tab", true); expect(view.state.doc.toString()).toContain("\n- [ ] child");
  });

  it("exits an empty list and continues a blockquote using Enter", () => {
    mount("- first\n- "); select("- ", 2); view.dispatch({ selection: { anchor: view.state.doc.length } }); key("Enter");
    expect(view.state.doc.toString()).not.toContain("\n- ");
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "> quote" }, selection: { anchor: 7 } }); key("Enter");
    expect(view.state.doc.toString()).toContain("> quote\n> ");
  });

  it("edits only a link destination and retains its formatted label and title", () => {
    mount('[**label**](old "title")');
    const target = markdownLinkTarget(view.state, 4)!;
    expect(serializeMarkdownEdit(target, "https://example.com/new", "ignored")).toBe('[**label**](https://example.com/new "title")');
    expect(() => serializeMarkdownEdit(target, "javascript:alert(1)", "")).toThrow();
  });

  it("maps a property editor target past unrelated changes and invalidates overlapping changes", () => {
    mount("before [label](url) after");
    const target = markdownLinkTarget(view.state, 10)!;
    const mapped = mapMarkdownEditTarget(target, ChangeSet.of({ from: 0, insert: "new " }, view.state.doc.length))!;
    expect(mapped.from).toBe(target.from + 4);
    expect(mapped.to).toBe(target.to + 4);
    expect(mapMarkdownEditTarget(target, ChangeSet.of({ from: target.from + 2, insert: "x" }, view.state.doc.length))).toBeNull();
  });

  it("converts rich text and spreadsheet tables to rendered Markdown", () => {
    const html = '<h2>标题</h2><p><strong>重点</strong>与<em>说明</em></p><ul><li>条目</li></ul><table><tr><td>名称</td><td>说明</td></tr><tr><td>Hero</td><td>A<br>B</td></tr></table>';
    const markdown = htmlToEditorMarkdown(html);
    expect(markdown).toContain("## 标题"); expect(markdown).toContain("**重点**"); expect(markdown).toContain("*说明*");
    expect(markdown).toContain("| 名称 | 说明 |"); expect(markdown).toContain("A<br>B");
    mount(""); paste(html);
    expect(view.dom.querySelector(".cm-live-heading-2")).not.toBeNull();
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(2);
    expect(undo(view)).toBe(true); expect(view.state.doc.length).toBe(0);
  });

  it("preserves alignment, pipes and paragraphs when importing HTML tables", () => {
    const markdown = htmlToEditorMarkdown('<table><tr><td align="right">A</td><td>B</td></tr><tr><td>a|b</td><td><p>one</p><p>two</p></td></tr></table>');
    expect(markdown).toContain("| ---: | --- |");
    expect(markdown).toContain("a\\|b"); expect(markdown).toContain("one<br>two");
    mount(markdown); expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(2);
  });

  it("keeps local image paths with spaces resolvable after editing", () => {
    const insert = serializeMarkdownEdit({ kind: "image", from: 0, to: 0, source: "", url: "", label: "" }, "F:/My Images/hero.png", "hero");
    expect(insert).toBe("![hero](<F:/My Images/hero.png>)");
    mount(insert);
    expect(view.dom.querySelector<HTMLElement>(".cm-live-image-frame")?.dataset.source).toBe("F:/My Images/hero.png");
  });

  it("retains embedded images when changing their description", () => {
    const url = "data:image/png;base64,abcd";
    expect(serializeMarkdownEdit({ kind: "image", from: 0, to: 1, source: "", url, label: "old" }, url, "new")).toBe(`![new](${url})`);
  });

  it("updates a JSON Unity property target while retaining other entries and property selectors", () => {
    const json = { properties: [
      { target: { kind: "sceneComponent", scenePath: "Assets/Main.unity", objectPath: "Root/A", objectFileId: 12, componentType: "Player", propertyPath: "speed" } },
      { target: { kind: "asset", path: "Assets/Other.asset", propertyPath: "name" } },
    ] };
    const source = JSON.stringify(json);
    const result = JSON.parse(serializeMarkdownEdit({
      kind: "reference", from: 0, to: source.length, source, label: "speed", url: "Assets/Main.unity/Root/A",
      reference: { from: 0, to: source.length, raw: source, label: "speed", path: "Assets/Main.unity/Root/A", kind: "unity-property" },
    }, "Assets/Main.unity/Root/B", "speed"));
    expect(result.properties[0].target).toEqual({ kind: "sceneComponent", scenePath: "Assets/Main.unity", objectPath: "Root/B", componentType: "Player", propertyPath: "speed" });
    expect(result.properties[1]).toEqual(json.properties[1]);
  });

  it("does not retain active HTML or convert code block paste as prose", () => {
    expect(htmlToEditorMarkdown('<p onclick="alert(1)">ok<script>alert(2)</script><a href="javascript:alert(3)">link</a></p>')).toBe("oklink");
    mount("```html\ncode\n```"); select("code"); paste("<strong>literal</strong>", "<strong>literal</strong>");
    expect(view.state.doc.toString()).toContain("<strong>literal</strong>");
  });
});

describe("visual table operations", () => {
  const source = "intro\n\n| A | B |\n| --- | --- |\n| **one** | two |\n| keep | value |";
  it("inserts/removes rows and columns without losing formatted cells", () => {
    mount(source); select("two"); editMarkdownTable(view, "column-before");
    expect(view.state.doc.toString()).toContain("| **one** |  | two |");
    editMarkdownTable(view, "column-delete"); expect(view.state.doc.toString()).toBe(source);
    editMarkdownTable(view, "row-after"); expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(4);
    editMarkdownTable(view, "row-delete"); expect(view.state.doc.toString()).toBe(source);
    undo(view); expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(4);
  });
  it("sets the current column alignment", () => {
    mount(source); select("two"); editMarkdownTable(view, "center");
    expect(view.state.doc.toString()).toContain("| --- | :---: |");
    expect(view.dom.querySelector('[data-column="1"]')?.getAttribute("data-align")).toBe("center");
  });
  it("pastes a rectangle and expands the table while retaining neighboring cells", () => {
    mount(source); select("two"); pasteMarkdownTable(view, [["x", "y"], ["z", "w"]]);
    expect(view.state.doc.toString()).toContain("| **one** | x | y |\n| keep | z | w |");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
    undo(view); expect(view.state.doc.toString()).toBe(source);
  });

  it("escapes literal backslashes followed by pipes during rectangular paste", () => {
    mount(source); select("two"); pasteMarkdownTable(view, [["a\\|b", "next"]]);
    expect(view.state.doc.toString()).toContain("a\\\\\\|b");
    expect(view.dom.querySelectorAll(".cm-live-table-row")).toHaveLength(3);
  });
  it("parses spreadsheet quoted newlines, tabs and escaped quotes", () => {
    expect(parseClipboardTable('"a\nb"\t"a""b"\r\nx\ty\r\n')).toEqual([["a\nb", 'a"b'], ["x", "y"]]);
  });
  it("pastes HTML cells as a rectangle instead of inserting an entire nested table", () => {
    mount(source); select("two"); paste("<table><tr><td><strong>x</strong></td><td>y</td></tr><tr><td>z</td><td>w</td></tr></table>", "x\ty\nz\tw");
    expect(view.state.doc.toString()).toContain("| **one** | **x** | y |\n| keep | z | w |");
  });
  it("does not mutate a read-only table", () => {
    mount(source, true); view.dispatch({ selection: { anchor: source.indexOf("two") } });
    expect(editMarkdownTable(view, "column-delete")).toBe(false);
    expect(pasteMarkdownTable(view, [["x"]])).toBe(false);
    expect(view.state.doc.toString()).toBe(source);
  });
});
