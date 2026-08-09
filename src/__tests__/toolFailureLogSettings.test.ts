import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool failure log setting", () => {
  it("is persisted, defaults off, gates recording, and is exposed in general settings", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustCommands = read("src-tauri/src/commands/workspace.rs");
    const rustAgent = read("src-tauri/src/agent/instance/mod.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const service = read("src/services/system.ts");
    const settings = read("src/components/settings/GeneralSettings.vue");

    expect(rustConfig).toContain("fn default_tool_failure_log_enabled()");
    expect(rustConfig).toContain("Arc::new(AtomicBool::new(false))");
    expect(rustConfig).toContain("pub fn tool_failure_log_enabled(&self) -> bool");
    expect(rustCommands).toContain("pub async fn get_tool_failure_log_enabled");
    expect(rustCommands).toContain("pub async fn set_tool_failure_log_enabled");
    expect(rustAgent).toContain("config.tool_failure_log_enabled()");
    expect(rustAgent).toContain("result.outcome != ToolRunOutcome::Error || !enabled");
    expect(rustApp).toContain("commands::get_tool_failure_log_enabled");
    expect(rustApp).toContain("commands::set_tool_failure_log_enabled");
    expect(service).toContain('ipcInvoke<boolean>("get_tool_failure_log_enabled")');
    expect(service).toContain('ipcInvoke<void>("set_tool_failure_log_enabled", { value })');
    expect(settings).toContain('t("settings.general.toolFailureLog")');
    expect(settings).toContain(':model-value="toolFailureLogEnabled"');
    expect(settings).toContain('v-if="toolFailureLogReady"');
  });
});
