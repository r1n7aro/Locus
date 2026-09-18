// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import CollabView from "../components/CollabView.vue";
import WorkbenchSecondarySidebar from "../components/workbench/WorkbenchSecondarySidebar.vue";
import { createWorkbenchEditorInput } from "../stores/workbench";
import { findCollabSidebarOwner } from "../components/workbench/collabSidebarOwner";
import type { GitCommitInfo, GitHistorySnapshot, GitStashEntry } from "../types";
import type { WorkbenchEditorGroup } from "../types/workbench";

const mocks = vi.hoisted(() => ({
  gitProbe: vi.fn(), gitHistorySnapshot: vi.fn(), gitBranches: vi.fn(),
  gitSubmodules: vi.fn(), gitCheckUserConfig: vi.fn(), gitStatus: vi.fn(),
  gitCommitBody: vi.fn(), gitCommitFiles: vi.fn(), gitBranchAction: vi.fn(),
  gitCommitAction: vi.fn(), gitStashAction: vi.fn(), gitDiscardFile: vi.fn(),
  gitInitUnity: vi.fn(), gitSetUserConfig: vi.fn(), gitStage: vi.fn(),
  gitStagePaths: vi.fn(), gitStageAll: vi.fn(), gitUnstage: vi.fn(),
  gitUnstagePaths: vi.fn(), gitUnstageAll: vi.fn(),
  selectHistory: vi.fn(), addNotice: vi.fn(),
}));
vi.mock("../services/git", () => mocks);
vi.mock("../stores/project", () => ({ useProjectStore: () => ({ unityConnected: false }) }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: mocks.addNotice }) }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../composables/useDiffProgress", () => ({ useDiffProgress: () => ({ progress: ref(0), phaseLabel: ref(""), reset: vi.fn() }) }));
vi.mock("../components/GitTerminal.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/StagingArea.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/CommitDetail.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/MergeQueuePanel.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/MergeResolutionPanel.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/GitConfigPopover.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/diff/FileDiffViewer.vue", () => ({ default: defineComponent(() => () => h("div")) }));
vi.mock("../components/collab/GitGraph.vue", () => ({
  default: defineComponent({ setup(_, { expose }) {
    expose({ selectHistory: mocks.selectHistory });
    return () => h("div", { class: "test-graph" });
  } }),
}));

const workspace = { checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 };
const head: GitCommitInfo = { hash: "aaaaaaa111", shortHash: "aaaaaaa", parents: [], author: "User", date: 3, message: "head", refs: [], isStash: false };
const older: GitCommitInfo = { ...head, hash: "bbbbbbb222", shortHash: "bbbbbbb", date: 1, message: "older" };
const stash: GitStashEntry = { index: 0, refName: "stash@{0}", hash: "ccccccc333", shortHash: "ccccccc", baseHash: head.hash, parentHashes: [head.hash], message: "saved changes", author: "User", date: 4 };
function snapshot(commits = [head, older], hasMore = false): GitHistorySnapshot {
  return {
    isRepo: true, commits, hasMore,
    head: { hash: head.hash, kind: "attached", refName: "main" },
    refs: [
      { fullName: "refs/heads/main", shortName: "main", branchName: "main", kind: "localBranch", targetHash: head.hash, isCurrent: true },
      { fullName: "refs/heads/feature", shortName: "feature", branchName: "feature", kind: "localBranch", targetHash: older.hash, isCurrent: false },
      { fullName: "refs/remotes/origin/feature", shortName: "origin/feature", branchName: "feature", remoteName: "origin", kind: "remoteBranch", targetHash: older.hash, isCurrent: false },
    ],
    stashes: [stash], workspace: { changeCount: 0, unstagedCount: 0, stagedCount: 0, unmergedCount: 0 },
  };
}
const mounted: App[] = [];
async function flush() { for (let i = 0; i < 14; i++) await nextTick(); }

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.setItem("locus-display-settings", JSON.stringify({ showCollabSidebar: false }));
  mocks.gitProbe.mockResolvedValue({ available: true, inPath: true, isRepo: true });
  mocks.gitHistorySnapshot.mockResolvedValue(snapshot());
  mocks.gitBranches.mockResolvedValue({
    local: [{ name: "main", isCurrent: true, shortHash: head.shortHash, message: "head" }, { name: "feature", isCurrent: false, shortHash: older.shortHash, message: "older" }],
    remotes: [["origin", [{ name: "feature", shortHash: older.shortHash, message: "older" }]]],
  });
  mocks.gitSubmodules.mockResolvedValue([]);
  mocks.gitStatus.mockResolvedValue({ unstaged: [], staged: [], blocked: [], unmerged: [], operation: null });
  mocks.gitCheckUserConfig.mockResolvedValue({ name: "User", email: "user@example.com" });
  mocks.gitCommitFiles.mockResolvedValue([]);
  mocks.gitCommitBody.mockResolvedValue("");
  mocks.selectHistory.mockResolvedValue(true);
});
afterEach(() => {
  mounted.splice(0).forEach(app => app.unmount());
  document.body.innerHTML = "";
  localStorage.clear();
  vi.useRealTimers();
});

async function mountCollab(activate = vi.fn(async () => {})) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const sidebarTarget = ref<HTMLElement | null>(null);
  const toolbarTarget = ref<HTMLElement | null>(null);
  const showSidebar = ref(true);
  const props = reactive({ workingDir: "F:/repo", workspaceRef: { ...workspace }, isActive: false, selectedModelId: "", selectedAgentId: "", models: [] });
  const app = createApp(() => h("div", [
    h(WorkbenchSecondarySidebar, { title: "Collaboration" }, {
      default: ({ toolbarTarget: toolbar }: { toolbarTarget: HTMLElement | null }) => h("div", {
        class: "sidebar-content",
        ref: (element: any) => { sidebarTarget.value = element; toolbarTarget.value = toolbar; },
      }),
    }),
    h("div", { class: "inactive-editor", style: "display:none" }, [h(CollabView, {
      ...props, sidebarTarget: showSidebar.value ? sidebarTarget.value : null, sidebarToolbarTarget: toolbarTarget.value, activateEditor: activate,
    })]),
  ]));
  mounted.push(app);
  app.mount(host);
  await flush();
  return { host, props, activate, showSidebar };
}

