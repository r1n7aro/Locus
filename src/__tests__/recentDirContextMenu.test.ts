import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("recent directory context menu", () => {
  it("offers file explorer open and list removal actions from the workspace dropdown", () => {
    const app = read("src/App.vue");
    const projectStore = read("src/stores/project.ts");
    const projectService = read("src/services/project.ts");
    const rustWorkspace = read("src-tauri/src/commands/workspace.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(app).toContain("const recentDirContextMenu = ref<RecentDirContextMenu | null>(null);");
    expect(app).toContain("@contextmenu.prevent.stop=\"openRecentDirContextMenu($event, dir)\"");
    expect(app).toContain("openContextRecentDirInFileExplorer");
    expect(app).toContain("removeContextRecentDir");
    expect(app).toContain("class=\"recent-dir-ctx-menu\"");
    expect(app).toContain('t("common.openInFileExplorer")');
    expect(app).toContain('t("app.dir.removeRecent")');

    expect(projectStore).toContain("async function removeRecentDir(path: string)");
    expect(projectStore).toContain("async function openDirInFileExplorer(path: string)");
    expect(projectService).toContain('ipcInvoke<string[]>("remove_recent_dir"');
    expect(projectService).toContain('ipcInvoke<void>("open_dir_in_file_explorer"');

    expect(rustWorkspace).toContain("pub async fn remove_recent_dir");
    expect(rustWorkspace).toContain("pub async fn open_dir_in_file_explorer");
    expect(rustApp).toContain("commands::remove_recent_dir");
    expect(rustApp).toContain("commands::open_dir_in_file_explorer");

    expect(zh).toContain('"app.dir.removeRecent": "从列表移除"');
    expect(en).toContain('"app.dir.removeRecent": "Remove from list"');
  });
});
