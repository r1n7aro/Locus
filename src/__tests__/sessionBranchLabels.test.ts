import { beforeEach, describe, expect, it, vi } from "vitest";
import { computed } from "vue";
import { gitHistorySnapshot } from "../services/git";
import { projectCollaborationSnapshot } from "../services/workspaceExplorer";
import { listWorktreeBranches } from "../services/worktrees";
import {
  beginWorkspaceGitHeadObservation,
  differingSessionBranchLabel,
  rememberWorkspaceGitHead,
  workspaceGitHead,
} from "../services/workspaceGitHead";
import type { GitHeadState, GitHistorySnapshot, SessionExecutionTarget } from "../types";
import type { WorkspaceRef } from "../services/project";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));

const main: GitHeadState = { kind: "attached", refName: "main", hash: "a".repeat(40) };
const feature: GitHeadState = { kind: "attached", refName: "feature/ui", hash: "b".repeat(40) };
let sequence = 0;
let workspace: WorkspaceRef;

function target(branchRef?: string | null, headOid?: string | null): SessionExecutionTarget {
  return { checkoutId: workspace.checkoutId, branchRef, headOid };
}

function snapshot(head: GitHeadState): GitHistorySnapshot {
  return { head, isRepo: true, commits: [], hasMore: false, refs: [], stashes: [],
    workspace: { changeCount: 0, unstagedCount: 0, stagedCount: 0, unmergedCount: 0 } };
}

function remember(head: GitHeadState): void {
  rememberWorkspaceGitHead(workspace, head, beginWorkspaceGitHeadObservation());
}

beforeEach(() => {
  mocks.invoke.mockReset();
  workspace = { checkoutId: `branch-test-${++sequence}`, expectedGeneration: 2, expectedMaterializationEpoch: 1 };
});

describe("session branch comparison", () => {
  it("shows the recorded branch after switching branches in the same checkout", () => {
    const session = target("refs/heads/main", main.hash);
    expect(differingSessionBranchLabel(session, main)).toBe("");
    expect(differingSessionBranchLabel(session, feature)).toBe("main");
    expect(differingSessionBranchLabel(session, main)).toBe("");
  });

  it("compares branch names rather than checkout identities or commit hashes", () => {
    expect(differingSessionBranchLabel({ ...target("refs/heads/main", feature.hash), checkoutId: "sibling" }, main)).toBe("");
    expect(differingSessionBranchLabel(target(" feature/ui "), main)).toBe("feature/ui");
    expect(differingSessionBranchLabel(target("Main"), main)).toBe("Main");
  });

  it("keeps unknown historical branches empty and handles detached HEADs", () => {
    expect(differingSessionBranchLabel(undefined, main)).toBe("");
    expect(differingSessionBranchLabel(target(), main)).toBe("");
    expect(differingSessionBranchLabel(target("main"), undefined)).toBe("");
    expect(differingSessionBranchLabel(target("main"), { kind: "detached", refName: null, hash: null })).toBe("");
    const detached: GitHeadState = { kind: "detached", refName: null, hash: main.hash };
    expect(differingSessionBranchLabel(target(null, main.hash), detached)).toBe("");
    expect(differingSessionBranchLabel(target(null, feature.hash), detached)).toBe("bbbbbbbb");
    expect(differingSessionBranchLabel(target("main"), detached)).toBe("main");
    expect(differingSessionBranchLabel(target(null, main.hash), main)).toBe("aaaaaaaa");
  });
});

describe("workspace HEAD reuse", () => {
  it("uses existing Git responses without extra IPC, including repeated renders", async () => {
    mocks.invoke.mockResolvedValue(snapshot(main));
    await gitHistorySnapshot(0, 100, workspace);
    const label = computed(() => differingSessionBranchLabel(target("feature/ui"), workspaceGitHead(workspace)));
    for (let row = 0; row < 1000; row++) expect(label.value).toBe("feature/ui");
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("git_history_snapshot", { skip: 0, limit: 100, workspaceRef: workspace });
  });

  it("seeds the initial branch from the existing project collaboration request", async () => {
    mocks.invoke.mockResolvedValue({ projectId: "project", checkouts: [{ checkoutId: workspace.checkoutId,
      workspaceGeneration: 2, root: "F:/Game", branchRef: "main", headOid: main.hash }] });
    await projectCollaborationSnapshot("project");
    expect(workspaceGitHead(workspace)).toEqual(main);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    mocks.invoke.mockResolvedValue(snapshot(feature));
    await gitHistorySnapshot(0, 100, workspace);
    expect(workspaceGitHead(workspace)).toEqual(feature);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("does not invalidate comparisons when HEAD is unchanged", () => {
    remember(main);
    const compare = vi.fn(() => differingSessionBranchLabel(target("feature/ui"), workspaceGitHead(workspace)));
    const label = computed(compare);
    expect(label.value).toBe("feature/ui");
    for (let refresh = 0; refresh < 20; refresh++) {
      remember({ ...main });
      expect(label.value).toBe("feature/ui");
    }
    expect(compare).toHaveBeenCalledTimes(1);
    remember(feature);
    expect(label.value).toBe("");
    expect(compare).toHaveBeenCalledTimes(2);
  });

  it("reuses the worktree selector's current branch without confusing sibling worktrees", async () => {
    mocks.invoke.mockResolvedValue([
      { current: false, branch: "feature/ui", headOid: feature.hash },
      { current: true, branch: "main", headOid: main.hash },
    ]);
    await listWorktreeBranches(workspace);
    expect(workspaceGitHead(workspace)).toEqual(main);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it("ignores late responses and keeps checkout generations and materializations separate", () => {
    const old = beginWorkspaceGitHeadObservation();
    remember(feature);
    rememberWorkspaceGitHead(workspace, main, old);
    expect(workspaceGitHead(workspace)).toEqual(feature);
    expect(workspaceGitHead({ ...workspace, checkoutId: "unrelated" })).toBeUndefined();
    expect(workspaceGitHead({ ...workspace, expectedGeneration: 3 })).toBeUndefined();
    expect(workspaceGitHead({ ...workspace, expectedMaterializationEpoch: 2 })).toBeUndefined();
  });
});
