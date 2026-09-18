// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import GitGraph from "../components/collab/GitGraph.vue";
import type { GitCommitInfo, GitGraphRef } from "../types";

vi.mock("../i18n", () => ({ t: (key: string) => key }));

const commit: GitCommitInfo = {
  hash: "abc123456789", shortHash: "abc1234", parents: [], author: "tester",
  date: 1, message: "branch tip", refs: [], isStash: false,
};
const mounted: App[] = [];

function branchRef(name: string, remoteName?: string): GitGraphRef {
  return {
    fullName: remoteName ? `refs/remotes/${remoteName}/${name}` : `refs/heads/${name}`,
    shortName: remoteName ? `${remoteName}/${name}` : name,
    branchName: name, remoteName, targetHash: commit.hash,
    kind: remoteName ? "remoteBranch" : "localBranch", isCurrent: false,
  };
}

async function mountGraph(graphRefs: GitGraphRef[]) {
  const onBranchContextmenu = vi.fn();
  const onHistoryContextmenu = vi.fn();
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({
    render: () => h(GitGraph, {
      commits: [commit], graphRefs, headState: { kind: "detached", hash: commit.hash, refName: null },
      selectedHistory: null, loading: false, loadingMore: false, hasMoreCommits: false,
      currentBranch: "", currentAuthor: "tester", stashes: [], workspaceChangeCount: 0,
      onBranchContextmenu, onHistoryContextmenu,
    }),
  });
  mounted.push(app);
  app.mount(host);
  await nextTick();
  return { host, onBranchContextmenu, onHistoryContextmenu };
}

function rightClick(element: Element | null) {
  expect(element).not.toBeNull();
  element!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
}

afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount());
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("git graph context menus", () => {
  it.each([undefined, "origin"])("opens the branch menu from a %s branch badge without bubbling to the commit", async (remoteName) => {
    const { host, onBranchContextmenu, onHistoryContextmenu } = await mountGraph([branchRef("白盒", remoteName)]);
    rightClick(host.querySelector(".ref-badge-text"));
    expect(onBranchContextmenu).toHaveBeenCalledOnce();
    expect(onBranchContextmenu.mock.calls[0]![1]).toMatchObject({
      kind: remoteName ? "remoteBranch" : "localBranch", branch: { name: "白盒" },
      ...(remoteName ? { remoteName } : {}),
    });
    expect(onHistoryContextmenu).not.toHaveBeenCalled();
  });

  it("uses the local branch when a badge groups local and remote refs", async () => {
    const { host, onBranchContextmenu } = await mountGraph([
      branchRef("feature/a", "origin"), branchRef("feature/a"),
    ]);
    rightClick(host.querySelector(".ref-badge"));
    expect(onBranchContextmenu.mock.calls[0]![1]).toMatchObject({ kind: "localBranch", branch: { name: "feature/a" } });
  });

  it("opens the commit menu for shared remote badges so the user can choose the remote", async () => {
    const { host, onBranchContextmenu, onHistoryContextmenu } = await mountGraph([
      branchRef("feature/a", "origin"), branchRef("feature/a", "upstream"),
    ]);
    rightClick(host.querySelector(".ref-badge"));
    expect(onBranchContextmenu).not.toHaveBeenCalled();
    expect(onHistoryContextmenu).toHaveBeenCalledOnce();
    expect(onHistoryContextmenu.mock.calls[0]![1]).toEqual({ kind: "commit", commit });
  });

  it("opens the branch menu from the expanded overflow badges", async () => {
    const { host, onBranchContextmenu, onHistoryContextmenu } = await mountGraph([
      branchRef("feature/first-long-name"), branchRef("feature/second-long-name", "origin"),
      branchRef("feature/third-long-name"),
    ]);
    const badge = [...host.querySelectorAll(".graph-row-ref-badges-expanded .ref-badge")]
      .find(element => element.textContent?.includes("feature/second-long-name"));
    rightClick(badge ?? null);
    expect(onBranchContextmenu.mock.calls[0]![1]).toMatchObject({
      kind: "remoteBranch", remoteName: "origin", branch: { name: "feature/second-long-name" },
    });
    expect(onHistoryContextmenu).not.toHaveBeenCalled();
  });

  it("keeps the commit menu on messages and tags", async () => {
    const { host, onBranchContextmenu, onHistoryContextmenu } = await mountGraph([{
      fullName: "refs/tags/v1", shortName: "v1", kind: "tag", targetHash: commit.hash, isCurrent: false,
    }]);
    rightClick(host.querySelector(".graph-row-title"));
    rightClick(host.querySelector(".ref-tag"));
    expect(onBranchContextmenu).not.toHaveBeenCalled();
    expect(onHistoryContextmenu).toHaveBeenCalledTimes(2);
    expect(onHistoryContextmenu.mock.calls[1]![1]).toEqual({ kind: "commit", commit });
  });
});
