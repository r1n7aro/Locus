import { describe, expect, it } from "vitest";
import {
  resolveCollabRightPanelMode,
  resolveConflictActionHint,
  resolveMergeOperationBadge,
} from "../components/collab/collabViewMode";
import type { MergeOperation } from "../types";

describe("collabViewMode", () => {
  it("prioritizes merge mode over selected commit details", () => {
    expect(resolveCollabRightPanelMode("commit", true)).toBe("merge");
  });

  it("shows commit details only when there is no blocking conflict state", () => {
    expect(resolveCollabRightPanelMode("commit", false)).toBe("commit");
    expect(resolveCollabRightPanelMode("stash", false)).toBe("commit");
  });

  it("falls back to workspace mode for WIP selection", () => {
    expect(resolveCollabRightPanelMode("workspace", false)).toBe("workspace");
    expect(resolveCollabRightPanelMode("none", false)).toBe("workspace");
  });

  it("shows a generic conflict badge when only unmerged files are present", () => {
    expect(resolveMergeOperationBadge(null, true)).toBe("CONFLICT");
  });

  it("keeps operation-specific badges when an explicit merge operation exists", () => {
    const operation: MergeOperation = {
      kind: "rebase",
      canContinue: false,
      canSkip: true,
      canAbort: true,
      label: "Rebasing main",
    };
    expect(resolveMergeOperationBadge(operation, true)).toBe("REBASE");
  });

  it("uses a generic disable hint when the repository only has unresolved files", () => {
    expect(resolveConflictActionHint(null)).toBe("存在未解决冲突，操作已禁用");
  });
});
