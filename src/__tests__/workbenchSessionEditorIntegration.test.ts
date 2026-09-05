// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceRef } from "../services/project";
import { saveSessionExecutionState } from "../services/session";
import { broadcastSessionExecutionState } from "../services/sessionExecutionState";
import type { WorkbenchEditorInput } from "../types/workbench";

const compact = vi.fn();
const resumeInterrupted = vi.fn();
const setPlanMode = vi.fn();
const forkFromMessage = vi.fn();
const forkSession = vi.fn();
const loadOlderHistory = vi.fn();
const loadSessionTurnPreview = vi.fn();
const loadSessionHistoryThroughMessage = vi.fn();
const addNotice = vi.fn();
const changesState = reactive({ panelVisible: false });

const controller = {
  inputText: ref(""),
  restoredComposerDraft: ref(null),
  clearRestoredComposerDraft: vi.fn(),
  messages: ref([]),
  streamingText: ref(""),
  thinkingText: ref(""),
  thinkingStream: ref(null),
  streamingTextOrder: ref(null),
  thinkingOrder: ref(null),
  liveRenderParts: ref([]),
  livePartStreams: ref(new Map()),
  isStreaming: ref(false),
  isCancelling: ref(false),
  isCompacting: ref(false),
  compactQueued: ref(false),
  isThinking: ref(false),
  hasThinking: ref(true),
  thinkingDuration: ref(1200),
  activeToolCalls: ref([]),
  tokenUsage: ref({ inputTokens: 10, outputTokens: 5, totalTokens: 15 }),
  undoableMessageIds: ref(new Set<string>()),
  pendingQuestion: ref(null),
  pendingToolConfirms: ref([]),
  queuedFollowUp: ref(null),
  errorMessage: ref(null),
  sessionId: ref<string | null>("session-source"),
  currentRunId: ref<string | null>("run-pane"),
  sessionAgentId: ref<string | null>("agent-a"),
  sessionModelId: ref<string | null>("model-a"),
  sessionEffort: ref("high"),
  sessionFastMode: ref(true),
  sessionMultiAgentEnabled: ref(false),
  parentSessionId: ref<string | null>("parent-session"),
  latestCompletedRunId: ref<string | null>("run-complete"),
  planModeActive: ref(true),
  canResumeInterrupted: ref(true),
  sessionHistoryHasMore: ref(true),
  sessionHistoryLoading: ref(true),
  sessionUserMessageIds: ref(["user-old", "user-new"]),
  setExecutionSelection: vi.fn(),
  sendComposerPayload: vi.fn(),
  compact,
  resumeInterrupted,
  insertQueuedFollowUp: vi.fn(),
  deleteQueuedFollowUp: vi.fn(),
  reEditQueuedFollowUp: vi.fn(),
  cancel: vi.fn(),
  setPlanMode,
  exitPlanMode: vi.fn(),
  loadOlderHistory,
  loadSessionHistoryThroughMessage,
  loadSessionTurnPreview,
  restoreComposerDraft: vi.fn(),
  forkFromMessage,
  forkSession,
  checkUndoConflicts: vi.fn(),
  checkUndoDirty: vi.fn(),
  performUndo: vi.fn(),
  rollbackConversation: vi.fn(),
  rollbackFilesAndConversation: vi.fn(),
  resetSession: vi.fn(),
  answerQuestion: vi.fn(),
  answerToolConfirm: vi.fn(),
  answerAllToolConfirms: vi.fn(),
  applyKnowledgeProposal: vi.fn(),
  ignoreKnowledgeProposal: vi.fn(),
};

let chatViewProps: Record<string, unknown> | null = null;
let thinkingPanelProps: Record<string, unknown> | null = null;
let sidebarProps: Record<string, unknown> | null = null;

