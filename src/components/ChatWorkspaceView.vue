<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { saveRawContext as saveCtx } from "../services/session";
import type { EffortLevel, SaveRawContextRequest } from "../types";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";
import { useChatChangesStore } from "../stores/chatChanges";
import { useModelStore } from "../stores/model";
import { useNotificationStore } from "../stores/notification";
import { useProjectStore } from "../stores/project";
import { useSkills } from "../composables/useSkills";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../composables/resizeObserver";
import ChatView from "./ChatView.vue";
import ThinkingPanel from "./ThinkingPanel.vue";
import ChatSidebarPanel from "./ChatSidebarPanel.vue";

type ChatLayoutMode = "auto" | "horizontal" | "vertical";
type ResolvedChatLayoutMode = "horizontal" | "vertical";

const props = withDefaults(defineProps<{
  active?: boolean;
  layoutMode?: ChatLayoutMode;
  defaultSessionPanelCollapsed?: boolean;
  sessionPanelStorageScope?: string;
}>(), {
  active: true,
  layoutMode: "auto",
  defaultSessionPanelCollapsed: false,
  sessionPanelStorageScope: "",
});

const agentStore = useAgentStore();
const chatStore = useChatStore();
const chatChangesStore = useChatChangesStore();
const modelStore = useModelStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const { skillItems } = useSkills();

const workspaceRef = ref<HTMLElement | null>(null);
const workspaceWidth = ref(0);
const isVerticalLayout = computed(() => props.layoutMode === "vertical");
const showAssistantSidebar = computed(() =>
  props.active && (chatStore.showTodoPanel || chatChangesStore.currentPanelVisible),
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

function handleLayoutModeChange(_mode: ResolvedChatLayoutMode) {}

function beforeEnterSidebarPanel(element: Element) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.classList.contains("layout-bottom");
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
    return;
  }

  shell.style.width = "0px";
  shell.style.minWidth = "0px";
  shell.style.maxWidth = "0px";
}

function enterSidebarPanel(element: Element, done: () => void) {
  const shell = element as HTMLElement;
  const isBottomLayout = shell.dataset.enterAxis === "vertical";
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
  let finished = false;
  let fallbackTimer = 0;
  const finish = () => {
    if (finished) return;
    finished = true;
    window.clearTimeout(fallbackTimer);
    shell.removeEventListener("transitionend", onTransitionEnd);
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

async function saveRawContext(request?: string | SaveRawContextRequest) {
  const sid = typeof request === "string"
    ? request
    : request?.sessionId || chatStore.activeSessionId;
  const includeSystemPrompt = typeof request === "string"
    ? true
    : request?.includeSystemPrompt ?? true;
  if (!sid) return;
  try {
    const filePath = await save({
      defaultPath: includeSystemPrompt
        ? `context_${sid.slice(0, 8)}_with_system_prompt.md`
        : `context_${sid.slice(0, 8)}_without_system_prompt.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!filePath) return;
    await saveCtx(sid, filePath, includeSystemPrompt);
  } catch (e) {
    const err = normalizeAppError(e);
    console.error("save_raw_context failed:", e);
    notificationStore.addNotice("error", t("app.saveFailed", err.message), {
      code: err.code,
      operation: "saveRawContext",
      skipConsoleLog: true,
    });
  }
}

onMounted(() => {
  nextTick(connectWorkspaceResizeObserver);
});

onUnmounted(() => {
  disconnectWorkspaceResizeObserver();
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
      :messages="chatStore.messages"
      :streaming-text="chatStore.streamingText"
      :streaming-text-order="chatStore.streamingTextOrder"
      :is-streaming="chatStore.isStreaming"
      :is-cancelling="chatStore.isCancelling"
      :is-compacting="chatStore.isCompacting"
      :is-thinking="chatStore.isThinking"
      :has-thinking="chatStore.streamingThinking.length > 0"
      :thinking-text="chatStore.streamingThinking"
      :thinking-order="chatStore.thinkingOrder"
      :thinking-duration="chatStore.thinkingDuration"
      :live-render-parts="chatStore.liveRenderParts"
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
      :token-usage="chatStore.tokenUsage"
      :pending-question="chatStore.pendingQuestion"
      :pending-tool-confirms="chatStore.pendingToolConfirms"
      :sessions="chatStore.sessions"
      :active-session-id="chatStore.activeSessionId"
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
      :streaming-session-ids="chatStore.streamingSessionIds"
      :undoable-message-ids="chatStore.undoableMessageIds"
      @send="chatStore.sendMessage"
      @compact="chatStore.compactSession"
      @fork="chatStore.forkSession"
      @cancel="chatStore.cancelChat"
      @select-agent="(id: string) => agentStore.selectAgent(id)"
      @select-model="(id: string) => modelStore.selectModel(id)"
      @select-effort="(level: EffortLevel) => modelStore.selectEffort(level)"
      @save-raw-context="saveRawContext"
      @answer-question="chatStore.answerQuestion"
      @answer-tool-confirm="chatStore.answerToolConfirm"
      @answer-all-tool-confirms="chatStore.answerAllToolConfirms"
      @open-thinking="chatStore.openThinkingPanel"
      @select-session="chatStore.selectSession"
      @new-chat="chatStore.newChat"
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
      :thinking="chatStore.thinkingPanelContent || chatStore.streamingThinking"
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
        :todos="chatStore.visibleTodos"
        :is-streaming="chatStore.isStreaming"
        :todo-write-version="chatStore.todoCelebrationVersion"
        :celebration-enabled="chatStore.todoCelebrationEnabled"
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
