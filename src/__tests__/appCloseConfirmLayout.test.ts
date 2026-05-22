import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("app close confirm flow", () => {
  it("routes main window close requests through a running task confirmation", () => {
    const app = read("src/App.vue");
    const systemService = read("src/services/system.ts");
    const rustSystem = read("src-tauri/src/commands/system.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustApp).toContain("const MAIN_WINDOW_CLOSE_REQUESTED_EVENT: &str = \"locus-main-window-close-requested\";");
    expect(rustApp).toContain("window.emit(MAIN_WINDOW_CLOSE_REQUESTED_EVENT, ())");
    expect(rustSystem).toContain("pub fn request_app_exit(app_handle: AppHandle)");
    expect(rustSystem).toContain("app_handle.exit(0);");
    expect(systemService).toContain("export const APP_CLOSE_REQUESTED_EVENT = \"locus-main-window-close-requested\";");
    expect(systemService).toContain("export function requestAppExit(): Promise<void>");

    expect(app).toContain("const appCloseConfirmOpen = ref(false);");
    expect(app).toContain("const appCloseRunningTaskCount = ref(0);");
    expect(app).toContain("appCloseRunningTaskCount.value = runningTaskCount;");
    expect(app).toContain("appCloseConfirmOpen.value = true;");
    expect(app).toContain("await requestAppExit();");
    expect(app).toContain("await listen<void>(APP_CLOSE_REQUESTED_EVENT");
    expect(app).toContain("class=\"workspace-switch-overlay app-close-overlay\"");
    expect(app).toContain("class=\"workspace-switch-dialog app-close-dialog\"");
    expect(app).toContain('variant="danger"');
    expect(app).toContain('t("app.close.runningConfirmAction")');

    expect(zh).toContain('"app.close.runningConfirmTitle": "关闭 Locus"');
    expect(zh).toContain('"app.close.runningConfirmAction": "仍要关闭"');
    expect(en).toContain('"app.close.runningConfirmTitle": "Close Locus"');
    expect(en).toContain('"app.close.runningConfirmAction": "Close anyway"');
  });
});
