import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Unity Console log tool", () => {
  it("declares single-level and OR multi-level filtering with a bounded result limit", () => {
    const definition = JSON.parse(read("tools/unity_get_console_log.json"));

    expect(definition.parameters.properties.level.enum).toEqual(["error", "warn", "info"]);
    expect(definition.parameters.properties.levels).toMatchObject({
      type: "array",
      minItems: 1,
      uniqueItems: true,
      items: { type: "string", enum: ["error", "warn", "info"] },
    });
    expect(definition.parameters.properties.limit).toMatchObject({
      minimum: 1,
      maximum: 200,
      default: 50,
    });
    expect(definition.description).toContain("ctx.GetConsoleLog(level, limit)");
    expect(definition.description).toContain("OR semantics");
  });

  it("uses one Unity-side reader for the Agent tool and unity_execute context", () => {
    const consoleBridge = read("locus_unity/Editor/LocusBridge.Console.cs");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const executeContext = read(
      "locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs",
    );
    const builtin = read("src-tauri/src/tool/builtins/unity.rs");
    const executeDefinition = JSON.parse(read("tools/unity_execute.json"));

    expect(consoleBridge).toContain("BuildConsoleLogResult");
    expect(consoleBridge).toContain("GetEntryCount");
    expect(consoleBridge).toContain("group.entry.count += occurrences");
    expect(consoleBridge).toContain("NormalizeConsoleLogLevel");
    expect(consoleBridge).toContain("public string[] levels");
    expect(consoleBridge).toContain("levelFilters.Contains(normalizedLevel)");
    expect(bridge).toContain('case "unity_get_console_log"');
    expect(bridge).toContain("BuildConsoleLogPayloadJson(requestJson)");
    expect(executeContext).toContain(
      "public ConsoleLogResult GetConsoleLog(string level = null, int limit = 50)",
    );
    expect(executeContext).toContain(
      "public ConsoleLogResult GetConsoleLog(string[] levels, int limit = 50)",
    );
    expect(executeContext).toContain("BuildConsoleLogResult(level, limit)");
    expect(executeContext).toContain("BuildConsoleLogResult(null, levels, limit)");
    expect(executeDefinition.description).toContain(
      'ctx.GetConsoleLog(new[] { "warn", "error" }, 50)',
    );
    expect(builtin).toContain('name: "unity_get_console_log".to_string()');
    expect(builtin).toContain('"unity_get_console_log"');

    const consoleTool = builtin.slice(
      builtin.indexOf("pub(super) fn unity_get_console_log"),
      builtin.indexOf("// ─── Unity YAML tools"),
    );
    expect(consoleTool).toContain("crate::unity_bridge::send_message(");
    expect(consoleTool).not.toContain("is_unity_connected");
  });

  it("exposes the tool to Unity and the Locus MCP surface", () => {
    const dev = JSON.parse(read("agent/dev/config.json"));
    const mcpTools = read("src-tauri/src/mcp/server/tools.rs");
    const registry = read("src-tauri/src/tool/builtins/mod.rs");

    expect(dev.tools).toContain("unity_get_console_log");
    expect(mcpTools).toContain('"unity_get_console_log"');
    expect(registry).toContain("registry.register_builtin(unity::unity_get_console_log())");
  });

  it("avoids eager status probes for MCP tools and ordinary file reads", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const mcpTools = read("src-tauri/src/mcp/server/tools.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");

    expect(agent).toContain("tool_context_requires_unity_probe");
    expect(agent).toContain("crate::tool::is_unity_yaml_candidate_path");
    expect(mcpTools).toContain("unity_connected: None");
    expect(bridge).toContain("query_unity_status_response_waiting_with_timeout");
    expect(bridge).toContain("try_query_unity_status_response_with_timeout");
  });
});
