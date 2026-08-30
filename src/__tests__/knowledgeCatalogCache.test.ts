import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearKnowledgeCatalogCacheForTests,
  invalidateKnowledgeCatalogCache,
  readKnowledgeCatalogCached,
} from "../composables/knowledgeCatalogCache";
import type { WorkspaceRef } from "../services/project";

const workspaceRef: WorkspaceRef = {
  checkoutId: "checkout-a",
  expectedGeneration: 2,
};

beforeEach(() => {
  clearKnowledgeCatalogCacheForTests();
});

describe("knowledgeCatalogCache", () => {
  it("shares one in-flight catalog read", async () => {
    let resolveRead: ((value: string[]) => void) | undefined;
    const load = vi.fn(() => new Promise<string[]>((resolve) => {
      resolveRead = resolve;
    }));

    const first = readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      load,
    );
    const second = readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      load,
    );
    expect(load).toHaveBeenCalledTimes(1);
    resolveRead?.(["one"]);
    await expect(first).resolves.toEqual(["one"]);
    await expect(second).resolves.toEqual(["one"]);
  });

  it("shares forced reads and retries an invalidated in-flight result", async () => {
    const resolvers: Array<(value: string[]) => void> = [];
    const load = vi.fn(() => new Promise<string[]>((resolve) => {
      resolvers.push(resolve);
    }));

    const first = readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      load,
      { force: true },
    );
    const second = readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      load,
      { force: true },
    );
    expect(load).toHaveBeenCalledTimes(1);

    invalidateKnowledgeCatalogCache("F:/repo", workspaceRef, "documents:design");
    resolvers[0]?.(["stale"]);
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    resolvers[1]?.(["fresh"]);

    await expect(first).resolves.toEqual(["fresh"]);
    await expect(second).resolves.toEqual(["fresh"]);
  });

  it("invalidates one catalog segment", async () => {
    const designLoad = vi.fn(async () => ["design"]);
    const memoryLoad = vi.fn(async () => ["memory"]);
    await readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      designLoad,
    );
    await readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:memory",
      memoryLoad,
    );

    invalidateKnowledgeCatalogCache(
      "F:/repo",
      workspaceRef,
      "documents:design",
    );
    await readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      designLoad,
    );
    await readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:memory",
      memoryLoad,
    );
    expect(designLoad).toHaveBeenCalledTimes(2);
    expect(memoryLoad).toHaveBeenCalledTimes(1);
  });

  it("separates workspace generations", async () => {
    const load = vi.fn(async () => ["value"]);
    await readKnowledgeCatalogCached(
      "F:/repo",
      workspaceRef,
      "documents:design",
      load,
    );
    await readKnowledgeCatalogCached(
      "F:/repo",
      { ...workspaceRef, expectedGeneration: 3 },
      "documents:design",
      load,
    );
    expect(load).toHaveBeenCalledTimes(2);
  });
});
