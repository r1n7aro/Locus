import { beforeEach, describe, expect, it, vi } from "vitest";

const projectMocks = vi.hoisted(() => ({
  statWorkspaceEntries: vi.fn(),
}));

vi.mock("../services/project", () => projectMocks);

import {
  clearMarkdownPathStatusCache,
  loadCachedMarkdownPathStatuses,
  peekCachedMarkdownPathStatuses,
  prefetchMarkdownPathStatuses,
} from "../composables/markdownPathStatusCache";

describe("markdown path status cache", () => {
  const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 2 };
  beforeEach(() => {
    vi.resetAllMocks();
    clearMarkdownPathStatusCache();
    projectMocks.statWorkspaceEntries.mockImplementation(async (paths: string[]) => (
      paths.map((path) => ({ path, exists: true, entryKind: "file" }))
    ));
  });

  it("coalesces renderer requests from the same paint into one workspace stat", async () => {
    const first = loadCachedMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"], workspaceRef);
    const second = loadCachedMarkdownPathStatuses("F:/repo", ["b.ts", "c.ts"], workspaceRef);

    const [firstStatuses, secondStatuses] = await Promise.all([first, second]);

    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(1);
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledWith(
      ["a.ts", "b.ts", "c.ts"],
      workspaceRef,
    );
    expect([...firstStatuses.keys()]).toEqual(["a.ts", "b.ts"]);
    expect([...secondStatuses.keys()]).toEqual(["b.ts", "c.ts"]);
  });

  it("reuses fresh statuses and chunks batches at the backend limit", async () => {
    const paths = Array.from({ length: 301 }, (_, index) => `path-${index}.ts`);
    await loadCachedMarkdownPathStatuses("F:/repo", paths, workspaceRef);

    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(2);
    expect(projectMocks.statWorkspaceEntries.mock.calls[0]?.[0]).toHaveLength(300);
    expect(projectMocks.statWorkspaceEntries.mock.calls[1]?.[0]).toHaveLength(1);

    await loadCachedMarkdownPathStatuses("F:/repo", paths, workspaceRef);
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(2);
  });

  it("keeps rendering a stale status synchronously while it is revalidated", async () => {
    vi.useFakeTimers();
    try {
      await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
      expect(peekCachedMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"], workspaceRef)).toEqual({
        statuses: new Map([["a.ts", { path: "a.ts", exists: true, entryKind: "file" }]]),
        missing: ["b.ts"],
        stale: false,
      });

      vi.advanceTimersByTime(2_500);
      const stale = peekCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
      expect(stale.statuses.get("a.ts")?.exists).toBe(true);
      expect(stale).toMatchObject({ missing: [], stale: true });

      projectMocks.statWorkspaceEntries.mockResolvedValueOnce([{ path: "a.ts", exists: false, entryKind: null }]);
      const revalidated = await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
      expect(revalidated.get("a.ts")?.exists).toBe(false);
      expect(peekCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef).stale).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("prefetches only paths without a known status", async () => {
    await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
    expect(prefetchMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef)).toBeNull();

    const prefetch = prefetchMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"], workspaceRef);
    expect(prefetch).toBeInstanceOf(Promise);
    await prefetch;
    expect(projectMocks.statWorkspaceEntries).toHaveBeenLastCalledWith(["b.ts"], workspaceRef);
    expect(peekCachedMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"], workspaceRef).missing).toEqual([]);
  });

  it("keeps a stale status when its revalidation fails", async () => {
    vi.useFakeTimers();
    try {
      await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
      vi.advanceTimersByTime(2_500);
      projectMocks.statWorkspaceEntries.mockRejectedValueOnce(new Error("offline"));
      const statuses = await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
      expect(statuses.get("a.ts")?.exists).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits for a stat already in flight instead of sending its paths again", async () => {
    let finishStat!: () => void;
    projectMocks.statWorkspaceEntries.mockImplementationOnce((paths: string[]) => new Promise((resolve) => {
      finishStat = () => resolve(paths.map((path) => ({ path, exists: true, entryKind: "file" })));
    }));

    const prefetch = prefetchMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
    await Promise.resolve();
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(1);

    const render = loadCachedMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"], workspaceRef);
    await Promise.resolve();
    await Promise.resolve();
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(2);
    expect(projectMocks.statWorkspaceEntries).toHaveBeenLastCalledWith(["b.ts"], workspaceRef);

    finishStat();
    await prefetch;
    const statuses = await render;
    expect([...statuses.keys()]).toEqual(["a.ts", "b.ts"]);
  });

  it("separates statuses by workspace scope", async () => {
    await loadCachedMarkdownPathStatuses("F:/repo", ["a.ts"], workspaceRef);
    const other = { checkoutId: "checkout-b", expectedGeneration: 1 };
    expect(peekCachedMarkdownPathStatuses("F:/repo", ["a.ts"], other).missing).toEqual(["a.ts"]);
  });
});
