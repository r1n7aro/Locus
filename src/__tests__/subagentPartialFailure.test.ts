import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("subagent partial failure recovery", () => {
  it("returns persisted assistant text as an interrupted tool result", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const recovery = agent.slice(
      agent.indexOf("fn recover_subagent_partial_output"),
      agent.indexOf("struct SystemPromptParts"),
    );
    const runSubagent = agent.slice(
      agent.indexOf("async fn run_subagent"),
      agent.indexOf("fn suppressed_subagent_tool_result"),
    );
    const executeSubagentStart = agent.indexOf("async fn execute_subagent");
    const executeSubagent = agent.slice(
      executeSubagentStart,
      agent.indexOf("#[cfg(test)]", executeSubagentStart),
    );

    expect(recovery).toContain("message.role == MessageRole::Assistant");
    expect(recovery).toContain("CONTEXT_HANDOFF_MARKER");
    expect(recovery).toContain("ToolRunOutcome::Interrupted");
    expect(recovery).toContain("ToolRunOutcome::Error");
    expect(runSubagent).toContain("collect_assistant_tool_calls(&child_messages)");
    expect(runSubagent).toContain("subagent_failure_result(&child_messages, &e)");
    expect(executeSubagent).toContain("is_error: outcome == ToolRunOutcome::Error");
    expect(executeSubagent).toContain("nested_tool_calls: (!tool_calls.is_empty()).then_some(tool_calls)");
  });

  it("routes subagent task join failures through the same recovery path", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const runSubagent = agent.slice(
      agent.indexOf("async fn run_subagent"),
      agent.indexOf("fn suppressed_subagent_tool_result"),
    );

    expect(runSubagent).toContain("Err(format!(\"Subagent task failed to join: {}\", error))");
    expect(runSubagent).not.toContain(".map_err(|error| format!(\"Subagent task failed to join: {}\", error))?");
  });
});
