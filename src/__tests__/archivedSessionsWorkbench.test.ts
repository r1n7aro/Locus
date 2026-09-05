import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("archived sessions workspace", () => {
  it("exposes archived sessions as a hidden workspace system node", () => {
    const store = read("src/stores/workspaceExplorer.ts");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const tree = read("src-tauri/src/workspace_tree.rs");

    expect(store).toContain('const ARCHIVED_SYSTEM_RESOURCE_ID = "archived";');
    expect(store).toContain('resourceId: ARCHIVED_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain('labelKey: "app.tab.archived"');
    expect(workbench).toContain('section: "archived"');
    expect(tree).toContain('const DEFAULT_HIDDEN_SYSTEM_RESOURCE_ID: &str = "archived";');
    expect(tree).toContain('resource_id == DEFAULT_HIDDEN_SYSTEM_RESOURCE_ID');
  });

  it("uses a split workbench editor with the shared transcript renderer", () => {
    const editor = read("src/components/workbench/WorkbenchArchivedSessionsEditor.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const settings = read("src/components/SettingsView.vue");

    expect(editor).toContain("listArchivedCheckoutSessions(workspaceRef)");
    expect(editor).toContain('class="archived-sidebar"');
    expect(editor).toContain('class="archived-conversation"');
    expect(editor).toContain("<ChatTranscript");
    expect(editor).not.toContain("<ChatComposer");
    expect(editor).not.toContain("<RichChatInput");
    expect(workbench).toContain("<WorkbenchArchivedSessionsEditor");
    expect(settings).not.toContain("ArchivedSessionsSettings");
    expect(settings).not.toContain("activeCategory === 'archived'");
  });
});
