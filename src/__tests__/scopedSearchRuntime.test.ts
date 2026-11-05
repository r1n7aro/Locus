// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EditorView } from "@codemirror/view";
import { registerSearchScope, useGlobalSearchShortcut } from "../composables/useSearchScope";
import { createDefaultShortcutSettings, formatShortcut, useKeyboardShortcuts } from "../composables/useKeyboardShortcuts";
import { setSettingValue, settingValue } from "../composables/useSettingsStore";
import { documentSearchRanges } from "../composables/documentSearch";
import BaseMarkdownEditor from "../components/ui/BaseMarkdownEditor.vue";
import WorkspaceTree from "../components/explorer/WorkspaceTree.vue";
import AssetTextViewer from "../components/asset/AssetTextViewer.vue";
import SearchableContent from "../components/ui/SearchableContent.vue";
import ShortcutSettings from "../components/settings/ShortcutSettings.vue";

const cleanup: Array<() => void> = [];
function mount(component: Parameters<typeof createApp>[0]) {
  const host = document.createElement("div"); document.body.append(host);
  const app: App = createApp(component); app.mount(host);
  cleanup.push(() => app.unmount());
  return host;
}
function key(target: EventTarget, value = "f", modifiers: KeyboardEventInit = { ctrlKey: true }) {
  const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...modifiers });
  target.dispatchEvent(event); return event;
}
function point(target: Element) { target.dispatchEvent(new Event("pointerdown", { bubbles: true })); }
async function settle() { await nextTick(); await nextTick(); }
async function input(root: Element, value: string) {
  const field = root.querySelector<HTMLInputElement>(".find-bar-input")!;
  field.value = value; field.dispatchEvent(new Event("input", { bubbles: true })); await settle();
  return field;
}
function resetShortcuts() {
  const shortcuts = useKeyboardShortcuts();
  for (const action of ["newChat", "cancelRun", "findInContext", "globalSearch"] as const) shortcuts.resetShortcut(action);
}
beforeEach(() => {
  resetShortcuts();
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
  HTMLElement.prototype.scrollIntoView = vi.fn();
});
afterEach(() => {
  for (const dispose of cleanup.splice(0).reverse()) dispose();
  document.body.replaceChildren();
  resetShortcuts();
});

