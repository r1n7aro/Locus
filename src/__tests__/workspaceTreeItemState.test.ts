import { describe, expect, it } from "vitest";
import { mountedEntryInPinnedSubtree, pinnedInsertionBefore, workspaceTreeItemState } from "../components/explorer/workspaceTreeItemState";
import type { ProjectExplorerSnapshot } from "../types/workbench";

describe("workspace tree item state", () => {
  it("treats old presets as unmarked and keeps mounted child state separate from its root", () => {
    const snapshot = { itemStates: [
      { nodeId: "mount", pinned: false, highlighted: true },
      { nodeId: "mount", relativePath: "notes/file.md", pinned: true, highlighted: false },
    ] } as ProjectExplorerSnapshot;
    expect(workspaceTreeItemState(undefined, "mount")).toBeUndefined();
    expect(workspaceTreeItemState({} as ProjectExplorerSnapshot, "mount")).toBeUndefined();
    expect(workspaceTreeItemState(snapshot, "mount", null)?.highlighted).toBe(true);
    expect(workspaceTreeItemState(snapshot, "mount", "notes/file.md")?.pinned).toBe(true);
    expect(workspaceTreeItemState(snapshot, "other", "notes/file.md")).toBeUndefined();
  });

  it("extracts nested mounted pins once and restores them to their parent after unpinning", () => {
    const paths = ["notes", "notes/a.md", "notes/sub", "notes/sub/b.md", "other.md"];
    const pins = new Set(["notes", "notes/sub/b.md"]);
    const section = (root?: string) => paths.filter((path) => mountedEntryInPinnedSubtree(path, pins, root));
    expect(section()).toEqual(["other.md"]);
    expect(section("notes")).toEqual(["notes", "notes/a.md", "notes/sub"]);
    expect(section("notes/sub/b.md")).toEqual(["notes/sub/b.md"]);
    pins.delete("notes/sub/b.md");
    expect(section("notes")).toEqual(paths.slice(0, 4));
    pins.clear();
    expect(section()).toEqual(paths);
  });

  it("finds insertion anchors above, below and at the end while skipping dragged pins", () => {
    const states = ["a", "b", "c", "d"].map((nodeId) => ({ nodeId, pinned: true, highlighted: false }));
    expect(pinnedInsertionBefore(states, [states[3]!], states[0]!, false)).toEqual(states[0]);
    expect(pinnedInsertionBefore(states, [states[0]!], states[2]!, true)).toEqual({ nodeId: "d", relativePath: undefined });
    expect(pinnedInsertionBefore(states, [states[0]!, states[2]!], states[1]!, true)).toEqual({ nodeId: "d", relativePath: undefined });
    expect(pinnedInsertionBefore(states, [states[0]!], states[3]!, true)).toBeNull();
    expect(pinnedInsertionBefore(states, [states[0]!, states[1]!], states[1]!, true)).toEqual(states[1]);
  });

  it("reorders mounted child pins independently and ignores star-only state records", () => {
    const states = [
      { nodeId: "mount", relativePath: "first.md", pinned: true, highlighted: true },
      { nodeId: "star-only", pinned: false, highlighted: true },
      { nodeId: "mount", relativePath: "second.md", pinned: true, highlighted: false },
    ];
    expect(pinnedInsertionBefore(states, [{ nodeId: "other" }], states[0]!, true))
      .toEqual({ nodeId: "mount", relativePath: "second.md" });
    expect(pinnedInsertionBefore(states, [states[0]!], states[2]!, true)).toBeNull();
  });
});
