import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("mcp settings page", () => {
  it("wires the mcp category through sidebar, state unions, and i18n", () => {
    const settingsView = read("src/components/SettingsView.vue");
    const settingsState = read("src/composables/useSettingsState.ts");
    const uiStore = read("src/stores/ui.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(settingsView).toContain("import McpSettings from \"./settings/McpSettings.vue\"");
    expect(settingsView).toContain("activeCategory === 'mcp'");
    expect(settingsView).toContain("settings.tab.mcp");
    expect(settingsState).toContain('"mcp" |');
    expect(uiStore).toContain('"mcp" |');
    expect(zh).toContain('"settings.tab.mcp"');
    expect(en).toContain('"settings.tab.mcp"');
  });

  it("keeps zh/en mcp keys in sync", () => {
    const zh = JSON.parse(read("src/language/zh.json")) as Record<string, string>;
    const en = JSON.parse(read("src/language/en.json")) as Record<string, string>;
    const zhKeys = Object.keys(zh).filter((k) => k.startsWith("settings.mcp."));
    const enKeys = Object.keys(en).filter((k) => k.startsWith("settings.mcp."));
    expect(zhKeys.length).toBeGreaterThan(0);
    expect(zhKeys.sort()).toEqual(enKeys.sort());
  });

  it("ships the connect-software builtin skill", () => {
    const skill = read("knowledge/skill/connect-software.md");
    expect(skill).toContain("id: kd_skill_builtin_connect_software");
    // Dual surface: auto recall plus the /connect command.
    expect(skill).toContain("skillSurface: both");
    expect(skill).toContain("commandTrigger: /connect");
    expect(skill).toContain("commandEnabled: true");
    expect(skill).toContain("mcp_servers.json");
    expect(skill).toContain("- bash");
    // The upgraded skill covers research, CLI/HTTP channels, and persistence.
    expect(skill).toContain("- web_fetch");
    expect(skill).toContain("registry.modelcontextprotocol.io");
    expect(skill).toContain("memory/integrations/");
  });

  it("registers the mcp commands on the backend", () => {
    const rustApp = read("src-tauri/src/lib.rs");
    const rustCommands = read("src-tauri/src/commands/mod.rs");
    const service = read("src/services/mcp.ts");

    for (const command of [
      "mcp_servers_get",
      "mcp_servers_upsert",
      "mcp_servers_remove",
      "mcp_server_test",
      "mcp_get_status",
      "mcp_server_set_enabled",
      "mcp_import_scan",
      "mcp_import_apply",
      "mcp_server_wire_tools",
    ]) {
      expect(rustApp).toContain(`commands::${command}`);
      expect(service).toContain(`"${command}"`);
    }
    expect(rustCommands).toContain("mod mcp;");
    expect(rustCommands).toContain("pub use mcp::*;");
  });

  it("supports both transports with per-server tool governance", () => {
    const config = read("src-tauri/src/mcp/config.rs");
    const settings = read("src/components/settings/McpSettings.vue");
    const service = read("src/services/mcp.ts");

    // HTTP transport is accepted by normalize (stdio-only rejection is gone).
    expect(config).toContain("McpTransport::Http => {");
    expect(config).not.toContain("HTTP transport is not supported yet");
    // Allow/deny filtering happens against raw MCP tool names.
    expect(config).toContain("pub fn tool_exposed");
    // The form exposes transport, load mode, auto-restart and the lists.
    expect(settings).toContain("formTransport");
    expect(settings).toContain("formLoadMode");
    expect(settings).toContain("formAutoRestart");
    expect(settings).toContain("toolAllowlist");
    expect(settings).toContain("toolDenylist");
    // Presets ship with the software-side setup notes.
    expect(service).toContain("MCP_PRESETS");
    expect(service).toContain("blender-mcp");
    expect(service).toContain("http://127.0.0.1:3845/mcp");
  });

  it("keeps the import flow read-only until applied and disabled after", () => {
    const importer = read("src-tauri/src/mcp/import.rs");
    const commands = read("src-tauri/src/commands/mcp.rs");
    const settings = read("src/components/settings/McpSettings.vue");

    expect(importer).toContain("claude_desktop_config.json");
    expect(importer).toContain(".claude.json");
    expect(importer).toContain("mcp.json");
    // Imports must never arrive enabled.
    expect(importer).toContain("enabled: false");
    expect(commands).toContain("incoming.enabled = false");
    expect(settings).toContain("mcpImportScan");
    expect(settings).toContain("mcpImportApply");
  });

  it("wires robustness: ping keepalive, cancellation, auto-restart, list_changed", () => {
    const manager = read("src-tauri/src/mcp/manager.rs");
    const stdio = read("src-tauri/src/mcp/stdio.rs");
    const http = read("src-tauri/src/mcp/http.rs");
    const instance = read("src-tauri/src/agent/instance/mod.rs");

    expect(manager).toContain("fn ensure_ping_loop");
    expect(manager).toContain("PING_FAILURES_BEFORE_DEAD");
    expect(manager).toContain("fn handle_server_notification");
    expect(manager).toContain("notifications/tools/list_changed");
    expect(manager).toContain("fn auto_restart_loop");
    expect(manager).toContain("MAX_CRASHES_IN_WINDOW");
    // Cancellation posts notifications/cancelled on both transports.
    expect(stdio).toContain("notifications/cancelled");
    expect(http).toContain("notifications/cancelled");
    // The agent layer maps the cancel marker to the interrupted result and
    // forwards MCP images through the native image channel.
    expect(instance).toContain("MCP_CALL_CANCELLED");
    expect(instance).toContain("describe_images_as_placeholder");
  });

  it("groups mcp tools separately and defaults them to lazy loading", () => {
    const agentView = read("src/components/AgentView.vue");
    const instance = read("src-tauri/src/agent/instance/mod.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(agentView).toContain('"tools:mcp"');
    expect(agentView).toContain("toolItemDisplayTitle(item)");
    expect(instance).toContain('"mcp"');
    expect(instance).toContain("mcpServerName");
    // Lazy default: the mcp branch in default_tool_load_mode returns Lazy.
    expect(instance).toContain("return ToolLoadMode::Lazy;");
    expect(zh).toContain('"agent.mcpTools"');
    expect(en).toContain('"agent.mcpTools"');
  });

  it("wires the chat-bar mcp status badge", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const service = read("src/services/mcp.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    // Badge identity: StatusId/StatusIcon unions, icon map, conditional item.
    expect(indicators).toContain('"hotReload" | "mcp"');
    expect(indicators).toContain("mcp: Plug");
    expect(indicators).toContain("mcpStatus.value.length > 0");
    // Per-server toggle in the popover.
    expect(indicators).toContain("toggleMcpServer(server)");
    expect(indicators).toContain("mcpServerSetEnabled");
    // Component-local subscription lifecycle.
    expect(indicators).toContain("subscribeMcpStatus");
    expect(indicators).toContain("mcpUnsubscribe?.()");
    expect(service).toContain('"mcp-status"');
    expect(zh).toContain('"chat.status.mcp.title"');
    expect(en).toContain('"chat.status.mcp.title"');
  });
});