describe("search shortcut routing", () => {
  function fixture() {
    const global = vi.fn(); const left = vi.fn(); const right = vi.fn();
    const host = mount(defineComponent({
      setup() {
        useGlobalSearchShortcut(window, { open: global, enabled: () => true });
        return () => h("div", [
          h("section", { "data-search-boundary": "", id: "left" }, [h("button", { id: "tab" }, "tab"), h("div", { id: "left-content" })]),
          h("section", { "data-search-boundary": "", id: "right" }, [h("input", { id: "right-content" })]),
          h("button", { id: "outside" }, "outside"),
        ]);
      },
    }));
    const leftRoot = host.querySelector<HTMLElement>("#left-content")!;
    const rightRoot = host.querySelector<HTMLInputElement>("#right-content")!;
    cleanup.push(registerSearchScope(window, { root: () => leftRoot, open: left }));
    cleanup.push(registerSearchScope(window, { root: () => rightRoot, open: right }));
    return { host, global, left, right, leftRoot, rightRoot };
  }
  it("uses the clicked region even when focus remains in another pane, and forces global search with Shift", () => {
    const f = fixture(); f.rightRoot.focus(); point(f.leftRoot);
    expect(key(f.rightRoot).defaultPrevented).toBe(true);
    expect(f.left).toHaveBeenCalledOnce(); expect(f.right).not.toHaveBeenCalled(); expect(f.global).not.toHaveBeenCalled();
    key(f.rightRoot, "F", { ctrlKey: true, shiftKey: true }); expect(f.global).toHaveBeenCalledOnce();
  });
  it("resolves a tab header to its visible document and skips retained hidden/inert tabs", () => {
    const f = fixture(); const hidden = document.createElement("div"); hidden.style.display = "none";
    f.leftRoot.before(hidden); const stale = vi.fn();
    cleanup.push(registerSearchScope(window, { root: () => hidden, open: stale, priority: 10 }));
    point(f.host.querySelector("#tab")!); key(window);
    expect(f.left).toHaveBeenCalledOnce(); expect(stale).not.toHaveBeenCalled();
    f.leftRoot.setAttribute("inert", ""); key(window); expect(f.global).toHaveBeenCalledOnce();
  });
  it("falls back outside a search scope, ignores IME and leaves shortcut recording alone", () => {
    const f = fixture(); point(f.host.querySelector("#outside")!); key(window); expect(f.global).toHaveBeenCalledOnce();
    point(f.leftRoot); expect(key(window, "f", { ctrlKey: true, isComposing: true }).defaultPrevented).toBe(false);
    const recorder = f.host.querySelector<HTMLElement>("#outside")!;
    recorder.dataset.shortcutRecording = "true"; recorder.focus();
    expect(key(recorder).defaultPrevented).toBe(false); expect(f.left).not.toHaveBeenCalled();
  });
  it("uses changed shortcuts immediately and persists both search actions", () => {
    const f = fixture(); const settings = useKeyboardShortcuts(); point(f.leftRoot);
    settings.setShortcut("findInContext", { ctrl: true, alt: false, meta: false, shift: false, key: "h" });
    settings.setShortcut("globalSearch", { ctrl: true, alt: true, meta: false, shift: false, key: "g" });
    expect(key(window).defaultPrevented).toBe(false);
    key(window, "h", { ctrlKey: true }); expect(f.left).toHaveBeenCalledOnce();
    key(window, "g", { ctrlKey: true, altKey: true }); expect(f.global).toHaveBeenCalledOnce();
    expect(settingValue("shortcuts.globalSearch")).toBe("ctrl+alt+g");
  });
  it("uses defaults for actions without a stored shortcut when another window changes one", () => {
    void setSettingValue("shortcuts.newChat", "ctrl+alt+n");
    const state = useKeyboardShortcuts().state;
    expect(formatShortcut(state.newChat)).toBe("Ctrl+Alt+N");
    expect(formatShortcut(state.findInContext)).toBe("Ctrl+F");
    expect(formatShortcut(state.globalSearch)).toBe("Ctrl+Shift+F");
    expect(formatShortcut(createDefaultShortcutSettings("mac").globalSearch, "mac")).toBe("Shift+Cmd+F");
  });
  it("isolates auxiliary-window scopes and cleans up the router", () => {
    const frame = document.createElement("iframe"); document.body.append(frame);
    const owner = frame.contentWindow!; const root = owner.document.createElement("div"); owner.document.body.append(root);
    const open = vi.fn(); const release = registerSearchScope(owner, { root: () => root, open }); cleanup.push(release);
    point(root); key(root); expect(open).toHaveBeenCalledOnce();
    key(window); expect(open).toHaveBeenCalledOnce();
    release(); key(root); expect(open).toHaveBeenCalledOnce();
  });
});

