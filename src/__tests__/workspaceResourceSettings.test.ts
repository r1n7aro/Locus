// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceResourceSettings from "../components/settings/WorkspaceResourceSettings.vue";
import { getWorkspaceResourceLimits, setWorkspaceResourceLimits, type WorkspaceResourceLimits } from "../services/worktrees";

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../services/worktrees", () => ({ getWorkspaceResourceLimits: vi.fn(), setWorkspaceResourceLimits: vi.fn(), getUnityEditorResources: vi.fn().mockResolvedValue([]) }));

const limits: WorkspaceResourceLimits = {
  maxRunningSessions: 4, maxUnityEditors: 3, maxRunningWorkspaceServices: 4,
  maxWatchedWorkspaces: 8, maxLspProcesses: 4, maxConcurrentServiceStarts: 2,
  maxConcurrentCompileJobs: 2, maxCompileQueueDepth: 17, workspaceIdleTimeoutSecs: 421,
  serviceIdleTimeoutSecs: 422, lspIdleTimeoutSecs: 423,
};
let app: App | undefined;
let root: HTMLDivElement;
async function settle() { await nextTick(); await Promise.resolve(); await nextTick(); }
async function change(value: string) {
  const input = root.querySelector<HTMLInputElement>("input")!;
  input.value = value;
  input.dispatchEvent(new Event("change", { bubbles: true }));
  await settle();
  return input;
}
beforeEach(async () => {
  vi.mocked(getWorkspaceResourceLimits).mockResolvedValue({ revision: 1, limits: { ...limits } });
  vi.mocked(setWorkspaceResourceLimits).mockReset();
  root = document.createElement("div"); document.body.append(root);
  app = createApp(WorkspaceResourceSettings); app.mount(root); await settle();
});
afterEach(() => { app?.unmount(); root.remove(); });

describe("workspace concurrency settings", () => {
  it("saves idle minutes as seconds while preserving concurrency limits", async () => {
    vi.mocked(setWorkspaceResourceLimits).mockImplementation(async (updated) => ({ revision: 2, limits: updated }));
    const inputs = root.querySelectorAll<HTMLInputElement>('input');
    const input = inputs[inputs.length - 1]!;
    input.value = "15";
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(setWorkspaceResourceLimits).toHaveBeenCalledWith({ ...limits, serviceIdleTimeoutSecs: 900 });
  });
  it("changes the requested budget without resetting hidden queue and idle budgets", async () => {
    vi.mocked(setWorkspaceResourceLimits).mockImplementation(async (updated) => ({ revision: 2, limits: updated }));
    await change("7");
    expect(setWorkspaceResourceLimits).toHaveBeenCalledWith({ ...limits, maxRunningSessions: 7 });
    expect(root.querySelector<HTMLInputElement>("input")!.value).toBe("7");
  });
  it("does not send fractional, empty or zero concurrency values", async () => {
    for (const value of ["1.5", "", "0", "-2"]) expect((await change(value)).value).toBe("4");
    expect(setWorkspaceResourceLimits).not.toHaveBeenCalled();
  });
  it("restores the effective limit after persistence fails and permits retry", async () => {
    vi.mocked(setWorkspaceResourceLimits).mockRejectedValueOnce(new Error("configuration write failed"));
    expect((await change("8")).value).toBe("4");
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("configuration write failed");
    vi.mocked(setWorkspaceResourceLimits).mockImplementationOnce(async (updated) => ({ revision: 2, limits: updated }));
    expect((await change("6")).value).toBe("6");
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });
});
