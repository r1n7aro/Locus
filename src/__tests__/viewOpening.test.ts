// @vitest-environment jsdom
import { createApp, defineComponent, h, provide } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkbenchEditorTransferRecord } from "../types/workbench";

const mocks = vi.hoisted(() => ({
  run: vi.fn(), read: vi.fn(), requirements: vi.fn(), create: vi.fn(), dispatch: vi.fn(), cancel: vi.fn(), remove: vi.fn(),
  checkout: { projectId: "project-a", runtime: { workspaceGeneration: 7, materializationEpoch: 3 } },
  runtimeLabel: "main", listener: null as null | ((event: { payload: unknown }) => void),
  unlisten: vi.fn(), persist: vi.fn(), removeTransfer: vi.fn(), createNative: vi.fn(), closeNative: vi.fn(), unregister: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: mocks.runtimeLabel, listen: async (_event: string, handler: typeof mocks.listener) => { mocks.listener = handler; return mocks.unlisten; } }) }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ WebviewWindow: { getByLabel: async () => ({ close: mocks.closeNative }) } }));
vi.mock("../services/view", () => ({ viewRun: mocks.run, viewRead: mocks.read, checkViewOpenRequirements: mocks.requirements }));
vi.mock("../services/workbenchWindow", () => ({
  currentWorkbenchCursorPosition: async () => ({ x: 100, y: 200 }),
  createInMemoryWorkbenchTransferRecord: (record: object) => ({ ...record, token: "view-transfer", version: 1, createdAt: 123 }),
  WORKBENCH_WINDOW_TRANSFER_ACK_EVENT: "transfer-ack", WORKBENCH_TRANSFER_TIMEOUT_MS: 8000,
  persistWorkbenchTransferRecord: mocks.persist, removeWorkbenchTransferRecord: mocks.removeTransfer,
  createDetachedWorkbenchWindow: mocks.createNative, unregisterWorkbenchAuxWindow: mocks.unregister,
}));
vi.mock("../services/sharedWorkbenchWindow", () => ({ createSharedDetachedWorkbenchWindow: mocks.create, removeSharedWorkbenchWindowHost: mocks.remove }));
vi.mock("../services/sharedWorkbenchTransfer", () => ({ dispatchSharedWorkbenchTransfer: mocks.dispatch, cancelSharedWorkbenchTransfer: mocks.cancel }));
vi.mock("../stores/workspaceContext", () => ({ useWorkspaceContextStore: () => ({ checkoutsById: { "checkout-a": mocks.checkout } }) }));
vi.mock("../stores/workbench", () => ({
  useWorkbenchStore: () => ({ windows: { "workbench-shared-child": { focusedPaneId: "pane-child" } } }),
  createWorkbenchEditorInput: (resource: object, title: string, options: object) => ({ editorId: "new-view", resource, title, ...options }),
}));
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { setSettingValue, settingValue } from "../composables/useSettingsStore";
import { useOpenView, VIEW_OPEN_WINDOW_KEY } from "../composables/useOpenView";
import { openViewPackage, setViewOpenOverride, viewOpenPreferenceKey } from "../services/viewOpening";

const scope = { checkoutId: "checkout-a", expectedGeneration: 7, expectedMaterializationEpoch: 3 };
beforeEach(() => {
  vi.clearAllMocks();
  mocks.runtimeLabel = "main";
  mocks.listener = null;
  mocks.persist.mockResolvedValue(undefined);
  mocks.removeTransfer.mockResolvedValue(undefined);
  mocks.closeNative.mockResolvedValue(undefined);
  mocks.createNative.mockImplementation(async () => {
    mocks.listener?.({ payload: { token: "view-transfer", targetWindowId: "workbench-native", editorId: "native-view", paneId: "native-pane" } });
    return { label: "workbench-native" };
  });
  useDisplaySettings().set("viewOpenTarget", "tab");
  useDisplaySettings().set("viewOpenOverrides", {});
  mocks.checkout.runtime = { workspaceGeneration: 7, materializationEpoch: 3 };
  mocks.read.mockResolvedValue({ summary: { id: "hero", name: "Hero" }, manifest: { name: "Hero" } });
  mocks.requirements.mockResolvedValue(null);
  mocks.create.mockResolvedValue({ label: "workbench-new", pooled: true });
  mocks.dispatch.mockResolvedValue({ targetWindowId: "workbench-new", editorId: "new-view", paneId: "new-pane" });
  mocks.cancel.mockResolvedValue(undefined);
  mocks.remove.mockResolvedValue(undefined);
});
afterEach(() => document.body.replaceChildren());

