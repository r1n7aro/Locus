import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

function read(relativePath: string) {
  return readFileSync(resolve(root, relativePath), "utf8");
}

describe("foreground bash progress", () => {
  it("streams captured stdout and stderr through the active tool call", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const shell = read("src-tauri/src/tool/builtins/shell.rs");
    const reducer = read("src/composables/useStreamReducer.ts");
    const block = read("src/components/ToolCallBlock.vue");
    const definition = read("tools/bash.json");

    const foregroundBash = agent.slice(
      agent.indexOf('if tc.name == "bash" {', agent.indexOf("let mut tool_context = self")),
      agent.indexOf("let mut result = self", agent.indexOf("let mut tool_context = self")),
    );

    expect(foregroundBash).toContain("tool_context.output = Some");
    expect(foregroundBash).toContain("StreamEvent::ToolCallDelta");
    expect(shell).toContain("let execution = run_captured_command_with_input(");
    expect(shell).toContain("ctx.output_path.clone()");
    expect(shell).toContain("spawn_managed(command, process_owner)");
    expect(shell).toContain("report(decode_console_bytes(&chunk))");
    expect(reducer).toContain('case "toolCallDelta"');
    expect(reducer).toContain('type: "appendToolDelta"');
    expect(block).toContain("displayedToolOutput");
    expect(block).toContain("output-streaming-indicator");
    expect(definition).toContain("Both modes stream emitted output to the tool block");
    expect(definition).toContain("git clone --progress");
  });
});
