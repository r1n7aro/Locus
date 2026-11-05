// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  setTarget: vi.fn(),
  closeBackground: vi.fn(),
  installAddon: vi.fn(),
  reloadHosts: vi.fn(),
}));

vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join("|") }));
vi.mock("../services/blender", () => ({
  setBlenderSessionTarget: mocks.setTarget,
  closeBlenderBackground: mocks.closeBackground,
  installBlenderAddon: mocks.installAddon,
  reloadBlenderHosts: mocks.reloadHosts,
}));
vi.mock("../components/ToolResultImages.vue", () => ({
  default: { name: "ToolResultImages", props: ["images"], render(this: { images: unknown[] }) {
    return h("div", { class: "stub-images" }, String(this.images.length));
  } },
}));

import BlenderStatusPanel from "../components/chat/BlenderStatusPanel.vue";
import BlenderExecuteToolBlock from "../components/tool-block-overrides/BlenderExecuteToolBlock.vue";
import type { BlenderWorkspaceStatus } from "../services/blender";

let app: App | null = null;
let host: HTMLElement;

async function flush() { for (let i = 0; i < 6; i++) await nextTick(); }

async function mount(component: object, props: Record<string, unknown>) {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(component, props) });
  app.mount(host);
  await flush();
}

const WORKSPACE = { checkoutId: "checkout-b", expectedGeneration: 1 };

function instance(overrides: Record<string, unknown> = {}) {
  return {
    key: "100-5", pid: 100, mode: "interactive", version: "4.2.1", versionString: "4.2.1 LTS", buildBranch: "",
    binary: "", python: "3.11", file: "F:/p/hero.blend", dirty: true, savedVersion: "4.2", hostHash: "h",
    connection: "connected", launchedByCheckout: null, busy: false, capabilities: [], inWorkspace: true,
    isTarget: false, ownedBackground: false, sharedFile: false, ...overrides,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.setTarget.mockResolvedValue("100-5");
  mocks.closeBackground.mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = null;
  host?.remove();
});

