// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  confirm: vi.fn(),
  getState: vi.fn(),
  save: vi.fn(),
}));

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm }));
vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({ subscribe: () => Promise.resolve(() => {}) }),
}));
vi.mock("../services/remoteAccess", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../services/remoteAccess")>();
  return {
    ...actual,
    getRemoteAccessState: mocks.getState,
    saveRemoteAccessSettings: mocks.save,
  };
});
vi.mock("../components/settings/RemotePairingDialog.vue", () => ({
  default: { name: "RemotePairingDialog", render: () => h("div") },
}));

import RemoteAccessSettings from "../components/settings/RemoteAccessSettings.vue";
import type { RemoteAccessSettingsInput, RemoteAccessState } from "../services/remoteAccess";

function remoteState(enabled: boolean): RemoteAccessState {
  return {
    settings: {
      enabled,
      port: 47831,
      loopback: true,
      interfaces: [],
      hostnames: [],
      requireConfirmation: true,
      tokenIdleDays: 30,
      appDownload: true,
      desktopId: "desktop",
    },
    desktopName: "pc",
    running: enabled,
    listeners: [],
    listenerErrors: [],
    lastError: null,
    interfaces: [],
    fingerprint: null,
    devices: [],
    connections: 0,
    pairing: null,
    tailscaleServeCommand: "",
    appPackage: null,
  };
}

let app: App;
let host: HTMLElement;

async function flush() { for (let i = 0; i < 8; i++) await nextTick(); }

async function mount() {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(RemoteAccessSettings) });
  app.mount(host);
  await flush();
}

function enableSwitch(): HTMLButtonElement {
  return host.querySelector<HTMLButtonElement>('[aria-label="remoteAccess.enable"]')!;
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.save.mockImplementation(async (input: RemoteAccessSettingsInput) => remoteState(input.enabled));
});

afterEach(() => {
  app?.unmount();
  host?.remove();
});

describe("RemoteAccessSettings", () => {
  it("shows the experimental badge", async () => {
    mocks.getState.mockResolvedValue(remoteState(false));
    await mount();
    expect(host.querySelector(".settings-badge-experimental")?.textContent?.trim())
      .toBe("settings.badge.experimental");
  });

  it("warns before turning remote access on and keeps it off when cancelled", async () => {
    mocks.getState.mockResolvedValue(remoteState(false));
    mocks.confirm.mockResolvedValue(false);
    await mount();

    enableSwitch().click();
    await flush();

    expect(mocks.confirm).toHaveBeenCalledTimes(1);
    expect(mocks.confirm).toHaveBeenCalledWith(
      "remoteAccess.enableWarning.message",
      expect.objectContaining({ title: "remoteAccess.enableWarning.title", kind: "warning" }),
    );
    expect(mocks.save).not.toHaveBeenCalled();
    expect(enableSwitch().getAttribute("aria-checked")).toBe("false");
  });

  it("turns remote access on after the warning is accepted", async () => {
    mocks.getState.mockResolvedValue(remoteState(false));
    mocks.confirm.mockResolvedValue(true);
    await mount();

    enableSwitch().click();
    await flush();

    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ enabled: true }));
    expect(enableSwitch().getAttribute("aria-checked")).toBe("true");
  });

  it("turns remote access off without a warning", async () => {
    mocks.getState.mockResolvedValue(remoteState(true));
    await mount();

    enableSwitch().click();
    await flush();

    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ enabled: false }));
  });
});
