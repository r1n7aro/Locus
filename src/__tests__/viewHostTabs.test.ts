import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View host tabs", () => {
  it("renders draggable View tabs and registers merged tab hosts", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const service = read("src/services/view.ts");
    const commands = read("src-tauri/src/commands/view.rs");
    const runtime = read("src-tauri/src/view.rs");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(host).toContain('const VIEW_HOST_TABS_MERGE_EVENT = "view-host-tabs-merge"');
    expect(host).toContain('const VIEW_HOST_TABS_SELECT_EVENT = "view-host-tabs-select"');
    expect(host).toContain('role="tablist"');
    expect(host).toContain('class="view-host-tabs"');
    expect(host).toContain("data-tauri-drag-region");
    expect(host).toContain('class="view-host-tab"');
    expect(host).toContain('@pointerdown="startTabDrag($event, tab.id)"');
    expect(host).toContain('@click="onTabClick($event, tab.id)"');
    expect(host).toContain(".view-host-tabs {\n  -webkit-app-region: drag;");
    expect(host).toContain(".view-host-tab {\n  -webkit-app-region: no-drag;");
    expect(host).toContain("background: color-mix(in srgb, var(--panel-bg) 64%, var(--sidebar-bg) 36%)");
    expect(host).toContain("box-shadow: inset 0 2px 0 var(--accent-color)");
    expect(host).toContain("cursor: grab");
    expect(host).toContain("TauriWindow.getAll()");
    expect(host).toContain("findTabDropTargetAt");
    expect(host).toContain("mergeTabIntoWindow");
    expect(host).toContain("isCurrentTabBandAt");
    expect(host).toContain("detachTab");
    expect(host).toContain("viewDetachTab({");
    expect(host).not.toContain("appWindow.setPosition");
    expect(host).toContain("viewSetTabHost({ hostLabel: currentWindowLabel, viewIds })");

    expect(service).toContain("export interface ViewSetTabHostRequest");
    expect(service).toContain("export interface ViewDetachTabRequest");
    expect(service).toContain('ipcInvoke<void>("view_set_tab_host"');
    expect(service).toContain('ipcInvoke<ViewRunResult>("view_detach_tab"');
    expect(commands).toContain("pub async fn view_set_tab_host");
    expect(commands).toContain("pub async fn view_detach_tab");
    expect(runtime).toContain("fn view_tab_hosts()");
    expect(runtime).toContain("set_view_tab_host_sync");
    expect(runtime).toContain("detach_view_tab_window");
    expect(runtime).toContain("detached_view_window_label");
    expect(runtime).toContain("active_view_window_label(app_handle, view_id)");
    expect(runtime).toContain("emit_view_host_tab_select(app_handle, &label, view_id)");
    expect(capabilities).not.toContain('"core:window:allow-set-position"');
    expect(capabilities).toContain('"core:window:allow-get-all-windows"');
    expect(capabilities).toContain('"core:window:allow-cursor-position"');
  });
});
