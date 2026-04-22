import { describe, expect, it } from "vitest";
import {
  buildConflictResolutionKey,
  prunePendingConflictResolutionKeys,
} from "../components/collab/conflictResolutionState";
import type { UnmergedFileEntry } from "../types";

function makeConflict(overrides: Partial<UnmergedFileEntry> = {}): UnmergedFileEntry {
  return {
    path: "Assets/BuildinCatalog.asset",
    conflictCode: "UU",
    semanticLabel: "both modified",
    baseOid: "base-1",
    leftOid: "left-1",
    rightOid: "right-1",
    lfs: false,
    headMode: "100644",
    stage1Mode: "100644",
    stage2Mode: "100644",
    stage3Mode: "100644",
    ...overrides,
  };
}

describe("conflictResolutionState", () => {
  it("keeps a pending resolution while the same conflict still exists", () => {
    const file = makeConflict();
    const pending = new Set([buildConflictResolutionKey(file)]);

    const next = prunePendingConflictResolutionKeys(pending, [file]);

    expect([...next]).toEqual([...pending]);
  });

  it("clears a pending resolution after the conflict disappears from git status", () => {
    const file = makeConflict();
    const pending = new Set([buildConflictResolutionKey(file)]);

    const next = prunePendingConflictResolutionKeys(pending, []);

    expect(next.size).toBe(0);
  });

  it("clears a stale pending resolution when the same path reports a new conflict identity", () => {
    const file = makeConflict();
    const refreshed = makeConflict({
      leftOid: "left-2",
      rightOid: "right-2",
    });
    const pending = new Set([buildConflictResolutionKey(file)]);

    const next = prunePendingConflictResolutionKeys(pending, [refreshed]);

    expect(next.size).toBe(0);
  });
});