describe("Collaboration secondary sidebar", () => {
  it("renders outside its inactive tab with actions in the shared header despite legacy hiding", async () => {
    const { host } = await mountCollab();
    expect(host.querySelector(".sidebar-content .git-sidebar")).not.toBeNull();
    expect(host.querySelector(".inactive-editor .git-sidebar")).toBeNull();
    expect(host.querySelectorAll(".secondary-sidebar-header .sidebar-toolbar-button")).toHaveLength(2);
    expect(host.querySelectorAll("button.branch-item")).toHaveLength(3);
    expect(host.querySelector("button.stash-item")?.textContent).toContain(stash.message);
    expect(host.querySelector(".git-sidebar .sidebar-header")).toBeNull();
    expect(mocks.gitBranches).toHaveBeenCalledWith(workspace);
  });

  it("activates its editor before locating local and remote branches without checking them out", async () => {
    const { host, activate } = await mountCollab();
    mocks.selectHistory.mockImplementation(async () => { expect(activate).toHaveBeenCalled(); return true; });
    const branches = host.querySelectorAll<HTMLButtonElement>("button.branch-item");
    branches[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await flush();
    expect(mocks.selectHistory).toHaveBeenLastCalledWith({ kind: "commit", hash: older.hash }, { scroll: true, behavior: "smooth" });
    branches[2]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(activate).toHaveBeenCalledTimes(2);
    expect(mocks.gitBranchAction).not.toHaveBeenCalled();
  });

  it("activates the owning editor when opening a stash", async () => {
    const { host, activate } = await mountCollab();
    host.querySelector<HTMLButtonElement>("button.stash-item")!.click();
    await flush();
    expect(activate).toHaveBeenCalledOnce();
    expect(mocks.selectHistory).toHaveBeenLastCalledWith({ kind: "stash", hash: stash.hash }, { scroll: true, behavior: "smooth" });
    expect(mocks.gitStashAction).not.toHaveBeenCalled();
  });

  it("loads an older branch tip using the editor checkout before jumping", async () => {
    mocks.gitHistorySnapshot.mockImplementation(async (skip: number) => skip ? snapshot([older]) : snapshot([head], true));
    const { host } = await mountCollab();
    host.querySelectorAll("button.branch-item")[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await flush();
    expect(mocks.gitHistorySnapshot).toHaveBeenCalledWith(1, expect.any(Number), workspace);
    expect(mocks.selectHistory).toHaveBeenLastCalledWith({ kind: "commit", hash: older.hash }, { scroll: true, behavior: "smooth" });
  });

  it("cancels a pending jump when the checkout assignment changes", async () => {
    let release!: () => void;
    const activate = vi.fn(() => new Promise<void>(resolve => { release = resolve; }));
    const { host, props } = await mountCollab(activate);
    host.querySelectorAll("button.branch-item")[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    props.workspaceRef = { ...workspace, expectedMaterializationEpoch: 8 };
    await flush();
    release();
    await flush();
    expect(mocks.selectHistory).not.toHaveBeenCalled();
  });

  it("refreshes a visible sidebar while its collaboration editor is inactive", async () => {
    vi.useFakeTimers();
    await mountCollab();
    mocks.gitBranches.mockClear();
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(100);
    expect(mocks.gitBranches).toHaveBeenCalledWith(workspace);
  });

  it("removes the teleported list and cancels a jump after the sidebar changes owners", async () => {
    let release!: () => void;
    const activate = vi.fn(() => new Promise<void>(resolve => { release = resolve; }));
    const { host, showSidebar } = await mountCollab(activate);
    host.querySelectorAll("button.branch-item")[1]!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    showSidebar.value = false;
    await flush();
    release();
    await flush();
    expect(host.querySelector(".git-sidebar")).toBeNull();
    expect(host.querySelector(".test-graph")).not.toBeNull();
    expect(mocks.selectHistory).not.toHaveBeenCalled();
  });

  it("keeps the original tab group as owner and follows tab moves instead of global focus", () => {
    const editor = createWorkbenchEditorInput({ kind: "section", projectId: "p", section: "collab" }, "Collaboration", { checkoutBinding: workspace });
    const session = createWorkbenchEditorInput({ kind: "session", projectId: "p", sessionId: "s" }, "Session");
    const groups: Record<string, WorkbenchEditorGroup> = {
      left: { paneId: "left", tabs: [editor, session], activeEditorId: session.editorId },
      right: { paneId: "right", tabs: [], activeEditorId: null },
    };
    const target = { editorId: editor.editorId, checkoutId: workspace.checkoutId };
    expect(findCollabSidebarOwner(groups, target)?.paneId).toBe("left");
    groups.left!.tabs = [session];
    groups.right!.tabs = [editor];
    expect(findCollabSidebarOwner(groups, target)?.paneId).toBe("right");
    expect(findCollabSidebarOwner(groups, { ...target, checkoutId: "checkout-b" })).toBeNull();
    editor.availability = "unavailable";
    expect(findCollabSidebarOwner(groups, target)).toBeNull();
    groups.right!.tabs = [];
    expect(findCollabSidebarOwner(groups, target)).toBeNull();
  });
});
