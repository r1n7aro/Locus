// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { installFakeSettingsBackend, type FakeSettingsBackend } from "./fixtures/settingsBackend";
import { readLegacyFrontendSettings } from "../composables/settingsLegacy";

const BOOT_CACHE = "locus-settings-boot-cache";
const MIGRATED = "locus-settings-migrated-v1";

let backend: FakeSettingsBackend | null = null;

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
});

afterEach(() => {
  backend?.uninstall();
  backend = null;
});

function bootCache(): Record<string, unknown> | undefined {
  const raw = localStorage.getItem(BOOT_CACHE);
  return raw ? JSON.parse(raw).values : undefined;
}

describe("legacy localStorage preferences", () => {
  it("maps every stored preference to its setting key", () => {
    localStorage.setItem("locus-locale", "zh");
    localStorage.setItem("locus-theme-preference", "light");
    localStorage.setItem("locus-unity-embed-theme-preference", "bogus");
    localStorage.setItem("locus-display-settings", JSON.stringify({
      compactToolCalls: false,
      sessionMessagePageSize: 240,
      fonts: { ui: "Inter", prose: "" },
      workspaceSectionVisibility: { collab: false },
      viewOpenOverrides: { "p:hero": "window" },
      showCollabSidebar: false,
    }));
    localStorage.setItem("locus-keyboard-shortcuts", JSON.stringify({
      newChat: { ctrl: true, alt: true, key: "n" },
      globalSearch: { ctrl: true, shift: true, key: "f" },
    }));
    localStorage.setItem("locus-new-session-settings", JSON.stringify({ agentMode: "fixed", defaultAgentId: "unity" }));
    localStorage.setItem("locus-global-search-settings", JSON.stringify({ fileName: false, enabled: "yes" }));
    localStorage.setItem("locus-app-update-channel", "experimental");

    expect(readLegacyFrontendSettings()).toEqual({
      "appearance.language": "zh",
      "appearance.theme.main": "light",
      "chat.messages.compactToolCalls": false,
      "chat.history.pageSize": 240,
      "appearance.fonts.ui": "Inter",
      "appearance.fonts.prose": "",
      "workspace.tree.sections.collab": false,
      "workbench.open.viewOverrides": { "p:hero": "window" },
      // The platform default stays unset so it keeps following the platform.
      "shortcuts.newChat": "ctrl+alt+n",
      "agent.newSession.agentMode": "fixed",
      "agent.newSession.defaultAgentId": "unity",
      "workspace.search.sources.fileName": false,
      "updates.channel": "experimental",
    });
  });
});

describe("settings store without a desktop backend", () => {
  it("starts from the legacy keys, then keeps writes in the boot cache", async () => {
    localStorage.setItem("locus-theme-preference", "light");
    const store = await import("../composables/useSettingsStore");
    expect(store.settingValue("appearance.theme.main")).toBe("light");

    await store.setSettingValue("chat.messages.compactToolCalls", false);
    expect(bootCache()).toMatchObject({ "chat.messages.compactToolCalls": false });

    vi.resetModules();
    localStorage.setItem("locus-theme-preference", "dark");
    const reloaded = await import("../composables/useSettingsStore");
    // The boot cache wins over the legacy keys once it exists.
    expect(reloaded.settingValue("appearance.theme.main")).toBe("light");
    expect(reloaded.settingValue("chat.messages.compactToolCalls")).toBe(false);
  });
});

describe("settings store with the desktop backend", () => {
  it("imports legacy preferences once, then follows the backend snapshot", async () => {
    localStorage.setItem("locus-locale", "zh");
    localStorage.setItem("locus-display-settings", JSON.stringify({ hideThinkingBlocks: false }));
    backend = await installFakeSettingsBackend({ "appearance.theme.main": "system" });
    const store = await import("../composables/useSettingsStore");
    // First frame: the legacy values, before the backend answers.
    expect(store.settingValue("appearance.language")).toBe("zh");
    await store.ensureSettingsLoaded();

    expect(backend.imports).toEqual([{ "appearance.language": "zh", "chat.messages.hideThinkingBlocks": false }]);
    expect(localStorage.getItem(MIGRATED)).not.toBeNull();
    expect(store.settingValue("appearance.theme.main")).toBe("system");
    expect(bootCache()).toMatchObject({
      "appearance.language": "zh",
      "appearance.theme.main": "system",
      "chat.messages.hideThinkingBlocks": false,
    });

    backend.uninstall();
    vi.resetModules();
    backend = await installFakeSettingsBackend({ "appearance.language": "en" });
    const second = await import("../composables/useSettingsStore");
    expect(second.settingValue("appearance.language")).toBe("zh");
    await second.ensureSettingsLoaded();
    expect(backend.imports).toEqual([]);
    expect(second.settingValue("appearance.language")).toBe("en");
  });

  it("applies changes from other windows and the agent", async () => {
    backend = await installFakeSettingsBackend({ "chat.history.pageSize": 120 });
    const store = await import("../composables/useSettingsStore");
    await store.ensureSettingsLoaded();
    backend.change("chat.history.pageSize", 400);
    expect(store.settingValue("chat.history.pageSize")).toBe(400);
    expect(bootCache()).toMatchObject({ "chat.history.pageSize": 400 });
  });

  it("updates optimistically, coalesces rapid writes and adopts the stored form", async () => {
    backend = await installFakeSettingsBackend({ "appearance.fonts.ui": "" });
    const store = await import("../composables/useSettingsStore");
    await store.ensureSettingsLoaded();

    const writes = ["I", "In", "Int", "Inter"].map((font) => store.setSettingValue("appearance.fonts.ui", font));
    expect(store.settingValue("appearance.fonts.ui")).toBe("Inter");
    await Promise.all(writes);
    // The first write goes out at once; the rest collapse into the latest value.
    expect(backend.writes.map((batch) => batch[0]!.value)).toEqual(["I", "Inter"]);
    expect(store.settingValue("appearance.fonts.ui")).toBe("Inter");
  });

  it("sends the workspace a change is made from", async () => {
    backend = await installFakeSettingsBackend({ "unity.codeAnalysis.csharpLsp": false });
    const store = await import("../composables/useSettingsStore");
    await store.ensureSettingsLoaded();
    const workspaceRef = { checkoutId: "checkout-1", expectedGeneration: 3 };
    await store.setSettingValue("unity.codeAnalysis.csharpLsp", true, workspaceRef);
    await store.setSettingValue("unity.codeAnalysis.csharpLsp", false);
    expect(backend.writeScopes).toEqual([workspaceRef, null]);
  });

  it("rolls back a rejected write and reports it per key", async () => {
    backend = await installFakeSettingsBackend({ "notifications.sound.volume": 50 });
    const store = await import("../composables/useSettingsStore");
    await store.ensureSettingsLoaded();
    const volume = store.useSetting("notifications.sound.volume", store.readNumber(50, 0, 100));
    backend.failNextWrite("Invalid value for 'notifications.sound.volume'");
    const pending = volume.set(80);
    expect(volume.value.value).toBe(80);
    expect(volume.busy.value).toBe(true);
    await expect(pending).rejects.toMatchObject({ message: expect.stringContaining("Invalid value") });
    expect(volume.value.value).toBe(50);
    expect(volume.error.value).toContain("Invalid value");
    expect(volume.busy.value).toBe(false);
  });
});
