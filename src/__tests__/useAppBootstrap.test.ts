import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick, reactive } from "vue";

let uiStoreMock: any;
let authStoreMock: any;
let agentStoreMock: any;
let modelStoreMock: any;
let projectStoreMock: any;
let chatStoreMock: any;
let notificationStoreMock: any;
let loadSkillsMock: ReturnType<typeof vi.fn>;
let maybeNotifyStreamEventMock: any;
let resetSystemNotificationStateMock: any;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  WebviewWindow: class {
    constructor(..._args: any[]) {}
    once(..._args: any[]) {}
  },
}));

vi.mock("../services/canvas", () => ({
  canvasSetSpec: vi.fn(),
}));

vi.mock("../stores/ui", () => ({
  useUiStore: () => uiStoreMock,
}));

vi.mock("../stores/auth", () => ({
  useAuthStore: () => authStoreMock,
}));

vi.mock("../stores/agent", () => ({
  useAgentStore: () => agentStoreMock,
}));

vi.mock("../stores/model", () => ({
  useModelStore: () => modelStoreMock,
}));

vi.mock("../stores/project", () => ({
  useProjectStore: () => projectStoreMock,
}));

vi.mock("../stores/chat", () => ({
  useChatStore: () => chatStoreMock,
}));

vi.mock("../stores/notification", () => ({
  useNotificationStore: () => notificationStoreMock,
}));

vi.mock("../composables/useSkills", () => ({
  useSkills: () => ({
    skillItems: [],
    loadSkills: loadSkillsMock,
  }),
}));

vi.mock("../services/errors", () => ({
  normalizeAppError: (error: unknown) => error,
}));

vi.mock("../services/systemNotifications", () => ({
  maybeNotifyStreamEvent: (...args: unknown[]) => maybeNotifyStreamEventMock(...args),
  resetSystemNotificationState: (...args: unknown[]) => resetSystemNotificationStateMock(...args),
}));

vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: () => true,
}));

vi.mock("../composables/warmupCache", () => ({
  setScope: vi.fn(),
  setWarmup: vi.fn(),
  clearWarmup: vi.fn(),
}));

vi.mock("../services/auth", () => ({
  getProviders: vi.fn(),
  codexStatus: vi.fn(),
}));

vi.mock("../services/model", () => ({
  getModelDefaults: vi.fn(),
  getCustomEndpoints: vi.fn(),
}));

vi.mock("../services/permissions", () => ({
  getToolPermissions: vi.fn(),
}));

vi.mock("../services/git", () => ({
  gitProbe: vi.fn(),
  gitHistorySnapshot: vi.fn(),
  gitStatus: vi.fn(),
  gitBranches: vi.fn(),
  gitSubmodules: vi.fn(),
}));

vi.mock("../services/knowledge", () => ({
  knowledgeList: vi.fn(),
  knowledgeListPage: vi.fn().mockResolvedValue({
    items: [],
    nextCursor: null,
  }),
  knowledgeGetLexicalRebuildStatus: vi.fn().mockResolvedValue({
    running: false,
    stage: null,
    detail: null,
    currentFile: null,
    processedDocs: null,
    totalDocs: null,
    error: null,
    startedAt: null,
    completedAt: null,
  }),
}));

vi.mock("../services/knowledgeLexicalProgressWindow", () => ({
  getKnowledgeLexicalProgressRunKey: vi.fn().mockReturnValue(""),
  isKnowledgeLexicalProgressWindowLocation: () => false,
  KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT: "knowledge-lexical-rebuild-status",
  openKnowledgeLexicalProgressWindow: vi.fn().mockResolvedValue(undefined),
  shouldAutoOpenKnowledgeLexicalProgressWindow: vi.fn().mockReturnValue(false),
}));

vi.mock("../services/agent", () => ({
  listAgents: vi.fn(),
  listSubagentDefs: vi.fn(),
}));

vi.mock("../config/providerVisibility", () => ({
  filterVisibleProviders: (providers: unknown) => providers,
}));

vi.mock("../i18n", () => ({
  t: (key: string, ...args: (string | number)[]) =>
    args.length > 0 ? `${key}: ${args.join(" ")}` : key,
}));

