import { describe, expect, it } from "vitest";
import { shouldShowKnowledgeEmptyFolder } from "../components/knowledge/knowledgeExplorerEmptyFolder";

const confirmedEmpty = {
  searchMode: false,
  expanded: true,
  directChildCount: 0,
  contentsLoaded: true,
  contentsLoading: false,
  hasMoreContents: false,
  hasTransientChild: false,
};

describe("knowledge explorer empty-folder feedback", () => {
  it("shows feedback only after an expanded folder is confirmed empty", () => {
    expect(shouldShowKnowledgeEmptyFolder(confirmedEmpty)).toBe(true);
  });

  it.each([
    ["search results", { searchMode: true }],
    ["collapsed folders", { expanded: false }],
    ["folders with children", { directChildCount: 1 }],
    ["folders awaiting their first load", { contentsLoaded: false }],
    ["folders still loading", { contentsLoading: true }],
    ["folders with another page", { hasMoreContents: true }],
    ["folders with an inline or drag preview child", { hasTransientChild: true }],
  ])("hides feedback for %s", (_label, patch) => {
    expect(shouldShowKnowledgeEmptyFolder({ ...confirmedEmpty, ...patch })).toBe(false);
  });
});