vi.mock("../composables/useEmbeddedChatSession", () => ({
  useEmbeddedChatSession: () => controller,
}));
vi.mock("../composables/useSkills", () => ({
  useSkills: () => ({ skillItems: ref([]) }),
}));
vi.mock("../composables/useKnowledgeAccessMode", () => ({
  useKnowledgeAccessMode: () => ({ state: reactive({ mode: "full" }) }),
}));
vi.mock("../composables/useWorkspaceAssetDbStatus", () => ({
  useWorkspaceAssetDbStatus: () => ({
    scanPhase: ref("idle"),
    lastScanStats: ref(null),
    startScan: vi.fn(),
  }),
}));
vi.mock("../composables/useWorkspaceUnityStatus", () => ({
  useWorkspaceUnityStatus: () => ({
    connected: ref(false),
    connectionStatus: ref(null),
    pluginStatus: ref(null),
    pluginInstalling: ref(false),
    launching: ref(false),
    launchState: ref(null),
    launch: vi.fn(),
    installPlugin: vi.fn(),
  }),
}));
vi.mock("../services/session", () => ({ saveSessionExecutionState: vi.fn() }));
vi.mock("../services/sessionExecutionState", () => ({
  broadcastSessionExecutionState: vi.fn(),
}));
vi.mock("../stores/agent", () => ({
  useAgentStore: () => ({
    selectedAgentId: "agent-a",
    agents: [{
      id: "agent-a",
      name: "Agent A",
      isDefault: true,
      defaultEffort: "high",
    }, {
      id: "agent-child",
      name: "Child Agent",
      isDefault: false,
      defaultEffort: "low",
    }],
  }),
}));
vi.mock("../stores/auth", () => ({
  useAuthStore: () => ({ codexAuthenticated: true }),
}));
vi.mock("../stores/chatChanges", () => ({
  useChatChangesStore: () => ({
    sessionState: () => changesState,
    setActiveRunId: vi.fn(),
    setLatestCompletedRunId: vi.fn(),
    refresh: vi.fn(async () => undefined),
  }),
}));
vi.mock("../stores/model", () => ({
  useModelStore: () => ({
    selectedModelId: "model-a",
    effort: "high",
    effectiveCodexFastMode: true,
    availableModels: [{
      id: "model-a",
      name: "Model A",
      provider: "openai_codex",
      supportedEfforts: ["low", "high"],
    }, {
      id: "model-child",
      name: "Child Model",
      provider: "openai_codex",
      supportedEfforts: ["low", "high"],
    }],
    availableEfforts: ["low", "high"],
    codexTransport: "websocket",
    hasUserDefaultEffort: true,
    defaultEffort: "high",
  }),
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice }),
}));
vi.mock("../stores/workspaceContext", () => ({
  useWorkspaceContextStore: () => ({
    checkoutsById: {
      "checkout-pane": {
        checkoutId: "checkout-pane",
        root: "F:/project/pane",
        runtime: { detectedServices: [] },
      },
    },
  }),
}));
vi.mock("../i18n", () => ({ t: (key: string) => key }));

vi.mock("../components/ChatView.vue", () => ({
  default: defineComponent({
    name: "ChatViewStub",
    inheritAttrs: false,
    props: {
      scopedSession: Boolean,
      managedNativeDrops: Boolean,
      workspaceRef: Object,
      currentRunId: String,
      isViewingSubagent: Boolean,
      selectedAgentId: String,
      selectedModelId: String,
      effort: String,
      fastModeEnabled: Boolean,
      multiAgentEnabled: Boolean,
      sessionHistoryLoading: Boolean,
      sessionHistoryHasMore: Boolean,
      loadOlderHistory: Function,
      sessionUserMessageIds: Array,
      loadSessionTurnPreview: Function,
      loadSessionHistoryThroughMessage: Function,
      forkFromMessage: Function,
    },
    emits: [
      "compact",
      "fork",
      "resume",
      "requestPlanMode",
      "exportSessionContext",
      "reviewSessionContext",
      "openThinking",
      "selectMultiAgent",
    ],
    setup(props, { emit, expose }) {
      chatViewProps = props as unknown as Record<string, unknown>;
      expose({
        applyDraftPrefill: vi.fn(),
        appendComposerDraft: vi.fn(),
        exportComposerDraft: vi.fn(() => null),
        focusComposerInput: vi.fn(),
      });
      return () => h("div", { class: "chat-view-stub" }, [
        h("button", { class: "multi-agent", onClick: () => emit("selectMultiAgent", !props.multiAgentEnabled) }, "multi agent"),
        h("button", { class: "compact", onClick: () => emit("compact") }, "compact"),
        h("button", { class: "resume", onClick: () => emit("resume") }, "resume"),
        h("button", { class: "plan", onClick: () => emit("requestPlanMode", true) }, "plan"),
        h("button", { class: "fork-session", onClick: () => emit("fork") }, "fork session"),
        h("button", {
          class: "fork-message",
          onClick: () => (props.forkFromMessage as ((id: string) => unknown) | undefined)?.("message-1"),
        }, "fork message"),
        h("button", {
          class: "export",
          onClick: () => emit("exportSessionContext", { sessionId: "session-source" }),
        }, "export"),
        h("button", {
          class: "review",
          onClick: () => emit("reviewSessionContext", { sessionId: "session-source" }),
        }, "review"),
        h("button", {
          class: "thinking",
          onClick: () => emit("openThinking", "Scoped reasoning"),
        }, "thinking"),
      ]);
    },
  }),
}));
vi.mock("../components/ThinkingPanel.vue", () => ({
  default: defineComponent({
    name: "ThinkingPanelStub",
    inheritAttrs: false,
    props: { text: String, stream: Object, isThinking: Boolean },
    emits: ["close"],
    setup(props, { emit }) {
      thinkingPanelProps = props as unknown as Record<string, unknown>;
      return () => h("div", { class: "thinking-panel-stub" }, [
        h("span", props.text),
        h("button", { class: "close-thinking", onClick: () => emit("close") }, "close"),
      ]);
    },
  }),
}));
vi.mock("../components/ChatSidebarPanel.vue", () => ({
  default: defineComponent({
    name: "ChatSidebarPanelStub",
    inheritAttrs: false,
    props: { scopedSession: Boolean, workspaceRef: Object, sessionId: String },
    setup(props) {
      sidebarProps = props as unknown as Record<string, unknown>;
      return () => h("aside", { class: "chat-sidebar-stub" });
    },
  }),
}));

