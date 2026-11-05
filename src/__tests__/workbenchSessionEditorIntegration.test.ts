// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive, ref } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceRef } from "../services/project";
import { saveSessionExecutionState, unarchiveSession } from "../services/session";
import { broadcastSessionExecutionState } from "../services/sessionExecutionState";
import type { WorkbenchEditorInput } from "../types/workbench";
import { useWorkspaceNavigationMode } from "../composables/useWorkspaceNavigationMode";
import { useNewSessionSettings } from "../composables/useNewSessionSettings";
import { useSurfaceContentReadiness } from "../composables/surfaceContentReadiness";
import { clampEffortToLevels } from "../stores/newSessionSelection";
import { createDocumentSelectionHub, DOCUMENT_SELECTION_HUB_KEY } from "../composables/documentSelection";
import type { EffortLevel } from "../types";

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
const agentStore = reactive({
  selectedAgentId: "agent-a",
  workspaceCheckoutId: "checkout-pane",
  agents: [
    { id: "agent-a", name: "Agent A", isDefault: true, defaultEffort: "high" },
    { id: "agent-child", name: "Child Agent", isDefault: false, defaultEffort: "low" },
  ],
});

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
  isLoading: ref(false),
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
let requestedSession: { value: string | null } | null = null;
let beforeSessionLaunch: ((sessionId: string) => Promise<void>) | undefined;
let holdChatContentOnMount = false;
let releaseChatContent: (() => void) | null = null;