describe("View opening preferences and window ownership", () => {
  it("opens from a standalone document window and accepts an early transfer acknowledgement", async () => {
    mocks.runtimeLabel = "knowledge-preview";
    useDisplaySettings().set("viewOpenTarget", "window");
    await openViewPackage(scope, "hero");
    expect(mocks.persist).toHaveBeenCalledOnce();
    expect(mocks.createNative).toHaveBeenCalledOnce();
    expect(mocks.create).not.toHaveBeenCalled();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.removeTransfer).toHaveBeenCalledWith("view-transfer");
  });
  it("closes a standalone View window when its transfer fails", async () => {
    mocks.runtimeLabel = "knowledge-preview";
    useDisplaySettings().set("viewOpenTarget", "window");
    mocks.createNative.mockImplementation(async () => {
      mocks.listener?.({ payload: { token: "view-transfer", error: "View unavailable" } });
      return { label: "workbench-native" };
    });
    await expect(openViewPackage(scope, "hero")).rejects.toThrow("View unavailable");
    expect(mocks.closeNative).toHaveBeenCalledOnce();
    expect(mocks.unregister).toHaveBeenCalledWith("workbench-native");
    expect(mocks.removeTransfer).toHaveBeenCalledWith("view-transfer");
  });
  it.each(["main", "workspace-app-project", "workbench-shared-child"])("opens a tab in its source %s", async label => {
    await openViewPackage(scope, "hero", label);
    expect(mocks.run).toHaveBeenCalledWith(scope, "hero", label);
    expect(mocks.create).not.toHaveBeenCalled();
  });
  it("captures the containing shared Workbench instead of Tauri's main webview", async () => {
    let open!: ReturnType<typeof useOpenView>;
    const child = defineComponent({ setup() { open = useOpenView(); return () => h("span"); } });
    const app = createApp(defineComponent({ setup() { provide(VIEW_OPEN_WINDOW_KEY, "workbench-shared-child"); return () => h(child); } }));
    app.mount(document.createElement("div"));
    try { await open(scope, "hero"); expect(mocks.run).toHaveBeenCalledWith(scope, "hero", "workbench-shared-child"); }
    finally { app.unmount(); }
  });
  it("opens a new window without adding or activating a tab in the source", async () => {
    useDisplaySettings().set("viewOpenTarget", "window");
    await openViewPackage(scope, "hero", "workbench-shared-child");
    expect(mocks.run).not.toHaveBeenCalled();
    const record = mocks.dispatch.mock.calls[0]![1] as WorkbenchEditorTransferRecord;
    expect(record.sourceWindowId).toBe("workbench-shared-child");
    expect(record.sourcePaneId).toBe("pane-child");
    expect(record.editor).toMatchObject({ resource: { kind: "view", projectId: "project-a", viewId: "hero" }, preview: false, pinned: true, checkoutBinding: scope });
    expect(mocks.create).toHaveBeenCalledWith(record.token, { x: 100, y: 200 }, record.dragStartedAt);
  });
  it("persists per-view overrides, separates checkouts, and can return to the global default", async () => {
    useDisplaySettings().set("viewOpenTarget", "window");
    setViewOpenOverride(scope, "hero", "tab");
    await openViewPackage(scope, "hero", "workspace-app-project");
    expect(mocks.run).toHaveBeenCalledOnce();
    expect((settingValue("workbench.open.viewOverrides") as Record<string, string>)[viewOpenPreferenceKey(scope, "hero")]).toBe("tab");
    expect(viewOpenPreferenceKey({ ...scope, checkoutId: "checkout-b" }, "hero")).not.toBe(viewOpenPreferenceKey(scope, "hero"));
    setViewOpenOverride(scope, "hero", "default");
    await openViewPackage(scope, "hero");
    expect(mocks.create).toHaveBeenCalledOnce();
  });
  it("applies changes from another window and normalizes invalid stored defaults", () => {
    void setSettingValue("workbench.open.view", "invalid");
    void setSettingValue("workbench.open.viewOverrides", { a: "window", b: "invalid" });
    expect(useDisplaySettings().state.viewOpenTarget).toBe("tab");
    expect(useDisplaySettings().state.viewOpenOverrides).toEqual({ a: "window" });
  });
  it.each(["generation", "materialization", "unity"])("rejects invalid %s before creating a window", async kind => {
    useDisplaySettings().set("viewOpenTarget", "window");
    if (kind === "generation") mocks.checkout.runtime.workspaceGeneration++;
    if (kind === "materialization") mocks.checkout.runtime.materializationEpoch++;
    if (kind === "unity") mocks.requirements.mockResolvedValue({ message: "Unity required" });
    await expect(openViewPackage(scope, "hero")).rejects.toBeTruthy();
    expect(mocks.create).not.toHaveBeenCalled();
  });
  it.each([null, { error: "Mount failed" }])("cleans up a failed new window transfer", async acknowledgement => {
    useDisplaySettings().set("viewOpenTarget", "window");
    mocks.dispatch.mockResolvedValue(acknowledgement);
    await expect(openViewPackage(scope, "hero")).rejects.toThrow();
    expect(mocks.cancel).toHaveBeenCalledWith("workbench-new", "view-transfer");
    expect(mocks.remove).toHaveBeenCalledWith("workbench-new");
  });
});
