import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("persistent user Agents", () => {
  it("keeps user Agents outside installer-owned resources", () => {
    const definitions = read("src-tauri/src/agent/definition.rs");
    const installer = read("src-tauri/nsis/installer.nsi");
    const bundle = read("src-tauri/tauri.conf.json");

    expect(definitions).toContain('pub const USER_AGENTS_DIR_NAME: &str = "user-agents"');
    expect(definitions).toContain("Self::scan_user_agent_dir");
    expect(bundle).toContain('"../agent": "agent/"');
    expect(bundle).not.toContain('"../user-agents"');
    expect(installer).toContain("$INSTDIR\\user-agents is user-owned");
    expect(installer).not.toContain('RMDir /r "$INSTDIR\\user-agents"');
  });

  it("ships a command-only create-agent Skill and refresh tool", () => {
    const skill = read("knowledge/skill/create-agent.md");
    const tool = JSON.parse(read("tools/agent_reload.json"));
    const builtins = read("src-tauri/src/tool/builtins/mod.rs");

    expect(skill).toContain("injectMode: none");
    expect(skill).toContain("skillSurface: command");
    expect(skill).toContain("commandTrigger: /create-agent");
    expect(skill).toContain("  - agent_reload");
    expect(tool.parameters).toEqual({
      type: "object",
      additionalProperties: false,
      properties: {},
      required: [],
    });
    expect(builtins).toContain("agent::agent_reload()");
    expect(skill).toContain("injection_config.json");
    expect(skill).toContain("tools/<tool-name>.json");
  });

  it("supports per-Agent injection and tool-description overrides", () => {
    const definitions = read("src-tauri/src/agent/definition.rs");
    const knowledgeCommands = read("src-tauri/src/commands/knowledge.rs");
    const agentView = read("src/components/AgentView.vue");
    const agentService = read("src/services/agent.ts");

    expect(definitions).toContain('agent_dir.join("tools")');
    expect(definitions).toContain("apply_tool_description_override");
    expect(definitions).toContain("apply_schema_description_overlay");
    expect(knowledgeCommands).toContain('join("injection_config.json")');
    expect(knowledgeCommands).toContain("set_agent_injection_enabled");
    expect(agentService).toContain('ipcInvoke("set_agent_injection_enabled"');
    expect(agentView).toContain("setInjectionEnabledState(item, $event)");
    expect(agentView).toContain("setSelectedInjectionEnabledState");
  });

  it("previews the active model's native tool-search surface", () => {
    const backend = read("src-tauri/src/agent/instance/mod.rs");
    const sessionCommands = read("src-tauri/src/commands/session.rs");
    const agentView = read("src/components/AgentView.vue");

    expect(sessionCommands).toContain("selected_model: Option<String>");
    expect(sessionCommands).toContain("configure_preview_lazy_tool_renderer");
    expect(backend).toContain('matches!(name.as_str(), "tool_load" | "tool_call")');
    expect(agentView).toContain("modelStore.selectedModelId");
  });

  it("removes retired built-in Agent definitions and aliases them to Unity", () => {
    const definitions = read("src-tauri/src/agent/definition.rs");
    const dev = JSON.parse(read("agent/dev/config.json"));

    expect(dev.name).toBe("Unity");
    for (const id of ["git", "knowledge", "runtime_debugger"]) {
      expect(existsSync(resolve(root, "agent", id, "config.json"))).toBe(false);
      expect(definitions).toContain(`\"${id}\"`);
    }
    expect(definitions).toContain("=> DEFAULT_AGENT_ID");
  });

  it("routes former specialized entry points through Unity", () => {
    const gitTerminal = read("src/components/GitTerminal.vue");
    const knowledgeChat = read("src/components/knowledge/KnowledgeChatPane.vue");

    expect(gitTerminal).toContain('agentId: "dev"');
    expect(gitTerminal).not.toContain('agentId: "git"');
    expect(knowledgeChat).toContain("agent.isDefault");
    expect(knowledgeChat).not.toContain('agent.id === "knowledge"');
  });
});
