// @vitest-environment jsdom
import { markdown } from "@codemirror/lang-markdown";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { GFM } from "@lezer/markdown";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { markdownLivePreview } from "../components/ui/markdown-editor/markdownLivePreview";

let view: EditorView | null = null;

function mountLivePreview(doc: string, anchor = 0): EditorView {
  const parent = document.createElement("div");
  document.body.appendChild(parent);
  view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      selection: { anchor },
      extensions: [markdown({ extensions: GFM }), markdownLivePreview()],
    }),
  });
  return view;
}

beforeEach(() => {
  if (!(Range.prototype as Range & { getClientRects?: unknown }).getClientRects) {
    Object.defineProperty(Range.prototype, "getClientRects", {
      configurable: true,
      value: () => [],
    });
  }
  if (!(Range.prototype as Range & { getBoundingClientRect?: unknown }).getBoundingClientRect) {
    Object.defineProperty(Range.prototype, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
        width: 0,
        height: 0,
      }),
    });
  }
});

afterEach(() => {
  view?.destroy();
  view = null;
  document.body.replaceChildren();
});

describe("markdown Live Preview", () => {
  it("decorates visible Markdown while retaining one EditorView DOM", () => {
    const mode = new Compartment();
    const parent = document.createElement("div");
    document.body.appendChild(parent);
    const state = EditorState.create({
      doc: [
        "intro",
        "",
        "# Heading",
        "",
        "**bold** and *italic* with ~~old~~ `code` and [docs](https://example.com)",
        "",
        "- [x] done",
        "1. ordered",
        "",
        "> quote",
        "",
        "---",
        "",
        "```ts",
        "const value = 1",
        "```",
      ].join("\n"),
      selection: { anchor: 0 },
      extensions: [
        markdown({ extensions: GFM }),
        mode.of(markdownLivePreview()),
      ],
    });

    view = new EditorView({ state, parent });
    const editorDom = view.dom;

    expect(editorDom.classList.contains("cm-live-preview")).toBe(true);
    expect(parent.querySelector(".cm-live-heading-1")).not.toBeNull();
    expect(parent.querySelector(".cm-live-strong")).not.toBeNull();
    expect(parent.querySelector(".cm-live-emphasis")).not.toBeNull();
    expect(parent.querySelector(".cm-live-strikethrough")).not.toBeNull();
    expect(parent.querySelector(".cm-live-inline-code")).not.toBeNull();
    expect(parent.querySelector(".cm-live-link")).not.toBeNull();
    expect(parent.querySelector(".cm-live-list-marker")).not.toBeNull();
    expect(parent.querySelector(".cm-live-task-checkbox")).not.toBeNull();
    expect(parent.querySelector(".cm-live-blockquote")).not.toBeNull();
    expect(parent.querySelector(".cm-live-horizontal-rule")).not.toBeNull();
    expect(parent.querySelector(".cm-live-fenced-code")).not.toBeNull();

    const checkbox = parent.querySelector<HTMLInputElement>(".cm-live-task-checkbox");
    checkbox?.dispatchEvent(new Event("change"));
    expect(view.state.doc.toString()).toContain("- [ ] done");

    view.dispatch({
      effects: mode.reconfigure(EditorView.editorAttributes.of({ class: "cm-source-mode" })),
      annotations: Transaction.addToHistory.of(false),
    });

    expect(view.dom).toBe(editorDom);
    expect(editorDom.classList.contains("cm-source-mode")).toBe(true);
    expect(parent.querySelectorAll(".cm-editor")).toHaveLength(1);
  });

  it("keeps inline formatting rendered while editing its text", () => {
    const editor = mountLivePreview("plain **bold** and *italic*");

    expect(editor.contentDOM.textContent).toContain("plain bold and italic");
    expect(editor.contentDOM.textContent).not.toContain("**bold**");

    editor.focus();
    editor.dispatch({ selection: { anchor: 9 } });
    expect(editor.contentDOM.textContent).toContain("plain bold and italic");
    expect(editor.contentDOM.textContent).not.toContain("**bold**");

    editor.dispatch({ selection: { anchor: 8, head: 12 } });
    expect(editor.contentDOM.textContent).not.toContain("**bold**");

    editor.dispatch({ selection: { anchor: 7 } });
    expect(editor.contentDOM.textContent).toContain("**bold");
    expect(editor.contentDOM.textContent).not.toContain("**bold**");
    expect(editor.contentDOM.textContent).not.toContain("*italic*");
  });

  it("keeps list and task markers rendered while editing item text", () => {
    const doc = "- [ ] task text";
    const editor = mountLivePreview(doc);
    editor.focus();
    editor.dispatch({ selection: { anchor: doc.indexOf("task") + 2 } });

    expect(editor.dom.querySelector(".cm-live-list-marker")).not.toBeNull();
    expect(editor.dom.querySelector(".cm-live-task-checkbox")).not.toBeNull();
    expect(editor.contentDOM.textContent).not.toContain("-");
    expect(editor.contentDOM.textContent).not.toContain("[ ]");

    editor.dispatch({ selection: { anchor: 0 } });
    expect(editor.dom.querySelector(".cm-live-list-marker")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("-");
    expect(editor.dom.querySelector(".cm-live-task-checkbox")).not.toBeNull();

    editor.dispatch({ selection: { anchor: 3 } });
    expect(editor.dom.querySelector(".cm-live-list-marker")).not.toBeNull();
    expect(editor.dom.querySelector(".cm-live-task-checkbox")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("[ ]");
  });

  it("reveals heading and quote markers only at their source tokens", () => {
    const doc = "## Heading\n\n> quote";
    const editor = mountLivePreview(doc);
    editor.focus();

    editor.dispatch({ selection: { anchor: doc.indexOf("Heading") + 2 } });
    expect(editor.contentDOM.textContent).not.toContain("##");

    editor.dispatch({ selection: { anchor: 1 } });
    expect(editor.contentDOM.textContent).toContain("## Heading");

    editor.dispatch({ selection: { anchor: doc.indexOf("quote") + 2 } });
    expect(editor.contentDOM.textContent).not.toContain(">");
    expect(editor.contentDOM.textContent).not.toContain("##");

    editor.dispatch({ selection: { anchor: doc.indexOf(">") } });
    expect(editor.contentDOM.textContent).toContain("> quote");
  });

  it("keeps link labels rendered and opens only the target source group", () => {
    const doc = "[label](target \"title\")";
    const editor = mountLivePreview(doc);
    editor.focus();

    editor.dispatch({ selection: { anchor: doc.indexOf("label") + 2 } });
    expect(editor.contentDOM.textContent).toBe("label");

    editor.dispatch({ selection: { anchor: doc.indexOf("target") + 2 } });
    expect(editor.contentDOM.textContent).toBe("label(target \"title\")");

    editor.dispatch({ selection: { anchor: 0 } });
    expect(editor.contentDOM.textContent).toBe("[label");
  });

  it("keeps inline and fenced code delimiters local to the edited token", () => {
    const doc = [
      "`inline`",
      "",
      "```ts",
      "const value = 1",
      "```",
    ].join("\n");
    const editor = mountLivePreview(doc);
    editor.focus();

    editor.dispatch({ selection: { anchor: doc.indexOf("inline") + 2 } });
    expect(editor.contentDOM.textContent).not.toContain("`inline`");

    editor.dispatch({ selection: { anchor: 0 } });
    expect(editor.contentDOM.textContent).toContain("`inline");
    expect(editor.contentDOM.textContent).not.toContain("`inline`");

    editor.dispatch({ selection: { anchor: doc.indexOf("const") + 3 } });
    expect(editor.contentDOM.textContent).not.toContain("```ts");

    editor.dispatch({ selection: { anchor: doc.indexOf("ts") + 1 } });
    expect(editor.contentDOM.textContent).toContain("```ts");
    expect(editor.contentDOM.textContent).not.toContain("```ts\nconst value = 1\n```");
  });
});
