import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("sub-agent model defaults", () => {
  it("exposes model, reasoning effort, and inherited/standard/Fast controls", () => {
    const panel = read("src/components/settings/ModelDefaults.vue");
    const types = read("src/types.ts");
    const zh = read("src/language/zh.json");

    expect(types).toContain("subagentEfforts: Record<string, EffortLevel>");
    expect(types).toContain("subagentFastModes: Record<string, boolean>");
    expect(panel).toContain("updateSubagentEffort");
    expect(panel).toContain("updateSubagentSpeed");
    expect(panel).toContain('{ value: "standard"');
    expect(panel).toContain('{ value: "fast"');
    expect(zh).toContain('"settings.models.subagentEffort": "思考强度"');
    expect(zh).toContain('"settings.models.subagentSpeedDefault": "继承速度"');
  });

  it("passes both overrides into the child-agent runtime", () => {
    const chatStore = read("src/stores/chat.ts");
    const sessionCommand = read("src-tauri/src/commands/session.rs");
    const agentRuntime = read("src-tauri/src/agent/instance/subagent_model.rs");

    expect(chatStore).toContain("subagentEfforts:");
    expect(chatStore).toContain("subagentFastModes:");
    expect(sessionCommand).toContain("instance.set_subagent_runtime_overrides(");
    expect(agentRuntime).toContain("self.resolve_subagent_effort(subagent_type)");
    expect(agentRuntime).toContain("self.resolve_subagent_fast_mode(subagent_type)");
  });
});
