import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("dynamic tool loading settings", () => {
  it("adds native, meta-tool, and direct mode to config, settings UI, and tool_load docs", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustSystem = read("src-tauri/src/commands/system.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const systemService = read("src/services/system.ts");
    const settingsState = read("src/composables/useSettingsState.ts");
    const apiProviders = read("src/components/settings/ApiProviders.vue");
    const toolLoad = read("tools/tool_load.json");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustConfig).toContain("pub enum DynamicToolLoadingMode");
    expect(rustConfig).toContain("MetaTool");
    expect(rustConfig).toContain("Direct");
    expect(rustConfig).toContain("Native");
    expect(rustConfig).toContain("fn default_dynamic_tool_loading_mode()");
    expect(rustSystem).toContain("pub fn get_dynamic_tool_loading_mode");
    expect(rustSystem).toContain("pub fn set_dynamic_tool_loading_mode");
    expect(rustApp).toContain("commands::get_dynamic_tool_loading_mode");
    expect(rustApp).toContain("commands::set_dynamic_tool_loading_mode");

    expect(systemService).toContain(
      'export type DynamicToolLoadingMode = "metaTool" | "direct" | "native";',
    );
    expect(systemService).toContain("export async function getDynamicToolLoadingMode()");
    expect(systemService).toContain("export function setDynamicToolLoadingMode");

    expect(settingsState).toContain("dynamicToolLoadingMode");
    expect(settingsState).toContain("loadDynamicToolLoadingMode");
    expect(settingsState).toContain("setDynamicToolLoadingMode");
    expect(apiProviders).toContain("settings.dynamicToolLoading.title");
    expect(apiProviders).toContain("dynamicToolLoadingOptions");
    expect(apiProviders).toContain("BaseSegmented");

    expect(toolLoad).toContain("meta-tool mode");
    expect(toolLoad).toContain("direct mode");
    expect(zh).toContain('"settings.dynamicToolLoading.native": "Native"');
    expect(zh).toContain('"settings.dynamicToolLoading.metaTool": "Meta-tool"');
    expect(zh).toContain('"settings.dynamicToolLoading.direct": "Direct"');
    expect(en).toContain('"settings.dynamicToolLoading.native": "Native"');
    expect(en).toContain('"settings.dynamicToolLoading.metaTool": "Meta-tool"');
    expect(en).toContain('"settings.dynamicToolLoading.direct": "Direct"');
  });

  it("wires the Anthropic endpoint deferred-loading switch end to end, default on", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustSystem = read("src-tauri/src/commands/system.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const rustInstance = read("src-tauri/src/agent/instance/mod.rs");
    const systemService = read("src/services/system.ts");
    const settingsState = read("src/composables/useSettingsState.ts");
    const settingsView = read("src/components/SettingsView.vue");
    const apiProviders = read("src/components/settings/ApiProviders.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    // Config field defaults to enabled and persists a manual opt-out.
    expect(rustConfig).toContain("pub anthropic_native_lazy_enabled");
    expect(rustConfig).toContain("fn default_anthropic_native_lazy_enabled()");
    expect(rustConfig).toContain("AtomicBool::new(true)");
    expect(rustSystem).toContain("pub fn get_anthropic_native_lazy_enabled");
    expect(rustSystem).toContain("pub fn set_anthropic_native_lazy_enabled");
    expect(rustApp).toContain("commands::get_anthropic_native_lazy_enabled");
    expect(rustApp).toContain("commands::set_anthropic_native_lazy_enabled");

    // The renderer gate consumes the switch: opted-out endpoints must not
    // resolve the Anthropic native renderer.
    expect(rustInstance).toContain("anthropic_endpoint_lazy_enabled");
    expect(rustInstance).toContain("anthropic_native_lazy_enabled_from_app_handle");

    expect(systemService).toContain("export function getAnthropicNativeLazyEnabled()");
    expect(systemService).toContain("export function setAnthropicNativeLazyEnabled");
    expect(settingsState).toContain("anthropicNativeLazyEnabled");
    expect(settingsState).toContain("loadAnthropicNativeLazyEnabled");
    expect(settingsState).toContain("setAnthropicNativeLazyEnabled");
    expect(settingsView).toContain(":anthropic-native-lazy-enabled=");
    expect(settingsView).toContain("@update:anthropic-native-lazy-enabled=");
    expect(apiProviders).toContain("settings.anthropic.nativeLazyTitle");
    expect(apiProviders).toContain("update:anthropicNativeLazyEnabled");

    expect(zh).toContain('"settings.anthropic.nativeLazyTitle"');
    expect(zh).toContain('"settings.anthropic.nativeLazyDesc"');
    expect(en).toContain('"settings.anthropic.nativeLazyTitle"');
    expect(en).toContain('"settings.anthropic.nativeLazyDesc"');
  });
});
