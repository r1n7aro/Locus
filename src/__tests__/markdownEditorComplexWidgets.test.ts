// @vitest-environment jsdom
import { markdown } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { GFM } from "@lezer/markdown";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { markdownLivePreview } from "../components/ui/markdown-editor/markdownLivePreview";

let view: EditorView | null = null;

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
      value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }),
    });
  }
});

afterEach(() => {
  view?.destroy();
  view = null;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function mountComplexEditor(
  doc: string,
  options: Parameters<typeof markdownLivePreview>[0] = {},
): EditorView {
  const parent = document.createElement("div");
  document.body.appendChild(parent);
  view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      selection: { anchor: 0 },
      extensions: [markdown({ extensions: GFM }), markdownLivePreview(options)],
    }),
  });
  return view;
}

describe("Markdown complex Live Preview widgets", () => {
  it("renders table rows and maps a widget click back to the source table", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const editor = mountComplexEditor([
      "intro",
      "",
      "| Name | Value |",
      "| :--- | ---: |",
      "| Hero | 42 |",
    ].join("\n"));

    const rows = editor.dom.querySelectorAll(".cm-live-table-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("Name");
    expect(rows[1]?.querySelector("[data-align='right']")?.textContent).toBe("42");
    expect(editor.dom.querySelector(".cm-live-collapsed-line")).not.toBeNull();

    rows[1]?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(editor.state.selection.main.head).toBe(editor.state.doc.line(5).from);
    expect(editor.dom.querySelector(".cm-live-table-row")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("| Hero | 42 |");
    expect(consoleError).not.toHaveBeenCalled();
  });

  it("resolves relative images once and ignores stale async DOM", async () => {
    let resolveImage: (value: { url: string; displayPath: string }) => void = () => undefined;
    const resolver = vi.fn(() => new Promise<{ url: string; displayPath: string }>((resolve) => {
      resolveImage = resolve;
    }));
    const editor = mountComplexEditor([
      "intro",
      "",
      "![Hero](Assets/Textures/Hero.png)",
      "",
      "![Hero again](Assets/Textures/Hero.png)",
    ].join("\n"), {
      imageResolver: resolver,
      imageContext: { cacheKey: "checkout@7", contentPath: "docs/design.md" },
    });

    expect(editor.dom.querySelectorAll(".cm-live-image-frame")).toHaveLength(2);
    await Promise.resolve();
    expect(resolver).toHaveBeenCalledTimes(1);
    resolveImage({
      url: "http://locus-binary.localhost/blob/hero",
      displayPath: "Assets/Textures/Hero.png",
    });
    await Promise.resolve();
    await Promise.resolve();

    const frames = editor.dom.querySelectorAll<HTMLElement>(".cm-live-image-frame");
    const images = editor.dom.querySelectorAll<HTMLImageElement>(".cm-live-image");
    await vi.waitFor(() => {
      expect(images[0]?.src).toContain("locus-binary.localhost/blob/hero");
    });
    images[0]?.dispatchEvent(new Event("load"));
    expect(frames[0]?.dataset.state).toBe("ready");

    editor.destroy();
    view = null;
    expect(document.querySelector(".cm-live-image-frame")).toBeNull();
  });

  it("renders math and keeps invalid/code math as source", () => {
    const editor = mountComplexEditor([
      "intro",
      "",
      "Formula $x^2 + y^2$.",
      "",
      "$$",
      "\\frac{a}{b}",
      "$$",
      "",
      "`$inside_code$` and \\$escaped\\$ and $ open",
    ].join("\n"));

    expect(editor.dom.querySelectorAll(".cm-live-math")).toHaveLength(2);
    expect(editor.dom.querySelector(".cm-live-math")?.textContent).toContain("x^2 + y^2");
    expect(editor.contentDOM.textContent).toContain("$inside_code$");
    expect(editor.contentDOM.textContent).toContain("\\$escaped\\$");
    expect(editor.contentDOM.textContent).toContain("$ open");

    editor.dom.querySelector(".cm-live-math")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(editor.dom.querySelectorAll(".cm-live-math")).toHaveLength(1);
    expect(editor.contentDOM.textContent).toContain("$x^2 + y^2$");
  });

  it("renders Locus and Unity references while invalid mixed fences stay source", () => {
    const tick = "`";
    const onReferenceOpen = vi.fn();
    const editor = mountComplexEditor([
      "intro",
      "",
      `${tick}design/editor.md${tick}`,
      "@src/main.ts",
      "view:tools/dashboard",
      "",
      `${tick.repeat(3)}unity:preview`,
      "Assets/Prefabs/Hero.prefab",
      "Assets/Scenes/Main.unity/Root/Camera",
      tick.repeat(3),
      "",
      `${tick.repeat(3)}unity-property`,
      "Assets/Data/Config.asset#m_Name",
      tick.repeat(3),
      "",
      `${tick.repeat(3)}unity:preview`,
      "Assets/Prefabs/Hero.prefab",
      "invalid line",
      tick.repeat(3),
    ].join("\n"), { onReferenceOpen });

    expect(editor.dom.querySelector("[data-reference-kind='knowledge']")?.textContent).toContain("editor.md");
    expect(editor.dom.querySelector("[data-reference-kind='workspace']")?.textContent).toContain("main.ts");
    expect(editor.dom.querySelector("[data-reference-kind='view']")?.textContent).toContain("tools/dashboard");
    expect(editor.dom.querySelectorAll("[data-reference-kind='unity-asset']")).toHaveLength(1);
    expect(editor.dom.querySelectorAll("[data-reference-kind='unity-scene-object']")).toHaveLength(1);
    expect(editor.dom.querySelector("[data-reference-kind='unity-property']")).not.toBeNull();
    expect(editor.contentDOM.textContent).toContain("invalid line");
    expect(editor.contentDOM.textContent).toContain("```unity:preview");
    expect(editor.dom.querySelector<HTMLElement>("[data-reference-kind='workspace']")?.dataset.workspacePath)
      .toBe("src/main.ts");
    expect(editor.dom.querySelector<HTMLElement>("[data-reference-kind='unity-asset']")?.dataset.assetPath)
      .toBe("Assets/Prefabs/Hero.prefab");
    expect(editor.dom.querySelector<HTMLElement>("[data-reference-kind='unity-property']")?.dataset.assetPath)
      .toBe("Assets/Data/Config.asset");

    editor.dom.querySelector("[data-reference-kind='unity-asset']")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(editor.dom.querySelector("[data-reference-kind='unity-scene-object']")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("Assets/Scenes/Main.unity/Root/Camera");

    const knowledge = editor.dom.querySelector("[data-reference-kind='knowledge']");
    expect((knowledge as HTMLElement | null)?.draggable).toBe(true);
    expect((knowledge as HTMLElement | null)?.dataset.knowledgeType).toBe("design");
    knowledge?.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
    expect(onReferenceOpen).toHaveBeenCalledWith(expect.objectContaining({
      kind: "knowledge",
      path: "design/editor.md",
    }));
    expect(editor.dom.querySelector("[data-reference-kind='knowledge']")).not.toBeNull();
    knowledge?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(editor.dom.querySelector("[data-reference-kind='knowledge']")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("`design/editor.md`");
  });

  it("falls back to source for unsafe images and expands image widgets on click", () => {
    const editor = mountComplexEditor([
      "intro",
      "",
      "![unsafe](javascript:alert(1))",
      "",
      "![safe](https://example.com/image.webp)",
    ].join("\n"));

    expect(editor.dom.querySelectorAll(".cm-live-image-frame")).toHaveLength(1);
    expect(editor.contentDOM.textContent).toContain("![unsafe](javascript:alert(1))");
    editor.dom.querySelector(".cm-live-image-frame")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(editor.dom.querySelector(".cm-live-image-frame")).toBeNull();
    expect(editor.contentDOM.textContent).toContain("![safe](https://example.com/image.webp)");
  });

  it("keeps oversized tables and fences as bounded editable source", () => {
    const tableRows = Array.from({ length: 201 }, (_, index) => `| row-${index} | ${index} |`);
    const fenceRows = Array.from({ length: 241 }, (_, index) => `Assets/Prefabs/Hero-${index}.prefab`);
    const tick = "`";
    const editor = mountComplexEditor([
      "intro",
      "",
      "| Name | Value |",
      "| --- | ---: |",
      ...tableRows,
      "",
      `${tick.repeat(3)}unity:preview`,
      ...fenceRows,
      tick.repeat(3),
    ].join("\n"));

    expect(editor.dom.querySelector(".cm-live-table-row")).toBeNull();
    expect(editor.dom.querySelector("[data-reference-kind='unity-asset']")).toBeNull();
    expect(editor.state.doc.toString()).toContain("| row-200 | 200 |");
    expect(editor.state.doc.toString()).toContain("Hero-240.prefab");
  });
});
