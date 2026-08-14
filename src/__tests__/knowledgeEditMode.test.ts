import { describe, expect, it } from "vitest";
import type { KnowledgeDocument } from "../types";
import {
  buildKnowledgeEditModePatch,
  getKnowledgeEditMode,
  isKnowledgeEditModeLocked,
} from "../components/knowledge/knowledgeEditMode";

function makeEditModeDocument(
  overrides: Partial<
    Pick<KnowledgeDocument, "type" | "readOnly" | "aiEditMode" | "aiMaintained" | "storageSource" | "externalSource">
  > = {},
): Pick<KnowledgeDocument, "type" | "readOnly" | "aiEditMode" | "aiMaintained" | "storageSource" | "externalSource"> {
  return {
    type: "design",
    readOnly: false,
    aiEditMode: "confirm",
    aiMaintained: false,
    storageSource: "project",
    externalSource: null,
    ...overrides,
  };
}

describe("knowledgeEditMode", () => {
  it("keeps read-only state independent from the four AI edit modes", () => {
    expect(
      getKnowledgeEditMode(makeEditModeDocument({ readOnly: true, aiEditMode: "auto" })),
    ).toBe("auto");
    expect(getKnowledgeEditMode(makeEditModeDocument({ aiEditMode: "disabled" }))).toBe("disabled");
    expect(buildKnowledgeEditModePatch("disabled")).toEqual({
      aiEditMode: "disabled",
    });
    expect(getKnowledgeEditMode(makeEditModeDocument({ aiEditMode: "confirm" }))).toBe("proposal");
    expect(buildKnowledgeEditModePatch("proposal")).toEqual({
      aiEditMode: "confirm",
    });
    expect(getKnowledgeEditMode(makeEditModeDocument({ aiEditMode: "auto" }))).toBe("auto");
    expect(buildKnowledgeEditModePatch("auto")).toEqual({
      aiEditMode: "auto",
    });
  });

  it("locks the edit mode selector for read-only documents", () => {
    expect(isKnowledgeEditModeLocked(makeEditModeDocument({ readOnly: true }))).toBe(true);
  });

  it("locks the edit mode selector for managed external sources", () => {
    expect(
      isKnowledgeEditModeLocked(makeEditModeDocument({ storageSource: "app" })),
    ).toBe(true);
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
    expect(
      isKnowledgeEditModeLocked(
        makeEditModeDocument({ externalSource: { provider: "package" } }),
      ),
    ).toBe(true);
  });

  it("keeps the edit mode selector available for editable internal documents", () => {
    expect(isKnowledgeEditModeLocked(makeEditModeDocument())).toBe(false);
  });
});
