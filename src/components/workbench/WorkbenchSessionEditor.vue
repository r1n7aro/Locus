<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import { useEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
import { useSkills } from "../../composables/useSkills";
import { useKnowledgeAccessMode } from "../../composables/useKnowledgeAccessMode";
import { useWorkspaceAssetDbStatus } from "../../composables/useWorkspaceAssetDbStatus";
import { useWorkspaceUnityStatus } from "../../composables/useWorkspaceUnityStatus";
import type { WorkspaceRef } from "../../services/project";
import { saveSessionExecutionState } from "../../services/session";
import { broadcastSessionExecutionState } from "../../services/sessionExecutionState";
import { useAgentStore } from "../../stores/agent";
import { useAuthStore } from "../../stores/auth";
import { useChatChangesStore } from "../../stores/chatChanges";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import type {
  AssetRefAttachment,
  ChatComposerSendPayload,
  EffortLevel,
  ImageAttachment,
  KnowledgeDocumentType,
  SessionContextExportRequest,
  UserIntentMeta,
} from "../../types";
import type { WorkbenchEditorInput } from "../../types/workbench";
import ChatView from "../ChatView.vue";
import ChatSidebarPanel from "../ChatSidebarPanel.vue";
import ThinkingPanel from "../ThinkingPanel.vue";

const props = defineProps<{
  editor: WorkbenchEditorInput;
  workspaceRef: WorkspaceRef | null;
  referenceDropAvailable?: boolean;
  referenceDropActive?: boolean;
  shortcutActive?: boolean;
  newChatShortcutAction?: "keepCurrent" | "replaceCurrent" | "newTab";
}>();

const emit = defineEmits<{
  (event: "session-created", payload: { editorId: string; sessionId: string }): void;
  (event: "new-session-requested", payload: {
    editorId: string;
    source: "control" | "shortcut";
  }): void;
  (event: "composer-draft-change", payload: { editorId: string; hasDraft: boolean }): void;
  (event: "composer-focus", payload: { editorId: string }): void;
  (event: "session-forked", payload: {
    editorId: string;
    sourceSessionId: string;
    forkedSessionId: string;
  }): void;
  (event: "export-session-context", payload: {
    editorId: string;
    request: SessionContextExportRequest;
  }): void;
  (event: "review-session-context", payload: {
    editorId: string;
    request: SessionContextExportRequest;
  }): void;
  (event: "open-knowledge-document", payload: {
    editorId: string;
    target: "editor" | "knowledge";
    request: { docType: KnowledgeDocumentType; path: string; workspaceRef: WorkspaceRef };
  }): void;
}>();

const agentStore = useAgentStore();
const authStore = useAuthStore();
const chatChangesStore = useChatChangesStore();
const modelStore = useModelStore();
const notificationStore = useNotificationStore();
const workspaceContextStore = useWorkspaceContextStore();
const { skillItems } = useSkills();
const { state: knowledgeAccessState } = useKnowledgeAccessMode();
const chatViewRef = ref<InstanceType<typeof ChatView> | null>(null);

const requestedSessionId = computed(() => (
  props.editor.resource.kind === "session" ? props.editor.resource.sessionId : null
));
const sessionKey = computed(() => `workbench:${props.editor.editorId}`);
const editorAgentId = ref(agentStore.selectedAgentId?.trim() ?? "");
const editorModelId = ref(modelStore.selectedModelId);
const editorEffort = ref<EffortLevel>(modelStore.effort);
const editorFastMode = ref(modelStore.effectiveCodexFastMode);
const editorMultiAgentEnabled = ref(false);
const selectedAgentId = computed(() => {
  const current = editorAgentId.value.trim();
  if (current && agentStore.agents.some((agent) => agent.id === current)) return current;
  return agentStore.agents.find((agent) => agent.isDefault)?.id
    ?? agentStore.agents[0]?.id
    ?? "";
});
const editorEfforts = computed<EffortLevel[]>(() => {
  const model = modelStore.availableModels.find((candidate) => candidate.id === editorModelId.value);
  if (model?.supportedEfforts?.length) return model.supportedEfforts;
  if (editorModelId.value === modelStore.selectedModelId) return modelStore.availableEfforts;
  return [];
});
const editorEffortSupported = computed(() => editorEfforts.value.length > 0);
const editorFastModeAvailable = computed(() => {
  const model = modelStore.availableModels.find((candidate) => candidate.id === editorModelId.value);
  return model?.provider === "openai_codex" || model?.additionalSpeedTiers?.includes("fast") === true;
});
const checkout = computed(() => (
  props.workspaceRef
    ? workspaceContextStore.checkoutsById[props.workspaceRef.checkoutId] ?? null
    : null
));
const workingDir = computed(() => checkout.value?.root ?? "");
const projectServices = computed(() => checkout.value?.runtime?.detectedServices ?? []);
const isUnityWorkspace = computed(() => (
  projectServices.value.some((serviceId) => serviceId.trim().toLowerCase() === "unity")
));
const {
  scanPhase: workspaceScanPhase,
  lastScanStats: workspaceLastScanStats,
  startScan: startWorkspaceAssetScan,
} = useWorkspaceAssetDbStatus({
  workspaceRef: computed(() => props.workspaceRef),
  enabled: isUnityWorkspace,
  onScanError(error) {
    notificationStore.addNotice("error", error.message, {
      code: error.code,
      operation: "ref_graph_scan_start",
      skipConsoleLog: true,
    });
  },
});
const {
  connected: workspaceUnityConnected,
  connectionStatus: workspaceUnityConnectionStatus,
  pluginStatus: workspaceUnityPluginStatus,
  pluginInstalling: workspaceUnityPluginInstalling,
  launching: workspaceUnityLaunching,
  launchState: workspaceUnityLaunchState,
  launch: launchWorkspaceUnity,
  installPlugin: installWorkspaceUnityPlugin,
} = useWorkspaceUnityStatus({
  workspaceRef: computed(() => props.workspaceRef),
  enabled: isUnityWorkspace,
  onError(error, operation) {
    notificationStore.addNotice("error", error.message, {
      code: error.code,
      operation,
      skipConsoleLog: true,
    });
  },
});

const {
  inputText,
  restoredComposerDraft,
  clearRestoredComposerDraft,
  messages,
  streamingText,
  thinkingStream,
  streamingTextOrder,
  thinkingOrder,
  liveRenderParts,
  livePartStreams,
  isStreaming,
  isCancelling,
  isCompacting,
  compactQueued,
  isThinking,
  hasThinking,
  thinkingDuration,
  activeToolCalls,
  tokenUsage,
  undoableMessageIds,
  pendingQuestion,
  pendingToolConfirms,
  queuedFollowUp,
  errorMessage,
  sessionId,
  currentRunId,
  sessionAgentId,
  sessionModelId,
  sessionEffort,
  sessionFastMode,
  sessionMultiAgentEnabled,
  parentSessionId,
  latestCompletedRunId,
  planModeActive,
  canResumeInterrupted,
  sessionHistoryHasMore,
  sessionHistoryLoading,
  sessionUserMessageIds,
  setExecutionSelection,
  sendComposerPayload,
  compact,
  resumeInterrupted,
  insertQueuedFollowUp,
  deleteQueuedFollowUp,
  reEditQueuedFollowUp,
  cancel,
  setPlanMode,
  exitPlanMode,
  loadOlderHistory,
  loadSessionHistoryThroughMessage,
  loadSessionTurnPreview,
  restoreComposerDraft,
  forkFromMessage,
  forkSession,
  checkUndoConflicts,
  checkUndoDirty,
  performUndo,
  rollbackConversation,
  rollbackFilesAndConversation,
  resetSession,
  answerQuestion,
  answerToolConfirm,
  answerAllToolConfirms,
  applyKnowledgeProposal,
  ignoreKnowledgeProposal,
} = useEmbeddedChatSession({
  sessionKey,
  initialSessionId: requestedSessionId,
  workspaceRef: computed(() => props.workspaceRef),
  sessionType: "chat",
  // Generic chat titles are derived from the first user prompt by the backend.
  // The editor title is only a UI placeholder while this is a new session.
  sessionTitle: null,
  selectedModelId: editorModelId,
  selectedAgentId,
  effort: editorEffort,
  effortSupported: editorEffortSupported,
  fastMode: editorFastMode,
  multiAgentEnabled: editorMultiAgentEnabled,
  knowledgeMode: computed(() => knowledgeAccessState.mode),
  buildRequest(input) {
    return { text: input, displayText: input };
  },
});

watch(restoredComposerDraft, async (draft) => {
  if (!draft) return;
  await nextTick();
  await chatViewRef.value?.applyDraftPrefill(draft);
  clearRestoredComposerDraft(draft);
}, { flush: "post" });

watch(
  () => inputText.value.length > 0,
  (hasDraft) => {
    emit("composer-draft-change", {
      editorId: props.editor.editorId,
      hasDraft,
    });
  },
  { immediate: true },
);

const streamingSessionIds = computed(() => (
  sessionId.value && isStreaming.value ? new Set([sessionId.value]) : new Set<string>()
));
const changesPanelVisible = computed(() => (
  chatChangesStore.sessionState(sessionId.value)?.panelVisible ?? false
));
const showThinkingPanel = ref(false);
const thinkingPanelContent = ref("");

watch(sessionId, (nextSessionId) => {
  if (!nextSessionId) return;
  chatChangesStore.setActiveRunId(nextSessionId, currentRunId.value);
  chatChangesStore.setLatestCompletedRunId(nextSessionId, latestCompletedRunId.value);
  void chatChangesStore.refresh(nextSessionId, { allowAutoOpen: false });
}, { immediate: true });

watch([sessionId, currentRunId] as const, ([targetSessionId, runId]) => {
  chatChangesStore.setActiveRunId(targetSessionId, runId);
});

watch([sessionId, latestCompletedRunId] as const, ([targetSessionId, runId]) => {
  chatChangesStore.setLatestCompletedRunId(targetSessionId, runId);
});

watch(
  [sessionId, () => undoableMessageIds.value.size] as const,
  ([targetSessionId, undoableCount], [previousSessionId, previousUndoableCount]) => {
    if (
      !targetSessionId
      || targetSessionId !== previousSessionId
      || undoableCount === previousUndoableCount
    ) return;
    chatChangesStore.setActiveRunId(targetSessionId, currentRunId.value);
    void chatChangesStore.refresh(targetSessionId);
  },
);

watch(isStreaming, (streaming, wasStreaming) => {
  if (!streaming && wasStreaming && sessionId.value) {
    void chatChangesStore.refresh(sessionId.value);
  }
});

watch(sessionId, (nextSessionId) => {
  if (!nextSessionId || requestedSessionId.value === nextSessionId) return;
  emit("session-created", {
    editorId: props.editor.editorId,
    sessionId: nextSessionId,
  });
});

watch(errorMessage, (message) => {
  if (message) notificationStore.addNotice("error", message);
});

watch(
  [sessionAgentId, sessionModelId, sessionEffort, sessionFastMode, sessionMultiAgentEnabled] as const,
  ([agentId, modelId, effort, fastMode, multiAgentEnabled]) => {
    if (agentId && agentStore.agents.some((agent) => agent.id === agentId)) editorAgentId.value = agentId;
    if (modelId && modelStore.availableModels.some((model) => model.id === modelId)) editorModelId.value = modelId;
    if (effort && editorEfforts.value.includes(effort)) editorEffort.value = effort;
    if (fastMode != null) editorFastMode.value = editorFastModeAvailable.value && fastMode;
    editorMultiAgentEnabled.value = multiAgentEnabled;
  },
  { immediate: true },
);

watch(editorFastModeAvailable, (available) => {
  if (!available) editorFastMode.value = false;
}, { immediate: true });

let executionStateSaveQueue = Promise.resolve();

function persistExecutionSelection(): void {
  const targetSessionId = sessionId.value;
  if (!targetSessionId) return;
  const modelId = editorModelId.value;
  const effort = editorEffort.value;
  const fastMode = editorFastModeAvailable.value && editorFastMode.value;
  const multiAgentEnabled = editorMultiAgentEnabled.value;
  void broadcastSessionExecutionState({
    sessionId: targetSessionId,
    modelId,
    effort,
    fastMode,
    multiAgentEnabled,
  });
  executionStateSaveQueue = executionStateSaveQueue
    .catch(() => undefined)
    .then(() => saveSessionExecutionState(targetSessionId, modelId, effort, fastMode, multiAgentEnabled))
    .catch((error: unknown) => {
      console.warn("save_session_execution_state failed:", error);
    });
}

watch(
  [selectedAgentId, editorModelId, editorEffort, editorFastMode, editorMultiAgentEnabled] as const,
  ([agentId, modelId, effort, fastMode, multiAgentEnabled]) => {
    setExecutionSelection({ agentId, modelId, effort, fastMode, multiAgentEnabled });
  },
  { immediate: true },
);

function handleSend(
  text: string,
  images: ImageAttachment[],
  assetRefs: AssetRefAttachment[],
  overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null },
): void {
  const payload: ChatComposerSendPayload = {
    text,
    displayText: overrides?.displayText ?? text,
    images,
    assetRefs,
    mode: overrides?.mode ?? null,
    userIntent: overrides?.userIntent ?? null,
  };
  sendComposerPayload(payload);
}