describe("Blender status panel", () => {
  it("lists workspace instances, sets the session target and closes background instances", async () => {
    const status: BlenderWorkspaceStatus = {
      enabled: true,
      running: true,
      instances: [
        instance(),
        instance({ key: "200-7", pid: 200, inWorkspace: false, file: "D:/other.blend" }),
        instance({ key: "301-2", pid: 301, file: "F:/p/b.blend", otherSessionTarget: true }),
      ] as never,
      background: [{ pid: 300, checkout: "checkout-b", installation: "x", version: "4.2.1", file: null, log: null, uptimeSecs: 3 }],
      workspaceVersion: {
        pinned: null, inferred: "5.0", effective: "5.0",
        distribution: { versions: [["5.0", 3], ["4.2", 1]], files: 4, unreadable: 0, truncated: false },
      },
      addon: { installation: null, status: null, outdatedInstances: [] },
    };
    const changed = vi.fn();
    await mount(BlenderStatusPanel, { status, workspaceRef: WORKSPACE, sessionId: "session-1", onChanged: changed });
    const text = host.textContent ?? "";
    const row = host.querySelector(".blender-row")!;
    expect(row.querySelector(".blender-file")!.textContent).toBe("hero.blend*");
    expect(row.querySelector(".blender-meta")!.textContent).toBe("4.2.1 · PID 100");
    expect(text).toContain("blender.status.versionMismatch|5.0");
    // Another session works in this instance.
    expect(text).toContain("blender.status.otherSessionTarget");
    // The saved-version distribution stays in the tooltip.
    expect(text).not.toContain("blender.status.versionCount");
    expect(host.querySelector(".blender-footer-line")!.getAttribute("title")).toContain("blender.status.versionCount|5.0|3");

    const setTarget = [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("blender.status.setTarget"));
    setTarget!.click();
    await flush();
    expect(mocks.setTarget).toHaveBeenCalledWith(WORKSPACE, "session-1", "100-5");
    expect(changed).toHaveBeenCalled();

    // A background process that has not registered yet is listed with Close.
    const close = [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("blender.status.close"));
    close!.click();
    await flush();
    expect(mocks.closeBackground).toHaveBeenCalledWith(300);
  });

  it("says a lazy service is not connected yet instead of reporting no instances", async () => {
    const connect = vi.fn();
    await mount(BlenderStatusPanel, {
      status: { enabled: true, running: false, instances: [], background: [], workspaceVersion: null, addon: null },
      workspaceRef: WORKSPACE, sessionId: "session-1", dormant: true, onConnect: connect,
    });
    expect(host.textContent).toContain("blender.status.dormantHint");
    expect(host.textContent).not.toContain("blender.status.noWorkspaceInstance");
    [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("blender.status.connect"))!.click();
    expect(connect).toHaveBeenCalled();
  });

  it("offers one add-on action: update while outdated, reload only for stale hosts", async () => {
    const base = {
      enabled: true, running: true, instances: [instance()] as never, background: [], workspaceVersion: null,
    };
    const installation = { id: "c:/b/blender.exe", displayVersion: "5.2.3" };
    mocks.installAddon.mockResolvedValue([{ installation: installation.id, version: "5.2.3", ok: true, reloaded: [100], restartNeeded: [] }]);
    mocks.reloadHosts.mockResolvedValue({ reloaded: [], restartNeeded: [100] });
    await mount(BlenderStatusPanel, {
      status: { ...base, addon: { installation, status: "outdated", outdatedInstances: [100] } },
      workspaceRef: WORKSPACE, sessionId: "session-1",
    });
    expect(host.textContent).toContain("blender.status.addonOutdated|5.2.3");
    expect(host.textContent).not.toContain("blender.status.outdatedHosts");
    [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("blender.status.update"))!.click();
    await flush();
    expect(mocks.installAddon).toHaveBeenCalledWith("c:/b/blender.exe");

    app?.unmount();
    host.remove();
    await mount(BlenderStatusPanel, {
      status: { ...base, addon: { installation, status: "up_to_date", outdatedInstances: [100] } },
      workspaceRef: WORKSPACE, sessionId: "session-1",
    });
    expect(host.textContent).toContain("blender.status.outdatedHosts|1");
    [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("blender.status.reload"))!.click();
    await flush();
    expect(mocks.reloadHosts).toHaveBeenCalled();
    // An instance the reload could not reach needs a restart; no endless Reload.
    expect(host.textContent).toContain("blender.status.restartNeeded|1");
    expect([...host.querySelectorAll("button")].some((b) => b.textContent?.includes("blender.status.reload"))).toBe(false);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it("keeps a draft's target pick local until the first message creates the session", async () => {
    const status = { enabled: true, running: true, instances: [instance()] as never, background: [] };
    const buttonText = (key: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(key));

    // A host that cannot carry the pick offers no target action in a draft.
    await mount(BlenderStatusPanel, { status, workspaceRef: WORKSPACE, sessionId: null });
    expect(buttonText("blender.status.setTarget")).toBeUndefined();
    app?.unmount();
    host.remove();

    const update = vi.fn();
    await mount(BlenderStatusPanel, {
      status, workspaceRef: WORKSPACE, sessionId: null, draftTarget: null, "onUpdate:draftTarget": update,
    });
    buttonText("blender.status.setTarget")!.click();
    await flush();
    expect(update).toHaveBeenCalledWith("100-5");
    expect(mocks.setTarget).not.toHaveBeenCalled();
    app?.unmount();
    host.remove();

    // The pick shows as the target and can be cleared again.
    update.mockClear();
    await mount(BlenderStatusPanel, {
      status, workspaceRef: WORKSPACE, sessionId: null, draftTarget: "100-5", "onUpdate:draftTarget": update,
    });
    expect(host.querySelector(".blender-tag")?.textContent).toBe("blender.status.target");
    buttonText("blender.status.clearTarget")!.click();
    await flush();
    expect(update).toHaveBeenCalledWith(null);
    expect(mocks.setTarget).not.toHaveBeenCalled();
  });
});

describe("blender_execute tool block", () => {
  it("shows the instance line and images, and highlights the code when expanded", async () => {
    const toolCall = {
      id: "t1",
      name: "blender_execute",
      arguments: JSON.stringify({ code: "import bpy\nreturn blender.look()", readonly: true, target: "background" }),
      status: "done",
      output: "[Blender 4.2.1 LTS · PID 100 · hero.blend · interactive]\nReturn value: <Image 1024x768 png viewport>",
      images: [{ data: "aGk=", mimeType: "image/png" }],
    };
    await mount(BlenderExecuteToolBlock, { toolCall });
    expect(host.textContent).toContain("4.2.1 LTS · PID 100 · hero.blend · interactive");
    expect(host.textContent).toContain("background · import bpy");
    expect(host.querySelector(".stub-images")?.textContent).toBe("1");

    host.querySelector<HTMLButtonElement>(".tool-call-header")!.click();
    await flush();
    expect(host.querySelector("pre.hljs")).not.toBeNull();
    expect(host.textContent).toContain("Return value: <Image 1024x768 png viewport>");
  });
});
