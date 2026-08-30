// @vitest-environment jsdom
import type { EditorView } from "@codemirror/view";
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseMarkdownEditor from "../components/ui/BaseMarkdownEditor.vue";

const imageMocks = vi.hoisted(() => ({
  resolve: vi.fn(),
}));

vi.mock("../services/markdownImage", () => ({
  resolveMarkdownImage: imageMocks.resolve,
}));

type EditorExpose = { getEditorView(): EditorView | null };
let app: App<Element> | null = null;

beforeEach(() => {
  imageMocks.resolve.mockReset();
  imageMocks.resolve.mockResolvedValue({
    url: "data:image/png;base64,AA==",
    mimeType: "image/png",
    byteSize: 1,
    displayPath: "Assets/Docs/preview.png",
  });
  if (!Range.prototype.getClientRects) {
    Object.defineProperty(Range.prototype, "getClientRects", {
      configurable: true,
      value: () => [],
    });
  }
  if (!Range.prototype.getBoundingClientRect) {
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
  app?.unmount();
  app = null;
  document.body.replaceChildren();
});

describe("BaseMarkdownEditor workspace images", () => {
  it("resolves local markdown images in the active checkout", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: "![Preview](Assets/Docs/preview.png)",
          viewMode: "rendered",
          contentPath: "design/rendering.md",
          contentKey: "checkout-a:design/rendering.md:body",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
        });
      },
    });
    app.mount(root);
    await nextTick();

    await vi.waitFor(() => expect(imageMocks.resolve).toHaveBeenCalledWith(
      { checkoutId: "checkout-a", expectedGeneration: 7 },
      "Assets/Docs/preview.png",
    ));
    await vi.waitFor(() => expect(
      root.querySelector<HTMLImageElement>(".cm-live-image")?.getAttribute("src"),
    ).toBe("data:image/png;base64,AA=="));
    expect(editorRef.value?.getEditorView()?.state.doc.toString()).toContain("Assets/Docs/preview.png");
  });
});
