import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("subagent tool name", () => {
  it("registers and dispatches subagent as the model-facing tool", () => {
    const registry = read("src-tauri/src/tool/mod.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");

    expect(registry).toContain("pub fn register_subagent_tool");
    expect(registry).toContain('name: "subagent".to_string()');
    expect(registry).not.toContain('name: "task".to_string()');
    expect(agent).toContain('tc.name == "subagent"');
    expect(agent).toContain("self.execute_subagent(");
  });

  it("normalizes legacy configuration and permissions", () => {
    const definitions = read("src-tauri/src/agent/definition.rs");
    const app = read("src-tauri/src/lib.rs");
    const settings = read("src/composables/useSettingsState.ts");

    expect(definitions).toContain('"task" => "subagent"');
    expect(app).toContain('initial_tool_perms.get("task")');
    expect(app).toContain('initial_tool_perms.insert("subagent".to_string(), mode)');
    expect(settings).toContain("normalized.subagent = normalized.task");
    expect(settings).toContain("delete normalized.task");
  });

  it("ships built-in agents and plan guidance with the new name", () => {
    const dev = JSON.parse(read("agent/dev/config.json")) as { tools: string[] };
    const knowledge = JSON.parse(read("agent/knowledge/config.json")) as { tools: string[] };
    const planReminder = read("prompt/plan-reminder.md");

    for (const tools of [dev.tools, knowledge.tools]) {
      expect(tools).toContain("subagent");
      expect(tools).not.toContain("task");
    }
    expect(planReminder).toContain("with the subagent tool");
  });
});
