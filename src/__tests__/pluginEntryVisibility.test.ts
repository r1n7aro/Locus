import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin entry visibility", () => {
  it("shows the plugin page and keeps lazy loading guarded by the entry flag", () => {
    const app = read("src/App.vue");

    expect(app).toContain("const showPluginEntry = true;");
    expect(app).toContain('{ id: "plugins", labelKey: "app.tab.plugins", visible: showPluginEntry && displaySettings.showPluginsTab }');
    expect(app).toContain('v-for="tab in visibleTopTabs"');
    expect(app).toContain('@click="uiStore.setTab(tab.id)"');
    expect(app).toContain("if (!showPluginEntry || !displaySettings.showPluginsTab || !mounted) return;");
    expect(app).toContain("v-if=\"showPluginEntry && uiStore.pluginsMounted && pluginViewComponent\"");
  });
});
