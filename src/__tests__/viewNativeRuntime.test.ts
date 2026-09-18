// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, nextTick, ref, type App } from "vue";
import { createPinia, defineStore } from "pinia";
import { compileViewPackageSource, scopeViewCss } from "../components/view/viewCompilationCore";
import { createViewRuntimeComponent, type ViewRuntimeApi } from "../components/view/viewRuntime";
import { createViewExecutionScope } from "../components/view/viewExecutionScope";
import type { ViewPackageDetail } from "../services/view";

const apps: App[] = [];
afterEach(() => { apps.splice(0).forEach((app) => app.unmount()); document.body.innerHTML = ""; vi.useRealTimers(); vi.restoreAllMocks(); });
function detail(app: string, main = 'import { createApp } from "vue"; import App from "./App.vue"; createApp(App).mount("#app");', extras: Record<string, string> = {}): ViewPackageDetail {
  return {
    summary: { id: "native-view", name: "Native", apiVersion: "1", version: "1", template: "blank", displayPath: "", packageRoot: "/test/native-view", packageRelPath: "", manifestPath: "", updatedAt: 0, capabilities: { unity: false }, requirements: { unityConnection: false } },
    manifest: { schema: "locus.view.v1", apiVersion: "1", id: "native-view", name: "Native", version: "1", template: "blank", entry: "src/main.ts", style: "style.css", scripts: [], capabilities: { unity: false }, requirements: { unityConnection: false } },
    files: Object.entries({ "src/main.ts": main, "src/App.vue": app, "style.css": "body { color: red; }", ...extras }).map(([relPath, content]) => ({ relPath, content, kind: "source", size: content.length, truncated: false })),
  };
}
const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 3 };
function prepare(pkg: ViewPackageDetail, state = new Map<string, unknown>(), log = vi.fn()) {
  const scope = createViewExecutionScope({ viewId: pkg.manifest.id, editorId: "editor-a", workspaceRef, active: ref(true), state, log });
  const api = new Proxy({ workspaceRef }, { get(target, key) { return key === "workspaceRef" ? target.workspaceRef : vi.fn(async () => null); } }) as ViewRuntimeApi;
  const compilation = compileViewPackageSource({ id: 1, key: "test", scopeId: "test-scope", detail: pkg });
  return { component: createViewRuntimeComponent({ detail: pkg, api, compilation, scope }), compilation, scope, log };
}
function mount(component: ReturnType<typeof prepare>["component"], pinia = createPinia()) {
  const root = document.createElement("div"); document.body.appendChild(root);
  const app = createApp(defineComponent({ setup: () => () => h(component) }));
  app.use(pinia); app.provide("native-token", "inherited"); apps.push(app); app.mount(root);
  return { root, app };
}

