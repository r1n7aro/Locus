import { beforeEach, describe, expect, it, vi } from "vitest";

const projectMocks = vi.hoisted(() => ({
  statWorkspaceEntries: vi.fn(),
}));

vi.mock("../services/project", () => projectMocks);

import {
  clearMarkdownPathStatusCache,
  loadCachedMarkdownPathStatuses,
} from "../composables/markdownPathStatusCache";

describe("markdown path status cache", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    clearMarkdownPathStatusCache();
    projectMocks.statWorkspaceEntries.mockImplementation(async (paths: string[]) => (
      paths.map((path) => ({ path, exists: true, entryKind: "file" }))
    ));
  });

  it("coalesces renderer requests from the same paint into one workspace stat", async () => {
    const first = loadCachedMarkdownPathStatuses("F:/repo", ["a.ts", "b.ts"]);
    const second = loadCachedMarkdownPathStatuses("F:/repo", ["b.ts", "c.ts"]);

    const [firstStatuses, secondStatuses] = await Promise.all([first, second]);

    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(1);
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledWith(["a.ts", "b.ts", "c.ts"]);
    expect([...firstStatuses.keys()]).toEqual(["a.ts", "b.ts"]);
    expect([...secondStatuses.keys()]).toEqual(["b.ts", "c.ts"]);
  });

  it("reuses fresh statuses and chunks batches at the backend limit", async () => {
    const paths = Array.from({ length: 301 }, (_, index) => `path-${index}.ts`);
    await loadCachedMarkdownPathStatuses("F:/repo", paths);

    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(2);
    expect(projectMocks.statWorkspaceEntries.mock.calls[0]?.[0]).toHaveLength(300);
    expect(projectMocks.statWorkspaceEntries.mock.calls[1]?.[0]).toHaveLength(1);

    await loadCachedMarkdownPathStatuses("F:/repo", paths);
    expect(projectMocks.statWorkspaceEntries).toHaveBeenCalledTimes(2);
  });
});
