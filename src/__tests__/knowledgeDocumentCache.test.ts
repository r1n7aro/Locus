import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cacheKnowledgeDocument,
  clearKnowledgeDocumentCacheForTests,
  getCachedKnowledgeDocument,
  invalidateKnowledgeDocumentCache,
  invalidateKnowledgeDocumentCacheSubtree,
  knowledgeDocumentCacheStats,
  readKnowledgeDocumentCached,
  runKnowledgeDocumentEnrichment,
} from "../composables/knowledgeDocumentCache";
import type { KnowledgeDocument } from "../types";
import type { WorkspaceRef } from "../services/project";

const workspaceRef: WorkspaceRef = {
  checkoutId: "checkout-a",
  expectedGeneration: 3,
};

function document(body = "body"): KnowledgeDocument {
  return {
    id: "design-1",
    type: "design",
    path: "combat/core-loop.md",
    title: "Core loop",
    injectMode: "excerpt",
    effectiveInjectMode: "excerpt",
    readOnly: false,
    aiMaintained: false,
    effectiveAiMaintained: false,
    summary: null,
    maintenanceRules: null,
    effectiveMaintenanceRules: null,
    body,
    modifiedAt: 1,
  };
}

beforeEach(() => {
  clearKnowledgeDocumentCacheForTests();
});

