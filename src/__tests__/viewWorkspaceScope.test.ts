import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  isExactViewWorkspaceBinding,
  viewRead,
  viewStorageGet,
  viewTree,
  viewWorkspaceRefFromLocation,
} from "../services/view";

const runtimeMocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => runtimeMocks,
}));

describe("View checkout scope", () => {
  const checkoutA = { checkoutId: "checkout-a", expectedGeneration: 3 };
  const checkoutB = { checkoutId: "checkout-b", expectedGeneration: 7 };

  beforeEach(() => {
    runtimeMocks.invoke.mockReset();
    runtimeMocks.invoke.mockResolvedValue({});
  });

  it("keeps same view id and relative storage key isolated in IPC arguments", async () => {
    await Promise.all([
      viewRead(checkoutA, "shared-view"),
      viewRead(checkoutB, "shared-view"),
      viewStorageGet(checkoutA, { viewId: "shared-view", key: "selection" }),
      viewStorageGet(checkoutB, { viewId: "shared-view", key: "selection" }),
    ]);

    expect(runtimeMocks.invoke).toHaveBeenNthCalledWith(1, "view_read", {
      workspaceRef: checkoutA,
      viewId: "shared-view",
    });
    expect(runtimeMocks.invoke).toHaveBeenNthCalledWith(2, "view_read", {
      workspaceRef: checkoutB,
      viewId: "shared-view",
    });
    expect(runtimeMocks.invoke).toHaveBeenNthCalledWith(3, "view_storage_get", {
      workspaceRef: checkoutA,
      request: { viewId: "shared-view", key: "selection" },
    });
    expect(runtimeMocks.invoke).toHaveBeenNthCalledWith(4, "view_storage_get", {
      workspaceRef: checkoutB,
      request: { viewId: "shared-view", key: "selection" },
    });
  });

  it("preserves request ownership when checkout B finishes before checkout A", async () => {
    let resolveA!: (value: unknown) => void;
    const slowA = new Promise((resolve) => { resolveA = resolve; });
    runtimeMocks.invoke.mockImplementation((_command: string, args: { workspaceRef: typeof checkoutA }) => (
      args.workspaceRef.checkoutId === checkoutA.checkoutId
        ? slowA
        : Promise.resolve({ views: [{ id: "b" }], folders: [], order: [] })
    ));

    const pendingA = viewTree(checkoutA);
    const resultB = await viewTree(checkoutB);
    resolveA({ views: [{ id: "a" }], folders: [], order: [] });
    const resultA = await pendingA;

    expect(resultB.views[0]?.id).toBe("b");
    expect(resultA.views[0]?.id).toBe("a");
    expect(runtimeMocks.invoke.mock.calls.map((call) => call[1].workspaceRef.checkoutId))
      .toEqual(["checkout-a", "checkout-b"]);
  });

  it("parses checkout identity from a View host URL", () => {
    expect(viewWorkspaceRefFromLocation(
      "?viewHost=1&id=shared-view&checkoutId=checkout-a&workspaceGeneration=3",
    )).toEqual(checkoutA);
    expect(viewWorkspaceRefFromLocation(
      "?unityEmbed=1&windowId=view-636865636b6f75742d61-shared-view&target=view&id=shared-view",
    )).toBeNull();
    expect(viewWorkspaceRefFromLocation(
      "?viewHost=1&id=shared-view&checkoutId=checkout-a",
    )).toBeNull();
  });

  it("requires both checkout and generation to match the immutable URL binding", () => {
    expect(isExactViewWorkspaceBinding(checkoutA, {
      checkoutId: "checkout-a",
      workspaceGeneration: 3,
    })).toBe(true);
    expect(isExactViewWorkspaceBinding(checkoutA, {
      checkoutId: "checkout-a",
      workspaceGeneration: 4,
    })).toBe(false);
    expect(isExactViewWorkspaceBinding(checkoutA, {
      checkoutId: "checkout-b",
      workspaceGeneration: 3,
    })).toBe(false);
  });
});
