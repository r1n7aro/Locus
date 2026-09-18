// @vitest-environment jsdom
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import CollabView from "../components/CollabView.vue";
import type { GitContextTarget } from "../components/collab/gitContextMenu";
import type { GitCommitInfo, GitStashEntry } from "../types";

const mocks = vi.hoisted(() => ({
  state: {} as Record<string, any>, target: null as GitContextTarget | null,
  commit: vi.fn(), branch: vi.fn(), stash: vi.fn(), discard: vi.fn(), notice: vi.fn(), copy: vi.fn(), diff: vi.fn(),
}));
vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join(" ") }));
vi.mock("../composables/useCollabState", () => ({ useCollabState: () => mocks.state }));
vi.mock("../services/git", () => ({ gitCommitAction: mocks.commit, gitBranchAction: mocks.branch, gitStashAction: mocks.stash, gitDiscardFile: mocks.discard }));
vi.mock("../stores/project", () => ({ useProjectStore: () => ({ unityConnected: true }) }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: mocks.notice }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../services/unity", () => ({ selectUnityAsset: vi.fn(), openFileExternal: vi.fn(), showInFolder: vi.fn() }));
vi.mock("../services/diff", () => ({ computeRequestKey: vi.fn(() => "diff"), diffSingleFile: mocks.diff, refetchDiffByKey: vi.fn() }));
vi.mock("../services/chatDiffReviewWindow", () => ({ openFileDiffReviewWindow: vi.fn() }));
vi.mock("../services/collabSearchWindow", () => ({ COLLAB_SEARCH_SELECT_EVENT: "search", openCollabSearchWindow: vi.fn() }));
vi.mock("../composables/useDiffProgress", () => ({ useDiffProgress: () => ({ progress: ref(0), reset: vi.fn(), start: vi.fn(), stop: vi.fn() }) }));
vi.mock("../components/GitTerminal.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/GitInitOverlay.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/GitSidebar.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/GitConfigPopover.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/CommitDetail.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/MergeQueuePanel.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/MergeResolutionPanel.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/diff/FileDiffViewer.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/collab/StagingArea.vue", () => ({ default: {
  inheritAttrs: false, emits: ["fileContextmenu"],
  setup(_: unknown, { emit }: { emit: (...args: unknown[]) => void }) {
    return () => h("button", { id: "file", onContextmenu: (event: MouseEvent) => {
      if (mocks.target?.kind === "file") emit("fileContextmenu", event, mocks.target.file, mocks.target.source, new Set(mocks.target.selectedFiles.map(file => file.path)));
    } }, "file");
  },
} }));
vi.mock("../components/collab/GitGraph.vue", () => ({ default: {
  inheritAttrs: false, emits: ["historyContextmenu", "branchContextmenu"],
  setup(_: unknown, { emit }: { emit: (...args: unknown[]) => void }) {
    return () => h("button", { id: "graph", onContextmenu: (event: MouseEvent) => {
      emit(mocks.target?.kind === "localBranch" || mocks.target?.kind === "remoteBranch" ? "branchContextmenu" : "historyContextmenu", event, mocks.target);
    } }, "graph");
  },
} }));

const commit: GitCommitInfo = { hash: "abcdef0123456789", shortHash: "abcdef0", parents: ["base"], message: "test commit", author: "test", date: 1, refs: [], isStash: false };
const workspace = { checkoutId: "menu-test", expectedGeneration: 1, expectedMaterializationEpoch: 3 };
let app: App | null = null;
async function settle() { for (let i = 0; i < 12; i++) await Promise.resolve(); await nextTick(); }
async function open(target: GitContextTarget) {
  mocks.target = target;
  document.querySelector(target.kind === "file" ? "#file" : "#graph")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 30, clientY: 40 }));
  await settle();
}
function action(name: string) { return document.querySelector<HTMLButtonElement>(`[data-action="${name}"]`)!; }
async function click(name: string) { action(name).click(); await settle(); }
async function prompt(value: string) {
  const input = document.querySelector<HTMLInputElement>(".collab-git-prompt input")!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
  input.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", bubbles: true }));
  await settle();
}
async function confirm() {
  document.querySelector<HTMLButtonElement>(".commit-modal-actions .danger")!.click();
  await settle();
}
beforeEach(async () => {
  vi.clearAllMocks();
  mocks.commit.mockResolvedValue({ status: "success" });
  mocks.branch.mockResolvedValue({ status: "success" });
  mocks.stash.mockResolvedValue({ status: "success" });
  mocks.copy.mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: mocks.copy } });
  const state: Record<string, any> = {};
  for (const name of ["commits", "graphRefs", "unstagedFiles", "stagedFiles", "blockedFiles", "commitFiles", "unmergedFiles", "localBranches", "remoteBranches", "stashes", "tags", "submodules"]) state[name] = ref([]);
  for (const name of ["loading", "loadingMore", "hasMoreCommits", "initLoading", "showGitConfigModal", "gitConfigSaving", "filesLoading", "stageOperationBusy", "isMerging", "hasUnresolvedFiles"]) state[name] = ref(false);
  for (const name of ["initError", "gitProbeState", "gitHelpText", "gitConfigName", "gitConfigEmail", "gitConfigError", "currentGitAuthor", "commitBody", "draggingClass"]) state[name] = ref("");
  for (const name of ["selectedCommitHash", "selectedCommit", "mergeOperation", "containerRef", "leftAreaRef", "leftColRef"]) state[name] = ref(null);
  for (const name of ["pendingStagePaths", "pendingUnstagePaths", "expandedRemoteNames"]) state[name] = ref(new Set());
  for (const name of ["isRepo", "gitAvailable", "expandLocal", "expandRemotes", "expandStashes", "expandTags", "expandSubmodules"]) state[name] = ref(true);
  for (const name of ["initGitUnity", "saveGitConfigAndInit", "cancelGitConfig", "toggleRemote", "stageFile", "unstageFile", "stageFiles", "unstageFiles", "stageAll", "unstageAll", "loadMoreCommits", "onCommitted", "onTerminalDone", "onTerminalTouched", "onRefresh", "onVSplitterMouseDown", "onHSplitterMouseDown"]) state[name] = vi.fn(async () => {});
  Object.assign(state, { headState: ref({ kind: "attached", hash: "base", refName: "main" }), currentBranch: ref("main"), selectedHistory: ref({ kind: "workspace" }), workspaceChangeCount: ref(1), totalChanges: ref(1), leftColWidth: ref(70), terminalHeight: ref(240) });
  state.commits.value = [commit];
  mocks.state = state;
  const root = document.createElement("div");
  document.body.append(root);
  app = createApp(CollabView, { workingDir: "F:/Project", workspaceRef: workspace, isActive: true, selectedModelId: "", selectedAgentId: "", models: [] });
  app.mount(root);
  await settle();
});
afterEach(() => { app?.unmount(); app = null; document.body.innerHTML = ""; localStorage.clear(); });

