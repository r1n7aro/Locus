// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, onUnmounted, reactive, ref, type App, type PropType } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "../types";
import type { WorkspaceRef } from "../services/project";

const mocks = vi.hoisted(() => {
  const globalWorkspaceRef = { checkoutId: "global-checkout", expectedGeneration: 1 };
  return {
    globalWorkspaceRef,
    loadIndicatorState: vi.fn<(scope: WorkspaceRef) => Promise<string>>(),
    releaseIndicatorState: vi.fn(),
    chatStore: {
      activeSessionPlanMode: false,
      activeQueuedFollowUp: null,
      isCompactQueued: false,
      currentRunId: "global-run",
      sessionHistoryLoading: false,
      sessionHistoryHasMore: true,
      sessionUserMessageIds: [],
      insertActiveQueuedFollowUp: vi.fn(),
      deleteActiveQueuedFollowUp: vi.fn(),
      reEditActiveQueuedFollowUp: vi.fn(),
      applyKnowledgeProposal: vi.fn(),
      ignoreKnowledgeProposal: vi.fn(),
      undoLatestConversationTurn: vi.fn(async () => true),
      forkSessionFromMessage: vi.fn(async () => "global-fork"),
      checkUndoDirty: vi.fn(async () => []),
      rollbackToMessage: vi.fn(async () => true),
      performUndo: vi.fn(async () => true),
      getSessionScrollState: vi.fn(() => null),
      rememberSessionScrollState: vi.fn(),
      loadOlderSessionHistory: vi.fn(async () => true),
      setSessionPlanMode: vi.fn(async () => undefined),
      loadSessionTurnPreview: vi.fn(),
      loadSessionHistoryThroughMessage: vi.fn(),
    },
    uiStore: {
      pendingChatPrefill: null,
      isWindowResizing: false,
      isAssistantSidebarTransitioning: false,
      activeTab: "development",
      stageChatDraftPrefill: vi.fn(),
      clearPendingChatPrefill: vi.fn(),
    },
    projectStore: {
      requireWorkspaceRef: vi.fn(() => globalWorkspaceRef),
    },
    chatChangesStore: {
      inlineDiffPayload: null,
      inlineDiffLoading: false,
      inlineDiffError: "",
      inlineDiffRequestKey: "",
      hasAnyChanges: false,
      currentPanelVisible: false,
      closeInlineDiff: vi.fn(),
      togglePanel: vi.fn(),
      sessionState: vi.fn<(sessionId: string | null) => { panelVisible: boolean } | null>(() => null),
      inlineDiffStateForSession: vi.fn(() => null),
      hasChangesForSession: vi.fn<(sessionId: string | null) => boolean>(() => false),
      closeInlineDiffForSession: vi.fn(),
      togglePanelForSession: vi.fn(),
    },
    notificationStore: {
      addNotice: vi.fn(),
      clearByOperation: vi.fn(),
    },
    openFileExternal: vi.fn(async () => undefined),
    showInFolder: vi.fn(async () => undefined),
    selectUnityAsset: vi.fn(async () => undefined),
    selectUnitySceneObject: vi.fn(async () => undefined),
    openUnityAssetInspector: vi.fn(async () => undefined),
    openUnitySceneObjectInspector: vi.fn(async () => undefined),
    legacyOpenKnowledgeDocument: vi.fn(async () => undefined),
    legacyOpenKnowledgeDocumentInKnowledge: vi.fn(),
    knowledgeRevealTarget: vi.fn(async () => undefined),
    openLocusAssetInspectorWorkbenchTab: vi.fn(async () => true),
    displaySettings: {
      showTurnNavigationRail: false,
      showWelcomeSubtitle: false,
      showViewsInSessionPanel: false,
      planApprovalTarget: "inline",
      assetRefClickAction: "fileBrowser",
      unityEmbedAssetRefClickAction: "fileBrowser",
    },
    sessionUndoSettings: {
      enabled: true,
      ready: true,
      busy: false,
    },
    loadSessionUndoSettings: vi.fn(async () => true),
  };
});

