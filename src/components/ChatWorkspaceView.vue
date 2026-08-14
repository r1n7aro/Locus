<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { createSession, exportSessionContext as exportContext } from "../services/session";
import { broadcastSessionExecutionState } from "../services/sessionExecutionState";
import type {
  EffortLevel,
  ImageAttachment,
  AssetRefAttachment,
  ManagedLocalFileAttachment,
  SessionContextExportRequest,
  SkillIntentItem,
  UserIntentMeta,
} from "../types";
import type { UserMessageDraft } from "../composables/chatMessageDraft";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import { useModelStore } from "../stores/model";
import { useNotificationStore } from "../stores/notification";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import { useSkills } from "../composables/useSkills";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../composables/resizeObserver";
import ChatView from "./ChatView.vue";
import ThinkingPanel from "./ThinkingPanel.vue";
import ChatSidebarPanel from "./ChatSidebarPanel.vue";
import { resolveChatContentBalanceInset } from "./chat/chatSidebarBalance";
import { sessionContextExportFileName } from "../composables/sessionContextExport";

type ChatLayoutMode = "auto" | "horizontal" | "vertical";
type ResolvedChatLayoutMode = "horizontal" | "vertical";

const props = withDefaults(defineProps<{
  active?: boolean;
  layoutMode?: ChatLayoutMode;
  defaultSessionPanelCollapsed?: boolean;
  sessionPanelStorageScope?: string;
  showSessionNavigation?: boolean;
  persistSessionSelection?: boolean;
}>(), {
  active: true,
  layoutMode: "auto",
  defaultSessionPanelCollapsed: false,
  sessionPanelStorageScope: "",
  showSessionNavigation: true,
  persistSessionSelection: true,
});

const agentStore = useAgentStore();
const chatStore = useChatStore();
const chatChangesStore = useChatChangesStore();
const modelStore = useModelStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const uiStore = useUiStore();
const { skillItems } = useSkills();
const contextReviewFiles = ref(new Map<string, ManagedLocalFileAttachment>());

const workspaceRef = ref<HTMLElement | null>(null);
const workspaceWidth = ref(0);
const assistantSidebarBalanceWidth = ref(0);
const isVerticalLayout = computed(() => props.layoutMode === "vertical");
const showAssistantSidebar = computed(() =>
  props.active && chatChangesStore.currentPanelVisible,
);
const ASSISTANT_PANEL_MIN_CHAT_WIDTH = 560;
const ASSISTANT_SIDEBAR_SIDE_MAX_WIDTH = 520;
const ASSISTANT_SIDEBAR_RESIZE_HANDLE_WIDTH = 3;
const ASSISTANT_SIDEBAR_MAX_WORKSPACE_RATIO = 0.34;
const THINKING_PANEL_SIDE_WIDTH = 340;
const SIDEBAR_ENTER_TRANSITION_MS = 200;
const SIDEBAR_EXIT_TRANSITION_MS = 180;
const fixedAuxiliarySideWidth = computed(() =>
  chatStore.showThinkingPanel ? THINKING_PANEL_SIDE_WIDTH : 0,
);
const assistantSidebarMaxSideWidth = computed(() => {
  const width = workspaceWidth.value;
  if (!showAssistantSidebar.value || width <= 0) {
    return ASSISTANT_SIDEBAR_SIDE_MAX_WIDTH;
  }

  const remainingWidthBound = width
    - fixedAuxiliarySideWidth.value
    - ASSISTANT_SIDEBAR_RESIZE_HANDLE_WIDTH
    - ASSISTANT_PANEL_MIN_CHAT_WIDTH;
  const ratioBound = Math.floor(width * ASSISTANT_SIDEBAR_MAX_WORKSPACE_RATIO);
  return Math.max(
    0,
    Math.min(ASSISTANT_SIDEBAR_SIDE_MAX_WIDTH, remainingWidthBound, ratioBound),
  );
});
let workspaceResizeObserver: ResizeObserverHandle | null = null;
let assistantSidebarResizeObserver: ResizeObserver | null = null;
let assistantSidebarShell: HTMLElement | null = null;

function syncAssistantSidebarContentBalance(shell = assistantSidebarShell) {
  if (isVerticalLayout.value || !shell || !workspaceRef.value) {
    assistantSidebarBalanceWidth.value = 0;
    return;
  }
  const chatSurface = workspaceRef.value.querySelector<HTMLElement>(".chat-view");
  if (!chatSurface) {
    assistantSidebarBalanceWidth.value = 0;
    return;
  }
  assistantSidebarBalanceWidth.value = resolveChatContentBalanceInset(
    chatSurface.clientWidth,
    shell.getBoundingClientRect().width,
  );
}

