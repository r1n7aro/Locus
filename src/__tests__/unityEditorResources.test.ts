// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import UnityEditorResources from "../components/settings/UnityEditorResources.vue";
import { getUnityEditorResources } from "../services/worktrees";
vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../services/worktrees", () => ({ getUnityEditorResources: vi.fn() }));
let app: App | undefined;
let root: HTMLDivElement;
async function settle() { await nextTick(); await Promise.resolve(); await nextTick(); }
beforeEach(() => { vi.useFakeTimers(); root = document.createElement("div"); document.body.append(root); vi.mocked(getUnityEditorResources).mockReset(); });
afterEach(() => { app?.unmount(); root.remove(); vi.useRealTimers(); });
it("shows measured and unavailable memory without adding foreground lifecycle controls", async () => {
  vi.mocked(getUnityEditorResources).mockResolvedValue([
    { projectPath:"F:/Game/main", processId:12, mode:"interactive", managed:false, workingSetBytes:null, importWorkerCount:0, lastError:null },
    { projectPath:"F:/Game/worktree", processId:34, mode:"headless", managed:true, workingSetBytes:1024*1024*1024, importWorkerCount:2, lastError:"Unsaved scene: Test" },
  ]);
  app = createApp(UnityEditorResources); app.mount(root); await settle();
  expect(root.querySelectorAll("tbody tr")).toHaveLength(2);
  expect(root.textContent).toContain("1.00 GB");
  expect(root.textContent).toContain("—");
  expect(root.textContent).toContain("Unsaved scene: Test");
  expect(root.querySelectorAll("button")).toHaveLength(1);
  expect(root.querySelector("button")?.textContent).toBe("common.refresh");
});
it("refreshes while mounted and stops polling after the settings pane closes", async () => {
  vi.mocked(getUnityEditorResources).mockResolvedValue([]);
  app = createApp(UnityEditorResources); app.mount(root); await settle();
  await vi.advanceTimersByTimeAsync(5000); await settle();
  expect(getUnityEditorResources).toHaveBeenCalledTimes(2);
  app.unmount(); app = undefined;
  await vi.advanceTimersByTimeAsync(10000);
  expect(getUnityEditorResources).toHaveBeenCalledTimes(2);
});
it("reports failed measurements and recovers on refresh", async () => {
  vi.mocked(getUnityEditorResources).mockRejectedValueOnce(new Error("process query failed")).mockResolvedValue([]);
  app = createApp(UnityEditorResources); app.mount(root); await settle();
  expect(root.querySelector('[role="alert"]')?.textContent).toContain("process query failed");
  root.querySelector("button")!.click(); await settle();
  expect(root.querySelector('[role="alert"]')).toBeNull();
  expect(root.textContent).toContain("settings.resources.editors.empty");
});