vi.mock("../stores/chat", () => ({ useChatStore: () => mocks.chatStore }));
vi.mock("../stores/ui", () => ({ useUiStore: () => mocks.uiStore }));
vi.mock("../stores/project", () => ({ useProjectStore: () => mocks.projectStore }));
vi.mock("../stores/chatChanges", () => ({ useChatChangesStore: () => mocks.chatChangesStore }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => mocks.notificationStore }));
vi.mock("../stores/workspaceContext", () => ({
  useWorkspaceContextStore: () => ({ checkoutsById: {} }),
}));

vi.mock("../services/unity", () => ({
  selectUnityAsset: mocks.selectUnityAsset,
  openUnityAssetInspector: mocks.openUnityAssetInspector,
  selectUnitySceneObject: mocks.selectUnitySceneObject,
  openUnitySceneObjectInspector: mocks.openUnitySceneObjectInspector,
  classifyUnitySceneObjectError: () => "unknown",
  openFileExternal: mocks.openFileExternal,
  showInFolder: mocks.showInFolder,
}));
vi.mock("../services/knowledge", () => ({
  knowledgeRevealTarget: mocks.knowledgeRevealTarget,
}));
vi.mock("../services/locusAssetInspector", () => ({
  openLocusAssetInspectorWorkbenchTab: mocks.openLocusAssetInspectorWorkbenchTab,
}));
vi.mock("../composables/useKnowledgeDocumentOpen", () => ({
  useKnowledgeDocumentOpen: () => ({
    openDocument: mocks.legacyOpenKnowledgeDocument,
    openInKnowledge: mocks.legacyOpenKnowledgeDocumentInKnowledge,
  }),
}));
vi.mock("../composables/useKnowledgeAccessMode", () => ({
  useKnowledgeAccessMode: () => ({ state: { mode: "full" }, setMode: vi.fn() }),
}));
vi.mock("../composables/useDisplaySettings", () => ({
  useDisplaySettings: () => ({
    state: mocks.displaySettings,
  }),
}));
vi.mock("../composables/useSessionUndoSettings", () => ({
  useSessionUndoSettings: () => ({
    state: mocks.sessionUndoSettings,
    load: mocks.loadSessionUndoSettings,
    setEnabled: vi.fn(),
  }),
}));
vi.mock("../composables/useChatInputSettings", () => ({
  getChatSubmitModifierLabel: () => "Ctrl+Enter",
  useChatInputSettings: () => ({
    state: { submitMode: "enter-send", runningSendMode: "queue" },
  }),
}));
vi.mock("../composables/useDiffProgress", () => ({
  useDiffProgress: () => ({ progress: { value: 0 }, reset: vi.fn() }),
}));
vi.mock("../composables/useInternalDrag", () => ({
  useInternalDragController: () => ({ start: vi.fn(() => false) }),
}));
vi.mock("../i18n", () => ({ t: (key: string) => key }));

vi.mock("../components/chat/ChatTranscript.vue", () => ({
  default: defineComponent({
    name: "ChatTranscriptStub",
    props: {
      messages: { type: Array, default: () => [] },
    },
    emits: [
      "scroll",
      "userScrollIntent",
      "contentClick",
      "contentContextmenu",
      "contentPointerdown",
      "openThinking",
    ],
    setup(props, { emit, expose }) {
      const scrollElement = ref<HTMLElement | null>(null);
      expose({
        getScrollElement: () => scrollElement.value,
        getContentElement: () => scrollElement.value,
      });
      return () => h("div", {
        ref: scrollElement,
        class: "test-chat-transcript",
        onScroll: () => emit("scroll"),
        onClick: (event: MouseEvent) => emit("contentClick", event),
        onContextmenu: (event: MouseEvent) => emit("contentContextmenu", event),
      }, [
        ...(props.messages as ChatMessage[]).map((message) => h("div", {
          class: "test-message",
          "data-chat-message-id": message.id,
        }, message.content)),
        h("span", {
          class: "md-file-ref test-file-ref",
          "data-file-path": "Assets/Scripts/Scoped.cs",
          "data-asset-path": "Assets/Scripts/Scoped.cs",
        }, "Scoped.cs"),
        h("span", {
          class: "md-knowledge-ref test-knowledge-ref",
          "data-knowledge-type": "design",
          "data-knowledge-path": "design/spec.md",
        }, "spec.md"),
        h("button", {
          class: "test-open-thinking",
          onClick: (event: MouseEvent) => {
            event.stopPropagation();
            emit("openThinking", "pane thinking");
          },
        }, "thinking"),
      ]);
    },
  }),
}));

