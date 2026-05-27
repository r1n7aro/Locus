import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("app close behavior settings", () => {
  it("adds exit and tray behavior to config, backend close handling, and general settings", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustSystem = read("src-tauri/src/commands/system.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const systemService = read("src/services/system.ts");
    const generalSettings = read("src/components/settings/GeneralSettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustConfig).toContain("pub enum AppCloseBehavior");
    expect(rustConfig).toContain("MinimizeToTray");
    expect(rustConfig).toContain("fn default_close_behavior()");
    expect(rustConfig).toContain("Self::Exit");
    expect(rustSystem).toContain("pub fn get_close_behavior");
    expect(rustSystem).toContain("pub fn set_close_behavior");

    expect(rustApp).toContain("const MAIN_TRAY_ID: &str = \"locus-main-tray\";");
    expect(rustApp).toContain("TrayIconBuilder::with_id(MAIN_TRAY_ID)");
    expect(rustApp).toContain("AppCloseBehavior::MinimizeToTray");
    expect(rustApp).toContain("hide_main_window_to_tray(window);");
    expect(rustApp).toContain("window.hide()");

    expect(systemService).toContain("export type AppCloseBehavior = \"exit\" | \"minimizeToTray\";");
    expect(systemService).toContain("export async function getCloseBehavior()");
    expect(systemService).toContain("export function setCloseBehavior");

    expect(generalSettings).toContain("getCloseBehavior");
    expect(generalSettings).toContain("setCloseBehavior");
    expect(generalSettings).toContain("settings.general.closeBehaviorExit");
    expect(generalSettings).toContain("settings.general.closeBehaviorTray");
    expect(generalSettings).toContain("BaseSegmented");

    expect(zh).toContain('"settings.general.closeBehavior": "关闭行为"');
    expect(zh).toContain('"settings.general.closeBehaviorExit": "直接退出应用"');
    expect(zh).toContain('"settings.general.closeBehaviorTray": "最小化到托盘"');
    expect(en).toContain('"settings.general.closeBehavior": "Close Behavior"');
    expect(en).toContain('"settings.general.closeBehaviorExit": "Exit App"');
    expect(en).toContain('"settings.general.closeBehaviorTray": "Minimize to Tray"');
  });
});
