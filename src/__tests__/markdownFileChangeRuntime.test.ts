// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MarkdownRenderer from "../components/MarkdownRenderer.vue";
import { provideSurfaceHidden } from "../composables/surfaceVisibility";
import type { RoutedWorkspaceEvent } from "../services/project";
import type { WorkspaceFileChangedPayload } from "../services/workspaceExplorer";

type Change = RoutedWorkspaceEvent<WorkspaceFileChangedPayload>;
const mocks = vi.hoisted(() => ({ listeners: new Set<(event: Change) => void>(), resolve: vi.fn(), propertyMounts: vi.fn() }));
vi.mock("../services/workspaceExplorer", async importOriginal => ({
  ...await importOriginal<typeof import("../services/workspaceExplorer")>(),
  subscribeWorkspaceFileChanges: async (listener: (event: Change) => void) => {
    mocks.listeners.add(listener); return () => { mocks.listeners.delete(listener); };
  },
}));
vi.mock("../services/markdownImage", () => ({ resolveMarkdownImage: mocks.resolve }));
vi.mock("../components/unity/UnityPropertyFenceBlock.vue", () => ({ default: {
  props: ["source", "surfaceHidden"], mounted() { mocks.propertyMounts(); },
  render(this: { source: string; surfaceHidden?: boolean }) {
    return h("div", { class: "test-property-fence", "data-hidden": String(!!this.surfaceHidden) }, this.source);
  },
} }));
let app: App | null;
let host: HTMLDivElement;
const scope = { checkoutId: "checkout", expectedGeneration: 7, expectedMaterializationEpoch: 2 };
async function flush() { await nextTick(); await Promise.resolve(); await nextTick(); }
function emit(path: string, rescan = false, overrides: Partial<Change> = {}) {
  const event: Change = { eventName: "workspace-file-changed", streamRevision: 1, projectId: "project",
    checkoutId: "checkout", workspaceGeneration: 7, materializationEpoch: 2,
    payload: { path, rescan, seq: 1, generation: 1, kind: "upsert", source: "os_watcher" }, ...overrides };
  for (const listener of mocks.listeners) listener(event);
}
function mount(content: string) {
  const props = reactive({ content, workspaceRef: { ...scope } });
  app = createApp({ render: () => h(MarkdownRenderer, props) });
  app.use(createPinia()); app.mount(host); return props;
}
beforeEach(() => {
  vi.useFakeTimers();
  host = document.createElement("div"); document.body.append(host);
  mocks.propertyMounts.mockClear();
  mocks.resolve.mockReset().mockResolvedValue({ url: "data:image/png;base64,AA==", displayPath: "images/local.png" });
});
afterEach(() => {
  app?.unmount(); app = null; host.remove(); mocks.listeners.clear(); vi.useRealTimers();
});

describe("Markdown property fences in a hidden surface", () => {
  it("passes the host surface visibility to fences mounted outside the component tree", async () => {
    const hidden = ref(true);
    app = createApp(defineComponent({
      setup() {
        provideSurfaceHidden(hidden);
        const content = ["```unity-property", "Assets/Data/A.asset#m_Name", "```"].join(String.fromCharCode(10));
        return () => h(MarkdownRenderer, { content, workspaceRef: { ...scope } });
      },
    }));
    app.use(createPinia());
    app.mount(host);
    await flush();
    const fence = () => host.querySelector<HTMLElement>(".test-property-fence");
    expect(fence()?.dataset.hidden).toBe("true");
    hidden.value = false;
    await flush();
    expect(fence()?.dataset.hidden).toBe("false");
    // Re-rendered in place, not remounted (a remount would re-read every row).
    expect(mocks.propertyMounts).toHaveBeenCalledTimes(1);
  });
});

describe("Markdown file event hot path", () => {
  it("does not scan long messages or create refresh tasks for unrelated file storms", async () => {
    mount("Large message without any file dependencies. ".repeat(3000)); await flush();
    const lower = String.prototype.toLocaleLowerCase;
    let fullTextScans = 0;
    const spy = vi.spyOn(String.prototype, "toLocaleLowerCase").mockImplementation(function(this: string, ...args) {
      if (this.length > 10_000) fullTextScans++;
      return lower.apply(this, args);
    });
    const timersBefore = vi.getTimerCount();
    try {
      for (let i = 0; i < 10_000; i++) emit(`node_modules/package/file-${i}.js`);
      emit(".", true);
      expect(fullTextScans).toBe(0);
      expect(vi.getTimerCount()).toBe(timersBefore);
      expect(mocks.resolve).not.toHaveBeenCalled();
    } finally { spy.mockRestore(); }
  });

  it("refreshes related images once per burst, accepts overflow, and ignores stale scopes", async () => {
    mount("![local](images/local.png)"); await flush();
    expect(mocks.resolve).toHaveBeenCalledTimes(1);
    mocks.resolve.mockClear();
    emit("images/other.png");
    emit("node_modules", true);
    emit("images/local.png", false, { checkoutId: "other" });
    emit(".", true, { materializationEpoch: 1 });
    emit(".", true, { workspaceGeneration: 6 });
    await vi.advanceTimersByTimeAsync(150); await flush();
    expect(mocks.resolve).not.toHaveBeenCalled();
    for (let i = 0; i < 100; i++) emit("images/local.png");
    await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).toHaveBeenCalledTimes(1);
    emit(".", true);
    await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).toHaveBeenCalledTimes(2);
  });

  it("preserves property fence sources when a referenced asset is refreshed repeatedly", async () => {
    const source = '{"kind":"asset","path":"Assets/Config.asset","propertyPath":"m_Name"}';
    mount(`\`\`\`unity-property\n${source}\n\`\`\``); await flush();
    expect(host.querySelector(".test-property-fence")?.textContent).toBe(source);
    for (let i = 0; i < 2; i++) {
      emit("Assets/Config.asset");
      await vi.advanceTimersByTimeAsync(100); await flush();
      expect(host.querySelector(".test-property-fence")?.textContent).toBe(source);
      expect(mocks.propertyMounts).toHaveBeenCalledTimes(i + 2);
    }
  });

  it("does not cache or display an old image request after invalidation", async () => {
    let finish!: (value: { url: string; displayPath: string }) => void;
    mocks.resolve.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    mount("![local](images/local.png)"); await flush();
    emit("images/local.png"); await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).toHaveBeenCalledTimes(2);
    finish({ url: "data:image/png;base64,OLD", displayPath: "images/local.png" }); await flush();
    expect(host.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,AA==");
    emit("images", true); await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).toHaveBeenCalledTimes(3);
  });

  it("rebuilds dependencies when text changes and cancels scheduled refresh on unmount", async () => {
    const props = mount("![local](images/local.png)"); await flush();
    props.content = "![next](images/next.png)"; await flush();
    mocks.resolve.mockClear();
    emit("images/local.png"); await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).not.toHaveBeenCalled();
    emit("images/next.png"); app!.unmount(); app = null;
    await vi.advanceTimersByTimeAsync(100); await flush();
    expect(mocks.resolve).not.toHaveBeenCalled();
    expect(mocks.listeners.size).toBe(0);
  });
});
