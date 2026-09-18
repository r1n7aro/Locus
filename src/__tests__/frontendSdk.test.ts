// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createFrontendSdk } from "../services/frontendSdk";
import { executeFrontendTypeScript } from "../services/frontendExecution";
import { registerFrontendWorkbench } from "../services/frontendWorkbench";
import { registerNativeViewHost } from "../components/view/viewHostRegistry";
import * as viewService from "../services/view";
import * as ipc from "../services/ipc";

const releases: Array<() => void> = [];
const scope = { checkoutId: "a", expectedGeneration: 2, expectedMaterializationEpoch: 4 };
afterEach(() => { releases.splice(0).forEach((release) => release()); document.body.innerHTML = ""; vi.restoreAllMocks(); });

describe("unified frontend TypeScript SDK", () => {
  it("discovers native components and initializes a directory structure without a template", async () => {
    const sdk = createFrontendSdk(scope);
    expect(await sdk.views.components()).toEqual(expect.arrayContaining(["LinkBoard", "CanvasView", "GraphView", "SerializedTableView", "UnityPropertyDraw", "BaseButton"]));
    expect(sdk.views).not.toHaveProperty("templates");
    const invoke = vi.spyOn(ipc, "ipcInvoke").mockResolvedValue({});
    await sdk.views.create({ fileName: "new-panel.vue", directories: ["src/components", "unity"] });
    expect(invoke).toHaveBeenCalledWith("view_create", { workspaceRef: scope, request: { fileName: "new-panel.vue", directories: ["src/components", "unity"] } });
  });

  it("creates a filename-based component View through the scoped native command without requiring template metadata", async () => {
    const component = '<template><p>Single file</p></template><style scoped>p { color: var(--text-color); }</style>';
    const invoke = vi.spyOn(ipc, "ipcInvoke").mockResolvedValue({ manifest: { id: "counter" }, summary: { packageRoot: "/views/counter" } });
    const output = await executeFrontendTypeScript(`
      const created = await locus.views.create({ fileName: "counter.vue", component: ${JSON.stringify(component)} });
      return { id: created.manifest.id, root: created.summary.packageRoot };
    `, scope);
    expect(output.result).toEqual({ id: "counter", root: "/views/counter" });
    expect(invoke).toHaveBeenCalledWith("view_create", { workspaceRef: scope, request: { fileName: "counter.vue", component } });
  });

  it("executes TypeScript imports and controls actual native DOM without a View", async () => {
    const button = document.createElement("button"); button.textContent = "Run"; button.id = "run";
    const input = document.createElement("input"); input.id = "value";
    document.body.append(button, input);
    button.getBoundingClientRect = () => ({ x: 0, y: 0, width: 100, height: 30, top: 0, left: 0, right: 100, bottom: 30, toJSON: () => ({}) });
    const click = vi.fn(); button.onclick = click;
    const nativeLog = console.log;
    const output = await executeFrontendTypeScript(`
      import { locus } from "@locus/frontend";
      const value: string = "changed";
      await locus.ui.locator("#value").fill(value);
      await locus.ui.getByRole("button", "Run").click();
      console.log("completed");
      return { value, count: 1 as number };
    `, scope);
    expect(input.value).toBe("changed"); expect(click).toHaveBeenCalledTimes(1);
    expect(output.result).toEqual({ value: "changed", count: 1 });
    expect(output.logs).toEqual([{ level: "log", message: "completed" }]);
    expect(console.log).toBe(nativeLog);
  });

  it("routes workbench operations to the mounted native controller", async () => {
    const activate = vi.fn(async () => {}); const close = vi.fn(async () => {});
    releases.push(registerFrontendWorkbench({ tabs: () => [{ editorId: "editor", paneId: "pane", title: "Panel", kind: "view", active: true }], activate, close }));
    releases.push(registerFrontendWorkbench({ tabs: () => [], activate: vi.fn(), close: vi.fn() }, "workbench-shared-pool-1"));
    const output = await executeFrontendTypeScript('const [tab] = locus.workbench.tabs(); await locus.workbench.activate(tab.editorId); return tab.title;', scope);
    expect(output.result).toBe("Panel"); expect(activate).toHaveBeenCalledWith("editor");
    expect(close).not.toHaveBeenCalled();
  });

  it("isolates View handles by checkout, generation, epoch and instance", async () => {
    function add(id: string, workspaceRef = scope) {
      const execute = vi.fn(async () => ({ id }));
      releases.push(registerNativeViewHost({ viewId: "same", instanceId: id, workspaceRef, root: () => null, active: () => true, ready: async () => {}, reload: async () => {}, activate: async () => {}, execute }));
      return execute;
    }
    add("other", { ...scope, checkoutId: "b" }); add("stale", { ...scope, expectedMaterializationEpoch: 3 });
    const execute = add("current");
    const sdk = createFrontendSdk(scope);
    expect(sdk.views.instances().map((view) => view.instanceId)).toEqual(["current"]);
    expect(await sdk.views.get("same").snapshot()).toEqual({ id: "current" }); expect(execute).toHaveBeenCalledTimes(1);
    add("second"); expect(() => sdk.views.get("same").activate()).toThrow(/Multiple View instances/);
    await sdk.views.get("current").snapshot(); expect(execute).toHaveBeenCalledTimes(2);
  });

  it("waits for Workbench restoration before opening in the addressed window", async () => {
    let ready!: () => void;
    const restored = new Promise<void>((resolve) => { ready = resolve; });
    const waitForCheckout = vi.fn(() => restored);
    const windowLabel = "workbench-shared";
    releases.push(registerFrontendWorkbench({ ready: waitForCheckout, tabs: () => [], activate: async () => {}, close: async () => {} }, windowLabel));
    const open = vi.spyOn(viewService, "viewRun").mockImplementation(async () => {
      releases.push(registerNativeViewHost({ viewId: "native", instanceId: "editor", windowLabel, workspaceRef: scope, root: () => null, active: () => true, ready: async () => {}, reload: async () => {}, activate: async () => {}, execute: async () => ({}) }));
      return { id: "native", windowLabel, hostUrl: "", packageRoot: "" };
    });
    const pending = createFrontendSdk(scope, { windowLabel }).views.open("native");
    await Promise.resolve(); expect(open).not.toHaveBeenCalled();
    expect(waitForCheckout).toHaveBeenCalledWith(scope);
    ready(); await pending;
    expect(open).toHaveBeenCalledWith(scope, "native", windowLabel);
  });

  it("reuses an existing View tab and keeps its returned handle bound to that tab", async () => {
    const firstExecute = vi.fn(async () => ({ instance: "first-tab" }));
    const secondExecute = vi.fn(async () => ({ instance: "second-tab" }));
    const firstActivate = vi.fn(async () => {});
    // Registration order can differ from tab order after a tab is transferred.
    for (const [instanceId, execute, activate] of [
      ["second-tab", secondExecute, vi.fn(async () => {})],
      ["first-tab", firstExecute, firstActivate],
    ] as const) {
      releases.push(registerNativeViewHost({ viewId: "existing", instanceId, workspaceRef: scope,
        root: () => null, active: () => false, ready: async () => {}, reload: async () => {}, activate, execute }));
    }
    releases.push(registerFrontendWorkbench({
      tabs: () => [
        { editorId: "chat", paneId: "main", title: "Chat", kind: "session", active: true },
        { editorId: "first-tab", paneId: "main", title: "View", kind: "view", active: false },
        { editorId: "second-tab", paneId: "other", title: "View", kind: "view", active: false },
      ], activate: async () => {}, close: async () => {},
    }));
    vi.spyOn(viewService, "viewRun").mockResolvedValue({ id: "existing", windowLabel: "main", hostUrl: "", packageRoot: "" });
    const sdk = createFrontendSdk(scope);
    const panel = await sdk.views.open("existing");
    await sdk.views.open("existing");
    expect(firstActivate).toHaveBeenCalledTimes(2);
    expect(sdk.views.instances()).toHaveLength(2);
    expect(await panel.snapshot()).toEqual({ instance: "first-tab" });
    expect(secondExecute).not.toHaveBeenCalled();
  });

  it("rejects syntax errors before touching the frontend and cleans timed-out executions", async () => {
    await expect(executeFrontendTypeScript('const broken = ;', scope)).rejects.toThrow();
    const controller = new AbortController(); const sdk = createFrontendSdk(scope, { signal: controller.signal });
    controller.abort(); await expect(sdk.ui.locator("button").click()).rejects.toThrow(/cancelled/);
  });
});