vi.mock("../components/chat/RichChatInput.vue", () => ({
  default: defineComponent({
    name: "RichChatInputStub",
    emits: ["requestPlanMode", "requestNewSession", "undo", "compact", "resume"],
    setup(_props, { emit, expose }) {
      expose({
        resizeTextarea: vi.fn(),
        focus: vi.fn(),
        setSelectionRange: vi.fn(),
        isDraftEmpty: () => true,
        applyDraftPrefill: vi.fn(),
        appendDraft: vi.fn(),
        exportDraft: () => null,
        resetDraft: vi.fn(),
      });
      return () => h("div", { class: "test-rich-input" }, [
        h("button", {
          class: "test-request-plan",
          onClick: () => emit("requestPlanMode", true),
        }, "plan"),
        h("button", {
          class: "test-request-undo",
          onClick: () => emit("undo"),
        }, "undo"),
        h("button", {
          class: "test-request-compact",
          onClick: () => emit("compact"),
        }, "compact"),
        h("button", {
          class: "test-request-resume",
          onClick: () => emit("resume"),
        }, "resume"),
      ]);
    },
  }),
}));

vi.mock("../components/chat/ChatStatusIndicators.vue", () => ({
  default: defineComponent({
    name: "ChatStatusIndicatorsStub",
    props: { workspaceRef: Object as PropType<WorkspaceRef | null> },
    setup(props) {
      const binding = props.workspaceRef ? { ...props.workspaceRef } : null;
      const status = ref("loading");
      if (binding) void mocks.loadIndicatorState(binding).then(value => { status.value = value; });
      onUnmounted(() => mocks.releaseIndicatorState(binding));
      return () => h("div", { class: "test-status-indicators" }, status.value);
    },
  }),
}));
vi.mock("../components/ModelEffortSelector.vue", () => ({
  default: defineComponent({ name: "ModelEffortSelectorStub", setup: () => () => h("div") }),
}));
vi.mock("../components/chat/TokenUsageBar.vue", () => ({
  default: defineComponent({ name: "TokenUsageBarStub", setup: () => () => h("div") }),
}));
vi.mock("../components/chat/ChatTurnNavigationRail.vue", () => ({
  default: defineComponent({
    name: "ChatTurnNavigationRailStub",
    props: {
      userMessageIds: { type: Array, default: () => [] },
      loadPreview: { type: Function, default: undefined },
      loadTurn: { type: Function, default: undefined },
    },
    setup(props) {
      return () => h("button", {
        class: "test-turn-navigation",
        onClick: async () => {
          const messageId = (props.userMessageIds as string[])[0];
          if (!messageId) return;
          await (props.loadPreview as ((id: string) => Promise<unknown>) | undefined)?.(messageId);
          await (props.loadTurn as ((id: string) => Promise<unknown>) | undefined)?.(messageId);
        },
      }, "turn");
    },
  }),
}));

import ChatView from "../components/ChatView.vue";

const paneWorkspaceRef: WorkspaceRef = {
  checkoutId: "pane-checkout",
  expectedGeneration: 7,
};

const baseProps = {
  messages: [] as ChatMessage[],
  streamingText: "",
  isStreaming: false,
  isCompacting: false,
  isThinking: false,
  hasThinking: false,
  thinkingDuration: 0,
  activeToolCalls: [],
  agents: [],
  selectedAgentId: "",
  agentLocked: false,
  models: [],
  selectedModelId: "",
  effort: "medium",
  effortSupported: false,
  effortLevels: [],
  fastModeEnabled: false,
  fastModeAvailable: false,
  tokenUsage: { inputTokens: 0, outputTokens: 0, totalTokens: 0 },
  pendingQuestion: null,
  pendingToolConfirms: [],
  sessions: [],
  activeSessionId: "pane-session",
  workspaceRef: paneWorkspaceRef,
  workingDir: "F:/Pane",
  showSessionNavigation: false,
  scopedSession: true,
  shortcutActive: true,
};