async function loadUseAppBootstrap() {
  const mod = await import("../composables/useAppBootstrap");
  return mod.useAppBootstrap;
}

describe("useAppBootstrap onboarding completion", () => {
  beforeEach(() => {
    loadSkillsMock = vi.fn().mockResolvedValue(undefined);
    maybeNotifyStreamEventMock = vi.fn().mockResolvedValue(undefined);
    resetSystemNotificationStateMock = vi.fn();

    uiStoreMock = reactive({
      activeTab: "chat",
      completeOnboarding: vi.fn(),
      init: vi.fn().mockResolvedValue(undefined),
      setTab: vi.fn(),
      cleanup: vi.fn(),
    });

    authStoreMock = reactive({
      checkAuth: vi.fn().mockResolvedValue([]),
    });

    agentStoreMock = reactive({
      selectedAgentId: "",
      agents: [],
      loadAgents: vi.fn().mockResolvedValue(undefined),
    });

    modelStoreMock = reactive({
      effort: "none",
      defaultEffort: "none",
      hasUserDefaultEffort: false,
      loadModelDefaults: vi.fn().mockResolvedValue(undefined),
      loadLastModel: vi.fn().mockResolvedValue(undefined),
      loadLastEffort: vi.fn().mockResolvedValue(undefined),
      loadCustomEndpoints: vi.fn().mockResolvedValue(undefined),
      loadCodexModelConfig: vi.fn().mockResolvedValue(undefined),
      loadCodexAvailableModels: vi.fn().mockResolvedValue(undefined),
      resolveSelectedModel: vi.fn(),
      applyContextEffort: vi.fn((level: string | null | undefined) => {
        modelStoreMock.effort = level || "none";
      }),
      restoreDefaultEffort: vi.fn(() => {
        modelStoreMock.effort = modelStoreMock.defaultEffort;
      }),
    });

    projectStoreMock = reactive({
      workingDir: "",
      loadWorkingDir: vi.fn().mockResolvedValue(undefined),
      loadRecentDirs: vi.fn().mockResolvedValue(undefined),
      checkUnityConnection: vi.fn().mockResolvedValue(undefined),
      checkUnityPlugin: vi.fn().mockResolvedValue(undefined),
      loadAssetDbStatus: vi.fn().mockResolvedValue(undefined),
      handleUnityConnectionStatus: vi.fn(),
      handleUnityConnectionStatusDetail: vi.fn(),
    });

    chatStoreMock = reactive({
      activeSessionId: null,
      sessions: [],
      refreshSessions: vi.fn().mockResolvedValue(undefined),
      loadToolPermissionMode: vi.fn().mockResolvedValue(undefined),
      setCanvasAutoOpenCallback: vi.fn(),
      handleStreamEvent: vi.fn().mockReturnValue(true),
      cleanupAnim: vi.fn(),
    });

    notificationStoreMock = {
      addNotice: vi.fn(),
    };
  });

  it("uses the agent default effort when no user default exists", async () => {
    chatStoreMock.activeSessionId = "session-1";
    agentStoreMock.selectedAgentId = "git";
    agentStoreMock.agents = [
      { id: "dev", defaultEffort: "medium" },
      { id: "git", defaultEffort: "low" },
    ];

    const useAppBootstrap = await loadUseAppBootstrap();
    useAppBootstrap();
    await nextTick();

    expect(modelStoreMock.applyContextEffort).toHaveBeenLastCalledWith("low");

    modelStoreMock.applyContextEffort.mockClear();
    modelStoreMock.restoreDefaultEffort.mockClear();

    chatStoreMock.activeSessionId = null;
    agentStoreMock.selectedAgentId = "dev";
    await nextTick();

    expect(modelStoreMock.restoreDefaultEffort).toHaveBeenCalledTimes(1);
    expect(modelStoreMock.applyContextEffort).not.toHaveBeenCalled();
    expect(modelStoreMock.effort).toBe("none");
  });

  it("keeps the saved user default effort while a session is active", async () => {
    modelStoreMock.defaultEffort = "high";
    modelStoreMock.hasUserDefaultEffort = true;
    chatStoreMock.activeSessionId = "session-1";
    agentStoreMock.selectedAgentId = "dev";
    agentStoreMock.agents = [
      { id: "dev", defaultEffort: "medium" },
    ];

    const useAppBootstrap = await loadUseAppBootstrap();
    useAppBootstrap();
    await nextTick();

    expect(modelStoreMock.restoreDefaultEffort).toHaveBeenCalledTimes(1);
    expect(modelStoreMock.applyContextEffort).not.toHaveBeenCalled();
    expect(modelStoreMock.effort).toBe("high");
  });

  it("reloads sessions after onboarding completes", async () => {
    const useAppBootstrap = await loadUseAppBootstrap();
    const { onOnboardingCompleted } = useAppBootstrap();

    await onOnboardingCompleted();

    expect(uiStoreMock.completeOnboarding).toHaveBeenCalledTimes(1);
    expect(modelStoreMock.loadLastEffort).toHaveBeenCalledTimes(1);
    expect(chatStoreMock.refreshSessions).toHaveBeenCalledTimes(1);
    expect(projectStoreMock.loadWorkingDir).toHaveBeenCalledTimes(1);
  });

  it("shows sticky startup banners when auth restore fails", async () => {
    authStoreMock.checkAuth.mockResolvedValue([
      {
        target: "providers",
        error: {
          code: "providers_failed",
          message: "keychain unavailable",
          retryable: false,
          severity: "error",
        },
      },
      {
        target: "codex",
        error: {
          code: "codex_failed",
          message: "device auth missing",
          retryable: false,
          severity: "error",
        },
      },
    ]);

    const useAppBootstrap = await loadUseAppBootstrap();
    const { bootstrapCritical } = useAppBootstrap();

    await bootstrapCritical();

    expect(notificationStoreMock.addNotice).toHaveBeenNthCalledWith(
      1,
      "error",
      expect.stringContaining("keychain unavailable"),
      expect.objectContaining({
        code: "providers_failed",
        operation: "startup-auth-providers",
        sticky: true,
        replaceOperation: true,
      }),
    );
    expect(notificationStoreMock.addNotice).toHaveBeenNthCalledWith(
      2,
      "error",
      expect.stringContaining("device auth missing"),
      expect.objectContaining({
        code: "codex_failed",
        operation: "startup-auth-codex",
        sticky: true,
        replaceOperation: true,
      }),
    );
  });

  it("treats missing auth failure results as an empty list", async () => {
    authStoreMock.checkAuth.mockResolvedValue(undefined);

    const useAppBootstrap = await loadUseAppBootstrap();
    const { bootstrapCritical } = useAppBootstrap();

    await expect(bootstrapCritical()).resolves.toBeUndefined();
    expect(notificationStoreMock.addNotice).not.toHaveBeenCalled();
  });

  it("loads the global tool permission mode before auth unlocks the main shell", async () => {
    const useAppBootstrap = await loadUseAppBootstrap();
    const { bootstrapCritical } = useAppBootstrap();

    await bootstrapCritical();

    expect(chatStoreMock.loadToolPermissionMode).toHaveBeenCalledTimes(1);
    expect(modelStoreMock.loadLastEffort).toHaveBeenCalledTimes(1);
    expect(
      agentStoreMock.loadAgents.mock.invocationCallOrder[0],
    ).toBeLessThan(modelStoreMock.loadLastEffort.mock.invocationCallOrder[0]);
    expect(
      chatStoreMock.loadToolPermissionMode.mock.invocationCallOrder[0],
    ).toBeLessThan(authStoreMock.checkAuth.mock.invocationCallOrder[0]);
  });

  it("auto-opens the lexical progress window only once per rebuild run", async () => {
    projectStoreMock.workingDir = "F:/Project";

    const eventModule = await import("@tauri-apps/api/event");
    const knowledgeModule = await import("../services/knowledge");
    const progressWindowModule =
      await import("../services/knowledgeLexicalProgressWindow");
    const handlers = new Map<string, (event: { payload: any }) => void>();

    (
      eventModule.listen as unknown as ReturnType<typeof vi.fn>
    ).mockImplementation(
      async (name: string, handler: (event: { payload: any }) => void) => {
        handlers.set(name, handler);
        return vi.fn();
      },
    );
    (
      progressWindowModule.shouldAutoOpenKnowledgeLexicalProgressWindow as unknown as ReturnType<
        typeof vi.fn
      >
    ).mockReturnValue(true);
    (
      progressWindowModule.getKnowledgeLexicalProgressRunKey as unknown as ReturnType<
        typeof vi.fn
      >
    ).mockImplementation(
      (
        status:
          | { running?: boolean; startedAt?: string | null }
          | null
          | undefined,
      ) => (status?.running ? (status.startedAt ?? "active") : ""),
    );

    const useAppBootstrap = await loadUseAppBootstrap();
    const { registerListeners, cleanup } = useAppBootstrap();
    await registerListeners();

    const lexicalStatusHandler = handlers.get("knowledge-lexical-rebuild-status");
    expect(lexicalStatusHandler).toBeTypeOf("function");
    expect(
      knowledgeModule.knowledgeGetLexicalRebuildStatus,
    ).not.toHaveBeenCalled();

    lexicalStatusHandler?.({
      payload: {
        running: true,
        stage: "preparing",
        detail: "Preparing docs",
        currentFile: null,
        processedDocs: 24,
        totalDocs: 4096,
        error: null,
        startedAt: "2026-04-16T00:00:00Z",
        completedAt: null,
      },
    });
    await Promise.resolve();
    expect(
      progressWindowModule.openKnowledgeLexicalProgressWindow,
    ).toHaveBeenCalledTimes(1);

    lexicalStatusHandler?.({
      payload: {
        running: true,
        stage: "committing",
        detail: "Committing docs",
        currentFile: null,
        processedDocs: 4096,
        totalDocs: 4096,
        error: null,
        startedAt: "2026-04-16T00:00:00Z",
        completedAt: null,
      },
    });
    await Promise.resolve();
    expect(
      progressWindowModule.openKnowledgeLexicalProgressWindow,
    ).toHaveBeenCalledTimes(1);

    cleanup();
  });

  it("dispatches system notifications only after the chat store accepts a stream event", async () => {
    const eventModule = await import("@tauri-apps/api/event");
    const listenMock = eventModule.listen as unknown as ReturnType<typeof vi.fn>;
    const handlers = new Map<string, (event: { payload: any }) => void>();

    listenMock.mockImplementation(
      async (name: string, handler: (event: { payload: any }) => void) => {
        handlers.set(name, handler);
        return vi.fn();
      },
    );

    chatStoreMock.sessions = [{ id: "session-1", title: "Session A" }];

    const useAppBootstrap = await loadUseAppBootstrap();
    const { registerListeners } = useAppBootstrap();
    await registerListeners();

    const streamHandler = handlers.get("stream-event");
    expect(streamHandler).toBeTypeOf("function");

    streamHandler?.({
      payload: {
        type: "done",
        runId: "run-1",
        sessionId: "session-1",
        messageId: "message-1",
        fullText: "Completed response",
      },
    });

    expect(chatStoreMock.handleStreamEvent).toHaveBeenCalledTimes(1);
    expect(maybeNotifyStreamEventMock).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "done",
        runId: "run-1",
        sessionId: "session-1",
      }),
      { sessionTitle: "Session A" },
    );

    maybeNotifyStreamEventMock.mockClear();
    chatStoreMock.sessions = [
      { id: "session-1", title: "Session A" },
      {
        id: "session-child-1",
        title: "Explorer",
        parentSessionId: "session-1",
      },
    ];

    streamHandler?.({
      payload: {
        type: "done",
        runId: "run-child-1",
        sessionId: "session-child-1",
        messageId: "message-child-1",
        fullText: "Child response",
      },
    });

    expect(maybeNotifyStreamEventMock).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "done",
        runId: "run-child-1",
        sessionId: "session-child-1",
      }),
      { sessionTitle: "Explorer", isSubagent: true },
    );

    maybeNotifyStreamEventMock.mockClear();
    chatStoreMock.handleStreamEvent.mockReturnValue(false);

    streamHandler?.({
      payload: {
        type: "error",
        runId: "run-2",
        sessionId: "session-1",
        error: {
          code: "failed",
          message: "nope",
          retryable: false,
          severity: "error",
        },
      },
    });

    expect(maybeNotifyStreamEventMock).not.toHaveBeenCalled();
  });
});
