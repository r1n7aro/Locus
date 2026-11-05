<script setup lang="ts">
import { computed, inject, nextTick, onBeforeUnmount, ref, watch, watchPostEffect } from "vue";
import { t } from "../../i18n";
import { DOCUMENT_SELECTION_HUB_KEY, liveSelectionQuoteForCheckout } from "../../composables/documentSelection";
import { useEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
import { useWorkspaceNavigationMode } from "../../composables/useWorkspaceNavigationMode";
import { readSharedComposerDraft } from "../../composables/chatComposerDraftMemory";
import { provideSurfaceContentReadiness } from "../../composables/surfaceContentReadiness";
import { useSkills } from "../../composables/useSkills";
import { useKnowledgeAccessMode } from "../../composables/useKnowledgeAccessMode";
import { useWorkspaceAssetDbStatus } from "../../composables/useWorkspaceAssetDbStatus";
import { useWorkspaceUnityStatus } from "../../composables/useWorkspaceUnityStatus";
import type { WorkspaceRef } from "../../services/project";
import type { ManagedWorktree } from "../../services/worktrees";
import { saveSessionExecutionState, unarchiveSession } from "../../services/session";
import { broadcastSessionExecutionState } from "../../services/sessionExecutionState";
import { useAgentStore } from "../../stores/agent";
import { useAuthStore } from "../../stores/auth";
import { useChatChangesStore } from "../../stores/chatChanges";
import { useModelStore } from "../../stores/model";
import { useNotificationStore } from "../../stores/notification";
import { useSelectionMemoryStore } from "../../stores/selectionMemory";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import type {
  AssetRefAttachment,
  ChatComposerSendPayload,
  EffortLevel,
  ImageAttachment,
  KnowledgeDocumentType,
  SessionContextExportRequest,
  SessionSummary,
  UserIntentMeta,
} from "../../types";
import type { WorkbenchEditorInput, WorkbenchEditorTransferSnapshot } from "../../types/workbench";
import ChatView from "../ChatView.vue";
import ChatSidebarPanel from "../ChatSidebarPanel.vue";
import ThinkingPanel from "../ThinkingPanel.vue";
import SessionComposerPicker from "../chat/SessionComposerPicker.vue";

// A literal `[]` would be a new array per render and update ChatView every time.
const NO_SESSIONS: SessionSummary[] = [];
const props = defineProps<{
  editor: WorkbenchEditorInput;
  workspaceRef: WorkspaceRef | null;
  selectWorktree?: (item: ManagedWorktree) => Promise<void>;
  workspaceChanging?: boolean;
  sessions?: SessionSummary[];
  active?: boolean;
  /** Whether the editor stack shows this surface (false while it is prepared behind another). */
  presented?: boolean;
  /** Mounted but not shown (`display: none`); explicit, so absent means shown. */
  surfaceHidden?: boolean;
  referenceDropAvailable?: boolean;
  referenceDropActive?: boolean;
  shortcutActive?: boolean;
  newChatShortcutAction?: "keepCurrent" | "replaceCurrent" | "newTab";
}>();

const emit = defineEmits<{
  (event: "session-unarchived", sessionId: string): void;
  (event: "ready"): void;
  (event: "session-created", payload: { editorId: string; sessionId: string }): void;
  (event: "session-selected", payload: { editorId: string; sessionId: string }): void;
  (event: "new-session-requested", payload: {
    editorId: string;
    source: "control" | "shortcut";
  }): void;
  (event: "composer-draft-change", payload: { editorId: string; hasDraft: boolean }): void;
  (event: "composer-focus", payload: { editorId: string }): void;
  /** Whether the session is busy and must stay mounted while its tab is hidden. */
  (event: "keep-mounted-change", keepMounted: boolean): void;
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
const workspaceNavigation = useWorkspaceNavigationMode();
const showSessionPicker = computed(() => (
  workspaceNavigation.getMode(props.workspaceRef?.checkoutId) === "files"
));
const { skillItems, loadSkills } = useSkills(() => props.workspaceRef);
const { state: knowledgeAccessState } = useKnowledgeAccessMode();
const selectionMemory = useSelectionMemoryStore();
const chatViewRef = ref<InstanceType<typeof ChatView> | null>(null);
const documentSelectionHub = inject(DOCUMENT_SELECTION_HUB_KEY, null);
// New and existing conversations both follow the window's document selection.
const liveSelectionQuote = computed(() => {
  const selection = documentSelectionHub?.current.value;
  if (!selection) return null;
  return liveSelectionQuoteForCheckout(selection, props.workspaceRef?.checkoutId);
});

const requestedSessionId = computed(() => (
  props.editor.resource.kind === "session"
    || (props.editor.resource.kind === "section" && props.editor.resource.section === "archived")
    ? props.editor.resource.sessionId ?? null : null
));
const sessionKey = computed(() => `workbench:${props.editor.editorId}`);
// Explicit values come from the user, the session, or a transferred tab. Until
// then the tab follows the workspace's remembered choice, so it stays correct
// while the workspace memory and model catalogs load.
const editorAgentId = ref(requestedSessionId.value ? agentStore.selectedAgentId?.trim() ?? "" : "");
const explicitModelId = ref<string | null>(null);
const explicitEffort = ref<EffortLevel | null>(null);
const explicitFastMode = ref<boolean | null>(null);
const editorMultiAgentEnabled = ref(false);
const selectedAgentId = computed(() => {
  const current = editorAgentId.value.trim();
  if (current && agentStore.agents.some((agent) => agent.id === current)) return current;
  // Workspace catalogs load asynchronously. Do not pin the previous checkout's
  // default through the embedded controller's execution-selection echo.
  if (props.workspaceRef && agentStore.workspaceCheckoutId !== props.workspaceRef.checkoutId) return "";
  return selectionMemory.resolveAgentId(
    props.workspaceRef?.checkoutId,
    agentStore.agents,
    { forNewSession: !requestedSessionId.value },
  );
});
const selectedAgent = computed(() => agentStore.agents.find((agent) => agent.id === selectedAgentId.value) ?? null);
const resolvedExecution = computed(() => selectionMemory.resolveExecution(
  props.workspaceRef?.checkoutId,
  selectedAgent.value ?? (selectedAgentId.value ? { id: selectedAgentId.value } : null),
));
const editorModelId = computed<string>({
  get: () => explicitModelId.value ?? (resolvedExecution.value.modelId || modelStore.selectedModelId),
  set: (modelId) => { explicitModelId.value = modelId; },
});
const editorEfforts = computed<EffortLevel[]>(() => modelStore.effortsForModel(editorModelId.value));
const editorEffort = computed<EffortLevel>({
  get: () => explicitEffort.value
    ?? selectionMemory.clampEffort(editorModelId.value, resolvedExecution.value.effort),
  set: (effort) => { explicitEffort.value = effort; },
});
const editorFastMode = computed<boolean>({
  get: () => editorFastModeAvailable.value && (explicitFastMode.value ?? resolvedExecution.value.fastMode),
  set: (enabled) => { explicitFastMode.value = enabled; },
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
  claudeCodePhase,
  hasThinking,
  thinkingDuration,
  activeToolCalls,
  tokenUsage,
  undoableMessageIds,
  pendingQuestion,
  pendingToolConfirms,
  queuedFollowUp,
  errorMessage,
  errorNotificationHandled,
  isLoading,
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
  async beforeSessionLaunch(targetSessionId) {
    if (props.editor.resource.kind !== "section" || props.editor.resource.section !== "archived") return;
    await unarchiveSession(targetSessionId);
    emit("session-unarchived", targetSessionId);
  },
  buildRequest(input) {
    return { text: input, displayText: input };
  },
});

// The editor stack keeps the previous surface on screen until "ready". Content
// still resolving a layout-changing value (unknown inline path statuses) holds
// it, so the first presented frame is final. A stalled request never blocks
// the switch for longer than the timeout.
const CONTENT_READY_TIMEOUT_MS = 200;
const contentReadiness = provideSurfaceContentReadiness();
let contentReadyTimer: ReturnType<typeof setTimeout> | undefined;

function clearContentReadyTimer(): void {
  clearTimeout(contentReadyTimer);
  contentReadyTimer = undefined;
}

watchPostEffect(() => {
  if (isLoading.value) return;
  if (contentReadiness.pending.value > 0) {
    contentReadyTimer ??= setTimeout(() => {
      contentReadyTimer = undefined;
      emit("ready");
    }, CONTENT_READY_TIMEOUT_MS);
    return;
  }
  clearContentReadyTimer();
  emit("ready");
});

onBeforeUnmount(clearContentReadyTimer);

// Restore before the input's controlled-value watcher can replace the cached text.
const recoveredDraft = readSharedComposerDraft(sessionKey.value);
if (!inputText.value && recoveredDraft) inputText.value = recoveredDraft.text;

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

// The workbench unmounts hidden session tabs beyond its per-group limit. A
// running or waiting session stays mounted so its in-flight UI is not rebuilt.
watch(
  () => isStreaming.value || isCancelling.value || isCompacting.value || compactQueued.value
    || !!pendingQuestion.value || (pendingToolConfirms.value?.length ?? 0) > 0
    || !!queuedFollowUp.value,
  (keepMounted) => emit("keep-mounted-change", keepMounted),
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

watch([errorMessage, errorNotificationHandled], ([message, notified]) => {
  if (message && !notified) notificationStore.addNotice("error", message);
});

// What this tab last wrote into the embedded session state. Until a session
// exists those values only echo back here; adopting them as explicit would pin
// a selection resolved while the Agent catalog, workspace memory or saved
// default effort were still loading (Claude even accepts the "none" fallback).
let pushedSelection: {
  agentId: string;
  modelId: string;
  effort: EffortLevel;
  fastMode: boolean;
} | null = null;

watch(
  [sessionId, sessionAgentId, sessionModelId, sessionEffort, sessionFastMode, sessionMultiAgentEnabled] as const,
  ([targetSessionId, agentId, modelId, effort, fastMode, multiAgentEnabled]) => {
    const echo = targetSessionId ? null : pushedSelection;
    if (agentId && agentId !== echo?.agentId && agentStore.agents.some((agent) => agent.id === agentId)) {
      editorAgentId.value = agentId;
    }
    if (modelId && modelId !== echo?.modelId) {
      if (modelId.startsWith("claude_code/") || modelId.startsWith("locus-server/") || modelStore.availableModels.some((model) => model.id === modelId)) editorModelId.value = modelId;
    }
    if (effort && effort !== echo?.effort && editorEfforts.value.includes(effort)) editorEffort.value = effort;
    if (fastMode != null && fastMode !== echo?.fastMode) editorFastMode.value = editorFastModeAvailable.value && fastMode;
    editorMultiAgentEnabled.value = multiAgentEnabled;
  },
  { immediate: true },
);

watch(editorFastModeAvailable, (available) => {
  if (!available && explicitFastMode.value) explicitFastMode.value = false;
}, { immediate: true });

watch(() => props.workspaceRef?.checkoutId, () => {
  void selectionMemory.load(props.workspaceRef);
}, { immediate: true });

/** Records an explicit choice so the next session in this workspace starts with it. */
function rememberEditorSelection(): void {
  selectionMemory.remember(props.workspaceRef, {
    agentId: selectedAgentId.value,
    modelId: editorModelId.value,
    effort: editorEffortSupported.value ? editorEffort.value : null,
    fastMode: editorFastModeAvailable.value ? editorFastMode.value : null,
  });
}

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
    pushedSelection = { agentId, modelId, effort, fastMode };
    setExecutionSelection({ agentId, modelId, effort, fastMode, multiAgentEnabled });
  },
  { immediate: true },
);

function handleSend(
  text: string,
  images: ImageAttachment[],
  assetRefs: AssetRefAttachment[],
  overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null; blenderTarget?: string | null },
): void {
  const payload: ChatComposerSendPayload = {
    text,
    displayText: overrides?.displayText ?? text,
    images,
    assetRefs,
    mode: overrides?.mode ?? null,
    userIntent: overrides?.userIntent ?? null,
    blenderTarget: overrides?.blenderTarget ?? null,
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
  emit("composer-draft-change", {
    editorId: props.editor.editorId,
    hasDraft: !(chatViewRef.value?.isComposerDraftEmpty() ?? true),
  });
}

async function appendComposerDraft(
  draft: Parameters<NonNullable<InstanceType<typeof ChatView>["appendComposerDraft"]>>[0],
): Promise<void> {
  await chatViewRef.value?.appendComposerDraft(draft);
  emit("composer-draft-change", {
    editorId: props.editor.editorId,
    hasDraft: !(chatViewRef.value?.isComposerDraftEmpty() ?? true),
  });
}

function exportTransferSnapshot() {
  return {
    kind: "session" as const,
    composerDraft: chatViewRef.value?.exportComposerDraft() ?? null,
    executionSelection: exportExecutionSelection(),
    scrollState: chatViewRef.value?.exportWorkStateScroll() ?? null,
  };
}

function exportComposerDraft() {
  return chatViewRef.value?.exportComposerDraft() ?? null;
}

async function restoreWorkState(snapshot: WorkbenchEditorTransferSnapshot): Promise<boolean> {
  if (snapshot.kind !== "session") return false;
  await nextTick();
  if (!chatViewRef.value) return false;
  if (snapshot.composerDraft) await applyDraftPrefill(snapshot.composerDraft as Parameters<typeof applyDraftPrefill>[0]);
  if (snapshot.executionSelection) applyExecutionSelection(snapshot.executionSelection);
  if (snapshot.scrollState) chatViewRef.value.restoreWorkStateScroll(snapshot.scrollState);
  return true;
}

function exportExecutionSelection() {
  return { agentId: selectedAgentId.value, modelId: editorModelId.value, effort: editorEffort.value,
    fastMode: editorFastMode.value, multiAgentEnabled: editorMultiAgentEnabled.value };
}
function applyExecutionSelection(selection: ReturnType<typeof exportExecutionSelection>) {
  editorAgentId.value = selection.agentId;
  editorModelId.value = selection.modelId;
  editorEffort.value = selection.effort;
  editorFastMode.value = selection.fastMode;
  editorMultiAgentEnabled.value = selection.multiAgentEnabled;
}

async function focusComposerInput(): Promise<void> {
  await chatViewRef.value?.focusComposerInput();
}

function handleSelectAgent(agentId: string): void {
  const agent = agentStore.agents.find((item) => item.id === agentId);
  const rememberedModel = selectionMemory.snapshotFor(props.workspaceRef?.checkoutId)?.memory.agents[agentId];
  editorAgentId.value = agentId;
  if (!sessionId.value && (explicitModelId.value === null || rememberedModel)) {
    // A new session switches to the model and effort this Agent last used here.
    explicitModelId.value = null;
    explicitEffort.value = null;
  } else {
    const fallbackEffort = selectionMemory.fallbackEffort(agent);
    if (editorEfforts.value.includes(fallbackEffort)) editorEffort.value = fallbackEffort;
  }
  rememberEditorSelection();
}

function handleSelectModel(modelId: string): void {
  if (!modelStore.availableModels.some((model) => model.id === modelId)) return;
  editorModelId.value = modelId;
  const fittedEffort = selectionMemory.clampEffort(modelId, editorEffort.value);
  if (fittedEffort !== editorEffort.value) editorEffort.value = fittedEffort;
  if (!editorFastModeAvailable.value) editorFastMode.value = false;
  persistExecutionSelection();
  rememberEditorSelection();
}

function handleSelectEffort(effort: EffortLevel): void {
  if (!editorEfforts.value.includes(effort)) return;
  editorEffort.value = effort;
  persistExecutionSelection();
  rememberEditorSelection();
}

function handleSelectFastMode(enabled: boolean): void {
  editorFastMode.value = editorFastModeAvailable.value && enabled;
  persistExecutionSelection();
  rememberEditorSelection();
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
    editorAgentId.value = props.workspaceRef && agentStore.workspaceCheckoutId !== props.workspaceRef.checkoutId
      ? ""
      : selectionMemory.resolveAgentId(props.workspaceRef?.checkoutId, agentStore.agents);
    // The new session follows the workspace's latest remembered choice.
    explicitModelId.value = null;
    explicitEffort.value = null;
    explicitFastMode.value = null;
  }
  emit("new-session-requested", {
    editorId: props.editor.editorId,
    source: request.source,
  });
}

