import { computed, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ipcInvoke } from "./ipc";
import { workspaceMaterializationMatches, type WorkspaceRef } from "./project";
import * as viewService from "./view";
import { createViewAutomation } from "../components/view/viewAutomation";
import { nativeViewHosts, type NativeViewHost } from "../components/view/viewHostRegistry";
import { getFrontendWorkbench } from "./frontendWorkbench";
import { getDebugConsoleSnapshot } from "./debugConsole";
import { createUnityAssets } from "./unityAssets";
import { createUnityPropertyApi } from "./unityPropertyApi";

export interface FrontendImage { data: string; mimeType: string; width?: number; height?: number; }
export interface FrontendLocatorTarget { selector?: string; text?: string; name?: string; role?: string; id?: string; }
export type FrontendAction = "click" | "doubleClick" | "hover" | "focus" | "type" | "setValue" | "selectOption" | "check" | "uncheck" | "press" | "scroll" | "drag";
export interface FrontendWaitOptions {
  condition?: "runtimeReady" | "selectorVisible" | "selectorHidden" | "textPresent" | "textAbsent" | "noConsoleError";
  selector?: string; text?: string; timeoutMs?: number; pollIntervalMs?: number;
}
export interface FrontendSnapshotOptions { selector?: string; maxElements?: number; includeHidden?: boolean; }
export type FrontendSnapshot = ReturnType<ReturnType<typeof createViewAutomation>["snapshot"]>;
type Execute = (kind: string, payload: Record<string, unknown>) => Promise<unknown>;

function uiHandle(execute: Execute) {
  const action = (action: FrontendAction, target?: FrontendLocatorTarget, options: Record<string, unknown> = {}) => execute("action", { action, target, ...options });
  const locator = (target: FrontendLocatorTarget) => ({
    click: () => action("click", target), doubleClick: () => action("doubleClick", target),
    hover: () => action("hover", target), focus: () => action("focus", target),
    fill: (value: string) => action("setValue", target, { value }),
    type: (text: string) => action("type", target, { text }),
    press: (key: string) => action("press", target, { key }),
    check: (checked = true) => action(checked ? "check" : "uncheck", target),
    select: (value: string) => action("selectOption", target, { value }),
    scroll: (deltaY: number, deltaX = 0) => action("scroll", target, { deltaX, deltaY }),
    dragTo: (to: FrontendLocatorTarget) => action("drag", target, { to }),
  });
  return {
    snapshot: (options: FrontendSnapshotOptions = {}) => execute("snapshot", { ...options }) as Promise<FrontendSnapshot>,
    wait: (options: FrontendWaitOptions = {}) => execute("wait", { ...options }),
    action,
    locator: (selector: string) => locator({ selector }),
    getByRole: (role: string, name: string) => locator({ role, name }),
    getByText: (text: string) => locator({ text }),
    getById: (id: string) => locator({ id }),
  };
}

/** The same typed SDK is used by native modules, View packages and the single
 * TypeScript tool. All project operations retain the caller's checkout scope. */