function disconnectAssistantSidebarResizeObserver() {
  assistantSidebarResizeObserver?.disconnect();
  assistantSidebarResizeObserver = null;
  assistantSidebarShell = null;
  assistantSidebarBalanceWidth.value = 0;
}

function connectAssistantSidebarResizeObserver(shell: HTMLElement) {
  disconnectAssistantSidebarResizeObserver();
  assistantSidebarShell = shell;
  syncAssistantSidebarContentBalance(shell);
  if (typeof ResizeObserver === "undefined") return;
  assistantSidebarResizeObserver = new ResizeObserver(() => {
    syncAssistantSidebarContentBalance(shell);
  });
  assistantSidebarResizeObserver.observe(shell);
  const chatSurface = workspaceRef.value?.querySelector<HTMLElement>(".chat-view");
  if (chatSurface) {
    assistantSidebarResizeObserver.observe(chatSurface);
  }
}

function handleLayoutModeChange(_mode: ResolvedChatLayoutMode) {}

function selectWorkspaceSession(sessionId: string) {
  void chatStore.selectSession(sessionId, {
    persist: props.persistSessionSelection,
  });
}

function createWorkspaceSession() {
  chatStore.newChat({
    persistSelection: props.persistSessionSelection,
  });
}

function publishSessionExecutionState() {
  const sessionId = chatStore.activeSessionId;
  if (!sessionId) return;
  chatStore.applyActiveSessionExecutionState(
    sessionId,
    modelStore.selectedModelId,
    modelStore.effort,
  );
  void broadcastSessionExecutionState({
    sessionId,
    modelId: modelStore.selectedModelId,
    effort: modelStore.effort,
  });
}

async function selectWorkspaceModel(modelId: string) {
  modelStore.selectModel(modelId);
  await nextTick();
  publishSessionExecutionState();
}

function selectWorkspaceEffort(effort: EffortLevel) {
  modelStore.selectEffort(effort);
  publishSessionExecutionState();
}

function beforeEnterSidebarPanel(element: Element) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.classList.contains("layout-bottom");
  connectAssistantSidebarResizeObserver(shell);
  shell.dataset.enterAxis = isBottomLayout ? "vertical" : "horizontal";
  shell.style.pointerEvents = "none";
  shell.style.overflow = "hidden";
  shell.style.opacity = "0";
  shell.style.transform = isBottomLayout ? "translateY(8px)" : "translateX(12px)";
  shell.style.willChange = "width, min-width, max-width, height, min-height, max-height, transform, opacity";

  if (isBottomLayout) {
    shell.style.height = "0px";
    shell.style.minHeight = "0px";
    shell.style.maxHeight = "0px";
    syncAssistantSidebarContentBalance(shell);
    return;
  }

  shell.style.width = "0px";
  shell.style.minWidth = "0px";
  shell.style.maxWidth = "0px";
  syncAssistantSidebarContentBalance(shell);
}

