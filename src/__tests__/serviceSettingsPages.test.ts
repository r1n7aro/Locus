// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { createPinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getSwitches: vi.fn(),
  setEnabled: vi.fn(),
  getMultiAgent: vi.fn(),
  setMultiAgent: vi.fn(),
  listInstallations: vi.fn(),
  installAddon: vi.fn(),
  addInstallation: vi.fn(),
  openDialog: vi.fn(),
}));

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.openDialog }));
vi.mock("../services/workspaceServices", () => ({
  getServiceSwitches: mocks.getSwitches,
  setServiceEnabled: mocks.setEnabled,
  subscribeWorkspaceServicesChanged: () => Promise.resolve(() => {}),
}));
vi.mock("../services/system", () => ({
  getUnityMultiAgentEditorEnabled: mocks.getMultiAgent,
  setUnityMultiAgentEditorEnabled: mocks.setMultiAgent,
}));
vi.mock("../services/blender", () => ({
  listBlenderInstallations: mocks.listInstallations,
  refreshBlenderInstallations: mocks.listInstallations,
  listBlenderBackgroundInstances: () => Promise.resolve([]),
  addBlenderInstallation: mocks.addInstallation,
  closeBlenderBackground: vi.fn(),
  installBlenderAddon: mocks.installAddon,
  removeBlenderInstallation: vi.fn(),
  setDefaultBlenderInstallation: vi.fn(),
  unhideBlenderInstallation: vi.fn(),
  subscribeBlenderInstallationsChanged: () => Promise.resolve(() => {}),
  subscribeBlenderInstancesChanged: () => Promise.resolve(() => {}),
}));
vi.mock("../services/worktrees", () => ({
  getWorkspaceResourceLimits: () => Promise.resolve({ revision: 1, limits: {
    workspaceIdleTimeoutSecs: 1, serviceIdleTimeoutSecs: 1, lspIdleTimeoutSecs: 1,
    blenderMaxBackgroundInstances: 2, blenderBackgroundIdleTimeoutSecs: 600,
  } }),
  setWorkspaceResourceLimits: vi.fn(),
}));
function stub(name: string) {
  return { default: { name, render: () => h("div", { class: `stub-${name}` }) } };
}
vi.mock("../components/settings/UnityDiagnostics.vue", () => stub("diagnostics"));
vi.mock("../components/settings/UnityEditorResources.vue", () => stub("editors"));

import UnitySettings from "../components/settings/UnitySettings.vue";
import BlenderSettings from "../components/settings/BlenderSettings.vue";

let app: App;
let host: HTMLElement;

async function flush() { for (let i = 0; i < 8; i++) await nextTick(); }

async function mount(component: object) {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(component) });
  app.use(createPinia());
  app.mount(host);
  await flush();
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.getMultiAgent.mockResolvedValue(false);
  mocks.listInstallations.mockResolvedValue([]);
  mocks.setEnabled.mockImplementation(async (service: string, enabled: boolean) => ({
    unity: service === "unity" ? enabled : false,
    blender: service === "blender" ? enabled : false,
  }));
});

afterEach(() => {
  app?.unmount();
  host?.remove();
});

describe("service settings pages", () => {
  it("shows only the switch while Unity is off and reveals its settings when turned on", async () => {
    mocks.getSwitches.mockResolvedValue({ unity: false, blender: false });
    await mount(UnitySettings);
    expect(host.querySelector('[data-settings-section="connection"]')).toBeNull();
    expect(host.querySelectorAll('[role="switch"]')).toHaveLength(1);

    host.querySelector<HTMLButtonElement>('[role="switch"]')!.click();
    await flush();
    expect(mocks.setEnabled).toHaveBeenCalledWith("unity", true);
    for (const section of ["connection", "codeAnalysis", "compile", "editors", "testing", "multiAgent", "diagnostics"]) {
      expect(host.querySelector(`[data-settings-section="${section}"]`)).not.toBeNull();
    }
    expect(host.querySelector(".stub-editors")).not.toBeNull();
    expect(host.querySelector('[data-setting-key="unity.embed.enabled"]')).not.toBeNull();
    // The multi-Agent editor switch moved here from the Experimental page.
    expect(host.textContent).toContain("settings.experimental.unityMultiAgentEditor");
    // Live state and self-tests need a focused workspace; without one the
    // page says so instead.
    expect(host.querySelector(".stub-diagnostics")).toBeNull();
    expect(host.textContent).toContain("settings.unity.diagnosticsNone");
  });

  it("lists Blender installations only while Blender is on and installs the add-on", async () => {
    mocks.getSwitches.mockResolvedValue({ unity: true, blender: true });
    mocks.listInstallations.mockResolvedValue([{
      id: "c:/blender/blender.exe", path: "C:/Blender/blender.exe", sources: ["program_files"], hidden: false,
      isDefault: true, probeState: "ready", support: "supported", displayVersion: "4.2.1",
      probe: { version: [4, 2, 1], versionString: "4.2.1 LTS", buildBranch: "farlocus", buildHash: "", python: "3.11.7",
        userScripts: "C:/u", portable: false, binary: "" },
      addon: { status: "missing", path: "C:/u/startup/locus_host" },
    }]);
    mocks.installAddon.mockResolvedValue([]);
    await mount(BlenderSettings);
    expect(host.textContent).toContain("4.2.1 farlocus");
    expect(host.textContent).toContain("settings.blender.addon.missing");
    const install = [...host.querySelectorAll("button")].find((button) => button.textContent?.includes("settings.blender.installAddon"));
    install!.click();
    await flush();
    expect(mocks.installAddon).toHaveBeenCalledWith("c:/blender/blender.exe");
  });

  it("explains why a manually added Blender was refused, next to the list", async () => {
    mocks.getSwitches.mockResolvedValue({ unity: true, blender: true });
    mocks.openDialog.mockResolvedValue("C:/Windows/notepad.exe");
    mocks.addInstallation.mockRejectedValue({
      code: "blender.add.not_blender", message: "Could not add this Blender.",
      detail: "this file is not a Blender executable", retryable: false, severity: "error",
    });
    await mount(BlenderSettings);
    const add = [...host.querySelectorAll("button")].find((button) => button.textContent?.includes("settings.blender.add"));
    add!.click();
    await flush();
    expect(mocks.addInstallation).toHaveBeenCalledWith("C:/Windows/notepad.exe");
    const alert = host.querySelector('[role="alert"]')!;
    expect(alert.textContent).toBe("settings.blender.addError.not_blender");
    // Shown in the installations block, not under the background instances.
    expect(alert.closest(".blender-settings-block")!.textContent).toContain("settings.blender.installations");
  });

  it("does not probe installations while Blender is off", async () => {
    mocks.getSwitches.mockResolvedValue({ unity: true, blender: false });
    await mount(BlenderSettings);
    expect(mocks.listInstallations).not.toHaveBeenCalled();
  });

  it("toggles the Blender switch through the service command", async () => {
    mocks.getSwitches.mockResolvedValue({ unity: true, blender: false });
    await mount(BlenderSettings);
    const toggle = host.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    toggle.click();
    await flush();
    expect(mocks.setEnabled).toHaveBeenCalledWith("blender", true);
    expect(host.querySelector('[role="switch"]')!.getAttribute("aria-checked")).toBe("true");
  });
});
