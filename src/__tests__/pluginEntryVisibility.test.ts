import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin entry visibility", () => {
  it("keeps the plugin page behind a release visibility flag", () => {
    const app = read("src/App.vue");

    expect(app).toContain("const showPluginEntry = false;");
    expect(app).toMatch(/v-if="showPluginEntry"[\s\S]*@click="uiStore\.setTab\('plugins'\)"/);
    expect(app).toContain("if (!showPluginEntry || !mounted) return;");
    expect(app).toContain("v-if=\"showPluginEntry && uiStore.pluginsMounted && pluginViewComponent\"");
  });
});