defineExpose({
  exportExecutionSelection,
  applyExecutionSelection,
  applyDraftPrefill,
  appendComposerDraft,
  exportComposerDraft,
  exportTransferSnapshot,
  restoreWorkState,
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
    :claude-code-phase="claudeCodePhase"
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
    :sessions="NO_SESSIONS"
    :active-session-id="sessionId"
    :active="active"
    :presented="presented"
    :surface-hidden="surfaceHidden"
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
    :select-worktree="selectWorktree"
    :workspace-changing="workspaceChanging"
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
    :live-selection-quote="liveSelectionQuote"
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
    @composer-focus="loadSkills(); emit('composer-focus', { editorId: props.editor.editorId })"
    @open-thinking="handleOpenThinking"
    @open-knowledge-document="handleOpenKnowledgeDocument('editor', $event)"
    @open-knowledge-document-in-knowledge="handleOpenKnowledgeDocument('knowledge', $event)"
    @new-chat="handleNewSessionRequest"
    @start-scan="startWorkspaceAssetScan"
    @install-plugin="installWorkspaceUnityPlugin"
    @launch-unity-project="launchWorkspaceUnity"
    >
      <template v-if="showSessionPicker" #session-picker>
        <SessionComposerPicker
          :sessions="sessions ?? []"
          :active-session-id="sessionId"
          :active="active"
          :disabled="workspaceChanging"
          @select-session="emit('session-selected', { editorId: editor.editorId, sessionId: $event })"
        />
      </template>
    </ChatView>
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
