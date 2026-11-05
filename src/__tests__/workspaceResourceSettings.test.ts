// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceResourceSettings from "../components/settings/WorkspaceResourceSettings.vue";
import { getWorkspaceResourceLimits, setWorkspaceResourceLimits, type WorkspaceResourceLimits } from "../services/worktrees";

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../services/worktrees", () => ({ getWorkspaceResourceLimits: vi.fn(), setWorkspaceResourceLimits: vi.fn(), getUnityEditorResources: vi.fn().mockResolvedValue([]), subscribeUnityEditorProcessesChanged: vi.fn(async () => () => {}) }));

const limits: WorkspaceResourceLimits = {
  workspaceIdleTimeoutSecs: 421,
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

describe("workspace resource lifetime settings", () => {
  it("saves idle minutes as seconds while preserving other idle preferences", async () => {
    vi.mocked(setWorkspaceResourceLimits).mockImplementation(async (updated) => ({ revision: 2, limits: updated }));
    const inputs = root.querySelectorAll<HTMLInputElement>('input');
    const input = inputs[inputs.length - 1]!;
    input.value = "15";
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(setWorkspaceResourceLimits).toHaveBeenCalledWith({ ...limits, serviceIdleTimeoutSecs: 900 });
  });
  it("shows the idle timeout without any concurrency controls", () => {
    expect(root.querySelectorAll('input')).toHaveLength(1);
    expect(root.querySelector('label')?.textContent).toContain('settings.resources.serviceIdleTimeoutSecs');
    expect(root.textContent).not.toContain('settings.resources.max');
  });
  it("does not send fractional, empty or zero idle minutes", async () => {
    for (const value of ["1.5", "", "0", "-2"]) expect((await change(value)).value).toBe("8");
    expect(setWorkspaceResourceLimits).not.toHaveBeenCalled();
  });
  it("restores the effective limit after persistence fails and permits retry", async () => {
    vi.mocked(setWorkspaceResourceLimits).mockRejectedValueOnce(new Error("configuration write failed"));
    expect((await change("9")).value).toBe("8");
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("configuration write failed");
    vi.mocked(setWorkspaceResourceLimits).mockImplementationOnce(async (updated) => ({ revision: 2, limits: updated }));
    expect((await change("6")).value).toBe("6");
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });
});
