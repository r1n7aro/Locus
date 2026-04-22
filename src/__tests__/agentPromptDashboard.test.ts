import { describe, expect, it } from "vitest";

import {
  buildAgentPromptDashboard,
  estimatePromptTokens,
} from "../components/agent/agentPromptDashboard";

function makeToolMeta(name: string, properties: Record<string, unknown> = {}) {
  return {
    name,
    description: `${name} tool`,
    parameters: {
      type: "object",
      properties,
      required: Object.keys(properties),
    },
  };
}

describe("agentPromptDashboard", () => {
  it("aggregates prompt totals and runtime counts", () => {
    const summary = buildAgentPromptDashboard(
      {
        baseChars: 1200,
        envChars: 400,
        rulesChars: 800,
        knowledgeChars: 600,
        totalChars: 3000,
      },
      [{ enabled: true }, { enabled: false }, { enabled: true }],
      [
        { kind: "context" },
        { kind: "tools", meta: makeToolMeta("read", { filePath: { type: "string" } }) },
        { kind: "tools", meta: makeToolMeta("edit", { filePath: { type: "string" }, oldText: { type: "string" } }) },
      ],
    );

    expect(summary.totalChars).toBeGreaterThan(3000);
    expect(summary.totalTokens).toBeGreaterThan(estimatePromptTokens(3000));
    expect(summary.enabledRuleCount).toBe(2);
    expect(summary.totalRuleCount).toBe(3);
    expect(summary.injectedContextCount).toBe(1);
    expect(summary.toolCount).toBe(2);
    expect(summary.parts.map((part) => part.key)).toEqual([
      "base",
      "env",
      "rules",
      "knowledge",
      "tools",
    ]);
    expect(summary.parts.find((part) => part.key === "tools")?.tokens).toBeGreaterThan(0);
  });

  it("keeps a compact balanced prompt in the healthy range", () => {
    const summary = buildAgentPromptDashboard(
      {
        baseChars: 1000,
        envChars: 320,
        rulesChars: 760,
        knowledgeChars: 480,
        totalChars: 2560,
      },
      [{ enabled: true }, { enabled: true }, { enabled: true }],
      [
        { kind: "context" },
        { kind: "tools", meta: makeToolMeta("read", { filePath: { type: "string" } }) },
      ],
    );

    expect(summary.health.level).toBe("healthy");
    expect(summary.health.score).toBeGreaterThanOrEqual(82);
    expect(summary.health.dominantShare).toBeLessThan(0.6);
  });

  it("flags a large knowledge-heavy prompt as heavy", () => {
    const summary = buildAgentPromptDashboard(
      {
        baseChars: 1400,
        envChars: 1200,
        rulesChars: 2600,
        knowledgeChars: 10000,
        totalChars: 15200,
      },
      [{ enabled: false }, { enabled: false }],
      [
        { kind: "context" },
        { kind: "context" },
        { kind: "tools", meta: makeToolMeta("unity_execute", { code: { type: "string" }, waitMs: { type: "number" } }) },
      ],
    );

    expect(summary.health.level).toBe("heavy");
    expect(summary.health.score).toBeLessThan(60);
    expect(summary.health.dominantPartKey).toBe("knowledge");
  });
});
