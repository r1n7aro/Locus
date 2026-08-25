import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("general settings debug mode switch", () => {
  it("renders the switch only after debug mode is hydrated", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const permissions = read("src/services/permissions.ts");

    expect(source).toContain("const initialDebugMode = getCachedDebugMode();");
    expect(source).toContain("const debugReady = ref(initialDebugMode !== null);");
    expect(source).toContain("if (!debugReady.value) return t(\"common.loading\");");
    expect(source).toContain('v-if="debugReady"');
    expect(source).toContain('class="debug-toggle-placeholder"');
    expect(permissions).toContain("let cachedDebugMode: boolean | null = null;");
    expect(permissions).toContain("export function getCachedDebugMode(): boolean | null");
  });

  it("exposes DevTools only through debug mode in release builds", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const runtime = read("src/services/tauriRuntime.ts");
    const main = read("src/main.ts");
    const cargo = read("src-tauri/Cargo.toml");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain('import BaseButton from "../ui/BaseButton.vue";');
    expect(source).toContain('v-if="debugEnabled"');
    expect(source).toContain('@click="toggleDevtools"');
    expect(source).toContain('t("settings.general.devtoolsToggle")');
    expect(runtime).toContain("type DevtoolsAccessResolver");
    expect(runtime).toContain("event.stopImmediatePropagation();");
    expect(runtime).toContain("if (!enabled) return;");
    expect(main).toContain("installTauriDevtoolsHotkeys(getDebugMode);");
    expect(cargo).toContain('features = ["tray-icon", "devtools"]');
    expect(zh).toContain('"settings.general.devtoolsToggle": "打开 / 关闭 DevTools"');
    expect(en).toContain('"settings.general.devtoolsToggle": "Toggle DevTools"');
  });
});
