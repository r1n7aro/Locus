import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { branchTargetName, resolveBranchDblclickAction, resolveBranchTargetHash, resolveCommitBranchTargets } from "../components/collab/branchInteraction";
import type { GitCommitInfo, GitGraphRef } from "../types";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function localRef(name: string, targetHash: string): GitGraphRef {
  return {
    fullName: `refs/heads/${name}`,
    shortName: name,
    targetHash,
    kind: "localBranch",
    isCurrent: false,
    branchName: name,
    remoteName: null,
  };
}

function remoteRef(remoteName: string, name: string, targetHash: string): GitGraphRef {
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

describe("branch interaction", () => {
  const commit: GitCommitInfo = {
    hash: "abc123456789", shortHash: "abc1234", parents: [], author: "tester",
    date: 1, message: "branch tip", refs: [], isStash: false,
  };

  it("offers local and remote branches at the commit, excluding tags and remote HEAD aliases", () => {
    const refs = [
      remoteRef("origin", "feature/a", commit.hash),
      localRef("feature/a", commit.hash),
      localRef("other-commit", "elsewhere"),
      remoteRef("origin", "HEAD", commit.hash),
      { ...localRef("v1", commit.hash), kind: "tag" as const, fullName: "refs/tags/v1" },
      localRef("feature/a", commit.hash),
      remoteRef("upstream", "feature/a", commit.hash),
    ];
    expect(resolveCommitBranchTargets(commit, refs).map(branchTargetName)).toEqual([
      "feature/a", "origin/feature/a", "upstream/feature/a",
    ]);
    expect(resolveCommitBranchTargets(commit, [])).toEqual([]);
  });

  it("checks out a remote-only commit as a tracking branch", () => {
    const [target] = resolveCommitBranchTargets(commit, [remoteRef("origin", "白盒", commit.hash)]);
    expect(resolveBranchDblclickAction(target!, [])).toEqual({
      action: "checkoutTracking", branchName: "origin/白盒", targetKind: "remote",
    });
  });

  it("preserves the current branch state and does not check it out again", () => {
    const [target] = resolveCommitBranchTargets(commit, [{ ...localRef("main", commit.hash), isCurrent: true }]);
    expect(resolveBranchDblclickAction(target!, [])).toBeNull();
    expect(resolveBranchDblclickAction({
      kind: "remoteBranch", remoteName: "origin",
      branch: { name: "main", shortHash: commit.shortHash, message: commit.message },
    }, [{ name: "main", isCurrent: true, shortHash: commit.shortHash, message: commit.message }])).toBeNull();
  });

  it("keeps slash-containing remote branch names when optional metadata is absent", () => {
    const [target] = resolveCommitBranchTargets(commit, [{
      ...remoteRef("upstream", "feature/nested/name", commit.hash),
      branchName: null, remoteName: null,
    }]);
    expect(branchTargetName(target!)).toBe("upstream/feature/nested/name");
    expect(resolveBranchDblclickAction(target!, [])).toEqual({
      action: "checkoutTracking", branchName: "upstream/feature/nested/name", targetKind: "remote",
    });
  });

  it("switches a local branch on double click when it is not current", () => {
    expect(resolveBranchDblclickAction(
      {
        kind: "localBranch",
        branch: {
          name: "feature/a",
          isCurrent: false,
          shortHash: "abc1234",
          message: "feature",
        },
      },
      [],
    )).toEqual({
      action: "switch",
      branchName: "feature/a",
      targetKind: "local",
    });
  });

  it("ignores double click on the current local branch", () => {
    expect(resolveBranchDblclickAction(
      {
        kind: "localBranch",
        branch: {
          name: "main",
          isCurrent: true,
          shortHash: "abc1234",
          message: "head",
        },
      },
      [],
    )).toBeNull();
  });

  it("switches to an existing same-name local branch before tracking a remote branch", () => {
    expect(resolveBranchDblclickAction(
      {
        kind: "remoteBranch",
        remoteName: "origin",
        branch: {
          name: "feature/a",
          shortHash: "abc1234",
          message: "feature",
        },
      },
      [
        {
          name: "feature/a",
          isCurrent: false,
          shortHash: "def5678",
          message: "local feature",
        },
      ],
    )).toEqual({
      action: "switch",
      branchName: "feature/a",
      targetKind: "local",
    });
  });

  it("checks out a tracking branch when the remote branch has no local counterpart", () => {
    expect(resolveBranchDblclickAction(
      {
        kind: "remoteBranch",
        remoteName: "origin",
        branch: {
          name: "feature/a",
          shortHash: "abc1234",
          message: "feature",
        },
      },
      [],
    )).toEqual({
      action: "checkoutTracking",
      branchName: "origin/feature/a",
      targetKind: "remote",
    });
  });

  it("resolves local and remote branch clicks to graph ref hashes", () => {
    const refs = [
      localRef("feature/a", "1111111111111111111111111111111111111111"),
      remoteRef("origin", "feature/a", "2222222222222222222222222222222222222222"),
    ];

    expect(resolveBranchTargetHash(
      {
        kind: "localBranch",
        branch: {
          name: "feature/a",
          isCurrent: false,
          shortHash: "1111111",
          message: "feature",
        },
      },
      refs,
    )).toBe("1111111111111111111111111111111111111111");

    expect(resolveBranchTargetHash(
      {
        kind: "remoteBranch",
        remoteName: "origin",
        branch: {
          name: "feature/a",
          shortHash: "2222222",
          message: "remote feature",
        },
      },
      refs,
    )).toBe("2222222222222222222222222222222222222222");
  });

  it("keeps the remote branch row wired to the shared double click event", () => {
    const gitSidebar = read("src/components/collab/GitSidebar.vue");

    expect(gitSidebar).toContain("@dblclick=\"emit('branchDblclick', { kind: 'remoteBranch', remoteName, branch: rb })\"");
  });

  it("routes sidebar branch clicks through the graph selection API", () => {
    const collabView = read("src/components/CollabView.vue");
    const gitSidebar = read("src/components/collab/GitSidebar.vue");

    expect(gitSidebar).toContain('(e: "selectBranch", target: GitBranchTarget): void');
    expect(gitSidebar).toContain("@click=\"emit('selectBranch', { kind: 'remoteBranch', remoteName, branch: rb })\"");
    expect(collabView).toContain('@select-branch="onSelectBranch"');
    expect(collabView).toContain("resolveBranchTargetHash(target, graphRefs.value)");
    expect(collabView).toContain('selectHistoryInGraph({ kind: "commit", hash })');
  });

  it("hides optional tag and submodule sections when their lists are empty", () => {
    const gitSidebar = read("src/components/collab/GitSidebar.vue");

    expect(gitSidebar).toContain('v-if="props.tags.length > 0" class="sidebar-section"');
    expect(gitSidebar).toContain('v-if="props.submodules.length > 0" class="sidebar-section"');
  });
});
