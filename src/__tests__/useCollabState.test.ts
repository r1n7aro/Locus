import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { nextTick, reactive } from "vue";
import { useCollabState } from "../composables/useCollabState";

const gitServiceMocks = vi.hoisted(() => ({
  gitBranches: vi.fn(),
  gitCheckUserConfig: vi.fn(),
  gitCommitBody: vi.fn(),
  gitCommitFiles: vi.fn(),
  gitHistorySnapshot: vi.fn(),
  gitInitUnity: vi.fn(),
  gitProbe: vi.fn(),
  gitSetUserConfig: vi.fn(),
  gitStage: vi.fn(),
  gitStagePaths: vi.fn(),
  gitStageAll: vi.fn(),
  gitStatus: vi.fn(),
  gitSubmodules: vi.fn(),
  gitUnstage: vi.fn(),
  gitUnstagePaths: vi.fn(),
  gitUnstageAll: vi.fn(),
}));

const notificationStoreMock = vi.hoisted(() => ({
  addNotice: vi.fn(),
}));

vi.mock("../services/git", () => gitServiceMocks);
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => notificationStoreMock,
}));
vi.mock("../composables/useHideMeta", () => ({
  useHideMeta: () => ({
    hideMeta: { value: false },
  }),
  withMetaCompanionPaths: (paths: string[]) => paths,
}));
vi.mock("../i18n", () => ({
  t: (key: string) => key,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}

function createStorageMock() {
  const store = new Map<string, string>();
  return {
    getItem(key: string) {
      return store.has(key) ? store.get(key)! : null;
    },
    setItem(key: string, value: string) {
      store.set(key, value);
    },
    removeItem(key: string) {
      store.delete(key);
    },
    clear() {
      store.clear();
    },
  };
}

function createDocumentMock() {
  const listeners = new Map<string, Set<(event: any) => void>>();
  const bodyClasses = new Set<string>();
  return {
    body: {
      style: {
        cursor: "",
        userSelect: "",
      },
      classList: {
        add(token: string) {
          bodyClasses.add(token);
        },
        remove(token: string) {
          bodyClasses.delete(token);
        },
        contains(token: string) {
          return bodyClasses.has(token);
        },
      },
    },
    addEventListener(type: string, handler: (event: any) => void) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(handler);
    },
    removeEventListener(type: string, handler: (event: any) => void) {
      listeners.get(type)?.delete(handler);
    },
    dispatch(type: string, event: any = {}) {
      for (const handler of listeners.get(type) ?? []) {
        handler(event);
      }
    },
  };
}

function snapshot(commits: any[], headHash: string | null, extra: Partial<any> = {}) {
  return {
    isRepo: true,
    commits,
    hasMore: false,
    head: {
      hash: headHash,
      kind: headHash ? "attached" : "detached",
      refName: headHash ? "main" : null,
    },
    refs: [],
    stashes: [],
    workspace: {
      changeCount: 0,
      unstagedCount: 0,
      stagedCount: 0,
      unmergedCount: 0,
    },
    ...extra,
  };
}

describe("useCollabState", () => {
  let documentMock: ReturnType<typeof createDocumentMock>;

  beforeEach(() => {
    vi.clearAllMocks();
    documentMock = createDocumentMock();
    vi.stubGlobal("localStorage", createStorageMock());
    vi.stubGlobal("document", documentMock);

    gitServiceMocks.gitProbe.mockResolvedValue({
      available: true,
      inPath: true,
      isRepo: false,
    });
    gitServiceMocks.gitStatus.mockResolvedValue({
      unstaged: [],
      staged: [],
      blocked: [],
      unmerged: [],
      operation: null,
    });
    gitServiceMocks.gitBranches.mockResolvedValue({
      local: [],
      remotes: [],
    });
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([], null));
    gitServiceMocks.gitSubmodules.mockResolvedValue([]);
    gitServiceMocks.gitCommitBody.mockResolvedValue("");
    gitServiceMocks.gitCommitFiles.mockResolvedValue([]);
    gitServiceMocks.gitCheckUserConfig.mockResolvedValue({
      name: "Global User",
      email: "global@example.com",
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("ignores stale git refresh results from a previous working directory", async () => {
    const firstLog = deferred<any>();
    const secondLog = deferred<any>();

    gitServiceMocks.gitHistorySnapshot
      .mockImplementationOnce(() => firstLog.promise)
      .mockImplementationOnce(() => secondLog.promise);

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo-a";
    await nextTick();
    await flushPromises();

    props.workingDir = "F:/repo-b";
    await nextTick();
    await flushPromises();

    secondLog.resolve({
      ...snapshot([{ hash: "bbbbbbb", shortHash: "bbbbbbb", parents: [], author: "tester", date: 1, message: "repo b", refs: [], isStash: false }], "bbbbbbb"),
    });
    await flushPromises();

    expect(state.isRepo.value).toBe(true);
    expect(state.headHash.value).toBe("bbbbbbb");

    firstLog.resolve({
      isRepo: false,
      commits: [],
      hasMore: false,
      head: { hash: null, kind: "detached", refName: null },
      refs: [],
      stashes: [],
      workspace: { changeCount: 0, unstagedCount: 0, stagedCount: 0, unmergedCount: 0 },
    });
    await flushPromises();

    expect(state.isRepo.value).toBe(true);
    expect(state.headHash.value).toBe("bbbbbbb");
    expect(state.commits.value).toHaveLength(1);
  });

  it("refreshes git data when the collab tab becomes active again", async () => {
    vi.useFakeTimers();

    gitServiceMocks.gitHistorySnapshot
      .mockResolvedValueOnce(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"))
      .mockResolvedValueOnce(snapshot([{ hash: "bbbbbbb", shortHash: "bbbbbbb", parents: [], author: "tester", date: 2, message: "repo b", refs: [], isStash: false }], "bbbbbbb"));

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.headHash.value).toBe("aaaaaaa");

    props.isActive = true;
    await nextTick();
    await vi.advanceTimersByTimeAsync(100);
    await flushPromises();

    expect(state.headHash.value).toBe("bbbbbbb");
    expect(state.commits.value[0]?.hash).toBe("bbbbbbb");
  });

  it("uses the snapshot hasMore flag instead of inferring from the page size", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(
      snapshot(
        [{ hash: "shortpage", shortHash: "shortpa", parents: [], author: "tester", date: 1, message: "short", refs: [], isStash: false }],
        "shortpage",
        { hasMore: true },
      ),
    );

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.hasMoreCommits.value).toBe(true);
  });

  it("derives sorted sidebar tags from graph refs", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(
      snapshot([], "aaaaaaa", {
        refs: [
          {
            fullName: "refs/tags/v2.0.0",
            shortName: "v2.0.0",
            targetHash: "bbbbbbb",
            kind: "tag",
            isCurrent: false,
            remoteName: null,
            branchName: null,
          },
          {
            fullName: "refs/heads/main",
            shortName: "main",
            targetHash: "aaaaaaa",
            kind: "localBranch",
            isCurrent: true,
            remoteName: null,
            branchName: "main",
          },
          {
            fullName: "refs/tags/v1.0.0",
            shortName: "v1.0.0",
            targetHash: "aaaaaaa",
            kind: "tag",
            isCurrent: false,
            remoteName: null,
            branchName: null,
          },
        ],
      }),
    );

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.tags.value.map(tag => tag.shortName)).toEqual(["v1.0.0", "v2.0.0"]);
  });

  it("counts unique workspace changes across staged, unstaged, untracked, and unmerged files", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));
    gitServiceMocks.gitStatus.mockResolvedValue({
      unstaged: [
        { path: "src/App.vue", status: "M", lfs: false },
        { path: "src/new-file.ts", status: "?", lfs: false },
      ],
      staged: [
        { path: "src/App.vue", status: "M", lfs: false },
        { path: "src/main.ts", status: "A", lfs: false },
      ],
      blocked: [],
      unmerged: [
        {
          path: "src/conflict.ts",
          conflictCode: "UU",
          semanticLabel: "both modified",
          baseOid: "1",
          leftOid: "2",
          rightOid: "3",
          lfs: false,
          headMode: "100644",
          stage1Mode: "100644",
          stage2Mode: "100644",
          stage3Mode: "100644",
        },
      ],
      operation: null,
    });

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.workspaceChangeCount.value).toBe(4);
  });

  it("derives current branch from the snapshot head state instead of commit refs", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue({
      ...snapshot([{ hash: "detached1", shortHash: "detached", parents: [], author: "tester", date: 1, message: "detached", refs: [], isStash: false }], "detached1"),
      head: {
        hash: "detached1",
        kind: "detached",
        refName: null,
      },
    });

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.currentBranch.value).toBe("HEAD (detached)");
  });

  it("loads current git author from user config", async () => {
    gitServiceMocks.gitCheckUserConfig.mockResolvedValue({
      name: "Repo User",
      email: "repo@example.com",
    });
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(
      snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"),
    );

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.currentGitAuthor.value).toBe("Repo User");
  });

  it("keeps files in place until stage is confirmed by a fresh git status", async () => {
    const stageOp = deferred<void>();

    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));
    gitServiceMocks.gitStatus
      .mockResolvedValueOnce({
        unstaged: [{ path: "src/App.vue", status: "M", lfs: false }],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      })
      .mockResolvedValueOnce({
        unstaged: [],
        staged: [{ path: "src/App.vue", status: "M", lfs: false }],
        blocked: [],
        unmerged: [],
        operation: null,
      });
    gitServiceMocks.gitStage.mockImplementation(() => stageOp.promise);

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    const pending = state.stageFile("src/App.vue");
    await flushPromises();

    expect(state.unstagedFiles.value.map(file => file.path)).toEqual(["src/App.vue"]);
    expect(state.stagedFiles.value).toEqual([]);
    expect(state.stageOperationBusy.value).toBe(true);
    expect([...state.pendingStagePaths.value]).toEqual(["src/App.vue"]);

    stageOp.resolve();
    await pending;
    await flushPromises();

    expect(state.unstagedFiles.value).toEqual([]);
    expect(state.stagedFiles.value.map(file => file.path)).toEqual(["src/App.vue"]);
    expect(state.stageOperationBusy.value).toBe(false);
    expect([...state.pendingStagePaths.value]).toEqual([]);
  });

  it("routes stage errors to the git terminal output hook", async () => {
    const terminalOutput = vi.fn();
    const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));
      gitServiceMocks.gitStatus.mockResolvedValue({
        unstaged: [{ path: "src/App.vue", status: "M", lfs: false }],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      });
      gitServiceMocks.gitStage.mockRejectedValue(new Error("git add failed: index.lock exists"));

      const props = reactive({
        workingDir: "",
        isActive: false,
        selectedModelId: "",
        selectedAgentId: "",
        models: [],
      });

      const state = useCollabState(props, {
        onGitTerminalOutput: terminalOutput,
      });

      props.workingDir = "F:/repo";
      await nextTick();
      await flushPromises();

      await state.stageFile("src/App.vue");
      await flushPromises();

      expect(terminalOutput).toHaveBeenCalledWith(
        "git add -- src/App.vue",
        "git add failed: index.lock exists",
        true,
      );
      expect(consoleErrorSpy).not.toHaveBeenCalled();
    } finally {
      consoleErrorSpy.mockRestore();
    }
  });

  it("stages renamed files with both current and old pathspecs", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));
    gitServiceMocks.gitStatus
      .mockResolvedValueOnce({
        unstaged: [{ path: "src/NewName.vue", oldPath: "src/OldName.vue", status: "R", lfs: false }],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      })
      .mockResolvedValueOnce({
        unstaged: [],
        staged: [{ path: "src/NewName.vue", oldPath: "src/OldName.vue", status: "R", lfs: false }],
        blocked: [],
        unmerged: [],
        operation: null,
      });
    gitServiceMocks.gitStagePaths.mockResolvedValue(undefined);

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    await state.stageFile("src/NewName.vue");
    await flushPromises();

    expect(gitServiceMocks.gitStagePaths).toHaveBeenCalledWith([
      "src/NewName.vue",
      "src/OldName.vue",
    ]);
  });

  it("unstages renamed files with both current and old pathspecs", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));
    gitServiceMocks.gitStatus
      .mockResolvedValueOnce({
        unstaged: [],
        staged: [{ path: "src/NewName.vue", oldPath: "src/OldName.vue", status: "R", lfs: false }],
        blocked: [],
        unmerged: [],
        operation: null,
      })
      .mockResolvedValueOnce({
        unstaged: [{ path: "src/NewName.vue", oldPath: "src/OldName.vue", status: "R", lfs: false }],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      });
    gitServiceMocks.gitUnstagePaths.mockResolvedValue(undefined);

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    await state.unstageFile("src/NewName.vue");
    await flushPromises();

    expect(gitServiceMocks.gitUnstagePaths).toHaveBeenCalledWith([
      "src/NewName.vue",
      "src/OldName.vue",
    ]);
  });

  it("resizes the collab git sidebar and persists the width", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }], "aaaaaaa"));

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    const leftAreaEl = {
      getBoundingClientRect: () => ({
        width: 560,
        height: 400,
        top: 0,
        right: 560,
        bottom: 400,
        left: 0,
        x: 0,
        y: 0,
        toJSON: () => "",
      }),
    };
    state.leftAreaRef.value = leftAreaEl as any;
    expect(state.gitSidebarWidth.value).toBe(220);

    state.onSidebarSplitterMouseDown({
      clientX: 220,
      preventDefault() {},
      stopPropagation() {},
    } as MouseEvent);
    documentMock.dispatch("mousemove", { clientX: 280 });

    expect(state.gitSidebarWidth.value).toBe(280);

    documentMock.dispatch("mouseup");

    expect(localStorage.getItem("locus:collabSidebarWidth")).toBe("280");
  });

  it("dedupes overlapping commits when loading more history", async () => {
    gitServiceMocks.gitHistorySnapshot
      .mockResolvedValueOnce(
        snapshot(
          [{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }],
          "aaaaaaa",
          { hasMore: true },
        ),
      )
      .mockResolvedValueOnce(
        snapshot(
          [
            { hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false },
            { hash: "bbbbbbb", shortHash: "bbbbbbb", parents: ["aaaaaaa"], author: "tester", date: 2, message: "repo b", refs: [], isStash: false },
          ],
          "aaaaaaa",
          { hasMore: false },
        ),
      );

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    await state.loadMoreCommits();
    await flushPromises();

    expect(state.commits.value.map(commit => commit.hash)).toEqual(["aaaaaaa", "bbbbbbb"]);
  });

  it("ignores stale load-more results after a newer history refresh", async () => {
    const deferredLoadMore = deferred<any>();

    gitServiceMocks.gitHistorySnapshot
      .mockResolvedValueOnce(
        snapshot(
          [{ hash: "aaaaaaa", shortHash: "aaaaaaa", parents: [], author: "tester", date: 1, message: "repo a", refs: [], isStash: false }],
          "aaaaaaa",
          { hasMore: true },
        ),
      )
      .mockImplementationOnce(() => deferredLoadMore.promise)
      .mockResolvedValueOnce(
        snapshot(
          [{ hash: "ccccccc", shortHash: "ccccccc", parents: [], author: "tester", date: 3, message: "repo c", refs: [], isStash: false }],
          "ccccccc",
          { hasMore: false },
        ),
      );

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    const pendingLoadMore = state.loadMoreCommits();
    await flushPromises();

    state.onRefresh();
    await flushPromises();

    deferredLoadMore.resolve(
      snapshot(
        [{ hash: "bbbbbbb", shortHash: "bbbbbbb", parents: ["aaaaaaa"], author: "tester", date: 2, message: "repo b", refs: [], isStash: false }],
        "aaaaaaa",
        { hasMore: false },
      ),
    );
    await pendingLoadMore;
    await flushPromises();

    expect(state.commits.value.map(commit => commit.hash)).toEqual(["ccccccc"]);
    expect(state.hasMoreCommits.value).toBe(false);
  });

  it("counts blocked platform paths in workspace totals", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([], null));
    gitServiceMocks.gitStatus.mockResolvedValue({
      unstaged: [{ path: "src/App.vue", status: "M", lfs: false }],
      staged: [],
      blocked: [{
        path: "nul",
        status: "?",
        reason: "windowsReservedName",
        segment: "nul",
      }],
      unmerged: [],
      operation: null,
    });

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.blockedFiles.value.map(file => file.path)).toEqual(["nul"]);
    expect(state.totalChanges.value).toBe(2);
    expect(state.workspaceChangeCount.value).toBe(2);
  });

  it("surfaces git status warnings through the shared banner", async () => {
    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([], null));
    gitServiceMocks.gitStatus.mockResolvedValue({
      unstaged: [{ path: "src/App.vue", status: "M", lfs: false }],
      staged: [],
      blocked: [],
      unmerged: [],
      operation: null,
      warnings: [{
        code: "git.index_lock",
        message: "Git index is locked: F:/repo/.git/index.lock",
        detail: "fatal: Unable to create 'F:/repo/.git/index.lock': File exists.",
        operation: "git_status",
        retryable: true,
        severity: "warning",
      }],
    });

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props);

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    expect(state.unstagedFiles.value.map(file => file.path)).toEqual(["src/App.vue"]);
    expect(notificationStoreMock.addNotice).toHaveBeenCalledWith(
      "warning",
      "collab.gitIndexLocked",
      expect.objectContaining({
        code: "git.index_lock",
        operation: "git_status",
        replaceOperation: true,
        ttl: 10_000,
      }),
    );
  });

  it("reports partial stage-all results as warnings instead of failures", async () => {
    const terminalOutput = vi.fn();

    gitServiceMocks.gitHistorySnapshot.mockResolvedValue(snapshot([], null));
    gitServiceMocks.gitStatus
      .mockResolvedValueOnce({
        unstaged: [{ path: "src/App.vue", status: "M", lfs: false }],
        staged: [],
        blocked: [{
          path: "nul",
          status: "?",
          reason: "windowsReservedName",
          segment: "nul",
        }],
        unmerged: [],
        operation: null,
      })
      .mockResolvedValueOnce({
        unstaged: [],
        staged: [{ path: "src/App.vue", status: "M", lfs: false }],
        blocked: [{
          path: "nul",
          status: "?",
          reason: "windowsReservedName",
          segment: "nul",
        }],
        unmerged: [],
        operation: null,
      });
    gitServiceMocks.gitStageAll.mockResolvedValue({
      stagedCount: 1,
      skippedCount: 1,
      blocked: [{
        path: "nul",
        status: "?",
        reason: "windowsReservedName",
        segment: "nul",
      }],
      stdout: "",
      stderr: "",
    });

    const props = reactive({
      workingDir: "",
      isActive: false,
      selectedModelId: "",
      selectedAgentId: "",
      models: [],
    });

    const state = useCollabState(props, {
      onGitTerminalOutput: terminalOutput,
    });

    props.workingDir = "F:/repo";
    await nextTick();
    await flushPromises();

    await state.stageAll();
    await flushPromises();

    expect(terminalOutput).toHaveBeenCalledWith(
      "git add --pathspec-from-file=- --pathspec-file-nul --ignore-errors",
      expect.stringContaining("collab.stageAllPartial"),
      false,
    );
    expect(notificationStoreMock.addNotice).toHaveBeenCalledWith(
      "warning",
      "collab.stageAllPartial",
      expect.objectContaining({ operation: "collabStageAll" }),
    );
    expect(state.stagedFiles.value.map(file => file.path)).toEqual(["src/App.vue"]);
    expect(state.blockedFiles.value.map(file => file.path)).toEqual(["nul"]);
  });
});
