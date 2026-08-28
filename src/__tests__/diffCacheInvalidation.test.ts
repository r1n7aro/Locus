import { beforeEach, describe, expect, it, vi } from "vitest";

const ipcInvokeMock = vi.hoisted(() => vi.fn());

vi.mock("../services/ipc", () => ({
  ipcInvoke: ipcInvokeMock,
}));

import {
  computeRequestKey,
  diffSingleFile,
  invalidateDiffCacheForFiles,
} from "../services/diff";
import type { FileDiffRequest } from "../types";
import type { WorkspaceRef } from "../services/project";

function request(filePath: string, oldPath?: string): FileDiffRequest {
  return {
    source: "chatCheckpoint",
    filePath,
    oldPath,
    sessionId: "s1",
    assistantMessageId: "m1",
    detail: "full",
  };
}

describe("invalidateDiffCacheForFiles", () => {
  beforeEach(() => {
    ipcInvokeMock.mockReset();
    ipcInvokeMock.mockImplementation(
      (_cmd: string, args: { request: FileDiffRequest }) =>
        Promise.resolve({
          key: computeRequestKey(args.request),
          filePath: args.request.filePath,
          oldPath: args.request.oldPath,
        }),
    );
  });

  it("drops cached diffs for a matching filePath and keeps others", async () => {
    const target = request("Assets/Reverted.cs");
    const other = request("Assets/Untouched.cs");
    await diffSingleFile(target);
    await diffSingleFile(other);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(2);

    // Both served from the LRU cache now.
    await diffSingleFile(target);
    await diffSingleFile(other);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(2);

    invalidateDiffCacheForFiles(["Assets/Reverted.cs"]);

    await diffSingleFile(target);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(3);
    await diffSingleFile(other);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(3);
  });

  it("matches renamed rows by oldPath too", async () => {
    const renamed = request("Assets/NewName.cs", "Assets/OldName.cs");
    await diffSingleFile(renamed);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(1);

    invalidateDiffCacheForFiles(["Assets/OldName.cs"]);

    await diffSingleFile(renamed);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(2);
  });

  it("ignores an empty path list", async () => {
    const target = request("Assets/Stable.cs");
    await diffSingleFile(target);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(1);

    invalidateDiffCacheForFiles([]);

    await diffSingleFile(target);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(1);
  });

  it("namespaces identical relative paths by checkout generation", async () => {
    const target = request("Assets/Shared.cs");
    const checkoutA: WorkspaceRef = {
      checkoutId: "checkout-a",
      expectedGeneration: 3,
    };
    const checkoutB: WorkspaceRef = {
      checkoutId: "checkout-b",
      expectedGeneration: 8,
    };

    await diffSingleFile(target, checkoutA);
    await diffSingleFile(target, checkoutB);

    expect(computeRequestKey(target, checkoutA)).not.toBe(computeRequestKey(target, checkoutB));
    expect(ipcInvokeMock).toHaveBeenNthCalledWith(1, "diff_single_file", {
      request: target,
      workspaceRef: checkoutA,
    });
    expect(ipcInvokeMock).toHaveBeenNthCalledWith(2, "diff_single_file", {
      request: target,
      workspaceRef: checkoutB,
    });
  });
});