describe("Collab Git menu execution", () => {
  it("dispatches cherry-pick with full hash and bound checkout", async () => {
    await open({ kind: "commit", commit });
    await click("cherryPick");
    expect(mocks.commit).toHaveBeenCalledWith(commit.hash, "cherryPick", undefined, undefined, workspace);
    expect(mocks.state.onRefresh).toHaveBeenCalledOnce();
  });
  it("validates a merge mainline before dispatching revert", async () => {
    await open({ kind: "commit", commit: { ...commit, parents: ["parent-a", "parent-b"] } });
    await click("revert");
    await prompt("3");
    expect(mocks.commit).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alert"]')).not.toBeNull();
    await prompt("2");
    expect(mocks.commit).toHaveBeenCalledWith(commit.hash, "revert", "2", undefined, workspace);
  });
  it("validates names and creates a branch without checking it out", async () => {
    await open({ kind: "commit", commit });
    await click("createBranch");
    await prompt("--force");
    expect(mocks.commit).not.toHaveBeenCalled();
    await prompt("feature/白盒");
    expect(mocks.commit).toHaveBeenCalledWith(commit.hash, "createBranch", undefined, "feature/白盒", workspace);
  });
  it("confirms remote deletion and preserves the exact remote identity", async () => {
    await open({ kind: "remoteBranch", remoteName: "team/origin", branch: { name: "feature/a", shortHash: "abc", message: "" } });
    await click("deleteRemoteBranch");
    expect(mocks.branch).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("team/origin/feature/a");
    await confirm();
    expect(mocks.branch).toHaveBeenCalledWith("team/origin/feature/a", "remote", "deleteRemote", undefined, workspace, "team/origin");
  });
  it("requires confirmation before rebasing the current branch", async () => {
    await open({ kind: "localBranch", branch: { name: "feature/a", isCurrent: false, shortHash: "abc", message: "" } });
    await click("rebaseCurrentOnto");
    expect(mocks.branch).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("main feature/a");
    await confirm();
    expect(mocks.branch).toHaveBeenCalledWith("feature/a", "local", "rebaseCurrentOnto", undefined, workspace, undefined);
  });
  it("drops multiple stashes from newest index to oldest, checking each identity", async () => {
    const stash: GitStashEntry = { ...commit, refName: "stash@{0}", index: 0, parentHashes: ["base"] };
    const older = { ...stash, index: 2, refName: "stash@{2}", hash: "older" };
    await open({ kind: "stash", stash, selectedStashes: [stash, older] });
    await click("stashDrop");
    await confirm();
    expect(mocks.stash.mock.calls).toEqual([
      ["stash@{2}", "drop", workspace, { expectedHash: "older" }],
      ["stash@{0}", "drop", workspace, { expectedHash: commit.hash }],
    ]);
  });
  it("reports clipboard failure without showing a false success", async () => {
    mocks.copy.mockRejectedValueOnce(new Error("clipboard denied"));
    await open({ kind: "commit", commit });
    await click("copyHash");
    expect(mocks.copy).toHaveBeenCalledWith(commit.hash);
    expect(mocks.notice).toHaveBeenCalledWith("error", "collab.menu.copyFailed", expect.anything());
    expect(mocks.notice.mock.calls.some(call => call[0] === "success")).toBe(false);
  });
  it("blocks repeated mutation while a Git request is in flight", async () => {
    let finish!: (value: unknown) => void;
    mocks.commit.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    await open({ kind: "commit", commit });
    await click("cherryPick");
    await open({ kind: "commit", commit });
    expect(action("cherryPick").disabled).toBe(true);
    await click("cherryPick");
    expect(mocks.commit).toHaveBeenCalledOnce();
    finish({ status: "success" });
    await settle();
    expect(action("cherryPick").disabled).toBe(false);
  });
  it("copies selected file paths and stages the same selection", async () => {
    const files = [{ path: "Assets/A.cs", status: "M", lfs: false }, { path: "Assets/B.cs", status: "M", lfs: false }];
    mocks.state.unstagedFiles.value = files;
    const target = { kind: "file" as const, source: "gitUnstaged" as const, file: files[0]!, selectedFiles: files };
    await open(target);
    await click("copyAbsolutePath");
    expect(mocks.copy).toHaveBeenCalledWith("F:/Project/Assets/A.cs\nF:/Project/Assets/B.cs");
    await open(target);
    await click("stage");
    expect(mocks.state.stageFiles).toHaveBeenCalledWith(["Assets/A.cs", "Assets/B.cs"]);
  });
  it("supports keyboard navigation and Escape", async () => {
    await open({ kind: "commit", commit });
    expect(document.activeElement).toBe(action("checkoutDetached"));
    action("checkoutDetached").dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(document.activeElement).toBe(action("copyMessage"));
    document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await settle();
    expect(document.querySelector(".collab-git-menu")).toBeNull();
  });
});
