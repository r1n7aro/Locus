import { describe, expect, it } from "vitest";
import type { KnowledgeDocument } from "../types";
import { isKnowledgeEditModeLocked } from "../components/knowledge/knowledgeEditMode";

function makeEditModeDocument(
  overrides: Partial<
    Pick<KnowledgeDocument, "type" | "readOnly" | "aiMaintained" | "inheritAiConfig" | "externalSource">
  > = {},
): Pick<KnowledgeDocument, "type" | "readOnly" | "aiMaintained" | "inheritAiConfig" | "externalSource"> {
  return {
    type: "design",
    readOnly: false,
    aiMaintained: false,
    inheritAiConfig: false,
    externalSource: null,
    ...overrides,
  };
}

describe("knowledgeEditMode", () => {
  it("locks the edit mode selector for read-only documents", () => {
    expect(isKnowledgeEditModeLocked(makeEditModeDocument({ readOnly: true }))).toBe(true);
  });

  it("locks the edit mode selector for managed external sources", () => {
    expect(
      isKnowledgeEditModeLocked(
        makeEditModeDocument({ externalSource: { provider: "feishu" } }),
      ),
    ).toBe(true);
    expect(
      isKnowledgeEditModeLocked(
        makeEditModeDocument({ externalSource: { provider: "local_folder" } }),
      ),
    ).toBe(true);
  });

  it("keeps the edit mode selector available for editable internal documents", () => {
    expect(isKnowledgeEditModeLocked(makeEditModeDocument())).toBe(false);
  });
});