async function handleReEditQueuedFollowUp(): Promise<void> {
  const draft = await reEditQueuedFollowUp();
  if (!draft) return;
  await nextTick();
  await chatViewRef.value?.applyDraftPrefill(draft);
}

async function applyDraftPrefill(
  draft: Parameters<NonNullable<InstanceType<typeof ChatView>["applyDraftPrefill"]>>[0],
): Promise<void> {
  await chatViewRef.value?.applyDraftPrefill(draft);
}

async function appendComposerDraft(
  draft: Parameters<NonNullable<InstanceType<typeof ChatView>["appendComposerDraft"]>>[0],
): Promise<void> {
  await chatViewRef.value?.appendComposerDraft(draft);
}

function exportTransferSnapshot() {
  return {
    kind: "session" as const,
    composerDraft: chatViewRef.value?.exportComposerDraft() ?? null,
  };
}

function exportComposerDraft() {
  return chatViewRef.value?.exportComposerDraft() ?? null;
}

async function focusComposerInput(): Promise<void> {
  await chatViewRef.value?.focusComposerInput();
}

function handleSelectAgent(agentId: string): void {
  const agent = agentStore.agents.find((item) => item.id === agentId);
  const fallbackEffort = modelStore.hasUserDefaultEffort
    ? modelStore.defaultEffort
    : (agent?.defaultEffort ?? "none");
  editorAgentId.value = agentId;
  if (editorEfforts.value.includes(fallbackEffort)) editorEffort.value = fallbackEffort;
}

