// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useGlobalSearch } from "../composables/useGlobalSearch";
import { normalizeGlobalSearchSettings } from "../composables/useGlobalSearchSettings";
import { searchHighlightParts, type GlobalSearchHit, type GlobalSearchPage, type GlobalSearchRequest, type GlobalSearchTarget } from "../services/globalSearch";

const targets: GlobalSearchTarget[] = ["a", "b"].map((id) => ({ projectId: id, projectName: id, workspaceRef: { checkoutId: id, expectedGeneration: 1 } }));
const all = normalizeGlobalSearchSettings(null);
const titleOnly = { ...all, knowledgeContent: false, sessionContent: false };
function hit(id: string, kind: "session" | "knowledge" = "session"): GlobalSearchHit {
  return { id, kind, title: id, excerpt: id, field: "title", docType: null, path: null, messageId: null, archived: false, modifiedAt: 1_700_000_000_000 };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolveValue) => { resolve = resolveValue; });
  return { promise, resolve };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("global search scheduling", () => {
  it("combines title and content hits for the same document without merging different projects", async () => {
    const fetch = vi.fn(async (request: GlobalSearchRequest): Promise<GlobalSearchPage> => ({
      matches: [{ ...hit("shared-id", "knowledge"), field: request.source === "knowledgeTitle" ? "title" : "content", excerpt: request.source }],
      nextCursor: null,
    }));
    const search = useGlobalSearch(fetch);
    search.search("x", targets, { ...all, sessionTitle: false, sessionContent: false });
    await vi.advanceTimersByTimeAsync(120);
    expect(search.results.value).toHaveLength(2);
    expect(search.results.value.map((result) => result.target.projectId)).toEqual(["a", "b"]);
    expect(search.results.value.every((result) => result.excerpt === "knowledgeContent")).toBe(true);
  });
  it("debounces typing and searches only enabled fields across every supplied project", async () => {
    const fetch = vi.fn(async (_request: GlobalSearchRequest): Promise<GlobalSearchPage> => ({ matches: [], nextCursor: null }));
    const search = useGlobalSearch(fetch);
    search.search("中", targets, titleOnly);
    await vi.advanceTimersByTimeAsync(60);
    search.search("中文", targets, titleOnly);
    await vi.advanceTimersByTimeAsync(119);
    expect(fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(fetch).toHaveBeenCalledTimes(6);
    expect(new Set(fetch.mock.calls.map(([request]) => request.workspaceRef.checkoutId))).toEqual(new Set(["a", "b"]));
    expect(fetch.mock.calls.every(([request]) => request.query === "中文" && request.source.endsWith("Title"))).toBe(true);
    expect(search.searching.value).toBe(false);
  });

  it("never publishes stale results or overlaps obsolete requests in the same family", async () => {
    const old = deferred<GlobalSearchPage>();
    const fetch = vi.fn((request: GlobalSearchRequest) => request.query === "old" ? old.promise : Promise.resolve({ matches: [hit(request.query)], nextCursor: null }));
    const search = useGlobalSearch(fetch);
    const settings = { ...all, knowledgeTitle: false, knowledgeContent: false, sessionContent: false };
    search.search("old", [targets[0]!], settings);
    await vi.advanceTimersByTimeAsync(120);
    search.search("new", [targets[1]!], settings);
    await vi.advanceTimersByTimeAsync(120);
    expect(fetch).toHaveBeenCalledTimes(1);
    old.resolve({ matches: [hit("old")], nextCursor: "stale-next" });
    await vi.advanceTimersByTimeAsync(0);
    expect(search.results.value.map((result) => result.title)).toEqual(["new"]);
    expect(search.results.value[0]?.target.projectId).toBe("b");
    expect(fetch.mock.calls.some(([request]) => request.cursor === "stale-next")).toBe(false);
  });

  it("continues empty pages, bounds automatic results and resumes explicitly", async () => {
    let page = 0;
    const fetch = vi.fn(async (): Promise<GlobalSearchPage> => {
      page++;
      return { matches: page === 1 ? [] : Array.from({ length: 20 }, (_, i) => hit(`${page}-${i}`, "knowledge")), nextCursor: page < 5 ? String(page) : null };
    });
    const search = useGlobalSearch(fetch);
    search.search("query", [targets[0]!], { ...all, knowledgeContent: false, sessionTitle: false, sessionContent: false });
    await vi.advanceTimersByTimeAsync(120);
    expect(search.results.value).toHaveLength(40);
    expect(fetch).toHaveBeenCalledTimes(3);
    expect(search.hasMore.value).toBe(true);
    search.loadMore();
    await vi.advanceTimersByTimeAsync(0);
    expect(search.results.value).toHaveLength(80);
    expect(search.hasMore.value).toBe(false);
  });

  it("keeps other project results when one project fails and preserves project identity", async () => {
    const fetch = vi.fn(async (request: GlobalSearchRequest): Promise<GlobalSearchPage> => {
      if (request.workspaceRef.checkoutId === "a") throw new Error("unavailable");
      return { matches: [hit("same-id", "knowledge")], nextCursor: null };
    });
    const search = useGlobalSearch(fetch);
    search.search("x", targets, { ...all, knowledgeContent: false, sessionTitle: false, sessionContent: false });
    await vi.advanceTimersByTimeAsync(120);
    expect(search.results.value[0]?.target.projectId).toBe("b");
    expect(search.errors.value[0]).toContain("a:");
    expect(search.empty.value).toBe(false);
  });

  it("does not issue requests after closing or disabling search", async () => {
    const fetch = vi.fn(async (): Promise<GlobalSearchPage> => ({ matches: [], nextCursor: null }));
    const search = useGlobalSearch(fetch);
    search.search("x", targets, all);
    search.cancel();
    await vi.advanceTimersByTimeAsync(500);
    search.search("x", targets, { ...all, enabled: false });
    await vi.advanceTimersByTimeAsync(500);
    expect(fetch).not.toHaveBeenCalled();
  });
});

describe("literal highlighting and settings", () => {
  it("keeps matches near the start of long Chinese excerpts without splitting emoji", () => {
    const parts = searchHighlightParts(`${"中文🙂".repeat(50)}Needle 后文`, "needle", 24);
    expect(Array.from(parts[0]!.text)).toHaveLength(25);
    expect(parts[0]!.text.startsWith("…")).toBe(true);
    expect(parts[1]).toEqual({ text: "Needle", match: true });
    expect(parts[0]!.text).not.toMatch(/[\uD800-\uDBFF]$/);
  });
  it("highlights all literal matches without treating HTML or regex characters as markup", () => {
    const text = '<img src=x onerror=alert(1)> 中文🙂 A+b a+B';
    const parts = searchHighlightParts(text, "a+b");
    expect(parts.map((part) => part.text).join("")).toBe(text);
    expect(parts.filter((part) => part.match).map((part) => part.text)).toEqual(["A+b", "a+B"]);
    expect(searchHighlightParts(text, "中文🙂").filter((part) => part.match)[0]?.text).toBe("中文🙂");
    expect(searchHighlightParts(text, ".*")).toEqual([{ text, match: false }]);
  });
  it("restores missing settings without converting invalid strings into booleans", () => {
    expect(normalizeGlobalSearchSettings({ enabled: false, sessionContent: "false" })).toEqual({ ...all, enabled: false });
    expect(normalizeGlobalSearchSettings(null)).toEqual(all);
  });
});
