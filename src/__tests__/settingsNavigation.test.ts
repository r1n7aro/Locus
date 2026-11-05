// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const apps: App[] = [];

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
});

afterEach(() => {
  apps.splice(0).forEach((app) => app.unmount());
  document.body.replaceChildren();
});

async function mount(component: unknown, props: Record<string, unknown> = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({ render: () => h(component as never, props) });
  apps.push(app);
  app.mount(host);
  await nextTick();
  return host;
}

describe("settings navigation table", () => {
  it("has unique pages and sections in known groups", async () => {
    const { SETTINGS_GROUPS, SETTINGS_PAGES, isSettingsPageId } = await import("../components/settings/nav");
    const groups = new Set(SETTINGS_GROUPS.map((group) => group.id));
    const pageIds = SETTINGS_PAGES.map((page) => page.id);
    expect(new Set(pageIds).size).toBe(pageIds.length);
    for (const page of SETTINGS_PAGES) {
      expect(groups.has(page.group)).toBe(true);
      const sections = page.sections.map((section) => section.id);
      expect(new Set(sections).size).toBe(sections.length);
    }
    expect(isSettingsPageId("workspace")).toBe(true);
    expect(isSettingsPageId("experimental")).toBe(false);
  });

  it("finds pages, sections and single settings, and lists experimental ones", async () => {
    const { buildSettingsSearchIndex, searchSettings } = await import("../components/settings/nav");
    const labels: Record<string, string> = {
      "settings.tab.workspace": "Workspace",
      "settings.section.worktree": "Worktree",
      "settings.badge.experimental": "实验",
      "settings.item.chat.history.pageSize.label": "Messages per load",
      "settings.item.chat.history.pageSize.desc": "Messages loaded when opening a session",
      "settings.item.providers.claudeCode.enabled.label": "Claude Code CLI",
    };
    const translate = (key: string) => labels[key] ?? key;
    const index = buildSettingsSearchIndex(
      translate,
      ["chat.history.pageSize", "providers.claudeCode.enabled", "workspace.worktree.enabled"],
      new Set(["providers.claudeCode.enabled", "workspace.worktree.enabled"]),
    );

    expect(searchSettings(index, "workspace", translate)).toContainEqual(
      { pageId: "workspace", sectionId: null, settingLabel: null },
    );
    expect(searchSettings(index, "per load", translate)).toEqual([
      { pageId: "chat", sectionId: "history", settingLabel: "Messages per load" },
    ]);
    // Setting keys are searchable too.
    expect(searchSettings(index, "history.pageSize", translate)[0]?.pageId).toBe("chat");
    const experimental = searchSettings(index, "实验", translate).map((result) => `${result.pageId}/${result.sectionId}`);
    expect(experimental).toEqual(expect.arrayContaining(["providers/claudeCode", "workspace/worktree"]));
    expect(experimental).not.toContain("chat/history");
    expect(searchSettings(index, "  ", translate)).toEqual([]);
  });
});

describe("setting rows", () => {
  it("renders a switch, segmented choices, numbers and lists from the setting value", async () => {
    const store = await import("../composables/useSettingsStore");
    await store.setSettingValue("chat.messages.compactToolCalls", true);
    await store.setSettingValue("workbench.open.view", "tab");
    await store.setSettingValue("notifications.sound.volume", 50);
    await store.setSettingValue("workspace.explorer.hiddenDirectories", [".git", "dist"]);
    const { default: SettingRow } = await import("../components/settings/SettingRow.vue");

    const switchHost = await mount(SettingRow, { settingKey: "chat.messages.compactToolCalls" });
    expect(switchHost.querySelector(".setting-row-label")?.textContent).toContain("Collapse completed tool calls");
    (switchHost.querySelector('[role="switch"]') as HTMLElement).click();
    await nextTick();
    expect(store.settingValue("chat.messages.compactToolCalls")).toBe(false);

    const segmentHost = await mount(SettingRow, {
      settingKey: "workbench.open.view",
      options: [{ value: "tab", label: "Tab" }, { value: "window", label: "Window" }],
    });
    const windowButton = [...segmentHost.querySelectorAll("button")].find((button) => button.textContent?.includes("Window"));
    windowButton!.click();
    await nextTick();
    expect(store.settingValue("workbench.open.view")).toBe("window");

    const numberHost = await mount(SettingRow, { settingKey: "notifications.sound.volume" });
    const number = numberHost.querySelector<HTMLInputElement>('input[type="number"]')!;
    number.value = "70";
    number.dispatchEvent(new Event("change"));
    expect(store.settingValue("notifications.sound.volume")).toBe(70);

    const listHost = await mount(SettingRow, { settingKey: "workspace.explorer.hiddenDirectories" });
    const list = listHost.querySelector<HTMLInputElement>('input[type="text"]')!;
    expect(list.value).toBe(".git, dist");
    list.value = ".git，node_modules; build";
    list.dispatchEvent(new Event("change"));
    expect(store.settingValue("workspace.explorer.hiddenDirectories")).toEqual([".git", "node_modules", "build"]);
  });

  it("shows the experimental badge and a rejected write inline", async () => {
    const { installFakeSettingsBackend } = await import("./fixtures/settingsBackend");
    const backend = await installFakeSettingsBackend({ "workspace.worktree.enabled": false });
    const store = await import("../composables/useSettingsStore");
    await store.ensureSettingsLoaded();
    const { default: SettingRow } = await import("../components/settings/SettingRow.vue");
    const host = await mount(SettingRow, { settingKey: "workspace.worktree.enabled", experimental: true });
    expect(host.querySelector(".settings-badge-experimental")).not.toBeNull();

    backend.failNextWrite("Invalid value for 'workspace.worktree.enabled'");
    (host.querySelector('[role="switch"]') as HTMLElement).click();
    await backend.settle();
    await nextTick();
    expect(host.querySelector(".setting-row-error")?.textContent).toContain("Invalid value");
    expect(store.settingValue("workspace.worktree.enabled")).toBe(false);
    backend.uninstall();
  });
});
