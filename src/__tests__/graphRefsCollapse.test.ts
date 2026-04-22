import { describe, expect, it } from "vitest";
import type { HistoryGraphDisplayRef } from "../components/collab/graph/types";
import {
  collapseDisplayRefsForRail,
  estimateDisplayRefsRailWidth,
  estimateDisplayRefOverflowWidth,
  estimateDisplayRefWidth,
} from "../components/collab/graph/refs";

function branchRef(text: string): HistoryGraphDisplayRef {
  return {
    key: text,
    kind: "branch",
    text,
    sourceMarkers: [
      {
        key: `local:${text}`,
        kind: "local",
        title: "local",
      },
    ],
  };
}

function remoteRef(text: string): HistoryGraphDisplayRef {
  return {
    key: `remote:${text}`,
    kind: "remote",
    text,
    sourceMarkers: [
      {
        key: `remote:${text}`,
        kind: "remote",
        title: "origin",
        variant: 0,
      },
    ],
  };
}

describe("graph refs collapse", () => {
  it("keeps every ref visible when the rail has enough space", () => {
    const refs = [branchRef("master"), branchRef("分支1")];
    const availableWidth = refs.reduce((sum, ref) => sum + estimateDisplayRefWidth(ref), 0) + 6;

    const collapsed = collapseDisplayRefsForRail(refs, availableWidth);

    expect(collapsed.isCollapsed).toBe(false);
    expect(collapsed.hiddenCount).toBe(0);
    expect(collapsed.visibleRefs.map(ref => ref.text)).toEqual(["master", "分支1"]);
  });

  it("shows the remaining refs as a +N badge when the rail would overlap", () => {
    const refs = [branchRef("master"), branchRef("分支1")];
    const availableWidth = estimateDisplayRefWidth(refs[0]!) + 6 + estimateDisplayRefOverflowWidth(1);

    const collapsed = collapseDisplayRefsForRail(refs, availableWidth);

    expect(collapsed.isCollapsed).toBe(true);
    expect(collapsed.visibleRefs.map(ref => ref.text)).toEqual(["master"]);
    expect(collapsed.hiddenRefs.map(ref => ref.text)).toEqual(["分支1"]);
    expect(collapsed.hiddenCount).toBe(1);
  });

  it("preserves the leading ref when multiple refs overflow the rail", () => {
    const refs = [branchRef("master"), branchRef("分支1"), branchRef("release/demo")];
    const availableWidth = estimateDisplayRefWidth(refs[0]!) + 6 + estimateDisplayRefOverflowWidth(2) - 1;

    const collapsed = collapseDisplayRefsForRail(refs, availableWidth);

    expect(collapsed.visibleRefs.map(ref => ref.text)).toEqual(["master"]);
    expect(collapsed.hiddenRefs.map(ref => ref.text)).toEqual(["分支1", "release/demo"]);
    expect(collapsed.hiddenCount).toBe(2);
  });

  it("sizes the default refs rail from the collapsed summary instead of every badge", () => {
    const headRefs = [branchRef("master")];
    const oldCommitRefs = [branchRef("分支1"), remoteRef("master")];
    const railWidth = estimateDisplayRefsRailWidth([headRefs, oldCommitRefs]);

    const collapsed = collapseDisplayRefsForRail(oldCommitRefs, railWidth);

    expect(railWidth).toBeLessThan(
      estimateDisplayRefWidth(oldCommitRefs[0]!)
      + 6
      + estimateDisplayRefWidth(oldCommitRefs[1]!),
    );
    expect(collapsed.visibleRefs.map(ref => ref.text)).toEqual(["分支1"]);
    expect(collapsed.hiddenRefs.map(ref => ref.text)).toEqual(["master"]);
    expect(collapsed.hiddenCount).toBe(1);
  });
});
