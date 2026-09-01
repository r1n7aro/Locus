import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View Workbench routing", () => {
  it("removes standalone View window routing and settings", () => {
    const app = read("src/App.vue");
    const windowApp = read("src/WindowApp.vue");
    const runtime = read("src-tauri/src/view.rs");
    const commands = read("src-tauri/src/commands/view.rs");
    const lib = read("src-tauri/src/lib.rs");
    const config = read("src-tauri/src/config.rs");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(app).not.toContain("isViewHostWindowLocation");
    expect(app).not.toContain('v-else-if="isViewHostWindow"');
    expect(windowApp).not.toContain('kind: "view-host"');
    expect(runtime).toContain("pub async fn open_view_in_workbench(");
    expect(runtime).toContain("VIEW_WORKBENCH_OPEN_EVENT");
    expect(commands).not.toContain("pub async fn view_detach_tab");
    expect(commands).not.toContain("pub async fn view_host_pool_prepare");
    expect(commands).not.toContain("pub async fn view_open_inspector_tab");
    expect(lib).not.toContain("commands::view_detach_tab");
    expect(lib).not.toContain("commands::view_host_pool_prepare");
    expect(lib).not.toContain("commands::view_open_inspector_tab");
    expect(config).not.toContain("view_windows_above_main");
    expect(config).not.toContain("view_open_in_existing_window");
    expect(displayPanel).not.toContain("viewOpenInExistingWindow");
    expect(displayPanel).not.toContain("viewWindowsAboveMain");
    expect(workbench).toContain("VIEW_WORKBENCH_OPEN_EVENT");
    expect(workbench).toContain("WORKBENCH_INSPECTOR_OPEN_EVENT");
  });
});
