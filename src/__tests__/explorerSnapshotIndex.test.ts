import { describe, expect, it } from "vitest";
import { explorerChildNodes } from "../components/explorer/explorerSnapshotIndex";
import { workspaceTreeItemState } from "../components/explorer/workspaceTreeItemState";
import type { ProjectExplorerItemState, ProjectExplorerNode, ProjectExplorerSnapshot } from "../types/workbench";

function node(nodeId: string, parentNodeId: string | null | undefined): ProjectExplorerNode {
  return { nodeId, parentNodeId, nodeKind: "resource", position: 0 } as unknown as ProjectExplorerNode;
}

describe("explorer snapshot index", () => {
  it("groups nodes by parent in snapshot order, with missing parents at the root", () => {
    const nodes = [node("a", null), node("b", "a"), node("c", undefined), node("d", "a"), node("e", "b")];
    expect(explorerChildNodes(nodes, null).map((item) => item.nodeId)).toEqual(["a", "c"]);
    expect(explorerChildNodes(nodes, "a").map((item) => item.nodeId)).toEqual(["b", "d"]);
    expect(explorerChildNodes(nodes, "e")).toEqual([]);
    // The index belongs to the array: a replaced snapshot gets its own.
    const next = [...nodes, node("f", "e")];
    expect(explorerChildNodes(next, "e").map((item) => item.nodeId)).toEqual(["f"]);
    expect(explorerChildNodes(nodes, "e")).toEqual([]);
  });

  it("returns the same item state as a linear search for every lookup", () => {
    const itemStates = [
      { nodeId: "a", pinned: true, highlighted: false },
      { nodeId: "a", pinned: false, highlighted: true },
      { nodeId: "a", relativePath: "x.md", pinned: false, highlighted: true },
      { nodeId: "b", relativePath: "", pinned: true, highlighted: false },
      { nodeId: "b", relativePath: null, pinned: false, highlighted: true },
      { nodeId: "c", relativePath: "dir/y.md", pinned: true, highlighted: true },
    ] as ProjectExplorerItemState[];
    const snapshot = { itemStates } as ProjectExplorerSnapshot;
    const linear = (nodeId: string, relativePath?: string | null) => itemStates.find((state) => (
      state.nodeId === nodeId && (state.relativePath ?? null) === (relativePath || null)
    ));
    for (const nodeId of ["a", "b", "c", "missing"]) {
      for (const relativePath of [undefined, null, "", "x.md", "dir/y.md"]) {
        expect(workspaceTreeItemState(snapshot, nodeId, relativePath)).toBe(linear(nodeId, relativePath));
      }
    }
  });
});
