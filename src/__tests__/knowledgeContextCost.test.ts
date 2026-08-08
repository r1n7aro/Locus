import { describe, expect, it } from "vitest";

import type { InjectedPromptItem } from "../types";
import {
  estimateKnowledgeContextCostTokens,
  isDirectKnowledgeToolItem,
  isKnowledgeInjectionItem,
} from "../components/chat/knowledgeContextCost";
import { estimatePromptTokens, estimateToolPrompt } from "../components/agent/agentPromptDashboard";

function makeToolItem(
  name: string,
  loadMode: "direct" | "lazy" | "skill" = "direct",
): InjectedPromptItem {
  return {
    id: `available_tool::${name}`,
    title: name,
    kind: "tools",
    content: `${name} description`,
    source: "runtime",
    meta: {
      function: {
        name,
        description: `${name} description`,
        parameters: {
          type: "object",
          properties: {
            path: { type: "string" },
          },
        },
      },
      loadMode,
    },
  };
}

describe("knowledgeContextCost", () => {
  it("counts knowledge context blocks and directly loaded knowledge tool schemas", () => {
    const knowledgeContext: InjectedPromptItem = {
      id: "knowledge_context",
      title: "Knowledge",
      kind: "context",
      content: "abcd".repeat(20),
      source: "system",
    };
    const knowledgeRule: InjectedPromptItem = {
      id: "knowledge_rule::memory::team.md",
      title: "Team Memory",
      kind: "rule",
      content: "abcd".repeat(10),
      source: "system",
    };
    const knowledgeQuery = makeToolItem("knowledge_query");
    const retiredKnowledgeRead = makeToolItem("knowledge_read");
    const read = makeToolItem("read");

    const total = estimateKnowledgeContextCostTokens([
      knowledgeContext,
      knowledgeRule,
      knowledgeQuery,
      retiredKnowledgeRead,
      read,
    ]);

    expect(isKnowledgeInjectionItem(knowledgeContext)).toBe(true);
    expect(isDirectKnowledgeToolItem(knowledgeQuery)).toBe(true);
    expect(isDirectKnowledgeToolItem(retiredKnowledgeRead)).toBe(false);
    expect(isDirectKnowledgeToolItem(read)).toBe(false);
    expect(total).toBe(
      estimatePromptTokens(knowledgeContext.content.length)
        + estimatePromptTokens(knowledgeRule.content.length)
        + estimateToolPrompt(knowledgeQuery.meta).tokens,
    );
  });
});
