import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View sidebar settings", () => {
  it("keeps the bundled /view skill display name short", () => {
    const manifest = JSON.parse(read("skills/view/skill.json")) as {
      name: string;
      command?: { trigger?: string };
    };
    const skill = read("skills/view/SKILL.md");

    expect(manifest.name).toBe("View");
    expect(manifest.command?.trigger).toBe("/view");
    expect(skill).toContain("# View");
    expect(skill).not.toContain("# View Package");
  });

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
    expect(sessionPanel).toContain("const DEFAULT_VIEW_SECTION_RATIO = 1 / 3;");
    expect(sessionPanel).toContain("flex: 1 1 66.667%;");
    expect(sessionPanel).toContain("flex: 0 0 33.333%;");
    expect(sessionPanel).toContain("view-package-reloaded");
    expect(sessionPanel).toContain("class=\"sp-view-resize\"");
    expect(sessionPanel).toContain(":icon=\"HelpCircle\"");
    expect(sessionPanel).toContain("class=\"sp-view-help-overlay\"");
    expect(sessionPanel).toContain("class=\"sp-view-help-dialog\"");
    expect(sessionPanel).toContain("class=\"sp-view-help-footer\"");
    expect(sessionPanel).toContain("<BaseButton size=\"md\" @click=\"closeViewHelp\">");
    expect(sessionPanel).toContain("view.list.helpFeatureTitle");
    expect(sessionPanel).toContain("view.list.helpCreate");
    expect(sessionPanel).not.toContain("view.list.helpUseCases");
    expect(sessionPanel).toContain("onViewResizeMouseDown");
    expect(sessionPanel).toContain("visibleViewEntries");
    expect(sessionPanel).toContain("interface ViewPointerDragState");
    expect(sessionPanel).toContain("@pointerdown=\"onViewPointerDown(entry.row, $event)\"");
    expect(sessionPanel).toContain("sp-view-pointer-dragging");
    expect(sessionPanel).toContain("@contextmenu.prevent.stop=\"openViewContextMenu($event, entry.row)\"");
    expect(sessionPanel).toContain("async function revealViewContextLocation");
    expect(sessionPanel).toContain("projectStore.openDirInFileExplorer(targetPath)");
    expect(sessionPanel).toContain("t('view.action.reveal')");
    expect(sessionPanel).toContain("class=\"sp-view-row-shell\"");
    expect(sessionPanel).toContain("@drop=\"onViewFolderDrop(entry.row, $event)\"");
    expect(sessionPanel).toContain("class=\"sp-view-create-actions\"");
    expect(sessionPanel).toContain("class=\"sp-view-rename-input\"");
    expect(sessionPanel).toContain("beginRenameViewEntry");
    expect(sessionPanel).toContain("@click=\"closeViewCreateFolder\"");
    expect(sessionPanel).not.toContain("sp-view-refresh");

    expect(zh).toContain('"settings.display.showViewsInSessionPanel": "会话列表中显示视图"');
    expect(en).toContain('"settings.display.showViewsInSessionPanel": "Show Views in session list"');
    expect(zh).toContain('"view.list.helpLabel": "视图（实验性）"');
    expect(zh).toContain('"view.list.helpBody": "视图是实验性功能，用于通过 Locus 自身的前端创建、打开和运行项目编辑器。Agent 可以把 Vue 前端界面、运行脚本和 Unity 属性数据组合为一个 View package，并在 Locus 中作为独立工具运行。"');
    expect(zh).toContain('"view.list.helpCreate": "在会话输入 /view 加上需求后，Agent 会进入 View workflow，按需求创建或更新 View package。生成后的视图会出现在当前工作区的 Locus/View 列表中，点击名称即可打开。"');
    expect(en).toContain('"view.list.helpLabel": "Views (Experimental)"');
    expect(en).toContain('"view.list.helpBody": "Views are experimental tools for creating, opening, and running project editors through Locus\'s own frontend. The agent can combine a Vue frontend, runtime scripts, and Unity property data into a View package that runs inside Locus as a standalone tool."');
    expect(en).toContain('"view.list.helpCreate": "Type /view with a request in a session and the agent enters the View workflow to create or update a View package. Generated Views appear in the current workspace Locus/View list and open when selected."');
    expect(zh).toContain('"view.tree.createFolder": "新建文件夹"');
    expect(zh).toContain('"view.tree.rename": "重命名"');
    expect(en).toContain('"view.tree.createFolder": "New Folder"');
    expect(en).toContain('"view.tree.rename": "Rename"');
    expect(zh).toContain('"view.action.reveal": "在文件游览器中显示"');
    expect(en).toContain('"view.action.reveal": "Show in File Explorer"');
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
    expect(viewPage).toContain(":icon=\"resolveLocusViewIcon(entry.row.node.view?.icon)\"");
    expect(viewPage).toContain("view-package-reloaded");
    expect(service).toContain("icon?: string | null;");
    expect(createTool).toContain("\"icon\"");
    expect(createTool).toContain("\"InspectionPanel\"");
    expect(createTool).toContain("\"serialized-table\"");
  });

  it("renders the View management page as a directory tree", () => {
    const viewPage = read("src/components/ViewPackageView.vue");

    expect(viewPage).toContain("viewTree()");
    expect(viewPage).toContain("viewCreateFolder");
    expect(viewPage).toContain("viewDeleteEntry");
    expect(viewPage).toContain("viewRenameEntry");
    expect(viewPage).toContain("viewMoveEntry");
    expect(viewPage).toContain("view-tree-changed");
    expect(viewPage).toContain("visibleViewEntries");
    expect(viewPage).toContain("class=\"view-tree-row-shell\"");
    expect(viewPage).toContain("@pointerdown=\"onTreePointerDown(entry.row, $event)\"");
    expect(viewPage).toContain(":data-view-node-kind=\"entry.row.node.kind\"");
    expect(viewPage).toContain("@contextmenu.prevent.stop=\"openTreeContextMenu($event, entry.row)\"");
    expect(viewPage).toContain("@drop=\"onTreeFolderDrop(entry.row, $event)\"");
    expect(viewPage).toContain("v-else-if=\"entry.row.node.kind === 'folder' || entry.row.depth > 0\"");
    expect(viewPage).toContain("class=\"view-tree-row-actions\"");
    expect(viewPage).toContain("@click.stop=\"openTreeView(entry.row)\"");
    expect(viewPage).toContain(".view-tree-row-shell:hover .view-tree-row-actions");
    expect(viewPage).toContain("class=\"view-tree-rename-input\"");
    expect(viewPage).toContain("beginRenameFromContext");
    expect(viewPage).toContain("class=\"view-tree-create-actions\"");
    expect(viewPage).toContain("view.tree.deleteConfirmMessage");
    expect(viewPage).toContain("return !!node.relPath.trim();");
  });

  it("closes View context menus around delete confirmation", () => {
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const viewPage = read("src/components/ViewPackageView.vue");

    expect(sessionPanel).toContain("interface ViewDeleteConfirmState");
    expect(sessionPanel).toContain("viewDeleteConfirm.value = {\n    x: menu.x,\n    y: menu.y,\n    node: menu.node,\n  };\n  closeViewContextMenu();");
    expect(sessionPanel).toContain("} finally {\n    closeViewDeleteConfirm();\n  }");
    expect(sessionPanel).toContain('v-if="viewDeleteConfirm"');
    expect(sessionPanel).toContain("viewDeleteConfirm.node.label");

    expect(viewPage).toContain("interface ViewDeleteConfirmState");
    expect(viewPage).toContain("deleteConfirm.value = {\n    x: menu.x,\n    y: menu.y,\n    node: menu.node,\n  };\n  closeContextMenu();");
    expect(viewPage).toContain("} finally {\n    closeDeleteConfirm();\n  }");
    expect(viewPage).toContain('v-if="deleteConfirm"');
    expect(viewPage).toContain("deleteConfirm.node.label");
  });

  it("lets view_create create temporary packages outside the visible View tree", () => {
    const service = read("src/services/view.ts");
    const createTool = read("tools/view_create.json");
    const runtime = read("src-tauri/src/view.rs");
    const tool = read("src-tauri/src/tool/builtins/view.rs");

    expect(service).toContain("temporary?: boolean;");
    expect(createTool).toContain("\"temporary\"");
    expect(createTool).toContain("do not appear in view_list");
    expect(runtime).toContain("temporary_views_root_for_workspace");
    expect(runtime).toContain("parse_view_create_request");
    expect(runtime).toContain("create_view_sync_with_scope");
    expect(runtime).toContain("resolve_view_package_root");
    expect(tool).toContain("parse_view_create_request(args)");
    expect(tool).toContain("create_view_sync_with_scope(&working_dir, request, temporary)");
  });

  it("keeps View tree operations display-path based and package-aware", () => {
    const service = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(service).toContain("export interface ViewTreeSnapshot");
    expect(service).toContain("displayPath: string;");
    expect(service).toContain("export function viewTree");
    expect(service).toContain("export function viewCreateFolder");
    expect(service).toContain("export function viewDeleteEntry");
    expect(service).toContain("export function viewRenameEntry");
    expect(service).toContain("export function viewMoveEntry");
    expect(commands).toContain("pub async fn view_tree");
    expect(commands).toContain("pub async fn view_create_folder");
    expect(commands).toContain("pub async fn view_delete_entry");
    expect(commands).toContain("pub async fn view_rename_entry");
    expect(commands).toContain("pub async fn view_move_entry");
    expect(lib).toContain("commands::view_tree");
    expect(lib).toContain("commands::view_create_folder");
    expect(lib).toContain("commands::view_delete_entry");
    expect(lib).toContain("commands::view_rename_entry");
    expect(lib).toContain("commands::view_move_entry");
    expect(runtime).toContain("pub display_path: Option<String>");
    expect(runtime).toContain("VIEW_TREE_METADATA_REL_PATH");
    expect(runtime).toContain("set_view_manifest_display_path");
    expect(runtime).toContain("pub fn rename_view_entry_sync");
    expect(runtime).toContain("std::fs::remove_dir_all(root)");
    expect(runtime).not.toContain("std::fs::rename(&source, &target)");
  });

  it("adds View package import and export actions", () => {
    const viewPage = read("src/components/ViewPackageView.vue");
    const service = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(viewPage).toContain('import { open, save } from "@tauri-apps/plugin-dialog"');
    expect(viewPage).toContain("async function importViewPackage");
    expect(viewPage).toContain("async function exportViewPackage");
    expect(viewPage).toContain("viewImportPackage({");
    expect(viewPage).toContain("viewExportPackage({");
    expect(viewPage).toContain('filters: [{ name: t("view.archive.filter"), extensions: ["zip"] }]');
    expect(viewPage).toContain("view.import.imported");
    expect(viewPage).toContain("view.export.exported");
    expect(service).toContain("export interface ViewExportPackageRequest");
    expect(service).toContain("export interface ViewImportPackageRequest");
    expect(service).toContain('ipcInvoke<string>("view_export_package"');
    expect(service).toContain('ipcInvoke<ViewPackageImportResult>("view_import_package"');
    expect(commands).toContain("pub async fn view_export_package");
    expect(commands).toContain("pub async fn view_import_package");
    expect(lib).toContain("commands::view_export_package");
    expect(lib).toContain("commands::view_import_package");
    expect(runtime).toContain("pub fn export_view_package_sync");
    expect(runtime).toContain("pub fn import_view_package_sync");
    expect(runtime).toContain("zip::ZipWriter::new");
    expect(runtime).toContain("zip::ZipArchive::new");
    expect(runtime).toContain("View package id already exists");
    expect(runtime).toContain("is_view_internal_path(Path::new(&package_rel_path))");
    expect(zh).toContain('"view.action.import": "导入"');
    expect(zh).toContain('"view.action.export": "导出"');
    expect(en).toContain('"view.action.import": "Import"');
    expect(en).toContain('"view.action.export": "Export"');
  });
});