describe("local search surfaces", () => {
  it.each(["native", "rendered"] as const)("searches a %s document without changing its text or dirty state", async (viewMode) => {
    const editor = ref<{ getEditorView(): EditorView } | null>(null); const changed = vi.fn();
    const host = mount(() => h("section", { "data-search-boundary": "" }, [
      h("h1", "Document"), h(BaseMarkdownEditor, { ref: editor, modelValue: "# Alpha\n\nalpha **text**\n\n末尾 Alpha", viewMode, disabled: true, onDocumentChange: changed }),
    ]));
    await settle(); point(host.querySelector("h1")!); key(window); await settle();
    const field = await input(host, "alpha");
    expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/3");
    key(field, "Enter", {}); await settle(); expect(host.querySelector(".find-bar-count")?.textContent).toBe("2/3");
    key(field, "Enter", { shiftKey: true }); await settle(); expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/3");
    expect(editor.value!.getEditorView().state.doc.toString()).toBe("# Alpha\n\nalpha **text**\n\n末尾 Alpha");
    expect(changed).not.toHaveBeenCalled();
    key(field, "Escape", {}); await settle(); expect(host.querySelector(".find-bar")).toBeNull();
  });
  it("finds and scrolls to file names beyond the virtualized viewport, wraps, and preserves the list", async () => {
    const items = Array.from({ length: 500 }, (_, index) => ({ key: String(index), treeRow: { key: String(index), name: index === 480 || index === 490 ? `Match-${index}.md` : `File-${index}.txt`, kind: "file" as const, depth: 0 } }));
    const host = mount(() => h(WorkspaceTree, { items })); await settle();
    expect(host.querySelector('[data-tree-key="480"]')).toBeNull();
    point(host.querySelector(".file-tree-list")!); key(window); await settle();
    const field = await input(host, "match");
    expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/2");
    expect(host.querySelector(".is-search-current")?.getAttribute("data-tree-key")).toBe("480");
    key(field, "Enter", { shiftKey: true }); await settle();
    expect(host.querySelector(".is-search-current")?.getAttribute("data-tree-key")).toBe("490");
    key(field, "Enter", {}); await settle();
    expect(host.querySelector(".is-search-current")?.getAttribute("data-tree-key")).toBe("480");
    await input(host, "missing"); expect(host.querySelector(".find-bar-count")?.textContent).toBe("0/0");
    expect(host.querySelectorAll(".workspace-tree-row").length).toBeGreaterThan(1);
    key(field, "Escape", {}); await settle(); expect(host.querySelector(".find-bar")).toBeNull();
  });
  it("searches a read-only text preview while excluding line numbers and deferring IME input", async () => {
    const host = mount(() => h(AssetTextViewer, { snippet: "hello\nHello 12\n世界", totalLines: 3, truncated: false })); await settle();
    point(host.querySelector(".atv-text")!); key(window); await settle();
    const field = await input(host, "hello"); expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/2");
    field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    await input(host, "世"); expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/2");
    expect(key(field, "Enter", { isComposing: true }).defaultPrevented).toBe(false);
    field.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })); await settle();
    expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/1");
    await input(host, "3"); expect(host.querySelector(".find-bar-count")?.textContent).toBe("0/0");
  });
  it("finds phrases across markdown inline nodes, ignores controls, and preserves rendered markup", async () => {
    const host = mount(() => h(SearchableContent, null, { default: () => h("p", ["Hello ", h("strong", "world"), h("button", "world")]) })); await settle();
    const original = host.querySelector("p")!.innerHTML;
    const ranges = documentSearchRanges(host.querySelector("p")!, "hello world");
    expect(ranges.map((range) => range.toString())).toEqual(["Hello world"]);
    point(host.querySelector("p")!); key(window); await settle(); await input(host, "world");
    expect(host.querySelector(".find-bar-count")?.textContent).toBe("1/1");
    expect(host.querySelector("p")!.innerHTML).toBe(original);
  });
  it("anchors the floating search at the owner's top right and follows a narrower pane", async () => {
    const host = mount(() => h(SearchableContent, null, { default: () => h("p", "Searchable text") })); await settle();
    const surface = host.querySelector<HTMLElement>(".searchable-content")!;
    let bounds = { top: 80, left: 260, right: 1100, width: 840 };
    surface.getBoundingClientRect = () => bounds as DOMRect;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
    point(surface); key(window); await settle();
    const floating = host.querySelector<HTMLElement>(".floating-find-bar")!;
    expect(floating.style.top).toBe("86px");
    expect(floating.style.left).toBe("672px");
    expect(floating.style.width).toBe("420px");
    bounds = { top: 100, left: 10, right: 270, width: 260 };
    window.dispatchEvent(new Event("resize"));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); await settle();
    expect(floating.style.top).toBe("106px");
    expect(floating.style.left).toBe("18px");
    expect(floating.style.width).toBe("244px");
    expect(host.querySelector("p")?.textContent).toBe("Searchable text");
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1024 });
  });
  it("records search shortcuts in Settings, prevents conflicts, and restores defaults", async () => {
    const host = mount(ShortcutSettings); await settle();
    const rows = host.querySelectorAll(".shortcut-row");
    const record = rows[0]!.querySelector<HTMLButtonElement>("button")!;
    record.click(); await settle(); key(record, "f", { ctrlKey: true, shiftKey: true }); await settle();
    expect(host.querySelector(".shortcut-error")?.textContent).toBeTruthy();
    expect(record.dataset.shortcutRecording).toBe("true");
    key(record, "h", { ctrlKey: true }); await settle();
    expect(formatShortcut(useKeyboardShortcuts().state.findInContext)).toBe("Ctrl+H");
    rows[0]!.querySelectorAll<HTMLButtonElement>("button")[1]!.click(); await settle();
    expect(formatShortcut(useKeyboardShortcuts().state.findInContext)).toBe("Ctrl+F");
  });
});