describe("native View runtime", () => {
  it("mounts a single Vue entry with inline metadata, native controls and scoped styles, without bootstrap or CSS files", async () => {
    const source = `<view>{"name":"Counter"}</view>
      <script setup lang="ts">
        import { ref } from "vue";
        import { BaseButton } from "@locus/components";
        const count = ref(0);
      </script>
      <template><main><BaseButton @click="count++">Count {{ count }}</BaseButton></main></template>
      <style scoped>main { color: var(--text-color); }</style>`;
    const pkg = detail(source);
    pkg.manifest.entry = "native-view.vue";
    pkg.manifest.template = "component";
    delete pkg.manifest.style;
    pkg.summary.packageRelPath = "workspace/native-view";
    pkg.files = [{ relPath: "workspace/native-view/native-view.vue", content: source, kind: "source", size: source.length, truncated: false }];
    const prepared = prepare(pkg);
    const { root } = mount(prepared.component);
    expect(root.textContent).toBe("Count 0");
    root.querySelector("button")!.click(); await nextTick();
    expect(root.textContent).toBe("Count 1");
    expect(prepared.compilation.styles).toEqual([]);
    expect(Object.keys(prepared.compilation.modules)).toEqual(["workspace/native-view/native-view.vue"]);
    expect(prepared.compilation.modules[pkg.files[0]!.relPath]!.styles.join("")).toContain("data-locus-view-scope");
  });

  it("inherits the native Pinia and provide context and executes legacy plugin registration", async () => {
    const pinia = createPinia();
    const useShared = defineStore("native-shared", { state: () => ({ count: 7 }) });
    const store = useShared(pinia);
    const pkg = detail(`<script setup lang="ts">
      import { inject } from "vue"; import { defineStore } from "pinia";
      const store = defineStore("native-shared", { state: () => ({ count: 0 }) })();
      const native = inject("native-token"); const local = inject("local-token");
    </script><template><main>{{ native }}:{{ local }}:{{ store.count }}<TestGlobal /></main></template>`, `
      import { createApp, h } from "vue"; import App from "./App.vue";
      createApp(App).use({ install(app) { app.provide("local-token", "installed"); app.component("TestGlobal", { render: () => h("b", "registered") }); } }).mount("#app");`);
    const { root } = mount(prepare(pkg).component, pinia);
    expect(root.textContent).toBe("inherited:installed:7registered");
    store.count = 9; await nextTick();
    expect(root.textContent).toContain("installed:9");
    expect(document.querySelectorAll("#app")).toHaveLength(0);
  });

  it("executes JSON modules and binds window.locus and console without changing window globals", () => {
    const nativeConsole = console.log;
    const original = (window as unknown as { locus?: unknown }).locus;
    const prepared = prepare(detail(`<script setup lang="ts">import data from "./data.json"; console.log(window.locus.view.manifest.id);</script><template>{{ data.answer }}</template>`, undefined, { "src/data.json": '{"answer":42}' }));
    const { root } = mount(prepared.component);
    expect(root.textContent).toBe("42");
    expect(prepared.log).toHaveBeenCalledWith("log", ["native-view"]);
    expect(console.log).toBe(nativeConsole);
    expect((window as unknown as { locus?: unknown }).locus).toBe(original);
  });

  it("keeps scope ids stable for CSS-only edits and changes no component code", () => {
    const one = prepare(detail('<template><p class="label">hello</p></template><style scoped>.label { color: red; }</style>'));
    const two = prepare(detail('<template><p class="label">hello</p></template><style scoped>.label { color: blue; }</style>'));
    expect(one.compilation.scriptKey).toBe(two.compilation.scriptKey);
    expect(one.compilation.modules["src/App.vue"]!.scopeId).toBe(two.compilation.modules["src/App.vue"]!.scopeId);
    expect(one.compilation.modules["src/App.vue"]!.styles).not.toEqual(two.compilation.modules["src/App.vue"]!.styles);
    expect(one.compilation.modules["src/main.ts"]).toBe(two.compilation.modules["src/main.ts"]);
    const css = scopeViewCss('html body { margin:0 } .sidebar, button:hover { color:red } @media (min-width:1px){:root{--x:1}}', "test");
    expect(css).not.toContain("html body");
    expect(css).toContain(':where([data-locus-view-scope="test"]) .sidebar');
    const named = scopeViewCss('.body, [data-label="body"], :is(body, .body) { color:red }', "test");
    expect(named).toContain(':where([data-locus-view-scope="test"]) .body');
    expect(named).toContain('[data-label="body"]');
    expect(named.replace(/\s+/g, " ")).toContain(':where([data-locus-view-scope="test"]) :is(');
  });

  it("releases late subscriptions and module timers, while preserving explicit View state", async () => {
    vi.useFakeTimers();
    const state = new Map<string, unknown>();
    const source = '<script setup lang="ts">import { useViewState } from "@locus/view-runtime"; const state = useViewState({ count: 0 }, "counter"); setInterval(() => state.count++, 10);</script><template>{{ state.count }}</template>';
    const first = prepare(detail(source), state);
    const { app, root } = mount(first.component);
    await vi.advanceTimersByTimeAsync(20); await nextTick(); expect(root.textContent).toBe("2");
    const release = vi.fn(); let finish!: (fn: () => void) => void;
    const late = first.scope.trackAsync(new Promise((resolve) => { finish = resolve; }));
    app.unmount(); apps.splice(apps.indexOf(app), 1);
    finish(release); await late; expect(release).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(100); expect(vi.getTimerCount()).toBe(0);
    const second = mount(prepare(detail(source), state).component);
    expect(second.root.textContent).toBe("2");
  });

  it("disposes module-level Vue watches when a View is replaced", async () => {
    const state = new Map<string, unknown>();
    const prepared = prepare(detail('<template>module watcher</template>', 'import { watch } from "vue"; import { useViewState } from "@locus/view-runtime"; import App from "./App.vue"; const state = useViewState({ count: 0, runs: 0 }, "module"); watch(() => state.count, () => { state.runs++; console.log(state.count); }, { immediate: true }); export default App;'), state);
    mount(prepared.component);
    const watched = state.get("module") as { count: number; runs: number };
    watched.count = 1; await nextTick();
    expect(prepared.log).toHaveBeenLastCalledWith("log", [1]);
    prepared.scope.dispose();
    watched.count = 2; await nextTick();
    expect(prepared.log).toHaveBeenCalledTimes(2);
    expect(watched.runs).toBe(2);
  });

  it("cleans animation frames and timers even when browser handle ids overlap", () => {
    const prepared = prepare(detail('<template>lifetime</template>'));
    vi.spyOn(window, "setTimeout").mockReturnValue(7 as unknown as ReturnType<typeof window.setTimeout>);
    vi.spyOn(window, "requestAnimationFrame").mockReturnValue(7);
    const clear = vi.spyOn(window, "clearTimeout");
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    const globals = prepared.scope.globals({}) as unknown as Window;
    globals.setTimeout(() => {}, 10);
    globals.requestAnimationFrame(() => {});
    prepared.scope.dispose();
    expect(clear).toHaveBeenCalledWith(7);
    expect(cancel).toHaveBeenCalledWith(7);
  });
});
