import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import type { TodoItem, TodoSnapshot } from "../types";

const sessionServiceMocks = vi.hoisted(() => ({
  applyKnowledgeProposal: vi.fn(),
  archiveSession: vi.fn(),
  chat: vi.fn(),
  cancelChat: vi.fn(),
  deleteSession: vi.fn(),
  forkSession: vi.fn(),
  forkSessionFromMessage: vi.fn(),
  getActiveSessionSelection: vi.fn(),
  getSessionUsage: vi.fn(),
  getSessionActiveRun: vi.fn(),
  getTodos: vi.fn(),
  ignoreKnowledgeProposal: vi.fn(),
  listSessionEvents: vi.fn(),
  listArchivedSessions: vi.fn(),
  listSessions: vi.fn(),
  loadSession: vi.fn(),
  insertPendingChatInput: vi.fn(),
  queueChatInput: vi.fn(),
  renameSession: vi.fn(),
  saveActiveSessionSelection: vi.fn(),
  staleKnowledgeProposals: vi.fn(),
  rollbackSessionToMessage: vi.fn(),
  undoLatestConversationTurn: vi.fn(),
}));

const undoServiceMocks = vi.hoisted(() => ({
  undoList: vi.fn(),
  undoPerform: vi.fn(),
  undoPerformToMessage: vi.fn(),
}));

const displaySettingsState = vi.hoisted(() => ({
  todoAutoOpen: true,
  changesAutoOpen: true,
  changesAutoClose: true,
  chatDiffReviewTarget: "inline",
  gitDiffReviewTarget: "inline",
  rightAlignUserMessages: false,
  compactToolCalls: true,
  hideThinkingBlocks: true,
  mergeGitTreeStatusIcon: true,
  hideGitCommandSuggestions: false,
}));

const agentStoreMocks = vi.hoisted(() => ({
  resetToDefault: vi.fn(),
  selectedAgentId: "",
  selectAgent: vi.fn(),
}));

const modelStoreMocks = vi.hoisted(() => ({
  availableModels: [{ id: "model-a", name: "Model A", provider: "custom" as const }],
  modelDefaults: {
    mainModel: "model-a",
    planModel: "",
    subagentModels: {},
  },
  selectedModelId: "model-a",
  effort: "none" as const,
  effortSupported: false,
}));

const notificationStoreMocks = vi.hoisted(() => ({
  addNotice: vi.fn(),
}));

const projectStoreState = vi.hoisted(() => ({
  workingDir: "C:\\workspace\\locus",
}));

vi.mock("../services/session", () => sessionServiceMocks);
vi.mock("../services/undo", () => undoServiceMocks);
vi.mock("../composables/useDisplaySettings", () => ({
  useDisplaySettings: () => ({ state: displaySettingsState }),
}));
vi.mock("../stores/agent", () => ({
  useAgentStore: () => agentStoreMocks,
}));
vi.mock("../stores/model", () => ({
  useModelStore: () => modelStoreMocks,
}));
vi.mock("../stores/project", () => ({
  useProjectStore: () => projectStoreState,
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => notificationStoreMocks,
}));
vi.mock("../i18n", () => ({
  t: (_key: string, fallback?: string) => fallback ?? "",
}));

function emptyUsage() {
  return {
    totalInputTokens: 0,
    totalOutputTokens: 0,
    totalCacheReadTokens: 0,
    totalCacheWriteTokens: 0,
    totalCostUsd: 0,
    pricedRounds: 0,
    contextTokens: 0,
    contextLimit: 0,
  };
}

function makeTodo(content = "Do thing"): TodoItem {
  return {
    content,
    status: "pending",
    priority: "medium",
  };
}

function makeUndoEntry(
  sessionId: string,
  path = "src/file.ts",
  options?: {
    assistantMessageId?: string;
    runId?: string | null;
    createdAt?: number;
    status?: string;
    oldPath?: string;
  },
) {
  return {
    id: `undo-${sessionId}-${path}`,
    sessionId,
    assistantMessageId: options?.assistantMessageId ?? `msg-${sessionId}`,
    runId: options?.runId ?? null,
    checkpoint: {
      id: `checkpoint-${sessionId}`,
      label: "round-1",
      createdAt: options?.createdAt ?? 1,
    },
    changedFiles: [{ status: options?.status ?? "M", path, oldPath: options?.oldPath }],
    hasUnityExecute: false,
    consumed: false,
  };
}

