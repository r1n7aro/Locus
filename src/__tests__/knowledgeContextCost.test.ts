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
    const knowledgeRead = makeToolItem("knowledge_read");
    const knowledgeCreate = makeToolItem("knowledge_create", "lazy");
    const read = makeToolItem("read");

    const total = estimateKnowledgeContextCostTokens([
      knowledgeContext,
      knowledgeRule,
      knowledgeRead,
      knowledgeCreate,
      read,
    ]);

    expect(isKnowledgeInjectionItem(knowledgeContext)).toBe(true);
    expect(isDirectKnowledgeToolItem(knowledgeRead)).toBe(true);
    expect(isDirectKnowledgeToolItem(knowledgeCreate)).toBe(false);
    expect(isDirectKnowledgeToolItem(read)).toBe(false);
    expect(total).toBe(
      estimatePromptTokens(knowledgeContext.content.length)
        + estimatePromptTokens(knowledgeRule.content.length)
        + estimateToolPrompt(knowledgeRead.meta).tokens,
    );
  });
});
