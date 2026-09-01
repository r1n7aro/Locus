import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View editor tabs", () => {
  it("uses Workbench tabs while keeping View content in an embedded child WebView", () => {
    const app = read("src/App.vue");
    const windowApp = read("src/WindowApp.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const editor = read("src/components/workbench/WorkbenchViewEditor.vue");
    const tabs = read("src/components/workbench/WorkbenchEditorTabs.vue");
    const runtime = read("src-tauri/src/view.rs");

    expect(workbench).toContain("<WorkbenchEditorTabs");
    expect(workbench).toContain("<WorkbenchViewEditor");
    expect(tabs).toContain('case "view"');
    expect(editor).toContain("viewContentMount(workspaceRef, request)");
    expect(editor).toContain("hostLabel: appWindow.label");
    expect(app).toContain('<ViewHostWindow v-else-if="isViewContentWindow" embedded />');
    expect(windowApp).toContain('kind: "view-content"');
    expect(windowApp).not.toContain('kind: "view-host"');
    expect(runtime).toContain("position_view_content_child_window");
    expect(runtime).toContain("SetParent(child, Some(parent))");
  });
});
