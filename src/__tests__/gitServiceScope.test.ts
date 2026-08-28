import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  gitCommitAction,
  gitHistorySnapshot,
  gitStagePaths,
} from "../services/git";
import type { WorkspaceRef } from "../services/project";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef: WorkspaceRef = {
  checkoutId: "checkout-feature",
  expectedGeneration: 17,
};

describe("Git workspace scope", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
  });

  it("forwards WorkspaceRef on history reads", async () => {
    await gitHistorySnapshot(20, 50, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("git_history_snapshot", {
      skip: 20,
      limit: 50,
      workspaceRef,
    });
  });

  it("forwards WorkspaceRef on worktree mutations", async () => {
    await gitStagePaths(["Assets/A.cs", "Assets/B.cs"], workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("git_stage_paths", {
      paths: ["Assets/A.cs", "Assets/B.cs"],
      workspaceRef,
    });
  });

  it("forwards WorkspaceRef on repository actions", async () => {
    await gitCommitAction("abc123", "reset", "hard", undefined, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("git_commit_action", {
      rev: "abc123",
      action: "reset",
      mode: "hard",
      branchName: undefined,
      workspaceRef,
    });
  });
});
