import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Collaboration sidebar display setting", () => {
  it("hides the sidebar by default and exposes a display setting", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const collabView = read("src/components/CollabView.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("showCollabSidebar: boolean;");
    expect(displaySettings).toContain("showCollabSidebar: false,");
    expect(displayPanel).toContain(':model-value="display.showCollabSidebar"');
    expect(displayPanel).toContain("setDisplay('showCollabSidebar', $event)");

    expect(collabView).toContain('v-if="displaySettings.showCollabSidebar"');
    expect(collabView).toContain('v-if="displaySettings.showCollabSidebar && !sidebarCollapsed"');

    expect(zh).toContain('"settings.display.showCollabSidebar": "显示协作侧栏"');
    expect(en).toContain('"settings.display.showCollabSidebar": "Show Collaboration sidebar"');
  });
});