function handleSelectModel(modelId: string): void {
  if (!modelStore.availableModels.some((model) => model.id === modelId)) return;
  editorModelId.value = modelId;
  if (editorEfforts.value.length > 0 && !editorEfforts.value.includes(editorEffort.value)) {
    editorEffort.value = editorEfforts.value[0]!;
  }
  if (!editorFastModeAvailable.value) editorFastMode.value = false;
  persistExecutionSelection();
}

function handleSelectEffort(effort: EffortLevel): void {
  if (!editorEfforts.value.includes(effort)) return;
  editorEffort.value = effort;
  persistExecutionSelection();
}

function handleSelectFastMode(enabled: boolean): void {
  editorFastMode.value = editorFastModeAvailable.value && enabled;
  persistExecutionSelection();
}

function handleSelectMultiAgent(enabled: boolean): void {
  editorMultiAgentEnabled.value = enabled;
  persistExecutionSelection();
}

function handleOpenThinking(content: string): void {
  thinkingPanelContent.value = content;
  showThinkingPanel.value = true;
}

function closeThinkingPanel(): void {
  showThinkingPanel.value = false;
}

async function handleForkFromMessage(messageId: string): Promise<void> {
  const sourceSessionId = sessionId.value;
  if (!sourceSessionId) return;
  const forkedSessionId = await forkFromMessage(
    messageId,
    t("chat.session.forkTitle", props.editor.title),
  );
  if (!forkedSessionId) return;
  emit("session-forked", {
    editorId: props.editor.editorId,
    sourceSessionId,
    forkedSessionId,
  });
}