import WorkbenchSessionEditor from "../components/workbench/WorkbenchSessionEditor.vue";

const workspaceRef: WorkspaceRef = {
  checkoutId: "checkout-pane",
  expectedGeneration: 9,
};
const editor: WorkbenchEditorInput = {
  editorId: "editor-pane",
  resource: { kind: "session", projectId: "project-a", sessionId: "session-source" },
  title: "Source session",
  preview: false,
  pinned: true,
  dirty: false,
  capabilities: { split: true, detach: true, duplicate: true },
  checkoutBinding: workspaceRef,
  availability: "available",
};

async function flushUi() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

function mountEditor(listeners: Record<string, unknown> = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({
    render: () => h(WorkbenchSessionEditor, {
      editor,
      workspaceRef,
      ...listeners,
    } as never),
  });
  app.mount(host);
  return { app, host };
}

beforeEach(() => {
  vi.clearAllMocks();
  controller.sessionAgentId.value = "agent-a";
  controller.sessionModelId.value = "model-a";
  controller.sessionEffort.value = "high";
  controller.sessionFastMode.value = true;
  controller.sessionMultiAgentEnabled.value = false;
  changesState.panelVisible = false;
  chatViewProps = null;
  thinkingPanelProps = null;
  sidebarProps = null;
  forkFromMessage.mockResolvedValue("session-message-fork");
  forkSession.mockResolvedValue("session-whole-fork");
});

