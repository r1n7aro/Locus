// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import GitGraph from "../components/collab/GitGraph.vue";
import type { GitGraphPublicApi } from "../components/collab/gitGraphSelection";
import type { GitCommitInfo, GitGraphRef, GitHistorySelection, GitStashEntry } from "../types";

vi.mock("../i18n", () => ({ t: (key: string) => key }));

const mounted: App[] = [];
let frames: FrameRequestCallback[] = [];

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", undefined);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => frames.push(callback));
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
});

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount());
  document.body.innerHTML = "";
  localStorage.clear();
  frames = [];
  vi.unstubAllGlobals();
});

async function flushScroll() {
  frames.splice(0).forEach(callback => callback(0));
  await nextTick();
}

async function mountGraph(count = 100, withAux = false) {
  const state = reactive({
    commits: Array.from({ length: count }, (_, index): GitCommitInfo => ({
      hash: `c${index}`, shortHash: `c${index}`, parents: index < count - 1 ? [`c${index + 1}`] : [],
      author: "tester", date: 1, message: `commit ${index}`, refs: [], isStash: false,
    })),
    selectedHistory: null as GitHistorySelection | null,
    graphRefs: [] as GitGraphRef[],
    workspaceChangeCount: withAux ? 3 : 0,
    stashes: withAux ? [{
      index: 0, refName: "stash@{0}", hash: "stash0", shortHash: "stash0", author: "tester",
      date: 2, message: "WIP on main", parentHashes: ["c0"], baseHash: "c0",
    }] as GitStashEntry[] : [],
  });
  const onSelectCommit = vi.fn((hash: string | null) => {
    state.selectedHistory = hash === null ? { kind: "workspace" }
      : hash === "stash0" ? { kind: "stash", hash, refName: "stash@{0}" } : { kind: "commit", hash };
  });
  const graph = ref<GitGraphPublicApi | null>(null);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({ render: () => h(GitGraph, {
    ...state, ref: graph, headState: { kind: "attached", hash: "c0", refName: "main" },
    loading: false, loadingMore: false, hasMoreCommits: false, currentBranch: "main", currentAuthor: "tester",
    onSelectCommit,
  }) });
  mounted.push(app);
  app.mount(host);
  await nextTick();
  const scroll = host.querySelector<HTMLElement>(".graph-scroll")!;
  const header = host.querySelector<HTMLElement>(".graph-header-row")!;
  Object.defineProperty(scroll, "clientHeight", { configurable: true, value: 260 });
  Object.defineProperty(scroll, "scrollHeight", { configurable: true, get: () => 24 + 22 + (state.commits.length + (withAux ? 2 : 0)) * 34 });
  Object.defineProperty(header, "offsetHeight", { configurable: true, value: 24 });
  const scrollTo = vi.fn((options?: ScrollToOptions | number, y?: number) => {
    if (typeof options === "number") scroll.scrollTop = y ?? 0;
    else if (options?.behavior !== "smooth") scroll.scrollTop = options?.top ?? 0;
  });
  scroll.scrollTo = scrollTo;
  window.dispatchEvent(new Event("resize"));
  await nextTick();
  const row = (id: string) => host.querySelector<HTMLButtonElement>(`[data-history-row-id="${id}"]`);
  return { state, host, scroll, scrollTo, graph: graph.value!, row, onSelectCommit };
}