async function handleForkSession(): Promise<void> {
  const sourceSessionId = sessionId.value;
  if (!sourceSessionId) return;
  const forkedSessionId = await forkSession(t("chat.session.forkTitle", props.editor.title));
  if (!forkedSessionId) return;
  emit("session-forked", {
    editorId: props.editor.editorId,
    sourceSessionId,
    forkedSessionId,
  });
}

function handleExportSessionContext(request: SessionContextExportRequest): void {
  if (!request.sessionId.trim()) return;
  emit("export-session-context", { editorId: props.editor.editorId, request });
}

function handleReviewSessionContext(request: SessionContextExportRequest): void {
  if (!request.sessionId.trim()) return;
  emit("review-session-context", { editorId: props.editor.editorId, request });
}

function handleOpenKnowledgeDocument(
  target: "editor" | "knowledge",
  request: { docType: KnowledgeDocumentType; path: string; workspaceRef: WorkspaceRef },
): void {
  emit("open-knowledge-document", {
    editorId: props.editor.editorId,
    target,
    request,
  });
}

function handleNewSessionRequest(request: { source: "control" | "shortcut" }): void {
  if (request.source !== "shortcut" || props.newChatShortcutAction !== "newTab") {
    resetSession();
  }
  emit("new-session-requested", {
    editorId: props.editor.editorId,
    source: request.source,
  });
}

