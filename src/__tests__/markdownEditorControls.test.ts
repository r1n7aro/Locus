// @vitest-environment jsdom
import { createApp, h, nextTick, ref, type App } from "vue";
import type { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseMarkdownEditor from "../components/ui/BaseMarkdownEditor.vue";
import { searchMarkdownEditorResources } from "../components/ui/markdown-editor/markdownEditorResources";

vi.mock("../components/ui/markdown-editor/markdownEditorResources", () => ({ searchMarkdownEditorResources: vi.fn() }));
let app: App;
let view: EditorView;
const documentKey = ref("a");
const model = ref("");

beforeEach(() => {
  documentKey.value = "a";
  if (!Range.prototype.getClientRects) Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  if (!Range.prototype.getBoundingClientRect) Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
});
afterEach(() => { app?.unmount(); document.body.replaceChildren(); vi.clearAllMocks(); });
async function mount(source: string) {
  model.value = source;
  const root = document.createElement("div"); document.body.append(root);
  const editor = ref<{ getEditorView(): EditorView }>();
  app = createApp({ setup: () => () => h(BaseMarkdownEditor, {
    ref: editor, contentKey: documentKey.value, modelValue: model.value, workspaceRef: { checkoutId: "test", expectedGeneration: 7 },
    "onUpdate:modelValue": (value: string) => { model.value = value; },
  }) });
  app.mount(root); await nextTick(); await nextTick(); view = editor.value!.getEditorView();
}
async function click(selector: string) {
  const element = document.querySelector<HTMLElement>(selector)!;
  expect(element).not.toBeNull(); element.click(); await nextTick();
}
async function input(selector: string, text: string) {
  const element = document.querySelector<HTMLInputElement>(selector)!;
  element.value = text; element.dispatchEvent(new Event("input", { bubbles: true })); await nextTick();
}
function submit() { document.querySelector(".md-edit-form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }

describe("Markdown visual property controls", () => {
  it("edits a link URL in a popover while keeping the label rendered and undoable", async () => {
    await mount('[**label**](https://example.com "title")');
    const root = view.dom;
    await click(".cm-live-link");
    expect(view.contentDOM.textContent).toBe("label");
    await input('.md-edit-form input[aria-label="地址"]', "https://example.com/new"); submit(); await nextTick();
    expect(model.value).toBe('[**label**](https://example.com/new "title")');
    expect(view.dom).toBe(root);
    expect(undo(view)).toBe(true); expect(view.state.doc.toString()).toBe('[**label**](https://example.com "title")');
  });

  it("maps an open link editor past external edits before the link", async () => {
    await mount("before [label](old) after"); await click(".cm-live-link");
    await input('.md-edit-form input[aria-label="地址"]', "new");
    model.value = "external before [label](old) after"; await nextTick();
    expect(document.querySelector(".md-edit-form")).not.toBeNull();
    submit(); await nextTick(); expect(model.value).toBe("external before [label](new) after");
  });

  it("closes a stale property editor when the target changes or another document opens", async () => {
    await mount("[label](old)"); await click(".cm-live-link");
    model.value = "[label](external)"; await nextTick(); expect(document.querySelector(".md-edit-form")).toBeNull();
    await click(".cm-live-link"); documentKey.value = "b"; model.value = "other document"; await nextTick();
    expect(document.querySelector(".md-edit-form")).toBeNull(); expect(view.state.doc.toString()).toBe("other document");
  });

  it("edits image descriptions without switching the image to source", async () => {
    await mount('![old](https://example.com/image.png "caption")'); await click(".cm-live-image-frame");
    expect(view.dom.querySelector(".cm-live-image-frame")).not.toBeNull();
    await input('input[aria-label="图片说明"]', "new"); submit(); await nextTick();
    expect(model.value).toBe('![new](https://example.com/image.png "caption")');
  });

  it("selects a resource and updates only the selected line of a Unity reference fence", async () => {
    vi.mocked(searchMarkdownEditorResources).mockResolvedValue([{ name: "New", path: "Assets/New.prefab" }]);
    await mount("```unity:preview\nAssets/Old.prefab\nAssets/Keep.prefab\n```"); await click('[data-reference-path="Assets/Old.prefab"]');
    const choose = Array.from(document.querySelectorAll<HTMLElement>(".md-edit-form button")).find((button) => button.textContent === "选择资源")!;
    choose.click(); await nextTick(); await nextTick();
    await vi.waitFor(() => expect(document.querySelector(".md-resource-results button")).not.toBeNull());
    await click(".md-resource-results button"); submit(); await nextTick();
    expect(model.value).toBe("```unity:preview\nAssets/New.prefab\nAssets/Keep.prefab\n```");
    expect(searchMarkdownEditorResources).toHaveBeenCalledWith("", expect.anything(), { checkoutId: "test", expectedGeneration: 7 });
  });

  it("discards resource search results after switching documents", async () => {
    let resolve!: (value: { name: string; path: string }[]) => void;
    vi.mocked(searchMarkdownEditorResources).mockReturnValue(new Promise((done) => { resolve = done; }));
    await mount("`design/old.md`"); await click(".cm-live-reference");
    Array.from(document.querySelectorAll<HTMLElement>(".md-edit-form button")).find((button) => button.textContent === "选择资源")!.click();
    await nextTick(); documentKey.value = "b"; model.value = "other"; await nextTick();
    resolve([{ name: "late", path: "design/late.md" }]); await nextTick(); await nextTick();
    expect(document.querySelector(".md-edit-form")).toBeNull(); expect(model.value).toBe("other");
  });
});
