import { describe, expect, it } from "vitest";
import { layoutHistoryGraph } from "../components/collab/graph/layout";
import { normalizeHistoryGraph } from "../components/collab/graph/normalize";
import { layoutHistoryGraphEdges, type HistoryGraphEdgeRoute } from "../components/collab/graph/edges";
import type { HistoryGraphInput, HistoryGraphLayoutResult } from "../components/collab/graph/types";

function inputFor(parents: string[][]): HistoryGraphInput {
  return {
    commits: parents.map((parents, index) => ({
      hash: `c${index}`, shortHash: `c${index}`, parents, message: `commit ${index}`,
      refs: [], author: "tester", date: 100 - index, isStash: false,
    })),
    refs: ["main", "branch1", "branch2"].map((name, index) => ({
      fullName: `refs/heads/${name}`, shortName: name, branchName: name,
      kind: "localBranch", targetHash: `c${index}`, isCurrent: index === 0,
    })),
    stashes: [], headState: { kind: "attached", hash: "c0", refName: "main" },
    selectedHistory: null, workspaceChangeCount: 0,
  };
}

function crossings(layout: HistoryGraphLayoutResult) {
  return layout.commits.flatMap(row => row.downEdges.flatMap((edge, index, edges) =>
    edges.slice(index + 1)
      .filter(other => (edge.fromLane - other.fromLane) * (edge.toLane - other.toLane) < 0)
      .map(other => ({ hash: row.commit.hash, edge, other })),
  ));
}

describe("history graph routing", () => {
  it("opens a merge parent beside its source instead of crossing an unrelated branch", () => {
    // main's second parent used to jump from lane 1 to 3 through branch1 in lane 2.
    const input = inputFor([["c3"], ["c4"], ["c4"], ["c7", "c5"], ["c7"], ["c6"], ["c7"], []]);
    const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
    expect(crossings(layout)).toEqual([]);
    expect(layout.commits.map(row => row.commit.hash)).toEqual(input.commits.map(commit => commit.hash));
  });

  it("joins shared parents at the actual commit instead of merging above unrelated rows", () => {
    const input = inputFor([["c4"], ["c4"], ["c3"], ["c4"], []]);
    const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
    for (let index = 0; index < layout.commits.length - 1; index++) {
      const row = layout.commits[index]!;
      const next = layout.commits[index + 1]!;
      const destinations = new Map<number, number>();
      for (const edge of row.downEdges) {
        destinations.set(edge.toLane, (destinations.get(edge.toLane) ?? 0) + 1);
      }
      for (const [lane, count] of destinations) {
        if (count > 1) expect({ hash: next.commit.hash, lane }).toEqual({ hash: "c4", lane: next.lane });
      }
    }
  });

  it("keeps every continuation connected when a later tip changes preview reservations", () => {
    const input = inputFor([["c3"], ["c5"], ["c4"], ["c5"], ["c5"], []]);
    const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
    for (let index = 0; index < layout.commits.length - 1; index++) {
      const current = layout.commits[index]!;
      const next = layout.commits[index + 1]!;
      const incoming = new Set(current.downEdges.map(edge => edge.toLane));
      for (const edge of next.downEdges) {
        if (edge.fromLane !== next.lane) expect(incoming.has(edge.fromLane)).toBe(true);
      }
    }
  });

  it("marks unavoidable crossings without hiding real parent junctions", () => {
    const input = inputFor([["c6"], ["c3", "c5"], ["c5", "c7"], ["c7"], ["c6"], ["c7"], ["c7"], []]);
    const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
    expect(crossings(layout).length).toBeGreaterThan(0);
    for (const crossing of crossings(layout)) {
      const edge = layout.edges.find(edge => edge.id === `lane:${crossing.hash}:${crossing.other.fromLane}:${crossing.other.toLane}`)!;
      expect(edge.crossings?.length).toBeGreaterThan(0);
    }
    const root = layout.commits[7]!;
    const joins = layout.edges.filter(edge => edge.endRowIndex === root.rowIndex);
    expect(joins.length).toBeGreaterThan(1);
    expect(joins.every(edge => !edge.crossings?.length)).toBe(true);
  });

  it("routes WIP around newer commits when HEAD changes lanes, and reserves its track from stashes", () => {
    const input = inputFor([["c7"], ["c5"], ["c4"], ["c7"], ["c6"], ["c7"], ["c7"], []]);
    input.refs[0]!.targetHash = "c4";
    input.headState.hash = "c4";
    input.workspaceChangeCount = 3;
    input.stashes = ["c2", "c4"].map((baseHash, index) => ({
      index, refName: `stash@{${index}}`, hash: `stash${index}`, shortHash: `stash${index}`,
      date: 100, author: "tester", message: "WIP on main", parentHashes: [baseHash], baseHash,
    }));
    const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
    const workspace = layout.auxNodes.find(node => node.kind === "workspace")!;
    const head = layout.commits.find(row => row.isHead)!;
    expect(workspace.lane).not.toBe(head.lane);
    expect(layout.rows.filter(row => row.rowIndex > workspace.rowIndex && row.rowIndex < head.rowIndex)
      .some(row => row.lane === workspace.lane)).toBe(false);
    const stem = layout.edges.find(edge => edge.id === "aux:workspace:c4:stem")!;
    expect(stem.path).toBe(`M${workspace.x},${workspace.y}L${workspace.x},${head.y - head.height}`);
    expect(layout.edges.find(edge => edge.id === "aux:workspace:c4")?.path.endsWith(`${head.x},${head.y}`)).toBe(true);
  });

  it("keeps every displayed parent reachable without touching an unrelated commit", () => {
    let seed = 731;
    const random = (n: number) => ((seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) % n);
    for (let fixture = 0; fixture < 100; fixture++) {
      const count = 18;
      const parents = Array.from({ length: count }, (_, index) => index === count - 1 ? [] : [
        ...new Set(Array.from({ length: 1 + random(3) }, () => `c${index + 1 + random(count - index - 1)}`)),
      ]);
      const input = inputFor(parents);
      const layout = layoutHistoryGraph(normalizeHistoryGraph(input));
      for (const child of layout.commits) {
        const destinations = new Set<string>();
        let lanes = new Set([child.lane]);
        for (let index = child.rowIndex; index < layout.commits.length - 1; index++) {
          const current = layout.commits[index]!;
          const next = layout.commits[index + 1]!;
          lanes = new Set(current.downEdges.filter(edge => lanes.has(edge.fromLane)).map(edge => edge.toLane));
          if (lanes.delete(next.lane)) destinations.add(next.commit.hash);
        }
        expect([...destinations].sort(), `fixture ${fixture}, child ${child.commit.hash}`).toEqual([...child.commit.parents].sort());
      }
    }
  });
});

