import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Unity Play Mode tool", () => {
  it("exposes an explicit play/edit contract", () => {
    const definition = JSON.parse(read("tools/unity_set_play_mode.json"));

    expect(definition.parameters.properties.mode.enum).toEqual(["play", "edit"]);
    expect(definition.parameters.required).toContain("mode");
    expect(definition.description).toContain("unity_execute");
    expect(definition.description).toContain("wait until the requested mode is reached");
  });

  it("registers the tool across Agent and MCP execution paths", () => {
    const registry = read("src-tauri/src/tool/builtins/mod.rs");
    const builtin = read("src-tauri/src/tool/builtins/unity.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const mcp = read("src-tauri/src/mcp/server/tools.rs");

    expect(registry).toContain("registry.register_builtin(unity::unity_set_play_mode())");
    expect(builtin).toContain('name: "unity_set_play_mode".to_string()');
    expect(builtin).toContain("set_editor_status(&project_path, requested_status)");
    expect(agent).toContain('tc.name == "unity_set_play_mode"');
    expect(agent).toContain('"unity_set_play_mode",\n                tool_call_id');
    expect(mcp).toContain('"unity_set_play_mode"');
  });

  it("offers the tool to agents that can execute Unity code", () => {
    const dev = JSON.parse(read("agent/dev/config.json"));
    const runtimeDebugger = JSON.parse(read("agent/runtime_debugger/config.json"));
    const wiki = JSON.parse(read("agent/wiki/config.json"));

    for (const agent of [dev, runtimeDebugger, wiki]) {
      expect(agent.tools).toContain("unity_execute");
      expect(agent.tools).toContain("unity_set_play_mode");
    }
  });
});