describe("knowledgeDocumentCache", () => {
  it("isolates documents by checkout generation", () => {
    cacheKnowledgeDocument("F:/repo", workspaceRef, document("cached"));

    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/core-loop.md" },
    )?.body).toBe("cached");
    expect(getCachedKnowledgeDocument(
      "F:/repo",
      { ...workspaceRef, expectedGeneration: 4 },
      { type: "design", path: "combat/core-loop.md" },
    )).toBeNull();
  });

  it("deduplicates concurrent reads for one document", async () => {
    let resolveRead: ((value: KnowledgeDocument) => void) | undefined;
    const load = vi.fn(() => new Promise<KnowledgeDocument>((resolve) => {
      resolveRead = resolve;
    }));
    const target = { type: "design" as const, path: "combat/core-loop.md" };

    const first = readKnowledgeDocumentCached("F:/repo", workspaceRef, target, load);
    const second = readKnowledgeDocumentCached("F:/repo", workspaceRef, target, load);
    expect(load).toHaveBeenCalledTimes(1);

    resolveRead?.(document("loaded"));
    await expect(first).resolves.toMatchObject({ body: "loaded" });
    await expect(second).resolves.toMatchObject({ body: "loaded" });
    expect(knowledgeDocumentCacheStats().pendingReads).toBe(0);
  });

  it("deduplicates forced reads and retries when invalidated in flight", async () => {
    const resolvers: Array<(value: KnowledgeDocument) => void> = [];
    const load = vi.fn(() => new Promise<KnowledgeDocument>((resolve) => {
      resolvers.push(resolve);
    }));
    const target = { type: "design" as const, path: "combat/core-loop.md" };

    const first = readKnowledgeDocumentCached("F:/repo", workspaceRef, target, load);
    const forced = readKnowledgeDocumentCached(
      "F:/repo",
      workspaceRef,
      target,
      load,
      { force: true },
    );
    expect(load).toHaveBeenCalledTimes(1);

    invalidateKnowledgeDocumentCache("F:/repo", workspaceRef, target);
    resolvers[0]?.(document("stale"));
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    resolvers[1]?.(document("fresh"));

    await expect(first).resolves.toMatchObject({ body: "fresh" });
    await expect(forced).resolves.toMatchObject({ body: "fresh" });
    expect(getCachedKnowledgeDocument("F:/repo", workspaceRef, target)?.body).toBe("fresh");
  });

  it("invalidates only the changed document", () => {
    cacheKnowledgeDocument("F:/repo", workspaceRef, document("one"));
    cacheKnowledgeDocument("F:/repo", workspaceRef, {
      ...document("two"),
      id: "design-2",
      path: "combat/rules.md",
    });

    invalidateKnowledgeDocumentCache(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/core-loop.md" },
    );

    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/core-loop.md" },
    )).toBeNull();
    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/rules.md" },
    )?.body).toBe("two");
  });

  it("invalidates a directory subtree without cooling sibling documents", () => {
    cacheKnowledgeDocument("F:/repo", workspaceRef, document("root"));
    cacheKnowledgeDocument("F:/repo", workspaceRef, {
      ...document("nested"),
      id: "design-2",
      path: "combat/nested/rules.md",
    });
    cacheKnowledgeDocument("F:/repo", workspaceRef, {
      ...document("sibling"),
      id: "design-3",
      path: "other/notes.md",
    });

    invalidateKnowledgeDocumentCacheSubtree(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat" },
    );

    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/core-loop.md" },
    )).toBeNull();
    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "combat/nested/rules.md" },
    )).toBeNull();
    expect(getCachedKnowledgeDocument(
      "F:/repo",
      workspaceRef,
      { type: "design", path: "other/notes.md" },
    )?.body).toBe("sibling");
  });

  it("deduplicates history enrichment", async () => {
    let resolveWork: (() => void) | undefined;
    const work = vi.fn(() => new Promise<void>((resolve) => {
      resolveWork = resolve;
    }));
    const target = { type: "skill" as const, path: "tool/SKILL.md" };

    const first = runKnowledgeDocumentEnrichment(
      "F:/repo",
      workspaceRef,
      target,
      work,
    );
    const second = runKnowledgeDocumentEnrichment(
      "F:/repo",
      workspaceRef,
      target,
      work,
    );
    expect(work).toHaveBeenCalledTimes(1);
    resolveWork?.();
    await Promise.all([first, second]);

    await runKnowledgeDocumentEnrichment("F:/repo", workspaceRef, target, work);
    expect(work).toHaveBeenCalledTimes(1);
  });

  it("keeps invalidated enrichment work deduplicated and expires its completion", async () => {
    let resolveWork: (() => void) | undefined;
    let workCount = 0;
    const work = vi.fn(() => {
      workCount += 1;
      if (workCount > 1) return Promise.resolve();
      return new Promise<void>((resolve) => {
        resolveWork = resolve;
      });
    });
    const target = { type: "skill" as const, path: "tool/SKILL.md" };

    const first = runKnowledgeDocumentEnrichment("F:/repo", workspaceRef, target, work);
    invalidateKnowledgeDocumentCache("F:/repo", workspaceRef, target);
    const joined = runKnowledgeDocumentEnrichment("F:/repo", workspaceRef, target, work);

    expect(work).toHaveBeenCalledTimes(1);
    resolveWork?.();
    await Promise.all([first, joined]);

    await runKnowledgeDocumentEnrichment("F:/repo", workspaceRef, target, work);
    expect(work).toHaveBeenCalledTimes(2);
  });

  it("does not commit enrichment data read before invalidation", async () => {
    let resolveWork: ((value: string) => void) | undefined;
    const commit = vi.fn();
    const target = { type: "skill" as const, path: "tool/SKILL.md" };
    const pending = runKnowledgeDocumentEnrichment(
      "F:/repo",
      workspaceRef,
      target,
      () => new Promise<string>((resolve) => {
        resolveWork = resolve;
      }),
      commit,
    );

    invalidateKnowledgeDocumentCache("F:/repo", workspaceRef, target);
    resolveWork?.("stale metadata");
    await pending;

    expect(commit).not.toHaveBeenCalled();
    expect(knowledgeDocumentCacheStats()).toMatchObject({
      pendingEnrichments: 0,
      enrichmentMetadata: 0,
      epochs: 0,
    });
  });

  it("releases settled epoch and enrichment metadata on exact invalidation", async () => {
    const target = { type: "skill" as const, path: "tool/SKILL.md" };
    cacheKnowledgeDocument("F:/repo", workspaceRef, {
      ...document(),
      id: "skill-1",
      type: "skill",
      path: target.path,
    });
    await runKnowledgeDocumentEnrichment(
      "F:/repo",
      workspaceRef,
      target,
      async () => undefined,
    );
    expect(knowledgeDocumentCacheStats().enrichmentMetadata).toBe(1);

    invalidateKnowledgeDocumentCache("F:/repo", workspaceRef, target);

    expect(knowledgeDocumentCacheStats()).toMatchObject({
      documents: 0,
      enrichmentMetadata: 0,
      epochs: 0,
    });
  });
});
