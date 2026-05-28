import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View host window controls", () => {
  it("exposes the same always-on-top control as the main window", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const capabilities = read("src-tauri/capabilities/default.json");
    const runtime = read("src-tauri/src/view.rs");
    const config = read("src-tauri/src/config.rs");
    const commands = read("src-tauri/src/commands/view.rs");
    const systemCommands = read("src-tauri/src/commands/system.rs");
    const systemService = read("src/services/system.ts");
    const configRegistry = read("src-tauri/src/config_registry.rs");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const tool = read("src-tauri/src/tool/builtins/view.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(host).toContain("const alwaysOnTop = ref(false)");
    expect(host).toContain("async function toggleAlwaysOnTop()");
    expect(host).toContain("appWindow.setAlwaysOnTop(alwaysOnTop.value)");
    expect(host).toContain("alwaysOnTop ? t('app.pin.unpin') : t('app.pin.pin')");
    expect(host).toContain("view-host-win-pinned");

    expect(capabilities).toContain('"view-*"');
    expect(capabilities).toContain('"core:window:allow-set-always-on-top"');

    expect(runtime).toContain('const MAIN_WINDOW_LABEL: &str = "main"');
    expect(runtime).toContain("app_handle.get_webview_window(MAIN_WINDOW_LABEL)");
    expect(runtime).toContain(".parent(&main_window)");
    expect(runtime).toContain("view_windows_above_main: bool");

    expect(config).toContain("pub view_windows_above_main: Arc<AtomicBool>");
    expect(config).toContain("fn default_view_windows_above_main() -> Arc<AtomicBool>");
    expect(config).toContain("view_windows_above_main_defaults_to_disabled");
    expect(config).toContain("pub fn view_windows_above_main_enabled(&self) -> bool");
    expect(config).toContain("pub fn set_view_windows_above_main_enabled(&self, value: bool)");
    expect(systemCommands).toContain("pub fn get_view_windows_above_main");
    expect(systemCommands).toContain("pub fn set_view_windows_above_main");
    expect(systemService).toContain("export function getViewWindowsAboveMain()");
    expect(systemService).toContain("export function setViewWindowsAboveMain(value: boolean)");
    expect(commands).toContain("config.view_windows_above_main_enabled()");
    expect(tool).toContain("config.view_windows_above_main_enabled()");
    expect(tool).toContain(".unwrap_or(false)");
    expect(configRegistry).toContain('"display.view_windows_above_main"');
    expect(configRegistry).toContain(".unwrap_or(false)");

    expect(displayPanel).toContain("const viewWindowsAboveMain = ref(false)");
    expect(displayPanel).toContain("getViewWindowsAboveMain");
    expect(displayPanel).toContain("setViewWindowsAboveMain");
    expect(displayPanel).toContain(":model-value=\"viewWindowsAboveMain\"");
    expect(displayPanel).toContain("settings.display.viewWindowsAboveMain");
    expect(zh).toContain('"settings.display.viewWindowsAboveMain": "视图窗口保持在主窗口上方"');
    expect(en).toContain('"settings.display.viewWindowsAboveMain": "Keep View windows above main window"');
  });
});
