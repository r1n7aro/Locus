// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, ref, type App } from "vue";
import { createPinia } from "pinia";
import type { ViewPackageDetail } from "../services/view";

const transport = vi.hoisted(() => ({ read: vi.fn(), logs: vi.fn(async () => {}), listeners: new Set<(event: unknown) => void>() }));
vi.mock("../services/locusRuntime", () => ({ getLocusRuntime: () => ({ invoke: vi.fn(), subscribe: async (_name: string, handler: (event: unknown) => void) => { transport.listeners.add(handler); return () => transport.listeners.delete(handler); } }) }));
vi.mock("../services/view", async (original) => ({ ...await original<typeof import("../services/view")>(), viewRead: transport.read, viewAppendFrontendLogs: transport.logs }));
import ViewRuntimeHost from "../components/view/ViewRuntimeHost.vue";
import { nativeViewHosts } from "../components/view/viewHostRegistry";
let app: App | null = null;
afterEach(async () => { app?.unmount(); app = null; document.body.innerHTML = ""; transport.read.mockReset(); transport.logs.mockClear(); transport.listeners.clear(); await new Promise((resolve) => setTimeout(resolve, 0)); });

function pkg(source: string): ViewPackageDetail {
  return {
    summary: { id: "host", name: "Host", apiVersion: "1", version: "1", template: "blank", displayPath: "", packageRoot: "/runtime-host", packageRelPath: "", manifestPath: "", updatedAt: 0, capabilities: { unity: false }, requirements: { unityConnection: false } },
    manifest: { schema: "locus.view.v1", apiVersion: "1", id: "host", name: "Host", version: "1", template: "component", entry: "host.vue", scripts: [], capabilities: { unity: false }, requirements: { unityConnection: false } },
    files: [{ relPath: "host.vue", content: source, kind: "source", size: source.length, truncated: false }],
  };
}
const source = (text: string, color = "red") => `<script setup lang="ts">import { useViewState } from "@locus/view-runtime"; const state = useViewState({ count: 0 }, "state");</script><template><button @click="state.count++">${text}:{{state.count}}</button></template><style scoped>button {color:${color}}</style>`;
async function mount() {
  const root = document.createElement("div"); document.body.append(root);
  const host = ref<InstanceType<typeof ViewRuntimeHost> | null>(null);
  app = createApp({ setup: () => () => h(ViewRuntimeHost, { ref: host, viewId: "host", instanceId: "instance", workspaceRef: { checkoutId: "a", expectedGeneration: 1 } }) });
  app.use(createPinia()); app.mount(root);
  await nextTick(); await host.value!.ensureMounted();
  return { root, host: host.value! };
}

describe("native View host hot updates", () => {
  it("retains the actual DOM on CSS-only updates, restores View state on script/template updates and keeps the last valid View on compile errors", async () => {
    transport.read.mockResolvedValue(pkg(source("first")));
    const { root, host } = await mount();
    const button = root.querySelector("button")!;
    expect(button.textContent).toBe("first:0"); button.click(); await nextTick();
    transport.read.mockResolvedValue(pkg(source("first", "blue"))); await host.reload();
    expect(root.querySelector("button")).toBe(button); expect(button.textContent).toBe("first:1");
    transport.read.mockResolvedValue(pkg(source("second"))); await host.reload();
    expect(root.querySelector("button")!.textContent).toBe("second:1");
    const last = root.querySelector("button");
    transport.read.mockResolvedValue(pkg('<script setup>const bad = ;</script>')); await host.reload();
    expect(root.querySelector("button")).toBe(last); expect(root.querySelector('[role="alert"]')).not.toBeNull();
    app!.unmount(); app = null;
    expect(nativeViewHosts()).toHaveLength(0);
    expect(document.head.querySelectorAll('[data-locus-view-instance="instance"]')).toHaveLength(0);
    expect(transport.listeners.size).toBe(0);
  });

  it("never installs an older snapshot that resolves after the newest edit", async () => {
    transport.read.mockResolvedValue(pkg(source("initial")));
    const { root, host } = await mount();
    let slow!: (value: ViewPackageDetail) => void;
    transport.read.mockImplementationOnce(() => new Promise((resolve) => { slow = resolve; }));
    const old = host.reload();
    transport.read.mockResolvedValue(pkg(source("latest"))); await host.reload();
    slow(pkg(source("outdated"))); await old;
    expect(root.querySelector("button")!.textContent).toBe("latest:0");
  });
});
