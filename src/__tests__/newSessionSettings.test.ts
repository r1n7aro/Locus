// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { installFakeSettingsBackend } from "./fixtures/settingsBackend";

const key = "locus-new-session-settings";

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
});

describe("new session settings", () => {
  it.each([null, "{invalid", "null", '{"defaultAgentId":42}'])(
    "uses the workspace default for absent or invalid settings: %s",
    async (saved) => {
      if (saved !== null) localStorage.setItem(key, saved);
      const { useNewSessionSettings } = await import("../composables/useNewSessionSettings");
      expect(useNewSessionSettings().state.defaultAgentId).toBe("");
    },
  );

  it("persists an explicit Agent and the workspace-default choice across reloads", async () => {
    const first = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    first.setDefaultAgentId(" simple ");
    vi.resetModules();
    const second = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    expect(second.state.defaultAgentId).toBe("simple");
    second.setDefaultAgentId("");
    vi.resetModules();
    expect((await import("../composables/useNewSessionSettings")).useNewSessionSettings().state.defaultAgentId).toBe("");
  });

  it("remembers the last choice by default and seeds it from a pre-migration Agent", async () => {
    const fresh = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    expect(fresh.state.agentMode).toBe("remember");
    localStorage.setItem(key, JSON.stringify({ defaultAgentId: "simple" }));
    vi.resetModules();
    const migrated = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    expect(migrated.state).toMatchObject({ agentMode: "remember", legacyAgentId: "simple" });
    migrated.setAgentMode("workspace");
    vi.resetModules();
    const reloaded = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    expect(reloaded.state.agentMode).toBe("workspace");
    reloaded.setDefaultAgentId("unity");
    expect(reloaded.state).toMatchObject({ agentMode: "fixed", defaultAgentId: "unity" });
    reloaded.setDefaultAgentId("");
    expect(reloaded.state.agentMode).toBe("workspace");
  });

  it("syncs detached windows and clears the saved preference on reset", async () => {
    localStorage.setItem(key, JSON.stringify({ defaultAgentId: "legacy" }));
    const backend = await installFakeSettingsBackend();
    const settings = (await import("../composables/useNewSessionSettings")).useNewSessionSettings();
    await backend.settle();
    backend.change("agent.newSession.defaultAgentId", "simple");
    backend.change("agent.newSession.agentMode", "fixed");
    expect(settings.state).toMatchObject({ agentMode: "fixed", defaultAgentId: "simple" });
    settings.reset();
    expect(settings.state).toMatchObject({ agentMode: "remember", defaultAgentId: "", legacyAgentId: "" });
    expect(localStorage.getItem(key)).toBeNull();
    await backend.settle();
    expect(backend.values).toMatchObject({
      "agent.newSession.agentMode": "remember",
      "agent.newSession.defaultAgentId": "",
    });
    backend.uninstall();
  });
});