let mountedApp: App<Element> | null = null;
let mountedHost: HTMLElement | null = null;

function mountChat(
  props: Record<string, unknown> = {},
  listeners: Record<string, unknown> = {},
) {
  mountedHost = document.createElement("div");
  document.body.appendChild(mountedHost);
  mountedApp = createApp({
    render: () => h(ChatView, { ...baseProps, ...props, ...listeners } as never),
  });
  mountedApp.use(createPinia());
  mountedApp.mount(mountedHost);
  return mountedHost;
}

async function flushUi() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

function contextMenuButton(label: string) {
  return Array.from(document.body.querySelectorAll<HTMLButtonElement>(".asset-ref-ctx-item"))
    .find((button) => button.textContent?.includes(label)) ?? null;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.loadIndicatorState.mockReset().mockResolvedValue("ready");
  mocks.chatChangesStore.sessionState.mockReturnValue(null);
  mocks.chatChangesStore.inlineDiffStateForSession.mockReturnValue(null);
  mocks.chatChangesStore.hasChangesForSession.mockReturnValue(false);
  mocks.sessionUndoSettings.enabled = true;
  mocks.sessionUndoSettings.ready = true;
  mocks.sessionUndoSettings.busy = false;
  mocks.displaySettings.showTurnNavigationRail = false;
  mocks.chatStore.sessionHistoryLoading = false;
  mocks.chatStore.sessionHistoryHasMore = true;
  mocks.uiStore.activeTab = "development";
});

afterEach(() => {
  mountedApp?.unmount();
  mountedHost?.remove();
  mountedApp = null;
  mountedHost = null;
  document.body.innerHTML = "";
});

