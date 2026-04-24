import { describe, expect, it } from "vitest";
import type { GitCommitInfo, GitGraphRef, GitStashEntry } from "../types";
import { collectUnanchoredStashHashes, normalizeHistoryGraph } from "../components/collab/graph/normalize";
import { layoutHistoryGraph } from "../components/collab/graph/layout";
import {
  HISTORY_GRAPH_REF_CONNECTOR_GAP,
  historyGraphNodeOuterRadius,
  historyGraphRefConnectorRunout,
} from "../components/collab/graph/geometry";
import { collapseDisplayRefsForRail } from "../components/collab/graph/refs";
import type { HistoryGraphInput } from "../components/collab/graph/types";

function commit(
  hash: string,
  parents: string[],
  message: string,
  extra: Partial<GitCommitInfo> = {},
): GitCommitInfo {
  return {
    hash,
    shortHash: hash.slice(0, 7),
    parents,
    author: "tester",
    date: 1,
    message,
    refs: [],
    isStash: false,
    ...extra,
  };
}

function stash(
  index: number,
  hash: string,
  message: string,
  baseHash = "c2",
  extra: Partial<GitStashEntry> = {},
): GitStashEntry {
  return {
    index,
    refName: `stash@{${index}}`,
    hash,
    shortHash: hash.slice(0, 7),
    author: "tester",
    date: 1,
    message,
    parentHashes: extra.parentHashes ?? (baseHash ? [baseHash] : []),
    baseHash: extra.baseHash ?? baseHash,
    ...extra,
  };
}

function localRef(name: string, targetHash: string, isCurrent = false): GitGraphRef {
  return {
    fullName: `refs/heads/${name}`,
    shortName: name,
    targetHash,
    kind: "localBranch",
    isCurrent,
    branchName: name,
    remoteName: null,
  };
}

function remoteRef(name: string, targetHash: string, remoteName = "origin"): GitGraphRef {
  return {
    fullName: `refs/remotes/${remoteName}/${name}`,
    shortName: `${remoteName}/${name}`,
    targetHash,
    kind: "remoteBranch",
    isCurrent: false,
    branchName: name,
    remoteName,
  };
}

function tagRef(name: string, targetHash: string): GitGraphRef {
  return {
    fullName: `refs/tags/${name}`,
    shortName: name,
    targetHash,
    kind: "tag",
    isCurrent: false,
    branchName: null,
    remoteName: null,
  };
}

