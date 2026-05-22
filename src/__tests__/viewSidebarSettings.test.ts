import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View sidebar settings", () => {
  it("lets display settings control the session list View section", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const chatView = read("src/components/ChatView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("showViewsInSessionPanel: boolean;");
    expect(displaySettings).toContain("showViewsInSessionPanel: true,");
    expect(displayPanel).toContain(":model-value=\"display.showViewsInSessionPanel\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('showViewsInSessionPanel', $event)\"");
    expect(chatView).toContain(":show-views=\"displaySettings.showViewsInSessionPanel\"");

    expect(sessionPanel).toContain("const showSessionViews = computed(() => props.showViews !== false)");
    expect(sessionPanel).toContain("view-package-reloaded");
    expect(sessionPanel).toContain("class=\"sp-view-resize\"");
    expect(sessionPanel).toContain("onViewResizeMouseDown");
    expect(sessionPanel).toContain("visibleViewEntries");
    expect(sessionPanel).toContain("@contextmenu.prevent.stop=\"openViewContextMenu($event, entry.row)\"");
    expect(sessionPanel).toContain("class=\"sp-view-row-shell\"");
    expect(sessionPanel).toContain("@drop=\"onViewFolderDrop(entry.row, $event)\"");
    expect(sessionPanel).toContain("class=\"sp-view-create-actions\"");
    expect(sessionPanel).toContain("@click=\"closeViewCreateFolder\"");
    expect(sessionPanel).not.toContain("sp-view-refresh");

    expect(zh).toContain('"settings.display.showViewsInSessionPanel": "会话列表中显示视图"');
    expect(en).toContain('"settings.display.showViewsInSessionPanel": "Show Views in session list"');
    expect(zh).toContain('"view.tree.createFolder": "新建文件夹"');
    expect(en).toContain('"view.tree.createFolder": "New Folder"');
  });

  it("renders View list icons from manifest icon configuration", () => {
    const icons = read("src/components/icons/locusViewIcons.ts");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const viewPage = read("src/components/ViewPackageView.vue");
    const service = read("src/services/view.ts");
    const createTool = read("tools/view_create.json");

    expect(icons).toContain("export const LOCUS_VIEW_ICON_LIBRARY");
    expect(icons).toContain("export function resolveLocusViewIcon");
    expect(sessionPanel).toContain(":icon=\"resolveLocusViewIcon(entry.row.node.view?.icon)\"");
    expect(viewPage).toContain(":icon=\"resolveLocusViewIcon(view.icon)\"");
    expect(viewPage).toContain("view-package-reloaded");
    expect(service).toContain("icon?: string | null;");
    expect(createTool).toContain("\"icon\"");
    expect(createTool).toContain("\"InspectionPanel\"");
  });

  it("keeps View tree operations package-level and disk-backed", () => {
    const service = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(service).toContain("export interface ViewTreeSnapshot");
    expect(service).toContain("export function viewTree");
    expect(service).toContain("export function viewCreateFolder");
    expect(service).toContain("export function viewDeleteEntry");
    expect(service).toContain("export function viewMoveEntry");
    expect(commands).toContain("pub async fn view_tree");
    expect(commands).toContain("pub async fn view_create_folder");
    expect(commands).toContain("pub async fn view_delete_entry");
    expect(commands).toContain("pub async fn view_move_entry");
    expect(lib).toContain("commands::view_tree");
    expect(lib).toContain("commands::view_create_folder");
    expect(lib).toContain("commands::view_delete_entry");
    expect(lib).toContain("commands::view_move_entry");
    expect(runtime).toContain("std::fs::remove_dir_all(&target)");
    expect(runtime).toContain("std::fs::rename(&source, &target)");
  });
});