describe("ChatView scoped access boundary", () => {
  it.each([false, true])("reviews the clicked message in its pane session while streaming=%s", async (isStreaming) => {
    const reviewSessionContext = vi.fn();
    const host = mountChat({
      isStreaming,
      messages: [{ id: "selected-message", role: "assistant", content: "Selected answer", createdAt: 1 }],
    }, { onReviewSessionContext: reviewSessionContext });
    await flushUi();
    host.querySelector(".test-message")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true, clientX: 10, clientY: 10,
    }));
    await flushUi();
    const review = contextMenuButton("chat.messageMenu.reviewMessage");
    expect(review).not.toBeNull();
    expect(review?.disabled).toBe(false);
    review?.click();
    await flushUi();
    expect(reviewSessionContext).toHaveBeenCalledExactlyOnceWith({
      sessionId: "pane-session", messageId: "selected-message",
    });
    expect(contextMenuButton("chat.messageMenu.reviewMessage")).toBeNull();
  });

  it.each([
    { checkoutId: "another-worktree", expectedGeneration: 7, expectedMaterializationEpoch: 1 },
    { checkoutId: "pane-checkout", expectedGeneration: 8, expectedMaterializationEpoch: 1 },
    { checkoutId: "pane-checkout", expectedGeneration: 7, expectedMaterializationEpoch: 2 },
  ])("clears the previous status immediately when the Worktree binding changes: %j", async (nextScope) => {
    const previous = { ...paneWorkspaceRef, expectedMaterializationEpoch: 1 };
    const props = reactive({ workspaceRef: previous });
    const host = mountChat(props);
    await flushUi();
    expect(host.querySelector(".test-status-indicators")?.textContent).toBe("ready");

    let finish!: (status: string) => void;
    mocks.loadIndicatorState.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    props.workspaceRef = nextScope;
    await flushUi();
    expect(host.querySelector(".test-status-indicators")?.textContent).toBe("loading");
    expect(mocks.releaseIndicatorState).toHaveBeenCalledExactlyOnceWith(previous);
    expect(mocks.loadIndicatorState).toHaveBeenLastCalledWith(nextScope);
    finish("new-worktree-idle");
    await flushUi();
    expect(host.querySelector(".test-status-indicators")?.textContent).toBe("new-worktree-idle");
  });

  it("ignores a late status response from the previously selected Worktree", async () => {
    let finishOld!: (status: string) => void;
    mocks.loadIndicatorState.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve; }));
    const props = reactive<{ workspaceRef: WorkspaceRef | null }>({ workspaceRef: paneWorkspaceRef });
    const host = mountChat(props);
    await flushUi();
    props.workspaceRef = { checkoutId: "new-worktree", expectedGeneration: 1 };
    await flushUi();
    finishOld("old-worktree-playing");
    await flushUi();
    expect(host.querySelector(".test-status-indicators")?.textContent).toBe("ready");

    props.workspaceRef = { ...props.workspaceRef };
    await flushUi();
    expect(mocks.loadIndicatorState).toHaveBeenCalledTimes(2);
    props.workspaceRef = null;
    await flushUi();
    expect(host.querySelector(".test-status-indicators")?.textContent).toBe("loading");
    expect(mocks.releaseIndicatorState).toHaveBeenCalledTimes(2);
  });

  it("routes re-edit and message fork through the pane controller", async () => {
    const undoConversation = vi.fn(async () => true);
    const restoreComposerDraft = vi.fn(async () => undefined);
    const forkFromMessage = vi.fn(async () => "pane-fork");
    const message: ChatMessage = {
      id: "user-1",
      role: "user",
      content: "Scoped prompt",
      createdAt: 1,
    };
    const host = mountChat({
      messages: [message],
      currentRunId: "pane-run",
      undoConversation,
      restoreComposerDraft,
      forkFromMessage,
    });
    await flushUi();

    host.querySelector(".test-message")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("chat.messageMenu.reEditUserMessage")?.click();
    await flushUi();

    expect(undoConversation).toHaveBeenCalledWith(null);
    expect(restoreComposerDraft).toHaveBeenCalledWith(expect.objectContaining({
      text: "Scoped prompt",
    }));
    expect(mocks.chatStore.undoLatestConversationTurn).not.toHaveBeenCalled();
    expect(mocks.uiStore.stageChatDraftPrefill).not.toHaveBeenCalled();

    host.querySelector(".test-message")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("chat.messageMenu.forkFromMessage")?.click();
    await flushUi();

    expect(forkFromMessage).toHaveBeenCalledWith("user-1");
    expect(mocks.chatStore.forkSessionFromMessage).not.toHaveBeenCalled();
  });

  it("fails closed when a scoped controller omits destructive message actions", async () => {
    const host = mountChat({
      messages: [{
        id: "user-1",
        role: "user",
        content: "Scoped prompt",
        createdAt: 1,
      }],
    });
    await flushUi();

    host.querySelector(".test-message")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();

    expect(contextMenuButton("chat.messageMenu.reEditUserMessage")?.disabled).toBe(true);
    expect(contextMenuButton("chat.messageMenu.forkFromMessage")?.disabled).toBe(true);
    expect(mocks.chatStore.undoLatestConversationTurn).not.toHaveBeenCalled();
    expect(mocks.chatStore.forkSessionFromMessage).not.toHaveBeenCalled();
  });

  it("keeps the scoped undo chooser keyboard-accessible and calls only the pane controller", async () => {
    const undoConversation = vi.fn(async () => true);
    const undoFilesAndConversation = vi.fn(async () => true);
    const restoreComposerDraft = vi.fn(async () => undefined);
    const checkUndoDirty = vi.fn(async () => []);
    const host = mountChat({
      messages: [
        {
          id: "user-undo",
          role: "user",
          content: "Restore this prompt",
          createdAt: 1,
        },
        {
          id: "assistant-undo",
          role: "assistant",
          content: "Changed files",
          createdAt: 2,
        },
      ],
      undoableMessageIds: new Set(["assistant-undo"]),
      undoConversation,
      undoFilesAndConversation,
      restoreComposerDraft,
      checkUndoDirty,
    });
    await flushUi();

    host.querySelector<HTMLButtonElement>(".test-request-undo")?.click();
    await flushUi();

    const backdrop = document.body.querySelector<HTMLElement>(".undo-chooser-backdrop");
    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    const choices = Array.from(document.body.querySelectorAll<HTMLButtonElement>(".undo-chooser-action"));
    expect(backdrop).not.toBeNull();
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(dialog?.getAttribute("aria-label")).toBe("chat.undo.dialogTitle");
    expect(choices).toHaveLength(2);
    expect(choices[1]?.getAttribute("aria-pressed")).toBe("true");

    backdrop?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "ArrowDown",
      bubbles: true,
      cancelable: true,
    }));
    await flushUi();
    expect(choices[0]?.getAttribute("aria-pressed")).toBe("true");

    backdrop?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    }));
    await flushUi();

    expect(undoConversation).toHaveBeenCalledWith(null);
    expect(undoFilesAndConversation).not.toHaveBeenCalled();
    expect(restoreComposerDraft).toHaveBeenCalledWith(expect.objectContaining({
      text: "Restore this prompt",
    }));
    expect(mocks.chatStore.undoLatestConversationTurn).not.toHaveBeenCalled();
  });

  it("routes plan, history, and thinking interactions to the scoped surface", async () => {
    const exitPlanMode = vi.fn(async () => undefined);
    const loadOlderHistory = vi.fn(async () => true);
    const requestPlanMode = vi.fn();
    const openThinking = vi.fn();
    const host = mountChat({
      planModeActive: true,
      exitPlanMode,
      sessionHistoryHasMore: true,
      sessionHistoryLoading: false,
      loadOlderHistory,
    }, {
      onRequestPlanMode: requestPlanMode,
      onOpenThinking: openThinking,
    });
    await flushUi();

    host.querySelector<HTMLButtonElement>(".plan-status-exit")?.click();
    host.querySelector<HTMLButtonElement>(".test-request-plan")?.click();
    host.querySelector<HTMLButtonElement>(".test-open-thinking")?.click();
    host.querySelector<HTMLElement>(".test-chat-transcript")?.dispatchEvent(new Event("scroll"));
    await flushUi();

    expect(exitPlanMode).toHaveBeenCalledTimes(1);
    expect(requestPlanMode).toHaveBeenCalledWith(true);
    expect(openThinking).toHaveBeenCalledWith("pane thinking");
    expect(loadOlderHistory).toHaveBeenCalledTimes(1);
    expect(mocks.chatStore.setSessionPlanMode).not.toHaveBeenCalled();
    expect(mocks.chatStore.loadOlderSessionHistory).not.toHaveBeenCalled();
    expect(mocks.chatStore.rememberSessionScrollState).not.toHaveBeenCalled();
  });

  it("forwards compact and resume actions to the scoped surface owner", async () => {
    const compact = vi.fn();
    const resume = vi.fn();
    const host = mountChat({}, {
      onCompact: compact,
      onResume: resume,
    });
    await flushUi();

    host.querySelector<HTMLButtonElement>(".test-request-compact")?.click();
    host.querySelector<HTMLButtonElement>(".test-request-resume")?.click();
    await flushUi();

    expect(compact).toHaveBeenCalledOnce();
    expect(resume).toHaveBeenCalledOnce();
  });

  it("routes turn navigation previews and history loading through the pane session", async () => {
    mocks.displaySettings.showTurnNavigationRail = true;
    const loadSessionTurnPreview = vi.fn(async () => ({
      messageId: "user-history",
      prompt: "Prompt",
      response: "Response",
    }));
    const loadSessionHistoryThroughMessage = vi.fn(async () => true);
    const host = mountChat({
      sessionUserMessageIds: ["user-history"],
      loadSessionTurnPreview,
      loadSessionHistoryThroughMessage,
    });
    await flushUi();

    host.querySelector<HTMLButtonElement>(".test-turn-navigation")?.click();
    await flushUi();
    await flushUi();

    expect(loadSessionTurnPreview).toHaveBeenCalledWith("user-history");
    expect(loadSessionHistoryThroughMessage).toHaveBeenCalledWith("user-history");
    expect(mocks.chatStore.loadSessionTurnPreview).not.toHaveBeenCalled();
    expect(mocks.chatStore.loadSessionHistoryThroughMessage).not.toHaveBeenCalled();
  });

  it("binds the Changes control to the pane session and keeps it keyboard-readable", async () => {
    mocks.sessionUndoSettings.enabled = false;
    mocks.chatChangesStore.sessionState.mockReturnValue({ panelVisible: false });
    mocks.chatChangesStore.hasChangesForSession.mockImplementation(
      (sessionId: string | null) => sessionId === "pane-session",
    );
    const host = mountChat();
    await flushUi();

    const toggle = host.querySelector<HTMLButtonElement>(".changes-toggle-btn");
    expect(toggle).not.toBeNull();
    expect(toggle?.disabled).toBe(false);
    expect(toggle?.getAttribute("aria-label")).toBe("chat.changes.toggle");

    toggle?.click();
    await flushUi();

    expect(mocks.chatChangesStore.hasChangesForSession).toHaveBeenCalledWith("pane-session");
    expect(mocks.chatChangesStore.togglePanelForSession).toHaveBeenCalledWith("pane-session");
    expect(mocks.chatChangesStore.togglePanel).not.toHaveBeenCalled();
  });

  it("shows the Changes control before the first change when file tracking is enabled", async () => {
    mocks.chatChangesStore.hasChangesForSession.mockReturnValue(false);
    const host = mountChat();
    await flushUi();

    expect(host.querySelector(".changes-toggle-btn")).not.toBeNull();
  });

  it("hides the empty Changes control when file tracking is disabled", async () => {
    mocks.sessionUndoSettings.enabled = false;
    mocks.chatChangesStore.hasChangesForSession.mockReturnValue(false);
    const host = mountChat();
    await flushUi();

    expect(host.querySelector(".changes-toggle-btn")).toBeNull();
  });

  it("uses the pane workspace for file, Unity, Inspector, and knowledge actions", async () => {
    const openKnowledgeDocument = vi.fn();
    const host = mountChat({ unityConnected: true }, {
      onOpenKnowledgeDocument: openKnowledgeDocument,
    });
    await flushUi();

    host.querySelector(".test-file-ref")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("common.openInEditor")?.click();
    await flushUi();
    expect(mocks.openFileExternal).toHaveBeenCalledWith(
      "Assets/Scripts/Scoped.cs",
      paneWorkspaceRef,
    );

    host.querySelector(".test-file-ref")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("common.selectInUnity")?.click();
    await flushUi();
    expect(mocks.selectUnityAsset).toHaveBeenCalledWith(
      paneWorkspaceRef,
      "Assets/Scripts/Scoped.cs",
    );

    host.querySelector(".test-file-ref")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("common.openInLocusInspector")?.click();
    await flushUi();
    expect(mocks.openLocusAssetInspectorWorkbenchTab).toHaveBeenCalledWith(
      paneWorkspaceRef,
      { assetPath: "Assets/Scripts/Scoped.cs" },
    );

    host.querySelector<HTMLSpanElement>(".test-knowledge-ref")?.click();
    await flushUi();
    expect(openKnowledgeDocument).toHaveBeenCalledWith({
      docType: "design",
      path: "design/spec.md",
      workspaceRef: paneWorkspaceRef,
    });
    expect(mocks.legacyOpenKnowledgeDocument).not.toHaveBeenCalled();
    expect(mocks.projectStore.requireWorkspaceRef).not.toHaveBeenCalled();
  });

  it("preserves legacy store routing outside scoped surfaces", async () => {
    const host = mountChat({
      scopedSession: false,
      planModeActive: undefined,
      messages: [{
        id: "user-legacy",
        role: "user",
        content: "Legacy prompt",
        createdAt: 1,
      }],
    });
    await flushUi();

    host.querySelector(".test-message")?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      clientX: 10,
      clientY: 10,
    }));
    await flushUi();
    contextMenuButton("chat.messageMenu.reEditUserMessage")?.click();
    await flushUi();

    expect(mocks.chatStore.undoLatestConversationTurn).toHaveBeenCalledTimes(1);
    expect(mocks.uiStore.stageChatDraftPrefill).toHaveBeenCalledWith(expect.objectContaining({
      text: "Legacy prompt",
    }));

    host.querySelector<HTMLSpanElement>(".test-knowledge-ref")?.click();
    await flushUi();
    expect(mocks.legacyOpenKnowledgeDocument).toHaveBeenCalledWith("design", "design/spec.md");
  });
});
