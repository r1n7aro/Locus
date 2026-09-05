// @vitest-environment jsdom
import { createApp, defineComponent, h } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  publishSessionAsyncTaskUpdate,
  publishSessionExecutionState,
  publishSessionStreamEvent,
} from "../services/sessionStreamEventHub";
import { useEmbeddedChatSession } from "../composables/useEmbeddedChatSession";

const mocks = vi.hoisted(() => ({
  loadSession: vi.fn(),
  loadSessionView: vi.fn(),
  loadSessionMessagePage: vi.fn(),
  loadSessionTurnPreview: vi.fn(),
  getSessionUsage: vi.fn(),
  getTodos: vi.fn(),
  getSessionResumeAvailable: vi.fn(),
  getSessionPlanState: vi.fn(),
  setSessionPlanMode: vi.fn(),
  forkSession: vi.fn(),
  forkSessionFromMessage: vi.fn(),
  rollbackSessionToMessage: vi.fn(),
  undoLatestConversationTurn: vi.fn(),
  undoList: vi.fn(),
  undoCheckConflicts: vi.fn(),
  undoCheckDirty: vi.fn(),
  undoPerform: vi.fn(),
  undoPerformToMessage: vi.fn(),
  chat: vi.fn(),
  queueSessionCompact: vi.fn(),
  cancelChat: vi.fn(),
  tauriListen: vi.fn().mockResolvedValue(vi.fn()),
  modelDefaults: {
    planModel: "",
    subagentModels: {},
    subagentEfforts: {},
    subagentFastModes: {},
  },
  availableModels: [{
    id: "model-a",
    name: "Model A",
    provider: "openrouter",
    contextWindow: 128_000,
    supportedEfforts: ["none", "low", "medium", "high"],
  }],
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.tauriListen,
}));

vi.mock("../services/session", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../services/session")>();
  return {
    ...actual,
    loadSession: mocks.loadSession,
    loadSessionView: mocks.loadSessionView,
    loadSessionMessagePage: mocks.loadSessionMessagePage,
    loadSessionTurnPreview: mocks.loadSessionTurnPreview,
    getSessionUsage: mocks.getSessionUsage,
    getTodos: mocks.getTodos,
    getSessionResumeAvailable: mocks.getSessionResumeAvailable,
    getSessionPlanState: mocks.getSessionPlanState,
    setSessionPlanMode: mocks.setSessionPlanMode,
    forkSession: mocks.forkSession,
    forkSessionFromMessage: mocks.forkSessionFromMessage,
    rollbackSessionToMessage: mocks.rollbackSessionToMessage,
    undoLatestConversationTurn: mocks.undoLatestConversationTurn,
    chat: mocks.chat,
    queueSessionCompact: mocks.queueSessionCompact,
    cancelChat: mocks.cancelChat,
  };
});

vi.mock("../stores/model", () => ({
  useModelStore: () => ({
    modelDefaults: mocks.modelDefaults,
    availableModels: mocks.availableModels,
  }),
}));

vi.mock("../services/undo", () => ({
  undoList: mocks.undoList,
  undoCheckConflicts: mocks.undoCheckConflicts,
  undoCheckDirty: mocks.undoCheckDirty,
  undoPerform: mocks.undoPerform,
  undoPerformToMessage: mocks.undoPerformToMessage,
}));