function enterSidebarPanel(element: Element, done: () => void) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.dataset.enterAxis === "vertical";
  uiStore.beginAssistantSidebarTransition();
  let finished = false;
  let fallbackTimer = 0;
  let measureFrame = 0;
  let enterFrame = 0;
  let finishFrame = 0;
  const finish = () => {
    if (finished) return;
    finished = true;
    cancelAnimationFrame(measureFrame);
    cancelAnimationFrame(enterFrame);
    cancelAnimationFrame(finishFrame);
    window.clearTimeout(fallbackTimer);
    shell.removeEventListener("transitionend", onTransitionEnd);
    uiStore.endAssistantSidebarTransition();
    done();
  };
  const queueFinish = () => {
    if (finishFrame) return;
    finishFrame = requestAnimationFrame(finish);
  };
  const onTransitionEnd = (event: TransitionEvent) => {
    if (event.target !== shell) return;
    if (isBottomLayout && event.propertyName === "height") queueFinish();
    if (!isBottomLayout && event.propertyName === "width") queueFinish();
  };

  const startEnterTransition = () => {
    if (finished) return;
    shell.style.transition = "none";
    if (isBottomLayout) {
      shell.style.height = "";
      shell.style.minHeight = "";
      shell.style.maxHeight = "";
    } else {
      shell.style.width = "";
      shell.style.minWidth = "";
      shell.style.maxWidth = "";
    }

    const rect = shell.getBoundingClientRect();
    const targetSize = isBottomLayout ? rect.height : rect.width;
    if (targetSize <= 0) {
      finish();
      return;
    }

    if (isBottomLayout) {
      shell.style.height = "0px";
      shell.style.minHeight = "0px";
      shell.style.maxHeight = "0px";
    } else {
      shell.style.width = "0px";
      shell.style.minWidth = "0px";
      shell.style.maxWidth = "0px";
    }
    shell.getBoundingClientRect();
    shell.addEventListener("transitionend", onTransitionEnd);
    shell.style.transition = [
      `width ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `min-width ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `max-width ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `height ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `min-height ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `max-height ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      `transform ${SIDEBAR_ENTER_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
      "opacity 160ms ease",
    ].join(", ");

    enterFrame = requestAnimationFrame(() => {
      shell.style.opacity = "1";
      shell.style.transform = "translate(0, 0)";
      if (isBottomLayout) {
        shell.style.height = `${targetSize}px`;
        shell.style.minHeight = `${targetSize}px`;
        shell.style.maxHeight = `${targetSize}px`;
        return;
      }
      shell.style.width = `${targetSize}px`;
      shell.style.minWidth = `${targetSize}px`;
      shell.style.maxWidth = `${targetSize}px`;
    });

    fallbackTimer = window.setTimeout(finish, SIDEBAR_ENTER_TRANSITION_MS + 100);
  };

  void nextTick(() => {
    measureFrame = requestAnimationFrame(startEnterTransition);
  });
}

function afterEnterSidebarPanel(element: Element) {
  const shell = element as HTMLElement;
  delete shell.dataset.enterAxis;
  shell.removeAttribute("style");
}

function beforeLeaveSidebarPanel(element: Element) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.classList.contains("layout-bottom");
  const rect = shell.getBoundingClientRect();
  shell.dataset.exitAxis = isBottomLayout ? "vertical" : "horizontal";
  shell.style.pointerEvents = "none";
  shell.style.overflow = "hidden";
  shell.style.opacity = "1";
  shell.style.transform = "translate(0, 0)";
  shell.style.willChange = "width, min-width, max-width, height, min-height, max-height, transform, opacity";

  if (isBottomLayout) {
    shell.style.height = `${rect.height}px`;
    shell.style.minHeight = `${rect.height}px`;
    shell.style.maxHeight = `${rect.height}px`;
    return;
  }

  shell.style.width = `${rect.width}px`;
  shell.style.minWidth = `${rect.width}px`;
  shell.style.maxWidth = `${rect.width}px`;
}

function leaveSidebarPanel(element: Element, done: () => void) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.dataset.exitAxis === "vertical";
  uiStore.beginAssistantSidebarTransition();
  let finished = false;
  let fallbackTimer = 0;
  const finish = () => {
    if (finished) return;
    finished = true;
    window.clearTimeout(fallbackTimer);
    shell.removeEventListener("transitionend", onTransitionEnd);
    uiStore.endAssistantSidebarTransition();
    done();
  };
  const onTransitionEnd = (event: TransitionEvent) => {
    if (event.target !== shell) return;
    if (isBottomLayout && event.propertyName === "height") finish();
    if (!isBottomLayout && event.propertyName === "width") finish();
  };

  shell.addEventListener("transitionend", onTransitionEnd);
  shell.getBoundingClientRect();
  shell.style.transition = [
    `width ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `min-width ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `max-width ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `height ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `min-height ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `max-height ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    `transform ${SIDEBAR_EXIT_TRANSITION_MS}ms cubic-bezier(0.2, 0, 0, 1)`,
    "opacity 140ms ease",
  ].join(", ");

  requestAnimationFrame(() => {
    shell.style.opacity = "0";
    if (isBottomLayout) {
      shell.style.height = "0px";
      shell.style.minHeight = "0px";
      shell.style.maxHeight = "0px";
      shell.style.transform = "translateY(100%)";
      return;
    }
    shell.style.width = "0px";
    shell.style.minWidth = "0px";
    shell.style.maxWidth = "0px";
    shell.style.transform = "translateX(100%)";
  });

  fallbackTimer = window.setTimeout(finish, SIDEBAR_EXIT_TRANSITION_MS + 80);
}

function afterLeaveSidebarPanel(element: Element) {
  const shell = element as HTMLElement;
  delete shell.dataset.exitAxis;
  shell.removeAttribute("style");
  disconnectAssistantSidebarResizeObserver();
}

function setWorkspaceWidth(width: number) {
  const nextWidth = Math.max(0, Math.round(width));
  if (workspaceWidth.value === nextWidth) return;
  workspaceWidth.value = nextWidth;
}

function updateWorkspaceWidth() {
  setWorkspaceWidth(workspaceRef.value?.clientWidth ?? 0);
}

function handleWorkspaceResize(entries: ResizeObserverEntry[]) {
  const width = entries[0]?.contentRect.width ?? workspaceRef.value?.clientWidth ?? 0;
  setWorkspaceWidth(width);
  syncAssistantSidebarContentBalance();
}

function disconnectWorkspaceResizeObserver() {
  workspaceResizeObserver?.disconnect();
  workspaceResizeObserver = null;
}

function connectWorkspaceResizeObserver() {
  disconnectWorkspaceResizeObserver();
  updateWorkspaceWidth();
  if (typeof ResizeObserver === "undefined" || !workspaceRef.value) return;
  workspaceResizeObserver = createAnimationFrameResizeObserver(handleWorkspaceResize);
  if (!workspaceResizeObserver) return;
  workspaceResizeObserver.observe(workspaceRef.value);
}

function resolveContextSessionId(request?: string | SessionContextExportRequest): string {
  return (typeof request === "string" ? request : request?.sessionId || chatStore.activeSessionId)?.trim() ?? "";
}

function reviewContextSkillIntent(): UserIntentMeta {
  const manifest = skillItems.value.find((skill) =>
    skill.dirName === "review-context" && skill.source === "project"
  ) ?? skillItems.value.find((skill) =>
    skill.dirName === "review-context" && skill.source === "app"
  );
  const skill: SkillIntentItem = manifest
    ? {
        dirName: manifest.dirName,
        source: manifest.source,
        name: manifest.name,
      }
    : {
        dirName: "review-context",
        source: "app",
        name: "Review Context",
      };
  return {
    kind: "user_intent_v1",
    mode: "build",
    skills: [skill],
  };
}

const activeContextReviewFiles = computed(() => {
  const sessionId = chatStore.activeSessionId;
  if (!sessionId) return [];
  const file = contextReviewFiles.value.get(sessionId);
  return file ? [file] : [];
});

function setContextReviewFile(sessionId: string, file: ManagedLocalFileAttachment) {
  const next = new Map(contextReviewFiles.value);
  next.set(sessionId, file);
  contextReviewFiles.value = next;
}

function removeContextReviewFile(sessionId: string, fileId?: string) {
  const current = contextReviewFiles.value.get(sessionId);
  if (!current || (fileId && current.id !== fileId)) return;
  const next = new Map(contextReviewFiles.value);
  next.delete(sessionId);
  contextReviewFiles.value = next;
}

function contextReviewDraft(): UserMessageDraft {
  const intent = reviewContextSkillIntent();
  return {
    text: t("chat.contextReviewPrompt"),
    images: [],
    assetRefs: [],
    localFiles: [],
    consoleTexts: [],
    intent: {
      mode: intent.mode,
      skills: intent.skills,
    },
  };
}

function contextReviewFileName(filePath: string, fallback: string) {
  return filePath.replace(/\\/g, "/").split("/").filter(Boolean).pop() || fallback;
}

async function sendWorkspaceMessage(
  text: string,
  images: ImageAttachment[],
  assetRefs: AssetRefAttachment[],
  overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null },
) {
  const sessionId = chatStore.activeSessionId;
  if (sessionId) {
    removeContextReviewFile(sessionId);
  }
  await chatStore.sendMessage(text, images, assetRefs, overrides);
}

function removeManagedComposerFile(fileId: string) {
  const sessionId = chatStore.activeSessionId;
  if (!sessionId) return;
  removeContextReviewFile(sessionId, fileId);
}

async function exportSessionContext(request?: string | SessionContextExportRequest) {
  const sid = resolveContextSessionId(request);
  if (!sid) return;
  try {
    const sessionTitle = chatStore.sessions.find((session) => session.id === sid)?.title || "untitled";
    const filePath = await save({
      defaultPath: sessionContextExportFileName(sid, sessionTitle),
      filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
    });
    if (!filePath) return;
    const result = await exportContext(sid, filePath);
    notificationStore.addNotice("success", t("chat.contextExported", result.filePath), {
      operation: "exportSessionContext",
      replaceOperation: true,
    });
  } catch (e) {
    const err = normalizeAppError(e);
    console.error("export_session_context failed:", e);
    notificationStore.addNotice("error", t("app.saveFailed", err.message), {
      code: err.code,
      operation: "exportSessionContext",
      skipConsoleLog: true,
    });
  }
}

async function reviewSessionContext(request?: string | SessionContextExportRequest) {
  const sid = resolveContextSessionId(request);
  if (!sid) return;

  const source = chatStore.sessions.find((session) => session.id === sid);
  const sourceTitle = source?.title || sid.slice(0, 8);
  chatStore.newChat({ persistSelection: props.persistSessionSelection });

  let reviewSessionId = "";
  try {
    reviewSessionId = await createSession({
      title: t("chat.contextReviewTitle", sourceTitle),
      sessionType: "chat",
      agentId: agentStore.selectedAgentId || null,
    });
    const fileId = `context-review:${reviewSessionId}`;
    const loadingName = sessionContextExportFileName(sid, sourceTitle);
    setContextReviewFile(reviewSessionId, {
      id: fileId,
      name: loadingName,
      typeLabel: "YAML",
      status: "loading",
    });
    await chatStore.selectSession(reviewSessionId, {
      persist: props.persistSessionSelection,
    });
    uiStore.stageChatDraftPrefill(contextReviewDraft(), {
      sessionId: reviewSessionId,
    });
    void chatStore.refreshSessions();

    try {
      const result = await exportContext(sid, null);
      const current = contextReviewFiles.value.get(reviewSessionId);
      if (!current || current.id !== fileId) return;
      setContextReviewFile(reviewSessionId, {
        ...current,
        name: contextReviewFileName(result.filePath, loadingName),
        path: result.filePath,
        status: "ready",
      });
    } catch (e) {
      const current = contextReviewFiles.value.get(reviewSessionId);
      if (current?.id === fileId) {
        setContextReviewFile(reviewSessionId, {
          ...current,
          status: "error",
        });
      }
      throw e;
    }
  } catch (e) {
    const err = normalizeAppError(e);
    console.error("review_session_context failed:", e);
    notificationStore.addNotice("error", t("chat.contextReviewFailed", err.message), {
      code: err.code,
      operation: "reviewSessionContext",
      skipConsoleLog: true,
    });
    if (!reviewSessionId && chatStore.activeSessionId === null) {
      void chatStore.selectSession(sid, {
        persist: props.persistSessionSelection,
      });
    }
  }
}

onMounted(() => {
  nextTick(connectWorkspaceResizeObserver);
});

onUnmounted(() => {
  disconnectWorkspaceResizeObserver();
  disconnectAssistantSidebarResizeObserver();
});
</script>

<template>
  <div
    ref="workspaceRef"
    class="chat-workspace-view"
    :class="{
      'is-horizontal-layout': !isVerticalLayout,
      'is-vertical-layout': isVerticalLayout,
    }"
  >
    <ChatView
      v-show="active"
      :layout-mode="layoutMode"
      :default-session-panel-collapsed="defaultSessionPanelCollapsed"
      :session-panel-storage-scope="sessionPanelStorageScope"
      :show-session-navigation="showSessionNavigation"
      :content-start-inset="assistantSidebarBalanceWidth"
      :messages="chatStore.messages"
      streaming-text=""
      :typed-text-stream="chatStore.typedStream"
      :has-streaming-text="chatStore.hasStreamingText"
      :streaming-text-order="chatStore.streamingTextOrder"
      :is-streaming="chatStore.isStreaming"
      :is-cancelling="chatStore.isCancelling"
      :can-resume-interrupted="chatStore.canResumeInterrupted"
      :is-compacting="chatStore.isCompacting"
      :is-thinking="chatStore.isThinking"
      :has-thinking="chatStore.hasStreamingThinking"
      :thinking-order="chatStore.thinkingOrder"
      :thinking-duration="chatStore.thinkingDuration"
      :live-render-parts="chatStore.liveRenderParts"
      :live-part-streams="chatStore.livePartStreams"
      :active-tool-calls="chatStore.activeToolCalls"
      :agents="agentStore.agents"
      :selected-agent-id="agentStore.selectedAgentId"
      :agent-locked="chatStore.sessionAgentLocked"
      :models="modelStore.availableModels"
      :selected-model-id="modelStore.selectedModelId"
      :codex-transport="modelStore.codexTransport"
      :effort="modelStore.effort"
      :effort-supported="modelStore.effortSupported"
      :effort-levels="modelStore.availableEfforts"
      :fast-mode-enabled="modelStore.effectiveCodexFastMode"
      :fast-mode-available="modelStore.codexFastModeAvailable"
      :token-usage="chatStore.tokenUsage"
      :pending-question="chatStore.pendingQuestion"
      :pending-tool-confirms="chatStore.pendingToolConfirms"
      :sessions="chatStore.sessions"
      :active-session-id="chatStore.activeSessionId"
      :pending-session-id="chatStore.pendingSelectionSessionId"
      :unity-connected="projectStore.unityConnected"
      :unity-plugin-status="projectStore.pluginToast"
      :unity-plugin-installing="projectStore.pluginInstalling"
      :unity-launching="projectStore.unityLaunching"
      :unity-launch-state="projectStore.unityLaunchState"
      :unity-connection-status="projectStore.unityConnectionStatus"
      :working-dir="projectStore.workingDir"
      :scan-phase="projectStore.scanPhase"
      :last-scan-stats="projectStore.lastScanStats"
      :is-unity-project="projectStore.isUnityProject"
      :skills="skillItems"
      :managed-local-files="activeContextReviewFiles"
      :streaming-session-ids="chatStore.streamingSessionIds"
      :undoable-message-ids="chatStore.undoableMessageIds"
      @send="sendWorkspaceMessage"
      @compact="chatStore.compactSession"
      @fork="chatStore.forkSession"
      @cancel="chatStore.cancelChat"
      @resume="chatStore.resumeInterrupted"
      @select-agent="(id: string) => agentStore.selectAgent(id)"
      @select-model="selectWorkspaceModel"
      @select-effort="selectWorkspaceEffort"
      @select-fast-mode="(enabled: boolean) => modelStore.selectCodexFastMode(enabled)"
      @export-session-context="exportSessionContext"
      @review-session-context="reviewSessionContext"
      @remove-managed-composer-file="removeManagedComposerFile"
      @answer-question="chatStore.answerQuestion"
      @answer-tool-confirm="chatStore.answerToolConfirm"
      @answer-all-tool-confirms="chatStore.answerAllToolConfirms"
      @open-thinking="chatStore.openThinkingPanel"
      @select-session="selectWorkspaceSession"
      @new-chat="createWorkspaceSession"
      @rename-session="chatStore.renameSession"
      @archive-session="chatStore.archiveSession"
      @delete-session="chatStore.deleteSession"
      @start-scan="projectStore.startScan"
      @install-plugin="projectStore.installPlugin"
      @launch-unity-project="projectStore.launchUnityProject"
      @layout-mode-change="handleLayoutModeChange"
    />
    <ThinkingPanel
      v-if="active && chatStore.showThinkingPanel"
      :text="chatStore.thinkingPanelContent"
      :stream="chatStore.thinkingStream"
      :is-thinking="chatStore.isThinking && !chatStore.thinkingPanelContent"
      @close="chatStore.showThinkingPanel = false"
    />
    <Transition
      :css="false"
      @before-enter="beforeEnterSidebarPanel"
      @enter="enterSidebarPanel"
      @after-enter="afterEnterSidebarPanel"
      @before-leave="beforeLeaveSidebarPanel"
      @leave="leaveSidebarPanel"
      @after-leave="afterLeaveSidebarPanel"
    >
      <ChatSidebarPanel
        v-if="showAssistantSidebar"
        :layout="isVerticalLayout ? 'bottom' : 'side'"
        :max-side-width="assistantSidebarMaxSideWidth"
        :storage-scope="sessionPanelStorageScope"
      />
    </Transition>
  </div>
</template>

<style scoped>
.chat-workspace-view {
  flex: 1 1 0;
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.chat-workspace-view.is-horizontal-layout {
  flex-direction: row;
}

.chat-workspace-view.is-vertical-layout {
  flex-direction: column;
}

.chat-workspace-view.is-vertical-layout :deep(.thinking-panel) {
  width: 100%;
  min-width: 0;
  height: 220px;
  min-height: 180px;
  border-left: none;
  border-top: 1px solid var(--border-color);
  flex-shrink: 0;
}
</style>
