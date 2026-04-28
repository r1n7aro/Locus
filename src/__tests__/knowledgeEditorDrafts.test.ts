import { describe, expect, it } from "vitest";
import type { KnowledgeDocument } from "../types";
import {
  createKnowledgeEditorDraftValues,
  mergeKnowledgeEditorDraftValues,
} from "../components/knowledge/knowledgeEditorDrafts";

function makeDocument(overrides: Partial<KnowledgeDocument> = {}): KnowledgeDocument {
  return {
    id: "design-1",
    type: "design",
    path: "combat/core-loop.md",
    title: "核心循环",
    injectMode: "excerpt",
    summaryEnabled: true,
    commandEnabled: false,
    readOnly: false,
    aiMaintained: false,
    explicitMaintenanceRules: true,
    externalSource: null,
    skillEnabled: null,
    skillSurface: null,
    commandTrigger: null,
    argumentHint: null,
    summary: "摘要 v1",
    body: "正文 v1",
    maintenanceRules: "规则 v1",
    createdAt: 1,
    updatedAt: 2,
    hasSummary: true,
    ...overrides,
  };
}

describe("knowledgeEditorDrafts", () => {
  it("preserves dirty sections while applying clean remote updates", () => {
    const current = makeDocument();
    const drafts = createKnowledgeEditorDraftValues(current);
    drafts.body = "正文 v2";

    const updated = makeDocument({
      updatedAt: 3,
      summary: "摘要 v2",
      body: "正文 v1",
    });

    const result = mergeKnowledgeEditorDraftValues(
      updated,
      drafts,
      new Set(["body"]),
    );

    expect(result.drafts.summary).toBe("摘要 v2");
    expect(result.drafts.body).toBe("正文 v2");
    expect(result.dirtySections).toEqual(new Set(["body"]));
  });

  it("clears dirty sections when the persisted content catches up", () => {
    const updated = makeDocument({
      updatedAt: 3,
      body: "正文 v2",
    });
    const drafts = createKnowledgeEditorDraftValues(makeDocument());
    drafts.body = "正文 v2";

    const result = mergeKnowledgeEditorDraftValues(
      updated,
      drafts,
      new Set(["body"]),
    );

    expect(result.drafts.body).toBe("正文 v2");
    expect(result.dirtySections.size).toBe(0);
  });

  it("fully resets drafts when switching documents", () => {
    const result = mergeKnowledgeEditorDraftValues(
      makeDocument({
        id: "memory-1",
        type: "memory",
        path: "project-understanding.md",
        title: "项目理解",
        summary: "",
        body: "记忆正文",
        maintenanceRules: "记忆规则",
      }),
      {
        summary: "旧摘要",
        body: "旧正文",
        maintenanceRules: "旧规则",
      },
      new Set(["body"]),
      true,
    );

    expect(result.drafts).toEqual({
      summary: "",
      body: "记忆正文",
      maintenanceRules: "记忆规则",
    });
    expect(result.dirtySections.size).toBe(0);
  });
});