describe("history graph normalize/layout", () => {
  it("keeps stash roots out of the primary commit rows", () => {
    const commits = [
      commit("stash000", ["c2"], "WIP on master", { isStash: true, refs: ["refs/stash"] }),
      commit("stashIdx", ["c2"], "index on master: head"),
      commit("c1", ["c2"], "head"),
      commit("c2", ["c3"], "base"),
      commit("c3", [], "root"),
    ];
    const stashes = [stash(0, "stash000", "WIP on master: head", "c2", { parentHashes: ["c2", "stashIdx"] })];

    const scene = normalizeHistoryGraph({
      commits,
      stashes,
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "stash", hash: "stash000", refName: "stash@{0}" },
      workspaceChangeCount: 2,
    });

    expect(scene.primaryCommits.map(commit => commit.hash)).toEqual(["c1", "c2", "c3"]);
    expect(scene.auxNodes.map(node => node.kind)).toEqual(["workspace", "stash"]);
    expect(scene.auxNodes.find(node => node.kind === "stash")?.anchorHash).toBe("c2");
    expect(scene.selectionKind).toBe("stash");
    expect(scene.commitRefs.c1.map(ref => ref.text)).toEqual(["main"]);
  });

  it("keeps workspace pinned at the top of the visible row flow", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("stash000", ["c2"], "WIP on master", { isStash: true, refs: ["refs/stash"] }),
        commit("c1", ["c2"], "head"),
        commit("c2", ["c3"], "base"),
        commit("c3", [], "root"),
      ],
      stashes: [stash(0, "stash000", "WIP on master: head")],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "workspace" },
      workspaceChangeCount: 2,
    });

    const layout = layoutHistoryGraph(scene);
    const head = layout.commits.find(commit => commit.commit.hash === "c1");
    const workspaceNode = layout.auxNodes.find(node => node.kind === "workspace");

    expect(layout.visibleCommitCount).toBe(3);
    expect(workspaceNode).toBeTruthy();
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "workspace",
      "c1",
      "stash:stash@{0}",
      "c2",
      "c3",
    ]);
    expect(workspaceNode!.rowIndex).toBeLessThan(head!.rowIndex);
    expect(workspaceNode!.lane).toBe(head!.lane);
    expect(workspaceNode!.x).toBe(head!.x);
    expect(layout.edges.find(edge => edge.id.startsWith("aux:workspace"))?.dashed).toBe(true);
    expect(layout.edges.find(edge => edge.id.startsWith("aux:stash"))?.dashed).not.toBe(true);
    expect(layout.rails.graphWidth).toBeGreaterThan(40);
  });

  it("keeps workspace above newer side-branch tips while staying attached to head", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("f1", ["base"], "feature head"),
        commit("m1", ["base"], "main head"),
        commit("base", [], "base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "m1", true),
        localRef("feat/test", "f1"),
      ],
      headState: { hash: "m1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "workspace" },
      workspaceChangeCount: 3,
    });

    const layout = layoutHistoryGraph(scene);
    const workspaceNode = layout.auxNodes.find(node => node.kind === "workspace")!;
    const mainHead = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const featureHead = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const workspaceEdge = layout.edges.find(edge => edge.id.startsWith("aux:workspace"))!;

    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "workspace",
      "f1",
      "m1",
      "base",
    ]);
    expect(workspaceNode.rowIndex).toBe(0);
    expect(workspaceNode.rowIndex).toBeLessThan(featureHead.rowIndex);
    expect(workspaceNode.lane).toBe(mainHead.lane);
    expect(workspaceNode.x).toBe(mainHead.x);
    expect(workspaceEdge.endRowIndex).toBe(mainHead.rowIndex);
    expect(workspaceEdge.dashed).toBe(true);
  });

  it("shows the selected stash as an anchored row before its base commit", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("stash000", ["c2"], "WIP on master", { isStash: true, refs: ["refs/stash"] }),
        commit("c1", ["c2"], "head"),
        commit("c2", ["c3"], "base"),
        commit("c3", [], "root"),
      ],
      stashes: [stash(0, "stash000", "WIP on master: head")],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "stash", hash: "stash000", refName: "stash@{0}" },
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const stashNode = layout.auxNodes.find(node => node.kind === "stash")!;
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "c1",
      "stash:stash@{0}",
      "c2",
      "c3",
    ]);
    expect(stashNode.rowIndex).toBeLessThan(layout.commits.find(commit => commit.commit.hash === "c2")!.rowIndex);
    const baseCommit = layout.commits.find(commit => commit.commit.hash === "c2")!;
    const stashEdge = layout.edges.find(edge => edge.id.startsWith("aux:stash"))!;
    expect(stashNode.color).not.toBe(baseCommit.color);
    expect(stashNode.lane).toBe(baseCommit.lane - 1);
    expect(stashNode.x).toBeLessThan(baseCommit.x);
    expect(stashEdge.color).toBe(stashNode.color);
    expect(stashEdge.dashed).not.toBe(true);
  });

  it("places stash on the nearest open lane when the anchor branch is on the right", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("stash000", ["f1"], "WIP on feature", { isStash: true, refs: ["refs/stash"] }),
        commit("m2", ["m1"], "main head"),
        commit("f1", ["base"], "feature head"),
        commit("m1", ["base"], "main prev"),
        commit("base", [], "base"),
      ],
      stashes: [stash(0, "stash000", "WIP on feature", "f1")],
      refs: [
        localRef("main", "m2", true),
        localRef("feat/test", "f1"),
      ],
      headState: { hash: "m2", kind: "attached", refName: "main" },
      selectedHistory: { kind: "stash", hash: "stash000", refName: "stash@{0}" },
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const stashNode = layout.auxNodes.find(node => node.kind === "stash")!;
    const featureCommit = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const mainHead = layout.commits.find(commit => commit.commit.hash === "m2")!;

    expect(stashNode.rowIndex).toBeLessThan(featureCommit.rowIndex);
    expect(featureCommit.lane).toBeGreaterThan(mainHead.lane);
    expect(stashNode.lane).toBeGreaterThan(featureCommit.lane);
    expect(stashNode.x).toBeGreaterThan(featureCommit.x);
    expect(layout.rails.graphWidth).toBeGreaterThan(featureCommit.x);
  });

  it("shows every stash whose base commit is visible in the current window", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("stash000", ["c2"], "WIP on master", { isStash: true, refs: ["refs/stash"] }),
        commit("stash001", ["c3"], "WIP on feature", { isStash: true, refs: ["refs/stash"] }),
        commit("c1", ["c2"], "head"),
        commit("c2", ["c3"], "base 1"),
        commit("c3", [], "base 2"),
      ],
      stashes: [
        stash(0, "stash000", "WIP on master: head", "c2"),
        stash(1, "stash001", "WIP on feature: head", "c3"),
      ],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    expect(scene.auxNodes.map(node => node.id)).toEqual([
      "stash:stash@{0}",
      "stash:stash@{1}",
    ]);

    const layout = layoutHistoryGraph(scene);
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "c1",
      "stash:stash@{0}",
      "c2",
      "stash:stash@{1}",
      "c3",
    ]);
    expect(scene.auxNodes.every(node => node.kind !== "stash" || node.refs.length === 0)).toBe(true);
  });

  it("filters symbolic head refs out of the left rail", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "c1", true),
        remoteRef("HEAD", "c1"),
        remoteRef("main", "c1"),
      ],
      headState: { hash: "c1", kind: "detached", refName: null },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    expect(scene.commitRefs.c1).toHaveLength(1);
    expect(scene.commitRefs.c1[0]).toMatchObject({
      kind: "branch",
      text: "main",
      title: "main, origin/main",
    });
    expect(scene.commitRefs.c1[0].sourceMarkers).toEqual([
      { key: "local", kind: "local", title: "local" },
      { key: "remote:origin", kind: "remote", title: "origin", variant: 0 },
    ]);
  });

  it("renders remote-only branches with a remote marker and branch name", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "base"),
      ],
      stashes: [],
      refs: [
        remoteRef("release", "c1"),
      ],
      headState: { hash: "c1", kind: "detached", refName: null },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    expect(scene.commitRefs.c1).toHaveLength(1);
    expect(scene.commitRefs.c1[0]).toMatchObject({
      kind: "remote",
      text: "release",
      title: "origin/release",
    });
    expect(scene.commitRefs.c1[0].sourceMarkers).toEqual([
      { key: "remote:origin", kind: "remote", title: "origin", variant: 0 },
    ]);
  });

  it("keeps separate remote markers when multiple remotes point at the same branch", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "c1", true),
        remoteRef("main", "c1", "origin"),
        remoteRef("main", "c1", "upstream"),
      ],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    expect(scene.commitRefs.c1).toHaveLength(1);
    expect(scene.commitRefs.c1[0].sourceMarkers).toEqual([
      { key: "local", kind: "local", title: "local" },
      { key: "remote:origin", kind: "remote", title: "origin", variant: 0 },
      { key: "remote:upstream", kind: "remote", title: "upstream", variant: 1 },
    ]);
  });

  it("keeps workspace in the graph while off-page stash bases stay sidebar-only", () => {
    const input: HistoryGraphInput = {
      commits: [
        commit("c4", ["c3"], "head", { date: 300 }),
        commit("c3", ["c2"], "older", { date: 220 }),
      ],
      stashes: [stash(0, "stash000", "WIP on master: head", "missing-base", { date: 240 })],
      refs: [localRef("main", "missing-head", true)],
      headState: { hash: "missing-head", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 1,
    };
    const scene = normalizeHistoryGraph(input);
    const unanchoredStashHashes = collectUnanchoredStashHashes(input);

    expect([...unanchoredStashHashes]).toEqual(["stash000"]);
    expect(scene.auxNodes).toHaveLength(1);
    expect(scene.auxNodes[0].kind).toBe("workspace");
    expect(scene.auxNodes[0].anchorHash).toBe(null);

    const layout = layoutHistoryGraph(scene);
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "workspace",
      "c4",
      "c3",
    ]);
    expect(layout.auxNodes).toHaveLength(1);
    expect(layout.auxNodes[0].kind).toBe("workspace");
    expect(layout.edges.some(edge => edge.id.startsWith("aux:"))).toBe(false);
  });

  it("does not mark older stashes unanchored before their history range is loaded", () => {
    const input: HistoryGraphInput = {
      commits: [
        commit("c1", ["c2"], "latest", { date: 300 }),
        commit("c2", ["c3"], "newer", { date: 220 }),
      ],
      stashes: [
        stash(0, "s-new", "recent off-page stash", "missing-a", { date: 210 }),
      ],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    };
    const scene = normalizeHistoryGraph(input);
    const unanchoredStashHashes = collectUnanchoredStashHashes(input);

    expect(unanchoredStashHashes).toEqual(new Set());
    expect(scene.auxNodes).toEqual([]);
    const layout = layoutHistoryGraph(scene);
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "c1",
      "c2",
    ]);
    expect(layout.edges.some(edge => edge.id.startsWith("aux:stash"))).toBe(false);

    const nextInput: HistoryGraphInput = {
      commits: [
        commit("c1", ["c2"], "latest", { date: 300 }),
        commit("c2", ["c3"], "newer", { date: 220 }),
        commit("c3", [], "older", { date: 180 }),
      ],
      stashes: [
        stash(0, "s-new", "recent off-page stash", "c3", { date: 210 }),
      ],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "stash", hash: "s-new", refName: "stash@{0}" },
      workspaceChangeCount: 0,
    };
    const nextScene = normalizeHistoryGraph(nextInput);

    const nextLayout = layoutHistoryGraph(nextScene);
    expect(collectUnanchoredStashHashes(nextInput)).toEqual(new Set());
    expect(nextScene.auxNodes[0]).toMatchObject({
      id: "stash:stash@{0}",
      kind: "stash",
      anchorHash: "c3",
      unanchored: false,
    });
    expect(nextLayout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "c1",
      "c2",
      "stash:stash@{0}",
      "c3",
    ]);
  });

  it("marks a stash unanchored only after the loaded history window reaches its timestamp", () => {
    const input: HistoryGraphInput = {
      commits: [
        commit("c1", ["c2"], "latest", { date: 300 }),
        commit("c2", ["base"], "older", { date: 180 }),
      ],
      stashes: [
        stash(0, "s-window", "stash inside loaded window", "missing-base", { date: 210 }),
      ],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    };

    const scene = normalizeHistoryGraph(input);
    const unanchoredStashHashes = collectUnanchoredStashHashes(input);
    const layout = layoutHistoryGraph(scene);

    expect(unanchoredStashHashes).toEqual(new Set(["s-window"]));
    expect(scene.auxNodes).toEqual([]);
    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "c1",
      "c2",
    ]);
  });

  it("keeps commits in strict row order while side branches only shift graph lanes", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("a1", ["a2"], "main 1"),
        commit("b1", ["b2"], "branch 1"),
        commit("a2", ["a3"], "main 2"),
        commit("b2", ["a3"], "branch 2"),
        commit("a3", [], "base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "a1", true),
        localRef("feature/test", "b1"),
      ],
      headState: { hash: "a1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const mainCommit = layout.commits.find(commit => commit.commit.hash === "a1")!;
    const branchCommit = layout.commits.find(commit => commit.commit.hash === "b1")!;
    const nextMainCommit = layout.commits.find(commit => commit.commit.hash === "a2")!;

    expect(layout.rows.map(row => row.kind === "commit" ? row.commit.hash : row.id)).toEqual([
      "a1",
      "b1",
      "a2",
      "b2",
      "a3",
    ]);
    expect(branchCommit.lane).toBeGreaterThan(mainCommit.lane);
    expect(nextMainCommit.rowIndex).toBeGreaterThan(branchCommit.rowIndex);
  });

  it("keeps master on the left when it enters after the current feature branch", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("f2", ["f1"], "feature head"),
        commit("m2", ["m1"], "master head"),
        commit("f1", ["base"], "feature base"),
        commit("m1", ["base"], "master base"),
        commit("base", [], "shared base"),
      ],
      stashes: [],
      refs: [
        localRef("feat/test", "f2", true),
        localRef("master", "m2"),
      ],
      headState: { hash: "f2", kind: "attached", refName: "feat/test" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureHead = layout.commits.find(commit => commit.commit.hash === "f2")!;
    const masterHead = layout.commits.find(commit => commit.commit.hash === "m2")!;
    const masterBase = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const featureBase = layout.commits.find(commit => commit.commit.hash === "f1")!;

    expect(featureHead.lane).toBeGreaterThan(masterHead.lane);
    expect(masterHead.lane).toBe(1);
    expect(masterBase.lane).toBe(1);
    expect(featureBase.lane).toBeGreaterThan(masterHead.lane);
    expect(layout.edges.some(edge => edge.id === "lane:f2:2:2")).toBe(true);
  });

  it("uses preview reservations only for slot placement, not extra continuation lanes", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("f1", ["base"], "feature head"),
        commit("m1", ["base"], "master head"),
        commit("base", [], "shared base"),
      ],
      stashes: [],
      refs: [
        localRef("feat/test", "f1", true),
        localRef("master", "m1"),
      ],
      headState: { hash: "f1", kind: "attached", refName: "feat/test" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureHead = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const masterHead = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const featureRowEdges = layout.edges.filter(edge => edge.startRowIndex === featureHead.rowIndex);

    expect(featureHead.lane).toBeGreaterThan(masterHead.lane);
    expect(featureHead.downEdges.filter(edge => edge.kind === "continuation")).toHaveLength(1);
    expect(featureRowEdges.map(edge => edge.id)).toEqual([
      `lane:f1:${featureHead.lane}:${featureHead.lane}`,
    ]);
    expect(layout.visibleCommitCount).toBe(3);
  });

  it("reserves a future mainline slot before a higher-priority tip becomes visible", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("f2", ["f1"], "feature head"),
        commit("m2", ["m1"], "master head"),
        commit("f1", ["base"], "feature base"),
        commit("m1", ["base"], "master base"),
        commit("base", [], "shared base"),
      ],
      stashes: [],
      refs: [
        localRef("feat/test", "f2", true),
        localRef("master", "m2"),
      ],
      headState: { hash: "f2", kind: "attached", refName: "feat/test" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureHead = layout.commits.find(commit => commit.commit.hash === "f2")!;
    const masterHead = layout.commits.find(commit => commit.commit.hash === "m2")!;

    expect(featureHead.lane).toBeGreaterThan(masterHead.lane);
    expect(layout.edges.some(edge => edge.id === `lane:f2:${featureHead.lane}:${featureHead.lane}`)).toBe(true);
    expect(layout.edges.some(edge => edge.id === "lane:f2:1:2")).toBe(false);
  });

  it("prefers the longest visible lineage when no mainline branch name is present", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("f2", ["f1"], "feature head"),
        commit("r3", ["r2"], "release head"),
        commit("f1", ["base"], "feature base"),
        commit("r2", ["r1"], "release mid"),
        commit("r1", ["base"], "release base"),
        commit("base", [], "shared base"),
      ],
      stashes: [],
      refs: [
        localRef("feature", "f2", true),
        localRef("release", "r3"),
      ],
      headState: { hash: "f2", kind: "attached", refName: "feature" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const releaseHead = layout.commits.find(commit => commit.commit.hash === "r3")!;
    const releaseMid = layout.commits.find(commit => commit.commit.hash === "r2")!;
    const featureBase = layout.commits.find(commit => commit.commit.hash === "f1")!;

    expect(releaseHead.lane).toBe(1);
    expect(releaseMid.lane).toBe(1);
    expect(featureBase.lane).toBeGreaterThan(releaseHead.lane);
  });

  it("does not reserve phantom active lanes for later unrelated tips", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("s1", ["r0"], "short tip"),
        commit("f1", ["o0"], "feature tip"),
        commit("m2", ["m1"], "main head"),
        commit("m1", ["r0"], "main prev"),
        commit("r0", ["o0"], "recent base"),
        commit("o0", [], "old base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "m2", true),
        localRef("feature", "f1"),
        localRef("fix", "s1"),
      ],
      headState: { hash: "m2", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const shortTip = layout.commits.find(commit => commit.commit.hash === "s1")!;
    const featureTip = layout.commits.find(commit => commit.commit.hash === "f1")!;

    expect(shortTip.lane).toBe(1);
    expect(featureTip.lane).toBeGreaterThan(shortTip.lane);
    expect(layout.edges.some(edge => edge.id === "lane:s1:1:1")).toBe(true);
    expect(layout.edges.some(edge => edge.id === "lane:s1:1:2")).toBe(false);
  });

  it("keeps branch colors stable while freed inner lanes stay reserved", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("m1", ["a2", "b2"], "merge feature"),
        commit("a2", ["a3"], "main prev"),
        commit("a3", [], "shared base"),
        commit("b2", ["b1"], "feature prev"),
        commit("b1", ["a3"], "feature base"),
      ],
      stashes: [],
      refs: [
        localRef("main", "m1", true),
        localRef("feature", "b2"),
      ],
      headState: { hash: "m1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureTip = layout.commits.find(commit => commit.commit.hash === "b2")!;
    const featureBase = layout.commits.find(commit => commit.commit.hash === "b1")!;

    expect(featureTip.lane).toBe(2);
    expect(featureBase.lane).toBe(2);
    expect(featureTip.color).toBe(featureBase.color);
  });

  it("keeps first-parent color stable when other branches preclaim the shared ancestor", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("m1", ["m0"], "main head"),
        commit("f1", ["base"], "feature head"),
        commit("r2", ["r1"], "remote 2"),
        commit("r1", ["base"], "remote 1"),
        commit("m0", ["base"], "main prev"),
        commit("base", ["root"], "shared base"),
        commit("root", [], "root"),
      ],
      stashes: [],
      refs: [
        localRef("main", "m1", true),
        localRef("feature/test", "f1"),
        remoteRef("main", "r2"),
      ],
      headState: { hash: "m1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const mainHead = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const mainPrev = layout.commits.find(commit => commit.commit.hash === "m0")!;
    const sharedBase = layout.commits.find(commit => commit.commit.hash === "base")!;
    const featureHead = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const featureRowEdges = layout.edges.filter(edge => edge.startRowIndex === featureHead.rowIndex);
    const sharedBaseEdges = layout.edges.filter(edge => edge.color === sharedBase.color);
    const forkEdges = layout.edges.filter(edge => edge.startRowIndex === mainPrev.rowIndex);

    expect(mainPrev.color).toBe(mainHead.color);
    expect(sharedBase.color).toBe(mainHead.color);
    expect(sharedBase.color).not.toBe(featureHead.color);
    expect(featureRowEdges.some(edge => edge.color === featureHead.color)).toBe(true);
    expect(sharedBaseEdges.length).toBeGreaterThan(0);
    expect(forkEdges.some(edge => edge.color === mainPrev.color)).toBe(true);
    expect(forkEdges.some(edge => edge.color !== mainPrev.color)).toBe(true);
  });

  it("hands off lineage color when switching to a branch created on an older commit", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("m3", ["m2"], "master head"),
        commit("m2", ["m1"], "master prev"),
        commit("m1", ["old"], "before branch point"),
        commit("old", ["base"], "branch point"),
        commit("base", ["root"], "base"),
        commit("root", [], "root"),
      ],
      stashes: [],
      refs: [
        localRef("master", "m3"),
        localRef("分支1", "old", true),
      ],
      headState: { hash: "old", kind: "attached", refName: "分支1" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const masterHead = layout.commits.find(commit => commit.commit.hash === "m3")!;
    const masterPrev = layout.commits.find(commit => commit.commit.hash === "m2")!;
    const beforeBranchPoint = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const branchPoint = layout.commits.find(commit => commit.commit.hash === "old")!;
    const base = layout.commits.find(commit => commit.commit.hash === "base")!;
    const branchPointEdges = layout.edges.filter(edge => edge.startRowIndex === branchPoint.rowIndex);
    const beforeBranchPointEdges = layout.edges.filter(edge => edge.startRowIndex === beforeBranchPoint.rowIndex);

    expect(masterHead.lane).toBe(1);
    expect(branchPoint.lane).toBe(1);
    expect(base.lane).toBe(1);
    expect(masterHead.color).toBe(masterPrev.color);
    expect(masterPrev.color).toBe(beforeBranchPoint.color);
    expect(branchPoint.color).toBe(base.color);
    expect(branchPoint.color).not.toBe(beforeBranchPoint.color);
    expect(beforeBranchPointEdges).toHaveLength(1);
    expect(beforeBranchPointEdges[0]?.color).toBe(beforeBranchPoint.color);
    expect(branchPointEdges).toHaveLength(1);
    expect(branchPointEdges[0]?.color).toBe(branchPoint.color);
  });

  it("keeps an older create-branch row collapsed in the default refs rail", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("m4", ["m3"], "master head"),
        commit("m3", ["m2"], "master prev"),
        commit("m2", ["m1"], "branch source"),
        commit("m1", ["base"], "older"),
        commit("base", [], "base"),
      ],
      stashes: [],
      refs: [
        localRef("master", "m4"),
        localRef("分支1", "m2"),
        remoteRef("master", "m2"),
        localRef("分支2", "m1", true),
      ],
      headState: { hash: "m1", kind: "attached", refName: "分支2" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const olderBranchRow = layout.commits.find(commit => commit.commit.hash === "m2")!;
    const collapsed = collapseDisplayRefsForRail(
      olderBranchRow.refs,
      Math.max(48, layout.rails.refsWidth - 18),
    );

    expect(collapsed.visibleRefs.map(ref => ref.text)).toEqual(["分支1"]);
    expect(collapsed.hiddenRefs.map(ref => ref.text)).toEqual(["master"]);
    expect(collapsed.hiddenCount).toBe(1);
  });

  it("starts the refs rail narrower for short single-branch rows", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "base"),
      ],
      stashes: [],
      refs: [localRef("master", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "master" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);

    expect(layout.rails.refsWidth).toBeLessThan(116);
    expect(layout.rails.refsWidth).toBeGreaterThanOrEqual(88);
  });

  it("stops ref connectors before the commit node edge", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "tagged root"),
      ],
      stashes: [],
      refs: [
        localRef("main", "c1", true),
        tagRef("v0.1", "c2"),
      ],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: { kind: "commit", hash: "c1" },
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const head = layout.commits.find(commit => commit.commit.hash === "c1")!;
    const taggedRoot = layout.commits.find(commit => commit.commit.hash === "c2")!;

    expect(taggedRoot.refs.map(ref => ref.text)).toEqual(["v0.1"]);
    expect(historyGraphRefConnectorRunout(taggedRoot)).toBeLessThan(taggedRoot.x);
    expect(taggedRoot.x - historyGraphRefConnectorRunout(taggedRoot) - historyGraphNodeOuterRadius(taggedRoot))
      .toBeCloseTo(HISTORY_GRAPH_REF_CONNECTOR_GAP, 5);
    expect(historyGraphNodeOuterRadius(head)).toBeGreaterThan(historyGraphNodeOuterRadius(taggedRoot));
  });

  it("keeps an outer branch color stable even when an inner unnamed lane stays active", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("u1", ["c4"], "unnamed tip"),
        commit("f1", ["base"], "feature tip"),
        commit("m2", ["m1"], "master head"),
        commit("m1", ["c4"], "master prev"),
        commit("c4", ["c3"], "join"),
        commit("c3", ["base"], "pre-base"),
        commit("base", ["root"], "base"),
        commit("root", [], "root"),
      ],
      stashes: [],
      refs: [
        localRef("master", "m2", true),
        localRef("feat/test", "f1"),
      ],
      headState: { hash: "m2", kind: "attached", refName: "master" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureTip = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const masterHead = layout.commits.find(commit => commit.commit.hash === "m2")!;
    const preBase = layout.commits.find(commit => commit.commit.hash === "c3")!;
    const masterHeadEdges = layout.edges.filter(edge => edge.startRowIndex === masterHead.rowIndex);
    const preBaseEdges = layout.edges.filter(edge => edge.startRowIndex === preBase.rowIndex);

    expect(featureTip.lane).toBeGreaterThan(masterHead.lane);
    expect(masterHeadEdges.some(edge => edge.id === `lane:m2:${featureTip.lane}:${featureTip.lane}`)).toBe(true);
    expect(masterHeadEdges.find(edge => edge.id === `lane:m2:${featureTip.lane}:${featureTip.lane}`)?.color)
      .toBe(featureTip.color);
    expect(preBaseEdges.find(edge => edge.id === `lane:c3:${featureTip.lane}:1`)?.color)
      .toBe(featureTip.color);
  });

  it("keeps an outer branch on its lane until its own merge point", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("s1", ["mid"], "short tip"),
        commit("f1", ["base"], "feature tip"),
        commit("m2", ["m1"], "main tip"),
        commit("m1", ["mid"], "main prev"),
        commit("mid", ["prev"], "short merge point"),
        commit("prev", ["base"], "before shared base"),
        commit("base", ["root"], "shared base"),
        commit("root", [], "root"),
      ],
      stashes: [],
      refs: [
        localRef("master", "m2", true),
        localRef("feat/test", "f1"),
        localRef("fix", "s1"),
      ],
      headState: { hash: "m2", kind: "attached", refName: "master" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene);
    const featureTip = layout.commits.find(commit => commit.commit.hash === "f1")!;
    const mainPrev = layout.commits.find(commit => commit.commit.hash === "m1")!;
    const shortMergePoint = layout.commits.find(commit => commit.commit.hash === "mid")!;
    const beforeSharedBase = layout.commits.find(commit => commit.commit.hash === "prev")!;

    expect(featureTip.lane).toBeGreaterThan(mainPrev.lane);
    expect(shortMergePoint.lane).toBe(mainPrev.lane);
    expect(layout.edges.some(edge => edge.id === "lane:m1:3:3")).toBe(true);
    expect(layout.edges.some(edge => edge.id === "lane:mid:3:3")).toBe(true);
    expect(layout.edges.some(edge => edge.id === "lane:prev:3:1")).toBe(true);
    expect(beforeSharedBase.lane).toBe(mainPrev.lane);
  });

  it("keeps branch and stash colors stable when older refs enter the loaded window", () => {
    const refs = [
      localRef("main", "m1", true),
      localRef("feature/old", "f1"),
      remoteRef("release", "r1"),
    ];
    const stashes = [stash(0, "s0", "WIP on main", "m0")];

    const partialScene = normalizeHistoryGraph({
      commits: [
        commit("m1", ["m0"], "main head"),
        commit("r1", ["r0"], "remote head"),
        commit("m0", ["base"], "main prev"),
        commit("r0", [], "remote base"),
        commit("base", [], "shared base"),
      ],
      stashes,
      refs,
      headState: { hash: "m1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const fullScene = normalizeHistoryGraph({
      commits: [
        commit("m1", ["m0"], "main head"),
        commit("r1", ["r0"], "remote head"),
        commit("f1", ["f0"], "feature head"),
        commit("m0", ["base"], "main prev"),
        commit("r0", [], "remote base"),
        commit("f0", [], "feature base"),
        commit("base", [], "shared base"),
      ],
      stashes,
      refs,
      headState: { hash: "m1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const partialLayout = layoutHistoryGraph(partialScene);
    const fullLayout = layoutHistoryGraph(fullScene);
    const partialRemote = partialLayout.commits.find(commit => commit.commit.hash === "r1")!;
    const fullRemote = fullLayout.commits.find(commit => commit.commit.hash === "r1")!;
    const partialStash = partialLayout.auxNodes.find(node => node.kind === "stash")!;
    const fullStash = fullLayout.auxNodes.find(node => node.kind === "stash")!;

    expect(partialRemote.color).toBe(fullRemote.color);
    expect(partialStash.color).toBe(fullStash.color);
  });

  it("applies column width overrides to the rendered rails", () => {
    const scene = normalizeHistoryGraph({
      commits: [
        commit("c1", ["c2"], "head"),
        commit("c2", [], "base"),
      ],
      stashes: [],
      refs: [localRef("main", "c1", true)],
      headState: { hash: "c1", kind: "attached", refName: "main" },
      selectedHistory: null,
      workspaceChangeCount: 0,
    });

    const layout = layoutHistoryGraph(scene, {
      refsWidth: 180,
      messageWidth: 480,
      metaWidth: 260,
    });

    expect(layout.rails.refsWidth).toBe(180);
    expect(layout.rails.messageWidth).toBe(480);
    expect(layout.rails.metaWidth).toBe(260);
    expect(layout.rails.bodyMinWidth).toBe(180 + layout.rails.graphWidth + 480 + 260);
  });
});