function route(id: string, x1: number, y1: number, x2: number, y2: number): HistoryGraphEdgeRoute {
  return { id, start: { x: x1, y: y1 }, end: { x: x2, y: y2 }, color: "#29b7d6", startRowIndex: 0, endRowIndex: 1 };
}

describe("history graph crossing geometry", () => {
  it("cuts only the turning line where it crosses a straight track", () => {
    const edges = layoutHistoryGraphEdges([route("turn", 0, 0, 44, 34), route("straight", 22, 0, 22, 34)]);
    expect(edges[0]!.crossings).toHaveLength(1);
    expect(edges[0]!.crossings![0]!.x).toBeCloseTo(22);
    expect(edges[0]!.crossings![0]!.y).toBeCloseTo(17);
    expect(edges[1]!.crossings).toBeUndefined();
  });

  it("handles two turning lines and long WIP or stash connections", () => {
    const curves = layoutHistoryGraphEdges([route("left", 0, 0, 44, 34), route("right", 44, 0, 0, 34)]);
    expect(curves.flatMap(edge => edge.crossings ?? [])).toEqual([{ x: 22, y: 17 }]);
    const long = layoutHistoryGraphEdges([route("wip", 22, 0, 22, 136), route("merge", 0, 34, 44, 68)]);
    expect(long[1]!.crossings).toHaveLength(1);
    expect(long[1]!.crossings![0]!.x).toBeCloseTo(22);
    expect(long[1]!.crossings![0]!.y).toBeCloseTo(51);
  });

  it("does not cut branches that share a source or parent", () => {
    const edges = layoutHistoryGraphEdges([
      route("fork-left", 22, 0, 0, 34), route("fork-right", 22, 0, 44, 34),
      route("join-left", 0, 34, 22, 68), route("join-right", 44, 34, 22, 68),
    ]);
    expect(edges.every(edge => !edge.crossings?.length)).toBe(true);
  });
});
