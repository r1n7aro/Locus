// @vitest-environment jsdom
import { createApp, defineComponent, h } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { publishSessionStreamEvent } from "../services/sessionStreamEventHub";
import { useEmbeddedChatSession } from "../composables/useEmbeddedChatSession";

const mocks = vi.hoisted(() => ({
  loadSession: vi.fn(),
  getSessionUsage: vi.fn(),
  rollbackSessionToMessage: vi.fn(),
  undoLatestConversationTurn: vi.fn(),
  undoList: vi.fn(),
  undoCheckDirty: vi.fn(),
  undoPerform: vi.fn(),
  undoPerformToMessage: vi.fn(),
  chat: vi.fn(),
  cancelChat: vi.fn(),
  tauriListen: vi.fn().mockResolvedValue(vi.fn()),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.tauriListen,
}));

vi.mock("../services/session", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../services/session")>();
  return {
    ...actual,
    loadSession: mocks.loadSession,
    getSessionUsage: mocks.getSessionUsage,
    rollbackSessionToMessage: mocks.rollbackSessionToMessage,
    undoLatestConversationTurn: mocks.undoLatestConversationTurn,
    chat: mocks.chat,
    cancelChat: mocks.cancelChat,
  };
});

vi.mock("../stores/model", () => ({
  useModelStore: () => ({
    modelDefaults: {
      subagentModels: {},
      subagentEfforts: {},
      subagentFastModes: {},
    },
  }),
}));

vi.mock("../services/undo", () => ({
  undoList: mocks.undoList,
  undoCheckDirty: mocks.undoCheckDirty,
  undoPerform: mocks.undoPerform,
  undoPerformToMessage: mocks.undoPerformToMessage,
}));

describe("embedded chat session distribution", () => {
  beforeEach(() => {
    mocks.undoList.mockResolvedValue([]);
    mocks.undoCheckDirty.mockResolvedValue([]);
    mocks.undoPerform.mockResolvedValue(undefined);
    mocks.undoPerformToMessage.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
    mocks.loadSession.mockReset();
    mocks.getSessionUsage.mockReset();
    mocks.rollbackSessionToMessage.mockReset();
    mocks.undoLatestConversationTurn.mockReset();
    mocks.undoList.mockReset();
    mocks.undoCheckDirty.mockReset();
    mocks.undoPerform.mockReset();
    mocks.undoPerformToMessage.mockReset();
    mocks.chat.mockReset();
    mocks.cancelChat.mockReset();
    mocks.tauriListen.mockClear();
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
    mocks.loadSession.mockResolvedValue(detail([targetMessage, laterMessage]));
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
});
