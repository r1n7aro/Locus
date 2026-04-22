import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { resolveBranchDblclickAction } from "../components/collab/branchInteraction";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("branch interaction", () => {
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

  it("keeps the remote branch row wired to the shared double click event", () => {
    const gitSidebar = read("src/components/collab/GitSidebar.vue");

    expect(gitSidebar).toContain("@dblclick=\"emit('branchDblclick', { kind: 'remoteBranch', remoteName, branch: rb })\"");
  });
});
