import { describe, expect, it, vi } from "vitest";
import { gitBranchAction, gitCommitAction, gitStashAction } from "../services/git";
import { ipcInvoke } from "../services/ipc";
vi.mock("../services/ipc", () => ({ ipcInvoke: vi.fn(async () => ({ status: "success" })) }));
const workspaceRef = { checkoutId: "feature-checkout", expectedGeneration: 3, expectedMaterializationEpoch: 8 };
describe("Collab menu action scope", () => {
  it("forwards mainline and tag names without losing checkout identity", async () => {
    await gitCommitAction("commit-hash", "revert", "2", undefined, workspaceRef);
    expect(ipcInvoke).toHaveBeenLastCalledWith("git_commit_action", { rev: "commit-hash", action: "revert", mode: "2", branchName: undefined, workspaceRef });
    await gitCommitAction("commit-hash", "createTag", undefined, "release/v1", workspaceRef);
    expect(ipcInvoke).toHaveBeenLastCalledWith("git_commit_action", { rev: "commit-hash", action: "createTag", mode: undefined, branchName: "release/v1", workspaceRef });
  });
  it("forwards explicit remote and stash identities", async () => {
    await gitBranchAction("upstream/feature/a", "remote", "deleteRemote", undefined, workspaceRef, "upstream");
    expect(ipcInvoke).toHaveBeenLastCalledWith("git_branch_action", { target: "upstream/feature/a", targetKind: "remote", action: "deleteRemote", newName: undefined, remoteName: "upstream", workspaceRef });
    await gitStashAction("stash@{2}", "branch", workspaceRef, { expectedHash: "stash-hash", branchName: "from-stash" });
    expect(ipcInvoke).toHaveBeenLastCalledWith("git_stash_action", { refName: "stash@{2}", action: "branch", expectedHash: "stash-hash", branchName: "from-stash", workspaceRef });
  });
});
