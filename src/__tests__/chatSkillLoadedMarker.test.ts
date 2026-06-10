import { describe, expect, it } from "vitest";
import {
  resolveSkillLoadedMarkerForToolCall,
} from "../components/toolCallSkillLoadedMarker";
import type { ToolCallDisplay, ToolCallInfo } from "../types";

function knowledgeReadToolCall(args: Record<string, unknown>, overrides: Partial<ToolCallInfo> = {}): ToolCallInfo {
  return {
    id: "call_read_skill",
    name: "knowledge_read",
    arguments: JSON.stringify(args),
    outcome: "done",
    ...overrides,
  };
}

describe("chat skill loaded marker", () => {
  it("uses the full knowledge_read heading as the loaded Skill name", () => {
    const marker = resolveSkillLoadedMarkerForToolCall(
      knowledgeReadToolCall({ path: "skill/builtin/profiler.md" }),
      "# Unity Profiler Runtime Sampling\n\n## Content\nUse this skill.",
    );

    expect(marker).toEqual({
      name: "Unity Profiler Runtime Sampling",
      path: "skill/builtin/profiler.md",
    });
  });

  it("falls back to the Skill package root path when the output has no title", () => {
    const marker = resolveSkillLoadedMarkerForToolCall(
      knowledgeReadToolCall({ path: "skill/studio.tools.psd-to-ugui", part: "body" }),
      "Use this package.",
    );

    expect(marker).toEqual({
      name: "studio.tools.psd-to-ugui",
      path: "skill/studio.tools.psd-to-ugui",
    });
  });

  it("keeps summary-only reads and non-Skill reads as regular tool calls", () => {
    expect(resolveSkillLoadedMarkerForToolCall(
      knowledgeReadToolCall({ path: "skill/builtin/profiler.md", part: "summary" }),
      "Profiler workflow",
    )).toBeNull();

    expect(resolveSkillLoadedMarkerForToolCall(
      knowledgeReadToolCall({ path: "reference/unity/input-system.md" }),
      "# Input System",
    )).toBeNull();
  });

  it("supports completed tool display rows", () => {
    const toolCall: ToolCallDisplay = {
      id: "display_read_skill",
      name: "knowledge_read",
      arguments: JSON.stringify({ path: "skill/builtin/profiler.md" }),
      status: "done",
      output: "# Unity Profiler Runtime Sampling",
    };

    expect(resolveSkillLoadedMarkerForToolCall(toolCall, toolCall.output)).toEqual({
      name: "Unity Profiler Runtime Sampling",
      path: "skill/builtin/profiler.md",
    });
  });

  it("keeps running display rows as regular tool calls", () => {
    const toolCall: ToolCallDisplay = {
      id: "display_read_skill",
      name: "knowledge_read",
      arguments: JSON.stringify({ path: "skill/builtin/profiler.md" }),
      status: "running",
      output: "# Unity Profiler Runtime Sampling",
    };

    expect(resolveSkillLoadedMarkerForToolCall(toolCall, toolCall.output)).toBeNull();
  });
});