export function createFrontendSdk(workspaceRef: WorkspaceRef, options: { signal?: AbortSignal; images?: FrontendImage[]; windowLabel?: string; ownerWindow?: Window } = {}) {
  const windowLabel = options.windowLabel ?? "main";
  const ownerWindow = options.ownerWindow ?? window;
  const controller = new AbortController();
  const signal = options.signal ?? controller.signal;
  const refScope = Object.freeze({ ...workspaceRef });
  function assertLive() { if (signal.aborted) throw new Error("Frontend execution was cancelled."); }
  function hosts() { return nativeViewHosts().filter((host) => (host.windowLabel ?? "main") === windowLabel && host.workspaceRef.checkoutId === refScope.checkoutId && host.workspaceRef.expectedGeneration === refScope.expectedGeneration && workspaceMaterializationMatches(refScope.expectedMaterializationEpoch, host.workspaceRef.expectedMaterializationEpoch)); }
  function getHost(id: string): NativeViewHost {
    assertLive();
    const instance = hosts().find((host) => host.instanceId === id);
    if (instance) return instance;
    const matches = hosts().filter((host) => host.viewId === id);
    if (matches.length !== 1) throw new Error(matches.length ? `Multiple View instances match ${id}; use an instanceId from locus.views.instances().` : `View is not open in this window: ${id}`);
    return matches[0]!;
  }
  async function capture(element?: HTMLElement | null) {
    assertLive();
    if ((options.images?.length ?? 0) >= 4) throw new Error("A frontend execution can attach at most four screenshots.");
    const rect = element?.getBoundingClientRect();
    if (rect && (rect.width <= 0 || rect.height <= 0)) throw new Error("Activate the View before capturing it.");
    const image = await ipcInvoke<FrontendImage>("frontend_capture", { windowLabel: options.windowLabel ?? getCurrentWindow().label, clip: rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height, scale: 1 } : null });
    options.images?.push(image);
    return { captured: true, width: image.width, height: image.height, mimeType: image.mimeType };
  }
  function handle(id: string) {
    const host = () => getHost(id);
    const ui = uiHandle(async (kind, payload) => { assertLive(); await host().ready(); return host().execute(kind, payload); });
    return {
      ...ui,
      activate: () => host().activate(),
      reload: () => host().reload(),
      capture: async () => { await host().activate(); return capture(host().root()); },
      logs: (limit = 20) => viewService.viewReadFrontendLog(refScope, { viewId: host().viewId, limit }),
    };
  }
  const automation = () => createViewAutomation({ root: () => ownerWindow.document.body, globals: () => ({ window: ownerWindow, globalThis: ownerWindow }), signal,
    activeViewId: ref("locus"), detail: ref(null), manifest: ref(null), loading: ref(false), error: ref(""), latestFrontendLog: ref(null), runtimeComponent: computed(() => ({})) });
  const ui = uiHandle(async (kind, payload) => { assertLive(); return automation().handle(kind, payload); });
  return {
    workspace: refScope,
    assets: createUnityAssets(refScope, { signal }),
    windowLabel,
    logs: { read: (limit = 100) => getDebugConsoleSnapshot().slice(-Math.min(500, Math.max(1, limit))) },
    ui: { ...ui, capture: () => capture() },
    workbench: {
      tabs: () => getFrontendWorkbench(windowLabel).tabs(),
      activate: (editorId: string) => { assertLive(); return getFrontendWorkbench(windowLabel).activate(editorId); },
      close: (editorId: string) => { assertLive(); return getFrontendWorkbench(windowLabel).close(editorId); },
    },
    views: {
      list: () => viewService.viewList(refScope),
      components: async () => {
        assertLive();
        const { LOCUS_COMPONENTS } = await import("../components/view/viewRuntime");
        assertLive();
        return Object.keys(LOCUS_COMPONENTS);
      },
      create: (request: viewService.ViewCreateRequest) => { assertLive(); return viewService.viewCreate(refScope, request); },
      read: (viewId: string) => viewService.viewRead(refScope, viewId),
      reload: (viewId: string) => { assertLive(); return viewService.viewReload(refScope, viewId); },
      async open(viewId: string) {
        assertLive();
        await getFrontendWorkbench(windowLabel).ready?.(refScope);
        assertLive(); await viewService.viewRun(refScope, viewId, windowLabel);
        const started = Date.now();
        while (!hosts().some((host) => host.viewId === viewId)) {
          assertLive(); if (Date.now() - started > 10_000) throw new Error(`View did not mount in this window: ${viewId}`);
          await new Promise((resolve) => setTimeout(resolve, 30));
        }
        const candidates = hosts().filter((host) => host.viewId === viewId);
        // Match the Workbench's tab order and bind the handle to the selected
        // instance, so another tab cannot make later operations ambiguous.
        const tab = getFrontendWorkbench(windowLabel).tabs().find((tab) => candidates.some((host) => host.instanceId === tab.editorId));
        const view = handle(tab?.editorId ?? candidates[0]!.instanceId);
        await view.activate(); return view;
      },
      get: handle,
      instances: () => hosts().map((host) => ({ viewId: host.viewId, instanceId: host.instanceId, active: host.active(), workspaceRef: host.workspaceRef })),
      compileScript: (request: viewService.ViewCompileScriptRequest) => { assertLive(); return viewService.viewCompileScript(refScope, request); },
      callScript: (request: viewService.ViewCallScriptRequest) => { assertLive(); return viewService.viewCallScript(refScope, request); },
    },
    fs: {
      read: async (path: string) => (await viewService.viewFsReadFile(refScope, { path, encoding: "utf8" })).data as string,
      write: (path: string, content: string) => { assertLive(); return viewService.viewFsWriteFile(refScope, { path, data: content, encoding: "utf8" }); },
      list: (path: string) => viewService.viewFsReaddir(refScope, { path, withFileTypes: true }),
    },
    unity: { property: createUnityPropertyApi(refScope, { signal }) },
  };
}
export type LocusFrontendSdk = ReturnType<typeof createFrontendSdk>;
