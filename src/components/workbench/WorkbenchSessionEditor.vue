<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { useEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
import { useSkills } from "../../composables/useSkills";
import { useWorkspaceAssetDbStatus } from "../../composables/useWorkspaceAssetDbStatus";
import { useWorkspaceUnityStatus } from "../../composables/useWorkspaceUnityStatus";
import type { WorkspaceRef } from "../../services/project";
import { useAgentStore } from "../../stores/agent";
import { useAuthStore } from "../../stores/auth";
import { useChatStore } from "../../stores/chat";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import type {
  AssetRefAttachment,
  ChatComposerSendPayload,
  EffortLevel,
  ImageAttachment,
  UserIntentMeta,
} from "../../types";
import type { WorkbenchEditorInput } from "../../types/workbench";
import ChatView from "../ChatView.vue";

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
}>();

const agentStore = useAgentStore();
const authStore = useAuthStore();
const chatStore = useChatStore();
const modelStore = useModelStore();
const notificationStore = useNotificationStore();
const workspaceContextStore = useWorkspaceContextStore();
const { skillItems } = useSkills();
const chatViewRef = ref<InstanceType<typeof ChatView> | null>(null);

const requestedSessionId = computed(() => (
  props.editor.resource.kind === "session" ? props.editor.resource.sessionId : null
));
const sessionKey = computed(() => `workbench:${props.editor.editorId}`);
const editorAgentId = ref(agentStore.selectedAgentId?.trim() ?? "");
const editorModelId = ref(modelStore.selectedModelId);
const editorEffort = ref<EffortLevel>(modelStore.effort);
const editorFastMode = ref(modelStore.effectiveCodexFastMode);
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
  streamingTextOrder,
  thinkingOrder,
  liveRenderParts,
  livePartStreams,
  isStreaming,
  isCancelling,
  isCompacting,
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
  sessionAgentId,
  sessionModelId,
  sessionEffort,
  sessionFastMode,
  setExecutionSelection,
  sendComposerPayload,
  insertQueuedFollowUp,
  deleteQueuedFollowUp,
  reEditQueuedFollowUp,
  cancel,
  checkUndoDirty,
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
  [sessionAgentId, sessionModelId, sessionEffort, sessionFastMode] as const,
  ([agentId, modelId, effort, fastMode]) => {
    if (agentId && agentStore.agents.some((agent) => agent.id === agentId)) editorAgentId.value = agentId;
    if (modelId && modelStore.availableModels.some((model) => model.id === modelId)) editorModelId.value = modelId;
    if (effort && editorEfforts.value.includes(effort)) editorEffort.value = effort;
    if (fastMode != null) editorFastMode.value = fastMode;
  },
  { immediate: true },
);

watch(
  [selectedAgentId, editorModelId, editorEffort, editorFastMode] as const,
  ([agentId, modelId, effort, fastMode]) => {
    setExecutionSelection({ agentId, modelId, effort, fastMode });
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
    :can-resume-interrupted="false"
    :is-compacting="isCompacting"
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
    :fast-mode-enabled="editorFastMode"
    :fast-mode-available="editorFastModeAvailable"
    :token-usage="tokenUsage"
    :codex-connected="authStore.codexAuthenticated"
    :pending-question="pendingQuestion"
    :pending-tool-confirms="pendingToolConfirms"
    :queued-follow-up="queuedFollowUp"
    :composer-value="inputText"
    :sessions="chatStore.sessions"
    :active-session-id="sessionId"
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
    :show-session-navigation="false"
    :session-panel-storage-scope="sessionKey"
    @send="handleSend"
    @cancel="cancel"
    @select-agent="handleSelectAgent"
    @select-model="handleSelectModel"
    @select-effort="editorEffort = $event"
    @select-fast-mode="editorFastMode = $event"
    @answer-question="answerQuestion"
    @answer-tool-confirm="answerToolConfirm"
    @answer-all-tool-confirms="answerAllToolConfirms"
    @insert-queued-follow-up="insertQueuedFollowUp"
    @re-edit-queued-follow-up="handleReEditQueuedFollowUp"
    @delete-queued-follow-up="deleteQueuedFollowUp"
    @apply-knowledge-proposal="applyKnowledgeProposal"
    @ignore-knowledge-proposal="ignoreKnowledgeProposal"
    @update-composer-value="inputText = $event"
    @new-chat="handleNewSessionRequest"
    @start-scan="startWorkspaceAssetScan"
    @install-plugin="installWorkspaceUnityPlugin"
    @launch-unity-project="launchWorkspaceUnity"
  />
</template>

<style scoped>
.workbench-session-editor {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}
</style>