describe("WorkbenchSessionEditor scoped host", () => {
  it("persists multi agent independently of the selected effort", async () => {
    const { app, host } = mountEditor();
    try {
      await flushUi();
      host.querySelector<HTMLButtonElement>(".multi-agent")!.click();
      await flushUi();
      expect(chatViewProps).toMatchObject({ effort: "high", multiAgentEnabled: true });
      expect(saveSessionExecutionState).toHaveBeenLastCalledWith("session-source", "model-a", "high", true, true);
      expect(broadcastSessionExecutionState).toHaveBeenLastCalledWith(expect.objectContaining({ sessionId: "session-source", multiAgentEnabled: true }));
      host.querySelector<HTMLButtonElement>(".multi-agent")!.click();
      await flushUi();
      expect(saveSessionExecutionState).toHaveBeenLastCalledWith("session-source", "model-a", "high", true, false);
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it.each([false, true])("restores child execution settings with delayed hydration: %s", async (delayed) => {
    function hydrateChild() {
      controller.sessionAgentId.value = "agent-child";
      controller.sessionModelId.value = "model-child";
      controller.sessionEffort.value = "low";
      controller.sessionFastMode.value = false;
      controller.sessionMultiAgentEnabled.value = true;
    }
    if (!delayed) hydrateChild();
    const { app, host } = mountEditor();
    try {
      await flushUi();
      if (delayed) {
        hydrateChild();
        await flushUi();
      }

      expect(chatViewProps).toMatchObject({
        isViewingSubagent: true,
        selectedAgentId: "agent-child",
        selectedModelId: "model-child",
        effort: "low",
        fastModeEnabled: false,
        multiAgentEnabled: true,
      });
      expect(controller.setExecutionSelection).toHaveBeenLastCalledWith({
        agentId: "agent-child",
        modelId: "model-child",
        effort: "low",
        fastMode: false,
        multiAgentEnabled: true,
      });
      expect(saveSessionExecutionState).not.toHaveBeenCalled();
      expect(broadcastSessionExecutionState).not.toHaveBeenCalled();
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("binds the pane controller and forwards session host actions", async () => {
    const sessionForked = vi.fn();
    const exportSessionContext = vi.fn();
    const reviewSessionContext = vi.fn();
    const { app, host } = mountEditor({
      onSessionForked: sessionForked,
      onExportSessionContext: exportSessionContext,
      onReviewSessionContext: reviewSessionContext,
    });
    await flushUi();

    expect(chatViewProps).toMatchObject({
      scopedSession: true,
      managedNativeDrops: true,
      workspaceRef,
      currentRunId: "run-pane",
      isViewingSubagent: true,
      sessionHistoryLoading: true,
      sessionHistoryHasMore: true,
      sessionUserMessageIds: ["user-old", "user-new"],
    });
    expect(chatViewProps?.loadOlderHistory).toBe(loadOlderHistory);
    expect(chatViewProps?.loadSessionTurnPreview).toBe(loadSessionTurnPreview);
    expect(chatViewProps?.loadSessionHistoryThroughMessage).toBe(loadSessionHistoryThroughMessage);

    host.querySelector<HTMLButtonElement>(".compact")?.click();
    host.querySelector<HTMLButtonElement>(".resume")?.click();
    host.querySelector<HTMLButtonElement>(".plan")?.click();
    await flushUi();
    expect(compact).toHaveBeenCalledOnce();
    expect(resumeInterrupted).toHaveBeenCalledOnce();
    expect(setPlanMode).toHaveBeenCalledWith(true);

    host.querySelector<HTMLButtonElement>(".fork-message")?.click();
    await flushUi();
    expect(forkFromMessage).toHaveBeenCalledWith("message-1", "chat.session.forkTitle");
    expect(sessionForked).toHaveBeenLastCalledWith({
      editorId: "editor-pane",
      sourceSessionId: "session-source",
      forkedSessionId: "session-message-fork",
    });

    host.querySelector<HTMLButtonElement>(".fork-session")?.click();
    await flushUi();
    expect(forkSession).toHaveBeenCalledWith("chat.session.forkTitle");
    expect(sessionForked).toHaveBeenLastCalledWith({
      editorId: "editor-pane",
      sourceSessionId: "session-source",
      forkedSessionId: "session-whole-fork",
    });

    host.querySelector<HTMLButtonElement>(".export")?.click();
    host.querySelector<HTMLButtonElement>(".review")?.click();
    await flushUi();
    expect(exportSessionContext).toHaveBeenCalledWith({
      editorId: "editor-pane",
      request: { sessionId: "session-source" },
    });
    expect(reviewSessionContext).toHaveBeenCalledWith({
      editorId: "editor-pane",
      request: { sessionId: "session-source" },
    });

    app.unmount();
    host.remove();
  });

  it("opens pane-owned thinking and Changes panels with scoped state", async () => {
    const { app, host } = mountEditor();
    await flushUi();

    host.querySelector<HTMLButtonElement>(".thinking")?.click();
    await flushUi();
    expect(host.querySelector(".thinking-panel-stub")).not.toBeNull();
    expect(thinkingPanelProps).toMatchObject({
      text: "Scoped reasoning",
      isThinking: false,
    });

    host.querySelector<HTMLButtonElement>(".close-thinking")?.click();
    await flushUi();
    expect(host.querySelector(".thinking-panel-stub")).toBeNull();

    changesState.panelVisible = true;
    await flushUi();
    expect(host.querySelector(".chat-sidebar-stub")).not.toBeNull();
    expect(sidebarProps).toMatchObject({
      scopedSession: true,
      workspaceRef,
      sessionId: "session-source",
    });

    app.unmount();
    host.remove();
  });
});