describe("chat session panel state", () => {
  let todoData: Record<string, TodoSnapshot>;
  let undoData: Record<string, ReturnType<typeof makeUndoEntry>[]>;
  let latestCompletedRunIdData: Record<string, string | null>;

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();

    displaySettingsState.todoAutoOpen = true;
    displaySettingsState.changesAutoOpen = true;
    displaySettingsState.changesAutoClose = true;
    displaySettingsState.rightAlignUserMessages = false;
    displaySettingsState.compactToolCalls = true;
    displaySettingsState.hideThinkingBlocks = true;
    displaySettingsState.mergeGitTreeStatusIcon = true;
    displaySettingsState.hideGitCommandSuggestions = false;

    todoData = {
      s1: { items: [makeTodo("Todo from history")], latestRunId: "run-history" },
      s2: { items: [], latestRunId: null },
    };
    undoData = {
      s1: [makeUndoEntry("s1")],
      s2: [],
    };
    latestCompletedRunIdData = {
      s1: null,
      s2: null,
    };

    sessionServiceMocks.loadSession.mockImplementation(async (sessionId: string) => ({
      id: sessionId,
      title: `Session ${sessionId}`,
      messages: [],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: latestCompletedRunIdData[sessionId] ?? null,
      createdAt: 0,
      updatedAt: 0,
    }));
    sessionServiceMocks.applyKnowledgeProposal.mockResolvedValue(undefined);
    sessionServiceMocks.archiveSession.mockResolvedValue(undefined);
    sessionServiceMocks.chat.mockResolvedValue({ sessionId: "s1", runId: "run-default" });
    sessionServiceMocks.cancelChat.mockResolvedValue(undefined);
    sessionServiceMocks.deleteSession.mockResolvedValue(undefined);
    sessionServiceMocks.forkSession.mockResolvedValue("s-copy");
    sessionServiceMocks.forkSessionFromMessage.mockResolvedValue("s-copy");
    sessionServiceMocks.getActiveSessionSelection.mockResolvedValue(null);
    sessionServiceMocks.getSessionUsage.mockImplementation(async () => emptyUsage());
    sessionServiceMocks.getSessionActiveRun.mockResolvedValue(null);
    sessionServiceMocks.getTodos.mockImplementation(async (sessionId: string) => (
      todoData[sessionId] ?? { items: [], latestRunId: null }
    ));
    sessionServiceMocks.ignoreKnowledgeProposal.mockResolvedValue(undefined);
    sessionServiceMocks.listSessionEvents.mockResolvedValue([]);
    sessionServiceMocks.listArchivedSessions.mockImplementation(async () => []);
    sessionServiceMocks.listSessions.mockImplementation(async () => []);
    sessionServiceMocks.queueChatInput.mockImplementation(async (params: any) => ({
      id: "pending-1",
      sessionId: params.sessionId,
      runId: params.runId,
      mergeGroupId: params.mergeGroupId,
      status: "queued",
      delivery: params.delivery ?? "after_run",
      text: params.text,
      displayText: params.displayText ?? params.text,
      images: params.images ?? undefined,
      assetRefs: params.assetRefs ?? undefined,
      mode: params.mode ?? null,
      userIntent: params.userIntent ?? null,
      clientMessageId: params.clientMessageId ?? null,
      messageId: null,
      createdAt: 1,
      updatedAt: 1,
    }));
    sessionServiceMocks.insertPendingChatInput.mockImplementation(async (
      sessionId: string,
      runId: string,
      pendingInputId: string | null,
    ) => ({
      id: pendingInputId ?? "pending-1",
      sessionId,
      runId,
      mergeGroupId: "group-1",
      status: "queued",
      delivery: "immediate",
      text: "queued",
      displayText: "queued",
      messageId: null,
      createdAt: 1,
      updatedAt: 2,
    }));
    sessionServiceMocks.renameSession.mockResolvedValue(undefined);
    sessionServiceMocks.rollbackSessionToMessage.mockImplementation(async (sessionId: string) =>
      sessionServiceMocks.loadSession(sessionId),
    );
    sessionServiceMocks.saveActiveSessionSelection.mockResolvedValue(undefined);
    sessionServiceMocks.staleKnowledgeProposals.mockResolvedValue(undefined);
    sessionServiceMocks.undoLatestConversationTurn.mockImplementation(async (sessionId: string) =>
      sessionServiceMocks.loadSession(sessionId),
    );
    undoServiceMocks.undoList.mockImplementation(async (sessionId: string) => undoData[sessionId] ?? []);
    undoServiceMocks.undoPerform.mockResolvedValue(undefined);
    undoServiceMocks.undoPerformToMessage.mockResolvedValue(undefined);
  });

  it("keeps historical todos closed on first session switch and allows manual reopen", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");

    expect(chatStore.todos).toHaveLength(1);
    expect(chatStore.visibleTodos).toHaveLength(0);
    expect(chatStore.todoMode).toBe("current");
    expect(chatStore.showTodoPanel).toBe(false);

    chatStore.toggleTodoPanel();
    expect(chatStore.showTodoPanel).toBe(true);
  });

  it("restores persisted context usage when opening a historical session", async () => {
    const chatStore = useChatStore();
    sessionServiceMocks.getSessionUsage.mockResolvedValueOnce({
      ...emptyUsage(),
      contextTokens: 42000,
      contextLimit: 258400,
    });

    await chatStore.selectSession("s1");

    expect(chatStore.tokenUsage.contextTokens).toBe(42000);
    expect(chatStore.tokenUsage.contextLimit).toBe(258400);
  });

  it("restores the persisted active session after refreshing sessions", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.getActiveSessionSelection.mockResolvedValue("s1");
    sessionServiceMocks.listSessions.mockResolvedValue([
      {
        id: "s1",
        title: "Persisted",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ]);

    await chatStore.refreshSessions();

    expect(chatStore.activeSessionId).toBe("s1");
    expect(sessionServiceMocks.loadSession).toHaveBeenCalledWith("s1");
    expect(sessionServiceMocks.saveActiveSessionSelection).not.toHaveBeenCalled();
  });

  it("applies active session selection broadcasts without echoing persistence", async () => {
    const chatStore = useChatStore();

    chatStore.activeSessionId = "s2";
    await chatStore.syncActiveSessionSelection("s1");

    expect(chatStore.activeSessionId).toBe("s1");
    expect(chatStore.todos).toHaveLength(1);
    expect(sessionServiceMocks.saveActiveSessionSelection).not.toHaveBeenCalled();

    await chatStore.syncActiveSessionSelection(null);

    expect(chatStore.activeSessionId).toBeNull();
    expect(sessionServiceMocks.saveActiveSessionSelection).not.toHaveBeenCalled();
  });

  it("marks active chat runs as streaming when runStart arrives from another window", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");

    expect(chatStore.isStreaming).toBe(false);
    chatStore.handleStreamEvent({
      runId: "run-external",
      type: "runStart",
      sessionId: "s1",
    });

    expect(chatStore.isStreaming).toBe(true);
    expect(chatStore.currentRunId).toBe("run-external");
    expect(chatStore.streamingSessionIds.has("s1")).toBe(true);
  });

  it("remembers todo panel visibility per session", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallDone",
      sessionId: "s1",
      toolCallId: "tc-1",
      toolName: "todowrite",
      output: `Todos updated: ${JSON.stringify(todoData.s1.items)}`,
      outcome: "done",
    });
    expect(chatStore.showTodoPanel).toBe(true);

    await chatStore.selectSession("s2");
    await chatStore.selectSession("s1");
    expect(chatStore.showTodoPanel).toBe(true);

    chatStore.closeTodoPanel();
    await chatStore.selectSession("s2");
    await chatStore.selectSession("s1");
    expect(chatStore.showTodoPanel).toBe(false);
  });

  it("keeps historical file changes closed on first session switch", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    await chatStore.selectSession("s1");

    expect(changesStore.currentFileCount).toBe(1);
    expect(changesStore.currentPanelVisible).toBe(false);
  });

  it("preserves file changes panel state per session and still auto-opens on fresh arrival", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    await chatStore.selectSession("s1");
    changesStore.togglePanel();
    expect(changesStore.currentPanelVisible).toBe(true);

    await chatStore.selectSession("s2");
    await chatStore.selectSession("s1");
    expect(changesStore.currentPanelVisible).toBe(true);

    changesStore.closePanel();
    expect(changesStore.currentPanelVisible).toBe(false);

    await chatStore.selectSession("s2");
    undoData.s2 = [makeUndoEntry("s2", "src/brand-new.ts")];
    await changesStore.refresh("s2");
    expect(changesStore.currentPanelVisible).toBe(true);
  });

  it("auto-opens file changes again when a later round adds new changes", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [];
    await chatStore.selectSession("s1");

    undoData.s1 = [
      makeUndoEntry("s1", "src/first.ts", {
        assistantMessageId: "msg-first",
        runId: "run-1",
        createdAt: 1,
      }),
    ];
    await changesStore.refresh("s1");
    expect(changesStore.currentPanelVisible).toBe(true);

    changesStore.closePanel();
    expect(changesStore.currentPanelVisible).toBe(false);

    undoData.s1 = [
      ...undoData.s1,
      makeUndoEntry("s1", "src/second.ts", {
        assistantMessageId: "msg-second",
        runId: "run-2",
        createdAt: 2,
      }),
    ];
    await changesStore.refresh("s1");

    expect(changesStore.currentPanelVisible).toBe(true);
  });

  it("groups current changes by run id across multiple tool-call rounds", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/alpha.ts", {
        assistantMessageId: "msg-a1",
        runId: "run-a",
        createdAt: 1,
      }),
      makeUndoEntry("s1", "src/beta.ts", {
        assistantMessageId: "msg-a2",
        runId: "run-a",
        createdAt: 2,
      }),
      makeUndoEntry("s1", "src/gamma.ts", {
        assistantMessageId: "msg-b1",
        runId: "run-b",
        createdAt: 3,
      }),
      makeUndoEntry("s1", "src/delta.ts", {
        assistantMessageId: "msg-b2",
        runId: "run-b",
        createdAt: 4,
      }),
    ];

    await chatStore.selectSession("s1");

    expect(changesStore.latestTurnRounds).toHaveLength(2);
    expect(changesStore.latestTurnFiles.map((file) => file.path)).toEqual([
      "src/gamma.ts",
      "src/delta.ts",
    ]);
    expect(changesStore.currentFileCount).toBe(2);
  });

  it("shows an empty current round when the latest completed run has no undo entry", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/changed.ts", {
        assistantMessageId: "msg-change",
        runId: "run-change",
        createdAt: 1,
      }),
    ];

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-noop",
      type: "done",
      sessionId: "s1",
      messageId: "msg-noop",
      fullText: "这一轮没有文件改动",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(changesStore.latestTurnRounds).toEqual([]);
    expect(changesStore.currentFileCount).toBe(0);
    expect(changesStore.currentMode).toBe("current");
    expect(changesStore.hasAnyChanges).toBe(true);

    changesStore.setMode("all");
    expect(changesStore.currentFileCount).toBe(1);
    expect(changesStore.currentFiles.map((file) => "finalPath" in file ? file.finalPath : file.path)).toEqual([
      "src/changed.ts",
    ]);
  });

  it("shows file changes from the cancelled run after interruption", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/old.ts", {
        assistantMessageId: "msg-old",
        runId: "run-old",
        createdAt: 1,
      }),
    ];
    latestCompletedRunIdData.s1 = "run-old";

    await chatStore.selectSession("s1");
    changesStore.closePanel();

    undoData.s1 = [
      ...undoData.s1,
      makeUndoEntry("s1", "src/cancelled.ts", {
        assistantMessageId: "msg-cancelled",
        runId: "run-cancelled",
        createdAt: 2,
      }),
    ];

    chatStore.handleStreamEvent({
      runId: "run-cancelled",
      type: "cancelled",
      sessionId: "s1",
      messageId: "msg-cancelled",
      fullText: "已中断",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(changesStore.latestTurnRounds).toHaveLength(1);
    expect(changesStore.latestTurnFiles.map((file) => file.path)).toEqual([
      "src/cancelled.ts",
    ]);
    expect(changesStore.currentFileCount).toBe(1);
    expect(changesStore.currentPanelVisible).toBe(true);
  });

  it("keeps current mode pinned to the completed run even when the final sub-round has no undo entry", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/alpha.ts", {
        assistantMessageId: "msg-a1",
        runId: "run-a",
        createdAt: 1,
      }),
      makeUndoEntry("s1", "src/beta.ts", {
        assistantMessageId: "msg-a2",
        runId: "run-a",
        createdAt: 2,
      }),
      makeUndoEntry("s1", "src/legacy.ts", {
        assistantMessageId: "msg-legacy",
        runId: "run-legacy",
        createdAt: 0,
      }),
    ];
    latestCompletedRunIdData.s1 = "run-a";

    await chatStore.selectSession("s1");

    expect(changesStore.latestTurnRounds).toHaveLength(2);
    expect(changesStore.latestTurnFiles.map((file) => file.path)).toEqual([
      "src/alpha.ts",
      "src/beta.ts",
    ]);
    expect(changesStore.currentFileCount).toBe(2);
  });

  it("shows file changes from the active run as soon as undo arrives", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/old.ts", {
        assistantMessageId: "msg-old",
        runId: "run-old",
        createdAt: 1,
      }),
    ];
    latestCompletedRunIdData.s1 = "run-old";

    await chatStore.selectSession("s1");
    changesStore.closePanel();

    chatStore.handleStreamEvent({
      runId: "run-new",
      type: "runStart",
      sessionId: "s1",
    });

    expect(changesStore.latestTurnRounds).toEqual([]);
    expect(changesStore.currentFileCount).toBe(0);

    undoData.s1 = [
      ...undoData.s1,
      makeUndoEntry("s1", "src/live.ts", {
        assistantMessageId: "msg-live",
        runId: "run-new",
        createdAt: 2,
      }),
    ];

    chatStore.handleStreamEvent({
      runId: "run-new",
      type: "undoAvailable",
      sessionId: "s1",
      assistantMessageId: "msg-live",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(changesStore.currentPanelVisible).toBe(true);
    expect(changesStore.latestTurnRounds).toHaveLength(1);
    expect(changesStore.latestTurnFiles.map((file) => file.path)).toEqual([
      "src/live.ts",
    ]);
    expect(changesStore.currentFileCount).toBe(1);
  });

  it("clears previous current-round file changes after chat returns a new run id", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [
      makeUndoEntry("s1", "src/old.ts", {
        assistantMessageId: "msg-old",
        runId: "run-old",
        createdAt: 1,
      }),
    ];
    latestCompletedRunIdData.s1 = "run-old";
    sessionServiceMocks.chat.mockResolvedValueOnce({ sessionId: "s1", runId: "run-new" });

    await chatStore.selectSession("s1");
    expect(changesStore.currentFileCount).toBe(1);

    await chatStore.sendMessage("next");

    expect(changesStore.latestTurnRounds).toEqual([]);
    expect(changesStore.currentFileCount).toBe(0);
    expect(changesStore.hasAnyChanges).toBe(true);
  });

  it("tracks background-session undo arrivals and opens the panel when that session is revisited", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    undoData.s1 = [];
    undoData.s2 = [];

    await chatStore.selectSession("s1");

    undoData.s2 = [
      makeUndoEntry("s2", "src/background.ts", {
        assistantMessageId: "msg-bg",
        runId: "run-bg",
        createdAt: 2,
      }),
    ];

    chatStore.handleStreamEvent({
      runId: "run-bg",
      type: "undoAvailable",
      sessionId: "s2",
      assistantMessageId: "msg-bg",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    await chatStore.selectSession("s2");
    expect(changesStore.currentPanelVisible).toBe(true);
    expect(changesStore.currentFileCount).toBe(1);
  });

  it("tracks background session streaming state for non-dev sessions", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.listSessions
      .mockResolvedValueOnce([
        {
          id: "git-1",
          title: "git status",
          agentId: "git",
          sessionType: "git",
          updatedAt: 1,
          runtimeStatus: "running",
        },
      ])
      .mockResolvedValueOnce([
        {
          id: "git-1",
          title: "git status",
          agentId: "git",
          sessionType: "git",
          updatedAt: 2,
          runtimeStatus: null,
        },
      ]);

    chatStore.handleStreamEvent({
      runId: "run-bg",
      type: "runStart",
      sessionId: "git-1",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(chatStore.streamingSessionIds.has("git-1")).toBe(true);
    expect(sessionServiceMocks.listSessions).toHaveBeenCalledTimes(1);

    chatStore.handleStreamEvent({
      runId: "run-bg",
      type: "done",
      sessionId: "git-1",
      messageId: "msg-git-1",
      fullText: "done",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(chatStore.streamingSessionIds.has("git-1")).toBe(false);
    expect(sessionServiceMocks.listSessions).toHaveBeenCalledTimes(2);
  });

  it("reconciles stale streaming session ids against backend runtime status", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Recovered session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);

    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s1",
        title: "Recovered session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 2,
        runtimeStatus: null,
      },
    ]);

    await chatStore.refreshSessions();

    expect(chatStore.isStreaming).toBe(false);
    expect(chatStore.streamingSessionIds.has("s1")).toBe(false);
  });

  it("hydrates active backend runs and replays current stream events on refresh", async () => {
    const chatStore = useChatStore();

    chatStore.activeSessionId = "s1";
    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s1",
        title: "Running session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 2,
        runtimeStatus: "running",
      },
    ]);
    sessionServiceMocks.getSessionActiveRun.mockResolvedValueOnce({
      runId: "run-1",
      sessionId: "s1",
      status: "running",
      startedAt: 1,
      updatedAt: 2,
      finishedAt: null,
      errorMessage: null,
    });
    sessionServiceMocks.listSessionEvents.mockResolvedValueOnce([
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 1,
        eventType: "runStart",
        payload: { type: "runStart", sessionId: "s1" },
        createdAt: 1,
      },
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 2,
        eventType: "toolCallStart",
        payload: {
          type: "toolCallStart",
          sessionId: "s1",
          toolCallId: "tc-active",
          toolName: "read",
          arguments: "{}",
        },
        createdAt: 2,
      },
    ]);

    await chatStore.refreshSessions();

    expect(chatStore.streamingSessionIds.has("s1")).toBe(true);
    expect(chatStore.currentRunId).toBe("run-1");
    expect(chatStore.isStreaming).toBe(true);
    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls[0]?.id).toBe("tc-active");
  });

  it("replays the active run tool chain after loading persisted messages", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.loadSession.mockResolvedValueOnce({
      id: "s1",
      title: "Session s1",
      messages: [
        {
          id: "msg-round",
          role: "assistant",
          content: "persisted round",
          createdAt: 1,
          toolCalls: [{ id: "tc-old", name: "read", arguments: "{}" }],
        },
      ],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: null,
      createdAt: 0,
      updatedAt: 0,
    });
    sessionServiceMocks.getSessionActiveRun.mockResolvedValueOnce({
      runId: "run-1",
      sessionId: "s1",
      status: "running",
      startedAt: 1,
      updatedAt: 2,
      finishedAt: null,
      errorMessage: null,
    });
    sessionServiceMocks.listSessionEvents.mockResolvedValueOnce([
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 1,
        eventType: "toolCallStart",
        payload: {
          type: "toolCallStart",
          sessionId: "s1",
          toolCallId: "tc-old",
          toolName: "read",
          arguments: "{}",
        },
        createdAt: 1,
      },
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 2,
        eventType: "toolCallDone",
        payload: {
          type: "toolCallDone",
          sessionId: "s1",
          toolCallId: "tc-old",
          toolName: "read",
          output: "old output",
          outcome: "done",
        },
        createdAt: 2,
      },
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 3,
        eventType: "toolCallRoundDone",
        payload: {
          type: "toolCallRoundDone",
          sessionId: "s1",
          messageId: "msg-round",
          fullText: "persisted round",
          toolCalls: [{ id: "tc-old", name: "read", arguments: "{}" }],
        },
        createdAt: 3,
      },
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 4,
        eventType: "toolCallStart",
        payload: {
          type: "toolCallStart",
          sessionId: "s1",
          toolCallId: "tc-current",
          toolName: "grep",
          arguments: "{}",
        },
        createdAt: 4,
      },
    ]);

    await chatStore.selectSession("s1");

    const assistantMessages = chatStore.messages.filter((message) => message.role === "assistant");
    expect(assistantMessages).toHaveLength(1);
    expect(assistantMessages[0]?.id).toBe("msg-round");
    expect(assistantMessages[0]?.renderParts?.some((part) => part.kind === "toolCall" && part.id === "tc-old")).toBe(true);
    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls.map((toolCall) => toolCall.id)).toEqual(["tc-current"]);
    expect(chatStore.activeToolCalls[0]?.status).toBe("running");
  });

  it("replaces a loaded assistant tool round when the live round-done event arrives", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.loadSession.mockResolvedValueOnce({
      id: "s1",
      title: "Session s1",
      messages: [
        {
          id: "msg-round",
          role: "assistant",
          content: "",
          createdAt: 1,
          toolCalls: [{ id: "tc-run", name: "unity_run_states", arguments: "{}" }],
        },
      ],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: null,
      createdAt: 0,
      updatedAt: 0,
    });
    sessionServiceMocks.getSessionActiveRun.mockResolvedValueOnce({
      runId: "run-1",
      sessionId: "s1",
      status: "running",
      startedAt: 1,
      updatedAt: 2,
      finishedAt: null,
      errorMessage: null,
    });
    sessionServiceMocks.listSessionEvents.mockResolvedValueOnce([
      {
        sessionId: "s1",
        runId: "run-1",
        seq: 1,
        eventType: "toolCallStart",
        payload: {
          type: "toolCallStart",
          sessionId: "s1",
          toolCallId: "tc-run",
          toolName: "unity_run_states",
          arguments: "{}",
        },
        createdAt: 1,
      },
    ]);

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallDone",
      sessionId: "s1",
      toolCallId: "tc-run",
      toolName: "unity_run_states",
      output: "status: ok",
      outcome: "done",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallRoundDone",
      sessionId: "s1",
      messageId: "msg-round",
      fullText: "profile done",
      toolCalls: [
        {
          id: "tc-run",
          name: "unity_run_states",
          arguments: "{}",
          outcome: "done",
        },
      ],
    });

    const assistantRounds = chatStore.messages.filter((message) => message.id === "msg-round");
    expect(assistantRounds).toHaveLength(1);
    expect(assistantRounds[0]?.content).toBe("profile done");
    expect(assistantRounds[0]?.toolCalls?.[0]?.outcome).toBe("done");
    expect(chatStore.messages.filter((message) => message.role === "tool")).toHaveLength(1);
  });

  it("suppresses stale backend running status after a terminal event", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Finished session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
        runtimeStatus: "running",
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);

    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s1",
        title: "Finished session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 2,
        runtimeStatus: "running",
      },
    ]);

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "done",
      sessionId: "s1",
      messageId: "msg-1",
      fullText: "done",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(chatStore.isStreaming).toBe(false);
    expect(chatStore.streamingSessionIds.has("s1")).toBe(false);
    expect(chatStore.sessions[0]?.runtimeStatus).toBeNull();
  });

  it("keeps locally tracked subagent runs streaming when backend runtime status is empty", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "root-1",
        title: "Parent run",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
      {
        id: "child-1",
        title: "sub:inspect",
        agentId: "explorer",
        sessionType: "chat",
        parentSessionId: "root-1",
        updatedAt: 2,
      },
    ] as any;
    chatStore.streamingSessionIds = new Set(["child-1"]);
    chatStore.handleStreamEvent({
      runId: "child-run-1",
      type: "runStart",
      sessionId: "child-1",
    });

    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "root-1",
        title: "Parent run",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
        runtimeStatus: "running",
      },
      {
        id: "child-1",
        title: "sub:inspect",
        agentId: "explorer",
        sessionType: "chat",
        parentSessionId: "root-1",
        updatedAt: 2,
        runtimeStatus: null,
      },
    ]);

    await chatStore.refreshSessions();

    expect(chatStore.streamingSessionIds.has("child-1")).toBe(true);
  });

  it("treats cancelled stream events as silent stop signals", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Cancelled run",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);

    await chatStore.cancelChat();
    notificationStoreMocks.addNotice.mockClear();

    chatStore.handleStreamEvent({
      runId: "cancel-1",
      type: "cancelled",
      sessionId: "s1",
    });

    expect(sessionServiceMocks.cancelChat).toHaveBeenCalledWith("s1");
    expect(chatStore.isStreaming).toBe(false);
    expect(chatStore.streamingSessionIds.has("s1")).toBe(false);
    expect(notificationStoreMocks.addNotice).not.toHaveBeenCalled();
  });

  it("clears pending question cards when a cancelled event closes the run", () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Cancelled question",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "cancel-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);
    chatStore.pendingQuestion = {
      questionId: "q1",
      toolCallId: "tc-ask",
      question: "Which shape?",
      options: [{ label: "Rect", description: "rect" }],
    } as any;
    chatStore.pendingToolConfirms = [
      {
        questionId: "q2",
        toolCallId: "tc-write",
        display: {
          kind: "basic",
          toolName: "write",
          arguments: "{\"path\":\"foo.ts\"}",
        },
      },
    ] as any;

    chatStore.handleStreamEvent({
      runId: "cancel-1",
      type: "cancelled",
      sessionId: "s1",
    });

    expect(chatStore.pendingQuestion).toBeNull();
    expect(chatStore.pendingToolConfirms).toEqual([]);
  });

  it("clears answered pending input cards from replayed stream events", () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Reloading Unity",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);
    chatStore.pendingQuestion = {
      questionId: "q1",
      toolCallId: "tc-recompile",
      question: "Exit play mode?",
      options: [{ label: "Confirm", description: "Exit play mode and recompile" }],
    } as any;
    chatStore.pendingToolConfirms = [
      {
        questionId: "q2",
        toolCallId: "tc-write",
        display: {
          kind: "basic",
          toolName: "write",
          arguments: "{}",
        },
      },
    ] as any;

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "inputAnswered",
      sessionId: "s1",
      questionId: "q1",
    });

    expect(chatStore.pendingQuestion).toBeNull();
    expect(chatStore.pendingToolConfirms).toHaveLength(1);

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "inputAnswered",
      sessionId: "s1",
      questionId: "q2",
    });

    expect(chatStore.pendingToolConfirms).toEqual([]);
  });

  it("clears pending question cards after cancelChat resolves", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Question in progress",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);
    chatStore.pendingQuestion = {
      questionId: "q1",
      toolCallId: "tc-ask",
      question: "Which shape?",
      options: [{ label: "Rect", description: "rect" }],
    } as any;
    chatStore.pendingToolConfirms = [
      {
        questionId: "q2",
        toolCallId: "tc-write",
        display: {
          kind: "basic",
          toolName: "write",
          arguments: "{\"path\":\"foo.ts\"}",
        },
      },
    ] as any;

    await chatStore.cancelChat();

    expect(chatStore.pendingQuestion).toBeNull();
    expect(chatStore.pendingToolConfirms).toEqual([]);
  });

  it("does not clear the current round immediately when cancellation is requested", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "In-flight round",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds = new Set(["s1"]);
    chatStore.activeToolCalls = [
      { id: "tc-1", name: "read", arguments: "{}", status: "running" },
    ];

    await chatStore.cancelChat();

    expect(chatStore.isStreaming).toBe(true);
    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls[0]?.id).toBe("tc-1");
  });

  it("keeps processing the active run while cancellation is pending", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Cancelled before start",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    sessionServiceMocks.chat.mockResolvedValueOnce({ sessionId: "s1", runId: "run-1" });
    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s1",
        title: "Cancelled before start",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
        runtimeStatus: "running",
      },
    ]);

    await chatStore.sendMessage("first");
    await chatStore.cancelChat();

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "runStart",
      sessionId: "s1",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallStart",
      sessionId: "s1",
      toolCallId: "tc-late",
      toolName: "read",
      arguments: "{}",
    });

    expect(chatStore.isStreaming).toBe(true);
    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls[0]?.status).toBe("running");
  });

  it("shows an empty current task round when the latest completed run has no todowrite", async () => {
    const chatStore = useChatStore();

    todoData.s1 = {
      items: [makeTodo("历史任务")],
      latestRunId: "run-change",
    };
    latestCompletedRunIdData.s1 = "run-noop";

    await chatStore.selectSession("s1");

    expect(chatStore.hasAnyTodos).toBe(true);
    expect(chatStore.visibleTodos).toEqual([]);
    expect(chatStore.visibleTodoCount).toBe(0);
    expect(chatStore.todoCelebrationEnabled).toBe(false);
    expect(chatStore.todoCelebrationVersion).toBe(0);
    chatStore.setTodoMode("all");
    expect(chatStore.todoMode).toBe("current");
    expect(chatStore.visibleTodos).toEqual([]);
  });

  it("clears current-round tasks during a new noop run while keeping the task panel pinned to current", async () => {
    const chatStore = useChatStore();

    todoData.s1 = {
      items: [makeTodo("已完成的旧任务"), { ...makeTodo("另一个旧任务"), status: "completed" }],
      latestRunId: "run-old",
    };
    latestCompletedRunIdData.s1 = "run-old";

    await chatStore.selectSession("s1");

    expect(chatStore.visibleTodos).toHaveLength(2);
    expect(chatStore.todoCelebrationEnabled).toBe(true);

    chatStore.isStreaming = true;
    chatStore.handleStreamEvent({
      runId: "run-next",
      type: "runStart",
      sessionId: "s1",
    });

    expect(chatStore.visibleTodos).toEqual([]);
    expect(chatStore.todoCelebrationEnabled).toBe(false);
    expect(chatStore.todoCelebrationVersion).toBe(0);

    chatStore.setTodoMode("all");
    expect(chatStore.todoMode).toBe("current");
    expect(chatStore.visibleTodos).toEqual([]);

    chatStore.setTodoMode("current");
    chatStore.handleStreamEvent({
      runId: "run-next",
      type: "done",
      sessionId: "s1",
      messageId: "msg-noop",
      fullText: "这一轮没有任务",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(chatStore.visibleTodos).toEqual([]);
    expect(chatStore.todoCelebrationEnabled).toBe(false);
    expect(chatStore.todoCelebrationVersion).toBe(0);
  });

  it("cancels every requested streaming session during a workspace-scoped stop", async () => {
    const chatStore = useChatStore();

    chatStore.streamingSessionIds = new Set(["s1", "s2"]);
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.sessions = [
      {
        id: "s1",
        title: "Main session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
      {
        id: "s2",
        title: "Background session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 2,
      },
    ] as any;
    chatStore.pendingQuestion = {
      questionId: "q-1",
      prompt: "Continue?",
      options: [],
      createdAt: 1,
    } as any;
    chatStore.pendingToolConfirms = [
      {
        questionId: "tc-1",
        toolName: "read",
        argsText: "{}",
      } as any,
    ];

    await chatStore.cancelSessions(["s1", "s2"]);

    expect(sessionServiceMocks.cancelChat).toHaveBeenNthCalledWith(1, "s1");
    expect(sessionServiceMocks.cancelChat).toHaveBeenNthCalledWith(2, "s2");
    expect(chatStore.pendingQuestion).toBeNull();
    expect(chatStore.pendingToolConfirms).toEqual([]);
  });

  it("stops streaming cleanly when a server-tool-only round ends with a terminal done event", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Web search session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.isStreaming = true;

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "runStart",
      sessionId: "s1",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallStart",
      sessionId: "s1",
      toolCallId: "ws-1",
      toolName: "web_search",
      arguments: "{\"query\":\"unity camera preview\"}",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallDone",
      sessionId: "s1",
      toolCallId: "ws-1",
      toolName: "web_search",
      output: "Searched: unity camera preview",
      outcome: "done",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallRoundDone",
      sessionId: "s1",
      messageId: "msg-1",
      fullText: "可以直接在 Scene 里预览。",
      toolCalls: [
        {
          id: "ws-1",
          name: "web_search",
          arguments: "{\"query\":\"unity camera preview\"}",
          serverToolOutput: "Searched: unity camera preview",
        },
      ],
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "done",
      sessionId: "s1",
      messageId: "msg-1",
      fullText: "可以直接在 Scene 里预览。",
    });

    expect(chatStore.isStreaming).toBe(false);
    expect(chatStore.currentRunId).toBeNull();
    expect(chatStore.streamingSessionIds.has("s1")).toBe(false);
    expect(chatStore.activeToolCalls).toEqual([]);
    expect(chatStore.messages.filter((message) => message.role === "assistant")).toHaveLength(1);
    expect(chatStore.messages.find((message) => message.id === "msg-1")?.content)
      .toBe("可以直接在 Scene 里预览。");
    expect(chatStore.messages.find((message) => message.id === "msg-1")?.toolCalls).toEqual([
      {
        id: "ws-1",
        name: "web_search",
        arguments: "{\"query\":\"unity camera preview\"}",
        serverToolOutput: "Searched: unity camera preview",
      },
    ]);
  });

  it("ignores terminal events from an older cancelled run after a new run starts", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Retry session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    sessionServiceMocks.chat
      .mockResolvedValueOnce({ sessionId: "s1", runId: "run-1" })
      .mockResolvedValueOnce({ sessionId: "s1", runId: "run-2" });
    sessionServiceMocks.listSessions
      .mockResolvedValueOnce([
        {
          id: "s1",
          title: "Retry session",
          agentId: null,
          sessionType: "chat",
          updatedAt: 1,
          runtimeStatus: "running",
        },
      ])
      .mockResolvedValueOnce([
        {
          id: "s1",
          title: "Retry session",
          agentId: null,
          sessionType: "chat",
          updatedAt: 2,
          runtimeStatus: "running",
        },
      ])
      .mockResolvedValue([
        {
          id: "s1",
          title: "Retry session",
          agentId: null,
          sessionType: "chat",
          updatedAt: 2,
          runtimeStatus: "running",
        },
      ]);

    await chatStore.sendMessage("first");
    await chatStore.cancelChat();
    await chatStore.sendMessage("second");

    expect(sessionServiceMocks.queueChatInput).toHaveBeenCalledTimes(1);
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "cancelled",
      sessionId: "s1",
    });
    await new Promise((resolve) => globalThis.setTimeout(resolve, 0));

    chatStore.handleStreamEvent({
      runId: "run-2",
      type: "toolCallStart",
      sessionId: "s1",
      toolCallId: "tc-new",
      toolName: "read",
      arguments: "{}",
    });

    expect(chatStore.isStreaming).toBe(true);
    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls[0].id).toBe("tc-new");
  });

  it("assigns unique ids to local pending user messages", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.listSessions.mockResolvedValue([
      {
        id: "s1",
        title: "Pending ids",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
        runtimeStatus: "running",
      },
    ]);

    await chatStore.sendMessage("first");
    chatStore.isStreaming = false;
    await chatStore.sendMessage("second");

    expect(chatStore.messages).toHaveLength(2);
    expect(chatStore.messages[0].id).not.toBe(chatStore.messages[1].id);
  });

  it("removes the local pending user message when chat launch fails", async () => {
    const chatStore = useChatStore();
    sessionServiceMocks.chat.mockRejectedValueOnce(new Error("session locked"));

    await chatStore.sendMessage("will fail");

    expect(chatStore.isStreaming).toBe(false);
    expect(chatStore.messages).toEqual([]);
  });

  it("sends a client message id that matches the local pending user message", async () => {
    const chatStore = useChatStore();

    await chatStore.sendMessage("hello");

    const userMessage = chatStore.messages.find((message) => message.role === "user");
    const chatParams = sessionServiceMocks.chat.mock.calls[sessionServiceMocks.chat.mock.calls.length - 1]?.[0];
    expect(userMessage?.id).toMatch(/^user_pending_/);
    expect(chatParams?.userIntent?.clientMessageId).toBe(userMessage?.id);
    expect(userMessage?.thinkingSignature).toContain(userMessage?.id ?? "");
  });

  it("queues and merges follow-up messages while a run is active", async () => {
    const chatStore = useChatStore();
    const pendingByGroup = new Map<string, any>();
    sessionServiceMocks.queueChatInput.mockImplementation(async (params: any) => {
      const existing = pendingByGroup.get(params.mergeGroupId);
      const pending = existing
        ? {
          ...existing,
          text: `${existing.text}\n${params.text}`,
          displayText: `${existing.displayText}\n${params.displayText ?? params.text}`,
          updatedAt: existing.updatedAt + 1,
        }
        : {
          id: "pending-running",
          sessionId: params.sessionId,
          runId: params.runId,
          mergeGroupId: params.mergeGroupId,
          status: "queued",
          delivery: params.delivery ?? "after_run",
          text: params.text,
          displayText: params.displayText ?? params.text,
          images: params.images ?? undefined,
          assetRefs: params.assetRefs ?? undefined,
          mode: params.mode ?? null,
          userIntent: params.userIntent ?? null,
          clientMessageId: params.clientMessageId ?? null,
          messageId: null,
          createdAt: 1,
          updatedAt: 1,
        };
      pendingByGroup.set(params.mergeGroupId, pending);
      return pending;
    });

    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;

    await chatStore.sendMessage("first");
    await chatStore.sendMessage("second");

    expect(sessionServiceMocks.chat).not.toHaveBeenCalled();
    expect(sessionServiceMocks.queueChatInput).toHaveBeenCalledTimes(2);
    const firstCall = sessionServiceMocks.queueChatInput.mock.calls[0]![0];
    const secondCall = sessionServiceMocks.queueChatInput.mock.calls[1]![0];
    expect(firstCall.delivery).toBe("after_run");
    expect(secondCall.mergeGroupId).toBe(firstCall.mergeGroupId);
    expect(chatStore.activeQueuedFollowUp?.displayText).toBe("first\nsecond");

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "pendingInputAccepted",
      sessionId: "s1",
      pendingInputId: "pending-running",
      messageId: "msg-accepted",
    });

    expect(chatStore.activeQueuedFollowUp).toBeNull();
  });

  it("shows an inserted user message after the active tool round is persisted", () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Running session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.messages = [
      {
        id: "user-1",
        role: "user",
        content: "first",
        createdAt: 1,
      },
    ] as any;

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallStart",
      sessionId: "s1",
      toolCallId: "tc-1",
      toolName: "read",
      arguments: "{}",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "userMessage",
      sessionId: "s1",
      message: {
        id: "user-follow-up",
        role: "user",
        content: "next",
        createdAt: 2,
      },
    });

    expect(chatStore.messages.map((message) => message.id)).toEqual(["user-1"]);

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "pendingInputAccepted",
      sessionId: "s1",
      pendingInputId: "pending-follow-up",
      messageId: "user-follow-up",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallDone",
      sessionId: "s1",
      toolCallId: "tc-1",
      toolName: "read",
      output: "ok",
      outcome: "done",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallRoundDone",
      sessionId: "s1",
      messageId: "assistant-round",
      fullText: "finished previous round",
      toolCalls: [{ id: "tc-1", name: "read", arguments: "{}", outcome: "done" }],
    });

    expect(chatStore.messages.filter((message) => message.role !== "tool").map((message) => message.id)).toEqual([
      "user-1",
      "assistant-round",
      "user-follow-up",
    ]);
  });

  it("does not reinsert a queued follow-up after it was accepted", async () => {
    const chatStore = useChatStore();
    let resolveQueue: ((value: any) => void) | null = null;
    sessionServiceMocks.queueChatInput.mockImplementationOnce((params: any) => new Promise((resolve) => {
      resolveQueue = resolve;
      void params;
    }));

    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;

    const sendPromise = chatStore.sendMessage("race");
    await vi.waitFor(() => {
      expect(sessionServiceMocks.queueChatInput).toHaveBeenCalledTimes(1);
    });
    const params = sessionServiceMocks.queueChatInput.mock.calls[0]![0];

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "pendingInputAccepted",
      sessionId: "s1",
      pendingInputId: "pending-race",
      messageId: "msg-accepted",
    });
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "pendingInputQueued",
      sessionId: "s1",
      input: {
        id: "pending-race",
        sessionId: "s1",
        runId: "run-1",
        mergeGroupId: params.mergeGroupId,
        status: "queued",
        delivery: "after_run",
        text: "race",
        displayText: "race",
        messageId: null,
        createdAt: 1,
        updatedAt: 1,
      },
    });

    expect(resolveQueue).toBeTruthy();
    const resolveQueuedInput = resolveQueue as unknown as (value: any) => void;
    resolveQueuedInput({
      id: "pending-race",
      sessionId: "s1",
      runId: "run-1",
      mergeGroupId: params.mergeGroupId,
      status: "queued",
      delivery: "after_run",
      text: "race",
      displayText: "race",
      messageId: null,
      createdAt: 1,
      updatedAt: 1,
    });
    await sendPromise;

    expect(chatStore.activeQueuedFollowUp).toBeNull();
  });

  it("keeps a closing-run follow-up and sends it after the terminal event", async () => {
    const chatStore = useChatStore();
    sessionServiceMocks.queueChatInput.mockRejectedValueOnce({
      code: "session.pending_input.run_closed",
      message: "run is closing",
    });

    chatStore.sessions = [
      {
        id: "s1",
        title: "Closing run",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    chatStore.currentRunId = "run-1";
    chatStore.isStreaming = true;
    chatStore.streamingSessionIds.add("s1");

    await chatStore.sendMessage("next turn");

    expect(sessionServiceMocks.queueChatInput).toHaveBeenCalledTimes(1);
    expect(sessionServiceMocks.chat).not.toHaveBeenCalled();
    expect(chatStore.activeQueuedFollowUp?.displayText).toBe("next turn");

    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "done",
      sessionId: "s1",
      messageId: "msg-1",
      fullText: "done",
    });
    await new Promise((resolve) => globalThis.setTimeout(resolve, 25));
    await Promise.resolve();

    expect(chatStore.activeQueuedFollowUp).toBeNull();
    expect(sessionServiceMocks.chat).toHaveBeenCalledTimes(1);
    expect(sessionServiceMocks.chat.mock.calls[0]?.[0]).toMatchObject({
      sessionId: "s1",
      text: "next turn",
    });
    expect(notificationStoreMocks.addNotice).not.toHaveBeenCalled();
  });

  it("marks pending knowledge proposals stale when the user continues chatting", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    chatStore.messages = [
      {
        id: "kp-msg",
        role: "assistant",
        content: "",
        createdAt: 1,
        knowledgeProposal: {
          proposalId: "kp-1",
          status: "pending",
          confidence: 0.82,
          verify: "required",
          estTokens: 1200,
          items: [
            {
              kind: "memory",
              mode: "replace",
              target: "project-understanding.md",
              draft: "# Project Understanding",
            },
          ],
          createdAt: 1,
          updatedAt: 1,
        },
      } as any,
    ];

    await chatStore.sendMessage("continue");

    expect(chatStore.messages.find((message) => message.id === "kp-msg")?.knowledgeProposal?.status)
      .toBe("stale");
    expect(sessionServiceMocks.staleKnowledgeProposals).toHaveBeenCalledWith("s1");
  });

  it("accepts knowledge proposal stream events even when the active run id differs", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "runStart",
      sessionId: "s1",
    });
    chatStore.handleStreamEvent({
      runId: "knowledge_1",
      type: "knowledgeProposal",
      sessionId: "s1",
      message: {
        id: "kp-msg",
        role: "assistant",
        content: "",
        createdAt: 1,
        knowledgeProposal: {
          proposalId: "kp-1",
          status: "pending",
          confidence: 0.82,
          verify: "required",
          estTokens: 1200,
          items: [],
          createdAt: 1,
          updatedAt: 1,
        },
      } as any,
    });

    expect(chatStore.messages.find((message) => message.id === "kp-msg")?.knowledgeProposal?.proposalId)
      .toBe("kp-1");
  });

  it("marks the proposal applying when the user applies it", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    chatStore.messages = [
      {
        id: "kp-msg",
        role: "assistant",
        content: "",
        createdAt: 1,
        knowledgeProposal: {
          proposalId: "kp-1",
          status: "pending",
          confidence: 0.82,
          verify: "required",
          estTokens: 1200,
          items: [],
          createdAt: 1,
          updatedAt: 1,
        },
      } as any,
    ];

    await chatStore.applyKnowledgeProposal("kp-1");

    expect(sessionServiceMocks.applyKnowledgeProposal).toHaveBeenCalledWith(
      "s1",
      "kp-1",
    );
    expect(chatStore.messages.find((message) => message.id === "kp-msg")?.knowledgeProposal?.status)
      .toBe("applying");
  });

  it("clears workspace-scoped chat state during onboarding reset", async () => {
    const chatStore = useChatStore();
    const changesStore = useChatChangesStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Old session",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ];
    await chatStore.selectSession("s1");
    chatStore.messages = [
      {
        id: "m1",
        role: "user",
        content: "stale message",
        createdAt: 1,
      } as any,
    ];
    changesStore.openInlineDiff({} as any, "msg-s1");

    chatStore.resetWorkspaceScope();

    expect(chatStore.activeSessionId).toBeNull();
    expect(chatStore.sessions).toEqual([]);
    expect(chatStore.messages).toEqual([]);
    expect(changesStore.inlineDiffPayload).toBeNull();
    expect(agentStoreMocks.resetToDefault).toHaveBeenCalled();
  });

  it("restores the correct run id when switching back to a streaming session", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "runStart",
      sessionId: "s1",
    });

    await chatStore.selectSession("s2");
    chatStore.handleStreamEvent({
      runId: "run-2",
      type: "runStart",
      sessionId: "s2",
    });

    await chatStore.selectSession("s1");
    chatStore.handleStreamEvent({
      runId: "run-1",
      type: "toolCallStart",
      sessionId: "s1",
      toolCallId: "tc-resume",
      toolName: "read",
      arguments: "{\"path\":\"resume.txt\"}",
    });

    expect(chatStore.activeToolCalls).toHaveLength(1);
    expect(chatStore.activeToolCalls[0].id).toBe("tc-resume");
  });

  it("forks the active root session and switches to the copy", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Root session",
        agentId: null,
        sessionType: "chat",
        parentSessionId: null,
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    sessionServiceMocks.forkSession.mockResolvedValueOnce("s-copy");
    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s-copy",
        title: "Root session copy",
        agentId: null,
        sessionType: "chat",
        parentSessionId: null,
        updatedAt: 2,
      },
      {
        id: "s1",
        title: "Root session",
        agentId: null,
        sessionType: "chat",
        parentSessionId: null,
        updatedAt: 1,
      },
    ]);

    await chatStore.forkSession();

    expect(sessionServiceMocks.forkSession).toHaveBeenCalledWith("s1", "Root session");
    expect(chatStore.activeSessionId).toBe("s-copy");
    expect(sessionServiceMocks.loadSession).toHaveBeenCalledWith("s-copy");
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "success",
      "",
      { operation: "forkSession" },
    );
  });

  it("forks from a specific message boundary", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Root session",
        agentId: null,
        sessionType: "chat",
        parentSessionId: null,
        updatedAt: 1,
      },
    ] as any;
    chatStore.activeSessionId = "s1";
    sessionServiceMocks.forkSessionFromMessage.mockResolvedValueOnce("s-copy");
    sessionServiceMocks.listSessions.mockResolvedValueOnce([
      {
        id: "s-copy",
        title: "Root session copy",
        agentId: null,
        sessionType: "chat",
        parentSessionId: null,
        updatedAt: 2,
      },
    ]);

    await chatStore.forkSessionFromMessage("user-2");

    expect(sessionServiceMocks.forkSessionFromMessage).toHaveBeenCalledWith(
      "s1",
      "user-2",
      "Root session",
    );
    expect(chatStore.activeSessionId).toBe("s-copy");
  });

  it("blocks child session forks before calling the backend", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "child-1",
        title: "Child session",
        agentId: "explorer",
        sessionType: "chat",
        parentSessionId: "root-1",
        updatedAt: 2,
      },
    ] as any;
    chatStore.activeSessionId = "child-1";

    await chatStore.forkSession();

    expect(sessionServiceMocks.forkSession).not.toHaveBeenCalled();
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "warning",
      "",
      { code: "session.fork_child", operation: "forkSession" },
    );
  });

  it("archives the active session, clears local state, and notifies the user", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Archive me",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;

    await chatStore.selectSession("s1");
    chatStore.messages = [
      {
        id: "m1",
        role: "user",
        content: "hello",
        createdAt: 1,
      } as any,
    ];

    await chatStore.archiveSession("s1");

    expect(sessionServiceMocks.archiveSession).toHaveBeenCalledWith("s1");
    expect(sessionServiceMocks.listSessions).toHaveBeenCalled();
    expect(chatStore.activeSessionId).toBeNull();
    expect(chatStore.messages).toEqual([]);
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "success",
      "",
      { operation: "archiveSession" },
    );
  });

  it("deletes the active session, clears local state, and notifies the user", async () => {
    const chatStore = useChatStore();

    chatStore.sessions = [
      {
        id: "s1",
        title: "Delete me",
        agentId: null,
        sessionType: "chat",
        updatedAt: 1,
      },
    ] as any;

    await chatStore.selectSession("s1");
    chatStore.messages = [
      {
        id: "m1",
        role: "user",
        content: "hello",
        createdAt: 1,
      } as any,
    ];

    await chatStore.deleteSession("s1");

    expect(sessionServiceMocks.deleteSession).toHaveBeenCalledWith("s1");
    expect(sessionServiceMocks.listSessions).toHaveBeenCalled();
    expect(chatStore.activeSessionId).toBeNull();
    expect(chatStore.messages).toEqual([]);
    expect(notificationStoreMocks.addNotice).toHaveBeenCalledWith(
      "success",
      "",
      { operation: "deleteSession" },
    );
  });

  it("returns true after undo succeeds and reloads the session state", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.loadSession.mockImplementation(async (sessionId: string) => ({
      id: sessionId,
      messages: [
        {
          id: "user-restored",
          role: "user",
          content: "恢复到输入框的内容",
          createdAt: 1,
        },
      ],
      agentId: null,
      sessionType: "chat",
    }));
    undoData.s1 = [];

    await chatStore.selectSession("s1");
    const result = await chatStore.performUndo("assistant-1");

    expect(result).toBe(true);
    expect(undoServiceMocks.undoPerform).toHaveBeenCalledWith("s1", "assistant-1", false);
    expect(chatStore.messages).toHaveLength(1);
    expect(chatStore.messages[0]).toMatchObject({
      id: "user-restored",
      content: "恢复到输入框的内容",
    });
  });

  it("returns true after conversation-only undo reloads the session state", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.undoLatestConversationTurn.mockImplementation(async (sessionId: string) => ({
      id: sessionId,
      title: `Session ${sessionId}`,
      messages: [
        {
          id: "user-kept",
          role: "user",
          content: "保留的上一轮",
          createdAt: 1,
        },
      ],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: null,
      createdAt: 0,
      updatedAt: 0,
    }));
    undoData.s1 = [];

    await chatStore.selectSession("s1");
    const result = await chatStore.undoLatestConversationTurn();

    expect(result).toBe(true);
    expect(sessionServiceMocks.undoLatestConversationTurn).toHaveBeenCalledWith("s1");
    expect(chatStore.messages).toHaveLength(1);
    expect(chatStore.messages[0]).toMatchObject({
      id: "user-kept",
      content: "保留的上一轮",
    });
  });

  it("rolls back conversation from an exact message without file undo", async () => {
    const chatStore = useChatStore();

    const rollbackDetail = {
      id: "s1",
      title: "Session s1",
      messages: [
        {
          id: "user-before",
          role: "user",
          content: "保留",
          createdAt: 1,
        },
        {
          id: "user-target",
          role: "user",
          content: "选中消息",
          createdAt: 2,
        },
      ],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: null,
      createdAt: 0,
      updatedAt: 0,
    };
    sessionServiceMocks.rollbackSessionToMessage.mockResolvedValueOnce(rollbackDetail);
    sessionServiceMocks.loadSession.mockResolvedValue(rollbackDetail);
    undoData.s1 = [];

    await chatStore.selectSession("s1");
    const result = await chatStore.rollbackToMessage("user-target", { includeFiles: false });

    expect(result).toBe(true);
    expect(sessionServiceMocks.rollbackSessionToMessage).toHaveBeenCalledWith("s1", "user-target");
    expect(undoServiceMocks.undoPerformToMessage).not.toHaveBeenCalled();
    expect(chatStore.messages.map((message) => message.id)).toEqual(["user-before", "user-target"]);
  });

  it("rolls back files from the requested assistant while preserving the selected message", async () => {
    const chatStore = useChatStore();

    sessionServiceMocks.loadSession.mockImplementation(async (sessionId: string) => ({
      id: sessionId,
      title: "Session s1",
      messages: [
        {
          id: "user-before",
          role: "user",
          content: "保留",
          createdAt: 1,
        },
        {
          id: "user-target",
          role: "user",
          content: "选中消息",
          createdAt: 2,
        },
      ],
      agentId: null,
      sessionType: "chat",
      parentSessionId: null,
      latestCompletedRunId: null,
      createdAt: 0,
      updatedAt: 0,
    }));
    undoData.s1 = [];

    await chatStore.selectSession("s1");
    const result = await chatStore.rollbackToMessage("user-target", {
      includeFiles: true,
      fileUndoTarget: "assistant-after",
    });

    expect(result).toBe(true);
    expect(undoServiceMocks.undoPerformToMessage).toHaveBeenCalledWith(
      "s1",
      "assistant-after",
      "user-target",
      false,
    );
    expect(sessionServiceMocks.rollbackSessionToMessage).not.toHaveBeenCalled();
    expect(chatStore.messages.map((message) => message.id)).toEqual(["user-before", "user-target"]);
  });

  it("returns false when undo fails", async () => {
    const chatStore = useChatStore();

    await chatStore.selectSession("s1");
    undoServiceMocks.undoPerform.mockRejectedValueOnce(new Error("undo failed"));

    const result = await chatStore.performUndo("assistant-1");

    expect(result).toBe(false);
    expect(notificationStoreMocks.addNotice).toHaveBeenCalled();
  });
});
