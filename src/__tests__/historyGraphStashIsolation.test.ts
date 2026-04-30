import { describe, expect, it } from "vitest";
import { layoutHistoryGraph } from "../components/collab/graph/layout";
import { collectUnanchoredStashHashes, normalizeHistoryGraph } from "../components/collab/graph/normalize";
import type { HistoryGraphInput } from "../components/collab/graph/types";

function makeCommit(
  hash: string,
  parents: string[],
  refs: string[] = [],
) {
  return {
    hash,
    shortHash: hash.slice(0, 8),
    parents,
    author: "tester",
    date: 1,
    message: hash,
    refs,
    isStash: false,
  };
}

function makeExampleProjectInput(): HistoryGraphInput {
  return {
    commits: [
      makeCommit("63c0ffbe", ["9eea004e"], ["feat/test"]),
      makeCommit("06bc99e5", ["50941eec"], ["HEAD -> master", "origin/master", "origin/HEAD"]),
      makeCommit("50941eec", ["de937dc5"]),
      makeCommit("de937dc5", ["bbca96a5"]),
      makeCommit("bbca96a5", ["eccbe622"]),
      makeCommit("eccbe622", ["3701d283"]),
      makeCommit("3701d283", ["220e35e0"]),
      makeCommit("220e35e0", ["9eea004e"]),
      makeCommit("9eea004e", []),
    ],
    stashes: [
      {
        index: 0,
        refName: "stash@{0}",
        hash: "22b0cab4",
        shortHash: "22b0cab4",
        author: "tester",
        date: 2,
        message: "WIP on master: f66d66fc",
        parentHashes: ["f66d66fc", "90301996"],
        baseHash: "f66d66fc",
      },
    ],
    refs: [
      {
        fullName: "refs/heads/feat/test",
        shortName: "feat/test",
        targetHash: "63c0ffbe",
        kind: "localBranch",
        isCurrent: false,
        remoteName: null,
        branchName: "feat/test",
      },
      {
        fullName: "refs/heads/master",
        shortName: "master",
        targetHash: "06bc99e5",
        kind: "localBranch",
        isCurrent: true,
        remoteName: "origin",
        branchName: "master",
      },
      {
        fullName: "refs/remotes/origin/master",
        shortName: "origin/master",
        targetHash: "06bc99e5",
        kind: "remoteBranch",
        isCurrent: false,
        remoteName: "origin",
        branchName: "master",
      },
    ],
    headState: {
      hash: "06bc99e5",
      kind: "attached",
      refName: "master",
    },
    selectedHistory: null,
    workspaceChangeCount: 0,
  };
}

describe("history graph stash isolation", () => {
  it("keeps unanchored stash lineage out of the graph while preserving sidebar status", () => {
    const input = makeExampleProjectInput();

    const scene = normalizeHistoryGraph(input);
    const unanchoredStashHashes = collectUnanchoredStashHashes(input);

    expect(scene.primaryCommits.map(commit => commit.hash)).not.toContain("f66d66fc");
    expect(unanchoredStashHashes).toEqual(new Set(["22b0cab4"]));
    expect(scene.auxNodes).toEqual([]);

    const layout = layoutHistoryGraph(scene);

    expect(layout.rows[0]).toEqual(expect.objectContaining({
      kind: "commit",
      commit: expect.objectContaining({ hash: "63c0ffbe" }),
    }));
    expect(layout.commits.map(commit => commit.commit.hash)).not.toContain("f66d66fc");
    expect(layout.edges.some(edge => edge.id.startsWith("aux:stash"))).toBe(false);
  });

  it("keeps example project graph on two real commit lanes after stash isolation", () => {
    const input = makeExampleProjectInput();
    const scene = normalizeHistoryGraph(input);
    const layout = layoutHistoryGraph(scene);

    expect(new Set(layout.commits.map(commit => commit.lane))).toEqual(new Set([1, 2]));
    expect(Math.max(...layout.commits.map(commit => commit.lane))).toBe(2);
    expect(layout.edges.some(edge => edge.id.includes(":3:"))).toBe(false);
  });
});
