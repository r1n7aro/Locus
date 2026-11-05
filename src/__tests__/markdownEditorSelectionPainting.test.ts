// @vitest-environment jsdom
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { afterEach, describe, expect, it } from "vitest";
import {
  markdownEditorBaseExtensions,
  markdownEditorLanguageExtension,
  markdownEditorModeExtension,
  markdownEditorReadOnlyExtension,
} from "../components/ui/markdown-editor/codeMirrorMarkdownExtensions";

let view: EditorView | undefined;
afterEach(() => {
  view?.destroy();
  view = undefined;
  document.body.replaceChildren();
});

describe("Markdown native selection painting", () => {
  // jsdom cannot rasterize ::selection. This guards the scoped CSS contract;
  // the full-line -> partial-line transition also needs a Chromium pixel test.
  it.each([
    ["rendered", false],
    ["rendered", true],
    ["native", false],
    ["native", true],
  ] as const)("keeps native selection almost transparent in %s mode (readOnly=%s)", (mode, readOnly) => {
    const parent = document.createElement("div");
    document.body.append(parent);
    view = new EditorView({
      parent,
      state: EditorState.create({
        doc: "普通文字与 **强调文字**，选区回缩不能重复绘制整行。",
        extensions: [
          ...markdownEditorBaseExtensions(() => undefined),
          markdownEditorLanguageExtension("markdown"),
          markdownEditorModeExtension(mode, "markdown"),
          markdownEditorReadOnlyExtension(readOnly),
        ],
      }),
    });

    const expected = document.createElement("span");
    expected.style.backgroundColor = "#00000001";
    const rules = Array.from(document.styleSheets).flatMap(sheet => Array.from(sheet.cssRules))
      .filter((rule): rule is CSSStyleRule => rule instanceof CSSStyleRule);
    const selectionRule = rules.find(rule =>
      rule.selectorText.includes(".cm-content .cm-line::selection")
      && rule.style.backgroundColor === expected.style.backgroundColor);
    expect(selectionRule).toBeDefined();
    expect(selectionRule!.style.getPropertyPriority("background-color")).toBe("important");
    expect(selectionRule!.selectorText).toContain(".cm-content .cm-line ::selection");

    const selectors = selectionRule!.selectorText
      .replaceAll(" ::selection", " *")
      .replaceAll("::selection", "");
    const line = view.contentDOM.querySelector(".cm-line")!;
    expect(line.matches(selectors)).toBe(true);
    const inline = document.createElement("span");
    line.append(inline);
    expect(inline.matches(selectors)).toBe(true);
    const outside = document.createElement("div");
    outside.className = "cm-content";
    outside.innerHTML = '<div class="cm-line">Other editor</div>';
    document.body.append(outside);
    expect(outside.firstElementChild!.matches(selectors)).toBe(false);
  });

  // drawSelection paints behind the opaque table rows, so table cells rely on
  // a visible native selection that must outrank the near-transparent rule.
  it("paints native selection inside rendered table cells", () => {
    const parent = document.createElement("div");
    document.body.append(parent);
    view = new EditorView({
      parent,
      state: EditorState.create({
        doc: "intro\n\n| 位置 | 例子 |\n| --- | --- |\n| 事后 | **知道**结局 |\n\nend",
        extensions: [
          ...markdownEditorBaseExtensions(() => undefined),
          markdownEditorLanguageExtension("markdown"),
          markdownEditorModeExtension("rendered", "markdown"),
        ],
      }),
    });

    const rules = Array.from(document.styleSheets).flatMap(sheet => Array.from(sheet.cssRules))
      .filter((rule): rule is CSSStyleRule => rule instanceof CSSStyleRule);
    const tableRule = rules.find(rule => rule.selectorText.includes(".cm-live-table-cell::selection"));
    const lineRule = rules.find(rule => rule.selectorText.includes(".cm-content .cm-line::selection"));
    expect(tableRule).toBeDefined();
    expect(lineRule).toBeDefined();
    expect(tableRule!.selectorText).toContain(".cm-live-table-cell ::selection");
    // jsdom drops color-mix() declarations, so check the injected source text.
    const css = Array.from(document.querySelectorAll("style")).map(style => style.textContent ?? "").join("\n");
    expect(css).toMatch(/\.cm-live-table-cell ::selection\s*\{\s*background-color:\s*color-mix\(in srgb, var\(--accent-color\) 18%, transparent\) !important;?\s*\}/);

    const classCount = (rule: CSSStyleRule) => Math.min(...rule.selectorText.split(",").map(part => part.match(/\.[\w-]+/g)!.length));
    expect(classCount(tableRule!)).toBeGreaterThan(classCount(lineRule!));

    const selectors = tableRule!.selectorText
      .replaceAll(" ::selection", " *")
      .replaceAll("::selection", "");
    const cells = Array.from(view.contentDOM.querySelectorAll(".cm-live-table-cell"));
    expect(cells.length).toBe(4);
    for (const cell of cells) expect(cell.matches(selectors)).toBe(true);
    const nested = cells.find(cell => cell.textContent?.includes("知道"))!.querySelector("*");
    expect(nested?.matches(selectors)).toBe(true);
    const paragraph = Array.from(view.contentDOM.querySelectorAll(".cm-line")).find(line => line.textContent === "intro")!;
    expect(paragraph.matches(selectors)).toBe(false);
  });
});
