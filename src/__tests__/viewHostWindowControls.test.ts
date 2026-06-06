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
    const unityBridge = read("src-tauri/src/unity_bridge/mod.rs");
    const lib = read("src-tauri/src/lib.rs");
    const config = read("src-tauri/src/config.rs");
    const commands = read("src-tauri/src/commands/view.rs");
    const systemCommands = read("src-tauri/src/commands/system.rs");
    const systemService = read("src/services/system.ts");
    const viewService = read("src/services/view.ts");
    const configRegistry = read("src-tauri/src/config_registry.rs");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const tool = read("src-tauri/src/tool/builtins/view.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(host).toContain("const alwaysOnTop = ref(false)");
    expect(host).toContain("async function syncAlwaysOnTopState()");
    expect(host).toContain("async function toggleAlwaysOnTop()");
    expect(host).toContain("appWindow.isAlwaysOnTop()");
    expect(host).toContain("appWindow.setAlwaysOnTop(alwaysOnTop.value)");
    expect(host).toContain("alwaysOnTop ? t('app.pin.unpin') : t('app.pin.pin')");
    expect(host).toContain("view-host-win-pinned");
    expect(host).toContain("viewHostRevealed(currentWindowLabel)");
    expect(host).toContain("host-reveal-owner-sync-failed");

    expect(capabilities).toContain('"view-*"');
    expect(capabilities).toContain('"core:window:allow-set-always-on-top"');
    expect(capabilities).toContain('"core:window:allow-is-always-on-top"');

    expect(runtime).toContain('const MAIN_WINDOW_LABEL: &str = "main"');
    expect(runtime).toContain("app_handle.get_webview_window(MAIN_WINDOW_LABEL)");
    expect(runtime).toContain("main_window_always_on_top");
    expect(runtime).toContain(".always_on_top(inherit_always_on_top)");
    expect(runtime).toContain(".parent(&main_window)");
    expect(runtime).toContain("view_windows_above_main: bool");
    expect(runtime).toContain("track_view_host_unity_owner");
    expect(runtime).toContain("sync_unity_owned_view_windows_for_project");
    expect(runtime).toContain("find_unity_owner_window_for_process");
    expect(runtime).toContain("GWLP_HWNDPARENT");
    expect(runtime).toContain("GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) == owner_hwnd");
    expect(runtime).toContain("GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) == 0");
    expect(runtime).toContain("SWP_NOACTIVATE");
    expect(runtime).toContain("revealed: bool");
    expect(runtime).toContain("attached_owner_hwnd: Option<isize>");
    expect(runtime).toContain("owner_sync_suspended: bool");
    expect(runtime).toContain("pub async fn mark_view_host_revealed");
    expect(runtime).toContain("focus_view_host_window_with_unity_owner_guard");
    expect(runtime).toContain("clear_view_host_unity_owner_for_focus");
    expect(runtime).toContain("restore_view_host_unity_owner_after_focus");
    expect(runtime).toContain("&& !entry.owner_sync_suspended");
    expect(unityBridge).toContain("sync_unity_owned_view_windows_for_project");

    expect(config).toContain("pub view_windows_above_main: Arc<AtomicBool>");
    expect(config).toContain("pub view_open_in_existing_window: Arc<AtomicBool>");
    expect(config).toContain("fn default_view_windows_above_main() -> Arc<AtomicBool>");
    expect(config).toContain("fn default_view_open_in_existing_window() -> Arc<AtomicBool>");
    expect(config).toContain("view_windows_above_main_defaults_to_disabled");
    expect(config).toContain("view_open_in_existing_window_defaults_to_enabled");
    expect(config).toContain("pub fn view_windows_above_main_enabled(&self) -> bool");
    expect(config).toContain("pub fn set_view_windows_above_main_enabled(&self, value: bool)");
    expect(config).toContain("pub fn view_open_in_existing_window_enabled(&self) -> bool");
    expect(config).toContain("pub fn set_view_open_in_existing_window_enabled(&self, value: bool)");
    expect(systemCommands).toContain("pub fn get_view_windows_above_main");
    expect(systemCommands).toContain("pub fn set_view_windows_above_main");
    expect(systemCommands).toContain("pub fn get_view_open_in_existing_window");
    expect(systemCommands).toContain("pub fn set_view_open_in_existing_window");
    expect(systemService).toContain("export function getViewWindowsAboveMain()");
    expect(systemService).toContain("export function setViewWindowsAboveMain(value: boolean)");
    expect(systemService).toContain("export function getViewOpenInExistingWindow()");
    expect(systemService).toContain("export function setViewOpenInExistingWindow(value: boolean)");
    expect(viewService).toContain("export function viewHostRevealed");
    expect(viewService).toContain('"view_host_revealed"');
    expect(commands).toContain("config.view_windows_above_main_enabled()");
    expect(commands).toContain("config.view_open_in_existing_window_enabled()");
    expect(commands).toContain("pub async fn view_host_revealed");
    expect(lib).toContain("commands::view_host_revealed");
    expect(tool).toContain("config.view_windows_above_main_enabled()");
    expect(tool).toContain("config.view_open_in_existing_window_enabled()");
    expect(tool).toContain(".unwrap_or(false)");
    expect(tool).toContain(".unwrap_or(true)");
    expect(configRegistry).toContain('"display.view_windows_above_main"');
    expect(configRegistry).toContain('"display.view_open_in_existing_window"');
    expect(configRegistry).toContain(".unwrap_or(false)");
    expect(configRegistry).toContain(".unwrap_or(true)");
    expect(runtime).toContain('const VIEW_HOST_TABS_MERGE_EVENT: &str = "view-host-tabs-merge"');
    expect(runtime).toContain("reusable_view_host_window_label");
    expect(runtime).toContain("merge_view_tab_into_host_window");
    expect(runtime).toContain("view_open_in_existing_window: bool");

    expect(displayPanel).toContain("const viewOpenInExistingWindow = ref(true)");
    expect(displayPanel).toContain("const viewWindowsAboveMain = ref(false)");
    expect(displayPanel).toContain("getViewOpenInExistingWindow");
    expect(displayPanel).toContain("setViewOpenInExistingWindow");
    expect(displayPanel).toContain("getViewWindowsAboveMain");
    expect(displayPanel).toContain("setViewWindowsAboveMain");
    expect(displayPanel).toContain(":model-value=\"viewOpenInExistingWindow\"");
    expect(displayPanel).toContain(":model-value=\"viewWindowsAboveMain\"");
    expect(displayPanel).toContain("settings.display.viewOpenInExistingWindow");
    expect(displayPanel).toContain("settings.display.viewWindowsAboveMain");
    expect(zh).toContain('"settings.display.viewOpenInExistingWindow": "新视图在现有窗口打开"');
    expect(zh).toContain('"settings.display.viewWindowsAboveMain": "视图窗口保持在主窗口上方"');
    expect(en).toContain('"settings.display.viewOpenInExistingWindow": "Open new Views in existing window"');
    expect(en).toContain('"settings.display.viewWindowsAboveMain": "Keep View windows above main window"');
  });
});
