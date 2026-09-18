import { describe, expect, it } from "vitest";
import { previewWorkspaceTreeMove, previewWorkspaceTreePin, workspaceTreeMoveOperation, workspaceTreeMoveOperations, workspaceTreePinOperations } from "../components/explorer/workspaceTreeDropPreview";
import type { ProjectExplorerNode, ProjectExplorerSnapshot } from "../types/workbench";

function snapshot(): ProjectExplorerSnapshot {
  const node = (nodeId: string, position: number, parentNodeId?: string): ProjectExplorerNode => ({
    nodeId, projectId: "project", nodeKind: "resource", resourceKind: "knowledge", resourceId: nodeId,
    hidden: false, position, parentNodeId,
  });
  return {
    projectId: "project", presetId: "default", presetName: "Default", manifestPath: "tree.json", revision: 5,
    presets: [],
    nodes: [node("a", 0), node("b", 1), node("c", 2),
      { ...node("folder", 3), nodeKind: "folder" }, node("child", 0, "folder")],
    itemStates: [{ nodeId: "b", pinned: true, highlighted: true }],
  };
}

function order(value: ProjectExplorerSnapshot, parent: string | null = null): string[] {
  return value.nodes.filter((node) => (node.parentNodeId ?? null) === parent)
    .sort((left, right) => left.position - right.position).map((node) => node.nodeId);
}

describe("workspace tree released-row preview", () => {
  it.each([
    [["a", "b"], 4, ["c", "folder", "a", "b"]],
    [["c", "a"], 1, ["c", "a", "b", "folder"]],
    [["a", "c"], 1, ["a", "c", "b", "folder"]],
  ] as const)("keeps selection %s together at insertion %s", (ids, position, expected) => {
    const before = snapshot();
    const sources = ids.map((id) => before.nodes.find((node) => node.nodeId === id)!);
    const operations = workspaceTreeMoveOperations(before, sources, { parentNodeId: null, position });
    const after = operations.reduce(previewWorkspaceTreeMove, before);
    expect(order(after)).toEqual(expected);
    expect(order(before)).toEqual(["a", "b", "c", "folder"]);
  });
  it("pins a regular document directly before the target and leaves persistence untouched", () => {
    const before = snapshot();
    const original = structuredClone(before);
    const items = [{ nodeId: "c" }];
    const anchor = { nodeId: "b" };
    const preview = previewWorkspaceTreePin(before, items, anchor);
    expect(preview.itemStates).toEqual([
      { nodeId: "c", pinned: true, highlighted: false },
      { nodeId: "b", pinned: true, highlighted: true },
    ]);
    expect(workspaceTreePinOperations(items, anchor)).toEqual([
      { kind: "setItemState", nodeId: "c", pinned: true },
      { kind: "movePinnedItems", items, before: anchor },
    ]);
    expect(preview.nodes).toBe(before.nodes);
    expect(before).toEqual(original);
  });

  it("retains stars and saved relative order when reordering multiple pins", () => {
    const before = snapshot();
    before.itemStates = [
      { nodeId: "a", pinned: true, highlighted: true },
      { nodeId: "b", pinned: true, highlighted: false },
      { nodeId: "c", pinned: true, highlighted: true },
    ];
    const preview = previewWorkspaceTreePin(before, [{ nodeId: "c" }, { nodeId: "a" }], null);
    expect(preview.itemStates?.map((state) => state.nodeId)).toEqual(["b", "a", "c"]);
    expect(preview.itemStates?.map((state) => state.highlighted)).toEqual([false, true, true]);
    expect(previewWorkspaceTreePin(before, [{ nodeId: "a" }], { nodeId: "a" }).itemStates).toEqual(before.itemStates);
  });

  it("distinguishes mounted descendants from the directory's own pin", () => {
    const before = snapshot();
    before.itemStates = [{ nodeId: "folder", relativePath: "a.md", pinned: true, highlighted: true }];
    const preview = previewWorkspaceTreePin(before, [{ nodeId: "folder", relativePath: "b.md" }], { nodeId: "folder", relativePath: "a.md" });
    expect(preview.itemStates?.map((state) => state.relativePath)).toEqual(["b.md", "a.md"]);
    expect(preview.itemStates?.every((state) => state.pinned)).toBe(true);
    expect(before.itemStates).toHaveLength(1);
  });

  it("keeps existing highlight placement when pinning without an insertion target", () => {
    const before = snapshot();
    before.itemStates!.unshift({ nodeId: "a", pinned: false, highlighted: true });
    const preview = previewWorkspaceTreePin(before, [{ nodeId: "a" }, { nodeId: "c" }]);
    expect(preview.itemStates?.map((state) => state.nodeId)).toEqual(["a", "b", "c"]);
    expect(preview.itemStates?.every((state) => state.pinned)).toBe(true);
    expect(before.itemStates![0]!.pinned).toBe(false);
  });

  it.each([
    ["a", 3, ["b", "c", "a", "folder"]],
    ["c", 0, ["c", "a", "b", "folder"]],
    ["b", 2, ["a", "b", "c", "folder"]],
    ["a", 99, ["b", "c", "folder", "a"]],
  ] as const)("keeps %s at the final insertion position %s", (id, position, expected) => {
    const before = snapshot();
    const operation = workspaceTreeMoveOperation(before.nodes.find((node) => node.nodeId === id)!, {
      parentNodeId: null, position,
    });
    const preview = previewWorkspaceTreeMove(before, operation);
    expect(order(preview)).toEqual(expected);
    expect(preview.nodes.filter((node) => node.nodeId === id)).toHaveLength(1);
    expect(order(before)).toEqual(["a", "b", "c", "folder"]);
    expect(preview.revision).toBe(5);
  });

  it("moves into a folder without losing children or modifying the persisted snapshot", () => {
    const before = snapshot();
    const original = structuredClone(before);
    const preview = previewWorkspaceTreeMove(before, { kind: "moveNode", nodeId: "b", parentNodeId: "folder", position: 0 });
    expect(order(preview)).toEqual(["a", "c", "folder"]);
    expect(order(preview, "folder")).toEqual(["b", "child"]);
    expect(preview.itemStates).toEqual([{ nodeId: "b", pinned: false, highlighted: true }]);
    expect(before).toEqual(original);
  });

  it("normalizes both folders when moving a nested document back to the root", () => {
    const before = snapshot();
    const preview = previewWorkspaceTreeMove(before, { kind: "moveNode", nodeId: "child", position: 1 });
    expect(order(preview)).toEqual(["a", "child", "b", "c", "folder"]);
    expect(order(preview, "folder")).toEqual([]);
    expect(preview.nodes.find((node) => node.nodeId === "folder")?.position).toBe(4);
  });

  it("counts hidden siblings and retains pinned descendants of a moved folder", () => {
    const before = snapshot();
    before.nodes[1]!.hidden = true;
    before.itemStates!.push({ nodeId: "folder", relativePath: "nested/design.md", pinned: true, highlighted: false });
    const preview = previewWorkspaceTreeMove(before, { kind: "moveNode", nodeId: "folder", position: 1 });
    expect(order(preview)).toEqual(["a", "folder", "b", "c"]);
    expect(order(preview, "folder")).toEqual(["child"]);
    expect(preview.nodes.find((node) => node.nodeId === "b")?.hidden).toBe(true);
    expect(preview.itemStates).toEqual(before.itemStates);
  });
});