vi.mock("../composables/useEmbeddedChatSession", () => ({
  useEmbeddedChatSession: (options: { initialSessionId: { value: string | null }; beforeSessionLaunch: (sessionId: string) => Promise<void> }) => {
    requestedSession = options.initialSessionId;
    beforeSessionLaunch = options.beforeSessionLaunch;
    return controller;
  },
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
vi.mock("../services/session", () => ({ saveSessionExecutionState: vi.fn(), unarchiveSession: vi.fn() }));
const selectionMemoryService = vi.hoisted(() => ({
  getWorkspaceSelection: vi.fn(),
  saveWorkspaceSelection: vi.fn(),
}));
vi.mock("../services/selectionMemory", () => selectionMemoryService);
vi.mock("../services/sessionExecutionState", () => ({
  broadcastSessionExecutionState: vi.fn(),
}));
vi.mock("../stores/agent", () => ({
  useAgentStore: () => agentStore,
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
const modelStoreState = reactive({
  modelDefaults: { mainModel: "" },
  agentModelPreferences: {} as Record<string, { modelId: string; effort: string }>,
  lastModelId: "",
  hasUserDefaultEffort: true,
  defaultEffort: "high" as EffortLevel,
});
// Claude models accept "none" (thinking off) as a regular level.
const effortLevelsFor = (modelId: string): EffortLevel[] => (
  modelId.startsWith("claude_code/")
    ? ["none", "low", "high"]
    : modelId.startsWith("model-") ? ["low", "high"] : []
);
vi.mock("../stores/model", () => ({
  useModelStore: () => ({
    get modelDefaults() { return modelStoreState.modelDefaults; },
    get agentModelPreferences() { return modelStoreState.agentModelPreferences; },
    get lastModelId() { return modelStoreState.lastModelId; },
    defaultCodexFastMode: true,
    effortsForModel: (modelId: string) => effortLevelsFor(modelId),
    clampEffortForModel: (modelId: string, level: EffortLevel) => clampEffortToLevels(effortLevelsFor(modelId), level),
    noteRememberedSelection: (_agentId: string, modelId: string) => { modelStoreState.lastModelId = modelId; },
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
    }, {
      id: "claude_code/claude-cli",
      name: "Claude CLI",
      provider: "claude_code",
      supportedEfforts: ["none", "low", "high"],
    }],
    availableEfforts: ["low", "high"],
    codexTransport: "websocket",
    get hasUserDefaultEffort() { return modelStoreState.hasUserDefaultEffort; },
    get defaultEffort() { return modelStoreState.defaultEffort; },
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
      liveSelectionQuote: Object,
      presented: { type: Boolean, default: undefined },
    },
    emits: [
      "newChat",
      "compact",
      "fork",
      "resume",
      "requestPlanMode",
      "exportSessionContext",
      "reviewSessionContext",
      "openThinking",
      "selectMultiAgent",
      "selectAgent",
      "selectModel",
    ],
    setup(props, { emit, expose, slots }) {
      chatViewProps = props as unknown as Record<string, unknown>;
      if (holdChatContentOnMount) releaseChatContent = useSurfaceContentReadiness()?.hold() ?? null;
      expose({
        applyDraftPrefill: vi.fn(),
        appendComposerDraft: vi.fn(),
        exportComposerDraft: vi.fn(() => null),
        focusComposerInput: vi.fn(),
      });
      return () => h("div", { class: "chat-view-stub" }, [
        slots["session-picker"]?.(),
        h("button", { class: "new-chat", onClick: () => emit("newChat", { source: "control" }) }, "new chat"),
        h("button", { class: "multi-agent", onClick: () => emit("selectMultiAgent", !props.multiAgentEnabled) }, "multi agent"),
        h("button", { class: "select-child-agent", onClick: () => emit("selectAgent", "agent-child") }, "child agent"),
        h("button", { class: "select-child-model", onClick: () => emit("selectModel", "model-child") }, "child model"),
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
          class: "review-message",
          onClick: () => emit("reviewSessionContext", { sessionId: "session-source", messageId: "message-1" }),
        }, "review message"),
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
  setActivePinia(createPinia());
  modelStoreState.modelDefaults = { mainModel: "" };
  modelStoreState.agentModelPreferences = {};
  modelStoreState.lastModelId = "";
  modelStoreState.hasUserDefaultEffort = true;
  modelStoreState.defaultEffort = "high";
  selectionMemoryService.getWorkspaceSelection.mockResolvedValue({
    projectId: "project-a",
    memory: { agentId: "", modelId: "", effort: "", agents: {}, updatedAtMs: 0 },
    lastAgentId: "",
    workspaceDefaultAgentId: null,
  });
  selectionMemoryService.saveWorkspaceSelection.mockImplementation(async () => undefined);
  controller.setExecutionSelection.mockReset();
  controller.sessionId.value = "session-source";
  agentStore.selectedAgentId = "agent-a";
  agentStore.workspaceCheckoutId = "checkout-pane";
  agentStore.agents = [
    { id: "agent-a", name: "Agent A", isDefault: true, defaultEffort: "high" },
    { id: "agent-child", name: "Child Agent", isDefault: false, defaultEffort: "low" },
  ];
  useNewSessionSettings().reset();
  useWorkspaceNavigationMode().setMode(workspaceRef.checkoutId, "sessions");
  controller.isLoading.value = false;
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
  it("waits for the new Unity workspace catalog instead of pinning the previous Simple default", async () => {
    agentStore.workspaceCheckoutId = "previous-generic-workspace";
    agentStore.selectedAgentId = "simple";
    agentStore.agents = [
      { id: "simple", name: "Simple", isDefault: true, defaultEffort: "high" },
      { id: "unity", name: "Unity", isDefault: false, defaultEffort: "high" },
    ];
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    // Match the real embedded controller's reactive execution-selection echo.
    controller.setExecutionSelection.mockImplementation(({ agentId }: { agentId: string }) => {
      controller.sessionAgentId.value = agentId;
    });
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("");
      agentStore.agents = agentStore.agents.map(agent => ({ ...agent, isDefault: agent.id === "unity" }));
      agentStore.workspaceCheckoutId = workspaceRef.checkoutId;
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("unity");
      expect(controller.sessionAgentId.value).toBe("unity");
      expect(controller.setExecutionSelection).not.toHaveBeenCalledWith(expect.objectContaining({ agentId: "simple" }));
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("uses the configured Agent in new tabs instead of the global current selection", async () => {
    useNewSessionSettings().setDefaultAgentId("agent-child");
    controller.sessionAgentId.value = null;
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-child");
      expect(controller.setExecutionSelection).toHaveBeenLastCalledWith(
        expect.objectContaining({ agentId: "agent-child" }),
      );
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("preserves a recorded Agent when the new-session default changes", async () => {
    useNewSessionSettings().setDefaultAgentId("agent-child");
    const { app, host } = mountEditor();
    try {
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-a");
      useNewSessionSettings().setDefaultAgentId("unavailable");
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-a");
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("uses the configured Agent when starting a new session in the current tab", async () => {
    useNewSessionSettings().setDefaultAgentId("agent-child");
    const onNewSessionRequested = vi.fn();
    const { app, host } = mountEditor({ onNewSessionRequested });
    try {
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-a");
      host.querySelector<HTMLButtonElement>(".new-chat")!.click();
      await flushUi();
      expect(controller.resetSession).toHaveBeenCalledOnce();
      expect(onNewSessionRequested).toHaveBeenCalledWith({ editorId: editor.editorId, source: "control" });
      expect(chatViewProps?.selectedAgentId).toBe("agent-child");
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("remembers a manual model switch for the next new tab in the workspace", async () => {
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    controller.sessionModelId.value = null;
    const newTab = { ...editor, resource: { kind: "newSession" as const, projectId: "project-a" } };
    const first = mountEditor({ editor: newTab });
    try {
      await flushUi();
      expect(chatViewProps?.selectedModelId).toBe("model-a");
      first.host.querySelector<HTMLButtonElement>(".select-child-model")!.click();
      await flushUi();
      expect(chatViewProps?.selectedModelId).toBe("model-child");
      expect(selectionMemoryService.saveWorkspaceSelection).toHaveBeenCalledWith(workspaceRef, {
        agentId: "agent-a",
        modelId: "model-child",
        effort: "high",
        fastMode: true,
      });
    } finally {
      first.app.unmount();
      first.host.remove();
    }

    const second = mountEditor({ editor: { ...newTab, editorId: "editor-second" } });
    try {
      await flushUi();
      expect(chatViewProps?.selectedModelId).toBe("model-child");
    } finally {
      second.app.unmount();
      second.host.remove();
    }
  });

  it("switches a new tab to the model the chosen Agent last used in this workspace", async () => {
    selectionMemoryService.getWorkspaceSelection.mockResolvedValue({
      projectId: "project-a",
      memory: {
        agentId: "agent-a",
        modelId: "model-a",
        effort: "high",
        agents: { "agent-child": { modelId: "model-child", effort: "low" } },
        updatedAtMs: 1,
      },
      lastAgentId: "agent-a",
      workspaceDefaultAgentId: null,
    });
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    controller.sessionModelId.value = null;
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-a");
      expect(chatViewProps?.selectedModelId).toBe("model-a");
      host.querySelector<HTMLButtonElement>(".select-child-agent")!.click();
      await flushUi();
      expect(chatViewProps?.selectedAgentId).toBe("agent-child");
      expect(chatViewProps?.selectedModelId).toBe("model-child");
      expect(chatViewProps?.effort).toBe("low");
      expect(selectionMemoryService.saveWorkspaceSelection).toHaveBeenLastCalledWith(workspaceRef, {
        agentId: "agent-child",
        modelId: "model-child",
        effort: "low",
        fastMode: true,
      });
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("starts new tabs with a pinned main model instead of the remembered one", async () => {
    modelStoreState.modelDefaults = { mainModel: "model-child" };
    selectionMemoryService.getWorkspaceSelection.mockResolvedValue({
      projectId: "project-a",
      memory: { agentId: "agent-a", modelId: "model-a", effort: "high", agents: {}, updatedAtMs: 1 },
      lastAgentId: "agent-a",
      workspaceDefaultAgentId: null,
    });
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    controller.sessionModelId.value = null;
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.selectedModelId).toBe("model-child");
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("restores a Claude gateway model without retaining the previous API selection", async () => {
    const { app, host } = mountEditor();
    try {
      controller.sessionModelId.value = "claude_code/vendor/model-not-in-catalog";
      await flushUi();
      expect(chatViewProps?.selectedModelId).toBe("claude_code/vendor/model-not-in-catalog");
    } finally {
      app.unmount();
      host.remove();
    }
  });
  it("shows the composer session picker only in file mode and identifies the originating tab", async () => {
    Object.defineProperty(Element.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
    const selected = vi.fn();
    const { app, host } = mountEditor({
      sessions: [{ id: "session-target", title: "Target", updatedAt: 1, sessionType: "chat" }],
      onSessionSelected: selected,
    });
    try {
      expect(host.querySelector(".session-composer-trigger")).toBeNull();
      useWorkspaceNavigationMode().setMode(workspaceRef.checkoutId, "files");
      await flushUi();
      host.querySelector<HTMLButtonElement>(".session-composer-trigger")!.click();
      await flushUi();
      document.querySelector<HTMLButtonElement>(".session-composer-option")!.click();
      expect(selected).toHaveBeenCalledExactlyOnceWith({ editorId: "editor-pane", sessionId: "session-target" });
      useWorkspaceNavigationMode().setMode(workspaceRef.checkoutId, "sessions");
      await flushUi();
      expect(host.querySelector(".session-composer-trigger")).toBeNull();
      expect(document.querySelector(".session-composer-panel")).toBeNull();
    } finally {
      app.unmount();
      host.remove();
      Reflect.deleteProperty(Element.prototype, "scrollIntoView");
    }
  });

  it("does not pin a new session's selection resolved before startup state loads", async () => {
    // The main window mounts restored tabs once auth is checked, before the
    // workspace Agent catalog, workspace memory and saved default effort load.
    modelStoreState.hasUserDefaultEffort = false;
    modelStoreState.lastModelId = "claude_code/claude-cli";
    agentStore.workspaceCheckoutId = "previous-workspace";
    agentStore.agents = [{ id: "agent-a", name: "Agent A", isDefault: true }] as never;
    let resolveSnapshot!: (value: unknown) => void;
    selectionMemoryService.getWorkspaceSelection.mockReturnValue(new Promise((resolve) => { resolveSnapshot = resolve; }));
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    controller.sessionModelId.value = null;
    controller.sessionEffort.value = null as never;
    controller.sessionFastMode.value = null as never;
    // Match the real embedded controller's reactive execution-selection echo.
    controller.setExecutionSelection.mockImplementation((selection: Record<string, unknown>) => {
      controller.sessionAgentId.value = (selection.agentId as string) || null;
      controller.sessionModelId.value = (selection.modelId as string) || null;
      controller.sessionEffort.value = (selection.effort as never) ?? null;
      controller.sessionFastMode.value = (selection.fastMode as never) ?? null;
    });
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.effort).toBe("none");
      agentStore.workspaceCheckoutId = workspaceRef.checkoutId;
      modelStoreState.hasUserDefaultEffort = true;
      resolveSnapshot({
        projectId: "project-a",
        memory: {
          agentId: "agent-a",
          modelId: "claude_code/claude-cli",
          effort: "low",
          agents: { "agent-a": { modelId: "claude_code/claude-cli", effort: "low" } },
          updatedAtMs: 1,
        },
        lastAgentId: "agent-a",
        workspaceDefaultAgentId: null,
      });
      await flushUi();
      await flushUi();
      expect(chatViewProps).toMatchObject({
        selectedAgentId: "agent-a",
        selectedModelId: "claude_code/claude-cli",
        effort: "low",
      });
      expect(controller.setExecutionSelection).toHaveBeenLastCalledWith(
        expect.objectContaining({ agentId: "agent-a", modelId: "claude_code/claude-cli", effort: "low" }),
      );
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("keeps a launched session's effort when the workspace default changes", async () => {
    modelStoreState.lastModelId = "claude_code/claude-cli";
    controller.sessionId.value = null;
    controller.sessionAgentId.value = null;
    controller.sessionModelId.value = null;
    controller.sessionEffort.value = null as never;
    controller.setExecutionSelection.mockImplementation((selection: Record<string, unknown>) => {
      controller.sessionAgentId.value = (selection.agentId as string) || null;
      controller.sessionModelId.value = (selection.modelId as string) || null;
      controller.sessionEffort.value = (selection.effort as never) ?? null;
    });
    const { app, host } = mountEditor({
      editor: { ...editor, resource: { kind: "newSession", projectId: "project-a" } },
    });
    try {
      await flushUi();
      expect(chatViewProps?.effort).toBe("high");
      // Launching binds the session id; its execution state is the tab's selection.
      controller.sessionId.value = "session-launched";
      await flushUi();
      modelStoreState.defaultEffort = "low";
      await flushUi();
      expect(chatViewProps?.effort).toBe("high");
    } finally {
      app.unmount();
      host.remove();
    }
  });

  it("offers the live document selection to existing and new conversations", async () => {
    const resources = [
      editor.resource,
      { kind: "newSession" as const, projectId: "project-a" },
    ];
    for (const resource of resources) {
      const hub = createDocumentSelectionHub();
      const host = document.createElement("div");
      document.body.appendChild(host);
      const app = createApp({
        render: () => h(WorkbenchSessionEditor, { editor: { ...editor, resource }, workspaceRef } as never),
      });
      app.provide(DOCUMENT_SELECTION_HUB_KEY, hub);
      app.mount(host);
      try {
        await flushUi();
        expect(chatViewProps?.liveSelectionQuote).toBeNull();
        hub.report({
          id: "file:1:1",
          ownerId: "file:1",
          checkoutId: workspaceRef.checkoutId,
          filePath: "F:/project/notes.md",
          name: "notes.md",
          location: "notes.md:3-4",
          content: "F:/project/notes.md:3-4\n```markdown\nselected\n```",
        });
        await flushUi();
        expect(chatViewProps?.liveSelectionQuote).toMatchObject({
          id: "file:1:1",
          path: "F:/project/notes.md",
          source: "file",
          location: "notes.md:3-4",
        });
        hub.clear("file:1");
        await flushUi();
        expect(chatViewProps?.liveSelectionQuote).toBeNull();
      } finally {
        app.unmount();
        host.remove();
      }
    }
  });

  it("loads archived sessions through the same controller and unarchives before continuing", async () => {
    const restore = vi.fn();
    const archivedEditor = reactive({
      ...editor,
      resource: { kind: "section", section: "archived", projectId: "project-a", sessionId: "session-source" },
    } as WorkbenchEditorInput);
    const { app, host } = mountEditor({ editor: archivedEditor, onSessionUnarchived: restore });
    await flushUi();
    expect(requestedSession?.value).toBe("session-source");
    expect(chatViewProps?.scopedSession).toBe(true);
    expect(chatViewProps?.loadOlderHistory).toBe(loadOlderHistory);
    expect(chatViewProps?.loadSessionHistoryThroughMessage).toBe(loadSessionHistoryThroughMessage);
    await beforeSessionLaunch?.("session-source");
    expect(unarchiveSession).toHaveBeenCalledWith("session-source");
    expect(restore).toHaveBeenCalledWith("session-source");
    archivedEditor.resource = { kind: "session", projectId: "project-a", sessionId: "session-source" };
    await flushUi();
    expect(requestedSession?.value).toBe("session-source");
    vi.mocked(unarchiveSession).mockClear();
    await beforeSessionLaunch?.("session-source");
    expect(unarchiveSession).not.toHaveBeenCalled();
    app.unmount();
    host.remove();
  });
  it("reports readiness only after the requested session finishes loading", async () => {
    controller.isLoading.value = true;
    const ready = vi.fn();
    const { app } = mountEditor({ onReady: ready });
    try {
      await flushUi();
      expect(ready).not.toHaveBeenCalled();
      controller.isLoading.value = false;
      await flushUi();
      expect(ready).toHaveBeenCalledOnce();
    } finally {
      app.unmount();
    }
  });

  it("holds readiness while content inside the surface is still resolving", async () => {
    holdChatContentOnMount = true;
    const ready = vi.fn();
    const { app } = mountEditor({ onReady: ready });
    try {
      await flushUi();
      expect(releaseChatContent).toBeTypeOf("function");
      expect(ready).not.toHaveBeenCalled();
      releaseChatContent?.();
      await flushUi();
      expect(ready).toHaveBeenCalledOnce();
    } finally {
      holdChatContentOnMount = false;
      releaseChatContent = null;
      app.unmount();
    }
  });

  it("does not let stalled content block the session switch", async () => {
    vi.useFakeTimers();
    holdChatContentOnMount = true;
    const ready = vi.fn();
    const { app } = mountEditor({ onReady: ready });
    try {
      await flushUi();
      expect(ready).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(200);
      expect(ready).toHaveBeenCalledOnce();
    } finally {
      holdChatContentOnMount = false;
      releaseChatContent = null;
      app.unmount();
      vi.useRealTimers();
    }
  });

  it("passes the stack presentation state to the chat surface", async () => {
    const { app } = mountEditor({ presented: false });
    try {
      await flushUi();
      expect(chatViewProps?.presented).toBe(false);
    } finally {
      app.unmount();
    }
  });

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
    host.querySelector<HTMLButtonElement>(".review-message")?.click();
    await flushUi();
    expect(reviewSessionContext).toHaveBeenLastCalledWith({
      editorId: "editor-pane",
      request: { sessionId: "session-source", messageId: "message-1" },
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
