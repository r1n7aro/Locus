import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("dynamic tool loading settings", () => {
  it("adds meta-tool and direct mode to config, settings UI, and tool_load docs", () => {
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
    expect(rustConfig).toContain("fn default_dynamic_tool_loading_mode()");
    expect(rustSystem).toContain("pub fn get_dynamic_tool_loading_mode");
    expect(rustSystem).toContain("pub fn set_dynamic_tool_loading_mode");
    expect(rustApp).toContain("commands::get_dynamic_tool_loading_mode");
    expect(rustApp).toContain("commands::set_dynamic_tool_loading_mode");

    expect(systemService).toContain('export type DynamicToolLoadingMode = "metaTool" | "direct";');
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
    expect(zh).toContain('"settings.dynamicToolLoading.metaTool": "Meta-tool"');
    expect(zh).toContain('"settings.dynamicToolLoading.direct": "Direct"');
    expect(en).toContain('"settings.dynamicToolLoading.metaTool": "Meta-tool"');
    expect(en).toContain('"settings.dynamicToolLoading.direct": "Direct"');
  });
});
