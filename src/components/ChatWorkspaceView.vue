<script setup lang="ts">
import { computed, ref } from "vue";
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
import ChatView from "./ChatView.vue";
import ThinkingPanel from "./ThinkingPanel.vue";
import ChatSidebarPanel from "./ChatSidebarPanel.vue";

type ChatLayoutMode = "auto" | "horizontal" | "vertical";
type ResolvedChatLayoutMode = "horizontal" | "vertical";

const props = withDefaults(defineProps<{
  active?: boolean;
  layoutMode?: ChatLayoutMode;
}>(), {
  active: true,
  layoutMode: "auto",
});

const agentStore = useAgentStore();
const chatStore = useChatStore();
const chatChangesStore = useChatChangesStore();
const modelStore = useModelStore();
const notificationStore = useNotificationStore();
const projectStore = useProjectStore();
const { skillItems } = useSkills();

const resolvedLayoutMode = ref<ResolvedChatLayoutMode>("horizontal");
const isVerticalLayout = computed(() => resolvedLayoutMode.value === "vertical");
const showAssistantSidebar = computed(() =>
  props.active && (chatStore.showTodoPanel || chatChangesStore.currentPanelVisible),
);

function handleLayoutModeChange(mode: ResolvedChatLayoutMode) {
  resolvedLayoutMode.value = mode;
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
</script>

<template>
  <div
    class="chat-workspace-view"
    :class="{
      'is-horizontal-layout': !isVerticalLayout,
      'is-vertical-layout': isVerticalLayout,
    }"
  >
    <ChatView
      v-show="active"
      :layout-mode="layoutMode"
      :messages="chatStore.messages"
      :streaming-text="chatStore.streamingText"
      :is-streaming="chatStore.isStreaming"
      :is-thinking="chatStore.isThinking"
      :has-thinking="chatStore.streamingThinking.length > 0"
      :thinking-text="chatStore.streamingThinking"
      :thinking-duration="chatStore.thinkingDuration"
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
      :scan-phase="projectStore.scanPhase"
      :last-scan-stats="projectStore.lastScanStats"
      :is-unity-project="projectStore.isUnityProject"
      :skills="skillItems"
      :streaming-session-ids="chatStore.streamingSessionIds"
      :undoable-message-ids="chatStore.undoableMessageIds"
      @send="chatStore.sendMessage"
      @cancel="chatStore.cancelChat"
      @select-agent="(id: string) => agentStore.selectAgent(id)"
      @select-model="(id: string) => modelStore.selectModel(id)"
      @select-effort="(level: EffortLevel) => modelStore.effort = level"
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
      @layout-mode-change="handleLayoutModeChange"
    />
    <ThinkingPanel
      v-if="active && chatStore.showThinkingPanel"
      :thinking="chatStore.thinkingPanelContent || chatStore.streamingThinking"
      :is-thinking="chatStore.isThinking && !chatStore.thinkingPanelContent"
      @close="chatStore.showThinkingPanel = false"
    />
    <ChatSidebarPanel
      v-if="showAssistantSidebar"
      :layout="isVerticalLayout ? 'bottom' : 'side'"
      :todos="chatStore.visibleTodos"
      :is-streaming="chatStore.isStreaming"
      :todo-write-version="chatStore.todoCelebrationVersion"
      :celebration-enabled="chatStore.todoCelebrationEnabled"
    />
  </div>
</template>

<style scoped>
.chat-workspace-view {
  flex: 1;
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