defineExpose({
  applyDraftPrefill,
  appendComposerDraft,
  exportComposerDraft,
  exportTransferSnapshot,
  focusComposerInput,
});
</script>

<template>
  <div class="workbench-session-shell" :data-session-id="sessionId || undefined">
    <ChatView
      ref="chatViewRef"
      class="workbench-session-editor"
      scoped-session
      managed-native-drops
    :session-surface-key="sessionKey"
    :messages="messages"
    :streaming-text="streamingText"
    :has-streaming-text="streamingText.length > 0"
    :streaming-text-order="streamingTextOrder"
    :is-streaming="isStreaming"
    :is-cancelling="isCancelling"
    :can-resume-interrupted="canResumeInterrupted"
    :is-compacting="isCompacting"
    :compact-queued="compactQueued"
    :is-thinking="isThinking"
    :has-thinking="hasThinking"
    :thinking-order="thinkingOrder"
    :thinking-duration="thinkingDuration"
    :live-render-parts="liveRenderParts"
    :live-part-streams="livePartStreams"
    :active-tool-calls="activeToolCalls"
    :agents="agentStore.agents"
    :selected-agent-id="selectedAgentId"
    :agent-locked="false"
    :models="modelStore.availableModels"
    :selected-model-id="editorModelId"
    :codex-transport="modelStore.codexTransport"
    :effort="editorEffort"
    :effort-supported="editorEffortSupported"
    :effort-levels="editorEfforts"
    :fast-mode-enabled="editorFastModeAvailable && editorFastMode"
    :fast-mode-available="editorFastModeAvailable"
    :token-usage="tokenUsage"
    :codex-connected="authStore.codexAuthenticated"
    :pending-question="pendingQuestion"
    :pending-tool-confirms="pendingToolConfirms"
    :queued-follow-up="queuedFollowUp"
    :composer-value="inputText"
    :sessions="[]"
    :active-session-id="sessionId"
    :current-run-id="currentRunId"
    :is-viewing-subagent="!!parentSessionId"
    :pending-session-id="null"
    :unity-connected="workspaceUnityConnected"
    :unity-plugin-status="workspaceUnityPluginStatus"
    :unity-plugin-installing="workspaceUnityPluginInstalling"
    :unity-launching="workspaceUnityLaunching"
    :unity-launch-state="workspaceUnityLaunchState"
    :unity-connection-status="workspaceUnityConnectionStatus"
    :workspace-ref="workspaceRef"
    :project-id="editor.resource.projectId"
    :reference-drop-available="referenceDropAvailable"
    :reference-drop-active="referenceDropActive"
    :shortcut-active="shortcutActive"
    :new-chat-shortcut-action="newChatShortcutAction"
    :project-services="projectServices"
    :working-dir="workingDir"
    :scan-phase="workspaceScanPhase"
    :last-scan-stats="workspaceLastScanStats"
    :skills="skillItems"
    :streaming-session-ids="streamingSessionIds"
    :undoable-message-ids="undoableMessageIds"
    :check-undo-dirty="checkUndoDirty"
    :undo-conversation="rollbackConversation"
    :undo-files-and-conversation="rollbackFilesAndConversation"
    :restore-composer-draft="restoreComposerDraft"
    :fork-from-message="handleForkFromMessage"
    :exit-plan-mode="exitPlanMode"
    :plan-mode-active="planModeActive"
    :session-history-loading="sessionHistoryLoading"
    :session-history-has-more="sessionHistoryHasMore"
    :load-older-history="loadOlderHistory"
    :session-user-message-ids="sessionUserMessageIds"
    :load-session-turn-preview="loadSessionTurnPreview"
    :load-session-history-through-message="loadSessionHistoryThroughMessage"
    :show-session-navigation="false"
    :session-panel-storage-scope="sessionKey"
    :composer-draft-state-key="sessionKey"
    @send="handleSend"
    @compact="compact"
    @fork="handleForkSession"
    @cancel="cancel"
    @resume="resumeInterrupted"
    @select-agent="handleSelectAgent"
    @select-model="handleSelectModel"
    @select-effort="handleSelectEffort"
    @select-fast-mode="handleSelectFastMode"
    :multi-agent-enabled="editorMultiAgentEnabled"
    @select-multi-agent="handleSelectMultiAgent"
    @answer-question="answerQuestion"
    @answer-tool-confirm="answerToolConfirm"
    @answer-all-tool-confirms="answerAllToolConfirms"
    @insert-queued-follow-up="insertQueuedFollowUp"
    @re-edit-queued-follow-up="handleReEditQueuedFollowUp"
    @delete-queued-follow-up="deleteQueuedFollowUp"
    @apply-knowledge-proposal="applyKnowledgeProposal"
    @ignore-knowledge-proposal="ignoreKnowledgeProposal"
    @update-composer-value="inputText = $event"
    @request-plan-mode="setPlanMode"
    @export-session-context="handleExportSessionContext"
    @review-session-context="handleReviewSessionContext"
    @composer-focus="emit('composer-focus', { editorId: props.editor.editorId })"
    @open-thinking="handleOpenThinking"
    @open-knowledge-document="handleOpenKnowledgeDocument('editor', $event)"
    @open-knowledge-document-in-knowledge="handleOpenKnowledgeDocument('knowledge', $event)"
    @new-chat="handleNewSessionRequest"
    @start-scan="startWorkspaceAssetScan"
    @install-plugin="installWorkspaceUnityPlugin"
    @launch-unity-project="launchWorkspaceUnity"
    />
    <ThinkingPanel
      v-if="showThinkingPanel"
      :text="thinkingPanelContent"
      :stream="thinkingPanelContent ? null : thinkingStream"
      :is-thinking="isThinking && !thinkingPanelContent"
      @close="closeThinkingPanel"
    />
    <ChatSidebarPanel
      v-if="changesPanelVisible"
      scoped-session
      :storage-scope="sessionKey"
      :workspace-ref="workspaceRef"
      :session-id="sessionId"
      :messages="messages"
      :is-streaming="isStreaming"
      :unity-connected="workspaceUnityConnected"
      :check-undo-conflicts="checkUndoConflicts"
      :check-undo-dirty="checkUndoDirty"
      :perform-undo="performUndo"
      :restore-composer-draft="restoreComposerDraft"
    />
  </div>
</template>

<style scoped>
.workbench-session-shell {
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.workbench-session-editor {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
}
</style>