describe("embedded chat session distribution", () => {
  beforeEach(() => {
    mocks.loadSessionView.mockImplementation(async (sessionId: string) => {
      const session = await mocks.loadSession(sessionId);
      return {
        session,
        userMessageIds: session.messages
          .filter((message: { role: string }) => message.role === "user")
          .map((message: { id: string }) => message.id),
        oldestMessageRowId: null,
        hasMoreHistory: false,
      };
    });
    mocks.getSessionUsage.mockResolvedValue(null);
    mocks.getTodos.mockResolvedValue({ items: [], latestRunId: null });
    mocks.getSessionResumeAvailable.mockResolvedValue(false);
    mocks.getSessionPlanState.mockResolvedValue({
      active: false,
      planFilePath: "",
      planFileExists: false,
    });
    mocks.undoList.mockResolvedValue([]);
    mocks.undoCheckConflicts.mockResolvedValue([]);
    mocks.undoCheckDirty.mockResolvedValue([]);
    mocks.undoPerform.mockResolvedValue(undefined);
    mocks.undoPerformToMessage.mockResolvedValue(undefined);
    mocks.modelDefaults.planModel = "";
    mocks.availableModels.splice(0, mocks.availableModels.length, {
      id: "model-a",
      name: "Model A",
      provider: "openrouter",
      contextWindow: 128_000,
      supportedEfforts: ["none", "low", "medium", "high"],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    mocks.loadSession.mockReset();
    mocks.loadSessionView.mockReset();
    mocks.loadSessionMessagePage.mockReset();
    mocks.loadSessionTurnPreview.mockReset();
    mocks.getSessionUsage.mockReset();
    mocks.getTodos.mockReset();
    mocks.getSessionResumeAvailable.mockReset();
    mocks.getSessionPlanState.mockReset();
    mocks.setSessionPlanMode.mockReset();
    mocks.forkSession.mockReset();
    mocks.forkSessionFromMessage.mockReset();
    mocks.rollbackSessionToMessage.mockReset();
    mocks.undoLatestConversationTurn.mockReset();
    mocks.undoList.mockReset();
    mocks.undoCheckConflicts.mockReset();
    mocks.undoCheckDirty.mockReset();
    mocks.undoPerform.mockReset();
    mocks.undoPerformToMessage.mockReset();
    mocks.chat.mockReset();
    mocks.queueSessionCompact.mockReset();
    mocks.cancelChat.mockReset();
    mocks.tauriListen.mockClear();
  });

  it("restores multi agent per session, synchronizes it, and forwards it to each run", async () => {
    mocks.loadSession.mockImplementation(async (id: string) => ({
      id, title: id, sessionType: "chat", parentSessionId: null,
      createdAt: 1, updatedAt: 1, messages: [], pendingInputs: [],
      lastMultiAgentEnabled: id === "multi-agent-on" ? true : undefined,
    }));
    mocks.chat.mockImplementation(async (request) => ({ sessionId: request.sessionId, runId: `run-${request.sessionId}` }));
    let first!: ReturnType<typeof useEmbeddedChatSession>;
    let second!: ReturnType<typeof useEmbeddedChatSession>;
    const app = createApp(defineComponent({
      setup() {
        const common = {
          workspaceRef: { checkoutId: "checkout-multi-agent", expectedGeneration: 1 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        first = useEmbeddedChatSession({ ...common, sessionKey: "multi-agent-pane-on", initialSessionId: "multi-agent-on" });
        second = useEmbeddedChatSession({ ...common, sessionKey: "multi-agent-pane-off", initialSessionId: "multi-agent-off" });
        return () => h("div");
      },
    }));
    app.mount(document.createElement("div"));
    try {
      await vi.waitFor(() => expect(first.sessionMultiAgentEnabled.value).toBe(true));
      expect(second.sessionMultiAgentEnabled.value).toBe(false);
      publishSessionExecutionState({ sessionId: "multi-agent-on", modelId: "model-a", effort: "max", fastMode: false, multiAgentEnabled: false });
      expect(first.sessionMultiAgentEnabled.value).toBe(false);
      expect(second.sessionMultiAgentEnabled.value).toBe(false);
      publishSessionExecutionState({ sessionId: "multi-agent-on", modelId: "model-a", effort: "high", fastMode: false, multiAgentEnabled: true });
      await first.compact();
      await second.compact();
      expect(mocks.chat).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "multi-agent-on", multiAgentEnabled: true }));
      expect(mocks.chat).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "multi-agent-off", multiAgentEnabled: false }));
    } finally {
      app.unmount();
    }
  });

  it("starts manual compaction for an idle workbench session without adding a user message", async () => {
    const sessionId = "idle-workbench-compact-session";
    const existingMessage = {
      id: "user-before-compact",
      role: "user" as const,
      content: "Keep this message",
      createdAt: 1,
    };
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Idle compact",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [existingMessage],
      pendingInputs: [],
    });
    mocks.getSessionUsage.mockResolvedValue(null);
    mocks.chat.mockResolvedValue({ sessionId, runId: "manual-compact-run" });

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "idle-workbench-compact-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-compact", expectedGeneration: 3 },
          selectedModelId: "model-a",
          selectedAgentId: "unity",
          effort: "high",
          effortSupported: true,
          fastMode: true,
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.messages.value).toHaveLength(1));

    await expect(session.compact()).resolves.toBe(true);

    expect(mocks.chat).toHaveBeenCalledWith(expect.objectContaining({
      sessionId,
      text: "",
      mode: "compact",
      images: null,
      assetRefs: null,
      userIntent: null,
    }));
    expect(session.messages.value).toEqual([existingMessage]);
    expect(session.isStreaming.value).toBe(true);

    app.unmount();
  });

  it("queues manual compaction behind the active workbench run", async () => {
    const sessionId = "running-workbench-compact-session";
    const runId = "running-workbench-run";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Running compact",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [],
      pendingInputs: [],
      runtime: {
        activeRun: {
          runId,
          sessionId,
          status: "running",
          startedAt: 1,
          updatedAt: 1,
        },
        activeToolCalls: [],
        pendingToolConfirms: [],
        isCompacting: false,
        compactQueued: false,
      },
    });
    mocks.getSessionUsage.mockResolvedValue(null);
    mocks.queueSessionCompact.mockResolvedValue(true);

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "running-workbench-compact-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-running-compact", expectedGeneration: 5 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.isStreaming.value).toBe(true));

    await expect(session.compact()).resolves.toBe(true);

    expect(mocks.queueSessionCompact).toHaveBeenCalledWith(sessionId, runId);
    expect(mocks.chat).not.toHaveBeenCalled();
    expect(session.compactQueued.value).toBe(true);

    publishSessionStreamEvent({
      event: {
        type: "compactStart",
        sessionId,
        runId,
        contextTokens: 100,
        contextLimit: 1_000,
      },
      source: {
        kind: "workspace",
        projectId: "project-running-compact",
        checkoutId: "checkout-running-compact",
        workspaceGeneration: 5,
        streamRevision: 1,
      },
    });
    expect(session.compactQueued.value).toBe(false);
    expect(session.isCompacting.value).toBe(true);

    app.unmount();
  });

  it("queues manual compaction requested while a workbench run is still launching", async () => {
    const sessionId = "launching-workbench-compact-session";
    const runId = "launching-workbench-run";
    let resolveLaunch!: (value: { sessionId: string; runId: string }) => void;
    mocks.chat.mockReturnValue(new Promise((resolve) => {
      resolveLaunch = resolve;
    }));
    mocks.queueSessionCompact.mockResolvedValue(true);

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "launching-workbench-compact-pane",
          workspaceRef: { checkoutId: "checkout-launching-compact", expectedGeneration: 8 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));

    const sendPromise = session.send({ text: "Start the run" });
    await vi.waitFor(() => expect(session.isStreaming.value).toBe(true));
    await expect(session.compact()).resolves.toBe(true);
    expect(mocks.queueSessionCompact).not.toHaveBeenCalled();

    resolveLaunch({ sessionId, runId });
    await sendPromise;
    await vi.waitFor(() => {
      expect(mocks.queueSessionCompact).toHaveBeenCalledWith(sessionId, runId);
    });
    expect(session.compactQueued.value).toBe(true);

    app.unmount();
  });

  it("refreshes the pane after conversation rollback when file undo is unavailable", async () => {
    const sessionId = "conversation-only-rollback-session";
    const targetMessage = {
      id: "assistant-target",
      role: "assistant" as const,
      content: "Keep this answer",
      createdAt: 1,
    };
    const laterMessage = {
      id: "user-later",
      role: "user" as const,
      content: "Remove this turn",
      createdAt: 2,
    };
    const detail = (messages: typeof targetMessage[] | Array<typeof targetMessage | typeof laterMessage>) => ({
      id: sessionId,
      title: "Conversation rollback",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 2,
      messages,
      pendingInputs: [],
    });
    mocks.loadSession
      .mockResolvedValueOnce(detail([targetMessage, laterMessage]))
      .mockResolvedValue(detail([targetMessage]));
    mocks.getSessionUsage.mockResolvedValue(null);
    mocks.undoList.mockResolvedValue([]);
    mocks.rollbackSessionToMessage.mockResolvedValue(detail([targetMessage]));

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "conversation-only-rollback-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-rollback", expectedGeneration: 4 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.messages.value).toHaveLength(2));

    expect(session.undoableMessageIds.value.size).toBe(0);
    await expect(session.rollbackConversation("assistant-target")).resolves.toBe(true);
    expect(mocks.rollbackSessionToMessage).toHaveBeenCalledWith(sessionId, "assistant-target");
    expect(session.messages.value.map((message) => message.id)).toEqual(["assistant-target"]);

    await expect(session.performUndo("assistant-target", {
      force: true,
      acceptDirty: true,
    })).resolves.toBe(true);
    expect(mocks.undoPerform).toHaveBeenCalledWith(
      sessionId,
      "assistant-target",
      true,
      true,
    );

    app.unmount();
  });

  it("keeps the raw Tauri stream listener out of session pane lifecycles", async () => {
    const Root = defineComponent({
      setup() {
        const common = {
          workspaceRef: { checkoutId: "checkout-transport-owner", expectedGeneration: 2 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        useEmbeddedChatSession({ ...common, sessionKey: "transport-pane-a" });
        useEmbeddedChatSession({ ...common, sessionKey: "transport-pane-b" });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await Promise.resolve();
    app.unmount();

    expect(mocks.tauriListen).not.toHaveBeenCalled();
  });

  it("reduces a session event once while two panes observe the same durable session", async () => {
    const sessionId = "shared-session-distribution";
    const runId = "shared-run-distribution";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Shared session",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [],
      pendingInputs: [],
      runtime: {
        activeRun: {
          runId,
          sessionId,
          status: "running",
          startedAt: 1,
          updatedAt: 1,
        },
        activeToolCalls: [],
        pendingToolConfirms: [],
        isCompacting: false,
      },
    });
    mocks.getSessionUsage.mockResolvedValue(null);

    let first!: ReturnType<typeof useEmbeddedChatSession>;
    let second!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        const common = {
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 3 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        first = useEmbeddedChatSession({ ...common, sessionKey: "pane-a" });
        second = useEmbeddedChatSession({ ...common, sessionKey: "pane-b" });
        return () => h("div");
      },
    });
    const host = document.createElement("div");
    const app = createApp(Root);
    app.mount(host);
    await vi.waitFor(() => {
      expect(first.isStreaming.value).toBe(true);
      expect(second.isStreaming.value).toBe(true);
    });

    expect(first.messages.value).toBe(second.messages.value);
    first.inputText.value = "pane-local draft";
    expect(second.inputText.value).toBe("");

    vi.useFakeTimers();
    const textEvent = {
      type: "textDelta" as const,
      sessionId,
      runId,
      text: "Hello",
    };
    publishSessionStreamEvent({
      event: textEvent,
      source: {
        kind: "workspace",
        projectId: "project-a",
        checkoutId: "checkout-a",
        workspaceGeneration: 3,
        streamRevision: 1,
      },
    });
    await vi.advanceTimersByTimeAsync(100);

    expect(first.streamingText.value).toBe("Hello");
    expect(second.streamingText.value).toBe("Hello");

    publishSessionStreamEvent({
      event: {
        type: "done",
        sessionId,
        runId,
        messageId: "assistant-1",
        fullText: "Hello",
      },
      source: {
        kind: "workspace",
        projectId: "project-a",
        checkoutId: "checkout-a",
        workspaceGeneration: 3,
        streamRevision: 2,
      },
    });

    expect(first.isStreaming.value).toBe(false);
    expect(second.isStreaming.value).toBe(false);
    expect(first.messages.value).toBe(second.messages.value);
    expect(first.messages.value).toEqual([
      expect.objectContaining({
        id: "assistant-1",
        role: "assistant",
        content: "Hello",
      }),
    ]);

    app.unmount();
  });

  it("replays a terminal event received while a durable session editor is remounting", async () => {
    const sessionId = "shared-session-remount-gap";
    const runId = "shared-run-remount-gap";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Remount gap session",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [{
        id: "user-before-remount",
        role: "user",
        content: "Keep observing this run",
        createdAt: 1,
      }],
      pendingInputs: [],
      runtime: {
        activeRun: {
          runId,
          sessionId,
          status: "running",
          startedAt: 1,
          updatedAt: 1,
        },
        activeToolCalls: [],
        pendingToolConfirms: [],
        isCompacting: false,
      },
    });
    mocks.getSessionUsage.mockResolvedValue(null);

    const common = {
      initialSessionId: sessionId,
      workspaceRef: { checkoutId: "checkout-remount", expectedGeneration: 7 },
      selectedModelId: "model-a",
      buildRequest: (input: string) => ({ text: input }),
    };
    let first!: ReturnType<typeof useEmbeddedChatSession>;
    const FirstRoot = defineComponent({
      setup() {
        first = useEmbeddedChatSession({ ...common, sessionKey: "remount-gap-pane" });
        return () => h("div");
      },
    });
    const firstApp = createApp(FirstRoot);
    firstApp.mount(document.createElement("div"));
    await vi.waitFor(() => expect(first.isStreaming.value).toBe(true));
    firstApp.unmount();

    publishSessionStreamEvent({
      event: {
        type: "done",
        sessionId,
        runId,
        messageId: "assistant-during-remount",
        fullText: "The run finished while the editor was moving.",
      },
      source: {
        kind: "workspace",
        projectId: "project-remount",
        checkoutId: "checkout-remount",
        workspaceGeneration: 7,
        streamRevision: 1,
      },
    });

    let remounted!: ReturnType<typeof useEmbeddedChatSession>;
    const RemountedRoot = defineComponent({
      setup() {
        remounted = useEmbeddedChatSession({ ...common, sessionKey: "remount-gap-pane" });
        return () => h("div");
      },
    });
    const remountedApp = createApp(RemountedRoot);
    remountedApp.mount(document.createElement("div"));

    await vi.waitFor(() => expect(remounted.isStreaming.value).toBe(false));
    expect(remounted.messages.value).toContainEqual(expect.objectContaining({
      id: "assistant-during-remount",
      role: "assistant",
      content: "The run finished while the editor was moving.",
    }));
    expect(mocks.loadSession).toHaveBeenCalledTimes(1);
    remountedApp.unmount();
  });

  it("recovers from a stale run id when a newer run starts", async () => {
    const sessionId = "stale-run-recovery-session";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Stale run recovery",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [],
      pendingInputs: [],
      runtime: {
        activeRun: {
          runId: "stale-run",
          sessionId,
          status: "running",
          startedAt: 1,
          updatedAt: 1,
        },
        activeToolCalls: [],
        pendingToolConfirms: [],
        isCompacting: false,
      },
    });
    mocks.getSessionUsage.mockResolvedValue(null);

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "stale-run-recovery-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-stale-run", expectedGeneration: 11 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.isStreaming.value).toBe(true));

    const source = {
      kind: "workspace" as const,
      projectId: "project-stale-run",
      checkoutId: "checkout-stale-run",
      workspaceGeneration: 11,
      streamRevision: 1,
    };
    publishSessionStreamEvent({
      event: { type: "runStart", sessionId, runId: "current-run" },
      source,
    });
    publishSessionStreamEvent({
      event: {
        type: "done",
        sessionId,
        runId: "current-run",
        messageId: "current-run-assistant",
        fullText: "Recovered on the current run.",
      },
      source: { ...source, streamRevision: 2 },
    });

    expect(session.isStreaming.value).toBe(false);
    expect(session.messages.value).toContainEqual(expect.objectContaining({
      id: "current-run-assistant",
      content: "Recovered on the current run.",
    }));
    app.unmount();
  });

  it("refreshes backend state after cancelling an embedded session", async () => {
    const sessionId = "cancel-refresh-session";
    const base = {
      id: sessionId,
      title: "Cancel refresh",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      pendingInputs: [],
    };
    mocks.loadSession
      .mockResolvedValueOnce({
        ...base,
        messages: [],
        runtime: {
          activeRun: {
            runId: "cancel-refresh-run",
            sessionId,
            status: "running",
            startedAt: 1,
            updatedAt: 1,
          },
          activeToolCalls: [],
          pendingToolConfirms: [],
          isCompacting: false,
        },
      })
      .mockResolvedValueOnce({
        ...base,
        messages: [{
          id: "cancelled-assistant",
          role: "assistant",
          content: "Interrupted output",
          createdAt: 2,
        }],
        runtime: null,
      });
    mocks.getSessionUsage.mockResolvedValue(null);
    mocks.cancelChat.mockResolvedValue(undefined);

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "cancel-refresh-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-cancel-refresh", expectedGeneration: 12 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.isStreaming.value).toBe(true));

    await session.cancel();

    expect(mocks.cancelChat).toHaveBeenCalledWith(sessionId);
    expect(session.isCancelling.value).toBe(false);
    expect(session.isStreaming.value).toBe(false);
    expect(session.messages.value).toContainEqual(expect.objectContaining({
      id: "cancelled-assistant",
      content: "Interrupted output",
    }));
    app.unmount();
  });

  it("detaches a reset pane without clearing another pane bound to the same session", async () => {
    const sessionId = "shared-session-reset";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Previous session",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 1,
      messages: [{
        id: "user-before-reset",
        role: "user",
        content: "Keep this transcript",
        createdAt: 1,
      }],
      pendingInputs: [],
      runtime: null,
    });
    mocks.getSessionUsage.mockResolvedValue(null);

    let first!: ReturnType<typeof useEmbeddedChatSession>;
    let second!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        const common = {
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-reset", expectedGeneration: 4 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        first = useEmbeddedChatSession({ ...common, sessionKey: "reset-pane-a" });
        second = useEmbeddedChatSession({ ...common, sessionKey: "reset-pane-b" });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => {
      expect(first.messages.value).toHaveLength(1);
      expect(second.messages.value).toHaveLength(1);
    });

    first.resetSession();

    expect(first.sessionId.value).toBeNull();
    expect(first.messages.value).toEqual([]);
    expect(second.sessionId.value).toBe(sessionId);
    expect(second.messages.value).toEqual([
      expect.objectContaining({
        id: "user-before-reset",
        content: "Keep this transcript",
      }),
    ]);
    app.unmount();
  });

  it("defers cancellation until launch and keeps a visible prompt out of the composer", async () => {
    let resolveLaunch!: (value: { sessionId: string; runId: string }) => void;
    mocks.chat.mockImplementationOnce(() => new Promise((resolve) => {
      resolveLaunch = resolve;
    }));
    mocks.cancelChat.mockResolvedValue(undefined);

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "cancel-draft-pane",
          workspaceRef: { checkoutId: "checkout-cancel", expectedGeneration: 9 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input, displayText: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));

    session.inputText.value = "restore this prompt";
    const sending = session.send();
    await session.cancel();
    expect(mocks.cancelChat).not.toHaveBeenCalled();

    resolveLaunch({ sessionId: "cancel-session", runId: "cancel-run" });
    await sending;
    expect(mocks.cancelChat).toHaveBeenCalledWith("cancel-session");

    const pendingMessage = session.messages.value.find((message) => message.role === "user");
    publishSessionStreamEvent({
      event: {
        type: "userMessage",
        sessionId: "cancel-session",
        runId: "cancel-run",
        message: {
          id: "cancel-user-persisted",
          role: "user",
          content: "restore this prompt",
          createdAt: 1,
          thinkingSignature: pendingMessage?.thinkingSignature,
        },
      },
      source: {
        kind: "workspace",
        projectId: "project-cancel",
        checkoutId: "checkout-cancel",
        workspaceGeneration: 9,
        streamRevision: 1,
      },
    });

    publishSessionStreamEvent({
      event: {
        type: "cancelled",
        sessionId: "cancel-session",
        runId: "cancel-run",
        messageId: "assistant-cancelled",
        fullText: "partial",
      },
      source: {
        kind: "workspace",
        projectId: "project-cancel",
        checkoutId: "checkout-cancel",
        workspaceGeneration: 9,
        streamRevision: 2,
      },
    });

    expect(session.restoredComposerDraft.value).toBeNull();
    expect(session.messages.value).toContainEqual(expect.objectContaining({
      id: "cancel-user-persisted",
      content: "restore this prompt",
    }));
    app.unmount();
  });

  it("returns a backend-revoked prompt to its composer", async () => {
    mocks.chat.mockResolvedValueOnce({ sessionId: "revoked-session", runId: "revoked-run" });
    const promptText = [
      "revoked prompt",
      "",
      "<locus-local-files>",
      "These are local paths supplied by drag and drop.",
      "- file: `E:/cache/revoked-reference.psd`; type: psd",
      "</locus-local-files>",
    ].join("\n");

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "revoked-draft-pane",
          workspaceRef: { checkoutId: "checkout-revoked", expectedGeneration: 10 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({
            text: promptText,
            displayText: `${input}\n\nrevoked-reference.psd`,
          }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));

    session.inputText.value = "revoked prompt";
    await session.send();
    const pendingMessage = session.messages.value.find((message) => message.role === "user");
    const persistedMessage = {
      id: "revoked-user-persisted",
      role: "user" as const,
      content: "revoked prompt",
      createdAt: 1,
      thinkingSignature: pendingMessage?.thinkingSignature,
    };
    const source = {
      kind: "workspace" as const,
      projectId: "project-revoked",
      checkoutId: "checkout-revoked",
      workspaceGeneration: 10,
      streamRevision: 1,
    };
    publishSessionStreamEvent({
      event: {
        type: "userMessage",
        sessionId: "revoked-session",
        runId: "revoked-run",
        message: persistedMessage,
      },
      source,
    });
    publishSessionStreamEvent({
      event: {
        type: "cancelled",
        sessionId: "revoked-session",
        runId: "revoked-run",
        removedUserMessage: persistedMessage,
      },
      source: { ...source, streamRevision: 2 },
    });

    expect(session.messages.value).toEqual([]);
    expect(session.restoredComposerDraft.value?.text).toBe("revoked prompt");
    expect(session.restoredComposerDraft.value?.localFiles).toEqual([
      expect.objectContaining({
        path: "E:/cache/revoked-reference.psd",
        isDir: false,
        typeLabel: "psd",
      }),
    ]);
    app.unmount();
  });

  it("binds pre-launch events by the session id returned from each concurrent chat", async () => {
    let resolveFirst!: (value: { sessionId: string; runId: string }) => void;
    let resolveSecond!: (value: { sessionId: string; runId: string }) => void;
    const firstLaunch = new Promise<{ sessionId: string; runId: string }>((resolve) => {
      resolveFirst = resolve;
    });
    const secondLaunch = new Promise<{ sessionId: string; runId: string }>((resolve) => {
      resolveSecond = resolve;
    });
    mocks.chat.mockImplementation((request: { text: string }) => (
      request.text === "first" ? firstLaunch : secondLaunch
    ));

    let first!: ReturnType<typeof useEmbeddedChatSession>;
    let second!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        const common = {
          workspaceRef: { checkoutId: "checkout-concurrent", expectedGeneration: 8 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        first = useEmbeddedChatSession({ ...common, sessionKey: "concurrent-pane-a" });
        second = useEmbeddedChatSession({ ...common, sessionKey: "concurrent-pane-b" });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    vi.useFakeTimers();
    first.inputText.value = "first";
    second.inputText.value = "second";
    const firstSend = first.send();
    const secondSend = second.send();

    const source = {
      kind: "workspace" as const,
      projectId: "project-concurrent",
      checkoutId: "checkout-concurrent",
      workspaceGeneration: 8,
      streamRevision: 1,
    };
    publishSessionStreamEvent({
      event: { type: "runStart", sessionId: "session-second", runId: "run-second" },
      source,
    });
    publishSessionStreamEvent({
      event: {
        type: "textDelta",
        sessionId: "session-second",
        runId: "run-second",
        text: "second response",
      },
      source: { ...source, streamRevision: 2 },
    });
    publishSessionStreamEvent({
      event: { type: "runStart", sessionId: "session-first", runId: "run-first" },
      source: { ...source, streamRevision: 3 },
    });
    publishSessionStreamEvent({
      event: {
        type: "textDelta",
        sessionId: "session-first",
        runId: "run-first",
        text: "first response",
      },
      source: { ...source, streamRevision: 4 },
    });

    resolveSecond({ sessionId: "session-second", runId: "run-second" });
    resolveFirst({ sessionId: "session-first", runId: "run-first" });
    await Promise.all([firstSend, secondSend]);
    await vi.advanceTimersByTimeAsync(100);

    expect(first.sessionId.value).toBe("session-first");
    expect(second.sessionId.value).toBe("session-second");
    expect(first.streamingText.value).toBe("first response");
    expect(second.streamingText.value).toBe("second response");
    app.unmount();
  });

  it("hydrates pane-owned Plan, Resume, Todo and Undo state and accepts synthetic Plan events", async () => {
    const sessionId = "scoped-auxiliary-state-session";
    const workspaceRef = { checkoutId: "checkout-auxiliary", expectedGeneration: 12 };
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Auxiliary state",
      sessionType: "chat",
      parentSessionId: "parent-session",
      latestCompletedRunId: "completed-run",
      createdAt: 1,
      updatedAt: 2,
      messages: [],
      pendingInputs: [],
    });
    mocks.getTodos.mockResolvedValue({
      latestRunId: "completed-run",
      items: [{ content: "Verify pane state", status: "in_progress", priority: "high" }],
    });
    mocks.getSessionResumeAvailable.mockResolvedValue(true);
    mocks.getSessionPlanState.mockResolvedValue({
      active: true,
      planFilePath: "plan/scoped.md",
      planFileExists: true,
    });
    mocks.setSessionPlanMode.mockResolvedValue({
      active: true,
      planFilePath: "plan/new.md",
      planFileExists: true,
    });

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "scoped-auxiliary-state-pane",
          initialSessionId: sessionId,
          workspaceRef,
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(mocks.loadSessionView).toHaveBeenCalledWith(sessionId, expect.any(Number)));
    await vi.waitFor(() => expect(session.errorMessage.value).toBeNull());
    await vi.waitFor(() => expect(session.parentSessionId.value).toBe("parent-session"));

    expect(session.parentSessionId.value).toBe("parent-session");
    expect(session.planModeActive.value).toBe(true);
    expect(session.canResumeInterrupted.value).toBe(true);
    expect(session.visibleTodos.value).toEqual([
      { content: "Verify pane state", status: "in_progress", priority: "high" },
    ]);

    const source = {
      kind: "workspace" as const,
      projectId: "project-auxiliary",
      checkoutId: workspaceRef.checkoutId,
      workspaceGeneration: workspaceRef.expectedGeneration,
      streamRevision: 1,
    };
    publishSessionStreamEvent({
      event: { type: "runStart", sessionId, runId: "active-pane-run" },
      source,
    });
    publishSessionStreamEvent({
      event: {
        type: "planModeChanged",
        sessionId,
        runId: "synthetic-plan-command",
        active: false,
        planFilePath: null,
      },
      source: { ...source, streamRevision: 2 },
    });
    expect(session.planModeActive.value).toBe(false);

    publishSessionStreamEvent({
      event: {
        type: "undoAvailable",
        sessionId,
        runId: "active-pane-run",
        assistantMessageId: "assistant-undoable",
      },
      source: { ...source, streamRevision: 3 },
    });
    expect(session.undoableMessageIds.value.has("assistant-undoable")).toBe(true);

    publishSessionStreamEvent({
      event: {
        type: "toolCallStart",
        sessionId,
        runId: "active-pane-run",
        toolCallId: "todo-write",
        toolName: "todowrite",
        arguments: JSON.stringify({
          todos: [{ content: "Streamed todo", status: "pending", priority: "medium" }],
        }),
      },
      source: { ...source, streamRevision: 4 },
    });
    publishSessionStreamEvent({
      event: {
        type: "toolCallDone",
        sessionId,
        runId: "active-pane-run",
        toolCallId: "todo-write",
        toolName: "todowrite",
        output: "Todos updated",
        outcome: "done",
      },
      source: { ...source, streamRevision: 5 },
    });
    expect(session.currentTodos.value).toEqual([
      { content: "Streamed todo", status: "pending", priority: "medium" },
    ]);
    publishSessionStreamEvent({
      event: {
        type: "done",
        sessionId,
        runId: "active-pane-run",
        messageId: "assistant-finished",
        fullText: "Finished",
      },
      source: { ...source, streamRevision: 6 },
    });

    await expect(session.setPlanMode(true)).resolves.toBe(true);
    expect(mocks.setSessionPlanMode).toHaveBeenCalledWith(sessionId, true, workspaceRef);
    expect(session.planModeActive.value).toBe(true);
    publishSessionExecutionState({
      sessionId,
      modelId: "runtime-model",
      effort: "low",
      fastMode: true,
    });
    expect(session.sessionModelId.value).toBe("runtime-model");
    expect(session.sessionEffort.value).toBe("low");
    expect(session.sessionFastMode.value).toBe(true);
    app.unmount();
  });

  it("uses the configured Plan model and constrains pane Fast mode by the effective model", async () => {
    mocks.modelDefaults.planModel = "plan-model";
    mocks.availableModels.splice(
      0,
      mocks.availableModels.length,
      {
        id: "model-a",
        name: "Model A",
        provider: "openai_codex",
        contextWindow: 128_000,
        supportedEfforts: ["high"],
      },
      {
        id: "plan-model",
        name: "Plan Model",
        provider: "openrouter",
        contextWindow: 128_000,
        supportedEfforts: ["high"],
      },
    );
    mocks.chat.mockResolvedValue({ sessionId: "plan-model-session", runId: "plan-model-run" });

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "plan-model-pane",
          workspaceRef: { checkoutId: "checkout-plan-model", expectedGeneration: 21 },
          selectedModelId: "model-a",
          selectedAgentId: "unity-agent",
          effort: "high",
          effortSupported: true,
          fastMode: true,
          knowledgeMode: "read_only",
          knowledgeFocus: { docType: "design", path: "Design/combat.md" },
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));

    await session.send({ text: "Create a plan", mode: "plan" });

    expect(mocks.chat).toHaveBeenCalledWith(expect.objectContaining({
      workspaceRef: { checkoutId: "checkout-plan-model", expectedGeneration: 21 },
      model: "plan-model",
      effort: "high",
      fastMode: false,
      knowledgeMode: "read_only",
      knowledgeDocType: "design",
      knowledgeDocPath: "Design/combat.md",
      mode: "plan",
    }));
    expect(session.sessionModelId.value).toBe("plan-model");
    expect(session.sessionFastMode.value).toBe(false);
    expect(session.planModeActive.value).toBe(true);
    app.unmount();
  });

  it("resumes an interrupted pane through the shared hidden-run path", async () => {
    const sessionId = "scoped-resume-session";
    const trailingAssistant = {
      id: "assistant-interrupted",
      role: "assistant" as const,
      content: "",
      createdAt: 1,
      toolCalls: [{
        id: "tool-interrupted",
        name: "read",
        arguments: "{}",
      }],
    };
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Resume",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 2,
      messages: [trailingAssistant],
      pendingInputs: [],
    });
    mocks.getSessionResumeAvailable.mockResolvedValue(true);
    mocks.chat.mockResolvedValue({ sessionId, runId: "resumed-run" });

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "scoped-resume-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-resume", expectedGeneration: 4 },
          selectedModelId: "model-a",
          knowledgeMode: "disabled",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.canResumeInterrupted.value).toBe(true));

    await expect(session.resumeInterrupted()).resolves.toBe(true);

    expect(mocks.chat).toHaveBeenCalledWith(expect.objectContaining({
      sessionId,
      workspaceRef: { checkoutId: "checkout-resume", expectedGeneration: 4 },
      text: "",
      resume: true,
      mode: "build",
      knowledgeMode: "disabled",
    }));
    expect(session.messages.value.filter((message) => message.role === "user")).toEqual([]);
    expect(session.messages.value).toContainEqual(expect.objectContaining({
      id: "synthetic_tool_result:assistant-interrupted:tool-interrupted",
      role: "tool",
      toolCallId: "tool-interrupted",
    }));
    expect(session.canResumeInterrupted.value).toBe(false);
    expect(session.isStreaming.value).toBe(true);
    app.unmount();
  });

  it("loads paged history and turn previews for the pane session only", async () => {
    const sessionId = "scoped-history-session";
    mocks.loadSessionView.mockResolvedValueOnce({
      session: {
        id: sessionId,
        title: "History",
        sessionType: "chat",
        parentSessionId: null,
        createdAt: 1,
        updatedAt: 2,
        messages: [{ id: "message-new", role: "assistant", content: "new", createdAt: 2 }],
        pendingInputs: [],
      },
      userMessageIds: ["message-old"],
      oldestMessageRowId: 20,
      hasMoreHistory: true,
    });
    mocks.loadSessionMessagePage.mockResolvedValueOnce({
      messages: [{ id: "message-old", role: "user", content: "old", createdAt: 1 }],
      oldestMessageRowId: 10,
      hasMoreHistory: false,
    });
    mocks.loadSessionTurnPreview.mockResolvedValueOnce({
      messageId: "message-old",
      prompt: "old",
      response: "new",
    });

    let session!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        session = useEmbeddedChatSession({
          sessionKey: "scoped-history-pane",
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-history", expectedGeneration: 7 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(session.sessionHistoryHasMore.value).toBe(true));

    await expect(session.loadSessionTurnPreview("message-old")).resolves.toEqual({
      messageId: "message-old",
      prompt: "old",
      response: "new",
    });
    await expect(session.loadSessionHistoryThroughMessage("message-old")).resolves.toBe(true);

    expect(mocks.loadSessionTurnPreview).toHaveBeenCalledWith(sessionId, "message-old");
    expect(mocks.loadSessionMessagePage).toHaveBeenCalledWith(sessionId, 20, expect.any(Number));
    expect(session.messages.value.map((message) => message.id)).toEqual([
      "message-old",
      "message-new",
    ]);
    expect(session.sessionHistoryHasMore.value).toBe(false);
    expect(session.sessionUserMessageIds.value).toEqual(["message-old"]);
    app.unmount();
  });

  it("applies one async task update to shared durable state observed by two panes", async () => {
    const sessionId = "shared-async-task-session";
    mocks.loadSession.mockResolvedValue({
      id: sessionId,
      title: "Async task",
      sessionType: "chat",
      parentSessionId: null,
      createdAt: 1,
      updatedAt: 2,
      messages: [{
        id: "assistant-async",
        role: "assistant",
        content: "",
        createdAt: 1,
        toolCalls: [{ id: "tool-async", name: "background_task", arguments: "{}" }],
      }],
      pendingInputs: [],
    });

    let first!: ReturnType<typeof useEmbeddedChatSession>;
    let second!: ReturnType<typeof useEmbeddedChatSession>;
    const Root = defineComponent({
      setup() {
        const common = {
          initialSessionId: sessionId,
          workspaceRef: { checkoutId: "checkout-async", expectedGeneration: 2 },
          selectedModelId: "model-a",
          buildRequest: (input: string) => ({ text: input }),
        };
        first = useEmbeddedChatSession({ ...common, sessionKey: "async-pane-a" });
        second = useEmbeddedChatSession({ ...common, sessionKey: "async-pane-b" });
        return () => h("div");
      },
    });
    const app = createApp(Root);
    app.mount(document.createElement("div"));
    await vi.waitFor(() => expect(first.messages.value).toHaveLength(1));

    publishSessionAsyncTaskUpdate({
      sessionId,
      assistantMessageId: "assistant-async",
      toolCallId: "tool-async",
      taskId: "task-async",
      toolName: "background_task",
      status: "completed",
      output: "Async result",
    });

    expect(first.messages.value).toBe(second.messages.value);
    expect(first.messages.value[0]?.toolCalls?.[0]).toEqual(expect.objectContaining({
      id: "tool-async",
      outcome: "done",
      recordedOutput: "Async result",
    }));
    app.unmount();
  });
});
