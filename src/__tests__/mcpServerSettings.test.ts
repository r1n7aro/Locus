import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("mcp server (expose-to-agents) settings page", () => {
  it("wires the mcpServer category through sidebar, state unions, and i18n", () => {
    const settingsView = read("src/components/SettingsView.vue");
    const settingsState = read("src/composables/useSettingsState.ts");
    const uiStore = read("src/stores/ui.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(settingsView).toContain(
      'import McpServerSettings from "./settings/McpServerSettings.vue"',
    );
    expect(settingsView).toContain("activeCategory === 'mcpServer'");
    expect(settingsView).toContain("settings.tab.mcpServer");
    expect(settingsState).toContain('"mcpServer" |');
    expect(uiStore).toContain('"mcpServer" |');
    expect(zh).toContain('"settings.tab.mcpServer"');
    expect(en).toContain('"settings.tab.mcpServer"');
  });

  it("keeps zh/en mcpServer keys in sync", () => {
    const zh = JSON.parse(read("src/language/zh.json")) as Record<string, string>;
    const en = JSON.parse(read("src/language/en.json")) as Record<string, string>;
    const zhKeys = Object.keys(zh).filter((k) => k.startsWith("settings.mcpServer."));
    const enKeys = Object.keys(en).filter((k) => k.startsWith("settings.mcpServer."));
    expect(zhKeys.length).toBeGreaterThan(0);
    expect(zhKeys.sort()).toEqual(enKeys.sort());
  });

  it("registers the mcp server commands end to end", () => {
    const rustApp = read("src-tauri/src/lib.rs");
    const rustCommands = read("src-tauri/src/commands/mcp.rs");
    const service = read("src/services/mcpServer.ts");
    const component = read("src/components/settings/McpServerSettings.vue");

    for (const command of [
      "mcp_server_get_state",
      "mcp_server_update_settings",
      "mcp_server_regenerate_token",
      "mcp_server_tool_inventory",
      "mcp_server_integrations",
      "mcp_server_integration_apply",
      "mcp_server_integration_remove",
    ]) {
      expect(rustApp).toContain(`commands::${command}`);
      expect(rustCommands).toContain(`pub async fn ${command}`);
      expect(service).toContain(`"${command}"`);
    }
    // The page consumes the service, not raw invokes.
    expect(component).toContain("mcpServerGetState");
    expect(component).toContain("subscribeMcpServerStatus");
  });

  it("keeps the auth token out of AppConfig / config_registry", () => {
    const serverConfig = read("src-tauri/src/mcp/server/config.rs");
    const appConfig = read("src-tauri/src/config.rs");
    const configRegistry = read("src-tauri/src/config_registry.rs");

    // Token lives in its own file, never in config.json which config_query
    // exposes to agents.
    expect(serverConfig).toContain("mcp_server.json");
    expect(serverConfig).toContain("pub token: String");
    expect(appConfig).not.toContain("mcp_server_token");
    expect(configRegistry).not.toContain("mcp_server.json");
  });

  it("hardens the localhost endpoint", () => {
    const http = read("src-tauri/src/mcp/server/http.rs");
    expect(http).toContain('TcpListener::bind(("127.0.0.1", port))');
    expect(http).toContain("fn token_matches");
    expect(http).toContain("fn host_allowed");
    expect(http).toContain('req.headers().get("origin").is_some()');
    expect(http).toContain("mcp-session-id");
  });

  it("exposes only unity-domain tools with the project-awareness surface", () => {
    const tools = read("src-tauri/src/mcp/server/tools.rs");
    const component = read("src/components/settings/McpServerSettings.vue");

    expect(tools).toContain('"unity_project_info"');
    expect(tools).toContain('"unity_execute"');
    expect(tools).toContain('"unity_yaml_read"');
    expect(tools).toContain('"code_hover"');
    // File/shell tools stay internal.
    expect(tools).not.toContain('"bash"');
    expect(tools).not.toContain('"write"');
    // Project awareness: initialize instructions + external status switch.
    expect(tools).toContain("pub async fn build_instructions");
    expect(tools).toContain("ensure_editor_status");
    // The settings page renders per-tool switches (BaseSwitch, no native selects).
    expect(component).toContain("BaseSwitch");
    expect(component).not.toContain("<select");
  });

  it("ships one-click integrations for the common harnesses", () => {
    const install = read("src-tauri/src/mcp/server/install.rs");
    for (const id of ["claude_code", "codex", "opencode", "cursor", "gemini"]) {
      expect(install).toContain(`"${id}"`);
    }
    expect(install).toContain(".claude.json");
    expect(install).toContain("config.toml");
    expect(install).toContain("toml_edit");
    expect(install).toContain("refusing to modify");
  });
});
