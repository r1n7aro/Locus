import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin status notice", () => {
  it("routes plugin attention through the global error notice style and top warning strip", () => {
    const app = read("src/App.vue");
    const projectStore = read("src/stores/project.ts");

    expect(projectStore).toContain('const PLUGIN_STATUS_NOTICE_OPERATION = "unity-plugin-status";');
    expect(projectStore).toContain('notificationStore.addNotice("error", pluginStatusLabel(status)');
    expect(projectStore).toContain("replaceOperation: true");
    expect(projectStore).toContain("notificationStore.clearByOperation(PLUGIN_STATUS_NOTICE_OPERATION)");
    expect(app).toContain('class="tab-plugin-warn"');
    expect(app).toContain('class="tab-plugin-icon"');
    expect(app).toContain("var(--status-danger-bg)");
    expect(app).toContain("var(--status-danger-fg)");
    expect(app).toContain("border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color) 28%);");
  });

  it("keeps the top tabs single-line when the plugin notice is visible", () => {
    const app = read("src/App.vue");

    expect(app).toMatch(/\.tab-item\s*\{[\s\S]*flex:\s*0 0 auto;[\s\S]*white-space:\s*nowrap;/);
    expect(app).toMatch(/\.tab-plugin-warn\s*\{[\s\S]*flex:\s*0 0 auto;[\s\S]*white-space:\s*nowrap;/);
    expect(app).toMatch(/\.workspace-selector\s*\{[\s\S]*flex:\s*0 1 320px;[\s\S]*width:\s*320px;[\s\S]*min-width:\s*120px;[\s\S]*max-width:\s*320px;/);
    expect(app).toMatch(/\.workspace-btn\s*\{[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;[\s\S]*max-width:\s*none;/);
    expect(app).toMatch(/\.ws-name\s*\{[\s\S]*flex:\s*1;[\s\S]*min-width:\s*0;[\s\S]*text-overflow:\s*ellipsis;/);
  });
});
