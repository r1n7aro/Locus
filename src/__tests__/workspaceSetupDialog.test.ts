// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceSetupDialog from "../components/WorkspaceSetupDialog.vue";
import { requestWorkspaceSetup, useWorkspaceSetup } from "../composables/useWorkspaceSetup";

const mocks = vi.hoisted(() => ({ inspect: vi.fn(), save: vi.fn(), open: vi.fn() }));
vi.mock("../services/project", () => ({ inspectWorkspaceSetup: mocks.inspect, saveWorkspaceSetup: mocks.save }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));

const setup = useWorkspaceSetup();
let app: App;
let host: HTMLElement;

async function flush() { for (let i = 0; i < 6; i++) await nextTick(); }
function button(label: string): HTMLButtonElement {
  const result = Array.from(document.querySelectorAll<HTMLButtonElement>("dialog button"))
    .find(element => element.textContent?.trim() === label);
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function show(mode: "add" | "edit" = "edit") {
  const result = requestWorkspaceSetup("G:/story", mode);
  await flush();
  return { result };
}
function submit() {
  document.querySelector("dialog form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.inspect.mockResolvedValue({ isUnityProject: false, knowledgeEnabled: false, unityTestToolsEnabled: false, extraWorkdirs: [] });
  mocks.save.mockImplementation(async (_path, value) => value);
  HTMLDialogElement.prototype.showModal = function () { this.open = true; };
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(WorkspaceSetupDialog) });
  app.mount(host);
});
afterEach(() => {
  setup.cancel();
  app.unmount();
  host.remove();
});

describe("unified workspace settings panel", () => {
  it.each(["add", "edit"] as const)("saves options, directories, comments and read-only state together in %s mode", async mode => {
    const { result } = await show(mode);
    mocks.open.mockResolvedValue("F:/references");
    button("extraWorkdirs.addFolder").click();
    await flush();
    const comment = document.querySelector<HTMLInputElement>("dialog input[type=text]")!;
    comment.value = "Reference scripts";
    comment.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLButtonElement>(".workspace-setup-readonly button")!.click();
    document.querySelector<HTMLButtonElement>(".workspace-setup-option button")!.click();
    submit();
    await flush();
    expect(await result).toBe(true);
    expect(mocks.save).toHaveBeenCalledWith("G:/story", {
      isUnityProject: false, knowledgeEnabled: true, unityTestToolsEnabled: false,
      extraWorkdirs: [{ path: "F:/references", comment: "Reference scripts", readOnly: true, exists: true }],
    });
    expect(document.querySelector("dialog")).toBeNull();
  });

  it("saves the per-service modes chosen from the detected defaults", async () => {
    mocks.inspect.mockResolvedValue({
      isUnityProject: true, knowledgeEnabled: true, unityTestToolsEnabled: false, extraWorkdirs: [],
      services: [
        { service: "unity", detected: true, mode: "auto" },
        { service: "blender", detected: false, mode: "off" },
      ],
    });
    const { result } = await show("add");
    const rows = Array.from(document.querySelectorAll<HTMLElement>(".workspace-setup-service"));
    expect(rows.map(row => row.querySelector(".workspace-setup-service-name")?.textContent))
      .toEqual(["settings.tab.unity", "settings.tab.blender"]);
    const option = (row: HTMLElement, mode: string) => Array.from(row.querySelectorAll<HTMLButtonElement>("button"))
      .find(element => element.textContent?.trim() === `workspace.setup.serviceMode.${mode}`)!;
    expect(option(rows[0], "auto").getAttribute("aria-checked")).toBe("true");
    expect(document.body.textContent).toContain("workspace.setup.unityTests");

    option(rows[0], "off").click();
    await flush();
    // Unity test tools only matter while the Unity service is on.
    expect(document.body.textContent).not.toContain("workspace.setup.unityTests");
    option(rows[0], "lazy").click();
    option(rows[1], "auto").click();
    await flush();
    submit();
    await flush();
    expect(await result).toBe(true);
    expect(mocks.save.mock.calls[0][1].services).toEqual([
      { service: "unity", detected: true, mode: "lazy" },
      { service: "blender", detected: false, mode: "auto" },
    ]);
  });

  it("keeps saved attachments untouched when cancelling a removal", async () => {
    const stored = [{ path: "F:/references", comment: "Reference", readOnly: true, exists: false }];
    mocks.inspect.mockResolvedValue({ isUnityProject: false, knowledgeEnabled: true, unityTestToolsEnabled: false, extraWorkdirs: structuredClone(stored) });
    const { result } = await show();
    expect(document.querySelector(".workspace-setup-directory-missing")?.textContent).toBe("extraWorkdirs.missingBadge");
    document.querySelector<HTMLButtonElement>(".workspace-setup-remove")!.click();
    await flush();
    expect(document.querySelector(".workspace-setup-empty")).not.toBeNull();
    button("common.cancel").click();
    expect(await result).toBe(false);
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("rejects workspace descendants and duplicate directories", async () => {
    await show();
    for (const path of ["g:\\story\\", "G:/story/child"]) {
      mocks.open.mockResolvedValue(path);
      button("extraWorkdirs.addFolder").click();
      await flush();
      expect(document.querySelector('[role="alert"]')?.textContent).toBe("extraWorkdirs.insideWorkspace");
    }
    mocks.open.mockResolvedValue("F:/references");
    button("extraWorkdirs.addFolder").click();
    await flush();
    mocks.open.mockResolvedValue("f:\\references\\");
    button("extraWorkdirs.addFolder").click();
    await flush();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe("extraWorkdirs.duplicate");
    expect(setup.request.value?.setup.extraWorkdirs).toHaveLength(1);
  });

  it("preserves the entire draft after a failed save and supports retry", async () => {
    const { result } = await show();
    mocks.open.mockResolvedValue("F:/references");
    button("extraWorkdirs.addFolder").click();
    await flush();
    mocks.save.mockRejectedValueOnce(new Error("write failed"));
    submit();
    await flush();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe("write failed");
    expect(setup.request.value?.setup.extraWorkdirs).toHaveLength(1);
    submit();
    await flush();
    expect(await result).toBe(true);
  });

  it("ignores a folder picker result after the panel has been cancelled", async () => {
    const { result } = await show();
    let resolvePicker!: (path: string) => void;
    mocks.open.mockReturnValue(new Promise<string>(resolve => { resolvePicker = resolve; }));
    button("extraWorkdirs.addFolder").click();
    await flush();
    expect(button("common.save").disabled).toBe(true);
    button("common.cancel").click();
    expect(await result).toBe(false);
    const reopened = await show();
    resolvePicker("F:/old-selection");
    await flush();
    expect(setup.request.value?.setup.extraWorkdirs).toEqual([]);
    button("common.cancel").click();
    expect(await reopened.result).toBe(false);
  });
});