describe("git graph interactions", () => {
  it("selects and toggles commit, stash and workspace rows without duplicate events", async () => {
    const { row, onSelectCommit } = await mountGraph(4, true);
    row("commit:c0")!.click();
    await nextTick();
    expect(row("commit:c0")!.getAttribute("aria-pressed")).toBe("true");
    row("commit:c0")!.click();
    await nextTick();
    row("stash:stash@{0}")!.click();
    await nextTick();
    expect(row("stash:stash@{0}")!.getAttribute("aria-pressed")).toBe("true");
    row("workspace")!.click();
    await nextTick();
    expect(onSelectCommit.mock.calls.map(call => call[0])).toEqual(["c0", null, "stash0", null]);
    expect(row("workspace")!.getAttribute("aria-pressed")).toBe("true");
  });

  it("leaves selection unchanged for unloaded or incorrectly typed targets", async () => {
    const { graph, onSelectCommit } = await mountGraph();
    expect(await graph.selectHistory({ kind: "commit", hash: "missing" })).toBe(false);
    expect(await graph.selectHistory({ kind: "stash", hash: "c0" })).toBe(false);
    expect(await graph.selectHistory({ kind: "workspace" })).toBe(false);
    expect(onSelectCommit).not.toHaveBeenCalled();
  });

  it("can select without scrolling and explicitly toggle the same row", async () => {
    const { graph, scrollTo, onSelectCommit } = await mountGraph();
    expect(await graph.selectHistory({ kind: "commit", hash: "c40" }, { scroll: false })).toBe(true);
    expect(await graph.selectHistory({ kind: "commit", hash: "c40" }, { toggle: true })).toBe(true);
    expect(onSelectCommit.mock.calls.map(call => call[0])).toEqual(["c40", null]);
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("still selects sidebar-only stashes and branch tips outside the loaded graph", async () => {
    const { state, graph, scrollTo, onSelectCommit } = await mountGraph(4, true);
    state.stashes[0]!.baseHash = "unloaded-base";
    state.graphRefs = [{
      fullName: "refs/heads/older", shortName: "older", kind: "localBranch", targetHash: "unloaded-tip", isCurrent: false,
    }];
    await nextTick();
    expect(await graph.selectHistory({ kind: "stash", hash: "stash0" })).toBe(false);
    expect(await graph.selectHistory({ kind: "commit", hash: "unloaded-tip" })).toBe(false);
    expect(onSelectCommit.mock.calls.map(call => call[0])).toEqual(["stash0", "unloaded-tip"]);
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("centers a selected row below the sticky header", async () => {
    const { graph, scrollTo, row } = await mountGraph();
    await graph.selectHistory({ kind: "commit", hash: "c40" });
    await nextTick();
    expect(scrollTo).toHaveBeenCalledWith({ top: 8 + 40 * 34 + 17 - (260 - 24) / 2, behavior: "auto" });
    expect(row("commit:c40")).not.toBeNull();
  });

  it("keeps the current viewport rendered until smooth scrolling actually moves", async () => {
    const { graph, scroll, row } = await mountGraph();
    await graph.selectHistory({ kind: "commit", hash: "c80" }, { behavior: "smooth" });
    await nextTick();
    expect(row("commit:c0")).not.toBeNull();
    expect(row("commit:c80")).toBeNull();
    scroll.scrollTop = 2600;
    scroll.dispatchEvent(new Event("scroll"));
    await flushScroll();
    expect(row("commit:c80")).not.toBeNull();
  });

  it("navigates with arrow keys, Home and End across virtualized rows", async () => {
    const { row, host, onSelectCommit, scroll } = await mountGraph(100, true);
    const key = async (id: string, key: string) => {
      row(id)!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      await nextTick();
      await nextTick();
    };
    row("workspace")!.focus();
    await key("workspace", "ArrowDown");
    expect(document.activeElement).toBe(row("stash:stash@{0}"));
    await key("stash:stash@{0}", "ArrowDown");
    expect(document.activeElement).toBe(row("commit:c0"));
    await key("commit:c0", "End");
    expect(document.activeElement).toBe(row("commit:c99"));
    expect(scroll.scrollTop).toBeGreaterThan(3000);
    expect(host.querySelectorAll('.graph-row[tabindex="0"]')).toHaveLength(1);
    await key("commit:c99", "Home");
    expect(document.activeElement).toBe(row("workspace"));
    await key("workspace", "ArrowUp");
    expect(document.activeElement).toBe(row("workspace"));
    expect(onSelectCommit.mock.calls.map(call => call[0])).toEqual(["stash0", "c0", "c99", null, null]);
  });

  it("retains a keyboard entry point when the selected row scrolls out of view", async () => {
    const { host, scroll } = await mountGraph();
    scroll.scrollTop = 2600;
    scroll.dispatchEvent(new Event("scroll"));
    await flushScroll();
    expect(host.querySelectorAll('.graph-row[tabindex="0"]')).toHaveLength(1);
  });

  it("does not render an empty virtual window after history is replaced with fewer rows", async () => {
    const { state, scroll, host } = await mountGraph();
    scroll.scrollTop = 2600;
    scroll.dispatchEvent(new Event("scroll"));
    await flushScroll();
    state.commits = state.commits.slice(0, 4);
    await nextTick();
    expect(host.querySelectorAll(".graph-row").length).toBeGreaterThan(0);
    expect(host.querySelectorAll('.graph-row[tabindex="0"]')).toHaveLength(1);
  });

  it("renders transparent crossing masks on the generated SVG paths", async () => {
    const { state, host } = await mountGraph(8);
    const parents = [["c6"], ["c3", "c5"], ["c5", "c7"], ["c7"], ["c6"], ["c7"], ["c7"], []];
    state.commits = state.commits.map((commit, index) => ({ ...commit, parents: parents[index]! }));
    await nextTick();
    const masks = [...host.querySelectorAll("mask")];
    expect(masks.length).toBeGreaterThan(0);
    for (const mask of masks) {
      expect(mask.querySelectorAll('circle[fill="black"]').length).toBeGreaterThan(0);
      expect(host.querySelector(`path[mask="url(#${mask.id})"]`)).not.toBeNull();
      expect(Number(mask.getAttribute("height"))).toBeLessThan(Number(host.querySelector(".graph-svg")!.getAttribute("height")));
    }
  });
});
